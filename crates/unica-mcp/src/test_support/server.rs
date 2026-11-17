use crate::canonical_cancellation::CanonicalCancellation;
use crate::daemon_router::*;
use crate::*;
use rmcp::model::ErrorData;
use serde_json::Value;
use std::sync::Arc;
use std::time::{Duration, Instant};
use unica_daemon_protocol::client_v5::V5TaskExchangeError;
use unica_daemon_protocol::protocol_v5::V5DaemonErrorCode;
use unica_execution::receipt_runtime::response::V5DaemonTaskSnapshot;

use crate::test_support::{FakeDaemon, Step};
use serde_json::json;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::time::timeout;
use unica_daemon_protocol::protocol_v5::V5ClientRequest;

// Guards a wait that ends on its event. Under a fully loaded runner the
// event can come late; the guard only stops a hang (#2063).
pub const TEST_STEP: Duration = Duration::from_secs(30);

pub struct McpClient {
    pub writer: tokio::io::WriteHalf<tokio::io::DuplexStream>,
    pub reader: tokio::io::Lines<BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>>,
    pub server: tokio::task::JoinHandle<()>,
}

impl McpClient {
    pub async fn send(&mut self, message: Value) {
        let mut line = message.to_string();
        line.push('\n');
        self.writer.write_all(line.as_bytes()).await.unwrap();
        self.writer.flush().await.unwrap();
    }

    pub async fn receive(&mut self) -> Value {
        let line = self.receive_raw().await;
        serde_json::from_str(&line).expect("MCP server emitted invalid JSON")
    }

    pub async fn receive_raw(&mut self) -> String {
        let line = timeout(TEST_STEP, self.reader.next_line())
            .await
            .expect("timed out waiting for MCP response")
            .expect("MCP transport failed")
            .expect("MCP server closed the stream before responding");
        line
    }

    pub async fn initialize(&mut self) -> Value {
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

    pub async fn shutdown(mut self) {
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

pub fn spawn_server(handler: Arc<ToolCallHandler>) -> (McpClient, Arc<InFlightRegistry>) {
    spawn_unica_server(UnicaServer::legacy_for_test(handler))
}

pub fn spawn_unica_server(server: UnicaServer) -> (McpClient, Arc<InFlightRegistry>) {
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
            Err(error) if matches!(error.as_ref(), ServerInitializeError::ConnectionClosed(_)) => {}
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

pub fn modern_meta() -> Value {
    json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientCapabilities": {}
    })
}

pub fn modern_tasks_meta() -> Value {
    json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientCapabilities": {
            "extensions": {"io.modelcontextprotocol/tasks": {}}
        }
    })
}

pub fn canonical_result(summary: &str) -> unica_execution_contract::invocation::DomainResult {
    unica_execution_contract::invocation::DomainResult {
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
        source_text: None,
        source_text_read_hint: None,
        human_choice: None,
    }
}

/// A durable v5 snapshot for the handler fakes: the same identity and
/// timing every test expects on the wire, one closed variant per status.
pub fn canonical_snapshot(
    task_id: unica_execution_contract::invocation::TaskId,
    status: unica_execution_contract::invocation::InvocationStatus,
    result: Option<unica_execution_contract::invocation::DomainResult>,
) -> V5DaemonTaskSnapshot {
    canonical_snapshot_at(
        task_id,
        status,
        result,
        1_777_012_345_678,
        1_777_012_346_789,
    )
}

