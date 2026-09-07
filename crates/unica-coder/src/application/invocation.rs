//! The daemon-side response budget shared by every daemon stage, and the
//! canonical argument digest a receipt key carries. The executor that once
//! lived here belonged to protocol v3 and retired with it.
use crate::application::ports::Clock;
use crate::domain::invocation::NormalizedArgumentsHash;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub(crate) const INVOCATION_HANDOFF_WINDOW: Duration = Duration::from_secs(7);
pub(crate) const RESPONSE_SERIALIZATION_MARGIN_MS: u64 = 125;
pub(crate) const RESPONSE_SERIALIZATION_MARGIN: Duration =
    Duration::from_millis(RESPONSE_SERIALIZATION_MARGIN_MS);
pub(crate) const TASK_RECONCILIATION_BUDGET: Duration = Duration::from_secs(2);

pub(crate) fn handoff_budget(host_remaining: Option<Duration>) -> Duration {
    host_remaining
        .map(|remaining| remaining.saturating_sub(RESPONSE_SERIALIZATION_MARGIN))
        .unwrap_or(INVOCATION_HANDOFF_WINDOW)
        .min(INVOCATION_HANDOFF_WINDOW)
}

/// Opaque daemon-receipt capability shared by admission, preparation,
/// execution handoff and the final response writer. The frontend contributes
/// only a shrinking remaining budget; no later daemon stage may add that
/// duration to a fresh clock reading.
#[derive(Clone)]
pub(crate) struct InvocationResponseDeadline {
    clock: Arc<dyn Clock>,
    receipt_at: Instant,
    handoff_at: Instant,
    response_at: Instant,
}

impl std::fmt::Debug for InvocationResponseDeadline {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("InvocationResponseDeadline")
            .field("receipt_at", &self.receipt_at)
            .field("handoff_at", &self.handoff_at)
            .field("response_at", &self.response_at)
            .finish()
    }
}

impl InvocationResponseDeadline {
    pub(crate) fn capture(clock: Arc<dyn Clock>) -> Self {
        let receipt_at = clock.now();
        let handoff_at = receipt_at + INVOCATION_HANDOFF_WINDOW;
        Self {
            clock,
            receipt_at,
            handoff_at,
            response_at: handoff_at + RESPONSE_SERIALIZATION_MARGIN,
        }
    }

    pub(crate) fn restrict_to_frontend_budget(mut self, remaining: Duration) -> Self {
        self.handoff_at = self
            .handoff_at
            .min(self.receipt_at + remaining.min(INVOCATION_HANDOFF_WINDOW));
        self.response_at = self.handoff_at + RESPONSE_SERIALIZATION_MARGIN;
        self
    }

    #[cfg(test)]
    fn capture_at_for_test(receipt_at: Instant) -> Self {
        Self::capture(Arc::new(FixedInvocationResponseClock(receipt_at)))
    }

    fn handoff_at(&self) -> Instant {
        self.handoff_at
    }

    pub(crate) fn response_at(&self) -> Instant {
        self.response_at
    }

    pub(crate) fn now(&self) -> Instant {
        self.clock.now()
    }

    fn actor_admission_at(&self) -> Instant {
        if self.handoff_at == self.receipt_at {
            self.response_at
        } else {
            self.handoff_at
        }
    }

    pub(crate) fn remaining_actor_admission_budget(&self) -> Duration {
        self.actor_admission_at()
            .saturating_duration_since(self.now())
    }

    pub(crate) fn remaining_handoff_budget(&self) -> Duration {
        self.handoff_at.saturating_duration_since(self.now())
    }

    pub(crate) fn checkpoint_actor_admission(&self) -> Result<(), &'static str> {
        if self.now() >= self.actor_admission_at() {
            Err("daemon actor admission deadline exceeded")
        } else {
            Ok(())
        }
    }

    pub(crate) fn checkpoint_handoff(&self) -> Result<(), &'static str> {
        if self.now() >= self.handoff_at {
            Err("daemon handoff deadline exceeded")
        } else {
            Ok(())
        }
    }

    fn belongs_to(&self, clock: &Arc<dyn Clock>) -> bool {
        Arc::ptr_eq(&self.clock, clock)
    }

    fn handoff_elapsed(&self) -> bool {
        self.handoff_at == self.receipt_at || self.now() >= self.handoff_at
    }

    fn same_authority_and_boundary(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.clock, &other.clock)
            && self.receipt_at == other.receipt_at
            && self.handoff_at == other.handoff_at
            && self.response_at == other.response_at
    }
}

