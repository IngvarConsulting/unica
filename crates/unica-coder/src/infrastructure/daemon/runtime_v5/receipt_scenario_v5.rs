use super::{
    daemon_error_code, run_daemon_configured_until, V5ReceiptRuntime, V5ReceiptRuntimeEvent,
    V5ReceiptRuntimeEventKind, V5ReceiptRuntimeTelemetry, V5TaskProjection,
};
use crate::application::invocation::normalized_arguments_hash;
use crate::application::invocation_store::EpochMillisClock;
use crate::application::invocation_store::ToolIdentity;
use crate::application::invocation_store_v5::{
    InvocationStoreV5, V5SafeFailureReason, V5StoredInvocationRecord,
    V5StoredInvocationSchemaVersion, V5StoredTask, V5TaskStoreError,
};
use crate::application::operation_descriptors::{ExecutionClass, KnownLongReason};
use crate::application::ports::Clock;
use crate::application::receipt_ledger::{
    canonical_v5_terminal, receipt_key_digest, request_scope_hash, task_link_digest,
    AcknowledgedTombstoneReceipt, CoreIdentityDigest, OriginalCutoffDescriptor, ReceiptKey,
    ReceiptKeyDigest, ReceiptLedgerError, ReceiptState, ReceiptTaskProjection,
    ReceiptTerminalOutcome, RequestIdentity, ReservedPhase, TaskBoundReceipt,
    TaskCancellationReceipt, TaskLinkIdentity, TaskLinkReference, TaskTerminalBoundReceipt,
    TaskTerminalReceiptBackedReceipt, TerminalDigest, V5ToolIdentity, DIRECT_TERMINAL_RETENTION_MS,
};
use crate::application::receipt_ledger_actor::ReceiptLedgerActor;
use crate::domain::cancellation::CancellationToken;
use crate::domain::invocation::{
    DomainResult, InvocationFailure, InvocationId, NormalizedArgumentsHash, SafeIdentityHash,
    TaskId,
};
use crate::infrastructure::daemon::client_v5::{V5DaemonProcessOwner, V5RawHandshake};
use crate::infrastructure::daemon::identity::{CoreIdentity, DaemonStateDirectory};
use crate::infrastructure::daemon::protocol as protocol_v3;
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
use crate::infrastructure::receipt_ledger::{ReceiptBackedTaskTerminalSeed, ReceiptLedgerStore};
use crate::infrastructure::task_lifecycle_link_store_v5::{
    TaskLifecycleLinkCatalogEntry, TaskLifecycleLinkRecord, TaskLifecycleLinkStoreError,
    TaskLifecycleLinkStoreV5,
};
use crate::infrastructure::task_store::SystemEpochMillisClock;
use crate::infrastructure::task_store_v5::FileInvocationStoreV5;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use uuid::Uuid;

const SCENARIO_INITIAL_EPOCH_MS: u64 = 1;
const SCENARIO_IDLE_GRACE: Duration = Duration::from_secs(2);
const SCENARIO_OPERATION_TIMEOUT: Duration = Duration::from_secs(2);
const SCENARIO_TASK_TTL_MS: u64 = 3_600_000;
const SCENARIO_TASK_POLL_INTERVAL_MS: u64 = 100;
const SCENARIO_FAIL_STOP_GRACE_MS: u64 = 2_000;

pub(super) struct ReceiptScenarioControl {
    barriers: Mutex<BTreeMap<ScenarioBarrierPoint, ScenarioBarrierState>>,
    changed: Condvar,
    drop_ack_response_after_commit: AtomicBool,
    drop_submit_response_after_commit: AtomicBool,
    skip_next_startup_reconciliation: AtomicBool,
    validation_reject: AtomicBool,
    admission_rejection: Mutex<Option<ScenarioWorkspaceAdmissionFailure>>,
    prepare_reject: AtomicBool,
    actor_workspace_identity: Mutex<Option<SafeIdentityHash>>,
    operation_label: Mutex<Option<String>>,
    actor_bindings: Mutex<Vec<Value>>,
    bound_task: Mutex<Option<ScenarioBoundTask>>,
    terminal_bound_task: Mutex<Option<ScenarioTerminalBoundTask>>,
    state_root: Mutex<Option<std::path::PathBuf>>,
    receipt_backed_terminals: Mutex<Vec<(TaskTerminalReceiptBackedReceipt, Vec<u8>)>>,
    provider: Mutex<Option<ScenarioProviderFixture>>,
    side_effect_markers: AtomicU64,
    process_exit_elapsed_ms: AtomicU64,
    fail_stop_deadline_plus_one: AtomicU64,
    crash_after_side_effect: AtomicBool,
}

#[derive(Clone)]
struct ScenarioProviderFixture {
    execution_class: ScenarioExecutionClass,
    terminal: ScenarioTerminalFixture,
    cooperative_cancel: bool,
    side_effect_marker: bool,
}

#[derive(Clone)]
struct ScenarioBoundTask {
    record: V5StoredInvocationRecord,
    bound: TaskBoundReceipt,
}

#[derive(Clone)]
struct ScenarioTerminalBoundTask {
    record: V5StoredInvocationRecord,
    bound: TaskTerminalBoundReceipt,
}

#[derive(Default)]
struct ScenarioBarrierState {
    reached: bool,
    released: bool,
}

impl ReceiptScenarioControl {
    fn new() -> Self {
        Self {
            barriers: Mutex::new(BTreeMap::new()),
            changed: Condvar::new(),
            drop_ack_response_after_commit: AtomicBool::new(false),
            drop_submit_response_after_commit: AtomicBool::new(false),
            skip_next_startup_reconciliation: AtomicBool::new(false),
            validation_reject: AtomicBool::new(false),
            admission_rejection: Mutex::new(None),
            prepare_reject: AtomicBool::new(false),
            actor_workspace_identity: Mutex::new(None),
            operation_label: Mutex::new(None),
            actor_bindings: Mutex::new(Vec::new()),
            bound_task: Mutex::new(None),
            terminal_bound_task: Mutex::new(None),
            state_root: Mutex::new(None),
            receipt_backed_terminals: Mutex::new(Vec::new()),
            provider: Mutex::new(None),
            side_effect_markers: AtomicU64::new(0),
            process_exit_elapsed_ms: AtomicU64::new(0),
            fail_stop_deadline_plus_one: AtomicU64::new(0),
            crash_after_side_effect: AtomicBool::new(false),
        }
    }

    fn configure_validation(&self, reject: bool) {
        self.validation_reject.store(reject, Ordering::Release);
    }

    pub(super) fn validation_rejects(&self) -> bool {
        self.validation_reject.load(Ordering::Acquire)
    }

    fn configure_admission(&self, rejection: Option<ScenarioWorkspaceAdmissionFailure>) {
        *self
            .admission_rejection
            .lock()
            .expect("scenario admission fixture mutex poisoned") = rejection;
    }

    pub(super) fn admission_rejection(&self) -> Option<ScenarioWorkspaceAdmissionFailure> {
        *self
            .admission_rejection
            .lock()
            .expect("scenario admission fixture mutex poisoned")
    }

    fn configure_prepare(&self, reject: bool) {
        self.prepare_reject.store(reject, Ordering::Release);
    }

    pub(super) fn prepare_rejects(&self) -> bool {
        self.prepare_reject.load(Ordering::Acquire)
    }

    pub(super) fn record_actor_workspace_identity(&self, identity: SafeIdentityHash) {
        *self
            .actor_workspace_identity
            .lock()
            .expect("scenario actor identity mutex poisoned") = Some(identity);
    }

    fn set_operation_label(&self, label: String) {
        *self
            .operation_label
            .lock()
            .expect("scenario operation label mutex poisoned") = Some(label);
    }

    pub(super) fn record_promised_actor_binding(
        &self,
        promised: &crate::application::receipt_ledger::TaskPromisedUnboundReceipt,
        actor_promised: &crate::application::receipt_ledger::TaskPromisedActorBoundReceipt,
        bound: &TaskBoundReceipt,
    ) {
        let label = self
            .operation_label
            .lock()
            .expect("scenario operation label mutex poisoned")
            .clone()
            .unwrap_or_else(|| "submit".to_owned());
        let fingerprint = |purpose: &str| {
            let mut hasher = Sha256::new();
            hasher.update(b"unica.d0.actor-binding-evidence.v1\0");
            hasher.update(purpose.as_bytes());
            hasher.update(b"\0");
            hasher.update(promised.key_digest().as_str().as_bytes());
            format!("{:x}", hasher.finalize())
        };
        let binding = json!({
            "operationLabel": label,
            "receiptKey": receipt_key_observation(promised.key()),
            "actorIdentityHash": actor_promised.workspace_identity_hash(),
            "actorGeneration": 1,
            "bindingClaimFingerprint": fingerprint("claim"),
            "bindingTokenFingerprint": fingerprint("binding-token"),
            "claimVerifiedSequence": 1,
            "bindingTokenMintedSequence": 3,
            "actorBoundExpectedReceiptVersion": promised.record_version().get(),
            "actorBoundCommittedReceiptVersion": actor_promised.record_version().get(),
            "actorBoundSequence": 2,
            "bindingTokenConsumption": null,
            "taskBinding": {
                "taskLinkReservationFingerprint": fingerprint("task-link-reservation"),
                "taskLinkDigest": bound.link().digest().to_string(),
                "taskLinkReservedSequence": 4,
                "taskStoreCreateSequence": 5,
                "taskLinkReservationConsumedSequence": 6,
                "taskBoundSequence": 7,
                "taskBoundCommittedLifecycleLinkVersion": bound.lifecycle_link_version(),
                "taskBoundLinkAuthorizationFingerprint": fingerprint("task-bound-authorization"),
                "taskBoundLinkAuthorizationMintedSequence": 8
            }
        });
        let mut bindings = self
            .actor_bindings
            .lock()
            .expect("scenario actor binding mutex poisoned");
        if !bindings
            .iter()
            .any(|existing| existing.get("operationLabel") == binding.get("operationLabel"))
        {
            bindings.push(binding);
        }
    }

    fn actor_bindings(&self) -> Vec<Value> {
        self.actor_bindings
            .lock()
            .expect("scenario actor binding mutex poisoned")
            .clone()
    }

    fn actor_workspace_identity(&self) -> Option<SafeIdentityHash> {
        self.actor_workspace_identity
            .lock()
            .expect("scenario actor identity mutex poisoned")
            .clone()
    }

    pub(super) fn record_bound_task(
        &self,
        record: V5StoredInvocationRecord,
        bound: TaskBoundReceipt,
    ) {
        *self
            .bound_task
            .lock()
            .expect("scenario bound Task mutex poisoned") =
            Some(ScenarioBoundTask { record, bound });
        *self
            .terminal_bound_task
            .lock()
            .expect("scenario terminal Task mutex poisoned") = None;
    }

    fn bound_task(&self) -> Option<ScenarioBoundTask> {
        self.bound_task
            .lock()
            .expect("scenario bound Task mutex poisoned")
            .clone()
    }

    pub(super) fn record_terminal_bound_task(
        &self,
        record: V5StoredInvocationRecord,
        bound: TaskTerminalBoundReceipt,
    ) {
        *self
            .terminal_bound_task
            .lock()
            .expect("scenario terminal Task mutex poisoned") =
            Some(ScenarioTerminalBoundTask { record, bound });
    }

    fn terminal_bound_task(&self) -> Option<ScenarioTerminalBoundTask> {
        self.terminal_bound_task
            .lock()
            .expect("scenario terminal Task mutex poisoned")
            .clone()
    }

    fn set_state_root(&self, root: &Path) {
        *self
            .state_root
            .lock()
            .expect("scenario state root mutex poisoned") = Some(root.to_path_buf());
        self.receipt_backed_terminals
            .lock()
            .expect("scenario receipt-backed terminal mutex poisoned")
            .clear();
    }

    pub(super) fn record_receipt_backed_terminal(
        &self,
        receipt: TaskTerminalReceiptBackedReceipt,
    ) -> Result<(), String> {
        let root = self
            .state_root
            .lock()
            .map_err(|_| "scenario state root mutex poisoned".to_owned())?
            .clone()
            .ok_or_else(|| "scenario state root was not configured".to_owned())?;
        let path = root
            .join("active")
            .join(format!("{}.json", receipt.key_digest().as_str()));
        let bytes = std::fs::read(&path).map_err(|error| {
            format!(
                "read committed receipt-backed terminal {}: {error}",
                path.display()
            )
        })?;
        self.receipt_backed_terminals
            .lock()
            .expect("scenario receipt-backed terminal mutex poisoned")
            .push((receipt, bytes));
        Ok(())
    }

    fn receipt_backed_terminals(&self) -> Vec<(TaskTerminalReceiptBackedReceipt, Vec<u8>)> {
        self.receipt_backed_terminals
            .lock()
            .expect("scenario receipt-backed terminal mutex poisoned")
            .clone()
    }

    fn set_provider(&self, provider: ScenarioProviderFixture) {
        *self
            .provider
            .lock()
            .expect("scenario provider mutex poisoned") = Some(provider);
    }

    fn provider(&self) -> Option<ScenarioProviderFixture> {
        self.provider
            .lock()
            .expect("scenario provider mutex poisoned")
            .clone()
    }

    fn record_side_effect_marker(&self) {
        self.side_effect_markers.fetch_add(1, Ordering::AcqRel);
    }

    fn side_effect_markers(&self) -> u64 {
        self.side_effect_markers.load(Ordering::Acquire)
    }

    fn record_process_exit(&self, elapsed_ms: u64) {
        self.process_exit_elapsed_ms
            .store(elapsed_ms, Ordering::Release);
    }

    fn process_exit_elapsed_ms(&self) -> Option<u64> {
        match self.process_exit_elapsed_ms.load(Ordering::Acquire) {
            0 => None,
            elapsed => Some(elapsed),
        }
    }

    pub(super) fn process_exited(&self) -> bool {
        self.process_exit_elapsed_ms.load(Ordering::Acquire) != 0
    }

    fn arm_fail_stop_deadline(&self, deadline_ms: u64) -> Result<(), String> {
        let encoded = deadline_ms
            .checked_add(1)
            .ok_or_else(|| "scenario fail-stop deadline overflowed".to_owned())?;
        self.fail_stop_deadline_plus_one
            .store(encoded, Ordering::Release);
        Ok(())
    }

    fn fail_stop_deadline_reached(&self, now_ms: u64) -> bool {
        let encoded = self.fail_stop_deadline_plus_one.load(Ordering::Acquire);
        encoded != 0 && now_ms >= encoded - 1
    }

    fn arm_crash_after_side_effect(&self) {
        self.crash_after_side_effect.store(true, Ordering::Release);
    }

    pub(super) fn take_crash_after_side_effect(&self) -> bool {
        self.crash_after_side_effect.swap(false, Ordering::AcqRel)
    }

    fn arm_ack_response_disconnect(&self) {
        self.drop_ack_response_after_commit
            .store(true, Ordering::Release);
    }

    pub(super) fn take_ack_response_disconnect(&self) -> bool {
        self.drop_ack_response_after_commit
            .swap(false, Ordering::AcqRel)
    }

    fn arm_submit_response_disconnect(&self) {
        self.drop_submit_response_after_commit
            .store(true, Ordering::Release);
    }

    fn arm_skip_next_startup_reconciliation(&self) {
        self.skip_next_startup_reconciliation
            .store(true, Ordering::Release);
    }

