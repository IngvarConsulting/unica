use super::identity::{CoreIdentity, DaemonStateDirectory};
use super::protocol::{
    parse_request, read_bounded_json_line, ClientRequest, DaemonErrorCode, EndpointRecord,
    ServerResponse, DAEMON_PROTOCOL_VERSION,
};
use crate::composition::open_daemon_invocation_store_from_directory;
use crate::infrastructure::workspace_actor::WorkspaceActorRegistry;
use std::collections::HashSet;
use std::io::{self, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const ACCEPT_POLL_INTERVAL: Duration = Duration::from_millis(20);
const CONNECTION_READ_TIMEOUT: Duration = Duration::from_millis(100);
const HANDSHAKE_READ_TIMEOUT: Duration = Duration::from_secs(2);
pub(crate) const MAX_HANDSHAKES: usize = 8;
pub(crate) const MAX_OWNER_SESSIONS: usize = 64;

#[derive(Debug, Clone)]
pub(crate) struct DaemonServerConfig {
    pub(crate) state_root: std::path::PathBuf,
    pub(crate) core_identity: CoreIdentity,
    pub(crate) idle_grace: Duration,
    #[cfg(test)]
    handshake_pause: Option<Arc<HandshakePause>>,
}

impl DaemonServerConfig {
    pub(crate) fn new(
        state_root: std::path::PathBuf,
        core_identity: CoreIdentity,
        idle_grace: Duration,
    ) -> Self {
        Self {
            state_root,
            core_identity,
            idle_grace,
            #[cfg(test)]
            handshake_pause: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_handshake_pause(mut self, pause: &HandshakePauseGuard) -> Self {
        self.handshake_pause = Some(Arc::clone(&pause.pause));
        self
    }
}

pub(crate) fn run_daemon(config: DaemonServerConfig) -> Result<(), String> {
    if config.idle_grace.is_zero() {
        return Err("daemon idle grace must be positive".to_string());
    }
    let state = DaemonStateDirectory::open(&config.state_root, &config.core_identity)?;
    if let Some(existing) = state.read_endpoint_record()? {
        if existing.core_identity() != &config.core_identity {
            return Err("daemon endpoint record belongs to a foreign core identity".to_string());
        }
    }

    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .map_err(|error| daemon_io_error("bind daemon loopback endpoint", error))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| daemon_io_error("configure daemon listener", error))?;
    let port = listener
        .local_addr()
        .map_err(|error| daemon_io_error("inspect daemon listener", error))?
        .port();

    let task_store_directory = state.create_private_subdirectory("tasks")?;
    let opened_store = open_daemon_invocation_store_from_directory(task_store_directory)?;
    // Recovery belongs to this daemon even before Task 7 routes work. Keeping the report beside
    // the sole-writer store makes it impossible for the stdio frontend to consume it early.
    let _recovery_classifications = opened_store.recovery.classifications.len();
    let _store = opened_store.store;
    // Task 7 routes invocations into this registry. Creating it at the daemon
    // boundary now makes the daemon, rather than either stdio frontend, the
    // sole owner of canonical workspace actors without changing v0.12 calls.
    let _workspace_actors = WorkspaceActorRegistry::default();

    let record = EndpointRecord::new(config.core_identity.clone(), port);
    let published = state.publish_endpoint_record(&record)?;
    let active_leases = Arc::new(LeaseRegistry::default());
    let admitted_connections = Arc::new(AtomicUsize::new(0));
    let shutting_down = Arc::new(AtomicBool::new(false));
    let mut handlers = Vec::new();
    let mut idle_since = Instant::now();

    loop {
        match listener.accept() {
            Ok((stream, address)) if address.ip().is_loopback() => {
                match ConnectionSlot::acquire(Arc::clone(&admitted_connections)) {
                    Some(slot) => handlers.push(spawn_connection_handler(
                        stream,
                        record.clone(),
                        Arc::clone(&active_leases),
                        Arc::clone(&shutting_down),
                        slot,
                        #[cfg(test)]
                        config.handshake_pause.clone(),
                    )),
                    None => reject_overloaded_connection(stream),
                }
            }
            Ok((_stream, _)) => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(error) => {
                shutting_down.store(true, Ordering::Release);
                join_handlers(handlers);
                let _ = state.remove_endpoint_if_owned(&published);
                return Err(daemon_io_error("accept daemon connection", error));
            }
        }

        handlers = reap_finished_handlers(handlers);
        if active_leases.is_empty()? && admitted_connections.load(Ordering::Acquire) == 0 {
            if idle_since.elapsed() >= config.idle_grace {
                break;
            }
        } else {
            idle_since = Instant::now();
        }
        thread::sleep(ACCEPT_POLL_INTERVAL);
    }

    shutting_down.store(true, Ordering::Release);
    join_handlers(handlers);
    state.remove_endpoint_if_owned(&published)?;
    Ok(())
}

fn spawn_connection_handler(
    stream: TcpStream,
    record: EndpointRecord,
    active_leases: Arc<LeaseRegistry>,
    shutting_down: Arc<AtomicBool>,
    slot: ConnectionSlot,
    #[cfg(test)] handshake_pause: Option<Arc<HandshakePause>>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        #[cfg(test)]
        pause_before_handshake_if_configured(handshake_pause);
        let _ = handle_connection(stream, &record, &active_leases, &shutting_down, slot);
    })
}

