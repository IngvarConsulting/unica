use crate::application::invocation_store::{
    InvocationStore, InvocationStoreError, NewInvocationRecord, SafeFailureReason,
    SafeStatusMessage, StoredInvocationRecord, TaskTransition, ToolIdentity,
};
use crate::application::invocation_store_actor::InvocationStoreActor;
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
const RECONCILIATION_BUDGET: Duration = Duration::from_secs(2);
const RECONCILIATION_INITIAL_BACKOFF: Duration = Duration::from_millis(10);
const RECONCILIATION_MAX_BACKOFF: Duration = Duration::from_millis(250);

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

#[derive(Clone)]
pub(crate) struct PreparedDaemonInvocation {
    tool: ToolIdentity,
    normalized_arguments_hash: NormalizedArgumentsHash,
    workspace_identity_hash: crate::domain::invocation::SafeIdentityHash,
    class: ExecutionClass,
    response_budget: Duration,
    resource_lease: Option<Arc<dyn Send + Sync>>,
}

impl std::fmt::Debug for PreparedDaemonInvocation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PreparedDaemonInvocation")
            .field("tool", &self.tool)
            .field("normalized_arguments_hash", &self.normalized_arguments_hash)
            .field("workspace_identity_hash", &self.workspace_identity_hash)
            .field("class", &self.class)
            .field("response_budget", &self.response_budget)
            .field("has_resource_lease", &self.resource_lease.is_some())
            .finish()
    }
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
            resource_lease: None,
        }
    }

    pub(crate) fn with_resource_lease(mut self, lease: Arc<dyn Send + Sync>) -> Self {
        self.resource_lease = Some(lease);
        self
    }
}

#[derive(Debug)]
pub(crate) enum InvocationExecutorError {
    Store(InvocationStoreError),
    ExecutionFailed,
    StatePoisoned,
    RestartRequested,
}

impl From<InvocationStoreError> for InvocationExecutorError {
    fn from(error: InvocationStoreError) -> Self {
        Self::Store(error)
    }
}

enum LiveInvocationState {
    Running {
        task_id: Option<TaskId>,
    },
    Materializing {
        task_id: TaskId,
        outcome: Option<Box<Result<DomainResult, InvocationFailure>>>,
    },
    Direct(Box<Result<DomainResult, InvocationFailure>>),
    DurableTerminal,
    RestartRequested,
}

struct LiveInvocation {
    state: Mutex<LiveInvocationState>,
    changed: Condvar,
    cancellation: crate::domain::cancellation::CancellationToken,
    resource_lease: Mutex<Option<Arc<dyn Send + Sync>>>,
}

impl LiveInvocation {
    fn new(task_id: Option<TaskId>, resource_lease: Option<Arc<dyn Send + Sync>>) -> Self {
        Self {
            state: Mutex::new(LiveInvocationState::Running { task_id }),
            changed: Condvar::new(),
            cancellation: crate::domain::cancellation::CancellationToken::new(),
            resource_lease: Mutex::new(resource_lease),
        }
    }

