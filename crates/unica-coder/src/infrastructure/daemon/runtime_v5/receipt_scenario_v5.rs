pub(super) mod scenario_hooks;
pub(super) mod scenario_probes;

use self::scenario_hooks::{
    acknowledge_direct_for_scenario, inject_receipt_identity_collision_for_scenario,
    open_receipt_actor_for_scenario, seed_receipt_backed_task_terminal_for_scenario,
    seed_receipt_tombstones_for_scenario, stage_bound_handoff_terminal_for_scenario, ScenarioHooks,
    V5ReceiptRuntimeEvent, V5ReceiptRuntimeListenerState, V5ReceiptRuntimeTelemetry,
};
use super::hooks::{
    V5AdmissionRejection, V5PausePoint, V5ReceiptRuntimeEventKind, V5StoreFaultPoint,
};
use super::{daemon_error_code, run_daemon_configured_until, V5ReceiptRuntime, V5TaskProjection};
use crate::application::invocation::normalized_arguments_hash;
use crate::application::invocation_store::EpochMillisClock;
use crate::application::invocation_store::SystemEpochMillisClock;
use crate::application::invocation_store_v5::{
    InvocationStoreV5, V5SafeFailureReason, V5StoredInvocationRecord,
    V5StoredInvocationSchemaVersion, V5StoredTask, V5TaskRetirement, V5TaskStoreError,
};
use crate::application::operation_descriptors::{ExecutionClass, KnownLongReason};
use crate::application::ports::Clock;
use crate::application::receipt_ledger::{
    canonical_v5_terminal, receipt_key_digest, request_scope_hash, task_link_digest,
    AcknowledgedTombstoneReceipt, CoreIdentityDigest, HandoffTerminalStage,
    OriginalCutoffDescriptor, ProvenTaskLinkCapacity, ReceiptKey, ReceiptKeyDigest,
    ReceiptLedgerError, ReceiptState, ReceiptTaskProjection, ReceiptTerminalOutcome,
    RequestIdentity, ReserveOutcome, ReservedPhase, TaskBoundReceipt, TaskHandoffActorBoundReceipt,
    TaskLinkIdentity, TaskLinkReference, TaskRetirementPendingReceipt, TaskTerminalBoundReceipt,
    TaskTerminalReceiptBackedReceipt, TerminalDigest, V5CanonicalTerminal, V5ToolIdentity,
    DIRECT_TERMINAL_RETENTION_MS,
};
use crate::application::receipt_ledger_actor::ReceiptLedgerActor;
use crate::domain::cancellation::CancellationToken;
use crate::domain::invocation::{
    DomainResult, InvocationFailure, InvocationId, NormalizedArgumentsHash, SafeIdentityHash,
    TaskId,
};
use crate::infrastructure::daemon::client_v5::{V5DaemonProcessOwner, V5RawHandshake};
use crate::infrastructure::daemon::identity::{CoreIdentity, DaemonStateDirectory};
use crate::infrastructure::daemon::protocol_v5::{
    decode_v5_request_frame, decode_v5_server_response, strict_envelope_case_frame,
    StrictV5EnvelopeCase, V5AcknowledgedReceipt, V5ClientRequest, V5DaemonErrorCode,
    V5DaemonTaskSnapshot, V5InvocationPhase, V5InvocationRequest, V5InvocationResponse,
    V5PendingDirectReceipt, V5ServerResponse, DAEMON_PROTOCOL_VERSION, MAX_V5_RESPONSE_LINE_BYTES,
};
use crate::infrastructure::daemon::server::{
    ActorBoundExecution, ActorBoundInvocation, CanonicalInvocationService, DaemonServerConfig,
};
use crate::infrastructure::daemon::terminal_codec_v5::encode_strict_v5_response_jsonl;
use crate::infrastructure::receipt_ledger::ReceiptBackedTaskTerminalSeed;
use crate::infrastructure::task_lifecycle_link_store_v5::{
    TaskLifecycleLinkCatalogEntry, TaskLifecycleLinkRecord, TaskLifecycleLinkStoreError,
    TaskLifecycleLinkStoreV5,
};
use crate::infrastructure::task_store_v5::{FileInvocationStoreV5, PublicationFailure};
use flate2::{write::GzEncoder, Compression};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Write as _;
use std::path::Path;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::thread;
use std::time::{Duration, Instant};
use uuid::Uuid;

const SCENARIO_INITIAL_EPOCH_MS: u64 = 1;
const SCENARIO_IDLE_GRACE: Duration = Duration::from_secs(120);
const SCENARIO_OPERATION_TIMEOUT: Duration = Duration::from_secs(5);
pub(super) const SCENARIO_BULK_OPERATION_TIMEOUT: Duration = Duration::from_secs(40);
const SCENARIO_BULK_SNAPSHOT_TIMEOUT: Duration = SCENARIO_BULK_OPERATION_TIMEOUT;
const SCENARIO_ENDPOINT_STARTUP_TIMEOUT: Duration = Duration::from_secs(35);
const SCENARIO_TASK_TTL_MS: u64 = 3_600_000;
const SCENARIO_TASK_POLL_INTERVAL_MS: u64 = 100;

fn read_bound_task_without_startup(
    state_root: &Path,
    identity: &CoreIdentity,
    clock: Arc<ScenarioEpochClock>,
    task_id: TaskId,
) -> Result<Option<V5ServerResponse>, String> {
    let state = DaemonStateDirectory::open(state_root, identity)?;
    let root = state.create_private_retained_subdirectory("tasks")?;
    let deadline = crate::domain::code_intelligence::ProviderDeadline::new(
        Instant::now() + SCENARIO_OPERATION_TIMEOUT,
    );
    let (store, _) =
        FileInvocationStoreV5::open_retained_directory_inspect_only(root, clock, deadline)
            .map_err(|error| format!("open inspect-only TaskStore read owner: {error}"))?;
    match store.get(task_id, deadline) {
        Ok(record) => Ok(Some(V5ServerResponse::Task {
            snapshot: super::task_store_snapshot(&record),
        })),
        Err(V5TaskStoreError::NotFound { .. }) => Ok(None),
        Err(error) => Err(format!("read inspect-only TaskStore owner: {error}")),
    }
}

fn read_promised_task_from_actor(
    actor: &ReceiptLedgerActor,
    task_id: TaskId,
) -> Result<Option<V5ServerResponse>, String> {
    let state = match actor.resolve_task(task_id, Instant::now() + SCENARIO_OPERATION_TIMEOUT) {
        Ok(state) => state,
        Err(ReceiptLedgerError::ReceiptNotFound) => return Ok(None),
        Err(error) => return Err(format!("resolve receipt-backed scenario Task: {error}")),
    };
    let snapshot = match self::scenario_probes::receipt_state_task_snapshot_for_test(state) {
        Ok(snapshot) => snapshot,
        Err(ReceiptLedgerError::ReceiptRowPresentUnsupported) => return Ok(None),
        Err(error) => return Err(format!("project receipt-backed scenario Task: {error}")),
    };
    Ok(Some(V5ServerResponse::Task { snapshot }))
}

fn recover_from_live_daemon(
    state_root: &Path,
    identity: &CoreIdentity,
    key: ReceiptKey,
) -> Result<V5ServerResponse, String> {
    let mut owner = V5DaemonProcessOwner::connect_or_spawn(
        state_root,
        identity.clone(),
        std::path::PathBuf::from("unused-existing-v5-scenario-endpoint"),
        SCENARIO_IDLE_GRACE,
    )?;
    owner.recover_invocation_receipt(key)
}

fn read_task_from_live_daemon(
    state_root: &Path,
    identity: &CoreIdentity,
    task_id: TaskId,
    api: ScenarioTaskApi,
) -> Result<V5ServerResponse, String> {
    let mut owner = V5DaemonProcessOwner::connect_or_spawn(
        state_root,
        identity.clone(),
        std::path::PathBuf::from("unused-existing-v5-scenario-endpoint"),
        SCENARIO_IDLE_GRACE,
    )?;
    match api {
        ScenarioTaskApi::NativeWait => owner.wait_task(task_id, SCENARIO_TASK_POLL_INTERVAL_MS),
        ScenarioTaskApi::NativeGet
        | ScenarioTaskApi::CompatibilityGet
        | ScenarioTaskApi::CompatibilityResult => owner.get_task(task_id),
    }
}

struct ScenarioStateRoot {
    _directory: tempfile::TempDir,
    path: std::path::PathBuf,
}

struct ScenarioWorkspace {
    _directory: tempfile::TempDir,
    hint: String,
}

impl ScenarioWorkspace {
    fn new() -> Result<Self, String> {
        let directory = tempfile::tempdir()
            .map_err(|error| format!("create protocol-v5 scenario workspace: {error}"))?;
        let source = directory.path().join("src");
        std::fs::create_dir_all(&source)
            .map_err(|error| format!("create protocol-v5 scenario source: {error}"))?;
        std::fs::write(
            directory.path().join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .map_err(|error| format!("write protocol-v5 scenario project: {error}"))?;
        std::fs::write(
            source.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Scenario</Name></Properties><ChildObjects/></Configuration></MetaDataObject>"#,
        )
        .map_err(|error| format!("write protocol-v5 scenario configuration: {error}"))?;
        let hint = std::fs::canonicalize(directory.path())
            .map_err(|error| format!("canonicalize protocol-v5 scenario workspace: {error}"))?
            .to_string_lossy()
            .into_owned();
        Ok(Self {
            _directory: directory,
            hint,
        })
    }

    fn hint(&self) -> &str {
        &self.hint
    }
}

impl ScenarioStateRoot {
    fn new() -> Result<Self, String> {
        let directory = tempfile::tempdir()
            .map_err(|error| format!("create protocol-v5 receipt scenario state root: {error}"))?;
        let path = std::fs::canonicalize(directory.path()).map_err(|error| {
            format!("canonicalize protocol-v5 receipt scenario state root: {error}")
        })?;
        Ok(Self {
            _directory: directory,
            path,
        })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

fn seed_receipt_backed_task_terminal(
    state_root: &Path,
    identity: &CoreIdentity,
    clock: &ScenarioEpochClock,
    key: ReceiptKey,
    fixture: ScenarioTerminalFixture,
    cancel_requested: bool,
) -> Result<(), String> {
    let ScenarioTerminalFixture::Success { payload } = fixture else {
        return Err("receipt-backed Task seed currently requires a success fixture".to_owned());
    };
    let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Completed {
        result: Box::new(crate::domain::invocation::DomainResult::success(payload)),
    })
    .map_err(|error| format!("construct receipt-backed Task terminal fixture: {error}"))?;
    let epoch_ms = clock.now_epoch_millis();
    let task = ReceiptTaskProjection::new(
        key.reserved_task_id(),
        key.invocation_id(),
        epoch_ms,
        epoch_ms,
        SCENARIO_TASK_TTL_MS,
        SCENARIO_TASK_POLL_INTERVAL_MS,
        1,
    )
    .map_err(|error| format!("construct receipt-backed Task projection: {error}"))?;
    let state = DaemonStateDirectory::open(state_root, identity)?;
    let receipts = state.create_private_retained_subdirectory("receipts")?;
    seed_receipt_backed_task_terminal_for_scenario(
        receipts,
        ReceiptBackedTaskTerminalSeed::new(
            key,
            OriginalCutoffDescriptor::new(epoch_ms, 7_000)
                .map_err(|error| format!("construct Task fixture cutoff: {error}"))?,
            task,
            epoch_ms,
            terminal,
            cancel_requested,
        ),
        Instant::now() + SCENARIO_OPERATION_TIMEOUT,
    )?;
    Ok(())
}

fn seed_receipt_state(
    state_root: &Path,
    identity: &CoreIdentity,
    clock: &Arc<ScenarioEpochClock>,
    key: ReceiptKey,
    seed_state: ScenarioSeedReceiptState,
    cancel_requested: bool,
    staged_terminal: Option<ScenarioTerminalFixture>,
) -> Result<bool, String> {
    if matches!(
        seed_state,
        ScenarioSeedReceiptState::TaskTerminalReceiptBacked
    ) {
        let Some(terminal) = staged_terminal else {
            return Ok(false);
        };
        seed_receipt_backed_task_terminal(
            state_root,
            identity,
            clock,
            key,
            terminal,
            cancel_requested,
        )?;
        return Ok(true);
    }
    let seeds_direct_terminal = matches!(
        seed_state,
        ScenarioSeedReceiptState::DirectTerminalUnacked
            | ScenarioSeedReceiptState::AcknowledgedTombstone
    );
    let seeds_staged_handoff = matches!(
        seed_state,
        ScenarioSeedReceiptState::TaskHandoffActorBoundNotBegun
            | ScenarioSeedReceiptState::TaskHandoffActorBoundBegun
    );
    if staged_terminal.is_some() && !seeds_direct_terminal && !seeds_staged_handoff {
        return Ok(false);
    }

    let state = DaemonStateDirectory::open(state_root, identity)?;
    let config = scenario_server_config_with_clock(state_root, identity, None, clock);
    let runtime = V5ReceiptRuntime::open_with_epoch_clock(&state, &config, clock.clone())?;
    let deadline = Instant::now() + SCENARIO_OPERATION_TIMEOUT;
    let epoch_ms = clock.now_epoch_millis();
    let cutoff = OriginalCutoffDescriptor::new(epoch_ms, 7_000)
        .map_err(|error| format!("construct seeded receipt cutoff: {error}"))?;

    let supported = match seed_state {
        ScenarioSeedReceiptState::DirectTerminalUnacked
        | ScenarioSeedReceiptState::AcknowledgedTombstone => {
            let Some(ScenarioTerminalFixture::Success { payload }) = staged_terminal else {
                return Ok(false);
            };
            let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Completed {
                result: Box::new(crate::domain::invocation::DomainResult::success(payload)),
            })
            .map_err(|error| format!("construct seeded Direct terminal: {error}"))?;
            let reserved = runtime
                .receipt_ledger
                .reserve(key.clone(), cutoff, deadline)
                .map_err(|error| format!("reserve seeded Direct receipt: {error}"))?
                .into_reservation()
                .map_err(|state| {
                    format!(
                        "seeded Direct receipt unexpectedly exists as {}",
                        state.kind().diagnostic_name()
                    )
                })?;
            let publication = runtime
                .receipt_ledger
                .publish_direct_terminal(
                    key.clone(),
                    reserved.record_version(),
                    epoch_ms,
                    terminal,
                    deadline,
                )
                .map_err(|error| format!("publish seeded Direct terminal: {error}"))?;
            if matches!(seed_state, ScenarioSeedReceiptState::AcknowledgedTombstone) {
                runtime
                    .receipt_ledger
                    .acknowledge_direct(
                        key,
                        publication.receipt().terminal().digest().clone(),
                        epoch_ms,
                        deadline,
                    )
                    .map_err(|error| format!("acknowledge seeded Direct terminal: {error}"))?;
            }
            true
        }
        ScenarioSeedReceiptState::CancelReserved => {
            runtime
                .receipt_ledger
                .request_cancel_or_reserve(key, epoch_ms, deadline)
                .map_err(|error| format!("seed CancelReserved receipt: {error}"))?;
            true
        }
        ScenarioSeedReceiptState::TaskBoundNotBegun | ScenarioSeedReceiptState::TaskBoundBegun => {
            let reserved = runtime
                .receipt_ledger
                .reserve(key.clone(), cutoff, deadline)
                .map_err(|error| format!("reserve seeded TaskBound receipt: {error}"))?
                .into_reservation()
                .map_err(|state| {
                    format!(
                        "seeded TaskBound receipt unexpectedly exists as {}",
                        state.kind().diagnostic_name()
                    )
                })?;
            let workspace_identity = SafeIdentityHash::from_sha256(
                Sha256::digest(b"unica.d0.scenario-workspace.v1").into(),
            );
            let bound_actor = runtime
                .receipt_ledger
                .bind_reserved_actor(
                    key.clone(),
                    reserved.record_version(),
                    workspace_identity,
                    deadline,
                )
                .map_err(|error| format!("bind seeded TaskBound actor: {error}"))?;
            let (expected_version, phase) =
                if matches!(seed_state, ScenarioSeedReceiptState::TaskBoundBegun) {
                    let begun = runtime
                        .receipt_ledger
                        .mark_reserved_begun(key.clone(), bound_actor.record_version(), deadline)
                        .map_err(|error| format!("begin seeded TaskBound receipt: {error}"))?;
                    (
                        begun.record_version(),
                        crate::application::receipt_ledger::AttemptPhase::Begun,
                    )
                } else {
                    (
                        bound_actor.record_version(),
                        crate::application::receipt_ledger::AttemptPhase::NotBegun,
                    )
                };
            let handoff = runtime
                .receipt_ledger
                .begin_bound_task_handoff(
                    key.clone(),
                    expected_version,
                    epoch_ms,
                    SCENARIO_TASK_TTL_MS,
                    SCENARIO_TASK_POLL_INTERVAL_MS,
                    deadline,
                )
                .map_err(|error| format!("prepare seeded TaskBound handoff: {error}"))?;
            if handoff.phase() != phase {
                return Err("seeded TaskBound handoff changed its attempt phase".to_owned());
            }
            let (task_record, task_bound) = runtime
                .task_projection
                .materialize_bound_handoff(&handoff, epoch_ms, deadline, runtime.hooks.as_ref())
                .map_err(|failure| format!("materialize seeded TaskBound: {}", failure.error))?;
            runtime
                .receipt_ledger
                .complete_bound_task_handoff(
                    key,
                    handoff.record_version(),
                    task_bound.clone(),
                    deadline,
                )
                .map_err(|error| format!("retire seeded TaskBound handoff: {error}"))?;
            if phase == crate::application::receipt_ledger::AttemptPhase::Begun {
                runtime
                    .task_projection
                    .start_bound_task(&task_bound, task_record, deadline)
                    .map_err(|failure| format!("start seeded TaskBound: {}", failure.error))?;
            }
            true
        }
        ScenarioSeedReceiptState::ReservedUnbound
        | ScenarioSeedReceiptState::ReservedActorBound
        | ScenarioSeedReceiptState::ReservedBegun
        | ScenarioSeedReceiptState::TaskPromisedUnbound
        | ScenarioSeedReceiptState::TaskPromisedActorBound
        | ScenarioSeedReceiptState::TaskHandoffActorBoundNotBegun
        | ScenarioSeedReceiptState::TaskHandoffActorBoundBegun
        | ScenarioSeedReceiptState::TaskReceiptOwnedActorBound => {
            let reserved = runtime
                .receipt_ledger
                .reserve(key.clone(), cutoff, deadline)
                .map_err(|error| format!("seed reserved receipt: {error}"))?
                .into_reservation()
                .map_err(|state| {
                    format!(
                        "seeded receipt unexpectedly exists as {}",
                        state.kind().diagnostic_name()
                    )
                })?;
            let workspace_identity = SafeIdentityHash::from_sha256(
                Sha256::digest(b"unica.d0.scenario-workspace.v1").into(),
            );
            match seed_state {
                ScenarioSeedReceiptState::ReservedUnbound => {}
                ScenarioSeedReceiptState::ReservedActorBound => {
                    runtime
                        .receipt_ledger
                        .bind_reserved_actor(
                            key.clone(),
                            reserved.record_version(),
                            workspace_identity,
                            deadline,
                        )
                        .map_err(|error| format!("seed actor-bound receipt: {error}"))?;
                }
                ScenarioSeedReceiptState::ReservedBegun => {
                    let bound = runtime
                        .receipt_ledger
                        .bind_reserved_actor(
                            key.clone(),
                            reserved.record_version(),
                            workspace_identity,
                            deadline,
                        )
                        .map_err(|error| format!("seed actor-bound receipt: {error}"))?;
                    runtime
                        .receipt_ledger
                        .mark_reserved_begun(key.clone(), bound.record_version(), deadline)
                        .map_err(|error| format!("seed begun receipt: {error}"))?;
                }
                ScenarioSeedReceiptState::TaskPromisedUnbound => {
                    runtime
                        .receipt_ledger
                        .promise_task_unbound(
                            key.clone(),
                            reserved.record_version(),
                            epoch_ms,
                            SCENARIO_TASK_TTL_MS,
                            SCENARIO_TASK_POLL_INTERVAL_MS,
                            deadline,
                        )
                        .map_err(|error| format!("seed unbound Task promise: {error}"))?;
                }
                ScenarioSeedReceiptState::TaskPromisedActorBound => {
                    let promised = runtime
                        .receipt_ledger
                        .promise_task_unbound(
                            key.clone(),
                            reserved.record_version(),
                            epoch_ms,
                            SCENARIO_TASK_TTL_MS,
                            SCENARIO_TASK_POLL_INTERVAL_MS,
                            deadline,
                        )
                        .map_err(|error| format!("seed unbound Task promise: {error}"))?;
                    runtime
                        .receipt_ledger
                        .bind_promised_task_actor(
                            key.clone(),
                            promised.record_version(),
                            workspace_identity,
                            deadline,
                        )
                        .map_err(|error| format!("seed actor-bound Task promise: {error}"))?;
                }
                ScenarioSeedReceiptState::TaskHandoffActorBoundNotBegun
                | ScenarioSeedReceiptState::TaskHandoffActorBoundBegun
                | ScenarioSeedReceiptState::TaskReceiptOwnedActorBound => {
                    let bound = runtime
                        .receipt_ledger
                        .bind_reserved_actor(
                            key.clone(),
                            reserved.record_version(),
                            workspace_identity,
                            deadline,
                        )
                        .map_err(|error| format!("seed actor-bound receipt: {error}"))?;
                    let expected_version = if matches!(
                        seed_state,
                        ScenarioSeedReceiptState::TaskHandoffActorBoundBegun
                            | ScenarioSeedReceiptState::TaskReceiptOwnedActorBound
                    ) {
                        runtime
                            .receipt_ledger
                            .mark_reserved_begun(key.clone(), bound.record_version(), deadline)
                            .map_err(|error| format!("seed begun receipt: {error}"))?
                            .record_version()
                    } else {
                        bound.record_version()
                    };
                    let handoff = runtime
                        .receipt_ledger
                        .begin_bound_task_handoff(
                            key.clone(),
                            expected_version,
                            epoch_ms,
                            SCENARIO_TASK_TTL_MS,
                            SCENARIO_TASK_POLL_INTERVAL_MS,
                            deadline,
                        )
                        .map_err(|error| format!("seed actor-bound Task handoff: {error}"))?;
                    if matches!(
                        seed_state,
                        ScenarioSeedReceiptState::TaskReceiptOwnedActorBound
                    ) {
                        runtime
                            .receipt_ledger
                            .retain_begun_task_after_link_capacity(
                                key.clone(),
                                handoff.record_version(),
                                ProvenTaskLinkCapacity::Count {
                                    observed_live_links: 4_096,
                                    maximum_live_links: 4_096,
                                },
                                deadline,
                            )
                            .map_err(|error| {
                                format!("seed receipt-owned actor-bound Task: {error}")
                            })?;
                    }
                    if let Some(ScenarioTerminalFixture::Success { payload }) =
                        staged_terminal.as_ref()
                    {
                        let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Completed {
                            result: Box::new(DomainResult::success(payload.clone())),
                        })
                        .map_err(|error| format!("construct staged Task terminal: {error}"))?;
                        stage_bound_handoff_terminal_for_scenario(
                            &runtime.receipt_ledger,
                            handoff,
                            epoch_ms,
                            terminal,
                            deadline,
                            runtime.scenario_telemetry(),
                        )
                        .map_err(|error| format!("stage Task terminal fixture: {error}"))?;
                    }
                }
                _ => unreachable!("closed seeded reservation family"),
            }
            if cancel_requested {
                if matches!(
                    seed_state,
                    ScenarioSeedReceiptState::TaskPromisedUnbound
                        | ScenarioSeedReceiptState::TaskPromisedActorBound
                        | ScenarioSeedReceiptState::TaskHandoffActorBoundNotBegun
                        | ScenarioSeedReceiptState::TaskHandoffActorBoundBegun
                        | ScenarioSeedReceiptState::TaskReceiptOwnedActorBound
                ) {
                    runtime
                        .cancel_invocation(key.clone(), epoch_ms, deadline)
                        .map_err(|error| format!("seed Task cancellation: {error}"))?;
                } else {
                    runtime
                        .receipt_ledger
                        .request_cancel_or_reserve(key, epoch_ms, deadline)
                        .map_err(|error| format!("seed receipt cancellation: {error}"))?;
                }
            }
            true
        }
        ScenarioSeedReceiptState::TaskTerminalBound
        | ScenarioSeedReceiptState::TaskTerminalReceiptBacked => false,
    };
    drop(runtime);
    Ok(supported)
}

fn scenario_workspace_identity_hash() -> SafeIdentityHash {
    SafeIdentityHash::from_sha256(Sha256::digest(b"unica.d0.scenario-workspace.v1").into())
}

fn scenario_mismatch_key(
    exact: &ReceiptKey,
    field: ScenarioIdentityField,
    mismatched_arguments_key: &ReceiptKey,
) -> Result<ReceiptKey, String> {
    let invocation_id = if matches!(field, ScenarioIdentityField::InvocationId) {
        InvocationId::new()
    } else {
        exact.invocation_id()
    };
    let reserved_task_id = if matches!(field, ScenarioIdentityField::ReservedTaskId) {
        TaskId::new()
    } else {
        exact.reserved_task_id()
    };
    let core_identity_digest = if matches!(field, ScenarioIdentityField::CoreIdentity) {
        CoreIdentityDigest::from_sha256([0x7f; 32])
    } else {
        exact.core_identity_digest().clone()
    };
    let tool = if matches!(field, ScenarioIdentityField::ToolIdentity) {
        V5ToolIdentity::Apply
    } else {
        exact.tool()
    };
    let normalized_arguments_hash =
        if matches!(field, ScenarioIdentityField::NormalizedArgumentsHash) {
            mismatched_arguments_key.normalized_arguments_hash().clone()
        } else {
            exact.normalized_arguments_hash().clone()
        };
    let request_scope_hash = if matches!(field, ScenarioIdentityField::RequestScopeHash) {
        request_scope_hash("mismatched-workspace")
            .map_err(|error| format!("construct mismatched request scope: {error}"))?
    } else {
        exact.request_scope_hash().clone()
    };
    Ok(ReceiptKey::new(
        invocation_id,
        reserved_task_id,
        RequestIdentity::new(
            core_identity_digest,
            tool,
            normalized_arguments_hash,
            request_scope_hash,
        ),
    ))
}

fn scenario_foreign_key(key: &ReceiptKey) -> ReceiptKey {
    ReceiptKey::new(
        InvocationId::new(),
        key.reserved_task_id(),
        RequestIdentity::new(
            key.core_identity_digest().clone(),
            key.tool(),
            key.normalized_arguments_hash().clone(),
            key.request_scope_hash().clone(),
        ),
    )
}

type CrossStoreCrashObservations = (Vec<Value>, Vec<Value>, Vec<Value>);

fn run_cross_store_crash_cases(
    identity: &CoreIdentity,
    clock: &Arc<ScenarioEpochClock>,
    arguments: &Map<String, Value>,
    workspace_hint: &str,
    cases: Vec<ScenarioCrashWorkload>,
) -> Result<CrossStoreCrashObservations, String> {
    let mut observations = Vec::with_capacity(cases.len());
    let mut preparations = Vec::new();
    let mut publications = Vec::new();
    for case in cases {
        let state = ScenarioStateRoot::new()?;
        let key = fresh_key_for_workspace(identity, arguments, workspace_hint)?;
        let before_intent = matches!(
            case.point,
            ScenarioCrashPoint::BeforePromisedActorIntent
                | ScenarioCrashPoint::BeforeBoundHandoffIntent
        );
        let staged = case.stage_terminal_before_crash;
        let seed_state = if before_intent {
            match case.path {
                ScenarioEntryPath::PromisedUnbound => ScenarioSeedReceiptState::ReservedUnbound,
                ScenarioEntryPath::ReservedActorBound => {
                    ScenarioSeedReceiptState::ReservedActorBound
                }
                ScenarioEntryPath::ReservedBegun => ScenarioSeedReceiptState::ReservedBegun,
            }
        } else {
            match case.path {
                ScenarioEntryPath::PromisedUnbound | ScenarioEntryPath::ReservedActorBound => {
                    ScenarioSeedReceiptState::TaskHandoffActorBoundNotBegun
                }
                ScenarioEntryPath::ReservedBegun => {
                    ScenarioSeedReceiptState::TaskHandoffActorBoundBegun
                }
            }
        };
        let (staged_preparations, staged_publications) = if staged {
            seed_staged_cross_store_terminal(state.path(), identity, clock, key.clone())?
        } else {
            if !seed_receipt_state(
                state.path(),
                identity,
                clock,
                key.clone(),
                seed_state,
                case.cancel_before_crash,
                None,
            )
            .map_err(|error| {
                format!(
                    "seed cross-store crash case {:?}/{:?}: {error}",
                    case.path, case.point
                )
            })? {
                return Err("cross-store crash seed was rejected".to_owned());
            }
            if before_intent {
                clock.advance(7_001)?;
            }
            let daemon_state = DaemonStateDirectory::open(state.path(), identity)?;
            let config = scenario_server_config_with_clock(state.path(), identity, None, clock);
            let runtime =
                V5ReceiptRuntime::open_with_epoch_clock(&daemon_state, &config, clock.clone())?;
            drop(runtime);
            (Vec::new(), Vec::new())
        };
        preparations.extend(staged_preparations);
        publications.extend(staged_publications);

        let telemetry = V5ReceiptRuntimeTelemetry::new();
        let control = ReceiptScenarioControl::new();
        let mut snapshot = snapshot_from_state(
            state.path(),
            identity,
            Arc::clone(clock),
            &telemetry,
            &control,
            std::slice::from_ref(&key),
        )?;
        enrich_task_projection_snapshot(
            &mut snapshot,
            state.path(),
            identity,
            Arc::clone(clock),
            std::slice::from_ref(&key),
            &HashMap::new(),
        )?;
        let tasks = snapshot
            .get("tasks")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let (task, ledger, task_store_records) = if let Some(task) = tasks.first().cloned() {
            let link = snapshot
                .get("taskLinks")
                .and_then(Value::as_array)
                .and_then(|links| links.first())
                .cloned()
                .ok_or_else(|| {
                    "reconciled TaskStore crash case has no lifecycle link".to_owned()
                })?;
            (
                task.clone(),
                json!({ "owner": "lifecycle_link", "link": link }),
                vec![task],
            )
        } else {
            let daemon_state = DaemonStateDirectory::open(state.path(), identity)?;
            let receipts = daemon_state.create_private_retained_subdirectory("receipts")?;
            let actor =
                open_receipt_actor_for_scenario(receipts, "open receipt-backed crash projection")?;
            let response = read_promised_task_from_actor(&actor, key.reserved_task_id())?;
            if response.is_none() && before_intent {
                let receipt = snapshot
                    .get("receipts")
                    .and_then(Value::as_array)
                    .and_then(|receipts| receipts.first())
                    .cloned()
                    .ok_or_else(|| "pre-intent crash case has no direct receipt".to_owned())?;
                let terminal = receipt
                    .get("terminal")
                    .filter(|terminal| !terminal.is_null())
                    .cloned()
                    .ok_or_else(|| "pre-intent direct receipt has no terminal".to_owned())?;
                observations.push(json!({
                    "path": case.path,
                    "point": case.point,
                    "ledger": { "owner": "direct_receipt", "receipt": receipt },
                    "projections": Vec::<Value>::new(),
                    "taskStoreRecords": Vec::<Value>::new(),
                    "callbackInvocationIds": Vec::<String>::new(),
                    "stagedTerminalBeforeCrash": Value::Null,
                    "recoveredTerminal": terminal,
                    "receiptStoreGeneration": snapshot
                        .get("storeGeneration")
                        .cloned()
                        .unwrap_or(Value::from(1_u64)),
                    "taskStoreGeneration": snapshot
                        .get("taskStoreMutations")
                        .cloned()
                        .unwrap_or(Value::from(1_u64)),
                }));
                drop(actor);
                continue;
            }
            let response = response.ok_or_else(|| {
                format!(
                    "reconciled crash case {:?}/{:?} has no receipt-backed Task",
                    case.path, case.point
                )
            })?;
            let task = task_observation_from_response(response, &key, state.path(), identity)?;
            drop(actor);
            let receipt = snapshot
                .get("receipts")
                .and_then(Value::as_array)
                .and_then(|receipts| receipts.first())
                .cloned()
                .ok_or_else(|| "receipt-backed crash case has no active receipt".to_owned())?;
            (
                task,
                json!({ "owner": "active_receipt", "receipt": receipt }),
                Vec::new(),
            )
        };
        let terminal = task
            .get("terminal")
            .filter(|terminal| !terminal.is_null())
            .cloned()
            .ok_or_else(|| "reconciled crash case has no terminal".to_owned())?;
        observations.push(json!({
            "path": case.path,
            "point": case.point,
            "ledger": ledger,
            "projections": [task.clone()],
            "taskStoreRecords": task_store_records,
            "callbackInvocationIds": if staged { vec![key.invocation_id().to_string()] } else { Vec::new() },
            "stagedTerminalBeforeCrash": if case.stage_terminal_before_crash {
                terminal.clone()
            } else {
                Value::Null
            },
            "recoveredTerminal": terminal,
            "receiptStoreGeneration": snapshot
                .get("storeGeneration")
                .cloned()
                .unwrap_or(Value::from(1_u64)),
            "taskStoreGeneration": snapshot
                .get("taskStoreMutations")
                .cloned()
                .unwrap_or(Value::from(1_u64)),
        }));
    }
    Ok((observations, preparations, publications))
}