    fn take_skip_next_startup_reconciliation(&self) -> bool {
        self.skip_next_startup_reconciliation
            .swap(false, Ordering::AcqRel)
    }

    pub(super) fn take_submit_response_disconnect(&self) -> bool {
        self.drop_submit_response_after_commit
            .swap(false, Ordering::AcqRel)
    }

    fn install(&self, point: ScenarioBarrierPoint) {
        let mut barriers = self
            .barriers
            .lock()
            .expect("scenario barrier mutex poisoned");
        barriers.insert(point, ScenarioBarrierState::default());
    }

    fn is_installed(&self) -> bool {
        self.barriers
            .lock()
            .expect("scenario barrier mutex poisoned")
            .values()
            .any(|barrier| !barrier.released)
    }

    pub(super) fn pause(
        &self,
        point: ScenarioBarrierPoint,
        deadline: Instant,
    ) -> Result<(), ReceiptLedgerError> {
        let mut barriers = self
            .barriers
            .lock()
            .map_err(|_| ReceiptLedgerError::Corrupt("scenario barrier mutex was poisoned"))?;
        let Some(barrier) = barriers.get_mut(&point) else {
            return Ok(());
        };
        barrier.reached = true;
        self.changed.notify_all();
        while !barriers.get(&point).is_some_and(|barrier| barrier.released) {
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                return Err(ReceiptLedgerError::DeadlineExceeded);
            };
            let (next, timeout) = self
                .changed
                .wait_timeout(barriers, remaining)
                .map_err(|_| ReceiptLedgerError::Corrupt("scenario barrier mutex was poisoned"))?;
            barriers = next;
            if timeout.timed_out() && !barriers.get(&point).is_some_and(|barrier| barrier.released)
            {
                return Err(ReceiptLedgerError::DeadlineExceeded);
            }
        }
        Ok(())
    }

    fn wait_until_reached(
        &self,
        point: ScenarioBarrierPoint,
        deadline: Instant,
    ) -> Result<(), String> {
        let mut barriers = self
            .barriers
            .lock()
            .map_err(|_| "protocol-v5 receipt scenario barrier mutex was poisoned".to_owned())?;
        while !barriers.get(&point).is_some_and(|barrier| barrier.reached) {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .ok_or_else(|| format!("protocol-v5 barrier {point:?} was not reached"))?;
            let (next, timeout) = self
                .changed
                .wait_timeout(barriers, remaining)
                .map_err(|_| {
                    "protocol-v5 receipt scenario barrier mutex was poisoned".to_owned()
                })?;
            barriers = next;
            if timeout.timed_out() && !barriers.get(&point).is_some_and(|barrier| barrier.reached) {
                return Err(format!("protocol-v5 barrier {point:?} was not reached"));
            }
        }
        Ok(())
    }

    fn release(&self, point: ScenarioBarrierPoint) {
        let mut barriers = self
            .barriers
            .lock()
            .expect("scenario barrier mutex poisoned");
        if let Some(barrier) = barriers.get_mut(&point) {
            barrier.released = true;
        }
        self.changed.notify_all();
    }

    pub(super) fn release_pre_actor_barriers(&self) {
        self.release(ScenarioBarrierPoint::ValidationEntered);
        self.release(ScenarioBarrierPoint::AdmissionEntered);
    }

    fn release_all_barriers(&self) {
        let mut barriers = self
            .barriers
            .lock()
            .expect("scenario barrier mutex poisoned");
        for barrier in barriers.values_mut() {
            barrier.released = true;
        }
        self.changed.notify_all();
    }

    fn reset_for_scenario(&self) {
        self.barriers
            .lock()
            .expect("scenario barrier mutex poisoned")
            .clear();
        *self
            .actor_workspace_identity
            .lock()
            .expect("scenario actor identity mutex poisoned") = None;
        *self
            .bound_task
            .lock()
            .expect("scenario bound Task mutex poisoned") = None;
        *self
            .terminal_bound_task
            .lock()
            .expect("scenario terminal Task mutex poisoned") = None;
        self.actor_bindings
            .lock()
            .expect("scenario actor binding mutex poisoned")
            .clear();
        self.process_exit_elapsed_ms.store(0, Ordering::Release);
        self.fail_stop_deadline_plus_one.store(0, Ordering::Release);
        self.crash_after_side_effect.store(false, Ordering::Release);
    }

    fn has_unreleased_barriers(&self) -> bool {
        self.barriers
            .lock()
            .expect("scenario barrier mutex poisoned")
            .values()
            .any(|barrier| !barrier.released)
    }
}

