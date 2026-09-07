//! Canonical v0.13 router over the protocol-v5 user daemon.
//!
//! The stdio frontend keeps one anchor session for its lifetime and gives every
//! invocation and every Task operation a peer session of its own. A Direct
//! terminal is acknowledged only after its host-facing value exists; a lost
//! submit response is recovered by the exact receipt key the frontend derives
//! itself, never by a second submission.
use super::task_projection::{self, DirectProjection};
use crate::application::invocation::{
    handoff_budget, normalized_arguments_hash, INVOCATION_HANDOFF_WINDOW,
    RESPONSE_SERIALIZATION_MARGIN, RESPONSE_SERIALIZATION_MARGIN_MS,
};
use crate::application::receipt_ledger::{
    request_scope_hash, ReceiptKey, RequestIdentity, V5ToolIdentity,
    MAX_ORIGINAL_RESPONSE_BUDGET_MS,
};
use crate::domain::cancellation::CancellationToken;
use crate::domain::invocation::{InvocationId, TaskId};
use crate::infrastructure::daemon::client_v5::{
    V5DaemonProcessOwner, V5TaskExchangeError, V5TransportError,
};
use crate::infrastructure::daemon::protocol_v5::{
    V5DaemonErrorCode, V5DaemonTaskSnapshot, V5InvocationRequest, V5InvocationResponse,
    V5ServerResponse,
};
use rmcp::model::{CallToolResult, ErrorCode, ErrorData};
use serde_json::{Map, Value};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub(super) const TOOL_EXECUTION_ERROR: i32 = -32000;
/// The acknowledgement travels after the transport budget of the invocation
/// may already be spent; it gets a short budget of its own instead of racing
/// an exhausted cutoff.
const ACKNOWLEDGEMENT_BUDGET: Duration = Duration::from_millis(500);
/// Recovery of a still-reserved receipt polls the daemon no more often than this.
const RECOVERY_POLL_INTERVAL: Duration = Duration::from_millis(25);
/// A submit response is most often lost exactly at the frontend cutoff: the
/// daemon hands the work off to a Task within its own budget, and the durable
/// promotion under load overruns the 125 ms margin. Exact recovery is a
/// read-only reconciliation of that receipt, so it gets a bounded window of
/// its own after the cutoff instead of failing with nothing left to spend.
const RECOVERY_BUDGET: Duration = Duration::from_millis(750);

/// One absolute frontend budget per request: captured at receipt and only
/// narrowed afterwards, never re-derived from a later `now`.
#[derive(Debug, Clone, Copy)]
pub(super) struct FrontendInvocationDeadline {
    received_at: Instant,
    host_remaining_at_receipt: Option<Duration>,
}

impl FrontendInvocationDeadline {
    pub(super) fn new(received_at: Instant, host_remaining_at_receipt: Option<Duration>) -> Self {
        Self {
            received_at,
            host_remaining_at_receipt,
        }
    }

    pub(super) fn received_at(self) -> Instant {
        self.received_at
    }

    pub(super) fn remaining_at(self, now: Instant) -> Duration {
        remaining_invocation_budget(self.received_at, now, self.host_remaining_at_receipt)
    }

    #[cfg(test)]
    pub(super) fn remaining_transport_at(self, now: Instant) -> Duration {
        let elapsed = now.saturating_duration_since(self.received_at);
        let own_remaining = INVOCATION_HANDOFF_WINDOW
            .saturating_add(RESPONSE_SERIALIZATION_MARGIN)
            .saturating_sub(elapsed);
        let host_remaining = self
            .host_remaining_at_receipt
            .map(|remaining| remaining.saturating_sub(elapsed));
        host_remaining.map_or(own_remaining, |remaining| own_remaining.min(remaining))
    }

    pub(super) fn transport_cutoff(self) -> Instant {
        let own_cutoff = self
            .received_at
            .checked_add(INVOCATION_HANDOFF_WINDOW.saturating_add(RESPONSE_SERIALIZATION_MARGIN))
            .expect("bounded frontend transport cutoff");
        self.host_remaining_at_receipt
            .and_then(|remaining| self.received_at.checked_add(remaining))
            .map_or(own_cutoff, |host_cutoff| own_cutoff.min(host_cutoff))
    }
}

pub(super) fn remaining_invocation_budget(
    received_at: Instant,
    now: Instant,
    host_remaining_at_receipt: Option<Duration>,
) -> Duration {
    let elapsed = now.saturating_duration_since(received_at);
    let own_remaining = INVOCATION_HANDOFF_WINDOW.saturating_sub(elapsed);
    let host_remaining =
        host_remaining_at_receipt.map(|remaining| remaining.saturating_sub(elapsed));
    own_remaining.min(handoff_budget(host_remaining))
}

/// What one canonical call produced: the final `CallToolResult` of an
/// acknowledged Direct terminal, or the durable Task the daemon handed off to.
#[derive(Debug)]
pub(super) enum CanonicalCallOutcome {
    Direct(CallToolResult),
    Task(V5DaemonTaskSnapshot),
}

pub(super) type CanonicalCallHandler = dyn Fn(
        V5ToolIdentity,
        &Map<String, Value>,
        FrontendInvocationDeadline,
        CancellationToken,
    ) -> Result<CanonicalCallOutcome, ErrorData>
    + Send
    + Sync;

pub(super) type CanonicalTaskHandler = dyn Fn(TaskId, FrontendInvocationDeadline) -> Result<V5DaemonTaskSnapshot, V5TaskExchangeError>
    + Send
    + Sync;

pub(super) type CanonicalTaskWaitHandler = dyn Fn(TaskId, u64, FrontendInvocationDeadline) -> Result<V5DaemonTaskSnapshot, V5TaskExchangeError>
    + Send
    + Sync;

