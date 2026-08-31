//! Feature-only integration facade for the protocol-v5 ReceiptLedger matrix.
//!
//! This module is intentionally scenario-oriented. It cannot expose stores or
//! construct production evidence; production owners return opaque evidence
//! after crossing their real boundaries.

use crate::application::receipt_ledger::{
    canonical_v5_terminal, receipt_key_digest, request_scope_hash, task_link_digest,
    CoreIdentityDigest, ReceiptKey, ReceiptKeyDigest, ReceiptTerminalOutcome, RequestIdentity,
    RequestScopeHash, TaskLinkIdentity, V5ToolIdentity,
};
use crate::domain::invocation::{InvocationId, NormalizedArgumentsHash, SafeIdentityHash, TaskId};
use crate::infrastructure::daemon::protocol_v5::{
    run_strict_envelope_reachability_probe_for_test, StrictV5EnvelopeCase,
};
use crate::infrastructure::daemon::runtime_v5::{
    run_direct_load_reachability_probe_for_test, run_lazy_cancel_storm_reachability_probe_for_test,
    run_protocol_ping_reachability_probe_for_test, run_seed_task_reachability_probe_for_test,
    run_submit_reachability_probe_for_test,
};
use crate::infrastructure::receipt_ledger_reachability::{
    run_acknowledge_reachability_probe_for_test,
    run_attempt_task_store_bind_under_gate_reachability_probe_for_test,
    run_cancel_reachability_probe_for_test,
    run_compare_client_server_identity_reachability_probe_for_test,
    run_cross_store_crash_workload_reachability_probe_for_test,
    run_fill_receipt_pool_reachability_probe_for_test,
    run_fill_task_links_leaving_one_reservation_slot_reachability_probe_for_test,
    run_fill_task_links_reachability_probe_for_test,
    run_fill_tombstones_reachability_probe_for_test,
    run_inject_persisted_identity_collision_reachability_probe_for_test,
    run_inject_store_fault_reachability_probe_for_test,
    run_open_task_store_inspect_only_reachability_probe_for_test,
    run_reconcile_startup_reachability_probe_for_test, run_recover_reachability_probe_for_test,
    run_seed_receipt_reachability_probe_for_test, run_spawn_cancel_reachability_probe_for_test,
    run_task_retirement_workload_reachability_probe_for_test,
};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use std::fmt;
use std::str::FromStr;

pub fn execute_scenario_json(request: &str) -> Result<String, String> {
    execute_scenario(request).map_err(|error| error.to_string())
}

pub fn request_scope_hash_for_test(workspace_hint: &str) -> String {
    request_scope_hash(workspace_hint)
        .unwrap_or_else(|error| panic!("invalid request scope supplied by test: {error}"))
        .to_string()
}

pub fn receipt_key_digest_for_test(
    invocation_id: &str,
    reserved_task_id: &str,
    core_identity_digest: &str,
    tool_wire_name: &str,
    normalized_arguments_hash: &str,
    request_scope_hash_value: &str,
) -> String {
    let core_identity_digest: CoreIdentityDigest =
        parse_from_str(core_identity_digest, "core identity digest");
    let normalized_arguments_hash: NormalizedArgumentsHash =
        parse_application_json_string(normalized_arguments_hash, "normalized arguments hash");
    let request_scope_hash_value: RequestScopeHash =
        parse_from_str(request_scope_hash_value, "request scope hash");
    let request_identity = RequestIdentity::new(
        core_identity_digest,
        V5ToolIdentity::from_wire_name(tool_wire_name)
            .unwrap_or_else(|| panic!("invalid v5 tool identity supplied by test")),
        normalized_arguments_hash,
        request_scope_hash_value,
    );
    let invocation_id: InvocationId = parse_from_str(invocation_id, "invocation id");
    let reserved_task_id: TaskId = parse_from_str(reserved_task_id, "reserved task id");
    let key = ReceiptKey::new(invocation_id, reserved_task_id, request_identity);
    receipt_key_digest(&key).to_string()
}

pub fn task_link_digest_for_test(
    receipt_key_digest_value: &str,
    task_id: &str,
    invocation_id: &str,
    workspace_identity_hash: &str,
) -> String {
    let receipt_key_digest_value: ReceiptKeyDigest =
        parse_from_str(receipt_key_digest_value, "receipt key digest");
    let task_id: TaskId = parse_from_str(task_id, "task id");
    let invocation_id: InvocationId = parse_from_str(invocation_id, "invocation id");
    let workspace_identity_hash: SafeIdentityHash =
        parse_application_json_string(workspace_identity_hash, "workspace identity hash");
    let identity = TaskLinkIdentity::new(
        receipt_key_digest_value,
        task_id,
        invocation_id,
        workspace_identity_hash,
    );
    task_link_digest(&identity).to_string()
}

pub fn canonical_v5_terminal_for_test(terminal_json: &str) -> (Vec<u8>, String) {
    let outcome: ReceiptTerminalOutcome = serde_json::from_str(terminal_json)
        .unwrap_or_else(|error| panic!("invalid strict v5 terminal supplied by test: {error}"));
    let terminal = canonical_v5_terminal(&outcome)
        .unwrap_or_else(|error| panic!("v5 terminal cannot be canonicalized: {error}"));
    (terminal.payload().to_vec(), terminal.digest().to_string())
}

fn parse_from_str<T>(value: &str, label: &str) -> T
where
    T: FromStr,
    T::Err: fmt::Display,
{
    value
        .parse()
        .unwrap_or_else(|error| panic!("invalid {label} supplied by test: {error}"))
}

fn parse_application_json_string<T>(value: &str, label: &str) -> T
where
    T: DeserializeOwned,
{
    serde_json::from_value(serde_json::Value::String(value.to_owned()))
        .unwrap_or_else(|error| panic!("invalid {label} supplied by test: {error}"))
}

