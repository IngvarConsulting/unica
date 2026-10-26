//! Owned persistent stdio JSON-RPC tool calls and bounded causal observations.
use crate::transport_failure::{
    FailureCause, FailurePhase, PrivateFailureSnapshot, TransportFailure,
};
use crate::ManagedChild;
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::process::{ChildStdin, Command};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc, Mutex,
};
use std::thread;
use std::time::Duration;
#[cfg(test)]
use std::time::Instant;
use unica_work_control::cancellation::{cancelled_error, CancellationToken};
use unica_work_control::elapsed_budget::ElapsedBudgetDeadline;

const SERVICE_REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const SERVICE_RESPONSE_LINE_LIMIT: usize = 8 * 1024 * 1024;
const BSL_STDERR_TAIL_LIMIT: usize = 64 * 1024;
/// How long the cancellation notice of an abandoned call may take to write.
const CANCEL_NOTICE_TIMEOUT: Duration = Duration::from_secs(1);

pub struct PersistentMcpSession {
    child: ManagedChild,
    writer: Option<mpsc::SyncSender<BslWriteRequest>>,
    rx: mpsc::Receiver<BslReaderEvent>,
    stderr_tail: Arc<Mutex<BoundedByteTail>>,
    reader_terminal: Arc<AtomicBool>,
    next_id: i64,
    valid: bool,
    active_failure_request: Option<i64>,
    failure_snapshot: Option<PrivateFailureSnapshot>,
    #[cfg(any(test, feature = "test-support"))]
    before_invalidate_hook: Option<fn(&mut PersistentMcpSession)>,
}

struct BoundedByteTail {
    bytes: VecDeque<u8>,
    limit: usize,
}

struct BslWriteRequest {
    payload: Value,
    result: mpsc::SyncSender<Result<(), TransportFailure>>,
}

enum BslReaderEvent {
    Message(Value),
    ProtocolError(TransportFailure),
    Closed,
}

impl PersistentMcpSession {
    pub fn start_with_command(
        command: Command,
        cancellation: &CancellationToken,
    ) -> Result<Self, String> {
        if cancellation.is_cancelled() {
            return Err(cancelled_error("persistent MCP start stopped"));
        }
        let mut child = ManagedChild::spawn_process(command, None, cancellation.clone())
            .map_err(|err| format!("failed to start persistent MCP transport: {err}"))?;
        let stdin = child
            .take_stdin()
            .ok_or_else(|| "failed to open persistent MCP stdin".to_string())?;
        let stdout = child
            .take_stdout()
            .ok_or_else(|| "failed to open persistent MCP stdout".to_string())?;
        let stderr = child
            .take_stderr()
            .ok_or_else(|| "failed to open persistent MCP stderr".to_string())?;

        let (writer, writer_rx) = mpsc::sync_channel::<BslWriteRequest>(1);
        thread::spawn(move || bsl_writer(stdin, writer_rx));
        let (tx, rx) = mpsc::sync_channel::<BslReaderEvent>(8);
        let reader_terminal = Arc::new(AtomicBool::new(false));
        let stdout_state = Arc::clone(&reader_terminal);
        thread::spawn(move || bsl_reader_with_state(stdout, tx, stdout_state));
        let stderr_tail = Arc::new(Mutex::new(BoundedByteTail::new(BSL_STDERR_TAIL_LIMIT)));
        let stderr_target = Arc::clone(&stderr_tail);
        thread::spawn(move || bsl_stderr_reader(stderr, stderr_target));

        let mut session = Self {
            child,
            writer: Some(writer),
            rx,
            stderr_tail,
            reader_terminal,
            next_id: 2,
            valid: true,
            active_failure_request: None,
            failure_snapshot: None,
            #[cfg(any(test, feature = "test-support"))]
            before_invalidate_hook: None,
        };
        session.initialize(cancellation)?;
        Ok(session)
    }

