//! Two daemons for the frontend tests: a scripted fake that answers frame
//! by frame, and the real protocol-v5 runtime in a thread with an injected
//! canonical service.
use crate::daemon_router::*;
use std::sync::Arc;
use std::time::{Duration, Instant};
use unica_daemon_protocol::client_v5::V5DaemonProcessOwner;
use unica_daemon_protocol::protocol_v5::V5ServerResponse;
use unica_execution::receipt_ledger::ReceiptKey;
use unica_execution::receipt_runtime::response::V5InvocationResponse;

use std::io::{BufReader, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::thread;
use unica_daemon_protocol::identity::CoreIdentity;
use unica_daemon_protocol::protocol_v5::{
    decode_v5_request_frame, read_bounded_v5_request_frame, V5ClientRequest, V5EndpointRecord,
    V5HandshakeServerResponse,
};
use unica_execution::receipt_ledger::{canonical_v5_terminal, ReceiptTerminalOutcome};
use unica_execution::receipt_runtime::response::V5PendingDirectReceipt;
use unica_execution_contract::invocation::DomainResult;

/// One accepted session of the fake: what it saw and what it answered.
pub enum Step {
    Reply(V5ServerResponse),
    Close,
}

pub type Script = Box<dyn FnMut(&[u8], &V5ClientRequest) -> Step + Send>;
/// A script that writes the reply bytes itself; `false` closes the session.
pub type RawScript = Box<dyn FnMut(&[u8], &V5ClientRequest, &mut TcpStream) -> bool + Send>;

pub enum SessionScript {
    Framed(Script),
    Raw(RawScript),
}

pub struct FakeDaemon {
    pub workspace: tempfile::TempDir,
    pub record: V5EndpointRecord,
    pub seen: Arc<Mutex<Vec<V5ClientRequest>>>,
    pub sessions: Arc<AtomicUsize>,
    pub thread: Option<thread::JoinHandle<()>>,
}

impl FakeDaemon {
    pub fn start(script: Script) -> Self {
        Self::start_with_handshake_delay(script, Duration::ZERO)
    }

    /// Same fake, but every session answers `Hello` only after `delay`:
    /// the frontend cutoff tests spend budget on the handshake on purpose.
    pub fn start_with_handshake_delay(script: Script, handshake_delay: Duration) -> Self {
        Self::start_session_script(SessionScript::Framed(script), handshake_delay)
    }

    pub fn start_with_raw_script(script: RawScript) -> Self {
        Self::start_session_script(SessionScript::Raw(script), Duration::ZERO)
    }

    /// The anchor session stands; every later session is dropped right
    /// after its handshake request, as a daemon refusing the connection.
    pub fn start_rejecting_peers() -> Self {
        Self::start_sessions(
            SessionScript::Framed(Box::new(|_, request| {
                panic!("a refused peer sends nothing: {request:?}")
            })),
            Duration::ZERO,
            true,
        )
    }

    pub fn start_session_script(script: SessionScript, handshake_delay: Duration) -> Self {
        Self::start_sessions(script, handshake_delay, false)
    }

    pub fn start_sessions(
        script: SessionScript,
        handshake_delay: Duration,
        reject_peers: bool,
    ) -> Self {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        listener
            .set_nonblocking(true)
            .expect("nonblocking fake listener");
        let record = V5EndpointRecord::new(
            CoreIdentity::production_v5(),
            listener.local_addr().unwrap().port(),
        )
        .unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sessions = Arc::new(AtomicUsize::new(0));
        let script = Arc::new(Mutex::new(script));
        let thread_record = record.clone();
        let thread_seen = Arc::clone(&seen);
        let thread_sessions = Arc::clone(&sessions);
        let thread = thread::spawn(move || {
            let started = Instant::now();
            // Every session runs on its own thread: the anchor session stays
            // silent for the whole test while peers come and go beside it.
            while started.elapsed() < Duration::from_secs(20) {
                let (stream, _) = match listener.accept() {
                    Ok(accepted) => accepted,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(_) => return,
                };
                let session = thread_sessions.fetch_add(1, Ordering::SeqCst);
                if reject_peers && session > 0 {
                    // Read the handshake request, then close unanswered.
                    let mut reader = BufReader::new(stream);
                    let _ = reader.get_ref().set_nonblocking(false);
                    let _ = read_bounded_v5_request_frame(&mut reader);
                    continue;
                }
                let record = thread_record.clone();
                let seen = Arc::clone(&thread_seen);
                let script = Arc::clone(&script);
                thread::spawn(move || serve_session(stream, record, seen, script, handshake_delay));
            }
        });
        Self {
            workspace: tempfile::tempdir().expect("fake daemon workspace"),
            record,
            seen,
            sessions,
            thread: Some(thread),
        }
    }

    pub fn workspace_hint(&self) -> String {
        unica_workspace::test_support::canonical_path(self.workspace.path())
            .to_str()
            .expect("fake daemon workspace path")
            .to_owned()
    }

    pub fn owner(&self) -> V5DaemonProcessOwner {
        V5DaemonProcessOwner::connect_before(
            self.record.clone(),
            Instant::now() + Duration::from_secs(5),
        )
        .expect("connect fake daemon anchor")
    }

    pub fn seen(&self) -> Vec<V5ClientRequest> {
        self.seen.lock().unwrap().clone()
    }

    pub fn submissions(&self) -> usize {
        self.seen()
            .iter()
            .filter(|request| matches!(request, V5ClientRequest::SubmitInvocation { .. }))
            .count()
    }
}

impl Drop for FakeDaemon {
    fn drop(&mut self) {
        // The accept loop ends by its own timeout; the router tests never wait for it.
        if let Some(thread) = self.thread.take() {
            drop(thread);
        }
    }
}

pub fn serve_session(
    stream: TcpStream,
    record: V5EndpointRecord,
    seen: Arc<Mutex<Vec<V5ClientRequest>>>,
    script: Arc<Mutex<SessionScript>>,
    handshake_delay: Duration,
) {
    stream.set_nonblocking(false).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(20)))
        .unwrap();
    let mut writer = stream.try_clone().unwrap();
    let mut reader = BufReader::new(stream);
    let hello = read_bounded_v5_request_frame(&mut reader).expect("hello frame");
    let hello = decode_v5_request_frame(hello).expect("strict hello");
    assert!(matches!(hello.request(), V5ClientRequest::Hello { .. }));
    if !handshake_delay.is_zero() {
        thread::sleep(handshake_delay);
    }
    write_frame(&mut writer, &V5HandshakeServerResponse::ready(&record));
    while let Ok(frame) = read_bounded_v5_request_frame(&mut reader) {
        let decoded = decode_v5_request_frame(frame.clone()).expect("strict client frame");
        let request = decoded.into_request();
        seen.lock().unwrap().push(request.clone());
        let mut script = script.lock().unwrap();
        // Bound first: the registry's Rust parser rejects a call on a boxed
        // closure as a `match` scrutinee.
        let keep_serving = match &mut *script {
            SessionScript::Framed(framed) => {
                let step = framed(&frame, &request);
                match step {
                    Step::Reply(response) => {
                        write_frame(&mut writer, &response);
                        true
                    }
                    Step::Close => false,
                }
            }
            SessionScript::Raw(raw_script) => raw_script(&frame, &request, &mut writer),
        };
        if !keep_serving {
            break;
        }
    }
}

