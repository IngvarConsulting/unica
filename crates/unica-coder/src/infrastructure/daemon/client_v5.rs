use super::identity::{CoreIdentity, DaemonStateDirectory};
#[cfg(feature = "receipt-ledger-test-support")]
use super::protocol_v5::V5DaemonErrorCode;
use super::protocol_v5::{
    decode_v5_server_response, read_bounded_v5_probe_response_frame_before, V5ClientRequest,
    V5EndpointRecord, V5HandshakeServerResponse, V5InvocationRequest, V5ProbeResponseKind,
    V5ProbeServerResponse, V5ServerResponse,
};
use crate::application::invocation::RESPONSE_SERIALIZATION_MARGIN;
use crate::application::receipt_ledger::{ReceiptKey, TerminalDigest};
#[cfg(any(test, feature = "receipt-ledger-test-support"))]
use crate::domain::invocation::TaskId;
use crate::infrastructure::platform::ManagedStartupChild;
use std::io::{self, BufReader, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use uuid::Uuid;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const OWNER_RESPONSE_SAFETY_TIMEOUT: Duration = Duration::from_secs(10);
const SPAWN_LOCK_TIMEOUT: Duration = Duration::from_secs(5);
const EXISTING_ENDPOINT_PROBE_TIMEOUT: Duration = Duration::from_millis(500);
const STARTUP_CLEANUP_TIMEOUT: Duration = Duration::from_secs(2);
const RETRY_INTERVAL: Duration = Duration::from_millis(20);

/// Minimal side-by-side protocol-v5 process client.
///
/// This is intentionally narrower than the v3 production client while W0a is
/// non-default: it proves the same-binary `--daemon` dispatch and one strict
/// Hello/Ping exchange without introducing a second launch selector or routing
/// ordinary frontend traffic to v5 before W0c.
pub(crate) struct V5DaemonProcessOwner {
    writer: TcpStream,
    reader: BufReader<TcpStream>,
    record: V5EndpointRecord,
    poisoned: bool,
}

#[cfg(feature = "receipt-ledger-test-support")]
pub(crate) enum V5RawHandshake {
    Ready {
        owner: V5DaemonProcessOwner,
        client_hello_frame: Vec<u8>,
        server_ready_frame: Vec<u8>,
    },
    Rejected {
        client_hello_frame: Vec<u8>,
        server_response_frame: Vec<u8>,
        code: V5DaemonErrorCode,
    },
}

struct StartupEndpointObservation {
    expected_pid: u32,
    record: Option<V5EndpointRecord>,
}

impl StartupEndpointObservation {
    fn new(expected_pid: u32) -> Self {
        Self {
            expected_pid,
            record: None,
        }
    }

    fn observe(&mut self, record: &V5EndpointRecord) -> bool {
        if record.pid() != self.expected_pid {
            return false;
        }
        if let Some(observed) = self.record.as_ref() {
            return observed == record;
        }
        self.record = Some(record.clone());
        true
    }

    fn cleanup(&self, state: &DaemonStateDirectory) -> Result<bool, String> {
        let Some(record) = self.record.as_ref() else {
            return Ok(false);
        };
        state.remove_matching_v5_endpoint_record(record)
    }
}

trait V5StartupChildControl {
    fn detach(&mut self) -> Result<(), String>;
    fn terminate_bounded(&mut self, wait_limit: Duration) -> Result<(), String>;
}

impl V5StartupChildControl for ManagedStartupChild {
    fn detach(&mut self) -> Result<(), String> {
        ManagedStartupChild::detach(self)
    }

    fn terminate_bounded(&mut self, wait_limit: Duration) -> Result<(), String> {
        ManagedStartupChild::terminate_bounded(self, wait_limit)
    }
}

fn finish_ready_startup<C: V5StartupChildControl>(
    owner: V5DaemonProcessOwner,
    child: &mut C,
    state: &DaemonStateDirectory,
    endpoint: &StartupEndpointObservation,
) -> Result<V5DaemonProcessOwner, String> {
    match child.detach() {
        Ok(()) => Ok(owner),
        Err(error) => {
            let termination = child.terminate_bounded(STARTUP_CLEANUP_TIMEOUT);
            let endpoint_cleanup = endpoint.cleanup(state);
            let mut diagnostic =
                format!("protocol-v5 daemon became ready but ownership detach failed: {error}");
            if let Err(cleanup) = termination {
                diagnostic.push_str(&format!(
                    "; protocol-v5 daemon startup cleanup failed: {cleanup}"
                ));
            }
            if let Err(cleanup) = endpoint_cleanup {
                diagnostic.push_str(&format!("; protocol-v5 endpoint cleanup failed: {cleanup}"));
            }
            Err(diagnostic)
        }
    }
}

impl V5DaemonProcessOwner {
    pub(crate) fn connect_or_spawn_for_protocol_test(
        state_root: &Path,
        core_identity: CoreIdentity,
        executable: PathBuf,
        idle_grace: Duration,
    ) -> Result<Self, String> {
        if core_identity != CoreIdentity::production_v5() {
            return Err("protocol-v5 client requires the exact production-v5 identity".to_string());
        }
        let deadline = Instant::now()
            .checked_add(CONNECT_TIMEOUT)
            .ok_or_else(|| "protocol-v5 startup deadline overflow".to_string())?;
        let state = DaemonStateDirectory::open(state_root, &core_identity)?;
        if let Some(record) = state.read_v5_endpoint_record()? {
            let probe_deadline = existing_endpoint_probe_deadline(deadline)?;
            if let Ok(owner) = Self::connect_before(record, probe_deadline) {
                return Ok(owner);
            }
        }

        let lock_budget = remaining(deadline, "spawn lock")?.min(SPAWN_LOCK_TIMEOUT);
        let _spawn_lock = state.acquire_spawn_lock(lock_budget)?;
        if let Some(record) = state.read_v5_endpoint_record()? {
            let probe_deadline = existing_endpoint_probe_deadline(deadline)?;
            if let Ok(owner) = Self::connect_before(record, probe_deadline) {
                return Ok(owner);
            }
        }

        let command =
            super::daemon_process_command(&executable, state_root, &core_identity, idle_grace);
        let mut child = ManagedStartupChild::spawn_configured(command)
            .map_err(|error| format!("failed to spawn protocol-v5 daemon: {error}"))?;
        let expected_pid = child.id();
        let mut endpoint = StartupEndpointObservation::new(expected_pid);

        let readiness = loop {
            let child_status = match child.try_wait_status() {
                Ok(status) => status,
                Err(error) => break Err(error),
            };
            if let Some(status) = child_status {
                break Err(format!(
                    "spawned protocol-v5 daemon {expected_pid} exited before readiness with {status}"
                ));
            }
            let record = match state.read_v5_endpoint_record() {
                Ok(record) => record,
                Err(error) => break Err(error),
            };
            if let Some(record) = record {
                if record.core_identity() != &core_identity {
                    break Err(
                        "protocol-v5 endpoint belongs to a foreign core identity".to_string()
                    );
                }
                if record.pid() == expected_pid {
                    let child_status = match child.try_wait_status() {
                        Ok(status) => status,
                        Err(error) => break Err(error),
                    };
                    if let Some(status) = child_status {
                        break Err(format!(
                            "spawned protocol-v5 daemon {expected_pid} exited before readiness with {status}"
                        ));
                    }
                    if endpoint.observe(&record) {
                        if let Ok(owner) = Self::connect_before(record, deadline) {
                            break Ok(owner);
                        }
                    }
                }
            }
            let retry_budget = match remaining(deadline, "spawn readiness") {
                Ok(remaining) => remaining,
                Err(error) => break Err(error),
            };
            std::thread::sleep(RETRY_INTERVAL.min(retry_budget));
        };

        match readiness {
            Ok(owner) => finish_ready_startup(owner, &mut child, &state, &endpoint),
            Err(error) => {
                let cleanup = child.terminate_bounded(STARTUP_CLEANUP_TIMEOUT);
                let endpoint_cleanup = endpoint.cleanup(&state);
                let mut diagnostic = error;
                if let Err(cleanup) = cleanup {
                    diagnostic.push_str(&format!(
                        "; protocol-v5 daemon startup cleanup failed: {cleanup}"
                    ));
                }
                if let Err(cleanup) = endpoint_cleanup {
                    diagnostic
                        .push_str(&format!("; protocol-v5 endpoint cleanup failed: {cleanup}"));
                }
                Err(diagnostic)
            }
        }
    }

    fn connect_before(record: V5EndpointRecord, deadline: Instant) -> Result<Self, String> {
        let address = record.loopback_addr()?;
        let stream = TcpStream::connect_timeout(&address.into(), remaining(deadline, "connect")?)
            .map_err(|error| format!("connect protocol-v5 daemon: {error}"))?;
        stream
            .set_nonblocking(false)
            .map_err(|error| format!("configure protocol-v5 client stream: {error}"))?;
        let reader_stream = stream
            .try_clone()
            .map_err(|error| format!("clone protocol-v5 client stream: {error}"))?;
        let mut owner = Self {
            writer: stream,
            reader: BufReader::new(reader_stream),
            record,
            poisoned: false,
        };
        let hello = V5ClientRequest::Hello {
            protocol_version: super::protocol_v5::DAEMON_PROTOCOL_VERSION,
            token: owner.record.token().to_string(),
            core_identity: owner.record.core_identity().clone(),
            owner_lease: Uuid::new_v4().to_string(),
        };
        owner.write_before(&hello, deadline, "handshake request")?;
        let frame = owner.read_before(deadline, "handshake response")?;
        let ready: V5HandshakeServerResponse = serde_json::from_slice(&frame)
            .map_err(|_| "protocol-v5 handshake response is not strict JSON".to_string())?;
        if !ready.matches_record(&owner.record) {
            return Err("protocol-v5 handshake response does not match endpoint".to_string());
        }
        Ok(owner)
    }

    #[cfg(feature = "receipt-ledger-test-support")]
    pub(crate) fn connect_existing_raw_for_test(
        record: V5EndpointRecord,
        protocol_version: u32,
        core_identity: CoreIdentity,
        owner_lease: String,
        deadline: Instant,
    ) -> Result<V5RawHandshake, String> {
        let address = record.loopback_addr()?;
        let stream = TcpStream::connect_timeout(&address.into(), remaining(deadline, "connect")?)
            .map_err(|error| format!("connect protocol-v5 daemon: {error}"))?;
        stream
            .set_nonblocking(false)
            .map_err(|error| format!("configure protocol-v5 client stream: {error}"))?;
        let reader_stream = stream
            .try_clone()
            .map_err(|error| format!("clone protocol-v5 client stream: {error}"))?;
        let mut owner = Self {
            writer: stream,
            reader: BufReader::new(reader_stream),
            record,
            poisoned: false,
        };
        let hello = V5ClientRequest::Hello {
            protocol_version,
            token: owner.record.token().to_string(),
            core_identity,
            owner_lease,
        };
        let mut client_hello_frame =
            serde_json::to_vec(&hello).map_err(|_| "serialize protocol-v5 handshake request")?;
        client_hello_frame.push(b'\n');
        owner.write_raw_before(&client_hello_frame, true, deadline, "handshake request")?;
        let server_response_frame = owner.read_before(deadline, "handshake response")?;
        match serde_json::from_slice::<V5HandshakeServerResponse>(&server_response_frame) {
            Ok(ready) => {
                if !ready.matches_record(&owner.record) {
                    return Err(
                        "protocol-v5 handshake response does not match endpoint".to_string()
                    );
                }
                Ok(V5RawHandshake::Ready {
                    owner,
                    client_hello_frame,
                    server_ready_frame: server_response_frame,
                })
            }
            Err(_) => {
                let response: V5ProbeServerResponse =
                    serde_json::from_slice(&server_response_frame).map_err(|_| {
                        "protocol-v5 handshake response is not strict JSON".to_string()
                    })?;
                let Some(code) = response.error_code() else {
                    return Err(
                        "protocol-v5 handshake rejection returned a non-error response".to_string(),
                    );
                };
                drop(owner);
                Ok(V5RawHandshake::Rejected {
                    client_hello_frame,
                    server_response_frame,
                    code,
                })
            }
        }
    }

    pub(crate) fn daemon_pid(&self) -> u32 {
        self.record.pid()
    }

    pub(crate) fn ping(&mut self) -> Result<(), String> {
        if self.poisoned {
            return Err("protocol-v5 owner session is poisoned".to_string());
        }
        let deadline = Instant::now()
            .checked_add(CONNECT_TIMEOUT)
            .ok_or_else(|| "protocol-v5 ping deadline overflow".to_string())?;
        if let Err(error) = self.write_before(&V5ClientRequest::Ping {}, deadline, "ping request") {
            self.poison();
            return Err(error);
        }
        let frame = match self.read_before(deadline, "ping response") {
            Ok(frame) => frame,
            Err(error) => {
                self.poison();
                return Err(error);
            }
        };
        let response: V5ProbeServerResponse = match serde_json::from_slice(&frame) {
            Ok(response) => response,
            Err(_) => {
                self.poison();
                return Err("protocol-v5 ping response is not strict JSON".to_string());
            }
        };
        if response.kind() != V5ProbeResponseKind::Pong {
            self.poison();
            return Err("protocol-v5 ping returned an unexpected response".to_string());
        }
        Ok(())
    }

    #[allow(dead_code)]
    pub(crate) fn submit_invocation(
        &mut self,
        invocation: V5InvocationRequest,
    ) -> Result<V5ServerResponse, String> {
        self.exchange(
            V5ClientRequest::SubmitInvocation { invocation },
            "submit invocation",
        )
    }

    #[cfg(feature = "receipt-ledger-test-support")]
    pub(crate) fn submit_invocation_with_timeout_for_test(
        &mut self,
        invocation: V5InvocationRequest,
        timeout: Duration,
    ) -> Result<V5ServerResponse, String> {
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| "protocol-v5 scenario submit deadline overflow".to_owned())?;
        self.exchange_before(
            V5ClientRequest::SubmitInvocation { invocation },
            "scenario submit invocation",
            deadline,
        )
    }

    #[allow(dead_code)]
    pub(crate) fn cancel_invocation(
        &mut self,
        receipt_key: ReceiptKey,
    ) -> Result<V5ServerResponse, String> {
        self.exchange(
            V5ClientRequest::CancelInvocation { receipt_key },
            "cancel invocation",
        )
    }

    #[allow(dead_code)]
    pub(crate) fn recover_invocation_receipt(
        &mut self,
        receipt_key: ReceiptKey,
    ) -> Result<V5ServerResponse, String> {
        self.exchange(
            V5ClientRequest::RecoverInvocationReceipt { receipt_key },
            "recover invocation receipt",
        )
    }

    #[allow(dead_code)]
    pub(crate) fn acknowledge_invocation_receipt(
        &mut self,
        receipt_key: ReceiptKey,
        terminal_digest: TerminalDigest,
    ) -> Result<V5ServerResponse, String> {
        self.exchange(
            V5ClientRequest::AcknowledgeInvocationReceipt {
                receipt_key,
                terminal_digest,
            },
            "acknowledge invocation receipt",
        )
    }

    #[cfg(any(test, feature = "receipt-ledger-test-support"))]
    #[cfg_attr(test, allow(dead_code))]
    pub(crate) fn get_task(&mut self, task_id: TaskId) -> Result<V5ServerResponse, String> {
        self.exchange(V5ClientRequest::GetTask { task_id }, "get task")
    }

    #[cfg(any(test, feature = "receipt-ledger-test-support"))]
    #[cfg_attr(test, allow(dead_code))]
    pub(crate) fn wait_task(
        &mut self,
        task_id: TaskId,
        wait_ms: u64,
    ) -> Result<V5ServerResponse, String> {
        self.exchange(V5ClientRequest::WaitTask { task_id, wait_ms }, "wait task")
    }

    #[cfg(any(test, feature = "receipt-ledger-test-support"))]
    #[cfg_attr(test, allow(dead_code))]
    pub(crate) fn cancel_task(&mut self, task_id: TaskId) -> Result<V5ServerResponse, String> {
        self.exchange(V5ClientRequest::CancelTask { task_id }, "cancel task")
    }

    #[allow(dead_code)]
    fn exchange(
        &mut self,
        request: V5ClientRequest,
        stage: &'static str,
    ) -> Result<V5ServerResponse, String> {
        let response_timeout = match &request {
            V5ClientRequest::SubmitInvocation { invocation } => {
                Duration::from_millis(invocation.response_budget_ms())
                    .saturating_add(RESPONSE_SERIALIZATION_MARGIN)
            }
            _ => CONNECT_TIMEOUT,
        }
        .min(OWNER_RESPONSE_SAFETY_TIMEOUT);
        let deadline = Instant::now()
            .checked_add(response_timeout)
            .ok_or_else(|| format!("protocol-v5 {stage} deadline overflow"))?;
        self.exchange_before(request, stage, deadline)
    }

    fn exchange_before(
        &mut self,
        request: V5ClientRequest,
        stage: &'static str,
        deadline: Instant,
    ) -> Result<V5ServerResponse, String> {
        if self.poisoned {
            return Err("protocol-v5 owner session is poisoned".to_string());
        }
        if let Err(error) = self.write_before(&request, deadline, stage) {
            self.poison();
            return Err(error);
        }
        let frame = match self.read_before(deadline, stage) {
            Ok(frame) => frame,
            Err(error) => {
                self.poison();
                return Err(error);
            }
        };
        match decode_v5_server_response(&frame) {
            Ok(response) => Ok(response),
            Err(error) => {
                self.poison();
                Err(error)
            }
        }
    }

    #[cfg(feature = "receipt-ledger-test-support")]
    pub(crate) fn exchange_raw_frame(
        &mut self,
        frame: &[u8],
        stage: &'static str,
    ) -> Result<Vec<u8>, String> {
        if self.poisoned {
            return Err("protocol-v5 owner session is poisoned".to_string());
        }
        let deadline = Instant::now()
            .checked_add(OWNER_RESPONSE_SAFETY_TIMEOUT.min(CONNECT_TIMEOUT))
            .ok_or_else(|| format!("protocol-v5 {stage} deadline overflow"))?;
        if let Err(error) = self.write_raw_before(frame, true, deadline, stage) {
            self.poison();
            return Err(error);
        }
        match self.read_before(deadline, stage) {
            Ok(frame) => Ok(frame),
            Err(error) => {
                self.poison();
                Err(error)
            }
        }
    }

    #[cfg(feature = "receipt-ledger-test-support")]
    pub(crate) fn write_raw_frame_and_disconnect(
        mut self,
        frame: &[u8],
        stage: &'static str,
    ) -> Result<(), String> {
        let deadline = Instant::now()
            .checked_add(CONNECT_TIMEOUT)
            .ok_or_else(|| "protocol-v5 raw write deadline overflow".to_string())?;
        self.write_raw_before(frame, true, deadline, stage)?;
        self.poison();
        Ok(())
    }

    fn write_before<T: serde::Serialize>(
        &mut self,
        value: &T,
        deadline: Instant,
        stage: &'static str,
    ) -> Result<(), String> {
        self.writer
            .set_write_timeout(Some(remaining(deadline, stage)?))
            .map_err(|error| format!("configure protocol-v5 {stage} timeout: {error}"))?;
        let mut bytes =
            serde_json::to_vec(value).map_err(|_| format!("serialize protocol-v5 {stage}"))?;
        bytes.push(b'\n');
        self.writer
            .write_all(&bytes)
            .map_err(|error| format!("write protocol-v5 {stage}: {error}"))?;
        remaining(deadline, stage).map(|_| ())
    }

    #[cfg(feature = "receipt-ledger-test-support")]
    fn write_raw_before(
        &mut self,
        bytes: &[u8],
        ensure_newline: bool,
        deadline: Instant,
        stage: &'static str,
    ) -> Result<(), String> {
        self.writer
            .set_write_timeout(Some(remaining(deadline, stage)?))
            .map_err(|error| format!("configure protocol-v5 {stage} timeout: {error}"))?;
        let mut frame = Vec::with_capacity(bytes.len().saturating_add(1));
        frame.extend_from_slice(bytes);
        if ensure_newline && frame.last() != Some(&b'\n') {
            frame.push(b'\n');
        }
        self.writer
            .write_all(&frame)
            .map_err(|error| format!("write protocol-v5 {stage}: {error}"))?;
        remaining(deadline, stage).map(|_| ())
    }

    fn read_before(&mut self, deadline: Instant, stage: &'static str) -> Result<Vec<u8>, String> {
        let frame = read_bounded_v5_probe_response_frame_before(&mut self.reader, |reader| {
            let budget = remaining(deadline, stage)
                .map_err(|error| io::Error::new(io::ErrorKind::TimedOut, error))?;
            reader.get_ref().set_read_timeout(Some(budget))
        })
        .map_err(|error| format!("read protocol-v5 {stage}: {error}"))?;
        remaining(deadline, stage).map(|_| frame)
    }

    fn poison(&mut self) {
        self.poisoned = true;
        let _ = self.writer.shutdown(std::net::Shutdown::Both);
        let _ = self.reader.get_ref().shutdown(std::net::Shutdown::Both);
    }
}

