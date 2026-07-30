//! Public `unica` stdio MCP server on the official Rust SDK (`rmcp`).
//!
//! ADR-0013: the SDK owns the JSON-RPC loop, handshake, protocol version
//! negotiation, per-request task spawning, `ping`, and `notifications/cancelled`
//! bookkeeping. This module only maps SDK requests onto the transport-neutral
//! application layer (ADR-0002) and keeps the tool contract data-driven from
//! operation descriptors (ADR-0001) instead of SDK macros.

use crate::application::{input_schema_for_tool, ToolSpec, UnicaApplication};
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

/// Executes one tool call synchronously and renders the MCP text payload.
/// Injectable so transport tests can substitute slow or failing tools.
type ToolCallHandler = dyn Fn(&str, &Map<String, Value>, CancellationToken) -> Result<String, (i32, String)>
    + Send
    + Sync;

pub fn run_stdio() {
    let app = Arc::new(UnicaApplication::new());
    let handler: Arc<ToolCallHandler> = Arc::new(move |name, arguments, cancellation| {
        call_tool_text(&app, name, arguments, cancellation)
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
        let arguments = request.arguments.unwrap_or_default();
        let result =
            tokio::task::spawn_blocking(move || handler(&name, &arguments, cancellation)).await;
        bridge.abort();
        drop(admission);

        match result {
            Ok(Ok(text)) => Ok(CallToolResult::success(vec![ContentBlock::text(text)])),
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
            Tool::new(spec.name, spec.description, schema)
        })
        .collect()
}

fn call_tool_text(
    app: &UnicaApplication,
    name: &str,
    args: &Map<String, Value>,
    cancellation: CancellationToken,
) -> Result<String, (i32, String)> {
    let result = app
        .call_tool_cancellable(name, args, cancellation)
        .map_err(|message| (TOOL_EXECUTION_ERROR, message))?;
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
    use serde_json::json;
    use std::cell::Cell;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::time::timeout;

    const TEST_STEP: Duration = Duration::from_secs(10);

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
            call_tool_text(&app, name, arguments, cancellation)
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
        client.shutdown().await;
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
    fn source_resource_mcp_round_trip_reuses_instance_snapshot_and_hides_private_state() {
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
        std::fs::write(
            source.join("CommonModules/Shared/Ext/Module.bsl"),
            b"\xef\xbb\xbfProcedure Run()\r\nEndProcedure\r\n",
        )
        .unwrap();

        let app = UnicaApplication::new();
        let resources_args = json!({
            "cwd": root,
            "sourceSet": "main",
            "metadataPath": "CommonModule.Shared.Module",
            "scope": "self",
            "limit": 50
        })
        .as_object()
        .unwrap()
        .clone();
        let resources: Value = serde_json::from_str(
            &call_tool_text(
                &app,
                "unica.source.resources",
                &resources_args,
                CancellationToken::new(),
            )
            .unwrap(),
        )
        .unwrap();
        assert!(resources["ok"].as_bool().unwrap());
        assert_eq!(resources["data"]["completeness"], "complete");
        assert_eq!(resources["data"]["resources"].as_array().unwrap().len(), 1);
        assert_eq!(resources["cache"]["events"], json!([]));
        assert_eq!(resources["cache"]["invalidated"], json!([]));
        assert_no_private_source_resource_keys(&resources);

        let read_args = json!({
            "cwd": root,
            "snapshotId": resources["data"]["snapshotId"],
            "resourceId": resources["data"]["resources"][0]["resourceId"],
            "offset": 0,
            "limit": 9
        })
        .as_object()
        .unwrap()
        .clone();
        let read: Value = serde_json::from_str(
            &call_tool_text(
                &app,
                "unica.source.read",
                &read_args,
                CancellationToken::new(),
            )
            .unwrap(),
        )
        .unwrap();
        assert!(read["ok"].as_bool().unwrap());
        assert_eq!(read["data"]["contentEncoding"], "utf-8");
        assert_eq!(read["data"]["textProfile"]["bomPrefixBytes"], 3);
        assert_eq!(read["data"]["textProfile"]["eol"], "crlf");
        assert!(read["data"]["length"].as_u64().unwrap() <= 9);
        assert_no_private_source_resource_keys(&read);

        let other_app_error = call_tool_text(
            &UnicaApplication::new(),
            "unica.source.read",
            &read_args,
            CancellationToken::new(),
        )
        .unwrap_err();
        assert!(
            other_app_error.1.contains("snapshot_not_found"),
            "{other_app_error:?}"
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
            Ok("unreachable success".to_string())
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
            Ok("unreachable success".to_string())
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
        let registry = Arc::new(InFlightRegistry::default());
        let guard = registry.admit().unwrap();
        let cancellation = guard.token();
        let guard_thread = std::thread::spawn(move || {
            while !cancellation.is_cancelled() {
                std::thread::yield_now();
            }
            std::thread::sleep(Duration::from_millis(80));
            drop(guard);
        });
        let provider_budget = Cell::new(None);

        assert!(drain_mcp_shutdown_with(
            &registry,
            Duration::from_millis(200),
            |remaining| {
                provider_budget.set(Some(remaining));
                true
            }
        ));
        assert!(
            provider_budget.get().unwrap() < Duration::from_millis(150),
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
            Ok("released".to_string())
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
            assert_eq!(response["result"]["content"][0]["text"], "released");
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
        std::fs::write(src.join("CommonModules/Sample.xml"), "<MetaDataObject/>").unwrap();
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
