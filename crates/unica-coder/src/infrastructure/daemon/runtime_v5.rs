use super::identity::{CoreIdentity, DaemonStateDirectory, ReceiptAuthorityLock};
#[cfg(all(feature = "receipt-ledger-test-support", test))]
use super::protocol_v5::read_and_decode_v5_request;
use super::protocol_v5::{
    read_and_decode_v5_request_before, DecodedV5Request, V5ClientRequestKind, V5DaemonErrorCode,
    V5EndpointRecord, V5HandshakeServerResponse, V5ProbeServerResponse, V5RequestFrameError,
    DAEMON_PROTOCOL_VERSION, MAX_V5_RESPONSE_LINE_BYTES,
};
#[cfg(feature = "receipt-ledger-test-support")]
use super::protocol_v5::{
    read_bounded_v5_probe_response_frame, V5ClientRequest, V5InvocationRequest,
};
use super::server::{CanonicalInvocationService, DaemonServerConfig};
use crate::infrastructure::platform::filesystem::RetainedDirectoryCapability;
#[cfg(feature = "receipt-ledger-test-support")]
use crate::infrastructure::receipt_ledger::StableReceiptLedgerObservation;
use crate::infrastructure::receipt_ledger::{MissingReceiptObservation, ReceiptLedgerStore};
#[cfg(feature = "receipt-ledger-test-support")]
use crate::infrastructure::receipt_ledger_test_evidence::ProductionMissingTransitionEvidence;
use serde::Serialize;
use std::io::{self, BufReader, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
#[cfg(feature = "receipt-ledger-test-support")]
use std::sync::mpsc::{sync_channel, SyncSender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

const ACCEPT_POLL_INTERVAL: Duration = Duration::from_millis(10);
const AUTHORITY_ACQUIRE_TIMEOUT: Duration = Duration::from_secs(2);
const HANDSHAKE_READ_TIMEOUT: Duration = Duration::from_secs(2);
const SESSION_READ_TIMEOUT: Duration = Duration::from_secs(2);

struct V5ReceiptRuntime {
    core_identity: CoreIdentity,
    _stable_authority: ReceiptAuthorityLock,
    receipt_ledger: ReceiptLedgerStore,
    #[cfg_attr(not(feature = "receipt-ledger-test-support"), allow(dead_code))]
    invocation_executor: V5InvocationExecutor,
    #[cfg_attr(not(feature = "receipt-ledger-test-support"), allow(dead_code))]
    task_projection: V5TaskProjection,
    #[cfg(feature = "receipt-ledger-test-support")]
    evidence_capture: Option<SyncSender<ProductionMissingTransitionEvidence>>,
}

struct V5TaskProjection {
    #[cfg_attr(not(feature = "receipt-ledger-test-support"), allow(dead_code))]
    task_store_root: RetainedDirectoryCapability,
}

impl V5TaskProjection {
    fn open(state: &DaemonStateDirectory) -> Result<Self, String> {
        Ok(Self {
            task_store_root: state.create_private_retained_subdirectory("tasks")?,
        })
    }

    #[cfg(feature = "receipt-ledger-test-support")]
    fn validate_named_identity(&self) -> Result<(), String> {
        self.task_store_root
            .validate_named_identity()
            .map_err(|error| format!("validate protocol-v5 task projection root: {error}"))
    }

    #[cfg(feature = "receipt-ledger-test-support")]
    fn observe_missing_seed_writer(
        &self,
        observation: StableReceiptLedgerObservation,
    ) -> V5TaskProjectionReachability {
        V5TaskProjectionReachability {
            _task_store_root: self.task_store_root.clone(),
            observation,
        }
    }
}

#[cfg(feature = "receipt-ledger-test-support")]
pub(in crate::infrastructure) struct V5TaskProjectionReachability {
    _task_store_root: RetainedDirectoryCapability,
    observation: StableReceiptLedgerObservation,
}

#[cfg(feature = "receipt-ledger-test-support")]
impl V5TaskProjectionReachability {
    pub(in crate::infrastructure) const fn observation(&self) -> &StableReceiptLedgerObservation {
        &self.observation
    }
}

struct V5InvocationExecutor {
    _invocation_service: Arc<dyn CanonicalInvocationService>,
}

impl V5InvocationExecutor {
    fn new(invocation_service: Arc<dyn CanonicalInvocationService>) -> Self {
        Self {
            _invocation_service: invocation_service,
        }
    }

    #[cfg(feature = "receipt-ledger-test-support")]
    fn observe_missing_writer(
        &self,
        action: V5ExecutorReachabilityAction,
        observation: StableReceiptLedgerObservation,
    ) -> V5ExecutorReachability {
        V5ExecutorReachability {
            action,
            observation,
        }
    }
}

#[cfg(feature = "receipt-ledger-test-support")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::infrastructure) enum V5ExecutorReachabilityAction {
    RunDirectLoad,
    RunLazyCancelStorm,
}

#[cfg(feature = "receipt-ledger-test-support")]
impl V5ExecutorReachabilityAction {
    pub(in crate::infrastructure) const fn wire_name(self) -> &'static str {
        match self {
            Self::RunDirectLoad => "run_direct_load",
            Self::RunLazyCancelStorm => "run_lazy_cancel_storm",
        }
    }
}

#[cfg(feature = "receipt-ledger-test-support")]
pub(in crate::infrastructure) struct V5ExecutorReachability {
    action: V5ExecutorReachabilityAction,
    observation: StableReceiptLedgerObservation,
}

#[cfg(feature = "receipt-ledger-test-support")]
impl V5ExecutorReachability {
    pub(in crate::infrastructure) const fn action(&self) -> V5ExecutorReachabilityAction {
        self.action
    }

    pub(in crate::infrastructure) const fn observation(&self) -> &StableReceiptLedgerObservation {
        &self.observation
    }
}

impl V5ReceiptRuntime {
    fn open(state: &DaemonStateDirectory, config: &DaemonServerConfig) -> Result<Self, String> {
        let stable_authority = state.acquire_receipt_authority(AUTHORITY_ACQUIRE_TIMEOUT)?;
        let receipts = state.create_private_retained_subdirectory("receipts")?;
        let receipt_ledger = ReceiptLedgerStore::open_retained_directory(receipts)
            .map_err(|error| format!("open protocol-v5 receipt ledger: {error}"))?;
        receipt_ledger
            .generation()
            .map_err(|error| format!("read protocol-v5 receipt generation: {error}"))?;
        Ok(Self {
            core_identity: config.core_identity.clone(),
            _stable_authority: stable_authority,
            receipt_ledger,
            invocation_executor: V5InvocationExecutor::new(config.invocation_service_for_v5()),
            task_projection: V5TaskProjection::open(state)?,
            #[cfg(feature = "receipt-ledger-test-support")]
            evidence_capture: None,
        })
    }

