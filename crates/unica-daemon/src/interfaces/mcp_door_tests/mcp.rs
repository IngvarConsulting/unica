#![cfg(test)]
use super::prelude::*;
use serde_json::json;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::time::timeout;

fn application_handler() -> Arc<ToolCallHandler> {
    let app = Arc::new(crate::composition::testing::application());
    Arc::new(move |name, arguments, cancellation, progress| {
        call_tool_result_observed(&app, name, arguments, cancellation, progress)
    })
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

/// A known-long service: every call hands off to a durable Task before
/// execution, which is what the compatibility receipts observe.
fn known_long_service() -> Arc<ScriptedService> {
    Arc::new(ScriptedService {
        delay: Duration::from_millis(50),
        known_long: true,
        outcome: Mutex::new(Some(Ok(
            unica_execution_contract::invocation::DomainResult::success(
                "durable compatibility result",
            ),
        ))),
        executions: AtomicUsize::new(0),
    })
}

async fn compatibility_daemon_restart_case() {
    use std::str::FromStr;
    use unica_execution_contract::invocation::TaskId;

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
async fn compatibility_adapter_reconnects_to_the_same_durable_task_after_daemon_restart() {
    compatibility_daemon_restart_case().await;
}

/// rule: CTR.WIRE.COMPATIBILITY-TASK-TOOLS
/// rule: CTR.WIRE.NATIVE-TASK-PROJECTION
#[tokio::test]
async fn v13_compatibility_task_tools_are_profile_gated_durable_and_replay_free() {
    surface_profiles_case().await;
    compatibility_receipts_case().await;
    compatibility_wait_budget_case();
    compatibility_wait_single_deadline_case().await;
    compatibility_wait_frontend_cutoff_is_not_rebased_case();
    compatibility_wait_late_payload_case(true);
    compatibility_wait_late_payload_case(false);
    compatibility_wait_authenticated_long_and_host_cutoff_case(7_000, None, 6_000..=6_940, true);
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

/// rule: INV.RUNTIME.RUNNER-MUTATION-CANCELLATION
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn public_task_cancel_and_result_preserve_a_started_infobase_create_receipt() {
    protected_create_cancel_case(false).await;
}

/// rule: INV.WIRE.COOPERATIVE-CANCELLATION
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn canonical_manual_cancellation_protected_create_preserves_external_receipt_without_replay()
{
    protected_create_cancel_case(true).await;
}

async fn protected_create_cancel_case(manual: bool) {
    use crate::infrastructure::daemon::server::actor_capacity_tests::{
        canonical_v13_service, install_cancellable_create_runner, LiveV5Daemon,
    };

    async fn terminal_result(client: &mut McpClient, task_id: &str, deadline: Instant) -> Value {
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
        flush.arm().await;
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
        let internal: unica_execution_contract::invocation::TaskId = task_id.parse().unwrap();
        let watchdog = Instant::now() + Duration::from_secs(3);
        loop {
            let snapshot = daemon.get(&owner, internal);
            if snapshot.cancel_requested() {
                assert_eq!(
                    snapshot.status(),
                    unica_execution_contract::invocation::InvocationStatus::Working
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
    let internal_task: unica_execution_contract::invocation::TaskId = task_id.parse().unwrap();
    assert!(matches!(
        daemon.get(&owner, internal_task),
        unica_execution::receipt_runtime::response::V5DaemonTaskSnapshot::Completed {
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

/// rule: INV.RUNTIME.RUNNER-MUTATION-CANCELLATION
#[tokio::test]
async fn public_native_cancel_waits_for_slow_job_attach_before_answering() {
    use crate::infrastructure::daemon::server::actor_capacity_tests::{
        canonical_v13_service, install_cancellable_create_runner, LiveV5Daemon,
    };
    use unica_process::test_support::JobAttachGateForTest;

    if !JobAttachGateForTest::supported() {
        unica_process::test_support::skip_without_host_capability(
            unica_process::test_support::HostCapability::WindowsJobObject,
            "Windows Job attachment exists only on Windows",
        );
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

    let target = crate::infrastructure::current_target_id().unwrap();
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

struct ManualDaemonOwner(Option<LiveDaemon>);

impl ManualDaemonOwner {
    fn start(service: Arc<ManualDaemonService>) -> Self {
        Self(Some(LiveDaemon::start(service)))
    }

    fn start_with_hooks(
        service: Arc<ManualDaemonService>,
        hooks: Arc<
            dyn unica_execution::receipt_runtime::hooks::V5RuntimeHooks<
                crate::infrastructure::daemon::server::CanonicalInvocationHost,
            >,
        >,
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
        timeout(TEST_STEP, async {
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
        unica_execution::receipt_runtime::ports::ExecutionClass,
        Box<unica_execution_contract::invocation::DomainResult>,
    > {
        use unica_execution::receipt_runtime::ports::{ExecutionClass, KnownLongReason};
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
        unica_execution_contract::invocation::DomainResult,
        unica_execution_contract::invocation::InvocationFailure,
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
        while !state.release_all && !state.released.contains(&id) && !cancellation.is_cancelled() {
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
            Err(
                unica_execution_contract::invocation::InvocationFailure::new(
                    "cancelled",
                    "manual call stopped at its checkpoint",
                ),
            )
        } else {
            Ok(unica_execution_contract::invocation::DomainResult::success(
                id,
            ))
        }
    }
}

#[derive(Default)]
struct ManualFlushGate {
    armed: AtomicBool,
    entered: AtomicBool,
    released: AtomicBool,
    flush_completed: AtomicBool,
    observed: tokio::sync::Notify,
    waiter: Mutex<Option<std::task::Waker>>,
    partial_write: AtomicBool,
    prefix_written: AtomicBool,
    fail_after_prefix: AtomicBool,
    panic_after_prefix: AtomicBool,
    written: Mutex<Vec<u8>>,
    unflushed: AtomicBool,
}

impl ManualFlushGate {
    /// Waits until no written byte awaits its flush. A client reads a
    /// frame before the server's flush of it completes; arming or clearing
    /// the capture in that window would hold or lose the earlier frame
    /// (#2063). The mark is set before the inner write wakes the reader.
    async fn quiesce(&self) {
        let quiet = timeout(TEST_STEP, async {
            loop {
                let observed = self.observed.notified();
                if !self.unflushed.load(Ordering::Acquire) {
                    break;
                }
                observed.await;
            }
        })
        .await;
        assert!(quiet.is_ok(), "earlier MCP output never finished flushing");
    }

    async fn arm(&self) {
        self.quiesce().await;
        self.armed.store(true, Ordering::Release);
    }

    fn poll_held(&self, cx: &mut std::task::Context<'_>) -> bool {
        if self.armed.load(Ordering::Acquire) && !self.released.load(Ordering::Acquire) {
            *self.waiter.lock().unwrap() = Some(cx.waker().clone());
            self.entered.store(true, Ordering::Release);
            self.observed.notify_waiters();
            if !self.released.load(Ordering::Acquire) {
                return true;
            }
        }
        false
    }

    async fn wait(&self) -> bool {
        timeout(TEST_STEP, async {
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
    initialize_flush_pause: Option<Arc<ManualFlushGate>>,
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
        // Marked before the inner write: the duplex wakes the reader inside it.
        self.gate.unflushed.store(true, Ordering::Release);
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
        let initialize_frame_written = self.initialize_flush_pause.is_some()
            && self.gate.written.lock().unwrap().contains(&b'\n');
        if initialize_frame_written {
            if let Some(pause) = &self.initialize_flush_pause {
                if !pause.flush_completed.load(Ordering::Acquire) && pause.poll_held(cx) {
                    return std::task::Poll::Pending;
                }
            }
        }
        if self.gate.poll_held(cx) {
            return std::task::Poll::Pending;
        }
        let result = std::pin::Pin::new(&mut self.writer).poll_flush(cx);
        if matches!(&result, std::task::Poll::Ready(Ok(()))) {
            self.gate.unflushed.store(false, Ordering::Release);
            self.gate.observed.notify_waiters();
        }
        if initialize_frame_written && matches!(&result, std::task::Poll::Ready(Ok(()))) {
            if let Some(pause) = &self.initialize_flush_pause {
                pause.flush_completed.store(true, Ordering::Release);
            }
        }
        result
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

type ManualReceiptKeys = Arc<Mutex<Vec<unica_execution::receipt_ledger::ReceiptKey>>>;

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
        timeout(TEST_STEP, async {
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

impl
    unica_execution::receipt_runtime::hooks::V5RuntimeHooks<
        crate::infrastructure::daemon::server::CanonicalInvocationHost,
    > for ManualCancelReplyHooks
{
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
    key: Mutex<Option<unica_execution::receipt_ledger::ReceiptKey>>,
    released: Mutex<bool>,
    changed: Condvar,
    observed: tokio::sync::Notify,
}

impl ManualReceiptGate {
    fn before_submit(&self, key: &unica_execution::receipt_ledger::ReceiptKey) {
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

    async fn wait(&self) -> Option<unica_execution::receipt_ledger::ReceiptKey> {
        timeout(TEST_STEP, async {
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
    let router = unica_mcp::test_support::canonical_daemon_router_observed(
        owner,
        workspace,
        Arc::new(move |key| gate.before_submit(key)),
    );
    UnicaServer::with_canonical_v13_router(router)
}

fn manual_daemon_server(
    owner: V5DaemonProcessOwner,
    workspace: String,
) -> (UnicaServer, ManualReceiptKeys) {
    let keys = Arc::new(Mutex::new(Vec::new()));
    let observed = keys.clone();
    let router = unica_mcp::test_support::canonical_daemon_router_observed(
        owner,
        workspace,
        Arc::new(move |key| observed.lock().unwrap().push(key.clone())),
    );
    (UnicaServer::with_canonical_v13_router(router), keys)
}

fn spawn_manual_flush_server(
    server: UnicaServer,
    gate: Arc<ManualFlushGate>,
) -> (McpClient, Arc<InFlightRegistry>) {
    spawn_manual_flush_server_with_initialize_pause(server, gate, None)
}

fn spawn_manual_flush_server_with_initialize_pause(
    server: UnicaServer,
    gate: Arc<ManualFlushGate>,
    initialize_flush_pause: Option<Arc<ManualFlushGate>>,
) -> (McpClient, Arc<InFlightRegistry>) {
    let (client_io, server_io) = tokio::io::duplex(4 * 1024 * 1024);
    let in_flight = server.in_flight_registry();
    let server = tokio::spawn(async move {
        let (read, writer) = tokio::io::split(server_io);
        let transport = rmcp::transport::async_rw::AsyncRwTransport::new_server(
            read,
            ManualFlushWriter {
                writer,
                gate,
                initialize_flush_pause,
            },
        );
        let transport = DiscoveryProbeTransport::new(transport, &server);
        match ObservedServer(server).serve(transport).await {
            Ok(running) => {
                let _ = running.waiting().await;
            }
            Err(error) if matches!(error.as_ref(), ServerInitializeError::ConnectionClosed(_)) => {}
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

impl
    unica_execution::receipt_runtime::hooks::V5RuntimeHooks<
        crate::infrastructure::daemon::server::CanonicalInvocationHost,
    > for ManualWaitHooks
{
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn wait_task_received(&self, _: unica_execution_contract::invocation::TaskId) {
        self.received.store(true, Ordering::Release);
        self.observed.notify_waiters();
    }
}

impl ManualWaitHooks {
    async fn wait(&self) -> bool {
        timeout(TEST_STEP, async {
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

/// rule: INV.WIRE.COOPERATIVE-CANCELLATION
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
    let calls = server.manual_calls_for_test();
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
    let draining = tokio::task::spawn_blocking(move || drain_stdio_frontend(&in_flight, &calls));
    let drained = matches!(timeout(TEST_STEP, draining).await, Ok(Ok(true)));
    drop(cleanup);
    drop(anchor);
    daemon.finish();
    assert!(actual_dispatch && drained, "an actual SDK refusal after output close must settle registered manual ownership without a handler or receipt key");
    assert!(keys.lock().unwrap().is_empty());
    assert_eq!(service.executions("sdk-refused"), 0);
}

/// rule: INV.WIRE.COOPERATIVE-CANCELLATION
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
    let released_observation = timeout(TEST_STEP, async {
        while in_flight.running() != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .is_ok();
    let producer_cancelled = service.cancelled("wait-owned-producer");
    service.release("wait-owned-producer");
    let completed = service.wait("wait-owned-producer", true).await;
    client.send(json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"unica.task.result","arguments":{"taskId":task_id,"waitMs":7000}}})).await;
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
    assert_eq!(
        replay["result"]["structuredContent"], result["result"]["structuredContent"],
        "completed result observation must preserve the factual receipt without another execution"
    );
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
    let settled = timeout(TEST_STEP, async {
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

/// rule: INV.WIRE.COOPERATIVE-CANCELLATION
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn canonical_manual_cancellation_eof_before_handler_preserves_registered_intent() {
    use unica_daemon_protocol::protocol_v5::V5ServerResponse;
    use unica_execution::receipt_runtime::response::{V5InvocationPhase, V5InvocationResponse};
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
        .manual_calls_for_test()
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
            observer.recover_invocation_receipt_before(key.clone(), Instant::now() + TEST_STEP),
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
        let cancelled_digest = unica_execution::receipt_ledger::canonical_v5_terminal(
            &unica_execution::receipt_ledger::ReceiptTerminalOutcome::Cancelled,
            unica_bootstrap::MAX_RESPONSE_BUDGET,
        )
        .unwrap()
        .digest()
        .clone();
        let watchdog = Instant::now() + TEST_STEP;
        while Instant::now() < watchdog {
            if matches!(observer.recover_invocation_receipt_before(key.clone(), watchdog), Ok(V5ServerResponse::Invocation { outcome: V5InvocationResponse::Direct { ref receipt } }) if receipt.receipt_key() == &key && receipt.terminal() == &unica_execution::receipt_ledger::ReceiptTerminalOutcome::Cancelled && receipt.terminal_digest() == &cancelled_digest)
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

/// rule: INV.WIRE.COOPERATIVE-CANCELLATION
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
    let manual_calls = server.manual_calls_for_test();
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

/// rule: INV.WIRE.COOPERATIVE-CANCELLATION
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

/// rule: INV.WIRE.COOPERATIVE-CANCELLATION
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

/// rule: INV.WIRE.COOPERATIVE-CANCELLATION
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
    let watchdog = Instant::now() + TEST_STEP;
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
                unica_daemon_protocol::protocol_v5::V5ServerResponse::Invocation {
                    outcome:
                        unica_execution::receipt_runtime::response::V5InvocationResponse::Task {
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

/// rule: INV.WIRE.COOPERATIVE-CANCELLATION
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn canonical_manual_cancellation_before_submit_reserves_exact_cancel_before_admission() {
    manual_before_submit_exact_cancel(false).await;
}

/// rule: INV.WIRE.COOPERATIVE-CANCELLATION
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn canonical_manual_cancellation_lost_cancel_answer_reconciles_original_key_without_replay() {
    manual_before_submit_exact_cancel(true).await;
}

async fn manual_before_submit_exact_cancel(lost_reply: bool) {
    use unica_daemon_protocol::protocol_v5::V5ServerResponse;
    use unica_execution::receipt_ledger::{canonical_v5_terminal, ReceiptTerminalOutcome};
    use unica_execution::receipt_runtime::response::{V5InvocationPhase, V5InvocationResponse};
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
    let server =
        manual_before_submit_server(daemon.owner(), daemon.workspace_hint.clone(), gate.clone());
    let output = Arc::new(ManualFlushGate::default());
    let (mut client, in_flight) = spawn_manual_flush_server(server, output.clone());
    client.initialize().await;
    output.quiesce().await;
    output.written.lock().unwrap().clear();
    send_manual_daemon_call(&mut client, 1, "before-submit").await;
    let key = gate.wait().await;
    let key_created_before_submit = key.is_some();
    let no_execution_before_submit = service.executions("before-submit") == 0;
    cancel_manual_daemon_call(&mut client, 1).await;
    let mut cancel_reserved = false;
    if let Some(key) = &key {
        let watchdog = Instant::now() + TEST_STEP;
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
    let suppressed = timeout(TEST_STEP, async {
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
    let cancelled_digest = canonical_v5_terminal(
        &ReceiptTerminalOutcome::Cancelled,
        unica_bootstrap::MAX_RESPONSE_BUDGET,
    )
    .unwrap()
    .digest()
    .clone();
    let mut exact_cancelled_terminal = false;
    if let Some(key) = &key {
        let watchdog = Instant::now() + TEST_STEP;
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

/// rule: INV.WIRE.COOPERATIVE-CANCELLATION
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn canonical_manual_cancellation_stdio_drain_does_not_cancel_held_daemon_admission() {
    use unica_daemon_protocol::protocol_v5::V5ServerResponse;
    use unica_execution::receipt_runtime::response::V5InvocationResponse;
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
    let draining =
        tokio::task::spawn_blocking(move || drain_mcp_shutdown(&in_flight, EOF_CANCELLATION_GRACE));
    let admission_cancelled = timeout(TEST_STEP, async {
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
    let watchdog = Instant::now() + TEST_STEP;
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
    output.quiesce().await;
    output.written.lock().unwrap().clear();
    output.partial_write.store(true, Ordering::Release);
    output.fail_after_prefix.store(!panic, Ordering::Release);
    output.panic_after_prefix.store(panic, Ordering::Release);
    output.arm().await;
    send_manual_daemon_call(&mut client, 1, "failed-output").await;
    let started = service.wait("failed-output", false).await;
    let fragment = timeout(TEST_STEP, client.reader.next_line()).await;
    let ended = timeout(TEST_STEP, client.reader.next_line()).await;
    let settled = timeout(TEST_STEP, async {
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

/// rule: INV.WIRE.COOPERATIVE-CANCELLATION
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
    gate.quiesce().await;
    gate.written.lock().unwrap().clear();
    gate.partial_write.store(partial, Ordering::Release);
    gate.arm().await;
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
            matches!(observer_owner.recover_invocation_receipt_before(key, Instant::now() + TEST_STEP), Ok(unica_daemon_protocol::protocol_v5::V5ServerResponse::Invocation { outcome: unica_execution::receipt_runtime::response::V5InvocationResponse::Task { snapshot } }) if snapshot.cancel_requested())
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
    joined: Mutex<std::collections::HashMap<String, unica_workspace::actor::IndexWorkIdentity>>,
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
        unica_execution::receipt_runtime::ports::ExecutionClass,
        Box<unica_execution_contract::invocation::DomainResult>,
    > {
        Ok(unica_execution::receipt_runtime::ports::ExecutionClass::InlineCandidate)
    }
    fn execute(
        &self,
        invocation: &crate::infrastructure::daemon::server::ActorBoundExecution,
        cancellation: CancellationToken,
    ) -> Result<
        unica_execution_contract::invocation::DomainResult,
        unica_execution_contract::invocation::InvocationFailure,
    > {
        use unica_work_control::shared_work::SharedWorkSnapshot;
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
                "semantic-fixture",
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
                unica_execution_contract::invocation::InvocationFailure::new("index", error)
            })?;
        self.joined.lock().unwrap().insert(label.clone(), identity);
        self.observed.notify_waiters();
        let result = loop {
            // The adapter owns this consumer checkpoint. Dropping its
            // real lease must not signal the other consumer's producer.
            if cancellation.is_cancelled() {
                break Err(
                    unica_execution_contract::invocation::InvocationFailure::new(
                        "cancelled",
                        "own shared-work consumer stopped",
                    ),
                );
            }
            match lease.wait_timeout(Duration::from_millis(10)) {
                SharedWorkSnapshot::Running { .. } => {}
                SharedWorkSnapshot::Ready(_) => {
                    break Ok(unica_execution_contract::invocation::DomainResult::success(
                        label.clone(),
                    ))
                }
                SharedWorkSnapshot::Failed(_) => {
                    break Err(
                        unica_execution_contract::invocation::InvocationFailure::new(
                            "index",
                            "shared producer failed",
                        ),
                    )
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

/// rule: INV.WIRE.COOPERATIVE-CANCELLATION
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

/// rule: INV.WIRE.COOPERATIVE-CANCELLATION
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn canonical_manual_cancellation_after_completed_before_flush_preserves_exact_terminal() {
    use unica_daemon_protocol::protocol_v5::V5ServerResponse;
    use unica_execution::receipt_runtime::response::V5InvocationResponse;
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
    gate.arm().await;
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

/// rule: INV.WIRE.COOPERATIVE-CANCELLATION
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
    let calls = server.manual_calls_for_test();
    let initialize_flush_pause = Arc::new(ManualFlushGate::default());
    initialize_flush_pause.arm().await;
    let initialize_flush_release = ManualFlushRelease(initialize_flush_pause.clone());
    let (mut client, in_flight) = spawn_manual_flush_server_with_initialize_pause(
        server,
        flush.clone(),
        Some(initialize_flush_pause.clone()),
    );
    client.send(json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2026-07-28","capabilities":{},"clientInfo":{"name":"unica-tests","version":"1"}}})).await;
    let initialized = client.receive().await;
    assert_eq!(
        initialized["result"]["protocolVersion"], "2026-07-28",
        "{initialized}"
    );
    assert_eq!(initialized["id"], 0, "{initialized}");
    assert!(
        initialize_flush_pause.wait().await,
        "causal fixture must pause the complete initialize frame before its flush"
    );
    client
        .send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
        .await;
    client.send(json!({"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"unica.run","arguments":{"op":"test.long-work","args":{"label":"reuse-original"}},"_meta":modern_meta()}})).await;
    drop(initialize_flush_release);
    let original_entered = service.wait("reuse-original", false).await;
    let receipt_count = keys.lock().unwrap().len();
    assert!(
            original_entered,
            "initial execute not observed: init_pause_entered={}, init_pause_released={}, initialize_flush_completed={}, response_flush_entered={}, receipt_keys={}, executions={}",
            initialize_flush_pause.entered.load(Ordering::Acquire),
            initialize_flush_pause.released.load(Ordering::Acquire),
            initialize_flush_pause.flush_completed.load(Ordering::Acquire),
            flush.entered.load(Ordering::Acquire),
            receipt_count,
            service.executions("reuse-original")
        );
    assert!(
        initialize_flush_pause
            .flush_completed
            .load(Ordering::Acquire),
        "original execute must follow the completed initialize flush"
    );
    flush.arm().await;
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

/// rule: INV.WIRE.COOPERATIVE-CANCELLATION
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn canonical_manual_cancellation_sole_engine_waiter_preserves_delivery_and_pinned_artifact() {
    const CHILD: &str = "UNICA_MANUAL_ENGINE_WAITER_CHILD";
    const ROOT: &str = "UNICA_MANUAL_ENGINE_WAITER_ROOT";
    if std::env::var_os(CHILD).is_none() {
        let root = tempfile::tempdir().unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "interfaces::mcp_door_tests::mcp::canonical_manual_cancellation_sole_engine_waiter_preserves_delivery_and_pinned_artifact", "--nocapture"])
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
    use unica_artifact_delivery::{BootstrapError, DownloadObserver, Downloader, HostTarget};
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
    let runner_version = "0.15.3"; // Fixture pin must match the production runner contract.
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
    let target = crate::infrastructure::current_target_id().unwrap();
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
    let daemon = ManualCanonicalDaemonOwner(Some(LiveV5Daemon::start_with_delivery_downloader(
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
    flush.arm().await;
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
        unica_execution_contract::invocation::InvocationStatus::Cancelled,
        "{terminal:?}"
    );
    assert!(!downloader.finished.load(Ordering::Acquire));
    assert!(!workspace.join("runner-dispatch.log").exists());
    flush.release();
    download_gate.release();
    let installed = timeout(TEST_STEP, async {
        loop {
            if let Some(path) =
                crate::infrastructure::bundled_tools::installed_engine_path(&plugin, "v8-runner")
            {
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
        unica_execution_contract::invocation::InvocationStatus::Completed,
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

/// rule: INV.WIRE.COOPERATIVE-CANCELLATION
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
    flush.arm().await;
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
        &crate::composition::testing::application(),
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
        &crate::composition::testing::application(),
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
/// rule: CTR.WIRE.COMPATIBILITY-TASK-TOOLS
#[tokio::test]
async fn compatibility_long_producer_survives_7000_and_100_ms_observations_once() {
    // The producer outlives the first read by the same margin in both
    // cases, so `working` survives load; a 100 ms read does not need the
    // 9500 ms producer of the 7000 ms read to observe that.
    const MARGIN_MS: u64 = 2500;
    for wait_ms in [7000_u64, 100] {
        let service = Arc::new(ScriptedService {
            delay: Duration::from_millis(wait_ms + MARGIN_MS),
            known_long: true,
            outcome: Mutex::new(Some(Ok(
                unica_execution_contract::invocation::DomainResult::success("one durable terminal"),
            ))),
            executions: AtomicUsize::new(0),
        });
        let daemon = LiveDaemon::start(service.clone());
        let (mut client, _) = spawn_unica_server(UnicaServer::with_canonical_daemon(
            daemon.owner(),
            daemon.workspace_hint.clone(),
        ));
        client.send(json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"unica.run","arguments":{"op":"test.long-work","args":{}},"_meta":modern_meta()}})).await;
        let initial = client.receive().await;
        let id = initial["result"]["structuredContent"]["data"]["task"]["taskId"]
            .as_str()
            .unwrap()
            .to_owned();
        client.send(json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"unica.task.result","arguments":{"taskId":id,"waitMs":wait_ms},"_meta":modern_meta()}})).await;
        let working = client.receive().await;
        assert_eq!(
            working["result"]["structuredContent"]["data"]["task"]["taskId"], id,
            "{working}"
        );
        assert_eq!(
            working["result"]["structuredContent"]["data"]["task"]["status"], "working",
            "{working}"
        );
        let mut recovered = false;
        for observation_id in 3..=5 {
            client.send(json!({"jsonrpc":"2.0","id":observation_id,"method":"tools/call","params":{"name":"unica.task.result","arguments":{"taskId":id,"waitMs":7000},"_meta":modern_meta()}})).await;
            let terminal = client.receive().await;
            let value = &terminal["result"]["structuredContent"];
            assert_eq!(value["ok"], true, "{terminal}");
            if value["data"]["task"]["status"] == "working" {
                continue;
            }
            assert_eq!(value["summary"], "one durable terminal", "{terminal}");
            recovered = true;
            break;
        }
        assert!(
            recovered,
            "the same Task must finish within three observations"
        );
        assert_eq!(service.executions.load(Ordering::SeqCst), 1);
        client.shutdown().await;
    }
}
