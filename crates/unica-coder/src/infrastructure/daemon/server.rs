use super::identity::{CoreIdentity, DaemonStateDirectory};
use super::protocol::{
    parse_request, read_bounded_request_line, ClientRequest, DaemonErrorCode, DaemonTaskSnapshot,
    EndpointRecord, InvocationRequest, InvocationResponse, ServerResponse, DAEMON_PROTOCOL_VERSION,
};
use crate::application::invocation::{
    normalized_arguments_hash, InvocationExecutor, InvocationExecutorError,
    PreparedDaemonInvocation, RESPONSE_SERIALIZATION_MARGIN_MS,
};
use crate::application::invocation_store::{InvocationStore, InvocationStoreError};
use crate::application::operation_descriptors::ExecutionClass;
use crate::application::ports::{Clock, TokioClock};
use crate::application::tool_contracts::SurfaceRelease;
use crate::composition::open_daemon_invocation_store_from_directory;
use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::ProviderDeadline;
use crate::domain::invocation::{
    DomainResult, InvocationFailure, InvocationOutcome, SafeIdentityHash,
};
use crate::infrastructure::workspace::discover_workspace;
use crate::infrastructure::workspace_actor::{
    ProviderRootBinding, WorkspaceActor, WorkspaceActorRegistry, WorkspaceActorRegistryError,
    WorkspaceRevisionFence,
};
use std::collections::HashSet;
use std::io::{self, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const ACCEPT_POLL_INTERVAL: Duration = Duration::from_millis(20);
const CONNECTION_READ_TIMEOUT: Duration = Duration::from_millis(100);
const HANDSHAKE_READ_TIMEOUT: Duration = Duration::from_secs(2);
const OWNER_RESPONSE_WRITE_TIMEOUT: Duration = Duration::from_secs(10);
pub(crate) const MAX_HANDSHAKES: usize = 8;
pub(crate) const MAX_OWNER_SESSIONS: usize = 64;
const ACTOR_OPERATION_BUDGET: Duration = Duration::from_secs(7);

#[derive(Clone)]
pub(crate) struct ActorBoundInvocation {
    tool: crate::application::invocation_store::ToolIdentity,
    arguments: serde_json::Map<String, serde_json::Value>,
    response_budget: Duration,
    actor: Arc<WorkspaceActor>,
    provider_root: ProviderRootBinding,
    workspace_identity_hash: SafeIdentityHash,
}

impl ActorBoundInvocation {
    pub(crate) fn tool(&self) -> crate::application::invocation_store::ToolIdentity {
        self.tool
    }

    pub(crate) fn arguments(&self) -> &serde_json::Map<String, serde_json::Value> {
        &self.arguments
    }

    pub(crate) fn workspace_identity_hash(&self) -> &SafeIdentityHash {
        &self.workspace_identity_hash
    }

    fn begin_execution(
        self,
        cancellation: &CancellationToken,
    ) -> Result<ActorBoundExecution, String> {
        let revision = self.actor.capture_revision(
            &self.provider_root,
            ProviderDeadline::from_budget(ACTOR_OPERATION_BUDGET),
            cancellation,
        )?;
        Ok(ActorBoundExecution {
            invocation: self,
            revision,
        })
    }
}

pub(crate) struct ActorBoundExecution {
    invocation: ActorBoundInvocation,
    revision: WorkspaceRevisionFence,
}

impl ActorBoundExecution {
    // The hidden V13 profile installs real consumers in Tasks 10-21. Until the
    // Task 22 cutover, only injected canonical services exercise this narrow
    // capability surface.
    #[allow(dead_code)]
    pub(crate) fn tool(&self) -> crate::application::invocation_store::ToolIdentity {
        self.invocation.tool()
    }

    #[allow(dead_code)]
    pub(crate) fn arguments(&self) -> &serde_json::Map<String, serde_json::Value> {
        self.invocation.arguments()
    }

    #[allow(dead_code)]
    pub(crate) fn workspace_identity_hash(&self) -> &SafeIdentityHash {
        self.invocation.workspace_identity_hash()
    }

    #[allow(dead_code)]
    pub(crate) fn read_relative_file(
        &self,
        relative: &std::path::Path,
        max_bytes: usize,
    ) -> Result<Vec<u8>, String> {
        self.invocation.actor.read_relative_file(
            &self.invocation.provider_root,
            relative,
            max_bytes,
        )
    }

    fn publish<T>(self, staged: T, cancellation: &CancellationToken) -> Result<T, String> {
        self.invocation
            .actor
            .begin_publication(
                &self.revision,
                ProviderDeadline::from_budget(ACTOR_OPERATION_BUDGET),
                cancellation,
            )?
            .publish(
                staged,
                ProviderDeadline::from_budget(ACTOR_OPERATION_BUDGET),
                cancellation,
            )
    }
}

pub(crate) trait CanonicalInvocationService: Send + Sync {
    fn prepare(
        &self,
        invocation: &ActorBoundInvocation,
    ) -> Result<ExecutionClass, Box<DomainResult>>;

    fn execute(
        &self,
        invocation: &ActorBoundExecution,
        cancellation: CancellationToken,
    ) -> Result<DomainResult, InvocationFailure>;
}

struct DormantCanonicalV13Service;

impl CanonicalInvocationService for DormantCanonicalV13Service {
    fn prepare(
        &self,
        _invocation: &ActorBoundInvocation,
    ) -> Result<ExecutionClass, Box<DomainResult>> {
        Ok(ExecutionClass::InlineCandidate)
    }

    fn execute(
        &self,
        _invocation: &ActorBoundExecution,
        _cancellation: CancellationToken,
    ) -> Result<DomainResult, InvocationFailure> {
        Ok(failed_domain_result(
            "canonical v0.13 handler is not installed before the Task 22 cutover",
        ))
    }
}

fn bind_workspace_invocation(
    request: &InvocationRequest,
    actors: &WorkspaceActorRegistry,
) -> Result<ActorBoundInvocation, WorkspaceAdmissionError> {
    let context = discover_workspace(Some(std::path::PathBuf::from(request.workspace_hint())))
        .map_err(|_| WorkspaceAdmissionError::Invalid)?;
    let source_root = context.workspace_root.clone();
    let actor = actors
        .get_or_create(&context, [("main", &source_root)], "canonical-v0.13")
        .map_err(|error| match error {
            WorkspaceActorRegistryError::Capacity { .. } => WorkspaceAdmissionError::Capacity,
            WorkspaceActorRegistryError::InvalidIdentity(_) => WorkspaceAdmissionError::Invalid,
            WorkspaceActorRegistryError::Poisoned => WorkspaceAdmissionError::RegistryFailed,
        })?;
    let provider_root = actor
        .bind_provider_root("main", &source_root)
        .map_err(|_| WorkspaceAdmissionError::Invalid)?;
    let workspace_identity_hash = actor
        .safe_identity_hash()
        .map_err(|_| WorkspaceAdmissionError::Invalid)?;
    Ok(ActorBoundInvocation {
        tool: request.tool(),
        arguments: request.arguments().clone(),
        response_budget: Duration::from_millis(request.response_budget_ms()),
        actor,
        provider_root,
        workspace_identity_hash,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkspaceAdmissionError {
    Capacity,
    RegistryFailed,
    Invalid,
}

#[derive(Debug)]
enum DaemonInvocationError {
    WorkspaceCapacity,
    WorkspaceRegistryFailed,
    Executor(InvocationExecutorError),
}

impl From<InvocationExecutorError> for DaemonInvocationError {
    fn from(error: InvocationExecutorError) -> Self {
        Self::Executor(error)
    }
}

impl DaemonInvocationError {
    fn protocol_code(&self) -> DaemonErrorCode {
        match self {
            Self::WorkspaceCapacity => DaemonErrorCode::WorkspaceCapacity,
            Self::WorkspaceRegistryFailed => DaemonErrorCode::WorkspaceRegistryFailed,
            Self::Executor(InvocationExecutorError::Store(InvocationStoreError::NotFound)) => {
                DaemonErrorCode::TaskNotFound
            }
            Self::Executor(InvocationExecutorError::Store(InvocationStoreError::Expired)) => {
                DaemonErrorCode::TaskExpired
            }
            Self::Executor(InvocationExecutorError::Store(InvocationStoreError::Capacity {
                ..
            })) => DaemonErrorCode::TaskCapacity,
            Self::Executor(InvocationExecutorError::Store(
                InvocationStoreError::ResultTooLarge { .. },
            ))
            | Self::Executor(InvocationExecutorError::ResultTooLarge) => {
                DaemonErrorCode::ResultTooLarge
            }
            Self::Executor(InvocationExecutorError::Store(_)) => DaemonErrorCode::StoreFailed,
            Self::Executor(InvocationExecutorError::ExecutionFailed) => {
                DaemonErrorCode::InvocationFailed
            }
            Self::Executor(InvocationExecutorError::StatePoisoned) => {
                DaemonErrorCode::InvocationFailed
            }
            Self::Executor(InvocationExecutorError::RestartRequested) => {
                DaemonErrorCode::DurabilityUncertain
            }
        }
    }
}

#[cfg(test)]
pub(crate) fn workspace_capacity_protocol_code_for_test() -> DaemonErrorCode {
    DaemonInvocationError::WorkspaceCapacity.protocol_code()
}

fn failed_domain_result(summary: &str) -> DomainResult {
    let mut result = DomainResult::success(summary);
    result.ok = false;
    result
}

fn validate_hidden_v13_request(request: &InvocationRequest) -> Result<(), String> {
    let catalog = crate::application::v13::tool_catalog::catalog_for(SurfaceRelease::V13)
        .ok_or_else(|| "canonical v0.13 catalog is unavailable".to_string())?;
    let contract = catalog
        .tools
        .iter()
        .find(|contract| contract.name == request.tool().catalog_name())
        .ok_or_else(|| "canonical tool identity is not in the v0.13 catalog".to_string())?;
    let schema = contract
        .input_schema
        .as_object()
        .ok_or_else(|| "canonical tool schema is not an object".to_string())?;
    let properties = schema
        .get("properties")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| "canonical tool schema has no properties".to_string())?;
    if let Some(unknown) = request
        .arguments()
        .keys()
        .find(|name| !properties.contains_key(*name))
    {
        return Err(format!(
            "canonical invocation has unknown argument `{unknown}`"
        ));
    }
    for required in schema
        .get("required")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
    {
        if !request.arguments().contains_key(required) {
            return Err(format!(
                "canonical invocation requires argument `{required}`"
            ));
        }
    }
    for (name, value) in request.arguments() {
        validate_canonical_value(name, value, &properties[name])?;
    }
    Ok(())
}

fn validate_canonical_value(
    name: &str,
    value: &serde_json::Value,
    schema: &serde_json::Value,
) -> Result<(), String> {
    let expected = schema.get("type").and_then(serde_json::Value::as_str);
    let type_matches = match expected {
        Some("string") => value.is_string(),
        Some("boolean") => value.is_boolean(),
        Some("integer") => value.as_i64().is_some() || value.as_u64().is_some(),
        Some("object") => value.is_object(),
        Some("array") => value.is_array(),
        Some(_) | None => true,
    };
    if !type_matches {
        return Err(format!(
            "canonical invocation argument `{name}` has the wrong type"
        ));
    }
    if matches!(name, "at" | "scope" | "left" | "right")
        && value
            .as_str()
            .is_some_and(|text| text.trim().is_empty() || text.chars().any(char::is_control))
    {
        return Err(format!(
            "canonical invocation address argument `{name}` is invalid"
        ));
    }
    if let (Some(minimum), Some(number)) = (
        schema.get("minimum").and_then(serde_json::Value::as_u64),
        value.as_u64(),
    ) {
        if number < minimum {
            return Err(format!(
                "canonical invocation argument `{name}` is below its minimum"
            ));
        }
    }
    if let (Some(min_items), Some(items)) = (
        schema.get("minItems").and_then(serde_json::Value::as_u64),
        value.as_array(),
    ) {
        if items.len() < min_items as usize {
            return Err(format!(
                "canonical invocation argument `{name}` has too few items"
            ));
        }
    }
    if let (Some(items), Some(item_schema)) = (value.as_array(), schema.get("items")) {
        for item in items {
            validate_canonical_value(name, item, item_schema)?;
        }
    }
    if let (Some(object), Some(properties)) = (
        value.as_object(),
        schema
            .get("properties")
            .and_then(serde_json::Value::as_object),
    ) {
        if schema
            .get("additionalProperties")
            .is_some_and(|value| value == false)
        {
            if let Some(unknown) = object.keys().find(|field| !properties.contains_key(*field)) {
                return Err(format!(
                    "canonical invocation argument `{name}` has unknown field `{unknown}`"
                ));
            }
        }
        for required in schema
            .get("required")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(serde_json::Value::as_str)
        {
            if !object.contains_key(required) {
                return Err(format!(
                    "canonical invocation argument `{name}` requires field `{required}`"
                ));
            }
        }
        for (field, nested) in object {
            if let Some(field_schema) = properties.get(field) {
                validate_canonical_value(field, nested, field_schema)?;
            }
        }
    }
    Ok(())
}

pub(crate) struct DaemonInvocationRuntime {
    executor: Arc<InvocationExecutor>,
    service: Arc<dyn CanonicalInvocationService>,
    workspace_actors: WorkspaceActorRegistry,
}

impl DaemonInvocationRuntime {
    fn new(
        store: Arc<dyn InvocationStore>,
        service: Arc<dyn CanonicalInvocationService>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            executor: Arc::new(InvocationExecutor::new(store, clock)),
            service,
            workspace_actors: WorkspaceActorRegistry::default(),
        }
    }

    #[cfg(test)]
    fn new_with_reconciliation_budget_for_test(
        store: Arc<dyn InvocationStore>,
        service: Arc<dyn CanonicalInvocationService>,
        clock: Arc<dyn Clock>,
        budget: Duration,
    ) -> Self {
        Self {
            executor: Arc::new(InvocationExecutor::new_with_reconciliation_budget_for_test(
                store, clock, budget,
            )),
            service,
            workspace_actors: WorkspaceActorRegistry::default(),
        }
    }

    fn submit(
        &self,
        request: InvocationRequest,
    ) -> Result<InvocationResponse, DaemonInvocationError> {
        let actor_bound = match validate_hidden_v13_request(&request) {
            Ok(()) => match bind_workspace_invocation(&request, &self.workspace_actors) {
                Ok(bound) => Ok(bound),
                Err(WorkspaceAdmissionError::Capacity) => {
                    return Err(DaemonInvocationError::WorkspaceCapacity)
                }
                Err(WorkspaceAdmissionError::RegistryFailed) => {
                    return Err(DaemonInvocationError::WorkspaceRegistryFailed)
                }
                Err(WorkspaceAdmissionError::Invalid) => {
                    Err(failed_domain_result("workspace actor admission failed"))
                }
            },
            Err(summary) => Err(failed_domain_result(&summary)),
        };
        let prepared = match &actor_bound {
            Ok(invocation) => self.service.prepare(invocation).map_err(|result| *result),
            Err(result) => Err(result.clone()),
        }
        .map(|class| {
            let invocation = actor_bound
                .as_ref()
                .expect("successful preparation retains actor-bound invocation");
            PreparedDaemonInvocation::new(
                invocation.tool(),
                normalized_arguments_hash(invocation.arguments()),
                invocation.workspace_identity_hash().clone(),
                class,
                invocation.response_budget,
            )
            .with_resource_lease(Arc::new(invocation.clone()))
        });
        let service = Arc::clone(&self.service);
        let execute_invocation = actor_bound.ok();
        self.executor
            .submit_prepared(prepared, move |cancellation| {
                let invocation = execute_invocation.ok_or_else(|| {
                    InvocationFailure::new(
                        "workspace_admission_failed",
                        "workspace actor admission failed",
                    )
                })?;
                let execution = invocation.begin_execution(&cancellation).map_err(|_| {
                    InvocationFailure::new(
                        "workspace_changed",
                        "workspace actor capability changed before execution",
                    )
                })?;
                let outcome = service.execute(&execution, cancellation.clone());
                execution.publish(outcome, &cancellation).map_err(|_| {
                    InvocationFailure::new(
                        "workspace_changed",
                        "workspace actor capability changed before publication",
                    )
                })?
            })
            .map(|outcome| match outcome {
                InvocationOutcome::Direct(result) => InvocationResponse::Direct(result),
                InvocationOutcome::Task(task) => {
                    InvocationResponse::Task(DaemonTaskSnapshot::from_domain(task))
                }
            })
            .map_err(DaemonInvocationError::from)
    }

    fn get(
        &self,
        task_id: crate::domain::invocation::TaskId,
    ) -> Result<DaemonTaskSnapshot, DaemonInvocationError> {
        self.executor
            .get_task(task_id)
            .map(DaemonTaskSnapshot::from_domain)
            .map_err(DaemonInvocationError::from)
    }

    fn wait(
        &self,
        task_id: crate::domain::invocation::TaskId,
        wait_ms: u64,
    ) -> Result<DaemonTaskSnapshot, DaemonInvocationError> {
        self.executor
            .wait_task(task_id, Duration::from_millis(wait_ms))
            .map(DaemonTaskSnapshot::from_domain)
            .map_err(DaemonInvocationError::from)
    }

    fn cancel(
        &self,
        task_id: crate::domain::invocation::TaskId,
    ) -> Result<DaemonTaskSnapshot, DaemonInvocationError> {
        self.executor
            .cancel_task(task_id)
            .map(DaemonTaskSnapshot::from_domain)
            .map_err(DaemonInvocationError::from)
    }

    fn has_active_invocations(&self) -> bool {
        self.executor.has_active_invocations()
    }

    fn restart_requested(&self) -> bool {
        self.executor.restart_requested()
    }
}

#[derive(Clone)]
pub(crate) struct DaemonServerConfig {
    pub(crate) state_root: std::path::PathBuf,
    pub(crate) core_identity: CoreIdentity,
    pub(crate) idle_grace: Duration,
    invocation_service: Arc<dyn CanonicalInvocationService>,
    #[cfg(test)]
    invocation_store: Option<Arc<dyn InvocationStore>>,
    #[cfg(test)]
    reconciliation_budget: Option<Duration>,
    #[cfg(test)]
    handshake_pause: Option<Arc<HandshakePause>>,
}

impl DaemonServerConfig {
    pub(crate) fn new(
        state_root: std::path::PathBuf,
        core_identity: CoreIdentity,
        idle_grace: Duration,
    ) -> Self {
        Self {
            state_root,
            core_identity,
            idle_grace,
            invocation_service: Arc::new(DormantCanonicalV13Service),
            #[cfg(test)]
            invocation_store: None,
            #[cfg(test)]
            reconciliation_budget: None,
            #[cfg(test)]
            handshake_pause: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_invocation_service(
        mut self,
        service: Arc<dyn CanonicalInvocationService>,
    ) -> Self {
        self.invocation_service = service;
        self
    }

    #[cfg(test)]
    pub(crate) fn with_invocation_store_for_test(
        mut self,
        store: Arc<dyn InvocationStore>,
    ) -> Self {
        self.invocation_store = Some(store);
        self
    }

    #[cfg(test)]
    pub(crate) fn with_reconciliation_budget_for_test(mut self, budget: Duration) -> Self {
        self.reconciliation_budget = Some(budget);
        self
    }

    #[cfg(test)]
    pub(crate) fn with_handshake_pause(mut self, pause: &HandshakePauseGuard) -> Self {
        self.handshake_pause = Some(Arc::clone(&pause.pause));
        self
    }
}

pub(crate) fn run_daemon(config: DaemonServerConfig) -> Result<(), String> {
    if config.idle_grace.is_zero() {
        return Err("daemon idle grace must be positive".to_string());
    }
    let state = DaemonStateDirectory::open(&config.state_root, &config.core_identity)?;
    if let Some(existing) = state.read_endpoint_record()? {
        if existing.core_identity() != &config.core_identity {
            return Err("daemon endpoint record belongs to a foreign core identity".to_string());
        }
    }

    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .map_err(|error| daemon_io_error("bind daemon loopback endpoint", error))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| daemon_io_error("configure daemon listener", error))?;
    let port = listener
        .local_addr()
        .map_err(|error| daemon_io_error("inspect daemon listener", error))?
        .port();

    #[cfg(test)]
    let invocation_store = if let Some(store) = config.invocation_store.clone() {
        store
    } else {
        let task_store_directory = state.create_private_subdirectory("tasks")?;
        let opened_store = open_daemon_invocation_store_from_directory(task_store_directory)?;
        // Recovery belongs to this daemon before routing starts. Keeping the report beside the
        // sole-writer store prevents the stdio frontend from consuming it early.
        let _recovery_classifications = opened_store.recovery.classifications.len();
        opened_store.store
    };
    #[cfg(not(test))]
    let invocation_store = {
        let task_store_directory = state.create_private_subdirectory("tasks")?;
        let opened_store = open_daemon_invocation_store_from_directory(task_store_directory)?;
        let _recovery_classifications = opened_store.recovery.classifications.len();
        opened_store.store
    };
    #[cfg(test)]
    let invocation_runtime = Arc::new(match config.reconciliation_budget {
        Some(budget) => DaemonInvocationRuntime::new_with_reconciliation_budget_for_test(
            invocation_store,
            Arc::clone(&config.invocation_service),
            Arc::new(TokioClock),
            budget,
        ),
        None => DaemonInvocationRuntime::new(
            invocation_store,
            Arc::clone(&config.invocation_service),
            Arc::new(TokioClock),
        ),
    });
    #[cfg(not(test))]
    let invocation_runtime = Arc::new(DaemonInvocationRuntime::new(
        invocation_store,
        Arc::clone(&config.invocation_service),
        Arc::new(TokioClock),
    ));

    let record = EndpointRecord::new(config.core_identity.clone(), port);
    let published = state.publish_endpoint_record(&record)?;
    let active_leases = Arc::new(LeaseRegistry::default());
    let admitted_connections = Arc::new(AtomicUsize::new(0));
    let shutting_down = Arc::new(AtomicBool::new(false));
    let mut handlers = Vec::new();
    let mut idle_since = Instant::now();
    let mut restart_requested = false;

    loop {
        match listener.accept() {
            Ok((stream, address)) if address.ip().is_loopback() => {
                match ConnectionSlot::acquire(Arc::clone(&admitted_connections)) {
                    Some(slot) => handlers.push(spawn_connection_handler(
                        stream,
                        record.clone(),
                        Arc::clone(&active_leases),
                        Arc::clone(&shutting_down),
                        Arc::clone(&invocation_runtime),
                        slot,
                        #[cfg(test)]
                        config.handshake_pause.clone(),
                    )),
                    None => reject_overloaded_connection(stream),
                }
            }
            Ok((_stream, _)) => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(error) => {
                shutting_down.store(true, Ordering::Release);
                join_handlers(handlers);
                let _ = state.remove_endpoint_if_owned(&published);
                return Err(daemon_io_error("accept daemon connection", error));
            }
        }

        handlers = reap_finished_handlers(handlers);
        if invocation_runtime.restart_requested() {
            restart_requested = true;
            break;
        }
        if active_leases.is_empty()?
            && admitted_connections.load(Ordering::Acquire) == 0
            && !invocation_runtime.has_active_invocations()
        {
            if idle_since.elapsed() >= config.idle_grace {
                break;
            }
        } else {
            idle_since = Instant::now();
        }
        thread::sleep(ACCEPT_POLL_INTERVAL);
    }

    shutting_down.store(true, Ordering::Release);
    drop(listener);
    if restart_requested {
        // Process death, not an in-process map or thread join, is the authority
        // that releases a stalled store syscall or non-cooperative execution.
        // Keep the PID-bound endpoint for stale-owner cleanup by the successor.
        return Ok(());
    }
    join_handlers(handlers);
    state.remove_endpoint_if_owned(&published)?;
    Ok(())
}

fn spawn_connection_handler(
    stream: TcpStream,
    record: EndpointRecord,
    active_leases: Arc<LeaseRegistry>,
    shutting_down: Arc<AtomicBool>,
    invocation_runtime: Arc<DaemonInvocationRuntime>,
    slot: ConnectionSlot,
    #[cfg(test)] handshake_pause: Option<Arc<HandshakePause>>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        #[cfg(test)]
        pause_before_handshake_if_configured(handshake_pause);
        let _ = handle_connection(
            stream,
            &record,
            &active_leases,
            &shutting_down,
            &invocation_runtime,
            slot,
        );
    })
}

fn handle_connection(
    mut stream: TcpStream,
    record: &EndpointRecord,
    active_leases: &Arc<LeaseRegistry>,
    shutting_down: &AtomicBool,
    invocation_runtime: &DaemonInvocationRuntime,
    handshake_slot: ConnectionSlot,
) -> Result<(), String> {
    stream
        .set_read_timeout(Some(HANDSHAKE_READ_TIMEOUT))
        .map_err(|error| daemon_io_error("configure daemon handshake timeout", error))?;
    stream
        .set_write_timeout(Some(HANDSHAKE_READ_TIMEOUT))
        .map_err(|error| daemon_io_error("configure daemon response timeout", error))?;
    let reader_stream = stream
        .try_clone()
        .map_err(|error| daemon_io_error("clone daemon client stream", error))?;
    let mut reader = BufReader::new(reader_stream);
    let request = match read_bounded_request_line(&mut reader).and_then(|bytes| {
        parse_request(&bytes).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }) {
        Ok(request) => request,
        Err(_) => {
            write_response(
                &mut stream,
                &ServerResponse::error(DaemonErrorCode::InvalidRequest),
            )?;
            return Ok(());
        }
    };

    let ClientRequest::Hello {
        protocol_version,
        token,
        core_identity,
        owner_lease,
    } = request
    else {
        write_response(
            &mut stream,
            &ServerResponse::error(DaemonErrorCode::HandshakeRequired),
        )?;
        return Ok(());
    };
    if protocol_version != DAEMON_PROTOCOL_VERSION {
        write_response(
            &mut stream,
            &ServerResponse::error(DaemonErrorCode::ProtocolMismatch),
        )?;
        return Ok(());
    }
    if core_identity != *record.core_identity() {
        write_response(
            &mut stream,
            &ServerResponse::error(DaemonErrorCode::CoreMismatch),
        )?;
        return Ok(());
    }
    if !tokens_equal(&token, record.token()) {
        write_response(
            &mut stream,
            &ServerResponse::error(DaemonErrorCode::Unauthorized),
        )?;
        return Ok(());
    }

    let _owner = match active_leases.acquire(owner_lease)? {
        LeaseAdmission::Acquired(owner) => owner,
        LeaseAdmission::Duplicate => {
            write_response(
                &mut stream,
                &ServerResponse::error(DaemonErrorCode::DuplicateLease),
            )?;
            return Ok(());
        }
        LeaseAdmission::Capacity => {
            write_response(
                &mut stream,
                &ServerResponse::error(DaemonErrorCode::OwnerCapacity),
            )?;
            return Ok(());
        }
    };
    // The owner lease becomes the lifecycle fence before the pre-authentication admission
    // permit is released, so idle shutdown observes at least one of them throughout handoff.
    drop(handshake_slot);
    write_response(&mut stream, &ServerResponse::ready(record))?;
    stream
        .set_write_timeout(Some(OWNER_RESPONSE_WRITE_TIMEOUT))
        .map_err(|error| daemon_io_error("configure daemon owner response timeout", error))?;
    stream
        .set_read_timeout(Some(CONNECTION_READ_TIMEOUT))
        .map_err(|error| daemon_io_error("configure daemon owner timeout", error))?;
    while !shutting_down.load(Ordering::Acquire) {
        let request = match read_bounded_request_line(&mut reader) {
            Ok(bytes) => match parse_request(&bytes) {
                Ok(request) => request,
                Err(_) => {
                    write_response(
                        &mut stream,
                        &ServerResponse::error(DaemonErrorCode::InvalidRequest),
                    )?;
                    break;
                }
            },
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                continue
            }
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(_) => break,
        };
        match request {
            ClientRequest::Ping {} => write_response_before(
                &mut stream,
                &ServerResponse::Pong,
                session_response_deadline(Duration::ZERO),
            )?,
            ClientRequest::Release {} => {
                write_response_before(
                    &mut stream,
                    &ServerResponse::Released,
                    session_response_deadline(Duration::ZERO),
                )?;
                break;
            }
            ClientRequest::Hello { .. } => {
                write_response(
                    &mut stream,
                    &ServerResponse::error(DaemonErrorCode::InvalidRequest),
                )?;
                break;
            }
            ClientRequest::SubmitInvocation { invocation } => {
                let deadline = session_response_deadline(Duration::from_millis(
                    invocation.response_budget_ms(),
                ));
                match invocation_runtime.submit(invocation) {
                    Ok(outcome) => write_response_before(
                        &mut stream,
                        &ServerResponse::invocation(outcome),
                        deadline,
                    )?,
                    Err(error) => write_response_before(
                        &mut stream,
                        &ServerResponse::error(error.protocol_code()),
                        deadline,
                    )?,
                }
            }
            ClientRequest::GetTask { task_id } => {
                let deadline = session_response_deadline(Duration::ZERO);
                write_task_response_before(&mut stream, invocation_runtime.get(task_id), deadline)?
            }
            ClientRequest::WaitTask { task_id, wait_ms } => {
                let deadline = session_response_deadline(Duration::from_millis(wait_ms));
                write_task_response_before(
                    &mut stream,
                    invocation_runtime.wait(task_id, wait_ms),
                    deadline,
                )?
            }
            ClientRequest::CancelTask { task_id } => {
                let deadline = session_response_deadline(Duration::ZERO);
                write_task_response_before(
                    &mut stream,
                    invocation_runtime.cancel(task_id),
                    deadline,
                )?
            }
        }
    }
    Ok(())
}