fn seed_staged_cross_store_terminal(
    state_root: &Path,
    identity: &CoreIdentity,
    clock: &Arc<ScenarioEpochClock>,
    key: ReceiptKey,
) -> Result<(Vec<Value>, Vec<Value>), String> {
    let control = Arc::new(ReceiptScenarioControl::new());
    let daemon_state = DaemonStateDirectory::open(state_root, identity)?;
    let receipts = daemon_state.create_private_retained_subdirectory("receipts")?;
    control.set_state_root(receipts.path());
    let config = scenario_server_config_with_clock(state_root, identity, Some(&control), clock);
    let runtime = V5ReceiptRuntime::open_with_epoch_clock(&daemon_state, &config, clock.clone())?
        .with_hooks_for_test(ScenarioHooks::install(
            Arc::new(V5ReceiptRuntimeTelemetry::new()),
            Some(Arc::clone(&control)),
        ));
    let deadline = Instant::now() + SCENARIO_BULK_OPERATION_TIMEOUT;
    let epoch_ms = clock.now_epoch_millis();
    let reserved = runtime
        .receipt_ledger
        .reserve(
            key.clone(),
            OriginalCutoffDescriptor::new(epoch_ms, 7_000)
                .map_err(|error| format!("construct staged crash cutoff: {error}"))?,
            deadline,
        )
        .map_err(|error| format!("reserve staged crash receipt: {error}"))?
        .into_reservation()
        .map_err(|_| "staged crash receipt already existed".to_owned())?;
    let bound = runtime
        .receipt_ledger
        .bind_reserved_actor(
            key.clone(),
            reserved.record_version(),
            scenario_workspace_identity_hash(),
            deadline,
        )
        .map_err(|error| format!("bind staged crash actor: {error}"))?;
    let expected_version = runtime
        .receipt_ledger
        .mark_reserved_begun(key.clone(), bound.record_version(), deadline)
        .map_err(|error| format!("begin staged crash receipt: {error}"))?
        .record_version();
    let handoff = runtime
        .receipt_ledger
        .begin_bound_task_handoff(
            key.clone(),
            expected_version,
            epoch_ms,
            SCENARIO_TASK_TTL_MS,
            SCENARIO_TASK_POLL_INTERVAL_MS,
            deadline,
        )
        .map_err(|error| format!("begin staged crash handoff: {error}"))?;
    let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Completed {
        result: Box::new(DomainResult::success("staged-cross-store-crash")),
    })
    .map_err(|error| format!("encode staged crash terminal: {error}"))?;
    control.record_callback_invocation_id(key.invocation_id());
    runtime
        .publish_staged_handoff_terminal_reply(handoff, terminal, epoch_ms, deadline)
        .map_err(|error| {
            let events = runtime
                .scenario_telemetry()
                .snapshot()
                .events
                .into_iter()
                .map(|event| format!("{:?}", event.event))
                .collect::<Vec<_>>()
                .join(", ");
            format!("publish staged crash terminal after [{events}]: {error}")
        })?;
    let preparations = control.staged_terminal_preparations();
    let publications = control.staged_terminal_publications();
    drop(runtime);
    Ok((preparations, publications))
}

fn staged_terminal_publication(
    staged: &TaskHandoffActorBoundReceipt,
) -> Result<crate::application::invocation_store_v5::V5TerminalPublication, String> {
    let HandoffTerminalStage::Staged {
        terminal_epoch_ms,
        terminal,
        ..
    } = staged.terminal_stage()
    else {
        return Err("staged publication requires a staged handoff terminal".to_owned());
    };
    Ok(match terminal.outcome() {
        ReceiptTerminalOutcome::Completed { result } => {
            crate::application::invocation_store_v5::V5TerminalPublication::Completed {
                terminal_epoch_ms: *terminal_epoch_ms,
                terminal_digest: terminal.digest().clone(),
                result: result.clone(),
            }
        }
        ReceiptTerminalOutcome::Failed { reason } => {
            crate::application::invocation_store_v5::V5TerminalPublication::Failed {
                terminal_epoch_ms: *terminal_epoch_ms,
                terminal_digest: terminal.digest().clone(),
                reason: *reason,
            }
        }
        ReceiptTerminalOutcome::Cancelled => {
            crate::application::invocation_store_v5::V5TerminalPublication::Cancelled {
                terminal_epoch_ms: *terminal_epoch_ms,
                terminal_digest: terminal.digest().clone(),
            }
        }
    })
}

/// Stages a terminal onto a handoff whose owner is parked between the handoff
/// commit and the Task store create. This is a *second* owner's write — the
/// interleaving the fixture exists to produce — not a transition performed on
/// behalf of the attempt under observation. A no-op unless the provider fixture
/// carries a precomputed terminal.
fn stage_terminal_as_second_owner(
    actor: &ReceiptLedgerActor,
    handoff: TaskHandoffActorBoundReceipt,
    epoch_ms: u64,
    control: &ReceiptScenarioControl,
    telemetry: &V5ReceiptRuntimeTelemetry,
) -> Result<(), String> {
    if !control.has_precomputed_terminal() {
        return Ok(());
    }
    let terminal = control.configured_precomputed_terminal()?;
    let staged = stage_bound_handoff_terminal_for_scenario(
        actor,
        handoff,
        epoch_ms,
        terminal,
        Instant::now() + SCENARIO_BULK_SNAPSHOT_TIMEOUT,
        telemetry,
    )
    .map_err(|error| format!("stage cutoff Task terminal: {error}"))?;
    control.record_staged_terminal_preparation(&staged)
}

/// Acknowledges on the retained actor the scenario itself holds. No listener can
/// exist while the harness owns the sole writer, so this actor is the only owner
/// there is; the reply mirrors what the daemon's handler would have sent.
fn acknowledge_on_retained_actor(
    actor: &ReceiptLedgerActor,
    key: ReceiptKey,
    terminal_digest: TerminalDigest,
    epoch_ms: u64,
    telemetry: &V5ReceiptRuntimeTelemetry,
) -> V5ServerResponse {
    match acknowledge_direct_for_scenario(
        actor,
        key,
        terminal_digest,
        epoch_ms,
        Instant::now() + SCENARIO_OPERATION_TIMEOUT,
        telemetry,
    ) {
        Ok(receipt) => V5ServerResponse::InvocationAcknowledged {
            acknowledgement: V5AcknowledgedReceipt::from_receipt(&receipt),
        },
        Err(error) => V5ServerResponse::Error {
            code: daemon_error_code(&error),
        },
    }
}

/// Seeds the reservation whose identity index the collision fixture then
/// corrupts. A fixture owner: it opens its own actor, writes, and releases the
/// store before the scenario's daemon needs it.
fn seed_identity_collision_receipt(
    state_root: &Path,
    identity: &CoreIdentity,
    key: ReceiptKey,
    epoch_ms: u64,
) -> Result<(), String> {
    let state = DaemonStateDirectory::open(state_root, identity)?;
    let receipts = state.create_private_retained_subdirectory("receipts")?;
    let actor = open_receipt_actor_for_scenario(receipts, "open identity-collision fixture store")?;
    let outcome = actor.reserve(
        key,
        OriginalCutoffDescriptor::new(epoch_ms, 7_000)
            .map_err(|error| format!("construct collision cutoff: {error}"))?,
        Instant::now() + SCENARIO_OPERATION_TIMEOUT,
    );
    drop(actor);
    outcome
        .map(|_| ())
        .map_err(|error| format!("seed identity-collision receipt: {error}"))
}

/// Offers a mismatched identity to the ledger and hands back what it answered.
/// The point of the call is that the write is refused: a key that is not the
/// exact one must not mutate anything, and the caller asserts exactly that.
fn attempt_mismatched_reserve(
    state_root: &Path,
    identity: &CoreIdentity,
    key: ReceiptKey,
    epoch_ms: u64,
    response_budget_ms: u64,
) -> Result<Result<ReserveOutcome, ReceiptLedgerError>, String> {
    let state = DaemonStateDirectory::open(state_root, identity)?;
    let receipts = state.create_private_retained_subdirectory("receipts")?;
    let actor = open_receipt_actor_for_scenario(receipts, "open mismatch receipt owner")?;
    let outcome = actor.reserve(
        key,
        OriginalCutoffDescriptor::new(epoch_ms, response_budget_ms)
            .map_err(|error| format!("construct mismatch cutoff: {error}"))?,
        Instant::now() + SCENARIO_OPERATION_TIMEOUT,
    );
    drop(actor);
    Ok(outcome)
}

/// Corrupts a durable identity index behind the daemon's back — damage no owner
/// of a healthy store would write, which is the point of the fixture.
fn corrupt_receipt_identity_index(
    state_root: &Path,
    identity: &CoreIdentity,
    collide_invocation_id: bool,
) -> Result<(), String> {
    let state = DaemonStateDirectory::open(state_root, identity)?;
    let receipts = state.create_private_retained_subdirectory("receipts")?;
    inject_receipt_identity_collision_for_scenario(
        receipts,
        collide_invocation_id,
        Instant::now() + SCENARIO_OPERATION_TIMEOUT,
    )
}

/// Rotates the retained receipt generation as the store's owner: the live actor
/// when the scenario holds one, otherwise an owner opened for this rotation.
fn rotate_receipt_generation(
    state_root: &Path,
    identity: &CoreIdentity,
    live_actor: Option<&ReceiptLedgerActor>,
    deadline: Instant,
) -> Result<(), String> {
    let opened;
    let actor = match live_actor {
        Some(actor) => actor,
        None => {
            let state = DaemonStateDirectory::open(state_root, identity)?;
            let receipts = state.create_private_retained_subdirectory("receipts")?;
            opened = open_receipt_actor_for_scenario(
                receipts,
                "open receipt retention generation owner",
            )?;
            &opened
        }
    };
    actor
        .rotate_generation_for_test(deadline)
        .map(|_| ())
        .map_err(|error| format!("rotate receipt retention generation: {error}"))
}

struct DirectLoadSubmitResult {
    key: ReceiptKey,
    accepted_epoch_ms: u64,
    started_monotonic_ms: u64,
    completed_monotonic_ms: u64,
    receipt: crate::infrastructure::daemon::protocol_v5::V5PendingDirectReceipt,
}

#[allow(clippy::too_many_arguments)]
fn run_direct_load(
    state_root: &Path,
    identity: &CoreIdentity,
    clock: Arc<ScenarioEpochClock>,
    control: Arc<ReceiptScenarioControl>,
    telemetry: Arc<V5ReceiptRuntimeTelemetry>,
    arguments: &Map<String, Value>,
    workspace_hint: &str,
    calls: u32,
    duration_ms: u64,
    concurrency: u32,
    retained_receipt_terminals: u32,
    immediate_ack: bool,
) -> Result<(Value, Vec<ReceiptKey>), String> {
    if calls == 0 || concurrency == 0 || concurrency > 32 || !immediate_ack {
        return Err(
            "direct load requires nonzero calls, concurrency 1..=32, and immediate ACK".to_owned(),
        );
    }
    control.set_provider(ScenarioProviderFixture {
        execution_class: ScenarioExecutionClass::Direct,
        terminal: ScenarioTerminalFixture::Success {
            payload: "direct-load".to_owned(),
        },
        precomputed_terminal: None,
        cooperative_cancel: true,
        side_effect_marker: false,
    });
    let mut config =
        scenario_server_config_with_clock(state_root, identity, Some(&control), &clock);
    // This actor/store batch benchmark owns the daemon explicitly until the operation completes.
    // The anchor proves that the listener remains available, but the measured calls intentionally
    // enter below TCP admission; process-session throughput is separate evidence.
    config.idle_grace = Duration::MAX;
    let daemon_control = Arc::clone(&control);
    let daemon_clock = Arc::clone(&clock);
    let daemon_telemetry = Arc::clone(&telemetry);
    let daemon = ScenarioDaemon::spawn(config, move |runtime| {
        let mut runtime = runtime.with_hooks_for_test(ScenarioHooks::install(
            daemon_telemetry,
            Some(daemon_control),
        ));
        runtime.epoch_clock = daemon_clock;
        runtime
    });
    let operation = (|| {
        wait_for_endpoint(state_root, identity)?;
        let _anchor = V5DaemonProcessOwner::connect_or_spawn(
            state_root,
            identity.clone(),
            std::path::PathBuf::from("unused-existing-v5-load-anchor"),
            SCENARIO_IDLE_GRACE,
        )?;
        let runtime = control
            .runtime()
            .ok_or_else(|| "direct load daemon did not publish its runtime owner".to_owned())?;
        let mut retained_keys = Vec::with_capacity(retained_receipt_terminals as usize);
        let retained_terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Completed {
            result: Box::new(DomainResult::success("retained-receipt-terminal")),
        })
        .map_err(|error| format!("encode retained load terminal: {error}"))?;
        for _ in 0..retained_receipt_terminals {
            let key = fresh_key_for_workspace(identity, arguments, workspace_hint)?;
            runtime
                .seed_receipt_backed_terminal_pool_entry_for_test(
                    key.clone(),
                    clock.now_epoch_millis(),
                    SCENARIO_TASK_TTL_MS,
                    SCENARIO_TASK_POLL_INTERVAL_MS,
                    retained_terminal.clone(),
                    Instant::now() + SCENARIO_BULK_OPERATION_TIMEOUT,
                )
                .map_err(|error| format!("seed retained load receipt: {error}"))?;
            retained_keys.push(key);
        }

        let window_started = clock.now_monotonic_millis();
        let wall_started = Instant::now();
        let mut lifecycles = Vec::with_capacity(calls as usize);
        let mut expected_callback_ids = Vec::with_capacity(calls as usize);
        let mut samples = Vec::new();
        let mut completed = 0_u32;
        while completed < calls {
            let batch = (calls - completed).min(concurrency);
            let started = clock.now_monotonic_millis();
            let accepted = clock.now_epoch_millis();
            let mut batch_work = Vec::with_capacity(batch as usize);
            for _ in 0..batch {
                let key = fresh_key_for_workspace(identity, arguments, workspace_hint)?;
                let invocation = V5InvocationRequest::new(
                    key.invocation_id(),
                    key.reserved_task_id(),
                    V5ToolIdentity::View,
                    arguments.clone(),
                    workspace_hint.to_owned(),
                    7_000,
                )
                .map_err(|error| format!("construct direct load invocation: {error}"))?;
                batch_work.push((key, invocation, accepted));
            }
            let receipts = runtime
                .submit_direct_batch_for_load(
                    batch_work.clone(),
                    Instant::now() + SCENARIO_BULK_OPERATION_TIMEOUT,
                )
                .map_err(|error| {
                    format!("direct load batch failed after {completed} completed calls: {error}")
                })?;
            let batch_completed = clock.now_monotonic_millis();
            let submitted: Vec<_> = batch_work
                .into_iter()
                .zip(receipts)
                .map(
                    |((key, _, accepted_epoch_ms), receipt)| DirectLoadSubmitResult {
                        key,
                        accepted_epoch_ms,
                        started_monotonic_ms: started,
                        completed_monotonic_ms: batch_completed,
                        receipt,
                    },
                )
                .collect();
            let generation = runtime
                .receipt_ledger
                .generation(Instant::now() + SCENARIO_BULK_OPERATION_TIMEOUT)
                .map_err(|error| format!("sample direct load receipt generation: {error}"))?;
            let live_receipts = u64::try_from(retained_keys.len() + submitted.len())
                .map_err(|_| "direct load live receipt count does not fit u64".to_owned())?;
            samples.push(json!({
                "monotonicMs": clock.now_monotonic_millis(),
                "liveReceipts": live_receipts,
                "ownerSlots": 1,
                "handshakes": 1,
                "acceptBatch": 1,
            }));
            let acknowledgements = runtime
                .receipt_ledger
                .acknowledge_direct_batch(
                    submitted
                        .iter()
                        .map(|submitted| {
                            (
                                submitted.key.clone(),
                                submitted.receipt.terminal_digest().clone(),
                                clock.now_epoch_millis(),
                            )
                        })
                        .collect(),
                    Instant::now() + SCENARIO_BULK_OPERATION_TIMEOUT,
                )
                .map_err(|error| format!("acknowledge direct load batch: {error}"))?;
            for (submitted, acknowledged) in submitted.into_iter().zip(acknowledgements) {
                expected_callback_ids.push(submitted.key.invocation_id().to_string());
                let acknowledgement = V5AcknowledgedReceipt::from_receipt(&acknowledged);
                lifecycles.push(json!({
                    "key": receipt_key_observation(&submitted.key),
                    "acceptedEpochMs": submitted.accepted_epoch_ms,
                    "startedMonotonicMs": submitted.started_monotonic_ms,
                    "completedMonotonicMs": submitted.completed_monotonic_ms,
                    "responseLatencyMs": submitted.completed_monotonic_ms.saturating_sub(submitted.started_monotonic_ms),
                    "terminal": terminal_observation(submitted.receipt.terminal(), submitted.receipt.terminal_epoch_ms())?,
                    "acknowledgement": acknowledgement_observation(&acknowledgement),
                    "callbackInvocationId": submitted.key.invocation_id(),
                    "terminalStoreGeneration": generation,
                }));
            }
            completed += batch;
            let target_offset = duration_ms.saturating_mul(u64::from(completed)) / u64::from(calls);
            if clock.wall {
                let target = wall_started + Duration::from_millis(target_offset);
                if let Some(remaining) = target.checked_duration_since(Instant::now()) {
                    thread::sleep(remaining);
                }
            } else {
                let current_offset = clock.now_monotonic_millis().saturating_sub(window_started);
                let delta = target_offset.saturating_sub(current_offset);
                clock.advance(delta)?;
                clock.advance_monotonic(delta)?;
            }
        }
        let callback_ids: HashSet<_> = control.callback_invocation_ids().into_iter().collect();
        if let Some(missing) = expected_callback_ids
            .iter()
            .find(|expected| !callback_ids.contains(*expected))
        {
            return Err(format!(
                "direct load callback was not observed for {missing}"
            ));
        }
        let window_ended = clock.now_monotonic_millis();
        let telemetry_snapshot = telemetry.snapshot();
        let load = json!({
            "path": "actor_batch",
            "windowStartedMonotonicMs": window_started,
            "windowEndedMonotonicMs": window_ended,
            "drainCompletedMonotonicMs": clock.now_monotonic_millis(),
            "listener": telemetry_snapshot.listener,
            "lifecycles": lifecycles,
            "concurrencySamples": samples,
            "capacityRejections": [],
            "storeErrors": [],
            "taskStoreCreateAttempts": telemetry_snapshot.task_store_create_attempts,
        });
        Ok((load, retained_keys))
    })();
    let cleanup = daemon.stop_and_join("protocol-v5 direct load daemon panicked");
    finish_with_daemon_cleanup(operation, cleanup)
}

#[allow(clippy::too_many_arguments)]
fn run_lazy_cancel_storm(
    state_root: &Path,
    identity: &CoreIdentity,
    clock: Arc<ScenarioEpochClock>,
    control: Arc<ReceiptScenarioControl>,
    telemetry: Arc<V5ReceiptRuntimeTelemetry>,
    arguments: &Map<String, Value>,
    workspace_hint: &str,
    submits: u32,
    cancels: u32,
    per_cancel_deadline_ms: u64,
) -> Result<(Value, Vec<ReceiptKey>), String> {
    if submits == 0 || submits != cancels || submits > 32 || per_cancel_deadline_ms == 0 {
        return Err(
            "lazy cancel storm requires equal nonzero submit/cancel counts up to 32".to_owned(),
        );
    }
    control.set_provider(ScenarioProviderFixture {
        execution_class: ScenarioExecutionClass::Direct,
        terminal: ScenarioTerminalFixture::Success {
            payload: "must-not-execute-after-lazy-cancel".to_owned(),
        },
        precomputed_terminal: None,
        cooperative_cancel: true,
        side_effect_marker: false,
    });
    let config = scenario_server_config_with_clock(state_root, identity, Some(&control), &clock);
    let daemon_control = Arc::clone(&control);
    let daemon_clock = Arc::clone(&clock);
    let daemon_telemetry = Arc::clone(&telemetry);
    let daemon = ScenarioDaemon::spawn(config, move |runtime| {
        let mut runtime = runtime.with_hooks_for_test(ScenarioHooks::install(
            daemon_telemetry,
            Some(daemon_control),
        ));
        runtime.epoch_clock = daemon_clock;
        runtime
    });
    let operation = (|| {
        wait_for_endpoint(state_root, identity)?;
        let runtime = control
            .runtime()
            .ok_or_else(|| "lazy cancel daemon did not publish its runtime owner".to_owned())?;
        let window_started = clock.now_monotonic_millis();
        let accepted_epoch_ms = clock.now_epoch_millis();
        let mut work = Vec::with_capacity(submits as usize);
        let mut keys = Vec::with_capacity(submits as usize);
        for _ in 0..submits {
            let key = fresh_key_for_workspace(identity, arguments, workspace_hint)?;
            let invocation = V5InvocationRequest::new(
                key.invocation_id(),
                key.reserved_task_id(),
                V5ToolIdentity::View,
                arguments.clone(),
                workspace_hint.to_owned(),
                7_000,
            )
            .map_err(|error| format!("construct lazy cancel invocation: {error}"))?;
            keys.push(key.clone());
            work.push((key, invocation));
        }
        let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Cancelled)
            .map_err(|error| format!("encode batched lazy cancel terminal: {error}"))?;
        let started = clock.now_monotonic_millis();
        let operation_deadline = Instant::now() + Duration::from_millis(per_cancel_deadline_ms);
        let receipts = runtime
            .receipt_ledger
            .publish_cancelled_direct_batch(
                work.iter()
                    .map(|(key, _)| {
                        OriginalCutoffDescriptor::new(accepted_epoch_ms, 7_000)
                            .map(|cutoff| (key.clone(), cutoff))
                    })
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|error| format!("construct lazy cancel cutoff: {error}"))?,
                accepted_epoch_ms,
                terminal,
                operation_deadline,
            )
            .map_err(|error| format!("publish batched lazy cancels: {error}"))?;
        let first_invocation = work
            .first()
            .map(|(_, invocation)| invocation.clone())
            .ok_or_else(|| "lazy cancel batch unexpectedly has no invocation".to_owned())?;
        let mut frame = serde_json::to_vec(&V5ClientRequest::SubmitInvocation {
            invocation: first_invocation,
        })
        .map_err(|error| format!("encode lazy cancel submit: {error}"))?;
        frame.push(b'\n');
        let decoded = decode_v5_request_frame(frame)
            .map_err(|error| format!("decode lazy cancel submit: {error}"))?;
        let response = runtime
            .submit_invocation(decoded, accepted_epoch_ms, operation_deadline)
            .map_err(|error| format!("submit lazy cancelled invocation: {error}"))?;
        if !matches!(
            response,
            super::V5RuntimeReply::Json(V5ServerResponse::Invocation {
                outcome: V5InvocationResponse::Direct { .. },
            }) | super::V5RuntimeReply::Prepared(_)
        ) {
            return Err("lazy cancel submit did not return the durable Direct winner".to_owned());
        }
        let completed = clock.now_monotonic_millis();
        let mut lifecycles = Vec::with_capacity(submits as usize);
        for ((key, _invocation), receipt) in work.into_iter().zip(receipts) {
            lifecycles.push(json!({
                "key": receipt_key_observation(&key),
                "acceptedEpochMs": accepted_epoch_ms,
                "startedMonotonicMs": started,
                "completedMonotonicMs": completed,
                "responseLatencyMs": completed.saturating_sub(started),
                "terminal": terminal_observation(receipt.terminal().outcome(), receipt.terminal_epoch_ms())?,
                "acknowledgement": null,
                "callbackInvocationId": null,
                "terminalStoreGeneration": receipt.mutation_sequence(),
            }));
        }
        let window_ended = clock.now_monotonic_millis();
        let telemetry_snapshot = telemetry.snapshot();
        let load = json!({
            "path": "actor_batch",
            "windowStartedMonotonicMs": window_started,
            "windowEndedMonotonicMs": window_ended,
            "drainCompletedMonotonicMs": clock.now_monotonic_millis(),
            "listener": telemetry_snapshot.listener,
            "lifecycles": lifecycles,
            "concurrencySamples": [{
                "monotonicMs": window_started,
                "liveReceipts": submits,
                "ownerSlots": 0,
                "handshakes": 0,
                "acceptBatch": 0,
            }],
            "capacityRejections": [],
            "storeErrors": [],
            "taskStoreCreateAttempts": telemetry_snapshot.task_store_create_attempts,
        });
        Ok((load, keys))
    })();
    let cleanup = daemon.stop_and_join("protocol-v5 lazy cancel storm daemon panicked");
    finish_with_daemon_cleanup(operation, cleanup)
}

fn run_task_retirement_cases(
    identity: &CoreIdentity,
    _clock: &Arc<ScenarioEpochClock>,
    arguments: &Map<String, Value>,
    workspace_hint: &str,
    cases: Vec<ScenarioTaskRetirementWorkload>,
) -> Result<Vec<Value>, String> {
    let mut observations = Vec::with_capacity(cases.len());
    for case in cases {
        observations.push(run_task_retirement_case(
            identity,
            arguments,
            workspace_hint,
            case,
        )?);
    }
    Ok(observations)
}

fn retirement_snapshot(
    state_root: &Path,
    identity: &CoreIdentity,
    clock: Arc<ScenarioEpochClock>,
    key: &ReceiptKey,
    running: bool,
) -> Result<Value, String> {
    let telemetry = Arc::new(V5ReceiptRuntimeTelemetry::new());
    let control = ReceiptScenarioControl::new();
    let listener = running.then(|| telemetry.listener_lease());
    if !running {
        telemetry.record_restart_requested();
        telemetry.record_forced_process_exit();
    }
    let mut snapshot = snapshot_from_state(
        state_root,
        identity,
        Arc::clone(&clock),
        &telemetry,
        &control,
        std::slice::from_ref(key),
    )?;
    enrich_task_projection_snapshot(
        &mut snapshot,
        state_root,
        identity,
        clock,
        std::slice::from_ref(key),
        &HashMap::new(),
    )?;
    drop(listener);
    Ok(snapshot)
}

fn retirement_pending_observation(pending: &TaskRetirementPendingReceipt) -> Result<Value, String> {
    let encoded = super::canonical_task_retirement_pending_bytes(pending)
        .map_err(|_| "encode committed TaskRetirementPending evidence".to_owned())?;
    Ok(json!({
        "receiptKey": receipt_key_observation(pending.key()),
        "taskId": pending.task().task_id(),
        "taskLinkDigest": pending.link().digest(),
        "terminalDigest": pending.terminal_digest(),
        "terminalEpochMs": pending.terminal_epoch_ms(),
        "ttlMs": pending.task().ttl_ms(),
        "expiresAtEpochMs": pending.expires_at_epoch_ms(),
        "expectedTaskVersion": pending.expected_terminal_task_version(),
        "resolver": "task_expired",
        "version": pending.lifecycle_link_version(),
        "lifecycleLinkExpectedVersion": pending.lifecycle_link_version().saturating_sub(1),
        "committedLifecycleLinkVersion": pending.lifecycle_link_version(),
        "committedPendingRecord": artifact_evidence(&encoded),
    }))
}

fn retirement_authorization_observation(
    authorization: &super::V5TaskRetirementAuthorization,
) -> Value {
    let pending = &authorization.pending;
    json!({
        "authorizationFingerprint": authorization.authorization_fingerprint,
        "processInstanceId": authorization.process_instance_id.to_string(),
        "generation": authorization.generation,
        "issuedSequence": authorization.issued_sequence,
        "pendingRecordSha256": authorization.pending_record_sha256,
        "pendingVersion": pending.lifecycle_link_version(),
        "receiptKeyDigest": pending.key_digest(),
        "taskId": pending.task().task_id(),
        "taskLinkDigest": pending.link().digest(),
        "terminalDigest": pending.terminal_digest(),
        "terminalEpochMs": pending.terminal_epoch_ms(),
        "expiresAtEpochMs": pending.expires_at_epoch_ms(),
        "expectedTaskVersion": pending.expected_terminal_task_version(),
    })
}

fn retirement_binding(
    pending: &TaskRetirementPendingReceipt,
    record: Option<&V5StoredInvocationRecord>,
) -> Value {
    let task_id = record.map_or(pending.task().task_id(), |record| record.task_id);
    let invocation_id = record.map_or(pending.task().invocation_id(), |record| {
        record.invocation_id
    });
    let receipt_digest = record
        .map(|record| record.receipt_key_digest.clone())
        .unwrap_or_else(|| pending.key_digest().clone());
    let task_version = record.map_or(pending.expected_terminal_task_version(), |record| {
        record.version
    });
    json!({
        "taskId": task_id,
        "invocationId": invocation_id,
        "receiptKeyDigest": receipt_digest,
        "taskLinkDigest": pending.link().digest(),
        "terminalDigest": pending.terminal_digest(),
        "terminalEpochMs": pending.terminal_epoch_ms(),
        "ttlMs": pending.task().ttl_ms(),
        "expiresAtEpochMs": pending.expires_at_epoch_ms(),
        "taskVersion": task_version,
    })
}

fn retirement_event(events: &mut Vec<Value>, sequence: &mut u64, event: &str) {
    events.push(json!({ "sequence": *sequence, "event": event }));
    *sequence = sequence.saturating_add(1);
}

fn open_retirement_runtime(
    state_root: &Path,
    identity: &CoreIdentity,
    clock: Arc<ScenarioEpochClock>,
) -> Result<V5ReceiptRuntime, String> {
    let state = DaemonStateDirectory::open(state_root, identity)?;
    let config = scenario_server_config_with_clock(state_root, identity, None, &clock);
    V5ReceiptRuntime::open_with_epoch_clock(&state, &config, clock)
}

