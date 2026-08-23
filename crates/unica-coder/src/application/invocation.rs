use crate::application::ports::Clock;
use crate::application::OperationResult;
use crate::domain::cache::CacheReport;
use crate::domain::invocation::{
    DomainResult, InvocationFailure, InvocationId, InvocationOutcome, InvocationStatus,
    NormalizedArgumentsHash, ResumeDescriptor, TaskId, TaskSnapshot,
};
use serde_json::Value;
use std::sync::Arc;
use std::time::Instant;

#[derive(Debug, Clone, PartialEq)]
enum InlineState {
    Active(InvocationStatus),
    Failed(InvocationFailure),
    Cancelled,
}

impl InlineState {
    fn status(&self) -> InvocationStatus {
        match self {
            Self::Active(status) => *status,
            Self::Failed(_) => InvocationStatus::Failed,
            Self::Cancelled => InvocationStatus::Cancelled,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum InvocationState {
    Inline(InlineState),
    Outcome(InvocationOutcome),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InvocationTransitionError {
    InvalidTransition {
        from: InvocationStatus,
        attempted: &'static str,
    },
    DirectResponseCommitted,
}

#[derive(Clone)]
pub(crate) struct Invocation {
    id: InvocationId,
    tool: String,
    normalized_arguments_hash: NormalizedArgumentsHash,
    created_at: Instant,
    updated_at: Instant,
    state: InvocationState,
    clock: Arc<dyn Clock>,
}

impl std::fmt::Debug for Invocation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Invocation")
            .field("id", &self.id)
            .field("tool", &self.tool)
            .field("normalized_arguments_hash", &self.normalized_arguments_hash)
            .field("created_at", &self.created_at)
            .field("updated_at", &self.updated_at)
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

impl PartialEq for Invocation {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.tool == other.tool
            && self.normalized_arguments_hash == other.normalized_arguments_hash
            && self.created_at == other.created_at
            && self.updated_at == other.updated_at
            && self.state == other.state
    }
}

impl Invocation {
    pub(crate) fn new(
        id: InvocationId,
        tool: impl Into<String>,
        normalized_arguments_hash: NormalizedArgumentsHash,
        clock: Arc<dyn Clock>,
    ) -> Self {
        let now = clock.now();
        Self {
            id,
            tool: tool.into(),
            normalized_arguments_hash,
            created_at: now,
            updated_at: now,
            state: InvocationState::Inline(InlineState::Active(InvocationStatus::Queued)),
            clock,
        }
    }

    pub(crate) fn status(&self) -> InvocationStatus {
        match &self.state {
            InvocationState::Inline(state) => state.status(),
            InvocationState::Outcome(InvocationOutcome::Direct(_)) => InvocationStatus::Completed,
            InvocationState::Outcome(InvocationOutcome::Task(task)) => task.status,
        }
    }

    pub(crate) fn updated_at(&self) -> Instant {
        self.updated_at
    }

    pub(crate) fn outcome(&self) -> Option<&InvocationOutcome> {
        match &self.state {
            InvocationState::Inline(_) => None,
            InvocationState::Outcome(outcome) => Some(outcome),
        }
    }

    pub(crate) fn start_working(&mut self) -> Result<(), InvocationTransitionError> {
        if self.status() != InvocationStatus::Queued {
            return Err(self.invalid("start_working"));
        }
        let now = self.clock.now();
        match &mut self.state {
            InvocationState::Inline(state) => {
                *state = InlineState::Active(InvocationStatus::Working);
            }
            InvocationState::Outcome(InvocationOutcome::Task(task)) => {
                task.status = InvocationStatus::Working;
                task.updated_at = now;
            }
            InvocationState::Outcome(InvocationOutcome::Direct(_)) => unreachable!(),
        }
        self.updated_at = now;
        Ok(())
    }

    pub(crate) fn materialize_task(
        &mut self,
        task_id: TaskId,
        resume: Option<ResumeDescriptor>,
    ) -> Result<(), InvocationTransitionError> {
        let status = match &self.state {
            InvocationState::Inline(InlineState::Active(status @ InvocationStatus::Queued))
            | InvocationState::Inline(InlineState::Active(status @ InvocationStatus::Working)) => {
                *status
            }
            InvocationState::Outcome(InvocationOutcome::Direct(_)) => {
                return Err(InvocationTransitionError::DirectResponseCommitted)
            }
            _ => return Err(self.invalid("materialize_task")),
        };
        let now = self.clock.now();
        self.state = InvocationState::Outcome(InvocationOutcome::Task(TaskSnapshot {
            task_id,
            invocation_id: self.id,
            status,
            result: None,
            failure: None,
            resume,
            created_at: now,
            updated_at: now,
        }));
        self.updated_at = now;
        Ok(())
    }

    pub(crate) fn complete(
        &mut self,
        result: DomainResult,
    ) -> Result<(), InvocationTransitionError> {
        if self.status() != InvocationStatus::Working {
            return Err(self.invalid("complete"));
        }
        let now = self.clock.now();
        match &mut self.state {
            InvocationState::Inline(_) => {
                self.state = InvocationState::Outcome(InvocationOutcome::Direct(result));
            }
            InvocationState::Outcome(InvocationOutcome::Task(task)) => {
                task.status = InvocationStatus::Completed;
                task.result = Some(result);
                task.failure = None;
                task.resume = None;
                task.updated_at = now;
            }
            InvocationState::Outcome(InvocationOutcome::Direct(_)) => unreachable!(),
        }
        self.updated_at = now;
        Ok(())
    }

    pub(crate) fn fail(
        &mut self,
        failure: InvocationFailure,
    ) -> Result<(), InvocationTransitionError> {
        if self.status() != InvocationStatus::Working {
            return Err(self.invalid("fail"));
        }
        let now = self.clock.now();
        match &mut self.state {
            InvocationState::Inline(state) => *state = InlineState::Failed(failure),
            InvocationState::Outcome(InvocationOutcome::Task(task)) => {
                task.status = InvocationStatus::Failed;
                task.result = None;
                task.failure = Some(failure);
                task.resume = None;
                task.updated_at = now;
            }
            InvocationState::Outcome(InvocationOutcome::Direct(_)) => unreachable!(),
        }
        self.updated_at = now;
        Ok(())
    }

    pub(crate) fn cancel(&mut self) -> Result<(), InvocationTransitionError> {
        if self.status() == InvocationStatus::Cancelled {
            return Ok(());
        }
        if self.status() != InvocationStatus::Working {
            return Err(self.invalid("cancel"));
        }
        let now = self.clock.now();
        match &mut self.state {
            InvocationState::Inline(state) => *state = InlineState::Cancelled,
            InvocationState::Outcome(InvocationOutcome::Task(task)) => {
                task.status = InvocationStatus::Cancelled;
                task.result = None;
                task.failure = None;
                task.resume = None;
                task.updated_at = now;
            }
            InvocationState::Outcome(InvocationOutcome::Direct(_)) => unreachable!(),
        }
        self.updated_at = now;
        Ok(())
    }

    fn invalid(&self, attempted: &'static str) -> InvocationTransitionError {
        InvocationTransitionError::InvalidTransition {
            from: self.status(),
            attempted,
        }
    }
}

fn legacy_string(value: Value) -> String {
    match value {
        Value::String(text) => text,
        other => other.to_string(),
    }
}

const LEGACY_UNOBSERVED_CACHE_MODE: &str = "unobserved";
const LEGACY_UNOBSERVED_CACHE_ROOT: &str = "<unobserved-cache-root>";

/// Compatibility-only v0.12 projection of the canonical v0.13 result.
///
/// The outer shape is deliberately legacy and therefore lossy, but its `data`
/// slot retains the complete canonical result so the conversion loses no
/// domain information. A v0.13 transport must serialize [`DomainResult`]
/// directly and must not treat this [`OperationResult`] as its wire envelope.
///
/// `OperationResult` requires a cache report while `DomainResult` intentionally
/// carries none. `unobserved` plus the non-path root sentinel makes epoch zero
/// mean "not observed" here; it must not be read as an observed workspace epoch.
impl From<DomainResult> for OperationResult {
    fn from(result: DomainResult) -> Self {
        let canonical_data =
            serde_json::to_value(&result).expect("DomainResult serialization is infallible");
        let DomainResult {
            ok,
            summary,
            changed,
            warnings,
            diagnostics,
            artifacts,
            ..
        } = result;
        let errors = if ok {
            Vec::new()
        } else {
            diagnostics.iter().cloned().map(legacy_string).collect()
        };
        Self {
            ok,
            summary,
            changes: changed.into_iter().map(legacy_string).collect(),
            warnings: warnings.into_iter().map(legacy_string).collect(),
            errors,
            artifacts: artifacts.into_iter().map(legacy_string).collect(),
            cache: CacheReport {
                mode: LEGACY_UNOBSERVED_CACHE_MODE.to_string(),
                root: LEGACY_UNOBSERVED_CACHE_ROOT.to_string(),
                workspace_epoch: 0,
                events: Vec::new(),
                invalidated: Vec::new(),
                refreshed: Vec::new(),
                lazy_rebuilt: Vec::new(),
                stale: Vec::new(),
                fresh: Vec::new(),
                publication_warnings: Vec::new(),
            },
            stdout: None,
            stderr: None,
            command: None,
            diagnostics: (!diagnostics.is_empty()).then_some(Value::Array(diagnostics)),
            data: Some(canonical_data),
            job: None,
            work: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Invocation;
    use crate::application::ports::{Clock, TokioClock};
    use crate::application::OperationResult;
    use crate::domain::invocation::{
        DomainResult, InvocationFailure, InvocationId, InvocationOutcome, InvocationStatus,
        NormalizedArgumentsHash, TaskId,
    };
    use serde_json::json;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

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

    fn result(summary: &str) -> DomainResult {
        DomainResult {
            ok: true,
            at: Some("source://main/Catalog.Items".to_string()),
            summary: summary.to_string(),
            data: Some(json!({"kind": "catalog", "name": "Items"})),
            changed: vec![json!({"at": "source://main/Catalog.Items"})],
            warnings: vec![json!({"code": "support_warning"})],
            diagnostics: vec![json!({"severity": "info", "code": "checked"})],
            artifacts: vec![json!({"kind": "cf", "sha256": "ab"})],
            next: vec![json!({"op": "view"})],
            rev: Some("rev-2".to_string()),
            cursor: Some("cursor-2".to_string()),
        }
    }

    fn invocation(clock: Arc<ManualClock>) -> Invocation {
        Invocation::new(
            InvocationId::new(),
            "unica.view",
            NormalizedArgumentsHash::from_sha256([0x22; 32]),
            clock,
        )
    }

    #[test]
    fn inline_invocation_moves_from_queued_through_working_to_direct_completion() {
        let clock = Arc::new(ManualClock::new(Instant::now()));
        let mut invocation = invocation(clock.clone());
        assert_eq!(invocation.status(), InvocationStatus::Queued);

        invocation.start_working().expect("start inline work");
        clock.advance(Duration::from_millis(25));
        invocation
            .complete(result("direct"))
            .expect("complete inline work");

        assert_eq!(invocation.status(), InvocationStatus::Completed);
        assert!(matches!(
            invocation.outcome(),
            Some(InvocationOutcome::Direct(result)) if result.summary == "direct"
        ));
    }

    #[test]
    fn materialized_task_moves_from_queued_through_working_to_each_terminal_state() {
        let clock = Arc::new(ManualClock::new(Instant::now()));

        let mut completed = invocation(clock.clone());
        completed
            .materialize_task(TaskId::new(), None)
            .expect("materialize completed task");
        completed.start_working().expect("start completed task");
        completed
            .complete(result("task result"))
            .expect("complete task");
        assert_eq!(completed.status(), InvocationStatus::Completed);

        let mut failed = invocation(clock.clone());
        failed
            .materialize_task(TaskId::new(), None)
            .expect("materialize failed task");
        failed.start_working().expect("start failed task");
        failed
            .fail(InvocationFailure::new("json_rpc", "transport failed"))
            .expect("fail task");
        assert_eq!(failed.status(), InvocationStatus::Failed);

        let mut cancelled = invocation(clock);
        cancelled
            .materialize_task(TaskId::new(), None)
            .expect("materialize cancelled task");
        cancelled.start_working().expect("start cancelled task");
        cancelled.cancel().expect("cancel task");
        assert_eq!(cancelled.status(), InvocationStatus::Cancelled);
    }

    #[test]
    fn illegal_backwards_and_second_terminal_transitions_fail_without_mutation() {
        let clock = Arc::new(ManualClock::new(Instant::now()));
        let mut invocation = invocation(clock);
        invocation
            .materialize_task(TaskId::new(), None)
            .expect("materialize task");
        invocation.start_working().expect("start task");
        invocation.complete(result("first")).expect("complete task");

        let terminal = invocation.clone();
        assert!(invocation.start_working().is_err(), "terminal -> working");
        assert_eq!(invocation, terminal, "backwards transition mutated state");
        assert!(
            invocation.complete(result("second")).is_err(),
            "second terminal result"
        );
        assert_eq!(invocation, terminal, "second result mutated state");
    }

    #[test]
    fn materialization_after_direct_response_fails_without_mutation() {
        let clock = Arc::new(ManualClock::new(Instant::now()));
        let mut invocation = invocation(clock);
        invocation.start_working().expect("start inline work");
        invocation
            .complete(result("direct"))
            .expect("complete inline work");

        let direct = invocation.clone();
        assert!(invocation.materialize_task(TaskId::new(), None).is_err());
        assert_eq!(invocation, direct);
    }

    #[test]
    fn repeated_cancel_is_idempotent_and_does_not_mutate_the_terminal_task() {
        let clock = Arc::new(ManualClock::new(Instant::now()));
        let mut invocation = invocation(clock);
        invocation.materialize_task(TaskId::new(), None).unwrap();
        invocation.start_working().unwrap();
        invocation.cancel().unwrap();

        let cancelled = invocation.clone();
        invocation
            .cancel()
            .expect("repeated cancellation is idempotent");
        assert_eq!(invocation, cancelled);
    }

    #[test]
    fn state_timestamps_come_from_the_injected_monotonic_clock() {
        let started_at = Instant::now();
        let clock = Arc::new(ManualClock::new(started_at));
        let mut invocation = invocation(clock.clone());
        assert_eq!(invocation.updated_at(), started_at);

        clock.advance(Duration::from_millis(17));
        invocation.start_working().unwrap();

        assert_eq!(
            invocation.updated_at(),
            started_at + Duration::from_millis(17)
        );
    }

    #[test]
    fn direct_and_terminal_task_paths_serialize_the_identical_domain_result() {
        let clock = Arc::new(ManualClock::new(Instant::now()));
        let mut direct = invocation(clock.clone());
        direct.start_working().unwrap();
        direct.complete(result("same result")).unwrap();

        let mut task = invocation(clock);
        task.materialize_task(TaskId::new(), None).unwrap();
        task.start_working().unwrap();
        task.complete(result("same result")).unwrap();

        let direct_json = serde_json::to_value(
            direct
                .outcome()
                .and_then(InvocationOutcome::terminal_result)
                .expect("direct result"),
        )
        .unwrap();
        let task_json = serde_json::to_value(
            task.outcome()
                .and_then(InvocationOutcome::terminal_result)
                .expect("task result"),
        )
        .unwrap();
        assert_eq!(direct_json, task_json);
        assert_eq!(direct_json["summary"], "same result");
    }

    #[test]
    fn legacy_adapter_changes_shape_but_preserves_the_entire_canonical_result_as_data() {
        let domain = result("legacy adapter");
        let canonical = serde_json::to_value(&domain).expect("serialize canonical result");
        let legacy: OperationResult = domain.into();
        let projected = serde_json::to_value(&legacy).expect("serialize legacy projection");

        assert_ne!(
            projected, canonical,
            "OperationResult is a v0.12 projection, not the v0.13 wire shape"
        );
        assert_eq!(projected["data"], canonical);
        assert_eq!(projected["data"]["at"], "source://main/Catalog.Items");
        assert_eq!(projected["data"]["next"][0]["op"], "view");
        assert_eq!(projected["data"]["rev"], "rev-2");
        assert_eq!(projected["data"]["cursor"], "cursor-2");
        assert_eq!(projected["data"]["data"]["kind"], "catalog");
    }

    #[test]
    fn legacy_adapter_marks_cache_as_explicitly_unobserved() {
        let legacy: OperationResult = result("legacy adapter").into();
        let projected = serde_json::to_value(legacy).expect("serialize legacy projection");

        assert_eq!(
            projected["cache"],
            json!({
                "mode": "unobserved",
                "root": "<unobserved-cache-root>",
                "workspace_epoch": 0,
                "events": [],
                "invalidated": [],
                "refreshed": [],
                "lazy_rebuilt": [],
                "stale": [],
                "fresh": []
            }),
            "epoch zero means non-observation only when mode is unobserved"
        );
    }

    #[test]
    fn legacy_adapter_mirrors_supported_fields_without_job_or_work() {
        let domain = result("legacy adapter");
        let legacy: OperationResult = domain.clone().into();
        let projected = serde_json::to_value(&legacy).expect("serialize legacy projection");

        assert_eq!(legacy.summary, "legacy adapter");
        assert_eq!(legacy.changes, [r#"{"at":"source://main/Catalog.Items"}"#]);
        assert_eq!(legacy.warnings, [r#"{"code":"support_warning"}"#]);
        assert_eq!(legacy.artifacts, [r#"{"kind":"cf","sha256":"ab"}"#]);
        assert!(legacy.errors.is_empty());
        assert_eq!(projected["diagnostics"][0]["code"], "checked");
        assert!(legacy.job.is_none());
        assert!(legacy.work.is_none());
        assert_eq!(projected.get("job"), None);
        assert_eq!(projected.get("work"), None);

        let mut failed = domain;
        failed.ok = false;
        let failed: OperationResult = failed.into();
        assert_eq!(failed.errors, [r#"{"severity":"info","code":"checked"}"#]);
    }

    #[test]
    fn production_clock_returns_non_decreasing_successive_samples() {
        let first = TokioClock.now();
        let second = TokioClock.now();

        assert!(first <= second);
    }
}
