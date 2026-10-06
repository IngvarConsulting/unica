//! Public `unica` stdio MCP server on the official Rust SDK (`rmcp`).
//!
//! ADR-0013: the SDK owns the JSON-RPC loop and handshake after the optional
//! initial `server/discover` probe, protocol version negotiation, per-request
//! task spawning, `ping`, and `notifications/cancelled` bookkeeping. This
//! module maps SDK requests onto the transport-neutral application layer
//! (ADR-0002) and keeps the tool contract data-driven from operation
//! descriptors (ADR-0001) instead of SDK macros.

use super::canonical_cancellation::CanonicalCancellation;
use super::daemon_router::{
    canonical_daemon_router, CanonicalCallOutcome, CanonicalDaemonRouter,
    FrontendInvocationDeadline, TOOL_EXECUTION_ERROR,
};
mod manual_calls;
mod receive_pump;
#[cfg(test)]
use super::daemon_router::{CanonicalCallHandler, CanonicalTaskHandler, CanonicalTaskWaitHandler};
use crate::application::receipt_ledger::V5ToolIdentity;
use crate::application::tool_contracts::{SurfaceRelease, V13TaskProfile};
use crate::application::{
    code_search_output_schema, input_schema_for_tool, metadata_argument_failure_result,
    operation_result_output_schema, role_edit_argument_failure_result, role_edit_output_schema,
    strip_schema_descriptions, CodeIntelligenceOperation, OperationResult, ToolHandler, ToolSpec,
    UnicaApplication,
};
use crate::domain::cancellation::CancellationToken;
use crate::domain::progress::{NoopProgressSink, ProgressEvent, ProgressSink};
use crate::domain::refusal::RefusalDetail;
use manual_calls::ManualCalls;
use rmcp::model::{
    CacheScope, CallToolRequestParams, CallToolResponse, CallToolResult, CancelTaskParams,
    ClientJsonRpcMessage, ClientNotification, ClientRequest, ContentBlock, DiscoverResult,
    ErrorCode, ErrorData, GetMeta, GetTaskParams, GetTaskResult, Implementation,
    InitializeRequestParams, InitializeResult, ListPromptsResult, ListResourceTemplatesResult,
    ListResourcesResult, ListToolsResult, NotificationMetaObject, PaginatedRequestParams,
    ProgressNotificationParam, ProgressToken, ProtocolVersion, RequestMetaObject,
    ServerCapabilities, ServerInfo, ServerJsonRpcMessage, ServerResult, Tool, UpdateTaskParams,
    TASKS_EXTENSION_ID,
};
use rmcp::service::{NotificationContext, RequestContext, ServerInitializeError};
use rmcp::transport::Transport;
use rmcp::{RoleServer, ServerHandler, ServiceExt};
use serde_json::{Map, Value};
use std::collections::HashSet;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

#[cfg(test)]
use crate::infrastructure::daemon::client_v5::V5DaemonProcessOwner;
use crate::infrastructure::daemon::client_v5::{V5DaemonClient, V5TaskExchangeError};
use crate::infrastructure::daemon::protocol_v5::{V5DaemonErrorCode, V5DaemonTaskSnapshot};

const EOF_CANCELLATION_GRACE: Duration = Duration::from_secs(2);
const RUNTIME_SHUTDOWN_GRACE: Duration = Duration::from_millis(250);

/// Executes one tool call synchronously without leaking SDK types into the application.
/// Injectable so transport tests can substitute slow or failing tools.
type ToolCallHandler = dyn Fn(
        &str,
        &Map<String, Value>,
        CancellationToken,
        Arc<dyn ProgressSink>,
    ) -> Result<OperationResult, (i32, String)>
    + Send
    + Sync;

#[derive(Clone)]
enum SurfaceToolRouter {
    #[allow(dead_code)] // constructed only by the explicit legacy test seam
    LegacyV12(Arc<ToolCallHandler>),
    CanonicalV13(CanonicalDaemonRouter),
}

enum SurfaceToolOutcome {
    Legacy(Box<OperationResult>),
    /// A compatibility Task tool answered with a canonical result to project.
    Canonical(crate::domain::invocation::DomainResult),
    /// An acknowledged Direct terminal, already the final `CallToolResult`.
    Direct(CallToolResult),
    Task(V5DaemonTaskSnapshot),
}

pub fn run_stdio() {
    if SurfaceRelease::from_package_version() != SurfaceRelease::V13 {
        eprintln!("this package does not select the canonical v0.13 MCP surface");
        return;
    }
    let state_root = match crate::interfaces::daemon::default_user_daemon_state_root() {
        Ok(root) => root,
        Err(error) => {
            eprintln!("failed to resolve unica user daemon state: {error}");
            return;
        }
    };
    let client = match crate::interfaces::daemon::connect_default_user_daemon(&state_root) {
        Ok(client) => client,
        Err(error) => {
            eprintln!("failed to connect to unica user daemon: {error}");
            return;
        }
    };
    let workspace = unica_bootstrap::capture_host_workspace_context();
    let notice = startup_notice_from(std::env::var(STARTUP_NOTICE_ENV).ok());
    let server = UnicaServer::canonical_v13_daemon(client, workspace, notice);
    let in_flight = server.in_flight();
    let manual_calls = server.manual_calls.clone();

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("failed to start unica mcp runtime: {error}");
            return;
        }
    };
    runtime.block_on(async move {
        let (stdin, stdout) = rmcp::transport::stdio();
        let transport = rmcp::transport::async_rw::AsyncRwTransport::new_server(stdin, stdout);
        let transport = DiscoveryProbeTransport::new(transport, &server);
        match ObservedServer(server).serve(transport).await {
            Ok(running) => {
                let _ = running.waiting().await;
            }
            // A host that closes stdin before the handshake is a clean shutdown,
            // matching the pre-SDK loop; anything else is worth a stderr line.
            Err(error) if matches!(error.as_ref(), ServerInitializeError::ConnectionClosed(_)) => {}
            Err(error) => eprintln!("unica mcp initialization failed: {error}"),
        }
    });
    // The SDK drained finishing calls before `waiting()` returned. Whatever is
    // still running is cancelled and given a bounded grace so tool
    // implementations can terminate their child process trees.
    if !drain_stdio_frontend(&in_flight, &manual_calls) {
        eprintln!(
            "unica mcp shutdown grace expired while tool calls or provider workers were cleaning up"
        );
    }
    runtime.shutdown_timeout(RUNTIME_SHUTDOWN_GRACE);
}

fn drain_stdio_frontend(in_flight: &InFlightRegistry, manual_calls: &ManualCalls) -> bool {
    manual_calls.wait_manual_settled();
    drain_mcp_shutdown(in_flight, EOF_CANCELLATION_GRACE)
}

/// `rmcp` treats its first `server/discover` as a permanent modern opener. A
/// host may probe first and then choose `initialize`; answer only that initial
/// probe before handing the actual opener to the SDK. Once an opener reaches
/// the SDK, all requests and their protocol checks belong to it unchanged.
struct DiscoveryProbeTransport<T: Transport<RoleServer>> {
    inner: receive_pump::ReceivePump<T::Error>,
    discovery: DiscoverResult,
    awaiting_opener: bool,
    manual_calls: Option<Arc<ManualCalls>>,
}

impl<T: Transport<RoleServer> + 'static> DiscoveryProbeTransport<T> {
    fn new(inner: T, server: &UnicaServer) -> Self {
        Self {
            inner: receive_pump::ReceivePump::new(
                inner,
                server.manual_calls.clone(),
                matches!(server.router, SurfaceToolRouter::CanonicalV13(_)),
            ),
            discovery: DiscoverResult::from_server_info(
                server.supported_protocol_versions().into_owned(),
                server.get_info(),
            ),
            awaiting_opener: true,
            manual_calls: matches!(server.router, SurfaceToolRouter::CanonicalV13(_))
                .then(|| server.manual_calls.clone()),
        }
    }
}

impl<T: Transport<RoleServer> + 'static> Transport<RoleServer> for DiscoveryProbeTransport<T> {
    type Error = receive_pump::PumpError<T::Error>;

    fn send(
        &mut self,
        item: ServerJsonRpcMessage,
    ) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send + 'static {
        let tracked = self.manual_calls.as_ref().and_then(|calls| {
            let id = match &item {
                ServerJsonRpcMessage::Response(response) => Some(&response.id),
                ServerJsonRpcMessage::Error(error) => error.id.as_ref(),
                _ => None,
            }?;
            calls.get(id).map(|call| (calls.clone(), id.clone(), call))
        });
        if let Some((calls, _, call)) = &tracked {
            calls.response_seen(call);
        }
        // Once inner.send starts it may have written part of a JSON frame.
        // A later Cancel controls the daemon independently; it never drops
        // this send future or leaves a truncated frame on stdout.
        let suppressed = tracked
            .as_ref()
            .is_some_and(|(_, _, call)| call.cancellation.requested());
        let sending = (!suppressed).then(|| self.inner.send(item));
        async move {
            let result = match sending {
                Some(sending) => sending.await,
                None => Ok(()),
            };
            if result.is_ok() {
                if let Some((calls, id, call)) = tracked {
                    calls.delivery_done(&id, &call);
                }
            }
            result
        }
    }

    async fn receive(&mut self) -> Option<ClientJsonRpcMessage> {
        loop {
            let message = self.inner.receive().await?;
            if self.awaiting_opener {
                if let ClientJsonRpcMessage::Request(request) = &message {
                    match &request.request {
                        ClientRequest::DiscoverRequest(_)
                            if request
                                .request
                                .get_meta()
                                .missing_required_keys(&ProtocolVersion::V_2026_07_28)
                                .is_empty()
                                && request.request.get_meta().protocol_version().is_some_and(
                                    |version| self.discovery.supported_versions.contains(&version),
                                ) =>
                        {
                            let response = ServerJsonRpcMessage::response(
                                ServerResult::DiscoverResult(self.discovery.clone()),
                                request.id.clone(),
                            );
                            if let Err(error) = self.inner.send(response).await {
                                eprintln!("unica mcp discovery probe response failed: {error}");
                                return None;
                            }
                            continue;
                        }
                        ClientRequest::PingRequest(_) => return Some(message),
                        _ => {}
                    }
                }
                self.awaiting_opener = false;
            }
            return Some(message);
        }
    }

    fn close(&mut self) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send {
        let calls = self.manual_calls.clone();
        async move {
            let result = self.inner.close().await;
            if let Some(calls) = calls {
                calls.closed();
            }
            result
        }
    }
}

fn drain_mcp_shutdown(in_flight: &InFlightRegistry, grace: Duration) -> bool {
    drain_mcp_shutdown_with(in_flight, grace, |remaining| {
        let deadline = Instant::now() + remaining;
        let code_search_idle =
            crate::application::code_intelligence::drain_code_search_workers(remaining);
        let diagnostics_idle = crate::application::diagnostics::drain_diagnostic_workers(
            deadline.saturating_duration_since(Instant::now()),
        );
        code_search_idle && diagnostics_idle
    })
}

fn drain_mcp_shutdown_with(
    in_flight: &InFlightRegistry,
    grace: Duration,
    drain_providers: impl FnOnce(Duration) -> bool,
) -> bool {
    let deadline = Instant::now() + grace;
    in_flight.cancel_all();
    let calls_idle = in_flight.wait_idle(deadline.saturating_duration_since(Instant::now()));
    let providers_idle = drain_providers(deadline.saturating_duration_since(Instant::now()));
    calls_idle && providers_idle
}

/// Переменная, в которой загрузчик передаёт рассказ о прошлом запуске.
///
/// Убитая установка своего провода не имела: он появляется только здесь, и
/// рассказать о ней может лишь тот, кого запустили следом.
const STARTUP_NOTICE_ENV: &str = "UNICA_STARTUP_NOTICE";

const CANONICAL_INSTRUCTIONS: &str = "Start with unica.view using an empty object when the workspace or logical address is unknown. Use returned addresses instead of guessing at. A qualified logical address has the form <sourceSet>:<Kind>[.<Name>...]. Use unica.check to confirm source-set admission or logical-node readability.";

pub struct UnicaServer {
    router: SurfaceToolRouter,
    in_flight: Arc<InFlightRegistry>,
    manual_calls: Arc<ManualCalls>,
    structured_tools: HashSet<&'static str>,
    /// О чём рассказать вызывающему при рукопожатии. Обычная сессия платит за
    /// это ноль байтов поверхности: рассказывать нечего.
    startup_notice: Option<String>,
}

#[allow(dead_code)]
fn assert_unica_server_implements_official_rmcp_server_handler()
where
    UnicaServer: ::rmcp::ServerHandler,
{
}

/// Пустое значение — это «нечего рассказывать», а не пустой рассказ.
fn startup_notice_from(value: Option<String>) -> Option<String> {
    let notice = value?.trim().to_owned();
    (!notice.is_empty()).then_some(notice)
}

impl UnicaServer {
    #[cfg(test)]
    fn legacy_for_test(handler: Arc<ToolCallHandler>) -> Self {
        let notice = startup_notice_from(std::env::var(STARTUP_NOTICE_ENV).ok());
        Self::legacy_with_startup_notice_for_test(handler, notice)
    }

    #[cfg(test)]
    fn legacy_with_startup_notice_for_test(
        handler: Arc<ToolCallHandler>,
        startup_notice: Option<String>,
    ) -> Self {
        Self {
            router: SurfaceToolRouter::LegacyV12(handler),
            in_flight: Arc::new(InFlightRegistry::default()),
            manual_calls: Arc::new(ManualCalls::default()),
            structured_tools: crate::application::tools()
                .into_iter()
                .filter_map(|spec| has_structured_output(&spec).then_some(spec.name))
                .collect(),
            startup_notice,
        }
    }

    #[cfg(test)]
    fn with_canonical_v13(handler: Arc<CanonicalCallHandler>) -> Self {
        let unavailable: Arc<CanonicalTaskHandler> =
            Arc::new(|_, _| Err(V5TaskExchangeError::Transport));
        Self::with_canonical_v13_tasks(handler, Arc::clone(&unavailable), unavailable)
    }

    #[cfg(test)]
    fn with_canonical_v13_tasks(
        call: Arc<CanonicalCallHandler>,
        get: Arc<CanonicalTaskHandler>,
        cancel: Arc<CanonicalTaskHandler>,
    ) -> Self {
        let wait_get = Arc::clone(&get);
        let wait: Arc<CanonicalTaskWaitHandler> =
            Arc::new(move |task_id, _, deadline, _| wait_get(task_id, deadline));
        Self::with_canonical_v13_task_handlers(call, get, wait, cancel)
    }

    #[cfg(test)]
    fn with_canonical_v13_task_handlers(
        call: Arc<CanonicalCallHandler>,
        get: Arc<CanonicalTaskHandler>,
        wait: Arc<CanonicalTaskWaitHandler>,
        cancel: Arc<CanonicalTaskHandler>,
    ) -> Self {
        Self {
            router: SurfaceToolRouter::CanonicalV13(CanonicalDaemonRouter {
                call,
                get,
                wait,
                cancel,
            }),
            in_flight: Arc::new(InFlightRegistry::default()),
            manual_calls: Arc::new(ManualCalls::default()),
            structured_tools: HashSet::new(),
            startup_notice: None,
        }
    }

    fn canonical_v13_daemon(
        client: V5DaemonClient,
        workspace: unica_bootstrap::HostWorkspaceContext,
        startup_notice: Option<String>,
    ) -> Self {
        let router = canonical_daemon_router(client, workspace);
        Self {
            router: SurfaceToolRouter::CanonicalV13(router),
            in_flight: Arc::new(InFlightRegistry::default()),
            manual_calls: Arc::new(ManualCalls::default()),
            structured_tools: HashSet::new(),
            startup_notice,
        }
    }

    #[cfg(test)]
    fn with_canonical_daemon(owner: V5DaemonProcessOwner, workspace_hint: String) -> Self {
        Self::canonical_v13_daemon(owner.into(), workspace_hint.into(), None)
    }

    fn in_flight(&self) -> Arc<InFlightRegistry> {
        Arc::clone(&self.in_flight)
    }
}

struct SurfaceToolCall<'a> {
    name: &'a str,
    arguments: &'a Map<String, Value>,
    host: &'a unica_bootstrap::HostRequest,
}

fn execute_surface_tool(
    router: &SurfaceToolRouter,
    call: SurfaceToolCall<'_>,
    cancellation: impl Into<CanonicalCancellation>,
    progress: Arc<dyn ProgressSink>,
    deadline: FrontendInvocationDeadline,
    client_supports_tasks: bool,
) -> Result<SurfaceToolOutcome, ErrorData> {
    let cancellation = cancellation.into();
    let SurfaceToolCall {
        name,
        arguments,
        host,
    } = call;
    match router {
        SurfaceToolRouter::LegacyV12(handler) => {
            handler(name, arguments, cancellation.token(), progress)
                .map(Box::new)
                .map(SurfaceToolOutcome::Legacy)
                .map_err(|(code, message)| ErrorData::new(ErrorCode(code), message, None))
        }
        SurfaceToolRouter::CanonicalV13(router) => {
            if let Some(request) =
                crate::application::v13::task_tools::parse_task_tool_call(name, arguments)
            {
                if client_supports_tasks {
                    return Err(ErrorData::invalid_params(
                        "compatibility task tools are unavailable when native Tasks is active",
                        None,
                    ));
                }
                return Ok(SurfaceToolOutcome::Canonical(
                    execute_compatibility_task_tool(router, request, deadline, cancellation),
                ));
            }
            let tool = V5ToolIdentity::from_wire_name(name).ok_or_else(|| {
                ErrorData::invalid_params("tool is not in the canonical v0.13 profile", None)
            })?;
            match (router.call)(tool, arguments, host, deadline, cancellation)? {
                CanonicalCallOutcome::Direct(result) => Ok(SurfaceToolOutcome::Direct(result)),
                CanonicalCallOutcome::Task(snapshot) if client_supports_tasks => {
                    Ok(SurfaceToolOutcome::Task(snapshot))
                }
                CanonicalCallOutcome::Task(snapshot) => Ok(SurfaceToolOutcome::Canonical(
                    project_compatibility_snapshot(&snapshot, CompatibilityProjection::State),
                )),
            }
        }
    }
}

use crate::application::v13::task_tools::{
    CompatibilityProjection, CompatibilityTaskSnapshot, TaskToolAction, TaskToolError,
};

fn execute_compatibility_task_tool(
    router: &CanonicalDaemonRouter,
    request: Result<crate::application::v13::task_tools::TaskToolRequest, TaskToolError>,
    deadline: FrontendInvocationDeadline,
    cancellation: CanonicalCancellation,
) -> crate::domain::invocation::DomainResult {
    let request = match request {
        Ok(request) => request,
        Err(error) => return crate::application::v13::task_tools::task_tool_error_result(error),
    };
    let exchange = match request.action {
        TaskToolAction::Get => (router.get)(request.task_id, deadline),
        TaskToolAction::Result { wait_ms } => {
            let bounded = bounded_compatibility_wait_ms(wait_ms, deadline, Instant::now());
            (router.wait)(request.task_id, bounded, deadline, cancellation)
        }
        TaskToolAction::Cancel => (router.cancel)(request.task_id, deadline),
    };
    let snapshot = match exchange {
        Ok(snapshot) if snapshot.task_id() == request.task_id => snapshot,
        Ok(_) => {
            return crate::application::v13::task_tools::task_tool_error_result(
                TaskToolError::TaskProtocolFailed,
            )
        }
        Err(error) => {
            return crate::application::v13::task_tools::task_tool_error_result(
                compatibility_task_exchange_error(error),
            )
        }
    };
    let projection = match request.action {
        TaskToolAction::Result { .. } => CompatibilityProjection::TerminalResult,
        TaskToolAction::Get | TaskToolAction::Cancel => CompatibilityProjection::State,
    };
    project_compatibility_snapshot(&snapshot, projection)
}

fn bounded_compatibility_wait_ms(
    requested_wait_ms: u64,
    deadline: FrontendInvocationDeadline,
    now: Instant,
) -> u64 {
    let remaining_ms = deadline
        .remaining_at(now)
        .as_millis()
        .min(u128::from(u64::MAX)) as u64;
    requested_wait_ms.min(remaining_ms)
}

fn compatibility_task_exchange_error(error: V5TaskExchangeError) -> TaskToolError {
    match error {
        V5TaskExchangeError::Protocol(V5DaemonErrorCode::TaskNotFound) => {
            TaskToolError::TaskNotFound
        }
        V5TaskExchangeError::Protocol(V5DaemonErrorCode::TaskExpired) => TaskToolError::TaskExpired,
        V5TaskExchangeError::Protocol(code) => {
            TaskToolError::TaskBackendFailed(backend_detail(code))
        }
        V5TaskExchangeError::Transport => TaskToolError::TaskTransportFailed,
        V5TaskExchangeError::SessionPoisoned => TaskToolError::TaskSessionClosed,
        V5TaskExchangeError::UnexpectedResponse => TaskToolError::TaskProtocolFailed,
    }
}

/// The compatibility receipt carries the durable state only: the closed v5
/// snapshot cannot violate the status/result/failure matrix, and a failure
/// reaches the host as presence, never as text.
fn project_compatibility_snapshot(
    snapshot: &V5DaemonTaskSnapshot,
    projection: CompatibilityProjection,
) -> crate::domain::invocation::DomainResult {
    let state = CompatibilityTaskSnapshot::new(
        snapshot.task_id(),
        snapshot.status(),
        snapshot.completed_result().cloned(),
        snapshot.failure_reason().is_some(),
        snapshot.cancel_requested(),
        snapshot.created_at_epoch_ms(),
        snapshot.updated_at_epoch_ms(),
        snapshot.ttl_ms(),
        snapshot.poll_interval_ms(),
    );
    crate::application::v13::task_tools::project_task_snapshot(&state, projection).unwrap_or_else(
        |_| {
            crate::application::v13::task_tools::task_tool_error_result(
                TaskToolError::ProjectionFailed,
            )
        },
    )
}

fn structured_output_schema(spec: &ToolSpec) -> Option<Value> {
    match spec.handler {
        ToolHandler::Metadata { .. } => Some(operation_result_output_schema()),
        ToolHandler::NativeOperation {
            operation: "role-edit",
            ..
        } => Some(role_edit_output_schema()),
        ToolHandler::CodeIntelligence {
            operation: CodeIntelligenceOperation::Search,
        } => Some(code_search_output_schema()),
        _ => None,
    }
}

#[allow(dead_code)] // legacy surface test support; production selects canonical V13
fn has_structured_output(spec: &ToolSpec) -> bool {
    structured_output_schema(spec).is_some()
}

/// Page size for the modern-era `tools/list` (legacy peers get the whole
/// registry in one page, exactly as before pagination existed).
const TOOLS_PAGE_SIZE: usize = 25;

/// Validate a client-presented cursor against the offsets this server issues:
/// positive multiples of the page size strictly inside the collection.
fn parse_issued_cursor(cursor: &str, page_size: usize, len: usize) -> Result<usize, ErrorData> {
    let issued = |offset: usize| offset != 0 && offset.is_multiple_of(page_size) && offset < len;
    match cursor.parse::<usize>() {
        Ok(offset) if issued(offset) => Ok(offset),
        _ => Err(ErrorData::invalid_params(
            format!("cursor was not issued by this server: {cursor:?}"),
            None,
        )),
    }
}

/// The full registry projection is ~1.3 MB of JSON and is immutable for the
/// process lifetime; build it once instead of once per page.
fn all_tool_definitions() -> &'static [Tool] {
    static ALL: std::sync::OnceLock<Vec<Tool>> = std::sync::OnceLock::new();
    ALL.get_or_init(|| tool_definitions(&crate::application::tools()))
}

fn v13_tool_definitions(profile: V13TaskProfile) -> &'static [Tool] {
    static NATIVE: std::sync::OnceLock<Vec<Tool>> = std::sync::OnceLock::new();
    static COMPATIBILITY: std::sync::OnceLock<Vec<Tool>> = std::sync::OnceLock::new();
    let build = || {
        let catalog = crate::application::v13::tool_catalog::catalog_for(SurfaceRelease::V13)
            .expect("canonical v0.13 profile has a catalog");
        let mut tools = catalog
            .tools
            .into_iter()
            .map(|contract| {
                v13_tool_definition(
                    contract.name,
                    Some(contract.description),
                    contract.input_schema,
                )
            })
            .collect::<Vec<_>>();
        if profile == V13TaskProfile::Compatibility {
            tools.extend(
                crate::application::v13::task_tools::compatibility_tool_contracts()
                    .into_iter()
                    .map(|contract| {
                        v13_tool_definition(
                            contract.name,
                            Some(contract.description),
                            contract.input_schema,
                        )
                    }),
            );
        }
        tools
    };
    match profile {
        V13TaskProfile::Native => NATIVE.get_or_init(build),
        V13TaskProfile::Compatibility => COMPATIBILITY.get_or_init(build),
    }
}

fn v13_tool_definition(name: &str, description: Option<&str>, schema: Value) -> Tool {
    let schema = match schema {
        Value::Object(schema) => schema,
        other => unreachable!("V13 tool unica.{name} produced non-object schema: {other}"),
    };
    let mut tool = Tool::new(
        format!("unica.{name}"),
        description.unwrap_or_default().to_string(),
        schema,
    );
    if description.is_none() {
        tool.description = None;
    }
    tool
}

/// SEP-2549 cache fields are required on list results from protocol revision
/// 2026-07-28; older peers must keep the exact legacy wire shape.
fn modern_peer(context: &RequestContext<RoleServer>) -> bool {
    context
        .protocol_version()
        .is_some_and(|version| version.as_str() >= ProtocolVersion::V_2026_07_28.as_str())
}

fn modern_protocol_authority(context: &RequestContext<RoleServer>) -> bool {
    context
        .protocol_version()
        .is_some_and(|version| version == ProtocolVersion::V_2026_07_28)
        && context
            .peer
            .peer_info()
            .is_none_or(|peer| peer.protocol_version == ProtocolVersion::V_2026_07_28)
}

/// The served protocol versions are exactly the #490 guaranteed matrix: the
/// two legacy `initialize` revisions real hosts speak today plus the modern
/// direct-first lifecycle. Older revisions are not offered — an accepted
/// handshake would promise semantics nobody verifies.
const SUPPORTED_PROTOCOL_VERSIONS: &[ProtocolVersion] = &[
    ProtocolVersion::V_2025_06_18,
    ProtocolVersion::V_2025_11_25,
    ProtocolVersion::V_2026_07_28,
];

/// Observe the SDK's actual dispatch completion, including validation refusal
/// before `call_tool`, without duplicating the SDK's request dispatch rules.
struct ObservedServer(UnicaServer);

impl ObservedServer {
    async fn serve<T: Transport<RoleServer> + 'static>(
        self,
        transport: T,
    ) -> Result<rmcp::service::RunningService<RoleServer, Self>, Box<ServerInitializeError>> {
        let calls = self.0.manual_calls.clone();
        let result = ServiceExt::serve(self, transport).await;
        if result.is_err() {
            // No running SDK loop was created. Stop the input pump and await
            // actual close before proving there is no future dispatch.
            calls.initialization_failed().await;
        }
        result.map_err(Box::new)
    }
}

impl rmcp::Service<RoleServer> for ObservedServer {
    async fn handle_request(
        &self,
        request: ClientRequest,
        context: RequestContext<RoleServer>,
    ) -> Result<ServerResult, ErrorData> {
        let is_call = matches!(&request, ClientRequest::CallToolRequest(_));
        let _dispatch = is_call
            .then(|| self.0.manual_calls.dispatch(context.id.clone()))
            .flatten();
        if is_call {
            self.0.manual_calls.before_handler().await;
        }
        rmcp::Service::<RoleServer>::handle_request(&self.0, request, context).await
    }

    async fn handle_notification(
        &self,
        notification: ClientNotification,
        context: NotificationContext<RoleServer>,
    ) -> Result<(), ErrorData> {
        rmcp::Service::<RoleServer>::handle_notification(&self.0, notification, context).await
    }

    fn get_info(&self) -> ServerInfo {
        ServerHandler::get_info(&self.0)
    }

    fn supported_protocol_versions(&self) -> std::borrow::Cow<'static, [ProtocolVersion]> {
        ServerHandler::supported_protocol_versions(&self.0)
    }
}

impl ServerHandler for UnicaServer {
    fn supported_protocol_versions(&self) -> std::borrow::Cow<'static, [ProtocolVersion]> {
        std::borrow::Cow::Borrowed(SUPPORTED_PROTOCOL_VERSIONS)
    }

    fn get_info(&self) -> ServerInfo {
        // #490: the negotiation fallback is pinned, not inherited from the
        // SDK LATEST constant, so an SDK bump cannot move it silently.
        //
        // Only the implemented surface is declared. Prompts, resources,
        // completions, logging and ui stay withheld. Tasks are advertised only
        // by the injected V13 router and initialize strips them again unless
        // the negotiated protocol is 2026-07-28.
        let mut capabilities = match &self.router {
            SurfaceToolRouter::LegacyV12(_) => ServerCapabilities::builder().enable_tools().build(),
            SurfaceToolRouter::CanonicalV13(_) => ServerCapabilities::builder()
                .enable_tools()
                .enable_tasks()
                .build(),
        };
        if matches!(&self.router, SurfaceToolRouter::CanonicalV13(_)) {
            capabilities.experimental = Some(unica_bootstrap::host_workspace_capabilities());
        }
        let info = InitializeResult::new(capabilities)
            .with_protocol_version(ProtocolVersion::V_2025_11_25)
            .with_server_info(Implementation::new("unica", env!("CARGO_PKG_VERSION")));
        // Что осталось от убитого запуска, дополняет стабильный маршрут первого
        // вызова: notice не должен стирать инструкцию дискавери и наоборот.
        let instructions = match &self.startup_notice {
            Some(notice) => format!("{CANONICAL_INSTRUCTIONS}\n\nStartup notice: {notice}"),
            None => CANONICAL_INSTRUCTIONS.to_string(),
        };
        info.with_instructions(instructions)
    }

    async fn initialize(
        &self,
        request: InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, ErrorData> {
        context.peer.set_peer_info(request.clone());
        let mut info = self.get_info();
        info.protocol_version = if SUPPORTED_PROTOCOL_VERSIONS.contains(&request.protocol_version) {
            request.protocol_version
        } else {
            info.protocol_version
        };
        if info.protocol_version.as_str() < ProtocolVersion::V_2026_07_28.as_str() {
            if let Some(extensions) = info.capabilities.extensions.as_mut() {
                extensions.remove(TASKS_EXTENSION_ID);
                if extensions.is_empty() {
                    info.capabilities.extensions = None;
                }
            }
        }
        Ok(info)
    }

    fn accepted_subscription_filter(
        &self,
        requested: &rmcp::model::SubscriptionFilter,
    ) -> Option<rmcp::model::SubscriptionFilter> {
        // Accept `subscriptions/listen` instead of failing it with -32601:
        // the SDK intersects the answer with the advertised capabilities, so
        // with no listChanged declared the accepted set is empty but the
        // stream is acknowledged — a client that probes anyway gets a clean
        // no-op subscription rather than an error-retry loop.
        Some(requested.clone())
    }

    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let (all, modern) = match &self.router {
            SurfaceToolRouter::LegacyV12(_) => (all_tool_definitions(), modern_peer(&context)),
            SurfaceToolRouter::CanonicalV13(_) => {
                let profile = if native_task_capability(&context) {
                    V13TaskProfile::Native
                } else {
                    V13TaskProfile::Compatibility
                };
                (
                    v13_tool_definitions(profile),
                    modern_protocol_authority(&context),
                )
            }
        };
        let cursor = request.and_then(|request| request.cursor);
        if !modern {
            // #490: the legacy surface is served whole; no cursor is ever
            // issued there, so a presented cursor is a contract violation.
            if let Some(cursor) = cursor {
                return Err(ErrorData::invalid_params(
                    format!("cursor is not part of the legacy tools/list contract: {cursor:?}"),
                    None,
                ));
            }
            return Ok(ListToolsResult::with_all_items(all.to_vec()));
        }
        // Modern peers page through the registry; only offsets this server
        // issued are valid cursors.
        let offset = match cursor {
            None => 0,
            Some(cursor) => parse_issued_cursor(&cursor, TOOLS_PAGE_SIZE, all.len())?,
        };
        let end = (offset + TOOLS_PAGE_SIZE).min(all.len());
        let mut result = ListToolsResult::with_all_items(all[offset..end].to_vec());
        if end < all.len() {
            result.next_cursor = Some(end.to_string());
        }
        // 2026-07-28 list results require the SEP-2549 cache fields; ttlMs 0
        // keeps the "tools/list is not cacheable" policy while satisfying the
        // modern wire schema.
        Ok(result.with_ttl_ms(0).with_cache_scope(CacheScope::Private))
    }

    async fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListPromptsResult, ErrorData> {
        let mut result = ListPromptsResult::with_all_items(Vec::new());
        if modern_peer(&context) {
            result = result.with_ttl_ms(0).with_cache_scope(CacheScope::Private);
        }
        Ok(result)
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        let mut result = ListResourcesResult::with_all_items(Vec::new());
        if modern_peer(&context) {
            result = result.with_ttl_ms(0).with_cache_scope(CacheScope::Private);
        }
        Ok(result)
    }

    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, ErrorData> {
        let mut result = ListResourceTemplatesResult::with_all_items(Vec::new());
        if modern_peer(&context) {
            result = result.with_ttl_ms(0).with_cache_scope(CacheScope::Private);
        }
        Ok(result)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let received_at = Instant::now();
        let client_supports_tasks = native_task_capability(&context);
        // The SDK may place wire metadata in the request or the context.
        // Keep it separate from model-selected tool arguments.
        let mut metadata = context.meta.0 .0.clone();
        if let Some(request_meta) = request.meta.as_ref() {
            metadata.extend(request_meta.0 .0.clone());
        }
        // Asked before the call is admitted: until then the call owns no
        // daemon work, so a transport closing during this exchange cannot
        // strand accepted work.
        let roots = if matches!(self.router, SurfaceToolRouter::CanonicalV13(_))
            && V5ToolIdentity::from_wire_name(&request.name).is_some()
        {
            client_roots(&context, &metadata).await
        } else {
            unica_bootstrap::ClientRoots::NotDeclared
        };
        let host = unica_bootstrap::HostRequest { metadata, roots };
        let admission = self
            .in_flight
            .admit()
            .map_err(|message| ErrorData::new(ErrorCode::INTERNAL_ERROR, message, None))?;
        let canonical = matches!(self.router, SurfaceToolRouter::CanonicalV13(_));
        let (manual_owner, standalone_admission, cancellation) = if canonical {
            match self.manual_calls.enter(context.id.clone(), admission) {
                Ok(owner) => {
                    let cancellation = owner.cancellation();
                    (Some(owner), None, cancellation)
                }
                Err(admission) => (None, Some(admission), CanonicalCancellation::default()),
            }
        } else {
            let cancellation = CanonicalCancellation::from(admission.token());
            (None, Some(admission), cancellation)
        };
        // SDK normal completion and run_stdio's EOF drain are independent
        // from canonical manual cancellation. Legacy calls retain their
        // established SDK/admission cancellation behavior.
        let bridge = (!canonical).then(|| {
            let sdk_token = context.ct.clone();
            let bridged = cancellation.token();
            tokio::spawn(async move {
                sdk_token.cancelled().await;
                bridged.cancel();
            })
        });

        let router = self.router.clone();
        let name = request.name.to_string();
        let handler_name = name.clone();
        let progress_token = request
            .meta
            .as_ref()
            .and_then(RequestMetaObject::get_progress_token)
            .or_else(|| context.meta.get_progress_token());
        let arguments = request.arguments.unwrap_or_default();
        let progress_forwarding = if let Some(progress_token) = progress_token {
            let (sender, mut receiver) =
                tokio::sync::mpsc::unbounded_channel::<Option<ProgressEvent>>();
            let sink: Arc<dyn ProgressSink> = Arc::new(McpProgressSink {
                sender: sender.clone(),
            });
            let peer = context.peer.clone();
            let forwarder = tokio::spawn(async move {
                while let Some(message) = receiver.recv().await {
                    let Some(event) = message else {
                        break;
                    };
                    let notification = progress_notification(progress_token.clone(), &event);
                    let _ = peer.notify_progress(notification).await;
                }
            });
            McpProgressForwarding {
                sink,
                forwarder: Some(forwarder),
                stop: Some(sender),
            }
        } else {
            McpProgressForwarding {
                sink: Arc::new(NoopProgressSink),
                forwarder: None,
                stop: None,
            }
        };
        let McpProgressForwarding {
            sink: progress,
            forwarder: progress_forwarder,
            stop: progress_stop,
        } = progress_forwarding;
        let result = tokio::task::spawn_blocking(move || {
            // The actual blocking operation owns completion even if the SDK
            // drops its asynchronous handler while draining stdin EOF.
            let _manual_owner = manual_owner;
            let _standalone_admission = standalone_admission;
            let deadline = FrontendInvocationDeadline::new(received_at, None);
            execute_surface_tool(
                &router,
                SurfaceToolCall {
                    name: &handler_name,
                    arguments: &arguments,
                    host: &host,
                },
                cancellation,
                progress,
                deadline,
                client_supports_tasks,
            )
        })
        .await;
        if let Some(stop) = progress_stop {
            let _ = stop.send(None);
        }
        if let Some(forwarder) = progress_forwarder {
            let _ = forwarder.await;
        }
        if let Some(bridge) = bridge {
            bridge.abort();
        }

        let outcome = match result {
            Ok(Ok(SurfaceToolOutcome::Legacy(result))) => {
                render_tool_result(self.structured_tools.contains(name.as_str()), *result)
                    .map(CallToolResponse::from)
            }
            Ok(Ok(SurfaceToolOutcome::Canonical(result))) => {
                crate::interfaces::task_projection::call_tool_result(&result)
                    .map(CallToolResponse::from)
                    .map_err(crate::interfaces::task_projection::projection_error)
            }
            Ok(Ok(SurfaceToolOutcome::Direct(result))) => Ok(CallToolResponse::from(result)),
            Ok(Ok(SurfaceToolOutcome::Task(snapshot))) => {
                crate::interfaces::task_projection::create_task_result_v5(&snapshot)
                    .map(CallToolResponse::from)
                    .map_err(crate::interfaces::task_projection::projection_error)
            }
            Ok(Err(error)) => Err(error),
            Err(join_error) => Err(ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                format!("tool worker failed: {join_error}"),
                None,
            )),
        };
        outcome
    }

    async fn get_task(
        &self,
        request: GetTaskParams,
        context: RequestContext<RoleServer>,
    ) -> Result<GetTaskResult, ErrorData> {
        let received_at = Instant::now();
        ensure_native_task_protocol(&context)?;
        let task_id = parse_task_id(&request.task_id)?;
        let handler = canonical_task_router(&self.router)?.get;
        let deadline = FrontendInvocationDeadline::new(received_at, None);
        let snapshot = tokio::task::spawn_blocking(move || handler(task_id, deadline))
            .await
            .map_err(|_| task_internal_error("task_worker_failed"))?
            .map_err(project_task_exchange_error)?;
        ensure_task_identity(task_id, &snapshot)?;
        crate::interfaces::task_projection::detailed_task_v5(&snapshot)
            .map(GetTaskResult::new)
            .map_err(crate::interfaces::task_projection::projection_error)
    }

    async fn update_task(
        &self,
        request: UpdateTaskParams,
        context: RequestContext<RoleServer>,
    ) -> Result<(), ErrorData> {
        let received_at = Instant::now();
        ensure_native_task_protocol(&context)?;
        let task_id = parse_task_id(&request.task_id)?;
        let handler = canonical_task_router(&self.router)?.get;
        // v0.13 never enters input_required. Still prove the task is a current
        // daemon-owned identity before returning the stable unsupported-input
        // classification; unknown and expired identities retain their codes.
        let deadline = FrontendInvocationDeadline::new(received_at, None);
        let snapshot = tokio::task::spawn_blocking(move || handler(task_id, deadline))
            .await
            .map_err(|_| task_internal_error("task_worker_failed"))?
            .map_err(project_task_exchange_error)?;
        ensure_task_identity(task_id, &snapshot)?;
        Err(ErrorData::invalid_params(
            "task_input_not_supported",
            Some(serde_json::json!({"code": "task_input_not_supported"})),
        ))
    }

    async fn cancel_task(
        &self,
        request: CancelTaskParams,
        context: RequestContext<RoleServer>,
    ) -> Result<(), ErrorData> {
        let received_at = Instant::now();
        ensure_native_task_protocol(&context)?;
        let task_id = parse_task_id(&request.task_id)?;
        let handler = canonical_task_router(&self.router)?.cancel;
        let deadline = FrontendInvocationDeadline::new(received_at, None);
        let snapshot = tokio::task::spawn_blocking(move || handler(task_id, deadline))
            .await
            .map_err(|_| task_internal_error("task_worker_failed"))?
            .map_err(project_task_exchange_error)?;
        ensure_task_identity(task_id, &snapshot)?;
        Ok(())
    }
}