pub fn write_frame<T: serde::Serialize>(stream: &mut TcpStream, value: &T) {
    let mut bytes = serde_json::to_vec(value).expect("serialize fake frame");
    bytes.push(b'\n');
    stream.write_all(&bytes).expect("write fake frame");
    stream.flush().expect("flush fake frame");
}

/// The receipt key the daemon itself would derive from a strict submit frame.
pub fn daemon_side_key(raw: &[u8]) -> ReceiptKey {
    decode_v5_request_frame(raw.to_vec())
        .expect("strict submit frame")
        .into_strict_submit(&CoreIdentity::production_v5())
        .expect("strict submission")
        .receipt_key()
        .clone()
}

pub fn direct_receipt(key: ReceiptKey, terminal: ReceiptTerminalOutcome) -> V5ServerResponse {
    let canonical = canonical_v5_terminal(&terminal, unica_bootstrap::MAX_RESPONSE_BUDGET)
        .expect("canonical terminal");
    V5ServerResponse::Invocation {
        outcome: V5InvocationResponse::Direct {
            receipt: V5PendingDirectReceipt::new(
                key,
                terminal,
                canonical.digest().clone(),
                epoch_ms_now(),
            ),
        },
    }
}

pub fn completed(summary: &str) -> ReceiptTerminalOutcome {
    ReceiptTerminalOutcome::Completed {
        result: Box::new(DomainResult::success(summary)),
    }
}

// --- the real protocol-v5 runtime in a thread -----------------------------