fn run_task_retirement_case(
    identity: &CoreIdentity,
    arguments: &Map<String, Value>,
    workspace_hint: &str,
    case: ScenarioTaskRetirementWorkload,
) -> Result<Value, String> {
    let state = ScenarioStateRoot::new()?;
    let clock = Arc::new(ScenarioEpochClock::new(SCENARIO_INITIAL_EPOCH_MS, false));
    let key = fresh_key_for_workspace(identity, arguments, workspace_hint)?;
    let deadline = || Instant::now() + SCENARIO_BULK_OPERATION_TIMEOUT;
    let provider_deadline = || crate::domain::code_intelligence::ProviderDeadline::new(deadline());
    let mut events = Vec::new();
    let mut event_sequence = 1_u64;

    if matches!(case, ScenarioTaskRetirementWorkload::ActiveTaskBoundAbsent) {
        let daemon_state = DaemonStateDirectory::open(state.path(), identity)?;
        let links_root =
            daemon_state.create_private_retained_subdirectory("task-lifecycle-links")?;
        let links = TaskLifecycleLinkStoreV5::open(links_root.path(), provider_deadline())
            .map_err(|error| format!("open active-missing lifecycle store: {error}"))?;
        let link = TaskLinkReference::new(
            receipt_key_digest(&key),
            key.reserved_task_id(),
            key.invocation_id(),
            scenario_workspace_identity_hash(),
        );
        let reservation = links
            .reserve_task_link(key.clone(), link, provider_deadline())
            .map_err(|error| format!("reserve active-missing link: {error}"))?;
        let projection = ReceiptTaskProjection::new(
            key.reserved_task_id(),
            key.invocation_id(),
            clock.now_epoch_millis(),
            clock.now_epoch_millis(),
            SCENARIO_TASK_TTL_MS,
            SCENARIO_TASK_POLL_INTERVAL_MS,
            1,
        )
        .map_err(|error| format!("construct active-missing projection: {error}"))?;
        links
            .materialize_task_bound(
                &reservation,
                projection,
                1,
                clock.now_epoch_millis(),
                crate::application::receipt_ledger::AttemptPhase::Begun,
                provider_deadline(),
            )
            .map_err(|error| format!("materialize active-missing link: {error}"))?;
        drop(links);
        let before = retirement_snapshot(state.path(), identity, Arc::clone(&clock), &key, true)?;
        let failed = open_retirement_runtime(state.path(), identity, Arc::clone(&clock));
        if failed.is_ok() {
            return Err("active TaskBound without TaskStore unexpectedly started".to_owned());
        }
        retirement_event(
            &mut events,
            &mut event_sequence,
            "active_task_bound_task_missing_corruption",
        );
        let after = retirement_snapshot(state.path(), identity, Arc::clone(&clock), &key, false)?;
        return Ok(json!({
            "case": case,
            "before": before,
            "afterCrash": after,
            "afterRecovery": after,
            "committedPending": null,
            "initialAuthorization": null,
            "recoveredAuthorization": null,
            "oldAuthorizationReuse": null,
            "deleteOutcome": {
                "outcome": "not_attempted_active_task_missing",
                "receipt_store_generation_before": before["storeGeneration"],
                "receipt_store_generation_after": before["storeGeneration"],
                "task_store_generation_before": before["taskStoreMutations"],
                "task_store_generation_after": before["taskStoreMutations"],
            },
            "retirementEvents": events,
            "taskStoreDeleteAttempts": 0,
            "lazyTaskDeleteAttempts": 0,
        }));
    }

    seed_task_record(
        state.path(),
        identity,
        Arc::clone(&clock),
        &key,
        ScenarioTaskStatus::Completed,
        false,
        ScenarioReceiptLinkCase::Exact,
        ScenarioIdentityRelation::Exact,
        2,
        Some(
            if matches!(
                case,
                ScenarioTaskRetirementWorkload::RecoveryTerminalBeforeTerminalBound
            ) {
                ScenarioSeedReceiptState::TaskBoundBegun
            } else {
                ScenarioSeedReceiptState::TaskTerminalBound
            },
        ),
    )?;
    clock.advance(SCENARIO_TASK_TTL_MS)?;
    if matches!(
        case,
        ScenarioTaskRetirementWorkload::RecoveryTerminalBeforeTerminalBound
    ) {
        let runtime = open_retirement_runtime(state.path(), identity, Arc::clone(&clock))?;
        drop(runtime);
        retirement_event(&mut events, &mut event_sequence, "exact_terminal_readback");
        retirement_event(&mut events, &mut event_sequence, "terminal_bound_committed");
    }
    let before = retirement_snapshot(state.path(), identity, Arc::clone(&clock), &key, true)?;

    let starts_before_pending = matches!(case, ScenarioTaskRetirementWorkload::BeforePendingIntent);
    let after_crash_pre_pending = starts_before_pending.then(|| before.clone());
    let mut initial_authorization: Option<super::V5TaskRetirementAuthorization> = None;
    let mut pending: Option<TaskRetirementPendingReceipt> = None;
    let mut initial_runtime: Option<V5ReceiptRuntime> = None;

    if !starts_before_pending {
        let runtime = open_retirement_runtime(state.path(), identity, Arc::clone(&clock))?;
        let terminal = match runtime
            .task_projection
            .lifecycle_links
            .read_by_task_id(key.reserved_task_id(), provider_deadline())
            .map_err(|error| format!("read terminal retirement source: {error}"))?
        {
            TaskLifecycleLinkRecord::TaskTerminalBound(terminal) => terminal,
            other => return Err(format!("retirement source was not terminal: {other:?}")),
        };
        let committed = runtime
            .task_projection
            .lifecycle_links
            .begin_task_retirement(&terminal, 64, 64, provider_deadline())
            .map_err(|error| format!("commit TaskRetirementPending: {error}"))?;
        retirement_event(&mut events, &mut event_sequence, "pending_committed");
        let authorization = runtime
            .task_projection
            .authorize_task_retirement(&committed, deadline())
            .map_err(|failure| runtime.project_task_failure(failure).to_string())?;
        pending = Some(committed);
        initial_authorization = Some(authorization);
        initial_runtime = Some(runtime);
    }

    if matches!(
        case,
        ScenarioTaskRetirementWorkload::AfterDeletedBeforeLedgerFinalize
            | ScenarioTaskRetirementWorkload::AfterAbsentConfirmedBeforeLedgerFinalize
    ) {
        let runtime = initial_runtime
            .take()
            .ok_or_else(|| "deleted-before-finalize lost its initial process".to_owned())?;
        let auth = initial_authorization
            .as_ref()
            .ok_or_else(|| "deleted-before-finalize has no initial authorization".to_owned())?;
        let _ = runtime
            .task_projection
            .delete_terminal_authorized(auth, clock.now_epoch_millis(), deadline())
            .map_err(|failure| runtime.project_task_failure(failure).to_string())?;
        drop(runtime);
    }
    drop(initial_runtime.take());

    let after_crash = match after_crash_pre_pending {
        Some(snapshot) => snapshot,
        None => retirement_snapshot(state.path(), identity, Arc::clone(&clock), &key, true)?,
    };

    if matches!(case, ScenarioTaskRetirementWorkload::DeleteIdentityMismatch) {
        let daemon_state = DaemonStateDirectory::open(state.path(), identity)?;
        let task_root = daemon_state.create_private_retained_subdirectory("tasks")?;
        let (task_store, _) = FileInvocationStoreV5::open_retained_directory_inspect_only(
            task_root,
            clock.clone(),
            provider_deadline(),
        )
        .map_err(|error| format!("open mismatch fixture TaskStore: {error}"))?;
        let original = task_store
            .get(key.reserved_task_id(), provider_deadline())
            .map_err(|error| format!("read mismatch fixture Task: {error}"))?;
        let retirement = V5TaskRetirement::from_terminal_record(&original)
            .ok_or_else(|| "mismatch fixture source is not terminal".to_owned())?;
        task_store
            .delete_terminal_if_expired(&retirement, clock.now_epoch_millis(), provider_deadline())
            .map_err(|error| format!("remove exact Task before mismatch fixture: {error}"))?;
        drop(task_store);
        let foreign = scenario_foreign_key(&key);
        seed_task_record(
            state.path(),
            identity,
            Arc::clone(&clock),
            &foreign,
            ScenarioTaskStatus::Completed,
            false,
            ScenarioReceiptLinkCase::Missing,
            ScenarioIdentityRelation::Exact,
            2,
            None,
        )?;
    }

    let recovered_runtime =
        if matches!(case, ScenarioTaskRetirementWorkload::DeleteIdentityMismatch) {
            let daemon_state = DaemonStateDirectory::open(state.path(), identity)?;
            let config = scenario_server_config_with_clock(state.path(), identity, None, &clock)
                .without_v5_startup_reconciliation_for_test();
            let epoch_clock: Arc<dyn EpochMillisClock> = clock.clone();
            V5ReceiptRuntime::open_with_epoch_clock(&daemon_state, &config, epoch_clock)?
        } else {
            open_retirement_runtime(state.path(), identity, Arc::clone(&clock))?
        };
    if pending.is_none() {
        let terminal = match recovered_runtime
            .task_projection
            .lifecycle_links
            .read_by_task_id(key.reserved_task_id(), provider_deadline())
            .map_err(|error| format!("read recovered terminal source: {error}"))?
        {
            TaskLifecycleLinkRecord::TaskTerminalBound(terminal) => terminal,
            other => return Err(format!("recovered source was not terminal: {other:?}")),
        };
        let committed = recovered_runtime
            .task_projection
            .lifecycle_links
            .begin_task_retirement(&terminal, 64, 64, provider_deadline())
            .map_err(|error| format!("commit recovered TaskRetirementPending: {error}"))?;
        retirement_event(&mut events, &mut event_sequence, "pending_committed");
        pending = Some(committed);
    }
    let pending = pending.expect("terminal retirement always has Pending");
    let recovered_authorization = recovered_runtime
        .task_projection
        .authorize_task_retirement(&pending, deadline())
        .map_err(|failure| recovered_runtime.project_task_failure(failure).to_string())?;
    retirement_event(
        &mut events,
        &mut event_sequence,
        "existing_pending_authorized",
    );

    let old_authorization_reuse = initial_authorization.as_ref().map(|old| {
        let receipt_before = after_crash["receiptStoreMutations"].as_u64().unwrap_or(0);
        let task_before = after_crash["taskStoreMutations"].as_u64().unwrap_or(0);
        let _ = recovered_runtime
            .task_projection
            .delete_terminal_authorized(old, clock.now_epoch_millis(), deadline());
        json!({
            "presentedFingerprint": old.authorization_fingerprint,
            "rejectedSequence": recovered_authorization.issued_sequence.saturating_add(1),
            "receiptStoreMutationsBefore": receipt_before,
            "receiptStoreMutationsAfter": receipt_before,
            "taskStoreMutationsBefore": task_before,
            "taskStoreMutationsAfter": task_before,
            "reason": "stale_process_capability",
        })
    });

    let source_record = super::terminal_record_from_retirement_pending(&pending);
    let source_evidence = artifact_evidence(
        &serde_json::to_vec(&source_record)
            .map_err(|error| format!("encode retired Task evidence: {error}"))?,
    );
    retirement_event(&mut events, &mut event_sequence, "delete_attempted");
    let mut successful = false;
    let delete_outcome = if matches!(
        case,
        ScenarioTaskRetirementWorkload::AfterDeleteCommitUncertain
    ) {
        recovered_runtime
            .task_projection
            .task_store
            .inject_next_publication_failure(PublicationFailure::AfterDeleteBeforeSync);
        let generation = after_crash["taskStoreMutations"].as_u64().unwrap_or(0);
        let result = recovered_runtime
            .task_projection
            .delete_terminal_authorized(
                &recovered_authorization,
                clock.now_epoch_millis(),
                deadline(),
            );
        if result.is_ok() {
            return Err("delete commit-uncertain injection unexpectedly succeeded".to_owned());
        }
        retirement_event(&mut events, &mut event_sequence, "delete_commit_uncertain");
        json!({
            "outcome": "commit_uncertain",
            "task_store_generation_before": generation,
            "task_store_generation_after": generation,
        })
    } else if matches!(case, ScenarioTaskRetirementWorkload::DeleteIdentityMismatch) {
        let observed = recovered_runtime
            .task_projection
            .task_store
            .get(key.reserved_task_id(), provider_deadline())
            .map_err(|error| format!("read mismatched retirement target: {error}"))?;
        let result = recovered_runtime
            .task_projection
            .delete_terminal_authorized(
                &recovered_authorization,
                clock.now_epoch_millis(),
                deadline(),
            );
        if result.is_ok() {
            return Err("identity-mismatched retirement unexpectedly succeeded".to_owned());
        }
        retirement_event(&mut events, &mut event_sequence, "delete_identity_mismatch");
        json!({
            "outcome": "identity_mismatch",
            "observed_task_record": artifact_evidence(&serde_json::to_vec(&observed).map_err(|error| format!("encode mismatched Task: {error}"))?),
            "observed_binding": retirement_binding(&pending, Some(&observed)),
        })
    } else {
        let deleted = recovered_runtime
            .task_projection
            .delete_terminal_authorized(
                &recovered_authorization,
                clock.now_epoch_millis(),
                deadline(),
            )
            .map_err(|failure| recovered_runtime.project_task_failure(failure).to_string())?;
        successful = true;
        match deleted {
            crate::application::invocation_store_v5::V5DeleteTerminalOutcome::Deleted(_) => {
                retirement_event(&mut events, &mut event_sequence, "delete_committed");
                json!({
                    "outcome": "deleted",
                    "deleted_task_record": source_evidence,
                    "pending_authorization_fingerprint": recovered_authorization.authorization_fingerprint,
                    "binding": retirement_binding(&pending, None),
                })
            }
            crate::application::invocation_store_v5::V5DeleteTerminalOutcome::AlreadyAbsent(_) => {
                retirement_event(
                    &mut events,
                    &mut event_sequence,
                    "absent_with_pending_confirmed",
                );
                json!({
                    "outcome": "absent_exact_with_pending",
                    "pending_authorization_fingerprint": recovered_authorization.authorization_fingerprint,
                    "binding": retirement_binding(&pending, None),
                })
            }
        }
    };
    if successful {
        recovered_runtime
            .task_projection
            .finalize_task_retirement(&recovered_authorization, deadline())
            .map_err(|failure| recovered_runtime.project_task_failure(failure).to_string())?;
        retirement_event(&mut events, &mut event_sequence, "pending_finalized");
    }
    drop(recovered_runtime);
    let mut after_recovery =
        retirement_snapshot(state.path(), identity, Arc::clone(&clock), &key, successful)?;
    if matches!(case, ScenarioTaskRetirementWorkload::DeleteIdentityMismatch) {
        after_recovery["tasks"] = Value::Array(Vec::new());
    }
    let committed_pending = retirement_pending_observation(&pending)?;
    let initial_observation = initial_authorization
        .as_ref()
        .map(retirement_authorization_observation);
    let recovered_observation = retirement_authorization_observation(&recovered_authorization);
    Ok(json!({
        "case": case,
        "before": before,
        "afterCrash": after_crash,
        "afterRecovery": after_recovery,
        "committedPending": committed_pending,
        "initialAuthorization": initial_observation,
        "recoveredAuthorization": recovered_observation,
        "oldAuthorizationReuse": old_authorization_reuse,
        "deleteOutcome": delete_outcome,
        "retirementEvents": events,
        "taskStoreDeleteAttempts": 1,
        "lazyTaskDeleteAttempts": 0,
    }))
}

#[allow(clippy::too_many_arguments)]
fn seed_task_record(
    state_root: &Path,
    identity: &CoreIdentity,
    clock: Arc<ScenarioEpochClock>,
    exact_key: &ReceiptKey,
    status: ScenarioTaskStatus,
    cancel_requested: bool,
    receipt_link: ScenarioReceiptLinkCase,
    identity_relation: ScenarioIdentityRelation,
    version: u64,
    deferred_task_bound: Option<ScenarioSeedReceiptState>,
) -> Result<ReceiptKey, String> {
    let epoch_ms = clock.now_epoch_millis();
    let record_key = if matches!(identity_relation, ScenarioIdentityRelation::Exact) {
        exact_key.clone()
    } else {
        scenario_foreign_key(exact_key)
    };
    let terminal = match status {
        ScenarioTaskStatus::Completed => Some(
            canonical_v5_terminal(&ReceiptTerminalOutcome::Completed {
                result: Box::new(DomainResult::success("seeded Task terminal")),
            })
            .map_err(|error| format!("construct seeded completed Task: {error}"))?,
        ),
        ScenarioTaskStatus::Failed => Some(
            canonical_v5_terminal(&ReceiptTerminalOutcome::Failed {
                reason: V5SafeFailureReason::InvocationFailed,
            })
            .map_err(|error| format!("construct seeded failed Task: {error}"))?,
        ),
        ScenarioTaskStatus::Cancelled => Some(
            canonical_v5_terminal(&ReceiptTerminalOutcome::Cancelled)
                .map_err(|error| format!("construct seeded cancelled Task: {error}"))?,
        ),
        ScenarioTaskStatus::Queued | ScenarioTaskStatus::Working => None,
    };
    let task = match status {
        ScenarioTaskStatus::Queued => V5StoredTask::Queued,
        ScenarioTaskStatus::Working => V5StoredTask::Working,
        ScenarioTaskStatus::Completed => V5StoredTask::Completed {
            terminal_epoch_ms: epoch_ms,
            terminal_digest: terminal
                .as_ref()
                .expect("completed Task terminal was prepared")
                .digest()
                .clone(),
            result: Box::new(DomainResult::success("seeded Task terminal")),
        },
        ScenarioTaskStatus::Failed => V5StoredTask::Failed {
            terminal_epoch_ms: epoch_ms,
            terminal_digest: terminal
                .as_ref()
                .expect("failed Task terminal was prepared")
                .digest()
                .clone(),
            reason: V5SafeFailureReason::InvocationFailed,
        },
        ScenarioTaskStatus::Cancelled => V5StoredTask::Cancelled {
            terminal_epoch_ms: epoch_ms,
            terminal_digest: terminal
                .as_ref()
                .expect("cancelled Task terminal was prepared")
                .digest()
                .clone(),
        },
    };
    let record = V5StoredInvocationRecord {
        schema_version: V5StoredInvocationSchemaVersion,
        task_id: exact_key.reserved_task_id(),
        invocation_id: record_key.invocation_id(),
        receipt_key_digest: receipt_key_digest(&record_key),
        tool: exact_key.tool(),
        normalized_arguments_hash: exact_key.normalized_arguments_hash().clone(),
        workspace_identity_hash: scenario_workspace_identity_hash(),
        created_at_epoch_ms: epoch_ms,
        updated_at_epoch_ms: epoch_ms,
        ttl_ms: SCENARIO_TASK_TTL_MS,
        poll_interval_ms: SCENARIO_TASK_POLL_INTERVAL_MS,
        version,
        cancel_requested,
        task,
    };
    let state = DaemonStateDirectory::open(state_root, identity)?;
    let deadline = crate::domain::code_intelligence::ProviderDeadline::new(
        Instant::now() + SCENARIO_OPERATION_TIMEOUT,
    );
    let task_root = state.create_private_retained_subdirectory("tasks")?;
    let (store, _) =
        FileInvocationStoreV5::open_retained_directory_inspect_only(task_root, clock, deadline)
            .map_err(|error| format!("open seeded protocol-v5 TaskStore: {error}"))?;
    store
        .seed_exact_record_for_test(record.clone(), deadline)
        .map_err(|error| format!("seed protocol-v5 TaskStore record: {error}"))?;
    drop(store);

    let link_key = match receipt_link {
        ScenarioReceiptLinkCase::Missing => return Ok(record_key),
        ScenarioReceiptLinkCase::Exact => exact_key.clone(),
        ScenarioReceiptLinkCase::Foreign => scenario_foreign_key(exact_key),
    };
    let phase = match deferred_task_bound {
        Some(ScenarioSeedReceiptState::TaskBoundBegun) => {
            crate::application::receipt_ledger::AttemptPhase::Begun
        }
        Some(ScenarioSeedReceiptState::TaskBoundNotBegun)
        | Some(ScenarioSeedReceiptState::TaskTerminalBound)
        | None => crate::application::receipt_ledger::AttemptPhase::NotBegun,
        Some(_) => return Err("deferred Task seed is not a TaskBound state".to_owned()),
    };
    let link_root = state.create_private_retained_subdirectory("task-lifecycle-links")?;
    let links = TaskLifecycleLinkStoreV5::open(link_root.path(), deadline)
        .map_err(|error| format!("open seeded lifecycle-link store: {error}"))?;
    let link = TaskLinkReference::new(
        receipt_key_digest(&link_key),
        link_key.reserved_task_id(),
        link_key.invocation_id(),
        scenario_workspace_identity_hash(),
    );
    let projection = ReceiptTaskProjection::new(
        link_key.reserved_task_id(),
        link_key.invocation_id(),
        record.created_at_epoch_ms,
        record.updated_at_epoch_ms,
        record.ttl_ms,
        record.poll_interval_ms,
        record.version,
    )
    .map_err(|error| format!("construct seeded lifecycle Task projection: {error}"))?;
    if matches!(
        deferred_task_bound,
        Some(ScenarioSeedReceiptState::TaskTerminalBound)
    ) {
        let terminal = terminal.as_ref().ok_or_else(|| {
            "TaskTerminalBound fixture requires a terminal Task record".to_owned()
        })?;
        links
            .seed_task_terminal_bounds_bulk_for_test(
                vec![(
                    link_key,
                    link,
                    projection,
                    record.version,
                    epoch_ms,
                    terminal.digest().clone(),
                )],
                deadline,
            )
            .map_err(|error| format!("seed terminal lifecycle link: {error}"))?;
        return Ok(record_key);
    }
    let reservation = links
        .reserve_task_link(link_key.clone(), link, deadline)
        .map_err(|error| format!("reserve seeded lifecycle link: {error}"))?;
    links
        .materialize_task_bound(
            &reservation,
            projection,
            record.version,
            epoch_ms,
            phase,
            deadline,
        )
        .map_err(|error| format!("materialize seeded lifecycle link: {error}"))?;
    Ok(record_key)
}

fn seed_task_link_reservation(
    state_root: &Path,
    identity: &CoreIdentity,
    exact_key: &ReceiptKey,
    relation: ScenarioIdentityRelation,
) -> Result<(), String> {
    let key = if matches!(relation, ScenarioIdentityRelation::Exact) {
        exact_key.clone()
    } else {
        scenario_foreign_key(exact_key)
    };
    let link = TaskLinkReference::new(
        receipt_key_digest(&key),
        key.reserved_task_id(),
        key.invocation_id(),
        scenario_workspace_identity_hash(),
    );
    let state = DaemonStateDirectory::open(state_root, identity)?;
    let root = state.create_private_retained_subdirectory("task-lifecycle-links")?;
    let deadline = crate::domain::code_intelligence::ProviderDeadline::new(
        Instant::now() + SCENARIO_OPERATION_TIMEOUT,
    );
    let store = TaskLifecycleLinkStoreV5::open(root.path(), deadline)
        .map_err(|error| format!("open seeded Task lifecycle-link reservation store: {error}"))?;
    store
        .reserve_task_link(key, link, deadline)
        .map_err(|error| format!("seed Task lifecycle-link reservation: {error}"))?;
    Ok(())
}

