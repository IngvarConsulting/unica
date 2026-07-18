use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

pub const CANCELLED_PREFIX: &str = "cancelled:";

pub fn cancelled_error(detail: impl AsRef<str>) -> String {
    format!("{CANCELLED_PREFIX} {}", detail.as_ref())
}

#[derive(Debug, Clone, Default)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
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
    fn cancellation_errors_have_stable_prefix() {
        assert!(cancelled_error("operation stopped").starts_with(CANCELLED_PREFIX));
    }
}
