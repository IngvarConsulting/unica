use std::time::Instant;

/// Monotonic clock observed by an invocation owner.
pub trait Clock: Send + Sync {
    fn now(&self) -> Instant;
}