pub fn canonical_snapshot_at(
    task_id: unica_execution_contract::invocation::TaskId,
    status: unica_execution_contract::invocation::InvocationStatus,
    result: Option<unica_execution_contract::invocation::DomainResult>,
    created_at_epoch_ms: u64,
    updated_at_epoch_ms: u64,
) -> V5DaemonTaskSnapshot {
    use unica_execution_contract::invocation::InvocationStatus;

    let invocation_id = unica_execution_contract::invocation::InvocationId::new();
    let receipt_key_digest: unica_execution::receipt_ledger::ReceiptKeyDigest =
        "07".repeat(32).parse().unwrap();
    let terminal_digest: unica_execution::receipt_ledger::TerminalDigest =
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
            awaits_interrupted_work: false,
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
            reason: unica_execution::invocation_store_v5::V5SafeFailureReason::InvocationFailed,
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
pub fn direct_outcome(
    result: unica_execution_contract::invocation::DomainResult,
) -> Result<CanonicalCallOutcome, ErrorData> {
    direct_outcome_within(result, unica_bootstrap::ResponseBudget::default())
}

/// Прямой ответ в бюджете клиента, как его строит роутер демона.
pub fn direct_outcome_within(
    result: unica_execution_contract::invocation::DomainResult,
    budget: unica_bootstrap::ResponseBudget,
) -> Result<CanonicalCallOutcome, ErrorData> {
    crate::task_projection::call_tool_result(&result, budget)
        .map(CanonicalCallOutcome::Direct)
        .map_err(crate::task_projection::projection_error)
}

pub fn canonical_profile_server() -> UnicaServer {
    let task_id = unica_execution_contract::invocation::TaskId::new();
    let call: Arc<CanonicalCallHandler> = Arc::new(move |_, _, _, _, _| {
        Ok(CanonicalCallOutcome::Task(canonical_snapshot(
            task_id,
            unica_execution_contract::invocation::InvocationStatus::Working,
            None,
        )))
    });
    let get: Arc<CanonicalTaskHandler> = Arc::new(move |_, _| {
        Ok(canonical_snapshot(
            task_id,
            unica_execution_contract::invocation::InvocationStatus::Working,
            None,
        ))
    });
    UnicaServer::with_canonical_v13_tasks(call, Arc::clone(&get), get)
}

pub async fn listed_tool_names(
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

pub fn assert_v13_profile_names(names: &[String], native_tasks: bool) {
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

pub async fn surface_profiles_case() {
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

    // 2026-07-28 has no initialize handshake: offering it there with the
    // Tasks capability yields a 2025-11-25 session on the compatibility
    // profile. Native tasks belong to direct 2026-07-28 requests below.
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
    assert_eq!(
        modern_native.receive().await["result"]["protocolVersion"],
        "2025-11-25"
    );
    assert_v13_profile_names(&listed_tool_names(&mut modern_native, 1, None).await, false);
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
    assert_eq!(
        modern_compat.receive().await["result"]["protocolVersion"],
        "2025-11-25"
    );
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

pub async fn compatibility_receipts_case() {
    use std::sync::atomic::AtomicUsize;
    use unica_execution_contract::invocation::{InvocationStatus, TaskId};

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
            cancelled["result"]["structuredContent"]["diagnostics"][0]["code"], "task_cancelled",
            "{cancelled}"
        );
    }
    assert_eq!(executions.load(Ordering::SeqCst), 1);
    assert_eq!(gets.load(Ordering::SeqCst), 1);
    assert_eq!(cancellations.load(Ordering::SeqCst), 2);
    client.shutdown().await;
}

pub fn compatibility_wait_budget_for_test(
    requested: u64,
    deadline: FrontendInvocationDeadline,
    now: Instant,
) -> u64 {
    crate::daemon_router::bounded_wait_ms(requested, daemon_wait_cutoff(requested, deadline), now)
}

pub fn compatibility_wait_budget_case() {
    let received = Instant::now();
    let deadline = FrontendInvocationDeadline::new(received, None);
    assert_eq!(
        compatibility_wait_budget_for_test(0, deadline, received),
        0,
        "zero is an immediate probe"
    );
    assert_eq!(
        compatibility_wait_budget_for_test(7_000, deadline, received),
        7_000
    );
    assert_eq!(
        compatibility_wait_budget_for_test(
            7_000,
            deadline,
            received + Duration::from_millis(6_999),
        ),
        1,
        "elapsed frontend time is never replenished"
    );
    assert_eq!(
        compatibility_wait_budget_for_test(7_000, deadline, received + Duration::from_secs(7),),
        0
    );
    assert_eq!(
        daemon_wait_cutoff(0, deadline),
        received + Duration::from_millis(125)
    );
    assert_eq!(
        daemon_wait_cutoff(1, deadline),
        received + Duration::from_millis(126)
    );
    assert_eq!(
        daemon_wait_cutoff(7_000, deadline),
        received + Duration::from_millis(7_125)
    );
    assert_eq!(
        daemon_wait_cutoff(7_000, deadline),
        received + Duration::from_millis(7_125),
        "elapsed frontend time is not replenished by the compatibility wait"
    );
    assert_eq!(
        daemon_wait_cutoff(
            7_000,
            FrontendInvocationDeadline::new(received, Some(Duration::from_millis(80)))
        ),
        received + Duration::from_millis(80),
        "an earlier host deadline is stronger than waitMs plus response margin"
    );
}

pub async fn compatibility_terminal_result_case() {
    use std::sync::atomic::AtomicUsize;
    use unica_execution_contract::invocation::{InvocationStatus, TaskId};

    // Третий случай — ответ с исходным текстом BSL: прямой ответ и итог
    // задания одинаково несут конверт и сырой текст блоками `content`.
    let source = "// Комментарий \"метода\".\r\nПроцедура А()\n\tВозврат;\nКонецПроцедуры\n";
    // Четвёртый — текст сверх бюджета клиента по умолчанию: прямой ответ
    // и итог задания одинаково отказывают.
    let oversized = format!("// {}\n", "a".repeat(20_000));
    // Пятый — тот же текст клиенту протокола 2026-07-28 без `initialize`,
    // назвавшему себя в `_meta` запроса известным хостом: его бюджет
    // больше, и текст проходит и прямым ответом, и итогом задания.
    let (known_client, known_budget) = unica_bootstrap::known_client_budgets()[0];
    assert!(known_budget > oversized.len() + 2048);
    for (is_error, source_text, named) in [
        (false, None, false),
        (true, None, false),
        (false, Some(source), false),
        (false, Some(oversized.as_str()), false),
        (false, Some(oversized.as_str()), true),
    ] {
        // Остальные случаи называют неизвестного клиента: бюджет по
        // умолчанию тогда не зависит от окружения запуска прогона.
        let mut meta = modern_meta();
        meta["io.modelcontextprotocol/clientInfo"] = json!({
            "name": if named { known_client } else { "unica-tests" },
            "version": "1"
        });
        let over_budget = !named && source_text.is_some_and(|text| text.len() > 16_384);
        let task_id = TaskId::new();
        let mut subject = canonical_result("same terminal subject result");
        subject.ok = !is_error;
        subject.source_text = source_text.map(str::to_string);
        subject.diagnostics = vec![json!({
            "code": "provider_unavailable", "outcome": "needsHuman",
            "message": "v8-runner is unavailable", "remediation": "prefetch the engine"
        })];
        let executions = Arc::new(AtomicUsize::new(0));
        let execution_observed = Arc::clone(&executions);
        let direct_subject = subject.clone();
        let call: Arc<CanonicalCallHandler> = Arc::new(move |_, arguments, host, _, _| {
            execution_observed.fetch_add(1, Ordering::SeqCst);
            if arguments.get("direct").and_then(Value::as_bool) == Some(true) {
                direct_outcome_within(direct_subject.clone(), host.response_budget)
            } else {
                Ok(CanonicalCallOutcome::Task(canonical_snapshot(
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
                    "name":"unica.check", "arguments":{"direct":true}, "_meta":meta.clone()
                }
            }))
            .await;
        let direct = client.receive().await;
        assert_eq!(direct["result"]["isError"], is_error || over_budget);
        if over_budget {
            assert_eq!(
                direct["result"]["structuredContent"]["diagnostics"][0]["code"], "result_too_large",
                "{direct}"
            );
        } else if is_error {
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
        } else if let Some(source) = source_text {
            assert!(
                direct["result"].get("structuredContent").is_none(),
                "{direct}"
            );
            let blocks = direct["result"]["content"].as_array().unwrap();
            assert_eq!(blocks.len(), 2, "{direct}");
            let envelope: Value =
                serde_json::from_str(blocks[0]["text"].as_str().unwrap()).unwrap();
            assert_eq!(envelope["summary"], "same terminal subject result");
            assert!(envelope.get("sourceText").is_none());
            assert_eq!(blocks[1]["text"].as_str().unwrap(), source);
        } else {
            assert_eq!(direct["result"]["content"], json!([]));
        }
        client
            .send(json!({
                "jsonrpc":"2.0", "id":2, "method":"tools/call",
                "params":{
                    "name":"unica.check", "arguments":{}, "_meta":meta.clone()
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
                    "_meta":meta.clone()
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
                    "_meta":meta.clone()
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

pub async fn compatibility_closed_errors_case() {
    use std::sync::atomic::AtomicUsize;
    use unica_execution_contract::invocation::{InvocationStatus, TaskId};

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

/// The fake answers every Task frame with a working snapshot, optionally
/// only after `response_delay`; the tests below spend the frontend budget
/// on connect, handshake and response on purpose.
pub fn working_task_fake(
    task_id: unica_execution_contract::invocation::TaskId,
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
            Step::Reply(unica_daemon_protocol::protocol_v5::V5ServerResponse::Task {
                snapshot: canonical_snapshot(
                    task_id,
                    unica_execution_contract::invocation::InvocationStatus::Working,
                    None,
                ),
            })
        }),
        handshake_delay,
    )
}

pub async fn compatibility_wait_single_deadline_case() {
    for requested_wait_ms in [0_u64, 1] {
        let task_id = unica_execution_contract::invocation::TaskId::new();
        let (observed, observations) = mpsc::channel();
        // Connect and handshake take longer than the requested wait, and the
        // daemon answers only after the 125 ms response margin has passed:
        // that spends the wait slice, not the transport cutoff.
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
                response["result"]["structuredContent"]["data"]["task"]["status"],
                "working",
                "connect and response past {requested_wait_ms}ms + 125ms must still answer the Task state: {response}"
            );
        let request = observations
            .recv_timeout(Duration::from_millis(500))
            .expect("the answered Task state proves the daemon saw the wait");
        assert_eq!(
            request,
            V5ClientRequest::WaitTask {
                task_id,
                wait_ms: 0
            },
            "connect time consumes the wait slice before the 125ms response margin"
        );
        mcp.shutdown().await;
    }
}

pub fn compatibility_wait_frontend_cutoff_is_not_rebased_case() {
    let task_id = unica_execution_contract::invocation::TaskId::new();
    let (observed, observations) = mpsc::channel();
    let fake = working_task_fake(task_id, Duration::ZERO, Duration::ZERO, observed);
    let router = canonical_daemon_router(fake.owner(), "/workspace".to_string());
    // The request was received past the whole frontend transport window and
    // only now reaches the router: a Duration rebase would open a fresh
    // window here, the absolute cutoff is already expired.
    let received = Instant::now()
        - unica_execution::invocation::INVOCATION_HANDOFF_WINDOW
        - unica_execution::invocation::RESPONSE_SERIALIZATION_MARGIN
        - Duration::from_secs(1);

    let outcome = (router.wait)(
        task_id,
        0,
        FrontendInvocationDeadline::new(received, None),
        CanonicalCancellation::default(),
    );

    let Err(V5TaskExchangeError::Transport(observation)) = outcome else {
        panic!("the operation must not rebase its cutoff after the injected pause");
    };
    assert_eq!(
        observation.stage,
        unica_daemon_protocol::client_v5::TaskObservationStage::AcquirePeer
    );
    assert!(observation.cutoff_expired);
    assert!(observation.endpoint_generation_hash.is_none());
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

/// A payload that arrives after the operation cutoff is never published,
/// valid or not, and the operation session is closed for reuse.
pub fn compatibility_wait_late_payload_case(valid_near_limit: bool) {
    use unica_daemon_protocol::protocol_v5::{V5ServerResponse, MAX_V5_RESPONSE_LINE_BYTES};
    use unica_execution_contract::result_size::MAX_CANONICAL_RESULT_BYTES;

    let task_id = unica_execution_contract::invocation::TaskId::new();
    let response_payload = if valid_near_limit {
        let snapshot = canonical_snapshot(
            task_id,
            unica_execution_contract::invocation::InvocationStatus::Completed,
            Some(unica_execution_contract::invocation::DomainResult::success(
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
        first.as_ref().err().map(|error| match error {
            V5TaskExchangeError::Transport(observation) => observation.cutoff_expired,
            _ => false,
        }),
        Some(true),
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

/// The wait the daemon is asked for once the frontend cutoff, the
/// handshake and the response margin have been subtracted. The frontend
/// deadline starts after the anchor session exists, so only the
/// operation's own connect and handshake spend it.
pub fn compatibility_wait_authenticated_long_and_host_cutoff_case(
    requested_wait_ms: u64,
    host_remaining: Option<Duration>,
    expected_daemon_wait_ms: std::ops::RangeInclusive<u64>,
    must_answer: bool,
) {
    let task_id = unica_execution_contract::invocation::TaskId::new();
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
        Err(V5TaskExchangeError::Transport(_)) if !must_answer => {}
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

pub async fn compatibility_hostile_status_payload_case() {
    use unica_execution_contract::invocation::{DomainResult, InvocationStatus, TaskId};

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
        let (mut client, _) = spawn_unica_server(UnicaServer::with_canonical_v13_task_handlers(
            call, get, wait, cancel,
        ));

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
            let diagnostic = &response["result"]["structuredContent"]["diagnostics"][0];
            if status == InvocationStatus::Failed {
                assert_eq!(diagnostic["failureReason"], "invocation_failed");
            } else {
                assert!(diagnostic.get("failureReason").is_none());
            }
        }
        client.shutdown().await;
    }
}