fn write_task_response_before(
    stream: &mut TcpStream,
    result: Result<DaemonTaskSnapshot, DaemonInvocationError>,
    deadline: Instant,
) -> Result<(), String> {
    match result {
        Ok(snapshot) => write_response_before(stream, &ServerResponse::task(snapshot), deadline),
        Err(error) => write_response_before(
            stream,
            &ServerResponse::error(error.protocol_code()),
            deadline,
        ),
    }
}

fn session_response_deadline(operation_budget: Duration) -> Instant {
    Instant::now() + operation_budget + Duration::from_millis(RESPONSE_SERIALIZATION_MARGIN_MS)
}

fn write_response(stream: &mut TcpStream, response: &ServerResponse) -> Result<(), String> {
    write_response_before(
        stream,
        response,
        Instant::now() + OWNER_RESPONSE_WRITE_TIMEOUT,
    )
}

fn write_response_before(
    stream: &mut TcpStream,
    response: &ServerResponse,
    session_deadline: Instant,
) -> Result<(), String> {
    let deadline = session_deadline.min(Instant::now() + OWNER_RESPONSE_WRITE_TIMEOUT);
    if Instant::now() >= deadline {
        return Err("write daemon response: session response deadline elapsed".to_string());
    }
    let mut bytes = match serialize_response_bounded(response) {
        Ok(bytes) => bytes,
        Err(ResponseSerializationError::TooLarge) => {
            serialize_response_bounded(&ServerResponse::error(DaemonErrorCode::ResultTooLarge))
                .map_err(|_| "bounded daemon error response could not be serialized".to_string())?
        }
        Err(ResponseSerializationError::Invalid) => {
            return Err("daemon response could not be serialized".to_string())
        }
    };
    if Instant::now() >= deadline {
        return Err("write daemon response: session response deadline elapsed".to_string());
    }
    bytes.push(b'\n');
    write_bytes_before(
        stream,
        &bytes,
        deadline,
        Instant::now,
        |stream, remaining| stream.set_write_timeout(Some(remaining)),
    )
}