    fn initialize(&mut self, cancellation: &CancellationToken) -> Result<(), String> {
        if cancellation.is_cancelled() {
            return Err(cancelled_error("persistent MCP initialize stopped"));
        }
        let deadline = ElapsedBudgetDeadline::new(SERVICE_REQUEST_TIMEOUT);
        self.send_json_cancellable(
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-03-26",
                    "capabilities": {},
                    "clientInfo": {
                        "name": "unica",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                }
            }),
            &deadline,
            cancellation,
        )?;
        let _ = self.read_response(1, &deadline, cancellation)?;
        if cancellation.is_cancelled() {
            return Err(cancelled_error("persistent MCP initialize stopped"));
        }
        self.send_json_cancellable(
            json!({
                "jsonrpc": "2.0",
                "method": "notifications/initialized"
            }),
            &deadline,
            cancellation,
        )?;
        if cancellation.is_cancelled() {
            return Err(cancelled_error("persistent MCP initialize stopped"));
        }
        Ok(())
    }

    pub fn call_with_failure_capture(
        &mut self,
        tool_name: &str,
        tool_args: Value,
        timeout: Duration,
        cancellation: &CancellationToken,
    ) -> Result<McpToolOutput, String> {
        self.failure_snapshot = None;
        self.active_failure_request = Some(self.next_id);
        let result = self.call(tool_name, tool_args, timeout, cancellation);
        self.active_failure_request = None;
        result
    }

    pub fn call(
        &mut self,
        tool_name: &str,
        tool_args: Value,
        timeout: Duration,
        cancellation: &CancellationToken,
    ) -> Result<McpToolOutput, String> {
        if cancellation.is_cancelled() {
            return Err(cancelled_error("persistent MCP request stopped"));
        }
        let id = self.next_id;
        self.next_id += 1;
        let deadline = ElapsedBudgetDeadline::new(timeout);
        self.send_json_cancellable(
            json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": "tools/call",
                "params": {
                    "name": tool_name,
                    "arguments": tool_args
                }
            }),
            &deadline,
            cancellation,
        )?;
        let response = match read_json_response_typed(&self.rx, id, &deadline, cancellation) {
            Ok(response) => response,
            Err(error)
                if matches!(
                    error.cause,
                    FailureCause::Cancelled | FailureCause::DeadlineElapsed
                ) =>
            {
                return self.abandon(id, error);
            }
            Err(error) => return self.fail(FailurePhase::WaitResponse, error),
        };
        // The exchange is whole from here: a cancellation only drops its answer.
        if cancellation.is_cancelled() {
            return Err(cancelled_error("persistent MCP request stopped"));
        }
        let result_text = match mcp_tool_text_typed(&response) {
            Ok(text) => text,
            Err(error) => {
                if cancellation.is_cancelled() {
                    return Err(cancelled_error("persistent MCP request stopped"));
                }
                return self.fail(FailurePhase::DecodeToolResult, error);
            }
        };
        if cancellation.is_cancelled() {
            return Err(cancelled_error("persistent MCP request stopped"));
        }
        let stderr = self
            .stderr_tail
            .lock()
            .map(|tail| tail.snapshot())
            .unwrap_or_default();
        Ok(McpToolOutput {
            result_text,
            stderr,
        })
    }

    /// Gives up waiting for call `id` without giving up the session: the
    /// server is told to stop the call, and its late answer, if any, is
    /// skipped by the next call's read. Only a failed notice retires it.
    fn abandon<T>(&mut self, id: i64, error: TransportFailure) -> Result<T, String> {
        self.send_json_cancellable(
            json!({
                "jsonrpc": "2.0",
                "method": "notifications/cancelled",
                "params": {"requestId": id, "reason": error.message}
            }),
            &ElapsedBudgetDeadline::new(CANCEL_NOTICE_TIMEOUT),
            &CancellationToken::new(),
        )?;
        Err(error.message)
    }

    pub fn is_reusable(&mut self) -> bool {
        self.valid
            && self.writer.is_some()
            && !self.reader_terminal.load(Ordering::Acquire)
            && self.child.is_running().unwrap_or(false)
    }

    pub fn retire_for_replacement(
        &mut self,
        remaining: impl Fn() -> Result<Duration, String>,
    ) -> Result<(), String> {
        if self.writer.is_none() || !self.valid {
            return self.terminate_for_replacement();
        }
        // EOF lets the direct provider release its supported cache ownership.
        // A replacement must not race that shutdown with the same cache path.
        drop(self.writer.take());
        let retired = (|| loop {
            let remaining = remaining()?;
            if self.child.poll_owned_leader_exit()? {
                self.valid = false;
                return Ok(());
            }
            thread::sleep(remaining.min(Duration::from_millis(10)));
        })();
        match retired {
            Ok(()) => Ok(()),
            Err(error) => match self.terminate_for_replacement() {
                Ok(()) => Err(error),
                Err(cleanup) => Err(format!(
                    "{error}; analyzer retirement remains unconfirmed: {cleanup}"
                )),
            },
        }
    }

    fn terminate_for_replacement(&mut self) -> Result<(), String> {
        self.child.terminate()?;
        if !self.child.poll_owned_leader_exit()? {
            return Err("analyzer leader termination is still pending".to_string());
        }
        self.valid = false;
        Ok(())
    }

    fn send_json_cancellable(
        &mut self,
        payload: Value,
        deadline: &ElapsedBudgetDeadline,
        cancellation: &CancellationToken,
    ) -> Result<(), String> {
        let (result, result_rx) = mpsc::sync_channel(1);
        let mut request = BslWriteRequest { payload, result };
        loop {
            if cancellation.is_cancelled() {
                return self.fail(
                    FailurePhase::WriteRequest,
                    TransportFailure::new(
                        FailureCause::Cancelled,
                        cancelled_error("persistent MCP write stopped"),
                    ),
                );
            }
            if deadline.remaining().is_none() {
                return self.fail(
                    FailurePhase::WriteRequest,
                    TransportFailure::new(
                        FailureCause::DeadlineElapsed,
                        "timeout: persistent MCP write timed out".to_string(),
                    ),
                );
            }
            let Some(writer) = &self.writer else {
                return self.fail(
                    FailurePhase::WriteRequest,
                    TransportFailure::new(
                        FailureCause::WriterUnavailable,
                        "persistent MCP stdin writer stopped".to_string(),
                    ),
                );
            };
            match writer.try_send(request) {
                Ok(()) => break,
                Err(mpsc::TrySendError::Full(returned)) => request = returned,
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    return self.fail(
                        FailurePhase::WriteRequest,
                        TransportFailure::new(
                            FailureCause::WriterUnavailable,
                            "persistent MCP stdin writer stopped".to_string(),
                        ),
                    );
                }
            }
            thread::sleep(Duration::from_millis(10));
        }
        loop {
            if cancellation.is_cancelled() {
                return self.fail(
                    FailurePhase::WriteRequest,
                    TransportFailure::new(
                        FailureCause::Cancelled,
                        cancelled_error("persistent MCP write stopped"),
                    ),
                );
            }
            let Some(remaining) = deadline.remaining() else {
                return self.fail(
                    FailurePhase::WriteRequest,
                    TransportFailure::new(
                        FailureCause::DeadlineElapsed,
                        "timeout: persistent MCP write timed out".to_string(),
                    ),
                );
            };
            let received = result_rx.recv_timeout(remaining.min(Duration::from_millis(25)));
            if cancellation.is_cancelled() {
                return self.fail(
                    FailurePhase::WriteRequest,
                    TransportFailure::new(
                        FailureCause::Cancelled,
                        cancelled_error("persistent MCP write stopped"),
                    ),
                );
            }
            if deadline.remaining().is_none() {
                return self.fail(
                    FailurePhase::WriteRequest,
                    TransportFailure::new(
                        FailureCause::DeadlineElapsed,
                        "timeout: persistent MCP write timed out".to_string(),
                    ),
                );
            }
            match received {
                Ok(Ok(())) => {
                    return Ok(());
                }
                Ok(Err(error)) => {
                    return self.fail(FailurePhase::WriteRequest, error);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return self.fail(
                        FailurePhase::WriteRequest,
                        TransportFailure::new(
                            FailureCause::WriterUnavailable,
                            "persistent MCP stdin writer stopped".to_string(),
                        ),
                    );
                }
            }
        }
    }

    fn read_response(
        &mut self,
        id: i64,
        deadline: &ElapsedBudgetDeadline,
        cancellation: &CancellationToken,
    ) -> Result<Value, String> {
        match read_json_response_typed(&self.rx, id, deadline, cancellation) {
            Ok(value) => Ok(value),
            Err(error) => self.fail(FailurePhase::WaitResponse, error),
        }
    }

    fn fail<T>(&mut self, phase: FailurePhase, error: TransportFailure) -> Result<T, String> {
        // This slot owns only finite, source-typed metadata before the first
        // termination. Raw error/stderr text is never part of the snapshot.
        if self.failure_snapshot.is_none() {
            if let Some(request_id) = self.active_failure_request {
                self.failure_snapshot = Some(PrivateFailureSnapshot {
                    phase,
                    cause: error.cause,
                    service_pid: std::process::id(),
                    helper_pid: self.child.id(),
                    request_id,
                });
            }
        }
        let stderr = self
            .stderr_tail
            .lock()
            .map(|tail| tail.snapshot())
            .unwrap_or_default();
        self.invalidate();
        let error = error.message;
        if stderr.is_empty() {
            Err(error)
        } else {
            Err(format!("{error}; stderr tail: {stderr}"))
        }
    }

    pub fn invalidate(&mut self) {
        if self.valid {
            #[cfg(any(test, feature = "test-support"))]
            if let Some(hook) = self.before_invalidate_hook.take() {
                hook(self);
            }
            self.valid = false;
            let _ = self.child.terminate();
        }
    }
}

