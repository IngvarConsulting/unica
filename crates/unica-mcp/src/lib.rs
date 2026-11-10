//! Public `unica` stdio MCP server on the official Rust SDK (`rmcp`).
//!
//! ADR-0013: the SDK owns the JSON-RPC loop and handshake after the optional
//! initial `server/discover` probe, protocol version negotiation, per-request
//! task spawning, `ping`, and `notifications/cancelled` bookkeeping. This
//! module maps SDK requests onto the transport-neutral application layer
//! (ADR-0002) and keeps the tool contract data-driven from operation
//! descriptors (ADR-0001) instead of SDK macros.

mod canonical_cancellation;
mod catalog;
mod daemon_router;
mod human_choice;
mod in_flight;
mod surface;
mod task_projection;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
mod tool_result;
mod transport;

use crate::canonical_cancellation::CanonicalCancellation;
use crate::daemon_router::{
    canonical_daemon_router, CanonicalCallOutcome, CanonicalDaemonRouter,
    FrontendInvocationDeadline, TOOL_EXECUTION_ERROR,
};
mod manual_calls;
mod receive_pump;
mod server;
use crate::application::tool_contracts::{SurfaceRelease, V13TaskProfile};
use crate::application::{
    code_search_output_schema, input_schema_for_tool, metadata_argument_failure_result,
    operation_result_output_schema, role_edit_argument_failure_result, role_edit_output_schema,
    strip_schema_descriptions, CodeIntelligenceOperation, OperationResult, ToolHandler, ToolSpec,
    UnicaApplication,
};
#[cfg(any(test, feature = "test-support"))]
use crate::daemon_router::{CanonicalCallHandler, CanonicalTaskHandler, CanonicalTaskWaitHandler};
use catalog::tool_definitions;
use in_flight::{drain_stdio_frontend, InFlightGuard, InFlightRegistry};
use manual_calls::ManualCalls;
use rmcp::model::{
    CacheScope, CallToolRequestParams, CallToolResponse, CallToolResult, CancelTaskParams,
    ClientJsonRpcMessage, ClientNotification, ClientRequest, ContentBlock, DiscoverResult,
    ErrorCode, ErrorData, GetMeta, GetTaskParams, GetTaskResult, Implementation,
    InitializeRequestParams, InitializeResult, ListPromptsResult, ListResourceTemplatesResult,
    ListResourcesResult, ListToolsResult, NotificationMetaObject, PaginatedRequestParams,
    ProgressNotificationParam, ProgressToken, ProtocolVersion, RequestMetaObject,
    ServerCapabilities, ServerConfig, ServerJsonRpcMessage, ServerResult, Tool, UpdateTaskParams,
    TASKS_EXTENSION_ID,
};
use rmcp::service::{NotificationContext, RequestContext, ServerInitializeError};
use rmcp::transport::Transport;
use rmcp::{RoleServer, ServerHandler, ServiceExt};
use serde_json::{Map, Value};
use server::UnicaServer;
use std::collections::HashSet;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use surface::SurfaceToolRouter;
#[cfg(any(test, feature = "test-support"))]
use surface::ToolCallHandler;
#[cfg(feature = "test-support")]
use tool_result::call_tool_result_observed;
use transport::{DiscoveryProbeTransport, ObservedServer};
use unica_application::application;
use unica_execution::receipt_ledger::V5ToolIdentity;
use unica_refusal::RefusalDetail;
use unica_work_control::cancellation::CancellationToken;
use unica_work_control::progress::{NoopProgressSink, ProgressEvent, ProgressSink};

#[cfg(any(test, feature = "test-support"))]
use unica_daemon_protocol::client_v5::V5DaemonProcessOwner;
use unica_daemon_protocol::client_v5::{
    TaskObservation, TaskObservationStage, TaskObservationTimer, V5DaemonClient,
    V5TaskExchangeError,
};
use unica_daemon_protocol::protocol_v5::V5DaemonErrorCode;
use unica_execution::receipt_runtime::response::V5DaemonTaskSnapshot;

// resource-limit: mcp.runtime-shutdown-grace
// class: deadline
// unit: ms
// scope: process
// value: 250
// resource: Срок остановки асинхронного рантайма MCP-frontend.
// status: unproven
// origin: #224
// problem: неизвестно
// on-reach: незавершённые задачи рантайма бросаются при выходе
const RUNTIME_SHUTDOWN_GRACE: Duration = Duration::from_millis(250);

