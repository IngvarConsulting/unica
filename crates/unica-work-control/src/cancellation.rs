use std::sync::{
    atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
    Arc, Mutex,
};

pub const CANCELLED_PREFIX: &str = "cancelled:";

pub fn cancelled_error(detail: impl AsRef<str>) -> String {
    format!("{CANCELLED_PREFIX} {}", detail.as_ref())
}

/// Proven execution facts affect terminal classification, never observation
/// of the caller's original cancellation signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FactualOutcomeAuthority {
    Cancellable,
    StartedExternalProcess,
    CommittedWorkspaceApply,
}

/// How many times a token was cancelled: the first cancel interrupts work, a
/// repeated one tells work that waits after the interrupt to stop waiting.
/// [`HARD_CANCEL`] stops at once; a cancel by a person never reaches it.
const HARD_CANCEL: u8 = u8::MAX;

#[derive(Debug, Clone)]
pub struct CancellationToken {
    own_signal: Arc<AtomicU8>,
    observed_signals: Arc<[Arc<AtomicU8>]>,
    /// Processes of this token that were interrupted by a cancel and still
    /// run until their own answer. Shared with linked children: the task of
    /// the token shows that its cancel waits.
    awaiting: Arc<AtomicUsize>,
    dispatch_gate: Arc<Mutex<()>>,
    protected_started: Arc<AtomicBool>,
    factual_outcome: Arc<AtomicU8>,
    protect_on_spawn: bool,
}

impl Default for CancellationToken {
    fn default() -> Self {
        let own_signal = Arc::new(AtomicU8::new(0));
        Self {
            own_signal: Arc::clone(&own_signal),
            observed_signals: Arc::from([own_signal]),
            awaiting: Arc::new(AtomicUsize::new(0)),
            dispatch_gate: Arc::new(Mutex::new(())),
            protected_started: Arc::new(AtomicBool::new(false)),
            factual_outcome: Arc::new(AtomicU8::new(FactualOutcomeAuthority::Cancellable as u8)),
            protect_on_spawn: false,
        }
    }
}

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        let _gate = self
            .dispatch_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut count = self.own_signal.load(Ordering::Acquire);
        while count < HARD_CANCEL - 1 {
            match self.own_signal.compare_exchange_weak(
                count,
                count + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(current) => count = current,
            }
        }
    }

    /// Cancel that waits for nothing: the daemon stops, so work interrupted
    /// by this token ends at once instead of waiting for its own answer.
    pub fn cancel_now(&self) {
        let _gate = self
            .dispatch_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.own_signal.store(HARD_CANCEL, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel_requests() > 0
    }

    /// How many cancels this token has seen, its own or its parents': the
    /// most of them.
    pub fn cancel_requests(&self) -> u8 {
        self.observed_signals
            .iter()
            .map(|signal| signal.load(Ordering::Acquire))
            .max()
            .unwrap_or(0)
    }

    /// Whether the token was cancelled with [`Self::cancel_now`].
    pub fn is_cancelled_now(&self) -> bool {
        self.cancel_requests() == HARD_CANCEL
    }

    /// A process of this token was interrupted by a cancel and goes on until
    /// its own answer; the guard marks the end of that wait.
    pub fn awaiting_interrupted_work(&self) -> AwaitingInterruptedWork {
        self.awaiting.fetch_add(1, Ordering::AcqRel);
        AwaitingInterruptedWork {
            awaiting: Arc::clone(&self.awaiting),
        }
    }

    /// Whether some process of this token, or of a linked child, waits for
    /// its own answer after a cancel interrupted it.
    pub fn awaits_interrupted_work(&self) -> bool {
        self.awaiting.load(Ordering::Acquire) > 0
    }

    pub fn linked_child(&self) -> Self {
        let own_signal = Arc::new(AtomicU8::new(0));
        let mut observed_signals = self.observed_signals.to_vec();
        observed_signals.push(Arc::clone(&own_signal));
        Self {
            own_signal,
            observed_signals: Arc::from(observed_signals),
            awaiting: Arc::clone(&self.awaiting),
            dispatch_gate: Arc::clone(&self.dispatch_gate),
            protected_started: Arc::new(AtomicBool::new(false)),
            factual_outcome: Arc::new(AtomicU8::new(FactualOutcomeAuthority::Cancellable as u8)),
            protect_on_spawn: false,
        }
    }

    /// Request a non-abortable child only after the process has crossed its
    /// final cancellation gate. The original token records that dispatch
    /// boundary for the receipt publisher.
    pub fn protect_process_on_spawn(&self) -> Self {
        let mut protected = self.clone();
        protected.protect_on_spawn = true;
        protected
    }

    pub fn spawn_with_gate<T>(
        &self,
        spawn: impl FnOnce() -> Result<T, String>,
    ) -> Result<(T, Self), String> {
        if !self.protect_on_spawn {
            return spawn().map(|child| (child, self.clone()));
        }
        let _gate = self
            .dispatch_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.is_cancelled() {
            return Err(cancelled_error("cancelled before protected process launch"));
        }
        let child = spawn()?;
        self.protected_started.store(true, Ordering::Release);
        Ok((child, Self::new()))
    }

    /// Transfer an already validated external effect under the same gate as
    /// cancellation. A failed release leaves ownership with the caller.
    pub fn handoff_with_gate(
        &self,
        release: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        let _gate = self
            .dispatch_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.is_cancelled() {
            return Err(cancelled_error("cancelled before process handoff"));
        }
        release()?;
        self.protected_started.store(true, Ordering::Release);
        Ok(())
    }

    /// Records an already confirmed actor publication. This is unconditional:
    /// cancellation cannot erase an effect that has actually committed.
    pub fn record_committed_apply(&self) {
        self.factual_outcome.store(
            FactualOutcomeAuthority::CommittedWorkspaceApply as u8,
            Ordering::Release,
        );
    }

    pub fn factual_outcome_authority(&self) -> FactualOutcomeAuthority {
        if self.factual_outcome.load(Ordering::Acquire)
            == FactualOutcomeAuthority::CommittedWorkspaceApply as u8
        {
            FactualOutcomeAuthority::CommittedWorkspaceApply
        } else if self.protected_process_started() {
            FactualOutcomeAuthority::StartedExternalProcess
        } else {
            FactualOutcomeAuthority::Cancellable
        }
    }

    pub fn protected_process_started(&self) -> bool {
        self.protected_started.load(Ordering::Acquire)
    }
}