pub(super) fn write_bytes_before<W, N, C>(
    writer: &mut W,
    bytes: &[u8],
    deadline: Instant,
    mut now: N,
    mut configure_timeout: C,
) -> Result<(), String>
where
    W: Write,
    N: FnMut() -> Instant,
    C: FnMut(&mut W, Duration) -> io::Result<()>,
{
    let mut written = 0;
    while written < bytes.len() {
        let remaining = deadline.saturating_duration_since(now());
        if remaining.is_zero() {
            return Err("write daemon response: bounded response deadline elapsed".to_string());
        }
        configure_timeout(writer, remaining)
            .map_err(|error| daemon_io_error("configure daemon response timeout", error))?;
        match writer.write(&bytes[written..]) {
            Ok(0) => {
                return Err("write daemon response: connection closed before response".to_string())
            }
            Ok(count) => written += count,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                std::thread::sleep(Duration::from_millis(1).min(remaining));
            }
            Err(error) => return Err(daemon_io_error("write daemon response", error)),
        }
    }
    writer
        .flush()
        .map_err(|error| daemon_io_error("flush daemon response", error))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResponseSerializationError {
    TooLarge,
    Invalid,
}

struct BoundedResponseWriter {
    bytes: Vec<u8>,
    max_bytes: usize,
    too_large: bool,
}

