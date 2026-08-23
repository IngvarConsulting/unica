use super::identity::{CoreIdentity, DaemonStateDirectory};
use super::protocol::{
    parse_request, read_bounded_json_line, ClientRequest, DaemonErrorCode, DaemonTaskSnapshot,
    EndpointRecord, InvocationRequest, InvocationResponse, ServerResponse, DAEMON_PROTOCOL_VERSION,
};
use crate::application::invocation::{
    normalized_arguments_hash, InvocationExecutor, PreparedDaemonInvocation,
};
use crate::application::invocation_store::InvocationStore;
use crate::application::operation_descriptors::ExecutionClass;
use crate::application::ports::{Clock, TokioClock};
use crate::application::tool_contracts::SurfaceRelease;
use crate::composition::open_daemon_invocation_store_from_directory;
use crate::domain::cancellation::CancellationToken;
use crate::domain::invocation::{
    DomainResult, InvocationFailure, InvocationOutcome, SafeIdentityHash,
};
use crate::infrastructure::workspace::discover_workspace;
use crate::infrastructure::workspace_actor::WorkspaceActorRegistry;
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
pub(crate) const MAX_HANDSHAKES: usize = 8;
pub(crate) const MAX_OWNER_SESSIONS: usize = 64;

pub(crate) trait CanonicalInvocationService: Send + Sync {
    fn prepare(&self, request: &InvocationRequest) -> Result<ExecutionClass, Box<DomainResult>>;

    fn execute(
        &self,
        request: InvocationRequest,
        cancellation: CancellationToken,
    ) -> Result<DomainResult, InvocationFailure>;
}

struct DormantCanonicalV13Service;

impl CanonicalInvocationService for DormantCanonicalV13Service {
    fn prepare(&self, request: &InvocationRequest) -> Result<ExecutionClass, Box<DomainResult>> {
        validate_hidden_v13_request(request)
            .map(|()| ExecutionClass::InlineCandidate)
            .map_err(|summary| Box::new(failed_domain_result(&summary)))
    }

    fn execute(
        &self,
        _request: InvocationRequest,
        _cancellation: CancellationToken,
    ) -> Result<DomainResult, InvocationFailure> {
        Ok(failed_domain_result(
            "canonical v0.13 handler is not installed before the Task 22 cutover",
        ))
    }
}

fn prepare_workspace_identity(
    request: &InvocationRequest,
    actors: &WorkspaceActorRegistry,
) -> Result<SafeIdentityHash, String> {
    let context = discover_workspace(Some(std::path::PathBuf::from(request.workspace_hint())))?;
    let source_root = context.workspace_root.clone();
    let actor = actors.get_or_create(&context, [("main", source_root)], "canonical-v0.13")?;
    actor.safe_identity_hash()
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

    fn submit(&self, request: InvocationRequest) -> Result<InvocationResponse, String> {
        let prepared = self
            .service
            .prepare(&request)
            .and_then(|class| {
                prepare_workspace_identity(&request, &self.workspace_actors)
                    .map(|workspace_identity_hash| (class, workspace_identity_hash))
                    .map_err(|summary| Box::new(failed_domain_result(&summary)))
            })
            .map(|(class, workspace_identity_hash)| {
                PreparedDaemonInvocation::new(
                    request.tool(),
                    normalized_arguments_hash(request.arguments()),
                    workspace_identity_hash,
                    class,
                    Duration::from_millis(request.response_budget_ms()),
                )
            })
            .map_err(|result| *result);
        let service = Arc::clone(&self.service);
        let execute_request = request.clone();
        self.executor
            .submit_prepared(prepared, move |cancellation| {
                service.execute(execute_request, cancellation)
            })
            .map(|outcome| match outcome {
                InvocationOutcome::Direct(result) => InvocationResponse::Direct(result),
                InvocationOutcome::Task(task) => {
                    InvocationResponse::Task(DaemonTaskSnapshot::from_domain(task))
                }
            })
    }

    fn get(
        &self,
        task_id: crate::domain::invocation::TaskId,
    ) -> Result<DaemonTaskSnapshot, String> {
        self.executor
            .get_task(task_id)
            .map(DaemonTaskSnapshot::from_domain)
    }

    fn wait(
        &self,
        task_id: crate::domain::invocation::TaskId,
        wait_ms: u64,
    ) -> Result<DaemonTaskSnapshot, String> {
        self.executor
            .wait_task(task_id, Duration::from_millis(wait_ms))
            .map(DaemonTaskSnapshot::from_domain)
    }

    fn cancel(
        &self,
        task_id: crate::domain::invocation::TaskId,
    ) -> Result<DaemonTaskSnapshot, String> {
        self.executor
            .cancel_task(task_id)
            .map(DaemonTaskSnapshot::from_domain)
    }

    fn has_active_invocations(&self) -> bool {
        self.executor.has_active_invocations()
    }
}

