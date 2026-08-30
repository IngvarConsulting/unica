use crate::application::receipt_ledger::{
    receipt_key_digest, OriginalCutoffDescriptor, ReceiptKey, ReceiptKeyDigest, ReceiptLedgerError,
    ReceiptLedgerPort, ReceiptState, ReserveOutcome,
};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

const RECEIPT_LEDGER_CHANNEL_CAPACITY: usize = 64;
const READY: u8 = 0;
const RECOVERY_REQUIRED: u8 = 1;
const ENQUEUE_RETRY_SLICE: Duration = Duration::from_millis(1);

#[derive(Clone)]
pub(crate) struct ReceiptLedgerActor {
    sender: SyncSender<Command>,
    health: Arc<ActorHealth>,
}

struct ActorHealth {
    state: AtomicU8,
    wake_generation: Mutex<u64>,
    changed: Condvar,
}

impl ActorHealth {
    fn ready() -> Self {
        Self {
            state: AtomicU8::new(READY),
            wake_generation: Mutex::new(0),
            changed: Condvar::new(),
        }
    }

    fn is_ready(&self) -> bool {
        self.state.load(Ordering::SeqCst) == READY
    }

    fn latch_recovery_required(&self) {
        self.state.store(RECOVERY_REQUIRED, Ordering::SeqCst);
        self.wake_all();
    }

    fn wake_all(&self) {
        let mut generation = self
            .wake_generation
            .lock()
            .expect("receipt actor wake mutex poisoned");
        *generation = generation.wrapping_add(1);
        self.changed.notify_all();
    }

    fn generation(&self) -> u64 {
        *self
            .wake_generation
            .lock()
            .expect("receipt actor wake mutex poisoned")
    }

    fn wait_for_change(&self, observed_generation: u64, timeout: Duration) {
        let generation = self
            .wake_generation
            .lock()
            .expect("receipt actor wake mutex poisoned");
        let (generation, _) = self
            .changed
            .wait_timeout_while(generation, timeout, |generation| {
                *generation == observed_generation
            })
            .expect("receipt actor wake mutex poisoned while waiting");
        drop(generation);
    }
}

enum Command {
    Reserve {
        key: ReceiptKey,
        original_cutoff: OriginalCutoffDescriptor,
        deadline: Instant,
        ticket: Arc<Ticket<ReserveOutcome>>,
    },
    Recover {
        key: ReceiptKey,
        deadline: Instant,
        ticket: Arc<Ticket<ReceiptState>>,
    },
}

enum TicketState<R> {
    Queued,
    Running,
    Finished(Option<Result<R, ReceiptLedgerError>>),
    Abandoned,
}

struct Ticket<R> {
    state: Mutex<TicketState<R>>,
}

impl<R> Ticket<R> {
    fn queued() -> Self {
        Self {
            state: Mutex::new(TicketState::Queued),
        }
    }

    /// Atomically decides whether the port invocation may begin.
    ///
    /// The time check and `Queued -> Running` transition share one gate with
    /// the caller's timeout path, so a queued command cannot start after its
    /// caller has returned `DeadlineExceeded`.
    fn try_begin(&self, deadline: Instant, health: &ActorHealth) -> bool {
        let mut state = self.state.lock().expect("receipt ticket mutex poisoned");
        let started = match &*state {
            TicketState::Queued if health.is_ready() && Instant::now() < deadline => {
                *state = TicketState::Running;
                true
            }
            TicketState::Queued => {
                let error = if health.is_ready() {
                    ReceiptLedgerError::DeadlineExceeded
                } else {
                    ReceiptLedgerError::StoreUnavailable
                };
                *state = TicketState::Finished(Some(Err(error)));
                false
            }
            TicketState::Abandoned | TicketState::Finished(_) => false,
            TicketState::Running => unreachable!("receipt ticket can begin only once"),
        };
        drop(state);
        if !started {
            health.wake_all();
        }
        started
    }

    fn finish(&self, result: Result<R, ReceiptLedgerError>, health: &ActorHealth) {
        if result
            .as_ref()
            .is_err_and(ReceiptLedgerError::requires_reopen)
        {
            health.latch_recovery_required();
        }

        let mut state = self.state.lock().expect("receipt ticket mutex poisoned");
        match &*state {
            TicketState::Running => {
                *state = TicketState::Finished(Some(result));
            }
            TicketState::Abandoned => {
                // The caller already classified the running timeout and
                // latched the actor. A late port result has no authority.
            }
            TicketState::Queued | TicketState::Finished(_) => {
                unreachable!("only a running receipt ticket can finish")
            }
        }
        drop(state);
        health.wake_all();
    }