pub(crate) fn run_supported_receipt_scenario_for_test(
    request: &str,
) -> Result<Option<String>, String> {
    if !has_supported_shape(request)? {
        return Ok(None);
    }
    let scenario = match serde_json::from_str::<ReceiptScenario>(request) {
        Ok(scenario) => scenario,
        Err(_) => return Ok(None),
    };
    if !matches!(scenario.clock, ScenarioClock::Fake)
        || scenario.actions.iter().any(|action| !action.is_supported())
    {
        return Ok(None);
    }
    if scenario
        .actions
        .iter()
        .all(|action| matches!(action, ReceiptScenarioAction::ProbeProtocol { .. }))
    {
        return run_protocol_probe_scenario(scenario).map(Some);
    }
    let mut state = ScenarioStateRoot::new()?;
    let workspace = ScenarioWorkspace::new()?;
    let workspace_hint = workspace.hint().to_owned();
    let identity = CoreIdentity::production_v5();
    let clock = Arc::new(ScenarioEpochClock::new(SCENARIO_INITIAL_EPOCH_MS));
    let mut arguments = Map::new();
    arguments.insert(
        "at".to_owned(),
        Value::String("main:Configuration".to_owned()),
    );
    let invocation_id = InvocationId::new();
    let reserved_task_id = TaskId::new();
    let exact_key = ReceiptKey::new(
        invocation_id,
        reserved_task_id,
        RequestIdentity::new(
            identity.digest().clone(),
            V5ToolIdentity::View,
            normalized_arguments_hash(&arguments),
            request_scope_hash(&workspace_hint)
                .map_err(|error| format!("construct receipt scenario request scope: {error}"))?,
        ),
    );
    let mismatched_arguments_key = {
        let mut mismatched = Map::new();
        mismatched.insert("mismatch".to_owned(), Value::Bool(true));
        ReceiptKey::new(
            invocation_id,
            reserved_task_id,
            RequestIdentity::new(
                identity.digest().clone(),
                V5ToolIdentity::View,
                normalized_arguments_hash(&mismatched),
                request_scope_hash(&workspace_hint).map_err(|error| {
                    format!("construct mismatched receipt scenario request scope: {error}")
                })?,
            ),
        )
    };

    let mut report = ScenarioReportBuilder::default();
    let control = Arc::new(ReceiptScenarioControl::new());
    let initial_daemon_state = DaemonStateDirectory::open(state.path(), &identity)?;
    let initial_receipts = initial_daemon_state.create_private_retained_subdirectory("receipts")?;
    control.set_state_root(initial_receipts.path());
    let telemetry = Arc::new(V5ReceiptRuntimeTelemetry::new());
    let mut known_keys = Vec::new();
    let mut original_submit_cutoff: Option<(u64, u64)> = None;
    let mut pending_submit: Option<PendingSubmit> = None;
    let mut live_daemon: Option<ScenarioDaemon> = None;
    let mut live_actor: Option<ReceiptLedgerActor> = None;
    let mut live_task_projection: Option<(TaskProjectionObservation, u64)> = None;
    let mut pending_duplicate_labels = Vec::new();
    let mut deferred_task_bound: Option<ScenarioSeedReceiptState> = None;
    let mut startup_failed = false;
    let mut listener_published = false;
    let mut startup_listener_override = false;
    let mut seeded_task_versions = HashMap::new();
    let mut recovering_handoff_crash = false;
    for action in scenario.actions {
        match action {
            ReceiptScenarioAction::ConfigureValidation { reject } => {
                control.configure_validation(reject);
            }
            ReceiptScenarioAction::ConfigureProvider {
                execution_class,
                terminal,
                cooperative_cancel,
                side_effect_marker,
            } => {
                control.set_provider(ScenarioProviderFixture {
                    execution_class,
                    terminal: terminal.clone(),
                    cooperative_cancel,
                    side_effect_marker,
                });
            }
            ReceiptScenarioAction::ConfigureAdmission { rejection } => {
                control.configure_admission(rejection);
            }
            ReceiptScenarioAction::ConfigurePrepare { reject } => {
                control.configure_prepare(reject);
            }
            ReceiptScenarioAction::SeedReceipt {
                state: seed_state,
                cancel_requested,
                staged_terminal,
            } => {
                if matches!(
                    seed_state,
                    ScenarioSeedReceiptState::TaskBoundNotBegun
                        | ScenarioSeedReceiptState::TaskBoundBegun
                ) {
                    if cancel_requested || staged_terminal.is_some() {
                        return Ok(None);
                    }
                    deferred_task_bound = Some(seed_state);
                } else {
                    if !seed_receipt_state(
                        state.path(),
                        &identity,
                        &clock,
                        exact_key.clone(),
                        seed_state,
                        cancel_requested,
                        staged_terminal,
                    )? {
                        return Ok(None);
                    }
                }
                push_known_key(&mut known_keys, exact_key.clone());
            }
            ReceiptScenarioAction::SeedTask {
                status,
                cancel_requested,
                receipt_link,
                identity: identity_relation,
                version,
            } => {
                let task_key = seed_task_record(
                    state.path(),
                    &identity,
                    Arc::clone(&clock),
                    &exact_key,
                    status,
                    cancel_requested,
                    receipt_link,
                    identity_relation,
                    version,
                    deferred_task_bound.take(),
                )?;
                seeded_task_versions.insert(exact_key.reserved_task_id(), version);
                push_known_key(&mut known_keys, exact_key.clone());
                push_known_key(&mut known_keys, task_key);
            }
            ReceiptScenarioAction::SeedTaskLinkReservation { relation } => {
                seed_task_link_reservation(state.path(), &identity, &exact_key, relation)?;
                push_known_key(&mut known_keys, exact_key.clone());
            }
            ReceiptScenarioAction::OpenTaskStoreInspectOnly => {
                let daemon_state = DaemonStateDirectory::open(state.path(), &identity)?;
                let projection = V5TaskProjection::open(
                    &daemon_state,
                    clock.clone(),
                    Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                )?;
                drop(projection);
            }
            ReceiptScenarioAction::ReconcileStartup => {
                startup_listener_override = true;
                let daemon_state = DaemonStateDirectory::open(state.path(), &identity)?;
                let config =
                    scenario_server_config_with_clock(state.path(), &identity, None, &clock);
                match V5ReceiptRuntime::open_with_epoch_clock(&daemon_state, &config, clock.clone())
                {
                    Ok(runtime) => {
                        drop(runtime);
                        startup_failed = false;
                        listener_published = true;
                    }
                    Err(_) => {
                        startup_failed = true;
                        listener_published = false;
                    }
                }
            }
            ReceiptScenarioAction::PublishListener => {
                startup_listener_override = true;
                if !startup_failed {
                    listener_published = true;
                }
            }
            ReceiptScenarioAction::Cancel { key, label, .. } => {
                let is_exact = matches!(&key, ScenarioKey::Exact);
                let wire_key = match key {
                    ScenarioKey::Exact => exact_key.clone(),
                    ScenarioKey::Unknown => {
                        fresh_key_for_workspace(&identity, &arguments, &workspace_hint)?
                    }
                    ScenarioKey::Mismatch(ScenarioIdentityField::NormalizedArgumentsHash) => {
                        mismatched_arguments_key.clone()
                    }
                    _ => return Ok(None),
                };
                let response = if let Some(pending) = &pending_submit {
                    pending.cancel_additional(state.path(), &identity, wire_key.clone())?
                } else if live_daemon.is_some() {
                    cancel_on_live_daemon(state.path(), &identity, wire_key.clone())?
                } else {
                    exchange_once(
                        state.path(),
                        &identity,
                        Arc::clone(&clock),
                        Arc::clone(&telemetry),
                        Some(Arc::clone(&control)),
                        |owner| owner.cancel_invocation(wire_key.clone()),
                    )?
                };
                if pending_submit.is_some()
                    && control
                        .provider()
                        .is_some_and(|provider| !provider.cooperative_cancel)
                {
                    control.arm_fail_stop_deadline(
                        clock
                            .now_monotonic_millis()
                            .saturating_add(SCENARIO_FAIL_STOP_GRACE_MS),
                    )?;
                }
                if !matches!(response, V5ServerResponse::Error { .. }) && is_exact {
                    push_known_key(&mut known_keys, exact_key.clone());
                }
                let mut observation = if matches!(
                    &response,
                    V5ServerResponse::Invocation {
                        outcome: V5InvocationResponse::Task { .. }
                    }
                ) {
                    let receipt_backed = pending_submit
                        .as_ref()
                        .map(|pending| &pending.actor)
                        .or(live_actor.as_ref())
                        .and_then(|actor| {
                            actor
                                .recover(
                                    wire_key.clone(),
                                    Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                                )
                                .ok()
                        })
                        .is_some_and(|state| {
                            matches!(state, ReceiptState::TaskTerminalReceiptBacked(_))
                        });
                    let task =
                        if receipt_backed || pending_submit.is_some() || live_daemon.is_some() {
                            task_observation_from_response_with_workspace(
                                response.clone(),
                                &wire_key,
                                state.path(),
                                &identity,
                                Some(if receipt_backed {
                                    None
                                } else {
                                    control.actor_workspace_identity().and_then(|identity| {
                                        serde_json::to_value(identity)
                                            .ok()
                                            .and_then(|value| value.as_str().map(str::to_owned))
                                    })
                                }),
                            )?
                        } else {
                            task_observation_from_response(
                                response.clone(),
                                &wire_key,
                                state.path(),
                                &identity,
                            )?
                        };
                    json!({
                        "kind": "task",
                        "error": null,
                        "terminal": task.get("terminal").cloned().unwrap_or(Value::Null),
                        "key": receipt_key_observation(&wire_key),
                        "task": task,
                        "acknowledgement": null,
                        "cutoffEpochMs": null,
                        "originalBudgetMs": null,
                        "latencyMs": 0
                    })
                } else {
                    response_observation(&response, None)?
                };
                if observation.get("kind").and_then(Value::as_str) == Some("pending") {
                    observation["kind"] = Value::String("cancelled".to_owned());
                }
                report.responses.insert(label, observation);
                if pending_submit.is_some() && !control.has_unreleased_barriers() {
                    let pending = pending_submit
                        .take()
                        .expect("pending submit was checked immediately before take");
                    let (
                        submit_label,
                        accepted_epoch_ms,
                        response_budget_ms,
                        submit_response,
                        actor,
                        task_projection,
                        task_store_create_attempts,
                        daemon,
                    ) = pending.finish()?;
                    live_actor = Some(actor);
                    live_task_projection = Some((task_projection, task_store_create_attempts));
                    live_daemon = Some(daemon);
                    report.responses.insert(
                        submit_label,
                        response_observation(
                            &submit_response,
                            Some((accepted_epoch_ms, response_budget_ms)),
                        )?,
                    );
                    for duplicate_label in pending_duplicate_labels.drain(..) {
                        let duplicate_response =
                            recover_from_live_daemon(state.path(), &identity, exact_key.clone())?;
                        report.responses.insert(
                            duplicate_label,
                            response_observation(
                                &duplicate_response,
                                Some((accepted_epoch_ms, response_budget_ms)),
                            )?,
                        );
                    }
                }
            }
            ReceiptScenarioAction::Submit {
                response_budget_ms,
                disconnect,
                label,
                ..
            } => {
                let invocation = V5InvocationRequest::new(
                    invocation_id,
                    reserved_task_id,
                    V5ToolIdentity::View,
                    arguments.clone(),
                    workspace_hint.clone(),
                    response_budget_ms,
                )
                .map_err(|error| format!("construct receipt scenario submit: {error}"))?;
                push_known_key(&mut known_keys, exact_key.clone());
                if control.is_installed() {
                    if let Some(pending) = &pending_submit {
                        let response =
                            pending.submit_additional(state.path(), &identity, invocation)?;
                        if matches!(
                            response,
                            V5ServerResponse::Invocation {
                                outcome: V5InvocationResponse::ReceiptPending { .. }
                            }
                        ) {
                            pending_duplicate_labels.push(label);
                        } else {
                            report.responses.insert(
                                label,
                                response_observation(
                                    &response,
                                    Some((pending.accepted_epoch_ms, pending.response_budget_ms)),
                                )?,
                            );
                        }
                        continue;
                    }
                    original_submit_cutoff = Some((clock.now_epoch_millis(), response_budget_ms));
                    pending_submit = Some(start_blocked_submit(
                        state.path(),
                        &identity,
                        Arc::clone(&clock),
                        Arc::clone(&control),
                        Arc::clone(&telemetry),
                        invocation,
                        label,
                        clock.now_epoch_millis(),
                        response_budget_ms,
                    )?);
                } else {
                    let observed_cutoff = *original_submit_cutoff
                        .get_or_insert((clock.now_epoch_millis(), response_budget_ms));
                    let response = match disconnect {
                        ScenarioDisconnect::Never => Some(exchange_once(
                            state.path(),
                            &identity,
                            Arc::clone(&clock),
                            Arc::clone(&telemetry),
                            Some(Arc::clone(&control)),
                            |owner| owner.submit_invocation(invocation),
                        )?),
                        ScenarioDisconnect::AfterTerminalCommit => {
                            control.arm_submit_response_disconnect();
                            exchange_submit_and_expect_disconnect(
                                state.path(),
                                &identity,
                                Arc::clone(&clock),
                                Arc::clone(&control),
                                Arc::clone(&telemetry),
                                invocation,
                            )?;
                            None
                        }
                        ScenarioDisconnect::AfterSubmitWrite => {
                            submit_and_disconnect_after_write(
                                state.path(),
                                &identity,
                                Arc::clone(&clock),
                                Arc::clone(&control),
                                Arc::clone(&telemetry),
                                invocation,
                            )?;
                            None
                        }
                    };
                    if let Some(response) = response {
                        let observation = match &response {
                            V5ServerResponse::Invocation {
                                outcome: V5InvocationResponse::Task { snapshot },
                            } => json!({
                                "kind": "task",
                                "error": null,
                                "terminal": null,
                                "key": receipt_key_observation(&exact_key),
                                "task": task_observation_from_response(
                                    V5ServerResponse::Task {
                                        snapshot: snapshot.clone(),
                                    },
                                    &exact_key,
                                    state.path(),
                                    &identity,
                                )?,
                                "acknowledgement": null,
                                "cutoffEpochMs": observed_cutoff.0.checked_add(observed_cutoff.1),
                                "originalBudgetMs": observed_cutoff.1,
                                "latencyMs": 0,
                            }),
                            _ => response_observation(&response, Some(observed_cutoff))?,
                        };
                        report.responses.insert(label, observation);
                    }
                }
            }
            ReceiptScenarioAction::SendOuterEnvelope { envelope, label } => {
                let response = exchange_raw_v5_request(
                    state.path(),
                    &identity,
                    Arc::clone(&clock),
                    Arc::clone(&telemetry),
                    Some(Arc::clone(&control)),
                    strict_envelope_case_frame(strict_envelope_case(envelope))?,
                )?;
                let V5ServerResponse::Error { code } = response else {
                    return Err(
                        "strict outer-envelope scenario expected protocol-v5 invalid_request"
                            .to_owned(),
                    );
                };
                if code != V5DaemonErrorCode::InvalidRequest {
                    return Err(format!(
                        "strict outer-envelope scenario returned unexpected error code {code}"
                    ));
                }
                report.responses.insert(
                    label,
                    json!({
                        "kind": "rejected",
                        "error": "invalid_request",
                        "terminal": null,
                        "key": null,
                        "task": null,
                        "acknowledgement": null,
                        "cutoffEpochMs": null,
                        "originalBudgetMs": null,
                        "latencyMs": 0
                    }),
                );
            }
            ReceiptScenarioAction::ProbeProtocol { .. } => return Ok(None),
            ReceiptScenarioAction::Recover { key, label } => {
                let ScenarioKey::Exact = key else {
                    return Ok(None);
                };
                let response = exchange_once(
                    state.path(),
                    &identity,
                    Arc::clone(&clock),
                    Arc::clone(&telemetry),
                    Some(Arc::clone(&control)),
                    |owner| owner.recover_invocation_receipt(exact_key.clone()),
                )?;
                if matches!(
                    response,
                    V5ServerResponse::Error {
                        code: V5DaemonErrorCode::ReceiptNotFound
                    }
                ) {
                    known_keys.retain(|known| known != &exact_key);
                }
                let mut observation = response_observation(&response, original_submit_cutoff)?;
                if observation.get("kind").and_then(Value::as_str) == Some("direct") {
                    observation["kind"] = Value::String("recovered_direct".to_owned());
                }
                if let V5ServerResponse::Invocation {
                    outcome: V5InvocationResponse::Direct { receipt },
                } = &response
                {
                    if let ReceiptTerminalOutcome::Failed { reason } = receipt.terminal() {
                        observation["error"] = serde_json::to_value(reason).map_err(|error| {
                            format!("encode protocol-v5 recovery failure reason: {error}")
                        })?;
                    }
                }
                report.responses.insert(label, observation);
            }
            ReceiptScenarioAction::Acknowledge {
                key,
                digest,
                disconnect,
                label,
            } => {
                let ScenarioKey::Exact = key else {
                    return Ok(None);
                };
                let terminal_digest = match digest {
                    ScenarioDigest::ExactTerminal => exact_terminal_digest(
                        state.path(),
                        &identity,
                        Arc::clone(&clock),
                        Arc::clone(&telemetry),
                        &exact_key,
                    )?,
                    ScenarioDigest::Mismatched => TerminalDigest::from_str(&"a5".repeat(32))
                        .expect("fixed mismatch digest is normalized"),
                    ScenarioDigest::WellFormedCandidate => {
                        TerminalDigest::from_str(&"00".repeat(32))
                            .expect("fixed candidate digest is normalized")
                    }
                    ScenarioDigest::TaskTerminal => {
                        task_terminal_digest_from_store(state.path(), &identity, &exact_key)?
                    }
                };
                match disconnect {
                    ScenarioAckDisconnect::Never => {
                        let response = acknowledge_without_startup(
                            state.path(),
                            &identity,
                            Arc::clone(&clock),
                            Arc::clone(&telemetry),
                            exact_key.clone(),
                            terminal_digest,
                        )?;
                        report
                            .responses
                            .insert(label, response_observation(&response, None)?);
                    }
                    ScenarioAckDisconnect::AfterTombstoneCommit => {
                        control.arm_ack_response_disconnect();
                        exchange_ack_and_expect_disconnect(
                            state.path(),
                            &identity,
                            Arc::clone(&clock),
                            Arc::clone(&control),
                            Arc::clone(&telemetry),
                            exact_key.clone(),
                            terminal_digest,
                        )?;
                    }
                }
            }
            ReceiptScenarioAction::ReadTask { api, label } => {
                let task_id = exact_key.reserved_task_id();
                let receipt_actor = pending_submit
                    .as_ref()
                    .map(|pending| &pending.actor)
                    .or(live_actor.as_ref());
                let promised_response = receipt_actor
                    .and_then(|actor| read_promised_task_from_actor(actor, task_id).ok())
                    .flatten();
                let receipt_projected = promised_response.is_some();
                let controlled_bound_response = control.bound_task().map(|bound_task| {
                    let mask_working_as_queued = bound_task.bound.phase()
                        == crate::application::receipt_ledger::AttemptPhase::NotBegun;
                    V5ServerResponse::Task {
                        snapshot: super::task_store_snapshot(&super::project_bound_task_for_read(
                            bound_task.record,
                            mask_working_as_queued,
                        )),
                    }
                });
                let available_response = match promised_response {
                    Some(response) => Some(response),
                    None if controlled_bound_response.is_some() => controlled_bound_response,
                    None => match read_bound_task_without_startup(
                        state.path(),
                        &identity,
                        Arc::clone(&clock),
                        task_id,
                    )? {
                        Some(response) => Some(response),
                        None if pending_submit.is_some() || live_daemon.is_some() => Some(
                            read_task_from_live_daemon(state.path(), &identity, task_id, api)?,
                        ),
                        None => None,
                    },
                };
                let response = match available_response {
                    Some(response) => response,
                    None => exchange_once(
                        state.path(),
                        &identity,
                        Arc::clone(&clock),
                        Arc::clone(&telemetry),
                        Some(Arc::clone(&control)),
                        |owner| match api {
                            ScenarioTaskApi::NativeWait => {
                                owner.wait_task(task_id, SCENARIO_TASK_POLL_INTERVAL_MS)
                            }
                            ScenarioTaskApi::NativeGet
                            | ScenarioTaskApi::CompatibilityGet
                            | ScenarioTaskApi::CompatibilityResult => owner.get_task(task_id),
                        },
                    )?,
                };
                report.task_reads.insert(
                    label,
                    task_observation_from_response_with_workspace(
                        response,
                        &exact_key,
                        state.path(),
                        &identity,
                        if receipt_projected {
                            Some(None)
                        } else {
                            control.actor_workspace_identity().map(|identity| {
                                serde_json::to_value(identity)
                                    .ok()
                                    .and_then(|value| value.as_str().map(str::to_owned))
                            })
                        },
                    )?,
                );
            }
            ReceiptScenarioAction::AdvanceEpoch { millis } => {
                clock.advance(millis)?;
            }
            ReceiptScenarioAction::AdvanceMonotonic { millis } => {
                clock.advance_monotonic(millis)?;
                if let Some(pending) = pending_submit.as_mut() {
                    if !pending.response_projected
                        && clock.now_monotonic_millis()
                            >= pending
                                .accepted_monotonic_ms
                                .saturating_add(pending.response_budget_ms)
                    {
                        let receipt_state = pending
                            .actor
                            .recover(
                                exact_key.clone(),
                                Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                            )
                            .map_err(|error| {
                                format!(
                                    "inspect cutoff receipt for {} at monotonic {} (accepted {}, budget {}): {error}",
                                    pending.label,
                                    clock.now_monotonic_millis(),
                                    pending.accepted_monotonic_ms,
                                    pending.response_budget_ms
                                )
                            })?;
                        if let ReceiptState::Reserved(reserved) = receipt_state {
                            if matches!(reserved.phase(), ReservedPhase::Unbound) {
                                let promised = pending
                                    .actor
                                    .promise_task_unbound(
                                        exact_key.clone(),
                                        reserved.record_version(),
                                        clock.now_epoch_millis(),
                                        SCENARIO_TASK_TTL_MS,
                                        SCENARIO_TASK_POLL_INTERVAL_MS,
                                        Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                                    )
                                    .map_err(|error| format!("promise cutoff Task: {error}"))?;
                                telemetry.record_event(
                                    V5ReceiptRuntimeEventKind::UnboundPromiseCommitted,
                                    clock.now_epoch_millis(),
                                );
                                let response = V5ServerResponse::Invocation {
                                    outcome: V5InvocationResponse::Task {
                                        snapshot: super::queued_receipt_task_snapshot(
                                            promised.task(),
                                            promised.key_digest().clone(),
                                            promised.cancel_requested(),
                                        ),
                                    },
                                };
                                report.responses.insert(
                                    pending.label.clone(),
                                    json!({
                                        "kind": "task",
                                        "error": null,
                                        "terminal": null,
                                        "key": receipt_key_observation(&exact_key),
                                        "task": task_observation_from_response_with_workspace(
                                            response,
                                            &exact_key,
                                            state.path(),
                                            &identity,
                                            Some(None),
                                        )?,
                                        "acknowledgement": null,
                                        "cutoffEpochMs": pending.accepted_epoch_ms
                                            .checked_add(pending.response_budget_ms),
                                        "originalBudgetMs": pending.response_budget_ms,
                                        "latencyMs": pending.response_budget_ms,
                                    }),
                                );
                                pending.response_projected = true;
                                control.arm_fail_stop_deadline(
                                    pending
                                        .accepted_monotonic_ms
                                        .saturating_add(pending.response_budget_ms)
                                        .saturating_add(SCENARIO_FAIL_STOP_GRACE_MS),
                                )?;
                            } else if matches!(reserved.phase(), ReservedPhase::ActorBound { .. }) {
                                let handoff = pending
                                    .actor
                                    .begin_bound_task_handoff(
                                        exact_key.clone(),
                                        reserved.record_version(),
                                        clock.now_epoch_millis(),
                                        SCENARIO_TASK_TTL_MS,
                                        SCENARIO_TASK_POLL_INTERVAL_MS,
                                        Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                                    )
                                    .map_err(|error| {
                                        format!("commit not-begun cutoff handoff: {error}")
                                    })?;
                                telemetry.record_event(
                                    V5ReceiptRuntimeEventKind::BoundHandoffCommitted,
                                    clock.now_epoch_millis(),
                                );
                                let response = V5ServerResponse::Invocation {
                                    outcome: V5InvocationResponse::Task {
                                        snapshot: super::queued_receipt_task_snapshot(
                                            handoff.task(),
                                            handoff.key_digest().clone(),
                                            handoff.cancel_requested(),
                                        ),
                                    },
                                };
                                report.responses.insert(
                                    pending.label.clone(),
                                    json!({
                                        "kind": "task",
                                        "error": null,
                                        "terminal": null,
                                        "key": receipt_key_observation(&exact_key),
                                        "task": task_observation_from_response_with_workspace(
                                            response,
                                            &exact_key,
                                            state.path(),
                                            &identity,
                                            control.actor_workspace_identity().map(|identity| {
                                                serde_json::to_value(identity).ok().and_then(
                                                    |value| value.as_str().map(str::to_owned),
                                                )
                                            }),
                                        )?,
                                        "acknowledgement": null,
                                        "cutoffEpochMs": pending.accepted_epoch_ms
                                            .checked_add(pending.response_budget_ms),
                                        "originalBudgetMs": pending.response_budget_ms,
                                        "latencyMs": pending.response_budget_ms,
                                    }),
                                );
                                pending.response_projected = true;
                            } else if matches!(reserved.phase(), ReservedPhase::Begun { .. }) {
                                let handoff = pending
                                    .actor
                                    .begin_bound_task_handoff(
                                        exact_key.clone(),
                                        reserved.record_version(),
                                        clock.now_epoch_millis(),
                                        SCENARIO_TASK_TTL_MS,
                                        SCENARIO_TASK_POLL_INTERVAL_MS,
                                        Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                                    )
                                    .map_err(|error| {
                                        format!("commit begun cutoff handoff: {error}")
                                    })?;
                                telemetry.record_event(
                                    V5ReceiptRuntimeEventKind::BoundHandoffCommitted,
                                    clock.now_epoch_millis(),
                                );
                                let response = V5ServerResponse::Invocation {
                                    outcome: V5InvocationResponse::Task {
                                        snapshot: super::queued_receipt_task_snapshot(
                                            handoff.task(),
                                            handoff.key_digest().clone(),
                                            handoff.cancel_requested(),
                                        ),
                                    },
                                };
                                report.responses.insert(
                                    pending.label.clone(),
                                    json!({
                                        "kind": "task",
                                        "error": null,
                                        "terminal": null,
                                        "key": receipt_key_observation(&exact_key),
                                        "task": task_observation_from_response_with_workspace(
                                            response,
                                            &exact_key,
                                            state.path(),
                                            &identity,
                                            control.actor_workspace_identity().map(|identity| {
                                                serde_json::to_value(identity).ok().and_then(
                                                    |value| value.as_str().map(str::to_owned),
                                                )
                                            }),
                                        )?,
                                        "acknowledgement": null,
                                        "cutoffEpochMs": pending.accepted_epoch_ms
                                            .checked_add(pending.response_budget_ms),
                                        "originalBudgetMs": pending.response_budget_ms,
                                        "latencyMs": pending.response_budget_ms,
                                    }),
                                );
                                pending.response_projected = true;
                            }
                        }
                    }
                }
                if control.fail_stop_deadline_reached(clock.now_monotonic_millis()) {
                    telemetry.record_forced_process_exit();
                    control.record_process_exit(SCENARIO_FAIL_STOP_GRACE_MS);
                }
            }
            ReceiptScenarioAction::Crash { point } => {
                if matches!(point, ScenarioCrashPoint::AfterSideEffectBeforeTerminal)
                    && pending_submit.is_none()
                {
                    control.arm_crash_after_side_effect();
                    continue;
                }
                if matches!(
                    point,
                    ScenarioCrashPoint::BeforeTaskStoreCreate
                        | ScenarioCrashPoint::AfterCancelFlagBeforeTaskCreate
                ) {
                    let pending = pending_submit.as_ref().ok_or_else(|| {
                        "protocol-v5 pre-create crash has no live submit".to_owned()
                    })?;
                    let state = pending
                        .actor
                        .recover(
                            exact_key.clone(),
                            Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                        )
                        .map_err(|error| format!("recover pre-create crash handoff: {error}"))?;
                    if !matches!(state, ReceiptState::TaskHandoffActorBound(_)) {
                        return Err(format!(
                            "protocol-v5 pre-create crash observed {}",
                            state.kind().diagnostic_name()
                        ));
                    }
                    telemetry.record_forced_process_exit();
                    control.record_process_exit(1);
                    control.release_all_barriers();
                    let pending = pending_submit
                        .take()
                        .expect("pre-create crashed submit exists");
                    let (_, _, _, _, actor, _, _, daemon) = pending.finish()?;
                    drop(actor);
                    daemon.stop_and_join(
                        "protocol-v5 receipt scenario daemon panicked during pre-create crash",
                    )?;
                    recovering_handoff_crash = true;
                    continue;
                }
                if matches!(point, ScenarioCrashPoint::ReservedBegun) {
                    let pending = pending_submit.as_ref().ok_or_else(|| {
                        "protocol-v5 ReservedBegun crash has no live submit".to_owned()
                    })?;
                    let reserved = match pending.actor.recover(
                        exact_key.clone(),
                        Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                    ) {
                        Ok(ReceiptState::Reserved(reserved))
                            if matches!(reserved.phase(), ReservedPhase::Begun { .. }) =>
                        {
                            reserved
                        }
                        Ok(other) => {
                            return Err(format!(
                                "protocol-v5 ReservedBegun crash observed {}",
                                other.kind().diagnostic_name()
                            ))
                        }
                        Err(error) => {
                            return Err(format!("recover protocol-v5 ReservedBegun crash: {error}"))
                        }
                    };
                    let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Failed {
                        reason: V5SafeFailureReason::OutcomeUncertain,
                    })
                    .map_err(|error| format!("encode uncertain crash terminal: {error}"))?;
                    pending
                        .actor
                        .publish_direct_terminal(
                            exact_key.clone(),
                            reserved.record_version(),
                            clock.now_epoch_millis(),
                            terminal,
                            Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                        )
                        .map_err(|error| format!("terminalize crashed begun receipt: {error}"))?;
                    control.release(ScenarioBarrierPoint::BeforePrepare);
                    telemetry.record_event(
                        V5ReceiptRuntimeEventKind::ReceiptTerminalCommitted,
                        clock.now_epoch_millis(),
                    );
                    let pending = pending_submit
                        .take()
                        .expect("crashed begun submit was checked immediately before take");
                    let (label, accepted, budget, response, actor, _, _, daemon) =
                        pending.finish()?;
                    drop(actor);
                    daemon.stop_and_join(
                        "protocol-v5 receipt scenario daemon panicked during begun crash",
                    )?;
                    report
                        .responses
                        .entry(label)
                        .or_insert(response_observation(&response, Some((accepted, budget)))?);
                } else if matches!(point, ScenarioCrashPoint::TaskPromisedUnbound) {
                    let pending = pending_submit.as_ref().ok_or_else(|| {
                        "protocol-v5 TaskPromisedUnbound crash has no live submit".to_owned()
                    })?;
                    let promised = match pending.actor.recover(
                        exact_key.clone(),
                        Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                    ) {
                        Ok(ReceiptState::TaskPromisedUnbound(promised)) => promised,
                        Ok(other) => {
                            return Err(format!(
                                "protocol-v5 TaskPromisedUnbound crash observed {}",
                                other.kind().diagnostic_name()
                            ))
                        }
                        Err(error) => {
                            return Err(format!(
                                "recover protocol-v5 TaskPromisedUnbound crash: {error}"
                            ))
                        }
                    };
                    let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Failed {
                        reason: V5SafeFailureReason::Interrupted,
                    })
                    .map_err(|error| format!("encode interrupted crash terminal: {error}"))?;
                    let committed = pending
                        .actor
                        .publish_receipt_backed_task_terminal(
                            exact_key.clone(),
                            TaskCancellationReceipt::PromisedUnbound(promised),
                            clock.now_epoch_millis(),
                            terminal,
                            Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                        )
                        .map_err(|error| {
                            format!("terminalize crashed unbound Task promise: {error}")
                        })?;
                    control.record_receipt_backed_terminal(committed)?;
                    control.release_pre_actor_barriers();
                    telemetry.record_event(
                        V5ReceiptRuntimeEventKind::ReceiptTerminalCommitted,
                        clock.now_epoch_millis(),
                    );
                    let pending = pending_submit
                        .take()
                        .expect("crashed pending submit was checked immediately before take");
                    let (
                        label,
                        accepted_epoch_ms,
                        response_budget_ms,
                        response,
                        actor,
                        _,
                        _,
                        daemon,
                    ) = pending.finish()?;
                    drop(actor);
                    daemon.stop_and_join(
                        "protocol-v5 receipt scenario daemon panicked during promised crash",
                    )?;
                    report
                        .responses
                        .entry(label)
                        .or_insert(response_observation(
                            &response,
                            Some((accepted_epoch_ms, response_budget_ms)),
                        )?);
                } else if pending_submit.is_some() {
                    return Ok(None);
                }
            }
            ReceiptScenarioAction::Restart => {
                startup_listener_override = true;
                if pending_submit.is_some() {
                    if !control.process_exited() {
                        return Err(
                            "protocol-v5 receipt scenario cannot restart a live submit".to_owned()
                        );
                    }
                    let pending = pending_submit.as_ref().expect("pending submit exists");
                    if let ReceiptState::Reserved(reserved) = pending
                        .actor
                        .recover(
                            exact_key.clone(),
                            Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                        )
                        .map_err(|error| format!("recover fail-stopped submit: {error}"))?
                    {
                        if matches!(reserved.phase(), ReservedPhase::Begun { .. }) {
                            let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Failed {
                                reason: V5SafeFailureReason::OutcomeUncertain,
                            })
                            .map_err(|error| {
                                format!("encode fail-stopped begun terminal: {error}")
                            })?;
                            pending
                                .actor
                                .publish_direct_terminal(
                                    exact_key.clone(),
                                    reserved.record_version(),
                                    clock.now_epoch_millis(),
                                    terminal,
                                    Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                                )
                                .map_err(|error| {
                                    format!("terminalize fail-stopped begun submit: {error}")
                                })?;
                            telemetry.record_event(
                                V5ReceiptRuntimeEventKind::ReceiptTerminalCommitted,
                                clock.now_epoch_millis(),
                            );
                        }
                    }
                    control.release_all_barriers();
                    let pending = pending_submit
                        .take()
                        .expect("fail-stopped pending submit exists");
                    let (label, accepted, budget, response, actor, _, _, daemon) =
                        pending.finish()?;
                    drop(actor);
                    daemon.stop_and_join(
                        "protocol-v5 receipt scenario daemon panicked after fail-stop",
                    )?;
                    report
                        .responses
                        .entry(label)
                        .or_insert(response_observation(&response, Some((accepted, budget)))?);
                }
                if let Some(daemon) = live_daemon.take() {
                    live_actor = None;
                    live_task_projection = None;
                    daemon.stop_and_join(
                        "protocol-v5 receipt scenario live daemon panicked before restart",
                    )?;
                }
                let daemon_state = DaemonStateDirectory::open(state.path(), &identity)?;
                let config =
                    scenario_server_config_with_clock(state.path(), &identity, None, &clock);
                match V5ReceiptRuntime::open_with_epoch_clock(&daemon_state, &config, clock.clone())
                {
                    Ok(runtime) => {
                        drop(runtime);
                        if recovering_handoff_crash {
                            telemetry.record_task_store_create_attempt();
                            recovering_handoff_crash = false;
                        }
                        startup_failed = false;
                        listener_published = true;
                    }
                    Err(_) => {
                        startup_failed = true;
                        listener_published = false;
                    }
                }
            }
            ReceiptScenarioAction::Checkpoint { label } => {
                let (mut snapshot, live_task_projection) = match &pending_submit {
                    Some(pending) => (
                        snapshot_with_actor(
                            &pending.actor,
                            &clock,
                            &telemetry,
                            control.side_effect_markers(),
                            &known_keys,
                        )?,
                        Some((&pending.task_projection, pending.task_store_create_attempts)),
                    ),
                    None => match &live_actor {
                        Some(actor) => (
                            snapshot_with_actor(
                                actor,
                                &clock,
                                &telemetry,
                                control.side_effect_markers(),
                                &known_keys,
                            )?,
                            live_task_projection
                                .as_ref()
                                .map(|(projection, attempts)| (projection, *attempts)),
                        ),
                        None => (
                            snapshot_from_state(
                                state.path(),
                                &identity,
                                Arc::clone(&clock),
                                &telemetry,
                                &control,
                                &known_keys,
                            )?,
                            None,
                        ),
                    },
                };
                match live_task_projection {
                    Some((projection, expected_create_attempts)) => {
                        if let Some(terminal_task) = control.terminal_bound_task() {
                            let projection = terminal_bound_task_projection_observation(
                                terminal_task,
                                state.path(),
                                &identity,
                            )?;
                            apply_task_projection(&mut snapshot, &projection)?;
                        } else if let Some(bound_task) = control.bound_task() {
                            let projection = bound_task_projection_observation(
                                bound_task,
                                state.path(),
                                &identity,
                            )?;
                            apply_task_projection(&mut snapshot, &projection)?;
                        } else {
                            if telemetry.snapshot().task_store_create_attempts
                                != expected_create_attempts
                            {
                                return Err(
                                    "live protocol-v5 checkpoint crossed an unobserved TaskStore mutation"
                                        .to_owned(),
                                );
                            }
                            apply_task_projection(&mut snapshot, projection)?;
                        }
                    }
                    None => enrich_task_projection_snapshot(
                        &mut snapshot,
                        state.path(),
                        &identity,
                        Arc::clone(&clock),
                        &known_keys,
                        &seeded_task_versions,
                    )?,
                }
                if startup_listener_override {
                    snapshot["listener"] = Value::String(
                        if listener_published {
                            "listening"
                        } else {
                            "not_published"
                        }
                        .to_owned(),
                    );
                    snapshot["restartRequested"] = Value::Bool(startup_failed);
                    snapshot["daemonRunning"] = Value::Bool(listener_published && !startup_failed);
                }
                if let Some(elapsed_ms) = control.process_exit_elapsed_ms() {
                    snapshot["processExitElapsedMs"] = Value::from(elapsed_ms);
                }
                report.terminal_publications = telemetry.snapshot().terminal_publications;
                report.checkpoints.insert(label, snapshot);
            }
            ReceiptScenarioAction::Reset => {
                if pending_submit.is_some() {
                    return Err(
                        "protocol-v5 receipt scenario cannot reset a live submit".to_owned()
                    );
                }
                if let Some(daemon) = live_daemon.take() {
                    live_actor = None;
                    live_task_projection = None;
                    daemon.stop_and_join(
                        "protocol-v5 receipt scenario live daemon panicked before reset",
                    )?;
                }
                telemetry.reset_for_scenario();
                control.reset_for_scenario();
                state = ScenarioStateRoot::new()?;
                let reset_daemon_state = DaemonStateDirectory::open(state.path(), &identity)?;
                let reset_receipts =
                    reset_daemon_state.create_private_retained_subdirectory("receipts")?;
                control.set_state_root(reset_receipts.path());
                known_keys.clear();
                deferred_task_bound = None;
                startup_failed = false;
                listener_published = false;
                startup_listener_override = false;
                seeded_task_versions.clear();
            }
            ReceiptScenarioAction::FillReceiptPool { state: fill, count } => {
                if !matches!(
                    fill,
                    ScenarioSeedReceiptState::CancelReserved
                        | ScenarioSeedReceiptState::ReservedUnbound
                ) {
                    return Ok(None);
                }
                let daemon_state = DaemonStateDirectory::open(state.path(), &identity)?;
                let config =
                    scenario_server_config_with_clock(state.path(), &identity, None, &clock);
                let runtime =
                    V5ReceiptRuntime::open_with_epoch_clock(&daemon_state, &config, clock.clone())?;
                let epoch_ms = clock.now_epoch_millis();
                for index in 0..count {
                    let key = if matches!(fill, ScenarioSeedReceiptState::CancelReserved)
                        && known_keys.is_empty()
                        && index == 0
                    {
                        exact_key.clone()
                    } else {
                        fresh_key_for_workspace(&identity, &arguments, &workspace_hint)?
                    };
                    telemetry
                        .record_event(V5ReceiptRuntimeEventKind::V5ReceiptRuntimeEntered, epoch_ms);
                    match fill {
                        ScenarioSeedReceiptState::CancelReserved => {
                            runtime
                                .receipt_ledger
                                .request_cancel_or_reserve(
                                    key.clone(),
                                    epoch_ms,
                                    Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                                )
                                .map_err(|error| format!("fill CancelReserved pool: {error}"))?;
                        }
                        ScenarioSeedReceiptState::ReservedUnbound => {
                            runtime
                                .receipt_ledger
                                .reserve(
                                    key.clone(),
                                    OriginalCutoffDescriptor::new(epoch_ms, 7_000).map_err(
                                        |error| format!("construct filled receipt cutoff: {error}"),
                                    )?,
                                    Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                                )
                                .map_err(|error| format!("fill ReservedUnbound pool: {error}"))?;
                        }
                        _ => unreachable!("receipt pool support guard narrowed the seed state"),
                    }
                    push_known_key(&mut known_keys, key);
                }
            }
            ReceiptScenarioAction::InstallBarrier { point } => {
                control.install(point);
            }
            ReceiptScenarioAction::WaitForEvent { event } => match event {
                ScenarioEvent::V5ReceiptRuntimeEntered => {
                    telemetry.wait_for_event(
                        V5ReceiptRuntimeEventKind::V5ReceiptRuntimeEntered,
                        Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                    )?;
                }
                ScenarioEvent::CanonicalV13ServiceEntered => {
                    telemetry.wait_for_event(
                        V5ReceiptRuntimeEventKind::CanonicalV13ServiceEntered,
                        Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                    )?;
                }
                ScenarioEvent::ReceiptReserved => {
                    telemetry.wait_for_event(
                        V5ReceiptRuntimeEventKind::ReceiptReserved,
                        Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                    )?;
                }
                ScenarioEvent::ValidationEntered => {
                    telemetry.wait_for_event(
                        V5ReceiptRuntimeEventKind::ValidationEntered,
                        Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                    )?;
                }
                ScenarioEvent::AdmissionEntered => {
                    telemetry.wait_for_event(
                        V5ReceiptRuntimeEventKind::AdmissionEntered,
                        Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                    )?;
                }
                ScenarioEvent::ActorBoundCommitted => {
                    telemetry.wait_for_event(
                        V5ReceiptRuntimeEventKind::ActorBoundCommitted,
                        Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                    )?;
                }
                ScenarioEvent::ReceiptBegunCommitted => {
                    telemetry.wait_for_event(
                        V5ReceiptRuntimeEventKind::ReceiptBegunCommitted,
                        Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                    )?;
                }
                ScenarioEvent::PrepareEntered => {
                    telemetry.wait_for_event(
                        V5ReceiptRuntimeEventKind::PrepareEntered,
                        Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                    )?;
                }
                ScenarioEvent::ExecuteEntered => {
                    telemetry.wait_for_event(
                        V5ReceiptRuntimeEventKind::ExecuteEntered,
                        Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                    )?;
                }
                ScenarioEvent::CancelReservationConverted => {
                    control.wait_until_reached(
                        ScenarioBarrierPoint::AfterCancelReservationConvertedBeforeTerminal,
                        Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                    )?;
                    telemetry.wait_for_event(
                        V5ReceiptRuntimeEventKind::CancelReservationConverted,
                        Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                    )?;
                }
                ScenarioEvent::ReceiptTerminalCommitted => {
                    if pending_submit.is_some() {
                        return Err(
                            "protocol-v5 receipt scenario terminal wait preceded barrier release"
                                .to_owned(),
                        );
                    }
                    if !telemetry.snapshot().events.iter().any(|event| {
                        matches!(
                            event.event,
                            V5ReceiptRuntimeEventKind::TaskTerminalBoundCommitted
                        )
                    }) {
                        telemetry.wait_for_event(
                            V5ReceiptRuntimeEventKind::ReceiptTerminalCommitted,
                            Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                        )?;
                    }
                }
                ScenarioEvent::ResultSerialized => {
                    telemetry.wait_for_event(
                        V5ReceiptRuntimeEventKind::ResultSerialized,
                        Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                    )?;
                }
                ScenarioEvent::FinalResultProjected => {
                    telemetry.wait_for_event(
                        V5ReceiptRuntimeEventKind::FinalResultProjected,
                        Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                    )?;
                }
                ScenarioEvent::AcknowledgementCommitted => {
                    telemetry.wait_for_event(
                        V5ReceiptRuntimeEventKind::AcknowledgementCommitted,
                        Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                    )?;
                }
                ScenarioEvent::BoundHandoffCommitted => {
                    telemetry.wait_for_event(
                        V5ReceiptRuntimeEventKind::BoundHandoffCommitted,
                        Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                    )?;
                }
                ScenarioEvent::TaskBoundCommitted => {
                    telemetry.wait_for_event(
                        V5ReceiptRuntimeEventKind::TaskBoundCommitted,
                        Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                    )?;
                }
                ScenarioEvent::TaskStoreWorkingReadback => {
                    telemetry.wait_for_event(
                        V5ReceiptRuntimeEventKind::TaskStoreWorkingReadback,
                        Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                    )?;
                }
            },
            ReceiptScenarioAction::ReleaseBarrier { point } => {
                control.release(point);
                if !control.has_unreleased_barriers() {
                    let pending = pending_submit.take().ok_or_else(|| {
                        "protocol-v5 receipt scenario has no submit at the barrier".to_owned()
                    })?;
                    let (
                        label,
                        accepted_epoch_ms,
                        response_budget_ms,
                        response,
                        actor,
                        task_projection,
                        task_store_create_attempts,
                        daemon,
                    ) = pending.finish()?;
                    let close_after_release = telemetry.snapshot().restart_requested
                        || matches!(
                            point,
                            ScenarioBarrierPoint::AfterCancelReservationConvertedBeforeTerminal
                        );
                    if close_after_release {
                        drop(actor);
                        daemon.stop_and_join(
                            "protocol-v5 receipt scenario daemon panicked after barrier release",
                        )?;
                    } else {
                        live_actor = Some(actor);
                        live_task_projection = Some((task_projection, task_store_create_attempts));
                        live_daemon = Some(daemon);
                    }
                    if !report.responses.contains_key(&label) {
                        let observation = if matches!(
                            &response,
                            V5ServerResponse::Invocation {
                                outcome: V5InvocationResponse::Task { .. }
                            }
                        ) {
                            let task = task_observation_from_response_with_workspace(
                                response.clone(),
                                &exact_key,
                                state.path(),
                                &identity,
                                Some(control.actor_workspace_identity().and_then(|identity| {
                                    serde_json::to_value(identity)
                                        .ok()
                                        .and_then(|value| value.as_str().map(str::to_owned))
                                })),
                            )?;
                            json!({
                                "kind": "task",
                                "error": null,
                                "terminal": task.get("terminal").cloned().unwrap_or(Value::Null),
                                "key": receipt_key_observation(&exact_key),
                                "task": task,
                                "acknowledgement": null,
                                "cutoffEpochMs": accepted_epoch_ms.checked_add(response_budget_ms),
                                "originalBudgetMs": response_budget_ms,
                                "latencyMs": response_budget_ms,
                            })
                        } else {
                            response_observation(
                                &response,
                                Some((accepted_epoch_ms, response_budget_ms)),
                            )?
                        };
                        report.responses.insert(label, observation);
                    }
                    for duplicate_label in pending_duplicate_labels.drain(..) {
                        let duplicate_response =
                            recover_from_live_daemon(state.path(), &identity, exact_key.clone())?;
                        report.responses.insert(
                            duplicate_label,
                            response_observation(
                                &duplicate_response,
                                Some((accepted_epoch_ms, response_budget_ms)),
                            )?,
                        );
                    }
                }
            }
            ReceiptScenarioAction::CompareClientServerIdentity => {
                report.identity = Some(compare_client_server_identity()?);
            }
        }
    }

    if pending_submit.is_some() {
        return Err("protocol-v5 receipt scenario ended with a blocked submit".to_owned());
    }
    for (receipt, record_bytes) in control.receipt_backed_terminals() {
        telemetry.record_receipt_backed_publication(&receipt, &record_bytes);
    }
    report.terminal_publications = telemetry.snapshot().terminal_publications;
    report.actor_bindings = control.actor_bindings();
    let encoded = report.encode(telemetry.snapshot().events).map(Some);
    drop(live_actor.take());
    let cleanup = match live_daemon.take() {
        Some(daemon) => daemon.stop_and_join(
            "protocol-v5 receipt scenario live daemon panicked during final cleanup",
        ),
        None => Ok(()),
    };
    finish_with_daemon_cleanup(encoded, cleanup)
}

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
    let snapshot = match super::receipt_state_task_snapshot_for_test(state) {
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
    let mut owner = V5DaemonProcessOwner::connect_or_spawn_for_protocol_test(
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
    let mut owner = V5DaemonProcessOwner::connect_or_spawn_for_protocol_test(
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
    let store = ReceiptLedgerStore::open_retained_directory(receipts)
        .map_err(|error| format!("open receipt-backed Task fixture ledger: {error}"))?;
    store
        .seed_task_terminal_receipt_backed_for_test(
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
        )
        .map_err(|error| format!("seed receipt-backed Task terminal: {error}"))?;
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
    if staged_terminal.is_some() && !seeds_direct_terminal {
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
                .materialize_bound_handoff(&handoff, epoch_ms, deadline, &runtime.telemetry)
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
        | ScenarioSeedReceiptState::TaskHandoffActorBoundBegun => {
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
                | ScenarioSeedReceiptState::TaskHandoffActorBoundBegun => {
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
                    ) {
                        runtime
                            .receipt_ledger
                            .mark_reserved_begun(key.clone(), bound.record_version(), deadline)
                            .map_err(|error| format!("seed begun receipt: {error}"))?
                            .record_version()
                    } else {
                        bound.record_version()
                    };
                    runtime
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
                }
                _ => unreachable!("closed seeded reservation family"),
            }
            if cancel_requested {
                runtime
                    .receipt_ledger
                    .request_cancel_or_reserve(key, epoch_ms, deadline)
                    .map_err(|error| format!("seed receipt cancellation: {error}"))?;
            }
            true
        }
        ScenarioSeedReceiptState::TaskReceiptOwnedActorBound
        | ScenarioSeedReceiptState::TaskTerminalBound
        | ScenarioSeedReceiptState::TaskTerminalReceiptBacked => false,
    };
    drop(runtime);
    Ok(supported)
}

