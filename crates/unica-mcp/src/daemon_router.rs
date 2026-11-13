//! Canonical v0.13 router over the protocol-v5 user daemon.
//!
//! The stdio frontend renews its daemon lease after replacement and gives every
//! invocation and every Task operation a peer session of its own. A Direct
//! terminal is acknowledged only after its host-facing value exists; a lost
//! submit response is recovered by the exact receipt key the frontend derives
//! itself, never by a second submission.
use crate::canonical_cancellation::{CanonicalCancellation, ControlFailure};
use crate::task_projection::{self, DirectProjection};
use rmcp::model::{CallToolResult, ErrorCode, ErrorData};
use serde_json::{Map, Value};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use unica_daemon_protocol::client_v5::{
    TaskObservationStage, TaskObservationTimer, V5DaemonClient, V5DaemonProcessOwner,
    V5TaskExchangeError, V5TransportError,
};
use unica_daemon_protocol::protocol_v5::{
    V5DaemonErrorCode, V5InvocationRequest, V5ServerResponse,
};
use unica_execution::invocation::{
    handoff_budget, normalized_arguments_hash, INVOCATION_HANDOFF_WINDOW,
    RESPONSE_SERIALIZATION_MARGIN, RESPONSE_SERIALIZATION_MARGIN_MS,
};
use unica_execution::receipt_ledger::{
    request_scope_hash, ReceiptKey, ReceiptTerminalOutcome, RequestIdentity, V5ToolIdentity,
    MAX_ORIGINAL_RESPONSE_BUDGET_MS,
};
use unica_execution::receipt_runtime::response::{
    V5DaemonTaskSnapshot, V5InvocationPhase, V5InvocationResponse,
};
use unica_execution_contract::invocation::{InvocationId, TaskId};
use unica_refusal::RefusalCode;
#[cfg(test)]
use unica_work_control::cancellation::CancellationToken;

pub const TOOL_EXECUTION_ERROR: i32 = -32000;
/// The acknowledgement travels after the transport budget of the invocation
/// may already be spent; it gets a short budget of its own instead of racing
/// an exhausted cutoff.
// resource-limit: daemon.acknowledgement-budget-ms
// class: deadline
// unit: ms
// scope: request
// value: 500
// resource: Собственный срок отправки подтверждения после того, как транспортный бюджет вызова уже мог быть исчерпан.
// status: unproven
// origin: dcbead88c (#774)
// problem: Подтверждение не должно гоняться с исчерпанным cutoff (комментарий в daemon_router.rs).
// on-reach: Подтверждение, не уложившееся в срок, не доставляется.
const ACKNOWLEDGEMENT_BUDGET: Duration = Duration::from_millis(500);
/// Recovery of a still-reserved receipt polls the daemon no more often than this.
// resource-limit: daemon.recovery-poll
// class: step
// unit: ms
// scope: request
// value: 25
// resource: Наименьший интервал опроса демона при восстановлении ещё зарезервированной квитанции.
// status: unproven
// origin: #774
// problem: неизвестно
// on-reach: frontend ждёт интервал и повторяет опрос; если до конца окна меньше интервала, отвечает закрытым отказом о потерянном вызове
const RECOVERY_POLL_INTERVAL: Duration = Duration::from_millis(25);
/// A submit response is most often lost exactly at the frontend cutoff: the
/// daemon hands the work off to a Task within its own budget, and the durable
/// promotion under load overruns the 125 ms margin. Exact recovery is a
/// read-only reconciliation of that receipt, so it gets a bounded window of
/// its own after the cutoff instead of failing with nothing left to spend.
// resource-limit: daemon.recovery-budget-ms
// class: deadline
// unit: ms
// scope: request
// value: 750
// resource: Окно точного восстановления потерянного ответа submit после cutoff фронтенда.
// status: unproven
// origin: 45c5825e6 (#780)
// problem: Ответ submit чаще всего теряется на cutoff, когда продвижение в задание под нагрузкой перерастает запас 125 мс (комментарий в daemon_router.rs).
// on-reach: Восстановление прекращается; отказ как у v3 (#780).
pub const RECOVERY_BUDGET: Duration = Duration::from_millis(750);

