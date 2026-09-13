use super::identity::{CoreIdentity, DaemonStateDirectory, ReceiptAuthorityLock};
use super::protocol_v5::{
    decode_v5_request_frame, read_bounded_v5_request_frame_before, DecodedV5Request,
    V5AcknowledgedReceipt, V5ClientRequest, V5ClientRequestKind, V5DaemonErrorCode,
    V5EndpointRecord, V5HandshakeServerResponse, V5InvocationPhase, V5InvocationRequest,
    V5InvocationResponse, V5ProbeServerResponse, V5RequestFrameError, V5ServerResponse,
    DAEMON_PROTOCOL_VERSION, MAX_V5_RESPONSE_LINE_BYTES,
};
use super::server::{
    CanonicalInvocationService, DaemonServerConfig, V5ActorBoundCanonicalInvocation,
    V5CanonicalInvocationRuntime, V5CanonicalPrepareError, MAX_HANDSHAKES, MAX_OWNER_SESSIONS,
};
use crate::application::invocation::{RESPONSE_SERIALIZATION_MARGIN, TASK_RECONCILIATION_BUDGET};
use crate::application::invocation_store::SystemEpochMillisClock;
use crate::application::invocation_store::{EpochMillisClock, ToolIdentity};
use crate::application::invocation_store_v5::{
    InvocationStoreV5, NewV5InvocationRecord, RecoveryTerminalReason, TaskStoreRecoveryCatalog,
    V5DeleteTerminalOutcome, V5SafeFailureReason, V5StartWorkingOutcome, V5StoredInvocationRecord,
    V5StoredTask, V5TaskIdentity, V5TaskRetirement, V5TaskStoreError, V5TerminalPublication,
};
use crate::application::invocation_v5::{
    classify_cancel_reserved_expiry_outcome, classify_recovered_receipt,
    decide_cancel_reserved_submit, decide_cancel_resolution, CancelInvocationDecision,
    CancelReservedExpiryDecision, CancelReservedRecoveryDecision, CancelReservedSubmitDecision,
};
use crate::application::ports::Clock;
use crate::application::receipt_ledger::{
    canonical_v5_terminal, AttemptPhase, CanonicalTerminalError, ClosedTerminalStatus,
    HandoffTerminalStage, OriginalCutoffDescriptor, PreparedWireFrame, ReceiptKey,
    ReceiptKeyDigest, ReceiptLedgerError, ReceiptState, ReceiptTaskProjection,
    ReceiptTerminalOutcome, ReserveOutcome, ReservedPhase, TaskBoundReceipt,
    TaskCancellationReceipt, TaskHandoffActorBoundReceipt, TaskPromisedActorBoundReceipt,
    TaskPromisedUnboundReceipt, TaskRetirementPendingReceipt, TaskTerminalBoundReceipt,
    TerminalDigest, DIRECT_TERMINAL_RETENTION_MS,
};
use crate::application::receipt_ledger_actor::ReceiptLedgerActor;
use crate::domain::cancellation::CancellationToken;
use crate::domain::refusal::RefusalCode;
use crate::infrastructure::platform::filesystem::RetainedDirectoryCapability;
use crate::infrastructure::receipt_ledger::arm_receipt_row_directory_sync_fault;
use crate::infrastructure::receipt_ledger::canonical_staged_transfer_certificate;
use crate::infrastructure::receipt_ledger::ReceiptLedgerStore;
use crate::infrastructure::task_lifecycle_link_store_v5::{
    TaskLifecycleLinkCatalogEntry, TaskLifecycleLinkRecord, TaskLifecycleLinkStoreError,
    TaskLifecycleLinkStoreV5, TaskLinkReservation,
};
use crate::infrastructure::task_store_v5::FileInvocationStoreV5;
use crate::infrastructure::task_store_v5::PublicationFailure;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::io::{self, BufReader, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};
use uuid::Uuid;

mod hooks;
pub(crate) use hooks::{
    NoHooks, V5AdmissionRejection, V5PausePoint, V5ReceiptRuntimeEventKind, V5RuntimeHooks,
    V5Stage, V5StoreFaultPoint,
};
#[cfg(feature = "receipt-ledger-test-support")]
mod receipt_scenario_v5;
#[cfg(feature = "receipt-ledger-test-support")]
pub(crate) use receipt_scenario_v5::run_supported_receipt_scenario_for_test;

const ACCEPT_POLL_INTERVAL: Duration = Duration::from_millis(10);
const AUTHORITY_ACQUIRE_TIMEOUT: Duration = Duration::from_secs(2);
const STARTUP_RECONCILIATION_TIMEOUT: Duration = Duration::from_secs(30);
const HANDSHAKE_READ_TIMEOUT: Duration = Duration::from_secs(2);
const SESSION_READ_TIMEOUT: Duration = Duration::from_secs(2);
const OWNER_RESPONSE_WRITE_TIMEOUT: Duration = Duration::from_secs(10);
const TASK_TERMINAL_PUBLICATION_TIMEOUT: Duration = Duration::from_secs(30);
/// The session handler owning an inline invocation re-reads its cutoff at
/// least this often while the worker runs; the worker wakes it earlier.
const INLINE_CUTOFF_POLL_INTERVAL: Duration = Duration::from_millis(5);
/// The durable handoff committed at the cutoff runs under its own bound: the
/// operation budget of the submit is spent by definition at that moment.
const CUTOFF_HANDOFF_COMMIT_BUDGET: Duration = TASK_RECONCILIATION_BUDGET;
const V5_TASK_POLL_INTERVAL_MS: u64 = 100;
static NEXT_RETIREMENT_PROCESS_GENERATION: AtomicU64 = AtomicU64::new(1);

pub(crate) struct V5ReceiptRuntime {
    core_identity: CoreIdentity,
    // On healthy shutdown Rust drops fields in declaration order: the actor
    // joins and releases its store before named authority is released. A
    // fail-stopped runtime is retained until process death instead.
    receipt_ledger: ReceiptLedgerActor,
    _stable_authority: ReceiptAuthorityLock,
    epoch_clock: Arc<dyn EpochMillisClock>,
    invocation_executor: V5InvocationExecutor,
    task_projection: V5TaskProjection,
    active_task_cancellations: Arc<V5ActiveTaskCancellations>,
    task_execution_threads: Mutex<Vec<thread::JoinHandle<()>>>,
    task_terminal_coordinator: Mutex<()>,
    external_store_fail_stop: AtomicBool,
    fail_stop_watchdogs: FailStopWatchdogs,
    /// How many promoted attempts the owner handed to a worker are still
    /// running their continuation: the contract harness waits on this to
    /// observe a promoted Task the runtime finishes off-thread.
    promoted_continuations: AtomicUsize,
    /// Who observes this runtime and what it injects; production installs
    /// `NoHooks`.
    hooks: Arc<dyn V5RuntimeHooks>,
}

#[derive(Default)]
struct V5ActiveTaskCancellations {
    tokens: Mutex<HashMap<crate::domain::invocation::TaskId, CancellationToken>>,
}