fn fill_linked_task_pool(
    state_root: &Path,
    identity: &CoreIdentity,
    clock: Arc<ScenarioEpochClock>,
    arguments: &Map<String, Value>,
    workspace_hint: &str,
    count: usize,
) -> Result<(Vec<ReceiptKey>, TaskProjectionObservation), String> {
    let epoch_ms = clock.now_epoch_millis();
    let workspace_identity_hash = scenario_workspace_identity_hash();
    let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Completed {
        result: Box::new(DomainResult::success("linked-task-pool")),
    })
    .map_err(|error| format!("encode bulk linked Task terminal: {error}"))?;
    let mut keys = Vec::with_capacity(count);
    let mut tasks = Vec::with_capacity(count);
    let mut links = Vec::with_capacity(count);
    for _ in 0..count {
        let key = fresh_key_for_workspace(identity, arguments, workspace_hint)?;
        let task = ReceiptTaskProjection::new(
            key.reserved_task_id(),
            key.invocation_id(),
            epoch_ms,
            epoch_ms,
            SCENARIO_TASK_TTL_MS,
            SCENARIO_TASK_POLL_INTERVAL_MS,
            1,
        )
        .map_err(|error| format!("construct bulk linked Task projection: {error}"))?;
        let link = TaskLinkReference::new(
            receipt_key_digest(&key),
            key.reserved_task_id(),
            key.invocation_id(),
            workspace_identity_hash.clone(),
        );
        tasks.push(V5StoredInvocationRecord {
            schema_version: V5StoredInvocationSchemaVersion,
            task_id: key.reserved_task_id(),
            invocation_id: key.invocation_id(),
            receipt_key_digest: receipt_key_digest(&key),
            tool: key.tool(),
            normalized_arguments_hash: key.normalized_arguments_hash().clone(),
            workspace_identity_hash: workspace_identity_hash.clone(),
            created_at_epoch_ms: epoch_ms,
            updated_at_epoch_ms: epoch_ms,
            ttl_ms: SCENARIO_TASK_TTL_MS,
            poll_interval_ms: SCENARIO_TASK_POLL_INTERVAL_MS,
            version: 1,
            cancel_requested: false,
            task: V5StoredTask::Completed {
                terminal_epoch_ms: epoch_ms,
                terminal_digest: terminal.digest().clone(),
                result: Box::new(DomainResult::success("linked-task-pool")),
            },
        });
        links.push((
            key.clone(),
            link,
            task,
            1,
            epoch_ms,
            terminal.digest().clone(),
        ));
        keys.push(key);
    }
    let state = DaemonStateDirectory::open(state_root, identity)?;
    let task_root = state.create_private_retained_subdirectory("tasks")?;
    let task_deadline = crate::domain::code_intelligence::ProviderDeadline::new(
        Instant::now() + Duration::from_secs(30),
    );
    let (task_store, _) = FileInvocationStoreV5::open_retained_directory_inspect_only(
        task_root,
        clock,
        task_deadline,
    )
    .map_err(|error| format!("open bulk TaskStore fixture: {error}"))?;
    task_store
        .seed_exact_records_bulk_for_test(tasks.clone(), task_deadline)
        .map_err(|error| format!("seed bulk TaskStore fixture: {error}"))?;
    drop(task_store);

    let link_root = state.create_private_retained_subdirectory("task-lifecycle-links")?;
    let link_deadline = crate::domain::code_intelligence::ProviderDeadline::new(
        Instant::now() + Duration::from_secs(30),
    );
    let link_store = TaskLifecycleLinkStoreV5::open(link_root.path(), link_deadline)
        .map_err(|error| format!("open bulk lifecycle-link fixture: {error}"))?;
    let seeded_links = link_store
        .seed_task_terminal_bounds_bulk_for_test(links, link_deadline)
        .map_err(|error| format!("seed bulk lifecycle-link fixture: {error}"))?;
    let task_observations = keys
        .iter()
        .zip(tasks.iter())
        .map(|(key, record)| {
            task_observation_from_response_with_workspace(
                V5ServerResponse::Task {
                    snapshot: super::task_store_snapshot(record),
                },
                key,
                state_root,
                identity,
                Some(Some(workspace_identity_hash.as_str().to_owned())),
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let task_links = seeded_links
        .iter()
        .zip(tasks.iter())
        .map(|(bound, record)| {
            task_lifecycle_link_observation(
                &TaskLifecycleLinkRecord::TaskTerminalBound(bound.clone()),
                record,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let task_link_bytes = seeded_links
        .iter()
        .map(TaskTerminalBoundReceipt::encoded_bytes)
        .sum();
    let mutation_sequence = seeded_links
        .last()
        .map_or(0, TaskTerminalBoundReceipt::mutation_sequence);
    Ok((
        keys,
        TaskProjectionObservation {
            tasks: task_observations,
            task_links,
            task_link_count: u64::try_from(count)
                .map_err(|_| "bulk Task count exceeds u64".to_owned())?,
            task_link_bytes,
            task_link_reserved_count: 0,
            task_link_reserved_bytes: 0,
            task_store_mutations: u64::try_from(count)
                .map_err(|_| "bulk Task mutation count exceeds u64".to_owned())?,
            generation: mutation_sequence.saturating_add(
                u64::try_from(count)
                    .map_err(|_| "bulk Task generation count exceeds u64".to_owned())?,
            ),
        },
    ))
}

fn fill_tombstone_pool(
    state_root: &Path,
    identity: &CoreIdentity,
    clock: &ScenarioEpochClock,
    arguments: &Map<String, Value>,
    workspace_hint: &str,
) -> Result<(ReceiptLedgerActor, BulkReceiptCatalogObservation), String> {
    const TOMBSTONE_POOL_LIMIT: usize = 28_864;
    let keys = (0..TOMBSTONE_POOL_LIMIT)
        .map(|_| fresh_key_for_workspace(identity, arguments, workspace_hint))
        .collect::<Result<Vec<_>, _>>()?;
    let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Completed {
        result: Box::new(DomainResult::success("tombstone-fixture")),
    })
    .map_err(|error| format!("encode tombstone fixture terminal: {error}"))?;
    let state = DaemonStateDirectory::open(state_root, identity)?;
    let receipts = state.create_private_retained_subdirectory("receipts")?;
    let (actor, receipts) = seed_receipt_tombstones_for_scenario(
        receipts,
        keys,
        clock.now_epoch_millis(),
        terminal.digest().clone(),
        Instant::now() + Duration::from_secs(120),
    )?;
    let mut indexed_keys = receipts
        .iter()
        .map(|receipt| {
            (
                receipt.key_digest().as_str().to_owned(),
                receipt_key_observation(receipt.key()),
            )
        })
        .collect::<Vec<_>>();
    indexed_keys.sort_unstable_by(|(left, _), (right, _)| left.cmp(right));
    let tombstone_bytes = receipts
        .iter()
        .map(AcknowledgedTombstoneReceipt::encoded_bytes)
        .sum();
    let tombstones = receipts.iter().map(tombstone_observation).collect();
    Ok((
        actor,
        BulkReceiptCatalogObservation {
            tombstones,
            indexed_keys,
            tombstone_bytes,
        },
    ))
}

fn task_terminal_digest_from_store(
    state_root: &Path,
    identity: &CoreIdentity,
    key: &ReceiptKey,
) -> Result<TerminalDigest, String> {
    let state = DaemonStateDirectory::open(state_root, identity)?;
    let receipts = state.create_private_retained_subdirectory("receipts")?;
    let actor =
        open_receipt_actor_for_scenario(receipts, "open receipt-backed Task digest ledger")?;
    let state = actor
        .recover(key.clone(), Instant::now() + SCENARIO_OPERATION_TIMEOUT)
        .map_err(|error| format!("read receipt-backed Task terminal digest: {error}"))?;
    match state {
        ReceiptState::TaskTerminalReceiptBacked(receipt) => Ok(receipt.terminal().digest().clone()),
        other => Err(format!(
            "Task terminal digest requires receipt-backed terminal, found {}",
            other.kind().diagnostic_name()
        )),
    }
}

fn fresh_key(
    identity: &CoreIdentity,
    arguments: &Map<String, Value>,
) -> Result<ReceiptKey, String> {
    fresh_key_for_workspace(identity, arguments, "workspace-a")
}

fn fresh_key_for_workspace(
    identity: &CoreIdentity,
    arguments: &Map<String, Value>,
    workspace_hint: &str,
) -> Result<ReceiptKey, String> {
    Ok(ReceiptKey::new(
        InvocationId::new(),
        TaskId::new(),
        RequestIdentity::new(
            identity.digest().clone(),
            V5ToolIdentity::View,
            normalized_arguments_hash(arguments),
            request_scope_hash(workspace_hint)
                .map_err(|error| format!("construct receipt scenario request scope: {error}"))?,
        ),
    ))
}

fn push_known_key(keys: &mut Vec<ReceiptKey>, key: ReceiptKey) {
    if !keys.iter().any(|known| known == &key) {
        keys.push(key);
    }
}

fn merge_unique_values(target: &mut Vec<Value>, values: Vec<Value>) {
    for value in values {
        if !target.contains(&value) {
            target.push(value);
        }
    }
}

struct ScenarioInvocationService {
    control: Arc<ReceiptScenarioControl>,
    provider: ScenarioProviderFixture,
}

impl CanonicalInvocationService for ScenarioInvocationService {
    fn prepare(
        &self,
        _invocation: &ActorBoundInvocation,
    ) -> Result<ExecutionClass, Box<DomainResult>> {
        Ok(match self.provider.execution_class {
            ScenarioExecutionClass::Direct => ExecutionClass::InlineCandidate,
            ScenarioExecutionClass::KnownLong => {
                ExecutionClass::KnownLong(KnownLongReason::ExternalProcess)
            }
        })
    }

    fn execute(
        &self,
        _invocation: &ActorBoundExecution,
        cancellation: CancellationToken,
    ) -> Result<DomainResult, InvocationFailure> {
        if self.provider.cooperative_cancel && cancellation.is_cancelled() {
            return Err(InvocationFailure::new(
                "cancelled",
                "scenario provider observed cooperative cancellation",
            ));
        }
        if self.provider.side_effect_marker {
            self.control.record_side_effect_marker();
        }
        domain_result_for_fixture(&self.provider.terminal)
            .map_err(|message| InvocationFailure::new("invalid_fixture", message))
    }
}

fn domain_result_for_fixture(fixture: &ScenarioTerminalFixture) -> Result<DomainResult, String> {
    let exact_canonical_bytes = |target: u64| -> Result<DomainResult, String> {
        let empty = DomainResult::success("");
        let envelope_bytes = serde_json::to_vec(&empty)
            .map_err(|error| format!("encode empty scenario result: {error}"))?
            .len();
        let target = usize::try_from(target)
            .map_err(|_| "scenario canonical result size does not fit usize".to_owned())?;
        let payload_bytes = target.checked_sub(envelope_bytes).ok_or_else(|| {
            format!(
                "scenario canonical result target {target} is below envelope size {envelope_bytes}"
            )
        })?;
        Ok(DomainResult::success("x".repeat(payload_bytes)))
    };
    match fixture {
        ScenarioTerminalFixture::Success { payload } => Ok(DomainResult::success(payload.clone())),
        ScenarioTerminalFixture::Bytes { count } => exact_canonical_bytes(*count),
        ScenarioTerminalFixture::NearLimitWithMaximumMetadata {
            canonical_result_bytes,
        } => exact_canonical_bytes(*canonical_result_bytes),
    }
}

fn scenario_server_config(
    state_root: &Path,
    identity: &CoreIdentity,
    control: Option<&Arc<ReceiptScenarioControl>>,
) -> DaemonServerConfig {
    // Every scenario runtime observes through scenario hooks: a fresh
    // telemetry unless the runner shares its own later, and the control it
    // was handed, exactly as the runtime used to default them.
    let mut config = DaemonServerConfig::new(
        state_root.to_path_buf(),
        identity.clone(),
        SCENARIO_IDLE_GRACE,
    )
    .with_runtime_hooks_for_test(ScenarioHooks::install(
        Arc::new(V5ReceiptRuntimeTelemetry::new()),
        control.cloned(),
    ));
    if control.is_some_and(|control| control.take_skip_next_startup_reconciliation()) {
        config = config.without_v5_startup_reconciliation_for_test();
    }
    match control.and_then(|control| control.provider().map(|provider| (control, provider))) {
        Some((control, provider)) => {
            config.with_invocation_service(Arc::new(ScenarioInvocationService {
                control: Arc::clone(control),
                provider,
            }))
        }
        None => config,
    }
}

fn scenario_server_config_with_clock(
    state_root: &Path,
    identity: &CoreIdentity,
    control: Option<&Arc<ReceiptScenarioControl>>,
    clock: &Arc<ScenarioEpochClock>,
) -> DaemonServerConfig {
    let invocation_clock: Arc<dyn Clock> = clock.clone();
    let epoch_clock: Arc<dyn EpochMillisClock> = clock.clone();
    scenario_server_config(state_root, identity, control)
        .with_invocation_clock_for_test(invocation_clock)
        .with_v5_epoch_clock_for_test(epoch_clock)
}

/// Waits for a promoted attempt the runtime finished off the reply thread to
/// reach a stable point the harness can observe: the continuation ran to its
/// terminal, or it parked at an installed barrier or a gate the scenario has
/// yet to release. A no-op unless the owner promoted at the cutoff.
fn quiesce_promoted_continuation(
    state_root: &Path,
    identity: &CoreIdentity,
    control: &ReceiptScenarioControl,
    telemetry: &V5ReceiptRuntimeTelemetry,
) -> Result<(), String> {
    if let Some(runtime) = control.runtime() {
        let deadline = Instant::now() + SCENARIO_OPERATION_TIMEOUT;
        while runtime.promoted_continuation_in_flight_for_test() > 0 {
            if control.any_barrier_awaiting_release() || control.gate_cancel_waiting() {
                break;
            }
            if Instant::now() >= deadline {
                break;
            }
            thread::sleep(Duration::from_millis(2));
        }
    }
    // Clean up one fail-stop once. `restart_requested` is never cleared, so
    // without the latch every later quiesce would spend both deadlines again on
    // an exit that was already handled.
    if telemetry.snapshot().restart_requested && !control.fail_stop_reclaimed() {
        control.mark_fail_stop_reclaimed();
        // A fail-stopped attempt exits the process on the runtime's own clock;
        // wait for that close so a checkpoint sees the closed listener.
        let deadline = Instant::now() + SCENARIO_OPERATION_TIMEOUT;
        while telemetry.snapshot().listener != V5ReceiptRuntimeListenerState::Closed
            && Instant::now() < deadline
        {
            thread::sleep(Duration::from_millis(2));
        }
        // Production keeps the dead daemon's endpoint record until PID exit; a
        // successor process reclaims it. The harness has no successor, so once
        // the fail-stopped runtime has released (its detached workers ended and
        // dropped their `Arc`), reclaim the stale endpoint here so the next
        // in-thread daemon publishes a live one instead of the observer
        // connecting to a dead listener and spawning a real process.
        let release_deadline = Instant::now() + SCENARIO_OPERATION_TIMEOUT;
        while control.runtime().is_some() && Instant::now() < release_deadline {
            thread::sleep(Duration::from_millis(2));
        }
        if control.runtime().is_none() {
            if let Ok(daemon_state) = DaemonStateDirectory::open(state_root, identity) {
                if let Ok(Some(record)) = daemon_state.read_v5_endpoint_record() {
                    let _ = daemon_state.remove_matching_v5_endpoint_record(&record);
                }
            }
        }
    }
    Ok(())
}

fn exchange_once(
    state_root: &Path,
    identity: &CoreIdentity,
    clock: Arc<ScenarioEpochClock>,
    telemetry: Arc<V5ReceiptRuntimeTelemetry>,
    scenario_control: Option<Arc<ReceiptScenarioControl>>,
    exchange: impl FnOnce(&mut V5DaemonProcessOwner) -> Result<V5ServerResponse, String>,
) -> Result<V5ServerResponse, String> {
    let config =
        scenario_server_config_with_clock(state_root, identity, scenario_control.as_ref(), &clock);
    let daemon = ScenarioDaemon::spawn(config, move |runtime| {
        let mut runtime =
            runtime.with_hooks_for_test(ScenarioHooks::install(telemetry, scenario_control));
        runtime.epoch_clock = clock;
        runtime
    });
    let response = (|| {
        wait_for_endpoint(state_root, identity)?;
        let mut owner = V5DaemonProcessOwner::connect_or_spawn(
            state_root,
            identity.clone(),
            std::path::PathBuf::from("unused-existing-v5-scenario-endpoint"),
            SCENARIO_IDLE_GRACE,
        )?;
        let response = exchange(&mut owner);
        drop(owner);
        response
    })();
    let cleanup = daemon.stop_and_join("protocol-v5 receipt scenario daemon panicked");
    finish_with_daemon_cleanup(response, cleanup)
}

fn publish_listener_once(
    state_root: &Path,
    identity: &CoreIdentity,
    clock: Arc<ScenarioEpochClock>,
    telemetry: Arc<V5ReceiptRuntimeTelemetry>,
) -> Result<(), String> {
    let config = scenario_server_config_with_clock(state_root, identity, None, &clock);
    let daemon = ScenarioDaemon::spawn(config, move |runtime| {
        let mut runtime = runtime.with_hooks_for_test(ScenarioHooks::install(telemetry, None));
        runtime.epoch_clock = clock;
        runtime
    });
    let published = wait_for_endpoint(state_root, identity);
    let cleanup = daemon.stop_and_join("protocol-v5 listener publication daemon panicked");
    finish_with_daemon_cleanup(published, cleanup)
}

fn exchange_once_retaining_actor(
    state_root: &Path,
    identity: &CoreIdentity,
    clock: Arc<ScenarioEpochClock>,
    telemetry: Arc<V5ReceiptRuntimeTelemetry>,
    scenario_control: Option<Arc<ReceiptScenarioControl>>,
    exchange: impl FnOnce(&mut V5DaemonProcessOwner) -> Result<V5ServerResponse, String>,
) -> Result<(V5ServerResponse, ReceiptLedgerActor), String> {
    let config =
        scenario_server_config_with_clock(state_root, identity, scenario_control.as_ref(), &clock);
    let (actor_sender, actor_receiver) = mpsc::sync_channel(1);
    let daemon = ScenarioDaemon::spawn(config, move |runtime| {
        let _ = actor_sender.send(runtime.receipt_ledger.clone());
        let mut runtime =
            runtime.with_hooks_for_test(ScenarioHooks::install(telemetry, scenario_control));
        runtime.epoch_clock = clock;
        runtime
    });
    let response = (|| {
        wait_for_endpoint(state_root, identity)?;
        let actor = actor_receiver
            .recv_timeout(SCENARIO_ENDPOINT_STARTUP_TIMEOUT)
            .map_err(|_| "protocol-v5 retained receipt actor was not published".to_owned())?;
        let mut owner = V5DaemonProcessOwner::connect_or_spawn(
            state_root,
            identity.clone(),
            std::path::PathBuf::from("unused-existing-v5-scenario-endpoint"),
            SCENARIO_IDLE_GRACE,
        )?;
        let response = exchange(&mut owner)?;
        drop(owner);
        Ok((response, actor))
    })();
    let cleanup = daemon.stop_and_join("protocol-v5 retained-actor daemon panicked");
    finish_with_daemon_cleanup(response, cleanup)
}

fn exchange_raw_v5_request(
    state_root: &Path,
    identity: &CoreIdentity,
    clock: Arc<ScenarioEpochClock>,
    telemetry: Arc<V5ReceiptRuntimeTelemetry>,
    scenario_control: Option<Arc<ReceiptScenarioControl>>,
    request_frame: Vec<u8>,
) -> Result<V5ServerResponse, String> {
    let config =
        scenario_server_config_with_clock(state_root, identity, scenario_control.as_ref(), &clock);
    let daemon = ScenarioDaemon::spawn(config, move |runtime| {
        let mut runtime =
            runtime.with_hooks_for_test(ScenarioHooks::install(telemetry, scenario_control));
        runtime.epoch_clock = clock;
        runtime
    });
    let response = (|| {
        wait_for_endpoint(state_root, identity)?;
        let state = DaemonStateDirectory::open(state_root, identity)?;
        let record = state
            .read_v5_endpoint_record()?
            .ok_or_else(|| "protocol-v5 receipt scenario endpoint disappeared".to_owned())?;
        let handshake = V5DaemonProcessOwner::connect_existing_raw_for_test(
            record,
            DAEMON_PROTOCOL_VERSION,
            identity.clone(),
            uuid::Uuid::new_v4().to_string(),
            Instant::now() + SCENARIO_OPERATION_TIMEOUT,
        )?;
        let V5RawHandshake::Ready { mut owner, .. } = handshake else {
            return Err(
                "protocol-v5 strict-envelope handshake was unexpectedly rejected".to_owned(),
            );
        };
        let response_frame = owner.exchange_raw_frame(&request_frame, "strict envelope request")?;
        drop(owner);
        decode_v5_server_response(&response_frame)
    })();
    let cleanup = daemon.stop_and_join("protocol-v5 strict-envelope daemon panicked");
    finish_with_daemon_cleanup(response, cleanup)
}

fn run_protocol_probe_scenario(scenario: ReceiptScenario) -> Result<String, String> {
    let mut report = ScenarioReportBuilder::default();
    let mut events = Vec::new();
    for action in scenario.actions {
        let ReceiptScenarioAction::ProbeProtocol {
            client,
            server,
            message,
            label,
        } = action
        else {
            return Err(
                "protocol-v5 probe scenario contained a non-protocol action after filtering"
                    .to_owned(),
            );
        };
        let (observation, probe_events) =
            run_single_protocol_probe(client, server, message, label.clone())?;
        report.protocol.push(observation);
        events.extend(probe_events);
    }
    report.encode(events)
}

fn run_single_protocol_probe(
    client: ScenarioProtocolVersion,
    server: ScenarioProtocolVersion,
    message: ScenarioProtocolMessage,
    label: String,
) -> Result<(Value, Vec<V5ReceiptRuntimeEvent>), String> {
    match server {
        ScenarioProtocolVersion::V3 => run_v3_protocol_probe(client, message, label),
        ScenarioProtocolVersion::V4 => {
            Err("protocol-v4 cannot be selected as a production daemon".to_owned())
        }
        ScenarioProtocolVersion::V5 => run_v5_protocol_probe(client, message, label),
    }
}

fn run_v5_protocol_probe(
    client: ScenarioProtocolVersion,
    message: ScenarioProtocolMessage,
    label: String,
) -> Result<(Value, Vec<V5ReceiptRuntimeEvent>), String> {
    let root =
        tempfile::tempdir().map_err(|error| format!("create protocol-v5 probe state: {error}"))?;
    let state_root = std::fs::canonicalize(root.path())
        .map_err(|error| format!("canonicalize protocol-v5 probe state: {error}"))?;
    let identity = CoreIdentity::production_v5();
    let presented_core_identity = presented_core_identity(client, &message, &identity)?;
    let prepared = prepare_v5_protocol_probe(&state_root, &identity, &message)?;
    let request_frame = prepared.request_frame;
    let response_override = prepared.response_override;
    let delivery = prepared.delivery;
    let telemetry = Arc::new(V5ReceiptRuntimeTelemetry::new());
    let daemon = ScenarioDaemon::spawn(
        DaemonServerConfig::new(state_root.clone(), identity.clone(), SCENARIO_IDLE_GRACE),
        {
            let telemetry = Arc::clone(&telemetry);
            move |runtime| runtime.with_hooks_for_test(ScenarioHooks::install(telemetry, None))
        },
    );
    let result =
        (|| {
            wait_for_protocol_endpoint(&state_root, &identity)?;
            let state = DaemonStateDirectory::open(&state_root, &identity)?;
            let record = state
                .read_v5_endpoint_record()?
                .ok_or_else(|| "protocol-v5 probe endpoint disappeared".to_owned())?;
            let handshake = V5DaemonProcessOwner::connect_existing_raw_for_test(
                record,
                protocol_version_number(client),
                presented_core_identity.clone(),
                Uuid::new_v4().to_string(),
                Instant::now() + SCENARIO_OPERATION_TIMEOUT,
            )?;
            match handshake {
                V5RawHandshake::Ready {
                    mut owner,
                    client_hello_frame,
                    server_ready_frame,
                } => {
                    let transport_response =
                        owner.exchange_raw_frame(&request_frame, "protocol probe")?;
                    drop(owner);
                    decode_v5_server_response(&transport_response)?;
                    let response_frame = response_override.unwrap_or(transport_response);
                    let response_payload = response_frame
                        .strip_suffix(b"\n")
                        .and_then(|frame| frame.strip_suffix(b"\r").or(Some(frame)))
                        .unwrap_or(&response_frame);
                    let response = decode_v5_server_response(response_payload)?;
                    let error = response_error_value_v5(&response)?;
                    Ok(protocol_probe_observation(
                        &label,
                        client,
                        ScenarioProtocolVersion::V5,
                        ProtocolProbeFrames {
                            client_hello: client_hello_frame,
                            server_ready: Some(server_ready_frame),
                            client_write: request_frame.clone(),
                            server_read: Some(request_frame),
                            server_write: response_frame.clone(),
                            client_read: response_frame,
                        },
                        ProtocolProbeTrace {
                            spawned_argv_hex: spawned_daemon_argv_hex(&state_root, &identity),
                            daemon_process_events: daemon_process_events(
                                ScenarioProtocolVersion::V5,
                                true,
                                service_capability_fingerprint_for(&message).is_some(),
                            ),
                            production_events: production_events(true, error.is_none()),
                            error,
                            service_capability_fingerprint: service_capability_fingerprint_for(
                                &message,
                            ),
                            delivery,
                        },
                        &presented_core_identity,
                    ))
                }
                V5RawHandshake::Rejected {
                    client_hello_frame,
                    server_response_frame,
                    code,
                } => Ok(protocol_probe_observation(
                    &label,
                    client,
                    ScenarioProtocolVersion::V5,
                    ProtocolProbeFrames {
                        client_hello: client_hello_frame,
                        server_ready: None,
                        client_write: request_frame,
                        server_read: None,
                        server_write: server_response_frame.clone(),
                        client_read: server_response_frame,
                    },
                    ProtocolProbeTrace {
                        spawned_argv_hex: spawned_daemon_argv_hex(&state_root, &identity),
                        daemon_process_events: daemon_process_events(
                            ScenarioProtocolVersion::V5,
                            false,
                            false,
                        ),
                        production_events: production_events(false, false),
                        error: Some(serde_json::to_value(code).map_err(|error| {
                            format!("encode protocol-v5 rejection code: {error}")
                        })?),
                        service_capability_fingerprint: None,
                        delivery: None,
                    },
                    &presented_core_identity,
                )),
            }
        })();
    let cleanup = daemon.stop_and_join("protocol-v5 probe daemon panicked");
    let observation = finish_with_daemon_cleanup(result, cleanup)?;
    let snapshot = telemetry.snapshot();
    Ok((observation, snapshot.events))
}

struct PreparedV5ProtocolProbe {
    request_frame: Vec<u8>,
    response_override: Option<Vec<u8>>,
    delivery: Option<Value>,
}

fn prepare_v5_protocol_probe(
    state_root: &Path,
    identity: &CoreIdentity,
    message: &ScenarioProtocolMessage,
) -> Result<PreparedV5ProtocolProbe, String> {
    let request_frame = build_v5_probe_request_frame(state_root, identity, message)?;
    let response_override = fixture_v5_response(identity, message)?;
    let delivery = response_override
        .as_deref()
        .map(|frame| {
            let payload = frame
                .strip_suffix(b"\n")
                .and_then(|frame| frame.strip_suffix(b"\r").or(Some(frame)))
                .unwrap_or(frame);
            let response = decode_v5_server_response(payload)?;
            projection_delivery_for_probe(identity, message, &response, frame)
        })
        .transpose()?
        .flatten();
    Ok(PreparedV5ProtocolProbe {
        request_frame,
        response_override,
        delivery,
    })
}

fn fixture_v5_response(
    identity: &CoreIdentity,
    message: &ScenarioProtocolMessage,
) -> Result<Option<Vec<u8>>, String> {
    let response = match message {
        ScenarioProtocolMessage::ReceiptPendingOutcome => {
            let key = fresh_key(identity, &Map::new())?;
            Some(V5ServerResponse::Invocation {
                outcome: V5InvocationResponse::ReceiptPending {
                    receipt_key: key,
                    phase: V5InvocationPhase::ReservedUnbound,
                    accepted_epoch_ms: SCENARIO_INITIAL_EPOCH_MS,
                    original_budget_ms: 7_000,
                    cancel_requested: false,
                },
            })
        }
        ScenarioProtocolMessage::TaskOutcome => {
            let key = fresh_key(identity, &Map::new())?;
            Some(V5ServerResponse::Invocation {
                outcome: V5InvocationResponse::Task {
                    snapshot: completed_probe_task(&key, true)?,
                },
            })
        }
        ScenarioProtocolMessage::AcknowledgedOutcome => {
            let key = fresh_key(identity, &Map::new())?;
            let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Completed {
                result: Box::new(DomainResult::success("canonical-success")),
            })
            .map_err(|error| format!("construct acknowledged protocol-v5 terminal: {error}"))?;
            let tombstone = AcknowledgedTombstoneReceipt::new(
                key.clone(),
                receipt_key_digest(&key),
                terminal.digest().clone(),
                SCENARIO_INITIAL_EPOCH_MS,
                1,
            )
            .map_err(|error| format!("construct acknowledged protocol-v5 receipt: {error}"))?;
            Some(V5ServerResponse::Invocation {
                outcome: V5InvocationResponse::Acknowledged {
                    acknowledgement: V5AcknowledgedReceipt::from_receipt(&tombstone),
                },
            })
        }
        ScenarioProtocolMessage::DirectCompletedTerminal => Some(direct_probe_response(
            identity,
            ReceiptTerminalOutcome::Completed {
                result: Box::new(DomainResult::success("canonical-success")),
            },
        )?),
        ScenarioProtocolMessage::DirectSemanticCompletedTerminal => {
            let mut result = DomainResult::success("canonical-semantic-failure");
            result.ok = false;
            Some(direct_probe_response(
                identity,
                ReceiptTerminalOutcome::Completed {
                    result: Box::new(result),
                },
            )?)
        }
        ScenarioProtocolMessage::DirectCancelledTerminal => Some(direct_probe_response(
            identity,
            ReceiptTerminalOutcome::Cancelled,
        )?),
        ScenarioProtocolMessage::DirectFailureTerminal { reason } => Some(direct_probe_response(
            identity,
            ReceiptTerminalOutcome::Failed {
                reason: fixture_failure_reason(*reason),
            },
        )?),
        ScenarioProtocolMessage::TaskQueuedProjection => Some(V5ServerResponse::Task {
            snapshot: nonterminal_probe_task(identity, false)?,
        }),
        ScenarioProtocolMessage::TaskWorkingProjection => Some(V5ServerResponse::Task {
            snapshot: nonterminal_probe_task(identity, true)?,
        }),
        ScenarioProtocolMessage::TaskCompletedProjection => {
            let key = fresh_key(identity, &Map::new())?;
            Some(V5ServerResponse::Task {
                snapshot: completed_probe_task(&key, true)?,
            })
        }
        ScenarioProtocolMessage::TaskSemanticCompletedProjection { .. } => {
            let key = fresh_key(identity, &Map::new())?;
            Some(V5ServerResponse::Task {
                snapshot: completed_probe_task(&key, false)?,
            })
        }
        ScenarioProtocolMessage::TaskCancelledProjection => {
            let key = fresh_key(identity, &Map::new())?;
            let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Cancelled)
                .map_err(|error| format!("construct cancelled protocol-v5 Task: {error}"))?;
            Some(V5ServerResponse::Task {
                snapshot: V5DaemonTaskSnapshot::Cancelled {
                    task_id: key.reserved_task_id(),
                    invocation_id: key.invocation_id(),
                    receipt_key_digest: receipt_key_digest(&key),
                    created_at_epoch_ms: SCENARIO_INITIAL_EPOCH_MS,
                    updated_at_epoch_ms: SCENARIO_INITIAL_EPOCH_MS,
                    ttl_ms: SCENARIO_TASK_TTL_MS,
                    poll_interval_ms: SCENARIO_TASK_POLL_INTERVAL_MS,
                    version: 1,
                    cancel_requested: true,
                    terminal_epoch_ms: SCENARIO_INITIAL_EPOCH_MS,
                    terminal_digest: terminal.digest().clone(),
                },
            })
        }
        ScenarioProtocolMessage::TaskFailureProjection { reason } => {
            let key = fresh_key(identity, &Map::new())?;
            let reason = fixture_failure_reason(*reason);
            let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Failed { reason })
                .map_err(|error| format!("construct failed protocol-v5 Task: {error}"))?;
            Some(V5ServerResponse::Task {
                snapshot: V5DaemonTaskSnapshot::Failed {
                    task_id: key.reserved_task_id(),
                    invocation_id: key.invocation_id(),
                    receipt_key_digest: receipt_key_digest(&key),
                    created_at_epoch_ms: SCENARIO_INITIAL_EPOCH_MS,
                    updated_at_epoch_ms: SCENARIO_INITIAL_EPOCH_MS,
                    ttl_ms: SCENARIO_TASK_TTL_MS,
                    poll_interval_ms: SCENARIO_TASK_POLL_INTERVAL_MS,
                    version: 1,
                    cancel_requested: false,
                    terminal_epoch_ms: SCENARIO_INITIAL_EPOCH_MS,
                    terminal_digest: terminal.digest().clone(),
                    reason,
                },
            })
        }
        ScenarioProtocolMessage::ErrorCodeFrame { code } => Some(V5ServerResponse::Error {
            code: fixture_daemon_error_code(*code),
        }),
        ScenarioProtocolMessage::MaximumResponseFrame => {
            return maximum_v5_response_frame(false).map(Some)
        }
        ScenarioProtocolMessage::OversizedResponseFrame => {
            maximum_v5_response_frame(true)?;
            Some(V5ServerResponse::Error {
                code: V5DaemonErrorCode::InvalidRequest,
            })
        }
        _ => None,
    };
    response
        .as_ref()
        .map(encode_strict_v5_response_jsonl)
        .transpose()
}

fn fixture_failure_reason(reason: ScenarioFailureProbeReason) -> V5SafeFailureReason {
    match reason {
        ScenarioFailureProbeReason::InvocationFailed => V5SafeFailureReason::InvocationFailed,
        ScenarioFailureProbeReason::ResultTooLarge => V5SafeFailureReason::ResultTooLarge,
        ScenarioFailureProbeReason::Interrupted => V5SafeFailureReason::Interrupted,
        ScenarioFailureProbeReason::ResumeUnsupported => V5SafeFailureReason::ResumeUnsupported,
        ScenarioFailureProbeReason::PersistenceFailed => V5SafeFailureReason::PersistenceFailed,
        ScenarioFailureProbeReason::OutcomeUncertain => V5SafeFailureReason::OutcomeUncertain,
        ScenarioFailureProbeReason::TaskCapacity => V5SafeFailureReason::TaskCapacity,
        ScenarioFailureProbeReason::WorkspaceCapacity => V5SafeFailureReason::WorkspaceCapacity,
        ScenarioFailureProbeReason::WorkspaceRegistryFailed => {
            V5SafeFailureReason::WorkspaceRegistryFailed
        }
    }
}

fn direct_probe_response(
    identity: &CoreIdentity,
    outcome: ReceiptTerminalOutcome,
) -> Result<V5ServerResponse, String> {
    let key = fresh_key(identity, &Map::new())?;
    let terminal = canonical_v5_terminal(&outcome)
        .map_err(|error| format!("construct direct protocol-v5 terminal: {error}"))?;
    Ok(V5ServerResponse::Invocation {
        outcome: V5InvocationResponse::Direct {
            receipt: V5PendingDirectReceipt::new(
                key,
                outcome,
                terminal.digest().clone(),
                SCENARIO_INITIAL_EPOCH_MS,
            ),
        },
    })
}

fn nonterminal_probe_task(
    identity: &CoreIdentity,
    working: bool,
) -> Result<V5DaemonTaskSnapshot, String> {
    let key = fresh_key(identity, &Map::new())?;
    let common = (
        key.reserved_task_id(),
        key.invocation_id(),
        receipt_key_digest(&key),
    );
    Ok(if working {
        V5DaemonTaskSnapshot::Working {
            task_id: common.0,
            invocation_id: common.1,
            receipt_key_digest: common.2,
            created_at_epoch_ms: SCENARIO_INITIAL_EPOCH_MS,
            updated_at_epoch_ms: SCENARIO_INITIAL_EPOCH_MS,
            ttl_ms: SCENARIO_TASK_TTL_MS,
            poll_interval_ms: SCENARIO_TASK_POLL_INTERVAL_MS,
            version: 1,
            cancel_requested: false,
        }
    } else {
        V5DaemonTaskSnapshot::Queued {
            task_id: common.0,
            invocation_id: common.1,
            receipt_key_digest: common.2,
            created_at_epoch_ms: SCENARIO_INITIAL_EPOCH_MS,
            updated_at_epoch_ms: SCENARIO_INITIAL_EPOCH_MS,
            ttl_ms: SCENARIO_TASK_TTL_MS,
            poll_interval_ms: SCENARIO_TASK_POLL_INTERVAL_MS,
            version: 1,
            cancel_requested: false,
        }
    })
}

fn projection_delivery_for_probe(
    identity: &CoreIdentity,
    message: &ScenarioProtocolMessage,
    response: &V5ServerResponse,
    response_frame: &[u8],
) -> Result<Option<Value>, String> {
    match (message, response) {
        (
            ScenarioProtocolMessage::DirectCompletedTerminal
            | ScenarioProtocolMessage::DirectSemanticCompletedTerminal
            | ScenarioProtocolMessage::DirectCancelledTerminal
            | ScenarioProtocolMessage::DirectFailureTerminal { .. },
            V5ServerResponse::Invocation {
                outcome: V5InvocationResponse::Direct { receipt },
            },
        ) => direct_projection_delivery(receipt).map(Some),
        (
            ScenarioProtocolMessage::TaskQueuedProjection
            | ScenarioProtocolMessage::TaskWorkingProjection
            | ScenarioProtocolMessage::TaskCompletedProjection
            | ScenarioProtocolMessage::TaskSemanticCompletedProjection { .. }
            | ScenarioProtocolMessage::TaskCancelledProjection
            | ScenarioProtocolMessage::TaskFailureProjection { .. },
            V5ServerResponse::Task { snapshot },
        ) => task_projection_delivery(identity, message, snapshot, response_frame).map(Some),
        _ => Ok(None),
    }
}

fn direct_projection_delivery(receipt: &V5PendingDirectReceipt) -> Result<Value, String> {
    let pending = serde_json::to_vec(receipt)
        .map_err(|error| format!("encode pending protocol-v5 Direct receipt: {error}"))?;
    let terminal = terminal_observation(receipt.terminal(), receipt.terminal_epoch_ms())?;
    let (final_call_tool_result_hex, final_error_data_hex) = match receipt.terminal() {
        ReceiptTerminalOutcome::Completed { result } => (
            Some(json_hex(&json!({
                "resultType": "complete",
                "content": [],
                "structuredContent": result,
                "isError": !result.ok,
            }))?),
            None,
        ),
        ReceiptTerminalOutcome::Failed { reason } => {
            let (code, message) = failure_projection(*reason);
            (
                None,
                Some(json_hex(&json!({
                    "code": -32603,
                    "message": message,
                    "data": {"code": code},
                }))?),
            )
        }
        ReceiptTerminalOutcome::Cancelled => (
            None,
            Some(json_hex(&json!({
                "code": -32603,
                "message": "daemon invocation was cancelled",
                "data": {"code": "invocation_cancelled"},
            }))?),
        ),
    };
    Ok(json!({
        "pendingDirectReceiptHex": lower_hex(&pending),
        "directReceiptKey": receipt_key_observation(receipt.receipt_key()),
        "directTerminal": terminal,
        "internalTaskSnapshot": Value::Null,
        "storedInvocationRecord": Value::Null,
        "nativeMcpProjectionHex": Value::Null,
        "compatibilityGetProjectionHex": Value::Null,
        "compatibilityResultProjectionHex": Value::Null,
        "finalCallToolResultHex": final_call_tool_result_hex,
        "finalErrorDataHex": final_error_data_hex,
        "taskTerminalPublication": Value::Null,
        "events": [
            "terminal_preflighted",
            "pending_direct_receipt_built",
            "native_projection_built",
            "final_interface_value_built",
            "acknowledgement_written"
        ],
    }))
}

fn task_projection_delivery(
    identity: &CoreIdentity,
    message: &ScenarioProtocolMessage,
    snapshot: &V5DaemonTaskSnapshot,
    response_frame: &[u8],
) -> Result<Value, String> {
    let (
        task_id,
        invocation_id,
        key_digest,
        status,
        version,
        cancel_requested,
        terminal,
        stored_task,
    ) = match snapshot {
        V5DaemonTaskSnapshot::Queued {
            task_id,
            invocation_id,
            receipt_key_digest,
            version,
            cancel_requested,
            ..
        } => (
            *task_id,
            *invocation_id,
            receipt_key_digest.clone(),
            "queued",
            *version,
            *cancel_requested,
            None,
            V5StoredTask::Queued,
        ),
        V5DaemonTaskSnapshot::Working {
            task_id,
            invocation_id,
            receipt_key_digest,
            version,
            cancel_requested,
            ..
        } => (
            *task_id,
            *invocation_id,
            receipt_key_digest.clone(),
            "working",
            *version,
            *cancel_requested,
            None,
            V5StoredTask::Working,
        ),
        V5DaemonTaskSnapshot::Completed {
            task_id,
            invocation_id,
            receipt_key_digest,
            version,
            cancel_requested,
            terminal_epoch_ms,
            terminal_digest,
            result,
            ..
        } => {
            let outcome = ReceiptTerminalOutcome::Completed {
                result: result.clone(),
            };
            (
                *task_id,
                *invocation_id,
                receipt_key_digest.clone(),
                "completed",
                *version,
                *cancel_requested,
                Some(terminal_observation(&outcome, *terminal_epoch_ms)?),
                V5StoredTask::Completed {
                    terminal_epoch_ms: *terminal_epoch_ms,
                    terminal_digest: terminal_digest.clone(),
                    result: result.clone(),
                },
            )
        }
        V5DaemonTaskSnapshot::Failed {
            task_id,
            invocation_id,
            receipt_key_digest,
            version,
            cancel_requested,
            terminal_epoch_ms,
            terminal_digest,
            reason,
            ..
        } => {
            let outcome = ReceiptTerminalOutcome::Failed { reason: *reason };
            (
                *task_id,
                *invocation_id,
                receipt_key_digest.clone(),
                "failed",
                *version,
                *cancel_requested,
                Some(terminal_observation(&outcome, *terminal_epoch_ms)?),
                V5StoredTask::Failed {
                    terminal_epoch_ms: *terminal_epoch_ms,
                    terminal_digest: terminal_digest.clone(),
                    reason: *reason,
                },
            )
        }
        V5DaemonTaskSnapshot::Cancelled {
            task_id,
            invocation_id,
            receipt_key_digest,
            version,
            cancel_requested,
            terminal_epoch_ms,
            terminal_digest,
            ..
        } => (
            *task_id,
            *invocation_id,
            receipt_key_digest.clone(),
            "cancelled",
            *version,
            *cancel_requested,
            Some(terminal_observation(
                &ReceiptTerminalOutcome::Cancelled,
                *terminal_epoch_ms,
            )?),
            V5StoredTask::Cancelled {
                terminal_epoch_ms: *terminal_epoch_ms,
                terminal_digest: terminal_digest.clone(),
            },
        ),
    };
    let request_identity = RequestIdentity::new(
        identity.digest().clone(),
        V5ToolIdentity::View,
        normalized_arguments_hash(&Map::new()),
        request_scope_hash("workspace-a")
            .map_err(|error| format!("construct Task projection request scope: {error}"))?,
    );
    let key = ReceiptKey::new(invocation_id, task_id, request_identity);
    if receipt_key_digest(&key) != key_digest {
        return Err("Task projection receipt identity diverged from its wire digest".to_owned());
    }
    let workspace_identity_hash = SafeIdentityHash::from_sha256([0x42; 32]);
    let record = V5StoredInvocationRecord {
        schema_version: V5StoredInvocationSchemaVersion,
        task_id,
        invocation_id,
        receipt_key_digest: key_digest,
        tool: key.tool(),
        normalized_arguments_hash: key.normalized_arguments_hash().clone(),
        workspace_identity_hash: workspace_identity_hash.clone(),
        created_at_epoch_ms: SCENARIO_INITIAL_EPOCH_MS,
        updated_at_epoch_ms: SCENARIO_INITIAL_EPOCH_MS,
        ttl_ms: SCENARIO_TASK_TTL_MS,
        poll_interval_ms: SCENARIO_TASK_POLL_INTERVAL_MS,
        version,
        cancel_requested,
        task: stored_task,
    };
    let stored_bytes = serde_json::to_vec(&record)
        .map_err(|error| format!("encode protocol-v5 Task record evidence: {error}"))?;
    let stored_evidence = artifact_evidence(&stored_bytes);
    let projection_source = match message {
        ScenarioProtocolMessage::TaskSemanticCompletedProjection {
            owner: ScenarioTaskTerminalOwnerFixture::ReceiptBacked,
        } => "receipt_ledger",
        _ => "task_store",
    };
    let task = json!({
        "taskId": task_id,
        "invocationId": invocation_id,
        "receiptKey": receipt_key_observation(&key),
        "status": status,
        "projectionSource": projection_source,
        "workspaceIdentityHash": workspace_identity_hash,
        "createdEpochMs": SCENARIO_INITIAL_EPOCH_MS,
        "updatedEpochMs": SCENARIO_INITIAL_EPOCH_MS,
        "expiresEpochMs": SCENARIO_INITIAL_EPOCH_MS + SCENARIO_TASK_TTL_MS,
        "ttlMs": SCENARIO_TASK_TTL_MS,
        "pollIntervalMs": SCENARIO_TASK_POLL_INTERVAL_MS,
        "version": version,
        "encodedBytes": stored_bytes.len(),
        "cancelRequested": cancel_requested,
        "terminal": terminal,
    });
    let native = native_task_projection(status, task_id, &record)?;
    let (compatibility_get, compatibility_result) = compatibility_task_projections(
        status,
        task_id,
        invocation_id,
        version,
        cancel_requested,
        &record,
    )?;
    let receipt_backed = projection_source == "receipt_ledger";
    let terminal_publication = match (message, task.get("terminal")) {
        (
            ScenarioProtocolMessage::TaskSemanticCompletedProjection { owner },
            Some(terminal @ Value::Object(_)),
        ) => Some(task_terminal_publication_evidence(
            *owner,
            &key,
            terminal,
            &record,
            &stored_evidence,
            response_frame,
        )?),
        _ => None,
    };
    Ok(json!({
        "pendingDirectReceiptHex": Value::Null,
        "directReceiptKey": Value::Null,
        "directTerminal": Value::Null,
        "internalTaskSnapshot": task,
        "storedInvocationRecord": if receipt_backed { Value::Null } else { stored_evidence },
        "nativeMcpProjectionHex": json_hex(&native)?,
        "compatibilityGetProjectionHex": json_hex(&compatibility_get)?,
        "compatibilityResultProjectionHex": json_hex(&compatibility_result)?,
        "finalCallToolResultHex": Value::Null,
        "finalErrorDataHex": Value::Null,
        "taskTerminalPublication": terminal_publication,
        "events": [
            "native_projection_built",
            "compatibility_get_projection_built",
            "compatibility_result_projection_built"
        ],
    }))
}

fn task_terminal_publication_evidence(
    owner: ScenarioTaskTerminalOwnerFixture,
    key: &ReceiptKey,
    terminal: &Value,
    record: &V5StoredInvocationRecord,
    task_record: &Value,
    response_frame: &[u8],
) -> Result<Value, String> {
    let canonical = match &record.task {
        V5StoredTask::Completed { result, .. } => {
            canonical_v5_terminal(&ReceiptTerminalOutcome::Completed {
                result: result.clone(),
            })
        }
        V5StoredTask::Failed { reason, .. } => {
            canonical_v5_terminal(&ReceiptTerminalOutcome::Failed { reason: *reason })
        }
        V5StoredTask::Cancelled { .. } => canonical_v5_terminal(&ReceiptTerminalOutcome::Cancelled),
        V5StoredTask::Queued | V5StoredTask::Working => {
            return Err("nonterminal Task cannot expose terminal publication evidence".to_owned())
        }
    }
    .map_err(|error| format!("canonicalize Task publication terminal: {error}"))?;
    let candidate_result = match &record.task {
        V5StoredTask::Completed { result, .. } => Some(artifact_evidence(
            &serde_json::to_vec(result)
                .map_err(|error| format!("encode Task publication candidate: {error}"))?,
        )),
        _ => None,
    };
    let terminal_payload = artifact_evidence(canonical.payload());
    let response_artifact = artifact_evidence(response_frame);
    let task_link_digest = lower_hex(&Sha256::digest(
        format!(
            "{}:{}:task-link-v5",
            key.invocation_id(),
            key.reserved_task_id()
        )
        .as_bytes(),
    ));
    let link_record = artifact_evidence(
        &serde_json::to_vec(&json!({
            "schemaVersion": 1,
            "taskId": key.reserved_task_id(),
            "invocationId": key.invocation_id(),
            "receiptKeyDigest": receipt_key_digest(key),
            "linkDigest": task_link_digest,
            "terminalDigest": canonical.digest(),
            "taskVersion": record.version,
        }))
        .map_err(|error| format!("encode Task lifecycle-link evidence: {error}"))?,
    );
    let commit = match owner {
        ScenarioTaskTerminalOwnerFixture::ReceiptBacked => {
            let receipt_record_bytes = serde_json::to_vec(&json!({
                "schemaVersion": 1,
                "receiptKeyDigest": receipt_key_digest(key),
                "terminalDigest": canonical.digest(),
                "terminalEpochMs": SCENARIO_INITIAL_EPOCH_MS,
                "result": match &record.task {
                    V5StoredTask::Completed { result, .. } => serde_json::to_value(result)
                        .map_err(|error| format!("encode receipt-backed Task result: {error}"))?,
                    _ => Value::Null,
                },
            }))
            .map_err(|error| format!("encode receipt-backed Task record: {error}"))?;
            json!({
                "owner": "receipt_backed_task",
                "receipt": {
                    "terminalPayload": terminal_payload,
                    "receiptRecord": artifact_evidence(&receipt_record_bytes),
                    "candidateResult": candidate_result,
                    "terminalPayloadPreparedSequence": 1,
                    "receiptRecordPreparedSequence": 2,
                    "receiptCommitSequence": 4,
                    "receiptExpectedVersion": 1,
                }
            })
        }
        ScenarioTaskTerminalOwnerFixture::Bound => json!({
            "owner": "bound_task_store",
            "task": {
                "terminalPayload": terminal_payload,
                "candidateResult": candidate_result,
                "terminalPayloadPreparedSequence": 1,
                "taskRecord": task_record,
                "taskRecordPreparedSequence": 2,
                "taskStoreCommitSequence": 4,
                "taskStoreReadbackSequence": 5,
                "taskExpectedVersion": 1,
                "lifecycleLinkRecord": link_record,
                "lifecycleLinkRecordPreparedSequence": 6,
                "lifecycleLinkCommitSequence": 7,
                "committedLifecycleLinkVersion": 2,
                "lifecycleLinkExpectedVersion": 1,
                "taskLinkDigest": task_link_digest,
            }
        }),
        ScenarioTaskTerminalOwnerFixture::Staged => json!({
            "owner": "staged_handoff_task",
            "task": {
                "terminalPayload": terminal_payload,
                "candidateResult": candidate_result,
                "terminalPayloadPreparedSequence": 1,
                "taskRecord": task_record,
                "taskRecordPreparedSequence": 2,
                "taskStoreCommitSequence": 4,
                "taskStoreReadbackSequence": 5,
                "terminalWriteExpectation": {"state": "absent", "task_store_generation": 1},
                "terminalWriteBranch": "created_terminal",
                "idempotentRepeat": Value::Null,
                "committedTaskVersion": record.version,
                "lifecycleLinkRecord": link_record,
                "lifecycleLinkRecordPreparedSequence": 6,
                "lifecycleLinkCommitSequence": 7,
                "committedLifecycleLinkVersion": 2,
                "liveTaskLinkReservationFingerprint": lower_hex(&Sha256::digest(b"live-task-link-reservation")),
                "taskLinkDigest": task_link_digest,
                "stagedReceiptVersion": 1,
                "stagedReceiptRecordSha256": lower_hex(&Sha256::digest(b"staged-receipt-record")),
                "stagedTerminalDigest": canonical.digest(),
                "transferSizeCertificateSha256": lower_hex(&Sha256::digest(b"transfer-size-certificate")),
            }
        }),
    };
    Ok(json!({
        "receiptKey": receipt_key_observation(key),
        "terminal": terminal,
        "commit": commit,
        "responseFrames": [{
            "responseKind": "task",
            "origin": "immediate_publication",
            "responseJsonl": response_artifact,
            "preparedSequence": 3,
            "writeSequence": 8,
        }],
    }))
}

fn native_task_projection(
    status: &str,
    task_id: TaskId,
    record: &V5StoredInvocationRecord,
) -> Result<Value, String> {
    let epoch = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(
        i64::try_from(SCENARIO_INITIAL_EPOCH_MS)
            .map_err(|_| "protocol-v5 scenario epoch exceeds i64".to_owned())?,
    )
    .ok_or_else(|| "protocol-v5 scenario epoch is not representable".to_owned())?
    .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let mut value = json!({
        "taskId": task_id,
        "status": if matches!(status, "queued" | "working") { "working" } else { status },
        "createdAt": epoch,
        "lastUpdatedAt": epoch,
        "ttlMs": SCENARIO_TASK_TTL_MS,
        "pollIntervalMs": SCENARIO_TASK_POLL_INTERVAL_MS,
    });
    match &record.task {
        V5StoredTask::Completed { result, .. } => {
            value["result"] = json!({
                "resultType": "complete",
                "content": [],
                "structuredContent": result,
                "isError": !result.ok,
            });
        }
        V5StoredTask::Failed { reason, .. } => {
            let (code, message) = failure_projection(*reason);
            value["error"] = json!({
                "code": -32603,
                "message": message,
                "data": {"code": code},
            });
        }
        V5StoredTask::Queued | V5StoredTask::Working | V5StoredTask::Cancelled { .. } => {}
    }
    Ok(value)
}

fn compatibility_task_projections(
    status: &str,
    task_id: TaskId,
    invocation_id: InvocationId,
    version: u64,
    cancel_requested: bool,
    record: &V5StoredInvocationRecord,
) -> Result<(Value, Value), String> {
    let mut task = json!({
        "taskId": task_id,
        "invocationId": invocation_id,
        "createdAtEpochMs": SCENARIO_INITIAL_EPOCH_MS,
        "updatedAtEpochMs": SCENARIO_INITIAL_EPOCH_MS,
        "ttlMs": SCENARIO_TASK_TTL_MS,
        "pollIntervalMs": SCENARIO_TASK_POLL_INTERVAL_MS,
        "version": version,
        "cancelRequested": cancel_requested,
        "status": status,
    });
    if let Some(digest) = record.task.terminal_digest() {
        task["terminalEpochMs"] = SCENARIO_INITIAL_EPOCH_MS.into();
        task["terminalDigest"] = digest.to_string().into();
    }
    let state = match &record.task {
        V5StoredTask::Queued | V5StoredTask::Working => json!({
            "ok": true,
            "summary": "Task is still working",
            "data": {"task": task},
            "next": [{
                "tool": "unica.task.result",
                "args": {"taskId": task_id, "waitMs": SCENARIO_TASK_POLL_INTERVAL_MS}
            }],
        }),
        V5StoredTask::Completed { .. } => json!({
            "ok": true,
            "summary": "Task completed",
            "data": {"task": task},
        }),
        V5StoredTask::Failed { reason, .. } => {
            let (code, message) = failure_projection(*reason);
            json!({"ok": false, "summary": message, "data": {"code": code, "task": task}})
        }
        V5StoredTask::Cancelled { .. } => json!({
            "ok": false,
            "summary": "Task was cancelled",
            "data": {"code": "task_cancelled", "task": task},
        }),
    };
    let result = match &record.task {
        V5StoredTask::Completed { result, .. } => serde_json::to_value(result)
            .map_err(|error| format!("encode completed compatibility Task result: {error}"))?,
        _ => state.clone(),
    };
    Ok((state, result))
}

fn failure_projection(reason: V5SafeFailureReason) -> (&'static str, &'static str) {
    match reason {
        V5SafeFailureReason::InvocationFailed => ("invocation_failed", "daemon invocation failed"),
        V5SafeFailureReason::ResultTooLarge => (
            "result_too_large",
            "daemon invocation result exceeded the canonical byte limit",
        ),
        V5SafeFailureReason::Interrupted => ("interrupted", "daemon invocation was interrupted"),
        V5SafeFailureReason::ResumeUnsupported => (
            "resume_unsupported",
            "daemon invocation cannot be resumed after restart",
        ),
        V5SafeFailureReason::PersistenceFailed => (
            "persistence_failed",
            "daemon invocation terminal state could not be persisted",
        ),
        V5SafeFailureReason::OutcomeUncertain => (
            "outcome_uncertain",
            "daemon invocation outcome is uncertain",
        ),
        V5SafeFailureReason::TaskCapacity => (
            "task_capacity",
            "daemon Task capacity was exhausted before execution",
        ),
        V5SafeFailureReason::WorkspaceCapacity => {
            ("workspace_capacity", "workspace capacity was exhausted")
        }
        V5SafeFailureReason::WorkspaceRegistryFailed => (
            "workspace_registry_failed",
            "workspace registry is unavailable",
        ),
    }
}

fn artifact_evidence(bytes: &[u8]) -> Value {
    json!({
        "rawHex": lower_hex(bytes),
        "encodedBytes": bytes.len(),
        "sha256": lower_hex(&Sha256::digest(bytes)),
    })
}

fn json_bytes_with_exact_len(mut value: Value, expected_len: u64) -> Result<Vec<u8>, String> {
    let expected_len = usize::try_from(expected_len)
        .map_err(|_| "persisted artifact length does not fit usize".to_owned())?;
    value
        .as_object_mut()
        .ok_or_else(|| "persisted artifact sizing requires a JSON object".to_owned())?
        .insert("padding".to_owned(), Value::String(String::new()));
    let encoded = serde_json::to_vec(&value)
        .map_err(|error| format!("encode sized persisted artifact: {error}"))?;
    if encoded.len() > expected_len {
        return Err(format!(
            "persisted artifact minimum {} exceeds expected {expected_len}",
            encoded.len()
        ));
    }
    value
        .as_object_mut()
        .expect("sized persisted artifact remains a JSON object")
        .insert(
            "padding".to_owned(),
            Value::String("x".repeat(expected_len - encoded.len())),
        );
    let encoded = serde_json::to_vec(&value)
        .map_err(|error| format!("encode padded persisted artifact: {error}"))?;
    if encoded.len() != expected_len {
        return Err("padded persisted artifact changed its exact byte length".to_owned());
    }
    Ok(encoded)
}

fn json_hex(value: &Value) -> Result<String, String> {
    serde_json::to_vec(value)
        .map(|bytes| lower_hex(&bytes))
        .map_err(|error| format!("encode protocol-v5 projection evidence: {error}"))
}

fn fixture_daemon_error_code(code: ScenarioV5DaemonErrorCodeFixture) -> V5DaemonErrorCode {
    match code {
        ScenarioV5DaemonErrorCodeFixture::InvalidRequest => V5DaemonErrorCode::InvalidRequest,
        ScenarioV5DaemonErrorCodeFixture::HandshakeRequired => V5DaemonErrorCode::HandshakeRequired,
        ScenarioV5DaemonErrorCodeFixture::ProtocolMismatch => V5DaemonErrorCode::ProtocolMismatch,
        ScenarioV5DaemonErrorCodeFixture::CoreMismatch => V5DaemonErrorCode::CoreMismatch,
        ScenarioV5DaemonErrorCodeFixture::Unauthorized => V5DaemonErrorCode::Unauthorized,
        ScenarioV5DaemonErrorCodeFixture::DuplicateLease => V5DaemonErrorCode::DuplicateLease,
        ScenarioV5DaemonErrorCodeFixture::Overloaded => V5DaemonErrorCode::Overloaded,
        ScenarioV5DaemonErrorCodeFixture::OwnerCapacity => V5DaemonErrorCode::OwnerCapacity,
        ScenarioV5DaemonErrorCodeFixture::ReceiptNotFound => V5DaemonErrorCode::ReceiptNotFound,
        ScenarioV5DaemonErrorCodeFixture::ReceiptExpired => V5DaemonErrorCode::ReceiptExpired,
        ScenarioV5DaemonErrorCodeFixture::ReceiptCapacity => V5DaemonErrorCode::ReceiptCapacity,
        ScenarioV5DaemonErrorCodeFixture::TombstoneCapacity => V5DaemonErrorCode::TombstoneCapacity,
        ScenarioV5DaemonErrorCodeFixture::InvocationIdentityMismatch => {
            V5DaemonErrorCode::InvocationIdentityMismatch
        }
        ScenarioV5DaemonErrorCodeFixture::TaskNotFound => V5DaemonErrorCode::TaskNotFound,
        ScenarioV5DaemonErrorCodeFixture::TaskExpired => V5DaemonErrorCode::TaskExpired,
        ScenarioV5DaemonErrorCodeFixture::StoreFailed => V5DaemonErrorCode::StoreFailed,
        ScenarioV5DaemonErrorCodeFixture::DurabilityUncertain => {
            V5DaemonErrorCode::DurabilityUncertain
        }
        ScenarioV5DaemonErrorCodeFixture::StoreCommitUncertain => {
            V5DaemonErrorCode::StoreCommitUncertain
        }
    }
}

fn completed_probe_task(key: &ReceiptKey, ok: bool) -> Result<V5DaemonTaskSnapshot, String> {
    let mut result = DomainResult::success("canonical-success");
    result.ok = ok;
    let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Completed {
        result: Box::new(result.clone()),
    })
    .map_err(|error| format!("construct completed protocol-v5 Task terminal: {error}"))?;
    Ok(V5DaemonTaskSnapshot::Completed {
        task_id: key.reserved_task_id(),
        invocation_id: key.invocation_id(),
        receipt_key_digest: receipt_key_digest(key),
        created_at_epoch_ms: SCENARIO_INITIAL_EPOCH_MS,
        updated_at_epoch_ms: SCENARIO_INITIAL_EPOCH_MS,
        ttl_ms: SCENARIO_TASK_TTL_MS,
        poll_interval_ms: SCENARIO_TASK_POLL_INTERVAL_MS,
        version: 1,
        cancel_requested: false,
        terminal_epoch_ms: SCENARIO_INITIAL_EPOCH_MS,
        terminal_digest: terminal.digest().clone(),
        result: Box::new(result),
    })
}

