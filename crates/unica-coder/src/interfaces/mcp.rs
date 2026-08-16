//! Public `unica` stdio MCP server on the official Rust SDK (`rmcp`).
//!
//! ADR-0013: the SDK owns the JSON-RPC loop, handshake, protocol version
//! negotiation, per-request task spawning, `ping`, and `notifications/cancelled`
//! bookkeeping. This module only maps SDK requests onto the transport-neutral
//! application layer (ADR-0002) and keeps the tool contract data-driven from
//! operation descriptors (ADR-0001) instead of SDK macros.

use crate::application::{
    code_search_output_schema, input_schema_for_tool, metadata_argument_failure_result,
    operation_result_output_schema, role_edit_argument_failure_result, role_edit_output_schema,
    CodeIntelligenceOperation, OperationResult, ToolHandler, ToolSpec, UnicaApplication,
};
use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::{
    NoopSearchProgressSink, SearchProgressSink, SearchProgressSnapshot,
};
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ContentBlock, ErrorCode, ErrorData, Implementation,
    InitializeResult, ListToolsResult, Meta, PaginatedRequestParams, ProgressNotificationParam,
    ProtocolVersion, ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::{RequestContext, ServerInitializeError};
use rmcp::{RoleServer, ServerHandler, ServiceExt};
use serde_json::{Map, Value};
use std::collections::HashSet;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

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
        Arc<dyn SearchProgressSink>,
    ) -> Result<OperationResult, (i32, String)>
    + Send
    + Sync;