impl Write for BoundedResponseWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let Some(next) = self.bytes.len().checked_add(buffer.len()) else {
            self.too_large = true;
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "response too large",
            ));
        };
        if next > self.max_bytes {
            self.too_large = true;
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "response too large",
            ));
        }
        self.bytes.extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn serialize_response_bounded(
    response: &ServerResponse,
) -> Result<Vec<u8>, ResponseSerializationError> {
    let mut writer = BoundedResponseWriter {
        bytes: Vec::new(),
        max_bytes: super::protocol::MAX_DAEMON_RESPONSE_LINE_BYTES.saturating_sub(1),
        too_large: false,
    };
    let serialized = serde_json::to_writer(&mut writer, response);
    if writer.too_large {
        return Err(ResponseSerializationError::TooLarge);
    }
    if serialized.is_err() {
        return Err(ResponseSerializationError::Invalid);
    }
    Ok(writer.bytes)
}

#[cfg(test)]
pub(crate) mod actor_capacity_tests {
    use super::*;
    use crate::application::invocation_store::{
        NewInvocationRecord, SafeStatusMessage, StoredInvocationRecord, TaskTransition,
        ToolIdentity,
    };
    use crate::application::operation_descriptors::KnownLongReason;
    use crate::infrastructure::task_store::{FileInvocationStore, SystemEpochMillisClock};
    use std::sync::{mpsc, Barrier};