#[derive(Clone)]
pub(super) struct CanonicalDaemonRouter {
    pub(super) call: Arc<CanonicalCallHandler>,
    pub(super) get: Arc<CanonicalTaskHandler>,
    pub(super) wait: Arc<CanonicalTaskWaitHandler>,
    pub(super) cancel: Arc<CanonicalTaskHandler>,
}

type ReceiptObserver = Arc<dyn Fn(&ReceiptKey) + Send + Sync>;

/// Build the router over one persistent protocol-v5 owner lease.
pub(super) fn canonical_daemon_router(
    owner: V5DaemonProcessOwner,
    workspace_hint: String,
) -> CanonicalDaemonRouter {
    build_router(owner, workspace_hint, None)
}

#[cfg(test)]
fn canonical_daemon_router_observed(
    owner: V5DaemonProcessOwner,
    workspace_hint: String,
    observer: ReceiptObserver,
) -> CanonicalDaemonRouter {
    build_router(owner, workspace_hint, Some(observer))
}

fn build_router(
    owner: V5DaemonProcessOwner,
    workspace_hint: String,
    observer: Option<ReceiptObserver>,
) -> CanonicalDaemonRouter {
    // The anchor holds the owner lease for the frontend lifetime; every
    // invocation runs on its own peer session so a slow direct response cannot
    // serialize another call behind it.
    let anchor = Arc::new(owner);
    let call_anchor = Arc::clone(&anchor);
    let call: Arc<CanonicalCallHandler> =
        Arc::new(move |tool, arguments, deadline, _cancellation| {
            submit_and_settle(
                &call_anchor,
                &workspace_hint,
                tool,
                arguments,
                deadline,
                observer.as_ref(),
            )
        });
    let get_anchor = Arc::clone(&anchor);
    let get: Arc<CanonicalTaskHandler> = Arc::new(move |task_id, deadline| {
        let cutoff = deadline.transport_cutoff();
        let mut peer = task_peer(&get_anchor, cutoff)?;
        peer.get_task_before(task_id, cutoff)
    });
    let wait_anchor = Arc::clone(&anchor);
    let wait: Arc<CanonicalTaskWaitHandler> = Arc::new(move |task_id, wait_ms, deadline| {
        let cutoff = wait_transport_cutoff(wait_ms, deadline);
        let mut peer = task_peer(&wait_anchor, cutoff)?;
        // The daemon wait shrinks by what connect and handshake already spent
        // and by the response margin; the cutoff itself never moves.
        let bounded_wait_ms = bounded_wait_ms(wait_ms, cutoff, Instant::now());
        peer.wait_task_before(task_id, bounded_wait_ms, cutoff)
    });
    let cancel: Arc<CanonicalTaskHandler> = Arc::new(move |task_id, deadline| {
        let cutoff = deadline.transport_cutoff();
        let mut peer = task_peer(&anchor, cutoff)?;
        peer.cancel_task_before(task_id, cutoff)
    });
    CanonicalDaemonRouter {
        call,
        get,
        wait,
        cancel,
    }
}

fn task_peer(
    anchor: &V5DaemonProcessOwner,
    cutoff: Instant,
) -> Result<V5DaemonProcessOwner, V5TaskExchangeError> {
    if Instant::now() >= cutoff {
        return Err(V5TaskExchangeError::Transport);
    }
    anchor
        .connect_peer_before(cutoff)
        .map_err(V5TaskExchangeError::from)
}

/// `unica.task.result` gets its own cutoff, derived once from the moment the
/// frontend received the request: the requested wait plus one response
/// margin, never beyond the frontend transport cutoff. Time spent before the
/// router runs is never replenished.
pub(super) fn wait_transport_cutoff(
    requested_wait_ms: u64,
    deadline: FrontendInvocationDeadline,
) -> Instant {
    let requested_cutoff = deadline
        .received_at()
        .checked_add(
            Duration::from_millis(requested_wait_ms).saturating_add(RESPONSE_SERIALIZATION_MARGIN),
        )
        .expect("bounded task wait cutoff");
    requested_cutoff.min(deadline.transport_cutoff())
}

fn bounded_wait_ms(requested_wait_ms: u64, cutoff: Instant, now: Instant) -> u64 {
    let remaining = cutoff
        .saturating_duration_since(now)
        .saturating_sub(RESPONSE_SERIALIZATION_MARGIN);
    requested_wait_ms.min(remaining.as_millis().min(u128::from(u64::MAX)) as u64)
}

fn submit_and_settle(
    anchor: &V5DaemonProcessOwner,
    workspace_hint: &str,
    tool: V5ToolIdentity,
    arguments: &Map<String, Value>,
    deadline: FrontendInvocationDeadline,
    observer: Option<&ReceiptObserver>,
) -> Result<CanonicalCallOutcome, ErrorData> {
    let cutoff = deadline.transport_cutoff();
    let response_budget_ms = deadline
        .remaining_at(Instant::now())
        .as_millis()
        .min(u128::from(MAX_ORIGINAL_RESPONSE_BUDGET_MS)) as u64;
    // Fresh identities for every call: an exact identity is never reused, so a
    // retry by the host is a new invocation with a new receipt.
    let invocation = V5InvocationRequest::new(
        InvocationId::new(),
        TaskId::new(),
        tool,
        arguments.clone(),
        workspace_hint.to_owned(),
        response_budget_ms,
    )
    .map_err(|message| ErrorData::invalid_params(message, None))?;
    let receipt_key = receipt_key_for(&invocation, anchor)?;
    if let Some(observer) = observer {
        observer(&receipt_key);
    }
    let mut peer = anchor
        .connect_peer_before(cutoff)
        .map_err(transport_refusal)?;
    let (peer, response) = match peer.submit_invocation_before(invocation, cutoff) {
        Ok(response) => (peer, response),
        // The frame reached the daemon and the answer did not come back: the
        // daemon may already hold a reserved or terminal receipt for it.
        Err(V5TransportError::ResponseLost(_)) => {
            let recovery_cutoff = cutoff.max(Instant::now()) + RECOVERY_BUDGET;
            recover_receipt(anchor, &receipt_key, recovery_cutoff)?
        }
        Err(error) => return Err(transport_refusal(error)),
    };
    settle(peer, response, &receipt_key)
}