    fn ensure_named_authority(&self) -> Result<(), String> {
        self.receipt_ledger
            .generation()
            .map(|_| ())
            .map_err(|error| format!("validate protocol-v5 receipt authority: {error}"))
    }

    fn inspect_strict_submit(
        &self,
        decoded: DecodedV5Request,
    ) -> Result<MissingReceiptObservation, String> {
        let strict = decoded
            .into_strict_submit(&self.core_identity)
            .map_err(|error| format!("derive protocol-v5 receipt identity: {error}"))?;
        self.receipt_ledger
            .inspect_exact(strict.receipt_key_digest())
            .map_err(|error| format!("inspect protocol-v5 receipt: {error}"))
    }

    #[cfg(feature = "receipt-ledger-test-support")]
    fn observe_missing_executor_writer(
        &self,
        action: V5ExecutorReachabilityAction,
    ) -> Result<V5ExecutorReachability, String> {
        self.ensure_named_authority()?;
        let observation = self
            .receipt_ledger
            .observe_stable_generation()
            .map_err(|error| format!("observe protocol-v5 executor receipt generation: {error}"))?;
        Ok(self
            .invocation_executor
            .observe_missing_writer(action, observation))
    }

    #[cfg(feature = "receipt-ledger-test-support")]
    fn observe_missing_task_projection_writer(
        &self,
    ) -> Result<V5TaskProjectionReachability, String> {
        self.ensure_named_authority()?;
        self.task_projection.validate_named_identity()?;
        let observation = self
            .receipt_ledger
            .observe_stable_generation()
            .map_err(|error| {
                format!("observe protocol-v5 task projection receipt generation: {error}")
            })?;
        Ok(self
            .task_projection
            .observe_missing_seed_writer(observation))
    }

    #[cfg(feature = "receipt-ledger-test-support")]
    fn with_evidence_capture(
        mut self,
        capture: SyncSender<ProductionMissingTransitionEvidence>,
    ) -> Self {
        self.evidence_capture = Some(capture);
        self
    }

    #[cfg(feature = "receipt-ledger-test-support")]
    fn capture_protocol_transition_after_frame(
        &self,
        decoded: &DecodedV5Request,
    ) -> Result<(), String> {
        let Some(capture) = &self.evidence_capture else {
            return Ok(());
        };
        self.ensure_named_authority()?;
        let evidence = ProductionMissingTransitionEvidence::protocol_behavior_unavailable(decoded);
        capture
            .try_send(evidence)
            .map_err(|_| "capture protocol-v5 reachability evidence".to_string())
    }

    #[cfg(feature = "receipt-ledger-test-support")]
    fn capture_missing_receipt(
        &self,
        observation: MissingReceiptObservation,
    ) -> Result<(), String> {
        let Some(capture) = &self.evidence_capture else {
            return Ok(());
        };
        self.ensure_named_authority()?;
        let evidence = ProductionMissingTransitionEvidence::receipt_row_absent(observation);
        capture
            .try_send(evidence)
            .map_err(|_| "capture protocol-v5 receipt evidence".to_string())
    }
}

pub(crate) fn run_daemon(config: DaemonServerConfig) -> Result<(), String> {
    run_daemon_configured(config, |runtime| runtime)
}

fn run_daemon_configured(
    config: DaemonServerConfig,
    configure_runtime: impl FnOnce(V5ReceiptRuntime) -> V5ReceiptRuntime,
) -> Result<(), String> {
    if config.core_identity != CoreIdentity::production_v5() {
        return Err(
            "protocol-v5 runtime requires the exact production-v5 core identity".to_string(),
        );
    }
    if config.idle_grace.is_zero() {
        return Err("daemon idle grace must be positive".to_string());
    }

    let state = DaemonStateDirectory::open(&config.state_root, &config.core_identity)?;
    if let Some(existing) = state.read_v5_endpoint_record()? {
        if existing.core_identity() != &config.core_identity {
            return Err("v5 daemon endpoint belongs to a foreign core identity".to_string());
        }
    }
    // Receipt ownership and the initial durable generation are established before
    // a listener can become discoverable.
    let runtime = configure_runtime(V5ReceiptRuntime::open(&state, &config)?);
    runtime.ensure_named_authority()?;
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .map_err(|error| daemon_io_error("bind protocol-v5 loopback endpoint", error))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| daemon_io_error("configure protocol-v5 listener", error))?;
    let port = listener
        .local_addr()
        .map_err(|error| daemon_io_error("inspect protocol-v5 listener", error))?
        .port();
    let record = V5EndpointRecord::new(config.core_identity.clone(), port)?;
    let published = state.publish_v5_endpoint_record(&record)?;
    if let Err(error) = runtime.ensure_named_authority() {
        let _ = state.remove_v5_endpoint_if_owned(&published);
        return Err(error);
    }
    let mut idle_since = Instant::now();

    loop {
        if let Err(error) = runtime.ensure_named_authority() {
            let _ = state.remove_v5_endpoint_if_owned(&published);
            return Err(error);
        }
        match listener.accept() {
            Ok((stream, address)) if address.ip().is_loopback() => {
                if let Err(error) = runtime.ensure_named_authority() {
                    drop(stream);
                    let _ = state.remove_v5_endpoint_if_owned(&published);
                    return Err(error);
                }
                idle_since = Instant::now();
                // This W0a shell deliberately handles one bounded probe synchronously. Full
                // owner/session concurrency and invocation traffic remain separate W0b work.
                let _ = handle_probe_connection(stream, &record, &runtime);
            }
            Ok((_stream, _)) => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                if idle_since.elapsed() >= config.idle_grace {
                    break;
                }
                thread::sleep(ACCEPT_POLL_INTERVAL);
            }
            Err(error) => {
                let _ = state.remove_v5_endpoint_if_owned(&published);
                return Err(daemon_io_error("accept protocol-v5 connection", error));
            }
        }
    }

    drop(listener);
    state.remove_v5_endpoint_if_owned(&published)?;
    Ok(())
}

