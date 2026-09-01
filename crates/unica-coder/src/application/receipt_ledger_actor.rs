use crate::application::receipt_ledger::{
    receipt_key_digest, AcknowledgedTombstoneReceipt, CancelExpiryOutcome, CancelResolution,
    CommittedDirectPublication, OriginalCutoffDescriptor, ReceiptKey, ReceiptKeyDigest,
    ReceiptLedgerError, ReceiptLedgerPort, ReceiptState, ReceiptVersion, ReserveOutcome,
    ReservedReceipt, TaskPromisedActorBoundReceipt, TaskPromisedUnboundReceipt, TerminalDigest,
    V5CanonicalTerminal,
};
#[cfg(feature = "receipt-ledger-test-support")]
use crate::application::receipt_ledger::{
    ReceiptLedgerCatalogSnapshot, ReceiptLedgerCatalogSnapshotAuthority,
};
use crate::domain::invocation::SafeIdentityHash;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

const RECEIPT_LEDGER_CHANNEL_CAPACITY: usize = 64;
const READY: u8 = 0;
const RECOVERY_REQUIRED: u8 = 1;
const ENQUEUE_RETRY_SLICE: Duration = Duration::from_millis(1);

struct HeavyResultSlot {
    occupied: Mutex<bool>,
    changed: Condvar,
}

impl HeavyResultSlot {
    fn available() -> Self {
        Self {
            occupied: Mutex::new(false),
            changed: Condvar::new(),
        }
    }

    fn acquire(
        self: &Arc<Self>,
        deadline: Instant,
        health: &ActorHealth,
    ) -> Result<HeavyResultPermit, ReceiptLedgerError> {
        let mut occupied = self
            .occupied
            .lock()
            .expect("receipt heavy-result permit mutex poisoned");
        loop {
            if !health.is_ready() {
                return Err(ReceiptLedgerError::StoreUnavailable);
            }

            let now = Instant::now();
            if now >= deadline {
                return Err(ReceiptLedgerError::DeadlineExceeded);
            }
            if !*occupied {
                *occupied = true;
                return Ok(HeavyResultPermit {
                    slot: Arc::clone(self),
                });
            }

            let (next, _) = self
                .changed
                .wait_timeout(occupied, deadline.saturating_duration_since(now))
                .expect("receipt heavy-result permit mutex poisoned while waiting");
            occupied = next;
        }
    }

    fn wake_all(&self) {
        // Synchronize health changes with the same mutex used by `acquire`.
        // Otherwise fail-stop could be published after the waiter checks
        // health but before it enters the condvar, losing the only wake-up.
        let occupied = self
            .occupied
            .lock()
            .expect("receipt heavy-result permit mutex poisoned while waking");
        self.changed.notify_all();
        drop(occupied);
    }
}

struct HeavyResultPermit {
    slot: Arc<HeavyResultSlot>,
}

impl Drop for HeavyResultPermit {
    fn drop(&mut self) {
        let mut occupied = self
            .slot
            .occupied
            .lock()
            .expect("receipt heavy-result permit mutex poisoned while releasing");
        debug_assert!(*occupied, "receipt heavy-result permit released only once");
        *occupied = false;
        self.slot.changed.notify_all();
    }
}

#[derive(Clone)]
pub(crate) struct ReceiptLedgerActor {
    worker: Arc<ReceiptLedgerWorkerOwner>,
    health: Arc<ActorHealth>,
}

struct ReceiptLedgerWorkerOwner {
    sender: Option<SyncSender<Command>>,
    worker: Option<std::thread::JoinHandle<()>>,
    health: Arc<ActorHealth>,
}

impl ReceiptLedgerWorkerOwner {
    fn sender(&self) -> &SyncSender<Command> {
        self.sender
            .as_ref()
            .expect("receipt ledger sender exists while an actor handle is alive")
    }
}

impl Drop for ReceiptLedgerWorkerOwner {
    fn drop(&mut self) {
        drop(self.sender.take());
        let Some(worker) = self.worker.take() else {
            return;
        };
        if !self.health.is_ready() {
            // A timed-out running port call is process-owned fail-stop work.
            // Joining it here would keep the process alive indefinitely and
            // violate the bounded caller contract.
            drop(worker);
            return;
        }
        if worker.thread().id() == std::thread::current().id() {
            return;
        }
        let _ = worker.join();
    }
}

struct ActorHealth {
    state: AtomicU8,
    wake_generation: Mutex<u64>,
    changed: Condvar,
    heavy_result_slot: Arc<HeavyResultSlot>,
}

impl ActorHealth {
    fn ready() -> Self {
        Self {
            state: AtomicU8::new(READY),
            wake_generation: Mutex::new(0),
            changed: Condvar::new(),
            heavy_result_slot: Arc::new(HeavyResultSlot::available()),
        }
    }

    fn is_ready(&self) -> bool {
        self.state.load(Ordering::SeqCst) == READY
    }

    fn latch_recovery_required(&self) {
        self.state.store(RECOVERY_REQUIRED, Ordering::SeqCst);
        self.heavy_result_slot.wake_all();
        self.wake_all();
    }

    fn acquire_heavy_result_permit(
        &self,
        deadline: Instant,
    ) -> Result<HeavyResultPermit, ReceiptLedgerError> {
        self.heavy_result_slot.acquire(deadline, self)
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
    #[cfg(feature = "receipt-ledger-test-support")]
    SnapshotCatalog {
        deadline: Instant,
        ticket: Arc<Ticket<ReceiptLedgerCatalogSnapshot>>,
    },
    Generation {
        deadline: Instant,
        ticket: Arc<Ticket<u64>>,
    },
    Reserve {
        key: ReceiptKey,
        original_cutoff: OriginalCutoffDescriptor,
        deadline: Instant,
        ticket: Arc<Ticket<ReserveOutcome>>,
    },
    BindReservedActor {
        key: ReceiptKey,
        expected_version: ReceiptVersion,
        bound_workspace_identity: SafeIdentityHash,
        deadline: Instant,
        ticket: Arc<Ticket<ReservedReceipt>>,
    },
    MarkReservedBegun {
        key: ReceiptKey,
        expected_version: ReceiptVersion,
        deadline: Instant,
        ticket: Arc<Ticket<ReservedReceipt>>,
    },
    PromiseTaskUnbound {
        key: ReceiptKey,
        expected_version: ReceiptVersion,
        created_at_epoch_ms: u64,
        ttl_ms: u64,
        poll_interval_ms: u64,
        deadline: Instant,
        ticket: Arc<Ticket<TaskPromisedUnboundReceipt>>,
    },
    BindPromisedTaskActor {
        key: ReceiptKey,
        expected_version: ReceiptVersion,
        workspace_identity_hash: SafeIdentityHash,
        deadline: Instant,
        ticket: Arc<Ticket<TaskPromisedActorBoundReceipt>>,
    },
    RequestCancelOrReserve {
        key: ReceiptKey,
        cancel_reserved_at_epoch_ms: u64,
        deadline: Instant,
        ticket: Arc<Ticket<CancelResolution>>,
    },
    ExpireCancelReserved {
        key: ReceiptKey,
        expected_version: ReceiptVersion,
        expected_mutation_sequence: u64,
        observed_at_epoch_ms: u64,
        deadline: Instant,
        ticket: Arc<Ticket<CancelExpiryOutcome>>,
    },
    PublishDirectTerminal {
        key: ReceiptKey,
        expected_version: ReceiptVersion,
        terminal_epoch_ms: u64,
        terminal: V5CanonicalTerminal,
        deadline: Instant,
        ticket: Arc<Ticket<CommittedDirectPublication>>,
    },
    AcknowledgeDirect {
        key: ReceiptKey,
        terminal_digest: TerminalDigest,
        acknowledged_at_epoch_ms: u64,
        deadline: Instant,
        ticket: Arc<Ticket<AcknowledgedTombstoneReceipt>>,
    },
    ReclaimExpiredTombstones {
        observed_at_epoch_ms: u64,
        deadline: Instant,
        ticket: Arc<Ticket<usize>>,
    },
    Recover {
        key: ReceiptKey,
        observed_at_epoch_ms: Option<u64>,
        deadline: Instant,
        ticket: Arc<Ticket<ReceiptState>>,
    },
    ResolveTask {
        task_id: crate::domain::invocation::TaskId,
        deadline: Instant,
        ticket: Arc<Ticket<ReceiptState>>,
    },
}

enum TicketState<R> {
    Queued,
    Running,
    Finished(Option<Result<R, ReceiptLedgerError>>),
    TimedOut(Option<ReceiptLedgerError>),
}

struct Ticket<R> {
    deadline: Instant,
    running_timeout_error: ReceiptLedgerError,
    _heavy_result_permit: Option<HeavyResultPermit>,
    state: Mutex<TicketState<R>>,
}

impl<R> Ticket<R> {
    fn queued(deadline: Instant, timeout_class: TimeoutClass) -> Self {
        Self {
            deadline,
            running_timeout_error: timeout_class.running_error(),
            _heavy_result_permit: None,
            state: Mutex::new(TicketState::Queued),
        }
    }

    fn queued_with_heavy_result_permit(
        deadline: Instant,
        timeout_class: TimeoutClass,
        heavy_result_permit: HeavyResultPermit,
    ) -> Self {
        Self {
            deadline,
            running_timeout_error: timeout_class.running_error(),
            _heavy_result_permit: Some(heavy_result_permit),
            state: Mutex::new(TicketState::Queued),
        }
    }