/// The exact key the daemon derives from the same submission
/// (`receipt_key_is_canonicalized_identically_by_client_and_server`).
fn receipt_key_for(
    invocation: &V5InvocationRequest,
    anchor: &V5DaemonProcessOwner,
) -> Result<ReceiptKey, ErrorData> {
    let scope = request_scope_hash(invocation.workspace_hint()).map_err(|_| {
        ErrorData::invalid_params("workspace hint is not a valid request scope", None)
    })?;
    Ok(ReceiptKey::new(
        invocation.invocation_id(),
        invocation.reserved_task_id(),
        RequestIdentity::new(
            anchor.core_identity().digest().clone(),
            invocation.tool(),
            normalized_arguments_hash(invocation.arguments()),
            scope,
        ),
    ))
}

/// Read the durable state of a submission whose response was lost. A receipt
/// still inside its original budget is polled until that budget settles;
/// nothing here opens a new invocation budget or submits again. When even the
/// recovery window closes, the host gets the same closed refusal the v3
/// frontend answered with: the daemon may hold the work, nobody replays it.
fn recover_receipt(
    anchor: &V5DaemonProcessOwner,
    receipt_key: &ReceiptKey,
    cutoff: Instant,
) -> Result<(V5DaemonProcessOwner, V5ServerResponse), ErrorData> {
    loop {
        let mut peer = anchor
            .connect_peer_before(cutoff)
            .map_err(|_| lost_submit_refusal())?;
        let response = peer
            .recover_invocation_receipt_before(receipt_key.clone(), cutoff)
            .map_err(|_| lost_submit_refusal())?;
        match response {
            V5ServerResponse::Invocation {
                outcome:
                    V5InvocationResponse::ReceiptPending {
                        accepted_epoch_ms,
                        original_budget_ms,
                        ..
                    },
            } => {
                let settles_at_epoch_ms = accepted_epoch_ms
                    .saturating_add(original_budget_ms)
                    .saturating_add(RESPONSE_SERIALIZATION_MARGIN_MS);
                let until_settled =
                    Duration::from_millis(settles_at_epoch_ms.saturating_sub(epoch_ms_now()));
                let remaining = cutoff.saturating_duration_since(Instant::now());
                // A receipt that settles after the frontend cutoff cannot be
                // delivered by this call; the daemon keeps it, nobody resubmits.
                if until_settled >= remaining || remaining <= RECOVERY_POLL_INTERVAL {
                    return Err(receipt_pending_refusal());
                }
                std::thread::sleep(until_settled.max(RECOVERY_POLL_INTERVAL));
            }
            // The daemon holds no receipt: the submission never reached its
            // reserve, which is the same loss the host already knows how to
            // treat.
            V5ServerResponse::Error {
                code: V5DaemonErrorCode::ReceiptNotFound,
            } => return Err(lost_submit_refusal()),
            other => return Ok((peer, other)),
        }
    }
}

fn settle(
    mut peer: V5DaemonProcessOwner,
    response: V5ServerResponse,
    receipt_key: &ReceiptKey,
) -> Result<CanonicalCallOutcome, ErrorData> {
    match response {
        V5ServerResponse::Invocation {
            outcome: V5InvocationResponse::Direct { receipt },
        } => {
            if receipt.receipt_key() != receipt_key {
                return Err(protocol_refusal(
                    "daemon answered with a receipt of a different invocation",
                ));
            }
            // The final interface value is built first; a receipt whose value
            // cannot be built stays unacknowledged and expires on its own.
            let projection = task_projection::project_direct_terminal(receipt.terminal())
                .map_err(task_projection::projection_error)?;
            let acknowledgement_deadline = Instant::now() + ACKNOWLEDGEMENT_BUDGET;
            // The acknowledgement proves the transfer daemon → frontend and
            // nothing more: its own outcome cannot change what the host gets.
            let _ = peer.acknowledge_invocation_receipt_before(
                receipt_key.clone(),
                receipt.terminal_digest().clone(),
                acknowledgement_deadline,
            );
            match projection {
                DirectProjection::Result(result) => Ok(CanonicalCallOutcome::Direct(result)),
                DirectProjection::Error(error) => Err(error),
            }
        }
        V5ServerResponse::Invocation {
            outcome: V5InvocationResponse::Task { snapshot },
        } => Ok(CanonicalCallOutcome::Task(snapshot)),
        V5ServerResponse::Invocation {
            outcome: V5InvocationResponse::ReceiptPending { .. },
        } => Err(receipt_pending_refusal()),
        V5ServerResponse::Invocation {
            outcome: V5InvocationResponse::Acknowledged { .. },
        } => Err(protocol_refusal(
            "daemon reported an acknowledged receipt before its result was delivered",
        )),
        V5ServerResponse::Error { code } => Err(submission_rejected(code)),
        V5ServerResponse::Ready { .. }
        | V5ServerResponse::Pong
        | V5ServerResponse::Released
        | V5ServerResponse::Task { .. }
        | V5ServerResponse::InvocationAcknowledged { .. } => Err(protocol_refusal(
            "daemon returned an unexpected response to an invocation",
        )),
    }
}

