//! Original native listener/session cases, using the real transport and receipt stores.
//! The typed fixture host rejects accidental product admission: these cases only exchange transport frames.
use super::*;
use crate::protocol_v5::{
    decode_v5_server_response, read_bounded_v5_probe_response_frame, V5ProbeResponseKind,
};
use serde_json::json;
use std::io::{BufRead, Read};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use unica_execution::invocation::InvocationResponseDeadline;
use unica_execution::invocation_store::SystemEpochMillisClock;
use unica_execution::receipt_runtime::hooks::{NoHooks, V5RuntimeHooks};
use unica_execution::receipt_runtime::ports::{
    BoundInvocation, ExecutionClass, PreparedInvocation, V5CanonicalPrepareError,
};
use unica_execution_contract::invocation::{DomainResult, InvocationFailure, SafeIdentityHash};
use unica_work_control::cancellation::CancellationToken;

struct TransportOnlyHost;
struct UnexpectedInvocation;

impl InvocationHost for TransportOnlyHost {
    const MAX_SUPPORTED_BODY_RESPONSE_BYTES: usize = 73_728;

    type Request = V5InvocationRequest;
    type Bound = UnexpectedInvocation;
    type Prepared = UnexpectedInvocation;

    fn has_retained_workspace_work(&self) -> bool {
        false
    }
    fn capture_response_deadline(&self, _: u64) -> InvocationResponseDeadline {
        panic!("transport-only fixture must never capture an invocation deadline")
    }
    fn bind(
        &self,
        _: Self::Request,
        _: InvocationResponseDeadline,
        _: &CancellationToken,
    ) -> Result<Self::Bound, V5CanonicalPrepareError> {
        panic!("transport-only fixture must never admit a product invocation")
    }
    fn discover_join_workspace_root(&self, _: &str) -> Result<PathBuf, String> {
        panic!("transport-only fixture must never inspect join sources")
    }
    fn discover_join_source_set_paths(&self, _: &Path) -> Result<Vec<String>, String> {
        panic!("transport-only fixture must never inspect join sources")
    }
}

impl BoundInvocation for UnexpectedInvocation {
    type Prepared = Self;
    fn response_deadline(&self) -> Option<InvocationResponseDeadline> {
        panic!("no invocation")
    }
    fn workspace_identity_hash(&self) -> &SafeIdentityHash {
        panic!("no invocation")
    }
    fn prepare(self) -> Result<Self::Prepared, Box<DomainResult>> {
        panic!("no invocation")
    }
}

impl PreparedInvocation for UnexpectedInvocation {
    fn execution_class(&self) -> &ExecutionClass {
        panic!("no invocation")
    }
    fn execute(self, _: CancellationToken) -> Result<DomainResult, InvocationFailure> {
        panic!("no invocation")
    }
}

struct FixtureObservation;
impl ObservationLifecycle for FixtureObservation {
    type Guard = ();
    fn start(
        &self,
        _: &DaemonStateDirectory,
        _: Arc<dyn Fn() -> bool + Send + Sync>,
    ) -> io::Result<()> {
        Ok(())
    }
}

type TestConfig = DaemonServerConfig<TransportOnlyHost, FixtureObservation>;
fn test_config(
    state_root: PathBuf,
    core_identity: CoreIdentity,
    idle_grace: Duration,
) -> TestConfig {
    let runtime = ReceiptRuntimeConfig::new(
        core_identity.digest().clone(),
        Arc::new(|| Arc::new(TransportOnlyHost)),
        Arc::new(SystemEpochMillisClock),
        Arc::new(NoHooks),
        true,
    );
    DaemonServerConfig {
        state_root,
        core_identity,
        idle_grace,
        runtime,
        observation: FixtureObservation,
        lsp: Arc::new(crate::lsp_lane::NoLspHost),
    }
}

trait TestHookBinding {
    fn with_runtime_hooks_for_test(self, hooks: Arc<dyn V5RuntimeHooks<TransportOnlyHost>>)
        -> Self;
}
impl TestHookBinding for TestConfig {
    fn with_runtime_hooks_for_test(
        self,
        hooks: Arc<dyn V5RuntimeHooks<TransportOnlyHost>>,
    ) -> Self {
        let runtime = ReceiptRuntimeConfig::new(
            self.core_identity.digest().clone(),
            Arc::new(|| Arc::new(TransportOnlyHost)),
            Arc::new(SystemEpochMillisClock),
            hooks,
            true,
        );
        Self { runtime, ..self }
    }
}