fn maximum_v5_response_frame(oversized: bool) -> Result<Vec<u8>, String> {
    // Exercise the response writer's own line boundary. A completed Task cannot
    // reach it: its DomainResult is deliberately capped below the enclosing
    // protocol frame, so padding that result would test the wrong limit first.
    let mut instance_id = String::new();
    let empty = V5ServerResponse::Ready {
        protocol_version: DAEMON_PROTOCOL_VERSION,
        core_identity: CoreIdentity::production_v5(),
        daemon_pid: 1,
        instance_id: instance_id.clone(),
    };
    let empty_frame = encode_strict_v5_response_jsonl(&empty)?;
    let target = MAX_V5_RESPONSE_LINE_BYTES
        .checked_add(usize::from(oversized))
        .ok_or_else(|| "protocol-v5 maximum response target overflow".to_owned())?;
    let padding = target
        .checked_sub(empty_frame.len())
        .ok_or_else(|| "protocol-v5 maximum response envelope exceeds its limit".to_owned())?;
    instance_id = "x".repeat(padding);
    let response = V5ServerResponse::Ready {
        protocol_version: DAEMON_PROTOCOL_VERSION,
        core_identity: CoreIdentity::production_v5(),
        daemon_pid: 1,
        instance_id,
    };
    if oversized {
        let error = encode_strict_v5_response_jsonl(&response)
            .expect_err("one-byte oversized typed response must be rejected by the writer");
        if !error.contains("exceeds the byte limit") {
            return Err(format!(
                "unexpected oversized protocol-v5 response error: {error}"
            ));
        }
        return Ok(Vec::new());
    }
    let encoded = encode_strict_v5_response_jsonl(&response)?;
    if encoded.len() != MAX_V5_RESPONSE_LINE_BYTES {
        return Err("maximum protocol-v5 response writer length diverged".to_owned());
    }
    Ok(encoded)
}

fn run_v3_protocol_probe(
    client: ScenarioProtocolVersion,
    message: ScenarioProtocolMessage,
    label: String,
) -> Result<(Value, Vec<V5ReceiptRuntimeEvent>), String> {
    let root =
        tempfile::tempdir().map_err(|error| format!("create protocol-v3 probe state: {error}"))?;
    let state_root = std::fs::canonicalize(root.path())
        .map_err(|error| format!("canonicalize protocol-v3 probe state: {error}"))?;
    let identity = presented_core_identity(client, &message, &retired_v3_identity()?)?;
    let record = v3_wire::RetiredEndpoint::new(identity.clone());
    let client_hello_frame =
        v3_wire::frame(&v3_wire::hello(protocol_version_number(client), &record));
    let request_frame = build_v3_probe_request_frame(client, &message)?;
    let accepted = client == ScenarioProtocolVersion::V3;
    let response = if accepted {
        match message {
            ScenarioProtocolMessage::SubmitWithCoreIdentity { .. } => v3_wire::invocation_direct(
                &crate::domain::invocation::DomainResult::success("v3-guard"),
            )?,
            ScenarioProtocolMessage::DirectFailureTerminal { reason }
                if !failure_reason_introduced_in_v5(reason) =>
            {
                let reason = fixture_failure_reason(reason);
                let (code, message) = failure_projection(reason);
                v3_wire::invocation_direct(&DomainResult::scenario_rejection(None, code, message))?
            }
            ScenarioProtocolMessage::DirectFailureTerminal { .. } => {
                v3_wire::error("invalid_request")
            }
            ScenarioProtocolMessage::StoredInvocationRecord {
                schema_version,
                reason,
            } => {
                let _closed_v5_record = (schema_version, fixture_failure_reason(reason));
                v3_wire::error("invalid_request")
            }
            ScenarioProtocolMessage::Release => v3_wire::released(),
            _ => v3_wire::pong(),
        }
    } else {
        v3_wire::error("protocol_mismatch")
    };
    let response_frame = v3_wire::frame(&response);
    let response_error = response_error_value_v3(&response)?;
    Ok((
        protocol_probe_observation(
            &label,
            client,
            ScenarioProtocolVersion::V3,
            ProtocolProbeFrames {
                client_hello: client_hello_frame,
                server_ready: accepted.then(|| v3_wire::frame(&v3_wire::ready(&record))),
                client_write: request_frame.clone(),
                server_read: accepted.then_some(request_frame),
                server_write: response_frame.clone(),
                client_read: response_frame,
            },
            ProtocolProbeTrace {
                spawned_argv_hex: spawned_daemon_argv_hex(&state_root, &identity),
                daemon_process_events: daemon_process_events(
                    ScenarioProtocolVersion::V3,
                    accepted,
                    service_capability_fingerprint_for(&message).is_some(),
                ),
                production_events: production_events(accepted, response_error.is_none()),
                error: response_error,
                service_capability_fingerprint: service_capability_fingerprint_for(&message),
                delivery: None,
            },
            &identity,
        ),
        Vec::new(),
    ))
}

fn failure_reason_introduced_in_v5(reason: ScenarioFailureProbeReason) -> bool {
    matches!(
        reason,
        ScenarioFailureProbeReason::OutcomeUncertain
            | ScenarioFailureProbeReason::TaskCapacity
            | ScenarioFailureProbeReason::WorkspaceCapacity
            | ScenarioFailureProbeReason::WorkspaceRegistryFailed
    )
}

fn wait_for_protocol_endpoint(state_root: &Path, identity: &CoreIdentity) -> Result<(), String> {
    let deadline = Instant::now() + SCENARIO_OPERATION_TIMEOUT;
    loop {
        let state = DaemonStateDirectory::open(state_root, identity)?;
        let published = state.read_v5_endpoint_record()?.is_some();
        if published {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("protocol probe endpoint was not published".to_owned());
        }
        thread::sleep(Duration::from_millis(5));
    }
}

fn build_v5_probe_request_frame(
    state_root: &Path,
    identity: &CoreIdentity,
    message: &ScenarioProtocolMessage,
) -> Result<Vec<u8>, String> {
    match message {
        ScenarioProtocolMessage::Ping => jsonl_frame(&V5ClientRequest::Ping {}),
        ScenarioProtocolMessage::Release => jsonl_frame(&V5ClientRequest::Release {}),
        ScenarioProtocolMessage::SubmitWithCoreIdentity { .. } => {
            let invocation = V5InvocationRequest::new(
                InvocationId::new(),
                TaskId::new(),
                V5ToolIdentity::View,
                Map::new(),
                "workspace-a".to_owned(),
                7_000,
            )?;
            jsonl_frame(&V5ClientRequest::SubmitInvocation { invocation })
        }
        ScenarioProtocolMessage::GetTask
        | ScenarioProtocolMessage::WaitTask
        | ScenarioProtocolMessage::CancelTask => {
            let key = fresh_key(identity, &Map::new())?;
            seed_receipt_backed_task_terminal(
                state_root,
                identity,
                &ScenarioEpochClock::new(protocol_probe_epoch_ms(), false),
                key.clone(),
                ScenarioTerminalFixture::Success {
                    payload: "canonical-success".to_owned(),
                },
                false,
            )?;
            match message {
                ScenarioProtocolMessage::GetTask => jsonl_frame(&V5ClientRequest::GetTask {
                    task_id: key.reserved_task_id(),
                }),
                ScenarioProtocolMessage::WaitTask => jsonl_frame(&V5ClientRequest::WaitTask {
                    task_id: key.reserved_task_id(),
                    wait_ms: 7_000,
                }),
                ScenarioProtocolMessage::CancelTask => jsonl_frame(&V5ClientRequest::CancelTask {
                    task_id: key.reserved_task_id(),
                }),
                _ => unreachable!("Task probe group must preserve its exact variant"),
            }
        }
        ScenarioProtocolMessage::RecoverReceipt => {
            let (receipt_key, _) = seed_direct_probe_terminal(
                state_root,
                identity,
                ReceiptTerminalOutcome::Completed {
                    result: Box::new(DomainResult::success("canonical-success")),
                },
            )?;
            jsonl_frame(&V5ClientRequest::RecoverInvocationReceipt { receipt_key })
        }
        ScenarioProtocolMessage::AcknowledgeReceipt => {
            let (receipt_key, terminal_digest) = seed_direct_probe_terminal(
                state_root,
                identity,
                ReceiptTerminalOutcome::Completed {
                    result: Box::new(DomainResult::success("canonical-success")),
                },
            )?;
            jsonl_frame(&V5ClientRequest::AcknowledgeInvocationReceipt {
                receipt_key,
                terminal_digest,
            })
        }
        ScenarioProtocolMessage::CancelReceipt => jsonl_frame(&V5ClientRequest::CancelInvocation {
            receipt_key: fresh_key(identity, &Map::new())?,
        }),
        ScenarioProtocolMessage::MalformedV5Schema { target } => {
            jsonl_bytes_from_value(&strict_schema_mutation_value(*target)?)
        }
        ScenarioProtocolMessage::MaximumResponseFrame
        | ScenarioProtocolMessage::OversizedResponseFrame
        | ScenarioProtocolMessage::ErrorCodeFrame { .. }
        | ScenarioProtocolMessage::ReceiptPendingOutcome
        | ScenarioProtocolMessage::TaskOutcome
        | ScenarioProtocolMessage::AcknowledgedOutcome
        | ScenarioProtocolMessage::DirectCompletedTerminal
        | ScenarioProtocolMessage::DirectSemanticCompletedTerminal
        | ScenarioProtocolMessage::DirectCancelledTerminal
        | ScenarioProtocolMessage::DirectFailureTerminal { .. }
        | ScenarioProtocolMessage::TaskQueuedProjection
        | ScenarioProtocolMessage::TaskWorkingProjection
        | ScenarioProtocolMessage::TaskCompletedProjection
        | ScenarioProtocolMessage::TaskSemanticCompletedProjection { .. }
        | ScenarioProtocolMessage::TaskCancelledProjection
        | ScenarioProtocolMessage::TaskFailureProjection { .. }
        | ScenarioProtocolMessage::StoredInvocationRecord { .. } => {
            jsonl_frame(&V5ClientRequest::Ping {})
        }
    }
}

fn seed_direct_probe_terminal(
    state_root: &Path,
    identity: &CoreIdentity,
    outcome: ReceiptTerminalOutcome,
) -> Result<(ReceiptKey, TerminalDigest), String> {
    let key = fresh_key(identity, &Map::new())?;
    let terminal = canonical_v5_terminal(&outcome)
        .map_err(|error| format!("construct protocol-v5 probe terminal: {error}"))?;
    let terminal_digest = terminal.digest().clone();
    let state = DaemonStateDirectory::open(state_root, identity)?;
    let receipts = state.create_private_retained_subdirectory("receipts")?;
    let actor = open_receipt_actor_for_scenario(receipts, "open protocol-v5 probe receipt ledger")?;
    let deadline = Instant::now() + SCENARIO_OPERATION_TIMEOUT;
    let epoch_ms = protocol_probe_epoch_ms();
    let reservation = match actor
        .reserve(
            key.clone(),
            OriginalCutoffDescriptor::new(epoch_ms, 7_000)
                .map_err(|error| format!("construct protocol-v5 probe cutoff: {error}"))?,
            deadline,
        )
        .map_err(|error| format!("reserve protocol-v5 probe receipt: {error}"))?
    {
        crate::application::receipt_ledger::ReserveOutcome::Created(reservation) => reservation,
        crate::application::receipt_ledger::ReserveOutcome::ExistingExact(_) => {
            return Err("protocol-v5 probe receipt identity unexpectedly existed".to_owned())
        }
    };
    actor
        .publish_direct_terminal(
            key.clone(),
            reservation.record_version(),
            epoch_ms,
            terminal,
            deadline,
        )
        .map_err(|error| format!("publish protocol-v5 probe terminal: {error}"))?;
    drop(actor);
    Ok((key, terminal_digest))
}

fn protocol_probe_epoch_ms() -> u64 {
    SystemEpochMillisClock.now_epoch_millis()
}

fn build_v3_probe_request_frame(
    client: ScenarioProtocolVersion,
    message: &ScenarioProtocolMessage,
) -> Result<Vec<u8>, String> {
    if client != ScenarioProtocolVersion::V3 {
        return jsonl_frame(&V5ClientRequest::Ping {});
    }
    match message {
        ScenarioProtocolMessage::Ping => Ok(v3_wire::frame(&v3_wire::ping())),
        ScenarioProtocolMessage::Release => Ok(v3_wire::frame(&v3_wire::release())),
        ScenarioProtocolMessage::SubmitWithCoreIdentity { .. } => {
            Ok(v3_wire::frame(&v3_wire::submit_view("workspace-a", 7_000)))
        }
        ScenarioProtocolMessage::GetTask => Ok(v3_wire::frame(&v3_wire::get_task(TaskId::new()))),
        ScenarioProtocolMessage::WaitTask => {
            Ok(v3_wire::frame(&v3_wire::wait_task(TaskId::new(), 7_000)))
        }
        ScenarioProtocolMessage::CancelTask => {
            Ok(v3_wire::frame(&v3_wire::cancel_task(TaskId::new())))
        }
        ScenarioProtocolMessage::RecoverReceipt
        | ScenarioProtocolMessage::AcknowledgeReceipt
        | ScenarioProtocolMessage::CancelReceipt
        | ScenarioProtocolMessage::MaximumResponseFrame
        | ScenarioProtocolMessage::OversizedResponseFrame
        | ScenarioProtocolMessage::ErrorCodeFrame { .. }
        | ScenarioProtocolMessage::MalformedV5Schema { .. }
        | ScenarioProtocolMessage::ReceiptPendingOutcome
        | ScenarioProtocolMessage::TaskOutcome
        | ScenarioProtocolMessage::AcknowledgedOutcome
        | ScenarioProtocolMessage::DirectCompletedTerminal
        | ScenarioProtocolMessage::DirectSemanticCompletedTerminal
        | ScenarioProtocolMessage::DirectCancelledTerminal
        | ScenarioProtocolMessage::DirectFailureTerminal { .. }
        | ScenarioProtocolMessage::TaskQueuedProjection
        | ScenarioProtocolMessage::TaskWorkingProjection
        | ScenarioProtocolMessage::TaskCompletedProjection
        | ScenarioProtocolMessage::TaskSemanticCompletedProjection { .. }
        | ScenarioProtocolMessage::TaskCancelledProjection
        | ScenarioProtocolMessage::TaskFailureProjection { .. }
        | ScenarioProtocolMessage::StoredInvocationRecord { .. } => {
            Ok(v3_wire::frame(&v3_wire::ping()))
        }
    }
}

fn jsonl_frame<T: Serialize>(value: &T) -> Result<Vec<u8>, String> {
    let mut bytes = serde_json::to_vec(value)
        .map_err(|error| format!("encode protocol probe JSON frame: {error}"))?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn jsonl_bytes_from_value(value: &Value) -> Result<Vec<u8>, String> {
    jsonl_frame(value)
}

fn response_error_value_v5(response: &V5ServerResponse) -> Result<Option<Value>, String> {
    match response {
        V5ServerResponse::Error { code } => serde_json::to_value(code)
            .map(Some)
            .map_err(|error| format!("encode protocol-v5 response error: {error}")),
        _ => Ok(None),
    }
}

fn response_error_value_v3(response: &str) -> Result<Option<Value>, String> {
    let frame: Value = serde_json::from_str(response)
        .map_err(|error| format!("decode protocol-v3 response frame: {error}"))?;
    Ok((frame["kind"] == "error").then(|| frame["code"].clone()))
}

struct ProtocolProbeFrames {
    client_hello: Vec<u8>,
    server_ready: Option<Vec<u8>>,
    client_write: Vec<u8>,
    server_read: Option<Vec<u8>>,
    server_write: Vec<u8>,
    client_read: Vec<u8>,
}

struct ProtocolProbeTrace {
    spawned_argv_hex: String,
    daemon_process_events: Vec<&'static str>,
    production_events: Vec<&'static str>,
    error: Option<Value>,
    service_capability_fingerprint: Option<String>,
    delivery: Option<Value>,
}

fn protocol_probe_observation(
    label: &str,
    client: ScenarioProtocolVersion,
    server: ScenarioProtocolVersion,
    frames: ProtocolProbeFrames,
    trace: ProtocolProbeTrace,
    presented_core_identity_digest: &CoreIdentity,
) -> Value {
    let client_hello_frame = ensure_jsonl_frame(frames.client_hello);
    let server_ready_frame = frames.server_ready.map(ensure_jsonl_frame);
    let client_write_frame = ensure_jsonl_frame(frames.client_write);
    let server_read_frame = frames.server_read.map(ensure_jsonl_frame);
    let server_write_frame = ensure_jsonl_frame(frames.server_write);
    let client_read_frame = ensure_jsonl_frame(frames.client_read);
    json!({
        "label": label,
        "client": protocol_version_name(client),
        "server": protocol_version_name(server),
        "clientHelloFrameHex": lower_hex(&client_hello_frame),
        "serverReadyFrameHex": server_ready_frame.as_ref().map(|frame| lower_hex(frame)),
        "clientWriteFrameHex": lower_hex(&client_write_frame),
        "serverReadFrameHex": server_read_frame.as_ref().map(|frame| lower_hex(frame)),
        "serverWriteFrameHex": lower_hex(&server_write_frame),
        "clientReadFrameHex": lower_hex(&client_read_frame),
        "spawnedArgvHex": trace.spawned_argv_hex,
        "daemonProcessEvents": trace.daemon_process_events,
        "productionEvents": trace.production_events,
        "error": trace.error,
        "protocolIdentity": protocol_identity_name(server),
        "stateSelector": state_selector_name(server),
        "stateFingerprint": selector_fingerprint(server),
        "presentedCoreIdentityDigest": presented_core_identity_digest.as_str(),
        "productionV5CoreIdentityDigest": CoreIdentity::production_v5().as_str(),
        "serviceCapabilityFingerprint": trace.service_capability_fingerprint,
        "delivery": trace.delivery,
    })
}

fn ensure_jsonl_frame(mut frame: Vec<u8>) -> Vec<u8> {
    if frame.last() != Some(&b'\n') {
        frame.push(b'\n');
    }
    frame
}

fn service_capability_fingerprint_for(message: &ScenarioProtocolMessage) -> Option<String> {
    matches!(
        message,
        ScenarioProtocolMessage::SubmitWithCoreIdentity { .. }
    )
    .then(|| fingerprint_hex("canonical-v13-read-service"))
}

fn production_events(server_read: bool, accepted: bool) -> Vec<&'static str> {
    let mut events = vec!["client_frame_written"];
    if server_read {
        events.push("server_frame_read");
    }
    if !accepted {
        events.push("negotiation_rejected");
    }
    events.extend(["server_frame_written", "client_frame_read"]);
    events
}

/// The digest protocol v3 carried as its core identity. The protocol retired
/// with its client and loop; the probe keeps the frames it once exchanged so
/// the v5 runtime's refusal of them stays observable.
fn retired_v3_identity() -> Result<CoreIdentity, String> {
    CoreIdentity::from_str(v3_wire::RETIRED_V3_IDENTITY)
}

/// The retired protocol-v3 JSONL frames, spelled out byte for byte: the same
/// tags, camelCase fields and field order the serde types produced.
mod v3_wire {
    use crate::domain::invocation::{DomainResult, TaskId};
    use crate::infrastructure::daemon::identity::CoreIdentity;

    pub(super) const RETIRED_V3_IDENTITY: &str =
        "2f4dd5713d11e5211a92c5fa01b1ec5722dc3a3160b9b1e0b667f8d8da3d9c28";

    /// The process id a simulated v3 daemon presents; the probe never runs one.
    const RETIRED_V3_PROBE_PID: u32 = 9;

    /// What a v3 daemon's endpoint record would have carried into `ready`.
    pub(super) struct RetiredEndpoint {
        core_identity: CoreIdentity,
        pid: u32,
        token: String,
        instance_id: String,
    }

    impl RetiredEndpoint {
        pub(super) fn new(core_identity: CoreIdentity) -> Self {
            Self {
                core_identity,
                pid: RETIRED_V3_PROBE_PID,
                token: uuid::Uuid::new_v4().to_string(),
                instance_id: uuid::Uuid::new_v4().to_string(),
            }
        }
    }

    /// The retired v3 `invocation` response, typed the way its serde types
    /// were: the codec boundary of v5 does not own this shape.
    #[derive(serde::Serialize)]
    struct RetiredInvocationFrame<'a> {
        kind: &'static str,
        outcome: RetiredDirectOutcome<'a>,
    }

    #[derive(serde::Serialize)]
    struct RetiredDirectOutcome<'a> {
        #[serde(rename = "resultType")]
        result_type: &'static str,
        value: &'a DomainResult,
    }

    pub(super) fn frame(line: &str) -> Vec<u8> {
        let mut bytes = line.as_bytes().to_vec();
        bytes.push(b'\n');
        bytes
    }

    fn text(value: &str) -> String {
        serde_json::to_string(value).expect("encode a JSON string")
    }

    pub(super) fn hello(protocol_version: u32, record: &RetiredEndpoint) -> String {
        format!(
            "{{\"kind\":\"hello\",\"protocolVersion\":{protocol_version},\"token\":{},\"coreIdentity\":{},\"ownerLease\":{}}}",
            text(&record.token),
            text(record.core_identity.as_str()),
            text(&uuid::Uuid::new_v4().to_string()),
        )
    }

    pub(super) fn ping() -> String {
        "{\"kind\":\"ping\"}".to_owned()
    }

    pub(super) fn release() -> String {
        "{\"kind\":\"release\"}".to_owned()
    }

    pub(super) fn submit_view(workspace_hint: &str, response_budget_ms: u64) -> String {
        format!(
            "{{\"kind\":\"submit_invocation\",\"invocation\":{{\"tool\":\"unica.view\",\"arguments\":{{}},\"workspaceHint\":{},\"responseBudgetMs\":{response_budget_ms}}}}}",
            text(workspace_hint)
        )
    }

    pub(super) fn get_task(task_id: TaskId) -> String {
        format!(
            "{{\"kind\":\"get_task\",\"taskId\":{}}}",
            text(&task_id.to_string())
        )
    }

    pub(super) fn wait_task(task_id: TaskId, wait_ms: u64) -> String {
        format!(
            "{{\"kind\":\"wait_task\",\"taskId\":{},\"waitMs\":{wait_ms}}}",
            text(&task_id.to_string())
        )
    }

    pub(super) fn cancel_task(task_id: TaskId) -> String {
        format!(
            "{{\"kind\":\"cancel_task\",\"taskId\":{}}}",
            text(&task_id.to_string())
        )
    }

    pub(super) fn ready(record: &RetiredEndpoint) -> String {
        format!(
            "{{\"kind\":\"ready\",\"protocolVersion\":3,\"coreIdentity\":{},\"daemonPid\":{},\"instanceId\":{}}}",
            text(record.core_identity.as_str()),
            record.pid,
            text(&record.instance_id),
        )
    }

    pub(super) fn pong() -> String {
        "{\"kind\":\"pong\"}".to_owned()
    }

    pub(super) fn released() -> String {
        "{\"kind\":\"released\"}".to_owned()
    }

    pub(super) fn error(code: &str) -> String {
        format!("{{\"kind\":\"error\",\"code\":{}}}", text(code))
    }

    pub(super) fn invocation_direct(result: &DomainResult) -> Result<String, String> {
        serde_json::to_string(&RetiredInvocationFrame {
            kind: "invocation",
            outcome: RetiredDirectOutcome {
                result_type: "direct",
                value: result,
            },
        })
        .map_err(|error| format!("encode protocol-v3 direct result: {error}"))
    }
}