fn epoch_ms_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

/// The closed answer for a submit whose response was lost and whose receipt
/// could not be recovered in time. Same code and text as the v3 frontend, so
/// hosts keep one rule: a read-only tool may be replayed, a mutation may not.
fn lost_submit_refusal() -> ErrorData {
    ErrorData::new(
        ErrorCode(TOOL_EXECUTION_ERROR),
        "daemon deadline expired during invocation submit response",
        None,
    )
}

fn transport_refusal(error: V5TransportError) -> ErrorData {
    ErrorData::new(ErrorCode(TOOL_EXECUTION_ERROR), error.to_string(), None)
}

fn submission_rejected(code: V5DaemonErrorCode) -> ErrorData {
    ErrorData::new(
        ErrorCode(TOOL_EXECUTION_ERROR),
        format!("daemon invocation submission rejected: {code}"),
        None,
    )
}

fn protocol_refusal(message: &'static str) -> ErrorData {
    ErrorData::new(
        ErrorCode::INTERNAL_ERROR,
        message,
        Some(serde_json::json!({"code": "daemon_protocol_failed"})),
    )
}

fn receipt_pending_refusal() -> ErrorData {
    ErrorData::new(
        ErrorCode::INTERNAL_ERROR,
        "daemon invocation receipt is still pending at the frontend cutoff",
        Some(serde_json::json!({"code": "receipt_pending"})),
    )
}

#[cfg(test)]
pub(in crate::interfaces) mod test_support {
    //! Two daemons for the frontend tests: a scripted fake that answers frame
    //! by frame, and the real protocol-v5 runtime in a thread with an injected
    //! canonical service.
    use super::*;
    use crate::application::receipt_ledger::{canonical_v5_terminal, ReceiptTerminalOutcome};
    use crate::domain::invocation::DomainResult;
    use crate::infrastructure::daemon::identity::{CoreIdentity, DaemonStateDirectory};
    use crate::infrastructure::daemon::protocol_v5::{
        decode_v5_request_frame, read_bounded_v5_request_frame, V5ClientRequest, V5EndpointRecord,
        V5HandshakeServerResponse, V5PendingDirectReceipt,
    };
    use crate::infrastructure::daemon::server::DaemonServerConfig;
    use std::io::{BufReader, Write};
    use std::net::{Ipv4Addr, TcpListener, TcpStream};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;
    use std::thread;

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