/// Bound on one `roots/list` exchange. A host answers from memory; the
/// remainder of the handoff window belongs to the daemon call.
const CLIENT_ROOTS_TIMEOUT: Duration = Duration::from_secs(2);

/// Roots of a client that declared them, asked for this call only.
///
/// Only a session established by a legacy-era initialize is asked: from
/// 2026-07-28 roots travel as an embedded input request, which this server
/// does not issue. The facade decides whether request metadata already
/// carries the workspace.
#[allow(deprecated)] // SEP-2577 retires roots only in the modern era.
async fn client_roots(
    context: &RequestContext<RoleServer>,
    metadata: &Map<String, Value>,
) -> unica_bootstrap::ClientRoots {
    use unica_bootstrap::ClientRoots;
    let declared = context
        .peer
        .peer_info()
        .is_some_and(|info| info.capabilities.roots.is_some());
    if !declared || modern_peer(context) || !unica_bootstrap::request_needs_client_roots(metadata) {
        return ClientRoots::NotDeclared;
    }
    let listed = tokio::time::timeout(CLIENT_ROOTS_TIMEOUT, context.peer.list_roots()).await;
    match listed {
        Ok(Ok(result)) => {
            ClientRoots::Listed(result.roots.into_iter().map(|root| root.uri).collect())
        }
        Ok(Err(error)) => ClientRoots::Unavailable(error.to_string()),
        Err(_) => ClientRoots::Unavailable("roots/list timed out".to_owned()),
    }
}

fn native_task_capability(context: &RequestContext<RoleServer>) -> bool {
    // Request metadata is allowed to shape one response, but it cannot replace
    // the protocol authority established by initialize. A direct-first request
    // has no peer_info and therefore carries its own complete authority.
    modern_protocol_authority(context)
        && context
            .client_capabilities()
            .is_some_and(|capabilities| capabilities.supports_tasks())
}

fn ensure_native_task_protocol(context: &RequestContext<RoleServer>) -> Result<(), ErrorData> {
    if native_task_capability(context) {
        Ok(())
    } else {
        Err(ErrorData::new(
            ErrorCode::METHOD_NOT_FOUND,
            "tasks_not_available_for_protocol",
            None,
        ))
    }
}

fn canonical_task_router(router: &SurfaceToolRouter) -> Result<CanonicalDaemonRouter, ErrorData> {
    match router {
        SurfaceToolRouter::CanonicalV13(router) => Ok(router.clone()),
        SurfaceToolRouter::LegacyV12(_) => Err(task_internal_error("task_profile_unavailable")),
    }
}

fn parse_task_id(encoded: &str) -> Result<crate::domain::invocation::TaskId, ErrorData> {
    encoded.parse().map_err(|_| {
        ErrorData::invalid_params(
            "invalid_task_id",
            Some(serde_json::json!({"code": "invalid_task_id"})),
        )
    })
}

fn ensure_task_identity(
    expected: crate::domain::invocation::TaskId,
    snapshot: &V5DaemonTaskSnapshot,
) -> Result<(), ErrorData> {
    if snapshot.task_id() == expected {
        Ok(())
    } else {
        Err(task_internal_error("task_protocol_failed"))
    }
}

/// Сводит код демона к уточнению, а не к одному имени: очередь и ёмкость
/// проходят с повтора, несовместимость требует человека, а сломанное
/// хранилище не лечится ни тем, ни другим. Широкая ветка `_` здесь и теряла
/// различие.
fn backend_detail(code: V5DaemonErrorCode) -> RefusalDetail {
    match code {
        V5DaemonErrorCode::Overloaded
        | V5DaemonErrorCode::OwnerCapacity
        | V5DaemonErrorCode::ReceiptCapacity
        | V5DaemonErrorCode::TombstoneCapacity => RefusalDetail::BackendBusy,
        V5DaemonErrorCode::ProtocolMismatch
        | V5DaemonErrorCode::CoreMismatch
        | V5DaemonErrorCode::Unauthorized
        | V5DaemonErrorCode::HandshakeRequired => RefusalDetail::BackendIncompatible,
        V5DaemonErrorCode::InvalidRequest
        | V5DaemonErrorCode::DuplicateLease
        | V5DaemonErrorCode::ReceiptNotFound
        | V5DaemonErrorCode::ReceiptExpired
        | V5DaemonErrorCode::InvocationIdentityMismatch
        | V5DaemonErrorCode::TaskNotFound
        | V5DaemonErrorCode::TaskExpired
        | V5DaemonErrorCode::StoreFailed
        | V5DaemonErrorCode::DurabilityUncertain
        | V5DaemonErrorCode::StoreCommitUncertain => RefusalDetail::BackendBroken,
    }
}

fn project_task_exchange_error(error: V5TaskExchangeError) -> ErrorData {
    match error {
        V5TaskExchangeError::Protocol(V5DaemonErrorCode::TaskNotFound) => {
            ErrorData::invalid_params(
                "task_not_found",
                Some(serde_json::json!({"code": "task_not_found"})),
            )
        }
        V5TaskExchangeError::Protocol(V5DaemonErrorCode::TaskExpired) => ErrorData::invalid_params(
            "task_expired",
            Some(serde_json::json!({"code": "task_expired"})),
        ),
        V5TaskExchangeError::Protocol(code) => task_internal_error_detailed(backend_detail(code)),
        V5TaskExchangeError::Transport => task_internal_error("task_transport_failed"),
        V5TaskExchangeError::SessionPoisoned => task_internal_error("task_session_closed"),
        V5TaskExchangeError::UnexpectedResponse => task_internal_error("task_protocol_failed"),
    }
}

fn task_internal_error(code: &'static str) -> ErrorData {
    ErrorData::new(
        ErrorCode::INTERNAL_ERROR,
        code,
        Some(serde_json::json!({"code": code})),
    )
}

/// То же, но с уточнением: исход берётся из него, а не из умолчания кода.
fn task_internal_error_detailed(detail: RefusalDetail) -> ErrorData {
    let code = detail.code().as_str();
    ErrorData::new(
        ErrorCode::INTERNAL_ERROR,
        code,
        Some(serde_json::json!({
            "code": code,
            "outcome": detail.outcome().as_str(),
            "detailCode": detail.as_str(),
        })),
    )
}

struct McpProgressForwarding {
    sink: Arc<dyn ProgressSink>,
    forwarder: Option<tokio::task::JoinHandle<()>>,
    stop: Option<tokio::sync::mpsc::UnboundedSender<Option<ProgressEvent>>>,
}

struct McpProgressSink {
    sender: tokio::sync::mpsc::UnboundedSender<Option<ProgressEvent>>,
}

impl ProgressSink for McpProgressSink {
    fn publish(&self, event: ProgressEvent) {
        let _ = self.sender.send(Some(event));
    }
}

/// Builds one `notifications/progress` payload. The meta key belongs to the
/// producing domain, so the transport copies it instead of naming one.
fn progress_notification(
    progress_token: ProgressToken,
    event: &ProgressEvent,
) -> ProgressNotificationParam {
    let mut meta = NotificationMetaObject::new();
    meta.0
        .insert(event.meta_key.to_string(), event.payload.clone());
    let mut notification = ProgressNotificationParam::new(progress_token, event.progress)
        .with_total(event.total)
        .with_message(event.message.clone());
    notification.meta = Some(meta);
    notification
}

/// Data-driven MCP tool definitions from the application descriptor registry.
pub fn tool_definitions(specs: &[ToolSpec]) -> Vec<Tool> {
    specs
        .iter()
        .map(|spec| {
            // #479 §1 schema-only baseline (owner decision, 2026-08-17): the
            // wire surface carries no prose while descriptions are reauthored;
            // the v0.12 history keeps the previous texts.
            let mut input_schema = input_schema_for_tool(spec);
            strip_schema_descriptions(&mut input_schema);
            let schema = match input_schema {
                Value::Object(schema) => schema,
                other => {
                    unreachable!("tool {} produced a non-object schema: {other}", spec.name)
                }
            };
            let mut tool = Tool::new(spec.name, spec.description, schema);
            tool.description = None;
            if let Some(mut schema) = structured_output_schema(spec) {
                strip_schema_descriptions(&mut schema);
                let output_schema = match schema {
                    Value::Object(schema) => schema,
                    other => unreachable!("OperationResult produced a non-object schema: {other}"),
                };
                tool.with_raw_output_schema(Arc::new(output_schema))
            } else {
                tool
            }
        })
        .collect()
}

fn render_tool_result(
    structured: bool,
    result: OperationResult,
) -> Result<CallToolResult, ErrorData> {
    let value = serde_json::to_value(&result)
        .map_err(|error| ErrorData::new(ErrorCode::INTERNAL_ERROR, error.to_string(), None))?;
    if structured {
        return Ok(if result.ok {
            CallToolResult::structured(value)
        } else {
            CallToolResult::structured_error(value)
        });
    }
    let text = serde_json::to_string_pretty(&value)
        .map_err(|error| ErrorData::new(ErrorCode::INTERNAL_ERROR, error.to_string(), None))?;
    let content = vec![ContentBlock::text(text)];
    Ok(if result.ok || !is_tool_execution_error(&result) {
        CallToolResult::success(content)
    } else {
        CallToolResult::error(content)
    })
}

fn is_tool_execution_error(result: &OperationResult) -> bool {
    result
        .errors
        .iter()
        .any(|error| error.starts_with("runtime_operation_unbounded:"))
}

#[cfg(test)]
fn call_tool_result(
    app: &UnicaApplication,
    name: &str,
    args: &Map<String, Value>,
    cancellation: CancellationToken,
) -> Result<OperationResult, (i32, String)> {
    call_tool_result_observed(app, name, args, cancellation, Arc::new(NoopProgressSink))
}

#[allow(dead_code)] // legacy surface test support; production dispatches through daemon
fn call_tool_result_observed(
    app: &UnicaApplication,
    name: &str,
    args: &Map<String, Value>,
    cancellation: CancellationToken,
    progress: Arc<dyn ProgressSink>,
) -> Result<OperationResult, (i32, String)> {
    if let Some(result) = role_edit_argument_failure_result(name, args) {
        return Ok(result);
    }
    if let Some(result) = metadata_argument_failure_result(name, args) {
        return Ok(result);
    }
    app.call_tool_observed(name, args, cancellation, progress)
        .map_err(|message| (TOOL_EXECUTION_ERROR, message))
}

#[cfg(test)]
fn call_tool_text(
    app: &UnicaApplication,
    name: &str,
    args: &Map<String, Value>,
    cancellation: CancellationToken,
) -> Result<String, (i32, String)> {
    let result = call_tool_result(app, name, args, cancellation)?;
    serde_json::to_string_pretty(&result)
        .map_err(|error| (ErrorCode::INTERNAL_ERROR.0, error.to_string()))
}

/// Retains each running call until its owner finishes, so shutdown can cancel
/// calls and wait without relying on SDK internals.
#[derive(Debug, Default)]
struct InFlightRegistry {
    state: Mutex<InFlightState>,
    changed: Condvar,
}

#[derive(Debug, Default)]
struct InFlightState {
    running: Vec<(u64, CancellationToken)>,
    next_id: u64,
}

impl InFlightRegistry {
    fn admit(self: &Arc<Self>) -> Result<InFlightGuard, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "in-flight registry lock poisoned".to_string())?;
        let id = state
            .next_id
            .checked_add(1)
            .ok_or_else(|| "in-flight call identifier overflow".to_string())?;
        state.next_id = id;
        let token = CancellationToken::new();
        state.running.push((id, token.clone()));
        Ok(InFlightGuard {
            registry: Arc::clone(self),
            id,
            token,
        })
    }

    #[cfg(test)]
    fn running(&self) -> usize {
        self.state
            .lock()
            .map(|state| state.running.len())
            .unwrap_or(0)
    }

    fn cancel_all(&self) {
        if let Ok(state) = self.state.lock() {
            for (_, token) in state.running.iter() {
                token.cancel();
            }
        }
    }

    fn wait_idle(&self, timeout: Duration) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        let deadline = Instant::now() + timeout;
        while !state.running.is_empty() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }
            let Ok((next, _)) = self.changed.wait_timeout(state, remaining) else {
                return false;
            };
            state = next;
        }
        true
    }

    fn release(&self, id: u64) {
        if let Ok(mut state) = self.state.lock() {
            state.running.retain(|(entry, _)| *entry != id);
        }
        self.changed.notify_all();
    }
}

#[derive(Debug)]
struct InFlightGuard {
    registry: Arc<InFlightRegistry>,
    id: u64,
    token: CancellationToken,
}

