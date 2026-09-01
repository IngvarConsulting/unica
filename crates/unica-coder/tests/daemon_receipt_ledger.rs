//! Black-box RED contract for the protocol-v5 durable ReceiptLedger.
//!
//! The hidden bridge accepts actions only. Expected product outcomes, metrics and missing-boundary
//! hints are never sent to the implementation: this file validates a closed production-missing
//! envelope dynamically against the indexed action, or decodes evidence captured from real stores,
//! protocol sessions, daemon processes, barriers and clocks and performs every assertion here. The
//! bridge is not a second ReceiptLedger and must not synthesize observations from a scenario name.

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use unica_coder::receipt_ledger_test_support::{
    canonical_v5_terminal_for_test, execute_scenario_json, receipt_key_digest_for_test,
    request_scope_hash_for_test, task_link_digest_for_test,
};

const CUTOFF_MS: u64 = 7_000;
const CANCEL_RESERVATION_TTL_MS: u64 = 7_125;
const CLEANUP_GRACE_MS: u64 = 2_000;
const DIRECT_TASK_TTL_MS: u64 = 3_600_000;
const TOMBSTONE_TTL_MS: u64 = 900_000;
const MAX_RESPONSE_LINE_BYTES: u64 = 8_454_144;
const LIVE_RECEIPT_LIMIT: u64 = 64;
const LIVE_RECEIPT_BYTES_LIMIT: u64 = 541_065_216;
const TASK_LINK_LIMIT: u64 = 4_096;
const TASK_LINK_BYTES_LIMIT: u64 = 4_194_304;
const TOMBSTONE_LIMIT: u64 = 28_864;
const TOMBSTONE_BYTES_LIMIT: u64 = 14_778_368;
const MAX_CANONICAL_RESULT_BYTES: u64 = 8 * 1_024 * 1_024;
const MAX_PROTOCOL_FRAME_BYTES: usize = 8 * 1_024 * 1_024 + 64 * 1_024;
const MAX_SCENARIO_REPORT_BYTES: usize = 64 * 1_024 * 1_024;
const SUCCESS_SUMMARY: &str = "canonical-success";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
enum ClockMode {
    Fake,
    Wall,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
struct Scenario {
    clock: ClockMode,
    actions: Vec<Action>,
}

impl Scenario {
    fn fake(actions: Vec<Action>) -> Self {
        Self {
            clock: ClockMode::Fake,
            actions,
        }
    }

    fn wall(actions: Vec<Action>) -> Self {
        Self {
            clock: ClockMode::Wall,
            actions,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum Action {
    ConfigureValidation {
        reject: bool,
    },
    ConfigureProvider {
        execution_class: ExecutionClass,
        terminal: TerminalFixture,
        cooperative_cancel: bool,
        side_effect_marker: bool,
    },
    ConfigureAdmission {
        rejection: Option<WorkspaceAdmissionFailure>,
    },
    ConfigurePrepare {
        reject: bool,
    },
    InstallBarrier {
        point: BarrierPoint,
    },
    ReleaseBarrier {
        point: BarrierPoint,
    },
    WaitForEvent {
        event: EventKind,
    },
    WaitForEventCount {
        event: EventKind,
        count: u32,
    },
    WaitForOperation {
        label: String,
        state: OperationState,
    },
    Submit {
        request: RequestCase,
        response_budget_ms: u64,
        disconnect: DisconnectPoint,
        label: String,
    },
    SendOuterEnvelope {
        envelope: EnvelopeCase,
        label: String,
    },
    Recover {
        key: KeyCase,
        label: String,
    },
    Acknowledge {
        key: KeyCase,
        digest: DigestCase,
        disconnect: AckDisconnectPoint,
        label: String,
    },
    Cancel {
        key: KeyCase,
        lazy_session: bool,
        label: String,
    },
    CancelTask {
        api: TaskCancelApi,
        task: TaskSelector,
        lazy_session: bool,
        label: String,
    },
    SpawnSubmit {
        request: RequestCase,
        response_budget_ms: u64,
        disconnect: DisconnectPoint,
        label: String,
    },
    SpawnCancel {
        key: KeyCase,
        lazy_session: bool,
        label: String,
    },
    SpawnMarkReservedBegun {
        proof: ActorProofCase,
        label: String,
    },
    SpawnStageBoundHandoffTerminal {
        terminal: TerminalFixture,
        label: String,
    },
    JoinOperation {
        label: String,
    },
    ReadTask {
        api: TaskApi,
        label: String,
    },
    AdvanceMonotonic {
        millis: u64,
    },
    AdvanceEpoch {
        millis: u64,
    },
    Crash {
        point: CrashPoint,
    },
    Restart,
    Checkpoint {
        label: String,
    },
    Reset,
    ProbeProtocol {
        client: ProtocolVersion,
        server: ProtocolVersion,
        message: ProtocolMessage,
        label: String,
    },
    CompareClientServerIdentity,
    InjectPersistedIdentityCollision {
        index: IdentityIndex,
    },
    RunCrossStoreCrashWorkload {
        cases: Vec<CrashWorkload>,
    },
    RunTaskRetirementWorkload {
        cases: Vec<TaskRetirementWorkload>,
    },
    SeedReceipt {
        state: SeedReceiptState,
        cancel_requested: bool,
        staged_terminal: Option<TerminalFixture>,
    },
    SeedTask {
        status: TaskStatus,
        cancel_requested: bool,
        receipt_link: ReceiptLinkCase,
        identity: IdentityRelation,
        version: u64,
    },
    SeedTaskLinkReservation {
        relation: IdentityRelation,
    },
    OpenTaskStoreInspectOnly,
    ReconcileStartup,
    PublishListener,
    InvalidateActorProof {
        proof: ActorProofCase,
        point: BarrierPoint,
        label: String,
    },
    AttemptBoundTaskStart {
        proof: ActorProofCase,
        label: String,
    },
    RunLazyCancelStorm {
        submits: u32,
        cancels: u32,
        per_cancel_deadline_ms: u64,
        label: String,
    },
    FillReceiptPool {
        state: SeedReceiptState,
        count: u32,
    },
    FillTaskLinks,
    FillTaskLinksLeavingOneReservationSlot,
    FillTombstones,
    SpawnTaskStoreCreateAndBindUnderGate {
        label: String,
    },
    AttemptTaskStoreBindUnderGate {
        label: String,
    },
    ContinueReceiptOwnedAttempt {
        terminal: TerminalFixture,
        label: String,
    },
    AttemptStagedTerminalAgainstProvisional {
        mismatch: Option<ProvisionalMismatchField>,
        repeat_same_terminal: bool,
        label: String,
    },
    RunDirectLoad {
        calls: u32,
        duration_ms: u64,
        concurrency: u32,
        retained_receipt_terminals: u32,
        immediate_ack: bool,
        label: String,
    },
    RotateReceiptSegments,
    ReclaimExpiredEvidence,
    InjectTaskStoreCapacityInvariantViolationOnce,
    InjectStoreFault {
        point: StoreFaultPoint,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ActionKind {
    ConfigureValidation,
    ConfigureProvider,
    ConfigureAdmission,
    ConfigurePrepare,
    InstallBarrier,
    ReleaseBarrier,
    WaitForEvent,
    WaitForEventCount,
    WaitForOperation,
    Submit,
    SendOuterEnvelope,
    Recover,
    Acknowledge,
    Cancel,
    CancelTask,
    SpawnSubmit,
    SpawnCancel,
    SpawnMarkReservedBegun,
    SpawnStageBoundHandoffTerminal,
    JoinOperation,
    ReadTask,
    AdvanceMonotonic,
    AdvanceEpoch,
    Crash,
    Restart,
    Checkpoint,
    Reset,
    ProbeProtocol,
    CompareClientServerIdentity,
    InjectPersistedIdentityCollision,
    RunCrossStoreCrashWorkload,
    RunTaskRetirementWorkload,
    SeedReceipt,
    SeedTask,
    SeedTaskLinkReservation,
    OpenTaskStoreInspectOnly,
    ReconcileStartup,
    PublishListener,
    InvalidateActorProof,
    AttemptBoundTaskStart,
    RunLazyCancelStorm,
    FillReceiptPool,
    FillTaskLinks,
    FillTaskLinksLeavingOneReservationSlot,
    FillTombstones,
    SpawnTaskStoreCreateAndBindUnderGate,
    AttemptTaskStoreBindUnderGate,
    ContinueReceiptOwnedAttempt,
    AttemptStagedTerminalAgainstProvisional,
    RunDirectLoad,
    RotateReceiptSegments,
    ReclaimExpiredEvidence,
    InjectTaskStoreCapacityInvariantViolationOnce,
    InjectStoreFault,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum OperationState {
    Blocked,
    Completed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ProductionBoundary {
    StrictEnvelopeValidation,
    V5ReceiptRuntime,
    V5Executor,
    ReceiptTransition,
    ReceiptIdentity,
    ProtocolNegotiation,
    TaskProjection,
    CrossStoreReconciliation,
    ActorLinearization,
    CapacityCoordination,
    RetentionReclamation,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum EvidenceCode {
    ProtocolVersionUnsupported,
    ProtocolBehaviorUnavailable,
    StrictEnvelopeObservationUnavailable,
    ReceiptRowAbsent,
    ReceiptTransitionUnavailable,
    ReceiptIdentityUnavailable,
    TaskProjectionUnavailable,
    CrossStoreIntentUnavailable,
    ActorFenceUnavailable,
    CapacityLatchUnavailable,
    RetentionIndexUnavailable,
    WriterPathUnavailable,
}

impl Action {
    fn kind(&self) -> ActionKind {
        match self {
            Self::ConfigureValidation { .. } => ActionKind::ConfigureValidation,
            Self::ConfigureProvider { .. } => ActionKind::ConfigureProvider,
            Self::ConfigureAdmission { .. } => ActionKind::ConfigureAdmission,
            Self::ConfigurePrepare { .. } => ActionKind::ConfigurePrepare,
            Self::InstallBarrier { .. } => ActionKind::InstallBarrier,
            Self::ReleaseBarrier { .. } => ActionKind::ReleaseBarrier,
            Self::WaitForEvent { .. } => ActionKind::WaitForEvent,
            Self::WaitForEventCount { .. } => ActionKind::WaitForEventCount,
            Self::WaitForOperation { .. } => ActionKind::WaitForOperation,
            Self::Submit { .. } => ActionKind::Submit,
            Self::SendOuterEnvelope { .. } => ActionKind::SendOuterEnvelope,
            Self::Recover { .. } => ActionKind::Recover,
            Self::Acknowledge { .. } => ActionKind::Acknowledge,
            Self::Cancel { .. } => ActionKind::Cancel,
            Self::CancelTask { .. } => ActionKind::CancelTask,
            Self::SpawnSubmit { .. } => ActionKind::SpawnSubmit,
            Self::SpawnCancel { .. } => ActionKind::SpawnCancel,
            Self::SpawnMarkReservedBegun { .. } => ActionKind::SpawnMarkReservedBegun,
            Self::SpawnStageBoundHandoffTerminal { .. } => {
                ActionKind::SpawnStageBoundHandoffTerminal
            }
            Self::JoinOperation { .. } => ActionKind::JoinOperation,
            Self::ReadTask { .. } => ActionKind::ReadTask,
            Self::AdvanceMonotonic { .. } => ActionKind::AdvanceMonotonic,
            Self::AdvanceEpoch { .. } => ActionKind::AdvanceEpoch,
            Self::Crash { .. } => ActionKind::Crash,
            Self::Restart => ActionKind::Restart,
            Self::Checkpoint { .. } => ActionKind::Checkpoint,
            Self::Reset => ActionKind::Reset,
            Self::ProbeProtocol { .. } => ActionKind::ProbeProtocol,
            Self::CompareClientServerIdentity => ActionKind::CompareClientServerIdentity,
            Self::InjectPersistedIdentityCollision { .. } => {
                ActionKind::InjectPersistedIdentityCollision
            }
            Self::RunCrossStoreCrashWorkload { .. } => ActionKind::RunCrossStoreCrashWorkload,
            Self::RunTaskRetirementWorkload { .. } => ActionKind::RunTaskRetirementWorkload,
            Self::SeedReceipt { .. } => ActionKind::SeedReceipt,
            Self::SeedTask { .. } => ActionKind::SeedTask,
            Self::SeedTaskLinkReservation { .. } => ActionKind::SeedTaskLinkReservation,
            Self::OpenTaskStoreInspectOnly => ActionKind::OpenTaskStoreInspectOnly,
            Self::ReconcileStartup => ActionKind::ReconcileStartup,
            Self::PublishListener => ActionKind::PublishListener,
            Self::InvalidateActorProof { .. } => ActionKind::InvalidateActorProof,
            Self::AttemptBoundTaskStart { .. } => ActionKind::AttemptBoundTaskStart,
            Self::RunLazyCancelStorm { .. } => ActionKind::RunLazyCancelStorm,
            Self::FillReceiptPool { .. } => ActionKind::FillReceiptPool,
            Self::FillTaskLinks => ActionKind::FillTaskLinks,
            Self::FillTaskLinksLeavingOneReservationSlot => {
                ActionKind::FillTaskLinksLeavingOneReservationSlot
            }
            Self::FillTombstones => ActionKind::FillTombstones,
            Self::SpawnTaskStoreCreateAndBindUnderGate { .. } => {
                ActionKind::SpawnTaskStoreCreateAndBindUnderGate
            }
            Self::AttemptTaskStoreBindUnderGate { .. } => ActionKind::AttemptTaskStoreBindUnderGate,
            Self::ContinueReceiptOwnedAttempt { .. } => ActionKind::ContinueReceiptOwnedAttempt,
            Self::AttemptStagedTerminalAgainstProvisional { .. } => {
                ActionKind::AttemptStagedTerminalAgainstProvisional
            }
            Self::RunDirectLoad { .. } => ActionKind::RunDirectLoad,
            Self::RotateReceiptSegments => ActionKind::RotateReceiptSegments,
            Self::ReclaimExpiredEvidence => ActionKind::ReclaimExpiredEvidence,
            Self::InjectTaskStoreCapacityInvariantViolationOnce => {
                ActionKind::InjectTaskStoreCapacityInvariantViolationOnce
            }
            Self::InjectStoreFault { .. } => ActionKind::InjectStoreFault,
        }
    }

    fn missing_boundary(&self) -> Option<(ProductionBoundary, EvidenceCode, Option<EventKind>)> {
        use EvidenceCode as E;
        use ProductionBoundary as B;
        match self {
            Self::SendOuterEnvelope { .. } => Some((
                B::StrictEnvelopeValidation,
                E::StrictEnvelopeObservationUnavailable,
                None,
            )),
            Self::Submit { .. } | Self::SpawnSubmit { .. } => Some((
                B::V5ReceiptRuntime,
                E::ReceiptRowAbsent,
                Some(EventKind::V5ReceiptRuntimeEntered),
            )),
            Self::Recover { .. }
            | Self::Acknowledge { .. }
            | Self::Cancel { .. }
            | Self::SpawnCancel { .. }
            | Self::SeedReceipt { .. }
            | Self::InjectStoreFault { .. } => {
                Some((B::ReceiptTransition, E::ReceiptTransitionUnavailable, None))
            }
            Self::CompareClientServerIdentity | Self::InjectPersistedIdentityCollision { .. } => {
                Some((B::ReceiptIdentity, E::ReceiptIdentityUnavailable, None))
            }
            Self::ProbeProtocol { .. } => Some((
                B::ProtocolNegotiation,
                E::ProtocolBehaviorUnavailable,
                Some(EventKind::ProtocolFrameRead),
            )),
            Self::ReadTask { .. }
            | Self::CancelTask { .. }
            | Self::SeedTask { .. }
            | Self::SeedTaskLinkReservation { .. }
            | Self::SpawnTaskStoreCreateAndBindUnderGate { .. }
            | Self::ContinueReceiptOwnedAttempt { .. }
            | Self::SpawnStageBoundHandoffTerminal { .. }
            | Self::AttemptStagedTerminalAgainstProvisional { .. } => {
                Some((B::TaskProjection, E::TaskProjectionUnavailable, None))
            }
            Self::RunCrossStoreCrashWorkload { .. }
            | Self::RunTaskRetirementWorkload { .. }
            | Self::OpenTaskStoreInspectOnly
            | Self::ReconcileStartup => Some((
                B::CrossStoreReconciliation,
                E::CrossStoreIntentUnavailable,
                None,
            )),
            Self::InvalidateActorProof { .. }
            | Self::AttemptBoundTaskStart { .. }
            | Self::SpawnMarkReservedBegun { .. } => {
                Some((B::ActorLinearization, E::ActorFenceUnavailable, None))
            }
            Self::FillReceiptPool { .. }
            | Self::FillTaskLinks
            | Self::FillTaskLinksLeavingOneReservationSlot
            | Self::FillTombstones
            | Self::AttemptTaskStoreBindUnderGate { .. } => {
                Some((B::CapacityCoordination, E::CapacityLatchUnavailable, None))
            }
            Self::RotateReceiptSegments | Self::ReclaimExpiredEvidence => {
                Some((B::RetentionReclamation, E::RetentionIndexUnavailable, None))
            }
            Self::RunDirectLoad { .. } | Self::RunLazyCancelStorm { .. } => Some((
                B::V5Executor,
                E::WriterPathUnavailable,
                Some(EventKind::V5ExecutorEntered),
            )),
            Self::ConfigureValidation { .. }
            | Self::ConfigureProvider { .. }
            | Self::ConfigureAdmission { .. }
            | Self::ConfigurePrepare { .. }
            | Self::InstallBarrier { .. }
            | Self::ReleaseBarrier { .. }
            | Self::WaitForEvent { .. }
            | Self::WaitForEventCount { .. }
            | Self::WaitForOperation { .. }
            | Self::JoinOperation { .. }
            | Self::AdvanceMonotonic { .. }
            | Self::AdvanceEpoch { .. }
            | Self::Crash { .. }
            | Self::Restart
            | Self::Checkpoint { .. }
            | Self::Reset
            | Self::InjectTaskStoreCapacityInvariantViolationOnce
            | Self::PublishListener => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum ExecutionClass {
    Direct,
    KnownLong,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "terminal", rename_all = "snake_case")]
enum TerminalFixture {
    Success { payload: String },
    Bytes { count: u64 },
    NearLimitWithMaximumMetadata { canonical_result_bytes: u64 },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum BarrierPoint {
    ValidationEntered,
    AdmissionEntered,
    ActorBound,
    BeforePrepare,
    PrepareEntered,
    BeforeTaskStoreCreate,
    AfterWorkingReadback,
    AfterFalseCancelObservation,
    BeforeReceiptBegun,
    BeforeTaskTerminalReceipt,
    AfterCancelReservationConvertedBeforeTerminal,
    AfterTaskStoreReadbackBeforeTaskBound,
    BeforeMarkReservedBegunGateAcquire,
    BeforeCancelGateAcquire,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum DisconnectPoint {
    Never,
    AfterSubmitWrite,
    AfterTerminalCommit,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum AckDisconnectPoint {
    Never,
    AfterTombstoneCommit,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum RequestCase {
    Canonical,
    Fresh(u32),
    SameIdentity,
    Mismatch(IdentityField),
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
enum KeyCase {
    Exact,
    ForSubmitLabel(String),
    Unknown,
    Mismatch(IdentityField),
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum DigestCase {
    ExactTerminal,
    Mismatched,
    TaskTerminal,
    WellFormedCandidate,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum EnvelopeCase {
    MissingInvocationId,
    NoncanonicalInvocationId,
    MissingReservedTaskId,
    NoncanonicalReservedTaskId,
    UnknownTool,
    UnknownField,
    MalformedArguments,
    OversizedArguments,
    ResponseBudgetAboveMaximum,
    EmptyWorkspaceHint,
    WorkspaceHintWithControl,
    MalformedWorkspaceHint,
    OversizedWorkspaceHint,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum IdentityField {
    InvocationId,
    ReservedTaskId,
    CoreIdentity,
    ToolIdentity,
    NormalizedArgumentsHash,
    RequestScopeHash,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum IdentityIndex {
    InvocationId,
    ReservedTaskId,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ProtocolVersion {
    V3,
    V4,
    V5,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum ProtocolMessage {
    Ping,
    Release,
    SubmitWithCoreIdentity {
        selection: CoreIdentitySelection,
    },
    GetTask,
    WaitTask,
    CancelTask,
    RecoverReceipt,
    AcknowledgeReceipt,
    CancelReceipt,
    MaximumResponseFrame,
    OversizedResponseFrame,
    ErrorCodeFrame {
        code: V5DaemonErrorCodeFixture,
    },
    MalformedV5Schema {
        target: StrictSchemaTarget,
    },
    ReceiptPendingOutcome,
    TaskOutcome,
    AcknowledgedOutcome,
    DirectCompletedTerminal,
    DirectSemanticCompletedTerminal,
    DirectCancelledTerminal,
    DirectFailureTerminal {
        reason: FailureProbeReason,
    },
    TaskQueuedProjection,
    TaskWorkingProjection,
    TaskCompletedProjection,
    TaskSemanticCompletedProjection {
        owner: TaskTerminalOwnerFixture,
    },
    TaskCancelledProjection,
    TaskFailureProjection {
        reason: FailureProbeReason,
    },
    StoredInvocationRecord {
        schema_version: u8,
        reason: FailureProbeReason,
    },
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum TaskTerminalOwnerFixture {
    ReceiptBacked,
    Bound,
    Staged,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum CoreIdentitySelection {
    ExactProductionV5,
    ArbitraryCanonical,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum FailureProbeReason {
    InvocationFailed,
    ResultTooLarge,
    Interrupted,
    ResumeUnsupported,
    PersistenceFailed,
    OutcomeUncertain,
    TaskCapacity,
    WorkspaceCapacity,
    WorkspaceRegistryFailed,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum V5DaemonErrorCodeFixture {
    InvalidRequest,
    HandshakeRequired,
    ProtocolMismatch,
    CoreMismatch,
    Unauthorized,
    DuplicateLease,
    Overloaded,
    OwnerCapacity,
    ReceiptNotFound,
    ReceiptExpired,
    ReceiptCapacity,
    TombstoneCapacity,
    InvocationIdentityMismatch,
    TaskNotFound,
    TaskExpired,
    StoreFailed,
    DurabilityUncertain,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum StrictSchemaTarget {
    RequestUnknownField,
    RequestMissingRequiredField,
    RequestCrossVariantField,
    ResponseUnknownField,
    ResponseMissingRequiredField,
    ResponseCrossVariantField,
    TerminalUnknownField,
    TerminalMissingRequiredField,
    TerminalCrossVariantField,
    TaskSnapshotUnknownField,
    TaskSnapshotMissingRequiredField,
    TaskSnapshotCrossVariantField,
    StoredRecordUnknownTopLevel,
    StoredRecordUnknownTaskField,
    StoredRecordMissingRequiredField,
    StoredRecordCrossVariantField,
    TransferCertificateUnknownField,
    TransferCertificateMissingRequiredField,
    TransferCertificateCrossVariantField,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum TaskApi {
    NativeGet,
    NativeWait,
    CompatibilityGet,
    CompatibilityResult,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum TaskCancelApi {
    Native,
    Compatibility,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
enum TaskSelector {
    ExactProjected,
    ForReadLabel(String),
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum WorkspaceAdmissionFailure {
    Invalid,
    Capacity,
    RegistryFailed,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TaskPublicationCapacityObservation {
    operation_label: String,
    receipt_key: ReceiptKeyObservation,
    terminal: Option<TerminalObservation>,
    staged_transfer_certificate_sha256: Option<String>,
    evidence: TaskPublicationCapacityEvidence,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(tag = "source", rename_all = "snake_case", deny_unknown_fields)]
enum TaskPublicationCapacityEvidence {
    LinkCapacity {
        capacity_checked_sequence: u64,
        capacity_rejected_sequence: u64,
        task_store_generation_before: u64,
        task_store_generation_after: u64,
        task_store_create_attempts_before: u64,
        task_store_create_attempts_after: u64,
    },
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TaskStoreCapacityInvariantViolationObservation {
    operation_label: String,
    receipt_key: ReceiptKeyObservation,
    staged_terminal: TerminalObservation,
    staged_transfer_certificate_sha256: String,
    live_task_link_reservation_fingerprint: String,
    task_link_reserved_sequence: u64,
    task_store_create_sequence: u64,
    capacity_observed_sequence: u64,
    listener_closed_sequence: u64,
    restart_requested_sequence: u64,
    daemon_stopped_sequence: u64,
    task_store_record_count_before: u64,
    task_store_record_count_after: u64,
    materialized_lifecycle_link_count_before: u64,
    materialized_lifecycle_link_count_after: u64,
    live_link_reservation_count_before: u64,
    live_link_reservation_count_after: u64,
    task_store_generation_before: u64,
    task_store_generation_after: u64,
    task_store_create_attempts_before: u64,
    task_store_create_attempts_after: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum CrashPoint {
    ReservedUnbound,
    ReservedBegun,
    TaskPromisedUnbound,
    AfterSideEffectBeforeTerminal,
    BeforePromisedActorIntent,
    AfterPromisedActorIntent,
    BeforeBoundHandoffIntent,
    AfterBoundHandoffIntent,
    AfterBegunHandoffIntent,
    AfterStagedTerminal,
    AfterStagedTaskStoreTerminalReadbackBeforeLedgerCommit,
    BeforeTaskStoreCreate,
    AfterTaskStoreCreateBeforeTaskBound,
    AfterCancelFlagBeforeTaskCreate,
    AfterTaskStoreCancelReadbackBeforeTaskBound,
    AfterWorkingReadbackBeforeReceiptBegun,
    AfterReceiptBegunBeforePrepare,
    AfterTaskStoreTerminalBeforeLifecycleLinkTerminal,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum TaskRetirementWorkload {
    RecoveryTerminalBeforeTerminalBound,
    ActiveTaskBoundAbsent,
    BeforePendingIntent,
    AfterPendingIntentBeforeDelete,
    AfterDeleteCommitUncertain,
    AfterDeletedBeforeLedgerFinalize,
    AfterAbsentConfirmedBeforeLedgerFinalize,
    DeleteIdentityMismatch,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum EntryPath {
    PromisedUnbound,
    ReservedActorBound,
    ReservedBegun,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum SeedReceiptState {
    CancelReserved,
    ReservedUnbound,
    ReservedActorBound,
    ReservedBegun,
    DirectTerminalUnacked,
    AcknowledgedTombstone,
    TaskPromisedUnbound,
    TaskPromisedActorBound,
    TaskHandoffActorBoundNotBegun,
    TaskHandoffActorBoundBegun,
    TaskReceiptOwnedActorBound,
    TaskTerminalReceiptBacked,
    TaskBoundNotBegun,
    TaskBoundBegun,
    TaskTerminalBound,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum TaskStatus {
    Queued,
    Working,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum IdentityRelation {
    Exact,
    Foreign,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum ReceiptLinkCase {
    Exact,
    Missing,
    Foreign,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum ProvisionalMismatchField {
    TaskId,
    InvocationId,
    Status,
    Version,
    CancelRequested,
    TaskLinkDigest,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ActorProofCase {
    Missing,
    Foreign,
    Stale,
    Exact,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum StoreFaultPoint {
    AfterTerminalPayloadRenameBeforeDirectorySync,
    AfterTaskCreateRenameBeforeDirectorySync,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CrashWorkload {
    path: EntryPath,
    point: CrashPoint,
    cancel_before_crash: bool,
    stage_terminal_before_crash: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ScenarioReport {
    checkpoints: BTreeMap<String, Snapshot>,
    responses: BTreeMap<String, ResponseObservation>,
    task_reads: BTreeMap<String, TaskObservation>,
    events: Vec<EventRecord>,
    gate_events: Vec<GateEvent>,
    operation_events: Vec<OperationEvent>,
    actor_bindings: Vec<ActorBindingObservation>,
    actor_authorizations: Vec<ActorAuthorizationObservation>,
    task_publication_capacity: Vec<TaskPublicationCapacityObservation>,
    task_store_capacity_invariant_violations: Vec<TaskStoreCapacityInvariantViolationObservation>,
    staged_terminal_preparations: Vec<StagedTerminalPreparationObservation>,
    terminal_publications: Vec<TerminalPublicationObservation>,
    protocol: Vec<ProtocolObservation>,
    identity: Option<IdentityObservation>,
    crash_cases: Vec<CrashCaseObservation>,
    task_retirement_cases: Vec<TaskRetirementCaseObservation>,
    load_runs: BTreeMap<String, LoadObservation>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Snapshot {
    receipts: Vec<ReceiptObservation>,
    tombstones: Vec<TombstoneObservation>,
    tasks: Vec<TaskObservation>,
    task_links: Vec<TaskLinkObservation>,
    invocation_index: Vec<ReceiptKeyObservation>,
    reserved_task_index: Vec<ReceiptKeyObservation>,
    receipt_live_count: u64,
    receipt_actual_bytes: u64,
    receipt_reserved_bytes: u64,
    task_link_count: u64,
    task_link_bytes: u64,
    task_link_reserved_count: u64,
    task_link_reserved_bytes: u64,
    tombstone_count: u64,
    tombstone_bytes: u64,
    callbacks: CallbackCounts,
    listener: ListenerState,
    restart_requested: bool,
    daemon_running: bool,
    actor_leases: u64,
    side_effect_markers: u64,
    task_store_create_attempts: u64,
    token_signals: u64,
    store_generation: u64,
    epoch_ms: u64,
    process_exit_elapsed_ms: Option<u64>,
    cancel_authority: Option<CancelAuthority>,
    receipt_store_mutations: u64,
    task_store_mutations: u64,
    fallback_executions: u64,
    staged_responses_exposed: u64,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReceiptKeyObservation {
    invocation_id: String,
    reserved_task_id: String,
    core_identity_digest: String,
    tool: ToolIdentityObservation,
    normalized_arguments_hash: String,
    request_scope_hash: RequestScopeHash,
    key_digest: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct V5ReceiptKeyWireObservation {
    invocation_id: String,
    reserved_task_id: String,
    core_identity_digest: String,
    tool: ToolIdentityObservation,
    normalized_arguments_hash: String,
    request_scope_hash: RequestScopeHash,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
struct RequestScopeHash(String);

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
enum ToolIdentityObservation {
    #[serde(rename = "unica.view")]
    View,
    #[serde(rename = "unica.apply")]
    Apply,
    #[serde(rename = "unica.find")]
    Find,
    #[serde(rename = "unica.search")]
    Search,
    #[serde(rename = "unica.check")]
    Check,
    #[serde(rename = "unica.diff")]
    Diff,
    #[serde(rename = "unica.run")]
    Run,
    #[serde(rename = "unica.docs")]
    Docs,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DomainResultObservation {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    at: Option<String>,
    summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    changed: Vec<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    diagnostics: Vec<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    artifacts: Vec<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    next: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rev: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cursor: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[allow(clippy::large_enum_variant)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
enum TerminalObservation {
    Completed {
        result: DomainResultObservation,
        canonical_payload_hex: String,
        terminal_digest: String,
        terminal_epoch_ms: u64,
    },
    Failed {
        reason: V5SafeFailureReason,
        canonical_payload_hex: String,
        terminal_digest: String,
        terminal_epoch_ms: u64,
    },
    Cancelled {
        canonical_payload_hex: String,
        terminal_digest: String,
        terminal_epoch_ms: u64,
    },
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TerminalPublicationObservation {
    receipt_key: ReceiptKeyObservation,
    terminal: TerminalObservation,
    commit: TerminalCommitPreflightObservation,
    response_frames: Vec<ResponseFramePreflightObservation>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StagedTerminalPreparationObservation {
    receipt_key: ReceiptKeyObservation,
    terminal: TerminalObservation,
    terminal_payload: ArtifactEvidence,
    staged_receipt_record: ArtifactEvidence,
    candidate_result: Option<ArtifactEvidence>,
    workspace_identity_hash: String,
    task_link_digest: String,
    receipt_expected_version: u64,
    committed_receipt_version: u64,
    terminal_payload_prepared_sequence: u64,
    staged_receipt_prepared_sequence: u64,
    stage_commit_sequence: u64,
    stage_readback_sequence: u64,
    transfer_size_certificate: StagedTerminalTransferSizeCertificateObservation,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StagedTerminalTransferSizeCertificateObservation {
    certificate: ArtifactEvidence,
    issued_sequence: u64,
    terminal_bound_link_record: ArtifactEvidence,
    cases: Vec<StagedTerminalTransferSizeCaseObservation>,
    capacity_fallback_cases: Vec<StagedCapacityFallbackSizeCaseObservation>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
enum StagedTerminalTransferSizeCaseObservation {
    Absent {
        final_task_record: ArtifactEvidence,
        task_response_jsonl: ArtifactEvidence,
    },
    ExactProvisional {
        provisional_status: TaskStatus,
        cancel_requested: bool,
        task_version: u64,
        final_task_record: ArtifactEvidence,
        task_response_jsonl: ArtifactEvidence,
    },
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StagedTerminalTransferSizeCertificate {
    certificate_version: u8,
    protocol_identity: String,
    core_identity_digest: String,
    receipt_key_digest: String,
    task_id: String,
    invocation_id: String,
    task_link_digest: String,
    terminal_digest: String,
    terminal_epoch_ms: u64,
    receipt_record_schema_version: u8,
    task_record_schema_version: u8,
    lifecycle_link_record_schema_version: u8,
    terminal_codec_version: u8,
    max_daemon_response_line_bytes: u64,
    max_task_lifecycle_link_record_bytes: u64,
    staged_receipt_record_max_bytes: u64,
    task_terminal_bound_link_record_max_bytes: u64,
    task_publication_cases: Vec<StagedTerminalTransferSizeCaseCertificate>,
    capacity_fallback_cases: Vec<StagedCapacityFallbackSizeCaseCertificate>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum StagedTerminalTransferSizeCaseCertificate {
    Absent {
        final_task_record_max_bytes: u64,
        task_response_frame_max_bytes: u64,
    },
    ExactProvisional {
        status: TaskStatus,
        cancel_requested: bool,
        version: u64,
        final_task_record_max_bytes: u64,
        task_response_frame_max_bytes: u64,
    },
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(tag = "source", rename_all = "snake_case", deny_unknown_fields)]
enum StagedCapacityFallbackSizeCaseCertificate {
    LinkCapacity {
        receipt_backed_record_max_bytes: u64,
        task_response_frame_max_bytes: u64,
    },
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(tag = "source", rename_all = "snake_case", deny_unknown_fields)]
enum StagedCapacityFallbackSizeCaseObservation {
    LinkCapacity {
        receipt_backed_record: ArtifactEvidence,
        task_response_jsonl: ArtifactEvidence,
    },
}

impl StagedTerminalTransferSizeCaseObservation {
    fn shape(&self) -> Option<(TaskStatus, bool, u64)> {
        match self {
            Self::Absent { .. } => None,
            Self::ExactProvisional {
                provisional_status,
                cancel_requested,
                task_version,
                ..
            } => Some((*provisional_status, *cancel_requested, *task_version)),
        }
    }

    fn artifacts(&self) -> (&ArtifactEvidence, &ArtifactEvidence) {
        match self {
            Self::Absent {
                final_task_record,
                task_response_jsonl,
            }
            | Self::ExactProvisional {
                final_task_record,
                task_response_jsonl,
                ..
            } => (final_task_record, task_response_jsonl),
        }
    }
}

impl StagedTerminalTransferSizeCaseCertificate {
    fn shape(&self) -> Option<(TaskStatus, bool, u64)> {
        match self {
            Self::Absent { .. } => None,
            Self::ExactProvisional {
                status,
                cancel_requested,
                version,
                ..
            } => Some((*status, *cancel_requested, *version)),
        }
    }

    fn bounds(&self) -> (u64, u64) {
        match self {
            Self::Absent {
                final_task_record_max_bytes,
                task_response_frame_max_bytes,
            }
            | Self::ExactProvisional {
                final_task_record_max_bytes,
                task_response_frame_max_bytes,
                ..
            } => (*final_task_record_max_bytes, *task_response_frame_max_bytes),
        }
    }
}

impl StagedCapacityFallbackSizeCaseCertificate {
    fn bounds(&self) -> (u64, u64) {
        match self {
            Self::LinkCapacity {
                receipt_backed_record_max_bytes,
                task_response_frame_max_bytes,
            } => (
                *receipt_backed_record_max_bytes,
                *task_response_frame_max_bytes,
            ),
        }
    }
}

impl StagedCapacityFallbackSizeCaseObservation {
    fn artifacts(&self) -> (&ArtifactEvidence, &ArtifactEvidence) {
        match self {
            Self::LinkCapacity {
                receipt_backed_record,
                task_response_jsonl,
            } => (receipt_backed_record, task_response_jsonl),
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum TerminalPublicationOwner {
    DirectReceiptLedger,
    ReceiptBackedTask,
    BoundTaskStore,
    StagedHandoffTask,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[allow(clippy::large_enum_variant)]
#[serde(tag = "owner", rename_all = "snake_case", deny_unknown_fields)]
enum TerminalCommitPreflightObservation {
    DirectReceiptLedger {
        receipt: ReceiptTerminalCommitPieces,
    },
    ReceiptBackedTask {
        receipt: ReceiptTerminalCommitPieces,
    },
    BoundTaskStore {
        task: BoundTaskTerminalCommitPieces,
    },
    StagedHandoffTask {
        task: StagedTaskTerminalCommitPieces,
    },
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReceiptTerminalCommitPieces {
    terminal_payload: ArtifactEvidence,
    receipt_record: ArtifactEvidence,
    candidate_result: Option<ArtifactEvidence>,
    terminal_payload_prepared_sequence: u64,
    receipt_record_prepared_sequence: u64,
    receipt_commit_sequence: u64,
    receipt_expected_version: u64,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BoundTaskTerminalCommitPieces {
    terminal_payload: ArtifactEvidence,
    candidate_result: Option<ArtifactEvidence>,
    terminal_payload_prepared_sequence: u64,
    task_record: ArtifactEvidence,
    task_record_prepared_sequence: u64,
    task_store_commit_sequence: u64,
    task_store_readback_sequence: u64,
    task_expected_version: u64,
    lifecycle_link_record: ArtifactEvidence,
    lifecycle_link_record_prepared_sequence: u64,
    lifecycle_link_commit_sequence: u64,
    committed_lifecycle_link_version: u64,
    lifecycle_link_expected_version: u64,
    task_link_digest: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StagedTaskTerminalCommitPieces {
    terminal_payload: ArtifactEvidence,
    candidate_result: Option<ArtifactEvidence>,
    terminal_payload_prepared_sequence: u64,
    task_record: ArtifactEvidence,
    task_record_prepared_sequence: u64,
    task_store_commit_sequence: u64,
    task_store_readback_sequence: u64,
    terminal_write_expectation: StagedTaskTerminalWriteExpectation,
    terminal_write_branch: StagedTaskTerminalWriteBranch,
    idempotent_repeat: Option<StagedTaskTerminalIdempotentReadback>,
    committed_task_version: u64,
    lifecycle_link_record: ArtifactEvidence,
    lifecycle_link_record_prepared_sequence: u64,
    lifecycle_link_commit_sequence: u64,
    committed_lifecycle_link_version: u64,
    live_task_link_reservation_fingerprint: String,
    task_link_digest: String,
    staged_receipt_version: u64,
    staged_receipt_record_sha256: String,
    staged_terminal_digest: String,
    transfer_size_certificate_sha256: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
enum StagedTaskTerminalWriteExpectation {
    Absent {
        task_store_generation: u64,
    },
    ExactProvisional {
        task_id: String,
        invocation_id: String,
        expected_version: u64,
        status: TaskStatus,
        cancel_requested: bool,
        task_identity_digest: String,
        task_link_digest: String,
        provisional_task_store_readback: ArtifactEvidence,
    },
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StagedTaskTerminalIdempotentReadback {
    task_record: ArtifactEvidence,
    readback_sequence: u64,
    task_store_generation_before: u64,
    task_store_generation_after: u64,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum StagedTaskTerminalWriteBranch {
    CreatedTerminal,
    ReplacedExactProvisional,
    ExactTerminalReadback,
}

impl TerminalCommitPreflightObservation {
    fn owner(&self) -> TerminalPublicationOwner {
        match self {
            Self::DirectReceiptLedger { .. } => TerminalPublicationOwner::DirectReceiptLedger,
            Self::ReceiptBackedTask { .. } => TerminalPublicationOwner::ReceiptBackedTask,
            Self::BoundTaskStore { .. } => TerminalPublicationOwner::BoundTaskStore,
            Self::StagedHandoffTask { .. } => TerminalPublicationOwner::StagedHandoffTask,
        }
    }

    fn receipt(&self) -> Option<&ReceiptTerminalCommitPieces> {
        match self {
            Self::DirectReceiptLedger { receipt } | Self::ReceiptBackedTask { receipt } => {
                Some(receipt)
            }
            Self::BoundTaskStore { .. } | Self::StagedHandoffTask { .. } => None,
        }
    }

    fn terminal_payload(&self) -> &ArtifactEvidence {
        match self {
            Self::DirectReceiptLedger { receipt } | Self::ReceiptBackedTask { receipt } => {
                &receipt.terminal_payload
            }
            Self::BoundTaskStore { task } => &task.terminal_payload,
            Self::StagedHandoffTask { task } => &task.terminal_payload,
        }
    }

    fn candidate_result(&self) -> Option<&ArtifactEvidence> {
        match self {
            Self::DirectReceiptLedger { receipt } | Self::ReceiptBackedTask { receipt } => {
                receipt.candidate_result.as_ref()
            }
            Self::BoundTaskStore { task } => task.candidate_result.as_ref(),
            Self::StagedHandoffTask { task } => task.candidate_result.as_ref(),
        }
    }

    fn terminal_payload_prepared_sequence(&self) -> u64 {
        match self {
            Self::DirectReceiptLedger { receipt } | Self::ReceiptBackedTask { receipt } => {
                receipt.terminal_payload_prepared_sequence
            }
            Self::BoundTaskStore { task } => task.terminal_payload_prepared_sequence,
            Self::StagedHandoffTask { task } => task.terminal_payload_prepared_sequence,
        }
    }

    fn durable_commit_sequence(&self) -> u64 {
        match self {
            Self::DirectReceiptLedger { receipt } | Self::ReceiptBackedTask { receipt } => {
                receipt.receipt_commit_sequence
            }
            Self::BoundTaskStore { task } => task.lifecycle_link_commit_sequence,
            Self::StagedHandoffTask { task } => task.lifecycle_link_commit_sequence,
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ResponseFramePreflightObservation {
    response_kind: ResponseKind,
    origin: ResponseFrameOrigin,
    response_jsonl: ArtifactEvidence,
    prepared_sequence: u64,
    write_sequence: Option<u64>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ResponseFrameOrigin {
    ImmediatePublication,
    ExactDuplicate,
    Recovery,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ArtifactEvidence {
    raw_hex: String,
    encoded_bytes: u64,
    sha256: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum V5SafeFailureReason {
    InvocationFailed,
    ResultTooLarge,
    Interrupted,
    OutcomeUncertain,
    TaskCapacity,
    PersistenceFailed,
    ResumeUnsupported,
    WorkspaceCapacity,
    WorkspaceRegistryFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TerminalClass {
    Absent,
    Completed(bool),
    Failed(V5SafeFailureReason),
    Cancelled,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AcknowledgementObservation {
    terminal_digest: String,
    ack_epoch_ms: u64,
    expires_epoch_ms: u64,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TombstoneObservation {
    key: ReceiptKeyObservation,
    terminal_digest: String,
    ack_epoch_ms: u64,
    expires_epoch_ms: u64,
    encoded_bytes: u64,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TaskLinkObservation {
    key: ReceiptKeyObservation,
    task_id: String,
    invocation_id: String,
    workspace_identity_hash: String,
    link_digest: String,
    encoded_bytes: u64,
    version: u64,
    lifecycle: TaskLinkLifecycleObservation,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[allow(clippy::enum_variant_names, clippy::large_enum_variant)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
enum TaskLinkLifecycleObservation {
    TaskBoundNotBegun {
        cancel_requested: bool,
        task_version: u64,
    },
    TaskBoundBegun {
        cancel_requested: bool,
        task_version: u64,
    },
    TaskTerminalBound {
        terminal_digest: String,
        terminal_epoch_ms: u64,
        ttl_ms: u64,
        expires_at_epoch_ms: u64,
        task_version: u64,
    },
    TaskRetirementPending {
        pending: TaskRetirementPendingObservation,
    },
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReceiptObservation {
    key: ReceiptKeyObservation,
    state: SeedReceiptState,
    cancel_requested: bool,
    accepted_epoch_ms: u64,
    original_budget_ms: u64,
    expires_epoch_ms: Option<u64>,
    bound_workspace_identity: Option<String>,
    staged_terminal: Option<TerminalObservation>,
    terminal: Option<TerminalObservation>,
    encoded_bytes: u64,
    reserved_result_bytes: u64,
    version: u64,
    mutation_sequence: u64,
    begun: bool,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TaskObservation {
    task_id: String,
    invocation_id: String,
    receipt_key: ReceiptKeyObservation,
    status: TaskStatus,
    projection_source: ProjectionSource,
    workspace_identity_hash: Option<String>,
    created_epoch_ms: u64,
    updated_epoch_ms: u64,
    expires_epoch_ms: u64,
    ttl_ms: u64,
    poll_interval_ms: u64,
    version: u64,
    encoded_bytes: u64,
    cancel_requested: bool,
    terminal: Option<TerminalObservation>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct V5StoredInvocationRecordObservation {
    schema_version: u8,
    task_id: String,
    invocation_id: String,
    receipt_key_digest: String,
    tool: ToolIdentityObservation,
    normalized_arguments_hash: String,
    workspace_identity_hash: String,
    created_at_epoch_ms: u64,
    updated_at_epoch_ms: u64,
    ttl_ms: u64,
    poll_interval_ms: u64,
    version: u64,
    cancel_requested: bool,
    task: V5StoredTaskObservation,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[allow(clippy::large_enum_variant)]
#[serde(
    tag = "status",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum V5StoredTaskObservation {
    Queued,
    Working,
    Completed {
        terminal_epoch_ms: u64,
        terminal_digest: String,
        result: DomainResultObservation,
    },
    Failed {
        terminal_epoch_ms: u64,
        terminal_digest: String,
        reason: V5SafeFailureReason,
    },
    Cancelled {
        terminal_epoch_ms: u64,
        terminal_digest: String,
    },
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ProjectionSource {
    ReceiptLedger,
    TaskStore,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ListenerState {
    NotPublished,
    Listening,
    Closed,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum CancelAuthority {
    ReceiptLedger,
    TaskStore,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CallbackCounts {
    validation: u64,
    admission: u64,
    prepare: u64,
    execute: u64,
}

impl CallbackCounts {
    fn total_domain(&self) -> u64 {
        self.validation + self.admission + self.prepare + self.execute
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ResponseObservation {
    kind: ResponseKind,
    error: Option<ErrorCode>,
    terminal: Option<TerminalObservation>,
    key: Option<ReceiptKeyObservation>,
    task: Option<TaskObservation>,
    acknowledgement: Option<AcknowledgementObservation>,
    cutoff_epoch_ms: Option<u64>,
    original_budget_ms: Option<u64>,
    latency_ms: u64,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ResponseKind {
    Direct,
    Task,
    RecoveredDirect,
    Acknowledged,
    Cancelled,
    Rejected,
    Tombstone,
    NotFound,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ErrorCode {
    InvalidRequest,
    HandshakeRequired,
    ProtocolMismatch,
    CoreMismatch,
    Unauthorized,
    DuplicateLease,
    Overloaded,
    OwnerCapacity,
    ReceiptNotFound,
    ReceiptExpired,
    ReceiptCapacity,
    TombstoneCapacity,
    InvocationIdentityMismatch,
    TaskNotFound,
    TaskExpired,
    StoreFailed,
    DurabilityUncertain,
    OutcomeUncertain,
    TaskCapacity,
    ResultTooLarge,
    StoreCommitUncertain,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EventRecord {
    sequence: u64,
    monotonic_ms: u64,
    epoch_ms: u64,
    event: EventKind,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GateEvent {
    sequence: u64,
    operation_label: String,
    transition: GateTransition,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum GateTransition {
    Waiting,
    Acquired,
    Released,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OperationEvent {
    sequence: u64,
    label: String,
    state: OperationEventState,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ActorAuthorizationObservation {
    operation_label: String,
    purpose: ActorAuthorizationPurpose,
    verifier: ActorVerifier,
    ledger_authorization: LedgerAuthorizationObservation,
    presented_authorization: Option<PresentedAuthorizationObservation>,
    task_bound_context: Option<TaskBoundContextObservation>,
    post_working_authorization: Option<PostWorkingActorAuthorizationObservation>,
    verifier_generation: u64,
    decision: ActorAuthorizationDecision,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ActorAuthorizationPurpose {
    ReservedBegin,
    BoundTaskStart,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ActorBindingObservation {
    operation_label: String,
    receipt_key: ReceiptKeyObservation,
    actor_identity_hash: String,
    actor_generation: u64,
    binding_claim_fingerprint: String,
    binding_token_fingerprint: String,
    claim_verified_sequence: u64,
    binding_token_minted_sequence: u64,
    actor_bound_expected_receipt_version: u64,
    actor_bound_committed_receipt_version: u64,
    actor_bound_sequence: u64,
    binding_token_consumption: Option<BindingTokenConsumptionObservation>,
    task_binding: Option<TaskBindingTransferObservation>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(tag = "consumer", rename_all = "snake_case", deny_unknown_fields)]
enum BindingTokenConsumptionObservation {
    MarkReservedBegun {
        consumed_sequence: u64,
        receipt_expected_version: u64,
        receipt_committed_version: u64,
    },
    AuthorizeBoundTaskStart {
        consumed_sequence: u64,
        lifecycle_link_expected_version: u64,
        lifecycle_link_committed_version: u64,
    },
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TaskBindingTransferObservation {
    task_link_reservation_fingerprint: String,
    task_link_digest: String,
    task_link_reserved_sequence: u64,
    task_store_create_sequence: u64,
    task_link_reservation_consumed_sequence: u64,
    task_bound_sequence: u64,
    task_bound_committed_lifecycle_link_version: u64,
    task_bound_link_authorization_fingerprint: String,
    task_bound_link_authorization_minted_sequence: u64,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TaskBoundContextObservation {
    receipt_key: ReceiptKeyObservation,
    task_id: String,
    task_link_digest: String,
    task_version: u64,
    lifecycle_link_version: u64,
    actor_generation: u64,
    consumed_binding_token_fingerprint: String,
    consumed_task_link_reservation_fingerprint: String,
    task_bound_link_authorization_fingerprint: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PostWorkingActorAuthorizationObservation {
    authorization_fingerprint: String,
    receipt_key: ReceiptKeyObservation,
    task_id: String,
    task_link_digest: String,
    expected_task_version: u64,
    actor_generation: u64,
    task_bound_link_authorization_fingerprint: String,
    task_bound_link_authorization_consumed_sequence: u64,
    minted_sequence: u64,
    working_write_sequence: u64,
    working_readback_sequence: u64,
    working_readback_task_link_digest: String,
    rechecked_sequence: u64,
    consumed_sequence: Option<u64>,
    mark_begun_expected_lifecycle_link_version: Option<u64>,
    mark_begun_committed_lifecycle_link_version: Option<u64>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ActorVerifier {
    InfrastructureLeaseRegistry,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LedgerAuthorizationObservation {
    issuer: ActorAuthorizationIssuer,
    receipt_key: ReceiptKeyObservation,
    authorization_fingerprint: String,
    generation: u64,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ActorAuthorizationIssuer {
    ReceiptLedger,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PresentedAuthorizationObservation {
    authorization_fingerprint: String,
    generation: u64,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ActorAuthorizationDecision {
    Accepted,
    Missing,
    Foreign,
    Stale,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum OperationEventState {
    Spawned,
    Blocked,
    Completed,
    Joined,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum EventKind {
    V5ReceiptRuntimeEntered,
    V5ExecutorEntered,
    CanonicalV13ServiceEntered,
    StrictEnvelopeParsed,
    ReceiptReserved,
    CancelReservationConverted,
    ValidationEntered,
    AdmissionEntered,
    ActorBoundCommitted,
    ReceiptBegunCommitted,
    PrepareEntered,
    ExecuteEntered,
    UnboundPromiseCommitted,
    BoundHandoffCommitted,
    TaskStoreCreated,
    TaskLinkCapacityReserved,
    TaskLinkCapacityRejected,
    TaskStoreCreateAttempted,
    TaskLinkReservationConverted,
    TaskLinkReservationReleased,
    TaskStoreWorkingReadback,
    FalseCancelObservationReached,
    TaskBoundCommitted,
    TaskStoreCapacityInvariantViolation,
    BoundHandoffTerminalStaged,
    TaskStoreTerminalCommitted,
    TaskStoreTerminalReadback,
    TaskTerminalBoundCommitted,
    ReceiptTerminalCommitted,
    ResultSerialized,
    FinalResultProjected,
    AcknowledgementCommitted,
    ListenerPublished,
    ListenerClosed,
    TokenSignalled,
    LeaseReleased,
    MarkReservedBegunBlocked,
    CancelCommitBlocked,
    ProtocolFrameRead,
    ProtocolFrameWritten,
    TaskStoreReadbackBeforeBind,
    CancelCommitted,
    OperationCompleted,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProtocolObservation {
    label: String,
    client: ProtocolVersion,
    server: ProtocolVersion,
    client_hello_frame_hex: String,
    server_ready_frame_hex: Option<String>,
    client_write_frame_hex: String,
    server_read_frame_hex: Option<String>,
    server_write_frame_hex: String,
    client_read_frame_hex: String,
    spawned_argv_hex: String,
    daemon_process_events: Vec<DaemonProcessEvent>,
    production_events: Vec<ProtocolProbeEvent>,
    error: Option<ErrorCode>,
    protocol_identity: String,
    state_selector: ProtocolStateSelector,
    state_fingerprint: String,
    presented_core_identity_digest: Option<String>,
    production_v5_core_identity_digest: String,
    service_capability_fingerprint: Option<String>,
    delivery: Option<ProjectionDeliveryObservation>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProjectionDeliveryObservation {
    pending_direct_receipt_hex: Option<String>,
    direct_receipt_key: Option<ReceiptKeyObservation>,
    direct_terminal: Option<TerminalObservation>,
    internal_task_snapshot: Option<TaskObservation>,
    stored_invocation_record: Option<ArtifactEvidence>,
    native_mcp_projection_hex: Option<String>,
    compatibility_get_projection_hex: Option<String>,
    compatibility_result_projection_hex: Option<String>,
    final_call_tool_result_hex: Option<String>,
    final_error_data_hex: Option<String>,
    task_terminal_publication: Option<TerminalPublicationObservation>,
    events: Vec<DeliveryEvent>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum DeliveryEvent {
    TerminalPreflighted,
    PendingDirectReceiptBuilt,
    NativeProjectionBuilt,
    CompatibilityGetProjectionBuilt,
    CompatibilityResultProjectionBuilt,
    FinalInterfaceValueBuilt,
    AcknowledgementWritten,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ProtocolProbeEvent {
    ClientFrameWritten,
    ServerFrameRead,
    ServerFrameWritten,
    ClientFrameRead,
    NegotiationRejected,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum DaemonProcessEvent {
    Spawned,
    InterfacesDaemonEntrypointEntered,
    DefaultV3CompositionSelected,
    VersionedV5DispatchSelected,
    V3HandshakeCompleted,
    V5HandshakeCompleted,
    ProtocolFrameHandled,
    CanonicalV13ServiceEntered,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ProtocolStateSelector {
    ProtocolV3,
    ReceiptV5,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct IdentityObservation {
    client_key: ReceiptKeyObservation,
    daemon_key: ReceiptKeyObservation,
    frozen_vector_key: ReceiptKeyObservation,
    frozen_task_link_vector: TaskLinkDigestVectorObservation,
    caller_claimed_key_digest: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TaskLinkDigestVectorObservation {
    receipt_key_digest: String,
    task_id: String,
    invocation_id: String,
    workspace_identity_hash: String,
    task_link_digest: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CrashCaseObservation {
    path: EntryPath,
    point: CrashPoint,
    ledger: CrashLedgerObservation,
    projections: Vec<TaskObservation>,
    task_store_records: Vec<TaskObservation>,
    callback_invocation_ids: Vec<String>,
    staged_terminal_before_crash: Option<TerminalObservation>,
    recovered_terminal: TerminalObservation,
    receipt_store_generation: u64,
    task_store_generation: u64,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[allow(clippy::large_enum_variant)]
#[serde(tag = "owner", rename_all = "snake_case", deny_unknown_fields)]
enum CrashLedgerObservation {
    ActiveReceipt { receipt: ReceiptObservation },
    LifecycleLink { link: TaskLinkObservation },
}

impl CrashLedgerObservation {
    fn key(&self) -> &ReceiptKeyObservation {
        match self {
            Self::ActiveReceipt { receipt } => &receipt.key,
            Self::LifecycleLink { link } => &link.key,
        }
    }

    fn active_receipt(&self) -> &ReceiptObservation {
        match self {
            Self::ActiveReceipt { receipt } => receipt,
            Self::LifecycleLink { .. } => panic!("expected active receipt crash owner"),
        }
    }

    fn lifecycle_link(&self) -> &TaskLinkObservation {
        match self {
            Self::LifecycleLink { link } => link,
            Self::ActiveReceipt { .. } => panic!("expected sole lifecycle-link crash owner"),
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TaskRetirementPendingObservation {
    receipt_key: ReceiptKeyObservation,
    task_id: String,
    task_link_digest: String,
    terminal_digest: String,
    terminal_epoch_ms: u64,
    ttl_ms: u64,
    expires_at_epoch_ms: u64,
    expected_task_version: u64,
    resolver: TaskRetirementResolver,
    version: u64,
    lifecycle_link_expected_version: u64,
    committed_lifecycle_link_version: u64,
    committed_pending_record: ArtifactEvidence,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TaskRetirementPendingRecordObservation {
    receipt_key: ReceiptKeyObservation,
    task_id: String,
    task_link_digest: String,
    terminal_digest: String,
    terminal_epoch_ms: u64,
    ttl_ms: u64,
    expires_at_epoch_ms: u64,
    expected_task_version: u64,
    resolver: TaskRetirementResolver,
    version: u64,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum TaskRetirementResolver {
    TaskExpired,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TaskRetirementCaseObservation {
    case: TaskRetirementWorkload,
    before: Snapshot,
    after_crash: Snapshot,
    after_recovery: Snapshot,
    committed_pending: Option<TaskRetirementPendingObservation>,
    initial_authorization: Option<TaskRetirementAuthorizationObservation>,
    recovered_authorization: Option<TaskRetirementAuthorizationObservation>,
    old_authorization_reuse: Option<TaskRetirementAuthorizationReuseObservation>,
    delete_outcome: TaskRetirementDeleteOutcome,
    retirement_events: Vec<TaskRetirementEventRecord>,
    task_store_delete_attempts: u64,
    lazy_task_delete_attempts: u64,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
enum TaskRetirementDeleteOutcome {
    Deleted {
        deleted_task_record: ArtifactEvidence,
        pending_authorization_fingerprint: String,
        binding: TaskRetirementDeleteBindingObservation,
    },
    AbsentExactWithPending {
        pending_authorization_fingerprint: String,
        binding: TaskRetirementDeleteBindingObservation,
    },
    CommitUncertain {
        task_store_generation_before: u64,
        task_store_generation_after: u64,
    },
    IdentityMismatch {
        observed_task_record: ArtifactEvidence,
        observed_binding: TaskRetirementDeleteBindingObservation,
    },
    NotAttemptedActiveTaskMissing {
        receipt_store_generation_before: u64,
        receipt_store_generation_after: u64,
        task_store_generation_before: u64,
        task_store_generation_after: u64,
    },
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TaskRetirementDeleteBindingObservation {
    task_id: String,
    invocation_id: String,
    receipt_key_digest: String,
    task_link_digest: String,
    terminal_digest: String,
    terminal_epoch_ms: u64,
    ttl_ms: u64,
    expires_at_epoch_ms: u64,
    task_version: u64,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TaskRetirementAuthorizationObservation {
    authorization_fingerprint: String,
    process_instance_id: String,
    generation: u64,
    issued_sequence: u64,
    pending_record_sha256: String,
    pending_version: u64,
    receipt_key_digest: String,
    task_id: String,
    task_link_digest: String,
    terminal_digest: String,
    terminal_epoch_ms: u64,
    expires_at_epoch_ms: u64,
    expected_task_version: u64,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TaskRetirementAuthorizationReuseObservation {
    presented_fingerprint: String,
    rejected_sequence: u64,
    receipt_store_mutations_before: u64,
    receipt_store_mutations_after: u64,
    task_store_mutations_before: u64,
    task_store_mutations_after: u64,
    reason: TaskRetirementAuthorizationRejection,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum TaskRetirementAuthorizationRejection {
    StaleProcessCapability,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TaskRetirementEventRecord {
    sequence: u64,
    event: TaskRetirementEvent,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum TaskRetirementEvent {
    ExactTerminalReadback,
    TerminalBoundCommitted,
    PendingCommitted,
    ExistingPendingAuthorized,
    DeleteAttempted,
    DeleteCommitted,
    AbsentWithPendingConfirmed,
    DeleteCommitUncertain,
    DeleteIdentityMismatch,
    PendingFinalized,
    ActiveTaskBoundTaskMissingCorruption,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LoadObservation {
    window_started_monotonic_ms: u64,
    window_ended_monotonic_ms: u64,
    drain_completed_monotonic_ms: u64,
    listener: ListenerState,
    lifecycles: Vec<LifecycleObservation>,
    concurrency_samples: Vec<ConcurrencySample>,
    capacity_rejections: Vec<ErrorCode>,
    store_errors: Vec<ErrorCode>,
    task_store_create_attempts: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LifecycleObservation {
    key: ReceiptKeyObservation,
    accepted_epoch_ms: u64,
    started_monotonic_ms: u64,
    completed_monotonic_ms: u64,
    response_latency_ms: u64,
    terminal: TerminalObservation,
    acknowledgement: Option<AcknowledgementObservation>,
    callback_invocation_id: Option<String>,
    terminal_store_generation: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ConcurrencySample {
    monotonic_ms: u64,
    live_receipts: u64,
    owner_slots: u64,
    handshakes: u64,
    accept_batch: u64,
}

#[derive(Debug, Deserialize)]
#[allow(clippy::large_enum_variant)]
#[serde(
    tag = "kind",
    content = "payload",
    rename_all = "snake_case",
    deny_unknown_fields
)]
enum FacadeEnvelope {
    Observed(ScenarioReport),
    ProductionMissingTransition(ProductionMissingTransition),
    HarnessFailure(HarnessFailure),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProductionMissingTransition {
    action_index: u32,
    action_kind: ActionKind,
    reached_boundary: ProductionBoundary,
    current_protocol: ProtocolVersion,
    evidence: MissingEvidence,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MissingEvidence {
    code: EvidenceCode,
    event: Option<EventKind>,
    generation_before: u64,
    generation_after: u64,
    fingerprint: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HarnessFailure {
    stage: HarnessStage,
    code: HarnessCode,
    action_index: Option<u32>,
    evidence_fingerprint: Option<String>,
    numeric_detail: Option<u64>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum HarnessStage {
    ScenarioDecode,
    FixtureSetup,
    BarrierTimeout,
    ProcessControl,
    EvidenceCapture,
    EvidenceEncode,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum HarnessCode {
    InvalidScenario,
    UnsupportedFixture,
    BarrierNotReached,
    ChildProcessFailed,
    EvidenceMissing,
    EvidenceMalformed,
    EvidenceTooLarge,
}

fn execute(scenario: Scenario) -> ScenarioReport {
    let requires_v5_receipt_runtime = scenario.actions.iter().any(|action| {
        matches!(
            action,
            Action::Submit { .. }
                | Action::SpawnSubmit { .. }
                | Action::Recover { .. }
                | Action::Acknowledge { .. }
                | Action::Cancel { .. }
                | Action::SpawnCancel { .. }
        )
    });
    let requires_v5_executor = scenario.actions.iter().any(|action| {
        matches!(
            action,
            Action::RunDirectLoad { .. } | Action::RunLazyCancelStorm { .. }
        )
    });
    let request = serde_json::to_string(&scenario)
        .unwrap_or_else(|_| panic!("HARNESS FAILURE: scenario_encode"));
    let response = execute_scenario_json(&request)
        .unwrap_or_else(|error| panic!("HARNESS FAILURE: bridge_transport: {error}"));
    assert!(
        response.len() <= MAX_SCENARIO_REPORT_BYTES,
        "HARNESS FAILURE: facade_envelope_too_large bytes={}",
        response.len()
    );
    let envelope: FacadeEnvelope = serde_json::from_str(&response)
        .unwrap_or_else(|_| panic!("HARNESS FAILURE: malformed_facade_envelope"));
    match envelope {
        FacadeEnvelope::Observed(report) => {
            assert_report_raw_bounds(&report);
            if requires_v5_receipt_runtime {
                assert!(
                    count_event(&report, EventKind::V5ReceiptRuntimeEntered) > 0,
                    "observed run must cross the dedicated v5 receipt runtime"
                );
            }
            if requires_v5_executor {
                assert!(
                    count_event(&report, EventKind::V5ExecutorEntered) > 0,
                    "load run must cross the dedicated v5 executor"
                );
            }
            report
        }
        FacadeEnvelope::ProductionMissingTransition(missing) => {
            let Some(action) = scenario.actions.get(missing.action_index as usize) else {
                panic!(
                    "HARNESS FAILURE: missing_boundary_action_out_of_range action_index={} action_count={}",
                    missing.action_index,
                    scenario.actions.len()
                );
            };
            let Some((expected_boundary, expected_code, expected_event)) =
                action.missing_boundary()
            else {
                panic!(
                    "HARNESS FAILURE: action_has_no_missing_boundary action_index={}",
                    missing.action_index
                );
            };
            let protocol_is_supported_red = matches!(
                missing.current_protocol,
                ProtocolVersion::V3 | ProtocolVersion::V5
            );
            let evidence_matches = missing.evidence.code == expected_code
                && missing.evidence.event == expected_event
                && missing.evidence.generation_before == missing.evidence.generation_after
                && is_safe_fingerprint(&missing.evidence.fingerprint);
            if action.kind() != missing.action_kind
                || missing.reached_boundary != expected_boundary
                || !protocol_is_supported_red
                || !evidence_matches
            {
                panic!(
                    "HARNESS FAILURE: missing_boundary_mismatch action_index={}",
                    missing.action_index
                );
            }
            panic!(
                "FUNCTIONAL RED: boundary={:?} action={:?} action_index={} protocol={:?} evidence={:?} event={:?} generation_before={} generation_after={} fingerprint={}",
                missing.reached_boundary,
                missing.action_kind,
                missing.action_index,
                missing.current_protocol,
                missing.evidence.code,
                missing.evidence.event,
                missing.evidence.generation_before,
                missing.evidence.generation_after,
                missing.evidence.fingerprint,
            );
        }
        FacadeEnvelope::HarnessFailure(failure) => {
            if failure
                .evidence_fingerprint
                .as_deref()
                .is_some_and(|fingerprint| !is_safe_fingerprint(fingerprint))
            {
                panic!("HARNESS FAILURE: malformed_failure_fingerprint");
            }
            panic!(
                "HARNESS FAILURE: stage={:?} code={:?} action_index={:?} numeric_detail={:?} evidence_fingerprint={:?}",
                failure.stage,
                failure.code,
                failure.action_index,
                failure.numeric_detail,
                failure.evidence_fingerprint,
            );
        }
    }
}

fn checkpoint<'a>(report: &'a ScenarioReport, label: &str) -> &'a Snapshot {
    let snapshot = report
        .checkpoints
        .get(label)
        .unwrap_or_else(|| panic!("HARNESS FAILURE: missing checkpoint {label}"));
    assert_snapshot_accounting(snapshot);
    snapshot
}

fn corrupted_checkpoint<'a>(report: &'a ScenarioReport, label: &str) -> &'a Snapshot {
    report
        .checkpoints
        .get(label)
        .unwrap_or_else(|| panic!("HARNESS FAILURE: missing corruption checkpoint {label}"))
}

fn response<'a>(report: &'a ScenarioReport, label: &str) -> &'a ResponseObservation {
    let observed = report
        .responses
        .get(label)
        .unwrap_or_else(|| panic!("HARNESS FAILURE: missing response {label}"));
    if let Some(key) = &observed.key {
        assert_receipt_key(key);
    }
    if let Some(terminal) = &observed.terminal {
        assert_terminal(terminal);
    }
    if let Some(task) = &observed.task {
        assert_task_observation(task);
    }
    if let Some(acknowledgement) = &observed.acknowledgement {
        assert_acknowledgement(acknowledgement);
    }
    observed
}

fn task_read<'a>(report: &'a ScenarioReport, label: &str) -> &'a TaskObservation {
    let task = report
        .task_reads
        .get(label)
        .unwrap_or_else(|| panic!("HARNESS FAILURE: missing task read {label}"));
    assert_task_observation(task);
    task
}

fn load_run<'a>(report: &'a ScenarioReport, label: &str) -> &'a LoadObservation {
    let load = report
        .load_runs
        .get(label)
        .unwrap_or_else(|| panic!("HARNESS FAILURE: missing load run {label}"));
    assert_load_raw_bounds(load);
    load
}

fn only_receipt(snapshot: &Snapshot) -> &ReceiptObservation {
    assert_eq!(snapshot.receipts.len(), 1, "expected one exact receipt");
    &snapshot.receipts[0]
}

fn only_task(snapshot: &Snapshot) -> &TaskObservation {
    assert_eq!(snapshot.tasks.len(), 1, "expected one exact task");
    &snapshot.tasks[0]
}

fn only_task_link(snapshot: &Snapshot) -> &TaskLinkObservation {
    assert_eq!(snapshot.task_links.len(), 1, "expected one exact TaskLink");
    &snapshot.task_links[0]
}

fn task_link_state(link: &TaskLinkObservation) -> SeedReceiptState {
    match &link.lifecycle {
        TaskLinkLifecycleObservation::TaskBoundNotBegun { .. } => {
            SeedReceiptState::TaskBoundNotBegun
        }
        TaskLinkLifecycleObservation::TaskBoundBegun { .. } => SeedReceiptState::TaskBoundBegun,
        TaskLinkLifecycleObservation::TaskTerminalBound { .. }
        | TaskLinkLifecycleObservation::TaskRetirementPending { .. } => {
            SeedReceiptState::TaskTerminalBound
        }
    }
}

fn retirement_pendings(snapshot: &Snapshot) -> Vec<&TaskRetirementPendingObservation> {
    snapshot
        .task_links
        .iter()
        .filter_map(|link| match &link.lifecycle {
            TaskLinkLifecycleObservation::TaskRetirementPending { pending } => Some(pending),
            TaskLinkLifecycleObservation::TaskBoundNotBegun { .. }
            | TaskLinkLifecycleObservation::TaskBoundBegun { .. }
            | TaskLinkLifecycleObservation::TaskTerminalBound { .. } => None,
        })
        .collect()
}

fn assert_event_order(report: &ScenarioReport, expected: &[EventKind]) {
    let mut cursor = 0;
    let mut previous_sequence = None;
    for record in &report.events {
        if record.event == expected[cursor] {
            if let Some(previous) = previous_sequence {
                assert!(record.sequence > previous, "event sequence must be strict");
            }
            previous_sequence = Some(record.sequence);
            cursor += 1;
            if cursor == expected.len() {
                return;
            }
        }
    }
    panic!(
        "missing ordered event subsequence {expected:?} in {:#?}",
        report.events
    );
}

fn count_event(report: &ScenarioReport, event: EventKind) -> usize {
    report
        .events
        .iter()
        .filter(|record| record.event == event)
        .count()
}

fn assert_safe_fingerprint(value: &str) {
    assert!(is_safe_fingerprint(value), "invalid SHA-256 fingerprint");
}

fn is_safe_fingerprint(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn assert_canonical_uuid_v4(value: &str) {
    let bytes = value.as_bytes();
    assert_eq!(
        bytes.len(),
        36,
        "UUID must use canonical hyphenated spelling"
    );
    for (index, byte) in bytes.iter().copied().enumerate() {
        if matches!(index, 8 | 13 | 18 | 23) {
            assert_eq!(byte, b'-', "UUID hyphen at byte {index}");
        } else {
            assert!(
                byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte),
                "UUID must be normalized lowercase hex"
            );
        }
    }
    assert_eq!(bytes[14], b'4', "UUID must be version 4");
    assert!(matches!(bytes[19], b'8' | b'9' | b'a' | b'b'));
}

fn assert_receipt_key(key: &ReceiptKeyObservation) {
    assert_canonical_uuid_v4(&key.invocation_id);
    assert_canonical_uuid_v4(&key.reserved_task_id);
    assert_safe_fingerprint(&key.core_identity_digest);
    let _ = tool_wire_name(key.tool);
    assert_safe_fingerprint(&key.normalized_arguments_hash);
    assert_safe_fingerprint(&key.request_scope_hash.0);
    assert_safe_fingerprint(&key.key_digest);
    assert_eq!(
        key.key_digest,
        receipt_key_digest_for_test(
            &key.invocation_id,
            &key.reserved_task_id,
            &key.core_identity_digest,
            tool_wire_name(key.tool),
            &key.normalized_arguments_hash,
            &key.request_scope_hash.0,
        )
    );
}

fn tool_wire_name(tool: ToolIdentityObservation) -> &'static str {
    match tool {
        ToolIdentityObservation::View => "unica.view",
        ToolIdentityObservation::Apply => "unica.apply",
        ToolIdentityObservation::Find => "unica.find",
        ToolIdentityObservation::Search => "unica.search",
        ToolIdentityObservation::Check => "unica.check",
        ToolIdentityObservation::Diff => "unica.diff",
        ToolIdentityObservation::Run => "unica.run",
        ToolIdentityObservation::Docs => "unica.docs",
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut encoded = String::with_capacity(64);
    for byte in digest {
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}

#[allow(clippy::chunks_exact_to_as_chunks)]
fn decode_hex(value: &str, maximum_bytes: usize) -> Vec<u8> {
    assert!(
        !value.is_empty(),
        "canonical byte evidence must be non-empty"
    );
    assert_eq!(value.len() % 2, 0, "hex evidence must contain whole bytes");
    assert!(
        value.len() / 2 <= maximum_bytes,
        "hex evidence exceeds bound"
    );
    assert!(
        value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "hex evidence must be normalized lowercase"
    );
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let nibble = |byte: u8| match byte {
                b'0'..=b'9' => byte - b'0',
                b'a'..=b'f' => byte - b'a' + 10,
                _ => unreachable!("validated lowercase hex"),
            };
            (nibble(pair[0]) << 4) | nibble(pair[1])
        })
        .collect()
}

fn canonical_domain_result_bytes(result: &DomainResultObservation) -> Vec<u8> {
    serde_json::to_vec(result)
        .unwrap_or_else(|_| panic!("HARNESS FAILURE: DomainResult evidence encode"))
}

fn production_canonical_terminal(terminal: &TerminalObservation) -> (Vec<u8>, String) {
    let input = match terminal {
        TerminalObservation::Completed { result, .. } => {
            serde_json::json!({"status": "completed", "result": result})
        }
        TerminalObservation::Failed { reason, .. } => {
            serde_json::json!({"status": "failed", "reason": reason})
        }
        TerminalObservation::Cancelled { .. } => serde_json::json!({"status": "cancelled"}),
    };
    canonical_v5_terminal_for_test(
        &serde_json::to_string(&input)
            .unwrap_or_else(|_| panic!("HARNESS FAILURE: terminal test input encode")),
    )
}

fn assert_domain_result(result: &DomainResultObservation, canonical_result: &[u8]) {
    assert!(
        canonical_result.len() <= MAX_CANONICAL_RESULT_BYTES as usize,
        "canonical DomainResult exceeds its 8 MiB product cap"
    );
    let raw: DomainResultObservation = serde_json::from_slice(canonical_result)
        .unwrap_or_else(|_| panic!("terminal DomainResult must be strict canonical JSON"));
    assert_eq!(&raw, result, "typed DomainResult must match its raw bytes");
    assert_eq!(
        canonical_result,
        canonical_domain_result_bytes(result),
        "DomainResult bytes must use the frozen canonical encoder"
    );
    assert!(!result.summary.is_empty());
    assert!(result.summary.len() <= MAX_CANONICAL_RESULT_BYTES as usize);
    if let Some(at) = &result.at {
        assert!(at.len() <= MAX_CANONICAL_RESULT_BYTES as usize);
    }
    if let Some(data) = &result.data {
        assert!(
            serde_json::to_vec(data).expect("JSON value encodes").len() <= canonical_result.len()
        );
    }
    for values in [
        &result.changed,
        &result.warnings,
        &result.diagnostics,
        &result.artifacts,
        &result.next,
    ] {
        assert!(values.len() <= canonical_result.len());
    }
    for token in [&result.rev, &result.cursor].into_iter().flatten() {
        assert!(token.len() <= canonical_result.len());
    }
}

fn terminal_payload_hex(terminal: &TerminalObservation) -> &str {
    match terminal {
        TerminalObservation::Completed {
            canonical_payload_hex,
            ..
        }
        | TerminalObservation::Failed {
            canonical_payload_hex,
            ..
        }
        | TerminalObservation::Cancelled {
            canonical_payload_hex,
            ..
        } => canonical_payload_hex,
    }
}

fn terminal_digest(terminal: &TerminalObservation) -> &str {
    match terminal {
        TerminalObservation::Completed {
            terminal_digest, ..
        }
        | TerminalObservation::Failed {
            terminal_digest, ..
        }
        | TerminalObservation::Cancelled {
            terminal_digest, ..
        } => terminal_digest,
    }
}

fn terminal_epoch_ms(terminal: &TerminalObservation) -> u64 {
    match terminal {
        TerminalObservation::Completed {
            terminal_epoch_ms, ..
        }
        | TerminalObservation::Failed {
            terminal_epoch_ms, ..
        }
        | TerminalObservation::Cancelled {
            terminal_epoch_ms, ..
        } => *terminal_epoch_ms,
    }
}

fn assert_artifact(evidence: &ArtifactEvidence, maximum_bytes: usize) -> Vec<u8> {
    let raw = decode_hex(&evidence.raw_hex, maximum_bytes);
    assert_eq!(evidence.encoded_bytes, raw.len() as u64);
    assert_eq!(evidence.sha256, sha256_hex(&raw));
    raw
}

fn decode_v5_stored_record(evidence: &ArtifactEvidence) -> V5StoredInvocationRecordObservation {
    let raw = assert_artifact(evidence, MAX_RESPONSE_LINE_BYTES as usize);
    let value: serde_json::Value = serde_json::from_slice(&raw)
        .unwrap_or_else(|_| panic!("v5 Task record must be one strict JSON object"));
    assert_exact_json_keys(
        &value,
        &[
            "schemaVersion",
            "taskId",
            "invocationId",
            "receiptKeyDigest",
            "tool",
            "normalizedArgumentsHash",
            "workspaceIdentityHash",
            "createdAtEpochMs",
            "updatedAtEpochMs",
            "ttlMs",
            "pollIntervalMs",
            "version",
            "cancelRequested",
            "task",
        ],
    );
    let record: V5StoredInvocationRecordObservation = serde_json::from_value(value)
        .unwrap_or_else(|_| panic!("v5 Task record must satisfy its closed schema-v1 algebra"));
    assert_eq!(record.schema_version, 1);
    assert_canonical_uuid_v4(&record.task_id);
    assert_canonical_uuid_v4(&record.invocation_id);
    assert_safe_fingerprint(&record.receipt_key_digest);
    assert_safe_fingerprint(&record.normalized_arguments_hash);
    assert_safe_fingerprint(&record.workspace_identity_hash);
    assert!(record.updated_at_epoch_ms >= record.created_at_epoch_ms);
    assert_eq!(record.ttl_ms, DIRECT_TASK_TTL_MS);
    assert!(record.poll_interval_ms > 0 && record.poll_interval_ms <= CUTOFF_MS);
    assert!(record.version > 0);
    record
}

fn count_json_field(value: &serde_json::Value, field: &str) -> usize {
    match value {
        serde_json::Value::Object(fields) => {
            usize::from(fields.contains_key(field))
                + fields
                    .values()
                    .map(|nested| count_json_field(nested, field))
                    .sum::<usize>()
        }
        serde_json::Value::Array(values) => values
            .iter()
            .map(|nested| count_json_field(nested, field))
            .sum(),
        _ => 0,
    }
}

fn decode_persisted_record_artifact(evidence: &ArtifactEvidence) -> serde_json::Value {
    let raw = assert_artifact(evidence, MAX_RESPONSE_LINE_BYTES as usize);
    assert_ne!(
        raw.last(),
        Some(&b'\n'),
        "persisted records are not JSONL frames"
    );
    let record: serde_json::Value = serde_json::from_slice(&raw)
        .unwrap_or_else(|_| panic!("persisted record artifact must be one exact JSON object"));
    assert!(record.is_object());
    for wire_only in ["kind", "protocolVersion", "outcome"] {
        assert_eq!(
            count_json_field(&record, wire_only),
            0,
            "persisted record must not embed a transient wire envelope"
        );
    }
    record
}

fn assert_staged_transfer_certificate(preparation: &StagedTerminalPreparationObservation) {
    let observed = &preparation.transfer_size_certificate;
    assert!(
        preparation.terminal_payload_prepared_sequence < observed.issued_sequence,
        "the sole codec must size the already-canonical terminal"
    );
    assert!(observed.issued_sequence < preparation.stage_commit_sequence);
    let raw = assert_artifact(&observed.certificate, 64 * 1_024);
    let raw_value: serde_json::Value = serde_json::from_slice(&raw)
        .unwrap_or_else(|_| panic!("transfer certificate must be one exact JSON record"));
    assert_exact_json_keys(
        &raw_value,
        &[
            "certificateVersion",
            "protocolIdentity",
            "coreIdentityDigest",
            "receiptKeyDigest",
            "taskId",
            "invocationId",
            "taskLinkDigest",
            "terminalDigest",
            "terminalEpochMs",
            "receiptRecordSchemaVersion",
            "taskRecordSchemaVersion",
            "lifecycleLinkRecordSchemaVersion",
            "terminalCodecVersion",
            "maxDaemonResponseLineBytes",
            "maxTaskLifecycleLinkRecordBytes",
            "stagedReceiptRecordMaxBytes",
            "taskTerminalBoundLinkRecordMaxBytes",
            "taskPublicationCases",
            "capacityFallbackCases",
        ],
    );
    let certificate: StagedTerminalTransferSizeCertificate = serde_json::from_value(raw_value)
        .unwrap_or_else(|_| panic!("transfer certificate must use its strict schema-v1 codec"));
    assert_eq!(certificate.certificate_version, 1);
    assert_eq!(certificate.protocol_identity, "v5");
    assert_eq!(
        certificate.core_identity_digest,
        preparation.receipt_key.core_identity_digest
    );
    assert_eq!(
        certificate.receipt_key_digest,
        preparation.receipt_key.key_digest
    );
    assert_eq!(
        certificate.task_id,
        preparation.receipt_key.reserved_task_id
    );
    assert_eq!(
        certificate.invocation_id,
        preparation.receipt_key.invocation_id
    );
    assert_eq!(certificate.task_link_digest, preparation.task_link_digest);
    assert_eq!(
        certificate.terminal_digest,
        terminal_digest(&preparation.terminal)
    );
    assert_eq!(
        certificate.terminal_epoch_ms,
        terminal_epoch_ms(&preparation.terminal)
    );
    assert_eq!(certificate.receipt_record_schema_version, 1);
    assert_eq!(certificate.task_record_schema_version, 1);
    assert_eq!(certificate.lifecycle_link_record_schema_version, 1);
    assert_eq!(certificate.terminal_codec_version, 1);
    assert_eq!(
        certificate.max_daemon_response_line_bytes,
        MAX_RESPONSE_LINE_BYTES
    );
    assert_eq!(certificate.max_task_lifecycle_link_record_bytes, 1_024);
    assert!(
        certificate.task_terminal_bound_link_record_max_bytes
            <= certificate.max_task_lifecycle_link_record_bytes
    );
    assert!(
        preparation.staged_receipt_record.encoded_bytes
            <= certificate.staged_receipt_record_max_bytes
    );

    let task_link = decode_persisted_record_artifact(&observed.terminal_bound_link_record);
    assert!(observed.terminal_bound_link_record.encoded_bytes <= 1_024);
    assert!(
        observed.terminal_bound_link_record.encoded_bytes
            <= certificate.task_terminal_bound_link_record_max_bytes
    );
    assert_eq!(count_json_field(&task_link, "result"), 0);
    assert_eq!(
        find_json_field(&task_link, "receiptKeyDigest").and_then(serde_json::Value::as_str),
        Some(preparation.receipt_key.key_digest.as_str())
    );
    assert_eq!(
        find_json_field(&task_link, "taskId").and_then(serde_json::Value::as_str),
        Some(preparation.receipt_key.reserved_task_id.as_str())
    );
    assert_eq!(
        find_json_field(&task_link, "linkDigest").and_then(serde_json::Value::as_str),
        Some(preparation.task_link_digest.as_str())
    );

    assert_eq!(observed.cases.len(), 5);
    assert_eq!(
        certificate.task_publication_cases.len(),
        observed.cases.len()
    );
    let expected_shapes = [
        None,
        Some((TaskStatus::Queued, false, u64::MAX)),
        Some((TaskStatus::Queued, true, u64::MAX)),
        Some((TaskStatus::Working, false, u64::MAX)),
        Some((TaskStatus::Working, true, u64::MAX)),
    ];
    for (index, expected_shape) in expected_shapes.into_iter().enumerate() {
        let observed_case = &observed.cases[index];
        let certified_case = &certificate.task_publication_cases[index];
        assert_eq!(observed_case.shape(), expected_shape);
        assert_eq!(certified_case.shape(), expected_shape);
        let (task_record, response_frame) = observed_case.artifacts();
        let stored = decode_v5_stored_record(task_record);
        assert_eq!(stored.task_id, preparation.receipt_key.reserved_task_id);
        assert_eq!(stored.invocation_id, preparation.receipt_key.invocation_id);
        if let Some((_, cancel_requested, version)) = expected_shape {
            assert_eq!(stored.cancel_requested, cancel_requested);
            assert_eq!(version, u64::MAX);
        }
        assert!(matches!(
            stored.task,
            V5StoredTaskObservation::Completed { .. }
                | V5StoredTaskObservation::Failed { .. }
                | V5StoredTaskObservation::Cancelled { .. }
        ));
        let frame = assert_artifact(response_frame, MAX_RESPONSE_LINE_BYTES as usize);
        assert_eq!(frame.last(), Some(&b'\n'));
        let bounds = certified_case.bounds();
        assert!(task_record.encoded_bytes <= bounds.0);
        assert!(response_frame.encoded_bytes <= bounds.1);
    }

    assert_eq!(observed.capacity_fallback_cases.len(), 1);
    assert_eq!(certificate.capacity_fallback_cases.len(), 1);
    let observed_case = &observed.capacity_fallback_cases[0];
    let certified_case = &certificate.capacity_fallback_cases[0];
    assert!(matches!(
        observed_case,
        StagedCapacityFallbackSizeCaseObservation::LinkCapacity { .. }
    ));
    assert!(matches!(
        certified_case,
        StagedCapacityFallbackSizeCaseCertificate::LinkCapacity { .. }
    ));
    let (record, frame) = observed_case.artifacts();
    let record_value = decode_persisted_record_artifact(record);
    assert_eq!(count_json_field(&record_value, "terminalDigest"), 1);
    let frame_bytes = assert_artifact(frame, MAX_RESPONSE_LINE_BYTES as usize);
    assert_eq!(frame_bytes.last(), Some(&b'\n'));
    let bounds = certified_case.bounds();
    assert!(record.encoded_bytes <= bounds.0);
    assert!(frame.encoded_bytes <= bounds.1);
}

fn assert_v5_stored_record_matches_task(
    record: &V5StoredInvocationRecordObservation,
    task: &TaskObservation,
) {
    assert_eq!(record.schema_version, 1);
    assert_eq!(record.task_id, task.task_id);
    assert_eq!(record.invocation_id, task.invocation_id);
    assert_eq!(record.receipt_key_digest, task.receipt_key.key_digest);
    assert_eq!(record.tool, task.receipt_key.tool);
    assert_eq!(
        record.normalized_arguments_hash,
        task.receipt_key.normalized_arguments_hash
    );
    assert_eq!(
        Some(record.workspace_identity_hash.as_str()),
        task.workspace_identity_hash.as_deref()
    );
    assert_eq!(record.created_at_epoch_ms, task.created_epoch_ms);
    assert_eq!(record.updated_at_epoch_ms, task.updated_epoch_ms);
    assert_eq!(record.ttl_ms, task.ttl_ms);
    assert_eq!(record.poll_interval_ms, task.poll_interval_ms);
    assert_eq!(record.version, task.version);
    assert_eq!(record.cancel_requested, task.cancel_requested);
    match (&record.task, task.status, task.terminal.as_ref()) {
        (V5StoredTaskObservation::Queued, TaskStatus::Queued, None)
        | (V5StoredTaskObservation::Working, TaskStatus::Working, None) => {}
        (
            V5StoredTaskObservation::Completed {
                terminal_epoch_ms: epoch,
                terminal_digest: digest,
                result,
            },
            TaskStatus::Completed,
            Some(terminal @ TerminalObservation::Completed { .. }),
        ) => {
            assert_eq!(*epoch, terminal_epoch_ms(terminal));
            assert_eq!(digest, terminal_digest(terminal));
            assert_eq!(result, completed_result(terminal));
        }
        (
            V5StoredTaskObservation::Failed {
                terminal_epoch_ms: epoch,
                terminal_digest: digest,
                reason,
            },
            TaskStatus::Failed,
            Some(terminal @ TerminalObservation::Failed { .. }),
        ) => {
            assert_eq!(*epoch, terminal_epoch_ms(terminal));
            assert_eq!(digest, terminal_digest(terminal));
            assert_eq!(
                terminal_class(Some(terminal)),
                TerminalClass::Failed(*reason)
            );
        }
        (
            V5StoredTaskObservation::Cancelled {
                terminal_epoch_ms: epoch,
                terminal_digest: digest,
            },
            TaskStatus::Cancelled,
            Some(terminal @ TerminalObservation::Cancelled { .. }),
        ) => {
            assert_eq!(*epoch, terminal_epoch_ms(terminal));
            assert_eq!(digest, terminal_digest(terminal));
        }
        other => panic!("stored record/task snapshot algebra mismatch: {other:?}"),
    }
}

fn assert_staged_terminal_preparation(preparation: &StagedTerminalPreparationObservation) {
    assert_receipt_key(&preparation.receipt_key);
    assert_terminal(&preparation.terminal);
    assert_safe_fingerprint(&preparation.workspace_identity_hash);
    assert_safe_fingerprint(&preparation.task_link_digest);
    assert_eq!(
        preparation.task_link_digest,
        task_link_digest_for_test(
            &preparation.receipt_key.key_digest,
            &preparation.receipt_key.reserved_task_id,
            &preparation.receipt_key.invocation_id,
            &preparation.workspace_identity_hash,
        )
    );
    assert!(preparation.receipt_expected_version > 0);
    assert_eq!(
        preparation.committed_receipt_version,
        preparation.receipt_expected_version + 1
    );
    assert!(preparation.terminal_payload_prepared_sequence > 0);
    assert!(preparation.staged_receipt_prepared_sequence > 0);
    assert!(preparation.stage_commit_sequence > preparation.terminal_payload_prepared_sequence);
    assert!(preparation.stage_commit_sequence > preparation.staged_receipt_prepared_sequence);
    assert!(preparation.stage_readback_sequence > preparation.stage_commit_sequence);
    assert_staged_transfer_certificate(preparation);
    let canonical_terminal = decode_hex(
        terminal_payload_hex(&preparation.terminal),
        MAX_RESPONSE_LINE_BYTES as usize,
    );
    assert_eq!(
        assert_artifact(
            &preparation.terminal_payload,
            MAX_RESPONSE_LINE_BYTES as usize
        ),
        canonical_terminal
    );
    let staged_record = decode_persisted_record_artifact(&preparation.staged_receipt_record);
    assert_eq!(count_json_field(&staged_record, "terminalDigest"), 1);
    match &preparation.terminal {
        TerminalObservation::Completed { result, .. } => {
            assert_eq!(count_json_field(&staged_record, "result"), 1);
            let candidate = preparation
                .candidate_result
                .as_ref()
                .expect("staged Completed must preflight the bare DomainResult");
            assert_eq!(
                assert_artifact(candidate, MAX_CANONICAL_RESULT_BYTES as usize),
                canonical_domain_result_bytes(result)
            );
        }
        TerminalObservation::Failed {
            reason: V5SafeFailureReason::ResultTooLarge,
            ..
        } => {
            assert_eq!(count_json_field(&staged_record, "result"), 0);
            let candidate = preparation
                .candidate_result
                .as_ref()
                .expect("staged ResultTooLarge must retain bounded rejected-candidate evidence");
            let candidate_len =
                assert_artifact(candidate, MAX_CANONICAL_RESULT_BYTES as usize + 1).len();
            assert!(candidate_len > 0);
            assert!(candidate_len <= MAX_CANONICAL_RESULT_BYTES as usize + 1);
        }
        TerminalObservation::Failed { .. } | TerminalObservation::Cancelled { .. } => {
            assert_eq!(count_json_field(&staged_record, "result"), 0);
            assert!(preparation.candidate_result.is_none());
        }
    }
}

fn assert_terminal_preflight(publication: &TerminalPublicationObservation) {
    assert_receipt_key(&publication.receipt_key);
    let terminal = &publication.terminal;
    let preflight = &publication.commit;
    assert!(preflight.terminal_payload_prepared_sequence() > 0);
    match preflight {
        TerminalCommitPreflightObservation::DirectReceiptLedger { receipt }
        | TerminalCommitPreflightObservation::ReceiptBackedTask { receipt } => {
            assert!(receipt.receipt_record_prepared_sequence > 0);
            assert!(receipt.receipt_commit_sequence > receipt.terminal_payload_prepared_sequence);
            assert!(receipt.receipt_commit_sequence > receipt.receipt_record_prepared_sequence);
            assert!(receipt.receipt_expected_version > 0);
            let receipt_record = decode_persisted_record_artifact(&receipt.receipt_record);
            assert_eq!(count_json_field(&receipt_record, "terminalDigest"), 1);
            assert_eq!(
                count_json_field(&receipt_record, "result"),
                usize::from(matches!(terminal, TerminalObservation::Completed { .. }))
            );
        }
        TerminalCommitPreflightObservation::BoundTaskStore { task } => {
            assert!(task.task_record_prepared_sequence > 0);
            assert!(task.task_store_commit_sequence > task.task_record_prepared_sequence);
            assert!(task.task_store_readback_sequence > task.task_store_commit_sequence);
            assert!(task.task_expected_version > 0);
            assert!(task.lifecycle_link_record_prepared_sequence > 0);
            assert!(task.lifecycle_link_commit_sequence > task.task_store_readback_sequence);
            assert!(
                task.lifecycle_link_commit_sequence > task.lifecycle_link_record_prepared_sequence
            );
            assert!(task.lifecycle_link_expected_version > 0);
            assert_safe_fingerprint(&task.task_link_digest);
            let task_record = decode_persisted_record_artifact(&task.task_record);
            assert_eq!(
                count_json_field(&task_record, "result"),
                usize::from(matches!(terminal, TerminalObservation::Completed { .. }))
            );
            let link_record = decode_persisted_record_artifact(&task.lifecycle_link_record);
            assert!(task.lifecycle_link_record.encoded_bytes <= 1_024);
            assert_eq!(count_json_field(&link_record, "result"), 0);
            assert_eq!(
                find_json_field(&link_record, "linkDigest").and_then(serde_json::Value::as_str),
                Some(task.task_link_digest.as_str())
            );
        }
        TerminalCommitPreflightObservation::StagedHandoffTask { task } => {
            assert!(task.task_record_prepared_sequence > 0);
            assert!(task.task_store_commit_sequence > task.task_record_prepared_sequence);
            assert!(task.task_store_readback_sequence > task.task_store_commit_sequence);
            assert!(task.lifecycle_link_record_prepared_sequence > 0);
            assert!(task.lifecycle_link_commit_sequence > task.task_store_readback_sequence);
            assert!(
                task.lifecycle_link_commit_sequence > task.lifecycle_link_record_prepared_sequence
            );
            assert!(task.committed_task_version > 0);
            assert_safe_fingerprint(&task.live_task_link_reservation_fingerprint);
            assert_safe_fingerprint(&task.task_link_digest);
            assert!(task.staged_receipt_version > 0);
            assert_safe_fingerprint(&task.staged_receipt_record_sha256);
            assert_safe_fingerprint(&task.staged_terminal_digest);
            assert_eq!(task.staged_terminal_digest, terminal_digest(terminal));
            match &task.terminal_write_expectation {
                StagedTaskTerminalWriteExpectation::Absent {
                    task_store_generation,
                } => {
                    assert!(*task_store_generation > 0);
                    assert!(matches!(
                        task.terminal_write_branch,
                        StagedTaskTerminalWriteBranch::CreatedTerminal
                            | StagedTaskTerminalWriteBranch::ExactTerminalReadback
                    ));
                }
                StagedTaskTerminalWriteExpectation::ExactProvisional {
                    task_id,
                    invocation_id,
                    expected_version,
                    status,
                    cancel_requested,
                    task_identity_digest,
                    task_link_digest,
                    provisional_task_store_readback,
                } => {
                    assert_canonical_uuid_v4(task_id);
                    assert_canonical_uuid_v4(invocation_id);
                    assert_eq!(task_id, &publication.receipt_key.reserved_task_id);
                    assert_eq!(invocation_id, &publication.receipt_key.invocation_id);
                    assert!(*expected_version > 0);
                    assert!(matches!(status, TaskStatus::Queued | TaskStatus::Working));
                    assert_safe_fingerprint(task_identity_digest);
                    assert_safe_fingerprint(task_link_digest);
                    assert_eq!(task_link_digest, &task.task_link_digest);
                    let provisional = decode_v5_stored_record(provisional_task_store_readback);
                    assert_eq!(provisional.task_id, *task_id);
                    assert_eq!(provisional.invocation_id, *invocation_id);
                    assert_eq!(provisional.version, *expected_version);
                    assert_eq!(provisional.cancel_requested, *cancel_requested);
                    assert_eq!(
                        match provisional.task {
                            V5StoredTaskObservation::Queued => TaskStatus::Queued,
                            V5StoredTaskObservation::Working => TaskStatus::Working,
                            V5StoredTaskObservation::Completed { .. }
                            | V5StoredTaskObservation::Failed { .. }
                            | V5StoredTaskObservation::Cancelled { .. } => {
                                panic!("ExactProvisional cannot bind a terminal Task readback")
                            }
                        },
                        *status
                    );
                    assert!(matches!(
                        task.terminal_write_branch,
                        StagedTaskTerminalWriteBranch::ReplacedExactProvisional
                            | StagedTaskTerminalWriteBranch::ExactTerminalReadback
                    ));
                }
            }
            if let Some(repeat) = &task.idempotent_repeat {
                assert!(repeat.readback_sequence > task.task_store_readback_sequence);
                assert_eq!(repeat.task_record, task.task_record);
                assert_eq!(
                    repeat.task_store_generation_before, repeat.task_store_generation_after,
                    "same-terminal retry must be an idempotent readback"
                );
                let repeated = decode_v5_stored_record(&repeat.task_record);
                assert!(matches!(
                    repeated.task,
                    V5StoredTaskObservation::Completed { .. }
                        | V5StoredTaskObservation::Failed { .. }
                        | V5StoredTaskObservation::Cancelled { .. }
                ));
            }
            let task_record = decode_persisted_record_artifact(&task.task_record);
            assert_eq!(
                count_json_field(&task_record, "result"),
                usize::from(matches!(terminal, TerminalObservation::Completed { .. }))
            );
            let link_record = decode_persisted_record_artifact(&task.lifecycle_link_record);
            assert!(task.lifecycle_link_record.encoded_bytes <= 1_024);
            assert_eq!(count_json_field(&link_record, "result"), 0);
            assert_eq!(
                find_json_field(&link_record, "linkDigest").and_then(serde_json::Value::as_str),
                Some(task.task_link_digest.as_str())
            );
        }
    }
    let payload = decode_hex(
        terminal_payload_hex(terminal),
        MAX_RESPONSE_LINE_BYTES as usize,
    );
    assert_eq!(
        production_canonical_terminal(terminal).1,
        terminal_digest(terminal),
    );
    assert_eq!(
        assert_artifact(
            preflight.terminal_payload(),
            MAX_RESPONSE_LINE_BYTES as usize
        ),
        payload
    );
    match terminal {
        TerminalObservation::Completed { result, .. } => {
            let candidate = preflight
                .candidate_result()
                .expect("Completed terminal must preserve candidate preflight evidence");
            let candidate = assert_artifact(candidate, MAX_CANONICAL_RESULT_BYTES as usize);
            assert_eq!(
                candidate,
                canonical_domain_result_bytes(result),
                "candidate result is the bare canonical DomainResult, not the terminal wrapper"
            );
            assert_domain_result(result, &candidate);
        }
        TerminalObservation::Failed {
            reason: V5SafeFailureReason::ResultTooLarge,
            ..
        } => {
            let candidate = preflight
                .candidate_result()
                .expect("result-too-large terminal must expose rejected candidate bytes");
            let rejected = assert_artifact(candidate, MAX_CANONICAL_RESULT_BYTES as usize + 1);
            assert!(!rejected.is_empty());
            assert!(rejected.len() <= MAX_CANONICAL_RESULT_BYTES as usize + 1);
        }
        TerminalObservation::Failed { .. } | TerminalObservation::Cancelled { .. } => {
            assert!(preflight.candidate_result().is_none());
        }
    }
    for response in &publication.response_frames {
        match response.origin {
            ResponseFrameOrigin::ImmediatePublication => {
                assert!(response.prepared_sequence < preflight.durable_commit_sequence());
                match preflight {
                    TerminalCommitPreflightObservation::BoundTaskStore { task, .. } => {
                        assert!(response.prepared_sequence < task.task_store_commit_sequence)
                    }
                    TerminalCommitPreflightObservation::StagedHandoffTask { task, .. } => {
                        assert!(response.prepared_sequence < task.task_store_commit_sequence)
                    }
                    TerminalCommitPreflightObservation::DirectReceiptLedger { .. }
                    | TerminalCommitPreflightObservation::ReceiptBackedTask { .. } => {}
                }
            }
            ResponseFrameOrigin::ExactDuplicate | ResponseFrameOrigin::Recovery => {
                assert!(response.prepared_sequence > preflight.durable_commit_sequence());
                assert!(response.write_sequence.is_some());
            }
        }
        if let Some(write_sequence) = response.write_sequence {
            assert!(write_sequence > response.prepared_sequence);
            assert!(write_sequence > preflight.durable_commit_sequence());
        }
        let frame = assert_artifact(&response.response_jsonl, MAX_RESPONSE_LINE_BYTES as usize);
        assert_eq!(frame.last(), Some(&b'\n'));
        assert_ne!(frame.get(frame.len().saturating_sub(2)), Some(&b'\n'));
        let value: serde_json::Value = serde_json::from_slice(&frame[..frame.len() - 1])
            .unwrap_or_else(|_| panic!("preflight response must be exact JSONL"));
        assert_eq!(
            find_json_field(&value, "terminalDigest").and_then(serde_json::Value::as_str),
            Some(terminal_digest(terminal))
        );
        if let Some(raw_terminal) = find_json_field(&value, "terminal") {
            let (canonical, digest) = production_canonical_terminal(terminal);
            assert_eq!(
                raw_terminal,
                &serde_json::from_slice::<serde_json::Value>(&canonical)
                    .expect("production canonical terminal is JSON")
            );
            assert_eq!(digest, terminal_digest(terminal));
        }
        match response.response_kind {
            ResponseKind::Direct
            | ResponseKind::Task
            | ResponseKind::RecoveredDirect
            | ResponseKind::Cancelled => {}
            ResponseKind::Acknowledged
            | ResponseKind::Rejected
            | ResponseKind::Tombstone
            | ResponseKind::NotFound => {
                panic!("terminal response frame cannot use nonterminal response kind")
            }
        }
    }
}

fn terminal_class(terminal: Option<&TerminalObservation>) -> TerminalClass {
    match terminal {
        None => TerminalClass::Absent,
        Some(TerminalObservation::Completed { result, .. }) => TerminalClass::Completed(result.ok),
        Some(TerminalObservation::Failed { reason, .. }) => TerminalClass::Failed(*reason),
        Some(TerminalObservation::Cancelled { .. }) => TerminalClass::Cancelled,
    }
}

fn assert_terminal(terminal: &TerminalObservation) {
    let payload = decode_hex(
        terminal_payload_hex(terminal),
        MAX_RESPONSE_LINE_BYTES as usize,
    );
    assert_eq!(
        payload,
        production_canonical_terminal(terminal).0,
        "terminal payload must come from the production canonical V5TerminalOutcome codec"
    );
    assert_safe_fingerprint(terminal_digest(terminal));
    assert_eq!(
        terminal_digest(terminal),
        production_canonical_terminal(terminal).1
    );
    assert!(terminal_epoch_ms(terminal) > 0);
    match terminal {
        TerminalObservation::Completed { result, .. } => {
            assert_domain_result(result, &canonical_domain_result_bytes(result));
        }
        TerminalObservation::Failed { reason, .. } => match reason {
            V5SafeFailureReason::InvocationFailed
            | V5SafeFailureReason::ResultTooLarge
            | V5SafeFailureReason::Interrupted
            | V5SafeFailureReason::OutcomeUncertain
            | V5SafeFailureReason::TaskCapacity
            | V5SafeFailureReason::PersistenceFailed
            | V5SafeFailureReason::ResumeUnsupported
            | V5SafeFailureReason::WorkspaceCapacity
            | V5SafeFailureReason::WorkspaceRegistryFailed => {}
        },
        TerminalObservation::Cancelled { .. } => {
            assert_eq!(payload, br#"{"status":"cancelled"}"#);
            assert_eq!(
                terminal_digest(terminal),
                "f2d0423d2613a0d09397b750542e4542f7653d78ebd5e0448f1326d09145d9ae",
                "Cancelled is the independent literal canonical-terminal golden vector"
            );
        }
    }
}

fn completed_result(terminal: &TerminalObservation) -> &DomainResultObservation {
    match terminal {
        TerminalObservation::Completed { result, .. } => result,
        other => panic!("expected Completed terminal, got {other:#?}"),
    }
}

fn response_terminal(response: &ResponseObservation) -> &TerminalObservation {
    response
        .terminal
        .as_ref()
        .expect("terminal response must expose the exact tagged terminal")
}

fn response_key(response: &ResponseObservation) -> &ReceiptKeyObservation {
    response
        .key
        .as_ref()
        .expect("receipt response must expose the exact full key")
}

fn terminal_of_receipt(receipt: &ReceiptObservation) -> &TerminalObservation {
    receipt
        .terminal
        .as_ref()
        .expect("terminal receipt must expose the exact tagged terminal")
}

fn terminal_publication_for<'a>(
    report: &'a ScenarioReport,
    key: &ReceiptKeyObservation,
    terminal: &TerminalObservation,
) -> &'a TerminalPublicationObservation {
    let matching: Vec<_> = report
        .terminal_publications
        .iter()
        .filter(|publication| publication.receipt_key == *key && publication.terminal == *terminal)
        .collect();
    assert_eq!(
        matching.len(),
        1,
        "each durable terminal commit must have one exact publication preflight"
    );
    matching[0]
}

fn assert_publication_matches_snapshot<'a>(
    report: &'a ScenarioReport,
    snapshot: &Snapshot,
    key: &ReceiptKeyObservation,
    terminal: &TerminalObservation,
    owner: TerminalPublicationOwner,
) -> &'a TerminalPublicationObservation {
    let publication = terminal_publication_for(report, key, terminal);
    assert_eq!(publication.commit.owner(), owner);
    match &publication.commit {
        TerminalCommitPreflightObservation::DirectReceiptLedger { receipt: commit }
        | TerminalCommitPreflightObservation::ReceiptBackedTask { receipt: commit } => {
            let receipt = snapshot
                .receipts
                .iter()
                .find(|receipt| receipt.key == *key)
                .expect(
                    "receipt-owned terminal publication must have its exact active-record readback",
                );
            assert_eq!(receipt.terminal.as_ref(), Some(terminal));
            assert_eq!(
                receipt.version,
                commit.receipt_expected_version + 1,
                "receipt terminal write must consume its own exact expected version"
            );
            assert_eq!(receipt.encoded_bytes, commit.receipt_record.encoded_bytes);
            assert!(
                snapshot.task_links.iter().all(|link| link.key != *key),
                "receipt-owned terminal must not also materialize a bound lifecycle link"
            );
        }
        TerminalCommitPreflightObservation::BoundTaskStore {
            task: task_commit, ..
        } => {
            assert!(
                snapshot.receipts.iter().all(|receipt| receipt.key != *key),
                "bound lifecycle metadata is represented solely by TaskLink"
            );
            let task = snapshot
                .tasks
                .iter()
                .find(|task| task.receipt_key == *key)
                .expect("bound terminal publication must have the exact TaskStore readback");
            assert_eq!(task.terminal.as_ref(), Some(terminal));
            assert_eq!(task.encoded_bytes, task_commit.task_record.encoded_bytes);
            let stored = decode_v5_stored_record(&task_commit.task_record);
            assert_v5_stored_record_matches_task(&stored, task);
            assert_eq!(task.version, task_commit.task_expected_version + 1);
            let link = snapshot
                .task_links
                .iter()
                .find(|link| link.key == *key)
                .expect("bound publication must preserve the exact materialized Task link");
            assert_eq!(task_commit.task_link_digest, link.link_digest);
            assert_eq!(
                link.encoded_bytes,
                task_commit.lifecycle_link_record.encoded_bytes
            );
            assert_eq!(link.version, task_commit.committed_lifecycle_link_version);
            assert_eq!(
                task_commit.committed_lifecycle_link_version,
                task_commit.lifecycle_link_expected_version + 1
            );
            assert_terminal_bound_link(link, task, terminal);
        }
        TerminalCommitPreflightObservation::StagedHandoffTask {
            task: task_commit, ..
        } => {
            assert!(
                snapshot.receipts.iter().all(|receipt| receipt.key != *key),
                "staged transfer must remove the active staged receipt when sole-link ownership commits"
            );
            let task = snapshot
                .tasks
                .iter()
                .find(|task| task.receipt_key == *key)
                .expect("staged handoff publication must exact-read its terminal Task");
            assert_eq!(task.terminal.as_ref(), Some(terminal));
            assert_eq!(task.encoded_bytes, task_commit.task_record.encoded_bytes);
            let stored = decode_v5_stored_record(&task_commit.task_record);
            assert_v5_stored_record_matches_task(&stored, task);
            assert_eq!(task.version, task_commit.committed_task_version);
            let link = snapshot
                .task_links
                .iter()
                .find(|link| link.key == *key)
                .expect("staged handoff publication must atomically materialize its reserved link");
            assert_eq!(task_commit.task_link_digest, link.link_digest);
            assert_eq!(
                link.encoded_bytes,
                task_commit.lifecycle_link_record.encoded_bytes
            );
            assert_eq!(link.version, task_commit.committed_lifecycle_link_version);
            assert_terminal_bound_link(link, task, terminal);
        }
    }
    publication
}

fn assert_terminal_bound_link(
    link: &TaskLinkObservation,
    task: &TaskObservation,
    terminal: &TerminalObservation,
) {
    assert_eq!(link.key, task.receipt_key);
    assert_eq!(link.task_id, task.task_id);
    assert_eq!(link.invocation_id, task.invocation_id);
    assert_eq!(
        link.workspace_identity_hash.as_str(),
        task.workspace_identity_hash
            .as_deref()
            .expect("bound terminal Task must retain its exact workspace identity",)
    );
    match &link.lifecycle {
        TaskLinkLifecycleObservation::TaskTerminalBound {
            terminal_digest: link_terminal_digest,
            terminal_epoch_ms: link_terminal_epoch_ms,
            ttl_ms,
            expires_at_epoch_ms,
            task_version,
        } => {
            assert_eq!(link_terminal_digest, terminal_digest(terminal));
            assert_eq!(*link_terminal_epoch_ms, terminal_epoch_ms(terminal));
            assert_eq!(*ttl_ms, DIRECT_TASK_TTL_MS);
            assert_eq!(*expires_at_epoch_ms, terminal_epoch_ms(terminal) + *ttl_ms);
            assert_eq!(*task_version, task.version);
        }
        other => panic!("terminal Task owner must end in sole TaskTerminalBound link: {other:?}"),
    }
}

fn assert_failed_terminal(terminal: &TerminalObservation, expected: V5SafeFailureReason) {
    match terminal {
        TerminalObservation::Failed { reason, .. } => assert_eq!(*reason, expected),
        other => panic!("expected Failed({expected:?}), got {other:#?}"),
    }
}

fn assert_cancelled_terminal(terminal: &TerminalObservation) {
    assert!(matches!(terminal, TerminalObservation::Cancelled { .. }));
}

fn assert_acknowledgement(acknowledgement: &AcknowledgementObservation) {
    assert_safe_fingerprint(&acknowledgement.terminal_digest);
    assert_eq!(
        acknowledgement.expires_epoch_ms,
        acknowledgement.ack_epoch_ms + TOMBSTONE_TTL_MS
    );
}

fn assert_task_observation(task: &TaskObservation) {
    assert_canonical_uuid_v4(&task.task_id);
    assert_canonical_uuid_v4(&task.invocation_id);
    assert_receipt_key(&task.receipt_key);
    assert_eq!(task.task_id, task.receipt_key.reserved_task_id);
    assert_eq!(task.invocation_id, task.receipt_key.invocation_id);
    assert!(task.updated_epoch_ms >= task.created_epoch_ms);
    assert_eq!(task.ttl_ms, DIRECT_TASK_TTL_MS);
    assert!(task.poll_interval_ms > 0 && task.poll_interval_ms <= CUTOFF_MS);
    assert!(task.encoded_bytes > 0 && task.encoded_bytes <= MAX_RESPONSE_LINE_BYTES);
    if let Some(workspace) = &task.workspace_identity_hash {
        assert_safe_fingerprint(workspace);
    }
    if let Some(terminal) = &task.terminal {
        assert_terminal(terminal);
        assert_eq!(
            task.expires_epoch_ms,
            terminal_epoch_ms(terminal) + task.ttl_ms
        );
    }
}

fn assert_same_stable_task(left: &TaskObservation, right: &TaskObservation) {
    assert_eq!(left.task_id, right.task_id);
    assert_eq!(left.invocation_id, right.invocation_id);
    assert_eq!(left.receipt_key, right.receipt_key);
    assert_eq!(left.workspace_identity_hash, right.workspace_identity_hash);
    assert_eq!(left.created_epoch_ms, right.created_epoch_ms);
    assert_eq!(left.expires_epoch_ms, right.expires_epoch_ms);
    assert_eq!(left.ttl_ms, right.ttl_ms);
    assert_eq!(left.poll_interval_ms, right.poll_interval_ms);
}

fn assert_task_retirement_pending(pending: &TaskRetirementPendingObservation, snapshot: &Snapshot) {
    assert_receipt_key(&pending.receipt_key);
    assert_canonical_uuid_v4(&pending.task_id);
    assert_eq!(pending.task_id, pending.receipt_key.reserved_task_id);
    assert_safe_fingerprint(&pending.task_link_digest);
    assert_safe_fingerprint(&pending.terminal_digest);
    assert_eq!(pending.resolver, TaskRetirementResolver::TaskExpired);
    assert_eq!(pending.ttl_ms, DIRECT_TASK_TTL_MS);
    assert_eq!(
        pending.expires_at_epoch_ms,
        pending.terminal_epoch_ms + pending.ttl_ms
    );
    assert!(pending.expected_task_version > 0);
    assert!(pending.version > 0);
    assert!(pending.lifecycle_link_expected_version > 0);
    assert_eq!(
        pending.committed_lifecycle_link_version,
        pending.lifecycle_link_expected_version + 1
    );
    assert_eq!(pending.version, pending.committed_lifecycle_link_version);
    let raw = assert_artifact(&pending.committed_pending_record, 1_024);
    let raw_value: serde_json::Value = serde_json::from_slice(&raw)
        .unwrap_or_else(|_| panic!("TaskRetirementPending must be one exact JSON record"));
    assert_exact_json_keys(
        &raw_value,
        &[
            "receiptKey",
            "taskId",
            "taskLinkDigest",
            "terminalDigest",
            "terminalEpochMs",
            "ttlMs",
            "expiresAtEpochMs",
            "expectedTaskVersion",
            "resolver",
            "version",
        ],
    );
    let persisted: TaskRetirementPendingRecordObservation = serde_json::from_value(raw_value)
        .unwrap_or_else(|_| panic!("TaskRetirementPending must use its strict persisted codec"));
    assert_eq!(persisted.receipt_key, pending.receipt_key);
    assert_eq!(persisted.task_id, pending.task_id);
    assert_eq!(persisted.task_link_digest, pending.task_link_digest);
    assert_eq!(persisted.terminal_digest, pending.terminal_digest);
    assert_eq!(persisted.terminal_epoch_ms, pending.terminal_epoch_ms);
    assert_eq!(persisted.ttl_ms, pending.ttl_ms);
    assert_eq!(persisted.expires_at_epoch_ms, pending.expires_at_epoch_ms);
    assert_eq!(
        persisted.expected_task_version,
        pending.expected_task_version
    );
    assert_eq!(persisted.resolver, pending.resolver);
    assert_eq!(persisted.version, pending.version);
    let links: Vec<_> = snapshot
        .task_links
        .iter()
        .filter(|link| {
            link.key == pending.receipt_key
                && link.task_id == pending.task_id
                && link.link_digest == pending.task_link_digest
        })
        .collect();
    assert_eq!(
        links.len(),
        1,
        "pending retirement retains its exact TaskLink"
    );
}

fn assert_task_retirement_authorization(
    authorization: &TaskRetirementAuthorizationObservation,
    pending: &TaskRetirementPendingObservation,
) {
    assert_safe_fingerprint(&authorization.authorization_fingerprint);
    assert_canonical_uuid_v4(&authorization.process_instance_id);
    assert!(authorization.generation > 0);
    assert!(authorization.issued_sequence > 0);
    assert_eq!(
        authorization.pending_record_sha256,
        pending.committed_pending_record.sha256
    );
    assert_eq!(authorization.pending_version, pending.version);
    assert_eq!(
        authorization.receipt_key_digest,
        pending.receipt_key.key_digest
    );
    assert_eq!(authorization.task_id, pending.task_id);
    assert_eq!(authorization.task_link_digest, pending.task_link_digest);
    assert_eq!(authorization.terminal_digest, pending.terminal_digest);
    assert_eq!(authorization.terminal_epoch_ms, pending.terminal_epoch_ms);
    assert_eq!(
        authorization.expires_at_epoch_ms,
        pending.expires_at_epoch_ms
    );
    assert_eq!(
        authorization.expected_task_version,
        pending.expected_task_version
    );
}

fn assert_exact_retirement_delete_binding(
    binding: &TaskRetirementDeleteBindingObservation,
    pending: &TaskRetirementPendingObservation,
) {
    assert_eq!(binding.task_id, pending.task_id);
    assert_eq!(binding.invocation_id, pending.receipt_key.invocation_id);
    assert_eq!(binding.receipt_key_digest, pending.receipt_key.key_digest);
    assert_eq!(binding.task_link_digest, pending.task_link_digest);
    assert_eq!(binding.terminal_digest, pending.terminal_digest);
    assert_eq!(binding.terminal_epoch_ms, pending.terminal_epoch_ms);
    assert_eq!(binding.ttl_ms, pending.ttl_ms);
    assert_eq!(binding.expires_at_epoch_ms, pending.expires_at_epoch_ms);
    assert_eq!(binding.task_version, pending.expected_task_version);
}

fn assert_snapshot_accounting(snapshot: &Snapshot) {
    assert_eq!(snapshot.receipt_live_count, snapshot.receipts.len() as u64);
    let actual: u64 = snapshot
        .receipts
        .iter()
        .map(|receipt| receipt.encoded_bytes)
        .sum();
    let reserved: u64 = snapshot
        .receipts
        .iter()
        .map(|receipt| receipt.reserved_result_bytes)
        .sum();
    assert_eq!(snapshot.receipt_actual_bytes, actual);
    assert_eq!(snapshot.receipt_reserved_bytes, reserved);
    assert!(actual + reserved <= LIVE_RECEIPT_BYTES_LIMIT);
    for receipt in &snapshot.receipts {
        assert_receipt_key(&receipt.key);
        assert!(receipt.version > 0);
        assert!(receipt.encoded_bytes <= MAX_RESPONSE_LINE_BYTES);
        assert!(receipt.reserved_result_bytes <= MAX_RESPONSE_LINE_BYTES);
        match receipt.state {
            SeedReceiptState::CancelReserved => {
                assert!(receipt.encoded_bytes <= 1_024);
                assert_eq!(receipt.reserved_result_bytes, 0);
            }
            SeedReceiptState::TaskBoundNotBegun
            | SeedReceiptState::TaskBoundBegun
            | SeedReceiptState::TaskTerminalBound => {
                panic!("materialized bound lifecycle state belongs only to the TaskLink pool")
            }
            SeedReceiptState::AcknowledgedTombstone => {
                panic!("acknowledged evidence belongs in the tombstone pool")
            }
            SeedReceiptState::ReservedUnbound
            | SeedReceiptState::ReservedActorBound
            | SeedReceiptState::ReservedBegun
            | SeedReceiptState::DirectTerminalUnacked
            | SeedReceiptState::TaskPromisedUnbound
            | SeedReceiptState::TaskPromisedActorBound
            | SeedReceiptState::TaskHandoffActorBoundNotBegun
            | SeedReceiptState::TaskHandoffActorBoundBegun
            | SeedReceiptState::TaskReceiptOwnedActorBound
            | SeedReceiptState::TaskTerminalReceiptBacked => assert_eq!(
                receipt.encoded_bytes + receipt.reserved_result_bytes,
                MAX_RESPONSE_LINE_BYTES
            ),
        }
        if let Some(identity) = &receipt.bound_workspace_identity {
            assert_safe_fingerprint(identity);
        }
        if let Some(staged) = &receipt.staged_terminal {
            assert!(matches!(
                receipt.state,
                SeedReceiptState::TaskHandoffActorBoundNotBegun
                    | SeedReceiptState::TaskHandoffActorBoundBegun
            ));
            assert!(receipt.terminal.is_none());
            assert_terminal(staged);
        } else if !matches!(
            receipt.state,
            SeedReceiptState::TaskHandoffActorBoundNotBegun
                | SeedReceiptState::TaskHandoffActorBoundBegun
        ) {
            assert!(receipt.staged_terminal.is_none());
        }
        if let Some(terminal) = &receipt.terminal {
            assert_terminal(terminal);
            if matches!(
                receipt.state,
                SeedReceiptState::DirectTerminalUnacked
                    | SeedReceiptState::TaskTerminalReceiptBacked
            ) {
                assert_eq!(
                    receipt.expires_epoch_ms,
                    Some(terminal_epoch_ms(terminal) + DIRECT_TASK_TTL_MS)
                );
            }
        }
    }
    assert_eq!(snapshot.tombstone_count, snapshot.tombstones.len() as u64);
    let tombstone_bytes: u64 = snapshot
        .tombstones
        .iter()
        .map(|tombstone| {
            assert_receipt_key(&tombstone.key);
            assert_safe_fingerprint(&tombstone.terminal_digest);
            assert!(tombstone.encoded_bytes <= 512);
            assert_eq!(
                tombstone.expires_epoch_ms,
                tombstone.ack_epoch_ms + TOMBSTONE_TTL_MS
            );
            tombstone.encoded_bytes
        })
        .sum();
    assert_eq!(snapshot.tombstone_bytes, tombstone_bytes);
    assert!(tombstone_bytes <= TOMBSTONE_BYTES_LIMIT);
    assert_eq!(snapshot.task_link_count, snapshot.task_links.len() as u64);
    let task_link_bytes: u64 = snapshot
        .task_links
        .iter()
        .map(|link| {
            assert_receipt_key(&link.key);
            assert_canonical_uuid_v4(&link.task_id);
            assert_canonical_uuid_v4(&link.invocation_id);
            assert_eq!(link.task_id, link.key.reserved_task_id);
            assert_eq!(link.invocation_id, link.key.invocation_id);
            assert_safe_fingerprint(&link.workspace_identity_hash);
            assert_safe_fingerprint(&link.link_digest);
            assert!(link.version > 0);
            assert_eq!(
                link.link_digest,
                task_link_digest_for_test(
                    &link.key.key_digest,
                    &link.task_id,
                    &link.invocation_id,
                    &link.workspace_identity_hash,
                )
            );
            assert!(link.encoded_bytes <= 1_024);
            match &link.lifecycle {
                TaskLinkLifecycleObservation::TaskBoundNotBegun { task_version, .. }
                | TaskLinkLifecycleObservation::TaskBoundBegun { task_version, .. } => {
                    assert!(*task_version > 0)
                }
                TaskLinkLifecycleObservation::TaskTerminalBound {
                    terminal_digest,
                    terminal_epoch_ms,
                    ttl_ms,
                    expires_at_epoch_ms,
                    task_version,
                } => {
                    assert_safe_fingerprint(terminal_digest);
                    assert_eq!(*ttl_ms, DIRECT_TASK_TTL_MS);
                    assert_eq!(*expires_at_epoch_ms, *terminal_epoch_ms + *ttl_ms);
                    assert!(*task_version > 0);
                }
                TaskLinkLifecycleObservation::TaskRetirementPending { pending } => {
                    assert_eq!(pending.receipt_key, link.key);
                    assert_eq!(pending.task_id, link.task_id);
                    assert_eq!(pending.task_link_digest, link.link_digest);
                    assert_task_retirement_pending(pending, snapshot);
                }
            }
            link.encoded_bytes
        })
        .sum();
    assert_eq!(snapshot.task_link_bytes, task_link_bytes);
    assert!(snapshot.task_link_reserved_count <= TASK_LINK_LIMIT);
    assert_eq!(
        snapshot.task_link_reserved_bytes,
        snapshot.task_link_reserved_count * 1_024
    );
    assert!(snapshot.task_link_count + snapshot.task_link_reserved_count <= TASK_LINK_LIMIT);
    assert!(task_link_bytes + snapshot.task_link_reserved_bytes <= TASK_LINK_BYTES_LIMIT);
    assert!(
        snapshot.tasks.len() as u64 <= snapshot.task_link_count + snapshot.task_link_reserved_count,
        "TaskStore records cannot outnumber materialized lifecycle links plus live reservations"
    );
    let unlinked_tasks = snapshot
        .tasks
        .iter()
        .filter(|task| {
            !snapshot
                .task_links
                .iter()
                .any(|link| link.key == task.receipt_key)
        })
        .count() as u64;
    assert!(
        unlinked_tasks <= snapshot.task_link_reserved_count,
        "every provisional TaskStore record needs a retained live link reservation"
    );
    for task in &snapshot.tasks {
        assert!(
            snapshot
                .task_links
                .iter()
                .filter(|link| link.key == task.receipt_key)
                .count()
                <= 1,
            "a TaskStore record cannot be owned by multiple lifecycle links"
        );
    }
    if snapshot.task_link_reserved_count == 0 {
        assert_eq!(snapshot.task_link_reserved_bytes, 0);
    } else {
        assert!(snapshot.task_link_reserved_bytes > 0);
    }
    let mut expected_keys: Vec<_> = snapshot
        .receipts
        .iter()
        .map(|receipt| receipt.key.clone())
        .chain(
            snapshot
                .tombstones
                .iter()
                .map(|tombstone| tombstone.key.clone()),
        )
        .chain(snapshot.task_links.iter().map(|link| link.key.clone()))
        .collect();
    expected_keys.sort_by(|left, right| left.key_digest.cmp(&right.key_digest));
    expected_keys.dedup_by(|left, right| left.key_digest == right.key_digest);
    let mut invocation_index = snapshot.invocation_index.clone();
    invocation_index.iter().for_each(assert_receipt_key);
    invocation_index.sort_by(|left, right| left.key_digest.cmp(&right.key_digest));
    let mut reserved_task_index = snapshot.reserved_task_index.clone();
    reserved_task_index.iter().for_each(assert_receipt_key);
    reserved_task_index.sort_by(|left, right| left.key_digest.cmp(&right.key_digest));
    assert_eq!(invocation_index, expected_keys);
    assert_eq!(reserved_task_index, expected_keys);
    let mut invocation_ids: Vec<_> = snapshot
        .invocation_index
        .iter()
        .map(|key| key.invocation_id.as_str())
        .collect();
    invocation_ids.sort_unstable();
    invocation_ids.dedup();
    assert_eq!(invocation_ids.len(), snapshot.invocation_index.len());
    let mut reserved_task_ids: Vec<_> = snapshot
        .reserved_task_index
        .iter()
        .map(|key| key.reserved_task_id.as_str())
        .collect();
    reserved_task_ids.sort_unstable();
    reserved_task_ids.dedup();
    assert_eq!(reserved_task_ids.len(), snapshot.reserved_task_index.len());
    snapshot.tasks.iter().for_each(assert_task_observation);
}

fn assert_exact_linked_task_pool(snapshot: &Snapshot, expected_count: u64) {
    assert_eq!(snapshot.task_link_count, expected_count);
    assert_eq!(snapshot.task_links.len() as u64, expected_count);
    assert_eq!(snapshot.tasks.len() as u64, expected_count);
    assert_eq!(snapshot.task_link_reserved_count, 0);
    for link in &snapshot.task_links {
        let matching_tasks: Vec<_> = snapshot
            .tasks
            .iter()
            .filter(|task| task.receipt_key == link.key)
            .collect();
        assert_eq!(matching_tasks.len(), 1);
        let task = matching_tasks[0];
        assert_eq!(task.task_id, link.task_id);
        assert_eq!(task.invocation_id, link.invocation_id);
        assert_eq!(
            task.workspace_identity_hash.as_deref(),
            Some(link.workspace_identity_hash.as_str())
        );
    }
    for task in &snapshot.tasks {
        assert_eq!(
            snapshot
                .task_links
                .iter()
                .filter(|link| link.key == task.receipt_key)
                .count(),
            1,
            "every TaskStore record must be injectively owned by one sole lifecycle link"
        );
    }
}

fn assert_report_raw_bounds(report: &ScenarioReport) {
    assert!(report.checkpoints.len() <= 256);
    assert!(report.responses.len() <= 256);
    assert!(report.task_reads.len() <= 256);
    assert!(report.events.len() <= 100_000);
    for record in &report.events {
        assert!(record.sequence > 0);
        assert!(record.epoch_ms > 0);
        let _ = record.monotonic_ms;
    }
    for event in &report.gate_events {
        assert!(event.sequence > 0);
        assert!(!event.operation_label.is_empty() && event.operation_label.len() <= 128);
        match event.transition {
            GateTransition::Waiting | GateTransition::Acquired | GateTransition::Released => {}
        }
    }
    for event in &report.operation_events {
        assert!(event.sequence > 0);
        assert!(!event.label.is_empty() && event.label.len() <= 128);
        match event.state {
            OperationEventState::Spawned
            | OperationEventState::Blocked
            | OperationEventState::Completed
            | OperationEventState::Joined => {}
        }
    }
    for binding in &report.actor_bindings {
        assert_actor_binding(binding);
    }
    for authorization in &report.actor_authorizations {
        assert_actor_authorization(authorization);
    }
    for capacity in &report.task_publication_capacity {
        let _ = observed_task_publication_capacity(report, &capacity.operation_label);
    }
    for violation in &report.task_store_capacity_invariant_violations {
        let _ =
            observed_task_store_capacity_invariant_violation(report, &violation.operation_label);
    }
    for preparation in &report.staged_terminal_preparations {
        assert_staged_terminal_preparation(preparation);
    }
    for case in &report.task_retirement_cases {
        assert_task_retirement_case(case);
    }
    for publication in &report.terminal_publications {
        assert_terminal(&publication.terminal);
        assert_terminal_preflight(publication);
        if let TerminalCommitPreflightObservation::StagedHandoffTask { task, .. } =
            &publication.commit
        {
            let matching: Vec<_> = report
                .staged_terminal_preparations
                .iter()
                .filter(|preparation| {
                    preparation.receipt_key == publication.receipt_key
                        && preparation.terminal == publication.terminal
                })
                .collect();
            assert_eq!(matching.len(), 1);
            let preparation = matching[0];
            assert_eq!(task.task_link_digest, preparation.task_link_digest);
            assert_eq!(
                task.staged_receipt_version,
                preparation.committed_receipt_version
            );
            assert_eq!(
                task.staged_receipt_record_sha256,
                preparation.staged_receipt_record.sha256
            );
            assert_eq!(
                task.staged_terminal_digest,
                terminal_digest(&preparation.terminal)
            );
            assert_eq!(
                task.transfer_size_certificate_sha256,
                preparation.transfer_size_certificate.certificate.sha256
            );
            assert!(
                task.task_record.encoded_bytes
                    <= preparation
                        .transfer_size_certificate
                        .cases
                        .iter()
                        .map(|case| case.artifacts().0.encoded_bytes)
                        .max()
                        .expect("certificate has task candidates")
            );
            assert!(
                task.lifecycle_link_record.encoded_bytes
                    <= preparation
                        .transfer_size_certificate
                        .terminal_bound_link_record
                        .encoded_bytes
            );
            let certified_frame_max = preparation
                .transfer_size_certificate
                .cases
                .iter()
                .map(|case| case.artifacts().1.encoded_bytes)
                .max()
                .expect("certificate has response candidates");
            for frame in &publication.response_frames {
                assert!(frame.response_jsonl.encoded_bytes <= certified_frame_max);
            }
        }
    }
    for protocol in &report.protocol {
        decode_hex(&protocol.spawned_argv_hex, 4_096);
        decode_hex(
            &protocol.client_write_frame_hex,
            MAX_PROTOCOL_FRAME_BYTES + 1,
        );
        if let Some(frame) = &protocol.server_read_frame_hex {
            decode_hex(frame, MAX_PROTOCOL_FRAME_BYTES);
        }
        decode_hex(&protocol.server_write_frame_hex, MAX_PROTOCOL_FRAME_BYTES);
        decode_hex(&protocol.client_read_frame_hex, MAX_PROTOCOL_FRAME_BYTES);
        assert_safe_fingerprint(&protocol.state_fingerprint);
        assert_safe_fingerprint(&protocol.production_v5_core_identity_digest);
        if let Some(presented) = &protocol.presented_core_identity_digest {
            assert_safe_fingerprint(presented);
        }
        if let Some(service) = &protocol.service_capability_fingerprint {
            assert_safe_fingerprint(service);
        }
        if let Some(delivery) = &protocol.delivery {
            if let Some(snapshot) = &delivery.internal_task_snapshot {
                assert_task_observation(snapshot);
            }
            if let Some(pending) = &delivery.pending_direct_receipt_hex {
                protocol_frame(pending, MAX_PROTOCOL_FRAME_BYTES);
            }
            if let Some(native) = &delivery.native_mcp_projection_hex {
                protocol_frame(native, MAX_PROTOCOL_FRAME_BYTES);
            }
            if let Some(compatibility) = &delivery.compatibility_get_projection_hex {
                protocol_frame(compatibility, MAX_PROTOCOL_FRAME_BYTES);
            }
            if let Some(compatibility) = &delivery.compatibility_result_projection_hex {
                protocol_frame(compatibility, MAX_PROTOCOL_FRAME_BYTES);
            }
            if let Some(result) = &delivery.final_call_tool_result_hex {
                protocol_frame(result, MAX_PROTOCOL_FRAME_BYTES);
            }
            if let Some(error) = &delivery.final_error_data_hex {
                protocol_frame(error, MAX_PROTOCOL_FRAME_BYTES);
            }
            if let Some(publication) = &delivery.task_terminal_publication {
                assert_terminal(&publication.terminal);
                assert_terminal_preflight(publication);
            }
            for event in &delivery.events {
                match event {
                    DeliveryEvent::TerminalPreflighted
                    | DeliveryEvent::PendingDirectReceiptBuilt
                    | DeliveryEvent::NativeProjectionBuilt
                    | DeliveryEvent::CompatibilityGetProjectionBuilt
                    | DeliveryEvent::CompatibilityResultProjectionBuilt
                    | DeliveryEvent::FinalInterfaceValueBuilt
                    | DeliveryEvent::AcknowledgementWritten => {}
                }
            }
        }
        for event in &protocol.production_events {
            match event {
                ProtocolProbeEvent::ClientFrameWritten
                | ProtocolProbeEvent::ServerFrameRead
                | ProtocolProbeEvent::ServerFrameWritten
                | ProtocolProbeEvent::ClientFrameRead
                | ProtocolProbeEvent::NegotiationRejected => {}
            }
        }
        for event in &protocol.daemon_process_events {
            match event {
                DaemonProcessEvent::Spawned
                | DaemonProcessEvent::InterfacesDaemonEntrypointEntered
                | DaemonProcessEvent::DefaultV3CompositionSelected
                | DaemonProcessEvent::VersionedV5DispatchSelected
                | DaemonProcessEvent::V3HandshakeCompleted
                | DaemonProcessEvent::V5HandshakeCompleted
                | DaemonProcessEvent::ProtocolFrameHandled
                | DaemonProcessEvent::CanonicalV13ServiceEntered => {}
            }
        }
    }
    if let Some(identity) = &report.identity {
        assert_receipt_key(&identity.client_key);
        assert_receipt_key(&identity.daemon_key);
        assert_receipt_key(&identity.frozen_vector_key);
        assert_safe_fingerprint(&identity.caller_claimed_key_digest);
    }
}

fn assert_task_retirement_case(case: &TaskRetirementCaseObservation) {
    assert_snapshot_accounting(&case.before);
    assert_snapshot_accounting(&case.after_crash);
    assert_snapshot_accounting(&case.after_recovery);
    assert_eq!(
        case.lazy_task_delete_attempts, 0,
        "v5 has no lazy Task delete"
    );
    let mut prior = 0;
    for event in &case.retirement_events {
        assert!(event.sequence > prior);
        prior = event.sequence;
    }
    let position = |needle: TaskRetirementEvent| {
        case.retirement_events
            .iter()
            .position(|event| event.event == needle)
    };
    if case.case == TaskRetirementWorkload::ActiveTaskBoundAbsent {
        assert!(case.committed_pending.is_none());
        assert!(case.initial_authorization.is_none());
        assert!(case.recovered_authorization.is_none());
        assert!(case.old_authorization_reuse.is_none());
    } else {
        let pending = case
            .committed_pending
            .as_ref()
            .expect("every terminal retirement case must expose the committed Pending record");
        assert_task_retirement_pending(pending, &case.before);
        let before_task = only_task(&case.before);
        let before_terminal = before_task
            .terminal
            .as_ref()
            .expect("retirement source Task must be terminal");
        assert_eq!(pending.receipt_key, before_task.receipt_key);
        assert_eq!(pending.task_id, before_task.task_id);
        assert_eq!(pending.terminal_digest, terminal_digest(before_terminal));
        assert_eq!(
            pending.terminal_epoch_ms,
            terminal_epoch_ms(before_terminal)
        );
        assert_eq!(pending.ttl_ms, before_task.ttl_ms);
        assert_eq!(pending.expires_at_epoch_ms, before_task.expires_epoch_ms);
        assert_eq!(pending.expected_task_version, before_task.version);
        assert_eq!(
            pending.task_link_digest,
            only_task_link(&case.before).link_digest
        );
        let recovered = case
            .recovered_authorization
            .as_ref()
            .expect("reopen must freshly authorize the exact committed Pending readback");
        assert_task_retirement_authorization(recovered, pending);
        assert!(
            position(TaskRetirementEvent::ExistingPendingAuthorized).is_some(),
            "reopen must authorize an existing exact Pending before delete"
        );
        if let Some(initial) = &case.initial_authorization {
            assert_task_retirement_authorization(initial, pending);
            assert_ne!(initial.process_instance_id, recovered.process_instance_id);
            assert_ne!(
                initial.authorization_fingerprint,
                recovered.authorization_fingerprint
            );
            assert!(initial.generation < recovered.generation);
            let reuse = case
                .old_authorization_reuse
                .as_ref()
                .expect("crash recovery must reject the old process capability");
            assert_eq!(
                reuse.presented_fingerprint,
                initial.authorization_fingerprint
            );
            assert_eq!(
                reuse.reason,
                TaskRetirementAuthorizationRejection::StaleProcessCapability
            );
            assert_eq!(
                reuse.receipt_store_mutations_before,
                reuse.receipt_store_mutations_after
            );
            assert_eq!(
                reuse.task_store_mutations_before,
                reuse.task_store_mutations_after
            );
            assert!(reuse.rejected_sequence > recovered.issued_sequence);
        } else {
            assert!(case.old_authorization_reuse.is_none());
        }
    }

    match case.case {
        TaskRetirementWorkload::ActiveTaskBoundAbsent => {
            assert_eq!(case.task_store_delete_attempts, 0);
            assert!(retirement_pendings(&case.before).is_empty());
            assert!(retirement_pendings(&case.after_crash).is_empty());
            assert!(retirement_pendings(&case.after_recovery).is_empty());
            assert_eq!(case.before.receipt_live_count, 0);
            assert_eq!(case.before.receipt_actual_bytes, 0);
            assert_eq!(case.before.receipt_reserved_bytes, 0);
            assert_eq!(case.before.task_links, case.after_recovery.task_links);
            assert_eq!(case.before.task_links, case.after_crash.task_links);
            assert_eq!(case.before.receipts, case.after_recovery.receipts);
            assert_eq!(case.before.receipts, case.after_crash.receipts);
            assert_eq!(
                case.before.invocation_index,
                case.after_crash.invocation_index
            );
            assert_eq!(
                case.before.invocation_index,
                case.after_recovery.invocation_index
            );
            assert_eq!(
                case.before.reserved_task_index,
                case.after_crash.reserved_task_index
            );
            assert_eq!(
                case.before.reserved_task_index,
                case.after_recovery.reserved_task_index
            );
            assert_eq!(
                case.before.task_link_bytes,
                case.after_crash.task_link_bytes
            );
            assert_eq!(
                case.before.task_link_bytes,
                case.after_recovery.task_link_bytes
            );
            assert_eq!(
                case.before.store_generation,
                case.after_crash.store_generation
            );
            assert_eq!(
                case.before.store_generation,
                case.after_recovery.store_generation
            );
            assert_eq!(case.retirement_events.len(), 1);
            assert_eq!(
                case.retirement_events[0].event,
                TaskRetirementEvent::ActiveTaskBoundTaskMissingCorruption
            );
            assert!(case.before.receipts.is_empty());
            assert_eq!(case.before.task_links.len(), 1);
            assert!(matches!(
                &only_task_link(&case.before).lifecycle,
                TaskLinkLifecycleObservation::TaskBoundBegun { .. }
            ));
            assert_eq!(case.before.invocation_index.len(), 1);
            assert_eq!(case.before.reserved_task_index.len(), 1);
            assert!(case.before.tasks.is_empty());
            assert_eq!(case.after_recovery.listener, ListenerState::Closed);
            assert!(case.after_recovery.restart_requested);
            assert!(!case.after_recovery.daemon_running);
            assert_eq!(
                case.after_recovery.task_store_mutations,
                case.before.task_store_mutations
            );
            assert_eq!(
                case.after_recovery.receipt_store_mutations,
                case.before.receipt_store_mutations
            );
        }
        TaskRetirementWorkload::RecoveryTerminalBeforeTerminalBound => {
            assert!(position(TaskRetirementEvent::ExactTerminalReadback).is_some());
            assert!(position(TaskRetirementEvent::TerminalBoundCommitted).is_some());
            assert!(
                position(TaskRetirementEvent::ExactTerminalReadback)
                    < position(TaskRetirementEvent::TerminalBoundCommitted)
            );
            assert!(
                position(TaskRetirementEvent::TerminalBoundCommitted)
                    < position(TaskRetirementEvent::PendingCommitted)
            );
            assert!(case.after_recovery.tasks.is_empty());
            assert!(case.after_recovery.task_links.is_empty());
            assert!(retirement_pendings(&case.after_recovery).is_empty());
        }
        TaskRetirementWorkload::BeforePendingIntent
        | TaskRetirementWorkload::AfterPendingIntentBeforeDelete
        | TaskRetirementWorkload::AfterDeletedBeforeLedgerFinalize
        | TaskRetirementWorkload::AfterAbsentConfirmedBeforeLedgerFinalize => {
            assert_eq!(case.task_store_delete_attempts, 1);
            let pending = position(TaskRetirementEvent::PendingCommitted)
                .expect("successful retirement must first commit pending intent");
            let attempted = position(TaskRetirementEvent::DeleteAttempted)
                .expect("pending retirement must drive one exact delete");
            let authorized = position(TaskRetirementEvent::ExistingPendingAuthorized)
                .expect("reopen must authorize the exact Pending before delete");
            let finalized = position(TaskRetirementEvent::PendingFinalized)
                .expect("successful retirement must atomically remove pending/link/indexes");
            assert!(pending < authorized && authorized < attempted && attempted < finalized);
            assert!(case.after_recovery.tasks.is_empty());
            assert!(case.after_recovery.task_links.is_empty());
            assert!(retirement_pendings(&case.after_recovery).is_empty());
            assert!(case.after_recovery.invocation_index.is_empty());
            assert!(case.after_recovery.reserved_task_index.is_empty());
        }
        TaskRetirementWorkload::AfterDeleteCommitUncertain
        | TaskRetirementWorkload::DeleteIdentityMismatch => {
            assert_eq!(case.task_store_delete_attempts, 1);
            assert_eq!(retirement_pendings(&case.after_recovery).len(), 1);
            assert_eq!(case.after_recovery.task_links.len(), 1);
            assert_eq!(case.after_recovery.listener, ListenerState::Closed);
            assert!(case.after_recovery.restart_requested);
            assert!(!case.after_recovery.daemon_running);
            assert!(position(TaskRetirementEvent::PendingFinalized).is_none());
            assert!(
                position(TaskRetirementEvent::ExistingPendingAuthorized)
                    < position(TaskRetirementEvent::DeleteAttempted)
            );
        }
    }

    match &case.delete_outcome {
        TaskRetirementDeleteOutcome::Deleted {
            deleted_task_record,
            pending_authorization_fingerprint,
            binding,
        } => {
            let deleted = decode_v5_stored_record(deleted_task_record);
            let pending = case
                .committed_pending
                .as_ref()
                .expect("Deleted proof must bind committed Pending");
            assert_eq!(
                pending_authorization_fingerprint,
                &case
                    .recovered_authorization
                    .as_ref()
                    .expect("delete proof uses current process authorization")
                    .authorization_fingerprint
            );
            assert_eq!(deleted.task_id, pending.task_id);
            assert_eq!(deleted.invocation_id, pending.receipt_key.invocation_id);
            assert_eq!(deleted.receipt_key_digest, pending.receipt_key.key_digest);
            assert_eq!(deleted.version, pending.expected_task_version);
            assert_eq!(deleted.ttl_ms, pending.ttl_ms);
            assert_exact_retirement_delete_binding(binding, pending);
            assert!(matches!(
                &deleted.task,
                V5StoredTaskObservation::Completed {
                    terminal_epoch_ms,
                    terminal_digest,
                    ..
                } | V5StoredTaskObservation::Failed {
                    terminal_epoch_ms,
                    terminal_digest,
                    ..
                } | V5StoredTaskObservation::Cancelled {
                    terminal_epoch_ms,
                    terminal_digest,
                } if *terminal_epoch_ms == pending.terminal_epoch_ms
                    && terminal_digest == &pending.terminal_digest
            ));
            assert!(position(TaskRetirementEvent::DeleteCommitted).is_some());
        }
        TaskRetirementDeleteOutcome::AbsentExactWithPending {
            pending_authorization_fingerprint,
            binding,
        } => {
            assert_safe_fingerprint(pending_authorization_fingerprint);
            assert_eq!(
                pending_authorization_fingerprint,
                &case
                    .recovered_authorization
                    .as_ref()
                    .expect("Absent proof must use current process authorization")
                    .authorization_fingerprint
            );
            let pending = case
                .committed_pending
                .as_ref()
                .expect("Absent proof must bind the exact committed Pending");
            assert_exact_retirement_delete_binding(binding, pending);
            assert!(position(TaskRetirementEvent::AbsentWithPendingConfirmed).is_some());
        }
        TaskRetirementDeleteOutcome::CommitUncertain {
            task_store_generation_before,
            task_store_generation_after,
        } => {
            assert_eq!(task_store_generation_before, task_store_generation_after);
            assert!(position(TaskRetirementEvent::DeleteCommitUncertain).is_some());
        }
        TaskRetirementDeleteOutcome::IdentityMismatch {
            observed_task_record,
            observed_binding,
        } => {
            let observed = decode_v5_stored_record(observed_task_record);
            let pending = case
                .committed_pending
                .as_ref()
                .expect("identity mismatch retains committed Pending evidence");
            assert_eq!(observed.task_id, observed_binding.task_id);
            assert_eq!(observed.invocation_id, observed_binding.invocation_id);
            assert_eq!(
                observed.receipt_key_digest,
                observed_binding.receipt_key_digest
            );
            assert_eq!(observed.version, observed_binding.task_version);
            assert_eq!(observed.ttl_ms, observed_binding.ttl_ms);
            assert_eq!(
                observed_binding.expires_at_epoch_ms,
                observed_binding.terminal_epoch_ms + observed_binding.ttl_ms
            );
            assert_safe_fingerprint(&observed_binding.task_link_digest);
            assert_safe_fingerprint(&observed_binding.terminal_digest);
            assert!(
                observed_binding.task_id != pending.task_id
                    || observed_binding.invocation_id != pending.receipt_key.invocation_id
                    || observed_binding.receipt_key_digest != pending.receipt_key.key_digest
                    || observed_binding.task_link_digest != pending.task_link_digest
                    || observed_binding.terminal_digest != pending.terminal_digest
                    || observed_binding.terminal_epoch_ms != pending.terminal_epoch_ms
                    || observed_binding.ttl_ms != pending.ttl_ms
                    || observed_binding.expires_at_epoch_ms != pending.expires_at_epoch_ms
                    || observed_binding.task_version != pending.expected_task_version,
                "mismatch fixture must differ in a bound identity/version field"
            );
            assert!(position(TaskRetirementEvent::DeleteIdentityMismatch).is_some());
        }
        TaskRetirementDeleteOutcome::NotAttemptedActiveTaskMissing {
            receipt_store_generation_before,
            receipt_store_generation_after,
            task_store_generation_before,
            task_store_generation_after,
        } => {
            assert_eq!(case.case, TaskRetirementWorkload::ActiveTaskBoundAbsent);
            assert_eq!(
                receipt_store_generation_before,
                receipt_store_generation_after
            );
            assert_eq!(task_store_generation_before, task_store_generation_after);
            assert_eq!(case.task_store_delete_attempts, 0);
        }
    }
}

fn assert_actor_binding(observed: &ActorBindingObservation) {
    assert!(!observed.operation_label.is_empty() && observed.operation_label.len() <= 128);
    assert_receipt_key(&observed.receipt_key);
    assert_safe_fingerprint(&observed.actor_identity_hash);
    assert!(observed.actor_generation > 0);
    for token in [
        &observed.binding_claim_fingerprint,
        &observed.binding_token_fingerprint,
    ] {
        assert_safe_fingerprint(token);
    }
    assert_ne!(
        observed.binding_claim_fingerprint,
        observed.binding_token_fingerprint
    );
    assert!(observed.claim_verified_sequence < observed.actor_bound_sequence);
    assert!(observed.actor_bound_sequence < observed.binding_token_minted_sequence);
    assert!(observed.actor_bound_expected_receipt_version > 0);
    assert_eq!(
        observed.actor_bound_committed_receipt_version,
        observed.actor_bound_expected_receipt_version + 1
    );
    if let Some(consumption) = &observed.binding_token_consumption {
        match consumption {
            BindingTokenConsumptionObservation::MarkReservedBegun {
                consumed_sequence,
                receipt_expected_version,
                receipt_committed_version,
            } => {
                assert!(observed.binding_token_minted_sequence < *consumed_sequence);
                assert!(
                    *receipt_expected_version >= observed.actor_bound_committed_receipt_version
                );
                assert_eq!(*receipt_committed_version, *receipt_expected_version + 1);
                assert!(observed.task_binding.is_none());
            }
            BindingTokenConsumptionObservation::AuthorizeBoundTaskStart {
                consumed_sequence,
                lifecycle_link_expected_version,
                lifecycle_link_committed_version,
            } => {
                assert!(observed.binding_token_minted_sequence < *consumed_sequence);
                let binding = observed.task_binding.as_ref().expect(
                    "bound start token consumption requires the sole lifecycle-link binding",
                );
                assert_eq!(
                    *lifecycle_link_expected_version,
                    binding.task_bound_committed_lifecycle_link_version
                );
                assert_eq!(
                    *lifecycle_link_committed_version,
                    *lifecycle_link_expected_version + 1
                );
            }
        }
    }
    if let Some(task_binding) = &observed.task_binding {
        for token in [
            &task_binding.task_link_reservation_fingerprint,
            &task_binding.task_link_digest,
            &task_binding.task_bound_link_authorization_fingerprint,
        ] {
            assert_safe_fingerprint(token);
        }
        assert_ne!(
            observed.binding_token_fingerprint,
            task_binding.task_link_reservation_fingerprint
        );
        assert_ne!(
            task_binding.task_link_reservation_fingerprint,
            task_binding.task_bound_link_authorization_fingerprint
        );
        assert_eq!(
            task_binding.task_link_digest,
            task_link_digest_for_test(
                &observed.receipt_key.key_digest,
                &observed.receipt_key.reserved_task_id,
                &observed.receipt_key.invocation_id,
                &observed.actor_identity_hash,
            ),
            "Task-link identity must use the actor-derived workspace identity and exclude mutable Task fields"
        );
        assert!(observed.actor_bound_sequence < task_binding.task_link_reserved_sequence);
        assert!(task_binding.task_link_reserved_sequence < task_binding.task_store_create_sequence);
        assert!(
            task_binding.task_store_create_sequence
                < task_binding.task_link_reservation_consumed_sequence
        );
        assert!(
            task_binding.task_link_reservation_consumed_sequence < task_binding.task_bound_sequence
        );
        assert!(
            task_binding.task_bound_sequence
                < task_binding.task_bound_link_authorization_minted_sequence
        );
        assert!(task_binding.task_bound_committed_lifecycle_link_version > 0);
    }
}

fn assert_actor_authorization(observed: &ActorAuthorizationObservation) {
    assert!(!observed.operation_label.is_empty() && observed.operation_label.len() <= 128);
    assert_eq!(
        observed.verifier,
        ActorVerifier::InfrastructureLeaseRegistry
    );
    assert_eq!(
        observed.ledger_authorization.issuer,
        ActorAuthorizationIssuer::ReceiptLedger
    );
    assert_receipt_key(&observed.ledger_authorization.receipt_key);
    assert_safe_fingerprint(&observed.ledger_authorization.authorization_fingerprint);
    assert!(observed.ledger_authorization.generation > 0);
    assert!(observed.verifier_generation >= observed.ledger_authorization.generation);
    if let Some(presented) = &observed.presented_authorization {
        assert_safe_fingerprint(&presented.authorization_fingerprint);
        assert!(presented.generation > 0);
    }
    match observed.purpose {
        ActorAuthorizationPurpose::ReservedBegin => {
            assert!(observed.task_bound_context.is_none());
            assert!(observed.post_working_authorization.is_none());
        }
        ActorAuthorizationPurpose::BoundTaskStart => {
            let context = observed
                .task_bound_context
                .as_ref()
                .expect("bound Task authorization needs exact post-bind context");
            assert_receipt_key(&context.receipt_key);
            assert_canonical_uuid_v4(&context.task_id);
            assert_eq!(context.task_id, context.receipt_key.reserved_task_id);
            assert_safe_fingerprint(&context.task_link_digest);
            assert!(context.task_version > 0);
            assert!(context.lifecycle_link_version > 0);
            assert!(context.actor_generation > 0);
            assert_safe_fingerprint(&context.consumed_binding_token_fingerprint);
            assert_safe_fingerprint(&context.consumed_task_link_reservation_fingerprint);
            assert_safe_fingerprint(&context.task_bound_link_authorization_fingerprint);
            assert_ne!(
                context.consumed_binding_token_fingerprint,
                context.consumed_task_link_reservation_fingerprint
            );
            assert_ne!(
                context.consumed_task_link_reservation_fingerprint,
                context.task_bound_link_authorization_fingerprint
            );
            if let Some(authorization) = &observed.post_working_authorization {
                assert_safe_fingerprint(&authorization.authorization_fingerprint);
                assert_ne!(
                    authorization.authorization_fingerprint,
                    context.consumed_binding_token_fingerprint
                );
                assert_ne!(
                    authorization.authorization_fingerprint,
                    context.consumed_task_link_reservation_fingerprint
                );
                assert_eq!(authorization.receipt_key, context.receipt_key);
                assert_eq!(authorization.task_id, context.task_id);
                assert_eq!(authorization.task_link_digest, context.task_link_digest);
                assert_eq!(authorization.expected_task_version, context.task_version);
                assert_eq!(authorization.actor_generation, context.actor_generation);
                assert_eq!(
                    authorization.task_bound_link_authorization_fingerprint,
                    context.task_bound_link_authorization_fingerprint
                );
                assert_eq!(
                    authorization.working_readback_task_link_digest,
                    context.task_link_digest
                );
                assert!(
                    authorization.minted_sequence
                        < authorization.task_bound_link_authorization_consumed_sequence
                );
                assert!(
                    authorization.task_bound_link_authorization_consumed_sequence
                        < authorization.working_write_sequence
                );
                assert!(
                    authorization.working_write_sequence < authorization.working_readback_sequence
                );
                assert!(authorization.working_readback_sequence < authorization.rechecked_sequence);
                match (
                    authorization.consumed_sequence,
                    authorization.mark_begun_expected_lifecycle_link_version,
                    authorization.mark_begun_committed_lifecycle_link_version,
                ) {
                    (Some(consumed), Some(expected_version), Some(committed_version)) => {
                        assert!(consumed > authorization.rechecked_sequence);
                        assert!(expected_version > 0);
                        assert_eq!(committed_version, expected_version + 1);
                    }
                    (None, None, None) => {}
                    other => panic!(
                        "mark-begun lifecycle-link consumption/CAS evidence must be all-or-none: {other:?}"
                    ),
                }
            }
        }
    }
    match observed.decision {
        ActorAuthorizationDecision::Accepted => {
            let presented = observed
                .presented_authorization
                .as_ref()
                .expect("accepted proof needs a presented opaque authorization");
            assert_eq!(
                presented.authorization_fingerprint,
                observed.ledger_authorization.authorization_fingerprint
            );
            assert_eq!(presented.generation, observed.verifier_generation);
            assert_eq!(
                observed.ledger_authorization.generation, observed.verifier_generation,
                "accepted authorization generation must match the live verifier exactly"
            );
            if observed.purpose == ActorAuthorizationPurpose::BoundTaskStart {
                assert!(observed
                    .post_working_authorization
                    .as_ref()
                    .and_then(|authorization| authorization.consumed_sequence)
                    .is_some());
            }
        }
        ActorAuthorizationDecision::Missing => {
            assert!(observed.presented_authorization.is_none());
            assert!(observed.post_working_authorization.is_none());
        }
        ActorAuthorizationDecision::Foreign => {
            let presented = observed
                .presented_authorization
                .as_ref()
                .expect("foreign proof needs raw presented evidence");
            assert_ne!(
                presented.authorization_fingerprint,
                observed.ledger_authorization.authorization_fingerprint
            );
            assert!(observed.post_working_authorization.is_none());
        }
        ActorAuthorizationDecision::Stale => {
            let presented = observed
                .presented_authorization
                .as_ref()
                .expect("stale proof needs raw presented evidence");
            assert_eq!(
                presented.authorization_fingerprint,
                observed.ledger_authorization.authorization_fingerprint
            );
            assert_eq!(
                presented.generation,
                observed.ledger_authorization.generation
            );
            assert!(observed.ledger_authorization.generation < observed.verifier_generation);
            if let Some(authorization) = &observed.post_working_authorization {
                assert!(authorization.consumed_sequence.is_none());
            }
        }
    }
}

fn protocol_frame(frame_hex: &str, maximum_bytes: usize) -> serde_json::Value {
    let frame = decode_hex(frame_hex, maximum_bytes);
    serde_json::from_slice(&frame)
        .unwrap_or_else(|_| panic!("protocol frame must contain one bounded JSON value"))
}

fn protocol_jsonl_frame(frame_hex: &str, maximum_bytes: usize) -> serde_json::Value {
    let frame = decode_hex(frame_hex, maximum_bytes);
    assert_eq!(
        frame.last(),
        Some(&b'\n'),
        "daemon wire frame must end in one LF"
    );
    assert!(
        frame[..frame.len() - 1]
            .iter()
            .all(|byte| !matches!(byte, b'\r' | b'\n')),
        "daemon wire frame must contain exactly one trailing line delimiter"
    );
    serde_json::from_slice(&frame[..frame.len() - 1])
        .unwrap_or_else(|_| panic!("daemon wire frame must contain one exact JSONL value"))
}

fn frame_kind(frame: &serde_json::Value) -> &str {
    frame
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .expect("protocol frame must expose a string kind")
}

fn find_json_field<'a>(value: &'a serde_json::Value, field: &str) -> Option<&'a serde_json::Value> {
    match value {
        serde_json::Value::Object(fields) => fields.get(field).or_else(|| {
            fields
                .values()
                .find_map(|nested| find_json_field(nested, field))
        }),
        serde_json::Value::Array(values) => values
            .iter()
            .find_map(|nested| find_json_field(nested, field)),
        _ => None,
    }
}

fn assert_frame_full_receipt_key(frame: &serde_json::Value) -> V5ReceiptKeyWireObservation {
    let raw = find_json_field(frame, "receiptKey")
        .expect("receipt protocol message must carry the full receiptKey object");
    assert_exact_json_keys(
        raw,
        &[
            "invocationId",
            "reservedTaskId",
            "coreIdentityDigest",
            "tool",
            "normalizedArgumentsHash",
            "requestScopeHash",
        ],
    );
    let key: V5ReceiptKeyWireObservation = serde_json::from_value(raw.clone())
        .unwrap_or_else(|_| panic!("receiptKey frame field must match the closed v5 key schema"));
    assert_canonical_uuid_v4(&key.invocation_id);
    assert_canonical_uuid_v4(&key.reserved_task_id);
    assert_safe_fingerprint(&key.core_identity_digest);
    let _ = tool_wire_name(key.tool);
    assert_safe_fingerprint(&key.normalized_arguments_hash);
    assert_safe_fingerprint(&key.request_scope_hash.0);
    assert_safe_fingerprint(&receipt_key_digest_for_test(
        &key.invocation_id,
        &key.reserved_task_id,
        &key.core_identity_digest,
        tool_wire_name(key.tool),
        &key.normalized_arguments_hash,
        &key.request_scope_hash.0,
    ));
    key
}

fn assert_wire_key_matches_internal(
    wire: &V5ReceiptKeyWireObservation,
    internal: &ReceiptKeyObservation,
) {
    assert_eq!(wire.invocation_id, internal.invocation_id);
    assert_eq!(wire.reserved_task_id, internal.reserved_task_id);
    assert_eq!(wire.core_identity_digest, internal.core_identity_digest);
    assert_eq!(wire.tool, internal.tool);
    assert_eq!(
        wire.normalized_arguments_hash,
        internal.normalized_arguments_hash
    );
    assert_eq!(wire.request_scope_hash, internal.request_scope_hash);
    assert_eq!(
        receipt_key_digest_for_test(
            &wire.invocation_id,
            &wire.reserved_task_id,
            &wire.core_identity_digest,
            tool_wire_name(wire.tool),
            &wire.normalized_arguments_hash,
            &wire.request_scope_hash.0,
        ),
        internal.key_digest
    );
}

fn strict_task_snapshot_fixture() -> serde_json::Value {
    serde_json::json!({
        "status": "queued",
        "taskId": "11111111-1111-4111-8111-111111111111",
        "invocationId": "22222222-2222-4222-8222-222222222222",
        "receiptKeyDigest": "0000000000000000000000000000000000000000000000000000000000000000",
        "createdAtEpochMs": 1,
        "updatedAtEpochMs": 1,
        "ttlMs": DIRECT_TASK_TTL_MS,
        "pollIntervalMs": 100,
        "version": 1,
        "cancelRequested": false,
    })
}

fn strict_stored_record_fixture() -> serde_json::Value {
    serde_json::json!({
        "schemaVersion": 1,
        "taskId": "11111111-1111-4111-8111-111111111111",
        "invocationId": "22222222-2222-4222-8222-222222222222",
        "receiptKeyDigest": "0000000000000000000000000000000000000000000000000000000000000000",
        "tool": "unica.view",
        "normalizedArgumentsHash": "0000000000000000000000000000000000000000000000000000000000000000",
        "workspaceIdentityHash": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "createdAtEpochMs": 1,
        "updatedAtEpochMs": 1,
        "ttlMs": DIRECT_TASK_TTL_MS,
        "pollIntervalMs": 100,
        "version": 1,
        "cancelRequested": false,
        "task": {"status": "queued"},
    })
}

fn strict_transfer_certificate_fixture() -> serde_json::Value {
    serde_json::json!({
        "certificateVersion": 1,
        "protocolIdentity": "v5",
        "coreIdentityDigest": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "receiptKeyDigest": "0000000000000000000000000000000000000000000000000000000000000000",
        "taskId": "11111111-1111-4111-8111-111111111111",
        "invocationId": "22222222-2222-4222-8222-222222222222",
        "taskLinkDigest": "4c73d08219973c72e759a9f85e156fa42c9d8e61a56e704b70d1c7c042b73da0",
        "terminalDigest": "f2d0423d2613a0d09397b750542e4542f7653d78ebd5e0448f1326d09145d9ae",
        "terminalEpochMs": 1,
        "receiptRecordSchemaVersion": 1,
        "taskRecordSchemaVersion": 1,
        "lifecycleLinkRecordSchemaVersion": 1,
        "terminalCodecVersion": 1,
        "maxDaemonResponseLineBytes": MAX_RESPONSE_LINE_BYTES,
        "maxTaskLifecycleLinkRecordBytes": 1_024,
        "stagedReceiptRecordMaxBytes": MAX_RESPONSE_LINE_BYTES,
        "taskTerminalBoundLinkRecordMaxBytes": 1_024,
        "taskPublicationCases": [
            {
                "kind": "absent",
                "finalTaskRecordMaxBytes": MAX_RESPONSE_LINE_BYTES,
                "taskResponseFrameMaxBytes": MAX_RESPONSE_LINE_BYTES,
            },
            {
                "kind": "exact_provisional",
                "status": "queued",
                "version": u64::MAX,
                "cancelRequested": false,
                "finalTaskRecordMaxBytes": MAX_RESPONSE_LINE_BYTES,
                "taskResponseFrameMaxBytes": MAX_RESPONSE_LINE_BYTES,
            },
            {
                "kind": "exact_provisional",
                "status": "queued",
                "version": u64::MAX,
                "cancelRequested": true,
                "finalTaskRecordMaxBytes": MAX_RESPONSE_LINE_BYTES,
                "taskResponseFrameMaxBytes": MAX_RESPONSE_LINE_BYTES,
            },
            {
                "kind": "exact_provisional",
                "status": "working",
                "version": u64::MAX,
                "cancelRequested": false,
                "finalTaskRecordMaxBytes": MAX_RESPONSE_LINE_BYTES,
                "taskResponseFrameMaxBytes": MAX_RESPONSE_LINE_BYTES,
            },
            {
                "kind": "exact_provisional",
                "status": "working",
                "version": u64::MAX,
                "cancelRequested": true,
                "finalTaskRecordMaxBytes": MAX_RESPONSE_LINE_BYTES,
                "taskResponseFrameMaxBytes": MAX_RESPONSE_LINE_BYTES,
            },
        ],
        "capacityFallbackCases": [{
            "source": "link_capacity",
            "receiptBackedRecordMaxBytes": MAX_RESPONSE_LINE_BYTES,
            "taskResponseFrameMaxBytes": MAX_RESPONSE_LINE_BYTES,
        }],
    })
}

fn strict_schema_mutation_fixture(target: StrictSchemaTarget) -> serde_json::Value {
    match target {
        StrictSchemaTarget::RequestUnknownField => {
            serde_json::json!({"kind": "ping", "unexpected": true})
        }
        StrictSchemaTarget::RequestMissingRequiredField => {
            serde_json::json!({"kind": "get_task"})
        }
        StrictSchemaTarget::RequestCrossVariantField => serde_json::json!({
            "kind": "get_task",
            "taskId": "11111111-1111-4111-8111-111111111111",
            "waitMs": 1,
        }),
        StrictSchemaTarget::ResponseUnknownField => {
            serde_json::json!({"kind": "pong", "unexpected": true})
        }
        StrictSchemaTarget::ResponseMissingRequiredField => {
            serde_json::json!({"kind": "task"})
        }
        StrictSchemaTarget::ResponseCrossVariantField => serde_json::json!({
            "kind": "pong",
            "snapshot": strict_task_snapshot_fixture(),
        }),
        StrictSchemaTarget::TerminalUnknownField => serde_json::json!({
            "status": "failed",
            "reason": "invocation_failed",
            "unexpected": true,
        }),
        StrictSchemaTarget::TerminalMissingRequiredField => {
            serde_json::json!({"status": "failed"})
        }
        StrictSchemaTarget::TerminalCrossVariantField => serde_json::json!({
            "status": "failed",
            "reason": "invocation_failed",
            "result": {"ok": false, "summary": "semantic-invalid"},
        }),
        StrictSchemaTarget::TaskSnapshotUnknownField => {
            let mut fixture = strict_task_snapshot_fixture();
            fixture["unexpected"] = true.into();
            fixture
        }
        StrictSchemaTarget::TaskSnapshotMissingRequiredField => {
            let mut fixture = strict_task_snapshot_fixture();
            fixture
                .as_object_mut()
                .expect("strict Task snapshot fixture is an object")
                .remove("taskId");
            fixture
        }
        StrictSchemaTarget::TaskSnapshotCrossVariantField => {
            let mut fixture = strict_task_snapshot_fixture();
            fixture["reason"] = "invocation_failed".into();
            fixture
        }
        StrictSchemaTarget::StoredRecordUnknownTopLevel => {
            let mut fixture = strict_stored_record_fixture();
            fixture["unexpected"] = true.into();
            fixture
        }
        StrictSchemaTarget::StoredRecordUnknownTaskField => {
            let mut fixture = strict_stored_record_fixture();
            fixture["task"]["unexpected"] = true.into();
            fixture
        }
        StrictSchemaTarget::StoredRecordMissingRequiredField => {
            let mut fixture = strict_stored_record_fixture();
            fixture
                .as_object_mut()
                .expect("strict stored record fixture is an object")
                .remove("schemaVersion");
            fixture
        }
        StrictSchemaTarget::StoredRecordCrossVariantField => {
            let mut fixture = strict_stored_record_fixture();
            fixture["task"]["reason"] = "invocation_failed".into();
            fixture
        }
        StrictSchemaTarget::TransferCertificateUnknownField => {
            let mut fixture = strict_transfer_certificate_fixture();
            fixture["unexpected"] = true.into();
            fixture
        }
        StrictSchemaTarget::TransferCertificateMissingRequiredField => {
            let mut fixture = strict_transfer_certificate_fixture();
            fixture
                .as_object_mut()
                .expect("strict transfer certificate fixture is an object")
                .remove("terminalCodecVersion");
            fixture
        }
        StrictSchemaTarget::TransferCertificateCrossVariantField => {
            let mut fixture = strict_transfer_certificate_fixture();
            fixture["taskPublicationCases"][0]["cancelRequested"] = false.into();
            fixture
        }
    }
}

fn assert_strict_schema_mutation(observed: &serde_json::Value, target: StrictSchemaTarget) {
    assert_eq!(
        *observed,
        strict_schema_mutation_fixture(target),
        "raw schema probe must contain the exact test-owned mutation for {target:?}"
    );
}

fn assert_strict_schema_oracle_is_discriminating(strict_cases: &[(&str, StrictSchemaTarget)]) {
    let mut mutations: Vec<_> = strict_cases
        .iter()
        .map(|(_, target)| {
            serde_json::to_string(&strict_schema_mutation_fixture(*target))
                .expect("strict mutation fixture must encode")
        })
        .collect();
    mutations.sort();
    mutations.dedup();
    assert_eq!(
        mutations.len(),
        strict_cases.len(),
        "every StrictSchemaTarget must have a distinct test-owned raw mutation"
    );
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExpectedInvocationResultType {
    ReceiptPending,
    Direct,
    Task,
    Acknowledged,
}

fn assert_invocation_response_frame(
    frame: &serde_json::Value,
    expected: ExpectedInvocationResultType,
) -> &serde_json::Value {
    assert_exact_json_keys(frame, &["kind", "outcome"]);
    assert_eq!(json_string(frame, "kind"), "invocation");
    let outcome = &frame["outcome"];
    match expected {
        ExpectedInvocationResultType::ReceiptPending => {
            assert_exact_json_keys(
                outcome,
                &[
                    "resultType",
                    "receiptKey",
                    "phase",
                    "acceptedEpochMs",
                    "originalBudgetMs",
                    "cancelRequested",
                ],
            );
            assert_eq!(json_string(outcome, "resultType"), "receipt_pending");
            assert!(matches!(
                json_string(outcome, "phase"),
                "cancel_reserved" | "reserved_unbound" | "reserved_actor_bound" | "reserved_begun"
            ));
            assert!(json_u64(outcome, "acceptedEpochMs") > 0);
            assert_eq!(json_u64(outcome, "originalBudgetMs"), CUTOFF_MS);
            let _ = json_bool(outcome, "cancelRequested");
            assert_frame_full_receipt_key(outcome);
        }
        ExpectedInvocationResultType::Direct => {
            assert_exact_json_keys(outcome, &["resultType", "receipt"]);
            assert_eq!(json_string(outcome, "resultType"), "direct");
            assert_exact_json_keys(
                &outcome["receipt"],
                &[
                    "receiptKey",
                    "terminal",
                    "terminalDigest",
                    "terminalEpochMs",
                ],
            );
            assert_frame_full_receipt_key(&outcome["receipt"]);
            assert_safe_fingerprint(json_string(&outcome["receipt"], "terminalDigest"));
            assert!(json_u64(&outcome["receipt"], "terminalEpochMs") > 0);
        }
        ExpectedInvocationResultType::Task => {
            assert_exact_json_keys(outcome, &["resultType", "snapshot"]);
            assert_eq!(json_string(outcome, "resultType"), "task");
            assert!(outcome["snapshot"].is_object());
        }
        ExpectedInvocationResultType::Acknowledged => {
            assert_exact_json_keys(outcome, &["resultType", "acknowledgement"]);
            assert_eq!(json_string(outcome, "resultType"), "acknowledged");
            let acknowledgement = &outcome["acknowledgement"];
            assert_exact_json_keys(
                acknowledgement,
                &[
                    "receiptKey",
                    "terminalDigest",
                    "ackEpochMs",
                    "expiresEpochMs",
                ],
            );
            assert_frame_full_receipt_key(acknowledgement);
            assert_safe_fingerprint(json_string(acknowledgement, "terminalDigest"));
            assert_eq!(
                json_u64(acknowledgement, "expiresEpochMs"),
                json_u64(acknowledgement, "ackEpochMs") + TOMBSTONE_TTL_MS
            );
        }
    }
    outcome
}

fn assert_exact_v5_request(frame: &serde_json::Value, expected_kind: &str) {
    assert_eq!(frame_kind(frame), expected_kind);
    match expected_kind {
        "ping" | "release" => assert_exact_json_keys(frame, &["kind"]),
        "submit_invocation" => {
            assert_exact_json_keys(frame, &["invocation", "kind"]);
            let invocation = &frame["invocation"];
            assert_exact_json_keys(
                invocation,
                &[
                    "arguments",
                    "invocationId",
                    "reservedTaskId",
                    "responseBudgetMs",
                    "tool",
                    "workspaceHint",
                ],
            );
            assert_canonical_uuid_v4(json_string(invocation, "invocationId"));
            assert_canonical_uuid_v4(json_string(invocation, "reservedTaskId"));
            assert!(invocation["arguments"].is_object());
            assert!(matches!(
                json_string(invocation, "tool"),
                "unica.view"
                    | "unica.apply"
                    | "unica.find"
                    | "unica.search"
                    | "unica.check"
                    | "unica.diff"
                    | "unica.run"
                    | "unica.docs"
            ));
            let workspace = json_string(invocation, "workspaceHint");
            assert!(!workspace.is_empty());
            assert!(!workspace.chars().any(char::is_control));
            assert_eq!(json_u64(invocation, "responseBudgetMs"), CUTOFF_MS);
        }
        "get_task" | "cancel_task" => {
            assert_exact_json_keys(frame, &["kind", "taskId"]);
            assert_canonical_uuid_v4(json_string(frame, "taskId"));
        }
        "wait_task" => {
            assert_exact_json_keys(frame, &["kind", "taskId", "waitMs"]);
            assert_canonical_uuid_v4(json_string(frame, "taskId"));
            assert!(json_u64(frame, "waitMs") <= CUTOFF_MS);
        }
        "recover_invocation_receipt" | "cancel_invocation" => {
            assert_exact_json_keys(frame, &["kind", "receiptKey"]);
            assert_frame_full_receipt_key(frame);
        }
        "acknowledge_invocation_receipt" => {
            assert_exact_json_keys(frame, &["kind", "receiptKey", "terminalDigest"]);
            assert_frame_full_receipt_key(frame);
            assert_safe_fingerprint(json_string(frame, "terminalDigest"));
        }
        other => panic!("unhandled v5 client request kind {other}"),
    }
}

fn assert_exact_v5_server_kind(frame: &serde_json::Value, expected_kind: &str) {
    assert_eq!(frame_kind(frame), expected_kind);
    match expected_kind {
        "pong" | "released" => assert_exact_json_keys(frame, &["kind"]),
        "invocation" => {
            assert_exact_json_keys(frame, &["kind", "outcome"]);
            assert!(frame["outcome"].is_object());
        }
        "task" => {
            assert_exact_json_keys(frame, &["kind", "snapshot"]);
            assert!(frame["snapshot"].is_object());
        }
        "invocation_acknowledged" => {
            assert_exact_json_keys(frame, &["acknowledgement", "kind"]);
            let acknowledgement = &frame["acknowledgement"];
            assert_exact_json_keys(
                acknowledgement,
                &[
                    "ackEpochMs",
                    "expiresEpochMs",
                    "receiptKey",
                    "terminalDigest",
                ],
            );
            assert_frame_full_receipt_key(acknowledgement);
            assert_safe_fingerprint(json_string(acknowledgement, "terminalDigest"));
            assert_eq!(
                json_u64(acknowledgement, "expiresEpochMs"),
                json_u64(acknowledgement, "ackEpochMs") + TOMBSTONE_TTL_MS
            );
        }
        "error" => {
            assert_exact_json_keys(frame, &["code", "kind"]);
            assert!(!json_string(frame, "code").is_empty());
        }
        other => panic!("unhandled v5 server message kind {other}"),
    }
}

fn assert_protocol_event_trace(probe: &ProtocolObservation, server_read: bool, accepted: bool) {
    let mut expected = vec![ProtocolProbeEvent::ClientFrameWritten];
    if server_read {
        expected.push(ProtocolProbeEvent::ServerFrameRead);
    }
    if !accepted {
        expected.push(ProtocolProbeEvent::NegotiationRejected);
    }
    expected.extend([
        ProtocolProbeEvent::ServerFrameWritten,
        ProtocolProbeEvent::ClientFrameRead,
    ]);
    assert_eq!(probe.production_events, expected);
}

fn protocol_version_number(version: ProtocolVersion) -> u64 {
    match version {
        ProtocolVersion::V3 => 3,
        ProtocolVersion::V4 => 4,
        ProtocolVersion::V5 => 5,
    }
}

fn assert_raw_handshake(probe: &ProtocolObservation) {
    let hello = protocol_jsonl_frame(&probe.client_hello_frame_hex, MAX_PROTOCOL_FRAME_BYTES);
    assert_exact_json_keys(
        &hello,
        &[
            "kind",
            "protocolVersion",
            "token",
            "coreIdentity",
            "ownerLease",
        ],
    );
    assert_eq!(json_string(&hello, "kind"), "hello");
    assert_eq!(
        json_u64(&hello, "protocolVersion"),
        protocol_version_number(probe.client)
    );
    let token = json_string(&hello, "token");
    assert!(!token.is_empty() && token.len() <= 4_096);
    let hello_core_identity = json_string(&hello, "coreIdentity");
    assert_safe_fingerprint(hello_core_identity);
    assert_eq!(
        probe.presented_core_identity_digest.as_deref(),
        Some(hello_core_identity)
    );
    assert_canonical_uuid_v4(json_string(&hello, "ownerLease"));

    if probe.client == probe.server {
        let ready = protocol_jsonl_frame(
            probe
                .server_ready_frame_hex
                .as_deref()
                .expect("compatible production handshake must expose raw ready bytes"),
            MAX_PROTOCOL_FRAME_BYTES,
        );
        assert_exact_json_keys(
            &ready,
            &[
                "kind",
                "protocolVersion",
                "coreIdentity",
                "daemonPid",
                "instanceId",
            ],
        );
        assert_eq!(json_string(&ready, "kind"), "ready");
        assert_eq!(
            json_u64(&ready, "protocolVersion"),
            protocol_version_number(probe.server)
        );
        assert_eq!(json_string(&ready, "coreIdentity"), hello_core_identity);
        let daemon_pid = json_u64(&ready, "daemonPid");
        assert!(daemon_pid > 0 && daemon_pid <= u32::MAX as u64);
        assert_canonical_uuid_v4(json_string(&ready, "instanceId"));
    } else {
        assert!(
            probe.server_ready_frame_hex.is_none(),
            "protocol mismatch must not forge a ready handshake"
        );
    }
}

fn assert_spawned_daemon_dispatch(probe: &ProtocolObservation) {
    assert_raw_handshake(probe);
    let argv = decode_hex(&probe.spawned_argv_hex, 4_096);
    let arguments: Vec<_> = argv
        .split(|byte| *byte == 0)
        .filter(|argument| !argument.is_empty())
        .collect();
    assert!(
        arguments.iter().any(|argument| *argument == b"--daemon"),
        "protocol evidence must traverse the real spawned --daemon entry"
    );
    let mut expected = match probe.server {
        ProtocolVersion::V3 => vec![
            DaemonProcessEvent::Spawned,
            DaemonProcessEvent::InterfacesDaemonEntrypointEntered,
            DaemonProcessEvent::DefaultV3CompositionSelected,
        ],
        ProtocolVersion::V4 => {
            panic!("v4 is an incompatible client-frame fixture, never a production daemon")
        }
        ProtocolVersion::V5 => vec![
            DaemonProcessEvent::Spawned,
            DaemonProcessEvent::InterfacesDaemonEntrypointEntered,
            DaemonProcessEvent::VersionedV5DispatchSelected,
        ],
    };
    if probe.client == probe.server {
        expected.extend([
            match probe.server {
                ProtocolVersion::V3 => DaemonProcessEvent::V3HandshakeCompleted,
                ProtocolVersion::V5 => DaemonProcessEvent::V5HandshakeCompleted,
                ProtocolVersion::V4 => unreachable!(),
            },
            DaemonProcessEvent::ProtocolFrameHandled,
        ]);
    }
    if probe.service_capability_fingerprint.is_some() {
        expected.push(DaemonProcessEvent::CanonicalV13ServiceEntered);
    }
    assert_eq!(probe.daemon_process_events, expected);
}

fn probe_action(
    label: &str,
    client: ProtocolVersion,
    server: ProtocolVersion,
    message: ProtocolMessage,
) -> Action {
    Action::ProbeProtocol {
        client,
        server,
        message,
        label: label.to_string(),
    }
}

fn observed_protocol<'a>(report: &'a ScenarioReport, label: &str) -> &'a ProtocolObservation {
    let matches: Vec<_> = report
        .protocol
        .iter()
        .filter(|probe| probe.label == label)
        .collect();
    assert_eq!(matches.len(), 1, "protocol label must occur exactly once");
    matches[0]
}

fn observed_actor_binding<'a>(
    report: &'a ScenarioReport,
    label: &str,
) -> &'a ActorBindingObservation {
    let matches: Vec<_> = report
        .actor_bindings
        .iter()
        .filter(|binding| binding.operation_label == label)
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "actor binding label must occur exactly once"
    );
    matches[0]
}

fn observed_task_publication_capacity<'a>(
    report: &'a ScenarioReport,
    label: &str,
) -> &'a TaskPublicationCapacityObservation {
    let matches: Vec<_> = report
        .task_publication_capacity
        .iter()
        .filter(|observation| observation.operation_label == label)
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "capacity operation label must have one raw source observation"
    );
    let observed = matches[0];
    assert_receipt_key(&observed.receipt_key);
    if let Some(terminal) = &observed.terminal {
        assert_terminal(terminal);
    }
    if let Some(certificate) = &observed.staged_transfer_certificate_sha256 {
        assert_safe_fingerprint(certificate);
    }
    let TaskPublicationCapacityEvidence::LinkCapacity {
        capacity_checked_sequence,
        capacity_rejected_sequence,
        task_store_generation_before,
        task_store_generation_after,
        task_store_create_attempts_before,
        task_store_create_attempts_after,
    } = &observed.evidence;
    assert!(*capacity_checked_sequence < *capacity_rejected_sequence);
    assert_eq!(task_store_generation_before, task_store_generation_after);
    assert_eq!(
        task_store_create_attempts_before, task_store_create_attempts_after,
        "LinkCapacity must be proven before any TaskStore create attempt"
    );
    observed
}

fn observed_task_store_capacity_invariant_violation<'a>(
    report: &'a ScenarioReport,
    label: &str,
) -> &'a TaskStoreCapacityInvariantViolationObservation {
    let matches: Vec<_> = report
        .task_store_capacity_invariant_violations
        .iter()
        .filter(|observation| observation.operation_label == label)
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "the injected impossible TaskStore Capacity must have one raw invariant observation"
    );
    let observed = matches[0];
    assert_receipt_key(&observed.receipt_key);
    assert_terminal(&observed.staged_terminal);
    assert_safe_fingerprint(&observed.staged_transfer_certificate_sha256);
    assert_safe_fingerprint(&observed.live_task_link_reservation_fingerprint);
    assert!(observed.task_link_reserved_sequence < observed.task_store_create_sequence);
    assert!(observed.task_store_create_sequence < observed.capacity_observed_sequence);
    assert!(observed.capacity_observed_sequence < observed.listener_closed_sequence);
    assert!(observed.listener_closed_sequence < observed.restart_requested_sequence);
    assert!(observed.restart_requested_sequence < observed.daemon_stopped_sequence);
    assert_eq!(
        observed.task_store_record_count_before,
        observed.task_store_record_count_after
    );
    assert_eq!(
        observed.materialized_lifecycle_link_count_before,
        observed.materialized_lifecycle_link_count_after
    );
    assert_eq!(
        observed.live_link_reservation_count_before,
        observed.live_link_reservation_count_after
    );
    assert_eq!(
        observed.task_store_generation_before,
        observed.task_store_generation_after
    );
    assert_eq!(
        observed.task_store_create_attempts_after,
        observed.task_store_create_attempts_before + 1
    );
    assert!(
        observed.task_store_record_count_before
            <= observed.materialized_lifecycle_link_count_before
                + observed.live_link_reservation_count_before
    );
    assert!(
        observed.materialized_lifecycle_link_count_before
            + observed.live_link_reservation_count_before
            <= TASK_LINK_LIMIT
    );
    observed
}

fn observed_actor_authorization<'a>(
    report: &'a ScenarioReport,
    label: &str,
) -> &'a ActorAuthorizationObservation {
    let matches: Vec<_> = report
        .actor_authorizations
        .iter()
        .filter(|authorization| authorization.operation_label == label)
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "actor authorization label must occur exactly once"
    );
    matches[0]
}

#[derive(Debug, Clone, Copy)]
struct FailureProjectionCase {
    probe: FailureProbeReason,
    reason: V5SafeFailureReason,
    suffix: &'static str,
    code: &'static str,
    message: &'static str,
    introduced_in_v5: bool,
}

const FAILURE_PROJECTION_CASES: [FailureProjectionCase; 9] = [
    FailureProjectionCase {
        probe: FailureProbeReason::InvocationFailed,
        reason: V5SafeFailureReason::InvocationFailed,
        suffix: "invocation-failed",
        code: "invocation_failed",
        message: "daemon invocation failed",
        introduced_in_v5: false,
    },
    FailureProjectionCase {
        probe: FailureProbeReason::ResultTooLarge,
        reason: V5SafeFailureReason::ResultTooLarge,
        suffix: "result-too-large",
        code: "result_too_large",
        message: "daemon invocation result exceeded the canonical byte limit",
        introduced_in_v5: false,
    },
    FailureProjectionCase {
        probe: FailureProbeReason::Interrupted,
        reason: V5SafeFailureReason::Interrupted,
        suffix: "interrupted",
        code: "interrupted",
        message: "daemon invocation was interrupted",
        introduced_in_v5: false,
    },
    FailureProjectionCase {
        probe: FailureProbeReason::ResumeUnsupported,
        reason: V5SafeFailureReason::ResumeUnsupported,
        suffix: "resume-unsupported",
        code: "resume_unsupported",
        message: "daemon invocation cannot be resumed after restart",
        introduced_in_v5: false,
    },
    FailureProjectionCase {
        probe: FailureProbeReason::PersistenceFailed,
        reason: V5SafeFailureReason::PersistenceFailed,
        suffix: "persistence-failed",
        code: "persistence_failed",
        message: "daemon invocation terminal state could not be persisted",
        introduced_in_v5: false,
    },
    FailureProjectionCase {
        probe: FailureProbeReason::OutcomeUncertain,
        reason: V5SafeFailureReason::OutcomeUncertain,
        suffix: "outcome-uncertain",
        code: "outcome_uncertain",
        message: "daemon invocation outcome is uncertain",
        introduced_in_v5: true,
    },
    FailureProjectionCase {
        probe: FailureProbeReason::TaskCapacity,
        reason: V5SafeFailureReason::TaskCapacity,
        suffix: "task-capacity",
        code: "task_capacity",
        message: "daemon Task capacity was exhausted before execution",
        introduced_in_v5: true,
    },
    FailureProjectionCase {
        probe: FailureProbeReason::WorkspaceCapacity,
        reason: V5SafeFailureReason::WorkspaceCapacity,
        suffix: "workspace-capacity",
        code: "workspace_capacity",
        message: "workspace capacity was exhausted",
        introduced_in_v5: true,
    },
    FailureProjectionCase {
        probe: FailureProbeReason::WorkspaceRegistryFailed,
        reason: V5SafeFailureReason::WorkspaceRegistryFailed,
        suffix: "workspace-registry-failed",
        code: "workspace_registry_failed",
        message: "workspace registry is unavailable",
        introduced_in_v5: true,
    },
];

#[derive(Debug, Clone, Copy)]
enum ExpectedProjection {
    Queued,
    Working,
    Completed(bool),
    Failed(FailureProjectionCase),
    Cancelled,
}

fn json_object(value: &serde_json::Value) -> &serde_json::Map<String, serde_json::Value> {
    value
        .as_object()
        .expect("raw projection must be a JSON object")
}

fn assert_exact_json_keys(value: &serde_json::Value, expected: &[&str]) {
    let mut actual: Vec<_> = json_object(value).keys().map(String::as_str).collect();
    actual.sort_unstable();
    let mut expected = expected.to_vec();
    expected.sort_unstable();
    assert_eq!(actual, expected, "raw projection has schema drift: {value}");
}

fn json_string<'a>(value: &'a serde_json::Value, field: &str) -> &'a str {
    value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_else(|| panic!("raw projection field {field} must be a string"))
}

fn json_u64(value: &serde_json::Value, field: &str) -> u64 {
    value
        .get(field)
        .and_then(serde_json::Value::as_u64)
        .unwrap_or_else(|| panic!("raw projection field {field} must be an unsigned integer"))
}

fn json_bool(value: &serde_json::Value, field: &str) -> bool {
    value
        .get(field)
        .and_then(serde_json::Value::as_bool)
        .unwrap_or_else(|| panic!("raw projection field {field} must be a boolean"))
}

fn iso8601_millis(epoch_ms: u64) -> String {
    let epoch_ms = i64::try_from(epoch_ms).expect("test epoch fits i64");
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(epoch_ms)
        .expect("test epoch is representable")
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn assert_error_data(value: &serde_json::Value, code: &str, message: &str) {
    assert_exact_json_keys(value, &["code", "data", "message"]);
    assert_eq!(value["code"], -32603);
    assert_eq!(value["message"], message);
    assert_eq!(value["data"], serde_json::json!({"code": code}));
}

fn assert_call_tool_result(value: &serde_json::Value, result: &DomainResultObservation) {
    assert_exact_json_keys(
        value,
        &["content", "isError", "resultType", "structuredContent"],
    );
    assert_eq!(value["resultType"], "complete");
    assert_eq!(value["content"], serde_json::json!([]));
    assert_eq!(
        value["structuredContent"],
        serde_json::to_value(result).unwrap()
    );
    assert_eq!(value["isError"], !result.ok);
}

fn assert_raw_terminal(terminal: &serde_json::Value, expected: ExpectedProjection) {
    match expected {
        ExpectedProjection::Completed(_) => {
            assert_exact_json_keys(terminal, &["result", "status"]);
            assert_eq!(json_string(terminal, "status"), "completed");
        }
        ExpectedProjection::Failed(case) => {
            assert_exact_json_keys(terminal, &["reason", "status"]);
            assert_eq!(json_string(terminal, "status"), "failed");
            assert_eq!(json_string(terminal, "reason"), case.code);
        }
        ExpectedProjection::Cancelled => {
            assert_exact_json_keys(terminal, &["status"]);
            assert_eq!(json_string(terminal, "status"), "cancelled");
        }
        ExpectedProjection::Queued | ExpectedProjection::Working => {
            panic!("nonterminal status cannot be a terminal wire value")
        }
    }
}

fn assert_terminal_matches_expected(
    terminal: Option<&TerminalObservation>,
    expected: ExpectedProjection,
) {
    match expected {
        ExpectedProjection::Queued | ExpectedProjection::Working => assert!(terminal.is_none()),
        ExpectedProjection::Completed(expected_ok) => {
            let terminal = terminal.expect("completed projection needs a terminal");
            let result = completed_result(terminal);
            assert_eq!(result.ok, expected_ok);
            if expected_ok {
                assert_eq!(result.summary, SUCCESS_SUMMARY);
            } else {
                assert!(!result.summary.is_empty());
            }
        }
        ExpectedProjection::Failed(case) => assert_failed_terminal(
            terminal.expect("failed projection needs a terminal"),
            case.reason,
        ),
        ExpectedProjection::Cancelled => {
            assert_cancelled_terminal(terminal.expect("cancelled projection needs a terminal"))
        }
    }
}

fn expected_internal_status(expected: ExpectedProjection) -> TaskStatus {
    match expected {
        ExpectedProjection::Queued => TaskStatus::Queued,
        ExpectedProjection::Working => TaskStatus::Working,
        ExpectedProjection::Completed(_) => TaskStatus::Completed,
        ExpectedProjection::Failed(_) => TaskStatus::Failed,
        ExpectedProjection::Cancelled => TaskStatus::Cancelled,
    }
}

fn expected_wire_status(expected: ExpectedProjection) -> &'static str {
    match expected {
        ExpectedProjection::Queued => "queued",
        ExpectedProjection::Working => "working",
        ExpectedProjection::Completed(_) => "completed",
        ExpectedProjection::Failed(_) => "failed",
        ExpectedProjection::Cancelled => "cancelled",
    }
}

fn assert_direct_projection(probe: &ProtocolObservation, expected: ExpectedProjection) {
    assert!(matches!(
        expected,
        ExpectedProjection::Completed(_)
            | ExpectedProjection::Failed(_)
            | ExpectedProjection::Cancelled
    ));
    let delivery = probe
        .delivery
        .as_ref()
        .expect("terminal Direct probe must expose raw delivery evidence");
    assert!(delivery.internal_task_snapshot.is_none());
    assert!(delivery.stored_invocation_record.is_none());
    assert!(delivery.task_terminal_publication.is_none());
    assert!(delivery.native_mcp_projection_hex.is_none());
    assert!(delivery.compatibility_get_projection_hex.is_none());
    assert!(delivery.compatibility_result_projection_hex.is_none());
    assert_eq!(
        delivery.events,
        [
            DeliveryEvent::TerminalPreflighted,
            DeliveryEvent::PendingDirectReceiptBuilt,
            DeliveryEvent::NativeProjectionBuilt,
            DeliveryEvent::FinalInterfaceValueBuilt,
            DeliveryEvent::AcknowledgementWritten,
        ]
    );

    let pending = protocol_frame(
        delivery
            .pending_direct_receipt_hex
            .as_deref()
            .expect("terminal Direct must preserve V5PendingDirectReceipt bytes"),
        MAX_PROTOCOL_FRAME_BYTES,
    );
    assert_exact_json_keys(
        &pending,
        &[
            "receiptKey",
            "terminal",
            "terminalDigest",
            "terminalEpochMs",
        ],
    );
    let key: V5ReceiptKeyWireObservation = serde_json::from_value(pending["receiptKey"].clone())
        .expect("pending Direct receiptKey must use the strict v5 key schema");
    assert_eq!(key, assert_frame_full_receipt_key(&pending));
    let internal_key = delivery
        .direct_receipt_key
        .as_ref()
        .expect("Direct delivery must expose independently observed internal receipt identity");
    assert_receipt_key(internal_key);
    assert_wire_key_matches_internal(&key, internal_key);
    assert_safe_fingerprint(json_string(&pending, "terminalDigest"));
    assert!(json_u64(&pending, "terminalEpochMs") > 0);
    assert_raw_terminal(&pending["terminal"], expected);
    let typed_terminal = delivery
        .direct_terminal
        .as_ref()
        .expect("Direct delivery must expose the exact typed canonical terminal");
    assert_terminal(typed_terminal);
    assert_terminal_matches_expected(Some(typed_terminal), expected);
    let (canonical_payload, canonical_digest) = production_canonical_terminal(typed_terminal);
    assert_eq!(
        pending["terminal"],
        serde_json::from_slice::<serde_json::Value>(&canonical_payload)
            .expect("production canonical terminal is JSON")
    );
    assert_eq!(json_string(&pending, "terminalDigest"), canonical_digest);
    assert_eq!(
        json_u64(&pending, "terminalEpochMs"),
        terminal_epoch_ms(typed_terminal)
    );
    let server_frame =
        protocol_jsonl_frame(&probe.server_write_frame_hex, MAX_PROTOCOL_FRAME_BYTES);
    let outcome =
        assert_invocation_response_frame(&server_frame, ExpectedInvocationResultType::Direct);
    assert_eq!(outcome["receipt"], pending);

    match expected {
        ExpectedProjection::Completed(expected_ok) => {
            let result: DomainResultObservation =
                serde_json::from_value(pending["terminal"]["result"].clone())
                    .expect("completed Direct terminal must carry strict DomainResult");
            assert_eq!(result.ok, expected_ok);
            if expected_ok {
                assert_eq!(result.summary, SUCCESS_SUMMARY);
            } else {
                assert!(!result.summary.is_empty());
            }
            let projected = protocol_frame(
                delivery
                    .final_call_tool_result_hex
                    .as_deref()
                    .expect("Completed Direct must build final CallToolResult before ACK"),
                MAX_PROTOCOL_FRAME_BYTES,
            );
            assert_call_tool_result(&projected, &result);
            assert!(delivery.final_error_data_hex.is_none());
        }
        ExpectedProjection::Failed(case) => {
            assert!(delivery.final_call_tool_result_hex.is_none());
            let projected = protocol_frame(
                delivery
                    .final_error_data_hex
                    .as_deref()
                    .expect("Failed Direct must build final ErrorData before ACK"),
                MAX_PROTOCOL_FRAME_BYTES,
            );
            assert_error_data(&projected, case.code, case.message);
        }
        ExpectedProjection::Cancelled => {
            assert!(delivery.final_call_tool_result_hex.is_none());
            let projected = protocol_frame(
                delivery
                    .final_error_data_hex
                    .as_deref()
                    .expect("Cancelled Direct must build final ErrorData before ACK"),
                MAX_PROTOCOL_FRAME_BYTES,
            );
            assert_error_data(
                &projected,
                "invocation_cancelled",
                "daemon invocation was cancelled",
            );
        }
        ExpectedProjection::Queued | ExpectedProjection::Working => unreachable!(),
    }
}

fn assert_v5_task_snapshot_frame(
    probe: &ProtocolObservation,
    task: &TaskObservation,
    expected: ExpectedProjection,
) {
    let frame = protocol_jsonl_frame(&probe.server_write_frame_hex, MAX_PROTOCOL_FRAME_BYTES);
    assert_exact_json_keys(&frame, &["kind", "snapshot"]);
    assert_eq!(json_string(&frame, "kind"), "task");
    let snapshot = &frame["snapshot"];
    let mut fields = vec![
        "cancelRequested",
        "createdAtEpochMs",
        "invocationId",
        "pollIntervalMs",
        "receiptKeyDigest",
        "status",
        "taskId",
        "ttlMs",
        "updatedAtEpochMs",
        "version",
    ];
    match expected {
        ExpectedProjection::Queued | ExpectedProjection::Working => {}
        ExpectedProjection::Completed(_) => {
            fields.extend(["result", "terminalDigest", "terminalEpochMs"])
        }
        ExpectedProjection::Failed(_) => {
            fields.extend(["reason", "terminalDigest", "terminalEpochMs"])
        }
        ExpectedProjection::Cancelled => fields.extend(["terminalDigest", "terminalEpochMs"]),
    }
    assert_exact_json_keys(snapshot, &fields);
    assert_eq!(json_string(snapshot, "taskId"), task.task_id);
    assert_eq!(json_string(snapshot, "invocationId"), task.invocation_id);
    assert_eq!(
        json_string(snapshot, "receiptKeyDigest"),
        task.receipt_key.key_digest
    );
    assert_eq!(
        json_u64(snapshot, "createdAtEpochMs"),
        task.created_epoch_ms
    );
    assert_eq!(
        json_u64(snapshot, "updatedAtEpochMs"),
        task.updated_epoch_ms
    );
    assert_eq!(json_u64(snapshot, "ttlMs"), task.ttl_ms);
    assert_eq!(json_u64(snapshot, "pollIntervalMs"), task.poll_interval_ms);
    assert_eq!(json_u64(snapshot, "version"), task.version);
    assert_eq!(
        json_bool(snapshot, "cancelRequested"),
        task.cancel_requested
    );
    assert_eq!(
        json_string(snapshot, "status"),
        expected_wire_status(expected)
    );

    assert_terminal_matches_expected(task.terminal.as_ref(), expected);
    if let Some(terminal) = &task.terminal {
        assert_eq!(
            json_u64(snapshot, "terminalEpochMs"),
            terminal_epoch_ms(terminal)
        );
        assert_eq!(
            json_string(snapshot, "terminalDigest"),
            terminal_digest(terminal)
        );
        match expected {
            ExpectedProjection::Completed(_) => assert_eq!(
                snapshot["result"],
                serde_json::to_value(completed_result(terminal)).unwrap()
            ),
            ExpectedProjection::Failed(case) => {
                assert_eq!(json_string(snapshot, "reason"), case.code)
            }
            ExpectedProjection::Cancelled => {}
            ExpectedProjection::Queued | ExpectedProjection::Working => unreachable!(),
        }
    }
}

fn assert_native_task_projection(
    raw: &serde_json::Value,
    task: &TaskObservation,
    expected: ExpectedProjection,
) {
    let mut fields = vec![
        "createdAt",
        "lastUpdatedAt",
        "pollIntervalMs",
        "status",
        "taskId",
        "ttlMs",
    ];
    match expected {
        ExpectedProjection::Completed(_) => fields.push("result"),
        ExpectedProjection::Failed(_) => fields.push("error"),
        ExpectedProjection::Queued
        | ExpectedProjection::Working
        | ExpectedProjection::Cancelled => {}
    }
    assert_exact_json_keys(raw, &fields);
    assert_eq!(json_string(raw, "taskId"), task.task_id);
    assert_eq!(
        json_string(raw, "status"),
        match expected {
            ExpectedProjection::Queued | ExpectedProjection::Working => "working",
            other => expected_wire_status(other),
        }
    );
    assert_eq!(
        json_string(raw, "createdAt"),
        iso8601_millis(task.created_epoch_ms)
    );
    assert_eq!(
        json_string(raw, "lastUpdatedAt"),
        iso8601_millis(task.updated_epoch_ms)
    );
    assert_eq!(json_u64(raw, "ttlMs"), task.ttl_ms);
    assert_eq!(json_u64(raw, "pollIntervalMs"), task.poll_interval_ms);
    match expected {
        ExpectedProjection::Completed(_) => assert_call_tool_result(
            &raw["result"],
            completed_result(task.terminal.as_ref().expect("completed Task terminal")),
        ),
        ExpectedProjection::Failed(case) => {
            assert_error_data(&raw["error"], case.code, case.message)
        }
        ExpectedProjection::Queued
        | ExpectedProjection::Working
        | ExpectedProjection::Cancelled => {}
    }
}

fn compatibility_task_value(
    task: &TaskObservation,
    expected: ExpectedProjection,
) -> serde_json::Value {
    let mut value = serde_json::json!({
        "taskId": task.task_id,
        "invocationId": task.invocation_id,
        "createdAtEpochMs": task.created_epoch_ms,
        "updatedAtEpochMs": task.updated_epoch_ms,
        "ttlMs": task.ttl_ms,
        "pollIntervalMs": task.poll_interval_ms,
        "version": task.version,
        "cancelRequested": task.cancel_requested,
        "status": expected_wire_status(expected),
    });
    if let Some(terminal) = &task.terminal {
        value["terminalEpochMs"] = terminal_epoch_ms(terminal).into();
        value["terminalDigest"] = terminal_digest(terminal).into();
    }
    value
}

fn assert_compatibility_task_projections(
    state: &serde_json::Value,
    result: &serde_json::Value,
    task: &TaskObservation,
    expected: ExpectedProjection,
) {
    let task_value = compatibility_task_value(task, expected);
    let expected_state = match expected {
        ExpectedProjection::Queued | ExpectedProjection::Working => serde_json::json!({
            "ok": true,
            "summary": "Task is still working",
            "data": {"task": task_value},
            "next": [{
                "tool": "unica.task.result",
                "args": {
                    "taskId": task.task_id,
                    "waitMs": task.poll_interval_ms.min(CUTOFF_MS),
                }
            }],
        }),
        ExpectedProjection::Completed(_) => serde_json::json!({
            "ok": true,
            "summary": "Task completed",
            "data": {"task": task_value},
        }),
        ExpectedProjection::Failed(case) => serde_json::json!({
            "ok": false,
            "summary": case.message,
            "data": {"code": case.code, "task": task_value},
        }),
        ExpectedProjection::Cancelled => serde_json::json!({
            "ok": false,
            "summary": "Task was cancelled",
            "data": {"code": "task_cancelled", "task": task_value},
        }),
    };
    assert_eq!(*state, expected_state);
    match expected {
        ExpectedProjection::Completed(_) => assert_eq!(
            *result,
            serde_json::to_value(completed_result(
                task.terminal.as_ref().expect("completed Task terminal")
            ))
            .unwrap()
        ),
        ExpectedProjection::Queued
        | ExpectedProjection::Working
        | ExpectedProjection::Failed(_)
        | ExpectedProjection::Cancelled => assert_eq!(*result, expected_state),
    }
}

fn assert_task_projection(
    probe: &ProtocolObservation,
    expected: ExpectedProjection,
    expected_owner: Option<TaskTerminalOwnerFixture>,
) {
    let delivery = probe
        .delivery
        .as_ref()
        .expect("Task projection probe must expose raw adapter evidence");
    assert!(delivery.pending_direct_receipt_hex.is_none());
    assert!(delivery.direct_receipt_key.is_none());
    assert!(delivery.direct_terminal.is_none());
    assert!(delivery.final_call_tool_result_hex.is_none());
    assert!(delivery.final_error_data_hex.is_none());
    assert_eq!(
        delivery.events,
        [
            DeliveryEvent::NativeProjectionBuilt,
            DeliveryEvent::CompatibilityGetProjectionBuilt,
            DeliveryEvent::CompatibilityResultProjectionBuilt,
        ]
    );
    let task = delivery
        .internal_task_snapshot
        .as_ref()
        .expect("Task projection must expose the exact internal v5 snapshot");
    assert_eq!(task.status, expected_internal_status(expected));
    if let Some(owner) = expected_owner {
        assert!(matches!(expected, ExpectedProjection::Completed(false)));
        let publication = delivery
            .task_terminal_publication
            .as_ref()
            .expect("semantic Task projection must expose its raw terminal publication");
        assert_eq!(publication.receipt_key, task.receipt_key);
        assert_eq!(publication.terminal, *task.terminal.as_ref().unwrap());
        assert_terminal_preflight(publication);
        assert!(
            publication.response_frames.iter().any(|frame| {
                frame.response_kind == ResponseKind::Task
                    && frame.response_jsonl.raw_hex == probe.server_write_frame_hex
            }),
            "owner publication must carry the exact Task response frame observed on the wire"
        );
        match owner {
            TaskTerminalOwnerFixture::ReceiptBacked => {
                assert_eq!(task.projection_source, ProjectionSource::ReceiptLedger);
                assert!(
                    delivery.stored_invocation_record.is_none(),
                    "receipt-backed Task must not fabricate a TaskStore record"
                );
                assert_eq!(
                    publication.commit.owner(),
                    TerminalPublicationOwner::ReceiptBackedTask
                );
                assert!(publication.commit.receipt().is_some());
            }
            TaskTerminalOwnerFixture::Bound => {
                assert_eq!(task.projection_source, ProjectionSource::TaskStore);
                let record_evidence = delivery
                    .stored_invocation_record
                    .as_ref()
                    .expect("bound semantic Task needs exact TaskStore record bytes");
                let TerminalCommitPreflightObservation::BoundTaskStore { task: commit } =
                    &publication.commit
                else {
                    panic!("bound semantic Task must expose BoundTaskStore publication pieces")
                };
                assert_eq!(record_evidence, &commit.task_record);
            }
            TaskTerminalOwnerFixture::Staged => {
                assert_eq!(task.projection_source, ProjectionSource::TaskStore);
                let record_evidence = delivery
                    .stored_invocation_record
                    .as_ref()
                    .expect("staged semantic Task needs exact TaskStore record bytes");
                let TerminalCommitPreflightObservation::StagedHandoffTask { task: commit } =
                    &publication.commit
                else {
                    panic!("staged semantic Task must expose StagedHandoffTask publication pieces")
                };
                assert_eq!(record_evidence, &commit.task_record);
            }
        }
    } else {
        assert!(
            delivery.task_terminal_publication.is_none(),
            "only owner-specific semantic probes carry terminal publication evidence"
        );
        let record_evidence = delivery
            .stored_invocation_record
            .as_ref()
            .expect("TaskStore projection must expose exact persisted schema-v1 record bytes");
        assert_eq!(task.projection_source, ProjectionSource::TaskStore);
        assert_eq!(record_evidence.encoded_bytes, task.encoded_bytes);
    }
    if let Some(record_evidence) = &delivery.stored_invocation_record {
        let record = decode_v5_stored_record(record_evidence);
        assert_v5_stored_record_matches_task(&record, task);
    }
    assert_v5_task_snapshot_frame(probe, task, expected);
    let native = protocol_frame(
        delivery
            .native_mcp_projection_hex
            .as_deref()
            .expect("Task projection must expose exact native MCP bytes"),
        MAX_PROTOCOL_FRAME_BYTES,
    );
    assert_native_task_projection(&native, task, expected);
    let compatibility_get = protocol_frame(
        delivery
            .compatibility_get_projection_hex
            .as_deref()
            .expect("Task projection must expose exact compatibility get bytes"),
        MAX_PROTOCOL_FRAME_BYTES,
    );
    let compatibility_result = protocol_frame(
        delivery
            .compatibility_result_projection_hex
            .as_deref()
            .expect("Task projection must expose exact compatibility result bytes"),
        MAX_PROTOCOL_FRAME_BYTES,
    );
    assert_compatibility_task_projections(
        &compatibility_get,
        &compatibility_result,
        task,
        expected,
    );
}

fn load_window_ms(load: &LoadObservation) -> u64 {
    load.window_ended_monotonic_ms - load.window_started_monotonic_ms
}

fn load_total_elapsed_ms(load: &LoadObservation) -> u64 {
    load.drain_completed_monotonic_ms - load.window_started_monotonic_ms
}

fn load_writer_drain_ms(load: &LoadObservation) -> u64 {
    load.drain_completed_monotonic_ms - load.window_ended_monotonic_ms
}

fn completed_at_window_end(load: &LoadObservation) -> usize {
    load.lifecycles
        .iter()
        .filter(|lifecycle| lifecycle.completed_monotonic_ms <= load.window_ended_monotonic_ms)
        .count()
}

fn load_p99_ms(load: &LoadObservation) -> u64 {
    assert!(!load.lifecycles.is_empty());
    let mut latencies: Vec<_> = load
        .lifecycles
        .iter()
        .map(|lifecycle| lifecycle.response_latency_ms)
        .collect();
    latencies.sort_unstable();
    let rank = (latencies.len() * 99).div_ceil(100).saturating_sub(1);
    latencies[rank]
}

fn max_concurrency_sample(
    load: &LoadObservation,
    select: impl Fn(&ConcurrencySample) -> u64,
) -> u64 {
    load.concurrency_samples
        .iter()
        .map(select)
        .max()
        .unwrap_or(0)
}

fn assert_direct_load_lifecycles(load: &LoadObservation, expected: usize) {
    assert_eq!(load.lifecycles.len(), expected);
    assert!(load.capacity_rejections.is_empty());
    assert!(load.store_errors.is_empty());
    let mut keys = Vec::with_capacity(expected);
    let mut callbacks = Vec::with_capacity(expected);
    for lifecycle in &load.lifecycles {
        assert!(completed_result(&lifecycle.terminal).ok);
        let acknowledgement = lifecycle
            .acknowledgement
            .as_ref()
            .expect("immediate-ACK lifecycle must expose the committed acknowledgement");
        assert_eq!(
            acknowledgement.terminal_digest,
            terminal_digest(&lifecycle.terminal)
        );
        let callback = lifecycle
            .callback_invocation_id
            .as_ref()
            .expect("each Direct lifecycle must expose its production callback identity");
        assert_eq!(callback, &lifecycle.key.invocation_id);
        assert!(lifecycle.terminal_store_generation > 0);
        keys.push(lifecycle.key.key_digest.as_str());
        callbacks.push(callback.as_str());
    }
    keys.sort_unstable();
    keys.dedup();
    callbacks.sort_unstable();
    callbacks.dedup();
    assert_eq!(keys.len(), expected);
    assert_eq!(callbacks.len(), expected);
}

fn assert_tombstones_match_active_ack_window(snapshot: &Snapshot, load: &LoadObservation) {
    let mut expected: Vec<_> = load
        .lifecycles
        .iter()
        .filter_map(|lifecycle| {
            let acknowledgement = lifecycle.acknowledgement.as_ref()?;
            (acknowledgement.ack_epoch_ms <= snapshot.epoch_ms
                && snapshot.epoch_ms < acknowledgement.expires_epoch_ms)
                .then_some((
                    lifecycle.key.clone(),
                    acknowledgement.terminal_digest.clone(),
                    acknowledgement.ack_epoch_ms,
                    acknowledgement.expires_epoch_ms,
                ))
        })
        .collect();
    let mut actual: Vec<_> = snapshot
        .tombstones
        .iter()
        .map(|tombstone| {
            (
                tombstone.key.clone(),
                tombstone.terminal_digest.clone(),
                tombstone.ack_epoch_ms,
                tombstone.expires_epoch_ms,
            )
        })
        .collect();
    expected.sort_by(|left, right| left.0.key_digest.cmp(&right.0.key_digest));
    actual.sort_by(|left, right| left.0.key_digest.cmp(&right.0.key_digest));
    assert_eq!(actual, expected);
}

fn assert_operation_trace(report: &ScenarioReport, label: &str, expected: &[OperationEventState]) {
    let actual: Vec<_> = report
        .operation_events
        .iter()
        .filter(|event| event.label == label)
        .map(|event| event.state)
        .collect();
    assert_eq!(actual, expected, "operation trace for {label}");
}

fn assert_gate_trace(report: &ScenarioReport, label: &str, expected: &[GateTransition]) {
    let actual: Vec<_> = report
        .gate_events
        .iter()
        .filter(|event| event.operation_label == label)
        .map(|event| event.transition)
        .collect();
    assert_eq!(actual, expected, "gate trace for {label}");
}

fn assert_load_raw_bounds(load: &LoadObservation) {
    assert!(load.window_ended_monotonic_ms >= load.window_started_monotonic_ms);
    assert!(load.drain_completed_monotonic_ms >= load.window_ended_monotonic_ms);
    assert!(load.lifecycles.len() <= 28_800);
    for lifecycle in &load.lifecycles {
        assert_receipt_key(&lifecycle.key);
        assert!(lifecycle.accepted_epoch_ms > 0);
        assert!(lifecycle.completed_monotonic_ms >= lifecycle.started_monotonic_ms);
        assert_eq!(
            lifecycle.response_latency_ms,
            lifecycle.completed_monotonic_ms - lifecycle.started_monotonic_ms
        );
        assert_terminal(&lifecycle.terminal);
        if let Some(acknowledgement) = &lifecycle.acknowledgement {
            assert_acknowledgement(acknowledgement);
            assert_eq!(
                acknowledgement.terminal_digest,
                terminal_digest(&lifecycle.terminal)
            );
            assert!(acknowledgement.ack_epoch_ms >= terminal_epoch_ms(&lifecycle.terminal));
        }
        if let Some(callback) = &lifecycle.callback_invocation_id {
            assert_canonical_uuid_v4(callback);
        }
        assert!(lifecycle.terminal_store_generation > 0);
    }
    for sample in &load.concurrency_samples {
        assert!(sample.monotonic_ms >= load.window_started_monotonic_ms);
        assert!(sample.owner_slots <= 65);
        assert!(sample.handshakes <= 32);
        assert!(sample.accept_batch <= 32);
        assert!(sample.live_receipts <= LIVE_RECEIPT_LIMIT);
    }
}

fn assert_exact_response_identity(observed: &ResponseObservation, receipt: &ReceiptObservation) {
    let key = observed
        .key
        .as_ref()
        .expect("receipt response must include its exact key");
    assert_receipt_key(key);
    assert_eq!(key, &receipt.key);
    assert_eq!(observed.original_budget_ms, Some(CUTOFF_MS));
    assert_eq!(
        observed.cutoff_epoch_ms,
        Some(receipt.accepted_epoch_ms + CUTOFF_MS)
    );
}

fn assert_max_result_entitlement(receipt: &ReceiptObservation) {
    assert_eq!(
        receipt.encoded_bytes + receipt.reserved_result_bytes,
        MAX_RESPONSE_LINE_BYTES
    );
}

fn receipt_quota_bytes(snapshot: &Snapshot) -> u64 {
    snapshot.receipt_actual_bytes + snapshot.receipt_reserved_bytes
}

fn assert_retained_receipt_backed_set(snapshot: &Snapshot, expected: usize) {
    assert_eq!(snapshot.receipt_live_count, expected as u64);
    let retained: Vec<_> = snapshot
        .receipts
        .iter()
        .filter(|receipt| receipt.state == SeedReceiptState::TaskTerminalReceiptBacked)
        .collect();
    assert_eq!(retained.len(), expected);
    let mut keys = Vec::with_capacity(expected);
    for receipt in retained {
        let terminal = receipt
            .terminal
            .as_ref()
            .expect("retained receipt-backed terminal must expose its terminal");
        assert!(completed_result(terminal).ok);
        assert_safe_fingerprint(terminal_digest(terminal));
        assert_eq!(
            receipt.expires_epoch_ms,
            Some(terminal_epoch_ms(terminal) + DIRECT_TASK_TTL_MS)
        );
        assert_max_result_entitlement(receipt);
        keys.push(receipt.key.key_digest.as_str());
    }
    keys.sort_unstable();
    keys.dedup();
    assert_eq!(keys.len(), expected);
    assert!(snapshot.receipt_actual_bytes > 0);
    assert!(snapshot.receipt_reserved_bytes > 0);
    assert_eq!(
        receipt_quota_bytes(snapshot),
        expected as u64 * MAX_RESPONSE_LINE_BYTES
    );
}

fn success_payload() -> TerminalFixture {
    TerminalFixture::Success {
        payload: "canonical-success".to_string(),
    }
}

fn direct_provider() -> Action {
    Action::ConfigureProvider {
        execution_class: ExecutionClass::Direct,
        terminal: success_payload(),
        cooperative_cancel: true,
        side_effect_marker: false,
    }
}

fn known_long_provider() -> Action {
    Action::ConfigureProvider {
        execution_class: ExecutionClass::KnownLong,
        terminal: success_payload(),
        cooperative_cancel: true,
        side_effect_marker: false,
    }
}

fn submit(label: &str) -> Action {
    Action::Submit {
        request: RequestCase::Canonical,
        response_budget_ms: CUTOFF_MS,
        disconnect: DisconnectPoint::Never,
        label: label.to_string(),
    }
}

fn checkpoint_action(label: &str) -> Action {
    Action::Checkpoint {
        label: label.to_string(),
    }
}

#[test]
fn malformed_outer_envelope_creates_no_receipt_and_runs_no_domain_code() {
    let malformed = [
        EnvelopeCase::MissingInvocationId,
        EnvelopeCase::NoncanonicalInvocationId,
        EnvelopeCase::MissingReservedTaskId,
        EnvelopeCase::NoncanonicalReservedTaskId,
        EnvelopeCase::UnknownTool,
        EnvelopeCase::UnknownField,
        EnvelopeCase::MalformedArguments,
        EnvelopeCase::OversizedArguments,
        EnvelopeCase::ResponseBudgetAboveMaximum,
        EnvelopeCase::EmptyWorkspaceHint,
        EnvelopeCase::WorkspaceHintWithControl,
        EnvelopeCase::MalformedWorkspaceHint,
        EnvelopeCase::OversizedWorkspaceHint,
    ];
    let mut actions = vec![direct_provider()];
    for (index, envelope) in malformed.into_iter().enumerate() {
        actions.push(Action::SendOuterEnvelope {
            envelope,
            label: format!("malformed-{index}"),
        });
    }
    actions.push(checkpoint_action("after-malformed-matrix"));

    let report = execute(Scenario::fake(actions));
    let after = checkpoint(&report, "after-malformed-matrix");
    assert_eq!(after.receipt_live_count, 0);
    assert!(after.receipts.is_empty());
    assert!(after.tasks.is_empty());
    assert_eq!(after.callbacks.total_domain(), 0);
    assert_eq!(report.responses.len(), malformed.len());
    for index in 0..malformed.len() {
        let rejected = response(&report, &format!("malformed-{index}"));
        assert_eq!(rejected.kind, ResponseKind::Rejected);
        assert_eq!(rejected.error, Some(ErrorCode::InvalidRequest));
    }
}

#[test]
fn reserve_precedes_validation_admission_prepare() {
    let cases = [
        (
            "validation-accepted",
            BarrierPoint::ValidationEntered,
            None,
            TerminalClass::Completed(true),
            false,
        ),
        (
            "validation-rejected",
            BarrierPoint::ValidationEntered,
            None,
            TerminalClass::Completed(false),
            false,
        ),
        (
            "admission-accepted",
            BarrierPoint::AdmissionEntered,
            None,
            TerminalClass::Completed(true),
            false,
        ),
        (
            "admission-invalid",
            BarrierPoint::AdmissionEntered,
            Some(WorkspaceAdmissionFailure::Invalid),
            TerminalClass::Completed(false),
            false,
        ),
        (
            "admission-capacity",
            BarrierPoint::AdmissionEntered,
            Some(WorkspaceAdmissionFailure::Capacity),
            TerminalClass::Failed(V5SafeFailureReason::WorkspaceCapacity),
            false,
        ),
        (
            "admission-registry-failed",
            BarrierPoint::AdmissionEntered,
            Some(WorkspaceAdmissionFailure::RegistryFailed),
            TerminalClass::Failed(V5SafeFailureReason::WorkspaceRegistryFailed),
            true,
        ),
        (
            "prepare-accepted",
            BarrierPoint::PrepareEntered,
            None,
            TerminalClass::Completed(true),
            false,
        ),
        (
            "prepare-rejected",
            BarrierPoint::PrepareEntered,
            None,
            TerminalClass::Completed(false),
            false,
        ),
    ];

    for (label, barrier, admission_rejection, expected_terminal, fail_stop) in cases {
        let event = match barrier {
            BarrierPoint::ValidationEntered => EventKind::ValidationEntered,
            BarrierPoint::AdmissionEntered => EventKind::AdmissionEntered,
            BarrierPoint::PrepareEntered => EventKind::PrepareEntered,
            _ => unreachable!("matrix contains only domain callback barriers"),
        };
        let blocked_state = if barrier == BarrierPoint::PrepareEntered {
            SeedReceiptState::ReservedBegun
        } else {
            SeedReceiptState::ReservedUnbound
        };
        let callbacks = match barrier {
            BarrierPoint::ValidationEntered => CallbackCounts {
                validation: 1,
                ..CallbackCounts::default()
            },
            BarrierPoint::AdmissionEntered => CallbackCounts {
                validation: 1,
                admission: 1,
                ..CallbackCounts::default()
            },
            BarrierPoint::PrepareEntered => CallbackCounts {
                validation: 1,
                admission: 1,
                prepare: 1,
                execute: 0,
            },
            _ => unreachable!("matrix contains only domain callback barriers"),
        };
        let mut actions = vec![direct_provider()];
        match barrier {
            BarrierPoint::ValidationEntered => actions.push(Action::ConfigureValidation {
                reject: expected_terminal == TerminalClass::Completed(false),
            }),
            BarrierPoint::AdmissionEntered => actions.push(Action::ConfigureAdmission {
                rejection: admission_rejection,
            }),
            BarrierPoint::PrepareEntered => actions.push(Action::ConfigurePrepare {
                reject: expected_terminal == TerminalClass::Completed(false),
            }),
            _ => unreachable!("matrix contains only domain callback barriers"),
        }
        actions.extend([
            Action::InstallBarrier { point: barrier },
            submit("submit"),
            Action::WaitForEvent { event },
            checkpoint_action(label),
            Action::ReleaseBarrier { point: barrier },
            Action::WaitForEvent {
                event: EventKind::ReceiptTerminalCommitted,
            },
            checkpoint_action("after-terminal"),
        ]);
        let report = execute(Scenario::fake(actions));
        let blocked = checkpoint(&report, label);
        assert_eq!(blocked.receipt_live_count, 1);
        let receipt = only_receipt(blocked);
        assert_eq!(receipt.state, blocked_state);
        assert_eq!(receipt.original_budget_ms, CUTOFF_MS);
        assert_event_order(
            &report,
            &[
                EventKind::StrictEnvelopeParsed,
                EventKind::ReceiptReserved,
                event,
            ],
        );
        assert_eq!(blocked.callbacks, callbacks);
        let terminal_snapshot = checkpoint(&report, "after-terminal");
        let terminal_receipt = only_receipt(terminal_snapshot);
        let observed = response(&report, "submit");
        assert_exact_response_identity(observed, terminal_receipt);
        assert_eq!(observed.kind, ResponseKind::Direct);
        assert_eq!(observed.error, None);
        assert_eq!(
            response_terminal(observed),
            terminal_of_receipt(terminal_receipt)
        );
        assert_eq!(
            terminal_class(Some(response_terminal(observed))),
            expected_terminal
        );
        assert_eq!(
            response_terminal(observed),
            terminal_of_receipt(terminal_receipt)
        );
        if fail_stop {
            assert_eq!(terminal_snapshot.listener, ListenerState::Closed);
            assert!(terminal_snapshot.restart_requested);
            assert!(!terminal_snapshot.daemon_running);
        } else {
            assert_eq!(terminal_snapshot.listener, ListenerState::Listening);
            assert!(!terminal_snapshot.restart_requested);
            assert!(terminal_snapshot.daemon_running);
        }
        assert_eq!(terminal_receipt.original_budget_ms, CUTOFF_MS);
        assert_eq!(terminal_receipt.key, receipt.key);
        assert_eq!(terminal_snapshot.receipt_live_count, 1);
        assert_publication_matches_snapshot(
            &report,
            terminal_snapshot,
            &terminal_receipt.key,
            terminal_of_receipt(terminal_receipt),
            TerminalPublicationOwner::DirectReceiptLedger,
        );
    }
}

#[test]
fn restart_reserved_without_committed_handoff_never_invents_task() {
    for state in [
        SeedReceiptState::ReservedUnbound,
        SeedReceiptState::ReservedActorBound,
    ] {
        for cancel_requested in [false, true] {
            let mut actions = vec![Action::SeedReceipt {
                state,
                cancel_requested,
                staged_terminal: None,
            }];
            if state == SeedReceiptState::ReservedUnbound {
                actions.push(Action::Crash {
                    point: CrashPoint::ReservedUnbound,
                });
            }
            actions.push(Action::Restart);
            actions.push(checkpoint_action("recovered"));
            let report = execute(Scenario::fake(actions));
            let recovered = checkpoint(&report, "recovered");
            let receipt = only_receipt(recovered);
            assert_eq!(receipt.state, SeedReceiptState::DirectTerminalUnacked);
            let terminal = terminal_of_receipt(receipt);
            if cancel_requested {
                assert_cancelled_terminal(terminal);
            } else {
                assert_failed_terminal(terminal, V5SafeFailureReason::Interrupted);
            }
            assert_eq!(
                receipt.expires_epoch_ms,
                Some(terminal_epoch_ms(terminal) + DIRECT_TASK_TTL_MS)
            );
            assert!(recovered.tasks.is_empty());
            assert_eq!(recovered.callbacks.total_domain(), 0);
        }
    }
}

#[test]
fn restart_begun_without_committed_handoff_is_direct_outcome_uncertain() {
    let report = execute(Scenario::fake(vec![
        Action::SeedReceipt {
            state: SeedReceiptState::ReservedBegun,
            cancel_requested: false,
            staged_terminal: None,
        },
        Action::Restart,
        checkpoint_action("recovered"),
    ]));

    let recovered = checkpoint(&report, "recovered");
    let receipt = only_receipt(recovered);
    assert_eq!(receipt.state, SeedReceiptState::DirectTerminalUnacked);
    assert_failed_terminal(
        terminal_of_receipt(receipt),
        V5SafeFailureReason::OutcomeUncertain,
    );
    assert!(recovered.tasks.is_empty());
    assert_eq!(recovered.callbacks.total_domain(), 0);
}

#[test]
fn v5_rejects_v3_v4_and_strictly_round_trips_receipt_messages() {
    let v5_cases = [
        ("v5-ping", ProtocolMessage::Ping, "ping", "pong"),
        (
            "v5-release",
            ProtocolMessage::Release,
            "release",
            "released",
        ),
        (
            "v5-submit",
            ProtocolMessage::SubmitWithCoreIdentity {
                selection: CoreIdentitySelection::ExactProductionV5,
            },
            "submit_invocation",
            "invocation",
        ),
        ("v5-get-task", ProtocolMessage::GetTask, "get_task", "task"),
        (
            "v5-wait-task",
            ProtocolMessage::WaitTask,
            "wait_task",
            "task",
        ),
        (
            "v5-cancel-task",
            ProtocolMessage::CancelTask,
            "cancel_task",
            "task",
        ),
        (
            "v5-recover",
            ProtocolMessage::RecoverReceipt,
            "recover_invocation_receipt",
            "invocation",
        ),
        (
            "v5-ack",
            ProtocolMessage::AcknowledgeReceipt,
            "acknowledge_invocation_receipt",
            "invocation_acknowledged",
        ),
        (
            "v5-cancel",
            ProtocolMessage::CancelReceipt,
            "cancel_invocation",
            "invocation",
        ),
    ];
    let mut actions = Vec::new();
    for (label, message, _, _) in v5_cases {
        actions.push(probe_action(
            label,
            ProtocolVersion::V5,
            ProtocolVersion::V5,
            message,
        ));
    }
    actions.push(probe_action(
        "v3-default-guard",
        ProtocolVersion::V3,
        ProtocolVersion::V3,
        ProtocolMessage::SubmitWithCoreIdentity {
            selection: CoreIdentitySelection::ArbitraryCanonical,
        },
    ));
    let incompatible_cases = [
        (
            "v5-recover-to-v3",
            ProtocolVersion::V5,
            ProtocolVersion::V3,
            ProtocolMessage::RecoverReceipt,
        ),
        (
            "v5-ack-to-v3",
            ProtocolVersion::V5,
            ProtocolVersion::V3,
            ProtocolMessage::AcknowledgeReceipt,
        ),
        (
            "v4-cancel-to-v3",
            ProtocolVersion::V4,
            ProtocolVersion::V3,
            ProtocolMessage::CancelReceipt,
        ),
        (
            "v3-to-v5",
            ProtocolVersion::V3,
            ProtocolVersion::V5,
            ProtocolMessage::RecoverReceipt,
        ),
        (
            "v4-to-v5",
            ProtocolVersion::V4,
            ProtocolVersion::V5,
            ProtocolMessage::RecoverReceipt,
        ),
    ];
    for (label, client, server, message) in incompatible_cases {
        actions.push(Action::ProbeProtocol {
            client,
            server,
            message,
            label: label.to_string(),
        });
    }
    let invocation_outcome_cases = [
        (
            "v5-outcome-receipt-pending",
            ProtocolMessage::ReceiptPendingOutcome,
            ExpectedInvocationResultType::ReceiptPending,
        ),
        (
            "v5-outcome-task",
            ProtocolMessage::TaskOutcome,
            ExpectedInvocationResultType::Task,
        ),
        (
            "v5-outcome-acknowledged",
            ProtocolMessage::AcknowledgedOutcome,
            ExpectedInvocationResultType::Acknowledged,
        ),
    ];
    for (label, message, _) in invocation_outcome_cases {
        actions.push(probe_action(
            label,
            ProtocolVersion::V5,
            ProtocolVersion::V5,
            message,
        ));
    }
    for (label, message) in [
        (
            "v5-direct-completed",
            ProtocolMessage::DirectCompletedTerminal,
        ),
        (
            "v5-direct-semantic-completed",
            ProtocolMessage::DirectSemanticCompletedTerminal,
        ),
        (
            "v5-direct-cancelled",
            ProtocolMessage::DirectCancelledTerminal,
        ),
        ("v5-task-queued", ProtocolMessage::TaskQueuedProjection),
        ("v5-task-working", ProtocolMessage::TaskWorkingProjection),
        (
            "v5-task-completed",
            ProtocolMessage::TaskCompletedProjection,
        ),
        (
            "v5-task-cancelled",
            ProtocolMessage::TaskCancelledProjection,
        ),
        (
            "v5-task-semantic-receipt-backed",
            ProtocolMessage::TaskSemanticCompletedProjection {
                owner: TaskTerminalOwnerFixture::ReceiptBacked,
            },
        ),
        (
            "v5-task-semantic-bound",
            ProtocolMessage::TaskSemanticCompletedProjection {
                owner: TaskTerminalOwnerFixture::Bound,
            },
        ),
        (
            "v5-task-semantic-staged",
            ProtocolMessage::TaskSemanticCompletedProjection {
                owner: TaskTerminalOwnerFixture::Staged,
            },
        ),
    ] {
        actions.push(probe_action(
            label,
            ProtocolVersion::V5,
            ProtocolVersion::V5,
            message,
        ));
    }
    for case in FAILURE_PROJECTION_CASES {
        actions.push(probe_action(
            &format!("v5-direct-failure-{}", case.suffix),
            ProtocolVersion::V5,
            ProtocolVersion::V5,
            ProtocolMessage::DirectFailureTerminal { reason: case.probe },
        ));
        actions.push(probe_action(
            &format!("v5-task-failure-{}", case.suffix),
            ProtocolVersion::V5,
            ProtocolVersion::V5,
            ProtocolMessage::TaskFailureProjection { reason: case.probe },
        ));
        actions.push(probe_action(
            &format!(
                "v3-{}-failure-{}",
                if case.introduced_in_v5 {
                    "rejects-v5"
                } else {
                    "accepts-legacy"
                },
                case.suffix
            ),
            ProtocolVersion::V3,
            ProtocolVersion::V3,
            ProtocolMessage::DirectFailureTerminal { reason: case.probe },
        ));
        if case.introduced_in_v5 {
            actions.push(probe_action(
                &format!("v3-schema-v2-rejects-v5-record-{}", case.suffix),
                ProtocolVersion::V3,
                ProtocolVersion::V3,
                ProtocolMessage::StoredInvocationRecord {
                    schema_version: 1,
                    reason: case.probe,
                },
            ));
        }
    }
    let strict_cases = [
        (
            "v5-request-unknown-field",
            StrictSchemaTarget::RequestUnknownField,
        ),
        (
            "v5-request-missing-required",
            StrictSchemaTarget::RequestMissingRequiredField,
        ),
        (
            "v5-request-cross-variant",
            StrictSchemaTarget::RequestCrossVariantField,
        ),
        (
            "v5-response-unknown-field",
            StrictSchemaTarget::ResponseUnknownField,
        ),
        (
            "v5-response-missing-required",
            StrictSchemaTarget::ResponseMissingRequiredField,
        ),
        (
            "v5-response-cross-variant",
            StrictSchemaTarget::ResponseCrossVariantField,
        ),
        (
            "v5-terminal-unknown-field",
            StrictSchemaTarget::TerminalUnknownField,
        ),
        (
            "v5-terminal-missing-required",
            StrictSchemaTarget::TerminalMissingRequiredField,
        ),
        (
            "v5-terminal-cross-variant",
            StrictSchemaTarget::TerminalCrossVariantField,
        ),
        (
            "v5-task-snapshot-unknown-field",
            StrictSchemaTarget::TaskSnapshotUnknownField,
        ),
        (
            "v5-task-snapshot-missing-required",
            StrictSchemaTarget::TaskSnapshotMissingRequiredField,
        ),
        (
            "v5-task-snapshot-cross-variant",
            StrictSchemaTarget::TaskSnapshotCrossVariantField,
        ),
        (
            "v5-stored-record-unknown-top-level",
            StrictSchemaTarget::StoredRecordUnknownTopLevel,
        ),
        (
            "v5-stored-record-unknown-task-field",
            StrictSchemaTarget::StoredRecordUnknownTaskField,
        ),
        (
            "v5-stored-record-missing-required-field",
            StrictSchemaTarget::StoredRecordMissingRequiredField,
        ),
        (
            "v5-stored-record-cross-variant-field",
            StrictSchemaTarget::StoredRecordCrossVariantField,
        ),
        (
            "v5-transfer-certificate-unknown-field",
            StrictSchemaTarget::TransferCertificateUnknownField,
        ),
        (
            "v5-transfer-certificate-missing-required",
            StrictSchemaTarget::TransferCertificateMissingRequiredField,
        ),
        (
            "v5-transfer-certificate-cross-variant",
            StrictSchemaTarget::TransferCertificateCrossVariantField,
        ),
    ];
    assert_strict_schema_oracle_is_discriminating(&strict_cases);
    for (label, target) in strict_cases {
        actions.push(probe_action(
            label,
            ProtocolVersion::V5,
            ProtocolVersion::V5,
            ProtocolMessage::MalformedV5Schema { target },
        ));
    }
    let error_code_cases = [
        (
            V5DaemonErrorCodeFixture::InvalidRequest,
            "invalid_request",
            ErrorCode::InvalidRequest,
        ),
        (
            V5DaemonErrorCodeFixture::HandshakeRequired,
            "handshake_required",
            ErrorCode::HandshakeRequired,
        ),
        (
            V5DaemonErrorCodeFixture::ProtocolMismatch,
            "protocol_mismatch",
            ErrorCode::ProtocolMismatch,
        ),
        (
            V5DaemonErrorCodeFixture::CoreMismatch,
            "core_mismatch",
            ErrorCode::CoreMismatch,
        ),
        (
            V5DaemonErrorCodeFixture::Unauthorized,
            "unauthorized",
            ErrorCode::Unauthorized,
        ),
        (
            V5DaemonErrorCodeFixture::DuplicateLease,
            "duplicate_lease",
            ErrorCode::DuplicateLease,
        ),
        (
            V5DaemonErrorCodeFixture::Overloaded,
            "overloaded",
            ErrorCode::Overloaded,
        ),
        (
            V5DaemonErrorCodeFixture::OwnerCapacity,
            "owner_capacity",
            ErrorCode::OwnerCapacity,
        ),
        (
            V5DaemonErrorCodeFixture::ReceiptNotFound,
            "receipt_not_found",
            ErrorCode::ReceiptNotFound,
        ),
        (
            V5DaemonErrorCodeFixture::ReceiptExpired,
            "receipt_expired",
            ErrorCode::ReceiptExpired,
        ),
        (
            V5DaemonErrorCodeFixture::ReceiptCapacity,
            "receipt_capacity",
            ErrorCode::ReceiptCapacity,
        ),
        (
            V5DaemonErrorCodeFixture::TombstoneCapacity,
            "tombstone_capacity",
            ErrorCode::TombstoneCapacity,
        ),
        (
            V5DaemonErrorCodeFixture::InvocationIdentityMismatch,
            "invocation_identity_mismatch",
            ErrorCode::InvocationIdentityMismatch,
        ),
        (
            V5DaemonErrorCodeFixture::TaskNotFound,
            "task_not_found",
            ErrorCode::TaskNotFound,
        ),
        (
            V5DaemonErrorCodeFixture::TaskExpired,
            "task_expired",
            ErrorCode::TaskExpired,
        ),
        (
            V5DaemonErrorCodeFixture::StoreFailed,
            "store_failed",
            ErrorCode::StoreFailed,
        ),
        (
            V5DaemonErrorCodeFixture::DurabilityUncertain,
            "durability_uncertain",
            ErrorCode::DurabilityUncertain,
        ),
    ];
    for (code, wire, _) in error_code_cases {
        actions.push(probe_action(
            &format!("v5-error-{wire}"),
            ProtocolVersion::V5,
            ProtocolVersion::V5,
            ProtocolMessage::ErrorCodeFrame { code },
        ));
    }
    actions.extend([
        Action::ProbeProtocol {
            client: ProtocolVersion::V5,
            server: ProtocolVersion::V5,
            message: ProtocolMessage::MaximumResponseFrame,
            label: "v5-max-response-frame".to_string(),
        },
        Action::ProbeProtocol {
            client: ProtocolVersion::V5,
            server: ProtocolVersion::V5,
            message: ProtocolMessage::OversizedResponseFrame,
            label: "v5-oversized-response-frame".to_string(),
        },
    ]);

    let expected_protocol_count = actions.len();
    let report = execute(Scenario::fake(actions));
    assert_eq!(report.protocol.len(), expected_protocol_count);
    assert!(count_event(&report, EventKind::V5ReceiptRuntimeEntered) > 0);
    assert!(count_event(&report, EventKind::CanonicalV13ServiceEntered) > 0);
    report
        .protocol
        .iter()
        .for_each(assert_spawned_daemon_dispatch);
    for (label, _, request_kind, response_kind) in v5_cases {
        let matches: Vec<_> = report
            .protocol
            .iter()
            .filter(|probe| probe.label == label)
            .collect();
        assert_eq!(matches.len(), 1, "protocol label must occur exactly once");
        let probe = matches[0];
        assert_eq!(probe.client, ProtocolVersion::V5);
        assert_eq!(probe.server, ProtocolVersion::V5);
        assert_eq!(probe.error, None);
        assert_protocol_event_trace(probe, true, true);
        let client_write =
            protocol_jsonl_frame(&probe.client_write_frame_hex, MAX_PROTOCOL_FRAME_BYTES);
        let server_read = protocol_jsonl_frame(
            probe
                .server_read_frame_hex
                .as_deref()
                .expect("accepted frame must reach the production server decoder"),
            MAX_PROTOCOL_FRAME_BYTES,
        );
        let server_write =
            protocol_jsonl_frame(&probe.server_write_frame_hex, MAX_PROTOCOL_FRAME_BYTES);
        let client_read =
            protocol_jsonl_frame(&probe.client_read_frame_hex, MAX_PROTOCOL_FRAME_BYTES);
        assert_eq!(client_write, server_read);
        assert_eq!(server_write, client_read);
        assert_exact_v5_request(&client_write, request_kind);
        assert_exact_v5_server_kind(&server_write, response_kind);
        assert_eq!(probe.protocol_identity, "unica-daemon-jsonl-5");
        assert_eq!(probe.state_selector, ProtocolStateSelector::ReceiptV5);
        assert_safe_fingerprint(&probe.state_fingerprint);
    }
    let v3_default = observed_protocol(&report, "v3-default-guard");
    assert_eq!(v3_default.error, None);
    assert_eq!(v3_default.protocol_identity, "unica-daemon-jsonl-3");
    assert_eq!(v3_default.state_selector, ProtocolStateSelector::ProtocolV3);
    assert_protocol_event_trace(v3_default, true, true);
    let v3_service = v3_default
        .service_capability_fingerprint
        .as_ref()
        .expect("v3 submit must expose the real CanonicalV13 service seam");
    let v5_service = observed_protocol(&report, "v5-submit")
        .service_capability_fingerprint
        .as_ref()
        .expect("v5 submit must reach the same service after durable reserve");
    assert_safe_fingerprint(v3_service);
    assert_eq!(v3_service, v5_service);
    let arbitrary_core = v3_default
        .presented_core_identity_digest
        .as_ref()
        .expect("v3 guard must present an arbitrary canonical CoreIdentity digest");
    assert_ne!(
        arbitrary_core,
        &v3_default.production_v5_core_identity_digest
    );
    assert_eq!(
        observed_protocol(&report, "v5-submit")
            .presented_core_identity_digest
            .as_ref(),
        Some(&observed_protocol(&report, "v5-submit").production_v5_core_identity_digest)
    );
    for (label, _, expected) in invocation_outcome_cases {
        let probe = observed_protocol(&report, label);
        assert_eq!(probe.error, None);
        assert_protocol_event_trace(probe, true, true);
        let server = protocol_jsonl_frame(&probe.server_write_frame_hex, MAX_PROTOCOL_FRAME_BYTES);
        assert_exact_v5_server_kind(&server, "invocation");
        assert_invocation_response_frame(&server, expected);
    }
    for (label, client, server, _) in incompatible_cases {
        let matches: Vec<_> = report
            .protocol
            .iter()
            .filter(|probe| probe.label == label)
            .collect();
        assert_eq!(matches.len(), 1, "protocol label must occur exactly once");
        let probe = matches[0];
        assert_eq!(probe.client, client);
        assert_eq!(probe.server, server);
        assert_eq!(probe.error, Some(ErrorCode::ProtocolMismatch));
        assert_protocol_event_trace(probe, false, false);
        assert!(probe.server_read_frame_hex.is_none());
        let error = protocol_jsonl_frame(&probe.server_write_frame_hex, MAX_PROTOCOL_FRAME_BYTES);
        assert_eq!(
            error,
            protocol_jsonl_frame(&probe.client_read_frame_hex, MAX_PROTOCOL_FRAME_BYTES)
        );
        assert_eq!(frame_kind(&error), "error");
        assert_exact_v5_server_kind(&error, "error");
        assert_eq!(json_string(&error, "code"), "protocol_mismatch");
        assert_safe_fingerprint(&probe.state_fingerprint);
        let (selector, identity) = match server {
            ProtocolVersion::V3 => (ProtocolStateSelector::ProtocolV3, "unica-daemon-jsonl-3"),
            ProtocolVersion::V5 => (ProtocolStateSelector::ReceiptV5, "unica-daemon-jsonl-5"),
            ProtocolVersion::V4 => {
                panic!("v4 cannot be selected as a production daemon protocol")
            }
        };
        assert_eq!(probe.state_selector, selector);
        assert_eq!(probe.protocol_identity, identity);
    }
    for (label, expected) in [
        ("v5-direct-completed", ExpectedProjection::Completed(true)),
        (
            "v5-direct-semantic-completed",
            ExpectedProjection::Completed(false),
        ),
        ("v5-direct-cancelled", ExpectedProjection::Cancelled),
    ] {
        let probe = observed_protocol(&report, label);
        assert_eq!(probe.error, None);
        assert_protocol_event_trace(probe, true, true);
        assert_direct_projection(probe, expected);
    }
    for (label, expected, owner) in [
        ("v5-task-queued", ExpectedProjection::Queued, None),
        ("v5-task-working", ExpectedProjection::Working, None),
        (
            "v5-task-completed",
            ExpectedProjection::Completed(true),
            None,
        ),
        ("v5-task-cancelled", ExpectedProjection::Cancelled, None),
        (
            "v5-task-semantic-receipt-backed",
            ExpectedProjection::Completed(false),
            Some(TaskTerminalOwnerFixture::ReceiptBacked),
        ),
        (
            "v5-task-semantic-bound",
            ExpectedProjection::Completed(false),
            Some(TaskTerminalOwnerFixture::Bound),
        ),
        (
            "v5-task-semantic-staged",
            ExpectedProjection::Completed(false),
            Some(TaskTerminalOwnerFixture::Staged),
        ),
    ] {
        let probe = observed_protocol(&report, label);
        assert_eq!(probe.error, None);
        assert_protocol_event_trace(probe, true, true);
        assert_task_projection(probe, expected, owner);
    }
    for case in FAILURE_PROJECTION_CASES {
        let direct = observed_protocol(&report, &format!("v5-direct-failure-{}", case.suffix));
        assert_eq!(direct.error, None);
        assert_protocol_event_trace(direct, true, true);
        assert_direct_projection(direct, ExpectedProjection::Failed(case));

        let task = observed_protocol(&report, &format!("v5-task-failure-{}", case.suffix));
        assert_eq!(task.error, None);
        assert_protocol_event_trace(task, true, true);
        assert_task_projection(task, ExpectedProjection::Failed(case), None);

        let v3 = observed_protocol(
            &report,
            &format!(
                "v3-{}-failure-{}",
                if case.introduced_in_v5 {
                    "rejects-v5"
                } else {
                    "accepts-legacy"
                },
                case.suffix
            ),
        );
        if case.introduced_in_v5 {
            assert_eq!(v3.error, Some(ErrorCode::InvalidRequest));
            assert_protocol_event_trace(v3, true, false);
            assert_eq!(
                frame_kind(&protocol_jsonl_frame(
                    &v3.server_write_frame_hex,
                    MAX_PROTOCOL_FRAME_BYTES
                )),
                "error"
            );
            let stored = observed_protocol(
                &report,
                &format!("v3-schema-v2-rejects-v5-record-{}", case.suffix),
            );
            assert_eq!(stored.protocol_identity, "unica-daemon-jsonl-3");
            assert_eq!(stored.state_selector, ProtocolStateSelector::ProtocolV3);
            assert_eq!(stored.error, Some(ErrorCode::InvalidRequest));
            assert_protocol_event_trace(stored, true, false);
            assert_eq!(
                frame_kind(&protocol_jsonl_frame(
                    &stored.server_write_frame_hex,
                    MAX_PROTOCOL_FRAME_BYTES
                )),
                "error"
            );
        } else {
            assert_eq!(v3.error, None);
            assert_protocol_event_trace(v3, true, true);
            let frame = protocol_jsonl_frame(&v3.server_write_frame_hex, MAX_PROTOCOL_FRAME_BYTES);
            assert_eq!(
                find_json_field(&frame, "code").and_then(serde_json::Value::as_str),
                Some(case.code)
            );
        }
    }
    for (label, target) in strict_cases {
        let unknown = observed_protocol(&report, label);
        assert_eq!(unknown.error, Some(ErrorCode::InvalidRequest));
        assert_protocol_event_trace(unknown, true, false);
        let client_write =
            protocol_jsonl_frame(&unknown.client_write_frame_hex, MAX_PROTOCOL_FRAME_BYTES);
        assert_strict_schema_mutation(&client_write, target);
        let server_read = protocol_jsonl_frame(
            unknown
                .server_read_frame_hex
                .as_deref()
                .expect("strict schema mutation must reach the selected production decoder"),
            MAX_PROTOCOL_FRAME_BYTES,
        );
        assert_eq!(
            server_read, client_write,
            "production decoder must inspect the exact test-owned malformed frame"
        );
        assert_eq!(
            protocol_jsonl_frame(&unknown.server_write_frame_hex, MAX_PROTOCOL_FRAME_BYTES),
            protocol_jsonl_frame(&unknown.client_read_frame_hex, MAX_PROTOCOL_FRAME_BYTES)
        );
        assert_exact_v5_server_kind(
            &protocol_jsonl_frame(&unknown.server_write_frame_hex, MAX_PROTOCOL_FRAME_BYTES),
            "error",
        );
    }
    for (_, wire, expected) in error_code_cases {
        let probe = observed_protocol(&report, &format!("v5-error-{wire}"));
        assert_eq!(probe.error, Some(expected));
        assert_protocol_event_trace(probe, true, false);
        let error = protocol_jsonl_frame(&probe.server_write_frame_hex, MAX_PROTOCOL_FRAME_BYTES);
        assert_exact_v5_server_kind(&error, "error");
        assert_eq!(json_string(&error, "code"), wire);
        assert_eq!(
            error,
            protocol_jsonl_frame(&probe.client_read_frame_hex, MAX_PROTOCOL_FRAME_BYTES)
        );
    }
    let maximum = report
        .protocol
        .iter()
        .find(|probe| probe.label == "v5-max-response-frame")
        .expect("maximum response frame probe");
    assert_eq!(maximum.error, None);
    assert_eq!(
        maximum.server_write_frame_hex.len() / 2,
        MAX_PROTOCOL_FRAME_BYTES
    );
    let maximum_frame =
        protocol_jsonl_frame(&maximum.server_write_frame_hex, MAX_PROTOCOL_FRAME_BYTES);
    assert!(maximum_frame.is_object());
    assert_protocol_event_trace(maximum, true, true);
    let oversized = report
        .protocol
        .iter()
        .find(|probe| probe.label == "v5-oversized-response-frame")
        .expect("oversized response frame probe");
    assert_eq!(oversized.error, Some(ErrorCode::InvalidRequest));
    assert!(oversized.server_write_frame_hex.len() / 2 <= MAX_PROTOCOL_FRAME_BYTES);
    let oversized_frame =
        protocol_jsonl_frame(&oversized.server_write_frame_hex, MAX_PROTOCOL_FRAME_BYTES);
    assert_exact_v5_server_kind(&oversized_frame, "error");
    assert_eq!(json_string(&oversized_frame, "code"), "invalid_request");
    assert_protocol_event_trace(oversized, true, false);
    let mut selector_fingerprints = Vec::new();
    for selector in [
        ProtocolStateSelector::ProtocolV3,
        ProtocolStateSelector::ReceiptV5,
    ] {
        let mut fingerprints: Vec<_> = report
            .protocol
            .iter()
            .filter(|probe| probe.state_selector == selector)
            .map(|probe| probe.state_fingerprint.clone())
            .collect();
        fingerprints.sort();
        fingerprints.dedup();
        assert_eq!(
            fingerprints.len(),
            1,
            "each state selector must expose exactly one fingerprint"
        );
        selector_fingerprints.push(fingerprints.remove(0));
    }
    assert_ne!(selector_fingerprints[0], selector_fingerprints[1]);
}

#[test]
fn receipt_key_is_canonicalized_identically_by_client_and_server() {
    let report = execute(Scenario::fake(vec![Action::CompareClientServerIdentity]));
    let identity = report
        .identity
        .as_ref()
        .unwrap_or_else(|| panic!("HARNESS FAILURE: missing identity evidence"));
    assert_eq!(identity.client_key, identity.daemon_key);
    assert_ne!(
        identity.caller_claimed_key_digest,
        identity.daemon_key.key_digest
    );
    assert_safe_fingerprint(&identity.caller_claimed_key_digest);
    assert_receipt_key(&identity.client_key);
    let vector = &identity.frozen_vector_key;
    assert_eq!(vector.invocation_id, "123e4567-e89b-42d3-a456-426614174000");
    assert_eq!(
        vector.reserved_task_id,
        "123e4567-e89b-42d3-b456-426614174001"
    );
    assert_eq!(vector.core_identity_digest, "00".repeat(32));
    assert_eq!(vector.tool, ToolIdentityObservation::View);
    assert_eq!(vector.normalized_arguments_hash, "11".repeat(32));
    assert_eq!(
        vector.request_scope_hash.0,
        "9f7a5a77bb6eb469cd20147a9aeee9d9769a8372f587bd89635d15684ee02b39"
    );
    assert_eq!(
        vector.request_scope_hash.0,
        request_scope_hash_for_test("workspace-a")
    );
    assert_eq!(
        vector.key_digest,
        "9d8f104e7dfb2f4827a24d4b41aefe6c6704bf31bd3df191d84f9893071db549"
    );
    assert_eq!(
        vector.key_digest,
        receipt_key_digest_for_test(
            &vector.invocation_id,
            &vector.reserved_task_id,
            &vector.core_identity_digest,
            tool_wire_name(vector.tool),
            &vector.normalized_arguments_hash,
            &vector.request_scope_hash.0,
        )
    );
    let link = &identity.frozen_task_link_vector;
    assert_eq!(link.receipt_key_digest, "0".repeat(64));
    assert_eq!(link.task_id, "11111111-1111-4111-8111-111111111111");
    assert_eq!(link.invocation_id, "22222222-2222-4222-8222-222222222222");
    assert_eq!(link.workspace_identity_hash, "a".repeat(64));
    assert_eq!(
        link.task_link_digest,
        "4c73d08219973c72e759a9f85e156fa42c9d8e61a56e704b70d1c7c042b73da0"
    );
    assert_eq!(
        link.task_link_digest,
        task_link_digest_for_test(
            &link.receipt_key_digest,
            &link.task_id,
            &link.invocation_id,
            &link.workspace_identity_hash,
        )
    );
}

#[test]
fn response_budget_is_not_receipt_identity() {
    let report = execute(Scenario::fake(vec![
        direct_provider(),
        Action::InstallBarrier {
            point: BarrierPoint::BeforePrepare,
        },
        Action::Submit {
            request: RequestCase::Canonical,
            response_budget_ms: CUTOFF_MS,
            disconnect: DisconnectPoint::Never,
            label: "original".to_string(),
        },
        Action::WaitForEvent {
            event: EventKind::ReceiptBegunCommitted,
        },
        Action::Submit {
            request: RequestCase::SameIdentity,
            response_budget_ms: 6_000,
            disconnect: DisconnectPoint::Never,
            label: "retry".to_string(),
        },
        checkpoint_action("duplicate-live"),
        Action::ReleaseBarrier {
            point: BarrierPoint::BeforePrepare,
        },
        Action::WaitForEvent {
            event: EventKind::ReceiptTerminalCommitted,
        },
        checkpoint_action("after-retry"),
    ]));

    let live = checkpoint(&report, "duplicate-live");
    let receipt = only_receipt(live);
    assert_eq!(receipt.original_budget_ms, CUTOFF_MS);
    assert_eq!(live.receipt_live_count, 1);
    let original = response(&report, "original");
    let retry = response(&report, "retry");
    assert_exact_response_identity(original, receipt);
    assert_exact_response_identity(retry, receipt);
    assert_eq!(response_key(original), response_key(retry));
    assert_eq!(original.cutoff_epoch_ms, retry.cutoff_epoch_ms);
    assert_eq!(original.original_budget_ms, Some(CUTOFF_MS));
    assert_eq!(retry.original_budget_ms, Some(CUTOFF_MS));
    assert_eq!(count_event(&report, EventKind::ReceiptReserved), 1);
    assert_eq!(count_event(&report, EventKind::ReceiptBegunCommitted), 1);
    let terminal = checkpoint(&report, "after-retry");
    assert_eq!(terminal.callbacks.execute, 1);
    let terminal_receipt = only_receipt(terminal);
    assert_publication_matches_snapshot(
        &report,
        terminal,
        &terminal_receipt.key,
        terminal_of_receipt(terminal_receipt),
        TerminalPublicationOwner::DirectReceiptLedger,
    );
}

#[test]
fn exact_duplicate_preserves_cutoff_without_second_domain_callback() {
    let report = execute(Scenario::fake(vec![
        direct_provider(),
        submit("original"),
        Action::Submit {
            request: RequestCase::SameIdentity,
            response_budget_ms: 1_000,
            disconnect: DisconnectPoint::Never,
            label: "duplicate".to_string(),
        },
        checkpoint_action("after-duplicate"),
    ]));

    let original = response(&report, "original");
    let duplicate = response(&report, "duplicate");
    let after = checkpoint(&report, "after-duplicate");
    let receipt = only_receipt(after);
    assert_exact_response_identity(original, receipt);
    assert_exact_response_identity(duplicate, receipt);
    assert_eq!(response_key(original), response_key(duplicate));
    assert_eq!(original.cutoff_epoch_ms, duplicate.cutoff_epoch_ms);
    assert_eq!(original.original_budget_ms, duplicate.original_budget_ms);
    assert_eq!(response_terminal(original), response_terminal(duplicate));
    assert_eq!(original.kind, ResponseKind::Direct);
    assert_eq!(duplicate.kind, ResponseKind::Direct);
    assert!(completed_result(response_terminal(duplicate)).ok);
    assert!(!terminal_payload_hex(response_terminal(duplicate)).is_empty());
    assert_safe_fingerprint(terminal_digest(response_terminal(duplicate)));
    assert_eq!(after.callbacks.prepare, 1);
    assert_eq!(after.callbacks.execute, 1);
    assert_eq!(after.receipt_live_count, 1);
    let publication = assert_publication_matches_snapshot(
        &report,
        after,
        &receipt.key,
        terminal_of_receipt(receipt),
        TerminalPublicationOwner::DirectReceiptLedger,
    );
    let direct_frames: Vec<_> = publication
        .response_frames
        .iter()
        .filter(|frame| frame.response_kind == ResponseKind::Direct)
        .collect();
    assert_eq!(
        direct_frames.len(),
        2,
        "duplicate writes need fresh preflight"
    );
    assert_ne!(
        direct_frames[0].prepared_sequence,
        direct_frames[1].prepared_sequence
    );
    assert_eq!(
        direct_frames[0].response_jsonl.raw_hex, direct_frames[1].response_jsonl.raw_hex,
        "response-budget changes cannot perturb the stored identity response"
    );
}

#[test]
fn submit_disconnect_before_cutoff_preserves_direct_lifecycle() {
    let report = execute(Scenario::fake(vec![
        direct_provider(),
        Action::Submit {
            request: RequestCase::Canonical,
            response_budget_ms: CUTOFF_MS,
            disconnect: DisconnectPoint::AfterSubmitWrite,
            label: "disconnected".to_string(),
        },
        Action::AdvanceMonotonic { millis: 1_000 },
        Action::Recover {
            key: KeyCase::Exact,
            label: "recovered".to_string(),
        },
        checkpoint_action("after-recover"),
    ]));

    let recovered = response(&report, "recovered");
    assert_eq!(recovered.kind, ResponseKind::RecoveredDirect);
    let after = checkpoint(&report, "after-recover");
    let receipt = only_receipt(after);
    assert_exact_response_identity(recovered, receipt);
    assert_eq!(receipt.state, SeedReceiptState::DirectTerminalUnacked);
    assert!(after.tasks.is_empty());
    assert_eq!(count_event(&report, EventKind::UnboundPromiseCommitted), 0);
    assert_eq!(count_event(&report, EventKind::BoundHandoffCommitted), 0);
    assert_eq!(after.callbacks.prepare, 1);
    assert_eq!(after.callbacks.execute, 1);
    let publication = assert_publication_matches_snapshot(
        &report,
        after,
        &receipt.key,
        terminal_of_receipt(receipt),
        TerminalPublicationOwner::DirectReceiptLedger,
    );
    let recovered_frames: Vec<_> = publication
        .response_frames
        .iter()
        .filter(|frame| frame.response_kind == ResponseKind::RecoveredDirect)
        .collect();
    assert_eq!(recovered_frames.len(), 1);
}

#[test]
fn receipt_identity_mismatches_reject_without_mutating_original() {
    let fields = [
        IdentityField::InvocationId,
        IdentityField::ReservedTaskId,
        IdentityField::CoreIdentity,
        IdentityField::ToolIdentity,
        IdentityField::NormalizedArgumentsHash,
        IdentityField::RequestScopeHash,
    ];
    let mut actions = vec![
        direct_provider(),
        submit("original"),
        checkpoint_action("original"),
    ];
    for field in fields {
        actions.push(Action::Submit {
            request: RequestCase::Mismatch(field),
            response_budget_ms: 6_000,
            disconnect: DisconnectPoint::Never,
            label: format!("mismatch-{field:?}").to_lowercase(),
        });
        actions.push(checkpoint_action(
            &format!("after-{field:?}").to_lowercase(),
        ));
    }
    actions.push(checkpoint_action("after-mismatches"));
    let report = execute(Scenario::fake(actions));
    let original = checkpoint(&report, "original");
    let original_mutation = only_receipt(original).mutation_sequence;
    let original_callbacks = original.callbacks;
    for field in fields {
        let response_label = format!("mismatch-{field:?}").to_lowercase();
        let checkpoint_label = format!("after-{field:?}").to_lowercase();
        let mismatch = response(&report, &response_label);
        assert_eq!(mismatch.kind, ResponseKind::Rejected);
        assert_eq!(mismatch.error, Some(ErrorCode::InvocationIdentityMismatch));
        let after_case = checkpoint(&report, &checkpoint_label);
        assert_eq!(
            only_receipt(after_case).mutation_sequence,
            original_mutation
        );
        assert_eq!(after_case.callbacks, original_callbacks);
        assert_eq!(after_case.invocation_index, original.invocation_index);
        assert_eq!(after_case.reserved_task_index, original.reserved_task_index);
    }
    let after = checkpoint(&report, "after-mismatches");
    assert_eq!(after.receipt_live_count, 1);
    assert_eq!(after.callbacks.prepare, 1);
    assert_eq!(after.callbacks.execute, 1);

    for index in [IdentityIndex::InvocationId, IdentityIndex::ReservedTaskId] {
        let collision = execute(Scenario::fake(vec![
            Action::InjectPersistedIdentityCollision { index },
            checkpoint_action("collision-before-open"),
            Action::OpenTaskStoreInspectOnly,
            Action::ReconcileStartup,
            checkpoint_action("collision-after-open"),
        ]));
        let before_open = corrupted_checkpoint(&collision, "collision-before-open");
        let after_open = corrupted_checkpoint(&collision, "collision-after-open");
        assert_eq!(before_open.receipts, after_open.receipts);
        assert_eq!(before_open.tombstones, after_open.tombstones);
        assert_eq!(before_open.task_links, after_open.task_links);
        assert_eq!(before_open.store_generation, after_open.store_generation);
        assert_eq!(after_open.listener, ListenerState::NotPublished);
        assert!(after_open.restart_requested);
        assert!(!after_open.daemon_running);
    }
}

#[test]
fn lost_direct_transport_recovers_durable_terminal_before_ack() {
    let report = execute(Scenario::fake(vec![
        direct_provider(),
        Action::Submit {
            request: RequestCase::Canonical,
            response_budget_ms: CUTOFF_MS,
            disconnect: DisconnectPoint::AfterTerminalCommit,
            label: "lost".to_string(),
        },
        Action::Recover {
            key: KeyCase::Exact,
            label: "live-recover".to_string(),
        },
        Action::Restart,
        Action::Recover {
            key: KeyCase::Exact,
            label: "restart-recover".to_string(),
        },
        checkpoint_action("after-restart-recover"),
    ]));

    let live = response(&report, "live-recover");
    let restarted = response(&report, "restart-recover");
    assert_eq!(live.kind, ResponseKind::RecoveredDirect);
    assert_eq!(restarted.kind, ResponseKind::RecoveredDirect);
    assert!(!terminal_payload_hex(response_terminal(live)).is_empty());
    assert_safe_fingerprint(terminal_digest(response_terminal(live)));
    assert_eq!(response_terminal(live), response_terminal(restarted));
    assert_eq!(response_key(live), response_key(restarted));
    let after = checkpoint(&report, "after-restart-recover");
    let receipt = only_receipt(after);
    assert_exact_response_identity(live, receipt);
    assert_exact_response_identity(restarted, receipt);
    assert_eq!(after.callbacks.prepare, 1);
    assert_eq!(after.callbacks.execute, 1);
    let publication = assert_publication_matches_snapshot(
        &report,
        after,
        &receipt.key,
        terminal_of_receipt(receipt),
        TerminalPublicationOwner::DirectReceiptLedger,
    );
    let recovered_frames: Vec<_> = publication
        .response_frames
        .iter()
        .filter(|frame| frame.response_kind == ResponseKind::RecoveredDirect)
        .collect();
    assert_eq!(recovered_frames.len(), 2);
    assert_ne!(
        recovered_frames[0].prepared_sequence,
        recovered_frames[1].prepared_sequence
    );
    assert_ne!(
        recovered_frames[0].write_sequence,
        recovered_frames[1].write_sequence
    );
    assert_eq!(
        recovered_frames[0].response_jsonl.raw_hex, recovered_frames[1].response_jsonl.raw_hex,
        "live and post-restart recovery must rebuild byte-equivalent frames"
    );
}

#[test]
fn crash_after_begun_returns_outcome_uncertain_without_replay() {
    let report = execute(Scenario::fake(vec![
        direct_provider(),
        Action::InstallBarrier {
            point: BarrierPoint::BeforePrepare,
        },
        submit("submit"),
        Action::WaitForEvent {
            event: EventKind::ReceiptBegunCommitted,
        },
        Action::Crash {
            point: CrashPoint::ReservedBegun,
        },
        Action::Restart,
        Action::Recover {
            key: KeyCase::Exact,
            label: "recover".to_string(),
        },
        checkpoint_action("recovered"),
    ]));

    let recovered = checkpoint(&report, "recovered");
    assert_failed_terminal(
        terminal_of_receipt(only_receipt(recovered)),
        V5SafeFailureReason::OutcomeUncertain,
    );
    assert_eq!(
        response(&report, "recover").error,
        Some(ErrorCode::OutcomeUncertain)
    );
    assert_eq!(count_event(&report, EventKind::ReceiptBegunCommitted), 1);
    assert_eq!(recovered.callbacks.prepare, 0);
    assert_eq!(recovered.callbacks.execute, 0);
}

#[test]
fn side_effect_before_terminal_returns_outcome_uncertain_without_replay() {
    let report = execute(Scenario::fake(vec![
        Action::ConfigureProvider {
            execution_class: ExecutionClass::Direct,
            terminal: success_payload(),
            cooperative_cancel: true,
            side_effect_marker: true,
        },
        Action::Crash {
            point: CrashPoint::AfterSideEffectBeforeTerminal,
        },
        submit("submit"),
        Action::Restart,
        Action::Recover {
            key: KeyCase::Exact,
            label: "recover".to_string(),
        },
        checkpoint_action("recovered"),
    ]));

    let recovered = checkpoint(&report, "recovered");
    assert_eq!(recovered.side_effect_markers, 1);
    assert_eq!(recovered.callbacks.execute, 1);
    assert_failed_terminal(
        terminal_of_receipt(only_receipt(recovered)),
        V5SafeFailureReason::OutcomeUncertain,
    );
    assert_eq!(count_event(&report, EventKind::ExecuteEntered), 1);
}

#[test]
fn acknowledge_receipt_compacts_to_bounded_tombstone() {
    let report = execute(Scenario::fake(vec![
        direct_provider(),
        Action::SeedReceipt {
            state: SeedReceiptState::ReservedUnbound,
            cancel_requested: false,
            staged_terminal: None,
        },
        checkpoint_action("premature-before"),
        Action::Acknowledge {
            key: KeyCase::Exact,
            digest: DigestCase::WellFormedCandidate,
            disconnect: AckDisconnectPoint::Never,
            label: "premature-ack".to_string(),
        },
        checkpoint_action("premature-after"),
        Action::Reset,
        direct_provider(),
        submit("original"),
        Action::WaitForEvent {
            event: EventKind::FinalResultProjected,
        },
        checkpoint_action("terminal-before-ack"),
        Action::Acknowledge {
            key: KeyCase::Exact,
            digest: DigestCase::Mismatched,
            disconnect: AckDisconnectPoint::Never,
            label: "mismatched-ack".to_string(),
        },
        checkpoint_action("after-mismatched-ack"),
        Action::Acknowledge {
            key: KeyCase::Exact,
            digest: DigestCase::ExactTerminal,
            disconnect: AckDisconnectPoint::AfterTombstoneCommit,
            label: "lost-ack-response".to_string(),
        },
        checkpoint_action("after-lost-ack-response"),
        Action::Acknowledge {
            key: KeyCase::Exact,
            digest: DigestCase::ExactTerminal,
            disconnect: AckDisconnectPoint::Never,
            label: "ack-retry".to_string(),
        },
        Action::Submit {
            request: RequestCase::SameIdentity,
            response_budget_ms: 6_000,
            disconnect: DisconnectPoint::Never,
            label: "duplicate".to_string(),
        },
        Action::AdvanceEpoch {
            millis: TOMBSTONE_TTL_MS - 1,
        },
        checkpoint_action("before-expiry"),
        Action::AdvanceEpoch { millis: 1 },
        checkpoint_action("at-expiry"),
    ]));

    assert_eq!(
        response(&report, "premature-ack").kind,
        ResponseKind::Rejected
    );
    assert_eq!(
        response(&report, "premature-ack").error,
        Some(ErrorCode::InvalidRequest)
    );
    assert_eq!(
        checkpoint(&report, "premature-before").receipts,
        checkpoint(&report, "premature-after").receipts
    );
    assert_eq!(
        checkpoint(&report, "premature-after")
            .callbacks
            .total_domain(),
        0
    );
    assert_eq!(
        response(&report, "mismatched-ack").kind,
        ResponseKind::Rejected
    );
    assert_eq!(
        response(&report, "mismatched-ack").error,
        Some(ErrorCode::InvalidRequest)
    );
    let after_mismatch = checkpoint(&report, "after-mismatched-ack");
    assert_eq!(
        checkpoint(&report, "terminal-before-ack").receipts,
        after_mismatch.receipts
    );
    let original_response = response(&report, "original");
    let original_terminal = only_receipt(after_mismatch);
    assert_exact_response_identity(original_response, original_terminal);
    let original_key = response_key(original_response).clone();
    let original_digest = terminal_digest(response_terminal(original_response));
    assert_safe_fingerprint(original_digest);
    assert_eq!(
        terminal_digest(terminal_of_receipt(original_terminal)),
        original_digest
    );
    assert_eq!(
        only_receipt(after_mismatch).state,
        SeedReceiptState::DirectTerminalUnacked
    );
    assert!(!terminal_payload_hex(terminal_of_receipt(original_terminal)).is_empty());
    assert_eq!(
        response(&report, "ack-retry").kind,
        ResponseKind::Acknowledged
    );
    assert_eq!(response(&report, "duplicate").kind, ResponseKind::Tombstone);
    let after_lost = checkpoint(&report, "after-lost-ack-response");
    assert!(after_lost.receipts.is_empty());
    assert_eq!(after_lost.tombstones.len(), 1);
    let first_tombstone = &after_lost.tombstones[0];
    assert_eq!(first_tombstone.key, original_key);
    assert_eq!(first_tombstone.terminal_digest, original_digest);
    let retry = response(&report, "ack-retry");
    let retry_ack = retry
        .acknowledgement
        .as_ref()
        .expect("idempotent ACK retry must expose first-ACK evidence");
    assert_eq!(retry_ack.ack_epoch_ms, first_tombstone.ack_epoch_ms);
    assert_eq!(retry_ack.terminal_digest, first_tombstone.terminal_digest);
    let before_expiry = checkpoint(&report, "before-expiry");
    assert!(before_expiry.receipts.is_empty());
    let tombstone = &before_expiry.tombstones[0];
    assert_eq!(tombstone.key, original_key);
    assert_eq!(tombstone.terminal_digest, original_digest);
    assert_eq!(tombstone.ack_epoch_ms, first_tombstone.ack_epoch_ms);
    assert_eq!(tombstone.expires_epoch_ms, before_expiry.epoch_ms + 1);
    assert_eq!(before_expiry.tombstone_count, 1);
    assert_eq!(before_expiry.callbacks.execute, 1);
    let duplicate = response(&report, "duplicate");
    assert_eq!(response_key(duplicate), &original_key);
    assert_eq!(duplicate.original_budget_ms, None);
    assert_eq!(duplicate.cutoff_epoch_ms, None);
    assert_eq!(duplicate.terminal, None);
    let duplicate_ack = duplicate
        .acknowledgement
        .as_ref()
        .expect("tombstone response must expose ACK epoch and terminal digest");
    assert_eq!(duplicate_ack.ack_epoch_ms, tombstone.ack_epoch_ms);
    assert_eq!(duplicate_ack.terminal_digest, tombstone.terminal_digest);
    assert_event_order(
        &report,
        &[
            EventKind::ResultSerialized,
            EventKind::ReceiptTerminalCommitted,
            EventKind::FinalResultProjected,
            EventKind::AcknowledgementCommitted,
        ],
    );
    let expired = checkpoint(&report, "at-expiry");
    assert_eq!(expired.tombstone_count, 0);
    assert!(expired.receipts.is_empty());
}

#[test]
fn cutoff_during_admission_projects_exact_unbound_task() {
    let report = execute(Scenario::fake(vec![
        direct_provider(),
        Action::InstallBarrier {
            point: BarrierPoint::AdmissionEntered,
        },
        submit("submit"),
        Action::WaitForEvent {
            event: EventKind::AdmissionEntered,
        },
        Action::AdvanceMonotonic {
            millis: CUTOFF_MS - 1,
        },
        checkpoint_action("one-ms-before-cutoff"),
        Action::AdvanceMonotonic { millis: 1 },
        Action::ReadTask {
            api: TaskApi::NativeGet,
            label: "promised".to_string(),
        },
        checkpoint_action("at-cutoff"),
        Action::ReleaseBarrier {
            point: BarrierPoint::AdmissionEntered,
        },
    ]));

    let before_cutoff = checkpoint(&report, "one-ms-before-cutoff");
    assert_eq!(
        only_receipt(before_cutoff).state,
        SeedReceiptState::ReservedUnbound
    );
    assert!(before_cutoff.tasks.is_empty());
    let cutoff = checkpoint(&report, "at-cutoff");
    let receipt = only_receipt(cutoff);
    assert_eq!(receipt.state, SeedReceiptState::TaskPromisedUnbound);
    assert!(receipt.bound_workspace_identity.is_none());
    assert!(
        cutoff.tasks.is_empty(),
        "TaskStore must remain empty before actor bind"
    );
    let task = task_read(&report, "promised");
    assert_eq!(task.status, TaskStatus::Queued);
    assert_eq!(task.projection_source, ProjectionSource::ReceiptLedger);
    assert!(task.workspace_identity_hash.is_none());
    assert_eq!(response(&report, "submit").kind, ResponseKind::Task);
    assert_exact_response_identity(response(&report, "submit"), receipt);
    let projected = response(&report, "submit")
        .task
        .as_ref()
        .expect("cutoff Task response must carry its stable projection");
    assert_eq!(projected, task);
    assert_eq!(projected.task_id, receipt.key.reserved_task_id);
    assert_eq!(projected.invocation_id, receipt.key.invocation_id);
    assert!(response(&report, "submit").latency_ms <= CUTOFF_MS + 125);
}

#[test]
fn unbound_promise_terminal_keeps_canonical_payload_until_task_ttl() {
    let report = execute(Scenario::fake(vec![
        direct_provider(),
        Action::InstallBarrier {
            point: BarrierPoint::AdmissionEntered,
        },
        submit("submit"),
        Action::WaitForEvent {
            event: EventKind::AdmissionEntered,
        },
        Action::AdvanceMonotonic { millis: CUTOFF_MS },
        Action::ReadTask {
            api: TaskApi::NativeGet,
            label: "cancel-promised".to_string(),
        },
        Action::Cancel {
            key: KeyCase::Exact,
            lazy_session: false,
            label: "cancel".to_string(),
        },
        Action::ReadTask {
            api: TaskApi::NativeGet,
            label: "first".to_string(),
        },
        checkpoint_action("terminal"),
        Action::AdvanceEpoch {
            millis: DIRECT_TASK_TTL_MS - 1,
        },
        Action::ReadTask {
            api: TaskApi::CompatibilityResult,
            label: "before-expiry".to_string(),
        },
        checkpoint_action("before-expiry"),
    ]));

    let terminal = checkpoint(&report, "terminal");
    let retained = checkpoint(&report, "before-expiry");
    assert_eq!(
        only_receipt(terminal).state,
        SeedReceiptState::TaskTerminalReceiptBacked
    );
    assert_cancelled_terminal(terminal_of_receipt(only_receipt(terminal)));
    assert_eq!(terminal.receipt_live_count, 1);
    assert_eq!(retained.receipt_live_count, 1);
    assert_eq!(
        terminal.receipt_reserved_bytes,
        retained.receipt_reserved_bytes
    );
    assert_max_result_entitlement(only_receipt(terminal));
    assert_max_result_entitlement(only_receipt(retained));
    assert_eq!(receipt_quota_bytes(terminal), MAX_RESPONSE_LINE_BYTES);
    assert_eq!(receipt_quota_bytes(retained), MAX_RESPONSE_LINE_BYTES);
    assert_eq!(
        task_read(&report, "first"),
        task_read(&report, "before-expiry")
    );
    let promised = task_read(&report, "cancel-promised");
    let cancelled = task_read(&report, "first");
    assert_eq!(promised.task_id, cancelled.task_id);
    assert_same_stable_task(promised, cancelled);
    assert_eq!(cancelled.status, TaskStatus::Cancelled);
    let cancelled_terminal = cancelled
        .terminal
        .as_ref()
        .expect("cancelled Task must expose the durable terminal");
    assert_cancelled_terminal(cancelled_terminal);
    assert_eq!(
        cancelled_terminal,
        terminal_of_receipt(only_receipt(terminal))
    );
    assert_eq!(
        cancelled.expires_epoch_ms,
        terminal_epoch_ms(cancelled_terminal) + DIRECT_TASK_TTL_MS
    );
}

#[test]
fn validation_rejection_after_promise_recovers_receipt_backed_terminal() {
    let cases = [
        (
            "validation",
            BarrierPoint::ValidationEntered,
            EventKind::ValidationEntered,
            Action::ConfigureValidation { reject: true },
            TerminalClass::Completed(false),
            false,
        ),
        (
            "admission-invalid",
            BarrierPoint::AdmissionEntered,
            EventKind::AdmissionEntered,
            Action::ConfigureAdmission {
                rejection: Some(WorkspaceAdmissionFailure::Invalid),
            },
            TerminalClass::Completed(false),
            false,
        ),
        (
            "admission-capacity",
            BarrierPoint::AdmissionEntered,
            EventKind::AdmissionEntered,
            Action::ConfigureAdmission {
                rejection: Some(WorkspaceAdmissionFailure::Capacity),
            },
            TerminalClass::Failed(V5SafeFailureReason::WorkspaceCapacity),
            false,
        ),
        (
            "admission-registry-failed",
            BarrierPoint::AdmissionEntered,
            EventKind::AdmissionEntered,
            Action::ConfigureAdmission {
                rejection: Some(WorkspaceAdmissionFailure::RegistryFailed),
            },
            TerminalClass::Failed(V5SafeFailureReason::WorkspaceRegistryFailed),
            true,
        ),
    ];
    for (case, barrier, event, rejection, expected_terminal, fail_stop) in cases {
        let mut actions = vec![
            direct_provider(),
            rejection,
            Action::InstallBarrier { point: barrier },
            submit("submit"),
            Action::WaitForEvent { event },
            Action::AdvanceMonotonic { millis: CUTOFF_MS },
            checkpoint_action("promised"),
            Action::ReleaseBarrier { point: barrier },
            Action::WaitForEvent {
                event: EventKind::ReceiptTerminalCommitted,
            },
            checkpoint_action("terminal-before-restart"),
        ];
        if fail_stop {
            actions.push(Action::Restart);
            actions.push(Action::ReadTask {
                api: TaskApi::NativeGet,
                label: "before-restart".to_string(),
            });
        } else {
            actions.push(Action::ReadTask {
                api: TaskApi::NativeGet,
                label: "before-restart".to_string(),
            });
            actions.push(Action::Restart);
        }
        actions.extend([
            Action::ReadTask {
                api: TaskApi::CompatibilityResult,
                label: "after-restart".to_string(),
            },
            checkpoint_action("reopened"),
            Action::AdvanceEpoch {
                millis: DIRECT_TASK_TTL_MS - 1,
            },
            Action::ReadTask {
                api: TaskApi::NativeWait,
                label: "before-full-ttl".to_string(),
            },
            checkpoint_action("before-full-ttl"),
        ]);
        let report = execute(Scenario::fake(actions));

        assert_eq!(
            only_receipt(checkpoint(&report, "promised")).state,
            SeedReceiptState::TaskPromisedUnbound,
            "{case}"
        );
        let reopened = checkpoint(&report, "reopened");
        assert!(reopened.tasks.is_empty(), "{case}");
        let receipt = only_receipt(reopened);
        assert_eq!(
            receipt.state,
            SeedReceiptState::TaskTerminalReceiptBacked,
            "{case}"
        );
        let receipt_terminal = terminal_of_receipt(receipt);
        assert_eq!(
            terminal_class(Some(receipt_terminal)),
            expected_terminal,
            "{case}"
        );
        assert_eq!(
            receipt.expires_epoch_ms,
            Some(terminal_epoch_ms(receipt_terminal) + DIRECT_TASK_TTL_MS),
            "{case}"
        );
        let submit_response = response(&report, "submit");
        assert_eq!(submit_response.kind, ResponseKind::Task, "{case}");
        assert_eq!(submit_response.error, None, "{case}");
        assert_exact_response_identity(submit_response, receipt);
        let before_restart = task_read(&report, "before-restart");
        let after_restart = task_read(&report, "after-restart");
        let before_full_ttl = task_read(&report, "before-full-ttl");
        assert_eq!(before_restart, after_restart, "{case}");
        assert_eq!(before_restart, before_full_ttl, "{case}");
        assert_eq!(
            before_restart.status,
            match expected_terminal {
                TerminalClass::Completed(_) => TaskStatus::Completed,
                TerminalClass::Failed(_) => TaskStatus::Failed,
                TerminalClass::Absent | TerminalClass::Cancelled => {
                    unreachable!("rejection matrix has only completed/failed terminal")
                }
            },
            "{case}"
        );
        assert_eq!(
            before_restart.terminal.as_ref(),
            Some(receipt_terminal),
            "{case}"
        );
        assert_eq!(
            terminal_class(before_restart.terminal.as_ref()),
            expected_terminal,
            "{case}"
        );
        assert_eq!(
            before_restart.expires_epoch_ms,
            terminal_epoch_ms(receipt_terminal) + DIRECT_TASK_TTL_MS,
            "{case}"
        );
        assert_eq!(reopened.receipt_live_count, 1, "{case}");
        assert_max_result_entitlement(receipt);
        assert_eq!(
            receipt_quota_bytes(reopened),
            MAX_RESPONSE_LINE_BYTES,
            "{case}"
        );
        assert_eq!(
            checkpoint(&report, "before-full-ttl").receipt_live_count,
            1,
            "{case}"
        );
        let terminal_snapshot = checkpoint(&report, "terminal-before-restart");
        assert_publication_matches_snapshot(
            &report,
            terminal_snapshot,
            &only_receipt(terminal_snapshot).key,
            terminal_of_receipt(only_receipt(terminal_snapshot)),
            TerminalPublicationOwner::ReceiptBackedTask,
        );
        if fail_stop {
            assert_eq!(terminal_snapshot.listener, ListenerState::Closed);
            assert!(terminal_snapshot.restart_requested);
            assert!(!terminal_snapshot.daemon_running);
        } else {
            assert_eq!(terminal_snapshot.listener, ListenerState::Listening);
            assert!(!terminal_snapshot.restart_requested);
        }
    }

    let staged_handoff = execute(Scenario::fake(vec![
        direct_provider(),
        Action::ConfigurePrepare { reject: true },
        Action::InstallBarrier {
            point: BarrierPoint::PrepareEntered,
        },
        Action::InstallBarrier {
            point: BarrierPoint::BeforeTaskStoreCreate,
        },
        submit("staged-prepare-rejection"),
        Action::WaitForEvent {
            event: EventKind::PrepareEntered,
        },
        Action::AdvanceMonotonic { millis: CUTOFF_MS },
        Action::WaitForEvent {
            event: EventKind::BoundHandoffCommitted,
        },
        Action::ReleaseBarrier {
            point: BarrierPoint::PrepareEntered,
        },
        Action::WaitForEvent {
            event: EventKind::BoundHandoffTerminalStaged,
        },
        checkpoint_action("prepare-rejection-staged"),
        Action::ReleaseBarrier {
            point: BarrierPoint::BeforeTaskStoreCreate,
        },
        Action::WaitForEvent {
            event: EventKind::TaskStoreTerminalCommitted,
        },
        Action::WaitForEvent {
            event: EventKind::TaskTerminalBoundCommitted,
        },
        Action::ReadTask {
            api: TaskApi::NativeGet,
            label: "staged-prepare-final-task".to_string(),
        },
        checkpoint_action("staged-prepare-final"),
    ]));
    let staged = checkpoint(&staged_handoff, "prepare-rejection-staged");
    assert_eq!(
        only_receipt(staged).state,
        SeedReceiptState::TaskHandoffActorBoundBegun
    );
    assert_eq!(
        terminal_class(only_receipt(staged).staged_terminal.as_ref()),
        TerminalClass::Completed(false)
    );
    assert!(staged.tasks.is_empty());
    assert_eq!(staged.task_link_reserved_count, 1);
    assert_max_result_entitlement(only_receipt(staged));
    let staged_preparations: Vec<_> = staged_handoff
        .staged_terminal_preparations
        .iter()
        .filter(|preparation| preparation.receipt_key == only_receipt(staged).key)
        .collect();
    assert_eq!(staged_preparations.len(), 1);
    assert_eq!(
        staged_preparations[0].terminal,
        *only_receipt(staged).staged_terminal.as_ref().unwrap()
    );
    assert_eq!(
        staged_preparations[0].committed_receipt_version,
        only_receipt(staged).version
    );
    let final_snapshot = checkpoint(&staged_handoff, "staged-prepare-final");
    assert_eq!(
        task_link_state(only_task_link(final_snapshot)),
        SeedReceiptState::TaskTerminalBound
    );
    assert!(final_snapshot.receipts.is_empty());
    assert!(only_task_link(final_snapshot).encoded_bytes <= 1_024);
    let final_task = task_read(&staged_handoff, "staged-prepare-final-task");
    assert_eq!(final_task.projection_source, ProjectionSource::TaskStore);
    assert_eq!(final_task.status, TaskStatus::Completed);
    assert!(!completed_result(final_task.terminal.as_ref().unwrap()).ok);
    assert_eq!(
        final_task.terminal.as_ref(),
        Some(&staged_preparations[0].terminal)
    );
    assert_eq!(
        terminal_digest(final_task.terminal.as_ref().unwrap()),
        terminal_digest(&staged_preparations[0].terminal)
    );
    assert_event_order(
        &staged_handoff,
        &[
            EventKind::PrepareEntered,
            EventKind::BoundHandoffCommitted,
            EventKind::BoundHandoffTerminalStaged,
            EventKind::TaskLinkCapacityReserved,
            EventKind::TaskStoreCreated,
            EventKind::TaskStoreTerminalCommitted,
            EventKind::TaskStoreTerminalReadback,
            EventKind::TaskLinkReservationConverted,
            EventKind::TaskTerminalBoundCommitted,
        ],
    );
    assert_eq!(
        count_event(&staged_handoff, EventKind::TaskBoundCommitted),
        0
    );
    assert_publication_matches_snapshot(
        &staged_handoff,
        final_snapshot,
        &only_task_link(final_snapshot).key,
        final_task.terminal.as_ref().unwrap(),
        TerminalPublicationOwner::StagedHandoffTask,
    );

    let already_bound = execute(Scenario::fake(vec![
        direct_provider(),
        Action::ConfigurePrepare { reject: true },
        Action::InstallBarrier {
            point: BarrierPoint::PrepareEntered,
        },
        submit("bound-prepare-rejection"),
        Action::WaitForEvent {
            event: EventKind::PrepareEntered,
        },
        Action::AdvanceMonotonic { millis: CUTOFF_MS },
        Action::WaitForEvent {
            event: EventKind::TaskBoundCommitted,
        },
        checkpoint_action("prepare-rejection-bound"),
        Action::ReleaseBarrier {
            point: BarrierPoint::PrepareEntered,
        },
        Action::WaitForEvent {
            event: EventKind::TaskStoreTerminalCommitted,
        },
        Action::ReadTask {
            api: TaskApi::CompatibilityResult,
            label: "bound-prepare-final-task".to_string(),
        },
        checkpoint_action("bound-prepare-final"),
    ]));
    assert_eq!(
        task_link_state(only_task_link(checkpoint(
            &already_bound,
            "prepare-rejection-bound"
        ))),
        SeedReceiptState::TaskBoundBegun
    );
    assert!(checkpoint(&already_bound, "prepare-rejection-bound")
        .receipts
        .is_empty());
    let bound_final = checkpoint(&already_bound, "bound-prepare-final");
    assert_eq!(
        task_link_state(only_task_link(bound_final)),
        SeedReceiptState::TaskTerminalBound
    );
    assert!(bound_final.receipts.is_empty());
    let bound_task = task_read(&already_bound, "bound-prepare-final-task");
    assert_eq!(bound_task.projection_source, ProjectionSource::TaskStore);
    assert_eq!(bound_task.status, TaskStatus::Completed);
    assert!(!completed_result(bound_task.terminal.as_ref().unwrap()).ok);
    assert_publication_matches_snapshot(
        &already_bound,
        bound_final,
        &only_task_link(bound_final).key,
        bound_task.terminal.as_ref().unwrap(),
        TerminalPublicationOwner::BoundTaskStore,
    );
}

#[test]
fn receipt_backed_task_terminal_survives_reopen_byte_equivalent() {
    let report = execute(Scenario::fake(vec![
        Action::SeedReceipt {
            state: SeedReceiptState::TaskTerminalReceiptBacked,
            cancel_requested: false,
            staged_terminal: Some(success_payload()),
        },
        Action::ReadTask {
            api: TaskApi::NativeGet,
            label: "before-task".to_string(),
        },
        checkpoint_action("before"),
        Action::Restart,
        Action::ReadTask {
            api: TaskApi::CompatibilityResult,
            label: "after-task".to_string(),
        },
        checkpoint_action("after"),
    ]));

    let before = only_receipt(checkpoint(&report, "before"));
    let after = only_receipt(checkpoint(&report, "after"));
    assert_eq!(before, after);
    assert_eq!(
        task_read(&report, "before-task"),
        task_read(&report, "after-task")
    );
    assert_eq!(
        task_read(&report, "after-task").terminal.as_ref(),
        after.terminal.as_ref()
    );
    assert!(checkpoint(&report, "after").tasks.is_empty());
    assert_eq!(checkpoint(&report, "after").callbacks.total_domain(), 0);
}

#[test]
fn receipt_backed_task_result_is_repeatable_and_direct_ack_is_rejected() {
    let report = execute(Scenario::fake(vec![
        Action::SeedReceipt {
            state: SeedReceiptState::TaskTerminalReceiptBacked,
            cancel_requested: false,
            staged_terminal: Some(success_payload()),
        },
        Action::ReadTask {
            api: TaskApi::NativeGet,
            label: "native".to_string(),
        },
        Action::ReadTask {
            api: TaskApi::NativeWait,
            label: "wait".to_string(),
        },
        Action::ReadTask {
            api: TaskApi::CompatibilityResult,
            label: "compat".to_string(),
        },
        Action::Acknowledge {
            key: KeyCase::Exact,
            digest: DigestCase::TaskTerminal,
            disconnect: AckDisconnectPoint::Never,
            label: "invalid-ack".to_string(),
        },
        checkpoint_action("after-reads"),
    ]));

    assert_eq!(task_read(&report, "native"), task_read(&report, "wait"));
    assert_eq!(task_read(&report, "native"), task_read(&report, "compat"));
    let task = task_read(&report, "native");
    assert_eq!(task.status, TaskStatus::Completed);
    let task_terminal = task
        .terminal
        .as_ref()
        .expect("receipt-backed terminal Task must expose its terminal");
    assert!(completed_result(task_terminal).ok);
    assert_eq!(completed_result(task_terminal).summary, SUCCESS_SUMMARY);
    let task_digest = terminal_digest(task_terminal);
    assert_safe_fingerprint(task_digest);
    assert_eq!(
        response(&report, "invalid-ack").kind,
        ResponseKind::Rejected
    );
    assert_eq!(
        response(&report, "invalid-ack").error,
        Some(ErrorCode::InvalidRequest)
    );
    let after_reads = checkpoint(&report, "after-reads");
    let receipt = only_receipt(after_reads);
    assert_eq!(receipt.state, SeedReceiptState::TaskTerminalReceiptBacked);
    assert_eq!(terminal_of_receipt(receipt), task_terminal);
    assert_eq!(terminal_digest(terminal_of_receipt(receipt)), task_digest);
    assert_eq!(after_reads.tombstone_count, 0);
}

#[test]
fn actor_bind_promotes_the_same_promised_task_once() {
    let report = execute(Scenario::fake(vec![
        direct_provider(),
        Action::InstallBarrier {
            point: BarrierPoint::AdmissionEntered,
        },
        Action::InstallBarrier {
            point: BarrierPoint::BeforePrepare,
        },
        submit("submit"),
        Action::WaitForEvent {
            event: EventKind::AdmissionEntered,
        },
        Action::AdvanceMonotonic { millis: CUTOFF_MS },
        Action::ReadTask {
            api: TaskApi::NativeGet,
            label: "promised".to_string(),
        },
        Action::ReleaseBarrier {
            point: BarrierPoint::AdmissionEntered,
        },
        Action::WaitForEvent {
            event: EventKind::TaskBoundCommitted,
        },
        Action::ReadTask {
            api: TaskApi::CompatibilityGet,
            label: "bound".to_string(),
        },
        checkpoint_action("bound"),
        Action::ReleaseBarrier {
            point: BarrierPoint::BeforePrepare,
        },
    ]));

    let promised = task_read(&report, "promised");
    let bound = task_read(&report, "bound");
    assert_same_stable_task(promised, bound);
    assert_eq!(bound.projection_source, ProjectionSource::TaskStore);
    let snapshot = checkpoint(&report, "bound");
    assert_eq!(snapshot.tasks.len(), 1);
    assert_eq!(snapshot.task_store_create_attempts, 1);
    assert_eq!(snapshot.task_link_reserved_count, 0);
    assert_eq!(snapshot.task_links.len(), 1);
    let binding = observed_actor_binding(&report, "submit");
    assert_eq!(binding.receipt_key, bound.receipt_key);
    let task_binding = binding
        .task_binding
        .as_ref()
        .expect("Task promotion must expose the one-shot link reservation transfer");
    assert_eq!(
        task_binding.task_link_digest,
        snapshot.task_links[0].link_digest
    );
    assert_eq!(
        binding.actor_identity_hash,
        bound.workspace_identity_hash.clone().unwrap()
    );
    assert_eq!(count_event(&report, EventKind::UnboundPromiseCommitted), 1);
    assert_eq!(count_event(&report, EventKind::TaskStoreCreated), 1);
    assert_eq!(count_event(&report, EventKind::TaskBoundCommitted), 1);
    assert_event_order(
        &report,
        &[
            EventKind::TaskLinkCapacityReserved,
            EventKind::TaskStoreCreated,
            EventKind::TaskLinkReservationConverted,
            EventKind::TaskBoundCommitted,
        ],
    );
}

#[test]
fn cancel_or_restart_before_actor_bind_terminalizes_without_callback() {
    let cancel_report = execute(Scenario::fake(vec![
        direct_provider(),
        Action::InstallBarrier {
            point: BarrierPoint::AdmissionEntered,
        },
        submit("submit"),
        Action::WaitForEvent {
            event: EventKind::AdmissionEntered,
        },
        Action::AdvanceMonotonic { millis: CUTOFF_MS },
        Action::ReadTask {
            api: TaskApi::NativeGet,
            label: "cancel-promised".to_string(),
        },
        Action::Cancel {
            key: KeyCase::Exact,
            lazy_session: false,
            label: "cancel".to_string(),
        },
        Action::ReadTask {
            api: TaskApi::CompatibilityGet,
            label: "cancel-terminal".to_string(),
        },
        checkpoint_action("cancelled"),
    ]));
    let cancelled = checkpoint(&cancel_report, "cancelled");
    assert!(cancelled.tasks.is_empty());
    assert_eq!(
        only_receipt(cancelled).state,
        SeedReceiptState::TaskTerminalReceiptBacked
    );
    assert_cancelled_terminal(terminal_of_receipt(only_receipt(cancelled)));
    let cancel_terminal = task_read(&cancel_report, "cancel-terminal");
    assert_eq!(
        task_read(&cancel_report, "cancel-promised").task_id,
        cancel_terminal.task_id
    );
    assert_cancelled_terminal(
        cancel_terminal
            .terminal
            .as_ref()
            .expect("cancel terminal Task must carry its tagged terminal"),
    );
    assert_eq!(
        cancel_terminal.terminal.as_ref(),
        only_receipt(cancelled).terminal.as_ref()
    );
    assert_eq!(
        cancel_terminal.expires_epoch_ms,
        terminal_epoch_ms(
            cancel_terminal
                .terminal
                .as_ref()
                .expect("cancel terminal exists")
        ) + DIRECT_TASK_TTL_MS
    );
    assert_eq!(cancelled.callbacks.prepare, 0);
    assert_eq!(cancelled.callbacks.execute, 0);

    let restart_report = execute(Scenario::fake(vec![
        direct_provider(),
        Action::InstallBarrier {
            point: BarrierPoint::AdmissionEntered,
        },
        submit("submit"),
        Action::WaitForEvent {
            event: EventKind::AdmissionEntered,
        },
        Action::AdvanceMonotonic { millis: CUTOFF_MS },
        Action::ReadTask {
            api: TaskApi::NativeGet,
            label: "restart-promised".to_string(),
        },
        Action::Crash {
            point: CrashPoint::TaskPromisedUnbound,
        },
        Action::Restart,
        Action::ReadTask {
            api: TaskApi::CompatibilityGet,
            label: "restart-terminal".to_string(),
        },
        checkpoint_action("restarted"),
    ]));
    let restarted = checkpoint(&restart_report, "restarted");
    assert!(restarted.tasks.is_empty());
    assert_eq!(
        only_receipt(restarted).state,
        SeedReceiptState::TaskTerminalReceiptBacked
    );
    assert_failed_terminal(
        terminal_of_receipt(only_receipt(restarted)),
        V5SafeFailureReason::Interrupted,
    );
    assert_eq!(
        task_read(&restart_report, "restart-promised").task_id,
        task_read(&restart_report, "restart-terminal").task_id
    );
    let restart_terminal = task_read(&restart_report, "restart-terminal");
    assert_failed_terminal(
        restart_terminal
            .terminal
            .as_ref()
            .expect("restart terminal Task must carry its tagged terminal"),
        V5SafeFailureReason::Interrupted,
    );
    assert_eq!(
        restart_terminal.terminal.as_ref(),
        only_receipt(restarted).terminal.as_ref()
    );
    assert_eq!(
        restart_terminal.expires_epoch_ms,
        terminal_epoch_ms(
            restart_terminal
                .terminal
                .as_ref()
                .expect("restart terminal exists")
        ) + DIRECT_TASK_TTL_MS
    );
    assert_eq!(restarted.callbacks.prepare, 0);
    assert_eq!(restarted.callbacks.execute, 0);
}

#[test]
fn unbound_validation_and_admission_share_one_two_second_fail_stop_grace() {
    let validation_grace_ms = 750;
    let admission_grace_ms = CLEANUP_GRACE_MS - validation_grace_ms;
    let report = execute(Scenario::fake(vec![
        direct_provider(),
        Action::InstallBarrier {
            point: BarrierPoint::ValidationEntered,
        },
        Action::InstallBarrier {
            point: BarrierPoint::AdmissionEntered,
        },
        submit("submit"),
        Action::WaitForEvent {
            event: EventKind::ValidationEntered,
        },
        Action::AdvanceMonotonic { millis: CUTOFF_MS },
        checkpoint_action("promised"),
        Action::AdvanceMonotonic {
            millis: validation_grace_ms,
        },
        Action::ReleaseBarrier {
            point: BarrierPoint::ValidationEntered,
        },
        Action::WaitForEvent {
            event: EventKind::AdmissionEntered,
        },
        checkpoint_action("admission-with-remainder"),
        Action::AdvanceMonotonic {
            millis: admission_grace_ms - 1,
        },
        checkpoint_action("one-ms-before-shared-deadline"),
        Action::AdvanceMonotonic { millis: 1 },
        checkpoint_action("fail-stop"),
        Action::ReleaseBarrier {
            point: BarrierPoint::AdmissionEntered,
        },
        checkpoint_action("late-release"),
    ]));

    let promised = checkpoint(&report, "promised");
    let promised_epoch = only_receipt(promised).accepted_epoch_ms;
    assert_eq!(
        only_receipt(promised).state,
        SeedReceiptState::TaskPromisedUnbound
    );
    let admission = checkpoint(&report, "admission-with-remainder");
    assert!(admission.daemon_running);
    assert_eq!(admission.callbacks.validation, 1);
    assert_eq!(admission.callbacks.admission, 1);
    let before = checkpoint(&report, "one-ms-before-shared-deadline");
    assert_eq!(before.listener, ListenerState::Listening);
    assert!(before.daemon_running);
    assert!(!before.restart_requested);
    let stopped = checkpoint(&report, "fail-stop");
    assert_eq!(stopped.listener, ListenerState::Closed);
    assert!(stopped.restart_requested);
    assert!(!stopped.daemon_running);
    assert_eq!(stopped.process_exit_elapsed_ms, Some(CLEANUP_GRACE_MS));
    assert_eq!(only_receipt(stopped).accepted_epoch_ms, promised_epoch);
    let late = checkpoint(&report, "late-release");
    assert_eq!(late.callbacks.validation, 1);
    assert_eq!(late.callbacks.admission, 1);
    assert_eq!(late.callbacks.prepare, 0);
    assert_eq!(late.callbacks.execute, 0);
    assert_eq!(late.actor_leases, 0);
}

#[test]
fn known_long_requires_begun_bound_handoff_intent() {
    let report = execute(Scenario::fake(vec![
        known_long_provider(),
        submit("submit"),
        Action::WaitForEvent {
            event: EventKind::BoundHandoffCommitted,
        },
        checkpoint_action("handoff"),
    ]));

    assert_event_order(
        &report,
        &[
            EventKind::ReceiptReserved,
            EventKind::ActorBoundCommitted,
            EventKind::ReceiptBegunCommitted,
            EventKind::PrepareEntered,
            EventKind::BoundHandoffCommitted,
        ],
    );
    assert!(
        checkpoint(&report, "handoff").receipts.is_empty(),
        "a confirmed TaskBound must retire the transient receipt-owned handoff"
    );
    assert_eq!(count_event(&report, EventKind::UnboundPromiseCommitted), 0);
}

#[test]
fn known_long_after_prepare_never_becomes_unbound_promise() {
    let report = execute(Scenario::fake(vec![
        known_long_provider(),
        submit("submit"),
        Action::WaitForEvent {
            event: EventKind::TaskBoundCommitted,
        },
        Action::ReadTask {
            api: TaskApi::NativeGet,
            label: "working".to_string(),
        },
        checkpoint_action("bound"),
    ]));

    assert_eq!(count_event(&report, EventKind::UnboundPromiseCommitted), 0);
    assert_eq!(count_event(&report, EventKind::PrepareEntered), 1);
    assert_eq!(count_event(&report, EventKind::BoundHandoffCommitted), 1);
    let task = task_read(&report, "working");
    assert_eq!(task.status, TaskStatus::Working);
    assert_eq!(task.projection_source, ProjectionSource::TaskStore);
    assert_eq!(checkpoint(&report, "bound").task_store_create_attempts, 1);
}

#[test]
fn begun_cutoff_intent_survives_crash_before_task_create() {
    let report = execute(Scenario::fake(vec![
        direct_provider(),
        Action::InstallBarrier {
            point: BarrierPoint::PrepareEntered,
        },
        Action::InstallBarrier {
            point: BarrierPoint::BeforeTaskStoreCreate,
        },
        submit("submit"),
        Action::WaitForEvent {
            event: EventKind::PrepareEntered,
        },
        Action::AdvanceMonotonic {
            millis: CUTOFF_MS - 1,
        },
        checkpoint_action("begun-one-ms-before-cutoff"),
        Action::AdvanceMonotonic { millis: 1 },
        Action::WaitForEvent {
            event: EventKind::BoundHandoffCommitted,
        },
        Action::Crash {
            point: CrashPoint::BeforeTaskStoreCreate,
        },
        Action::Restart,
        Action::ReadTask {
            api: TaskApi::NativeGet,
            label: "recovered".to_string(),
        },
        checkpoint_action("restarted"),
    ]));

    let before_cutoff = checkpoint(&report, "begun-one-ms-before-cutoff");
    assert_eq!(
        only_receipt(before_cutoff).state,
        SeedReceiptState::ReservedBegun
    );
    assert!(before_cutoff.tasks.is_empty());
    assert_eq!(before_cutoff.task_store_create_attempts, 0);
    let restarted = checkpoint(&report, "restarted");
    let task = task_read(&report, "recovered");
    assert_eq!(task.status, TaskStatus::Failed);
    assert_failed_terminal(
        task.terminal
            .as_ref()
            .expect("recovered begun Task must expose terminal"),
        V5SafeFailureReason::OutcomeUncertain,
    );
    assert_eq!(restarted.tasks.len(), 1);
    assert_eq!(only_task(restarted).task_id, task.task_id);
    assert_eq!(restarted.callbacks.prepare, 1);
    assert_eq!(restarted.callbacks.execute, 0);
    assert_eq!(count_event(&report, EventKind::PrepareEntered), 1);
}

#[test]
fn every_cross_store_crash_point_reconciles_without_split_brain() {
    #[derive(Clone, Copy)]
    enum ExpectedTerminal {
        Completed,
        Failed(V5SafeFailureReason),
    }
    let expected = [
        (
            EntryPath::PromisedUnbound,
            CrashPoint::BeforePromisedActorIntent,
            0,
            ExpectedTerminal::Failed(V5SafeFailureReason::Interrupted),
            false,
        ),
        (
            EntryPath::PromisedUnbound,
            CrashPoint::AfterPromisedActorIntent,
            1,
            ExpectedTerminal::Failed(V5SafeFailureReason::Interrupted),
            false,
        ),
        (
            EntryPath::PromisedUnbound,
            CrashPoint::AfterTaskStoreCreateBeforeTaskBound,
            1,
            ExpectedTerminal::Failed(V5SafeFailureReason::Interrupted),
            false,
        ),
        (
            EntryPath::ReservedActorBound,
            CrashPoint::BeforeBoundHandoffIntent,
            0,
            ExpectedTerminal::Failed(V5SafeFailureReason::Interrupted),
            false,
        ),
        (
            EntryPath::ReservedActorBound,
            CrashPoint::AfterBoundHandoffIntent,
            1,
            ExpectedTerminal::Failed(V5SafeFailureReason::Interrupted),
            false,
        ),
        (
            EntryPath::ReservedActorBound,
            CrashPoint::AfterStagedTerminal,
            1,
            ExpectedTerminal::Completed,
            true,
        ),
        (
            EntryPath::ReservedActorBound,
            CrashPoint::AfterStagedTaskStoreTerminalReadbackBeforeLedgerCommit,
            1,
            ExpectedTerminal::Completed,
            true,
        ),
        (
            EntryPath::ReservedActorBound,
            CrashPoint::AfterTaskStoreCreateBeforeTaskBound,
            1,
            ExpectedTerminal::Failed(V5SafeFailureReason::Interrupted),
            false,
        ),
        (
            EntryPath::ReservedBegun,
            CrashPoint::BeforeBoundHandoffIntent,
            0,
            ExpectedTerminal::Failed(V5SafeFailureReason::OutcomeUncertain),
            false,
        ),
        (
            EntryPath::ReservedBegun,
            CrashPoint::AfterBegunHandoffIntent,
            1,
            ExpectedTerminal::Failed(V5SafeFailureReason::OutcomeUncertain),
            false,
        ),
        (
            EntryPath::ReservedBegun,
            CrashPoint::AfterStagedTerminal,
            1,
            ExpectedTerminal::Completed,
            true,
        ),
        (
            EntryPath::ReservedBegun,
            CrashPoint::AfterStagedTaskStoreTerminalReadbackBeforeLedgerCommit,
            1,
            ExpectedTerminal::Completed,
            true,
        ),
        (
            EntryPath::ReservedBegun,
            CrashPoint::AfterTaskStoreCreateBeforeTaskBound,
            1,
            ExpectedTerminal::Failed(V5SafeFailureReason::OutcomeUncertain),
            false,
        ),
    ];
    let cases = expected
        .iter()
        .map(|(path, point, _, _, staged)| CrashWorkload {
            path: *path,
            point: *point,
            cancel_before_crash: false,
            stage_terminal_before_crash: *staged,
        })
        .collect();
    let report = execute(Scenario::fake(vec![Action::RunCrossStoreCrashWorkload {
        cases,
    }]));

    assert_eq!(report.crash_cases.len(), expected.len());
    for (path, point, task_store_record_count, expected_terminal, staged) in expected {
        let matches: Vec<_> = report
            .crash_cases
            .iter()
            .filter(|case| case.path == path && case.point == point)
            .collect();
        assert_eq!(
            matches.len(),
            1,
            "missing/duplicate crash pair {path:?}/{point:?}"
        );
        let case = matches[0];
        let key = case.ledger.key();
        assert_receipt_key(key);
        assert_eq!(case.projections.len(), 1, "{case:#?}");
        assert_eq!(
            case.task_store_records.len(),
            task_store_record_count,
            "{case:#?}"
        );
        let projection = &case.projections[0];
        assert_task_observation(projection);
        assert_eq!(&projection.receipt_key, key, "{case:#?}");
        assert_eq!(projection.task_id, key.reserved_task_id);
        assert_eq!(projection.invocation_id, key.invocation_id);
        assert_eq!(projection.terminal.as_ref(), Some(&case.recovered_terminal));
        match expected_terminal {
            ExpectedTerminal::Completed => {
                assert!(completed_result(&case.recovered_terminal).ok, "{case:#?}")
            }
            ExpectedTerminal::Failed(reason) => {
                assert_failed_terminal(&case.recovered_terminal, reason)
            }
        }
        if task_store_record_count == 1 {
            assert_eq!(case.task_store_records[0], *projection, "{case:#?}");
            let link = case.ledger.lifecycle_link();
            assert_eq!(task_link_state(link), SeedReceiptState::TaskTerminalBound);
            assert!(link.encoded_bytes <= 1_024);
            assert_terminal_bound_link(link, projection, &case.recovered_terminal);
        } else {
            let receipt = case.ledger.active_receipt();
            assert_eq!(receipt.state, SeedReceiptState::TaskTerminalReceiptBacked);
            assert_eq!(receipt.terminal.as_ref(), Some(&case.recovered_terminal));
        }
        let mut callback_ids: Vec<_> = case.callback_invocation_ids.iter().collect();
        callback_ids.sort_unstable();
        callback_ids.dedup();
        assert_eq!(callback_ids.len(), case.callback_invocation_ids.len());
        assert!(
            callback_ids.len() <= 1,
            "recovery must never replay callbacks"
        );
        assert!(callback_ids
            .iter()
            .all(|id| id.as_str() == key.invocation_id));
        assert_eq!(case.staged_terminal_before_crash.is_some(), staged);
        if let Some(staged_terminal) = &case.staged_terminal_before_crash {
            assert_eq!(staged_terminal, &case.recovered_terminal);
            let publication = terminal_publication_for(&report, key, &case.recovered_terminal);
            assert_eq!(
                publication.commit.owner(),
                TerminalPublicationOwner::StagedHandoffTask
            );
            let task_commit = match &publication.commit {
                TerminalCommitPreflightObservation::StagedHandoffTask { task, .. } => task,
                other => panic!("staged crash recovered through wrong owner bundle: {other:?}"),
            };
            assert_eq!(
                task_commit.staged_terminal_digest,
                terminal_digest(staged_terminal)
            );
            assert_eq!(
                case.task_store_records[0].terminal.as_ref(),
                Some(staged_terminal)
            );
            assert_eq!(
                callback_ids.len(),
                1,
                "staged outcome callback must not replay"
            );
        }
        let mut task_ids: Vec<_> = case
            .projections
            .iter()
            .chain(case.task_store_records.iter())
            .map(|task| task.task_id.as_str())
            .collect();
        task_ids.sort_unstable();
        task_ids.dedup();
        assert_eq!(task_ids, vec![key.reserved_task_id.as_str()]);
        assert!(case.receipt_store_generation > 0);
        assert!(case.task_store_generation > 0);
    }

    for status in [TaskStatus::Queued, TaskStatus::Working] {
        for begun in [false, true] {
            for cancel_requested in [false, true] {
                let label = format!(
                    "provisional-{}-begun-{begun}-cancel-{cancel_requested}",
                    match status {
                        TaskStatus::Queued => "queued",
                        TaskStatus::Working => "working",
                        _ => unreachable!(),
                    }
                );
                let before_label = format!("{label}-before");
                let after_label = format!("{label}-after");
                let matrix = execute(Scenario::fake(vec![
                    Action::SeedReceipt {
                        state: if begun {
                            SeedReceiptState::TaskHandoffActorBoundBegun
                        } else {
                            SeedReceiptState::TaskHandoffActorBoundNotBegun
                        },
                        cancel_requested,
                        staged_terminal: Some(success_payload()),
                    },
                    Action::SeedTask {
                        status,
                        cancel_requested,
                        receipt_link: ReceiptLinkCase::Missing,
                        identity: IdentityRelation::Exact,
                        version: 17,
                    },
                    Action::SeedTaskLinkReservation {
                        relation: IdentityRelation::Exact,
                    },
                    checkpoint_action(&before_label),
                    Action::AttemptStagedTerminalAgainstProvisional {
                        mismatch: None,
                        repeat_same_terminal: true,
                        label: label.clone(),
                    },
                    checkpoint_action(&after_label),
                ]));
                let before = checkpoint(&matrix, &before_label);
                let after = checkpoint(&matrix, &after_label);
                assert_eq!(before.task_link_reserved_count, 1);
                assert_eq!(before.tasks.len(), 1);
                assert_eq!(after.receipt_live_count, 0);
                assert!(after.receipts.is_empty());
                assert_eq!(
                    task_link_state(only_task_link(after)),
                    SeedReceiptState::TaskTerminalBound
                );
                assert_eq!(after.task_link_reserved_count, 0);
                assert_eq!(after.task_link_count, 1);
                let terminal = only_task(after)
                    .terminal
                    .as_ref()
                    .expect("exact provisional transfer publishes the staged terminal");
                assert!(completed_result(terminal).ok);
                assert_eq!(
                    terminal,
                    only_receipt(before)
                        .staged_terminal
                        .as_ref()
                        .expect("setup staged terminal")
                );
                let publication =
                    terminal_publication_for(&matrix, &only_task_link(after).key, terminal);
                let task_commit = match &publication.commit {
                    TerminalCommitPreflightObservation::StagedHandoffTask { task, .. } => task,
                    other => panic!("exact provisional used wrong owner bundle: {other:?}"),
                };
                let expectation = match &task_commit.terminal_write_expectation {
                    StagedTaskTerminalWriteExpectation::ExactProvisional {
                        task_id,
                        invocation_id,
                        expected_version,
                        status: observed_status,
                        cancel_requested: observed_cancel,
                        task_link_digest,
                        ..
                    } => (
                        task_id,
                        invocation_id,
                        expected_version,
                        observed_status,
                        observed_cancel,
                        task_link_digest,
                    ),
                    other => panic!("provisional matrix needs exact readback: {other:?}"),
                };
                assert_eq!(expectation.0, &only_task(before).task_id);
                assert_eq!(expectation.1, &only_task(before).invocation_id);
                assert_eq!(*expectation.2, only_task(before).version);
                assert_eq!(*expectation.3, status);
                assert_eq!(*expectation.4, cancel_requested);
                assert_terminal_bound_link(only_task_link(after), only_task(after), terminal);
                assert_eq!(expectation.5, &only_task_link(after).link_digest);
                assert_eq!(
                    task_commit.terminal_write_branch,
                    StagedTaskTerminalWriteBranch::ReplacedExactProvisional
                );
                let repeat = task_commit
                    .idempotent_repeat
                    .as_ref()
                    .expect("same-terminal retry must be an exact idempotent readback");
                assert_eq!(repeat.task_record, task_commit.task_record);
                assert_eq!(
                    repeat.task_store_generation_before,
                    repeat.task_store_generation_after
                );
                assert_eq!(
                    after.callbacks.total_domain(),
                    before.callbacks.total_domain()
                );
            }
        }
    }

    for mismatch in [
        ProvisionalMismatchField::TaskId,
        ProvisionalMismatchField::InvocationId,
        ProvisionalMismatchField::Status,
        ProvisionalMismatchField::Version,
        ProvisionalMismatchField::CancelRequested,
        ProvisionalMismatchField::TaskLinkDigest,
    ] {
        let label = format!("provisional-mismatch-{mismatch:?}");
        let mismatch_report = execute(Scenario::fake(vec![
            Action::SeedReceipt {
                state: SeedReceiptState::TaskHandoffActorBoundBegun,
                cancel_requested: true,
                staged_terminal: Some(success_payload()),
            },
            Action::SeedTask {
                status: TaskStatus::Working,
                cancel_requested: true,
                receipt_link: ReceiptLinkCase::Missing,
                identity: IdentityRelation::Exact,
                version: 23,
            },
            Action::SeedTaskLinkReservation {
                relation: IdentityRelation::Exact,
            },
            checkpoint_action("mismatch-before"),
            Action::AttemptStagedTerminalAgainstProvisional {
                mismatch: Some(mismatch),
                repeat_same_terminal: false,
                label,
            },
            checkpoint_action("mismatch-after"),
        ]));
        let before = checkpoint(&mismatch_report, "mismatch-before");
        let after = checkpoint(&mismatch_report, "mismatch-after");
        assert_eq!(after.receipts, before.receipts, "{mismatch:?}");
        assert_eq!(after.tasks, before.tasks, "{mismatch:?}");
        assert_eq!(after.task_links, before.task_links, "{mismatch:?}");
        assert_eq!(
            after.receipt_store_mutations,
            before.receipt_store_mutations
        );
        assert_eq!(after.task_store_mutations, before.task_store_mutations);
        assert_eq!(after.store_generation, before.store_generation);
        assert_eq!(after.listener, ListenerState::Closed);
        assert!(after.restart_requested);
        assert!(!after.daemon_running);
        assert!(mismatch_report.terminal_publications.is_empty());
    }
}

#[test]
fn task_store_inspect_only_open_preserves_queued_and_working_until_receipt_reconciliation() {
    for status in [TaskStatus::Queued, TaskStatus::Working] {
        let report = execute(Scenario::fake(vec![
            Action::SeedReceipt {
                state: SeedReceiptState::TaskBoundNotBegun,
                cancel_requested: false,
                staged_terminal: None,
            },
            Action::SeedTask {
                status,
                cancel_requested: false,
                receipt_link: ReceiptLinkCase::Exact,
                identity: IdentityRelation::Exact,
                version: 7,
            },
            checkpoint_action("before-open"),
            Action::OpenTaskStoreInspectOnly,
            checkpoint_action("after-open"),
        ]));

        let before = checkpoint(&report, "before-open");
        let after = checkpoint(&report, "after-open");
        assert_eq!(before.tasks, after.tasks);
        assert_eq!(before.task_store_mutations, after.task_store_mutations);
        assert_eq!(before.store_generation, after.store_generation);
        assert_eq!(only_task(after).status, status);
        assert_eq!(only_task(after).version, 7);
        assert_eq!(after.listener, ListenerState::NotPublished);

        let orphan = execute(Scenario::fake(vec![
            Action::SeedTask {
                status,
                cancel_requested: false,
                receipt_link: ReceiptLinkCase::Missing,
                identity: IdentityRelation::Exact,
                version: 9,
            },
            checkpoint_action("orphan-before"),
            Action::OpenTaskStoreInspectOnly,
            Action::ReconcileStartup,
            checkpoint_action("orphan-after"),
        ]));
        let orphan_before = checkpoint(&orphan, "orphan-before");
        let orphan_after = checkpoint(&orphan, "orphan-after");
        assert_eq!(orphan_before.tasks, orphan_after.tasks, "{status:?}");
        assert_eq!(
            orphan_before.task_store_mutations,
            orphan_after.task_store_mutations
        );
        assert_eq!(
            orphan_before.store_generation,
            orphan_after.store_generation
        );
        assert_eq!(only_task(orphan_after).version, 9);
        assert_eq!(orphan_after.listener, ListenerState::NotPublished);
        assert!(orphan_after.restart_requested);
        assert!(!orphan_after.daemon_running);
    }
}

#[test]
fn receipt_led_startup_distinguishes_working_begun_false_from_begun_true() {
    for (receipt_state, expected) in [
        (
            SeedReceiptState::TaskBoundNotBegun,
            V5SafeFailureReason::Interrupted,
        ),
        (
            SeedReceiptState::TaskBoundBegun,
            V5SafeFailureReason::OutcomeUncertain,
        ),
    ] {
        let report = execute(Scenario::fake(vec![
            Action::SeedReceipt {
                state: receipt_state,
                cancel_requested: false,
                staged_terminal: None,
            },
            Action::SeedTask {
                status: TaskStatus::Working,
                cancel_requested: false,
                receipt_link: ReceiptLinkCase::Exact,
                identity: IdentityRelation::Exact,
                version: 3,
            },
            Action::OpenTaskStoreInspectOnly,
            Action::ReconcileStartup,
            checkpoint_action("reconciled"),
        ]));

        let reconciled = checkpoint(&report, "reconciled");
        assert_eq!(only_task(reconciled).status, TaskStatus::Failed);
        assert_failed_terminal(
            only_task(reconciled)
                .terminal
                .as_ref()
                .expect("reconciled Task must expose failure terminal"),
            expected,
        );
        assert_eq!(reconciled.callbacks.total_domain(), 0);
        assert_eq!(reconciled.listener, ListenerState::Listening);
    }
}

#[test]
fn v5_active_task_without_exact_receipt_link_fail_stops_before_listener() {
    for receipt_link in [ReceiptLinkCase::Missing, ReceiptLinkCase::Foreign] {
        let mut actions = Vec::new();
        if matches!(receipt_link, ReceiptLinkCase::Foreign) {
            actions.push(Action::SeedReceipt {
                state: SeedReceiptState::TaskBoundNotBegun,
                cancel_requested: false,
                staged_terminal: None,
            });
        }
        actions.extend([
            Action::SeedTask {
                status: TaskStatus::Working,
                cancel_requested: false,
                receipt_link,
                identity: IdentityRelation::Exact,
                version: 11,
            },
            checkpoint_action("before-open"),
            Action::OpenTaskStoreInspectOnly,
            Action::ReconcileStartup,
            checkpoint_action("after-reconcile"),
        ]);
        let report = execute(Scenario::fake(actions));

        let before = checkpoint(&report, "before-open");
        let after = checkpoint(&report, "after-reconcile");
        assert_eq!(before.tasks, after.tasks);
        assert_eq!(before.receipts, after.receipts);
        assert_eq!(before.task_store_mutations, after.task_store_mutations);
        assert_eq!(
            before.receipt_store_mutations,
            after.receipt_store_mutations
        );
        assert_eq!(after.listener, ListenerState::NotPublished);
        assert!(after.restart_requested);
        assert!(!after.daemon_running);
        assert_eq!(after.callbacks.total_domain(), 0);
    }

    for reservation in [None, Some(IdentityRelation::Foreign)] {
        let mut actions = vec![Action::SeedReceipt {
            state: SeedReceiptState::TaskHandoffActorBoundNotBegun,
            cancel_requested: false,
            staged_terminal: None,
        }];
        if let Some(relation) = reservation {
            actions.push(Action::SeedTaskLinkReservation { relation });
        }
        actions.extend([
            Action::SeedTask {
                status: TaskStatus::Queued,
                cancel_requested: false,
                receipt_link: ReceiptLinkCase::Missing,
                identity: IdentityRelation::Exact,
                version: 1,
            },
            checkpoint_action("unbound-task-before-reconcile"),
            Action::OpenTaskStoreInspectOnly,
            Action::ReconcileStartup,
            checkpoint_action("unbound-task-after-reconcile"),
        ]);
        let report = execute(Scenario::fake(actions));
        let before = checkpoint(&report, "unbound-task-before-reconcile");
        let after = checkpoint(&report, "unbound-task-after-reconcile");
        assert_eq!(after.listener, ListenerState::NotPublished);
        assert!(after.restart_requested);
        assert!(!after.daemon_running);
        assert_eq!(after.receipts, before.receipts);
        assert_eq!(after.tasks, before.tasks);
        assert_eq!(after.task_links, before.task_links);
        assert_eq!(after.task_store_mutations, before.task_store_mutations);
        assert_eq!(
            after.receipt_store_mutations,
            before.receipt_store_mutations
        );
        assert_eq!(after.callbacks.total_domain(), 0);
    }

    let exact = execute(Scenario::fake(vec![
        Action::SeedReceipt {
            state: SeedReceiptState::TaskHandoffActorBoundNotBegun,
            cancel_requested: false,
            staged_terminal: None,
        },
        Action::SeedTaskLinkReservation {
            relation: IdentityRelation::Exact,
        },
        Action::SeedTask {
            status: TaskStatus::Queued,
            cancel_requested: false,
            receipt_link: ReceiptLinkCase::Missing,
            identity: IdentityRelation::Exact,
            version: 1,
        },
        Action::OpenTaskStoreInspectOnly,
        Action::ReconcileStartup,
        checkpoint_action("exact-reservation-reconciled"),
    ]));
    let reconciled = checkpoint(&exact, "exact-reservation-reconciled");
    assert_eq!(reconciled.listener, ListenerState::Listening);
    assert_eq!(reconciled.task_links.len(), 1);
    assert_eq!(reconciled.task_link_reserved_count, 0);
    assert_eq!(
        task_link_state(only_task_link(reconciled)),
        SeedReceiptState::TaskBoundNotBegun
    );
    assert!(reconciled.receipts.is_empty());
    assert_eq!(reconciled.callbacks.total_domain(), 0);
}

#[test]
fn working_readback_before_receipt_begun_recovers_interrupted_without_callback() {
    let report = execute(Scenario::fake(vec![
        Action::SeedReceipt {
            state: SeedReceiptState::TaskBoundNotBegun,
            cancel_requested: false,
            staged_terminal: None,
        },
        Action::SeedTask {
            status: TaskStatus::Working,
            cancel_requested: false,
            receipt_link: ReceiptLinkCase::Exact,
            identity: IdentityRelation::Exact,
            version: 2,
        },
        Action::Crash {
            point: CrashPoint::AfterWorkingReadbackBeforeReceiptBegun,
        },
        Action::Restart,
        checkpoint_action("recovered"),
    ]));

    let recovered = checkpoint(&report, "recovered");
    assert_eq!(only_task(recovered).status, TaskStatus::Failed);
    assert_failed_terminal(
        only_task(recovered)
            .terminal
            .as_ref()
            .expect("interrupted Task terminal"),
        V5SafeFailureReason::Interrupted,
    );
    assert_eq!(
        task_link_state(only_task_link(recovered)),
        SeedReceiptState::TaskTerminalBound
    );
    assert!(recovered.receipts.is_empty());
    assert_terminal_bound_link(
        only_task_link(recovered),
        only_task(recovered),
        only_task(recovered).terminal.as_ref().unwrap(),
    );
    assert_eq!(recovered.callbacks.total_domain(), 0);
}

#[test]
fn receipt_begun_before_prepare_recovers_outcome_uncertain_without_callback() {
    let report = execute(Scenario::fake(vec![
        Action::SeedReceipt {
            state: SeedReceiptState::TaskBoundBegun,
            cancel_requested: false,
            staged_terminal: None,
        },
        Action::SeedTask {
            status: TaskStatus::Working,
            cancel_requested: false,
            receipt_link: ReceiptLinkCase::Exact,
            identity: IdentityRelation::Exact,
            version: 2,
        },
        Action::Crash {
            point: CrashPoint::AfterReceiptBegunBeforePrepare,
        },
        Action::Restart,
        checkpoint_action("recovered"),
    ]));

    let recovered = checkpoint(&report, "recovered");
    assert_eq!(only_task(recovered).status, TaskStatus::Failed);
    assert_failed_terminal(
        only_task(recovered)
            .terminal
            .as_ref()
            .expect("uncertain Task terminal"),
        V5SafeFailureReason::OutcomeUncertain,
    );
    assert_eq!(
        task_link_state(only_task_link(recovered)),
        SeedReceiptState::TaskTerminalBound
    );
    assert!(recovered.receipts.is_empty());
    assert_terminal_bound_link(
        only_task_link(recovered),
        only_task(recovered),
        only_task(recovered).terminal.as_ref().unwrap(),
    );
    assert_eq!(recovered.callbacks.total_domain(), 0);
}

#[test]
fn begun_receipt_with_queued_task_is_fail_stop() {
    for (status, identity) in [
        (TaskStatus::Queued, IdentityRelation::Exact),
        (TaskStatus::Working, IdentityRelation::Foreign),
    ] {
        let report = execute(Scenario::fake(vec![
            Action::SeedReceipt {
                state: SeedReceiptState::TaskBoundBegun,
                cancel_requested: false,
                staged_terminal: None,
            },
            Action::SeedTask {
                status,
                cancel_requested: false,
                receipt_link: ReceiptLinkCase::Exact,
                identity,
                version: 1,
            },
            Action::OpenTaskStoreInspectOnly,
            Action::ReconcileStartup,
            checkpoint_action("failed-startup"),
        ]));

        let failed = checkpoint(&report, "failed-startup");
        assert_eq!(only_task(failed).status, status);
        assert_eq!(
            task_link_state(only_task_link(failed)),
            SeedReceiptState::TaskBoundBegun
        );
        assert!(failed.receipts.is_empty());
        assert_eq!(failed.task_store_mutations, 0);
        assert_eq!(failed.receipt_store_mutations, 0);
        assert_eq!(failed.listener, ListenerState::NotPublished);
        assert!(failed.restart_requested);
        assert_eq!(failed.callbacks.total_domain(), 0);
    }
}

#[test]
fn task_bound_false_masks_working_as_queued_until_receipt_begun() {
    let report = execute(Scenario::fake(vec![
        direct_provider(),
        Action::InstallBarrier {
            point: BarrierPoint::AdmissionEntered,
        },
        Action::InstallBarrier {
            point: BarrierPoint::BeforeReceiptBegun,
        },
        submit("submit"),
        Action::WaitForEvent {
            event: EventKind::AdmissionEntered,
        },
        Action::AdvanceMonotonic { millis: CUTOFF_MS },
        Action::ReleaseBarrier {
            point: BarrierPoint::AdmissionEntered,
        },
        Action::WaitForEvent {
            event: EventKind::TaskStoreWorkingReadback,
        },
        Action::ReadTask {
            api: TaskApi::NativeGet,
            label: "native-before-begun".to_string(),
        },
        Action::ReadTask {
            api: TaskApi::CompatibilityGet,
            label: "compat-before-begun".to_string(),
        },
        checkpoint_action("working-masked"),
        Action::ReleaseBarrier {
            point: BarrierPoint::BeforeReceiptBegun,
        },
        Action::WaitForEvent {
            event: EventKind::ReceiptBegunCommitted,
        },
        Action::ReadTask {
            api: TaskApi::NativeGet,
            label: "after-begun".to_string(),
        },
    ]));

    assert_eq!(
        only_task(checkpoint(&report, "working-masked")).status,
        TaskStatus::Working
    );
    assert_eq!(
        task_read(&report, "native-before-begun").status,
        TaskStatus::Queued
    );
    assert_eq!(
        task_read(&report, "compat-before-begun").status,
        TaskStatus::Queued
    );
    assert_eq!(
        task_read(&report, "after-begun").status,
        TaskStatus::Working
    );
    assert_eq!(count_event(&report, EventKind::ReceiptBegunCommitted), 1);
}

#[test]
fn bound_false_cancel_flag_recovers_cancelled_without_callback() {
    let cancelled_report = execute(Scenario::fake(vec![
        Action::SeedReceipt {
            state: SeedReceiptState::TaskBoundNotBegun,
            cancel_requested: false,
            staged_terminal: None,
        },
        Action::SeedTask {
            status: TaskStatus::Working,
            cancel_requested: true,
            receipt_link: ReceiptLinkCase::Exact,
            identity: IdentityRelation::Exact,
            version: 2,
        },
        Action::Restart,
        checkpoint_action("cancelled"),
    ]));
    let cancelled = checkpoint(&cancelled_report, "cancelled");
    assert_eq!(only_task(cancelled).status, TaskStatus::Cancelled);
    assert_cancelled_terminal(
        only_task(cancelled)
            .terminal
            .as_ref()
            .expect("cancelled Task terminal"),
    );
    assert_eq!(cancelled.callbacks.total_domain(), 0);

    let normal_report = execute(Scenario::fake(vec![
        direct_provider(),
        Action::InstallBarrier {
            point: BarrierPoint::AdmissionEntered,
        },
        submit("submit"),
        Action::WaitForEvent {
            event: EventKind::AdmissionEntered,
        },
        Action::AdvanceMonotonic { millis: CUTOFF_MS },
        Action::ReleaseBarrier {
            point: BarrierPoint::AdmissionEntered,
        },
        Action::WaitForEvent {
            event: EventKind::ReceiptBegunCommitted,
        },
    ]));
    assert_event_order(
        &normal_report,
        &[
            EventKind::TaskStoreWorkingReadback,
            EventKind::ReceiptBegunCommitted,
            EventKind::PrepareEntered,
        ],
    );
    let authorization = observed_actor_authorization(&normal_report, "submit");
    assert_eq!(
        authorization.purpose,
        ActorAuthorizationPurpose::BoundTaskStart
    );
    assert_eq!(authorization.decision, ActorAuthorizationDecision::Accepted);
    let binding = observed_actor_binding(&normal_report, "submit");
    let context = authorization
        .task_bound_context
        .as_ref()
        .expect("accepted bound start must preserve exact bound-link context");
    assert_eq!(
        context.consumed_binding_token_fingerprint,
        binding.binding_token_fingerprint
    );
    assert_eq!(
        context.consumed_task_link_reservation_fingerprint,
        binding
            .task_binding
            .as_ref()
            .expect("bound start requires a consumed Task-link reservation")
            .task_link_reservation_fingerprint
    );
    let task_binding = binding.task_binding.as_ref().unwrap();
    assert_eq!(context.task_link_digest, task_binding.task_link_digest);
    assert_eq!(
        context.task_bound_link_authorization_fingerprint,
        task_binding.task_bound_link_authorization_fingerprint
    );
    let token_consumption = binding
        .binding_token_consumption
        .as_ref()
        .expect("bound Task start must consume the actor binding token after ActorBound");
    let (token_consumed_sequence, consumed_expected_version, consumed_committed_version) =
        match token_consumption {
            BindingTokenConsumptionObservation::AuthorizeBoundTaskStart {
                consumed_sequence,
                lifecycle_link_expected_version,
                lifecycle_link_committed_version,
            } => (
                *consumed_sequence,
                *lifecycle_link_expected_version,
                *lifecycle_link_committed_version,
            ),
            BindingTokenConsumptionObservation::MarkReservedBegun { .. } => {
                panic!("bound Task start cannot consume a Direct receipt token")
            }
        };
    assert_eq!(consumed_expected_version, context.lifecycle_link_version);
    assert_eq!(consumed_committed_version, consumed_expected_version + 1);
    let post_working = authorization
        .post_working_authorization
        .as_ref()
        .expect("accepted bound start must mint its one-shot post-Working authorization");
    assert!(token_consumed_sequence < post_working.minted_sequence);
    assert_eq!(
        post_working.mark_begun_expected_lifecycle_link_version,
        Some(consumed_committed_version)
    );
}

#[test]
fn bound_task_start_rejects_missing_foreign_stale_actor_proof_without_mutation() {
    let proofs = [
        ActorProofCase::Missing,
        ActorProofCase::Foreign,
        ActorProofCase::Stale,
    ];
    for proof in proofs {
        let label = format!("proof-{proof:?}").to_lowercase();
        let report = execute(Scenario::fake(vec![
            Action::SeedReceipt {
                state: SeedReceiptState::TaskBoundNotBegun,
                cancel_requested: false,
                staged_terminal: None,
            },
            Action::SeedTask {
                status: TaskStatus::Queued,
                cancel_requested: false,
                receipt_link: ReceiptLinkCase::Exact,
                identity: IdentityRelation::Exact,
                version: 1,
            },
            checkpoint_action("before-proof"),
            Action::AttemptBoundTaskStart {
                proof,
                label: label.clone(),
            },
            checkpoint_action("after-proof"),
        ]));
        let before = checkpoint(&report, "before-proof");
        let after = checkpoint(&report, "after-proof");
        assert_eq!(before.tasks, after.tasks, "{proof:?}");
        assert_eq!(before.receipts, after.receipts, "{proof:?}");
        assert_eq!(before.task_links, after.task_links, "{proof:?}");
        assert_eq!(
            before.task_store_mutations, after.task_store_mutations,
            "{proof:?}"
        );
        assert_eq!(
            before.receipt_store_mutations, after.receipt_store_mutations,
            "{proof:?}"
        );
        assert_eq!(after.callbacks.prepare, 0, "{proof:?}");
        assert_eq!(after.callbacks.execute, 0, "{proof:?}");
        assert_eq!(response(&report, &label).kind, ResponseKind::Rejected);
        let authorization = observed_actor_authorization(&report, &label);
        assert_eq!(
            authorization.purpose,
            ActorAuthorizationPurpose::BoundTaskStart
        );
        assert_eq!(
            authorization.decision,
            match proof {
                ActorProofCase::Missing => ActorAuthorizationDecision::Missing,
                ActorProofCase::Foreign => ActorAuthorizationDecision::Foreign,
                ActorProofCase::Stale => ActorAuthorizationDecision::Stale,
                ActorProofCase::Exact => unreachable!(),
            }
        );
        let context = authorization.task_bound_context.as_ref().unwrap();
        assert_eq!(context.receipt_key, only_task_link(before).key);
        assert_eq!(context.task_id, only_task(before).task_id);
        assert_eq!(context.task_link_digest, before.task_links[0].link_digest);
        assert_eq!(context.task_version, only_task(before).version);
        assert_eq!(
            context.lifecycle_link_version,
            only_task_link(before).version
        );
    }
}

#[test]
fn bound_task_start_rechecks_proof_after_working_readback() {
    let report = execute(Scenario::fake(vec![
        direct_provider(),
        Action::InstallBarrier {
            point: BarrierPoint::AdmissionEntered,
        },
        submit("submit"),
        Action::WaitForEvent {
            event: EventKind::AdmissionEntered,
        },
        Action::AdvanceMonotonic { millis: CUTOFF_MS },
        Action::ReleaseBarrier {
            point: BarrierPoint::AdmissionEntered,
        },
        Action::InvalidateActorProof {
            proof: ActorProofCase::Stale,
            point: BarrierPoint::AfterWorkingReadback,
            label: "stale-after-working-authorization".to_string(),
        },
        Action::WaitForEvent {
            event: EventKind::TaskStoreWorkingReadback,
        },
        checkpoint_action("stale-after-working"),
        Action::Restart,
        checkpoint_action("recovered"),
    ]));

    let stale = checkpoint(&report, "stale-after-working");
    assert_eq!(
        task_link_state(only_task_link(stale)),
        SeedReceiptState::TaskBoundNotBegun
    );
    assert!(stale.receipts.is_empty());
    assert_eq!(only_task(stale).status, TaskStatus::Working);
    assert!(stale.restart_requested);
    assert_eq!(stale.callbacks.prepare, 0);
    assert_eq!(stale.callbacks.execute, 0);
    let stale_authorization =
        observed_actor_authorization(&report, "stale-after-working-authorization");
    assert_eq!(
        stale_authorization.purpose,
        ActorAuthorizationPurpose::BoundTaskStart
    );
    assert_eq!(
        stale_authorization.decision,
        ActorAuthorizationDecision::Stale
    );
    let post_working = stale_authorization
        .post_working_authorization
        .as_ref()
        .expect(
            "stale-after-readback must preserve the minted post-working authorization evidence",
        );
    assert!(post_working.consumed_sequence.is_none());
    assert_eq!(post_working.task_id, only_task(stale).task_id);
    let recovered = checkpoint(&report, "recovered");
    assert_failed_terminal(
        only_task(recovered)
            .terminal
            .as_ref()
            .expect("interrupted Task terminal"),
        V5SafeFailureReason::Interrupted,
    );
}

#[test]
fn bound_actor_lease_is_retained_until_terminal_or_process_fail_stop() {
    let report = execute(Scenario::fake(vec![
        direct_provider(),
        Action::InstallBarrier {
            point: BarrierPoint::BeforePrepare,
        },
        submit("direct"),
        Action::WaitForEvent {
            event: EventKind::ReceiptBegunCommitted,
        },
        checkpoint_action("direct-live"),
        Action::ReleaseBarrier {
            point: BarrierPoint::BeforePrepare,
        },
        Action::WaitForEvent {
            event: EventKind::ReceiptTerminalCommitted,
        },
        checkpoint_action("normal-terminal"),
        Action::Reset,
        known_long_provider(),
        Action::InstallBarrier {
            point: BarrierPoint::BeforeTaskTerminalReceipt,
        },
        submit("task"),
        Action::WaitForEvent {
            event: EventKind::TaskBoundCommitted,
        },
        checkpoint_action("task-live"),
        Action::ReleaseBarrier {
            point: BarrierPoint::BeforeTaskTerminalReceipt,
        },
        Action::WaitForEvent {
            event: EventKind::TaskTerminalBoundCommitted,
        },
        checkpoint_action("task-terminal"),
        Action::Reset,
        Action::ConfigureProvider {
            execution_class: ExecutionClass::Direct,
            terminal: success_payload(),
            cooperative_cancel: false,
            side_effect_marker: false,
        },
        Action::InstallBarrier {
            point: BarrierPoint::PrepareEntered,
        },
        submit("noncooperative"),
        Action::WaitForEvent {
            event: EventKind::PrepareEntered,
        },
        checkpoint_action("noncooperative-before-exit"),
        Action::Cancel {
            key: KeyCase::Exact,
            lazy_session: true,
            label: "noncooperative-cancel".to_string(),
        },
        Action::AdvanceMonotonic {
            millis: CLEANUP_GRACE_MS,
        },
        checkpoint_action("noncooperative-after-exit"),
    ]));

    assert_eq!(checkpoint(&report, "direct-live").actor_leases, 1);
    assert_eq!(checkpoint(&report, "task-live").actor_leases, 1);
    assert_eq!(checkpoint(&report, "normal-terminal").actor_leases, 0);
    assert_eq!(checkpoint(&report, "task-terminal").actor_leases, 0);
    let blocked = checkpoint(&report, "noncooperative-before-exit");
    assert_eq!(blocked.actor_leases, 1);
    assert!(blocked.daemon_running);
    let exited = checkpoint(&report, "noncooperative-after-exit");
    assert!(!exited.daemon_running);
    assert_eq!(exited.actor_leases, 0);
    assert_eq!(exited.process_exit_elapsed_ms, Some(CLEANUP_GRACE_MS));
    assert_eq!(count_event(&report, EventKind::LeaseReleased), 3);
    assert_eq!(count_event(&report, EventKind::ListenerClosed), 1);
}

#[test]
fn direct_actor_bound_cancel_vs_mark_begun_has_one_linearized_winner() {
    let report = execute(Scenario::fake(vec![
        Action::SeedReceipt {
            state: SeedReceiptState::ReservedActorBound,
            cancel_requested: false,
            staged_terminal: None,
        },
        Action::InstallBarrier {
            point: BarrierPoint::BeforeMarkReservedBegunGateAcquire,
        },
        Action::SpawnMarkReservedBegun {
            proof: ActorProofCase::Exact,
            label: "cancel-first-mark".to_string(),
        },
        Action::WaitForOperation {
            label: "cancel-first-mark".to_string(),
            state: OperationState::Blocked,
        },
        Action::WaitForEvent {
            event: EventKind::MarkReservedBegunBlocked,
        },
        Action::SpawnCancel {
            key: KeyCase::Exact,
            lazy_session: false,
            label: "cancel-first-cancel".to_string(),
        },
        Action::WaitForOperation {
            label: "cancel-first-cancel".to_string(),
            state: OperationState::Completed,
        },
        Action::JoinOperation {
            label: "cancel-first-cancel".to_string(),
        },
        Action::ReleaseBarrier {
            point: BarrierPoint::BeforeMarkReservedBegunGateAcquire,
        },
        Action::JoinOperation {
            label: "cancel-first-mark".to_string(),
        },
        checkpoint_action("cancel-first"),
        Action::Reset,
        Action::SeedReceipt {
            state: SeedReceiptState::ReservedActorBound,
            cancel_requested: false,
            staged_terminal: None,
        },
        Action::InstallBarrier {
            point: BarrierPoint::BeforeCancelGateAcquire,
        },
        Action::SpawnCancel {
            key: KeyCase::Exact,
            lazy_session: false,
            label: "begun-first-cancel".to_string(),
        },
        Action::WaitForOperation {
            label: "begun-first-cancel".to_string(),
            state: OperationState::Blocked,
        },
        Action::WaitForEvent {
            event: EventKind::CancelCommitBlocked,
        },
        Action::SpawnMarkReservedBegun {
            proof: ActorProofCase::Exact,
            label: "begun-first-mark".to_string(),
        },
        Action::WaitForOperation {
            label: "begun-first-mark".to_string(),
            state: OperationState::Completed,
        },
        Action::JoinOperation {
            label: "begun-first-mark".to_string(),
        },
        Action::ReleaseBarrier {
            point: BarrierPoint::BeforeCancelGateAcquire,
        },
        Action::JoinOperation {
            label: "begun-first-cancel".to_string(),
        },
        checkpoint_action("begun-first"),
    ]));

    let cancel_first = checkpoint(&report, "cancel-first");
    assert_eq!(
        only_receipt(cancel_first).state,
        SeedReceiptState::DirectTerminalUnacked
    );
    assert_cancelled_terminal(terminal_of_receipt(only_receipt(cancel_first)));
    assert!(!only_receipt(cancel_first).begun);
    assert_eq!(cancel_first.callbacks.prepare, 0);
    assert_eq!(cancel_first.callbacks.execute, 0);
    assert_eq!(cancel_first.token_signals, 0);

    let begun_first = checkpoint(&report, "begun-first");
    assert!(only_receipt(begun_first).begun);
    assert_eq!(
        only_receipt(begun_first).state,
        SeedReceiptState::ReservedBegun
    );
    assert!(only_receipt(begun_first).terminal.is_none());
    assert_eq!(begun_first.callbacks.prepare, 0);
    assert_eq!(begun_first.token_signals, 1);
    assert_eq!(count_event(&report, EventKind::ReceiptBegunCommitted), 1);
    let begun_authorization = observed_actor_authorization(&report, "begun-first-mark");
    assert_eq!(
        begun_authorization.purpose,
        ActorAuthorizationPurpose::ReservedBegin
    );
    assert_eq!(
        begun_authorization.decision,
        ActorAuthorizationDecision::Accepted
    );
    assert_eq!(
        begun_authorization.ledger_authorization.receipt_key,
        only_receipt(begun_first).key
    );
    assert_eq!(count_event(&report, EventKind::MarkReservedBegunBlocked), 1);
    assert_eq!(count_event(&report, EventKind::CancelCommitBlocked), 1);
    assert!(count_event(&report, EventKind::OperationCompleted) >= 4);
    assert_operation_trace(
        &report,
        "cancel-first-mark",
        &[
            OperationEventState::Spawned,
            OperationEventState::Blocked,
            OperationEventState::Completed,
            OperationEventState::Joined,
        ],
    );
    assert_operation_trace(
        &report,
        "cancel-first-cancel",
        &[
            OperationEventState::Spawned,
            OperationEventState::Completed,
            OperationEventState::Joined,
        ],
    );
    assert_operation_trace(
        &report,
        "begun-first-cancel",
        &[
            OperationEventState::Spawned,
            OperationEventState::Blocked,
            OperationEventState::Completed,
            OperationEventState::Joined,
        ],
    );
    assert_operation_trace(
        &report,
        "begun-first-mark",
        &[
            OperationEventState::Spawned,
            OperationEventState::Completed,
            OperationEventState::Joined,
        ],
    );
    assert_gate_trace(
        &report,
        "cancel-first-mark",
        &[
            GateTransition::Waiting,
            GateTransition::Acquired,
            GateTransition::Released,
        ],
    );
    assert_gate_trace(
        &report,
        "begun-first-cancel",
        &[
            GateTransition::Waiting,
            GateTransition::Acquired,
            GateTransition::Released,
        ],
    );
}

#[test]
fn task_terminal_receipt_crash_reconciles_without_replay() {
    let report = execute(Scenario::fake(vec![
        known_long_provider(),
        submit("submit"),
        Action::WaitForEvent {
            event: EventKind::TaskStoreTerminalCommitted,
        },
        Action::Crash {
            point: CrashPoint::AfterTaskStoreTerminalBeforeLifecycleLinkTerminal,
        },
        Action::Restart,
        Action::ReadTask {
            api: TaskApi::NativeGet,
            label: "recovered".to_string(),
        },
        checkpoint_action("after-reconcile"),
    ]));

    let reconciled = checkpoint(&report, "after-reconcile");
    assert_eq!(only_task(reconciled).status, TaskStatus::Completed);
    assert!(
        completed_result(
            only_task(reconciled)
                .terminal
                .as_ref()
                .expect("reconciled Task terminal")
        )
        .ok
    );
    assert_eq!(
        task_link_state(only_task_link(reconciled)),
        SeedReceiptState::TaskTerminalBound
    );
    assert!(reconciled.receipts.is_empty());
    assert_terminal_bound_link(
        only_task_link(reconciled),
        only_task(reconciled),
        only_task(reconciled).terminal.as_ref().unwrap(),
    );
    assert_eq!(reconciled.callbacks.prepare, 1);
    assert_eq!(reconciled.callbacks.execute, 1);
    assert_eq!(count_event(&report, EventKind::ExecuteEntered), 1);
    assert_eq!(task_read(&report, "recovered"), only_task(reconciled));
}

#[test]
fn cancel_before_submit_is_full_key_bounded_and_expires() {
    let report = execute(Scenario::fake(vec![
        Action::Cancel {
            key: KeyCase::Exact,
            lazy_session: true,
            label: "reserve".to_string(),
        },
        checkpoint_action("reserved"),
        Action::AdvanceEpoch { millis: 1_000 },
        Action::Cancel {
            key: KeyCase::Exact,
            lazy_session: true,
            label: "duplicate-cancel".to_string(),
        },
        Action::Cancel {
            key: KeyCase::Mismatch(IdentityField::NormalizedArgumentsHash),
            lazy_session: true,
            label: "mismatch-cancel".to_string(),
        },
        checkpoint_action("after-duplicate"),
        Action::AdvanceEpoch {
            millis: CANCEL_RESERVATION_TTL_MS - 1_001,
        },
        checkpoint_action("one-ms-before-submit-expiry"),
        submit("exact-submit"),
        checkpoint_action("after-submit"),
    ]));

    let reserved = only_receipt(checkpoint(&report, "reserved"));
    let after_duplicate = only_receipt(checkpoint(&report, "after-duplicate"));
    assert_eq!(reserved.state, SeedReceiptState::CancelReserved);
    assert!(reserved.cancel_requested);
    assert_eq!(
        reserved.expires_epoch_ms,
        Some(reserved.accepted_epoch_ms + CANCEL_RESERVATION_TTL_MS)
    );
    assert_eq!(reserved.expires_epoch_ms, after_duplicate.expires_epoch_ms);
    assert_eq!(reserved.key, after_duplicate.key);
    assert_eq!(
        checkpoint(&report, "one-ms-before-submit-expiry").epoch_ms + 1,
        reserved
            .expires_epoch_ms
            .expect("CancelReserved has absolute expiry")
    );
    assert_eq!(checkpoint(&report, "after-duplicate").receipt_live_count, 1);
    assert_eq!(
        response(&report, "mismatch-cancel").kind,
        ResponseKind::Rejected
    );
    assert_eq!(
        response(&report, "mismatch-cancel").error,
        Some(ErrorCode::InvocationIdentityMismatch)
    );
    let after_submit = checkpoint(&report, "after-submit");
    assert_eq!(
        only_receipt(after_submit).state,
        SeedReceiptState::DirectTerminalUnacked
    );
    assert_cancelled_terminal(terminal_of_receipt(only_receipt(after_submit)));
    assert_eq!(
        response_key(response(&report, "exact-submit")),
        &reserved.key
    );
    assert_eq!(after_submit.callbacks.total_domain(), 0);
    assert_eq!(
        count_event(&report, EventKind::V5ReceiptRuntimeEntered),
        4,
        "one runtime-entry event must come from each authenticated action frame"
    );
    assert_eq!(
        count_event(&report, EventKind::CancelReservationConverted),
        1
    );
    assert_eq!(count_event(&report, EventKind::ReceiptTerminalCommitted), 1);
}

#[test]
fn cancel_reserved_reopens_with_original_7125ms_expiry() {
    let report = execute(Scenario::fake(vec![
        Action::Cancel {
            key: KeyCase::Exact,
            lazy_session: true,
            label: "initial".to_string(),
        },
        checkpoint_action("initial"),
        Action::AdvanceEpoch { millis: 3_000 },
        Action::Restart,
        Action::Cancel {
            key: KeyCase::Exact,
            lazy_session: true,
            label: "duplicate-after-reopen".to_string(),
        },
        checkpoint_action("reopened"),
        Action::AdvanceEpoch {
            millis: CANCEL_RESERVATION_TTL_MS - 3_001,
        },
        checkpoint_action("one-ms-before-expiry"),
        Action::AdvanceEpoch { millis: 1 },
        Action::Recover {
            key: KeyCase::Exact,
            label: "after-expiry".to_string(),
        },
        checkpoint_action("expired"),
    ]));

    let initial = only_receipt(checkpoint(&report, "initial"));
    let reopened = only_receipt(checkpoint(&report, "reopened"));
    assert_eq!(initial.expires_epoch_ms, reopened.expires_epoch_ms);
    assert_eq!(initial.accepted_epoch_ms, reopened.accepted_epoch_ms);
    assert_eq!(initial.mutation_sequence, reopened.mutation_sequence);
    assert_eq!(
        checkpoint(&report, "one-ms-before-expiry").receipt_live_count,
        1
    );
    assert_eq!(checkpoint(&report, "expired").receipt_live_count, 0);
    assert!(checkpoint(&report, "expired").receipts.is_empty());
    assert!(matches!(
        response(&report, "after-expiry").error,
        Some(ErrorCode::ReceiptExpired | ErrorCode::ReceiptNotFound)
    ));
    assert_eq!(checkpoint(&report, "expired").callbacks.total_domain(), 0);
    assert_eq!(
        count_event(&report, EventKind::V5ReceiptRuntimeEntered),
        3,
        "reopen markers and checkpoints must not invent action-frame events"
    );
    assert_eq!(
        count_event(&report, EventKind::CancelReservationConverted),
        0
    );
    assert_eq!(count_event(&report, EventKind::ReceiptTerminalCommitted), 0);
}

#[test]
fn cancel_reserved_shares_live_64_count_without_result_reservation() {
    let report = execute(Scenario::fake(vec![
        Action::FillReceiptPool {
            state: SeedReceiptState::CancelReserved,
            count: LIVE_RECEIPT_LIMIT as u32,
        },
        checkpoint_action("cancel-pool-full"),
        Action::Cancel {
            key: KeyCase::Unknown,
            lazy_session: true,
            label: "sixty-fifth-cancel".to_string(),
        },
        checkpoint_action("after-overflow"),
        Action::Reset,
        Action::FillReceiptPool {
            state: SeedReceiptState::CancelReserved,
            count: 1,
        },
        Action::FillReceiptPool {
            state: SeedReceiptState::ReservedUnbound,
            count: 62,
        },
        checkpoint_action("before-conversion"),
        Action::InstallBarrier {
            point: BarrierPoint::AfterCancelReservationConvertedBeforeTerminal,
        },
        submit("converted-submit"),
        Action::WaitForEvent {
            event: EventKind::CancelReservationConverted,
        },
        checkpoint_action("conversion-full-reserve"),
        Action::ReleaseBarrier {
            point: BarrierPoint::AfterCancelReservationConvertedBeforeTerminal,
        },
        Action::WaitForEvent {
            event: EventKind::ReceiptTerminalCommitted,
        },
        checkpoint_action("after-conversion"),
    ]));

    let full = checkpoint(&report, "cancel-pool-full");
    assert_eq!(full.receipt_live_count, LIVE_RECEIPT_LIMIT);
    assert_eq!(full.receipt_reserved_bytes, 0);
    assert!(full.receipt_actual_bytes <= LIVE_RECEIPT_LIMIT * 1_024);
    assert_eq!(
        response(&report, "sixty-fifth-cancel").kind,
        ResponseKind::Rejected
    );
    assert_eq!(
        response(&report, "sixty-fifth-cancel").error,
        Some(ErrorCode::ReceiptCapacity)
    );
    let before = checkpoint(&report, "before-conversion");
    assert_eq!(before.receipt_live_count, LIVE_RECEIPT_LIMIT - 1);
    let cancel_metadata: Vec<_> = before
        .receipts
        .iter()
        .filter(|receipt| receipt.state == SeedReceiptState::CancelReserved)
        .collect();
    assert_eq!(cancel_metadata.len(), 1);
    assert!(cancel_metadata[0].encoded_bytes > 0);
    assert_eq!(cancel_metadata[0].reserved_result_bytes, 0);
    let full_entitlements: Vec<_> = before
        .receipts
        .iter()
        .filter(|receipt| receipt.state == SeedReceiptState::ReservedUnbound)
        .collect();
    assert_eq!(full_entitlements.len(), 62);
    full_entitlements
        .iter()
        .for_each(|receipt| assert_max_result_entitlement(receipt));
    assert_eq!(
        receipt_quota_bytes(before),
        cancel_metadata[0].encoded_bytes + 62 * MAX_RESPONSE_LINE_BYTES
    );
    assert!(
        before.receipt_actual_bytes + before.receipt_reserved_bytes <= LIVE_RECEIPT_BYTES_LIMIT,
        "CancelReserved metadata must fit in the one-record headroom"
    );
    let converting = checkpoint(&report, "conversion-full-reserve");
    assert_eq!(converting.receipt_live_count, LIVE_RECEIPT_LIMIT - 1);
    assert_eq!(
        receipt_quota_bytes(converting),
        (LIVE_RECEIPT_LIMIT - 1) * MAX_RESPONSE_LINE_BYTES
    );
    assert!(receipt_quota_bytes(converting) <= LIVE_RECEIPT_BYTES_LIMIT);
    let converted_in_flight: Vec<_> = converting
        .receipts
        .iter()
        .filter(|receipt| receipt.cancel_requested)
        .collect();
    assert_eq!(converted_in_flight.len(), 1);
    assert_max_result_entitlement(converted_in_flight[0]);
    assert_eq!(
        converted_in_flight[0].state,
        SeedReceiptState::ReservedUnbound
    );
    assert_eq!(converting.listener, ListenerState::Listening);
    assert!(converting.daemon_running);
    assert_eq!(converting.actor_leases, 1);
    assert_eq!(converting.callbacks.total_domain(), 0);
    let after = checkpoint(&report, "after-conversion");
    assert_eq!(after.receipt_live_count, LIVE_RECEIPT_LIMIT - 1);
    assert!(after.receipt_actual_bytes + after.receipt_reserved_bytes <= LIVE_RECEIPT_BYTES_LIMIT);
    assert_eq!(after.callbacks.total_domain(), 0);
    assert_eq!(
        response(&report, "converted-submit").kind,
        ResponseKind::Cancelled
    );
    let converted_terminal = after
        .receipts
        .iter()
        .find(|receipt| receipt.cancel_requested)
        .expect("converted terminal must remain durably observable");
    assert_max_result_entitlement(converted_terminal);
    assert_cancelled_terminal(terminal_of_receipt(converted_terminal));
    assert_exact_response_identity(response(&report, "converted-submit"), converted_terminal);
    assert_eq!(
        response_terminal(response(&report, "converted-submit")),
        terminal_of_receipt(converted_terminal)
    );
    assert_eq!(after.listener, ListenerState::Closed);
    assert!(!after.daemon_running);
    assert_eq!(after.actor_leases, 0);
    assert_eq!(
        count_event(&report, EventKind::V5ReceiptRuntimeEntered),
        129,
        "batched actions must report every authenticated action frame"
    );
    assert_event_order(
        &report,
        &[
            EventKind::V5ReceiptRuntimeEntered,
            EventKind::CancelReservationConverted,
            EventKind::ReceiptTerminalCommitted,
        ],
    );
    assert_eq!(
        count_event(&report, EventKind::CancelReservationConverted),
        1
    );
    assert_eq!(count_event(&report, EventKind::ReceiptTerminalCommitted), 1);
}

#[test]
fn cancel_flag_transfers_monotonically_at_task_bind() {
    let paths = [
        EntryPath::PromisedUnbound,
        EntryPath::ReservedActorBound,
        EntryPath::ReservedBegun,
    ];
    let crash_points = [
        CrashPoint::AfterCancelFlagBeforeTaskCreate,
        CrashPoint::AfterTaskStoreCancelReadbackBeforeTaskBound,
    ];
    let crash_cases = paths
        .iter()
        .flat_map(|path| {
            crash_points.iter().map(|point| CrashWorkload {
                path: *path,
                point: *point,
                cancel_before_crash: true,
                stage_terminal_before_crash: false,
            })
        })
        .collect();
    let report = execute(Scenario::fake(vec![
        Action::SeedReceipt {
            state: SeedReceiptState::TaskPromisedActorBound,
            cancel_requested: false,
            staged_terminal: None,
        },
        Action::Cancel {
            key: KeyCase::Exact,
            lazy_session: false,
            label: "cancel-before-bind".to_string(),
        },
        checkpoint_action("cancel-transfer-before-bind"),
        Action::InstallBarrier {
            point: BarrierPoint::AfterTaskStoreReadbackBeforeTaskBound,
        },
        Action::SpawnTaskStoreCreateAndBindUnderGate {
            label: "bind-under-gate".to_string(),
        },
        Action::WaitForEvent {
            event: EventKind::TaskStoreReadbackBeforeBind,
        },
        Action::WaitForOperation {
            label: "bind-under-gate".to_string(),
            state: OperationState::Blocked,
        },
        checkpoint_action("cancel-transfer-after-readback"),
        Action::SpawnCancel {
            key: KeyCase::Exact,
            lazy_session: false,
            label: "cancel-during-bind".to_string(),
        },
        Action::WaitForOperation {
            label: "cancel-during-bind".to_string(),
            state: OperationState::Blocked,
        },
        checkpoint_action("cancel-transfer-gate-held"),
        Action::ReleaseBarrier {
            point: BarrierPoint::AfterTaskStoreReadbackBeforeTaskBound,
        },
        Action::JoinOperation {
            label: "bind-under-gate".to_string(),
        },
        Action::JoinOperation {
            label: "cancel-during-bind".to_string(),
        },
        checkpoint_action("cancel-transfer-after-bind"),
        Action::RunCrossStoreCrashWorkload { cases: crash_cases },
    ]));

    let before = checkpoint(&report, "cancel-transfer-before-bind");
    assert_eq!(
        before.cancel_authority,
        Some(CancelAuthority::ReceiptLedger)
    );
    assert!(only_receipt(before).cancel_requested);
    let created = checkpoint(&report, "cancel-transfer-after-readback");
    assert_eq!(
        created.cancel_authority,
        Some(CancelAuthority::ReceiptLedger)
    );
    assert!(only_task(created).cancel_requested);
    assert_eq!(only_task(created).receipt_key, only_receipt(created).key);
    let gate_held = checkpoint(&report, "cancel-transfer-gate-held");
    assert_eq!(
        gate_held.cancel_authority,
        Some(CancelAuthority::ReceiptLedger)
    );
    assert_eq!(gate_held.tasks, created.tasks);
    assert_eq!(gate_held.receipts, created.receipts);
    let after = checkpoint(&report, "cancel-transfer-after-bind");
    assert_eq!(after.cancel_authority, Some(CancelAuthority::TaskStore));
    assert!(only_task(after).cancel_requested);
    assert_eq!(after.task_store_create_attempts, 1);
    assert_same_stable_task(only_task(created), only_task(after));
    assert!(after.receipts.is_empty());
    assert_eq!(after.task_links.len(), 1);
    assert_eq!(after.task_link_reserved_count, 0);
    let binding = observed_actor_binding(&report, "bind-under-gate");
    assert_eq!(binding.receipt_key, only_task_link(after).key);
    assert!(
        binding.binding_token_consumption.is_none(),
        "Task bind transfers authority but cannot consume the later start token"
    );
    let task_binding = binding
        .task_binding
        .as_ref()
        .expect("bound authority transfer must consume a reserved Task-link slot");
    assert_eq!(
        task_binding.task_link_digest,
        after.task_links[0].link_digest
    );
    assert!(
        task_binding.task_link_reserved_sequence < task_binding.task_store_create_sequence,
        "Task-link capacity must be durably reserved before any TaskStore create"
    );
    assert_operation_trace(
        &report,
        "bind-under-gate",
        &[
            OperationEventState::Spawned,
            OperationEventState::Blocked,
            OperationEventState::Completed,
            OperationEventState::Joined,
        ],
    );
    assert_operation_trace(
        &report,
        "cancel-during-bind",
        &[
            OperationEventState::Spawned,
            OperationEventState::Blocked,
            OperationEventState::Completed,
            OperationEventState::Joined,
        ],
    );
    assert_gate_trace(
        &report,
        "cancel-during-bind",
        &[
            GateTransition::Waiting,
            GateTransition::Acquired,
            GateTransition::Released,
        ],
    );
    assert_eq!(report.crash_cases.len(), paths.len() * crash_points.len());
    for path in paths {
        for point in crash_points {
            let matches: Vec<_> = report
                .crash_cases
                .iter()
                .filter(|case| case.path == path && case.point == point)
                .collect();
            assert_eq!(matches.len(), 1, "missing/duplicate {path:?}/{point:?}");
            let case = matches[0];
            assert_eq!(case.task_store_records.len(), 1);
            assert_eq!(case.projections.len(), 1);
            assert_eq!(case.task_store_records[0], case.projections[0]);
            let key = case.ledger.key();
            let link = case.ledger.lifecycle_link();
            assert_eq!(&case.projections[0].receipt_key, key);
            assert_eq!(case.projections[0].task_id, key.reserved_task_id);
            assert_eq!(case.projections[0].invocation_id, key.invocation_id);
            assert!(case.callback_invocation_ids.is_empty());
            if path == EntryPath::ReservedBegun {
                assert_failed_terminal(
                    &case.recovered_terminal,
                    V5SafeFailureReason::OutcomeUncertain,
                );
            } else {
                assert_cancelled_terminal(&case.recovered_terminal);
            }
            assert_eq!(
                case.projections[0].terminal.as_ref(),
                Some(&case.recovered_terminal)
            );
            assert_terminal_bound_link(link, &case.projections[0], &case.recovered_terminal);
        }
    }

    let closed_states = [
        (
            SeedReceiptState::CancelReserved,
            false,
            None,
            TerminalClass::Absent,
        ),
        (
            SeedReceiptState::ReservedUnbound,
            false,
            None,
            TerminalClass::Cancelled,
        ),
        (
            SeedReceiptState::ReservedActorBound,
            false,
            None,
            TerminalClass::Cancelled,
        ),
        (
            SeedReceiptState::ReservedBegun,
            false,
            None,
            TerminalClass::Absent,
        ),
        (
            SeedReceiptState::DirectTerminalUnacked,
            true,
            None,
            TerminalClass::Completed(true),
        ),
        (
            SeedReceiptState::AcknowledgedTombstone,
            true,
            None,
            TerminalClass::Absent,
        ),
        (
            SeedReceiptState::TaskPromisedUnbound,
            false,
            None,
            TerminalClass::Cancelled,
        ),
        (
            SeedReceiptState::TaskPromisedActorBound,
            false,
            None,
            TerminalClass::Cancelled,
        ),
        (
            SeedReceiptState::TaskHandoffActorBoundNotBegun,
            false,
            None,
            TerminalClass::Cancelled,
        ),
        (
            SeedReceiptState::TaskHandoffActorBoundBegun,
            false,
            None,
            TerminalClass::Absent,
        ),
        (
            SeedReceiptState::TaskHandoffActorBoundBegun,
            true,
            None,
            TerminalClass::Completed(true),
        ),
        (
            SeedReceiptState::TaskReceiptOwnedActorBound,
            false,
            None,
            TerminalClass::Absent,
        ),
        (
            SeedReceiptState::TaskTerminalReceiptBacked,
            true,
            None,
            TerminalClass::Completed(true),
        ),
        (
            SeedReceiptState::TaskBoundNotBegun,
            false,
            Some(TaskStatus::Queued),
            TerminalClass::Cancelled,
        ),
        (
            SeedReceiptState::TaskBoundBegun,
            false,
            Some(TaskStatus::Working),
            TerminalClass::Absent,
        ),
        (
            SeedReceiptState::TaskTerminalBound,
            true,
            Some(TaskStatus::Completed),
            TerminalClass::Completed(true),
        ),
    ];
    for (state, seeded_terminal, task_status, expected_terminal) in closed_states {
        let mut api_observations = Vec::new();
        for api in [TaskCancelApi::Native, TaskCancelApi::Compatibility] {
            let mut actions = vec![Action::SeedReceipt {
                state,
                cancel_requested: false,
                staged_terminal: seeded_terminal.then(success_payload),
            }];
            if let Some(status) = task_status {
                actions.push(Action::SeedTask {
                    status,
                    cancel_requested: false,
                    receipt_link: ReceiptLinkCase::Exact,
                    identity: IdentityRelation::Exact,
                    version: 17,
                });
            }
            actions.extend([
                Action::ReadTask {
                    api: TaskApi::NativeGet,
                    label: "before-cancel-selector".to_string(),
                },
                checkpoint_action("closed-before"),
                Action::CancelTask {
                    api,
                    task: match api {
                        TaskCancelApi::Native => TaskSelector::ExactProjected,
                        TaskCancelApi::Compatibility => {
                            TaskSelector::ForReadLabel("before-cancel-selector".to_string())
                        }
                    },
                    lazy_session: false,
                    label: "closed-cancel".to_string(),
                },
                Action::ReadTask {
                    api: TaskApi::CompatibilityGet,
                    label: "after-cancel-read".to_string(),
                },
                checkpoint_action("closed-after"),
            ]);
            let api_report = execute(Scenario::fake(actions));
            let before = checkpoint(&api_report, "closed-before");
            let after = checkpoint(&api_report, "closed-after");
            let cancel = response(&api_report, "closed-cancel");
            let before_read = task_read(&api_report, "before-cancel-selector");
            let after_read = task_read(&api_report, "after-cancel-read");
            assert_same_stable_task(before_read, after_read);
            assert_eq!(
                before_read.task_id,
                before_read.receipt_key.reserved_task_id
            );
            assert_eq!(after_read.task_id, after_read.receipt_key.reserved_task_id);
            if let Some(task) = &cancel.task {
                assert_same_stable_task(before_read, task);
            }
            let after_terminal = after
                .receipts
                .first()
                .and_then(|receipt| {
                    receipt
                        .terminal
                        .as_ref()
                        .or(receipt.staged_terminal.as_ref())
                })
                .or_else(|| after.tasks.first().and_then(|task| task.terminal.as_ref()));
            assert_eq!(
                terminal_class(after_terminal),
                expected_terminal,
                "{state:?}/{api:?}"
            );
            let before_terminal = before
                .receipts
                .first()
                .and_then(|receipt| {
                    receipt
                        .terminal
                        .as_ref()
                        .or(receipt.staged_terminal.as_ref())
                })
                .or_else(|| before.tasks.first().and_then(|task| task.terminal.as_ref()));
            if matches!(terminal_class(before_terminal), TerminalClass::Completed(_)) {
                assert_eq!(after_terminal, before_terminal, "terminal winner changed");
            } else {
                assert!(!matches!(
                    terminal_class(after_terminal),
                    TerminalClass::Completed(_)
                ));
            }
            if matches!(
                state,
                SeedReceiptState::DirectTerminalUnacked
                    | SeedReceiptState::TaskTerminalReceiptBacked
                    | SeedReceiptState::TaskTerminalBound
            ) {
                assert_eq!(before.receipts, after.receipts);
                assert_eq!(before.tasks, after.tasks);
                assert_eq!(before.task_links, after.task_links);
            }
            if state == SeedReceiptState::AcknowledgedTombstone {
                assert!(before.receipts.is_empty());
                assert_eq!(before.tombstones, after.tombstones);
            }
            api_observations.push((
                cancel.kind,
                cancel.error,
                terminal_class(cancel.terminal.as_ref()),
                after.receipts.first().map(|receipt| receipt.state),
                after.tasks.first().map(|task| task.status),
                terminal_class(after_terminal),
                after.tombstone_count,
            ));
        }
        assert_eq!(
            api_observations[0], api_observations[1],
            "native/compat cancel diverged for {state:?}"
        );
    }
}

#[test]
fn begun_handoff_cancel_crash_before_task_create_or_token_is_uncertain() {
    let report = execute(Scenario::fake(vec![
        direct_provider(),
        Action::InstallBarrier {
            point: BarrierPoint::PrepareEntered,
        },
        Action::InstallBarrier {
            point: BarrierPoint::BeforeTaskStoreCreate,
        },
        submit("submit"),
        Action::WaitForEvent {
            event: EventKind::PrepareEntered,
        },
        Action::AdvanceMonotonic { millis: CUTOFF_MS },
        Action::WaitForEvent {
            event: EventKind::BoundHandoffCommitted,
        },
        Action::Cancel {
            key: KeyCase::Exact,
            lazy_session: false,
            label: "cancel".to_string(),
        },
        Action::Crash {
            point: CrashPoint::AfterCancelFlagBeforeTaskCreate,
        },
        Action::Restart,
        checkpoint_action("recovered"),
    ]));

    let recovered = checkpoint(&report, "recovered");
    assert_eq!(recovered.tasks.len(), 1);
    assert_eq!(only_task(recovered).status, TaskStatus::Failed);
    assert!(only_task(recovered).cancel_requested);
    assert_failed_terminal(
        only_task(recovered)
            .terminal
            .as_ref()
            .expect("begun crash Task terminal"),
        V5SafeFailureReason::OutcomeUncertain,
    );
    assert_eq!(
        task_link_state(only_task_link(recovered)),
        SeedReceiptState::TaskTerminalBound
    );
    assert!(recovered.receipts.is_empty());
    assert_terminal_bound_link(
        only_task_link(recovered),
        only_task(recovered),
        only_task(recovered).terminal.as_ref().unwrap(),
    );
    assert_eq!(recovered.callbacks.prepare, 1);
    assert_eq!(recovered.callbacks.execute, 0);
    assert_eq!(count_event(&report, EventKind::PrepareEntered), 1);
    assert_eq!(recovered.task_store_create_attempts, 1);
}

#[test]
fn cancel_after_false_observation_before_atomic_working_wins_without_callback() {
    let report = execute(Scenario::fake(vec![
        direct_provider(),
        Action::InstallBarrier {
            point: BarrierPoint::AdmissionEntered,
        },
        Action::InstallBarrier {
            point: BarrierPoint::AfterFalseCancelObservation,
        },
        submit("submit"),
        Action::WaitForEvent {
            event: EventKind::AdmissionEntered,
        },
        Action::AdvanceMonotonic { millis: CUTOFF_MS },
        Action::ReleaseBarrier {
            point: BarrierPoint::AdmissionEntered,
        },
        Action::WaitForEvent {
            event: EventKind::FalseCancelObservationReached,
        },
        Action::Cancel {
            key: KeyCase::Exact,
            lazy_session: false,
            label: "cancel".to_string(),
        },
        Action::ReleaseBarrier {
            point: BarrierPoint::AfterFalseCancelObservation,
        },
        checkpoint_action("winner"),
    ]));

    let winner = checkpoint(&report, "winner");
    assert_ne!(only_task(winner).status, TaskStatus::Working);
    assert_eq!(only_task(winner).status, TaskStatus::Cancelled);
    assert!(only_task(winner).cancel_requested);
    assert_eq!(winner.callbacks.prepare, 0);
    assert_eq!(winner.callbacks.execute, 0);
    assert_eq!(count_event(&report, EventKind::ReceiptBegunCommitted), 0);
}

#[test]
fn cancel_after_working_before_receipt_begun_waits_and_is_post_begun() {
    let report = execute(Scenario::fake(vec![
        direct_provider(),
        Action::InstallBarrier {
            point: BarrierPoint::AdmissionEntered,
        },
        Action::InstallBarrier {
            point: BarrierPoint::AfterWorkingReadback,
        },
        submit("submit"),
        Action::WaitForEvent {
            event: EventKind::AdmissionEntered,
        },
        Action::AdvanceMonotonic { millis: CUTOFF_MS },
        Action::ReleaseBarrier {
            point: BarrierPoint::AdmissionEntered,
        },
        Action::WaitForEvent {
            event: EventKind::TaskStoreWorkingReadback,
        },
        Action::SpawnCancel {
            key: KeyCase::Exact,
            lazy_session: true,
            label: "waiting-cancel".to_string(),
        },
        Action::WaitForOperation {
            label: "waiting-cancel".to_string(),
            state: OperationState::Blocked,
        },
        checkpoint_action("cancel-waiting"),
        Action::ReleaseBarrier {
            point: BarrierPoint::AfterWorkingReadback,
        },
        Action::WaitForEvent {
            event: EventKind::ReceiptBegunCommitted,
        },
        Action::WaitForEvent {
            event: EventKind::TokenSignalled,
        },
        Action::WaitForOperation {
            label: "waiting-cancel".to_string(),
            state: OperationState::Completed,
        },
        Action::JoinOperation {
            label: "waiting-cancel".to_string(),
        },
        checkpoint_action("post-begun-cancel"),
    ]));

    let waiting = checkpoint(&report, "cancel-waiting");
    assert_eq!(waiting.token_signals, 0);
    assert!(!only_task(waiting).cancel_requested);
    assert!(!only_receipt(waiting).begun);
    let after = checkpoint(&report, "post-begun-cancel");
    assert!(only_receipt(after).begun);
    assert!(only_task(after).cancel_requested);
    assert_eq!(after.token_signals, 1);
    assert_event_order(
        &report,
        &[
            EventKind::TaskStoreWorkingReadback,
            EventKind::ReceiptBegunCommitted,
            EventKind::TokenSignalled,
        ],
    );
    assert_operation_trace(
        &report,
        "waiting-cancel",
        &[
            OperationEventState::Spawned,
            OperationEventState::Blocked,
            OperationEventState::Completed,
            OperationEventState::Joined,
        ],
    );
    assert_gate_trace(
        &report,
        "waiting-cancel",
        &[
            GateTransition::Waiting,
            GateTransition::Acquired,
            GateTransition::Released,
        ],
    );
}

#[test]
fn thirty_two_lazy_cancel_sessions_finish_within_125ms() {
    let report = execute(Scenario::wall(vec![Action::RunLazyCancelStorm {
        submits: 32,
        cancels: 32,
        per_cancel_deadline_ms: 125,
        label: "cancel-storm".to_string(),
    }]));

    let load = load_run(&report, "cancel-storm");
    assert_eq!(load.lifecycles.len(), 32);
    assert!(load.capacity_rejections.is_empty());
    assert!(load.store_errors.is_empty());
    assert_eq!(
        max_concurrency_sample(load, |sample| sample.owner_slots),
        65
    );
    assert_eq!(max_concurrency_sample(load, |sample| sample.handshakes), 32);
    assert_eq!(
        max_concurrency_sample(load, |sample| sample.accept_batch),
        32
    );
    assert!(load_p99_ms(load) <= 125);
    let mut receipt_keys: Vec<_> = load
        .lifecycles
        .iter()
        .map(|outcome| outcome.key.key_digest.as_str())
        .collect();
    receipt_keys.sort_unstable();
    receipt_keys.dedup();
    assert_eq!(receipt_keys.len(), 32);
    assert!(load_window_ms(load) <= 125);
    assert!(load_total_elapsed_ms(load) <= 125);
    for outcome in &load.lifecycles {
        assert_receipt_key(&outcome.key);
        assert_safe_fingerprint(terminal_digest(&outcome.terminal));
        assert_cancelled_terminal(&outcome.terminal);
        assert!(outcome.response_latency_ms <= 125);
        assert!(outcome.terminal_store_generation > 0);
    }
    assert_eq!(load.task_store_create_attempts, 0);
    assert_eq!(load.listener, ListenerState::Listening);
}

#[test]
fn noncooperative_prepare_forces_fail_stop_after_two_second_grace() {
    let report = execute(Scenario::fake(vec![
        Action::ConfigureProvider {
            execution_class: ExecutionClass::Direct,
            terminal: success_payload(),
            cooperative_cancel: false,
            side_effect_marker: false,
        },
        Action::InstallBarrier {
            point: BarrierPoint::PrepareEntered,
        },
        submit("submit"),
        Action::WaitForEvent {
            event: EventKind::PrepareEntered,
        },
        Action::Cancel {
            key: KeyCase::Exact,
            lazy_session: true,
            label: "cancel".to_string(),
        },
        Action::AdvanceMonotonic { millis: 1_999 },
        checkpoint_action("one-ms-before-fail-stop"),
        Action::AdvanceMonotonic { millis: 1 },
        checkpoint_action("process-exited"),
        Action::Restart,
        Action::Recover {
            key: KeyCase::Exact,
            label: "successor-recover".to_string(),
        },
        checkpoint_action("successor"),
    ]));

    let before = checkpoint(&report, "one-ms-before-fail-stop");
    assert_eq!(before.listener, ListenerState::Listening);
    assert!(before.daemon_running);
    assert!(!before.restart_requested);
    assert_eq!(before.actor_leases, 1);
    let exited = checkpoint(&report, "process-exited");
    assert_eq!(exited.listener, ListenerState::Closed);
    assert!(exited.restart_requested);
    assert!(!exited.daemon_running);
    assert_eq!(exited.process_exit_elapsed_ms, Some(CLEANUP_GRACE_MS));
    assert_eq!(exited.actor_leases, 0);
    let successor = checkpoint(&report, "successor");
    assert_failed_terminal(
        terminal_of_receipt(only_receipt(successor)),
        V5SafeFailureReason::OutcomeUncertain,
    );
    assert_eq!(count_event(&report, EventKind::PrepareEntered), 1);
}

#[test]
fn promised_and_handoff_states_hold_worst_case_result_quota() {
    let report = execute(Scenario::fake(vec![
        direct_provider(),
        Action::InstallBarrier {
            point: BarrierPoint::ValidationEntered,
        },
        submit("reserved-submit"),
        Action::WaitForEvent {
            event: EventKind::ValidationEntered,
        },
        checkpoint_action("quota-reserved"),
        Action::ReleaseBarrier {
            point: BarrierPoint::ValidationEntered,
        },
        Action::WaitForEvent {
            event: EventKind::ReceiptTerminalCommitted,
        },
        Action::Reset,
        direct_provider(),
        Action::InstallBarrier {
            point: BarrierPoint::AdmissionEntered,
        },
        submit("promised-unbound-submit"),
        Action::WaitForEvent {
            event: EventKind::AdmissionEntered,
        },
        Action::AdvanceMonotonic { millis: CUTOFF_MS },
        checkpoint_action("quota-promised-unbound"),
        Action::ReleaseBarrier {
            point: BarrierPoint::AdmissionEntered,
        },
        Action::WaitForEvent {
            event: EventKind::ReceiptTerminalCommitted,
        },
        Action::Reset,
        direct_provider(),
        Action::InstallBarrier {
            point: BarrierPoint::AdmissionEntered,
        },
        Action::InstallBarrier {
            point: BarrierPoint::ActorBound,
        },
        submit("promised-actor-submit"),
        Action::WaitForEvent {
            event: EventKind::AdmissionEntered,
        },
        Action::AdvanceMonotonic { millis: CUTOFF_MS },
        Action::ReleaseBarrier {
            point: BarrierPoint::AdmissionEntered,
        },
        Action::WaitForEvent {
            event: EventKind::ActorBoundCommitted,
        },
        checkpoint_action("quota-promised-actor-bound"),
        Action::ReleaseBarrier {
            point: BarrierPoint::ActorBound,
        },
        Action::WaitForEvent {
            event: EventKind::ReceiptTerminalCommitted,
        },
        Action::Reset,
        direct_provider(),
        Action::InstallBarrier {
            point: BarrierPoint::BeforeReceiptBegun,
        },
        submit("handoff-not-begun-submit"),
        Action::WaitForEvent {
            event: EventKind::ActorBoundCommitted,
        },
        Action::AdvanceMonotonic { millis: CUTOFF_MS },
        Action::WaitForEvent {
            event: EventKind::BoundHandoffCommitted,
        },
        checkpoint_action("quota-handoff-not-begun"),
        Action::ReleaseBarrier {
            point: BarrierPoint::BeforeReceiptBegun,
        },
        Action::WaitForEvent {
            event: EventKind::TaskBoundCommitted,
        },
        Action::Reset,
        direct_provider(),
        Action::InstallBarrier {
            point: BarrierPoint::PrepareEntered,
        },
        submit("handoff-begun-submit"),
        Action::WaitForEvent {
            event: EventKind::PrepareEntered,
        },
        Action::AdvanceMonotonic { millis: CUTOFF_MS },
        Action::WaitForEvent {
            event: EventKind::BoundHandoffCommitted,
        },
        checkpoint_action("quota-handoff-begun"),
        Action::ReleaseBarrier {
            point: BarrierPoint::PrepareEntered,
        },
        Action::WaitForEvent {
            event: EventKind::TaskBoundCommitted,
        },
        Action::Reset,
        known_long_provider(),
        Action::InstallBarrier {
            point: BarrierPoint::BeforeTaskTerminalReceipt,
        },
        submit("bound-submit"),
        Action::WaitForEvent {
            event: EventKind::TaskBoundCommitted,
        },
        checkpoint_action("quota-task-bound"),
        Action::ReleaseBarrier {
            point: BarrierPoint::BeforeTaskTerminalReceipt,
        },
    ]));

    for label in [
        "quota-reserved",
        "quota-promised-unbound",
        "quota-promised-actor-bound",
        "quota-handoff-not-begun",
        "quota-handoff-begun",
    ] {
        let snapshot = checkpoint(&report, label);
        assert_eq!(snapshot.receipt_live_count, 1, "{label}");
        assert_eq!(
            receipt_quota_bytes(snapshot),
            MAX_RESPONSE_LINE_BYTES,
            "{label}"
        );
        assert_max_result_entitlement(only_receipt(snapshot));
    }
    assert_eq!(
        only_receipt(checkpoint(&report, "quota-reserved")).state,
        SeedReceiptState::ReservedUnbound
    );
    assert_eq!(
        only_receipt(checkpoint(&report, "quota-promised-unbound")).state,
        SeedReceiptState::TaskPromisedUnbound
    );
    assert_eq!(
        only_receipt(checkpoint(&report, "quota-promised-actor-bound")).state,
        SeedReceiptState::TaskPromisedActorBound
    );
    assert_eq!(
        only_receipt(checkpoint(&report, "quota-handoff-not-begun")).state,
        SeedReceiptState::TaskHandoffActorBoundNotBegun
    );
    assert_eq!(
        only_receipt(checkpoint(&report, "quota-handoff-begun")).state,
        SeedReceiptState::TaskHandoffActorBoundBegun
    );
    let bound = checkpoint(&report, "quota-task-bound");
    assert_eq!(bound.receipt_actual_bytes, 0);
    assert_eq!(bound.receipt_reserved_bytes, 0);
    assert_eq!(bound.task_link_count, 1);
}

#[test]
fn task_bind_direct_ack_and_receipt_terminal_expiry_release_exact_quota() {
    let task_bind = execute(Scenario::fake(vec![
        known_long_provider(),
        Action::InstallBarrier {
            point: BarrierPoint::BeforeTaskStoreCreate,
        },
        Action::InstallBarrier {
            point: BarrierPoint::BeforeTaskTerminalReceipt,
        },
        Action::SpawnSubmit {
            request: RequestCase::Fresh(1),
            response_budget_ms: CUTOFF_MS,
            disconnect: DisconnectPoint::Never,
            label: "bind-one".to_string(),
        },
        Action::SpawnSubmit {
            request: RequestCase::Fresh(2),
            response_budget_ms: CUTOFF_MS,
            disconnect: DisconnectPoint::Never,
            label: "bind-two".to_string(),
        },
        Action::WaitForEventCount {
            event: EventKind::BoundHandoffCommitted,
            count: 2,
        },
        checkpoint_action("task-bind-before"),
        Action::ReleaseBarrier {
            point: BarrierPoint::BeforeTaskStoreCreate,
        },
        Action::WaitForEventCount {
            event: EventKind::TaskBoundCommitted,
            count: 2,
        },
        checkpoint_action("task-bind-after"),
        Action::ReleaseBarrier {
            point: BarrierPoint::BeforeTaskTerminalReceipt,
        },
        Action::JoinOperation {
            label: "bind-one".to_string(),
        },
        Action::JoinOperation {
            label: "bind-two".to_string(),
        },
        Action::Restart,
        checkpoint_action("task-bind-reopened"),
    ]));
    let bind_before = checkpoint(&task_bind, "task-bind-before");
    let bind_after = checkpoint(&task_bind, "task-bind-after");
    assert_eq!(bind_before.receipt_live_count, 2);
    assert_eq!(
        receipt_quota_bytes(bind_before),
        2 * MAX_RESPONSE_LINE_BYTES
    );
    bind_before
        .receipts
        .iter()
        .for_each(assert_max_result_entitlement);
    assert_eq!(bind_after.receipt_live_count, 0);
    assert_eq!(bind_after.receipt_actual_bytes, 0);
    assert_eq!(bind_after.receipt_reserved_bytes, 0);
    assert_eq!(bind_after.task_link_count, 2);
    assert_eq!(bind_after.task_links.len(), 2);
    let bind_reopened = checkpoint(&task_bind, "task-bind-reopened");
    assert_eq!(bind_reopened.task_link_count, 2);
    assert_eq!(bind_reopened.receipt_actual_bytes, 0);
    assert_eq!(bind_reopened.receipt_reserved_bytes, 0);
    assert_eq!(bind_reopened.task_links, bind_after.task_links);
    assert_eq!(bind_reopened.task_link_bytes, bind_after.task_link_bytes);

    let direct_ack = execute(Scenario::fake(vec![
        direct_provider(),
        Action::Submit {
            request: RequestCase::Fresh(1),
            response_budget_ms: CUTOFF_MS,
            disconnect: DisconnectPoint::Never,
            label: "direct-one".to_string(),
        },
        Action::Submit {
            request: RequestCase::Fresh(2),
            response_budget_ms: CUTOFF_MS,
            disconnect: DisconnectPoint::Never,
            label: "direct-two".to_string(),
        },
        checkpoint_action("direct-ack-before"),
        Action::Acknowledge {
            key: KeyCase::ForSubmitLabel("direct-one".to_string()),
            digest: DigestCase::ExactTerminal,
            disconnect: AckDisconnectPoint::Never,
            label: "ack-one".to_string(),
        },
        checkpoint_action("direct-ack-after-one"),
        Action::Acknowledge {
            key: KeyCase::ForSubmitLabel("direct-two".to_string()),
            digest: DigestCase::ExactTerminal,
            disconnect: AckDisconnectPoint::Never,
            label: "ack-two".to_string(),
        },
        checkpoint_action("direct-ack-after-two"),
        Action::Restart,
        checkpoint_action("direct-ack-reopened"),
    ]));
    let ack_before = checkpoint(&direct_ack, "direct-ack-before");
    let ack_one = checkpoint(&direct_ack, "direct-ack-after-one");
    let ack_two = checkpoint(&direct_ack, "direct-ack-after-two");
    assert_eq!(ack_before.receipt_live_count, 2);
    assert_eq!(ack_one.receipt_live_count, 1);
    assert_eq!(ack_two.receipt_live_count, 0);
    let first_key = response_key(response(&direct_ack, "direct-one"));
    let first_encoded_bytes = ack_before
        .receipts
        .iter()
        .find(|receipt| &receipt.key == first_key)
        .expect("first receipt must exist before acknowledgement")
        .encoded_bytes;
    let first_quota_bytes = ack_before
        .receipts
        .iter()
        .find(|receipt| &receipt.key == first_key)
        .map(|receipt| receipt.encoded_bytes + receipt.reserved_result_bytes)
        .expect("first receipt quota must exist before acknowledgement");
    let surviving_key = response_key(response(&direct_ack, "direct-two"));
    let surviving = ack_one
        .receipts
        .iter()
        .find(|receipt| &receipt.key == surviving_key)
        .expect("acknowledging one receipt must retain the other exact receipt");
    assert!(surviving.terminal.is_some());
    assert_eq!(first_quota_bytes, MAX_RESPONSE_LINE_BYTES);
    assert_max_result_entitlement(surviving);
    assert_eq!(
        ack_before.receipt_actual_bytes - ack_one.receipt_actual_bytes,
        first_encoded_bytes
    );
    assert_eq!(
        ack_one.receipt_actual_bytes - ack_two.receipt_actual_bytes,
        surviving.encoded_bytes
    );
    assert_eq!(ack_two.receipt_actual_bytes, 0);
    assert_eq!(
        receipt_quota_bytes(ack_before) - receipt_quota_bytes(ack_one),
        first_quota_bytes
    );
    assert_eq!(
        receipt_quota_bytes(ack_one) - receipt_quota_bytes(ack_two),
        surviving.encoded_bytes + surviving.reserved_result_bytes
    );
    assert_eq!(receipt_quota_bytes(ack_two), 0);
    assert_eq!(ack_two.tombstone_count, 2);
    assert_eq!(
        checkpoint(&direct_ack, "direct-ack-reopened").tombstone_count,
        2
    );
    assert_eq!(
        checkpoint(&direct_ack, "direct-ack-reopened").receipt_actual_bytes,
        0
    );
    assert_eq!(
        checkpoint(&direct_ack, "direct-ack-reopened").receipt_reserved_bytes,
        0
    );

    let receipt_expiry = execute(Scenario::fake(vec![
        Action::ConfigureValidation { reject: true },
        Action::InstallBarrier {
            point: BarrierPoint::ValidationEntered,
        },
        Action::SpawnSubmit {
            request: RequestCase::Fresh(1),
            response_budget_ms: CUTOFF_MS,
            disconnect: DisconnectPoint::Never,
            label: "expiry-one".to_string(),
        },
        Action::SpawnSubmit {
            request: RequestCase::Fresh(2),
            response_budget_ms: CUTOFF_MS,
            disconnect: DisconnectPoint::Never,
            label: "expiry-two".to_string(),
        },
        Action::WaitForEventCount {
            event: EventKind::ValidationEntered,
            count: 2,
        },
        Action::AdvanceMonotonic { millis: CUTOFF_MS },
        Action::ReleaseBarrier {
            point: BarrierPoint::ValidationEntered,
        },
        Action::WaitForEventCount {
            event: EventKind::ReceiptTerminalCommitted,
            count: 2,
        },
        Action::JoinOperation {
            label: "expiry-one".to_string(),
        },
        Action::JoinOperation {
            label: "expiry-two".to_string(),
        },
        checkpoint_action("receipt-terminal-expiry-before"),
        Action::AdvanceEpoch {
            millis: DIRECT_TASK_TTL_MS,
        },
        checkpoint_action("receipt-terminal-expiry-after"),
        Action::Restart,
        checkpoint_action("receipt-terminal-expiry-reopened"),
    ]));
    let expiry_before = checkpoint(&receipt_expiry, "receipt-terminal-expiry-before");
    let expiry_after = checkpoint(&receipt_expiry, "receipt-terminal-expiry-after");
    assert_eq!(expiry_before.receipt_live_count, 2);
    assert_eq!(
        receipt_quota_bytes(expiry_before),
        2 * MAX_RESPONSE_LINE_BYTES
    );
    expiry_before
        .receipts
        .iter()
        .for_each(assert_max_result_entitlement);
    assert_eq!(expiry_after.receipt_live_count, 0);
    assert_eq!(expiry_after.receipt_actual_bytes, 0);
    assert_eq!(expiry_after.receipt_reserved_bytes, 0);
    assert_eq!(
        checkpoint(&receipt_expiry, "receipt-terminal-expiry-reopened").receipt_live_count,
        0
    );

    let retirement_cases = [
        TaskRetirementWorkload::RecoveryTerminalBeforeTerminalBound,
        TaskRetirementWorkload::ActiveTaskBoundAbsent,
        TaskRetirementWorkload::BeforePendingIntent,
        TaskRetirementWorkload::AfterPendingIntentBeforeDelete,
        TaskRetirementWorkload::AfterDeleteCommitUncertain,
        TaskRetirementWorkload::AfterDeletedBeforeLedgerFinalize,
        TaskRetirementWorkload::AfterAbsentConfirmedBeforeLedgerFinalize,
        TaskRetirementWorkload::DeleteIdentityMismatch,
    ];
    let retirement = execute(Scenario::fake(vec![Action::RunTaskRetirementWorkload {
        cases: retirement_cases.to_vec(),
    }]));
    assert_eq!(
        retirement.task_retirement_cases.len(),
        retirement_cases.len()
    );
    for expected_case in retirement_cases {
        let matches: Vec<_> = retirement
            .task_retirement_cases
            .iter()
            .filter(|case| case.case == expected_case)
            .collect();
        assert_eq!(
            matches.len(),
            1,
            "missing retirement case {expected_case:?}"
        );
        let case = matches[0];
        if expected_case == TaskRetirementWorkload::ActiveTaskBoundAbsent {
            assert!(case.before.tasks.is_empty());
            assert!(case.before.receipts.is_empty());
            assert_eq!(case.before.task_links.len(), 1);
            assert!(matches!(
                &only_task_link(&case.before).lifecycle,
                TaskLinkLifecycleObservation::TaskBoundBegun { .. }
            ));
            assert_eq!(case.before.invocation_index.len(), 1);
            assert_eq!(case.before.reserved_task_index.len(), 1);
            assert!(retirement_pendings(&case.before).is_empty());
            assert_eq!(
                case.before.receipt_store_mutations,
                case.after_recovery.receipt_store_mutations
            );
            assert_eq!(
                case.before.task_store_mutations,
                case.after_recovery.task_store_mutations
            );
            assert_eq!(case.after_recovery.listener, ListenerState::Closed);
            assert!(case.after_recovery.restart_requested);
            assert!(!case.after_recovery.daemon_running);
            assert!(matches!(
                &case.delete_outcome,
                TaskRetirementDeleteOutcome::NotAttemptedActiveTaskMissing { .. }
            ));
            continue;
        }
        let before_task = only_task(&case.before);
        let terminal = before_task
            .terminal
            .as_ref()
            .expect("only terminal Tasks are eligible for retirement");
        assert!(matches!(
            before_task.status,
            TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Cancelled
        ));
        assert_eq!(
            before_task.expires_epoch_ms,
            terminal_epoch_ms(terminal) + before_task.ttl_ms
        );
        assert!(case.before.epoch_ms >= before_task.expires_epoch_ms);
        assert_eq!(case.before.task_links.len(), 1);
        match expected_case {
            TaskRetirementWorkload::BeforePendingIntent
            | TaskRetirementWorkload::AfterPendingIntentBeforeDelete
            | TaskRetirementWorkload::RecoveryTerminalBeforeTerminalBound => assert!(matches!(
                &case.delete_outcome,
                TaskRetirementDeleteOutcome::Deleted { .. }
            )),
            TaskRetirementWorkload::AfterDeletedBeforeLedgerFinalize
            | TaskRetirementWorkload::AfterAbsentConfirmedBeforeLedgerFinalize => {
                assert!(matches!(
                    &case.delete_outcome,
                    TaskRetirementDeleteOutcome::AbsentExactWithPending { .. }
                ))
            }
            TaskRetirementWorkload::AfterDeleteCommitUncertain => assert!(matches!(
                &case.delete_outcome,
                TaskRetirementDeleteOutcome::CommitUncertain { .. }
            )),
            TaskRetirementWorkload::DeleteIdentityMismatch => assert!(matches!(
                &case.delete_outcome,
                TaskRetirementDeleteOutcome::IdentityMismatch { .. }
            )),
            TaskRetirementWorkload::ActiveTaskBoundAbsent => unreachable!(),
        }
    }
}

#[test]
fn direct_unacked_expiry_deletes_payload_and_releases_exact_quota() {
    let report = execute(Scenario::fake(vec![
        direct_provider(),
        submit("submit"),
        checkpoint_action("terminal"),
        Action::AdvanceEpoch {
            millis: DIRECT_TASK_TTL_MS - 1,
        },
        checkpoint_action("before-expiry"),
        Action::AdvanceEpoch { millis: 1 },
        checkpoint_action("expired"),
        Action::Restart,
        Action::Recover {
            key: KeyCase::Exact,
            label: "recover-after-expiry".to_string(),
        },
        checkpoint_action("reopened"),
    ]));

    let terminal = checkpoint(&report, "terminal");
    assert_eq!(terminal.receipt_live_count, 1);
    let terminal_receipt = only_receipt(terminal);
    assert!(terminal_receipt.terminal.is_some());
    assert_eq!(
        terminal_receipt.expires_epoch_ms,
        Some(terminal_epoch_ms(terminal_of_receipt(terminal_receipt)) + DIRECT_TASK_TTL_MS)
    );
    assert_max_result_entitlement(terminal_receipt);
    assert!(terminal.receipt_actual_bytes > 0);
    let before_expiry = checkpoint(&report, "before-expiry");
    assert_eq!(before_expiry.receipt_live_count, 1);
    assert_eq!(
        before_expiry.receipt_actual_bytes,
        terminal.receipt_actual_bytes
    );
    let expired = checkpoint(&report, "expired");
    assert_eq!(expired.receipt_live_count, 0);
    assert_eq!(expired.receipt_actual_bytes, 0);
    assert_eq!(expired.receipt_reserved_bytes, 0);
    assert_eq!(expired.tombstone_count, 0);
    assert!(expired.receipts.is_empty());
    assert_eq!(checkpoint(&report, "reopened").receipt_live_count, 0);
    assert_eq!(
        response(&report, "recover-after-expiry").kind,
        ResponseKind::NotFound
    );
    assert!(matches!(
        response(&report, "recover-after-expiry").error,
        Some(ErrorCode::ReceiptExpired | ErrorCode::ReceiptNotFound)
    ));
}

#[test]
fn link_capacity_before_begun_terminalizes_receipt_backed_without_callback() {
    let report = execute(Scenario::fake(vec![
        Action::FillTaskLinks,
        Action::PublishListener,
        checkpoint_action("promised-baseline"),
        Action::SeedReceipt {
            state: SeedReceiptState::TaskPromisedActorBound,
            cancel_requested: false,
            staged_terminal: None,
        },
        Action::AttemptTaskStoreBindUnderGate {
            label: "promised-capacity".to_string(),
        },
        Action::JoinOperation {
            label: "promised-capacity".to_string(),
        },
        checkpoint_action("promised"),
        Action::Reset,
        Action::FillTaskLinks,
        Action::PublishListener,
        checkpoint_action("handoff-baseline"),
        Action::SeedReceipt {
            state: SeedReceiptState::TaskHandoffActorBoundNotBegun,
            cancel_requested: false,
            staged_terminal: None,
        },
        Action::AttemptTaskStoreBindUnderGate {
            label: "handoff-capacity".to_string(),
        },
        Action::JoinOperation {
            label: "handoff-capacity".to_string(),
        },
        checkpoint_action("handoff"),
    ]));

    for (label, baseline) in [
        ("promised", "promised-baseline"),
        ("handoff", "handoff-baseline"),
    ] {
        let snapshot = checkpoint(&report, label);
        assert_eq!(
            only_receipt(snapshot).state,
            SeedReceiptState::TaskTerminalReceiptBacked
        );
        assert_failed_terminal(
            terminal_of_receipt(only_receipt(snapshot)),
            V5SafeFailureReason::TaskCapacity,
        );
        assert_eq!(snapshot.tasks.len() as u64, TASK_LINK_LIMIT);
        assert_eq!(
            checkpoint(&report, baseline).task_link_count,
            TASK_LINK_LIMIT
        );
        assert_eq!(
            checkpoint(&report, baseline).tasks.len() as u64,
            TASK_LINK_LIMIT
        );
        assert_eq!(snapshot.tasks, checkpoint(&report, baseline).tasks);
        assert_eq!(
            snapshot.task_links,
            checkpoint(&report, baseline).task_links
        );
        assert_eq!(snapshot.callbacks.prepare, 0);
        assert_eq!(snapshot.callbacks.execute, 0);
        assert_eq!(snapshot.listener, ListenerState::Listening);
        assert_eq!(snapshot.task_store_create_attempts, 0);
        assert_eq!(snapshot.task_link_reserved_count, 0);
        assert_eq!(
            response(&report, &format!("{label}-capacity")).error,
            Some(ErrorCode::TaskCapacity)
        );
        let capacity = observed_task_publication_capacity(&report, &format!("{label}-capacity"));
        assert_eq!(capacity.receipt_key, only_receipt(snapshot).key);
        assert_eq!(
            capacity.terminal.as_ref(),
            only_receipt(snapshot).terminal.as_ref()
        );
        assert!(capacity.staged_transfer_certificate_sha256.is_none());
        assert_publication_matches_snapshot(
            &report,
            snapshot,
            &only_receipt(snapshot).key,
            terminal_of_receipt(only_receipt(snapshot)),
            TerminalPublicationOwner::ReceiptBackedTask,
        );
    }
    assert_eq!(count_event(&report, EventKind::ListenerPublished), 2);
    assert_eq!(count_event(&report, EventKind::TaskLinkCapacityRejected), 2);
    assert_eq!(count_event(&report, EventKind::TaskLinkCapacityReserved), 0);
    assert_eq!(count_event(&report, EventKind::TaskStoreCreateAttempted), 0);
    assert_eq!(
        count_event(&report, EventKind::TaskLinkReservationReleased),
        0
    );
}

#[test]
fn task_store_capacity_after_reservation_is_invariant_violation_and_fail_stops() {
    let report = execute(Scenario::fake(vec![
        Action::FillTaskLinksLeavingOneReservationSlot,
        Action::PublishListener,
        checkpoint_action("baseline"),
        Action::SeedReceipt {
            state: SeedReceiptState::TaskHandoffActorBoundBegun,
            cancel_requested: false,
            staged_terminal: None,
        },
        Action::SpawnStageBoundHandoffTerminal {
            terminal: success_payload(),
            label: "stage-before-invariant".to_string(),
        },
        Action::WaitForOperation {
            label: "stage-before-invariant".to_string(),
            state: OperationState::Completed,
        },
        Action::JoinOperation {
            label: "stage-before-invariant".to_string(),
        },
        checkpoint_action("staged"),
        Action::InjectTaskStoreCapacityInvariantViolationOnce,
        Action::AttemptTaskStoreBindUnderGate {
            label: "post-reserve-capacity-invariant".to_string(),
        },
        Action::JoinOperation {
            label: "post-reserve-capacity-invariant".to_string(),
        },
        checkpoint_action("fail-stopped"),
        Action::Restart,
        Action::ReconcileStartup,
        Action::PublishListener,
        Action::Recover {
            key: KeyCase::Exact,
            label: "recovered-after-invariant".to_string(),
        },
        checkpoint_action("reopened"),
    ]));

    let baseline = checkpoint(&report, "baseline");
    assert_eq!(baseline.tasks.len() as u64, TASK_LINK_LIMIT - 1);
    assert_eq!(baseline.task_link_count, TASK_LINK_LIMIT - 1);
    assert_eq!(baseline.task_link_reserved_count, 0);
    let staged = checkpoint(&report, "staged");
    let staged_receipt = only_receipt(staged);
    let staged_winner = staged_receipt
        .staged_terminal
        .as_ref()
        .expect("the invariant fault must happen only after the certified terminal is staged");
    assert!(completed_result(staged_winner).ok);
    let preparation = report
        .staged_terminal_preparations
        .iter()
        .find(|preparation| preparation.receipt_key == staged_receipt.key)
        .expect("staged winner must retain its exact transfer-size certificate");
    assert_eq!(preparation.terminal, *staged_winner);

    let stopped = checkpoint(&report, "fail-stopped");
    let retained = only_receipt(stopped);
    assert_eq!(retained.key, staged_receipt.key);
    assert_eq!(retained.state, SeedReceiptState::TaskHandoffActorBoundBegun);
    assert_eq!(retained.staged_terminal.as_ref(), Some(staged_winner));
    assert_eq!(stopped.tasks, baseline.tasks);
    assert_eq!(stopped.task_links, baseline.task_links);
    assert_eq!(stopped.task_link_reserved_count, 1);
    assert_eq!(stopped.listener, ListenerState::Closed);
    assert!(stopped.restart_requested);
    assert!(!stopped.daemon_running);
    assert_eq!(stopped.callbacks, staged.callbacks);
    assert!(!report
        .responses
        .contains_key("post-reserve-capacity-invariant"));
    assert_eq!(
        count_event(&report, EventKind::TaskStoreCapacityInvariantViolation),
        1
    );
    assert_eq!(
        count_event(&report, EventKind::TaskLinkReservationReleased),
        0
    );
    assert_eq!(
        count_event(&report, EventKind::TaskLinkReservationConverted),
        1
    );
    let violation = observed_task_store_capacity_invariant_violation(
        &report,
        "post-reserve-capacity-invariant",
    );
    assert_eq!(violation.receipt_key, retained.key);
    assert_eq!(violation.staged_terminal, *staged_winner);
    assert_eq!(
        violation.staged_transfer_certificate_sha256,
        preparation.transfer_size_certificate.certificate.sha256
    );
    assert_eq!(
        violation.task_store_record_count_before,
        TASK_LINK_LIMIT - 1
    );
    assert_eq!(
        violation.materialized_lifecycle_link_count_before,
        TASK_LINK_LIMIT - 1
    );
    assert_eq!(violation.live_link_reservation_count_before, 1);

    let reopened = checkpoint(&report, "reopened");
    assert_eq!(reopened.receipt_live_count, 0);
    assert!(reopened
        .receipts
        .iter()
        .all(|receipt| receipt.key != retained.key));
    assert_eq!(reopened.tasks.len() as u64, TASK_LINK_LIMIT);
    assert_eq!(reopened.task_link_count, TASK_LINK_LIMIT);
    assert_eq!(reopened.task_link_reserved_count, 0);
    assert_eq!(reopened.listener, ListenerState::Listening);
    assert!(!reopened.restart_requested);
    assert!(reopened.daemon_running);
    assert_eq!(reopened.callbacks, stopped.callbacks);
    let recovered_task = reopened
        .tasks
        .iter()
        .find(|task| task.receipt_key == retained.key)
        .expect("startup must reconcile the retained reservation into the exact Task");
    assert_eq!(recovered_task.terminal.as_ref(), Some(staged_winner));
    let recovered_link = reopened
        .task_links
        .iter()
        .find(|link| link.key == retained.key)
        .expect("startup must atomically materialize the retained reservation as the sole link");
    assert_terminal_bound_link(recovered_link, recovered_task, staged_winner);
    assert_eq!(
        response_terminal(response(&report, "recovered-after-invariant")),
        staged_winner
    );
    assert_publication_matches_snapshot(
        &report,
        reopened,
        &retained.key,
        staged_winner,
        TerminalPublicationOwner::StagedHandoffTask,
    );
}

#[test]
fn link_capacity_preserves_staged_terminal_winner() {
    let report = execute(Scenario::fake(vec![
        Action::FillTaskLinks,
        checkpoint_action("link-capacity-baseline"),
        Action::SeedReceipt {
            state: SeedReceiptState::TaskHandoffActorBoundBegun,
            cancel_requested: false,
            staged_terminal: None,
        },
        Action::SpawnStageBoundHandoffTerminal {
            terminal: success_payload(),
            label: "link-capacity-stage".to_string(),
        },
        Action::WaitForOperation {
            label: "link-capacity-stage".to_string(),
            state: OperationState::Completed,
        },
        Action::JoinOperation {
            label: "link-capacity-stage".to_string(),
        },
        checkpoint_action("link-capacity-staged"),
        Action::AttemptTaskStoreBindUnderGate {
            label: "staged-link-capacity".to_string(),
        },
        Action::JoinOperation {
            label: "staged-link-capacity".to_string(),
        },
        checkpoint_action("link-capacity-terminal"),
        Action::Restart,
        Action::Recover {
            key: KeyCase::Exact,
            label: "link-capacity-recovered".to_string(),
        },
        checkpoint_action("link-capacity-reopened"),
    ]));

    let link_baseline = checkpoint(&report, "link-capacity-baseline");
    assert_eq!(link_baseline.task_link_count, TASK_LINK_LIMIT);
    assert_eq!(link_baseline.tasks.len() as u64, TASK_LINK_LIMIT);
    let link_staged = checkpoint(&report, "link-capacity-staged");
    let link_staged_receipt = only_receipt(link_staged);
    let link_winner = link_staged_receipt
        .staged_terminal
        .as_ref()
        .expect("staged winner must commit before link admission is attempted");
    assert!(completed_result(link_winner).ok);
    let link_terminal = checkpoint(&report, "link-capacity-terminal");
    let link_terminal_receipt = only_receipt(link_terminal);
    assert_eq!(
        link_terminal_receipt.state,
        SeedReceiptState::TaskTerminalReceiptBacked
    );
    assert_eq!(terminal_of_receipt(link_terminal_receipt), link_winner);
    assert!(link_terminal_receipt.staged_terminal.is_none());
    assert_eq!(link_terminal.task_links, link_baseline.task_links);
    assert_eq!(link_terminal.tasks, link_baseline.tasks);
    assert_eq!(link_terminal.task_link_reserved_count, 0);
    assert_eq!(
        link_terminal.task_store_create_attempts, link_baseline.task_store_create_attempts,
        "proven LinkCapacity must precede every TaskStore create"
    );
    assert_eq!(
        response_terminal(response(&report, "staged-link-capacity")),
        link_winner
    );
    assert!(response(&report, "staged-link-capacity").error.is_none());
    let link_capacity = observed_task_publication_capacity(&report, "staged-link-capacity");
    assert_eq!(link_capacity.receipt_key, link_terminal_receipt.key);
    assert_eq!(link_capacity.terminal.as_ref(), Some(link_winner));
    assert!(matches!(
        link_capacity.evidence,
        TaskPublicationCapacityEvidence::LinkCapacity { .. }
    ));
    let link_preparation = report
        .staged_terminal_preparations
        .iter()
        .find(|preparation| preparation.receipt_key == link_terminal_receipt.key)
        .expect("LinkCapacity fallback must consume the exact staged certificate");
    assert_eq!(link_preparation.terminal, *link_winner);
    assert_eq!(
        link_capacity.staged_transfer_certificate_sha256.as_deref(),
        Some(
            link_preparation
                .transfer_size_certificate
                .certificate
                .sha256
                .as_str()
        )
    );
    let link_capacity_certificate = link_preparation
        .transfer_size_certificate
        .capacity_fallback_cases
        .first()
        .expect("certificate must cover the sole LinkCapacity receipt-backed winner");
    assert!(matches!(
        link_capacity_certificate,
        StagedCapacityFallbackSizeCaseObservation::LinkCapacity { .. }
    ));
    let (link_record_bound, link_frame_bound) = link_capacity_certificate.artifacts();
    let link_publication = terminal_publication_for(
        &report,
        &link_terminal_receipt.key,
        terminal_of_receipt(link_terminal_receipt),
    );
    let link_receipt_piece = link_publication
        .commit
        .receipt()
        .expect("LinkCapacity staged winner stays receipt-owned");
    assert!(link_receipt_piece.receipt_record.encoded_bytes <= link_record_bound.encoded_bytes);
    assert!(link_publication
        .response_frames
        .iter()
        .all(|frame| frame.response_jsonl.encoded_bytes <= link_frame_bound.encoded_bytes));
    assert_publication_matches_snapshot(
        &report,
        link_terminal,
        &link_terminal_receipt.key,
        terminal_of_receipt(link_terminal_receipt),
        TerminalPublicationOwner::ReceiptBackedTask,
    );
    let link_reopened = checkpoint(&report, "link-capacity-reopened");
    assert_eq!(link_reopened.receipts, link_terminal.receipts);
    assert_eq!(link_reopened.tasks, link_terminal.tasks);
    assert_eq!(link_reopened.task_links, link_terminal.task_links);
    assert_eq!(
        link_reopened.receipt_actual_bytes,
        link_terminal.receipt_actual_bytes
    );
    assert_eq!(
        link_reopened.receipt_reserved_bytes,
        link_terminal.receipt_reserved_bytes
    );
    assert_eq!(
        response_terminal(response(&report, "link-capacity-recovered")),
        terminal_of_receipt(link_terminal_receipt)
    );
}

#[test]
fn receipt_owned_begun_crash_terminalizes_outcome_uncertain_without_task_store() {
    let report = execute(Scenario::fake(vec![
        Action::SeedReceipt {
            state: SeedReceiptState::TaskReceiptOwnedActorBound,
            cancel_requested: false,
            staged_terminal: None,
        },
        Action::Restart,
        Action::ReadTask {
            api: TaskApi::NativeGet,
            label: "recovered".to_string(),
        },
        checkpoint_action("recovered"),
    ]));

    let recovered = checkpoint(&report, "recovered");
    assert!(recovered.tasks.is_empty());
    assert_eq!(
        only_receipt(recovered).state,
        SeedReceiptState::TaskTerminalReceiptBacked
    );
    assert_failed_terminal(
        terminal_of_receipt(only_receipt(recovered)),
        V5SafeFailureReason::OutcomeUncertain,
    );
    assert_failed_terminal(
        task_read(&report, "recovered")
            .terminal
            .as_ref()
            .expect("receipt-backed recovered Task terminal"),
        V5SafeFailureReason::OutcomeUncertain,
    );
    assert_eq!(
        task_read(&report, "recovered").terminal.as_ref(),
        only_receipt(recovered).terminal.as_ref()
    );
    assert_eq!(recovered.callbacks.total_domain(), 0);
    assert_eq!(recovered.task_store_create_attempts, 0);
}

#[test]
fn task_store_4097_boundary_preserves_existing_tasks_and_listener_availability() {
    let pre_report = execute(Scenario::fake(vec![
        Action::FillTaskLinks,
        Action::PublishListener,
        checkpoint_action("pre-baseline"),
        Action::SeedReceipt {
            state: SeedReceiptState::TaskPromisedActorBound,
            cancel_requested: false,
            staged_terminal: None,
        },
        Action::AttemptTaskStoreBindUnderGate {
            label: "pre-capacity-4097".to_string(),
        },
        Action::JoinOperation {
            label: "pre-capacity-4097".to_string(),
        },
        checkpoint_action("pre-begun-4097"),
    ]));
    let begun_report = execute(Scenario::fake(vec![
        Action::FillTaskLinks,
        Action::PublishListener,
        checkpoint_action("begun-baseline"),
        Action::SeedReceipt {
            state: SeedReceiptState::TaskHandoffActorBoundBegun,
            cancel_requested: false,
            staged_terminal: None,
        },
        Action::AttemptTaskStoreBindUnderGate {
            label: "begun-capacity-4097".to_string(),
        },
        Action::JoinOperation {
            label: "begun-capacity-4097".to_string(),
        },
        checkpoint_action("begun-4097-latched"),
        Action::ContinueReceiptOwnedAttempt {
            terminal: success_payload(),
            label: "begun-receipt-owned-continuation".to_string(),
        },
        checkpoint_action("begun-4097-terminal"),
    ]));

    let pre_baseline = checkpoint(&pre_report, "pre-baseline");
    let pre = checkpoint(&pre_report, "pre-begun-4097");
    assert_eq!(pre.tasks.len() as u64, TASK_LINK_LIMIT);
    assert_eq!(pre.task_link_count, TASK_LINK_LIMIT);
    assert_eq!(pre_baseline.task_link_count, TASK_LINK_LIMIT);
    assert_eq!(pre.task_link_bytes, pre_baseline.task_link_bytes);
    assert!(pre.task_link_bytes <= TASK_LINK_BYTES_LIMIT);
    assert_eq!(pre.task_links.len() as u64, TASK_LINK_LIMIT);
    assert_eq!(pre.task_links, pre_baseline.task_links);
    assert_eq!(pre.tasks, pre_baseline.tasks);
    for link in &pre.task_links {
        let task = pre
            .tasks
            .iter()
            .find(|task| task.receipt_key == link.key)
            .expect("each full-pool lifecycle link must retain its exact TaskStore record");
        assert_eq!(task.task_id, link.task_id);
        assert_eq!(task.invocation_id, link.invocation_id);
        assert_eq!(
            task.workspace_identity_hash.as_deref(),
            Some(link.workspace_identity_hash.as_str())
        );
    }
    assert_failed_terminal(
        terminal_of_receipt(only_receipt(pre)),
        V5SafeFailureReason::TaskCapacity,
    );
    assert_eq!(
        response(&pre_report, "pre-capacity-4097").error,
        Some(ErrorCode::TaskCapacity)
    );
    assert_eq!(pre.callbacks.total_domain(), 0);
    assert_eq!(pre.listener, ListenerState::Listening);
    assert_eq!(pre.task_store_create_attempts, 0);
    assert_eq!(
        count_event(&pre_report, EventKind::TaskLinkCapacityRejected),
        1
    );
    assert_eq!(
        count_event(&pre_report, EventKind::TaskStoreCreateAttempted),
        0
    );
    let pre_capacity = observed_task_publication_capacity(&pre_report, "pre-capacity-4097");
    assert!(matches!(
        pre_capacity.evidence,
        TaskPublicationCapacityEvidence::LinkCapacity { .. }
    ));
    assert_eq!(
        pre_capacity.terminal.as_ref(),
        only_receipt(pre).terminal.as_ref()
    );
    assert!(pre_capacity.staged_transfer_certificate_sha256.is_none());

    let begun_baseline = checkpoint(&begun_report, "begun-baseline");
    let begun = checkpoint(&begun_report, "begun-4097-latched");
    assert_eq!(
        only_receipt(begun).state,
        SeedReceiptState::TaskReceiptOwnedActorBound
    );
    assert_eq!(begun.task_store_create_attempts, 0);
    assert_eq!(begun.task_link_count, TASK_LINK_LIMIT);
    assert_eq!(begun_baseline.task_link_count, TASK_LINK_LIMIT);
    assert_eq!(begun.task_link_bytes, begun_baseline.task_link_bytes);
    assert!(begun.task_link_bytes <= TASK_LINK_BYTES_LIMIT);
    assert_eq!(begun.task_links.len() as u64, TASK_LINK_LIMIT);
    assert_eq!(begun.task_links, begun_baseline.task_links);
    assert_eq!(begun.tasks, begun_baseline.tasks);
    assert_eq!(
        count_event(&begun_report, EventKind::TaskLinkCapacityRejected),
        1
    );
    assert_eq!(
        count_event(&begun_report, EventKind::TaskStoreCreateAttempted),
        0
    );
    assert_eq!(begun.listener, ListenerState::Listening);
    assert_eq!(
        response(&begun_report, "begun-capacity-4097").error,
        Some(ErrorCode::TaskCapacity)
    );
    let begun_capacity = observed_task_publication_capacity(&begun_report, "begun-capacity-4097");
    assert!(matches!(
        begun_capacity.evidence,
        TaskPublicationCapacityEvidence::LinkCapacity { .. }
    ));
    assert!(begun_capacity.terminal.is_none());
    assert!(begun_capacity.staged_transfer_certificate_sha256.is_none());
    let begun_terminal = checkpoint(&begun_report, "begun-4097-terminal");
    let begun_terminal_receipt = only_receipt(begun_terminal);
    assert_eq!(
        begun_terminal_receipt.state,
        SeedReceiptState::TaskTerminalReceiptBacked
    );
    assert!(completed_result(terminal_of_receipt(begun_terminal_receipt)).ok);
    assert_eq!(
        begun_terminal.task_store_create_attempts, begun_baseline.task_store_create_attempts,
        "receipt-owned continuation must never retry TaskStore create"
    );
    assert_eq!(begun_terminal.task_links, begun_baseline.task_links);
    assert_eq!(begun_terminal.tasks, begun_baseline.tasks);
    assert_publication_matches_snapshot(
        &begun_report,
        begun_terminal,
        &begun_terminal_receipt.key,
        terminal_of_receipt(begun_terminal_receipt),
        TerminalPublicationOwner::ReceiptBackedTask,
    );
    assert_eq!(count_event(&pre_report, EventKind::ListenerPublished), 1);
    assert_eq!(count_event(&begun_report, EventKind::ListenerPublished), 1);
}

#[test]
fn receipt_pools_and_task_store_limits_are_independent_after_restart() {
    let report = execute(Scenario::fake(vec![
        Action::FillReceiptPool {
            state: SeedReceiptState::CancelReserved,
            count: 2,
        },
        Action::FillReceiptPool {
            state: SeedReceiptState::TaskTerminalReceiptBacked,
            count: 62,
        },
        Action::FillTaskLinks,
        Action::FillTombstones,
        checkpoint_action("before-restart"),
        Action::Restart,
        checkpoint_action("after-restart"),
        submit("convert-cancel-reservation"),
        checkpoint_action("after-conversion"),
    ]));

    for label in ["before-restart", "after-restart"] {
        let snapshot = checkpoint(&report, label);
        assert_eq!(snapshot.receipt_live_count, LIVE_RECEIPT_LIMIT);
        let cancels: Vec<_> = snapshot
            .receipts
            .iter()
            .filter(|receipt| receipt.state == SeedReceiptState::CancelReserved)
            .collect();
        assert_eq!(cancels.len(), 2);
        assert!(cancels.iter().all(|receipt| receipt.encoded_bytes > 0));
        assert!(cancels
            .iter()
            .all(|receipt| receipt.reserved_result_bytes == 0));
        let terminals: Vec<_> = snapshot
            .receipts
            .iter()
            .filter(|receipt| receipt.state == SeedReceiptState::TaskTerminalReceiptBacked)
            .collect();
        assert_eq!(terminals.len(), 62);
        terminals
            .iter()
            .for_each(|receipt| assert_max_result_entitlement(receipt));
        let cancel_bytes: u64 = cancels.iter().map(|receipt| receipt.encoded_bytes).sum();
        assert_eq!(
            receipt_quota_bytes(snapshot),
            62 * MAX_RESPONSE_LINE_BYTES + cancel_bytes
        );
        assert!(receipt_quota_bytes(snapshot) <= LIVE_RECEIPT_BYTES_LIMIT);
        assert_exact_linked_task_pool(snapshot, TASK_LINK_LIMIT);
        assert!(snapshot.task_link_bytes <= TASK_LINK_BYTES_LIMIT);
        assert_eq!(snapshot.tombstone_count, TOMBSTONE_LIMIT);
        assert!(snapshot.tombstone_bytes <= TOMBSTONE_BYTES_LIMIT);
    }
    let before_restart = checkpoint(&report, "before-restart");
    let after_restart = checkpoint(&report, "after-restart");
    assert_eq!(before_restart.receipts.len() as u64, LIVE_RECEIPT_LIMIT);
    assert_eq!(before_restart.tombstones.len() as u64, TOMBSTONE_LIMIT);
    assert_eq!(before_restart.receipts, after_restart.receipts);
    assert_eq!(before_restart.tombstones, after_restart.tombstones);
    assert_eq!(before_restart.tasks, after_restart.tasks);
    assert_eq!(before_restart.task_links, after_restart.task_links);
    assert_eq!(
        before_restart.invocation_index,
        after_restart.invocation_index
    );
    assert_eq!(
        before_restart.reserved_task_index,
        after_restart.reserved_task_index
    );
    assert_eq!(
        before_restart.receipt_live_count,
        after_restart.receipt_live_count
    );
    assert_eq!(
        before_restart.receipt_actual_bytes,
        after_restart.receipt_actual_bytes
    );
    assert_eq!(
        before_restart.receipt_reserved_bytes,
        after_restart.receipt_reserved_bytes
    );
    assert_eq!(
        before_restart.task_link_count,
        after_restart.task_link_count
    );
    assert_eq!(
        before_restart.task_link_bytes,
        after_restart.task_link_bytes
    );
    assert_eq!(
        before_restart.tombstone_count,
        after_restart.tombstone_count
    );
    assert_eq!(
        before_restart.tombstone_bytes,
        after_restart.tombstone_bytes
    );
    assert_eq!(
        before_restart.store_generation,
        after_restart.store_generation
    );
    let converted = checkpoint(&report, "after-conversion");
    assert!(receipt_quota_bytes(converted) <= LIVE_RECEIPT_BYTES_LIMIT);
    assert_eq!(converted.receipt_live_count, LIVE_RECEIPT_LIMIT);
    assert_eq!(converted.task_link_count, TASK_LINK_LIMIT);
    assert_eq!(converted.tombstone_count, TOMBSTONE_LIMIT);
    assert_eq!(converted.task_links, after_restart.task_links);
    assert_eq!(converted.tombstones, after_restart.tombstones);
    assert_eq!(converted.tasks, after_restart.tasks);
    assert_exact_linked_task_pool(converted, TASK_LINK_LIMIT);
    assert_eq!(converted.callbacks.total_domain(), 0);
    let remaining_cancels: Vec<_> = converted
        .receipts
        .iter()
        .filter(|receipt| receipt.state == SeedReceiptState::CancelReserved)
        .collect();
    assert_eq!(remaining_cancels.len(), 1);
    assert_eq!(remaining_cancels[0].reserved_result_bytes, 0);
    let entitled_live: Vec<_> = converted
        .receipts
        .iter()
        .filter(|receipt| receipt.state != SeedReceiptState::CancelReserved)
        .collect();
    assert_eq!(entitled_live.len(), 63);
    entitled_live
        .iter()
        .for_each(|receipt| assert_max_result_entitlement(receipt));
    assert_eq!(
        receipt_quota_bytes(converted),
        63 * MAX_RESPONSE_LINE_BYTES + remaining_cancels[0].encoded_bytes
    );
    let conversion = response(&report, "convert-cancel-reservation");
    assert_eq!(conversion.kind, ResponseKind::Cancelled);
    let converted_key = response_key(conversion);
    let converted_receipt = converted
        .receipts
        .iter()
        .find(|receipt| &receipt.key == converted_key)
        .expect("cancel conversion must remain durably observable");
    assert_eq!(
        converted_receipt.state,
        SeedReceiptState::DirectTerminalUnacked
    );
    assert_cancelled_terminal(terminal_of_receipt(converted_receipt));
    assert_max_result_entitlement(converted_receipt);
    assert_exact_response_identity(conversion, converted_receipt);
}

#[test]
fn retained_receipt_backed_terminals_do_not_starve_target_direct_load() {
    let report = execute(Scenario::fake(vec![
        Action::RunDirectLoad {
            calls: 28_800,
            duration_ms: TOMBSTONE_TTL_MS,
            concurrency: 32,
            retained_receipt_terminals: 32,
            immediate_ack: true,
            label: "target-load".to_string(),
        },
        checkpoint_action("retained-after-load"),
        Action::FillReceiptPool {
            state: SeedReceiptState::ReservedUnbound,
            count: 32,
        },
        checkpoint_action("exact-sixty-four"),
        Action::Submit {
            request: RequestCase::Canonical,
            response_budget_ms: CUTOFF_MS,
            disconnect: DisconnectPoint::Never,
            label: "sixty-fifth".to_string(),
        },
        checkpoint_action("after-sixty-fifth"),
    ]));

    let load = load_run(&report, "target-load");
    assert_direct_load_lifecycles(load, 28_800);
    assert_eq!(load_window_ms(load), TOMBSTONE_TTL_MS);
    assert_eq!(completed_at_window_end(load), 28_800);
    assert!(max_concurrency_sample(load, |sample| sample.live_receipts) <= LIVE_RECEIPT_LIMIT);
    assert!(max_concurrency_sample(load, |sample| sample.owner_slots) <= 65);
    assert_eq!(load.task_store_create_attempts, 0);
    let retained = checkpoint(&report, "retained-after-load");
    assert_retained_receipt_backed_set(retained, 32);
    assert_tombstones_match_active_ack_window(retained, load);
    assert_eq!(
        checkpoint(&report, "exact-sixty-four").receipt_live_count,
        LIVE_RECEIPT_LIMIT
    );
    assert_eq!(
        response(&report, "sixty-fifth").kind,
        ResponseKind::Rejected
    );
    assert_eq!(
        response(&report, "sixty-fifth").error,
        Some(ErrorCode::ReceiptCapacity)
    );
    assert_eq!(
        checkpoint(&report, "after-sixty-fifth").callbacks.execute,
        28_800
    );
}

#[test]
fn deterministic_horizon_load_does_not_saturate() {
    let report = execute(Scenario::fake(vec![
        Action::FillTaskLinks,
        checkpoint_action("before-horizon-load"),
        Action::RunDirectLoad {
            calls: 28_800,
            duration_ms: TOMBSTONE_TTL_MS,
            concurrency: 32,
            retained_receipt_terminals: 32,
            immediate_ack: true,
            label: "horizon-load".to_string(),
        },
        checkpoint_action("after-horizon-load"),
        Action::AdvanceEpoch {
            millis: TOMBSTONE_TTL_MS - 1,
        },
        Action::ReclaimExpiredEvidence,
        Action::RotateReceiptSegments,
        checkpoint_action("one-ms-before-tombstone-expiry"),
        Action::AdvanceEpoch { millis: 1 },
        Action::ReclaimExpiredEvidence,
        Action::RotateReceiptSegments,
        checkpoint_action("at-tombstone-expiry"),
        Action::Restart,
        checkpoint_action("horizon-reopened"),
        Action::FillReceiptPool {
            state: SeedReceiptState::ReservedUnbound,
            count: 32,
        },
        checkpoint_action("horizon-exact-sixty-four"),
        direct_provider(),
        Action::Submit {
            request: RequestCase::Fresh(65),
            response_budget_ms: CUTOFF_MS,
            disconnect: DisconnectPoint::Never,
            label: "horizon-sixty-fifth".to_string(),
        },
        checkpoint_action("horizon-after-boundary"),
        Action::Reset,
        Action::FillTombstones,
        checkpoint_action("tombstone-full"),
        direct_provider(),
        submit("tombstone-overflow-terminal"),
        checkpoint_action("before-tombstone-overflow-ack"),
        Action::Acknowledge {
            key: KeyCase::Exact,
            digest: DigestCase::ExactTerminal,
            disconnect: AckDisconnectPoint::Never,
            label: "tombstone-overflow-ack".to_string(),
        },
        checkpoint_action("after-tombstone-overflow-ack"),
        Action::AdvanceEpoch {
            millis: TOMBSTONE_TTL_MS - 1,
        },
        Action::ReclaimExpiredEvidence,
        Action::RotateReceiptSegments,
        checkpoint_action("tombstone-pool-one-ms-before-expiry"),
        Action::AdvanceEpoch { millis: 1 },
        Action::ReclaimExpiredEvidence,
        Action::RotateReceiptSegments,
        Action::Acknowledge {
            key: KeyCase::Exact,
            digest: DigestCase::ExactTerminal,
            disconnect: AckDisconnectPoint::Never,
            label: "tombstone-ack-after-reclaim".to_string(),
        },
        checkpoint_action("tombstone-pool-reclaimed"),
    ]));

    let model = load_run(&report, "horizon-load");
    assert_direct_load_lifecycles(model, 28_800);
    assert_eq!(completed_at_window_end(model), 28_800);
    assert_eq!(load_window_ms(model), TOMBSTONE_TTL_MS);
    assert_eq!(
        max_concurrency_sample(model, |sample| sample.live_receipts),
        LIVE_RECEIPT_LIMIT
    );
    assert_eq!(model.task_store_create_attempts, 0);
    assert_eq!(model.listener, ListenerState::Listening);
    let before_load = checkpoint(&report, "before-horizon-load");
    assert_exact_linked_task_pool(before_load, TASK_LINK_LIMIT);
    let after_load = checkpoint(&report, "after-horizon-load");
    assert_retained_receipt_backed_set(after_load, 32);
    assert_tombstones_match_active_ack_window(after_load, model);
    assert_exact_linked_task_pool(after_load, TASK_LINK_LIMIT);
    assert_eq!(after_load.task_link_bytes, before_load.task_link_bytes);
    assert_eq!(after_load.task_links, before_load.task_links);
    assert_eq!(after_load.tasks, before_load.tasks);
    let before_expiry = checkpoint(&report, "one-ms-before-tombstone-expiry");
    assert_eq!(before_expiry.receipt_live_count, 32);
    assert_eq!(before_expiry.task_link_count, TASK_LINK_LIMIT);
    assert_tombstones_match_active_ack_window(before_expiry, model);
    assert!(before_expiry.store_generation > after_load.store_generation);
    let at_expiry = checkpoint(&report, "at-tombstone-expiry");
    assert_tombstones_match_active_ack_window(at_expiry, model);
    assert_eq!(at_expiry.receipt_live_count, 32);
    assert_eq!(at_expiry.task_link_count, TASK_LINK_LIMIT);
    assert!(at_expiry.store_generation > before_expiry.store_generation);
    let reopened = checkpoint(&report, "horizon-reopened");
    assert_exact_linked_task_pool(reopened, TASK_LINK_LIMIT);
    assert_eq!(reopened.receipt_live_count, at_expiry.receipt_live_count);
    assert_eq!(
        reopened.receipt_actual_bytes,
        at_expiry.receipt_actual_bytes
    );
    assert_eq!(
        reopened.receipt_reserved_bytes,
        at_expiry.receipt_reserved_bytes
    );
    assert_eq!(reopened.task_link_count, at_expiry.task_link_count);
    assert_eq!(reopened.task_link_bytes, at_expiry.task_link_bytes);
    assert_eq!(reopened.task_links, at_expiry.task_links);
    assert_eq!(reopened.tombstones, at_expiry.tombstones);
    assert_eq!(reopened.receipts, at_expiry.receipts);
    assert_eq!(reopened.tasks, at_expiry.tasks);
    assert_eq!(reopened.store_generation, at_expiry.store_generation);
    assert_eq!(
        checkpoint(&report, "horizon-exact-sixty-four").receipt_live_count,
        LIVE_RECEIPT_LIMIT
    );
    assert_eq!(
        response(&report, "horizon-sixty-fifth").kind,
        ResponseKind::Rejected
    );
    assert_eq!(
        response(&report, "horizon-sixty-fifth").error,
        Some(ErrorCode::ReceiptCapacity)
    );
    assert_eq!(
        checkpoint(&report, "horizon-after-boundary").receipt_live_count,
        LIVE_RECEIPT_LIMIT
    );
    let tombstone_full = checkpoint(&report, "tombstone-full");
    assert_eq!(tombstone_full.tombstone_count, TOMBSTONE_LIMIT);
    let before_overflow = checkpoint(&report, "before-tombstone-overflow-ack");
    let after_overflow = checkpoint(&report, "after-tombstone-overflow-ack");
    assert_eq!(after_overflow.tombstones, tombstone_full.tombstones);
    assert_eq!(after_overflow.receipts, before_overflow.receipts);
    assert_eq!(
        after_overflow.store_generation,
        before_overflow.store_generation
    );
    assert_eq!(
        response(&report, "tombstone-overflow-ack").error,
        Some(ErrorCode::TombstoneCapacity)
    );
    assert_eq!(
        response(&report, "tombstone-overflow-ack").kind,
        ResponseKind::Rejected
    );
    assert_eq!(
        checkpoint(&report, "tombstone-pool-one-ms-before-expiry").tombstones,
        tombstone_full.tombstones
    );
    let reclaimed = checkpoint(&report, "tombstone-pool-reclaimed");
    assert_eq!(reclaimed.tombstone_count, 1);
    assert!(reclaimed.receipts.is_empty());
    assert_eq!(
        response(&report, "tombstone-ack-after-reclaim").kind,
        ResponseKind::Acknowledged
    );
}

#[test]
fn wall_clock_writer_sustains_32_receipts_per_second_on_each_os() {
    let report = execute(Scenario::wall(vec![Action::RunDirectLoad {
        calls: 1_920,
        duration_ms: 60_000,
        concurrency: 32,
        retained_receipt_terminals: 0,
        immediate_ack: true,
        label: "wall-clock".to_string(),
    }]));

    let load = load_run(&report, "wall-clock");
    assert_direct_load_lifecycles(load, 1_920);
    assert!(load_window_ms(load) >= 60_000);
    assert!(
        load_total_elapsed_ms(load) <= 62_000,
        "wall gate overran: {}ms",
        load_total_elapsed_ms(load)
    );
    assert!(completed_at_window_end(load) >= 1_920);
    assert!(load_p99_ms(load) <= 250, "p99={}ms", load_p99_ms(load));
    assert!(load_writer_drain_ms(load) <= 2_000);
    assert!(max_concurrency_sample(load, |sample| sample.live_receipts) <= LIVE_RECEIPT_LIMIT);
    assert_eq!(load.task_store_create_attempts, 0);
    assert_eq!(load.listener, ListenerState::Listening);
}

#[test]
fn oversized_result_and_uncertain_store_commit_fail_closed() {
    let near_limit = execute(Scenario::fake(vec![
        Action::ConfigureProvider {
            execution_class: ExecutionClass::Direct,
            terminal: TerminalFixture::Bytes {
                count: MAX_CANONICAL_RESULT_BYTES,
            },
            cooperative_cancel: true,
            side_effect_marker: false,
        },
        submit("near-limit"),
        checkpoint_action("near-limit"),
    ]));
    let near_limit_snapshot = checkpoint(&near_limit, "near-limit");
    let near_limit_receipt = only_receipt(near_limit_snapshot);
    assert!(completed_result(terminal_of_receipt(near_limit_receipt)).ok);
    let near_limit_publication = terminal_publication_for(
        &near_limit,
        &near_limit_receipt.key,
        terminal_of_receipt(near_limit_receipt),
    );
    assert_eq!(
        near_limit_publication
            .commit
            .receipt()
            .expect("near-limit Direct publication is receipt-owned")
            .candidate_result
            .as_ref()
            .expect("near-limit success needs exact canonical result evidence")
            .encoded_bytes,
        MAX_CANONICAL_RESULT_BYTES
    );
    assert!(near_limit_publication
        .response_frames
        .iter()
        .all(|frame| frame.response_jsonl.encoded_bytes <= MAX_RESPONSE_LINE_BYTES));
    assert_publication_matches_snapshot(
        &near_limit,
        near_limit_snapshot,
        &near_limit_receipt.key,
        terminal_of_receipt(near_limit_receipt),
        TerminalPublicationOwner::DirectReceiptLedger,
    );

    let oversized = execute(Scenario::fake(vec![
        Action::ConfigureProvider {
            execution_class: ExecutionClass::Direct,
            terminal: TerminalFixture::Bytes {
                count: 8 * 1_024 * 1_024 + 1,
            },
            cooperative_cancel: true,
            side_effect_marker: false,
        },
        submit("oversized"),
        checkpoint_action("oversized"),
    ]));
    let oversized_snapshot = checkpoint(&oversized, "oversized");
    let oversized_receipt = only_receipt(oversized_snapshot);
    assert_failed_terminal(
        terminal_of_receipt(oversized_receipt),
        V5SafeFailureReason::ResultTooLarge,
    );
    let oversized_response = response(&oversized, "oversized");
    assert_eq!(oversized_response.kind, ResponseKind::Direct);
    assert_eq!(oversized_response.error, Some(ErrorCode::ResultTooLarge));
    assert_eq!(
        response_terminal(oversized_response),
        terminal_of_receipt(oversized_receipt)
    );
    assert_exact_response_identity(oversized_response, oversized_receipt);
    let publication = terminal_publication_for(
        &oversized,
        &oversized_receipt.key,
        terminal_of_receipt(oversized_receipt),
    );
    let preflight = &publication.commit;
    assert_eq!(
        preflight
            .receipt()
            .expect("oversized Direct failure is receipt-owned")
            .candidate_result
            .as_ref()
            .expect("oversized candidate evidence")
            .encoded_bytes,
        MAX_CANONICAL_RESULT_BYTES + 1
    );
    let direct_frame = publication
        .response_frames
        .iter()
        .find(|frame| frame.response_kind == ResponseKind::Direct)
        .expect("oversized Direct failure must preflight its final response write");
    assert!(direct_frame.response_jsonl.encoded_bytes <= MAX_RESPONSE_LINE_BYTES);
    assert!(oversized_receipt.encoded_bytes <= MAX_RESPONSE_LINE_BYTES);
    assert_publication_matches_snapshot(
        &oversized,
        oversized_snapshot,
        &oversized_receipt.key,
        terminal_of_receipt(oversized_receipt),
        TerminalPublicationOwner::DirectReceiptLedger,
    );

    let transfer_boundary = execute(Scenario::fake(vec![
        Action::ConfigureProvider {
            execution_class: ExecutionClass::KnownLong,
            terminal: TerminalFixture::NearLimitWithMaximumMetadata {
                canonical_result_bytes: MAX_CANONICAL_RESULT_BYTES,
            },
            cooperative_cancel: true,
            side_effect_marker: false,
        },
        Action::InstallBarrier {
            point: BarrierPoint::BeforeTaskStoreCreate,
        },
        submit("transfer-boundary"),
        Action::AdvanceMonotonic { millis: CUTOFF_MS },
        Action::WaitForEvent {
            event: EventKind::BoundHandoffTerminalStaged,
        },
        Action::ReleaseBarrier {
            point: BarrierPoint::BeforeTaskStoreCreate,
        },
        Action::WaitForEvent {
            event: EventKind::TaskTerminalBoundCommitted,
        },
        checkpoint_action("transfer-boundary"),
    ]));
    let boundary_snapshot = checkpoint(&transfer_boundary, "transfer-boundary");
    let boundary_task = only_task(boundary_snapshot);
    let boundary_terminal = boundary_task
        .terminal
        .as_ref()
        .expect("pre-stage transfer sizing must publish the durable terminal");
    assert!(completed_result(boundary_terminal).ok);
    assert_eq!(
        task_link_state(only_task_link(boundary_snapshot)),
        SeedReceiptState::TaskTerminalBound
    );
    assert!(boundary_snapshot.receipts.is_empty());
    let boundary_preparation = transfer_boundary
        .staged_terminal_preparations
        .iter()
        .find(|preparation| preparation.receipt_key == only_task_link(boundary_snapshot).key)
        .expect("near-limit transfer must expose its pre-stage size certificate");
    assert!(completed_result(&boundary_preparation.terminal).ok);
    assert_eq!(
        boundary_preparation
            .candidate_result
            .as_ref()
            .expect("transfer-only overflow keeps the bounded canonical candidate")
            .encoded_bytes,
        MAX_CANONICAL_RESULT_BYTES
    );
    assert_eq!(
        boundary_task.terminal.as_ref(),
        Some(&boundary_preparation.terminal),
        "no post-stage reclassification is permitted"
    );
    assert_ne!(
        response(&transfer_boundary, "transfer-boundary").error,
        Some(ErrorCode::ResultTooLarge),
        "a certified 8 MiB canonical result cannot be reclassified after staging"
    );
    assert_publication_matches_snapshot(
        &transfer_boundary,
        boundary_snapshot,
        &only_task_link(boundary_snapshot).key,
        boundary_terminal,
        TerminalPublicationOwner::StagedHandoffTask,
    );

    let uncertain = execute(Scenario::fake(vec![
        direct_provider(),
        Action::InjectStoreFault {
            point: StoreFaultPoint::AfterTerminalPayloadRenameBeforeDirectorySync,
        },
        submit("uncertain"),
        checkpoint_action("fail-closed"),
    ]));
    let failed = checkpoint(&uncertain, "fail-closed");
    assert_eq!(failed.listener, ListenerState::Closed);
    assert!(failed.restart_requested);
    assert_eq!(failed.staged_responses_exposed, 0);
    assert_eq!(failed.fallback_executions, 0);
    assert_eq!(
        response(&uncertain, "uncertain").kind,
        ResponseKind::Rejected
    );
    assert_eq!(
        response(&uncertain, "uncertain").error,
        Some(ErrorCode::StoreCommitUncertain)
    );
    assert_eq!(count_event(&uncertain, EventKind::ResultSerialized), 1);
    assert_eq!(
        count_event(&uncertain, EventKind::ReceiptTerminalCommitted),
        0
    );
    assert_eq!(count_event(&uncertain, EventKind::ListenerClosed), 1);

    let task_create_uncertain = execute(Scenario::fake(vec![
        known_long_provider(),
        Action::InjectStoreFault {
            point: StoreFaultPoint::AfterTaskCreateRenameBeforeDirectorySync,
        },
        submit("task-create-uncertain"),
        checkpoint_action("task-create-fail-closed"),
    ]));
    let task_failed = checkpoint(&task_create_uncertain, "task-create-fail-closed");
    assert_eq!(task_failed.listener, ListenerState::Closed);
    assert!(task_failed.restart_requested);
    assert!(!task_failed.daemon_running);
    assert_eq!(task_failed.staged_responses_exposed, 0);
    assert_eq!(task_failed.fallback_executions, 0);
    assert_eq!(task_failed.task_store_create_attempts, 1);
    assert_eq!(task_failed.task_link_reserved_count, 1);
    assert!(task_failed.tasks.is_empty());
    assert_eq!(
        only_receipt(task_failed).state,
        SeedReceiptState::TaskHandoffActorBoundBegun
    );
    assert!(only_receipt(task_failed).terminal.is_none());
    assert_eq!(
        response(&task_create_uncertain, "task-create-uncertain").error,
        Some(ErrorCode::StoreCommitUncertain)
    );
    assert_eq!(
        count_event(&task_create_uncertain, EventKind::TaskStoreCreateAttempted),
        1
    );
    assert_eq!(
        count_event(
            &task_create_uncertain,
            EventKind::TaskStoreCapacityInvariantViolation,
        ),
        0
    );
    assert_eq!(
        count_event(
            &task_create_uncertain,
            EventKind::TaskLinkReservationReleased
        ),
        0
    );
}
