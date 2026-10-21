use super::tests::{cleanup, test_context, test_record};
use super::*;
use std::process::{Child, ExitStatus};
use unica_process::mcp_session::failure_contracts::wait_for_controlled_stderr;

const WORKER_ROOT: &str = "UNICA_PRIVATE_FAILURE_TEST_WORKER_ROOT";
const PREFIX: &str = "UNICA_PRIVATE_FAILURE_V1 ";
const CASE: &str = concat!(
    module_path!(),
    "::bsl_active_private_error_retains_safe_cause_before_cleanup"
);

struct OwnedContext(WorkspaceContext);

impl Drop for OwnedContext {
    fn drop(&mut self) {
        cleanup(&self.0);
    }
}

struct OwnedWorker(Child);

impl Drop for OwnedWorker {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn wait_worker(worker: &mut OwnedWorker) -> ExitStatus {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(status) = worker.0.try_wait().unwrap() {
            return status;
        }
        assert!(
            Instant::now() < deadline,
            "private-failure worker exceeded its bound"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn bsl_active_private_error_retains_safe_cause_before_cleanup() {
    if let Some(root) = env::var_os(WORKER_ROOT) {
        run_failure_worker(Path::new(&root));
        return;
    }
    let directory = tempfile::Builder::new()
        .prefix("unica-private-failure-")
        .tempdir()
        .unwrap();
    let root = directory.path();
    let source = root.join("producer.rs");
    fs::write(
        &source,
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/analyzer/private-failure-producer.rs"
        )),
    )
    .unwrap();
    let executable =
        unica_process::test_support::fixture_executable_path(root, "private-failure-producer");
    let status = Command::new("rustc")
        .arg("--edition=2021")
        .arg(&source)
        .arg("-o")
        .arg(&executable)
        .status()
        .unwrap();
    assert!(status.success(), "controlled producer did not compile");
    let case = CASE.split_once("::").unwrap().1;
    let log = root.join("service.stderr.log");
    let mut worker = OwnedWorker(
        Command::new(env::current_exe().unwrap())
            .args(["--exact", case, "--nocapture"])
            .env(WORKER_ROOT, root)
            .stdout(Stdio::from(
                fs::File::create(root.join("worker.stdout.log")).unwrap(),
            ))
            .stderr(Stdio::from(fs::File::create(&log).unwrap()))
            .spawn()
            .unwrap(),
    );
    let status = wait_worker(&mut worker);
    assert!(
        status.success(),
        "real private-failure worker failed before the record oracle: {status}"
    );
    let proof: Value =
        serde_json::from_slice(&fs::read(root.join("worker-proof.json")).unwrap()).unwrap();
    assert_eq!(proof["original_error_seen"], true);
    assert_eq!(proof["slot_empty"], true);
    assert_eq!(proof["helper_reaped"], true);
    let log = fs::read_to_string(log).unwrap();
    for marker in [
        "PRIVATE_SECRET_TOOL_ARGS",
        "PRIVATE_SECRET_PROTOCOL_PAYLOAD",
        "PRIVATE_SECRET_HELPER_STDERR",
    ] {
        assert!(
            !log.contains(marker),
            "captured service log leaked a fixture secret marker"
        );
    }
    let records = log
        .lines()
        .filter_map(|line| line.strip_prefix(PREFIX))
        .collect::<Vec<_>>();
    // All original error and process-cleanup checks have completed first.
    // Old main reaches only this absence assertion with a genuine protocol Err.
    assert_eq!(
        records.len(),
        1,
        "active private failure lost its safe cause record"
    );
    let record: Value = serde_json::from_str(records[0]).unwrap();
    let expected = json!({
        "version": 1,
        "provider": "bsl_analyzer",
        "phase": "wait_response",
        "cause": "invalid_json",
        "io_kind": null,
        "service_pid": proof["service_pid"],
        "helper_pid": proof["helper_pid"],
        "request_id": 2,
    });
    assert_eq!(record, expected);
    // Only the validated closed record crosses to the outer runner's stdout.
    println!("{PREFIX}{}", serde_json::to_string(&record).unwrap());
}

fn run_failure_worker(root: &Path) {
    let context_owner = OwnedContext(test_context(&format!("private-failure-{}", Uuid::new_v4())));
    let context = &context_owner.0;
    let source_root = context.workspace_root.join("src");
    let identity = WorkspaceServiceIdentity::new(context, &source_root).unwrap();
    let record = test_record(&identity, 1, CoreIdentity::production().as_str());
    let mut runtime = new_runtime(identity, &record);
    let executable =
        unica_process::test_support::fixture_executable_path(root, "private-failure-producer");
    let pid_file = root.join("producer.pid");
    let route_file = root.join("producer.route");
    runtime.test_set_analyzer_starter(Arc::new({
        let pid_file = pid_file.clone();
        let route_file = route_file.clone();
        move |_, _, cancellation| {
            let mut command = Command::new(&executable);
            command.arg(&pid_file).arg(&route_file).arg("malformed");
            let session = PersistentMcpSession::start_with_command(command, cancellation)?;
            wait_for_controlled_stderr(&session);
            Ok(session)
        }
    }));
    let response = runtime.test_handle_bsl_mcp(
        "unica.code.search",
        json!({"query": "PRIVATE_SECRET_TOOL_ARGS"}),
        5,
        0,
        &CancellationToken::new(),
    );
    assert!(
        !response.ok,
        "protocol fault must preserve the original error"
    );
    assert!(response
        .error
        .as_deref()
        .unwrap_or_default()
        .starts_with("persistent MCP emitted malformed JSON:"));
    assert!(response.error.as_deref().unwrap_or_default().contains("PRIVATE_SECRET_HELPER_STDERR"),
        "the unchanged private Err must retain the controlled raw tail for the privacy counterexample");
    assert!(response.result_text.is_none());
    assert!(runtime.test_analyzer().lock().unwrap().is_none());
    assert_eq!(
        fs::read_to_string(route_file).unwrap(),
        "initialize\ninitialized\nactive\n"
    );
    let pid: u32 = fs::read_to_string(pid_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(
        unica_process::test_support::wait_for_process_exit(pid, Duration::from_secs(2)),
        "failing native helper must be reaped"
    );
    assert!(runtime
        .test_session_teardowns()
        .drain(SESSION_TEARDOWN_GRACE));
    fs::write(
        root.join("worker-proof.json"),
        serde_json::to_vec(&json!({
            "original_error_seen": true, "slot_empty": true, "helper_reaped": true,
            "service_pid": std::process::id(), "helper_pid": pid,
        }))
        .unwrap(),
    )
    .unwrap();
}

#[test]
fn bsl_private_failure_is_owned_before_first_terminate_and_keeps_first_cause() {
    unica_process::mcp_session::failure_contracts::bsl_private_failure_is_owned_before_first_terminate_and_keeps_first_cause(unica_adapter_bsl_analyzer::test_support::write_private_failure_record);
}

#[test]
fn valid_bsl_calls_remain_reusable_without_failure_snapshot() {
    unica_process::mcp_session::failure_contracts::valid_bsl_calls_remain_reusable_without_failure_snapshot();
}

#[test]
fn ordinary_shared_transport_call_does_not_arm_bsl_capture() {
    unica_process::mcp_session::failure_contracts::ordinary_shared_transport_call_does_not_arm_bsl_capture();
}

#[test]
fn private_reader_classifies_actual_stream_faults_without_parsing_messages() {
    unica_process::mcp_session::failure_contracts::private_reader_classifies_actual_stream_faults_without_parsing_messages();
}

#[test]
fn private_writer_retains_typed_io_stage_and_original_error_text() {
    unica_process::mcp_session::failure_contracts::private_writer_retains_typed_io_stage_and_original_error_text();
}

#[test]
fn private_response_precedence_and_tool_errors_keep_original_messages() {
    unica_process::mcp_session::failure_contracts::private_response_precedence_and_tool_errors_keep_original_messages(unica_adapter_bsl_analyzer::test_support::write_private_failure_record);
}
