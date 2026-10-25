//! Private daemon transport, listener ownership and authenticated sessions.

use crate::identity::{CoreIdentity, DaemonStateDirectory};
use crate::protocol_v5::{
    decode_v5_request_frame, read_bounded_v5_handshake_frame_before,
    read_bounded_v5_request_frame_before, DecodedSubmitAdapter, DecodedV5Request, V5ClientRequest,
    V5ClientRequestKind, V5DaemonErrorCode, V5EndpointRecord, V5HandshakeServerResponse,
    V5InvocationRequest, V5ProbeServerResponse, V5RequestFrameError, V5ServerResponse,
    DAEMON_PROTOCOL_VERSION, MAX_V5_RESPONSE_LINE_BYTES,
};
use serde::Serialize;
use std::collections::HashSet;
use std::io::{self, BufReader, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use unica_execution::invocation::{RESPONSE_SERIALIZATION_MARGIN, TASK_RECONCILIATION_BUDGET};
use unica_execution::receipt_ledger::ReceiptLedgerError;
use unica_execution::receipt_runtime::authority::state_io_error;
use unica_execution::receipt_runtime::hooks::{V5ListenerOwners, V5ReceiptRuntimeEventKind};
use unica_execution::receipt_runtime::ports::{InvocationHost, ReceiptRuntimeConfig};
use unica_execution::receipt_runtime::{
    ReceiptReply, ReceiptResponse, V5ReceiptRuntime, DURABLE_STEP_BUDGET,
};

/// The already-existing observation writer lifecycle, bound by composition.
/// Its guard remains alive until the same shutdown/fail-stop point.
pub trait ObservationLifecycle: Send + Sync {
    type Guard;
    fn start(
        &self,
        state: &DaemonStateDirectory,
        authority_valid: Arc<dyn Fn() -> bool + Send + Sync>,
    ) -> io::Result<Self::Guard>;
}

pub struct DaemonServerConfig<
    H: InvocationHost<Request = V5InvocationRequest>,
    O: ObservationLifecycle,
> {
    pub state_root: std::path::PathBuf,
    pub core_identity: CoreIdentity,
    pub idle_grace: Duration,
    pub runtime: ReceiptRuntimeConfig<H>,
    pub observation: O,
}

const ACCEPT_POLL_INTERVAL: Duration = Duration::from_millis(10);

const HANDSHAKE_READ_TIMEOUT: Duration = Duration::from_secs(2);

const SESSION_READ_TIMEOUT: Duration = Duration::from_secs(2);

/// A session frame must arrive whole within this long of its first byte. No
/// frontend waits longer: its whole call budget is at most 7 s.
const SESSION_FRAME_READ_TIMEOUT: Duration =
    Duration::from_millis(unica_execution::receipt_ledger::MAX_ORIGINAL_RESPONSE_BUDGET_MS);

const OWNER_RESPONSE_WRITE_TIMEOUT: Duration = Duration::from_secs(10);

pub fn run_daemon<H: InvocationHost<Request = V5InvocationRequest>, O: ObservationLifecycle>(
    config: DaemonServerConfig<H, O>,
) -> Result<(), String> {
    run_daemon_configured(config, |runtime| runtime)
}

#[cfg(any(test, feature = "test-support"))]
pub fn run_daemon_until_for_test<
    H: InvocationHost<Request = V5InvocationRequest>,
    O: ObservationLifecycle,
>(
    config: DaemonServerConfig<H, O>,
    stop: Arc<AtomicBool>,
) -> Result<(), String> {
    run_daemon_configured_until(config, |runtime| runtime, || stop.load(Ordering::Acquire))
}

#[cfg(any(test, feature = "test-support"))]
pub mod test_support {
    use super::*;

    pub const SESSION_READ_TIMEOUT: Duration = super::SESSION_READ_TIMEOUT;

    pub struct RequestDeadlines(super::V5RequestDeadlines);

    impl RequestDeadlines {
        pub fn operation(&self) -> Instant {
            self.0.operation
        }

        pub fn durable(&self) -> Instant {
            self.0.durable
        }

        pub fn response(&self) -> Instant {
            self.0.response
        }
    }

    pub fn v5_request_deadlines(
        decoded: &DecodedV5Request,
        request_received_at: Instant,
    ) -> Result<RequestDeadlines, String> {
        super::v5_request_deadlines(decoded, request_received_at).map(RequestDeadlines)
    }

    pub fn fail_stop_response_write_timeout(
        original_response_deadline: Instant,
        observed_at: Instant,
    ) -> Duration {
        super::fail_stop_response_write_timeout(original_response_deadline, observed_at)
    }

    pub fn write_fail_stop_json_line<T: Serialize>(
        stream: &mut TcpStream,
        value: &T,
        original_response_deadline: Instant,
    ) -> Result<(), String> {
        super::write_fail_stop_json_line(stream, value, original_response_deadline)
    }

    pub fn write_runtime_json_line_before<
        H: InvocationHost<Request = V5InvocationRequest>,
        T: Serialize,
    >(
        stream: &mut TcpStream,
        runtime: &V5ReceiptRuntime<H>,
        value: &T,
        deadline: Instant,
    ) -> Result<(), String> {
        super::write_runtime_json_line_before(stream, runtime, value, deadline)
    }

    pub fn outer_lifecycle_response(response: ReceiptResponse) -> V5ServerResponse {
        super::outer_response(response)
    }

    pub fn daemon_error_code(error: &ReceiptLedgerError) -> V5DaemonErrorCode {
        super::daemon_error_code(error)
    }

    #[cfg(feature = "receipt-ledger-test-support")]
    pub fn encode_lifecycle_response(response: ReceiptResponse) -> Result<Vec<u8>, String> {
        crate::protocol_v5::encode_strict_v5_response_jsonl(&super::outer_response(response))
    }

    pub fn run_daemon_configured<
        H: InvocationHost<Request = V5InvocationRequest>,
        O: ObservationLifecycle,
    >(
        config: DaemonServerConfig<H, O>,
        configure_runtime: impl FnOnce(V5ReceiptRuntime<H>) -> V5ReceiptRuntime<H>,
    ) -> Result<(), String> {
        super::run_daemon_configured(config, configure_runtime)
    }

    pub fn run_daemon_configured_until<
        H: InvocationHost<Request = V5InvocationRequest>,
        O: ObservationLifecycle,
    >(
        config: DaemonServerConfig<H, O>,
        configure_runtime: impl FnOnce(V5ReceiptRuntime<H>) -> V5ReceiptRuntime<H>,
        stop_requested: impl Fn() -> bool,
    ) -> Result<(), String> {
        super::run_daemon_configured_until(config, configure_runtime, stop_requested)
    }
}

fn run_daemon_configured<
    H: InvocationHost<Request = V5InvocationRequest>,
    O: ObservationLifecycle,
>(
    config: DaemonServerConfig<H, O>,
    configure_runtime: impl FnOnce(V5ReceiptRuntime<H>) -> V5ReceiptRuntime<H>,
) -> Result<(), String> {
    run_daemon_configured_until(config, configure_runtime, || false)
}

fn run_daemon_configured_until<
    H: InvocationHost<Request = V5InvocationRequest>,
    O: ObservationLifecycle,
>(
    config: DaemonServerConfig<H, O>,
    configure_runtime: impl FnOnce(V5ReceiptRuntime<H>) -> V5ReceiptRuntime<H>,
    stop_requested: impl Fn() -> bool,
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
    // a listener can become discoverable. A refusal of the persisted state is
    // recorded for the frontend, which sees only this process's exit (#1360).
    let opened = match V5ReceiptRuntime::open(&state, &config.runtime) {
        Ok(opened) => opened,
        Err(error) => {
            if let Err(record_error) = state.publish_startup_failure(std::process::id(), &error) {
                eprintln!("protocol-v5 startup failure could not be recorded: {record_error}");
            }
            return Err(error);
        }
    };
    let runtime = Arc::new(configure_runtime(opened));
    runtime.hooks().runtime_opened(&runtime);
    if let Err(error) = runtime.ensure_named_authority() {
        if runtime.restart_required() {
            std::mem::forget(runtime);
        }
        return Err(error);
    }
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .map_err(|error| state_io_error("bind protocol-v5 loopback endpoint", error))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| state_io_error("configure protocol-v5 listener", error))?;
    let port = listener
        .local_addr()
        .map_err(|error| state_io_error("inspect protocol-v5 listener", error))?
        .port();
    let record = V5EndpointRecord::new(config.core_identity.clone(), port)?;
    let published = state.publish_v5_endpoint_record(&record)?;
    // This process started: an earlier refusal no longer describes the state.
    let _ = state.remove_startup_failure();
    if let Err(error) = runtime.ensure_named_authority() {
        if runtime.restart_required() {
            drop(listener);
            std::mem::forget(runtime);
            return Ok(());
        }
        let _ = state.remove_v5_endpoint_if_owned(&published);
        return Err(error);
    }
    // A losing startup process never opens or rewrites the observation state.
    // The writer begins only after the endpoint's second authority check.
    let authority = Arc::downgrade(&runtime);
    let observation_authority: Arc<dyn Fn() -> bool + Send + Sync> = Arc::new(move || {
        authority.upgrade().is_some_and(|runtime| {
            !runtime.restart_required() && runtime.ensure_named_authority().is_ok()
        })
    });
    let mut capacity_writer = match config.observation.start(&state, observation_authority) {
        Ok(writer) => Some(writer),
        Err(_) => {
            eprintln!("local capacity observation disabled: storage unavailable");
            None
        }
    };
    let listener_lease = runtime.hooks().listener_lease();
    let active_leases = Arc::new(V5LeaseRegistry::default());
    let admitted_connections = Arc::new(AtomicUsize::new(0));
    let shutting_down = Arc::new(AtomicBool::new(false));
    let mut idle_since = Instant::now();
    let mut restart_requested = false;
    let mut sessions = Vec::new();

    loop {
        if stop_requested() {
            break;
        }
        // A durable generation probe is an actor command. Enqueuing one on every listener poll
        // lets a healthy request occupy the sole writer long enough for the probe's shorter
        // maintenance deadline to expire, which then tears down the listener before the
        // request's own response budget elapses. Mutations and session admission validate the
        // named authority themselves; the accept loop only observes their process-owned
        // fail-stop latch. A slow admission or an attempt that ignores its cancel does not
        // stop the process: it keeps only its own Task (#1251).
        if runtime.restart_required() {
            restart_requested = true;
            runtime.hooks().forced_process_exit(None);
            break;
        }
        match listener.accept() {
            Ok((stream, address)) if address.ip().is_loopback() => {
                if runtime.restart_required() {
                    drop(stream);
                    restart_requested = true;
                    runtime.hooks().forced_process_exit(None);
                    break;
                }
                let connection = V5AcceptedConnection {
                    stream,
                    handshake_deadline: Instant::now() + HANDSHAKE_READ_TIMEOUT,
                };
                match V5ConnectionSlot::acquire(Arc::clone(&admitted_connections)) {
                    Some(slot) => match spawn_v5_connection_handler(
                        connection,
                        record.clone(),
                        Arc::clone(&active_leases),
                        Arc::clone(&shutting_down),
                        Arc::clone(&runtime),
                        slot,
                    ) {
                        Ok(handler) => sessions.push(handler),
                        Err(error) => {
                            // No handler started: its closure released only this
                            // transport and admission slot. Existing owners keep
                            // the listener and their work alive.
                            let _ = writeln!(
                                io::stderr().lock(),
                                "protocol-v5 connection handler could not start: {error}"
                            );
                        }
                    },
                    None => reject_overloaded_v5_connection(connection),
                }
            }
            Ok((_stream, _)) => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(error) => {
                shutting_down.store(true, Ordering::Release);
                join_v5_handlers(sessions);
                let _ = state.remove_v5_endpoint_if_owned(&published);
                return Err(state_io_error("accept protocol-v5 connection", error));
            }
        }

        sessions = reap_finished_v5_handlers(sessions);
        // Only this listener adds handshake slots. A handler installs its
        // owner lease before releasing that slot, so observing slots first
        // cannot combine an old empty lease set with the post-handoff count.
        let handshakes_empty = admitted_connections.load(Ordering::Acquire) == 0;
        let owners_empty = active_leases.is_empty()?;
        runtime.hooks().after_idle_owner_leases_read(owners_empty);
        let no_active_work = owners_empty
            && handshakes_empty
            && runtime.active_cancellations_empty()
            && !runtime.has_retained_workspace_work();
        // Reading every owner is an observer's question: the decision above
        // stops at the first owner that holds.
        if runtime.hooks().observes_listener_owners() {
            runtime.hooks().listener_owners_observed(V5ListenerOwners {
                client_leases: !owners_empty,
                handshakes: !handshakes_empty,
                task_cancellations: !runtime.active_cancellations_empty(),
                retained_workspace_work: runtime.has_retained_workspace_work(),
            });
        }
        runtime.hooks().listener_ownership_observed(no_active_work);
        if no_active_work {
            if idle_since.elapsed() >= config.idle_grace {
                break;
            }
        } else {
            idle_since = Instant::now();
        }
        thread::sleep(ACCEPT_POLL_INTERVAL);
    }

    shutting_down.store(true, Ordering::Release);
    drop(listener);
    drop(listener_lease);
    if restart_requested {
        // INV.APP.DAEMON-STORE-FAIL-STOP: keep both the PID-bound endpoint and
        // receipt authority alive until process death. A detached worker may
        // still be inside an uninterruptible adapter or syscall.
        if runtime.hooks().releases_authority_on_fail_stop() {
            // The contract harness runs the daemon in a thread; joining that
            // thread is its simulated process-death boundary. Release the
            // authority so unrelated scenarios do not share resources that
            // production releases at PID exit. A promoted attempt the owner
            // handed to a worker still holds an `Arc` to this runtime; join
            // those workers first so the drop actually releases the authority
            // an off-thread continuation would otherwise keep alive.
            join_v5_handlers(sessions);
            runtime.join_task_executions();
            drop(capacity_writer.take());
            drop(runtime);
        } else {
            drop(sessions);
            std::mem::forget(runtime);
        }
        return Ok(());
    }
    join_v5_handlers(sessions);
    runtime.join_task_executions();
    drop(capacity_writer.take());
    state.remove_v5_endpoint_if_owned(&published)?;
    Ok(())
}