fn handle_connection(
    mut stream: TcpStream,
    record: &EndpointRecord,
    active_leases: &Arc<LeaseRegistry>,
    shutting_down: &AtomicBool,
    handshake_slot: ConnectionSlot,
) -> Result<(), String> {
    stream
        .set_read_timeout(Some(HANDSHAKE_READ_TIMEOUT))
        .map_err(|error| daemon_io_error("configure daemon handshake timeout", error))?;
    stream
        .set_write_timeout(Some(HANDSHAKE_READ_TIMEOUT))
        .map_err(|error| daemon_io_error("configure daemon response timeout", error))?;
    let reader_stream = stream
        .try_clone()
        .map_err(|error| daemon_io_error("clone daemon client stream", error))?;
    let mut reader = BufReader::new(reader_stream);
    let request = match read_bounded_json_line(&mut reader).and_then(|bytes| {
        parse_request(&bytes).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }) {
        Ok(request) => request,
        Err(_) => {
            write_response(
                &mut stream,
                &ServerResponse::error(DaemonErrorCode::InvalidRequest),
            )?;
            return Ok(());
        }
    };

    let ClientRequest::Hello {
        protocol_version,
        token,
        core_identity,
        owner_lease,
    } = request
    else {
        write_response(
            &mut stream,
            &ServerResponse::error(DaemonErrorCode::HandshakeRequired),
        )?;
        return Ok(());
    };
    if protocol_version != DAEMON_PROTOCOL_VERSION {
        write_response(
            &mut stream,
            &ServerResponse::error(DaemonErrorCode::ProtocolMismatch),
        )?;
        return Ok(());
    }
    if core_identity != *record.core_identity() {
        write_response(
            &mut stream,
            &ServerResponse::error(DaemonErrorCode::CoreMismatch),
        )?;
        return Ok(());
    }
    if !tokens_equal(&token, record.token()) {
        write_response(
            &mut stream,
            &ServerResponse::error(DaemonErrorCode::Unauthorized),
        )?;
        return Ok(());
    }

    let _owner = match active_leases.acquire(owner_lease)? {
        LeaseAdmission::Acquired(owner) => owner,
        LeaseAdmission::Duplicate => {
            write_response(
                &mut stream,
                &ServerResponse::error(DaemonErrorCode::DuplicateLease),
            )?;
            return Ok(());
        }
        LeaseAdmission::Capacity => {
            write_response(
                &mut stream,
                &ServerResponse::error(DaemonErrorCode::OwnerCapacity),
            )?;
            return Ok(());
        }
    };
    // The owner lease becomes the lifecycle fence before the pre-authentication admission
    // permit is released, so idle shutdown observes at least one of them throughout handoff.
    drop(handshake_slot);
    write_response(&mut stream, &ServerResponse::ready(record))?;
    stream
        .set_read_timeout(Some(CONNECTION_READ_TIMEOUT))
        .map_err(|error| daemon_io_error("configure daemon owner timeout", error))?;
    while !shutting_down.load(Ordering::Acquire) {
        let request = match read_bounded_json_line(&mut reader) {
            Ok(bytes) => match parse_request(&bytes) {
                Ok(request) => request,
                Err(_) => {
                    write_response(
                        &mut stream,
                        &ServerResponse::error(DaemonErrorCode::InvalidRequest),
                    )?;
                    break;
                }
            },
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                continue
            }
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(_) => break,
        };
        match request {
            ClientRequest::Ping {} => write_response(&mut stream, &ServerResponse::Pong)?,
            ClientRequest::Release {} => {
                write_response(&mut stream, &ServerResponse::Released)?;
                break;
            }
            ClientRequest::Hello { .. } => {
                write_response(
                    &mut stream,
                    &ServerResponse::error(DaemonErrorCode::InvalidRequest),
                )?;
                break;
            }
        }
    }
    Ok(())
}

fn write_response(stream: &mut TcpStream, response: &ServerResponse) -> Result<(), String> {
    let mut bytes = serde_json::to_vec(response)
        .map_err(|_| "daemon response could not be serialized".to_string())?;
    bytes.push(b'\n');
    stream
        .write_all(&bytes)
        .and_then(|_| stream.flush())
        .map_err(|error| daemon_io_error("write daemon response", error))
}