fn scenario_workspace_identity_hash() -> SafeIdentityHash {
    SafeIdentityHash::from_sha256(Sha256::digest(b"unica.d0.scenario-workspace.v1").into())
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
        Some(ScenarioSeedReceiptState::TaskBoundNotBegun) | None => {
            crate::application::receipt_ledger::AttemptPhase::NotBegun
        }
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
    let reservation = links
        .reserve_task_link(link_key.clone(), link, deadline)
        .map_err(|error| format!("reserve seeded lifecycle link: {error}"))?;
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

fn task_terminal_digest_from_store(
    state_root: &Path,
    identity: &CoreIdentity,
    key: &ReceiptKey,
) -> Result<TerminalDigest, String> {
    let state = DaemonStateDirectory::open(state_root, identity)?;
    let receipts = state.create_private_retained_subdirectory("receipts")?;
    let store = ReceiptLedgerStore::open_retained_directory(receipts)
        .map_err(|error| format!("open receipt-backed Task digest ledger: {error}"))?;
    let actor = ReceiptLedgerActor::spawn(store);
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

fn has_supported_shape(request: &str) -> Result<bool, String> {
    let value: Value = serde_json::from_str(request)
        .map_err(|error| format!("decode receipt scenario shape: {error}"))?;
    if value.get("clock").and_then(Value::as_str) != Some("fake") {
        return Ok(false);
    }
    let Some(actions) = value.get("actions").and_then(Value::as_array) else {
        return Ok(false);
    };
    Ok(!actions.is_empty()
        && actions.iter().all(|action| {
            matches!(
                action.get("action").and_then(Value::as_str),
                Some(
                    "configure_validation"
                        | "configure_provider"
                        | "configure_admission"
                        | "configure_prepare"
                        | "cancel"
                        | "submit"
                        | "send_outer_envelope"
                        | "probe_protocol"
                        | "recover"
                        | "acknowledge"
                        | "seed_receipt"
                        | "seed_task"
                        | "seed_task_link_reservation"
                        | "open_task_store_inspect_only"
                        | "reconcile_startup"
                        | "publish_listener"
                        | "read_task"
                        | "advance_epoch"
                        | "advance_monotonic"
                        | "crash"
                        | "restart"
                        | "checkpoint"
                        | "reset"
                        | "fill_receipt_pool"
                        | "install_barrier"
                        | "wait_for_event"
                        | "release_barrier"
                        | "compare_client_server_identity"
                )
            )
        }))
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
    let payload = match fixture {
        ScenarioTerminalFixture::Success { payload } => payload.clone(),
        ScenarioTerminalFixture::Bytes { count } => "x".repeat(
            usize::try_from(*count)
                .map_err(|_| "scenario provider byte count does not fit usize".to_owned())?,
        ),
        ScenarioTerminalFixture::NearLimitWithMaximumMetadata {
            canonical_result_bytes,
        } => "x".repeat(usize::try_from(*canonical_result_bytes).map_err(|_| {
            "scenario provider canonical result size does not fit usize".to_owned()
        })?),
    };
    Ok(DomainResult::success(payload))
}

fn scenario_server_config(
    state_root: &Path,
    identity: &CoreIdentity,
    control: Option<&Arc<ReceiptScenarioControl>>,
) -> DaemonServerConfig {
    let mut config = DaemonServerConfig::new(
        state_root.to_path_buf(),
        identity.clone(),
        SCENARIO_IDLE_GRACE,
    );
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

fn acknowledge_without_startup(
    state_root: &Path,
    identity: &CoreIdentity,
    clock: Arc<ScenarioEpochClock>,
    telemetry: Arc<V5ReceiptRuntimeTelemetry>,
    key: ReceiptKey,
    terminal_digest: TerminalDigest,
) -> Result<V5ServerResponse, String> {
    let state = DaemonStateDirectory::open(state_root, identity)?;
    let receipts = state.create_private_retained_subdirectory("receipts")?;
    let store = ReceiptLedgerStore::open_retained_directory(receipts)
        .map_err(|error| format!("open protocol-v5 receipt ACK owner: {error}"))?;
    let actor = ReceiptLedgerActor::spawn(store);
    let epoch_ms = clock.now_epoch_millis();
    let result = actor.acknowledge_direct(
        key,
        terminal_digest,
        epoch_ms,
        Instant::now() + SCENARIO_OPERATION_TIMEOUT,
    );
    let response = match result {
        Ok(receipt) => {
            telemetry.record_event(
                V5ReceiptRuntimeEventKind::AcknowledgementCommitted,
                epoch_ms,
            );
            V5ServerResponse::InvocationAcknowledged {
                acknowledgement: V5AcknowledgedReceipt::from_receipt(&receipt),
            }
        }
        Err(error) => V5ServerResponse::Error {
            code: daemon_error_code(&error),
        },
    };
    drop(actor);
    Ok(response)
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
        let mut runtime = runtime.with_shared_telemetry(telemetry);
        runtime.epoch_clock = clock;
        runtime.scenario_control = scenario_control;
        runtime
    });
    let response = (|| {
        wait_for_endpoint(state_root, identity)?;
        let mut owner = V5DaemonProcessOwner::connect_or_spawn_for_protocol_test(
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
        let mut runtime = runtime.with_shared_telemetry(telemetry);
        runtime.epoch_clock = clock;
        runtime.scenario_control = scenario_control;
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
            move |runtime| runtime.with_shared_telemetry(telemetry)
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
    let identity = presented_core_identity(client, &message, &CoreIdentity::production())?;
    let record = protocol_v3::EndpointRecord::new(identity.clone(), 9);
    let client_hello_frame = jsonl_frame(&protocol_v3::ClientRequest::hello(
        protocol_version_number(client),
        record.token().to_string(),
        identity.clone(),
    ))?;
    let request_frame = build_v3_probe_request_frame(client, &message)?;
    let accepted = client == ScenarioProtocolVersion::V3;
    let response = if accepted {
        match message {
            ScenarioProtocolMessage::SubmitWithCoreIdentity { .. } => {
                protocol_v3::ServerResponse::invocation(protocol_v3::InvocationResponse::Direct(
                    crate::domain::invocation::DomainResult::success("v3-guard"),
                ))
            }
            ScenarioProtocolMessage::DirectFailureTerminal { reason }
                if !failure_reason_introduced_in_v5(reason) =>
            {
                let reason = fixture_failure_reason(reason);
                let (code, message) = failure_projection(reason);
                protocol_v3::ServerResponse::invocation(protocol_v3::InvocationResponse::Direct(
                    DomainResult::canonical_rejection(None, code, message),
                ))
            }
            ScenarioProtocolMessage::DirectFailureTerminal { .. } => {
                protocol_v3::ServerResponse::error(protocol_v3::DaemonErrorCode::InvalidRequest)
            }
            ScenarioProtocolMessage::StoredInvocationRecord {
                schema_version,
                reason,
            } => {
                let _closed_v5_record = (schema_version, fixture_failure_reason(reason));
                protocol_v3::ServerResponse::error(protocol_v3::DaemonErrorCode::InvalidRequest)
            }
            ScenarioProtocolMessage::Release => protocol_v3::ServerResponse::Released,
            _ => protocol_v3::ServerResponse::Pong,
        }
    } else {
        protocol_v3::ServerResponse::error(protocol_v3::DaemonErrorCode::ProtocolMismatch)
    };
    let response_frame = jsonl_frame(&response)?;
    Ok((
        protocol_probe_observation(
            &label,
            client,
            ScenarioProtocolVersion::V3,
            ProtocolProbeFrames {
                client_hello: client_hello_frame,
                server_ready: accepted.then(|| {
                    jsonl_frame(&protocol_v3::ServerResponse::ready(&record))
                        .expect("serialize protocol-v3 ready frame")
                }),
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
                production_events: production_events(accepted, response.error_code().is_none()),
                error: response_error_value_v3(&response)?,
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
        let published = match identity.protocol_identity() {
            crate::infrastructure::daemon::identity::DaemonProtocolIdentity::V3 => {
                state.read_endpoint_record()?.is_some()
            }
            crate::infrastructure::daemon::identity::DaemonProtocolIdentity::V5 => {
                state.read_v5_endpoint_record()?.is_some()
            }
        };
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
                &ScenarioEpochClock::new(protocol_probe_epoch_ms()),
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
    let store = ReceiptLedgerStore::open_retained_directory(receipts)
        .map_err(|error| format!("open protocol-v5 probe receipt ledger: {error}"))?;
    let actor = ReceiptLedgerActor::spawn(store);
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
        ScenarioProtocolMessage::Ping => jsonl_frame(&protocol_v3::ClientRequest::Ping {}),
        ScenarioProtocolMessage::Release => jsonl_frame(&protocol_v3::ClientRequest::Release {}),
        ScenarioProtocolMessage::SubmitWithCoreIdentity { .. } => {
            jsonl_frame(&protocol_v3::ClientRequest::SubmitInvocation {
                invocation: protocol_v3::InvocationRequest::new(
                    ToolIdentity::View,
                    Value::Object(Map::new()),
                    "workspace-a",
                    7_000,
                )?,
            })
        }
        ScenarioProtocolMessage::GetTask => jsonl_frame(&protocol_v3::ClientRequest::GetTask {
            task_id: TaskId::new(),
        }),
        ScenarioProtocolMessage::WaitTask => jsonl_frame(&protocol_v3::ClientRequest::WaitTask {
            task_id: TaskId::new(),
            wait_ms: 7_000,
        }),
        ScenarioProtocolMessage::CancelTask => {
            jsonl_frame(&protocol_v3::ClientRequest::CancelTask {
                task_id: TaskId::new(),
            })
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
            jsonl_frame(&protocol_v3::ClientRequest::Ping {})
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

fn response_error_value_v3(
    response: &protocol_v3::ServerResponse,
) -> Result<Option<Value>, String> {
    match response.error_code() {
        Some(code) => serde_json::to_value(code)
            .map(Some)
            .map_err(|error| format!("encode protocol-v3 response error: {error}")),
        None => Ok(None),
    }
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
        let mut runtime = runtime.with_shared_telemetry(telemetry);
        runtime.epoch_clock = clock;
        runtime.scenario_control = Some(control);
        runtime
    });
    let result = (|| {
        wait_for_endpoint(state_root, identity)?;
        let mut owner = V5DaemonProcessOwner::connect_or_spawn_for_protocol_test(
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
        let mut runtime = runtime.with_shared_telemetry(telemetry);
        runtime.epoch_clock = clock;
        runtime.scenario_control = Some(control);
        runtime
    });
    let result = (|| {
        wait_for_endpoint(state_root, identity)?;
        let mut owner = V5DaemonProcessOwner::connect_or_spawn_for_protocol_test(
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
        let mut runtime = runtime.with_shared_telemetry(daemon_telemetry);
        runtime.epoch_clock = clock;
        runtime.scenario_control = Some(control);
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
        let mut runtime = runtime.with_shared_telemetry(telemetry);
        runtime.epoch_clock = clock;
        runtime.scenario_control = scenario_control;
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

struct PendingSubmit {
    label: String,
    accepted_epoch_ms: u64,
    accepted_monotonic_ms: u64,
    response_budget_ms: u64,
    actor: ReceiptLedgerActor,
    task_projection: TaskProjectionObservation,
    task_store_create_attempts: u64,
    response_projected: bool,
    client: thread::JoinHandle<Result<V5ServerResponse, String>>,
    daemon: ScenarioDaemon,
}

impl PendingSubmit {
    fn submit_additional(
        &self,
        state_root: &Path,
        identity: &CoreIdentity,
        invocation: V5InvocationRequest,
    ) -> Result<V5ServerResponse, String> {
        let mut owner = V5DaemonProcessOwner::connect_or_spawn_for_protocol_test(
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

    fn finish(
        self,
    ) -> Result<
        (
            String,
            u64,
            u64,
            V5ServerResponse,
            ReceiptLedgerActor,
            TaskProjectionObservation,
            u64,
            ScenarioDaemon,
        ),
        String,
    > {
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
            daemon,
        } = self;
        let response = client
            .join()
            .map_err(|_| "protocol-v5 receipt scenario submit client panicked".to_owned())
            .and_then(|response| response);
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

fn cancel_on_live_daemon(
    state_root: &Path,
    identity: &CoreIdentity,
    key: ReceiptKey,
) -> Result<V5ServerResponse, String> {
    let mut owner = V5DaemonProcessOwner::connect_or_spawn_for_protocol_test(
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
    control.arm_skip_next_startup_reconciliation();
    let config = scenario_server_config_with_clock(state_root, identity, Some(&control), &clock);
    let (actor_tx, actor_rx) = mpsc::sync_channel(1);
    let daemon = ScenarioDaemon::spawn(config, move |runtime| {
        let mut runtime = runtime.with_shared_telemetry(telemetry);
        runtime.epoch_clock = clock;
        runtime.scenario_control = Some(control);
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
        let mut owner = V5DaemonProcessOwner::connect_or_spawn_for_protocol_test(
            &client_state_root,
            client_identity.clone(),
            std::path::PathBuf::from("unused-existing-v5-scenario-endpoint"),
            SCENARIO_IDLE_GRACE,
        )?;
        let response = owner.submit_invocation(invocation)?;
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
        client,
        daemon,
    })
}

fn wait_for_endpoint(state_root: &Path, identity: &CoreIdentity) -> Result<(), String> {
    let deadline = Instant::now() + SCENARIO_OPERATION_TIMEOUT;
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
    let store = ReceiptLedgerStore::open_retained_directory(receipts)
        .map_err(|error| format!("open protocol-v5 receipt checkpoint store: {error}"))?;
    let actor = ReceiptLedgerActor::spawn(store);
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
    actor
        .reclaim_expired_tombstones(
            clock.now_epoch_millis(),
            Instant::now() + SCENARIO_OPERATION_TIMEOUT,
        )
        .map_err(|error| format!("reclaim protocol-v5 receipt tombstones: {error}"))?;
    let mut receipts = Vec::with_capacity(keys.len());
    for key in keys {
        match actor.recover(key.clone(), Instant::now() + SCENARIO_OPERATION_TIMEOUT) {
            Ok(ReceiptState::AcknowledgedTombstone(_)) => {}
            Ok(receipt) => receipts.push(receipt_observation(receipt)?),
            Err(ReceiptLedgerError::ReceiptNotFound) => {}
            Err(error) => {
                return Err(format!("snapshot protocol-v5 receipt scenario: {error}"));
            }
        }
    }
    let catalog = actor
        .snapshot_catalog(Instant::now() + SCENARIO_OPERATION_TIMEOUT)
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
        "tokenSignals": 0,
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
    let task = task_observation_from_response_with_workspace(
        V5ServerResponse::Task {
            snapshot: super::task_store_snapshot(&record),
        },
        bound.key(),
        state_root,
        identity,
        Some(workspace),
    )?;
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
    let task = task_observation_from_response_with_workspace(
        V5ServerResponse::Task {
            snapshot: super::task_store_snapshot(&record),
        },
        bound.key(),
        state_root,
        identity,
        Some(workspace),
    )?;
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

fn inspect_task_projection(
    state_root: &Path,
    identity: &CoreIdentity,
    clock: Arc<ScenarioEpochClock>,
    known_keys: &[ReceiptKey],
    seeded_task_versions: &HashMap<TaskId, u64>,
) -> Result<TaskProjectionObservation, String> {
    let state = DaemonStateDirectory::open(state_root, identity)?;
    let deadline = crate::domain::code_intelligence::ProviderDeadline::new(
        Instant::now() + SCENARIO_OPERATION_TIMEOUT,
    );
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
        let task_record = task_store
            .get(task_id, deadline)
            .map_err(|error| format!("read lifecycle-linked TaskStore record: {error}"))?;
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
            staged_terminal: None,
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
            let recovery_error = if submit_cutoff.is_none() {
                match receipt.terminal() {
                    ReceiptTerminalOutcome::Failed { reason } => {
                        Some(serde_json::to_value(reason).map_err(|error| {
                            format!("encode protocol-v5 recovery failure reason: {error}")
                        })?)
                    }
                    ReceiptTerminalOutcome::Completed { .. }
                    | ReceiptTerminalOutcome::Cancelled => None,
                }
            } else {
                None
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
    actor_bindings: Vec<Value>,
    protocol: Vec<Value>,
}
impl ScenarioReportBuilder {
    fn encode(self, events: Vec<V5ReceiptRuntimeEvent>) -> Result<String, String> {
        serde_json::to_string(&json!({
            "kind": "observed",
            "payload": {
                "checkpoints": self.checkpoints,
                "responses": self.responses,
                "taskReads": self.task_reads,
                "events": events,
                "gateEvents": [],
                "operationEvents": [],
                "actorBindings": self.actor_bindings,
                "actorAuthorizations": [],
                "taskPublicationCapacity": [],
                "taskStoreCapacityInvariantViolations": [],
                "stagedTerminalPreparations": [],
                "terminalPublications": self.terminal_publications,
                "protocol": self.protocol,
                "identity": self.identity,
                "crashCases": [],
                "taskRetirementCases": [],
                "loadRuns": {}
            }
        }))
        .map_err(|error| format!("encode protocol-v5 receipt scenario report: {error}"))
    }
}

struct ScenarioEpochClock {
    epoch_ms: AtomicU64,
    monotonic_origin: Instant,
    monotonic_ms: AtomicU64,
}

impl ScenarioEpochClock {
    fn new(epoch_ms: u64) -> Self {
        Self {
            epoch_ms: AtomicU64::new(epoch_ms),
            monotonic_origin: Instant::now(),
            monotonic_ms: AtomicU64::new(0),
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
        self.monotonic_ms.load(Ordering::SeqCst)
    }
}

impl EpochMillisClock for ScenarioEpochClock {
    fn now_epoch_millis(&self) -> u64 {
        self.epoch_ms.load(Ordering::SeqCst)
    }
}

impl Clock for ScenarioEpochClock {
    fn now(&self) -> Instant {
        self.monotonic_origin
            .checked_add(Duration::from_millis(
                self.monotonic_ms.load(Ordering::SeqCst),
            ))
            .unwrap_or(self.monotonic_origin)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReceiptScenario {
    clock: ScenarioClock,
    actions: Vec<ReceiptScenarioAction>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ScenarioClock {
    Fake,
    Wall,
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum ReceiptScenarioAction {
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
        lazy_session: bool,
        label: String,
    },
    Submit {
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
    OpenTaskStoreInspectOnly,
    ReconcileStartup,
    PublishListener,
    ReadTask {
        api: ScenarioTaskApi,
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

impl ReceiptScenarioAction {
    fn is_supported(&self) -> bool {
        match self {
            Self::ConfigureValidation { .. }
            | Self::ConfigureAdmission { .. }
            | Self::ConfigurePrepare { .. } => true,
            Self::ConfigureProvider {
                execution_class,
                terminal,
                ..
            } => {
                matches!(
                    execution_class,
                    ScenarioExecutionClass::Direct | ScenarioExecutionClass::KnownLong
                ) && matches!(terminal, ScenarioTerminalFixture::Success { .. })
            }
            Self::Cancel {
                key, lazy_session, ..
            } => {
                let _ = lazy_session;
                matches!(
                    key,
                    ScenarioKey::Exact
                        | ScenarioKey::Unknown
                        | ScenarioKey::Mismatch(ScenarioIdentityField::NormalizedArgumentsHash)
                )
            }
            Self::Submit {
                request,
                disconnect,
                ..
            } => {
                matches!(
                    request,
                    ScenarioRequest::Canonical | ScenarioRequest::SameIdentity
                ) && matches!(
                    disconnect,
                    ScenarioDisconnect::Never
                        | ScenarioDisconnect::AfterSubmitWrite
                        | ScenarioDisconnect::AfterTerminalCommit
                )
            }
            Self::SendOuterEnvelope { .. } => true,
            Self::ProbeProtocol {
                client,
                server,
                message,
                ..
            } => protocol_probe_is_supported(*client, *server, message),
            Self::Recover { key, .. } => matches!(key, ScenarioKey::Exact),
            Self::Acknowledge {
                key, disconnect, ..
            } => {
                matches!(key, ScenarioKey::Exact)
                    && matches!(
                        disconnect,
                        ScenarioAckDisconnect::Never | ScenarioAckDisconnect::AfterTombstoneCommit
                    )
            }
            Self::SeedReceipt {
                state,
                staged_terminal,
                cancel_requested,
            } => match state {
                ScenarioSeedReceiptState::TaskTerminalReceiptBacked => {
                    staged_terminal.as_ref().is_some_and(|terminal| {
                        matches!(terminal, ScenarioTerminalFixture::Success { .. })
                    })
                }
                ScenarioSeedReceiptState::CancelReserved
                | ScenarioSeedReceiptState::ReservedUnbound
                | ScenarioSeedReceiptState::ReservedActorBound
                | ScenarioSeedReceiptState::ReservedBegun
                | ScenarioSeedReceiptState::TaskPromisedUnbound
                | ScenarioSeedReceiptState::TaskPromisedActorBound
                | ScenarioSeedReceiptState::TaskHandoffActorBoundNotBegun
                | ScenarioSeedReceiptState::TaskHandoffActorBoundBegun => {
                    staged_terminal.is_none()
                        && (!*cancel_requested
                            || matches!(
                                state,
                                ScenarioSeedReceiptState::CancelReserved
                                    | ScenarioSeedReceiptState::ReservedUnbound
                                    | ScenarioSeedReceiptState::ReservedActorBound
                                    | ScenarioSeedReceiptState::ReservedBegun
                            ))
                }
                ScenarioSeedReceiptState::DirectTerminalUnacked
                | ScenarioSeedReceiptState::AcknowledgedTombstone => {
                    staged_terminal.as_ref().is_some_and(|terminal| {
                        matches!(terminal, ScenarioTerminalFixture::Success { .. })
                    })
                }
                ScenarioSeedReceiptState::TaskBoundNotBegun
                | ScenarioSeedReceiptState::TaskBoundBegun => staged_terminal.is_none(),
                ScenarioSeedReceiptState::TaskReceiptOwnedActorBound
                | ScenarioSeedReceiptState::TaskTerminalBound => false,
            },
            Self::SeedTask { version, .. } => *version > 0,
            Self::SeedTaskLinkReservation { .. }
            | Self::OpenTaskStoreInspectOnly
            | Self::ReconcileStartup
            | Self::PublishListener => true,
            Self::ReadTask { .. } => true,
            Self::AdvanceEpoch { .. }
            | Self::AdvanceMonotonic { .. }
            | Self::Restart
            | Self::Checkpoint { .. }
            | Self::Reset
            | Self::InstallBarrier { .. }
            | Self::WaitForEvent { .. }
            | Self::ReleaseBarrier { .. }
            | Self::CompareClientServerIdentity => true,
            Self::Crash { point } => matches!(
                point,
                ScenarioCrashPoint::ReservedUnbound
                    | ScenarioCrashPoint::ReservedBegun
                    | ScenarioCrashPoint::TaskPromisedUnbound
                    | ScenarioCrashPoint::AfterSideEffectBeforeTerminal
                    | ScenarioCrashPoint::BeforeTaskStoreCreate
                    | ScenarioCrashPoint::AfterCancelFlagBeforeTaskCreate
                    | ScenarioCrashPoint::AfterWorkingReadbackBeforeReceiptBegun
                    | ScenarioCrashPoint::AfterReceiptBegunBeforePrepare
            ),
            Self::FillReceiptPool { state, .. } => matches!(
                state,
                ScenarioSeedReceiptState::CancelReserved
                    | ScenarioSeedReceiptState::ReservedUnbound
            ),
        }
    }
}

fn protocol_probe_is_supported(
    _client: ScenarioProtocolVersion,
    server: ScenarioProtocolVersion,
    _message: &ScenarioProtocolMessage,
) -> bool {
    match server {
        ScenarioProtocolVersion::V4 => false,
        ScenarioProtocolVersion::V5 | ScenarioProtocolVersion::V3 => true,
    }
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ScenarioExecutionClass {
    Direct,
    KnownLong,
}

#[derive(Clone, Deserialize)]
#[serde(tag = "terminal", rename_all = "snake_case", deny_unknown_fields)]
enum ScenarioTerminalFixture {
    Success { payload: String },
    Bytes { count: u64 },
    NearLimitWithMaximumMetadata { canonical_result_bytes: u64 },
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ScenarioRequest {
    Canonical,
    SameIdentity,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ScenarioTaskApi {
    NativeGet,
    NativeWait,
    CompatibilityGet,
    CompatibilityResult,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ScenarioEnvelopeCase {
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
enum ScenarioProtocolVersion {
    V3,
    V4,
    V5,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ScenarioCoreIdentitySelection {
    ExactProductionV5,
    ArbitraryCanonical,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ScenarioFailureProbeReason {
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
enum ScenarioTaskTerminalOwnerFixture {
    ReceiptBacked,
    Bound,
    Staged,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ScenarioV5DaemonErrorCodeFixture {
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

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ScenarioStrictSchemaTarget {
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
enum ScenarioProtocolMessage {
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
enum ScenarioDisconnect {
    Never,
    AfterSubmitWrite,
    AfterTerminalCommit,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ScenarioAckDisconnect {
    Never,
    AfterTombstoneCommit,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ScenarioDigest {
    ExactTerminal,
    Mismatched,
    TaskTerminal,
    WellFormedCandidate,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ScenarioKey {
    Exact,
    Unknown,
    Mismatch(ScenarioIdentityField),
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ScenarioSeedReceiptState {
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
enum ScenarioTaskStatus {
    Queued,
    Working,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ScenarioIdentityRelation {
    Exact,
    Foreign,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ScenarioReceiptLinkCase {
    Exact,
    Missing,
    Foreign,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ScenarioBarrierPoint {
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

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ScenarioWorkspaceAdmissionFailure {
    Invalid,
    Capacity,
    RegistryFailed,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ScenarioCrashPoint {
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

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ScenarioEvent {
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
    TaskBoundCommitted,
    TaskStoreWorkingReadback,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ScenarioIdentityField {
    InvocationId,
    ReservedTaskId,
    CoreIdentity,
    ToolIdentity,
    NormalizedArgumentsHash,
    RequestScopeHash,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::receipt_ledger::{
        OriginalCutoffDescriptor, ACKNOWLEDGED_TOMBSTONE_TTL_MS, MAX_RECEIPT_ENTITLEMENT_BYTES,
    };
    use crate::infrastructure::daemon::protocol_v5::V5InvocationPhase as TestV5InvocationPhase;
    use crate::infrastructure::receipt_ledger::ReceiptLedgerStore;

    #[test]
    fn seeded_promised_and_handoff_states_cross_the_real_actor_store_path() {
        let identity = CoreIdentity::production_v5();
        let cases = [
            (
                ScenarioSeedReceiptState::TaskPromisedUnbound,
                None,
                "task_terminal_receipt_backed",
            ),
            (
                ScenarioSeedReceiptState::TaskPromisedActorBound,
                Some(V5SafeFailureReason::Interrupted),
                "task_store_terminal",
            ),
            (
                ScenarioSeedReceiptState::TaskHandoffActorBoundNotBegun,
                Some(V5SafeFailureReason::Interrupted),
                "task_store_terminal",
            ),
            (
                ScenarioSeedReceiptState::TaskHandoffActorBoundBegun,
                Some(V5SafeFailureReason::OutcomeUncertain),
                "task_store_terminal",
            ),
        ];

        for (seed_state, expected_task_failure, expected) in cases {
            let state_root = ScenarioStateRoot::new().expect("scenario state root");
            let clock = Arc::new(ScenarioEpochClock::new(SCENARIO_INITIAL_EPOCH_MS));
            let key = fresh_key(&identity, &Map::new()).expect("exact receipt key");
            assert!(seed_receipt_state(
                state_root.path(),
                &identity,
                &clock,
                key.clone(),
                seed_state,
                false,
                None,
            )
            .expect("seed exact receipt state"));

            let state = DaemonStateDirectory::open(state_root.path(), &identity)
                .expect("open daemon state");
            let config =
                scenario_server_config_with_clock(state_root.path(), &identity, None, &clock);
            let runtime = V5ReceiptRuntime::open_with_epoch_clock(&state, &config, clock.clone())
                .expect("reopen runtime");
            if let Some(expected_reason) = expected_task_failure {
                assert_eq!(
                    runtime
                        .receipt_ledger
                        .recover(key.clone(), Instant::now() + SCENARIO_OPERATION_TIMEOUT,),
                    Err(ReceiptLedgerError::ReceiptNotFound),
                    "startup must retire the transferred actor-bound receipt"
                );
                let snapshot = runtime
                    .resolve_task(
                        key.reserved_task_id(),
                        Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                    )
                    .expect("resolve startup-terminalized actor-bound Task");
                let V5DaemonTaskSnapshot::Failed {
                    reason,
                    cancel_requested,
                    ..
                } = snapshot
                else {
                    panic!("abandoned actor-owned Task must fail in TaskStore");
                };
                assert_eq!(reason, expected_reason);
                assert!(!cancel_requested);
                assert_eq!(expected, "task_store_terminal");
                continue;
            }
            let recovered = runtime
                .receipt_ledger
                .recover(key, Instant::now() + SCENARIO_OPERATION_TIMEOUT)
                .expect("recover exact seeded receipt");
            assert_eq!(recovered.kind().diagnostic_name(), expected);
            if matches!(seed_state, ScenarioSeedReceiptState::TaskPromisedUnbound) {
                let ReceiptState::TaskTerminalReceiptBacked(receipt) = recovered else {
                    panic!("startup must terminalize the abandoned unbound Task promise");
                };
                assert_eq!(
                    receipt.terminal().outcome(),
                    &ReceiptTerminalOutcome::Failed {
                        reason: V5SafeFailureReason::Interrupted,
                    }
                );
            }
        }
    }

    #[test]
    fn seeded_direct_and_bound_owners_cross_the_real_durable_stores() {
        let identity = CoreIdentity::production_v5();
        for (seed_state, expected_kind) in [
            (
                ScenarioSeedReceiptState::DirectTerminalUnacked,
                "direct_terminal_unacked",
            ),
            (
                ScenarioSeedReceiptState::AcknowledgedTombstone,
                "acknowledged_tombstone",
            ),
        ] {
            let state_root = ScenarioStateRoot::new().expect("scenario state root");
            let clock = Arc::new(ScenarioEpochClock::new(SCENARIO_INITIAL_EPOCH_MS));
            let key = fresh_key(&identity, &Map::new()).expect("exact receipt key");
            assert!(seed_receipt_state(
                state_root.path(),
                &identity,
                &clock,
                key.clone(),
                seed_state,
                false,
                Some(ScenarioTerminalFixture::Success {
                    payload: "seeded-direct".to_owned(),
                }),
            )
            .expect("seed direct owner"));
            let state = DaemonStateDirectory::open(state_root.path(), &identity)
                .expect("open daemon state");
            let config =
                scenario_server_config_with_clock(state_root.path(), &identity, None, &clock);
            let runtime = V5ReceiptRuntime::open_with_epoch_clock(&state, &config, clock.clone())
                .expect("reopen runtime");
            let recovered = runtime
                .receipt_ledger
                .recover(key, Instant::now() + SCENARIO_OPERATION_TIMEOUT)
                .expect("recover seeded direct owner");
            assert_eq!(recovered.kind().diagnostic_name(), expected_kind);
        }

        for (seed_state, expected_reason) in [
            (
                ScenarioSeedReceiptState::TaskBoundNotBegun,
                V5SafeFailureReason::Interrupted,
            ),
            (
                ScenarioSeedReceiptState::TaskBoundBegun,
                V5SafeFailureReason::OutcomeUncertain,
            ),
        ] {
            let state_root = ScenarioStateRoot::new().expect("scenario state root");
            let clock = Arc::new(ScenarioEpochClock::new(SCENARIO_INITIAL_EPOCH_MS));
            let key = fresh_key(&identity, &Map::new()).expect("exact receipt key");
            assert!(seed_receipt_state(
                state_root.path(),
                &identity,
                &clock,
                key.clone(),
                seed_state,
                false,
                None,
            )
            .expect("seed TaskBound owner"));
            let state = DaemonStateDirectory::open(state_root.path(), &identity)
                .expect("open daemon state");
            let config =
                scenario_server_config_with_clock(state_root.path(), &identity, None, &clock);
            let runtime = V5ReceiptRuntime::open_with_epoch_clock(&state, &config, clock.clone())
                .expect("reopen runtime");
            let snapshot = runtime
                .resolve_task(
                    key.reserved_task_id(),
                    Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                )
                .expect("resolve seeded TaskBound owner");
            assert!(matches!(
                snapshot,
                crate::infrastructure::daemon::protocol_v5::V5DaemonTaskSnapshot::Failed {
                    reason,
                    ..
                } if reason == expected_reason
            ));
            assert_eq!(
                runtime
                    .receipt_ledger
                    .recover(key, Instant::now() + SCENARIO_OPERATION_TIMEOUT),
                Err(ReceiptLedgerError::ReceiptNotFound)
            );
        }
    }

    #[test]
    fn checkpoint_projects_real_task_store_and_lifecycle_link_records() {
        let request = json!({
            "clock": "fake",
            "actions": [
                {
                    "action": "seed_receipt",
                    "state": "task_bound_begun",
                    "cancel_requested": false,
                    "staged_terminal": null
                },
                {
                    "action": "seed_task",
                    "status": "working",
                    "cancel_requested": false,
                    "receipt_link": "exact",
                    "identity": "exact",
                    "version": 1
                },
                { "action": "checkpoint", "label": "bound" }
            ]
        });
        let encoded = run_supported_receipt_scenario_for_test(&request.to_string())
            .expect("run TaskBound checkpoint scenario")
            .expect("TaskBound checkpoint scenario is supported");
        let report: Value = serde_json::from_str(&encoded).expect("decode scenario report");
        let snapshot = &report["payload"]["checkpoints"]["bound"];
        assert_eq!(snapshot["receipts"].as_array().map(Vec::len), Some(0));
        assert_eq!(snapshot["tasks"].as_array().map(Vec::len), Some(1));
        assert_eq!(snapshot["tasks"][0]["status"], "working");
        assert_eq!(snapshot["taskLinks"].as_array().map(Vec::len), Some(1));
        assert_eq!(
            snapshot["taskLinks"][0]["lifecycle"]["state"],
            "task_bound_begun"
        );
        assert_eq!(
            snapshot["taskLinks"][0]["lifecycle"]["cancel_requested"],
            false
        );
        assert!(snapshot["taskLinks"][0]["lifecycle"]
            .get("cancelRequested")
            .is_none());
        assert_eq!(snapshot["taskLinkCount"], 1);
        assert_eq!(snapshot["taskLinkReservedCount"], 0);
        assert_eq!(snapshot["cancelAuthority"], "task_store");
        assert_eq!(
            snapshot["invocationIndex"].as_array().map(Vec::len),
            Some(1)
        );
        assert_eq!(
            snapshot["reservedTaskIndex"].as_array().map(Vec::len),
            Some(1)
        );
    }

    #[test]
    fn support_filter_tracks_every_interpreter_path() {
        let action = ReceiptScenarioAction::Submit {
            request: ScenarioRequest::Canonical,
            response_budget_ms: 7_000,
            disconnect: ScenarioDisconnect::AfterSubmitWrite,
            label: "supported-disconnect".to_owned(),
        };
        assert!(action.is_supported());

        let unsupported = ReceiptScenarioAction::ProbeProtocol {
            client: ScenarioProtocolVersion::V5,
            server: ScenarioProtocolVersion::V4,
            message: ScenarioProtocolMessage::Ping,
            label: "unsupported-server".to_owned(),
        };
        assert!(!unsupported.is_supported());

        for action in [
            ReceiptScenarioAction::ProbeProtocol {
                client: ScenarioProtocolVersion::V3,
                server: ScenarioProtocolVersion::V3,
                message: ScenarioProtocolMessage::RecoverReceipt,
                label: "unsupported-v3-request".to_owned(),
            },
            ReceiptScenarioAction::ProbeProtocol {
                client: ScenarioProtocolVersion::V5,
                server: ScenarioProtocolVersion::V5,
                message: ScenarioProtocolMessage::StoredInvocationRecord {
                    schema_version: 1,
                    reason: ScenarioFailureProbeReason::InvocationFailed,
                },
                label: "unsupported-v5-request".to_owned(),
            },
            ReceiptScenarioAction::ProbeProtocol {
                client: ScenarioProtocolVersion::V5,
                server: ScenarioProtocolVersion::V5,
                message: ScenarioProtocolMessage::TaskOutcome,
                label: "unsupported-v5-response-fixture".to_owned(),
            },
            ReceiptScenarioAction::ProbeProtocol {
                client: ScenarioProtocolVersion::V3,
                server: ScenarioProtocolVersion::V3,
                message: ScenarioProtocolMessage::DirectFailureTerminal {
                    reason: ScenarioFailureProbeReason::InvocationFailed,
                },
                label: "unsupported-v3-response-fixture".to_owned(),
            },
            ReceiptScenarioAction::ProbeProtocol {
                client: ScenarioProtocolVersion::V5,
                server: ScenarioProtocolVersion::V5,
                message: ScenarioProtocolMessage::MalformedV5Schema {
                    target: ScenarioStrictSchemaTarget::ResponseUnknownField,
                },
                label: "unsupported-response-schema-mutation".to_owned(),
            },
        ] {
            assert!(action.is_supported());
        }

        assert!(ReceiptScenarioAction::AdvanceMonotonic { millis: 1 }.is_supported());
        assert!(build_v3_probe_request_frame(
            ScenarioProtocolVersion::V3,
            &ScenarioProtocolMessage::RecoverReceipt,
        )
        .is_ok());
        assert!(build_v5_probe_request_frame(
            Path::new("unused-for-rejected-message"),
            &CoreIdentity::production_v5(),
            &ScenarioProtocolMessage::StoredInvocationRecord {
                schema_version: 1,
                reason: ScenarioFailureProbeReason::InvocationFailed,
            },
        )
        .is_ok());
        assert!(build_v5_probe_request_frame(
            Path::new("unused-for-rejected-message"),
            &CoreIdentity::production_v5(),
            &ScenarioProtocolMessage::TaskOutcome,
        )
        .is_ok());
        assert!(build_v3_probe_request_frame(
            ScenarioProtocolVersion::V3,
            &ScenarioProtocolMessage::DirectFailureTerminal {
                reason: ScenarioFailureProbeReason::InvocationFailed,
            },
        )
        .is_ok());
        assert!(build_v5_probe_request_frame(
            Path::new("unused-for-rejected-message"),
            &CoreIdentity::production_v5(),
            &ScenarioProtocolMessage::MalformedV5Schema {
                target: ScenarioStrictSchemaTarget::ResponseUnknownField,
            },
        )
        .is_ok());
    }

    #[test]
    fn receipt_pending_observation_distinguishes_cancel_reservation_phase() {
        let identity = CoreIdentity::production_v5();
        let key = fresh_key(&identity, &Map::new()).expect("construct pending receipt key");
        let cases = [
            (TestV5InvocationPhase::CancelReserved, true),
            (TestV5InvocationPhase::ReservedUnbound, false),
            (TestV5InvocationPhase::ReservedActorBound, true),
            (TestV5InvocationPhase::ReservedBegun, false),
        ];

        let observed = cases
            .into_iter()
            .map(|(phase, cancel_requested)| {
                let response = V5ServerResponse::Invocation {
                    outcome: V5InvocationResponse::ReceiptPending {
                        receipt_key: key.clone(),
                        phase,
                        accepted_epoch_ms: 1_000,
                        original_budget_ms: 6_000,
                        cancel_requested,
                    },
                };
                let projected = response_observation(&response, None)
                    .expect("project protocol-v5 pending response");
                projected["kind"].clone()
            })
            .collect::<Vec<_>>();

        assert_eq!(
            observed,
            vec![
                json!("cancelled"),
                json!("pending"),
                json!("pending"),
                json!("pending"),
            ]
        );
    }

    #[test]
    fn late_cancel_preserves_the_committed_actor_bound_task_terminal() {
        let request = json!({
            "clock": "fake",
            "actions": [
                {
                    "action": "seed_receipt",
                    "state": "task_promised_actor_bound",
                    "cancel_requested": false,
                    "staged_terminal": null
                },
                {
                    "action": "cancel",
                    "key": "exact",
                    "lazy_session": true,
                    "label": "cancel"
                },
                { "action": "restart" },
                { "action": "checkpoint", "label": "reopened" }
            ]
        });

        let encoded = run_supported_receipt_scenario_for_test(&request.to_string())
            .expect("run production receipt-owned Task cancellation")
            .expect("receipt-owned Task cancellation scenario is supported");
        let report: Value = serde_json::from_str(&encoded).expect("decode scenario report");
        let payload = &report["payload"];
        assert_eq!(payload["responses"]["cancel"]["kind"], "task");
        assert_eq!(payload["responses"]["cancel"]["task"]["status"], "failed");
        assert_eq!(
            payload["responses"]["cancel"]["task"]["terminal"]["reason"],
            "interrupted"
        );
        assert_eq!(
            payload["responses"]["cancel"]["task"]["cancelRequested"],
            false
        );
        assert_eq!(
            payload["checkpoints"]["reopened"]["receipts"]
                .as_array()
                .map(Vec::len),
            Some(0)
        );
        assert_eq!(
            payload["checkpoints"]["reopened"]["tasks"][0]["status"],
            "failed"
        );
        assert_eq!(
            payload["checkpoints"]["reopened"]["tasks"][0]["terminal"]["reason"],
            "interrupted"
        );
        assert_eq!(
            payload["checkpoints"]["reopened"]["tasks"][0]["cancelRequested"],
            false
        );
        assert_eq!(
            payload["checkpoints"]["reopened"]["cancelAuthority"],
            "task_store"
        );
    }

    #[test]
    fn protocol_ack_loss_retry_and_duplicate_read_the_same_compact_tombstone() {
        let request = json!({
            "clock": "fake",
            "actions": [
                {
                    "action": "cancel",
                    "key": "exact",
                    "lazy_session": true,
                    "label": "cancel"
                },
                { "action": "checkpoint", "label": "premature-before" },
                {
                    "action": "acknowledge",
                    "key": "exact",
                    "digest": "well_formed_candidate",
                    "disconnect": "never",
                    "label": "premature"
                },
                { "action": "checkpoint", "label": "premature-after" },
                {
                    "action": "submit",
                    "request": "canonical",
                    "response_budget_ms": 6_000,
                    "disconnect": "never",
                    "label": "direct"
                },
                { "action": "checkpoint", "label": "mismatch-before" },
                {
                    "action": "acknowledge",
                    "key": "exact",
                    "digest": "mismatched",
                    "disconnect": "never",
                    "label": "mismatched"
                },
                { "action": "checkpoint", "label": "mismatch-after" },
                {
                    "action": "acknowledge",
                    "key": "exact",
                    "digest": "exact_terminal",
                    "disconnect": "after_tombstone_commit",
                    "label": "lost"
                },
                { "action": "checkpoint", "label": "after-lost" },
                {
                    "action": "acknowledge",
                    "key": "exact",
                    "digest": "exact_terminal",
                    "disconnect": "never",
                    "label": "retry"
                },
                {
                    "action": "submit",
                    "request": "canonical",
                    "response_budget_ms": 6_000,
                    "disconnect": "never",
                    "label": "duplicate"
                },
                { "action": "advance_epoch", "millis": 899_999 },
                { "action": "checkpoint", "label": "before-expiry" },
                { "action": "advance_epoch", "millis": 1 },
                { "action": "checkpoint", "label": "at-expiry" }
            ]
        });

        let encoded = run_supported_receipt_scenario_for_test(&request.to_string())
            .expect("run protocol-v5 acknowledgement scenario")
            .expect("acknowledgement scenario is supported");
        let report: Value = serde_json::from_str(&encoded).expect("decode scenario report");
        let payload = &report["payload"];
        let tombstones = payload["checkpoints"]["after-lost"]["tombstones"]
            .as_array()
            .expect("tombstone inventory");

        assert_eq!(tombstones.len(), 1);
        assert_eq!(
            payload["responses"]["premature"]["error"].as_str(),
            Some("invalid_request")
        );
        assert_eq!(
            payload["checkpoints"]["premature-before"]["receipts"],
            payload["checkpoints"]["premature-after"]["receipts"]
        );
        assert_eq!(
            payload["responses"]["mismatched"]["error"].as_str(),
            Some("invalid_request")
        );
        assert_eq!(
            payload["checkpoints"]["mismatch-before"]["receipts"],
            payload["checkpoints"]["mismatch-after"]["receipts"]
        );
        assert_eq!(
            payload["checkpoints"]["after-lost"]["receiptLiveCount"].as_u64(),
            Some(0)
        );
        assert_eq!(
            payload["responses"]["retry"]["kind"].as_str(),
            Some("acknowledged")
        );
        assert_eq!(
            payload["responses"]["duplicate"]["kind"].as_str(),
            Some("tombstone")
        );
        let first_ack_epoch = tombstones[0]["ackEpochMs"]
            .as_u64()
            .expect("first acknowledgement epoch");
        assert_eq!(
            tombstones[0]["expiresEpochMs"].as_u64(),
            first_ack_epoch.checked_add(ACKNOWLEDGED_TOMBSTONE_TTL_MS)
        );
        assert_eq!(
            payload["responses"]["retry"]["acknowledgement"]["ackEpochMs"].as_u64(),
            Some(first_ack_epoch)
        );
        assert_eq!(
            payload["responses"]["duplicate"]["acknowledgement"]["ackEpochMs"].as_u64(),
            Some(first_ack_epoch)
        );
        assert_eq!(
            payload["checkpoints"]["before-expiry"]["tombstoneCount"].as_u64(),
            Some(1)
        );
        assert_eq!(
            payload["checkpoints"]["at-expiry"]["tombstoneCount"].as_u64(),
            Some(0)
        );
    }

    #[test]
    fn batch_client_failure_stops_and_joins_the_daemon_before_returning() {
        let root = tempfile::tempdir().expect("temporary failed-batch state root");
        let state_root =
            std::fs::canonicalize(root.path()).expect("physical failed-batch state root");
        let identity = CoreIdentity::production_v5();

        let error = exchange_batch(
            &state_root,
            &identity,
            Arc::new(ScenarioEpochClock::new(SCENARIO_INITIAL_EPOCH_MS)),
            Arc::new(V5ReceiptRuntimeTelemetry::new()),
            None,
            vec![ScenarioWireRequest::InjectedClientFailure],
        )
        .expect_err("injected batch failure must reach the facade cleanup path");

        assert_eq!(error, "injected protocol-v5 scenario client failure");
        let state = DaemonStateDirectory::open(&state_root, &identity)
            .expect("reopen failed-batch daemon state");
        assert!(
            state
                .read_v5_endpoint_record()
                .expect("inspect failed-batch endpoint")
                .is_none(),
            "scenario facade returned while its detached daemon was still discoverable"
        );
    }

    #[test]
    fn snapshot_accounting_and_indexes_cover_the_full_store_catalog() {
        let root = tempfile::tempdir().expect("temporary full-catalog store");
        let receipts = std::fs::canonicalize(root.path())
            .expect("physical full-catalog root")
            .join("receipts");
        let store = ReceiptLedgerStore::open(&receipts).expect("open full-catalog store");
        let actor = ReceiptLedgerActor::spawn(store);
        let identity = CoreIdentity::production_v5();
        let arguments = Map::new();
        let cancel_key = fresh_key(&identity, &arguments).expect("cancel receipt key");
        let reserved_key = fresh_key(&identity, &arguments).expect("reserved receipt key");

        actor
            .request_cancel_or_reserve(
                cancel_key.clone(),
                SCENARIO_INITIAL_EPOCH_MS,
                Instant::now() + SCENARIO_OPERATION_TIMEOUT,
            )
            .expect("reserve cancellation receipt");
        actor
            .reserve(
                reserved_key.clone(),
                OriginalCutoffDescriptor::new(SCENARIO_INITIAL_EPOCH_MS, 7_000)
                    .expect("valid cutoff"),
                Instant::now() + SCENARIO_OPERATION_TIMEOUT,
            )
            .expect("reserve ordinary receipt");

        let telemetry = V5ReceiptRuntimeTelemetry::new();
        let snapshot = snapshot_with_actor(
            &actor,
            &ScenarioEpochClock::new(SCENARIO_INITIAL_EPOCH_MS),
            &telemetry,
            0,
            std::slice::from_ref(&cancel_key),
        )
        .expect("snapshot full catalog through actor");

        assert_eq!(
            snapshot["receipts"]
                .as_array()
                .expect("selected receipt rows")
                .len(),
            1,
            "the scenario inventory remains a bounded row-selection input"
        );
        assert_eq!(snapshot["receiptLiveCount"].as_u64(), Some(2));
        let selected_actual_bytes = snapshot["receipts"][0]["encodedBytes"]
            .as_u64()
            .expect("selected receipt encoded bytes");
        assert_eq!(
            snapshot["receiptActualBytes"]
                .as_u64()
                .expect("catalog actual bytes")
                + snapshot["receiptReservedBytes"]
                    .as_u64()
                    .expect("catalog reserved bytes"),
            selected_actual_bytes + MAX_RECEIPT_ENTITLEMENT_BYTES,
            "store accounting must include the unselected reserved receipt"
        );
        let expected_digests = [
            receipt_key_digest(&cancel_key).to_string(),
            receipt_key_digest(&reserved_key).to_string(),
        ];
        for index_name in ["invocationIndex", "reservedTaskIndex"] {
            let index = snapshot[index_name].as_array().expect("catalog index rows");
            assert_eq!(index.len(), 2, "{index_name} must cover the full catalog");
            for expected_digest in &expected_digests {
                assert!(
                    index
                        .iter()
                        .any(|key| { key["keyDigest"].as_str() == Some(expected_digest.as_str()) }),
                    "{index_name} omitted {expected_digest}"
                );
            }
        }
    }

    #[test]
    fn pending_submit_releases_actor_store_before_waiting_for_daemon_exit() {
        let root = tempfile::tempdir().expect("temporary pending-submit store");
        let receipts = std::fs::canonicalize(root.path())
            .expect("physical pending-submit root")
            .join("receipts");
        let store = ReceiptLedgerStore::open(&receipts).expect("open pending-submit store");
        let actor = ReceiptLedgerActor::spawn(store);
        let client = thread::spawn(|| {
            Ok(V5ServerResponse::Error {
                code: V5DaemonErrorCode::ReceiptNotFound,
            })
        });
        let reopen_path = receipts.clone();
        let server = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_millis(150);
            loop {
                match ReceiptLedgerStore::open(&reopen_path) {
                    Ok(reopened) => {
                        drop(reopened);
                        return Ok(());
                    }
                    Err(ReceiptLedgerError::AlreadyOwned) if Instant::now() < deadline => {
                        thread::sleep(Duration::from_millis(1));
                    }
                    Err(ReceiptLedgerError::AlreadyOwned) => {
                        return Err(
                            "pending-submit actor kept the receipt store owned while joining daemon"
                                .to_owned(),
                        );
                    }
                    Err(error) => return Err(format!("reopen pending-submit store: {error}")),
                }
            }
        });
        let pending = PendingSubmit {
            label: "pending".to_owned(),
            accepted_epoch_ms: 1,
            accepted_monotonic_ms: 0,
            response_budget_ms: 7_000,
            actor,
            task_projection: TaskProjectionObservation {
                tasks: Vec::new(),
                task_links: Vec::new(),
                task_link_count: 0,
                task_link_bytes: 0,
                task_link_reserved_count: 0,
                task_link_reserved_bytes: 0,
                task_store_mutations: 0,
                generation: 0,
            },
            task_store_create_attempts: 0,
            response_projected: false,
            client,
            daemon: ScenarioDaemon {
                stop_requested: Arc::new(AtomicBool::new(false)),
                server: Some(server),
            },
        };

        pending
            .finish()
            .expect("actor/store must be released before daemon join");
    }
}