fn handle_probe_connection(
    mut stream: TcpStream,
    record: &V5EndpointRecord,
    runtime: &V5ReceiptRuntime,
) -> Result<(), String> {
    runtime.ensure_named_authority()?;
    stream
        .set_nonblocking(false)
        .map_err(|error| daemon_io_error("configure protocol-v5 client stream", error))?;
    let reader_stream = stream
        .try_clone()
        .map_err(|error| daemon_io_error("clone protocol-v5 client stream", error))?;
    let mut reader = BufReader::new(reader_stream);
    let handshake_deadline = Instant::now() + HANDSHAKE_READ_TIMEOUT;
    let decoded = match read_v5_request_before(&mut reader, handshake_deadline) {
        Ok(decoded) => decoded,
        Err(V5RequestFrameError::InvalidRequest(_)) => {
            write_runtime_probe_error_before(
                &mut stream,
                runtime,
                V5DaemonErrorCode::InvalidRequest,
                handshake_deadline,
            )?;
            return Ok(());
        }
        Err(V5RequestFrameError::Read(_)) => return Ok(()),
    };
    let Some((protocol_version, token, core_identity, _owner_lease)) =
        decoded.request().hello_parts()
    else {
        write_runtime_probe_error_before(
            &mut stream,
            runtime,
            V5DaemonErrorCode::HandshakeRequired,
            handshake_deadline,
        )?;
        return Ok(());
    };
    if protocol_version != DAEMON_PROTOCOL_VERSION {
        write_runtime_probe_error_before(
            &mut stream,
            runtime,
            V5DaemonErrorCode::ProtocolMismatch,
            handshake_deadline,
        )?;
        return Ok(());
    }
    if core_identity != record.core_identity() {
        write_runtime_probe_error_before(
            &mut stream,
            runtime,
            V5DaemonErrorCode::CoreMismatch,
            handshake_deadline,
        )?;
        return Ok(());
    }
    if !tokens_equal(token, record.token()) {
        write_runtime_probe_error_before(
            &mut stream,
            runtime,
            V5DaemonErrorCode::Unauthorized,
            handshake_deadline,
        )?;
        return Ok(());
    }
    write_runtime_json_line_before(
        &mut stream,
        runtime,
        &V5HandshakeServerResponse::ready(record),
        handshake_deadline,
    )?;
    let session_deadline = Instant::now() + SESSION_READ_TIMEOUT;
    let decoded = match read_v5_request_before(&mut reader, session_deadline) {
        Ok(decoded) => decoded,
        Err(_) => return Ok(()),
    };
    runtime.ensure_named_authority()?;
    let kind = decoded.request().kind();
    match kind {
        V5ClientRequestKind::Ping => {
            #[cfg(feature = "receipt-ledger-test-support")]
            runtime.capture_protocol_transition_after_frame(&decoded)?;
            write_runtime_json_line_before(
                &mut stream,
                runtime,
                &V5ProbeServerResponse::Pong {},
                session_deadline,
            )
        }
        V5ClientRequestKind::SubmitInvocation => match runtime.inspect_strict_submit(decoded) {
            Ok(observation) => {
                #[cfg(feature = "receipt-ledger-test-support")]
                runtime.capture_missing_receipt(observation)?;
                #[cfg(not(feature = "receipt-ledger-test-support"))]
                drop(observation);
                write_runtime_probe_error_before(
                    &mut stream,
                    runtime,
                    V5DaemonErrorCode::ReceiptNotFound,
                    session_deadline,
                )
            }
            Err(_) => write_runtime_probe_error_before(
                &mut stream,
                runtime,
                V5DaemonErrorCode::StoreFailed,
                session_deadline,
            ),
        },
        _ => write_runtime_probe_error_before(
            &mut stream,
            runtime,
            V5DaemonErrorCode::InvalidRequest,
            session_deadline,
        ),
    }
}

fn read_v5_request_before(
    reader: &mut BufReader<TcpStream>,
    deadline: Instant,
) -> Result<DecodedV5Request, V5RequestFrameError> {
    let decoded = read_and_decode_v5_request_before(reader, |reader| {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|remaining| !remaining.is_zero())
            .ok_or_else(|| io::Error::from(io::ErrorKind::TimedOut))?;
        reader.get_ref().set_read_timeout(Some(remaining))
    })?;
    if Instant::now() >= deadline {
        return Err(V5RequestFrameError::Read(io::Error::from(
            io::ErrorKind::TimedOut,
        )));
    }
    Ok(decoded)
}

fn write_runtime_probe_error_before(
    stream: &mut TcpStream,
    runtime: &V5ReceiptRuntime,
    code: V5DaemonErrorCode,
    deadline: Instant,
) -> Result<(), String> {
    write_runtime_json_line_before(
        stream,
        runtime,
        &V5ProbeServerResponse::Error { code },
        deadline,
    )
}

fn write_runtime_json_line_before<T: Serialize>(
    stream: &mut TcpStream,
    runtime: &V5ReceiptRuntime,
    value: &T,
    deadline: Instant,
) -> Result<(), String> {
    runtime.ensure_named_authority()?;
    write_json_line_before(stream, value, deadline)
}

fn write_json_line_before<T: Serialize>(
    stream: &mut TcpStream,
    value: &T,
    deadline: Instant,
) -> Result<(), String> {
    let mut bytes = serde_json::to_vec(value)
        .map_err(|_| "protocol-v5 response could not be serialized".to_string())?;
    bytes.push(b'\n');
    if bytes.len() > MAX_V5_RESPONSE_LINE_BYTES {
        return Err("protocol-v5 response exceeds the byte limit".to_string());
    }
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| "protocol-v5 response deadline expired".to_string())?;
    stream
        .set_write_timeout(Some(remaining))
        .map_err(|error| daemon_io_error("configure protocol-v5 response timeout", error))?;
    stream
        .write_all(&bytes)
        .map_err(|error| daemon_io_error("write protocol-v5 response", error))?;
    deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .map(|_| ())
        .ok_or_else(|| "protocol-v5 response deadline expired".to_string())
}