impl InFlightGuard {
    fn token(&self) -> CancellationToken {
        self.token.clone()
    }
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        self.registry.release(self.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::invocation::{
        INVOCATION_HANDOFF_WINDOW, RESPONSE_SERIALIZATION_MARGIN,
    };
    use crate::application::{ResultContract, ToolExecution};
    use crate::domain::cache::CacheReport;
    use crate::infrastructure::daemon::protocol_v5::V5ClientRequest;
    use crate::interfaces::daemon_router::test_support::{
        FakeDaemon, LiveDaemon, ScriptedService, Step,
    };
    use crate::interfaces::daemon_router::{remaining_invocation_budget, wait_transport_cutoff};
    use serde_json::json;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::mpsc;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::time::timeout;

    const TEST_STEP: Duration = Duration::from_secs(10);

    #[test]
    fn unica_server_implements_official_rmcp_server_handler() {
        super::assert_unica_server_implements_official_rmcp_server_handler();
    }

    #[test]
    fn production_mcp_surface_exposes_only_canonical_v13_tools_and_task_compatibility() {
        let canonical: Arc<CanonicalCallHandler> = Arc::new(|_, _, _, _, _| {
            direct_outcome(crate::domain::invocation::DomainResult::success(
                "canonical",
            ))
        });
        let server = UnicaServer::with_canonical_v13(canonical);

        assert!(
            matches!(server.router, SurfaceToolRouter::CanonicalV13(_)),
            "the production MCP constructor must select the canonical v0.13 router"
        );

        let native = v13_tool_definitions(V13TaskProfile::Native)
            .iter()
            .map(|tool| tool.name.as_ref())
            .collect::<Vec<_>>();
        assert_eq!(
            native,
            [
                "unica.view",
                "unica.apply",
                "unica.resolve",
                "unica.search",
                "unica.check",
                "unica.diff",
                "unica.run",
                "unica.docs",
            ],
            "the native Tasks-capable profile must expose exactly the eight canonical tools"
        );

        let compatibility = v13_tool_definitions(V13TaskProfile::Compatibility)
            .iter()
            .map(|tool| tool.name.as_ref())
            .collect::<Vec<_>>();
        assert_eq!(
            compatibility,
            [
                "unica.view",
                "unica.apply",
                "unica.resolve",
                "unica.search",
                "unica.check",
                "unica.diff",
                "unica.run",
                "unica.docs",
                "unica.task.get",
                "unica.task.result",
                "unica.task.cancel",
            ],
            "the compatibility profile must add only the three task projection tools"
        );
    }

    #[test]
    fn canonical_tools_are_described_within_wire_budget() {
        let tools = v13_tool_definitions(V13TaskProfile::Compatibility);
        for tool in tools {
            let description = tool.description.as_deref().unwrap_or_default();
            assert!(
                !description.trim().is_empty(),
                "{} has no model-facing description",
                tool.name
            );
            assert!(
                description.len() <= 2 * 1024,
                "{} description exceeds the 2 KiB client limit",
                tool.name
            );
            let arguments = tool.input_schema["properties"]
                .as_object()
                .expect("tool input declares its arguments");
            // Conditional constraints such as `if.properties` refine an argument;
            // descriptions belong to its declaration and nested argument objects.
            for properties in
                std::iter::once(arguments).chain(object_schema_property_maps(arguments))
            {
                for (name, property) in properties {
                    let description = property
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    assert!(
                        !description.trim().is_empty(),
                        "{} argument `{name}` has no model-facing description",
                        tool.name
                    );
                }
            }
        }
        let wire = serde_json::to_vec(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {"tools": tools},
        }))
        .expect("tools/list response serializes");
        assert!(
            wire.len() <= 16 * 1024,
            "compatibility tools/list response is {} bytes",
            wire.len()
        );
    }

    #[test]
    fn surface_release_structurally_gates_v12_legacy_dispatch_from_v13_daemon_dispatch() {
        use std::sync::atomic::AtomicUsize;

        let legacy_count = Arc::new(AtomicUsize::new(0));
        let legacy_observed = Arc::clone(&legacy_count);
        let legacy: Arc<ToolCallHandler> = Arc::new(move |_, _, _, _| {
            legacy_observed.fetch_add(1, Ordering::SeqCst);
            Ok(successful_test_result("legacy"))
        });
        let v12 = UnicaServer::legacy_for_test(legacy);
        let received = Instant::now();
        let deadline = FrontendInvocationDeadline::new(received, None);
        let result = execute_surface_tool(
            &v12.router,
            SurfaceToolCall {
                name: "unica.check",
                arguments: &Map::new(),
                host: &unica_bootstrap::HostRequest::default(),
            },
            CancellationToken::new(),
            Arc::new(NoopProgressSink),
            deadline,
            false,
        )
        .unwrap();
        let SurfaceToolOutcome::Legacy(result) = result else {
            panic!("v0.12 must retain the legacy result envelope");
        };
        assert_eq!(result.summary, "legacy");
        assert_eq!(legacy_count.load(Ordering::SeqCst), 1);

        let daemon_count = Arc::new(AtomicUsize::new(0));
        let daemon_observed = Arc::clone(&daemon_count);
        let canonical: Arc<CanonicalCallHandler> = Arc::new(move |tool, _, _, deadline, _| {
            assert_eq!(tool, V5ToolIdentity::Check);
            assert_eq!(deadline.remaining_at(received), Duration::from_secs(7));
            daemon_observed.fetch_add(1, Ordering::SeqCst);
            direct_outcome(crate::domain::invocation::DomainResult::success(
                "canonical",
            ))
        });
        let v13 = UnicaServer::with_canonical_v13(canonical);
        let result = execute_surface_tool(
            &v13.router,
            SurfaceToolCall {
                name: "unica.check",
                arguments: &Map::new(),
                host: &unica_bootstrap::HostRequest::default(),
            },
            CancellationToken::new(),
            Arc::new(NoopProgressSink),
            deadline,
            true,
        )
        .unwrap();
        let SurfaceToolOutcome::Direct(result) = result else {
            panic!("v0.13 direct calls must arrive as the acknowledged final result");
        };
        assert_eq!(
            result
                .structured_content
                .as_ref()
                .and_then(|value| value["summary"].as_str()),
            Some("canonical")
        );
        assert_eq!(daemon_count.load(Ordering::SeqCst), 1);
        assert_eq!(legacy_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn frontend_receipt_deadline_transmits_zero_or_earlier_host_budget_without_reexecution() {
        let received = Instant::now();
        let deadline = FrontendInvocationDeadline::new(received, None);
        assert_eq!(
            deadline.remaining_at(received + Duration::from_secs(7)),
            Duration::ZERO,
            "queueing before daemon submission must not replenish the frontend budget",
        );
        assert_eq!(
            deadline.remaining_transport_at(received + Duration::from_secs(7)),
            Duration::from_millis(125),
            "the bounded serialization margin covers connection and submit together",
        );
        assert_eq!(
            remaining_invocation_budget(received, received, None),
            Duration::from_secs(7)
        );
        assert_eq!(
            remaining_invocation_budget(received, received + Duration::from_secs(7), None,),
            Duration::ZERO
        );
        assert_eq!(
            remaining_invocation_budget(
                received,
                received + Duration::from_millis(250),
                Some(Duration::from_secs(2)),
            ),
            Duration::from_millis(1_625),
            "host budget reserves the 125 ms response margin after elapsed frontend time",
        );
    }

    fn object_schema_property_maps(
        schema: &serde_json::Map<String, serde_json::Value>,
    ) -> Vec<&serde_json::Map<String, serde_json::Value>> {
        fn visit_value<'a>(
            value: &'a serde_json::Value,
            property_maps: &mut Vec<&'a serde_json::Map<String, serde_json::Value>>,
        ) {
            match value {
                serde_json::Value::Object(object) => visit_object(object, property_maps),
                serde_json::Value::Array(items) => {
                    for item in items {
                        visit_value(item, property_maps);
                    }
                }
                _ => {}
            }
        }

        fn visit_object<'a>(
            object: &'a serde_json::Map<String, serde_json::Value>,
            property_maps: &mut Vec<&'a serde_json::Map<String, serde_json::Value>>,
        ) {
            if let Some(properties) = object
                .get("properties")
                .and_then(serde_json::Value::as_object)
            {
                property_maps.push(properties);
            }
            for value in object.values() {
                visit_value(value, property_maps);
            }
        }

        let mut property_maps = Vec::new();
        visit_object(schema, &mut property_maps);
        property_maps
    }

    fn successful_test_result(summary: &str) -> OperationResult {
        OperationResult {
            ok: true,
            summary: summary.to_string(),
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: Vec::new(),
            artifacts: Vec::new(),
            cache: CacheReport {
                mode: "read".to_string(),
                root: String::new(),
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
            diagnostics: None,
            data: None,
            job: None,
            work: None,
        }
    }

    fn code_search_test_result() -> OperationResult {
        let mut result = successful_test_result("search complete");
        result.data = Some(json!({
            "coverage": "partial",
            "elapsedMs": 12,
            "sections": [
                {
                    "role": "semantic",
                    "provider": "rlm",
                    "status": "unavailable",
                    "termination": {"code": "providerUnavailable", "retryable": false},
                    "searchComplete": false,
                    "ranking": "none",
                    "ordering": "provider",
                    "matches": {"returned": 0, "relation": "unknown"},
                    "hits": [],
                    "diagnostics": ["index unavailable"]
                },
                {
                    "role": "symbol",
                    "provider": "bsl-analyzer",
                    "status": "empty",
                    "termination": null,
                    "searchComplete": true,
                    "ranking": "provider",
                    "ordering": "provider",
                    "matches": {"returned": 0, "total": 0, "relation": "exact"},
                    "hits": [],
                    "diagnostics": []
                },
                {
                    "role": "lexical",
                    "provider": "git-grep",
                    "status": "limitReached",
                    "termination": {"code": "limitReached", "retryable": false},
                    "searchComplete": false,
                    "ranking": "none",
                    "ordering": "providerTraversal",
                    "matches": {"returned": 1, "total": 1, "relation": "lowerBound"},
                    "hits": [{
                        "location": {
                            "kind": "unaddressable",
                            "sourceSet": "main",
                            "path": "CommonModules/Smoke/Ext/Module.bsl"
                        },
                        "line": 3,
                        "endLine": null,
                        "symbol": null,
                        "kind": "text",
                        "snippet": "Needle",
                        "attributes": {}
                    }],
                    "diagnostics": []
                }
            ]
        }));
        result
    }

    struct McpClient {
        writer: tokio::io::WriteHalf<tokio::io::DuplexStream>,
        reader: tokio::io::Lines<BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>>,
        server: tokio::task::JoinHandle<()>,
    }

    impl McpClient {
        async fn send(&mut self, message: Value) {
            let mut line = message.to_string();
            line.push('\n');
            self.writer.write_all(line.as_bytes()).await.unwrap();
            self.writer.flush().await.unwrap();
        }

        async fn receive(&mut self) -> Value {
            let line = self.receive_raw().await;
            serde_json::from_str(&line).expect("MCP server emitted invalid JSON")
        }

        async fn receive_raw(&mut self) -> String {
            let line = timeout(TEST_STEP, self.reader.next_line())
                .await
                .expect("timed out waiting for MCP response")
                .expect("MCP transport failed")
                .expect("MCP server closed the stream before responding");
            line
        }

        async fn initialize(&mut self) -> Value {
            self.send(json!({
                "jsonrpc": "2.0",
                "id": 0,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "clientInfo": {"name": "unica-tests", "version": "1"}
                }
            }))
            .await;
            let response = self.receive().await;
            assert_eq!(response["id"], 0);
            self.send(json!({
                "jsonrpc": "2.0",
                "method": "notifications/initialized"
            }))
            .await;
            response
        }

        async fn shutdown(mut self) {
            // Dropping a WriteHalf does not close the duplex; shut it down so
            // the server observes EOF.
            self.writer.shutdown().await.unwrap();
            drop(self.writer);
            while timeout(TEST_STEP, self.reader.next_line())
                .await
                .expect("timed out waiting for MCP stdout EOF")
                .expect("MCP transport failed")
                .is_some()
            {}
            timeout(TEST_STEP, self.server)
                .await
                .expect("timed out waiting for the MCP server to stop")
                .unwrap();
        }
    }

    fn spawn_server(handler: Arc<ToolCallHandler>) -> (McpClient, Arc<InFlightRegistry>) {
        spawn_unica_server(UnicaServer::legacy_for_test(handler))
    }

    fn spawn_unica_server(server: UnicaServer) -> (McpClient, Arc<InFlightRegistry>) {
        let (client_io, server_io) = tokio::io::duplex(4 * 1024 * 1024);
        let in_flight = server.in_flight();
        let server = tokio::spawn(async move {
            let (read, write) = tokio::io::split(server_io);
            let transport = rmcp::transport::async_rw::AsyncRwTransport::new_server(read, write);
            let transport = DiscoveryProbeTransport::new(transport, &server);
            match ObservedServer(server).serve(transport).await {
                Ok(running) => {
                    let _ = running.waiting().await;
                }
                Err(error)
                    if matches!(error.as_ref(), ServerInitializeError::ConnectionClosed(_)) => {}
                Err(error) => panic!("test MCP server failed to initialize: {error}"),
            }
        });
        let (read_half, writer) = tokio::io::split(client_io);
        let reader = BufReader::new(read_half).lines();
        (
            McpClient {
                writer,
                reader,
                server,
            },
            in_flight,
        )
    }

    fn spawn_unwrapped_unica_server(server: UnicaServer) -> McpClient {
        let (client_io, server_io) = tokio::io::duplex(4 * 1024 * 1024);
        let server = tokio::spawn(async move {
            match server.serve(server_io).await {
                Ok(running) => {
                    let _ = running.waiting().await;
                }
                Err(ServerInitializeError::ConnectionClosed(_)) => {}
                Err(error) => panic!("test MCP server failed to initialize: {error}"),
            }
        });
        let (read_half, writer) = tokio::io::split(client_io);
        McpClient {
            writer,
            reader: BufReader::new(read_half).lines(),
            server,
        }
    }

    fn application_handler() -> Arc<ToolCallHandler> {
        let app = Arc::new(UnicaApplication::new());
        Arc::new(move |name, arguments, cancellation, progress| {
            call_tool_result_observed(&app, name, arguments, cancellation, progress)
        })
    }

    #[test]
    fn initialize_carries_what_a_killed_startup_left_behind() {
        // Убитая установка своего провода не имела: её рассказ приходит сюда
        // от загрузчика и уходит вызывающему обычным ответом на `initialize`.
        let notice = "a Unica startup was killed while downloading unica 0.13.0";
        let server = UnicaServer::legacy_with_startup_notice_for_test(
            application_handler(),
            Some(notice.to_owned()),
        );

        let instructions = server.get_info().instructions.expect("instructions");
        assert!(instructions.contains("unica.view"), "{instructions}");
        assert!(instructions.contains("sourceSet"), "{instructions}");
        assert!(instructions.contains(notice), "{instructions}");
    }

    #[test]
    fn a_session_without_notice_still_carries_bootstrap_instructions() {
        let server = UnicaServer::legacy_with_startup_notice_for_test(application_handler(), None);

        let instructions = server.get_info().instructions.expect("instructions");
        assert!(instructions.contains("unica.view"), "{instructions}");
        assert!(instructions.contains("sourceSet"), "{instructions}");
    }

    #[test]
    fn an_empty_notice_is_the_same_as_no_notice() {
        // Переменная, которую хост передал пустой, — это «нечего рассказывать»,
        // а не пустой рассказ.
        assert_eq!(startup_notice_from(Some(String::new())), None);
        assert_eq!(startup_notice_from(Some("   \n".to_owned())), None);
        assert_eq!(
            startup_notice_from(Some("  killed while downloading  ".to_owned())),
            Some("killed while downloading".to_owned())
        );
        assert_eq!(startup_notice_from(None), None);
    }

    #[tokio::test]
    async fn initialize_uses_single_public_server_name_and_negotiates_version() {
        let (mut client, _) = spawn_server(application_handler());
        let response = client.initialize().await;
        assert_eq!(response["result"]["serverInfo"]["name"], "unica");
        assert_eq!(
            response["result"]["serverInfo"]["version"],
            env!("CARGO_PKG_VERSION")
        );
        assert_eq!(
            response["result"]["protocolVersion"], "2025-06-18",
            "the SDK must negotiate the client protocol version instead of pinning one"
        );
        client.shutdown().await;
    }

    #[tokio::test]
    async fn applied_runtime_answers_once_without_input_disclosure() {
        const CWD_SENTINEL: &str = "/missing/unica-issue-406-private-workspace";
        const CONNECTION_SENTINEL: &str = "File=/private/issue-406-sensitive.ib";
        let (mut client, _) = spawn_server(application_handler());
        client.initialize().await;
        client
            .send(json!({
                "jsonrpc": "2.0",
                "id": "runtime-refusal",
                "method": "tools/call",
                "params": {
                    "name": "unica.runtime.execute",
                    "arguments": {
                        "cwd": CWD_SENTINEL,
                        "dryRun": false,
                        "operation": "config-init",
                        "config": "v8project.yaml",
                        "connection": CONNECTION_SENTINEL
                    }
                }
            }))
            .await;

        let response = client.receive().await;
        assert_eq!(response["id"], "runtime-refusal", "{response}");
        // The applied call is no longer refused before discovery, so
        // this fixture answers with the missing bundled runner instead. What the
        // test still pins is the shape: one terminal answer, no input echoed.
        let serialized = response.to_string();
        assert!(
            !serialized.contains("runtime_operation_unbounded"),
            "the applied refusal is retired: {response}"
        );
        assert!(!serialized.contains(CWD_SENTINEL), "{response}");
        assert!(!serialized.contains(CONNECTION_SENTINEL), "{response}");
        assert!(
            timeout(Duration::from_millis(50), client.reader.next_line())
                .await
                .is_err(),
            "one tools/call must produce exactly one terminal response"
        );
        client.shutdown().await;
    }

    #[test]
    fn application_registry_owns_tool_names_descriptions_and_wire_schemas() {
        let specs = crate::application::tools();
        let listed = tool_definitions(&specs);

        assert_eq!(listed.len(), specs.len());
        let unique_names: HashSet<&str> = specs.iter().map(|spec| spec.name).collect();
        assert_eq!(
            unique_names.len(),
            specs.len(),
            "ToolSpec names must be unique"
        );

        for (spec, tool) in specs.iter().zip(&listed) {
            assert_eq!(tool.name, spec.name);
            assert!(
                !spec.description.trim().is_empty(),
                "{} must retain its application-owned description",
                spec.name
            );
            assert_eq!(
                tool.description, None,
                "{} must keep application prose off the schema-only wire",
                spec.name
            );

            let mut expected_input = input_schema_for_tool(spec);
            strip_schema_descriptions(&mut expected_input);
            assert_eq!(
                Value::Object(tool.input_schema.as_ref().clone()),
                expected_input,
                "{} input schema must be projected from the application contract",
                spec.name
            );

            let mut expected_output = structured_output_schema(spec);
            if let Some(schema) = &mut expected_output {
                strip_schema_descriptions(schema);
            }
            let actual_output = tool
                .output_schema
                .as_ref()
                .map(|schema| Value::Object(schema.as_ref().clone()));
            assert_eq!(
                actual_output, expected_output,
                "{} output schema must follow its application handler contract",
                spec.name
            );
        }
    }

    #[tokio::test]
    async fn tools_list_serves_schema_only_baseline() {
        // #479 §1 baseline experiment: the wire carries no prose. Stripping an
        // already served schema must be an identity, and no tool publishes a
        // description.
        let (mut client, _) = spawn_server(application_handler());
        client.initialize().await;
        client
            .send(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {} }))
            .await;
        let response = client.receive().await;
        let listed = response["result"]["tools"].as_array().unwrap();
        assert!(!listed.is_empty());
        for tool in listed {
            assert!(
                tool.get("description").is_none(),
                "tool {} still publishes a description",
                tool["name"]
            );
            for key in ["inputSchema", "outputSchema"] {
                if let Some(schema) = tool.get(key) {
                    let mut stripped = schema.clone();
                    crate::application::strip_schema_descriptions(&mut stripped);
                    assert_eq!(
                        &stripped, schema,
                        "tool {} still carries description annotations in {key}",
                        tool["name"]
                    );
                }
            }
        }
        client.shutdown().await;
    }

    #[tokio::test]
    async fn registry_keeps_runtime_execute_preview_guidance() {
        // The preview-only guidance survives in the descriptor registry while
        // the wire stays schema-only; reauthoring replaces it deliberately.
        let spec = crate::application::tools()
            .into_iter()
            .find(|spec| spec.name == "unica.runtime.execute")
            .expect("runtime tool is registered");
        assert_eq!(
            spec.description,
            "Preview typed v8-runner workflows, or run a classified applied operation and answer with its terminal result plus a named risk warning; an unclassified operation still fails closed before workspace discovery or process spawn."
        );
        let schema = input_schema_for_tool(&spec);
        assert_eq!(
            schema["properties"]["dryRun"]["description"],
            "Preview typed v8-runner runtime arguments; omitted or true reports the planned command without mutation, while false runs a classified operation and returns its terminal result in this call with a named risk warning; an unclassified operation stays refused."
        );
    }

    // #490 wire matrix: the guaranteed versions are 2025-06-18, 2025-11-25
    // (legacy `initialize` sessions) and 2026-07-28 (direct-first + discover).
    // Version handling itself belongs to the SDK; these tests pin the served
    // contract, not host behavior.

    #[tokio::test]
    async fn initialize_declares_only_the_implemented_surface() {
        // Undeclared surfaces are a deliberate choice: each feature
        // (prompts, resources, logging, completions, tasks, ui) re-enters the
        // declaration together with its implementation slice, so agents never
        // see an advertised-but-empty capability.
        let (mut client, _) = spawn_server(application_handler());
        client
            .send(json!({
                "jsonrpc": "2.0",
                "id": 0,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-11-25",
                    "capabilities": {},
                    "clientInfo": {"name": "unica-tests", "version": "1"}
                }
            }))
            .await;
        let response = client.receive().await;
        assert_eq!(
            response["result"]["capabilities"],
            json!({"tools": {}}),
            "capabilities must stay exactly the implemented surface"
        );
        client.shutdown().await;
    }

    #[test]
    fn progress_notification_carries_the_producing_domain_meta_key() {
        let event = ProgressEvent {
            meta_key: "io.unica/runtimeProgress",
            payload: serde_json::json!({"phase": "running"}),
            progress: 1.0,
            total: 3.0,
            message: "running".to_string(),
        };

        let notification = progress_notification(
            ProgressToken(rmcp::model::NumberOrString::String("t".into())),
            &event,
        );

        let meta = notification
            .meta
            .expect("a progress notification carries its payload in meta");
        assert_eq!(meta.0["io.unica/runtimeProgress"]["phase"], "running");
    }

    #[tokio::test]
    async fn modern_list_results_carry_required_cache_fields_and_legacy_stays_clean() {
        // 2026-07-28 wire schemas (SEP-2549) require ttlMs/cacheScope on list
        // results; the legacy shape must stay byte-identical to pre-2026.
        let (mut client, _) = spawn_server(application_handler());
        client
            .send(json!({
                "jsonrpc": "2.0",
                "id": 0,
                "method": "tools/list",
                "params": { "_meta": modern_meta() }
            }))
            .await;
        let modern = client.receive().await;
        assert_eq!(modern["result"]["ttlMs"], 0);
        assert_eq!(modern["result"]["cacheScope"], "private");
        client.shutdown().await;

        let (mut client, _) = spawn_server(application_handler());
        client
            .send(json!({
                "jsonrpc": "2.0",
                "id": 0,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-11-25",
                    "capabilities": {},
                    "clientInfo": {"name": "unica-tests", "version": "1"}
                }
            }))
            .await;
        client.receive().await;
        client
            .send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
            .await;
        client
            .send(json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}))
            .await;
        let legacy = client.receive().await;
        assert!(legacy["result"].get("ttlMs").is_none(), "got {legacy}");
        assert!(legacy["result"].get("cacheScope").is_none(), "got {legacy}");
        client.shutdown().await;
    }

    #[tokio::test]
    async fn modern_subscriptions_listen_is_acknowledged_not_rejected() {
        // The Inspector auto-opens `subscriptions/listen` whenever listChanged
        // capabilities are advertised; the default SDK filter (None) turned
        // every attempt into -32601 and an endless client retry loop.
        let (mut client, _) = spawn_server(application_handler());
        client
            .send(json!({
                "jsonrpc": "2.0",
                "id": 0,
                "method": "tools/list",
                "params": { "_meta": modern_meta() }
            }))
            .await;
        client.receive().await;
        client
            .send(json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "subscriptions/listen",
                "params": {
                    "_meta": modern_meta(),
                    "notifications": {
                        "toolsListChanged": true,
                        "promptsListChanged": true,
                        "resourcesListChanged": true
                    }
                }
            }))
            .await;
        let reply = client.receive().await;
        assert_eq!(
            reply["method"], "notifications/subscriptions/acknowledged",
            "expected the acknowledgment notification, got {reply}"
        );
        // With no listChanged capability advertised, the SDK intersects the
        // accepted set down to nothing — a clean no-op stream, not an error.
        assert_eq!(
            reply["params"]["notifications"],
            json!({}),
            "nothing is advertised, so nothing may be accepted"
        );
        client.shutdown().await;
    }

    #[tokio::test]
    async fn undeclared_surfaces_answer_cleanly_when_probed_anyway() {
        // These surfaces are not advertised; a client probing them anyway
        // gets valid empty lists (SDK defaults plus our handlers), while
        // logging stays method_not_found — nothing pretends to exist.
        let (mut client, _) = spawn_server(application_handler());
        client
            .send(json!({
                "jsonrpc": "2.0",
                "id": 0,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-11-25",
                    "capabilities": {},
                    "clientInfo": {"name": "unica-tests", "version": "1"}
                }
            }))
            .await;
        client.receive().await;
        client
            .send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
            .await;

        client
            .send(json!({"jsonrpc": "2.0", "id": 1, "method": "prompts/list"}))
            .await;
        let prompts = client.receive().await;
        assert_eq!(prompts["result"]["prompts"], json!([]));

        client
            .send(json!({"jsonrpc": "2.0", "id": 2, "method": "resources/list"}))
            .await;
        let resources = client.receive().await;
        assert_eq!(resources["result"]["resources"], json!([]));

        client
            .send(json!({"jsonrpc": "2.0", "id": 3, "method": "resources/templates/list"}))
            .await;
        let templates = client.receive().await;
        assert_eq!(templates["result"]["resourceTemplates"], json!([]));

        client
            .send(json!({
                "jsonrpc": "2.0",
                "id": 4,
                "method": "logging/setLevel",
                "params": {"level": "debug"}
            }))
            .await;
        let level = client.receive().await;
        assert_eq!(level["error"]["code"], -32601, "got {level}");

        client.shutdown().await;
    }

    fn modern_meta() -> Value {
        json!({
            "io.modelcontextprotocol/protocolVersion": "2026-07-28",
            "io.modelcontextprotocol/clientCapabilities": {}
        })
    }

    fn modern_tasks_meta() -> Value {
        json!({
            "io.modelcontextprotocol/protocolVersion": "2026-07-28",
            "io.modelcontextprotocol/clientCapabilities": {
                "extensions": {"io.modelcontextprotocol/tasks": {}}
            }
        })
    }

    fn canonical_result(summary: &str) -> crate::domain::invocation::DomainResult {
        crate::domain::invocation::DomainResult {
            ok: false,
            at: Some("main:Catalog.Товары".into()),
            summary: summary.into(),
            data: Some(json!({"nested": [1, 2, 3]})),
            changed: vec![json!({"at": "main:Catalog.Товары.Attribute.Код"})],
            warnings: vec![json!({"code": "warning"})],
            diagnostics: vec![json!({"code": "bad_value"})],
            artifacts: vec![json!({"kind": "report"})],
            next: vec![json!({"op": "view"})],
            rev: Some("rev-7".into()),
            cursor: Some("cursor-2".into()),
            page: None,
        }
    }

    /// A durable v5 snapshot for the handler fakes: the same identity and
    /// timing every test expects on the wire, one closed variant per status.
    fn canonical_snapshot(
        task_id: crate::domain::invocation::TaskId,
        status: crate::domain::invocation::InvocationStatus,
        result: Option<crate::domain::invocation::DomainResult>,
    ) -> V5DaemonTaskSnapshot {
        canonical_snapshot_at(
            task_id,
            status,
            result,
            1_777_012_345_678,
            1_777_012_346_789,
        )
    }

    fn canonical_snapshot_at(
        task_id: crate::domain::invocation::TaskId,
        status: crate::domain::invocation::InvocationStatus,
        result: Option<crate::domain::invocation::DomainResult>,
        created_at_epoch_ms: u64,
        updated_at_epoch_ms: u64,
    ) -> V5DaemonTaskSnapshot {
        use crate::domain::invocation::InvocationStatus;

        let invocation_id = crate::domain::invocation::InvocationId::new();
        let receipt_key_digest: crate::application::receipt_ledger::ReceiptKeyDigest =
            "07".repeat(32).parse().unwrap();
        let terminal_digest: crate::application::receipt_ledger::TerminalDigest =
            "09".repeat(32).parse().unwrap();
        let (ttl_ms, poll_interval_ms, version, cancel_requested) = (3_600_000, 250, 2, false);
        match status {
            InvocationStatus::Queued => V5DaemonTaskSnapshot::Queued {
                task_id,
                invocation_id,
                receipt_key_digest,
                created_at_epoch_ms,
                updated_at_epoch_ms,
                ttl_ms,
                poll_interval_ms,
                version,
                cancel_requested,
            },
            InvocationStatus::Working => V5DaemonTaskSnapshot::Working {
                task_id,
                invocation_id,
                receipt_key_digest,
                created_at_epoch_ms,
                updated_at_epoch_ms,
                ttl_ms,
                poll_interval_ms,
                version,
                cancel_requested,
            },
            InvocationStatus::Completed => V5DaemonTaskSnapshot::Completed {
                task_id,
                invocation_id,
                receipt_key_digest,
                created_at_epoch_ms,
                updated_at_epoch_ms,
                ttl_ms,
                poll_interval_ms,
                version,
                cancel_requested,
                terminal_epoch_ms: updated_at_epoch_ms,
                terminal_digest,
                result: Box::new(result.expect("a completed snapshot carries its result")),
            },
            InvocationStatus::Failed => V5DaemonTaskSnapshot::Failed {
                task_id,
                invocation_id,
                receipt_key_digest,
                created_at_epoch_ms,
                updated_at_epoch_ms,
                ttl_ms,
                poll_interval_ms,
                version,
                cancel_requested,
                terminal_epoch_ms: updated_at_epoch_ms,
                terminal_digest,
                reason:
                    crate::application::invocation_store_v5::V5SafeFailureReason::InvocationFailed,
            },
            InvocationStatus::Cancelled => V5DaemonTaskSnapshot::Cancelled {
                task_id,
                invocation_id,
                receipt_key_digest,
                created_at_epoch_ms,
                updated_at_epoch_ms,
                ttl_ms,
                poll_interval_ms,
                version,
                cancel_requested: true,
                terminal_epoch_ms: updated_at_epoch_ms,
                terminal_digest,
            },
        }
    }

    /// The handler fakes answer a Direct terminal the way the router does: as
    /// the final projected `CallToolResult`, or the projection's refusal.
    fn direct_outcome(
        result: crate::domain::invocation::DomainResult,
    ) -> Result<CanonicalCallOutcome, ErrorData> {
        crate::interfaces::task_projection::call_tool_result(&result)
            .map(CanonicalCallOutcome::Direct)
            .map_err(crate::interfaces::task_projection::projection_error)
    }

    fn canonical_profile_server() -> UnicaServer {
        let task_id = crate::domain::invocation::TaskId::new();
        let call: Arc<CanonicalCallHandler> = Arc::new(move |_, _, _, _, _| {
            Ok(CanonicalCallOutcome::Task(canonical_snapshot(
                task_id,
                crate::domain::invocation::InvocationStatus::Working,
                None,
            )))
        });
        let get: Arc<CanonicalTaskHandler> = Arc::new(move |_, _| {
            Ok(canonical_snapshot(
                task_id,
                crate::domain::invocation::InvocationStatus::Working,
                None,
            ))
        });
        UnicaServer::with_canonical_v13_tasks(call, Arc::clone(&get), get)
    }

    async fn listed_tool_names(
        client: &mut McpClient,
        id: u64,
        meta: Option<Value>,
    ) -> Vec<String> {
        let mut params = json!({});
        if let Some(meta) = meta {
            params["_meta"] = meta;
        }
        client
            .send(json!({"jsonrpc":"2.0", "id":id, "method":"tools/list", "params":params}))
            .await;
        let response = client.receive().await;
        assert!(response.get("error").is_none(), "{response}");
        let tools = response["result"]["tools"]
            .as_array()
            .expect("tools/list must return tools");
        for tool in tools {
            let schema = &tool["inputSchema"];
            assert_eq!(schema["type"], "object", "{}", tool["name"]);
            for keyword in ["oneOf", "anyOf", "allOf"] {
                assert!(
                    schema.get(keyword).is_none(),
                    "{} uses unsupported root-level {keyword} in its input schema",
                    tool["name"]
                );
            }
            if tool["name"] == "unica.apply" {
                let validator = jsonschema::validator_for(schema).expect("valid apply wire schema");
                let plan = json!({"at":"main:Document.Order", "ops":[{"op":"props.set"}]});
                assert!(validator.is_valid(&plan));
                assert!(validator.is_valid(&json!({"executionToken":"saved-plan"})));
                let mut mixed = plan.clone();
                mixed["executionToken"] = Value::Null;
                assert!(!validator.is_valid(&mixed));
                mixed["executionToken"] = json!("saved-plan");
                assert!(!validator.is_valid(&mixed));
            }
        }
        tools
            .iter()
            .map(|tool| tool["name"].as_str().expect("tool name").to_string())
            .collect()
    }

    fn assert_v13_profile_names(names: &[String], native_tasks: bool) {
        let mut expected = vec![
            "unica.view",
            "unica.apply",
            "unica.resolve",
            "unica.search",
            "unica.check",
            "unica.diff",
            "unica.run",
            "unica.docs",
        ];
        if !native_tasks {
            expected.extend(["unica.task.get", "unica.task.result", "unica.task.cancel"]);
        }
        assert_eq!(names, expected, "wrong canonical v0.13 tools/list profile");
        for forbidden in [
            "unica.task.list",
            "unica.task.logs",
            "unica.runtime.job.start",
            "unica.runtime.job.status",
            "unica.runtime.job.wait",
            "unica.runtime.job.logs",
            "unica.runtime.job.list",
            "unica.runtime.job.cancel",
        ] {
            assert!(
                !names.iter().any(|name| name == forbidden),
                "leaked {forbidden}"
            );
        }
    }

    async fn surface_profiles_case() {
        // A legacy initialized session stays on the compatibility profile even
        // when one request carries modern Tasks metadata.
        let (mut legacy, _) = spawn_unica_server(canonical_profile_server());
        legacy
            .send(json!({
                "jsonrpc":"2.0", "id":0, "method":"initialize",
                "params":{
                    "protocolVersion":"2025-11-25",
                    "capabilities":{},
                    "clientInfo":{"name":"legacy-profile","version":"1"}
                }
            }))
            .await;
        assert_eq!(
            legacy.receive().await["result"]["protocolVersion"],
            "2025-11-25"
        );
        assert_v13_profile_names(&listed_tool_names(&mut legacy, 1, None).await, false);
        assert_v13_profile_names(
            &listed_tool_names(&mut legacy, 2, Some(modern_tasks_meta())).await,
            false,
        );
        legacy.shutdown().await;

        // A legitimately negotiated modern session selects from its own
        // capabilities and never from another client's previous list.
        let (mut modern_native, _) = spawn_unica_server(canonical_profile_server());
        modern_native
            .send(json!({
                "jsonrpc":"2.0", "id":0, "method":"initialize",
                "params":{
                    "protocolVersion":"2026-07-28",
                    "capabilities":{"extensions":{"io.modelcontextprotocol/tasks":{}}},
                    "clientInfo":{"name":"modern-native","version":"1"}
                }
            }))
            .await;
        modern_native.receive().await;
        assert_v13_profile_names(&listed_tool_names(&mut modern_native, 1, None).await, true);
        modern_native.shutdown().await;

        let (mut modern_compat, _) = spawn_unica_server(canonical_profile_server());
        modern_compat
            .send(json!({
                "jsonrpc":"2.0", "id":0, "method":"initialize",
                "params":{
                    "protocolVersion":"2026-07-28",
                    "capabilities":{},
                    "clientInfo":{"name":"modern-compat","version":"1"}
                }
            }))
            .await;
        modern_compat.receive().await;
        assert_v13_profile_names(&listed_tool_names(&mut modern_compat, 1, None).await, false);
        modern_compat.shutdown().await;

        // Direct-first requests select independently per request.
        let (mut direct, _) = spawn_unica_server(canonical_profile_server());
        assert_v13_profile_names(
            &listed_tool_names(&mut direct, 1, Some(modern_tasks_meta())).await,
            true,
        );
        assert_v13_profile_names(
            &listed_tool_names(&mut direct, 2, Some(modern_meta())).await,
            false,
        );
        direct.shutdown().await;
    }

    #[tokio::test]
    async fn surface_profiles_publish_eight_native_or_eleven_compatibility_tools_per_client() {
        surface_profiles_case().await;
    }

    #[tokio::test]
    async fn canonical_wire_preserves_optional_arguments_and_minimal_calls() {
        for protocol in ["2024-11-05", "2025-11-25", "2026-07-28"] {
            let observed = Arc::new(std::sync::Mutex::new(Vec::new()));
            let received = Arc::clone(&observed);
            let call: Arc<CanonicalCallHandler> = Arc::new(move |tool, arguments, _, _, _| {
                assert_eq!(tool, V5ToolIdentity::Run);
                received.lock().unwrap().push(arguments.clone());
                direct_outcome(crate::domain::invocation::DomainResult::success(
                    "transport accepted the unchanged request",
                ))
            });
            let (mut client, _) = spawn_unica_server(UnicaServer::with_canonical_v13(call));
            client
                .send(json!({
                    "jsonrpc":"2.0", "id":0, "method":"initialize",
                    "params":{
                        "protocolVersion":protocol,
                        "capabilities":{},
                        "clientInfo":{"name":"optional-arguments-client","version":"1"}
                    }
                }))
                .await;
            let initialized = client.receive().await;
            assert!(initialized.get("error").is_none(), "{initialized}");
            client
                .send(json!({"jsonrpc":"2.0", "id":1, "method":"tools/list", "params":{}}))
                .await;
            let listed = client.receive().await;
            let tools = listed["result"]["tools"].as_array().expect("listed tools");
            let run = tools
                .iter()
                .find(|tool| tool["name"] == "unica.run")
                .unwrap();
            let schema = &run["inputSchema"];
            assert_eq!(schema["required"], json!([]));
            assert_eq!(schema["additionalProperties"], false);
            let validator = jsonschema::validator_for(schema).expect("valid wire schema");
            let minimal_calls = [json!({}), json!({"op":"push", "dryRun":true})];
            for (index, arguments) in minimal_calls.iter().enumerate() {
                validator
                    .validate(arguments)
                    .expect("minimal request accepted by wire schema");
                client
                    .send(json!({
                        "jsonrpc":"2.0", "id":index + 2, "method":"tools/call",
                        "params":{"name":"unica.run", "arguments":arguments}
                    }))
                    .await;
                let response = client.receive().await;
                assert!(response.get("error").is_none(), "{response}");
            }
            assert!(!validator.is_valid(&json!({"allExtensions":false})));
            assert!(!validator.is_valid(&json!({"op":null})));
            assert!(!validator.is_valid(&json!({"dryRun":"true"})));
            let received = observed.lock().unwrap().clone();
            assert_eq!(received.len(), minimal_calls.len());
            for (actual, expected) in received.iter().zip(minimal_calls) {
                assert_eq!(Value::Object(actual.clone()), expected);
            }
            for (name, required) in [
                ("unica.docs", json!(["query"])),
                ("unica.diff", json!(["left", "right"])),
            ] {
                let tool = tools.iter().find(|tool| tool["name"] == name).unwrap();
                assert_eq!(tool["inputSchema"]["required"], required);
            }
            client.shutdown().await;
        }
    }

    async fn compatibility_receipts_case() {
        use crate::domain::invocation::{InvocationStatus, TaskId};
        use std::sync::atomic::AtomicUsize;

        let task_id = TaskId::new();
        let executions = Arc::new(AtomicUsize::new(0));
        let execution_observed = Arc::clone(&executions);
        let call: Arc<CanonicalCallHandler> = Arc::new(move |_, _, _, _, _| {
            execution_observed.fetch_add(1, Ordering::SeqCst);
            Ok(CanonicalCallOutcome::Task(canonical_snapshot(
                task_id,
                InvocationStatus::Working,
                None,
            )))
        });
        let gets = Arc::new(AtomicUsize::new(0));
        let get_observed = Arc::clone(&gets);
        let get: Arc<CanonicalTaskHandler> = Arc::new(move |_, _| {
            get_observed.fetch_add(1, Ordering::SeqCst);
            Ok(canonical_snapshot(task_id, InvocationStatus::Working, None))
        });
        let cancellations = Arc::new(AtomicUsize::new(0));
        let cancel_observed = Arc::clone(&cancellations);
        let cancel: Arc<CanonicalTaskHandler> = Arc::new(move |_, _| {
            cancel_observed.fetch_add(1, Ordering::SeqCst);
            Ok(canonical_snapshot(
                task_id,
                InvocationStatus::Cancelled,
                None,
            ))
        });
        let (mut client, _) =
            spawn_unica_server(UnicaServer::with_canonical_v13_tasks(call, get, cancel));

        client
            .send(json!({
                "jsonrpc":"2.0", "id":1, "method":"tools/call",
                "params":{
                    "name":"unica.check", "arguments":{}, "_meta":modern_meta()
                }
            }))
            .await;
        let initial = client.receive().await;
        assert_ne!(initial["result"]["resultType"], "task", "{initial}");
        assert_eq!(initial["result"]["content"], json!([]), "{initial}");
        assert_eq!(
            initial["result"]["structuredContent"]["data"]["task"]["taskId"],
            task_id.to_string(),
            "{initial}"
        );
        assert_eq!(
            initial["result"]["structuredContent"]["data"]["task"]["status"],
            "working"
        );
        assert!(initial["result"]["structuredContent"].get("work").is_none());
        assert!(initial["result"]["structuredContent"].get("job").is_none());

        client
            .send(json!({
                "jsonrpc":"2.0", "id":2, "method":"tools/call",
                "params":{
                    "name":"unica.task.get",
                    "arguments":{"taskId":task_id.to_string()},
                    "_meta":modern_meta()
                }
            }))
            .await;
        let get_result = client.receive().await;
        assert_eq!(
            get_result["result"]["structuredContent"]["data"]["task"],
            initial["result"]["structuredContent"]["data"]["task"]
        );

        for id in [3, 4] {
            client
                .send(json!({
                    "jsonrpc":"2.0", "id":id, "method":"tools/call",
                    "params":{
                        "name":"unica.task.cancel",
                        "arguments":{"taskId":task_id.to_string()},
                        "_meta":modern_meta()
                    }
                }))
                .await;
            let cancelled = client.receive().await;
            assert_eq!(
                cancelled["result"]["structuredContent"]["diagnostics"][0]["code"],
                "task_cancelled",
                "{cancelled}"
            );
        }
        assert_eq!(executions.load(Ordering::SeqCst), 1);
        assert_eq!(gets.load(Ordering::SeqCst), 1);
        assert_eq!(cancellations.load(Ordering::SeqCst), 2);
        client.shutdown().await;
    }

    fn compatibility_wait_budget_case() {
        let received = Instant::now();
        let deadline = FrontendInvocationDeadline::new(received, None);
        assert_eq!(
            bounded_compatibility_wait_ms(0, deadline, received),
            0,
            "zero is an immediate probe"
        );
        assert_eq!(
            bounded_compatibility_wait_ms(7_000, deadline, received),
            7_000
        );
        assert_eq!(
            bounded_compatibility_wait_ms(7_000, deadline, received + Duration::from_millis(6_999),),
            1,
            "elapsed frontend time is never replenished"
        );
        assert_eq!(
            bounded_compatibility_wait_ms(7_000, deadline, received + Duration::from_secs(7),),
            0
        );
        assert_eq!(
            wait_transport_cutoff(0, deadline),
            received + Duration::from_millis(125)
        );
        assert_eq!(
            wait_transport_cutoff(1, deadline),
            received + Duration::from_millis(126)
        );
        assert_eq!(
            wait_transport_cutoff(7_000, deadline),
            received + Duration::from_millis(7_125)
        );
        assert_eq!(
            wait_transport_cutoff(7_000, deadline),
            received + Duration::from_millis(7_125),
            "elapsed frontend time is not replenished by the compatibility wait"
        );
        assert_eq!(
            wait_transport_cutoff(
                7_000,
                FrontendInvocationDeadline::new(received, Some(Duration::from_millis(80)))
            ),
            received + Duration::from_millis(80),
            "an earlier host deadline is stronger than waitMs plus response margin"
        );
    }

    async fn compatibility_terminal_result_case() {
        use crate::domain::invocation::{InvocationStatus, TaskId};
        use std::sync::atomic::AtomicUsize;

        for is_error in [false, true] {
            let task_id = TaskId::new();
            let mut subject = canonical_result("same terminal subject result");
            subject.ok = !is_error;
            subject.diagnostics = vec![json!({
                "code": "provider_unavailable", "outcome": "needsHuman",
                "message": "v8-runner is unavailable", "remediation": "prefetch the engine"
            })];
            let executions = Arc::new(AtomicUsize::new(0));
            let execution_observed = Arc::clone(&executions);
            let direct_subject = subject.clone();
            let call: Arc<CanonicalCallHandler> = Arc::new(move |_, arguments, _, _, _| {
                execution_observed.fetch_add(1, Ordering::SeqCst);
                if arguments.get("direct").and_then(Value::as_bool) == Some(true) {
                    direct_outcome(direct_subject.clone())
                } else {
                    Ok(CanonicalCallOutcome::Task(canonical_snapshot(
                        task_id,
                        InvocationStatus::Working,
                        None,
                    )))
                }
            });
            let get: Arc<CanonicalTaskHandler> = Arc::new(move |_, _| {
                Ok(canonical_snapshot(task_id, InvocationStatus::Working, None))
            });
            let waits = Arc::new(Mutex::new(Vec::<u64>::new()));
            let waits_observed = Arc::clone(&waits);
            let wait_subject = subject.clone();
            let wait: Arc<CanonicalTaskWaitHandler> = Arc::new(move |_, wait_ms, _, _| {
                waits_observed.lock().unwrap().push(wait_ms);
                Ok(if wait_ms == 0 {
                    canonical_snapshot(task_id, InvocationStatus::Working, None)
                } else {
                    canonical_snapshot(
                        task_id,
                        InvocationStatus::Completed,
                        Some(wait_subject.clone()),
                    )
                })
            });
            let cancel = Arc::clone(&get);
            let server = UnicaServer::with_canonical_v13_task_handlers(call, get, wait, cancel);
            let (mut client, _) = spawn_unica_server(server);

            client
                .send(json!({
                    "jsonrpc":"2.0", "id":1, "method":"tools/call",
                    "params":{
                        "name":"unica.check", "arguments":{"direct":true}, "_meta":modern_meta()
                    }
                }))
                .await;
            let direct = client.receive().await;
            assert_eq!(direct["result"]["isError"], is_error);
            if is_error {
                let fallback: Value = serde_json::from_str(
                    direct["result"]["content"][0]["text"]
                        .as_str()
                        .expect("readable error"),
                )
                .unwrap();
                assert_eq!(fallback["diagnostics"][0]["outcome"], "needsHuman");
                assert_eq!(
                    fallback["diagnostics"][0]["remediation"],
                    "prefetch the engine"
                );
            } else {
                assert_eq!(direct["result"]["content"], json!([]));
            }
            client
                .send(json!({
                    "jsonrpc":"2.0", "id":2, "method":"tools/call",
                    "params":{
                        "name":"unica.check", "arguments":{}, "_meta":modern_meta()
                    }
                }))
                .await;
            let initial = client.receive().await;
            assert_eq!(
                initial["result"]["structuredContent"]["data"]["task"]["taskId"],
                task_id.to_string()
            );
            client
                .send(json!({
                    "jsonrpc":"2.0", "id":3, "method":"tools/call",
                    "params":{
                        "name":"unica.task.result",
                        "arguments":{"taskId":task_id.to_string(), "waitMs":0},
                        "_meta":modern_meta()
                    }
                }))
                .await;
            let still_working = client.receive().await;
            assert_eq!(
                still_working["result"]["structuredContent"]["data"]["task"]["status"], "working",
                "{still_working}"
            );
            client
                .send(json!({
                    "jsonrpc":"2.0", "id":4, "method":"tools/call",
                    "params":{
                        "name":"unica.task.result",
                        "arguments":{"taskId":task_id.to_string()},
                        "_meta":modern_meta()
                    }
                }))
                .await;
            let terminal = client.receive().await;
            assert_eq!(
                serde_json::to_vec(&direct["result"]).unwrap(),
                serde_json::to_vec(&terminal["result"]).unwrap()
            );
            assert_eq!(executions.load(Ordering::SeqCst), 2);
            {
                let waits = waits.lock().unwrap();
                assert_eq!(waits.len(), 2);
                assert_eq!(waits[0], 0);
                assert!(waits[1] <= 7_000);
                assert!(
                    waits[1] > 0,
                    "default result wait must not become immediate"
                );
            }
            client.shutdown().await;
        }
    }

    async fn compatibility_closed_errors_case() {
        use crate::domain::invocation::{InvocationStatus, TaskId};
        use std::sync::atomic::AtomicUsize;

        let known = TaskId::new();
        let unknown = TaskId::new();
        let expired = TaskId::new();
        let subject_executions = Arc::new(AtomicUsize::new(0));
        let subject_observed = Arc::clone(&subject_executions);
        let call: Arc<CanonicalCallHandler> = Arc::new(move |_, _, _, _, _| {
            subject_observed.fetch_add(1, Ordering::SeqCst);
            Ok(CanonicalCallOutcome::Task(canonical_snapshot(
                known,
                InvocationStatus::Working,
                None,
            )))
        });
        let get_calls = Arc::new(AtomicUsize::new(0));
        let get_observed = Arc::clone(&get_calls);
        let get: Arc<CanonicalTaskHandler> = Arc::new(move |task_id, _| {
            get_observed.fetch_add(1, Ordering::SeqCst);
            if task_id == unknown {
                Err(V5TaskExchangeError::Protocol(
                    V5DaemonErrorCode::TaskNotFound,
                ))
            } else {
                Ok(canonical_snapshot(known, InvocationStatus::Working, None))
            }
        });
        let wait_calls = Arc::new(AtomicUsize::new(0));
        let wait_observed = Arc::clone(&wait_calls);
        let wait: Arc<CanonicalTaskWaitHandler> = Arc::new(move |task_id, _, _, _| {
            wait_observed.fetch_add(1, Ordering::SeqCst);
            if task_id == expired {
                Err(V5TaskExchangeError::Protocol(
                    V5DaemonErrorCode::TaskExpired,
                ))
            } else {
                Ok(canonical_snapshot(known, InvocationStatus::Working, None))
            }
        });
        let cancel = Arc::clone(&get);
        let (mut compat, _) = spawn_unica_server(UnicaServer::with_canonical_v13_task_handlers(
            Arc::clone(&call),
            Arc::clone(&get),
            Arc::clone(&wait),
            Arc::clone(&cancel),
        ));

        for (id, name, arguments, expected) in [
            (
                1,
                "unica.task.get",
                json!({"taskId":unknown.to_string()}),
                "task_not_found",
            ),
            (
                2,
                "unica.task.result",
                json!({"taskId":expired.to_string(), "waitMs":0}),
                "task_expired",
            ),
            (
                3,
                "unica.task.get",
                json!({"taskId":"not-canonical"}),
                "invalid_task_id",
            ),
            (
                4,
                "unica.task.result",
                json!({"taskId":known.to_string(), "waitMs":7_001}),
                "bad_wait_ms",
            ),
        ] {
            compat
                .send(json!({
                    "jsonrpc":"2.0", "id":id, "method":"tools/call",
                    "params":{"name":name, "arguments":arguments, "_meta":modern_meta()}
                }))
                .await;
            let response = compat.receive().await;
            assert_eq!(
                response["result"]["structuredContent"]["diagnostics"][0]["code"], expected,
                "{response}"
            );
            assert_eq!(response["result"]["isError"], true, "{response}");
        }
        assert_eq!(subject_executions.load(Ordering::SeqCst), 0);
        assert_eq!(get_calls.load(Ordering::SeqCst), 1);
        assert_eq!(wait_calls.load(Ordering::SeqCst), 1);
        compat.shutdown().await;

        let (mut native, _) = spawn_unica_server(UnicaServer::with_canonical_v13_task_handlers(
            call, get, wait, cancel,
        ));
        native
            .send(json!({
                "jsonrpc":"2.0", "id":1, "method":"tools/call",
                "params":{
                    "name":"unica.task.get",
                    "arguments":{"taskId":known.to_string()},
                    "_meta":modern_tasks_meta()
                }
            }))
            .await;
        let rejected = native.receive().await;
        assert_eq!(rejected["error"]["code"], -32602, "{rejected}");
        assert_eq!(subject_executions.load(Ordering::SeqCst), 0);
        native.shutdown().await;
    }

    /// A known-long service: every call hands off to a durable Task before
    /// execution, which is what the compatibility receipts observe.
    fn known_long_service() -> Arc<ScriptedService> {
        Arc::new(ScriptedService {
            delay: Duration::from_millis(50),
            known_long: true,
            outcome: Mutex::new(Some(Ok(crate::domain::invocation::DomainResult::success(
                "durable compatibility result",
            )))),
            executions: AtomicUsize::new(0),
        })
    }

    async fn compatibility_daemon_restart_case() {
        use crate::domain::invocation::TaskId;
        use std::str::FromStr;

        let first_service = known_long_service();
        let mut daemon = LiveDaemon::start(first_service.clone());
        let workspace_hint = daemon.workspace_hint.clone();
        let (mut first, _) = spawn_unica_server(UnicaServer::with_canonical_daemon(
            daemon.owner(),
            workspace_hint.clone(),
        ));
        first
            .send(json!({
                "jsonrpc":"2.0", "id":1, "method":"tools/call",
                "params":{
                    "name":"unica.run",
                    "arguments":{"op": "test.long-work", "args": {}},
                    "_meta":modern_meta()
                }
            }))
            .await;
        let initial = first.receive().await;
        assert_ne!(initial["result"]["resultType"], "task", "{initial}");
        let task_id_text = initial["result"]["structuredContent"]["data"]["task"]["taskId"]
            .as_str()
            .expect("compatibility receipt must disclose the durable task id")
            .to_owned();
        let task_id = TaskId::from_str(&task_id_text).unwrap();

        first
            .send(json!({
                "jsonrpc":"2.0", "id":2, "method":"tools/call",
                "params":{
                    "name":"unica.task.result", "arguments":{"taskId":task_id_text},
                    "_meta":modern_meta()
                }
            }))
            .await;
        let first_result = first.receive().await;
        assert_eq!(
            first_result["result"]["structuredContent"]["summary"], "durable compatibility result",
            "{first_result}"
        );
        first
            .send(json!({
                "jsonrpc":"2.0", "id":3, "method":"tools/call",
                "params":{
                    "name":"unica.task.get", "arguments":{"taskId":task_id.to_string()},
                    "_meta":modern_meta()
                }
            }))
            .await;
        let before_restart = first.receive().await;
        let before_task = before_restart["result"]["structuredContent"]["data"]["task"].clone();
        assert_eq!(before_task["status"], "completed", "{before_restart}");
        assert_eq!(first_service.executions.load(Ordering::SeqCst), 1);

        first.shutdown().await;
        daemon.stop();

        let second_service = known_long_service();
        daemon.restart(second_service.clone(), Duration::from_millis(400));
        let (mut second, _) = spawn_unica_server(UnicaServer::with_canonical_daemon(
            daemon.owner(),
            workspace_hint,
        ));
        second
            .send(json!({
                "jsonrpc":"2.0", "id":4, "method":"tools/call",
                "params":{
                    "name":"unica.task.get", "arguments":{"taskId":task_id.to_string()},
                    "_meta":modern_meta()
                }
            }))
            .await;
        let after_restart = second.receive().await;
        assert_eq!(
            after_restart["result"]["structuredContent"]["data"]["task"], before_task,
            "task identity, status, timestamps, and TTL must survive daemon restart: {after_restart}"
        );
        second
            .send(json!({
                "jsonrpc":"2.0", "id":5, "method":"tools/call",
                "params":{
                    "name":"unica.task.result", "arguments":{"taskId":task_id.to_string()},
                    "_meta":modern_meta()
                }
            }))
            .await;
        let after_result = second.receive().await;
        assert_eq!(
            serde_json::to_vec(&after_result["result"]).unwrap(),
            serde_json::to_vec(&first_result["result"]).unwrap(),
            "the restarted adapter must project the same durable terminal result"
        );
        assert_eq!(
            second_service.executions.load(Ordering::SeqCst),
            0,
            "a restart never re-executes a completed task"
        );
        second.shutdown().await;
        daemon.finish();
    }

    #[tokio::test]
    async fn compatibility_tools_return_durable_receipts_without_native_tasks_or_reexecution() {
        compatibility_receipts_case().await;
    }

    #[test]
    fn compatibility_result_wait_is_bounded_by_request_and_original_frontend_window() {
        compatibility_wait_budget_case();
    }

    /// The fake answers every Task frame with a working snapshot, optionally
    /// only after `response_delay`; the tests below spend the frontend budget
    /// on connect, handshake and response on purpose.
    fn working_task_fake(
        task_id: crate::domain::invocation::TaskId,
        handshake_delay: Duration,
        response_delay: Duration,
        observed: mpsc::Sender<V5ClientRequest>,
    ) -> FakeDaemon {
        FakeDaemon::start_with_handshake_delay(
            Box::new(move |_, request| {
                let _ = observed.send(request.clone());
                if !response_delay.is_zero() {
                    std::thread::sleep(response_delay);
                }
                Step::Reply(
                    crate::infrastructure::daemon::protocol_v5::V5ServerResponse::Task {
                        snapshot: canonical_snapshot(
                            task_id,
                            crate::domain::invocation::InvocationStatus::Working,
                            None,
                        ),
                    },
                )
            }),
            handshake_delay,
        )
    }

    async fn compatibility_wait_single_deadline_case() {
        for requested_wait_ms in [0_u64, 1] {
            let task_id = crate::domain::invocation::TaskId::new();
            let (observed, observations) = mpsc::channel();
            // Connect and handshake take longer than the requested wait, and the
            // daemon answers only after the 125 ms response margin has passed.
            let fake = working_task_fake(
                task_id,
                Duration::from_millis(60),
                Duration::from_millis(400),
                observed,
            );
            let (mut mcp, _) = spawn_unica_server(UnicaServer::with_canonical_daemon(
                fake.owner(),
                "/workspace".to_string(),
            ));
            mcp.send(json!({
                "jsonrpc":"2.0", "id":1, "method":"tools/call",
                "params":{
                    "name":"unica.task.result",
                    "arguments":{"taskId":task_id.to_string(), "waitMs":requested_wait_ms},
                    "_meta":modern_meta()
                }
            }))
            .await;
            let response = mcp.receive().await;
            assert_eq!(
                response["result"]["structuredContent"]["diagnostics"][0]["code"],
                "task_transport_failed",
                "connect plus wait response exceeded the single {requested_wait_ms}ms + 125ms operation budget: {response}"
            );
            // One budget covers connect, handshake and response: on a loaded
            // runner the handshake alone may cross it, and then the daemon never
            // sees the request. When it does, the wait slice is already spent.
            if let Ok(request) = observations.recv_timeout(Duration::from_millis(500)) {
                assert_eq!(
                    request,
                    V5ClientRequest::WaitTask {
                        task_id,
                        wait_ms: 0
                    },
                    "connect time consumes the wait slice before the 125ms response margin"
                );
            }
            mcp.shutdown().await;
        }
    }

    #[tokio::test]
    async fn compatibility_wait_zero_and_one_share_one_budget_across_connect_and_response() {
        compatibility_wait_single_deadline_case().await;
    }

    fn compatibility_wait_frontend_cutoff_is_not_rebased_case() {
        let task_id = crate::domain::invocation::TaskId::new();
        let (observed, observations) = mpsc::channel();
        let fake = working_task_fake(task_id, Duration::ZERO, Duration::ZERO, observed);
        let router = canonical_daemon_router(fake.owner(), "/workspace".to_string());
        // The request was received a second ago and only now reaches the
        // router: a Duration rebase would open a fresh window here, the
        // absolute cutoff is already expired.
        let received = Instant::now() - Duration::from_secs(1);

        let outcome = (router.wait)(
            task_id,
            0,
            FrontendInvocationDeadline::new(received, None),
            CanonicalCancellation::default(),
        );

        assert_eq!(
            outcome,
            Err(V5TaskExchangeError::Transport),
            "the operation must not rebase its cutoff after the injected pause"
        );
        assert!(
            observations.try_recv().is_err(),
            "an expired absolute cutoff must stop before operation admission"
        );
        assert_eq!(
            fake.sessions.load(Ordering::SeqCst),
            1,
            "only the anchor session was opened"
        );
    }

    fn compatibility_immediate_task_deadline_case(
        tool_name: &'static str,
        host_budget: Duration,
        handshake_elapsed: Duration,
        response_elapsed: Duration,
    ) -> crate::domain::invocation::DomainResult {
        let task_id = crate::domain::invocation::TaskId::new();
        let (observed, observations) = mpsc::channel();
        let fake = working_task_fake(task_id, handshake_elapsed, response_elapsed, observed);
        let received = Instant::now();
        let router = SurfaceToolRouter::CanonicalV13(canonical_daemon_router(
            fake.owner(),
            "/workspace".to_string(),
        ));
        let arguments = json!({"taskId": task_id.to_string()})
            .as_object()
            .unwrap()
            .clone();
        let outcome = execute_surface_tool(
            &router,
            SurfaceToolCall {
                name: tool_name,
                arguments: &arguments,
                host: &unica_bootstrap::HostRequest::default(),
            },
            CancellationToken::new(),
            Arc::new(NoopProgressSink),
            FrontendInvocationDeadline::new(received, Some(host_budget)),
            false,
        )
        .unwrap();
        let SurfaceToolOutcome::Canonical(result) = outcome else {
            panic!("compatibility task tools must return canonical results");
        };
        if let Ok(request) = observations.recv_timeout(Duration::from_millis(500)) {
            let expected = match tool_name {
                "unica.task.get" => V5ClientRequest::GetTask { task_id },
                "unica.task.cancel" => V5ClientRequest::CancelTask { task_id },
                other => panic!("unexpected immediate compatibility tool {other}"),
            };
            assert_eq!(request, expected);
        }
        result
    }

    #[test]
    fn compatibility_get_and_cancel_do_not_replace_open_frontend_cutoff_with_125ms() {
        for tool_name in ["unica.task.get", "unica.task.cancel"] {
            let result = compatibility_immediate_task_deadline_case(
                tool_name,
                Duration::from_millis(1_500),
                Duration::from_millis(200),
                Duration::ZERO,
            );
            assert!(
                result.ok,
                "{tool_name} replaced the open frontend cutoff: {result:?}"
            );
        }
    }

    #[test]
    fn compatibility_get_and_cancel_share_one_absolute_cutoff_across_connect_and_exchange() {
        for tool_name in ["unica.task.get", "unica.task.cancel"] {
            let result = compatibility_immediate_task_deadline_case(
                tool_name,
                Duration::from_millis(300),
                Duration::from_millis(200),
                Duration::from_millis(200),
            );
            assert_eq!(
                result
                    .diagnostics
                    .first()
                    .and_then(|entry| entry["code"].as_str()),
                Some("task_transport_failed"),
                "{tool_name} reopened its transport budget after connect: {result:?}"
            );
        }
    }

    /// A payload that arrives after the operation cutoff is never published,
    /// valid or not, and the operation session is closed for reuse.
    fn compatibility_wait_late_payload_case(valid_near_limit: bool) {
        use crate::application::invocation_store::MAX_CANONICAL_RESULT_BYTES;
        use crate::infrastructure::daemon::protocol_v5::{
            V5ServerResponse, MAX_V5_RESPONSE_LINE_BYTES,
        };

        let task_id = crate::domain::invocation::TaskId::new();
        let response_payload = if valid_near_limit {
            let snapshot = canonical_snapshot(
                task_id,
                crate::domain::invocation::InvocationStatus::Completed,
                Some(crate::domain::invocation::DomainResult::success(
                    "x".repeat(MAX_CANONICAL_RESULT_BYTES - 4_096),
                )),
            );
            let mut bytes = serde_json::to_vec(&V5ServerResponse::Task { snapshot }).unwrap();
            bytes.push(b'\n');
            bytes
        } else {
            let mut hostile = br#"{"kind":"task","snapshot":{"unknown":""#.to_vec();
            hostile.extend(std::iter::repeat_n(
                b'x',
                MAX_V5_RESPONSE_LINE_BYTES - hostile.len() - 4_096,
            ));
            hostile.extend_from_slice(b"\"}}\n");
            hostile
        };
        let (second_request_seen, second_request_seen_wait) = mpsc::channel();
        let fake = FakeDaemon::start_with_raw_script(Box::new(move |_, request, writer| {
            use std::io::Write as _;
            match request {
                V5ClientRequest::WaitTask { .. } => {
                    std::thread::sleep(Duration::from_millis(400));
                    let _ = writer.write_all(&response_payload);
                    let _ = writer.flush();
                    true
                }
                _ => {
                    let _ = second_request_seen.send(());
                    false
                }
            }
        }));
        let anchor = fake.owner();
        let deadline = Instant::now() + Duration::from_millis(150);
        let mut operation = anchor.connect_peer_before(deadline).unwrap();
        let first = operation.wait_task_before(task_id, 0, deadline);
        let second = operation.get_task_before(task_id, Instant::now() + Duration::from_secs(1));
        let saw_second = second_request_seen_wait
            .recv_timeout(Duration::from_millis(800))
            .is_ok();

        assert_eq!(
            first,
            Err(V5TaskExchangeError::Transport),
            "a payload that crossed the cutoff must not publish its snapshot"
        );
        assert_eq!(
            second,
            Err(V5TaskExchangeError::SessionPoisoned),
            "a missed cutoff must poison the operation session"
        );
        assert!(
            !saw_second,
            "a missed cutoff must close the operation session before reuse"
        );
    }

    #[test]
    fn compatibility_wait_preserves_frontend_cutoff_across_client_admission_pause() {
        compatibility_wait_frontend_cutoff_is_not_rebased_case();
    }

    #[test]
    fn compatibility_wait_post_parse_expiry_wins_for_valid_and_malformed_near_limit_frames() {
        compatibility_wait_late_payload_case(true);
        compatibility_wait_late_payload_case(false);
    }

    /// The wait the daemon is asked for once the frontend cutoff, the
    /// handshake and the response margin have been subtracted. The frontend
    /// deadline starts after the anchor session exists, so only the
    /// operation's own connect and handshake spend it.
    fn compatibility_wait_authenticated_long_and_host_cutoff_case(
        requested_wait_ms: u64,
        host_remaining: Option<Duration>,
        expected_daemon_wait_ms: std::ops::RangeInclusive<u64>,
        must_answer: bool,
    ) {
        let task_id = crate::domain::invocation::TaskId::new();
        let (observed, observations) = mpsc::channel();
        let fake = working_task_fake(task_id, Duration::from_millis(60), Duration::ZERO, observed);
        let router = canonical_daemon_router(fake.owner(), "/workspace".to_string());
        let received = Instant::now();

        let outcome = (router.wait)(
            task_id,
            requested_wait_ms,
            FrontendInvocationDeadline::new(received, host_remaining),
            CanonicalCancellation::default(),
        );

        match outcome {
            Ok(snapshot) => assert_eq!(snapshot.task_id(), task_id),
            // A host cutoff shorter than handshake plus margin may expire before
            // the answer arrives; the request the daemon saw is still bounded.
            Err(V5TaskExchangeError::Transport) if !must_answer => {}
            Err(error) => panic!("wait failed: {error:?}"),
        }
        let observed = observations.recv_timeout(Duration::from_secs(2));
        let request = match observed {
            Ok(request) => request,
            Err(_) if !must_answer => return,
            Err(_) => panic!("the daemon never saw the wait request"),
        };
        let V5ClientRequest::WaitTask {
            task_id: asked,
            wait_ms,
        } = request
        else {
            panic!("unexpected frame {request:?}");
        };
        assert_eq!(asked, task_id);
        assert!(
            expected_daemon_wait_ms.contains(&wait_ms),
            "daemon wait {wait_ms} outside {expected_daemon_wait_ms:?}"
        );
    }

    #[test]
    fn compatibility_wait_authenticated_transport_bounds_7000_and_earlier_host_cutoff() {
        // 7000 + 125 ms cutoff, minus the 60 ms handshake and the 125 ms margin;
        // a loaded runner only lowers the value, never raises it above 6940.
        compatibility_wait_authenticated_long_and_host_cutoff_case(
            7_000,
            None,
            6_000..=6_940,
            true,
        );
        // An earlier host cutoff consumes the wait entirely.
        compatibility_wait_authenticated_long_and_host_cutoff_case(
            7_000,
            Some(Duration::from_millis(180)),
            0..=0,
            false,
        );
    }

    #[tokio::test]
    async fn compatibility_result_uses_wait_handler_and_preserves_terminal_direct_bytes() {
        compatibility_terminal_result_case().await;
    }

    #[tokio::test]
    async fn compatibility_task_errors_are_closed_and_native_profile_rejects_adapters() {
        compatibility_closed_errors_case().await;
    }

    async fn compatibility_hostile_status_payload_case() {
        use crate::domain::invocation::{DomainResult, InvocationStatus, TaskId};

        // The v5 snapshot is a closed union: a status cannot arrive with the
        // wrong payload, and a failure arrives as a closed reason without
        // text. Every status therefore projects, and none of them can leak.
        let statuses = [
            InvocationStatus::Queued,
            InvocationStatus::Working,
            InvocationStatus::Completed,
            InvocationStatus::Failed,
            InvocationStatus::Cancelled,
        ];
        for status in statuses {
            let task_id = TaskId::new();
            let snapshot = canonical_snapshot(
                task_id,
                status,
                (status == InvocationStatus::Completed).then(|| {
                    DomainResult::success("hostile result /private/result-secret bearer-result")
                }),
            );
            let get_snapshot = snapshot.clone();
            let get: Arc<CanonicalTaskHandler> = Arc::new(move |_, _| Ok(get_snapshot.clone()));
            let wait_snapshot = snapshot.clone();
            let wait: Arc<CanonicalTaskWaitHandler> =
                Arc::new(move |_, _, _, _| Ok(wait_snapshot.clone()));
            let cancel = Arc::clone(&get);
            let call: Arc<CanonicalCallHandler> =
                Arc::new(move |_, _, _, _, _| direct_outcome(DomainResult::success("unused")));
            let (mut client, _) = spawn_unica_server(
                UnicaServer::with_canonical_v13_task_handlers(call, get, wait, cancel),
            );

            for (id, name, arguments) in [
                (1, "unica.task.get", json!({"taskId": task_id.to_string()})),
                (
                    2,
                    "unica.task.result",
                    json!({"taskId": task_id.to_string(), "waitMs": 0}),
                ),
            ] {
                client
                    .send(json!({
                        "jsonrpc":"2.0", "id":id, "method":"tools/call",
                        "params":{
                            "name":name, "arguments":arguments, "_meta":modern_meta()
                        }
                    }))
                    .await;
                let response = client.receive().await;
                let code = response["result"]["structuredContent"]["diagnostics"][0]["code"]
                    .as_str()
                    .unwrap_or("");
                match status {
                    InvocationStatus::Failed => assert_eq!(code, "task_failed", "{response}"),
                    InvocationStatus::Cancelled => {
                        assert_eq!(code, "task_cancelled", "{response}")
                    }
                    _ => assert_ne!(
                        code, "task_projection_failed",
                        "status={status:?} {name}: {response}"
                    ),
                }
                let serialized = serde_json::to_string(&response).unwrap();
                if status != InvocationStatus::Completed {
                    for forbidden in ["/private/result-secret", "bearer-result"] {
                        assert!(!serialized.contains(forbidden), "leaked {forbidden}");
                    }
                }
                assert!(
                    !serialized.contains("invocation_failed"),
                    "the closed failure reason stays on the daemon side: {serialized}"
                );
            }
            client.shutdown().await;
        }
    }

    #[tokio::test]
    async fn compatibility_adapter_rejects_every_hostile_status_payload_shape_without_leaking_failure(
    ) {
        compatibility_hostile_status_payload_case().await;
    }

    #[tokio::test]
    async fn compatibility_adapter_reconnects_to_the_same_durable_task_after_daemon_restart() {
        compatibility_daemon_restart_case().await;
    }

    #[tokio::test]
    async fn v13_compatibility_task_tools_are_profile_gated_durable_and_replay_free() {
        surface_profiles_case().await;
        compatibility_receipts_case().await;
        compatibility_wait_budget_case();
        compatibility_wait_single_deadline_case().await;
        compatibility_wait_frontend_cutoff_is_not_rebased_case();
        compatibility_wait_late_payload_case(true);
        compatibility_wait_late_payload_case(false);
        compatibility_wait_authenticated_long_and_host_cutoff_case(
            7_000,
            None,
            6_000..=6_940,
            true,
        );
        compatibility_wait_authenticated_long_and_host_cutoff_case(
            7_000,
            Some(Duration::from_millis(180)),
            0..=0,
            false,
        );
        compatibility_terminal_result_case().await;
        compatibility_closed_errors_case().await;
        compatibility_hostile_status_payload_case().await;
        compatibility_daemon_restart_case().await;
    }

    struct ManualCanonicalDaemonOwner(
        Option<crate::infrastructure::daemon::server::actor_capacity_tests::LiveV5Daemon>,
    );
    impl std::ops::Deref for ManualCanonicalDaemonOwner {
        type Target = crate::infrastructure::daemon::server::actor_capacity_tests::LiveV5Daemon;
        fn deref(&self) -> &Self::Target {
            self.0.as_ref().unwrap()
        }
    }
    impl ManualCanonicalDaemonOwner {
        fn finish(mut self, owner: V5DaemonProcessOwner) {
            self.0.take().unwrap().finish(owner);
        }
    }
    impl Drop for ManualCanonicalDaemonOwner {
        fn drop(&mut self) {
            if let Some(daemon) = self.0.take() {
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let owner = daemon.owner();
                    daemon.finish(owner);
                }));
            }
        }
    }
    struct ManualRunnerRelease(std::path::PathBuf);
    impl Drop for ManualRunnerRelease {
        fn drop(&mut self) {
            let _ = std::fs::write(&self.0, "finish mutation");
        }
    }
    struct ManualFlushRelease(Arc<ManualFlushGate>);
    impl Drop for ManualFlushRelease {
        fn drop(&mut self) {
            self.0.release();
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn public_task_cancel_and_result_preserve_a_started_infobase_create_receipt() {
        protected_create_cancel_case(false).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn canonical_manual_cancellation_protected_create_preserves_external_receipt_without_replay(
    ) {
        protected_create_cancel_case(true).await;
    }

    async fn protected_create_cancel_case(manual: bool) {
        use crate::infrastructure::daemon::server::actor_capacity_tests::{
            canonical_v13_service, install_cancellable_create_runner, LiveV5Daemon,
        };

        async fn terminal_result(
            client: &mut McpClient,
            task_id: &str,
            deadline: Instant,
        ) -> Value {
            let mut request_id = 10_u64;
            loop {
                assert!(Instant::now() < deadline, "task.result did not settle");
                client
                    .send(json!({
                        "jsonrpc":"2.0", "id":request_id, "method":"tools/call",
                        "params":{"name":"unica.task.result", "arguments":{"taskId":task_id,"waitMs":1000}, "_meta":modern_meta()}
                    }))
                    .await;
                let result = client.receive().await;
                if !matches!(
                    result["result"]["structuredContent"]["data"]["task"]["status"].as_str(),
                    Some("queued" | "working")
                ) {
                    return result;
                }
                request_id += 1;
            }
        }

        let root = tempfile::tempdir().unwrap();
        install_cancellable_create_runner(root.path());
        let workspace = root.path().join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(
            workspace.join("v8project.yaml"),
            "format: DESIGNER\ninfobase:\n  connection: 'File=build/ib'\n",
        )
        .unwrap();
        let workspace = std::fs::canonicalize(workspace).unwrap();
        let daemon = ManualCanonicalDaemonOwner(Some(LiveV5Daemon::start(canonical_v13_service())));
        let _runner_release = ManualRunnerRelease(workspace.join("release.marker"));
        let owner = daemon.owner();
        let flush = Arc::new(ManualFlushGate::default());
        let _flush_release = ManualFlushRelease(flush.clone());
        let server = UnicaServer::with_canonical_daemon(
            daemon.owner(),
            workspace.to_string_lossy().into_owned(),
        );
        let (mut client, _) = if manual {
            spawn_manual_flush_server(server, flush.clone())
        } else {
            spawn_unica_server(server)
        };
        client
            .send(json!({
                "jsonrpc":"2.0", "id":1, "method":"tools/call",
                "params":{"name":"unica.run", "arguments":{"op":"infobase.create","args":{},"dryRun":true}, "_meta":modern_meta()}
            }))
            .await;
        let preview = client.receive().await;
        let preview_id = preview["result"]["structuredContent"]["data"]["task"]["taskId"]
            .as_str()
            .unwrap_or_else(|| panic!("preview did not return a task: {preview}"));
        let preview_result = terminal_result(
            &mut client,
            preview_id,
            Instant::now() + Duration::from_secs(20),
        )
        .await;
        assert_eq!(
            preview_result["result"]["structuredContent"]["ok"], true,
            "{preview_result}"
        );
        assert!(
            preview_result["result"]["structuredContent"]
                .get("rev")
                .is_none(),
            "{preview_result}"
        );
        assert!(!workspace.join("entered.marker").exists());
        if manual {
            flush.armed.store(true, Ordering::Release);
        }
        client
            .send(json!({
                "jsonrpc":"2.0", "id":3, "method":"tools/call",
                "params":{"name":"unica.run", "arguments":{"op":"infobase.create","args":{},"dryRun":false}, "_meta":modern_meta()}
            }))
            .await;
        let apply = client.receive().await;
        let task_id = apply["result"]["structuredContent"]["data"]["task"]["taskId"]
            .as_str()
            .unwrap_or_else(|| panic!("apply did not return a task: {apply}"))
            .to_owned();
        let deadline = Instant::now() + Duration::from_secs(20);
        while !workspace.join("entered.marker").exists() {
            assert!(Instant::now() < deadline, "mutating runner did not start");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        if manual {
            assert!(
                flush.wait().await,
                "original apply response must still own the actual blocked flush"
            );
            cancel_manual_daemon_call(&mut client, 3).await;
            let internal: crate::domain::invocation::TaskId = task_id.parse().unwrap();
            let watchdog = Instant::now() + Duration::from_secs(3);
            loop {
                let snapshot = daemon.get(&owner, internal);
                if snapshot.cancel_requested() {
                    assert_eq!(
                        snapshot.status(),
                        crate::domain::invocation::InvocationStatus::Working
                    );
                    break;
                }
                assert!(
                    Instant::now() < watchdog,
                    "manual notification did not reach original protected create"
                );
                tokio::task::yield_now().await;
            }
            flush.release();
        } else {
            client.send(json!({"jsonrpc":"2.0", "id":4, "method":"tools/call", "params":{"name":"unica.task.cancel", "arguments":{"taskId":task_id}, "_meta":modern_meta()}})).await;
            let cancelled = client.receive().await;
            assert_eq!(
                cancelled["result"]["structuredContent"]["data"]["task"]["status"], "working",
                "{cancelled}"
            );
            assert_eq!(
                cancelled["result"]["structuredContent"]["data"]["task"]["cancelRequested"], true,
                "{cancelled}"
            );
        }
        assert!(!workspace.join("created.marker").exists());
        std::fs::write(workspace.join("release.marker"), "finish mutation").unwrap();
        let result = terminal_result(
            &mut client,
            &task_id,
            Instant::now() + Duration::from_secs(20),
        )
        .await;
        assert_eq!(
            result["result"]["structuredContent"]["ok"], true,
            "{result}"
        );
        assert_eq!(
            result["result"]["structuredContent"]["data"]["state"], "created",
            "{result}"
        );
        assert!(
            result["result"]["structuredContent"].get("rev").is_none(),
            "{result}"
        );
        assert_eq!(
            result["result"]["structuredContent"]["data"]["receipt"],
            "repeated preview reports nothing left to create",
            "{result}"
        );
        assert!(workspace.join("created.marker").exists());
        let internal_task: crate::domain::invocation::TaskId = task_id.parse().unwrap();
        assert!(matches!(
            daemon.get(&owner, internal_task),
            crate::infrastructure::daemon::protocol_v5::V5DaemonTaskSnapshot::Completed {
                cancel_requested: true,
                ..
            }
        ));
        let replay = terminal_result(
            &mut client,
            &task_id,
            Instant::now() + Duration::from_secs(20),
        )
        .await;
        assert_eq!(
            replay["result"]["structuredContent"], result["result"]["structuredContent"],
            "observation must preserve the factual receipt without replaying creation"
        );
        assert_eq!(
            std::fs::read_to_string(workspace.join("apply-dispatch.log"))
                .unwrap()
                .lines()
                .count(),
            1,
            "repeated task observation must never dispatch a second mutating runner"
        );
        client.shutdown().await;
        daemon.finish(owner);
    }

    #[tokio::test]
    async fn public_native_cancel_waits_for_slow_job_attach_before_answering() {
        use crate::infrastructure::daemon::server::actor_capacity_tests::{
            canonical_v13_service, install_cancellable_create_runner, LiveV5Daemon,
        };
        use crate::infrastructure::platform::JobAttachGateForTest;

        if !JobAttachGateForTest::supported() {
            eprintln!("Windows Job attachment is unavailable on this host");
            return;
        }

        let root = tempfile::tempdir().unwrap();
        install_cancellable_create_runner(root.path());
        let workspace = root.path().join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(
            workspace.join("v8project.yaml"),
            "format: DESIGNER\ninfobase:\n  connection: 'File=build/ib'\n",
        )
        .unwrap();
        let workspace = std::fs::canonicalize(workspace).unwrap();
        let daemon = LiveV5Daemon::start(canonical_v13_service());
        let owner = daemon.owner();
        let (mut client, _) = spawn_unica_server(UnicaServer::with_canonical_daemon(
            daemon.owner(),
            workspace.to_string_lossy().into_owned(),
        ));

        let target = crate::infrastructure::platform::current_target_id().unwrap();
        let runner = root
            .path()
            .join("plugins/unica/bin")
            .join(target)
            .join(format!("v8-runner{}", std::env::consts::EXE_SUFFIX));
        let attach = JobAttachGateForTest::install(std::fs::canonicalize(runner).unwrap());
        client
            .send(json!({
                "jsonrpc":"2.0", "id":3, "method":"tools/call",
                "params":{"name":"unica.run", "arguments":{"op":"infobase.create","args":{},"dryRun":false}, "_meta":modern_meta()}
            }))
            .await;
        let apply = client.receive().await;
        let task_id = apply["result"]["structuredContent"]["data"]["task"]["taskId"]
            .as_str()
            .unwrap_or_else(|| panic!("apply did not return a task: {apply}"))
            .to_owned();
        let parsed_task_id = task_id.parse().unwrap();
        attach.wait_spawned(Duration::from_secs(20));
        assert!(!workspace.join("entered.marker").exists());

        let sent_at = Instant::now();
        client
            .send(json!({
                "jsonrpc":"2.0", "id":4, "method":"tasks/cancel",
                "params":{"taskId":task_id, "_meta":modern_tasks_meta()}
            }))
            .await;
        let intent_deadline = Instant::now() + Duration::from_secs(5);
        while !daemon.get(&owner, parsed_task_id).cancel_requested() {
            assert!(
                Instant::now() < intent_deadline,
                "cancel intent was not saved"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(
            timeout(Duration::from_millis(250), client.reader.next_line())
                .await
                .is_err(),
            "tasks/cancel answered before the suspended runner was attached"
        );
        assert!(!workspace.join("entered.marker").exists());
        attach.release();
        let cancelled = client.receive().await;
        let elapsed = sent_at.elapsed();
        eprintln!("public tasks/cancel with delayed Windows Job attach: {elapsed:?}");
        assert!(elapsed >= Duration::from_millis(250), "{elapsed:?}");
        assert!(elapsed <= Duration::from_millis(7_125), "{elapsed:?}");
        assert_eq!(cancelled["result"]["resultType"], "complete", "{cancelled}");

        client
            .send(json!({
                "jsonrpc":"2.0", "id":5, "method":"tasks/get",
                "params":{"taskId":task_id, "_meta":modern_tasks_meta()}
            }))
            .await;
        let task = client.receive().await;
        assert_eq!(task["result"]["status"], "working", "{task}");
        assert!(task["result"].get("statusMessage").is_some(), "{task}");

        let entered_deadline = Instant::now() + Duration::from_secs(20);
        while !workspace.join("entered.marker").exists() {
            assert!(Instant::now() < entered_deadline, "runner did not start");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        std::fs::write(workspace.join("release.marker"), "finish mutation").unwrap();
        let result_deadline = Instant::now() + Duration::from_secs(20);
        loop {
            assert!(Instant::now() < result_deadline, "task did not settle");
            client
                .send(json!({
                    "jsonrpc":"2.0", "id":6, "method":"tools/call",
                    "params":{"name":"unica.task.result", "arguments":{"taskId":task_id,"waitMs":1000}, "_meta":modern_meta()}
                }))
                .await;
            let result = client.receive().await;
            if matches!(
                result["result"]["structuredContent"]["data"]["task"]["status"].as_str(),
                Some("queued" | "working")
            ) {
                continue;
            }
            assert_eq!(
                result["result"]["structuredContent"]["ok"], true,
                "{result}"
            );
            assert_eq!(
                result["result"]["structuredContent"]["data"]["state"], "created",
                "{result}"
            );
            break;
        }
        assert!(workspace.join("created.marker").exists());
        client.shutdown().await;
        daemon.finish(owner);
    }

    async fn tasks_direct_first_capability_case() {
        use crate::domain::invocation::{InvocationStatus, TaskId};
        use std::sync::atomic::AtomicUsize;

        let task_id = TaskId::new();
        let executions = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&executions);
        let call: Arc<CanonicalCallHandler> = Arc::new(move |_, _, _, _, _| {
            observed.fetch_add(1, Ordering::SeqCst);
            Ok(CanonicalCallOutcome::Task(canonical_snapshot(
                task_id,
                InvocationStatus::Working,
                None,
            )))
        });
        let get: Arc<CanonicalTaskHandler> =
            Arc::new(move |_, _| Ok(canonical_snapshot(task_id, InvocationStatus::Working, None)));
        let cancel = Arc::clone(&get);

        let server = UnicaServer::with_canonical_v13_tasks(call, get, cancel);
        assert!(server.get_info().capabilities.supports_tasks());
        let (mut client, _) = spawn_unica_server(server);
        client
            .send(json!({
                "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": {
                    "name": "unica.check", "arguments": {},
                    "_meta": modern_tasks_meta()
                }
            }))
            .await;
        let native = client.receive().await;
        assert_eq!(native["result"]["resultType"], "task", "{native}");
        assert_eq!(native["result"]["taskId"], task_id.to_string());
        assert_eq!(executions.load(Ordering::SeqCst), 1);
        assert!(
            timeout(Duration::from_millis(50), client.reader.next_line())
                .await
                .is_err(),
            "task projection must not synthesize progress or polling traffic after CreateTaskResult"
        );
        client.shutdown().await;

        let executions_without = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&executions_without);
        let call: Arc<CanonicalCallHandler> = Arc::new(move |_, _, _, _, _| {
            observed.fetch_add(1, Ordering::SeqCst);
            Ok(CanonicalCallOutcome::Task(canonical_snapshot(
                task_id,
                InvocationStatus::Working,
                None,
            )))
        });
        let get: Arc<CanonicalTaskHandler> =
            Arc::new(move |_, _| Ok(canonical_snapshot(task_id, InvocationStatus::Working, None)));
        let server = UnicaServer::with_canonical_v13_tasks(call, Arc::clone(&get), get);
        let (mut client, _) = spawn_unica_server(server);
        client
            .send(json!({
                "jsonrpc": "2.0", "id": 2, "method": "tools/call",
                "params": {
                    "name": "unica.check", "arguments": {},
                    "_meta": modern_meta()
                }
            }))
            .await;
        let compatibility = client.receive().await;
        assert_ne!(
            compatibility["result"]["resultType"], "task",
            "{compatibility}"
        );
        assert_eq!(executions_without.load(Ordering::SeqCst), 1);
        client.shutdown().await;

        let legacy_session_executions = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&legacy_session_executions);
        let call: Arc<CanonicalCallHandler> = Arc::new(move |_, _, _, _, _| {
            observed.fetch_add(1, Ordering::SeqCst);
            Ok(CanonicalCallOutcome::Task(canonical_snapshot(
                task_id,
                InvocationStatus::Working,
                None,
            )))
        });
        let get: Arc<CanonicalTaskHandler> =
            Arc::new(move |_, _| Ok(canonical_snapshot(task_id, InvocationStatus::Working, None)));
        let server = UnicaServer::with_canonical_v13_tasks(call, Arc::clone(&get), get);
        let (mut client, _) = spawn_unica_server(server);
        client
            .send(json!({
                "jsonrpc":"2.0", "id":0, "method":"initialize",
                "params": {
                    "protocolVersion":"2025-11-25",
                    "capabilities": {"extensions":{"io.modelcontextprotocol/tasks":{}}},
                    "clientInfo":{"name":"explicit-task-client","version":"1"}
                }
            }))
            .await;
        let initialized = client.receive().await;
        assert!(
            initialized["result"]["capabilities"]["extensions"]["io.modelcontextprotocol/tasks"]
                .is_null(),
            "2025-11-25 must not advertise SEP-2663: {initialized}"
        );
        client
            .send(json!({
                "jsonrpc":"2.0", "id":8, "method":"tools/call",
                "params":{"name":"unica.check", "arguments":{}}
            }))
            .await;
        let native = client.receive().await;
        assert_ne!(native["result"]["resultType"], "task", "{native}");
        assert_eq!(legacy_session_executions.load(Ordering::SeqCst), 1);
        client
            .send(json!({
                "jsonrpc":"2.0", "id":9, "method":"tasks/get",
                "params":{"taskId":task_id.to_string()}
            }))
            .await;
        let unavailable = client.receive().await;
        assert_eq!(unavailable["error"]["code"], -32601, "{unavailable}");
        client.shutdown().await;
    }

    async fn legacy_initialized_session_cannot_escalate_tasks_per_request_case() {
        use crate::domain::invocation::{InvocationStatus, TaskId};
        use std::sync::atomic::AtomicUsize;

        let task_id = TaskId::new();
        let executions = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&executions);
        let call: Arc<CanonicalCallHandler> = Arc::new(move |_, _, _, _, _| {
            observed.fetch_add(1, Ordering::SeqCst);
            Ok(CanonicalCallOutcome::Task(canonical_snapshot(
                task_id,
                InvocationStatus::Working,
                None,
            )))
        });
        let get: Arc<CanonicalTaskHandler> =
            Arc::new(move |_, _| Ok(canonical_snapshot(task_id, InvocationStatus::Working, None)));
        let server = UnicaServer::with_canonical_v13_tasks(call, Arc::clone(&get), get);
        let (mut client, _) = spawn_unica_server(server);
        client
            .send(json!({
                "jsonrpc":"2.0", "id":0, "method":"initialize",
                "params": {
                    "protocolVersion":"2025-11-25",
                    "capabilities":{"extensions":{"io.modelcontextprotocol/tasks":{}}},
                    "clientInfo":{"name":"legacy-hybrid-client","version":"1"}
                }
            }))
            .await;
        let initialized = client.receive().await;
        assert_eq!(initialized["result"]["protocolVersion"], "2025-11-25");

        client
            .send(json!({
                "jsonrpc":"2.0", "id":1, "method":"tools/call",
                "params": {
                    "name":"unica.check", "arguments":{},
                    "_meta":modern_tasks_meta()
                }
            }))
            .await;
        let call_response = client.receive().await;

        let mut task_method_codes = Vec::new();
        for (id, method) in [(2, "tasks/get"), (3, "tasks/update"), (4, "tasks/cancel")] {
            let mut params = json!({
                "taskId":task_id.to_string(),
                "_meta":modern_tasks_meta()
            });
            if method == "tasks/update" {
                params["inputResponses"] = json!({});
            }
            client
                .send(json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params}))
                .await;
            task_method_codes.push(client.receive().await["error"]["code"].as_i64());
        }

        assert_eq!(
            (
                call_response["result"]["resultType"].as_str(),
                task_method_codes,
                executions.load(Ordering::SeqCst),
            ),
            (
                Some("complete"),
                vec![Some(-32601), Some(-32601), Some(-32601)],
                1,
            ),
            "legacy initialize authority was escalated by request metadata: {call_response}"
        );
        client.shutdown().await;
    }

    async fn modern_initialized_session_retains_native_tasks_case() {
        use crate::domain::invocation::{InvocationStatus, TaskId};

        let task_id = TaskId::new();
        let call: Arc<CanonicalCallHandler> = Arc::new(move |_, _, _, _, _| {
            Ok(CanonicalCallOutcome::Task(canonical_snapshot(
                task_id,
                InvocationStatus::Working,
                None,
            )))
        });
        let get: Arc<CanonicalTaskHandler> =
            Arc::new(move |_, _| Ok(canonical_snapshot(task_id, InvocationStatus::Working, None)));
        let server = UnicaServer::with_canonical_v13_tasks(call, Arc::clone(&get), get);
        let (mut client, _) = spawn_unica_server(server);
        client
            .send(json!({
                "jsonrpc":"2.0", "id":0, "method":"initialize",
                "params": {
                    "protocolVersion":"2026-07-28",
                    "capabilities":{"extensions":{"io.modelcontextprotocol/tasks":{}}},
                    "clientInfo":{"name":"modern-task-client","version":"1"}
                }
            }))
            .await;
        let initialized = client.receive().await;
        assert_eq!(initialized["result"]["protocolVersion"], "2026-07-28");
        client
            .send(json!({
                "jsonrpc":"2.0", "id":1, "method":"tools/call",
                "params":{"name":"unica.check", "arguments":{}}
            }))
            .await;
        let response = client.receive().await;
        assert_eq!(response["result"]["resultType"], "task", "{response}");
        client.shutdown().await;
    }

    async fn native_task_methods_preserve_one_frontend_transport_cutoff_case() {
        use crate::domain::invocation::{InvocationStatus, TaskId};

        let task_id = TaskId::new();
        let call: Arc<CanonicalCallHandler> = Arc::new(move |_, _, _, _, _| {
            Ok(CanonicalCallOutcome::Task(canonical_snapshot(
                task_id,
                InvocationStatus::Working,
                None,
            )))
        });
        let (observed, observations) = mpsc::channel();
        let get_observed = observed.clone();
        let get: Arc<CanonicalTaskHandler> = Arc::new(move |_, deadline| {
            get_observed
                .send(deadline.remaining_transport_at(Instant::now()))
                .unwrap();
            Ok(canonical_snapshot(task_id, InvocationStatus::Working, None))
        });
        let cancel: Arc<CanonicalTaskHandler> = Arc::new(move |_, deadline| {
            observed
                .send(deadline.remaining_transport_at(Instant::now()))
                .unwrap();
            Ok(canonical_snapshot(
                task_id,
                InvocationStatus::Cancelled,
                None,
            ))
        });
        let server = UnicaServer::with_canonical_v13_tasks(call, get, cancel);
        let (mut client, _) = spawn_unica_server(server);
        client
            .send(json!({
                "jsonrpc":"2.0", "id":0, "method":"initialize",
                "params": {
                    "protocolVersion":"2026-07-28",
                    "capabilities":{"extensions":{"io.modelcontextprotocol/tasks":{}}},
                    "clientInfo":{"name":"native-task-deadline-client","version":"1"}
                }
            }))
            .await;
        let initialized = client.receive().await;
        assert_eq!(initialized["result"]["protocolVersion"], "2026-07-28");

        for (id, method) in [(1, "tasks/get"), (2, "tasks/update"), (3, "tasks/cancel")] {
            let mut params = json!({
                "taskId": task_id.to_string(),
                "_meta": modern_tasks_meta()
            });
            if method == "tasks/update" {
                params["inputResponses"] = json!({});
            }
            client
                .send(json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params}))
                .await;
            let _response = client.receive().await;
        }

        let upper = INVOCATION_HANDOFF_WINDOW + RESPONSE_SERIALIZATION_MARGIN;
        for method in ["tasks/get", "tasks/update", "tasks/cancel"] {
            let remaining = observations
                .recv_timeout(Duration::from_secs(1))
                .expect("native task handler did not observe its frontend cutoff");
            assert!(
                remaining > Duration::from_millis(250) && remaining <= upper,
                "{method} replaced the shared frontend cutoff with a phase-local window: {remaining:?}"
            );
        }
        client.shutdown().await;
    }

    async fn tasks_direct_and_completed_get_case() {
        use crate::domain::invocation::{InvocationStatus, TaskId};

        for is_error in [false, true] {
            let task_id = TaskId::new();
            let mut expected = canonical_result("same canonical result");
            expected.ok = !is_error;
            expected.diagnostics = vec![json!({
                "code": "bad_value", "outcome": "fixCall", "message": "correct the source set"
            })];
            let direct_expected = expected.clone();
            let call: Arc<CanonicalCallHandler> = Arc::new(move |_, arguments, _, _, _| {
                if arguments.get("async").and_then(Value::as_bool) == Some(true) {
                    Ok(CanonicalCallOutcome::Task(canonical_snapshot(
                        task_id,
                        InvocationStatus::Working,
                        None,
                    )))
                } else {
                    direct_outcome(direct_expected.clone())
                }
            });
            let get_expected = expected.clone();
            let get: Arc<CanonicalTaskHandler> = Arc::new(move |_, _| {
                Ok(canonical_snapshot(
                    task_id,
                    InvocationStatus::Completed,
                    Some(get_expected.clone()),
                ))
            });
            let server = UnicaServer::with_canonical_v13_tasks(call, Arc::clone(&get), get);
            let (mut client, _) = spawn_unica_server(server);

            for (id, arguments) in [(1, json!({})), (2, json!({"async": true}))] {
                client
                    .send(json!({
                        "jsonrpc": "2.0", "id": id, "method": "tools/call",
                        "params": {
                            "name": "unica.check", "arguments": arguments,
                            "_meta": modern_tasks_meta()
                        }
                    }))
                    .await;
                let response = client.receive().await;
                if id == 1 {
                    assert_eq!(response["result"]["resultType"], "complete");
                    assert_eq!(response["result"]["isError"], is_error);
                    if is_error {
                        let fallback: Value = serde_json::from_str(
                            response["result"]["content"][0]["text"]
                                .as_str()
                                .expect("readable error"),
                        )
                        .unwrap();
                        assert_eq!(fallback["diagnostics"][0]["outcome"], "fixCall");
                    } else {
                        assert_eq!(response["result"]["content"], json!([]));
                    }
                    client
                        .send(json!({
                            "jsonrpc": "2.0", "id": 3, "method": "tasks/get",
                            "params": {"taskId": task_id.to_string(), "_meta": modern_tasks_meta()}
                        }))
                        .await;
                    let completed = client.receive().await;
                    assert_eq!(
                    serde_json::to_vec(&response["result"]).unwrap(),
                    serde_json::to_vec(&completed["result"]["result"]).unwrap(),
                    "direct and durable terminal projections diverged: direct={response}, task={completed}"
                );
                } else {
                    assert_eq!(response["result"]["resultType"], "task", "{response}");
                }
            }
            client.shutdown().await;
        }
    }

    async fn tasks_projection_rejects_reverse_timestamps_on_wire_case() {
        use crate::domain::invocation::{InvocationStatus, TaskId};

        let task_id = TaskId::new();
        let reversed = canonical_snapshot_at(
            task_id,
            InvocationStatus::Working,
            None,
            1_777_012_345_678,
            1_777_012_345_677,
        );
        let call_snapshot = reversed.clone();
        let call: Arc<CanonicalCallHandler> =
            Arc::new(move |_, _, _, _, _| Ok(CanonicalCallOutcome::Task(call_snapshot.clone())));
        let get: Arc<CanonicalTaskHandler> = Arc::new(move |_, _| Ok(reversed.clone()));
        let server = UnicaServer::with_canonical_v13_tasks(call, Arc::clone(&get), get);
        let (mut client, _) = spawn_unica_server(server);

        let mut projection_codes = Vec::new();
        for (id, method, params) in [
            (
                1,
                "tools/call",
                json!({"name":"unica.check", "arguments":{}, "_meta":modern_tasks_meta()}),
            ),
            (
                2,
                "tasks/get",
                json!({"taskId":task_id.to_string(), "_meta":modern_tasks_meta()}),
            ),
        ] {
            client
                .send(json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params}))
                .await;
            projection_codes.push(client.receive().await["error"]["data"]["code"].clone());
        }
        assert_eq!(
            projection_codes,
            vec![
                json!("task_projection_failed"),
                json!("task_projection_failed")
            ]
        );
        client.shutdown().await;
    }

    async fn tasks_projection_keeps_near_limit_wire_bounded_and_rejects_over_limit_case() {
        use crate::application::invocation_store::{
            MAX_CANONICAL_RESULT_BYTES, MAX_TASK_RECORD_ENVELOPE_BYTES,
        };
        use crate::domain::invocation::{InvocationStatus, TaskId};
        use std::sync::atomic::AtomicUsize;

        let near = crate::domain::invocation::DomainResult::success(
            "x".repeat(MAX_CANONICAL_RESULT_BYTES - 4_096),
        );
        let over = crate::domain::invocation::DomainResult::success(
            "x".repeat(MAX_CANONICAL_RESULT_BYTES + 1),
        );
        let near_task = TaskId::new();
        let over_task = TaskId::new();
        let executions = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&executions);
        let call_near = near.clone();
        let call_over = over.clone();
        let call: Arc<CanonicalCallHandler> = Arc::new(move |_, arguments, _, _, _| {
            observed.fetch_add(1, Ordering::SeqCst);
            match arguments.get("mode").and_then(Value::as_str) {
                Some("near-task") => Ok(CanonicalCallOutcome::Task(canonical_snapshot(
                    near_task,
                    InvocationStatus::Working,
                    None,
                ))),
                Some("over-direct") => direct_outcome(call_over.clone()),
                Some("over-task") => Ok(CanonicalCallOutcome::Task(canonical_snapshot(
                    over_task,
                    InvocationStatus::Working,
                    None,
                ))),
                _ => direct_outcome(call_near.clone()),
            }
        });
        let get_near = near.clone();
        let get_over = over.clone();
        let get: Arc<CanonicalTaskHandler> = Arc::new(move |task_id, _| {
            Ok(if task_id == near_task {
                canonical_snapshot(
                    near_task,
                    InvocationStatus::Completed,
                    Some(get_near.clone()),
                )
            } else {
                canonical_snapshot(
                    over_task,
                    InvocationStatus::Completed,
                    Some(get_over.clone()),
                )
            })
        });
        let server = UnicaServer::with_canonical_v13_tasks(call, Arc::clone(&get), get);
        let (mut client, _) = spawn_unica_server(server);

        client
            .send(json!({
                "jsonrpc":"2.0", "id":1, "method":"tools/call",
                "params":{"name":"unica.check", "arguments":{}, "_meta":modern_tasks_meta()}
            }))
            .await;
        let direct = client.receive().await;
        client
            .send(json!({
                "jsonrpc":"2.0", "id":2, "method":"tools/call",
                "params":{"name":"unica.check", "arguments":{"mode":"near-task"}, "_meta":modern_tasks_meta()}
            }))
            .await;
        let _created = client.receive().await;
        client
            .send(json!({
                "jsonrpc":"2.0", "id":3, "method":"tasks/get",
                "params":{"taskId":near_task.to_string(), "_meta":modern_tasks_meta()}
            }))
            .await;
        let completed = client.receive().await;

        let projection_limit = MAX_CANONICAL_RESULT_BYTES + MAX_TASK_RECORD_ENVELOPE_BYTES;
        let direct_bytes = serde_json::to_vec(&direct["result"]).unwrap();
        let detailed_bytes = serde_json::to_vec(&completed["result"]).unwrap();
        assert_eq!(
            serde_json::to_vec(&direct["result"]).unwrap(),
            serde_json::to_vec(&completed["result"]["result"]).unwrap()
        );
        assert_eq!(direct["result"]["content"], json!([]));
        assert!(
            direct_bytes.len() <= projection_limit,
            "direct bytes={}",
            direct_bytes.len()
        );
        assert!(
            detailed_bytes.len() <= projection_limit,
            "detailed bytes={}",
            detailed_bytes.len()
        );

        let mut over_codes = Vec::new();
        for (id, method, params) in [
            (
                4,
                "tools/call",
                json!({"name":"unica.check", "arguments":{"mode":"over-direct"}, "_meta":modern_tasks_meta()}),
            ),
            (
                5,
                "tools/call",
                json!({"name":"unica.check", "arguments":{"mode":"over-task"}, "_meta":modern_tasks_meta()}),
            ),
            (
                6,
                "tasks/get",
                json!({"taskId":over_task.to_string(), "_meta":modern_tasks_meta()}),
            ),
        ] {
            client
                .send(json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params}))
                .await;
            let response = client.receive().await;
            if id != 5 {
                over_codes.push(response["error"]["data"]["code"].clone());
            }
        }
        assert_eq!(
            over_codes,
            vec![json!("result_too_large"), json!("result_too_large")]
        );
        assert_eq!(executions.load(Ordering::SeqCst), 4);
        client.shutdown().await;
    }

    async fn tasks_hooks_closed_errors_case() {
        use crate::domain::invocation::{InvocationStatus, TaskId};
        use std::str::FromStr;
        use std::sync::atomic::AtomicUsize;

        let known = TaskId::new();
        let unknown = TaskId::new();
        let expired = TaskId::new();
        let mismatched = TaskId::new();
        let lookup_count = Arc::new(AtomicUsize::new(0));
        let lookup_observed = Arc::clone(&lookup_count);
        let get: Arc<CanonicalTaskHandler> = Arc::new(move |task_id, _| {
            lookup_observed.fetch_add(1, Ordering::SeqCst);
            if task_id == unknown {
                Err(V5TaskExchangeError::Protocol(
                    V5DaemonErrorCode::TaskNotFound,
                ))
            } else if task_id == expired {
                Err(V5TaskExchangeError::Protocol(
                    V5DaemonErrorCode::TaskExpired,
                ))
            } else {
                Ok(canonical_snapshot(known, InvocationStatus::Working, None))
            }
        });
        let cancellations = Arc::new(AtomicUsize::new(0));
        let cancellation_observed = Arc::clone(&cancellations);
        let cancel: Arc<CanonicalTaskHandler> = Arc::new(move |_, _| {
            cancellation_observed.fetch_add(1, Ordering::SeqCst);
            Ok(canonical_snapshot(known, InvocationStatus::Cancelled, None))
        });
        let call: Arc<CanonicalCallHandler> = Arc::new(move |_, _, _, _, _| {
            Ok(CanonicalCallOutcome::Task(canonical_snapshot(
                known,
                InvocationStatus::Working,
                None,
            )))
        });
        let server = UnicaServer::with_canonical_v13_tasks(call, get, cancel);
        let (mut client, _) = spawn_unica_server(server);

        for (id, method, task_id, expected_code) in [
            (1, "tasks/get", unknown.to_string(), "task_not_found"),
            (2, "tasks/get", expired.to_string(), "task_expired"),
            (
                3,
                "tasks/get",
                "not-a-canonical-uuid".into(),
                "invalid_task_id",
            ),
            (4, "tasks/update", unknown.to_string(), "task_not_found"),
            (
                5,
                "tasks/update",
                known.to_string(),
                "task_input_not_supported",
            ),
            (
                8,
                "tasks/get",
                mismatched.to_string(),
                "task_protocol_failed",
            ),
        ] {
            let mut params = json!({"taskId": task_id, "_meta": modern_tasks_meta()});
            if method == "tasks/update" {
                params["inputResponses"] = json!({});
            }
            client
                .send(json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params}))
                .await;
            let response = client.receive().await;
            let expected_jsonrpc = if expected_code == "task_protocol_failed" {
                -32603
            } else {
                -32602
            };
            assert_eq!(response["error"]["code"], expected_jsonrpc, "{response}");
            assert_eq!(
                response["error"]["data"]["code"], expected_code,
                "{response}"
            );
        }
        assert!(TaskId::from_str("not-a-canonical-uuid").is_err());

        for id in [6, 7] {
            client
                .send(json!({
                    "jsonrpc":"2.0", "id":id, "method":"tasks/cancel",
                    "params":{"taskId":known.to_string(), "_meta":modern_tasks_meta()}
                }))
                .await;
            let response = client.receive().await;
            assert_eq!(response["result"]["resultType"], "complete", "{response}");
        }
        assert_eq!(cancellations.load(Ordering::SeqCst), 2);
        assert_eq!(lookup_count.load(Ordering::SeqCst), 5);
        client.shutdown().await;
    }

    #[tokio::test]
    async fn tasks_direct_first_capability_controls_native_projection_without_reexecution() {
        tasks_direct_first_capability_case().await;
    }

    #[tokio::test]
    async fn legacy_initialized_session_cannot_escalate_tasks_with_modern_request_metadata() {
        legacy_initialized_session_cannot_escalate_tasks_per_request_case().await;
    }

    #[tokio::test]
    async fn modern_initialized_session_can_use_negotiated_native_tasks() {
        modern_initialized_session_retains_native_tasks_case().await;
    }

    #[tokio::test]
    async fn native_task_methods_preserve_one_frontend_transport_cutoff() {
        native_task_methods_preserve_one_frontend_transport_cutoff_case().await;
    }

    #[tokio::test]
    async fn tasks_direct_and_completed_get_use_the_same_call_result_renderer() {
        tasks_direct_and_completed_get_case().await;
    }

    #[tokio::test]
    async fn tasks_projection_rejects_reverse_durable_timestamps_on_wire() {
        tasks_projection_rejects_reverse_timestamps_on_wire_case().await;
    }

    #[tokio::test]
    async fn tasks_projection_bounds_near_limit_wire_and_rejects_over_limit() {
        tasks_projection_keeps_near_limit_wire_bounded_and_rejects_over_limit_case().await;
    }

    #[tokio::test]
    async fn tasks_hooks_preserve_closed_unknown_expired_invalid_and_update_semantics() {
        tasks_hooks_closed_errors_case().await;
    }

    #[tokio::test]
    async fn native_task_projection_contract_is_capability_gated_durable_and_replay_free() {
        assert!(
            !UnicaServer::legacy_for_test(application_handler())
                .get_info()
                .capabilities
                .supports_tasks(),
            "the explicit v0.12 test profile must not advertise Tasks"
        );
        tasks_direct_first_capability_case().await;
        legacy_initialized_session_cannot_escalate_tasks_per_request_case().await;
        modern_initialized_session_retains_native_tasks_case().await;
        native_task_methods_preserve_one_frontend_transport_cutoff_case().await;
        tasks_direct_and_completed_get_case().await;
        tasks_projection_rejects_reverse_timestamps_on_wire_case().await;
        tasks_projection_keeps_near_limit_wire_bounded_and_rejects_over_limit_case().await;
        tasks_hooks_closed_errors_case().await;
    }

    #[tokio::test]
    async fn native_task_get_reports_late_cancel_request_without_claiming_cancellation() {
        use crate::domain::invocation::{InvocationStatus, TaskId};

        fn requested_working(task_id: TaskId) -> V5DaemonTaskSnapshot {
            let mut snapshot = canonical_snapshot(task_id, InvocationStatus::Working, None);
            let V5DaemonTaskSnapshot::Working {
                cancel_requested, ..
            } = &mut snapshot
            else {
                unreachable!()
            };
            *cancel_requested = true;
            snapshot
        }

        let task_id = TaskId::new();
        let call: Arc<CanonicalCallHandler> = Arc::new(move |_, _, _, _, _| {
            Ok(CanonicalCallOutcome::Task(canonical_snapshot(
                task_id,
                InvocationStatus::Working,
                None,
            )))
        });
        let get: Arc<CanonicalTaskHandler> = Arc::new(move |_, _| Ok(requested_working(task_id)));
        let cancel: Arc<CanonicalTaskHandler> =
            Arc::new(move |_, _| Ok(requested_working(task_id)));
        let (mut client, _) =
            spawn_unica_server(UnicaServer::with_canonical_v13_tasks(call, get, cancel));

        client
            .send(json!({
                "jsonrpc":"2.0", "id":1, "method":"tools/call",
                "params":{"name":"unica.check", "arguments":{}, "_meta":modern_tasks_meta()}
            }))
            .await;
        let seed = client.receive().await;
        assert_eq!(seed["result"]["status"], "working", "{seed}");
        assert!(seed["result"].get("statusMessage").is_none(), "{seed}");

        client
            .send(json!({
                "jsonrpc":"2.0", "id":2, "method":"tasks/cancel",
                "params":{"taskId":task_id.to_string(), "_meta":modern_tasks_meta()}
            }))
            .await;
        let cancelled = client.receive().await;
        assert_eq!(cancelled["result"]["resultType"], "complete", "{cancelled}");
        client
            .send(json!({
                "jsonrpc":"2.0", "id":3, "method":"tasks/get",
                "params":{"taskId":task_id.to_string(), "_meta":modern_tasks_meta()}
            }))
            .await;
        let task = client.receive().await;
        assert_eq!(task["result"]["status"], "working", "{task}");
        assert_eq!(
            task["result"]["statusMessage"],
            "Cancellation was requested; check task status and result for the actual outcome",
            "{task}"
        );
        client.shutdown().await;
    }

    #[tokio::test]
    async fn legacy_offer_2025_11_25_is_echoed() {
        let (mut client, _) = spawn_server(application_handler());
        client
            .send(json!({
                "jsonrpc": "2.0",
                "id": 0,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-11-25",
                    "capabilities": {},
                    "clientInfo": {"name": "unica-tests", "version": "1"}
                }
            }))
            .await;
        let response = client.receive().await;
        assert_eq!(response["result"]["protocolVersion"], "2025-11-25");
        client.shutdown().await;
    }

    #[tokio::test]
    async fn legacy_unknown_offer_falls_back_to_pinned_version() {
        // The fallback is pinned to 2025-11-25 explicitly; an SDK bump that
        // moves `ProtocolVersion::LATEST` must not move this answer.
        let (mut client, _) = spawn_server(application_handler());
        client
            .send(json!({
                "jsonrpc": "2.0",
                "id": 0,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2099-01-01",
                    "capabilities": {},
                    "clientInfo": {"name": "unica-tests", "version": "1"}
                }
            }))
            .await;
        let response = client.receive().await;
        assert_eq!(response["result"]["protocolVersion"], "2025-11-25");
        client.shutdown().await;
    }

    #[tokio::test]
    async fn legacy_session_responses_stay_legacy_shaped() {
        let (mut client, _) = spawn_server(application_handler());
        client.initialize().await;
        client
            .send(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {} }))
            .await;
        let response = client.receive().await;
        assert!(response["result"]["tools"].is_array());
        assert!(
            response["result"].get("resultType").is_none(),
            "legacy sessions must not receive modern result fields"
        );
        client.shutdown().await;
    }

    #[tokio::test]
    async fn modern_meta_inside_legacy_session_keeps_the_session_model() {
        // SDK semantics, pinned as observed: a request that declares full
        // modern `_meta` inside an `initialize` session gets a modern-shaped
        // response for itself, while the session is not switched — the next
        // plain request keeps the legacy wire shape.
        let (mut client, _) = spawn_server(application_handler());
        client.initialize().await;
        client
            .send(json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/list",
                "params": { "_meta": modern_meta() }
            }))
            .await;
        let modern_shaped = client.receive().await;
        // Modern semantics follow the request's effective encoding, pagination
        // included: the first page plus a continuation cursor.
        assert_eq!(
            modern_shaped["result"]["tools"].as_array().unwrap().len(),
            TOOLS_PAGE_SIZE
        );
        assert!(modern_shaped["result"]["nextCursor"].is_string());
        assert_eq!(modern_shaped["result"]["resultType"], "complete");
        client
            .send(json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {} }))
            .await;
        let plain = client.receive().await;
        assert!(
            plain["result"].get("resultType").is_none(),
            "a plain request after a modern-declared one stays legacy-shaped"
        );
        client.shutdown().await;
    }

    #[tokio::test]
    async fn tools_list_rejects_any_presented_cursor() {
        let (mut client, _) = spawn_server(application_handler());
        client.initialize().await;
        client
            .send(json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/list",
                "params": { "cursor": "anything" }
            }))
            .await;
        let response = client.receive().await;
        assert_eq!(response["error"]["code"], -32602);
        client.shutdown().await;
    }

    #[tokio::test]
    async fn modern_discover_can_open_the_connection() {
        let (mut client, _) = spawn_unica_server(canonical_profile_server());
        client
            .send(json!({
                "jsonrpc": "2.0",
                "id": 0,
                "method": "server/discover",
                "params": { "_meta": modern_meta() }
            }))
            .await;
        let response = client.receive().await;
        let result = &response["result"];
        assert_eq!(result["resultType"], "complete");
        let supported: Vec<&str> = result["supportedVersions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        // The served set is exactly the guaranteed host matrix — nothing older.
        assert_eq!(supported, ["2025-06-18", "2025-11-25", "2026-07-28"]);
        assert!(result["ttlMs"].is_number());
        assert!(result["cacheScope"].is_string());
        assert_eq!(
            result["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
            "unica"
        );
        client
            .send(json!({
                "jsonrpc": "2.0", "id": 1, "method": "tools/list",
                "params": { "_meta": modern_tasks_meta() }
            }))
            .await;
        let listed = client.receive().await;
        assert_eq!(listed["result"]["resultType"], "complete", "{listed}");
        let names = listed["result"]["tools"]
            .as_array()
            .expect("modern tools/list must succeed")
            .iter()
            .map(|tool| tool["name"].as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        assert_v13_profile_names(&names, true);
        client.shutdown().await;
    }

    #[tokio::test]
    async fn discovery_probe_response_matches_unwrapped_sdk_bytes() {
        let probe = json!({
            "jsonrpc": "2.0", "id": "probe", "method": "server/discover",
            "params": { "_meta": modern_meta() }
        });
        let (mut wrapped, _) = spawn_unica_server(canonical_profile_server());
        let mut sdk = spawn_unwrapped_unica_server(canonical_profile_server());
        wrapped.send(probe.clone()).await;
        sdk.send(probe).await;
        assert_eq!(wrapped.receive_raw().await, sdk.receive_raw().await);
        wrapped.shutdown().await;
        sdk.shutdown().await;
    }

    #[tokio::test]
    async fn discover_probe_followed_by_legacy_initialize_serves_compatibility_tools() {
        let (mut client, _) = spawn_unica_server(canonical_profile_server());
        client
            .send(json!({
                "jsonrpc": "2.0", "id": 0, "method": "server/discover",
                "params": { "_meta": modern_meta() }
            }))
            .await;
        let discovered = client.receive().await;
        assert_eq!(
            discovered["result"]["resultType"], "complete",
            "{discovered}"
        );
        client
            .send(json!({
                "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": {
                    "protocolVersion": "2025-11-25",
                    "capabilities": {},
                    "clientInfo": {"name": "probing-host", "version": "1"}
                }
            }))
            .await;
        let initialized = client.receive().await;
        assert_eq!(initialized["result"]["protocolVersion"], "2025-11-25");
        assert!(
            initialized["result"]["capabilities"]["extensions"][TASKS_EXTENSION_ID].is_null(),
            "legacy initialize must not negotiate Tasks: {initialized}"
        );
        client
            .send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
            .await;
        client
            .send(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}))
            .await;
        let listed = client.receive().await;
        let names = listed["result"]["tools"]
            .as_array()
            .expect("a legacy tools/list must succeed")
            .iter()
            .map(|tool| tool["name"].as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        assert_v13_profile_names(&names, false);
        assert!(listed["result"].get("resultType").is_none());
        assert_v13_profile_names(
            &listed_tool_names(&mut client, 3, Some(modern_tasks_meta())).await,
            false,
        );
        client
            .send(json!({
                "jsonrpc": "2.0", "id": 4, "method": "tasks/get",
                "params": {"taskId": "not-a-task"}
            }))
            .await;
        let unavailable = client.receive().await;
        assert_eq!(unavailable["error"]["code"], -32021, "{unavailable}");
        client.shutdown().await;
    }

    #[tokio::test]
    async fn queued_ping_discovery_and_legacy_initialize_keep_every_frame() {
        let (mut client, _) = spawn_unica_server(canonical_profile_server());
        let frames = [
            json!({"jsonrpc": "2.0", "id": 0, "method": "ping"}),
            json!({
                "jsonrpc": "2.0", "id": 1, "method": "server/discover",
                "params": {"_meta": modern_meta()}
            }),
            json!({
                "jsonrpc": "2.0", "id": 2, "method": "initialize",
                "params": {
                    "protocolVersion": "2025-11-25",
                    "capabilities": {},
                    "clientInfo": {"name": "batched-host", "version": "1"}
                }
            }),
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            json!({"jsonrpc": "2.0", "id": 3, "method": "tools/list", "params": {}}),
        ];
        let batch = frames
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        client.writer.write_all(batch.as_bytes()).await.unwrap();
        client.writer.flush().await.unwrap();
        for id in 0..=3 {
            let response = client.receive().await;
            assert_eq!(response["id"], id, "lost or duplicated frame: {response}");
            assert!(response.get("error").is_none(), "{response}");
        }
        client
            .send(json!({"jsonrpc": "2.0", "id": 4, "method": "tools/list", "params": {}}))
            .await;
        assert_eq!(client.receive().await["id"], 4);
        client.shutdown().await;
    }

    #[tokio::test]
    async fn unsupported_discovery_version_still_uses_sdk_error() {
        let (mut client, _) = spawn_unica_server(canonical_profile_server());
        client
            .send(json!({
                "jsonrpc": "2.0", "id": 0, "method": "server/discover",
                "params": {"_meta": {
                    "io.modelcontextprotocol/protocolVersion": "2099-01-01",
                    "io.modelcontextprotocol/clientCapabilities": {}
                }}
            }))
            .await;
        let response = client.receive().await;
        assert_eq!(response["error"]["code"], -32022, "{response}");
        client.shutdown().await;
    }

    #[tokio::test]
    async fn modern_direct_first_tools_list_pages_through_the_full_registry() {
        // Modern peers page the registry (25 per page, offset cursors);
        // walking every page must reproduce the complete surface exactly.
        let (mut client, _) = spawn_server(application_handler());
        let mut names = Vec::new();
        let mut cursor: Option<String> = None;
        let mut id = 0;
        loop {
            let mut params = json!({ "_meta": modern_meta() });
            if let Some(cursor) = &cursor {
                params["cursor"] = json!(cursor);
            }
            client
                .send(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "method": "tools/list",
                    "params": params
                }))
                .await;
            let response = client.receive().await;
            assert_eq!(response["result"]["resultType"], "complete");
            let tools = response["result"]["tools"].as_array().unwrap();
            assert!(
                tools.len() <= TOOLS_PAGE_SIZE,
                "page overflow: {}",
                tools.len()
            );
            assert!(
                tools.iter().all(|tool| tool.get("description").is_none()),
                "the schema-only baseline holds on the modern branch too"
            );
            names.extend(
                tools
                    .iter()
                    .map(|tool| tool["name"].as_str().unwrap().to_string()),
            );
            match response["result"]["nextCursor"].as_str() {
                Some(next) => cursor = Some(next.to_string()),
                None => break,
            }
            id += 1;
        }
        let registry_size = crate::application::tools().len();
        assert_eq!(names.len(), registry_size);
        let unique: HashSet<&String> = names.iter().collect();
        assert_eq!(unique.len(), registry_size, "pages must not overlap");
        client.shutdown().await;
    }

    #[tokio::test]
    async fn modern_tools_list_rejects_a_cursor_the_server_never_issued() {
        let (mut client, _) = spawn_server(application_handler());
        for (id, bad) in ["banana", "7", "0", "10000"].into_iter().enumerate() {
            let mut params = json!({ "_meta": modern_meta() });
            params["cursor"] = json!(bad);
            client
                .send(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "method": "tools/list",
                    "params": params
                }))
                .await;
            let response = client.receive().await;
            assert_eq!(
                response["error"]["code"], -32602,
                "cursor {bad}: {response}"
            );
        }
        client.shutdown().await;
    }

    #[tokio::test]
    async fn modern_unknown_version_direct_first_gets_unsupported_error() {
        let (mut client, _) = spawn_server(application_handler());
        client
            .send(json!({
                "jsonrpc": "2.0",
                "id": 0,
                "method": "tools/list",
                "params": { "_meta": {
                    "io.modelcontextprotocol/protocolVersion": "2099-01-01",
                    "io.modelcontextprotocol/clientCapabilities": {}
                } }
            }))
            .await;
        let response = client.receive().await;
        assert_eq!(response["error"]["code"], -32022);
        let supported = response["error"]["data"]["supported"].to_string();
        assert!(supported.contains("2025-11-25"), "got {supported}");
        client.shutdown().await;
    }

    #[tokio::test]
    async fn modern_partial_meta_opener_is_rejected_before_serving() {
        // A direct-first request with an incomplete reserved set is not a
        // silent legacy downgrade: admission refuses the connection.
        for method in ["tools/list", "server/discover"] {
            let (client_io, server_io) = tokio::io::duplex(4 * 1024 * 1024);
            let server = UnicaServer::legacy_for_test(application_handler());
            let handle = tokio::spawn(async move {
                let (read, write) = tokio::io::split(server_io);
                let transport =
                    rmcp::transport::async_rw::AsyncRwTransport::new_server(read, write);
                let transport = DiscoveryProbeTransport::new(transport, &server);
                server
                    .serve(transport)
                    .await
                    .map(|_| ())
                    .map_err(|error| error.to_string())
            });
            let (read_half, mut writer) = tokio::io::split(client_io);
            let mut reader = BufReader::new(read_half).lines();
            let mut line = json!({
                "jsonrpc": "2.0",
                "id": 0,
                "method": method,
                "params": { "_meta": {
                    "io.modelcontextprotocol/protocolVersion": "2026-07-28"
                } }
            })
            .to_string();
            line.push('\n');
            writer.write_all(line.as_bytes()).await.unwrap();
            writer.flush().await.unwrap();
            let next = timeout(TEST_STEP, reader.next_line())
                .await
                .expect("timed out waiting for admission verdict")
                .expect("MCP transport failed");
            assert!(
                next.is_none(),
                "{method}: admission must close without serving, got {next:?}"
            );
            let outcome = handle.await.unwrap();
            let error = outcome.expect_err("admission failure surfaces as a serve error");
            assert!(
                error.to_lowercase().contains("initialize"),
                "{method}: unexpected admission error: {error}"
            );
        }
    }

    #[tokio::test]
    async fn progress_token_receives_typed_search_snapshot_before_result() {
        let handler: Arc<ToolCallHandler> = Arc::new(|_, _, _, progress| {
            let snapshot = crate::domain::code_intelligence::SearchProgressSnapshot {
                schema_version: 1,
                elapsed_ms: 5,
                deadline_ms: 300_000,
                next_update_within_ms: 2_000,
                providers: vec![crate::domain::code_intelligence::SearchProviderProgress {
                    identity: crate::domain::code_intelligence::ProviderId::GitGrep.identity(),
                    state: crate::domain::code_intelligence::SearchProviderState::Running,
                    phase: crate::domain::code_intelligence::SearchProviderPhase::Searching,
                    detail_code: None,
                    results_found: 2,
                }],
            };
            progress.publish(snapshot.to_progress_event());
            Ok(code_search_test_result())
        });
        let (mut client, _) = spawn_server(handler);
        client.initialize().await;
        client
            .send(json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {
                    "_meta": {"progressToken": "search-17"},
                    "name": "unica.code.search",
                    "arguments": {}
                }
            }))
            .await;

        let notification = client.receive().await;
        assert_eq!(notification["method"], "notifications/progress");
        assert_eq!(notification["params"]["progressToken"], "search-17");
        assert_eq!(notification["params"]["progress"], 0.0);
        assert_eq!(notification["params"]["total"], 1.0);
        assert_eq!(
            notification["params"]["_meta"]["io.unica/searchProgress"]["providers"][0]["role"],
            "lexical"
        );
        let response = client.receive().await;
        assert_eq!(response["id"], 1);
        assert_eq!(
            response["result"]["structuredContent"]["data"]["coverage"],
            "partial"
        );
        client.shutdown().await;
    }

    #[tokio::test]
    async fn retained_progress_sink_does_not_hold_the_tool_response_open() {
        let retained = Arc::new(Mutex::new(None::<Arc<dyn ProgressSink>>));
        let retained_by_handler = Arc::clone(&retained);
        let handler: Arc<ToolCallHandler> = Arc::new(move |_, _, _, progress| {
            *retained_by_handler.lock().unwrap() = Some(progress);
            Ok(code_search_test_result())
        });
        let (mut client, _) = spawn_server(handler);
        client.initialize().await;
        client
            .send(json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {
                    "_meta": {"progressToken": "search-retained"},
                    "name": "unica.code.search",
                    "arguments": {}
                }
            }))
            .await;

        let line = timeout(Duration::from_millis(500), client.reader.next_line())
            .await
            .expect("a retained progress sink must not delay the tool response")
            .expect("MCP transport failed")
            .expect("MCP server closed the stream before responding");
        let response: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(response["id"], 1);

        retained.lock().unwrap().take();
        client.shutdown().await;
    }

    #[tokio::test]
    async fn progress_forwarder_preserves_rapid_phase_transitions() {
        let retained = Arc::new(Mutex::new(None::<Arc<dyn ProgressSink>>));
        let retained_by_handler = Arc::clone(&retained);
        let handler: Arc<ToolCallHandler> = Arc::new(move |_, _, _, progress| {
            for (elapsed_ms, phase, detail_code) in [
                (
                    5,
                    crate::domain::code_intelligence::SearchProviderPhase::Preparing,
                    "reconcilingSources",
                ),
                (
                    6,
                    crate::domain::code_intelligence::SearchProviderPhase::Searching,
                    "executingQuery",
                ),
            ] {
                let snapshot = crate::domain::code_intelligence::SearchProgressSnapshot {
                    schema_version: 1,
                    elapsed_ms,
                    deadline_ms: 300_000,
                    next_update_within_ms: 2_000,
                    providers: vec![crate::domain::code_intelligence::SearchProviderProgress {
                        identity: crate::domain::code_intelligence::ProviderId::Rlm.identity(),
                        state: crate::domain::code_intelligence::SearchProviderState::Running,
                        phase,
                        detail_code: Some(detail_code.to_string()),
                        results_found: 0,
                    }],
                };
                progress.publish(snapshot.to_progress_event());
            }
            *retained_by_handler.lock().unwrap() = Some(progress);
            Ok(code_search_test_result())
        });
        let (mut client, _) = spawn_server(handler);
        client.initialize().await;
        client
            .send(json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {
                    "_meta": {"progressToken": "search-phases"},
                    "name": "unica.code.search",
                    "arguments": {}
                }
            }))
            .await;

        let mut messages = Vec::new();
        for _ in 0..3 {
            let line = timeout(Duration::from_millis(500), client.reader.next_line())
                .await
                .expect("every phase transition and the result must be forwarded")
                .expect("MCP transport failed")
                .expect("MCP server closed the stream before responding");
            messages.push(serde_json::from_str::<Value>(&line).unwrap());
        }
        assert_eq!(messages[0]["method"], "notifications/progress");
        assert_eq!(messages[1]["method"], "notifications/progress");
        assert_eq!(
            messages[0]["params"]["_meta"]["io.unica/searchProgress"]["providers"][0]["detailCode"],
            "reconcilingSources"
        );
        assert_eq!(
            messages[1]["params"]["_meta"]["io.unica/searchProgress"]["providers"][0]["detailCode"],
            "executingQuery"
        );
        assert_eq!(messages[2]["id"], 1);

        retained.lock().unwrap().take();
        client.shutdown().await;
    }

    #[test]
    fn tool_definitions_expose_logical_diagnostics_action_union() {
        let listed = tool_definitions(&crate::application::tools());
        let diagnostics = listed
            .iter()
            .find(|tool| tool.name == "unica.code.diagnostics")
            .expect("unica.code.diagnostics must be listed");

        let schema = diagnostics.input_schema.as_ref();
        let branches = schema["oneOf"].as_array().expect("closed action union");
        assert_eq!(branches.len(), 4);
        for branch in branches {
            let properties = branch["properties"].as_object().unwrap();
            assert!(properties.contains_key("action"));
            assert!(properties.contains_key("sourceSet"));
            assert!(properties.contains_key("cwd"));
            for legacy in ["sourceDir", "mode", "path", "codes"] {
                assert!(!properties.contains_key(legacy), "legacy field {legacy}");
            }
        }
    }

    #[test]
    fn metadata_output_schema_follows_the_registered_handler_variant() {
        let listed = tool_definitions(&[ToolSpec {
            name: "unica.meta.future",
            description: "Synthetic metadata registry entry.",
            execution: ToolExecution::Read,
            result_contract: ResultContract::Typed,
            cache_access: crate::domain::cache::CacheAccess::default(),
            handler: crate::application::ToolHandler::Metadata {
                operation: crate::application::metadata::MetadataOperation::Info,
            },
        }]);

        assert!(listed[0].output_schema.is_some());
    }

    #[test]
    fn code_search_publishes_a_closed_typed_result_schema() {
        let listed = tool_definitions(&crate::application::tools());
        let code_search = listed
            .iter()
            .find(|tool| tool.name == "unica.code.search")
            .expect("code.search must be listed");
        let output = code_search
            .output_schema
            .as_ref()
            .expect("code.search must publish outputSchema");

        assert_eq!(output["type"], "object");
        assert_eq!(output["additionalProperties"], false);
        assert!(output["required"]
            .as_array()
            .unwrap()
            .contains(&json!("data")));
        for forbidden in ["stdout", "stderr", "command", "job"] {
            assert!(output["properties"].get(forbidden).is_none());
        }
        assert_eq!(output["properties"]["data"]["additionalProperties"], false);
        assert_eq!(
            output["properties"]["data"]["required"],
            json!(["coverage", "elapsedMs", "sections"])
        );
        let section = &output["properties"]["data"]["properties"]["sections"]["items"];
        assert_eq!(section["additionalProperties"], false);
        assert!(section["required"]
            .as_array()
            .unwrap()
            .contains(&json!("searchComplete")));
        assert!(section["required"]
            .as_array()
            .unwrap()
            .contains(&json!("termination")));
        let location = &section["properties"]["hits"]["items"]["properties"]["location"];
        assert_eq!(location["oneOf"].as_array().unwrap().len(), 2);

        let schema = Value::Object(output.as_ref().clone());
        let instance = serde_json::to_value(code_search_test_result()).unwrap();
        jsonschema::validator_for(&schema)
            .expect("code.search outputSchema must compile")
            .validate(&instance)
            .expect("the serialized code.search result must satisfy its advertised schema");
    }

    #[tokio::test]
    async fn role_edit_mcp_calls_return_structured_success_and_error() {
        let handler: Arc<ToolCallHandler> = Arc::new(|name, arguments, _, _| {
            assert_eq!(name, "unica.role.edit");
            let rejected = arguments
                .get("operations")
                .and_then(Value::as_array)
                .and_then(|operations| operations.first())
                .and_then(|operation| operation.get("value"))
                .and_then(Value::as_bool)
                == Some(true);
            let mut result = successful_test_result(if rejected {
                "role edit rejected"
            } else {
                "role edit applied"
            });
            result.cache.root.clear();
            result.ok = !rejected;
            if rejected {
                result.errors.push("unsupported_right".to_string());
            }
            result.data = Some(json!({
                "metadataPath": "Role.Demo",
                "changed": !rejected,
                "effects": if rejected { json!([]) } else { json!([{
                    "operationIndex": 0,
                    "operation": "setRight",
                    "objectName": "Catalog.Demo",
                    "right": "Delete",
                    "before": true,
                    "after": false,
                    "action": "setRight",
                    "changed": true
                }]) },
                "validation": {"status": if rejected { "failed" } else { "passed" }},
                "diagnostics": if rejected { json!([{
                    "code": "unsupported_right",
                    "severity": "error",
                    "message": "right is not supported",
                    "operationIndex": 0
                }]) } else { json!([]) }
            }));
            Ok(result)
        });
        let (mut client, _) = spawn_server(handler);
        client.initialize().await;

        for (id, value, expected_error) in [(1, false, false), (2, true, true)] {
            client
                .send(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "method": "tools/call",
                    "params": {
                        "name": "unica.role.edit",
                        "arguments": {
                            "sourceSet": "main",
                            "metadataPath": "Role.Demo",
                            "operations": [{
                                "op": "setRight",
                                "objectName": "Catalog.Demo",
                                "right": "Delete",
                                "value": value
                            }]
                        }
                    }
                }))
                .await;
            let response = client.receive().await;
            assert!(response.get("error").is_none(), "{response}");
            assert_eq!(response["result"]["isError"], expected_error);
            assert_eq!(
                response["result"]["structuredContent"]["ok"],
                !expected_error
            );
            assert_eq!(
                response["result"]["structuredContent"]["data"]["metadataPath"],
                "Role.Demo"
            );
            assert_eq!(response["result"]["structuredContent"]["cache"]["root"], "");
        }
        client.shutdown().await;
    }

    #[tokio::test]
    async fn role_edit_mcp_projects_owner_matrix_rejection_with_operation_index() {
        let (mut client, _) = spawn_server(application_handler());
        client.initialize().await;
        client
            .send(json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": {
                    "name": "unica.role.edit",
                    "arguments": {
                        "sourceSet": "main",
                        "metadataPath": "Role.Demo",
                        "operations": [{
                            "op": "setRight",
                            "objectName": "DataProcessor.Worker",
                            "right": "Delete",
                            "value": false
                        }]
                    }
                }
            }))
            .await;
        let response = client.receive().await;
        assert!(response.get("error").is_none(), "{response}");
        assert_eq!(response["result"]["isError"], true);
        let structured = &response["result"]["structuredContent"];
        assert_eq!(structured["ok"], false);
        assert_eq!(structured["cache"]["root"], "");
        assert_eq!(structured["data"]["metadataPath"], "Role.Demo");
        assert_eq!(structured["data"]["validation"]["status"], "failed");
        assert_eq!(
            structured["data"]["diagnostics"][0]["code"],
            "unsupported_right"
        );
        assert_eq!(structured["data"]["diagnostics"][0]["operationIndex"], 0);
        client.shutdown().await;
    }

    #[test]
    fn no_public_tool_schema_exposes_raw_adapter_args() {
        for tool in tool_definitions(&crate::application::tools()) {
            for properties in object_schema_property_maps(&tool.input_schema) {
                assert!(
                    properties.get("args").is_none(),
                    "{} must not expose raw adapter args",
                    tool.name
                );
            }
        }
    }

    #[test]
    fn object_schema_property_maps_visit_nested_schema_nodes() {
        let schema = json!({
            "properties": {
                "object": {"properties": {"args": {"type": "string"}}},
                "array": {"items": {"properties": {"args": {"type": "string"}}}},
                "map": {"additionalProperties": {"properties": {"args": {"type": "string"}}}},
                "combinators": {
                    "allOf": [{"properties": {"args": {"type": "string"}}}],
                    "anyOf": [{"properties": {"args": {"type": "string"}}}],
                    "oneOf": [{"properties": {"args": {"type": "string"}}}],
                    "not": {"properties": {"args": {"type": "string"}}},
                    "if": {"properties": {"args": {"type": "string"}}},
                    "then": {"properties": {"args": {"type": "string"}}},
                    "else": {"properties": {"args": {"type": "string"}}},
                    "dependentSchemas": {
                        "mode": {"properties": {"args": {"type": "string"}}}
                    },
                    "definitions": {
                        "legacy": {"properties": {"args": {"type": "string"}}}
                    },
                    "$defs": {
                        "modern": {"properties": {"args": {"type": "string"}}}
                    }
                }
            }
        });
        let maps = object_schema_property_maps(schema.as_object().unwrap());

        assert_eq!(maps.len(), 14);
        assert_eq!(
            maps.into_iter()
                .filter(|properties| properties.contains_key("args"))
                .count(),
            13
        );
    }

    struct ManualDaemonOwner(Option<LiveDaemon>);

    impl ManualDaemonOwner {
        fn start(service: Arc<ManualDaemonService>) -> Self {
            Self(Some(LiveDaemon::start(service)))
        }

        fn start_with_hooks(
            service: Arc<ManualDaemonService>,
            hooks: Arc<dyn crate::infrastructure::daemon::runtime_v5::V5RuntimeHooks>,
        ) -> Self {
            Self(Some(LiveDaemon::start_with_hooks(service, Some(hooks))))
        }

        fn finish(mut self) {
            self.0.take().unwrap().finish();
        }
    }

    impl std::ops::Deref for ManualDaemonOwner {
        type Target = LiveDaemon;

        fn deref(&self) -> &Self::Target {
            self.0.as_ref().unwrap()
        }
    }

    impl Drop for ManualDaemonOwner {
        fn drop(&mut self) {
            if let Some(daemon) = self.0.take() {
                // These tests use a multithreaded Tokio runtime: its other
                // worker can finish SDK EOF cleanup while this thread joins.
                // Executor/gate guards are declared after this owner so their
                // Drop has already released all blocking work.
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    daemon.finish();
                }));
            }
        }
    }

    #[derive(Default)]
    struct ManualDaemonState {
        calls: std::collections::HashMap<String, (usize, CancellationToken)>,
        completed: std::collections::HashMap<String, bool>,
        released: HashSet<String>,
        release_all: bool,
    }

    struct ManualDaemonService {
        known_long: bool,
        state: Mutex<ManualDaemonState>,
        wake: Condvar,
        observed: tokio::sync::Notify,
    }

    impl ManualDaemonService {
        fn new(known_long: bool) -> Arc<Self> {
            Arc::new(Self {
                known_long,
                state: Mutex::new(ManualDaemonState::default()),
                wake: Condvar::new(),
                observed: tokio::sync::Notify::new(),
            })
        }

        async fn wait(&self, id: &str, completed: bool) -> bool {
            timeout(Duration::from_secs(3), async {
                loop {
                    let observed = self.observed.notified();
                    let ready = {
                        let state = self.state.lock().unwrap();
                        if completed {
                            state.completed.contains_key(id)
                        } else {
                            state.calls.contains_key(id)
                        }
                    };
                    if ready {
                        break;
                    }
                    observed.await;
                }
            })
            .await
            .is_ok()
        }

        fn release(&self, id: &str) {
            self.state.lock().unwrap().released.insert(id.to_owned());
            self.wake.notify_all();
        }

        fn release_all(&self) {
            self.state.lock().unwrap().release_all = true;
            self.wake.notify_all();
        }

        fn cancelled(&self, id: &str) -> bool {
            self.state
                .lock()
                .unwrap()
                .calls
                .get(id)
                .is_some_and(|(_, token)| token.is_cancelled())
        }

        fn executions(&self, id: &str) -> usize {
            self.state
                .lock()
                .unwrap()
                .calls
                .get(id)
                .map_or(0, |(count, _)| *count)
        }
    }

    impl crate::infrastructure::daemon::server::CanonicalInvocationService for ManualDaemonService {
        fn prepare(
            &self,
            _invocation: &crate::infrastructure::daemon::server::ActorBoundInvocation,
        ) -> Result<
            crate::application::operation_descriptors::ExecutionClass,
            Box<crate::domain::invocation::DomainResult>,
        > {
            use crate::application::operation_descriptors::{ExecutionClass, KnownLongReason};
            Ok(if self.known_long {
                ExecutionClass::KnownLong(KnownLongReason::ExternalProcess)
            } else {
                ExecutionClass::InlineCandidate
            })
        }

        fn execute(
            &self,
            invocation: &crate::infrastructure::daemon::server::ActorBoundExecution,
            cancellation: CancellationToken,
        ) -> Result<
            crate::domain::invocation::DomainResult,
            crate::domain::invocation::InvocationFailure,
        > {
            let id = invocation.arguments()["args"]["label"]
                .as_str()
                .unwrap()
                .to_owned();
            let mut state = self.state.lock().unwrap();
            let count = state.calls.get(&id).map_or(1, |(count, _)| count + 1);
            state
                .calls
                .insert(id.clone(), (count, cancellation.clone()));
            self.observed.notify_waiters();
            while !state.release_all
                && !state.released.contains(&id)
                && !cancellation.is_cancelled()
            {
                (state, _) = self
                    .wake
                    .wait_timeout(state, Duration::from_millis(10))
                    .unwrap();
            }
            let cancelled = cancellation.is_cancelled();
            state.completed.insert(id.clone(), cancelled);
            drop(state);
            self.observed.notify_waiters();
            if cancelled {
                Err(crate::domain::invocation::InvocationFailure::new(
                    "cancelled",
                    "manual call stopped at its checkpoint",
                ))
            } else {
                Ok(crate::domain::invocation::DomainResult::success(id))
            }
        }
    }

    #[derive(Default)]
    struct ManualFlushGate {
        armed: AtomicBool,
        entered: AtomicBool,
        released: AtomicBool,
        observed: tokio::sync::Notify,
        waiter: Mutex<Option<std::task::Waker>>,
        partial_write: AtomicBool,
        prefix_written: AtomicBool,
        fail_after_prefix: AtomicBool,
        panic_after_prefix: AtomicBool,
        written: Mutex<Vec<u8>>,
    }

    impl ManualFlushGate {
        async fn wait(&self) -> bool {
            timeout(Duration::from_secs(3), async {
                loop {
                    let observed = self.observed.notified();
                    if self.entered.load(Ordering::Acquire) {
                        break;
                    }
                    observed.await;
                }
            })
            .await
            .is_ok()
        }

        fn release(&self) {
            self.released.store(true, Ordering::Release);
            if let Some(waiter) = self.waiter.lock().unwrap().take() {
                waiter.wake();
            }
        }
    }

    struct ManualFlushWriter {
        writer: tokio::io::WriteHalf<tokio::io::DuplexStream>,
        gate: Arc<ManualFlushGate>,
    }

    impl tokio::io::AsyncWrite for ManualFlushWriter {
        fn poll_write(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
            bytes: &[u8],
        ) -> std::task::Poll<std::io::Result<usize>> {
            let partial = self.gate.armed.load(Ordering::Acquire)
                && self.gate.partial_write.load(Ordering::Acquire)
                && !self.gate.released.load(Ordering::Acquire);
            if partial && self.gate.prefix_written.load(Ordering::Acquire) {
                if self.gate.panic_after_prefix.load(Ordering::Acquire) {
                    panic!("injected failure after actual partial MCP output");
                }
                if self.gate.fail_after_prefix.load(Ordering::Acquire) {
                    return std::task::Poll::Ready(Err(std::io::Error::new(
                        std::io::ErrorKind::BrokenPipe,
                        "injected failure after actual partial MCP output",
                    )));
                }
                *self.gate.waiter.lock().unwrap() = Some(cx.waker().clone());
                self.gate.entered.store(true, Ordering::Release);
                self.gate.observed.notify_waiters();
                if !self.gate.released.load(Ordering::Acquire) {
                    return std::task::Poll::Pending;
                }
            }
            let bytes = if partial && !self.gate.prefix_written.load(Ordering::Acquire) {
                &bytes[..bytes.len().min(8)]
            } else {
                bytes
            };
            let result = std::pin::Pin::new(&mut self.writer).poll_write(cx, bytes);
            if let std::task::Poll::Ready(Ok(count)) = &result {
                self.gate
                    .written
                    .lock()
                    .unwrap()
                    .extend_from_slice(&bytes[..*count]);
                if partial && *count > 0 {
                    self.gate.prefix_written.store(true, Ordering::Release);
                }
            }
            result
        }

        fn poll_flush(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            if self.gate.armed.load(Ordering::Acquire)
                && !self.gate.released.load(Ordering::Acquire)
            {
                *self.gate.waiter.lock().unwrap() = Some(cx.waker().clone());
                self.gate.entered.store(true, Ordering::Release);
                self.gate.observed.notify_waiters();
                if !self.gate.released.load(Ordering::Acquire) {
                    return std::task::Poll::Pending;
                }
            }
            std::pin::Pin::new(&mut self.writer).poll_flush(cx)
        }

        fn poll_shutdown(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::pin::Pin::new(&mut self.writer).poll_shutdown(cx)
        }
    }

    struct ManualDaemonCleanup {
        service: Arc<ManualDaemonService>,
        flush: Option<Arc<ManualFlushGate>>,
        before_submit: Option<Arc<ManualReceiptGate>>,
    }

    impl Drop for ManualDaemonCleanup {
        fn drop(&mut self) {
            self.service.release_all();
            if let Some(flush) = &self.flush {
                flush.release();
            }
            if let Some(before_submit) = &self.before_submit {
                before_submit.release();
            }
        }
    }

    type ManualReceiptKeys = Arc<Mutex<Vec<crate::application::receipt_ledger::ReceiptKey>>>;

    struct ManualHandlerRelease(Arc<manual_calls::HandlerGate>);

    impl Drop for ManualHandlerRelease {
        fn drop(&mut self) {
            self.0.release();
        }
    }

    #[derive(Default)]
    struct ManualCancelReplyHooks {
        replies: AtomicUsize,
        observed: tokio::sync::Notify,
        drop_first: bool,
        gate: Option<Arc<ManualReceiptGate>>,
    }

    impl ManualCancelReplyHooks {
        async fn wait(&self, count: usize) -> bool {
            timeout(Duration::from_secs(3), async {
                loop {
                    let observed = self.observed.notified();
                    if self.replies.load(Ordering::Acquire) >= count {
                        break;
                    }
                    observed.await;
                }
            })
            .await
            .is_ok()
        }
    }

    impl crate::infrastructure::daemon::runtime_v5::V5RuntimeHooks for ManualCancelReplyHooks {
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }

        fn cancel_response_disconnect(&self) -> bool {
            let previous = self.replies.fetch_add(1, Ordering::AcqRel);
            self.observed.notify_waiters();
            if previous == 0 {
                if let Some(gate) = &self.gate {
                    gate.hold();
                }
                self.drop_first
            } else {
                false
            }
        }
    }

    #[derive(Default)]
    struct ManualReceiptGate {
        key: Mutex<Option<crate::application::receipt_ledger::ReceiptKey>>,
        released: Mutex<bool>,
        changed: Condvar,
        observed: tokio::sync::Notify,
    }

    impl ManualReceiptGate {
        fn before_submit(&self, key: &crate::application::receipt_ledger::ReceiptKey) {
            *self.key.lock().unwrap() = Some(key.clone());
            self.observed.notify_waiters();
            self.hold();
        }

        fn hold(&self) {
            let mut released = self.released.lock().unwrap();
            while !*released {
                released = self.changed.wait(released).unwrap();
            }
        }

        async fn wait(&self) -> Option<crate::application::receipt_ledger::ReceiptKey> {
            timeout(Duration::from_secs(3), async {
                loop {
                    let observed = self.observed.notified();
                    let key = self.key.lock().unwrap().clone();
                    if let Some(key) = key {
                        break key;
                    }
                    observed.await;
                }
            })
            .await
            .ok()
        }

        fn release(&self) {
            *self.released.lock().unwrap() = true;
            self.changed.notify_all();
        }
    }

    fn manual_before_submit_server(
        owner: V5DaemonProcessOwner,
        workspace: String,
        gate: Arc<ManualReceiptGate>,
    ) -> UnicaServer {
        let router = crate::interfaces::daemon_router::canonical_daemon_router_observed(
            owner,
            workspace,
            Arc::new(move |key| gate.before_submit(key)),
        );
        UnicaServer {
            router: SurfaceToolRouter::CanonicalV13(router),
            in_flight: Arc::new(InFlightRegistry::default()),
            manual_calls: Arc::new(ManualCalls::default()),
            structured_tools: HashSet::new(),
            startup_notice: None,
        }
    }

    fn manual_daemon_server(
        owner: V5DaemonProcessOwner,
        workspace: String,
    ) -> (UnicaServer, ManualReceiptKeys) {
        let keys = Arc::new(Mutex::new(Vec::new()));
        let observed = keys.clone();
        let router = crate::interfaces::daemon_router::canonical_daemon_router_observed(
            owner,
            workspace,
            Arc::new(move |key| observed.lock().unwrap().push(key.clone())),
        );
        (
            UnicaServer {
                router: SurfaceToolRouter::CanonicalV13(router),
                in_flight: Arc::new(InFlightRegistry::default()),
                manual_calls: Arc::new(ManualCalls::default()),
                structured_tools: HashSet::new(),
                startup_notice: None,
            },
            keys,
        )
    }

    fn spawn_manual_flush_server(
        server: UnicaServer,
        gate: Arc<ManualFlushGate>,
    ) -> (McpClient, Arc<InFlightRegistry>) {
        let (client_io, server_io) = tokio::io::duplex(4 * 1024 * 1024);
        let in_flight = server.in_flight();
        let server = tokio::spawn(async move {
            let (read, writer) = tokio::io::split(server_io);
            let transport = rmcp::transport::async_rw::AsyncRwTransport::new_server(
                read,
                ManualFlushWriter { writer, gate },
            );
            let transport = DiscoveryProbeTransport::new(transport, &server);
            match ObservedServer(server).serve(transport).await {
                Ok(running) => {
                    let _ = running.waiting().await;
                }
                Err(error)
                    if matches!(error.as_ref(), ServerInitializeError::ConnectionClosed(_)) => {}
                Err(error) => {
                    panic!("manual cancellation MCP fixture initialization failed: {error}")
                }
            }
        });
        let (read, writer) = tokio::io::split(client_io);
        (
            McpClient {
                writer,
                reader: BufReader::new(read).lines(),
                server,
            },
            in_flight,
        )
    }

    async fn send_manual_daemon_call(client: &mut McpClient, request_id: u64, label: &str) {
        client.send(json!({"jsonrpc": "2.0", "id": request_id, "method": "tools/call", "params": {"name": "unica.run", "arguments": {"op": "test.long-work", "args": {"label": label}}}})).await;
    }

    async fn cancel_manual_daemon_call(client: &mut McpClient, request_id: u64) {
        client.send(json!({"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": request_id, "reason": "explicit user cancel"}})).await;
    }

    #[derive(Default)]
    struct ManualWaitHooks {
        received: AtomicBool,
        observed: tokio::sync::Notify,
    }

    impl crate::infrastructure::daemon::runtime_v5::V5RuntimeHooks for ManualWaitHooks {
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        fn wait_task_received(&self, _: crate::domain::invocation::TaskId) {
            self.received.store(true, Ordering::Release);
            self.observed.notify_waiters();
        }
    }

    impl ManualWaitHooks {
        async fn wait(&self) -> bool {
            timeout(Duration::from_secs(3), async {
                loop {
                    let notified = self.observed.notified();
                    if self.received.load(Ordering::Acquire) {
                        break;
                    }
                    notified.await;
                }
            })
            .await
            .is_ok()
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn canonical_manual_cancellation_sdk_refusal_after_eof_retires_registered_request() {
        let service = ManualDaemonService::new(false);
        let daemon = ManualDaemonOwner::start(service.clone());
        let cleanup = ManualDaemonCleanup {
            service: service.clone(),
            flush: None,
            before_submit: None,
        };
        let anchor = daemon.owner();
        let (server, keys) = manual_daemon_server(daemon.owner(), daemon.workspace_hint.clone());
        let calls = server.manual_calls.clone();
        let gate = Arc::new(manual_calls::HandlerGate::default());
        let release = ManualHandlerRelease(gate.clone());
        calls.install_handler_gate(gate.clone());
        let (mut client, in_flight) = spawn_unica_server(server);
        client.initialize().await;
        // Inline modern metadata selects SDK validation, but deliberately
        // omits the required method capabilities and client identity.
        client.send(json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"unica.run","arguments":{"op":"test.long-work","args":{"label":"sdk-refused"}},"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28"}}})).await;
        let actual_dispatch = gate.wait().await;
        cancel_manual_daemon_call(&mut client, 1).await;
        client.shutdown().await;
        drop(release);
        let draining =
            tokio::task::spawn_blocking(move || drain_stdio_frontend(&in_flight, &calls));
        let drained = matches!(timeout(TEST_STEP, draining).await, Ok(Ok(true)));
        drop(cleanup);
        drop(anchor);
        daemon.finish();
        assert!(actual_dispatch && drained, "an actual SDK refusal after output close must settle registered manual ownership without a handler or receipt key");
        assert!(keys.lock().unwrap().is_empty());
        assert_eq!(service.executions("sdk-refused"), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn canonical_manual_cancellation_compatibility_wait_releases_only_observation() {
        let service = ManualDaemonService::new(true);
        let hooks = Arc::new(ManualWaitHooks::default());
        let daemon = ManualDaemonOwner::start_with_hooks(service.clone(), hooks.clone());
        let cleanup = ManualDaemonCleanup {
            service: service.clone(),
            flush: None,
            before_submit: None,
        };
        let (server, _) = manual_daemon_server(daemon.owner(), daemon.workspace_hint.clone());
        let (mut client, in_flight) = spawn_unica_server(server);
        client.initialize().await;
        send_manual_daemon_call(&mut client, 1, "wait-owned-producer").await;
        let started = service.wait("wait-owned-producer", false).await;
        let response = client.receive().await;
        let task_id = response["result"]["structuredContent"]["data"]["task"]["taskId"]
            .as_str()
            .unwrap()
            .to_owned();
        client.send(json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"unica.task.result","arguments":{"taskId":task_id,"waitMs":7000}}})).await;
        let wait_entered = hooks.wait().await;
        cancel_manual_daemon_call(&mut client, 2).await;
        let released_observation = timeout(Duration::from_secs(3), async {
            while in_flight.running() != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .is_ok();
        let producer_cancelled = service.cancelled("wait-owned-producer");
        service.release("wait-owned-producer");
        let completed = service.wait("wait-owned-producer", true).await;
        client.send(json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"unica.task.result","arguments":{"taskId":task_id,"waitMs":1000}}})).await;
        // A cancelled observation is suppressed; the next visible response
        // must belong to the new result request for the same actual task.
        let result = client.receive().await;
        client.send(json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"unica.task.get","arguments":{"taskId":task_id}}})).await;
        let terminal_state = client.receive().await;
        client.send(json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"unica.task.result","arguments":{"taskId":task_id,"waitMs":0}}})).await;
        let replay = client.receive().await;
        drop(cleanup);
        client.shutdown().await;
        daemon.finish();
        assert!(
            started && wait_entered && released_observation && completed,
            "manual WAIT cancellation must interrupt only its own observation connection"
        );
        assert!(!producer_cancelled && !service.cancelled("wait-owned-producer"));
        assert_eq!(service.executions("wait-owned-producer"), 1);
        assert_eq!(result["id"], 3, "{result}");
        assert_eq!(
            result["result"]["structuredContent"]["ok"], true,
            "{result}"
        );
        assert_eq!(
            result["result"]["structuredContent"]["summary"], "wait-owned-producer",
            "{result}"
        );
        assert_eq!(terminal_state["id"], 4, "{terminal_state}");
        assert_eq!(
            terminal_state["result"]["structuredContent"]["data"]["task"]["status"], "completed",
            "{terminal_state}"
        );
        assert_eq!(replay["id"], 5, "{replay}");
        assert_eq!(replay["result"]["structuredContent"], result["result"]["structuredContent"], "completed result observation must preserve the factual receipt without another execution");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn canonical_manual_cancellation_mixed_duplicate_id_preserves_original_owner() {
        let service = ManualDaemonService::new(false);
        let daemon = ManualDaemonOwner::start(service.clone());
        let cleanup = ManualDaemonCleanup {
            service: service.clone(),
            flush: None,
            before_submit: None,
        };
        let (server, _) = manual_daemon_server(daemon.owner(), daemon.workspace_hint.clone());
        let (mut client, in_flight) = spawn_unica_server(server);
        client.initialize().await;
        send_manual_daemon_call(&mut client, 1, "duplicate-original").await;
        let started = service.wait("duplicate-original", false).await;
        client
            .send(json!({"jsonrpc":"2.0","id":1,"method":"ping"}))
            .await;
        let duplicate = client.receive().await;
        cancel_manual_daemon_call(&mut client, 1).await;
        let stopped = service.wait("duplicate-original", true).await;
        let cancelled = service.cancelled("duplicate-original");
        let settled = timeout(Duration::from_secs(3), async {
            while in_flight.running() != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .is_ok();
        client
            .send(json!({"jsonrpc":"2.0","id":99,"method":"ping"}))
            .await;
        let ping = client.receive().await;
        service.release("duplicate-reused");
        send_manual_daemon_call(&mut client, 1, "duplicate-reused").await;
        let reused = client.receive().await;
        drop(cleanup);
        client.shutdown().await;
        daemon.finish();
        assert!(started && stopped && cancelled && settled);
        assert_eq!(
            duplicate["error"]["code"], -32600,
            "a different method cannot reuse a currently owned CallTool ID"
        );
        assert_eq!(ping["id"], 99);
        assert_eq!(reused["id"], 1);
        assert_eq!(service.executions("duplicate-original"), 1);
        assert_eq!(service.executions("duplicate-reused"), 1);
        assert!(!service.cancelled("duplicate-reused"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn canonical_manual_cancellation_sdk_refusal_and_unknown_cancel_leave_id_reusable() {
        let service = ManualDaemonService::new(false);
        let daemon = ManualDaemonOwner::start(service.clone());
        let cleanup = ManualDaemonCleanup {
            service: service.clone(),
            flush: None,
            before_submit: None,
        };
        let (server, keys) = manual_daemon_server(daemon.owner(), daemon.workspace_hint.clone());
        let (mut client, _) = spawn_unica_server(server);
        client.initialize().await;
        client.send(json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"unica.run","arguments":{"op":"test.long-work","args":{"label":"rejected"}},"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28"}}})).await;
        let refused = client.receive().await;
        cancel_manual_daemon_call(&mut client, 1).await;
        cancel_manual_daemon_call(&mut client, 999).await;
        service.release("after-refusal");
        send_manual_daemon_call(&mut client, 1, "after-refusal").await;
        let reused = client.receive().await;
        drop(cleanup);
        client.shutdown().await;
        daemon.finish();
        assert_eq!(refused["error"]["code"], -32602);
        assert_eq!(reused["id"], 1);
        assert_eq!(service.executions("rejected"), 0);
        assert_eq!(service.executions("after-refusal"), 1);
        assert!(!service.cancelled("after-refusal"));
        assert_eq!(keys.lock().unwrap().len(), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn canonical_manual_cancellation_direct_first_call_reaches_executor() {
        let service = ManualDaemonService::new(false);
        let daemon = ManualDaemonOwner::start(service.clone());
        let cleanup = ManualDaemonCleanup {
            service: service.clone(),
            flush: None,
            before_submit: None,
        };
        let (server, _) = manual_daemon_server(daemon.owner(), daemon.workspace_hint.clone());
        let (mut client, _) = spawn_unica_server(server);
        client.send(json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"unica.run","arguments":{"op":"test.long-work","args":{"label":"direct-first"}},"_meta":modern_meta()}})).await;
        let started = service.wait("direct-first", false).await;
        cancel_manual_daemon_call(&mut client, 1).await;
        let stopped = service.wait("direct-first", true).await;
        let cancelled = service.cancelled("direct-first");
        drop(cleanup);
        client.shutdown().await;
        daemon.finish();
        assert!(started && stopped && cancelled, "manual cancellation must also reach a canonical call that opens a modern session without initialize");
        assert_eq!(service.executions("direct-first"), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn canonical_manual_cancellation_eof_before_handler_preserves_registered_intent() {
        use crate::infrastructure::daemon::protocol_v5::{
            V5InvocationPhase, V5InvocationResponse, V5ServerResponse,
        };
        let service = ManualDaemonService::new(false);
        let daemon = ManualDaemonOwner::start(service.clone());
        let receipt_gate = Arc::new(ManualReceiptGate::default());
        let cleanup = ManualDaemonCleanup {
            service: service.clone(),
            flush: None,
            before_submit: Some(receipt_gate.clone()),
        };
        let handler_gate = Arc::new(manual_calls::HandlerGate::default());
        let handler_release = ManualHandlerRelease(handler_gate.clone());
        let mut observer = daemon.owner();
        let server = manual_before_submit_server(
            daemon.owner(),
            daemon.workspace_hint.clone(),
            receipt_gate.clone(),
        );
        server
            .manual_calls
            .install_handler_gate(handler_gate.clone());
        let (mut client, _) = spawn_unica_server(server);
        client.initialize().await;
        send_manual_daemon_call(&mut client, 1, "delayed-handler").await;
        let registered_before_handler = handler_gate.wait().await;
        cancel_manual_daemon_call(&mut client, 1).await;
        client.shutdown().await;
        drop(handler_release);
        let key = receipt_gate.wait().await;
        let cancel_reserved = key.as_ref().is_some_and(|key| {
            matches!(
                observer.recover_invocation_receipt_before(
                    key.clone(),
                    Instant::now() + Duration::from_secs(3)
                ),
                Ok(V5ServerResponse::Invocation {
                    outcome: V5InvocationResponse::ReceiptPending {
                        phase: V5InvocationPhase::CancelReserved,
                        cancel_requested: true,
                        ..
                    }
                })
            )
        });
        drop(cleanup);
        let mut cancelled_terminal = false;
        if let Some(key) = key {
            let cancelled_digest = crate::application::receipt_ledger::canonical_v5_terminal(
                &crate::application::receipt_ledger::ReceiptTerminalOutcome::Cancelled,
            )
            .unwrap()
            .digest()
            .clone();
            let watchdog = Instant::now() + Duration::from_secs(3);
            while Instant::now() < watchdog {
                if matches!(observer.recover_invocation_receipt_before(key.clone(), watchdog), Ok(V5ServerResponse::Invocation { outcome: V5InvocationResponse::Direct { ref receipt } }) if receipt.receipt_key() == &key && receipt.terminal() == &crate::application::receipt_ledger::ReceiptTerminalOutcome::Cancelled && receipt.terminal_digest() == &cancelled_digest)
                {
                    cancelled_terminal = true;
                    break;
                }
                if matches!(observer.recover_invocation_receipt_before(key.clone(), watchdog), Ok(V5ServerResponse::Invocation { outcome: V5InvocationResponse::Acknowledged { ref acknowledgement } }) if acknowledgement.receipt_key() == &key && acknowledgement.terminal_digest() == &cancelled_digest)
                {
                    cancelled_terminal = true;
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
        drop(observer);
        daemon.finish();
        assert!(registered_before_handler && cancel_reserved && cancelled_terminal, "output close cannot discard an accepted manual request before its real handler binds the original receipt key");
        assert_eq!(service.executions("delayed-handler"), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn canonical_manual_cancellation_eof_waits_for_accepted_control_confirmation() {
        let service = ManualDaemonService::new(false);
        let gate = Arc::new(ManualReceiptGate::default());
        let hooks = Arc::new(ManualCancelReplyHooks {
            gate: Some(gate.clone()),
            ..Default::default()
        });
        let daemon = ManualDaemonOwner::start_with_hooks(service.clone(), hooks.clone());
        let cleanup = ManualDaemonCleanup {
            service: service.clone(),
            flush: None,
            before_submit: Some(gate.clone()),
        };
        let anchor = daemon.owner();
        let (server, _) = manual_daemon_server(daemon.owner(), daemon.workspace_hint.clone());
        let manual_calls = server.manual_calls.clone();
        let (mut client, in_flight) = spawn_unica_server(server);
        client.initialize().await;
        send_manual_daemon_call(&mut client, 1, "manual-then-eof").await;
        let started = service.wait("manual-then-eof", false).await;
        cancel_manual_daemon_call(&mut client, 1).await;
        let control_committed = hooks.wait(1).await;
        let stopped = service.wait("manual-then-eof", true).await;
        client.shutdown().await;
        let (shutdown_done, mut shutdown_result) = tokio::sync::oneshot::channel();
        let draining = tokio::task::spawn_blocking(move || {
            let drained = drain_stdio_frontend(&in_flight, &manual_calls);
            let _ = shutdown_done.send(drained);
        });
        // The ordinary EOF grace can elapse, but it cannot finish the
        // frontend while an already accepted exact control lacks its answer.
        let premature_shutdown = timeout(
            EOF_CANCELLATION_GRACE + Duration::from_millis(200),
            &mut shutdown_result,
        )
        .await
        .is_ok();
        gate.release();
        let settled_shutdown = if premature_shutdown {
            true
        } else {
            matches!(timeout(TEST_STEP, shutdown_result).await, Ok(Ok(true)))
        };
        let joined = timeout(TEST_STEP, draining).await;
        assert!(
            matches!(joined, Ok(Ok(()))),
            "frontend drain must actually return after control release"
        );
        drop(cleanup);
        drop(anchor);
        daemon.finish();
        assert!(started && control_committed && stopped && service.cancelled("manual-then-eof"));
        assert!(
            settled_shutdown,
            "accepted control must settle after its actual answer"
        );
        assert!(!premature_shutdown, "run_stdio must retain the already accepted manual control until its exact acknowledgement or genuine failure settles");
        assert_eq!(service.executions("manual-then-eof"), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn canonical_manual_cancellation_normal_response_does_not_cancel_daemon_operation() {
        let service = ManualDaemonService::new(false);
        let daemon = ManualDaemonOwner::start(service.clone());
        let cleanup = ManualDaemonCleanup {
            service: service.clone(),
            flush: None,
            before_submit: None,
        };
        let (server, _) = manual_daemon_server(daemon.owner(), daemon.workspace_hint.clone());
        let (mut client, _) = spawn_unica_server(server);
        client.initialize().await;
        send_manual_daemon_call(&mut client, 1, "normal").await;
        let entered = service.wait("normal", false).await;
        service.release("normal");
        let response = client.receive().await;
        client.shutdown().await;
        drop(cleanup);
        daemon.finish();
        assert!(entered);
        assert_eq!(response["id"], 1);
        assert_eq!(response["result"]["structuredContent"]["summary"], "normal");
        assert!(
            !service.cancelled("normal"),
            "SDK normal response cancellation must not become manual daemon cancellation"
        );
        assert_eq!(service.executions("normal"), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn canonical_manual_cancellation_before_task_id_stops_only_target_and_preserves_ping() {
        let service = ManualDaemonService::new(false);
        let daemon = ManualDaemonOwner::start(service.clone());
        let cleanup = ManualDaemonCleanup {
            service: service.clone(),
            flush: None,
            before_submit: None,
        };
        let (server, _) = manual_daemon_server(daemon.owner(), daemon.workspace_hint.clone());
        let (mut client, _) = spawn_unica_server(server);
        client.initialize().await;
        send_manual_daemon_call(&mut client, 1, "target").await;
        let target_started = service.wait("target", false).await;
        send_manual_daemon_call(&mut client, 2, "neighbor").await;
        let neighbor_started = service.wait("neighbor", false).await;
        // Both calls are still held inline. The host has received no TaskID.
        cancel_manual_daemon_call(&mut client, 1).await;
        client
            .send(json!({"jsonrpc": "2.0", "id": 99, "method": "ping"}))
            .await;
        let ping = client.receive().await;
        let target_stopped = service.wait("target", true).await;
        let target_cancelled = service.cancelled("target");
        let neighbor_cancelled = service.cancelled("neighbor");
        drop(cleanup);
        client.shutdown().await;
        daemon.finish();
        assert!(target_started && neighbor_started);
        assert_eq!(
            ping["id"], 99,
            "ping must remain responsive before target TaskID"
        );
        assert!(
            target_stopped && target_cancelled,
            "explicit MCP cancellation never reached the actual inline daemon executor"
        );
        assert!(!neighbor_cancelled);
        assert_eq!(service.executions("target"), 1);
        assert_eq!(service.executions("neighbor"), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn canonical_manual_cancellation_eof_preserves_accepted_work_and_exact_recovery_without_replay(
    ) {
        let service = ManualDaemonService::new(true);
        let daemon = ManualDaemonOwner::start(service.clone());
        let cleanup = ManualDaemonCleanup {
            service: service.clone(),
            flush: None,
            before_submit: None,
        };
        let observer_owner = daemon.owner();
        let (server, keys) = manual_daemon_server(daemon.owner(), daemon.workspace_hint.clone());
        let (mut client, _) = spawn_unica_server(server);
        client.initialize().await;
        send_manual_daemon_call(&mut client, 1, "eof-work").await;
        let started = service.wait("eof-work", false).await;
        // Close stdin without any notifications/cancelled. A separate owner
        // keeps daemon idle lifetime out of this cancellation test.
        client.shutdown().await;
        let cancelled_on_eof = service.cancelled("eof-work");
        service.release("eof-work");
        let completed = service.wait("eof-work", true).await;
        let key = keys.lock().unwrap().first().cloned();
        let mut recovered = None;
        let watchdog = Instant::now() + Duration::from_secs(3);
        let recovery_peer = observer_owner.connect_peer_before(watchdog);
        if let (Some(key), Ok(mut recovery_peer)) = (key, recovery_peer) {
            loop {
                let Ok(response) =
                    recovery_peer.recover_invocation_receipt_before(key.clone(), watchdog)
                else {
                    break;
                };
                if matches!(
                    &response,
                    crate::infrastructure::daemon::protocol_v5::V5ServerResponse::Invocation {
                        outcome:
                            crate::infrastructure::daemon::protocol_v5::V5InvocationResponse::Task {
                                snapshot: V5DaemonTaskSnapshot::Completed { .. }
                            }
                    }
                ) {
                    recovered = Some(response);
                    break;
                }
                if Instant::now() >= watchdog {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
        drop(cleanup);
        drop(observer_owner);
        daemon.finish();
        assert!(started && completed);
        assert!(!cancelled_on_eof && !service.cancelled("eof-work"));
        assert!(
            recovered.is_some(),
            "a new daemon connection must recover the exact accepted result after MCP EOF"
        );
        assert_eq!(service.executions("eof-work"), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn canonical_manual_cancellation_before_submit_reserves_exact_cancel_before_admission() {
        manual_before_submit_exact_cancel(false).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn canonical_manual_cancellation_lost_cancel_answer_reconciles_original_key_without_replay(
    ) {
        manual_before_submit_exact_cancel(true).await;
    }

    async fn manual_before_submit_exact_cancel(lost_reply: bool) {
        use crate::application::receipt_ledger::{canonical_v5_terminal, ReceiptTerminalOutcome};
        use crate::infrastructure::daemon::protocol_v5::{
            V5InvocationPhase, V5InvocationResponse, V5ServerResponse,
        };
        let service = ManualDaemonService::new(false);
        let hooks = Arc::new(ManualCancelReplyHooks {
            drop_first: lost_reply,
            ..Default::default()
        });
        let daemon = ManualDaemonOwner::start_with_hooks(service.clone(), hooks.clone());
        let gate = Arc::new(ManualReceiptGate::default());
        let cleanup = ManualDaemonCleanup {
            service: service.clone(),
            flush: None,
            before_submit: Some(gate.clone()),
        };
        let mut observer_owner = daemon.owner();
        let server = manual_before_submit_server(
            daemon.owner(),
            daemon.workspace_hint.clone(),
            gate.clone(),
        );
        let output = Arc::new(ManualFlushGate::default());
        let (mut client, in_flight) = spawn_manual_flush_server(server, output.clone());
        client.initialize().await;
        output.written.lock().unwrap().clear();
        send_manual_daemon_call(&mut client, 1, "before-submit").await;
        let key = gate.wait().await;
        let key_created_before_submit = key.is_some();
        let no_execution_before_submit = service.executions("before-submit") == 0;
        cancel_manual_daemon_call(&mut client, 1).await;
        let mut cancel_reserved = false;
        if let Some(key) = &key {
            let watchdog = Instant::now() + Duration::from_secs(3);
            while Instant::now() < watchdog {
                if matches!(
                    observer_owner.recover_invocation_receipt_before(key.clone(), watchdog),
                    Ok(V5ServerResponse::Invocation {
                        outcome: V5InvocationResponse::ReceiptPending {
                            phase: V5InvocationPhase::CancelReserved,
                            cancel_requested: true,
                            ..
                        }
                    })
                ) {
                    cancel_reserved = true;
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
        let exact_control_confirmed = !lost_reply || hooks.wait(2).await;
        gate.release();
        let suppressed = timeout(Duration::from_secs(3), async {
            while in_flight.running() != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .is_ok();
        let suppressed_bytes = output.written.lock().unwrap().len();
        // The cancelled response has no bytes; a distinct Ping is the next
        // complete frame, and the now released ID may serve fresh work.
        client
            .send(json!({"jsonrpc":"2.0","id":99,"method":"ping"}))
            .await;
        let ping = client.receive().await;
        service.release("reused-id");
        send_manual_daemon_call(&mut client, 1, "reused-id").await;
        let reused = client.receive().await;
        drop(cleanup);
        client.shutdown().await;
        let cancelled_digest = canonical_v5_terminal(&ReceiptTerminalOutcome::Cancelled)
            .unwrap()
            .digest()
            .clone();
        let mut exact_cancelled_terminal = false;
        if let Some(key) = &key {
            let watchdog = Instant::now() + Duration::from_secs(3);
            while Instant::now() < watchdog {
                match observer_owner.recover_invocation_receipt_before(key.clone(), watchdog) {
                    Ok(V5ServerResponse::Invocation {
                        outcome: V5InvocationResponse::Direct { receipt },
                    }) if receipt.receipt_key() == key
                        && receipt.terminal() == &ReceiptTerminalOutcome::Cancelled
                        && receipt.terminal_digest() == &cancelled_digest =>
                    {
                        exact_cancelled_terminal = true;
                        break;
                    }
                    Ok(V5ServerResponse::Invocation {
                        outcome: V5InvocationResponse::Acknowledged { acknowledgement },
                    }) if acknowledgement.receipt_key() == key
                        && acknowledgement.terminal_digest() == &cancelled_digest =>
                    {
                        exact_cancelled_terminal = true;
                        break;
                    }
                    _ => {}
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
        drop(observer_owner);
        daemon.finish();
        assert!(key_created_before_submit && no_execution_before_submit);
        assert!(cancel_reserved, "manual cancellation must reserve the exact key before the single Submit is allowed to leave the frontend");
        assert!(exact_cancelled_terminal, "the original CancelReserved key must become a cancelled terminal after its corresponding Submit");
        assert_eq!(service.executions("before-submit"), 0);
        assert!(exact_control_confirmed && suppressed);
        assert_eq!(
            suppressed_bytes, 0,
            "suppression before send must write zero bytes of the cancelled response"
        );
        assert_eq!(ping["id"], 99);
        assert_eq!(reused["id"], 1);
        assert_eq!(service.executions("reused-id"), 1);
        assert!(!service.cancelled("reused-id"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn canonical_manual_cancellation_stdio_drain_does_not_cancel_held_daemon_admission() {
        use crate::infrastructure::daemon::protocol_v5::{V5InvocationResponse, V5ServerResponse};
        let service = ManualDaemonService::new(false);
        let daemon = ManualDaemonOwner::start(service.clone());
        let cleanup = ManualDaemonCleanup {
            service: service.clone(),
            flush: None,
            before_submit: None,
        };
        let anchor = daemon.owner();
        let (server, keys) = manual_daemon_server(daemon.owner(), daemon.workspace_hint.clone());
        let (mut client, in_flight) = spawn_unica_server(server);
        client.initialize().await;
        send_manual_daemon_call(&mut client, 1, "stdio-drain").await;
        let started = service.wait("stdio-drain", false).await;
        let admission_token = in_flight
            .state
            .lock()
            .unwrap()
            .running
            .first()
            .map(|(_, token)| token.clone());
        let McpClient {
            mut writer,
            reader,
            server,
        } = client;
        writer.shutdown().await.unwrap();
        drop(writer);
        // This is the real SDK EOF boundary preceding run_stdio's drain. The
        // guided inline executor keeps the canonical admission unfinished.
        let sdk_stopped = matches!(timeout(TEST_STEP, server).await, Ok(Ok(())));
        let held_at_drain = in_flight.running() == 1;
        let draining = tokio::task::spawn_blocking(move || {
            drain_mcp_shutdown(&in_flight, EOF_CANCELLATION_GRACE)
        });
        let admission_cancelled = timeout(Duration::from_secs(3), async {
            loop {
                if admission_token
                    .as_ref()
                    .is_some_and(CancellationToken::is_cancelled)
                {
                    break true;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap_or(false);
        let daemon_cancelled_on_drain = service.cancelled("stdio-drain");
        service.release("stdio-drain");
        let completed = service.wait("stdio-drain", true).await;
        let drained = draining.await.unwrap();
        let key = keys.lock().unwrap().first().cloned();
        let watchdog = Instant::now() + Duration::from_secs(3);
        let mut terminal_recovered = false;
        if let (Some(key), Ok(mut peer)) = (key, anchor.connect_peer_before(watchdog)) {
            while Instant::now() < watchdog {
                if matches!(
                    peer.recover_invocation_receipt_before(key.clone(), watchdog),
                    Ok(V5ServerResponse::Invocation {
                        outcome: V5InvocationResponse::Direct { .. }
                            | V5InvocationResponse::Acknowledged { .. }
                            | V5InvocationResponse::Task {
                                snapshot: V5DaemonTaskSnapshot::Completed { .. }
                            }
                    })
                ) {
                    terminal_recovered = true;
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
        drop(reader);
        drop(cleanup);
        drop(anchor);
        daemon.finish();
        assert!(started && sdk_stopped && held_at_drain && admission_cancelled && drained);
        assert!(completed && terminal_recovered);
        assert!(
            !daemon_cancelled_on_drain && !service.cancelled("stdio-drain"),
            "run_stdio EOF drain must not become explicit cancellation of accepted daemon work"
        );
        assert_eq!(service.executions("stdio-drain"), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn canonical_manual_cancellation_partial_send_error_closes_output_without_replay() {
        manual_partial_send_failure(false).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn canonical_manual_cancellation_partial_send_panic_closes_output_without_replay() {
        manual_partial_send_failure(true).await;
    }

    async fn manual_partial_send_failure(panic: bool) {
        let service = ManualDaemonService::new(true);
        let daemon = ManualDaemonOwner::start(service.clone());
        let output = Arc::new(ManualFlushGate::default());
        let cleanup = ManualDaemonCleanup {
            service: service.clone(),
            flush: Some(output.clone()),
            before_submit: None,
        };
        let anchor = daemon.owner();
        let (server, _) = manual_daemon_server(daemon.owner(), daemon.workspace_hint.clone());
        let (mut client, in_flight) = spawn_manual_flush_server(server, output.clone());
        client.initialize().await;
        output.written.lock().unwrap().clear();
        output.partial_write.store(true, Ordering::Release);
        output.fail_after_prefix.store(!panic, Ordering::Release);
        output.panic_after_prefix.store(panic, Ordering::Release);
        output.armed.store(true, Ordering::Release);
        send_manual_daemon_call(&mut client, 1, "failed-output").await;
        let started = service.wait("failed-output", false).await;
        let fragment = timeout(TEST_STEP, client.reader.next_line()).await;
        let ended = timeout(TEST_STEP, client.reader.next_line()).await;
        let settled = timeout(Duration::from_secs(3), async {
            while in_flight.running() != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .is_ok();
        let cancelled = service.cancelled("failed-output");
        drop(cleanup);
        let completed = service.wait("failed-output", true).await;
        client.shutdown().await;
        drop(anchor);
        daemon.finish();
        assert!(started && settled && completed);
        assert!(
            matches!(fragment, Ok(Ok(Some(ref text))) if text.len() == 8 && serde_json::from_str::<Value>(text).is_err()),
            "fault must occur after an actual incomplete output prefix"
        );
        assert!(
            matches!(ended, Ok(Ok(None))),
            "a failed partial write must close output, never append a second JSON frame"
        );
        assert!(!cancelled && !service.cancelled("failed-output"));
        assert_eq!(service.executions("failed-output"), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn canonical_manual_cancellation_after_handler_return_before_flush_reaches_exact_task() {
        manual_cancellation_during_send(false).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn canonical_manual_cancellation_after_partial_json_finishes_the_same_frame() {
        manual_cancellation_during_send(true).await;
    }

    async fn manual_cancellation_during_send(partial: bool) {
        let service = ManualDaemonService::new(true);
        let daemon = ManualDaemonOwner::start(service.clone());
        let gate = Arc::new(ManualFlushGate::default());
        let cleanup = ManualDaemonCleanup {
            service: service.clone(),
            flush: Some(gate.clone()),
            before_submit: None,
        };
        let mut observer_owner = daemon.owner();
        let (server, keys) = manual_daemon_server(daemon.owner(), daemon.workspace_hint.clone());
        let (mut client, _) = spawn_manual_flush_server(server, gate.clone());
        client.initialize().await;
        send_manual_daemon_call(&mut client, 1, "flush-neighbor").await;
        let neighbor_started = service.wait("flush-neighbor", false).await;
        let neighbor_response = client.receive().await;
        gate.written.lock().unwrap().clear();
        gate.partial_write.store(partial, Ordering::Release);
        gate.armed.store(true, Ordering::Release);
        send_manual_daemon_call(&mut client, 2, "flush-target").await;
        let target_started = service.wait("flush-target", false).await;
        let send_started = gate.wait().await;
        let held_prefix = gate.written.lock().unwrap().clone();
        // Actual AsyncRwTransport::send reached poll_flush only after SDK
        // awaited call_tool's return; this is not an arbitrary early cancel.
        cancel_manual_daemon_call(&mut client, 2).await;
        let target_stopped = service.wait("flush-target", true).await;
        let target_cancelled = service.cancelled("flush-target");
        let neighbor_cancelled = service.cancelled("flush-neighbor");
        let target_key = keys.lock().unwrap().get(1).cloned();
        let cancel_recorded = target_key.is_some_and(|key| {
            matches!(observer_owner.recover_invocation_receipt_before(key, Instant::now() + Duration::from_secs(2)), Ok(crate::infrastructure::daemon::protocol_v5::V5ServerResponse::Invocation { outcome: crate::infrastructure::daemon::protocol_v5::V5InvocationResponse::Task { snapshot } }) if snapshot.cancel_requested())
        });
        gate.release();
        let target_response = client.receive().await;
        drop(cleanup);
        client.shutdown().await;
        drop(observer_owner);
        daemon.finish();
        assert!(neighbor_started && target_started && send_started);
        if partial {
            assert_eq!(
                held_prefix.len(),
                8,
                "the cancellation race must start after an actual partial JSON write"
            );
        }
        assert_eq!(
            target_response["id"], 2,
            "the already started response must remain a complete parseable frame"
        );
        assert_eq!(neighbor_response["id"], 1);
        assert!(target_stopped && target_cancelled && cancel_recorded, "manual cancellation was lost after the handler returned but before the real transport flush");
        assert!(!neighbor_cancelled);
        assert_eq!(service.executions("flush-target"), 1);
        assert_eq!(service.executions("flush-neighbor"), 1);
    }

    struct ManualSharedService {
        activity: Arc<ManualDaemonService>,
        producer_gate: Arc<ManualReceiptGate>,
        producer_starts: Arc<AtomicUsize>,
        producer_cancelled: Arc<AtomicBool>,
        joined: Mutex<
            std::collections::HashMap<
                String,
                crate::infrastructure::workspace_actor::IndexWorkIdentity,
            >,
        >,
        observed: tokio::sync::Notify,
    }

    impl ManualSharedService {
        async fn wait_joined(&self) -> bool {
            timeout(TEST_STEP, async {
                loop {
                    let changed = self.observed.notified();
                    if self.joined.lock().unwrap().len() == 2 {
                        break;
                    }
                    changed.await;
                }
            })
            .await
            .is_ok()
        }
    }

    impl crate::infrastructure::daemon::server::CanonicalInvocationService for ManualSharedService {
        fn prepare(
            &self,
            _: &crate::infrastructure::daemon::server::ActorBoundInvocation,
        ) -> Result<
            crate::application::operation_descriptors::ExecutionClass,
            Box<crate::domain::invocation::DomainResult>,
        > {
            Ok(crate::application::operation_descriptors::ExecutionClass::InlineCandidate)
        }
        fn execute(
            &self,
            invocation: &crate::infrastructure::daemon::server::ActorBoundExecution,
            cancellation: CancellationToken,
        ) -> Result<
            crate::domain::invocation::DomainResult,
            crate::domain::invocation::InvocationFailure,
        > {
            use crate::application::shared_work::SharedWorkSnapshot;
            let label = invocation.arguments()["args"]["label"]
                .as_str()
                .unwrap()
                .to_owned();
            {
                let mut state = self.activity.state.lock().unwrap();
                let count = state.calls.get(&label).map_or(1, |(count, _)| count + 1);
                state
                    .calls
                    .insert(label.clone(), (count, cancellation.clone()));
            }
            self.activity.observed.notify_waiters();
            let gate = self.producer_gate.clone();
            let starts = self.producer_starts.clone();
            let producer_cancelled = self.producer_cancelled.clone();
            let (identity, lease) = invocation
                .join_index_work(
                    "rlm",
                    "bsl-1",
                    "manual-shared-generation",
                    move |producer| {
                        starts.fetch_add(1, Ordering::AcqRel);
                        gate.hold();
                        producer_cancelled.store(producer.is_cancelled(), Ordering::Release);
                        Ok(())
                    },
                )
                .map_err(|error| {
                    crate::domain::invocation::InvocationFailure::new("index", error)
                })?;
            self.joined.lock().unwrap().insert(label.clone(), identity);
            self.observed.notify_waiters();
            let result = loop {
                // The adapter owns this consumer checkpoint. Dropping its
                // real lease must not signal the other consumer's producer.
                if cancellation.is_cancelled() {
                    break Err(crate::domain::invocation::InvocationFailure::new(
                        "cancelled",
                        "own shared-work consumer stopped",
                    ));
                }
                match lease.wait_timeout(Duration::from_millis(10)) {
                    SharedWorkSnapshot::Running { .. } => {}
                    SharedWorkSnapshot::Ready(_) => {
                        break Ok(crate::domain::invocation::DomainResult::success(
                            label.clone(),
                        ))
                    }
                    SharedWorkSnapshot::Failed(_) => {
                        break Err(crate::domain::invocation::InvocationFailure::new(
                            "index",
                            "shared producer failed",
                        ))
                    }
                }
            };
            drop(lease);
            self.activity
                .state
                .lock()
                .unwrap()
                .completed
                .insert(label, cancellation.is_cancelled());
            self.activity.observed.notify_waiters();
            result
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn canonical_manual_cancellation_shared_index_stops_only_its_consumer() {
        let activity = ManualDaemonService::new(false);
        let producer_gate = Arc::new(ManualReceiptGate::default());
        let service = Arc::new(ManualSharedService {
            activity: activity.clone(),
            producer_gate: producer_gate.clone(),
            producer_starts: Arc::new(AtomicUsize::new(0)),
            producer_cancelled: Arc::new(AtomicBool::new(false)),
            joined: Mutex::new(std::collections::HashMap::new()),
            observed: tokio::sync::Notify::new(),
        });
        let daemon = ManualDaemonOwner(Some(LiveDaemon::start(service.clone())));
        let cleanup = ManualDaemonCleanup {
            service: activity.clone(),
            flush: None,
            before_submit: Some(producer_gate.clone()),
        };
        let (server, keys) = manual_daemon_server(daemon.owner(), daemon.workspace_hint.clone());
        let (mut client, _) = spawn_unica_server(server);
        client.initialize().await;
        send_manual_daemon_call(&mut client, 1, "shared-target").await;
        assert!(activity.wait("shared-target", false).await);
        send_manual_daemon_call(&mut client, 2, "shared-neighbor").await;
        let joined = service.wait_joined().await;
        let same_work = {
            let identities = service.joined.lock().unwrap();
            identities.contains_key("shared-target")
                && identities.get("shared-target") == identities.get("shared-neighbor")
        };
        let distinct_receipts = {
            let receipts = keys.lock().unwrap();
            receipts.len() == 2 && receipts[0] != receipts[1]
        };
        cancel_manual_daemon_call(&mut client, 1).await;
        let target_stopped = activity.wait("shared-target", true).await;
        client
            .send(json!({"jsonrpc":"2.0","id":99,"method":"ping"}))
            .await;
        let ping = client.receive().await;
        let neighbor_cancelled = activity.cancelled("shared-neighbor");
        let before_release = service.producer_starts.load(Ordering::Acquire);
        let neighbor_still_waiting = !activity
            .state
            .lock()
            .unwrap()
            .completed
            .contains_key("shared-neighbor");
        producer_gate.release();
        let neighbor_result = client.receive().await;
        let neighbor_completed = activity.wait("shared-neighbor", true).await;
        drop(cleanup);
        client.shutdown().await;
        daemon.finish();
        assert!(joined && same_work && distinct_receipts, "two real actor-bound consumers must share one exact index identity and separate invocation receipts");
        assert!(target_stopped && activity.cancelled("shared-target"));
        assert_eq!(ping["id"], 99);
        assert!(!neighbor_cancelled && neighbor_still_waiting && neighbor_completed);
        assert_eq!(
            neighbor_result["id"], 2,
            "cancelled follower must not emit a competing result"
        );
        assert_eq!(
            neighbor_result["result"]["structuredContent"]["summary"], "shared-neighbor",
            "{neighbor_result}"
        );
        assert_eq!(before_release, 1);
        assert_eq!(service.producer_starts.load(Ordering::Acquire), 1);
        assert!(!service.producer_cancelled.load(Ordering::Acquire));
        assert_eq!(activity.executions("shared-target"), 1);
        assert_eq!(activity.executions("shared-neighbor"), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn canonical_manual_cancellation_after_completed_before_flush_preserves_exact_terminal() {
        use crate::infrastructure::daemon::protocol_v5::{V5InvocationResponse, V5ServerResponse};
        let service = ManualDaemonService::new(false);
        let hooks = Arc::new(ManualCancelReplyHooks::default());
        let daemon = ManualDaemonOwner::start_with_hooks(service.clone(), hooks.clone());
        let gate = Arc::new(ManualFlushGate::default());
        let cleanup = ManualDaemonCleanup {
            service: service.clone(),
            flush: Some(gate.clone()),
            before_submit: None,
        };
        let mut observer = daemon.owner();
        let (server, keys) = manual_daemon_server(daemon.owner(), daemon.workspace_hint.clone());
        let (mut client, _) = spawn_manual_flush_server(server, gate.clone());
        client.initialize().await;
        gate.armed.store(true, Ordering::Release);
        send_manual_daemon_call(&mut client, 1, "completed-winner").await;
        assert!(service.wait("completed-winner", false).await);
        service.release("completed-winner");
        let completed = service.wait("completed-winner", true).await;
        let send_started = gate.wait().await;
        let original_response = client.receive().await;
        let key = keys.lock().unwrap()[0].clone();
        let original_terminal = observer
            .recover_invocation_receipt_before(key.clone(), Instant::now() + TEST_STEP)
            .unwrap();
        let terminal_digest = match &original_terminal {
            V5ServerResponse::Invocation {
                outcome: V5InvocationResponse::Acknowledged { acknowledgement },
            } if acknowledgement.receipt_key() == &key => acknowledgement.terminal_digest().clone(),
            V5ServerResponse::Invocation {
                outcome: V5InvocationResponse::Direct { receipt },
            } if receipt.receipt_key() == &key => receipt.terminal_digest().clone(),
            other => {
                panic!("completed inline result must already have a durable terminal: {other:?}")
            }
        };
        cancel_manual_daemon_call(&mut client, 1).await;
        // This Ping cannot flush before the original frame; reading and exact
        // control still proceed independently of the blocked output.
        let manual_confirmed = hooks.wait(1).await;
        let preserved = observer
            .recover_invocation_receipt_before(key.clone(), Instant::now() + TEST_STEP)
            .unwrap();
        let unchanged = matches!(preserved, V5ServerResponse::Invocation { outcome: V5InvocationResponse::Acknowledged { ref acknowledgement } } if acknowledgement.receipt_key() == &key && acknowledgement.terminal_digest() == &terminal_digest)
            || matches!(preserved, V5ServerResponse::Invocation { outcome: V5InvocationResponse::Direct { ref receipt } } if receipt.receipt_key() == &key && receipt.terminal_digest() == &terminal_digest);
        gate.release();
        client
            .send(json!({"jsonrpc":"2.0","id":99,"method":"ping"}))
            .await;
        let ping = client.receive().await;
        drop(cleanup);
        client.shutdown().await;
        drop(observer);
        daemon.finish();
        assert!(completed && send_started && manual_confirmed && unchanged);
        assert_eq!(original_response["id"], 1);
        assert_eq!(
            original_response["result"]["structuredContent"]["summary"], "completed-winner",
            "{original_response}"
        );
        assert_eq!(ping["id"], 99);
        assert!(!service.cancelled("completed-winner"));
        assert_eq!(service.executions("completed-winner"), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn canonical_manual_cancellation_reused_id_keeps_old_control_and_targets_new_receipt() {
        let service = ManualDaemonService::new(false);
        let old_control_gate = Arc::new(ManualReceiptGate::default());
        let hooks = Arc::new(ManualCancelReplyHooks {
            gate: Some(old_control_gate.clone()),
            ..Default::default()
        });
        let daemon = ManualDaemonOwner::start_with_hooks(service.clone(), hooks.clone());
        let flush = Arc::new(ManualFlushGate::default());
        let cleanup = ManualDaemonCleanup {
            service: service.clone(),
            flush: Some(flush.clone()),
            before_submit: Some(old_control_gate.clone()),
        };
        let anchor = daemon.owner();
        let (server, keys) = manual_daemon_server(daemon.owner(), daemon.workspace_hint.clone());
        let calls = server.manual_calls.clone();
        let (mut client, in_flight) = spawn_manual_flush_server(server, flush.clone());
        client.send(json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2026-07-28","capabilities":{},"clientInfo":{"name":"unica-tests","version":"1"}}})).await;
        let initialized = client.receive().await;
        assert_eq!(
            initialized["result"]["protocolVersion"], "2026-07-28",
            "{initialized}"
        );
        client
            .send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
            .await;
        flush.armed.store(true, Ordering::Release);
        client.send(json!({"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"unica.run","arguments":{"op":"test.long-work","args":{"label":"reuse-original"}},"_meta":modern_meta()}})).await;
        assert!(service.wait("reuse-original", false).await);
        service.release("reuse-original");
        let actual_flush_held = flush.wait().await;
        // The modern sender has received a complete original response. That
        // permits reuse even while the underlying output flush is still held.
        cancel_manual_daemon_call(&mut client, 7).await;
        let old_cancel_committed = hooks.wait(1).await;
        let original = client.receive().await;
        client.send(json!({"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"unica.run","arguments":{"op":"test.long-work","args":{"label":"reuse-new"}},"_meta":modern_meta()}})).await;
        let new_entered = service.wait("reuse-new", false).await;
        cancel_manual_daemon_call(&mut client, 7).await;
        let new_stopped = if new_entered {
            service.wait("reuse-new", true).await
        } else {
            false
        };
        let distinct_receipts = {
            let keys = keys.lock().unwrap();
            keys.len() == 2 && keys[0] != keys[1]
        };
        flush.release();
        client
            .send(json!({"jsonrpc":"2.0","id":99,"method":"ping","params":{"_meta":modern_meta()}}))
            .await;
        let mut ping = client.receive().await;
        if ping["id"] != 99 {
            ping = client.receive().await;
        }
        let new_retired = timeout(TEST_STEP, async {
            while calls.get(&rmcp::model::RequestId::Number(7)).is_some() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .is_ok();
        client.shutdown().await;
        let (started, entered) = tokio::sync::oneshot::channel();
        let draining = tokio::task::spawn_blocking(move || {
            let _ = started.send(());
            drain_stdio_frontend(&in_flight, &calls)
        });
        assert!(matches!(timeout(TEST_STEP, entered).await, Ok(Ok(()))));
        let mut draining = draining;
        let premature_drain = timeout(Duration::from_millis(50), &mut draining)
            .await
            .is_ok();
        old_control_gate.release();
        let drain_settled =
            premature_drain || matches!(timeout(TEST_STEP, draining).await, Ok(Ok(true)));
        drop(cleanup);
        drop(anchor);
        daemon.finish();
        assert_eq!(original["id"], 7);
        assert_eq!(
            original["result"]["structuredContent"]["summary"],
            "reuse-original"
        );
        assert!(actual_flush_held && old_cancel_committed);
        assert!(new_entered && new_stopped && distinct_receipts && new_retired, "receiving a complete modern response permits the same RequestId for a new invocation while the old flush/control is still retained");
        assert!(service.cancelled("reuse-new"));
        assert!(!service.cancelled("reuse-original"));
        assert_eq!(service.executions("reuse-original"), 1);
        assert_eq!(service.executions("reuse-new"), 1);
        assert_eq!(ping["id"], 99);
        assert!(!premature_drain && drain_settled, "an older accepted control must remain owned through its actual acknowledgement after RequestId reuse");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn canonical_manual_cancellation_sole_engine_waiter_preserves_delivery_and_pinned_artifact(
    ) {
        const CHILD: &str = "UNICA_MANUAL_ENGINE_WAITER_CHILD";
        const ROOT: &str = "UNICA_MANUAL_ENGINE_WAITER_ROOT";
        if std::env::var_os(CHILD).is_none() {
            let root = tempfile::tempdir().unwrap();
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "interfaces::mcp::tests::canonical_manual_cancellation_sole_engine_waiter_preserves_delivery_and_pinned_artifact", "--nocapture"])
                .env(CHILD, "1").env(ROOT, root.path()).output().unwrap();
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(stdout.contains("running 1 test"), "{stdout}\n{stderr}");
            assert!(output.status.success(), "{stdout}\n{stderr}");
            return;
        }
        use crate::infrastructure::daemon::server::actor_capacity_tests::{
            canonical_v13_service, LiveV5Daemon,
        };
        use sha2::{Digest, Sha256};
        use unica_bootstrap::{BootstrapError, DownloadObserver, Downloader, HostTarget};
        struct ControlledDownload {
            bytes: Vec<u8>,
            gate: Arc<ManualReceiptGate>,
            starts: AtomicUsize,
            started: tokio::sync::Notify,
            finished: AtomicBool,
        }
        impl Downloader for ControlledDownload {
            fn download(
                &self,
                _: &str,
                destination: &std::path::Path,
                observer: &dyn DownloadObserver,
            ) -> Result<(), BootstrapError> {
                self.starts.fetch_add(1, Ordering::AcqRel);
                self.started.notify_waiters();
                self.gate.hold();
                std::fs::write(destination, &self.bytes)?;
                observer.transferred(self.bytes.len() as u64, Some(self.bytes.len() as u64));
                self.finished.store(true, Ordering::Release);
                Ok(())
            }
        }
        let root = std::path::PathBuf::from(std::env::var_os(ROOT).unwrap());
        let workspace = root.join("workspace");
        let plugin = root.join("plugin");
        let cache = root.join("cache");
        std::fs::create_dir_all(workspace.join("src")).unwrap();
        std::fs::create_dir_all(plugin.join("third-party")).unwrap();
        std::fs::write(workspace.join("v8project.yaml"), "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\ninfobase:\n  connection: 'File=base'\n").unwrap();
        std::fs::write(workspace.join("src/Configuration.xml"), r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Test</Name></Properties><ChildObjects/></Configuration></MetaDataObject>"#).unwrap();
        let source = root.join("engine-probe.rs");
        std::fs::write(&source, r###"
fn main() {
    let cwd = std::env::current_dir().unwrap();
    use std::io::Write;
    let mut runs = std::fs::OpenOptions::new().create(true).append(true).open(cwd.join("runner-dispatch.log")).unwrap();
    writeln!(runs, "extensions").unwrap();
    println!("{}", r#"{"ok":true,"command":"extensions","data":{"ok":true,"provider_dispatched":false,"provider":{"selected":"ibcmd","origin":{"kind":"default"}},"requested":{"kind":"all"},"extensions":[],"plan":"fixture"}}"#);
}
"###).unwrap();
        let binary = root.join(format!("engine-probe{}", std::env::consts::EXE_SUFFIX));
        let compiled =
            std::process::Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
                .arg("--edition=2021")
                .arg(&source)
                .arg("-o")
                .arg(&binary)
                .output()
                .unwrap();
        assert!(
            compiled.status.success(),
            "{}",
            String::from_utf8_lossy(&compiled.stderr)
        );
        let bytes = std::fs::read(&binary).unwrap();
        let digest = format!("{:x}", Sha256::digest(&bytes));
        let mut core_targets = serde_json::Map::new();
        let mut runner_targets = serde_json::Map::new();
        let runner_version = "0.11.4"; // Fixture pin must match the production runner contract.
        for host in HostTarget::ALL {
            let target = host.as_str();
            let suffix = if target == "win-x64" { ".exe" } else { "" };
            let core_asset = format!("unica-runtime-{target}.tar.gz");
            let core_path = format!("bin/{target}/unica{suffix}");
            core_targets.insert(target.to_owned(), json!({
                "asset":{"name":core_asset,"url":format!("https://github.com/IngvarConsulting/unica/releases/download/v{}/{core_asset}",env!("CARGO_PKG_VERSION")),"mediaType":"application/gzip","sha256":"0".repeat(64)},
                "files":[{"path":core_path,"sha256":"0".repeat(64),"executable":true}],"entrypoint":core_path
            }));
            let asset = format!("v8-runner-{target}{suffix}");
            runner_targets.insert(target.to_owned(), json!({
                "asset":{"name":asset,"url":format!("https://github.com/IngvarConsulting/v8-runner-rust/releases/download/v8-runner-v{runner_version}-build.1/{asset}"),"mediaType":"application/octet-stream","sha256":digest},
                "files":[{"path":format!("bin/{target}/v8-runner{suffix}"),"sha256":digest,"executable":true}]
            }));
        }
        let release = json!({"schemaVersion":2,"pluginVersion":env!("CARGO_PKG_VERSION"),
            "source":{"repository":"https://github.com/IngvarConsulting/unica","commit":"0".repeat(40)},
            "release":{"repository":"https://github.com/IngvarConsulting/unica","tag":format!("v{}",env!("CARGO_PKG_VERSION"))},
            "artifacts":{"unica":{"version":env!("CARGO_PKG_VERSION"),"role":"core","targets":core_targets},"v8-runner":{"version":runner_version,"role":"engine","targets":runner_targets}}
        });
        let release_path = plugin.join("runtime-manifest.json");
        std::fs::write(&release_path, release.to_string()).unwrap();
        let target = crate::infrastructure::platform::current_target_id().unwrap();
        let relative = format!("bin/{target}/v8-runner{}", std::env::consts::EXE_SUFFIX);
        std::fs::write(plugin.join("third-party/manifest.json"), json!({"schemaVersion":2,"artifactAssets":{"v8-runner":{"sha256":digest}},"tools":[{"name":"v8-runner","version":runner_version,"binaryPath":relative,"deliveredPath":relative,"sha256":digest}]}).to_string()).unwrap();
        // Only this one-test child owns these global overrides.
        std::env::set_var("UNICA_PLUGIN_ROOT", &plugin);
        std::env::set_var("UNICA_ARTIFACT_CACHE", &cache);
        std::env::set_var("UNICA_RUNTIME_MANIFEST", &release_path);
        let download_gate = Arc::new(ManualReceiptGate::default());
        let downloader = Arc::new(ControlledDownload {
            bytes,
            gate: download_gate.clone(),
            starts: AtomicUsize::new(0),
            started: tokio::sync::Notify::new(),
            finished: AtomicBool::new(false),
        });
        let daemon =
            ManualCanonicalDaemonOwner(Some(LiveV5Daemon::start_with_delivery_downloader(
                canonical_v13_service(),
                downloader.clone(),
            )));
        let _download_release = ManualProducerRelease(download_gate.clone());
        let owner = daemon.owner();
        let flush = Arc::new(ManualFlushGate::default());
        let _flush_release = ManualFlushRelease(flush.clone());
        let server = UnicaServer::with_canonical_daemon(
            daemon.owner(),
            workspace.to_string_lossy().into_owned(),
        );
        let (mut client, _) = spawn_manual_flush_server(server, flush.clone());
        client.initialize().await;
        flush.armed.store(true, Ordering::Release);
        client.send(json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"unica.run","arguments":{"op":"extensions.list","args":{},"dryRun":true},"_meta":modern_meta()}})).await;
        let response = client.receive().await;
        let task_id = response["result"]["structuredContent"]["data"]["task"]["taskId"]
            .as_str()
            .unwrap_or_else(|| panic!("expected actual Task: {response}"))
            .parse()
            .unwrap();
        assert!(flush.wait().await);
        let started = timeout(TEST_STEP, async {
            loop {
                let changed = downloader.started.notified();
                if downloader.starts.load(Ordering::Acquire) == 1 {
                    break;
                }
                changed.await;
            }
        })
        .await
        .is_ok();
        assert!(
            started,
            "the sole real run must reach its pinned engine downloader"
        );
        cancel_manual_daemon_call(&mut client, 1).await;
        let terminal = daemon.wait_terminal(&owner, task_id, TEST_STEP);
        assert_eq!(
            terminal.status(),
            crate::domain::invocation::InvocationStatus::Cancelled,
            "{terminal:?}"
        );
        assert!(!downloader.finished.load(Ordering::Acquire));
        assert!(!workspace.join("runner-dispatch.log").exists());
        flush.release();
        download_gate.release();
        let installed = timeout(TEST_STEP, async {
            loop {
                if let Some(path) = crate::infrastructure::bundled_tools::installed_engine_path(
                    &plugin,
                    "v8-runner",
                ) {
                    break path;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("process-owned delivery must finish with no remaining waiter");
        assert!(
            installed.is_absolute() && installed.starts_with(&cache),
            "{}",
            installed.display()
        );
        assert_eq!(
            format!("{:x}", Sha256::digest(std::fs::read(&installed).unwrap())),
            digest
        );
        client.send(json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"unica.run","arguments":{"op":"extensions.list","args":{},"dryRun":true},"_meta":modern_meta()}})).await;
        let fresh = client.receive().await;
        let fresh_task = fresh["result"]["structuredContent"]["data"]["task"]["taskId"]
            .as_str()
            .unwrap_or_else(|| panic!("expected fresh Task: {fresh}"))
            .parse()
            .unwrap();
        let fresh_terminal = daemon.wait_terminal(&owner, fresh_task, TEST_STEP);
        assert_eq!(
            fresh_terminal.status(),
            crate::domain::invocation::InvocationStatus::Completed,
            "{fresh_terminal:?}"
        );
        assert_eq!(downloader.starts.load(Ordering::Acquire), 1);
        assert_eq!(
            std::fs::read_to_string(workspace.join("runner-dispatch.log"))
                .unwrap()
                .lines()
                .count(),
            1
        );
        client.shutdown().await;
        daemon.finish(owner);
    }

    struct ManualProducerRelease(Arc<ManualReceiptGate>);
    impl Drop for ManualProducerRelease {
        fn drop(&mut self) {
            self.0.release();
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn canonical_manual_cancellation_native_reuse_does_not_inherit_old_suppression() {
        let service = ManualDaemonService::new(false);
        let control_gate = Arc::new(ManualReceiptGate::default());
        let hooks = Arc::new(ManualCancelReplyHooks {
            gate: Some(control_gate.clone()),
            ..Default::default()
        });
        let daemon = ManualDaemonOwner::start_with_hooks(service.clone(), hooks.clone());
        let flush = Arc::new(ManualFlushGate::default());
        let cleanup = ManualDaemonCleanup {
            service: service.clone(),
            flush: Some(flush.clone()),
            before_submit: Some(control_gate.clone()),
        };
        let (server, _) = manual_daemon_server(daemon.owner(), daemon.workspace_hint.clone());
        let (mut client, _) = spawn_manual_flush_server(server, flush.clone());
        client.send(json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2026-07-28","capabilities":{},"clientInfo":{"name":"unica-tests","version":"1"}}})).await;
        let initialization = client.receive().await;
        assert_eq!(initialization["result"]["protocolVersion"], "2026-07-28");
        client
            .send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
            .await;
        flush.armed.store(true, Ordering::Release);
        client.send(json!({"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"unica.run","arguments":{"op":"test.long-work","args":{"label":"native-reuse-original"}},"_meta":modern_meta()}})).await;
        assert!(service.wait("native-reuse-original", false).await);
        service.release("native-reuse-original");
        assert!(flush.wait().await);
        cancel_manual_daemon_call(&mut client, 7).await;
        let committed = hooks.wait(1).await;
        let original = client.receive().await;
        client
            .send(json!({"jsonrpc":"2.0","id":7,"method":"ping","params":{"_meta":modern_meta()}}))
            .await;
        flush.release();
        let reused_ping = client.receive().await;
        control_gate.release();
        drop(cleanup);
        client.shutdown().await;
        daemon.finish();
        assert!(committed);
        assert_eq!(
            original["result"]["structuredContent"]["summary"],
            "native-reuse-original"
        );
        assert_eq!(reused_ping["id"], 7);
        assert_eq!(reused_ping["error"]["code"], -32601, "the SDK must receive the native request and return its actual modern-method refusal, without old CallTool suppression or duplicate-ID rejection: {reused_ping}");
        assert_eq!(reused_ping["error"]["message"], "ping", "{reused_ping}");
        assert!(!service.cancelled("native-reuse-original"));
        assert_eq!(service.executions("native-reuse-original"), 1);
    }

    #[tokio::test]
    async fn ping_stays_responsive_and_cancellation_reaches_the_tool() {
        let cancellation_seen = Arc::new(AtomicBool::new(false));
        let seen = Arc::clone(&cancellation_seen);
        let handler: Arc<ToolCallHandler> = Arc::new(move |_, _, cancellation, _| {
            let give_up = Instant::now() + 4 * TEST_STEP;
            while !cancellation.is_cancelled() {
                if Instant::now() > give_up {
                    return Err((-32603, "test handler was never cancelled".to_string()));
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            seen.store(true, Ordering::SeqCst);
            Ok(successful_test_result("unreachable success"))
        });
        let (mut client, _) = spawn_server(handler);
        client.initialize().await;

        client
            .send(json!({
                "jsonrpc": "2.0",
                "id": 7,
                "method": "tools/call",
                "params": { "name": "unica.code.search", "arguments": {} }
            }))
            .await;
        client
            .send(json!({ "jsonrpc": "2.0", "id": 8, "method": "ping" }))
            .await;
        let response = client.receive().await;
        assert_eq!(response["id"], 8, "ping must not wait for tools/call");
        assert!(!cancellation_seen.load(Ordering::SeqCst));

        client
            .send(json!({
                "jsonrpc": "2.0",
                "method": "notifications/cancelled",
                "params": { "requestId": 7, "reason": "test" }
            }))
            .await;
        // The specification says a cancelled request gets no response; the next
        // response on the wire must belong to the follow-up ping.
        client
            .send(json!({ "jsonrpc": "2.0", "id": 9, "method": "ping" }))
            .await;
        let response = client.receive().await;
        assert_eq!(
            response["id"], 9,
            "cancelled tools/call must not produce a response"
        );
        let deadline = Instant::now() + TEST_STEP;
        while !cancellation_seen.load(Ordering::SeqCst) {
            assert!(
                Instant::now() < deadline,
                "cancellation did not reach the tool implementation"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        client.shutdown().await;
    }

    #[tokio::test]
    async fn eof_cancels_active_calls_within_a_bounded_grace() {
        let cancellation_seen = Arc::new(AtomicBool::new(false));
        let seen = Arc::clone(&cancellation_seen);
        let handler: Arc<ToolCallHandler> = Arc::new(move |_, _, cancellation, _| {
            let give_up = Instant::now() + 4 * TEST_STEP;
            while !cancellation.is_cancelled() {
                if Instant::now() > give_up {
                    return Err((-32603, "test handler was never cancelled".to_string()));
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            seen.store(true, Ordering::SeqCst);
            Ok(successful_test_result("unreachable success"))
        });
        let (mut client, in_flight) = spawn_server(handler);
        client.initialize().await;
        client
            .send(json!({
                "jsonrpc": "2.0",
                "id": "work",
                "method": "tools/call",
                "params": { "name": "unica.code.search", "arguments": {} }
            }))
            .await;
        // Give the call a moment to be admitted before closing the transport.
        let admitted_deadline = Instant::now() + TEST_STEP;
        while in_flight.running() == 0 {
            assert!(
                Instant::now() < admitted_deadline,
                "tools/call was not admitted"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        client.writer.shutdown().await.unwrap();
        drop(client.writer);
        timeout(TEST_STEP, client.server)
            .await
            .expect("server did not stop after EOF")
            .unwrap();

        // Mirror the run_stdio shutdown path: cancel leftovers and share one
        // aggregate grace with tracked provider cleanup.
        let drained = tokio::task::spawn_blocking(move || {
            drain_mcp_shutdown(&in_flight, EOF_CANCELLATION_GRACE)
        })
        .await
        .unwrap();
        assert!(drained, "cancelled call did not finish within the grace");
        assert!(cancellation_seen.load(Ordering::SeqCst));
    }

    #[test]
    fn admission_crosses_the_former_call_quota_and_retains_exact_ownership() {
        let registry = Arc::new(InFlightRegistry::default());
        let mut guards = Vec::new();
        for _ in 0..33 {
            guards.push(
                registry
                    .admit()
                    .expect("call count must not refuse admission"),
            );
        }
        assert_eq!(registry.running(), 33);
        let identifiers: std::collections::BTreeSet<_> =
            guards.iter().map(|guard| guard.id).collect();
        assert_eq!(identifiers.len(), 33);
        let released = guards.pop().unwrap();
        let released_token = released.token();
        drop(released);
        assert_eq!(registry.running(), 32);
        registry.cancel_all();
        assert!(!released_token.is_cancelled());
        assert!(guards.iter().all(|guard| guard.token().is_cancelled()));
        assert_eq!(
            registry.running(),
            32,
            "cancel must retain executing owners"
        );
        let next = registry.admit().unwrap();
        assert!(!next.token().is_cancelled());
        assert!(!identifiers.contains(&next.id));
        drop(next);
        drop(guards);
        assert!(registry.wait_idle(Duration::from_millis(100)));
    }

    #[test]
    fn admission_identifier_overflow_preserves_existing_call_ownership() {
        let registry = Arc::new(InFlightRegistry::default());
        let existing = registry.admit().unwrap();
        let existing_token = existing.token();
        registry.state.lock().unwrap().next_id = u64::MAX;

        assert_eq!(
            registry.admit().unwrap_err(),
            "in-flight call identifier overflow"
        );
        {
            let state = registry.state.lock().unwrap();
            assert_eq!(state.next_id, u64::MAX);
            assert_eq!(state.running.len(), 1);
            assert_eq!(state.running[0].0, existing.id);
            assert!(!state.running[0].1.is_cancelled());
        }
        registry.cancel_all();
        assert!(existing_token.is_cancelled());
        assert_eq!(registry.running(), 1);
        drop(existing);
        assert!(registry.wait_idle(Duration::from_millis(100)));
    }

    #[test]
    fn eof_cleanup_drains_tracked_code_search_workers_within_grace() {
        crate::application::code_intelligence::track_code_search_worker_for_test(
            std::thread::spawn(|| std::thread::sleep(Duration::from_millis(50))),
        );

        assert!(
            crate::application::code_intelligence::drain_code_search_workers(
                EOF_CANCELLATION_GRACE
            ),
            "tracked code-search worker outlived the EOF cleanup grace"
        );
    }

    #[test]
    fn eof_cleanup_drains_noncooperative_diagnostic_worker_within_the_same_grace() {
        // This worker deliberately has no cancellation token. It models a
        // provider that ignored cancellation after its tool call returned.
        crate::application::diagnostics::track_diagnostic_worker_for_test(std::thread::spawn(
            || std::thread::sleep(Duration::from_millis(50)),
        ));

        let registry = InFlightRegistry::default();
        assert!(
            drain_mcp_shutdown(&registry, EOF_CANCELLATION_GRACE),
            "tracked diagnostics worker outlived the EOF cleanup grace"
        );
    }

    #[test]
    fn eof_cleanup_shares_one_aggregate_grace_between_calls_and_provider_workers() {
        // The tracked call outlives the whole grace: its release is only
        // published after the drain has returned, so the call phase provably
        // consumes the entire aggregate budget and provider cleanup must be
        // handed exactly the remainder — zero. A drain that granted provider
        // cleanup a fresh grace would hand over the full `AGGREGATE_GRACE`
        // instead. Every assertion below rests on event ordering alone
        // (channels and thread joins), never on wall-clock measurements, so
        // scheduler delays on a loaded runner stretch the test but can never
        // flip a comparison.
        const AGGREGATE_GRACE: Duration = Duration::from_millis(200);

        let registry = Arc::new(InFlightRegistry::default());
        let guard = registry.admit().unwrap();
        let cancellation = guard.token();
        let (cancelled_tx, cancelled_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let guard_thread = std::thread::spawn(move || {
            while !cancellation.is_cancelled() {
                std::thread::yield_now();
            }
            cancelled_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            drop(guard);
        });

        let drain_registry = Arc::clone(&registry);
        let drain_thread = std::thread::spawn(move || {
            let mut provider_budget = None;
            let drained = drain_mcp_shutdown_with(&drain_registry, AGGREGATE_GRACE, |remaining| {
                provider_budget = Some(remaining);
                true
            });
            (drained, provider_budget)
        });

        // Liveness handshake, deliberately not bounded by the grace: it only
        // proves the drain cancelled the tracked call, so the zero remainder
        // below is the shared deadline at work and not an idle registry.
        cancelled_rx
            .recv_timeout(4 * TEST_STEP)
            .expect("cancellation did not reach the tracked call");

        // Joining before the release is the point of the test: the call is
        // still tracked for the drain's whole lifetime, purely by ordering.
        let (drained, provider_budget) = drain_thread.join().unwrap();
        assert!(
            !drained,
            "the drain reported success while the call was still tracked"
        );
        assert_eq!(
            provider_budget,
            Some(Duration::ZERO),
            "provider cleanup received a fresh grace instead of the aggregate remainder"
        );

        // The call still cleans up after the grace expired; the registry must
        // come back to idle once the release is published.
        release_tx.send(()).unwrap();
        guard_thread.join().unwrap();
        assert!(
            registry.wait_idle(Duration::ZERO),
            "the released call did not leave the in-flight registry"
        );
    }

    #[tokio::test]
    async fn dispatcher_executes_all_calls_past_the_former_quota() {
        let release = Arc::new(AtomicBool::new(false));
        let gate = Arc::clone(&release);
        let entered = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let worker_entered = Arc::clone(&entered);
        let handler: Arc<ToolCallHandler> = Arc::new(move |_, _, _, _| {
            worker_entered.fetch_add(1, Ordering::SeqCst);
            let give_up = Instant::now() + 4 * TEST_STEP;
            while !gate.load(Ordering::SeqCst) {
                if Instant::now() > give_up {
                    return Err((-32603, "test handler was never released".to_string()));
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            Ok(successful_test_result("released"))
        });
        let (mut client, in_flight) = spawn_server(handler);
        client.initialize().await;
        let expected: std::collections::BTreeSet<_> =
            (0..33).map(|id| format!("blocked-{id}")).collect();
        for id in &expected {
            client
                .send(json!({
                    "jsonrpc": "2.0", "id": id, "method": "tools/call",
                    "params": { "name": "unica.code.search", "arguments": {} }
                }))
                .await;
        }
        let admitted_deadline = Instant::now() + TEST_STEP;
        while entered.load(Ordering::SeqCst) < 33 && Instant::now() < admitted_deadline {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let entered_before_release = entered.load(Ordering::SeqCst);
        let tracked_before_release = in_flight.running();
        // Always release and drain every owned worker before asserting the admission result.
        release.store(true, Ordering::SeqCst);
        let mut responses = Vec::new();
        for _ in 0..33 {
            responses.push(client.receive().await);
        }
        client.shutdown().await;
        let refused: Vec<_> = responses
            .iter()
            .filter_map(|response| response.get("error"))
            .collect();
        assert!(
            refused.is_empty(),
            "calls were refused by count: {refused:?}"
        );
        assert_eq!(
            entered_before_release, 33,
            "every accepted call must actually execute"
        );
        assert_eq!(tracked_before_release, 33);
        let actual: std::collections::BTreeSet<_> = responses
            .iter()
            .map(|response| response["id"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(actual, expected);
        for response in &responses {
            let payload: Value =
                serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap())
                    .unwrap();
            assert_eq!(payload["summary"], "released");
        }
        assert_eq!(in_flight.running(), 0);
    }

    #[test]
    fn code_patch_mcp_text_contains_an_object_data_field_instead_of_json_stdout() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "unica-code-patch-mcp-{}-{nanos}",
            std::process::id()
        ));
        let src = root.join("src");
        let module = src.join("CommonModules/Sample/Ext/Module.bsl");
        std::fs::create_dir_all(module.parent().unwrap()).unwrap();
        std::fs::write(
            root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(
            src.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration/></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(
            src.join("CommonModules/Sample.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><CommonModule><Properties><Name>Sample</Name></Properties></CommonModule></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(&module, "Procedure Run()\nEndProcedure\n").unwrap();
        let args = json!({
            "cwd": root,
            "sourceSet": "main",
            "metadataPath": "CommonModule.Sample.Module",
            "operation": "insert",
            "selector": {"method": "Run"},
            "content": "Procedure Added()\nEndProcedure",
            "position": "after"
        })
        .as_object()
        .unwrap()
        .clone();

        let text = call_tool_text(
            &UnicaApplication::new(),
            "unica.code.patch",
            &args,
            CancellationToken::new(),
        )
        .unwrap();
        let result: Value = serde_json::from_str(&text).unwrap();

        assert!(result["data"].is_object());
        assert_eq!(result["data"]["sourceSet"], "main");
        assert_eq!(result["data"]["metadataPath"], "CommonModule.Sample.Module");
        assert_eq!(result["data"]["targetKind"], "module");
        assert!(result["data"].get("path").is_none());
        assert_eq!(result["data"]["validation"]["status"], "passed");
        assert!(result.get("stdout").is_none());

        let before_invalid = std::fs::read(&module).unwrap();
        let mut invalid_args = args;
        invalid_args.insert("selector".to_string(), json!({"anchor": "EndProcedure"}));
        invalid_args.insert("position".to_string(), json!("before"));
        invalid_args.insert("content".to_string(), json!("    If True Then"));
        invalid_args.insert("dryRun".to_string(), json!(false));

        let failed_text = call_tool_text(
            &UnicaApplication::new(),
            "unica.code.patch",
            &invalid_args,
            CancellationToken::new(),
        )
        .unwrap();
        let failed: Value = serde_json::from_str(&failed_text).unwrap();
        assert_eq!(failed["ok"], false);
        assert_eq!(failed["data"]["validation"]["status"], "failed");
        assert!(failed["data"]["validation"]["diagnostics"]
            .as_array()
            .is_some_and(|diagnostics| !diagnostics.is_empty()));
        assert!(failed.get("stdout").is_none());
        assert_eq!(std::fs::read(&module).unwrap(), before_invalid);
        std::fs::remove_dir_all(root).unwrap();
    }
}