    struct BlockingService {
        entered: mpsc::Sender<()>,
    }

    struct RejectingAfterActorAdmissionService;

    struct NonCooperativeActorService {
        entered: mpsc::Sender<()>,
        release: Mutex<mpsc::Receiver<()>>,
    }

    struct UncertainCancelStore {
        inner: FileInvocationStore,
    }

    impl InvocationStore for UncertainCancelStore {
        fn create(
            &self,
            record: NewInvocationRecord,
        ) -> Result<StoredInvocationRecord, InvocationStoreError> {
            self.inner.create(record)
        }

        fn create_working(
            &self,
            record: NewInvocationRecord,
        ) -> Result<StoredInvocationRecord, InvocationStoreError> {
            self.inner.create_working(record)
        }

        fn get(
            &self,
            task_id: crate::domain::invocation::TaskId,
        ) -> Result<StoredInvocationRecord, InvocationStoreError> {
            self.inner.get(task_id)
        }

        fn update(
            &self,
            task_id: crate::domain::invocation::TaskId,
            transition: TaskTransition,
        ) -> Result<StoredInvocationRecord, InvocationStoreError> {
            self.inner.update(task_id, transition)
        }

        fn cancel(
            &self,
            task_id: crate::domain::invocation::TaskId,
            _status_message: SafeStatusMessage,
        ) -> Result<StoredInvocationRecord, InvocationStoreError> {
            Err(InvocationStoreError::CommitUncertain {
                task_id,
                operation: crate::application::invocation_store::CommitOperation::Cancel,
            })
        }
    }