fn tokens_equal(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.as_bytes()
        .iter()
        .zip(right.as_bytes())
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

fn daemon_io_error(operation: &str, error: io::Error) -> String {
    format!("{operation}: {error}")
}

#[cfg(feature = "receipt-ledger-test-support")]
pub(crate) fn run_protocol_ping_reachability_probe_for_test(
) -> Result<ProductionMissingTransitionEvidence, String> {
    run_v5_reachability_probe_for_test(
        V5ClientRequest::Ping {},
        ReachabilityStoreFixture::Empty,
        ReachabilityExpectedResponse::Pong,
        ReachabilityEvidenceExpectation::Captured,
    )?
    .ok_or_else(|| "protocol-v5 reachability evidence was not captured".to_string())
}

#[cfg(feature = "receipt-ledger-test-support")]
pub(crate) fn run_submit_reachability_probe_for_test(
) -> Result<ProductionMissingTransitionEvidence, String> {
    run_v5_reachability_probe_for_test(
        fixed_submit_request_for_test()?,
        ReachabilityStoreFixture::Empty,
        ReachabilityExpectedResponse::Error(V5DaemonErrorCode::ReceiptNotFound),
        ReachabilityEvidenceExpectation::Captured,
    )?
    .ok_or_else(|| "protocol-v5 receipt evidence was not captured".to_string())
}

#[cfg(feature = "receipt-ledger-test-support")]
pub(crate) fn run_direct_load_reachability_probe_for_test(
) -> Result<ProductionMissingTransitionEvidence, String> {
    run_executor_reachability_probe_for_test(V5ExecutorReachabilityAction::RunDirectLoad)
}

#[cfg(feature = "receipt-ledger-test-support")]
pub(crate) fn run_lazy_cancel_storm_reachability_probe_for_test(
) -> Result<ProductionMissingTransitionEvidence, String> {
    run_executor_reachability_probe_for_test(V5ExecutorReachabilityAction::RunLazyCancelStorm)
}

#[cfg(feature = "receipt-ledger-test-support")]
pub(crate) fn run_seed_task_reachability_probe_for_test(
) -> Result<ProductionMissingTransitionEvidence, String> {
    let root =
        tempfile::tempdir().map_err(|error| format!("create v5 task projection state: {error}"))?;
    let state_root = std::fs::canonicalize(root.path())
        .map_err(|error| format!("canonicalize v5 task projection state: {error}"))?;
    let identity = CoreIdentity::production_v5();
    let state = DaemonStateDirectory::open(&state_root, &identity)?;
    let runtime = V5ReceiptRuntime::open(
        &state,
        &DaemonServerConfig::new(state_root, identity, Duration::from_millis(50)),
    )?;
    let token = runtime.observe_missing_task_projection_writer()?;
    Ok(ProductionMissingTransitionEvidence::task_projection_unavailable(token))
}

#[cfg(feature = "receipt-ledger-test-support")]
fn run_executor_reachability_probe_for_test(
    action: V5ExecutorReachabilityAction,
) -> Result<ProductionMissingTransitionEvidence, String> {
    let root = tempfile::tempdir().map_err(|error| format!("create v5 executor state: {error}"))?;
    let state_root = std::fs::canonicalize(root.path())
        .map_err(|error| format!("canonicalize v5 executor state: {error}"))?;
    let identity = CoreIdentity::production_v5();
    let state = DaemonStateDirectory::open(&state_root, &identity)?;
    let runtime = V5ReceiptRuntime::open(
        &state,
        &DaemonServerConfig::new(state_root, identity, Duration::from_millis(50)),
    )?;
    let token = runtime.observe_missing_executor_writer(action)?;
    Ok(ProductionMissingTransitionEvidence::writer_path_unavailable(token))
}

#[cfg(all(feature = "receipt-ledger-test-support", test))]
fn run_present_submit_reachability_probe_for_test() -> Result<(), String> {
    let evidence = run_v5_reachability_probe_for_test(
        fixed_submit_request_for_test()?,
        ReachabilityStoreFixture::PresentFixedSubmit,
        ReachabilityExpectedResponse::Error(V5DaemonErrorCode::StoreFailed),
        ReachabilityEvidenceExpectation::Absent,
    )?;
    if evidence.is_some() {
        return Err("present receipt row minted missing-row evidence".to_string());
    }
    Ok(())
}

#[cfg(feature = "receipt-ledger-test-support")]
fn fixed_submit_request_for_test() -> Result<V5ClientRequest, String> {
    use crate::application::receipt_ledger::V5ToolIdentity;
    use crate::domain::invocation::{InvocationId, TaskId};
    use std::str::FromStr;

    let invocation = V5InvocationRequest::new(
        InvocationId::from_str("11111111-1111-4111-8111-111111111111")
            .map_err(|_| "invalid fixed v5 reachability invocation id".to_string())?,
        TaskId::from_str("22222222-2222-4222-8222-222222222222")
            .map_err(|_| "invalid fixed v5 reachability task id".to_string())?,
        V5ToolIdentity::View,
        serde_json::Map::new(),
        "workspace-a".to_string(),
        7_000,
    )?;
    Ok(V5ClientRequest::SubmitInvocation { invocation })
}

#[cfg(feature = "receipt-ledger-test-support")]
#[derive(Clone, Copy)]
enum ReachabilityExpectedResponse {
    Pong,
    Error(V5DaemonErrorCode),
}

#[cfg(feature = "receipt-ledger-test-support")]
#[derive(Clone, Copy)]
enum ReachabilityStoreFixture {
    Empty,
    #[cfg(test)]
    PresentFixedSubmit,
}

#[cfg(feature = "receipt-ledger-test-support")]
#[derive(Clone, Copy)]
enum ReachabilityEvidenceExpectation {
    Captured,
    #[cfg(test)]
    Absent,
}

#[cfg(feature = "receipt-ledger-test-support")]
fn run_v5_reachability_probe_for_test(
    request: V5ClientRequest,
    _store_fixture: ReachabilityStoreFixture,
    expected_response: ReachabilityExpectedResponse,
    evidence_expectation: ReachabilityEvidenceExpectation,
) -> Result<Option<ProductionMissingTransitionEvidence>, String> {
    let root = tempfile::tempdir().map_err(|error| format!("create v5 probe state: {error}"))?;
    let state_root = std::fs::canonicalize(root.path())
        .map_err(|error| format!("canonicalize v5 probe state: {error}"))?;
    let identity = CoreIdentity::production_v5();
    let config = DaemonServerConfig::new(
        state_root.clone(),
        identity.clone(),
        Duration::from_millis(50),
    );
    let (evidence_tx, evidence_rx) = sync_channel(1);
    let server = thread::spawn(move || {
        run_daemon_configured(config, |runtime| runtime.with_evidence_capture(evidence_tx))
    });

    let startup_deadline = Instant::now() + Duration::from_secs(5);
    let record = loop {
        let state = DaemonStateDirectory::open(&state_root, &identity)?;
        if let Some(record) = state.read_v5_endpoint_record()? {
            break record;
        }
        if Instant::now() >= startup_deadline {
            return Err("protocol-v5 reachability endpoint was not published".to_string());
        }
        thread::sleep(Duration::from_millis(5));
    };

    #[cfg(test)]
    if matches!(_store_fixture, ReachabilityStoreFixture::PresentFixedSubmit) {
        seed_present_submit_receipt_for_test(&state_root, &identity, &request)?;
    }

    let mut stream = TcpStream::connect(record.loopback_addr()?)
        .map_err(|error| daemon_io_error("connect protocol-v5 reachability endpoint", error))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|error| daemon_io_error("bound protocol-v5 reachability read", error))?;
    write_json_line_before(
        &mut stream,
        &V5ClientRequest::Hello {
            protocol_version: DAEMON_PROTOCOL_VERSION,
            token: record.token().to_string(),
            core_identity: identity,
            owner_lease: uuid::Uuid::new_v4().to_string(),
        },
        Instant::now() + Duration::from_secs(2),
    )?;
    let mut reader = BufReader::new(
        stream
            .try_clone()
            .map_err(|error| daemon_io_error("clone protocol-v5 reachability stream", error))?,
    );
    let ready_frame = read_bounded_v5_probe_response_frame(&mut reader)
        .map_err(|error| daemon_io_error("read protocol-v5 reachability ready", error))?;
    let ready: V5HandshakeServerResponse = serde_json::from_slice(&ready_frame)
        .map_err(|_| "protocol-v5 reachability ready is not strict JSON".to_string())?;
    if !ready.matches_record(&record) {
        return Err("protocol-v5 reachability ready does not match endpoint".to_string());
    }

    write_json_line_before(
        &mut stream,
        &request,
        Instant::now() + Duration::from_secs(2),
    )?;
    let response_frame = read_bounded_v5_probe_response_frame(&mut reader)
        .map_err(|error| daemon_io_error("read protocol-v5 reachability response", error))?;
    let response: V5ProbeServerResponse = serde_json::from_slice(&response_frame)
        .map_err(|_| "protocol-v5 reachability response is not strict JSON".to_string())?;
    let response_matches = match expected_response {
        ReachabilityExpectedResponse::Pong => {
            response.kind() == super::protocol_v5::V5ProbeResponseKind::Pong
        }
        ReachabilityExpectedResponse::Error(expected) => response.error_code() == Some(expected),
    };
    if !response_matches {
        return Err("protocol-v5 reachability probe received an unexpected response".to_string());
    }
    let evidence = match evidence_expectation {
        ReachabilityEvidenceExpectation::Captured => Some(
            evidence_rx
                .recv_timeout(Duration::from_secs(2))
                .map_err(|_| "protocol-v5 reachability evidence was not captured".to_string())?,
        ),
        #[cfg(test)]
        ReachabilityEvidenceExpectation::Absent => {
            match evidence_rx.recv_timeout(Duration::from_millis(100)) {
                Ok(_) => return Err("unexpected protocol-v5 reachability evidence".to_string()),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout)
                | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => None,
            }
        }
    };
    drop(stream);
    server
        .join()
        .map_err(|_| "protocol-v5 reachability daemon panicked".to_string())??;
    Ok(evidence)
}

