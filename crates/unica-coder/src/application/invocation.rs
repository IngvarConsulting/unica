use crate::application::invocation_store::{
    InvocationStore, NewInvocationRecord, SafeStatusMessage, StoredInvocationRecord,
    TaskTransition, ToolIdentity,
};
use crate::application::operation_descriptors::ExecutionClass;
use crate::application::ports::Clock;
use crate::application::OperationResult;
use crate::domain::cache::CacheReport;
use crate::domain::invocation::{
    DomainResult, InvocationFailure, InvocationId, InvocationOutcome, InvocationStatus,
    NormalizedArgumentsHash, ResumeDescriptor, TaskId, TaskSnapshot,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

pub(crate) const INVOCATION_HANDOFF_WINDOW: Duration = Duration::from_secs(7);
pub(crate) const RESPONSE_SERIALIZATION_MARGIN: Duration = Duration::from_millis(125);
const INVOCATION_POLL_INTERVAL_MS: u64 = 250;
const INVOCATION_TTL_MS: u64 = 60 * 60 * 1_000;

pub(crate) fn handoff_budget(host_remaining: Option<Duration>) -> Duration {
    host_remaining
        .map(|remaining| remaining.saturating_sub(RESPONSE_SERIALIZATION_MARGIN))
        .unwrap_or(INVOCATION_HANDOFF_WINDOW)
        .min(INVOCATION_HANDOFF_WINDOW)
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

#[derive(Debug, Clone)]
pub(crate) struct PreparedDaemonInvocation {
    tool: ToolIdentity,
    normalized_arguments_hash: NormalizedArgumentsHash,
    workspace_identity_hash: crate::domain::invocation::SafeIdentityHash,
    class: ExecutionClass,
    response_budget: Duration,
}

impl PreparedDaemonInvocation {
    pub(crate) fn new(
        tool: ToolIdentity,
        normalized_arguments_hash: NormalizedArgumentsHash,
        workspace_identity_hash: crate::domain::invocation::SafeIdentityHash,
        class: ExecutionClass,
        response_budget: Duration,
    ) -> Self {
        Self {
            tool,
            normalized_arguments_hash,
            workspace_identity_hash,
            class,
            response_budget,
        }
    }
}

enum LiveInvocationState {
    Running { task_id: Option<TaskId> },
    Direct(Box<Result<DomainResult, InvocationFailure>>),
    DurableTerminal,
}

struct LiveInvocation {
    state: Mutex<LiveInvocationState>,
    changed: Condvar,
    cancellation: crate::domain::cancellation::CancellationToken,
}

impl LiveInvocation {
    fn new(task_id: Option<TaskId>) -> Self {
        Self {
            state: Mutex::new(LiveInvocationState::Running { task_id }),
            changed: Condvar::new(),
            cancellation: crate::domain::cancellation::CancellationToken::new(),
        }
    }
}

/// Daemon-owned single execution path. Query/cancel methods accept no domain
/// closure, making re-execution through polling structurally impossible.
pub(crate) struct InvocationExecutor {
    store: Arc<dyn InvocationStore>,
    clock: Arc<dyn Clock>,
    live_tasks: Mutex<HashMap<TaskId, Arc<LiveInvocation>>>,
    inline_waiters: Mutex<Vec<std::sync::Weak<LiveInvocation>>>,
}

impl InvocationExecutor {
    pub(crate) fn new(store: Arc<dyn InvocationStore>, clock: Arc<dyn Clock>) -> Self {
        Self {
            store,
            clock,
            live_tasks: Mutex::new(HashMap::new()),
            inline_waiters: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn submit_prepared<F>(
        self: &Arc<Self>,
        prepared: Result<PreparedDaemonInvocation, DomainResult>,
        execute: F,
    ) -> Result<InvocationOutcome, String>
    where
        F: FnOnce(
                crate::domain::cancellation::CancellationToken,
            ) -> Result<DomainResult, InvocationFailure>
            + Send
            + 'static,
    {
        match prepared {
            Ok(prepared) => self.submit(prepared, execute),
            Err(invalid) => Ok(InvocationOutcome::Direct(invalid)),
        }
    }

    pub(crate) fn submit<F>(
        self: &Arc<Self>,
        prepared: PreparedDaemonInvocation,
        execute: F,
    ) -> Result<InvocationOutcome, String>
    where
        F: FnOnce(
                crate::domain::cancellation::CancellationToken,
            ) -> Result<DomainResult, InvocationFailure>
            + Send
            + 'static,
    {
        let invocation_id = InvocationId::new();
        if matches!(prepared.class, ExecutionClass::KnownLong(_))
            || prepared.response_budget.is_zero()
        {
            let record = self.materialize(&prepared, invocation_id)?;
            let snapshot = snapshot_from_record(record.clone(), self.clock.now());
            let live = Arc::new(LiveInvocation::new(Some(record.task_id)));
            self.insert_live(record.task_id, Arc::clone(&live))?;
            self.spawn_execution(live, execute);
            return Ok(InvocationOutcome::Task(snapshot));
        }

        let live = Arc::new(LiveInvocation::new(None));
        self.inline_waiters
            .lock()
            .map_err(|_| "daemon inline waiter registry is poisoned".to_string())?
            .push(Arc::downgrade(&live));
        // The transmitted response budget is already shrinking when the daemon receives the
        // request. Capture its local boundary before scheduling execution so thread startup can
        // never replenish the caller's handoff window.
        let deadline = self.clock.now() + prepared.response_budget;
        self.spawn_execution(Arc::clone(&live), execute);
        let mut state = live
            .state
            .lock()
            .map_err(|_| "daemon invocation state is poisoned".to_string())?;
        loop {
            match &*state {
                LiveInvocationState::Direct(result) => {
                    let result = result.as_ref().clone();
                    if self.clock.now() < deadline {
                        return result
                            .map(InvocationOutcome::Direct)
                            .map_err(|failure| format!("{}: {}", failure.code, failure.message));
                    }
                    let working = self.materialize(&prepared, invocation_id)?;
                    let terminal = self.persist_terminal(working.task_id, result)?;
                    *state = LiveInvocationState::DurableTerminal;
                    return Ok(InvocationOutcome::Task(snapshot_from_record(
                        terminal,
                        self.clock.now(),
                    )));
                }
                LiveInvocationState::DurableTerminal => {
                    return Err("durable invocation terminated before task publication".into());
                }
                LiveInvocationState::Running { task_id: Some(_) } => {
                    return Err("inline invocation was materialized twice".into());
                }
                LiveInvocationState::Running { task_id: None } => {}
            }
            let remaining = deadline.saturating_duration_since(self.clock.now());
            if remaining.is_zero() {
                let record = self.materialize(&prepared, invocation_id)?;
                *state = LiveInvocationState::Running {
                    task_id: Some(record.task_id),
                };
                self.insert_live(record.task_id, Arc::clone(&live))?;
                return Ok(InvocationOutcome::Task(snapshot_from_record(
                    record,
                    self.clock.now(),
                )));
            }
            let (next, _) = live
                .changed
                .wait_timeout(state, remaining.min(Duration::from_millis(10)))
                .map_err(|_| "daemon invocation state is poisoned".to_string())?;
            state = next;
        }
    }

    fn materialize(
        &self,
        prepared: &PreparedDaemonInvocation,
        invocation_id: InvocationId,
    ) -> Result<StoredInvocationRecord, String> {
        let queued = self
            .store
            .create(NewInvocationRecord::new(
                invocation_id,
                prepared.tool,
                prepared.normalized_arguments_hash.clone(),
                prepared.workspace_identity_hash.clone(),
                SafeStatusMessage::Queued,
                INVOCATION_POLL_INTERVAL_MS,
                INVOCATION_TTL_MS,
                None,
            ))
            .map_err(|error| error.to_string())?;
        self.store
            .update(
                queued.task_id,
                TaskTransition::StartWorking {
                    status_message: SafeStatusMessage::Working,
                },
            )
            .map_err(|error| error.to_string())
    }

    fn insert_live(&self, task_id: TaskId, live: Arc<LiveInvocation>) -> Result<(), String> {
        self.live_tasks
            .lock()
            .map_err(|_| "daemon live-task registry is poisoned".to_string())?
            .insert(task_id, live);
        Ok(())
    }

    fn persist_terminal(
        &self,
        task_id: TaskId,
        outcome: Result<DomainResult, InvocationFailure>,
    ) -> Result<StoredInvocationRecord, String> {
        let transition = match outcome {
            Ok(result) => TaskTransition::Complete {
                status_message: SafeStatusMessage::Completed,
                result: Box::new(result),
            },
            Err(_) => TaskTransition::Fail {
                status_message: SafeStatusMessage::Failed,
            },
        };
        self.store
            .update(task_id, transition)
            .map_err(|error| error.to_string())
    }

    fn spawn_execution<F>(self: &Arc<Self>, live: Arc<LiveInvocation>, execute: F)
    where
        F: FnOnce(
                crate::domain::cancellation::CancellationToken,
            ) -> Result<DomainResult, InvocationFailure>
            + Send
            + 'static,
    {
        let executor = Arc::clone(self);
        let cancellation = live.cancellation.clone();
        std::thread::spawn(move || {
            let outcome = execute(cancellation);
            let mut state = match live.state.lock() {
                Ok(state) => state,
                Err(_) => return,
            };
            let task_id = match &*state {
                LiveInvocationState::Running { task_id } => *task_id,
                LiveInvocationState::Direct(_) | LiveInvocationState::DurableTerminal => return,
            };
            if let Some(task_id) = task_id {
                // A committed cancellation wins over late completion. The
                // failed transition is deliberately not retried or published.
                let _ = executor.persist_terminal(task_id, outcome);
                *state = LiveInvocationState::DurableTerminal;
                live.changed.notify_all();
                if let Ok(mut tasks) = executor.live_tasks.lock() {
                    tasks.remove(&task_id);
                }
            } else {
                *state = LiveInvocationState::Direct(Box::new(outcome));
                live.changed.notify_all();
            }
        });
    }

    pub(crate) fn get_task(&self, task_id: TaskId) -> Result<TaskSnapshot, String> {
        self.store
            .get(task_id)
            .map(|record| snapshot_from_record(record, self.clock.now()))
            .map_err(|error| error.to_string())
    }

    pub(crate) fn wait_task(
        &self,
        task_id: TaskId,
        wait: Duration,
    ) -> Result<TaskSnapshot, String> {
        let current = self.get_task(task_id)?;
        if current.status != InvocationStatus::Working || wait.is_zero() {
            return Ok(current);
        }
        let live = self
            .live_tasks
            .lock()
            .map_err(|_| "daemon live-task registry is poisoned".to_string())?
            .get(&task_id)
            .cloned();
        if let Some(live) = live {
            let state = live
                .state
                .lock()
                .map_err(|_| "daemon invocation state is poisoned".to_string())?;
            let _ = live
                .changed
                .wait_timeout(state, wait)
                .map_err(|_| "daemon invocation state is poisoned".to_string())?;
        }
        self.get_task(task_id)
    }

    pub(crate) fn cancel_task(&self, task_id: TaskId) -> Result<TaskSnapshot, String> {
        let record = self
            .store
            .cancel(task_id, SafeStatusMessage::Cancelled)
            .map_err(|error| error.to_string())?;
        if let Some(live) = self
            .live_tasks
            .lock()
            .map_err(|_| "daemon live-task registry is poisoned".to_string())?
            .get(&task_id)
            .cloned()
        {
            live.cancellation.cancel();
            live.changed.notify_all();
        }
        Ok(snapshot_from_record(record, self.clock.now()))
    }

    pub(crate) fn has_active_invocations(&self) -> bool {
        if self.live_tasks.lock().is_ok_and(|tasks| !tasks.is_empty()) {
            return true;
        }
        self.inline_waiters.lock().is_ok_and(|mut waiters| {
            waiters.retain(|waiter| waiter.strong_count() > 0);
            !waiters.is_empty()
        })
    }

    #[cfg(test)]
    fn wake_deadline_waiters_for_test(&self) {
        if let Ok(mut waiters) = self.inline_waiters.lock() {
            waiters.retain(|waiter| {
                waiter.upgrade().is_some_and(|live| {
                    live.changed.notify_all();
                    true
                })
            });
        }
    }
}

fn snapshot_from_record(record: StoredInvocationRecord, observed_at: Instant) -> TaskSnapshot {
    let failure = (record.status == InvocationStatus::Failed).then(|| {
        if record.status_message == SafeStatusMessage::Interrupted {
            InvocationFailure::new("interrupted", "daemon invocation was interrupted")
        } else {
            InvocationFailure::new("invocation_failed", "daemon invocation failed")
        }
    });
    TaskSnapshot {
        task_id: record.task_id,
        invocation_id: record.invocation_id,
        status: record.status,
        result: record.result,
        failure,
        resume: record.resume,
        created_at: observed_at,
        updated_at: observed_at,
    }
}

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
    Outcome(Box<InvocationOutcome>),
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
            InvocationState::Outcome(outcome) => match outcome.as_ref() {
                InvocationOutcome::Direct(_) => InvocationStatus::Completed,
                InvocationOutcome::Task(task) => task.status,
            },
        }
    }

    pub(crate) fn updated_at(&self) -> Instant {
        self.updated_at
    }

    pub(crate) fn outcome(&self) -> Option<&InvocationOutcome> {
        match &self.state {
            InvocationState::Inline(_) => None,
            InvocationState::Outcome(outcome) => Some(outcome.as_ref()),
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
            InvocationState::Outcome(outcome) => match outcome.as_mut() {
                InvocationOutcome::Task(task) => {
                    task.status = InvocationStatus::Working;
                    task.updated_at = now;
                }
                InvocationOutcome::Direct(_) => unreachable!(),
            },
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
            InvocationState::Outcome(outcome)
                if matches!(outcome.as_ref(), InvocationOutcome::Direct(_)) =>
            {
                return Err(InvocationTransitionError::DirectResponseCommitted)
            }
            _ => return Err(self.invalid("materialize_task")),
        };
        let now = self.clock.now();
        self.state = InvocationState::Outcome(Box::new(InvocationOutcome::Task(TaskSnapshot {
            task_id,
            invocation_id: self.id,
            status,
            result: None,
            failure: None,
            resume,
            created_at: now,
            updated_at: now,
        })));
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
                self.state = InvocationState::Outcome(Box::new(InvocationOutcome::Direct(result)));
            }
            InvocationState::Outcome(outcome) => match outcome.as_mut() {
                InvocationOutcome::Task(task) => {
                    task.status = InvocationStatus::Completed;
                    task.result = Some(result);
                    task.failure = None;
                    task.resume = None;
                    task.updated_at = now;
                }
                InvocationOutcome::Direct(_) => unreachable!(),
            },
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
            InvocationState::Outcome(outcome) => match outcome.as_mut() {
                InvocationOutcome::Task(task) => {
                    task.status = InvocationStatus::Failed;
                    task.result = None;
                    task.failure = Some(failure);
                    task.resume = None;
                    task.updated_at = now;
                }
                InvocationOutcome::Direct(_) => unreachable!(),
            },
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
            InvocationState::Outcome(outcome) => match outcome.as_mut() {
                InvocationOutcome::Task(task) => {
                    task.status = InvocationStatus::Cancelled;
                    task.result = None;
                    task.failure = None;
                    task.resume = None;
                    task.updated_at = now;
                }
                InvocationOutcome::Direct(_) => unreachable!(),
            },
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
    use super::{handoff_budget, Invocation, InvocationExecutor, PreparedDaemonInvocation};
    use crate::application::invocation_store::{
        InvocationStore, InvocationStoreError, NewInvocationRecord, SafeStatusMessage,
        StoredInvocationRecord, TaskTransition, ToolIdentity,
    };
    use crate::application::operation_descriptors::{ExecutionClass, KnownLongReason};
    use crate::application::ports::{Clock, TokioClock};
    use crate::application::OperationResult;
    use crate::domain::invocation::{
        DomainResult, InvocationFailure, InvocationId, InvocationOutcome, InvocationStatus,
        NormalizedArgumentsHash, SafeIdentityHash, TaskId,
    };
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{mpsc, Arc, Condvar, Mutex};
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

    #[derive(Default)]
    struct MemoryStore {
        records: Mutex<HashMap<TaskId, StoredInvocationRecord>>,
    }

    impl InvocationStore for MemoryStore {
        fn create(
            &self,
            record: NewInvocationRecord,
        ) -> Result<StoredInvocationRecord, InvocationStoreError> {
            let stored = record.into_stored(TaskId::new(), 1);
            self.records
                .lock()
                .unwrap()
                .insert(stored.task_id, stored.clone());
            Ok(stored)
        }

        fn get(&self, task_id: TaskId) -> Result<StoredInvocationRecord, InvocationStoreError> {
            self.records
                .lock()
                .unwrap()
                .get(&task_id)
                .cloned()
                .ok_or(InvocationStoreError::NotFound)
        }

        fn update(
            &self,
            task_id: TaskId,
            transition: TaskTransition,
        ) -> Result<StoredInvocationRecord, InvocationStoreError> {
            let mut records = self.records.lock().unwrap();
            let record = records
                .get_mut(&task_id)
                .ok_or(InvocationStoreError::NotFound)?;
            match transition {
                TaskTransition::StartWorking { status_message } => {
                    record.status = InvocationStatus::Working;
                    record.status_message = status_message;
                }
                TaskTransition::Complete {
                    status_message,
                    result,
                } => {
                    record.status = InvocationStatus::Completed;
                    record.status_message = status_message;
                    record.result = Some(*result);
                }
                TaskTransition::Fail { status_message } => {
                    record.status = InvocationStatus::Failed;
                    record.status_message = status_message;
                }
            }
            Ok(record.clone())
        }

        fn cancel(
            &self,
            task_id: TaskId,
            status_message: SafeStatusMessage,
        ) -> Result<StoredInvocationRecord, InvocationStoreError> {
            let mut records = self.records.lock().unwrap();
            let record = records
                .get_mut(&task_id)
                .ok_or(InvocationStoreError::NotFound)?;
            if !record.is_terminal() {
                record.status = InvocationStatus::Cancelled;
                record.status_message = status_message;
                record.result = None;
            }
            Ok(record.clone())
        }
    }

    fn prepared(class: ExecutionClass, budget: Duration) -> PreparedDaemonInvocation {
        PreparedDaemonInvocation::new(
            ToolIdentity::Check,
            NormalizedArgumentsHash::from_sha256([0x22; 32]),
            SafeIdentityHash::from_sha256([0x33; 32]),
            class,
            budget,
        )
    }

    #[test]
    fn fake_clock_6999_is_direct_and_7000_is_already_a_durable_task() {
        let started = Instant::now();

        let direct_clock = Arc::new(ManualClock::new(started));
        let direct_executor = Arc::new(InvocationExecutor::new(
            Arc::new(MemoryStore::default()),
            direct_clock.clone(),
        ));
        let (direct_started, direct_started_wait) = mpsc::channel();
        let (direct_release, direct_wait) = mpsc::channel();
        let direct_run = {
            let executor = Arc::clone(&direct_executor);
            std::thread::spawn(move || {
                executor.submit(
                    prepared(ExecutionClass::InlineCandidate, Duration::from_secs(7)),
                    move |_| {
                        direct_started.send(()).unwrap();
                        direct_wait.recv().unwrap();
                        Ok(result("6999"))
                    },
                )
            })
        };
        direct_started_wait.recv().unwrap();
        direct_clock.advance(Duration::from_millis(6_999));
        direct_release.send(()).unwrap();
        let direct = direct_run.join().unwrap().unwrap();
        assert!(matches!(direct, InvocationOutcome::Direct(result) if result.summary == "6999"));

        let task_clock = Arc::new(ManualClock::new(started));
        let task_executor = Arc::new(InvocationExecutor::new(
            Arc::new(MemoryStore::default()),
            task_clock.clone(),
        ));
        let (task_started, task_started_wait) = mpsc::channel();
        let (task_release, task_wait) = mpsc::channel();
        let task_run = {
            let executor = Arc::clone(&task_executor);
            std::thread::spawn(move || {
                executor.submit(
                    prepared(ExecutionClass::InlineCandidate, Duration::from_secs(7)),
                    move |_| {
                        task_started.send(()).unwrap();
                        task_wait.recv().unwrap();
                        Ok(result("7000"))
                    },
                )
            })
        };
        task_started_wait.recv().unwrap();
        task_clock.advance(Duration::from_millis(7_000));
        task_executor.wake_deadline_waiters_for_test();
        let task = task_run.join().unwrap().unwrap();
        let task_id = match task {
            InvocationOutcome::Task(snapshot) => snapshot.task_id,
            other => panic!("expected durable task at boundary, got {other:?}"),
        };
        assert_eq!(
            task_executor.get_task(task_id).unwrap().status,
            InvocationStatus::Working
        );
        task_release.send(()).unwrap();
        assert_eq!(
            task_executor
                .wait_task(task_id, Duration::from_secs(1))
                .unwrap()
                .status,
            InvocationStatus::Completed
        );
    }

    #[test]
    fn zero_budget_is_materialized_before_execution_and_never_returns_direct() {
        let store = Arc::new(MemoryStore::default());
        let executor = Arc::new(InvocationExecutor::new(
            store.clone(),
            Arc::new(ManualClock::new(Instant::now())),
        ));
        let store_during_execution = store.clone();
        let outcome = executor
            .submit(
                prepared(ExecutionClass::InlineCandidate, Duration::ZERO),
                move |_| {
                    assert_eq!(store_during_execution.records.lock().unwrap().len(), 1);
                    Ok(result("zero-budget task"))
                },
            )
            .unwrap();
        let task_id = match outcome {
            InvocationOutcome::Task(snapshot) => snapshot.task_id,
            other => panic!("zero budget must hand off immediately: {other:?}"),
        };
        assert_eq!(
            executor
                .wait_task(task_id, Duration::from_secs(1))
                .unwrap()
                .status,
            InvocationStatus::Completed
        );
    }

    #[test]
    fn simultaneous_completion_and_handoff_publish_one_terminal_result_from_one_execution() {
        let clock = Arc::new(ManualClock::new(Instant::now()));
        let executor = Arc::new(InvocationExecutor::new(
            Arc::new(MemoryStore::default()),
            clock.clone(),
        ));
        let count = Arc::new(AtomicUsize::new(0));
        let count_run = Arc::clone(&count);
        let (started, started_wait) = mpsc::channel();
        let (release, release_wait) = mpsc::channel();
        let run = {
            let executor = Arc::clone(&executor);
            std::thread::spawn(move || {
                executor.submit(
                    prepared(ExecutionClass::InlineCandidate, Duration::from_secs(7)),
                    move |_| {
                        count_run.fetch_add(1, Ordering::SeqCst);
                        started.send(()).unwrap();
                        release_wait.recv().unwrap();
                        Ok(result("race terminal"))
                    },
                )
            })
        };
        started_wait.recv().unwrap();
        clock.advance(Duration::from_secs(7));
        release.send(()).unwrap();
        executor.wake_deadline_waiters_for_test();
        let outcome = run.join().unwrap().unwrap();
        match outcome {
            InvocationOutcome::Direct(result) => assert_eq!(result.summary, "race terminal"),
            InvocationOutcome::Task(snapshot) => {
                let terminal = executor
                    .wait_task(snapshot.task_id, Duration::from_secs(1))
                    .unwrap();
                assert_eq!(terminal.status, InvocationStatus::Completed);
                assert_eq!(terminal.result.unwrap().summary, "race terminal");
            }
        }
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn completion_staged_at_the_7000_boundary_is_a_durable_terminal_task_not_direct() {
        let clock = Arc::new(ManualClock::new(Instant::now()));
        let executor = Arc::new(InvocationExecutor::new(
            Arc::new(MemoryStore::default()),
            clock.clone(),
        ));
        let (started, started_wait) = mpsc::channel();
        let (release, release_wait) = mpsc::channel();
        let run = {
            let executor = Arc::clone(&executor);
            std::thread::spawn(move || {
                executor.submit(
                    prepared(ExecutionClass::InlineCandidate, Duration::from_secs(7)),
                    move |_| {
                        started.send(()).unwrap();
                        release_wait.recv().unwrap();
                        Ok(result("boundary terminal"))
                    },
                )
            })
        };
        started_wait.recv().unwrap();
        clock.advance(Duration::from_secs(7));
        release.send(()).unwrap();
        executor.wake_deadline_waiters_for_test();
        let outcome = run.join().unwrap().unwrap();
        let snapshot = match outcome {
            InvocationOutcome::Task(snapshot) => snapshot,
            other => panic!("boundary completion escaped as direct: {other:?}"),
        };
        let terminal = executor
            .wait_task(snapshot.task_id, Duration::from_secs(1))
            .unwrap();
        assert_eq!(terminal.status, InvocationStatus::Completed);
        assert_eq!(terminal.result.unwrap().summary, "boundary terminal");
    }

    #[test]
    fn canonical_handoff_boundary_is_direct_before_7000_and_durable_at_or_before_deadline() {
        fake_clock_6999_is_direct_and_7000_is_already_a_durable_task();
        zero_budget_is_materialized_before_execution_and_never_returns_direct();
        simultaneous_completion_and_handoff_publish_one_terminal_result_from_one_execution();
        completion_staged_at_the_7000_boundary_is_a_durable_terminal_task_not_direct();
    }

    #[test]
    fn every_known_long_reason_materializes_before_execution_and_invalid_preparation_is_direct() {
        for reason in [
            KnownLongReason::MissingEngine,
            KnownLongReason::ColdIndex,
            KnownLongReason::ProviderStartup,
            KnownLongReason::OccupiedWriteLease,
            KnownLongReason::ExternalProcess,
        ] {
            let store = Arc::new(MemoryStore::default());
            let executor = Arc::new(InvocationExecutor::new(
                store.clone(),
                Arc::new(ManualClock::new(Instant::now())),
            ));
            let observed = Arc::new(AtomicUsize::new(0));
            let observed_run = Arc::clone(&observed);
            let outcome = executor
                .submit(
                    prepared(ExecutionClass::KnownLong(reason), Duration::from_secs(7)),
                    move |_| {
                        observed_run.fetch_add(1, Ordering::SeqCst);
                        Ok(result("known long"))
                    },
                )
                .unwrap();
            let task_id = match outcome {
                InvocationOutcome::Task(snapshot) => snapshot.task_id,
                other => panic!("{reason:?} was not materialized: {other:?}"),
            };
            assert!(
                store.get(task_id).is_ok(),
                "{reason:?} executed before durable state"
            );
            let terminal = executor.wait_task(task_id, Duration::from_secs(1)).unwrap();
            assert_eq!(terminal.status, InvocationStatus::Completed);
            assert_eq!(observed.load(Ordering::SeqCst), 1);
        }

        let executor = Arc::new(InvocationExecutor::new(
            Arc::new(MemoryStore::default()),
            Arc::new(ManualClock::new(Instant::now())),
        ));
        let count = Arc::new(AtomicUsize::new(0));
        let invalid = DomainResult {
            ok: false,
            summary: "invalid address".into(),
            ..DomainResult::success("unused")
        };
        let outcome = executor
            .submit_prepared(Err(invalid.clone()), {
                let count = Arc::clone(&count);
                move |_| {
                    count.fetch_add(1, Ordering::SeqCst);
                    Ok(result("must not run"))
                }
            })
            .unwrap();
        assert_eq!(outcome, InvocationOutcome::Direct(invalid));
        assert_eq!(count.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn polling_and_idempotent_cancellation_never_launch_a_second_execution() {
        let executor = Arc::new(InvocationExecutor::new(
            Arc::new(MemoryStore::default()),
            Arc::new(ManualClock::new(Instant::now())),
        ));
        let count = Arc::new(AtomicUsize::new(0));
        let count_run = Arc::clone(&count);
        let gate = Arc::new((Mutex::new(()), Condvar::new()));
        let gate_run = Arc::clone(&gate);
        let (started, started_wait) = mpsc::channel();
        let outcome = executor
            .submit(
                prepared(
                    ExecutionClass::KnownLong(KnownLongReason::OccupiedWriteLease),
                    Duration::from_secs(7),
                ),
                move |cancellation| {
                    count_run.fetch_add(1, Ordering::SeqCst);
                    started.send(()).unwrap();
                    let (lock, changed) = &*gate_run;
                    let mut cancelled = lock.lock().unwrap();
                    while !cancellation.is_cancelled() {
                        let (next, _) = changed
                            .wait_timeout(cancelled, Duration::from_millis(10))
                            .unwrap();
                        cancelled = next;
                    }
                    Err(InvocationFailure::new("cancelled", "cancelled"))
                },
            )
            .unwrap();
        let task_id = match outcome {
            InvocationOutcome::Task(snapshot) => snapshot.task_id,
            _ => unreachable!(),
        };
        started_wait.recv().unwrap();
        executor.get_task(task_id).unwrap();
        executor.wait_task(task_id, Duration::ZERO).unwrap();
        executor.cancel_task(task_id).unwrap();
        executor.cancel_task(task_id).unwrap();
        executor.get_task(task_id).unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert_eq!(
            executor.get_task(task_id).unwrap().status,
            InvocationStatus::Cancelled
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
    fn interrupted_record_projects_closed_failure_without_an_execution_seam() {
        // FileInvocationStore owns the restart transition and proves it separately. The
        // application boundary consumes only the resulting closed record and exposes no callback
        // on get, so observing recovery cannot resume domain execution.
        let store = Arc::new(MemoryStore::default());
        let queued = store
            .create(NewInvocationRecord::new(
                InvocationId::new(),
                ToolIdentity::Search,
                NormalizedArgumentsHash::from_sha256([0x71; 32]),
                SafeIdentityHash::from_sha256([0x72; 32]),
                SafeStatusMessage::Queued,
                250,
                60_000,
                None,
            ))
            .unwrap();
        let working = store
            .update(
                queued.task_id,
                TaskTransition::StartWorking {
                    status_message: SafeStatusMessage::Working,
                },
            )
            .unwrap();
        store
            .update(
                working.task_id,
                TaskTransition::Fail {
                    status_message: SafeStatusMessage::Interrupted,
                },
            )
            .unwrap();
        let executor = InvocationExecutor::new(store, Arc::new(ManualClock::new(Instant::now())));
        let snapshot = executor.get_task(working.task_id).unwrap();
        assert_eq!(snapshot.status, InvocationStatus::Failed);
        assert_eq!(
            snapshot.failure,
            Some(InvocationFailure::new(
                "interrupted",
                "daemon invocation was interrupted",
            ))
        );
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
