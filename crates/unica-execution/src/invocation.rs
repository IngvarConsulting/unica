//! The daemon-side response budget shared by every daemon stage, and the
//! canonical argument digest a receipt key carries. The executor that once
//! lived here belonged to protocol v3 and retired with it.
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::{Duration, Instant};
use unica_execution_contract::invocation::NormalizedArgumentsHash;
use unica_work_control::clock::Clock;

// resource-limit: invocation.handoff-window
// class: deadline
// unit: ms
// scope: request
// value: 7000
// resource: Окно ожидания прямого ответа, после которого вызов передаётся в задание.
// status: unproven
// origin: коммит 88b99b0e0 (feat(invocation): hand long calls off after seven seconds)
// problem: неизвестно
// on-reach: демон возвращает задание с заранее выделенным taskId, та же попытка продолжает работу
// decision: закреплено правилом
// decision-source: arch/rules/runtime/invocation-handoff.md
pub const INVOCATION_HANDOFF_WINDOW: Duration = Duration::from_secs(7);
// resource-limit: invocation.response-serialization-margin
// class: deadline
// unit: ms
// scope: request
// value: 125
// resource: Добавка к бюджету прямого ответа на сериализацию ответа.
// status: unproven
// origin: коммит 215497a92 (fix(invocation): bound durable result publication)
// problem: неизвестно
// on-reach: квитанция, не решённая к бюджету плюс 125 мс, считается потерянной: frontend отвечает закрытым отказом о потерянном вызове
// decision: закреплено правилом INV.WIRE.DAEMON-RESPONSE-TIME
// decision-source: arch/rules/runtime/daemon-response-time.md
pub const RESPONSE_SERIALIZATION_MARGIN_MS: u64 = 125;
pub const RESPONSE_SERIALIZATION_MARGIN: Duration =
    Duration::from_millis(RESPONSE_SERIALIZATION_MARGIN_MS);
// resource-limit: invocation.task-reconciliation-budget
// class: deadline
// unit: ms
// scope: operation
// value: 2000
// resource: Добавка к ожиданию задания на сверку и нижняя граница срока долговечного шага хранилища квитанций.
// status: unproven
// origin: коммит 6115f4cd3 (fix(v13): align daemon task deadlines)
// problem: Короткий бюджет вызывающего не должен прерывать начатую запись хранилища и переводить демон в fail-stop (#1387).
// on-reach: ошибка срока ожидания задания; начатый шаг хранилища получает не меньше этого времени
// decision: закреплено правилом
// decision-source: arch/rules/runtime/receipt-store-fail-stop.md
pub const TASK_RECONCILIATION_BUDGET: Duration = Duration::from_secs(2);

pub fn handoff_budget(host_remaining: Option<Duration>) -> Duration {
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
pub struct InvocationResponseDeadline {
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
    pub fn capture(clock: Arc<dyn Clock>) -> Self {
        let receipt_at = clock.now();
        let handoff_at = receipt_at + INVOCATION_HANDOFF_WINDOW;
        Self {
            clock,
            receipt_at,
            handoff_at,
            response_at: handoff_at + RESPONSE_SERIALIZATION_MARGIN,
        }
    }

    pub fn restrict_to_frontend_budget(mut self, remaining: Duration) -> Self {
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

    pub fn response_at(&self) -> Instant {
        self.response_at
    }

    /// How long after the frame's receipt the answer is due: the frontend
    /// budget within the handoff window plus the serialization margin.
    pub fn response_window(&self) -> Duration {
        self.response_at.saturating_duration_since(self.receipt_at)
    }

    pub fn now(&self) -> Instant {
        self.clock.now()
    }

    pub fn remaining_handoff_budget(&self) -> Duration {
        self.handoff_at.saturating_duration_since(self.now())
    }

    pub fn checkpoint_handoff(&self) -> Result<(), &'static str> {
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

pub fn normalized_arguments_hash(
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
    use crate::clock::TokioClock;
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
    fn zero_frontend_budget_leaves_no_handoff_budget() {
        let started = Instant::now();
        let zero_clock = Arc::new(ManualClock::new(started));
        let zero = response_deadline_from_clock(&zero_clock, Duration::ZERO);
        assert_eq!(
            zero.remaining_handoff_budget(),
            Duration::ZERO,
            "direct bootstrap work must not borrow the serialization reserve"
        );
        let nonzero_clock = Arc::new(ManualClock::new(started));
        let nonzero = response_deadline_from_clock(&nonzero_clock, Duration::from_secs(7));
        assert_eq!(nonzero.remaining_handoff_budget(), Duration::from_secs(7));
        nonzero_clock.advance(Duration::from_secs(7));
        assert_eq!(nonzero.remaining_handoff_budget(), Duration::ZERO);
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
