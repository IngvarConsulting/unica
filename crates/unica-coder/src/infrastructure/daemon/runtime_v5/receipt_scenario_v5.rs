use super::{
    run_daemon_configured_until, V5ReceiptRuntime, V5ReceiptRuntimeEvent,
    V5ReceiptRuntimeEventKind, V5ReceiptRuntimeTelemetry,
};
use crate::application::invocation::normalized_arguments_hash;
use crate::application::invocation_store::EpochMillisClock;
use crate::application::invocation_store::ToolIdentity;
use crate::application::invocation_store_v5::{
    V5SafeFailureReason, V5StoredInvocationRecord, V5StoredInvocationSchemaVersion, V5StoredTask,
};
use crate::application::operation_descriptors::{ExecutionClass, KnownLongReason};
use crate::application::ports::Clock;
use crate::application::receipt_ledger::{
    canonical_v5_terminal, receipt_key_digest, request_scope_hash, task_link_digest,
    AcknowledgedTombstoneReceipt, CoreIdentityDigest, OriginalCutoffDescriptor, ReceiptKey,
    ReceiptKeyDigest, ReceiptLedgerError, ReceiptState, ReceiptTaskProjection,
    ReceiptTerminalOutcome, RequestIdentity, ReservedPhase, TaskLinkIdentity, TerminalDigest,
    V5ToolIdentity, DIRECT_TERMINAL_RETENTION_MS,
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
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
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

pub(super) struct ReceiptScenarioControl {
    barrier: Mutex<CancelConversionBarrier>,
    changed: Condvar,
    drop_ack_response_after_commit: AtomicBool,
    provider: Mutex<Option<ScenarioProviderFixture>>,
    side_effect_markers: AtomicU64,
}

#[derive(Clone)]
struct ScenarioProviderFixture {
    execution_class: ScenarioExecutionClass,
    terminal: ScenarioTerminalFixture,
    cooperative_cancel: bool,
    side_effect_marker: bool,
}

#[derive(Default)]
struct CancelConversionBarrier {
    installed: bool,
    reached: bool,
    released: bool,
}

impl ReceiptScenarioControl {
    fn new() -> Self {
        Self {
            barrier: Mutex::new(CancelConversionBarrier::default()),
            changed: Condvar::new(),
            drop_ack_response_after_commit: AtomicBool::new(false),
            provider: Mutex::new(None),
            side_effect_markers: AtomicU64::new(0),
        }
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

    fn arm_ack_response_disconnect(&self) {
        self.drop_ack_response_after_commit
            .store(true, Ordering::Release);
    }

    pub(super) fn take_ack_response_disconnect(&self) -> bool {
        self.drop_ack_response_after_commit
            .swap(false, Ordering::AcqRel)
    }

    fn install(&self) {
        let mut barrier = self
            .barrier
            .lock()
            .expect("scenario barrier mutex poisoned");
        *barrier = CancelConversionBarrier {
            installed: true,
            reached: false,
            released: false,
        };
    }

    fn is_installed(&self) -> bool {
        self.barrier
            .lock()
            .expect("scenario barrier mutex poisoned")
            .installed
    }

    pub(super) fn pause_after_cancel_conversion(
        &self,
        deadline: Instant,
    ) -> Result<(), ReceiptLedgerError> {
        let mut barrier = self
            .barrier
            .lock()
            .map_err(|_| ReceiptLedgerError::Corrupt("scenario barrier mutex was poisoned"))?;
        if !barrier.installed {
            return Ok(());
        }
        barrier.reached = true;
        self.changed.notify_all();
        while !barrier.released {
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                return Err(ReceiptLedgerError::DeadlineExceeded);
            };
            let (next, timeout) = self
                .changed
                .wait_timeout(barrier, remaining)
                .map_err(|_| ReceiptLedgerError::Corrupt("scenario barrier mutex was poisoned"))?;
            barrier = next;
            if timeout.timed_out() && !barrier.released {
                return Err(ReceiptLedgerError::DeadlineExceeded);
            }
        }
        Ok(())
    }

    fn wait_until_reached(&self, deadline: Instant) -> Result<(), String> {
        let mut barrier = self
            .barrier
            .lock()
            .map_err(|_| "protocol-v5 receipt scenario barrier mutex was poisoned".to_owned())?;
        while !barrier.reached {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .ok_or_else(|| {
                    "protocol-v5 cancel conversion barrier was not reached".to_owned()
                })?;
            let (next, timeout) = self.changed.wait_timeout(barrier, remaining).map_err(|_| {
                "protocol-v5 receipt scenario barrier mutex was poisoned".to_owned()
            })?;
            barrier = next;
            if timeout.timed_out() && !barrier.reached {
                return Err("protocol-v5 cancel conversion barrier was not reached".to_owned());
            }
        }
        Ok(())
    }

    fn release(&self) {
        let mut barrier = self
            .barrier
            .lock()
            .expect("scenario barrier mutex poisoned");
        barrier.released = true;
        self.changed.notify_all();
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
    let telemetry = Arc::new(V5ReceiptRuntimeTelemetry::new());
    let mut known_keys = Vec::new();
    let mut original_submit_cutoff: Option<(u64, u64)> = None;
    let mut pending_submit: Option<PendingSubmit> = None;
    for action in scenario.actions {
        match action {
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
            ReceiptScenarioAction::SeedReceipt {
                state: ScenarioSeedReceiptState::TaskTerminalReceiptBacked,
                cancel_requested,
                staged_terminal: Some(terminal),
            } => {
                seed_receipt_backed_task_terminal(
                    state.path(),
                    &identity,
                    &clock,
                    exact_key.clone(),
                    terminal,
                    cancel_requested,
                )?;
                push_known_key(&mut known_keys, exact_key.clone());
            }
            ReceiptScenarioAction::SeedReceipt { .. } => return Ok(None),
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
                let response = exchange_once(
                    state.path(),
                    &identity,
                    Arc::clone(&clock),
                    Arc::clone(&telemetry),
                    Some(Arc::clone(&control)),
                    |owner| owner.cancel_invocation(wire_key),
                )?;
                if !matches!(response, V5ServerResponse::Error { .. }) && is_exact {
                    push_known_key(&mut known_keys, exact_key.clone());
                }
                report
                    .responses
                    .insert(label, response_observation(&response, None)?);
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
                    if pending_submit.is_some() {
                        return Err(
                            "protocol-v5 receipt scenario already has a pending submit".to_owned()
                        );
                    }
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
                    let response = exchange_once(
                        state.path(),
                        &identity,
                        Arc::clone(&clock),
                        Arc::clone(&telemetry),
                        Some(Arc::clone(&control)),
                        |owner| owner.submit_invocation(invocation),
                    )?;
                    if matches!(disconnect, ScenarioDisconnect::Never) {
                        report.responses.insert(
                            label,
                            response_observation(&response, Some(observed_cutoff))?,
                        );
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
                        let response = exchange_once(
                            state.path(),
                            &identity,
                            Arc::clone(&clock),
                            Arc::clone(&telemetry),
                            Some(Arc::clone(&control)),
                            |owner| {
                                owner.acknowledge_invocation_receipt(
                                    exact_key.clone(),
                                    terminal_digest,
                                )
                            },
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
                let response = exchange_once(
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
                )?;
                report
                    .task_reads
                    .insert(label, task_observation_from_response(response, &exact_key)?);
            }
            ReceiptScenarioAction::AdvanceEpoch { millis } => {
                clock.advance(millis)?;
            }
            ReceiptScenarioAction::AdvanceMonotonic { millis } => {
                clock.advance_monotonic(millis)?;
            }
            ReceiptScenarioAction::Restart => {
                // Every production action in this feature-only driver is a fresh authenticated
                // daemon lifetime. The explicit marker therefore requires no synthetic state.
            }
            ReceiptScenarioAction::Checkpoint { label } => {
                let snapshot = match &pending_submit {
                    Some(pending) => snapshot_with_actor(
                        &pending.actor,
                        &clock,
                        &telemetry,
                        control.side_effect_markers(),
                        &known_keys,
                    )?,
                    None => snapshot_from_state(
                        state.path(),
                        &identity,
                        Arc::clone(&clock),
                        &telemetry,
                        &control,
                        &known_keys,
                    )?,
                };
                report.terminal_publications = telemetry.snapshot().terminal_publications;
                report.checkpoints.insert(label, snapshot);
            }
            ReceiptScenarioAction::Reset => {
                if pending_submit.is_some() {
                    return Err(
                        "protocol-v5 receipt scenario cannot reset a live submit".to_owned()
                    );
                }
                state = ScenarioStateRoot::new()?;
                known_keys.clear();
            }
            ReceiptScenarioAction::FillReceiptPool { state: fill, count } => {
                let mut requests = Vec::with_capacity(count as usize);
                let mut added = Vec::with_capacity(count as usize);
                for index in 0..count {
                    let key = if matches!(fill, ScenarioSeedReceiptState::CancelReserved)
                        && known_keys.is_empty()
                        && index == 0
                    {
                        exact_key.clone()
                    } else {
                        fresh_key_for_workspace(&identity, &arguments, &workspace_hint)?
                    };
                    let request = match fill {
                        ScenarioSeedReceiptState::CancelReserved => {
                            ScenarioWireRequest::Cancel(key.clone())
                        }
                        ScenarioSeedReceiptState::ReservedUnbound => {
                            ScenarioWireRequest::Submit(invocation_for_key(
                                &key,
                                arguments.clone(),
                                workspace_hint.clone(),
                                7_000,
                            )?)
                        }
                        _ => return Ok(None),
                    };
                    requests.push(request);
                    added.push(key);
                }
                let responses = exchange_batch(
                    state.path(),
                    &identity,
                    Arc::clone(&clock),
                    Arc::clone(&telemetry),
                    Some(Arc::clone(&control)),
                    requests,
                )?;
                if let Some(error) = responses
                    .iter()
                    .find(|response| matches!(response, V5ServerResponse::Error { .. }))
                {
                    return Err(format!(
                        "protocol-v5 receipt scenario pool fill was rejected: {error:?}"
                    ));
                }
                for key in added {
                    push_known_key(&mut known_keys, key);
                }
            }
            ReceiptScenarioAction::InstallBarrier { point } => {
                if !matches!(
                    point,
                    ScenarioBarrierPoint::AfterCancelReservationConvertedBeforeTerminal
                ) {
                    return Ok(None);
                }
                control.install();
            }
            ReceiptScenarioAction::WaitForEvent { event } => match event {
                ScenarioEvent::CancelReservationConverted => {
                    control.wait_until_reached(Instant::now() + SCENARIO_OPERATION_TIMEOUT)?;
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
                    telemetry.wait_for_event(
                        V5ReceiptRuntimeEventKind::ReceiptTerminalCommitted,
                        Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                    )?;
                }
            },
            ReceiptScenarioAction::ReleaseBarrier { point } => {
                if !matches!(
                    point,
                    ScenarioBarrierPoint::AfterCancelReservationConvertedBeforeTerminal
                ) {
                    return Ok(None);
                }
                control.release();
                let pending = pending_submit.take().ok_or_else(|| {
                    "protocol-v5 receipt scenario has no submit at the barrier".to_owned()
                })?;
                let (label, accepted_epoch_ms, response_budget_ms, response) = pending.finish()?;
                report.responses.insert(
                    label,
                    response_observation(&response, Some((accepted_epoch_ms, response_budget_ms)))?,
                );
            }
            ReceiptScenarioAction::CompareClientServerIdentity => {
                report.identity = Some(compare_client_server_identity()?);
            }
        }
    }

    if pending_submit.is_some() {
        return Err("protocol-v5 receipt scenario ended with a blocked submit".to_owned());
    }

    report.encode(telemetry.snapshot().events).map(Some)
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

fn invocation_for_key(
    key: &ReceiptKey,
    arguments: Map<String, Value>,
    workspace_hint: String,
    response_budget_ms: u64,
) -> Result<V5InvocationRequest, String> {
    V5InvocationRequest::new(
        key.invocation_id(),
        key.reserved_task_id(),
        key.tool(),
        arguments,
        workspace_hint,
        response_budget_ms,
    )
    .map_err(|error| format!("construct receipt scenario invocation: {error}"))
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
                    "configure_provider"
                        | "cancel"
                        | "submit"
                        | "send_outer_envelope"
                        | "probe_protocol"
                        | "recover"
                        | "acknowledge"
                        | "seed_receipt"
                        | "read_task"
                        | "advance_epoch"
                        | "advance_monotonic"
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
    let config = DaemonServerConfig::new(
        state_root.to_path_buf(),
        identity.clone(),
        SCENARIO_IDLE_GRACE,
    );
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
    scenario_server_config(state_root, identity, control)
        .with_invocation_clock_for_test(invocation_clock)
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
                &ScenarioEpochClock::new(SCENARIO_INITIAL_EPOCH_MS),
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
            jsonl_bytes_from_value(&strict_schema_mutation_value(*target))
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
        | ScenarioProtocolMessage::TaskFailureProjection { .. } => {
            jsonl_frame(&V5ClientRequest::Ping {})
        }
        ScenarioProtocolMessage::StoredInvocationRecord { .. } => {
            Err("protocol-v5 request builder does not support stored-record probes".to_owned())
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
    let reservation = match actor
        .reserve(
            key.clone(),
            OriginalCutoffDescriptor::new(SCENARIO_INITIAL_EPOCH_MS, 7_000)
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
            SCENARIO_INITIAL_EPOCH_MS,
            terminal,
            deadline,
        )
        .map_err(|error| format!("publish protocol-v5 probe terminal: {error}"))?;
    drop(actor);
    Ok((key, terminal_digest))
}

fn build_v3_probe_request_frame(
    client: ScenarioProtocolVersion,
    message: &ScenarioProtocolMessage,
) -> Result<Vec<u8>, String> {
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
        ScenarioProtocolMessage::DirectFailureTerminal { .. }
        | ScenarioProtocolMessage::StoredInvocationRecord { .. }
            if client == ScenarioProtocolVersion::V3 =>
        {
            jsonl_frame(&protocol_v3::ClientRequest::Ping {})
        }
        ScenarioProtocolMessage::RecoverReceipt
        | ScenarioProtocolMessage::AcknowledgeReceipt
        | ScenarioProtocolMessage::CancelReceipt
            if client != ScenarioProtocolVersion::V3 =>
        {
            jsonl_frame(&protocol_v3::ClientRequest::Ping {})
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
            Err("protocol-v3 request builder does not support this probe".to_owned())
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
    server: ScenarioProtocolVersion,
    message: &ScenarioProtocolMessage,
    default_identity: &CoreIdentity,
) -> Result<CoreIdentity, String> {
    match (server, message) {
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

fn strict_schema_mutation_value(target: ScenarioStrictSchemaTarget) -> Value {
    match target {
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
        ScenarioStrictSchemaTarget::ResponseCrossVariantField => json!({
            "kind": "pong",
            "snapshot": strict_task_snapshot_value(),
        }),
        ScenarioStrictSchemaTarget::TerminalUnknownField => json!({
            "status": "failed",
            "reason": "invocation_failed",
            "unexpected": true,
        }),
        ScenarioStrictSchemaTarget::TerminalMissingRequiredField => json!({
            "status": "failed",
        }),
        ScenarioStrictSchemaTarget::TerminalCrossVariantField => json!({
            "status": "failed",
            "reason": "invocation_failed",
            "result": {"ok": false, "summary": "semantic-invalid"},
        }),
        ScenarioStrictSchemaTarget::TaskSnapshotUnknownField => {
            let mut value = strict_task_snapshot_value();
            value["unexpected"] = true.into();
            value
        }
        ScenarioStrictSchemaTarget::TaskSnapshotMissingRequiredField => {
            let mut value = strict_task_snapshot_value();
            value
                .as_object_mut()
                .expect("strict Task snapshot fixture is an object")
                .remove("taskId");
            value
        }
        ScenarioStrictSchemaTarget::TaskSnapshotCrossVariantField => {
            let mut value = strict_task_snapshot_value();
            value["reason"] = "invocation_failed".into();
            value
        }
        ScenarioStrictSchemaTarget::StoredRecordUnknownTopLevel => {
            let mut value = strict_stored_record_value();
            value["unexpected"] = true.into();
            value
        }
        ScenarioStrictSchemaTarget::StoredRecordUnknownTaskField => {
            let mut value = strict_stored_record_value();
            value["task"]["unexpected"] = true.into();
            value
        }
        ScenarioStrictSchemaTarget::StoredRecordMissingRequiredField => {
            let mut value = strict_stored_record_value();
            value
                .as_object_mut()
                .expect("strict stored record fixture is an object")
                .remove("schemaVersion");
            value
        }
        ScenarioStrictSchemaTarget::StoredRecordCrossVariantField => {
            let mut value = strict_stored_record_value();
            value["task"]["reason"] = "invocation_failed".into();
            value
        }
        ScenarioStrictSchemaTarget::TransferCertificateUnknownField => {
            let mut value = strict_transfer_certificate_value();
            value["unexpected"] = true.into();
            value
        }
        ScenarioStrictSchemaTarget::TransferCertificateMissingRequiredField => {
            let mut value = strict_transfer_certificate_value();
            value
                .as_object_mut()
                .expect("strict transfer certificate fixture is an object")
                .remove("terminalCodecVersion");
            value
        }
        ScenarioStrictSchemaTarget::TransferCertificateCrossVariantField => {
            let mut value = strict_transfer_certificate_value();
            value["taskPublicationCases"][0]["cancelRequested"] = false.into();
            value
        }
    }
}

fn strict_task_snapshot_value() -> Value {
    json!({
        "status": "queued",
        "taskId": "11111111-1111-4111-8111-111111111111",
        "invocationId": "22222222-2222-4222-8222-222222222222",
        "receiptKeyDigest": "0".repeat(64),
        "createdAtEpochMs": 1,
        "updatedAtEpochMs": 1,
        "ttlMs": 3_600_000,
        "pollIntervalMs": 100,
        "version": 1,
        "cancelRequested": false,
    })
}

fn strict_stored_record_value() -> Value {
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
        "ttlMs": 3_600_000,
        "pollIntervalMs": 100,
        "version": 1,
        "cancelRequested": false,
        "task": {"status": "queued"},
    })
}

fn strict_transfer_certificate_value() -> Value {
    let exact_provisional = |status: &str, cancel_requested: bool| {
        json!({
            "kind": "exact_provisional",
            "status": status,
            "version": u64::MAX,
            "cancelRequested": cancel_requested,
            "finalTaskRecordMaxBytes": MAX_V5_RESPONSE_LINE_BYTES,
            "taskResponseFrameMaxBytes": MAX_V5_RESPONSE_LINE_BYTES,
        })
    };
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
            {
                "kind": "absent",
                "finalTaskRecordMaxBytes": MAX_V5_RESPONSE_LINE_BYTES,
                "taskResponseFrameMaxBytes": MAX_V5_RESPONSE_LINE_BYTES,
            },
            exact_provisional("queued", false),
            exact_provisional("queued", true),
            exact_provisional("working", false),
            exact_provisional("working", true),
        ],
        "capacityFallbackCases": [{
            "source": "link_capacity",
            "receiptBackedRecordMaxBytes": MAX_V5_RESPONSE_LINE_BYTES,
            "taskResponseFrameMaxBytes": MAX_V5_RESPONSE_LINE_BYTES,
        }],
    })
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

enum ScenarioWireRequest {
    Cancel(ReceiptKey),
    Submit(V5InvocationRequest),
    #[cfg(test)]
    InjectedClientFailure,
}

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
            let mut owner = V5DaemonProcessOwner::connect_or_spawn_for_protocol_test(
                state_root,
                identity.clone(),
                std::path::PathBuf::from("unused-existing-v5-scenario-endpoint"),
                SCENARIO_IDLE_GRACE,
            )?;
            let response = match request {
                ScenarioWireRequest::Cancel(key) => owner.cancel_invocation(key),
                ScenarioWireRequest::Submit(invocation) => owner.submit_invocation(invocation),
                #[cfg(test)]
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
    response_budget_ms: u64,
    actor: ReceiptLedgerActor,
    client: thread::JoinHandle<Result<V5ServerResponse, String>>,
    daemon: ScenarioDaemon,
}

impl PendingSubmit {
    fn finish(self) -> Result<(String, u64, u64, V5ServerResponse), String> {
        let Self {
            label,
            accepted_epoch_ms,
            response_budget_ms,
            actor,
            client,
            daemon,
        } = self;
        let response = client
            .join()
            .map_err(|_| "protocol-v5 receipt scenario submit client panicked".to_owned())
            .and_then(|response| response);
        drop(actor);
        let cleanup = daemon.stop_and_join("protocol-v5 receipt scenario blocked daemon panicked");
        let response = finish_with_daemon_cleanup(response, cleanup)?;
        Ok((label, accepted_epoch_ms, response_budget_ms, response))
    }
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
    wait_for_endpoint(state_root, identity)?;
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
        response_budget_ms,
        actor,
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
    let config = scenario_server_config_with_clock(state_root, identity, None, &clock);
    let runtime = V5ReceiptRuntime::open_with_epoch_clock(&state, &config, clock.clone())?;
    let snapshot = snapshot_with_actor(
        &runtime.receipt_ledger,
        &clock,
        telemetry,
        control.side_effect_markers(),
        keys,
    );
    drop(runtime);
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
        "restartRequested": false,
        "daemonRunning": runtime.daemon_running,
        "actorLeases": runtime.actor_leases,
        "sideEffectMarkers": side_effect_markers,
        "taskStoreCreateAttempts": 0,
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
                    cancel_requested,
                },
        } => Ok(json!({
            "kind": match phase {
                V5InvocationPhase::CancelReserved => "cancelled",
                V5InvocationPhase::ReservedUnbound
                | V5InvocationPhase::ReservedActorBound
                | V5InvocationPhase::ReservedBegun => "pending",
            },
            "cancelRequested": cancel_requested,
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
            let (accepted_epoch_ms, original_budget_ms) = submit_cutoff
                .map(|(accepted, budget)| (Some(accepted), Some(budget)))
                .unwrap_or((None, None));
            Ok(json!({
                "kind": if matches!(receipt.terminal(), ReceiptTerminalOutcome::Cancelled) {
                    "cancelled"
                } else {
                    "direct"
                },
                "error": null,
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
) -> Result<Value, String> {
    let V5ServerResponse::Task { snapshot } = response else {
        return Err(format!(
            "Task read expected a protocol-v5 Task response, received {response:?}"
        ));
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
    Ok(json!({
        "taskId": task_id,
        "invocationId": invocation_id,
        "receiptKey": receipt_key_observation(receipt_key),
        "status": status,
        "projectionSource": "receipt_ledger",
        "workspaceIdentityHash": null,
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
                "actorBindings": [],
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
    ConfigureProvider {
        execution_class: ScenarioExecutionClass,
        terminal: ScenarioTerminalFixture,
        cooperative_cancel: bool,
        side_effect_marker: bool,
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
            Self::ConfigureProvider {
                execution_class,
                terminal,
                ..
            } => {
                matches!(execution_class, ScenarioExecutionClass::Direct)
                    && matches!(terminal, ScenarioTerminalFixture::Success { .. })
            }
            Self::Cancel {
                key, lazy_session, ..
            } => {
                *lazy_session
                    && matches!(
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
                ..
            } => {
                matches!(state, ScenarioSeedReceiptState::TaskTerminalReceiptBacked)
                    && staged_terminal.as_ref().is_some_and(|terminal| {
                        matches!(terminal, ScenarioTerminalFixture::Success { .. })
                    })
            }
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
            Self::FillReceiptPool { state, .. } => matches!(
                state,
                ScenarioSeedReceiptState::CancelReserved
                    | ScenarioSeedReceiptState::ReservedUnbound
            ),
        }
    }
}

fn protocol_probe_is_supported(
    client: ScenarioProtocolVersion,
    server: ScenarioProtocolVersion,
    message: &ScenarioProtocolMessage,
) -> bool {
    match server {
        ScenarioProtocolVersion::V4 => false,
        ScenarioProtocolVersion::V5 => !matches!(
            message,
            ScenarioProtocolMessage::StoredInvocationRecord { .. }
        ),
        ScenarioProtocolVersion::V3 => match message {
            ScenarioProtocolMessage::Ping
            | ScenarioProtocolMessage::Release
            | ScenarioProtocolMessage::SubmitWithCoreIdentity { .. }
            | ScenarioProtocolMessage::GetTask
            | ScenarioProtocolMessage::WaitTask
            | ScenarioProtocolMessage::CancelTask => true,
            ScenarioProtocolMessage::DirectFailureTerminal { .. }
            | ScenarioProtocolMessage::StoredInvocationRecord { .. } => {
                client == ScenarioProtocolVersion::V3
            }
            ScenarioProtocolMessage::RecoverReceipt
            | ScenarioProtocolMessage::AcknowledgeReceipt
            | ScenarioProtocolMessage::CancelReceipt => client != ScenarioProtocolVersion::V3,
            _ => false,
        },
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

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ScenarioBarrierPoint {
    AfterCancelReservationConvertedBeforeTerminal,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ScenarioEvent {
    CancelReservationConverted,
    ReceiptTerminalCommitted,
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
    fn support_filter_rejects_actions_without_an_interpreter_path() {
        for disconnect in [
            ScenarioDisconnect::AfterSubmitWrite,
            ScenarioDisconnect::AfterTerminalCommit,
        ] {
            let action = ReceiptScenarioAction::Submit {
                request: ScenarioRequest::Canonical,
                response_budget_ms: 7_000,
                disconnect,
                label: "unsupported-disconnect".to_owned(),
            };
            assert!(action.is_supported());
        }

        for action in [
            ReceiptScenarioAction::ProbeProtocol {
                client: ScenarioProtocolVersion::V5,
                server: ScenarioProtocolVersion::V4,
                message: ScenarioProtocolMessage::Ping,
                label: "unsupported-server".to_owned(),
            },
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
        ] {
            assert!(!action.is_supported());
        }

        assert!(ReceiptScenarioAction::AdvanceMonotonic { millis: 1 }.is_supported());
        assert!(build_v3_probe_request_frame(
            ScenarioProtocolVersion::V3,
            &ScenarioProtocolMessage::RecoverReceipt,
        )
        .is_err());
        assert!(build_v5_probe_request_frame(
            Path::new("unused-for-rejected-message"),
            &CoreIdentity::production_v5(),
            &ScenarioProtocolMessage::StoredInvocationRecord {
                schema_version: 1,
                reason: ScenarioFailureProbeReason::InvocationFailed,
            },
        )
        .is_err());
    }

    #[test]
    fn receipt_pending_observation_distinguishes_phase_and_projects_cancel_request() {
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
                json!({
                    "kind": projected["kind"],
                    "cancelRequested": projected["cancelRequested"],
                })
            })
            .collect::<Vec<_>>();

        assert_eq!(
            observed,
            vec![
                json!({ "kind": "cancelled", "cancelRequested": true }),
                json!({ "kind": "pending", "cancelRequested": false }),
                json!({ "kind": "pending", "cancelRequested": true }),
                json!({ "kind": "pending", "cancelRequested": false }),
            ]
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
            response_budget_ms: 7_000,
            actor,
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