fn tokens_equal(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.bytes()
        .zip(right.bytes())
        .fold(0u8, |difference, (left, right)| difference | (left ^ right))
        == 0
}

#[derive(Default)]
struct LeaseRegistry {
    leases: Mutex<HashSet<String>>,
}

impl LeaseRegistry {
    fn acquire(self: &Arc<Self>, lease: String) -> Result<LeaseAdmission, String> {
        let mut leases = self
            .leases
            .lock()
            .map_err(|_| "daemon owner lease registry is poisoned".to_string())?;
        if leases.contains(&lease) {
            return Ok(LeaseAdmission::Duplicate);
        }
        if leases.len() >= MAX_OWNER_SESSIONS {
            return Ok(LeaseAdmission::Capacity);
        }
        leases.insert(lease.clone());
        drop(leases);
        Ok(LeaseAdmission::Acquired(LeaseGuard {
            registry: Arc::clone(self),
            lease,
        }))
    }

    fn is_empty(&self) -> Result<bool, String> {
        self.leases
            .lock()
            .map(|leases| leases.is_empty())
            .map_err(|_| "daemon owner lease registry is poisoned".to_string())
    }
}

enum LeaseAdmission {
    Acquired(LeaseGuard),
    Duplicate,
    Capacity,
}

struct LeaseGuard {
    registry: Arc<LeaseRegistry>,
    lease: String,
}

impl Drop for LeaseGuard {
    fn drop(&mut self) {
        if let Ok(mut leases) = self.registry.leases.lock() {
            leases.remove(&self.lease);
        }
    }
}

struct ConnectionSlot {
    admitted: Arc<AtomicUsize>,
}

impl ConnectionSlot {
    fn acquire(admitted: Arc<AtomicUsize>) -> Option<Self> {
        admitted
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                (current < MAX_HANDSHAKES).then_some(current + 1)
            })
            .ok()
            .map(|_| Self { admitted })
    }
}

impl Drop for ConnectionSlot {
    fn drop(&mut self) {
        self.admitted.fetch_sub(1, Ordering::AcqRel);
    }
}

fn reject_overloaded_connection(mut stream: TcpStream) {
    let _ = stream.set_write_timeout(Some(Duration::from_millis(100)));
    let _ = write_response(
        &mut stream,
        &ServerResponse::error(DaemonErrorCode::Overloaded),
    );
}

fn reap_finished_handlers(handlers: Vec<JoinHandle<()>>) -> Vec<JoinHandle<()>> {
    let mut active = Vec::with_capacity(handlers.len());
    for handler in handlers {
        if handler.is_finished() {
            let _ = handler.join();
        } else {
            active.push(handler);
        }
    }
    active
}

fn join_handlers(handlers: Vec<JoinHandle<()>>) {
    for handler in handlers {
        let _ = handler.join();
    }
}

fn daemon_io_error(operation: &str, error: io::Error) -> String {
    format!("{operation}: {error}")
}

#[cfg(test)]
#[derive(Debug, Default)]
struct HandshakePauseState {
    entered: bool,
    released: bool,
}

#[cfg(test)]
#[derive(Debug)]
struct HandshakePause {
    state: Mutex<HandshakePauseState>,
    wake: std::sync::Condvar,
}

#[cfg(test)]
pub(crate) struct HandshakePauseGuard {
    pause: Arc<HandshakePause>,
}

#[cfg(test)]
impl HandshakePauseGuard {
    pub(crate) fn wait_until_entered(&self) {
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut state = self.pause.state.lock().expect("handshake pause state");
        while !state.entered {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "daemon handler did not reach handshake pause"
            );
            let (next, timeout) = self
                .pause
                .wake
                .wait_timeout(state, remaining)
                .expect("handshake pause wait");
            state = next;
            assert!(
                !timeout.timed_out() || state.entered,
                "daemon handler pause timed out"
            );
        }
    }

    pub(crate) fn release(&self) {
        let mut state = self.pause.state.lock().expect("handshake pause state");
        state.released = true;
        self.pause.wake.notify_all();
    }
}

#[cfg(test)]
impl Drop for HandshakePauseGuard {
    fn drop(&mut self) {
        self.release();
    }
}

#[cfg(test)]
pub(crate) fn install_handshake_pause() -> HandshakePauseGuard {
    let pause = Arc::new(HandshakePause {
        state: Mutex::new(HandshakePauseState::default()),
        wake: std::sync::Condvar::new(),
    });
    HandshakePauseGuard { pause }
}

#[cfg(test)]
fn pause_before_handshake_if_configured(pause: Option<Arc<HandshakePause>>) {
    let Some(pause) = pause else {
        return;
    };
    let mut state = pause.state.lock().expect("handshake pause state");
    state.entered = true;
    pause.wake.notify_all();
    while !state.released {
        state = pause.wake.wait(state).expect("handshake pause wait");
    }
}
