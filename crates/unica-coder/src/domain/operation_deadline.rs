use std::time::{Duration, Instant};

/// An operation either has an explicit monotonic deadline or has no time limit.
/// Absence is not an expired deadline and is never encoded as a large duration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OperationDeadline<T = Instant> {
    NoDeadline,
    Finite(T),
}

impl OperationDeadline<Instant> {
    pub(crate) fn remaining_at(self, now: Instant) -> Option<Duration> {
        match self {
            Self::NoDeadline => None,
            Self::Finite(deadline) => Some(deadline.saturating_duration_since(now)),
        }
    }

    pub(crate) fn is_elapsed_at(self, now: Instant) -> bool {
        matches!(self, Self::Finite(deadline) if now >= deadline)
    }

    pub(crate) fn as_instant(self) -> Option<Instant> {
        match self {
            Self::NoDeadline => None,
            Self::Finite(deadline) => Some(deadline),
        }
    }
}

impl From<Instant> for OperationDeadline {
    fn from(deadline: Instant) -> Self {
        Self::Finite(deadline)
    }
}

impl From<Option<Instant>> for OperationDeadline {
    fn from(deadline: Option<Instant>) -> Self {
        deadline.map_or(Self::NoDeadline, Self::Finite)
    }
}

#[cfg(test)]
mod tests {
    use super::OperationDeadline;
    use std::time::{Duration, Instant};

    #[test]
    fn absence_and_elapsed_finite_deadline_are_distinct() {
        let now = Instant::now();
        let later = now + Duration::from_secs(10);
        let finite = OperationDeadline::from(now);
        assert_eq!(finite.remaining_at(later), Some(Duration::ZERO));
        assert!(finite.is_elapsed_at(later));
        assert_eq!(OperationDeadline::NoDeadline.remaining_at(later), None);
        assert!(!OperationDeadline::NoDeadline.is_elapsed_at(later));
        assert_eq!(finite.as_instant(), Some(now));
        assert_eq!(OperationDeadline::NoDeadline.as_instant(), None);
        assert_eq!(OperationDeadline::from(None), OperationDeadline::NoDeadline);
        assert_eq!(OperationDeadline::from(Some(now)), finite);
    }
}