impl Drop for PersistentMcpSession {
    fn drop(&mut self) {
        self.invalidate();
    }
}

fn bsl_writer(mut stdin: ChildStdin, requests: mpsc::Receiver<BslWriteRequest>) {
    while let Ok(request) = requests.recv() {
        let result = write_bsl_request(&mut stdin, &request.payload);
        let failed = result.is_err();
        let _ = request.result.send(result);
        if failed {
            break;
        }
    }
}

fn write_bsl_request(writer: &mut impl Write, payload: &Value) -> Result<(), TransportFailure> {
    let mut bytes = serde_json::to_vec(payload).map_err(|error| {
        TransportFailure::new(
            FailureCause::EncodeRequest,
            format!("failed to encode persistent MCP request: {error}"),
        )
    })?;
    bytes.push(b'\n');
    writer.write_all(&bytes).map_err(|error| {
        TransportFailure::new(
            FailureCause::WriteIo(error.kind().into()),
            format!("failed to write persistent MCP request: {error}"),
        )
    })?;
    writer.flush().map_err(|error| {
        TransportFailure::new(
            FailureCause::FlushIo(error.kind().into()),
            format!("failed to flush persistent MCP request: {error}"),
        )
    })
}

fn bsl_stderr_reader(mut stderr: impl Read, tail: Arc<Mutex<BoundedByteTail>>) {
    let mut chunk = [0_u8; 8192];
    loop {
        match stderr.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(count) => {
                if let Ok(mut tail) = tail.lock() {
                    tail.append(&chunk[..count]);
                } else {
                    return;
                }
            }
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
fn bsl_reader(stdout: impl Read, events: mpsc::SyncSender<BslReaderEvent>) {
    bsl_reader_with_state(stdout, events, Arc::new(AtomicBool::new(false)));
}

fn bsl_reader_with_state(
    mut stdout: impl Read,
    events: mpsc::SyncSender<BslReaderEvent>,
    closed: Arc<AtomicBool>,
) {
    let _terminal = BslReaderTerminalGuard(closed);
    let mut pending = Vec::new();
    let mut chunk = [0_u8; 8192];
    loop {
        match stdout.read(&mut chunk) {
            Ok(0) => {
                let event = if pending.is_empty() {
                    BslReaderEvent::Closed
                } else {
                    BslReaderEvent::ProtocolError(TransportFailure::new(
                        FailureCause::IncompleteFrame,
                        "persistent MCP stdout closed with an incomplete JSON line".to_string(),
                    ))
                };
                let _ = events.send(event);
                return;
            }
            Ok(count) => {
                for &byte in &chunk[..count] {
                    if byte == b'\n' {
                        if pending.last() == Some(&b'\r') {
                            pending.pop();
                        }
                        let event = match serde_json::from_slice::<Value>(&pending) {
                            Ok(value) => BslReaderEvent::Message(value),
                            Err(error) => BslReaderEvent::ProtocolError(TransportFailure::new(
                                FailureCause::InvalidJson,
                                format!("persistent MCP emitted malformed JSON: {error}"),
                            )),
                        };
                        let malformed = matches!(event, BslReaderEvent::ProtocolError(_));
                        if events.send(event).is_err() || malformed {
                            return;
                        }
                        pending.clear();
                    } else {
                        pending.push(byte);
                        if pending.len() > SERVICE_RESPONSE_LINE_LIMIT {
                            let _ = events.send(BslReaderEvent::ProtocolError(TransportFailure::new(
                                FailureCause::ResponseTooLarge,
                                format!("persistent MCP response exceeds {SERVICE_RESPONSE_LINE_LIMIT} bytes"),
                            )));
                            return;
                        }
                    }
                }
            }
            Err(error) => {
                let _ = events.send(BslReaderEvent::ProtocolError(TransportFailure::new(
                    FailureCause::ReadIo(error.kind().into()),
                    format!("failed to read persistent MCP stdout: {error}"),
                )));
                return;
            }
        }
    }
}

#[cfg(test)]
fn read_json_response_cancellable(
    rx: &mpsc::Receiver<BslReaderEvent>,
    id: i64,
    deadline: &ElapsedBudgetDeadline,
    cancellation: &CancellationToken,
) -> Result<Value, String> {
    read_json_response_typed(rx, id, deadline, cancellation).map_err(|error| error.message)
}

fn read_json_response_typed(
    rx: &mpsc::Receiver<BslReaderEvent>,
    id: i64,
    deadline: &ElapsedBudgetDeadline,
    cancellation: &CancellationToken,
) -> Result<Value, TransportFailure> {
    while let Some(remaining) = deadline.remaining() {
        if cancellation.is_cancelled() {
            return Err(TransportFailure::new(
                FailureCause::Cancelled,
                cancelled_error(format!("persistent MCP request {id} stopped")),
            ));
        }
        let received = rx.recv_timeout(remaining.min(Duration::from_millis(50)));
        if cancellation.is_cancelled() {
            return Err(TransportFailure::new(
                FailureCause::Cancelled,
                cancelled_error(format!("persistent MCP request {id} stopped")),
            ));
        }
        if deadline.remaining().is_none() {
            return Err(TransportFailure::new(
                FailureCause::DeadlineElapsed,
                format!("timeout: persistent MCP request {id} timed out"),
            ));
        }
        match received {
            Ok(BslReaderEvent::Message(value)) => {
                if value.get("id").and_then(Value::as_i64) == Some(id) {
                    return Ok(value);
                }
            }
            Ok(BslReaderEvent::ProtocolError(error)) => {
                return Err(error);
            }
            Ok(BslReaderEvent::Closed) => {
                return Err(TransportFailure::new(
                    FailureCause::StdoutEof,
                    "persistent MCP stdout closed before response".to_string(),
                ));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(TransportFailure::new(
                    FailureCause::ReaderDisconnected,
                    "persistent MCP stdout closed before response".to_string(),
                ));
            }
        }
    }
    if cancellation.is_cancelled() {
        return Err(TransportFailure::new(
            FailureCause::Cancelled,
            cancelled_error(format!("persistent MCP request {id} stopped")),
        ));
    }
    Err(TransportFailure::new(
        FailureCause::DeadlineElapsed,
        format!("timeout: persistent MCP request {id} timed out"),
    ))
}

#[cfg(test)]
fn mcp_tool_text(response: &Value) -> Result<String, String> {
    mcp_tool_text_typed(response).map_err(|error| error.message)
}

fn mcp_tool_text_typed(response: &Value) -> Result<String, TransportFailure> {
    if let Some(error) = response.get("error") {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("bsl-analyzer MCP JSON-RPC error");
        return Err(TransportFailure::new(
            FailureCause::RemoteRpcError,
            message.to_string(),
        ));
    }
    let result = response.get("result").ok_or_else(|| {
        TransportFailure::new(
            FailureCause::MissingResult,
            "bsl-analyzer MCP response is missing result".to_string(),
        )
    })?;
    if result
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        let message = result
            .get("content")
            .and_then(Value::as_array)
            .and_then(|content| {
                content
                    .iter()
                    .filter_map(|item| item.get("text").and_then(Value::as_str))
                    .next()
            })
            .unwrap_or("bsl-analyzer MCP tool returned an error");
        return Err(TransportFailure::new(
            FailureCause::RemoteToolError,
            message.to_string(),
        ));
    }
    if let Some(content) = result.get("content").and_then(Value::as_array) {
        let parts = content
            .iter()
            .filter_map(|item| item.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>();
        if !parts.is_empty() {
            return Ok(parts.join("\n"));
        }
    }
    Ok(result.to_string())
}

struct BslReaderTerminalGuard(Arc<AtomicBool>);

impl Drop for BslReaderTerminalGuard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

#[derive(Debug, Clone)]
pub struct McpToolOutput {
    pub result_text: String,
    pub stderr: String,
}

impl PersistentMcpSession {
    pub fn take_failure_snapshot(&mut self) -> Option<PrivateFailureSnapshot> {
        self.failure_snapshot.take()
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn test_poll_owned_leader_exit(&mut self) -> Result<bool, String> {
        self.child.poll_owned_leader_exit()
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn test_child_pid(&self) -> u32 {
        self.child.id()
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn test_reader_terminal(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.reader_terminal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_service_control_path_analyzer_wait_observes_operation_token() {
        let (_tx, rx) = mpsc::channel::<BslReaderEvent>();
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let started = Instant::now();
        let deadline = ElapsedBudgetDeadline::new(Duration::from_secs(30));

        let error = read_json_response_cancellable(&rx, 7, &deadline, &cancellation).unwrap_err();

        assert!(error.starts_with("cancelled:"));
        assert!(started.elapsed() < Duration::from_millis(500));
    }

    use std::io;
    struct ChunkedReader {
        bytes: std::io::Cursor<Vec<u8>>,
        chunk_size: usize,
    }

    impl Read for ChunkedReader {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let limit = buffer.len().min(self.chunk_size);
            self.bytes.read(&mut buffer[..limit])
        }
    }

    #[test]
    fn bsl_session_stdout_framing_accepts_fragments_and_rejects_bad_or_oversized_lines() {
        let (tx, rx) = mpsc::sync_channel(8);
        bsl_reader(
        ChunkedReader {
            bytes: std::io::Cursor::new(
                b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\"}\n{\"jsonrpc\":\"2.0\",\"id\":7,\"result\":{}}\n".to_vec(),
            ),
            chunk_size: 3,
        },
        tx,
    );
        let deadline = ElapsedBudgetDeadline::new(Duration::from_secs(1));
        let response =
            read_json_response_cancellable(&rx, 7, &deadline, &CancellationToken::new()).unwrap();
        assert_eq!(response["id"], 7);

        let (tx, rx) = mpsc::sync_channel(8);
        bsl_reader(std::io::Cursor::new(b"not-json\n"), tx);
        assert!(matches!(
            rx.recv().unwrap(),
            BslReaderEvent::ProtocolError(error) if error.message.contains("malformed JSON")
        ));

        let (tx, rx) = mpsc::sync_channel(8);
        let terminal = Arc::new(AtomicBool::new(false));
        bsl_reader_with_state(
            std::io::Cursor::new(vec![b'x'; SERVICE_RESPONSE_LINE_LIMIT + 1]),
            tx,
            Arc::clone(&terminal),
        );
        assert!(matches!(
            rx.recv().unwrap(),
            BslReaderEvent::ProtocolError(error) if error.message.contains("exceeds")
        ));
        assert!(terminal.load(Ordering::Acquire));
    }

    #[test]
    fn bsl_session_incomplete_eof_is_an_immediate_protocol_error() {
        let (tx, rx) = mpsc::sync_channel(8);
        bsl_reader(std::io::Cursor::new(b"{\"jsonrpc\":\"2.0\""), tx);
        let started = Instant::now();
        let deadline = ElapsedBudgetDeadline::new(Duration::from_secs(30));
        let error = read_json_response_cancellable(&rx, 1, &deadline, &CancellationToken::new())
            .unwrap_err();
        assert!(error.contains("incomplete JSON line"), "{error}");
        assert!(started.elapsed() < Duration::from_millis(100));
    }

    #[test]
    fn bsl_session_stderr_reader_keeps_only_a_bounded_lossy_tail() {
        let mut noisy = vec![b'x'; BSL_STDERR_TAIL_LIMIT * 4];
        noisy.extend_from_slice(&[0xff, 0xfe]);
        noisy.extend_from_slice(b"TAIL-MARKER");
        let tail = Arc::new(Mutex::new(BoundedByteTail::new(BSL_STDERR_TAIL_LIMIT)));
        let started = Instant::now();
        bsl_stderr_reader(std::io::Cursor::new(noisy), Arc::clone(&tail));
        let snapshot = tail.lock().unwrap().snapshot();
        assert!(snapshot.len() <= BSL_STDERR_TAIL_LIMIT);
        assert!(snapshot.ends_with("TAIL-MARKER"), "{snapshot}");
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn bsl_session_cancellation_wins_over_protocol_and_eof_races() {
        for event in [
            BslReaderEvent::ProtocolError(TransportFailure::new(
                FailureCause::InvalidJson,
                "bad response".to_string(),
            )),
            BslReaderEvent::Closed,
        ] {
            let (tx, rx) = mpsc::channel();
            tx.send(event).unwrap();
            let cancellation = CancellationToken::new();
            cancellation.cancel();
            let deadline = ElapsedBudgetDeadline::new(Duration::from_secs(1));
            let error =
                read_json_response_cancellable(&rx, 7, &deadline, &cancellation).unwrap_err();
            assert!(error.starts_with("cancelled:"), "{error}");
        }
    }

    #[test]
    fn mcp_tool_text_rejects_tool_level_error() {
        let response = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "result": {
                "content": [{ "type": "text", "text": "schema is unavailable" }],
                "isError": true
            }
        });

        let error = mcp_tool_text(&response).unwrap_err();

        assert_eq!(error, "schema is unavailable");
    }
}

impl BoundedByteTail {
    fn new(limit: usize) -> Self {
        Self {
            bytes: VecDeque::with_capacity(limit),
            limit,
        }
    }

    fn append(&mut self, bytes: &[u8]) {
        let bytes = if bytes.len() > self.limit {
            &bytes[bytes.len() - self.limit..]
        } else {
            bytes
        };
        let overflow = self
            .bytes
            .len()
            .saturating_add(bytes.len())
            .saturating_sub(self.limit);
        self.bytes.drain(..overflow.min(self.bytes.len()));
        self.bytes.extend(bytes);
    }

    fn snapshot(&self) -> String {
        let bytes = self.bytes.iter().copied().collect::<Vec<_>>();
        let text = String::from_utf8_lossy(&bytes).into_owned();
        if text.len() <= self.limit {
            return text;
        }
        let mut start = text.len() - self.limit;
        while !text.is_char_boundary(start) {
            start += 1;
        }
        text[start..].to_string()
    }
}

#[cfg(any(test, feature = "test-support"))]
#[path = "mcp_session/failure_contracts.rs"]
pub mod failure_contracts;
