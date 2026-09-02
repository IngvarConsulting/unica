//! Public `unica` stdio MCP server on the official Rust SDK (`rmcp`).
//!
//! ADR-0013: the SDK owns the JSON-RPC loop, handshake, protocol version
//! negotiation, per-request task spawning, `ping`, and `notifications/cancelled`
//! bookkeeping. This module only maps SDK requests onto the transport-neutral
//! application layer (ADR-0002) and keeps the tool contract data-driven from
//! operation descriptors (ADR-0001) instead of SDK macros.

use crate::application::invocation::{
    handoff_budget, INVOCATION_HANDOFF_WINDOW, RESPONSE_SERIALIZATION_MARGIN,
};
use crate::application::invocation_store::ToolIdentity;
use crate::application::tool_contracts::{SurfaceRelease, V13TaskProfile};
use crate::application::{
    code_search_output_schema, input_schema_for_tool, metadata_argument_failure_result,
    operation_result_output_schema, role_edit_argument_failure_result, role_edit_output_schema,
    strip_schema_descriptions, CodeIntelligenceOperation, OperationResult, ToolHandler, ToolSpec,
    UnicaApplication,
};
use crate::domain::cancellation::CancellationToken;
use crate::domain::progress::{NoopProgressSink, ProgressEvent, ProgressSink};
use rmcp::model::{
    CacheScope, CallToolRequestParams, CallToolResponse, CallToolResult, CancelTaskParams,
    ContentBlock, ErrorCode, ErrorData, GetTaskParams, GetTaskResult, Implementation,
    InitializeRequestParams, InitializeResult, ListPromptsResult, ListResourceTemplatesResult,
    ListResourcesResult, ListToolsResult, NotificationMetaObject, PaginatedRequestParams,
    ProgressNotificationParam, ProgressToken, ProtocolVersion, RequestMetaObject,
    ServerCapabilities, ServerInfo, Tool, UpdateTaskParams, TASKS_EXTENSION_ID,
};
use rmcp::service::{RequestContext, ServerInitializeError};
use rmcp::{RoleServer, ServerHandler, ServiceExt};
use serde_json::{Map, Value};
use std::collections::HashSet;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::infrastructure::daemon::protocol::{InvocationRequest, InvocationResponse};

pub const MCP_MAX_TOOL_WORKERS: usize = 32;
const EOF_CANCELLATION_GRACE: Duration = Duration::from_secs(2);
const RUNTIME_SHUTDOWN_GRACE: Duration = Duration::from_millis(250);
const TOOL_EXECUTION_ERROR: i32 = -32000;

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

type CanonicalToolCallHandler = dyn Fn(
        ToolIdentity,
        &Map<String, Value>,
        FrontendInvocationDeadline,
        CancellationToken,
    ) -> Result<InvocationResponse, (i32, String)>
    + Send
    + Sync;

type CanonicalTaskHandler = dyn Fn(
        crate::domain::invocation::TaskId,
        FrontendInvocationDeadline,
    ) -> Result<
        crate::infrastructure::daemon::protocol::DaemonTaskSnapshot,
        crate::infrastructure::daemon::client::DaemonTaskExchangeError,
    > + Send
    + Sync;

type CanonicalTaskWaitHandler = dyn Fn(
        crate::domain::invocation::TaskId,
        u64,
        FrontendInvocationDeadline,
    ) -> Result<
        crate::infrastructure::daemon::protocol::DaemonTaskSnapshot,
        crate::infrastructure::daemon::client::DaemonTaskExchangeError,
    > + Send
    + Sync;

#[derive(Clone)]
struct CanonicalV13Router {
    call: Arc<CanonicalToolCallHandler>,
    get: Arc<CanonicalTaskHandler>,
    wait: Arc<CanonicalTaskWaitHandler>,
    cancel: Arc<CanonicalTaskHandler>,
}

#[derive(Clone)]
enum SurfaceToolRouter {
    #[allow(dead_code)] // constructed only by the explicit legacy test seam
    LegacyV12(Arc<ToolCallHandler>),
    CanonicalV13(CanonicalV13Router),
}

enum SurfaceToolOutcome {
    Legacy(Box<OperationResult>),
    Canonical(crate::domain::invocation::DomainResult),
    Task(crate::infrastructure::daemon::protocol::DaemonTaskSnapshot),
}

#[derive(Debug, Clone, Copy)]
struct FrontendInvocationDeadline {
    received_at: Instant,
    host_remaining_at_receipt: Option<Duration>,
}

impl FrontendInvocationDeadline {
    fn new(received_at: Instant, host_remaining_at_receipt: Option<Duration>) -> Self {
        Self {
            received_at,
            host_remaining_at_receipt,
        }
    }

    fn remaining_at(self, now: Instant) -> Duration {
        remaining_invocation_budget(self.received_at, now, self.host_remaining_at_receipt)
    }

    fn remaining_transport_at(self, now: Instant) -> Duration {
        let elapsed = now.saturating_duration_since(self.received_at);
        let own_remaining = INVOCATION_HANDOFF_WINDOW
            .saturating_add(RESPONSE_SERIALIZATION_MARGIN)
            .saturating_sub(elapsed);
        let host_remaining = self
            .host_remaining_at_receipt
            .map(|remaining| remaining.saturating_sub(elapsed));
        host_remaining.map_or(own_remaining, |remaining| own_remaining.min(remaining))
    }

    fn transport_cutoff(self) -> Instant {
        let own_cutoff = self
            .received_at
            .checked_add(INVOCATION_HANDOFF_WINDOW.saturating_add(RESPONSE_SERIALIZATION_MARGIN))
            .expect("bounded frontend transport cutoff");
        self.host_remaining_at_receipt
            .and_then(|remaining| self.received_at.checked_add(remaining))
            .map_or(own_cutoff, |host_cutoff| own_cutoff.min(host_cutoff))
    }
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
    let owner = match crate::interfaces::daemon::connect_default_user_daemon(&state_root) {
        Ok(owner) => owner,
        Err(error) => {
            eprintln!("failed to connect to unica user daemon: {error}");
            return;
        }
    };
    let workspace_hint = match std::env::current_dir() {
        Ok(path) => path.to_string_lossy().into_owned(),
        Err(error) => {
            eprintln!("failed to determine unica MCP workspace: {error}");
            return;
        }
    };
    let notice = startup_notice_from(std::env::var(STARTUP_NOTICE_ENV).ok());
    let server = UnicaServer::canonical_v13_daemon(owner, workspace_hint, notice);
    let in_flight = server.in_flight();

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
        match server.serve(rmcp::transport::stdio()).await {
            Ok(running) => {
                let _ = running.waiting().await;
            }
            // A host that closes stdin before the handshake is a clean shutdown,
            // matching the pre-SDK loop; anything else is worth a stderr line.
            Err(ServerInitializeError::ConnectionClosed(_)) => {}
            Err(error) => eprintln!("unica mcp initialization failed: {error}"),
        }
    });
    // The SDK drained finishing calls before `waiting()` returned. Whatever is
    // still running is cancelled and given a bounded grace so tool
    // implementations can terminate their child process trees.
    if !drain_mcp_shutdown(&in_flight, EOF_CANCELLATION_GRACE) {
        eprintln!(
            "unica mcp shutdown grace expired while tool calls or provider workers were cleaning up"
        );
    }
    runtime.shutdown_timeout(RUNTIME_SHUTDOWN_GRACE);
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
            structured_tools: crate::application::tools()
                .into_iter()
                .filter_map(|spec| has_structured_output(&spec).then_some(spec.name))
                .collect(),
            startup_notice,
        }
    }

    #[cfg(test)]
    fn with_canonical_v13(handler: Arc<CanonicalToolCallHandler>) -> Self {
        let unavailable: Arc<CanonicalTaskHandler> = Arc::new(|_, _| {
            Err(crate::infrastructure::daemon::client::DaemonTaskExchangeError::Transport)
        });
        Self::with_canonical_v13_tasks(handler, Arc::clone(&unavailable), unavailable)
    }

    #[cfg(test)]
    fn with_canonical_v13_tasks(
        call: Arc<CanonicalToolCallHandler>,
        get: Arc<CanonicalTaskHandler>,
        cancel: Arc<CanonicalTaskHandler>,
    ) -> Self {
        let wait_get = Arc::clone(&get);
        let wait: Arc<CanonicalTaskWaitHandler> =
            Arc::new(move |task_id, _, deadline| wait_get(task_id, deadline));
        Self::with_canonical_v13_task_handlers(call, get, wait, cancel)
    }

    #[cfg(test)]
    fn with_canonical_v13_task_handlers(
        call: Arc<CanonicalToolCallHandler>,
        get: Arc<CanonicalTaskHandler>,
        wait: Arc<CanonicalTaskWaitHandler>,
        cancel: Arc<CanonicalTaskHandler>,
    ) -> Self {
        Self {
            router: SurfaceToolRouter::CanonicalV13(CanonicalV13Router {
                call,
                get,
                wait,
                cancel,
            }),
            in_flight: Arc::new(InFlightRegistry::default()),
            structured_tools: HashSet::new(),
            startup_notice: None,
        }
    }

    fn canonical_v13_daemon(
        owner: crate::infrastructure::daemon::client::DaemonOwner,
        workspace_hint: String,
        startup_notice: Option<String>,
    ) -> Self {
        let router = canonical_daemon_router(owner, workspace_hint);
        Self {
            router: SurfaceToolRouter::CanonicalV13(router),
            in_flight: Arc::new(InFlightRegistry::default()),
            structured_tools: HashSet::new(),
            startup_notice,
        }
    }

    #[cfg(test)]
    fn with_canonical_daemon(
        owner: crate::infrastructure::daemon::client::DaemonOwner,
        workspace_hint: String,
    ) -> Self {
        Self::canonical_v13_daemon(owner, workspace_hint, None)
    }

    fn in_flight(&self) -> Arc<InFlightRegistry> {
        Arc::clone(&self.in_flight)
    }
}

fn remaining_invocation_budget(
    received_at: Instant,
    now: Instant,
    host_remaining_at_receipt: Option<Duration>,
) -> Duration {
    let elapsed = now.saturating_duration_since(received_at);
    let own_remaining = INVOCATION_HANDOFF_WINDOW.saturating_sub(elapsed);
    let host_remaining =
        host_remaining_at_receipt.map(|remaining| remaining.saturating_sub(elapsed));
    own_remaining.min(handoff_budget(host_remaining))
}