    /// Atomically decides whether the port invocation may begin.
    ///
    /// The time check and `Queued -> Running` transition share one gate with
    /// the caller's timeout path, so a queued command cannot start after its
    /// caller has returned `DeadlineExceeded`.
    fn try_begin(&self, health: &ActorHealth) -> bool {
        let mut state = self.state.lock().expect("receipt ticket mutex poisoned");
        let started = match &*state {
            TicketState::Queued if health.is_ready() && Instant::now() < self.deadline => {
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
            TicketState::TimedOut(_) | TicketState::Finished(_) => false,
            TicketState::Running => unreachable!("receipt ticket can begin only once"),
        };
        drop(state);
        if !started {
            health.wake_all();
        }
        started
    }

    fn finish(&self, result: Result<R, ReceiptLedgerError>, health: &ActorHealth) {
        self.finish_at(result, Instant::now(), health);
    }

    fn finish_at(
        &self,
        result: Result<R, ReceiptLedgerError>,
        completed_at: Instant,
        health: &ActorHealth,
    ) {
        let mut state = self.state.lock().expect("receipt ticket mutex poisoned");
        let should_latch = match &*state {
            TicketState::Running if completed_at >= self.deadline => {
                *state = TicketState::TimedOut(Some(self.running_timeout_error.clone()));
                true
            }
            TicketState::Running => {
                let should_latch = result
                    .as_ref()
                    .is_err_and(ReceiptLedgerError::requires_reopen);
                *state = TicketState::Finished(Some(result));
                should_latch
            }
            TicketState::TimedOut(_) => {
                // The caller already classified the running timeout and
                // latched the actor. A late port result has no authority.
                false
            }
            TicketState::Queued | TicketState::Finished(_) => {
                unreachable!("only a running receipt ticket can finish")
            }
        };
        if should_latch {
            // Keep the ticket locked until the fail-stop latch is visible, so
            // no caller can consume a fail-stop or late-completion result and
            // race a second operation against an apparently healthy actor.
            health.latch_recovery_required();
        }
        drop(state);
        if !should_latch {
            health.wake_all();
        }
    }

    fn wait(&self, health: &ActorHealth) -> Result<R, ReceiptLedgerError> {
        loop {
            let observed_generation = health.generation();
            let mut state = self.state.lock().expect("receipt ticket mutex poisoned");
            match &mut *state {
                TicketState::Finished(result) => {
                    return result
                        .take()
                        .expect("receipt ticket result can be consumed only once");
                }
                TicketState::TimedOut(error) => {
                    return Err(error
                        .take()
                        .expect("receipt ticket timeout can be consumed only once"));
                }
                TicketState::Queued if !health.is_ready() => {
                    *state = TicketState::TimedOut(None);
                    return Err(ReceiptLedgerError::StoreUnavailable);
                }
                TicketState::Queued | TicketState::Running if Instant::now() >= self.deadline => {
                    let was_running = matches!(&*state, TicketState::Running);
                    *state = TicketState::TimedOut(None);
                    drop(state);
                    if was_running {
                        health.latch_recovery_required();
                        return Err(self.running_timeout_error.clone());
                    }
                    return Err(ReceiptLedgerError::DeadlineExceeded);
                }
                TicketState::Queued | TicketState::Running => {
                    let remaining = self.deadline.saturating_duration_since(Instant::now());
                    drop(state);
                    health.wait_for_change(observed_generation, remaining);
                }
            }
        }
    }
}

enum TimeoutClass {
    #[cfg(feature = "receipt-ledger-test-support")]
    SnapshotCatalog,
    Generation,
    Reserve(ReceiptKeyDigest),
    BindReservedActor(ReceiptKeyDigest),
    MarkReservedBegun(ReceiptKeyDigest),
    PromiseTaskUnbound(ReceiptKeyDigest),
    BindPromisedTaskActor(ReceiptKeyDigest),
    RequestCancelOrReserve(ReceiptKeyDigest),
    ExpireCancelReserved(ReceiptKeyDigest),
    PublishDirectTerminal(ReceiptKeyDigest),
    AcknowledgeDirect(ReceiptKeyDigest),
    ReclaimExpiredTombstones,
    Recover,
    ResolveTask,
}

impl TimeoutClass {
    fn running_error(self) -> ReceiptLedgerError {
        match self {
            #[cfg(feature = "receipt-ledger-test-support")]
            Self::SnapshotCatalog => ReceiptLedgerError::StoreUnavailable,
            Self::Generation => ReceiptLedgerError::StoreUnavailable,
            Self::Reserve(receipt_key_digest)
            | Self::BindReservedActor(receipt_key_digest)
            | Self::MarkReservedBegun(receipt_key_digest)
            | Self::PromiseTaskUnbound(receipt_key_digest)
            | Self::BindPromisedTaskActor(receipt_key_digest)
            | Self::RequestCancelOrReserve(receipt_key_digest)
            | Self::ExpireCancelReserved(receipt_key_digest)
            | Self::PublishDirectTerminal(receipt_key_digest)
            | Self::AcknowledgeDirect(receipt_key_digest) => {
                ReceiptLedgerError::CommitUncertain { receipt_key_digest }
            }
            Self::ReclaimExpiredTombstones => ReceiptLedgerError::StoreUnavailable,
            Self::Recover => ReceiptLedgerError::StoreUnavailable,
            Self::ResolveTask => ReceiptLedgerError::StoreUnavailable,
        }
    }
}

impl ReceiptLedgerActor {
    pub(crate) fn spawn(port: impl ReceiptLedgerPort) -> Self {
        let (sender, receiver) = mpsc::sync_channel(RECEIPT_LEDGER_CHANNEL_CAPACITY);
        let health = Arc::new(ActorHealth::ready());
        let worker_health = Arc::clone(&health);
        let worker = std::thread::Builder::new()
            .name("unica-receipt-ledger".to_owned())
            .spawn(move || run_worker(port, receiver, worker_health))
            .expect("spawn receipt ledger actor");
        Self {
            worker: Arc::new(ReceiptLedgerWorkerOwner {
                sender: Some(sender),
                worker: Some(worker),
                health: Arc::clone(&health),
            }),
            health,
        }
    }

    #[cfg(feature = "receipt-ledger-test-support")]
    pub(crate) fn snapshot_catalog(
        &self,
        deadline: Instant,
    ) -> Result<ReceiptLedgerCatalogSnapshot, ReceiptLedgerError> {
        if Instant::now() >= deadline {
            return Err(ReceiptLedgerError::DeadlineExceeded);
        }
        if !self.health.is_ready() {
            return Err(ReceiptLedgerError::StoreUnavailable);
        }

        let heavy_result_permit = self.health.acquire_heavy_result_permit(deadline)?;
        let ticket = Arc::new(Ticket::queued_with_heavy_result_permit(
            deadline,
            TimeoutClass::SnapshotCatalog,
            heavy_result_permit,
        ));
        self.enqueue(
            Command::SnapshotCatalog {
                deadline,
                ticket: Arc::clone(&ticket),
            },
            deadline,
        )?;
        ticket.wait(&self.health)
    }

    pub(crate) fn generation(&self, deadline: Instant) -> Result<u64, ReceiptLedgerError> {
        if Instant::now() >= deadline {
            return Err(ReceiptLedgerError::DeadlineExceeded);
        }
        if !self.health.is_ready() {
            return Err(ReceiptLedgerError::StoreUnavailable);
        }

        let ticket = Arc::new(Ticket::queued(deadline, TimeoutClass::Generation));
        self.enqueue(
            Command::Generation {
                deadline,
                ticket: Arc::clone(&ticket),
            },
            deadline,
        )?;
        ticket.wait(&self.health)
    }

    pub(crate) fn restart_required(&self) -> bool {
        !self.health.is_ready()
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

        let heavy_result_permit = self.health.acquire_heavy_result_permit(deadline)?;
        let digest = receipt_key_digest(&key);
        let ticket = Arc::new(Ticket::queued_with_heavy_result_permit(
            deadline,
            TimeoutClass::Reserve(digest),
            heavy_result_permit,
        ));
        self.enqueue(
            Command::Reserve {
                key,
                original_cutoff,
                deadline,
                ticket: Arc::clone(&ticket),
            },
            deadline,
        )?;
        ticket.wait(&self.health)
    }

    pub(crate) fn bind_reserved_actor(
        &self,
        key: ReceiptKey,
        expected_version: ReceiptVersion,
        bound_workspace_identity: SafeIdentityHash,
        deadline: Instant,
    ) -> Result<ReservedReceipt, ReceiptLedgerError> {
        if Instant::now() >= deadline {
            return Err(ReceiptLedgerError::DeadlineExceeded);
        }
        if !self.health.is_ready() {
            return Err(ReceiptLedgerError::StoreUnavailable);
        }
        let heavy_result_permit = self.health.acquire_heavy_result_permit(deadline)?;
        let digest = receipt_key_digest(&key);
        let ticket = Arc::new(Ticket::queued_with_heavy_result_permit(
            deadline,
            TimeoutClass::BindReservedActor(digest),
            heavy_result_permit,
        ));
        self.enqueue(
            Command::BindReservedActor {
                key,
                expected_version,
                bound_workspace_identity,
                deadline,
                ticket: Arc::clone(&ticket),
            },
            deadline,
        )?;
        ticket.wait(&self.health)
    }

    pub(crate) fn mark_reserved_begun(
        &self,
        key: ReceiptKey,
        expected_version: ReceiptVersion,
        deadline: Instant,
    ) -> Result<ReservedReceipt, ReceiptLedgerError> {
        if Instant::now() >= deadline {
            return Err(ReceiptLedgerError::DeadlineExceeded);
        }
        if !self.health.is_ready() {
            return Err(ReceiptLedgerError::StoreUnavailable);
        }
        let heavy_result_permit = self.health.acquire_heavy_result_permit(deadline)?;
        let digest = receipt_key_digest(&key);
        let ticket = Arc::new(Ticket::queued_with_heavy_result_permit(
            deadline,
            TimeoutClass::MarkReservedBegun(digest),
            heavy_result_permit,
        ));
        self.enqueue(
            Command::MarkReservedBegun {
                key,
                expected_version,
                deadline,
                ticket: Arc::clone(&ticket),
            },
            deadline,
        )?;
        ticket.wait(&self.health)
    }

    pub(crate) fn promise_task_unbound(
        &self,
        key: ReceiptKey,
        expected_version: ReceiptVersion,
        created_at_epoch_ms: u64,
        ttl_ms: u64,
        poll_interval_ms: u64,
        deadline: Instant,
    ) -> Result<TaskPromisedUnboundReceipt, ReceiptLedgerError> {
        if Instant::now() >= deadline {
            return Err(ReceiptLedgerError::DeadlineExceeded);
        }
        if !self.health.is_ready() {
            return Err(ReceiptLedgerError::StoreUnavailable);
        }
        let heavy_result_permit = self.health.acquire_heavy_result_permit(deadline)?;
        let digest = receipt_key_digest(&key);
        let ticket = Arc::new(Ticket::queued_with_heavy_result_permit(
            deadline,
            TimeoutClass::PromiseTaskUnbound(digest),
            heavy_result_permit,
        ));
        self.enqueue(
            Command::PromiseTaskUnbound {
                key,
                expected_version,
                created_at_epoch_ms,
                ttl_ms,
                poll_interval_ms,
                deadline,
                ticket: Arc::clone(&ticket),
            },
            deadline,
        )?;
        ticket.wait(&self.health)
    }

    pub(crate) fn bind_promised_task_actor(
        &self,
        key: ReceiptKey,
        expected_version: ReceiptVersion,
        workspace_identity_hash: SafeIdentityHash,
        deadline: Instant,
    ) -> Result<TaskPromisedActorBoundReceipt, ReceiptLedgerError> {
        if Instant::now() >= deadline {
            return Err(ReceiptLedgerError::DeadlineExceeded);
        }
        if !self.health.is_ready() {
            return Err(ReceiptLedgerError::StoreUnavailable);
        }
        let heavy_result_permit = self.health.acquire_heavy_result_permit(deadline)?;
        let digest = receipt_key_digest(&key);
        let ticket = Arc::new(Ticket::queued_with_heavy_result_permit(
            deadline,
            TimeoutClass::BindPromisedTaskActor(digest),
            heavy_result_permit,
        ));
        self.enqueue(
            Command::BindPromisedTaskActor {
                key,
                expected_version,
                workspace_identity_hash,
                deadline,
                ticket: Arc::clone(&ticket),
            },
            deadline,
        )?;
        ticket.wait(&self.health)
    }

