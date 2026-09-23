use super::frontend_process::*;
use serde_json::Value;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

#[test]
fn task_restart_daemon_process_fixture() {
    let Some(state) = std::env::var_os("UNICA_TASK_RESTART_STATE") else {
        return;
    };
    let control = std::env::var_os("UNICA_TASK_RESTART_CONTROL").unwrap();
    unica_coder::frontend_restart_test_support::run_daemon(Path::new(&state), Path::new(&control))
        .unwrap();
}

fn spawn_task_daemon(state: &Path, control: &Path) -> OwnedProcess {
    std::fs::create_dir_all(state).unwrap();
    let process = OwnedProcess(
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "frontend_restart::task_restart_daemon_process_fixture",
                "--nocapture",
            ])
            .env("UNICA_TASK_RESTART_STATE", state)
            .env("UNICA_TASK_RESTART_CONTROL", control)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    wait_until(
        Duration::from_secs(10),
        || {
            std::fs::read(endpoint_path(state, PRODUCTION_V5_IDENTITY))
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                .is_some_and(|record| record["pid"] == process.0.id())
        },
        "Task fixture daemon endpoint",
    );
    process
}

#[test]
fn same_stdio_frontend_recovers_durable_task_without_reexecution() {
    let root = tempfile::tempdir().unwrap();
    let physical = std::fs::canonicalize(root.path()).unwrap();
    let state = physical.join("state");
    let workspace = make_stdio_workspace(&physical);
    std::fs::write(workspace.join("src/Configuration.xml"), r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Store</Name></Properties><ChildObjects/></Configuration></MetaDataObject>"#).unwrap();
    let mut initial = spawn_task_daemon(&state, &physical);
    let mut frontend = StdioFrontend::spawn(&state, &workspace);
    let initial_response = frontend.request(2, "tools/call", serde_json::json!({
        "name": "unica.view", "arguments": {"at": "main:Configuration"},
        "_meta": {"io.modelcontextprotocol/protocolVersion": "2026-07-28", "io.modelcontextprotocol/clientCapabilities": {}}
    }));
    let task_id = initial_response["result"]["structuredContent"]["data"]["task"]["taskId"]
        .as_str()
        .unwrap_or_else(|| panic!("durable Task receipt: {initial_response}"))
        .to_owned();
    wait_until(
        Duration::from_secs(5),
        || std::fs::read_to_string(physical.join("executions")).unwrap_or_default() == "execute\n",
        "provider execution begun",
    );
    initial.stop();
    let mut successor = spawn_task_daemon(&state, &physical);
    for (id, name) in [
        (3, "unica.task.get"),
        (4, "unica.task.result"),
        (5, "unica.task.cancel"),
    ] {
        if id > 3 {
            successor.stop();
            successor = spawn_task_daemon(&state, &physical);
        }
        let response = frontend.request(id, "tools/call", serde_json::json!({
            "name": name, "arguments": {"taskId": task_id},
            "_meta": {"io.modelcontextprotocol/protocolVersion": "2026-07-28", "io.modelcontextprotocol/clientCapabilities": {}}
        }));
        assert!(response.get("result").is_some(), "{name}: {response}");
        let result = &response["result"]["structuredContent"];
        assert_eq!(
            result["data"]["task"]["taskId"], task_id,
            "{name}: {response}"
        );
        assert_eq!(
            result["data"]["task"]["status"], "failed",
            "{name}: {response}"
        );
    }
    assert_eq!(
        std::fs::read_to_string(physical.join("executions")).unwrap(),
        "execute\n",
        "recovery and Task APIs must not repeat the provider"
    );
}

#[test]
fn same_native_stdio_frontend_observes_uncertain_task_without_reexecution() {
    native_restart_case(false);
}

#[test]
fn same_native_stdio_frontend_preserves_completed_task_result_after_restart() {
    native_restart_case(true);
}

fn native_restart_case(complete_before_crash: bool) {
    let root = tempfile::tempdir().unwrap();
    let physical = std::fs::canonicalize(root.path()).unwrap();
    let state = physical.join("state");
    let workspace = make_stdio_workspace(&physical);
    std::fs::write(workspace.join("src/Configuration.xml"), r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Store</Name></Properties><ChildObjects/></Configuration></MetaDataObject>"#).unwrap();
    let mut initial = spawn_task_daemon(&state, &physical);
    let mut frontend = StdioFrontend::spawn_native(&state, &workspace);
    let response = frontend.request(
        2,
        "tools/call",
        serde_json::json!({
            "name": "unica.view", "arguments": {"at": "main:Configuration"}
        }),
    );
    assert_eq!(response["result"]["resultType"], "task", "{response}");
    let task_id = response["result"]["taskId"].as_str().unwrap().to_owned();
    wait_until(
        Duration::from_secs(5),
        || std::fs::read_to_string(physical.join("executions")).unwrap_or_default() == "execute\n",
        "native provider begun",
    );
    let mut completed = None;
    if complete_before_crash {
        std::fs::write(physical.join("release"), "release").unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let observed = frontend.request(3, "tasks/get", serde_json::json!({"taskId": task_id}));
            if observed["result"]["status"] == "completed" {
                assert_eq!(
                    observed["result"]["result"]["structuredContent"]["summary"],
                    "provider completed",
                    "{observed}"
                );
                completed = Some(observed["result"].clone());
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "provider did not commit terminal result: {observed}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    initial.stop();
    let mut successor = spawn_task_daemon(&state, &physical);
    let observed = frontend.request(4, "tasks/get", serde_json::json!({"taskId": task_id}));
    assert_eq!(observed["result"]["taskId"], task_id, "{observed}");
    if let Some(completed) = completed {
        assert_eq!(
            observed["result"], completed,
            "committed result and Task metadata must survive restart"
        );
    } else {
        assert_eq!(observed["result"]["status"], "failed", "{observed}");
        assert_eq!(
            observed["result"]["error"]["data"]["code"], "outcome_uncertain",
            "{observed}"
        );
    }
    successor.stop();
    let _successor = spawn_task_daemon(&state, &physical);
    let cancelled = frontend.request(5, "tasks/cancel", serde_json::json!({"taskId": task_id}));
    assert!(cancelled.get("result").is_some(), "{cancelled}");
    let after_cancel = frontend.request(6, "tasks/get", serde_json::json!({"taskId": task_id}));
    assert_eq!(
        after_cancel["result"], observed["result"],
        "cancel cannot rewrite a recovered terminal"
    );
    assert_eq!(
        std::fs::read_to_string(physical.join("executions")).unwrap(),
        "execute\n",
        "Task APIs and recovery must not execute the provider again"
    );
    frontend.check(7);
}