fn remaining(deadline: Instant, stage: &'static str) -> Result<Duration, String> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| format!("protocol-v5 deadline expired during {stage}"))
}

fn existing_endpoint_probe_deadline(outer_deadline: Instant) -> Result<Instant, String> {
    let now = Instant::now();
    let outer_budget = outer_deadline
        .checked_duration_since(now)
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| "protocol-v5 deadline expired during existing endpoint probe".to_string())?;
    now.checked_add(outer_budget.min(EXISTING_ENDPOINT_PROBE_TIMEOUT))
        .ok_or_else(|| "protocol-v5 existing endpoint probe deadline overflow".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, Read};
    use std::net::{Ipv4Addr, Shutdown, TcpListener};
    use std::thread;

    #[derive(Default)]
    struct DetachFailingChild {
        termination_attempted: bool,
    }

    impl V5StartupChildControl for DetachFailingChild {
        fn detach(&mut self) -> Result<(), String> {
            Err("injected detach failure".to_string())
        }

        fn terminate_bounded(&mut self, _wait_limit: Duration) -> Result<(), String> {
            self.termination_attempted = true;
            Ok(())
        }
    }

    fn connected_owner() -> (V5DaemonProcessOwner, TcpStream) {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let client = TcpStream::connect(address).unwrap();
        client.set_nodelay(true).unwrap();
        let (peer, _) = listener.accept().unwrap();
        peer.set_nodelay(true).unwrap();
        let reader = BufReader::new(client.try_clone().unwrap());
        let record = V5EndpointRecord::new(CoreIdentity::production_v5(), address.port()).unwrap();
        (
            V5DaemonProcessOwner {
                writer: client,
                reader,
                record,
                poisoned: false,
            },
            peer,
        )
    }

    fn state_directory() -> (tempfile::TempDir, DaemonStateDirectory) {
        let root = tempfile::tempdir().unwrap();
        let physical_root = std::fs::canonicalize(root.path()).unwrap();
        let state =
            DaemonStateDirectory::open(&physical_root, &CoreIdentity::production_v5()).unwrap();
        (root, state)
    }

    #[test]
    fn partial_v5_response_bytes_cannot_replenish_client_deadline() {
        let (mut owner, mut peer) = connected_owner();
        let peer_thread = thread::spawn(move || {
            for byte in b"{\"kind\":\"pong\"}\n" {
                thread::sleep(Duration::from_millis(50));
                if peer.write_all(&[*byte]).is_err() {
                    break;
                }
            }
        });
        let deadline = Instant::now() + Duration::from_millis(200);

        let started = Instant::now();
        let result = owner.read_before(deadline, "slow response");
        let elapsed = started.elapsed();
        drop(owner);
        peer_thread.join().unwrap();

        assert!(result.is_err(), "slow-drip response crossed the cutoff");
        assert!(
            elapsed < Duration::from_millis(600),
            "partial response reads replenished the deadline for {elapsed:?}"
        );
    }

    #[test]
    fn malformed_v5_response_permanently_poisons_owner_session() {
        let (mut owner, mut peer) = connected_owner();
        let peer_thread = thread::spawn(move || {
            let mut request = Vec::new();
            BufReader::new(&peer)
                .read_until(b'\n', &mut request)
                .unwrap();
            peer.write_all(b"{malformed}\n").unwrap();
            peer.shutdown(Shutdown::Write).unwrap();
            let mut trailing = Vec::new();
            let _ = peer.read_to_end(&mut trailing);
            trailing
        });

        assert_eq!(
            owner.ping().unwrap_err(),
            "protocol-v5 ping response is not strict JSON"
        );
        assert_eq!(
            owner.ping().unwrap_err(),
            "protocol-v5 owner session is poisoned"
        );
        drop(owner);
        let trailing = peer_thread.join().unwrap();
        assert!(
            trailing.is_empty(),
            "poisoned owner wrote another request after malformed response"
        );
    }

    #[test]
    fn failed_startup_cleanup_never_adopts_successor_with_reused_pid() {
        let (_root, state) = state_directory();
        let observed = V5EndpointRecord::new(CoreIdentity::production_v5(), 41_001).unwrap();
        drop(state.publish_v5_endpoint_record(&observed).unwrap());
        let observed = state.read_v5_endpoint_record().unwrap().unwrap();
        let mut endpoint = StartupEndpointObservation::new(observed.pid());
        assert!(endpoint.observe(&observed));

        let successor = V5EndpointRecord::new(CoreIdentity::production_v5(), 41_002).unwrap();
        assert_eq!(successor.pid(), observed.pid(), "PID-reuse fixture drift");
        assert_ne!(successor, observed, "full endpoint identity must change");
        assert!(
            !endpoint.observe(&successor),
            "startup observation adopted a different same-PID record"
        );
        drop(state.publish_v5_endpoint_record(&successor).unwrap());

        assert!(!endpoint.cleanup(&state).unwrap());
        assert_eq!(
            state.read_v5_endpoint_record().unwrap(),
            Some(successor),
            "cleanup adopted and removed the same-PID successor"
        );
    }

    #[test]
    fn detach_failure_cleanup_is_identity_bound_to_observed_ready_record() {
        let (_root, state) = state_directory();
        let (owner, peer) = connected_owner();
        let observed = owner.record.clone();
        drop(state.publish_v5_endpoint_record(&observed).unwrap());
        let mut endpoint = StartupEndpointObservation::new(observed.pid());
        assert!(endpoint.observe(&observed));
        let mut child = DetachFailingChild::default();

        let error = match finish_ready_startup(owner, &mut child, &state, &endpoint) {
            Ok(_) => panic!("injected detach failure unexpectedly succeeded"),
            Err(error) => error,
        };
        drop(peer);

        assert!(error.contains("ownership detach failed"));
        assert!(child.termination_attempted);
        assert_eq!(
            state.read_v5_endpoint_record().unwrap(),
            None,
            "detach failure left the observed ready endpoint published"
        );
    }
}
