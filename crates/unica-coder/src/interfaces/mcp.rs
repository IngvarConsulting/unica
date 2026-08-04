//! Public `unica` stdio MCP server on the official Rust SDK (`rmcp`).
//!
//! ADR-0013: the SDK owns the JSON-RPC loop, handshake, protocol version
//! negotiation, per-request task spawning, `ping`, and `notifications/cancelled`
//! bookkeeping. This module only maps SDK requests onto the transport-neutral
//! application layer (ADR-0002) and keeps the tool contract data-driven from
//! operation descriptors (ADR-0001) instead of SDK macros.

use crate::application::{
    input_schema_for_tool, metadata_argument_failure_result, operation_result_output_schema,
    OperationResult, ToolSpec, UnicaApplication,
};
use crate::domain::cancellation::CancellationToken;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ContentBlock, ErrorCode, ErrorData, Implementation,
    InitializeResult, ListToolsResult, PaginatedRequestParams, ProtocolVersion, ServerCapabilities,
    ServerInfo, Tool,
};
use rmcp::service::{RequestContext, ServerInitializeError};
use rmcp::{RoleServer, ServerHandler, ServiceExt};
use serde_json::{Map, Value};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

pub const MCP_MAX_TOOL_WORKERS: usize = 32;
const EOF_CANCELLATION_GRACE: Duration = Duration::from_secs(2);
const RUNTIME_SHUTDOWN_GRACE: Duration = Duration::from_millis(250);
const TOOL_EXECUTION_ERROR: i32 = -32000;

/// Executes one tool call synchronously without leaking SDK types into the application.
/// Injectable so transport tests can substitute slow or failing tools.
type ToolCallHandler = dyn Fn(&str, &Map<String, Value>, CancellationToken) -> Result<OperationResult, (i32, String)>
    + Send
    + Sync;

pub fn run_stdio() {
    let app = Arc::new(UnicaApplication::new());
    let handler: Arc<ToolCallHandler> = Arc::new(move |name, arguments, cancellation| {
        call_tool_result(&app, name, arguments, cancellation)
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
            "unica mcp shutdown grace expired while tool calls or code-search providers were cleaning up"
        );
    }
    runtime.shutdown_timeout(RUNTIME_SHUTDOWN_GRACE);
}

fn drain_mcp_shutdown(in_flight: &InFlightRegistry, grace: Duration) -> bool {
    drain_mcp_shutdown_with(in_flight, grace, |remaining| {
        crate::application::code_intelligence::drain_code_search_workers(remaining)
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
}

impl UnicaServer {
    fn new(handler: Arc<ToolCallHandler>) -> Self {
        Self {
            handler,
            in_flight: Arc::new(InFlightRegistry::default()),
        }
    }

    fn in_flight(&self) -> Arc<InFlightRegistry> {
        Arc::clone(&self.in_flight)
    }
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
        let arguments = request.arguments.unwrap_or_default();
        let result =
            tokio::task::spawn_blocking(move || handler(&handler_name, &arguments, cancellation))
                .await;
        bridge.abort();
        drop(admission);

        match result {
            Ok(Ok(result)) => render_tool_result(&name, result),
            Ok(Err((code, message))) => Err(ErrorData::new(ErrorCode(code), message, None)),
            Err(join_error) => Err(ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                format!("tool worker failed: {join_error}"),
                None,
            )),
        }
    }
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
            if is_meta_tool(spec.name) {
                let output_schema = match operation_result_output_schema() {
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

fn is_meta_tool(name: &str) -> bool {
    matches!(
        name,
        "unica.meta.info" | "unica.meta.add" | "unica.meta.edit" | "unica.meta.remove"
    )
}

fn render_tool_result(name: &str, result: OperationResult) -> Result<CallToolResult, ErrorData> {
    let value = serde_json::to_value(&result)
        .map_err(|error| ErrorData::new(ErrorCode::INTERNAL_ERROR, error.to_string(), None))?;
    if is_meta_tool(name) {
        return Ok(if result.ok {
            CallToolResult::structured(value)
        } else {
            CallToolResult::structured_error(value)
        });
    }
    let text = serde_json::to_string_pretty(&value)
        .map_err(|error| ErrorData::new(ErrorCode::INTERNAL_ERROR, error.to_string(), None))?;
    Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
}

fn call_tool_result(
    app: &UnicaApplication,
    name: &str,
    args: &Map<String, Value>,
    cancellation: CancellationToken,
) -> Result<OperationResult, (i32, String)> {
    if let Some(result) = metadata_argument_failure_result(name, args) {
        return Ok(result);
    }
    app.call_tool_cancellable(name, args, cancellation)
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
    use crate::domain::cache::CacheReport;
    use serde_json::json;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::time::timeout;

    const TEST_STEP: Duration = Duration::from_secs(10);

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
        Arc::new(move |name, arguments, cancellation| {
            call_tool_result(&app, name, arguments, cancellation)
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
        assert!(
            compact_result_bytes < 1_300_000,
            "tools/list result consumes {compact_result_bytes} compact JSON bytes"
        );
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

        let previous_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(&root).unwrap();
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
        std::env::set_current_dir(previous_cwd).unwrap();
        std::fs::remove_dir_all(root).unwrap();
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
    fn tool_definitions_expose_flat_diagnostics_worktree_contract() {
        let listed = tool_definitions(&crate::application::tools());
        let diagnostics = listed
            .iter()
            .find(|tool| tool.name == "unica.code.diagnostics")
            .expect("unica.code.diagnostics must be listed");

        let schema = diagnostics.input_schema.as_ref();
        let properties = schema["properties"].as_object().unwrap();
        for name in [
            "cwd",
            "sourceDir",
            "mode",
            "path",
            "codes",
            "timeoutSeconds",
        ] {
            assert!(properties.contains_key(name), "missing {name}");
        }
        assert!(schema.get("oneOf").is_none());
    }

    #[test]
    fn native_tool_schema_is_typed_and_does_not_expose_raw_args() {
        let listed = tool_definitions(&crate::application::tools());
        let cf_info = listed
            .iter()
            .find(|tool| tool.name == "unica.cf.info")
            .expect("unica.cf.info must be listed");

        let schema = cf_info.input_schema.as_ref();
        assert_eq!(schema["additionalProperties"], false);
        assert!(schema["properties"].get("ConfigPath").is_some());
        assert!(schema["properties"].get("cwd").is_some());
        assert!(schema["properties"].get("dryRun").is_some());
        assert!(schema["properties"].get("args").is_none());
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
        assert_eq!(schema["required"], json!(["RightsPath"]));
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
            assert!(
                tool.input_schema["properties"].get("args").is_none(),
                "{} must not expose raw adapter args",
                tool.name
            );
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
        client.shutdown().await;
    }

    #[tokio::test]
    async fn ping_stays_responsive_and_cancellation_reaches_the_tool() {
        let cancellation_seen = Arc::new(AtomicBool::new(false));
        let seen = Arc::clone(&cancellation_seen);
        let handler: Arc<ToolCallHandler> = Arc::new(move |_, _, cancellation| {
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
        let handler: Arc<ToolCallHandler> = Arc::new(move |_, _, cancellation| {
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
        let held = Instant::now();
        std::thread::sleep(Duration::from_millis(20));
        release_tx.send(()).unwrap();
        let held = held.elapsed();

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
        let handler: Arc<ToolCallHandler> = Arc::new(move |_, _, _| {
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
