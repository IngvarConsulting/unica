use super::tests::{cleanup, test_context, test_record};
use super::*;
use crate::infrastructure::platform::testing;
use std::process::{Child, ExitStatus};

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
    let executable = testing::fixture_executable_path(root, "private-failure-producer");
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
    let mut runtime = WorkspaceServiceRuntime::new(identity, &record);
    let executable = testing::fixture_executable_path(root, "private-failure-producer");
    let pid_file = root.join("producer.pid");
    let route_file = root.join("producer.route");
    runtime.runtime_mut_for_test().analyzer_starter = Arc::new({
        let pid_file = pid_file.clone();
        let route_file = route_file.clone();
        move |_, _, cancellation| {
            let mut command = Command::new(&executable);
            command.arg(&pid_file).arg(&route_file).arg("malformed");
            let session = PersistentMcpSession::start_with_command(command, cancellation)?;
            wait_for_controlled_stderr(&session);
            Ok(session)
        }
    });
    let response = runtime.handle_bsl_mcp(
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
    assert!(runtime.analyzer().lock().unwrap().is_none());
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
        testing::wait_for_process_exit(pid, Duration::from_secs(2)),
        "failing native helper must be reaped"
    );
    assert!(runtime.session_teardowns().drain(SESSION_TEARDOWN_GRACE));
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

fn compile_controlled_producer(root: &Path) -> PathBuf {
    let source = root.join("producer.rs");
    fs::write(
        &source,
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/analyzer/private-failure-producer.rs"
        )),
    )
    .unwrap();
    let executable = testing::fixture_executable_path(root, "private-failure-producer");
    assert!(Command::new("rustc")
        .arg("--edition=2021")
        .arg(source)
        .arg("-o")
        .arg(&executable)
        .status()
        .unwrap()
        .success());
    executable
}

fn controlled_session(root: &Path, mode: &str) -> PersistentMcpSession {
    let executable = compile_controlled_producer(root);
    let mut command = Command::new(executable);
    command
        .arg(root.join("producer.pid"))
        .arg(root.join("producer.route"))
        .arg(mode);
    let session =
        PersistentMcpSession::start_with_command(command, &CancellationToken::new()).unwrap();
    if mode == "malformed" {
        wait_for_controlled_stderr(&session);
    }
    session
}

fn wait_for_controlled_stderr(session: &PersistentMcpSession) {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if session
            .stderr_tail
            .lock()
            .unwrap()
            .snapshot()
            .contains("PRIVATE_SECRET_HELPER_STDERR")
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "controlled stderr was not retained before the active call"
        );
        thread::yield_now();
    }
}

#[test]
fn bsl_private_failure_is_owned_before_first_terminate_and_keeps_first_cause() {
    let directory = tempfile::Builder::new()
        .prefix("unica-private-order-")
        .tempdir()
        .unwrap();
    let mut session = controlled_session(directory.path(), "malformed");
    let pid = session.child.id();
    session.before_invalidate_hook = Some(|session| {
        let snapshot = session
            .failure_snapshot
            .expect("first terminate lost the cause captured before invalidation");
        assert_eq!(snapshot.phase, FailurePhase::WaitResponse);
        assert_eq!(snapshot.cause, FailureCause::InvalidJson);
        assert_eq!(snapshot.helper_pid, session.child.id());
        assert_eq!(snapshot.request_id, 2);
        assert!(
            session.child.is_running().unwrap(),
            "capture must precede termination of the actual live helper"
        );
    });
    let error = session
        .call_with_failure_capture(
            "search",
            json!({"query":"PRIVATE_SECRET_TOOL_ARGS"}),
            Duration::from_secs(5),
            &CancellationToken::new(),
        )
        .unwrap_err();
    assert!(error.starts_with("persistent MCP emitted malformed JSON:"));
    assert!(
        session.before_invalidate_hook.is_none(),
        "first-invalidate observer did not run"
    );
    assert!(testing::wait_for_process_exit(pid, Duration::from_secs(2)));
    let first = session.failure_snapshot.unwrap();
    // A later failure/cleanup can preserve its ordinary String but cannot
    // rewrite the earlier source-typed record, even while context remains armed.
    session.active_failure_request = Some(999);
    let _ = session.fail::<()>(
        FailurePhase::WriteRequest,
        TransportFailure::new(
            FailureCause::WriterUnavailable,
            "PRIVATE_SECRET_SECOND_ERROR".to_string(),
        ),
    );
    assert_eq!(session.failure_snapshot, Some(first));
    let mut line = Vec::new();
    private_failure::write_record_to(&mut line, first).unwrap();
    let line = String::from_utf8(line).unwrap();
    assert!(!line.contains("PRIVATE_SECRET"));
    assert!(!line.contains(directory.path().to_string_lossy().as_ref()));
}

