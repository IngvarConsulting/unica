//! Two daemons for the frontend tests: a scripted fake that answers frame
//! by frame, and the real protocol-v5 runtime in a thread with an injected
//! canonical service.
use super::*;
use crate::infrastructure::daemon::server::DaemonServerConfig;
use std::io::{BufReader, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::thread;
use unica_daemon_protocol::identity::{CoreIdentity, DaemonStateDirectory};
use unica_daemon_protocol::protocol_v5::{
    decode_v5_request_frame, read_bounded_v5_request_frame, V5ClientRequest, V5EndpointRecord,
    V5HandshakeServerResponse,
};
use unica_execution::receipt_ledger::{canonical_v5_terminal, ReceiptTerminalOutcome};
use unica_execution::receipt_runtime::response::V5PendingDirectReceipt;
use unica_execution_contract::invocation::DomainResult;

const WORKSPACE_MANIFEST: &str =
    "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: .\n";

// --- scripted fake daemon -------------------------------------------------

/// One accepted session of the fake: what it saw and what it answered.
pub(in crate::interfaces) enum Step {
    Reply(V5ServerResponse),
    Close,
}

pub(in crate::interfaces) type Script = Box<dyn FnMut(&[u8], &V5ClientRequest) -> Step + Send>;
/// A script that writes the reply bytes itself; `false` closes the session.
pub(in crate::interfaces) type RawScript =
    Box<dyn FnMut(&[u8], &V5ClientRequest, &mut TcpStream) -> bool + Send>;

enum SessionScript {
    Framed(Script),
    Raw(RawScript),
}

pub(in crate::interfaces) struct FakeDaemon {
    workspace: tempfile::TempDir,
    pub(in crate::interfaces) record: V5EndpointRecord,
    seen: Arc<Mutex<Vec<V5ClientRequest>>>,
    pub(in crate::interfaces) sessions: Arc<AtomicUsize>,
    thread: Option<thread::JoinHandle<()>>,
}

impl FakeDaemon {
    pub(in crate::interfaces) fn start(script: Script) -> Self {
        Self::start_with_handshake_delay(script, Duration::ZERO)
    }

    /// Same fake, but every session answers `Hello` only after `delay`:
    /// the frontend cutoff tests spend budget on the handshake on purpose.
    pub(in crate::interfaces) fn start_with_handshake_delay(
        script: Script,
        handshake_delay: Duration,
    ) -> Self {
        Self::start_session_script(SessionScript::Framed(script), handshake_delay)
    }

    pub(in crate::interfaces) fn start_with_raw_script(script: RawScript) -> Self {
        Self::start_session_script(SessionScript::Raw(script), Duration::ZERO)
    }

    /// The anchor session stands; every later session is dropped right
    /// after its handshake request, as a daemon refusing the connection.
    pub(in crate::interfaces) fn start_rejecting_peers() -> Self {
        Self::start_sessions(
            SessionScript::Framed(Box::new(|_, request| {
                panic!("a refused peer sends nothing: {request:?}")
            })),
            Duration::ZERO,
            true,
        )
    }

    fn start_session_script(script: SessionScript, handshake_delay: Duration) -> Self {
        Self::start_sessions(script, handshake_delay, false)
    }

    fn start_sessions(
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

    pub(in crate::interfaces) fn workspace_hint(&self) -> String {
        unica_workspace::test_support::canonical_path(self.workspace.path())
            .to_str()
            .expect("fake daemon workspace path")
            .to_owned()
    }

    pub(in crate::interfaces) fn owner(&self) -> V5DaemonProcessOwner {
        V5DaemonProcessOwner::connect_before(
            self.record.clone(),
            Instant::now() + Duration::from_secs(5),
        )
        .expect("connect fake daemon anchor")
    }

    pub(in crate::interfaces) fn seen(&self) -> Vec<V5ClientRequest> {
        self.seen.lock().unwrap().clone()
    }

    pub(in crate::interfaces) fn submissions(&self) -> usize {
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

fn serve_session(
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

pub(in crate::interfaces) fn write_frame<T: serde::Serialize>(stream: &mut TcpStream, value: &T) {
    let mut bytes = serde_json::to_vec(value).expect("serialize fake frame");
    bytes.push(b'\n');
    stream.write_all(&bytes).expect("write fake frame");
    stream.flush().expect("flush fake frame");
}

/// The receipt key the daemon itself would derive from a strict submit frame.
pub(in crate::interfaces) fn daemon_side_key(raw: &[u8]) -> ReceiptKey {
    decode_v5_request_frame(raw.to_vec())
        .expect("strict submit frame")
        .into_strict_submit(&CoreIdentity::production_v5())
        .expect("strict submission")
        .receipt_key()
        .clone()
}

pub(in crate::interfaces) fn direct_receipt(
    key: ReceiptKey,
    terminal: ReceiptTerminalOutcome,
) -> V5ServerResponse {
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

pub(in crate::interfaces) fn completed(summary: &str) -> ReceiptTerminalOutcome {
    ReceiptTerminalOutcome::Completed {
        result: Box::new(DomainResult::success(summary)),
    }
}

// --- the real protocol-v5 runtime in a thread -----------------------------

pub(in crate::interfaces) struct ScriptedService {
    pub(in crate::interfaces) delay: Duration,
    pub(in crate::interfaces) known_long: bool,
    pub(in crate::interfaces) outcome: Mutex<
        Option<Result<DomainResult, unica_execution_contract::invocation::InvocationFailure>>,
    >,
    pub(in crate::interfaces) executions: AtomicUsize,
}

impl crate::infrastructure::daemon::server::CanonicalInvocationService for ScriptedService {
    fn prepare(
        &self,
        _invocation: &crate::infrastructure::daemon::server::ActorBoundInvocation,
    ) -> Result<unica_execution::receipt_runtime::ports::ExecutionClass, Box<DomainResult>> {
        use unica_execution::receipt_runtime::ports::{ExecutionClass, KnownLongReason};
        Ok(if self.known_long {
            ExecutionClass::KnownLong(KnownLongReason::ExternalProcess)
        } else {
            ExecutionClass::InlineCandidate
        })
    }

    fn execute(
        &self,
        _invocation: &crate::infrastructure::daemon::server::ActorBoundExecution,
        _cancellation: CancellationToken,
    ) -> Result<DomainResult, unica_execution_contract::invocation::InvocationFailure> {
        self.executions.fetch_add(1, Ordering::SeqCst);
        thread::sleep(self.delay);
        self.outcome
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(|| Ok(DomainResult::success("executed")))
    }
}

pub(in crate::interfaces) struct LiveDaemon {
    _state: Option<tempfile::TempDir>,
    _workspace: Option<tempfile::TempDir>,
    pub(in crate::interfaces) workspace_hint: String,
    pub(in crate::interfaces) state_root: std::path::PathBuf,
    thread: Option<thread::JoinHandle<Result<(), String>>>,
    hooks: Option<
        Arc<
            dyn unica_execution::receipt_runtime::hooks::V5RuntimeHooks<
                crate::infrastructure::daemon::server::CanonicalInvocationHost,
            >,
        >,
    >,
    epoch_clock: Option<Arc<dyn unica_execution::invocation_store::EpochMillisClock>>,
}

impl LiveDaemon {
    pub(in crate::interfaces) fn start(
        service: Arc<dyn crate::infrastructure::daemon::server::CanonicalInvocationService>,
    ) -> Self {
        Self::start_with_hooks(service, None)
    }

    pub(in crate::interfaces) fn start_with_hooks(
        service: Arc<dyn crate::infrastructure::daemon::server::CanonicalInvocationService>,
        hooks: Option<
            Arc<
                dyn unica_execution::receipt_runtime::hooks::V5RuntimeHooks<
                    crate::infrastructure::daemon::server::CanonicalInvocationHost,
                >,
            >,
        >,
    ) -> Self {
        let workspace = tempfile::tempdir().unwrap();
        std::fs::write(workspace.path().join("v8project.yaml"), WORKSPACE_MANIFEST).unwrap();
        Self::start_over(workspace, service, hooks, None)
    }

    /// A workspace whose sources live in `src/` and whose files are older
    /// than the join freshness margin by the time the first call starts:
    /// an identical read may join only while nothing changed since the
    /// walk it joins began.
    pub(in crate::interfaces) fn start_with_aged_sources(
        service: Arc<dyn crate::infrastructure::daemon::server::CanonicalInvocationService>,
        hooks: Option<
            Arc<
                dyn unica_execution::receipt_runtime::hooks::V5RuntimeHooks<
                    crate::infrastructure::daemon::server::CanonicalInvocationHost,
                >,
            >,
        >,
    ) -> Self {
        Self::start_with_aged_sources_and_clock(service, hooks, None)
    }

    pub(in crate::interfaces) fn start_with_aged_sources_and_clock(
        service: Arc<dyn crate::infrastructure::daemon::server::CanonicalInvocationService>,
        hooks: Option<
            Arc<
                dyn unica_execution::receipt_runtime::hooks::V5RuntimeHooks<
                    crate::infrastructure::daemon::server::CanonicalInvocationHost,
                >,
            >,
        >,
        epoch_clock: Option<Arc<dyn unica_execution::invocation_store::EpochMillisClock>>,
    ) -> Self {
        let workspace = tempfile::tempdir().unwrap();
        std::fs::write(
                workspace.path().join("v8project.yaml"),
                "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
            )
            .unwrap();
        std::fs::create_dir_all(workspace.path().join("src/CommonModules")).unwrap();
        std::fs::write(
            workspace.path().join("src/CommonModules/Module.bsl"),
            "// v1\n",
        )
        .unwrap();
        thread::sleep(
            unica_execution::receipt_runtime::test_support::TIMESTAMP_MARGIN
                + Duration::from_millis(200),
        );
        Self::start_over(workspace, service, hooks, epoch_clock)
    }

    fn start_over(
        workspace: tempfile::TempDir,
        service: Arc<dyn crate::infrastructure::daemon::server::CanonicalInvocationService>,
        hooks: Option<
            Arc<
                dyn unica_execution::receipt_runtime::hooks::V5RuntimeHooks<
                    crate::infrastructure::daemon::server::CanonicalInvocationHost,
                >,
            >,
        >,
        epoch_clock: Option<Arc<dyn unica_execution::invocation_store::EpochMillisClock>>,
    ) -> Self {
        let state = tempfile::tempdir().unwrap();
        let state_root = std::fs::canonicalize(state.path()).unwrap();
        let workspace_hint = std::fs::canonicalize(workspace.path())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let mut daemon = Self {
            _state: Some(state),
            _workspace: Some(workspace),
            workspace_hint,
            state_root,
            thread: None,
            hooks,
            epoch_clock,
        };
        daemon.restart(service, Duration::from_millis(400));
        daemon
    }

    /// Start (again) the daemon over the same state root: the second life
    /// of a restart scenario reads what the first one left durable.
    pub(in crate::interfaces) fn restart(
        &mut self,
        service: Arc<dyn crate::infrastructure::daemon::server::CanonicalInvocationService>,
        idle_grace: Duration,
    ) {
        assert!(
            self.thread.is_none(),
            "the previous daemon life must be finished first"
        );
        let config = DaemonServerConfig::new(
            self.state_root.clone(),
            CoreIdentity::production_v5(),
            idle_grace,
        )
        .with_invocation_service(service);
        let config = match &self.epoch_clock {
            Some(clock) => config.with_v5_epoch_clock_for_test(clock.clone()),
            None => config,
        };
        let config = match &self.hooks {
            Some(hooks) => config.with_runtime_hooks_for_test(hooks.clone()),
            None => config,
        };
        self.thread = Some(thread::spawn(move || {
            unica_daemon_protocol::transport::run_daemon(config.into_transport_config())
        }));
    }

    pub(in crate::interfaces) fn owner(&self) -> V5DaemonProcessOwner {
        let identity = CoreIdentity::production_v5();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let state = DaemonStateDirectory::open(&self.state_root, &identity).unwrap();
            if let Some(record) = state.read_v5_endpoint_record().unwrap() {
                return V5DaemonProcessOwner::connect_before(record, deadline)
                    .expect("connect live v5 daemon");
            }
            assert!(Instant::now() < deadline, "v5 endpoint was not published");
            thread::sleep(Duration::from_millis(5));
        }
    }

    /// Wait for the daemon to exit on its idle grace; the state root stays.
    pub(in crate::interfaces) fn stop(&mut self) {
        self.thread
            .take()
            .expect("a running daemon life")
            .join()
            .expect("join v5 daemon thread")
            .expect("v5 daemon exits cleanly after its idle grace");
    }

    pub(in crate::interfaces) fn finish(mut self) {
        self.stop();
    }
}