    impl CanonicalInvocationService for BlockingService {
        fn prepare(
            &self,
            _invocation: &ActorBoundInvocation,
        ) -> Result<ExecutionClass, Box<DomainResult>> {
            Ok(ExecutionClass::KnownLong(KnownLongReason::ExternalProcess))
        }

        fn execute(
            &self,
            _invocation: &ActorBoundExecution,
            cancellation: CancellationToken,
        ) -> Result<DomainResult, InvocationFailure> {
            self.entered.send(()).unwrap();
            while !cancellation.is_cancelled() {
                std::thread::yield_now();
            }
            Err(InvocationFailure::new("cancelled", "test cancellation"))
        }
    }

    impl CanonicalInvocationService for RejectingAfterActorAdmissionService {
        fn prepare(
            &self,
            invocation: &ActorBoundInvocation,
        ) -> Result<ExecutionClass, Box<DomainResult>> {
            assert_eq!(invocation.tool(), ToolIdentity::Run);
            Err(Box::new(failed_domain_result(
                "test rejection after actor admission",
            )))
        }

        fn execute(
            &self,
            _invocation: &ActorBoundExecution,
            _cancellation: CancellationToken,
        ) -> Result<DomainResult, InvocationFailure> {
            panic!("rejected preparation must not reach execution")
        }
    }

    impl CanonicalInvocationService for NonCooperativeActorService {
        fn prepare(
            &self,
            _invocation: &ActorBoundInvocation,
        ) -> Result<ExecutionClass, Box<DomainResult>> {
            Ok(ExecutionClass::KnownLong(KnownLongReason::ExternalProcess))
        }

        fn execute(
            &self,
            _invocation: &ActorBoundExecution,
            _cancellation: CancellationToken,
        ) -> Result<DomainResult, InvocationFailure> {
            self.entered.send(()).unwrap();
            self.release.lock().unwrap().recv().unwrap();
            Ok(DomainResult::success(
                "non-cooperative staged actor result must stay hidden",
            ))
        }
    }

    fn task_id(response: InvocationResponse) -> crate::domain::invocation::TaskId {
        match response {
            InvocationResponse::Task(snapshot) => snapshot.task_id,
            other => panic!("expected durable task: {other:?}"),
        }
    }

