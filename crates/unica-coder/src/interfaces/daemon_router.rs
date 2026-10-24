//! Canonical v0.13 router over the protocol-v5 user daemon.
//!
//! The stdio frontend renews its daemon lease after replacement and gives every
//! invocation and every Task operation a peer session of its own. A Direct
//! terminal is acknowledged only after its host-facing value exists; a lost
//! submit response is recovered by the exact receipt key the frontend derives
//! itself, never by a second submission.
use super::canonical_cancellation::{CanonicalCancellation, ControlFailure};
use super::task_projection::{self, DirectProjection};
use rmcp::model::{CallToolResult, ErrorCode, ErrorData};
use serde_json::{Map, Value};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use unica_daemon::client_v5::{
    V5DaemonClient, V5DaemonProcessOwner, V5TaskExchangeError, V5TransportError,
};
use unica_daemon::protocol_v5::{V5DaemonErrorCode, V5InvocationRequest, V5ServerResponse};
use unica_execution::invocation::{
    handoff_budget, normalized_arguments_hash, INVOCATION_HANDOFF_WINDOW,
    RESPONSE_SERIALIZATION_MARGIN, RESPONSE_SERIALIZATION_MARGIN_MS,
};
use unica_execution::receipt_ledger::{
    request_scope_hash, ReceiptKey, RequestIdentity, V5ToolIdentity,
    MAX_ORIGINAL_RESPONSE_BUDGET_MS,
};
use unica_execution::receipt_runtime::response::{
    V5DaemonTaskSnapshot, V5InvocationPhase, V5InvocationResponse,
};
use unica_execution_contract::invocation::{InvocationId, TaskId};
use unica_refusal::RefusalCode;
#[cfg(test)]
use unica_work_control::cancellation::CancellationToken;

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
    /// The submit answer was lost and could not be recovered: the call may
    /// continue as the Task reserved for it. A client with native Tasks gets
    /// that Task to poll; any other client gets the refusal naming it.
    LostSubmit(Box<LostSubmit>),
}

#[derive(Debug)]
pub(super) struct LostSubmit {
    pub(super) task: V5DaemonTaskSnapshot,
    pub(super) refusal: ErrorData,
}

pub(super) type CanonicalCallHandler = dyn Fn(
        V5ToolIdentity,
        &Map<String, Value>,
        &unica_bootstrap::HostRequest,
        FrontendInvocationDeadline,
        CanonicalCancellation,
    ) -> Result<CanonicalCallOutcome, ErrorData>
    + Send
    + Sync;

pub(super) type CanonicalTaskHandler = dyn Fn(TaskId, FrontendInvocationDeadline) -> Result<V5DaemonTaskSnapshot, V5TaskExchangeError>
    + Send
    + Sync;

pub(super) type CanonicalTaskWaitHandler = dyn Fn(
        TaskId,
        u64,
        FrontendInvocationDeadline,
        CanonicalCancellation,
    ) -> Result<V5DaemonTaskSnapshot, V5TaskExchangeError>
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

/// Build the router over a renewable protocol-v5 owner lease.
pub(super) fn canonical_daemon_router(
    client: impl Into<V5DaemonClient>,
    workspace: impl Into<unica_bootstrap::HostWorkspaceContext>,
) -> CanonicalDaemonRouter {
    build_router(client.into(), workspace.into(), None)
}

#[cfg(test)]
pub(super) fn canonical_daemon_router_observed(
    owner: V5DaemonProcessOwner,
    workspace_hint: String,
    observer: ReceiptObserver,
) -> CanonicalDaemonRouter {
    build_router(owner.into(), workspace_hint.into(), Some(observer))
}

fn build_router(
    client: V5DaemonClient,
    workspace: unica_bootstrap::HostWorkspaceContext,
    observer: Option<ReceiptObserver>,
) -> CanonicalDaemonRouter {
    // The client retains the current owner lease for the frontend lifetime; every
    // invocation runs on its own peer session so a slow direct response cannot
    // serialize another call behind it.
    let client = Arc::new(client);
    let call_client = Arc::clone(&client);
    let call: Arc<CanonicalCallHandler> =
        Arc::new(move |tool, arguments, host, deadline, cancellation| {
            // Resolve once per invocation. Never publish this choice back into
            // shared frontend state: another call may belong to another tree.
            let resolved = match workspace.resolve_request(host) {
                Ok(resolved) => resolved,
                Err(message) => {
                    let refusal =
                        unica_execution_contract::invocation::DomainResult::canonical_rejection(
                            None,
                            unica_refusal::RefusalCode::InvalidState,
                            message,
                        );
                    return task_projection::call_tool_result(&refusal, host.response_budget)
                        .map(CanonicalCallOutcome::Direct)
                        .map_err(task_projection::projection_error);
                }
            };
            submit_and_settle(
                &call_client,
                resolved,
                tool,
                arguments,
                deadline,
                observer.as_ref(),
                cancellation,
                host.response_budget,
            )
        });
    let get_client = Arc::clone(&client);
    let get: Arc<CanonicalTaskHandler> = Arc::new(move |task_id, deadline| {
        let cutoff = deadline.transport_cutoff();
        let mut peer = task_peer(&get_client, cutoff)?;
        peer.get_task_before(task_id, cutoff)
    });
    let wait_client = Arc::clone(&client);
    let wait: Arc<CanonicalTaskWaitHandler> =
        Arc::new(move |task_id, wait_ms, deadline, cancellation| {
            let cutoff = wait_transport_cutoff(wait_ms, deadline);
            let mut peer = task_peer(&wait_client, cutoff)?;
            // The daemon wait shrinks by what connect and handshake already spent
            // and by the response margin; the cutoff itself never moves.
            let interrupt = peer
                .observation_interrupt_handle()
                .map_err(|_| V5TaskExchangeError::Transport)?;
            cancellation
                .bind_control(Arc::new(move || {
                    interrupt
                        .interrupt()
                        .map_err(|error| ControlFailure::BeforeSend(error.to_string()))
                }))
                .map_err(|_| V5TaskExchangeError::Transport)?;
            let bounded_wait_ms = bounded_wait_ms(wait_ms, cutoff, Instant::now());
            peer.wait_task_before(task_id, bounded_wait_ms, cutoff)
        });
    let cancel: Arc<CanonicalTaskHandler> = Arc::new(move |task_id, deadline| {
        let cutoff = deadline.transport_cutoff();
        let mut peer = task_peer(&client, cutoff)?;
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
    client: &V5DaemonClient,
    cutoff: Instant,
) -> Result<V5DaemonProcessOwner, V5TaskExchangeError> {
    if Instant::now() >= cutoff {
        return Err(V5TaskExchangeError::Transport);
    }
    client
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

#[allow(clippy::too_many_arguments)]
fn submit_and_settle(
    client: &V5DaemonClient,
    workspace: unica_bootstrap::ResolvedWorkspace,
    tool: V5ToolIdentity,
    arguments: &Map<String, Value>,
    deadline: FrontendInvocationDeadline,
    observer: Option<&ReceiptObserver>,
    cancellation: CanonicalCancellation,
    budget: unica_bootstrap::ResponseBudget,
) -> Result<CanonicalCallOutcome, ErrorData> {
    let cutoff = deadline.transport_cutoff();
    // No frame of this call has left the frontend yet: whatever the
    // connection failure, the daemon holds no receipt and will never start it.
    let mut peer = client
        .connect_peer_before(cutoff)
        .map_err(|error| unconnected_refusal(&error, cutoff))?;
    // The daemon anchors its handoff at the receipt of the frame, so its budget
    // is what remains of the frontend budget once the connection stands: a slow
    // connect or a spawn shortens the daemon's window instead of pushing its
    // handoff past the frontend cutoff.
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
        workspace,
        response_budget_ms,
    )
    .map_err(|message| ErrorData::invalid_params(message, None))?;
    let receipt_key = receipt_key_for(&invocation, client)?;
    let endpoint = peer.control_endpoint();
    let control_key = receipt_key.clone();
    cancellation
        .bind_control(Arc::new(move || {
            cancel_exact_endpoint(endpoint.clone(), control_key.clone())
        }))
        .map_err(|error| {
            ErrorData::new(
                rmcp::model::ErrorCode(TOOL_EXECUTION_ERROR),
                error.to_string(),
                None,
            )
        })?;
    if let Some(observer) = observer {
        observer(&receipt_key);
    }
    let (peer, response) = match peer.submit_invocation_before(invocation, cutoff) {
        Ok(response) => (peer, response),
        // The frame reached the daemon and the answer did not come back: the
        // daemon may already hold a reserved or terminal receipt for it.
        Err(V5TransportError::ResponseLost(_)) => {
            let recovery_cutoff = cutoff.max(Instant::now()) + RECOVERY_BUDGET;
            match recover_receipt(client, &receipt_key, recovery_cutoff) {
                Ok(recovered) => recovered,
                Err(refusal) => return lost_submit_outcome(refusal, &receipt_key),
            }
        }
        // The frame exceeds the daemon bound and was never written: the
        // daemon holds no receipt, and the same call will never fit.
        Err(V5TransportError::RequestTooLarge {
            frame_bytes,
            limit_bytes,
        }) => return request_too_large_refusal(frame_bytes, limit_bytes),
        // Writing never started: the frame did not reach the daemon.
        Err(error) => return Err(not_sent_refusal(&error)),
    };
    settle(peer, response, &receipt_key, budget)
        .map_err(|error| read_only_uncertain_outcome(error, tool, arguments))
}

/// `outcome_uncertain` значит: демон пропал, пока шёл вызов, и исход не
/// сохранён. Повтора нет (INV.APP.DAEMON-INVOCATION-OWNERSHIP), но вызов,
/// который только читает, ничего не менял: отказ говорит это и предлагает
/// тот же вызов новым вызовом, чтобы агенту не гадать (#1804).
fn read_only_uncertain_outcome(
    error: ErrorData,
    tool: V5ToolIdentity,
    arguments: &Map<String, Value>,
) -> ErrorData {
    let read_only = matches!(
        tool,
        V5ToolIdentity::View
            | V5ToolIdentity::Resolve
            | V5ToolIdentity::Search
            | V5ToolIdentity::Check
            | V5ToolIdentity::Diff
            | V5ToolIdentity::Docs
    );
    let uncertain = error
        .data
        .as_ref()
        .and_then(|data| data.get("code"))
        .is_some_and(|code| code == "outcome_uncertain");
    if !read_only || !uncertain {
        return error;
    }
    let tool = tool.wire_name();
    ErrorData::new(
        error.code,
        format!(
            "{}; `{tool}` only reads, so the call changed nothing: repeat it",
            error.message
        ),
        Some(serde_json::json!({
            "code": "outcome_uncertain",
            "next": [{"tool": tool, "args": arguments}],
        })),
    )
}

fn cancel_exact_endpoint(
    endpoint: unica_daemon::protocol_v5::V5EndpointRecord,
    key: ReceiptKey,
) -> Result<(), ControlFailure> {
    let mut uncertain = false;
    loop {
        match V5DaemonProcessOwner::cancel_at_endpoint(endpoint.clone(), key.clone()) {
            Ok(V5ServerResponse::Invocation { outcome }) => {
                let confirmed = match outcome {
                    V5InvocationResponse::ReceiptPending {
                        receipt_key,
                        cancel_requested,
                        ..
                    } => receipt_key == key && cancel_requested,
                    V5InvocationResponse::Direct { receipt } => receipt.receipt_key() == &key,
                    V5InvocationResponse::Acknowledged { acknowledgement } => {
                        acknowledgement.receipt_key() == &key
                    }
                    V5InvocationResponse::Task { snapshot } => {
                        snapshot.task_id() == key.reserved_task_id()
                            && snapshot.invocation_id() == key.invocation_id()
                            && snapshot.receipt_key_digest()
                                == &unica_execution::receipt_ledger::receipt_key_digest(&key)
                            && (snapshot.cancel_requested()
                                || matches!(
                                    snapshot,
                                    V5DaemonTaskSnapshot::Completed { .. }
                                        | V5DaemonTaskSnapshot::Failed { .. }
                                        | V5DaemonTaskSnapshot::Cancelled { .. }
                                ))
                    }
                };
                return if confirmed {
                    Ok(())
                } else {
                    Err(ControlFailure::Uncertain(
                        "daemon control response did not confirm the original exact key".to_owned(),
                    ))
                };
            }
            Err(V5TransportError::ResponseLost(_)) => {
                // Cancel may already be committed. Its idempotent repeat keeps
                // the same complete key and never replays Submit or effects.
                uncertain = true;
                std::thread::sleep(RECOVERY_POLL_INTERVAL);
            }
            Err(V5TransportError::RequestNotSent(message)) if !uncertain => {
                return Err(ControlFailure::BeforeSend(message))
            }
            Err(error) => return Err(ControlFailure::Uncertain(error.to_string())),
            Ok(response) => {
                return Err(ControlFailure::Uncertain(format!(
                    "daemon rejected exact cancellation: {response:?}"
                )))
            }
        }
    }
}

/// The exact key the daemon derives from the same submission
/// (`receipt_key_is_canonicalized_identically_by_client_and_server`).
fn receipt_key_for(
    invocation: &V5InvocationRequest,
    client: &V5DaemonClient,
) -> Result<ReceiptKey, ErrorData> {
    let scope = request_scope_hash(invocation.workspace_hint()).map_err(|_| {
        ErrorData::invalid_params("workspace hint is not a valid request scope", None)
    })?;
    Ok(ReceiptKey::new(
        invocation.invocation_id(),
        invocation.reserved_task_id(),
        RequestIdentity::new(
            client.core_identity().digest().clone(),
            invocation.tool(),
            normalized_arguments_hash(invocation.arguments()),
            scope,
        ),
    ))
}

/// Read the durable state of a submission whose response was lost. A receipt
/// still inside its original budget is polled until that budget settles;
/// nothing here opens a new invocation budget or submits again. When even the
/// recovery window closes, the host gets the closed lost-submit refusal that
/// names the reserved Task: the daemon may hold the work, nobody replays it.
fn recover_receipt(
    client: &V5DaemonClient,
    receipt_key: &ReceiptKey,
    cutoff: Instant,
) -> Result<(V5DaemonProcessOwner, V5ServerResponse), ErrorData> {
    loop {
        let mut peer = client
            .connect_peer_before(cutoff)
            .map_err(|_| lost_submit_refusal(receipt_key))?;
        let response = peer
            .recover_invocation_receipt_before(receipt_key.clone(), cutoff)
            .map_err(|_| lost_submit_refusal(receipt_key))?;
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
                // A receipt that settles after the recovery window cannot be
                // delivered by this call: the daemon keeps it and promotes it
                // on its own, nobody resubmits, and the host gets the same
                // closed refusal as for a lost submission.
                if until_settled >= remaining || remaining <= RECOVERY_POLL_INTERVAL {
                    return Err(lost_submit_refusal(receipt_key));
                }
                std::thread::sleep(until_settled.max(RECOVERY_POLL_INTERVAL));
            }
            // The daemon holds no receipt yet. The frame was written, so the
            // daemon may still read and reserve it after this answer; only a
            // cancel reserved for the exact key proves it never runs.
            V5ServerResponse::Error {
                code: V5DaemonErrorCode::ReceiptNotFound,
            } => return Err(fence_unreceived_submit(client, receipt_key, cutoff)),
            other => return Ok((peer, other)),
        }
    }
}