    fn wait(
        &self,
        deadline: Instant,
        health: &ActorHealth,
        timeout_class: TimeoutClass,
    ) -> Result<R, ReceiptLedgerError> {
        loop {
            let observed_generation = health.generation();
            let mut state = self.state.lock().expect("receipt ticket mutex poisoned");
            match &mut *state {
                TicketState::Finished(result) => {
                    return result
                        .take()
                        .expect("receipt ticket result can be consumed only once");
                }
                TicketState::Queued if !health.is_ready() => {
                    *state = TicketState::Abandoned;
                    return Err(ReceiptLedgerError::StoreUnavailable);
                }
                TicketState::Queued | TicketState::Running if Instant::now() >= deadline => {
                    let was_running = matches!(&*state, TicketState::Running);
                    *state = TicketState::Abandoned;
                    drop(state);
                    if was_running {
                        health.latch_recovery_required();
                        return Err(timeout_class.running_error());
                    }
                    return Err(ReceiptLedgerError::DeadlineExceeded);
                }
                TicketState::Queued | TicketState::Running => {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    drop(state);
                    health.wait_for_change(observed_generation, remaining);
                }
                TicketState::Abandoned => {
                    unreachable!("only the waiting caller can abandon its ticket")
                }
            }
        }
    }
}

enum TimeoutClass {
    Reserve(ReceiptKeyDigest),
    Recover,
}

impl TimeoutClass {
    fn running_error(self) -> ReceiptLedgerError {
        match self {
            Self::Reserve(receipt_key_digest) => {
                ReceiptLedgerError::CommitUncertain { receipt_key_digest }
            }
            Self::Recover => ReceiptLedgerError::StoreUnavailable,
        }
    }
}

impl ReceiptLedgerActor {
    pub(crate) fn spawn(port: impl ReceiptLedgerPort) -> Self {
        let (sender, receiver) = mpsc::sync_channel(RECEIPT_LEDGER_CHANNEL_CAPACITY);
        let health = Arc::new(ActorHealth::ready());
        let worker_health = Arc::clone(&health);
        std::thread::Builder::new()
            .name("unica-receipt-ledger".to_owned())
            .spawn(move || run_worker(port, receiver, worker_health))
            .expect("spawn receipt ledger actor");
        Self { sender, health }
    }

    pub(crate) fn reserve(
        &self,
        key: ReceiptKey,
        original_cutoff: OriginalCutoffDescriptor,
        deadline: Instant,
    ) -> Result<ReserveOutcome, ReceiptLedgerError> {
        if Instant::now() >= deadline {
            return Err(ReceiptLedgerError::DeadlineExceeded);
        }
        if !self.health.is_ready() {
            return Err(ReceiptLedgerError::StoreUnavailable);
        }

        let digest = receipt_key_digest(&key);
        let ticket = Arc::new(Ticket::queued());
        self.enqueue(
            Command::Reserve {
                key,
                original_cutoff,
                deadline,
                ticket: Arc::clone(&ticket),
            },
            deadline,
        )?;
        ticket.wait(deadline, &self.health, TimeoutClass::Reserve(digest))
    }

    pub(crate) fn recover(
        &self,
        key: ReceiptKey,
        deadline: Instant,
    ) -> Result<ReceiptState, ReceiptLedgerError> {
        if Instant::now() >= deadline {
            return Err(ReceiptLedgerError::DeadlineExceeded);
        }
        if !self.health.is_ready() {
            return Err(ReceiptLedgerError::StoreUnavailable);
        }

        let ticket = Arc::new(Ticket::queued());
        self.enqueue(
            Command::Recover {
                key,
                deadline,
                ticket: Arc::clone(&ticket),
            },
            deadline,
        )?;
        ticket.wait(deadline, &self.health, TimeoutClass::Recover)
    }

    fn enqueue(&self, mut command: Command, deadline: Instant) -> Result<(), ReceiptLedgerError> {
        loop {
            if !self.health.is_ready() {
                return Err(ReceiptLedgerError::StoreUnavailable);
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(ReceiptLedgerError::DeadlineExceeded);
            }

            match self.sender.try_send(command) {
                Ok(()) => return Ok(()),
                Err(TrySendError::Full(returned)) => {
                    command = returned;
                    std::thread::park_timeout(
                        ENQUEUE_RETRY_SLICE.min(deadline.saturating_duration_since(now)),
                    );
                }
                Err(TrySendError::Disconnected(_)) => {
                    self.health.latch_recovery_required();
                    return Err(ReceiptLedgerError::StoreUnavailable);
                }
            }
        }
    }
}

