use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

pub const CANCELLED_PREFIX: &str = "cancelled:";

pub fn cancelled_error(detail: impl AsRef<str>) -> String {
    format!("{CANCELLED_PREFIX} {}", detail.as_ref())
}

#[derive(Debug, Clone)]
pub struct CancellationToken {
    own_signal: Arc<AtomicBool>,
    observed_signals: Arc<[Arc<AtomicBool>]>,
}

impl Default for CancellationToken {
    fn default() -> Self {
        let own_signal = Arc::new(AtomicBool::new(false));
        Self {
            own_signal: Arc::clone(&own_signal),
            observed_signals: Arc::from([own_signal]),
        }
    }
}

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{cancelled_error, CancellationToken, CANCELLED_PREFIX};

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
    fn cancellation_errors_have_stable_prefix() {
        assert!(cancelled_error("operation stopped").starts_with(CANCELLED_PREFIX));
    }
}