#[test]
fn valid_bsl_calls_remain_reusable_without_failure_snapshot() {
    let directory = tempfile::Builder::new()
        .prefix("unica-private-valid-")
        .tempdir()
        .unwrap();
    let mut session = controlled_session(directory.path(), "valid");
    let pid = session.child.id();
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    let error = session
        .call_with_failure_capture("search", json!({}), Duration::from_secs(5), &cancelled)
        .unwrap_err();
    assert_eq!(error, "cancelled: persistent MCP request stopped");
    assert!(session.failure_snapshot.is_none());
    assert!(session.active_failure_request.is_none());
    assert!(session.is_reusable());
    for _ in 0..2 {
        let result = session
            .call_with_failure_capture(
                "search",
                json!({}),
                Duration::from_secs(5),
                &CancellationToken::new(),
            )
            .unwrap();
        assert_eq!(result.result_text, "valid");
        assert!(session.failure_snapshot.is_none());
        assert!(session.active_failure_request.is_none());
        assert!(session.is_reusable());
    }
    session.invalidate();
    assert!(testing::wait_for_process_exit(pid, Duration::from_secs(2)));
}

#[test]
fn ordinary_shared_transport_call_does_not_arm_bsl_capture() {
    let directory = tempfile::Builder::new()
        .prefix("unica-private-unarmed-")
        .tempdir()
        .unwrap();
    let mut session = controlled_session(directory.path(), "malformed");
    let pid = session.child.id();
    let error = session
        .call(
            "search",
            json!({}),
            Duration::from_secs(5),
            &CancellationToken::new(),
        )
        .unwrap_err();
    assert!(error.starts_with("persistent MCP emitted malformed JSON:"));
    assert!(
        session.failure_snapshot.is_none(),
        "RLM/shared ordinary call must not produce a BSL capture"
    );
    assert!(testing::wait_for_process_exit(pid, Duration::from_secs(2)));
}

#[test]
fn private_reader_classifies_actual_stream_faults_without_parsing_messages() {
    use private_failure::SafeIoKind;
    let cases = [
        (Vec::new(), FailureCause::StdoutEof),
        (b"{\"id\":".to_vec(), FailureCause::IncompleteFrame),
        (
            b"PRIVATE_SECRET_BAD_JSON\n".to_vec(),
            FailureCause::InvalidJson,
        ),
        (
            vec![b'x'; SERVICE_RESPONSE_LINE_LIMIT + 1],
            FailureCause::ResponseTooLarge,
        ),
    ];
    for (bytes, cause) in cases {
        let (tx, rx) = mpsc::sync_channel(8);
        bsl_reader(std::io::Cursor::new(bytes), tx);
        let error = read_json_response_typed(
            &rx,
            7,
            &ElapsedBudgetDeadline::new(Duration::from_secs(1)),
            &CancellationToken::new(),
        )
        .unwrap_err();
        assert_eq!(error.cause, cause);
    }
    struct FaultyReader;
    impl Read for FaultyReader {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::new(
                io::ErrorKind::ConnectionReset,
                "PRIVATE_SECRET timeout stdout closed",
            ))
        }
    }
    let (tx, rx) = mpsc::sync_channel(8);
    bsl_reader(FaultyReader, tx);
    let error = read_json_response_typed(
        &rx,
        7,
        &ElapsedBudgetDeadline::new(Duration::from_secs(1)),
        &CancellationToken::new(),
    )
    .unwrap_err();
    assert_eq!(
        error.cause,
        FailureCause::ReadIo(SafeIoKind::ConnectionReset)
    );
    assert!(error
        .message
        .starts_with("failed to read persistent MCP stdout:"));
    let (tx, rx) = mpsc::sync_channel(1);
    drop(tx);
    let error = read_json_response_typed(
        &rx,
        7,
        &ElapsedBudgetDeadline::new(Duration::from_secs(1)),
        &CancellationToken::new(),
    )
    .unwrap_err();
    assert_eq!(error.cause, FailureCause::ReaderDisconnected);
    assert_eq!(
        error.message,
        "persistent MCP stdout closed before response"
    );
}