    fn release_resource_lease(&self) {
        if let Ok(mut lease) = self.resource_lease.lock() {
            lease.take();
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InvocationExecutorHealth {
    Healthy,
    RestartRequested,
}

#[derive(Debug, Clone, Copy)]
struct ReconciliationPolicy {
    budget: Duration,
    initial_backoff: Duration,
    max_backoff: Duration,
}

impl ReconciliationPolicy {
    const fn production() -> Self {
        Self {
            budget: RECONCILIATION_BUDGET,
            initial_backoff: RECONCILIATION_INITIAL_BACKOFF,
            max_backoff: RECONCILIATION_MAX_BACKOFF,
        }
    }

    #[cfg(test)]
    const fn with_budget_for_test(budget: Duration) -> Self {
        Self {
            budget,
            initial_backoff: RECONCILIATION_INITIAL_BACKOFF,
            max_backoff: RECONCILIATION_MAX_BACKOFF,
        }
    }
}

trait ReconciliationTimer: Send + Sync {
    fn now(&self) -> Instant;
    fn wait(&self, duration: Duration);
}

struct SystemReconciliationTimer;

impl ReconciliationTimer for SystemReconciliationTimer {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn wait(&self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

/// Daemon-owned single execution path. Query/cancel methods accept no domain
/// closure, making re-execution through polling structurally impossible.
pub(crate) struct InvocationExecutor {
    store: InvocationStoreActor,
    clock: Arc<dyn Clock>,
    live_tasks: Mutex<HashMap<TaskId, Arc<LiveInvocation>>>,
    inline_waiters: Mutex<Vec<std::sync::Weak<LiveInvocation>>>,
    health: Arc<Mutex<InvocationExecutorHealth>>,
    reconciliation: ReconciliationPolicy,
    reconciliation_timer: Arc<dyn ReconciliationTimer>,
}

impl InvocationExecutor {
    pub(crate) fn new(store: Arc<dyn InvocationStore>, clock: Arc<dyn Clock>) -> Self {
        Self {
            store: InvocationStoreActor::spawn(store),
            clock,
            live_tasks: Mutex::new(HashMap::new()),
            inline_waiters: Mutex::new(Vec::new()),
            health: Arc::new(Mutex::new(InvocationExecutorHealth::Healthy)),
            reconciliation: ReconciliationPolicy::production(),
            reconciliation_timer: Arc::new(SystemReconciliationTimer),
        }
    }

    #[cfg(test)]
    pub(crate) fn new_with_reconciliation_budget_for_test(
        store: Arc<dyn InvocationStore>,
        clock: Arc<dyn Clock>,
        budget: Duration,
    ) -> Self {
        Self {
            store: InvocationStoreActor::spawn(store),
            clock,
            live_tasks: Mutex::new(HashMap::new()),
            inline_waiters: Mutex::new(Vec::new()),
            health: Arc::new(Mutex::new(InvocationExecutorHealth::Healthy)),
            reconciliation: ReconciliationPolicy::with_budget_for_test(budget),
            reconciliation_timer: Arc::new(SystemReconciliationTimer),
        }
    }

    #[cfg(test)]
    fn new_with_reconciliation_for_test(
        store: Arc<dyn InvocationStore>,
        clock: Arc<dyn Clock>,
        reconciliation: ReconciliationPolicy,
        reconciliation_timer: Arc<dyn ReconciliationTimer>,
    ) -> Self {
        Self {
            store: InvocationStoreActor::spawn(store),
            clock,
            live_tasks: Mutex::new(HashMap::new()),
            inline_waiters: Mutex::new(Vec::new()),
            health: Arc::new(Mutex::new(InvocationExecutorHealth::Healthy)),
            reconciliation,
            reconciliation_timer,
        }
    }

    fn ensure_healthy(&self) -> Result<(), InvocationExecutorError> {
        match self.health.lock() {
            Ok(health) if *health == InvocationExecutorHealth::Healthy => Ok(()),
            Ok(_) | Err(_) => Err(InvocationExecutorError::RestartRequested),
        }
    }

    pub(crate) fn restart_requested(&self) -> bool {
        self.health
            .lock()
            .map(|health| *health != InvocationExecutorHealth::Healthy)
            .unwrap_or(true)
    }

    fn request_restart(&self) {
        let mut health = match self.health.lock() {
            Ok(health) => health,
            Err(_) => return,
        };
        if *health != InvocationExecutorHealth::Healthy {
            return;
        }
        *health = InvocationExecutorHealth::RestartRequested;
        drop(health);

        let mut live = self
            .live_tasks
            .lock()
            .map(|mut tasks| tasks.drain().map(|(_, live)| live).collect::<Vec<_>>())
            .unwrap_or_default();
        if let Ok(mut waiters) = self.inline_waiters.lock() {
            live.extend(waiters.drain(..).filter_map(|waiter| waiter.upgrade()));
        }
        for invocation in live {
            invocation.cancellation.cancel();
            if let Ok(mut state) = invocation.state.lock() {
                *state = LiveInvocationState::RestartRequested;
                invocation.changed.notify_all();
            }
            invocation.release_resource_lease();
        }
    }

    fn wait_before_reconciliation_retry(&self, deadline: Instant, backoff: &mut Duration) -> bool {
        let now = self.reconciliation_timer.now();
        if now >= deadline {
            return false;
        }
        let wait = (*backoff).min(deadline.saturating_duration_since(now));
        if wait.is_zero() {
            return false;
        }
        self.reconciliation_timer.wait(wait);
        *backoff = (*backoff)
            .checked_mul(2)
            .unwrap_or(self.reconciliation.max_backoff)
            .min(self.reconciliation.max_backoff);
        self.reconciliation_timer.now() < deadline
    }

    pub(crate) fn submit_prepared<F>(
        self: &Arc<Self>,
        prepared: Result<PreparedDaemonInvocation, DomainResult>,
        execute: F,
    ) -> Result<InvocationOutcome, InvocationExecutorError>
    where
        F: FnOnce(
                crate::domain::cancellation::CancellationToken,
            ) -> Result<DomainResult, InvocationFailure>
            + Send
            + 'static,
    {
        self.ensure_healthy()?;
        match prepared {
            Ok(prepared) => self.submit(prepared, execute),
            Err(invalid) => Ok(InvocationOutcome::Direct(invalid)),
        }
    }

    pub(crate) fn submit<F>(
        self: &Arc<Self>,
        prepared: PreparedDaemonInvocation,
        execute: F,
    ) -> Result<InvocationOutcome, InvocationExecutorError>
    where
        F: FnOnce(
                crate::domain::cancellation::CancellationToken,
            ) -> Result<DomainResult, InvocationFailure>
            + Send
            + 'static,
    {
        self.ensure_healthy()?;
        let invocation_id = InvocationId::new();
        if matches!(prepared.class, ExecutionClass::KnownLong(_))
            || prepared.response_budget.is_zero()
        {
            let intended = self.new_materialization_record(&prepared, invocation_id);
            let task_id = intended.task_id();
            let live = Arc::new(LiveInvocation::new(None, prepared.resource_lease.clone()));
            *live
                .state
                .lock()
                .map_err(|_| InvocationExecutorError::StatePoisoned)? =
                LiveInvocationState::Materializing {
                    task_id,
                    outcome: None,
                };
            self.insert_live(task_id, Arc::clone(&live))?;
            let store_deadline = self.reconciliation_timer.now() + self.reconciliation.budget;
            let record = match self.materialize(
                intended,
                &prepared,
                invocation_id,
                store_deadline,
                &live.cancellation,
            ) {
                Ok(record) => record,
                Err(error) => {
                    self.abandon_pending(&live, task_id);
                    return Err(error);
                }
            };
            if self.confirm_materialization(&live, task_id)?.is_some() {
                self.request_restart();
                return Err(InvocationExecutorError::RestartRequested);
            }
            let snapshot = snapshot_from_record(record.clone(), self.clock.now());
            self.spawn_execution(live, execute);
            return Ok(InvocationOutcome::Task(snapshot));
        }

        let live = Arc::new(LiveInvocation::new(None, prepared.resource_lease.clone()));
        self.inline_waiters
            .lock()
            .map_err(|_| InvocationExecutorError::StatePoisoned)?
            .push(Arc::downgrade(&live));
        // The transmitted response budget is already shrinking when the daemon receives the
        // request. Capture its local boundary before scheduling execution so thread startup can
        // never replenish the caller's handoff window.
        let deadline = self.clock.now() + prepared.response_budget;
        self.spawn_execution(Arc::clone(&live), execute);
        let mut state = live
            .state
            .lock()
            .map_err(|_| InvocationExecutorError::StatePoisoned)?;
        loop {
            match &*state {
                LiveInvocationState::Direct(result) => {
                    let result = result.as_ref().clone();
                    if self.clock.now() < deadline {
                        return result
                            .map(InvocationOutcome::Direct)
                            .map_err(|_| InvocationExecutorError::ExecutionFailed);
                    }
                    let intended = self.new_materialization_record(&prepared, invocation_id);
                    let task_id = intended.task_id();
                    *state = LiveInvocationState::Materializing {
                        task_id,
                        outcome: Some(Box::new(result)),
                    };
                    self.insert_live(task_id, Arc::clone(&live))?;
                    drop(state);
                    let store_deadline =
                        self.reconciliation_timer.now() + self.reconciliation.budget;
                    let working = match self.materialize(
                        intended,
                        &prepared,
                        invocation_id,
                        store_deadline,
                        &live.cancellation,
                    ) {
                        Ok(record) => record,
                        Err(error) => {
                            self.abandon_pending(&live, task_id);
                            return Err(error);
                        }
                    };
                    let staged = self
                        .confirm_materialization(&live, task_id)?
                        .ok_or(InvocationExecutorError::StatePoisoned)?;
                    self.spawn_terminal_reconciliation(Arc::clone(&live), task_id, staged);
                    return Ok(InvocationOutcome::Task(snapshot_from_record(
                        working,
                        self.clock.now(),
                    )));
                }
                LiveInvocationState::DurableTerminal => {
                    return Err(InvocationExecutorError::StatePoisoned);
                }
                LiveInvocationState::RestartRequested => {
                    return Err(InvocationExecutorError::RestartRequested);
                }
                LiveInvocationState::Running { task_id: Some(_) } => {
                    return Err(InvocationExecutorError::StatePoisoned);
                }
                LiveInvocationState::Running { task_id: None } => {}
                LiveInvocationState::Materializing { .. } => {
                    return Err(InvocationExecutorError::StatePoisoned);
                }
            }
            let remaining = deadline.saturating_duration_since(self.clock.now());
            if remaining.is_zero() {
                let intended = self.new_materialization_record(&prepared, invocation_id);
                let task_id = intended.task_id();
                *state = LiveInvocationState::Materializing {
                    task_id,
                    outcome: None,
                };
                self.insert_live(task_id, Arc::clone(&live))?;
                drop(state);
                let store_deadline = self.reconciliation_timer.now() + self.reconciliation.budget;
                let record = match self.materialize(
                    intended,
                    &prepared,
                    invocation_id,
                    store_deadline,
                    &live.cancellation,
                ) {
                    Ok(record) => record,
                    Err(error) => {
                        self.abandon_pending(&live, task_id);
                        return Err(error);
                    }
                };
                if let Some(staged) = self.confirm_materialization(&live, task_id)? {
                    self.spawn_terminal_reconciliation(Arc::clone(&live), task_id, staged);
                }
                return Ok(InvocationOutcome::Task(snapshot_from_record(
                    record,
                    self.clock.now(),
                )));
            }
            let (next, _) = live
                .changed
                .wait_timeout(state, remaining.min(Duration::from_millis(10)))
                .map_err(|_| InvocationExecutorError::StatePoisoned)?;
            state = next;
        }
    }

    fn new_materialization_record(
        &self,
        prepared: &PreparedDaemonInvocation,
        invocation_id: InvocationId,
    ) -> NewInvocationRecord {
        NewInvocationRecord::new(
            invocation_id,
            prepared.tool,
            prepared.normalized_arguments_hash.clone(),
            prepared.workspace_identity_hash.clone(),
            SafeStatusMessage::Queued,
            INVOCATION_POLL_INTERVAL_MS,
            INVOCATION_TTL_MS,
            None,
        )
    }

    fn materialize(
        &self,
        intended: NewInvocationRecord,
        prepared: &PreparedDaemonInvocation,
        invocation_id: InvocationId,
        deadline: Instant,
        cancellation: &crate::domain::cancellation::CancellationToken,
    ) -> Result<StoredInvocationRecord, InvocationExecutorError> {
        let intended_task_id = intended.task_id();
        match self.store.create_working(intended, deadline, cancellation) {
            Ok(record)
                if initial_working_matches(&record, intended_task_id, prepared, invocation_id) =>
            {
                Ok(record)
            }
            Ok(_) => {
                self.request_restart();
                Err(InvocationExecutorError::RestartRequested)
            }
            Err(InvocationStoreError::CommitUncertain {
                task_id,
                operation: crate::application::invocation_store::CommitOperation::Create,
            }) => {
                if task_id != intended_task_id {
                    self.request_restart();
                    return Err(InvocationExecutorError::RestartRequested);
                }
                let mut backoff = self.reconciliation.initial_backoff;
                let mut first_read = true;
                loop {
                    match self.store.get(task_id, deadline, cancellation) {
                        Ok(record)
                            if initial_working_matches(
                                &record,
                                intended_task_id,
                                prepared,
                                invocation_id,
                            ) =>
                        {
                            return Ok(record);
                        }
                        Err(InvocationStoreError::NotFound) if first_read => {
                            return Err(InvocationStoreError::NotFound.into())
                        }
                        Ok(_) | Err(_) => {}
                    }
                    first_read = false;
                    if !self.wait_before_reconciliation_retry(deadline, &mut backoff) {
                        self.request_restart();
                        return Err(InvocationExecutorError::RestartRequested);
                    }
                }
            }
            Err(
                error @ (InvocationStoreError::DeadlineExceeded
                | InvocationStoreError::Cancelled
                | InvocationStoreError::ActorUnavailable),
            ) => {
                self.request_restart();
                let _ = error;
                Err(InvocationExecutorError::RestartRequested)
            }
            Err(error) => Err(error.into()),
        }
    }

    fn confirm_materialization(
        &self,
        live: &Arc<LiveInvocation>,
        task_id: TaskId,
    ) -> Result<Option<Result<DomainResult, InvocationFailure>>, InvocationExecutorError> {
        let mut state = live
            .state
            .lock()
            .map_err(|_| InvocationExecutorError::StatePoisoned)?;
        let previous = std::mem::replace(
            &mut *state,
            LiveInvocationState::Running {
                task_id: Some(task_id),
            },
        );
        match previous {
            LiveInvocationState::Materializing {
                task_id: pending_task_id,
                outcome,
            } if pending_task_id == task_id => Ok(outcome.map(|outcome| *outcome)),
            LiveInvocationState::RestartRequested => {
                *state = LiveInvocationState::RestartRequested;
                Err(InvocationExecutorError::RestartRequested)
            }
            other => {
                *state = other;
                Err(InvocationExecutorError::StatePoisoned)
            }
        }
    }

    fn abandon_pending(&self, live: &Arc<LiveInvocation>, task_id: TaskId) {
        live.cancellation.cancel();
        if let Ok(mut state) = live.state.lock() {
            *state = LiveInvocationState::RestartRequested;
            live.changed.notify_all();
        }
        live.release_resource_lease();
        if let Ok(mut tasks) = self.live_tasks.lock() {
            if tasks
                .get(&task_id)
                .is_some_and(|registered| Arc::ptr_eq(registered, live))
            {
                tasks.remove(&task_id);
            }
        }
    }

    fn insert_live(
        &self,
        task_id: TaskId,
        live: Arc<LiveInvocation>,
    ) -> Result<(), InvocationExecutorError> {
        self.live_tasks
            .lock()
            .map_err(|_| InvocationExecutorError::StatePoisoned)?
            .insert(task_id, live);
        Ok(())
    }

    fn persist_terminal_once(
        &self,
        task_id: TaskId,
        outcome: Result<DomainResult, InvocationFailure>,
        deadline: Instant,
        cancellation: &crate::domain::cancellation::CancellationToken,
    ) -> Result<StoredInvocationRecord, InvocationStoreError> {
        let before = self.store.get(task_id, deadline, cancellation)?;
        let transition = match &outcome {
            Ok(result) => TaskTransition::Complete {
                status_message: SafeStatusMessage::Completed,
                result: Box::new(result.clone()),
            },
            Err(_) => TaskTransition::Fail {
                status_message: SafeStatusMessage::Failed,
                reason: SafeFailureReason::InvocationFailed,
            },
        };
        match self
            .store
            .update(task_id, transition, deadline, cancellation)
        {
            Ok(record)
                if same_record_identity(&before, &record)
                    && terminal_matches(&record, &outcome) =>
            {
                Ok(record)
            }
            Ok(_) => Err(InvocationStoreError::CommitUncertain {
                task_id,
                operation: crate::application::invocation_store::CommitOperation::Update,
            }),
            Err(
                error @ InvocationStoreError::CommitUncertain {
                    operation: crate::application::invocation_store::CommitOperation::Update,
                    ..
                },
            )
            | Err(error @ InvocationStoreError::InvalidTransition { .. }) => {
                match self.store.get(task_id, deadline, cancellation) {
                    Ok(record)
                        if same_record_identity(&before, &record)
                            && (terminal_matches(&record, &outcome)
                                || record.status == InvocationStatus::Cancelled) =>
                    {
                        Ok(record)
                    }
                    _ => Err(error),
                }
            }
            Err(error) => Err(error),
        }
    }

    fn spawn_terminal_reconciliation(
        self: &Arc<Self>,
        live: Arc<LiveInvocation>,
        task_id: TaskId,
        outcome: Result<DomainResult, InvocationFailure>,
    ) {
        let executor = Arc::clone(self);
        std::thread::spawn(move || {
            let deadline = executor.reconciliation_timer.now() + executor.reconciliation.budget;
            let mut backoff = executor.reconciliation.initial_backoff;
            let store_cancellation = crate::domain::cancellation::CancellationToken::new();
            loop {
                match executor.persist_terminal_once(
                    task_id,
                    outcome.clone(),
                    deadline,
                    &store_cancellation,
                ) {
                    Ok(_) => {
                        if let Ok(mut state) = live.state.lock() {
                            *state = LiveInvocationState::DurableTerminal;
                            live.changed.notify_all();
                        }
                        live.release_resource_lease();
                        if let Ok(mut tasks) = executor.live_tasks.lock() {
                            tasks.remove(&task_id);
                        }
                        return;
                    }
                    Err(_) if executor.wait_before_reconciliation_retry(deadline, &mut backoff) => {
                    }
                    Err(_) => {
                        executor.request_restart();
                        return;
                    }
                }
            }
        });
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
                LiveInvocationState::Materializing { .. } => {
                    if let LiveInvocationState::Materializing {
                        outcome: staged, ..
                    } = &mut *state
                    {
                        *staged = Some(Box::new(outcome));
                        live.changed.notify_all();
                    }
                    return;
                }
                LiveInvocationState::Direct(_)
                | LiveInvocationState::DurableTerminal
                | LiveInvocationState::RestartRequested => return,
            };
            if let Some(task_id) = task_id {
                // A committed cancellation wins over late completion. The
                // failed transition is deliberately not retried or published.
                drop(state);
                executor.spawn_terminal_reconciliation(live, task_id, outcome);
            } else {
                *state = LiveInvocationState::Direct(Box::new(outcome));
                live.changed.notify_all();
            }
        });
    }

    pub(crate) fn get_task(
        &self,
        task_id: TaskId,
    ) -> Result<TaskSnapshot, InvocationExecutorError> {
        self.ensure_healthy()?;
        let cancellation = crate::domain::cancellation::CancellationToken::new();
        let deadline = Instant::now() + RECONCILIATION_BUDGET;
        self.get_task_before(task_id, deadline, &cancellation)
    }

    fn get_task_before(
        &self,
        task_id: TaskId,
        deadline: Instant,
        cancellation: &crate::domain::cancellation::CancellationToken,
    ) -> Result<TaskSnapshot, InvocationExecutorError> {
        self.handle_store_call(self.store.get(task_id, deadline, cancellation))
            .map(|record| snapshot_from_record(record, self.clock.now()))
    }

    pub(crate) fn wait_task(
        &self,
        task_id: TaskId,
        wait: Duration,
    ) -> Result<TaskSnapshot, InvocationExecutorError> {
        self.ensure_healthy()?;
        let cancellation = crate::domain::cancellation::CancellationToken::new();
        let deadline = Instant::now() + wait + RECONCILIATION_BUDGET;
        let current = self.get_task_before(task_id, deadline, &cancellation)?;
        if current.status != InvocationStatus::Working || wait.is_zero() {
            return Ok(current);
        }
        let live = self
            .live_tasks
            .lock()
            .map_err(|_| InvocationExecutorError::StatePoisoned)?
            .get(&task_id)
            .cloned();
        if let Some(live) = live {
            let state = live
                .state
                .lock()
                .map_err(|_| InvocationExecutorError::StatePoisoned)?;
            let _ = live
                .changed
                .wait_timeout(state, wait)
                .map_err(|_| InvocationExecutorError::StatePoisoned)?;
        }
        self.get_task_before(task_id, deadline, &cancellation)
    }

    pub(crate) fn cancel_task(
        &self,
        task_id: TaskId,
    ) -> Result<TaskSnapshot, InvocationExecutorError> {
        self.ensure_healthy()?;
        let cancellation = crate::domain::cancellation::CancellationToken::new();
        let deadline = self.reconciliation_timer.now() + self.reconciliation.budget;
        let before = self.handle_store_call(self.store.get(task_id, deadline, &cancellation))?;
        let record = match self.store.cancel(
            task_id,
            SafeStatusMessage::Cancelled,
            deadline,
            &cancellation,
        ) {
            Ok(record) if same_record_identity(&before, &record) && record.is_terminal() => record,
            Ok(_) => {
                self.confirm_terminal_after_cancel(&before, task_id, deadline, &cancellation)?
            }
            Err(InvocationStoreError::CommitUncertain {
                operation: crate::application::invocation_store::CommitOperation::Cancel,
                ..
            })
            | Err(InvocationStoreError::InvalidTransition { .. }) => {
                self.confirm_terminal_after_cancel(&before, task_id, deadline, &cancellation)?
            }
            Err(
                error @ (InvocationStoreError::DeadlineExceeded
                | InvocationStoreError::Cancelled
                | InvocationStoreError::ActorUnavailable),
            ) => {
                let _ = error;
                self.request_restart();
                return Err(InvocationExecutorError::RestartRequested);
            }
            Err(error) => return Err(error.into()),
        };
        if record.status == InvocationStatus::Cancelled {
            if let Some(live) = self
                .live_tasks
                .lock()
                .map_err(|_| InvocationExecutorError::StatePoisoned)?
                .get(&task_id)
                .cloned()
            {
                live.cancellation.cancel();
                live.changed.notify_all();
            }
        }
        Ok(snapshot_from_record(record, self.clock.now()))
    }

    fn confirm_terminal_after_cancel(
        &self,
        before: &StoredInvocationRecord,
        task_id: TaskId,
        deadline: Instant,
        cancellation: &crate::domain::cancellation::CancellationToken,
    ) -> Result<StoredInvocationRecord, InvocationExecutorError> {
        let mut backoff = self.reconciliation.initial_backoff;
        loop {
            match self.store.get(task_id, deadline, cancellation) {
                Ok(record) if same_record_identity(before, &record) && record.is_terminal() => {
                    return Ok(record)
                }
                Ok(_) | Err(_) => {}
            }
            if !self.wait_before_reconciliation_retry(deadline, &mut backoff) {
                self.request_restart();
                return Err(InvocationExecutorError::RestartRequested);
            }
        }
    }

    fn handle_store_call<T>(
        &self,
        result: Result<T, InvocationStoreError>,
    ) -> Result<T, InvocationExecutorError> {
        match result {
            Ok(value) => Ok(value),
            Err(
                InvocationStoreError::DeadlineExceeded
                | InvocationStoreError::Cancelled
                | InvocationStoreError::ActorUnavailable,
            ) => {
                self.request_restart();
                Err(InvocationExecutorError::RestartRequested)
            }
            Err(error) => Err(error.into()),
        }
    }

    pub(crate) fn has_active_invocations(&self) -> bool {
        match self.live_tasks.lock() {
            Ok(tasks) if !tasks.is_empty() => return true,
            Err(_) => return true,
            Ok(_) => {}
        }
        match self.inline_waiters.lock() {
            Ok(mut waiters) => {
                waiters.retain(|waiter| waiter.strong_count() > 0);
                !waiters.is_empty()
            }
            Err(_) => true,
        }
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

fn initial_working_matches(
    record: &StoredInvocationRecord,
    expected_task_id: TaskId,
    prepared: &PreparedDaemonInvocation,
    invocation_id: InvocationId,
) -> bool {
    record.schema_version == crate::application::invocation_store::INVOCATION_RECORD_SCHEMA_VERSION
        && record.task_id == expected_task_id
        && record.invocation_id == invocation_id
        && record.tool == prepared.tool
        && record.normalized_arguments_hash == prepared.normalized_arguments_hash
        && record.workspace_identity_hash == prepared.workspace_identity_hash
        && record.status == InvocationStatus::Working
        && record.status_message == SafeStatusMessage::Working
        && record.poll_interval_ms == INVOCATION_POLL_INTERVAL_MS
        && record.ttl_ms == INVOCATION_TTL_MS
        && record.result.is_none()
        && record.failure_reason.is_none()
        && record.resume.is_none()
        && record.created_at_epoch_ms == record.updated_at_epoch_ms
}

fn terminal_matches(
    record: &StoredInvocationRecord,
    outcome: &Result<DomainResult, InvocationFailure>,
) -> bool {
    match outcome {
        Ok(result) => {
            record.status == InvocationStatus::Completed
                && record.status_message == SafeStatusMessage::Completed
                && record.result.as_ref() == Some(result)
                && record.failure_reason.is_none()
                && record.resume.is_none()
        }
        Err(_) => {
            record.status == InvocationStatus::Failed
                && record.status_message == SafeStatusMessage::Failed
                && record.result.is_none()
                && record.failure_reason == Some(SafeFailureReason::InvocationFailed)
                && record.resume.is_none()
        }
    }
}

fn same_record_identity(left: &StoredInvocationRecord, right: &StoredInvocationRecord) -> bool {
    left.schema_version == right.schema_version
        && left.task_id == right.task_id
        && left.invocation_id == right.invocation_id
        && left.tool == right.tool
        && left.normalized_arguments_hash == right.normalized_arguments_hash
        && left.workspace_identity_hash == right.workspace_identity_hash
        && left.created_at_epoch_ms == right.created_at_epoch_ms
        && left.poll_interval_ms == right.poll_interval_ms
        && left.ttl_ms == right.ttl_ms
}

fn snapshot_from_record(record: StoredInvocationRecord, observed_at: Instant) -> TaskSnapshot {
    let failure = record.failure_reason.map(|reason| match reason {
        SafeFailureReason::InvocationFailed => {
            InvocationFailure::new("invocation_failed", "daemon invocation failed")
        }
        SafeFailureReason::Interrupted => {
            InvocationFailure::new("interrupted", "daemon invocation was interrupted")
        }
        SafeFailureReason::ResumeUnsupported => InvocationFailure::new(
            "resume_unsupported",
            "daemon invocation cannot be resumed after restart",
        ),
        SafeFailureReason::PersistenceFailed => InvocationFailure::new(
            "persistence_failed",
            "daemon invocation terminal state could not be persisted",
        ),
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
        InvocationStore, InvocationStoreError, NewInvocationRecord, SafeFailureReason,
        SafeStatusMessage, StoredInvocationRecord, TaskTransition, ToolIdentity,
    };
    use crate::application::operation_descriptors::{ExecutionClass, KnownLongReason};
    use crate::application::ports::{Clock, TokioClock};
    use crate::application::OperationResult;
    use crate::domain::invocation::{
        DomainResult, InvocationFailure, InvocationId, InvocationOutcome, InvocationStatus,
        NormalizedArgumentsHash, SafeIdentityHash, TaskId,
    };
    use serde_json::json;
    use std::collections::{HashMap, VecDeque};
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

    struct RecordingReconciliationTimer {
        now: Mutex<Instant>,
        waits: Mutex<Vec<Duration>>,
    }

    impl RecordingReconciliationTimer {
        fn new(now: Instant) -> Self {
            Self {
                now: Mutex::new(now),
                waits: Mutex::new(Vec::new()),
            }
        }
    }

    impl super::ReconciliationTimer for RecordingReconciliationTimer {
        fn now(&self) -> Instant {
            *self.now.lock().unwrap()
        }

        fn wait(&self, duration: Duration) {
            self.waits.lock().unwrap().push(duration);
            *self.now.lock().unwrap() += duration;
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
        queued_creates: AtomicUsize,
        working_creates: AtomicUsize,
        create_working_fault: Mutex<Option<Arc<CommitFault>>>,
        create_record_mismatch: AtomicUsize,
        create_task_id_mismatch: AtomicUsize,
        get_faults: Mutex<VecDeque<InvocationStoreError>>,
        get_always_fails: AtomicUsize,
        update_fault: Mutex<Option<Arc<CommitFault>>>,
        update_always_fails: AtomicUsize,
        update_attempts: AtomicUsize,
        cancel_fault: Mutex<Option<Arc<CommitFault>>>,
        cancel_returns_invalid_transition: AtomicUsize,
        cancel_always_uncertain: AtomicUsize,
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum CommitFaultTiming {
        Before,
        After,
    }

    struct CommitFault {
        timing: CommitFaultTiming,
        entered: Mutex<Option<mpsc::Sender<()>>>,
        release: Mutex<mpsc::Receiver<()>>,
    }

    impl CommitFault {
        fn new(timing: CommitFaultTiming) -> (Arc<Self>, mpsc::Receiver<()>, mpsc::Sender<()>) {
            let (entered_send, entered_wait) = mpsc::channel();
            let (release_send, release_wait) = mpsc::channel();
            (
                Arc::new(Self {
                    timing,
                    entered: Mutex::new(Some(entered_send)),
                    release: Mutex::new(release_wait),
                }),
                entered_wait,
                release_send,
            )
        }

        fn checkpoint(&self, timing: CommitFaultTiming) -> bool {
            if self.timing != timing {
                return false;
            }
            if let Some(entered) = self.entered.lock().unwrap().take() {
                entered.send(()).unwrap();
            }
            self.release.lock().unwrap().recv().unwrap();
            true
        }
    }

    impl InvocationStore for MemoryStore {
        fn create(
            &self,
            record: NewInvocationRecord,
        ) -> Result<StoredInvocationRecord, InvocationStoreError> {
            self.queued_creates.fetch_add(1, Ordering::SeqCst);
            let stored = record.into_stored(1);
            self.records
                .lock()
                .unwrap()
                .insert(stored.task_id, stored.clone());
            Ok(stored)
        }

        fn get(&self, task_id: TaskId) -> Result<StoredInvocationRecord, InvocationStoreError> {
            if self.get_always_fails.load(Ordering::SeqCst) != 0 {
                return Err(InvocationStoreError::Storage(
                    "permanent confirming read failure".into(),
                ));
            }
            if let Some(error) = self.get_faults.lock().unwrap().pop_front() {
                return Err(error);
            }
            self.records
                .lock()
                .unwrap()
                .get(&task_id)
                .cloned()
                .ok_or(InvocationStoreError::NotFound)
        }

        fn create_working(
            &self,
            record: NewInvocationRecord,
        ) -> Result<StoredInvocationRecord, InvocationStoreError> {
            self.working_creates.fetch_add(1, Ordering::SeqCst);
            let stored = record.into_working_stored(1);
            let fault = self.create_working_fault.lock().unwrap().take();
            if fault
                .as_ref()
                .is_some_and(|fault| fault.checkpoint(CommitFaultTiming::Before))
            {
                return Err(InvocationStoreError::Storage(
                    "injected pre-commit create failure".into(),
                ));
            }
            let mut persisted = stored.clone();
            if self.create_record_mismatch.load(Ordering::SeqCst) != 0 {
                persisted.workspace_identity_hash = SafeIdentityHash::from_sha256([0x99; 32]);
            }
            if self.create_task_id_mismatch.load(Ordering::SeqCst) != 0 {
                persisted.task_id = TaskId::new();
            }
            self.records
                .lock()
                .unwrap()
                .insert(persisted.task_id, persisted.clone());
            if fault
                .as_ref()
                .is_some_and(|fault| fault.checkpoint(CommitFaultTiming::After))
            {
                return Err(InvocationStoreError::CommitUncertain {
                    task_id: stored.task_id,
                    operation: crate::application::invocation_store::CommitOperation::Create,
                });
            }
            Ok(persisted)
        }

        fn update(
            &self,
            task_id: TaskId,
            transition: TaskTransition,
        ) -> Result<StoredInvocationRecord, InvocationStoreError> {
            self.update_attempts.fetch_add(1, Ordering::SeqCst);
            if self.update_always_fails.load(Ordering::SeqCst) != 0 {
                return Err(InvocationStoreError::Storage(
                    "permanent terminal publication failure".into(),
                ));
            }
            let fault = self.update_fault.lock().unwrap().take();
            if fault
                .as_ref()
                .is_some_and(|fault| fault.checkpoint(CommitFaultTiming::Before))
            {
                return Err(InvocationStoreError::Storage(
                    "injected pre-commit update failure".into(),
                ));
            }
            let mut records = self.records.lock().unwrap();
            let record = records
                .get_mut(&task_id)
                .ok_or(InvocationStoreError::NotFound)?;
            if record.is_terminal() {
                return Err(InvocationStoreError::InvalidTransition {
                    from: record.status,
                    attempted: "update",
                });
            }
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
                TaskTransition::Fail {
                    status_message,
                    reason,
                } => {
                    record.status = InvocationStatus::Failed;
                    record.status_message = status_message;
                    record.failure_reason = Some(reason);
                }
            }
            let stored = record.clone();
            drop(records);
            if fault
                .as_ref()
                .is_some_and(|fault| fault.checkpoint(CommitFaultTiming::After))
            {
                return Err(InvocationStoreError::CommitUncertain {
                    task_id,
                    operation: crate::application::invocation_store::CommitOperation::Update,
                });
            }
            Ok(stored)
        }

        fn cancel(
            &self,
            task_id: TaskId,
            status_message: SafeStatusMessage,
        ) -> Result<StoredInvocationRecord, InvocationStoreError> {
            if self
                .cancel_returns_invalid_transition
                .swap(0, Ordering::SeqCst)
                != 0
            {
                let from = self
                    .records
                    .lock()
                    .unwrap()
                    .get(&task_id)
                    .map(|record| record.status)
                    .ok_or(InvocationStoreError::NotFound)?;
                return Err(InvocationStoreError::InvalidTransition {
                    from,
                    attempted: "cancel",
                });
            }
            if self.cancel_always_uncertain.load(Ordering::SeqCst) != 0 {
                return Err(InvocationStoreError::CommitUncertain {
                    task_id,
                    operation: crate::application::invocation_store::CommitOperation::Cancel,
                });
            }
            let fault = self.cancel_fault.lock().unwrap().take();
            if fault
                .as_ref()
                .is_some_and(|fault| fault.checkpoint(CommitFaultTiming::Before))
            {
                return Err(InvocationStoreError::Storage(
                    "injected pre-commit cancel failure".into(),
                ));
            }
            let mut records = self.records.lock().unwrap();
            let record = records
                .get_mut(&task_id)
                .ok_or(InvocationStoreError::NotFound)?;
            if !record.is_terminal() {
                record.status = InvocationStatus::Cancelled;
                record.status_message = status_message;
                record.result = None;
            }
            let stored = record.clone();
            drop(records);
            if fault
                .as_ref()
                .is_some_and(|fault| fault.checkpoint(CommitFaultTiming::After))
            {
                return Err(InvocationStoreError::CommitUncertain {
                    task_id,
                    operation: crate::application::invocation_store::CommitOperation::Cancel,
                });
            }
            Ok(stored)
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
    fn materialization_atomically_creates_the_initial_working_record() {
        let store = Arc::new(MemoryStore::default());
        let executor = Arc::new(InvocationExecutor::new(
            store.clone(),
            Arc::new(ManualClock::new(Instant::now())),
        ));
        let outcome = executor
            .submit(
                prepared(
                    ExecutionClass::KnownLong(KnownLongReason::ColdIndex),
                    Duration::ZERO,
                ),
                |_| Ok(result("atomic working")),
            )
            .unwrap();
        assert!(matches!(outcome, InvocationOutcome::Task(_)));
        assert_eq!(store.queued_creates.load(Ordering::SeqCst), 0);
        assert_eq!(store.working_creates.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn create_working_faults_are_resolved_by_exact_identity_bound_readback() {
        for timing in [CommitFaultTiming::Before, CommitFaultTiming::After] {
            let store = Arc::new(MemoryStore::default());
            let (fault, entered, release) = CommitFault::new(timing);
            *store.create_working_fault.lock().unwrap() = Some(fault);
            let executor = Arc::new(InvocationExecutor::new(
                store.clone(),
                Arc::new(ManualClock::new(Instant::now())),
            ));
            let run = {
                let executor = Arc::clone(&executor);
                std::thread::spawn(move || {
                    executor.submit(
                        prepared(
                            ExecutionClass::KnownLong(KnownLongReason::ColdIndex),
                            Duration::ZERO,
                        ),
                        |_| Ok(result("created exactly once")),
                    )
                })
            };
            entered.recv().unwrap();
            assert_eq!(
                store.records.lock().unwrap().len(),
                usize::from(timing == CommitFaultTiming::After)
            );
            release.send(()).unwrap();
            let outcome = run.join().unwrap();
            match timing {
                CommitFaultTiming::Before => assert!(matches!(
                    outcome,
                    Err(super::InvocationExecutorError::Store(
                        InvocationStoreError::Storage(_)
                    ))
                )),
                CommitFaultTiming::After => {
                    let task_id = match outcome.unwrap() {
                        InvocationOutcome::Task(snapshot) => snapshot.task_id,
                        other => panic!("expected task after confirmed create: {other:?}"),
                    };
                    assert_eq!(
                        executor
                            .wait_task(task_id, Duration::from_secs(1))
                            .unwrap()
                            .status,
                        InvocationStatus::Completed
                    );
                }
            }
        }
    }

    #[test]
    fn uncertain_create_transient_readback_is_reconciled_before_execution() {
        let store = Arc::new(MemoryStore::default());
        let (fault, entered, release) = CommitFault::new(CommitFaultTiming::After);
        *store.create_working_fault.lock().unwrap() = Some(fault);
        store
            .get_faults
            .lock()
            .unwrap()
            .push_back(InvocationStoreError::Storage(
                "transient confirming read failure".into(),
            ));
        let executor = Arc::new(InvocationExecutor::new(
            store.clone(),
            Arc::new(ManualClock::new(Instant::now())),
        ));
        let executions = Arc::new(AtomicUsize::new(0));
        let run_count = Arc::clone(&executions);
        let run = {
            let executor = Arc::clone(&executor);
            std::thread::spawn(move || {
                executor.submit(
                    prepared(
                        ExecutionClass::KnownLong(KnownLongReason::ColdIndex),
                        Duration::ZERO,
                    ),
                    move |_| {
                        run_count.fetch_add(1, Ordering::SeqCst);
                        Ok(result("confirmed after transient readback"))
                    },
                )
            })
        };

        entered.recv().unwrap();
        assert_eq!(store.records.lock().unwrap().len(), 1);
        assert_eq!(executions.load(Ordering::SeqCst), 0);
        let pending_task_id = *store.records.lock().unwrap().keys().next().unwrap();
        let pending_is_owned = executor
            .live_tasks
            .lock()
            .unwrap()
            .contains_key(&pending_task_id);
        release.send(()).unwrap();
        assert!(
            pending_is_owned,
            "the exact preallocated TaskId must have live actor ownership before confirmation"
        );

        let outcome = run
            .join()
            .unwrap()
            .expect("bounded readback reconciliation confirms exact Working record");
        let task_id = match outcome {
            InvocationOutcome::Task(snapshot) => snapshot.task_id,
            other => panic!("confirmed uncertain create must hand off: {other:?}"),
        };
        assert_eq!(
            executor
                .wait_task(task_id, Duration::from_secs(1))
                .unwrap()
                .status,
            InvocationStatus::Completed
        );
        assert_eq!(executions.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn completion_at_handoff_reconciles_create_without_exposing_staged_result() {
        for transient in [true, false] {
            let store = Arc::new(MemoryStore::default());
            let (fault, fault_entered, fault_release) = CommitFault::new(CommitFaultTiming::After);
            *store.create_working_fault.lock().unwrap() = Some(fault);
            if transient {
                store
                    .get_faults
                    .lock()
                    .unwrap()
                    .push_back(InvocationStoreError::Storage(
                        "transient completion-handoff read failure".into(),
                    ));
            } else {
                store.get_always_fails.store(1, Ordering::SeqCst);
            }
            let clock = Arc::new(ManualClock::new(Instant::now()));
            let executor = Arc::new(InvocationExecutor::new_with_reconciliation_budget_for_test(
                store.clone(),
                clock.clone(),
                Duration::from_secs(1),
            ));
            let executions = Arc::new(AtomicUsize::new(0));
            let run_count = Arc::clone(&executions);
            let (execution_entered, execution_entered_wait) = mpsc::channel();
            let (execution_release, execution_wait) = mpsc::channel();
            let run = {
                let executor = Arc::clone(&executor);
                std::thread::spawn(move || {
                    executor.submit(
                        prepared(ExecutionClass::InlineCandidate, Duration::from_secs(7)),
                        move |_| {
                            run_count.fetch_add(1, Ordering::SeqCst);
                            execution_entered.send(()).unwrap();
                            execution_wait.recv().unwrap();
                            Ok(result("completion staged at handoff"))
                        },
                    )
                })
            };
            execution_entered_wait.recv().unwrap();
            clock.advance(Duration::from_secs(7));
            execution_release.send(()).unwrap();
            executor.wake_deadline_waiters_for_test();
            fault_entered.recv().unwrap();
            let record = store
                .records
                .lock()
                .unwrap()
                .values()
                .next()
                .unwrap()
                .clone();
            assert_eq!(record.status, InvocationStatus::Working);
            assert!(record.result.is_none());
            assert!(executor.has_active_invocations());
            fault_release.send(()).unwrap();

            if transient {
                let task_id = match run.join().unwrap().unwrap() {
                    InvocationOutcome::Task(snapshot) => snapshot.task_id,
                    other => panic!("completion-at-handoff must be durable: {other:?}"),
                };
                assert_eq!(
                    executor
                        .wait_task(task_id, Duration::from_secs(1))
                        .unwrap()
                        .status,
                    InvocationStatus::Completed
                );
            } else {
                assert!(matches!(
                    run.join().unwrap(),
                    Err(super::InvocationExecutorError::RestartRequested)
                ));
                assert!(executor.restart_requested());
                assert!(store
                    .records
                    .lock()
                    .unwrap()
                    .values()
                    .all(|record| record.result.is_none()));
            }
            assert_eq!(executions.load(Ordering::SeqCst), 1);
        }
    }

    #[test]
    fn permanent_uncertain_create_fails_closed_and_stops_new_execution() {
        struct LeaseDrop(Arc<AtomicUsize>);
        impl Drop for LeaseDrop {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }

        for mismatch in [false, true] {
            let store = Arc::new(MemoryStore::default());
            let (fault, entered, release) = CommitFault::new(CommitFaultTiming::After);
            *store.create_working_fault.lock().unwrap() = Some(fault);
            if mismatch {
                store.create_record_mismatch.store(1, Ordering::SeqCst);
            } else {
                store.get_always_fails.store(1, Ordering::SeqCst);
            }
            let executor = Arc::new(InvocationExecutor::new_with_reconciliation_budget_for_test(
                store.clone(),
                Arc::new(ManualClock::new(Instant::now())),
                Duration::from_secs(1),
            ));
            let executions = Arc::new(AtomicUsize::new(0));
            let run_count = Arc::clone(&executions);
            let lease_drops = Arc::new(AtomicUsize::new(0));
            let run = {
                let executor = Arc::clone(&executor);
                let lease_drops = Arc::clone(&lease_drops);
                std::thread::spawn(move || {
                    executor.submit(
                        prepared(
                            ExecutionClass::KnownLong(KnownLongReason::ColdIndex),
                            Duration::ZERO,
                        )
                        .with_resource_lease(Arc::new(LeaseDrop(lease_drops))),
                        move |_| {
                            run_count.fetch_add(1, Ordering::SeqCst);
                            Ok(result("must never execute"))
                        },
                    )
                })
            };

            entered.recv().unwrap();
            assert_eq!(executions.load(Ordering::SeqCst), 0);
            assert!(executor.has_active_invocations());
            release.send(()).unwrap();
            assert!(matches!(
                run.join().unwrap(),
                Err(super::InvocationExecutorError::RestartRequested)
            ));
            assert!(executor.restart_requested());
            assert!(!executor.has_active_invocations());
            assert_eq!(lease_drops.load(Ordering::SeqCst), 1);
            assert_eq!(store.records.lock().unwrap().len(), 1);
            assert_eq!(executions.load(Ordering::SeqCst), 0);

            let second_run_count = Arc::clone(&executions);
            let second = executor.submit(
                prepared(
                    ExecutionClass::KnownLong(KnownLongReason::ColdIndex),
                    Duration::ZERO,
                ),
                move |_| {
                    second_run_count.fetch_add(1, Ordering::SeqCst);
                    Ok(result("second invocation must not execute"))
                },
            );
            assert!(matches!(
                second,
                Err(super::InvocationExecutorError::RestartRequested)
            ));
            assert_eq!(executions.load(Ordering::SeqCst), 0);
        }
    }

    #[test]
    fn normal_ok_create_mismatch_fails_stop_before_execution_and_disclosure() {
        for different_task_id in [false, true] {
            let store = Arc::new(MemoryStore::default());
            if different_task_id {
                store.create_task_id_mismatch.store(1, Ordering::SeqCst);
            } else {
                store.create_record_mismatch.store(1, Ordering::SeqCst);
            }
            let executor = Arc::new(InvocationExecutor::new_with_reconciliation_budget_for_test(
                store.clone(),
                Arc::new(ManualClock::new(Instant::now())),
                Duration::from_secs(1),
            ));
            let executions = Arc::new(AtomicUsize::new(0));
            let run_count = Arc::clone(&executions);

            let outcome = executor.submit(
                prepared(
                    ExecutionClass::KnownLong(KnownLongReason::ColdIndex),
                    Duration::ZERO,
                ),
                move |_| {
                    run_count.fetch_add(1, Ordering::SeqCst);
                    Ok(result("foreign create result must stay hidden"))
                },
            );

            assert!(matches!(
                outcome,
                Err(super::InvocationExecutorError::RestartRequested)
            ));
            assert!(executor.restart_requested());
            assert_eq!(executions.load(Ordering::SeqCst), 0);
            assert!(!executor.has_active_invocations());
            assert!(store
                .records
                .lock()
                .unwrap()
                .values()
                .all(|record| record.result.is_none()));
        }
    }

    #[test]
    fn blocked_create_store_does_not_hold_caller_execution_or_actor_capability_past_budget() {
        struct LeaseDrop(Arc<AtomicUsize>);
        impl Drop for LeaseDrop {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }

        let store = Arc::new(MemoryStore::default());
        let (fault, entered, release) = CommitFault::new(CommitFaultTiming::After);
        *store.create_working_fault.lock().unwrap() = Some(fault);
        let executor = Arc::new(InvocationExecutor::new_with_reconciliation_budget_for_test(
            store,
            Arc::new(ManualClock::new(Instant::now())),
            Duration::from_millis(40),
        ));
        let executions = Arc::new(AtomicUsize::new(0));
        let run_count = Arc::clone(&executions);
        let lease_drops = Arc::new(AtomicUsize::new(0));
        let (finished, finished_wait) = mpsc::channel();
        let run = {
            let executor = Arc::clone(&executor);
            let lease_drops = Arc::clone(&lease_drops);
            std::thread::spawn(move || {
                let outcome = executor.submit(
                    prepared(
                        ExecutionClass::KnownLong(KnownLongReason::ColdIndex),
                        Duration::ZERO,
                    )
                    .with_resource_lease(Arc::new(LeaseDrop(lease_drops))),
                    move |_| {
                        run_count.fetch_add(1, Ordering::SeqCst);
                        Ok(result("blocked store must not permit execution"))
                    },
                );
                finished.send(outcome).unwrap();
            })
        };

        entered.recv_timeout(Duration::from_secs(1)).unwrap();
        let bounded = finished_wait.recv_timeout(Duration::from_millis(250));
        release.send(()).unwrap();
        let outcome = bounded.expect("caller must drain while the store adapter remains blocked");
        assert!(matches!(
            outcome,
            Err(super::InvocationExecutorError::RestartRequested)
        ));
        assert_eq!(executions.load(Ordering::SeqCst), 0);
        assert_eq!(lease_drops.load(Ordering::SeqCst), 1);
        run.join().unwrap();
    }

    #[test]
    fn permanent_terminal_failure_uses_bounded_policy_then_requires_restart() {
        struct LeaseDrop(Arc<AtomicUsize>);
        impl Drop for LeaseDrop {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }

        let store = Arc::new(MemoryStore::default());
        store.update_always_fails.store(1, Ordering::SeqCst);
        let executor = Arc::new(InvocationExecutor::new_with_reconciliation_budget_for_test(
            store.clone(),
            Arc::new(ManualClock::new(Instant::now())),
            Duration::from_secs(1),
        ));
        let executions = Arc::new(AtomicUsize::new(0));
        let run_count = Arc::clone(&executions);
        let lease_drops = Arc::new(AtomicUsize::new(0));
        let outcome = executor
            .submit(
                prepared(
                    ExecutionClass::KnownLong(KnownLongReason::ExternalProcess),
                    Duration::ZERO,
                )
                .with_resource_lease(Arc::new(LeaseDrop(Arc::clone(&lease_drops)))),
                move |_| {
                    run_count.fetch_add(1, Ordering::SeqCst);
                    Ok(result("staged terminal must stay hidden"))
                },
            )
            .unwrap();
        let task_id = match outcome {
            InvocationOutcome::Task(snapshot) => snapshot.task_id,
            other => panic!("expected task: {other:?}"),
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        while !executor.restart_requested() && Instant::now() < deadline {
            std::thread::yield_now();
        }

        assert!(executor.restart_requested());
        assert!(!executor.has_active_invocations());
        assert_eq!(executions.load(Ordering::SeqCst), 1);
        assert!(store.update_attempts.load(Ordering::SeqCst) <= 10);
        let persisted = store.records.lock().unwrap()[&task_id].clone();
        assert_eq!(persisted.status, InvocationStatus::Working);
        assert!(persisted.result.is_none());
        assert_eq!(lease_drops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn restart_request_does_not_claim_noncooperative_execution_released_in_process() {
        struct ExecutionDrop(Arc<AtomicUsize>);
        impl Drop for ExecutionDrop {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }

        let store = Arc::new(MemoryStore::default());
        store.cancel_always_uncertain.store(1, Ordering::SeqCst);
        let executor = Arc::new(InvocationExecutor::new_with_reconciliation_budget_for_test(
            store,
            Arc::new(ManualClock::new(Instant::now())),
            Duration::from_millis(100),
        ));
        let execution_drops = Arc::new(AtomicUsize::new(0));
        let (started, started_wait) = mpsc::channel();
        let (release, release_wait) = mpsc::channel();
        let probe = ExecutionDrop(Arc::clone(&execution_drops));
        let outcome = executor
            .submit(
                prepared(
                    ExecutionClass::KnownLong(KnownLongReason::ExternalProcess),
                    Duration::ZERO,
                ),
                move |_| {
                    let _probe = probe;
                    started.send(()).unwrap();
                    release_wait.recv().unwrap();
                    Ok(result("noncooperative staged result"))
                },
            )
            .unwrap();
        let task_id = match outcome {
            InvocationOutcome::Task(snapshot) => snapshot.task_id,
            other => panic!("expected task: {other:?}"),
        };
        started_wait.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(matches!(
            executor.cancel_task(task_id),
            Err(super::InvocationExecutorError::RestartRequested)
        ));
        let deadline = Instant::now() + Duration::from_secs(10);
        while !executor.restart_requested() && Instant::now() < deadline {
            std::thread::yield_now();
        }

        assert!(executor.restart_requested());
        assert_eq!(execution_drops.load(Ordering::SeqCst), 0);
        release.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while execution_drops.load(Ordering::SeqCst) == 0 && Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert_eq!(execution_drops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn terminal_reconciliation_uses_bounded_exponential_backoff() {
        let store = Arc::new(MemoryStore::default());
        store.update_always_fails.store(1, Ordering::SeqCst);
        let timer = Arc::new(RecordingReconciliationTimer::new(Instant::now()));
        let executor = Arc::new(InvocationExecutor::new_with_reconciliation_for_test(
            store.clone(),
            Arc::new(ManualClock::new(Instant::now())),
            super::ReconciliationPolicy::with_budget_for_test(Duration::from_millis(75)),
            timer.clone(),
        ));
        let outcome = executor
            .submit(
                prepared(
                    ExecutionClass::KnownLong(KnownLongReason::ExternalProcess),
                    Duration::ZERO,
                ),
                |_| Ok(result("must not publish")),
            )
            .unwrap();
        assert!(matches!(outcome, InvocationOutcome::Task(_)));
        let deadline = Instant::now() + Duration::from_secs(1);
        while !executor.restart_requested() && Instant::now() < deadline {
            std::thread::yield_now();
        }

        assert!(executor.restart_requested());
        assert_eq!(
            *timer.waits.lock().unwrap(),
            vec![
                Duration::from_millis(10),
                Duration::from_millis(20),
                Duration::from_millis(40),
                Duration::from_millis(5),
            ]
        );
        assert_eq!(store.update_attempts.load(Ordering::SeqCst), 4);
    }

    fn wait_until_inactive(executor: &InvocationExecutor) {
        let deadline = Instant::now() + Duration::from_secs(1);
        while executor.has_active_invocations() && Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert!(!executor.has_active_invocations());
    }

    #[test]
    fn terminal_complete_and_fail_faults_reconcile_without_reexecution_or_early_idle() {
        for timing in [CommitFaultTiming::Before, CommitFaultTiming::After] {
            for should_fail in [false, true] {
                let store = Arc::new(MemoryStore::default());
                let (fault, entered, release) = CommitFault::new(timing);
                *store.update_fault.lock().unwrap() = Some(fault);
                let executor = Arc::new(InvocationExecutor::new(
                    store.clone(),
                    Arc::new(ManualClock::new(Instant::now())),
                ));
                let executions = Arc::new(AtomicUsize::new(0));
                let run_count = Arc::clone(&executions);
                let outcome = executor
                    .submit(
                        prepared(
                            ExecutionClass::KnownLong(KnownLongReason::ExternalProcess),
                            Duration::ZERO,
                        ),
                        move |_| {
                            run_count.fetch_add(1, Ordering::SeqCst);
                            if should_fail {
                                Err(InvocationFailure::new(
                                    "SECRET_RUNTIME_CODE",
                                    "/private/runtime/error SECRET",
                                ))
                            } else {
                                Ok(result("terminal result"))
                            }
                        },
                    )
                    .unwrap();
                let task_id = match outcome {
                    InvocationOutcome::Task(snapshot) => snapshot.task_id,
                    other => panic!("expected materialized task: {other:?}"),
                };
                entered.recv().unwrap();
                assert!(executor.has_active_invocations());
                let observed = store.get(task_id).unwrap();
                assert_eq!(
                    observed.status,
                    if timing == CommitFaultTiming::Before {
                        InvocationStatus::Working
                    } else if should_fail {
                        InvocationStatus::Failed
                    } else {
                        InvocationStatus::Completed
                    }
                );
                release.send(()).unwrap();
                let terminal = executor.wait_task(task_id, Duration::from_secs(1)).unwrap();
                assert_eq!(
                    terminal.status,
                    if should_fail {
                        InvocationStatus::Failed
                    } else {
                        InvocationStatus::Completed
                    }
                );
                if should_fail {
                    assert_eq!(
                        terminal.failure,
                        Some(InvocationFailure::new(
                            "invocation_failed",
                            "daemon invocation failed",
                        ))
                    );
                    let persisted = serde_json::to_string(&store.get(task_id).unwrap()).unwrap();
                    assert!(!persisted.contains("SECRET"));
                    assert!(!persisted.contains("/private/runtime/error"));
                }
                assert_eq!(executions.load(Ordering::SeqCst), 1);
                wait_until_inactive(&executor);
            }
        }
    }

    #[test]
    fn actor_resource_capability_is_retained_through_terminal_reconciliation() {
        struct LeaseDrop(Arc<AtomicUsize>);
        impl Drop for LeaseDrop {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }

        let store = Arc::new(MemoryStore::default());
        let (fault, entered, release) = CommitFault::new(CommitFaultTiming::After);
        *store.update_fault.lock().unwrap() = Some(fault);
        let executor = Arc::new(InvocationExecutor::new(
            store,
            Arc::new(ManualClock::new(Instant::now())),
        ));
        let drops = Arc::new(AtomicUsize::new(0));
        let lease: Arc<dyn Send + Sync> = Arc::new(LeaseDrop(Arc::clone(&drops)));
        let outcome = executor
            .submit(
                prepared(
                    ExecutionClass::KnownLong(KnownLongReason::ExternalProcess),
                    Duration::ZERO,
                )
                .with_resource_lease(lease),
                |_| Ok(result("retained actor capability")),
            )
            .unwrap();
        let task_id = match outcome {
            InvocationOutcome::Task(snapshot) => snapshot.task_id,
            other => panic!("expected task: {other:?}"),
        };
        entered.recv().unwrap();
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        assert!(executor.has_active_invocations());
        release.send(()).unwrap();
        assert_eq!(
            executor
                .wait_task(task_id, Duration::from_secs(1))
                .unwrap()
                .status,
            InvocationStatus::Completed
        );
        wait_until_inactive(&executor);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn cancel_faults_keep_the_live_owner_until_cancellation_is_durable() {
        for timing in [CommitFaultTiming::Before, CommitFaultTiming::After] {
            let store = Arc::new(MemoryStore::default());
            let executor = Arc::new(InvocationExecutor::new(
                store.clone(),
                Arc::new(ManualClock::new(Instant::now())),
            ));
            let (started_send, started_wait) = mpsc::channel();
            let outcome = executor
                .submit(
                    prepared(
                        ExecutionClass::KnownLong(KnownLongReason::OccupiedWriteLease),
                        Duration::ZERO,
                    ),
                    move |cancellation| {
                        started_send.send(()).unwrap();
                        while !cancellation.is_cancelled() {
                            std::thread::yield_now();
                        }
                        Err(InvocationFailure::new("cancelled", "caller cancelled"))
                    },
                )
                .unwrap();
            let task_id = match outcome {
                InvocationOutcome::Task(snapshot) => snapshot.task_id,
                other => panic!("expected task: {other:?}"),
            };
            started_wait.recv().unwrap();
            let (fault, entered, release) = CommitFault::new(timing);
            *store.cancel_fault.lock().unwrap() = Some(fault);
            let cancel = {
                let executor = Arc::clone(&executor);
                std::thread::spawn(move || executor.cancel_task(task_id))
            };
            entered.recv().unwrap();
            assert!(executor.has_active_invocations());
            assert_eq!(
                store.get(task_id).unwrap().status,
                if timing == CommitFaultTiming::Before {
                    InvocationStatus::Working
                } else {
                    InvocationStatus::Cancelled
                }
            );
            release.send(()).unwrap();
            let cancelled = cancel.join().unwrap();
            if timing == CommitFaultTiming::Before {
                assert!(matches!(
                    cancelled,
                    Err(super::InvocationExecutorError::Store(
                        InvocationStoreError::Storage(_)
                    ))
                ));
                executor.cancel_task(task_id).unwrap();
            } else {
                assert_eq!(cancelled.unwrap().status, InvocationStatus::Cancelled);
            }
            assert_eq!(
                executor
                    .wait_task(task_id, Duration::from_secs(1))
                    .unwrap()
                    .status,
                InvocationStatus::Cancelled
            );
            wait_until_inactive(&executor);
        }
    }

    #[test]
    fn cancel_returns_the_exact_completed_or_failed_terminal_winner() {
        for should_fail in [false, true] {
            let store = Arc::new(MemoryStore::default());
            let executor = Arc::new(InvocationExecutor::new(
                store,
                Arc::new(ManualClock::new(Instant::now())),
            ));
            let executions = Arc::new(AtomicUsize::new(0));
            let run_count = Arc::clone(&executions);
            let outcome = executor
                .submit(
                    prepared(
                        ExecutionClass::KnownLong(KnownLongReason::ExternalProcess),
                        Duration::ZERO,
                    ),
                    move |_| {
                        run_count.fetch_add(1, Ordering::SeqCst);
                        if should_fail {
                            Err(InvocationFailure::new(
                                "runtime_failure",
                                "runtime failure is projected safely",
                            ))
                        } else {
                            Ok(result("completed before cancel"))
                        }
                    },
                )
                .unwrap();
            let task_id = match outcome {
                InvocationOutcome::Task(snapshot) => snapshot.task_id,
                other => panic!("expected task: {other:?}"),
            };
            let winner = executor.wait_task(task_id, Duration::from_secs(1)).unwrap();
            let cancel = executor
                .cancel_task(task_id)
                .expect("cancel observes the already committed terminal winner");

            assert_eq!(cancel, winner);
            assert_eq!(
                cancel.status,
                if should_fail {
                    InvocationStatus::Failed
                } else {
                    InvocationStatus::Completed
                }
            );
            assert_eq!(executions.load(Ordering::SeqCst), 1);
        }
    }

    #[test]
    fn cancel_invalid_transition_readback_returns_the_exact_terminal_winner() {
        for should_fail in [false, true] {
            let store = Arc::new(MemoryStore::default());
            let executor = Arc::new(InvocationExecutor::new(
                store.clone(),
                Arc::new(ManualClock::new(Instant::now())),
            ));
            let executions = Arc::new(AtomicUsize::new(0));
            let run_count = Arc::clone(&executions);
            let task_id = match executor
                .submit(
                    prepared(
                        ExecutionClass::KnownLong(KnownLongReason::ExternalProcess),
                        Duration::ZERO,
                    ),
                    move |_| {
                        run_count.fetch_add(1, Ordering::SeqCst);
                        if should_fail {
                            Err(InvocationFailure::new("failed", "safe failure"))
                        } else {
                            Ok(result("completed before invalid transition"))
                        }
                    },
                )
                .unwrap()
            {
                InvocationOutcome::Task(snapshot) => snapshot.task_id,
                other => panic!("expected task: {other:?}"),
            };
            let winner = executor.wait_task(task_id, Duration::from_secs(1)).unwrap();
            store
                .cancel_returns_invalid_transition
                .store(1, Ordering::SeqCst);

            assert_eq!(executor.cancel_task(task_id).unwrap(), winner);
            assert_eq!(executions.load(Ordering::SeqCst), 1);
        }

        let store = Arc::new(MemoryStore::default());
        let executor = Arc::new(InvocationExecutor::new(
            store.clone(),
            Arc::new(ManualClock::new(Instant::now())),
        ));
        let task_id = match executor
            .submit(
                prepared(
                    ExecutionClass::KnownLong(KnownLongReason::OccupiedWriteLease),
                    Duration::ZERO,
                ),
                |cancellation| {
                    while !cancellation.is_cancelled() {
                        std::thread::yield_now();
                    }
                    Err(InvocationFailure::new("cancelled", "safe cancellation"))
                },
            )
            .unwrap()
        {
            InvocationOutcome::Task(snapshot) => snapshot.task_id,
            other => panic!("expected task: {other:?}"),
        };
        let winner = executor.cancel_task(task_id).unwrap();
        assert_eq!(winner.status, InvocationStatus::Cancelled);
        store
            .cancel_returns_invalid_transition
            .store(1, Ordering::SeqCst);
        assert_eq!(executor.cancel_task(task_id).unwrap(), winner);
    }

    #[test]
    fn permanent_cancel_uncertainty_requires_restart_without_exposing_or_reexecuting() {
        struct LeaseDrop(Arc<AtomicUsize>);
        impl Drop for LeaseDrop {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }

        let store = Arc::new(MemoryStore::default());
        store.cancel_always_uncertain.store(1, Ordering::SeqCst);
        let executor = Arc::new(InvocationExecutor::new_with_reconciliation_budget_for_test(
            store.clone(),
            Arc::new(ManualClock::new(Instant::now())),
            Duration::from_secs(1),
        ));
        let executions = Arc::new(AtomicUsize::new(0));
        let run_count = Arc::clone(&executions);
        let lease_drops = Arc::new(AtomicUsize::new(0));
        let (started, started_wait) = mpsc::channel();
        let task_id = match executor
            .submit(
                prepared(
                    ExecutionClass::KnownLong(KnownLongReason::OccupiedWriteLease),
                    Duration::ZERO,
                )
                .with_resource_lease(Arc::new(LeaseDrop(Arc::clone(&lease_drops)))),
                move |cancellation| {
                    run_count.fetch_add(1, Ordering::SeqCst);
                    started.send(()).unwrap();
                    while !cancellation.is_cancelled() {
                        std::thread::yield_now();
                    }
                    Err(InvocationFailure::new("cancelled", "safe cancellation"))
                },
            )
            .unwrap()
        {
            InvocationOutcome::Task(snapshot) => snapshot.task_id,
            other => panic!("expected task: {other:?}"),
        };
        started_wait.recv().unwrap();

        assert!(matches!(
            executor.cancel_task(task_id),
            Err(super::InvocationExecutorError::RestartRequested)
        ));
        assert!(executor.restart_requested());
        assert!(!executor.has_active_invocations());
        assert_eq!(lease_drops.load(Ordering::SeqCst), 1);
        assert_eq!(executions.load(Ordering::SeqCst), 1);
        let persisted = store.get(task_id).unwrap();
        assert_eq!(persisted.status, InvocationStatus::Working);
        assert!(persisted.result.is_none());
        assert!(matches!(
            executor.submit(
                prepared(ExecutionClass::InlineCandidate, Duration::ZERO),
                |_| Ok(result("must not execute")),
            ),
            Err(super::InvocationExecutorError::RestartRequested)
        ));
        assert_eq!(executions.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn cancel_observes_complete_or_fail_that_committed_before_uncertain_return() {
        for should_fail in [false, true] {
            let store = Arc::new(MemoryStore::default());
            let (fault, entered, release) = CommitFault::new(CommitFaultTiming::After);
            *store.update_fault.lock().unwrap() = Some(fault);
            let executor = Arc::new(InvocationExecutor::new(
                store,
                Arc::new(ManualClock::new(Instant::now())),
            ));
            let executions = Arc::new(AtomicUsize::new(0));
            let run_count = Arc::clone(&executions);
            let outcome = executor
                .submit(
                    prepared(
                        ExecutionClass::KnownLong(KnownLongReason::ExternalProcess),
                        Duration::ZERO,
                    ),
                    move |_| {
                        run_count.fetch_add(1, Ordering::SeqCst);
                        if should_fail {
                            Err(InvocationFailure::new("failed", "safe failure"))
                        } else {
                            Ok(result("completed before uncertain return"))
                        }
                    },
                )
                .unwrap();
            let task_id = match outcome {
                InvocationOutcome::Task(snapshot) => snapshot.task_id,
                other => panic!("expected task: {other:?}"),
            };
            entered.recv().unwrap();

            let (winner_send, winner_wait) = mpsc::channel();
            let cancel_executor = Arc::clone(&executor);
            let cancel = std::thread::spawn(move || {
                winner_send
                    .send(cancel_executor.cancel_task(task_id))
                    .unwrap();
            });
            // The serial store actor must finish the already-committed terminal
            // command before the queued cancel readback can observe its winner.
            release.send(()).unwrap();
            let winner = winner_wait
                .recv_timeout(Duration::from_secs(10))
                .unwrap()
                .unwrap();
            assert_eq!(
                winner.status,
                if should_fail {
                    InvocationStatus::Failed
                } else {
                    InvocationStatus::Completed
                }
            );
            cancel.join().unwrap();
            let deadline = Instant::now() + Duration::from_secs(1);
            while executor.has_active_invocations() && Instant::now() < deadline {
                std::thread::yield_now();
            }
            assert!(!executor.has_active_invocations());
            assert_eq!(executions.load(Ordering::SeqCst), 1);
        }
    }

    #[test]
    fn terminal_publication_faults_reconcile_without_reexecution_or_false_idle() {
        create_working_faults_are_resolved_by_exact_identity_bound_readback();
        uncertain_create_transient_readback_is_reconciled_before_execution();
        normal_ok_create_mismatch_fails_stop_before_execution_and_disclosure();
        permanent_uncertain_create_fails_closed_and_stops_new_execution();
        completion_at_handoff_reconciles_create_without_exposing_staged_result();
        terminal_complete_and_fail_faults_reconcile_without_reexecution_or_early_idle();
        terminal_reconciliation_uses_bounded_exponential_backoff();
        permanent_terminal_failure_uses_bounded_policy_then_requires_restart();
        cancel_faults_keep_the_live_owner_until_cancellation_is_durable();
        cancel_returns_the_exact_completed_or_failed_terminal_winner();
        cancel_invalid_transition_readback_returns_the_exact_terminal_winner();
        cancel_observes_complete_or_fail_that_committed_before_uncertain_return();
        permanent_cancel_uncertainty_requires_restart_without_exposing_or_reexecuting();
        actor_resource_capability_is_retained_through_terminal_reconciliation();
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
                    reason: SafeFailureReason::Interrupted,
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