#[cfg(all(feature = "receipt-ledger-test-support", test))]
fn seed_present_submit_receipt_for_test(
    state_root: &std::path::Path,
    identity: &CoreIdentity,
    request: &V5ClientRequest,
) -> Result<(), String> {
    use crate::infrastructure::platform::filesystem::create_owner_only_file_child;
    use std::ffi::OsStr;
    use std::io::Cursor;

    let mut frame = serde_json::to_vec(request)
        .map_err(|_| "encode fixed present-receipt request".to_string())?;
    frame.push(b'\n');
    let mut reader = BufReader::new(Cursor::new(frame));
    let strict = read_and_decode_v5_request(&mut reader)
        .map_err(|error| format!("decode fixed present-receipt request: {error}"))?
        .into_strict_submit(identity)
        .map_err(|error| format!("derive fixed present-receipt identity: {error}"))?;
    let state = DaemonStateDirectory::open(state_root, identity)?;
    let receipts = state.create_private_retained_subdirectory("receipts")?;
    let active = receipts
        .retain_directory_child(OsStr::new("active"))
        .map_err(|error| daemon_io_error("retain protocol-v5 receipt active fixture", error))?;
    let active = active
        .try_clone_directory()
        .map_err(|error| daemon_io_error("clone protocol-v5 receipt active fixture", error))?;
    let row_name = format!("{}.json", strict.receipt_key_digest().as_str());
    let mut row = create_owner_only_file_child(&active, OsStr::new(&row_name))
        .map_err(|error| daemon_io_error("create protocol-v5 receipt row fixture", error))?;
    row.write_all(b"{}")
        .map_err(|error| daemon_io_error("write protocol-v5 receipt row fixture", error))?;
    row.sync_all()
        .map_err(|error| daemon_io_error("sync protocol-v5 receipt row fixture", error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::daemon::identity::{CoreIdentity, DaemonStateDirectory};
    use crate::infrastructure::daemon::protocol_v5::{
        read_bounded_v5_probe_response_frame, V5EndpointRecord, V5ProbeResponseKind,
        V5ProbeServerResponse,
    };
    use crate::infrastructure::platform::testing::{
        attempt_retained_directory_replacement_for_test, RetainedDirectoryReplacementOutcome,
    };
    use serde_json::json;
    use std::io::{BufReader, Read, Write};
    use std::net::TcpStream;
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, Instant};

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

    fn wait_for_replacement_v5_record(
        state_root: &std::path::Path,
        core_identity: &CoreIdentity,
        displaced_instance_id: &str,
    ) -> V5EndpointRecord {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let state = DaemonStateDirectory::open(state_root, core_identity)
                .expect("open v5 daemon state while waiting for successor");
            if let Some(record) = state
                .read_v5_endpoint_record()
                .expect("read successor v5 endpoint record")
            {
                if record.instance_id() != displaced_instance_id {
                    return record;
                }
            }
            assert!(Instant::now() < deadline, "v5 successor was not published");
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn exact_v5_runtime_opens_receipt_ledger_and_serves_real_handshake_and_ping() {
        let root = tempfile::tempdir().expect("temporary state root");
        let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
        let identity = CoreIdentity::production_v5();
        let config = DaemonServerConfig::new(
            state_root.clone(),
            identity.clone(),
            Duration::from_millis(80),
        );
        let server = thread::spawn(move || run_daemon(config));
        let record = wait_for_v5_record(&state_root, &identity);

        let mut stream = TcpStream::connect(record.loopback_addr().expect("v5 loopback address"))
            .expect("connect v5 daemon");
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("bound v5 response read");
        write_json_line(
            &mut stream,
            &json!({
                "kind": "hello",
                "protocolVersion": 5,
                "token": record.token(),
                "coreIdentity": identity.as_str(),
                "ownerLease": "33333333-3333-4333-8333-333333333333"
            }),
        );
        let mut reader = BufReader::new(stream.try_clone().expect("clone v5 stream"));
        let ready = read_bounded_v5_probe_response_frame(&mut reader).expect("read v5 ready");
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&ready).expect("decode v5 ready"),
            json!({
                "kind": "ready",
                "protocolVersion": 5,
                "coreIdentity": identity.as_str(),
                "daemonPid": std::process::id(),
                "instanceId": record.instance_id()
            })
        );

        write_json_line(&mut stream, &json!({"kind": "ping"}));
        let pong = read_bounded_v5_probe_response_frame(&mut reader).expect("read v5 pong");
        let pong: V5ProbeServerResponse =
            serde_json::from_slice(&pong).expect("decode strict v5 pong");
        assert_eq!(pong.kind(), V5ProbeResponseKind::Pong);
        drop(stream);

        server.join().expect("join v5 runtime").expect("v5 runtime");
        let state = DaemonStateDirectory::open(&state_root, &identity).expect("reopen v5 state");
        assert!(state.read_v5_endpoint_record().unwrap().is_none());
        let receipts = state
            .create_private_retained_subdirectory("receipts")
            .expect("retain production receipts directory");
        assert_eq!(
            std::fs::read(receipts.path().join("generation")).expect("read v5 generation"),
            b"0\n"
        );
    }

    #[cfg(feature = "receipt-ledger-test-support")]
    #[test]
    fn task_projection_evidence_changes_with_production_stable_receipt_observation() {
        let root = tempfile::tempdir().expect("temporary task projection state");
        let state_root =
            std::fs::canonicalize(root.path()).expect("physical task projection state");
        let identity = CoreIdentity::production_v5();
        let state = DaemonStateDirectory::open(&state_root, &identity)
            .expect("open task projection daemon state");
        let encode_projection = |runtime: &V5ReceiptRuntime| {
            let observation = runtime
                .receipt_ledger
                .observe_stable_generation()
                .expect("observe production receipt generation");
            let token = runtime
                .observe_missing_task_projection_writer()
                .expect("observe production task projection boundary");
            let evidence = ProductionMissingTransitionEvidence::task_projection_unavailable(token);
            let encoded = evidence
                .encode_facade_envelope(0, "seed_task")
                .expect("encode task projection evidence");
            (
                observation,
                serde_json::from_str::<serde_json::Value>(&encoded)
                    .expect("decode task projection evidence"),
            )
        };

        let runtime = V5ReceiptRuntime::open(
            &state,
            &DaemonServerConfig::new(
                state_root.clone(),
                identity.clone(),
                Duration::from_millis(50),
            ),
        )
        .expect("open baseline production v5 receipt runtime");
        let (baseline_observation, baseline) = encode_projection(&runtime);
        drop(runtime);
        std::fs::write(state.path().join("receipts/generation"), b"7\n")
            .expect("advance production receipt generation fixture");
        let runtime = V5ReceiptRuntime::open(
            &state,
            &DaemonServerConfig::new(state_root, identity, Duration::from_millis(50)),
        )
        .expect("open advanced production v5 receipt runtime");
        let (advanced_observation, advanced) = encode_projection(&runtime);

        assert_eq!(baseline_observation.generation_before(), 0);
        assert_eq!(baseline_observation.generation_after(), 0);
        assert_eq!(advanced_observation.generation_before(), 7);
        assert_eq!(advanced_observation.generation_after(), 7);
        assert_eq!(baseline["payload"]["evidence"]["generationBefore"], 0);
        assert_eq!(baseline["payload"]["evidence"]["generationAfter"], 0);
        assert_eq!(advanced["payload"]["evidence"]["generationBefore"], 7);
        assert_eq!(advanced["payload"]["evidence"]["generationAfter"], 7);
        assert_ne!(
            baseline["payload"]["evidence"]["fingerprint"],
            advanced["payload"]["evidence"]["fingerprint"]
        );
    }

    #[cfg(feature = "receipt-ledger-test-support")]
    #[test]
    fn missing_executor_and_task_projection_writers_are_minted_only_after_v5_owner_entry() {
        type Probe = fn() -> Result<ProductionMissingTransitionEvidence, String>;
        for (action, boundary, code, event, probe) in [
            (
                "run_direct_load",
                "v5_executor",
                "writer_path_unavailable",
                Some("v5_executor_entered"),
                run_direct_load_reachability_probe_for_test as Probe,
            ),
            (
                "run_lazy_cancel_storm",
                "v5_executor",
                "writer_path_unavailable",
                Some("v5_executor_entered"),
                run_lazy_cancel_storm_reachability_probe_for_test as Probe,
            ),
            (
                "seed_task",
                "task_projection",
                "task_projection_unavailable",
                None,
                run_seed_task_reachability_probe_for_test as Probe,
            ),
        ] {
            let evidence = probe().unwrap_or_else(|error| panic!("{action} owner entry: {error}"));
            let encoded = evidence
                .encode_facade_envelope(0, action)
                .expect("owner evidence correlates");
            let encoded: serde_json::Value =
                serde_json::from_str(&encoded).expect("closed owner evidence envelope");
            assert_eq!(encoded["payload"]["reachedBoundary"], boundary);
            assert_eq!(encoded["payload"]["currentProtocol"], "v5");
            assert_eq!(encoded["payload"]["evidence"]["code"], code);
            match event {
                Some(event) => assert_eq!(encoded["payload"]["evidence"]["event"], event),
                None => assert!(encoded["payload"]["evidence"]["event"].is_null()),
            }
        }
    }

    #[test]
    fn direct_runtime_entry_rejects_every_non_v5_identity_before_state_creation() {
        use std::str::FromStr;

        for identity in [
            CoreIdentity::production(),
            CoreIdentity::from_str(
                "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
            )
            .expect("arbitrary accepted identity"),
        ] {
            let root = tempfile::tempdir().expect("temporary state root");
            let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
            let result = run_daemon(DaemonServerConfig::new(
                state_root,
                identity,
                Duration::from_millis(10),
            ));

            assert_eq!(
                result,
                Err(
                    "protocol-v5 runtime requires the exact production-v5 core identity"
                        .to_string()
                )
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
        let config = DaemonServerConfig::new(
            state_root.clone(),
            identity.clone(),
            Duration::from_millis(20),
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
        stream
            .read_to_end(&mut response)
            .expect("expired handshake must close the transport");

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
    fn displaced_receipt_authority_cannot_keep_accepting_beside_a_successor_owner() {
        let root = tempfile::tempdir().expect("temporary state root");
        let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
        let identity = CoreIdentity::production_v5();
        let config = DaemonServerConfig::new(
            state_root.clone(),
            identity.clone(),
            Duration::from_millis(80),
        );
        let server = thread::spawn(move || run_daemon(config));
        let record = wait_for_v5_record(&state_root, &identity);
        let state = DaemonStateDirectory::open(&state_root, &identity).expect("open daemon state");
        let receipts = state.path().join("receipts");
        let displaced = state.path().join("receipts-displaced");

        match attempt_retained_directory_replacement_for_test(&receipts, &displaced)
            .expect("attempt receipt authority replacement")
        {
            RetainedDirectoryReplacementOutcome::PreventedByRetainedHandle => {
                server.join().expect("join v5 runtime").expect("v5 runtime");
            }
            RetainedDirectoryReplacementOutcome::Replaced => {
                let successor_config = DaemonServerConfig::new(
                    state_root.clone(),
                    identity.clone(),
                    Duration::from_millis(120),
                );
                let successor = thread::spawn(move || run_daemon(successor_config));
                let successor_record =
                    wait_for_replacement_v5_record(&state_root, &identity, record.instance_id());
                let displaced_still_ready =
                    match TcpStream::connect(record.loopback_addr().expect("old v5 address")) {
                        Ok(mut stream) => {
                            stream
                                .set_read_timeout(Some(Duration::from_secs(1)))
                                .expect("bound displaced-daemon read");
                            let hello = json!({
                                "kind": "hello",
                                "protocolVersion": 5,
                                "token": record.token(),
                                "coreIdentity": identity.as_str(),
                                "ownerLease": "33333333-3333-4333-8333-333333333333"
                            });
                            if serde_json::to_writer(&mut stream, &hello).is_ok()
                                && stream.write_all(b"\n").is_ok()
                            {
                                let mut reader = BufReader::new(
                                    stream.try_clone().expect("clone displaced v5 stream"),
                                );
                                read_bounded_v5_probe_response_frame(&mut reader).is_ok()
                            } else {
                                false
                            }
                        }
                        Err(_) => false,
                    };

                let mut successor_stream = TcpStream::connect(
                    successor_record
                        .loopback_addr()
                        .expect("successor v5 loopback address"),
                )
                .expect("connect successor daemon");
                successor_stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .expect("bound successor-daemon read");
                write_json_line(
                    &mut successor_stream,
                    &json!({
                        "kind": "hello",
                        "protocolVersion": 5,
                        "token": successor_record.token(),
                        "coreIdentity": identity.as_str(),
                        "ownerLease": "44444444-4444-4444-8444-444444444444"
                    }),
                );
                let mut successor_reader = BufReader::new(
                    successor_stream
                        .try_clone()
                        .expect("clone successor v5 stream"),
                );
                read_bounded_v5_probe_response_frame(&mut successor_reader)
                    .expect("successor ready");
                write_json_line(&mut successor_stream, &json!({"kind": "ping"}));
                let successor_pong = read_bounded_v5_probe_response_frame(&mut successor_reader)
                    .expect("successor pong");
                assert_eq!(
                    serde_json::from_slice::<V5ProbeServerResponse>(&successor_pong)
                        .expect("decode successor pong")
                        .kind(),
                    V5ProbeResponseKind::Pong
                );
                drop(successor_stream);

                let server_result = server.join().expect("join displaced v5 runtime");
                let successor_result = successor.join().expect("join successor v5 runtime");
                assert!(
                    !displaced_still_ready,
                    "displaced receipt owner still accepted a handshake"
                );
                assert!(
                    server_result.is_err(),
                    "displaced receipt owner exited as if its authority were still named"
                );
                assert!(
                    successor_result.is_ok(),
                    "successor failed: {successor_result:?}"
                );
            }
        }
    }

    #[test]
    fn displaced_runtime_retains_stable_authority_until_the_old_owner_drops() {
        let root = tempfile::tempdir().expect("temporary state root");
        let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
        let identity = CoreIdentity::production_v5();
        let state = DaemonStateDirectory::open(&state_root, &identity).expect("open daemon state");
        let first = V5ReceiptRuntime::open(
            &state,
            &DaemonServerConfig::new(
                state_root.clone(),
                identity.clone(),
                Duration::from_millis(80),
            ),
        )
        .expect("open first runtime owner");
        let receipts = state.path().join("receipts");
        let displaced = state.path().join("receipts-displaced");

        match attempt_retained_directory_replacement_for_test(&receipts, &displaced)
            .expect("attempt receipt authority replacement")
        {
            RetainedDirectoryReplacementOutcome::PreventedByRetainedHandle => return,
            RetainedDirectoryReplacementOutcome::Replaced => {}
        }

        let successor_state =
            DaemonStateDirectory::open(&state_root, &identity).expect("open successor state");
        let successor_while_old_is_live = V5ReceiptRuntime::open(
            &successor_state,
            &DaemonServerConfig::new(
                state_root.clone(),
                identity.clone(),
                Duration::from_millis(80),
            ),
        );
        assert!(
            successor_while_old_is_live.is_err(),
            "replacement receipts directory created a second live runtime authority"
        );

        drop(first);
        V5ReceiptRuntime::open(
            &successor_state,
            &DaemonServerConfig::new(state_root, identity, Duration::from_millis(80)),
        )
        .expect("successor acquires the stable authority after old owner drops");
    }

    #[test]
    fn replacement_receipt_authority_directory_alone_cannot_create_a_successor_runtime() {
        let root = tempfile::tempdir().expect("temporary state root");
        let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
        let identity = CoreIdentity::production_v5();
        let state = DaemonStateDirectory::open(&state_root, &identity).expect("open daemon state");
        let first = V5ReceiptRuntime::open(
            &state,
            &DaemonServerConfig::new(
                state_root.clone(),
                identity.clone(),
                Duration::from_millis(80),
            ),
        )
        .expect("open first runtime owner");
        let authority = state.path().join(".receipt-authority");
        let displaced = state.path().join(".receipt-authority-displaced");

        match attempt_retained_directory_replacement_for_test(&authority, &displaced)
            .expect("attempt stable receipt-authority replacement")
        {
            RetainedDirectoryReplacementOutcome::PreventedByRetainedHandle => return,
            RetainedDirectoryReplacementOutcome::Replaced => {}
        }

        let successor_state =
            DaemonStateDirectory::open(&state_root, &identity).expect("open successor state");
        let successor_while_old_is_live = V5ReceiptRuntime::open(
            &successor_state,
            &DaemonServerConfig::new(
                state_root.clone(),
                identity.clone(),
                Duration::from_millis(80),
            ),
        );

        let error = match successor_while_old_is_live {
            Ok(_) => {
                panic!("replacement receipt-authority directory created a second live runtime")
            }
            Err(error) => error,
        };
        assert_eq!(
            error,
            "open protocol-v5 receipt ledger: receipt ledger is already owned"
        );
        first
            .ensure_named_authority()
            .expect("unchanged receipt ledger keeps the original runtime authoritative");

        drop(first);
        V5ReceiptRuntime::open(
            &successor_state,
            &DaemonServerConfig::new(state_root, identity, Duration::from_millis(80)),
        )
        .expect("successor acquires both authority layers after old owner drops");
    }

    #[test]
    fn displaced_runtime_cannot_write_a_response_after_the_final_authority_check() {
        let root = tempfile::tempdir().expect("temporary state root");
        let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
        let identity = CoreIdentity::production_v5();
        let state = DaemonStateDirectory::open(&state_root, &identity).expect("open daemon state");
        let runtime = V5ReceiptRuntime::open(
            &state,
            &DaemonServerConfig::new(state_root, identity, Duration::from_millis(80)),
        )
        .expect("open runtime owner");
        let receipts = state.path().join("receipts");
        let displaced = state.path().join("receipts-displaced");
        match attempt_retained_directory_replacement_for_test(&receipts, &displaced)
            .expect("attempt receipt authority replacement")
        {
            RetainedDirectoryReplacementOutcome::PreventedByRetainedHandle => return,
            RetainedDirectoryReplacementOutcome::Replaced => {}
        }

        let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .expect("bind displaced response fixture");
        let address = listener.local_addr().expect("displaced response address");
        let client = TcpStream::connect(address).expect("connect displaced response fixture");
        client
            .set_read_timeout(Some(Duration::from_secs(1)))
            .expect("bound displaced response read");
        let (mut server, _) = listener
            .accept()
            .expect("accept displaced response fixture");
        let result = write_runtime_json_line_before(
            &mut server,
            &runtime,
            &V5ProbeServerResponse::Pong {},
            Instant::now() + Duration::from_secs(1),
        );
        drop(server);
        let mut reader = BufReader::new(client);
        let mut response = Vec::new();
        reader
            .read_to_end(&mut response)
            .expect("read displaced response transport");

        assert!(result.is_err(), "displaced runtime wrote a response");
        assert!(
            response.is_empty(),
            "displaced response escaped: {response:?}"
        );
    }

    #[cfg(feature = "receipt-ledger-test-support")]
    #[test]
    fn real_tcp_ping_mints_protocol_evidence_only_after_runtime_frame_handling() {
        let evidence = run_protocol_ping_reachability_probe_for_test()
            .expect("run typed protocol-v5 reachability probe");
        let encoded = evidence
            .encode_facade_envelope(9, "probe_protocol")
            .expect("encode runtime-owned protocol evidence");
        let encoded: serde_json::Value =
            serde_json::from_str(&encoded).expect("decode evidence envelope");

        assert_eq!(encoded["kind"], "production_missing_transition");
        assert_eq!(encoded["payload"]["actionIndex"], 9);
        assert_eq!(encoded["payload"]["actionKind"], "probe_protocol");
        assert_eq!(
            encoded["payload"]["reachedBoundary"],
            "protocol_negotiation"
        );
        assert_eq!(encoded["payload"]["currentProtocol"], "v5");
        assert_eq!(
            encoded["payload"]["evidence"]["code"],
            "protocol_behavior_unavailable"
        );
        assert_eq!(
            encoded["payload"]["evidence"]["event"],
            "protocol_frame_read"
        );
    }

    #[cfg(feature = "receipt-ledger-test-support")]
    #[test]
    fn tcp_submit_missing_row_returns_runtime_evidence_after_exact_inspection() {
        let evidence =
            run_submit_reachability_probe_for_test().expect("run typed submit reachability probe");
        let encoded = evidence
            .encode_facade_envelope(3, "submit")
            .expect("encode runtime-owned receipt evidence");
        let encoded: serde_json::Value =
            serde_json::from_str(&encoded).expect("decode evidence envelope");

        assert_eq!(encoded["kind"], "production_missing_transition");
        assert_eq!(encoded["payload"]["actionIndex"], 3);
        assert_eq!(encoded["payload"]["actionKind"], "submit");
        assert_eq!(encoded["payload"]["reachedBoundary"], "v5_receipt_runtime");
        assert_eq!(encoded["payload"]["currentProtocol"], "v5");
        assert_eq!(encoded["payload"]["evidence"]["code"], "receipt_row_absent");
        assert_eq!(
            encoded["payload"]["evidence"]["event"],
            "v5_receipt_runtime_entered"
        );
        assert_eq!(encoded["payload"]["evidence"]["generationBefore"], 0);
        assert_eq!(encoded["payload"]["evidence"]["generationAfter"], 0);
    }

    #[cfg(feature = "receipt-ledger-test-support")]
    #[test]
    fn present_exact_receipt_row_never_mints_receipt_row_absent() {
        run_present_submit_reachability_probe_for_test()
            .expect("run typed present-receipt reachability probe");
    }
}