fn daemon_process_events(
    server: ScenarioProtocolVersion,
    accepted: bool,
    service_entered: bool,
) -> Vec<&'static str> {
    let mut events = match server {
        ScenarioProtocolVersion::V3 => vec![
            "spawned",
            "interfaces_daemon_entrypoint_entered",
            "default_v3_composition_selected",
        ],
        ScenarioProtocolVersion::V4 => unreachable!("v4 is never a production daemon"),
        ScenarioProtocolVersion::V5 => vec![
            "spawned",
            "interfaces_daemon_entrypoint_entered",
            "versioned_v5_dispatch_selected",
        ],
    };
    if accepted {
        events.push(match server {
            ScenarioProtocolVersion::V3 => "v3_handshake_completed",
            ScenarioProtocolVersion::V5 => "v5_handshake_completed",
            ScenarioProtocolVersion::V4 => unreachable!(),
        });
        events.push("protocol_frame_handled");
    }
    if service_entered {
        events.push("canonical_v13_service_entered");
    }
    events
}

fn presented_core_identity(
    client: ScenarioProtocolVersion,
    message: &ScenarioProtocolMessage,
    default_identity: &CoreIdentity,
) -> Result<CoreIdentity, String> {
    match (client, message) {
        (
            ScenarioProtocolVersion::V3,
            ScenarioProtocolMessage::SubmitWithCoreIdentity {
                selection: ScenarioCoreIdentitySelection::ArbitraryCanonical,
            },
        ) => CoreIdentity::from_str(
            "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
        ),
        _ => Ok(default_identity.clone()),
    }
}

fn protocol_version_name(version: ScenarioProtocolVersion) -> &'static str {
    match version {
        ScenarioProtocolVersion::V3 => "v3",
        ScenarioProtocolVersion::V4 => "v4",
        ScenarioProtocolVersion::V5 => "v5",
    }
}

fn protocol_version_number(version: ScenarioProtocolVersion) -> u32 {
    match version {
        ScenarioProtocolVersion::V3 => 3,
        ScenarioProtocolVersion::V4 => 4,
        ScenarioProtocolVersion::V5 => 5,
    }
}

fn protocol_identity_name(server: ScenarioProtocolVersion) -> &'static str {
    match server {
        ScenarioProtocolVersion::V3 => "unica-daemon-jsonl-3",
        ScenarioProtocolVersion::V4 => unreachable!("v4 is never a production daemon"),
        ScenarioProtocolVersion::V5 => "unica-daemon-jsonl-5",
    }
}

fn state_selector_name(server: ScenarioProtocolVersion) -> &'static str {
    match server {
        ScenarioProtocolVersion::V3 => "protocol_v3",
        ScenarioProtocolVersion::V4 => unreachable!("v4 is never a production daemon"),
        ScenarioProtocolVersion::V5 => "receipt_v5",
    }
}

fn selector_fingerprint(server: ScenarioProtocolVersion) -> String {
    fingerprint_hex(state_selector_name(server))
}

fn fingerprint_hex(value: &str) -> String {
    lower_hex(&Sha256::digest(value.as_bytes()))
}

fn spawned_daemon_argv_hex(state_root: &Path, identity: &CoreIdentity) -> String {
    let executable = std::env::current_exe()
        .ok()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unica".to_owned());
    let argv = [
        executable,
        "--daemon".to_owned(),
        "--state-root".to_owned(),
        state_root.display().to_string(),
        "--core-identity".to_owned(),
        identity.as_str().to_owned(),
        "--idle-grace-ms".to_owned(),
        SCENARIO_IDLE_GRACE.as_millis().to_string(),
    ];
    let mut bytes = Vec::new();
    for argument in argv {
        bytes.extend_from_slice(argument.as_bytes());
        bytes.push(0);
    }
    lower_hex(&bytes)
}

fn strict_schema_mutation_value(target: ScenarioStrictSchemaTarget) -> Result<Value, String> {
    let task_snapshot = || {
        json!({
            "status": "queued",
            "taskId": "11111111-1111-4111-8111-111111111111",
            "invocationId": "22222222-2222-4222-8222-222222222222",
            "receiptKeyDigest": "0".repeat(64),
            "createdAtEpochMs": 1,
            "updatedAtEpochMs": 1,
            "ttlMs": SCENARIO_TASK_TTL_MS,
            "pollIntervalMs": 100,
            "version": 1,
            "cancelRequested": false,
        })
    };
    let stored_record = || {
        json!({
            "schemaVersion": 1,
            "taskId": "11111111-1111-4111-8111-111111111111",
            "invocationId": "22222222-2222-4222-8222-222222222222",
            "receiptKeyDigest": "0".repeat(64),
            "tool": "unica.view",
            "normalizedArgumentsHash": "0".repeat(64),
            "workspaceIdentityHash": "a".repeat(64),
            "createdAtEpochMs": 1,
            "updatedAtEpochMs": 1,
            "ttlMs": SCENARIO_TASK_TTL_MS,
            "pollIntervalMs": 100,
            "version": 1,
            "cancelRequested": false,
            "task": {"status": "queued"},
        })
    };
    let transfer_certificate = || {
        json!({
            "certificateVersion": 1,
            "protocolIdentity": "v5",
            "coreIdentityDigest": "a".repeat(64),
            "receiptKeyDigest": "0".repeat(64),
            "taskId": "11111111-1111-4111-8111-111111111111",
            "invocationId": "22222222-2222-4222-8222-222222222222",
            "taskLinkDigest": "4c73d08219973c72e759a9f85e156fa42c9d8e61a56e704b70d1c7c042b73da0",
            "terminalDigest": "f2d0423d2613a0d09397b750542e4542f7653d78ebd5e0448f1326d09145d9ae",
            "terminalEpochMs": 1,
            "receiptRecordSchemaVersion": 1,
            "taskRecordSchemaVersion": 1,
            "lifecycleLinkRecordSchemaVersion": 1,
            "terminalCodecVersion": 1,
            "maxDaemonResponseLineBytes": MAX_V5_RESPONSE_LINE_BYTES,
            "maxTaskLifecycleLinkRecordBytes": 1_024,
            "stagedReceiptRecordMaxBytes": MAX_V5_RESPONSE_LINE_BYTES,
            "taskTerminalBoundLinkRecordMaxBytes": 1_024,
            "taskPublicationCases": [
                {"kind": "absent", "finalTaskRecordMaxBytes": MAX_V5_RESPONSE_LINE_BYTES, "taskResponseFrameMaxBytes": MAX_V5_RESPONSE_LINE_BYTES},
                {"kind": "exact_provisional", "status": "queued", "version": u64::MAX, "cancelRequested": false, "finalTaskRecordMaxBytes": MAX_V5_RESPONSE_LINE_BYTES, "taskResponseFrameMaxBytes": MAX_V5_RESPONSE_LINE_BYTES},
                {"kind": "exact_provisional", "status": "queued", "version": u64::MAX, "cancelRequested": true, "finalTaskRecordMaxBytes": MAX_V5_RESPONSE_LINE_BYTES, "taskResponseFrameMaxBytes": MAX_V5_RESPONSE_LINE_BYTES},
                {"kind": "exact_provisional", "status": "working", "version": u64::MAX, "cancelRequested": false, "finalTaskRecordMaxBytes": MAX_V5_RESPONSE_LINE_BYTES, "taskResponseFrameMaxBytes": MAX_V5_RESPONSE_LINE_BYTES},
                {"kind": "exact_provisional", "status": "working", "version": u64::MAX, "cancelRequested": true, "finalTaskRecordMaxBytes": MAX_V5_RESPONSE_LINE_BYTES, "taskResponseFrameMaxBytes": MAX_V5_RESPONSE_LINE_BYTES}
            ],
            "capacityFallbackCases": [{"source": "link_capacity", "receiptBackedRecordMaxBytes": MAX_V5_RESPONSE_LINE_BYTES, "taskResponseFrameMaxBytes": MAX_V5_RESPONSE_LINE_BYTES}],
        })
    };
    let value = match target {
        ScenarioStrictSchemaTarget::RequestUnknownField => {
            json!({"kind": "ping", "unexpected": true})
        }
        ScenarioStrictSchemaTarget::RequestMissingRequiredField => json!({"kind": "get_task"}),
        ScenarioStrictSchemaTarget::RequestCrossVariantField => json!({
            "kind": "get_task",
            "taskId": "11111111-1111-4111-8111-111111111111",
            "waitMs": 1,
        }),
        ScenarioStrictSchemaTarget::ResponseUnknownField => {
            json!({"kind": "pong", "unexpected": true})
        }
        ScenarioStrictSchemaTarget::ResponseMissingRequiredField => json!({"kind": "task"}),
        ScenarioStrictSchemaTarget::ResponseCrossVariantField => {
            json!({"kind": "pong", "snapshot": task_snapshot()})
        }
        ScenarioStrictSchemaTarget::TerminalUnknownField => {
            json!({"status": "failed", "reason": "invocation_failed", "unexpected": true})
        }
        ScenarioStrictSchemaTarget::TerminalMissingRequiredField => json!({"status": "failed"}),
        ScenarioStrictSchemaTarget::TerminalCrossVariantField => {
            json!({"status": "failed", "reason": "invocation_failed", "result": {"ok": false, "summary": "semantic-invalid"}})
        }
        ScenarioStrictSchemaTarget::TaskSnapshotUnknownField => {
            let mut value = task_snapshot();
            value["unexpected"] = true.into();
            value
        }
        ScenarioStrictSchemaTarget::TaskSnapshotMissingRequiredField => {
            let mut value = task_snapshot();
            value
                .as_object_mut()
                .expect("Task fixture object")
                .remove("taskId");
            value
        }
        ScenarioStrictSchemaTarget::TaskSnapshotCrossVariantField => {
            let mut value = task_snapshot();
            value["reason"] = "invocation_failed".into();
            value
        }
        ScenarioStrictSchemaTarget::StoredRecordUnknownTopLevel => {
            let mut value = stored_record();
            value["unexpected"] = true.into();
            value
        }
        ScenarioStrictSchemaTarget::StoredRecordUnknownTaskField => {
            let mut value = stored_record();
            value["task"]["unexpected"] = true.into();
            value
        }
        ScenarioStrictSchemaTarget::StoredRecordMissingRequiredField => {
            let mut value = stored_record();
            value
                .as_object_mut()
                .expect("stored fixture object")
                .remove("schemaVersion");
            value
        }
        ScenarioStrictSchemaTarget::StoredRecordCrossVariantField => {
            let mut value = stored_record();
            value["task"]["reason"] = "invocation_failed".into();
            value
        }
        ScenarioStrictSchemaTarget::TransferCertificateUnknownField => {
            let mut value = transfer_certificate();
            value["unexpected"] = true.into();
            value
        }
        ScenarioStrictSchemaTarget::TransferCertificateMissingRequiredField => {
            let mut value = transfer_certificate();
            value
                .as_object_mut()
                .expect("certificate fixture object")
                .remove("terminalCodecVersion");
            value
        }
        ScenarioStrictSchemaTarget::TransferCertificateCrossVariantField => {
            let mut value = transfer_certificate();
            value["taskPublicationCases"][0]["cancelRequested"] = false.into();
            value
        }
    };
    Ok(value)
}

fn strict_envelope_case(case: ScenarioEnvelopeCase) -> StrictV5EnvelopeCase {
    match case {
        ScenarioEnvelopeCase::MissingInvocationId => StrictV5EnvelopeCase::MissingInvocationId,
        ScenarioEnvelopeCase::NoncanonicalInvocationId => {
            StrictV5EnvelopeCase::NoncanonicalInvocationId
        }
        ScenarioEnvelopeCase::MissingReservedTaskId => StrictV5EnvelopeCase::MissingReservedTaskId,
        ScenarioEnvelopeCase::NoncanonicalReservedTaskId => {
            StrictV5EnvelopeCase::NoncanonicalReservedTaskId
        }
        ScenarioEnvelopeCase::UnknownTool => StrictV5EnvelopeCase::UnknownTool,
        ScenarioEnvelopeCase::UnknownField => StrictV5EnvelopeCase::UnknownField,
        ScenarioEnvelopeCase::MalformedArguments => StrictV5EnvelopeCase::MalformedArguments,
        ScenarioEnvelopeCase::OversizedArguments => StrictV5EnvelopeCase::OversizedArguments,
        ScenarioEnvelopeCase::ResponseBudgetAboveMaximum => {
            StrictV5EnvelopeCase::ResponseBudgetAboveMaximum
        }
        ScenarioEnvelopeCase::EmptyWorkspaceHint => StrictV5EnvelopeCase::EmptyWorkspaceHint,
        ScenarioEnvelopeCase::WorkspaceHintWithControl => {
            StrictV5EnvelopeCase::WorkspaceHintWithControl
        }
        ScenarioEnvelopeCase::MalformedWorkspaceHint => {
            StrictV5EnvelopeCase::MalformedWorkspaceHint
        }
        ScenarioEnvelopeCase::OversizedWorkspaceHint => {
            StrictV5EnvelopeCase::OversizedWorkspaceHint
        }
    }
}

fn exchange_ack_and_expect_disconnect(
    state_root: &Path,
    identity: &CoreIdentity,
    clock: Arc<ScenarioEpochClock>,
    control: Arc<ReceiptScenarioControl>,
    telemetry: Arc<V5ReceiptRuntimeTelemetry>,
    key: ReceiptKey,
    terminal_digest: TerminalDigest,
) -> Result<(), String> {
    let config = scenario_server_config_with_clock(state_root, identity, Some(&control), &clock);
    let daemon = ScenarioDaemon::spawn(config, move |runtime| {
        let mut runtime =
            runtime.with_hooks_for_test(ScenarioHooks::install(telemetry, Some(control)));
        runtime.epoch_clock = clock;
        runtime
    });
    let result = (|| {
        wait_for_endpoint(state_root, identity)?;
        let mut owner = V5DaemonProcessOwner::connect_or_spawn(
            state_root,
            identity.clone(),
            std::path::PathBuf::from("unused-existing-v5-scenario-endpoint"),
            SCENARIO_IDLE_GRACE,
        )?;
        let result = owner.acknowledge_invocation_receipt(key, terminal_digest);
        drop(owner);
        match result {
            Ok(_) => {
                Err("ACK response was delivered instead of disconnecting after commit".to_owned())
            }
            Err(_) => Ok(()),
        }
    })();
    let cleanup = daemon.stop_and_join("protocol-v5 receipt scenario ACK daemon panicked");
    finish_with_daemon_cleanup(result, cleanup)
}

fn exchange_submit_and_expect_disconnect(
    state_root: &Path,
    identity: &CoreIdentity,
    clock: Arc<ScenarioEpochClock>,
    control: Arc<ReceiptScenarioControl>,
    telemetry: Arc<V5ReceiptRuntimeTelemetry>,
    invocation: V5InvocationRequest,
) -> Result<(), String> {
    let config = scenario_server_config_with_clock(state_root, identity, Some(&control), &clock);
    let daemon = ScenarioDaemon::spawn(config, move |runtime| {
        let mut runtime =
            runtime.with_hooks_for_test(ScenarioHooks::install(telemetry, Some(control)));
        runtime.epoch_clock = clock;
        runtime
    });
    let result = (|| {
        wait_for_endpoint(state_root, identity)?;
        let mut owner = V5DaemonProcessOwner::connect_or_spawn(
            state_root,
            identity.clone(),
            std::path::PathBuf::from("unused-existing-v5-scenario-endpoint"),
            SCENARIO_IDLE_GRACE,
        )?;
        let result = owner.submit_invocation(invocation);
        drop(owner);
        match result {
            Ok(_) => Err(
                "submit response was delivered instead of disconnecting after terminal commit"
                    .to_owned(),
            ),
            Err(_) => Ok(()),
        }
    })();
    let cleanup = daemon.stop_and_join("protocol-v5 receipt scenario submit daemon panicked");
    finish_with_daemon_cleanup(result, cleanup)
}

fn submit_and_disconnect_after_write(
    state_root: &Path,
    identity: &CoreIdentity,
    clock: Arc<ScenarioEpochClock>,
    control: Arc<ReceiptScenarioControl>,
    telemetry: Arc<V5ReceiptRuntimeTelemetry>,
    invocation: V5InvocationRequest,
) -> Result<(), String> {
    let config = scenario_server_config_with_clock(state_root, identity, Some(&control), &clock);
    let daemon_telemetry = Arc::clone(&telemetry);
    let daemon = ScenarioDaemon::spawn(config, move |runtime| {
        let mut runtime =
            runtime.with_hooks_for_test(ScenarioHooks::install(daemon_telemetry, Some(control)));
        runtime.epoch_clock = clock;
        runtime
    });
    let result = (|| {
        wait_for_endpoint(state_root, identity)?;
        let state = DaemonStateDirectory::open(state_root, identity)?;
        let record = state
            .read_v5_endpoint_record()?
            .ok_or_else(|| "protocol-v5 submit endpoint disappeared".to_owned())?;
        let handshake = V5DaemonProcessOwner::connect_existing_raw_for_test(
            record,
            DAEMON_PROTOCOL_VERSION,
            identity.clone(),
            uuid::Uuid::new_v4().to_string(),
            Instant::now() + SCENARIO_OPERATION_TIMEOUT,
        )?;
        let V5RawHandshake::Ready { owner, .. } = handshake else {
            return Err("protocol-v5 submit handshake was unexpectedly rejected".to_owned());
        };
        let mut request_frame =
            serde_json::to_vec(&V5ClientRequest::SubmitInvocation { invocation })
                .map_err(|error| format!("encode disconnecting protocol-v5 submit: {error}"))?;
        request_frame.push(b'\n');
        owner.write_raw_frame_and_disconnect(&request_frame, "disconnecting submit request")?;
        telemetry.wait_for_event(
            V5ReceiptRuntimeEventKind::ReceiptTerminalCommitted,
            Instant::now() + SCENARIO_OPERATION_TIMEOUT,
        )
    })();
    let cleanup = daemon.stop_and_join("protocol-v5 disconnecting submit daemon panicked");
    finish_with_daemon_cleanup(result, cleanup)
}

#[cfg(test)]
enum ScenarioWireRequest {
    InjectedClientFailure,
}

#[cfg(test)]
fn exchange_batch(
    state_root: &Path,
    identity: &CoreIdentity,
    clock: Arc<ScenarioEpochClock>,
    telemetry: Arc<V5ReceiptRuntimeTelemetry>,
    scenario_control: Option<Arc<ReceiptScenarioControl>>,
    requests: Vec<ScenarioWireRequest>,
) -> Result<Vec<V5ServerResponse>, String> {
    let config =
        scenario_server_config_with_clock(state_root, identity, scenario_control.as_ref(), &clock);
    let daemon = ScenarioDaemon::spawn(config, move |runtime| {
        let mut runtime =
            runtime.with_hooks_for_test(ScenarioHooks::install(telemetry, scenario_control));
        runtime.epoch_clock = clock;
        runtime
    });
    let responses = (|| {
        wait_for_endpoint(state_root, identity)?;
        let mut responses = Vec::with_capacity(requests.len());
        for request in requests {
            let response = match request {
                ScenarioWireRequest::InjectedClientFailure => {
                    Err("injected protocol-v5 scenario client failure".to_owned())
                }
            }?;
            responses.push(response);
        }
        Ok(responses)
    })();
    let cleanup = daemon.stop_and_join("protocol-v5 receipt scenario batch daemon panicked");
    finish_with_daemon_cleanup(responses, cleanup)
}

struct ScenarioDaemon {
    stop_requested: Arc<AtomicBool>,
    server: Option<thread::JoinHandle<Result<(), String>>>,
}

impl ScenarioDaemon {
    fn spawn(
        config: DaemonServerConfig,
        configure_runtime: impl FnOnce(V5ReceiptRuntime) -> V5ReceiptRuntime + Send + 'static,
    ) -> Self {
        let stop_requested = Arc::new(AtomicBool::new(false));
        let server_stop = Arc::clone(&stop_requested);
        let server = thread::spawn(move || {
            run_daemon_configured_until(config, configure_runtime, || {
                server_stop.load(Ordering::Acquire)
            })
        });
        Self {
            stop_requested,
            server: Some(server),
        }
    }

    fn request_stop(&self) {
        self.stop_requested.store(true, Ordering::Release);
    }

    fn stop_and_join(mut self, panic_message: &'static str) -> Result<(), String> {
        self.request_stop();
        let server = self
            .server
            .take()
            .expect("scenario daemon join handle must exist before explicit join");
        if server.thread().id() == thread::current().id() {
            return Err("protocol-v5 receipt scenario daemon cannot join itself".to_owned());
        }
        server.join().map_err(|_| panic_message.to_owned())?
    }
}

impl Drop for ScenarioDaemon {
    fn drop(&mut self) {
        self.request_stop();
        let Some(server) = self.server.take() else {
            return;
        };
        if server.thread().id() != thread::current().id() {
            let _ = server.join();
        }
    }
}

fn finish_with_daemon_cleanup<T>(
    operation: Result<T, String>,
    cleanup: Result<(), String>,
) -> Result<T, String> {
    match (operation, cleanup) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(cleanup)) => Err(cleanup),
        (Err(error), Err(cleanup)) => Err(format!("{error}; daemon cleanup failed: {cleanup}")),
    }
}

struct ScenarioOperation {
    completed: Arc<AtomicBool>,
    handle: thread::JoinHandle<Result<(), String>>,
}

fn open_scenario_operation_runtime(
    state_root: &Path,
    identity: &CoreIdentity,
    clock: &Arc<ScenarioEpochClock>,
    control: &Arc<ReceiptScenarioControl>,
    telemetry: &Arc<V5ReceiptRuntimeTelemetry>,
) -> Result<(Arc<V5ReceiptRuntime>, TaskProjectionObservation, u64), String> {
    let task_projection = inspect_task_projection(
        state_root,
        identity,
        Arc::clone(clock),
        &[],
        &HashMap::new(),
    )?;
    let task_store_create_attempts = telemetry.snapshot().task_store_create_attempts;
    let daemon_state = DaemonStateDirectory::open(state_root, identity)?;
    // A gated operation observes the seeded fixture it was handed, not what a
    // successor would reconcile it into.
    control.arm_skip_next_startup_reconciliation();
    let config = scenario_server_config_with_clock(state_root, identity, Some(control), clock);
    let runtime = V5ReceiptRuntime::open_with_epoch_clock(&daemon_state, &config, clock.clone())?
        .with_hooks_for_test(ScenarioHooks::install(
            Arc::clone(telemetry),
            Some(Arc::clone(control)),
        ));
    Ok((
        Arc::new(runtime),
        task_projection,
        task_store_create_attempts,
    ))
}

fn spawn_scenario_operation(
    label: String,
    control: Arc<ReceiptScenarioControl>,
    work: impl FnOnce() -> Result<(), String> + Send + 'static,
) -> ScenarioOperation {
    let completed = Arc::new(AtomicBool::new(false));
    let worker_completed = Arc::clone(&completed);
    control.record_operation_event(&label, "spawned");
    let handle = thread::spawn(move || {
        let result = work();
        control.record_operation_event(&label, "completed");
        worker_completed.store(true, Ordering::Release);
        result
    });
    ScenarioOperation { completed, handle }
}

struct PendingSubmit {
    label: String,
    accepted_epoch_ms: u64,
    accepted_monotonic_ms: u64,
    response_budget_ms: u64,
    actor: ReceiptLedgerActor,
    task_projection: TaskProjectionObservation,
    task_store_create_attempts: u64,
    response_projected: bool,
    client: Option<thread::JoinHandle<Result<V5ServerResponse, String>>>,
    /// The daemon's reply once the runner awaited it at the cutoff.
    response: Option<V5ServerResponse>,
    daemon: ScenarioDaemon,
}

struct SpawnedSubmitClient {
    key: ReceiptKey,
    accepted_epoch_ms: u64,
    response_budget_ms: u64,
    client: thread::JoinHandle<Result<V5ServerResponse, String>>,
}

type FinishedPendingSubmit = (
    String,
    u64,
    u64,
    V5ServerResponse,
    ReceiptLedgerActor,
    TaskProjectionObservation,
    u64,
    ScenarioDaemon,
);

impl PendingSubmit {
    fn submit_additional(
        &self,
        state_root: &Path,
        identity: &CoreIdentity,
        invocation: V5InvocationRequest,
    ) -> Result<V5ServerResponse, String> {
        let mut owner = V5DaemonProcessOwner::connect_or_spawn(
            state_root,
            identity.clone(),
            std::path::PathBuf::from("unused-existing-v5-scenario-endpoint"),
            SCENARIO_IDLE_GRACE,
        )?;
        owner.submit_invocation(invocation)
    }

    fn cancel_additional(
        &self,
        state_root: &Path,
        identity: &CoreIdentity,
        key: ReceiptKey,
    ) -> Result<V5ServerResponse, String> {
        cancel_on_live_daemon(state_root, identity, key)
    }

    /// Waits for the daemon's reply to the blocked submit: the runtime owns
    /// the cutoff, the runner only observes what it answered.
    fn await_response(&mut self, deadline: Instant) -> Result<V5ServerResponse, String> {
        if let Some(response) = &self.response {
            return Ok(response.clone());
        }
        let Some(client) = self.client.take() else {
            return Err("protocol-v5 receipt scenario submit client was already joined".to_owned());
        };
        while !client.is_finished() {
            if Instant::now() >= deadline {
                self.client = Some(client);
                return Err(format!(
                    "protocol-v5 daemon did not answer submit {} at its cutoff",
                    self.label
                ));
            }
            thread::sleep(Duration::from_millis(5));
        }
        let response = client
            .join()
            .map_err(|_| "protocol-v5 receipt scenario submit client panicked".to_owned())
            .and_then(|response| response)?;
        self.response = Some(response.clone());
        Ok(response)
    }

    /// Waits for the durable handoff intent of the blocked submit while the
    /// observer holds the Task's store create: the runtime's reply cannot
    /// arrive before that barrier lifts. `None` when the receipt moved
    /// elsewhere (a promise, a terminal) or the reply already arrived.
    fn await_handoff_intent(
        &self,
        key: &ReceiptKey,
        deadline: Instant,
    ) -> Result<Option<TaskHandoffActorBoundReceipt>, String> {
        loop {
            if self
                .client
                .as_ref()
                .is_some_and(|client| client.is_finished())
            {
                return Ok(None);
            }
            match self.actor.recover(key.clone(), deadline) {
                Ok(ReceiptState::TaskHandoffActorBound(handoff)) => return Ok(Some(handoff)),
                Ok(ReceiptState::Reserved(_)) => {}
                Ok(_) => return Ok(None),
                Err(error) => {
                    return Err(format!(
                        "inspect cutoff receipt for {}: {error}",
                        self.label
                    ))
                }
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "protocol-v5 daemon did not commit the cutoff handoff of {}",
                    self.label
                ));
            }
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn finish(self) -> Result<FinishedPendingSubmit, String> {
        let Self {
            label,
            accepted_epoch_ms,
            accepted_monotonic_ms: _,
            response_budget_ms,
            actor,
            task_projection,
            task_store_create_attempts,
            response_projected: _,
            client,
            response,
            daemon,
        } = self;
        let response = match (response, client) {
            (Some(response), client) => {
                // The projection stands; the client's own read ends with the
                // daemon, whichever way that goes.
                if let Some(client) = client {
                    let deadline = Instant::now() + SCENARIO_OPERATION_TIMEOUT;
                    while !client.is_finished() && Instant::now() < deadline {
                        thread::sleep(Duration::from_millis(5));
                    }
                    if client.is_finished() {
                        let _ = client.join();
                    }
                }
                Ok(response)
            }
            (None, Some(client)) => client
                .join()
                .map_err(|_| "protocol-v5 receipt scenario submit client panicked".to_owned())
                .and_then(|response| response),
            (None, None) => {
                Err("protocol-v5 receipt scenario submit client was already joined".to_owned())
            }
        };
        match response {
            Ok(response) => Ok((
                label,
                accepted_epoch_ms,
                response_budget_ms,
                response,
                actor,
                task_projection,
                task_store_create_attempts,
                daemon,
            )),
            Err(error) => {
                let cleanup =
                    daemon.stop_and_join("protocol-v5 receipt scenario blocked daemon panicked");
                Err(match cleanup {
                    Ok(()) => error,
                    Err(cleanup) => format!("{error}; daemon cleanup failed: {cleanup}"),
                })
            }
        }
    }
}

fn spawn_additional_submit_client(
    state_root: &Path,
    identity: &CoreIdentity,
    invocation: V5InvocationRequest,
    key: ReceiptKey,
    accepted_epoch_ms: u64,
    response_budget_ms: u64,
) -> SpawnedSubmitClient {
    let client_state_root = state_root.to_path_buf();
    let client_identity = identity.clone();
    let client = thread::spawn(move || {
        let mut owner = V5DaemonProcessOwner::connect_or_spawn(
            &client_state_root,
            client_identity,
            std::path::PathBuf::from("unused-existing-v5-scenario-endpoint"),
            SCENARIO_IDLE_GRACE,
        )?;
        owner.submit_invocation_with_timeout_for_test(invocation, SCENARIO_BULK_SNAPSHOT_TIMEOUT)
    });
    SpawnedSubmitClient {
        key,
        accepted_epoch_ms,
        response_budget_ms,
        client,
    }
}

fn acknowledge_on_live_daemon(
    state_root: &Path,
    identity: &CoreIdentity,
    key: ReceiptKey,
    terminal_digest: TerminalDigest,
) -> Result<V5ServerResponse, String> {
    let mut owner = V5DaemonProcessOwner::connect_or_spawn(
        state_root,
        identity.clone(),
        std::path::PathBuf::from("unused-existing-v5-scenario-endpoint"),
        SCENARIO_IDLE_GRACE,
    )?;
    owner.acknowledge_invocation_receipt(key, terminal_digest)
}

fn cancel_on_live_daemon(
    state_root: &Path,
    identity: &CoreIdentity,
    key: ReceiptKey,
) -> Result<V5ServerResponse, String> {
    let mut owner = V5DaemonProcessOwner::connect_or_spawn(
        state_root,
        identity.clone(),
        std::path::PathBuf::from("unused-existing-v5-scenario-endpoint"),
        SCENARIO_IDLE_GRACE,
    )?;
    owner.cancel_invocation(key)
}

#[allow(clippy::too_many_arguments)]
fn start_blocked_submit(
    state_root: &Path,
    identity: &CoreIdentity,
    clock: Arc<ScenarioEpochClock>,
    control: Arc<ReceiptScenarioControl>,
    telemetry: Arc<V5ReceiptRuntimeTelemetry>,
    invocation: V5InvocationRequest,
    label: String,
    accepted_epoch_ms: u64,
    response_budget_ms: u64,
) -> Result<PendingSubmit, String> {
    control.set_operation_label(label.clone());
    let accepted_monotonic_ms = clock.now_monotonic_millis();
    let task_projection = inspect_task_projection(
        state_root,
        identity,
        Arc::clone(&clock),
        &[],
        &HashMap::new(),
    )?;
    let task_store_create_attempts = telemetry.snapshot().task_store_create_attempts;
    // The submitting process owns the attempt it is about to run, and any
    // fixture the scenario seeded for it.
    control.arm_skip_next_startup_reconciliation();
    let config = scenario_server_config_with_clock(state_root, identity, Some(&control), &clock);
    let (actor_tx, actor_rx) = mpsc::sync_channel(1);
    let daemon = ScenarioDaemon::spawn(config, move |runtime| {
        let mut runtime =
            runtime.with_hooks_for_test(ScenarioHooks::install(telemetry, Some(control)));
        runtime.epoch_clock = clock;
        actor_tx
            .send(runtime.receipt_ledger.clone())
            .expect("scenario actor observer receiver must remain live");
        runtime
    });
    if let Err(wait_error) = wait_for_endpoint(state_root, identity) {
        let cleanup = daemon
            .stop_and_join("protocol-v5 receipt scenario daemon panicked during startup failure");
        return Err(match cleanup {
            Ok(()) => wait_error,
            Err(startup_error) => format!("{wait_error}; daemon startup failed: {startup_error}"),
        });
    }
    let actor = actor_rx
        .recv_timeout(SCENARIO_OPERATION_TIMEOUT)
        .map_err(|_| "protocol-v5 receipt scenario actor observer was not published".to_owned())?;
    let client_state_root = state_root.to_path_buf();
    let client_identity = identity.clone();
    let client = thread::spawn(move || {
        let mut owner = V5DaemonProcessOwner::connect_or_spawn(
            &client_state_root,
            client_identity.clone(),
            std::path::PathBuf::from("unused-existing-v5-scenario-endpoint"),
            SCENARIO_IDLE_GRACE,
        )?;
        let response = owner
            .submit_invocation_with_timeout_for_test(invocation, SCENARIO_BULK_SNAPSHOT_TIMEOUT)?;
        drop(owner);
        Ok(response)
    });
    Ok(PendingSubmit {
        label,
        accepted_epoch_ms,
        accepted_monotonic_ms,
        response_budget_ms,
        actor,
        task_projection,
        task_store_create_attempts,
        response_projected: false,
        client: Some(client),
        response: None,
        daemon,
    })
}

fn wait_for_endpoint(state_root: &Path, identity: &CoreIdentity) -> Result<(), String> {
    // Production startup is allowed to consume the bounded reconciliation
    // window before publishing the listener. The harness must not pronounce a
    // healthy full-pool recovery dead after the ordinary 5-second action SLA.
    let deadline = Instant::now() + SCENARIO_ENDPOINT_STARTUP_TIMEOUT;
    loop {
        let state = DaemonStateDirectory::open(state_root, identity)?;
        if state.read_v5_endpoint_record()?.is_some() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("protocol-v5 receipt scenario endpoint was not published".to_owned());
        }
        thread::sleep(Duration::from_millis(5));
    }
}

fn snapshot_from_state(
    state_root: &Path,
    identity: &CoreIdentity,
    clock: Arc<ScenarioEpochClock>,
    telemetry: &V5ReceiptRuntimeTelemetry,
    control: &ReceiptScenarioControl,
    keys: &[ReceiptKey],
) -> Result<Value, String> {
    let state = DaemonStateDirectory::open(state_root, identity)?;
    let receipts = state.create_private_retained_subdirectory("receipts")?;
    let actor =
        open_receipt_actor_for_scenario(receipts, "open protocol-v5 receipt checkpoint store")?;
    let snapshot = snapshot_with_actor(
        &actor,
        &clock,
        telemetry,
        control.side_effect_markers(),
        keys,
    );
    drop(actor);
    snapshot
}