    fn live_actor_capacity_reuses_alias_and_rejects_only_a_distinct_third_root() {
        let task_root = tempfile::tempdir().unwrap();
        let workspace_parent = tempfile::tempdir().unwrap();
        let roots = (0..3)
            .map(|index| {
                let root = workspace_parent.path().join(format!("workspace-{index}"));
                std::fs::create_dir(&root).unwrap();
                std::fs::canonicalize(root).unwrap()
            })
            .collect::<Vec<_>>();
        let (store, _) =
            FileInvocationStore::open(task_root.path(), Arc::new(SystemEpochMillisClock)).unwrap();
        let (entered, _entered_wait) = mpsc::channel();
        let runtime = DaemonInvocationRuntime {
            executor: Arc::new(InvocationExecutor::new(
                Arc::new(store),
                Arc::new(TokioClock),
            )),
            service: Arc::new(BlockingService { entered }),
            workspace_actors: WorkspaceActorRegistry::with_capacity_for_test(2),
        };
        let request = |root: &std::path::Path| {
            InvocationRequest::new(
                ToolIdentity::Run,
                serde_json::json!({}),
                root.to_string_lossy(),
                0,
            )
            .unwrap()
        };

        let first = task_id(runtime.submit(request(&roots[0])).unwrap());
        let second = task_id(runtime.submit(request(&roots[1])).unwrap());
        let alias = task_id(runtime.submit(request(&roots[0].join("."))).unwrap());
        // A returned task snapshot is the durable admission point. Its actor lease is already
        // retained before the background worker is scheduled, so capacity evidence must not
        // depend on runner scheduling.
        let rejected = runtime.submit(request(&roots[2])).unwrap_err();
        assert_eq!(rejected.protocol_code(), DaemonErrorCode::WorkspaceCapacity);
        assert_eq!(runtime.workspace_actors.entry_len_for_test().unwrap(), 2);

        for task_id in [first, second, alias] {
            assert_eq!(
                runtime.cancel(task_id).unwrap().status,
                crate::domain::invocation::InvocationStatus::Cancelled
            );
        }
    }

    fn concurrent_same_identity_admission_creates_one_actor() {
        let workspace = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(workspace.path()).unwrap();
        let context = discover_workspace(Some(root.clone())).unwrap();
        let registry = Arc::new(WorkspaceActorRegistry::default());
        let barrier = Arc::new(Barrier::new(9));
        let mut admissions = Vec::new();
        for _ in 0..8 {
            let registry = Arc::clone(&registry);
            let barrier = Arc::clone(&barrier);
            let context = context.clone();
            let root = root.clone();
            admissions.push(std::thread::spawn(move || {
                barrier.wait();
                registry
                    .get_or_create(&context, [("main", &root)], "canonical-v0.13")
                    .unwrap()
            }));
        }
        barrier.wait();
        let actors = admissions
            .into_iter()
            .map(|admission| admission.join().unwrap())
            .collect::<Vec<_>>();
        for actor in actors.iter().skip(1) {
            assert!(Arc::ptr_eq(&actors[0], actor));
        }
        assert_eq!(registry.entry_len_for_test().unwrap(), 1);
    }

    fn poisoned_registry_is_a_closed_internal_error_and_admits_nothing() {
        let task_root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let (store, _) =
            FileInvocationStore::open(task_root.path(), Arc::new(SystemEpochMillisClock)).unwrap();
        let (entered, _entered_wait) = mpsc::channel();
        let runtime = DaemonInvocationRuntime::new(
            Arc::new(store),
            Arc::new(BlockingService { entered }),
            Arc::new(TokioClock),
        );
        runtime.workspace_actors.poison_for_test();

        let error = runtime
            .submit(
                InvocationRequest::new(
                    ToolIdentity::Run,
                    serde_json::json!({}),
                    std::fs::canonicalize(workspace.path())
                        .unwrap()
                        .to_string_lossy(),
                    0,
                )
                .unwrap(),
            )
            .unwrap_err();
        assert_eq!(
            error.protocol_code(),
            DaemonErrorCode::WorkspaceRegistryFailed
        );
    }

    fn sequential_direct_admissions_release_actor_capabilities_and_prune_dead_entries() {
        let task_root = tempfile::tempdir().unwrap();
        let workspaces = tempfile::tempdir().unwrap();
        let (store, _) =
            FileInvocationStore::open(task_root.path(), Arc::new(SystemEpochMillisClock)).unwrap();
        let runtime = DaemonInvocationRuntime::new(
            Arc::new(store),
            Arc::new(RejectingAfterActorAdmissionService),
            Arc::new(TokioClock),
        );

        for index in 0..80 {
            let workspace = workspaces.path().join(format!("workspace-{index}"));
            std::fs::create_dir(&workspace).unwrap();
            let response = runtime
                .submit(
                    InvocationRequest::new(
                        ToolIdentity::Run,
                        serde_json::json!({}),
                        std::fs::canonicalize(&workspace).unwrap().to_string_lossy(),
                        0,
                    )
                    .unwrap(),
                )
                .unwrap();
            assert!(matches!(
                response,
                InvocationResponse::Direct(result)
                    if !result.ok && result.summary == "test rejection after actor admission"
            ));
            assert!(runtime.workspace_actors.entry_len_for_test().unwrap() <= 1);
        }
        assert_eq!(runtime.workspace_actors.entry_len_for_test().unwrap(), 1);
    }

    #[test]
    pub(crate) fn restart_request_does_not_claim_noncooperative_actor_released_in_process() {
        let task_root = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let (store, _) =
            FileInvocationStore::open(task_root.path(), Arc::new(SystemEpochMillisClock)).unwrap();
        let (entered, entered_wait) = mpsc::channel();
        let (release, release_wait) = mpsc::channel();
        let runtime = DaemonInvocationRuntime::new_with_reconciliation_budget_for_test(
            Arc::new(UncertainCancelStore { inner: store }),
            Arc::new(NonCooperativeActorService {
                entered,
                release: Mutex::new(release_wait),
            }),
            Arc::new(TokioClock),
            Duration::from_millis(100),
        );
        let request = InvocationRequest::new(
            ToolIdentity::Run,
            serde_json::json!({}),
            std::fs::canonicalize(workspace.path())
                .unwrap()
                .to_string_lossy(),
            0,
        )
        .unwrap();
        let task_id = task_id(runtime.submit(request).unwrap());
        entered_wait.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(runtime.workspace_actors.live_len_for_test().unwrap(), 1);

        assert!(matches!(
            runtime.cancel(task_id),
            Err(DaemonInvocationError::Executor(
                InvocationExecutorError::RestartRequested
            ))
        ));
        assert!(runtime.restart_requested());
        assert_eq!(
            runtime.workspace_actors.live_len_for_test().unwrap(),
            1,
            "the executing ActorBoundExecution still owns the actor until process death"
        );

        release.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while runtime.workspace_actors.live_len_for_test().unwrap() != 0
            && Instant::now() < deadline
        {
            std::thread::yield_now();
        }
        assert_eq!(runtime.workspace_actors.live_len_for_test().unwrap(), 0);
    }

    #[test]
    fn daemon_workspace_actor_admission_is_concurrent_bounded_and_fail_closed() {
        live_actor_capacity_reuses_alias_and_rejects_only_a_distinct_third_root();
        concurrent_same_identity_admission_creates_one_actor();
        poisoned_registry_is_a_closed_internal_error_and_admits_nothing();
        sequential_direct_admissions_release_actor_capabilities_and_prune_dead_entries();
    }

