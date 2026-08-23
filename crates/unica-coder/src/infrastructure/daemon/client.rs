use super::identity::{CoreIdentity, DaemonStateDirectory};
use super::protocol::{
    parse_response, read_bounded_json_line, ClientRequest, EndpointRecord, ServerResponse,
    DAEMON_PROTOCOL_VERSION,
};
use crate::infrastructure::platform::ManagedStartupChild;
use std::io::{self, BufReader, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_SPAWN_LOCK_TIMEOUT: Duration = Duration::from_secs(10);
const STARTUP_CLEANUP_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone)]
pub(crate) struct DaemonClientConfig {
    pub(crate) state_root: PathBuf,
    pub(crate) core_identity: CoreIdentity,
    pub(crate) executable: Option<PathBuf>,
    pub(crate) idle_grace: Duration,
    pub(crate) connect_timeout: Duration,
}

impl DaemonClientConfig {
    pub(crate) fn new(
        state_root: PathBuf,
        core_identity: CoreIdentity,
        executable: PathBuf,
        idle_grace: Duration,
    ) -> Self {
        Self {
            state_root,
            core_identity,
            executable: Some(executable),
            idle_grace,
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
        }
    }

    #[cfg(test)]
    pub(crate) fn existing_only(state_root: PathBuf, core_identity: CoreIdentity) -> Self {
        Self {
            state_root,
            core_identity,
            executable: None,
            idle_grace: Duration::from_secs(30),
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
        }
    }
}

pub(crate) struct DaemonClient {
    config: DaemonClientConfig,
}

impl DaemonClient {
    pub(crate) fn new(config: DaemonClientConfig) -> Self {
        Self { config }
    }

