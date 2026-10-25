//! The single durable invocation, cutoff, cancellation, Task and recovery owner.
pub mod authority;
#[cfg(feature = "receipt-ledger-test-support")]
pub mod contracts;
pub mod hooks;
mod join_freshness;
mod joining;
pub mod ports;
pub mod response;
mod retirement_facts;
use crate::invocation::{RESPONSE_SERIALIZATION_MARGIN, TASK_RECONCILIATION_BUDGET};
use crate::invocation_store::EpochMillisClock;
use crate::invocation_store_v5::{
    InvocationStoreV5, NewV5InvocationRecord, RecoveryTerminalReason, TaskStoreRecoveryCatalog,
    V5DeleteTerminalOutcome, V5SafeFailureReason, V5StartWorkingOutcome, V5StoredInvocationRecord,
    V5StoredInvocationSchemaVersion, V5StoredTask, V5TaskIdentity, V5TaskMismatch,
    V5TaskRetirement, V5TaskStoreError, V5TerminalPublication,
};
use crate::invocation_v5::{
    decide_cancel_reserved_submit, decide_cancel_resolution, CancelInvocationDecision,
    CancelReservedSubmitDecision,
};
use crate::lifecycle_link_store::{
    TaskLifecycleLinkCatalogEntry, TaskLifecycleLinkRecord, TaskLifecycleLinkStoreError,
    TaskLifecycleLinkStoreV5, TaskLinkReservation,
};
use crate::receipt_ledger::{
    canonical_v5_terminal, AttemptPhase, CanonicalTerminalError, ClosedTerminalStatus,
    CoreIdentityDigest, HandoffTerminalStage, OriginalCutoffDescriptor, PreparedWireFrame,
    ReceiptKey, ReceiptKeyDigest, ReceiptLedgerError, ReceiptState, ReceiptTaskProjection,
    ReceiptTerminalOutcome, ReserveOutcome, ReservedPhase, TaskBoundReceipt,
    TaskCancellationReceipt, TaskHandoffActorBoundReceipt, TaskPromisedActorBoundReceipt,
    TaskPromisedUnboundReceipt, TaskRetirementPendingReceipt, TaskTerminalBoundReceipt,
    TerminalDigest, DIRECT_TERMINAL_RETENTION_MS,
};
use crate::receipt_ledger_actor::{observation, ReceiptLedgerActor};
use crate::receipt_store::{
    arm_receipt_row_directory_sync_fault, canonical_staged_transfer_certificate, ReceiptLedgerStore,
};
use crate::task_store::{FileInvocationStoreV5, PublicationFailure};
use authority::{ReceiptAuthorityLock, ReceiptStateRoot};
use hooks::{
    V5AdmissionRejection, V5PausePoint, V5ReceiptRuntimeEventKind, V5RetirementStep,
    V5RuntimeHooks, V5Stage, V5StoreFaultPoint,
};
use joining::{
    tool_is_joinable, HolderCancel, HolderProjection, HolderReplyContext, HolderReplyProjection,
    JoinHolder, JoinRegistry, ReceiptHandles, SubmitFences,
};
use ports::{
    BoundInvocation, DecodedSubmit, InvocationHost, InvocationRequestFacts, PreparedInvocation,
    ReceiptRuntimeConfig, V5CanonicalPrepareError,
};
use response::{V5AcknowledgedReceipt, V5InvocationPhase, V5InvocationResponse};
use retirement_facts::{V5TaskProjectionFailure, V5TaskRetirementAuthorization};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};
use unica_refusal::RefusalCode;
use unica_work_control::cancellation::{CancellationToken, FactualOutcomeAuthority};
use unica_workspace::state_fs::StateDirectory;
use uuid::Uuid;

/// The lifecycle values the transport wraps once in its existing outer frame.
pub enum ReceiptResponse {
    Invocation {
        outcome: V5InvocationResponse,
    },
    Task {
        snapshot: response::V5DaemonTaskSnapshot,
    },
    InvocationAcknowledged {
        acknowledgement: V5AcknowledgedReceipt,
    },
}

const AUTHORITY_ACQUIRE_TIMEOUT: Duration = Duration::from_secs(2);

const STARTUP_RECONCILIATION_TIMEOUT: Duration = Duration::from_secs(30);

const TASK_TERMINAL_PUBLICATION_TIMEOUT: Duration = Duration::from_secs(30);

/// The session handler owning an inline invocation re-reads its cutoff at
/// least this often while the worker runs; the worker wakes it earlier.
const INLINE_CUTOFF_POLL_INTERVAL: Duration = Duration::from_millis(5);

/// The durable handoff committed at the cutoff runs under its own bound: the
/// operation budget of the submit is spent by definition at that moment.
const CUTOFF_HANDOFF_COMMIT_BUDGET: Duration = TASK_RECONCILIATION_BUDGET;

/// The least time a request gives the receipt store for its own steps. A
/// caller budget shorter than one durable commit narrows only the wait for
/// the answer: a write the store already began is never declared uncertain,
/// and the daemon never fail-stops, because the caller was in a hurry (#1387).
pub const DURABLE_STEP_BUDGET: Duration = TASK_RECONCILIATION_BUDGET;

/// The bound of a durable step that carries out a decision already
/// committed: the Task exists and the key has moved to its link, so the step
/// must not fail only because the request, the startup or an earlier step
/// spent the deadline. It gets [`DURABLE_STEP_BUDGET`] from its own start.
fn committed_decision_step_deadline(deadline: Instant) -> Instant {
    Instant::now()
        .checked_add(DURABLE_STEP_BUDGET)
        .map_or(deadline, |floor| deadline.max(floor))
}

const V5_TASK_POLL_INTERVAL_MS: u64 = 100;

static NEXT_RETIREMENT_PROCESS_GENERATION: AtomicU64 = AtomicU64::new(1);

/// Pause before an expired Task whose retirement failed is tried again. Each
/// further failure of the same record doubles it up to the maximum, so a
/// record that cannot be retired costs one attempt per pause, not one per poll.
const RETIREMENT_RETRY_INITIAL_DELAY_MS: u64 = 60_000;

const RETIREMENT_RETRY_MAX_DELAY_MS: u64 = 3_600_000;

pub struct V5ReceiptRuntime<H: InvocationHost> {
    core_identity_digest: CoreIdentityDigest,
    // On healthy shutdown Rust drops fields in declaration order: the actor
    // joins and releases its store before named authority is released. A
    // fail-stopped runtime is retained until process death instead.
    receipt_ledger: ReceiptLedgerActor,
    #[cfg(any(test, feature = "test-support"))]
    receipt_owner_identity_for_test: unica_workspace::state_fs::FileIdentity,
    _stable_authority: ReceiptAuthorityLock,
    epoch_clock: Arc<dyn EpochMillisClock>,
    invocation_executor: Arc<H>,
    task_projection: V5TaskProjection,
    active_task_cancellations: Arc<V5ActiveTaskCancellations>,
    task_execution_threads: Mutex<Vec<thread::JoinHandle<()>>>,
    task_terminal_coordinator: Mutex<()>,
    external_store_fail_stop: AtomicBool,
    /// How many promoted attempts the owner handed to a worker are still
    /// running their continuation: the contract harness waits on this to
    /// observe a promoted Task the runtime finishes off-thread.
    promoted_continuations: AtomicUsize,
    /// Who observes this runtime and what it injects; production installs
    /// `NoHooks`.
    hooks: Arc<dyn V5RuntimeHooks<H>>,
    /// Identical reads joined to one running Task (#1376); memory only.
    joins: JoinRegistry,
    /// Receipts not (yet) found as a Task, by their reserved Task id: the id
    /// a lost-submit refusal names must answer while the call is still
    /// inline or after it ended directly (#1376); memory only.
    receipt_handles: ReceiptHandles,
    /// Lost-submit frames this process must never run (#1376); memory only,
    /// like the frames themselves.
    submit_fences: SubmitFences,
    /// Task ids whose attempt runs in this process, while its pipeline
    /// worker runs: such a Task is ended by its attempt, not
    /// by a cancel that names it (#1399).
    live_attempts: V5LiveAttempts,
}

/// The Task ids of the attempts this process is running.
#[derive(Default)]
struct V5LiveAttempts {
    attempts: Mutex<HashMap<unica_execution_contract::invocation::TaskId, usize>>,
}

impl V5LiveAttempts {
    fn lock(&self) -> MutexGuard<'_, HashMap<unica_execution_contract::invocation::TaskId, usize>> {
        self.attempts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn enter(
        &self,
        task_id: unica_execution_contract::invocation::TaskId,
    ) -> V5LiveAttemptGuard<'_> {
        *self.lock().entry(task_id).or_default() += 1;
        V5LiveAttemptGuard {
            attempts: self,
            task_id,
        }
    }

    fn contains(&self, task_id: unica_execution_contract::invocation::TaskId) -> bool {
        self.lock().contains_key(&task_id)
    }
}

struct V5LiveAttemptGuard<'a> {
    attempts: &'a V5LiveAttempts,
    task_id: unica_execution_contract::invocation::TaskId,
}

impl Drop for V5LiveAttemptGuard<'_> {
    fn drop(&mut self) {
        let mut attempts = self.attempts.lock();
        if let Some(count) = attempts.get_mut(&self.task_id) {
            *count -= 1;
            if *count == 0 {
                attempts.remove(&self.task_id);
            }
        }
    }
}

#[derive(Default)]
struct V5ActiveTaskCancellations {
    tokens: Mutex<HashMap<unica_execution_contract::invocation::TaskId, CancellationToken>>,
}

impl V5ActiveTaskCancellations {
    fn register(
        self: &Arc<Self>,
        task_id: unica_execution_contract::invocation::TaskId,
    ) -> Result<(CancellationToken, V5ActiveTaskCancellationGuard), ReceiptLedgerError> {
        let mut tokens = self
            .tokens
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if tokens.contains_key(&task_id) {
            return Err(ReceiptLedgerError::Corrupt(
                "Task already owns an active cancellation token",
            ));
        }
        let token = CancellationToken::new();
        tokens.insert(task_id, token.clone());
        Ok((
            token,
            V5ActiveTaskCancellationGuard {
                registry: Arc::clone(self),
                task_id,
            },
        ))
    }

    fn cancel(&self, task_id: unica_execution_contract::invocation::TaskId) {
        if let Some(token) = self
            .tokens
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&task_id)
            .cloned()
        {
            token.cancel();
        }
    }

    fn outcome_authority(
        &self,
        task_id: unica_execution_contract::invocation::TaskId,
    ) -> FactualOutcomeAuthority {
        self.tokens
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&task_id)
            .map_or(
                FactualOutcomeAuthority::Cancellable,
                CancellationToken::factual_outcome_authority,
            )
    }

    fn is_empty(&self) -> bool {
        self.tokens
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty()
    }

    fn cancel_all(&self) {
        for token in self
            .tokens
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
        {
            token.cancel();
        }
    }
}

struct V5ActiveTaskCancellationGuard {
    registry: Arc<V5ActiveTaskCancellations>,
    task_id: unica_execution_contract::invocation::TaskId,
}

impl Drop for V5ActiveTaskCancellationGuard {
    fn drop(&mut self) {
        self.registry
            .tokens
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.task_id);
    }
}

/// What the pipeline worker hands back to the session handler that owns
/// the reply and the pre-`Begun` cutoff.
#[allow(clippy::large_enum_variant)]
enum PipelineReport {
    Reply(ReceiptReply),
    Failed(ReceiptLedgerError),
}

/// Who answers the request while the worker runs the pipeline.
enum PipelineDecision {
    /// The session handler waits for the worker's reply under the cutoff.
    Waiting,
    /// The cutoff passed before `Begun`: the handler promotes the receipt.
    HandoffInProgress,
    /// The handler took the worker's reply.
    HandlerOwns,
    /// The handler promoted the receipt before `Begun` and answered with its
    /// projection; the worker continues the attempt into that Task and its
    /// own reply is dropped.
    PromotedByOwner,
}

struct PipelineSlotState {
    decision: PipelineDecision,
    report: Option<PipelineReport>,
    /// The worker committed `Begun`: from here the inline drive on the
    /// worker thread owns the cutoff, not the session handler.
    begun: bool,
    /// A Task the session handler materialized at the begun cutoff while the
    /// worker was paused before prepare: the worker executes into it.
    owner_materialized: Option<(V5StoredInvocationRecord, TaskBoundReceipt)>,
}

/// What the worker's drive learns when it claims the cutoff.
#[allow(clippy::large_enum_variant)]
enum CutoffClaim {
    /// The drive owns the cutoff from here.
    Claimed,
    /// The handler promoted the begun receipt first, with the Task it
    /// materialized for this attempt, if it got that far.
    Promoted(Option<(V5StoredInvocationRecord, TaskBoundReceipt)>),
}

/// One attempt shared between the session handler that owns the reply and
/// the pre-`Begun` cutoff and the worker that runs the pipeline.
struct PipelineSlot {
    state: Mutex<PipelineSlotState>,
    changed: Condvar,
}