/// One absolute frontend budget per request: captured at receipt and only
/// narrowed afterwards, never re-derived from a later `now`.
#[derive(Debug, Clone, Copy)]
pub struct FrontendInvocationDeadline {
    received_at: Instant,
    host_remaining_at_receipt: Option<Duration>,
}

impl FrontendInvocationDeadline {
    pub fn new(received_at: Instant, host_remaining_at_receipt: Option<Duration>) -> Self {
        Self {
            received_at,
            host_remaining_at_receipt,
        }
    }

    pub fn received_at(self) -> Instant {
        self.received_at
    }

    pub fn remaining_at(self, now: Instant) -> Duration {
        remaining_invocation_budget(self.received_at, now, self.host_remaining_at_receipt)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn remaining_transport_at(self, now: Instant) -> Duration {
        let elapsed = now.saturating_duration_since(self.received_at);
        let own_remaining = INVOCATION_HANDOFF_WINDOW
            .saturating_add(RESPONSE_SERIALIZATION_MARGIN)
            .saturating_sub(elapsed);
        let host_remaining = self
            .host_remaining_at_receipt
            .map(|remaining| remaining.saturating_sub(elapsed));
        host_remaining.map_or(own_remaining, |remaining| own_remaining.min(remaining))
    }

    pub fn transport_cutoff(self) -> Instant {
        let own_cutoff = self
            .received_at
            .checked_add(INVOCATION_HANDOFF_WINDOW.saturating_add(RESPONSE_SERIALIZATION_MARGIN))
            .expect("bounded frontend transport cutoff");
        self.host_remaining_at_receipt
            .and_then(|remaining| self.received_at.checked_add(remaining))
            .map_or(own_cutoff, |host_cutoff| own_cutoff.min(host_cutoff))
    }
}

pub fn remaining_invocation_budget(
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
pub enum CanonicalCallOutcome {
    Direct(CallToolResult),
    /// A Direct refusal whose private result offers a person a choice; the
    /// refusal is already projected without it.
    Choice {
        refusal: CallToolResult,
        choice: Box<unica_execution_contract::invocation::HumanChoice>,
    },
    /// The Task the call continues as: the daemon's handoff, or, when the
    /// submit answer was lost and could not be recovered, the queued Task
    /// reserved for the call.
    Task(V5DaemonTaskSnapshot),
}

/// How recovery of a lost submit answer ended without a daemon answer.
enum RecoveryEnd {
    /// The frame reached the daemon: the call may continue as its Task.
    Lost,
    /// The answer is a refusal of its own.
    Refused(ErrorData),
}

pub type CanonicalCallHandler = dyn Fn(
        V5ToolIdentity,
        &Map<String, Value>,
        &unica_bootstrap::HostRequest,
        FrontendInvocationDeadline,
        CanonicalCancellation,
    ) -> Result<CanonicalCallOutcome, ErrorData>
    + Send
    + Sync;

pub type CanonicalTaskHandler = dyn Fn(TaskId, FrontendInvocationDeadline) -> Result<V5DaemonTaskSnapshot, V5TaskExchangeError>
    + Send
    + Sync;

pub type CanonicalTaskWaitHandler = dyn Fn(
        TaskId,
        u64,
        FrontendInvocationDeadline,
        CanonicalCancellation,
    ) -> Result<V5DaemonTaskSnapshot, V5TaskExchangeError>
    + Send
    + Sync;

#[derive(Clone)]
pub struct CanonicalDaemonRouter {
    pub call: Arc<CanonicalCallHandler>,
    pub get: Arc<CanonicalTaskHandler>,
    pub wait: Arc<CanonicalTaskWaitHandler>,
    pub cancel: Arc<CanonicalTaskHandler>,
}

type ReceiptObserver = Arc<dyn Fn(&ReceiptKey) + Send + Sync>;

/// Build the router over a renewable protocol-v5 owner lease.
pub fn canonical_daemon_router(
    client: impl Into<V5DaemonClient>,
    workspace: impl Into<unica_bootstrap::HostWorkspaceContext>,
) -> CanonicalDaemonRouter {
    build_router(client.into(), workspace.into(), None)
}

#[cfg(feature = "test-support")]
pub fn canonical_daemon_router_observed(
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
            // Connect and exchange spend the frontend's own transport cutoff,
            // as `get` and `cancel` do; `waitMs` bounds only the daemon wait.
            let cutoff = deadline.transport_cutoff();
            let wait_cutoff = daemon_wait_cutoff(wait_ms, deadline);
            let mut peer = task_peer(&wait_client, cutoff)?;
            let control_timer = TaskObservationTimer::start(cutoff);
            let interrupt = peer.observation_interrupt_handle().map_err(|_| {
                V5TaskExchangeError::Transport(control_timer.failed(
                    TaskObservationStage::BindControl,
                    Instant::now(),
                    Some(&peer.control_endpoint()),
                ))
            })?;
            cancellation
                .bind_control(Arc::new(move || {
                    interrupt
                        .interrupt()
                        .map_err(|error| ControlFailure::BeforeSend(error.to_string()))
                }))
                .map_err(|_| {
                    V5TaskExchangeError::Transport(control_timer.failed(
                        TaskObservationStage::BindControl,
                        Instant::now(),
                        Some(&peer.control_endpoint()),
                    ))
                })?;
            // The daemon wait shrinks by what connect and handshake already spent
            // and by the response margin; neither cutoff ever moves.
            let bounded_wait_ms = bounded_wait_ms(wait_ms, wait_cutoff, Instant::now());
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
    let timer = TaskObservationTimer::start(cutoff);
    if Instant::now() >= cutoff {
        return Err(V5TaskExchangeError::Transport(timer.failed(
            TaskObservationStage::AcquirePeer,
            Instant::now(),
            None,
        )));
    }
    client.connect_peer_before(cutoff).map_err(|_| {
        V5TaskExchangeError::Transport(timer.failed(
            TaskObservationStage::AcquirePeer,
            Instant::now(),
            None,
        ))
    })
}

/// The end of the daemon wait of `unica.task.result`, derived once from the
/// moment the frontend received the request: the requested wait plus one
/// response margin, never beyond the frontend transport cutoff. Time spent
/// before the daemon wait starts is never replenished. Connect and exchange
/// are bounded by the transport cutoff, not by this one: a ready result read
/// with `waitMs: 0` on a slow host arrives as the result.
pub fn daemon_wait_cutoff(requested_wait_ms: u64, deadline: FrontendInvocationDeadline) -> Instant {
    let requested_cutoff = deadline
        .received_at()
        .checked_add(
            Duration::from_millis(requested_wait_ms).saturating_add(RESPONSE_SERIALIZATION_MARGIN),
        )
        .expect("bounded task wait cutoff");
    requested_cutoff.min(deadline.transport_cutoff())
}

pub fn bounded_wait_ms(requested_wait_ms: u64, cutoff: Instant, now: Instant) -> u64 {
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
                Err(RecoveryEnd::Lost) => return Ok(lost_submit_task(&receipt_key)),
                Err(RecoveryEnd::Refused(refusal)) => return Err(refusal),
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
    endpoint: unica_daemon_protocol::protocol_v5::V5EndpointRecord,
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
pub fn receipt_key_for(
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
/// recovery window closes, the host gets the reserved Task: the daemon may
/// hold the work, nobody replays it.
fn recover_receipt(
    client: &V5DaemonClient,
    receipt_key: &ReceiptKey,
    cutoff: Instant,
) -> Result<(V5DaemonProcessOwner, V5ServerResponse), RecoveryEnd> {
    loop {
        let mut peer = client
            .connect_peer_before(cutoff)
            .map_err(|_| RecoveryEnd::Lost)?;
        let response = peer
            .recover_invocation_receipt_before(receipt_key.clone(), cutoff)
            .map_err(|_| RecoveryEnd::Lost)?;
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
                // reserved Task as for a lost submission.
                if until_settled >= remaining || remaining <= RECOVERY_POLL_INTERVAL {
                    return Err(RecoveryEnd::Lost);
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
) -> RecoveryEnd {
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
        }) if &fenced_key == receipt_key => RecoveryEnd::Refused(not_delivered_refusal(
            "daemon did not receive the invocation submit; it will not run".to_owned(),
        )),
        _ => RecoveryEnd::Lost,
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
            let choice = match receipt.terminal() {
                ReceiptTerminalOutcome::Completed { result } if !result.ok => {
                    result.human_choice.clone()
                }
                _ => None,
            };
            let acknowledgement_deadline = Instant::now() + ACKNOWLEDGEMENT_BUDGET;
            // The acknowledgement proves the transfer daemon → frontend and
            // nothing more: its own outcome cannot change what the host gets.
            let _ = peer.acknowledge_invocation_receipt_before(
                receipt_key.clone(),
                receipt.terminal_digest().clone(),
                acknowledgement_deadline,
            );
            match projection {
                DirectProjection::Result(refusal) if choice.is_some() => {
                    Ok(CanonicalCallOutcome::Choice {
                        refusal,
                        choice: choice.expect("checked above"),
                    })
                }
                DirectProjection::Result(result) => Ok(CanonicalCallOutcome::Direct(result)),
                DirectProjection::Error(error) => Err(error),
            }
        }
        V5ServerResponse::Invocation {
            outcome: V5InvocationResponse::Task { snapshot },
        } => {
            // A submitted or recovered call continues only as the Task reserved
            // for its exact key.
            if snapshot.task_id() != receipt_key.reserved_task_id()
                || snapshot.invocation_id() != receipt_key.invocation_id()
                || snapshot.receipt_key_digest()
                    != &unica_execution::receipt_ledger::receipt_key_digest(receipt_key)
            {
                return Err(protocol_refusal(
                    "daemon answered with a task of a different invocation",
                ));
            }
            Ok(CanonicalCallOutcome::Task(snapshot))
        }
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
        | V5ServerResponse::InvocationAcknowledged { .. }
        | V5ServerResponse::LspOpened { .. }
        | V5ServerResponse::LspAnswer { .. }
        | V5ServerResponse::LspAccepted {}
        | V5ServerResponse::LspNotifications { .. }
        | V5ServerResponse::LspRefused { .. } => Err(protocol_refusal(
            "daemon returned an unexpected response to an invocation",
        )),
    }
}

pub fn epoch_ms_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

/// Poll interval the queued Task of a lost submit announces; the daemon's
/// own receipt-backed snapshots use the same 100 ms.
// resource-limit: mcp.lost-submit-poll-interval-ms
// class: step
// unit: ms
// scope: record
// value: 100
// resource: Интервал опроса, объявляемый заданию потерянного submit.
// status: unproven
// origin: #1376 (#1388)
// problem: неизвестно
// on-reach: клиенту сообщается pollInterval 100 мс; сам предел ничего не обрывает
const LOST_SUBMIT_POLL_INTERVAL_MS: u64 = 100;

/// The queued Task reserved for a call whose submit answer was lost. The
/// daemon answers it by that id whether the call is still inline, a Task, or
/// ended directly; an id the daemon never took answers `task_not_found`.
fn lost_submit_task(receipt_key: &ReceiptKey) -> CanonicalCallOutcome {
    let now = epoch_ms_now();
    CanonicalCallOutcome::Task(V5DaemonTaskSnapshot::Queued {
        task_id: receipt_key.reserved_task_id(),
        invocation_id: receipt_key.invocation_id(),
        receipt_key_digest: unica_execution::receipt_ledger::receipt_key_digest(receipt_key),
        created_at_epoch_ms: now,
        updated_at_epoch_ms: now,
        ttl_ms: unica_execution::receipt_ledger::DIRECT_TERMINAL_RETENTION_MS,
        poll_interval_ms: LOST_SUBMIT_POLL_INTERVAL_MS,
        version: 1,
        cancel_requested: false,
    })
}

/// A connection that could not stand in the time of the call is transient:
/// the daemon may be alive and busy, the call never left the frontend and may
/// be repeated as is. One refused — the daemon could not be started,
/// rejected the handshake or belongs to another core — stays refused on a
/// repeat: a person has to look.
fn unconnected_refusal(error: &V5TransportError, cutoff: Instant) -> ErrorData {
    if let V5TransportError::NotConnectedInTime(message) = error {
        return not_delivered_refusal(format!(
            "{message}; the call did not reach the daemon and may be repeated as is"
        ));
    }
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
mod medium_tests;