fn execute_scenario(request: &str) -> Result<String, ScenarioExecutionError> {
    macro_rules! return_owner_evidence {
        ($probe:expr, $action_index:expr, $action_kind:literal) => {{
            let evidence = $probe()
                .map_err(|error| ScenarioExecutionError::ProductionProbe(error.to_string()))?;
            return evidence
                .encode_facade_envelope($action_index, $action_kind)
                .map_err(ScenarioExecutionError::EvidenceEncode);
        }};
    }

    let scenario: ScenarioInput = serde_json::from_str(request)
        .map_err(|error| ScenarioExecutionError::InvalidScenario(error.to_string()))?;
    let ScenarioInput { clock, actions } = scenario;
    let mut setup = StagedScenarioSetup::new(clock);

    for (index, action) in actions.into_iter().enumerate() {
        let action_index = u32::try_from(index)
            .map_err(|_| ScenarioExecutionError::ActionIndexOutOfRange(index))?;
        match action {
            ActionInput::ConfigureValidation { reject } => {
                setup.validation = Some(ValidationSetup { reject });
            }
            ActionInput::ConfigureProvider {
                execution_class,
                terminal,
                cooperative_cancel,
                side_effect_marker,
            } => {
                setup.provider = Some(ProviderSetup {
                    execution_class,
                    terminal,
                    cooperative_cancel,
                    side_effect_marker,
                });
            }
            ActionInput::ConfigureAdmission { rejection } => {
                setup.admission = Some(AdmissionSetup { rejection });
            }
            ActionInput::ConfigurePrepare { reject } => {
                setup.prepare = Some(PrepareSetup { reject });
            }
            ActionInput::InstallBarrier { point } => setup.barriers.push(point),
            ActionInput::Submit { .. } => {
                let evidence = run_submit_reachability_probe_for_test()
                    .map_err(ScenarioExecutionError::ProductionProbe)?;
                return evidence
                    .encode_facade_envelope(action_index, "submit")
                    .map_err(ScenarioExecutionError::EvidenceEncode);
            }
            ActionInput::SpawnSubmit { .. } => {
                let evidence = run_submit_reachability_probe_for_test()
                    .map_err(ScenarioExecutionError::ProductionProbe)?;
                return evidence
                    .encode_facade_envelope(action_index, "spawn_submit")
                    .map_err(ScenarioExecutionError::EvidenceEncode);
            }
            ActionInput::ProbeProtocol { .. } => {
                let evidence = run_protocol_ping_reachability_probe_for_test()
                    .map_err(ScenarioExecutionError::ProductionProbe)?;
                return evidence
                    .encode_facade_envelope(action_index, "probe_protocol")
                    .map_err(ScenarioExecutionError::EvidenceEncode);
            }
            ActionInput::SendOuterEnvelope { envelope, .. } => {
                let case = match envelope {
                    EnvelopeCaseInput::MissingInvocationId => {
                        StrictV5EnvelopeCase::MissingInvocationId
                    }
                    EnvelopeCaseInput::NoncanonicalInvocationId => {
                        StrictV5EnvelopeCase::NoncanonicalInvocationId
                    }
                    EnvelopeCaseInput::MissingReservedTaskId => {
                        StrictV5EnvelopeCase::MissingReservedTaskId
                    }
                    EnvelopeCaseInput::NoncanonicalReservedTaskId => {
                        StrictV5EnvelopeCase::NoncanonicalReservedTaskId
                    }
                    EnvelopeCaseInput::UnknownTool => StrictV5EnvelopeCase::UnknownTool,
                    EnvelopeCaseInput::UnknownField => StrictV5EnvelopeCase::UnknownField,
                    EnvelopeCaseInput::MalformedArguments => {
                        StrictV5EnvelopeCase::MalformedArguments
                    }
                    EnvelopeCaseInput::OversizedArguments => {
                        StrictV5EnvelopeCase::OversizedArguments
                    }
                    EnvelopeCaseInput::ResponseBudgetAboveMaximum => {
                        StrictV5EnvelopeCase::ResponseBudgetAboveMaximum
                    }
                    EnvelopeCaseInput::EmptyWorkspaceHint => {
                        StrictV5EnvelopeCase::EmptyWorkspaceHint
                    }
                    EnvelopeCaseInput::WorkspaceHintWithControl => {
                        StrictV5EnvelopeCase::WorkspaceHintWithControl
                    }
                    EnvelopeCaseInput::MalformedWorkspaceHint => {
                        StrictV5EnvelopeCase::MalformedWorkspaceHint
                    }
                    EnvelopeCaseInput::OversizedWorkspaceHint => {
                        StrictV5EnvelopeCase::OversizedWorkspaceHint
                    }
                };
                let evidence = run_strict_envelope_reachability_probe_for_test(case)
                    .map_err(ScenarioExecutionError::ProductionProbe)?;
                return evidence
                    .encode_facade_envelope(action_index, "send_outer_envelope")
                    .map_err(ScenarioExecutionError::EvidenceEncode);
            }
            ActionInput::Recover { .. } => return_owner_evidence!(
                run_recover_reachability_probe_for_test,
                action_index,
                "recover"
            ),
            ActionInput::Acknowledge { .. } => return_owner_evidence!(
                run_acknowledge_reachability_probe_for_test,
                action_index,
                "acknowledge"
            ),
            ActionInput::Cancel { .. } => return_owner_evidence!(
                run_cancel_reachability_probe_for_test,
                action_index,
                "cancel"
            ),
            ActionInput::SpawnCancel { .. } => return_owner_evidence!(
                run_spawn_cancel_reachability_probe_for_test,
                action_index,
                "spawn_cancel"
            ),
            ActionInput::SeedReceipt { .. } => return_owner_evidence!(
                run_seed_receipt_reachability_probe_for_test,
                action_index,
                "seed_receipt"
            ),
            ActionInput::Crash { point } => setup.crash_points.push(point),
            ActionInput::InjectStoreFault { .. } => return_owner_evidence!(
                run_inject_store_fault_reachability_probe_for_test,
                action_index,
                "inject_store_fault"
            ),
            ActionInput::CompareClientServerIdentity => return_owner_evidence!(
                run_compare_client_server_identity_reachability_probe_for_test,
                action_index,
                "compare_client_server_identity"
            ),
            ActionInput::InjectPersistedIdentityCollision { .. } => return_owner_evidence!(
                run_inject_persisted_identity_collision_reachability_probe_for_test,
                action_index,
                "inject_persisted_identity_collision"
            ),
            ActionInput::SeedTask { .. } => return_owner_evidence!(
                run_seed_task_reachability_probe_for_test,
                action_index,
                "seed_task"
            ),
            ActionInput::ReadTask { .. }
            | ActionInput::CancelTask { .. }
            | ActionInput::SeedTaskLinkReservation { .. }
            | ActionInput::SpawnTaskStoreCreateAndBindUnderGate { .. }
            | ActionInput::ContinueReceiptOwnedAttempt { .. }
            | ActionInput::SpawnStageBoundHandoffTerminal { .. }
            | ActionInput::AttemptStagedTerminalAgainstProvisional { .. } => {
                return Err(ScenarioExecutionError::OwnerUnavailable(
                    ScenarioOwner::TaskProjection,
                ));
            }
            ActionInput::RunCrossStoreCrashWorkload { .. } => return_owner_evidence!(
                run_cross_store_crash_workload_reachability_probe_for_test,
                action_index,
                "run_cross_store_crash_workload"
            ),
            ActionInput::RunTaskRetirementWorkload { .. } => return_owner_evidence!(
                run_task_retirement_workload_reachability_probe_for_test,
                action_index,
                "run_task_retirement_workload"
            ),
            ActionInput::OpenTaskStoreInspectOnly => return_owner_evidence!(
                run_open_task_store_inspect_only_reachability_probe_for_test,
                action_index,
                "open_task_store_inspect_only"
            ),
            ActionInput::ReconcileStartup => return_owner_evidence!(
                run_reconcile_startup_reachability_probe_for_test,
                action_index,
                "reconcile_startup"
            ),
            ActionInput::InvalidateActorProof { .. }
            | ActionInput::AttemptBoundTaskStart { .. }
            | ActionInput::SpawnMarkReservedBegun { .. } => {
                return Err(ScenarioExecutionError::OwnerUnavailable(
                    ScenarioOwner::ActorLinearization,
                ));
            }
            ActionInput::FillReceiptPool { .. } => return_owner_evidence!(
                run_fill_receipt_pool_reachability_probe_for_test,
                action_index,
                "fill_receipt_pool"
            ),
            ActionInput::FillTaskLinks => return_owner_evidence!(
                run_fill_task_links_reachability_probe_for_test,
                action_index,
                "fill_task_links"
            ),
            ActionInput::FillTaskLinksLeavingOneReservationSlot => return_owner_evidence!(
                run_fill_task_links_leaving_one_reservation_slot_reachability_probe_for_test,
                action_index,
                "fill_task_links_leaving_one_reservation_slot"
            ),
            ActionInput::FillTombstones => return_owner_evidence!(
                run_fill_tombstones_reachability_probe_for_test,
                action_index,
                "fill_tombstones"
            ),
            ActionInput::AttemptTaskStoreBindUnderGate { .. } => return_owner_evidence!(
                run_attempt_task_store_bind_under_gate_reachability_probe_for_test,
                action_index,
                "attempt_task_store_bind_under_gate"
            ),
            ActionInput::RotateReceiptSegments | ActionInput::ReclaimExpiredEvidence => {
                return Err(ScenarioExecutionError::OwnerUnavailable(
                    ScenarioOwner::RetentionCoordinator,
                ));
            }
            ActionInput::RunDirectLoad { .. } => return_owner_evidence!(
                run_direct_load_reachability_probe_for_test,
                action_index,
                "run_direct_load"
            ),
            ActionInput::RunLazyCancelStorm { .. } => return_owner_evidence!(
                run_lazy_cancel_storm_reachability_probe_for_test,
                action_index,
                "run_lazy_cancel_storm"
            ),
            ActionInput::ReleaseBarrier { .. }
            | ActionInput::WaitForEvent { .. }
            | ActionInput::WaitForEventCount { .. }
            | ActionInput::WaitForOperation { .. }
            | ActionInput::JoinOperation { .. }
            | ActionInput::AdvanceMonotonic { .. }
            | ActionInput::AdvanceEpoch { .. }
            | ActionInput::Restart
            | ActionInput::Checkpoint { .. }
            | ActionInput::Reset
            | ActionInput::PublishListener
            | ActionInput::InjectTaskStoreCapacityInvariantViolationOnce => {
                return Err(ScenarioExecutionError::ControlActionUnavailable);
            }
        }
    }

    Err(ScenarioExecutionError::NoProductionAction)
}