struct V5AcceptedConnection {
    stream: TcpStream,
    handshake_deadline: Instant,
}

fn spawn_v5_connection_handler<H: InvocationHost<Request = V5InvocationRequest>>(
    connection: V5AcceptedConnection,
    record: V5EndpointRecord,
    active_leases: Arc<V5LeaseRegistry>,
    shutting_down: Arc<AtomicBool>,
    runtime: Arc<V5ReceiptRuntime<H>>,
    slot: V5ConnectionSlot,
) -> io::Result<thread::JoinHandle<()>> {
    let hooks = Arc::clone(runtime.hooks());
    hooks.spawn_connection_handler(Box::new(move || {
        let _ = handle_probe_connection(
            connection.stream,
            connection.handshake_deadline,
            &record,
            &active_leases,
            &shutting_down,
            &runtime,
            slot,
        );
    }))
}

fn reject_overloaded_v5_connection(connection: V5AcceptedConnection) {
    let mut stream = connection.stream;
    let _ = write_json_line_before(
        &mut stream,
        &V5ServerResponse::Error {
            code: V5DaemonErrorCode::Overloaded,
        },
        connection
            .handshake_deadline
            .min(Instant::now() + Duration::from_millis(100)),
    );
}

fn reap_finished_v5_handlers(handlers: Vec<thread::JoinHandle<()>>) -> Vec<thread::JoinHandle<()>> {
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

fn join_v5_handlers(handlers: Vec<thread::JoinHandle<()>>) {
    for handler in handlers {
        let _ = handler.join();
    }
}

#[derive(Default)]
struct V5LeaseRegistry {
    leases: Mutex<HashSet<String>>,
}

impl V5LeaseRegistry {
    fn acquire(self: &Arc<Self>, lease: String) -> Result<V5LeaseAdmission, String> {
        let mut leases = self
            .leases
            .lock()
            .map_err(|_| "protocol-v5 owner lease registry is poisoned".to_string())?;
        if leases.contains(&lease) {
            return Ok(V5LeaseAdmission::Duplicate);
        }
        leases.insert(lease.clone());
        drop(leases);
        Ok(V5LeaseAdmission::Acquired(V5LeaseGuard {
            registry: Arc::clone(self),
            lease,
        }))
    }

    fn is_empty(&self) -> Result<bool, String> {
        self.leases
            .lock()
            .map(|leases| leases.is_empty())
            .map_err(|_| "protocol-v5 owner lease registry is poisoned".to_string())
    }
}

enum V5LeaseAdmission {
    Acquired(V5LeaseGuard),
    Duplicate,
}

struct V5LeaseGuard {
    registry: Arc<V5LeaseRegistry>,
    lease: String,
}

impl Drop for V5LeaseGuard {
    fn drop(&mut self) {
        if let Ok(mut leases) = self.registry.leases.lock() {
            leases.remove(&self.lease);
        }
    }
}

struct V5ConnectionSlot {
    admitted: Arc<AtomicUsize>,
}

impl V5ConnectionSlot {
    fn acquire(admitted: Arc<AtomicUsize>) -> Option<Self> {
        admitted
            .try_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                current.checked_add(1)
            })
            .ok()
            .map(|_| Self { admitted })
    }
}