impl V5ActiveTaskCancellations {
    fn register(
        self: &Arc<Self>,
        task_id: crate::domain::invocation::TaskId,
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

    fn cancel(&self, task_id: crate::domain::invocation::TaskId) {
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

    fn is_empty(&self) -> bool {
        self.tokens
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty()
    }
}

struct V5ActiveTaskCancellationGuard {
    registry: Arc<V5ActiveTaskCancellations>,
    task_id: crate::domain::invocation::TaskId,
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
    Reply(V5RuntimeReply),
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
    /// from here the drive answers the seventh second. `false` when the
    /// handler already promoted the receipt — the drive then hands off at
    /// once into that intent.
    fn claim_cutoff(&self) -> bool {
        let mut state = self.lock();
        while matches!(state.decision, PipelineDecision::HandoffInProgress) {
            state = self
                .changed
                .wait(state)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        if matches!(state.decision, PipelineDecision::PromotedByOwner) {
            return false;
        }
        state.begun = true;
        self.changed.notify_all();
        true
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

/// Deadlines after which a stalled attempt fail-stops the process: the grace
/// after an unbound promise without an actor bind, and the grace after a
/// cancel without a terminal. The accept loop reads them on the runtime's
/// own clock, so the observer's clock drives them the same way.
#[derive(Default)]
struct FailStopWatchdogs {
    armed: Mutex<HashMap<ReceiptKeyDigest, (Instant, Instant)>>,
}

impl FailStopWatchdogs {
    fn arm(&self, digest: ReceiptKeyDigest, now: Instant, grace: Duration) {
        self.armed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entry(digest)
            .or_insert((now, now + grace));
    }

    fn disarm(&self, digest: &ReceiptKeyDigest) {
        self.armed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(digest);
    }

    /// The elapsed grace of a watchdog that is due at `now`.
    fn due(&self, now: Instant) -> Option<Duration> {
        self.armed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .filter(|(_, due)| now >= *due)
            .map(|(armed_at, _)| now.saturating_duration_since(*armed_at))
            .max()
    }
}

/// The grace a promised or cancelled attempt gets before the process
/// fail-stops on it.
const FAIL_STOP_GRACE: Duration = TASK_RECONCILIATION_BUDGET;

/// What the inline worker hands back to the session handler.
enum InlineWorkerReport {
    Rejected(Box<crate::domain::invocation::DomainResult>),
    KnownLong(super::server::V5PreparedCanonicalInvocation),
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
    task_id: crate::domain::invocation::TaskId,
    guard: Option<V5ActiveTaskCancellationGuard>,
}

struct InlineSlotState {
    decision: InlineHandoffDecision,
    report: Option<InlineWorkerReport>,
}

/// One inline invocation shared between the session handler that owns the
/// cutoff and the worker that runs prepare and execute.
struct InlineExecutionSlot {
    state: Mutex<InlineSlotState>,
    changed: Condvar,
}

impl InlineExecutionSlot {
    fn new() -> Self {
        Self {
            state: Mutex::new(InlineSlotState {
                decision: InlineHandoffDecision::Waiting,
                report: None,
            }),
            changed: Condvar::new(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, InlineSlotState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The worker deposits its report and learns who publishes it. While the
    /// handler commits a handoff the worker waits for that decision instead of
    /// racing it.
    fn settle(&self, report: InlineWorkerReport) -> InlineSettlement {
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

enum InlineSettlement {
    HandlerOwns,
    Abandoned,
    Task {
        report: InlineWorkerReport,
        ownership: Box<InlineTaskOwnership>,
    },
}

/// What the cutoff commit produced: the Task the running attempt continues
/// into, or its terminal when the outcome was already there.
#[allow(clippy::large_enum_variant)]
enum CutoffCommit {
    Bound(TaskBoundReceipt, V5StoredInvocationRecord),
    Terminal(V5RuntimeReply),
}

/// How the session handler continues after the inline worker settled or the
/// cutoff took the invocation away from it.
enum InlineDrive {
    Rejected(Box<crate::domain::invocation::DomainResult>),
    KnownLong(super::server::V5PreparedCanonicalInvocation),
    Direct(ReceiptTerminalOutcome),
    HandedOff(Box<V5RuntimeReply>),
}

struct V5TaskProjection {
    #[allow(dead_code)]
    task_store_root: RetainedDirectoryCapability,
    #[allow(dead_code)]
    task_store: Arc<FileInvocationStoreV5>,
    #[allow(dead_code)]
    lifecycle_link_root: RetainedDirectoryCapability,
    lifecycle_links: TaskLifecycleLinkStoreV5,
    recovery: TaskStoreRecoveryCatalog,
    epoch_clock: Arc<dyn EpochMillisClock>,
    retirement_process_instance_id: Uuid,
    retirement_process_generation: u64,
    retirement_issued_sequence: AtomicU64,
    retirement_coordinator: Mutex<()>,
}

#[derive(Clone)]
struct V5TaskRetirementAuthorization {
    process_instance_id: Uuid,
    generation: u64,
    issued_sequence: u64,
    pending_record_sha256: String,
    authorization_fingerprint: String,
    pending: TaskRetirementPendingReceipt,
}

struct StartupTaskTerminalizationPlan {
    expected: TaskBoundReceipt,
    record: V5StoredInvocationRecord,
    reason: Option<RecoveryTerminalReason>,
}

impl V5TaskProjection {
    fn open(
        state: &DaemonStateDirectory,
        epoch_clock: Arc<dyn EpochMillisClock>,
        deadline: Instant,
    ) -> Result<Self, String> {
        let task_store_root = state.create_private_retained_subdirectory("tasks")?;
        let (store, recovery) = FileInvocationStoreV5::open_retained_directory_inspect_only(
            task_store_root.clone(),
            Arc::clone(&epoch_clock),
            crate::domain::code_intelligence::ProviderDeadline::new(deadline),
        )
        .map_err(|error| format!("open inspect-only protocol-v5 task store: {error}"))?;
        let lifecycle_link_root =
            state.create_private_retained_subdirectory("task-lifecycle-links")?;
        let lifecycle_links = TaskLifecycleLinkStoreV5::open(
            lifecycle_link_root.path(),
            crate::domain::code_intelligence::ProviderDeadline::new(deadline),
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
            retirement_coordinator: Mutex::new(()),
        })
    }

    fn authorize_task_retirement(
        &self,
        expected: &TaskRetirementPendingReceipt,
        deadline: Instant,
    ) -> Result<V5TaskRetirementAuthorization, V5TaskProjectionFailure> {
        let provider_deadline = crate::domain::code_intelligence::ProviderDeadline::new(deadline);
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
        let provider_deadline = crate::domain::code_intelligence::ProviderDeadline::new(deadline);
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
                crate::domain::code_intelligence::ProviderDeadline::new(deadline),
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
        let provider_deadline = crate::domain::code_intelligence::ProviderDeadline::new(deadline);
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

    fn retire_expired_terminal_tasks(
        &self,
        deadline: Instant,
        hooks: &dyn V5RuntimeHooks,
    ) -> Result<(), V5TaskProjectionFailure> {
        hooks
            .pause(V5PausePoint::BeforeRetirementSnapshot, deadline)
            .map_err(V5TaskProjectionFailure::fail_stop)?;
        // One process owns the receipt authority, and this coordinator makes
        // terminal retirement a single-owner operation inside that process.
        // A concurrent observation waits and then takes a fresh catalog snapshot
        // instead of treating an ordinary optimistic-version race as corruption.
        let _retirement_owner = self.retirement_coordinator.lock().map_err(|_| {
            V5TaskProjectionFailure::fail_stop(ReceiptLedgerError::Corrupt(
                "protocol-v5 task retirement coordinator is poisoned",
            ))
        })?;
        if Instant::now() >= deadline {
            return Err(V5TaskProjectionFailure {
                error: ReceiptLedgerError::DeadlineExceeded,
                fail_stop: false,
            });
        }
        let provider_deadline = crate::domain::code_intelligence::ProviderDeadline::new(deadline);
        let observed_at_epoch_ms = self.epoch_clock.now_epoch_millis();
        let snapshot = self
            .lifecycle_links
            .catalog_snapshot(provider_deadline)
            .map_err(V5TaskProjectionFailure::from_link_store)?;
        let mut pending = Vec::new();
        for entry in snapshot.entries() {
            let TaskLifecycleLinkCatalogEntry::Record(record) = entry else {
                continue;
            };
            match record {
                TaskLifecycleLinkRecord::TaskTerminalBound(terminal)
                    if observed_at_epoch_ms >= terminal.expires_at_epoch_ms() =>
                {
                    pending.push(
                        self.lifecycle_links
                            .begin_task_retirement(terminal, 64, 64, provider_deadline)
                            .map_err(V5TaskProjectionFailure::from_link_store)?,
                    );
                }
                TaskLifecycleLinkRecord::TaskRetirementPending(existing) => {
                    pending.push(existing.clone());
                }
                TaskLifecycleLinkRecord::TaskBound(_)
                | TaskLifecycleLinkRecord::TaskTerminalBound(_) => {}
            }
        }
        for pending in pending {
            let authorization = self.authorize_task_retirement(&pending, deadline)?;
            self.delete_terminal_authorized(&authorization, observed_at_epoch_ms, deadline)?;
            self.finalize_task_retirement(&authorization, deadline)?;
        }
        Ok(())
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

    fn startup_fail_stop(message: &'static str) -> V5TaskProjectionFailure {
        V5TaskProjectionFailure::fail_stop(ReceiptLedgerError::Corrupt(message))
    }

    fn materialize_bound_handoff(
        &self,
        handoff: &TaskHandoffActorBoundReceipt,
        bind_epoch_ms: u64,
        deadline: Instant,
        hooks: &dyn V5RuntimeHooks,
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

    fn reserve_bound_handoff_link(
        &self,
        handoff: &TaskHandoffActorBoundReceipt,
        _epoch_ms: u64,
        deadline: Instant,
        hooks: &dyn V5RuntimeHooks,
    ) -> Result<TaskLinkReservation, V5TaskProjectionFailure> {
        let reservation = self
            .lifecycle_links
            .reserve_task_link(
                handoff.key().clone(),
                handoff.link().clone(),
                crate::domain::code_intelligence::ProviderDeadline::new(deadline),
            )
            .map_err(V5TaskProjectionFailure::from_link_store)?;
        hooks.event(
            V5ReceiptRuntimeEventKind::TaskLinkCapacityReserved,
            _epoch_ms,
        );
        Ok(reservation)
    }

    fn materialize_staged_bound_handoff(
        &self,
        handoff: &TaskHandoffActorBoundReceipt,
        reservation: &TaskLinkReservation,
        bind_epoch_ms: u64,
        deadline: Instant,
        hooks: &dyn V5RuntimeHooks,
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

    fn materialize_recovered_handoff(
        &self,
        handoff: &TaskHandoffActorBoundReceipt,
        bind_epoch_ms: u64,
        deadline: Instant,
        hooks: &dyn V5RuntimeHooks,
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

    fn materialize_promised_actor_bound(
        &self,
        promised: &TaskPromisedActorBoundReceipt,
        bind_epoch_ms: u64,
        deadline: Instant,
        hooks: &dyn V5RuntimeHooks,
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
    fn materialize_actor_bound_task(
        &self,
        key: &ReceiptKey,
        link: &crate::application::receipt_ledger::TaskLinkReference,
        promised_task: &ReceiptTaskProjection,
        workspace_identity_hash: &crate::domain::invocation::SafeIdentityHash,
        cancel_requested: bool,
        phase: AttemptPhase,
        recovered_begun: bool,
        bind_epoch_ms: u64,
        deadline: Instant,
        hooks: &dyn V5RuntimeHooks,
    ) -> Result<(V5StoredInvocationRecord, TaskBoundReceipt), V5TaskProjectionFailure> {
        let provider_deadline = crate::domain::code_intelligence::ProviderDeadline::new(deadline);
        let reservation = self
            .lifecycle_links
            .reserve_task_link(key.clone(), link.clone(), provider_deadline)
            .map_err(V5TaskProjectionFailure::from_link_store)?;
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
    fn materialize_actor_bound_task_from_reservation(
        &self,
        key: &ReceiptKey,
        promised_task: &ReceiptTaskProjection,
        workspace_identity_hash: &crate::domain::invocation::SafeIdentityHash,
        cancel_requested: bool,
        phase: AttemptPhase,
        recovered_begun: bool,
        reservation: &TaskLinkReservation,
        bind_epoch_ms: u64,
        deadline: Instant,
        record_reservation_conversion: bool,
        hooks: &dyn V5RuntimeHooks,
    ) -> Result<(V5StoredInvocationRecord, TaskBoundReceipt), V5TaskProjectionFailure> {
        let provider_deadline = crate::domain::code_intelligence::ProviderDeadline::new(deadline);
        let identity = V5TaskIdentity::new(
            promised_task.task_id(),
            promised_task.invocation_id(),
            crate::application::receipt_ledger::receipt_key_digest(key),
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
                crate::domain::code_intelligence::ProviderDeadline::new(deadline),
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
        let provider_deadline = crate::domain::code_intelligence::ProviderDeadline::new(deadline);
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
        if expected.phase() != crate::application::receipt_ledger::AttemptPhase::Begun
            || record.cancel_requested
            || !matches!(&record.task, V5StoredTask::Queued)
        {
            return Ok((record, expected.clone()));
        }
        let provider_deadline = crate::domain::code_intelligence::ProviderDeadline::new(deadline);
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
        let provider_deadline = crate::domain::code_intelligence::ProviderDeadline::new(deadline);
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
                crate::domain::code_intelligence::ProviderDeadline::new(deadline),
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
        let provider_deadline = crate::domain::code_intelligence::ProviderDeadline::new(deadline);
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

    #[allow(clippy::too_many_arguments)]
    fn publish_bound_task_terminal(
        &self,
        expected: &TaskBoundReceipt,
        record: &V5StoredInvocationRecord,
        terminal: &crate::application::receipt_ledger::V5CanonicalTerminal,
        terminal_epoch_ms: u64,
        deadline: Instant,
        hooks: &dyn V5RuntimeHooks,
    ) -> Result<(V5StoredInvocationRecord, TaskTerminalBoundReceipt), V5TaskProjectionFailure> {
        let terminal_digest = terminal.digest().clone();
        let (publication, terminal_status) = match terminal.outcome() {
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
        };
        let provider_deadline = crate::domain::code_intelligence::ProviderDeadline::new(deadline);
        let identity = record.identity();
        let receipt_key_digest = record.receipt_key_digest.clone();
        let terminal_record = self
            .task_store
            .publish_terminal_exact(&identity, record.version, publication, provider_deadline)
            .map_err(|error| {
                V5TaskProjectionFailure::from_task_store(error, receipt_key_digest, true)
            })?;
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
        task_id: crate::domain::invocation::TaskId,
        deadline: Instant,
    ) -> Result<Option<V5StoredInvocationRecord>, V5TaskProjectionFailure> {
        let provider_deadline = crate::domain::code_intelligence::ProviderDeadline::new(deadline);
        let link = match self
            .lifecycle_links
            .read_by_task_id(task_id, provider_deadline)
        {
            Ok(link) => link,
            Err(TaskLifecycleLinkStoreError::NotFound { .. }) => return Ok(None),
            Err(error) => return Err(V5TaskProjectionFailure::from_link_store(error)),
        };
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

    fn cancel_bound_task(
        &self,
        task_id: crate::domain::invocation::TaskId,
        deadline: Instant,
    ) -> Result<Option<V5StoredInvocationRecord>, V5TaskProjectionFailure> {
        let Some(record) = self.read_bound_task(task_id, deadline)? else {
            return Ok(None);
        };
        if record.task.is_terminal() || record.cancel_requested {
            return Ok(Some(record));
        }
        let identity = record.identity();
        let receipt_key_digest = record.receipt_key_digest.clone();
        let cancelled = self
            .task_store
            .request_cancel_exact(
                &identity,
                record.version,
                crate::domain::code_intelligence::ProviderDeadline::new(deadline),
            )
            .map_err(|error| {
                V5TaskProjectionFailure::from_task_store(error, receipt_key_digest, true)
            })?;
        Ok(Some(cancelled))
    }

    fn cancel_exact_bound_task(
        &self,
        key: &ReceiptKey,
        deadline: Instant,
    ) -> Result<Option<V5StoredInvocationRecord>, V5TaskProjectionFailure> {
        let provider_deadline = crate::domain::code_intelligence::ProviderDeadline::new(deadline);
        let link = match self
            .lifecycle_links
            .read_by_task_id(key.reserved_task_id(), provider_deadline)
        {
            Ok(link) => link,
            Err(TaskLifecycleLinkStoreError::NotFound { .. }) => return Ok(None),
            Err(error) => return Err(V5TaskProjectionFailure::from_link_store(error)),
        };
        if link.key() != key {
            return Err(V5TaskProjectionFailure {
                error: ReceiptLedgerError::TaskBoundMismatch,
                fail_stop: false,
            });
        }
        self.cancel_bound_task(key.reserved_task_id(), deadline)
    }
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

fn lifecycle_entry_task_id(
    entry: &TaskLifecycleLinkCatalogEntry,
) -> crate::domain::invocation::TaskId {
    match entry {
        TaskLifecycleLinkCatalogEntry::Reservation(reservation) => {
            reservation.key().reserved_task_id()
        }
        TaskLifecycleLinkCatalogEntry::Record(record) => record.key().reserved_task_id(),
    }
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
            result: Box::new(crate::domain::invocation::DomainResult::success(
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
        schema_version: crate::application::invocation_store_v5::V5StoredInvocationSchemaVersion,
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
    link: &crate::application::receipt_ledger::TaskLinkReference,
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

struct V5TaskProjectionFailure {
    error: ReceiptLedgerError,
    fail_stop: bool,
}

impl V5TaskProjectionFailure {
    const fn fail_stop(error: ReceiptLedgerError) -> Self {
        Self {
            error,
            fail_stop: true,
        }
    }

    fn from_link_store(error: TaskLifecycleLinkStoreError) -> Self {
        match error {
            TaskLifecycleLinkStoreError::DeadlineExceeded => Self {
                error: ReceiptLedgerError::DeadlineExceeded,
                fail_stop: false,
            },
            TaskLifecycleLinkStoreError::Capacity { .. } => Self {
                error: ReceiptLedgerError::CapacityExceeded,
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

struct V5InvocationExecutor {
    invocation_runtime: Arc<V5CanonicalInvocationRuntime>,
}

impl V5InvocationExecutor {
    fn new(invocation_service: Arc<dyn CanonicalInvocationService>, clock: Arc<dyn Clock>) -> Self {
        Self::over(Arc::new(V5CanonicalInvocationRuntime::new(
            invocation_service,
            clock,
        )))
    }

    fn over(invocation_runtime: Arc<V5CanonicalInvocationRuntime>) -> Self {
        Self { invocation_runtime }
    }

    fn capture_response_deadline(
        &self,
        response_budget_ms: u64,
    ) -> crate::application::invocation::InvocationResponseDeadline {
        self.invocation_runtime
            .capture_response_deadline(response_budget_ms)
    }

    fn now(&self) -> Instant {
        self.invocation_runtime.now()
    }

    fn bind(
        &self,
        invocation: V5InvocationRequest,
        response_deadline: crate::application::invocation::InvocationResponseDeadline,
    ) -> Result<V5ActorBoundCanonicalInvocation, V5CanonicalPrepareError> {
        let tool = match invocation.tool() {
            crate::application::receipt_ledger::V5ToolIdentity::View => ToolIdentity::View,
            crate::application::receipt_ledger::V5ToolIdentity::Apply => ToolIdentity::Apply,
            crate::application::receipt_ledger::V5ToolIdentity::Resolve => ToolIdentity::Resolve,
            crate::application::receipt_ledger::V5ToolIdentity::Search => ToolIdentity::Search,
            crate::application::receipt_ledger::V5ToolIdentity::Check => ToolIdentity::Check,
            crate::application::receipt_ledger::V5ToolIdentity::Diff => ToolIdentity::Diff,
            crate::application::receipt_ledger::V5ToolIdentity::Run => ToolIdentity::Run,
            crate::application::receipt_ledger::V5ToolIdentity::Docs => ToolIdentity::Docs,
        };
        let request = super::protocol::InvocationRequest::new(
            tool,
            Value::Object(invocation.arguments().clone()),
            invocation.workspace_hint().to_owned(),
            invocation.response_budget_ms(),
        )
        .map_err(|error| {
            V5CanonicalPrepareError::Rejected(Box::new(
                crate::domain::invocation::DomainResult::canonical_rejection(
                    None,
                    RefusalCode::BadValue,
                    error,
                ),
            ))
        })?;
        self.invocation_runtime
            .bind_with_deadline(request, response_deadline)
    }
}

impl V5ReceiptRuntime {
    fn open(state: &DaemonStateDirectory, config: &DaemonServerConfig) -> Result<Self, String> {
        let epoch_clock = config
            .epoch_clock_for_v5()
            .unwrap_or_else(|| Arc::new(SystemEpochMillisClock));
        Self::open_with_epoch_clock(state, config, epoch_clock)
    }

    fn open_with_epoch_clock(
        state: &DaemonStateDirectory,
        config: &DaemonServerConfig,
        epoch_clock: Arc<dyn EpochMillisClock>,
    ) -> Result<Self, String> {
        let hooks = config
            .runtime_hooks_for_v5()
            .unwrap_or_else(|| Arc::new(NoHooks));
        let stable_authority = state.acquire_receipt_authority(AUTHORITY_ACQUIRE_TIMEOUT)?;
        let startup_deadline = Instant::now() + STARTUP_RECONCILIATION_TIMEOUT;
        let receipts = state.create_private_retained_subdirectory("receipts")?;
        let receipt_ledger =
            ReceiptLedgerStore::open_retained_directory_before(receipts, startup_deadline)
                .map_err(|error| format!("open protocol-v5 receipt ledger: {error}"))?;
        receipt_ledger
            .generation()
            .map_err(|error| format!("read protocol-v5 receipt generation: {error}"))?;
        let recovery_keys = receipt_ledger
            .recovery_keys(startup_deadline)
            .map_err(|error| format!("inspect protocol-v5 receipt recovery catalog: {error}"))?;
        let receipt_ledger = ReceiptLedgerActor::spawn(receipt_ledger);
        let task_projection =
            V5TaskProjection::open(state, Arc::clone(&epoch_clock), startup_deadline)?;
        let _task_recovery_entries = task_projection.recovery.entries().len();
        let runtime = Self {
            core_identity: config.core_identity.clone(),
            _stable_authority: stable_authority,
            receipt_ledger,
            epoch_clock,
            invocation_executor: {
                #[cfg(test)]
                let preset = config.canonical_runtime_for_v5();
                #[cfg(not(test))]
                let preset: Option<Arc<V5CanonicalInvocationRuntime>> = None;
                match preset {
                    Some(runtime) => V5InvocationExecutor::over(runtime),
                    None => V5InvocationExecutor::new(
                        config.invocation_service_for_v5(),
                        config.invocation_clock_for_v5(),
                    ),
                }
            },
            task_projection,
            active_task_cancellations: Arc::new(V5ActiveTaskCancellations::default()),
            task_execution_threads: Mutex::new(Vec::new()),
            task_terminal_coordinator: Mutex::new(()),
            external_store_fail_stop: AtomicBool::new(false),
            fail_stop_watchdogs: FailStopWatchdogs::default(),
            promoted_continuations: AtomicUsize::new(0),
            hooks,
        };
        if !config.skips_v5_startup_reconciliation() {
            runtime.preflight_existing_handoff_tasks(&recovery_keys, startup_deadline)?;
            runtime.reconcile_pre_task_startup(recovery_keys, startup_deadline)?;
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

    fn preflight_existing_handoff_tasks(
        &self,
        recovery_keys: &[ReceiptKey],
        deadline: Instant,
    ) -> Result<(), String> {
        let provider_deadline = crate::domain::code_intelligence::ProviderDeadline::new(deadline);
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
            let Some(recovery) = self
                .task_projection
                .recovery
                .entry(handoff.task().task_id())
            else {
                continue;
            };
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
            let expected = NewV5InvocationRecord::new(
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
            .with_initial_epoch_ms(handoff.task().created_at_epoch_ms());
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
        Ok(())
    }

    fn reconcile_pre_task_startup(
        &self,
        recovery_keys: Vec<ReceiptKey>,
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
                    let terminal = canonical_v5_terminal(&outcome).map_err(|error| {
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
                    let terminal = canonical_v5_terminal(&outcome).map_err(|error| {
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
                    let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Failed {
                        reason: V5SafeFailureReason::OutcomeUncertain,
                    })
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
                        .receipt_ledger
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
                    if let HandoffTerminalStage::Staged {
                        terminal_epoch_ms,
                        terminal,
                        ..
                    } = handoff.terminal_stage()
                    {
                        self.publish_staged_handoff_terminal_reply(
                            handoff.clone(),
                            terminal.clone(),
                            *terminal_epoch_ms,
                            deadline,
                        )
                        .map_err(|error| {
                            format!(
                                "reconcile protocol-v5 staged handoff terminal before listener: {error}"
                            )
                        })?;
                        continue;
                    }
                    let receipt_version = handoff.record_version();
                    let phase = handoff.phase();
                    let (record, task_bound) = self
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
                        })?;
                    let task_bound = self
                        .receipt_ledger
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

    fn ensure_named_authority(&self) -> Result<(), String> {
        self.ensure_named_authority_before(Instant::now() + AUTHORITY_ACQUIRE_TIMEOUT)
    }

    fn ensure_named_authority_before(&self, deadline: Instant) -> Result<(), String> {
        if self.external_store_fail_stop.load(Ordering::Acquire) {
            return Err("protocol-v5 external durable store requires reopen".to_owned());
        }
        self.receipt_ledger
            .generation(deadline)
            .map(|_| ())
            .map_err(|error| format!("validate protocol-v5 receipt authority: {error}"))
    }

    /// Whether the attempt on this thread must stop: the observer simulated
    /// the process's death, or the process latched fail-stop for real.
    /// A cancelled Working Task gets the grace to reach its terminal before
    /// the process fail-stops on it.
    fn arm_cancel_grace(&self, record: &V5StoredInvocationRecord) {
        if record.task == V5StoredTask::Working {
            self.fail_stop_watchdogs.arm(
                record.receipt_key_digest.clone(),
                self.invocation_executor.now(),
                FAIL_STOP_GRACE,
            );
        }
    }

    fn attempt_is_dead(&self) -> bool {
        self.hooks.process_exited() || self.restart_required()
    }

    fn restart_required(&self) -> bool {
        self.receipt_ledger.restart_required()
            || self.external_store_fail_stop.load(Ordering::Acquire)
    }

    fn project_task_failure(&self, failure: V5TaskProjectionFailure) -> ReceiptLedgerError {
        if failure.fail_stop {
            self.external_store_fail_stop.store(true, Ordering::Release);
        }
        failure.error
    }

    fn epoch_ms(&self) -> u64 {
        self.epoch_clock.now_epoch_millis()
    }

    fn cancel_invocation(
        &self,
        key: ReceiptKey,
        epoch_ms: u64,
        deadline: Instant,
    ) -> Result<V5RuntimeReply, ReceiptLedgerError> {
        self.validate_receipt_key(&key)?;
        {
            let _task_terminal_gate = self
                .task_terminal_coordinator
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(record) = self
                .task_projection
                .cancel_exact_bound_task(&key, deadline)
                .map_err(|failure| self.project_task_failure(failure))?
            {
                self.active_task_cancellations.cancel(record.task_id);
                self.arm_cancel_grace(&record);
                return Ok(V5RuntimeReply::Json(V5ServerResponse::Invocation {
                    outcome: V5InvocationResponse::Task {
                        snapshot: task_store_snapshot(&record),
                    },
                }));
            }
        }
        let resolution = self
            .receipt_ledger
            .request_cancel_or_reserve(key, epoch_ms, deadline)?;
        match decide_cancel_resolution(resolution) {
            CancelInvocationDecision::Accepted { receipt, .. } => {
                Ok(V5RuntimeReply::Json(pending_cancel_response(&receipt)))
            }
            CancelInvocationDecision::ExistingDirectTerminal(receipt) => self
                .reply_for_existing_state(ReceiptState::DirectTerminalUnacked(receipt), deadline),
            CancelInvocationDecision::Rejected(rejection) => {
                let state = rejection.into_state();
                let expected = match state {
                    ReceiptState::TaskPromisedUnbound(receipt) => {
                        let task_id = receipt.task().task_id();
                        let cancelled = self.receipt_ledger.request_task_cancel(
                            receipt.key().clone(),
                            TaskCancellationReceipt::PromisedUnbound(receipt),
                            deadline,
                        )?;
                        let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Cancelled)
                            .map_err(|_| {
                                ReceiptLedgerError::Corrupt("canonical v5 terminal failed")
                            })?;
                        let committed = self.receipt_ledger.publish_receipt_backed_task_terminal(
                            cancelled.key().clone(),
                            cancelled,
                            epoch_ms,
                            terminal,
                            deadline,
                        )?;
                        self.fail_stop_watchdogs.disarm(committed.key_digest());
                        self.hooks.receipt_backed_terminal(&committed)?;
                        self.hooks.release_pre_actor_pauses();
                        self.hooks.event(
                            V5ReceiptRuntimeEventKind::ReceiptTerminalCommitted,
                            epoch_ms,
                        );
                        let snapshot = self.resolve_task(task_id, deadline)?;
                        return Ok(V5RuntimeReply::Json(V5ServerResponse::Invocation {
                            outcome: V5InvocationResponse::Task { snapshot },
                        }));
                    }
                    ReceiptState::TaskPromisedActorBound(receipt) => {
                        TaskCancellationReceipt::PromisedActorBound(receipt)
                    }
                    ReceiptState::TaskHandoffActorBound(receipt) => {
                        TaskCancellationReceipt::HandoffActorBound(receipt)
                    }
                    ReceiptState::TaskReceiptOwnedActorBound(receipt) => {
                        TaskCancellationReceipt::ReceiptOwnedActorBound(receipt)
                    }
                    ReceiptState::TaskTerminalReceiptBacked(receipt) => {
                        let snapshot = self.resolve_task(receipt.task().task_id(), deadline)?;
                        return Ok(V5RuntimeReply::Json(V5ServerResponse::Invocation {
                            outcome: V5InvocationResponse::Task { snapshot },
                        }));
                    }
                    ReceiptState::Reserved(reserved)
                        if matches!(reserved.phase(), ReservedPhase::Begun { .. }) =>
                    {
                        // A running inline attempt: signal its token and give
                        // it the grace before the process fail-stops on it.
                        self.active_task_cancellations
                            .cancel(reserved.key().reserved_task_id());
                        self.fail_stop_watchdogs.arm(
                            crate::application::receipt_ledger::receipt_key_digest(reserved.key()),
                            self.invocation_executor.now(),
                            FAIL_STOP_GRACE,
                        );
                        return self
                            .reply_for_existing_state(ReceiptState::Reserved(reserved), deadline);
                    }
                    other => return self.reply_for_existing_state(other, deadline),
                };
                let cancelled = self.receipt_ledger.request_task_cancel(
                    expected.key().clone(),
                    expected,
                    deadline,
                )?;
                let snapshot = queued_receipt_task_snapshot(
                    cancelled.task(),
                    crate::application::receipt_ledger::receipt_key_digest(cancelled.key()),
                    cancelled.cancel_requested(),
                );
                Ok(V5RuntimeReply::Json(V5ServerResponse::Invocation {
                    outcome: V5InvocationResponse::Task { snapshot },
                }))
            }
        }
    }

    fn submit_invocation(
        self: &Arc<Self>,
        decoded: DecodedV5Request,
        epoch_ms: u64,
        deadline: Instant,
    ) -> Result<V5RuntimeReply, ReceiptLedgerError> {
        self.hooks
            .event(V5ReceiptRuntimeEventKind::V5ExecutorEntered, epoch_ms);
        // One opaque daemon-side deadline per request, captured before strict
        // validation and narrowed to the frontend budget: every later stage
        // measures against it and none of them starts a new clock.
        let response_deadline = match decoded.request() {
            V5ClientRequest::SubmitInvocation { invocation } => self
                .invocation_executor
                .capture_response_deadline(invocation.response_budget_ms()),
            _ => return Err(ReceiptLedgerError::InvocationIdentityMismatch),
        };
        let strict = decoded
            .into_strict_submit(&self.core_identity)
            .map_err(|_| ReceiptLedgerError::InvocationIdentityMismatch)?;
        let invocation = strict.invocation().clone();
        let (key, response_budget_ms) = strict.into_parts();
        let cutoff = OriginalCutoffDescriptor::new(epoch_ms, response_budget_ms)
            .map_err(|_| ReceiptLedgerError::TimestampOverflow)?;
        let outcome = self.receipt_ledger.reserve(key, cutoff, deadline)?;
        if matches!(&outcome, ReserveOutcome::Created(_)) {
            self.hooks
                .event(V5ReceiptRuntimeEventKind::ReceiptReserved, epoch_ms);
        }
        let decision = decide_cancel_reserved_submit(outcome).map_err(|_| {
            ReceiptLedgerError::Corrupt("canonical cancelled terminal could not be constructed")
        })?;
        match decision {
            CancelReservedSubmitDecision::ExecuteReserved(reservation) => self
                .execute_reserved_invocation(
                    reservation,
                    invocation,
                    response_deadline,
                    epoch_ms,
                    deadline,
                ),
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
        reservation: crate::application::receipt_ledger::ReservedReceipt,
        invocation: V5InvocationRequest,
        response_deadline: crate::application::invocation::InvocationResponseDeadline,
        epoch_ms: u64,
        deadline: Instant,
    ) -> Result<V5RuntimeReply, ReceiptLedgerError> {
        let slot = Arc::new(PipelineSlot::new());
        let worker = {
            let runtime = Arc::clone(self);
            let slot = Arc::clone(&slot);
            let reservation = reservation.clone();
            let response_deadline = response_deadline.clone();
            thread::Builder::new()
                .name("unica-v5-invocation-pipeline".to_owned())
                .spawn(move || {
                    let outcome = runtime.run_reserved_pipeline(
                        reservation,
                        invocation,
                        response_deadline,
                        &slot,
                        epoch_ms,
                        deadline,
                    );
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
        reservation: &crate::application::receipt_ledger::ReservedReceipt,
        slot: &PipelineSlot,
        epoch_ms: u64,
    ) -> Result<Option<V5RuntimeReply>, ReceiptLedgerError> {
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
                // Validation or admission still runs: it gets the grace, and
                // the process fail-stops if the actor is not bound by then.
                self.fail_stop_watchdogs.arm(
                    promised.key_digest().clone(),
                    self.invocation_executor.now(),
                    FAIL_STOP_GRACE,
                );
                let _ = epoch_ms;
                Ok(Some(V5RuntimeReply::Json(V5ServerResponse::Invocation {
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
                Ok(Some(V5RuntimeReply::Json(V5ServerResponse::Invocation {
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
                    return Ok(Some(V5RuntimeReply::Json(V5ServerResponse::Invocation {
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
                let (task_record, task_bound) = self
                    .task_projection
                    .materialize_bound_handoff(
                        &handoff,
                        cutoff_epoch_ms,
                        deadline,
                        self.hooks.as_ref(),
                    )
                    .map_err(|failure| self.project_task_failure(failure))?;
                self.receipt_ledger.complete_bound_task_handoff(
                    handoff.key().clone(),
                    handoff.record_version(),
                    task_bound.clone(),
                    deadline,
                )?;
                let (task_record, task_bound) = self
                    .task_projection
                    .start_bound_task(&task_bound, task_record, deadline)
                    .map_err(|failure| self.project_task_failure(failure))?;
                self.hooks.bound_task(&task_record, &task_bound);
                self.hooks.event(
                    V5ReceiptRuntimeEventKind::TaskBoundCommitted,
                    cutoff_epoch_ms,
                );
                let reply = V5RuntimeReply::Json(V5ServerResponse::Invocation {
                    outcome: V5InvocationResponse::Task {
                        snapshot: task_store_snapshot(&task_record),
                    },
                });
                slot.stash_owner_materialized(task_record, task_bound);
                Ok(Some(reply))
            }
        }
    }

    /// A fresh bound for the durable steps of an attempt the handler already
    /// answered for: the operation budget of the submit is spent by definition.
    fn continuation_deadline() -> Instant {
        Instant::now() + TASK_TERMINAL_PUBLICATION_TIMEOUT
    }

    /// The pipeline of one reserved invocation on the worker thread.
    fn run_reserved_pipeline(
        self: &Arc<Self>,
        reservation: crate::application::receipt_ledger::ReservedReceipt,
        invocation: V5InvocationRequest,
        response_deadline: crate::application::invocation::InvocationResponseDeadline,
        slot: &PipelineSlot,
        epoch_ms: u64,
        deadline: Instant,
    ) -> Result<V5RuntimeReply, ReceiptLedgerError> {
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
            let terminal = injected_rejection_terminal("scenario validation rejected invocation")?;
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
                        crate::domain::invocation::DomainResult::canonical_rejection(
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
        // The actor bind may run into the grace of a promised Task: the
        // handler promises at the handoff moment, and after it the watchdog,
        // not the admission checkpoint, bounds the bind.
        let actor_bound = match self.invocation_executor.bind(
            invocation,
            response_deadline.with_actor_admission_grace(FAIL_STOP_GRACE),
        ) {
            Ok(actor_bound) => actor_bound,
            Err(error) => {
                let fail_stop = matches!(error, V5CanonicalPrepareError::WorkspaceRegistryFailed);
                let outcome = match error {
                    V5CanonicalPrepareError::Direct(result)
                    | V5CanonicalPrepareError::Rejected(result) => {
                        ReceiptTerminalOutcome::Completed { result }
                    }
                    V5CanonicalPrepareError::WorkspaceCapacity => ReceiptTerminalOutcome::Failed {
                        reason: V5SafeFailureReason::WorkspaceCapacity,
                    },
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
        if let Some((task_record, bound)) = slot.take_owner_materialized() {
            let deadline = Self::continuation_deadline();
            let (cancellation, cancellation_guard) = self
                .active_task_cancellations
                .register(task_record.task_id)?;
            return self.drive_prepared_bound_task(
                actor_bound,
                task_record,
                bound,
                reservation.key().invocation_id(),
                cancellation,
                cancellation_guard,
                epoch_ms,
                deadline,
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
                let terminal = injected_rejection_terminal("scenario prepare rejected invocation")?;
                return self.publish_direct_terminal(begun, epoch_ms, terminal, deadline);
            }
        }
        if !slot.claim_cutoff() {
            // The handler promoted this begun attempt at the cutoff before the
            // drive claimed it: continue the handoff the handler committed.
            let deadline = Self::continuation_deadline();
            return match self.receipt_ledger.recover(begun.key().clone(), deadline)? {
                ReceiptState::TaskHandoffActorBound(handoff) => {
                    self.continue_promoted_begun_handoff(handoff, epoch_ms, deadline)
                }
                other => self.reply_for_existing_state(other, deadline),
            };
        }
        let (prepared, direct_outcome) = match self.drive_inline_invocation(actor_bound, &begun)? {
            InlineDrive::Rejected(result) => {
                let terminal = crate::application::receipt_ledger::canonical_v5_terminal(
                    &crate::application::receipt_ledger::ReceiptTerminalOutcome::Completed {
                        result,
                    },
                )
                .map_err(|_| ReceiptLedgerError::Corrupt("canonical v5 terminal failed"))?;
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
            // Another owner may have staged a terminal onto this receipt while
            // the pause held: re-read it so the Task is materialized from the
            // committed state, not from a version that went stale in the pause.
            let handoff = self.reread_handoff_after_pause(handoff, deadline)?;
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
            if let HandoffTerminalStage::Staged { terminal, .. } = handoff.terminal_stage() {
                let (terminal_record, terminal_link) = self
                    .task_projection
                    .publish_bound_task_terminal(
                        &bound,
                        &task_record,
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
                let terminal_link = self.receipt_ledger.complete_staged_task_handoff(
                    handoff.key().clone(),
                    handoff.record_version(),
                    terminal_link,
                    deadline,
                )?;
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
                return Ok(V5RuntimeReply::Json(V5ServerResponse::Invocation {
                    outcome: V5InvocationResponse::Task {
                        snapshot: task_store_snapshot(&terminal_record),
                    },
                }));
            }
            self.receipt_ledger.complete_bound_task_handoff(
                handoff.key().clone(),
                handoff.record_version(),
                bound.clone(),
                deadline,
            )?;
            let (cancellation, cancellation_guard) = self
                .active_task_cancellations
                .register(task_record.task_id)?;
            let (task_record, bound) = self
                .task_projection
                .start_bound_task(&bound, task_record, deadline)
                .map_err(|failure| self.project_task_failure(failure))?;
            self.hooks.bound_task(&task_record, &bound);
            self.hooks
                .event(V5ReceiptRuntimeEventKind::TaskBoundCommitted, epoch_ms);
            if self.hooks.holds(V5PausePoint::BeforeTaskTerminalReceipt)
                || self
                    .hooks
                    .holds(V5PausePoint::AfterTaskStoreTerminalBeforeLifecycleLinkTerminal)
            {
                // An observer holds the terminal receipt: the known-long attempt
                // runs on this thread so the pause sees its exact outcome.
                self.hooks
                    .pause(V5PausePoint::BeforeTaskTerminalReceipt, deadline)?;
                self.hooks.stage_entered(V5Stage::Execute);
                self.hooks
                    .event(V5ReceiptRuntimeEventKind::ExecuteEntered, epoch_ms);
                self.hooks
                    .callback_invocation_id(reservation.key().invocation_id());
                let result = prepared.execute(cancellation.clone());
                let outcome = if cancellation.is_cancelled() {
                    ReceiptTerminalOutcome::Cancelled
                } else {
                    match result {
                        Ok(result) => ReceiptTerminalOutcome::Completed {
                            result: Box::new(result),
                        },
                        Err(_) => ReceiptTerminalOutcome::Failed {
                            reason: V5SafeFailureReason::InvocationFailed,
                        },
                    }
                };
                self.hooks
                    .event(V5ReceiptRuntimeEventKind::ResultSerialized, epoch_ms);
                let snapshot = self.publish_task_execution_outcome(
                    &bound,
                    task_record.task_id,
                    outcome,
                    deadline,
                )?;
                return Ok(V5RuntimeReply::Json(V5ServerResponse::Invocation {
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
            return Ok(V5RuntimeReply::Json(V5ServerResponse::Invocation {
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
        let (terminal, oversized_result) = match canonical_v5_terminal(&outcome) {
            Ok(terminal) => (terminal, false),
            Err(CanonicalTerminalError::ResultTooLarge) => (
                canonical_v5_terminal(&ReceiptTerminalOutcome::Failed {
                    reason: V5SafeFailureReason::ResultTooLarge,
                })
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
        actor_bound: super::server::V5ActorBoundCanonicalInvocation,
        begun: &crate::application::receipt_ledger::ReservedReceipt,
    ) -> Result<InlineDrive, ReceiptLedgerError> {
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
                            return Ok(InlineDrive::HandedOff(Box::new(V5RuntimeReply::Json(
                                V5ServerResponse::Invocation {
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
                    return Ok(InlineDrive::HandedOff(Box::new(V5RuntimeReply::Json(
                        V5ServerResponse::Invocation {
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
                Ok(InlineDrive::HandedOff(Box::new(V5RuntimeReply::Json(
                    V5ServerResponse::Invocation {
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
        actor_bound: super::server::V5ActorBoundCanonicalInvocation,
        slot: Arc<InlineExecutionSlot>,
        cancellation: CancellationToken,
        invocation_id: crate::domain::invocation::InvocationId,
    ) {
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
            crate::application::operation_descriptors::ExecutionClass::KnownLong(_)
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
        prepared: super::server::V5PreparedCanonicalInvocation,
        cancellation: &CancellationToken,
        invocation_id: crate::domain::invocation::InvocationId,
    ) -> ReceiptTerminalOutcome {
        self.hooks.stage_entered(V5Stage::Execute);
        self.hooks
            .event(V5ReceiptRuntimeEventKind::ExecuteEntered, self.epoch_ms());
        self.hooks.callback_invocation_id(invocation_id);
        let result = prepared.execute(cancellation.clone());
        if cancellation.is_cancelled() {
            ReceiptTerminalOutcome::Cancelled
        } else {
            match result {
                Ok(result) => ReceiptTerminalOutcome::Completed {
                    result: Box::new(result),
                },
                Err(_) => ReceiptTerminalOutcome::Failed {
                    reason: V5SafeFailureReason::InvocationFailed,
                },
            }
        }
    }

    /// A settlement the handler does not own goes into the Task by the worker.
    fn publish_inline_settlement(
        self: &Arc<Self>,
        settlement: InlineSettlement,
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
        task_id: crate::domain::invocation::TaskId,
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
        begun: &crate::application::receipt_ledger::ReservedReceipt,
        slot: &InlineExecutionSlot,
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
            let terminal = match canonical_v5_terminal(&outcome) {
                Ok(terminal) => terminal,
                Err(CanonicalTerminalError::ResultTooLarge) => {
                    canonical_v5_terminal(&ReceiptTerminalOutcome::Failed {
                        reason: V5SafeFailureReason::ResultTooLarge,
                    })
                    .map_err(|_| {
                        ReceiptLedgerError::Corrupt("canonical result-too-large terminal failed")
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
        self.receipt_ledger.complete_bound_task_handoff(
            handoff.key().clone(),
            handoff.record_version(),
            bound.clone(),
            deadline,
        )?;
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
        actor_bound: super::server::V5ActorBoundCanonicalInvocation,
        promised: TaskPromisedUnboundReceipt,
        reservation: &crate::application::receipt_ledger::ReservedReceipt,
        epoch_ms: u64,
        deadline: Instant,
    ) -> Result<V5RuntimeReply, ReceiptLedgerError> {
        self.hooks
            .actor_workspace_identity(actor_bound.workspace_identity_hash());
        let actor_promised = self.receipt_ledger.bind_promised_task_actor(
            promised.key().clone(),
            promised.record_version(),
            actor_bound.workspace_identity_hash().clone(),
            deadline,
        )?;
        self.fail_stop_watchdogs.disarm(actor_promised.key_digest());
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
        // Another owner may have staged a terminal onto this receipt while the
        // pause held: re-read it so the Task is materialized from the committed
        // state, not from a version that went stale in the pause.
        let handoff = self.reread_handoff_after_pause(handoff, deadline)?;
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
        self.hooks
            .promised_actor_binding(&promised, &actor_promised, &bound);
        if let HandoffTerminalStage::Staged { terminal, .. } = handoff.terminal_stage() {
            let (terminal_record, terminal_link) = self
                .task_projection
                .publish_bound_task_terminal(
                    &bound,
                    &task_record,
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
            let terminal_link = self.receipt_ledger.complete_staged_task_handoff(
                handoff.key().clone(),
                handoff.record_version(),
                terminal_link,
                deadline,
            )?;
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
            return Ok(V5RuntimeReply::Json(V5ServerResponse::Invocation {
                outcome: V5InvocationResponse::Task {
                    snapshot: task_store_snapshot(&terminal_record),
                },
            }));
        }
        self.receipt_ledger.complete_bound_task_handoff(
            handoff.key().clone(),
            handoff.record_version(),
            bound.clone(),
            deadline,
        )?;
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
        actor_bound: super::server::V5ActorBoundCanonicalInvocation,
        handoff: TaskHandoffActorBoundReceipt,
        invocation_id: crate::domain::invocation::InvocationId,
        epoch_ms: u64,
        deadline: Instant,
    ) -> Result<V5RuntimeReply, ReceiptLedgerError> {
        let (task_record, task_bound) = self
            .task_projection
            .materialize_bound_handoff(&handoff, epoch_ms, deadline, self.hooks.as_ref())
            .map_err(|failure| self.project_task_failure(failure))?;
        self.receipt_ledger.complete_bound_task_handoff(
            handoff.key().clone(),
            handoff.record_version(),
            task_bound.clone(),
            deadline,
        )?;
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
        actor_bound: super::server::V5ActorBoundCanonicalInvocation,
        task_record: V5StoredInvocationRecord,
        bound: TaskBoundReceipt,
        invocation_id: crate::domain::invocation::InvocationId,
        epoch_ms: u64,
        deadline: Instant,
    ) -> Result<V5RuntimeReply, ReceiptLedgerError> {
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
                .cancel_exact_bound_task(bound.key(), deadline)
                .map_err(|failure| self.project_task_failure(failure))?
            {
                task_record = cancelled;
                self.hooks.bound_task(&task_record, &bound);
            }
        }
        if task_record.cancel_requested {
            let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Cancelled)
                .map_err(|_| ReceiptLedgerError::Corrupt("canonical v5 terminal failed"))?;
            let (terminal_record, terminal_link) = self
                .task_projection
                .publish_bound_task_terminal(
                    &bound,
                    &task_record,
                    &terminal,
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
            self.hooks
                .terminal_bound_task(&terminal_record, &terminal_link);
            return Ok(V5RuntimeReply::Json(V5ServerResponse::Invocation {
                outcome: V5InvocationResponse::Task {
                    snapshot: task_store_snapshot(&terminal_record),
                },
            }));
        }
        let (cancellation, cancellation_guard) = self
            .active_task_cancellations
            .register(task_record.task_id)?;
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
        actor_bound: super::server::V5ActorBoundCanonicalInvocation,
        task_record: V5StoredInvocationRecord,
        bound: TaskBoundReceipt,
        invocation_id: crate::domain::invocation::InvocationId,
        cancellation: CancellationToken,
        cancellation_guard: V5ActiveTaskCancellationGuard,
        epoch_ms: u64,
        deadline: Instant,
    ) -> Result<V5RuntimeReply, ReceiptLedgerError> {
        if self.hooks.prepare_rejects() {
            let terminal = injected_rejection_terminal("scenario prepare rejected invocation")?;
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
                let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Completed { result })
                    .map_err(|_| ReceiptLedgerError::Corrupt("canonical v5 terminal failed"))?;
                return self.publish_bound_terminal_reply(
                    &bound,
                    &task_record,
                    &terminal,
                    epoch_ms,
                    deadline,
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
            self.hooks.stage_entered(V5Stage::Execute);
            self.hooks
                .event(V5ReceiptRuntimeEventKind::ExecuteEntered, epoch_ms);
            self.hooks.callback_invocation_id(invocation_id);
            let result = prepared.execute(cancellation.clone());
            let outcome = if cancellation.is_cancelled() {
                ReceiptTerminalOutcome::Cancelled
            } else {
                match result {
                    Ok(result) => ReceiptTerminalOutcome::Completed {
                        result: Box::new(result),
                    },
                    Err(_) => ReceiptTerminalOutcome::Failed {
                        reason: V5SafeFailureReason::InvocationFailed,
                    },
                }
            };
            if self.hooks.crash_after_side_effect() {
                return Err(ReceiptLedgerError::StoreUnavailable);
            }
            self.hooks
                .event(V5ReceiptRuntimeEventKind::ResultSerialized, epoch_ms);
            let snapshot = self.publish_task_execution_outcome(
                &bound,
                task_record.task_id,
                outcome,
                deadline,
            )?;
            drop(cancellation_guard);
            return Ok(V5RuntimeReply::Json(V5ServerResponse::Invocation {
                outcome: V5InvocationResponse::Task { snapshot },
            }));
        }
        // A known-long attempt runs on its own thread: the client already
        // holds the Task reply, so the continuation need not wait for it.
        if matches!(
            prepared.execution_class(),
            crate::application::operation_descriptors::ExecutionClass::KnownLong(_)
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
            return Ok(V5RuntimeReply::Json(V5ServerResponse::Invocation {
                outcome: V5InvocationResponse::Task {
                    snapshot: task_store_snapshot(&task_record),
                },
            }));
        }
        // A direct attempt promoted to a Task runs to its terminal on this
        // continuation thread; the client already holds the Task reply.
        self.hooks.stage_entered(V5Stage::Execute);
        self.hooks
            .event(V5ReceiptRuntimeEventKind::ExecuteEntered, epoch_ms);
        self.hooks.callback_invocation_id(invocation_id);
        let result = prepared.execute(cancellation.clone());
        let outcome = if cancellation.is_cancelled() {
            ReceiptTerminalOutcome::Cancelled
        } else {
            match result {
                Ok(result) => ReceiptTerminalOutcome::Completed {
                    result: Box::new(result),
                },
                Err(_) => ReceiptTerminalOutcome::Failed {
                    reason: V5SafeFailureReason::InvocationFailed,
                },
            }
        };
        if self.hooks.crash_after_side_effect() {
            return Err(ReceiptLedgerError::StoreUnavailable);
        }
        self.hooks
            .event(V5ReceiptRuntimeEventKind::ResultSerialized, epoch_ms);
        let snapshot =
            self.publish_task_execution_outcome(&bound, task_record.task_id, outcome, deadline);
        drop(cancellation_guard);
        snapshot.map(|snapshot| {
            V5RuntimeReply::Json(V5ServerResponse::Invocation {
                outcome: V5InvocationResponse::Task { snapshot },
            })
        })
    }

    fn spawn_task_execution(
        self: &Arc<Self>,
        prepared: super::server::V5PreparedCanonicalInvocation,
        bound: TaskBoundReceipt,
        task_record: V5StoredInvocationRecord,
        cancellation: CancellationToken,
        cancellation_guard: V5ActiveTaskCancellationGuard,
    ) -> Result<(), ReceiptLedgerError> {
        let runtime = Arc::clone(self);
        let execution = thread::Builder::new()
            .name("unica-v5-task-execution".to_owned())
            .spawn(move || {
                runtime.hooks.stage_entered(V5Stage::Execute);
                runtime.hooks.event(
                    V5ReceiptRuntimeEventKind::ExecuteEntered,
                    runtime.epoch_ms(),
                );
                let result = prepared.execute(cancellation.clone());
                let outcome = if cancellation.is_cancelled() {
                    ReceiptTerminalOutcome::Cancelled
                } else {
                    match result {
                        Ok(result) => ReceiptTerminalOutcome::Completed {
                            result: Box::new(result),
                        },
                        Err(_) => ReceiptTerminalOutcome::Failed {
                            reason: V5SafeFailureReason::InvocationFailed,
                        },
                    }
                };
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
        task_id: crate::domain::invocation::TaskId,
        candidate: ReceiptTerminalOutcome,
        deadline: Instant,
    ) -> Result<super::protocol_v5::V5DaemonTaskSnapshot, ReceiptLedgerError> {
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
        let provider_deadline = crate::domain::code_intelligence::ProviderDeadline::new(deadline);
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
        let outcome = if record.cancel_requested {
            ReceiptTerminalOutcome::Cancelled
        } else {
            candidate
        };
        let terminal = match canonical_v5_terminal(&outcome) {
            Ok(terminal) => terminal,
            Err(CanonicalTerminalError::ResultTooLarge) => {
                canonical_v5_terminal(&ReceiptTerminalOutcome::Failed {
                    reason: V5SafeFailureReason::ResultTooLarge,
                })
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
            V5RuntimeReply::Json(V5ServerResponse::Invocation {
                outcome: V5InvocationResponse::Task { snapshot },
            }) => Ok(snapshot),
            _ => Err(ReceiptLedgerError::Corrupt(
                "Task terminal publication returned a non-Task reply",
            )),
        }
    }

    fn join_task_executions(&self) {
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
        reservation: crate::application::receipt_ledger::ReservedReceipt,
        epoch_ms: u64,
        terminal: crate::application::receipt_ledger::V5CanonicalTerminal,
        deadline: Instant,
    ) -> Result<V5RuntimeReply, ReceiptLedgerError> {
        self.publish_direct_terminal_with_candidate(reservation, epoch_ms, terminal, deadline, None)
    }

    fn publish_direct_terminal_with_candidate(
        &self,
        reservation: crate::application::receipt_ledger::ReservedReceipt,
        epoch_ms: u64,
        terminal: crate::application::receipt_ledger::V5CanonicalTerminal,
        deadline: Instant,
        oversized_candidate: Option<&crate::domain::invocation::DomainResult>,
    ) -> Result<V5RuntimeReply, ReceiptLedgerError> {
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
        self.fail_stop_watchdogs
            .disarm(&crate::application::receipt_ledger::receipt_key_digest(
                reservation.key(),
            ));
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
        Ok(V5RuntimeReply::Prepared(publication.into_parts().1))
    }

    fn publish_bound_terminal_reply(
        &self,
        bound: &TaskBoundReceipt,
        task_record: &V5StoredInvocationRecord,
        terminal: &crate::application::receipt_ledger::V5CanonicalTerminal,
        epoch_ms: u64,
        deadline: Instant,
    ) -> Result<V5RuntimeReply, ReceiptLedgerError> {
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
        self.fail_stop_watchdogs
            .disarm(&task_record.receipt_key_digest);
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
        Ok(V5RuntimeReply::Json(V5ServerResponse::Invocation {
            outcome: V5InvocationResponse::Task {
                snapshot: task_store_snapshot(&terminal_record),
            },
        }))
    }

    /// Re-reads a handoff receipt after a pause that another owner could have
    /// used to stage a terminal onto it. A no-op without an observer: nothing
    /// pauses in production, so the receipt cannot have moved.
    fn reread_handoff_after_pause(
        &self,
        handoff: TaskHandoffActorBoundReceipt,
        deadline: Instant,
    ) -> Result<TaskHandoffActorBoundReceipt, ReceiptLedgerError> {
        if !self.hooks.observing() {
            return Ok(handoff);
        }
        match self
            .receipt_ledger
            .recover(handoff.key().clone(), deadline)?
        {
            ReceiptState::TaskHandoffActorBound(fresh) => Ok(fresh),
            _ => Ok(handoff),
        }
    }

    fn publish_staged_handoff_terminal_reply(
        &self,
        handoff: TaskHandoffActorBoundReceipt,
        terminal: crate::application::receipt_ledger::V5CanonicalTerminal,
        epoch_ms: u64,
        deadline: Instant,
    ) -> Result<V5RuntimeReply, ReceiptLedgerError> {
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
        let (task_record, task_bound) = self
            .task_projection
            .materialize_staged_bound_handoff(
                &staged,
                &reservation,
                epoch_ms,
                deadline,
                self.hooks.as_ref(),
            )
            .map_err(|failure| self.project_task_failure(failure))?;
        let (terminal_record, terminal_link) = self
            .task_projection
            .publish_bound_task_terminal(
                &task_bound,
                &task_record,
                &terminal,
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
        let terminal_link = self.receipt_ledger.complete_staged_task_handoff(
            staged.key().clone(),
            staged.record_version(),
            terminal_link,
            deadline,
        )?;
        self.hooks.staged_terminal_publication(
            &staged,
            &task_record,
            &terminal_record,
            &terminal_link,
        )?;
        self.hooks.event(
            V5ReceiptRuntimeEventKind::TaskTerminalBoundCommitted,
            epoch_ms,
        );
        self.hooks
            .terminal_bound_task(&terminal_record, &terminal_link);
        Ok(V5RuntimeReply::Json(V5ServerResponse::Invocation {
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
        reservation: crate::application::receipt_ledger::ReservedReceipt,
        outcome: ReceiptTerminalOutcome,
        fail_stop: bool,
        epoch_ms: u64,
        deadline: Instant,
    ) -> Result<V5RuntimeReply, ReceiptLedgerError> {
        let terminal = canonical_v5_terminal(&outcome)
            .map_err(|_| ReceiptLedgerError::Corrupt("canonical v5 terminal failed"))?;
        let reply = self.publish_pre_actor_terminal(reservation, epoch_ms, terminal, deadline);
        if fail_stop {
            self.hooks.restart_requested();
            self.external_store_fail_stop.store(true, Ordering::Release);
        }
        match (fail_stop, reply) {
            (true, Ok(V5RuntimeReply::Prepared(frame))) => {
                Ok(V5RuntimeReply::PreparedFailStop(frame))
            }
            (true, Ok(V5RuntimeReply::Json(response))) => {
                Ok(V5RuntimeReply::JsonFailStop(response))
            }
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
    ) -> Result<V5RuntimeReply, ReceiptLedgerError> {
        if self.hooks.prepare_rejects() {
            let terminal = injected_rejection_terminal("scenario prepare rejected invocation")?;
            return self
                .publish_staged_handoff_terminal_reply(handoff, terminal, epoch_ms, deadline);
        }
        let (task_record, task_bound) = self
            .task_projection
            .materialize_bound_handoff(&handoff, epoch_ms, deadline, self.hooks.as_ref())
            .map_err(|failure| self.project_task_failure(failure))?;
        self.receipt_ledger.complete_bound_task_handoff(
            handoff.key().clone(),
            handoff.record_version(),
            task_bound.clone(),
            deadline,
        )?;
        let (task_record, task_bound) = self
            .task_projection
            .start_bound_task(&task_bound, task_record, deadline)
            .map_err(|failure| self.project_task_failure(failure))?;
        self.hooks.bound_task(&task_record, &task_bound);
        self.hooks
            .event(V5ReceiptRuntimeEventKind::TaskBoundCommitted, epoch_ms);
        Ok(V5RuntimeReply::Json(V5ServerResponse::Invocation {
            outcome: V5InvocationResponse::Task {
                snapshot: task_store_snapshot(&task_record),
            },
        }))
    }

    fn publish_pre_actor_terminal(
        &self,
        reservation: crate::application::receipt_ledger::ReservedReceipt,
        epoch_ms: u64,
        terminal: crate::application::receipt_ledger::V5CanonicalTerminal,
        deadline: Instant,
    ) -> Result<V5RuntimeReply, ReceiptLedgerError> {
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
                self.fail_stop_watchdogs.disarm(receipt.key_digest());
                self.hooks.receipt_backed_terminal(&receipt)?;
                self.hooks.event(
                    V5ReceiptRuntimeEventKind::ReceiptTerminalCommitted,
                    epoch_ms,
                );
                let snapshot = self.resolve_task(task_id, deadline)?;
                Ok(V5RuntimeReply::Json(V5ServerResponse::Invocation {
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
    ) -> Result<V5RuntimeReply, ReceiptLedgerError> {
        match decision {
            CancelReservedSubmitDecision::ExecuteReserved(reservation) => Ok(V5RuntimeReply::Json(
                pending_reserved_response(&reservation),
            )),
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
                Ok(V5RuntimeReply::Prepared(publication.into_parts().1))
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

    fn recover_invocation(
        &self,
        key: ReceiptKey,
        epoch_ms: u64,
        deadline: Instant,
    ) -> Result<V5RuntimeReply, ReceiptLedgerError> {
        self.validate_receipt_key(&key)?;
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
                    || record.receipt_key_digest
                        != crate::application::receipt_ledger::receipt_key_digest(&key)
                    || record.tool != key.tool()
                    || record.normalized_arguments_hash != *key.normalized_arguments_hash()
                {
                    return Err(ReceiptLedgerError::InvocationIdentityMismatch);
                }
                return Ok(V5RuntimeReply::Json(V5ServerResponse::Invocation {
                    outcome: V5InvocationResponse::Task {
                        snapshot: task_store_snapshot(&record),
                    },
                }));
            }
            Err(error) => return Err(error),
        };
        match classify_recovered_receipt(state, epoch_ms) {
            CancelReservedRecoveryDecision::Current(state) => {
                let decision = decide_cancel_reserved_submit(ReserveOutcome::ExistingExact(*state))
                    .map_err(|_| {
                        ReceiptLedgerError::Corrupt(
                            "canonical recovery terminal could not be constructed",
                        )
                    })?;
                self.reply_for_cancel_submit_decision(
                    decision,
                    epoch_ms,
                    deadline,
                    "recovery",
                    "recovered_direct",
                )
            }
            CancelReservedRecoveryDecision::Expire(intent) => {
                let outcome = self.receipt_ledger.expire_cancel_reserved(
                    intent.key().clone(),
                    intent.expected_version(),
                    intent.expected_mutation_sequence(),
                    intent.observed_at_epoch_ms(),
                    deadline,
                )?;
                match classify_cancel_reserved_expiry_outcome(outcome) {
                    CancelReservedExpiryDecision::Expired => {
                        Err(ReceiptLedgerError::ReceiptNotFound)
                    }
                    CancelReservedExpiryDecision::Current(state) => {
                        let decision =
                            decide_cancel_reserved_submit(ReserveOutcome::ExistingExact(*state))
                                .map_err(|_| {
                                    ReceiptLedgerError::Corrupt(
                                        "canonical expiry-winner terminal could not be constructed",
                                    )
                                })?;
                        self.reply_for_cancel_submit_decision(
                            decision,
                            epoch_ms,
                            deadline,
                            "recovery",
                            "recovered_direct",
                        )
                    }
                }
            }
        }
    }

    fn acknowledge_invocation(
        &self,
        key: ReceiptKey,
        terminal_digest: TerminalDigest,
        epoch_ms: u64,
        deadline: Instant,
    ) -> Result<V5RuntimeReply, ReceiptLedgerError> {
        self.validate_receipt_key(&key)?;
        let acknowledged =
            self.receipt_ledger
                .acknowledge_direct(key, terminal_digest, epoch_ms, deadline)?;
        self.hooks.event(
            V5ReceiptRuntimeEventKind::AcknowledgementCommitted,
            epoch_ms,
        );
        Ok(V5RuntimeReply::Json(
            V5ServerResponse::InvocationAcknowledged {
                acknowledgement: V5AcknowledgedReceipt::from_receipt(&acknowledged),
            },
        ))
    }

    fn resolve_task(
        &self,
        task_id: crate::domain::invocation::TaskId,
        deadline: Instant,
    ) -> Result<super::protocol_v5::V5DaemonTaskSnapshot, ReceiptLedgerError> {
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
            crate::application::receipt_ledger::ReceiptTerminalOutcome::Completed { result } => {
                super::protocol_v5::V5DaemonTaskSnapshot::Completed {
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
            crate::application::receipt_ledger::ReceiptTerminalOutcome::Failed { reason } => {
                super::protocol_v5::V5DaemonTaskSnapshot::Failed {
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
            crate::application::receipt_ledger::ReceiptTerminalOutcome::Cancelled => {
                super::protocol_v5::V5DaemonTaskSnapshot::Cancelled {
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

    fn wait_task(
        &self,
        task_id: crate::domain::invocation::TaskId,
        wait_ms: u64,
        deadline: Instant,
    ) -> Result<super::protocol_v5::V5DaemonTaskSnapshot, ReceiptLedgerError> {
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

    fn cancel_task(
        &self,
        task_id: crate::domain::invocation::TaskId,
        deadline: Instant,
    ) -> Result<super::protocol_v5::V5DaemonTaskSnapshot, ReceiptLedgerError> {
        self.task_projection
            .retire_expired_terminal_tasks(deadline, self.hooks.as_ref())
            .map_err(|failure| self.project_task_failure(failure))?;
        {
            let _task_terminal_gate = self
                .task_terminal_coordinator
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(record) = self
                .task_projection
                .cancel_bound_task(task_id, deadline)
                .map_err(|failure| self.project_task_failure(failure))?
            {
                self.active_task_cancellations.cancel(task_id);
                self.arm_cancel_grace(&record);
                let provider_deadline =
                    crate::domain::code_intelligence::ProviderDeadline::new(deadline);
                let link = self
                    .task_projection
                    .lifecycle_links
                    .read_by_task_id(task_id, provider_deadline)
                    .map_err(|error| {
                        self.project_task_failure(V5TaskProjectionFailure::from_link_store(error))
                    })?;
                if let TaskLifecycleLinkRecord::TaskBound(bound) = link {
                    if bound.phase() == AttemptPhase::NotBegun && record.cancel_requested {
                        let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Cancelled)
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
            ReceiptState::TaskPromisedActorBound(receipt) => {
                (TaskCancellationReceipt::PromisedActorBound(receipt), true)
            }
            ReceiptState::TaskHandoffActorBound(receipt) => {
                let terminalize = receipt.phase() == AttemptPhase::NotBegun;
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
            let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Cancelled)
                .map_err(|_| ReceiptLedgerError::Corrupt("canonical v5 terminal failed"))?;
            let committed = self.receipt_ledger.publish_receipt_backed_task_terminal(
                cancelled.key().clone(),
                cancelled,
                self.epoch_ms(),
                terminal,
                deadline,
            )?;
            self.fail_stop_watchdogs.disarm(committed.key_digest());
            self.hooks.receipt_backed_terminal(&committed)?;
            self.hooks.release_pre_actor_pauses();
            self.hooks.event(
                V5ReceiptRuntimeEventKind::ReceiptTerminalCommitted,
                self.epoch_ms(),
            );
            return self.resolve_task(task_id, deadline);
        }
        Ok(queued_receipt_task_snapshot(
            cancelled.task(),
            crate::application::receipt_ledger::receipt_key_digest(cancelled.key()),
            cancelled.cancel_requested(),
        ))
    }

    fn reply_for_existing_state(
        &self,
        state: ReceiptState,
        deadline: Instant,
    ) -> Result<V5RuntimeReply, ReceiptLedgerError> {
        self.reply_for_existing_state_with_origin(state, deadline, "exact_duplicate", "direct")
    }

    fn reply_for_existing_state_with_origin(
        &self,
        state: ReceiptState,
        deadline: Instant,
        origin: &'static str,
        response_kind: &'static str,
    ) -> Result<V5RuntimeReply, ReceiptLedgerError> {
        match state {
            ReceiptState::CancelReserved(receipt) => {
                Ok(V5RuntimeReply::Json(pending_cancel_response(&receipt)))
            }
            ReceiptState::Reserved(reserved) => {
                Ok(V5RuntimeReply::Json(pending_reserved_response(&reserved)))
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
                Ok(V5RuntimeReply::Prepared(publication.into_parts().1))
            }
            ReceiptState::AcknowledgedTombstone(receipt) => {
                Ok(V5RuntimeReply::Json(V5ServerResponse::Invocation {
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
                Ok(V5RuntimeReply::Json(V5ServerResponse::Invocation {
                    outcome: V5InvocationResponse::Task { snapshot },
                }))
            }
            _ => Err(ReceiptLedgerError::ReceiptRowPresentUnsupported),
        }
    }

    fn validate_receipt_key(&self, key: &ReceiptKey) -> Result<(), ReceiptLedgerError> {
        if key.core_identity_digest() != self.core_identity.digest() {
            return Err(ReceiptLedgerError::InvocationIdentityMismatch);
        }
        Ok(())
    }
}

enum V5RuntimeReply {
    Json(V5ServerResponse),
    /// A closed reply written after the process latched fail-stop: the
    /// daemon answers this request and admits nothing further.
    JsonFailStop(V5ServerResponse),
    Prepared(PreparedWireFrame),
    PreparedFailStop(PreparedWireFrame),
}

impl V5RuntimeReply {
    /// The same reply as the closed one a fail-stopped process writes.
    fn after_fail_stop(self) -> Self {
        match self {
            Self::Json(response) => Self::JsonFailStop(response),
            Self::Prepared(frame) => Self::PreparedFailStop(frame),
            closed @ (Self::JsonFailStop(_) | Self::PreparedFailStop(_)) => closed,
        }
    }
}

/// The terminal an injected rejection publishes in place of the real stage.
fn injected_rejection_terminal(
    summary: &'static str,
) -> Result<crate::application::receipt_ledger::V5CanonicalTerminal, ReceiptLedgerError> {
    canonical_v5_terminal(&ReceiptTerminalOutcome::Completed {
        result: Box::new(
            crate::domain::invocation::DomainResult::canonical_rejection(
                None,
                RefusalCode::BadValue,
                summary,
            ),
        ),
    })
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
    receipt_key_digest: crate::application::receipt_ledger::ReceiptKeyDigest,
    cancel_requested: bool,
) -> super::protocol_v5::V5DaemonTaskSnapshot {
    super::protocol_v5::V5DaemonTaskSnapshot::Queued {
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
) -> Result<super::protocol_v5::V5DaemonTaskSnapshot, ReceiptLedgerError> {
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
        ReceiptTerminalOutcome::Completed { result } => {
            super::protocol_v5::V5DaemonTaskSnapshot::Completed {
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
        ReceiptTerminalOutcome::Failed { reason } => {
            super::protocol_v5::V5DaemonTaskSnapshot::Failed {
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
        ReceiptTerminalOutcome::Cancelled => super::protocol_v5::V5DaemonTaskSnapshot::Cancelled {
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

fn task_store_snapshot(
    record: &V5StoredInvocationRecord,
) -> super::protocol_v5::V5DaemonTaskSnapshot {
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
        V5StoredTask::Queued => super::protocol_v5::V5DaemonTaskSnapshot::Queued {
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
        V5StoredTask::Working => super::protocol_v5::V5DaemonTaskSnapshot::Working {
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
        } => super::protocol_v5::V5DaemonTaskSnapshot::Completed {
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
        } => super::protocol_v5::V5DaemonTaskSnapshot::Failed {
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
        } => super::protocol_v5::V5DaemonTaskSnapshot::Cancelled {
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

fn task_snapshot_is_terminal(snapshot: &super::protocol_v5::V5DaemonTaskSnapshot) -> bool {
    matches!(
        snapshot,
        super::protocol_v5::V5DaemonTaskSnapshot::Completed { .. }
            | super::protocol_v5::V5DaemonTaskSnapshot::Failed { .. }
            | super::protocol_v5::V5DaemonTaskSnapshot::Cancelled { .. }
    )
}

fn pending_cancel_response(
    receipt: &crate::application::receipt_ledger::CancelReservedReceipt,
) -> V5ServerResponse {
    V5ServerResponse::Invocation {
        outcome: V5InvocationResponse::ReceiptPending {
            receipt_key: receipt.key().clone(),
            phase: V5InvocationPhase::CancelReserved,
            accepted_epoch_ms: receipt.cancel_reserved_at_epoch_ms(),
            original_budget_ms: 0,
            cancel_requested: true,
        },
    }
}

fn pending_reserved_response(
    receipt: &crate::application::receipt_ledger::ReservedReceipt,
) -> V5ServerResponse {
    let phase = match receipt.phase() {
        ReservedPhase::Unbound => V5InvocationPhase::ReservedUnbound,
        ReservedPhase::ActorBound { .. } => V5InvocationPhase::ReservedActorBound,
        ReservedPhase::Begun { .. } => V5InvocationPhase::ReservedBegun,
    };
    V5ServerResponse::Invocation {
        outcome: V5InvocationResponse::ReceiptPending {
            receipt_key: receipt.key().clone(),
            phase,
            accepted_epoch_ms: receipt.original_cutoff().accepted_epoch_ms(),
            original_budget_ms: receipt.original_cutoff().response_budget_ms(),
            cancel_requested: receipt.cancel_requested(),
        },
    }
}

pub(crate) fn run_daemon(config: DaemonServerConfig) -> Result<(), String> {
    run_daemon_configured(config, |runtime| runtime)
}

fn run_daemon_configured(
    config: DaemonServerConfig,
    configure_runtime: impl FnOnce(V5ReceiptRuntime) -> V5ReceiptRuntime,
) -> Result<(), String> {
    run_daemon_configured_until(config, configure_runtime, || false)
}

fn run_daemon_configured_until(
    config: DaemonServerConfig,
    configure_runtime: impl FnOnce(V5ReceiptRuntime) -> V5ReceiptRuntime,
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
    // a listener can become discoverable.
    let runtime = Arc::new(configure_runtime(V5ReceiptRuntime::open(&state, &config)?));
    runtime.hooks.runtime_opened(&runtime);
    if let Err(error) = runtime.ensure_named_authority() {
        if runtime.restart_required() {
            std::mem::forget(runtime);
        }
        return Err(error);
    }
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .map_err(|error| daemon_io_error("bind protocol-v5 loopback endpoint", error))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| daemon_io_error("configure protocol-v5 listener", error))?;
    let port = listener
        .local_addr()
        .map_err(|error| daemon_io_error("inspect protocol-v5 listener", error))?
        .port();
    let record = V5EndpointRecord::new(config.core_identity.clone(), port)?;
    let published = state.publish_v5_endpoint_record(&record)?;
    if let Err(error) = runtime.ensure_named_authority() {
        if runtime.restart_required() {
            drop(listener);
            std::mem::forget(runtime);
            return Ok(());
        }
        let _ = state.remove_v5_endpoint_if_owned(&published);
        return Err(error);
    }
    let listener_lease = runtime.hooks.listener_lease();
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
        // fail-stop latch.
        if let Some(elapsed) = runtime
            .fail_stop_watchdogs
            .due(runtime.invocation_executor.now())
        {
            // A promised attempt without its actor, or a cancelled attempt
            // without its terminal, outlived the grace: the process stops
            // admitting and dies, and the successor terminalizes it.
            runtime
                .external_store_fail_stop
                .store(true, Ordering::Release);
            runtime.hooks.restart_requested();
            restart_requested = true;
            runtime.hooks.forced_process_exit(Some(elapsed));
            break;
        }
        if runtime.restart_required() {
            restart_requested = true;
            runtime.hooks.forced_process_exit(None);
            break;
        }
        match listener.accept() {
            Ok((stream, address)) if address.ip().is_loopback() => {
                if runtime.restart_required() {
                    drop(stream);
                    restart_requested = true;
                    runtime.hooks.forced_process_exit(None);
                    break;
                }
                let connection = V5AcceptedConnection {
                    stream,
                    handshake_deadline: Instant::now() + HANDSHAKE_READ_TIMEOUT,
                };
                match V5ConnectionSlot::acquire(Arc::clone(&admitted_connections)) {
                    Some(slot) => sessions.push(spawn_v5_connection_handler(
                        connection,
                        record.clone(),
                        Arc::clone(&active_leases),
                        Arc::clone(&shutting_down),
                        Arc::clone(&runtime),
                        slot,
                    )),
                    None => reject_overloaded_v5_connection(connection),
                }
            }
            Ok((_stream, _)) => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(error) => {
                shutting_down.store(true, Ordering::Release);
                join_v5_handlers(sessions);
                let _ = state.remove_v5_endpoint_if_owned(&published);
                return Err(daemon_io_error("accept protocol-v5 connection", error));
            }
        }

        sessions = reap_finished_v5_handlers(sessions);
        if active_leases.is_empty()?
            && admitted_connections.load(Ordering::Acquire) == 0
            && runtime.active_task_cancellations.is_empty()
        {
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
        if runtime.hooks.releases_authority_on_fail_stop() {
            // The contract harness runs the daemon in a thread; joining that
            // thread is its simulated process-death boundary. Release the
            // authority so unrelated scenarios do not share resources that
            // production releases at PID exit. A promoted attempt the owner
            // handed to a worker still holds an `Arc` to this runtime; join
            // those workers first so the drop actually releases the authority
            // an off-thread continuation would otherwise keep alive.
            join_v5_handlers(sessions);
            runtime.join_task_executions();
            drop(runtime);
        } else {
            drop(sessions);
            std::mem::forget(runtime);
        }
        return Ok(());
    }
    join_v5_handlers(sessions);
    runtime.join_task_executions();
    state.remove_v5_endpoint_if_owned(&published)?;
    Ok(())
}

struct V5AcceptedConnection {
    stream: TcpStream,
    handshake_deadline: Instant,
}

fn spawn_v5_connection_handler(
    connection: V5AcceptedConnection,
    record: V5EndpointRecord,
    active_leases: Arc<V5LeaseRegistry>,
    shutting_down: Arc<AtomicBool>,
    runtime: Arc<V5ReceiptRuntime>,
    slot: V5ConnectionSlot,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let _ = handle_probe_connection(
            connection.stream,
            connection.handshake_deadline,
            &record,
            &active_leases,
            &shutting_down,
            &runtime,
            slot,
        );
    })
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
        if leases.len() >= MAX_OWNER_SESSIONS {
            return Ok(V5LeaseAdmission::Capacity);
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
    Capacity,
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
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                (current < MAX_HANDSHAKES).then_some(current + 1)
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

fn handle_probe_connection(
    mut stream: TcpStream,
    handshake_deadline: Instant,
    record: &V5EndpointRecord,
    active_leases: &Arc<V5LeaseRegistry>,
    shutting_down: &AtomicBool,
    runtime: &Arc<V5ReceiptRuntime>,
    handshake_slot: V5ConnectionSlot,
) -> Result<(), String> {
    runtime.ensure_named_authority_before(handshake_deadline)?;
    stream
        .set_nonblocking(false)
        .map_err(|error| daemon_io_error("configure protocol-v5 client stream", error))?;
    let reader_stream = stream
        .try_clone()
        .map_err(|error| daemon_io_error("clone protocol-v5 client stream", error))?;
    let mut reader = BufReader::new(reader_stream);
    let decoded = match read_v5_request_before(&mut reader, handshake_deadline) {
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
        V5LeaseAdmission::Capacity => {
            write_runtime_probe_error_before(
                &mut stream,
                runtime,
                V5DaemonErrorCode::OwnerCapacity,
                handshake_deadline,
            )?;
            return Ok(());
        }
    };
    // The owner lease fences listener shutdown before pre-authentication admission
    // is released, so idle observation cannot see a gap between the two states.
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
            match read_v5_request_before(&mut reader, session_read_deadline) {
                Ok(observation) => observation,
                Err(V5RequestFrameError::InvalidRequest(_)) => {
                    write_runtime_probe_error_before(
                        &mut stream,
                        runtime,
                        V5DaemonErrorCode::InvalidRequest,
                        session_read_deadline,
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
                        session_read_deadline,
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
        let deadlines = match runtime.hooks.session_deadline_override() {
            Some(bulk_deadline) => V5RequestDeadlines {
                operation: bulk_deadline,
                response: bulk_deadline,
            },
            None => v5_request_deadlines(&decoded, request_received_at)?,
        };
        runtime.hooks.event(
            V5ReceiptRuntimeEventKind::StrictEnvelopeParsed,
            runtime.epoch_ms(),
        );
        runtime.hooks.event(
            V5ReceiptRuntimeEventKind::V5ReceiptRuntimeEntered,
            runtime.epoch_ms(),
        );
        runtime.ensure_named_authority_before(deadlines.operation)?;
        let kind = decoded.request().kind();
        let _actor_lease =
            (kind == V5ClientRequestKind::SubmitInvocation).then(|| runtime.hooks.actor_lease());
        let result = match kind {
            V5ClientRequestKind::Ping => write_runtime_json_line_before(
                &mut stream,
                runtime,
                &V5ProbeServerResponse::Pong {},
                deadlines.response,
            ),
            V5ClientRequestKind::SubmitInvocation => {
                let epoch_ms = runtime.epoch_ms();
                match runtime.submit_invocation(decoded, epoch_ms, deadlines.operation) {
                    Ok(reply) => {
                        if runtime.hooks.submit_response_disconnect() {
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
                        if runtime.hooks.ack_response_disconnect() {
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
    operation: Instant,
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
    Ok(V5RequestDeadlines {
        operation,
        response,
    })
}

fn read_v5_request_before(
    reader: &mut BufReader<TcpStream>,
    deadline: Instant,
) -> Result<(DecodedV5Request, Instant), V5RequestFrameError> {
    let raw_frame = read_bounded_v5_request_frame_before(reader, |reader| {
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

fn write_runtime_probe_error_before(
    stream: &mut TcpStream,
    runtime: &V5ReceiptRuntime,
    code: V5DaemonErrorCode,
    deadline: Instant,
) -> Result<(), String> {
    write_runtime_json_line_before(stream, runtime, &V5ServerResponse::Error { code }, deadline)
}

fn write_runtime_ledger_error_before(
    stream: &mut TcpStream,
    runtime: &V5ReceiptRuntime,
    error: &ReceiptLedgerError,
    deadline: Instant,
) -> Result<(), String> {
    let response = V5ServerResponse::Error {
        code: daemon_error_code(error),
    };
    if error.requires_reopen() {
        runtime.hooks.forced_process_exit(None);
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
            daemon_io_error("configure fail-stop protocol-v5 response timeout", error)
        })?;
    stream
        .write_all(&bytes)
        .map_err(|error| daemon_io_error("write fail-stop protocol-v5 response", error))
}

fn write_runtime_reply_before(
    stream: &mut TcpStream,
    runtime: &V5ReceiptRuntime,
    reply: V5RuntimeReply,
    deadline: Instant,
) -> Result<(), String> {
    match reply {
        V5RuntimeReply::Json(response) => {
            write_runtime_json_line_before(stream, runtime, &response, deadline)
        }
        V5RuntimeReply::JsonFailStop(response) => {
            write_fail_stop_json_line(stream, &response, deadline)
        }
        V5RuntimeReply::Prepared(frame) => {
            runtime.ensure_named_authority_before(deadline)?;
            if frame.jsonl().len() > MAX_V5_RESPONSE_LINE_BYTES {
                return Err("prepared protocol-v5 response exceeds the byte limit".to_string());
            }
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .filter(|remaining| !remaining.is_zero())
                .ok_or_else(|| "protocol-v5 response deadline expired".to_string())?;
            stream.set_write_timeout(Some(remaining)).map_err(|error| {
                daemon_io_error("configure prepared protocol-v5 response timeout", error)
            })?;
            stream
                .write_all(frame.jsonl())
                .map_err(|error| daemon_io_error("write prepared protocol-v5 response", error))?;
            deadline
                .checked_duration_since(Instant::now())
                .filter(|remaining| !remaining.is_zero())
                .map(|_| ())
                .ok_or_else(|| "protocol-v5 response deadline expired".to_string())
        }
        V5RuntimeReply::PreparedFailStop(frame) => {
            if frame.jsonl().len() > MAX_V5_RESPONSE_LINE_BYTES {
                return Err("prepared protocol-v5 response exceeds the byte limit".to_string());
            }
            let write_timeout = fail_stop_response_write_timeout(deadline, Instant::now());
            stream
                .set_write_timeout(Some(write_timeout))
                .map_err(|error| {
                    daemon_io_error("configure fail-stop prepared response timeout", error)
                })?;
            stream
                .write_all(frame.jsonl())
                .map_err(|error| daemon_io_error("write fail-stop prepared response", error))
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

fn write_runtime_json_line_before<T: Serialize>(
    stream: &mut TcpStream,
    runtime: &V5ReceiptRuntime,
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
        .map_err(|error| daemon_io_error("configure protocol-v5 response timeout", error))?;
    stream
        .write_all(&bytes)
        .map_err(|error| daemon_io_error("write protocol-v5 response", error))?;
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

fn daemon_io_error(operation: &str, error: io::Error) -> String {
    format!("{operation}: {error}")
}

#[cfg(test)]
mod tests;
