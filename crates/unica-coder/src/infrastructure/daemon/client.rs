use super::identity::{CoreIdentity, DaemonStateDirectory};
use super::protocol::{
    parse_response, read_bounded_response_line, ClientRequest, DaemonErrorCode, DaemonTaskSnapshot,
    EndpointRecord, InvocationRequest, InvocationResponse, ServerResponse, DAEMON_PROTOCOL_VERSION,
    MAX_DAEMON_REQUEST_LINE_BYTES,
};
use crate::application::invocation::RESPONSE_SERIALIZATION_MARGIN_MS;
use crate::infrastructure::platform::ManagedStartupChild;
use std::io::{self, BufReader, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_SPAWN_LOCK_TIMEOUT: Duration = Duration::from_secs(10);
const STARTUP_CLEANUP_TIMEOUT: Duration = Duration::from_secs(2);
const INVOCATION_RESPONSE_MARGIN: Duration =
    Duration::from_millis(RESPONSE_SERIALIZATION_MARGIN_MS);

trait DaemonClientClock: Send + Sync {
    fn elapsed(&self) -> Duration;
}

struct SystemDaemonClientClock(Instant);

impl SystemDaemonClientClock {
    fn new() -> Self {
        Self(Instant::now())
    }
}

impl DaemonClientClock for SystemDaemonClientClock {
    fn elapsed(&self) -> Duration {
        self.0.elapsed()
    }
}

#[cfg(test)]
#[derive(Clone, Default)]
pub(crate) struct ManualDaemonClientClock {
    elapsed: Arc<std::sync::Mutex<Duration>>,
}

#[cfg(test)]
impl ManualDaemonClientClock {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn advance(&self, amount: Duration) {
        let mut elapsed = self.elapsed.lock().expect("manual daemon client clock");
        *elapsed = elapsed.saturating_add(amount);
    }
}

#[cfg(test)]
impl DaemonClientClock for ManualDaemonClientClock {
    fn elapsed(&self) -> Duration {
        *self.elapsed.lock().expect("manual daemon client clock")
    }
}

#[derive(Clone)]
pub(crate) struct DaemonClientConfig {
    pub(crate) state_root: PathBuf,
    pub(crate) core_identity: CoreIdentity,
    pub(crate) executable: Option<PathBuf>,
    pub(crate) idle_grace: Duration,
    pub(crate) connect_timeout: Duration,
    clock: Arc<dyn DaemonClientClock>,
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
            clock: Arc::new(SystemDaemonClientClock::new()),
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
            clock: Arc::new(SystemDaemonClientClock::new()),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_clock_for_test(mut self, clock: ManualDaemonClientClock) -> Self {
        self.clock = Arc::new(clock);
        self
    }

    #[cfg(test)]
    pub(crate) fn with_connect_timeout_for_test(mut self, timeout: Duration) -> Self {
        self.connect_timeout = timeout;
        self
    }
}

#[derive(Clone)]
struct DaemonDeadline {
    started: Duration,
    budget: Duration,
    clock: Arc<dyn DaemonClientClock>,
}

impl DaemonDeadline {
    fn new(budget: Duration, clock: Arc<dyn DaemonClientClock>) -> Result<Self, String> {
        if budget.is_zero() {
            return Err("daemon deadline budget must be positive".to_string());
        }
        let started = clock.elapsed();
        Ok(Self {
            started,
            budget,
            clock,
        })
    }

    fn remaining(&self, stage: &'static str) -> Result<Duration, String> {
        self.budget
            .checked_sub(self.clock.elapsed().saturating_sub(self.started))
            .filter(|remaining| !remaining.is_zero())
            .ok_or_else(|| format!("daemon deadline expired during {stage}"))
    }

    fn checkpoint(&self, stage: &'static str) -> Result<(), String> {
        self.remaining(stage).map(|_| ())
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
        let deadline =
            DaemonDeadline::new(self.config.connect_timeout, Arc::clone(&self.config.clock))?;
        if let ExistingDaemon::Connected(owner) = self.connect_existing_before(&deadline)? {
            return Ok(owner);
        }
        let state =
            DaemonStateDirectory::open(&self.config.state_root, &self.config.core_identity)?;
        deadline.checkpoint("state directory open")?;
        let lock_budget = deadline
            .remaining("spawn lock")?
            .min(DEFAULT_SPAWN_LOCK_TIMEOUT);
        let _spawn_lock = state.acquire_spawn_lock(lock_budget)?;
        deadline.checkpoint("spawn lock")?;
        if let ExistingDaemon::Connected(owner) = self.connect_existing_from(&state, &deadline)? {
            return Ok(owner);
        }
        let executable = self
            .config
            .executable
            .as_ref()
            .ok_or_else(|| "daemon spawning is disabled for this client".to_string())?;
        deadline.checkpoint("child spawn")?;
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
        if let Err(error) = deadline.checkpoint("child spawn") {
            return cleanup_failed_startup(error, &mut child);
        }
        let expected_pid = child.id();
        let readiness = self.wait_for_spawned(&state, expected_pid, &mut child, &deadline);
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
            Err(error) => cleanup_failed_startup(error, &mut child),
        }
    }

    #[cfg(test)]
    pub(crate) fn connect_existing(&self) -> Result<ExistingDaemon, String> {
        let deadline =
            DaemonDeadline::new(self.config.connect_timeout, Arc::clone(&self.config.clock))?;
        self.connect_existing_before(&deadline)
    }

    fn connect_existing_before(&self, deadline: &DaemonDeadline) -> Result<ExistingDaemon, String> {
        let state =
            DaemonStateDirectory::open(&self.config.state_root, &self.config.core_identity)?;
        deadline.checkpoint("state directory open")?;
        self.connect_existing_from(&state, deadline)
    }

    fn connect_existing_from(
        &self,
        state: &DaemonStateDirectory,
        deadline: &DaemonDeadline,
    ) -> Result<ExistingDaemon, String> {
        let Some(record) = state.read_endpoint_record()? else {
            deadline.checkpoint("endpoint record lookup")?;
            return Ok(ExistingDaemon::Absent);
        };
        deadline.checkpoint("endpoint record lookup")?;
        if record.core_identity() != &self.config.core_identity {
            return Err("daemon endpoint record belongs to a foreign core identity".to_string());
        }
        match DaemonOwner::connect(
            &record,
            deadline,
            self.config.connect_timeout,
            Arc::clone(&self.config.clock),
        ) {
            Ok(owner) => Ok(ExistingDaemon::Connected(owner)),
            Err(ConnectFailure::Absent) => Ok(ExistingDaemon::Absent),
            Err(ConnectFailure::RetryLater(code)) => Err(retry_later_diagnostic(code)),
            Err(ConnectFailure::Rejected(error)) => Err(error),
        }
    }

    fn wait_for_spawned(
        &self,
        state: &DaemonStateDirectory,
        expected_pid: u32,
        child: &mut ManagedStartupChild,
        deadline: &DaemonDeadline,
    ) -> Result<DaemonOwner, String> {
        loop {
            if let Some(status) = child.try_wait_status()? {
                return Err(format!(
                    "spawned daemon {expected_pid} exited before readiness with {status}"
                ));
            }
            deadline.checkpoint("spawn readiness")?;
            if let Some(record) = state.read_endpoint_record()? {
                deadline.checkpoint("endpoint record lookup")?;
                if record.core_identity() != &self.config.core_identity {
                    return Err(
                        "daemon endpoint record belongs to a foreign core identity".to_string()
                    );
                }
                if record.pid() == expected_pid {
                    match DaemonOwner::connect(
                        &record,
                        deadline,
                        self.config.connect_timeout,
                        Arc::clone(&self.config.clock),
                    ) {
                        Ok(owner) => return Ok(owner),
                        Err(ConnectFailure::Absent) => {}
                        Err(ConnectFailure::RetryLater(code)) => {
                            return Err(retry_later_diagnostic(code));
                        }
                        Err(ConnectFailure::Rejected(error)) => return Err(error),
                    }
                }
            }
            if let Some(status) = child.try_wait_status()? {
                return Err(format!(
                    "spawned daemon {expected_pid} exited before readiness with {status}"
                ));
            }
            let remaining = deadline.remaining("spawn readiness")?;
            std::thread::sleep(Duration::from_millis(20).min(remaining));
        }
    }
}