impl Drop for V5ConnectionSlot {
    fn drop(&mut self) {
        self.admitted.fetch_sub(1, Ordering::AcqRel);
    }
}

fn handle_probe_connection<H: InvocationHost<Request = V5InvocationRequest>>(
    mut stream: TcpStream,
    handshake_deadline: Instant,
    record: &V5EndpointRecord,
    active_leases: &Arc<V5LeaseRegistry>,
    shutting_down: &AtomicBool,
    runtime: &Arc<V5ReceiptRuntime<H>>,
    handshake_slot: V5ConnectionSlot,
) -> Result<(), String> {
    runtime.hooks().before_owner_handshake(
        stream
            .peer_addr()
            .map_err(|error| state_io_error("inspect accepted protocol-v5 peer", error))?
            .port(),
    );
    runtime.ensure_named_authority_before(handshake_deadline)?;
    stream
        .set_nonblocking(false)
        .map_err(|error| state_io_error("configure protocol-v5 client stream", error))?;
    let reader_stream = stream
        .try_clone()
        .map_err(|error| state_io_error("clone protocol-v5 client stream", error))?;
    let mut reader = BufReader::new(reader_stream);
    let decoded = match read_v5_handshake_before(&mut reader, handshake_deadline) {
        Ok((decoded, _)) => decoded,
        Err(V5RequestFrameError::InvalidRequest(_)) => {
            write_runtime_probe_error_before(
                &mut stream,
                runtime,
                V5DaemonErrorCode::InvalidRequest,
                handshake_deadline,
            )?;
            return Ok(());
        }
        Err(V5RequestFrameError::Read(error)) if error.kind() == io::ErrorKind::InvalidData => {
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
    let Some((protocol_version, token, core_identity, owner_lease)) =
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
    let _owner = match active_leases.acquire(owner_lease.to_owned())? {
        V5LeaseAdmission::Acquired(owner) => owner,
        V5LeaseAdmission::Duplicate => {
            write_runtime_probe_error_before(
                &mut stream,
                runtime,
                V5DaemonErrorCode::DuplicateLease,
                handshake_deadline,
            )?;
            return Ok(());
        }
    };
    // The owner lease fences listener shutdown before pre-authentication admission
    // is released; the listener observes the admission count before owner leases.
    drop(handshake_slot);
    write_runtime_json_line_before(
        &mut stream,
        runtime,
        &V5HandshakeServerResponse::ready(record),
        handshake_deadline,
    )?;
    while !shutting_down.load(Ordering::Acquire) {
        let session_read_deadline = Instant::now() + SESSION_READ_TIMEOUT;
        let (decoded, request_received_at) =
            match read_v5_session_request(&mut reader, session_read_deadline) {
                Ok(observation) => observation,
                Err(V5RequestFrameError::InvalidRequest(_)) => {
                    write_runtime_probe_error_before(
                        &mut stream,
                        runtime,
                        V5DaemonErrorCode::InvalidRequest,
                        Instant::now() + SESSION_READ_TIMEOUT,
                    )?;
                    break;
                }
                Err(V5RequestFrameError::Read(error))
                    if error.kind() == io::ErrorKind::InvalidData =>
                {
                    write_runtime_probe_error_before(
                        &mut stream,
                        runtime,
                        V5DaemonErrorCode::InvalidRequest,
                        Instant::now() + SESSION_READ_TIMEOUT,
                    )?;
                    break;
                }
                Err(V5RequestFrameError::Read(error))
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    continue;
                }
                Err(V5RequestFrameError::Read(error))
                    if error.kind() == io::ErrorKind::UnexpectedEof =>
                {
                    break;
                }
                Err(V5RequestFrameError::Read(_)) => break,
            };
        // The call's own deadline is fixed here, at frame receipt, on the
        // runtime's clock; every later stage of the submit spends it.
        let call_deadline = match decoded.request() {
            V5ClientRequest::SubmitInvocation { invocation } => {
                Some(runtime.capture_submit_deadline(invocation.response_budget_ms()))
            }
            _ => None,
        };
        let deadlines = match runtime.hooks().session_deadline_override() {
            Some(bulk_deadline) => V5RequestDeadlines {
                operation: bulk_deadline,
                durable: bulk_deadline,
                response: bulk_deadline,
            },
            None => {
                let mut deadlines = v5_request_deadlines(&decoded, request_received_at)?;
                // A submit's answer is due by the call's own window from this
                // frame's receipt, under the same safety cap as every exchange.
                if let Some(call_deadline) = &call_deadline {
                    deadlines.response = (request_received_at + call_deadline.response_window())
                        .min(request_received_at + OWNER_RESPONSE_WRITE_TIMEOUT);
                }
                deadlines
            }
        };
        runtime.hooks().event(
            V5ReceiptRuntimeEventKind::StrictEnvelopeParsed,
            runtime.epoch_ms(),
        );
        runtime.hooks().event(
            V5ReceiptRuntimeEventKind::V5ReceiptRuntimeEntered,
            runtime.epoch_ms(),
        );
        runtime.ensure_named_authority_before(deadlines.operation)?;
        let kind = decoded.request().kind();
        let _actor_lease =
            (kind == V5ClientRequestKind::SubmitInvocation).then(|| runtime.hooks().actor_lease());
        let result = match kind {
            V5ClientRequestKind::Ping => write_runtime_json_line_before(
                &mut stream,
                runtime,
                &V5ProbeServerResponse::Pong {},
                deadlines.response,
            ),
            V5ClientRequestKind::SubmitInvocation => {
                let epoch_ms = runtime.epoch_ms();
                let call_deadline =
                    call_deadline.expect("a submit frame captured its call deadline");
                match runtime.submit_invocation(
                    DecodedSubmitAdapter::new(decoded, record.core_identity().clone()),
                    epoch_ms,
                    deadlines.durable,
                    call_deadline,
                ) {
                    Ok(reply) => {
                        if runtime.hooks().submit_response_disconnect() {
                            return Ok(());
                        }
                        // The process may have latched fail-stop while this
                        // attempt ran: its reply is still written, as the
                        // closed one this daemon admits nothing after.
                        let reply = if runtime.restart_required() {
                            reply.after_fail_stop()
                        } else {
                            reply
                        };
                        write_runtime_reply_before(&mut stream, runtime, reply, deadlines.response)
                    }
                    Err(error) => write_runtime_ledger_error_before(
                        &mut stream,
                        runtime,
                        &error,
                        deadlines.response,
                    ),
                }
            }
            V5ClientRequestKind::CancelInvocation => {
                let V5ClientRequest::CancelInvocation { receipt_key } = decoded.into_request()
                else {
                    unreachable!("request kind and decoded cancel variant diverged");
                };
                let epoch_ms = runtime.epoch_ms();
                match runtime.cancel_invocation(receipt_key, epoch_ms, deadlines.operation) {
                    Ok(reply) => {
                        if runtime.hooks().cancel_response_disconnect() {
                            return Ok(());
                        }
                        write_runtime_reply_before(&mut stream, runtime, reply, deadlines.response)
                    }
                    Err(error) => write_runtime_ledger_error_before(
                        &mut stream,
                        runtime,
                        &error,
                        deadlines.response,
                    ),
                }
            }
            V5ClientRequestKind::FenceInvocationReceipt => {
                let V5ClientRequest::FenceInvocationReceipt { receipt_key } =
                    decoded.into_request()
                else {
                    unreachable!("request kind and decoded fence variant diverged");
                };
                let epoch_ms = runtime.epoch_ms();
                match runtime.fence_invocation(receipt_key, epoch_ms, deadlines.operation) {
                    Ok(reply) => {
                        write_runtime_reply_before(&mut stream, runtime, reply, deadlines.response)
                    }
                    Err(error) => write_runtime_ledger_error_before(
                        &mut stream,
                        runtime,
                        &error,
                        deadlines.response,
                    ),
                }
            }
            V5ClientRequestKind::RecoverInvocationReceipt => {
                let V5ClientRequest::RecoverInvocationReceipt { receipt_key } =
                    decoded.into_request()
                else {
                    unreachable!("request kind and decoded recover variant diverged");
                };
                let epoch_ms = runtime.epoch_ms();
                match runtime.recover_invocation(receipt_key, epoch_ms, deadlines.operation) {
                    Ok(reply) => {
                        if runtime.hooks().recover_response_disconnect() {
                            return Ok(());
                        }
                        write_runtime_reply_before(&mut stream, runtime, reply, deadlines.response)
                    }
                    Err(error) => write_runtime_ledger_error_before(
                        &mut stream,
                        runtime,
                        &error,
                        deadlines.response,
                    ),
                }
            }
            V5ClientRequestKind::AcknowledgeInvocationReceipt => {
                let V5ClientRequest::AcknowledgeInvocationReceipt {
                    receipt_key,
                    terminal_digest,
                } = decoded.into_request()
                else {
                    unreachable!("request kind and decoded acknowledgement variant diverged");
                };
                let epoch_ms = runtime.epoch_ms();
                match runtime.acknowledge_invocation(
                    receipt_key,
                    terminal_digest,
                    epoch_ms,
                    deadlines.operation,
                ) {
                    Ok(reply) => {
                        if runtime.hooks().ack_response_disconnect() {
                            return Ok(());
                        }
                        write_runtime_reply_before(&mut stream, runtime, reply, deadlines.response)
                    }
                    Err(
                        ReceiptLedgerError::TerminalMismatch
                        | ReceiptLedgerError::ReceiptRowPresentUnsupported,
                    ) => write_runtime_probe_error_before(
                        &mut stream,
                        runtime,
                        V5DaemonErrorCode::InvalidRequest,
                        deadlines.response,
                    ),
                    Err(error) => write_runtime_ledger_error_before(
                        &mut stream,
                        runtime,
                        &error,
                        deadlines.response,
                    ),
                }
            }
            V5ClientRequestKind::GetTask => {
                let V5ClientRequest::GetTask { task_id } = decoded.into_request() else {
                    unreachable!("request kind and decoded get Task variant diverged");
                };
                match runtime.resolve_task(task_id, deadlines.operation) {
                    Ok(snapshot) => write_runtime_json_line_before(
                        &mut stream,
                        runtime,
                        &V5ServerResponse::Task { snapshot },
                        deadlines.response,
                    ),
                    Err(ReceiptLedgerError::ReceiptNotFound) => write_runtime_probe_error_before(
                        &mut stream,
                        runtime,
                        V5DaemonErrorCode::TaskNotFound,
                        deadlines.response,
                    ),
                    Err(error) => write_runtime_ledger_error_before(
                        &mut stream,
                        runtime,
                        &error,
                        deadlines.response,
                    ),
                }
            }
            V5ClientRequestKind::WaitTask => {
                let V5ClientRequest::WaitTask { task_id, wait_ms } = decoded.into_request() else {
                    unreachable!("request kind and decoded wait Task variant diverged");
                };
                runtime.hooks().wait_task_received(task_id);
                match runtime.wait_task(task_id, wait_ms, deadlines.operation) {
                    Ok(snapshot) => write_runtime_json_line_before(
                        &mut stream,
                        runtime,
                        &V5ServerResponse::Task { snapshot },
                        deadlines.response,
                    ),
                    Err(ReceiptLedgerError::ReceiptNotFound) => write_runtime_probe_error_before(
                        &mut stream,
                        runtime,
                        V5DaemonErrorCode::TaskNotFound,
                        deadlines.response,
                    ),
                    Err(error) => write_runtime_ledger_error_before(
                        &mut stream,
                        runtime,
                        &error,
                        deadlines.response,
                    ),
                }
            }
            V5ClientRequestKind::CancelTask => {
                let V5ClientRequest::CancelTask { task_id } = decoded.into_request() else {
                    unreachable!("request kind and decoded cancel Task variant diverged");
                };
                match runtime.cancel_task(task_id, deadlines.operation) {
                    Ok(snapshot) => write_runtime_json_line_before(
                        &mut stream,
                        runtime,
                        &V5ServerResponse::Task { snapshot },
                        deadlines.response,
                    ),
                    Err(ReceiptLedgerError::ReceiptNotFound) => write_runtime_probe_error_before(
                        &mut stream,
                        runtime,
                        V5DaemonErrorCode::TaskNotFound,
                        deadlines.response,
                    ),
                    Err(error) => write_runtime_ledger_error_before(
                        &mut stream,
                        runtime,
                        &error,
                        deadlines.response,
                    ),
                }
            }
            V5ClientRequestKind::Release => write_runtime_json_line_before(
                &mut stream,
                runtime,
                &V5ServerResponse::Released,
                deadlines.response,
            ),
            _ => write_runtime_probe_error_before(
                &mut stream,
                runtime,
                V5DaemonErrorCode::InvalidRequest,
                deadlines.response,
            ),
        };
        result?;
        if kind == V5ClientRequestKind::Release {
            break;
        }
    }
    Ok(())
}

struct V5RequestDeadlines {
    /// The caller's budget for the operation itself.
    operation: Instant,
    /// The bound of the request's receipt-store steps: the operation
    /// deadline, but never less than [`DURABLE_STEP_BUDGET`] after receipt.
    durable: Instant,
    /// The last moment the answer is written.
    response: Instant,
}

fn v5_request_deadlines(
    decoded: &DecodedV5Request,
    request_received_at: Instant,
) -> Result<V5RequestDeadlines, String> {
    let operation_budget = match decoded.request() {
        V5ClientRequest::SubmitInvocation { invocation } => {
            Duration::from_millis(invocation.response_budget_ms())
        }
        V5ClientRequest::WaitTask { wait_ms, .. } => {
            Duration::from_millis(*wait_ms).saturating_add(TASK_RECONCILIATION_BUDGET)
        }
        _ => SESSION_READ_TIMEOUT,
    };
    let operation = request_received_at
        .checked_add(operation_budget)
        .ok_or_else(|| "protocol-v5 operation deadline overflow".to_owned())?;
    let response = operation
        .checked_add(RESPONSE_SERIALIZATION_MARGIN)
        .ok_or_else(|| "protocol-v5 response deadline overflow".to_owned())?
        .min(
            request_received_at
                .checked_add(OWNER_RESPONSE_WRITE_TIMEOUT)
                .ok_or_else(|| "protocol-v5 response safety deadline overflow".to_owned())?,
        );
    let durable = request_received_at
        .checked_add(DURABLE_STEP_BUDGET)
        .ok_or_else(|| "protocol-v5 durable step deadline overflow".to_owned())?
        .max(operation);
    Ok(V5RequestDeadlines {
        operation,
        durable,
        response,
    })
}

type V5FrameRead = fn(
    &mut BufReader<TcpStream>,
    &mut dyn FnMut(&mut BufReader<TcpStream>) -> io::Result<()>,
) -> io::Result<Vec<u8>>;

/// The first frame of a connection comes before its token is checked and is
/// held to the small handshake bound.
fn read_v5_handshake_before(
    reader: &mut BufReader<TcpStream>,
    deadline: Instant,
) -> Result<(DecodedV5Request, Instant), V5RequestFrameError> {
    read_v5_frame_before(reader, deadline, |reader, before_fill| {
        read_bounded_v5_handshake_frame_before(reader, before_fill)
    })
}

fn read_v5_request_before(
    reader: &mut BufReader<TcpStream>,
    deadline: Instant,
) -> Result<(DecodedV5Request, Instant), V5RequestFrameError> {
    read_v5_frame_before(reader, deadline, |reader, before_fill| {
        read_bounded_v5_request_frame_before(reader, before_fill)
    })
}

/// An idle session waits for the first byte of a frame until `idle_deadline`
/// and is then polled again. Once that byte arrived, the frame gets its own
/// absolute deadline, which later bytes do not extend. A frame that began just
/// before the idle poll ended would otherwise lose its consumed prefix and
/// have its tail read as a new request; a large frame made that likely (#1309).
/// A frame that misses its own deadline ends the session: its tail is never
/// parsed as another request.
fn read_v5_session_request(
    reader: &mut BufReader<TcpStream>,
    idle_deadline: Instant,
) -> Result<(DecodedV5Request, Instant), V5RequestFrameError> {
    while reader.buffer().is_empty() {
        let remaining = idle_deadline
            .checked_duration_since(Instant::now())
            .filter(|remaining| !remaining.is_zero())
            .ok_or_else(|| V5RequestFrameError::Read(io::Error::from(io::ErrorKind::TimedOut)))?;
        reader
            .get_ref()
            .set_read_timeout(Some(remaining))
            .map_err(V5RequestFrameError::Read)?;
        match std::io::BufRead::fill_buf(reader) {
            Ok([]) => {
                return Err(V5RequestFrameError::Read(io::Error::from(
                    io::ErrorKind::UnexpectedEof,
                )))
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(V5RequestFrameError::Read(error)),
        }
    }
    read_v5_request_before(reader, Instant::now() + SESSION_FRAME_READ_TIMEOUT).map_err(|error| {
        match error {
            V5RequestFrameError::Read(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                V5RequestFrameError::Read(io::Error::other(
                    "protocol-v5 session frame missed its read deadline",
                ))
            }
            other => other,
        }
    })
}

fn read_v5_frame_before(
    reader: &mut BufReader<TcpStream>,
    deadline: Instant,
    read_frame: V5FrameRead,
) -> Result<(DecodedV5Request, Instant), V5RequestFrameError> {
    let raw_frame = read_frame(reader, &mut |reader| {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|remaining| !remaining.is_zero())
            .ok_or_else(|| io::Error::from(io::ErrorKind::TimedOut))?;
        reader.get_ref().set_read_timeout(Some(remaining))
    })
    .map_err(V5RequestFrameError::Read)?;
    let request_received_at = Instant::now();
    if request_received_at >= deadline {
        return Err(V5RequestFrameError::Read(io::Error::from(
            io::ErrorKind::TimedOut,
        )));
    }
    let decoded = decode_v5_request_frame(raw_frame)?;
    Ok((decoded, request_received_at))
}

fn write_runtime_probe_error_before<H: InvocationHost<Request = V5InvocationRequest>>(
    stream: &mut TcpStream,
    runtime: &V5ReceiptRuntime<H>,
    code: V5DaemonErrorCode,
    deadline: Instant,
) -> Result<(), String> {
    write_runtime_json_line_before(stream, runtime, &V5ServerResponse::Error { code }, deadline)
}

fn write_runtime_ledger_error_before<H: InvocationHost<Request = V5InvocationRequest>>(
    stream: &mut TcpStream,
    runtime: &V5ReceiptRuntime<H>,
    error: &ReceiptLedgerError,
    deadline: Instant,
) -> Result<(), String> {
    let response = V5ServerResponse::Error {
        code: daemon_error_code(error),
    };
    if error.requires_reopen() {
        runtime.hooks().forced_process_exit(None);
        // The actor has already latched fail-stop, so asking it for another
        // generation check would turn the required closed response into EOF.
        // The runtime still owns the authenticated stream, PID endpoint,
        // listener and process-scoped authority at this point. A running
        // mutation may be classified only after the original response cutoff
        // when the caller is descheduled, so its one transport margin starts
        // when that fail-stop result is observed.
        write_fail_stop_json_line(stream, &response, deadline)
    } else {
        write_runtime_json_line_before(stream, runtime, &response, deadline)
    }
}

fn fail_stop_response_write_timeout(
    original_response_deadline: Instant,
    observed_at: Instant,
) -> Duration {
    original_response_deadline
        .saturating_duration_since(observed_at)
        .max(RESPONSE_SERIALIZATION_MARGIN)
        .min(OWNER_RESPONSE_WRITE_TIMEOUT)
}

fn write_fail_stop_json_line<T: Serialize>(
    stream: &mut TcpStream,
    value: &T,
    original_response_deadline: Instant,
) -> Result<(), String> {
    // Serialize before deriving the transport timeout. The operation timeout
    // may be observed late under scheduler pressure, and an absolute deadline
    // computed before serialization lets that work (or simple descheduling)
    // consume the entire transport margin before `write(2)` starts.
    let bytes = encode_json_line(value)?;
    let write_timeout =
        fail_stop_response_write_timeout(original_response_deadline, Instant::now());
    stream
        .set_write_timeout(Some(write_timeout))
        .map_err(|error| {
            state_io_error("configure fail-stop protocol-v5 response timeout", error)
        })?;
    stream
        .write_all(&bytes)
        .map_err(|error| state_io_error("write fail-stop protocol-v5 response", error))
}

fn write_runtime_reply_before<H: InvocationHost<Request = V5InvocationRequest>>(
    stream: &mut TcpStream,
    runtime: &V5ReceiptRuntime<H>,
    reply: ReceiptReply,
    deadline: Instant,
) -> Result<(), String> {
    match reply {
        ReceiptReply::Json(response) => {
            let response = outer_response(response);
            write_runtime_json_line_before(stream, runtime, &response, deadline)
        }
        ReceiptReply::JsonFailStop(response) => {
            let response = outer_response(response);
            write_fail_stop_json_line(stream, &response, deadline)
        }
        ReceiptReply::Prepared(frame) => {
            runtime.ensure_named_authority_before(deadline)?;
            if frame.jsonl().len() > MAX_V5_RESPONSE_LINE_BYTES {
                return Err("prepared protocol-v5 response exceeds the byte limit".to_string());
            }
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .filter(|remaining| !remaining.is_zero())
                .ok_or_else(|| "protocol-v5 response deadline expired".to_string())?;
            stream.set_write_timeout(Some(remaining)).map_err(|error| {
                state_io_error("configure prepared protocol-v5 response timeout", error)
            })?;
            stream
                .write_all(frame.jsonl())
                .map_err(|error| state_io_error("write prepared protocol-v5 response", error))?;
            deadline
                .checked_duration_since(Instant::now())
                .filter(|remaining| !remaining.is_zero())
                .map(|_| ())
                .ok_or_else(|| "protocol-v5 response deadline expired".to_string())
        }
        ReceiptReply::PreparedFailStop(frame) => {
            if frame.jsonl().len() > MAX_V5_RESPONSE_LINE_BYTES {
                return Err("prepared protocol-v5 response exceeds the byte limit".to_string());
            }
            let write_timeout = fail_stop_response_write_timeout(deadline, Instant::now());
            stream
                .set_write_timeout(Some(write_timeout))
                .map_err(|error| {
                    state_io_error("configure fail-stop prepared response timeout", error)
                })?;
            stream
                .write_all(frame.jsonl())
                .map_err(|error| state_io_error("write fail-stop prepared response", error))
        }
    }
}

fn outer_response(response: ReceiptResponse) -> V5ServerResponse {
    match response {
        ReceiptResponse::Invocation { outcome } => V5ServerResponse::Invocation { outcome },
        ReceiptResponse::Task { snapshot } => V5ServerResponse::Task { snapshot },
        ReceiptResponse::InvocationAcknowledged { acknowledgement } => {
            V5ServerResponse::InvocationAcknowledged { acknowledgement }
        }
    }
}

fn daemon_error_code(error: &ReceiptLedgerError) -> V5DaemonErrorCode {
    match error {
        ReceiptLedgerError::InvocationIdentityMismatch
        | ReceiptLedgerError::ReservedTaskIdentityMismatch => {
            V5DaemonErrorCode::InvocationIdentityMismatch
        }
        ReceiptLedgerError::ReceiptNotFound => V5DaemonErrorCode::ReceiptNotFound,
        ReceiptLedgerError::CapacityExceeded => V5DaemonErrorCode::ReceiptCapacity,
        ReceiptLedgerError::TombstoneCapacityExceeded => V5DaemonErrorCode::TombstoneCapacity,
        ReceiptLedgerError::CommitUncertain { .. } => V5DaemonErrorCode::StoreCommitUncertain,
        ReceiptLedgerError::DeadlineExceeded => V5DaemonErrorCode::Overloaded,
        ReceiptLedgerError::AlreadyOwned => V5DaemonErrorCode::DuplicateLease,
        ReceiptLedgerError::EmptyBatch
        | ReceiptLedgerError::TerminalMismatch
        | ReceiptLedgerError::ReceiptRowPresentUnsupported => V5DaemonErrorCode::InvalidRequest,
        ReceiptLedgerError::RecordTooLarge
        | ReceiptLedgerError::TimestampOverflow
        | ReceiptLedgerError::ReceiptVersionMismatch { .. }
        | ReceiptLedgerError::ReceiptMutationSequenceMismatch { .. }
        | ReceiptLedgerError::ReceiptDigestCollision
        | ReceiptLedgerError::TaskBoundMismatch
        | ReceiptLedgerError::TaskCancellationMismatch
        | ReceiptLedgerError::StoreUnavailable
        | ReceiptLedgerError::ConcurrentGenerationChange { .. }
        | ReceiptLedgerError::Corrupt(_)
        | ReceiptLedgerError::Storage { .. } => V5DaemonErrorCode::StoreFailed,
    }
}

fn write_runtime_json_line_before<
    H: InvocationHost<Request = V5InvocationRequest>,
    T: Serialize,
>(
    stream: &mut TcpStream,
    runtime: &V5ReceiptRuntime<H>,
    value: &T,
    deadline: Instant,
) -> Result<(), String> {
    runtime.ensure_named_authority_before(deadline)?;
    write_json_line_before(stream, value, deadline)
}

fn write_json_line_before<T: Serialize>(
    stream: &mut TcpStream,
    value: &T,
    deadline: Instant,
) -> Result<(), String> {
    let bytes = encode_json_line(value)?;
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| "protocol-v5 response deadline expired".to_string())?;
    stream
        .set_write_timeout(Some(remaining))
        .map_err(|error| state_io_error("configure protocol-v5 response timeout", error))?;
    stream
        .write_all(&bytes)
        .map_err(|error| state_io_error("write protocol-v5 response", error))?;
    deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .map(|_| ())
        .ok_or_else(|| "protocol-v5 response deadline expired".to_string())
}

fn encode_json_line<T: Serialize>(value: &T) -> Result<Vec<u8>, String> {
    let mut bytes = serde_json::to_vec(value)
        .map_err(|_| "protocol-v5 response could not be serialized".to_string())?;
    bytes.push(b'\n');
    if bytes.len() > MAX_V5_RESPONSE_LINE_BYTES {
        return Err("protocol-v5 response exceeds the byte limit".to_string());
    }
    Ok(bytes)
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

#[cfg(test)]
#[path = "../tests/platform/listener.rs"]
mod tests;

#[cfg(test)]
#[path = "../tests/pure/wire.rs"]
mod wire_tests;