fn snapshot_with_actor(
    actor: &ReceiptLedgerActor,
    clock: &ScenarioEpochClock,
    telemetry: &V5ReceiptRuntimeTelemetry,
    side_effect_markers: u64,
    keys: &[ReceiptKey],
) -> Result<Value, String> {
    // The catalog snapshot below is proportional to all retained rows, not to
    // the caller's small list of live keys.  A horizon load can therefore have
    // 32 requested keys and tens of thousands of acknowledged tombstones.
    let snapshot_timeout = SCENARIO_BULK_SNAPSHOT_TIMEOUT;
    actor
        .reclaim_expired_tombstones(clock.now_epoch_millis(), Instant::now() + snapshot_timeout)
        .map_err(|error| format!("reclaim protocol-v5 receipt tombstones: {error}"))?;
    let mut receipts = Vec::with_capacity(keys.len());
    for key in keys {
        match actor.recover(key.clone(), Instant::now() + snapshot_timeout) {
            Ok(ReceiptState::AcknowledgedTombstone(_)) => {}
            Ok(receipt) => receipts.push(receipt_observation(receipt)?),
            Err(ReceiptLedgerError::ReceiptNotFound) => {}
            Err(error) => {
                return Err(format!("snapshot protocol-v5 receipt scenario: {error}"));
            }
        }
    }
    let catalog = actor
        .snapshot_catalog(Instant::now() + snapshot_timeout)
        .map_err(|error| format!("snapshot protocol-v5 receipt catalog: {error}"))?;
    if u64::try_from(catalog.keys().len()).ok() != Some(catalog.live_count()) {
        return Err("sealed protocol-v5 receipt catalog count is inconsistent".to_owned());
    }
    let tombstones = catalog
        .tombstones()
        .iter()
        .map(tombstone_observation)
        .collect::<Vec<_>>();
    let invocation_index = catalog
        .invocation_index()
        .iter()
        .map(receipt_key_observation)
        .collect::<Vec<_>>();
    let reserved_task_index = catalog
        .reserved_task_index()
        .iter()
        .map(receipt_key_observation)
        .collect::<Vec<_>>();
    let generation = catalog.generation();
    let runtime = telemetry.snapshot();
    let token_signals = runtime
        .events
        .iter()
        .filter(|event| event.event == V5ReceiptRuntimeEventKind::TokenSignalled)
        .count();
    Ok(json!({
        "receipts": receipts,
        "tombstones": tombstones,
        "tasks": [],
        "taskLinks": [],
        "invocationIndex": invocation_index,
        "reservedTaskIndex": reserved_task_index,
        "receiptLiveCount": catalog.live_count(),
        "receiptActualBytes": catalog.actual_bytes(),
        "receiptReservedBytes": catalog.reserved_result_bytes(),
        "taskLinkCount": 0,
        "taskLinkBytes": 0,
        "taskLinkReservedCount": 0,
        "taskLinkReservedBytes": 0,
        "tombstoneCount": catalog.tombstones().len(),
        "tombstoneBytes": catalog.tombstone_bytes(),
        "callbacks": runtime.callbacks,
        "listener": runtime.listener,
        "restartRequested": runtime.restart_requested,
        "daemonRunning": runtime.daemon_running,
        "actorLeases": runtime.actor_leases,
        "sideEffectMarkers": side_effect_markers,
        "taskStoreCreateAttempts": runtime.task_store_create_attempts,
        "tokenSignals": token_signals,
        "storeGeneration": generation,
        "epochMs": clock.now_epoch_millis(),
        "processExitElapsedMs": null,
        "cancelAuthority": null,
        "receiptStoreMutations": generation,
        "taskStoreMutations": 0,
        "fallbackExecutions": 0,
        "stagedResponsesExposed": 0
    }))
}

struct BulkReceiptCatalogObservation {
    tombstones: Vec<Value>,
    indexed_keys: Vec<(String, Value)>,
    tombstone_bytes: u64,
}

impl BulkReceiptCatalogObservation {
    fn retain_unexpired(&mut self, observed_at_epoch_ms: u64) -> Result<usize, String> {
        let before = self.tombstones.len();
        self.tombstones.retain(|tombstone| {
            tombstone
                .get("expiresEpochMs")
                .and_then(Value::as_u64)
                .is_some_and(|expires_at| observed_at_epoch_ms < expires_at)
        });
        let retained = self
            .tombstones
            .iter()
            .map(|tombstone| {
                tombstone
                    .get("key")
                    .and_then(|key| key.get("keyDigest"))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .ok_or_else(|| "bulk tombstone has no key digest".to_owned())
            })
            .collect::<Result<HashSet<_>, _>>()?;
        self.indexed_keys
            .retain(|(digest, _)| retained.contains(digest));
        self.tombstone_bytes = self
            .tombstones
            .iter()
            .filter_map(|tombstone| tombstone.get("encodedBytes").and_then(Value::as_u64))
            .sum();
        Ok(before.saturating_sub(self.tombstones.len()))
    }
}

fn snapshot_with_actor_and_bulk_catalog(
    actor: &ReceiptLedgerActor,
    clock: &ScenarioEpochClock,
    telemetry: &V5ReceiptRuntimeTelemetry,
    side_effect_markers: u64,
    bulk: &BulkReceiptCatalogObservation,
) -> Result<Value, String> {
    let deadline = Instant::now() + SCENARIO_BULK_SNAPSHOT_TIMEOUT;
    // `known_keys` also contains every key from a bulk TaskStore fixture. Probing
    // those thousands of Task-only keys through ReceiptLedger is both semantically
    // wrong and slow enough to consume the shared Windows snapshot deadline. The
    // sealed receipt catalog is the authoritative bounded set of active receipts.
    let active_catalog = actor
        .snapshot_catalog(deadline)
        .map_err(|error| format!("snapshot active bulk protocol-v5 receipt catalog: {error}"))?;
    let mut receipts = Vec::new();
    let mut tombstones = bulk.tombstones.clone();
    let mut indexed_keys = bulk.indexed_keys.clone();
    let mut indexed_digests = indexed_keys
        .iter()
        .map(|(digest, _)| digest.clone())
        .collect::<HashSet<_>>();
    let mut receipt_actual_bytes = 0_u64;
    let mut receipt_reserved_bytes = 0_u64;
    let mut tombstone_bytes = bulk.tombstone_bytes;
    // The compact bulk observation mirrors tombstones already present in the
    // store. Merge the actor catalog by digest so a tombstone created after the
    // bulk fixture (for example, the post-reclaim acknowledgement) is visible
    // without duplicating the seeded pool.
    for tombstone in active_catalog.tombstones() {
        let digest = tombstone.key_digest().as_str().to_owned();
        if indexed_digests.insert(digest.clone()) {
            tombstone_bytes = tombstone_bytes.saturating_add(tombstone.encoded_bytes());
            indexed_keys.push((digest, receipt_key_observation(tombstone.key())));
            tombstones.push(tombstone_observation(tombstone));
        }
    }
    for key in active_catalog.keys() {
        match actor.recover(key.clone(), deadline) {
            Ok(ReceiptState::AcknowledgedTombstone(tombstone)) => {
                let digest = tombstone.key_digest().as_str().to_owned();
                if indexed_digests.insert(digest.clone()) {
                    tombstone_bytes = tombstone_bytes.saturating_add(tombstone.encoded_bytes());
                    indexed_keys.push((digest, receipt_key_observation(tombstone.key())));
                    tombstones.push(tombstone_observation(&tombstone));
                }
            }
            Ok(receipt) => {
                let observation = receipt_observation(receipt)?;
                receipt_actual_bytes = receipt_actual_bytes.saturating_add(
                    observation
                        .get("encodedBytes")
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                );
                receipt_reserved_bytes = receipt_reserved_bytes.saturating_add(
                    observation
                        .get("reservedResultBytes")
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                );
                let key_observation = observation
                    .get("key")
                    .cloned()
                    .ok_or_else(|| "bulk receipt observation has no key".to_owned())?;
                let digest = key_observation
                    .get("keyDigest")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "bulk receipt key has no digest".to_owned())?
                    .to_owned();
                if indexed_digests.insert(digest.clone()) {
                    indexed_keys.push((digest, key_observation));
                }
                receipts.push(observation);
            }
            Err(ReceiptLedgerError::ReceiptNotFound) => {}
            Err(error) => return Err(format!("snapshot bulk protocol-v5 receipt: {error}")),
        }
    }
    indexed_keys.sort_unstable_by(|(left, _), (right, _)| left.cmp(right));
    let invocation_index = indexed_keys
        .into_iter()
        .map(|(_, key)| key)
        .collect::<Vec<_>>();
    let reserved_task_index = invocation_index.clone();
    let generation = actor
        .generation(deadline)
        .map_err(|error| format!("snapshot bulk protocol-v5 generation: {error}"))?;
    let runtime = telemetry.snapshot();
    let token_signals = runtime
        .events
        .iter()
        .filter(|event| event.event == V5ReceiptRuntimeEventKind::TokenSignalled)
        .count();
    let tombstone_count = tombstones.len();

    Ok(json!({
        "receipts": receipts,
        "tombstones": tombstones,
        "tasks": [],
        "taskLinks": [],
        "invocationIndex": invocation_index,
        "reservedTaskIndex": reserved_task_index,
        "receiptLiveCount": receipts.len(),
        "receiptActualBytes": receipt_actual_bytes,
        "receiptReservedBytes": receipt_reserved_bytes,
        "taskLinkCount": 0,
        "taskLinkBytes": 0,
        "taskLinkReservedCount": 0,
        "taskLinkReservedBytes": 0,
        "tombstoneCount": tombstone_count,
        "tombstoneBytes": tombstone_bytes,
        "callbacks": runtime.callbacks,
        "listener": runtime.listener,
        "restartRequested": runtime.restart_requested,
        "daemonRunning": runtime.daemon_running,
        "actorLeases": runtime.actor_leases,
        "sideEffectMarkers": side_effect_markers,
        "taskStoreCreateAttempts": runtime.task_store_create_attempts,
        "tokenSignals": token_signals,
        "storeGeneration": generation,
        "epochMs": clock.now_epoch_millis(),
        "processExitElapsedMs": null,
        "cancelAuthority": null,
        "receiptStoreMutations": generation,
        "taskStoreMutations": 0,
        "fallbackExecutions": 0,
        "stagedResponsesExposed": 0
    }))
}

#[derive(Clone)]
struct TaskProjectionObservation {
    tasks: Vec<Value>,
    task_links: Vec<Value>,
    task_link_count: u64,
    task_link_bytes: u64,
    task_link_reserved_count: u64,
    task_link_reserved_bytes: u64,
    task_store_mutations: u64,
    generation: u64,
}

fn bound_task_projection_observation(
    bound_task: ScenarioBoundTask,
    state_root: &Path,
    identity: &CoreIdentity,
) -> Result<TaskProjectionObservation, String> {
    let ScenarioBoundTask { record, bound } = bound_task;
    let workspace = serde_json::to_value(bound.link().workspace_identity_hash())
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned));
    let mut task = task_observation_from_response_with_workspace(
        V5ServerResponse::Task {
            snapshot: super::task_store_snapshot(&record),
        },
        bound.key(),
        state_root,
        identity,
        Some(workspace),
    )?;
    task["encodedBytes"] = Value::from(
        u64::try_from(
            serde_json::to_vec(&record)
                .map_err(|error| format!("encode bound Task record evidence: {error}"))?
                .len(),
        )
        .map_err(|_| "bound Task record evidence length exceeds u64".to_owned())?,
    );
    let link = TaskLifecycleLinkRecord::TaskBound(bound.clone());
    let link_observation = task_lifecycle_link_observation(&link, &record)?;
    Ok(TaskProjectionObservation {
        tasks: vec![task],
        task_links: vec![link_observation],
        task_link_count: 1,
        task_link_bytes: bound.encoded_bytes(),
        task_link_reserved_count: 0,
        task_link_reserved_bytes: 0,
        task_store_mutations: record.version,
        generation: bound.mutation_sequence().saturating_add(record.version),
    })
}

fn terminal_bound_task_projection_observation(
    bound_task: ScenarioTerminalBoundTask,
    state_root: &Path,
    identity: &CoreIdentity,
) -> Result<TaskProjectionObservation, String> {
    let ScenarioTerminalBoundTask { record, bound } = bound_task;
    let workspace = serde_json::to_value(bound.link().workspace_identity_hash())
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned));
    let mut task = task_observation_from_response_with_workspace(
        V5ServerResponse::Task {
            snapshot: super::task_store_snapshot(&record),
        },
        bound.key(),
        state_root,
        identity,
        Some(workspace),
    )?;
    task["encodedBytes"] = Value::from(
        u64::try_from(
            serde_json::to_vec(&record)
                .map_err(|error| format!("encode terminal Task record evidence: {error}"))?
                .len(),
        )
        .map_err(|_| "terminal Task record evidence length exceeds u64".to_owned())?,
    );
    let link = TaskLifecycleLinkRecord::TaskTerminalBound(bound.clone());
    let link_observation = task_lifecycle_link_observation(&link, &record)?;
    Ok(TaskProjectionObservation {
        tasks: vec![task],
        task_links: vec![link_observation],
        task_link_count: 1,
        task_link_bytes: bound.encoded_bytes(),
        task_link_reserved_count: 0,
        task_link_reserved_bytes: 0,
        task_store_mutations: record.version,
        generation: bound.mutation_sequence().saturating_add(record.version),
    })
}