#[derive(Clone)]
pub(crate) struct DaemonServerConfig {
    pub(crate) state_root: std::path::PathBuf,
    pub(crate) core_identity: CoreIdentity,
    pub(crate) idle_grace: Duration,
    invocation_service: Arc<dyn CanonicalInvocationService>,
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

    let task_store_directory = state.create_private_subdirectory("tasks")?;
    let opened_store = open_daemon_invocation_store_from_directory(task_store_directory)?;
    // Recovery belongs to this daemon even before Task 7 routes work. Keeping the report beside
    // the sole-writer store makes it impossible for the stdio frontend to consume it early.
    let _recovery_classifications = opened_store.recovery.classifications.len();
    let invocation_runtime = Arc::new(DaemonInvocationRuntime::new(
        opened_store.store,
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
    let request = match read_bounded_json_line(&mut reader).and_then(|bytes| {
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
        .set_read_timeout(Some(CONNECTION_READ_TIMEOUT))
        .map_err(|error| daemon_io_error("configure daemon owner timeout", error))?;
    while !shutting_down.load(Ordering::Acquire) {
        let request = match read_bounded_json_line(&mut reader) {
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
            ClientRequest::Ping {} => write_response(&mut stream, &ServerResponse::Pong)?,
            ClientRequest::Release {} => {
                write_response(&mut stream, &ServerResponse::Released)?;
                break;
            }
            ClientRequest::Hello { .. } => {
                write_response(
                    &mut stream,
                    &ServerResponse::error(DaemonErrorCode::InvalidRequest),
                )?;
                break;
            }
            ClientRequest::SubmitInvocation { invocation } => match invocation_runtime
                .submit(invocation)
            {
                Ok(outcome) => write_response(&mut stream, &ServerResponse::invocation(outcome))?,
                Err(_) => write_response(
                    &mut stream,
                    &ServerResponse::error(DaemonErrorCode::InvocationFailed),
                )?,
            },
            ClientRequest::GetTask { task_id } => {
                write_task_response(&mut stream, invocation_runtime.get(task_id))?
            }
            ClientRequest::WaitTask { task_id, wait_ms } => {
                write_task_response(&mut stream, invocation_runtime.wait(task_id, wait_ms))?
            }
            ClientRequest::CancelTask { task_id } => {
                write_task_response(&mut stream, invocation_runtime.cancel(task_id))?
            }
        }
    }
    Ok(())
}

fn write_task_response(
    stream: &mut TcpStream,
    result: Result<DaemonTaskSnapshot, String>,
) -> Result<(), String> {
    match result {
        Ok(snapshot) => write_response(stream, &ServerResponse::task(snapshot)),
        Err(error) if error == "task record not found" => write_response(
            stream,
            &ServerResponse::error(DaemonErrorCode::TaskNotFound),
        ),
        Err(error) if error == "task record expired" => {
            write_response(stream, &ServerResponse::error(DaemonErrorCode::TaskExpired))
        }
        Err(_) => write_response(stream, &ServerResponse::error(DaemonErrorCode::StoreFailed)),
    }
}

fn write_response(stream: &mut TcpStream, response: &ServerResponse) -> Result<(), String> {
    let mut bytes = serde_json::to_vec(response)
        .map_err(|_| "daemon response could not be serialized".to_string())?;
    bytes.push(b'\n');
    stream
        .write_all(&bytes)
        .and_then(|_| stream.flush())
        .map_err(|error| daemon_io_error("write daemon response", error))
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