    pub(crate) fn connect_or_spawn(&self) -> Result<DaemonOwner, String> {
        if let ExistingDaemon::Connected(owner) = self.connect_existing()? {
            return Ok(owner);
        }
        let state =
            DaemonStateDirectory::open(&self.config.state_root, &self.config.core_identity)?;
        let _spawn_lock = state.acquire_spawn_lock(DEFAULT_SPAWN_LOCK_TIMEOUT)?;
        if let ExistingDaemon::Connected(owner) = self.connect_existing_from(&state)? {
            return Ok(owner);
        }
        let executable = self
            .config
            .executable
            .as_ref()
            .ok_or_else(|| "daemon spawning is disabled for this client".to_string())?;
        let mut command = Command::new(executable);
        command
            .arg("--daemon")
            .arg("--state-root")
            .arg(&self.config.state_root)
            .arg("--core-identity")
            .arg(self.config.core_identity.as_str())
            .arg("--idle-grace-ms")
            .arg(self.config.idle_grace.as_millis().to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut child = ManagedStartupChild::spawn_configured(command)
            .map_err(|error| format!("failed to spawn daemon: {error}"))?;
        let expected_pid = child.id();
        let readiness = self.wait_for_spawned(&state, expected_pid);
        match readiness {
            Ok(owner) => match child.detach() {
                Ok(()) => Ok(owner),
                Err(error) => {
                    let _ = child.terminate_bounded(STARTUP_CLEANUP_TIMEOUT);
                    Err(format!(
                        "daemon became ready but ownership detach failed: {error}"
                    ))
                }
            },
            Err(error) => {
                let cleanup = child.terminate_bounded(STARTUP_CLEANUP_TIMEOUT);
                match cleanup {
                    Ok(()) => Err(error),
                    Err(cleanup) => {
                        Err(format!("{error}; daemon startup cleanup failed: {cleanup}"))
                    }
                }
            }
        }
    }

    pub(crate) fn connect_existing(&self) -> Result<ExistingDaemon, String> {
        let state =
            DaemonStateDirectory::open(&self.config.state_root, &self.config.core_identity)?;
        self.connect_existing_from(&state)
    }

    fn connect_existing_from(
        &self,
        state: &DaemonStateDirectory,
    ) -> Result<ExistingDaemon, String> {
        let Some(record) = state.read_endpoint_record()? else {
            return Ok(ExistingDaemon::Absent);
        };
        if record.core_identity() != &self.config.core_identity {
            return Err("daemon endpoint record belongs to a foreign core identity".to_string());
        }
        match DaemonOwner::connect(&record, self.config.connect_timeout) {
            Ok(owner) => Ok(ExistingDaemon::Connected(owner)),
            Err(ConnectFailure::Absent) => Ok(ExistingDaemon::Absent),
            Err(ConnectFailure::Rejected(error)) => Err(error),
        }
    }

    fn wait_for_spawned(
        &self,
        state: &DaemonStateDirectory,
        expected_pid: u32,
    ) -> Result<DaemonOwner, String> {
        let deadline = Instant::now() + self.config.connect_timeout;
        loop {
            if let Some(record) = state.read_endpoint_record()? {
                if record.core_identity() != &self.config.core_identity {
                    return Err(
                        "daemon endpoint record belongs to a foreign core identity".to_string()
                    );
                }
                if record.pid() == expected_pid {
                    match DaemonOwner::connect(&record, self.config.connect_timeout) {
                        Ok(owner) => return Ok(owner),
                        Err(ConnectFailure::Absent) => {}
                        Err(ConnectFailure::Rejected(error)) => return Err(error),
                    }
                }
            }
            if Instant::now() >= deadline {
                return Err("spawned daemon did not publish a connectable endpoint".to_string());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

pub(crate) enum ExistingDaemon {
    Connected(DaemonOwner),
    Absent,
}

impl std::fmt::Debug for ExistingDaemon {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Connected(_) => formatter.write_str("Connected(<owner lease>)"),
            Self::Absent => formatter.write_str("Absent"),
        }
    }
}

pub(crate) struct DaemonOwner {
    writer: TcpStream,
    reader: BufReader<TcpStream>,
    record: EndpointRecord,
    released: bool,
}

impl DaemonOwner {
    fn connect(record: &EndpointRecord, timeout: Duration) -> Result<Self, ConnectFailure> {
        let address = record.loopback_addr().map_err(ConnectFailure::Rejected)?;
        let mut writer = TcpStream::connect_timeout(&address.into(), timeout).map_err(|error| {
            if matches!(
                error.kind(),
                io::ErrorKind::ConnectionRefused
                    | io::ErrorKind::ConnectionAborted
                    | io::ErrorKind::ConnectionReset
                    | io::ErrorKind::TimedOut
            ) {
                ConnectFailure::Absent
            } else {
                ConnectFailure::Rejected(format!("connect daemon endpoint: {error}"))
            }
        })?;
        writer
            .set_read_timeout(Some(timeout))
            .and_then(|_| writer.set_write_timeout(Some(timeout)))
            .map_err(|error| {
                ConnectFailure::Rejected(format!("configure daemon stream: {error}"))
            })?;
        let reader_stream = writer
            .try_clone()
            .map_err(|error| ConnectFailure::Rejected(format!("clone daemon stream: {error}")))?;
        let mut reader = BufReader::new(reader_stream);
        let hello = ClientRequest::hello(
            DAEMON_PROTOCOL_VERSION,
            record.token().to_string(),
            record.core_identity().clone(),
        );
        write_request(&mut writer, &hello).map_err(ConnectFailure::Rejected)?;
        let response = read_response(&mut reader).map_err(ConnectFailure::Rejected)?;
        if !response.matches_record(record) {
            return Err(ConnectFailure::Rejected(response.error_code().map_or_else(
                || "daemon handshake identity mismatch".to_string(),
                |code| format!("daemon handshake rejected: {code}"),
            )));
        }
        Ok(Self {
            writer,
            reader,
            record: record.clone(),
            released: false,
        })
    }

    pub(crate) fn ping(&mut self) -> Result<(), String> {
        write_request(&mut self.writer, &ClientRequest::Ping {})?;
        match read_response(&mut self.reader)? {
            ServerResponse::Pong => Ok(()),
            response => Err(response.error_code().map_or_else(
                || "daemon ping returned an unexpected response".to_string(),
                |code| format!("daemon ping rejected: {code}"),
            )),
        }
    }

    pub(crate) fn daemon_pid(&self) -> u32 {
        self.record.pid()
    }

    fn release(&mut self) {
        if self.released {
            return;
        }
        self.released = true;
        let _ = write_request(&mut self.writer, &ClientRequest::Release {});
        let _ = read_response(&mut self.reader);
    }
}

impl Drop for DaemonOwner {
    fn drop(&mut self) {
        self.release();
    }
}

enum ConnectFailure {
    Absent,
    Rejected(String),
}

fn write_request(stream: &mut TcpStream, request: &ClientRequest) -> Result<(), String> {
    let mut bytes = serde_json::to_vec(request)
        .map_err(|_| "daemon request could not be serialized".to_string())?;
    debug_assert!(
        super::protocol::parse_request(&bytes).is_ok(),
        "locally constructed daemon request must satisfy the strict protocol"
    );
    bytes.push(b'\n');
    stream
        .write_all(&bytes)
        .and_then(|_| stream.flush())
        .map_err(|error| format!("write daemon request: {error}"))
}

fn read_response(reader: &mut BufReader<TcpStream>) -> Result<ServerResponse, String> {
    let bytes =
        read_bounded_json_line(reader).map_err(|error| format!("read daemon response: {error}"))?;
    parse_response(&bytes)
}
