use super::identity::{CoreIdentity, DaemonStateDirectory};
use super::protocol::{
    parse_response, read_bounded_response_line, read_bounded_response_line_before, ClientRequest,
    DaemonErrorCode, DaemonTaskSnapshot, EndpointRecord, InvocationRequest, InvocationResponse,
    ServerResponse, DAEMON_PROTOCOL_VERSION, MAX_DAEMON_REQUEST_LINE_BYTES,
};
use crate::application::invocation::RESPONSE_SERIALIZATION_MARGIN_MS;
use crate::infrastructure::platform::ManagedStartupChild;
use std::io::{self, BufReader, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_SPAWN_LOCK_TIMEOUT: Duration = Duration::from_secs(10);
const STARTUP_CLEANUP_TIMEOUT: Duration = Duration::from_secs(2);
const HANDSHAKE_RETRY_INTERVAL: Duration = Duration::from_millis(20);
const INVOCATION_RESPONSE_MARGIN: Duration =
    Duration::from_millis(RESPONSE_SERIALIZATION_MARGIN_MS);

trait DaemonClientClock: Send + Sync {
    fn now(&self) -> Instant;

    #[cfg(test)]
    fn response_read_completed(&self) {}

    #[cfg(test)]
    fn response_parse_started(&self) {}
}

struct SystemDaemonClientClock;

impl SystemDaemonClientClock {
    fn new() -> Self {
        Self
    }
}

impl DaemonClientClock for SystemDaemonClientClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

#[cfg(test)]
#[derive(Clone)]
pub(crate) struct ManualDaemonClientClock {
    origin: Instant,
    elapsed: Arc<std::sync::Mutex<Duration>>,
    advance_before_next_sample: Arc<std::sync::Mutex<Option<Duration>>>,
    advance_after_next_response_read: Arc<std::sync::Mutex<Option<Duration>>>,
    advance_during_next_response_parse: Arc<std::sync::Mutex<Option<Duration>>>,
}

#[cfg(test)]
impl ManualDaemonClientClock {
    pub(crate) fn new() -> Self {
        Self::new_at(Instant::now())
    }

    pub(crate) fn new_at(origin: Instant) -> Self {
        Self {
            origin,
            elapsed: Arc::default(),
            advance_before_next_sample: Arc::default(),
            advance_after_next_response_read: Arc::default(),
            advance_during_next_response_parse: Arc::default(),
        }
    }

    pub(crate) fn advance(&self, amount: Duration) {
        let mut elapsed = self.elapsed.lock().expect("manual daemon client clock");
        *elapsed = elapsed.saturating_add(amount);
    }

    pub(crate) fn advance_before_next_sample(&self, amount: Duration) {
        *self
            .advance_before_next_sample
            .lock()
            .expect("manual daemon client sample pause") = Some(amount);
    }

    pub(crate) fn advance_after_next_response_read(&self, amount: Duration) {
        *self
            .advance_after_next_response_read
            .lock()
            .expect("manual daemon client response read pause") = Some(amount);
    }

    pub(crate) fn advance_during_next_response_parse(&self, amount: Duration) {
        *self
            .advance_during_next_response_parse
            .lock()
            .expect("manual daemon client parse pause") = Some(amount);
    }
}

#[cfg(test)]
impl DaemonClientClock for ManualDaemonClientClock {
    fn now(&self) -> Instant {
        let pause = self
            .advance_before_next_sample
            .lock()
            .expect("manual daemon client sample pause")
            .take();
        let mut elapsed = self.elapsed.lock().expect("manual daemon client clock");
        if let Some(pause) = pause {
            *elapsed = elapsed.saturating_add(pause);
        }
        self.origin + *elapsed
    }

    fn response_read_completed(&self) {
        let pause = self
            .advance_after_next_response_read
            .lock()
            .expect("manual daemon client response read pause")
            .take();
        if let Some(pause) = pause {
            self.advance(pause);
        }
    }

    fn response_parse_started(&self) {
        let pause = self
            .advance_during_next_response_parse
            .lock()
            .expect("manual daemon client parse pause")
            .take();
        if let Some(pause) = pause {
            self.advance(pause);
        }
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
    cutoff: Instant,
    clock: Arc<dyn DaemonClientClock>,
}

impl DaemonDeadline {
    fn new(budget: Duration, clock: Arc<dyn DaemonClientClock>) -> Result<Self, String> {
        if budget.is_zero() {
            return Err("daemon deadline budget must be positive".to_string());
        }
        let cutoff = clock
            .now()
            .checked_add(budget)
            .ok_or_else(|| "daemon deadline cutoff overflow".to_string())?;
        Self::at(cutoff, clock)
    }

    fn at(cutoff: Instant, clock: Arc<dyn DaemonClientClock>) -> Result<Self, String> {
        let deadline = Self { cutoff, clock };
        deadline.checkpoint("deadline capture")?;
        Ok(deadline)
    }

    fn remaining(&self, stage: &'static str) -> Result<Duration, String> {
        self.cutoff
            .checked_duration_since(self.clock.now())
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
        let command = super::daemon_process_command(
            executable,
            &self.config.state_root,
            &self.config.core_identity,
            self.config.idle_grace,
        );
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

/// Closed task-exchange failures for interface adapters. Runtime and transport
/// prose never crosses this boundary and callers never classify formatted text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DaemonTaskExchangeError {
    Protocol(DaemonErrorCode),
    Transport,
    SessionPoisoned,
    UnexpectedResponse,
}

/// One task-adapter transport budget captured before connection admission.
/// The same clock/deadline is consumed by connect, handshake, request, and
/// response so a short compatibility wait cannot reopen the frontend window.
pub(crate) struct DaemonTaskDeadline {
    deadline: DaemonDeadline,
}

impl DaemonTaskDeadline {
    fn matches_clock(&self, clock: &Arc<dyn DaemonClientClock>) -> bool {
        Arc::ptr_eq(&self.deadline.clock, clock)
    }

    fn bounded_wait_ms(&self, requested_wait_ms: u64) -> Result<u64, DaemonTaskExchangeError> {
        let remaining = self
            .deadline
            .remaining("compatibility task wait budget")
            .map_err(|_| DaemonTaskExchangeError::Transport)?;
        let wait_ms = remaining
            .saturating_sub(INVOCATION_RESPONSE_MARGIN)
            .as_millis()
            .min(u128::from(u64::MAX)) as u64;
        Ok(requested_wait_ms.min(wait_ms))
    }
}

impl DaemonOwner {
    fn connect(
        record: &EndpointRecord,
        deadline: &DaemonDeadline,
        exchange_budget: Duration,
        clock: Arc<dyn DaemonClientClock>,
    ) -> Result<Self, ConnectFailure> {
        loop {
            match Self::connect_once(record, deadline, exchange_budget, Arc::clone(&clock)) {
                Ok(owner) => return Ok(owner),
                Err(HandshakeAttemptFailure::Terminal(failure)) => return Err(failure),
                Err(HandshakeAttemptFailure::RetryableTransport) => {
                    let remaining = deadline
                        .remaining("handshake response")
                        .map_err(ConnectFailure::Rejected)?;
                    std::thread::sleep(HANDSHAKE_RETRY_INTERVAL.min(remaining));
                    deadline
                        .checkpoint("handshake response")
                        .map_err(ConnectFailure::Rejected)?;
                }
            }
        }
    }

    fn connect_once(
        record: &EndpointRecord,
        deadline: &DaemonDeadline,
        exchange_budget: Duration,
        clock: Arc<dyn DaemonClientClock>,
    ) -> Result<Self, HandshakeAttemptFailure> {
        let address = record
            .loopback_addr()
            .map_err(|error| HandshakeAttemptFailure::Terminal(ConnectFailure::Rejected(error)))?;
        let connect_budget = deadline
            .remaining("endpoint connect")
            .map_err(|error| HandshakeAttemptFailure::Terminal(ConnectFailure::Rejected(error)))?;
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
                deadline.checkpoint("endpoint connect").map_err(|error| {
                    HandshakeAttemptFailure::Terminal(ConnectFailure::Rejected(error))
                })?;
                return Err(HandshakeAttemptFailure::Terminal(ConnectFailure::Absent));
            }
            Err(error) => {
                return Err(HandshakeAttemptFailure::Terminal(ConnectFailure::Rejected(
                    format!("connect daemon endpoint: {error}"),
                )))
            }
        };
        deadline
            .checkpoint("endpoint connect")
            .map_err(|error| HandshakeAttemptFailure::Terminal(ConnectFailure::Rejected(error)))?;
        let reader_stream = writer.try_clone().map_err(|error| {
            HandshakeAttemptFailure::Terminal(ConnectFailure::Rejected(format!(
                "clone daemon stream: {error}"
            )))
        })?;
        let mut reader = BufReader::new(reader_stream);
        let hello = ClientRequest::hello(
            DAEMON_PROTOCOL_VERSION,
            record.token().to_string(),
            record.core_identity().clone(),
        );
        write_handshake_request(&mut writer, &hello, deadline)?;
        let response = read_handshake_response(&mut reader, deadline)?;
        if !response.matches_record(record) {
            return Err(HandshakeAttemptFailure::Terminal(
                match response.error_code() {
                    Some(
                        code @ (DaemonErrorCode::Overloaded
                        | DaemonErrorCode::OwnerCapacity
                        | DaemonErrorCode::WorkspaceCapacity),
                    ) => ConnectFailure::RetryLater(code),
                    Some(code) => {
                        ConnectFailure::Rejected(format!("daemon handshake rejected: {code}"))
                    }
                    None => {
                        ConnectFailure::Rejected("daemon handshake identity mismatch".to_string())
                    }
                },
            ));
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

    #[cfg(test)]
    pub(crate) fn begin_task_deadline(
        &self,
        budget: Duration,
    ) -> Result<DaemonTaskDeadline, DaemonTaskExchangeError> {
        if self.poisoned {
            return Err(DaemonTaskExchangeError::SessionPoisoned);
        }
        DaemonDeadline::new(budget, Arc::clone(&self.clock))
            .map(|deadline| DaemonTaskDeadline { deadline })
            .map_err(|_| DaemonTaskExchangeError::Transport)
    }

    pub(crate) fn begin_task_deadline_at(
        &self,
        cutoff: Instant,
    ) -> Result<DaemonTaskDeadline, DaemonTaskExchangeError> {
        if self.poisoned {
            return Err(DaemonTaskExchangeError::SessionPoisoned);
        }
        DaemonDeadline::at(cutoff, Arc::clone(&self.clock))
            .map(|deadline| DaemonTaskDeadline { deadline })
            .map_err(|_| DaemonTaskExchangeError::Transport)
    }

    pub(crate) fn connect_peer_before(
        &self,
        deadline: &DaemonTaskDeadline,
    ) -> Result<Self, DaemonTaskExchangeError> {
        if self.poisoned {
            return Err(DaemonTaskExchangeError::SessionPoisoned);
        }
        if !deadline.matches_clock(&self.clock) {
            return Err(DaemonTaskExchangeError::Transport);
        }
        let exchange_budget = deadline
            .deadline
            .remaining("compatibility task peer admission")
            .map_err(|_| DaemonTaskExchangeError::Transport)?;
        Self::connect(
            &self.record,
            &deadline.deadline,
            exchange_budget,
            Arc::clone(&self.clock),
        )
        .map_err(|failure| match failure {
            ConnectFailure::RetryLater(code) => DaemonTaskExchangeError::Protocol(code),
            ConnectFailure::Absent | ConnectFailure::Rejected(_) => {
                DaemonTaskExchangeError::Transport
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
    ) -> Result<DaemonTaskSnapshot, DaemonTaskExchangeError> {
        self.task_exchange(ClientRequest::get_task(task_id), Duration::from_millis(125))
    }

    pub(crate) fn get_task_before(
        &mut self,
        task_id: crate::domain::invocation::TaskId,
        deadline: &DaemonTaskDeadline,
    ) -> Result<DaemonTaskSnapshot, DaemonTaskExchangeError> {
        if !deadline.matches_clock(&self.clock) {
            return Err(DaemonTaskExchangeError::Transport);
        }
        self.task_exchange_before(ClientRequest::get_task(task_id), &deadline.deadline)
    }

    #[allow(dead_code)]
    pub(crate) fn wait_task(
        &mut self,
        task_id: crate::domain::invocation::TaskId,
        wait_ms: u64,
    ) -> Result<DaemonTaskSnapshot, DaemonTaskExchangeError> {
        self.wait_task_with_transport_budget(
            task_id,
            wait_ms,
            Duration::from_millis(wait_ms).saturating_add(INVOCATION_RESPONSE_MARGIN),
        )
    }

    pub(crate) fn wait_task_with_transport_budget(
        &mut self,
        task_id: crate::domain::invocation::TaskId,
        wait_ms: u64,
        transport_budget: Duration,
    ) -> Result<DaemonTaskSnapshot, DaemonTaskExchangeError> {
        self.task_exchange(ClientRequest::wait_task(task_id, wait_ms), transport_budget)
    }

    pub(crate) fn wait_task_before(
        &mut self,
        task_id: crate::domain::invocation::TaskId,
        requested_wait_ms: u64,
        deadline: &DaemonTaskDeadline,
    ) -> Result<DaemonTaskSnapshot, DaemonTaskExchangeError> {
        if !deadline.matches_clock(&self.clock) {
            return Err(DaemonTaskExchangeError::Transport);
        }
        let wait_ms = deadline.bounded_wait_ms(requested_wait_ms)?;
        self.task_exchange_before(
            ClientRequest::wait_task(task_id, wait_ms),
            &deadline.deadline,
        )
    }

    #[allow(dead_code)]
    pub(crate) fn cancel_task(
        &mut self,
        task_id: crate::domain::invocation::TaskId,
    ) -> Result<DaemonTaskSnapshot, DaemonTaskExchangeError> {
        self.task_exchange(
            ClientRequest::cancel_task(task_id),
            Duration::from_millis(125),
        )
    }

    pub(crate) fn cancel_task_before(
        &mut self,
        task_id: crate::domain::invocation::TaskId,
        deadline: &DaemonTaskDeadline,
    ) -> Result<DaemonTaskSnapshot, DaemonTaskExchangeError> {
        if !deadline.matches_clock(&self.clock) {
            return Err(DaemonTaskExchangeError::Transport);
        }
        self.task_exchange_before(ClientRequest::cancel_task(task_id), &deadline.deadline)
    }

    fn task_exchange(
        &mut self,
        request: ClientRequest,
        budget: Duration,
    ) -> Result<DaemonTaskSnapshot, DaemonTaskExchangeError> {
        if self.poisoned {
            return Err(DaemonTaskExchangeError::SessionPoisoned);
        }
        let deadline = DaemonDeadline::new(
            budget.max(Duration::from_millis(1)),
            Arc::clone(&self.clock),
        )
        .map_err(|_| DaemonTaskExchangeError::Transport)?;
        self.task_exchange_before(request, &deadline)
    }

    fn task_exchange_before(
        &mut self,
        request: ClientRequest,
        deadline: &DaemonDeadline,
    ) -> Result<DaemonTaskSnapshot, DaemonTaskExchangeError> {
        write_request(&mut self.writer, &request, deadline, "task request")
            .map_err(|_| DaemonTaskExchangeError::Transport)?;
        match self
            .read_response_or_poison(deadline, "task response")
            .map_err(|_| DaemonTaskExchangeError::Transport)?
        {
            ServerResponse::Task { snapshot } => Ok(snapshot),
            ServerResponse::Error { code } => Err(DaemonTaskExchangeError::Protocol(code)),
            _ => {
                self.poisoned = true;
                let _ = self.writer.shutdown(std::net::Shutdown::Both);
                let _ = self.reader.get_ref().shutdown(std::net::Shutdown::Both);
                Err(DaemonTaskExchangeError::UnexpectedResponse)
            }
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

enum HandshakeAttemptFailure {
    RetryableTransport,
    Terminal(ConnectFailure),
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

fn retryable_handshake_transport(error: &io::Error) -> bool {
    !matches!(
        error.kind(),
        io::ErrorKind::InvalidData
            | io::ErrorKind::InvalidInput
            | io::ErrorKind::PermissionDenied
            | io::ErrorKind::Unsupported
            | io::ErrorKind::OutOfMemory
    )
}

fn write_handshake_request(
    stream: &mut TcpStream,
    request: &ClientRequest,
    deadline: &DaemonDeadline,
) -> Result<(), HandshakeAttemptFailure> {
    let mut bytes = serde_json::to_vec(request).map_err(|_| {
        HandshakeAttemptFailure::Terminal(ConnectFailure::Rejected(
            "daemon request could not be serialized".to_string(),
        ))
    })?;
    debug_assert!(
        super::protocol::parse_request(&bytes).is_ok(),
        "locally constructed daemon request must satisfy the strict protocol"
    );
    bytes.push(b'\n');
    if bytes.len() > MAX_DAEMON_REQUEST_LINE_BYTES {
        return Err(HandshakeAttemptFailure::Terminal(ConnectFailure::Rejected(
            "daemon request exceeds the byte limit".to_string(),
        )));
    }
    let remaining = deadline
        .remaining("handshake request")
        .map_err(|_| HandshakeAttemptFailure::RetryableTransport)?;
    stream.set_write_timeout(Some(remaining)).map_err(|error| {
        HandshakeAttemptFailure::Terminal(ConnectFailure::Rejected(format!(
            "configure daemon write timeout: {error}"
        )))
    })?;
    let result = stream
        .write_all(&bytes)
        .and_then(|_| stream.flush())
        .map_err(|error| {
            if retryable_handshake_transport(&error) {
                HandshakeAttemptFailure::RetryableTransport
            } else {
                HandshakeAttemptFailure::Terminal(ConnectFailure::Rejected(format!(
                    "write daemon request: {error}"
                )))
            }
        });
    deadline
        .checkpoint("handshake request")
        .map_err(|_| HandshakeAttemptFailure::RetryableTransport)?;
    result
}

fn read_handshake_response(
    reader: &mut BufReader<TcpStream>,
    deadline: &DaemonDeadline,
) -> Result<ServerResponse, HandshakeAttemptFailure> {
    let bytes = read_bounded_response_line_before(reader, |reader| {
        let remaining = deadline
            .remaining("handshake response")
            .map_err(|_| io::Error::from(io::ErrorKind::TimedOut))?;
        reader.get_ref().set_read_timeout(Some(remaining))
    });
    #[cfg(test)]
    deadline.clock.response_read_completed();
    deadline
        .checkpoint("handshake response")
        .map_err(|_| HandshakeAttemptFailure::RetryableTransport)?;
    #[cfg(test)]
    deadline.clock.response_parse_started();
    let response = match bytes {
        Ok(bytes) => parse_response(&bytes)
            .map_err(|error| HandshakeAttemptFailure::Terminal(ConnectFailure::Rejected(error))),
        Err(error) => {
            if retryable_handshake_transport(&error) {
                Err(HandshakeAttemptFailure::RetryableTransport)
            } else {
                Err(HandshakeAttemptFailure::Terminal(ConnectFailure::Rejected(
                    format!("read daemon response: {error}"),
                )))
            }
        }
    };
    deadline
        .checkpoint("handshake response")
        .map_err(|_| HandshakeAttemptFailure::RetryableTransport)?;
    response
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
    #[cfg(test)]
    deadline.clock.response_read_completed();
    deadline.checkpoint(stage)?;
    #[cfg(test)]
    deadline.clock.response_parse_started();
    let response =
        parse_response(&bytes.map_err(|error| format!("read daemon response: {error}"))?);
    deadline.checkpoint(stage)?;
    response
}