#[derive(Debug)]
enum ScenarioExecutionError {
    InvalidScenario(String),
    ActionIndexOutOfRange(usize),
    OwnerUnavailable(ScenarioOwner),
    ControlActionUnavailable,
    NoProductionAction,
    ProductionProbe(String),
    EvidenceEncode(String),
}

impl fmt::Display for ScenarioExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidScenario(error) => write!(formatter, "invalid closed scenario: {error}"),
            Self::ActionIndexOutOfRange(index) => {
                write!(formatter, "scenario action index {index} exceeds u32")
            }
            Self::OwnerUnavailable(owner) => {
                write!(
                    formatter,
                    "scenario production owner is not implemented: {owner}"
                )
            }
            Self::ControlActionUnavailable => {
                formatter.write_str("scenario control action has no active production session")
            }
            Self::NoProductionAction => {
                formatter.write_str("scenario ended before a production action")
            }
            Self::ProductionProbe(error) => write!(formatter, "production probe failed: {error}"),
            Self::EvidenceEncode(error) => write!(formatter, "evidence encode failed: {error}"),
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum ScenarioOwner {
    TaskProjection,
    ActorLinearization,
    RetentionCoordinator,
}

impl fmt::Display for ScenarioOwner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::TaskProjection => "task-projection",
            Self::ActorLinearization => "actor-linearization",
            Self::RetentionCoordinator => "retention-coordinator",
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ScenarioInput {
    clock: ClockModeInput,
    actions: Vec<ActionInput>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ClockModeInput {
    Fake,
    Wall,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum ActionInput {
    ConfigureValidation {
        reject: bool,
    },
    ConfigureProvider {
        execution_class: ExecutionClassInput,
        terminal: TerminalFixtureInput,
        cooperative_cancel: bool,
        side_effect_marker: bool,
    },
    ConfigureAdmission {
        rejection: Option<WorkspaceAdmissionFailureInput>,
    },
    ConfigurePrepare {
        reject: bool,
    },
    InstallBarrier {
        point: BarrierPointInput,
    },
    ReleaseBarrier {
        point: BarrierPointInput,
    },
    WaitForEvent {
        event: EventKindInput,
    },
    WaitForEventCount {
        event: EventKindInput,
        count: u32,
    },
    WaitForOperation {
        label: String,
        state: OperationStateInput,
    },
    Submit {
        request: RequestCaseInput,
        response_budget_ms: u64,
        disconnect: DisconnectPointInput,
        label: String,
    },
    SendOuterEnvelope {
        envelope: EnvelopeCaseInput,
        label: String,
    },
    Recover {
        key: KeyCaseInput,
        label: String,
    },
    Acknowledge {
        key: KeyCaseInput,
        digest: DigestCaseInput,
        disconnect: AckDisconnectPointInput,
        label: String,
    },
    Cancel {
        key: KeyCaseInput,
        lazy_session: bool,
        label: String,
    },
    CancelTask {
        api: TaskCancelApiInput,
        task: TaskSelectorInput,
        lazy_session: bool,
        label: String,
    },
    SpawnSubmit {
        request: RequestCaseInput,
        response_budget_ms: u64,
        disconnect: DisconnectPointInput,
        label: String,
    },
    SpawnCancel {
        key: KeyCaseInput,
        lazy_session: bool,
        label: String,
    },
    SpawnMarkReservedBegun {
        proof: ActorProofCaseInput,
        label: String,
    },
    SpawnStageBoundHandoffTerminal {
        terminal: TerminalFixtureInput,
        label: String,
    },
    JoinOperation {
        label: String,
    },
    ReadTask {
        api: TaskApiInput,
        label: String,
    },
    AdvanceMonotonic {
        millis: u64,
    },
    AdvanceEpoch {
        millis: u64,
    },
    Crash {
        point: CrashPointInput,
    },
    Restart,
    Checkpoint {
        label: String,
    },
    Reset,
    ProbeProtocol {
        client: ProtocolVersionInput,
        server: ProtocolVersionInput,
        message: ProtocolMessageInput,
        label: String,
    },
    CompareClientServerIdentity,
    InjectPersistedIdentityCollision {
        index: IdentityIndexInput,
    },
    RunCrossStoreCrashWorkload {
        cases: Vec<CrashWorkloadInput>,
    },
    RunTaskRetirementWorkload {
        cases: Vec<TaskRetirementWorkloadInput>,
    },
    SeedReceipt {
        state: SeedReceiptStateInput,
        cancel_requested: bool,
        staged_terminal: Option<TerminalFixtureInput>,
    },
    SeedTask {
        status: TaskStatusInput,
        cancel_requested: bool,
        receipt_link: ReceiptLinkCaseInput,
        identity: IdentityRelationInput,
        version: u64,
    },
    SeedTaskLinkReservation {
        relation: IdentityRelationInput,
    },
    OpenTaskStoreInspectOnly,
    ReconcileStartup,
    PublishListener,
    InvalidateActorProof {
        proof: ActorProofCaseInput,
        point: BarrierPointInput,
        label: String,
    },
    AttemptBoundTaskStart {
        proof: ActorProofCaseInput,
        label: String,
    },
    RunLazyCancelStorm {
        submits: u32,
        cancels: u32,
        per_cancel_deadline_ms: u64,
        label: String,
    },
    FillReceiptPool {
        state: SeedReceiptStateInput,
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
        terminal: TerminalFixtureInput,
        label: String,
    },
    AttemptStagedTerminalAgainstProvisional {
        mismatch: Option<ProvisionalMismatchFieldInput>,
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
        point: StoreFaultPointInput,
    },
}

#[allow(dead_code)]
#[derive(Debug)]
struct StagedScenarioSetup {
    clock: ClockModeInput,
    validation: Option<ValidationSetup>,
    provider: Option<ProviderSetup>,
    admission: Option<AdmissionSetup>,
    prepare: Option<PrepareSetup>,
    barriers: Vec<BarrierPointInput>,
    crash_points: Vec<CrashPointInput>,
}

impl StagedScenarioSetup {
    fn new(clock: ClockModeInput) -> Self {
        Self {
            clock,
            validation: None,
            provider: None,
            admission: None,
            prepare: None,
            barriers: Vec::new(),
            crash_points: Vec::new(),
        }
    }
}

#[allow(dead_code)]
#[derive(Debug)]
struct ValidationSetup {
    reject: bool,
}

#[allow(dead_code)]
#[derive(Debug)]
struct ProviderSetup {
    execution_class: ExecutionClassInput,
    terminal: TerminalFixtureInput,
    cooperative_cancel: bool,
    side_effect_marker: bool,
}

#[allow(dead_code)]
#[derive(Debug)]
struct AdmissionSetup {
    rejection: Option<WorkspaceAdmissionFailureInput>,
}

#[allow(dead_code)]
#[derive(Debug)]
struct PrepareSetup {
    reject: bool,
}

macro_rules! closed_unit_enum {
    ($name:ident { $($variant:ident),+ $(,)? }) => {
        #[allow(dead_code)]
        #[derive(Debug, Clone, Copy, Deserialize)]
        #[serde(rename_all = "snake_case")]
        enum $name {
            $($variant),+
        }
    };
}

closed_unit_enum!(ExecutionClassInput { Direct, KnownLong });
closed_unit_enum!(WorkspaceAdmissionFailureInput {
    Invalid,
    Capacity,
    RegistryFailed,
});
closed_unit_enum!(BarrierPointInput {
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
});
closed_unit_enum!(DisconnectPointInput {
    Never,
    AfterSubmitWrite,
    AfterTerminalCommit,
});
closed_unit_enum!(AckDisconnectPointInput {
    Never,
    AfterTombstoneCommit,
});
closed_unit_enum!(DigestCaseInput {
    ExactTerminal,
    Mismatched,
    TaskTerminal,
    WellFormedCandidate,
});
closed_unit_enum!(EnvelopeCaseInput {
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
});
closed_unit_enum!(IdentityFieldInput {
    InvocationId,
    ReservedTaskId,
    CoreIdentity,
    ToolIdentity,
    NormalizedArgumentsHash,
    RequestScopeHash,
});
closed_unit_enum!(IdentityIndexInput {
    InvocationId,
    ReservedTaskId,
});
closed_unit_enum!(ProtocolVersionInput { V3, V4, V5 });
closed_unit_enum!(TaskTerminalOwnerFixtureInput {
    ReceiptBacked,
    Bound,
    Staged,
});
closed_unit_enum!(CoreIdentitySelectionInput {
    ExactProductionV5,
    ArbitraryCanonical,
});
closed_unit_enum!(FailureProbeReasonInput {
    InvocationFailed,
    ResultTooLarge,
    Interrupted,
    ResumeUnsupported,
    PersistenceFailed,
    OutcomeUncertain,
    TaskCapacity,
    WorkspaceCapacity,
    WorkspaceRegistryFailed,
});
closed_unit_enum!(V5DaemonErrorCodeFixtureInput {
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
});
closed_unit_enum!(StrictSchemaTargetInput {
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
});
closed_unit_enum!(TaskApiInput {
    NativeGet,
    NativeWait,
    CompatibilityGet,
    CompatibilityResult,
});
closed_unit_enum!(TaskCancelApiInput {
    Native,
    Compatibility,
});
closed_unit_enum!(TaskRetirementWorkloadInput {
    RecoveryTerminalBeforeTerminalBound,
    ActiveTaskBoundAbsent,
    BeforePendingIntent,
    AfterPendingIntentBeforeDelete,
    AfterDeleteCommitUncertain,
    AfterDeletedBeforeLedgerFinalize,
    AfterAbsentConfirmedBeforeLedgerFinalize,
    DeleteIdentityMismatch,
});
closed_unit_enum!(CrashPointInput {
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
});
closed_unit_enum!(EntryPathInput {
    PromisedUnbound,
    ReservedActorBound,
    ReservedBegun,
});
closed_unit_enum!(SeedReceiptStateInput {
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
});
closed_unit_enum!(TaskStatusInput {
    Queued,
    Working,
    Completed,
    Failed,
    Cancelled,
});
closed_unit_enum!(IdentityRelationInput { Exact, Foreign });
closed_unit_enum!(ReceiptLinkCaseInput {
    Exact,
    Missing,
    Foreign,
});
closed_unit_enum!(ProvisionalMismatchFieldInput {
    TaskId,
    InvocationId,
    Status,
    Version,
    CancelRequested,
    TaskLinkDigest,
});
closed_unit_enum!(ActorProofCaseInput {
    Missing,
    Foreign,
    Stale,
    Exact,
});
closed_unit_enum!(StoreFaultPointInput {
    AfterTerminalPayloadRenameBeforeDirectorySync,
    AfterTaskCreateRenameBeforeDirectorySync,
});
closed_unit_enum!(OperationStateInput { Blocked, Completed });

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(tag = "terminal", rename_all = "snake_case", deny_unknown_fields)]
enum TerminalFixtureInput {
    Success { payload: String },
    Bytes { count: u64 },
    NearLimitWithMaximumMetadata { canonical_result_bytes: u64 },
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum RequestCaseInput {
    Canonical,
    Fresh(u32),
    SameIdentity,
    Mismatch(IdentityFieldInput),
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum KeyCaseInput {
    Exact,
    ForSubmitLabel(String),
    Unknown,
    Mismatch(IdentityFieldInput),
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum TaskSelectorInput {
    ExactProjected,
    ForReadLabel(String),
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum ProtocolMessageInput {
    Ping,
    Release,
    SubmitWithCoreIdentity {
        selection: CoreIdentitySelectionInput,
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
        code: V5DaemonErrorCodeFixtureInput,
    },
    MalformedV5Schema {
        target: StrictSchemaTargetInput,
    },
    ReceiptPendingOutcome,
    TaskOutcome,
    AcknowledgedOutcome,
    DirectCompletedTerminal,
    DirectSemanticCompletedTerminal,
    DirectCancelledTerminal,
    DirectFailureTerminal {
        reason: FailureProbeReasonInput,
    },
    TaskQueuedProjection,
    TaskWorkingProjection,
    TaskCompletedProjection,
    TaskSemanticCompletedProjection {
        owner: TaskTerminalOwnerFixtureInput,
    },
    TaskCancelledProjection,
    TaskFailureProjection {
        reason: FailureProbeReasonInput,
    },
    StoredInvocationRecord {
        schema_version: u8,
        reason: FailureProbeReasonInput,
    },
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CrashWorkloadInput {
    path: EntryPathInput,
    point: CrashPointInput,
    cancel_before_crash: bool,
    stage_terminal_before_crash: bool,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum EventKindInput {
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

#[cfg(test)]
mod tests {
    use super::*;
    use proc_macro2::{TokenStream, TokenTree};
    use std::collections::BTreeSet;
    use syn::parse::Parser;
    use syn::visit::Visit;

    fn facade_forbidden_authority_references(
        source: &str,
        forbidden: &[&str],
    ) -> Result<BTreeSet<String>, String> {
        let syntax = syn::parse_file(source).map_err(|error| error.to_string())?;
        let mut finder = FacadeAuthorityReferenceFinder {
            forbidden,
            references: BTreeSet::new(),
            unsupported_syntax: BTreeSet::new(),
        };
        finder.visit_file(&syntax);
        if finder.unsupported_syntax.is_empty() {
            Ok(finder.references)
        } else {
            Err(finder
                .unsupported_syntax
                .into_iter()
                .collect::<Vec<_>>()
                .join("; "))
        }
    }

    struct FacadeAuthorityReferenceFinder<'a> {
        forbidden: &'a [&'a str],
        references: BTreeSet<String>,
        unsupported_syntax: BTreeSet<String>,
    }

    impl FacadeAuthorityReferenceFinder<'_> {
        fn scan_text(&mut self, text: &str) {
            for authority in self.forbidden {
                if text.contains(authority) {
                    self.references.insert((*authority).to_string());
                }
            }
        }

        fn scan_literal(&mut self, literal: &syn::Lit) -> String {
            let decoded = match literal {
                syn::Lit::Str(value) => value.value(),
                syn::Lit::ByteStr(value) => {
                    String::from_utf8_lossy(value.value().as_slice()).into_owned()
                }
                syn::Lit::CStr(value) => {
                    String::from_utf8_lossy(value.value().to_bytes()).into_owned()
                }
                syn::Lit::Byte(value) => char::from(value.value()).to_string(),
                syn::Lit::Char(value) => value.value().to_string(),
                syn::Lit::Int(value) => value.base10_digits().to_string(),
                syn::Lit::Float(value) => value.base10_digits().to_string(),
                syn::Lit::Bool(value) => value.value.to_string(),
                syn::Lit::Verbatim(value) => {
                    self.unsupported_syntax.insert(format!(
                        "unsupported literal in facade authority guard: `{value}`"
                    ));
                    value.to_string()
                }
                _ => {
                    self.unsupported_syntax
                        .insert("unsupported future literal in facade authority guard".to_string());
                    String::new()
                }
            };
            self.scan_text(&decoded);
            decoded
        }

        fn scan_concat_tokens(&mut self, tokens: TokenStream) -> String {
            let expressions =
                match syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated
                    .parse2(tokens)
                {
                    Ok(expressions) => expressions,
                    Err(error) => {
                        self.unsupported_syntax.insert(format!(
                            "unsupported concat! syntax in facade authority guard: {error}"
                        ));
                        return String::new();
                    }
                };
            let mut concatenated = String::new();
            for expression in expressions {
                let fragment = match expression {
                    syn::Expr::Lit(expression) => self.scan_literal(&expression.lit),
                    syn::Expr::Group(expression) => {
                        self.scan_static_string_expression(&expression.expr)
                    }
                    syn::Expr::Paren(expression) => {
                        self.scan_static_string_expression(&expression.expr)
                    }
                    syn::Expr::Macro(expression)
                        if expression
                            .mac
                            .path
                            .segments
                            .last()
                            .is_some_and(|segment| segment.ident == "concat") =>
                    {
                        self.scan_concat_tokens(expression.mac.tokens)
                    }
                    _ => {
                        self.unsupported_syntax.insert(
                            "dynamic concat! expression in facade authority guard".to_string(),
                        );
                        String::new()
                    }
                };
                concatenated.push_str(&fragment);
            }
            self.scan_text(&concatenated);
            concatenated
        }

        fn scan_static_string_expression(&mut self, expression: &syn::Expr) -> String {
            match expression {
                syn::Expr::Lit(expression) => self.scan_literal(&expression.lit),
                syn::Expr::Group(expression) => {
                    self.scan_static_string_expression(&expression.expr)
                }
                syn::Expr::Paren(expression) => {
                    self.scan_static_string_expression(&expression.expr)
                }
                syn::Expr::Macro(expression)
                    if expression
                        .mac
                        .path
                        .segments
                        .last()
                        .is_some_and(|segment| segment.ident == "concat") =>
                {
                    self.scan_concat_tokens(expression.mac.tokens.clone())
                }
                _ => {
                    self.unsupported_syntax
                        .insert("dynamic string expression in facade authority guard".to_string());
                    String::new()
                }
            }
        }

        fn format_static_segments(format: &str) -> Vec<String> {
            let mut segments = vec![String::new()];
            let mut characters = format.chars().peekable();
            while let Some(character) = characters.next() {
                match character {
                    '{' if characters.peek() == Some(&'{') => {
                        characters.next();
                        segments.last_mut().expect("initial segment").push('{');
                    }
                    '}' if characters.peek() == Some(&'}') => {
                        characters.next();
                        segments.last_mut().expect("initial segment").push('}');
                    }
                    '{' => {
                        let mut closed = false;
                        for nested in characters.by_ref() {
                            if nested == '}' {
                                closed = true;
                                break;
                            }
                        }
                        if closed {
                            segments.push(String::new());
                        } else {
                            segments.last_mut().expect("initial segment").push('{');
                        }
                    }
                    character => segments
                        .last_mut()
                        .expect("initial segment")
                        .push(character),
                }
            }
            segments
        }

        fn format_has_explicit_placeholder_selector(format: &str) -> bool {
            let mut characters = format.chars().peekable();
            while let Some(character) = characters.next() {
                if character != '{' {
                    continue;
                }
                if characters.peek() == Some(&'{') {
                    characters.next();
                    continue;
                }
                let selector = characters
                    .by_ref()
                    .take_while(|character| *character != '}')
                    .take_while(|character| *character != ':')
                    .collect::<String>();
                if !selector.is_empty() {
                    return true;
                }
            }
            false
        }

        fn formatting_argument_literal(&mut self, expression: &syn::Expr) -> Option<String> {
            match expression {
                syn::Expr::Lit(expression) => Some(self.scan_literal(&expression.lit)),
                syn::Expr::Assign(expression) => {
                    self.formatting_argument_literal(&expression.right)
                }
                syn::Expr::Group(expression) => self.formatting_argument_literal(&expression.expr),
                syn::Expr::Paren(expression) => self.formatting_argument_literal(&expression.expr),
                _ => None,
            }
        }

        fn scan_formatting_macro(&mut self, tokens: TokenStream, has_destination: bool) {
            let expressions =
                match syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated
                    .parse2(tokens)
                {
                    Ok(expressions) => expressions.into_iter().collect::<Vec<_>>(),
                    Err(error) => {
                        self.unsupported_syntax.insert(format!(
                            "unsupported formatting macro in facade authority guard: {error}"
                        ));
                        return;
                    }
                };
            let format_index = usize::from(has_destination);
            let Some(syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(format),
                ..
            })) = expressions.get(format_index)
            else {
                self.unsupported_syntax
                    .insert("dynamic formatting template in facade authority guard".to_string());
                return;
            };
            if expressions.len() > format_index + 1
                && Self::format_has_explicit_placeholder_selector(&format.value())
            {
                self.unsupported_syntax.insert(
                    "indexed or named formatting arguments in facade authority guard".to_string(),
                );
                return;
            }
            let segments = Self::format_static_segments(&format.value());
            let arguments = expressions
                .iter()
                .skip(format_index + 1)
                .map(|expression| self.formatting_argument_literal(expression))
                .collect::<Vec<_>>();
            let mut rendered = segments.first().cloned().unwrap_or_default();
            for (index, segment) in segments.iter().skip(1).enumerate() {
                if let Some(Some(argument)) = arguments.get(index) {
                    rendered.push_str(argument);
                }
                rendered.push_str(segment);
            }
            for argument in arguments
                .iter()
                .skip(segments.len().saturating_sub(1))
                .flatten()
            {
                rendered.push_str(argument);
            }
            self.scan_text(&rendered);
        }

        fn scan_macro_tokens(&mut self, tokens: TokenStream) {
            let tokens = tokens.into_iter().collect::<Vec<_>>();
            let mut index = 0;
            while index < tokens.len() {
                match &tokens[index] {
                    TokenTree::Group(group) => {
                        self.scan_macro_tokens(group.stream());
                    }
                    TokenTree::Ident(identifier) => {
                        let identifier = identifier.to_string();
                        if matches!(
                            identifier.as_str(),
                            "concat"
                                | "format"
                                | "format_args"
                                | "write"
                                | "writeln"
                                | "include"
                                | "include_str"
                                | "include_bytes"
                        )
                            && tokens.get(index + 1).is_some_and(
                                |token| matches!(token, TokenTree::Punct(punct) if punct.as_char() == '!'),
                            )
                        {
                            self.unsupported_syntax.insert(format!(
                                "nested or dynamic `{identifier}!` in facade authority guard"
                            ));
                        }
                        self.scan_text(&identifier);
                    }
                    TokenTree::Literal(literal) => {
                        match syn::parse_str::<syn::Lit>(&literal.to_string()) {
                            Ok(literal) => {
                                self.scan_literal(&literal);
                            }
                            Err(error) => {
                                self.unsupported_syntax.insert(format!(
                                    "unsupported macro literal `{literal}`: {error}"
                                ));
                            }
                        }
                    }
                    TokenTree::Punct(_) => {}
                }
                index += 1;
            }
        }

        fn attributes_have_exact_cfg_test(attributes: &[syn::Attribute]) -> bool {
            attributes.iter().any(|attribute| {
                attribute.path().is_ident("cfg")
                    && attribute
                        .parse_args::<syn::Path>()
                        .is_ok_and(|path| path.is_ident("test"))
            })
        }

        fn item_has_exact_cfg_test(item: &syn::Item) -> bool {
            let attributes: &[syn::Attribute] = match item {
                syn::Item::Const(item) => &item.attrs,
                syn::Item::Enum(item) => &item.attrs,
                syn::Item::ExternCrate(item) => &item.attrs,
                syn::Item::Fn(item) => &item.attrs,
                syn::Item::ForeignMod(item) => &item.attrs,
                syn::Item::Impl(item) => &item.attrs,
                syn::Item::Macro(item) => &item.attrs,
                syn::Item::Mod(item) => &item.attrs,
                syn::Item::Static(item) => &item.attrs,
                syn::Item::Struct(item) => &item.attrs,
                syn::Item::Trait(item) => &item.attrs,
                syn::Item::TraitAlias(item) => &item.attrs,
                syn::Item::Type(item) => &item.attrs,
                syn::Item::Union(item) => &item.attrs,
                syn::Item::Use(item) => &item.attrs,
                syn::Item::Verbatim(_) => &[],
                _ => &[],
            };
            Self::attributes_have_exact_cfg_test(attributes)
        }

        fn impl_item_has_exact_cfg_test(item: &syn::ImplItem) -> bool {
            let attributes: &[syn::Attribute] = match item {
                syn::ImplItem::Const(item) => &item.attrs,
                syn::ImplItem::Fn(item) => &item.attrs,
                syn::ImplItem::Type(item) => &item.attrs,
                syn::ImplItem::Macro(item) => &item.attrs,
                syn::ImplItem::Verbatim(_) => &[],
                _ => &[],
            };
            Self::attributes_have_exact_cfg_test(attributes)
        }

        fn trait_item_has_exact_cfg_test(item: &syn::TraitItem) -> bool {
            let attributes: &[syn::Attribute] = match item {
                syn::TraitItem::Const(item) => &item.attrs,
                syn::TraitItem::Fn(item) => &item.attrs,
                syn::TraitItem::Type(item) => &item.attrs,
                syn::TraitItem::Macro(item) => &item.attrs,
                syn::TraitItem::Verbatim(_) => &[],
                _ => &[],
            };
            Self::attributes_have_exact_cfg_test(attributes)
        }

        fn foreign_item_has_exact_cfg_test(item: &syn::ForeignItem) -> bool {
            let attributes: &[syn::Attribute] = match item {
                syn::ForeignItem::Fn(item) => &item.attrs,
                syn::ForeignItem::Static(item) => &item.attrs,
                syn::ForeignItem::Type(item) => &item.attrs,
                syn::ForeignItem::Macro(item) => &item.attrs,
                syn::ForeignItem::Verbatim(_) => &[],
                _ => &[],
            };
            Self::attributes_have_exact_cfg_test(attributes)
        }
    }

    impl<'ast> Visit<'ast> for FacadeAuthorityReferenceFinder<'_> {
        fn visit_item(&mut self, item: &'ast syn::Item) {
            if Self::item_has_exact_cfg_test(item) {
                return;
            }
            if let syn::Item::Verbatim(tokens) = item {
                self.unsupported_syntax.insert(format!(
                    "unsupported verbatim item in facade authority guard: `{tokens}`"
                ));
                return;
            }
            syn::visit::visit_item(self, item);
        }

        fn visit_impl_item(&mut self, item: &'ast syn::ImplItem) {
            if Self::impl_item_has_exact_cfg_test(item) {
                return;
            }
            syn::visit::visit_impl_item(self, item);
        }

        fn visit_trait_item(&mut self, item: &'ast syn::TraitItem) {
            if Self::trait_item_has_exact_cfg_test(item) {
                return;
            }
            syn::visit::visit_trait_item(self, item);
        }

        fn visit_foreign_item(&mut self, item: &'ast syn::ForeignItem) {
            if Self::foreign_item_has_exact_cfg_test(item) {
                return;
            }
            syn::visit::visit_foreign_item(self, item);
        }

        fn visit_ident(&mut self, identifier: &'ast proc_macro2::Ident) {
            self.scan_text(&identifier.to_string());
        }

        fn visit_lit(&mut self, literal: &'ast syn::Lit) {
            self.scan_literal(literal);
        }

        fn visit_macro(&mut self, macro_: &'ast syn::Macro) {
            syn::visit::visit_path(self, &macro_.path);
            match macro_.path.segments.last().map(|segment| &segment.ident) {
                Some(identifier) if identifier == "concat" => {
                    self.scan_concat_tokens(macro_.tokens.clone());
                }
                Some(identifier)
                    if matches!(
                        identifier.to_string().as_str(),
                        "format" | "format_args" | "write" | "writeln"
                    ) =>
                {
                    self.scan_formatting_macro(
                        macro_.tokens.clone(),
                        matches!(identifier.to_string().as_str(), "write" | "writeln"),
                    );
                }
                Some(identifier)
                    if matches!(
                        identifier.to_string().as_str(),
                        "include" | "include_str" | "include_bytes"
                    ) =>
                {
                    self.unsupported_syntax.insert(format!(
                        "external `{identifier}!` content in facade authority guard"
                    ));
                }
                _ => {
                    self.scan_macro_tokens(macro_.tokens.clone());
                }
            }
        }

        fn visit_expr_method_call(&mut self, expression: &'ast syn::ExprMethodCall) {
            if expression.method == "concat" {
                self.unsupported_syntax
                    .insert("dynamic `.concat()` in facade authority guard".to_string());
            }
            syn::visit::visit_expr_method_call(self, expression);
        }
    }

    #[test]
    fn authority_wrappers_match_the_frozen_application_vectors() {
        assert_eq!(
            request_scope_hash_for_test("workspace-a"),
            "9f7a5a77bb6eb469cd20147a9aeee9d9769a8372f587bd89635d15684ee02b39"
        );
        assert_eq!(
            receipt_key_digest_for_test(
                "123e4567-e89b-42d3-a456-426614174000",
                "123e4567-e89b-42d3-b456-426614174001",
                &"00".repeat(32),
                "unica.view",
                &"11".repeat(32),
                "9f7a5a77bb6eb469cd20147a9aeee9d9769a8372f587bd89635d15684ee02b39",
            ),
            "9d8f104e7dfb2f4827a24d4b41aefe6c6704bf31bd3df191d84f9893071db549"
        );
        assert_eq!(
            task_link_digest_for_test(
                &"0".repeat(64),
                "11111111-1111-4111-8111-111111111111",
                "22222222-2222-4222-8222-222222222222",
                &"a".repeat(64),
            ),
            "4c73d08219973c72e759a9f85e156fa42c9d8e61a56e704b70d1c7c042b73da0"
        );
        assert_eq!(
            canonical_v5_terminal_for_test(r#"{"status":"cancelled"}"#),
            (
                br#"{"status":"cancelled"}"#.to_vec(),
                "f2d0423d2613a0d09397b750542e4542f7653d78ebd5e0448f1326d09145d9ae".to_string(),
            )
        );
    }

    #[test]
    fn scenario_wire_is_closed_at_the_scenario_and_action_levels() {
        let unknown_scenario_field =
            r#"{"clock":"fake","actions":[],"expectedMissing":"forbidden"}"#;
        assert!(execute_scenario_json(unknown_scenario_field).is_err());

        let unknown_action_field = r#"{"clock":"fake","actions":[{"action":"configure_validation","reject":false,"expectedCode":"forbidden"}]}"#;
        assert!(execute_scenario_json(unknown_action_field).is_err());
    }

    #[test]
    fn crash_is_a_staged_control_and_does_not_mint_receipt_transition_evidence() {
        let scenario = r#"{
            "clock":"fake",
            "actions":[
                {"action":"crash","point":"after_side_effect_before_terminal"},
                {"action":"send_outer_envelope","envelope":"missing_invocation_id","label":"strict"}
            ]
        }"#;

        let encoded = execute_scenario_json(scenario)
            .expect("the next real production operation owns the missing-boundary evidence");
        let envelope: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        assert_eq!(
            envelope["payload"]["actionIndex"], 1,
            "crash configures the next operation instead of terminating the scenario"
        );
        assert_eq!(
            envelope["payload"]["reachedBoundary"],
            "strict_envelope_validation"
        );
    }

    #[test]
    fn every_malformed_envelope_case_is_routed_individually_through_the_production_pipeline() {
        let cases = [
            "missing_invocation_id",
            "noncanonical_invocation_id",
            "missing_reserved_task_id",
            "noncanonical_reserved_task_id",
            "unknown_tool",
            "unknown_field",
            "malformed_arguments",
            "oversized_arguments",
            "response_budget_above_maximum",
            "empty_workspace_hint",
            "workspace_hint_with_control",
            "malformed_workspace_hint",
            "oversized_workspace_hint",
        ];
        let mut fingerprints = std::collections::BTreeSet::new();

        for envelope_case in cases {
            let scenario = format!(
                r#"{{"clock":"fake","actions":[{{"action":"send_outer_envelope","envelope":"{envelope_case}","label":"strict"}}]}}"#
            );
            let encoded = execute_scenario_json(&scenario)
                .unwrap_or_else(|error| panic!("{envelope_case} reaches production: {error}"));
            let envelope: serde_json::Value = serde_json::from_str(&encoded).unwrap();
            let payload = &envelope["payload"];
            assert_eq!(payload["actionIndex"], 0, "{envelope_case}");
            assert_eq!(
                payload["reachedBoundary"], "strict_envelope_validation",
                "{envelope_case}"
            );
            assert_eq!(
                payload["evidence"]["code"], "strict_envelope_observation_unavailable",
                "{envelope_case}"
            );
            assert!(
                fingerprints.insert(
                    payload["evidence"]["fingerprint"]
                        .as_str()
                        .expect("bounded production fingerprint")
                        .to_string()
                ),
                "{envelope_case} must retain distinct rejected bytes/reason evidence"
            );
        }

        assert_eq!(fingerprints.len(), cases.len());
    }

    #[test]
    fn facade_authority_guard_ignores_only_exact_cfg_test_subtrees() {
        let source = r#"
            fn production_entry() {}

            #[cfg(test)]
            mod tests {
                const EXPECTED_CODE: &str = "writer_path_unavailable";
                fn assertion_mentions_type() {
                    let _ = "ProductionBoundary";
                }
            }
        "#;

        assert!(facade_forbidden_authority_references(
            source,
            &["writer_path_unavailable", "ProductionBoundary"],
        )
        .expect("synthetic facade source parses")
        .is_empty());
    }

    #[test]
    fn facade_authority_guard_rejects_production_string_literals() {
        let source = r#"
            const FORGED_CODE: &str = "writer_path_unavailable";
        "#;

        assert_eq!(
            facade_forbidden_authority_references(source, &["writer_path_unavailable"])
                .expect("synthetic facade source parses"),
            BTreeSet::from(["writer_path_unavailable".to_string()])
        );
    }

    #[test]
    fn facade_authority_guard_audits_mixed_cfg_subtrees() {
        let source = r#"
            #[cfg(any(test, feature = "receipt-ledger-test-support"))]
            const FORGED_CODE: &str = "writer_path_unavailable";
        "#;

        assert_eq!(
            facade_forbidden_authority_references(source, &["writer_path_unavailable"])
                .expect("synthetic facade source parses"),
            BTreeSet::from(["writer_path_unavailable".to_string()])
        );
    }

    #[test]
    fn facade_authority_guard_reconstructs_split_concat_literals() {
        let source = r#"
            const FORGED_CODE: &str = concat!("writer_path", "_unavailable");
        "#;

        assert_eq!(
            facade_forbidden_authority_references(source, &["writer_path_unavailable"])
                .expect("synthetic facade source parses"),
            BTreeSet::from(["writer_path_unavailable".to_string()])
        );
    }

    #[test]
    fn facade_authority_guard_reconstructs_nested_concat_literals() {
        let source = r#"
            const FORGED_CODE: &str =
                concat!("writer_", concat!("path", "_unavailable"));
        "#;

        assert_eq!(
            facade_forbidden_authority_references(source, &["writer_path_unavailable"])
                .expect("synthetic facade source parses"),
            BTreeSet::from(["writer_path_unavailable".to_string()])
        );
    }

    #[test]
    fn facade_authority_guard_fails_closed_on_dynamic_string_construction() {
        for source in [
            r#"fn forged() { let _ = format!("writer_path{}", "_unavailable"); }"#,
            r#"fn forged() { let _ = format!("{1}{0}", "_unavailable", "writer_path"); }"#,
            r#"fn forged() { let _ = ["writer_path", "_unavailable"].concat(); }"#,
            r#"fn forged(out: &mut String) { let _ = write!(out, "writer_path{}", "_unavailable"); }"#,
            r#"include!("forged-evidence.rs");"#,
            r#"fn forged() { let _ = include_str!("forged-evidence.txt"); }"#,
        ] {
            match facade_forbidden_authority_references(source, &["writer_path_unavailable"]) {
                Err(_) => {}
                Ok(references) => assert!(
                    !references.is_empty(),
                    "dynamic production string construction must fail closed: {source}"
                ),
            }
        }
    }

    #[test]
    fn facade_authority_guard_does_not_concatenate_unrelated_macro_literals() {
        let source = r#"
            fn classify(value: &str) -> bool {
                matches!(value, "writer_path" | "_unavailable")
            }
        "#;

        assert!(
            facade_forbidden_authority_references(source, &["writer_path_unavailable"])
                .expect("ordinary macro literals are audited independently")
                .is_empty()
        );
    }

    #[test]
    fn facade_authority_guard_ignores_exact_cfg_test_associated_items() {
        let source = r#"
            struct ProductionOwner;
            impl ProductionOwner {
                #[cfg(test)]
                const EXPECTED_CODE: &str = "writer_path_unavailable";
            }
        "#;

        assert!(
            facade_forbidden_authority_references(source, &["writer_path_unavailable"])
                .expect("synthetic facade source parses")
                .is_empty()
        );
    }

    #[test]
    fn facade_source_keeps_the_exact_abi_and_has_no_production_authority() {
        let source = include_str!("receipt_ledger_test_support.rs");
        let syntax = syn::parse_file(source).expect("facade source parses");
        let public_functions = syntax
            .items
            .iter()
            .filter_map(|item| match item {
                syn::Item::Fn(function) if matches!(function.vis, syn::Visibility::Public(_)) => {
                    Some(function.sig.ident.to_string())
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            public_functions,
            [
                "execute_scenario_json",
                "request_scope_hash_for_test",
                "receipt_key_digest_for_test",
                "task_link_digest_for_test",
                "canonical_v5_terminal_for_test",
            ]
        );

        let forbidden = [
            concat!("sha", "2"),
            concat!("ReceiptLedger", "Store"),
            concat!("Production", "Boundary"),
            concat!("receipt_row", "_absent"),
            concat!("protocol_behavior", "_unavailable"),
            concat!("strict_envelope_observation", "_unavailable"),
            concat!("writer_path", "_unavailable"),
            concat!("task_projection", "_unavailable"),
            concat!("receipt_transition", "_unavailable"),
            concat!("capacity_latch", "_unavailable"),
            concat!("receipt_identity", "_unavailable"),
            concat!("cross_store_intent", "_unavailable"),
            concat!("v5_receipt_runtime", "_entered"),
            concat!("protocol_frame", "_read"),
            concat!("ReachedProduction", "Boundary"),
            concat!("Facade", "Envelope"),
            concat!("production_missing", "_transition"),
        ];
        let references = facade_forbidden_authority_references(source, &forbidden)
            .unwrap_or_else(|error| panic!("facade authority guard failed closed: {error}"));
        assert!(
            references.is_empty(),
            "facade source contains forbidden production authorities: {references:?}"
        );
    }
}