impl PipelineSlot {
    fn new() -> Self {
        Self {
            state: Mutex::new(PipelineSlotState {
                decision: PipelineDecision::Waiting,
                report: None,
                begun: false,
                owner_materialized: None,
            }),
            changed: Condvar::new(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, PipelineSlotState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The worker's inline drive claims the cutoff before it starts timing:
    /// from here the drive answers the seventh second. When the handler
    /// already promoted the receipt the drive continues that promotion
    /// instead, and takes the Task the handler materialized in the same
    /// step: the handler stores it before it publishes the promotion, so a
    /// worker that looked for it earlier, while the handler was still
    /// materializing, finds it here (#1399).
    fn claim_cutoff(&self) -> CutoffClaim {
        let mut state = self.lock();
        while matches!(state.decision, PipelineDecision::HandoffInProgress) {
            state = self
                .changed
                .wait(state)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        if matches!(state.decision, PipelineDecision::PromotedByOwner) {
            return CutoffClaim::Promoted(state.owner_materialized.take());
        }
        state.begun = true;
        self.changed.notify_all();
        CutoffClaim::Claimed
    }

    /// The session handler records the Task it materialized at the begun
    /// cutoff so the paused worker executes into it once released.
    fn stash_owner_materialized(&self, record: V5StoredInvocationRecord, bound: TaskBoundReceipt) {
        self.lock().owner_materialized = Some((record, bound));
    }

    fn take_owner_materialized(&self) -> Option<(V5StoredInvocationRecord, TaskBoundReceipt)> {
        self.lock().owner_materialized.take()
    }

    /// The worker deposits its reply. While the handler promotes the receipt
    /// the worker waits for that decision instead of racing it.
    fn report(&self, report: PipelineReport) {
        let mut state = self.lock();
        while matches!(state.decision, PipelineDecision::HandoffInProgress) {
            state = self
                .changed
                .wait(state)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        state.report = Some(report);
        self.changed.notify_all();
    }

    /// Whether the handler already answered with a promotion; a worker whose
    /// durable transition lost the race asks this before re-reading.
    fn promoted_by_owner(&self) -> bool {
        let mut state = self.lock();
        while matches!(state.decision, PipelineDecision::HandoffInProgress) {
            state = self
                .changed
                .wait(state)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        matches!(state.decision, PipelineDecision::PromotedByOwner)
    }
}

fn task_outcome_after_cancel(
    candidate: ReceiptTerminalOutcome,
    cancel_requested: bool,
    authority: FactualOutcomeAuthority,
) -> ReceiptTerminalOutcome {
    if cancel_requested && authority == FactualOutcomeAuthority::Cancellable {
        ReceiptTerminalOutcome::Cancelled
    } else {
        candidate
    }
}

/// What the inline worker hands back to the session handler.
enum InlineWorkerReport<P> {
    Rejected(Box<unica_execution_contract::invocation::DomainResult>),
    KnownLong(P),
    Outcome(ReceiptTerminalOutcome),
}

/// Who delivers the outcome of an inline invocation once the worker has it.
enum InlineHandoffDecision {
    /// The session handler waits for the worker under the cutoff.
    Waiting,
    /// The cutoff passed: the handler commits the durable handoff.
    HandoffInProgress,
    /// The handler took the report itself; the worker has nothing to publish.
    HandlerOwns,
    /// The invocation is a durable Task: whoever holds the outcome publishes
    /// it there, under this cancellation registration.
    Task(Box<InlineTaskOwnership>),
    /// The handoff failed after the cutoff: no reply can carry the outcome and
    /// the receipt is reconciled like any interrupted attempt.
    Abandoned,
}

/// The Task an inline attempt continues into after its cutoff.
struct InlineTaskOwnership {
    bound: TaskBoundReceipt,
    task_id: unica_execution_contract::invocation::TaskId,
    guard: Option<V5ActiveTaskCancellationGuard>,
}

struct InlineSlotState<P> {
    decision: InlineHandoffDecision,
    report: Option<InlineWorkerReport<P>>,
}

/// One inline invocation shared between the session handler that owns the
/// cutoff and the worker that runs prepare and execute.
struct InlineExecutionSlot<P> {
    state: Mutex<InlineSlotState<P>>,
    changed: Condvar,
}

impl<P> InlineExecutionSlot<P> {
    fn new() -> Self {
        Self {
            state: Mutex::new(InlineSlotState {
                decision: InlineHandoffDecision::Waiting,
                report: None,
            }),
            changed: Condvar::new(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, InlineSlotState<P>> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The worker deposits its report and learns who publishes it. While the
    /// handler commits a handoff the worker waits for that decision instead of
    /// racing it.
    fn settle(&self, report: InlineWorkerReport<P>) -> InlineSettlement<P> {
        let mut state = self.lock();
        state.report = Some(report);
        self.changed.notify_all();
        loop {
            match &mut state.decision {
                InlineHandoffDecision::Waiting | InlineHandoffDecision::HandlerOwns => {
                    return InlineSettlement::HandlerOwns;
                }
                InlineHandoffDecision::Abandoned => return InlineSettlement::Abandoned,
                InlineHandoffDecision::Task(ownership) => {
                    let ownership = Box::new(InlineTaskOwnership {
                        bound: ownership.bound.clone(),
                        task_id: ownership.task_id,
                        guard: ownership.guard.take(),
                    });
                    let report = state
                        .report
                        .take()
                        .expect("a settled report stays in the slot until claimed");
                    return InlineSettlement::Task { report, ownership };
                }
                InlineHandoffDecision::HandoffInProgress => {
                    state = self
                        .changed
                        .wait(state)
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                }
            }
        }
    }
}

enum InlineSettlement<P> {
    HandlerOwns,
    Abandoned,
    Task {
        report: InlineWorkerReport<P>,
        ownership: Box<InlineTaskOwnership>,
    },
}

/// What the cutoff commit produced: the Task the running attempt continues
/// into, or its terminal when the outcome was already there.
#[allow(clippy::large_enum_variant)]
enum CutoffCommit {
    Bound(TaskBoundReceipt, V5StoredInvocationRecord),
    Terminal(ReceiptReply),
}

/// What a committed handoff became once its key moved to the Task link.
#[allow(clippy::large_enum_variant)]
enum HandoffCompletion {
    /// The Task is bound; the attempt continues into it.
    Bound(V5StoredInvocationRecord, TaskBoundReceipt),
    /// The handoff carried a staged terminal, which the Task now holds.
    Terminal(V5StoredInvocationRecord),
}

/// The reply that names a Task by its current projection.
fn task_reply(record: &V5StoredInvocationRecord) -> ReceiptReply {
    ReceiptReply::Json(ReceiptResponse::Invocation {
        outcome: V5InvocationResponse::Task {
            snapshot: task_store_snapshot(record),
        },
    })
}

/// How the session handler continues after the inline worker settled or the
/// cutoff took the invocation away from it.
enum InlineDrive<P> {
    Rejected(Box<unica_execution_contract::invocation::DomainResult>),
    KnownLong(P),
    Direct(ReceiptTerminalOutcome),
    HandedOff(Box<ReceiptReply>),
}

struct V5TaskProjection {
    #[allow(dead_code)]
    task_store_root: StateDirectory,
    #[allow(dead_code)]
    task_store: Arc<FileInvocationStoreV5>,
    #[allow(dead_code)]
    lifecycle_link_root: StateDirectory,
    lifecycle_links: TaskLifecycleLinkStoreV5,
    recovery: TaskStoreRecoveryCatalog,
    epoch_clock: Arc<dyn EpochMillisClock>,
    retirement_process_instance_id: Uuid,
    retirement_process_generation: u64,
    retirement_issued_sequence: AtomicU64,
    /// Serializes retirement inside the process and remembers the expired
    /// records whose last retirement attempt failed.
    retirement_coordinator:
        Mutex<HashMap<unica_execution_contract::invocation::TaskId, RetirementDeferral>>,
}

/// An expired terminal Task whose retirement failed: it keeps its committed
/// state and is tried again after the pause.
struct RetirementDeferral {
    failures: u32,
    retry_at_epoch_ms: u64,
}

fn retirement_retry_delay_ms(failures: u32) -> u64 {
    let doublings = failures.saturating_sub(1).min(16);
    RETIREMENT_RETRY_INITIAL_DELAY_MS
        .saturating_mul(1_u64 << doublings)
        .min(RETIREMENT_RETRY_MAX_DELAY_MS)
}

/// An expired terminal Task the retirement pass found: either still
/// terminal-bound, or with an intent to retire committed by an earlier pass.
enum RetirementCandidate {
    Expired(TaskTerminalBoundReceipt),
    Pending(TaskRetirementPendingReceipt),
}

impl RetirementCandidate {
    fn task_id(&self) -> unica_execution_contract::invocation::TaskId {
        match self {
            Self::Expired(terminal) => terminal.task().task_id(),
            Self::Pending(pending) => pending.task().task_id(),
        }
    }
}

enum StagedHandoffStartupPlan {
    TaskBound {
        bound: TaskBoundReceipt,
        provisional: V5StoredInvocationRecord,
        observed: V5StoredInvocationRecord,
    },
    TaskTerminalBound {
        terminal_link: TaskTerminalBoundReceipt,
        observed: V5StoredInvocationRecord,
    },
}

impl StagedHandoffStartupPlan {
    fn observed(&self) -> &V5StoredInvocationRecord {
        match self {
            Self::TaskBound { observed, .. } | Self::TaskTerminalBound { observed, .. } => observed,
        }
    }
}

struct StartupTaskTerminalizationPlan {
    expected: TaskBoundReceipt,
    record: V5StoredInvocationRecord,
    reason: Option<RecoveryTerminalReason>,
}

impl V5TaskProjection {
    fn open(
        state: &dyn ReceiptStateRoot,
        epoch_clock: Arc<dyn EpochMillisClock>,
        deadline: Instant,
    ) -> Result<Self, String> {
        let task_store_root = state.create_private_retained_subdirectory("tasks")?;
        let (store, recovery) = FileInvocationStoreV5::open_retained_directory_inspect_only(
            task_store_root.clone(),
            Arc::clone(&epoch_clock),
            unica_work_control::provider_deadline::ProviderDeadline::new(deadline),
        )
        .map_err(|error| format!("open inspect-only protocol-v5 task store: {error}"))?;
        let lifecycle_link_root =
            state.create_private_retained_subdirectory("task-lifecycle-links")?;
        let lifecycle_links = TaskLifecycleLinkStoreV5::open(
            lifecycle_link_root.path(),
            unica_work_control::provider_deadline::ProviderDeadline::new(deadline),
        )
        .map_err(|error| format!("open protocol-v5 Task lifecycle-link store: {error}"))?;
        Ok(Self {
            task_store_root,
            task_store: Arc::new(store),
            lifecycle_link_root,
            lifecycle_links,
            recovery,
            epoch_clock,
            retirement_process_instance_id: Uuid::new_v4(),
            retirement_process_generation: NEXT_RETIREMENT_PROCESS_GENERATION
                .fetch_add(1, Ordering::AcqRel),
            retirement_issued_sequence: AtomicU64::new(1),
            retirement_coordinator: Mutex::new(HashMap::new()),
        })
    }

    fn authorize_task_retirement(
        &self,
        expected: &TaskRetirementPendingReceipt,
        deadline: Instant,
    ) -> Result<V5TaskRetirementAuthorization, V5TaskProjectionFailure> {
        let provider_deadline =
            unica_work_control::provider_deadline::ProviderDeadline::new(deadline);
        let _snapshot = self
            .lifecycle_links
            .catalog_snapshot(provider_deadline)
            .map_err(V5TaskProjectionFailure::from_link_store)?;
        let current = self
            .lifecycle_links
            .read_by_task_id(expected.task().task_id(), provider_deadline)
            .map_err(V5TaskProjectionFailure::from_link_store)?;
        if current != TaskLifecycleLinkRecord::TaskRetirementPending(expected.clone()) {
            return Err(V5TaskProjectionFailure::fail_stop(
                ReceiptLedgerError::Corrupt(
                    "retirement authorization requires exact committed TaskRetirementPending",
                ),
            ));
        }
        let pending_bytes = canonical_task_retirement_pending_bytes(expected)?;
        let pending_record_sha256 = lower_hex_digest(&Sha256::digest(&pending_bytes));
        let issued_sequence = self
            .retirement_issued_sequence
            .fetch_add(1, Ordering::AcqRel);
        let mut authorization_preimage = Vec::new();
        authorization_preimage.extend_from_slice(self.retirement_process_instance_id.as_bytes());
        authorization_preimage.extend_from_slice(&self.retirement_process_generation.to_be_bytes());
        authorization_preimage.extend_from_slice(&issued_sequence.to_be_bytes());
        authorization_preimage.extend_from_slice(pending_record_sha256.as_bytes());
        let authorization_fingerprint = lower_hex_digest(&Sha256::digest(&authorization_preimage));
        Ok(V5TaskRetirementAuthorization {
            process_instance_id: self.retirement_process_instance_id,
            generation: self.retirement_process_generation,
            issued_sequence,
            pending_record_sha256,
            authorization_fingerprint,
            pending: expected.clone(),
        })
    }

    fn delete_terminal_authorized(
        &self,
        authorization: &V5TaskRetirementAuthorization,
        observed_at_epoch_ms: u64,
        deadline: Instant,
    ) -> Result<V5DeleteTerminalOutcome, V5TaskProjectionFailure> {
        self.validate_retirement_authorization(authorization)?;
        let provider_deadline =
            unica_work_control::provider_deadline::ProviderDeadline::new(deadline);
        let current = self
            .lifecycle_links
            .read_by_task_id(authorization.pending.task().task_id(), provider_deadline)
            .map_err(V5TaskProjectionFailure::from_link_store)?;
        if current != TaskLifecycleLinkRecord::TaskRetirementPending(authorization.pending.clone())
        {
            return Err(V5TaskProjectionFailure::fail_stop(
                ReceiptLedgerError::Corrupt(
                    "retirement capability no longer names the exact pending record",
                ),
            ));
        }
        let (record, task_absent) = match self
            .task_store
            .get(authorization.pending.task().task_id(), provider_deadline)
        {
            Ok(record) => (record, false),
            Err(V5TaskStoreError::NotFound { .. }) => (
                terminal_record_from_retirement_pending(&authorization.pending),
                true,
            ),
            Err(error) => {
                return Err(V5TaskProjectionFailure::from_task_store(
                    error,
                    authorization.pending.key_digest().clone(),
                    true,
                ))
            }
        };
        if !task_absent && !task_retirement_pending_matches_record(&authorization.pending, &record)?
        {
            return Err(V5TaskProjectionFailure::fail_stop(
                ReceiptLedgerError::Corrupt(
                    "retirement capability does not bind the exact terminal Task",
                ),
            ));
        }
        let retirement = V5TaskRetirement::from_terminal_record(&record).ok_or_else(|| {
            V5TaskProjectionFailure::fail_stop(ReceiptLedgerError::Corrupt(
                "retirement target is not terminal",
            ))
        })?;
        self.task_store
            .delete_terminal_if_expired(&retirement, observed_at_epoch_ms, provider_deadline)
            .map_err(|error| {
                V5TaskProjectionFailure::from_task_store(
                    error,
                    authorization.pending.key_digest().clone(),
                    true,
                )
            })
    }

    fn finalize_task_retirement(
        &self,
        authorization: &V5TaskRetirementAuthorization,
        deadline: Instant,
    ) -> Result<(), V5TaskProjectionFailure> {
        self.validate_retirement_authorization(authorization)?;
        self.lifecycle_links
            .finalize_task_retirement(
                &authorization.pending,
                unica_work_control::provider_deadline::ProviderDeadline::new(deadline),
            )
            .map_err(V5TaskProjectionFailure::from_link_store)
    }

    fn validate_retirement_authorization(
        &self,
        authorization: &V5TaskRetirementAuthorization,
    ) -> Result<(), V5TaskProjectionFailure> {
        let pending_bytes = canonical_task_retirement_pending_bytes(&authorization.pending)?;
        let pending_record_sha256 = lower_hex_digest(&Sha256::digest(&pending_bytes));
        let mut preimage = Vec::new();
        preimage.extend_from_slice(authorization.process_instance_id.as_bytes());
        preimage.extend_from_slice(&authorization.generation.to_be_bytes());
        preimage.extend_from_slice(&authorization.issued_sequence.to_be_bytes());
        preimage.extend_from_slice(pending_record_sha256.as_bytes());
        let fingerprint = lower_hex_digest(&Sha256::digest(&preimage));
        if authorization.process_instance_id != self.retirement_process_instance_id
            || authorization.generation != self.retirement_process_generation
            || authorization.issued_sequence == 0
            || authorization.pending_record_sha256 != pending_record_sha256
            || authorization.authorization_fingerprint != fingerprint
        {
            return Err(V5TaskProjectionFailure::fail_stop(
                ReceiptLedgerError::Corrupt("stale or malformed process retirement capability"),
            ));
        }
        Ok(())
    }

    fn reconcile_materialized_startup(
        &self,
        deadline: Instant,
    ) -> Result<(), V5TaskProjectionFailure> {
        let provider_deadline =
            unica_work_control::provider_deadline::ProviderDeadline::new(deadline);
        let lifecycle = self
            .lifecycle_links
            .catalog_snapshot(provider_deadline)
            .map_err(V5TaskProjectionFailure::from_link_store)?;
        let mut plans = Vec::new();
        let mut recovered_records = HashMap::new();

        // Re-read every Task from the inspect-only startup catalog after the
        // receipt loop: that loop may have completed an exact handoff and
        // legitimately advanced the Task and lifecycle link. Keep that exact
        // read for the lifecycle pass below: the bounded full pool must not
        // deserialize and validate every Task twice during startup.
        for recovery in self.recovery.entries() {
            let record = self
                .task_store
                .get(recovery.identity().task_id(), provider_deadline)
                .map_err(|error| {
                    V5TaskProjectionFailure::from_task_store(
                        error,
                        recovery.identity().receipt_key_digest().clone(),
                        true,
                    )
                })?;
            if !recovery.identity().matches_record(&record) {
                return Err(Self::startup_fail_stop(
                    "TaskStore recovery identity changed before startup reconciliation",
                ));
            }
            if recovered_records
                .insert(recovery.identity().task_id(), record)
                .is_some()
            {
                return Err(Self::startup_fail_stop(
                    "TaskStore recovery catalog contains a duplicate Task identity",
                ));
            }
        }

        // Receipt-driven reconciliation can materialize rows that were absent
        // from the initial TaskStore preimage. Validate those links too, and
        // require terminal lifecycle evidence to match the exact terminal Task.
        let mut linked_task_ids = HashSet::new();
        for entry in lifecycle.entries() {
            let TaskLifecycleLinkCatalogEntry::Record(link) = entry else {
                continue;
            };
            let task_id = lifecycle_entry_task_id(entry);
            if !linked_task_ids.insert(task_id) {
                return Err(Self::startup_fail_stop(
                    "Task has more than one lifecycle link",
                ));
            }
            let receipt_key_digest = match link {
                TaskLifecycleLinkRecord::TaskBound(expected) => expected.key_digest(),
                TaskLifecycleLinkRecord::TaskTerminalBound(expected) => expected.key_digest(),
                TaskLifecycleLinkRecord::TaskRetirementPending(expected) => expected.key_digest(),
            };
            let record = match recovered_records.remove(&task_id) {
                Some(record) => record,
                None => match self.task_store.get(task_id, provider_deadline) {
                    Ok(record) => record,
                    Err(V5TaskStoreError::NotFound { .. })
                        if matches!(link, TaskLifecycleLinkRecord::TaskRetirementPending(_)) =>
                    {
                        // A committed Pending is the sole authority that permits
                        // an already-absent terminal Task. The successor process
                        // must freshly authorize that exact pending and finalize
                        // its link/indexes; absence is not active-Task corruption.
                        continue;
                    }
                    Err(error) => {
                        return Err(V5TaskProjectionFailure::from_task_store(
                            error,
                            receipt_key_digest.clone(),
                            true,
                        ))
                    }
                },
            };
            match link {
                TaskLifecycleLinkRecord::TaskBound(expected) => {
                    plans.push(self.preflight_task_bound_startup(expected, record)?);
                }
                TaskLifecycleLinkRecord::TaskTerminalBound(expected) => {
                    if !task_terminal_bound_matches_record(expected, &record)? {
                        return Err(Self::startup_fail_stop(
                            "TaskTerminalBound does not confirm the exact terminal Task",
                        ));
                    }
                }
                TaskLifecycleLinkRecord::TaskRetirementPending(expected) => {
                    if !task_retirement_pending_matches_record(expected, &record)? {
                        return Err(Self::startup_fail_stop(
                            "TaskRetirementPending does not confirm the exact terminal Task",
                        ));
                    }
                }
            }
        }

        if !recovered_records.is_empty() {
            return Err(Self::startup_fail_stop(
                "TaskStore Task has no exact lifecycle link",
            ));
        }

        for plan in plans {
            match plan.reason {
                Some(reason) => {
                    self.terminalize_recovered_bound(
                        &plan.expected,
                        plan.record,
                        reason,
                        deadline,
                    )?;
                }
                None => {
                    self.complete_recovered_terminal_bound(&plan.expected, plan.record, deadline)?;
                }
            }
        }
        Ok(())
    }

    /// Retires the expired terminal Tasks before a Task read or cancel.
    ///
    /// Retirement is housekeeping, not a precondition of the read: a record
    /// that cannot be retired keeps its committed state, is reported on stderr
    /// and waits for its retry pause, while the pass goes on with the other
    /// records and the caller answers its own Task. Only a failure of the
    /// stores themselves — the lifecycle-link catalog cannot be read back or a
    /// store root lost its authority — is returned, as before.
    fn retire_expired_terminal_tasks<H: InvocationHost>(
        &self,
        deadline: Instant,
        hooks: &dyn V5RuntimeHooks<H>,
    ) -> Result<(), V5TaskProjectionFailure> {
        hooks
            .pause(V5PausePoint::BeforeRetirementSnapshot, deadline)
            .map_err(V5TaskProjectionFailure::fail_stop)?;
        // One process owns the receipt authority, and this coordinator makes
        // terminal retirement a single-owner operation inside that process.
        // A concurrent observation waits and then takes a fresh catalog snapshot
        // instead of treating an ordinary optimistic-version race as corruption.
        let mut deferrals = self.retirement_coordinator.lock().map_err(|_| {
            V5TaskProjectionFailure::fail_stop(ReceiptLedgerError::Corrupt(
                "protocol-v5 task retirement coordinator is poisoned",
            ))
        })?;
        if Instant::now() >= deadline {
            return Ok(());
        }
        let provider_deadline =
            unica_work_control::provider_deadline::ProviderDeadline::new(deadline);
        let observed_at_epoch_ms = self.epoch_clock.now_epoch_millis();
        let snapshot = match self.lifecycle_links.catalog_snapshot(provider_deadline) {
            Ok(snapshot) => snapshot,
            Err(TaskLifecycleLinkStoreError::DeadlineExceeded) => return Ok(()),
            Err(error) => return Err(V5TaskProjectionFailure::from_link_store(error)),
        };
        let mut candidates = Vec::new();
        for entry in snapshot.entries() {
            let TaskLifecycleLinkCatalogEntry::Record(record) = entry else {
                continue;
            };
            match record {
                TaskLifecycleLinkRecord::TaskTerminalBound(terminal)
                    if observed_at_epoch_ms >= terminal.expires_at_epoch_ms() =>
                {
                    candidates.push(RetirementCandidate::Expired(terminal.clone()));
                }
                TaskLifecycleLinkRecord::TaskRetirementPending(existing) => {
                    candidates.push(RetirementCandidate::Pending(existing.clone()));
                }
                TaskLifecycleLinkRecord::TaskBound(_)
                | TaskLifecycleLinkRecord::TaskTerminalBound(_) => {}
            }
        }
        // A record that left the catalog no longer needs its retry pause.
        let awaiting = candidates
            .iter()
            .map(RetirementCandidate::task_id)
            .collect::<HashSet<_>>();
        deferrals.retain(|task_id, _| awaiting.contains(task_id));
        for candidate in candidates {
            let task_id = candidate.task_id();
            if deferrals
                .get(&task_id)
                .is_some_and(|deferral| observed_at_epoch_ms < deferral.retry_at_epoch_ms)
            {
                continue;
            }
            let failure = match self.retire_expired_terminal_task(
                candidate,
                observed_at_epoch_ms,
                deadline,
                provider_deadline,
                hooks,
            ) {
                Ok(()) => {
                    deferrals.remove(&task_id);
                    continue;
                }
                // The read spent its budget before this step changed
                // anything: the remaining records wait for the next read.
                Err(failure)
                    if !failure.fail_stop
                        && matches!(failure.error, ReceiptLedgerError::DeadlineExceeded) =>
                {
                    return Ok(());
                }
                Err(failure) => failure,
            };
            // A store failure may have left the durable link catalog ahead of
            // the copy in memory (its commit is uncertain). Adopt the durable
            // catalog before any other Task is served from it. A rejection
            // without a store failure (deadline, record size) changed nothing.
            if failure.fail_stop {
                self.reload_after_retirement_failure()?;
            }
            let failures = deferrals
                .get(&task_id)
                .map_or(1, |deferral| deferral.failures.saturating_add(1));
            let delay_ms = retirement_retry_delay_ms(failures);
            deferrals.insert(
                task_id,
                RetirementDeferral {
                    failures,
                    retry_at_epoch_ms: observed_at_epoch_ms.saturating_add(delay_ms),
                },
            );
            eprintln!(
                "protocol-v5 retirement of expired Task {task_id} failed ({}); \
                 the Task keeps its committed state, next attempt in {delay_ms} ms",
                failure.error
            );
        }
        Ok(())
    }

    /// Runs the four retirement steps for one expired terminal Task, resuming
    /// at authorization when an earlier pass already committed the intent.
    fn retire_expired_terminal_task<H: InvocationHost>(
        &self,
        candidate: RetirementCandidate,
        observed_at_epoch_ms: u64,
        deadline: Instant,
        provider_deadline: unica_work_control::provider_deadline::ProviderDeadline,
        hooks: &dyn V5RuntimeHooks<H>,
    ) -> Result<(), V5TaskProjectionFailure> {
        let task_id = candidate.task_id();
        let injected = |step| {
            if hooks.retirement_step_fault(step, task_id) {
                Err(V5TaskProjectionFailure::fail_stop(
                    ReceiptLedgerError::StoreUnavailable,
                ))
            } else {
                Ok(())
            }
        };
        let pending = match candidate {
            RetirementCandidate::Expired(terminal) => {
                injected(V5RetirementStep::Begin)?;
                self.lifecycle_links
                    .begin_task_retirement(&terminal, 64, 64, provider_deadline)
                    .map_err(V5TaskProjectionFailure::from_link_store)?
            }
            RetirementCandidate::Pending(pending) => pending,
        };
        injected(V5RetirementStep::Authorize)?;
        let authorization = self.authorize_task_retirement(&pending, deadline)?;
        injected(V5RetirementStep::Delete)?;
        self.delete_terminal_authorized(&authorization, observed_at_epoch_ms, deadline)?;
        injected(V5RetirementStep::Finalize)?;
        self.finalize_task_retirement(&authorization, deadline)
    }

    /// After a failed retirement step the process keeps serving other Tasks
    /// only from durable state: the link catalog is re-read from disk and both
    /// store roots must still be the ones this process owns. If either check
    /// fails, the stores themselves are unusable and the process fail-stops.
    fn reload_after_retirement_failure(&self) -> Result<(), V5TaskProjectionFailure> {
        let unbounded = unica_work_control::provider_deadline::ProviderDeadline::no_deadline();
        self.lifecycle_links
            .reload_durable_catalog(unbounded)
            .map_err(|_| {
                V5TaskProjectionFailure::fail_stop(ReceiptLedgerError::StoreUnavailable)
            })?;
        self.task_store
            .verify_store_authority()
            .map_err(|_| V5TaskProjectionFailure::fail_stop(ReceiptLedgerError::StoreUnavailable))
    }

    fn preflight_task_bound_startup(
        &self,
        expected: &TaskBoundReceipt,
        record: V5StoredInvocationRecord,
    ) -> Result<StartupTaskTerminalizationPlan, V5TaskProjectionFailure> {
        if !task_bound_matches_record(expected, &record)? {
            return Err(Self::startup_fail_stop(
                "TaskBound does not authorize the exact active Task",
            ));
        }
        if matches!(
            record.task,
            V5StoredTask::Completed { .. }
                | V5StoredTask::Failed { .. }
                | V5StoredTask::Cancelled { .. }
        ) {
            return Ok(StartupTaskTerminalizationPlan {
                expected: expected.clone(),
                record,
                reason: None,
            });
        }
        let reason = match expected.phase() {
            AttemptPhase::NotBegun if record.cancel_requested => RecoveryTerminalReason::Cancelled,
            AttemptPhase::NotBegun => RecoveryTerminalReason::InterruptedBeforeExecution,
            AttemptPhase::Begun if record.task == V5StoredTask::Working => {
                RecoveryTerminalReason::OutcomeUncertain
            }
            AttemptPhase::Begun => {
                return Err(Self::startup_fail_stop(
                    "TaskBound Begun requires exact Working Task",
                ))
            }
        };
        Ok(StartupTaskTerminalizationPlan {
            expected: expected.clone(),
            record,
            reason: Some(reason),
        })
    }

    fn staged_handoff_startup_plan(
        &self,
        staged: &TaskHandoffActorBoundReceipt,
        catalog: &[TaskLifecycleLinkCatalogEntry],
        deadline: Instant,
    ) -> Result<Option<StagedHandoffStartupPlan>, V5TaskProjectionFailure> {
        let HandoffTerminalStage::Staged {
            terminal,
            terminal_epoch_ms,
            certificate,
        } = staged.terminal_stage()
        else {
            return Err(Self::startup_fail_stop(
                "startup handoff has no saved terminal",
            ));
        };
        if !certificate.matches_staged_terminal(
            staged.key(),
            staged.key_digest(),
            staged.link(),
            *terminal_epoch_ms,
            terminal,
        ) {
            return Err(Self::startup_fail_stop(
                "startup handoff terminal certificate differs",
            ));
        }
        let matching = catalog
            .iter()
            .filter(|entry| lifecycle_entry_task_id(entry) == staged.task().task_id())
            .collect::<Vec<_>>();
        let entry = match matching.as_slice() {
            [] => return Ok(None),
            [TaskLifecycleLinkCatalogEntry::Reservation(reservation)] => {
                if reservation.key() != staged.key() || reservation.link() != staged.link() {
                    return Err(Self::startup_fail_stop(
                        "staged startup reservation differs from its receipt",
                    ));
                }
                return Ok(None);
            }
            [TaskLifecycleLinkCatalogEntry::Record(record)] => record,
            _ => {
                return Err(Self::startup_fail_stop(
                    "staged startup handoff has conflicting lifecycle owners",
                ))
            }
        };
        let durable_link = match entry {
            TaskLifecycleLinkRecord::TaskBound(record) => record.link(),
            TaskLifecycleLinkRecord::TaskTerminalBound(record) => record.link(),
            TaskLifecycleLinkRecord::TaskRetirementPending(record) => record.link(),
        };
        if entry.key() != staged.key() || durable_link != staged.link() {
            return Err(Self::startup_fail_stop(
                "staged startup lifecycle link differs from its receipt",
            ));
        }
        let observed = self
            .task_store
            .get(
                staged.task().task_id(),
                unica_work_control::provider_deadline::ProviderDeadline::new(deadline),
            )
            .map_err(|error| {
                V5TaskProjectionFailure::from_task_store(error, staged.key_digest().clone(), true)
            })?;
        let (publication, status) = Self::terminal_publication(terminal, *terminal_epoch_ms);
        match entry {
            TaskLifecycleLinkRecord::TaskBound(bound) => {
                if bound.key_digest() != staged.key_digest() || bound.phase() != staged.phase() {
                    return Err(Self::startup_fail_stop(
                        "staged startup TaskBound authority differs",
                    ));
                }
                let provisional_task = match staged.phase() {
                    AttemptPhase::NotBegun => V5StoredTask::Queued,
                    AttemptPhase::Begun => V5StoredTask::Working,
                };
                let provisional =
                    staged_record_from_durable_projection(staged, bound.task(), provisional_task)?;
                if provisional.version != bound.task_record_version()
                    || *terminal_epoch_ms < provisional.updated_at_epoch_ms
                {
                    return Err(Self::startup_fail_stop(
                        "staged startup successor version or epoch differs",
                    ));
                }
                let mut successor = provisional.clone();
                successor.version = provisional.version.checked_add(1).ok_or_else(|| {
                    Self::startup_fail_stop("staged startup Task version overflowed")
                })?;
                successor.updated_at_epoch_ms = *terminal_epoch_ms;
                successor.task = publication.into_stored_task();
                if observed != provisional && observed != successor {
                    return Err(Self::startup_fail_stop(
                        "staged startup Task differs from its full durable successor",
                    ));
                }
                Ok(Some(StagedHandoffStartupPlan::TaskBound {
                    bound: bound.clone(),
                    provisional,
                    observed,
                }))
            }
            TaskLifecycleLinkRecord::TaskTerminalBound(terminal_link) => {
                let expected = staged_record_from_durable_projection(
                    staged,
                    terminal_link.task(),
                    publication.into_stored_task(),
                )?;
                if terminal_link.key_digest() != staged.key_digest()
                    || terminal_link.terminal_status() != status
                    || terminal_link.terminal_digest() != terminal.digest()
                    || terminal_link.terminal_epoch_ms() != *terminal_epoch_ms
                    || expected.updated_at_epoch_ms != *terminal_epoch_ms
                    || expected != observed
                    || !task_terminal_bound_matches_record(terminal_link, &expected)?
                {
                    return Err(Self::startup_fail_stop(
                        "staged startup terminal link differs from its full saved winner",
                    ));
                }
                Ok(Some(StagedHandoffStartupPlan::TaskTerminalBound {
                    terminal_link: terminal_link.clone(),
                    observed,
                }))
            }
            TaskLifecycleLinkRecord::TaskRetirementPending(_) => Err(Self::startup_fail_stop(
                "active staged handoff has a retiring lifecycle link",
            )),
        }
    }

    fn startup_fail_stop(message: &'static str) -> V5TaskProjectionFailure {
        V5TaskProjectionFailure::fail_stop(ReceiptLedgerError::Corrupt(message))
    }

    #[cfg(any(
        test,
        feature = "test-support",
        feature = "receipt-ledger-test-support"
    ))]
    fn materialize_bound_handoff<H: InvocationHost>(
        &self,
        handoff: &TaskHandoffActorBoundReceipt,
        bind_epoch_ms: u64,
        deadline: Instant,
        hooks: &dyn V5RuntimeHooks<H>,
    ) -> Result<(V5StoredInvocationRecord, TaskBoundReceipt), V5TaskProjectionFailure> {
        self.materialize_actor_bound_task(
            handoff.key(),
            handoff.link(),
            handoff.task(),
            handoff.workspace_identity_hash(),
            handoff.cancel_requested(),
            handoff.phase(),
            handoff.phase() == AttemptPhase::Begun,
            bind_epoch_ms,
            deadline,
            hooks,
        )
    }

    fn reserve_bound_handoff_link<H: InvocationHost>(
        &self,
        handoff: &TaskHandoffActorBoundReceipt,
        _epoch_ms: u64,
        deadline: Instant,
        hooks: &dyn V5RuntimeHooks<H>,
    ) -> Result<TaskLinkReservation, V5TaskProjectionFailure> {
        let reservation = self
            .lifecycle_links
            .reserve_task_link(
                handoff.key().clone(),
                handoff.link().clone(),
                unica_work_control::provider_deadline::ProviderDeadline::new(deadline),
            )
            .map_err(V5TaskProjectionFailure::from_link_store)?;
        hooks.event(
            V5ReceiptRuntimeEventKind::TaskLinkCapacityReserved,
            _epoch_ms,
        );
        Ok(reservation)
    }

    fn materialize_staged_bound_handoff<H: InvocationHost>(
        &self,
        handoff: &TaskHandoffActorBoundReceipt,
        reservation: &TaskLinkReservation,
        bind_epoch_ms: u64,
        deadline: Instant,
        hooks: &dyn V5RuntimeHooks<H>,
    ) -> Result<(V5StoredInvocationRecord, TaskBoundReceipt), V5TaskProjectionFailure> {
        self.materialize_actor_bound_task_from_reservation(
            handoff.key(),
            handoff.task(),
            handoff.workspace_identity_hash(),
            handoff.cancel_requested(),
            handoff.phase(),
            handoff.phase() == AttemptPhase::Begun,
            reservation,
            bind_epoch_ms,
            deadline,
            true,
            hooks,
        )
    }

    fn materialize_recovered_handoff<H: InvocationHost>(
        &self,
        handoff: &TaskHandoffActorBoundReceipt,
        bind_epoch_ms: u64,
        deadline: Instant,
        hooks: &dyn V5RuntimeHooks<H>,
    ) -> Result<(V5StoredInvocationRecord, TaskBoundReceipt), V5TaskProjectionFailure> {
        self.materialize_actor_bound_task(
            handoff.key(),
            handoff.link(),
            handoff.task(),
            handoff.workspace_identity_hash(),
            handoff.cancel_requested(),
            handoff.phase(),
            handoff.phase() == AttemptPhase::Begun,
            bind_epoch_ms,
            deadline,
            hooks,
        )
    }

    fn materialize_promised_actor_bound<H: InvocationHost>(
        &self,
        promised: &TaskPromisedActorBoundReceipt,
        bind_epoch_ms: u64,
        deadline: Instant,
        hooks: &dyn V5RuntimeHooks<H>,
    ) -> Result<(V5StoredInvocationRecord, TaskBoundReceipt), V5TaskProjectionFailure> {
        self.materialize_actor_bound_task(
            promised.key(),
            promised.link(),
            promised.task(),
            promised.workspace_identity_hash(),
            promised.cancel_requested(),
            AttemptPhase::NotBegun,
            false,
            bind_epoch_ms,
            deadline,
            hooks,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn materialize_actor_bound_task<H: InvocationHost>(
        &self,
        key: &ReceiptKey,
        link: &crate::receipt_ledger::TaskLinkReference,
        promised_task: &ReceiptTaskProjection,
        workspace_identity_hash: &unica_execution_contract::invocation::SafeIdentityHash,
        cancel_requested: bool,
        phase: AttemptPhase,
        recovered_begun: bool,
        bind_epoch_ms: u64,
        deadline: Instant,
        hooks: &dyn V5RuntimeHooks<H>,
    ) -> Result<(V5StoredInvocationRecord, TaskBoundReceipt), V5TaskProjectionFailure> {
        let provider_deadline =
            unica_work_control::provider_deadline::ProviderDeadline::new(deadline);
        observation::mark(
            "materialize.reserve_link",
            "enter",
            Some(deadline.saturating_duration_since(Instant::now())),
        );
        let reservation = self
            .lifecycle_links
            .reserve_task_link(key.clone(), link.clone(), provider_deadline)
            .map_err(V5TaskProjectionFailure::from_link_store)?;
        observation::mark(
            "materialize.reserve_link",
            "complete",
            Some(deadline.saturating_duration_since(Instant::now())),
        );
        hooks.event(
            V5ReceiptRuntimeEventKind::TaskLinkCapacityReserved,
            bind_epoch_ms,
        );
        self.materialize_actor_bound_task_from_reservation(
            key,
            promised_task,
            workspace_identity_hash,
            cancel_requested,
            phase,
            recovered_begun,
            &reservation,
            bind_epoch_ms,
            deadline,
            true,
            hooks,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn materialize_actor_bound_task_from_reservation<H: InvocationHost>(
        &self,
        key: &ReceiptKey,
        promised_task: &ReceiptTaskProjection,
        workspace_identity_hash: &unica_execution_contract::invocation::SafeIdentityHash,
        cancel_requested: bool,
        phase: AttemptPhase,
        recovered_begun: bool,
        reservation: &TaskLinkReservation,
        bind_epoch_ms: u64,
        deadline: Instant,
        record_reservation_conversion: bool,
        hooks: &dyn V5RuntimeHooks<H>,
    ) -> Result<(V5StoredInvocationRecord, TaskBoundReceipt), V5TaskProjectionFailure> {
        let provider_deadline =
            unica_work_control::provider_deadline::ProviderDeadline::new(deadline);
        let identity = V5TaskIdentity::new(
            promised_task.task_id(),
            promised_task.invocation_id(),
            crate::receipt_ledger::receipt_key_digest(key),
        );
        let new_record = NewV5InvocationRecord::new(
            identity.clone(),
            key.tool(),
            key.normalized_arguments_hash().clone(),
            workspace_identity_hash.clone(),
            promised_task.poll_interval_ms(),
            promised_task.ttl_ms(),
        )
        .with_initial_epoch_ms(promised_task.created_at_epoch_ms());
        let new_record = if recovered_begun {
            new_record.for_recovered_begun(cancel_requested)
        } else {
            new_record
        };
        hooks.task_store_create_attempted();
        hooks.event(
            V5ReceiptRuntimeEventKind::TaskStoreCreateAttempted,
            bind_epoch_ms,
        );
        if hooks.store_fault(V5StoreFaultPoint::AfterTaskCreateRenameBeforeDirectorySync) {
            self.task_store
                .inject_next_publication_failure(PublicationFailure::AfterRenameBeforeSync);
        }
        observation::mark(
            "materialize.task_create",
            "enter",
            Some(deadline.saturating_duration_since(Instant::now())),
        );
        let created = self
            .task_store
            .create_exact(new_record, provider_deadline)
            .map_err(|error| {
                V5TaskProjectionFailure::from_task_store(
                    error,
                    identity.receipt_key_digest().clone(),
                    true,
                )
            })?;
        observation::mark(
            "materialize.task_create",
            "complete",
            Some(deadline.saturating_duration_since(Instant::now())),
        );
        observation::mark(
            "materialize.task_readback",
            "enter",
            Some(deadline.saturating_duration_since(Instant::now())),
        );
        let mut readback = self
            .task_store
            .get(created.task_id, provider_deadline)
            .map_err(|error| {
                V5TaskProjectionFailure::from_task_store(
                    error,
                    identity.receipt_key_digest().clone(),
                    true,
                )
            })?;
        if readback != created || !identity.matches_record(&readback) {
            return Err(V5TaskProjectionFailure::fail_stop(
                ReceiptLedgerError::CommitUncertain {
                    receipt_key_digest: identity.receipt_key_digest().clone(),
                },
            ));
        }
        observation::mark(
            "materialize.task_readback",
            "complete",
            Some(deadline.saturating_duration_since(Instant::now())),
        );
        hooks.event(V5ReceiptRuntimeEventKind::TaskStoreCreated, bind_epoch_ms);
        if cancel_requested && !readback.cancel_requested {
            readback = self
                .task_store
                .request_cancel_exact(&identity, readback.version, provider_deadline)
                .map_err(|error| {
                    V5TaskProjectionFailure::from_task_store(
                        error,
                        identity.receipt_key_digest().clone(),
                        true,
                    )
                })?;
        }
        let task = receipt_task_projection_from_store(&readback)?;
        observation::mark(
            "materialize.bind_link",
            "enter",
            Some(deadline.saturating_duration_since(Instant::now())),
        );
        let bound = self
            .lifecycle_links
            .materialize_task_bound(
                reservation,
                task,
                readback.version,
                bind_epoch_ms,
                phase,
                provider_deadline,
            )
            .map_err(V5TaskProjectionFailure::from_link_store)?;
        observation::mark(
            "materialize.bind_link",
            "complete",
            Some(deadline.saturating_duration_since(Instant::now())),
        );
        if record_reservation_conversion {
            hooks.event(
                V5ReceiptRuntimeEventKind::TaskLinkReservationConverted,
                bind_epoch_ms,
            );
        }
        Ok((readback, bound))
    }

    fn complete_recovered_terminal_bound(
        &self,
        expected: &TaskBoundReceipt,
        terminal: V5StoredInvocationRecord,
        deadline: Instant,
    ) -> Result<TaskTerminalBoundReceipt, V5TaskProjectionFailure> {
        let (terminal_status, terminal_digest, terminal_epoch_ms) = match &terminal.task {
            V5StoredTask::Completed {
                terminal_epoch_ms,
                terminal_digest,
                ..
            } => (
                ClosedTerminalStatus::Completed,
                terminal_digest.clone(),
                *terminal_epoch_ms,
            ),
            V5StoredTask::Failed {
                terminal_epoch_ms,
                terminal_digest,
                ..
            } => (
                ClosedTerminalStatus::Failed,
                terminal_digest.clone(),
                *terminal_epoch_ms,
            ),
            V5StoredTask::Cancelled {
                terminal_epoch_ms,
                terminal_digest,
            } => (
                ClosedTerminalStatus::Cancelled,
                terminal_digest.clone(),
                *terminal_epoch_ms,
            ),
            V5StoredTask::Queued | V5StoredTask::Working => {
                return Err(Self::startup_fail_stop(
                    "terminal Task reconciliation received an active Task",
                ))
            }
        };
        let terminal_task = receipt_task_projection_from_store(&terminal)?;
        self.lifecycle_links
            .publish_task_terminal_bound(
                expected,
                terminal_task,
                terminal.version,
                terminal_status,
                terminal_digest,
                terminal_epoch_ms,
                unica_work_control::provider_deadline::ProviderDeadline::new(deadline),
            )
            .map_err(V5TaskProjectionFailure::from_link_store)
    }

    fn terminalize_recovered_bound(
        &self,
        expected: &TaskBoundReceipt,
        record: V5StoredInvocationRecord,
        reason: RecoveryTerminalReason,
        deadline: Instant,
    ) -> Result<TaskTerminalBoundReceipt, V5TaskProjectionFailure> {
        let deadline = committed_decision_step_deadline(deadline);
        let provider_deadline =
            unica_work_control::provider_deadline::ProviderDeadline::new(deadline);
        let identity = record.identity();
        let receipt_key_digest = record.receipt_key_digest.clone();
        let terminal = self
            .task_store
            .terminalize_recovered_exact(&identity, record.version, reason, provider_deadline)
            .map_err(|error| {
                V5TaskProjectionFailure::from_task_store(error, receipt_key_digest, true)
            })?;
        let (terminal_status, terminal_digest, terminal_epoch_ms) = match &terminal.task {
            V5StoredTask::Failed {
                terminal_epoch_ms,
                terminal_digest,
                ..
            } => (
                ClosedTerminalStatus::Failed,
                terminal_digest.clone(),
                *terminal_epoch_ms,
            ),
            V5StoredTask::Cancelled {
                terminal_epoch_ms,
                terminal_digest,
            } => (
                ClosedTerminalStatus::Cancelled,
                terminal_digest.clone(),
                *terminal_epoch_ms,
            ),
            _ => {
                return Err(V5TaskProjectionFailure::fail_stop(
                    ReceiptLedgerError::Corrupt(
                        "recovery terminalization returned a non-recovery Task state",
                    ),
                ))
            }
        };
        let terminal_task = receipt_task_projection_from_store(&terminal)?;
        self.lifecycle_links
            .publish_task_terminal_bound(
                expected,
                terminal_task,
                terminal.version,
                terminal_status,
                terminal_digest,
                terminal_epoch_ms,
                provider_deadline,
            )
            .map_err(V5TaskProjectionFailure::from_link_store)
    }

    fn start_bound_task(
        &self,
        expected: &TaskBoundReceipt,
        record: V5StoredInvocationRecord,
        deadline: Instant,
    ) -> Result<(V5StoredInvocationRecord, TaskBoundReceipt), V5TaskProjectionFailure> {
        if expected.phase() != crate::receipt_ledger::AttemptPhase::Begun
            || record.cancel_requested
            || !matches!(&record.task, V5StoredTask::Queued)
        {
            return Ok((record, expected.clone()));
        }
        let provider_deadline =
            unica_work_control::provider_deadline::ProviderDeadline::new(deadline);
        let identity = record.identity();
        let receipt_key_digest = record.receipt_key_digest.clone();
        let record = match self
            .task_store
            .start_working_if_not_cancel_requested(&identity, record.version, provider_deadline)
            .map_err(|error| {
                V5TaskProjectionFailure::from_task_store(error, receipt_key_digest.clone(), true)
            })? {
            V5StartWorkingOutcome::Started(record)
            | V5StartWorkingOutcome::CancelOrTerminalWinner(record) => record,
        };
        let task = receipt_task_projection_from_store(&record)?;
        let bound = self
            .lifecycle_links
            .refresh_task_bound_projection(expected, task, provider_deadline)
            .map_err(V5TaskProjectionFailure::from_link_store)?;
        Ok((record, bound))
    }

    fn start_not_begun_bound_task(
        &self,
        expected: &TaskBoundReceipt,
        record: V5StoredInvocationRecord,
        deadline: Instant,
    ) -> Result<(V5StoredInvocationRecord, TaskBoundReceipt), V5TaskProjectionFailure> {
        if expected.phase() != AttemptPhase::NotBegun
            || record.cancel_requested
            || record.task != V5StoredTask::Queued
        {
            return Ok((record, expected.clone()));
        }
        let provider_deadline =
            unica_work_control::provider_deadline::ProviderDeadline::new(deadline);
        let identity = record.identity();
        let receipt_key_digest = record.receipt_key_digest.clone();
        let record = match self
            .task_store
            .start_working_if_not_cancel_requested(&identity, record.version, provider_deadline)
            .map_err(|error| {
                V5TaskProjectionFailure::from_task_store(error, receipt_key_digest, true)
            })? {
            V5StartWorkingOutcome::Started(record)
            | V5StartWorkingOutcome::CancelOrTerminalWinner(record) => record,
        };
        if record.task != V5StoredTask::Working {
            let task = receipt_task_projection_from_store(&record)?;
            let bound = self
                .lifecycle_links
                .refresh_task_bound_projection(expected, task, provider_deadline)
                .map_err(V5TaskProjectionFailure::from_link_store)?;
            return Ok((record, bound));
        }
        Ok((record, expected.clone()))
    }

    fn authorize_not_begun_bound_task_start(
        &self,
        expected: &TaskBoundReceipt,
        record: &V5StoredInvocationRecord,
        deadline: Instant,
    ) -> Result<TaskBoundReceipt, V5TaskProjectionFailure> {
        if expected.phase() != AttemptPhase::NotBegun
            || record.cancel_requested
            || record.task != V5StoredTask::Queued
            || !task_bound_matches_record(expected, record)?
        {
            return Err(V5TaskProjectionFailure::fail_stop(
                ReceiptLedgerError::TaskBoundMismatch,
            ));
        }
        self.lifecycle_links
            .refresh_task_bound_projection(
                expected,
                expected.task().clone(),
                unica_work_control::provider_deadline::ProviderDeadline::new(deadline),
            )
            .map_err(V5TaskProjectionFailure::from_link_store)
    }

    fn mark_not_begun_bound_task_begun(
        &self,
        expected: &TaskBoundReceipt,
        record: &V5StoredInvocationRecord,
        deadline: Instant,
    ) -> Result<TaskBoundReceipt, V5TaskProjectionFailure> {
        if expected.phase() != AttemptPhase::NotBegun || record.task != V5StoredTask::Working {
            return Err(V5TaskProjectionFailure::fail_stop(
                ReceiptLedgerError::TaskBoundMismatch,
            ));
        }
        let provider_deadline =
            unica_work_control::provider_deadline::ProviderDeadline::new(deadline);
        let bound = self
            .lifecycle_links
            .mark_task_bound_begun(
                expected,
                record.version,
                record.updated_at_epoch_ms,
                provider_deadline,
            )
            .map_err(V5TaskProjectionFailure::from_link_store)?;
        Ok(bound)
    }

    fn terminal_publication(
        terminal: &crate::receipt_ledger::V5CanonicalTerminal,
        terminal_epoch_ms: u64,
    ) -> (V5TerminalPublication, ClosedTerminalStatus) {
        let terminal_digest = terminal.digest().clone();
        match terminal.outcome() {
            ReceiptTerminalOutcome::Completed { result } => (
                V5TerminalPublication::Completed {
                    terminal_epoch_ms,
                    terminal_digest: terminal_digest.clone(),
                    result: result.clone(),
                },
                ClosedTerminalStatus::Completed,
            ),
            ReceiptTerminalOutcome::Failed { reason } => (
                V5TerminalPublication::Failed {
                    terminal_epoch_ms,
                    terminal_digest: terminal_digest.clone(),
                    reason: *reason,
                },
                ClosedTerminalStatus::Failed,
            ),
            ReceiptTerminalOutcome::Cancelled => (
                V5TerminalPublication::Cancelled {
                    terminal_epoch_ms,
                    terminal_digest: terminal_digest.clone(),
                },
                ClosedTerminalStatus::Cancelled,
            ),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn publish_bound_task_terminal<H: InvocationHost>(
        &self,
        expected: &TaskBoundReceipt,
        record: &V5StoredInvocationRecord,
        terminal: &crate::receipt_ledger::V5CanonicalTerminal,
        terminal_epoch_ms: u64,
        deadline: Instant,
        hooks: &dyn V5RuntimeHooks<H>,
    ) -> Result<(V5StoredInvocationRecord, TaskTerminalBoundReceipt), V5TaskProjectionFailure> {
        let (publication, _) = Self::terminal_publication(terminal, terminal_epoch_ms);
        let provider_deadline =
            unica_work_control::provider_deadline::ProviderDeadline::new(deadline);
        let identity = record.identity();
        let receipt_key_digest = record.receipt_key_digest.clone();
        let terminal_record = self
            .task_store
            .publish_terminal_exact(&identity, record.version, publication, provider_deadline)
            .map_err(|error| {
                V5TaskProjectionFailure::from_task_store(error, receipt_key_digest, true)
            })?;
        self.finish_task_terminal_publication(expected, terminal_record, deadline, hooks)
    }

    fn publish_staged_bound_task_terminal<H: InvocationHost>(
        &self,
        staged: &TaskHandoffActorBoundReceipt,
        expected: &TaskBoundReceipt,
        record: &V5StoredInvocationRecord,
        deadline: Instant,
        hooks: &dyn V5RuntimeHooks<H>,
    ) -> Result<(V5StoredInvocationRecord, TaskTerminalBoundReceipt), V5TaskProjectionFailure> {
        let HandoffTerminalStage::Staged {
            terminal,
            terminal_epoch_ms,
            certificate,
        } = staged.terminal_stage()
        else {
            return Err(V5TaskProjectionFailure::fail_stop(
                ReceiptLedgerError::TaskBoundMismatch,
            ));
        };
        if !certificate.matches_staged_terminal(
            staged.key(),
            staged.key_digest(),
            staged.link(),
            *terminal_epoch_ms,
            terminal,
        ) || expected.key() != staged.key()
            || expected.key_digest() != staged.key_digest()
            || expected.link() != staged.link()
            || expected.phase() != staged.phase()
            || expected.task_record_version() != record.version
            || expected.task() != &receipt_task_projection_from_store(record)?
            || record.task_id != staged.key().reserved_task_id()
            || record.invocation_id != staged.key().invocation_id()
            || &record.receipt_key_digest != staged.key_digest()
            || record.tool != staged.key().tool()
            || &record.normalized_arguments_hash != staged.key().normalized_arguments_hash()
            || &record.workspace_identity_hash != staged.workspace_identity_hash()
            || record.created_at_epoch_ms != staged.task().created_at_epoch_ms()
            || record.ttl_ms != staged.task().ttl_ms()
            || record.poll_interval_ms != staged.task().poll_interval_ms()
            || record.cancel_requested != staged.cancel_requested()
        {
            return Err(V5TaskProjectionFailure::fail_stop(
                ReceiptLedgerError::TaskBoundMismatch,
            ));
        }
        let (publication, _) = Self::terminal_publication(terminal, *terminal_epoch_ms);
        let terminal_record = self
            .task_store
            .publish_staged_terminal_against_exact_provisional(
                record,
                publication,
                unica_work_control::provider_deadline::ProviderDeadline::new(deadline),
            )
            .map_err(|error| {
                V5TaskProjectionFailure::from_task_store(error, staged.key_digest().clone(), true)
            })?;
        self.finish_task_terminal_publication(expected, terminal_record, deadline, hooks)
    }

    fn finish_task_terminal_publication<H: InvocationHost>(
        &self,
        expected: &TaskBoundReceipt,
        terminal_record: V5StoredInvocationRecord,
        deadline: Instant,
        hooks: &dyn V5RuntimeHooks<H>,
    ) -> Result<(V5StoredInvocationRecord, TaskTerminalBoundReceipt), V5TaskProjectionFailure> {
        let provider_deadline =
            unica_work_control::provider_deadline::ProviderDeadline::new(deadline);
        let readback = self
            .task_store
            .get(terminal_record.task_id, provider_deadline)
            .map_err(|error| {
                V5TaskProjectionFailure::from_task_store(
                    error,
                    terminal_record.receipt_key_digest.clone(),
                    true,
                )
            })?;
        if readback != terminal_record {
            return Err(V5TaskProjectionFailure::fail_stop(
                ReceiptLedgerError::CommitUncertain {
                    receipt_key_digest: terminal_record.receipt_key_digest.clone(),
                },
            ));
        }
        let (terminal_status, terminal_digest, terminal_epoch_ms) = match &terminal_record.task {
            V5StoredTask::Completed {
                terminal_digest,
                terminal_epoch_ms,
                ..
            } => (
                ClosedTerminalStatus::Completed,
                terminal_digest.clone(),
                *terminal_epoch_ms,
            ),
            V5StoredTask::Failed {
                terminal_digest,
                terminal_epoch_ms,
                ..
            } => (
                ClosedTerminalStatus::Failed,
                terminal_digest.clone(),
                *terminal_epoch_ms,
            ),
            V5StoredTask::Cancelled {
                terminal_digest,
                terminal_epoch_ms,
            } => (
                ClosedTerminalStatus::Cancelled,
                terminal_digest.clone(),
                *terminal_epoch_ms,
            ),
            V5StoredTask::Queued | V5StoredTask::Working => {
                return Err(V5TaskProjectionFailure::fail_stop(
                    ReceiptLedgerError::TaskBoundMismatch,
                ))
            }
        };
        if hooks.holds(V5PausePoint::AfterTaskStoreTerminalBeforeLifecycleLinkTerminal) {
            hooks.event(
                V5ReceiptRuntimeEventKind::TaskStoreTerminalCommitted,
                terminal_epoch_ms,
            );
            hooks
                .pause(
                    V5PausePoint::AfterTaskStoreTerminalBeforeLifecycleLinkTerminal,
                    hooks.commit_deadline_at(
                        V5PausePoint::AfterTaskStoreTerminalBeforeLifecycleLinkTerminal,
                        deadline,
                    ),
                )
                .map_err(V5TaskProjectionFailure::fail_stop)?;
            if hooks.process_exited() {
                return Err(V5TaskProjectionFailure::fail_stop(
                    ReceiptLedgerError::StoreUnavailable,
                ));
            }
        }
        let terminal_task = receipt_task_projection_from_store(&terminal_record)?;
        let terminal_link = self
            .lifecycle_links
            .publish_task_terminal_bound(
                expected,
                terminal_task,
                terminal_record.version,
                terminal_status,
                terminal_digest,
                terminal_epoch_ms,
                provider_deadline,
            )
            .map_err(V5TaskProjectionFailure::from_link_store)?;
        Ok((terminal_record, terminal_link))
    }

    fn read_bound_task(
        &self,
        task_id: unica_execution_contract::invocation::TaskId,
        deadline: Instant,
    ) -> Result<Option<V5StoredInvocationRecord>, V5TaskProjectionFailure> {
        let provider_deadline =
            unica_work_control::provider_deadline::ProviderDeadline::new(deadline);
        let link = match self
            .lifecycle_links
            .read_by_task_id(task_id, provider_deadline)
        {
            Ok(link) => link,
            Err(TaskLifecycleLinkStoreError::NotFound { .. }) => return Ok(None),
            Err(error) => return Err(V5TaskProjectionFailure::from_link_store(error)),
        };
        let retiring = matches!(link, TaskLifecycleLinkRecord::TaskRetirementPending(_));
        let mask_working_as_queued = matches!(
            &link,
            TaskLifecycleLinkRecord::TaskBound(record)
                if record.phase() == AttemptPhase::NotBegun
        );
        let expected = match link {
            TaskLifecycleLinkRecord::TaskBound(record) => record.link().clone(),
            TaskLifecycleLinkRecord::TaskTerminalBound(record) => record.link().clone(),
            TaskLifecycleLinkRecord::TaskRetirementPending(record) => record.link().clone(),
        };
        let record = match self.task_store.get(task_id, provider_deadline) {
            Ok(record) => record,
            // A committed retirement intent permits an absent Task: its
            // retirement deleted the record and has not yet removed the link.
            Err(V5TaskStoreError::NotFound { .. }) if retiring => return Ok(None),
            Err(V5TaskStoreError::NotFound { .. }) => {
                return Err(V5TaskProjectionFailure::fail_stop(
                    ReceiptLedgerError::Corrupt(
                        "active lifecycle link has no exact TaskStore record",
                    ),
                ))
            }
            Err(error) => {
                return Err(V5TaskProjectionFailure::from_task_store(
                    error,
                    expected.receipt_key_digest().clone(),
                    true,
                ))
            }
        };
        let identity = V5TaskIdentity::new(
            expected.task_id(),
            expected.invocation_id(),
            expected.receipt_key_digest().clone(),
        );
        if !identity.matches_record(&record) {
            return Err(V5TaskProjectionFailure::fail_stop(
                ReceiptLedgerError::Corrupt(
                    "TaskStore record contradicts its sole lifecycle-link identity",
                ),
            ));
        }
        Ok(Some(project_bound_task_for_read(
            record,
            mask_working_as_queued,
        )))
    }

    fn cancel_bound_task<H: InvocationHost>(
        &self,
        task_id: unica_execution_contract::invocation::TaskId,
        deadline: Instant,
        hooks: &dyn V5RuntimeHooks<H>,
    ) -> Result<Option<V5StoredInvocationRecord>, V5TaskProjectionFailure> {
        loop {
            let Some(record) = self.read_bound_task(task_id, deadline)? else {
                return Ok(None);
            };
            if record.task.is_terminal() || record.cancel_requested {
                return Ok(Some(record));
            }
            hooks
                .pause(V5PausePoint::BeforeTaskCancelWrite, deadline)
                .map_err(|error| V5TaskProjectionFailure {
                    error,
                    fail_stop: false,
                })?;
            let identity = record.identity();
            match self.task_store.request_cancel_exact(
                &identity,
                record.version,
                unica_work_control::provider_deadline::ProviderDeadline::new(deadline),
            ) {
                Ok(cancelled) => return Ok(Some(cancelled)),
                // The attempt moved its Task between the read and the cancel,
                // as when it starts right after its handoff (#1395): read the
                // Task again. Each retry follows another committed transition.
                Err(V5TaskStoreError::Mismatch {
                    reason: V5TaskMismatch::Version { .. },
                    ..
                }) => {}
                Err(error) => {
                    return Err(V5TaskProjectionFailure::from_task_store(
                        error,
                        record.receipt_key_digest,
                        true,
                    ))
                }
            }
        }
    }

    /// The lifecycle link of a Task id, if it has one.
    fn read_link(
        &self,
        task_id: unica_execution_contract::invocation::TaskId,
        deadline: Instant,
    ) -> Result<Option<TaskLifecycleLinkRecord>, V5TaskProjectionFailure> {
        match self.lifecycle_links.read_by_task_id(
            task_id,
            unica_work_control::provider_deadline::ProviderDeadline::new(deadline),
        ) {
            Ok(link) => Ok(Some(link)),
            Err(TaskLifecycleLinkStoreError::NotFound { .. }) => Ok(None),
            Err(error) => Err(V5TaskProjectionFailure::from_link_store(error)),
        }
    }

    fn cancel_exact_bound_task<H: InvocationHost>(
        &self,
        key: &ReceiptKey,
        deadline: Instant,
        hooks: &dyn V5RuntimeHooks<H>,
    ) -> Result<Option<V5StoredInvocationRecord>, V5TaskProjectionFailure> {
        let Some(link) = self.read_link(key.reserved_task_id(), deadline)? else {
            return Ok(None);
        };
        if link.key() != key {
            // Another call naming this Task id: the caller's identity is
            // wrong, the stores are not (as for an exact repeat of submit).
            return Err(V5TaskProjectionFailure {
                error: ReceiptLedgerError::InvocationIdentityMismatch,
                fail_stop: false,
            });
        }
        self.cancel_bound_task(key.reserved_task_id(), deadline, hooks)
    }

    /// The Task an exact key passed into, read by its link. A link of the
    /// same Task id under another key is a caller identity mismatch.
    fn read_exact_bound_task(
        &self,
        key: &ReceiptKey,
        deadline: Instant,
    ) -> Result<ExactBoundTask, V5TaskProjectionFailure> {
        let Some(link) = self.read_link(key.reserved_task_id(), deadline)? else {
            return Ok(ExactBoundTask::Absent);
        };
        if link.key() != key {
            return Err(V5TaskProjectionFailure {
                error: ReceiptLedgerError::InvocationIdentityMismatch,
                fail_stop: false,
            });
        }
        Ok(
            match self.read_bound_task(key.reserved_task_id(), deadline)? {
                Some(record) => ExactBoundTask::Live(record),
                None => ExactBoundTask::Retiring,
            },
        )
    }
}

/// What the lifecycle link store holds for an exact receipt key.
enum ExactBoundTask {
    /// No link: the key, if anywhere, is still in the receipt ledger.
    Absent,
    Live(V5StoredInvocationRecord),
    /// The link is being retired with its Task.
    Retiring,
}

fn project_bound_task_for_read(
    mut record: V5StoredInvocationRecord,
    mask_working_as_queued: bool,
) -> V5StoredInvocationRecord {
    if mask_working_as_queued && record.task == V5StoredTask::Working {
        record.task = V5StoredTask::Queued;
    }
    record
}

fn log_adopted_bound_handoff(task_id: unica_execution_contract::invocation::TaskId) {
    eprintln!(
        "protocol-v5 startup: Task {task_id} was bound before its predecessor exited; \
         completing its handoff with a recovery terminal"
    );
}

/// The immutable Task fields a handoff receipt promises its Task row.
fn expected_handoff_record(handoff: &TaskHandoffActorBoundReceipt) -> NewV5InvocationRecord {
    NewV5InvocationRecord::new(
        V5TaskIdentity::new(
            handoff.task().task_id(),
            handoff.task().invocation_id(),
            handoff.key_digest().clone(),
        ),
        handoff.key().tool(),
        handoff.key().normalized_arguments_hash().clone(),
        handoff.workspace_identity_hash().clone(),
        handoff.task().poll_interval_ms(),
        handoff.task().ttl_ms(),
    )
    .with_initial_epoch_ms(handoff.task().created_at_epoch_ms())
}

/// The Task row and TaskBound link a dead predecessor wrote for an unstaged
/// handoff it did not complete.
type BoundStartupHandoff = (V5StoredInvocationRecord, TaskBoundReceipt);

/// The TaskBound link of an unstaged handoff whose receipt was not completed
/// yet. `None` when the link is still only reserved or absent; an error when
/// the Task's lifecycle entry belongs to another owner.
fn exact_handoff_task_bound<'a>(
    handoff: &TaskHandoffActorBoundReceipt,
    entries: &'a [TaskLifecycleLinkCatalogEntry],
) -> Result<Option<&'a TaskBoundReceipt>, &'static str> {
    let matching = entries
        .iter()
        .filter(|entry| lifecycle_entry_task_id(entry) == handoff.task().task_id())
        .collect::<Vec<_>>();
    let [TaskLifecycleLinkCatalogEntry::Record(TaskLifecycleLinkRecord::TaskBound(bound))] =
        matching.as_slice()
    else {
        return Ok(None);
    };
    if bound.key() != handoff.key()
        || bound.key_digest() != handoff.key_digest()
        || bound.link() != handoff.link()
        || bound.phase() != handoff.phase()
    {
        return Err("preexisting handoff TaskBound link differs from its receipt");
    }
    Ok(Some(bound))
}

fn lifecycle_entry_task_id(
    entry: &TaskLifecycleLinkCatalogEntry,
) -> unica_execution_contract::invocation::TaskId {
    match entry {
        TaskLifecycleLinkCatalogEntry::Reservation(reservation) => {
            reservation.key().reserved_task_id()
        }
        TaskLifecycleLinkCatalogEntry::Record(record) => record.key().reserved_task_id(),
    }
}

fn staged_record_from_durable_projection(
    staged: &TaskHandoffActorBoundReceipt,
    projection: &ReceiptTaskProjection,
    task: V5StoredTask,
) -> Result<V5StoredInvocationRecord, V5TaskProjectionFailure> {
    if projection.task_id() != staged.task().task_id()
        || projection.invocation_id() != staged.task().invocation_id()
        || projection.created_at_epoch_ms() != staged.task().created_at_epoch_ms()
        || projection.ttl_ms() != staged.task().ttl_ms()
        || projection.poll_interval_ms() != staged.task().poll_interval_ms()
    {
        return Err(V5TaskProjection::startup_fail_stop(
            "staged startup durable task projection differs",
        ));
    }
    // Every field comes from durable receipt/link authority. The current Task
    // supplies no missing metadata and cannot authorize its own replacement.
    Ok(V5StoredInvocationRecord {
        schema_version: V5StoredInvocationSchemaVersion,
        task_id: projection.task_id(),
        invocation_id: projection.invocation_id(),
        receipt_key_digest: staged.key_digest().clone(),
        tool: staged.key().tool(),
        normalized_arguments_hash: staged.key().normalized_arguments_hash().clone(),
        workspace_identity_hash: staged.workspace_identity_hash().clone(),
        created_at_epoch_ms: projection.created_at_epoch_ms(),
        updated_at_epoch_ms: projection.updated_at_epoch_ms(),
        ttl_ms: projection.ttl_ms(),
        poll_interval_ms: projection.poll_interval_ms(),
        version: projection.version(),
        cancel_requested: staged.cancel_requested(),
        task,
    })
}

fn task_bound_matches_record(
    expected: &TaskBoundReceipt,
    record: &V5StoredInvocationRecord,
) -> Result<bool, V5TaskProjectionFailure> {
    if !task_link_identity_matches_record(
        expected.key(),
        expected.key_digest(),
        expected.link(),
        expected.task(),
        record,
    ) {
        return Ok(false);
    }
    let exact_projection = expected.task_record_version() == record.version
        && expected.task().version() == record.version
        && expected.task().updated_at_epoch_ms() == record.updated_at_epoch_ms;
    let one_step_successor = expected
        .task_record_version()
        .checked_add(1)
        .is_some_and(|version| version == record.version)
        && expected.task().version() == expected.task_record_version()
        && expected.task().updated_at_epoch_ms() <= record.updated_at_epoch_ms
        && (record.cancel_requested
            || (expected.phase() == AttemptPhase::NotBegun
                && record.task == V5StoredTask::Working)
            || matches!(
                record.task,
                V5StoredTask::Completed { .. }
                    | V5StoredTask::Failed { .. }
                    | V5StoredTask::Cancelled { .. }
            ));
    Ok(exact_projection || one_step_successor)
}

fn task_terminal_bound_matches_record(
    expected: &TaskTerminalBoundReceipt,
    record: &V5StoredInvocationRecord,
) -> Result<bool, V5TaskProjectionFailure> {
    Ok(task_link_identity_matches_record(
        expected.key(),
        expected.key_digest(),
        expected.link(),
        expected.task(),
        record,
    ) && expected.task() == &receipt_task_projection_from_store(record)?
        && expected.task_record_version() == record.version
        && terminal_record_matches(
            record,
            expected.terminal_status(),
            expected.terminal_digest(),
            expected.terminal_epoch_ms(),
        ))
}

fn task_retirement_pending_matches_record(
    expected: &TaskRetirementPendingReceipt,
    record: &V5StoredInvocationRecord,
) -> Result<bool, V5TaskProjectionFailure> {
    Ok(task_link_identity_matches_record(
        expected.key(),
        expected.key_digest(),
        expected.link(),
        expected.task(),
        record,
    ) && expected.task() == &receipt_task_projection_from_store(record)?
        && expected.expected_terminal_task_version() == record.version
        && terminal_record_matches(
            record,
            expected.terminal_status(),
            expected.terminal_digest(),
            expected.terminal_epoch_ms(),
        ))
}

fn canonical_task_retirement_pending_bytes(
    pending: &TaskRetirementPendingReceipt,
) -> Result<Vec<u8>, V5TaskProjectionFailure> {
    serde_json::to_vec(&serde_json::json!({
        "receiptKey": {
            "invocationId": pending.key().invocation_id(),
            "reservedTaskId": pending.key().reserved_task_id(),
            "coreIdentityDigest": pending.key().core_identity_digest(),
            "tool": pending.key().tool(),
            "normalizedArgumentsHash": pending.key().normalized_arguments_hash(),
            "requestScopeHash": pending.key().request_scope_hash(),
            "keyDigest": pending.key_digest(),
        },
        "taskId": pending.task().task_id(),
        "taskLinkDigest": pending.link().digest(),
        "terminalDigest": pending.terminal_digest(),
        "terminalEpochMs": pending.terminal_epoch_ms(),
        "ttlMs": pending.task().ttl_ms(),
        "expiresAtEpochMs": pending.expires_at_epoch_ms(),
        "expectedTaskVersion": pending.expected_terminal_task_version(),
        "resolver": "task_expired",
        "version": pending.lifecycle_link_version(),
    }))
    .map_err(|_| {
        V5TaskProjectionFailure::fail_stop(ReceiptLedgerError::Corrupt(
            "TaskRetirementPending authorization could not be encoded",
        ))
    })
}

fn lower_hex_digest(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn terminal_record_from_retirement_pending(
    pending: &TaskRetirementPendingReceipt,
) -> V5StoredInvocationRecord {
    let task = match pending.terminal_status() {
        ClosedTerminalStatus::Completed => V5StoredTask::Completed {
            terminal_epoch_ms: pending.terminal_epoch_ms(),
            terminal_digest: pending.terminal_digest().clone(),
            result: Box::new(unica_execution_contract::invocation::DomainResult::success(
                "retirement-absence-proof",
            )),
        },
        ClosedTerminalStatus::Failed => V5StoredTask::Failed {
            terminal_epoch_ms: pending.terminal_epoch_ms(),
            terminal_digest: pending.terminal_digest().clone(),
            reason: V5SafeFailureReason::InvocationFailed,
        },
        ClosedTerminalStatus::Cancelled => V5StoredTask::Cancelled {
            terminal_epoch_ms: pending.terminal_epoch_ms(),
            terminal_digest: pending.terminal_digest().clone(),
        },
    };
    V5StoredInvocationRecord {
        schema_version: crate::invocation_store_v5::V5StoredInvocationSchemaVersion,
        task_id: pending.task().task_id(),
        invocation_id: pending.task().invocation_id(),
        receipt_key_digest: pending.key_digest().clone(),
        tool: pending.key().tool(),
        normalized_arguments_hash: pending.key().normalized_arguments_hash().clone(),
        workspace_identity_hash: pending.link().workspace_identity_hash().clone(),
        created_at_epoch_ms: pending.task().created_at_epoch_ms(),
        updated_at_epoch_ms: pending.terminal_epoch_ms(),
        ttl_ms: pending.task().ttl_ms(),
        poll_interval_ms: pending.task().poll_interval_ms(),
        version: pending.expected_terminal_task_version(),
        cancel_requested: false,
        task,
    }
}

fn task_link_identity_matches_record(
    key: &ReceiptKey,
    key_digest: &ReceiptKeyDigest,
    link: &crate::receipt_ledger::TaskLinkReference,
    task: &ReceiptTaskProjection,
    record: &V5StoredInvocationRecord,
) -> bool {
    key_digest == &record.receipt_key_digest
        && key.reserved_task_id() == record.task_id
        && key.invocation_id() == record.invocation_id
        && key.tool() == record.tool
        && key.normalized_arguments_hash() == &record.normalized_arguments_hash
        && link.receipt_key_digest() == &record.receipt_key_digest
        && link.task_id() == record.task_id
        && link.invocation_id() == record.invocation_id
        && link.workspace_identity_hash() == &record.workspace_identity_hash
        && task.task_id() == record.task_id
        && task.invocation_id() == record.invocation_id
        && task.created_at_epoch_ms() == record.created_at_epoch_ms
        && task.ttl_ms() == record.ttl_ms
        && task.poll_interval_ms() == record.poll_interval_ms
}

fn terminal_record_matches(
    record: &V5StoredInvocationRecord,
    expected_status: ClosedTerminalStatus,
    expected_digest: &TerminalDigest,
    expected_epoch_ms: u64,
) -> bool {
    match &record.task {
        V5StoredTask::Completed {
            terminal_epoch_ms,
            terminal_digest,
            ..
        } => {
            expected_status == ClosedTerminalStatus::Completed
                && terminal_digest == expected_digest
                && *terminal_epoch_ms == expected_epoch_ms
        }
        V5StoredTask::Failed {
            terminal_epoch_ms,
            terminal_digest,
            ..
        } => {
            expected_status == ClosedTerminalStatus::Failed
                && terminal_digest == expected_digest
                && *terminal_epoch_ms == expected_epoch_ms
        }
        V5StoredTask::Cancelled {
            terminal_epoch_ms,
            terminal_digest,
        } => {
            expected_status == ClosedTerminalStatus::Cancelled
                && terminal_digest == expected_digest
                && *terminal_epoch_ms == expected_epoch_ms
        }
        V5StoredTask::Queued | V5StoredTask::Working => false,
    }
}

impl V5TaskProjectionFailure {
    const fn fail_stop(error: ReceiptLedgerError) -> Self {
        Self {
            error,
            fail_stop: true,
        }
    }

    fn from_link_store(error: TaskLifecycleLinkStoreError) -> Self {
        observation::failure(
            "projection.from_link_store",
            match &error {
                TaskLifecycleLinkStoreError::AlreadyOwned => "AlreadyOwned",
                TaskLifecycleLinkStoreError::DeadlineExceeded => "DeadlineExceeded",
                TaskLifecycleLinkStoreError::NotFound { .. } => "NotFound",
                TaskLifecycleLinkStoreError::AlreadyMaterialized { .. } => "AlreadyMaterialized",
                TaskLifecycleLinkStoreError::IdentityMismatch => "IdentityMismatch",
                TaskLifecycleLinkStoreError::ReservationMismatch => "ReservationMismatch",
                TaskLifecycleLinkStoreError::StateMismatch => "StateMismatch",
                TaskLifecycleLinkStoreError::VersionMismatch { .. } => "VersionMismatch",
                TaskLifecycleLinkStoreError::RecordTooLarge { .. } => "RecordTooLarge",
                TaskLifecycleLinkStoreError::CommitUncertain { .. } => "CommitUncertain",
                TaskLifecycleLinkStoreError::Corrupt(_) => "Corrupt",
                TaskLifecycleLinkStoreError::Storage { .. } => "Storage",
            },
            None,
        );
        match error {
            TaskLifecycleLinkStoreError::DeadlineExceeded => Self {
                error: ReceiptLedgerError::DeadlineExceeded,
                fail_stop: false,
            },
            TaskLifecycleLinkStoreError::RecordTooLarge { .. } => Self {
                error: ReceiptLedgerError::RecordTooLarge,
                fail_stop: false,
            },
            TaskLifecycleLinkStoreError::NotFound { .. } => Self {
                error: ReceiptLedgerError::ReceiptNotFound,
                fail_stop: false,
            },
            TaskLifecycleLinkStoreError::AlreadyOwned
            | TaskLifecycleLinkStoreError::AlreadyMaterialized { .. }
            | TaskLifecycleLinkStoreError::IdentityMismatch
            | TaskLifecycleLinkStoreError::ReservationMismatch
            | TaskLifecycleLinkStoreError::StateMismatch
            | TaskLifecycleLinkStoreError::VersionMismatch { .. }
            | TaskLifecycleLinkStoreError::CommitUncertain { .. }
            | TaskLifecycleLinkStoreError::Corrupt(_)
            | TaskLifecycleLinkStoreError::Storage { .. } => {
                Self::fail_stop(ReceiptLedgerError::StoreUnavailable)
            }
        }
    }

    fn from_task_store(
        error: V5TaskStoreError,
        receipt_key_digest: ReceiptKeyDigest,
        capacity_is_invariant: bool,
    ) -> Self {
        observation::failure(
            "projection.from_task_store",
            match &error {
                V5TaskStoreError::DeadlineExceeded => "DeadlineExceeded",
                V5TaskStoreError::Capacity { .. } => "Capacity",
                V5TaskStoreError::RecordTooLarge { .. } => "RecordTooLarge",
                V5TaskStoreError::NotFound { .. } => "NotFound",
                V5TaskStoreError::CommitUncertain { .. } => "CommitUncertain",
                V5TaskStoreError::Mismatch { .. } => "Mismatch",
                V5TaskStoreError::AlreadyOwned => "AlreadyOwned",
                V5TaskStoreError::Corrupt(_) => "Corrupt",
                V5TaskStoreError::Storage { .. } => "Storage",
            },
            None,
        );
        match error {
            V5TaskStoreError::DeadlineExceeded => Self {
                error: ReceiptLedgerError::DeadlineExceeded,
                fail_stop: false,
            },
            V5TaskStoreError::Capacity { .. } if !capacity_is_invariant => Self {
                error: ReceiptLedgerError::CapacityExceeded,
                fail_stop: false,
            },
            V5TaskStoreError::RecordTooLarge { .. } => Self {
                error: ReceiptLedgerError::RecordTooLarge,
                fail_stop: false,
            },
            V5TaskStoreError::NotFound { .. } => Self {
                error: ReceiptLedgerError::ReceiptNotFound,
                fail_stop: false,
            },
            V5TaskStoreError::CommitUncertain { .. } => {
                Self::fail_stop(ReceiptLedgerError::CommitUncertain { receipt_key_digest })
            }
            V5TaskStoreError::Capacity { .. }
            | V5TaskStoreError::Mismatch { .. }
            | V5TaskStoreError::AlreadyOwned
            | V5TaskStoreError::Corrupt(_)
            | V5TaskStoreError::Storage { .. } => {
                Self::fail_stop(ReceiptLedgerError::StoreUnavailable)
            }
        }
    }
}

impl From<ReceiptLedgerError> for V5TaskProjectionFailure {
    fn from(error: ReceiptLedgerError) -> Self {
        Self {
            fail_stop: error.requires_reopen(),
            error,
        }
    }
}

fn receipt_task_projection_from_store(
    record: &V5StoredInvocationRecord,
) -> Result<ReceiptTaskProjection, V5TaskProjectionFailure> {
    ReceiptTaskProjection::new(
        record.task_id,
        record.invocation_id,
        record.created_at_epoch_ms,
        record.updated_at_epoch_ms,
        record.ttl_ms,
        record.poll_interval_ms,
        record.version,
    )
    .map_err(Into::into)
}

impl<H: InvocationHost> V5ReceiptRuntime<H> {
    pub fn open(
        state: &dyn ReceiptStateRoot,
        config: &ReceiptRuntimeConfig<H>,
    ) -> Result<Self, String> {
        let epoch_clock = Arc::clone(&config.epoch_clock);
        Self::open_with_epoch_clock(state, config, epoch_clock)
    }

    pub fn open_with_epoch_clock(
        state: &dyn ReceiptStateRoot,
        config: &ReceiptRuntimeConfig<H>,
        epoch_clock: Arc<dyn EpochMillisClock>,
    ) -> Result<Self, String> {
        let hooks = Arc::clone(&config.hooks);
        let stable_authority = ReceiptAuthorityLock::acquire(state, AUTHORITY_ACQUIRE_TIMEOUT)?;
        let receipts = state.create_private_retained_subdirectory("receipts")?;
        let receipt_ledger = ReceiptLedgerStore::open_retained_directory(receipts)
            .map_err(|error| format!("open protocol-v5 receipt ledger: {error}"))?;
        receipt_ledger
            .generation()
            .map_err(|error| format!("read protocol-v5 receipt generation: {error}"))?;
        let recovery_keys = receipt_ledger
            .recovery_keys(None)
            .map_err(|error| format!("inspect protocol-v5 receipt recovery catalog: {error}"))?;
        #[cfg(any(test, feature = "test-support"))]
        let receipt_owner_identity_for_test = receipt_ledger.ownership_file_identity_for_test();
        let receipt_ledger = spawn_receipt_ledger_actor(receipt_ledger);
        let startup_deadline = Instant::now() + STARTUP_RECONCILIATION_TIMEOUT;
        let task_projection =
            V5TaskProjection::open(state, Arc::clone(&epoch_clock), startup_deadline)?;
        let _task_recovery_entries = task_projection.recovery.entries().len();
        let runtime = Self {
            core_identity_digest: config.core_identity_digest.clone(),
            _stable_authority: stable_authority,
            receipt_ledger,
            #[cfg(any(test, feature = "test-support"))]
            receipt_owner_identity_for_test,
            epoch_clock,
            invocation_executor: (config.host)(),
            task_projection,
            active_task_cancellations: Arc::new(V5ActiveTaskCancellations::default()),
            task_execution_threads: Mutex::new(Vec::new()),
            task_terminal_coordinator: Mutex::new(()),
            external_store_fail_stop: AtomicBool::new(false),
            promoted_continuations: AtomicUsize::new(0),
            hooks,
            joins: JoinRegistry::default(),
            receipt_handles: ReceiptHandles::default(),
            submit_fences: SubmitFences::default(),
            live_attempts: V5LiveAttempts::default(),
        };
        if config.reconcile_startup {
            let bound_handoffs =
                runtime.preflight_existing_handoff_tasks(&recovery_keys, startup_deadline)?;
            runtime.reconcile_pre_task_startup(recovery_keys, bound_handoffs, startup_deadline)?;
            runtime
                .task_projection
                .reconcile_materialized_startup(startup_deadline)
                .map_err(|failure| {
                    let error = runtime.project_task_failure(failure);
                    format!("reconcile protocol-v5 materialized Task startup: {error}")
                })?;
        }
        Ok(runtime)
    }

    /// Validates every unstaged handoff before startup changes any store.
    /// Returns the handoffs whose predecessor already bound the exact
    /// lifecycle link and Task (#1360): the receipt loop completes them with
    /// that link instead of reserving a second one.
    fn preflight_existing_handoff_tasks(
        &self,
        recovery_keys: &[ReceiptKey],
        deadline: Instant,
    ) -> Result<HashMap<unica_execution_contract::invocation::TaskId, BoundStartupHandoff>, String>
    {
        let mut bound_handoffs = HashMap::new();
        let provider_deadline =
            unica_work_control::provider_deadline::ProviderDeadline::new(deadline);
        let lifecycle = self
            .task_projection
            .lifecycle_links
            .catalog_snapshot(provider_deadline)
            .map_err(|error| {
                format!("inspect protocol-v5 startup handoff reservations: {error}")
            })?;
        for key in recovery_keys {
            let state = self
                .receipt_ledger
                .recover(key.clone(), deadline)
                .map_err(|error| format!("inspect protocol-v5 startup handoff receipt: {error}"))?;
            let ReceiptState::TaskHandoffActorBound(handoff) = state else {
                continue;
            };
            if matches!(
                handoff.terminal_stage(),
                HandoffTerminalStage::Staged { .. }
            ) {
                if let Some(plan) = self
                    .task_projection
                    .staged_handoff_startup_plan(&handoff, lifecycle.entries(), deadline)
                    .map_err(|failure| {
                        format!("inspect staged startup handoff proof: {}", failure.error)
                    })?
                {
                    let observed = plan.observed();
                    let Some(recovery) = self
                        .task_projection
                        .recovery
                        .entry(handoff.task().task_id())
                    else {
                        return Err(
                            "staged startup linked Task is absent from the inspected catalog"
                                .to_owned(),
                        );
                    };
                    if !recovery.identity().matches_record(observed)
                        || recovery.version() != observed.version
                        || recovery.status() != observed.task.status()
                        || recovery.cancel_requested() != observed.cancel_requested
                    {
                        return Err(
                            "staged startup Task changed after its initial inspection".to_owned()
                        );
                    }
                    continue;
                }
            }
            let Some(recovery) = self
                .task_projection
                .recovery
                .entry(handoff.task().task_id())
            else {
                continue;
            };
            if let Some(bound) =
                exact_handoff_task_bound(&handoff, lifecycle.entries()).map_err(str::to_owned)?
            {
                // The predecessor died after the link became TaskBound and
                // before the receipt took it over (#1360): the receipt loop
                // adopts this exact link instead of reserving a new one.
                let record = self
                    .task_projection
                    .task_store
                    .get(handoff.task().task_id(), provider_deadline)
                    .map_err(|error| {
                        format!("read preexisting protocol-v5 bound handoff Task: {error}")
                    })?;
                let bound_matches = task_bound_matches_record(bound, &record)
                    .map_err(|failure| failure.error.to_string())?;
                // The receipt still owns the attempt, so nothing started or
                // finished the Task behind it: NotBegun keeps it Queued, Begun
                // created it Working. A cancel flag the receipt does not carry
                // is only the one step a cancel through the link adds.
                let phase_status = match handoff.phase() {
                    AttemptPhase::NotBegun => V5StoredTask::Queued,
                    AttemptPhase::Begun => V5StoredTask::Working,
                };
                let cancel_matches = record.cancel_requested == handoff.cancel_requested()
                    || !record.cancel_requested
                    || bound
                        .task_record_version()
                        .checked_add(1)
                        .is_some_and(|version| version == record.version);
                if !recovery.identity().matches_record(&record)
                    || recovery.version() != record.version
                    || recovery.status() != record.task.status()
                    || recovery.cancel_requested() != record.cancel_requested
                    || !expected_handoff_record(&handoff).matches_record(&record)
                    || !bound_matches
                    || record.task != phase_status
                    || !cancel_matches
                {
                    return Err(
                        "preexisting handoff Task contradicts its TaskBound link".to_owned()
                    );
                }
                bound_handoffs.insert(handoff.task().task_id(), (record, bound.clone()));
                continue;
            }
            let matching = lifecycle
                .entries()
                .iter()
                .filter(|entry| lifecycle_entry_task_id(entry) == handoff.task().task_id())
                .collect::<Vec<_>>();
            let [TaskLifecycleLinkCatalogEntry::Reservation(reservation)] = matching.as_slice()
            else {
                return Err(
                    "preexisting handoff Task has no exact prior link reservation".to_owned(),
                );
            };
            if reservation.key() != handoff.key() || reservation.link() != handoff.link() {
                return Err(
                    "preexisting handoff Task has no exact prior link reservation".to_owned(),
                );
            }
            let record = self
                .task_projection
                .task_store
                .get(handoff.task().task_id(), provider_deadline)
                .map_err(|error| format!("read preexisting protocol-v5 handoff Task: {error}"))?;
            let expected = expected_handoff_record(&handoff);
            let state_matches = match handoff.phase() {
                AttemptPhase::NotBegun => {
                    record.task == V5StoredTask::Queued
                        && (!record.cancel_requested || handoff.cancel_requested())
                }
                AttemptPhase::Begun => {
                    record.task == V5StoredTask::Working
                        && record.cancel_requested == handoff.cancel_requested()
                }
            };
            if !recovery.identity().matches_record(&record)
                || recovery.version() != record.version
                || recovery.status() != record.task.status()
                || recovery.cancel_requested() != record.cancel_requested
                || !expected.matches_record(&record)
                || !state_matches
            {
                return Err("preexisting handoff Task contradicts its exact receipt".to_owned());
            }
        }
        Ok(bound_handoffs)
    }

    fn reconcile_pre_task_startup(
        &self,
        recovery_keys: Vec<ReceiptKey>,
        mut bound_handoffs: HashMap<
            unica_execution_contract::invocation::TaskId,
            BoundStartupHandoff,
        >,
        deadline: Instant,
    ) -> Result<(), String> {
        let terminal_epoch_ms = self.epoch_ms();
        for key in recovery_keys {
            let state =
                match self
                    .receipt_ledger
                    .recover_at(key.clone(), terminal_epoch_ms, deadline)
                {
                    Ok(state) => state,
                    Err(ReceiptLedgerError::ReceiptNotFound) => continue,
                    Err(error) => {
                        return Err(format!(
                            "classify protocol-v5 startup receipt recovery: {error}"
                        ))
                    }
                };
            match state {
                ReceiptState::Reserved(reserved) => {
                    let outcome = match reserved.phase() {
                        ReservedPhase::Begun { .. } => ReceiptTerminalOutcome::Failed {
                            reason: V5SafeFailureReason::OutcomeUncertain,
                        },
                        ReservedPhase::Unbound | ReservedPhase::ActorBound { .. }
                            if reserved.cancel_requested() =>
                        {
                            ReceiptTerminalOutcome::Cancelled
                        }
                        ReservedPhase::Unbound | ReservedPhase::ActorBound { .. } => {
                            ReceiptTerminalOutcome::Failed {
                                reason: V5SafeFailureReason::Interrupted,
                            }
                        }
                    };
                    let terminal = match &outcome {
                        ReceiptTerminalOutcome::Cancelled => {
                            crate::receipt_ledger::canonical_cancelled_terminal()
                        }
                        ReceiptTerminalOutcome::Failed { reason } => {
                            crate::receipt_ledger::canonical_failed_terminal(*reason)
                        }
                        ReceiptTerminalOutcome::Completed { .. } => {
                            unreachable!("recovery is never success")
                        }
                    }
                    .map_err(|error| {
                        format!("prepare protocol-v5 startup recovery terminal: {error}")
                    })?;
                    self.receipt_ledger
                        .publish_direct_terminal(
                            key,
                            reserved.record_version(),
                            terminal_epoch_ms,
                            terminal,
                            deadline,
                        )
                        .map_err(|error| {
                            format!("publish protocol-v5 startup recovery terminal: {error}")
                        })?;
                }
                ReceiptState::TaskPromisedUnbound(promised) => {
                    let expected = TaskCancellationReceipt::PromisedUnbound(promised);
                    let outcome = if expected.cancel_requested() {
                        ReceiptTerminalOutcome::Cancelled
                    } else {
                        ReceiptTerminalOutcome::Failed {
                            reason: V5SafeFailureReason::Interrupted,
                        }
                    };
                    let terminal = match &outcome {
                        ReceiptTerminalOutcome::Cancelled => {
                            crate::receipt_ledger::canonical_cancelled_terminal()
                        }
                        ReceiptTerminalOutcome::Failed { reason } => {
                            crate::receipt_ledger::canonical_failed_terminal(*reason)
                        }
                        ReceiptTerminalOutcome::Completed { .. } => {
                            unreachable!("recovery is never success")
                        }
                    }
                    .map_err(|error| {
                        format!("prepare protocol-v5 startup Task recovery terminal: {error}")
                    })?;
                    self.receipt_ledger
                        .publish_receipt_backed_task_terminal(
                            key,
                            expected,
                            terminal_epoch_ms,
                            terminal,
                            deadline,
                        )
                        .map_err(|error| {
                            format!(
                                "publish protocol-v5 startup receipt-backed Task terminal: {error}"
                            )
                        })?;
                }
                ReceiptState::TaskReceiptOwnedActorBound(receipt_owned) => {
                    let expected = TaskCancellationReceipt::ReceiptOwnedActorBound(receipt_owned);
                    let terminal = crate::receipt_ledger::canonical_failed_terminal(
                        V5SafeFailureReason::OutcomeUncertain,
                    )
                    .map_err(|error| {
                        format!("prepare protocol-v5 receipt-owned recovery terminal: {error}")
                    })?;
                    self.receipt_ledger
                        .publish_receipt_backed_task_terminal(
                            key,
                            expected,
                            terminal_epoch_ms,
                            terminal,
                            deadline,
                        )
                        .map_err(|error| {
                            format!("publish protocol-v5 receipt-owned recovery terminal: {error}")
                        })?;
                }
                ReceiptState::TaskPromisedActorBound(promised) => {
                    let receipt_version = promised.record_version();
                    let (record, task_bound) = self
                        .task_projection
                        .materialize_promised_actor_bound(
                            &promised,
                            terminal_epoch_ms,
                            deadline,
                            self.hooks.as_ref(),
                        )
                        .map_err(|failure| {
                            let error = self.project_task_failure(failure);
                            format!("materialize protocol-v5 startup actor-bound Task: {error}")
                        })?;
                    let task_bound = self
                        .complete_bound_task_handoff(key, receipt_version, task_bound, deadline)
                        .map_err(|error| {
                            format!(
                                "publish protocol-v5 startup actor-bound Task ownership: {error}"
                            )
                        })?;
                    let reason = if record.cancel_requested {
                        RecoveryTerminalReason::Cancelled
                    } else {
                        RecoveryTerminalReason::InterruptedBeforeExecution
                    };
                    self.task_projection
                        .terminalize_recovered_bound(&task_bound, record, reason, deadline)
                        .map_err(|failure| {
                            let error = self.project_task_failure(failure);
                            format!("terminalize protocol-v5 startup bound Task: {error}")
                        })?;
                }
                ReceiptState::TaskHandoffActorBound(handoff) => {
                    if matches!(
                        handoff.terminal_stage(),
                        HandoffTerminalStage::Staged { .. }
                    ) {
                        self.reconcile_staged_handoff_startup(handoff.clone(), deadline)
                        .map_err(|error| {
                            format!(
                                "reconcile protocol-v5 staged handoff terminal before listener: {error}"
                            )
                        })?;
                        continue;
                    }
                    let receipt_version = handoff.record_version();
                    let phase = handoff.phase();
                    let (record, task_bound) =
                        match bound_handoffs.remove(&handoff.task().task_id()) {
                            Some(adopted) => {
                                log_adopted_bound_handoff(handoff.task().task_id());
                                adopted
                            }
                            None => self
                                .task_projection
                                .materialize_recovered_handoff(
                                    &handoff,
                                    terminal_epoch_ms,
                                    deadline,
                                    self.hooks.as_ref(),
                                )
                                .map_err(|failure| {
                                    let error = self.project_task_failure(failure);
                                    format!("materialize protocol-v5 startup Task handoff: {error}")
                                })?,
                        };
                    let task_bound = self
                        .complete_bound_task_handoff(key, receipt_version, task_bound, deadline)
                        .map_err(|error| {
                            format!("publish protocol-v5 startup Task handoff ownership: {error}")
                        })?;
                    let reason = match phase {
                        AttemptPhase::Begun => RecoveryTerminalReason::OutcomeUncertain,
                        AttemptPhase::NotBegun if record.cancel_requested => {
                            RecoveryTerminalReason::Cancelled
                        }
                        AttemptPhase::NotBegun => {
                            RecoveryTerminalReason::InterruptedBeforeExecution
                        }
                    };
                    self.task_projection
                        .terminalize_recovered_bound(&task_bound, record, reason, deadline)
                        .map_err(|failure| {
                            let error = self.project_task_failure(failure);
                            format!("terminalize protocol-v5 startup handoff Task: {error}")
                        })?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// The receipt row of a key leaves the ledger only here, once its Task
    /// link stands. Under the key's lock, so an exact repeat of the submit
    /// that looked for the link a moment ago cannot reserve the key again
    /// after the row is gone (#1384).
    fn complete_bound_task_handoff(
        &self,
        key: ReceiptKey,
        expected_version: crate::receipt_ledger::ReceiptVersion,
        confirmed_task_bound: TaskBoundReceipt,
        deadline: Instant,
    ) -> Result<TaskBoundReceipt, ReceiptLedgerError> {
        let digest = crate::receipt_ledger::receipt_key_digest(&key);
        let key_lock = self.submit_fences.stripe(&digest);
        self.complete_bound_task_handoff_holding(
            &key_lock,
            key,
            expected_version,
            confirmed_task_bound,
            deadline,
        )
    }

    /// [`Self::complete_bound_task_handoff`] for an owner that already holds
    /// the key's lock across its re-read of the handoff and the Task
    /// materialization.
    fn complete_bound_task_handoff_holding(
        &self,
        _key_lock: &MutexGuard<'_, ()>,
        key: ReceiptKey,
        expected_version: crate::receipt_ledger::ReceiptVersion,
        confirmed_task_bound: TaskBoundReceipt,
        deadline: Instant,
    ) -> Result<TaskBoundReceipt, ReceiptLedgerError> {
        // Waiting for the key lock spends the caller's deadline. The Task is
        // already materialized, so its completion gets the daemon's own
        // durable step budget from now instead of an elapsed deadline.
        let deadline = committed_decision_step_deadline(deadline);
        self.receipt_ledger.complete_bound_task_handoff(
            key,
            expected_version,
            confirmed_task_bound,
            deadline,
        )
    }

    /// The staged counterpart of [`Self::complete_bound_task_handoff`].
    fn complete_staged_task_handoff(
        &self,
        key: ReceiptKey,
        expected_version: crate::receipt_ledger::ReceiptVersion,
        confirmed_terminal_bound: crate::receipt_ledger::TaskTerminalBoundReceipt,
        deadline: Instant,
    ) -> Result<crate::receipt_ledger::TaskTerminalBoundReceipt, ReceiptLedgerError> {
        let digest = crate::receipt_ledger::receipt_key_digest(&key);
        let key_lock = self.submit_fences.stripe(&digest);
        self.complete_staged_task_handoff_holding(
            &key_lock,
            key,
            expected_version,
            confirmed_terminal_bound,
            deadline,
        )
    }

    /// The staged counterpart of [`Self::complete_bound_task_handoff_holding`].
    fn complete_staged_task_handoff_holding(
        &self,
        _key_lock: &MutexGuard<'_, ()>,
        key: ReceiptKey,
        expected_version: crate::receipt_ledger::ReceiptVersion,
        confirmed_terminal_bound: crate::receipt_ledger::TaskTerminalBoundReceipt,
        deadline: Instant,
    ) -> Result<crate::receipt_ledger::TaskTerminalBoundReceipt, ReceiptLedgerError> {
        // Waiting for the key lock spends the caller's deadline. The Task is
        // already materialized, so its completion gets the daemon's own
        // durable step budget from now instead of an elapsed deadline.
        let deadline = committed_decision_step_deadline(deadline);
        self.receipt_ledger.complete_staged_task_handoff(
            key,
            expected_version,
            confirmed_terminal_bound,
            deadline,
        )
    }

    /// The handoff as the ledger holds it now. Read under the key's lock: a
    /// cancel of the exact key either marked it before (and the Task is
    /// materialized with that cancel) or waits until the key moved to its
    /// Task link (#1384).
    fn reread_handoff_under_lock(
        &self,
        _key_lock: &MutexGuard<'_, ()>,
        handoff: TaskHandoffActorBoundReceipt,
        deadline: Instant,
    ) -> Result<TaskHandoffActorBoundReceipt, ReceiptLedgerError> {
        match self
            .receipt_ledger
            .recover(handoff.key().clone(), deadline)?
        {
            ReceiptState::TaskHandoffActorBound(fresh) => Ok(fresh),
            _ => Ok(handoff),
        }
    }

    /// Materializes the Task of a committed handoff and moves the key from
    /// the ledger to the Task link. The key's lock is held from the re-read
    /// of the handoff to the completion: a cancel of the exact key either
    /// marked the handoff before (and the Task is materialized with that
    /// cancel) or waits and then finds the link (#1384, #1395). The lock is
    /// released before the Task starts, so from then on a cancel reaches the
    /// Task through its link. The handoff is already committed: neither the
    /// wait for the lock nor an earlier step that spent `deadline` may fail
    /// the steps that carry it out.
    fn complete_handoff_into_task(
        &self,
        handoff: TaskHandoffActorBoundReceipt,
        link_reservation: Option<TaskLinkReservation>,
        epoch_ms: u64,
        deadline: Instant,
        on_bound: impl FnOnce(&TaskBoundReceipt),
    ) -> Result<HandoffCompletion, ReceiptLedgerError> {
        let key_lock = self
            .submit_fences
            .stripe(&crate::receipt_ledger::receipt_key_digest(handoff.key()));
        let deadline = committed_decision_step_deadline(deadline);
        let handoff = self.reread_handoff_under_lock(&key_lock, handoff, deadline)?;
        let link_reservation = match link_reservation {
            Some(reservation) => reservation,
            None => self
                .task_projection
                .reserve_bound_handoff_link(&handoff, epoch_ms, deadline, self.hooks.as_ref())
                .map_err(|failure| self.project_task_failure(failure))?,
        };
        let (task_record, bound) = self
            .task_projection
            .materialize_staged_bound_handoff(
                &handoff,
                &link_reservation,
                epoch_ms,
                deadline,
                self.hooks.as_ref(),
            )
            .map_err(|failure| self.project_task_failure(failure))?;
        on_bound(&bound);
        if !matches!(
            handoff.terminal_stage(),
            HandoffTerminalStage::Staged { .. }
        ) {
            self.complete_bound_task_handoff_holding(
                &key_lock,
                handoff.key().clone(),
                handoff.record_version(),
                bound.clone(),
                deadline,
            )?;
            return Ok(HandoffCompletion::Bound(task_record, bound));
        }
        let (terminal_record, terminal_link) = self
            .task_projection
            .publish_staged_bound_task_terminal(
                &handoff,
                &bound,
                &task_record,
                deadline,
                self.hooks.as_ref(),
            )
            .map_err(|failure| self.project_task_failure(failure))?;
        self.hooks.event(
            V5ReceiptRuntimeEventKind::TaskStoreTerminalCommitted,
            epoch_ms,
        );
        self.hooks.event(
            V5ReceiptRuntimeEventKind::TaskStoreTerminalReadback,
            epoch_ms,
        );
        let terminal_link = self.complete_staged_task_handoff_holding(
            &key_lock,
            handoff.key().clone(),
            handoff.record_version(),
            terminal_link,
            deadline,
        )?;
        drop(key_lock);
        self.hooks.staged_terminal_publication(
            &handoff,
            &task_record,
            &terminal_record,
            &terminal_link,
        )?;
        self.hooks
            .terminal_bound_task(&terminal_record, &terminal_link);
        self.hooks.event(
            V5ReceiptRuntimeEventKind::TaskTerminalBoundCommitted,
            epoch_ms,
        );
        Ok(HandoffCompletion::Terminal(terminal_record))
    }

    /// Registers the cancellation token of a bound Task whose attempt is
    /// about to run, then reads the Task again. A cancel that reached the
    /// Task through its link before the token existed cancels the token at
    /// once instead of being lost (#1395). `true` when the Task carries a
    /// cancel: its attempt must not start.
    fn register_bound_task_cancellation(
        &self,
        task_record: &V5StoredInvocationRecord,
        deadline: Instant,
    ) -> Result<(CancellationToken, V5ActiveTaskCancellationGuard, bool), ReceiptLedgerError> {
        let (cancellation, guard) = self
            .active_task_cancellations
            .register(task_record.task_id)?;
        let cancel_requested = task_record.cancel_requested
            || self
                .task_projection
                .read_bound_task(
                    task_record.task_id,
                    committed_decision_step_deadline(deadline),
                )
                .map_err(|failure| self.project_task_failure(failure))?
                .is_some_and(|current| current.cancel_requested);
        if cancel_requested {
            cancellation.cancel();
        }
        Ok((cancellation, guard, cancel_requested))
    }

    /// Ends a bound Task whose cancel arrived before its attempt started:
    /// the attempt never runs, the Task publishes its cancelled terminal.
    fn cancel_unstarted_bound_task(
        &self,
        bound: &TaskBoundReceipt,
        task_id: unica_execution_contract::invocation::TaskId,
        cancellation_guard: V5ActiveTaskCancellationGuard,
        deadline: Instant,
    ) -> Result<ReceiptReply, ReceiptLedgerError> {
        let snapshot = self.publish_task_execution_outcome(
            bound,
            task_id,
            ReceiptTerminalOutcome::Cancelled,
            committed_decision_step_deadline(deadline),
        )?;
        drop(cancellation_guard);
        Ok(ReceiptReply::Json(ReceiptResponse::Invocation {
            outcome: V5InvocationResponse::Task { snapshot },
        }))
    }

    /// An exact repeat of a call whose receipt already passed into its Task
    /// reads that Task and runs nothing (#1384). A Task already being retired
    /// has no answer left to read.
    fn exact_bound_task_reply(
        &self,
        key: &ReceiptKey,
        deadline: Instant,
    ) -> Result<Option<ReceiptReply>, ReceiptLedgerError> {
        match self
            .task_projection
            .read_exact_bound_task(key, deadline)
            .map_err(|failure| self.project_task_failure(failure))?
        {
            ExactBoundTask::Absent => Ok(None),
            ExactBoundTask::Live(record) => {
                Ok(Some(ReceiptReply::Json(ReceiptResponse::Invocation {
                    outcome: V5InvocationResponse::Task {
                        snapshot: task_store_snapshot(&record),
                    },
                })))
            }
            ExactBoundTask::Retiring => Err(ReceiptLedgerError::ReceiptNotFound),
        }
    }

    pub fn ensure_named_authority(&self) -> Result<(), String> {
        self.ensure_named_authority_before(Instant::now() + AUTHORITY_ACQUIRE_TIMEOUT)
    }

    pub fn ensure_named_authority_before(&self, deadline: Instant) -> Result<(), String> {
        if self.external_store_fail_stop.load(Ordering::Acquire) {
            return Err("protocol-v5 external durable store requires reopen".to_owned());
        }
        // A probe the store already began is never cut short by a caller in
        // a hurry: overrunning it would latch fail-stop (#1387).
        let probe_deadline = deadline.max(Instant::now() + DURABLE_STEP_BUDGET);
        self.receipt_ledger
            .generation(probe_deadline)
            .map(|_| ())
            .map_err(|error| format!("validate protocol-v5 receipt authority: {error}"))
    }

    /// Whether the attempt on this thread must stop: the observer simulated
    /// the process's death, or the process latched fail-stop for real.
    fn attempt_is_dead(&self) -> bool {
        self.hooks.process_exited() || self.restart_required()
    }

    pub fn restart_required(&self) -> bool {
        self.receipt_ledger.restart_required()
            || self.external_store_fail_stop.load(Ordering::Acquire)
    }

    /// Fail-stop delivers its closed answer and nothing more: every attempt
    /// still running is asked to stop, whichever call latched it.
    pub fn cancel_active_attempts_after_fail_stop(&self) {
        self.active_task_cancellations.cancel_all();
    }

    fn project_task_failure(&self, failure: V5TaskProjectionFailure) -> ReceiptLedgerError {
        if failure.fail_stop {
            self.external_store_fail_stop.store(true, Ordering::Release);
        }
        failure.error
    }

    pub fn epoch_ms(&self) -> u64 {
        self.epoch_clock.now_epoch_millis()
    }

    pub fn cancel_invocation(
        &self,
        key: ReceiptKey,
        epoch_ms: u64,
        deadline: Instant,
    ) -> Result<ReceiptReply, ReceiptLedgerError> {
        let before = self.joins.capture_reply(key.reserved_task_id());
        let reply = self.cancel_invocation_canonical(key, epoch_ms, deadline)?;
        self.project_holder_reply(&before, reply, deadline)
    }

    fn cancel_invocation_canonical(
        &self,
        key: ReceiptKey,
        epoch_ms: u64,
        deadline: Instant,
    ) -> Result<ReceiptReply, ReceiptLedgerError> {
        self.validate_receipt_key(&key)?;
        let task_id = key.reserved_task_id();
        // Match the full live holder context before changing its interest.
        // Joined calls reserve no ledger row of their own.
        let holder_context = self.joins.read_context(task_id);
        if let Some(read) = &holder_context {
            if !read.holder.matches(&key) {
                return Err(ReceiptLedgerError::InvocationIdentityMismatch);
            }
            if read.leader == task_id {
                self.resolve_task_for_key(&key, deadline)?;
            }
        }
        let holder_cancel = match &holder_context {
            Some(read) => self
                .joins
                .cancel_holder_in_context(read, epoch_ms)
                .ok_or(ReceiptLedgerError::ReceiptNotFound)?,
            None => self.joins.cancel_holder(task_id, epoch_ms),
        };
        if holder_context
            .as_ref()
            .is_some_and(|read| read.leader != task_id)
        {
            self.cancel_task_with_decision(task_id, holder_cancel, deadline)?;
            return self.resolve_task_reply_for_key(&key, deadline);
        }
        if holder_cancel == HolderCancel::Released {
            return self.resolve_task_reply_for_key(&key, deadline);
        }
        /// What the cancel does once the key's lock is released.
        enum CancelStep {
            Reply(ReceiptReply),
            Existing(ReceiptState),
            ResolveTask(unica_execution_contract::invocation::TaskId),
            Marked(TaskCancellationReceipt),
        }
        // The key's lock orders the Task link lookup and the ledger writes
        // that reserve or mark the cancel against the handoff completion,
        // which moves the key from the ledger to the link under the same
        // lock: without it a cancel could miss both and leave a stray cancel
        // row beside the running Task, or lose the handoff row it is about to
        // mark (#1384). Only the replies, which read what the lock already
        // ordered, follow after it (#1395). Always taken before the terminal
        // gate, never inside it.
        let digest = crate::receipt_ledger::receipt_key_digest(&key);
        let step = {
            let _stripe = self.submit_fences.stripe(&digest);
            {
                let _task_terminal_gate = self
                    .task_terminal_coordinator
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if let Some(record) = self
                    .task_projection
                    .cancel_exact_bound_task(&key, deadline, self.hooks.as_ref())
                    .map_err(|failure| self.project_task_failure(failure))?
                {
                    // Исполнитель, который не ответит на отмену, держит только
                    // свою задачу: демон и соседние задачи продолжают работать.
                    self.active_task_cancellations.cancel(record.task_id);
                    return Ok(task_reply(&record));
                }
            }
            self.hooks
                .pause(V5PausePoint::BeforeCancelReservation, deadline)?;
            let resolution = self
                .receipt_ledger
                .request_cancel_or_reserve(key, epoch_ms, deadline)?;
            match decide_cancel_resolution(resolution) {
                CancelInvocationDecision::Accepted { receipt, .. } => {
                    CancelStep::Reply(ReceiptReply::Json(pending_cancel_response(&receipt)))
                }
                CancelInvocationDecision::ExistingDirectTerminal(receipt) => {
                    CancelStep::Existing(ReceiptState::DirectTerminalUnacked(receipt))
                }
                CancelInvocationDecision::Rejected(rejection) => {
                    let expected = match rejection.into_state() {
                        ReceiptState::TaskPromisedUnbound(receipt) => {
                            Ok(TaskCancellationReceipt::PromisedUnbound(receipt))
                        }
                        ReceiptState::TaskPromisedActorBound(receipt) => {
                            Ok(TaskCancellationReceipt::PromisedActorBound(receipt))
                        }
                        ReceiptState::TaskHandoffActorBound(receipt) => {
                            Ok(TaskCancellationReceipt::HandoffActorBound(receipt))
                        }
                        ReceiptState::TaskReceiptOwnedActorBound(receipt) => {
                            Ok(TaskCancellationReceipt::ReceiptOwnedActorBound(receipt))
                        }
                        ReceiptState::TaskTerminalReceiptBacked(receipt) => {
                            Err(CancelStep::ResolveTask(receipt.task().task_id()))
                        }
                        ReceiptState::Reserved(reserved)
                            if matches!(
                                reserved.phase(),
                                ReservedPhase::Begun { .. } | ReservedPhase::Unbound
                            ) =>
                        {
                            // A running inline attempt, or one still in
                            // validation or admission: signal its token. One
                            // that ignores it keeps only its own attempt; an
                            // attempt before its actor bind publishes its own
                            // cancelled terminal (#1251).
                            self.active_task_cancellations
                                .cancel(reserved.key().reserved_task_id());
                            Err(CancelStep::Existing(ReceiptState::Reserved(reserved)))
                        }
                        other => Err(CancelStep::Existing(other)),
                    };
                    match expected {
                        Err(step) => step,
                        Ok(expected) => {
                            let promised_unbound =
                                matches!(expected, TaskCancellationReceipt::PromisedUnbound(_));
                            let cancelled = self.receipt_ledger.request_task_cancel(
                                expected.key().clone(),
                                expected,
                                deadline,
                            )?;
                            if promised_unbound {
                                // Published under the lock as well: a second
                                // exact cancel finds the terminal, not a mark
                                // whose terminal epoch it would contest.
                                let task_id = cancelled.task().task_id();
                                let reserved_task_id = cancelled.key().reserved_task_id();
                                let terminal = crate::receipt_ledger::canonical_cancelled_terminal(
                                )
                                .map_err(|_| {
                                    ReceiptLedgerError::Corrupt("canonical v5 terminal failed")
                                })?;
                                let committed =
                                    self.receipt_ledger.publish_receipt_backed_task_terminal(
                                        cancelled.key().clone(),
                                        cancelled,
                                        epoch_ms,
                                        terminal,
                                        deadline,
                                    )?;
                                // The admission of this promised Task may
                                // still be running: once the terminal is
                                // durable its checkpoint observes this token
                                // and stops before the actor bind (#1251).
                                self.active_task_cancellations.cancel(reserved_task_id);
                                self.hooks.receipt_backed_terminal(&committed)?;
                                self.hooks.release_pre_actor_pauses();
                                self.hooks.event(
                                    V5ReceiptRuntimeEventKind::ReceiptTerminalCommitted,
                                    epoch_ms,
                                );
                                CancelStep::ResolveTask(task_id)
                            } else {
                                // An attempt already running toward this
                                // handoff learns of the cancel now; its Task
                                // is materialized with it (#1395).
                                self.active_task_cancellations
                                    .cancel(cancelled.key().reserved_task_id());
                                CancelStep::Marked(cancelled)
                            }
                        }
                    }
                }
            }
        };
        match step {
            CancelStep::Reply(reply) => Ok(reply),
            CancelStep::Existing(state) => self.reply_for_existing_state(state, deadline),
            CancelStep::ResolveTask(task_id) => {
                let snapshot = self.resolve_task(task_id, deadline)?;
                Ok(ReceiptReply::Json(ReceiptResponse::Invocation {
                    outcome: V5InvocationResponse::Task { snapshot },
                }))
            }
            CancelStep::Marked(cancelled) => {
                let snapshot = queued_receipt_task_snapshot(
                    cancelled.task(),
                    crate::receipt_ledger::receipt_key_digest(cancelled.key()),
                    cancelled.cancel_requested(),
                );
                Ok(ReceiptReply::Json(ReceiptResponse::Invocation {
                    outcome: V5InvocationResponse::Task { snapshot },
                }))
            }
        }
    }

    pub fn submit_invocation(
        self: &Arc<Self>,
        decoded: impl DecodedSubmit<H::Request>,
        epoch_ms: u64,
        deadline: Instant,
    ) -> Result<ReceiptReply, ReceiptLedgerError> {
        let task_id = decoded
            .reserved_task_id()
            .ok_or(ReceiptLedgerError::InvocationIdentityMismatch)?;
        let before = self.joins.capture_reply(task_id);
        let reply = self.submit_invocation_canonical(decoded, epoch_ms, deadline)?;
        self.project_holder_reply(&before, reply, deadline)
    }

    fn submit_invocation_canonical(
        self: &Arc<Self>,
        decoded: impl DecodedSubmit<H::Request>,
        epoch_ms: u64,
        deadline: Instant,
    ) -> Result<ReceiptReply, ReceiptLedgerError> {
        self.hooks
            .event(V5ReceiptRuntimeEventKind::V5ExecutorEntered, epoch_ms);
        // One opaque daemon-side deadline per request, captured before strict
        // validation and narrowed to the frontend budget: every later stage
        // measures against it and none of them starts a new clock.
        let response_budget_ms = decoded
            .response_budget_ms()
            .ok_or(ReceiptLedgerError::InvocationIdentityMismatch)?;
        let response_deadline = self
            .invocation_executor
            .capture_response_deadline(response_budget_ms);
        let strict = decoded
            .into_strict_submit(&self.core_identity_digest)
            .map_err(|_| ReceiptLedgerError::InvocationIdentityMismatch)?;
        let invocation = strict.invocation().clone();
        let (key, response_budget_ms) = strict.into_parts();
        let cutoff = OriginalCutoffDescriptor::new(epoch_ms, response_budget_ms)
            .map_err(|_| ReceiptLedgerError::TimestampOverflow)?;
        // A frame whose sender already gave up on it and fenced its key never
        // runs; nobody reads the answer to it.
        let digest = crate::receipt_ledger::receipt_key_digest(&key);
        if self.submit_fences.is_fenced(&digest) {
            return Err(ReceiptLedgerError::ReceiptNotFound);
        }
        if let Some(reply) =
            self.join_identical_read(&key, &invocation, &response_deadline, deadline)?
        {
            return Ok(reply);
        }
        let task_id = key.reserved_task_id();
        // Captured before the live-mutation check: a mutation submitted after
        // it keeps this read from leading (`register_leader`).
        let mutation_epoch = self.joins.mutation_epoch();
        let stripe = self.submit_fences.stripe(&digest);
        if self.submit_fences.is_fenced(&digest) {
            return Err(ReceiptLedgerError::ReceiptNotFound);
        }
        // Once its Task holds the call the ledger forgets the key; the link
        // is what remembers it. Looked up under the key's lock, which the
        // handoff completion also takes, so the row cannot leave between this
        // lookup and the reservation. The lock is one of 64 stripes: while it
        // is held — here one link read and one ledger write, by a handoff the
        // materialization of its Task — any key on the same stripe waits.
        observation::mark(
            "submit.exact_bound_lookup",
            "enter",
            Some(deadline.saturating_duration_since(Instant::now())),
        );
        if let Some(reply) = self.exact_bound_task_reply(&key, deadline)? {
            return Ok(reply);
        }
        observation::mark(
            "submit.exact_bound_lookup",
            "complete_no_bound_task",
            Some(deadline.saturating_duration_since(Instant::now())),
        );
        self.hooks
            .pause(V5PausePoint::BeforeSubmitReservation, deadline)?;
        observation::mark(
            "submit.reserve",
            "enter",
            Some(deadline.saturating_duration_since(Instant::now())),
        );
        let outcome = self.receipt_ledger.reserve(key.clone(), cutoff, deadline)?;
        observation::mark(
            "submit.reserve",
            "complete",
            Some(deadline.saturating_duration_since(Instant::now())),
        );
        drop(stripe);
        let created = matches!(&outcome, ReserveOutcome::Created(_));
        if created {
            self.hooks
                .event(V5ReceiptRuntimeEventKind::ReceiptReserved, epoch_ms);
            self.receipt_handles.insert(&key);
            if !tool_is_joinable(key.tool()) {
                self.joins.track_mutation(task_id);
            }
        }
        let decision = decide_cancel_reserved_submit(outcome).map_err(|_| {
            ReceiptLedgerError::Corrupt("canonical cancelled terminal could not be constructed")
        })?;
        match decision {
            CancelReservedSubmitDecision::ExecuteReserved(reservation) => {
                // Only an attempt that really runs can lead: a frame that
                // arrives cancelled ends at once and never settles as one.
                if created && tool_is_joinable(key.tool()) && !self.mutation_live(deadline) {
                    self.joins
                        .register_leader(&key, response_deadline.clone(), mutation_epoch);
                }
                let reply = self.execute_reserved_invocation(
                    reservation,
                    invocation,
                    response_deadline,
                    epoch_ms,
                    deadline,
                );
                // The handler answered: a Task is found in its stores from
                // now on, and a joiner waiting for this promotion may go on.
                if matches!(
                    &reply,
                    Ok(ReceiptReply::Json(ReceiptResponse::Invocation {
                        outcome: V5InvocationResponse::Task { .. }
                    }))
                ) {
                    self.receipt_handles.remove(task_id);
                }
                self.joins.leader_settled(task_id);
                reply
            }
            other => self.reply_for_cancel_submit_decision(
                other,
                epoch_ms,
                deadline,
                "exact_duplicate",
                "direct",
            ),
        }
    }

    /// The session handler owns the reply and the cutoff of a reserved
    /// invocation from the reservation on: a worker thread runs validation,
    /// admission, the durable transitions, prepare and execute, and the
    /// handler answers with whatever arrives first — the worker's reply or
    /// the seventh second. Before `Begun` the handler promotes the receipt
    /// itself (`TaskPromisedUnbound` or the actor-bound handoff intent) and
    /// answers with the projection; the worker then continues the single
    /// attempt into that Task. From `Begun` on the inline drive on the worker
    /// thread owns the cutoff, as before.
    fn execute_reserved_invocation(
        self: &Arc<Self>,
        reservation: crate::receipt_ledger::ReservedReceipt,
        invocation: H::Request,
        response_deadline: crate::invocation::InvocationResponseDeadline,
        epoch_ms: u64,
        deadline: Instant,
    ) -> Result<ReceiptReply, ReceiptLedgerError> {
        let slot = Arc::new(PipelineSlot::new());
        let worker = {
            let runtime = Arc::clone(self);
            let slot = Arc::clone(&slot);
            let reservation = reservation.clone();
            let response_deadline = response_deadline.clone();
            thread::Builder::new()
                .name("unica-v5-invocation-pipeline".to_owned())
                .spawn(move || {
                    // Until this attempt returns, it ends its Task itself.
                    let _live_attempt = runtime
                        .live_attempts
                        .enter(reservation.key().reserved_task_id());
                    let outcome = runtime.run_reserved_pipeline(
                        reservation.clone(),
                        invocation,
                        response_deadline,
                        &slot,
                        epoch_ms,
                        deadline,
                    );
                    if outcome.is_err() && slot.promoted_by_owner() {
                        runtime.settle_abandoned_promotion(&reservation, epoch_ms);
                    }
                    slot.report(match outcome {
                        Ok(reply) => PipelineReport::Reply(reply),
                        Err(error) => PipelineReport::Failed(error),
                    });
                    if slot.promoted_by_owner() {
                        runtime
                            .promoted_continuations
                            .fetch_sub(1, Ordering::AcqRel);
                    }
                })
                .map_err(|_| {
                    self.external_store_fail_stop.store(true, Ordering::Release);
                    ReceiptLedgerError::StoreUnavailable
                })?
        };
        let mut state = slot.lock();
        // The cutoff is answered once. A second `promote_at_cutoff` would only
        // enqueue another recover on the single ledger actor and delay the very
        // report it is waiting for.
        let mut cutoff_settled = false;
        loop {
            if let Some(report) = state.report.take() {
                let owned = !matches!(state.decision, PipelineDecision::PromotedByOwner);
                state.decision = PipelineDecision::HandlerOwns;
                drop(state);
                let _ = worker.join();
                return match report {
                    PipelineReport::Reply(reply) if owned => Ok(reply),
                    PipelineReport::Failed(error) if owned => Err(error),
                    // The handler already answered with the promotion; the
                    // worker's late reply belongs to a Task it published into.
                    PipelineReport::Reply(_) | PipelineReport::Failed(_) => {
                        Err(ReceiptLedgerError::Corrupt(
                            "promoted invocation reported a reply after its owner answered",
                        ))
                    }
                };
            }
            if worker.is_finished() {
                // The worker died without a report: the attempt failed and the
                // failure is what the caller learns.
                drop(state);
                let _ = worker.join();
                return Err(ReceiptLedgerError::StoreUnavailable);
            }
            if matches!(state.decision, PipelineDecision::PromotedByOwner)
                || state.begun
                || cutoff_settled
            {
                // Either the handler already answered, or the worker's drive
                // claimed the cutoff: nothing to time here, wait for the report.
                state = slot
                    .changed
                    .wait_timeout(state, INLINE_CUTOFF_POLL_INTERVAL)
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .0;
                continue;
            }
            let remaining = response_deadline.remaining_handoff_budget();
            if remaining.is_zero() {
                state.decision = PipelineDecision::HandoffInProgress;
                drop(state);
                let promotion = self.promote_at_cutoff(&reservation, &slot, epoch_ms);
                let mut next = slot.lock();
                match promotion {
                    Ok(Some(reply)) => {
                        // The worker continues this promoted attempt off the
                        // reply thread; the harness waits on the count to see
                        // the Task the runtime finishes for it. Count it before
                        // the decision becomes visible: the worker decrements as
                        // soon as it observes `PromotedByOwner`, and a decrement
                        // that overtook this increment would wrap the count.
                        self.promoted_continuations.fetch_add(1, Ordering::AcqRel);
                        next.decision = PipelineDecision::PromotedByOwner;
                        slot.changed.notify_all();
                        drop(next);
                        self.task_execution_threads
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .push(worker);
                        return Ok(reply);
                    }
                    Ok(None) => {
                        // The worker already ended the attempt: its reply is
                        // on the way, so stop timing the cutoff and wait.
                        cutoff_settled = true;
                        next.decision = PipelineDecision::Waiting;
                        slot.changed.notify_all();
                        state = next;
                        continue;
                    }
                    Err(error) => {
                        next.decision = PipelineDecision::HandlerOwns;
                        slot.changed.notify_all();
                        drop(next);
                        self.task_execution_threads
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .push(worker);
                        return Err(error);
                    }
                }
            }
            state = slot
                .changed
                .wait_timeout(state, remaining.min(INLINE_CUTOFF_POLL_INTERVAL))
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
    }

    /// The handler's promotion at the seventh second, by the durable phase the
    /// receipt is in: unbound — a promised Task with the fail-stop grace armed;
    /// actor-bound, or begun before the drive claimed the cutoff — the handoff
    /// intent, which the worker materializes and continues.
    fn promote_at_cutoff(
        &self,
        reservation: &crate::receipt_ledger::ReservedReceipt,
        slot: &PipelineSlot,
        epoch_ms: u64,
    ) -> Result<Option<ReceiptReply>, ReceiptLedgerError> {
        let deadline = Instant::now() + CUTOFF_HANDOFF_COMMIT_BUDGET;
        let cutoff_epoch_ms = self.epoch_ms();
        let current = self
            .receipt_ledger
            .recover(reservation.key().clone(), deadline)?;
        let reserved = match current {
            ReceiptState::Reserved(reserved) => reserved,
            // The worker already ended the attempt: its reply is on the way.
            _ => return Ok(None),
        };
        match reserved.phase() {
            ReservedPhase::Unbound => {
                let promised = self.receipt_ledger.promise_task_unbound(
                    reserved.key().clone(),
                    reserved.record_version(),
                    cutoff_epoch_ms,
                    DIRECT_TERMINAL_RETENTION_MS,
                    V5_TASK_POLL_INTERVAL_MS,
                    deadline,
                )?;
                self.hooks.event(
                    V5ReceiptRuntimeEventKind::UnboundPromiseCommitted,
                    cutoff_epoch_ms,
                );
                // Validation or admission still runs and continues into this
                // Task without a deadline; an explicit cancel stops it.
                let _ = epoch_ms;
                Ok(Some(ReceiptReply::Json(ReceiptResponse::Invocation {
                    outcome: V5InvocationResponse::Task {
                        snapshot: queued_receipt_task_snapshot(
                            promised.task(),
                            promised.key_digest().clone(),
                            promised.cancel_requested(),
                        ),
                    },
                })))
            }
            ReservedPhase::ActorBound { .. } => {
                let handoff = self.receipt_ledger.begin_bound_task_handoff(
                    reserved.key().clone(),
                    reserved.record_version(),
                    cutoff_epoch_ms,
                    DIRECT_TERMINAL_RETENTION_MS,
                    V5_TASK_POLL_INTERVAL_MS,
                    deadline,
                )?;
                self.hooks.event(
                    V5ReceiptRuntimeEventKind::BoundHandoffCommitted,
                    cutoff_epoch_ms,
                );
                Ok(Some(ReceiptReply::Json(ReceiptResponse::Invocation {
                    outcome: V5InvocationResponse::Task {
                        snapshot: queued_receipt_task_snapshot(
                            handoff.task(),
                            handoff.key_digest().clone(),
                            handoff.cancel_requested(),
                        ),
                    },
                })))
            }
            ReservedPhase::Begun { .. } => {
                let handoff = self.receipt_ledger.begin_bound_task_handoff(
                    reserved.key().clone(),
                    reserved.record_version(),
                    cutoff_epoch_ms,
                    DIRECT_TERMINAL_RETENTION_MS,
                    V5_TASK_POLL_INTERVAL_MS,
                    deadline,
                )?;
                self.hooks.event(
                    V5ReceiptRuntimeEventKind::BoundHandoffCommitted,
                    cutoff_epoch_ms,
                );
                if self.hooks.holds(V5PausePoint::BeforeTaskStoreCreate) {
                    // The observer holds the Task store create: the worker
                    // stages the terminal on release, so answer with the
                    // handoff's queued Task and defer materialization.
                    return Ok(Some(ReceiptReply::Json(ReceiptResponse::Invocation {
                        outcome: V5InvocationResponse::Task {
                            snapshot: queued_receipt_task_snapshot(
                                handoff.task(),
                                handoff.key_digest().clone(),
                                handoff.cancel_requested(),
                            ),
                        },
                    })));
                }
                // Materialize the Task now so a client polling at the cutoff
                // sees a working Task; the paused worker executes into it.
                let (task_record, task_bound) = match self.complete_handoff_into_task(
                    handoff,
                    None,
                    cutoff_epoch_ms,
                    deadline,
                    |_| {},
                )? {
                    HandoffCompletion::Bound(task_record, task_bound) => (task_record, task_bound),
                    HandoffCompletion::Terminal(terminal_record) => {
                        return Ok(Some(task_reply(&terminal_record)))
                    }
                };
                let (task_record, task_bound) = self
                    .task_projection
                    .start_bound_task(&task_bound, task_record, deadline)
                    .map_err(|failure| self.project_task_failure(failure))?;
                self.hooks.bound_task(&task_record, &task_bound);
                self.hooks.event(
                    V5ReceiptRuntimeEventKind::TaskBoundCommitted,
                    cutoff_epoch_ms,
                );
                let reply = ReceiptReply::Json(ReceiptResponse::Invocation {
                    outcome: V5InvocationResponse::Task {
                        snapshot: task_store_snapshot(&task_record),
                    },
                });
                slot.stash_owner_materialized(task_record, task_bound);
                Ok(Some(reply))
            }
        }
    }

    /// Executes the Task the session handler materialized when it promoted
    /// this begun attempt: nobody else runs it. A cancel that reached the
    /// Task before the attempt prepared ends it unstarted (#1395).
    fn execute_owner_materialized_task(
        self: &Arc<Self>,
        actor_bound: H::Bound,
        task_record: V5StoredInvocationRecord,
        bound: TaskBoundReceipt,
        invocation_id: unica_execution_contract::invocation::InvocationId,
        epoch_ms: u64,
    ) -> Result<ReceiptReply, ReceiptLedgerError> {
        let deadline = Self::continuation_deadline();
        let (cancellation, cancellation_guard, cancel_requested) =
            self.register_bound_task_cancellation(&task_record, deadline)?;
        if cancel_requested {
            return self.cancel_unstarted_bound_task(
                &bound,
                task_record.task_id,
                cancellation_guard,
                deadline,
            );
        }
        self.drive_prepared_bound_task(
            actor_bound,
            task_record,
            bound,
            invocation_id,
            cancellation,
            cancellation_guard,
            epoch_ms,
            deadline,
        )
    }

    /// A promoted attempt failed after the handler answered with its Task:
    /// no reply carries the error any more. A still-unbound promise gets a
    /// failed terminal; an actor-bound intermediate state stops the process
    /// so its successor terminalizes it, as the former watchdog did. Formerly
    /// that watchdog also caught this case; without it the Task would stay
    /// queued forever (#1251).
    fn settle_abandoned_promotion(
        &self,
        reservation: &crate::receipt_ledger::ReservedReceipt,
        epoch_ms: u64,
    ) {
        // A process that is already stopping leaves the attempt to its
        // successor's recovery, exactly like any interrupted attempt.
        if self.attempt_is_dead() {
            return;
        }
        let deadline = Self::continuation_deadline();
        let Ok(current) = self
            .receipt_ledger
            .recover(reservation.key().clone(), deadline)
        else {
            self.external_store_fail_stop.store(true, Ordering::Release);
            self.hooks.restart_requested();
            return;
        };
        match current {
            ReceiptState::TaskPromisedUnbound(_) => {
                let published = crate::receipt_ledger::canonical_failed_terminal(
                    V5SafeFailureReason::InvocationFailed,
                )
                .map_err(|_| ReceiptLedgerError::Corrupt("canonical v5 terminal failed"))
                .and_then(|terminal| {
                    self.publish_pre_actor_terminal(
                        reservation.clone(),
                        epoch_ms,
                        terminal,
                        deadline,
                    )
                });
                if published.is_err() {
                    self.external_store_fail_stop.store(true, Ordering::Release);
                    self.hooks.restart_requested();
                }
            }
            ReceiptState::TaskPromisedActorBound(_)
            | ReceiptState::TaskHandoffActorBound(_)
            | ReceiptState::TaskReceiptOwnedActorBound(_) => {
                self.external_store_fail_stop.store(true, Ordering::Release);
                self.hooks.restart_requested();
            }
            _ => {}
        }
    }

    /// The epoch a continuation binds its Task at. The handler created the
    /// Task at the handoff moment, after the submit was accepted; a bind
    /// stamped with the submit epoch would precede the Task it binds, which
    /// the lifecycle link rightly rejects as corrupt. Once admission may run
    /// past the handoff (#1251), this is the ordinary path, not a race.
    fn continuation_epoch_ms(&self, submit_epoch_ms: u64, task: &ReceiptTaskProjection) -> u64 {
        self.epoch_ms()
            .max(submit_epoch_ms)
            .max(task.created_at_epoch_ms())
    }

    /// A fresh bound for the durable steps of an attempt the handler already
    /// answered for: the operation budget of the submit is spent by definition.
    fn continuation_deadline() -> Instant {
        Instant::now() + TASK_TERMINAL_PUBLICATION_TIMEOUT
    }

    /// The pipeline of one reserved invocation on the worker thread.
    fn run_reserved_pipeline(
        self: &Arc<Self>,
        reservation: crate::receipt_ledger::ReservedReceipt,
        invocation: H::Request,
        response_deadline: crate::invocation::InvocationResponseDeadline,
        slot: &PipelineSlot,
        epoch_ms: u64,
        deadline: Instant,
    ) -> Result<ReceiptReply, ReceiptLedgerError> {
        self.hooks
            .event(V5ReceiptRuntimeEventKind::V5ReceiptRuntimeEntered, epoch_ms);
        self.hooks.event(
            V5ReceiptRuntimeEventKind::CanonicalV13ServiceEntered,
            epoch_ms,
        );
        self.hooks.stage_entered(V5Stage::Validation);
        self.hooks
            .event(V5ReceiptRuntimeEventKind::ValidationEntered, epoch_ms);
        self.hooks
            .pause(V5PausePoint::ValidationEntered, deadline)?;
        if self.hooks.observing() {
            let current = self
                .receipt_ledger
                .recover(reservation.key().clone(), deadline)?;
            if !reservation_is_still_unbound(&current) {
                return self.reply_for_existing_state(current, deadline);
            }
        }
        if self.hooks.validation_rejects() {
            let terminal = injected_rejection_terminal(
                "scenario validation rejected invocation",
                H::MAX_SUPPORTED_BODY_RESPONSE_BYTES,
            )?;
            return self.publish_pre_actor_terminal(reservation, epoch_ms, terminal, deadline);
        }
        self.hooks.stage_entered(V5Stage::Admission);
        self.hooks
            .event(V5ReceiptRuntimeEventKind::AdmissionEntered, epoch_ms);
        self.hooks.pause(V5PausePoint::AdmissionEntered, deadline)?;
        if self.hooks.observing() {
            let current = self
                .receipt_ledger
                .recover(reservation.key().clone(), deadline)?;
            if self.attempt_is_dead() || !reservation_is_still_unbound(&current) {
                return self.reply_for_existing_state(current, deadline);
            }
        }
        if let Some(rejection) = self.hooks.admission_rejection() {
            let fail_stop = matches!(rejection, V5AdmissionRejection::RegistryFailed);
            let outcome = match rejection {
                V5AdmissionRejection::Invalid => ReceiptTerminalOutcome::Completed {
                    result: Box::new(
                        unica_execution_contract::invocation::DomainResult::canonical_rejection(
                            None,
                            RefusalCode::BadValue,
                            "scenario workspace admission rejected invocation",
                        ),
                    ),
                },
                V5AdmissionRejection::Capacity => ReceiptTerminalOutcome::Failed {
                    reason: V5SafeFailureReason::WorkspaceCapacity,
                },
                V5AdmissionRejection::RegistryFailed => ReceiptTerminalOutcome::Failed {
                    reason: V5SafeFailureReason::WorkspaceRegistryFailed,
                },
            };
            return self.admission_failure_reply(
                reservation,
                outcome,
                fail_stop,
                epoch_ms,
                deadline,
            );
        }
        // The actor bind has no deadline: it may outlive the handoff moment
        // and continue into the Task the handler promised there. An explicit
        // cancel of that Task signals this token, and the admission
        // checkpoint stops the bind before any source operation begins.
        let (admission_cancellation, admission_cancellation_guard) = self
            .active_task_cancellations
            .register(reservation.key().reserved_task_id())?;
        let bound_actor =
            self.invocation_executor
                .bind(invocation, response_deadline, &admission_cancellation);
        drop(admission_cancellation_guard);
        if admission_cancellation.is_cancelled() {
            // Nothing of a cancelled attempt may begin. A Task cancel has
            // already published its terminal; an inline attempt cancelled
            // before the handoff, or promoted while it stopped, is ended here.
            let deadline = Self::continuation_deadline();
            let current = self
                .receipt_ledger
                .recover(reservation.key().clone(), deadline)?;
            if reservation_is_still_unbound(&current) {
                // Fail-stop stops the bind too, and that is no client cancel.
                let terminal = if self.restart_required() {
                    rejection_terminal(&Self::interrupted(), H::MAX_SUPPORTED_BODY_RESPONSE_BYTES)?
                } else {
                    crate::receipt_ledger::canonical_cancelled_terminal()
                        .map_err(|_| ReceiptLedgerError::Corrupt("canonical v5 terminal failed"))?
                };
                return self.publish_pre_actor_terminal(reservation, epoch_ms, terminal, deadline);
            }
            return self.reply_for_existing_state(current, deadline);
        }
        let deadline = if slot.promoted_by_owner() {
            Self::continuation_deadline()
        } else {
            deadline
        };
        let actor_bound = match bound_actor {
            Ok(actor_bound) => actor_bound,
            Err(error) => {
                let fail_stop = matches!(error, V5CanonicalPrepareError::WorkspaceRegistryFailed);
                let outcome = match error {
                    V5CanonicalPrepareError::Direct(result)
                    | V5CanonicalPrepareError::Rejected(result) => {
                        ReceiptTerminalOutcome::Completed { result }
                    }
                    V5CanonicalPrepareError::WorkspaceRegistryFailed => {
                        ReceiptTerminalOutcome::Failed {
                            reason: V5SafeFailureReason::WorkspaceRegistryFailed,
                        }
                    }
                };
                return self.admission_failure_reply(
                    reservation,
                    outcome,
                    fail_stop,
                    epoch_ms,
                    deadline,
                );
            }
        };
        let current = self
            .receipt_ledger
            .recover(reservation.key().clone(), deadline)?;
        if let ReceiptState::TaskPromisedUnbound(promised) = current {
            // The handler promised this Task at the cutoff; the submit's own
            // budget is spent, the continuation runs under its own bound.
            let deadline = if slot.promoted_by_owner() {
                Self::continuation_deadline()
            } else {
                deadline
            };
            return self.continue_promised(actor_bound, promised, &reservation, epoch_ms, deadline);
        }
        self.hooks
            .actor_workspace_identity(actor_bound.workspace_identity_hash());
        let bound = match self.receipt_ledger.bind_reserved_actor(
            reservation.key().clone(),
            reservation.record_version(),
            actor_bound.workspace_identity_hash().clone(),
            deadline,
        ) {
            Ok(bound) => bound,
            Err(error) if slot.promoted_by_owner() => {
                // The handler promised the Task while the actor was binding:
                // the durable state moved under us, continue into the promise.
                let deadline = Self::continuation_deadline();
                match self
                    .receipt_ledger
                    .recover(reservation.key().clone(), deadline)?
                {
                    ReceiptState::TaskPromisedUnbound(promised) => {
                        return self.continue_promised(
                            actor_bound,
                            promised,
                            &reservation,
                            epoch_ms,
                            deadline,
                        );
                    }
                    _ => return Err(error),
                }
            }
            Err(error) => return Err(error),
        };
        self.hooks
            .event(V5ReceiptRuntimeEventKind::ActorBoundCommitted, epoch_ms);
        self.hooks.pause(V5PausePoint::ActorBound, deadline)?;
        self.hooks
            .pause(V5PausePoint::BeforeReceiptBegun, deadline)?;
        if self.hooks.observing() {
            let current = self.receipt_ledger.recover(bound.key().clone(), deadline)?;
            if self.attempt_is_dead() {
                return self.reply_for_existing_state(current, deadline);
            }
            if let ReceiptState::TaskHandoffActorBound(handoff) = current {
                let deadline = if slot.promoted_by_owner() {
                    Self::continuation_deadline()
                } else {
                    deadline
                };
                return self.continue_handoff(
                    actor_bound,
                    handoff,
                    reservation.key().invocation_id(),
                    epoch_ms,
                    deadline,
                );
            }
        }
        let begun = match self.receipt_ledger.mark_reserved_begun(
            bound.key().clone(),
            bound.record_version(),
            deadline,
        ) {
            Ok(begun) => begun,
            Err(error) if slot.promoted_by_owner() => {
                // The handler committed the handoff intent at the cutoff while
                // this attempt was between actor bind and begun: materialize
                // its Task and continue into it.
                let deadline = Self::continuation_deadline();
                match self.receipt_ledger.recover(bound.key().clone(), deadline)? {
                    ReceiptState::TaskHandoffActorBound(handoff) => {
                        return self.continue_handoff(
                            actor_bound,
                            handoff,
                            reservation.key().invocation_id(),
                            epoch_ms,
                            deadline,
                        );
                    }
                    _ => return Err(error),
                }
            }
            Err(error) => return Err(error),
        };
        self.hooks
            .event(V5ReceiptRuntimeEventKind::ReceiptBegunCommitted, epoch_ms);
        self.hooks.pause(V5PausePoint::BeforePrepare, deadline)?;
        if self.hooks.observing() {
            let current = self.receipt_ledger.recover(begun.key().clone(), deadline)?;
            if self.attempt_is_dead() {
                return self.reply_for_existing_state(current, deadline);
            }
            if let ReceiptState::TaskHandoffActorBound(handoff) = current {
                // The handler promoted this begun attempt at the cutoff; the
                // submit budget is spent, the continuation runs under its own.
                return self.continue_promoted_begun_handoff(
                    handoff,
                    epoch_ms,
                    Self::continuation_deadline(),
                );
            }
            if !reservation_is_begun(&current) {
                return self.reply_for_existing_state(current, deadline);
            }
        }
        self.hooks.stage_entered(V5Stage::Prepare);
        self.hooks
            .event(V5ReceiptRuntimeEventKind::PrepareEntered, epoch_ms);
        self.hooks.pause(V5PausePoint::PrepareEntered, deadline)?;
        // The session handler may have materialized this Task at the begun
        // cutoff while the attempt was between `Begun` and the drive. Nobody
        // else executes it, so this attempt does — with or without an observer.
        // Looked for here before the observer re-reads the receipt; a Task the
        // handler stores only after this look is taken with the cutoff claim.
        if let Some((task_record, bound)) = slot.take_owner_materialized() {
            return self.execute_owner_materialized_task(
                actor_bound,
                task_record,
                bound,
                reservation.key().invocation_id(),
                epoch_ms,
            );
        }
        if self.hooks.observing() {
            let current = self.receipt_ledger.recover(begun.key().clone(), deadline)?;
            if self.attempt_is_dead() {
                return self.reply_for_existing_state(current, deadline);
            }
            if let ReceiptState::TaskHandoffActorBound(handoff) = current {
                // The handler promoted this begun attempt at the cutoff; the
                // submit budget is spent, the continuation runs under its own.
                return self.continue_promoted_begun_handoff(
                    handoff,
                    epoch_ms,
                    Self::continuation_deadline(),
                );
            }
            if !reservation_is_begun(&current) {
                return self.reply_for_existing_state(current, deadline);
            }
            if self.hooks.prepare_rejects() {
                let terminal = injected_rejection_terminal(
                    "scenario prepare rejected invocation",
                    H::MAX_SUPPORTED_BODY_RESPONSE_BYTES,
                )?;
                return self.publish_direct_terminal(begun, epoch_ms, terminal, deadline);
            }
        }
        self.hooks
            .pause(V5PausePoint::BeforeCutoffClaim, deadline)?;
        match slot.claim_cutoff() {
            CutoffClaim::Claimed => {}
            CutoffClaim::Promoted(Some((task_record, bound))) => {
                // The handler materialized the Task after the early look:
                // this attempt executes it, as if it had found it there.
                return self.execute_owner_materialized_task(
                    actor_bound,
                    task_record,
                    bound,
                    reservation.key().invocation_id(),
                    epoch_ms,
                );
            }
            CutoffClaim::Promoted(None) => {
                // The handler promoted this begun attempt at the cutoff before
                // the drive claimed it and left the Task to it: continue the
                // handoff the handler committed.
                let deadline = Self::continuation_deadline();
                return match self.receipt_ledger.recover(begun.key().clone(), deadline)? {
                    ReceiptState::TaskHandoffActorBound(handoff) => {
                        self.continue_promoted_begun_handoff(handoff, epoch_ms, deadline)
                    }
                    other => self.reply_for_existing_state(other, deadline),
                };
            }
        }
        let (prepared, direct_outcome) = match self.drive_inline_invocation(actor_bound, &begun)? {
            InlineDrive::Rejected(result) => {
                let terminal = rejection_terminal(
                    &ReceiptTerminalOutcome::Completed { result },
                    H::MAX_SUPPORTED_BODY_RESPONSE_BYTES,
                )?;
                return self.publish_direct_terminal(begun, epoch_ms, terminal, deadline);
            }
            InlineDrive::HandedOff(reply) => return Ok(*reply),
            InlineDrive::KnownLong(prepared) => (Some(prepared), None),
            InlineDrive::Direct(outcome) => (None, Some(outcome)),
        };
        if let Some(prepared) = prepared {
            let handoff = self.receipt_ledger.begin_bound_task_handoff(
                begun.key().clone(),
                begun.record_version(),
                epoch_ms,
                DIRECT_TERMINAL_RETENTION_MS,
                V5_TASK_POLL_INTERVAL_MS,
                deadline,
            )?;
            self.hooks
                .event(V5ReceiptRuntimeEventKind::BoundHandoffCommitted, epoch_ms);
            let link_reservation = self
                .task_projection
                .reserve_bound_handoff_link(&handoff, epoch_ms, deadline, self.hooks.as_ref())
                .map_err(|failure| self.project_task_failure(failure))?;
            self.hooks.pause(
                V5PausePoint::BeforeTaskStoreCreate,
                self.hooks
                    .commit_deadline_at(V5PausePoint::BeforeTaskStoreCreate, deadline),
            )?;
            let deadline = self
                .hooks
                .commit_deadline_at(V5PausePoint::BeforeTaskStoreCreate, deadline);
            // Another owner may have staged a terminal, or a cancel marked
            // this receipt, while the pause held: the Task is materialized
            // from the state committed under the key's lock.
            let (task_record, bound) = match self.complete_handoff_into_task(
                handoff,
                Some(link_reservation),
                epoch_ms,
                deadline,
                |_| {},
            )? {
                HandoffCompletion::Bound(task_record, bound) => (task_record, bound),
                HandoffCompletion::Terminal(terminal_record) => {
                    return Ok(task_reply(&terminal_record))
                }
            };
            let (cancellation, cancellation_guard, cancel_requested) =
                self.register_bound_task_cancellation(&task_record, deadline)?;
            let (task_record, bound) = self
                .task_projection
                .start_bound_task(&bound, task_record, deadline)
                .map_err(|failure| self.project_task_failure(failure))?;
            self.hooks.bound_task(&task_record, &bound);
            self.hooks
                .event(V5ReceiptRuntimeEventKind::TaskBoundCommitted, epoch_ms);
            if cancel_requested {
                return self.cancel_unstarted_bound_task(
                    &bound,
                    task_record.task_id,
                    cancellation_guard,
                    deadline,
                );
            }
            if self.hooks.holds(V5PausePoint::BeforeTaskTerminalReceipt)
                || self
                    .hooks
                    .holds(V5PausePoint::AfterTaskStoreTerminalBeforeLifecycleLinkTerminal)
            {
                // An observer holds the terminal receipt: the known-long attempt
                // runs on this thread so the pause sees its exact outcome.
                self.hooks
                    .pause(V5PausePoint::BeforeTaskTerminalReceipt, deadline)?;
                let outcome = self.provider_outcome(
                    prepared,
                    &cancellation,
                    epoch_ms,
                    Some(reservation.key().invocation_id()),
                );
                self.hooks
                    .event(V5ReceiptRuntimeEventKind::ResultSerialized, epoch_ms);
                let snapshot = self.publish_task_execution_outcome(
                    &bound,
                    task_record.task_id,
                    outcome,
                    deadline,
                )?;
                return Ok(ReceiptReply::Json(ReceiptResponse::Invocation {
                    outcome: V5InvocationResponse::Task { snapshot },
                }));
            }
            if task_record.task == V5StoredTask::Working {
                self.spawn_task_execution(
                    prepared,
                    bound,
                    task_record.clone(),
                    cancellation,
                    cancellation_guard,
                )?;
            } else {
                drop(cancellation_guard);
            }
            return Ok(ReceiptReply::Json(ReceiptResponse::Invocation {
                outcome: V5InvocationResponse::Task {
                    snapshot: task_store_snapshot(&task_record),
                },
            }));
        }
        let outcome = direct_outcome
            .expect("an inline drive without a handoff or a known-long class yields the outcome");
        if self.hooks.crash_after_side_effect() {
            return Err(ReceiptLedgerError::StoreUnavailable);
        }
        // An outcome that arrived on the last tick of the budget is still
        // published: the ledger command gets the serialization margin, not
        // an already spent operation deadline.
        let deadline = deadline.max(Instant::now() + RESPONSE_SERIALIZATION_MARGIN);
        let (terminal, oversized_result) =
            match canonical_v5_terminal(&outcome, H::MAX_SUPPORTED_BODY_RESPONSE_BYTES) {
                Ok(terminal) => (terminal, false),
                Err(CanonicalTerminalError::ResultTooLarge) => (
                    crate::receipt_ledger::canonical_oversized_terminal(
                        &outcome,
                        H::MAX_SUPPORTED_BODY_RESPONSE_BYTES,
                    )
                    .map_err(|_| {
                        ReceiptLedgerError::Corrupt("canonical result-too-large terminal failed")
                    })?,
                    true,
                ),
                Err(CanonicalTerminalError::Serialization) => {
                    return Err(ReceiptLedgerError::Corrupt(
                        "canonical v5 terminal serialization failed",
                    ))
                }
            };
        self.hooks
            .event(V5ReceiptRuntimeEventKind::ResultSerialized, epoch_ms);
        let oversized_candidate = match (&outcome, oversized_result) {
            (ReceiptTerminalOutcome::Completed { result }, true) => Some(result.as_ref()),
            _ => None,
        };
        self.publish_direct_terminal_with_candidate(
            begun,
            epoch_ms,
            terminal,
            deadline,
            oversized_candidate,
        )
    }

    /// Runs prepare and the inline execution of a begun receipt on a worker
    /// thread while the session handler owns the cutoff. An outcome before the
    /// handoff moment stays a Direct terminal; the handoff moment itself turns
    /// the receipt into a durable Task that the running worker completes —
    /// the seventh second divides direct and Task, no attempt is restarted.
    fn drive_inline_invocation(
        self: &Arc<Self>,
        actor_bound: H::Bound,
        begun: &crate::receipt_ledger::ReservedReceipt,
    ) -> Result<InlineDrive<H::Prepared>, ReceiptLedgerError> {
        let response_deadline = actor_bound.response_deadline();
        let slot = Arc::new(InlineExecutionSlot::new());
        // The running attempt is cancellable by its reserved Task identity
        // from `Begun` on; the cutoff hands the same registration to the Task.
        let (cancellation, guard) = self
            .active_task_cancellations
            .register(begun.key().reserved_task_id())?;
        let invocation_id = begun.key().invocation_id();
        let worker = {
            let runtime = Arc::clone(self);
            let slot = Arc::clone(&slot);
            let cancellation = cancellation.clone();
            thread::Builder::new()
                .name("unica-v5-inline-execution".to_owned())
                .spawn(move || {
                    runtime.run_inline_worker(actor_bound, slot, cancellation, invocation_id)
                })
                .map_err(|_| {
                    self.external_store_fail_stop.store(true, Ordering::Release);
                    ReceiptLedgerError::StoreUnavailable
                })?
        };
        let cutoff = response_deadline;

        let mut state = slot.lock();
        loop {
            if let Some(report) = state.report.take() {
                state.decision = InlineHandoffDecision::HandlerOwns;
                drop(state);
                let _ = worker.join();
                return Ok(match report {
                    InlineWorkerReport::Rejected(result) => InlineDrive::Rejected(result),
                    InlineWorkerReport::KnownLong(prepared) => InlineDrive::KnownLong(prepared),
                    InlineWorkerReport::Outcome(outcome) => InlineDrive::Direct(outcome),
                });
            }
            if worker.is_finished() {
                // The worker died without a report: the attempt failed and the
                // failure is the terminal, exactly as a returned error would be.
                state.decision = InlineHandoffDecision::HandlerOwns;
                drop(state);
                let _ = worker.join();
                return Ok(InlineDrive::Direct(ReceiptTerminalOutcome::Failed {
                    reason: V5SafeFailureReason::InvocationFailed,
                }));
            }
            let remaining = cutoff
                .as_ref()
                .map_or(INLINE_CUTOFF_POLL_INTERVAL, |deadline| {
                    deadline.remaining_handoff_budget()
                });
            if remaining.is_zero() {
                break;
            }
            let (next, _) = slot
                .changed
                .wait_timeout(state, remaining.min(INLINE_CUTOFF_POLL_INTERVAL))
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state = next;
        }

        // The cutoff: from here on the receipt is a Task and the reply is its
        // projection; the worker keeps the only attempt.
        state.decision = InlineHandoffDecision::HandoffInProgress;
        drop(state);
        let committed = self.commit_cutoff_handoff(begun, &slot);
        let mut state = slot.lock();
        match committed {
            Ok(CutoffCommit::Terminal(reply)) => {
                // The outcome arrived while the handoff committed and was
                // staged durably: the reply is the Task's terminal.
                state.decision = InlineHandoffDecision::HandlerOwns;
                slot.changed.notify_all();
                drop(state);
                let _ = worker.join();
                drop(guard);
                Ok(InlineDrive::HandedOff(Box::new(reply)))
            }
            Ok(CutoffCommit::Bound(bound, task_record)) => {
                let task_id = task_record.task_id;
                if let Some(report) = state.report.take() {
                    // The worker finished while the handoff was committing:
                    // the outcome goes into the Task right now.
                    state.decision = InlineHandoffDecision::HandlerOwns;
                    slot.changed.notify_all();
                    drop(state);
                    let _ = worker.join();
                    let outcome = match report {
                        InlineWorkerReport::Outcome(outcome) => outcome,
                        InlineWorkerReport::Rejected(result) => {
                            ReceiptTerminalOutcome::Completed { result }
                        }
                        InlineWorkerReport::KnownLong(prepared) => {
                            self.spawn_task_execution(
                                prepared,
                                bound,
                                task_record.clone(),
                                cancellation,
                                guard,
                            )?;
                            return Ok(InlineDrive::HandedOff(Box::new(ReceiptReply::Json(
                                ReceiptResponse::Invocation {
                                    outcome: V5InvocationResponse::Task {
                                        snapshot: task_store_snapshot(&task_record),
                                    },
                                },
                            ))));
                        }
                    };
                    let snapshot = self.publish_task_execution_outcome(
                        &bound,
                        task_id,
                        outcome,
                        Instant::now() + TASK_TERMINAL_PUBLICATION_TIMEOUT,
                    )?;
                    drop(guard);
                    return Ok(InlineDrive::HandedOff(Box::new(ReceiptReply::Json(
                        ReceiptResponse::Invocation {
                            outcome: V5InvocationResponse::Task { snapshot },
                        },
                    ))));
                }
                state.decision = InlineHandoffDecision::Task(Box::new(InlineTaskOwnership {
                    bound,
                    task_id,
                    guard: Some(guard),
                }));
                slot.changed.notify_all();
                drop(state);
                self.task_execution_threads
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(worker);
                Ok(InlineDrive::HandedOff(Box::new(ReceiptReply::Json(
                    ReceiptResponse::Invocation {
                        outcome: V5InvocationResponse::Task {
                            snapshot: task_store_snapshot(&task_record),
                        },
                    },
                ))))
            }
            Err(error) => {
                state.decision = InlineHandoffDecision::Abandoned;
                slot.changed.notify_all();
                drop(state);
                self.task_execution_threads
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(worker);
                Err(error)
            }
        }
    }

    /// The worker half of an inline invocation: prepare, then execute unless
    /// the class is known-long and the handler still waits for it.
    fn run_inline_worker(
        self: Arc<Self>,
        actor_bound: H::Bound,
        slot: Arc<InlineExecutionSlot<H::Prepared>>,
        cancellation: CancellationToken,
        invocation_id: unica_execution_contract::invocation::InvocationId,
    ) {
        if self.restart_required() {
            self.publish_inline_settlement(
                slot.settle(InlineWorkerReport::Outcome(Self::interrupted())),
                cancellation,
            );
            return;
        }
        let prepared = match actor_bound.prepare() {
            Ok(prepared) => prepared,
            Err(result) => {
                self.publish_inline_settlement(
                    slot.settle(InlineWorkerReport::Rejected(result)),
                    cancellation,
                );
                return;
            }
        };
        let prepared = if matches!(
            prepared.execution_class(),
            ports::ExecutionClass::KnownLong(_)
        ) {
            match slot.settle(InlineWorkerReport::KnownLong(prepared)) {
                InlineSettlement::HandlerOwns | InlineSettlement::Abandoned => return,
                // The cutoff already made the Task: prepare finished late and
                // this worker executes into it, as a known-long thread would.
                InlineSettlement::Task {
                    report: InlineWorkerReport::KnownLong(prepared),
                    ownership,
                } => {
                    let outcome = self.execute_inline(prepared, &cancellation, invocation_id);
                    let InlineTaskOwnership {
                        bound,
                        task_id,
                        guard,
                    } = *ownership;
                    self.publish_inline_outcome(&bound, task_id, outcome, guard);
                    return;
                }
                InlineSettlement::Task { .. } => return,
            }
        } else {
            prepared
        };
        let outcome = self.execute_inline(prepared, &cancellation, invocation_id);
        self.publish_inline_settlement(
            slot.settle(InlineWorkerReport::Outcome(outcome)),
            cancellation,
        );
    }

    fn execute_inline(
        &self,
        prepared: H::Prepared,
        cancellation: &CancellationToken,
        invocation_id: unica_execution_contract::invocation::InvocationId,
    ) -> ReceiptTerminalOutcome {
        self.provider_outcome(prepared, cancellation, self.epoch_ms(), Some(invocation_id))
    }

    /// The outcome of an attempt the process stopped on fail-stop.
    fn interrupted() -> ReceiptTerminalOutcome {
        ReceiptTerminalOutcome::Failed {
            reason: V5SafeFailureReason::Interrupted,
        }
    }

    /// Runs the provider and maps its result to the attempt's outcome. No
    /// subject work begins once the process latched fail-stop: the attempt
    /// then ends interrupted without entering the provider.
    fn provider_outcome(
        &self,
        prepared: H::Prepared,
        cancellation: &CancellationToken,
        epoch_ms: u64,
        invocation_id: Option<unica_execution_contract::invocation::InvocationId>,
    ) -> ReceiptTerminalOutcome {
        if self.restart_required() {
            return Self::interrupted();
        }
        self.hooks.stage_entered(V5Stage::Execute);
        self.hooks
            .event(V5ReceiptRuntimeEventKind::ExecuteEntered, epoch_ms);
        if let Some(invocation_id) = invocation_id {
            self.hooks.callback_invocation_id(invocation_id);
        }
        let result = prepared.execute(cancellation.clone());
        let authority = cancellation.factual_outcome_authority();
        if cancellation.is_cancelled() && authority == FactualOutcomeAuthority::Cancellable {
            // A stop the process asked for on fail-stop is no cancel the
            // client requested.
            if self.restart_required() {
                Self::interrupted()
            } else {
                ReceiptTerminalOutcome::Cancelled
            }
        } else {
            match result {
                Ok(result) => ReceiptTerminalOutcome::Completed {
                    result: Box::new(result),
                },
                Err(failure) => ReceiptTerminalOutcome::Failed {
                    reason: V5SafeFailureReason::of_execution_failure(&failure),
                },
            }
        }
    }

    /// A settlement the handler does not own goes into the Task by the worker.
    fn publish_inline_settlement(
        self: &Arc<Self>,
        settlement: InlineSettlement<H::Prepared>,
        cancellation: CancellationToken,
    ) {
        match settlement {
            InlineSettlement::HandlerOwns | InlineSettlement::Abandoned => {}
            InlineSettlement::Task { report, ownership } => {
                let InlineTaskOwnership {
                    bound,
                    task_id,
                    guard,
                } = *ownership;
                let outcome = match report {
                    InlineWorkerReport::Outcome(outcome) => outcome,
                    InlineWorkerReport::Rejected(result) => {
                        ReceiptTerminalOutcome::Completed { result }
                    }
                    InlineWorkerReport::KnownLong(prepared) => {
                        self.execute_inline(prepared, &cancellation, bound.key().invocation_id())
                    }
                };
                self.publish_inline_outcome(&bound, task_id, outcome, guard);
            }
        }
    }

    fn publish_inline_outcome(
        &self,
        bound: &TaskBoundReceipt,
        task_id: unica_execution_contract::invocation::TaskId,
        outcome: ReceiptTerminalOutcome,
        guard: Option<V5ActiveTaskCancellationGuard>,
    ) {
        let publication = self
            .publish_task_execution_outcome(
                bound,
                task_id,
                outcome,
                Instant::now() + TASK_TERMINAL_PUBLICATION_TIMEOUT,
            )
            .map(|_| ());
        if publication.is_err() {
            self.external_store_fail_stop.store(true, Ordering::Release);
        }
        drop(guard);
    }

    /// The durable handoff of a begun receipt at its cutoff: the same
    /// write-ahead intent, exact TaskStore record and `TaskBound` commit a
    /// known-long class gets, with the worker's own cancellation registered.
    fn commit_cutoff_handoff(
        &self,
        begun: &crate::receipt_ledger::ReservedReceipt,
        slot: &InlineExecutionSlot<H::Prepared>,
    ) -> Result<CutoffCommit, ReceiptLedgerError> {
        let epoch_ms = self.epoch_ms();
        let deadline = Instant::now() + CUTOFF_HANDOFF_COMMIT_BUDGET;
        let handoff = self.receipt_ledger.begin_bound_task_handoff(
            begun.key().clone(),
            begun.record_version(),
            epoch_ms,
            DIRECT_TERMINAL_RETENTION_MS,
            V5_TASK_POLL_INTERVAL_MS,
            deadline,
        )?;
        self.hooks
            .event(V5ReceiptRuntimeEventKind::BoundHandoffCommitted, epoch_ms);
        // An outcome that is already in the slot goes into the handoff
        // durably before any TaskStore row exists: a crash between the two
        // loses nothing, the successor publishes the staged terminal.
        let staged_outcome = {
            let mut state = slot.lock();
            match state.report.take() {
                Some(InlineWorkerReport::Outcome(outcome)) => Some(outcome),
                Some(other) => {
                    state.report = Some(other);
                    None
                }
                None => None,
            }
        };
        if let Some(outcome) = staged_outcome {
            let terminal =
                match canonical_v5_terminal(&outcome, H::MAX_SUPPORTED_BODY_RESPONSE_BYTES) {
                    Ok(terminal) => terminal,
                    Err(CanonicalTerminalError::ResultTooLarge) => {
                        crate::receipt_ledger::canonical_oversized_terminal(
                            &outcome,
                            H::MAX_SUPPORTED_BODY_RESPONSE_BYTES,
                        )
                        .map_err(|_| {
                            ReceiptLedgerError::Corrupt(
                                "canonical result-too-large terminal failed",
                            )
                        })?
                    }
                    Err(CanonicalTerminalError::Serialization) => {
                        return Err(ReceiptLedgerError::Corrupt(
                            "canonical v5 terminal serialization failed",
                        ))
                    }
                };
            let reply =
                self.publish_staged_handoff_terminal_reply(handoff, terminal, epoch_ms, deadline)?;
            return Ok(CutoffCommit::Terminal(reply));
        }
        let link_reservation = self
            .task_projection
            .reserve_bound_handoff_link(&handoff, epoch_ms, deadline, self.hooks.as_ref())
            .map_err(|failure| self.project_task_failure(failure))?;
        self.hooks.pause(
            V5PausePoint::BeforeTaskStoreCreate,
            self.hooks
                .commit_deadline_at(V5PausePoint::BeforeTaskStoreCreate, deadline),
        )?;
        let deadline = self
            .hooks
            .commit_deadline_at(V5PausePoint::BeforeTaskStoreCreate, deadline);
        let (task_record, bound) = match self.complete_handoff_into_task(
            handoff,
            Some(link_reservation),
            epoch_ms,
            deadline,
            |_| {},
        )? {
            HandoffCompletion::Bound(task_record, bound) => (task_record, bound),
            HandoffCompletion::Terminal(terminal_record) => {
                return Ok(CutoffCommit::Terminal(task_reply(&terminal_record)))
            }
        };
        let (task_record, bound) = self
            .task_projection
            .start_bound_task(&bound, task_record, deadline)
            .map_err(|failure| self.project_task_failure(failure))?;
        self.hooks.bound_task(&task_record, &bound);
        self.hooks
            .event(V5ReceiptRuntimeEventKind::TaskBoundCommitted, epoch_ms);
        Ok(CutoffCommit::Bound(bound, task_record))
    }

    /// The attempt after the handler promised its Task at the cutoff: bind
    /// the actor to the promise, materialize the Task and continue the single
    /// attempt into it. The handler already answered; the reply returned here
    /// is the Task's final projection and nobody reads it.
    fn continue_promised(
        self: &Arc<Self>,
        actor_bound: H::Bound,
        promised: TaskPromisedUnboundReceipt,
        reservation: &crate::receipt_ledger::ReservedReceipt,
        epoch_ms: u64,
        deadline: Instant,
    ) -> Result<ReceiptReply, ReceiptLedgerError> {
        let epoch_ms = self.continuation_epoch_ms(epoch_ms, promised.task());
        self.hooks
            .actor_workspace_identity(actor_bound.workspace_identity_hash());
        let actor_promised = self.receipt_ledger.bind_promised_task_actor(
            promised.key().clone(),
            promised.record_version(),
            actor_bound.workspace_identity_hash().clone(),
            deadline,
        )?;
        self.hooks
            .event(V5ReceiptRuntimeEventKind::ActorBoundCommitted, epoch_ms);
        self.hooks.pause(V5PausePoint::ActorBound, deadline)?;
        let handoff = self.receipt_ledger.begin_bound_task_handoff(
            actor_promised.key().clone(),
            actor_promised.record_version(),
            actor_promised.task().created_at_epoch_ms(),
            actor_promised.task().ttl_ms(),
            actor_promised.task().poll_interval_ms(),
            deadline,
        )?;
        self.hooks
            .event(V5ReceiptRuntimeEventKind::BoundHandoffCommitted, epoch_ms);
        let link_reservation = self
            .task_projection
            .reserve_bound_handoff_link(&handoff, epoch_ms, deadline, self.hooks.as_ref())
            .map_err(|failure| self.project_task_failure(failure))?;
        self.hooks.pause(
            V5PausePoint::BeforeTaskStoreCreate,
            self.hooks
                .commit_deadline_at(V5PausePoint::BeforeTaskStoreCreate, deadline),
        )?;
        let deadline = self
            .hooks
            .commit_deadline_at(V5PausePoint::BeforeTaskStoreCreate, deadline);
        // Another owner may have staged a terminal, or a cancel marked this
        // receipt, while the pause held: the Task is materialized from the
        // state committed under the key's lock.
        let (task_record, bound) = match self.complete_handoff_into_task(
            handoff,
            Some(link_reservation),
            epoch_ms,
            deadline,
            |bound| {
                self.hooks
                    .promised_actor_binding(&promised, &actor_promised, bound)
            },
        )? {
            HandoffCompletion::Bound(task_record, bound) => (task_record, bound),
            HandoffCompletion::Terminal(terminal_record) => {
                return Ok(task_reply(&terminal_record))
            }
        };
        self.continue_into_bound_task(
            actor_bound,
            task_record,
            bound,
            reservation.key().invocation_id(),
            epoch_ms,
            deadline,
        )
    }

    /// The attempt after the handler committed the actor-bound handoff intent
    /// at the cutoff: materialize the Task and continue the single attempt
    /// into it.
    fn continue_handoff(
        self: &Arc<Self>,
        actor_bound: H::Bound,
        handoff: TaskHandoffActorBoundReceipt,
        invocation_id: unica_execution_contract::invocation::InvocationId,
        epoch_ms: u64,
        deadline: Instant,
    ) -> Result<ReceiptReply, ReceiptLedgerError> {
        let epoch_ms = self.continuation_epoch_ms(epoch_ms, handoff.task());
        let (task_record, task_bound) =
            match self.complete_handoff_into_task(handoff, None, epoch_ms, deadline, |_| {})? {
                HandoffCompletion::Bound(task_record, task_bound) => (task_record, task_bound),
                HandoffCompletion::Terminal(terminal_record) => {
                    return Ok(task_reply(&terminal_record))
                }
            };
        self.continue_into_bound_task(
            actor_bound,
            task_record,
            task_bound,
            invocation_id,
            epoch_ms,
            deadline,
        )
    }

    /// One bound Task from `TaskBound` to its terminal, on the thread that
    /// holds the actor: start, begin, prepare, then execute here or on a
    /// known-long thread, and publish the outcome into the Task.
    fn continue_into_bound_task(
        self: &Arc<Self>,
        actor_bound: H::Bound,
        task_record: V5StoredInvocationRecord,
        bound: TaskBoundReceipt,
        invocation_id: unica_execution_contract::invocation::InvocationId,
        epoch_ms: u64,
        deadline: Instant,
    ) -> Result<ReceiptReply, ReceiptLedgerError> {
        // The handoff is committed: the rest of the attempt is no longer
        // bounded by what its request had left.
        let deadline = committed_decision_step_deadline(deadline);
        self.hooks.bound_task(&task_record, &bound);
        self.hooks
            .event(V5ReceiptRuntimeEventKind::TaskBoundCommitted, epoch_ms);
        let authorized_bound =
            if !task_record.cancel_requested && task_record.task == V5StoredTask::Queued {
                self.task_projection
                    .authorize_not_begun_bound_task_start(&bound, &task_record, deadline)
                    .map_err(|failure| self.project_task_failure(failure))?
            } else {
                bound
            };
        self.hooks.bound_task(&task_record, &authorized_bound);
        if !task_record.cancel_requested && task_record.task == V5StoredTask::Queued {
            self.hooks.event(
                V5ReceiptRuntimeEventKind::FalseCancelObservationReached,
                epoch_ms,
            );
            self.hooks
                .pause(V5PausePoint::AfterFalseCancelObservation, deadline)?;
        }
        let lifecycle_gate_acquired = if self.hooks.holds(V5PausePoint::AfterWorkingReadback) {
            self.hooks.acquire_lifecycle_gate("submit", deadline)?;
            true
        } else {
            false
        };
        let (task_record, bound) = self
            .task_projection
            .start_not_begun_bound_task(&authorized_bound, task_record, deadline)
            .map_err(|failure| self.project_task_failure(failure))?;
        let mut task_record = task_record;
        if task_record.task == V5StoredTask::Working {
            self.hooks.event(
                V5ReceiptRuntimeEventKind::TaskStoreWorkingReadback,
                epoch_ms,
            );
        }
        self.hooks.bound_task(&task_record, &bound);
        if task_record.task == V5StoredTask::Working && bound.phase() == AttemptPhase::NotBegun {
            self.hooks
                .pause(V5PausePoint::AfterWorkingReadback, deadline)?;
            self.hooks
                .pause(V5PausePoint::BeforeReceiptBegun, deadline)?;
            if self.hooks.process_exited() {
                let current = self.receipt_ledger.recover(bound.key().clone(), deadline)?;
                return self.reply_for_existing_state(current, deadline);
            }
        }
        let bound = if task_record.task == V5StoredTask::Working
            && bound.phase() == AttemptPhase::NotBegun
        {
            let begun = self
                .task_projection
                .mark_not_begun_bound_task_begun(&bound, &task_record, deadline)
                .map_err(|failure| self.project_task_failure(failure))?;
            self.hooks
                .event(V5ReceiptRuntimeEventKind::ReceiptBegunCommitted, epoch_ms);
            self.hooks
                .event(V5ReceiptRuntimeEventKind::TokenSignalled, epoch_ms);
            self.hooks
                .bound_task_start_authorization(&authorized_bound, &task_record, &begun);
            self.hooks.bound_task(&task_record, &begun);
            begun
        } else {
            bound
        };
        if lifecycle_gate_acquired {
            self.hooks.release_lifecycle_gate("submit");
            self.hooks.wait_for_gate_cancel(deadline)?;
            if let Some(cancelled) = self
                .task_projection
                .cancel_exact_bound_task(bound.key(), deadline, self.hooks.as_ref())
                .map_err(|failure| self.project_task_failure(failure))?
            {
                task_record = cancelled;
                self.hooks.bound_task(&task_record, &bound);
            }
        }
        if task_record.cancel_requested {
            let terminal = crate::receipt_ledger::canonical_cancelled_terminal()
                .map_err(|_| ReceiptLedgerError::Corrupt("canonical v5 terminal failed"))?;
            // The cancel may have reached the Task after this continuation
            // took its epoch: the terminal never precedes it (#1395).
            let terminal_epoch_ms = self
                .epoch_ms()
                .max(epoch_ms)
                .max(task_record.updated_at_epoch_ms);
            let (terminal_record, terminal_link) = self
                .task_projection
                .publish_bound_task_terminal(
                    &bound,
                    &task_record,
                    &terminal,
                    terminal_epoch_ms,
                    deadline,
                    self.hooks.as_ref(),
                )
                .map_err(|failure| self.project_task_failure(failure))?;
            self.hooks.event(
                V5ReceiptRuntimeEventKind::TaskStoreTerminalCommitted,
                epoch_ms,
            );
            self.hooks.event(
                V5ReceiptRuntimeEventKind::TaskStoreTerminalReadback,
                epoch_ms,
            );
            self.hooks.event(
                V5ReceiptRuntimeEventKind::TaskTerminalBoundCommitted,
                epoch_ms,
            );
            self.hooks
                .terminal_bound_task(&terminal_record, &terminal_link);
            return Ok(ReceiptReply::Json(ReceiptResponse::Invocation {
                outcome: V5InvocationResponse::Task {
                    snapshot: task_store_snapshot(&terminal_record),
                },
            }));
        }
        let (cancellation, cancellation_guard, cancel_requested) =
            self.register_bound_task_cancellation(&task_record, deadline)?;
        if cancel_requested {
            // A cancel reached the Task through its link after it started
            // and before this token existed: the attempt does not run (#1395).
            return self.cancel_unstarted_bound_task(
                &bound,
                task_record.task_id,
                cancellation_guard,
                deadline,
            );
        }
        self.hooks.pause(V5PausePoint::BeforePrepare, deadline)?;
        self.hooks.stage_entered(V5Stage::Prepare);
        self.hooks
            .event(V5ReceiptRuntimeEventKind::PrepareEntered, epoch_ms);
        self.hooks.pause(V5PausePoint::PrepareEntered, deadline)?;
        self.drive_prepared_bound_task(
            actor_bound,
            task_record,
            bound,
            invocation_id,
            cancellation,
            cancellation_guard,
            epoch_ms,
            deadline,
        )
    }

    /// Prepare and execute a begun bound Task on the thread that holds the
    /// actor. The `PrepareEntered` pause has already run; the caller registers
    /// the cancellation token so a cancel during that pause is observed.
    #[allow(clippy::too_many_arguments)]
    fn drive_prepared_bound_task(
        self: &Arc<Self>,
        actor_bound: H::Bound,
        task_record: V5StoredInvocationRecord,
        bound: TaskBoundReceipt,
        invocation_id: unica_execution_contract::invocation::InvocationId,
        cancellation: CancellationToken,
        cancellation_guard: V5ActiveTaskCancellationGuard,
        epoch_ms: u64,
        deadline: Instant,
    ) -> Result<ReceiptReply, ReceiptLedgerError> {
        if self.hooks.prepare_rejects() {
            let terminal = injected_rejection_terminal(
                "scenario prepare rejected invocation",
                H::MAX_SUPPORTED_BODY_RESPONSE_BYTES,
            )?;
            return self.publish_bound_terminal_reply(
                &bound,
                &task_record,
                &terminal,
                epoch_ms,
                deadline,
            );
        }
        if self.restart_required() {
            let terminal =
                rejection_terminal(&Self::interrupted(), H::MAX_SUPPORTED_BODY_RESPONSE_BYTES)?;
            return self.publish_bound_terminal_reply(
                &bound,
                &task_record,
                &terminal,
                epoch_ms,
                deadline,
            );
        }
        let prepared = match actor_bound.prepare() {
            Ok(prepared) => prepared,
            Err(result) => {
                let terminal = rejection_terminal(
                    &ReceiptTerminalOutcome::Completed { result },
                    H::MAX_SUPPORTED_BODY_RESPONSE_BYTES,
                )?;
                return self.publish_bound_terminal_reply(
                    &bound,
                    &task_record,
                    &terminal,
                    epoch_ms,
                    Self::continuation_deadline(),
                );
            }
        };
        if self.hooks.holds(V5PausePoint::BeforeTaskTerminalReceipt)
            || self
                .hooks
                .holds(V5PausePoint::AfterTaskStoreTerminalBeforeLifecycleLinkTerminal)
        {
            // An observer holds the terminal receipt: the promoted attempt
            // runs on this thread so the pause sees its exact outcome.
            self.hooks
                .pause(V5PausePoint::BeforeTaskTerminalReceipt, deadline)?;
            let outcome =
                self.provider_outcome(prepared, &cancellation, epoch_ms, Some(invocation_id));
            if self.hooks.crash_after_side_effect() {
                return Err(ReceiptLedgerError::StoreUnavailable);
            }
            self.hooks
                .event(V5ReceiptRuntimeEventKind::ResultSerialized, epoch_ms);
            let snapshot = self.publish_task_execution_outcome(
                &bound,
                task_record.task_id,
                outcome,
                Self::continuation_deadline(),
            )?;
            drop(cancellation_guard);
            return Ok(ReceiptReply::Json(ReceiptResponse::Invocation {
                outcome: V5InvocationResponse::Task { snapshot },
            }));
        }
        // A known-long attempt runs on its own thread: the client already
        // holds the Task reply, so the continuation need not wait for it.
        if matches!(
            prepared.execution_class(),
            ports::ExecutionClass::KnownLong(_)
        ) {
            if task_record.task == V5StoredTask::Working {
                self.spawn_task_execution(
                    prepared,
                    bound,
                    task_record.clone(),
                    cancellation,
                    cancellation_guard,
                )?;
            } else {
                drop(cancellation_guard);
            }
            return Ok(ReceiptReply::Json(ReceiptResponse::Invocation {
                outcome: V5InvocationResponse::Task {
                    snapshot: task_store_snapshot(&task_record),
                },
            }));
        }
        // A direct attempt promoted to a Task runs to its terminal on this
        // continuation thread; the client already holds the Task reply.
        let outcome = self.provider_outcome(prepared, &cancellation, epoch_ms, Some(invocation_id));
        if self.hooks.crash_after_side_effect() {
            return Err(ReceiptLedgerError::StoreUnavailable);
        }
        self.hooks
            .event(V5ReceiptRuntimeEventKind::ResultSerialized, epoch_ms);
        // The terminal bound starts when the outcome is ready, not when the
        // attempt was prepared: an execution may run far longer than the
        // per-step publication bound (#1251).
        let snapshot = self.publish_task_execution_outcome(
            &bound,
            task_record.task_id,
            outcome,
            Self::continuation_deadline(),
        );
        drop(cancellation_guard);
        snapshot.map(|snapshot| {
            ReceiptReply::Json(ReceiptResponse::Invocation {
                outcome: V5InvocationResponse::Task { snapshot },
            })
        })
    }

    fn spawn_task_execution(
        self: &Arc<Self>,
        prepared: H::Prepared,
        bound: TaskBoundReceipt,
        task_record: V5StoredInvocationRecord,
        cancellation: CancellationToken,
        cancellation_guard: V5ActiveTaskCancellationGuard,
    ) -> Result<(), ReceiptLedgerError> {
        let runtime = Arc::clone(self);
        let execution = thread::Builder::new()
            .name("unica-v5-task-execution".to_owned())
            .spawn(move || {
                let outcome =
                    runtime.provider_outcome(prepared, &cancellation, runtime.epoch_ms(), None);
                let publication = runtime
                    .publish_task_execution_outcome(
                        &bound,
                        task_record.task_id,
                        outcome,
                        Instant::now() + TASK_TERMINAL_PUBLICATION_TIMEOUT,
                    )
                    .map(|_| ());
                if publication.is_err() {
                    runtime
                        .external_store_fail_stop
                        .store(true, Ordering::Release);
                }
                drop(cancellation_guard);
            })
            .map_err(|_| {
                self.external_store_fail_stop.store(true, Ordering::Release);
                ReceiptLedgerError::StoreUnavailable
            })?;
        let finished = {
            let mut executions = self
                .task_execution_threads
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let (finished, mut running): (Vec<_>, Vec<_>) = std::mem::take(&mut *executions)
                .into_iter()
                .partition(thread::JoinHandle::is_finished);
            running.push(execution);
            *executions = running;
            finished
        };
        for execution in finished {
            let _ = execution.join();
        }
        Ok(())
    }

    fn publish_task_execution_outcome(
        &self,
        expected_bound: &TaskBoundReceipt,
        task_id: unica_execution_contract::invocation::TaskId,
        candidate: ReceiptTerminalOutcome,
        deadline: Instant,
    ) -> Result<response::V5DaemonTaskSnapshot, ReceiptLedgerError> {
        let _terminal_gate = self
            .task_terminal_coordinator
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let record = self
            .task_projection
            .read_bound_task(task_id, deadline)
            .map_err(|failure| self.project_task_failure(failure))?
            .ok_or(ReceiptLedgerError::ReceiptNotFound)?;
        if record.task.is_terminal() {
            return Ok(task_store_snapshot(&record));
        }
        let provider_deadline =
            unica_work_control::provider_deadline::ProviderDeadline::new(deadline);
        let bound = match self
            .task_projection
            .lifecycle_links
            .read_by_task_id(task_id, provider_deadline)
            .map_err(|error| {
                self.project_task_failure(V5TaskProjectionFailure::from_link_store(error))
            })? {
            TaskLifecycleLinkRecord::TaskBound(bound) if bound.key() == expected_bound.key() => {
                bound
            }
            TaskLifecycleLinkRecord::TaskTerminalBound(_) => {
                return Ok(task_store_snapshot(&record))
            }
            _ => return Err(ReceiptLedgerError::TaskBoundMismatch),
        };
        let outcome = task_outcome_after_cancel(
            candidate,
            record.cancel_requested,
            self.active_task_cancellations.outcome_authority(task_id),
        );
        let terminal = match canonical_v5_terminal(&outcome, H::MAX_SUPPORTED_BODY_RESPONSE_BYTES) {
            Ok(terminal) => terminal,
            Err(CanonicalTerminalError::ResultTooLarge) => {
                crate::receipt_ledger::canonical_oversized_terminal(
                    &outcome,
                    H::MAX_SUPPORTED_BODY_RESPONSE_BYTES,
                )
                .map_err(|_| {
                    ReceiptLedgerError::Corrupt(
                        "canonical background result-too-large terminal failed",
                    )
                })?
            }
            Err(CanonicalTerminalError::Serialization) => {
                return Err(ReceiptLedgerError::Corrupt(
                    "canonical background terminal failed",
                ))
            }
        };
        let reply = self.publish_bound_terminal_reply(
            &bound,
            &record,
            &terminal,
            self.epoch_ms(),
            deadline,
        )?;
        match reply {
            ReceiptReply::Json(ReceiptResponse::Invocation {
                outcome: V5InvocationResponse::Task { snapshot },
            }) => Ok(snapshot),
            _ => Err(ReceiptLedgerError::Corrupt(
                "Task terminal publication returned a non-Task reply",
            )),
        }
    }

    pub fn join_task_executions(&self) {
        let executions = std::mem::take(
            &mut *self
                .task_execution_threads
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        for execution in executions {
            if execution.thread().id() != thread::current().id() {
                let _ = execution.join();
            }
        }
    }

    fn publish_direct_terminal(
        &self,
        reservation: crate::receipt_ledger::ReservedReceipt,
        epoch_ms: u64,
        terminal: crate::receipt_ledger::V5CanonicalTerminal,
        deadline: Instant,
    ) -> Result<ReceiptReply, ReceiptLedgerError> {
        self.publish_direct_terminal_with_candidate(reservation, epoch_ms, terminal, deadline, None)
    }

    fn publish_direct_terminal_with_candidate(
        &self,
        reservation: crate::receipt_ledger::ReservedReceipt,
        epoch_ms: u64,
        terminal: crate::receipt_ledger::V5CanonicalTerminal,
        deadline: Instant,
        oversized_candidate: Option<&unica_execution_contract::invocation::DomainResult>,
    ) -> Result<ReceiptReply, ReceiptLedgerError> {
        if self
            .hooks
            .store_fault(V5StoreFaultPoint::AfterTerminalPayloadRenameBeforeDirectorySync)
        {
            arm_receipt_row_directory_sync_fault();
        }
        let publication = self.receipt_ledger.publish_direct_terminal(
            reservation.key().clone(),
            reservation.record_version(),
            epoch_ms,
            terminal,
            deadline,
        )?;
        self.hooks.event(
            V5ReceiptRuntimeEventKind::ReceiptTerminalCommitted,
            epoch_ms,
        );
        self.hooks
            .event(V5ReceiptRuntimeEventKind::FinalResultProjected, epoch_ms);
        self.hooks.direct_publication(
            &publication,
            "direct",
            "immediate_publication",
            oversized_candidate,
        );
        Ok(ReceiptReply::Prepared(publication.into_parts().1))
    }

    fn publish_bound_terminal_reply(
        &self,
        bound: &TaskBoundReceipt,
        task_record: &V5StoredInvocationRecord,
        terminal: &crate::receipt_ledger::V5CanonicalTerminal,
        epoch_ms: u64,
        deadline: Instant,
    ) -> Result<ReceiptReply, ReceiptLedgerError> {
        let (terminal_record, terminal_link) = self
            .task_projection
            .publish_bound_task_terminal(
                bound,
                task_record,
                terminal,
                epoch_ms,
                deadline,
                self.hooks.as_ref(),
            )
            .map_err(|failure| self.project_task_failure(failure))?;
        self.hooks.event(
            V5ReceiptRuntimeEventKind::TaskStoreTerminalCommitted,
            epoch_ms,
        );
        self.hooks.event(
            V5ReceiptRuntimeEventKind::TaskStoreTerminalReadback,
            epoch_ms,
        );
        self.hooks.event(
            V5ReceiptRuntimeEventKind::TaskTerminalBoundCommitted,
            epoch_ms,
        );
        self.hooks.bound_terminal_publication(
            bound,
            task_record,
            &terminal_record,
            &terminal_link,
            terminal,
            epoch_ms,
        )?;
        self.hooks
            .terminal_bound_task(&terminal_record, &terminal_link);
        Ok(ReceiptReply::Json(ReceiptResponse::Invocation {
            outcome: V5InvocationResponse::Task {
                snapshot: task_store_snapshot(&terminal_record),
            },
        }))
    }

    fn reconcile_staged_handoff_startup(
        &self,
        staged: TaskHandoffActorBoundReceipt,
        deadline: Instant,
    ) -> Result<ReceiptReply, ReceiptLedgerError> {
        let catalog = self
            .task_projection
            .lifecycle_links
            .catalog_snapshot(
                unica_work_control::provider_deadline::ProviderDeadline::new(deadline),
            )
            .map_err(|error| {
                self.project_task_failure(V5TaskProjectionFailure::from_link_store(error))
            })?;
        let plan = self
            .task_projection
            .staged_handoff_startup_plan(&staged, catalog.entries(), deadline)
            .map_err(|failure| self.project_task_failure(failure))?;
        match plan {
            None => {
                let HandoffTerminalStage::Staged {
                    terminal,
                    terminal_epoch_ms,
                    ..
                } = staged.terminal_stage()
                else {
                    return Err(ReceiptLedgerError::TaskBoundMismatch);
                };
                let terminal = terminal.clone();
                let terminal_epoch_ms = *terminal_epoch_ms;
                self.publish_staged_handoff_terminal_reply(
                    staged,
                    terminal,
                    terminal_epoch_ms,
                    deadline,
                )
            }
            Some(StagedHandoffStartupPlan::TaskBound {
                bound,
                provisional,
                observed,
            }) => {
                let (terminal_record, terminal_link) = self
                    .task_projection
                    .publish_staged_bound_task_terminal(
                        &staged,
                        &bound,
                        &provisional,
                        deadline,
                        self.hooks.as_ref(),
                    )
                    .map_err(|failure| self.project_task_failure(failure))?;
                // An existing successor was confirmed by readback. Do not emit
                // the hook that describes replacement of a provisional Task.
                let provisional_write = (observed == provisional).then_some(&provisional);
                self.complete_staged_handoff_reply(
                    staged,
                    provisional_write,
                    terminal_record,
                    terminal_link,
                    deadline,
                )
            }
            Some(StagedHandoffStartupPlan::TaskTerminalBound {
                terminal_link,
                observed,
            }) => {
                self.complete_staged_handoff_reply(staged, None, observed, terminal_link, deadline)
            }
        }
    }

    fn publish_staged_handoff_terminal_reply(
        &self,
        handoff: TaskHandoffActorBoundReceipt,
        terminal: crate::receipt_ledger::V5CanonicalTerminal,
        epoch_ms: u64,
        deadline: Instant,
    ) -> Result<ReceiptReply, ReceiptLedgerError> {
        let certificate = canonical_staged_transfer_certificate(
            handoff.key(),
            handoff.key_digest(),
            handoff.link(),
            epoch_ms,
            &terminal,
        )?;
        let staged = self.receipt_ledger.stage_bound_task_handoff_terminal(
            handoff.key().clone(),
            handoff.record_version(),
            epoch_ms,
            terminal.clone(),
            certificate,
            deadline,
        )?;
        self.hooks.staged_terminal_preparation(&staged)?;
        self.hooks.event(
            V5ReceiptRuntimeEventKind::BoundHandoffTerminalStaged,
            epoch_ms,
        );
        let reservation = self
            .task_projection
            .reserve_bound_handoff_link(&staged, epoch_ms, deadline, self.hooks.as_ref())
            .map_err(|failure| self.project_task_failure(failure))?;
        self.hooks.pause(
            V5PausePoint::BeforeTaskStoreCreate,
            self.hooks
                .commit_deadline_at(V5PausePoint::BeforeTaskStoreCreate, deadline),
        )?;
        match self.complete_handoff_into_task(
            staged,
            Some(reservation),
            epoch_ms,
            deadline,
            |_| {},
        )? {
            HandoffCompletion::Terminal(terminal_record) => Ok(task_reply(&terminal_record)),
            HandoffCompletion::Bound(..) => Err(ReceiptLedgerError::Corrupt(
                "a staged handoff completed without its terminal",
            )),
        }
    }

    fn complete_staged_handoff_reply(
        &self,
        staged: TaskHandoffActorBoundReceipt,
        provisional: Option<&V5StoredInvocationRecord>,
        terminal_record: V5StoredInvocationRecord,
        terminal_link: TaskTerminalBoundReceipt,
        deadline: Instant,
    ) -> Result<ReceiptReply, ReceiptLedgerError> {
        let epoch_ms = terminal_link.terminal_epoch_ms();
        self.hooks.event(
            V5ReceiptRuntimeEventKind::TaskStoreTerminalCommitted,
            epoch_ms,
        );
        self.hooks.event(
            V5ReceiptRuntimeEventKind::TaskStoreTerminalReadback,
            epoch_ms,
        );
        let terminal_link = self.complete_staged_task_handoff(
            staged.key().clone(),
            staged.record_version(),
            terminal_link,
            deadline,
        )?;
        if let Some(provisional) = provisional {
            self.hooks.staged_terminal_publication(
                &staged,
                provisional,
                &terminal_record,
                &terminal_link,
            )?;
        }
        self.hooks.event(
            V5ReceiptRuntimeEventKind::TaskTerminalBoundCommitted,
            epoch_ms,
        );
        self.hooks
            .terminal_bound_task(&terminal_record, &terminal_link);
        Ok(ReceiptReply::Json(ReceiptResponse::Invocation {
            outcome: V5InvocationResponse::Task {
                snapshot: task_store_snapshot(&terminal_record),
            },
        }))
    }

    /// Publishes the terminal of a failed validation or admission and, when
    /// the failure poisons the process, latches fail-stop so the reply is the
    /// last one this daemon admits.
    fn admission_failure_reply(
        &self,
        reservation: crate::receipt_ledger::ReservedReceipt,
        outcome: ReceiptTerminalOutcome,
        fail_stop: bool,
        epoch_ms: u64,
        deadline: Instant,
    ) -> Result<ReceiptReply, ReceiptLedgerError> {
        let terminal = rejection_terminal(&outcome, H::MAX_SUPPORTED_BODY_RESPONSE_BYTES)?;
        let reply = self.publish_pre_actor_terminal(reservation, epoch_ms, terminal, deadline);
        if fail_stop {
            self.hooks.restart_requested();
            self.external_store_fail_stop.store(true, Ordering::Release);
        }
        match (fail_stop, reply) {
            (true, Ok(ReceiptReply::Prepared(frame))) => Ok(ReceiptReply::PreparedFailStop(frame)),
            (true, Ok(ReceiptReply::Json(response))) => Ok(ReceiptReply::JsonFailStop(response)),
            (_, reply) => reply,
        }
    }

    /// A begun receipt an observer promoted to a handoff while this handler
    /// was paused: materialize the Task and answer with it.
    fn continue_promoted_begun_handoff(
        &self,
        handoff: TaskHandoffActorBoundReceipt,
        epoch_ms: u64,
        deadline: Instant,
    ) -> Result<ReceiptReply, ReceiptLedgerError> {
        let epoch_ms = self.continuation_epoch_ms(epoch_ms, handoff.task());
        if self.hooks.prepare_rejects() {
            let terminal = injected_rejection_terminal(
                "scenario prepare rejected invocation",
                H::MAX_SUPPORTED_BODY_RESPONSE_BYTES,
            )?;
            return self
                .publish_staged_handoff_terminal_reply(handoff, terminal, epoch_ms, deadline);
        }
        let (task_record, task_bound) =
            match self.complete_handoff_into_task(handoff, None, epoch_ms, deadline, |_| {})? {
                HandoffCompletion::Bound(task_record, task_bound) => (task_record, task_bound),
                HandoffCompletion::Terminal(terminal_record) => {
                    return Ok(task_reply(&terminal_record))
                }
            };
        let (task_record, task_bound) = self
            .task_projection
            .start_bound_task(&task_bound, task_record, deadline)
            .map_err(|failure| self.project_task_failure(failure))?;
        self.hooks.bound_task(&task_record, &task_bound);
        self.hooks
            .event(V5ReceiptRuntimeEventKind::TaskBoundCommitted, epoch_ms);
        Ok(ReceiptReply::Json(ReceiptResponse::Invocation {
            outcome: V5InvocationResponse::Task {
                snapshot: task_store_snapshot(&task_record),
            },
        }))
    }

    fn publish_pre_actor_terminal(
        &self,
        reservation: crate::receipt_ledger::ReservedReceipt,
        epoch_ms: u64,
        terminal: crate::receipt_ledger::V5CanonicalTerminal,
        deadline: Instant,
    ) -> Result<ReceiptReply, ReceiptLedgerError> {
        match self
            .receipt_ledger
            .recover(reservation.key().clone(), deadline)?
        {
            ReceiptState::Reserved(current) => {
                self.publish_direct_terminal(current, epoch_ms, terminal, deadline)
            }
            ReceiptState::TaskPromisedUnbound(promised) => {
                let task_id = promised.task().task_id();
                let receipt = self.receipt_ledger.publish_receipt_backed_task_terminal(
                    promised.key().clone(),
                    TaskCancellationReceipt::PromisedUnbound(promised),
                    epoch_ms,
                    terminal,
                    deadline,
                )?;
                self.hooks.receipt_backed_terminal(&receipt)?;
                self.hooks.event(
                    V5ReceiptRuntimeEventKind::ReceiptTerminalCommitted,
                    epoch_ms,
                );
                let snapshot = self.resolve_task(task_id, deadline)?;
                Ok(ReceiptReply::Json(ReceiptResponse::Invocation {
                    outcome: V5InvocationResponse::Task { snapshot },
                }))
            }
            _ => Err(ReceiptLedgerError::ReceiptRowPresentUnsupported),
        }
    }

    fn reply_for_cancel_submit_decision(
        &self,
        decision: CancelReservedSubmitDecision,
        epoch_ms: u64,
        deadline: Instant,
        origin: &'static str,
        response_kind: &'static str,
    ) -> Result<ReceiptReply, ReceiptLedgerError> {
        match decision {
            CancelReservedSubmitDecision::ExecuteReserved(reservation) => {
                Ok(ReceiptReply::Json(pending_reserved_response(&reservation)))
            }
            CancelReservedSubmitDecision::PublishCancelledDirect(intent) => {
                self.hooks.event(
                    V5ReceiptRuntimeEventKind::CancelReservationConverted,
                    epoch_ms,
                );
                self.hooks.pause(
                    V5PausePoint::AfterCancelReservationConvertedBeforeTerminal,
                    deadline,
                )?;
                let publication = self.receipt_ledger.publish_direct_terminal(
                    intent.reservation().key().clone(),
                    intent.reservation().record_version(),
                    epoch_ms,
                    intent.terminal().clone(),
                    deadline,
                )?;
                self.hooks.event(
                    V5ReceiptRuntimeEventKind::ReceiptTerminalCommitted,
                    epoch_ms,
                );
                self.hooks.direct_publication(
                    &publication,
                    "cancelled",
                    "immediate_publication",
                    None,
                );
                Ok(ReceiptReply::Prepared(publication.into_parts().1))
            }
            CancelReservedSubmitDecision::ExistingDirectTerminal(receipt) => self
                .reply_for_existing_state_with_origin(
                    ReceiptState::DirectTerminalUnacked(receipt),
                    deadline,
                    origin,
                    response_kind,
                ),
            CancelReservedSubmitDecision::Rejected(rejection) => {
                self.reply_for_existing_state(rejection.into_state(), deadline)
            }
        }
    }

    /// Fence the exact key of a submission whose answer was lost before the
    /// daemon reserved it: under the key's lock either the receipt is
    /// already there and is answered like its recovery, or the frame, if it
    /// still arrives, meets the fence and never runs.
    pub fn fence_invocation(
        &self,
        key: ReceiptKey,
        epoch_ms: u64,
        deadline: Instant,
    ) -> Result<ReceiptReply, ReceiptLedgerError> {
        let before = self.joins.capture_reply(key.reserved_task_id());
        let reply = self.fence_invocation_canonical(key, epoch_ms, deadline)?;
        self.project_holder_reply(&before, reply, deadline)
    }

    fn fence_invocation_canonical(
        &self,
        key: ReceiptKey,
        epoch_ms: u64,
        deadline: Instant,
    ) -> Result<ReceiptReply, ReceiptLedgerError> {
        self.validate_receipt_key(&key)?;
        let digest = crate::receipt_ledger::receipt_key_digest(&key);
        let _stripe = self.submit_fences.stripe(&digest);
        match self.recover_invocation_canonical(key.clone(), epoch_ms, deadline) {
            Err(ReceiptLedgerError::ReceiptNotFound) if self.submit_fences.fence(digest) => {
                Ok(ReceiptReply::Json(ReceiptResponse::Invocation {
                    outcome: V5InvocationResponse::ReceiptPending {
                        receipt_key: key,
                        phase: V5InvocationPhase::CancelReserved,
                        accepted_epoch_ms: epoch_ms,
                        original_budget_ms: 0,
                        cancel_requested: true,
                    },
                }))
            }
            other => other,
        }
    }

    pub fn recover_invocation(
        &self,
        key: ReceiptKey,
        epoch_ms: u64,
        deadline: Instant,
    ) -> Result<ReceiptReply, ReceiptLedgerError> {
        let before = self.joins.capture_reply(key.reserved_task_id());
        let reply = self.recover_invocation_canonical(key, epoch_ms, deadline)?;
        self.project_holder_reply(&before, reply, deadline)
    }

    fn recover_invocation_canonical(
        &self,
        key: ReceiptKey,
        epoch_ms: u64,
        deadline: Instant,
    ) -> Result<ReceiptReply, ReceiptLedgerError> {
        self.validate_receipt_key(&key)?;
        // A joined call has no receipt of its own: its alias answers.
        if let Some(alias) = self.joins.alias(key.reserved_task_id()) {
            if !alias.holder.matches(&key) {
                return Err(ReceiptLedgerError::InvocationIdentityMismatch);
            }
            let snapshot = self.resolve_task(key.reserved_task_id(), deadline)?;
            return Ok(ReceiptReply::Json(ReceiptResponse::Invocation {
                outcome: V5InvocationResponse::Task { snapshot },
            }));
        }
        let state = match self
            .receipt_ledger
            .recover_at(key.clone(), epoch_ms, deadline)
        {
            Ok(state) => state,
            Err(ReceiptLedgerError::ReceiptNotFound) => {
                let Some(record) = self
                    .task_projection
                    .read_bound_task(key.reserved_task_id(), deadline)
                    .map_err(|failure| self.project_task_failure(failure))?
                else {
                    return Err(ReceiptLedgerError::ReceiptNotFound);
                };
                if record.invocation_id != key.invocation_id()
                    || record.receipt_key_digest != crate::receipt_ledger::receipt_key_digest(&key)
                    || record.tool != key.tool()
                    || record.normalized_arguments_hash != *key.normalized_arguments_hash()
                {
                    return Err(ReceiptLedgerError::InvocationIdentityMismatch);
                }
                return Ok(ReceiptReply::Json(ReceiptResponse::Invocation {
                    outcome: V5InvocationResponse::Task {
                        snapshot: task_store_snapshot(&record),
                    },
                }));
            }
            Err(error) => return Err(error),
        };
        let decision = decide_cancel_reserved_submit(ReserveOutcome::ExistingExact(state))
            .map_err(|_| {
                ReceiptLedgerError::Corrupt("canonical recovery terminal could not be constructed")
            })?;
        self.reply_for_cancel_submit_decision(
            decision,
            epoch_ms,
            deadline,
            "recovery",
            "recovered_direct",
        )
    }

    pub fn acknowledge_invocation(
        &self,
        key: ReceiptKey,
        terminal_digest: TerminalDigest,
        epoch_ms: u64,
        deadline: Instant,
    ) -> Result<ReceiptReply, ReceiptLedgerError> {
        self.validate_receipt_key(&key)?;
        let task_id = key.reserved_task_id();
        let acknowledged =
            self.receipt_ledger
                .acknowledge_direct(key, terminal_digest, epoch_ms, deadline)?;
        // The frontend holds the answer: nobody waits for it by Task id.
        self.receipt_handles.remove(task_id);
        self.hooks.event(
            V5ReceiptRuntimeEventKind::AcknowledgementCommitted,
            epoch_ms,
        );
        Ok(ReceiptReply::Json(
            ReceiptResponse::InvocationAcknowledged {
                acknowledgement: V5AcknowledgedReceipt::from_receipt(&acknowledged),
            },
        ))
    }

    /// Canonical bodies may hold key locks while constructing a Task reply.
    /// Normalize only after they return; an unjoined handoff with unchanged
    /// membership keeps its already canonical snapshot without another read.
    fn project_holder_reply(
        &self,
        before: &HolderReplyContext,
        reply: ReceiptReply,
        deadline: Instant,
    ) -> Result<ReceiptReply, ReceiptLedgerError> {
        let (mut response, fail_stop) = match reply {
            ReceiptReply::Json(response) => (response, false),
            ReceiptReply::JsonFailStop(response) => (response, true),
            prepared => return Ok(prepared),
        };
        let snapshot = match &mut response {
            ReceiptResponse::Task { snapshot }
            | ReceiptResponse::Invocation {
                outcome: V5InvocationResponse::Task { snapshot },
            } => Some(snapshot),
            _ => None,
        };
        if let Some(snapshot) = snapshot {
            if snapshot.task_id() != before.task_id() {
                return Err(ReceiptLedgerError::InvocationIdentityMismatch);
            }
            match self.joins.reply_projection(before) {
                HolderReplyProjection::Retired => return Err(ReceiptLedgerError::ReceiptNotFound),
                HolderReplyProjection::Unchanged => {}
                HolderReplyProjection::Normalize => {
                    let fresh = self.resolve_task(snapshot.task_id(), deadline)?;
                    if fresh.invocation_id() != snapshot.invocation_id()
                        || fresh.receipt_key_digest() != snapshot.receipt_key_digest()
                    {
                        return Err(ReceiptLedgerError::InvocationIdentityMismatch);
                    }
                    if self.joins.reply_projection(before) == HolderReplyProjection::Retired {
                        return Err(ReceiptLedgerError::ReceiptNotFound);
                    }
                    *snapshot = fresh;
                }
            }
        }
        Ok(if fail_stop {
            ReceiptReply::JsonFailStop(response)
        } else {
            ReceiptReply::Json(response)
        })
    }

    /// The state of one Task as its holder sees it: a joined call sees the
    /// shared Task under its own identity, a holder that let go sees its own
    /// cancellation, everybody else sees the stored Task.
    pub fn resolve_task(
        &self,
        task_id: unica_execution_contract::invocation::TaskId,
        deadline: Instant,
    ) -> Result<response::V5DaemonTaskSnapshot, ReceiptLedgerError> {
        let original = self.joins.capture_reply(task_id);
        loop {
            if Instant::now() >= deadline {
                return Err(ReceiptLedgerError::DeadlineExceeded);
            }
            let captured = self.joins.capture_reply(task_id);
            let context = captured.read();
            let leader = context.map_or(task_id, |read| read.leader);
            let shared = match self.resolve_receipt_task(leader, deadline) {
                Err(ReceiptLedgerError::ReceiptNotFound) => {
                    if let Some(read) = &context {
                        self.joins.forget_missing_context(read);
                    }
                    return Err(ReceiptLedgerError::ReceiptNotFound);
                }
                other => other?,
            };
            if let Some(read) = &context {
                let canonical = &read.canonical_holder;
                if shared.task_id() != canonical.task_id
                    || shared.invocation_id() != canonical.invocation_id
                    || shared.receipt_key_digest() != &canonical.receipt_key_digest
                {
                    return Err(ReceiptLedgerError::ReceiptNotFound);
                }
            }
            // Take a fresh registry decision after the canonical read. It
            // validates the exact read context and orders observations against
            // cancellation without keeping the registry locked during I/O.
            match self
                .joins
                .project_holder(&captured, task_snapshot_is_terminal(&shared))
            {
                HolderProjection::Reload => continue,
                HolderProjection::Retired => return Err(ReceiptLedgerError::ReceiptNotFound),
                HolderProjection::Shared => {
                    if self.joins.reply_projection(&original) == HolderReplyProjection::Retired {
                        return Err(ReceiptLedgerError::ReceiptNotFound);
                    }
                    return Ok(match context {
                        Some(read) => joined_task_snapshot(&shared, &read.holder, None),
                        None => shared,
                    });
                }
                HolderProjection::Cancelled(at) => {
                    if self.joins.reply_projection(&original) == HolderReplyProjection::Retired {
                        return Err(ReceiptLedgerError::ReceiptNotFound);
                    }
                    let read = context.expect("a holder cancellation has a read context");
                    return Ok(joined_task_snapshot(&shared, &read.holder, Some(at)));
                }
            }
        }
    }

    fn resolve_task_for_key(
        &self,
        key: &ReceiptKey,
        deadline: Instant,
    ) -> Result<response::V5DaemonTaskSnapshot, ReceiptLedgerError> {
        let snapshot = self.resolve_task(key.reserved_task_id(), deadline)?;
        if snapshot.invocation_id() != key.invocation_id()
            || snapshot.receipt_key_digest() != &crate::receipt_ledger::receipt_key_digest(key)
        {
            return Err(ReceiptLedgerError::InvocationIdentityMismatch);
        }
        Ok(snapshot)
    }

    fn resolve_task_reply_for_key(
        &self,
        key: &ReceiptKey,
        deadline: Instant,
    ) -> Result<ReceiptReply, ReceiptLedgerError> {
        let snapshot = self.resolve_task_for_key(key, deadline)?;
        Ok(ReceiptReply::Json(ReceiptResponse::Invocation {
            outcome: V5InvocationResponse::Task { snapshot },
        }))
    }

    /// A Task from its stores, or else the receipt reserved under its id.
    fn resolve_receipt_task(
        &self,
        task_id: unica_execution_contract::invocation::TaskId,
        deadline: Instant,
    ) -> Result<response::V5DaemonTaskSnapshot, ReceiptLedgerError> {
        match self.resolve_stored_task(task_id, deadline) {
            Err(ReceiptLedgerError::ReceiptNotFound) => {
                self.resolve_receipt_handle(task_id, deadline)
            }
            other => other,
        }
    }

    /// The Task id a lost-submit refusal names while its call is no Task:
    /// a receipt still inline is a queued Task, one that ended directly and
    /// was never acknowledged answers with its terminal. Nothing runs again.
    fn resolve_receipt_handle(
        &self,
        task_id: unica_execution_contract::invocation::TaskId,
        deadline: Instant,
    ) -> Result<response::V5DaemonTaskSnapshot, ReceiptLedgerError> {
        let Some(key) = self.receipt_handles.get(task_id) else {
            return Err(ReceiptLedgerError::ReceiptNotFound);
        };
        let digest = crate::receipt_ledger::receipt_key_digest(&key);
        let queued = |created_at_epoch_ms: u64, cancel_requested: bool| {
            response::V5DaemonTaskSnapshot::Queued {
                task_id,
                invocation_id: key.invocation_id(),
                receipt_key_digest: digest.clone(),
                created_at_epoch_ms,
                updated_at_epoch_ms: created_at_epoch_ms,
                ttl_ms: DIRECT_TERMINAL_RETENTION_MS,
                poll_interval_ms: V5_TASK_POLL_INTERVAL_MS,
                version: 1,
                cancel_requested,
            }
        };
        match self.receipt_ledger.recover(key.clone(), deadline) {
            Ok(ReceiptState::Reserved(reserved)) => Ok(queued(
                reserved.original_cutoff().accepted_epoch_ms(),
                reserved.cancel_requested(),
            )),
            Ok(ReceiptState::CancelReserved(reserved)) => {
                Ok(queued(reserved.cancel_reserved_at_epoch_ms(), true))
            }
            Ok(ReceiptState::DirectTerminalUnacked(receipt)) => Ok(terminal_task_snapshot(
                TerminalSnapshotIdentity {
                    task_id,
                    invocation_id: key.invocation_id(),
                    receipt_key_digest: digest.clone(),
                    created_at_epoch_ms: receipt.original_cutoff().accepted_epoch_ms(),
                    updated_at_epoch_ms: receipt.terminal_epoch_ms(),
                    ttl_ms: DIRECT_TERMINAL_RETENTION_MS,
                    poll_interval_ms: V5_TASK_POLL_INTERVAL_MS,
                    version: receipt.record_version().get(),
                    cancel_requested: false,
                    terminal_epoch_ms: receipt.terminal_epoch_ms(),
                },
                receipt.terminal(),
            )),
            Ok(
                state @ (ReceiptState::TaskPromisedUnbound(_)
                | ReceiptState::TaskPromisedActorBound(_)
                | ReceiptState::TaskHandoffActorBound(_)
                | ReceiptState::TaskReceiptOwnedActorBound(_)
                | ReceiptState::TaskTerminalReceiptBacked(_)),
            ) => receipt_state_task_snapshot(state),
            // Acknowledged, expired or gone: the answer is not this id's any
            // more.
            Ok(_) | Err(ReceiptLedgerError::ReceiptNotFound) => {
                self.receipt_handles.remove(task_id);
                Err(ReceiptLedgerError::ReceiptNotFound)
            }
            Err(error) => Err(error),
        }
    }

    fn resolve_stored_task(
        &self,
        task_id: unica_execution_contract::invocation::TaskId,
        deadline: Instant,
    ) -> Result<response::V5DaemonTaskSnapshot, ReceiptLedgerError> {
        self.task_projection
            .retire_expired_terminal_tasks(deadline, self.hooks.as_ref())
            .map_err(|failure| self.project_task_failure(failure))?;
        if let Some(record) = self
            .task_projection
            .read_bound_task(task_id, deadline)
            .map_err(|failure| self.project_task_failure(failure))?
        {
            return Ok(task_store_snapshot(&record));
        }
        let state = self.receipt_ledger.resolve_task(task_id, deadline)?;
        let receipt = match state {
            ReceiptState::TaskPromisedUnbound(receipt) => {
                return Ok(queued_receipt_task_snapshot(
                    receipt.task(),
                    receipt.key_digest().clone(),
                    receipt.cancel_requested(),
                ));
            }
            ReceiptState::TaskPromisedActorBound(receipt) => {
                return Ok(queued_receipt_task_snapshot(
                    receipt.task(),
                    receipt.key_digest().clone(),
                    receipt.cancel_requested(),
                ));
            }
            ReceiptState::TaskHandoffActorBound(receipt) => {
                return Ok(queued_receipt_task_snapshot(
                    receipt.task(),
                    receipt.key_digest().clone(),
                    receipt.cancel_requested(),
                ));
            }
            ReceiptState::TaskReceiptOwnedActorBound(receipt) => {
                return Ok(queued_receipt_task_snapshot(
                    receipt.task(),
                    receipt.key_digest().clone(),
                    receipt.cancel_requested(),
                ));
            }
            ReceiptState::TaskTerminalReceiptBacked(receipt) => receipt,
            _ => return Err(ReceiptLedgerError::ReceiptRowPresentUnsupported),
        };
        let task = receipt.task();
        let common = (
            task.task_id(),
            task.invocation_id(),
            receipt.key_digest().clone(),
            task.created_at_epoch_ms(),
            task.updated_at_epoch_ms(),
            task.ttl_ms(),
            task.poll_interval_ms(),
            task.version(),
            receipt.cancel_requested(),
        );
        let snapshot = match receipt.terminal().outcome() {
            crate::receipt_ledger::ReceiptTerminalOutcome::Completed { result } => {
                response::V5DaemonTaskSnapshot::Completed {
                    task_id: common.0,
                    invocation_id: common.1,
                    receipt_key_digest: common.2,
                    created_at_epoch_ms: common.3,
                    updated_at_epoch_ms: common.4,
                    ttl_ms: common.5,
                    poll_interval_ms: common.6,
                    version: common.7,
                    cancel_requested: common.8,
                    terminal_epoch_ms: receipt.terminal_epoch_ms(),
                    terminal_digest: receipt.terminal().digest().clone(),
                    result: result.clone(),
                }
            }
            crate::receipt_ledger::ReceiptTerminalOutcome::Failed { reason } => {
                response::V5DaemonTaskSnapshot::Failed {
                    task_id: common.0,
                    invocation_id: common.1,
                    receipt_key_digest: common.2,
                    created_at_epoch_ms: common.3,
                    updated_at_epoch_ms: common.4,
                    ttl_ms: common.5,
                    poll_interval_ms: common.6,
                    version: common.7,
                    cancel_requested: common.8,
                    terminal_epoch_ms: receipt.terminal_epoch_ms(),
                    terminal_digest: receipt.terminal().digest().clone(),
                    reason: *reason,
                }
            }
            crate::receipt_ledger::ReceiptTerminalOutcome::Cancelled => {
                response::V5DaemonTaskSnapshot::Cancelled {
                    task_id: common.0,
                    invocation_id: common.1,
                    receipt_key_digest: common.2,
                    created_at_epoch_ms: common.3,
                    updated_at_epoch_ms: common.4,
                    ttl_ms: common.5,
                    poll_interval_ms: common.6,
                    version: common.7,
                    cancel_requested: common.8,
                    terminal_epoch_ms: receipt.terminal_epoch_ms(),
                    terminal_digest: receipt.terminal().digest().clone(),
                }
            }
        };
        Ok(snapshot)
    }

    pub fn wait_task(
        &self,
        task_id: unica_execution_contract::invocation::TaskId,
        wait_ms: u64,
        deadline: Instant,
    ) -> Result<response::V5DaemonTaskSnapshot, ReceiptLedgerError> {
        let wait_deadline = Instant::now()
            .checked_add(Duration::from_millis(wait_ms))
            .unwrap_or(deadline)
            .min(deadline);
        loop {
            let snapshot = self.resolve_task(task_id, deadline)?;
            if task_snapshot_is_terminal(&snapshot) || Instant::now() >= wait_deadline {
                return Ok(snapshot);
            }
            let remaining = wait_deadline.saturating_duration_since(Instant::now());
            thread::sleep(remaining.min(Duration::from_millis(10)));
        }
    }

    /// Cancel one holder's interest. The work itself is cancelled only when
    /// no joined call still waits for it.
    pub fn cancel_task(
        &self,
        task_id: unica_execution_contract::invocation::TaskId,
        deadline: Instant,
    ) -> Result<response::V5DaemonTaskSnapshot, ReceiptLedgerError> {
        let before = self.joins.capture_reply(task_id);
        let snapshot = self.cancel_task_canonical(task_id, deadline)?;
        let reply = self.project_holder_reply(
            &before,
            ReceiptReply::Json(ReceiptResponse::Task { snapshot }),
            deadline,
        )?;
        match reply {
            ReceiptReply::Json(ReceiptResponse::Task { snapshot }) => Ok(snapshot),
            _ => unreachable!("Task normalization preserves the reply envelope"),
        }
    }

    fn cancel_task_canonical(
        &self,
        task_id: unica_execution_contract::invocation::TaskId,
        deadline: Instant,
    ) -> Result<response::V5DaemonTaskSnapshot, ReceiptLedgerError> {
        let decision = self.joins.cancel_holder(task_id, self.epoch_ms());
        self.cancel_task_with_decision(task_id, decision, deadline)
    }

    fn cancel_task_with_decision(
        &self,
        task_id: unica_execution_contract::invocation::TaskId,
        decision: HolderCancel,
        deadline: Instant,
    ) -> Result<response::V5DaemonTaskSnapshot, ReceiptLedgerError> {
        match decision {
            HolderCancel::Released => self.resolve_task(task_id, deadline),
            HolderCancel::CancelShared(shared) if shared == task_id => {
                match self.cancel_stored_task(task_id, deadline) {
                    // Not a Task yet, or ended directly: the id a lost-submit
                    // refusal named cancels its exact receipt instead.
                    Err(ReceiptLedgerError::ReceiptNotFound) => {
                        let Some(key) = self.receipt_handles.get(task_id) else {
                            return Err(ReceiptLedgerError::ReceiptNotFound);
                        };
                        self.cancel_invocation(key, self.epoch_ms(), deadline)?;
                        self.resolve_task(task_id, deadline)
                    }
                    other => other,
                }
            }
            HolderCancel::CancelShared(shared) => {
                self.cancel_stored_task(shared, deadline)?;
                self.resolve_task(task_id, deadline)
            }
        }
    }

    /// The digest of the receipt key a Task id belongs to, read from its
    /// Task link or, before the handoff completion moved the key there, from
    /// the receipt ledger. The key leaves the ledger only once its link
    /// stands, so a key missed by both reads moved in between: the link is
    /// read once more. A receipt still inline is found by the handle its
    /// reservation left in this process; a handle evicted from the full
    /// handle table leaves such a cancel without the key's lock, as before
    /// #1399. `None` when nothing knows the Task.
    fn stored_task_key_digest(
        &self,
        task_id: unica_execution_contract::invocation::TaskId,
        deadline: Instant,
    ) -> Result<Option<crate::receipt_ledger::ReceiptKeyDigest>, ReceiptLedgerError> {
        let link_digest = || {
            self.task_projection
                .read_link(task_id, deadline)
                .map(|link| link.map(|link| crate::receipt_ledger::receipt_key_digest(link.key())))
                .map_err(|failure| self.project_task_failure(failure))
        };
        if let Some(digest) = link_digest()? {
            return Ok(Some(digest));
        }
        let key = match self.receipt_ledger.resolve_task(task_id, deadline) {
            Ok(ReceiptState::TaskPromisedUnbound(receipt)) => receipt.key().clone(),
            Ok(ReceiptState::TaskPromisedActorBound(receipt)) => receipt.key().clone(),
            Ok(ReceiptState::TaskHandoffActorBound(receipt)) => receipt.key().clone(),
            Ok(ReceiptState::TaskReceiptOwnedActorBound(receipt)) => receipt.key().clone(),
            Ok(ReceiptState::TaskTerminalReceiptBacked(receipt)) => receipt.key().clone(),
            Ok(ReceiptState::TaskBound(receipt)) => receipt.key().clone(),
            Ok(ReceiptState::TaskTerminalBound(receipt)) => receipt.key().clone(),
            Ok(ReceiptState::TaskRetirementPending(receipt)) => receipt.key().clone(),
            Ok(_) | Err(ReceiptLedgerError::ReceiptNotFound) => {
                if let Some(digest) = link_digest()? {
                    return Ok(Some(digest));
                }
                // A receipt this process reserved and still runs inline is
                // not a Task for the ledger yet, but its handler may hand it
                // off at any moment: its key is known here.
                match self.receipt_handles.get(task_id) {
                    Some(key) => key,
                    None => return Ok(None),
                }
            }
            Err(error) => return Err(error),
        };
        Ok(Some(crate::receipt_ledger::receipt_key_digest(&key)))
    }

    fn cancel_stored_task(
        &self,
        task_id: unica_execution_contract::invocation::TaskId,
        deadline: Instant,
    ) -> Result<response::V5DaemonTaskSnapshot, ReceiptLedgerError> {
        self.task_projection
            .retire_expired_terminal_tasks(deadline, self.hooks.as_ref())
            .map_err(|failure| self.project_task_failure(failure))?;
        // A cancel by Task id takes the key's lock like the exact-key cancel:
        // the handoff completion materializes the Task, publishes its link,
        // stages its terminal and moves the key under that lock, so this
        // cancel either marks the ledger row before it or reaches the Task
        // through its link after it, never a half-published Task (#1399).
        // Taken before the terminal gate, never inside it.
        let digest = self.stored_task_key_digest(task_id, deadline)?;
        let _key_lock = digest
            .as_ref()
            .map(|digest| self.submit_fences.stripe(digest));
        // An attempt still running in this process ends a Task that has not
        // begun itself once it hears the cancel, as after an exact-key cancel:
        // a terminal published here would move the Task under it. Only a Task
        // no attempt runs any more is ended here.
        let attempt_live = self.live_attempts.contains(task_id);
        {
            let _task_terminal_gate = self
                .task_terminal_coordinator
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(record) = self
                .task_projection
                .cancel_bound_task(task_id, deadline, self.hooks.as_ref())
                .map_err(|failure| self.project_task_failure(failure))?
            {
                self.active_task_cancellations.cancel(task_id);
                let provider_deadline =
                    unica_work_control::provider_deadline::ProviderDeadline::new(deadline);
                let link = self
                    .task_projection
                    .lifecycle_links
                    .read_by_task_id(task_id, provider_deadline)
                    .map_err(|error| {
                        self.project_task_failure(V5TaskProjectionFailure::from_link_store(error))
                    })?;
                if let TaskLifecycleLinkRecord::TaskBound(bound) = link {
                    if !attempt_live
                        && bound.phase() == AttemptPhase::NotBegun
                        && record.cancel_requested
                    {
                        let terminal = crate::receipt_ledger::canonical_cancelled_terminal()
                            .map_err(|_| {
                                ReceiptLedgerError::Corrupt("canonical v5 terminal failed")
                            })?;
                        let (terminal_record, terminal_link) = self
                            .task_projection
                            .publish_bound_task_terminal(
                                &bound,
                                &record,
                                &terminal,
                                self.epoch_ms(),
                                deadline,
                                self.hooks.as_ref(),
                            )
                            .map_err(|failure| self.project_task_failure(failure))?;
                        self.hooks.event(
                            V5ReceiptRuntimeEventKind::TaskStoreTerminalCommitted,
                            self.epoch_ms(),
                        );
                        self.hooks.event(
                            V5ReceiptRuntimeEventKind::TaskStoreTerminalReadback,
                            self.epoch_ms(),
                        );
                        self.hooks.event(
                            V5ReceiptRuntimeEventKind::TaskTerminalBoundCommitted,
                            self.epoch_ms(),
                        );
                        self.hooks
                            .terminal_bound_task(&terminal_record, &terminal_link);
                        return Ok(task_store_snapshot(&terminal_record));
                    }
                }
                return Ok(task_store_snapshot(&record));
            }
        }
        let state = self.receipt_ledger.resolve_task(task_id, deadline)?;
        let (expected, terminalize_without_started_attempt) = match state {
            ReceiptState::TaskPromisedUnbound(receipt) => {
                (TaskCancellationReceipt::PromisedUnbound(receipt), true)
            }
            ReceiptState::TaskPromisedActorBound(receipt) => (
                TaskCancellationReceipt::PromisedActorBound(receipt),
                !attempt_live,
            ),
            ReceiptState::TaskHandoffActorBound(receipt) => {
                let terminalize = receipt.phase() == AttemptPhase::NotBegun && !attempt_live;
                (
                    TaskCancellationReceipt::HandoffActorBound(receipt),
                    terminalize,
                )
            }
            ReceiptState::TaskReceiptOwnedActorBound(receipt) => (
                TaskCancellationReceipt::ReceiptOwnedActorBound(receipt),
                false,
            ),
            ReceiptState::TaskTerminalReceiptBacked(receipt) => {
                return self.resolve_task(receipt.task().task_id(), deadline)
            }
            _ => return Err(ReceiptLedgerError::ReceiptRowPresentUnsupported),
        };
        let cancelled =
            self.receipt_ledger
                .request_task_cancel(expected.key().clone(), expected, deadline)?;
        if terminalize_without_started_attempt {
            let terminal = crate::receipt_ledger::canonical_cancelled_terminal()
                .map_err(|_| ReceiptLedgerError::Corrupt("canonical v5 terminal failed"))?;
            let committed = self.receipt_ledger.publish_receipt_backed_task_terminal(
                cancelled.key().clone(),
                cancelled,
                self.epoch_ms(),
                terminal,
                deadline,
            )?;
            // An admission still running for this Task observes the token
            // at its next checkpoint, after the terminal is durable, and
            // stops before the actor bind (#1251).
            self.active_task_cancellations.cancel(task_id);
            self.hooks.receipt_backed_terminal(&committed)?;
            self.hooks.release_pre_actor_pauses();
            self.hooks.event(
                V5ReceiptRuntimeEventKind::ReceiptTerminalCommitted,
                self.epoch_ms(),
            );
            return self.resolve_task(task_id, deadline);
        }
        // An attempt already running toward this handoff learns of the
        // cancel now; its Task is materialized with it, as for the exact-key
        // cancel (#1395, #1399).
        self.active_task_cancellations.cancel(task_id);
        Ok(queued_receipt_task_snapshot(
            cancelled.task(),
            crate::receipt_ledger::receipt_key_digest(cancelled.key()),
            cancelled.cancel_requested(),
        ))
    }

    /// An identical read whose work already runs as a Task joins that Task
    /// instead of starting the same traversal again (#1376). The joined call
    /// reserves nothing: it answers with its own identity over the shared
    /// Task. `None` means the call runs on its own, as before.
    fn join_identical_read(
        &self,
        key: &ReceiptKey,
        invocation: &H::Request,
        response_deadline: &crate::invocation::InvocationResponseDeadline,
        deadline: Instant,
    ) -> Result<Option<ReceiptReply>, ReceiptLedgerError> {
        if !tool_is_joinable(key.tool()) {
            // A mutation closes joins before it reserves: a read submitted
            // after it never takes a result read before it.
            self.joins.close_joins();
            return Ok(None);
        }
        // The exact repeat of a joined call answers from its alias.
        if let Some(alias) = self.joins.alias(key.reserved_task_id()) {
            if !alias.holder.matches(key) {
                return Err(ReceiptLedgerError::InvocationIdentityMismatch);
            }
            let snapshot = self.resolve_task(key.reserved_task_id(), deadline)?;
            return Ok(Some(ReceiptReply::Json(ReceiptResponse::Invocation {
                outcome: V5InvocationResponse::Task { snapshot },
            })));
        }
        let identity = key.request_identity();
        loop {
            let Some(candidate) = self.joins.candidate(&identity) else {
                return Ok(None);
            };
            if candidate.task_id == key.reserved_task_id() || self.mutation_live(deadline) {
                return Ok(None);
            }
            match self.resolve_stored_task(candidate.task_id, deadline) {
                Ok(shared) if !task_snapshot_is_terminal(&shared) && !shared.cancel_requested() => {
                    // An exact repeat of a call that already holds its own
                    // receipt reads that receipt; it never becomes a joiner.
                    if !matches!(
                        self.receipt_ledger.recover(key.clone(), deadline),
                        Err(ReceiptLedgerError::ReceiptNotFound)
                    ) || !matches!(
                        self.resolve_stored_task(key.reserved_task_id(), deadline),
                        Err(ReceiptLedgerError::ReceiptNotFound)
                    ) {
                        return Ok(None);
                    }
                    // A join never answers with a walk older than the sources
                    // at this call: any change since the leader started, by
                    // Unica or by an external edit, makes the call run itself.
                    if !join_freshness::sources_unchanged_since(
                        invocation.workspace_hint(),
                        candidate.started_wall,
                        &|hint| self.invocation_executor.discover_join_workspace_root(hint),
                        &|root| {
                            self.invocation_executor
                                .discover_join_source_set_paths(root)
                        },
                    ) {
                        self.joins.forget_leader(&identity, candidate.task_id);
                        return Ok(None);
                    }
                    if self.joins.holders_full() {
                        self.forget_retired_shared_tasks(deadline);
                    }
                    let Some(alias) = self.joins.attach(&identity, candidate.task_id, key) else {
                        return Ok(None);
                    };
                    let snapshot = self.resolve_task(alias.holder.task_id, deadline)?;
                    return Ok(Some(ReceiptReply::Json(ReceiptResponse::Invocation {
                        outcome: V5InvocationResponse::Task { snapshot },
                    })));
                }
                Ok(_) => {
                    self.joins.forget_leader(&identity, candidate.task_id);
                    return Ok(None);
                }
                Err(ReceiptLedgerError::ReceiptNotFound) if candidate.in_flight => {
                    // The leader is still inline. Only a leader whose own
                    // cutoff already passed is about to become a Task; one
                    // inside its window may well answer directly.
                    let own = response_deadline.remaining_handoff_budget();
                    if !candidate
                        .leader_deadline
                        .remaining_handoff_budget()
                        .is_zero()
                        || own.is_zero()
                    {
                        return Ok(None);
                    }
                    self.joins
                        .wait_changed(own.min(INLINE_CUTOFF_POLL_INTERVAL * 5));
                }
                Err(ReceiptLedgerError::ReceiptNotFound) => {
                    self.joins.forget_leader(&identity, candidate.task_id);
                    return Ok(None);
                }
                // The leader's trouble is not this call's: it runs on its own.
                Err(_) => return Ok(None),
            }
        }
    }

    /// Whether an `apply` or `run` may still change what a read sees. A
    /// mutation is over once it has a terminal or nobody can find it.
    fn mutation_live(&self, deadline: Instant) -> bool {
        let mut live = false;
        for task_id in self.joins.tracked_mutations() {
            match self.resolve_receipt_task(task_id, deadline) {
                Ok(snapshot) if task_snapshot_is_terminal(&snapshot) => {
                    self.joins.mutation_finished(task_id)
                }
                Err(ReceiptLedgerError::ReceiptNotFound) => self.joins.mutation_finished(task_id),
                // Running, or its state unknown: reads do not join meanwhile.
                Ok(_) | Err(_) => live = true,
            }
        }
        live
    }

    fn forget_retired_shared_tasks(&self, deadline: Instant) {
        for leader in self.joins.shared_leaders() {
            if matches!(
                self.resolve_receipt_task(leader, deadline),
                Err(ReceiptLedgerError::ReceiptNotFound)
            ) {
                self.joins.forget_shared(leader);
            }
        }
    }

    fn reply_for_existing_state(
        &self,
        state: ReceiptState,
        deadline: Instant,
    ) -> Result<ReceiptReply, ReceiptLedgerError> {
        self.reply_for_existing_state_with_origin(state, deadline, "exact_duplicate", "direct")
    }

    fn reply_for_existing_state_with_origin(
        &self,
        state: ReceiptState,
        deadline: Instant,
        origin: &'static str,
        response_kind: &'static str,
    ) -> Result<ReceiptReply, ReceiptLedgerError> {
        match state {
            ReceiptState::CancelReserved(receipt) => {
                Ok(ReceiptReply::Json(pending_cancel_response(&receipt)))
            }
            ReceiptState::Reserved(reserved) => {
                Ok(ReceiptReply::Json(pending_reserved_response(&reserved)))
            }
            ReceiptState::DirectTerminalUnacked(receipt) => {
                let expected_version = receipt.record_version().checked_previous().ok_or(
                    ReceiptLedgerError::Corrupt(
                        "direct terminal receipt has no predecessor version",
                    ),
                )?;
                let publication = self.receipt_ledger.publish_direct_terminal(
                    receipt.key().clone(),
                    expected_version,
                    receipt.terminal_epoch_ms(),
                    receipt.terminal().clone(),
                    deadline,
                )?;
                self.hooks
                    .direct_publication(&publication, response_kind, origin, None);
                Ok(ReceiptReply::Prepared(publication.into_parts().1))
            }
            ReceiptState::AcknowledgedTombstone(receipt) => {
                Ok(ReceiptReply::Json(ReceiptResponse::Invocation {
                    outcome: V5InvocationResponse::Acknowledged {
                        acknowledgement: V5AcknowledgedReceipt::from_receipt(&receipt),
                    },
                }))
            }
            state @ (ReceiptState::TaskPromisedUnbound(_)
            | ReceiptState::TaskPromisedActorBound(_)
            | ReceiptState::TaskHandoffActorBound(_)
            | ReceiptState::TaskReceiptOwnedActorBound(_)
            | ReceiptState::TaskTerminalReceiptBacked(_)) => {
                let snapshot = receipt_state_task_snapshot(state)?;
                Ok(ReceiptReply::Json(ReceiptResponse::Invocation {
                    outcome: V5InvocationResponse::Task { snapshot },
                }))
            }
            _ => Err(ReceiptLedgerError::ReceiptRowPresentUnsupported),
        }
    }

    fn validate_receipt_key(&self, key: &ReceiptKey) -> Result<(), ReceiptLedgerError> {
        if key.core_identity_digest() != &self.core_identity_digest {
            return Err(ReceiptLedgerError::InvocationIdentityMismatch);
        }
        Ok(())
    }

    pub fn hooks(&self) -> &Arc<dyn V5RuntimeHooks<H>> {
        &self.hooks
    }
    pub fn active_cancellations_empty(&self) -> bool {
        self.active_task_cancellations.is_empty()
    }
    pub fn has_retained_workspace_work(&self) -> bool {
        self.invocation_executor.has_retained_workspace_work()
    }
}

pub enum ReceiptReply {
    Json(ReceiptResponse),
    /// A closed reply written after the process latched fail-stop: the
    /// daemon answers this request and admits nothing further.
    JsonFailStop(ReceiptResponse),
    Prepared(PreparedWireFrame),
    PreparedFailStop(PreparedWireFrame),
}

impl ReceiptReply {
    /// The same reply as the closed one a fail-stopped process writes.
    pub fn after_fail_stop(self) -> Self {
        match self {
            Self::Json(response) => Self::JsonFailStop(response),
            Self::Prepared(frame) => Self::PreparedFailStop(frame),
            closed @ (Self::JsonFailStop(_) | Self::PreparedFailStop(_)) => closed,
        }
    }
}

/// Pre-execution refusals have the same delivery limit as executed outcomes.
/// An oversized recovery route becomes a size refusal, not store corruption.
fn rejection_terminal(
    outcome: &ReceiptTerminalOutcome,
    max_supported_body_response_bytes: usize,
) -> Result<crate::receipt_ledger::V5CanonicalTerminal, ReceiptLedgerError> {
    match canonical_v5_terminal(outcome, max_supported_body_response_bytes) {
        Ok(terminal) => Ok(terminal),
        Err(CanonicalTerminalError::ResultTooLarge) => {
            crate::receipt_ledger::canonical_oversized_terminal(
                outcome,
                max_supported_body_response_bytes,
            )
            .map_err(|_| ReceiptLedgerError::Corrupt("canonical size refusal failed"))
        }
        Err(CanonicalTerminalError::Serialization) => {
            Err(ReceiptLedgerError::Corrupt("canonical v5 terminal failed"))
        }
    }
}

/// The terminal an injected rejection publishes in place of the real stage.
fn injected_rejection_terminal(
    summary: &'static str,
    max_supported_body_response_bytes: usize,
) -> Result<crate::receipt_ledger::V5CanonicalTerminal, ReceiptLedgerError> {
    canonical_v5_terminal(
        &ReceiptTerminalOutcome::Completed {
            result: Box::new(
                unica_execution_contract::invocation::DomainResult::canonical_rejection(
                    None,
                    RefusalCode::BadValue,
                    summary,
                ),
            ),
        },
        max_supported_body_response_bytes,
    )
    .map_err(|_| ReceiptLedgerError::Corrupt("canonical v5 terminal failed"))
}

fn reservation_is_still_unbound(state: &ReceiptState) -> bool {
    matches!(
        state,
        ReceiptState::Reserved(receipt) if matches!(receipt.phase(), ReservedPhase::Unbound)
    ) || matches!(state, ReceiptState::TaskPromisedUnbound(_))
}

fn reservation_is_begun(state: &ReceiptState) -> bool {
    matches!(
        state,
        ReceiptState::Reserved(receipt) if matches!(receipt.phase(), ReservedPhase::Begun { .. })
    )
}

fn queued_receipt_task_snapshot(
    task: &ReceiptTaskProjection,
    receipt_key_digest: crate::receipt_ledger::ReceiptKeyDigest,
    cancel_requested: bool,
) -> response::V5DaemonTaskSnapshot {
    response::V5DaemonTaskSnapshot::Queued {
        task_id: task.task_id(),
        invocation_id: task.invocation_id(),
        receipt_key_digest,
        created_at_epoch_ms: task.created_at_epoch_ms(),
        updated_at_epoch_ms: task.updated_at_epoch_ms(),
        ttl_ms: task.ttl_ms(),
        poll_interval_ms: task.poll_interval_ms(),
        version: task.version(),
        cancel_requested,
    }
}

fn receipt_state_task_snapshot(
    state: ReceiptState,
) -> Result<response::V5DaemonTaskSnapshot, ReceiptLedgerError> {
    let receipt = match state {
        ReceiptState::TaskPromisedUnbound(receipt) => {
            return Ok(queued_receipt_task_snapshot(
                receipt.task(),
                receipt.key_digest().clone(),
                receipt.cancel_requested(),
            ));
        }
        ReceiptState::TaskPromisedActorBound(receipt) => {
            return Ok(queued_receipt_task_snapshot(
                receipt.task(),
                receipt.key_digest().clone(),
                receipt.cancel_requested(),
            ));
        }
        ReceiptState::TaskHandoffActorBound(receipt) => {
            return Ok(queued_receipt_task_snapshot(
                receipt.task(),
                receipt.key_digest().clone(),
                receipt.cancel_requested(),
            ));
        }
        ReceiptState::TaskReceiptOwnedActorBound(receipt) => {
            return Ok(queued_receipt_task_snapshot(
                receipt.task(),
                receipt.key_digest().clone(),
                receipt.cancel_requested(),
            ));
        }
        ReceiptState::TaskTerminalReceiptBacked(receipt) => receipt,
        _ => return Err(ReceiptLedgerError::ReceiptRowPresentUnsupported),
    };
    let task = receipt.task();
    let common = (
        task.task_id(),
        task.invocation_id(),
        receipt.key_digest().clone(),
        task.created_at_epoch_ms(),
        task.updated_at_epoch_ms(),
        task.ttl_ms(),
        task.poll_interval_ms(),
        task.version(),
        receipt.cancel_requested(),
    );
    Ok(match receipt.terminal().outcome() {
        ReceiptTerminalOutcome::Completed { result } => response::V5DaemonTaskSnapshot::Completed {
            task_id: common.0,
            invocation_id: common.1,
            receipt_key_digest: common.2,
            created_at_epoch_ms: common.3,
            updated_at_epoch_ms: common.4,
            ttl_ms: common.5,
            poll_interval_ms: common.6,
            version: common.7,
            cancel_requested: common.8,
            terminal_epoch_ms: receipt.terminal_epoch_ms(),
            terminal_digest: receipt.terminal().digest().clone(),
            result: result.clone(),
        },
        ReceiptTerminalOutcome::Failed { reason } => response::V5DaemonTaskSnapshot::Failed {
            task_id: common.0,
            invocation_id: common.1,
            receipt_key_digest: common.2,
            created_at_epoch_ms: common.3,
            updated_at_epoch_ms: common.4,
            ttl_ms: common.5,
            poll_interval_ms: common.6,
            version: common.7,
            cancel_requested: common.8,
            terminal_epoch_ms: receipt.terminal_epoch_ms(),
            terminal_digest: receipt.terminal().digest().clone(),
            reason: *reason,
        },
        ReceiptTerminalOutcome::Cancelled => response::V5DaemonTaskSnapshot::Cancelled {
            task_id: common.0,
            invocation_id: common.1,
            receipt_key_digest: common.2,
            created_at_epoch_ms: common.3,
            updated_at_epoch_ms: common.4,
            ttl_ms: common.5,
            poll_interval_ms: common.6,
            version: common.7,
            cancel_requested: common.8,
            terminal_epoch_ms: receipt.terminal_epoch_ms(),
            terminal_digest: receipt.terminal().digest().clone(),
        },
    })
}

fn task_store_snapshot(record: &V5StoredInvocationRecord) -> response::V5DaemonTaskSnapshot {
    let common = (
        record.task_id,
        record.invocation_id,
        record.receipt_key_digest.clone(),
        record.created_at_epoch_ms,
        record.updated_at_epoch_ms,
        record.ttl_ms,
        record.poll_interval_ms,
        record.version,
        record.cancel_requested,
    );
    match &record.task {
        V5StoredTask::Queued => response::V5DaemonTaskSnapshot::Queued {
            task_id: common.0,
            invocation_id: common.1,
            receipt_key_digest: common.2,
            created_at_epoch_ms: common.3,
            updated_at_epoch_ms: common.4,
            ttl_ms: common.5,
            poll_interval_ms: common.6,
            version: common.7,
            cancel_requested: common.8,
        },
        V5StoredTask::Working => response::V5DaemonTaskSnapshot::Working {
            task_id: common.0,
            invocation_id: common.1,
            receipt_key_digest: common.2,
            created_at_epoch_ms: common.3,
            updated_at_epoch_ms: common.4,
            ttl_ms: common.5,
            poll_interval_ms: common.6,
            version: common.7,
            cancel_requested: common.8,
        },
        V5StoredTask::Completed {
            terminal_epoch_ms,
            terminal_digest,
            result,
        } => response::V5DaemonTaskSnapshot::Completed {
            task_id: common.0,
            invocation_id: common.1,
            receipt_key_digest: common.2,
            created_at_epoch_ms: common.3,
            updated_at_epoch_ms: common.4,
            ttl_ms: common.5,
            poll_interval_ms: common.6,
            version: common.7,
            cancel_requested: common.8,
            terminal_epoch_ms: *terminal_epoch_ms,
            terminal_digest: terminal_digest.clone(),
            result: result.clone(),
        },
        V5StoredTask::Failed {
            terminal_epoch_ms,
            terminal_digest,
            reason,
        } => response::V5DaemonTaskSnapshot::Failed {
            task_id: common.0,
            invocation_id: common.1,
            receipt_key_digest: common.2,
            created_at_epoch_ms: common.3,
            updated_at_epoch_ms: common.4,
            ttl_ms: common.5,
            poll_interval_ms: common.6,
            version: common.7,
            cancel_requested: common.8,
            terminal_epoch_ms: *terminal_epoch_ms,
            terminal_digest: terminal_digest.clone(),
            reason: *reason,
        },
        V5StoredTask::Cancelled {
            terminal_epoch_ms,
            terminal_digest,
        } => response::V5DaemonTaskSnapshot::Cancelled {
            task_id: common.0,
            invocation_id: common.1,
            receipt_key_digest: common.2,
            created_at_epoch_ms: common.3,
            updated_at_epoch_ms: common.4,
            ttl_ms: common.5,
            poll_interval_ms: common.6,
            version: common.7,
            cancel_requested: common.8,
            terminal_epoch_ms: *terminal_epoch_ms,
            terminal_digest: terminal_digest.clone(),
        },
    }
}

fn task_snapshot_is_terminal(snapshot: &response::V5DaemonTaskSnapshot) -> bool {
    matches!(
        snapshot,
        response::V5DaemonTaskSnapshot::Completed { .. }
            | response::V5DaemonTaskSnapshot::Failed { .. }
            | response::V5DaemonTaskSnapshot::Cancelled { .. }
    )
}

fn pending_cancel_response(
    receipt: &crate::receipt_ledger::CancelReservedReceipt,
) -> ReceiptResponse {
    ReceiptResponse::Invocation {
        outcome: V5InvocationResponse::ReceiptPending {
            receipt_key: receipt.key().clone(),
            phase: V5InvocationPhase::CancelReserved,
            accepted_epoch_ms: receipt.cancel_reserved_at_epoch_ms(),
            original_budget_ms: 0,
            cancel_requested: true,
        },
    }
}

fn pending_reserved_response(receipt: &crate::receipt_ledger::ReservedReceipt) -> ReceiptResponse {
    let phase = match receipt.phase() {
        ReservedPhase::Unbound => V5InvocationPhase::ReservedUnbound,
        ReservedPhase::ActorBound { .. } => V5InvocationPhase::ReservedActorBound,
        ReservedPhase::Begun { .. } => V5InvocationPhase::ReservedBegun,
    };
    ReceiptResponse::Invocation {
        outcome: V5InvocationResponse::ReceiptPending {
            receipt_key: receipt.key().clone(),
            phase,
            accepted_epoch_ms: receipt.original_cutoff().accepted_epoch_ms(),
            original_budget_ms: receipt.original_cutoff().response_budget_ms(),
            cancel_requested: receipt.cancel_requested(),
        },
    }
}

/// Identity and times of a Task snapshot built from a terminal.
struct TerminalSnapshotIdentity {
    task_id: unica_execution_contract::invocation::TaskId,
    invocation_id: unica_execution_contract::invocation::InvocationId,
    receipt_key_digest: ReceiptKeyDigest,
    created_at_epoch_ms: u64,
    updated_at_epoch_ms: u64,
    ttl_ms: u64,
    poll_interval_ms: u64,
    version: u64,
    cancel_requested: bool,
    terminal_epoch_ms: u64,
}

fn terminal_task_snapshot(
    identity: TerminalSnapshotIdentity,
    terminal: &crate::receipt_ledger::V5CanonicalTerminal,
) -> response::V5DaemonTaskSnapshot {
    let TerminalSnapshotIdentity {
        task_id,
        invocation_id,
        receipt_key_digest,
        created_at_epoch_ms,
        updated_at_epoch_ms,
        ttl_ms,
        poll_interval_ms,
        version,
        cancel_requested,
        terminal_epoch_ms,
    } = identity;
    let terminal_digest = terminal.digest().clone();
    match terminal.outcome() {
        ReceiptTerminalOutcome::Completed { result } => response::V5DaemonTaskSnapshot::Completed {
            task_id,
            invocation_id,
            receipt_key_digest,
            created_at_epoch_ms,
            updated_at_epoch_ms,
            ttl_ms,
            poll_interval_ms,
            version,
            cancel_requested,
            terminal_epoch_ms,
            terminal_digest,
            result: result.clone(),
        },
        ReceiptTerminalOutcome::Failed { reason } => response::V5DaemonTaskSnapshot::Failed {
            task_id,
            invocation_id,
            receipt_key_digest,
            created_at_epoch_ms,
            updated_at_epoch_ms,
            ttl_ms,
            poll_interval_ms,
            version,
            cancel_requested,
            terminal_epoch_ms,
            terminal_digest,
            reason: *reason,
        },
        ReceiptTerminalOutcome::Cancelled => response::V5DaemonTaskSnapshot::Cancelled {
            task_id,
            invocation_id,
            receipt_key_digest,
            created_at_epoch_ms,
            updated_at_epoch_ms,
            ttl_ms,
            poll_interval_ms,
            version,
            cancel_requested,
            terminal_epoch_ms,
            terminal_digest,
        },
    }
}

/// Apply the registry's already ordered terminal decision to the canonical
/// shared Task; durable wall timestamps do not determine causal order.
fn joined_task_snapshot(
    shared: &response::V5DaemonTaskSnapshot,
    holder: &JoinHolder,
    released_at_epoch_ms: Option<u64>,
) -> response::V5DaemonTaskSnapshot {
    use response::V5DaemonTaskSnapshot as Snapshot;
    if let Some(released) = released_at_epoch_ms {
        let cancelled = crate::receipt_ledger::canonical_cancelled_terminal()
            .expect("the canonical cancelled terminal has a fixed encoding");
        return terminal_task_snapshot(
            TerminalSnapshotIdentity {
                task_id: holder.task_id,
                invocation_id: holder.invocation_id,
                receipt_key_digest: holder.receipt_key_digest.clone(),
                created_at_epoch_ms: shared.created_at_epoch_ms(),
                updated_at_epoch_ms: shared.updated_at_epoch_ms().max(released),
                ttl_ms: shared.ttl_ms(),
                poll_interval_ms: shared.poll_interval_ms(),
                // The holder's own cancellation is a newer state than any
                // shared state it saw before.
                version: shared.version().saturating_add(1),
                cancel_requested: true,
                terminal_epoch_ms: released,
            },
            &cancelled,
        );
    }
    let mut projected = shared.clone();
    match &mut projected {
        Snapshot::Queued {
            task_id,
            invocation_id,
            receipt_key_digest,
            ..
        }
        | Snapshot::Working {
            task_id,
            invocation_id,
            receipt_key_digest,
            ..
        }
        | Snapshot::Completed {
            task_id,
            invocation_id,
            receipt_key_digest,
            ..
        }
        | Snapshot::Failed {
            task_id,
            invocation_id,
            receipt_key_digest,
            ..
        }
        | Snapshot::Cancelled {
            task_id,
            invocation_id,
            receipt_key_digest,
            ..
        } => {
            *task_id = holder.task_id;
            *invocation_id = holder.invocation_id;
            *receipt_key_digest = holder.receipt_key_digest.clone();
        }
    }
    projected
}

/// The receipt ledger actor as the daemon runs it: a begun store step gets
/// [`DURABLE_STEP_BUDGET`] from its own start, however little its request
/// had left; the request's deadline bounds only the wait in the queue (#1387).
fn spawn_receipt_ledger_actor(
    port: impl crate::receipt_ledger::ReceiptLedgerPort,
) -> ReceiptLedgerActor {
    ReceiptLedgerActor::spawn_with_running_floor(port, DURABLE_STEP_BUDGET)
}

#[cfg(any(
    test,
    feature = "test-support",
    feature = "receipt-ledger-test-support"
))]
pub mod test_support {
    #[cfg(any(
        test,
        feature = "test-support",
        feature = "receipt-ledger-test-support"
    ))]
    pub use super::retirement_facts::{V5TaskProjectionFailure, V5TaskRetirementAuthorization};
    use super::*;
    #[cfg(any(
        test,
        feature = "test-support",
        feature = "receipt-ledger-test-support"
    ))]
    pub fn authorize_task_retirement_for_fixture<H: super::ports::InvocationHost>(
        runtime: &super::V5ReceiptRuntime<H>,
        expected: &crate::receipt_ledger::TaskRetirementPendingReceipt,
        deadline: std::time::Instant,
    ) -> Result<V5TaskRetirementAuthorization, V5TaskProjectionFailure> {
        runtime
            .task_projection
            .authorize_task_retirement(expected, deadline)
    }
    #[cfg(any(
        test,
        feature = "test-support",
        feature = "receipt-ledger-test-support"
    ))]
    pub fn delete_terminal_authorized_for_fixture<H: super::ports::InvocationHost>(
        runtime: &super::V5ReceiptRuntime<H>,
        authorization: &V5TaskRetirementAuthorization,
        observed_at_epoch_ms: u64,
        deadline: std::time::Instant,
    ) -> Result<crate::invocation_store_v5::V5DeleteTerminalOutcome, V5TaskProjectionFailure> {
        runtime.task_projection.delete_terminal_authorized(
            authorization,
            observed_at_epoch_ms,
            deadline,
        )
    }
    #[cfg(any(
        test,
        feature = "test-support",
        feature = "receipt-ledger-test-support"
    ))]
    pub fn retire_expired_terminal_tasks_for_fixture<H: super::ports::InvocationHost>(
        runtime: &super::V5ReceiptRuntime<H>,
        deadline: std::time::Instant,
    ) -> Result<(), V5TaskProjectionFailure> {
        runtime
            .task_projection
            .retire_expired_terminal_tasks(deadline, runtime.hooks.as_ref())
    }

    pub use super::join_freshness::TIMESTAMP_MARGIN;
    #[cfg(any(test, feature = "test-support"))]
    pub fn receipt_owner_identity<H: super::ports::InvocationHost>(
        runtime: &super::V5ReceiptRuntime<H>,
    ) -> unica_workspace::state_fs::FileIdentity {
        runtime.receipt_owner_identity_for_test
    }
    #[cfg(any(
        test,
        feature = "test-support",
        feature = "receipt-ledger-test-support"
    ))]
    pub fn promoted_continuations<H: super::ports::InvocationHost>(
        runtime: &super::V5ReceiptRuntime<H>,
    ) -> usize {
        runtime
            .promoted_continuations
            .load(std::sync::atomic::Ordering::Acquire)
    }
    /// An existing owned actor channel used only by explicit fixture setup/fault/second-owner cases.
    #[cfg(any(
        test,
        feature = "test-support",
        feature = "receipt-ledger-test-support"
    ))]
    pub fn receipt_actor_for_fixture<H: super::ports::InvocationHost>(
        runtime: &super::V5ReceiptRuntime<H>,
    ) -> &crate::receipt_ledger_actor::ReceiptLedgerActor {
        &runtime.receipt_ledger
    }
    /// The same Task store instance; grants no source file or ambient namespace factory.
    #[cfg(any(
        test,
        feature = "test-support",
        feature = "receipt-ledger-test-support"
    ))]
    pub fn task_store_for_fixture<H: super::ports::InvocationHost>(
        runtime: &super::V5ReceiptRuntime<H>,
    ) -> &std::sync::Arc<crate::task_store::FileInvocationStoreV5> {
        &runtime.task_projection.task_store
    }
    #[cfg(any(
        test,
        feature = "test-support",
        feature = "receipt-ledger-test-support"
    ))]
    pub fn lifecycle_links_for_fixture<H: super::ports::InvocationHost>(
        runtime: &super::V5ReceiptRuntime<H>,
    ) -> &crate::lifecycle_link_store::TaskLifecycleLinkStoreV5 {
        &runtime.task_projection.lifecycle_links
    }

    pub fn sources_unchanged_since(
        workspace_hint: &str,
        since: std::time::SystemTime,
        discover_workspace_root: &dyn Fn(&str) -> Result<std::path::PathBuf, String>,
        discover_source_set_paths: &dyn Fn(&std::path::Path) -> Result<Vec<String>, String>,
    ) -> bool {
        super::join_freshness::sources_unchanged_since(
            workspace_hint,
            since,
            discover_workspace_root,
            discover_source_set_paths,
        )
    }

    #[cfg(any(
        test,
        feature = "test-support",
        feature = "receipt-ledger-test-support"
    ))]
    pub use super::joining::HolderReplyContext;
    #[cfg(any(
        test,
        feature = "test-support",
        feature = "receipt-ledger-test-support"
    ))]
    pub const V5_TASK_POLL_INTERVAL_MS: u64 = super::V5_TASK_POLL_INTERVAL_MS;
    #[cfg(any(
        test,
        feature = "test-support",
        feature = "receipt-ledger-test-support"
    ))]
    pub const RETIREMENT_RETRY_MAX_DELAY_MS: u64 = super::RETIREMENT_RETRY_MAX_DELAY_MS;
    #[cfg(any(
        test,
        feature = "test-support",
        feature = "receipt-ledger-test-support"
    ))]
    pub fn install_receipt_actor_for_fixture<H: super::ports::InvocationHost>(
        runtime: &mut super::V5ReceiptRuntime<H>,
        actor: super::ReceiptLedgerActor,
    ) {
        runtime.receipt_ledger = actor;
    }
    pub fn install_epoch_clock_for_fixture<H: super::ports::InvocationHost>(
        runtime: &mut super::V5ReceiptRuntime<H>,
        clock: std::sync::Arc<dyn crate::invocation_store::EpochMillisClock>,
    ) {
        runtime.epoch_clock = clock;
    }
    #[cfg(any(
        test,
        feature = "test-support",
        feature = "receipt-ledger-test-support"
    ))]
    pub fn stable_authority_identity<H: super::ports::InvocationHost>(
        runtime: &super::V5ReceiptRuntime<H>,
    ) -> unica_workspace::state_fs::FileIdentity {
        runtime._stable_authority.ownership_file_identity_for_test()
    }
    #[cfg(any(
        test,
        feature = "test-support",
        feature = "receipt-ledger-test-support"
    ))]
    pub fn startup_recovery_entry_count<H: super::ports::InvocationHost>(
        runtime: &super::V5ReceiptRuntime<H>,
    ) -> usize {
        runtime.task_projection.recovery.entries().len()
    }
    #[cfg(any(
        test,
        feature = "test-support",
        feature = "receipt-ledger-test-support"
    ))]
    pub fn submit_fence_for_fixture<'a, H: super::ports::InvocationHost>(
        runtime: &'a super::V5ReceiptRuntime<H>,
        digest: &crate::receipt_ledger::ReceiptKeyDigest,
    ) -> std::sync::MutexGuard<'a, ()> {
        runtime.submit_fences.stripe(digest)
    }
    #[cfg(any(
        test,
        feature = "test-support",
        feature = "receipt-ledger-test-support"
    ))]
    pub fn capture_holder_reply<H: super::ports::InvocationHost>(
        runtime: &super::V5ReceiptRuntime<H>,
        task: unica_execution_contract::invocation::TaskId,
    ) -> HolderReplyContext {
        runtime.joins.capture_reply(task)
    }
    #[cfg(any(
        test,
        feature = "test-support",
        feature = "receipt-ledger-test-support"
    ))]
    pub fn project_holder_reply<H: super::ports::InvocationHost>(
        runtime: &super::V5ReceiptRuntime<H>,
        before: &HolderReplyContext,
        reply: super::ReceiptReply,
        deadline: std::time::Instant,
    ) -> Result<super::ReceiptReply, crate::receipt_ledger::ReceiptLedgerError> {
        runtime.project_holder_reply(before, reply, deadline)
    }
    pub fn task_store_snapshot(
        record: &crate::invocation_store_v5::V5StoredInvocationRecord,
    ) -> super::response::V5DaemonTaskSnapshot {
        super::task_store_snapshot(record)
    }
    #[cfg(any(
        test,
        feature = "test-support",
        feature = "receipt-ledger-test-support"
    ))]
    pub fn receipt_task_projection_from_store(
        record: &crate::invocation_store_v5::V5StoredInvocationRecord,
    ) -> Result<crate::receipt_ledger::ReceiptTaskProjection, V5TaskProjectionFailure> {
        super::receipt_task_projection_from_store(record)
    }
    #[cfg(any(
        test,
        feature = "test-support",
        feature = "receipt-ledger-test-support"
    ))]
    pub fn terminal_publication(
        terminal: &crate::receipt_ledger::V5CanonicalTerminal,
        epoch: u64,
    ) -> (
        crate::invocation_store_v5::V5TerminalPublication,
        crate::receipt_ledger::ClosedTerminalStatus,
    ) {
        super::V5TaskProjection::terminal_publication(terminal, epoch)
    }
    #[cfg(any(
        test,
        feature = "test-support",
        feature = "receipt-ledger-test-support"
    ))]
    pub fn retirement_retry_delay_ms(failures: u32) -> u64 {
        super::retirement_retry_delay_ms(failures)
    }
    pub fn terminal_record_from_retirement_pending(
        pending: &crate::receipt_ledger::TaskRetirementPendingReceipt,
    ) -> crate::invocation_store_v5::V5StoredInvocationRecord {
        super::terminal_record_from_retirement_pending(pending)
    }
    #[cfg(any(
        test,
        feature = "test-support",
        feature = "receipt-ledger-test-support"
    ))]
    pub fn spawn_receipt_ledger_actor_for_fixture(
        port: impl crate::receipt_ledger::ReceiptLedgerPort,
    ) -> super::ReceiptLedgerActor {
        super::spawn_receipt_ledger_actor(port)
    }

    #[cfg(any(
        test,
        feature = "test-support",
        feature = "receipt-ledger-test-support"
    ))]
    pub fn cancel_bound_task_for_fixture<H: InvocationHost>(
        runtime: &super::V5ReceiptRuntime<H>,
        task_id: unica_execution_contract::invocation::TaskId,
        deadline: Instant,
        hooks: &dyn V5RuntimeHooks<H>,
    ) -> Result<Option<V5StoredInvocationRecord>, V5TaskProjectionFailure> {
        runtime
            .task_projection
            .cancel_bound_task(task_id, deadline, hooks)
    }
    #[cfg(any(
        test,
        feature = "test-support",
        feature = "receipt-ledger-test-support"
    ))]
    pub fn finalize_task_retirement_for_fixture<H: super::ports::InvocationHost>(
        runtime: &super::V5ReceiptRuntime<H>,
        authorization: &V5TaskRetirementAuthorization,
        deadline: Instant,
    ) -> Result<(), V5TaskProjectionFailure> {
        runtime
            .task_projection
            .finalize_task_retirement(authorization, deadline)
    }
    #[cfg(any(
        test,
        feature = "test-support",
        feature = "receipt-ledger-test-support"
    ))]
    pub fn finish_task_terminal_publication_for_fixture<H: InvocationHost>(
        runtime: &super::V5ReceiptRuntime<H>,
        expected: &TaskBoundReceipt,
        terminal_record: V5StoredInvocationRecord,
        deadline: Instant,
        hooks: &dyn V5RuntimeHooks<H>,
    ) -> Result<(V5StoredInvocationRecord, TaskTerminalBoundReceipt), V5TaskProjectionFailure> {
        runtime.task_projection.finish_task_terminal_publication(
            expected,
            terminal_record,
            deadline,
            hooks,
        )
    }
    #[cfg(any(
        test,
        feature = "test-support",
        feature = "receipt-ledger-test-support"
    ))]
    pub fn materialize_bound_handoff_for_fixture<H: InvocationHost>(
        runtime: &super::V5ReceiptRuntime<H>,
        handoff: &TaskHandoffActorBoundReceipt,
        bind_epoch_ms: u64,
        deadline: Instant,
        hooks: &dyn V5RuntimeHooks<H>,
    ) -> Result<(V5StoredInvocationRecord, TaskBoundReceipt), V5TaskProjectionFailure> {
        runtime
            .task_projection
            .materialize_bound_handoff(handoff, bind_epoch_ms, deadline, hooks)
    }
    #[cfg(any(
        test,
        feature = "test-support",
        feature = "receipt-ledger-test-support"
    ))]
    pub fn materialize_staged_bound_handoff_for_fixture<H: InvocationHost>(
        runtime: &super::V5ReceiptRuntime<H>,
        handoff: &TaskHandoffActorBoundReceipt,
        reservation: &TaskLinkReservation,
        bind_epoch_ms: u64,
        deadline: Instant,
        hooks: &dyn V5RuntimeHooks<H>,
    ) -> Result<(V5StoredInvocationRecord, TaskBoundReceipt), V5TaskProjectionFailure> {
        runtime.task_projection.materialize_staged_bound_handoff(
            handoff,
            reservation,
            bind_epoch_ms,
            deadline,
            hooks,
        )
    }
    #[cfg(any(
        test,
        feature = "test-support",
        feature = "receipt-ledger-test-support"
    ))]
    pub fn publish_bound_task_terminal_for_fixture<H: InvocationHost>(
        runtime: &super::V5ReceiptRuntime<H>,
        expected: &TaskBoundReceipt,
        record: &V5StoredInvocationRecord,
        terminal: &crate::receipt_ledger::V5CanonicalTerminal,
        terminal_epoch_ms: u64,
        deadline: Instant,
        hooks: &dyn V5RuntimeHooks<H>,
    ) -> Result<(V5StoredInvocationRecord, TaskTerminalBoundReceipt), V5TaskProjectionFailure> {
        runtime.task_projection.publish_bound_task_terminal(
            expected,
            record,
            terminal,
            terminal_epoch_ms,
            deadline,
            hooks,
        )
    }
    #[cfg(any(
        test,
        feature = "test-support",
        feature = "receipt-ledger-test-support"
    ))]
    pub fn reserve_bound_handoff_link_for_fixture<H: InvocationHost>(
        runtime: &super::V5ReceiptRuntime<H>,
        handoff: &TaskHandoffActorBoundReceipt,
        _epoch_ms: u64,
        deadline: Instant,
        hooks: &dyn V5RuntimeHooks<H>,
    ) -> Result<TaskLinkReservation, V5TaskProjectionFailure> {
        runtime
            .task_projection
            .reserve_bound_handoff_link(handoff, _epoch_ms, deadline, hooks)
    }
    #[cfg(any(
        test,
        feature = "test-support",
        feature = "receipt-ledger-test-support"
    ))]
    pub fn start_bound_task_for_fixture<H: super::ports::InvocationHost>(
        runtime: &super::V5ReceiptRuntime<H>,
        expected: &TaskBoundReceipt,
        record: V5StoredInvocationRecord,
        deadline: Instant,
    ) -> Result<(V5StoredInvocationRecord, TaskBoundReceipt), V5TaskProjectionFailure> {
        runtime
            .task_projection
            .start_bound_task(expected, record, deadline)
    }

    pub fn canonical_task_retirement_pending_bytes(
        pending: &crate::receipt_ledger::TaskRetirementPendingReceipt,
    ) -> Result<Vec<u8>, V5TaskProjectionFailure> {
        super::canonical_task_retirement_pending_bytes(pending)
    }
    #[cfg(any(
        test,
        feature = "test-support",
        feature = "receipt-ledger-test-support"
    ))]
    pub fn terminalize_recovered_bound_for_fixture<H: super::ports::InvocationHost>(
        runtime: &super::V5ReceiptRuntime<H>,
        expected: &TaskBoundReceipt,
        record: V5StoredInvocationRecord,
        reason: RecoveryTerminalReason,
        deadline: Instant,
    ) -> Result<TaskTerminalBoundReceipt, V5TaskProjectionFailure> {
        runtime
            .task_projection
            .terminalize_recovered_bound(expected, record, reason, deadline)
    }

    #[cfg(feature = "receipt-ledger-test-support")]
    pub fn exact_task_link_reservation<H: super::ports::InvocationHost>(
        runtime: &super::V5ReceiptRuntime<H>,
        key: &crate::receipt_ledger::ReceiptKey,
        deadline: std::time::Instant,
    ) -> Result<crate::lifecycle_link_store::TaskLinkReservation, V5TaskProjectionFailure> {
        runtime
            .task_projection
            .exact_task_link_reservation(key, deadline)
    }
    #[cfg(feature = "receipt-ledger-test-support")]
    pub fn exact_task_record<H: super::ports::InvocationHost>(
        runtime: &super::V5ReceiptRuntime<H>,
        key: &crate::receipt_ledger::ReceiptKey,
        deadline: std::time::Instant,
    ) -> Result<crate::invocation_store_v5::V5StoredInvocationRecord, V5TaskProjectionFailure> {
        runtime.task_projection.exact_task_record(key, deadline)
    }
    pub fn open_task_projection_inspect_only_for_fixture(
        state: &dyn super::authority::ReceiptStateRoot,
        clock: std::sync::Arc<dyn crate::invocation_store::EpochMillisClock>,
        deadline: std::time::Instant,
    ) -> Result<(), String> {
        let projection = super::V5TaskProjection::open(state, clock, deadline)?;
        drop(projection);
        Ok(())
    }
    pub fn with_bound_task_start_evidence<T>(
        state: &dyn super::authority::ReceiptStateRoot,
        clock: std::sync::Arc<dyn crate::invocation_store::EpochMillisClock>,
        open_deadline: std::time::Instant,
        key: &crate::receipt_ledger::ReceiptKey,
        read_deadline: impl FnOnce() -> unica_work_control::provider_deadline::ProviderDeadline,
        observe: impl FnOnce(
            &crate::receipt_ledger::TaskBoundReceipt,
            &crate::invocation_store_v5::V5StoredInvocationRecord,
        ) -> Result<T, String>,
    ) -> Result<T, String> {
        let projection = super::V5TaskProjection::open(state, clock, open_deadline)?;
        let deadline = read_deadline();
        let link = projection
            .lifecycle_links
            .read_by_task_id(key.reserved_task_id(), deadline)
            .map_err(|error| format!("read bound-start lifecycle proof: {error}"))?;
        let crate::lifecycle_link_store::TaskLifecycleLinkRecord::TaskBound(bound) = link else {
            return Err(
                "bound-start proof requires one active TaskBound lifecycle link".to_owned(),
            );
        };
        let record = projection
            .task_store
            .get(key.reserved_task_id(), deadline)
            .map_err(|error| format!("read bound-start Task proof: {error}"))?;
        observe(&bound, &record)
    }
    pub fn task_outcome_after_cancel(
        candidate: ReceiptTerminalOutcome,
        cancel_requested: bool,
        authority: FactualOutcomeAuthority,
    ) -> ReceiptTerminalOutcome {
        super::task_outcome_after_cancel(candidate, cancel_requested, authority)
    }
    pub fn project_bound_task_for_read(
        record: V5StoredInvocationRecord,
        mask_working_as_queued: bool,
    ) -> V5StoredInvocationRecord {
        super::project_bound_task_for_read(record, mask_working_as_queued)
    }
    pub fn task_terminal_bound_matches_record(
        expected: &TaskTerminalBoundReceipt,
        record: &V5StoredInvocationRecord,
    ) -> Result<bool, V5TaskProjectionFailure> {
        super::task_terminal_bound_matches_record(expected, record)
    }
    pub fn queued_receipt_task_snapshot(
        task: &ReceiptTaskProjection,
        receipt_key_digest: crate::receipt_ledger::ReceiptKeyDigest,
        cancel_requested: bool,
    ) -> response::V5DaemonTaskSnapshot {
        super::queued_receipt_task_snapshot(task, receipt_key_digest, cancel_requested)
    }
    pub fn task_snapshot_is_terminal(snapshot: &response::V5DaemonTaskSnapshot) -> bool {
        super::task_snapshot_is_terminal(snapshot)
    }

    pub const RETIREMENT_RETRY_INITIAL_DELAY_MS: u64 = super::RETIREMENT_RETRY_INITIAL_DELAY_MS;
    /// Latches the process-owned fail-stop as a failed durable write of
    /// another call would.
    pub fn latch_fail_stop_for_fixture<H: super::ports::InvocationHost>(
        runtime: &super::V5ReceiptRuntime<H>,
    ) {
        runtime
            .external_store_fail_stop
            .store(true, std::sync::atomic::Ordering::Release);
    }
    /// The original owner error projection, including its existing fail-stop effect.
    pub fn project_task_failure_for_fixture<H: super::ports::InvocationHost>(
        runtime: &super::V5ReceiptRuntime<H>,
        failure: V5TaskProjectionFailure,
    ) -> crate::receipt_ledger::ReceiptLedgerError {
        runtime.project_task_failure(failure)
    }

    /// The same normal owner transition examined by ordinary lifecycle unit tests.
    #[cfg(any(test, feature = "test-support"))]
    pub fn complete_bound_task_handoff_for_fixture<H: super::ports::InvocationHost>(
        runtime: &super::V5ReceiptRuntime<H>,
        key: ReceiptKey,
        expected_version: crate::receipt_ledger::ReceiptVersion,
        confirmed_task_bound: TaskBoundReceipt,
        deadline: Instant,
    ) -> Result<TaskBoundReceipt, ReceiptLedgerError> {
        runtime.complete_bound_task_handoff(key, expected_version, confirmed_task_bound, deadline)
    }

    /// Runs the existing publication and key-fence path; creates no separate writer.
    #[cfg(any(test, feature = "test-support"))]
    pub fn publish_staged_handoff_terminal_reply_for_fixture<H: super::ports::InvocationHost>(
        runtime: &super::V5ReceiptRuntime<H>,
        handoff: TaskHandoffActorBoundReceipt,
        terminal: crate::receipt_ledger::V5CanonicalTerminal,
        epoch_ms: u64,
        deadline: Instant,
    ) -> Result<super::ReceiptReply, ReceiptLedgerError> {
        runtime.publish_staged_handoff_terminal_reply(handoff, terminal, epoch_ms, deadline)
    }
}