enum SurfaceToolOutcome {
    Legacy(Box<OperationResult>),
    /// A compatibility Task tool answered with a canonical result to project.
    Canonical(unica_execution_contract::invocation::DomainResult),
    CompatibilityTask {
        result: CompatibilityTaskResult,
        timing: TaskObservationTimer,
    },
    /// An acknowledged Direct terminal, already the final `CallToolResult`.
    Direct(CallToolResult),
    /// An acknowledged Direct refusal that offers a person a choice: the
    /// door asks it when the client can ask, else answers the refusal.
    Choice {
        refusal: CallToolResult,
        choice: Box<unica_execution_contract::invocation::HumanChoice>,
    },
    Task(V5DaemonTaskSnapshot),
}

pub fn run_stdio(connect: fn() -> Result<V5DaemonClient, String>) {
    if SurfaceRelease::from_package_version() != SurfaceRelease::V13 {
        eprintln!("this package does not select the canonical v0.13 MCP surface");
        return;
    }
    let client = match connect() {
        Ok(client) => client,
        Err(error) => {
            eprintln!("{error}");
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

/// Переменная, в которой загрузчик передаёт рассказ о прошлом запуске.
///
/// Убитая установка своего провода не имела: он появляется только здесь, и
/// рассказать о ней может лишь тот, кого запустили следом.
const STARTUP_NOTICE_ENV: &str = "UNICA_STARTUP_NOTICE";

const CANONICAL_INSTRUCTIONS: &str = "Start with unica.view using an empty object when the workspace or logical address is unknown. Use returned addresses instead of guessing at. A qualified logical address has the form <sourceSet>:<Kind>[.<Name>...]. Use unica.check to confirm source-set admission or logical-node readability.";

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
    #[cfg(any(test, feature = "test-support"))]
    pub fn legacy_for_test(handler: Arc<ToolCallHandler>) -> Self {
        let notice = startup_notice_from(std::env::var(STARTUP_NOTICE_ENV).ok());
        Self::legacy_with_startup_notice_for_test(handler, notice)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn legacy_with_startup_notice_for_test(
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
            listed_tools_for_test: None,
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_canonical_v13(handler: Arc<CanonicalCallHandler>) -> Self {
        let unavailable: Arc<CanonicalTaskHandler> = Arc::new(|_, _| {
            Err(V5TaskExchangeError::Transport(
                TaskObservationTimer::start(Instant::now()).failed(
                    TaskObservationStage::AcquirePeer,
                    Instant::now(),
                    None,
                ),
            ))
        });
        Self::with_canonical_v13_tasks(handler, Arc::clone(&unavailable), unavailable)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_canonical_v13_and_startup_notice_for_test(
        handler: Arc<CanonicalCallHandler>,
        startup_notice: Option<String>,
    ) -> Self {
        Self {
            startup_notice,
            ..Self::with_canonical_v13(handler)
        }
    }

    /// Канонический маршрут с произвольным каталогом `tools/list`.
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_canonical_v13_tool_catalog_for_test(
        handler: Arc<CanonicalCallHandler>,
        tools: Vec<Tool>,
    ) -> Self {
        Self {
            listed_tools_for_test: Some(Arc::new(tools)),
            ..Self::with_canonical_v13(handler)
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_canonical_v13_tasks(
        call: Arc<CanonicalCallHandler>,
        get: Arc<CanonicalTaskHandler>,
        cancel: Arc<CanonicalTaskHandler>,
    ) -> Self {
        let wait_get = Arc::clone(&get);
        let wait: Arc<CanonicalTaskWaitHandler> =
            Arc::new(move |task_id, _, deadline, _| wait_get(task_id, deadline));
        Self::with_canonical_v13_task_handlers(call, get, wait, cancel)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_canonical_v13_task_handlers(
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
            listed_tools_for_test: None,
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
            #[cfg(any(test, feature = "test-support"))]
            listed_tools_for_test: None,
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_canonical_daemon(owner: V5DaemonProcessOwner, workspace_hint: String) -> Self {
        Self::canonical_v13_daemon(owner.into(), workspace_hint.into(), None)
    }

    fn in_flight(&self) -> Arc<InFlightRegistry> {
        Arc::clone(&self.in_flight)
    }
}

/// Сборка сервера двери для тестов демона: маршрутизатор к живому демону
/// и реестр вызовов, за которым тест следит.
#[cfg(any(test, feature = "test-support"))]
impl UnicaServer {
    /// Run the calls of a chosen answer in order. A refused call stops the
    /// chain; a call that continues as a Task ends it, and its Task is the
    /// answer. A call refused with a choice of its own is not asked again:
    /// one call asks the person once.
    async fn run_chosen(
        &self,
        option: &unica_execution_contract::invocation::HumanChoiceOption,
        host: &unica_bootstrap::HostRequest,
        progress_token: Option<ProgressToken>,
        context: &RequestContext<RoleServer>,
        response_budget: unica_bootstrap::ResponseBudget,
    ) -> Result<(CallToolResponse, Vec<human_choice::StepReport>), ErrorData> {
        let mut steps = Vec::new();
        let mut last = None;
        for (tool, arguments) in human_choice::calls(option) {
            let started = Instant::now();
            let outcome = invoke_surface_tool(
                self,
                SurfaceInvocation {
                    name: tool.to_string(),
                    arguments: arguments.clone(),
                    host: host.clone(),
                    progress_token: progress_token.clone(),
                    received_at: started,
                },
                context,
            )
            .await;
            let outcome = match outcome {
                Ok(SurfaceToolOutcome::Choice { refusal, .. }) => {
                    Ok(SurfaceToolOutcome::Direct(refusal))
                }
                other => other,
            };
            match project_surface_outcome(self, tool, outcome, response_budget, started)? {
                CallToolResponse::Complete(result) => {
                    let step = human_choice::StepReport::of(tool, arguments, &result);
                    let refused = step.refused;
                    steps.push(step);
                    last = Some(result);
                    if refused {
                        break;
                    }
                }
                other => return Ok((other, steps)),
            }
        }
        let result = last.expect("a chosen answer runs at least its own call");
        Ok((CallToolResponse::from(result), steps))
    }

    pub fn with_canonical_v13_router(router: CanonicalDaemonRouter) -> Self {
        Self {
            router: SurfaceToolRouter::CanonicalV13(router),
            in_flight: Arc::new(InFlightRegistry::default()),
            manual_calls: Arc::new(ManualCalls::default()),
            structured_tools: HashSet::new(),
            startup_notice: None,
            listed_tools_for_test: None,
        }
    }

    pub fn in_flight_registry(&self) -> Arc<InFlightRegistry> {
        self.in_flight()
    }

    pub fn manual_calls_for_test(&self) -> Arc<ManualCalls> {
        Arc::clone(&self.manual_calls)
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
                let cutoff = request.as_ref().ok().map(|_| deadline.transport_cutoff());
                let result =
                    execute_compatibility_task_tool(router, request, deadline, cancellation);
                return Ok(match cutoff {
                    Some(cutoff) => SurfaceToolOutcome::CompatibilityTask {
                        result,
                        timing: TaskObservationTimer::since(deadline.received_at(), cutoff),
                    },
                    None => SurfaceToolOutcome::Canonical(result.into_result()),
                });
            }
            let tool = V5ToolIdentity::from_wire_name(name).ok_or_else(|| {
                ErrorData::invalid_params("tool is not in the canonical v0.13 profile", None)
            })?;
            match (router.call)(tool, arguments, host, deadline, cancellation)? {
                CanonicalCallOutcome::Direct(result) => Ok(SurfaceToolOutcome::Direct(result)),
                CanonicalCallOutcome::Choice { refusal, choice } => {
                    Ok(SurfaceToolOutcome::Choice { refusal, choice })
                }
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

enum CompatibilityTaskResult {
    Snapshot(unica_execution_contract::invocation::DomainResult),
    Refusal(unica_execution_contract::invocation::DomainResult),
}
impl CompatibilityTaskResult {
    fn result(&self) -> &unica_execution_contract::invocation::DomainResult {
        match self {
            Self::Snapshot(result) | Self::Refusal(result) => result,
        }
    }
    fn into_result(self) -> unica_execution_contract::invocation::DomainResult {
        match self {
            Self::Snapshot(result) | Self::Refusal(result) => result,
        }
    }
}

fn execute_compatibility_task_tool(
    router: &CanonicalDaemonRouter,
    request: Result<crate::application::v13::task_tools::TaskToolRequest, TaskToolError>,
    deadline: FrontendInvocationDeadline,
    cancellation: CanonicalCancellation,
) -> CompatibilityTaskResult {
    let request = match request {
        Ok(request) => request,
        Err(error) => {
            return CompatibilityTaskResult::Refusal(
                crate::application::v13::task_tools::task_tool_error_result(error),
            )
        }
    };
    let exchange = match request.action {
        TaskToolAction::Get => (router.get)(request.task_id, deadline),
        TaskToolAction::Result { wait_ms } => {
            (router.wait)(request.task_id, wait_ms, deadline, cancellation)
        }
        TaskToolAction::Cancel => (router.cancel)(request.task_id, deadline),
    };
    let snapshot = match exchange {
        Ok(snapshot) if snapshot.task_id() == request.task_id => snapshot,
        Ok(_) => {
            return CompatibilityTaskResult::Refusal(
                crate::application::v13::task_tools::task_tool_error_result(
                    TaskToolError::TaskProtocolFailed,
                ),
            )
        }
        Err(error) => {
            if let V5TaskExchangeError::Transport(observation) = error {
                log_task_observation(&observation, deadline.received_at(), Instant::now());
            }
            return CompatibilityTaskResult::Refusal(
                crate::application::v13::task_tools::task_tool_error_result(
                    compatibility_task_exchange_error(error),
                ),
            );
        }
    };
    let projection = match request.action {
        TaskToolAction::Result { .. } => CompatibilityProjection::TerminalResult,
        TaskToolAction::Get | TaskToolAction::Cancel => CompatibilityProjection::State,
    };
    CompatibilityTaskResult::Snapshot(project_compatibility_snapshot(&snapshot, projection))
}

/// The host-facing CallToolResult is built before the last cutoff check.
/// The SDK owns subsequent JSON-RPC framing and writes; this checkpoint makes
/// no claim about delivery of the last stdout byte under host backpressure.
fn compatibility_result_before(
    result: &CompatibilityTaskResult,
    budget: unica_bootstrap::ResponseBudget,
    received_at: Instant,
    timing: TaskObservationTimer,
    after_projection: impl FnOnce() -> Instant,
) -> Result<CallToolResult, crate::task_projection::TaskProjectionError> {
    let projected = crate::task_projection::call_tool_result(result.result(), budget);
    // Preserve an already established exchange failure, including the phase
    // and generation that localized it. Only a snapshot can become late here.
    if matches!(result, CompatibilityTaskResult::Refusal(_)) {
        return projected;
    }
    let projected_at = after_projection();
    let observation = timing.failed(TaskObservationStage::ProjectResult, projected_at, None);
    if observation.cutoff_expired {
        log_task_observation(&observation, received_at, projected_at);
        let refusal = crate::application::v13::task_tools::task_tool_error_result(
            TaskToolError::TaskTransportFailed,
        );
        return crate::task_projection::call_tool_result(&refusal, budget);
    }
    projected
}

/// A Task transport observation diagnoses the host for a person; it does not
/// change what the agent does next, which the refusal code already decides.
/// The wire carries only the refusal, and the observation goes to the
/// frontend log as one line with a JSON payload. `sinceReceivedMs` measures
/// from the moment the frontend received the request to the failure, whatever
/// stage started the observation timer.
fn log_task_observation(observation: &TaskObservation, received_at: Instant, failed_at: Instant) {
    let since_received_ms =
        u64::try_from(failed_at.saturating_duration_since(received_at).as_millis())
            .unwrap_or(u64::MAX);
    #[cfg(any(test, feature = "test-support"))]
    LOGGED_TASK_OBSERVATIONS
        .with(|logged| logged.borrow_mut().push((*observation, since_received_ms)));
    eprintln!(
        "unica task observation: {}",
        serde_json::json!({"taskObservation": observation, "sinceReceivedMs": since_received_ms})
    );
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static LOGGED_TASK_OBSERVATIONS: std::cell::RefCell<Vec<(TaskObservation, u64)>> =
        const { std::cell::RefCell::new(Vec::new()) };
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
        V5TaskExchangeError::Transport(_) => TaskToolError::TaskTransportFailed,
        V5TaskExchangeError::SessionPoisoned => TaskToolError::TaskSessionClosed,
        V5TaskExchangeError::UnexpectedResponse | V5TaskExchangeError::MalformedResponse => {
            TaskToolError::TaskProtocolFailed
        }
    }
}

/// The compatibility receipt carries the durable state only: the closed v5
/// snapshot cannot violate the status/result/failure matrix, and a failure
/// reaches the host as presence, never as text.
fn project_compatibility_snapshot(
    snapshot: &V5DaemonTaskSnapshot,
    projection: CompatibilityProjection,
) -> unica_execution_contract::invocation::DomainResult {
    let state = CompatibilityTaskSnapshot::new(
        snapshot.task_id(),
        snapshot.status(),
        snapshot.completed_result().cloned(),
        snapshot.failure_reason(),
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
// resource-limit: mcp.tools-page-entries
// class: step
// unit: count
// scope: request
// value: 25
// resource: Число инструментов на одной странице tools/list в эпохе modern.
// status: unproven
// origin: #545
// problem: неизвестно
// on-reach: остальные инструменты выдаются по nextCursor; legacy-клиенты получают весь список одной страницей
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
    if returns_source_text(name) {
        // `view` отдаёт исходный текст в пределах бюджета клиента, а
        // `task.result` — тот же сохранённый ответ, переданный в задание.
        // Хост, который понимает это объявление, передаёт любой их ответ
        // модели целиком, а не выносит в файл.
        let (key, chars) = unica_bootstrap::source_text_tool_meta();
        let mut meta = rmcp::model::MetaObject::new();
        meta.insert(key.to_string(), Value::from(chars));
        tool.meta = Some(meta);
    }
    tool
}

/// Инструменты, чей ответ может нести исходный текст BSL.
fn returns_source_text(name: &str) -> bool {
    matches!(name, "view" | "task.result")
}

/// Ключ `_meta`, которым клиент без `initialize` (протокол 2026-07-28)
/// называет себя в каждом запросе.
const CLIENT_INFO_META: &str = "io.modelcontextprotocol/clientInfo";

/// Имя клиента, названное SDK, а без него — из `_meta` запроса.
fn client_name(peer: Option<String>, metadata: &Map<String, Value>) -> Option<String> {
    peer.or_else(|| {
        metadata
            .get(CLIENT_INFO_META)?
            .get("name")?
            .as_str()
            .map(str::to_owned)
    })
}

/// Бюджет ответа клиента: по его имени и окружению запуска (фасад хостов).
fn client_response_budget(
    context: &RequestContext<RoleServer>,
    metadata: &Map<String, Value>,
) -> unica_bootstrap::ResponseBudget {
    // SDK знает оба источника: `_meta` запроса и, для сеанса с
    // `initialize`, его `clientInfo`. Свой разбор — запасной путь для
    // `_meta`, который SDK положил только в параметры запроса.
    let named = context.client_info().map(|client| client.name);
    unica_bootstrap::host_response_budget(client_name(named, metadata).as_deref())
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

    fn get_info(&self) -> ServerConfig {
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
        #[cfg(any(test, feature = "test-support"))]
        let all: &[Tool] = self
            .listed_tools_for_test
            .as_deref()
            .map_or(all, Vec::as_slice);
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
        let response_budget = client_response_budget(&context, &metadata);
        let host = unica_bootstrap::HostRequest {
            metadata,
            roots,
            response_budget,
        };
        let progress_token = request
            .meta
            .as_ref()
            .and_then(RequestMetaObject::get_progress_token)
            .or_else(|| context.meta.get_progress_token());
        let name = request.name.to_string();
        // Only the door confirms a call, from a person's answer; a call that
        // brings a confirmation of its own would skip the question.
        if human_choice::carries_confirmation(request.arguments.as_ref()) {
            return Err(ErrorData::invalid_params(
                format!(
                    "`{}` is set only from a person's answer to a question; a call does not carry it",
                    unica_execution_contract::invocation::HUMAN_CONFIRMATION_KEY
                ),
                None,
            ));
        }
        let input_responses = request.input_responses;
        let request_state = request.request_state;
        let outcome = invoke_surface_tool(
            self,
            SurfaceInvocation {
                name: name.clone(),
                arguments: request.arguments.unwrap_or_default(),
                host: host.clone(),
                progress_token: progress_token.clone(),
                received_at,
            },
            &context,
        )
        .await;
        let (refusal, choice) = match outcome {
            Ok(SurfaceToolOutcome::Choice { refusal, choice }) => (refusal, choice),
            outcome => {
                return project_surface_outcome(self, &name, outcome, response_budget, received_at)
            }
        };
        // The refusal offers a choice: ask the person when the client can ask.
        let answer = match human_choice::choice_channel(
            context.client_capabilities().as_ref(),
            modern_peer(&context),
        ) {
            // A confirmation runs for a client that cannot ask, as before the
            // question existed; a choice keeps its refusal.
            human_choice::ChoiceChannel::None if choice.proceeds_unasked => {
                let option = &choice.options[0];
                let (response, _) = self
                    .run_chosen(option, &host, progress_token, &context, response_budget)
                    .await?;
                return Ok(response);
            }
            human_choice::ChoiceChannel::None => None,
            human_choice::ChoiceChannel::Request => elicit_choice(&context, &choice).await,
            human_choice::ChoiceChannel::InputRequired => {
                match human_choice::retried_answer(
                    &choice,
                    input_responses.as_ref(),
                    request_state.as_deref(),
                ) {
                    Some(answer) => Some(answer),
                    None => {
                        return Ok(CallToolResponse::from(human_choice::input_required(
                            &choice,
                        )))
                    }
                }
            }
        };
        let Some(option) = answer
            .as_ref()
            .and_then(|answer| human_choice::chosen(&choice, answer))
        else {
            return Ok(CallToolResponse::from(refusal));
        };
        match self
            .run_chosen(option, &host, progress_token, &context, response_budget)
            .await?
        {
            (CallToolResponse::Complete(result), steps) => Ok(CallToolResponse::from(
                human_choice::name_the_choice(result, &choice, option, &steps),
            )),
            (other, _) => Ok(other),
        }
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
            .map_err(|error| project_task_exchange_error(error, received_at))?;
        ensure_task_identity(task_id, &snapshot)?;
        crate::task_projection::detailed_task_v5(
            &snapshot,
            client_response_budget(&context, &context.meta.0 .0),
        )
        .map(GetTaskResult::new)
        .map_err(crate::task_projection::projection_error)
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
        // A Task never enters input_required: a person is asked only within
        // a synchronous tools/call (`human_choice`). Still prove the task is a current
        // daemon-owned identity before returning the stable unsupported-input
        // classification; unknown and expired identities retain their codes.
        let deadline = FrontendInvocationDeadline::new(received_at, None);
        let snapshot = tokio::task::spawn_blocking(move || handler(task_id, deadline))
            .await
            .map_err(|_| task_internal_error("task_worker_failed"))?
            .map_err(|error| project_task_exchange_error(error, received_at))?;
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
            .map_err(|error| project_task_exchange_error(error, received_at))?;
        ensure_task_identity(task_id, &snapshot)?;
        Ok(())
    }
}

/// Bound on one `roots/list` exchange. A host answers from memory; the
/// remainder of the handoff window belongs to the daemon call.
// resource-limit: mcp.client-roots-timeout-ms
// class: deadline
// unit: ms
// scope: request
// value: 2000
// resource: Срок одного обмена roots/list с клиентом MCP.
// status: unproven
// origin: 64f07293f (#1232)
// problem: Хост отвечает из памяти; остаток окна передачи принадлежит вызову демона (комментарий в mcp.rs).
// on-reach: Ответ roots/list не получен, рабочая папка из roots не берётся.
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
        Ok(Err(
            rmcp::service::ServiceError::TransportClosed
            | rmcp::service::ServiceError::TransportSend(_),
        )) => ClientRoots::Unavailable(unica_bootstrap::RootsFailure::Closed),
        Ok(Err(rmcp::service::ServiceError::Timeout { .. })) => {
            ClientRoots::Unavailable(unica_bootstrap::RootsFailure::Timeout)
        }
        Ok(Err(_)) => ClientRoots::Unavailable(unica_bootstrap::RootsFailure::Error),
        Err(_) => ClientRoots::Unavailable(unica_bootstrap::RootsFailure::Timeout),
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

fn parse_task_id(encoded: &str) -> Result<unica_execution_contract::invocation::TaskId, ErrorData> {
    encoded.parse().map_err(|_| {
        ErrorData::invalid_params(
            "invalid_task_id",
            Some(serde_json::json!({"code": "invalid_task_id"})),
        )
    })
}

fn ensure_task_identity(
    expected: unica_execution_contract::invocation::TaskId,
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

fn project_task_exchange_error(error: V5TaskExchangeError, received_at: Instant) -> ErrorData {
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
        V5TaskExchangeError::Transport(observation) => {
            log_task_observation(&observation, received_at, Instant::now());
            task_internal_error("task_transport_failed")
        }
        V5TaskExchangeError::SessionPoisoned => task_internal_error("task_session_closed"),
        V5TaskExchangeError::UnexpectedResponse | V5TaskExchangeError::MalformedResponse => {
            task_internal_error("task_protocol_failed")
        }
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

/// One tool call the door runs for a `tools/call`: the requested call, or
/// the call a person chose instead of a refusal.
struct SurfaceInvocation {
    name: String,
    arguments: Map<String, Value>,
    host: unica_bootstrap::HostRequest,
    progress_token: Option<ProgressToken>,
    received_at: Instant,
}

async fn invoke_surface_tool(
    server: &UnicaServer,
    invocation: SurfaceInvocation,
    context: &RequestContext<RoleServer>,
) -> Result<SurfaceToolOutcome, ErrorData> {
    let SurfaceInvocation {
        name,
        arguments,
        host,
        progress_token,
        received_at,
    } = invocation;
    let client_supports_tasks = native_task_capability(context);
    let admission = server
        .in_flight
        .admit()
        .map_err(|message| ErrorData::new(ErrorCode::INTERNAL_ERROR, message, None))?;
    let canonical = matches!(server.router, SurfaceToolRouter::CanonicalV13(_));
    let (manual_owner, standalone_admission, cancellation) = if canonical {
        match server.manual_calls.enter(context.id.clone(), admission) {
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

    let router = server.router.clone();
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
                name: &name,
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
    match result {
        Ok(outcome) => outcome,
        Err(join_error) => Err(ErrorData::new(
            ErrorCode::INTERNAL_ERROR,
            format!("tool worker failed: {join_error}"),
            None,
        )),
    }
}

fn project_surface_outcome(
    server: &UnicaServer,
    name: &str,
    outcome: Result<SurfaceToolOutcome, ErrorData>,
    response_budget: unica_bootstrap::ResponseBudget,
    received_at: Instant,
) -> Result<CallToolResponse, ErrorData> {
    match outcome? {
        SurfaceToolOutcome::Legacy(result) => {
            render_tool_result(server.structured_tools.contains(name), *result)
                .map(CallToolResponse::from)
        }
        SurfaceToolOutcome::CompatibilityTask { result, timing } => {
            compatibility_result_before(&result, response_budget, received_at, timing, Instant::now)
                .map(CallToolResponse::from)
                .map_err(crate::task_projection::projection_error)
        }
        SurfaceToolOutcome::Canonical(result) => {
            crate::task_projection::call_tool_result(&result, response_budget)
                .map(CallToolResponse::from)
                .map_err(crate::task_projection::projection_error)
        }
        SurfaceToolOutcome::Direct(result) => Ok(CallToolResponse::from(result)),
        // Nobody is asked here: the refusal the choice came with is the answer.
        SurfaceToolOutcome::Choice { refusal, .. } => Ok(CallToolResponse::from(refusal)),
        SurfaceToolOutcome::Task(snapshot) => {
            crate::task_projection::create_task_result_v5(&snapshot)
                .map(CallToolResponse::from)
                .map_err(crate::task_projection::projection_error)
        }
    }
}

/// Ask the person by an `elicitation/create` request (sessions before
/// `2026-07-28`). A failed exchange, an expired wait and a cancelled call are
/// no answer.
async fn elicit_choice(
    context: &RequestContext<RoleServer>,
    choice: &unica_execution_contract::invocation::HumanChoice,
) -> Option<rmcp::model::ElicitResult> {
    let asked = context.peer.create_elicitation_with_timeout(
        human_choice::elicitation_params(choice),
        Some(human_choice::ELICITATION_TIMEOUT),
    );
    tokio::select! {
        answer = asked => answer.ok(),
        () = context.ct.cancelled() => None,
    }
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

#[cfg(feature = "test-support")]
fn call_tool_result(
    app: &UnicaApplication,
    name: &str,
    args: &Map<String, Value>,
    cancellation: CancellationToken,
) -> Result<OperationResult, (i32, String)> {
    call_tool_result_observed(app, name, args, cancellation, Arc::new(NoopProgressSink))
}

#[cfg(test)]
mod human_choice_tests;
#[cfg(test)]
mod medium_tests;
