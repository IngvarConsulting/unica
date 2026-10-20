use super::*;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use std::{env, fs, io, thread};
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
    let executable = crate::test_support::fixture_executable_path(root, "private-failure-producer");
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

pub fn wait_for_controlled_stderr(session: &PersistentMcpSession) {
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

pub fn bsl_private_failure_is_owned_before_first_terminate_and_keeps_first_cause(
    format_record: fn(&mut dyn Write, PrivateFailureSnapshot) -> std::io::Result<()>,
) {
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
    assert!(crate::test_support::wait_for_process_exit(
        pid,
        Duration::from_secs(2)
    ));
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
    format_record(&mut line, first).unwrap();
    let line = String::from_utf8(line).unwrap();
    assert!(!line.contains("PRIVATE_SECRET"));
    assert!(!line.contains(directory.path().to_string_lossy().as_ref()));
}

pub fn valid_bsl_calls_remain_reusable_without_failure_snapshot() {
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
    assert!(crate::test_support::wait_for_process_exit(
        pid,
        Duration::from_secs(2)
    ));
}

pub fn ordinary_shared_transport_call_does_not_arm_bsl_capture() {
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
    assert!(crate::test_support::wait_for_process_exit(
        pid,
        Duration::from_secs(2)
    ));
}

pub fn private_reader_classifies_actual_stream_faults_without_parsing_messages() {
    use crate::transport_failure::SafeIoKind;
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

pub fn private_writer_retains_typed_io_stage_and_original_error_text() {
    use crate::transport_failure::SafeIoKind;
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

pub fn private_response_precedence_and_tool_errors_keep_original_messages(
    format_record: fn(&mut dyn Write, PrivateFailureSnapshot) -> std::io::Result<()>,
) {
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
        format_record(
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