fn cleanup_failed_startup(
    error: String,
    child: &mut ManagedStartupChild,
) -> Result<DaemonOwner, String> {
    // Cleanup is intentionally a separate, named two-second maximum after the one aggregate
    // five-second startup deadline. The intended worst failure bound is therefore seven seconds
    // plus negligible scheduler overhead; cleanup never replenishes startup/readiness time.
    match child.terminate_bounded(STARTUP_CLEANUP_TIMEOUT) {
        Ok(()) => Err(error),
        Err(cleanup) => Err(format!("{error}; daemon startup cleanup failed: {cleanup}")),
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
    exchange_budget: Duration,
    clock: Arc<dyn DaemonClientClock>,
    poisoned: bool,
}

impl DaemonOwner {
    fn connect(
        record: &EndpointRecord,
        deadline: &DaemonDeadline,
        exchange_budget: Duration,
        clock: Arc<dyn DaemonClientClock>,
    ) -> Result<Self, ConnectFailure> {
        let address = record.loopback_addr().map_err(ConnectFailure::Rejected)?;
        let connect_budget = deadline
            .remaining("endpoint connect")
            .map_err(ConnectFailure::Rejected)?;
        let mut writer = match TcpStream::connect_timeout(&address.into(), connect_budget) {
            Ok(writer) => writer,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::ConnectionRefused
                        | io::ErrorKind::ConnectionAborted
                        | io::ErrorKind::ConnectionReset
                        | io::ErrorKind::TimedOut
                ) =>
            {
                deadline
                    .checkpoint("endpoint connect")
                    .map_err(ConnectFailure::Rejected)?;
                return Err(ConnectFailure::Absent);
            }
            Err(error) => {
                return Err(ConnectFailure::Rejected(format!(
                    "connect daemon endpoint: {error}"
                )))
            }
        };
        deadline
            .checkpoint("endpoint connect")
            .map_err(ConnectFailure::Rejected)?;
        let reader_stream = writer
            .try_clone()
            .map_err(|error| ConnectFailure::Rejected(format!("clone daemon stream: {error}")))?;
        let mut reader = BufReader::new(reader_stream);
        let hello = ClientRequest::hello(
            DAEMON_PROTOCOL_VERSION,
            record.token().to_string(),
            record.core_identity().clone(),
        );
        write_request(&mut writer, &hello, deadline, "handshake request")
            .map_err(ConnectFailure::Rejected)?;
        let response = read_response(&mut reader, deadline, "handshake response")
            .map_err(ConnectFailure::Rejected)?;
        if !response.matches_record(record) {
            return Err(match response.error_code() {
                Some(
                    code @ (DaemonErrorCode::Overloaded
                    | DaemonErrorCode::OwnerCapacity
                    | DaemonErrorCode::WorkspaceCapacity),
                ) => ConnectFailure::RetryLater(code),
                Some(code) => {
                    ConnectFailure::Rejected(format!("daemon handshake rejected: {code}"))
                }
                None => ConnectFailure::Rejected("daemon handshake identity mismatch".to_string()),
            });
        }
        Ok(Self {
            writer,
            reader,
            record: record.clone(),
            exchange_budget,
            clock,
            poisoned: false,
        })
    }

    pub(crate) fn ping(&mut self) -> Result<(), String> {
        self.ensure_usable()?;
        let deadline = DaemonDeadline::new(self.exchange_budget, Arc::clone(&self.clock))?;
        write_request(
            &mut self.writer,
            &ClientRequest::Ping {},
            &deadline,
            "ping request",
        )?;
        match self.read_response_or_poison(&deadline, "ping response")? {
            ServerResponse::Pong => Ok(()),
            response => Err(response.error_code().map_or_else(
                || "daemon ping returned an unexpected response".to_string(),
                |code| format!("daemon ping rejected: {code}"),
            )),
        }
    }

    #[cfg(test)]
    pub(crate) fn submit_invocation(
        &mut self,
        request: InvocationRequest,
    ) -> Result<InvocationResponse, String> {
        let budget = Duration::from_millis(request.response_budget_ms())
            .saturating_add(INVOCATION_RESPONSE_MARGIN);
        self.submit_invocation_with_transport_budget(request, budget)
    }

    pub(crate) fn connect_peer(&self, budget: Duration) -> Result<Self, String> {
        let deadline = DaemonDeadline::new(budget, Arc::clone(&self.clock))?;
        Self::connect(&self.record, &deadline, budget, Arc::clone(&self.clock)).map_err(|failure| {
            match failure {
                ConnectFailure::Absent => "daemon endpoint is unavailable".to_string(),
                ConnectFailure::RetryLater(code) => retry_later_diagnostic(code),
                ConnectFailure::Rejected(error) => error,
            }
        })
    }

    pub(crate) fn submit_invocation_with_transport_budget(
        &mut self,
        request: InvocationRequest,
        transport_budget: Duration,
    ) -> Result<InvocationResponse, String> {
        self.ensure_usable()?;
        let deadline = DaemonDeadline::new(transport_budget, Arc::clone(&self.clock))?;
        write_request(
            &mut self.writer,
            &ClientRequest::submit_invocation(request),
            &deadline,
            "invocation submit request",
        )?;
        match self.read_response_or_poison(&deadline, "invocation submit response")? {
            ServerResponse::Invocation { outcome } => Ok(outcome),
            ServerResponse::Error { code } => {
                Err(format!("daemon invocation submission rejected: {code}"))
            }
            _ => Err("daemon invocation submission returned an unexpected response".into()),
        }
    }

    #[allow(dead_code)]
    pub(crate) fn get_task(
        &mut self,
        task_id: crate::domain::invocation::TaskId,
    ) -> Result<DaemonTaskSnapshot, String> {
        self.task_exchange(ClientRequest::get_task(task_id), Duration::from_millis(125))
    }

    #[allow(dead_code)]
    pub(crate) fn wait_task(
        &mut self,
        task_id: crate::domain::invocation::TaskId,
        wait_ms: u64,
    ) -> Result<DaemonTaskSnapshot, String> {
        self.task_exchange(
            ClientRequest::wait_task(task_id, wait_ms),
            Duration::from_millis(wait_ms).saturating_add(INVOCATION_RESPONSE_MARGIN),
        )
    }

    #[allow(dead_code)]
    pub(crate) fn cancel_task(
        &mut self,
        task_id: crate::domain::invocation::TaskId,
    ) -> Result<DaemonTaskSnapshot, String> {
        self.task_exchange(
            ClientRequest::cancel_task(task_id),
            Duration::from_millis(125),
        )
    }

    fn task_exchange(
        &mut self,
        request: ClientRequest,
        budget: Duration,
    ) -> Result<DaemonTaskSnapshot, String> {
        self.ensure_usable()?;
        let deadline = DaemonDeadline::new(
            budget.max(Duration::from_millis(1)),
            Arc::clone(&self.clock),
        )?;
        write_request(&mut self.writer, &request, &deadline, "task request")?;
        match self.read_response_or_poison(&deadline, "task response")? {
            ServerResponse::Task { snapshot } => Ok(snapshot),
            ServerResponse::Error { code } => Err(format!("daemon task request rejected: {code}")),
            _ => Err("daemon task request returned an unexpected response".into()),
        }
    }

    pub(crate) fn daemon_pid(&self) -> u32 {
        self.record.pid()
    }

    fn ensure_usable(&self) -> Result<(), String> {
        if self.poisoned {
            Err("daemon owner session is closed after a malformed response".to_string())
        } else {
            Ok(())
        }
    }

    fn read_response_or_poison(
        &mut self,
        deadline: &DaemonDeadline,
        stage: &'static str,
    ) -> Result<ServerResponse, String> {
        match read_response(&mut self.reader, deadline, stage) {
            Ok(response) => Ok(response),
            Err(error) => {
                self.poisoned = true;
                let _ = self.writer.shutdown(std::net::Shutdown::Both);
                let _ = self.reader.get_ref().shutdown(std::net::Shutdown::Both);
                Err(error)
            }
        }
    }
}