fn write_json_line(stream: &mut TcpStream, value: &serde_json::Value) {
    let mut bytes = serde_json::to_vec(value).expect("serialize v5 frame");
    bytes.push(b'\n');
    stream.write_all(&bytes).expect("write v5 frame");
}

fn wait_for_v5_record(
    state_root: &std::path::Path,
    core_identity: &CoreIdentity,
) -> V5EndpointRecord {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let state = DaemonStateDirectory::open(state_root, core_identity)
            .expect("open v5 daemon state while waiting");
        if let Some(record) = state
            .read_v5_endpoint_record()
            .expect("read v5 endpoint record")
        {
            return record;
        }
        assert!(Instant::now() < deadline, "v5 endpoint was not published");
        thread::sleep(Duration::from_millis(5));
    }
}

fn connect_v5_owner(
    record: &V5EndpointRecord,
    identity: &CoreIdentity,
    owner_lease: &str,
) -> (TcpStream, BufReader<TcpStream>) {
    let mut stream = TcpStream::connect(record.loopback_addr().expect("v5 loopback address"))
        .expect("connect v5 daemon owner");
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("bound v5 owner response read");
    let mut reader = BufReader::new(stream.try_clone().expect("clone v5 owner stream"));
    write_json_line(
        &mut stream,
        &json!({
            "kind": "hello",
            "protocolVersion": 5,
            "token": record.token(),
            "coreIdentity": identity.as_str(),
            "ownerLease": owner_lease
        }),
    );
    let ready = read_bounded_v5_probe_response_frame(&mut reader).expect("read v5 owner ready");
    assert!(matches!(
        decode_v5_server_response(&ready),
        Ok(V5ServerResponse::Ready { .. })
    ));
    (stream, reader)
}

fn probe_peer_cannot_reply(stream: &TcpStream) -> io::Result<bool> {
    if let Some(error) = stream.take_error()? {
        return match error.kind() {
            io::ErrorKind::ConnectionReset | io::ErrorKind::NotConnected => Ok(true),
            _ => Err(error),
        };
    }
    stream.set_nonblocking(true)?;
    match stream.peek(&mut [0u8; 1]) {
        Ok(0) => Ok(true),
        Ok(_) => Ok(false),
        Err(error) => match error.kind() {
            io::ErrorKind::ConnectionReset | io::ErrorKind::NotConnected => Ok(true),
            io::ErrorKind::WouldBlock => Ok(false),
            _ => Err(error),
        },
    }
}

fn displaced_owner_probe(mut stream: TcpStream, hello: &serde_json::Value) -> io::Result<bool> {
    if let Err(setup_error) = stream.set_read_timeout(Some(Duration::from_secs(1))) {
        // Darwin can reject SO_RCVTIMEO after a real reset. Observe the
        // transport; this does not establish process death or release authority.
        return match probe_peer_cannot_reply(&stream) {
            Ok(true) => Ok(false),
            Ok(false) => Err(setup_error),
            Err(observation_error) => Err(io::Error::new(
                setup_error.kind(),
                format!("probe timeout setup failed: {setup_error}; closure observation failed: {observation_error}"),
            )),
        };
    }
    if serde_json::to_writer(&mut stream, hello).is_ok() && stream.write_all(b"\n").is_ok() {
        let mut reader = BufReader::new(stream.try_clone()?);
        Ok(read_bounded_v5_probe_response_frame(&mut reader).is_ok())
    } else {
        Ok(false)
    }
}

struct HandlerSpawnFailureHooks {
    fail_next: AtomicBool,
    failed: mpsc::Sender<()>,
}

impl V5RuntimeHooks<TransportOnlyHost> for HandlerSpawnFailureHooks {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn spawn_connection_handler(
        &self,
        handler: Box<dyn FnOnce() + Send + 'static>,
    ) -> io::Result<thread::JoinHandle<()>> {
        if self.fail_next.swap(false, Ordering::SeqCst) {
            drop(handler);
            self.failed
                .send(())
                .expect("report actual handler spawn failure");
            return Err(io::Error::from(io::ErrorKind::WouldBlock));
        }
        thread::Builder::new()
            .name("unica-daemon-connection".into())
            .spawn(handler)
    }
}

struct HandlerSpawnDaemon {
    stop: Arc<AtomicBool>,
    streams: Vec<TcpStream>,
    server: Option<thread::JoinHandle<Result<(), String>>>,
}