        fn start_session_script(script: SessionScript, handshake_delay: Duration) -> Self {
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
                    thread_sessions.fetch_add(1, Ordering::SeqCst);
                    let record = thread_record.clone();
                    let seen = Arc::clone(&thread_seen);
                    let script = Arc::clone(&script);
                    thread::spawn(move || {
                        serve_session(stream, record, seen, script, handshake_delay)
                    });
                }
            });
            Self {
                record,
                seen,
                sessions,
                thread: Some(thread),
            }
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

    pub(in crate::interfaces) fn write_frame<T: serde::Serialize>(
        stream: &mut TcpStream,
        value: &T,
    ) {
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
        let canonical = canonical_v5_terminal(&terminal).expect("canonical terminal");
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
        pub(in crate::interfaces) outcome:
            Mutex<Option<Result<DomainResult, crate::domain::invocation::InvocationFailure>>>,
        pub(in crate::interfaces) executions: AtomicUsize,
    }

    impl crate::infrastructure::daemon::server::CanonicalInvocationService for ScriptedService {
        fn prepare(
            &self,
            _invocation: &crate::infrastructure::daemon::server::ActorBoundInvocation,
        ) -> Result<crate::application::operation_descriptors::ExecutionClass, Box<DomainResult>>
        {
            use crate::application::operation_descriptors::{ExecutionClass, KnownLongReason};
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
        ) -> Result<DomainResult, crate::domain::invocation::InvocationFailure> {
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
    }

    impl LiveDaemon {
        pub(in crate::interfaces) fn start(
            service: Arc<dyn crate::infrastructure::daemon::server::CanonicalInvocationService>,
        ) -> Self {
            let state = tempfile::tempdir().unwrap();
            let workspace = tempfile::tempdir().unwrap();
            std::fs::write(workspace.path().join("v8project.yaml"), WORKSPACE_MANIFEST).unwrap();
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
            self.thread = Some(thread::spawn(move || {
                crate::infrastructure::daemon::runtime_v5::run_daemon(config)
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
}

#[cfg(test)]
mod tests {
    use super::test_support::{
        completed, daemon_side_key, direct_receipt, FakeDaemon, LiveDaemon, ScriptedService, Step,
    };
    use super::*;
    use crate::application::receipt_ledger::{canonical_v5_terminal, ReceiptTerminalOutcome};
    use crate::infrastructure::daemon::protocol_v5::{V5ClientRequest, V5PendingDirectReceipt};
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;
    use std::thread;

    fn arguments() -> Map<String, Value> {
        json!({"at": "main:Catalog.Товары"})
            .as_object()
            .expect("argument object")
            .clone()
    }

    fn deadline(host_remaining: Option<Duration>) -> FrontendInvocationDeadline {
        FrontendInvocationDeadline::new(Instant::now(), host_remaining)
    }

    fn call(
        router: &CanonicalDaemonRouter,
        host_remaining: Option<Duration>,
    ) -> Result<CanonicalCallOutcome, ErrorData> {
        (router.call)(
            V5ToolIdentity::View,
            &arguments(),
            deadline(host_remaining),
            CancellationToken::new(),
        )
    }

    fn direct_result(outcome: Result<CanonicalCallOutcome, ErrorData>) -> CallToolResult {
        match outcome {
            Ok(CanonicalCallOutcome::Direct(result)) => result,
            Ok(CanonicalCallOutcome::Task(snapshot)) => {
                panic!("unexpected Task handoff: {snapshot:?}")
            }
            Err(error) => panic!("unexpected refusal: {error}"),
        }
    }

    #[test]
    fn direct_terminal_is_projected_before_it_is_acknowledged_with_the_exact_digest() {
        let fake = FakeDaemon::start(Box::new(|raw, request| match request {
            V5ClientRequest::SubmitInvocation { .. } => {
                Step::Reply(direct_receipt(daemon_side_key(raw), completed("viewed")))
            }
            V5ClientRequest::AcknowledgeInvocationReceipt { .. } => {
                Step::Reply(V5ServerResponse::Error {
                    code: V5DaemonErrorCode::TombstoneCapacity,
                })
            }
            other => panic!("unexpected frame {other:?}"),
        }));
        let router = canonical_daemon_router(fake.owner(), "/workspace".to_string());

        let result = direct_result(call(&router, None));

        assert_eq!(result.is_error, Some(false));
        assert!(result.content.is_empty());
        assert_eq!(
            result
                .structured_content
                .as_ref()
                .and_then(|value| value["summary"].as_str()),
            Some("viewed")
        );
        let seen = fake.seen();
        assert_eq!(seen.len(), 2, "submit then acknowledgement: {seen:?}");
        let V5ClientRequest::SubmitInvocation { invocation } = &seen[0] else {
            panic!("first frame must be the submission");
        };
        let V5ClientRequest::AcknowledgeInvocationReceipt {
            receipt_key,
            terminal_digest,
        } = &seen[1]
        else {
            panic!("second frame must be the acknowledgement");
        };
        assert_eq!(receipt_key.invocation_id(), invocation.invocation_id());
        assert_eq!(
            receipt_key.reserved_task_id(),
            invocation.reserved_task_id()
        );
        assert_eq!(
            terminal_digest,
            canonical_v5_terminal(&completed("viewed"))
                .unwrap()
                .digest()
        );
        assert!(
            (6_900..=7_000).contains(&invocation.response_budget_ms()),
            "the daemon budget is the remaining handoff window: {}",
            invocation.response_budget_ms()
        );
        assert_eq!(invocation.workspace_hint(), "/workspace");
    }

    #[test]
    fn failed_and_cancelled_direct_terminals_answer_closed_errors_after_acknowledgement() {
        use crate::application::invocation_store_v5::V5SafeFailureReason;

        let terminals = Arc::new(Mutex::new(vec![
            ReceiptTerminalOutcome::Cancelled,
            ReceiptTerminalOutcome::Failed {
                reason: V5SafeFailureReason::OutcomeUncertain,
            },
        ]));
        let script_terminals = Arc::clone(&terminals);
        let fake = FakeDaemon::start(Box::new(move |raw, request| match request {
            V5ClientRequest::SubmitInvocation { .. } => {
                let terminal = script_terminals.lock().unwrap().pop().unwrap();
                Step::Reply(direct_receipt(daemon_side_key(raw), terminal))
            }
            V5ClientRequest::AcknowledgeInvocationReceipt { .. } => Step::Close,
            other => panic!("unexpected frame {other:?}"),
        }));
        let router = canonical_daemon_router(fake.owner(), "/workspace".to_string());

        let uncertain = call(&router, None).expect_err("failed terminal is an error");
        let cancelled = call(&router, None).expect_err("cancelled terminal is an error");

        assert_eq!(uncertain.code, ErrorCode::INTERNAL_ERROR);
        assert_eq!(uncertain.message, "daemon invocation outcome is uncertain");
        assert_eq!(uncertain.data, Some(json!({"code": "outcome_uncertain"})));
        assert_eq!(cancelled.message, "daemon invocation was cancelled");
        assert_eq!(
            cancelled.data,
            Some(json!({"code": "invocation_cancelled"}))
        );
        let acknowledgements = fake
            .seen()
            .iter()
            .filter(|request| {
                matches!(
                    request,
                    V5ClientRequest::AcknowledgeInvocationReceipt { .. }
                )
            })
            .count();
        assert_eq!(
            acknowledgements, 2,
            "every delivered terminal is acknowledged"
        );
    }

    #[test]
    fn malformed_direct_receipt_is_recovered_by_key_and_only_the_strict_one_is_acknowledged() {
        let fake = FakeDaemon::start(Box::new(|raw, request| match request {
            V5ClientRequest::SubmitInvocation { .. } => {
                // A receipt whose digest does not match its terminal is not a
                // strict frame: the client poisons the session and recovers.
                let forged: crate::application::receipt_ledger::TerminalDigest =
                    "0a".repeat(32).parse().unwrap();
                Step::Reply(V5ServerResponse::Invocation {
                    outcome: V5InvocationResponse::Direct {
                        receipt: V5PendingDirectReceipt::new(
                            daemon_side_key(raw),
                            completed("forged"),
                            forged,
                            epoch_ms_now(),
                        ),
                    },
                })
            }
            V5ClientRequest::RecoverInvocationReceipt { receipt_key } => {
                Step::Reply(direct_receipt(receipt_key.clone(), completed("strict")))
            }
            V5ClientRequest::AcknowledgeInvocationReceipt { .. } => Step::Close,
            other => panic!("unexpected frame {other:?}"),
        }));
        let router = canonical_daemon_router(fake.owner(), "/workspace".to_string());

        let result = direct_result(call(&router, None));

        assert_eq!(
            result
                .structured_content
                .as_ref()
                .and_then(|value| value["summary"].as_str()),
            Some("strict")
        );
        let seen = fake.seen();
        assert_eq!(fake.submissions(), 1);
        let acknowledged = seen
            .iter()
            .filter_map(|request| match request {
                V5ClientRequest::AcknowledgeInvocationReceipt {
                    terminal_digest, ..
                } => Some(terminal_digest.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            acknowledged,
            vec![canonical_v5_terminal(&completed("strict"))
                .unwrap()
                .digest()
                .clone()],
            "only the strict receipt is acknowledged"
        );
    }

    #[test]
    fn lost_submit_response_is_recovered_by_the_exact_key_without_a_second_submission() {
        let fake = FakeDaemon::start(Box::new(|raw, request| match request {
            V5ClientRequest::SubmitInvocation { .. } => Step::Close,
            V5ClientRequest::RecoverInvocationReceipt { receipt_key } => {
                Step::Reply(direct_receipt(receipt_key.clone(), completed("recovered")))
            }
            V5ClientRequest::AcknowledgeInvocationReceipt { .. } => Step::Close,
            other => panic!("unexpected frame {raw:?} {other:?}"),
        }));
        let router = canonical_daemon_router(fake.owner(), "/workspace".to_string());

        let result = direct_result(call(&router, None));

        assert_eq!(
            result
                .structured_content
                .as_ref()
                .and_then(|value| value["summary"].as_str()),
            Some("recovered")
        );
        let seen = fake.seen();
        assert_eq!(
            fake.submissions(),
            1,
            "a lost response never resubmits: {seen:?}"
        );
        let V5ClientRequest::SubmitInvocation { invocation } = &seen[0] else {
            panic!("first frame must be the submission");
        };
        let V5ClientRequest::RecoverInvocationReceipt { receipt_key } = &seen[1] else {
            panic!("second frame must recover by key: {seen:?}");
        };
        assert_eq!(receipt_key.invocation_id(), invocation.invocation_id());
        assert_eq!(
            receipt_key.reserved_task_id(),
            invocation.reserved_task_id()
        );
        assert_eq!(receipt_key.tool(), V5ToolIdentity::View);
        assert!(matches!(
            seen[2],
            V5ClientRequest::AcknowledgeInvocationReceipt { .. }
        ));
        assert_eq!(
            fake.sessions.load(Ordering::SeqCst),
            3,
            "anchor, submit peer, recovery peer"
        );
    }

    #[test]
    fn pending_receipt_is_polled_until_its_original_budget_settles() {
        let recoveries = Arc::new(AtomicUsize::new(0));
        let script_recoveries = Arc::clone(&recoveries);
        let fake = FakeDaemon::start(Box::new(move |_, request| match request {
            V5ClientRequest::SubmitInvocation { .. } => Step::Close,
            V5ClientRequest::RecoverInvocationReceipt { receipt_key } => {
                if script_recoveries.fetch_add(1, Ordering::SeqCst) == 0 {
                    Step::Reply(V5ServerResponse::Invocation {
                        outcome: V5InvocationResponse::ReceiptPending {
                            receipt_key: receipt_key.clone(),
                            phase: crate::infrastructure::daemon::protocol_v5::V5InvocationPhase::ReservedBegun,
                            accepted_epoch_ms: epoch_ms_now(),
                            original_budget_ms: 100,
                            cancel_requested: false,
                        },
                    })
                } else {
                    Step::Reply(direct_receipt(receipt_key.clone(), completed("settled")))
                }
            }
            V5ClientRequest::AcknowledgeInvocationReceipt { .. } => Step::Close,
            other => panic!("unexpected frame {other:?}"),
        }));
        let router = canonical_daemon_router(fake.owner(), "/workspace".to_string());

        let started = Instant::now();
        let result = direct_result(call(&router, None));

        assert_eq!(
            result
                .structured_content
                .as_ref()
                .and_then(|value| value["summary"].as_str()),
            Some("settled")
        );
        assert!(
            started.elapsed() >= Duration::from_millis(100),
            "recovery waited for the budget to settle"
        );
        assert_eq!(recoveries.load(Ordering::SeqCst), 2);
        assert_eq!(fake.submissions(), 1);
    }

    #[test]
    fn pending_receipt_beyond_the_frontend_cutoff_is_a_closed_refusal_not_a_retry() {
        let fake = FakeDaemon::start(Box::new(move |_, request| {
            match request {
            V5ClientRequest::SubmitInvocation { .. } => Step::Close,
            V5ClientRequest::RecoverInvocationReceipt { receipt_key } => {
                Step::Reply(V5ServerResponse::Invocation {
                    outcome: V5InvocationResponse::ReceiptPending {
                        receipt_key: receipt_key.clone(),
                        phase: crate::infrastructure::daemon::protocol_v5::V5InvocationPhase::ReservedActorBound,
                        accepted_epoch_ms: epoch_ms_now(),
                        original_budget_ms: 7_000,
                        cancel_requested: false,
                    },
                })
            }
            other => panic!("unexpected frame {other:?}"),
        }
        }));
        let router = canonical_daemon_router(fake.owner(), "/workspace".to_string());

        let refusal = call(&router, Some(Duration::from_millis(400)))
            .expect_err("a receipt pending past the cutoff is refused");

        assert_eq!(refusal.data, Some(json!({"code": "receipt_pending"})));
        assert_eq!(fake.submissions(), 1);
    }

    #[test]
    fn recovery_without_a_receipt_is_a_transport_refusal_and_no_resubmission() {
        let fake = FakeDaemon::start(Box::new(|_, request| match request {
            V5ClientRequest::SubmitInvocation { .. } => Step::Close,
            V5ClientRequest::RecoverInvocationReceipt { .. } => {
                Step::Reply(V5ServerResponse::Error {
                    code: V5DaemonErrorCode::ReceiptNotFound,
                })
            }
            other => panic!("unexpected frame {other:?}"),
        }));
        let router = canonical_daemon_router(fake.owner(), "/workspace".to_string());

        let refusal = call(&router, None).expect_err("lost submission is refused");

        assert_eq!(refusal.code, ErrorCode(TOOL_EXECUTION_ERROR));
        assert_eq!(
            refusal.message,
            "daemon deadline expired during invocation submit response"
        );
        assert_eq!(fake.submissions(), 1);
    }

    #[test]
    fn response_lost_at_the_cutoff_is_recovered_into_the_handed_off_task() {
        let task_id = TaskId::new();
        let fake = FakeDaemon::start(Box::new(move |_, request| match request {
            // The daemon is still working when the frontend cutoff passes: the
            // submit session sees nothing before the client gives up on it.
            V5ClientRequest::SubmitInvocation { .. } => {
                thread::sleep(Duration::from_millis(600));
                Step::Close
            }
            V5ClientRequest::RecoverInvocationReceipt { receipt_key } => {
                Step::Reply(V5ServerResponse::Invocation {
                    outcome: V5InvocationResponse::Task {
                        snapshot: V5DaemonTaskSnapshot::Working {
                            task_id,
                            invocation_id: receipt_key.invocation_id(),
                            receipt_key_digest: "07".repeat(32).parse().unwrap(),
                            created_at_epoch_ms: epoch_ms_now(),
                            updated_at_epoch_ms: epoch_ms_now(),
                            ttl_ms: 3_600_000,
                            poll_interval_ms: 250,
                            version: 2,
                            cancel_requested: false,
                        },
                    },
                })
            }
            other => panic!("unexpected frame {other:?}"),
        }));
        let router = canonical_daemon_router(fake.owner(), "/workspace".to_string());

        let started = Instant::now();
        let outcome = call(&router, Some(Duration::from_millis(300)));

        let Ok(CanonicalCallOutcome::Task(snapshot)) = outcome else {
            panic!("a lost response must recover the handed-off Task: {outcome:?}");
        };
        assert_eq!(snapshot.task_id(), task_id);
        assert!(
            started.elapsed()
                < Duration::from_millis(300) + RECOVERY_BUDGET + Duration::from_millis(500),
            "recovery stays inside its bounded window: {:?}",
            started.elapsed()
        );
        assert_eq!(fake.submissions(), 1, "recovery never resubmits");
    }

    #[test]
    fn recovery_window_closing_answers_the_closed_lost_submit_refusal() {
        let fake = FakeDaemon::start(Box::new(move |_, request| match request {
            V5ClientRequest::SubmitInvocation { .. } => {
                thread::sleep(Duration::from_millis(500));
                Step::Close
            }
            // Recovery answers only after the recovery window is spent.
            V5ClientRequest::RecoverInvocationReceipt { .. } => {
                thread::sleep(RECOVERY_BUDGET + Duration::from_millis(300));
                Step::Close
            }
            other => panic!("unexpected frame {other:?}"),
        }));
        let router = canonical_daemon_router(fake.owner(), "/workspace".to_string());

        let refusal = call(&router, Some(Duration::from_millis(300)))
            .expect_err("a recovery that does not settle is refused");

        assert_eq!(refusal.code, ErrorCode(TOOL_EXECUTION_ERROR));
        assert_eq!(
            refusal.message,
            "daemon deadline expired during invocation submit response"
        );
        assert_eq!(fake.submissions(), 1);
    }

    #[test]
    fn daemon_rejection_and_foreign_receipt_are_distinct_closed_refusals() {
        let fake = FakeDaemon::start(Box::new(|raw, request| match request {
            V5ClientRequest::SubmitInvocation { invocation }
                if invocation.arguments().contains_key("at") =>
            {
                Step::Reply(V5ServerResponse::Error {
                    code: V5DaemonErrorCode::Overloaded,
                })
            }
            V5ClientRequest::SubmitInvocation { .. } => {
                let mut foreign = daemon_side_key(raw);
                foreign = ReceiptKey::new(
                    InvocationId::new(),
                    foreign.reserved_task_id(),
                    foreign.request_identity(),
                );
                Step::Reply(direct_receipt(foreign, completed("foreign")))
            }
            other => panic!("unexpected frame {other:?}"),
        }));
        let router = canonical_daemon_router(fake.owner(), "/workspace".to_string());

        let rejected = call(&router, None).expect_err("overloaded daemon refuses");
        let foreign = (router.call)(
            V5ToolIdentity::Docs,
            &Map::new(),
            deadline(None),
            CancellationToken::new(),
        )
        .expect_err("a foreign receipt is refused");

        assert_eq!(rejected.code, ErrorCode(TOOL_EXECUTION_ERROR));
        assert_eq!(
            rejected.message,
            "daemon invocation submission rejected: overloaded"
        );
        assert_eq!(
            foreign.data,
            Some(json!({"code": "daemon_protocol_failed"}))
        );
        thread::sleep(Duration::from_millis(50));
        assert!(
            !fake.seen().iter().any(|request| matches!(
                request,
                V5ClientRequest::AcknowledgeInvocationReceipt { .. }
            )),
            "neither refusal acknowledges anything"
        );
    }

    #[test]
    fn task_operations_use_peer_sessions_bounded_by_the_frontend_cutoff() {
        let task_id = TaskId::new();
        let fake = FakeDaemon::start(Box::new(move |_, request| match request {
            V5ClientRequest::GetTask { task_id: asked } => {
                assert_eq!(*asked, task_id);
                Step::Reply(V5ServerResponse::Error {
                    code: V5DaemonErrorCode::TaskExpired,
                })
            }
            V5ClientRequest::WaitTask { wait_ms, .. } => {
                assert!(
                    *wait_ms <= 300,
                    "wait shrank by the transport budget: {wait_ms}"
                );
                Step::Reply(V5ServerResponse::Pong)
            }
            V5ClientRequest::CancelTask { .. } => Step::Close,
            other => panic!("unexpected frame {other:?}"),
        }));
        let router = canonical_daemon_router(fake.owner(), "/workspace".to_string());

        let expired = (router.get)(task_id, deadline(None));
        let unexpected = (router.wait)(task_id, 7_000, deadline(Some(Duration::from_millis(400))));
        let closed = (router.cancel)(task_id, deadline(None));
        let past = (router.get)(
            task_id,
            FrontendInvocationDeadline::new(Instant::now() - Duration::from_secs(8), None),
        );

        assert_eq!(
            expired,
            Err(V5TaskExchangeError::Protocol(
                V5DaemonErrorCode::TaskExpired
            ))
        );
        assert_eq!(unexpected, Err(V5TaskExchangeError::UnexpectedResponse));
        assert_eq!(closed, Err(V5TaskExchangeError::Transport));
        assert_eq!(past, Err(V5TaskExchangeError::Transport));
    }

    #[test]
    fn live_daemon_executes_once_and_compacts_the_acknowledged_receipt_to_a_tombstone() {
        let service = Arc::new(ScriptedService {
            delay: Duration::ZERO,
            known_long: false,
            outcome: Mutex::new(None),
            executions: AtomicUsize::new(0),
        });
        let daemon = LiveDaemon::start(service.clone());
        let observed = Arc::new(Mutex::new(None));
        let sink = Arc::clone(&observed);
        let owner = daemon.owner();
        let router = canonical_daemon_router_observed(
            owner,
            daemon.workspace_hint.clone(),
            Arc::new(move |key: &ReceiptKey| *sink.lock().unwrap() = Some(key.clone())),
        );

        let result = direct_result(call(&router, None));
        let key = observed
            .lock()
            .unwrap()
            .clone()
            .expect("receipt key observed");
        let mut peer = daemon.owner();
        let recovered = peer
            .recover_invocation_receipt_before(key, Instant::now() + Duration::from_secs(5))
            .expect("recover after acknowledgement");

        assert_eq!(
            result
                .structured_content
                .as_ref()
                .and_then(|value| value["summary"].as_str()),
            Some("executed")
        );
        assert_eq!(service.executions.load(Ordering::SeqCst), 1);
        assert!(
            matches!(
                recovered,
                V5ServerResponse::Invocation {
                    outcome: V5InvocationResponse::Acknowledged { .. }
                }
            ),
            "acknowledged receipt must compact to a tombstone: {recovered:?}"
        );
        drop(peer);
        drop(router);
        daemon.finish();
    }

    #[test]
    fn live_daemon_failure_is_delivered_as_the_closed_reason_without_its_text() {
        let service = Arc::new(ScriptedService {
            delay: Duration::ZERO,
            known_long: false,
            outcome: Mutex::new(Some(Err(
                crate::domain::invocation::InvocationFailure::new(
                    "provider_exploded",
                    "/private/workspace bearer-secret",
                ),
            ))),
            executions: AtomicUsize::new(0),
        });
        let daemon = LiveDaemon::start(service.clone());
        let router = canonical_daemon_router(daemon.owner(), daemon.workspace_hint.clone());

        let refusal = call(&router, None).expect_err("failed execution is an error");

        assert_eq!(refusal.code, ErrorCode::INTERNAL_ERROR);
        assert_eq!(refusal.message, "daemon invocation failed");
        assert_eq!(refusal.data, Some(json!({"code": "invocation_failed"})));
        assert!(!format!("{refusal:?}").contains("bearer-secret"));
        drop(router);
        daemon.finish();
    }

    #[test]
    fn live_daemon_hands_slow_work_off_to_a_task_that_get_wait_and_cancel_observe() {
        // Known-long work is handed off before execution, so the Task path is
        // exercised without racing the seven-second cutoff on a loaded runner.
        let service = Arc::new(ScriptedService {
            delay: Duration::from_millis(300),
            known_long: true,
            outcome: Mutex::new(None),
            executions: AtomicUsize::new(0),
        });
        let daemon = LiveDaemon::start(service.clone());
        let router = canonical_daemon_router(daemon.owner(), daemon.workspace_hint.clone());

        let handoff = match call(&router, None) {
            Ok(CanonicalCallOutcome::Task(snapshot)) => snapshot,
            Ok(CanonicalCallOutcome::Direct(result)) => {
                panic!("slow work answered directly: {result:?}")
            }
            Err(error) => panic!("slow work refused: {error}"),
        };
        let task_id = handoff.task_id();
        let observed = (router.get)(task_id, deadline(None)).expect("get the handed-off task");
        let terminal =
            (router.wait)(task_id, 5_000, deadline(None)).expect("wait for the terminal");
        let cancelled = (router.cancel)(task_id, deadline(None)).expect("cancel after terminal");

        assert!(!matches!(
            handoff.status(),
            crate::domain::invocation::InvocationStatus::Completed
        ));
        assert_eq!(observed.task_id(), task_id);
        assert_eq!(
            terminal
                .completed_result()
                .map(|result| result.summary.as_str()),
            Some("executed")
        );
        assert_eq!(
            cancelled.status(),
            crate::domain::invocation::InvocationStatus::Completed
        );
        assert_eq!(service.executions.load(Ordering::SeqCst), 1);
        drop(router);
        daemon.finish();
    }
}
