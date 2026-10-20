use crate::operation_deadline::OperationDeadline;
use std::time::{Duration, Instant};

// Relative finite budgets remain representable even when their endpoint does
// not fit in Instant. They never become absence through checked arithmetic.
#[derive(Debug, Clone, Copy)]
struct RelativeProviderDeadline {
    started_at: Instant,
    budget: Duration,
}

#[derive(Debug, Clone, Copy)]
pub struct ProviderDeadline {
    deadline: OperationDeadline<RelativeProviderDeadline>,
    #[cfg(any(test, feature = "test-support"))]
    now: fn() -> Instant,
}

impl PartialEq for ProviderDeadline {
    fn eq(&self, other: &Self) -> bool {
        match (self.deadline, other.deadline) {
            (OperationDeadline::NoDeadline, OperationDeadline::NoDeadline) => true,
            (OperationDeadline::Finite(left), OperationDeadline::Finite(right)) => {
                if left.started_at <= right.started_at {
                    left.budget.checked_sub(right.budget)
                        == right.started_at.checked_duration_since(left.started_at)
                } else {
                    right.budget.checked_sub(left.budget)
                        == left.started_at.checked_duration_since(right.started_at)
                }
            }
            _ => false,
        }
    }
}

impl Eq for ProviderDeadline {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderDeadlineError {
    Unsupported,
}

impl std::fmt::Display for ProviderDeadlineError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .write_str("unsupported: this service requires an explicit finite operation deadline")
    }
}

impl std::error::Error for ProviderDeadlineError {}

impl ProviderDeadline {
    pub const fn no_deadline() -> Self {
        Self {
            deadline: OperationDeadline::NoDeadline,
            #[cfg(any(test, feature = "test-support"))]
            now: Instant::now,
        }
    }

    pub fn new(deadline: Instant) -> Self {
        let started_at = Instant::now();
        let budget = deadline
            .checked_duration_since(started_at)
            .unwrap_or(Duration::ZERO);
        Self::from_started_at(started_at, budget)
    }

    pub fn from_budget(budget: Duration) -> Self {
        Self::from_started_at(Instant::now(), budget)
    }

    pub fn from_started_at(started_at: Instant, budget: Duration) -> Self {
        Self {
            deadline: OperationDeadline::Finite(RelativeProviderDeadline { started_at, budget }),
            #[cfg(any(test, feature = "test-support"))]
            now: Instant::now,
        }
    }

    pub fn earlier(self, other: Self) -> Self {
        match (self.deadline, other.deadline) {
            (OperationDeadline::NoDeadline, _) => other,
            (_, OperationDeadline::NoDeadline) => self,
            (OperationDeadline::Finite(left), OperationDeadline::Finite(right)) => {
                let (first, first_finite, second, second_finite) =
                    if left.started_at <= right.started_at {
                        (self, left, other, right)
                    } else {
                        (other, right, self, left)
                    };
                let second_end = second_finite.budget.saturating_add(
                    second_finite
                        .started_at
                        .duration_since(first_finite.started_at),
                );
                if first_finite.budget <= second_end {
                    first
                } else {
                    second
                }
            }
        }
    }

    /// None means no operation cutoff; an elapsed finite deadline is Some(ZERO).
    pub fn remaining(self) -> Option<Duration> {
        let OperationDeadline::Finite(finite) = self.deadline else {
            return None;
        };
        #[cfg(any(test, feature = "test-support"))]
        let now = (self.now)();
        #[cfg(not(any(test, feature = "test-support")))]
        let now = Instant::now();
        let elapsed = now
            .checked_duration_since(finite.started_at)
            .unwrap_or(Duration::ZERO);
        Some(finite.budget.checked_sub(elapsed).unwrap_or(Duration::ZERO))
    }

    pub fn is_elapsed(self) -> bool {
        self.remaining()
            .is_some_and(|remaining| remaining.is_zero())
    }

    /// Legacy services accept only finite budgets. Refuse absence before dispatch.
    pub fn finite_remaining(self) -> Result<Duration, ProviderDeadlineError> {
        self.remaining().ok_or(ProviderDeadlineError::Unsupported)
    }