    pub(crate) fn request_cancel_or_reserve(
        &self,
        key: ReceiptKey,
        cancel_reserved_at_epoch_ms: u64,
        deadline: Instant,
    ) -> Result<CancelResolution, ReceiptLedgerError> {
        if Instant::now() >= deadline {
            return Err(ReceiptLedgerError::DeadlineExceeded);
        }
        if !self.health.is_ready() {
            return Err(ReceiptLedgerError::StoreUnavailable);
        }

        let heavy_result_permit = self.health.acquire_heavy_result_permit(deadline)?;
        let digest = receipt_key_digest(&key);
        let ticket = Arc::new(Ticket::queued_with_heavy_result_permit(
            deadline,
            TimeoutClass::RequestCancelOrReserve(digest),
            heavy_result_permit,
        ));
        self.enqueue(
            Command::RequestCancelOrReserve {
                key,
                cancel_reserved_at_epoch_ms,
                deadline,
                ticket: Arc::clone(&ticket),
            },
            deadline,
        )?;
        ticket.wait(&self.health)
    }

    pub(crate) fn expire_cancel_reserved(
        &self,
        key: ReceiptKey,
        expected_version: ReceiptVersion,
        expected_mutation_sequence: u64,
        observed_at_epoch_ms: u64,
        deadline: Instant,
    ) -> Result<CancelExpiryOutcome, ReceiptLedgerError> {
        if Instant::now() >= deadline {
            return Err(ReceiptLedgerError::DeadlineExceeded);
        }
        if !self.health.is_ready() {
            return Err(ReceiptLedgerError::StoreUnavailable);
        }

        let heavy_result_permit = self.health.acquire_heavy_result_permit(deadline)?;
        let digest = receipt_key_digest(&key);
        let ticket = Arc::new(Ticket::queued_with_heavy_result_permit(
            deadline,
            TimeoutClass::ExpireCancelReserved(digest),
            heavy_result_permit,
        ));
        self.enqueue(
            Command::ExpireCancelReserved {
                key,
                expected_version,
                expected_mutation_sequence,
                observed_at_epoch_ms,
                deadline,
                ticket: Arc::clone(&ticket),
            },
            deadline,
        )?;
        ticket.wait(&self.health)
    }

    pub(crate) fn recover(
        &self,
        key: ReceiptKey,
        deadline: Instant,
    ) -> Result<ReceiptState, ReceiptLedgerError> {
        self.recover_inner(key, None, deadline)
    }

    pub(crate) fn recover_at(
        &self,
        key: ReceiptKey,
        observed_at_epoch_ms: u64,
        deadline: Instant,
    ) -> Result<ReceiptState, ReceiptLedgerError> {
        self.recover_inner(key, Some(observed_at_epoch_ms), deadline)
    }

    fn recover_inner(
        &self,
        key: ReceiptKey,
        observed_at_epoch_ms: Option<u64>,
        deadline: Instant,
    ) -> Result<ReceiptState, ReceiptLedgerError> {
        if Instant::now() >= deadline {
            return Err(ReceiptLedgerError::DeadlineExceeded);
        }
        if !self.health.is_ready() {
            return Err(ReceiptLedgerError::StoreUnavailable);
        }

        let heavy_result_permit = self.health.acquire_heavy_result_permit(deadline)?;
        let ticket = Arc::new(Ticket::queued_with_heavy_result_permit(
            deadline,
            TimeoutClass::Recover,
            heavy_result_permit,
        ));
        self.enqueue(
            Command::Recover {
                key,
                observed_at_epoch_ms,
                deadline,
                ticket: Arc::clone(&ticket),
            },
            deadline,
        )?;
        ticket.wait(&self.health)
    }

    pub(crate) fn resolve_task(
        &self,
        task_id: crate::domain::invocation::TaskId,
        deadline: Instant,
    ) -> Result<ReceiptState, ReceiptLedgerError> {
        if Instant::now() >= deadline {
            return Err(ReceiptLedgerError::DeadlineExceeded);
        }
        if !self.health.is_ready() {
            return Err(ReceiptLedgerError::StoreUnavailable);
        }
        let heavy_result_permit = self.health.acquire_heavy_result_permit(deadline)?;
        let ticket = Arc::new(Ticket::queued_with_heavy_result_permit(
            deadline,
            TimeoutClass::ResolveTask,
            heavy_result_permit,
        ));
        self.enqueue(
            Command::ResolveTask {
                task_id,
                deadline,
                ticket: Arc::clone(&ticket),
            },
            deadline,
        )?;
        ticket.wait(&self.health)
    }

    pub(crate) fn publish_direct_terminal(
        &self,
        key: ReceiptKey,
        expected_version: ReceiptVersion,
        terminal_epoch_ms: u64,
        terminal: V5CanonicalTerminal,
        deadline: Instant,
    ) -> Result<CommittedDirectPublication, ReceiptLedgerError> {
        if Instant::now() >= deadline {
            return Err(ReceiptLedgerError::DeadlineExceeded);
        }
        if !self.health.is_ready() {
            return Err(ReceiptLedgerError::StoreUnavailable);
        }

        let heavy_result_permit = self.health.acquire_heavy_result_permit(deadline)?;
        let digest = receipt_key_digest(&key);
        let ticket = Arc::new(Ticket::queued_with_heavy_result_permit(
            deadline,
            TimeoutClass::PublishDirectTerminal(digest),
            heavy_result_permit,
        ));
        self.enqueue(
            Command::PublishDirectTerminal {
                key,
                expected_version,
                terminal_epoch_ms,
                terminal,
                deadline,
                ticket: Arc::clone(&ticket),
            },
            deadline,
        )?;
        ticket.wait(&self.health)
    }

    pub(crate) fn acknowledge_direct(
        &self,
        key: ReceiptKey,
        terminal_digest: TerminalDigest,
        acknowledged_at_epoch_ms: u64,
        deadline: Instant,
    ) -> Result<AcknowledgedTombstoneReceipt, ReceiptLedgerError> {
        if Instant::now() >= deadline {
            return Err(ReceiptLedgerError::DeadlineExceeded);
        }
        if !self.health.is_ready() {
            return Err(ReceiptLedgerError::StoreUnavailable);
        }

        let heavy_result_permit = self.health.acquire_heavy_result_permit(deadline)?;
        let digest = receipt_key_digest(&key);
        let ticket = Arc::new(Ticket::queued_with_heavy_result_permit(
            deadline,
            TimeoutClass::AcknowledgeDirect(digest),
            heavy_result_permit,
        ));
        self.enqueue(
            Command::AcknowledgeDirect {
                key,
                terminal_digest,
                acknowledged_at_epoch_ms,
                deadline,
                ticket: Arc::clone(&ticket),
            },
            deadline,
        )?;
        ticket.wait(&self.health)
    }