impl Drop for HandlerSpawnDaemon {
    fn drop(&mut self) {
        for stream in &self.streams {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
        self.stop.store(true, Ordering::SeqCst);
        if let Some(server) = self.server.take() {
            let _ = server.join();
        }
    }
}

#[test]
fn authenticated_release_closes_only_its_owner_session() {
    let root = tempfile::tempdir().expect("temporary release state root");
    let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
    let identity = CoreIdentity::production_v5();
    let config = test_config(
        state_root.clone(),
        identity.clone(),
        Duration::from_millis(500),
    );
    let server = thread::spawn(move || run_daemon(config));
    let record = wait_for_v5_record(&state_root, &identity);
    let mut stream = TcpStream::connect(record.loopback_addr().expect("loopback address"))
        .expect("connect release owner");
    let mut reader = BufReader::new(stream.try_clone().expect("clone release stream"));
    write_json_line(
        &mut stream,
        &json!({
            "kind": "hello",
            "protocolVersion": 5,
            "token": record.token(),
            "coreIdentity": identity.as_str(),
            "ownerLease": "77777777-7777-4777-8777-777777777777"
        }),
    );
    read_bounded_v5_probe_response_frame(&mut reader).expect("read release ready");

    write_json_line(&mut stream, &json!({"kind": "release"}));
    let released =
        read_bounded_v5_probe_response_frame(&mut reader).expect("read release response");
    let released = decode_v5_server_response(&released).expect("decode release response");
    drop(stream);

    let mut successor = TcpStream::connect(record.loopback_addr().expect("loopback address"))
        .expect("release must leave the daemon listener available");
    successor
        .set_read_timeout(Some(Duration::from_secs(1)))
        .expect("bound successor session read");
    let mut successor_reader =
        BufReader::new(successor.try_clone().expect("clone successor stream"));
    write_json_line(
        &mut successor,
        &json!({
            "kind": "hello",
            "protocolVersion": 5,
            "token": record.token(),
            "coreIdentity": identity.as_str(),
            "ownerLease": "88888888-8888-4888-8888-888888888888"
        }),
    );
    let successor_ready = read_bounded_v5_probe_response_frame(&mut successor_reader)
        .expect("released owner must not close successor admission");
    let successor_ready: V5HandshakeServerResponse =
        serde_json::from_slice(&successor_ready).expect("decode successor ready response");
    assert!(successor_ready.matches_record(&record));
    write_json_line(&mut successor, &json!({"kind": "ping"}));
    let successor_pong = read_bounded_v5_probe_response_frame(&mut successor_reader)
        .expect("successor session ping");
    let successor_pong: V5ProbeServerResponse =
        serde_json::from_slice(&successor_pong).expect("decode successor pong");
    assert_eq!(successor_pong.kind(), V5ProbeResponseKind::Pong);
    write_json_line(&mut successor, &json!({"kind": "release"}));
    let successor_released = read_bounded_v5_probe_response_frame(&mut successor_reader)
        .expect("release successor session");
    assert_eq!(
        decode_v5_server_response(&successor_released),
        Ok(V5ServerResponse::Released)
    );
    drop(successor);

    server
        .join()
        .expect("join released v5 runtime")
        .expect("released v5 runtime");

    assert_eq!(released, V5ServerResponse::Released);
}

#[test]
fn v5_owner_session_accepts_ping_then_release() {
    let root = tempfile::tempdir().expect("temporary session state root");
    let state_root = std::fs::canonicalize(root.path()).expect("physical session state root");
    let identity = CoreIdentity::production_v5();
    let config = test_config(
        state_root.clone(),
        identity.clone(),
        Duration::from_millis(100),
    );
    let server = thread::spawn(move || run_daemon(config));
    let record = wait_for_v5_record(&state_root, &identity);
    let (mut stream, mut reader) =
        connect_v5_owner(&record, &identity, "44444444-4444-4444-8444-444444444444");

    write_json_line(&mut stream, &json!({"kind": "ping"}));
    let pong = read_bounded_v5_probe_response_frame(&mut reader).expect("read v5 pong");
    assert_eq!(decode_v5_server_response(&pong), Ok(V5ServerResponse::Pong));

    write_json_line(&mut stream, &json!({"kind": "release"}));
    let released =
        read_bounded_v5_probe_response_frame(&mut reader).expect("read v5 release response");
    assert_eq!(
        decode_v5_server_response(&released),
        Ok(V5ServerResponse::Released)
    );
    drop(stream);

    server
        .join()
        .expect("join v5 session runtime")
        .expect("v5 session runtime");
}

#[test]
fn handler_spawn_failure_preserves_live_owner_listener_and_next_connection() {
    use std::io::Read;

    let root = tempfile::tempdir().expect("temporary handler-spawn state root");
    let state_root = std::fs::canonicalize(root.path()).unwrap();
    let identity = CoreIdentity::production_v5();
    let (failed, failed_wait) = mpsc::channel();
    let hooks = Arc::new(HandlerSpawnFailureHooks {
        fail_next: AtomicBool::new(false),
        failed,
    });
    let config = test_config(
        state_root.clone(),
        identity.clone(),
        Duration::from_millis(80),
    )
    .with_runtime_hooks_for_test(hooks.clone());
    let stop = Arc::new(AtomicBool::new(false));
    let server_stop = Arc::clone(&stop);
    let server = thread::spawn(move || {
        run_daemon_configured_until(
            config,
            |runtime| runtime,
            || server_stop.load(Ordering::SeqCst),
        )
    });
    let mut daemon = HandlerSpawnDaemon {
        stop,
        streams: Vec::new(),
        server: Some(server),
    };
    let record = wait_for_v5_record(&state_root, &identity);
    let (mut owner, mut owner_reader) =
        connect_v5_owner(&record, &identity, &uuid::Uuid::new_v4().to_string());
    daemon.streams.push(owner.try_clone().unwrap());

    hooks.fail_next.store(true, Ordering::SeqCst);
    let mut rejected = TcpStream::connect(record.loopback_addr().unwrap()).unwrap();
    rejected
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    daemon.streams.push(rejected.try_clone().unwrap());
    failed_wait
        .recv_timeout(Duration::from_secs(2))
        .expect("the accepted connection reached the failing spawn");
    let closed = rejected.read(&mut [0_u8]);
    assert!(
        matches!(closed, Ok(0))
            || matches!(closed, Err(ref error) if matches!(error.kind(), io::ErrorKind::ConnectionReset | io::ErrorKind::ConnectionAborted)),
        "failed spawn left its transport open: {closed:?}"
    );

    write_json_line(&mut owner, &json!({"kind": "ping"}));
    let pong = read_bounded_v5_probe_response_frame(&mut owner_reader)
        .expect("existing owner remains responsive");
    assert_eq!(decode_v5_server_response(&pong), Ok(V5ServerResponse::Pong));
    assert!(
        !daemon.server.as_ref().unwrap().is_finished(),
        "handler spawn failure terminated the listener"
    );

    let (mut successor, mut successor_reader) =
        connect_v5_owner(&record, &identity, &uuid::Uuid::new_v4().to_string());
    daemon.streams.push(successor.try_clone().unwrap());
    write_json_line(&mut successor, &json!({"kind": "ping"}));
    let pong = read_bounded_v5_probe_response_frame(&mut successor_reader)
        .expect("next accepted owner remains responsive");
    assert_eq!(decode_v5_server_response(&pong), Ok(V5ServerResponse::Pong));
    let state = DaemonStateDirectory::open(&state_root, &identity).unwrap();
    let current = state
        .read_v5_endpoint_record()
        .unwrap()
        .expect("same listener remains published");
    assert_eq!(current.instance_id(), record.instance_id());
    assert_eq!(
        current.loopback_addr().unwrap(),
        record.loopback_addr().unwrap()
    );

    for (stream, reader) in [
        (&mut successor, &mut successor_reader),
        (&mut owner, &mut owner_reader),
    ] {
        write_json_line(stream, &json!({"kind": "release"}));
        let released =
            read_bounded_v5_probe_response_frame(reader).expect("release admitted owner");
        assert_eq!(
            decode_v5_server_response(&released),
            Ok(V5ServerResponse::Released)
        );
    }
    let stopped_before = Instant::now() + Duration::from_secs(5);
    while !daemon.server.as_ref().unwrap().is_finished() {
        assert!(
            Instant::now() < stopped_before,
            "failed spawn retained admission ownership after both owners released"
        );
        thread::sleep(Duration::from_millis(5));
    }
    daemon
        .server
        .take()
        .unwrap()
        .join()
        .expect("listener did not panic")
        .expect("listener shut down after releasing both owners");
    assert!(state.read_v5_endpoint_record().unwrap().is_none());
}

#[test]
fn v5_duplicate_live_owner_lease_is_rejected() {
    let root = tempfile::tempdir().expect("temporary duplicate-lease state root");
    let state_root =
        std::fs::canonicalize(root.path()).expect("physical duplicate-lease state root");
    let identity = CoreIdentity::production_v5();
    let config = test_config(
        state_root.clone(),
        identity.clone(),
        Duration::from_millis(100),
    );
    let server = thread::spawn(move || run_daemon(config));
    let record = wait_for_v5_record(&state_root, &identity);
    let lease = "55555555-5555-4555-8555-555555555555";
    let (mut first, mut first_reader) = connect_v5_owner(&record, &identity, lease);

    let mut duplicate = TcpStream::connect(record.loopback_addr().expect("v5 loopback address"))
        .expect("connect duplicate v5 owner");
    duplicate
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("bound duplicate response read");
    let mut duplicate_reader =
        BufReader::new(duplicate.try_clone().expect("clone duplicate stream"));
    write_json_line(
        &mut duplicate,
        &json!({
            "kind": "hello",
            "protocolVersion": 5,
            "token": record.token(),
            "coreIdentity": identity.as_str(),
            "ownerLease": lease
        }),
    );
    let response = read_bounded_v5_probe_response_frame(&mut duplicate_reader)
        .expect("read duplicate owner rejection");
    assert_eq!(
        decode_v5_server_response(&response),
        Ok(V5ServerResponse::Error {
            code: V5DaemonErrorCode::DuplicateLease,
        })
    );

    write_json_line(&mut first, &json!({"kind": "release"}));
    let released =
        read_bounded_v5_probe_response_frame(&mut first_reader).expect("release original v5 owner");
    assert_eq!(
        decode_v5_server_response(&released),
        Ok(V5ServerResponse::Released)
    );
    drop(first);
    drop(duplicate);

    server
        .join()
        .expect("join duplicate-lease runtime")
        .expect("duplicate-lease runtime");
}

#[test]
fn connection_ownership_survives_former_limit_and_checked_overflow() {
    let admitted = Arc::new(AtomicUsize::new(0));
    let mut slots = (0..65)
        .map(|_| V5ConnectionSlot::acquire(Arc::clone(&admitted)).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(admitted.load(Ordering::Acquire), 65);
    let held = slots.pop().unwrap();
    drop(slots);
    assert_eq!(admitted.load(Ordering::Acquire), 1);
    drop(held);
    assert_eq!(admitted.load(Ordering::Acquire), 0);

    let maximum = Arc::new(AtomicUsize::new(usize::MAX));
    assert!(V5ConnectionSlot::acquire(Arc::clone(&maximum)).is_none());
    assert_eq!(maximum.load(Ordering::Acquire), usize::MAX);
}

#[test]
fn live_v5_owner_prevents_idle_listener_shutdown() {
    let root = tempfile::tempdir().expect("temporary owner-idle state root");
    let state_root = std::fs::canonicalize(root.path()).expect("physical owner-idle state root");
    let identity = CoreIdentity::production_v5();
    let config = test_config(
        state_root.clone(),
        identity.clone(),
        Duration::from_millis(80),
    );
    let server = thread::spawn(move || run_daemon(config));
    let record = wait_for_v5_record(&state_root, &identity);
    let (mut first, mut first_reader) =
        connect_v5_owner(&record, &identity, "66666666-6666-4666-8666-666666666666");

    thread::sleep(Duration::from_millis(200));
    let (mut successor, mut successor_reader) =
        connect_v5_owner(&record, &identity, "77777777-7777-4777-8777-777777777777");
    write_json_line(&mut successor, &json!({"kind": "release"}));
    let successor_released = read_bounded_v5_probe_response_frame(&mut successor_reader)
        .expect("release successor v5 owner");
    assert_eq!(
        decode_v5_server_response(&successor_released),
        Ok(V5ServerResponse::Released)
    );
    drop(successor);

    write_json_line(&mut first, &json!({"kind": "release"}));
    let first_released = read_bounded_v5_probe_response_frame(&mut first_reader)
        .expect("release original idle-fencing owner");
    assert_eq!(
        decode_v5_server_response(&first_released),
        Ok(V5ServerResponse::Released)
    );
    drop(first);

    server
        .join()
        .expect("join owner-idle runtime")
        .expect("owner-idle runtime");
}

/// The first frame is read before the token is checked and keeps the small
/// handshake bound; an authenticated session reads far larger frames (#1309).
#[test]
fn handshake_keeps_its_small_bound_while_an_authenticated_frame_may_be_far_larger() {
    use crate::protocol_v5::{MAX_V5_HANDSHAKE_LINE_BYTES, MAX_V5_REQUEST_LINE_BYTES};
    // JSON whitespace pads a frame without changing its meaning.
    fn padded_line(value: &serde_json::Value, line_bytes: usize) -> Vec<u8> {
        let mut bytes = serde_json::to_vec(value).expect("serialize v5 frame");
        bytes.resize(line_bytes - 1, b' ');
        bytes.push(b'\n');
        bytes
    }
    let root = tempfile::tempdir().expect("temporary state root");
    let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
    let identity = CoreIdentity::production_v5();
    let config = test_config(
        state_root.clone(),
        identity.clone(),
        Duration::from_millis(80),
    );
    let server = thread::spawn(move || run_daemon(config));
    let record = wait_for_v5_record(&state_root, &identity);
    let hello = json!({
        "kind": "hello",
        "protocolVersion": 5,
        "token": record.token(),
        "coreIdentity": identity.as_str(),
        "ownerLease": "44444444-4444-4444-8444-444444444444"
    });
    let connect = || {
        let stream = TcpStream::connect(record.loopback_addr().expect("v5 loopback address"))
            .expect("connect v5 daemon");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("bound v5 response read");
        let reader = BufReader::new(stream.try_clone().expect("clone v5 stream"));
        (stream, reader)
    };

    // One byte above the handshake bound: every byte is consumed before the
    // refusal, so the answer is not lost to a reset.
    let (mut stream, mut reader) = connect();
    stream
        .write_all(&padded_line(&hello, MAX_V5_HANDSHAKE_LINE_BYTES + 1))
        .expect("write oversized handshake");
    let refused = read_bounded_v5_probe_response_frame(&mut reader).expect("read refusal");
    assert_eq!(
        decode_v5_server_response(&refused),
        Ok(V5ServerResponse::Error {
            code: V5DaemonErrorCode::InvalidRequest
        })
    );
    drop((stream, reader));

    let (mut stream, mut reader) = connect();
    write_json_line(&mut stream, &hello);
    read_bounded_v5_probe_response_frame(&mut reader).expect("read v5 ready");
    let large = MAX_V5_HANDSHAKE_LINE_BYTES * 64;
    assert!(large < MAX_V5_REQUEST_LINE_BYTES);
    stream
        .write_all(&padded_line(&json!({"kind": "ping"}), large))
        .expect("write large authenticated frame");
    let pong = read_bounded_v5_probe_response_frame(&mut reader).expect("read v5 pong");
    let pong: V5ProbeServerResponse = serde_json::from_slice(&pong).expect("decode strict v5 pong");
    assert_eq!(pong.kind(), V5ProbeResponseKind::Pong);
    write_json_line(&mut stream, &json!({"kind": "release"}));
    read_bounded_v5_probe_response_frame(&mut reader).expect("read v5 release response");
    drop((stream, reader));
    server.join().expect("join v5 runtime").expect("v5 runtime");
}

/// A large frame that begins just before an idle session poll expires keeps
/// its consumed prefix: the frame has its own deadline from its first byte,
/// and its tail is never read as a new request (#1309).
#[test]
fn a_frame_straddling_the_idle_session_poll_is_read_whole() {
    let root = tempfile::tempdir().expect("temporary state root");
    let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
    let identity = CoreIdentity::production_v5();
    let config = test_config(
        state_root.clone(),
        identity.clone(),
        Duration::from_millis(80),
    );
    let server = thread::spawn(move || run_daemon(config));
    let record = wait_for_v5_record(&state_root, &identity);
    let mut stream = TcpStream::connect(record.loopback_addr().expect("v5 loopback address"))
        .expect("connect v5 daemon");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("bound v5 response read");
    let mut reader = BufReader::new(stream.try_clone().expect("clone v5 stream"));
    write_json_line(
        &mut stream,
        &json!({
            "kind": "hello",
            "protocolVersion": 5,
            "token": record.token(),
            "coreIdentity": identity.as_str(),
            "ownerLease": "55555555-5555-4555-8555-555555555555"
        }),
    );
    read_bounded_v5_probe_response_frame(&mut reader).expect("read v5 ready");
    let ready_at = Instant::now();

    let mut frame = serde_json::to_vec(&json!({"kind": "ping"})).expect("serialize ping");
    frame.resize(1024 * 1024, b' ');
    frame.push(b'\n');
    let parts: Vec<&[u8]> = frame.chunks(frame.len() / 4 + 1).collect();
    // The first part lands shortly before the idle poll ends, the rest after.
    thread::sleep(SESSION_READ_TIMEOUT.saturating_sub(Duration::from_millis(500)));
    for part in parts {
        stream.write_all(part).expect("write frame part");
        thread::sleep(Duration::from_millis(250));
    }
    assert!(ready_at.elapsed() > SESSION_READ_TIMEOUT);
    let answer = read_bounded_v5_probe_response_frame(&mut reader).expect("read v5 answer");
    let answer: V5ProbeServerResponse =
        serde_json::from_slice(&answer).expect("decode strict v5 answer");
    assert_eq!(answer.kind(), V5ProbeResponseKind::Pong);
    write_json_line(&mut stream, &json!({"kind": "release"}));
    read_bounded_v5_probe_response_frame(&mut reader).expect("read v5 release response");
    drop((stream, reader));
    server.join().expect("join v5 runtime").expect("v5 runtime");
}

/// A session frame that misses its own deadline ends the session: its tail
/// is never answered as another request.
#[test]
fn a_session_frame_missing_its_deadline_closes_the_session() {
    let root = tempfile::tempdir().expect("temporary state root");
    let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
    let identity = CoreIdentity::production_v5();
    let config = test_config(
        state_root.clone(),
        identity.clone(),
        Duration::from_millis(80),
    );
    let server = thread::spawn(move || run_daemon(config));
    let record = wait_for_v5_record(&state_root, &identity);
    let mut stream = TcpStream::connect(record.loopback_addr().expect("v5 loopback address"))
        .expect("connect v5 daemon");
    stream
        .set_read_timeout(Some(SESSION_FRAME_READ_TIMEOUT + Duration::from_secs(3)))
        .expect("bound v5 response read");
    let mut reader = BufReader::new(stream.try_clone().expect("clone v5 stream"));
    write_json_line(
        &mut stream,
        &json!({
            "kind": "hello",
            "protocolVersion": 5,
            "token": record.token(),
            "coreIdentity": identity.as_str(),
            "ownerLease": "66666666-6666-4666-8666-666666666666"
        }),
    );
    read_bounded_v5_probe_response_frame(&mut reader).expect("read v5 ready");
    // `{"kind":"ping"}` split so that its tail alone is a valid frame.
    stream.write_all(b"{\"a\":").expect("write frame head");
    thread::sleep(SESSION_FRAME_READ_TIMEOUT + Duration::from_millis(300));
    let _ = stream.write_all(b"1}\n{\"kind\":\"ping\"}\n");
    let mut rest = Vec::new();
    let _ = reader.read_to_end(&mut rest);
    assert!(
        rest.is_empty(),
        "an expired frame must close the session without an answer ({} bytes)",
        rest.len()
    );
    drop((stream, reader));
    server.join().expect("join v5 runtime").expect("v5 runtime");
}

#[test]
fn direct_runtime_entry_rejects_every_non_v5_identity_before_state_creation() {
    use std::str::FromStr;

    for identity in [
        CoreIdentity::from_str("2f4dd5713d11e5211a92c5fa01b1ec5722dc3a3160b9b1e0b667f8d8da3d9c28")
            .expect("the retired protocol-v3 digest still parses as a canonical identity"),
        CoreIdentity::from_str("eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee")
            .expect("arbitrary accepted identity"),
    ] {
        let root = tempfile::tempdir().expect("temporary state root");
        let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
        let result = run_daemon(test_config(state_root, identity, Duration::from_millis(10)));

        assert_eq!(
            result,
            Err("protocol-v5 runtime requires the exact production-v5 core identity".to_string())
        );
        assert_eq!(
            std::fs::read_dir(root.path())
                .expect("read untouched root")
                .count(),
            0
        );
    }
}

#[test]
fn partial_handshake_bytes_cannot_replenish_the_absolute_frame_deadline() {
    let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .expect("bind slowloris fixture");
    let address = listener.local_addr().expect("slowloris address");
    let (done_tx, done_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept slowloris fixture");
        stream
            .set_nonblocking(false)
            .expect("blocking fixture stream");
        let mut reader = BufReader::new(stream);
        let started = Instant::now();
        let result = read_v5_request_before(&mut reader, started + Duration::from_millis(60));
        done_tx
            .send((result.is_err(), started.elapsed()))
            .expect("report bounded read");
    });
    let mut client = TcpStream::connect(address).expect("connect slowloris fixture");
    for byte in b"{\"kind\":\"ping\"}\n" {
        if client.write_all(&[*byte]).is_err() {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    let (rejected, elapsed) = done_rx
        .recv_timeout(Duration::from_millis(250))
        .expect("absolute frame deadline must release the reader");
    assert!(rejected);
    assert!(elapsed < Duration::from_millis(180), "elapsed={elapsed:?}");
    server.join().expect("join slowloris fixture");
}

#[test]
fn expired_partial_handshake_closes_transport_without_a_late_protocol_response() {
    let root = tempfile::tempdir().expect("temporary state root");
    let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
    let identity = CoreIdentity::production_v5();
    let config = test_config(
        state_root.clone(),
        identity.clone(),
        HANDSHAKE_READ_TIMEOUT + Duration::from_secs(1),
    );
    let server = thread::spawn(move || run_daemon(config));
    let record = wait_for_v5_record(&state_root, &identity);

    let mut stream = TcpStream::connect(record.loopback_addr().expect("v5 loopback address"))
        .expect("connect v5 daemon");
    stream
        .set_read_timeout(Some(HANDSHAKE_READ_TIMEOUT + Duration::from_secs(1)))
        .expect("bound expired-handshake read");
    stream.write_all(b"{").expect("write partial handshake");
    let started = Instant::now();
    let mut response = Vec::new();
    if let Err(error) = stream.read_to_end(&mut response) {
        assert_eq!(
            error.kind(),
            io::ErrorKind::ConnectionReset,
            "expired handshake must close the transport: {error}"
        );
    }

    assert!(
        response.is_empty(),
        "transport timeout was misclassified as protocol response: {}",
        String::from_utf8_lossy(&response)
    );
    assert!(
        started.elapsed() < HANDSHAKE_READ_TIMEOUT + Duration::from_millis(500),
        "expired handshake received a replenished response budget: {:?}",
        started.elapsed()
    );
    server.join().expect("join v5 runtime").expect("v5 runtime");
}

#[test]
fn complete_v5_frame_near_cutoff_cannot_receive_a_fresh_response_budget() {
    let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .expect("bind response-deadline fixture");
    let address = listener.local_addr().expect("response-deadline address");
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept response-deadline fixture");
        let original_deadline = Instant::now() + Duration::from_millis(30);
        thread::sleep(Duration::from_millis(45));
        let result = write_json_line_before(
            &mut stream,
            &V5ProbeServerResponse::Pong {},
            original_deadline,
        );
        drop(stream);
        result
    });
    let mut client = TcpStream::connect(address).expect("connect response-deadline fixture");
    client
        .set_read_timeout(Some(Duration::from_secs(1)))
        .expect("bound response-deadline read");
    let mut response = Vec::new();
    client
        .read_to_end(&mut response)
        .expect("expired response deadline closes transport");

    assert!(
        server
            .join()
            .expect("join response-deadline fixture")
            .is_err(),
        "expired original deadline granted a new response-write budget"
    );
    assert!(response.is_empty(), "late response escaped: {response:?}");
}

#[test]
fn displaced_owner_probe_rejects_a_connection_reset_by_listener_shutdown() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind backlog probe fixture");
    let stream = TcpStream::connect(listener.local_addr().unwrap()).expect("connect pending probe");
    stream
        .set_read_timeout(Some(Duration::from_secs(1)))
        .expect("bound closure observation");
    drop(listener);
    match stream.peek(&mut [0u8; 1]) {
        Ok(0) => {}
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::ConnectionReset | io::ErrorKind::NotConnected
            ) => {}
        other => panic!("listener shutdown must actually close this pending connection: {other:?}"),
    }
    assert!(
        !displaced_owner_probe(stream, &json!({"kind":"hello"})).expect("probe actual reset"),
        "a physically reset peer cannot become ready"
    );
}