#[test]
fn private_writer_retains_typed_io_stage_and_original_error_text() {
    use private_failure::SafeIoKind;
    enum FailAt {
        Write,
        Flush,
    }
    struct FaultyWriter(FailAt);
    impl Write for FaultyWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            match self.0 {
                FailAt::Write => Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "PRIVATE_SECRET stdout closed",
                )),
                FailAt::Flush => Ok(bytes.len()),
            }
        }
        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "PRIVATE_SECRET timeout",
            ))
        }
    }
    let error = write_bsl_request(
        &mut FaultyWriter(FailAt::Write),
        &json!({"PRIVATE_SECRET":"payload"}),
    )
    .unwrap_err();
    assert_eq!(error.cause, FailureCause::WriteIo(SafeIoKind::BrokenPipe));
    assert_eq!(
        error.message,
        "failed to write persistent MCP request: PRIVATE_SECRET stdout closed"
    );
    let error = write_bsl_request(&mut FaultyWriter(FailAt::Flush), &json!({})).unwrap_err();
    assert_eq!(
        error.cause,
        FailureCause::FlushIo(SafeIoKind::PermissionDenied)
    );
    assert_eq!(
        error.message,
        "failed to flush persistent MCP request: PRIVATE_SECRET timeout"
    );
    let mut output = Vec::new();
    write_bsl_request(&mut output, &json!({"id":2})).unwrap();
    assert_eq!(output, b"{\"id\":2}\n");
}

#[test]
fn private_response_precedence_and_tool_errors_keep_original_messages() {
    let (tx, rx) = mpsc::sync_channel(1);
    tx.send(BslReaderEvent::ProtocolError(TransportFailure::new(
        FailureCause::InvalidJson,
        "PRIVATE_SECRET".to_string(),
    )))
    .unwrap();
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let error = read_json_response_typed(
        &rx,
        7,
        &ElapsedBudgetDeadline::new(Duration::from_secs(1)),
        &cancellation,
    )
    .unwrap_err();
    assert_eq!(error.cause, FailureCause::Cancelled);
    assert_eq!(error.message, "cancelled: persistent MCP request 7 stopped");
    let error = read_json_response_typed(
        &rx,
        7,
        &ElapsedBudgetDeadline::new(Duration::ZERO),
        &CancellationToken::new(),
    )
    .unwrap_err();
    assert_eq!(error.cause, FailureCause::DeadlineElapsed);
    assert_eq!(error.message, "timeout: persistent MCP request 7 timed out");
    for (response, expected_cause, expected_message) in [
        (
            json!({"error":{"message":"PRIVATE_SECRET_REMOTE_RPC"}}),
            FailureCause::RemoteRpcError,
            "PRIVATE_SECRET_REMOTE_RPC",
        ),
        (
            json!({"result":{"isError":true,"content":[{"type":"text","text":"PRIVATE_SECRET_REMOTE_TOOL"}]}}),
            FailureCause::RemoteToolError,
            "PRIVATE_SECRET_REMOTE_TOOL",
        ),
        (
            json!({}),
            FailureCause::MissingResult,
            "bsl-analyzer MCP response is missing result",
        ),
    ] {
        let error = mcp_tool_text_typed(&response).unwrap_err();
        assert_eq!(error.cause, expected_cause);
        assert_eq!(error.message, expected_message);
        let mut line = Vec::new();
        private_failure::write_record_to(
            &mut line,
            PrivateFailureSnapshot {
                phase: FailurePhase::DecodeToolResult,
                cause: error.cause,
                service_pid: 1,
                helper_pid: 2,
                request_id: 3,
            },
        )
        .unwrap();
        assert!(!String::from_utf8(line).unwrap().contains("PRIVATE_SECRET"));
    }
}
