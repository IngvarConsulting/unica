//! Провод сценария контракта ReceiptLedger: типы, в которые разбирается
//! запрос, и их отображение в события рантайма. Только форма запроса —
//! ни одного действия над квитанцией.

use super::*;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ReceiptScenario {
    pub(super) clock: ScenarioClock,
    pub(super) actions: Vec<ReceiptScenarioAction>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ScenarioClock {
    Fake,
    Wall,
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum ReceiptScenarioAction {
    ConfigureValidation {
        reject: bool,
    },
    ConfigureProvider {
        execution_class: ScenarioExecutionClass,
        terminal: ScenarioTerminalFixture,
        cooperative_cancel: bool,
        side_effect_marker: bool,
    },
    ConfigureAdmission {
        rejection: Option<ScenarioWorkspaceAdmissionFailure>,
    },
    ConfigurePrepare {
        reject: bool,
    },
    Cancel {
        key: ScenarioKey,
        #[serde(rename = "lazy_session")]
        _lazy_session: bool,
        label: String,
    },
    CancelTask {
        api: ScenarioTaskCancelApi,
        task: ScenarioTaskSelector,
        #[serde(rename = "lazy_session")]
        _lazy_session: bool,
        label: String,
    },
    SpawnCancel {
        key: ScenarioKey,
        #[serde(rename = "lazy_session")]
        _lazy_session: bool,
        label: String,
    },
    SpawnMarkReservedBegun {
        proof: ScenarioActorProof,
        label: String,
    },
    SpawnTaskStoreCreateAndBindUnderGate {
        label: String,
    },
    SpawnStageBoundHandoffTerminal {
        terminal: ScenarioTerminalFixture,
        label: String,
    },
    WaitForOperation {
        label: String,
        state: ScenarioOperationState,
    },
    WaitForEventCount {
        event: ScenarioEvent,
        count: u32,
    },
    Submit {
        request: ScenarioRequest,
        response_budget_ms: u64,
        disconnect: ScenarioDisconnect,
        label: String,
    },
    SpawnSubmit {
        request: ScenarioRequest,
        response_budget_ms: u64,
        disconnect: ScenarioDisconnect,
        label: String,
    },
    SendOuterEnvelope {
        envelope: ScenarioEnvelopeCase,
        label: String,
    },
    ProbeProtocol {
        client: ScenarioProtocolVersion,
        server: ScenarioProtocolVersion,
        message: ScenarioProtocolMessage,
        label: String,
    },
    Recover {
        key: ScenarioKey,
        label: String,
    },
    Acknowledge {
        key: ScenarioKey,
        digest: ScenarioDigest,
        disconnect: ScenarioAckDisconnect,
        label: String,
    },
    SeedReceipt {
        state: ScenarioSeedReceiptState,
        cancel_requested: bool,
        staged_terminal: Option<ScenarioTerminalFixture>,
    },
    SeedTask {
        status: ScenarioTaskStatus,
        cancel_requested: bool,
        receipt_link: ScenarioReceiptLinkCase,
        identity: ScenarioIdentityRelation,
        version: u64,
    },
    SeedTaskLinkReservation {
        relation: ScenarioIdentityRelation,
    },
    AttemptStagedTerminalAgainstProvisional {
        mismatch: Option<ScenarioProvisionalMismatchField>,
        repeat_same_terminal: bool,
        label: String,
    },
    InjectPersistedIdentityCollision {
        index: ScenarioIdentityIndex,
    },
    InjectStoreFault {
        point: ScenarioStoreFaultPoint,
    },
    OpenTaskStoreInspectOnly,
    ReconcileStartup,
    PublishListener,
    ReadTask {
        api: ScenarioTaskApi,
        label: String,
    },
    AttemptBoundTaskStart {
        proof: ScenarioActorProof,
        label: String,
    },
    InvalidateActorProof {
        proof: ScenarioActorProof,
        point: ScenarioBarrierPoint,
        label: String,
    },
    AdvanceEpoch {
        millis: u64,
    },
    AdvanceMonotonic {
        millis: u64,
    },
    Crash {
        point: ScenarioCrashPoint,
    },
    Restart,
    Checkpoint {
        label: String,
    },
    Reset,
    FillReceiptPool {
        state: ScenarioSeedReceiptState,
        count: u32,
    },
    FillTaskLinks,
    FillTaskLinksLeavingOneReservationSlot,
    FillTombstones,
    InjectTaskStoreCapacityInvariantViolationOnce,
    AttemptTaskStoreBindUnderGate {
        label: String,
    },
    AttemptUnstagedTaskBindAgainstStagedTerminal {
        label: String,
    },
    ContinueReceiptOwnedAttempt {
        terminal: ScenarioTerminalFixture,
        label: String,
    },
    RunCrossStoreCrashWorkload {
        cases: Vec<ScenarioCrashWorkload>,
    },
    RunTaskRetirementWorkload {
        cases: Vec<ScenarioTaskRetirementWorkload>,
    },
    RunLazyCancelStorm {
        submits: u32,
        cancels: u32,
        per_cancel_deadline_ms: u64,
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
    JoinOperation {
        label: String,
    },
    InstallBarrier {
        point: ScenarioBarrierPoint,
    },
    WaitForEvent {
        event: ScenarioEvent,
    },
    ReleaseBarrier {
        point: ScenarioBarrierPoint,
    },
    CompareClientServerIdentity,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ScenarioExecutionClass {
    Direct,
    KnownLong,
}

#[derive(Clone, Deserialize)]
#[serde(tag = "terminal", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum ScenarioTerminalFixture {
    Success { payload: String },
    Bytes { count: u64 },
    NearLimitWithMaximumMetadata { canonical_result_bytes: u64 },
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ScenarioRequest {
    Canonical,
    Fresh(u32),
    SameIdentity,
    Mismatch(ScenarioIdentityField),
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ScenarioTaskApi {
    NativeGet,
    NativeWait,
    CompatibilityGet,
    CompatibilityResult,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ScenarioTaskCancelApi {
    Native,
    Compatibility,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ScenarioTaskSelector {
    ExactProjected,
    ForReadLabel(String),
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ScenarioEnvelopeCase {
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

#[derive(Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum ScenarioProtocolVersion {
    V3,
    V4,
    V5,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ScenarioCoreIdentitySelection {
    ExactProductionV5,
    ArbitraryCanonical,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ScenarioFailureProbeReason {
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

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ScenarioTaskTerminalOwnerFixture {
    ReceiptBacked,
    Bound,
    Staged,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ScenarioV5DaemonErrorCodeFixture {
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
    StoreCommitUncertain,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ScenarioStrictSchemaTarget {
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

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ScenarioProtocolMessage {
    Ping,
    Release,
    SubmitWithCoreIdentity {
        selection: ScenarioCoreIdentitySelection,
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
        code: ScenarioV5DaemonErrorCodeFixture,
    },
    MalformedV5Schema {
        target: ScenarioStrictSchemaTarget,
    },
    ReceiptPendingOutcome,
    TaskOutcome,
    AcknowledgedOutcome,
    DirectCompletedTerminal,
    DirectSemanticCompletedTerminal,
    DirectCancelledTerminal,
    DirectFailureTerminal {
        reason: ScenarioFailureProbeReason,
    },
    TaskQueuedProjection,
    TaskWorkingProjection,
    TaskCompletedProjection,
    TaskSemanticCompletedProjection {
        owner: ScenarioTaskTerminalOwnerFixture,
    },
    TaskCancelledProjection,
    TaskFailureProjection {
        reason: ScenarioFailureProbeReason,
    },
    StoredInvocationRecord {
        schema_version: u8,
        reason: ScenarioFailureProbeReason,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ScenarioDisconnect {
    Never,
    AfterSubmitWrite,
    AfterTerminalCommit,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ScenarioAckDisconnect {
    Never,
    AfterTombstoneCommit,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ScenarioDigest {
    ExactTerminal,
    Mismatched,
    TaskTerminal,
    WellFormedCandidate,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ScenarioKey {
    Exact,
    ForSubmitLabel(String),
    Unknown,
    Mismatch(ScenarioIdentityField),
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ScenarioSeedReceiptState {
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

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ScenarioTaskStatus {
    Queued,
    Working,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ScenarioIdentityRelation {
    Exact,
    Foreign,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ScenarioIdentityIndex {
    InvocationId,
    ReservedTaskId,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ScenarioReceiptLinkCase {
    Exact,
    Missing,
    Foreign,
}

pub(super) type ScenarioBarrierPoint = V5PausePoint;

pub(super) type ScenarioWorkspaceAdmissionFailure = V5AdmissionRejection;

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ScenarioCrashPoint {
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

pub(super) type ScenarioStoreFaultPoint = V5StoreFaultPoint;

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ScenarioEntryPath {
    PromisedUnbound,
    ReservedActorBound,
    ReservedBegun,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ScenarioCrashWorkload {
    pub(super) path: ScenarioEntryPath,
    pub(super) point: ScenarioCrashPoint,
    pub(super) cancel_before_crash: bool,
    pub(super) stage_terminal_before_crash: bool,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ScenarioTaskRetirementWorkload {
    RecoveryTerminalBeforeTerminalBound,
    ActiveTaskBoundAbsent,
    BeforePendingIntent,
    AfterPendingIntentBeforeDelete,
    AfterDeleteCommitUncertain,
    AfterDeletedBeforeLedgerFinalize,
    AfterAbsentConfirmedBeforeLedgerFinalize,
    DeleteIdentityMismatch,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Clone, Copy)]
pub(super) enum ScenarioEvent {
    V5ReceiptRuntimeEntered,
    CanonicalV13ServiceEntered,
    ReceiptReserved,
    ValidationEntered,
    AdmissionEntered,
    ActorBoundCommitted,
    ReceiptBegunCommitted,
    PrepareEntered,
    ExecuteEntered,
    CancelReservationConverted,
    ResultSerialized,
    ReceiptTerminalCommitted,
    FinalResultProjected,
    AcknowledgementCommitted,
    BoundHandoffCommitted,
    BoundHandoffTerminalStaged,
    TaskBoundCommitted,
    TaskStoreWorkingReadback,
    FalseCancelObservationReached,
    TaskStoreTerminalCommitted,
    TaskStoreTerminalReadback,
    TaskTerminalBoundCommitted,
    TokenSignalled,
    MarkReservedBegunBlocked,
    CancelCommitBlocked,
    TaskStoreReadbackBeforeBind,
    CancelCommitted,
    OperationCompleted,
    LeaseReleased,
    ListenerClosed,
}

pub(super) fn scenario_runtime_event_kind(event: ScenarioEvent) -> V5ReceiptRuntimeEventKind {
    match event {
        ScenarioEvent::V5ReceiptRuntimeEntered => {
            V5ReceiptRuntimeEventKind::V5ReceiptRuntimeEntered
        }
        ScenarioEvent::CanonicalV13ServiceEntered => {
            V5ReceiptRuntimeEventKind::CanonicalV13ServiceEntered
        }
        ScenarioEvent::ReceiptReserved => V5ReceiptRuntimeEventKind::ReceiptReserved,
        ScenarioEvent::ValidationEntered => V5ReceiptRuntimeEventKind::ValidationEntered,
        ScenarioEvent::AdmissionEntered => V5ReceiptRuntimeEventKind::AdmissionEntered,
        ScenarioEvent::ActorBoundCommitted => V5ReceiptRuntimeEventKind::ActorBoundCommitted,
        ScenarioEvent::ReceiptBegunCommitted => V5ReceiptRuntimeEventKind::ReceiptBegunCommitted,
        ScenarioEvent::PrepareEntered => V5ReceiptRuntimeEventKind::PrepareEntered,
        ScenarioEvent::ExecuteEntered => V5ReceiptRuntimeEventKind::ExecuteEntered,
        ScenarioEvent::CancelReservationConverted => {
            V5ReceiptRuntimeEventKind::CancelReservationConverted
        }
        ScenarioEvent::ResultSerialized => V5ReceiptRuntimeEventKind::ResultSerialized,
        ScenarioEvent::ReceiptTerminalCommitted => {
            V5ReceiptRuntimeEventKind::ReceiptTerminalCommitted
        }
        ScenarioEvent::FinalResultProjected => V5ReceiptRuntimeEventKind::FinalResultProjected,
        ScenarioEvent::AcknowledgementCommitted => {
            V5ReceiptRuntimeEventKind::AcknowledgementCommitted
        }
        ScenarioEvent::BoundHandoffCommitted => V5ReceiptRuntimeEventKind::BoundHandoffCommitted,
        ScenarioEvent::BoundHandoffTerminalStaged => {
            V5ReceiptRuntimeEventKind::BoundHandoffTerminalStaged
        }
        ScenarioEvent::TaskBoundCommitted => V5ReceiptRuntimeEventKind::TaskBoundCommitted,
        ScenarioEvent::TaskStoreWorkingReadback => {
            V5ReceiptRuntimeEventKind::TaskStoreWorkingReadback
        }
        ScenarioEvent::FalseCancelObservationReached => {
            V5ReceiptRuntimeEventKind::FalseCancelObservationReached
        }
        ScenarioEvent::TaskStoreTerminalCommitted => {
            V5ReceiptRuntimeEventKind::TaskStoreTerminalCommitted
        }
        ScenarioEvent::TaskStoreTerminalReadback => {
            V5ReceiptRuntimeEventKind::TaskStoreTerminalReadback
        }
        ScenarioEvent::TaskTerminalBoundCommitted => {
            V5ReceiptRuntimeEventKind::TaskTerminalBoundCommitted
        }
        ScenarioEvent::TokenSignalled => V5ReceiptRuntimeEventKind::TokenSignalled,
        ScenarioEvent::MarkReservedBegunBlocked => {
            V5ReceiptRuntimeEventKind::MarkReservedBegunBlocked
        }
        ScenarioEvent::CancelCommitBlocked => V5ReceiptRuntimeEventKind::CancelCommitBlocked,
        ScenarioEvent::TaskStoreReadbackBeforeBind => {
            V5ReceiptRuntimeEventKind::TaskStoreReadbackBeforeBind
        }
        ScenarioEvent::CancelCommitted => V5ReceiptRuntimeEventKind::CancelCommitted,
        ScenarioEvent::OperationCompleted => V5ReceiptRuntimeEventKind::OperationCompleted,
        ScenarioEvent::LeaseReleased => V5ReceiptRuntimeEventKind::LeaseReleased,
        ScenarioEvent::ListenerClosed => V5ReceiptRuntimeEventKind::ListenerClosed,
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ScenarioIdentityField {
    InvocationId,
    ReservedTaskId,
    CoreIdentity,
    ToolIdentity,
    NormalizedArgumentsHash,
    RequestScopeHash,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ScenarioProvisionalMismatchField {
    TaskId,
    InvocationId,
    Status,
    Version,
    CancelRequested,
    TaskLinkDigest,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ScenarioActorProof {
    Exact,
    Missing,
    Foreign,
    Stale,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ScenarioOperationState {
    Blocked,
    Completed,
}