#[test]
fn displaced_owner_probe_observes_a_live_peer_response() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind live probe fixture");
    let stream = TcpStream::connect(listener.local_addr().unwrap()).expect("connect live probe");
    let (accepted, _) = listener.accept().expect("accept live probe");
    let server = thread::spawn(move || {
        let mut reader = BufReader::new(accepted);
        reader
            .get_ref()
            .set_read_timeout(Some(Duration::from_secs(1)))
            .expect("bound fixture hello");
        let mut hello = String::new();
        reader
            .read_line(&mut hello)
            .expect("read actual probe hello");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&hello).unwrap(),
            json!({"kind":"hello"})
        );
        reader
            .get_mut()
            .write_all(b"{\"kind\":\"ready\"}\n")
            .expect("send actual probe response");
    });
    let ready = displaced_owner_probe(stream, &json!({"kind":"hello"}));
    server.join().expect("join actual live probe");
    assert!(
        ready.expect("probe live peer"),
        "live response must not be classified as a closed peer"
    );
}

#[test]
fn probe_peer_without_a_response_or_with_payload_is_not_observed_closed() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind live closure observation");
    let stream = TcpStream::connect(listener.local_addr().unwrap()).expect("connect live peer");
    let (mut accepted, _) = listener.accept().expect("accept live peer");
    assert!(!probe_peer_cannot_reply(&stream).expect("observe idle live peer"));
    stream
        .set_nonblocking(false)
        .expect("restore fixture observation");
    stream
        .set_read_timeout(Some(Duration::from_secs(1)))
        .expect("bound payload observation");
    accepted.write_all(b"x").expect("send actual peer payload");
    assert_eq!(
        stream
            .peek(&mut [0u8; 1])
            .expect("observe delivered payload"),
        1
    );
    assert!(!probe_peer_cannot_reply(&stream).expect("observe live peer payload"));
}
