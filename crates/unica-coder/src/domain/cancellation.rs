use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

pub const CANCELLED_PREFIX: &str = "cancelled:";

pub fn cancelled_error(detail: impl AsRef<str>) -> String {
    format!("{CANCELLED_PREFIX} {}", detail.as_ref())
}

#[derive(Debug, Clone)]
pub struct CancellationToken {
    own_signal: Arc<AtomicBool>,
    observed_signals: Arc<[Arc<AtomicBool>]>,
    dispatch_gate: Arc<Mutex<()>>,
    protected_started: Arc<AtomicBool>,
    protect_on_spawn: bool,
}

impl Default for CancellationToken {
    fn default() -> Self {
        let own_signal = Arc::new(AtomicBool::new(false));
        Self {
            own_signal: Arc::clone(&own_signal),
            observed_signals: Arc::from([own_signal]),
            dispatch_gate: Arc::new(Mutex::new(())),
            protected_started: Arc::new(AtomicBool::new(false)),
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
        self.own_signal.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.observed_signals
            .iter()
            .any(|signal| signal.load(Ordering::Acquire))
    }

    pub fn linked_child(&self) -> Self {
        let own_signal = Arc::new(AtomicBool::new(false));
        let mut observed_signals = self.observed_signals.to_vec();
        observed_signals.push(Arc::clone(&own_signal));
        Self {
            own_signal,
            observed_signals: Arc::from(observed_signals),
            dispatch_gate: Arc::clone(&self.dispatch_gate),
            protected_started: Arc::new(AtomicBool::new(false)),
            protect_on_spawn: false,
        }
    }

    /// Request a non-abortable child only after the process has crossed its
    /// final cancellation gate. The original token records that dispatch
    /// boundary for the receipt publisher.
    pub(crate) fn protect_process_on_spawn(&self) -> Self {
        let mut protected = self.clone();
        protected.protect_on_spawn = true;
        protected
    }

    pub(crate) fn spawn_with_gate<T>(
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

    pub(crate) fn protected_process_started(&self) -> bool {
        self.protected_started.load(Ordering::Acquire)
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
    fn cancellation_errors_have_stable_prefix() {
        assert!(cancelled_error("operation stopped").starts_with(CANCELLED_PREFIX));
    }
}