/// See [`CancellationToken::awaiting_interrupted_work`].
#[derive(Debug)]
pub struct AwaitingInterruptedWork {
    awaiting: Arc<AtomicUsize>,
}

impl Drop for AwaitingInterruptedWork {
    fn drop(&mut self) {
        self.awaiting.fetch_sub(1, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use super::{cancelled_error, CancellationToken, CANCELLED_PREFIX};
    use std::{sync::mpsc, thread, time::Duration};

    #[test]
    fn clones_observe_cancellation() {
        let first = CancellationToken::new();
        let second = first.clone();
        assert!(!second.is_cancelled());
        first.cancel();
        assert!(second.is_cancelled());
    }

    #[test]
    fn linked_child_observes_parent_and_keeps_local_cancellation_local() {
        let parent = CancellationToken::new();
        let child = parent.linked_child();

        child.cancel();
        assert!(child.is_cancelled());
        assert!(!parent.is_cancelled());

        let sibling = parent.linked_child();
        parent.cancel();
        assert!(sibling.is_cancelled());
    }

    /// A repeated cancel is counted: work that waits after the first one
    /// learns that it should stop waiting. Linked children see the count of
    /// their parents. The hard cancel of a stopping daemon is told apart.
    #[test]
    fn repeated_cancels_are_counted_and_seen_by_linked_children() {
        let parent = CancellationToken::new();
        let child = parent.linked_child();
        assert_eq!(child.cancel_requests(), 0);
        parent.cancel();
        assert_eq!(child.cancel_requests(), 1);
        parent.cancel();
        assert_eq!(child.cancel_requests(), 2);
        assert!(!child.is_cancelled_now());
        for _ in 0..300 {
            parent.cancel();
        }
        assert!(!child.is_cancelled_now());
        parent.cancel_now();
        assert!(child.is_cancelled_now());
        assert!(child.is_cancelled());
    }

    /// Waiting after an interrupt is seen from the task's token, also when the
    /// process runs under a linked child, until the last wait ends.
    #[test]
    fn waiting_after_an_interrupt_is_seen_until_the_last_wait_ends() {
        let task = CancellationToken::new();
        let child = task.linked_child();
        assert!(!task.awaits_interrupted_work());
        let first = child.awaiting_interrupted_work();
        let second = task.awaiting_interrupted_work();
        assert!(task.awaits_interrupted_work());
        drop(first);
        assert!(task.awaits_interrupted_work());
        drop(second);
        assert!(!task.awaits_interrupted_work());
    }

    #[test]
    fn protected_spawn_has_one_ordered_cancellation_boundary() {
        let cancelled = CancellationToken::new();
        cancelled.cancel();
        assert!(cancelled
            .protect_process_on_spawn()
            .spawn_with_gate(|| Ok(()))
            .is_err());
        assert!(!cancelled.protected_process_started());

        let started = CancellationToken::new();
        let (_, child) = started
            .protect_process_on_spawn()
            .spawn_with_gate(|| Ok(()))
            .unwrap();
        started.cancel();
        assert!(started.protected_process_started());
        assert!(!child.is_cancelled());
    }

    /// rule: INV.RUNTIME.RUNNER-MUTATION-CANCELLATION
    #[test]
    fn cancellation_waits_for_in_flight_protected_dispatch() {
        let token = CancellationToken::new();
        let protected = token.protect_process_on_spawn();
        let (entered_tx, entered_rx) = mpsc::channel();
        let (finish_tx, finish_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            protected
                .spawn_with_gate(|| {
                    entered_tx.send(()).unwrap();
                    finish_rx.recv().unwrap();
                    Ok(())
                })
                .unwrap()
                .1
        });
        entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();

        let cancel_token = token.clone();
        let (cancelled_tx, cancelled_rx) = mpsc::channel();
        let canceller = thread::spawn(move || {
            cancel_token.cancel();
            cancelled_tx.send(()).unwrap();
        });
        assert!(cancelled_rx
            .recv_timeout(Duration::from_millis(20))
            .is_err());
        finish_tx.send(()).unwrap();
        let child = worker.join().unwrap();
        cancelled_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        canceller.join().unwrap();
        assert!(token.protected_process_started());
        assert!(token.is_cancelled());
        assert!(!child.is_cancelled());
    }

    #[test]
    fn verified_handoff_keeps_ownership_on_cancel_or_release_failure() {
        let cancelled = CancellationToken::new();
        cancelled.cancel();
        assert!(cancelled
            .handoff_with_gate(|| panic!("cancelled handoff must not release"))
            .is_err());
        assert!(!cancelled.protected_process_started());
        let failed = CancellationToken::new();
        assert!(failed
            .handoff_with_gate(|| Err("release failed".into()))
            .is_err());
        assert!(!failed.protected_process_started());
        let released = CancellationToken::new();
        released.handoff_with_gate(|| Ok(())).unwrap();
        released.cancel();
        assert!(released.protected_process_started());
    }

    #[test]
    fn committed_apply_fact_survives_cancel_without_protecting_a_process_or_child() {
        use super::FactualOutcomeAuthority;

        let original = CancellationToken::new();
        let clone = original.clone();
        let earlier_child = original.linked_child();
        original.cancel();
        original.record_committed_apply();
        assert!(original.is_cancelled());
        assert!(clone.is_cancelled());
        assert!(!original.protected_process_started());
        assert_eq!(
            original.factual_outcome_authority(),
            FactualOutcomeAuthority::CommittedWorkspaceApply
        );
        assert_eq!(
            clone.factual_outcome_authority(),
            FactualOutcomeAuthority::CommittedWorkspaceApply
        );
        assert_eq!(
            earlier_child.factual_outcome_authority(),
            FactualOutcomeAuthority::Cancellable
        );
        let later_child = original.linked_child();
        assert!(later_child.is_cancelled());
        assert_eq!(
            later_child.factual_outcome_authority(),
            FactualOutcomeAuthority::Cancellable
        );
        let independent = CancellationToken::new();
        let child = independent.linked_child();
        child.record_committed_apply();
        assert_eq!(
            independent.factual_outcome_authority(),
            FactualOutcomeAuthority::Cancellable
        );
    }

    #[test]
    fn cancellation_errors_have_stable_prefix() {
        assert!(cancelled_error("operation stopped").starts_with(CANCELLED_PREFIX));
    }
}