pub fn run_stdio() {
    let app = Arc::new(UnicaApplication::new());
    let handler: Arc<ToolCallHandler> = Arc::new(move |name, arguments, cancellation, progress| {
        call_tool_result_observed(&app, name, arguments, cancellation, progress)
    });
    let server = UnicaServer::new(handler);
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

pub struct UnicaServer {
    handler: Arc<ToolCallHandler>,
    in_flight: Arc<InFlightRegistry>,
    structured_tools: HashSet<&'static str>,
}

impl UnicaServer {
    fn new(handler: Arc<ToolCallHandler>) -> Self {
        Self {
            handler,
            in_flight: Arc::new(InFlightRegistry::default()),
            structured_tools: crate::application::tools()
                .into_iter()
                .filter_map(|spec| has_structured_output(&spec).then_some(spec.name))
                .collect(),
        }
    }

    fn in_flight(&self) -> Arc<InFlightRegistry> {
        Arc::clone(&self.in_flight)
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

fn has_structured_output(spec: &ToolSpec) -> bool {
    structured_output_schema(spec).is_some()
}

impl ServerHandler for UnicaServer {
    fn get_info(&self) -> ServerInfo {
        InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_protocol_version(ProtocolVersion::LATEST)
            .with_server_info(Implementation::new("unica", env!("CARGO_PKG_VERSION")))
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(tool_definitions(
            &crate::application::tools(),
        )))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
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

        let handler = Arc::clone(&self.handler);
        let name = request.name.to_string();
        let handler_name = name.clone();
        let progress_token = request
            .meta
            .as_ref()
            .and_then(Meta::get_progress_token)
            .or_else(|| context.meta.get_progress_token());
        let arguments = request.arguments.unwrap_or_default();
        let progress_forwarding = if let Some(progress_token) = progress_token {
            let (sender, mut receiver) =
                tokio::sync::mpsc::unbounded_channel::<Option<SearchProgressSnapshot>>();
            let sink: Arc<dyn SearchProgressSink> = Arc::new(McpSearchProgressSink {
                sender: sender.clone(),
            });
            let peer = context.peer.clone();
            let forwarder = tokio::spawn(async move {
                while let Some(message) = receiver.recv().await {
                    let Some(snapshot) = message else {
                        break;
                    };
                    let mut meta = Meta::default();
                    meta.0.insert(
                        "io.unica/searchProgress".to_string(),
                        serde_json::to_value(&snapshot).unwrap_or(Value::Null),
                    );
                    let notification = ProgressNotificationParam::new(
                        progress_token.clone(),
                        snapshot.terminal_roles() as f64,
                    )
                    .with_total(snapshot.providers.len() as f64)
                    .with_message(progress_message(&snapshot));
                    let mut notification = notification;
                    notification.meta = Some(meta);
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
                sink: Arc::new(NoopSearchProgressSink),
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
            handler(&handler_name, &arguments, cancellation, progress)
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

        match result {
            Ok(Ok(result)) => {
                render_tool_result(self.structured_tools.contains(name.as_str()), result)
            }
            Ok(Err((code, message))) => Err(ErrorData::new(ErrorCode(code), message, None)),
            Err(join_error) => Err(ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                format!("tool worker failed: {join_error}"),
                None,
            )),
        }
    }
}

struct McpProgressForwarding {
    sink: Arc<dyn SearchProgressSink>,
    forwarder: Option<tokio::task::JoinHandle<()>>,
    stop: Option<tokio::sync::mpsc::UnboundedSender<Option<SearchProgressSnapshot>>>,
}

struct McpSearchProgressSink {
    sender: tokio::sync::mpsc::UnboundedSender<Option<SearchProgressSnapshot>>,
}

impl SearchProgressSink for McpSearchProgressSink {
    fn publish(&self, snapshot: SearchProgressSnapshot) {
        let _ = self.sender.send(Some(snapshot));
    }
}

fn progress_message(snapshot: &SearchProgressSnapshot) -> String {
    snapshot
        .providers
        .iter()
        .map(|provider| {
            let detail = provider
                .detail_code
                .as_deref()
                .unwrap_or_else(|| provider.phase.as_str());
            format!(
                "{}: {} {detail} ({} results)",
                provider.identity.role.as_str(),
                provider.state.as_str(),
                provider.results_found
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// Data-driven MCP tool definitions from the application descriptor registry.
pub fn tool_definitions(specs: &[ToolSpec]) -> Vec<Tool> {
    specs
        .iter()
        .map(|spec| {
            let schema = match input_schema_for_tool(spec) {
                Value::Object(schema) => schema,
                other => {
                    unreachable!("tool {} produced a non-object schema: {other}", spec.name)
                }
            };
            let tool = Tool::new(spec.name, spec.description, schema);
            if let Some(schema) = structured_output_schema(spec) {
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
    call_tool_result_observed(
        app,
        name,
        args,
        cancellation,
        Arc::new(NoopSearchProgressSink),
    )
}

fn call_tool_result_observed(
    app: &UnicaApplication,
    name: &str,
    args: &Map<String, Value>,
    cancellation: CancellationToken,
    progress: Arc<dyn SearchProgressSink>,
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
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::time::timeout;

    const TEST_STEP: Duration = Duration::from_secs(10);

    fn object_schema_property_maps(
        schema: &serde_json::Map<String, serde_json::Value>,
    ) -> Vec<&serde_json::Map<String, serde_json::Value>> {
        if let Some(properties) = schema
            .get("properties")
            .and_then(serde_json::Value::as_object)
        {
            return vec![properties];
        }
        schema
            .get("oneOf")
            .expect("tool schema publishes properties or closed object oneOf branches")
            .as_array()
            .expect("tool schema publishes properties or closed object oneOf branches")
            .iter()
            .map(|branch| {
                branch["properties"]
                    .as_object()
                    .expect("oneOf branch properties are an object")
            })
            .collect()
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
        let (client_io, server_io) = tokio::io::duplex(4 * 1024 * 1024);
        let server = UnicaServer::new(handler);
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
    async fn applied_runtime_refusal_is_one_terminal_result_without_input_disclosure() {
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
        assert!(response.get("error").is_none(), "{response}");
        assert_eq!(response["result"]["isError"], true, "{response}");
        let receipt: Value = serde_json::from_str(
            response["result"]["content"][0]["text"]
                .as_str()
                .expect("terminal refusal text"),
        )
        .unwrap();
        assert_eq!(receipt["ok"], false, "{receipt}");
        assert!(receipt["errors"][0]
            .as_str()
            .is_some_and(|error| error.starts_with("runtime_operation_unbounded:")));
        let serialized = response.to_string();
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

    #[tokio::test]
    async fn tools_list_describes_runtime_execute_dry_run_as_preview_only() {
        let (mut client, _) = spawn_server(application_handler());
        client.initialize().await;
        client
            .send(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {} }))
            .await;
        let response = client.receive().await;
        let listed = response["result"]["tools"].as_array().unwrap();
        let runtime = listed
            .iter()
            .find(|tool| tool["name"] == "unica.runtime.execute")
            .expect("runtime tool is listed");
        assert_eq!(
            runtime["description"],
            "Preview typed v8-runner workflows; current applied operations return a terminal fail-closed result before workspace discovery or process spawn."
        );
        assert_eq!(
            runtime["inputSchema"]["properties"]["dryRun"]["description"],
            "Preview typed v8-runner runtime arguments; omitted or true reports the planned command without mutation, while false currently returns runtime_operation_unbounded before workspace discovery or process spawn."
        );
        client.shutdown().await;
    }

    #[tokio::test]
    async fn progress_token_receives_typed_search_snapshot_before_result() {
        let handler: Arc<ToolCallHandler> = Arc::new(|_, _, _, progress| {
            progress.publish(crate::domain::code_intelligence::SearchProgressSnapshot {
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
            });
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
        let retained = Arc::new(Mutex::new(None::<Arc<dyn SearchProgressSink>>));
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
        let retained = Arc::new(Mutex::new(None::<Arc<dyn SearchProgressSink>>));
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
                progress.publish(crate::domain::code_intelligence::SearchProgressSnapshot {
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
                });
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
        // The call is released by a channel rather than by a sleep, and the
        // budget is compared against the *measured* time the call stayed
        // tracked. A loaded runner moves both sides of that comparison
        // together, so the aggregate grace only has to outlast the test, not
        // the scheduler.
        const AGGREGATE_GRACE: Duration = Duration::from_secs(30);

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

        // Cancellation only reaches the call from inside the drain, so the
        // deadline was already running when this arrives: everything measured
        // from here is budget the call spent before provider cleanup starts.
        cancelled_rx
            .recv_timeout(AGGREGATE_GRACE)
            .expect("cancellation did not reach the tracked call within the aggregate grace");
        let held_since = Instant::now();
        std::thread::sleep(Duration::from_millis(20));
        // Sampled *before* the release is published, so `held` is a strict
        // sub-interval of what the drain observes. Sampling after the send
        // charges this thread's own scheduling delay to `held` while the drain
        // never sees it, and a loaded runner then inverts the comparison.
        let held = held_since.elapsed();
        release_tx.send(()).unwrap();

        let (drained, provider_budget) = drain_thread.join().unwrap();
        assert!(
            drained,
            "the drain gave up while the call was still tracked"
        );
        assert!(
            provider_budget.unwrap() <= AGGREGATE_GRACE.saturating_sub(held),
            "provider cleanup received a fresh grace instead of the aggregate remainder"
        );

        guard_thread.join().unwrap();
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