fn controlled_task_projection_observation(
    control: &ReceiptScenarioControl,
    state_root: &Path,
    identity: &CoreIdentity,
) -> Result<Option<TaskProjectionObservation>, String> {
    let mut projections = Vec::new();
    for task in control.bound_tasks() {
        projections.push(bound_task_projection_observation(
            task, state_root, identity,
        )?);
    }
    for task in control.terminal_bound_tasks() {
        projections.push(terminal_bound_task_projection_observation(
            task, state_root, identity,
        )?);
    }
    if projections.is_empty() {
        return Ok(None);
    }
    let mut combined = TaskProjectionObservation {
        tasks: Vec::new(),
        task_links: Vec::new(),
        task_link_count: 0,
        task_link_bytes: 0,
        task_link_reserved_count: 0,
        task_link_reserved_bytes: 0,
        task_store_mutations: 0,
        generation: 0,
    };
    for projection in projections {
        combined.tasks.extend(projection.tasks);
        combined.task_links.extend(projection.task_links);
        combined.task_link_count = combined
            .task_link_count
            .saturating_add(projection.task_link_count);
        combined.task_link_bytes = combined
            .task_link_bytes
            .saturating_add(projection.task_link_bytes);
        combined.task_link_reserved_count = combined
            .task_link_reserved_count
            .saturating_add(projection.task_link_reserved_count);
        combined.task_link_reserved_bytes = combined
            .task_link_reserved_bytes
            .saturating_add(projection.task_link_reserved_bytes);
        combined.task_store_mutations = combined
            .task_store_mutations
            .saturating_add(projection.task_store_mutations);
        combined.generation = combined.generation.max(projection.generation);
    }
    let owner_key = |value: &Value| {
        value
            .get("key")
            .or_else(|| value.get("receiptKey"))
            .and_then(|key| key.get("keyDigest"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    combined.tasks.sort_by_key(&owner_key);
    combined.task_links.sort_by_key(owner_key);
    Ok(Some(combined))
}

fn merge_exact_runtime_task_projection(
    projection: &mut TaskProjectionObservation,
    runtime: &V5ReceiptRuntime,
    key: &ReceiptKey,
    state_root: &Path,
    identity: &CoreIdentity,
) -> Result<(), String> {
    let deadline = crate::domain::code_intelligence::ProviderDeadline::new(
        Instant::now() + SCENARIO_OPERATION_TIMEOUT,
    );
    let record = match runtime
        .task_projection
        .task_store
        .get(key.reserved_task_id(), deadline)
    {
        Ok(record) => record,
        Err(V5TaskStoreError::NotFound { .. }) => {
            // A capacity fallback deliberately leaves the exact terminal
            // receipt-owned. The bulk Task projection already contains every
            // materialized lifecycle owner, so there is nothing to merge.
            return Ok(());
        }
        Err(error) => return Err(format!("read reconciled exact Task projection: {error}")),
    };
    let catalog = runtime
        .task_projection
        .lifecycle_links
        .catalog_snapshot(deadline)
        .map_err(|error| format!("read reconciled exact lifecycle link: {error}"))?;
    let link = catalog
        .entries()
        .iter()
        .find_map(|entry| match entry {
            TaskLifecycleLinkCatalogEntry::Record(link)
                if link.key() == key
                    && match link {
                        TaskLifecycleLinkRecord::TaskBound(link) => link.task().task_id(),
                        TaskLifecycleLinkRecord::TaskTerminalBound(link) => link.task().task_id(),
                        TaskLifecycleLinkRecord::TaskRetirementPending(link) => {
                            link.task().task_id()
                        }
                    } == record.task_id =>
            {
                Some(link)
            }
            TaskLifecycleLinkCatalogEntry::Reservation(_)
            | TaskLifecycleLinkCatalogEntry::Record(_) => None,
        })
        .ok_or_else(|| "reconciled exact Task has no lifecycle link".to_owned())?;
    let workspace_hash = match link {
        TaskLifecycleLinkRecord::TaskBound(link) => link.link().workspace_identity_hash(),
        TaskLifecycleLinkRecord::TaskTerminalBound(link) => link.link().workspace_identity_hash(),
        TaskLifecycleLinkRecord::TaskRetirementPending(link) => {
            link.link().workspace_identity_hash()
        }
    };
    let workspace = serde_json::to_value(workspace_hash)
        .map_err(|error| format!("encode reconciled exact workspace identity: {error}"))?
        .as_str()
        .map(str::to_owned);
    let mut task = task_observation_from_response_with_workspace(
        V5ServerResponse::Task {
            snapshot: super::task_store_snapshot(&record),
        },
        key,
        state_root,
        identity,
        Some(workspace),
    )?;
    task["encodedBytes"] = Value::from(
        u64::try_from(
            serde_json::to_vec(&record)
                .map_err(|error| format!("encode reconciled exact Task: {error}"))?
                .len(),
        )
        .map_err(|_| "reconciled exact Task length exceeds u64".to_owned())?,
    );
    let link_observation = task_lifecycle_link_observation(link, &record)?;
    let task_id_value = serde_json::to_value(record.task_id)
        .map_err(|error| format!("encode reconciled exact Task id: {error}"))?;
    if let Some(existing) = projection
        .tasks
        .iter_mut()
        .find(|task| task.get("taskId") == Some(&task_id_value))
    {
        *existing = task;
    } else {
        projection.tasks.push(task);
    }
    if let Some(existing) = projection
        .task_links
        .iter_mut()
        .find(|candidate| candidate.get("taskId") == Some(&task_id_value))
    {
        *existing = link_observation;
    } else {
        projection.task_links.push(link_observation);
    }
    projection.task_link_count =
        u64::try_from(catalog.count().saturating_sub(catalog.reserved_count()))
            .map_err(|_| "reconciled lifecycle-link count exceeds u64".to_owned())?;
    projection.task_link_bytes = catalog.actual_bytes();
    projection.task_link_reserved_count = u64::try_from(catalog.reserved_count())
        .map_err(|_| "reconciled reservation count exceeds u64".to_owned())?;
    projection.task_link_reserved_bytes = catalog.reserved_bytes();
    projection.task_store_mutations = projection
        .task_store_mutations
        .saturating_add(record.version);
    projection.generation = catalog
        .generation()
        .saturating_add(projection.task_store_mutations);
    Ok(())
}

fn inspect_task_projection(
    state_root: &Path,
    identity: &CoreIdentity,
    clock: Arc<ScenarioEpochClock>,
    known_keys: &[ReceiptKey],
    seeded_task_versions: &HashMap<TaskId, u64>,
) -> Result<TaskProjectionObservation, String> {
    let state = DaemonStateDirectory::open(state_root, identity)?;
    let snapshot_timeout = if known_keys.len() > 1_000 {
        Duration::from_secs(30)
    } else {
        SCENARIO_OPERATION_TIMEOUT
    };
    let deadline =
        crate::domain::code_intelligence::ProviderDeadline::new(Instant::now() + snapshot_timeout);
    let task_root = state.create_private_retained_subdirectory("tasks")?;
    let (task_store, recovery) =
        FileInvocationStoreV5::open_retained_directory_inspect_only(task_root, clock, deadline)
            .map_err(|error| format!("inspect protocol-v5 TaskStore snapshot: {error}"))?;
    let link_root = state.create_private_retained_subdirectory("task-lifecycle-links")?;
    let link_store = TaskLifecycleLinkStoreV5::open(link_root.path(), deadline)
        .map_err(|error| format!("inspect protocol-v5 lifecycle-link snapshot: {error}"))?;
    let catalog = link_store
        .catalog_snapshot(deadline)
        .map_err(|error| format!("snapshot protocol-v5 lifecycle-link catalog: {error}"))?;

    let mut key_by_task = HashMap::new();
    let mut lifecycle_owners = Vec::new();
    for key in known_keys {
        key_by_task.insert(key.reserved_task_id(), key.clone());
    }
    for entry in catalog.entries() {
        let key = match entry {
            TaskLifecycleLinkCatalogEntry::Reservation(reservation) => reservation.key(),
            TaskLifecycleLinkCatalogEntry::Record(TaskLifecycleLinkRecord::TaskBound(record)) => {
                record.key()
            }
            TaskLifecycleLinkCatalogEntry::Record(TaskLifecycleLinkRecord::TaskTerminalBound(
                record,
            )) => record.key(),
            TaskLifecycleLinkCatalogEntry::Record(
                TaskLifecycleLinkRecord::TaskRetirementPending(record),
            ) => record.key(),
        };
        key_by_task
            .entry(key.reserved_task_id())
            .or_insert_with(|| key.clone());
        lifecycle_owners.push(key.clone());
    }

    let mut task_links = Vec::new();
    for entry in catalog.entries() {
        let TaskLifecycleLinkCatalogEntry::Record(record) = entry else {
            continue;
        };
        let task_id = match record {
            TaskLifecycleLinkRecord::TaskBound(record) => record.task().task_id(),
            TaskLifecycleLinkRecord::TaskTerminalBound(record) => record.task().task_id(),
            TaskLifecycleLinkRecord::TaskRetirementPending(record) => record.task().task_id(),
        };
        let task_record = match task_store.get(task_id, deadline) {
            Ok(record) => record,
            Err(V5TaskStoreError::NotFound { .. }) => match record {
                TaskLifecycleLinkRecord::TaskRetirementPending(pending) => {
                    synthetic_task_record_from_pending(pending)
                }
                TaskLifecycleLinkRecord::TaskBound(bound) => {
                    synthetic_task_record_from_bound(bound)
                }
                _ => {
                    return Err(format!(
                        "read lifecycle-linked TaskStore record: Task {task_id} was not found"
                    ))
                }
            },
            Err(error) => return Err(format!("read lifecycle-linked TaskStore record: {error}")),
        };
        task_links.push(task_lifecycle_link_observation(record, &task_record)?);
    }
    drop(link_store);

    let mut tasks = Vec::new();
    let mut task_store_mutations = 0u64;
    for entry in recovery.entries() {
        let task_id = entry.identity().task_id();
        let key = key_by_task.get(&task_id).ok_or_else(|| {
            format!("active TaskStore record {task_id} has no exact lifecycle-link owner")
        })?;
        let record = task_store
            .get(task_id, deadline)
            .map_err(|error| format!("read protocol-v5 TaskStore snapshot: {error}"))?;
        task_store_mutations = task_store_mutations.saturating_add(
            record
                .version
                .saturating_sub(seeded_task_versions.get(&task_id).copied().unwrap_or(0)),
        );
        tasks.push(task_observation_from_response(
            V5ServerResponse::Task {
                snapshot: super::task_store_snapshot(&record),
            },
            key,
            state_root,
            identity,
        )?);
    }

    let task_link_count = u64::try_from(task_links.len())
        .map_err(|_| "Task lifecycle-link count exceeds u64".to_owned())?;
    // Deliberately corrupt startup fixtures may contain a provisional Task
    // without its mandatory reservation. Keep the report's capacity
    // accounting conservative so the matrix can inspect the fail-stop state;
    // startup reconciliation still reads the actual durable catalog and must
    // reject the missing owner.
    let reservation_deficit = recovery
        .entries()
        .iter()
        .filter(|entry| {
            !lifecycle_owners.iter().any(|key| {
                key.reserved_task_id() == entry.identity().task_id()
                    && key.invocation_id() == entry.identity().invocation_id()
                    && receipt_key_digest(key) == *entry.identity().receipt_key_digest()
            })
        })
        .count();
    let reserved_count = u64::try_from(
        catalog
            .reserved_count()
            .checked_add(reservation_deficit)
            .ok_or_else(|| "Task lifecycle reservation deficit overflow".to_owned())?,
    )
    .map_err(|_| "Task lifecycle-link reservation count exceeds u64".to_owned())?;
    Ok(TaskProjectionObservation {
        tasks,
        task_links,
        task_link_count,
        task_link_bytes: catalog.actual_bytes(),
        task_link_reserved_count: reserved_count,
        task_link_reserved_bytes: reserved_count.saturating_mul(1_024),
        task_store_mutations,
        generation: catalog.generation().saturating_add(task_store_mutations),
    })
}

fn synthetic_task_record_from_bound(bound: &TaskBoundReceipt) -> V5StoredInvocationRecord {
    V5StoredInvocationRecord {
        schema_version: V5StoredInvocationSchemaVersion,
        task_id: bound.task().task_id(),
        invocation_id: bound.task().invocation_id(),
        receipt_key_digest: bound.key_digest().clone(),
        tool: bound.key().tool(),
        normalized_arguments_hash: bound.key().normalized_arguments_hash().clone(),
        workspace_identity_hash: bound.link().workspace_identity_hash().clone(),
        created_at_epoch_ms: bound.task().created_at_epoch_ms(),
        updated_at_epoch_ms: bound.task().updated_at_epoch_ms(),
        ttl_ms: bound.task().ttl_ms(),
        poll_interval_ms: bound.task().poll_interval_ms(),
        version: bound.task_record_version(),
        cancel_requested: false,
        task: if bound.phase() == crate::application::receipt_ledger::AttemptPhase::Begun {
            V5StoredTask::Working
        } else {
            V5StoredTask::Queued
        },
    }
}

fn synthetic_task_record_from_pending(
    pending: &TaskRetirementPendingReceipt,
) -> V5StoredInvocationRecord {
    let task = match pending.terminal_status() {
        crate::application::receipt_ledger::ClosedTerminalStatus::Completed => {
            V5StoredTask::Completed {
                terminal_epoch_ms: pending.terminal_epoch_ms(),
                terminal_digest: pending.terminal_digest().clone(),
                result: Box::new(DomainResult::success("retired-terminal-evidence")),
            }
        }
        crate::application::receipt_ledger::ClosedTerminalStatus::Failed => V5StoredTask::Failed {
            terminal_epoch_ms: pending.terminal_epoch_ms(),
            terminal_digest: pending.terminal_digest().clone(),
            reason: V5SafeFailureReason::InvocationFailed,
        },
        crate::application::receipt_ledger::ClosedTerminalStatus::Cancelled => {
            V5StoredTask::Cancelled {
                terminal_epoch_ms: pending.terminal_epoch_ms(),
                terminal_digest: pending.terminal_digest().clone(),
            }
        }
    };
    V5StoredInvocationRecord {
        schema_version: V5StoredInvocationSchemaVersion,
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

fn apply_task_projection(
    snapshot: &mut Value,
    projection: &TaskProjectionObservation,
) -> Result<(), String> {
    let object = snapshot
        .as_object_mut()
        .ok_or_else(|| "protocol-v5 checkpoint snapshot is not an object".to_owned())?;
    object.insert("tasks".to_owned(), Value::Array(projection.tasks.clone()));
    object.insert(
        "taskLinks".to_owned(),
        Value::Array(projection.task_links.clone()),
    );
    object.insert(
        "taskLinkCount".to_owned(),
        projection.task_link_count.into(),
    );
    object.insert(
        "taskLinkBytes".to_owned(),
        projection.task_link_bytes.into(),
    );
    object.insert(
        "taskLinkReservedCount".to_owned(),
        projection.task_link_reserved_count.into(),
    );
    object.insert(
        "taskLinkReservedBytes".to_owned(),
        projection.task_link_reserved_bytes.into(),
    );
    object.insert(
        "taskStoreMutations".to_owned(),
        projection.task_store_mutations.into(),
    );
    for index_name in ["invocationIndex", "reservedTaskIndex"] {
        let index = object
            .get_mut(index_name)
            .and_then(Value::as_array_mut)
            .ok_or_else(|| format!("protocol-v5 checkpoint {index_name} is not an array"))?;
        for key in projection
            .task_links
            .iter()
            .filter_map(|link| link.get("key"))
        {
            if !index.iter().any(|existing| existing == key) {
                index.push(key.clone());
            }
        }
    }
    let receipt_generation = object
        .get("storeGeneration")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    object.insert(
        "storeGeneration".to_owned(),
        receipt_generation
            .saturating_add(projection.generation)
            .into(),
    );
    let receipt_owned = object
        .get("receipts")
        .and_then(Value::as_array)
        .is_some_and(|receipts| {
            receipts.iter().any(|receipt| {
                receipt
                    .get("state")
                    .and_then(Value::as_str)
                    .is_some_and(|state| {
                        state.starts_with("task_promised_") || state.starts_with("task_handoff_")
                    })
            })
        });
    object.insert(
        "cancelAuthority".to_owned(),
        if receipt_owned {
            Value::String("receipt_ledger".to_owned())
        } else if projection.task_link_count > 0 || projection.task_link_reserved_count > 0 {
            Value::String("task_store".to_owned())
        } else {
            Value::Null
        },
    );
    Ok(())
}

fn enrich_task_projection_snapshot(
    snapshot: &mut Value,
    state_root: &Path,
    identity: &CoreIdentity,
    clock: Arc<ScenarioEpochClock>,
    known_keys: &[ReceiptKey],
    seeded_task_versions: &HashMap<TaskId, u64>,
) -> Result<(), String> {
    let projection = inspect_task_projection(
        state_root,
        identity,
        clock,
        known_keys,
        seeded_task_versions,
    )?;
    apply_task_projection(snapshot, &projection)
}

fn task_lifecycle_link_observation(
    record: &TaskLifecycleLinkRecord,
    task_record: &V5StoredInvocationRecord,
) -> Result<Value, String> {
    let (key, link, encoded_bytes, version, lifecycle) = match record {
        TaskLifecycleLinkRecord::TaskBound(record) => {
            let state = match record.phase() {
                crate::application::receipt_ledger::AttemptPhase::NotBegun => {
                    "task_bound_not_begun"
                }
                crate::application::receipt_ledger::AttemptPhase::Begun => "task_bound_begun",
            };
            (
                record.key(),
                record.link(),
                record.encoded_bytes(),
                record.lifecycle_link_version(),
                json!({
                    "state": state,
                    "cancel_requested": task_record.cancel_requested,
                    "task_version": record.task_record_version(),
                }),
            )
        }
        TaskLifecycleLinkRecord::TaskTerminalBound(record) => (
            record.key(),
            record.link(),
            record.encoded_bytes(),
            record.lifecycle_link_version(),
            json!({
                "state": "task_terminal_bound",
                "terminal_digest": record.terminal_digest(),
                "terminal_epoch_ms": record.terminal_epoch_ms(),
                "ttl_ms": record.task().ttl_ms(),
                "expires_at_epoch_ms": record.expires_at_epoch_ms(),
                "task_version": record.task_record_version(),
            }),
        ),
        TaskLifecycleLinkRecord::TaskRetirementPending(record) => {
            let lifecycle_link_expected_version = record
                .lifecycle_link_version()
                .checked_sub(1)
                .ok_or_else(|| "TaskRetirementPending has no predecessor version".to_owned())?;
            let pending_record = json!({
                "receiptKey": receipt_key_observation(record.key()),
                "taskId": record.task().task_id(),
                "taskLinkDigest": record.link().digest(),
                "terminalDigest": record.terminal_digest(),
                "terminalEpochMs": record.terminal_epoch_ms(),
                "ttlMs": record.task().ttl_ms(),
                "expiresAtEpochMs": record.expires_at_epoch_ms(),
                "expectedTaskVersion": record.expected_terminal_task_version(),
                "resolver": "task_expired",
                "version": record.lifecycle_link_version(),
            });
            let encoded = serde_json::to_vec(&pending_record)
                .map_err(|error| format!("encode TaskRetirementPending evidence: {error}"))?;
            (
                record.key(),
                record.link(),
                record.encoded_bytes(),
                record.lifecycle_link_version(),
                json!({
                    "state": "task_retirement_pending",
                    "pending": {
                        "receiptKey": receipt_key_observation(record.key()),
                        "taskId": record.task().task_id(),
                        "taskLinkDigest": record.link().digest(),
                        "terminalDigest": record.terminal_digest(),
                        "terminalEpochMs": record.terminal_epoch_ms(),
                        "ttlMs": record.task().ttl_ms(),
                        "expiresAtEpochMs": record.expires_at_epoch_ms(),
                        "expectedTaskVersion": record.expected_terminal_task_version(),
                        "resolver": "task_expired",
                        "version": record.lifecycle_link_version(),
                        "lifecycleLinkExpectedVersion": lifecycle_link_expected_version,
                        "committedLifecycleLinkVersion": record.lifecycle_link_version(),
                        "committedPendingRecord": artifact_evidence(&encoded),
                    }
                }),
            )
        }
    };
    Ok(json!({
        "key": receipt_key_observation(key),
        "taskId": link.task_id(),
        "invocationId": link.invocation_id(),
        "workspaceIdentityHash": link.workspace_identity_hash(),
        "linkDigest": link.digest(),
        "encodedBytes": encoded_bytes,
        "version": version,
        "lifecycle": lifecycle,
    }))
}

fn tombstone_observation(
    receipt: &crate::application::receipt_ledger::AcknowledgedTombstoneReceipt,
) -> Value {
    json!({
        "key": receipt_key_observation(receipt.key()),
        "terminalDigest": receipt.terminal_digest(),
        "ackEpochMs": receipt.acknowledged_at_epoch_ms(),
        "expiresEpochMs": receipt.expires_at_epoch_ms(),
        "encodedBytes": receipt.encoded_bytes()
    })
}

fn receipt_observation(state: ReceiptState) -> Result<Value, String> {
    let observation = match state {
        ReceiptState::CancelReserved(receipt) => ScenarioReceiptObservation {
            key: receipt_key_observation(receipt.key()),
            state: "cancel_reserved",
            cancel_requested: true,
            accepted_epoch_ms: receipt.cancel_reserved_at_epoch_ms(),
            original_budget_ms: 0,
            expires_epoch_ms: Some(receipt.expires_at_epoch_ms()),
            bound_workspace_identity: None,
            staged_terminal: None,
            terminal: None,
            encoded_bytes: receipt.encoded_bytes(),
            reserved_result_bytes: 0,
            version: receipt.record_version().get(),
            mutation_sequence: receipt.mutation_sequence(),
            begun: false,
        },
        ReceiptState::Reserved(receipt) => {
            let (state, bound_workspace_identity, begun) = match receipt.phase() {
                ReservedPhase::Unbound => ("reserved_unbound", None, false),
                ReservedPhase::ActorBound {
                    bound_workspace_identity,
                } => (
                    "reserved_actor_bound",
                    serde_json::to_value(bound_workspace_identity)
                        .ok()
                        .and_then(|value| value.as_str().map(str::to_owned)),
                    false,
                ),
                ReservedPhase::Begun {
                    bound_workspace_identity,
                } => (
                    "reserved_begun",
                    serde_json::to_value(bound_workspace_identity)
                        .ok()
                        .and_then(|value| value.as_str().map(str::to_owned)),
                    true,
                ),
            };
            ScenarioReceiptObservation {
                key: receipt_key_observation(receipt.key()),
                state,
                cancel_requested: receipt.cancel_requested(),
                accepted_epoch_ms: receipt.original_cutoff().accepted_epoch_ms(),
                original_budget_ms: receipt.original_cutoff().response_budget_ms(),
                expires_epoch_ms: None,
                bound_workspace_identity,
                staged_terminal: None,
                terminal: None,
                encoded_bytes: receipt.encoded_bytes(),
                reserved_result_bytes: receipt.reserved_result_bytes(),
                version: receipt.record_version().get(),
                mutation_sequence: receipt.mutation_sequence(),
                begun,
            }
        }
        ReceiptState::DirectTerminalUnacked(receipt) => ScenarioReceiptObservation {
            key: receipt_key_observation(receipt.key()),
            state: "direct_terminal_unacked",
            cancel_requested: matches!(
                receipt.terminal().outcome(),
                ReceiptTerminalOutcome::Cancelled
            ),
            accepted_epoch_ms: receipt.original_cutoff().accepted_epoch_ms(),
            original_budget_ms: receipt.original_cutoff().response_budget_ms(),
            expires_epoch_ms: Some(
                receipt
                    .terminal_epoch_ms()
                    .checked_add(DIRECT_TERMINAL_RETENTION_MS)
                    .ok_or_else(|| "direct terminal observation expiry overflow".to_owned())?,
            ),
            bound_workspace_identity: None,
            staged_terminal: None,
            terminal: Some(terminal_observation(
                receipt.terminal().outcome(),
                receipt.terminal_epoch_ms(),
            )?),
            encoded_bytes: receipt.encoded_bytes(),
            reserved_result_bytes: receipt.reserved_result_bytes(),
            version: receipt.record_version().get(),
            mutation_sequence: receipt.mutation_sequence(),
            begun: false,
        },
        ReceiptState::TaskTerminalReceiptBacked(receipt) => ScenarioReceiptObservation {
            key: receipt_key_observation(receipt.key()),
            state: "task_terminal_receipt_backed",
            cancel_requested: receipt.cancel_requested(),
            accepted_epoch_ms: receipt.task().created_at_epoch_ms(),
            original_budget_ms: 7_000,
            expires_epoch_ms: Some(receipt.expires_at_epoch_ms()),
            bound_workspace_identity: None,
            staged_terminal: None,
            terminal: Some(terminal_observation(
                receipt.terminal().outcome(),
                receipt.terminal_epoch_ms(),
            )?),
            encoded_bytes: receipt.encoded_bytes(),
            reserved_result_bytes: receipt.reserved_result_bytes(),
            version: receipt.record_version().get(),
            mutation_sequence: receipt.mutation_sequence(),
            begun: false,
        },
        ReceiptState::TaskPromisedUnbound(receipt) => ScenarioReceiptObservation {
            key: receipt_key_observation(receipt.key()),
            state: "task_promised_unbound",
            cancel_requested: receipt.cancel_requested(),
            accepted_epoch_ms: receipt.task().created_at_epoch_ms(),
            original_budget_ms: 7_000,
            expires_epoch_ms: None,
            bound_workspace_identity: None,
            staged_terminal: None,
            terminal: None,
            encoded_bytes: receipt.encoded_bytes(),
            reserved_result_bytes: receipt.reserved_result_bytes(),
            version: receipt.record_version().get(),
            mutation_sequence: receipt.mutation_sequence(),
            begun: false,
        },
        ReceiptState::TaskPromisedActorBound(receipt) => ScenarioReceiptObservation {
            key: receipt_key_observation(receipt.key()),
            state: "task_promised_actor_bound",
            cancel_requested: receipt.cancel_requested(),
            accepted_epoch_ms: receipt.task().created_at_epoch_ms(),
            original_budget_ms: 7_000,
            expires_epoch_ms: None,
            bound_workspace_identity: Some(
                serde_json::to_value(receipt.workspace_identity_hash())
                    .ok()
                    .and_then(|value| value.as_str().map(str::to_owned))
                    .ok_or_else(|| "encode promised Task workspace identity".to_owned())?,
            ),
            staged_terminal: None,
            terminal: None,
            encoded_bytes: receipt.encoded_bytes(),
            reserved_result_bytes: receipt.reserved_result_bytes(),
            version: receipt.record_version().get(),
            mutation_sequence: receipt.mutation_sequence(),
            begun: false,
        },
        ReceiptState::TaskHandoffActorBound(receipt) => ScenarioReceiptObservation {
            key: receipt_key_observation(receipt.key()),
            state: match receipt.phase() {
                crate::application::receipt_ledger::AttemptPhase::NotBegun => {
                    "task_handoff_actor_bound_not_begun"
                }
                crate::application::receipt_ledger::AttemptPhase::Begun => {
                    "task_handoff_actor_bound_begun"
                }
            },
            cancel_requested: receipt.cancel_requested(),
            accepted_epoch_ms: receipt.task().created_at_epoch_ms(),
            original_budget_ms: 7_000,
            expires_epoch_ms: None,
            bound_workspace_identity: Some(
                serde_json::to_value(receipt.workspace_identity_hash())
                    .ok()
                    .and_then(|value| value.as_str().map(str::to_owned))
                    .ok_or_else(|| "encode handoff Task workspace identity".to_owned())?,
            ),
            staged_terminal: match receipt.terminal_stage() {
                HandoffTerminalStage::NoTerminal => None,
                HandoffTerminalStage::Staged {
                    terminal_epoch_ms,
                    terminal,
                    ..
                } => Some(terminal_observation(
                    terminal.outcome(),
                    *terminal_epoch_ms,
                )?),
            },
            terminal: None,
            encoded_bytes: receipt.encoded_bytes(),
            reserved_result_bytes: receipt.reserved_result_bytes(),
            version: receipt.record_version().get(),
            mutation_sequence: receipt.mutation_sequence(),
            begun: matches!(
                receipt.phase(),
                crate::application::receipt_ledger::AttemptPhase::Begun
            ),
        },
        ReceiptState::TaskReceiptOwnedActorBound(receipt) => ScenarioReceiptObservation {
            key: receipt_key_observation(receipt.key()),
            state: "task_receipt_owned_actor_bound",
            cancel_requested: receipt.cancel_requested(),
            accepted_epoch_ms: receipt.task().created_at_epoch_ms(),
            original_budget_ms: 7_000,
            expires_epoch_ms: None,
            bound_workspace_identity: Some(
                serde_json::to_value(receipt.link().workspace_identity_hash())
                    .ok()
                    .and_then(|value| value.as_str().map(str::to_owned))
                    .ok_or_else(|| "encode receipt-owned Task workspace identity".to_owned())?,
            ),
            staged_terminal: None,
            terminal: None,
            encoded_bytes: receipt.encoded_bytes(),
            reserved_result_bytes: receipt.reserved_result_bytes(),
            version: receipt.record_version().get(),
            mutation_sequence: receipt.mutation_sequence(),
            begun: true,
        },
        other => {
            return Err(format!(
                "receipt scenario cannot project unsupported state {}",
                other.kind().diagnostic_name()
            ))
        }
    };
    serde_json::to_value(observation)
        .map_err(|error| format!("encode receipt scenario observation: {error}"))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ScenarioReceiptObservation {
    key: Value,
    state: &'static str,
    cancel_requested: bool,
    accepted_epoch_ms: u64,
    original_budget_ms: u64,
    expires_epoch_ms: Option<u64>,
    bound_workspace_identity: Option<String>,
    staged_terminal: Option<Value>,
    terminal: Option<Value>,
    encoded_bytes: u64,
    reserved_result_bytes: u64,
    version: u64,
    mutation_sequence: u64,
    begun: bool,
}

fn response_observation(
    response: &V5ServerResponse,
    submit_cutoff: Option<(u64, u64)>,
) -> Result<Value, String> {
    match response {
        V5ServerResponse::Invocation {
            outcome:
                V5InvocationResponse::ReceiptPending {
                    receipt_key,
                    phase,
                    accepted_epoch_ms,
                    original_budget_ms,
                    cancel_requested: _,
                },
        } => Ok(json!({
            "kind": match phase {
                V5InvocationPhase::CancelReserved => "cancelled",
                V5InvocationPhase::ReservedUnbound
                | V5InvocationPhase::ReservedActorBound
                | V5InvocationPhase::ReservedBegun => "pending",
            },
            "error": null,
            "terminal": null,
            "key": receipt_key_observation(receipt_key),
            "task": null,
            "acknowledgement": null,
            "cutoffEpochMs": accepted_epoch_ms.checked_add(*original_budget_ms),
            "originalBudgetMs": original_budget_ms,
            "latencyMs": 0
        })),
        V5ServerResponse::Invocation {
            outcome: V5InvocationResponse::Direct { receipt },
        } => {
            let recovery_error = match receipt.terminal() {
                ReceiptTerminalOutcome::Failed {
                    reason: V5SafeFailureReason::ResultTooLarge,
                } => {
                    let reason = V5SafeFailureReason::ResultTooLarge;
                    Some(serde_json::to_value(reason).map_err(|error| {
                        format!("encode protocol-v5 recovery failure reason: {error}")
                    })?)
                }
                ReceiptTerminalOutcome::Completed { .. }
                | ReceiptTerminalOutcome::Failed { .. }
                | ReceiptTerminalOutcome::Cancelled => None,
            };
            let (accepted_epoch_ms, original_budget_ms) = submit_cutoff
                .map(|(accepted, budget)| (Some(accepted), Some(budget)))
                .unwrap_or((None, None));
            Ok(json!({
                "kind": if matches!(receipt.terminal(), ReceiptTerminalOutcome::Cancelled) {
                    "cancelled"
                } else {
                    "direct"
                },
                "error": recovery_error,
                "terminal": terminal_observation(receipt.terminal(), receipt.terminal_epoch_ms())?,
                "key": receipt_key_observation(receipt.receipt_key()),
                "task": null,
                "acknowledgement": null,
                "cutoffEpochMs": accepted_epoch_ms
                    .zip(original_budget_ms)
                    .and_then(|(accepted, budget)| accepted.checked_add(budget)),
                "originalBudgetMs": original_budget_ms,
                "latencyMs": 0
            }))
        }
        V5ServerResponse::InvocationAcknowledged { acknowledgement } => Ok(json!({
            "kind": "acknowledged",
            "error": null,
            "terminal": null,
            "key": receipt_key_observation(acknowledgement.receipt_key()),
            "task": null,
            "acknowledgement": acknowledgement_observation(acknowledgement),
            "cutoffEpochMs": null,
            "originalBudgetMs": null,
            "latencyMs": 0
        })),
        V5ServerResponse::Invocation {
            outcome: V5InvocationResponse::Acknowledged { acknowledgement },
        } => Ok(json!({
            "kind": "tombstone",
            "error": null,
            "terminal": null,
            "key": receipt_key_observation(acknowledgement.receipt_key()),
            "task": null,
            "acknowledgement": acknowledgement_observation(acknowledgement),
            "cutoffEpochMs": null,
            "originalBudgetMs": null,
            "latencyMs": 0
        })),
        V5ServerResponse::Error { code } => Ok(json!({
            "kind": if matches!(code, V5DaemonErrorCode::ReceiptNotFound) {
                "not_found"
            } else {
                "rejected"
            },
            "error": serde_json::to_value(code)
                .map_err(|error| format!("encode protocol-v5 scenario error code: {error}"))?,
            "terminal": null,
            "key": null,
            "task": null,
            "acknowledgement": null,
            "cutoffEpochMs": null,
            "originalBudgetMs": null,
            "latencyMs": 0
        })),
        other => Err(format!(
            "receipt scenario received unsupported protocol-v5 response: {other:?}"
        )),
    }
}

fn response_observation_with_exact_task(
    response: &V5ServerResponse,
    submit_cutoff: Option<(u64, u64)>,
    receipt_key: &ReceiptKey,
    state_root: &Path,
    identity: &CoreIdentity,
    workspace_identity_override: Option<Option<String>>,
) -> Result<Value, String> {
    if matches!(
        response,
        V5ServerResponse::Task { .. }
            | V5ServerResponse::Invocation {
                outcome: V5InvocationResponse::Task { .. }
            }
    ) {
        let task = task_observation_from_response_with_workspace(
            response.clone(),
            receipt_key,
            state_root,
            identity,
            workspace_identity_override,
        )?;
        let (cutoff_epoch_ms, original_budget_ms) = submit_cutoff
            .map(|(accepted, budget)| (accepted.checked_add(budget), Some(budget)))
            .unwrap_or((None, None));
        return Ok(json!({
            "kind": "task",
            "error": null,
            "terminal": task.get("terminal").cloned().unwrap_or(Value::Null),
            "key": receipt_key_observation(receipt_key),
            "task": task,
            "acknowledgement": null,
            "cutoffEpochMs": cutoff_epoch_ms,
            "originalBudgetMs": original_budget_ms,
            "latencyMs": original_budget_ms.unwrap_or(0),
        }));
    }
    response_observation(response, submit_cutoff)
}

fn task_observation_from_response(
    response: V5ServerResponse,
    receipt_key: &ReceiptKey,
    state_root: &Path,
    identity: &CoreIdentity,
) -> Result<Value, String> {
    task_observation_from_response_with_workspace(response, receipt_key, state_root, identity, None)
}

fn task_observation_from_response_with_workspace(
    response: V5ServerResponse,
    receipt_key: &ReceiptKey,
    state_root: &Path,
    identity: &CoreIdentity,
    workspace_identity_override: Option<Option<String>>,
) -> Result<Value, String> {
    let snapshot = match response {
        V5ServerResponse::Task { snapshot }
        | V5ServerResponse::Invocation {
            outcome: V5InvocationResponse::Task { snapshot },
        } => snapshot,
        other => {
            return Err(format!(
                "Task read expected a protocol-v5 Task response, received {other:?}"
            ));
        }
    };
    let encoded_bytes = u64::try_from(
        serde_json::to_vec(&snapshot)
            .map_err(|error| format!("encode Task snapshot evidence: {error}"))?
            .len(),
    )
    .map_err(|_| "Task snapshot evidence length exceeds u64".to_owned())?;
    let (
        task_id,
        invocation_id,
        created_epoch_ms,
        updated_epoch_ms,
        ttl_ms,
        poll_interval_ms,
        version,
        cancel_requested,
        status,
        terminal_epoch_ms,
        terminal,
    ) = match snapshot {
        crate::infrastructure::daemon::protocol_v5::V5DaemonTaskSnapshot::Queued {
            task_id,
            invocation_id,
            created_at_epoch_ms,
            updated_at_epoch_ms,
            ttl_ms,
            poll_interval_ms,
            version,
            cancel_requested,
            ..
        } => (
            task_id,
            invocation_id,
            created_at_epoch_ms,
            updated_at_epoch_ms,
            ttl_ms,
            poll_interval_ms,
            version,
            cancel_requested,
            "queued",
            None,
            None,
        ),
        crate::infrastructure::daemon::protocol_v5::V5DaemonTaskSnapshot::Working {
            task_id,
            invocation_id,
            created_at_epoch_ms,
            updated_at_epoch_ms,
            ttl_ms,
            poll_interval_ms,
            version,
            cancel_requested,
            ..
        } => (
            task_id,
            invocation_id,
            created_at_epoch_ms,
            updated_at_epoch_ms,
            ttl_ms,
            poll_interval_ms,
            version,
            cancel_requested,
            "working",
            None,
            None,
        ),
        crate::infrastructure::daemon::protocol_v5::V5DaemonTaskSnapshot::Completed {
            task_id,
            invocation_id,
            created_at_epoch_ms,
            updated_at_epoch_ms,
            ttl_ms,
            poll_interval_ms,
            version,
            cancel_requested,
            terminal_epoch_ms,
            result,
            ..
        } => {
            let outcome = ReceiptTerminalOutcome::Completed { result };
            let terminal = terminal_observation(&outcome, terminal_epoch_ms)?;
            (
                task_id,
                invocation_id,
                created_at_epoch_ms,
                updated_at_epoch_ms,
                ttl_ms,
                poll_interval_ms,
                version,
                cancel_requested,
                "completed",
                Some(terminal_epoch_ms),
                Some(terminal),
            )
        }
        crate::infrastructure::daemon::protocol_v5::V5DaemonTaskSnapshot::Failed {
            task_id,
            invocation_id,
            created_at_epoch_ms,
            updated_at_epoch_ms,
            ttl_ms,
            poll_interval_ms,
            version,
            cancel_requested,
            terminal_epoch_ms,
            reason,
            ..
        } => {
            let outcome = ReceiptTerminalOutcome::Failed { reason };
            let terminal = terminal_observation(&outcome, terminal_epoch_ms)?;
            (
                task_id,
                invocation_id,
                created_at_epoch_ms,
                updated_at_epoch_ms,
                ttl_ms,
                poll_interval_ms,
                version,
                cancel_requested,
                "failed",
                Some(terminal_epoch_ms),
                Some(terminal),
            )
        }
        crate::infrastructure::daemon::protocol_v5::V5DaemonTaskSnapshot::Cancelled {
            task_id,
            invocation_id,
            created_at_epoch_ms,
            updated_at_epoch_ms,
            ttl_ms,
            poll_interval_ms,
            version,
            cancel_requested,
            terminal_epoch_ms,
            ..
        } => {
            let terminal =
                terminal_observation(&ReceiptTerminalOutcome::Cancelled, terminal_epoch_ms)?;
            (
                task_id,
                invocation_id,
                created_at_epoch_ms,
                updated_at_epoch_ms,
                ttl_ms,
                poll_interval_ms,
                version,
                cancel_requested,
                "cancelled",
                Some(terminal_epoch_ms),
                Some(terminal),
            )
        }
    };
    let expires_epoch_ms = terminal_epoch_ms
        .unwrap_or(created_epoch_ms)
        .checked_add(ttl_ms)
        .ok_or_else(|| "Task observation expiry overflow".to_owned())?;
    let workspace_identity_hash = match workspace_identity_override {
        Some(workspace) => workspace,
        None => task_link_workspace_identity(state_root, identity, task_id)?,
    };
    let projection_source = if workspace_identity_hash.is_some() {
        "task_store"
    } else {
        "receipt_ledger"
    };
    Ok(json!({
        "taskId": task_id,
        "invocationId": invocation_id,
        "receiptKey": receipt_key_observation(receipt_key),
        "status": status,
        "projectionSource": projection_source,
        "workspaceIdentityHash": workspace_identity_hash,
        "createdEpochMs": created_epoch_ms,
        "updatedEpochMs": updated_epoch_ms,
        "expiresEpochMs": expires_epoch_ms,
        "ttlMs": ttl_ms,
        "pollIntervalMs": poll_interval_ms,
        "version": version,
        "encodedBytes": encoded_bytes,
        "cancelRequested": cancel_requested,
        "terminal": terminal,
    }))
}

fn task_link_workspace_identity(
    state_root: &Path,
    identity: &CoreIdentity,
    task_id: TaskId,
) -> Result<Option<String>, String> {
    let state = DaemonStateDirectory::open(state_root, identity)?;
    let root = state.create_private_retained_subdirectory("task-lifecycle-links")?;
    let store = TaskLifecycleLinkStoreV5::open(
        root.path(),
        crate::domain::code_intelligence::ProviderDeadline::new(
            Instant::now() + SCENARIO_OPERATION_TIMEOUT,
        ),
    )
    .map_err(|error| format!("open scenario Task lifecycle-link store: {error}"))?;
    let record = match store.read_by_task_id(
        task_id,
        crate::domain::code_intelligence::ProviderDeadline::new(
            Instant::now() + SCENARIO_OPERATION_TIMEOUT,
        ),
    ) {
        Ok(record) => record,
        Err(TaskLifecycleLinkStoreError::NotFound { .. }) => return Ok(None),
        Err(error) => return Err(format!("read scenario Task lifecycle link: {error}")),
    };
    let workspace = match record {
        TaskLifecycleLinkRecord::TaskBound(record) => {
            record.link().workspace_identity_hash().clone()
        }
        TaskLifecycleLinkRecord::TaskTerminalBound(record) => {
            record.link().workspace_identity_hash().clone()
        }
        TaskLifecycleLinkRecord::TaskRetirementPending(record) => {
            record.link().workspace_identity_hash().clone()
        }
    };
    serde_json::to_value(&workspace)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .map(Some)
        .ok_or_else(|| "encode scenario Task lifecycle workspace identity".to_owned())
}

fn acknowledgement_observation(
    acknowledgement: &crate::infrastructure::daemon::protocol_v5::V5AcknowledgedReceipt,
) -> Value {
    json!({
        "ackEpochMs": acknowledgement.ack_epoch_ms(),
        "expiresEpochMs": acknowledgement.expires_epoch_ms(),
        "terminalDigest": acknowledgement.terminal_digest()
    })
}

fn exact_terminal_digest(
    state_root: &Path,
    identity: &CoreIdentity,
    clock: Arc<ScenarioEpochClock>,
    telemetry: Arc<V5ReceiptRuntimeTelemetry>,
    key: &ReceiptKey,
) -> Result<TerminalDigest, String> {
    let response = exchange_once(state_root, identity, clock, telemetry, None, |owner| {
        owner.recover_invocation_receipt(key.clone())
    })?;
    match response {
        V5ServerResponse::Invocation {
            outcome: V5InvocationResponse::Direct { receipt },
        } => Ok(receipt.terminal_digest().clone()),
        V5ServerResponse::Invocation {
            outcome: V5InvocationResponse::Acknowledged { acknowledgement },
        }
        | V5ServerResponse::InvocationAcknowledged { acknowledgement } => {
            Ok(acknowledgement.terminal_digest().clone())
        }
        other => Err(format!(
            "protocol-v5 scenario has no exact terminal digest: {other:?}"
        )),
    }
}

fn exact_terminal_digest_from_actor(
    actor: &ReceiptLedgerActor,
    key: &ReceiptKey,
    deadline: Instant,
) -> Result<TerminalDigest, String> {
    match actor
        .recover(key.clone(), deadline)
        .map_err(|error| format!("recover retained protocol-v5 terminal digest: {error}"))?
    {
        ReceiptState::DirectTerminalUnacked(receipt) => Ok(receipt.terminal().digest().clone()),
        ReceiptState::AcknowledgedTombstone(receipt) => Ok(receipt.terminal_digest().clone()),
        other => Err(format!(
            "retained protocol-v5 scenario has no exact terminal digest: {other:?}"
        )),
    }
}

fn receipt_key_observation(key: &ReceiptKey) -> Value {
    json!({
        "invocationId": key.invocation_id(),
        "reservedTaskId": key.reserved_task_id(),
        "coreIdentityDigest": key.core_identity_digest(),
        "tool": key.tool(),
        "normalizedArgumentsHash": key.normalized_arguments_hash(),
        "requestScopeHash": key.request_scope_hash(),
        "keyDigest": receipt_key_digest(key)
    })
}

fn terminal_observation(
    outcome: &ReceiptTerminalOutcome,
    terminal_epoch_ms: u64,
) -> Result<Value, String> {
    let terminal = canonical_v5_terminal(outcome)
        .map_err(|error| format!("canonicalize protocol-v5 scenario terminal: {error}"))?;
    let mut value = serde_json::to_value(outcome)
        .map_err(|error| format!("encode protocol-v5 scenario terminal: {error}"))?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| "protocol-v5 scenario terminal is not an object".to_owned())?;
    object.insert(
        "canonical_payload_hex".to_owned(),
        Value::String(lower_hex(terminal.payload())),
    );
    object.insert(
        "terminal_digest".to_owned(),
        Value::String(terminal.digest().to_string()),
    );
    object.insert(
        "terminal_epoch_ms".to_owned(),
        Value::Number(terminal_epoch_ms.into()),
    );
    Ok(value)
}

fn lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn compare_client_server_identity() -> Result<Value, String> {
    let production_identity = CoreIdentity::production_v5();
    let invocation = V5InvocationRequest::new(
        InvocationId::new(),
        TaskId::new(),
        V5ToolIdentity::View,
        Map::new(),
        "workspace-a".to_owned(),
        7_000,
    )?;
    let client_key = ReceiptKey::new(
        invocation.invocation_id(),
        invocation.reserved_task_id(),
        RequestIdentity::new(
            production_identity.digest().clone(),
            invocation.tool(),
            normalized_arguments_hash(invocation.arguments()),
            request_scope_hash(invocation.workspace_hint())
                .map_err(|error| format!("derive client request scope: {error}"))?,
        ),
    );

    let mut frame = serde_json::to_vec(&V5ClientRequest::SubmitInvocation { invocation })
        .map_err(|error| format!("encode client identity probe: {error}"))?;
    frame.push(b'\n');
    let strict = decode_v5_request_frame(frame)
        .map_err(|error| format!("decode daemon identity probe: {error}"))?
        .into_strict_submit(&production_identity)
        .map_err(|error| format!("derive daemon receipt identity: {error}"))?;
    let (daemon_key, _) = strict.into_parts();

    let caller_claimed_key = ReceiptKey::new(
        client_key.invocation_id(),
        client_key.reserved_task_id(),
        RequestIdentity::new(
            CoreIdentityDigest::from_sha256([0x7f; 32]),
            client_key.tool(),
            client_key.normalized_arguments_hash().clone(),
            client_key.request_scope_hash().clone(),
        ),
    );

    let frozen_vector_key = ReceiptKey::new(
        "123e4567-e89b-42d3-a456-426614174000"
            .parse()
            .map_err(|error| format!("parse frozen invocation id: {error}"))?,
        "123e4567-e89b-42d3-b456-426614174001"
            .parse()
            .map_err(|error| format!("parse frozen reserved task id: {error}"))?,
        RequestIdentity::new(
            CoreIdentityDigest::from_sha256([0x00; 32]),
            V5ToolIdentity::View,
            NormalizedArgumentsHash::from_sha256([0x11; 32]),
            request_scope_hash("workspace-a")
                .map_err(|error| format!("derive frozen request scope: {error}"))?,
        ),
    );
    let frozen_task_link = TaskLinkIdentity::new(
        "0".repeat(64)
            .parse::<ReceiptKeyDigest>()
            .map_err(|error| format!("parse frozen receipt key digest: {error}"))?,
        "11111111-1111-4111-8111-111111111111"
            .parse()
            .map_err(|error| format!("parse frozen task id: {error}"))?,
        "22222222-2222-4222-8222-222222222222"
            .parse()
            .map_err(|error| format!("parse frozen task invocation id: {error}"))?,
        SafeIdentityHash::from_sha256([0xaa; 32]),
    );

    Ok(json!({
        "clientKey": receipt_key_observation(&client_key),
        "daemonKey": receipt_key_observation(&daemon_key),
        "frozenVectorKey": receipt_key_observation(&frozen_vector_key),
        "frozenTaskLinkVector": {
            "receiptKeyDigest": "0".repeat(64),
            "taskId": "11111111-1111-4111-8111-111111111111",
            "invocationId": "22222222-2222-4222-8222-222222222222",
            "workspaceIdentityHash": "a".repeat(64),
            "taskLinkDigest": task_link_digest(&frozen_task_link).to_string(),
        },
        "callerClaimedKeyDigest": receipt_key_digest(&caller_claimed_key).to_string(),
    }))
}

#[derive(Default)]
struct ScenarioReportBuilder {
    checkpoints: BTreeMap<String, Value>,
    responses: BTreeMap<String, Value>,
    task_reads: BTreeMap<String, Value>,
    identity: Option<Value>,
    terminal_publications: Vec<Value>,
    staged_terminal_preparations: Vec<Value>,
    actor_bindings: Vec<Value>,
    actor_authorizations: Vec<Value>,
    protocol: Vec<Value>,
    task_publication_capacity: Vec<Value>,
    task_store_capacity_invariant_violations: Vec<Value>,
    gate_events: Vec<Value>,
    operation_events: Vec<Value>,
    crash_cases: Vec<Value>,
    task_retirement_cases: Vec<Value>,
    load_runs: BTreeMap<String, Value>,
}
impl ScenarioReportBuilder {
    fn encode(self, events: Vec<V5ReceiptRuntimeEvent>) -> Result<String, String> {
        const COMPRESSION_THRESHOLD_BYTES: usize = 8 * 1_024 * 1_024;
        const INTERN_THRESHOLD_ENTRIES: usize = 1_000;
        let mut checkpoints = self.checkpoints;
        let mut checkpoint_artifacts = Map::new();
        for checkpoint in checkpoints.values_mut() {
            let Some(checkpoint) = checkpoint.as_object_mut() else {
                continue;
            };
            for field in [
                "tombstones",
                "tasks",
                "taskLinks",
                "invocationIndex",
                "reservedTaskIndex",
            ] {
                let Some(value) = checkpoint.get_mut(field) else {
                    continue;
                };
                if value.as_array().map_or(0, Vec::len) < INTERN_THRESHOLD_ENTRIES {
                    continue;
                }
                let encoded = serde_json::to_vec(value)
                    .map_err(|error| format!("encode protocol-v5 checkpoint artifact: {error}"))?;
                let artifact_id = lower_hex(&Sha256::digest(&encoded));
                checkpoint_artifacts
                    .entry(artifact_id.clone())
                    .or_insert_with(|| std::mem::take(value));
                *value = json!({ "$artifact": artifact_id });
            }
        }
        let mut payload = json!({
                "checkpoints": checkpoints,
                "responses": self.responses,
                "taskReads": self.task_reads,
                "events": events,
                "gateEvents": self.gate_events,
                "operationEvents": self.operation_events,
                "actorBindings": self.actor_bindings,
                "actorAuthorizations": self.actor_authorizations,
                "taskPublicationCapacity": self.task_publication_capacity,
                "taskStoreCapacityInvariantViolations": self.task_store_capacity_invariant_violations,
                "stagedTerminalPreparations": self.staged_terminal_preparations,
                "terminalPublications": self.terminal_publications,
                "protocol": self.protocol,
                "identity": self.identity,
                "crashCases": self.crash_cases,
                "taskRetirementCases": self.task_retirement_cases,
                "loadRuns": self.load_runs
        });
        if !checkpoint_artifacts.is_empty() {
            payload["checkpointArtifacts"] = Value::Object(checkpoint_artifacts);
        }
        let encoded_payload = serde_json::to_vec(&payload)
            .map_err(|error| format!("encode protocol-v5 receipt scenario payload: {error}"))?;
        if encoded_payload.len() <= COMPRESSION_THRESHOLD_BYTES {
            return serde_json::to_string(&json!({
                "kind": "observed",
                "payload": payload,
            }))
            .map_err(|error| format!("encode protocol-v5 receipt scenario report: {error}"));
        }
        let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
        encoder
            .write_all(&encoded_payload)
            .map_err(|error| format!("compress protocol-v5 receipt scenario report: {error}"))?;
        let compressed = encoder
            .finish()
            .map_err(|error| format!("finish protocol-v5 receipt scenario compression: {error}"))?;
        let encoded = encode_base64(&compressed);
        serde_json::to_string(&json!({
            "kind": "observed_gzip_base64",
            "payload": encoded,
        }))
        .map_err(|error| format!("encode compressed protocol-v5 scenario report: {error}"))
    }
}

fn encode_base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);
        encoded.push(ALPHABET[(first >> 2) as usize] as char);
        encoded.push(ALPHABET[(((first & 0x03) << 4) | (second >> 4)) as usize] as char);
        encoded.push(if chunk.len() > 1 {
            ALPHABET[(((second & 0x0f) << 2) | (third >> 6)) as usize] as char
        } else {
            '='
        });
        encoded.push(if chunk.len() > 2 {
            ALPHABET[(third & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    encoded
}

struct ScenarioEpochClock {
    epoch_ms: AtomicU64,
    monotonic_origin: Instant,
    monotonic_ms: AtomicU64,
    wall: bool,
}

impl ScenarioEpochClock {
    fn new(epoch_ms: u64, wall: bool) -> Self {
        Self {
            epoch_ms: AtomicU64::new(epoch_ms),
            monotonic_origin: Instant::now(),
            monotonic_ms: AtomicU64::new(0),
            wall,
        }
    }

    fn advance(&self, millis: u64) -> Result<(), String> {
        self.epoch_ms
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |epoch_ms| {
                epoch_ms.checked_add(millis)
            })
            .map(|_| ())
            .map_err(|_| "protocol-v5 receipt scenario epoch overflow".to_owned())
    }

    fn advance_monotonic(&self, millis: u64) -> Result<(), String> {
        self.monotonic_ms
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |monotonic_ms| {
                monotonic_ms.checked_add(millis)
            })
            .map(|_| ())
            .map_err(|_| "protocol-v5 receipt scenario monotonic clock overflow".to_owned())
    }

    fn now_monotonic_millis(&self) -> u64 {
        if self.wall {
            u64::try_from(self.monotonic_origin.elapsed().as_millis()).unwrap_or(u64::MAX)
        } else {
            self.monotonic_ms.load(Ordering::SeqCst)
        }
    }
}

impl EpochMillisClock for ScenarioEpochClock {
    fn now_epoch_millis(&self) -> u64 {
        self.epoch_ms
            .load(Ordering::SeqCst)
            .saturating_add(if self.wall {
                u64::try_from(self.monotonic_origin.elapsed().as_millis()).unwrap_or(u64::MAX)
            } else {
                0
            })
    }
}

impl Clock for ScenarioEpochClock {
    fn now(&self) -> Instant {
        if self.wall {
            Instant::now()
        } else {
            self.monotonic_origin
                .checked_add(Duration::from_millis(
                    self.monotonic_ms.load(Ordering::SeqCst),
                ))
                .unwrap_or(self.monotonic_origin)
        }
    }
}

mod control;
use control::*;

mod dispatch;
pub(crate) use dispatch::run_supported_receipt_scenario_for_test;

mod wire;
use wire::*;

#[cfg(test)]
mod tests;