    #[test]
    fn typed_executor_errors_map_to_closed_protocol_codes_without_text_matching() {
        let storage = DaemonInvocationError::Executor(InvocationExecutorError::Store(
            InvocationStoreError::Storage(
                "SECRET runtime prose /private/path must not classify".into(),
            ),
        ));
        assert_eq!(storage.protocol_code(), DaemonErrorCode::StoreFailed);
        assert_eq!(
            DaemonInvocationError::Executor(InvocationExecutorError::Store(
                InvocationStoreError::NotFound,
            ))
            .protocol_code(),
            DaemonErrorCode::TaskNotFound
        );
        assert_eq!(
            DaemonInvocationError::Executor(InvocationExecutorError::Store(
                InvocationStoreError::Capacity { max_records: 4_096 },
            ))
            .protocol_code(),
            DaemonErrorCode::TaskCapacity
        );
        assert_eq!(
            DaemonInvocationError::Executor(InvocationExecutorError::Store(
                InvocationStoreError::Expired,
            ))
            .protocol_code(),
            DaemonErrorCode::TaskExpired
        );
        assert_eq!(
            DaemonInvocationError::Executor(InvocationExecutorError::ExecutionFailed)
                .protocol_code(),
            DaemonErrorCode::InvocationFailed
        );
        assert_eq!(
            DaemonInvocationError::Executor(InvocationExecutorError::RestartRequested)
                .protocol_code(),
            DaemonErrorCode::DurabilityUncertain
        );
        assert_eq!(
            DaemonInvocationError::WorkspaceRegistryFailed.protocol_code(),
            DaemonErrorCode::WorkspaceRegistryFailed
        );
    }
}

fn tokens_equal(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.bytes()
        .zip(right.bytes())
        .fold(0u8, |difference, (left, right)| difference | (left ^ right))
        == 0
}

#[derive(Default)]
struct LeaseRegistry {
    leases: Mutex<HashSet<String>>,
}

impl LeaseRegistry {
    fn acquire(self: &Arc<Self>, lease: String) -> Result<LeaseAdmission, String> {
        let mut leases = self
            .leases
            .lock()
            .map_err(|_| "daemon owner lease registry is poisoned".to_string())?;
        if leases.contains(&lease) {
            return Ok(LeaseAdmission::Duplicate);
        }
        if leases.len() >= MAX_OWNER_SESSIONS {
            return Ok(LeaseAdmission::Capacity);
        }
        leases.insert(lease.clone());
        drop(leases);
        Ok(LeaseAdmission::Acquired(LeaseGuard {
            registry: Arc::clone(self),
            lease,
        }))
    }

    fn is_empty(&self) -> Result<bool, String> {
        self.leases
            .lock()
            .map(|leases| leases.is_empty())
            .map_err(|_| "daemon owner lease registry is poisoned".to_string())
    }
}

enum LeaseAdmission {
    Acquired(LeaseGuard),
    Duplicate,
    Capacity,
}

struct LeaseGuard {
    registry: Arc<LeaseRegistry>,
    lease: String,
}

impl Drop for LeaseGuard {
    fn drop(&mut self) {
        if let Ok(mut leases) = self.registry.leases.lock() {
            leases.remove(&self.lease);
        }
    }
}

struct ConnectionSlot {
    admitted: Arc<AtomicUsize>,
}

impl ConnectionSlot {
    fn acquire(admitted: Arc<AtomicUsize>) -> Option<Self> {
        admitted
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                (current < MAX_HANDSHAKES).then_some(current + 1)
            })
            .ok()
            .map(|_| Self { admitted })
    }
}

impl Drop for ConnectionSlot {
    fn drop(&mut self) {
        self.admitted.fetch_sub(1, Ordering::AcqRel);
    }
}

fn reject_overloaded_connection(mut stream: TcpStream) {
    let _ = stream.set_write_timeout(Some(Duration::from_millis(100)));
    let _ = write_response(
        &mut stream,
        &ServerResponse::error(DaemonErrorCode::Overloaded),
    );
}

fn reap_finished_handlers(handlers: Vec<JoinHandle<()>>) -> Vec<JoinHandle<()>> {
    let mut active = Vec::with_capacity(handlers.len());
    for handler in handlers {
        if handler.is_finished() {
            let _ = handler.join();
        } else {
            active.push(handler);
        }
    }
    active
}

fn join_handlers(handlers: Vec<JoinHandle<()>>) {
    for handler in handlers {
        let _ = handler.join();
    }
}

fn daemon_io_error(operation: &str, error: io::Error) -> String {
    format!("{operation}: {error}")
}

#[cfg(test)]
#[derive(Debug, Default)]
struct HandshakePauseState {
    entered: bool,
    released: bool,
}

#[cfg(test)]
#[derive(Debug)]
struct HandshakePause {
    state: Mutex<HandshakePauseState>,
    wake: std::sync::Condvar,
}

#[cfg(test)]
pub(crate) struct HandshakePauseGuard {
    pause: Arc<HandshakePause>,
}

#[cfg(test)]
impl HandshakePauseGuard {
    pub(crate) fn wait_until_entered(&self) {
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut state = self.pause.state.lock().expect("handshake pause state");
        while !state.entered {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "daemon handler did not reach handshake pause"
            );
            let (next, timeout) = self
                .pause
                .wake
                .wait_timeout(state, remaining)
                .expect("handshake pause wait");
            state = next;
            assert!(
                !timeout.timed_out() || state.entered,
                "daemon handler pause timed out"
            );
        }
    }

    pub(crate) fn release(&self) {
        let mut state = self.pause.state.lock().expect("handshake pause state");
        state.released = true;
        self.pause.wake.notify_all();
    }
}

#[cfg(test)]
impl Drop for HandshakePauseGuard {
    fn drop(&mut self) {
        self.release();
    }
}

#[cfg(test)]
pub(crate) fn install_handshake_pause() -> HandshakePauseGuard {
    let pause = Arc::new(HandshakePause {
        state: Mutex::new(HandshakePauseState::default()),
        wake: std::sync::Condvar::new(),
    });
    HandshakePauseGuard { pause }
}

#[cfg(test)]
fn pause_before_handshake_if_configured(pause: Option<Arc<HandshakePause>>) {
    let Some(pause) = pause else {
        return;
    };
    let mut state = pause.state.lock().expect("handshake pause state");
    state.entered = true;
    pause.wake.notify_all();
    while !state.released {
        state = pause.wake.wait(state).expect("handshake pause wait");
    }
}