/// Fence the exact key of a submission the daemon has not seen. A frame
/// still on its way can only be read by the daemon process it was sent to;
/// once that process holds the fence, the frame never runs there, so the host
/// may repeat the call. Any other answer means the frame got there first:
/// the call may continue as its Task.
fn fence_unreceived_submit(
    client: &V5DaemonClient,
    receipt_key: &ReceiptKey,
    cutoff: Instant,
) -> ErrorData {
    let fenced = client
        .connect_peer_before(cutoff)
        .and_then(|mut peer| peer.fence_invocation_receipt_before(receipt_key.clone(), cutoff));
    match fenced {
        Ok(V5ServerResponse::Invocation {
            outcome:
                V5InvocationResponse::ReceiptPending {
                    receipt_key: fenced_key,
                    phase: V5InvocationPhase::CancelReserved,
                    cancel_requested: true,
                    ..
                },
        }) if &fenced_key == receipt_key => not_delivered_refusal(
            "daemon did not receive the invocation submit; it will not run".to_owned(),
        ),
        _ => lost_submit_refusal(receipt_key),
    }
}

fn settle(
    mut peer: V5DaemonProcessOwner,
    response: V5ServerResponse,
    receipt_key: &ReceiptKey,
    budget: unica_bootstrap::ResponseBudget,
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
            let projection = task_projection::project_direct_terminal(receipt.terminal(), budget)
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
/// could not be recovered in time. The frame reached the daemon, so the call
/// may already be — or still become — the Task reserved for it: the refusal
/// names that Task and points at the wait instead of a second submission.
/// The Task identifier is the frontend's own opaque UUID; no path, argument
/// or daemon text travels in the data.
fn lost_submit_refusal(receipt_key: &ReceiptKey) -> ErrorData {
    let code = RefusalCode::SubmitResponseLost;
    let task_id = receipt_key.reserved_task_id().to_string();
    // Hosts often show the model only the message: it names the Task too.
    ErrorData::new(
        ErrorCode(TOOL_EXECUTION_ERROR),
        format!(
            "daemon deadline expired during invocation submit response; \
             the call may continue as Task {task_id}: wait for that Task \
             instead of repeating the call"
        ),
        Some(serde_json::json!({
            "code": code.as_str(),
            "outcome": code.outcome().as_str(),
            "taskId": task_id,
            "next": [{
                "tool": "unica.task.result",
                "args": {"taskId": task_id},
            }],
        })),
    )
}

/// Poll interval the queued Task of a lost submit announces; the daemon's
/// own receipt-backed snapshots use the same 100 ms.
const LOST_SUBMIT_POLL_INTERVAL_MS: u64 = 100;

/// A lost-submit refusal becomes the outcome that carries both answers: the
/// queued Task under the reserved id, which the daemon answers by that id
/// whether the call is still inline, a Task, or ended directly, and the
/// refusal for clients without native Tasks.
fn lost_submit_outcome(
    refusal: ErrorData,
    receipt_key: &ReceiptKey,
) -> Result<CanonicalCallOutcome, ErrorData> {
    let lost = refusal
        .data
        .as_ref()
        .and_then(|data| data.get("code"))
        .and_then(Value::as_str)
        == Some(RefusalCode::SubmitResponseLost.as_str());
    if !lost {
        return Err(refusal);
    }
    let now = epoch_ms_now();
    Ok(CanonicalCallOutcome::LostSubmit(Box::new(LostSubmit {
        task: V5DaemonTaskSnapshot::Queued {
            task_id: receipt_key.reserved_task_id(),
            invocation_id: receipt_key.invocation_id(),
            receipt_key_digest: unica_execution::receipt_ledger::receipt_key_digest(receipt_key),
            created_at_epoch_ms: now,
            updated_at_epoch_ms: now,
            ttl_ms: unica_execution::receipt_ledger::DIRECT_TERMINAL_RETENTION_MS,
            poll_interval_ms: LOST_SUBMIT_POLL_INTERVAL_MS,
            version: 1,
            cancel_requested: false,
        },
        refusal,
    })))
}

/// A connection that could not stand before the cutoff is transient: the
/// call never left the frontend and may be repeated as is. One refused
/// earlier — the daemon could not be started, rejected the handshake or
/// belongs to another core — stays refused on a repeat: a person has to look.
fn unconnected_refusal(error: &V5TransportError, cutoff: Instant) -> ErrorData {
    if Instant::now() >= cutoff {
        return not_sent_refusal(error);
    }
    let detail = unica_refusal::RefusalDetail::BackendIncompatible;
    ErrorData::new(
        ErrorCode(TOOL_EXECUTION_ERROR),
        format!("{error}; the call did not reach the daemon and will not run"),
        Some(serde_json::json!({
            "code": detail.code().as_str(),
            "outcome": detail.outcome().as_str(),
            "detailCode": detail.as_str(),
        })),
    )
}

/// The closed answer for a call whose frame never left the frontend: the
/// connection or the handshake failed, or writing could not start. The daemon
/// holds no receipt and will never start this call, so it may be repeated
/// as is.
fn not_sent_refusal(error: &V5TransportError) -> ErrorData {
    not_delivered_refusal(error.to_string())
}

fn not_delivered_refusal(message: String) -> ErrorData {
    let code = RefusalCode::SubmitNotDelivered;
    ErrorData::new(
        ErrorCode(TOOL_EXECUTION_ERROR),
        message,
        Some(serde_json::json!({
            "code": code.as_str(),
            "outcome": code.outcome().as_str(),
        })),
    )
}

/// The canonical refusal for a call too large for the daemon request frame,
/// delivered through `diagnostics` like any other refusal of a canonical call
/// (`INV.WIRE.V13-REFUSAL-CHANNEL`). Hosts often show the model only the
/// message: it names the resource, the unit and both sizes, and none of the
/// arguments. The call did not leave the frontend, so it is not running, and
/// repeating it unchanged meets the same bound.
fn request_too_large_refusal(
    frame_bytes: usize,
    limit_bytes: usize,
) -> Result<CanonicalCallOutcome, ErrorData> {
    let refusal = unica_execution_contract::invocation::DomainResult::canonical_rejection(
        None,
        RefusalCode::ProviderLimitExceeded,
        format!(
            "the call is {frame_bytes} bytes on the daemon wire, above its \
             {limit_bytes}-byte request limit; it was not sent and will not run: \
             reduce the arguments or split the change into several calls"
        ),
    );
    task_projection::call_tool_result(&refusal, unica_bootstrap::ResponseBudget::default())
        .map(CanonicalCallOutcome::Direct)
        .map_err(task_projection::projection_error)
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
    use crate::infrastructure::daemon::server::DaemonServerConfig;
    use std::io::{BufReader, Write};
    use std::net::{Ipv4Addr, TcpListener, TcpStream};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;
    use std::thread;
    use unica_daemon::identity::{CoreIdentity, DaemonStateDirectory};
    use unica_daemon::protocol_v5::{
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
                    thread::spawn(move || {
                        serve_session(stream, record, seen, script, handshake_delay)
                    });
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
        ) -> Result<unica_execution::receipt_runtime::ports::ExecutionClass, Box<DomainResult>>
        {
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
                unica_daemon::transport::run_daemon(config.into_transport_config())
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
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;
    use std::thread;
    use unica_daemon::protocol_v5::V5ClientRequest;
    use unica_execution::receipt_ledger::{canonical_v5_terminal, ReceiptTerminalOutcome};
    use unica_execution::receipt_runtime::response::V5PendingDirectReceipt;

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
            &unica_bootstrap::HostRequest::default(),
            deadline(host_remaining),
            CancellationToken::new().into(),
        )
    }

    fn direct_result(outcome: Result<CanonicalCallOutcome, ErrorData>) -> CallToolResult {
        match outcome {
            Ok(CanonicalCallOutcome::Direct(result)) => result,
            Ok(CanonicalCallOutcome::Task(snapshot)) => {
                panic!("unexpected Task handoff: {snapshot:?}")
            }
            Ok(CanonicalCallOutcome::LostSubmit(lost)) => {
                panic!("unexpected lost submit: {}", lost.refusal)
            }
            Err(error) => panic!("unexpected refusal: {error}"),
        }
    }

    /// A lost submit answers with the queued Task reserved for the call and
    /// the refusal naming that same Task.
    fn lost_refusal(outcome: Result<CanonicalCallOutcome, ErrorData>) -> ErrorData {
        let lost = match outcome {
            Ok(CanonicalCallOutcome::LostSubmit(lost)) => lost,
            Ok(CanonicalCallOutcome::Direct(result)) => panic!("unexpected direct: {result:?}"),
            Ok(CanonicalCallOutcome::Task(snapshot)) => panic!("unexpected Task: {snapshot:?}"),
            Err(error) => panic!("unexpected refusal: {error}"),
        };
        assert_eq!(
            lost.refusal.data.as_ref().unwrap()["taskId"],
            json!(lost.task.task_id().to_string())
        );
        assert_eq!(
            lost.task.status(),
            unica_execution_contract::invocation::InvocationStatus::Queued
        );
        lost.refusal
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
        let router = canonical_daemon_router(fake.owner(), fake.workspace_hint());

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
            canonical_v5_terminal(&completed("viewed"), unica_bootstrap::MAX_RESPONSE_BUDGET)
                .unwrap()
                .digest()
        );
        assert!(
            (6_900..=7_000).contains(&invocation.response_budget_ms()),
            "the daemon budget is the remaining handoff window: {}",
            invocation.response_budget_ms()
        );
        assert_eq!(invocation.workspace_hint(), fake.workspace_hint());
    }

    #[test]
    fn failed_and_cancelled_direct_terminals_answer_closed_errors_after_acknowledgement() {
        use unica_execution::invocation_store_v5::V5SafeFailureReason;

        let terminals = Arc::new(Mutex::new(vec![
            ReceiptTerminalOutcome::Cancelled,
            ReceiptTerminalOutcome::Failed {
                reason: V5SafeFailureReason::OutcomeUncertain,
            },
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
        let router = canonical_daemon_router(fake.owner(), fake.workspace_hint());

        let uncertain = call(&router, None).expect_err("failed terminal is an error");
        let mutation = (router.call)(
            V5ToolIdentity::Apply,
            &arguments(),
            &unica_bootstrap::HostRequest::default(),
            deadline(None),
            CancellationToken::new().into(),
        )
        .expect_err("failed terminal is an error");
        let cancelled = call(&router, None).expect_err("cancelled terminal is an error");

        // A read-only call changed nothing: the refusal says so and offers
        // the same call as a new one (#1804).
        assert_eq!(uncertain.code, ErrorCode::INTERNAL_ERROR);
        assert_eq!(
            uncertain.message,
            "daemon invocation outcome is uncertain; `unica.view` only reads, so the call changed nothing: repeat it"
        );
        assert_eq!(
            uncertain.data,
            Some(json!({
                "code": "outcome_uncertain",
                "next": [{"tool": "unica.view", "args": arguments()}],
            }))
        );
        // A mutation may have taken effect: the closed refusal stays as it is.
        assert_eq!(mutation.message, "daemon invocation outcome is uncertain");
        assert_eq!(mutation.data, Some(json!({"code": "outcome_uncertain"})));
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
            acknowledgements, 3,
            "every delivered terminal is acknowledged"
        );
    }

    #[test]
    fn malformed_direct_receipt_is_recovered_by_key_and_only_the_strict_one_is_acknowledged() {
        let fake = FakeDaemon::start(Box::new(|raw, request| match request {
            V5ClientRequest::SubmitInvocation { .. } => {
                // A receipt whose digest does not match its terminal is not a
                // strict frame: the client poisons the session and recovers.
                let forged: unica_execution::receipt_ledger::TerminalDigest =
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
        let router = canonical_daemon_router(fake.owner(), fake.workspace_hint());

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
            vec![
                canonical_v5_terminal(&completed("strict"), unica_bootstrap::MAX_RESPONSE_BUDGET)
                    .unwrap()
                    .digest()
                    .clone()
            ],
            "only the strict receipt is acknowledged"
        );
    }

    #[test]
    fn lost_submit_response_follows_replaced_endpoint_without_resubmitting() {
        use unica_daemon::identity::{CoreIdentity, DaemonStateDirectory};
        let root = tempfile::tempdir().unwrap();
        let state_root = std::fs::canonicalize(root.path()).unwrap();
        let identity = CoreIdentity::production_v5();
        let successor = FakeDaemon::start(Box::new(|_, request| match request {
            V5ClientRequest::RecoverInvocationReceipt { receipt_key } => Step::Reply(
                direct_receipt(receipt_key.clone(), completed("successor recovery")),
            ),
            V5ClientRequest::AcknowledgeInvocationReceipt { .. } => Step::Close,
            other => panic!("successor received an unexpected request: {other:?}"),
        }));
        let successor_record = successor.record.clone();
        let publish_root = state_root.clone();
        let first = FakeDaemon::start(Box::new(move |_, request| {
            assert!(matches!(request, V5ClientRequest::SubmitInvocation { .. }));
            let state =
                DaemonStateDirectory::open(&publish_root, &CoreIdentity::production_v5()).unwrap();
            state.publish_v5_endpoint_record(&successor_record).unwrap();
            Step::Close
        }));
        let state = DaemonStateDirectory::open(&state_root, &identity).unwrap();
        state.publish_v5_endpoint_record(&first.record).unwrap();
        let client = V5DaemonClient::connect(
            state_root,
            identity,
            root.path().join("must-not-spawn"),
            Duration::from_secs(1),
        )
        .unwrap();
        let router = canonical_daemon_router(client, first.workspace_hint());
        let result = direct_result(call(&router, None));
        assert_eq!(
            result.structured_content.as_ref().unwrap()["summary"],
            "successor recovery"
        );
        assert_eq!(first.submissions(), 1);
        assert_eq!(successor.submissions(), 0);
        let submissions = first.seen();
        let V5ClientRequest::SubmitInvocation { invocation } = &submissions[0] else {
            panic!("missing submit")
        };
        let recovery = successor.seen();
        let V5ClientRequest::RecoverInvocationReceipt { receipt_key } = &recovery[0] else {
            panic!("missing recovery")
        };
        assert_eq!(
            receipt_key,
            &receipt_key_for(invocation, &V5DaemonClient::from(first.owner())).unwrap()
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
        let router = canonical_daemon_router(fake.owner(), fake.workspace_hint());

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
                            phase: unica_execution::receipt_runtime::response::V5InvocationPhase::ReservedBegun,
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
        let router = canonical_daemon_router(fake.owner(), fake.workspace_hint());

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
                        phase: unica_execution::receipt_runtime::response::V5InvocationPhase::ReservedActorBound,
                        accepted_epoch_ms: epoch_ms_now(),
                        original_budget_ms: 7_000,
                        cancel_requested: false,
                    },
                })
            }
            other => panic!("unexpected frame {other:?}"),
        }
        }));
        let router = canonical_daemon_router(fake.owner(), fake.workspace_hint());

        let refusal = lost_refusal(call(&router, Some(Duration::from_millis(400))));

        assert_names_the_reserved_task(&refusal, &fake);
        assert_eq!(fake.submissions(), 1);
    }

    /// The lost-submit refusal names the Task reserved for the one submitted
    /// call, in the data and in the text a host shows, and points at the
    /// wait for it instead of a second submission.
    fn assert_names_the_reserved_task(refusal: &ErrorData, fake: &FakeDaemon) {
        let seen = fake.seen();
        let reserved = seen
            .iter()
            .find_map(|request| match request {
                V5ClientRequest::SubmitInvocation { invocation } => {
                    Some(invocation.reserved_task_id().to_string())
                }
                _ => None,
            })
            .expect("the call was submitted");
        assert_eq!(refusal.code, ErrorCode(TOOL_EXECUTION_ERROR));
        assert!(
            refusal
                .message
                .starts_with("daemon deadline expired during invocation submit response"),
            "{}",
            refusal.message
        );
        assert!(refusal.message.contains(&reserved), "{}", refusal.message);
        assert_eq!(
            refusal.data,
            Some(json!({
                "code": "submit_response_lost",
                "outcome": "goElsewhere",
                "taskId": reserved,
                "next": [{"tool": "unica.task.result", "args": {"taskId": reserved}}],
            }))
        );
    }

    fn assert_not_delivered(refusal: &ErrorData) {
        assert_eq!(refusal.code, ErrorCode(TOOL_EXECUTION_ERROR));
        assert_eq!(
            refusal.data,
            Some(json!({"code": "submit_not_delivered", "outcome": "retry"}))
        );
        assert!(!refusal.message.contains("Task"), "{}", refusal.message);
    }

    #[test]
    fn a_lost_submit_is_the_queued_task_reserved_for_the_call() {
        let fake = FakeDaemon::start(Box::new(|_, request| match request {
            V5ClientRequest::SubmitInvocation { .. } => Step::Close,
            V5ClientRequest::RecoverInvocationReceipt { .. } => Step::Close,
            other => panic!("unexpected frame {other:?}"),
        }));
        let router = canonical_daemon_router(fake.owner(), fake.workspace_hint());

        let Ok(CanonicalCallOutcome::LostSubmit(lost)) = call(&router, None) else {
            panic!("a lost submit carries its Task");
        };

        let seen = fake.seen();
        let V5ClientRequest::SubmitInvocation { invocation } = &seen[0] else {
            panic!("first frame must be the submission: {seen:?}");
        };
        let key = receipt_key_for(invocation, &V5DaemonClient::from(fake.owner())).unwrap();
        assert_eq!(lost.task.task_id(), invocation.reserved_task_id());
        assert_eq!(lost.task.invocation_id(), invocation.invocation_id());
        assert_eq!(
            lost.task.receipt_key_digest(),
            &unica_execution::receipt_ledger::receipt_key_digest(&key)
        );
        assert_eq!(
            lost.task.status(),
            unica_execution_contract::invocation::InvocationStatus::Queued
        );
        assert_names_the_reserved_task(&lost.refusal, &fake);
    }

    #[test]
    fn a_daemon_refusing_the_connection_needs_a_person_not_a_repeat() {
        // The anchor stands; every further session is dropped right after its
        // handshake request, long before the cutoff.
        let fake = FakeDaemon::start_rejecting_peers();
        let router = canonical_daemon_router(fake.owner(), fake.workspace_hint());

        let refusal = call(&router, None).expect_err("a refused connection is refused");

        assert_eq!(refusal.code, ErrorCode(TOOL_EXECUTION_ERROR));
        assert_eq!(
            refusal.data,
            Some(json!({
                "code": "task_backend_failed",
                "outcome": "needsHuman",
                "detailCode": "backend_incompatible",
            }))
        );
        assert!(
            refusal.message.contains("will not run"),
            "{}",
            refusal.message
        );
        assert_eq!(fake.submissions(), 0);
    }

    #[test]
    fn a_call_whose_connection_never_stood_was_not_delivered() {
        // A handshake slower than the whole host budget: no frame of the
        // call leaves the frontend.
        let fake = FakeDaemon::start_with_handshake_delay(
            Box::new(|_, request| panic!("nothing is submitted: {request:?}")),
            Duration::from_millis(600),
        );
        let router = canonical_daemon_router(fake.owner(), fake.workspace_hint());

        let refusal = call(&router, Some(Duration::from_millis(200)))
            .expect_err("an unreachable daemon refuses");

        assert_not_delivered(&refusal);
        assert_eq!(fake.submissions(), 0);
    }

    #[test]
    fn daemon_budget_is_what_remains_of_the_frontend_budget_once_the_connection_stands() {
        let handshake_delay = Duration::from_millis(400);
        let fake = FakeDaemon::start_with_handshake_delay(
            Box::new(|raw, request| match request {
                V5ClientRequest::SubmitInvocation { .. } => {
                    Step::Reply(direct_receipt(daemon_side_key(raw), completed("viewed")))
                }
                V5ClientRequest::AcknowledgeInvocationReceipt { .. } => Step::Close,
                other => panic!("unexpected frame {other:?}"),
            }),
            handshake_delay,
        );
        let router = canonical_daemon_router(fake.owner(), fake.workspace_hint());

        let result = direct_result(call(&router, None));

        assert_eq!(result.is_error, Some(false));
        let seen = fake.seen();
        let V5ClientRequest::SubmitInvocation { invocation } = &seen[0] else {
            panic!("first frame must be the submission: {seen:?}");
        };
        let budget = Duration::from_millis(invocation.response_budget_ms());
        assert!(
            budget <= INVOCATION_HANDOFF_WINDOW - handshake_delay,
            "the handshake spent before the submission comes off the daemon budget: {budget:?}"
        );
        assert!(
            budget >= INVOCATION_HANDOFF_WINDOW - handshake_delay - Duration::from_secs(2),
            "only the time actually spent comes off the daemon budget: {budget:?}"
        );
    }

    #[test]
    fn recovery_without_a_receipt_fences_the_exact_key_and_answers_not_delivered() {
        let fake = FakeDaemon::start(Box::new(|_, request| match request {
            V5ClientRequest::SubmitInvocation { .. } => Step::Close,
            V5ClientRequest::RecoverInvocationReceipt { .. } => {
                Step::Reply(V5ServerResponse::Error {
                    code: V5DaemonErrorCode::ReceiptNotFound,
                })
            }
            // The daemon fences the key it has never seen.
            V5ClientRequest::FenceInvocationReceipt { receipt_key } => {
                Step::Reply(V5ServerResponse::Invocation {
                    outcome: V5InvocationResponse::ReceiptPending {
                        receipt_key: receipt_key.clone(),
                        phase: V5InvocationPhase::CancelReserved,
                        accepted_epoch_ms: epoch_ms_now(),
                        original_budget_ms: 0,
                        cancel_requested: true,
                    },
                })
            }
            other => panic!("unexpected frame {other:?}"),
        }));
        let router = canonical_daemon_router(fake.owner(), fake.workspace_hint());

        let refusal = call(&router, None).expect_err("lost submission is refused");

        assert_not_delivered(&refusal);
        assert_eq!(fake.submissions(), 1, "the call is never submitted again");
        let seen = fake.seen();
        let V5ClientRequest::SubmitInvocation { invocation } = &seen[0] else {
            panic!("first frame must be the submission: {seen:?}");
        };
        let fenced = seen
            .iter()
            .find_map(|request| match request {
                V5ClientRequest::FenceInvocationReceipt { receipt_key } => {
                    Some(receipt_key.clone())
                }
                _ => None,
            })
            .expect("the exact key is fenced");
        assert_eq!(
            fenced,
            receipt_key_for(invocation, &V5DaemonClient::from(fake.owner())).unwrap()
        );
    }

    #[test]
    fn recovery_without_a_receipt_whose_fence_meets_the_call_names_its_task() {
        // The frame overtook the fence: the cancel finds the call's own
        // receipt, so it may run, and the refusal names its Task.
        let fake = FakeDaemon::start(Box::new(|_, request| match request {
            V5ClientRequest::SubmitInvocation { .. } => Step::Close,
            V5ClientRequest::RecoverInvocationReceipt { .. } => {
                Step::Reply(V5ServerResponse::Error {
                    code: V5DaemonErrorCode::ReceiptNotFound,
                })
            }
            V5ClientRequest::FenceInvocationReceipt { receipt_key } => {
                Step::Reply(V5ServerResponse::Invocation {
                    outcome: V5InvocationResponse::ReceiptPending {
                        receipt_key: receipt_key.clone(),
                        phase: V5InvocationPhase::ReservedBegun,
                        accepted_epoch_ms: epoch_ms_now(),
                        original_budget_ms: 7_000,
                        cancel_requested: true,
                    },
                })
            }
            other => panic!("unexpected frame {other:?}"),
        }));
        let router = canonical_daemon_router(fake.owner(), fake.workspace_hint());

        let refusal = lost_refusal(call(&router, None));

        assert_names_the_reserved_task(&refusal, &fake);
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
        let router = canonical_daemon_router(fake.owner(), fake.workspace_hint());

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
        let router = canonical_daemon_router(fake.owner(), fake.workspace_hint());

        let refusal = lost_refusal(call(&router, Some(Duration::from_millis(300))));

        assert_names_the_reserved_task(&refusal, &fake);
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
        let router = canonical_daemon_router(fake.owner(), fake.workspace_hint());

        let rejected = call(&router, None).expect_err("overloaded daemon refuses");
        let foreign = (router.call)(
            V5ToolIdentity::Docs,
            &Map::new(),
            &unica_bootstrap::HostRequest::default(),
            deadline(None),
            CancellationToken::new().into(),
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
        let router = canonical_daemon_router(fake.owner(), fake.workspace_hint());

        let expired = (router.get)(task_id, deadline(None));
        let unexpected = (router.wait)(
            task_id,
            7_000,
            deadline(Some(Duration::from_millis(400))),
            CanonicalCancellation::default(),
        );
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
    fn live_daemon_hands_inline_work_over_the_cutoff_to_a_task_the_same_attempt_completes() {
        let service = Arc::new(ScriptedService {
            delay: Duration::from_millis(9_000),
            known_long: false,
            outcome: Mutex::new(None),
            executions: AtomicUsize::new(0),
        });
        let daemon = LiveDaemon::start(service.clone());
        let router = canonical_daemon_router(daemon.owner(), daemon.workspace_hint.clone());

        let started = Instant::now();
        let outcome = call(&router, None);
        let answered_after = started.elapsed();

        let Ok(CanonicalCallOutcome::Task(snapshot)) = outcome else {
            panic!("inline work past the cutoff must become a Task: {outcome:?}");
        };
        assert!(
            snapshot.completed_result().is_none(),
            "the handoff answers before the attempt finishes: {snapshot:?}"
        );
        // The daemon hands off at its own seventh second and the frontend
        // still gets the answer inside its transport window plus recovery.
        assert!(
            answered_after
                < INVOCATION_HANDOFF_WINDOW + RESPONSE_SERIALIZATION_MARGIN + RECOVERY_BUDGET,
            "the Task answered at {answered_after:?}"
        );
        assert!(
            answered_after >= INVOCATION_HANDOFF_WINDOW - Duration::from_millis(500),
            "a direct attempt is not shortened before the cutoff: {answered_after:?}"
        );

        let task_id = snapshot.task_id();
        let settle_by = Instant::now() + Duration::from_secs(20);
        let terminal = loop {
            let observed = (router.wait)(
                task_id,
                2_000,
                deadline(None),
                CanonicalCancellation::default(),
            )
            .expect("wait on the handed-off Task");
            if observed.completed_result().is_some() {
                break observed;
            }
            assert!(
                Instant::now() < settle_by,
                "the Task never completed: {observed:?}"
            );
        };
        assert_eq!(
            terminal
                .completed_result()
                .map(|result| result.summary.as_str()),
            Some("executed"),
            "the worker's own outcome becomes the Task terminal: {terminal:?}"
        );
        assert_eq!(
            service.executions.load(Ordering::SeqCst),
            1,
            "the handoff keeps the only attempt; nothing is re-executed"
        );
        drop(router);
        daemon.finish();
    }

    #[test]
    fn live_daemon_hands_a_failing_inline_attempt_over_the_cutoff_and_the_task_fails_once() {
        // The production runtime (no `receipt-ledger-test-support` feature) owns
        // the cutoff: a slow inline attempt is answered as a Task at the seventh
        // second, and the same single attempt's failure becomes that Task's
        // terminal — proving the deadline owner, not the harness, drives it.
        let service = Arc::new(ScriptedService {
            delay: Duration::from_millis(9_000),
            known_long: false,
            outcome: Mutex::new(Some(Err(
                unica_execution_contract::invocation::InvocationFailure::new(
                    "provider_exploded",
                    "/private/workspace bearer-secret",
                ),
            ))),
            executions: AtomicUsize::new(0),
        });
        let daemon = LiveDaemon::start(service.clone());
        let router = canonical_daemon_router(daemon.owner(), daemon.workspace_hint.clone());

        let started = Instant::now();
        let outcome = call(&router, None);
        let answered_after = started.elapsed();

        let Ok(CanonicalCallOutcome::Task(snapshot)) = outcome else {
            panic!("a slow inline attempt past the cutoff must become a Task: {outcome:?}");
        };
        assert!(
            snapshot.completed_result().is_none(),
            "the handoff answers before the attempt finishes: {snapshot:?}"
        );
        assert!(
            answered_after
                < INVOCATION_HANDOFF_WINDOW + RESPONSE_SERIALIZATION_MARGIN + RECOVERY_BUDGET,
            "the Task answered at {answered_after:?}"
        );
        assert!(
            answered_after >= INVOCATION_HANDOFF_WINDOW - Duration::from_millis(500),
            "a running attempt is not shortened before the cutoff: {answered_after:?}"
        );

        let task_id = snapshot.task_id();
        let settle_by = Instant::now() + Duration::from_secs(20);
        let terminal = loop {
            let observed = (router.wait)(
                task_id,
                2_000,
                deadline(None),
                CanonicalCancellation::default(),
            )
            .expect("wait on the handed-off Task");
            if !matches!(
                observed.status(),
                unica_execution_contract::invocation::InvocationStatus::Working
                    | unica_execution_contract::invocation::InvocationStatus::Queued
            ) {
                break observed;
            }
            assert!(
                Instant::now() < settle_by,
                "the Task never reached its terminal: {observed:?}"
            );
        };
        assert_eq!(
            terminal.status(),
            unica_execution_contract::invocation::InvocationStatus::Failed,
            "the worker's own failure becomes the Task terminal: {terminal:?}"
        );
        // The safe failure reason never leaks the provider's private text.
        assert!(!format!("{terminal:?}").contains("bearer-secret"));
        assert_eq!(
            service.executions.load(Ordering::SeqCst),
            1,
            "the handoff keeps the only attempt; nothing is re-executed"
        );
        drop(router);
        daemon.finish();
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
        let observed = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&observed);
        let owner = daemon.owner();
        let router = canonical_daemon_router_observed(
            owner,
            daemon.workspace_hint.clone(),
            Arc::new(move |key: &ReceiptKey| sink.lock().unwrap().push(key.clone())),
        );

        for _ in 0..2 {
            let result = direct_result(call(&router, None));
            assert_eq!(
                result
                    .structured_content
                    .as_ref()
                    .and_then(|value| value["summary"].as_str()),
                Some("executed")
            );
        }
        let keys = observed.lock().unwrap().clone();
        assert_eq!(keys.len(), 2);
        assert_ne!(keys[0].invocation_id(), keys[1].invocation_id());
        assert_ne!(keys[0].reserved_task_id(), keys[1].reserved_task_id());
        let mut peer = daemon.owner();
        for key in keys {
            let recovered = peer
                .recover_invocation_receipt_before(key, Instant::now() + Duration::from_secs(5))
                .expect("recover after acknowledgement");
            assert!(
                matches!(
                    recovered,
                    V5ServerResponse::Invocation {
                        outcome: V5InvocationResponse::Acknowledged { .. }
                    }
                ),
                "acknowledged receipt must compact to a tombstone: {recovered:?}"
            );
        }
        assert_eq!(
            service.executions.load(Ordering::SeqCst),
            2,
            "each new call executes once; recovery does not execute it again"
        );
        drop(peer);
        drop(router);
        daemon.finish();
    }

    /// #1309: a call far above the former 16 KiB frame reaches the service and
    /// runs; one above the request bound never leaves the frontend and is
    /// refused with the limit and its own size, not a bare `invalid_request`.
    #[test]
    fn live_daemon_runs_a_large_call_and_refuses_one_above_the_bound_unsent() {
        use unica_daemon::protocol_v5::MAX_V5_REQUEST_LINE_BYTES;
        let service = Arc::new(ScriptedService {
            delay: Duration::ZERO,
            known_long: false,
            outcome: Mutex::new(None),
            executions: AtomicUsize::new(0),
        });
        let daemon = LiveDaemon::start(service.clone());
        let router = canonical_daemon_router(daemon.owner(), daemon.workspace_hint.clone());
        let call_with = |text: String| {
            // The scripted service ignores the address; only its size matters.
            let arguments = json!({"at": format!("main:CommonModule.{text}")});
            (router.call)(
                V5ToolIdentity::View,
                arguments.as_object().expect("argument object"),
                &unica_bootstrap::HostRequest::default(),
                deadline(None),
                CancellationToken::new().into(),
            )
        };

        // Cyrillic text: one character is two bytes on the wire.
        let large = "Процедура".repeat(64 * 1024);
        assert!(large.len() > 1024 * 1024);
        let result = direct_result(call_with(large));
        assert_eq!(
            result
                .structured_content
                .as_ref()
                .and_then(|value| value["summary"].as_str()),
            Some("executed")
        );
        assert_eq!(service.executions.load(Ordering::SeqCst), 1);

        let refusal = direct_result(call_with("x".repeat(MAX_V5_REQUEST_LINE_BYTES)));
        let refusal = refusal.structured_content.expect("canonical refusal");
        assert_eq!(refusal["ok"], json!(false));
        let diagnostic = &refusal["diagnostics"][0];
        assert_eq!(diagnostic["code"], json!("provider_limit_exceeded"));
        assert_eq!(diagnostic["outcome"], json!("fixCall"));
        // Hosts often show the model only the message: it names both numbers.
        let message = diagnostic["message"].as_str().expect("refusal message");
        assert!(message.contains(&format!("{MAX_V5_REQUEST_LINE_BYTES}-byte request limit")));
        let actual: usize = message
            .strip_prefix("the call is ")
            .and_then(|rest| rest.split(' ').next())
            .and_then(|number| number.parse().ok())
            .expect("the message starts with the frame size");
        assert!(actual > MAX_V5_REQUEST_LINE_BYTES, "actual {actual}");
        assert_eq!(
            service.executions.load(Ordering::SeqCst),
            1,
            "the oversized call must not reach the service"
        );

        // The refused call did not cost the daemon its session: the next one runs.
        direct_result(call(&router, None));
        assert_eq!(service.executions.load(Ordering::SeqCst), 2);
        drop(router);
        daemon.finish();
    }

    #[test]
    fn live_daemon_failure_is_delivered_as_the_closed_reason_without_its_text() {
        // A workspace that changed under the call keeps that cause and its
        // remedy (#1338); any other failure stays the internal one.
        for (failure_code, code, message) in [
            (
                "provider_exploded",
                "invocation_failed",
                "daemon invocation stopped without a result. Repeat the call; if the problem persists, report this reason.",
            ),
            (
                unica_execution_contract::invocation::WORKSPACE_CHANGED_FAILURE,
                "workspace_changed",
                "the workspace changed while the call ran, so its result was not published. Repeat the call.",
            ),
        ] {
            let service = Arc::new(ScriptedService {
                delay: Duration::ZERO,
                known_long: false,
                outcome: Mutex::new(Some(Err(
                    unica_execution_contract::invocation::InvocationFailure::new(
                        failure_code,
                        "/private/workspace bearer-secret",
                    ),
                ))),
                executions: AtomicUsize::new(0),
            });
            let daemon = LiveDaemon::start(service.clone());
            let router = canonical_daemon_router(daemon.owner(), daemon.workspace_hint.clone());

            let refusal = call(&router, None).expect_err("failed execution is an error");

            assert_eq!(refusal.code, ErrorCode::INTERNAL_ERROR);
            assert_eq!(refusal.message, message);
            assert_eq!(refusal.data, Some(json!({"code": code})));
            assert!(!format!("{refusal:?}").contains("bearer-secret"));
            drop(router);
            daemon.finish();
        }
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
            Ok(CanonicalCallOutcome::LostSubmit(lost)) => {
                panic!("slow work lost its answer: {}", lost.refusal)
            }
            Err(error) => panic!("slow work refused: {error}"),
        };
        let task_id = handoff.task_id();
        let observed = (router.get)(task_id, deadline(None)).expect("get the handed-off task");
        let terminal = (router.wait)(
            task_id,
            5_000,
            deadline(None),
            CanonicalCancellation::default(),
        )
        .expect("wait for the terminal");
        let cancelled = (router.cancel)(task_id, deadline(None)).expect("cancel after terminal");

        assert!(!matches!(
            handoff.status(),
            unica_execution_contract::invocation::InvocationStatus::Completed
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
            unica_execution_contract::invocation::InvocationStatus::Completed
        );
        assert_eq!(service.executions.load(Ordering::SeqCst), 1);
        drop(router);
        daemon.finish();
    }

    /// Known-long work that runs until the test releases it and records
    /// whether its cancellation reached it.
    struct GatedService {
        executions: AtomicUsize,
        cancelled: AtomicUsize,
        released: std::sync::atomic::AtomicBool,
        /// Whether `apply` waits for the release too or ends at once.
        gate_mutations: bool,
    }

    impl GatedService {
        fn new() -> Arc<Self> {
            Self::gating_mutations(true)
        }

        fn gating_mutations(gate_mutations: bool) -> Arc<Self> {
            Arc::new(Self {
                executions: AtomicUsize::new(0),
                cancelled: AtomicUsize::new(0),
                released: std::sync::atomic::AtomicBool::new(false),
                gate_mutations,
            })
        }

        fn release(&self) {
            self.released.store(true, Ordering::SeqCst);
        }
    }

    impl crate::infrastructure::daemon::server::CanonicalInvocationService for GatedService {
        fn prepare(
            &self,
            _invocation: &crate::infrastructure::daemon::server::ActorBoundInvocation,
        ) -> Result<unica_execution::receipt_runtime::ports::ExecutionClass, Box<DomainResultAlias>>
        {
            use unica_execution::receipt_runtime::ports::{ExecutionClass, KnownLongReason};
            Ok(ExecutionClass::KnownLong(KnownLongReason::ExternalProcess))
        }

        fn execute(
            &self,
            invocation: &crate::infrastructure::daemon::server::ActorBoundExecution,
            cancellation: CancellationToken,
        ) -> Result<DomainResultAlias, unica_execution_contract::invocation::InvocationFailure>
        {
            self.executions.fetch_add(1, Ordering::SeqCst);
            if !self.gate_mutations && invocation.arguments().contains_key("ops") {
                return Ok(DomainResultAlias::success("applied"));
            }
            let started = Instant::now();
            while !self.released.load(Ordering::SeqCst)
                && started.elapsed() < Duration::from_secs(20)
            {
                if cancellation.is_cancelled() {
                    self.cancelled.fetch_add(1, Ordering::SeqCst);
                    return Err(
                        unica_execution_contract::invocation::InvocationFailure::new(
                            "cancelled",
                            "cancelled",
                        ),
                    );
                }
                thread::sleep(Duration::from_millis(5));
            }
            Ok(DomainResultAlias::success("shared traversal"))
        }
    }

    type DomainResultAlias = unica_execution_contract::invocation::DomainResult;

    /// Loses the first submit answer and every recovery answer, as an
    /// overloaded transport does: the frontend runs out of its recovery
    /// window although the daemon holds the call.
    #[derive(Default)]
    struct LostSubmitHooks {
        submits: AtomicUsize,
        lose_recoveries: std::sync::atomic::AtomicBool,
    }

    impl
        unica_execution::receipt_runtime::hooks::V5RuntimeHooks<
            crate::infrastructure::daemon::server::CanonicalInvocationHost,
        > for LostSubmitHooks
    {
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }

        fn submit_response_disconnect(&self) -> bool {
            self.submits.fetch_add(1, Ordering::SeqCst) == 0
        }

        fn recover_response_disconnect(&self) -> bool {
            self.lose_recoveries.load(Ordering::SeqCst)
        }
    }

    fn call_tool(
        router: &CanonicalDaemonRouter,
        tool: V5ToolIdentity,
        arguments: &Map<String, Value>,
    ) -> Result<CanonicalCallOutcome, ErrorData> {
        (router.call)(
            tool,
            arguments,
            &unica_bootstrap::HostRequest::default(),
            deadline(None),
            CancellationToken::new().into(),
        )
    }

    fn task_of(outcome: Result<CanonicalCallOutcome, ErrorData>) -> V5DaemonTaskSnapshot {
        match outcome {
            Ok(CanonicalCallOutcome::Task(snapshot)) => snapshot,
            Ok(CanonicalCallOutcome::Direct(result)) => panic!("unexpected direct: {result:?}"),
            Ok(CanonicalCallOutcome::LostSubmit(lost)) => {
                panic!("unexpected lost submit: {}", lost.refusal)
            }
            Err(error) => panic!("unexpected refusal: {error}"),
        }
    }

    fn wait_terminal(router: &CanonicalDaemonRouter, task_id: TaskId) -> V5DaemonTaskSnapshot {
        let started = Instant::now();
        loop {
            let snapshot = (router.wait)(
                task_id,
                2_000,
                deadline(None),
                CanonicalCancellation::default(),
            )
            .expect("wait for the Task");
            if matches!(
                snapshot.status(),
                unica_execution_contract::invocation::InvocationStatus::Completed
                    | unica_execution_contract::invocation::InvocationStatus::Failed
                    | unica_execution_contract::invocation::InvocationStatus::Cancelled
            ) {
                return snapshot;
            }
            assert!(
                started.elapsed() < Duration::from_secs(20),
                "Task never ended"
            );
        }
    }

    #[test]
    fn lost_submit_names_its_task_and_a_retry_joins_it_instead_of_running_again() {
        let service = GatedService::new();
        let hooks = Arc::new(LostSubmitHooks::default());
        hooks.lose_recoveries.store(true, Ordering::SeqCst);
        let daemon = LiveDaemon::start_with_aged_sources(service.clone(), Some(hooks.clone()));
        let router = canonical_daemon_router(daemon.owner(), daemon.workspace_hint.clone());
        let search = json!({"query": "Товары"}).as_object().unwrap().clone();

        // The daemon hands the search off to a Task, and the host never
        // learns it: the submit answer and every recovery answer are lost.
        let refusal = lost_refusal(call_tool(&router, V5ToolIdentity::Search, &search));
        hooks.lose_recoveries.store(false, Ordering::SeqCst);
        let named: TaskId = refusal.data.as_ref().unwrap()["taskId"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(
            refusal.data.as_ref().unwrap()["code"],
            "submit_response_lost"
        );

        // The agent repeats the call anyway: it joins the running Task under
        // its own id instead of starting the same traversal again.
        let retry = task_of(call_tool(&router, V5ToolIdentity::Search, &search));
        assert_ne!(retry.task_id(), named);
        let first = (router.get)(named, deadline(None)).expect("the named Task is readable");
        assert_eq!(first.task_id(), named);

        service.release();
        let first = wait_terminal(&router, named);
        let joined = wait_terminal(&router, retry.task_id());
        assert_eq!(
            first
                .completed_result()
                .map(|result| result.summary.as_str()),
            Some("shared traversal")
        );
        assert_eq!(first.completed_result(), joined.completed_result());
        assert_eq!(joined.task_id(), retry.task_id());
        assert_eq!(
            service.executions.load(Ordering::SeqCst),
            1,
            "the retry never ran the traversal again"
        );
        drop(router);
        daemon.finish();
    }

    #[test]
    fn a_read_whose_answer_was_lost_inline_is_readable_by_its_task_id() {
        // Inline work that ends directly: its answer is lost and no Task
        // ever exists, yet the id the refusal names returns the result.
        let service = Arc::new(ScriptedService {
            delay: Duration::from_millis(10),
            known_long: false,
            outcome: Mutex::new(None),
            executions: AtomicUsize::new(0),
        });
        let hooks = Arc::new(LostSubmitHooks::default());
        hooks.lose_recoveries.store(true, Ordering::SeqCst);
        let daemon = LiveDaemon::start_with_aged_sources(service.clone(), Some(hooks.clone()));
        let router = canonical_daemon_router(daemon.owner(), daemon.workspace_hint.clone());

        let refusal = lost_refusal(call(&router, None));
        let named: TaskId = refusal.data.as_ref().unwrap()["taskId"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        let terminal = wait_terminal(&router, named);

        assert_eq!(terminal.task_id(), named);
        assert_eq!(
            terminal
                .completed_result()
                .map(|result| result.summary.as_str()),
            Some("executed")
        );
        assert_eq!(service.executions.load(Ordering::SeqCst), 1);
        drop(router);
        daemon.finish();
    }

    #[test]
    fn only_an_identical_read_joins_and_a_mutation_never_does() {
        let service = GatedService::new();
        let daemon = LiveDaemon::start_with_aged_sources(service.clone(), None);
        let router = canonical_daemon_router(daemon.owner(), daemon.workspace_hint.clone());
        let same = json!({"query": "Товары"}).as_object().unwrap().clone();
        let other = json!({"query": "Цены"}).as_object().unwrap().clone();
        let edit = json!({
            "at": "main:Catalog.Товары",
            "ops": [{"op": "props.set", "args": {"values": {"Comment": "joined"}}}]
        })
        .as_object()
        .unwrap()
        .clone();

        let leader = task_of(call_tool(&router, V5ToolIdentity::Search, &same));
        let joined = task_of(call_tool(&router, V5ToolIdentity::Search, &same));
        assert_ne!(joined.task_id(), leader.task_id());
        assert_eq!(service.executions.load(Ordering::SeqCst), 1);
        let different = task_of(call_tool(&router, V5ToolIdentity::Search, &other));
        let first_apply = task_of(call_tool(&router, V5ToolIdentity::Apply, &edit));
        let second_apply = task_of(call_tool(&router, V5ToolIdentity::Apply, &edit));
        // A read submitted after a mutation sees its own traversal, and while
        // the mutation runs no read becomes a leader another read could join.
        let after_mutation = task_of(call_tool(&router, V5ToolIdentity::Search, &same));
        let during_mutation = task_of(call_tool(&router, V5ToolIdentity::Search, &same));

        service.release();
        for task in [
            &leader,
            &joined,
            &different,
            &first_apply,
            &second_apply,
            &after_mutation,
            &during_mutation,
        ] {
            wait_terminal(&router, task.task_id());
        }
        assert_eq!(
            service.executions.load(Ordering::SeqCst),
            6,
            "leader, different read, both applies and both reads after them ran; the joined read did not"
        );
        drop(router);
        daemon.finish();
    }

    #[test]
    fn one_holder_cancelling_leaves_the_shared_work_to_the_others_until_all_cancel() {
        let service = GatedService::new();
        let daemon = LiveDaemon::start_with_aged_sources(service.clone(), None);
        let router = canonical_daemon_router(daemon.owner(), daemon.workspace_hint.clone());
        let same = json!({"query": "Товары"}).as_object().unwrap().clone();

        let leader = task_of(call_tool(&router, V5ToolIdentity::Search, &same));
        let first = task_of(call_tool(&router, V5ToolIdentity::Search, &same));
        let second = task_of(call_tool(&router, V5ToolIdentity::Search, &same));
        assert_eq!(service.executions.load(Ordering::SeqCst), 1);

        // One joined holder and then the leader's own holder let go: each sees
        // its own cancellation, the work goes on for the remaining holder.
        let first_cancelled = (router.cancel)(first.task_id(), deadline(None)).unwrap();
        let leader_cancelled = (router.cancel)(leader.task_id(), deadline(None)).unwrap();
        for (snapshot, holder) in [(&first_cancelled, &first), (&leader_cancelled, &leader)] {
            assert_eq!(snapshot.task_id(), holder.task_id());
            assert!(
                snapshot.version() > holder.version(),
                "the holder's own cancellation is a newer state than the one it saw"
            );
            assert_eq!(
                snapshot.status(),
                unica_execution_contract::invocation::InvocationStatus::Cancelled
            );
        }
        let remaining = (router.get)(second.task_id(), deadline(None)).unwrap();
        assert!(!remaining.cancel_requested());
        assert!(matches!(
            remaining.status(),
            unica_execution_contract::invocation::InvocationStatus::Working
                | unica_execution_contract::invocation::InvocationStatus::Queued
        ));
        thread::sleep(Duration::from_millis(100));
        assert_eq!(service.cancelled.load(Ordering::SeqCst), 0);

        // The last holder lets go: now the shared work itself is cancelled.
        let second_cancelled = (router.cancel)(second.task_id(), deadline(None)).unwrap();
        assert_eq!(
            second_cancelled.status(),
            unica_execution_contract::invocation::InvocationStatus::Cancelled
        );
        let started = Instant::now();
        while service.cancelled.load(Ordering::SeqCst) == 0 {
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "the shared work never saw its cancellation"
            );
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(service.executions.load(Ordering::SeqCst), 1);
        drop(router);
        daemon.finish();
    }

    struct FixedHolderEpoch;

    impl unica_execution::invocation_store::EpochMillisClock for FixedHolderEpoch {
        fn now_epoch_millis(&self) -> u64 {
            1_000
        }
    }

    /// Settle the held service and listener even when an assertion is RED.
    struct JoinedClockFixture {
        router: Option<CanonicalDaemonRouter>,
        daemon: Option<LiveDaemon>,
        service: Arc<GatedService>,
    }

    impl JoinedClockFixture {
        fn start() -> Self {
            let service = GatedService::new();
            let daemon = LiveDaemon::start_with_aged_sources_and_clock(
                service.clone(),
                None,
                Some(Arc::new(FixedHolderEpoch)),
            );
            let router = canonical_daemon_router(daemon.owner(), daemon.workspace_hint.clone());
            Self {
                router: Some(router),
                daemon: Some(daemon),
                service,
            }
        }

        fn router(&self) -> &CanonicalDaemonRouter {
            self.router.as_ref().unwrap()
        }
    }

    impl Drop for JoinedClockFixture {
        fn drop(&mut self) {
            self.service.release();
            drop(self.router.take());
            if let Some(daemon) = self.daemon.take() {
                daemon.finish();
            }
        }
    }

    #[test]
    fn holder_cancel_before_completion_stays_cancelled_at_the_same_epoch() {
        use unica_execution_contract::invocation::InvocationStatus;
        for cancel_leader in [true, false] {
            let fixture = JoinedClockFixture::start();
            let router = fixture.router();
            let same = json!({"query": "Товары"}).as_object().unwrap().clone();
            let leader = task_of(call_tool(router, V5ToolIdentity::Search, &same));
            let joined = task_of(call_tool(router, V5ToolIdentity::Search, &same));
            assert_eq!(fixture.service.executions.load(Ordering::SeqCst), 1);
            let (leaving, remaining) = if cancel_leader {
                (&leader, &joined)
            } else {
                (&joined, &leader)
            };
            let cancelled = (router.cancel)(leaving.task_id(), deadline(None)).unwrap();
            assert_eq!(cancelled.task_id(), leaving.task_id());
            assert_eq!(cancelled.invocation_id(), leaving.invocation_id());
            assert_eq!(
                cancelled.status(),
                InvocationStatus::Cancelled,
                "the holder's cancellation must complete before producer release"
            );
            assert!(cancelled.cancel_requested());
            let V5DaemonTaskSnapshot::Cancelled {
                terminal_epoch_ms, ..
            } = &cancelled
            else {
                unreachable!();
            };
            assert_eq!(*terminal_epoch_ms, 1_000);
            fixture.service.release();

            let kept = wait_terminal(router, remaining.task_id());
            assert_eq!(
                kept.completed_result()
                    .map(|result| result.summary.as_str()),
                Some("shared traversal")
            );
            let V5DaemonTaskSnapshot::Completed {
                terminal_epoch_ms, ..
            } = &kept
            else {
                unreachable!();
            };
            assert_eq!(*terminal_epoch_ms, 1_000);
            let after = wait_terminal(router, leaving.task_id());
            assert_eq!(after.status(), InvocationStatus::Cancelled,
                "a completed holder cancellation must survive later shared completion in the same millisecond");
            assert_eq!(after.task_id(), leaving.task_id());
            assert_eq!(after.invocation_id(), leaving.invocation_id());
            assert_eq!(fixture.service.executions.load(Ordering::SeqCst), 1);
            assert_eq!(fixture.service.cancelled.load(Ordering::SeqCst), 0);
        }
    }

    #[test]
    fn shared_completion_before_holder_cancel_keeps_result_at_the_same_epoch() {
        use unica_execution_contract::invocation::InvocationStatus;
        for cancel_leader in [true, false] {
            let fixture = JoinedClockFixture::start();
            let router = fixture.router();
            let same = json!({"query": "Товары"}).as_object().unwrap().clone();
            let leader = task_of(call_tool(router, V5ToolIdentity::Search, &same));
            let joined = task_of(call_tool(router, V5ToolIdentity::Search, &same));
            assert_eq!(fixture.service.executions.load(Ordering::SeqCst), 1);
            fixture.service.release();
            let (leaving, remaining) = if cancel_leader {
                (&leader, &joined)
            } else {
                (&joined, &leader)
            };
            let completed = wait_terminal(router, leaving.task_id());
            assert_eq!(completed.status(), InvocationStatus::Completed);
            let V5DaemonTaskSnapshot::Completed {
                terminal_epoch_ms, ..
            } = &completed
            else {
                unreachable!();
            };
            assert_eq!(*terminal_epoch_ms, 1_000);
            let late_cancel = (router.cancel)(leaving.task_id(), deadline(None)).unwrap();
            assert_eq!(late_cancel, completed,
                "a holder cancelling after completion must retain the original terminal even at the same millisecond");
            assert_eq!(wait_terminal(router, leaving.task_id()), completed);
            let kept = wait_terminal(router, remaining.task_id());
            assert_eq!(kept.completed_result(), completed.completed_result());
            assert_eq!(fixture.service.executions.load(Ordering::SeqCst), 1);
            assert_eq!(fixture.service.cancelled.load(Ordering::SeqCst), 0);
        }
    }

    #[test]
    fn a_joined_holder_keeps_the_result_after_another_holder_cancelled() {
        let service = GatedService::new();
        let daemon = LiveDaemon::start_with_aged_sources(service.clone(), None);
        let router = canonical_daemon_router(daemon.owner(), daemon.workspace_hint.clone());
        let same = json!({"query": "Товары"}).as_object().unwrap().clone();

        let leader = task_of(call_tool(&router, V5ToolIdentity::Search, &same));
        let joined = task_of(call_tool(&router, V5ToolIdentity::Search, &same));
        (router.cancel)(leader.task_id(), deadline(None)).unwrap();
        service.release();

        let kept = wait_terminal(&router, joined.task_id());
        assert_eq!(
            kept.completed_result()
                .map(|result| result.summary.as_str()),
            Some("shared traversal")
        );
        assert_eq!(
            wait_terminal(&router, leader.task_id()).status(),
            unica_execution_contract::invocation::InvocationStatus::Cancelled
        );
        assert_eq!(service.cancelled.load(Ordering::SeqCst), 0);
        drop(router);
        daemon.finish();
    }

    #[test]
    fn an_exact_repeat_of_a_call_with_its_own_receipt_never_becomes_a_joiner() {
        let service = GatedService::gating_mutations(false);
        let daemon = LiveDaemon::start_with_aged_sources(service.clone(), None);
        let router = canonical_daemon_router(daemon.owner(), daemon.workspace_hint.clone());
        let same = json!({"query": "Товары"}).as_object().unwrap().clone();
        let edit = json!({
            "at": "main:Catalog.Товары",
            "ops": [{"op": "props.set", "args": {"values": {"Comment": "between"}}}]
        })
        .as_object()
        .unwrap()
        .clone();
        let workspace = unica_bootstrap::HostWorkspaceContext::from(daemon.workspace_hint.clone())
            .resolve_request(&unica_bootstrap::HostRequest::default())
            .unwrap();
        let own = V5InvocationRequest::new(
            InvocationId::new(),
            TaskId::new(),
            V5ToolIdentity::Search,
            same.clone(),
            workspace,
            5_000,
        )
        .unwrap();
        let submit = |invocation: V5InvocationRequest| {
            daemon
                .owner()
                .submit_invocation_before(invocation, Instant::now() + Duration::from_secs(10))
                .unwrap()
        };

        // The call reserves its own receipt and becomes a Task. A mutation
        // then ends, and a later identical read leads the next joins.
        let first = match submit(own.clone()) {
            V5ServerResponse::Invocation {
                outcome: V5InvocationResponse::Task { snapshot },
            } => snapshot,
            other => panic!("the first call becomes its own Task: {other:?}"),
        };
        let applied = task_of(call_tool(&router, V5ToolIdentity::Apply, &edit));
        wait_terminal(&router, applied.task_id());
        let later = task_of(call_tool(&router, V5ToolIdentity::Search, &same));
        assert_eq!(service.executions.load(Ordering::SeqCst), 3);

        // The exact repeat answers from its own receipt, never as a holder of
        // the later read: cancelling it still cancels its own work.
        let _ = submit(own);
        (router.cancel)(first.task_id(), deadline(None)).unwrap();
        let started = Instant::now();
        while service.cancelled.load(Ordering::SeqCst) == 0 {
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "the repeat was joined to another Task instead of reading its own"
            );
            thread::sleep(Duration::from_millis(10));
        }
        service.release();
        assert_eq!(
            wait_terminal(&router, later.task_id())
                .completed_result()
                .map(|result| result.summary.as_str()),
            Some("shared traversal")
        );
        assert_eq!(service.executions.load(Ordering::SeqCst), 3);
        drop(router);
        daemon.finish();
    }

    fn raw_invocation(
        daemon: &LiveDaemon,
        tool: V5ToolIdentity,
        arguments: &Map<String, Value>,
        budget_ms: u64,
    ) -> V5InvocationRequest {
        let workspace = unica_bootstrap::HostWorkspaceContext::from(daemon.workspace_hint.clone())
            .resolve_request(&unica_bootstrap::HostRequest::default())
            .unwrap();
        V5InvocationRequest::new(
            InvocationId::new(),
            TaskId::new(),
            tool,
            arguments.clone(),
            workspace,
            budget_ms,
        )
        .unwrap()
    }

    fn raw_key(daemon: &LiveDaemon, invocation: &V5InvocationRequest) -> ReceiptKey {
        receipt_key_for(invocation, &V5DaemonClient::from(daemon.owner())).unwrap()
    }

    #[test]
    fn an_external_edit_after_the_leader_started_makes_the_repeat_run_fresh() {
        let service = GatedService::new();
        let daemon = LiveDaemon::start_with_aged_sources(service.clone(), None);
        let router = canonical_daemon_router(daemon.owner(), daemon.workspace_hint.clone());
        let check = json!({"query": "Модуль"}).as_object().unwrap().clone();

        let leader = task_of(call_tool(&router, V5ToolIdentity::Search, &check));
        // Nothing changed: the identical check joins the running walk.
        let joined = task_of(call_tool(&router, V5ToolIdentity::Search, &check));
        assert_eq!(service.executions.load(Ordering::SeqCst), 1);

        // The agent edits a module directly, not through Unica.
        std::fs::write(
            std::path::Path::new(&daemon.workspace_hint).join("src/CommonModules/Module.bsl"),
            "// v2\n",
        )
        .unwrap();
        let fresh = task_of(call_tool(&router, V5ToolIdentity::Search, &check));
        assert_ne!(fresh.task_id(), leader.task_id());
        let started = Instant::now();
        while service.executions.load(Ordering::SeqCst) < 2 {
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "the check after the edit joined the walk that began before it"
            );
            thread::sleep(Duration::from_millis(10));
        }
        service.release();
        for task in [&leader, &joined, &fresh] {
            wait_terminal(&router, task.task_id());
        }
        assert_eq!(service.executions.load(Ordering::SeqCst), 2);
        drop(router);
        daemon.finish();
    }

    #[test]
    fn a_frame_that_arrives_cancelled_never_leads_identical_reads() {
        let service = Arc::new(ScriptedService {
            delay: Duration::from_millis(10),
            known_long: false,
            outcome: Mutex::new(None),
            executions: AtomicUsize::new(0),
        });
        let daemon = LiveDaemon::start_with_aged_sources(service.clone(), None);
        let router = canonical_daemon_router(daemon.owner(), daemon.workspace_hint.clone());

        // An early cancel for the exact key, then the frame itself; its
        // budget is spent by the time the identical read arrives.
        let late = raw_invocation(&daemon, V5ToolIdentity::View, &arguments(), 1_000);
        let key = raw_key(&daemon, &late);
        V5DaemonProcessOwner::cancel_at_endpoint(daemon.owner().control_endpoint(), key).unwrap();
        let late = daemon
            .owner()
            .submit_invocation_before(late, Instant::now() + Duration::from_secs(10))
            .unwrap();
        assert!(
            matches!(late, V5ServerResponse::Invocation { .. }),
            "the cancelled frame is answered: {late:?}"
        );
        thread::sleep(Duration::from_millis(1_200));

        let started = Instant::now();
        let result = direct_result(call(&router, None));
        assert_eq!(result.is_error, Some(false));
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "the identical read waited for a leader that never runs: {:?}",
            started.elapsed()
        );
        assert_eq!(service.executions.load(Ordering::SeqCst), 1);
        drop(router);
        daemon.finish();
    }

    #[test]
    fn a_fenced_frame_never_runs_and_a_held_one_is_answered_by_its_state() {
        let service = GatedService::new();
        let daemon = LiveDaemon::start_with_aged_sources(service.clone(), None);
        let router = canonical_daemon_router(daemon.owner(), daemon.workspace_hint.clone());
        let search = json!({"query": "Товары"}).as_object().unwrap().clone();

        let fenced = raw_invocation(&daemon, V5ToolIdentity::Search, &search, 5_000);
        let fenced_key = raw_key(&daemon, &fenced);
        let answer = daemon
            .owner()
            .fence_invocation_receipt_before(
                fenced_key.clone(),
                Instant::now() + Duration::from_secs(5),
            )
            .unwrap();
        assert!(matches!(
            answer,
            V5ServerResponse::Invocation {
                outcome: V5InvocationResponse::ReceiptPending {
                    phase: V5InvocationPhase::CancelReserved,
                    cancel_requested: true,
                    ..
                }
            }
        ));
        let late = daemon
            .owner()
            .submit_invocation_before(fenced, Instant::now() + Duration::from_secs(10))
            .unwrap();
        assert!(
            !matches!(late, V5ServerResponse::Invocation { .. }),
            "{late:?}"
        );
        assert_eq!(service.executions.load(Ordering::SeqCst), 0);
        assert!((router.get)(fenced_key.reserved_task_id(), deadline(None)).is_err());

        // A call the daemon already holds is not fenced: it keeps running.
        let held = task_of(call_tool(&router, V5ToolIdentity::Search, &search));
        service.release();
        assert_eq!(
            wait_terminal(&router, held.task_id())
                .completed_result()
                .map(|result| result.summary.as_str()),
            Some("shared traversal")
        );
        drop(router);
        daemon.finish();
    }

    #[test]
    fn the_task_id_of_a_call_still_inline_can_be_cancelled() {
        let service = Arc::new(ScriptedService {
            delay: Duration::from_millis(1_500),
            known_long: false,
            outcome: Mutex::new(None),
            executions: AtomicUsize::new(0),
        });
        let daemon = LiveDaemon::start_with_aged_sources(service.clone(), None);
        let router = canonical_daemon_router(daemon.owner(), daemon.workspace_hint.clone());
        let inline = raw_invocation(&daemon, V5ToolIdentity::View, &arguments(), 7_000);
        let task_id = inline.reserved_task_id();
        let mut owner = daemon.owner();
        let submit = thread::spawn(move || {
            owner.submit_invocation_before(inline, Instant::now() + Duration::from_secs(10))
        });
        let started = Instant::now();
        while service.executions.load(Ordering::SeqCst) == 0 {
            assert!(started.elapsed() < Duration::from_secs(5), "never started");
            thread::sleep(Duration::from_millis(5));
        }

        let cancelled = (router.cancel)(task_id, deadline(None))
            .expect("the reserved id of an inline call is cancellable");

        assert_eq!(cancelled.task_id(), task_id);
        assert!(
            cancelled.cancel_requested()
                || cancelled.status()
                    == unica_execution_contract::invocation::InvocationStatus::Cancelled,
            "{cancelled:?}"
        );
        let _ = submit.join().unwrap();
        drop(router);
        daemon.finish();
    }
}
