use super::*;
use crate::application::{ResultContract, ToolExecution};
use crate::daemon_router::remaining_invocation_budget;
use crate::in_flight::drain_mcp_shutdown_with;
use crate::test_support::*;
use serde_json::json;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::time::timeout;
use unica_daemon_protocol::protocol_v5::V5ClientRequest;
use unica_execution::invocation::{INVOCATION_HANDOFF_WINDOW, RESPONSE_SERIALIZATION_MARGIN};
use unica_workspace::cache::CacheReport;

use unica_daemon_protocol::protocol_v5::task_frame_fixtures::{
    completed_task_response as daemon_task_frame_value, frame as daemon_frame_bytes,
    malformed_task_frames as malformed_daemon_task_frames, RESULT_MARKER,
};

#[test]
fn unica_server_implements_official_rmcp_server_handler() {
    super::assert_unica_server_implements_official_rmcp_server_handler();
}

/// rule: CTR.WIRE.CANONICAL-TOOL-PROFILES
/// rule: INV.WIRE.SURFACE-RELEASE-ROUTING
#[test]
fn production_mcp_surface_exposes_only_canonical_v13_tools_and_task_compatibility() {
    let canonical: Arc<CanonicalCallHandler> = Arc::new(|_, _, _, _, _| {
        direct_outcome(unica_execution_contract::invocation::DomainResult::success(
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

/// rule: INV.WIRE.SOURCE-TEXT-RESPONSE-BUDGET
#[test]
fn every_tool_that_can_return_source_text_declares_its_result_size() {
    let (key, chars) = unica_bootstrap::source_text_tool_meta();
    // Текст приходит прямым ответом `view` и итогом задания через
    // `task.result`; native `tasks/get` инструментом не является.
    let mut declaring = Vec::new();
    for profile in [V13TaskProfile::Native, V13TaskProfile::Compatibility] {
        for tool in v13_tool_definitions(profile) {
            assert!(tool.output_schema.is_none(), "{}", tool.name);
            match tool.meta.as_ref().and_then(|meta| meta.get(key).cloned()) {
                Some(declared) => {
                    assert_eq!(declared, Value::from(chars), "{}", tool.name);
                    declaring.push(tool.name.to_string());
                }
                None => assert!(tool.meta.is_none(), "{}", tool.name),
            }
        }
    }
    assert_eq!(
        declaring,
        ["unica.view", "unica.view", "unica.task.result"],
        "native and compatibility profiles"
    );
    // Бюджет любого хоста в байтах не длиннее объявленного предела
    // в символах: символ занимает хотя бы байт.
    assert!(unica_bootstrap::MAX_RESPONSE_BUDGET as u64 <= chars);
}

/// rule: INV.WIRE.SOURCE-TEXT-RESPONSE-BUDGET
#[test]
fn client_name_comes_from_initialize_or_from_the_request_meta() {
    let meta = json!({"io.modelcontextprotocol/clientInfo": {"name": "direct", "version": "1"}});
    let meta = meta.as_object().unwrap();
    assert_eq!(super::client_name(None, meta).as_deref(), Some("direct"));
    assert_eq!(
        super::client_name(Some("initialized".into()), meta).as_deref(),
        Some("initialized")
    );
    assert_eq!(super::client_name(None, &Map::new()), None);
}

/// rule: INV.SURFACE.ARGUMENTS-DESCRIBED
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
        for properties in std::iter::once(arguments).chain(object_schema_property_maps(arguments)) {
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

/// Visits every schema node of a published tool with its JSON pointer.
/// Keys of `properties` and similar maps are names, not keywords: an
/// argument called `type` or `default` is not mistaken for one.
fn visit_schema_nodes<'a>(
    value: &'a Value,
    pointer: &str,
    visit: &mut dyn FnMut(&str, &'a serde_json::Map<String, Value>),
) {
    const NAMED_SCHEMA_MAPS: &[&str] = &[
        "properties",
        "patternProperties",
        "dependentSchemas",
        "definitions",
        "$defs",
    ];
    match value {
        Value::Object(object) => {
            visit(pointer, object);
            for (key, child) in object {
                let child_pointer = format!("{pointer}/{key}");
                match (NAMED_SCHEMA_MAPS.contains(&key.as_str()), child) {
                    (true, Value::Object(named)) => {
                        for (name, schema) in named {
                            visit_schema_nodes(schema, &format!("{child_pointer}/{name}"), visit);
                        }
                    }
                    _ => visit_schema_nodes(child, &child_pointer, visit),
                }
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                visit_schema_nodes(item, &format!("{pointer}/{index}"), visit);
            }
        }
        _ => {}
    }
}

fn published_tool_values(profile: V13TaskProfile) -> Vec<(String, Value)> {
    v13_tool_definitions(profile)
        .iter()
        .map(|tool| {
            (
                tool.name.to_string(),
                serde_json::to_value(tool).expect("published tool serializes"),
            )
        })
        .collect()
}

/// Clients that translate tool schemas into a single-`type` dialect
/// refuse or mishandle `"type": [...]`; no published schema may use it.
#[test]
fn published_tool_schemas_declare_one_type_per_node() {
    for profile in [V13TaskProfile::Native, V13TaskProfile::Compatibility] {
        for (name, tool) in published_tool_values(profile) {
            let mut arrays = Vec::new();
            visit_schema_nodes(&tool, "", &mut |pointer, node| {
                if node.get("type").is_some_and(Value::is_array) {
                    arrays.push(pointer.to_string());
                }
            });
            assert!(
                arrays.is_empty(),
                "{name} publishes type arrays at {arrays:?}"
            );
        }
    }
    // The walker itself must see a type array where one is.
    let mut found = false;
    visit_schema_nodes(
        &json!({"properties": {"type": {"type": ["string", "null"]}}}),
        "",
        &mut |_, node| found |= node.get("type").is_some_and(Value::is_array),
    );
    assert!(found);
}

/// A form client fills every schema `default` into the call. A tool
/// that answers `{}` must therefore publish none: `unica.view {limit: 20}`
/// is refused because limit requires at. A tool with required arguments
/// gets a default only when it is valid next to those arguments alone;
/// each such default is named here with that justification.
#[test]
fn published_defaults_never_turn_a_form_call_invalid() {
    // search: `query` alone plus corpus `text`, regex `false` and limit 20
    // is the documented literal text search. docs: `query` plus limit 20.
    // task.result: `taskId` plus the 7000 ms wait.
    const JUSTIFIED: &[(&str, &str)] = &[
        ("unica.search", "/inputSchema/properties/corpus"),
        ("unica.search", "/inputSchema/properties/regex"),
        ("unica.search", "/inputSchema/properties/limit"),
        ("unica.docs", "/inputSchema/properties/limit"),
        ("unica.task.result", "/inputSchema/properties/waitMs"),
    ];
    for (name, tool) in [V13TaskProfile::Native, V13TaskProfile::Compatibility]
        .into_iter()
        .flat_map(published_tool_values)
    {
        let required = tool["inputSchema"]["required"]
            .as_array()
            .is_some_and(|required| !required.is_empty());
        let mut defaults = Vec::new();
        visit_schema_nodes(
            &tool["inputSchema"],
            "/inputSchema",
            &mut |pointer, node| {
                if node.contains_key("default") {
                    defaults.push(pointer.to_string());
                }
            },
        );
        for pointer in defaults {
            assert!(
                required,
                "{name} answers an empty call yet publishes a default at {pointer}"
            );
            assert!(
                JUSTIFIED.contains(&(name.as_str(), pointer.as_str())),
                "{name} publishes an unreviewed default at {pointer}"
            );
        }
    }
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
        direct_outcome(unica_execution_contract::invocation::DomainResult::success(
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
                "provider": "semantic-fixture",
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

/// Канонический обработчик, который отвечает прямым результатом.
fn canonical_direct_handler(summary: &'static str) -> Arc<CanonicalCallHandler> {
    Arc::new(move |_, _, _, _, _| direct_outcome(canonical_result(summary)))
}

/// Канонический сервер с прямым ответом на каждый вызов.
fn canonical_direct_server() -> UnicaServer {
    UnicaServer::with_canonical_v13(canonical_direct_handler("direct"))
}

/// Синтетический каталог длиннее двух страниц `tools/list`: компактный
/// каталог v0.13 не доходит до второй страницы.
fn synthetic_tool_catalog() -> Vec<Tool> {
    (0..2 * TOOLS_PAGE_SIZE + 3)
        .map(|index| {
            let mut tool = Tool::new(
                format!("unica.synthetic.{index}"),
                String::new(),
                Map::from_iter([("type".to_string(), json!("object"))]),
            );
            tool.description = None;
            tool
        })
        .collect()
}

fn synthetic_catalog_server() -> UnicaServer {
    UnicaServer::with_canonical_v13_tool_catalog_for_test(
        canonical_direct_handler("direct"),
        synthetic_tool_catalog(),
    )
}

#[test]
fn initialize_carries_what_a_killed_startup_left_behind() {
    // Убитая установка своего провода не имела: её рассказ приходит сюда
    // от загрузчика и уходит вызывающему обычным ответом на `initialize`.
    let notice = "a Unica startup was killed while downloading unica 0.13.0";
    let server = UnicaServer::with_canonical_v13_and_startup_notice_for_test(
        canonical_direct_handler("direct"),
        Some(notice.to_owned()),
    );

    let instructions = server.get_info().instructions.expect("instructions");
    assert!(instructions.contains("unica.view"), "{instructions}");
    assert!(instructions.contains("sourceSet"), "{instructions}");
    assert!(instructions.contains(notice), "{instructions}");
}

#[test]
fn a_session_without_notice_still_carries_bootstrap_instructions() {
    let server = UnicaServer::with_canonical_v13_and_startup_notice_for_test(
        canonical_direct_handler("direct"),
        None,
    );

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

/// rule: INV.WIRE.SDK-INITIALIZE
#[tokio::test]
async fn initialize_uses_single_public_server_name_and_negotiates_version() {
    let (mut client, _) = spawn_unica_server(canonical_direct_server());
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
    let (mut client, _) = spawn_unica_server(canonical_direct_server());
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
    // Tasks are stripped below 2026-07-28; the host workspace channels
    // are the implemented frontend surface, not an advertised feature.
    assert_eq!(
        response["result"]["capabilities"],
        json!({
            "tools": {},
            "experimental": unica_bootstrap::host_workspace_capabilities()
        }),
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

/// rule: CTR.WIRE.LIST-CACHE-FIELDS
#[tokio::test]
async fn modern_list_results_carry_required_cache_fields_and_legacy_stays_clean() {
    // 2026-07-28 wire schemas (SEP-2549) require ttlMs/cacheScope on list
    // results; the legacy shape must stay byte-identical to pre-2026.
    let (mut client, _) = spawn_unica_server(canonical_direct_server());
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

    let (mut client, _) = spawn_unica_server(canonical_direct_server());
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
    let (mut client, _) = spawn_unica_server(canonical_direct_server());
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
    let (mut client, _) = spawn_unica_server(canonical_direct_server());
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

/// rule: CTR.WIRE.CANONICAL-TOOL-PROFILES
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
            direct_outcome(unica_execution_contract::invocation::DomainResult::success(
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
        for (name, pointer, required) in [
            ("unica.docs", "/required", json!(["query"])),
            ("unica.diff", "/else/required", json!(["left", "right"])),
            ("unica.diff", "/then/required", json!(["executionToken"])),
        ] {
            let tool = tools.iter().find(|tool| tool["name"] == name).unwrap();
            assert_eq!(tool["inputSchema"].pointer(pointer), Some(&required));
        }
        let diff = tools
            .iter()
            .find(|tool| tool["name"] == "unica.diff")
            .unwrap();
        assert_eq!(diff["inputSchema"]["required"], json!([]));
        let diff_validator = jsonschema::validator_for(&diff["inputSchema"])
            .expect("valid conditional diff wire schema");
        for arguments in [
            json!({"left":"main:Catalog.Items", "right":"main:Catalog.Other"}),
            json!({"left":"main:Catalog.Items", "right":"main:Catalog.Other", "limit":50}),
            json!({"executionToken":"saved-plan"}),
            json!({"executionToken":"saved-plan", "limit":1, "cursor":""}),
            json!({"left":"main:Catalog.Items", "right":"main:Catalog.Other", "cursor":""}),
        ] {
            assert!(
                diff_validator.is_valid(&arguments),
                "valid diff request: {arguments}"
            );
        }
        for arguments in [
            json!({}),
            json!({"left":"main:Catalog.Items"}),
            json!({"right":"main:Catalog.Other"}),
            json!({"cursor":""}),
            json!({"executionToken":""}),
            json!({"executionToken":null}),
            json!({"executionToken":"saved-plan", "left":"main:Catalog.Items", "right":"main:Catalog.Other"}),
            json!({"left":"main:Catalog.Items", "right":"main:Catalog.Other", "cursor":7}),
            json!({"executionToken":"saved-plan", "limit":51}),
            json!({"left":"main:Catalog.Items", "right":"main:Catalog.Other", "limit":51}),
            json!({"executionToken":"saved-plan", "limit":0}),
        ] {
            assert!(
                !diff_validator.is_valid(&arguments),
                "invalid diff request: {arguments}"
            );
        }
        client.shutdown().await;
    }
}

#[tokio::test]
async fn compatibility_tools_return_durable_receipts_without_native_tasks_or_reexecution() {
    compatibility_receipts_case().await;
}

#[test]
fn compatibility_result_wait_is_bounded_by_request_and_original_frontend_window() {
    compatibility_wait_budget_case();
}

#[tokio::test]
async fn compatibility_wait_zero_and_one_bound_only_the_daemon_wait() {
    compatibility_wait_single_deadline_case().await;
}

/// A ready result read with `waitMs: 0` on a slow host arrives as the
/// result: connect and exchange spend the frontend transport cutoff, not
/// `waitMs + 125 ms` (#2063).
/// rule: CTR.WIRE.COMPATIBILITY-TASK-TOOLS
#[tokio::test]
async fn compatibility_ready_result_at_wait_zero_survives_a_slow_connect() {
    let task_id = unica_execution_contract::invocation::TaskId::new();
    let fake = FakeDaemon::start_with_handshake_delay(
        Box::new(move |_, request| {
            assert_eq!(
                request,
                &V5ClientRequest::WaitTask {
                    task_id,
                    wait_ms: 0
                }
            );
            std::thread::sleep(Duration::from_millis(200));
            Step::Reply(unica_daemon_protocol::protocol_v5::V5ServerResponse::Task {
                snapshot: canonical_snapshot(
                    task_id,
                    unica_execution_contract::invocation::InvocationStatus::Completed,
                    Some(unica_execution_contract::invocation::DomainResult::success(
                        "ready-result",
                    )),
                ),
            })
        }),
        Duration::from_millis(300),
    );
    let (mut mcp, _) = spawn_unica_server(UnicaServer::with_canonical_daemon(
        fake.owner(),
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
    assert_eq!(
        response["result"]["structuredContent"]["ok"], true,
        "{response}"
    );
    assert_eq!(
        response["result"]["structuredContent"]["summary"], "ready-result",
        "{response}"
    );
    mcp.shutdown().await;
}

fn compatibility_immediate_task_deadline_case(
    tool_name: &'static str,
    host_budget: Duration,
    handshake_elapsed: Duration,
    response_elapsed: Duration,
) -> unica_execution_contract::invocation::DomainResult {
    let task_id = unica_execution_contract::invocation::TaskId::new();
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
    let result = match outcome {
        SurfaceToolOutcome::Canonical(result) => result,
        SurfaceToolOutcome::CompatibilityTask { result, .. } => result.into_result(),
        _ => panic!("compatibility task tools must return canonical results"),
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

/// rule: CTR.WIRE.COMPATIBILITY-TASK-TOOLS
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

/// rule: CTR.WIRE.COMPATIBILITY-TASK-TOOLS
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

#[test]
fn compatibility_wait_preserves_frontend_cutoff_across_client_admission_pause() {
    compatibility_wait_frontend_cutoff_is_not_rebased_case();
}

#[test]
fn compatibility_wait_post_parse_expiry_wins_for_valid_and_malformed_near_limit_frames() {
    compatibility_wait_late_payload_case(true);
    compatibility_wait_late_payload_case(false);
}

#[test]
fn compatibility_wait_authenticated_transport_bounds_7000_and_earlier_host_cutoff() {
    // 7000 + 125 ms cutoff, minus the 60 ms handshake and the 125 ms margin;
    // a loaded runner only lowers the value, never raises it above 6940.
    compatibility_wait_authenticated_long_and_host_cutoff_case(7_000, None, 6_000..=6_940, true);
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

#[tokio::test]
async fn compatibility_adapter_rejects_every_hostile_status_payload_shape_without_leaking_failure()
{
    compatibility_hostile_status_payload_case().await;
}

/// rule: CTR.WIRE.COMPATIBILITY-TASK-TOOLS
#[tokio::test]
async fn compatibility_task_get_and_result_preserve_each_closed_failure_reason() {
    use unica_execution::invocation_store_v5::V5SafeFailureReason;
    use unica_execution_contract::invocation::{InvocationStatus, TaskId};

    for reason in V5SafeFailureReason::ALL {
        let task_id = TaskId::new();
        let mut snapshot = canonical_snapshot(task_id, InvocationStatus::Failed, None);
        let V5DaemonTaskSnapshot::Failed {
            reason: stored_reason,
            ..
        } = &mut snapshot
        else {
            panic!("failed fixture required");
        };
        *stored_reason = reason;
        let get_snapshot = snapshot.clone();
        let get: Arc<CanonicalTaskHandler> = Arc::new(move |_, _| Ok(get_snapshot.clone()));
        let wait_snapshot = snapshot.clone();
        let wait: Arc<CanonicalTaskWaitHandler> =
            Arc::new(move |_, _, _, _| Ok(wait_snapshot.clone()));
        let cancel = Arc::clone(&get);
        let call: Arc<CanonicalCallHandler> = Arc::new(move |_, _, _, _, _| {
            panic!("observation must never invoke a subject tool");
        });
        let (mut client, _) = spawn_unica_server(UnicaServer::with_canonical_v13_task_handlers(
            call, get, wait, cancel,
        ));
        for (id, name) in [(1, "unica.task.get"), (2, "unica.task.result")] {
            client.send(json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":name,"arguments":{"taskId":task_id.to_string()},"_meta":modern_meta()}})).await;
            let response = client.receive().await;
            let result = &response["result"]["structuredContent"];
            assert_eq!(result["ok"], false, "{response}");
            assert_eq!(result["data"]["task"]["status"], "failed", "{response}");
            assert_eq!(result["data"]["task"]["taskId"], task_id.to_string());
            assert_eq!(result["diagnostics"][0]["code"], "task_failed");
            assert_eq!(
                result["diagnostics"][0]["failureReason"],
                reason.wire_name(),
                "{name}: {response}"
            );
            assert!(result["diagnostics"][0].get("detailCode").is_none());
            assert!(result["next"].as_array().is_none_or(Vec::is_empty));
        }
        client.shutdown().await;
    }
}

/// A fake daemon that answers every Task frame with `frame` and counts the
/// Task operations it saw; any other request closes the session.
fn task_frame_fake(frame: Vec<u8>) -> FakeDaemon {
    FakeDaemon::start_with_raw_script(Box::new(move |_, request, writer| {
        use std::io::Write as _;
        match request {
            V5ClientRequest::GetTask { .. }
            | V5ClientRequest::WaitTask { .. }
            | V5ClientRequest::CancelTask { .. } => {
                let _ = writer.write_all(&frame);
                let _ = writer.flush();
                true
            }
            _ => false,
        }
    }))
}

/// Calls every Task read through the real exchange: the three
/// compatibility tools and the native `tasks/get` and `tasks/cancel`.
/// Returns the public code of each call, or `None` for a success.
async fn task_read_codes(
    owner: impl Fn() -> V5DaemonProcessOwner,
    task_id: unica_execution_contract::invocation::TaskId,
) -> Vec<(&'static str, Option<String>, String)> {
    let mut codes = Vec::new();
    let (mut compat, _) = spawn_unica_server(UnicaServer::with_canonical_daemon(
        owner(),
        "/workspace".to_string(),
    ));
    for (id, name, arguments) in [
        (1, "unica.task.get", json!({"taskId": task_id.to_string()})),
        (
            2,
            "unica.task.result",
            json!({"taskId": task_id.to_string(), "waitMs": 0}),
        ),
        (
            3,
            "unica.task.cancel",
            json!({"taskId": task_id.to_string()}),
        ),
    ] {
        compat
            .send(json!({
                "jsonrpc":"2.0", "id":id, "method":"tools/call",
                "params":{"name":name, "arguments":arguments, "_meta":modern_meta()}
            }))
            .await;
        let response = compat.receive().await;
        let result = &response["result"]["structuredContent"];
        let code = (result["ok"] != true).then(|| {
            result["diagnostics"][0]["code"]
                .as_str()
                .unwrap_or("<missing>")
                .to_owned()
        });
        codes.push((name, code, response.to_string()));
    }
    compat.shutdown().await;

    let (mut native, _) = spawn_unica_server(UnicaServer::with_canonical_daemon(
        owner(),
        "/workspace".to_string(),
    ));
    for (id, method) in [(4, "tasks/get"), (5, "tasks/cancel")] {
        native
            .send(json!({
                "jsonrpc":"2.0", "id":id, "method":method,
                "params":{"taskId":task_id.to_string(), "_meta":modern_tasks_meta()}
            }))
            .await;
        let response = native.receive().await;
        let code = response.get("error").map(|error| {
            error["data"]["code"]
                .as_str()
                .unwrap_or("<missing>")
                .to_owned()
        });
        codes.push((method, code, response.to_string()));
    }
    native.shutdown().await;
    codes
}

/// rule: INV.WIRE.TASK-RESPONSE-ERRORS
#[tokio::test]
async fn task_reads_report_complete_malformed_daemon_frames_as_protocol_failures() {
    let task_id = unica_execution_contract::invocation::TaskId::new();
    // Positive control: the same exchange publishes a strict frame.
    let valid = task_frame_fake(daemon_frame_bytes(&daemon_task_frame_value(task_id)));
    for (operation, code, response) in task_read_codes(|| valid.owner(), task_id).await {
        assert_eq!(code, None, "{operation} refused a strict frame: {response}");
    }

    for (case, frame) in malformed_daemon_task_frames(task_id) {
        let fake = task_frame_fake(frame);
        let codes = task_read_codes(|| fake.owner(), task_id).await;
        for (operation, code, response) in &codes {
            assert_eq!(
                code.as_deref(),
                Some("task_protocol_failed"),
                "{case} via {operation}: {response}"
            );
            assert!(
                !response.contains(RESULT_MARKER),
                "{case} via {operation} published the rejected payload: {response}"
            );
        }
        // One request per operation: the refusal neither retries the read
        // nor executes the subject operation again.
        let seen = fake.seen();
        assert_eq!(seen.len(), codes.len(), "{case}: {seen:?}");
        assert!(
            seen.iter().all(|request| matches!(
                request,
                V5ClientRequest::GetTask { .. }
                    | V5ClientRequest::WaitTask { .. }
                    | V5ClientRequest::CancelTask { .. }
            )),
            "{case}: {seen:?}"
        );
    }
}

/// An anchor session whose daemon stops listening right after the
/// handshake: every later operation connection is refused.
fn anchor_of_vanished_daemon() -> (V5DaemonProcessOwner, std::net::TcpStream) {
    use std::net::{Ipv4Addr, TcpListener};
    use unica_daemon_protocol::identity::CoreIdentity;
    use unica_daemon_protocol::protocol_v5::{
        read_bounded_v5_request_frame, V5EndpointRecord, V5HandshakeServerResponse,
    };

    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let record = V5EndpointRecord::new(
        CoreIdentity::production_v5(),
        listener.local_addr().unwrap().port(),
    )
    .unwrap();
    let ready = V5HandshakeServerResponse::ready(&record);
    let daemon = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
        read_bounded_v5_request_frame(&mut reader).unwrap();
        let mut writer = stream.try_clone().unwrap();
        crate::test_support::write_frame(&mut writer, &ready);
        stream
    });
    let owner =
        V5DaemonProcessOwner::connect_before(record, Instant::now() + Duration::from_secs(5))
            .unwrap();
    (owner, daemon.join().unwrap())
}

/// rule: INV.WIRE.TASK-RESPONSE-ERRORS
#[tokio::test]
async fn task_reads_report_unreachable_or_truncated_daemon_as_transport_failures() {
    let task_id = unica_execution_contract::invocation::TaskId::new();
    let complete = daemon_frame_bytes(&daemon_task_frame_value(task_id));

    let (anchor, _anchor_stream) = anchor_of_vanished_daemon();
    let anchor = std::sync::Mutex::new(Some(anchor));
    let unreachable = task_read_codes(
        || {
            anchor
                .lock()
                .unwrap()
                .take()
                .unwrap_or_else(|| anchor_of_vanished_daemon().0)
        },
        task_id,
    )
    .await;

    let closed = FakeDaemon::start_with_raw_script(Box::new(|_, _, _| false));
    let closed_codes = task_read_codes(|| closed.owner(), task_id).await;

    let prefix = complete[..complete.len() / 2].to_vec();
    let truncated = FakeDaemon::start_with_raw_script(Box::new(move |_, _, writer| {
        use std::io::Write as _;
        let _ = writer.write_all(&prefix);
        let _ = writer.flush();
        false
    }));
    let truncated_codes = task_read_codes(|| truncated.owner(), task_id).await;

    for (case, codes) in [
        ("connection refused", unreachable),
        ("closed before any byte", closed_codes),
        ("closed inside the frame", truncated_codes),
    ] {
        for (operation, code, response) in codes {
            assert_eq!(
                code.as_deref(),
                Some("task_transport_failed"),
                "{case} via {operation}: {response}"
            );
            // The observation goes to the frontend log on every path.
            assert!(
                !response.to_string().contains("taskObservation"),
                "{case} via {operation}: {response}"
            );
        }
    }
    assert_eq!(
        truncated.submissions() + closed.submissions(),
        0,
        "a failed read never executes the subject operation"
    );
}

async fn tasks_direct_first_capability_case() {
    use std::sync::atomic::AtomicUsize;
    use unica_execution_contract::invocation::{InvocationStatus, TaskId};

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
    use std::sync::atomic::AtomicUsize;
    use unica_execution_contract::invocation::{InvocationStatus, TaskId};

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
    use unica_execution_contract::invocation::{InvocationStatus, TaskId};

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
    use unica_execution_contract::invocation::{InvocationStatus, TaskId};

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
    use unica_execution_contract::invocation::{InvocationStatus, TaskId};

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
    use unica_execution_contract::invocation::{InvocationStatus, TaskId};

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
    use std::sync::atomic::AtomicUsize;
    use unica_execution_contract::invocation::{InvocationStatus, TaskId};
    use {
        unica_execution_contract::result_size::MAX_CANONICAL_RESULT_BYTES,
        unica_execution_contract::result_size::MAX_TASK_RECORD_ENVELOPE_BYTES,
    };

    let near = unica_execution_contract::invocation::DomainResult::success(
        "x".repeat(MAX_CANONICAL_RESULT_BYTES - 4_096),
    );
    let over = unica_execution_contract::invocation::DomainResult::success(
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
    use std::str::FromStr;
    use std::sync::atomic::AtomicUsize;
    use unica_execution_contract::invocation::{InvocationStatus, TaskId};

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

/// rule: INV.WIRE.LOST-SUBMIT-TASK-ID
#[tokio::test]
async fn a_lost_submit_is_a_task_for_native_clients_and_a_task_receipt_for_others() {
    use unica_execution_contract::invocation::{InvocationStatus, TaskId};

    let task_id = TaskId::new();
    let lost_call = move || -> Arc<CanonicalCallHandler> {
        Arc::new(move |_, _, _, _, _| {
            Ok(CanonicalCallOutcome::Task(canonical_snapshot(
                task_id,
                InvocationStatus::Queued,
                None,
            )))
        })
    };
    for (meta, native) in [(modern_tasks_meta(), true), (modern_meta(), false)] {
        let get: Arc<CanonicalTaskHandler> =
            Arc::new(move |_, _| Ok(canonical_snapshot(task_id, InvocationStatus::Queued, None)));
        let server = UnicaServer::with_canonical_v13_tasks(lost_call(), Arc::clone(&get), get);
        let (mut client, _) = spawn_unica_server(server);
        client
            .send(json!({
                "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": {"name": "unica.search", "arguments": {}, "_meta": meta}
            }))
            .await;
        let answer = client.receive().await;
        assert!(answer.get("error").is_none(), "{answer}");
        if native {
            assert_eq!(answer["result"]["resultType"], "task", "{answer}");
            assert_eq!(answer["result"]["taskId"], task_id.to_string(), "{answer}");
        } else {
            assert_ne!(answer["result"]["isError"], true, "{answer}");
            let receipt = &answer["result"]["structuredContent"];
            assert_eq!(
                receipt["data"]["task"]["taskId"],
                task_id.to_string(),
                "{answer}"
            );
            assert_eq!(receipt["data"]["task"]["status"], "queued", "{answer}");
            assert_eq!(receipt["next"][0]["tool"], "unica.task.result", "{answer}");
            assert_eq!(
                receipt["next"][0]["args"]["taskId"],
                task_id.to_string(),
                "{answer}"
            );
        }
        client.shutdown().await;
    }
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

/// rule: INV.APP.DAEMON-RESULT-SIZE
#[tokio::test]
async fn tasks_projection_bounds_near_limit_wire_and_rejects_over_limit() {
    tasks_projection_keeps_near_limit_wire_bounded_and_rejects_over_limit_case().await;
}

#[tokio::test]
async fn tasks_hooks_preserve_closed_unknown_expired_invalid_and_update_semantics() {
    tasks_hooks_closed_errors_case().await;
}

/// rule: CTR.WIRE.NATIVE-TASK-PROJECTION
/// rule: INV.WIRE.NATIVE-TASK-CAPABILITY
#[tokio::test]
async fn native_task_projection_contract_is_capability_gated_durable_and_replay_free() {
    tasks_direct_first_capability_case().await;
    legacy_initialized_session_cannot_escalate_tasks_per_request_case().await;
    modern_initialized_session_retains_native_tasks_case().await;
    native_task_methods_preserve_one_frontend_transport_cutoff_case().await;
    tasks_direct_and_completed_get_case().await;
    tasks_projection_rejects_reverse_timestamps_on_wire_case().await;
    tasks_projection_keeps_near_limit_wire_bounded_and_rejects_over_limit_case().await;
    tasks_hooks_closed_errors_case().await;
}

/// rule: CTR.WIRE.NATIVE-TASK-PROJECTION
/// rule: INV.RUNTIME.RUNNER-MUTATION-CANCELLATION
#[tokio::test]
async fn native_task_get_reports_late_cancel_request_without_claiming_cancellation() {
    use unica_execution_contract::invocation::{InvocationStatus, TaskId};

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
    let cancel: Arc<CanonicalTaskHandler> = Arc::new(move |_, _| Ok(requested_working(task_id)));
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

/// rule: INV.WIRE.VERSION-TIERS
#[tokio::test]
async fn legacy_offer_2025_11_25_is_echoed() {
    let (mut client, _) = spawn_unica_server(canonical_direct_server());
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

/// rule: INV.WIRE.VERSION-TIERS
#[tokio::test]
async fn legacy_unknown_offer_falls_back_to_pinned_version() {
    // The fallback is pinned to 2025-11-25 explicitly; an SDK bump that
    // moves `ProtocolVersion::LATEST` must not move this answer.
    let (mut client, _) = spawn_unica_server(canonical_direct_server());
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
    let (mut client, _) = spawn_unica_server(canonical_direct_server());
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

/// rule: INV.WIRE.TOOL-LIST-COMPATIBILITY
#[tokio::test]
async fn modern_meta_inside_legacy_session_keeps_the_session_model() {
    // Метаданные одного запроса не повышают полномочия сессии: внутри
    // сессии `initialize` современный `_meta` меняет форму ответа, но
    // страницы выдаются только сессии 2026-07-28. Каталог длиннее
    // страницы, чтобы целый ответ отличался от первой страницы.
    let (mut client, _) = spawn_unica_server(synthetic_catalog_server());
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
    assert_eq!(
        modern_shaped["result"]["tools"].as_array().unwrap().len(),
        synthetic_tool_catalog().len(),
        "{modern_shaped}"
    );
    assert!(
        modern_shaped["result"].get("nextCursor").is_none(),
        "{modern_shaped}"
    );
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

/// rule: INV.WIRE.TOOL-LIST-COMPATIBILITY
#[tokio::test]
async fn tools_list_rejects_any_presented_cursor() {
    let (mut client, _) = spawn_unica_server(canonical_direct_server());
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

/// rule: INV.WIRE.SURFACE-RELEASE-ROUTING
#[tokio::test]
async fn a_name_outside_the_canonical_profile_is_refused_without_dispatch() {
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&calls);
    let handler: Arc<CanonicalCallHandler> = Arc::new(move |_, _, _, _, _| {
        counted.fetch_add(1, Ordering::SeqCst);
        direct_outcome(canonical_result("dispatched"))
    });
    let (mut client, _) = spawn_unica_server(UnicaServer::with_canonical_v13(handler));
    client.initialize().await;
    // A retired v0.12 name and a name no profile ever had.
    for (id, name) in [(1, "unica.meta.add"), (2, "unica.nope")] {
        client
            .send(json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": "tools/call",
                "params": { "name": name, "arguments": {} }
            }))
            .await;
        let response = client.receive().await;
        assert_eq!(response["id"], id, "{response}");
        assert_eq!(response["error"]["code"], -32602, "{name}: {response}");
        assert_eq!(
            response["error"]["message"], "tool is not in the canonical v0.13 profile",
            "{name}: {response}"
        );
    }
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "a refused name must not reach the canonical handler"
    );
    client.shutdown().await;
}

/// rule: INV.WIRE.VERSION-TIERS
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

/// rule: INV.WIRE.VERSION-TIERS
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

/// rule: INV.WIRE.VERSION-TIERS
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

/// rule: INV.WIRE.VERSION-TIERS
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

/// rule: INV.WIRE.VERSION-TIERS
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

/// rule: INV.WIRE.TOOL-LIST-COMPATIBILITY
#[tokio::test]
async fn modern_direct_first_tools_list_pages_through_the_full_registry() {
    // Modern peers page the registry (25 per page, offset cursors);
    // walking every page must reproduce the complete surface exactly.
    let (mut client, _) = spawn_unica_server(synthetic_catalog_server());
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
    let registry_size = synthetic_tool_catalog().len();
    assert!(
        registry_size > 2 * TOOLS_PAGE_SIZE,
        "the catalog spans pages"
    );
    assert_eq!(names.len(), registry_size);
    let unique: HashSet<&String> = names.iter().collect();
    assert_eq!(unique.len(), registry_size, "pages must not overlap");
    client.shutdown().await;
}

/// rule: INV.WIRE.TOOL-LIST-COMPATIBILITY
#[tokio::test]
async fn modern_tools_list_rejects_a_cursor_the_server_never_issued() {
    // Каталог длиннее страницы: сервер выдаёт настоящие курсоры, и
    // отказ отличает невыданное смещение от любого предъявленного.
    let (mut client, _) = spawn_unica_server(synthetic_catalog_server());
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

/// rule: INV.WIRE.VERSION-TIERS
#[tokio::test]
async fn modern_unknown_version_direct_first_gets_unsupported_error() {
    let (mut client, _) = spawn_unica_server(canonical_direct_server());
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

/// rule: INV.WIRE.VERSION-TIERS
#[tokio::test]
async fn modern_partial_meta_opener_is_rejected_before_serving() {
    // A direct-first request with an incomplete reserved set is not a
    // silent legacy downgrade: admission refuses the connection.
    for method in ["tools/list", "server/discover"] {
        let (client_io, server_io) = tokio::io::duplex(4 * 1024 * 1024);
        let server = canonical_direct_server();
        let handle = tokio::spawn(async move {
            let (read, write) = tokio::io::split(server_io);
            let transport = rmcp::transport::async_rw::AsyncRwTransport::new_server(read, write);
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
        let snapshot = unica_code_intelligence::model::SearchProgressSnapshot {
            schema_version: 1,
            elapsed_ms: 5,
            deadline_ms: 300_000,
            next_update_within_ms: 2_000,
            providers: vec![unica_code_intelligence::model::SearchProviderProgress {
                identity: unica_code_intelligence::model::ProviderId::GitGrep.identity(),
                state: unica_code_intelligence::model::SearchProviderState::Running,
                phase: unica_code_intelligence::model::SearchProviderPhase::Searching,
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
                unica_code_intelligence::model::SearchProviderPhase::Preparing,
                "reconcilingSources",
            ),
            (
                6,
                unica_code_intelligence::model::SearchProviderPhase::Searching,
                "executingQuery",
            ),
        ] {
            let snapshot = unica_code_intelligence::model::SearchProgressSnapshot {
                schema_version: 1,
                elapsed_ms,
                deadline_ms: 300_000,
                next_update_within_ms: 2_000,
                providers: vec![unica_code_intelligence::model::SearchProviderProgress {
                    identity: unica_code_intelligence::model::ProviderId::SemanticFixture
                        .identity(),
                    state: unica_code_intelligence::model::SearchProviderState::Running,
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
        cache_access: unica_workspace::cache::CacheAccess::default(),
        handler: crate::application::ToolHandler::Metadata {
            operation: unica_metadata::request::MetadataOperation::Info,
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

#[tokio::test]
async fn ping_stays_responsive_and_cancellation_reaches_the_tool() {
    let cancellation_seen = Arc::new(AtomicBool::new(false));
    let seen = Arc::clone(&cancellation_seen);
    let handler: Arc<CanonicalCallHandler> = Arc::new(move |_, _, _, _, cancellation| {
        let cancellation = cancellation.token();
        let give_up = Instant::now() + 4 * TEST_STEP;
        while !cancellation.is_cancelled() {
            if Instant::now() > give_up {
                return Err(ErrorData::internal_error(
                    "test handler was never cancelled",
                    None,
                ));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        seen.store(true, Ordering::SeqCst);
        direct_outcome(canonical_result("unreachable success"))
    });
    let (mut client, _) = spawn_unica_server(UnicaServer::with_canonical_v13(handler));
    client.initialize().await;

    client
        .send(json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "tools/call",
            "params": { "name": "unica.search", "arguments": {} }
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
    let drained =
        tokio::task::spawn_blocking(move || drain_mcp_shutdown(&in_flight, EOF_CANCELLATION_GRACE))
            .await
            .unwrap();
    assert!(drained, "cancelled call did not finish within the grace");
    assert!(cancellation_seen.load(Ordering::SeqCst));
}

/// rule: INV.WIRE.BOUNDED-ADMISSION
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
    unica_code_intelligence::coordinator::track_code_search_worker_for_test(std::thread::spawn(
        || std::thread::sleep(Duration::from_millis(50)),
    ));

    assert!(
        unica_code_intelligence::coordinator::drain_code_search_workers(EOF_CANCELLATION_GRACE),
        "tracked code-search worker outlived the EOF cleanup grace"
    );
}

#[test]
fn eof_cleanup_drains_noncooperative_diagnostic_worker_within_the_same_grace() {
    // This worker deliberately has no cancellation token. It models a
    // provider that ignored cancellation after its tool call returned.
    unica_diagnostics::coordinator::track_diagnostic_worker_for_test(std::thread::spawn(|| {
        std::thread::sleep(Duration::from_millis(50))
    }));

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

/// rule: INV.WIRE.BOUNDED-ADMISSION
#[tokio::test]
async fn dispatcher_executes_all_calls_past_the_former_quota() {
    let release = Arc::new(AtomicBool::new(false));
    let gate = Arc::clone(&release);
    let entered = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let worker_entered = Arc::clone(&entered);
    let handler: Arc<CanonicalCallHandler> = Arc::new(move |_, _, _, _, _| {
        worker_entered.fetch_add(1, Ordering::SeqCst);
        let give_up = Instant::now() + 4 * TEST_STEP;
        while !gate.load(Ordering::SeqCst) {
            if Instant::now() > give_up {
                return Err(ErrorData::internal_error(
                    "test handler was never released",
                    None,
                ));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let mut released = canonical_result("released");
        released.ok = true;
        direct_outcome(released)
    });
    let (mut client, in_flight) = spawn_unica_server(UnicaServer::with_canonical_v13(handler));
    client.initialize().await;
    let expected: std::collections::BTreeSet<_> =
        (0..33).map(|id| format!("blocked-{id}")).collect();
    for id in &expected {
        client
            .send(json!({
                "jsonrpc": "2.0", "id": id, "method": "tools/call",
                "params": { "name": "unica.search", "arguments": {} }
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
        // A canonical Direct success carries its JSON in structuredContent.
        assert_eq!(
            response["result"]["structuredContent"]["summary"], "released",
            "{response}"
        );
    }
    assert_eq!(in_flight.running(), 0);
}

/// rule: CTR.WIRE.COMPATIBILITY-TASK-TOOLS
#[test]
fn compatibility_result_keeps_original_cutoff_after_frontend_admission_delay() {
    let task_id = unica_execution_contract::invocation::TaskId::new();
    let observed = Arc::new(Mutex::new(None));
    let captured = Arc::clone(&observed);
    let wait: Arc<CanonicalTaskWaitHandler> = Arc::new(move |_, wait_ms, deadline, _| {
        *captured.lock().unwrap() = Some(daemon_wait_cutoff(wait_ms, deadline));
        Ok(canonical_snapshot(
            task_id,
            unica_execution_contract::invocation::InvocationStatus::Working,
            None,
        ))
    });
    let router = CanonicalDaemonRouter {
        call: Arc::new(|_, _, _, _, _| panic!("observation must not submit")),
        get: Arc::new(|_, _| panic!("result must use wait")),
        wait,
        cancel: Arc::new(|_, _| panic!("observation must not cancel")),
    };
    let received = Instant::now() - Duration::from_millis(100);
    let deadline = FrontendInvocationDeadline::new(received, None);
    let arguments = json!({"taskId":task_id.to_string(),"waitMs":7000});
    let request = crate::application::v13::task_tools::parse_task_tool_call(
        "unica.task.result",
        arguments.as_object().unwrap(),
    )
    .unwrap();
    let result = execute_compatibility_task_tool(
        &router,
        request,
        deadline,
        CanonicalCancellation::default(),
    );
    assert!(result.result().ok, "{:?}", result.result());
    assert_eq!(*observed.lock().unwrap(), Some(received + Duration::from_millis(7125)), "admission time must shrink daemon wait without shortening the original transport cutoff twice");
}
/// rule: CTR.WIRE.COMPATIBILITY-TASK-TOOLS
#[test]
fn compatibility_final_projection_obeys_cutoff_and_same_result_remains_recoverable() {
    let result = CompatibilityTaskResult::Snapshot(canonical_result(
        "saved terminal; password=private-observation-marker",
    ));
    let received = Instant::now();
    let timing = TaskObservationTimer::since(received, received + Duration::from_millis(225));
    let budget = unica_bootstrap::ResponseBudget::default();
    let direct = crate::task_projection::call_tool_result(result.result(), budget).unwrap();
    let before = compatibility_result_before(&result, budget, received, timing, || {
        received + Duration::from_millis(224)
    })
    .unwrap();
    assert_eq!(
        serde_json::to_value(before).unwrap(),
        serde_json::to_value(&direct).unwrap()
    );
    LOGGED_TASK_OBSERVATIONS.with(|logged| logged.borrow_mut().clear());
    let late = compatibility_result_before(&result, budget, received, timing, || {
        received + Duration::from_millis(226)
    })
    .unwrap();
    let value = serde_json::to_value(late).unwrap();
    assert_eq!(
        value["structuredContent"]["diagnostics"][0]["code"],
        "task_transport_failed"
    );
    assert!(!value.to_string().contains("taskObservation"), "{value}");
    let logged = LOGGED_TASK_OBSERVATIONS.with(|logged| logged.borrow().clone());
    assert_eq!(logged.len(), 1, "{logged:?}");
    let (observation, since_received_ms) = logged[0];
    assert_eq!(observation.stage, TaskObservationStage::ProjectResult);
    assert_eq!(observation.elapsed_ms, 226);
    assert!(observation.cutoff_expired);
    assert_eq!(since_received_ms, 226);
    assert!(!value.to_string().contains("private-observation-marker"));
    let recovered = compatibility_result_before(
        &result,
        budget,
        Instant::now(),
        TaskObservationTimer::start(Instant::now() + Duration::from_secs(1)),
        Instant::now,
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(recovered).unwrap(),
        serde_json::to_value(direct).unwrap()
    );
}

/// rule: CTR.WIRE.COMPATIBILITY-TASK-TOOLS
#[test]
fn native_task_transport_failure_logs_the_observation_and_answers_only_the_refusal() {
    let observation = TaskObservationTimer::start(Instant::now()).failed(
        TaskObservationStage::WriteRequest,
        Instant::now(),
        None,
    );
    LOGGED_TASK_OBSERVATIONS.with(|logged| logged.borrow_mut().clear());
    let received_at = Instant::now() - Duration::from_millis(500);
    let error =
        project_task_exchange_error(V5TaskExchangeError::Transport(observation), received_at);
    assert_eq!(error.message, "task_transport_failed");
    assert_eq!(error.data, Some(json!({"code": "task_transport_failed"})));
    let logged = LOGGED_TASK_OBSERVATIONS.with(|logged| logged.borrow().clone());
    assert_eq!(logged.len(), 1, "{logged:?}");
    assert_eq!(logged[0].0, observation);
    assert!(
        logged[0].1 >= 500,
        "sinceReceivedMs counts from the request, not from the stage timer: {logged:?}"
    );
}

/// rule: CTR.WIRE.COMPATIBILITY-TASK-TOOLS
#[test]
fn compatibility_final_projection_preserves_exchange_failure_stage_and_generation() {
    let task_id = unica_execution_contract::invocation::TaskId::new();
    let start = Instant::now();
    let timing = TaskObservationTimer::since(start, start + Duration::from_millis(225));
    let record = unica_daemon_protocol::protocol_v5::V5EndpointRecord::new(
        unica_daemon_protocol::identity::CoreIdentity::production_v5(),
        41001,
    )
    .unwrap();
    let observation = timing.failed(
        TaskObservationStage::ReadResponse,
        start + Duration::from_millis(226),
        Some(&record),
    );
    let router = CanonicalDaemonRouter {
        call: Arc::new(|_, _, _, _, _| panic!("observation must not submit")),
        get: Arc::new(|_, _| panic!("result uses wait")),
        wait: Arc::new(move |_, _, _, _| Err(V5TaskExchangeError::Transport(observation))),
        cancel: Arc::new(|_, _| panic!("observation must not cancel")),
    };
    let args = json!({"taskId":task_id.to_string(),"waitMs":100});
    let request = crate::application::v13::task_tools::parse_task_tool_call(
        "unica.task.result",
        args.as_object().unwrap(),
    )
    .unwrap();
    LOGGED_TASK_OBSERVATIONS.with(|logged| logged.borrow_mut().clear());
    let result = execute_compatibility_task_tool(
        &router,
        request,
        FrontendInvocationDeadline::new(start, None),
        CanonicalCancellation::default(),
    );
    let logged = LOGGED_TASK_OBSERVATIONS.with(|logged| logged.borrow().clone());
    assert_eq!(
        logged.iter().map(|(logged, _)| *logged).collect::<Vec<_>>(),
        vec![observation],
        "the exchange failure keeps its stage and generation in the log"
    );
    let projected = compatibility_result_before(
        &result,
        unica_bootstrap::ResponseBudget::default(),
        start,
        timing,
        || start + Duration::from_millis(230),
    )
    .unwrap();
    let public = serde_json::to_value(projected).unwrap();
    assert!(!public.to_string().contains("taskObservation"), "{public}");
    assert_eq!(
        LOGGED_TASK_OBSERVATIONS.with(|logged| logged.borrow().len()),
        1,
        "a late projection of an established refusal is not observed again"
    );
    assert_eq!(
        public["structuredContent"]["diagnostics"][0]["code"],
        "task_transport_failed"
    );
    assert!(!public.to_string().contains(record.token()));
    assert!(!public.to_string().contains(record.instance_id()));
}
/// rule: CTR.WIRE.COMPATIBILITY-TASK-TOOLS
#[tokio::test]
async fn compatibility_recovers_same_task_after_partial_malformed_and_late_frames() {
    use std::io::Write as _;
    for defect in ["partial", "malformed", "late"] {
        let task_id = unica_execution_contract::invocation::TaskId::new();
        let complete = daemon_frame_bytes(&daemon_task_frame_value(task_id));
        let malformed = malformed_daemon_task_frames(task_id)
            .into_iter()
            .next()
            .unwrap()
            .1;
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&calls);
        let fake = FakeDaemon::start_with_raw_script(Box::new(move |_, request, writer| {
            assert!(
                matches!(request,V5ClientRequest::WaitTask { task_id: id, .. } if *id == task_id)
            );
            if observed.fetch_add(1, Ordering::SeqCst) == 0 {
                let frame = match defect {
                    "partial" => &complete[..complete.len() / 2],
                    "malformed" => malformed.as_slice(),
                    "late" => {
                        // `waitMs` bounds only the daemon wait; a frame is
                        // late once it crosses the frontend transport cutoff.
                        std::thread::sleep(
                            unica_execution::invocation::INVOCATION_HANDOFF_WINDOW
                                + unica_execution::invocation::RESPONSE_SERIALIZATION_MARGIN
                                + Duration::from_millis(300),
                        );
                        complete.as_slice()
                    }
                    _ => unreachable!(),
                };
                let _ = writer.write_all(frame);
                let _ = writer.flush();
                false
            } else {
                writer.write_all(&complete).unwrap();
                writer.flush().unwrap();
                true
            }
        }));
        let (mut client, _) = spawn_unica_server(UnicaServer::with_canonical_daemon(
            fake.owner(),
            "/workspace".into(),
        ));
        for (id, wait_ms) in [(1, 100), (2, 2000)] {
            // The recovery read gets room enough not to race the scripted
            // listener under load.
            client.send(json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":"unica.task.result","arguments":{"taskId":task_id.to_string(),"waitMs":wait_ms},"_meta":modern_meta()}})).await;
            let response = client.receive().await;
            let value = &response["result"]["structuredContent"];
            if id == 1 {
                assert_eq!(
                    value["diagnostics"][0]["code"],
                    if defect == "malformed" {
                        "task_protocol_failed"
                    } else {
                        "task_transport_failed"
                    },
                    "{defect}: {response}"
                );
                assert!(
                    !response.to_string().contains(RESULT_MARKER),
                    "rejected frame leaked"
                );
                // The observation goes to the frontend log, not to the agent.
                assert!(
                    !response.to_string().contains("taskObservation"),
                    "{response}"
                );
                // A scripted listener finishes the late frame before accepting
                // another peer; the second call receives its own original budget.
                if defect == "late" {
                    tokio::time::sleep(Duration::from_millis(150)).await;
                }
            } else {
                assert_eq!(value["ok"], true, "{defect}: {response}");
                assert!(
                    response.to_string().contains(RESULT_MARKER),
                    "same durable result was not recovered"
                );
            }
        }
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(
            fake.submissions(),
            0,
            "reads of a pre-existing Task never submit the subject"
        );
        client.shutdown().await;
    }
}