enum ConnectFailure {
    Absent,
    RetryLater(DaemonErrorCode),
    Rejected(String),
}

fn retry_later_diagnostic(code: DaemonErrorCode) -> String {
    match code {
        DaemonErrorCode::Overloaded => "daemon handshake capacity reached; retry later".to_string(),
        DaemonErrorCode::OwnerCapacity => "daemon owner capacity reached; retry later".to_string(),
        DaemonErrorCode::WorkspaceCapacity => {
            "daemon workspace capacity reached; retry later".to_string()
        }
        _ => "daemon temporarily unavailable; retry later".to_string(),
    }
}

fn write_request(
    stream: &mut TcpStream,
    request: &ClientRequest,
    deadline: &DaemonDeadline,
    stage: &'static str,
) -> Result<(), String> {
    let mut bytes = serde_json::to_vec(request)
        .map_err(|_| "daemon request could not be serialized".to_string())?;
    debug_assert!(
        super::protocol::parse_request(&bytes).is_ok(),
        "locally constructed daemon request must satisfy the strict protocol"
    );
    bytes.push(b'\n');
    if bytes.len() > MAX_DAEMON_REQUEST_LINE_BYTES {
        return Err("daemon request exceeds the byte limit".to_string());
    }
    let remaining = deadline.remaining(stage)?;
    stream
        .set_write_timeout(Some(remaining))
        .map_err(|error| format!("configure daemon write timeout: {error}"))?;
    let result = stream
        .write_all(&bytes)
        .and_then(|_| stream.flush())
        .map_err(|error| format!("write daemon request: {error}"));
    deadline.checkpoint(stage)?;
    result
}

fn read_response(
    reader: &mut BufReader<TcpStream>,
    deadline: &DaemonDeadline,
    stage: &'static str,
) -> Result<ServerResponse, String> {
    let remaining = deadline.remaining(stage)?;
    reader
        .get_ref()
        .set_read_timeout(Some(remaining))
        .map_err(|error| format!("configure daemon read timeout: {error}"))?;
    let bytes = read_bounded_response_line(reader);
    deadline.checkpoint(stage)?;
    parse_response(&bytes.map_err(|error| format!("read daemon response: {error}"))?)
}
