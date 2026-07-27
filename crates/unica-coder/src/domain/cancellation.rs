pub use unica_format_core::ports::OperationCancellation as CancellationToken;

pub const CANCELLED_PREFIX: &str = "cancelled:";

pub fn cancelled_error(detail: impl AsRef<str>) -> String {
    format!("{CANCELLED_PREFIX} {}", detail.as_ref())
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