fn execute_surface_tool(
    router: &SurfaceToolRouter,
    name: &str,
    arguments: &Map<String, Value>,
    cancellation: CancellationToken,
    progress: Arc<dyn ProgressSink>,
    deadline: FrontendInvocationDeadline,
    client_supports_tasks: bool,
) -> Result<SurfaceToolOutcome, (i32, String)> {
    match router {
        SurfaceToolRouter::LegacyV12(handler) => handler(name, arguments, cancellation, progress)
            .map(Box::new)
            .map(SurfaceToolOutcome::Legacy),
        SurfaceToolRouter::CanonicalV13(router) => {
            if let Some(request) =
                crate::application::v13::task_tools::parse_task_tool_call(name, arguments)
            {
                if client_supports_tasks {
                    return Err((
                        ErrorCode::INVALID_PARAMS.0,
                        "compatibility task tools are unavailable when native Tasks is active"
                            .to_string(),
                    ));
                }
                return Ok(SurfaceToolOutcome::Canonical(
                    execute_compatibility_task_tool(router, request, deadline),
                ));
            }
            let tool = ToolIdentity::from_wire_name(name).ok_or_else(|| {
                (
                    ErrorCode::INVALID_PARAMS.0,
                    "tool is not in the canonical v0.13 profile".to_string(),
                )
            })?;
            match (router.call)(tool, arguments, deadline, cancellation)? {
                InvocationResponse::Direct(result) => Ok(SurfaceToolOutcome::Canonical(result)),
                InvocationResponse::Task(snapshot) if client_supports_tasks => {
                    Ok(SurfaceToolOutcome::Task(snapshot))
                }
                InvocationResponse::Task(snapshot) => Ok(SurfaceToolOutcome::Canonical(
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
    router: &CanonicalV13Router,
    request: Result<crate::application::v13::task_tools::TaskToolRequest, TaskToolError>,
    deadline: FrontendInvocationDeadline,
) -> crate::domain::invocation::DomainResult {
    let request = match request {
        Ok(request) => request,
        Err(error) => return crate::application::v13::task_tools::task_tool_error_result(error),
    };
    let exchange = match request.action {
        TaskToolAction::Get => (router.get)(request.task_id, deadline),
        TaskToolAction::Result { wait_ms } => {
            let bounded = bounded_compatibility_wait_ms(wait_ms, deadline, Instant::now());
            (router.wait)(request.task_id, bounded, deadline)
        }
        TaskToolAction::Cancel => (router.cancel)(request.task_id, deadline),
    };
    let snapshot = match exchange {
        Ok(snapshot) if snapshot.task_id == request.task_id => snapshot,
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

fn compatibility_wait_transport_cutoff(
    requested_wait_ms: u64,
    deadline: FrontendInvocationDeadline,
    now: Instant,
) -> Instant {
    let requested_cutoff = now
        .checked_add(
            Duration::from_millis(requested_wait_ms).saturating_add(RESPONSE_SERIALIZATION_MARGIN),
        )
        .expect("bounded compatibility wait cutoff");
    requested_cutoff.min(deadline.transport_cutoff())
}

fn compatibility_task_exchange_error(
    error: crate::infrastructure::daemon::client::DaemonTaskExchangeError,
) -> TaskToolError {
    use crate::infrastructure::daemon::client::DaemonTaskExchangeError;
    use crate::infrastructure::daemon::protocol::DaemonErrorCode;

    match error {
        DaemonTaskExchangeError::Protocol(DaemonErrorCode::TaskNotFound) => {
            TaskToolError::TaskNotFound
        }
        DaemonTaskExchangeError::Protocol(DaemonErrorCode::TaskExpired) => {
            TaskToolError::TaskExpired
        }
        DaemonTaskExchangeError::Protocol(_) => TaskToolError::TaskBackendFailed,
        DaemonTaskExchangeError::Transport => TaskToolError::TaskTransportFailed,
        DaemonTaskExchangeError::SessionPoisoned => TaskToolError::TaskSessionClosed,
        DaemonTaskExchangeError::UnexpectedResponse => TaskToolError::TaskProtocolFailed,
    }
}

fn project_compatibility_snapshot(
    snapshot: &crate::infrastructure::daemon::protocol::DaemonTaskSnapshot,
    projection: CompatibilityProjection,
) -> crate::domain::invocation::DomainResult {
    let state = CompatibilityTaskSnapshot::new(
        snapshot.task_id,
        snapshot.status,
        snapshot.result.clone(),
        snapshot.failure.is_some(),
        snapshot.created_at_epoch_ms,
        snapshot.updated_at_epoch_ms,
        snapshot.ttl_ms,
        snapshot.poll_interval_ms,
    );
    crate::application::v13::task_tools::project_task_snapshot(&state, projection).unwrap_or_else(
        |_| {
            crate::application::v13::task_tools::task_tool_error_result(
                TaskToolError::ProjectionFailed,
            )
        },
    )
}

/// Build the canonical router backed by a persistent v3 user daemon. The
/// production stdio entrypoint owns one daemon lease for its session and uses
/// this router for every canonical tool and compatibility Task operation.
fn canonical_daemon_router(
    owner: crate::infrastructure::daemon::client::DaemonOwner,
    workspace_hint: String,
) -> CanonicalV13Router {
    // Retain one owner lease for the frontend lifetime, but give each Invocation its own protocol
    // session. A slow direct response therefore cannot serialize another call ahead of its
    // seven-second handoff boundary.
    let anchor = Arc::new(owner);
    let call_anchor = Arc::clone(&anchor);
    let call: Arc<CanonicalToolCallHandler> =
        Arc::new(move |tool, arguments, deadline, _cancellation| {
            let mut owner = call_anchor
                .connect_peer(deadline.remaining_transport_at(Instant::now()))
                .map_err(|message| (TOOL_EXECUTION_ERROR, message))?;
            let response_budget = deadline.remaining_at(Instant::now());
            let request = InvocationRequest::new(
                tool,
                Value::Object(arguments.clone()),
                workspace_hint.clone(),
                response_budget.as_millis().min(7_000) as u64,
            )
            .map_err(|message| (ErrorCode::INVALID_PARAMS.0, message))?;
            owner
                .submit_invocation_with_transport_budget(
                    request,
                    deadline.remaining_transport_at(Instant::now()),
                )
                .map_err(|message| (TOOL_EXECUTION_ERROR, message))
        });
    let get_anchor = Arc::clone(&anchor);
    let get: Arc<CanonicalTaskHandler> = Arc::new(move |task_id, deadline| {
        let task_deadline = get_anchor.begin_task_deadline_at(deadline.transport_cutoff())?;
        let mut owner = get_anchor.connect_peer_before(&task_deadline)?;
        owner.get_task_before(task_id, &task_deadline)
    });
    let wait_anchor = Arc::clone(&anchor);
    let wait: Arc<CanonicalTaskWaitHandler> = Arc::new(move |task_id, wait_ms, deadline| {
        let operation_cutoff =
            compatibility_wait_transport_cutoff(wait_ms, deadline, Instant::now());
        let task_deadline = wait_anchor.begin_task_deadline_at(operation_cutoff)?;
        let mut owner = wait_anchor.connect_peer_before(&task_deadline)?;
        owner.wait_task_before(task_id, wait_ms, &task_deadline)
    });
    let cancel: Arc<CanonicalTaskHandler> = Arc::new(move |task_id, deadline| {
        let task_deadline = anchor.begin_task_deadline_at(deadline.transport_cutoff())?;
        let mut owner = anchor.connect_peer_before(&task_deadline)?;
        owner.cancel_task_before(task_id, &task_deadline)
    });
    CanonicalV13Router {
        call,
        get,
        wait,
        cancel,
    }
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
        let capabilities = match &self.router {
            SurfaceToolRouter::LegacyV12(_) => ServerCapabilities::builder().enable_tools().build(),
            SurfaceToolRouter::CanonicalV13(_) => ServerCapabilities::builder()
                .enable_tools()
                .enable_tasks()
                .build(),
        };
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
        let admission = self
            .in_flight
            .admit()
            .map_err(|message| ErrorData::new(ErrorCode::INTERNAL_ERROR, message, None))?;
        let cancellation = admission.token();

        // `notifications/cancelled` cancels the SDK request token; bridge it to
        // the domain token the blocking tool implementation polls.
        let sdk_token = context.ct.clone();
        let bridged = cancellation.clone();
        let bridge = tokio::spawn(async move {
            sdk_token.cancelled().await;
            bridged.cancel();
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
            let deadline = FrontendInvocationDeadline::new(received_at, None);
            execute_surface_tool(
                &router,
                &handler_name,
                &arguments,
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
        bridge.abort();
        drop(admission);

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
            Ok(Ok(SurfaceToolOutcome::Task(snapshot))) => {
                crate::interfaces::task_projection::create_task_result(&snapshot)
                    .map(CallToolResponse::from)
                    .map_err(crate::interfaces::task_projection::projection_error)
            }
            Ok(Err((code, message))) => Err(ErrorData::new(ErrorCode(code), message, None)),
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
        crate::interfaces::task_projection::detailed_task(&snapshot)
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

fn canonical_task_router(router: &SurfaceToolRouter) -> Result<CanonicalV13Router, ErrorData> {
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
    snapshot: &crate::infrastructure::daemon::protocol::DaemonTaskSnapshot,
) -> Result<(), ErrorData> {
    if snapshot.task_id == expected {
        Ok(())
    } else {
        Err(task_internal_error("task_protocol_failed"))
    }
}

fn project_task_exchange_error(
    error: crate::infrastructure::daemon::client::DaemonTaskExchangeError,
) -> ErrorData {
    use crate::infrastructure::daemon::client::DaemonTaskExchangeError;
    use crate::infrastructure::daemon::protocol::DaemonErrorCode;

    match error {
        DaemonTaskExchangeError::Protocol(DaemonErrorCode::TaskNotFound) => {
            ErrorData::invalid_params(
                "task_not_found",
                Some(serde_json::json!({"code": "task_not_found"})),
            )
        }
        DaemonTaskExchangeError::Protocol(DaemonErrorCode::TaskExpired) => {
            ErrorData::invalid_params(
                "task_expired",
                Some(serde_json::json!({"code": "task_expired"})),
            )
        }
        DaemonTaskExchangeError::Protocol(_) => task_internal_error("task_backend_failed"),
        DaemonTaskExchangeError::Transport => task_internal_error("task_transport_failed"),
        DaemonTaskExchangeError::SessionPoisoned => task_internal_error("task_session_closed"),
        DaemonTaskExchangeError::UnexpectedResponse => task_internal_error("task_protocol_failed"),
    }
}

fn task_internal_error(code: &'static str) -> ErrorData {
    ErrorData::new(
        ErrorCode::INTERNAL_ERROR,
        code,
        Some(serde_json::json!({"code": code})),
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

/// Tracks running tool calls so shutdown can cancel them and wait, and so
/// admission stays bounded without relying on SDK internals.
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
        if state.running.len() >= MCP_MAX_TOOL_WORKERS {
            return Err(format!(
                "dispatcher overloaded: at most {MCP_MAX_TOOL_WORKERS} concurrent tools/call requests are allowed"
            ));
        }
        state.next_id += 1;
        let id = state.next_id;
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
    use crate::application::{ResultContract, ToolExecution};
    use crate::domain::cache::CacheReport;
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
        let canonical: Arc<CanonicalToolCallHandler> = Arc::new(|_, _, _, _| {
            Ok(InvocationResponse::Direct(
                crate::domain::invocation::DomainResult::success("canonical"),
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
                "unica.find",
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
                "unica.find",
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
            "unica.check",
            &Map::new(),
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
        let canonical: Arc<CanonicalToolCallHandler> = Arc::new(move |tool, _, deadline, _| {
            assert_eq!(tool, ToolIdentity::Check);
            assert_eq!(deadline.remaining_at(received), Duration::from_secs(7));
            daemon_observed.fetch_add(1, Ordering::SeqCst);
            Ok(InvocationResponse::Direct(
                crate::domain::invocation::DomainResult::success("canonical"),
            ))
        });
        let v13 = UnicaServer::with_canonical_v13(canonical);
        let result = execute_surface_tool(
            &v13.router,
            "unica.check",
            &Map::new(),
            CancellationToken::new(),
            Arc::new(NoopProgressSink),
            deadline,
            true,
        )
        .unwrap();
        let SurfaceToolOutcome::Canonical(result) = result else {
            panic!("v0.13 direct calls must retain the canonical result envelope");
        };
        assert_eq!(result.summary, "canonical");
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
            let line = timeout(TEST_STEP, self.reader.next_line())
                .await
                .expect("timed out waiting for MCP response")
                .expect("MCP transport failed")
                .expect("MCP server closed the stream before responding");
            serde_json::from_str(&line).expect("MCP server emitted invalid JSON")
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
            match server.serve(server_io).await {
                Ok(running) => {
                    let _ = running.waiting().await;
                }
                Err(ServerInitializeError::ConnectionClosed(_)) => {}
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
        // ADR-0074: the applied call is no longer refused before discovery, so
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

    #[tokio::test]
    async fn tools_list_round_trips_the_data_driven_registry() {
        let (mut client, _) = spawn_server(application_handler());
        client.initialize().await;
        client
            .send(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {} }))
            .await;
        let response = client.receive().await;
        let listed = response["result"]["tools"].as_array().unwrap();
        assert_eq!(listed[0]["name"], "unica.cf.edit");
        assert!(listed
            .iter()
            .any(|tool| tool["name"] == "unica.project.status"));
        // This is the actual SDK projection hosts place in model context, not
        // just the two largest source schemas measured in isolation.
        let compact_result_bytes = serde_json::to_vec(&response["result"]).unwrap().len();
        eprintln!("tools/list compact JSON bytes: {compact_result_bytes}");
        // Release baseline for the typed Meta surface (2026-08-04): 1,275,431
        // bytes. Keep a narrow ratchet here; the follow-up reduction target is
        // recorded in the implementation plan instead of silently spending
        // more model-context budget.
        assert!(
            compact_result_bytes < 1_285_000,
            "tools/list result consumes {compact_result_bytes} compact JSON bytes"
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
        assert!(legacy["result"]["ttlMs"].is_null(), "got {legacy}");
        assert!(legacy["result"]["cacheScope"].is_null(), "got {legacy}");
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
        }
    }

    fn canonical_snapshot(
        task_id: crate::domain::invocation::TaskId,
        status: crate::domain::invocation::InvocationStatus,
        result: Option<crate::domain::invocation::DomainResult>,
    ) -> crate::infrastructure::daemon::protocol::DaemonTaskSnapshot {
        crate::infrastructure::daemon::protocol::DaemonTaskSnapshot {
            task_id,
            invocation_id: crate::domain::invocation::InvocationId::new(),
            status,
            result,
            failure: None,
            poll_interval_ms: 250,
            created_at_epoch_ms: 1_777_012_345_678,
            updated_at_epoch_ms: 1_777_012_346_789,
            ttl_ms: 3_600_000,
        }
    }

    fn canonical_profile_server() -> UnicaServer {
        let task_id = crate::domain::invocation::TaskId::new();
        let call: Arc<CanonicalToolCallHandler> = Arc::new(move |_, _, _, _| {
            Ok(InvocationResponse::Task(canonical_snapshot(
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
        response["result"]["tools"]
            .as_array()
            .expect("tools/list must return tools")
            .iter()
            .map(|tool| tool["name"].as_str().expect("tool name").to_string())
            .collect()
    }

    fn assert_v13_profile_names(names: &[String], native_tasks: bool) {
        let mut expected = vec![
            "unica.view",
            "unica.apply",
            "unica.find",
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

    async fn compatibility_receipts_case() {
        use crate::domain::invocation::{InvocationStatus, TaskId};
        use std::sync::atomic::AtomicUsize;

        let task_id = TaskId::new();
        let executions = Arc::new(AtomicUsize::new(0));
        let execution_observed = Arc::clone(&executions);
        let call: Arc<CanonicalToolCallHandler> = Arc::new(move |_, _, _, _| {
            execution_observed.fetch_add(1, Ordering::SeqCst);
            Ok(InvocationResponse::Task(canonical_snapshot(
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
            compatibility_wait_transport_cutoff(0, deadline, received),
            received + Duration::from_millis(125)
        );
        assert_eq!(
            compatibility_wait_transport_cutoff(1, deadline, received),
            received + Duration::from_millis(126)
        );
        assert_eq!(
            compatibility_wait_transport_cutoff(7_000, deadline, received),
            received + Duration::from_millis(7_125)
        );
        assert_eq!(
            compatibility_wait_transport_cutoff(
                7_000,
                deadline,
                received + Duration::from_millis(6_999),
            ),
            received + Duration::from_millis(7_125),
            "elapsed frontend time is not replenished by the compatibility wait"
        );
        assert_eq!(
            compatibility_wait_transport_cutoff(
                7_000,
                FrontendInvocationDeadline::new(received, Some(Duration::from_millis(80))),
                received,
            ),
            received + Duration::from_millis(80),
            "an earlier host deadline is stronger than waitMs plus response margin"
        );
    }

    async fn compatibility_terminal_result_case() {
        use crate::domain::invocation::{InvocationStatus, TaskId};
        use std::sync::atomic::AtomicUsize;

        let task_id = TaskId::new();
        let subject = canonical_result("same terminal subject result");
        let executions = Arc::new(AtomicUsize::new(0));
        let execution_observed = Arc::clone(&executions);
        let direct_subject = subject.clone();
        let call: Arc<CanonicalToolCallHandler> = Arc::new(move |_, arguments, _, _| {
            execution_observed.fetch_add(1, Ordering::SeqCst);
            if arguments.get("direct").and_then(Value::as_bool) == Some(true) {
                Ok(InvocationResponse::Direct(direct_subject.clone()))
            } else {
                Ok(InvocationResponse::Task(canonical_snapshot(
                    task_id,
                    InvocationStatus::Working,
                    None,
                )))
            }
        });
        let get: Arc<CanonicalTaskHandler> =
            Arc::new(move |_, _| Ok(canonical_snapshot(task_id, InvocationStatus::Working, None)));
        let waits = Arc::new(Mutex::new(Vec::<u64>::new()));
        let waits_observed = Arc::clone(&waits);
        let wait_subject = subject.clone();
        let wait: Arc<CanonicalTaskWaitHandler> = Arc::new(move |_, wait_ms, _| {
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

    async fn compatibility_closed_errors_case() {
        use crate::domain::invocation::{InvocationStatus, TaskId};
        use crate::infrastructure::daemon::client::DaemonTaskExchangeError;
        use crate::infrastructure::daemon::protocol::DaemonErrorCode;
        use std::sync::atomic::AtomicUsize;

        let known = TaskId::new();
        let unknown = TaskId::new();
        let expired = TaskId::new();
        let subject_executions = Arc::new(AtomicUsize::new(0));
        let subject_observed = Arc::clone(&subject_executions);
        let call: Arc<CanonicalToolCallHandler> = Arc::new(move |_, _, _, _| {
            subject_observed.fetch_add(1, Ordering::SeqCst);
            Ok(InvocationResponse::Task(canonical_snapshot(
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
                Err(DaemonTaskExchangeError::Protocol(
                    DaemonErrorCode::TaskNotFound,
                ))
            } else {
                Ok(canonical_snapshot(known, InvocationStatus::Working, None))
            }
        });
        let wait_calls = Arc::new(AtomicUsize::new(0));
        let wait_observed = Arc::clone(&wait_calls);
        let wait: Arc<CanonicalTaskWaitHandler> = Arc::new(move |task_id, _, _| {
            wait_observed.fetch_add(1, Ordering::SeqCst);
            if task_id == expired {
                Err(DaemonTaskExchangeError::Protocol(
                    DaemonErrorCode::TaskExpired,
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

    struct CompatibilityRestartService {
        executions: Arc<AtomicUsize>,
    }

    impl crate::infrastructure::daemon::server::CanonicalInvocationService
        for CompatibilityRestartService
    {
        fn prepare(
            &self,
            _invocation: &crate::infrastructure::daemon::server::ActorBoundInvocation,
        ) -> Result<
            crate::application::operation_descriptors::ExecutionClass,
            Box<crate::domain::invocation::DomainResult>,
        > {
            Ok(
                crate::application::operation_descriptors::ExecutionClass::KnownLong(
                    crate::application::operation_descriptors::KnownLongReason::ExternalProcess,
                ),
            )
        }

        fn execute(
            &self,
            _invocation: &crate::infrastructure::daemon::server::ActorBoundExecution,
            _cancellation: crate::domain::cancellation::CancellationToken,
        ) -> Result<
            crate::domain::invocation::DomainResult,
            crate::domain::invocation::InvocationFailure,
        > {
            self.executions.fetch_add(1, Ordering::SeqCst);
            Ok(crate::domain::invocation::DomainResult::success(
                "durable compatibility result",
            ))
        }
    }

    fn wait_for_compatibility_daemon(
        state_root: &std::path::Path,
        identity: &crate::infrastructure::daemon::identity::CoreIdentity,
        daemon_done: &std::sync::mpsc::Receiver<Result<(), String>>,
        phase: &str,
    ) -> crate::infrastructure::daemon::protocol::EndpointRecord {
        let deadline = Instant::now() + TEST_STEP;
        loop {
            let state = crate::infrastructure::daemon::identity::DaemonStateDirectory::open(
                state_root, identity,
            )
            .unwrap();
            if let Some(record) = state.read_endpoint_record().unwrap() {
                return record;
            }
            match daemon_done.try_recv() {
                Ok(Ok(())) => {
                    panic!("{phase} compatibility daemon exited before its endpoint was observed")
                }
                Ok(Err(error)) => panic!(
                    "{phase} compatibility daemon failed before publishing its endpoint: {error}"
                ),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => panic!(
                    "{phase} compatibility daemon outcome channel disconnected before publication"
                ),
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
            assert!(
                Instant::now() < deadline,
                "{phase} compatibility daemon endpoint was not published"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn start_compatibility_daemon_with_anchor(
        config: crate::infrastructure::daemon::server::DaemonServerConfig,
        state_root: &std::path::Path,
        identity: &crate::infrastructure::daemon::identity::CoreIdentity,
        observation_delay: Duration,
        phase: &str,
    ) -> (
        std::thread::JoinHandle<Result<(), String>>,
        std::net::TcpStream,
    ) {
        use crate::infrastructure::daemon::protocol::{
            read_bounded_json_line, ClientRequest, ServerResponse, DAEMON_PROTOCOL_VERSION,
        };
        use crate::infrastructure::daemon::server::{install_startup_pause, run_daemon};
        use std::io::{BufReader as StdBufReader, Write};

        let startup_pause = install_startup_pause();
        let config = config.with_startup_pause(&startup_pause);
        let (daemon_done, daemon_done_wait) = std::sync::mpsc::channel();
        let daemon = std::thread::spawn(move || {
            let outcome = run_daemon(config);
            let _ = daemon_done.send(outcome.clone());
            outcome
        });

        // Deliberately model a runner scheduling gap longer than the short test idle grace.
        // The post-publication gate must retain the endpoint until an authenticated owner is
        // already queued; merely polling the endpoint leaves a publication-to-connect race.
        std::thread::sleep(observation_delay);
        let record = wait_for_compatibility_daemon(state_root, identity, &daemon_done_wait, phase);

        let mut anchor = std::net::TcpStream::connect(record.loopback_addr().unwrap()).unwrap();
        anchor.set_read_timeout(Some(TEST_STEP)).unwrap();
        let mut hello = serde_json::to_vec(&ClientRequest::hello(
            DAEMON_PROTOCOL_VERSION,
            record.token().to_string(),
            identity.clone(),
        ))
        .unwrap();
        hello.push(b'\n');
        anchor.write_all(&hello).unwrap();
        anchor.flush().unwrap();
        startup_pause.release();

        let ready: ServerResponse = serde_json::from_slice(
            &read_bounded_json_line(&mut StdBufReader::new(anchor.try_clone().unwrap())).unwrap(),
        )
        .unwrap();
        assert!(
            ready.matches_record(&record),
            "{phase} compatibility daemon rejected its queued startup anchor: {ready:?}"
        );
        (daemon, anchor)
    }

    fn connect_compatibility_daemon(
        state_root: std::path::PathBuf,
        identity: crate::infrastructure::daemon::identity::CoreIdentity,
    ) -> crate::infrastructure::daemon::client::DaemonOwner {
        let client = crate::infrastructure::daemon::client::DaemonClient::new(
            crate::infrastructure::daemon::client::DaemonClientConfig::existing_only(
                state_root, identity,
            ),
        );
        match client.connect_existing().unwrap() {
            crate::infrastructure::daemon::client::ExistingDaemon::Connected(owner) => owner,
            crate::infrastructure::daemon::client::ExistingDaemon::Absent => {
                panic!("published compatibility daemon must connect")
            }
        }
    }

    async fn compatibility_daemon_restart_case() {
        use crate::domain::invocation::TaskId;
        use crate::infrastructure::daemon::identity::CoreIdentity;
        use crate::infrastructure::daemon::server::DaemonServerConfig;
        use std::str::FromStr;

        let state = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        std::fs::write(
            workspace.path().join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: .\n",
        )
        .unwrap();
        let state_root = std::fs::canonicalize(state.path()).unwrap();
        let workspace_hint = std::fs::canonicalize(workspace.path())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let identity = CoreIdentity::production();
        let executions = Arc::new(AtomicUsize::new(0));
        let service = Arc::new(CompatibilityRestartService {
            executions: Arc::clone(&executions),
        });
        let idle_grace = Duration::from_millis(350);
        let first_config =
            DaemonServerConfig::new(state_root.clone(), identity.clone(), idle_grace)
                .with_invocation_service(service);
        let (first_daemon, first_startup_anchor) = start_compatibility_daemon_with_anchor(
            first_config,
            &state_root,
            &identity,
            idle_grace * 2,
            "first",
        );

        let first_owner = connect_compatibility_daemon(state_root.clone(), identity.clone());
        drop(first_startup_anchor);
        let (mut first, _) = spawn_unica_server(UnicaServer::with_canonical_daemon(
            first_owner,
            workspace_hint.clone(),
        ));
        first
            .send(json!({
                "jsonrpc":"2.0", "id":1, "method":"tools/call",
                "params":{
                    "name":"unica.run",
                    "arguments":{"op":"infobase.build", "args":{}},
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
        assert_eq!(executions.load(Ordering::SeqCst), 1);

        first.shutdown().await;
        first_daemon.join().unwrap().unwrap();

        let second_config =
            DaemonServerConfig::new(state_root.clone(), identity.clone(), idle_grace);
        let (second_daemon, second_startup_anchor) = start_compatibility_daemon_with_anchor(
            second_config,
            &state_root,
            &identity,
            Duration::ZERO,
            "second",
        );
        let second_owner = connect_compatibility_daemon(state_root, identity);
        drop(second_startup_anchor);
        let (mut second, _) = spawn_unica_server(UnicaServer::with_canonical_daemon(
            second_owner,
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
        assert_eq!(executions.load(Ordering::SeqCst), 1);
        second.shutdown().await;
        second_daemon.join().unwrap().unwrap();
    }

    #[tokio::test]
    async fn compatibility_tools_return_durable_receipts_without_native_tasks_or_reexecution() {
        compatibility_receipts_case().await;
    }

    #[test]
    fn compatibility_result_wait_is_bounded_by_request_and_original_frontend_window() {
        compatibility_wait_budget_case();
    }

    async fn compatibility_wait_single_deadline_case() {
        use crate::infrastructure::daemon::client::{
            DaemonClient, DaemonClientConfig, ExistingDaemon, ManualDaemonClientClock,
        };
        use crate::infrastructure::daemon::identity::{CoreIdentity, DaemonStateDirectory};
        use crate::infrastructure::daemon::protocol::{
            read_bounded_json_line, ClientRequest, DaemonTaskSnapshot, EndpointRecord,
            ServerResponse,
        };
        use std::io::{BufReader as StdBufReader, Write};
        use std::net::{Ipv4Addr, TcpListener};
        use std::thread;

        fn write_line(stream: &mut std::net::TcpStream, response: &ServerResponse) {
            serde_json::to_writer(&mut *stream, response).unwrap();
            stream.write_all(b"\n").unwrap();
            stream.flush().unwrap();
        }

        for requested_wait_ms in [0_u64, 1] {
            let state = tempfile::tempdir().unwrap();
            let state_root = std::fs::canonicalize(state.path()).unwrap();
            let identity = CoreIdentity::production();
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
            let record =
                EndpointRecord::new(identity.clone(), listener.local_addr().unwrap().port());
            let directory = DaemonStateDirectory::open(&state_root, &identity).unwrap();
            directory.write_endpoint_record_for_test(&record).unwrap();
            let clock = ManualDaemonClientClock::new();
            let peer_clock = clock.clone();
            let peer_record = record.clone();
            let task_id = crate::domain::invocation::TaskId::new();
            let fake_peer = thread::spawn(move || {
                let (mut anchor, _) = listener.accept().unwrap();
                let _hello = read_bounded_json_line(&mut StdBufReader::new(&anchor)).unwrap();
                write_line(&mut anchor, &ServerResponse::ready(&peer_record));

                let (mut operation, _) = listener.accept().unwrap();
                let mut operation_reader = StdBufReader::new(operation.try_clone().unwrap());
                let _hello = read_bounded_json_line(&mut operation_reader).unwrap();
                peer_clock.advance(Duration::from_millis(60));
                write_line(&mut operation, &ServerResponse::ready(&peer_record));

                let request = crate::infrastructure::daemon::protocol::parse_request(
                    &read_bounded_json_line(&mut operation_reader).unwrap(),
                )
                .unwrap();
                assert_eq!(
                    request,
                    ClientRequest::wait_task(task_id, 0),
                    "connect time consumes the wait slice before the 125ms response margin"
                );
                peer_clock.advance(Duration::from_millis(1_065 + requested_wait_ms));
                write_line(
                    &mut operation,
                    &ServerResponse::task(DaemonTaskSnapshot::working_for_test(task_id)),
                );
            });
            let client = DaemonClient::new(
                DaemonClientConfig::existing_only(state_root, identity).with_clock_for_test(clock),
            );
            let owner = match client.connect_existing().unwrap() {
                ExistingDaemon::Connected(owner) => owner,
                ExistingDaemon::Absent => panic!("fake compatibility daemon must connect"),
            };
            let (mut mcp, _) = spawn_unica_server(UnicaServer::with_canonical_daemon(
                owner,
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
            mcp.shutdown().await;
            fake_peer.join().unwrap();
        }
    }

    #[tokio::test]
    async fn compatibility_wait_zero_and_one_share_one_budget_across_connect_and_response() {
        compatibility_wait_single_deadline_case().await;
    }

    async fn compatibility_wait_frontend_cutoff_is_not_rebased_case() {
        use crate::infrastructure::daemon::client::{
            DaemonClient, DaemonClientConfig, ExistingDaemon, ManualDaemonClientClock,
        };
        use crate::infrastructure::daemon::identity::{CoreIdentity, DaemonStateDirectory};
        use crate::infrastructure::daemon::protocol::{
            read_bounded_json_line, DaemonTaskSnapshot, EndpointRecord, ServerResponse,
        };
        use std::io::{BufReader as StdBufReader, ErrorKind, Write};
        use std::net::{Ipv4Addr, TcpListener};
        use std::sync::atomic::AtomicUsize;
        use std::thread;

        fn write_line(stream: &mut std::net::TcpStream, response: &ServerResponse) {
            serde_json::to_writer(&mut *stream, response).unwrap();
            stream.write_all(b"\n").unwrap();
            stream.flush().unwrap();
        }

        let state = tempfile::tempdir().unwrap();
        let state_root = std::fs::canonicalize(state.path()).unwrap();
        let identity = CoreIdentity::production();
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let record = EndpointRecord::new(identity.clone(), listener.local_addr().unwrap().port());
        let directory = DaemonStateDirectory::open(&state_root, &identity).unwrap();
        directory.write_endpoint_record_for_test(&record).unwrap();
        let task_id = crate::domain::invocation::TaskId::new();
        let peer_record = record.clone();
        let operation_requests = Arc::new(AtomicUsize::new(0));
        let peer_requests = Arc::clone(&operation_requests);
        let (stop, stop_wait) = mpsc::channel();
        let fake_peer = thread::spawn(move || {
            let (mut anchor, _) = listener.accept().unwrap();
            let _hello = read_bounded_json_line(&mut StdBufReader::new(&anchor)).unwrap();
            write_line(&mut anchor, &ServerResponse::ready(&peer_record));

            listener.set_nonblocking(true).unwrap();
            loop {
                match listener.accept() {
                    Ok((mut operation, _)) => {
                        operation.set_nonblocking(false).unwrap();
                        let mut reader = StdBufReader::new(operation.try_clone().unwrap());
                        let _hello = read_bounded_json_line(&mut reader).unwrap();
                        write_line(&mut operation, &ServerResponse::ready(&peer_record));
                        let _request = read_bounded_json_line(&mut reader).unwrap();
                        peer_requests.fetch_add(1, Ordering::SeqCst);
                        write_line(
                            &mut operation,
                            &ServerResponse::task(DaemonTaskSnapshot::working_for_test(task_id)),
                        );
                        break;
                    }
                    Err(error) if error.kind() == ErrorKind::WouldBlock => {
                        if stop_wait.try_recv().is_ok() {
                            break;
                        }
                        thread::sleep(Duration::from_millis(1));
                    }
                    Err(error) => panic!("accept compatibility operation: {error}"),
                }
            }
        });
        let clock = ManualDaemonClientClock::new();
        let client = DaemonClient::new(
            DaemonClientConfig::existing_only(state_root, identity)
                .with_clock_for_test(clock.clone()),
        );
        let owner = match client.connect_existing().unwrap() {
            ExistingDaemon::Connected(owner) => owner,
            ExistingDaemon::Absent => panic!("fake compatibility daemon must connect"),
        };
        // The next daemon-clock sample is deliberately delayed until after the
        // frontend calculated waitMs + margin. A Duration rebase starts a fresh
        // window here; an absolute cutoff is already expired.
        clock.advance_before_next_sample(Duration::from_secs(1));
        let (mut mcp, _) = spawn_unica_server(UnicaServer::with_canonical_daemon(
            owner,
            "/workspace".to_string(),
        ));
        mcp.send(json!({
            "jsonrpc":"2.0", "id":1, "method":"tools/call",
            "params":{
                "name":"unica.task.result",
                "arguments":{"taskId":task_id.to_string(), "waitMs":0},
                "_meta":modern_meta()
            }
        }))
        .await;
        let response = mcp.receive().await;
        let _ = stop.send(());
        mcp.shutdown().await;
        fake_peer.join().unwrap();

        assert_eq!(
            response["result"]["structuredContent"]["diagnostics"][0]["code"],
            "task_transport_failed",
            "the operation must not rebase its cutoff after the injected pause: {response}"
        );
        assert_eq!(
            operation_requests.load(Ordering::SeqCst),
            0,
            "an expired absolute cutoff must stop before operation admission"
        );
    }

    fn compatibility_immediate_task_deadline_case(
        tool_name: &'static str,
        host_budget: Duration,
        handshake_elapsed: Duration,
        response_elapsed: Duration,
    ) -> crate::domain::invocation::DomainResult {
        use crate::infrastructure::daemon::client::{
            DaemonClient, DaemonClientConfig, ExistingDaemon, ManualDaemonClientClock,
        };
        use crate::infrastructure::daemon::identity::{CoreIdentity, DaemonStateDirectory};
        use crate::infrastructure::daemon::protocol::{
            read_bounded_json_line, ClientRequest, DaemonTaskSnapshot, EndpointRecord,
            ServerResponse,
        };
        use std::io::{BufReader as StdBufReader, Write};
        use std::net::{Ipv4Addr, TcpListener};
        use std::thread;

        fn write_line(stream: &mut std::net::TcpStream, response: &ServerResponse) {
            serde_json::to_writer(&mut *stream, response).unwrap();
            stream.write_all(b"\n").unwrap();
            stream.flush().unwrap();
        }

        let state = tempfile::tempdir().unwrap();
        let state_root = std::fs::canonicalize(state.path()).unwrap();
        let identity = CoreIdentity::production();
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let record = EndpointRecord::new(identity.clone(), listener.local_addr().unwrap().port());
        let directory = DaemonStateDirectory::open(&state_root, &identity).unwrap();
        directory.write_endpoint_record_for_test(&record).unwrap();
        let task_id = crate::domain::invocation::TaskId::new();
        let received = Instant::now();
        let clock = ManualDaemonClientClock::new_at(received);
        let peer_clock = clock.clone();
        let peer_record = record.clone();
        let fake_peer = thread::spawn(move || {
            let (mut anchor, _) = listener.accept().unwrap();
            let _hello = read_bounded_json_line(&mut StdBufReader::new(&anchor)).unwrap();
            write_line(&mut anchor, &ServerResponse::ready(&peer_record));

            let (mut operation, _) = listener.accept().unwrap();
            let mut operation_reader = StdBufReader::new(operation.try_clone().unwrap());
            let _hello = read_bounded_json_line(&mut operation_reader).unwrap();
            peer_clock.advance(handshake_elapsed);
            write_line(&mut operation, &ServerResponse::ready(&peer_record));

            let Ok(bytes) = read_bounded_json_line(&mut operation_reader) else {
                return;
            };
            let request = crate::infrastructure::daemon::protocol::parse_request(&bytes).unwrap();
            let expected = match tool_name {
                "unica.task.get" => ClientRequest::get_task(task_id),
                "unica.task.cancel" => ClientRequest::cancel_task(task_id),
                other => panic!("unexpected immediate compatibility tool {other}"),
            };
            assert_eq!(request, expected);
            peer_clock.advance(response_elapsed);
            write_line(
                &mut operation,
                &ServerResponse::task(DaemonTaskSnapshot::working_for_test(task_id)),
            );
        });
        let client = DaemonClient::new(
            DaemonClientConfig::existing_only(state_root, identity).with_clock_for_test(clock),
        );
        let owner = match client.connect_existing().unwrap() {
            ExistingDaemon::Connected(owner) => owner,
            ExistingDaemon::Absent => panic!("fake compatibility daemon must connect"),
        };
        let router = SurfaceToolRouter::CanonicalV13(canonical_daemon_router(
            owner,
            "/workspace".to_string(),
        ));
        let arguments = json!({"taskId": task_id.to_string()})
            .as_object()
            .unwrap()
            .clone();
        let outcome = execute_surface_tool(
            &router,
            tool_name,
            &arguments,
            CancellationToken::new(),
            Arc::new(NoopProgressSink),
            FrontendInvocationDeadline::new(received, Some(host_budget)),
            false,
        )
        .unwrap();
        let SurfaceToolOutcome::Canonical(result) = outcome else {
            panic!("compatibility task tools must return canonical results");
        };
        fake_peer.join().unwrap();
        result
    }

    #[test]
    fn compatibility_get_and_cancel_do_not_replace_open_frontend_cutoff_with_125ms() {
        for tool_name in ["unica.task.get", "unica.task.cancel"] {
            let result = compatibility_immediate_task_deadline_case(
                tool_name,
                Duration::from_millis(500),
                Duration::from_millis(130),
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
                Duration::from_millis(200),
                Duration::from_millis(110),
                Duration::from_millis(110),
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

    fn compatibility_wait_post_parse_cutoff_case(valid_near_limit: bool) {
        use crate::application::invocation_store::MAX_CANONICAL_RESULT_BYTES;
        use crate::domain::invocation::{DomainResult, InvocationStatus};
        use crate::infrastructure::daemon::client::{
            DaemonClient, DaemonClientConfig, DaemonTaskExchangeError, ExistingDaemon,
            ManualDaemonClientClock,
        };
        use crate::infrastructure::daemon::identity::{CoreIdentity, DaemonStateDirectory};
        use crate::infrastructure::daemon::protocol::{
            read_bounded_json_line, EndpointRecord, ServerResponse, MAX_DAEMON_RESPONSE_LINE_BYTES,
        };
        use std::io::{BufReader as StdBufReader, Write};
        use std::net::{Ipv4Addr, TcpListener};
        use std::thread;

        fn write_line(stream: &mut std::net::TcpStream, response: &ServerResponse) {
            serde_json::to_writer(&mut *stream, response).unwrap();
            stream.write_all(b"\n").unwrap();
            stream.flush().unwrap();
        }

        let state = tempfile::tempdir().unwrap();
        let state_root = std::fs::canonicalize(state.path()).unwrap();
        let identity = CoreIdentity::production();
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let record = EndpointRecord::new(identity.clone(), listener.local_addr().unwrap().port());
        let directory = DaemonStateDirectory::open(&state_root, &identity).unwrap();
        directory.write_endpoint_record_for_test(&record).unwrap();
        let task_id = crate::domain::invocation::TaskId::new();
        let peer_record = record.clone();
        let response_payload = if valid_near_limit {
            let snapshot = canonical_snapshot(
                task_id,
                InvocationStatus::Completed,
                Some(DomainResult::success(
                    "x".repeat(MAX_CANONICAL_RESULT_BYTES - 4_096),
                )),
            );
            let mut bytes = serde_json::to_vec(&ServerResponse::task(snapshot)).unwrap();
            bytes.push(b'\n');
            bytes
        } else {
            let mut hostile = br#"{"kind":"task","snapshot":{"unknown":""#.to_vec();
            hostile.extend(std::iter::repeat_n(
                b'x',
                MAX_DAEMON_RESPONSE_LINE_BYTES - hostile.len() - 4_096,
            ));
            hostile.extend_from_slice(b"\"}}\n");
            hostile
        };
        let clock = ManualDaemonClientClock::new();
        let peer_clock = clock.clone();
        let (second_request_seen, second_request_seen_wait) = mpsc::channel();
        let fake_peer = thread::spawn(move || {
            let (mut anchor, _) = listener.accept().unwrap();
            let _hello = read_bounded_json_line(&mut StdBufReader::new(&anchor)).unwrap();
            write_line(&mut anchor, &ServerResponse::ready(&peer_record));

            let (mut operation, _) = listener.accept().unwrap();
            operation
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut reader = StdBufReader::new(operation.try_clone().unwrap());
            let _hello = read_bounded_json_line(&mut reader).unwrap();
            write_line(&mut operation, &ServerResponse::ready(&peer_record));
            let _request = read_bounded_json_line(&mut reader).unwrap();
            peer_clock.advance_during_next_response_parse(Duration::from_millis(2_001));
            operation.write_all(&response_payload).unwrap();
            operation.flush().unwrap();
            let second = read_bounded_json_line(&mut reader).is_ok();
            if second {
                write_line(
                    &mut operation,
                    &ServerResponse::task(
                        crate::infrastructure::daemon::protocol::DaemonTaskSnapshot::working_for_test(
                            task_id,
                        ),
                    ),
                );
            }
            second_request_seen.send(second).unwrap();
        });
        let client = DaemonClient::new(
            DaemonClientConfig::existing_only(state_root, identity).with_clock_for_test(clock),
        );
        let anchor = match client.connect_existing().unwrap() {
            ExistingDaemon::Connected(owner) => owner,
            ExistingDaemon::Absent => panic!("fake compatibility daemon must connect"),
        };
        let deadline = anchor.begin_task_deadline(Duration::from_secs(2)).unwrap();
        let mut operation = anchor.connect_peer_before(&deadline).unwrap();
        let first = operation.wait_task_before(task_id, 0, &deadline);
        let second = operation.get_task(task_id);
        let saw_second = second_request_seen_wait.recv().unwrap();
        fake_peer.join().unwrap();

        assert!(
            matches!(first, Err(DaemonTaskExchangeError::Transport)),
            "a parsed response that crossed cutoff must not publish its snapshot"
        );
        assert!(
            matches!(second, Err(DaemonTaskExchangeError::SessionPoisoned)),
            "post-parse expiry must poison the operation session"
        );
        assert!(
            !saw_second,
            "post-parse expiry must close the operation session before reuse"
        );
    }

    #[tokio::test]
    async fn compatibility_wait_preserves_frontend_cutoff_across_client_admission_pause() {
        compatibility_wait_frontend_cutoff_is_not_rebased_case().await;
    }

    #[test]
    fn compatibility_wait_post_parse_expiry_wins_for_valid_and_malformed_near_limit_frames() {
        compatibility_wait_post_parse_cutoff_case(true);
        compatibility_wait_post_parse_cutoff_case(false);
    }

    fn compatibility_wait_authenticated_long_and_host_cutoff_case(
        requested_wait_ms: u64,
        host_remaining: Option<Duration>,
        expected_daemon_wait_ms: u64,
    ) {
        use crate::infrastructure::daemon::client::{
            DaemonClient, DaemonClientConfig, ExistingDaemon, ManualDaemonClientClock,
        };
        use crate::infrastructure::daemon::identity::{CoreIdentity, DaemonStateDirectory};
        use crate::infrastructure::daemon::protocol::{
            parse_request, read_bounded_json_line, ClientRequest, DaemonTaskSnapshot,
            EndpointRecord, ServerResponse,
        };
        use std::io::{BufReader as StdBufReader, Write};
        use std::net::{Ipv4Addr, TcpListener};
        use std::thread;

        fn write_line(stream: &mut std::net::TcpStream, response: &ServerResponse) {
            serde_json::to_writer(&mut *stream, response).unwrap();
            stream.write_all(b"\n").unwrap();
            stream.flush().unwrap();
        }

        let received = Instant::now();
        let state = tempfile::tempdir().unwrap();
        let state_root = std::fs::canonicalize(state.path()).unwrap();
        let identity = CoreIdentity::production();
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let record = EndpointRecord::new(identity.clone(), listener.local_addr().unwrap().port());
        let directory = DaemonStateDirectory::open(&state_root, &identity).unwrap();
        directory.write_endpoint_record_for_test(&record).unwrap();
        let task_id = crate::domain::invocation::TaskId::new();
        let peer_record = record.clone();
        let clock = ManualDaemonClientClock::new_at(received);
        let peer_clock = clock.clone();
        let (observed_request, observed_request_wait) = mpsc::channel();
        let fake_peer = thread::spawn(move || {
            let (mut anchor, _) = listener.accept().unwrap();
            let _hello = read_bounded_json_line(&mut StdBufReader::new(&anchor)).unwrap();
            write_line(&mut anchor, &ServerResponse::ready(&peer_record));

            let (mut operation, _) = listener.accept().unwrap();
            let mut reader = StdBufReader::new(operation.try_clone().unwrap());
            let _hello = read_bounded_json_line(&mut reader).unwrap();
            peer_clock.advance(Duration::from_millis(60));
            write_line(&mut operation, &ServerResponse::ready(&peer_record));
            let request = parse_request(&read_bounded_json_line(&mut reader).unwrap()).unwrap();
            observed_request.send(request).unwrap();
            write_line(
                &mut operation,
                &ServerResponse::task(DaemonTaskSnapshot::working_for_test(task_id)),
            );
        });
        let client = DaemonClient::new(
            DaemonClientConfig::existing_only(state_root, identity).with_clock_for_test(clock),
        );
        let owner = match client.connect_existing().unwrap() {
            ExistingDaemon::Connected(owner) => owner,
            ExistingDaemon::Absent => panic!("fake compatibility daemon must connect"),
        };
        let router = canonical_daemon_router(owner, "/workspace".to_string());
        let snapshot = (router.wait)(
            task_id,
            requested_wait_ms,
            FrontendInvocationDeadline::new(received, host_remaining),
        )
        .unwrap();
        assert_eq!(snapshot.task_id, task_id);
        assert_eq!(
            observed_request_wait.recv().unwrap(),
            ClientRequest::wait_task(task_id, expected_daemon_wait_ms)
        );
        fake_peer.join().unwrap();
    }

    #[test]
    fn compatibility_wait_authenticated_transport_bounds_7000_and_earlier_host_cutoff() {
        compatibility_wait_authenticated_long_and_host_cutoff_case(7_000, None, 6_940);
        compatibility_wait_authenticated_long_and_host_cutoff_case(
            7_000,
            Some(Duration::from_millis(80)),
            0,
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
        use crate::domain::invocation::{
            DomainResult, InvocationFailure, InvocationStatus, TaskId,
        };

        let statuses = [
            InvocationStatus::Queued,
            InvocationStatus::Working,
            InvocationStatus::Completed,
            InvocationStatus::Failed,
            InvocationStatus::Cancelled,
        ];
        for status in statuses {
            for has_result in [false, true] {
                for has_failure in [false, true] {
                    let valid = matches!(
                        (status, has_result, has_failure),
                        (InvocationStatus::Queued, false, false)
                            | (InvocationStatus::Working, false, false)
                            | (InvocationStatus::Completed, true, false)
                            | (InvocationStatus::Failed, false, true)
                            | (InvocationStatus::Cancelled, false, false)
                    );
                    if valid {
                        continue;
                    }

                    let task_id = TaskId::new();
                    let mut hostile = canonical_snapshot(
                        task_id,
                        status,
                        has_result.then(|| {
                            DomainResult::success(
                                "hostile result /private/result-secret bearer-result",
                            )
                        }),
                    );
                    hostile.failure = has_failure.then(|| {
                        InvocationFailure::new(
                            "hostile_failure_code",
                            "/private/failure-secret bearer-failure",
                        )
                    });
                    let get_snapshot = hostile.clone();
                    let get: Arc<CanonicalTaskHandler> =
                        Arc::new(move |_, _| Ok(get_snapshot.clone()));
                    let wait_snapshot = hostile.clone();
                    let wait: Arc<CanonicalTaskWaitHandler> =
                        Arc::new(move |_, _, _| Ok(wait_snapshot.clone()));
                    let cancel = Arc::clone(&get);
                    let call: Arc<CanonicalToolCallHandler> = Arc::new(move |_, _, _, _| {
                        Ok(InvocationResponse::Direct(DomainResult::success("unused")))
                    });
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
                        assert_eq!(
                            response["result"]["structuredContent"]["diagnostics"][0]["code"],
                            "task_projection_failed",
                            "status={status:?} result={has_result} failure={has_failure}: {response}"
                        );
                        let serialized = serde_json::to_string(&response).unwrap();
                        for forbidden in [
                            "/private/result-secret",
                            "bearer-result",
                            "/private/failure-secret",
                            "bearer-failure",
                            "hostile_failure_code",
                        ] {
                            assert!(!serialized.contains(forbidden), "leaked {forbidden}");
                        }
                    }
                    client.shutdown().await;
                }
            }
        }

        let task_id = TaskId::new();
        let mut failed = canonical_snapshot(task_id, InvocationStatus::Failed, None);
        failed.failure = Some(InvocationFailure::new(
            "hostile_failure_code",
            "/private/failure-secret bearer-failure",
        ));
        let get_failed = failed.clone();
        let get: Arc<CanonicalTaskHandler> = Arc::new(move |_, _| Ok(get_failed.clone()));
        let wait_failed = failed.clone();
        let wait: Arc<CanonicalTaskWaitHandler> = Arc::new(move |_, _, _| Ok(wait_failed.clone()));
        let cancel = Arc::clone(&get);
        let call: Arc<CanonicalToolCallHandler> = Arc::new(move |_, _, _, _| {
            Ok(InvocationResponse::Direct(DomainResult::success("unused")))
        });
        let (mut client, _) = spawn_unica_server(UnicaServer::with_canonical_v13_task_handlers(
            call, get, wait, cancel,
        ));
        client
            .send(json!({
                "jsonrpc":"2.0", "id":1, "method":"tools/call",
                "params":{
                    "name":"unica.task.get", "arguments":{"taskId":task_id.to_string()},
                    "_meta":modern_meta()
                }
            }))
            .await;
        let response = client.receive().await;
        assert_eq!(
            response["result"]["structuredContent"]["diagnostics"][0]["code"], "task_failed",
            "{response}"
        );
        let serialized = serde_json::to_string(&response).unwrap();
        for forbidden in [
            "/private/failure-secret",
            "bearer-failure",
            "hostile_failure_code",
        ] {
            assert!(!serialized.contains(forbidden), "leaked {forbidden}");
        }
        client.shutdown().await;
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
        compatibility_wait_frontend_cutoff_is_not_rebased_case().await;
        compatibility_wait_post_parse_cutoff_case(true);
        compatibility_wait_post_parse_cutoff_case(false);
        compatibility_wait_authenticated_long_and_host_cutoff_case(7_000, None, 6_940);
        compatibility_wait_authenticated_long_and_host_cutoff_case(
            7_000,
            Some(Duration::from_millis(80)),
            0,
        );
        compatibility_terminal_result_case().await;
        compatibility_closed_errors_case().await;
        compatibility_hostile_status_payload_case().await;
        compatibility_daemon_restart_case().await;
    }

    async fn tasks_direct_first_capability_case() {
        use crate::domain::invocation::{InvocationStatus, TaskId};
        use std::sync::atomic::AtomicUsize;

        let task_id = TaskId::new();
        let executions = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&executions);
        let call: Arc<CanonicalToolCallHandler> = Arc::new(move |_, _, _, _| {
            observed.fetch_add(1, Ordering::SeqCst);
            Ok(InvocationResponse::Task(canonical_snapshot(
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
        let call: Arc<CanonicalToolCallHandler> = Arc::new(move |_, _, _, _| {
            observed.fetch_add(1, Ordering::SeqCst);
            Ok(InvocationResponse::Task(canonical_snapshot(
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
        let call: Arc<CanonicalToolCallHandler> = Arc::new(move |_, _, _, _| {
            observed.fetch_add(1, Ordering::SeqCst);
            Ok(InvocationResponse::Task(canonical_snapshot(
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
        let call: Arc<CanonicalToolCallHandler> = Arc::new(move |_, _, _, _| {
            observed.fetch_add(1, Ordering::SeqCst);
            Ok(InvocationResponse::Task(canonical_snapshot(
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
        let call: Arc<CanonicalToolCallHandler> = Arc::new(move |_, _, _, _| {
            Ok(InvocationResponse::Task(canonical_snapshot(
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
        let call: Arc<CanonicalToolCallHandler> = Arc::new(move |_, _, _, _| {
            Ok(InvocationResponse::Task(canonical_snapshot(
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

        let task_id = TaskId::new();
        let expected = canonical_result("same canonical result");
        let direct_expected = expected.clone();
        let call: Arc<CanonicalToolCallHandler> = Arc::new(move |_, arguments, _, _| {
            if arguments.get("async").and_then(Value::as_bool) == Some(true) {
                Ok(InvocationResponse::Task(canonical_snapshot(
                    task_id,
                    InvocationStatus::Working,
                    None,
                )))
            } else {
                Ok(InvocationResponse::Direct(direct_expected.clone()))
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

    async fn tasks_projection_rejects_reverse_timestamps_on_wire_case() {
        use crate::domain::invocation::{InvocationStatus, TaskId};

        let task_id = TaskId::new();
        let mut reversed = canonical_snapshot(task_id, InvocationStatus::Working, None);
        reversed.updated_at_epoch_ms = reversed.created_at_epoch_ms - 1;
        let call_snapshot = reversed.clone();
        let call: Arc<CanonicalToolCallHandler> =
            Arc::new(move |_, _, _, _| Ok(InvocationResponse::Task(call_snapshot.clone())));
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
        let call: Arc<CanonicalToolCallHandler> = Arc::new(move |_, arguments, _, _| {
            observed.fetch_add(1, Ordering::SeqCst);
            match arguments.get("mode").and_then(Value::as_str) {
                Some("near-task") => Ok(InvocationResponse::Task(canonical_snapshot(
                    near_task,
                    InvocationStatus::Working,
                    None,
                ))),
                Some("over-direct") => Ok(InvocationResponse::Direct(call_over.clone())),
                Some("over-task") => Ok(InvocationResponse::Task(canonical_snapshot(
                    over_task,
                    InvocationStatus::Working,
                    None,
                ))),
                _ => Ok(InvocationResponse::Direct(call_near.clone())),
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
        use crate::infrastructure::daemon::client::DaemonTaskExchangeError;
        use crate::infrastructure::daemon::protocol::DaemonErrorCode;
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
                Err(DaemonTaskExchangeError::Protocol(
                    DaemonErrorCode::TaskNotFound,
                ))
            } else if task_id == expired {
                Err(DaemonTaskExchangeError::Protocol(
                    DaemonErrorCode::TaskExpired,
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
        let call: Arc<CanonicalToolCallHandler> = Arc::new(move |_, _, _, _| {
            Ok(InvocationResponse::Task(canonical_snapshot(
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
        let (mut client, _) = spawn_server(application_handler());
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
        let (client_io, server_io) = tokio::io::duplex(4 * 1024 * 1024);
        let server = UnicaServer::legacy_for_test(application_handler());
        let handle = tokio::spawn(async move {
            server
                .serve(server_io)
                .await
                .map(|_| ())
                .map_err(|error| error.to_string())
        });
        let (read_half, mut writer) = tokio::io::split(client_io);
        let mut reader = BufReader::new(read_half).lines();
        let mut line = json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "tools/list",
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
            "admission must close without serving, got {next:?}"
        );
        let outcome = handle.await.unwrap();
        let error = outcome.expect_err("admission failure surfaces as a serve error");
        assert!(
            error.to_lowercase().contains("initialize"),
            "unexpected admission error: {error}"
        );
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

    #[tokio::test]
    async fn tool_results_are_structured() {
        let root = std::env::temp_dir().join(format!(
            "unica-meta-structured-mcp-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let source = root.join("src");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(
            root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        for (relative, bytes) in [
            (
                "Configuration.xml",
                include_bytes!(
                    "../../../../tests/fixtures/unica_mcp_script_parity/meta-validate-language-aware/Configuration.xml"
                )
                .as_slice(),
            ),
            (
                "Languages/Русский.xml",
                include_bytes!(
                    "../../../../tests/fixtures/unica_mcp_script_parity/meta-validate-language-aware/Languages/Русский.xml"
                )
                .as_slice(),
            ),
            (
                "Languages/English.xml",
                include_bytes!(
                    "../../../../tests/fixtures/unica_mcp_script_parity/meta-validate-language-aware/Languages/English.xml"
                )
                .as_slice(),
            ),
            (
                "Enums/LanguageAware.xml",
                include_bytes!(
                    "../../../../tests/fixtures/unica_mcp_script_parity/meta-validate-language-aware/Enums/LanguageAware.xml"
                )
                .as_slice(),
            ),
        ] {
            let path = source.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }

        let cwd = crate::test_support::ProcessCwdGuard::enter(&root).unwrap();
        let (mut client, _) = spawn_server(application_handler());
        client.initialize().await;
        client
            .send(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {} }))
            .await;
        let listed = client.receive().await;
        let tools = listed["result"]["tools"].as_array().unwrap();
        let meta_schemas = tools
            .iter()
            .filter(|tool| {
                tool["name"]
                    .as_str()
                    .is_some_and(|name| name.starts_with("unica.meta."))
            })
            .map(|tool| {
                tool.get("outputSchema")
                    .expect("every Meta tool must publish outputSchema")
            })
            .collect::<Vec<_>>();
        assert_eq!(meta_schemas.len(), 4);
        assert!(meta_schemas.windows(2).all(|pair| pair[0] == pair[1]));
        let output_schema = meta_schemas[0];
        assert_eq!(output_schema["type"], "object");
        assert_eq!(output_schema["additionalProperties"], false);
        assert_eq!(
            output_schema["required"],
            json!([
                "ok",
                "summary",
                "changes",
                "warnings",
                "errors",
                "artifacts",
                "cache"
            ])
        );
        for open_subtree in ["data", "diagnostics", "job"] {
            assert_eq!(output_schema["properties"][open_subtree], json!({}));
        }
        let non_meta = tools
            .iter()
            .find(|tool| tool["name"] == "unica.project.status")
            .unwrap();
        assert!(non_meta.get("outputSchema").is_none());

        client
            .send(json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": {
                    "name": "unica.meta.add",
                    "arguments": {
                        "sourceSet": "main",
                        "kind": "Catalog",
                        "name": "Items"
                    }
                }
            }))
            .await;
        let success = client.receive().await;
        assert!(success.get("error").is_none(), "{success}");
        let success_result = &success["result"];
        let success_text: Value =
            serde_json::from_str(success_result["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(success_result["structuredContent"], success_text);
        assert_eq!(success_result["structuredContent"]["ok"], true, "{success}");
        assert_eq!(success_result["isError"], false);

        client
            .send(json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": {
                    "name": "unica.meta.info",
                    "arguments": {}
                }
            }))
            .await;
        let invalid = client.receive().await;
        assert!(invalid.get("error").is_none(), "{invalid}");
        let invalid_result = &invalid["result"];
        let invalid_text: Value =
            serde_json::from_str(invalid_result["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(invalid_result["structuredContent"], invalid_text);
        assert_eq!(invalid_result["structuredContent"]["ok"], false);
        assert_eq!(
            invalid_result["structuredContent"]["diagnostics"][0]["code"],
            "invalid_arguments"
        );
        assert_eq!(invalid_result["isError"], true);

        client.shutdown().await;
        drop(cwd);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn subsystem_provider_failures_are_normal_typed_tool_results() {
        for (case, descriptor) in [("missing", None), ("malformed", Some("<broken"))] {
            let root = tempfile::Builder::new()
                .prefix(&format!("unica-subsystem-mcp-{case}"))
                .tempdir()
                .unwrap();
            let workspace = root.path().join("workspace");
            let source = workspace.join("src");
            std::fs::create_dir_all(source.join("Subsystems")).unwrap();
            std::fs::write(
                workspace.join("v8project.yaml"),
                "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
            )
            .unwrap();
            std::fs::write(
                source.join("Configuration.xml"),
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Test</Name></Properties><ChildObjects><Subsystem>Sales</Subsystem></ChildObjects></Configuration></MetaDataObject>"#,
            )
            .unwrap();
            if let Some(descriptor) = descriptor {
                std::fs::write(source.join("Subsystems/Sales.xml"), descriptor).unwrap();
            }

            let (mut client, _) = spawn_server(application_handler());
            client.initialize().await;
            client
                .send(json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "tools/call",
                    "params": {
                        "name": "unica.subsystem.info",
                        "arguments": {
                            "cwd": workspace.canonicalize().unwrap(),
                            "SubsystemPath": "src/Subsystems/Sales.xml"
                        }
                    }
                }))
                .await;
            let response = client.receive().await;

            assert!(response.get("error").is_none(), "{case}: {response}");
            assert_eq!(response["result"]["isError"], false, "{case}: {response}");
            let result: Value = serde_json::from_str(
                response["result"]["content"][0]["text"]
                    .as_str()
                    .expect("text tool result"),
            )
            .unwrap();
            assert_eq!(result["ok"], false, "{case}: {result}");
            assert!(result.get("data").is_none(), "{case}: {result}");
            assert!(result.get("tree").is_none(), "{case}: {result}");
            assert!(
                result["diagnostics"]
                    .as_array()
                    .is_some_and(|diagnostics| diagnostics
                        .iter()
                        .any(|diagnostic| { diagnostic["code"] == "provider_unavailable" })),
                "{case}: {result}"
            );
            client.shutdown().await;
        }
    }

    #[test]
    fn tool_definitions_contain_orchestrated_tool_names() {
        let listed = tool_definitions(&crate::application::tools());
        assert_eq!(listed[0].name, "unica.cf.edit");
        for name in [
            "unica.project.status",
            "unica.project.map",
            "unica.standards.explain",
            "unica.runtime.job.start",
            "unica.runtime.job.status",
            "unica.runtime.job.wait",
            "unica.runtime.job.logs",
            "unica.runtime.job.cancel",
            "unica.runtime.job.list",
        ] {
            assert!(
                listed.iter().any(|tool| tool.name == name),
                "missing {name}"
            );
        }
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
    fn role_edit_alone_adds_closed_native_structured_output_schema() {
        let listed = tool_definitions(&crate::application::tools());
        let role_edit = listed
            .iter()
            .find(|tool| tool.name == "unica.role.edit")
            .expect("role.edit must be listed");
        let output = role_edit
            .output_schema
            .as_ref()
            .expect("role.edit must publish outputSchema");
        assert_eq!(output["properties"]["data"]["additionalProperties"], false);
        assert_eq!(
            output["properties"]["cache"]["properties"]["root"],
            json!({"const": ""})
        );
        assert!(output["required"]
            .as_array()
            .unwrap()
            .contains(&json!("data")));
        for forbidden in ["stdout", "stderr", "command", "diagnostics", "job"] {
            assert!(
                output["properties"].get(forbidden).is_none(),
                "role.edit must not publish legacy `{forbidden}` output"
            );
        }
        assert_eq!(
            output["properties"]["data"]["required"],
            json!([
                "metadataPath",
                "changed",
                "effects",
                "validation",
                "diagnostics"
            ])
        );
        let role_info = listed
            .iter()
            .find(|tool| tool.name == "unica.role.info")
            .unwrap();
        assert!(role_info.output_schema.is_none());
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
    fn native_reader_schema_is_typed_and_has_no_invocation_switch() {
        let listed = tool_definitions(&crate::application::tools());
        let cf_info = listed
            .iter()
            .find(|tool| tool.name == "unica.cf.info")
            .expect("unica.cf.info must be listed");

        let schema = cf_info.input_schema.as_ref();
        assert_eq!(schema["additionalProperties"], false);
        assert!(schema["properties"].get("ConfigPath").is_some());
        assert!(schema["properties"].get("cwd").is_some());
        assert!(schema["properties"].get("dryRun").is_none());
        assert!(schema["properties"].get("args").is_none());

        let form_edit = listed
            .iter()
            .find(|tool| tool.name == "unica.form.edit")
            .expect("unica.form.edit must be listed");
        assert!(form_edit.input_schema["properties"].get("dryRun").is_some());
    }

    #[tokio::test]
    async fn role_validate_schema_publishes_canonical_required_path_without_composition() {
        let (mut client, _) = spawn_server(application_handler());
        client.initialize().await;
        client
            .send(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {} }))
            .await;
        let response = client.receive().await;
        let role_validate = response["result"]["tools"]
            .as_array()
            .expect("tools/list must return an array")
            .iter()
            .find(|tool| tool["name"] == "unica.role.validate")
            .expect("unica.role.validate must be listed");

        let schema = &role_validate["inputSchema"];
        // ADR-0049 moved the requirement into the two selector branches: the
        // path is still required to reach the tool by path, and the logical
        // branch requires the address pair instead.
        assert_eq!(schema["required"], json!([]));
        assert_eq!(
            schema["oneOf"],
            json!([
                {
                    "required": ["sourceSet", "metadataPath"],
                    "not": {"required": ["RightsPath"]}
                },
                {
                    "required": ["RightsPath"],
                    "not": {"anyOf": [
                        {"required": ["sourceSet"]},
                        {"required": ["metadataPath"]}
                    ]}
                }
            ]),
            "{schema}"
        );
        assert!(schema.get("allOf").is_none());
        assert!(schema["properties"].get("RightsPath").is_some());
        assert!(schema["properties"].get("Detailed").is_some());
        assert!(schema["properties"].get("MaxErrors").is_some());
        for alias in ["rightsPath", "Path", "path"] {
            assert!(
                schema["properties"].get(alias).is_none(),
                "{alias} is a runtime compatibility alias, not a published argument"
            );
        }
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

    #[test]
    fn source_navigation_mcp_results_are_bounded_and_hide_provider_state() {
        let root = std::env::temp_dir().join(format!(
            "unica-source-navigation-mcp-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let source = root.join("src");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(
            root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(
            root.join(".v8-project.json"),
            r#"{"editingAllowedCheck":"off"}"#,
        )
        .unwrap();
        std::fs::write(
            source.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Main</Name></Properties></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        for name in ["Alpha", "Alpine", "Algebra"] {
            let directory = source.join("CommonModules").join(name);
            std::fs::create_dir_all(directory.join("Ext")).unwrap();
            std::fs::write(
                source.join("CommonModules").join(format!("{name}.xml")),
                format!(
                    r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><CommonModule><Properties><Name>{name}</Name></Properties></CommonModule></MetaDataObject>"#
                ),
            )
            .unwrap();
            std::fs::write(
                directory.join("Ext/Module.bsl"),
                "Procedure Run()\nEndProcedure\n",
            )
            .unwrap();
        }
        std::fs::create_dir_all(source.join("Catalogs")).unwrap();
        std::fs::write(
            source.join("Catalogs/Items.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog><Properties><Name>Items</Name></Properties></Catalog></MetaDataObject>"#,
        )
        .unwrap();

        let resolve_args = json!({
            "cwd": root,
            "sourceSet": "main",
            "query": "CommonModule.Al",
            "mode": "prefix",
            "limit": 2
        })
        .as_object()
        .unwrap()
        .clone();
        let resolve: Value = serde_json::from_str(
            &call_tool_text(
                &UnicaApplication::new(),
                "unica.source.resolve",
                &resolve_args,
                CancellationToken::new(),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(resolve["data"]["candidates"].as_array().unwrap().len(), 2);
        assert_eq!(resolve["data"]["completeness"], "partial");
        assert!(resolve["data"]["nextCursor"].is_string());
        assert_no_private_source_navigation_keys(&resolve);

        let children_args = json!({
            "cwd": root,
            "sourceSet": "main",
            "limit": 1
        })
        .as_object()
        .unwrap()
        .clone();
        let children: Value = serde_json::from_str(
            &call_tool_text(
                &UnicaApplication::new(),
                "unica.source.children",
                &children_args,
                CancellationToken::new(),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(children["data"]["children"].as_array().unwrap().len(), 1);
        assert!(children["data"]["nextCursor"].is_string());
        assert_no_private_source_navigation_keys(&children);

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn source_resource_mcp_round_trip_reuses_one_snapshot_and_hides_private_state() {
        let root = std::env::temp_dir().join(format!(
            "unica-source-resource-mcp-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let source = root.join("src");
        std::fs::create_dir_all(source.join("CommonModules/Shared/Ext")).unwrap();
        std::fs::write(
            root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(
            source.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Main</Name></Properties><ChildObjects><CommonModule>Shared</CommonModule></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(
            source.join("CommonModules/Shared.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><CommonModule><Properties><Name>Shared</Name></Properties></CommonModule></MetaDataObject>"#,
        )
        .unwrap();
        let module = source.join("CommonModules/Shared/Ext/Module.bsl");
        let bytes = b"\xef\xbb\xbfProcedure Run()\r\nEndProcedure\r\n";
        std::fs::write(&module, bytes).unwrap();

        let app = UnicaApplication::new();
        let resources: Value = serde_json::from_str(
            &call_tool_text(
                &app,
                "unica.source.resources",
                json!({
                    "cwd": root,
                    "sourceSet": "main",
                    "metadataPath": "CommonModule.Shared.Module",
                    "scope": "self"
                })
                .as_object()
                .unwrap(),
                CancellationToken::new(),
            )
            .unwrap(),
        )
        .unwrap();
        let snapshot = resources["data"]["snapshotId"]
            .as_str()
            .unwrap()
            .to_string();
        let resource = &resources["data"]["resources"][0];
        // The surface is read-only, so nothing may advertise a write.
        assert_eq!(resource["access"], json!(["read"]));
        assert_no_private_source_resource_keys(&resources);

        let read: Value = serde_json::from_str(
            &call_tool_text(
                &app,
                "unica.source.read",
                json!({
                    "cwd": root,
                    "snapshotId": snapshot,
                    "resourceId": resource["resourceId"].as_str().unwrap()
                })
                .as_object()
                .unwrap(),
                CancellationToken::new(),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(read["data"]["eof"], json!(true));
        assert_eq!(read["data"]["contentEncoding"], "utf-8");
        assert_no_private_source_resource_keys(&read);
        assert_eq!(
            std::fs::read(&module).unwrap(),
            bytes,
            "a read-only flow must not touch the module"
        );

        std::fs::remove_dir_all(root).unwrap();
    }

    fn assert_no_private_source_resource_keys(value: &Value) {
        match value {
            Value::Object(object) => {
                for key in object.keys() {
                    assert!(
                        !matches!(
                            key.as_str(),
                            "path"
                                | "sourceDir"
                                | "provider"
                                | "providerId"
                                | "providerRevision"
                                | "handle"
                                | "private"
                                | "workspaceRoot"
                        ),
                        "private source-resource key leaked: {key}"
                    );
                }
                for child in object.values() {
                    assert_no_private_source_resource_keys(child);
                }
            }
            Value::Array(items) => {
                for item in items {
                    assert_no_private_source_resource_keys(item);
                }
            }
            _ => {}
        }
    }

    fn assert_no_private_source_navigation_keys(value: &Value) {
        match value {
            Value::Object(object) => {
                for key in object.keys() {
                    assert!(
                        !matches!(
                            key.as_str(),
                            "path"
                                | "sourceDir"
                                | "provider"
                                | "providerId"
                                | "providerRevision"
                                | "handle"
                                | "private"
                        ),
                        "private source-navigation key leaked: {key}"
                    );
                }
                for child in object.values() {
                    assert_no_private_source_navigation_keys(child);
                }
            }
            Value::Array(items) => {
                for item in items {
                    assert_no_private_source_navigation_keys(item);
                }
            }
            _ => {}
        }
    }

    #[tokio::test]
    async fn tool_execution_failure_keeps_json_rpc_error_shape() {
        let (mut client, _) = spawn_server(application_handler());
        client.initialize().await;
        client
            .send(json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": { "name": "unica.no.such.tool", "arguments": {} }
            }))
            .await;
        let response = client.receive().await;
        assert_eq!(response["error"]["code"], TOOL_EXECUTION_ERROR);
        assert!(response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("unknown unica tool"));

        client
            .send(json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": {
                    "name": "unica.project.status",
                    "arguments": {"cwd": "/missing/workspace", "dryRun": true}
                }
            }))
            .await;
        let response = client.receive().await;
        assert_eq!(response["error"]["code"], TOOL_EXECUTION_ERROR);
        assert!(response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("does not accept argument `dryRun`"));
        client.shutdown().await;

        let handler: Arc<ToolCallHandler> = Arc::new(|_, _, _, _| {
            Err((
                TOOL_EXECUTION_ERROR,
                "typed_result_missing: unica.project.status returned ok without OperationResult.data"
                    .to_string(),
            ))
        });
        let (mut client, _) = spawn_server(handler);
        client.initialize().await;
        client
            .send(json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": {"name": "unica.project.status", "arguments": {}}
            }))
            .await;
        let response = client.receive().await;
        assert_eq!(response["error"]["code"], TOOL_EXECUTION_ERROR);
        assert!(response["error"]["message"]
            .as_str()
            .unwrap()
            .starts_with("typed_result_missing:"));
        client.shutdown().await;
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
    fn admission_is_bounded_and_reusable() {
        let registry = Arc::new(InFlightRegistry::default());
        let mut guards = Vec::new();
        for _ in 0..MCP_MAX_TOOL_WORKERS {
            guards.push(registry.admit().unwrap());
        }
        let overloaded = registry.admit().unwrap_err();
        assert!(overloaded.contains("overloaded"));
        guards.pop();
        guards.push(registry.admit().unwrap());
        drop(guards);
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
    async fn overloaded_dispatcher_returns_deterministic_json_rpc_error() {
        let release = Arc::new(AtomicBool::new(false));
        let gate = Arc::clone(&release);
        let handler: Arc<ToolCallHandler> = Arc::new(move |_, _, _, _| {
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
        for id in 0..MCP_MAX_TOOL_WORKERS {
            client
                .send(json!({
                    "jsonrpc": "2.0",
                    "id": format!("blocked-{id}"),
                    "method": "tools/call",
                    "params": { "name": "unica.code.search", "arguments": {} }
                }))
                .await;
        }
        let admitted_deadline = Instant::now() + TEST_STEP;
        while in_flight.running() < MCP_MAX_TOOL_WORKERS {
            assert!(
                Instant::now() < admitted_deadline,
                "workers were not admitted"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        client
            .send(json!({
                "jsonrpc": "2.0",
                "id": "overload",
                "method": "tools/call",
                "params": { "name": "unica.code.search", "arguments": {} }
            }))
            .await;
        let response = client.receive().await;
        assert_eq!(response["id"], "overload");
        assert_eq!(response["error"]["code"], ErrorCode::INTERNAL_ERROR.0);
        assert!(response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("overloaded"));
        release.store(true, Ordering::SeqCst);
        for _ in 0..MCP_MAX_TOOL_WORKERS {
            let response = client.receive().await;
            let payload: Value =
                serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap())
                    .unwrap();
            assert_eq!(payload["summary"], "released");
        }
        client.shutdown().await;
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