    /// A polling interval never introduces an operation cutoff for NoDeadline.
    pub fn wait_slice(self, pacing: Duration) -> Duration {
        self.remaining()
            .map_or(pacing, |remaining| remaining.min(pacing))
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_clock(deadline: Instant, now: fn() -> Instant) -> Self {
        let started_at = now();
        let budget = deadline
            .checked_duration_since(started_at)
            .unwrap_or(Duration::ZERO);
        Self {
            deadline: OperationDeadline::Finite(RelativeProviderDeadline { started_at, budget }),
            now,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn provider_absence_is_distinct_from_expired_and_contracts_with_finite_deadlines() {
        let absent = ProviderDeadline::no_deadline();
        let start = Instant::now();
        let finite = ProviderDeadline::from_started_at(start, Duration::from_secs(30));
        let expired = ProviderDeadline::from_started_at(start, Duration::ZERO);
        assert_eq!(absent.remaining(), None);
        assert_eq!(
            absent.finite_remaining(),
            Err(ProviderDeadlineError::Unsupported)
        );
        assert_eq!(expired.remaining(), Some(Duration::ZERO));
        assert_eq!(expired.finite_remaining(), Ok(Duration::ZERO));
        assert_ne!(absent, expired);
        assert_eq!(absent, ProviderDeadline::no_deadline());
        assert_eq!(absent.earlier(finite), finite);
        assert_eq!(finite.earlier(absent), finite);
        assert_eq!(absent.earlier(expired), expired);
        assert_eq!(absent.earlier(absent), absent);
    }

    #[test]
    fn provider_finite_extreme_budgets_preserve_remaining_without_instant_overflow() {
        static OBSERVED: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        fn observed_now() -> Instant {
            *OBSERVED.get().unwrap()
        }
        let start = Instant::now();
        OBSERVED.set(start + Duration::from_secs(10)).unwrap();
        for budget in [Duration::from_secs(i64::MAX as u64), Duration::MAX] {
            let mut finite = ProviderDeadline::from_started_at(start, budget);
            finite.now = observed_now;
            assert_eq!(finite.remaining(), Some(budget - Duration::from_secs(10)));
            assert_ne!(finite, ProviderDeadline::no_deadline());
            assert_eq!(finite.earlier(ProviderDeadline::no_deadline()), finite);
        }
    }

    #[test]
    fn provider_relative_equality_compares_the_same_endpoint_with_different_starts() {
        let start = Instant::now();
        let first = ProviderDeadline::from_started_at(start, Duration::from_secs(30));
        let same = ProviderDeadline::from_started_at(
            start + Duration::from_secs(10),
            Duration::from_secs(20),
        );
        let later = ProviderDeadline::from_started_at(
            start + Duration::from_secs(10),
            Duration::from_secs(21),
        );
        assert_eq!(first, same);
        assert_ne!(first, later);
        assert_eq!(later.earlier(first), first);
        assert_eq!(first.earlier(later), first);
    }

    #[test]
    fn provider_deadline_preserves_its_equality_contract() {
        fn assert_eq_contract<T: Eq>() {}

        assert_eq_contract::<ProviderDeadline>();
        let deadline = Instant::now() + Duration::from_secs(1);
        assert_eq!(
            ProviderDeadline::new(deadline),
            ProviderDeadline::new(deadline)
        );
        let start = Instant::now();
        let earlier = ProviderDeadline::from_started_at(start, Duration::from_millis(20));
        let later = ProviderDeadline::from_started_at(
            start + Duration::from_millis(10),
            Duration::from_millis(15),
        );
        // The later deadline has the smaller budget; selecting by budget alone
        // would replenish the earlier invocation's remaining lifetime.
        assert_eq!(earlier.earlier(later), earlier);
        assert_eq!(later.earlier(earlier), earlier);
        let expired = ProviderDeadline::from_started_at(start, Duration::ZERO);
        assert_eq!(earlier.earlier(expired), expired);
    }
}