fn run_worker(
    mut port: impl ReceiptLedgerPort,
    receiver: mpsc::Receiver<Command>,
    health: Arc<ActorHealth>,
) {
    while let Ok(command) = receiver.recv() {
        match command {
            Command::Reserve {
                key,
                original_cutoff,
                deadline,
                ticket,
            } => {
                if !ticket.try_begin(deadline, &health) {
                    continue;
                }
                let digest = receipt_key_digest(&key);
                let result = catch_unwind(AssertUnwindSafe(|| {
                    port.reserve(key, original_cutoff, deadline)
                }))
                .unwrap_or(Err(ReceiptLedgerError::CommitUncertain {
                    receipt_key_digest: digest,
                }));
                ticket.finish(result, &health);
            }
            Command::Recover {
                key,
                deadline,
                ticket,
            } => {
                if !ticket.try_begin(deadline, &health) {
                    continue;
                }
                let result = catch_unwind(AssertUnwindSafe(|| port.recover(&key, deadline)))
                    .unwrap_or(Err(ReceiptLedgerError::StoreUnavailable));
                ticket.finish(result, &health);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Command, ReceiptLedgerActor, ReceiptLedgerPort, Ticket, TimeoutClass};
    use crate::application::invocation::normalized_arguments_hash;
    use crate::application::receipt_ledger::{
        receipt_key_digest, request_scope_hash, CoreIdentityDigest, OriginalCutoffDescriptor,
        ReceiptKey, ReceiptLedgerError, ReceiptState, RequestIdentity, ReserveOutcome,
        V5ToolIdentity,
    };
    use crate::domain::invocation::{InvocationId, TaskId};
    use std::cell::Cell;
    use std::str::FromStr;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{mpsc, Arc};
    use std::time::{Duration, Instant};

    struct BlockingPort {
        entered: Option<mpsc::Sender<()>>,
        release: mpsc::Receiver<()>,
        calls: Arc<AtomicUsize>,
    }

    impl ReceiptLedgerPort for BlockingPort {
        fn reserve(
            &mut self,
            _key: ReceiptKey,
            _original_cutoff: OriginalCutoffDescriptor,
            _deadline: Instant,
        ) -> Result<ReserveOutcome, ReceiptLedgerError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if let Some(entered) = self.entered.take() {
                entered.send(()).expect("report first port entry");
                self.release.recv().expect("release first port call");
            }
            Err(ReceiptLedgerError::CapacityExceeded)
        }

        fn recover(
            &mut self,
            _key: &ReceiptKey,
            _deadline: Instant,
        ) -> Result<ReceiptState, ReceiptLedgerError> {
            Err(ReceiptLedgerError::ReceiptNotFound)
        }
    }

    struct PanickingPort;

    impl ReceiptLedgerPort for PanickingPort {
        fn reserve(
            &mut self,
            _key: ReceiptKey,
            _original_cutoff: OriginalCutoffDescriptor,
            _deadline: Instant,
        ) -> Result<ReserveOutcome, ReceiptLedgerError> {
            panic!("injected receipt port panic")
        }

        fn recover(
            &mut self,
            _key: &ReceiptKey,
            _deadline: Instant,
        ) -> Result<ReceiptState, ReceiptLedgerError> {
            panic!("injected receipt recover panic")
        }
    }

    struct ErrorPort {
        error: ReceiptLedgerError,
        calls: Arc<AtomicUsize>,
    }

    impl ReceiptLedgerPort for ErrorPort {
        fn reserve(
            &mut self,
            _key: ReceiptKey,
            _original_cutoff: OriginalCutoffDescriptor,
            _deadline: Instant,
        ) -> Result<ReserveOutcome, ReceiptLedgerError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(self.error.clone())
        }

        fn recover(
            &mut self,
            _key: &ReceiptKey,
            _deadline: Instant,
        ) -> Result<ReceiptState, ReceiptLedgerError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(self.error.clone())
        }
    }

    struct DeadlineRecordingSendOnlyPort {
        calls: Cell<usize>,
        seen: mpsc::Sender<(Instant, usize)>,
    }

    impl ReceiptLedgerPort for DeadlineRecordingSendOnlyPort {
        fn reserve(
            &mut self,
            _key: ReceiptKey,
            _original_cutoff: OriginalCutoffDescriptor,
            deadline: Instant,
        ) -> Result<ReserveOutcome, ReceiptLedgerError> {
            self.calls.set(self.calls.get() + 1);
            self.seen
                .send((deadline, self.calls.get()))
                .expect("record exact port deadline");
            Err(ReceiptLedgerError::CapacityExceeded)
        }

        fn recover(
            &mut self,
            _key: &ReceiptKey,
            deadline: Instant,
        ) -> Result<ReceiptState, ReceiptLedgerError> {
            self.calls.set(self.calls.get() + 1);
            self.seen
                .send((deadline, self.calls.get()))
                .expect("record exact port deadline");
            Err(ReceiptLedgerError::ReceiptNotFound)
        }
    }

    struct BlockingRecoverPort {
        entered: Option<mpsc::Sender<()>>,
        release: mpsc::Receiver<()>,
        calls: Arc<AtomicUsize>,
    }

    impl ReceiptLedgerPort for BlockingRecoverPort {
        fn reserve(
            &mut self,
            _key: ReceiptKey,
            _original_cutoff: OriginalCutoffDescriptor,
            _deadline: Instant,
        ) -> Result<ReserveOutcome, ReceiptLedgerError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(ReceiptLedgerError::CapacityExceeded)
        }

        fn recover(
            &mut self,
            _key: &ReceiptKey,
            _deadline: Instant,
        ) -> Result<ReceiptState, ReceiptLedgerError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if let Some(entered) = self.entered.take() {
                entered.send(()).expect("report recover port entry");
                self.release.recv().expect("release recover port call");
            }
            Err(ReceiptLedgerError::ReceiptNotFound)
        }
    }

    fn receipt_key() -> ReceiptKey {
        ReceiptKey::new(
            InvocationId::new(),
            TaskId::new(),
            RequestIdentity::new(
                CoreIdentityDigest::from_str(&"55".repeat(32)).expect("core identity digest"),
                V5ToolIdentity::View,
                normalized_arguments_hash(&serde_json::Map::new()),
                request_scope_hash("workspace-a").expect("request scope"),
            ),
        )
    }

    fn cutoff() -> OriginalCutoffDescriptor {
        OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid original cutoff")
    }

    #[test]
    fn expired_command_queued_behind_running_reserve_never_reaches_the_port() {
        let (entered, entered_wait) = mpsc::channel();
        let (release, release_wait) = mpsc::channel();
        let calls = Arc::new(AtomicUsize::new(0));
        let actor = ReceiptLedgerActor::spawn(BlockingPort {
            entered: Some(entered),
            release: release_wait,
            calls: Arc::clone(&calls),
        });
        let first_actor = actor.clone();
        let first = std::thread::spawn(move || {
            first_actor.reserve(
                receipt_key(),
                cutoff(),
                Instant::now() + Duration::from_secs(2),
            )
        });
        entered_wait
            .recv_timeout(Duration::from_secs(1))
            .expect("first reserve entered the port");

        assert_eq!(
            actor
                .reserve(
                    receipt_key(),
                    cutoff(),
                    Instant::now() + Duration::from_millis(40),
                )
                .expect_err("queued reserve must expire"),
            ReceiptLedgerError::DeadlineExceeded
        );
        release.send(()).expect("release first reserve");
        assert_eq!(
            first
                .join()
                .expect("first caller does not panic")
                .expect_err("fixture rejects first reserve"),
            ReceiptLedgerError::CapacityExceeded
        );
        assert_eq!(
            actor
                .reserve(
                    receipt_key(),
                    cutoff(),
                    Instant::now() + Duration::from_secs(1),
                )
                .expect_err("third reserve reaches fixture"),
            ReceiptLedgerError::CapacityExceeded
        );
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn running_reserve_deadline_is_commit_uncertain_and_fail_stops_actor() {
        let (entered, entered_wait) = mpsc::channel();
        let (release, release_wait) = mpsc::channel();
        let calls = Arc::new(AtomicUsize::new(0));
        let actor = ReceiptLedgerActor::spawn(BlockingPort {
            entered: Some(entered),
            release: release_wait,
            calls: Arc::clone(&calls),
        });
        let first_key = receipt_key();
        let expected_digest = crate::application::receipt_ledger::receipt_key_digest(&first_key);
        let first_actor = actor.clone();
        let first = std::thread::spawn(move || {
            first_actor.reserve(
                first_key,
                cutoff(),
                Instant::now() + Duration::from_millis(40),
            )
        });
        entered_wait
            .recv_timeout(Duration::from_secs(1))
            .expect("reserve entered the port");

        assert_eq!(
            first
                .join()
                .expect("deadline caller does not panic")
                .expect_err("running reserve cannot report a clean timeout"),
            ReceiptLedgerError::CommitUncertain {
                receipt_key_digest: expected_digest,
            }
        );
        assert_eq!(
            actor
                .reserve(
                    receipt_key(),
                    cutoff(),
                    Instant::now() + Duration::from_secs(1),
                )
                .expect_err("uncertain actor requires recovery"),
            ReceiptLedgerError::StoreUnavailable
        );
        release.send(()).expect("release uncertain fixture call");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn reserve_panic_is_commit_uncertain_and_fail_stops_actor() {
        let actor = ReceiptLedgerActor::spawn(PanickingPort);
        let key = receipt_key();
        let expected_digest = crate::application::receipt_ledger::receipt_key_digest(&key);

        assert_eq!(
            actor
                .reserve(key, cutoff(), Instant::now() + Duration::from_secs(1))
                .expect_err("port panic cannot escape as a clean failure"),
            ReceiptLedgerError::CommitUncertain {
                receipt_key_digest: expected_digest,
            }
        );
        assert_eq!(
            actor
                .reserve(
                    receipt_key(),
                    cutoff(),
                    Instant::now() + Duration::from_secs(1),
                )
                .expect_err("panicked actor requires recovery"),
            ReceiptLedgerError::StoreUnavailable
        );
    }

    #[test]
    fn fail_stop_port_error_latches_but_clean_capacity_rejection_does_not() {
        let fail_stop_calls = Arc::new(AtomicUsize::new(0));
        let fail_stop = ReceiptLedgerActor::spawn(ErrorPort {
            error: ReceiptLedgerError::Storage {
                operation: "injected receipt write",
                message: "failed".to_owned(),
            },
            calls: Arc::clone(&fail_stop_calls),
        });
        assert!(matches!(
            fail_stop.reserve(
                receipt_key(),
                cutoff(),
                Instant::now() + Duration::from_secs(1),
            ),
            Err(ReceiptLedgerError::Storage { .. })
        ));
        assert_eq!(
            fail_stop
                .reserve(
                    receipt_key(),
                    cutoff(),
                    Instant::now() + Duration::from_secs(1),
                )
                .expect_err("storage failure latches actor"),
            ReceiptLedgerError::StoreUnavailable
        );
        assert_eq!(fail_stop_calls.load(Ordering::SeqCst), 1);

        let capacity_calls = Arc::new(AtomicUsize::new(0));
        let capacity = ReceiptLedgerActor::spawn(ErrorPort {
            error: ReceiptLedgerError::CapacityExceeded,
            calls: Arc::clone(&capacity_calls),
        });
        for _ in 0..2 {
            assert_eq!(
                capacity
                    .reserve(
                        receipt_key(),
                        cutoff(),
                        Instant::now() + Duration::from_secs(1),
                    )
                    .expect_err("capacity fixture rejects without fail-stop"),
                ReceiptLedgerError::CapacityExceeded
            );
        }
        assert_eq!(capacity_calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn deadline_expired_before_enqueue_never_reaches_the_port() {
        let calls = Arc::new(AtomicUsize::new(0));
        let actor = ReceiptLedgerActor::spawn(ErrorPort {
            error: ReceiptLedgerError::CapacityExceeded,
            calls: Arc::clone(&calls),
        });

        assert_eq!(
            actor
                .reserve(receipt_key(), cutoff(), Instant::now())
                .expect_err("expired command is rejected before enqueue"),
            ReceiptLedgerError::DeadlineExceeded
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn send_only_port_receives_the_original_absolute_deadline_without_reset() {
        let (seen, seen_wait) = mpsc::channel();
        let actor = ReceiptLedgerActor::spawn(DeadlineRecordingSendOnlyPort {
            calls: Cell::new(0),
            seen,
        });
        let reserve_deadline = Instant::now() + Duration::from_secs(1);
        assert_eq!(
            actor
                .reserve(receipt_key(), cutoff(), reserve_deadline)
                .expect_err("fixture rejects reserve"),
            ReceiptLedgerError::CapacityExceeded
        );
        assert_eq!(
            seen_wait.recv().expect("reserve deadline observation"),
            (reserve_deadline, 1)
        );

        let recover_deadline = Instant::now() + Duration::from_secs(1);
        assert_eq!(
            actor
                .recover(receipt_key(), recover_deadline)
                .expect_err("fixture misses receipt"),
            ReceiptLedgerError::ReceiptNotFound
        );
        assert_eq!(
            seen_wait.recv().expect("recover deadline observation"),
            (recover_deadline, 2)
        );
    }

    #[test]
    fn receipt_not_found_is_a_clean_recover_result_and_actor_remains_reusable() {
        let calls = Arc::new(AtomicUsize::new(0));
        let actor = ReceiptLedgerActor::spawn(ErrorPort {
            error: ReceiptLedgerError::ReceiptNotFound,
            calls: Arc::clone(&calls),
        });

        for _ in 0..2 {
            assert_eq!(
                actor
                    .recover(receipt_key(), Instant::now() + Duration::from_secs(1),)
                    .expect_err("fixture misses receipt"),
                ReceiptLedgerError::ReceiptNotFound
            );
        }
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn running_recover_deadline_is_store_unavailable_and_fail_stops_actor() {
        let (entered, entered_wait) = mpsc::channel();
        let (release, release_wait) = mpsc::channel();
        let calls = Arc::new(AtomicUsize::new(0));
        let actor = ReceiptLedgerActor::spawn(BlockingRecoverPort {
            entered: Some(entered),
            release: release_wait,
            calls: Arc::clone(&calls),
        });
        let first_actor = actor.clone();
        let first = std::thread::spawn(move || {
            first_actor.recover(receipt_key(), Instant::now() + Duration::from_millis(40))
        });
        entered_wait
            .recv_timeout(Duration::from_secs(1))
            .expect("recover entered the port");

        assert_eq!(
            first
                .join()
                .expect("recover caller does not panic")
                .expect_err("running read cannot report a clean deadline"),
            ReceiptLedgerError::StoreUnavailable
        );
        assert_eq!(
            actor
                .reserve(
                    receipt_key(),
                    cutoff(),
                    Instant::now() + Duration::from_secs(1),
                )
                .expect_err("timed-out recover requires reopen"),
            ReceiptLedgerError::StoreUnavailable
        );
        release.send(()).expect("release recover fixture");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn recover_panic_is_store_unavailable_and_fail_stops_actor() {
        let actor = ReceiptLedgerActor::spawn(PanickingPort);

        assert_eq!(
            actor
                .recover(receipt_key(), Instant::now() + Duration::from_secs(1),)
                .expect_err("recover panic cannot escape"),
            ReceiptLedgerError::StoreUnavailable
        );
        assert_eq!(
            actor
                .reserve(
                    receipt_key(),
                    cutoff(),
                    Instant::now() + Duration::from_secs(1),
                )
                .expect_err("panicked actor requires reopen"),
            ReceiptLedgerError::StoreUnavailable
        );
    }

    #[test]
    fn queued_command_is_drained_without_port_call_after_running_timeout_latches_actor() {
        let (entered, entered_wait) = mpsc::channel();
        let (release, release_wait) = mpsc::channel();
        let calls = Arc::new(AtomicUsize::new(0));
        let actor = ReceiptLedgerActor::spawn(BlockingPort {
            entered: Some(entered),
            release: release_wait,
            calls: Arc::clone(&calls),
        });
        let first_actor = actor.clone();
        let first = std::thread::spawn(move || {
            first_actor.reserve(
                receipt_key(),
                cutoff(),
                Instant::now() + Duration::from_millis(80),
            )
        });
        entered_wait
            .recv_timeout(Duration::from_secs(1))
            .expect("first reserve entered the port");

        let queued_key = receipt_key();
        let queued_deadline = Instant::now() + Duration::from_millis(300);
        let queued_ticket = Arc::new(Ticket::queued());
        actor
            .enqueue(
                Command::Reserve {
                    key: queued_key.clone(),
                    original_cutoff: cutoff(),
                    deadline: queued_deadline,
                    ticket: Arc::clone(&queued_ticket),
                },
                queued_deadline,
            )
            .expect("second command is proven queued behind the running port call");
        assert!(matches!(
            first.join().expect("first caller does not panic"),
            Err(ReceiptLedgerError::CommitUncertain { .. })
        ));
        assert_eq!(
            queued_ticket
                .wait(
                    queued_deadline,
                    &actor.health,
                    TimeoutClass::Reserve(receipt_key_digest(&queued_key)),
                )
                .expect_err("queued command drains after latch"),
            ReceiptLedgerError::StoreUnavailable
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        release.send(()).expect("release first reserve");
    }
}