    pub(crate) fn reclaim_expired_tombstones(
        &self,
        observed_at_epoch_ms: u64,
        deadline: Instant,
    ) -> Result<usize, ReceiptLedgerError> {
        if Instant::now() >= deadline {
            return Err(ReceiptLedgerError::DeadlineExceeded);
        }
        if !self.health.is_ready() {
            return Err(ReceiptLedgerError::StoreUnavailable);
        }

        let ticket = Arc::new(Ticket::queued(
            deadline,
            TimeoutClass::ReclaimExpiredTombstones,
        ));
        self.enqueue(
            Command::ReclaimExpiredTombstones {
                observed_at_epoch_ms,
                deadline,
                ticket: Arc::clone(&ticket),
            },
            deadline,
        )?;
        ticket.wait(&self.health)
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

            match self.worker.sender().try_send(command) {
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
            #[cfg(feature = "receipt-ledger-test-support")]
            Command::SnapshotCatalog { deadline, ticket } => {
                if !ticket.try_begin(&health) {
                    continue;
                }
                let result = catch_unwind(AssertUnwindSafe(|| {
                    port.snapshot_catalog(ReceiptLedgerCatalogSnapshotAuthority::new(), deadline)
                }))
                .unwrap_or(Err(ReceiptLedgerError::StoreUnavailable));
                ticket.finish(result, &health);
            }
            Command::Generation { deadline, ticket } => {
                if !ticket.try_begin(&health) {
                    continue;
                }
                let result = catch_unwind(AssertUnwindSafe(|| port.generation(deadline)))
                    .unwrap_or(Err(ReceiptLedgerError::StoreUnavailable));
                ticket.finish(result, &health);
            }
            Command::Reserve {
                key,
                original_cutoff,
                deadline,
                ticket,
            } => {
                if !ticket.try_begin(&health) {
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
            Command::BindReservedActor {
                key,
                expected_version,
                bound_workspace_identity,
                deadline,
                ticket,
            } => {
                if !ticket.try_begin(&health) {
                    continue;
                }
                let digest = receipt_key_digest(&key);
                let result = catch_unwind(AssertUnwindSafe(|| {
                    port.bind_reserved_actor(
                        &key,
                        expected_version,
                        bound_workspace_identity,
                        deadline,
                    )
                }))
                .unwrap_or(Err(ReceiptLedgerError::CommitUncertain {
                    receipt_key_digest: digest,
                }));
                ticket.finish(result, &health);
            }
            Command::MarkReservedBegun {
                key,
                expected_version,
                deadline,
                ticket,
            } => {
                if !ticket.try_begin(&health) {
                    continue;
                }
                let digest = receipt_key_digest(&key);
                let result = catch_unwind(AssertUnwindSafe(|| {
                    port.mark_reserved_begun(&key, expected_version, deadline)
                }))
                .unwrap_or(Err(ReceiptLedgerError::CommitUncertain {
                    receipt_key_digest: digest,
                }));
                ticket.finish(result, &health);
            }
            Command::PromiseTaskUnbound {
                key,
                expected_version,
                created_at_epoch_ms,
                ttl_ms,
                poll_interval_ms,
                deadline,
                ticket,
            } => {
                if !ticket.try_begin(&health) {
                    continue;
                }
                let digest = receipt_key_digest(&key);
                let result = catch_unwind(AssertUnwindSafe(|| {
                    port.promise_task_unbound(
                        &key,
                        expected_version,
                        created_at_epoch_ms,
                        ttl_ms,
                        poll_interval_ms,
                        deadline,
                    )
                }))
                .unwrap_or(Err(ReceiptLedgerError::CommitUncertain {
                    receipt_key_digest: digest,
                }));
                ticket.finish(result, &health);
            }
            Command::BindPromisedTaskActor {
                key,
                expected_version,
                workspace_identity_hash,
                deadline,
                ticket,
            } => {
                if !ticket.try_begin(&health) {
                    continue;
                }
                let digest = receipt_key_digest(&key);
                let result = catch_unwind(AssertUnwindSafe(|| {
                    port.bind_promised_task_actor(
                        &key,
                        expected_version,
                        workspace_identity_hash,
                        deadline,
                    )
                }))
                .unwrap_or(Err(ReceiptLedgerError::CommitUncertain {
                    receipt_key_digest: digest,
                }));
                ticket.finish(result, &health);
            }
            Command::RequestCancelOrReserve {
                key,
                cancel_reserved_at_epoch_ms,
                deadline,
                ticket,
            } => {
                if !ticket.try_begin(&health) {
                    continue;
                }
                let digest = receipt_key_digest(&key);
                let result = catch_unwind(AssertUnwindSafe(|| {
                    port.request_cancel_or_reserve(key, cancel_reserved_at_epoch_ms, deadline)
                }))
                .unwrap_or(Err(ReceiptLedgerError::CommitUncertain {
                    receipt_key_digest: digest,
                }));
                ticket.finish(result, &health);
            }
            Command::ExpireCancelReserved {
                key,
                expected_version,
                expected_mutation_sequence,
                observed_at_epoch_ms,
                deadline,
                ticket,
            } => {
                if !ticket.try_begin(&health) {
                    continue;
                }
                let digest = receipt_key_digest(&key);
                let result = catch_unwind(AssertUnwindSafe(|| {
                    port.expire_cancel_reserved(
                        key,
                        expected_version,
                        expected_mutation_sequence,
                        observed_at_epoch_ms,
                        deadline,
                    )
                }))
                .unwrap_or(Err(ReceiptLedgerError::CommitUncertain {
                    receipt_key_digest: digest,
                }));
                ticket.finish(result, &health);
            }
            Command::PublishDirectTerminal {
                key,
                expected_version,
                terminal_epoch_ms,
                terminal,
                deadline,
                ticket,
            } => {
                if !ticket.try_begin(&health) {
                    continue;
                }
                let digest = receipt_key_digest(&key);
                let result = catch_unwind(AssertUnwindSafe(|| {
                    port.publish_direct_terminal(
                        &key,
                        expected_version,
                        terminal_epoch_ms,
                        terminal,
                        deadline,
                    )
                }))
                .unwrap_or(Err(ReceiptLedgerError::CommitUncertain {
                    receipt_key_digest: digest,
                }));
                ticket.finish(result, &health);
            }
            Command::AcknowledgeDirect {
                key,
                terminal_digest,
                acknowledged_at_epoch_ms,
                deadline,
                ticket,
            } => {
                if !ticket.try_begin(&health) {
                    continue;
                }
                let digest = receipt_key_digest(&key);
                let result = catch_unwind(AssertUnwindSafe(|| {
                    port.acknowledge_direct(
                        &key,
                        &terminal_digest,
                        acknowledged_at_epoch_ms,
                        deadline,
                    )
                }))
                .unwrap_or(Err(ReceiptLedgerError::CommitUncertain {
                    receipt_key_digest: digest,
                }));
                ticket.finish(result, &health);
            }
            Command::ReclaimExpiredTombstones {
                observed_at_epoch_ms,
                deadline,
                ticket,
            } => {
                if !ticket.try_begin(&health) {
                    continue;
                }
                let result = catch_unwind(AssertUnwindSafe(|| {
                    port.reclaim_expired_tombstones(observed_at_epoch_ms, deadline)
                }))
                .unwrap_or(Err(ReceiptLedgerError::StoreUnavailable));
                ticket.finish(result, &health);
            }
            Command::Recover {
                key,
                observed_at_epoch_ms,
                deadline,
                ticket,
            } => {
                if !ticket.try_begin(&health) {
                    continue;
                }
                let result = catch_unwind(AssertUnwindSafe(|| match observed_at_epoch_ms {
                    Some(observed_at_epoch_ms) => {
                        port.recover_at(&key, observed_at_epoch_ms, deadline)
                    }
                    None => port.recover(&key, deadline),
                }))
                .unwrap_or(Err(ReceiptLedgerError::StoreUnavailable));
                ticket.finish(result, &health);
            }
            Command::ResolveTask {
                task_id,
                deadline,
                ticket,
            } => {
                if !ticket.try_begin(&health) {
                    continue;
                }
                let result =
                    catch_unwind(AssertUnwindSafe(|| port.resolve_task(task_id, deadline)))
                        .unwrap_or(Err(ReceiptLedgerError::StoreUnavailable));
                ticket.finish(result, &health);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ActorHealth, Command, ReceiptLedgerActor, ReceiptLedgerPort, Ticket, TimeoutClass,
    };
    use crate::application::invocation::normalized_arguments_hash;
    use crate::application::receipt_ledger::{
        canonical_v5_terminal, receipt_key_digest, request_scope_hash,
        AcknowledgedTombstoneReceipt, CancelExpiryOutcome, CancelReservedReceipt, CancelResolution,
        CommittedDirectPublication, CoreIdentityDigest, OriginalCutoffDescriptor, ReceiptKey,
        ReceiptLedgerError, ReceiptState, ReceiptTerminalOutcome, ReceiptVersion, RequestIdentity,
        ReserveOutcome, TerminalDigest, V5CanonicalTerminal, V5ToolIdentity,
    };
    use crate::domain::invocation::{InvocationId, TaskId};
    use std::cell::Cell;
    use std::str::FromStr;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{mpsc, Arc, Mutex};
    use std::time::{Duration, Instant};

    struct BlockingPort {
        entered: Option<mpsc::Sender<()>>,
        release: mpsc::Receiver<()>,
        calls: Arc<AtomicUsize>,
        direct_calls: Arc<AtomicUsize>,
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

        fn publish_direct_terminal(
            &mut self,
            _key: &ReceiptKey,
            _expected_version: ReceiptVersion,
            _terminal_epoch_ms: u64,
            _terminal: V5CanonicalTerminal,
            _deadline: Instant,
        ) -> Result<CommittedDirectPublication, ReceiptLedgerError> {
            self.direct_calls.fetch_add(1, Ordering::SeqCst);
            Err(ReceiptLedgerError::CapacityExceeded)
        }

        fn request_cancel_or_reserve(
            &mut self,
            _key: ReceiptKey,
            _cancel_reserved_at_epoch_ms: u64,
            _deadline: Instant,
        ) -> Result<CancelResolution, ReceiptLedgerError> {
            Err(ReceiptLedgerError::CapacityExceeded)
        }

        fn expire_cancel_reserved(
            &mut self,
            _key: ReceiptKey,
            _expected_version: ReceiptVersion,
            _expected_mutation_sequence: u64,
            _observed_at_epoch_ms: u64,
            _deadline: Instant,
        ) -> Result<CancelExpiryOutcome, ReceiptLedgerError> {
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

        fn publish_direct_terminal(
            &mut self,
            _key: &ReceiptKey,
            _expected_version: ReceiptVersion,
            _terminal_epoch_ms: u64,
            _terminal: V5CanonicalTerminal,
            _deadline: Instant,
        ) -> Result<CommittedDirectPublication, ReceiptLedgerError> {
            panic!("injected direct terminal panic")
        }

        fn request_cancel_or_reserve(
            &mut self,
            _key: ReceiptKey,
            _cancel_reserved_at_epoch_ms: u64,
            _deadline: Instant,
        ) -> Result<CancelResolution, ReceiptLedgerError> {
            panic!("injected cancel reservation panic")
        }

        fn expire_cancel_reserved(
            &mut self,
            _key: ReceiptKey,
            _expected_version: ReceiptVersion,
            _expected_mutation_sequence: u64,
            _observed_at_epoch_ms: u64,
            _deadline: Instant,
        ) -> Result<CancelExpiryOutcome, ReceiptLedgerError> {
            panic!("injected cancel expiry panic")
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

        fn publish_direct_terminal(
            &mut self,
            _key: &ReceiptKey,
            _expected_version: ReceiptVersion,
            _terminal_epoch_ms: u64,
            _terminal: V5CanonicalTerminal,
            _deadline: Instant,
        ) -> Result<CommittedDirectPublication, ReceiptLedgerError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(self.error.clone())
        }

        fn request_cancel_or_reserve(
            &mut self,
            _key: ReceiptKey,
            _cancel_reserved_at_epoch_ms: u64,
            _deadline: Instant,
        ) -> Result<CancelResolution, ReceiptLedgerError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(self.error.clone())
        }

        fn expire_cancel_reserved(
            &mut self,
            _key: ReceiptKey,
            _expected_version: ReceiptVersion,
            _expected_mutation_sequence: u64,
            _observed_at_epoch_ms: u64,
            _deadline: Instant,
        ) -> Result<CancelExpiryOutcome, ReceiptLedgerError> {
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

        fn publish_direct_terminal(
            &mut self,
            _key: &ReceiptKey,
            _expected_version: ReceiptVersion,
            _terminal_epoch_ms: u64,
            _terminal: V5CanonicalTerminal,
            deadline: Instant,
        ) -> Result<CommittedDirectPublication, ReceiptLedgerError> {
            self.calls.set(self.calls.get() + 1);
            self.seen
                .send((deadline, self.calls.get()))
                .expect("record exact port deadline");
            Err(ReceiptLedgerError::TerminalMismatch)
        }

        fn request_cancel_or_reserve(
            &mut self,
            _key: ReceiptKey,
            _cancel_reserved_at_epoch_ms: u64,
            deadline: Instant,
        ) -> Result<CancelResolution, ReceiptLedgerError> {
            self.calls.set(self.calls.get() + 1);
            self.seen
                .send((deadline, self.calls.get()))
                .expect("record exact port deadline");
            Err(ReceiptLedgerError::CapacityExceeded)
        }

        fn expire_cancel_reserved(
            &mut self,
            _key: ReceiptKey,
            _expected_version: ReceiptVersion,
            _expected_mutation_sequence: u64,
            _observed_at_epoch_ms: u64,
            deadline: Instant,
        ) -> Result<CancelExpiryOutcome, ReceiptLedgerError> {
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

        fn publish_direct_terminal(
            &mut self,
            _key: &ReceiptKey,
            _expected_version: ReceiptVersion,
            _terminal_epoch_ms: u64,
            _terminal: V5CanonicalTerminal,
            _deadline: Instant,
        ) -> Result<CommittedDirectPublication, ReceiptLedgerError> {
            Err(ReceiptLedgerError::CapacityExceeded)
        }

        fn request_cancel_or_reserve(
            &mut self,
            _key: ReceiptKey,
            _cancel_reserved_at_epoch_ms: u64,
            _deadline: Instant,
        ) -> Result<CancelResolution, ReceiptLedgerError> {
            Err(ReceiptLedgerError::CapacityExceeded)
        }

        fn expire_cancel_reserved(
            &mut self,
            _key: ReceiptKey,
            _expected_version: ReceiptVersion,
            _expected_mutation_sequence: u64,
            _observed_at_epoch_ms: u64,
            _deadline: Instant,
        ) -> Result<CancelExpiryOutcome, ReceiptLedgerError> {
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

    struct WorkerGenerationGate {
        actor_to_drop: Option<Arc<Mutex<Option<ReceiptLedgerActor>>>>,
        entered: mpsc::Sender<()>,
        release: mpsc::Receiver<()>,
    }

    struct DropNotifyingPort {
        generation: u64,
        dropped: mpsc::Sender<()>,
        worker_generation: Option<WorkerGenerationGate>,
    }

    impl Drop for DropNotifyingPort {
        fn drop(&mut self) {
            let _ = self.dropped.send(());
        }
    }

    impl ReceiptLedgerPort for DropNotifyingPort {
        fn generation(&mut self, _deadline: Instant) -> Result<u64, ReceiptLedgerError> {
            if let Some(gate) = self.worker_generation.take() {
                gate.entered.send(()).expect("report generation entry");
                gate.release.recv().expect("release worker generation");
                if let Some(actor) = gate.actor_to_drop {
                    drop(actor.lock().expect("self-drop actor mutex poisoned").take());
                }
            }
            Ok(self.generation)
        }

        fn reserve(
            &mut self,
            _key: ReceiptKey,
            _original_cutoff: OriginalCutoffDescriptor,
            _deadline: Instant,
        ) -> Result<ReserveOutcome, ReceiptLedgerError> {
            Err(ReceiptLedgerError::StoreUnavailable)
        }

        fn publish_direct_terminal(
            &mut self,
            _key: &ReceiptKey,
            _expected_version: ReceiptVersion,
            _terminal_epoch_ms: u64,
            _terminal: V5CanonicalTerminal,
            _deadline: Instant,
        ) -> Result<CommittedDirectPublication, ReceiptLedgerError> {
            Err(ReceiptLedgerError::StoreUnavailable)
        }

        fn request_cancel_or_reserve(
            &mut self,
            _key: ReceiptKey,
            _cancel_reserved_at_epoch_ms: u64,
            _deadline: Instant,
        ) -> Result<CancelResolution, ReceiptLedgerError> {
            Err(ReceiptLedgerError::StoreUnavailable)
        }

        fn expire_cancel_reserved(
            &mut self,
            _key: ReceiptKey,
            _expected_version: ReceiptVersion,
            _expected_mutation_sequence: u64,
            _observed_at_epoch_ms: u64,
            _deadline: Instant,
        ) -> Result<CancelExpiryOutcome, ReceiptLedgerError> {
            Err(ReceiptLedgerError::StoreUnavailable)
        }

        fn recover(
            &mut self,
            _key: &ReceiptKey,
            _deadline: Instant,
        ) -> Result<ReceiptState, ReceiptLedgerError> {
            Err(ReceiptLedgerError::StoreUnavailable)
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

    fn cancel_reserved_receipt() -> CancelReservedReceipt {
        CancelReservedReceipt::new(receipt_key(), ReceiptVersion::initial(), 1, 512, 1_000)
            .expect("valid cancellation reservation")
    }

    fn cancelled_terminal() -> V5CanonicalTerminal {
        canonical_v5_terminal(&ReceiptTerminalOutcome::Cancelled)
            .expect("cancelled terminal is canonical")
    }

    struct AcknowledgePort {
        calls: Arc<AtomicUsize>,
    }

    impl ReceiptLedgerPort for AcknowledgePort {
        fn reserve(
            &mut self,
            _key: ReceiptKey,
            _original_cutoff: OriginalCutoffDescriptor,
            _deadline: Instant,
        ) -> Result<ReserveOutcome, ReceiptLedgerError> {
            Err(ReceiptLedgerError::StoreUnavailable)
        }

        fn request_cancel_or_reserve(
            &mut self,
            _key: ReceiptKey,
            _cancel_reserved_at_epoch_ms: u64,
            _deadline: Instant,
        ) -> Result<CancelResolution, ReceiptLedgerError> {
            Err(ReceiptLedgerError::StoreUnavailable)
        }

        fn expire_cancel_reserved(
            &mut self,
            _key: ReceiptKey,
            _expected_version: ReceiptVersion,
            _expected_mutation_sequence: u64,
            _observed_at_epoch_ms: u64,
            _deadline: Instant,
        ) -> Result<CancelExpiryOutcome, ReceiptLedgerError> {
            Err(ReceiptLedgerError::StoreUnavailable)
        }

        fn publish_direct_terminal(
            &mut self,
            _key: &ReceiptKey,
            _expected_version: ReceiptVersion,
            _terminal_epoch_ms: u64,
            _terminal: V5CanonicalTerminal,
            _deadline: Instant,
        ) -> Result<CommittedDirectPublication, ReceiptLedgerError> {
            Err(ReceiptLedgerError::StoreUnavailable)
        }

        fn acknowledge_direct(
            &mut self,
            key: &ReceiptKey,
            terminal_digest: &TerminalDigest,
            acknowledged_at_epoch_ms: u64,
            _deadline: Instant,
        ) -> Result<AcknowledgedTombstoneReceipt, ReceiptLedgerError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            AcknowledgedTombstoneReceipt::new(
                key.clone(),
                receipt_key_digest(key),
                terminal_digest.clone(),
                acknowledged_at_epoch_ms,
                256,
            )
        }

        fn recover(
            &mut self,
            _key: &ReceiptKey,
            _deadline: Instant,
        ) -> Result<ReceiptState, ReceiptLedgerError> {
            Err(ReceiptLedgerError::StoreUnavailable)
        }
    }

    #[test]
    fn acknowledge_direct_round_trips_through_the_serial_actor() {
        let calls = Arc::new(AtomicUsize::new(0));
        let actor = ReceiptLedgerActor::spawn(AcknowledgePort {
            calls: Arc::clone(&calls),
        });
        let key = receipt_key();
        let digest = cancelled_terminal().digest().clone();

        let acknowledged = actor
            .acknowledge_direct(
                key.clone(),
                digest.clone(),
                1_234,
                Instant::now() + Duration::from_secs(1),
            )
            .expect("acknowledgement reaches the ledger port");

        assert_eq!(acknowledged.key(), &key);
        assert_eq!(acknowledged.terminal_digest(), &digest);
        assert_eq!(acknowledged.acknowledged_at_epoch_ms(), 1_234);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn dropping_last_actor_waits_until_the_port_is_released() {
        let (entered, observed_entry) = mpsc::channel();
        let (release, released) = mpsc::channel();
        let (dropped, observed_drop) = mpsc::channel();
        let actor = ReceiptLedgerActor::spawn(DropNotifyingPort {
            generation: 41,
            dropped,
            worker_generation: Some(WorkerGenerationGate {
                actor_to_drop: None,
                entered,
                release: released,
            }),
        });
        let health = Arc::clone(&actor.health);
        let deadline = Instant::now() + Duration::from_secs(2);
        let ticket = Arc::new(Ticket::queued(deadline, TimeoutClass::Generation));
        actor
            .enqueue(
                Command::Generation {
                    deadline,
                    ticket: Arc::clone(&ticket),
                },
                deadline,
            )
            .expect("enqueue blocked generation probe");
        observed_entry.recv().expect("worker enters generation");
        let (drop_returned, observed_drop_return) = mpsc::channel();
        let dropper = std::thread::spawn(move || {
            drop(actor);
            drop_returned.send(()).expect("report actor drop return");
        });

        let returned_before_release =
            match observed_drop_return.recv_timeout(Duration::from_millis(100)) {
                Ok(()) => true,
                Err(mpsc::RecvTimeoutError::Timeout) => false,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    panic!("actor dropper disconnected before reporting return")
                }
            };
        release.send(()).expect("release blocked generation");

        assert_eq!(ticket.wait(&health), Ok(41));
        observed_drop
            .recv_timeout(Duration::from_secs(1))
            .expect("worker releases its port after generation returns");
        if !returned_before_release {
            observed_drop_return
                .recv_timeout(Duration::from_secs(1))
                .expect("actor drop returns after port release");
        }
        dropper.join().expect("actor dropper does not panic");
        assert!(
            !returned_before_release,
            "last actor drop returned while its worker still owned the port"
        );
    }

    #[test]
    fn fail_stopped_actor_drop_does_not_wait_for_a_stuck_port() {
        let (entered, observed_entry) = mpsc::channel();
        let (release, released) = mpsc::channel();
        let actor = ReceiptLedgerActor::spawn(BlockingPort {
            entered: Some(entered),
            release: released,
            calls: Arc::new(AtomicUsize::new(0)),
            direct_calls: Arc::new(AtomicUsize::new(0)),
        });
        let caller = actor.clone();
        let key = receipt_key();
        let call = std::thread::spawn(move || {
            caller.reserve(key, cutoff(), Instant::now() + Duration::from_millis(30))
        });
        observed_entry
            .recv_timeout(Duration::from_secs(1))
            .expect("stuck port enters the mutation");
        assert!(matches!(
            call.join().expect("join timed-out actor caller"),
            Err(ReceiptLedgerError::CommitUncertain { .. })
        ));
        assert!(actor.restart_required());

        let (drop_returned, observed_drop_return) = mpsc::channel();
        let dropper = std::thread::spawn(move || {
            drop(actor);
            drop_returned
                .send(())
                .expect("report fail-stopped actor drop return");
        });
        let returned_without_port = observed_drop_return
            .recv_timeout(Duration::from_millis(100))
            .is_ok();
        release
            .send(())
            .expect("release stuck port after observation");
        if !returned_without_port {
            observed_drop_return
                .recv_timeout(Duration::from_secs(1))
                .expect("actor drop returns after cleanup release");
        }
        dropper.join().expect("join actor dropper");

        assert!(
            returned_without_port,
            "fail-stop waited for a worker that process death must own"
        );
    }

    #[test]
    fn dropping_a_non_last_clone_keeps_the_worker_available() {
        let (dropped, observed_drop) = mpsc::channel();
        let actor = ReceiptLedgerActor::spawn(DropNotifyingPort {
            generation: 41,
            dropped,
            worker_generation: None,
        });
        let survivor = actor.clone();

        drop(actor);

        assert_eq!(
            observed_drop.try_recv(),
            Err(mpsc::TryRecvError::Empty),
            "a non-last clone cannot release the shared port"
        );
        assert_eq!(
            survivor.generation(Instant::now() + Duration::from_secs(1)),
            Ok(41)
        );
        drop(survivor);
        observed_drop
            .try_recv()
            .expect("last surviving clone releases the port synchronously");
    }

    #[test]
    fn dropping_last_actor_on_its_worker_never_self_joins() {
        let actor_slot = Arc::new(Mutex::new(None));
        let (entered, observed_entry) = mpsc::channel();
        let (release, released) = mpsc::channel();
        let (dropped, observed_drop) = mpsc::channel();
        let actor = ReceiptLedgerActor::spawn(DropNotifyingPort {
            generation: 43,
            dropped,
            worker_generation: Some(WorkerGenerationGate {
                actor_to_drop: Some(Arc::clone(&actor_slot)),
                entered,
                release: released,
            }),
        });
        actor_slot
            .lock()
            .expect("self-drop actor mutex poisoned")
            .replace(actor.clone());
        let health = Arc::clone(&actor.health);
        let deadline = Instant::now() + Duration::from_secs(2);
        let ticket = Arc::new(Ticket::queued(deadline, TimeoutClass::Generation));
        actor
            .enqueue(
                Command::Generation {
                    deadline,
                    ticket: Arc::clone(&ticket),
                },
                deadline,
            )
            .expect("enqueue self-drop probe");
        observed_entry
            .recv()
            .expect("worker enters self-drop probe");

        drop(actor);
        release.send(()).expect("release worker self-drop");

        assert_eq!(ticket.wait(&health), Ok(43));
        observed_drop
            .recv_timeout(Duration::from_secs(1))
            .expect("worker exits and releases the port without self-join");
    }

    #[test]
    fn late_clean_mutation_finish_is_commit_uncertain_and_latches_health() {
        let health = ActorHealth::ready();
        let key = receipt_key();
        let digest = receipt_key_digest(&key);
        let deadline = Instant::now() + Duration::from_secs(1);
        let ticket =
            Ticket::<ReserveOutcome>::queued(deadline, TimeoutClass::Reserve(digest.clone()));
        assert!(ticket.try_begin(&health));

        ticket.finish_at(Err(ReceiptLedgerError::CapacityExceeded), deadline, &health);

        assert_eq!(
            ticket
                .wait(&health)
                .expect_err("late clean mutation completion has no authority"),
            ReceiptLedgerError::CommitUncertain {
                receipt_key_digest: digest,
            }
        );
        assert!(
            !health.is_ready(),
            "late mutation completion must fail-stop"
        );
    }

    #[test]
    fn late_clean_recover_finish_is_store_unavailable_and_latches_health() {
        let health = ActorHealth::ready();
        let deadline = Instant::now() + Duration::from_secs(1);
        let ticket = Ticket::<ReceiptState>::queued(deadline, TimeoutClass::Recover);
        assert!(ticket.try_begin(&health));

        ticket.finish_at(Err(ReceiptLedgerError::ReceiptNotFound), deadline, &health);

        assert_eq!(
            ticket
                .wait(&health)
                .expect_err("late clean recover completion has no authority"),
            ReceiptLedgerError::StoreUnavailable
        );
        assert!(!health.is_ready(), "late recover completion must fail-stop");
    }

    #[test]
    fn clean_finish_before_deadline_retains_completion_authority() {
        let health = ActorHealth::ready();
        let key = receipt_key();
        let deadline = Instant::now() + Duration::from_secs(1);
        let ticket = Ticket::<ReserveOutcome>::queued(
            deadline,
            TimeoutClass::Reserve(receipt_key_digest(&key)),
        );
        assert!(ticket.try_begin(&health));
        ticket.finish_at(
            Err(ReceiptLedgerError::CapacityExceeded),
            deadline - Duration::from_nanos(1),
            &health,
        );

        assert_eq!(
            ticket
                .wait(&health)
                .expect_err("pre-deadline completion owns the result"),
            ReceiptLedgerError::CapacityExceeded
        );
        assert!(health.is_ready(), "clean completion must not fail-stop");
    }

    #[test]
    fn permit_release_wakes_every_waiter_by_contract() {
        let source = include_str!("receipt_ledger_actor.rs");
        let drop_body = source
            .split("impl Drop for ")
            .nth(1)
            .expect("permit has a Drop implementation")
            .split("#[derive(Clone)]")
            .next()
            .expect("permit Drop body ends before the actor declaration");

        assert!(
            drop_body.contains("notify_all()"),
            "a timed-out waiter may consume notify_one and strand a live waiter"
        );
        assert!(
            !drop_body.contains("notify_one()"),
            "permit release must not select a single possibly expired waiter"
        );
    }

    #[test]
    fn reserve_cannot_bypass_the_actor_heavy_result_permit() {
        let calls = Arc::new(AtomicUsize::new(0));
        let actor = ReceiptLedgerActor::spawn(ErrorPort {
            error: ReceiptLedgerError::CapacityExceeded,
            calls: Arc::clone(&calls),
        });
        let held = actor
            .health
            .acquire_heavy_result_permit(Instant::now() + Duration::from_secs(1))
            .expect("hold the only actor heavy-result permit");

        let result = actor.reserve(
            receipt_key(),
            cutoff(),
            Instant::now() + Duration::from_millis(200),
        );

        assert_eq!(result, Err(ReceiptLedgerError::DeadlineExceeded));
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "reserve must acquire the common permit before it can reach the port"
        );
        drop(held);
    }

    #[test]
    fn recover_cannot_bypass_the_actor_heavy_result_permit() {
        let calls = Arc::new(AtomicUsize::new(0));
        let actor = ReceiptLedgerActor::spawn(ErrorPort {
            error: ReceiptLedgerError::ReceiptNotFound,
            calls: Arc::clone(&calls),
        });
        let held = actor
            .health
            .acquire_heavy_result_permit(Instant::now() + Duration::from_secs(1))
            .expect("hold the only actor heavy-result permit");

        let result = actor.recover(receipt_key(), Instant::now() + Duration::from_millis(200));

        assert_eq!(result, Err(ReceiptLedgerError::DeadlineExceeded));
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "recover must acquire the common permit before it can reach the port"
        );
        drop(held);
    }

    #[test]
    fn cancel_reservation_cannot_bypass_the_actor_heavy_result_permit() {
        let calls = Arc::new(AtomicUsize::new(0));
        let actor = ReceiptLedgerActor::spawn(ErrorPort {
            error: ReceiptLedgerError::CapacityExceeded,
            calls: Arc::clone(&calls),
        });
        let held = actor
            .health
            .acquire_heavy_result_permit(Instant::now() + Duration::from_secs(1))
            .expect("hold the only actor heavy-result permit");

        let result = actor.request_cancel_or_reserve(
            receipt_key(),
            1_000,
            Instant::now() + Duration::from_millis(200),
        );

        assert_eq!(result, Err(ReceiptLedgerError::DeadlineExceeded));
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "cancel can return an existing Direct winner and must acquire the common permit"
        );
        drop(held);
    }

    #[test]
    fn cancel_expiry_cannot_bypass_the_actor_heavy_result_permit() {
        let calls = Arc::new(AtomicUsize::new(0));
        let actor = ReceiptLedgerActor::spawn(ExpiryOutcomePort {
            outcome: CancelExpiryOutcome::Missing,
            calls: Arc::clone(&calls),
        });
        let held = actor
            .health
            .acquire_heavy_result_permit(Instant::now() + Duration::from_secs(1))
            .expect("hold the only actor heavy-result permit");

        let result = actor.expire_cancel_reserved(
            receipt_key(),
            ReceiptVersion::initial(),
            1,
            8_125,
            Instant::now() + Duration::from_millis(200),
        );

        assert_eq!(result, Err(ReceiptLedgerError::DeadlineExceeded));
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "expiry can return an existing Direct winner and must acquire the common permit"
        );
        drop(held);
    }

    #[test]
    fn abandoned_queued_direct_ticket_retains_permit_until_command_arc_drops() {
        let health = Arc::new(ActorHealth::ready());
        let permit = health
            .acquire_heavy_result_permit(Instant::now() + Duration::from_secs(1))
            .expect("acquire the only direct-publication permit");
        let key = receipt_key();
        let caller = Arc::new(
            Ticket::<CommittedDirectPublication>::queued_with_heavy_result_permit(
                Instant::now(),
                TimeoutClass::PublishDirectTerminal(receipt_key_digest(&key)),
                permit,
            ),
        );
        let queued_command = Arc::clone(&caller);

        assert_eq!(
            caller
                .wait(&health)
                .expect_err("queued caller abandons at its original deadline"),
            ReceiptLedgerError::DeadlineExceeded
        );
        drop(caller);
        assert!(matches!(
            health.acquire_heavy_result_permit(Instant::now() + Duration::from_millis(10)),
            Err(ReceiptLedgerError::DeadlineExceeded)
        ));

        drop(queued_command);
        assert!(health
            .acquire_heavy_result_permit(Instant::now() + Duration::from_secs(1))
            .is_ok());
    }

    #[test]
    fn finished_direct_ticket_retains_permit_until_result_is_consumed_and_last_arc_drops() {
        let health = Arc::new(ActorHealth::ready());
        let permit = health
            .acquire_heavy_result_permit(Instant::now() + Duration::from_secs(1))
            .expect("acquire the only direct-publication permit");
        let deadline = Instant::now() + Duration::from_secs(10);
        let key = receipt_key();
        let caller = Arc::new(
            Ticket::<CommittedDirectPublication>::queued_with_heavy_result_permit(
                deadline,
                TimeoutClass::PublishDirectTerminal(receipt_key_digest(&key)),
                permit,
            ),
        );
        let worker = Arc::clone(&caller);
        assert!(worker.try_begin(&health));
        worker.finish(Err(ReceiptLedgerError::TerminalMismatch), &health);
        drop(worker);

        assert!(matches!(
            health.acquire_heavy_result_permit(Instant::now() + Duration::from_millis(10)),
            Err(ReceiptLedgerError::DeadlineExceeded)
        ));
        assert_eq!(
            caller
                .wait(&health)
                .expect_err("consume the finished publication result"),
            ReceiptLedgerError::TerminalMismatch
        );
        assert!(matches!(
            health.acquire_heavy_result_permit(Instant::now() + Duration::from_millis(10)),
            Err(ReceiptLedgerError::DeadlineExceeded)
        ));

        drop(caller);
        assert!(health
            .acquire_heavy_result_permit(Instant::now() + Duration::from_secs(1))
            .is_ok());
    }

    #[test]
    fn direct_permit_waiter_wakes_with_store_unavailable_after_fail_stop() {
        let health = Arc::new(ActorHealth::ready());
        let held = health
            .acquire_heavy_result_permit(Instant::now() + Duration::from_secs(1))
            .expect("hold the only direct-publication permit");
        let waiter_health = Arc::clone(&health);
        let (started, started_wait) = mpsc::channel();
        let (result, result_wait) = mpsc::channel();
        let waiter = std::thread::spawn(move || {
            started.send(()).expect("report permit waiter start");
            let acquired =
                waiter_health.acquire_heavy_result_permit(Instant::now() + Duration::from_secs(10));
            result
                .send(acquired.map(|_permit| ()))
                .expect("report permit waiter result");
        });
        started_wait
            .recv_timeout(Duration::from_secs(1))
            .expect("permit waiter started");
        std::thread::sleep(Duration::from_millis(10));

        health.latch_recovery_required();
        assert_eq!(
            result_wait
                .recv_timeout(Duration::from_secs(2))
                .expect("fail-stop wakes the permit waiter"),
            Err(ReceiptLedgerError::StoreUnavailable)
        );
        waiter.join().expect("permit waiter does not panic");
        drop(held);
    }

    struct BlockingDirectTerminalPort {
        entered: Option<mpsc::Sender<Instant>>,
        release: mpsc::Receiver<()>,
        calls: Arc<AtomicUsize>,
    }

    impl ReceiptLedgerPort for BlockingDirectTerminalPort {
        fn reserve(
            &mut self,
            _key: ReceiptKey,
            _original_cutoff: OriginalCutoffDescriptor,
            _deadline: Instant,
        ) -> Result<ReserveOutcome, ReceiptLedgerError> {
            Err(ReceiptLedgerError::CapacityExceeded)
        }

        fn publish_direct_terminal(
            &mut self,
            _key: &ReceiptKey,
            _expected_version: ReceiptVersion,
            _terminal_epoch_ms: u64,
            _terminal: V5CanonicalTerminal,
            deadline: Instant,
        ) -> Result<CommittedDirectPublication, ReceiptLedgerError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if let Some(entered) = self.entered.take() {
                entered
                    .send(deadline)
                    .expect("report direct terminal entry");
                self.release.recv().expect("release direct terminal call");
            }
            Err(ReceiptLedgerError::TerminalMismatch)
        }

        fn request_cancel_or_reserve(
            &mut self,
            _key: ReceiptKey,
            _cancel_reserved_at_epoch_ms: u64,
            _deadline: Instant,
        ) -> Result<CancelResolution, ReceiptLedgerError> {
            Err(ReceiptLedgerError::CapacityExceeded)
        }

        fn expire_cancel_reserved(
            &mut self,
            _key: ReceiptKey,
            _expected_version: ReceiptVersion,
            _expected_mutation_sequence: u64,
            _observed_at_epoch_ms: u64,
            _deadline: Instant,
        ) -> Result<CancelExpiryOutcome, ReceiptLedgerError> {
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

    struct BlockingCancelReservationPort {
        entered: Option<mpsc::Sender<Instant>>,
        release: mpsc::Receiver<()>,
        calls: Arc<AtomicUsize>,
    }

    impl ReceiptLedgerPort for BlockingCancelReservationPort {
        fn reserve(
            &mut self,
            _key: ReceiptKey,
            _original_cutoff: OriginalCutoffDescriptor,
            _deadline: Instant,
        ) -> Result<ReserveOutcome, ReceiptLedgerError> {
            Err(ReceiptLedgerError::CapacityExceeded)
        }

        fn request_cancel_or_reserve(
            &mut self,
            _key: ReceiptKey,
            _cancel_reserved_at_epoch_ms: u64,
            deadline: Instant,
        ) -> Result<CancelResolution, ReceiptLedgerError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if let Some(entered) = self.entered.take() {
                entered
                    .send(deadline)
                    .expect("report cancel reservation entry");
                self.release
                    .recv()
                    .expect("release cancel reservation call");
            }
            Err(ReceiptLedgerError::CapacityExceeded)
        }

        fn expire_cancel_reserved(
            &mut self,
            _key: ReceiptKey,
            _expected_version: ReceiptVersion,
            _expected_mutation_sequence: u64,
            _observed_at_epoch_ms: u64,
            _deadline: Instant,
        ) -> Result<CancelExpiryOutcome, ReceiptLedgerError> {
            Err(ReceiptLedgerError::CapacityExceeded)
        }

        fn publish_direct_terminal(
            &mut self,
            _key: &ReceiptKey,
            _expected_version: ReceiptVersion,
            _terminal_epoch_ms: u64,
            _terminal: V5CanonicalTerminal,
            _deadline: Instant,
        ) -> Result<CommittedDirectPublication, ReceiptLedgerError> {
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

    struct ExpiryOutcomePort {
        outcome: CancelExpiryOutcome,
        calls: Arc<AtomicUsize>,
    }

    impl ReceiptLedgerPort for ExpiryOutcomePort {
        fn reserve(
            &mut self,
            _key: ReceiptKey,
            _original_cutoff: OriginalCutoffDescriptor,
            _deadline: Instant,
        ) -> Result<ReserveOutcome, ReceiptLedgerError> {
            Err(ReceiptLedgerError::CapacityExceeded)
        }

        fn request_cancel_or_reserve(
            &mut self,
            _key: ReceiptKey,
            _cancel_reserved_at_epoch_ms: u64,
            _deadline: Instant,
        ) -> Result<CancelResolution, ReceiptLedgerError> {
            Err(ReceiptLedgerError::CapacityExceeded)
        }

        fn expire_cancel_reserved(
            &mut self,
            _key: ReceiptKey,
            _expected_version: ReceiptVersion,
            _expected_mutation_sequence: u64,
            _observed_at_epoch_ms: u64,
            _deadline: Instant,
        ) -> Result<CancelExpiryOutcome, ReceiptLedgerError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.outcome.clone())
        }

        fn publish_direct_terminal(
            &mut self,
            _key: &ReceiptKey,
            _expected_version: ReceiptVersion,
            _terminal_epoch_ms: u64,
            _terminal: V5CanonicalTerminal,
            _deadline: Instant,
        ) -> Result<CommittedDirectPublication, ReceiptLedgerError> {
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

    struct BlockingCancelExpiryPort {
        entered: Option<mpsc::Sender<Instant>>,
        release: mpsc::Receiver<()>,
        calls: Arc<AtomicUsize>,
    }

    impl ReceiptLedgerPort for BlockingCancelExpiryPort {
        fn reserve(
            &mut self,
            _key: ReceiptKey,
            _original_cutoff: OriginalCutoffDescriptor,
            _deadline: Instant,
        ) -> Result<ReserveOutcome, ReceiptLedgerError> {
            Err(ReceiptLedgerError::CapacityExceeded)
        }

        fn request_cancel_or_reserve(
            &mut self,
            _key: ReceiptKey,
            _cancel_reserved_at_epoch_ms: u64,
            _deadline: Instant,
        ) -> Result<CancelResolution, ReceiptLedgerError> {
            Err(ReceiptLedgerError::CapacityExceeded)
        }

        fn expire_cancel_reserved(
            &mut self,
            _key: ReceiptKey,
            _expected_version: ReceiptVersion,
            _expected_mutation_sequence: u64,
            _observed_at_epoch_ms: u64,
            deadline: Instant,
        ) -> Result<CancelExpiryOutcome, ReceiptLedgerError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if let Some(entered) = self.entered.take() {
                entered.send(deadline).expect("report cancel expiry entry");
                self.release.recv().expect("release cancel expiry call");
            }
            Ok(CancelExpiryOutcome::Expired)
        }

        fn publish_direct_terminal(
            &mut self,
            _key: &ReceiptKey,
            _expected_version: ReceiptVersion,
            _terminal_epoch_ms: u64,
            _terminal: V5CanonicalTerminal,
            _deadline: Instant,
        ) -> Result<CommittedDirectPublication, ReceiptLedgerError> {
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

    #[test]
    fn running_direct_terminal_deadline_is_commit_uncertain_and_fail_stops_actor() {
        let (entered, entered_wait) = mpsc::channel();
        let (release, release_wait) = mpsc::channel();
        let calls = Arc::new(AtomicUsize::new(0));
        let actor = ReceiptLedgerActor::spawn(BlockingDirectTerminalPort {
            entered: Some(entered),
            release: release_wait,
            calls: Arc::clone(&calls),
        });
        let key = receipt_key();
        let expected_digest = receipt_key_digest(&key);
        let deadline = Instant::now() + Duration::from_millis(80);
        let first_actor = actor.clone();
        let first = std::thread::spawn(move || {
            first_actor.publish_direct_terminal(
                key,
                ReceiptVersion::initial(),
                1_234,
                cancelled_terminal(),
                deadline,
            )
        });
        assert_eq!(
            entered_wait
                .recv_timeout(Duration::from_secs(1))
                .expect("direct terminal entered the port"),
            deadline
        );

        assert_eq!(
            first
                .join()
                .expect("direct terminal caller does not panic")
                .expect_err("running write cannot report a clean timeout"),
            ReceiptLedgerError::CommitUncertain {
                receipt_key_digest: expected_digest,
            }
        );
        assert_eq!(
            actor
                .recover(receipt_key(), Instant::now() + Duration::from_secs(1))
                .expect_err("uncertain actor requires recovery"),
            ReceiptLedgerError::StoreUnavailable
        );
        release.send(()).expect("release uncertain fixture call");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn running_cancel_reservation_deadline_is_commit_uncertain_and_fail_stops_actor() {
        let (entered, entered_wait) = mpsc::channel();
        let (release, release_wait) = mpsc::channel();
        let calls = Arc::new(AtomicUsize::new(0));
        let actor = ReceiptLedgerActor::spawn(BlockingCancelReservationPort {
            entered: Some(entered),
            release: release_wait,
            calls: Arc::clone(&calls),
        });
        let key = receipt_key();
        let expected_digest = receipt_key_digest(&key);
        let deadline = Instant::now() + Duration::from_millis(80);
        let first_actor = actor.clone();
        let first =
            std::thread::spawn(move || first_actor.request_cancel_or_reserve(key, 1_000, deadline));
        assert_eq!(
            entered_wait
                .recv_timeout(Duration::from_secs(1))
                .expect("cancel reservation entered the port"),
            deadline
        );

        assert_eq!(
            first
                .join()
                .expect("cancel reservation caller does not panic")
                .expect_err("running cancel write cannot report a clean timeout"),
            ReceiptLedgerError::CommitUncertain {
                receipt_key_digest: expected_digest,
            }
        );
        assert_eq!(
            actor
                .request_cancel_or_reserve(
                    receipt_key(),
                    1_000,
                    Instant::now() + Duration::from_secs(1),
                )
                .expect_err("uncertain actor requires recovery"),
            ReceiptLedgerError::StoreUnavailable
        );
        release.send(()).expect("release uncertain fixture call");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn running_cancel_expiry_deadline_is_commit_uncertain_and_fail_stops_actor() {
        let (entered, entered_wait) = mpsc::channel();
        let (release, release_wait) = mpsc::channel();
        let calls = Arc::new(AtomicUsize::new(0));
        let actor = ReceiptLedgerActor::spawn(BlockingCancelExpiryPort {
            entered: Some(entered),
            release: release_wait,
            calls: Arc::clone(&calls),
        });
        let key = receipt_key();
        let expected_digest = receipt_key_digest(&key);
        let deadline = Instant::now() + Duration::from_millis(80);
        let first_actor = actor.clone();
        let first = std::thread::spawn(move || {
            first_actor.expire_cancel_reserved(key, ReceiptVersion::initial(), 1, 8_125, deadline)
        });
        assert_eq!(
            entered_wait
                .recv_timeout(Duration::from_secs(1))
                .expect("cancel expiry entered the port"),
            deadline
        );

        assert_eq!(
            first
                .join()
                .expect("cancel expiry caller does not panic")
                .expect_err("running expiry mutation cannot report a clean timeout"),
            ReceiptLedgerError::CommitUncertain {
                receipt_key_digest: expected_digest,
            }
        );
        assert_eq!(
            actor
                .recover(receipt_key(), Instant::now() + Duration::from_secs(1))
                .expect_err("uncertain actor requires recovery"),
            ReceiptLedgerError::StoreUnavailable
        );
        release.send(()).expect("release uncertain expiry call");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
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
            direct_calls: Arc::new(AtomicUsize::new(0)),
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
            direct_calls: Arc::new(AtomicUsize::new(0)),
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
    fn direct_terminal_panic_is_commit_uncertain_and_fail_stops_actor() {
        let actor = ReceiptLedgerActor::spawn(PanickingPort);
        let key = receipt_key();
        let expected_digest = receipt_key_digest(&key);

        assert_eq!(
            actor
                .publish_direct_terminal(
                    key,
                    ReceiptVersion::initial(),
                    1_234,
                    cancelled_terminal(),
                    Instant::now() + Duration::from_secs(1),
                )
                .expect_err("direct terminal panic cannot escape as a clean failure"),
            ReceiptLedgerError::CommitUncertain {
                receipt_key_digest: expected_digest,
            }
        );
        assert_eq!(
            actor
                .recover(receipt_key(), Instant::now() + Duration::from_secs(1))
                .expect_err("panicked actor requires recovery"),
            ReceiptLedgerError::StoreUnavailable
        );
    }

    #[test]
    fn cancel_reservation_panic_is_commit_uncertain_and_fail_stops_actor() {
        let actor = ReceiptLedgerActor::spawn(PanickingPort);
        let key = receipt_key();
        let expected_digest = receipt_key_digest(&key);

        assert_eq!(
            actor
                .request_cancel_or_reserve(key, 1_000, Instant::now() + Duration::from_secs(1),)
                .expect_err("cancel reservation panic cannot escape as a clean failure"),
            ReceiptLedgerError::CommitUncertain {
                receipt_key_digest: expected_digest,
            }
        );
        assert_eq!(
            actor
                .recover(receipt_key(), Instant::now() + Duration::from_secs(1))
                .expect_err("panicked actor requires recovery"),
            ReceiptLedgerError::StoreUnavailable
        );
    }

    #[test]
    fn cancel_expiry_panic_is_commit_uncertain_and_fail_stops_actor() {
        let actor = ReceiptLedgerActor::spawn(PanickingPort);
        let key = receipt_key();
        let expected_digest = receipt_key_digest(&key);

        assert_eq!(
            actor
                .expire_cancel_reserved(
                    key,
                    ReceiptVersion::initial(),
                    1,
                    8_125,
                    Instant::now() + Duration::from_secs(1),
                )
                .expect_err("cancel expiry panic cannot escape as a clean failure"),
            ReceiptLedgerError::CommitUncertain {
                receipt_key_digest: expected_digest,
            }
        );
        assert_eq!(
            actor
                .recover(receipt_key(), Instant::now() + Duration::from_secs(1))
                .expect_err("panicked actor requires recovery"),
            ReceiptLedgerError::StoreUnavailable
        );
    }

    #[test]
    fn clean_direct_terminal_mismatches_do_not_latch_actor() {
        let mismatches = [
            ReceiptLedgerError::ReceiptVersionMismatch {
                expected: ReceiptVersion::initial(),
                actual: ReceiptVersion::initial()
                    .checked_next()
                    .expect("second receipt version"),
            },
            ReceiptLedgerError::TerminalMismatch,
        ];

        for mismatch in mismatches {
            let calls = Arc::new(AtomicUsize::new(0));
            let actor = ReceiptLedgerActor::spawn(ErrorPort {
                error: mismatch.clone(),
                calls: Arc::clone(&calls),
            });
            for _ in 0..2 {
                assert_eq!(
                    actor
                        .publish_direct_terminal(
                            receipt_key(),
                            ReceiptVersion::initial(),
                            1_234,
                            cancelled_terminal(),
                            Instant::now() + Duration::from_secs(1),
                        )
                        .expect_err("fixture returns a clean direct terminal mismatch"),
                    mismatch
                );
            }
            assert_eq!(calls.load(Ordering::SeqCst), 2);
        }
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
    fn cancel_reservation_fail_stop_error_latches_but_clean_rejection_does_not() {
        let fail_stop_calls = Arc::new(AtomicUsize::new(0));
        let fail_stop = ReceiptLedgerActor::spawn(ErrorPort {
            error: ReceiptLedgerError::Storage {
                operation: "injected cancel reservation write",
                message: "failed".to_owned(),
            },
            calls: Arc::clone(&fail_stop_calls),
        });
        assert!(matches!(
            fail_stop.request_cancel_or_reserve(
                receipt_key(),
                1_000,
                Instant::now() + Duration::from_secs(1),
            ),
            Err(ReceiptLedgerError::Storage { .. })
        ));
        assert_eq!(
            fail_stop
                .request_cancel_or_reserve(
                    receipt_key(),
                    1_000,
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
                    .request_cancel_or_reserve(
                        receipt_key(),
                        1_000,
                        Instant::now() + Duration::from_secs(1),
                    )
                    .expect_err("capacity fixture rejects without fail-stop"),
                ReceiptLedgerError::CapacityExceeded
            );
        }
        assert_eq!(capacity_calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn clean_cancel_expiry_outcomes_and_version_mismatch_do_not_latch_actor() {
        let clean_outcomes = [
            CancelExpiryOutcome::Missing,
            CancelExpiryOutcome::NotDue(cancel_reserved_receipt()),
        ];
        for outcome in clean_outcomes {
            let calls = Arc::new(AtomicUsize::new(0));
            let actor = ReceiptLedgerActor::spawn(ExpiryOutcomePort {
                outcome: outcome.clone(),
                calls: Arc::clone(&calls),
            });
            for _ in 0..2 {
                assert_eq!(
                    actor
                        .expire_cancel_reserved(
                            receipt_key(),
                            ReceiptVersion::initial(),
                            1,
                            8_125,
                            Instant::now() + Duration::from_secs(1),
                        )
                        .expect("clean expiry resolution"),
                    outcome
                );
            }
            assert_eq!(calls.load(Ordering::SeqCst), 2);
        }

        let calls = Arc::new(AtomicUsize::new(0));
        let mismatch = ReceiptLedgerError::ReceiptVersionMismatch {
            expected: ReceiptVersion::initial(),
            actual: ReceiptVersion::new(2).expect("nonzero second version"),
        };
        let actor = ReceiptLedgerActor::spawn(ErrorPort {
            error: mismatch.clone(),
            calls: Arc::clone(&calls),
        });
        for _ in 0..2 {
            assert_eq!(
                actor
                    .expire_cancel_reserved(
                        receipt_key(),
                        ReceiptVersion::initial(),
                        1,
                        8_125,
                        Instant::now() + Duration::from_secs(1),
                    )
                    .expect_err("fixture reports an exact version mismatch"),
                mismatch
            );
        }
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn cancel_expiry_requires_reopen_error_latches_actor() {
        let calls = Arc::new(AtomicUsize::new(0));
        let actor = ReceiptLedgerActor::spawn(ErrorPort {
            error: ReceiptLedgerError::Storage {
                operation: "injected cancel expiry write",
                message: "failed".to_owned(),
            },
            calls: Arc::clone(&calls),
        });

        assert!(matches!(
            actor.expire_cancel_reserved(
                receipt_key(),
                ReceiptVersion::initial(),
                1,
                8_125,
                Instant::now() + Duration::from_secs(1),
            ),
            Err(ReceiptLedgerError::Storage { .. })
        ));
        assert_eq!(
            actor
                .expire_cancel_reserved(
                    receipt_key(),
                    ReceiptVersion::initial(),
                    1,
                    8_125,
                    Instant::now() + Duration::from_secs(1),
                )
                .expect_err("storage failure latches actor"),
            ReceiptLedgerError::StoreUnavailable
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
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

        assert_eq!(
            actor
                .expire_cancel_reserved(
                    receipt_key(),
                    ReceiptVersion::initial(),
                    1,
                    8_125,
                    Instant::now(),
                )
                .expect_err("expired cancel expiry is rejected before enqueue"),
            ReceiptLedgerError::DeadlineExceeded
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);

        assert_eq!(
            actor
                .request_cancel_or_reserve(receipt_key(), 1_000, Instant::now())
                .expect_err("expired cancel reservation is rejected before enqueue"),
            ReceiptLedgerError::DeadlineExceeded
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);

        assert_eq!(
            actor
                .publish_direct_terminal(
                    receipt_key(),
                    ReceiptVersion::initial(),
                    1_234,
                    cancelled_terminal(),
                    Instant::now(),
                )
                .expect_err("expired direct terminal is rejected before enqueue"),
            ReceiptLedgerError::DeadlineExceeded
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn expired_direct_terminal_queued_behind_running_reserve_never_reaches_the_port() {
        let (entered, entered_wait) = mpsc::channel();
        let (release, release_wait) = mpsc::channel();
        let reserve_calls = Arc::new(AtomicUsize::new(0));
        let direct_calls = Arc::new(AtomicUsize::new(0));
        let actor = ReceiptLedgerActor::spawn(BlockingPort {
            entered: Some(entered),
            release: release_wait,
            calls: Arc::clone(&reserve_calls),
            direct_calls: Arc::clone(&direct_calls),
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
            .expect("reserve entered the port");

        assert_eq!(
            actor
                .publish_direct_terminal(
                    receipt_key(),
                    ReceiptVersion::initial(),
                    1_234,
                    cancelled_terminal(),
                    Instant::now() + Duration::from_millis(40),
                )
                .expect_err("queued direct terminal must expire"),
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
                .recover(receipt_key(), Instant::now() + Duration::from_secs(1))
                .expect_err("recover is a worker barrier"),
            ReceiptLedgerError::ReceiptNotFound
        );
        assert_eq!(reserve_calls.load(Ordering::SeqCst), 1);
        assert_eq!(direct_calls.load(Ordering::SeqCst), 0);
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

        let cancel_deadline = Instant::now() + Duration::from_secs(1);
        assert_eq!(
            actor
                .request_cancel_or_reserve(receipt_key(), 1_000, cancel_deadline)
                .expect_err("fixture rejects cancel reservation"),
            ReceiptLedgerError::CapacityExceeded
        );
        assert_eq!(
            seen_wait
                .recv()
                .expect("cancel reservation deadline observation"),
            (cancel_deadline, 2)
        );

        let expiry_deadline = Instant::now() + Duration::from_secs(1);
        assert_eq!(
            actor
                .expire_cancel_reserved(
                    receipt_key(),
                    ReceiptVersion::initial(),
                    1,
                    8_125,
                    expiry_deadline,
                )
                .expect_err("fixture rejects cancel expiry"),
            ReceiptLedgerError::CapacityExceeded
        );
        assert_eq!(
            seen_wait
                .recv()
                .expect("cancel expiry deadline observation"),
            (expiry_deadline, 3)
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
            (recover_deadline, 4)
        );

        let direct_deadline = Instant::now() + Duration::from_secs(1);
        assert_eq!(
            actor
                .publish_direct_terminal(
                    receipt_key(),
                    ReceiptVersion::initial(),
                    1_234,
                    cancelled_terminal(),
                    direct_deadline,
                )
                .expect_err("fixture rejects direct publication"),
            ReceiptLedgerError::TerminalMismatch
        );
        assert_eq!(
            seen_wait.recv().expect("direct deadline observation"),
            (direct_deadline, 5)
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
    fn heavy_result_permit_waiter_wakes_after_running_timeout_latches_actor() {
        let (entered, entered_wait) = mpsc::channel();
        let (release, release_wait) = mpsc::channel();
        let calls = Arc::new(AtomicUsize::new(0));
        let direct_calls = Arc::new(AtomicUsize::new(0));
        let actor = ReceiptLedgerActor::spawn(BlockingPort {
            entered: Some(entered),
            release: release_wait,
            calls: Arc::clone(&calls),
            direct_calls: Arc::clone(&direct_calls),
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

        let waiting_actor = actor.clone();
        let waiter = std::thread::spawn(move || {
            waiting_actor.publish_direct_terminal(
                receipt_key(),
                ReceiptVersion::initial(),
                1_234,
                cancelled_terminal(),
                Instant::now() + Duration::from_millis(300),
            )
        });
        assert!(matches!(
            first.join().expect("first caller does not panic"),
            Err(ReceiptLedgerError::CommitUncertain { .. })
        ));
        assert_eq!(
            waiter
                .join()
                .expect("permit waiter does not panic")
                .expect_err("permit waiter wakes after the fail-stop latch"),
            ReceiptLedgerError::StoreUnavailable
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            direct_calls.load(Ordering::SeqCst),
            0,
            "the waiting publication never reaches the port"
        );
        release.send(()).expect("release first reserve");
    }
}