#[cfg(test)]
struct FixedInvocationResponseClock(Instant);

#[cfg(test)]
impl Clock for FixedInvocationResponseClock {
    fn now(&self) -> Instant {
        self.0
    }
}

pub(crate) fn normalized_arguments_hash(
    arguments: &serde_json::Map<String, Value>,
) -> NormalizedArgumentsHash {
    fn canonical(value: &Value) -> Value {
        match value {
            Value::Object(object) => {
                let mut entries = object.iter().collect::<Vec<_>>();
                entries.sort_by(|left, right| left.0.cmp(right.0));
                Value::Object(
                    entries
                        .into_iter()
                        .map(|(key, value)| (key.clone(), canonical(value)))
                        .collect(),
                )
            }
            Value::Array(items) => Value::Array(items.iter().map(canonical).collect()),
            other => other.clone(),
        }
    }

    let bytes = serde_json::to_vec(&canonical(&Value::Object(arguments.clone())))
        .expect("canonical invocation arguments are always serializable");
    NormalizedArgumentsHash::from_sha256(Sha256::digest(bytes).into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::ports::TokioClock;
    use std::sync::Mutex;

    struct ManualClock(Mutex<Instant>);

    impl ManualClock {
        fn new(now: Instant) -> Self {
            Self(Mutex::new(now))
        }

        fn advance(&self, duration: Duration) {
            let mut now = self.0.lock().expect("manual clock lock");
            *now += duration;
        }
    }

    impl Clock for ManualClock {
        fn now(&self) -> Instant {
            *self.0.lock().expect("manual clock lock")
        }
    }

    fn response_deadline_from_clock(
        clock: &Arc<ManualClock>,
        budget: Duration,
    ) -> InvocationResponseDeadline {
        let clock: Arc<dyn Clock> = Arc::clone(clock) as Arc<dyn Clock>;
        InvocationResponseDeadline::capture(clock).restrict_to_frontend_budget(budget)
    }

    #[test]
    fn actor_admission_checkpoint_uses_only_zero_budget_response_margin() {
        let started = Instant::now();
        let zero_clock = Arc::new(ManualClock::new(started));
        let zero = response_deadline_from_clock(&zero_clock, Duration::ZERO);
        assert_eq!(
            zero.remaining_handoff_budget(),
            Duration::ZERO,
            "direct bootstrap work must not borrow the serialization reserve"
        );
        assert_eq!(
            zero.remaining_actor_admission_budget(),
            super::RESPONSE_SERIALIZATION_MARGIN
        );
        assert!(
            zero.checkpoint_actor_admission().is_ok(),
            "zero-budget actor admission must be allowed at receipt"
        );
        zero_clock.advance(super::RESPONSE_SERIALIZATION_MARGIN);
        assert!(
            zero.checkpoint_actor_admission().is_err(),
            "zero-budget actor admission exceeded the existing response boundary"
        );

        let nonzero_clock = Arc::new(ManualClock::new(started));
        let nonzero = response_deadline_from_clock(&nonzero_clock, Duration::from_secs(7));
        assert_eq!(
            nonzero.remaining_actor_admission_budget(),
            Duration::from_secs(7),
            "nonzero work must reserve the response serialization margin"
        );
        nonzero_clock.advance(Duration::from_secs(7));
        assert!(
            nonzero.checkpoint_actor_admission().is_err(),
            "nonzero actor admission borrowed the serialization margin"
        );
    }

    #[test]
    fn earlier_host_deadline_reserves_serialization_margin_without_using_timeout_as_prediction() {
        assert_eq!(handoff_budget(None), Duration::from_secs(7));
        assert_eq!(
            handoff_budget(Some(Duration::from_secs(5))),
            Duration::from_millis(4_875)
        );
        assert_eq!(
            handoff_budget(Some(Duration::from_millis(100))),
            Duration::ZERO
        );
    }

    #[test]
    fn production_clock_returns_non_decreasing_successive_samples() {
        let first = TokioClock.now();
        let second = TokioClock.now();

        assert!(first <= second);
    }
}
