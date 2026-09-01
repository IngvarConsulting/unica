use super::{
    run_daemon_configured_until, V5ReceiptRuntime, V5ReceiptRuntimeEvent,
    V5ReceiptRuntimeEventKind, V5ReceiptRuntimeTelemetry,
};
use crate::application::invocation::normalized_arguments_hash;
use crate::application::invocation_store::EpochMillisClock;
use crate::application::receipt_ledger::{
    canonical_v5_terminal, receipt_key_digest, request_scope_hash, task_link_digest,
    CoreIdentityDigest, ReceiptKey, ReceiptKeyDigest, ReceiptLedgerError, ReceiptState,
    ReceiptTerminalOutcome, RequestIdentity, ReservedPhase, TaskLinkIdentity, TerminalDigest,
    V5ToolIdentity, DIRECT_TERMINAL_RETENTION_MS,
};
use crate::application::receipt_ledger_actor::ReceiptLedgerActor;
use crate::domain::invocation::{InvocationId, NormalizedArgumentsHash, SafeIdentityHash, TaskId};
use crate::infrastructure::daemon::client_v5::V5DaemonProcessOwner;
use crate::infrastructure::daemon::identity::{CoreIdentity, DaemonStateDirectory};
use crate::infrastructure::daemon::protocol_v5::{
    decode_v5_request_frame, V5ClientRequest, V5DaemonErrorCode, V5InvocationPhase,
    V5InvocationRequest, V5InvocationResponse, V5ServerResponse,
};
use crate::infrastructure::daemon::server::DaemonServerConfig;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::path::Path;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const SCENARIO_INITIAL_EPOCH_MS: u64 = 1;
const SCENARIO_IDLE_GRACE: Duration = Duration::from_secs(2);
const SCENARIO_OPERATION_TIMEOUT: Duration = Duration::from_secs(2);

pub(super) struct ReceiptScenarioControl {
    barrier: Mutex<CancelConversionBarrier>,
    changed: Condvar,
    drop_ack_response_after_commit: AtomicBool,
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
        }
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
    let Ok(scenario) = serde_json::from_str::<ReceiptScenario>(request) else {
        return Ok(None);
    };
    if !matches!(scenario.clock, ScenarioClock::Fake)
        || scenario.actions.iter().any(|action| !action.is_supported())
    {
        return Ok(None);
    }

    let mut state = ScenarioStateRoot::new()?;
    let identity = CoreIdentity::production_v5();
    let clock = Arc::new(ScenarioEpochClock::new(SCENARIO_INITIAL_EPOCH_MS));
    let arguments = Map::new();
    let invocation_id = InvocationId::new();
    let reserved_task_id = TaskId::new();
    let exact_key = ReceiptKey::new(
        invocation_id,
        reserved_task_id,
        RequestIdentity::new(
            identity.digest().clone(),
            V5ToolIdentity::View,
            normalized_arguments_hash(&arguments),
            request_scope_hash("workspace-a")
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
                request_scope_hash("workspace-a").map_err(|error| {
                    format!("construct mismatched receipt scenario request scope: {error}")
                })?,
            ),
        )
    };

    let mut report = ScenarioReportBuilder::default();
    let control = Arc::new(ReceiptScenarioControl::new());
    let telemetry = Arc::new(V5ReceiptRuntimeTelemetry::new());
    let mut known_keys = Vec::new();
    let mut pending_submit: Option<PendingSubmit> = None;
    for action in scenario.actions {
        match action {
            ReceiptScenarioAction::Cancel { key, label, .. } => {
                let is_exact = matches!(&key, ScenarioKey::Exact);
                let wire_key = match key {
                    ScenarioKey::Exact => exact_key.clone(),
                    ScenarioKey::Unknown => fresh_key(&identity, &arguments)?,
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
                label,
                ..
            } => {
                let invocation = V5InvocationRequest::new(
                    invocation_id,
                    reserved_task_id,
                    V5ToolIdentity::View,
                    arguments.clone(),
                    "workspace-a".to_owned(),
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
                    let response = exchange_once(
                        state.path(),
                        &identity,
                        Arc::clone(&clock),
                        Arc::clone(&telemetry),
                        |owner| owner.submit_invocation(invocation),
                    )?;
                    report.responses.insert(
                        label,
                        response_observation(
                            &response,
                            Some((clock.now_epoch_millis(), response_budget_ms)),
                        )?,
                    );
                }
            }
            ReceiptScenarioAction::Recover { key, label } => {
                let ScenarioKey::Exact = key else {
                    return Ok(None);
                };
                let response = exchange_once(
                    state.path(),
                    &identity,
                    Arc::clone(&clock),
                    Arc::clone(&telemetry),
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
                report
                    .responses
                    .insert(label, response_observation(&response, None)?);
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
                    ScenarioDigest::TaskTerminal => return Ok(None),
                };
                match disconnect {
                    ScenarioAckDisconnect::Never => {
                        let response = exchange_once(
                            state.path(),
                            &identity,
                            Arc::clone(&clock),
                            Arc::clone(&telemetry),
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
            ReceiptScenarioAction::AdvanceEpoch { millis } => {
                clock.advance(millis)?;
            }
            ReceiptScenarioAction::Restart => {
                // Every production action in this feature-only driver is a fresh authenticated
                // daemon lifetime. The explicit marker therefore requires no synthetic state.
            }
            ReceiptScenarioAction::Checkpoint { label } => {
                let snapshot = match &pending_submit {
                    Some(pending) => {
                        snapshot_with_actor(&pending.actor, &clock, &telemetry, &known_keys)?
                    }
                    None => snapshot_from_state(
                        state.path(),
                        &identity,
                        Arc::clone(&clock),
                        &telemetry,
                        &known_keys,
                    )?,
                };
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
                        fresh_key(&identity, &arguments)?
                    };
                    let request = match fill {
                        ScenarioSeedReceiptState::CancelReserved => {
                            ScenarioWireRequest::Cancel(key.clone())
                        }
                        ScenarioSeedReceiptState::ReservedUnbound => ScenarioWireRequest::Submit(
                            invocation_for_key(&key, arguments.clone(), 7_000)?,
                        ),
                    };
                    requests.push(request);
                    added.push(key);
                }
                let responses = exchange_batch(
                    state.path(),
                    &identity,
                    Arc::clone(&clock),
                    Arc::clone(&telemetry),
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

fn fresh_key(
    identity: &CoreIdentity,
    arguments: &Map<String, Value>,
) -> Result<ReceiptKey, String> {
    Ok(ReceiptKey::new(
        InvocationId::new(),
        TaskId::new(),
        RequestIdentity::new(
            identity.digest().clone(),
            V5ToolIdentity::View,
            normalized_arguments_hash(arguments),
            request_scope_hash("workspace-a")
                .map_err(|error| format!("construct receipt scenario request scope: {error}"))?,
        ),
    ))
}

fn invocation_for_key(
    key: &ReceiptKey,
    arguments: Map<String, Value>,
    response_budget_ms: u64,
) -> Result<V5InvocationRequest, String> {
    V5InvocationRequest::new(
        key.invocation_id(),
        key.reserved_task_id(),
        key.tool(),
        arguments,
        "workspace-a".to_owned(),
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
                    "cancel"
                        | "submit"
                        | "recover"
                        | "acknowledge"
                        | "advance_epoch"
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

fn exchange_once(
    state_root: &Path,
    identity: &CoreIdentity,
    clock: Arc<ScenarioEpochClock>,
    telemetry: Arc<V5ReceiptRuntimeTelemetry>,
    exchange: impl FnOnce(&mut V5DaemonProcessOwner) -> Result<V5ServerResponse, String>,
) -> Result<V5ServerResponse, String> {
    let config = DaemonServerConfig::new(
        state_root.to_path_buf(),
        identity.clone(),
        SCENARIO_IDLE_GRACE,
    );
    let daemon = ScenarioDaemon::spawn(config, move |runtime| {
        let mut runtime = runtime.with_shared_telemetry(telemetry);
        runtime.epoch_clock = clock;
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

fn exchange_ack_and_expect_disconnect(
    state_root: &Path,
    identity: &CoreIdentity,
    clock: Arc<ScenarioEpochClock>,
    control: Arc<ReceiptScenarioControl>,
    telemetry: Arc<V5ReceiptRuntimeTelemetry>,
    key: ReceiptKey,
    terminal_digest: TerminalDigest,
) -> Result<(), String> {
    let config = DaemonServerConfig::new(
        state_root.to_path_buf(),
        identity.clone(),
        SCENARIO_IDLE_GRACE,
    );
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
    requests: Vec<ScenarioWireRequest>,
) -> Result<Vec<V5ServerResponse>, String> {
    let config = DaemonServerConfig::new(
        state_root.to_path_buf(),
        identity.clone(),
        SCENARIO_IDLE_GRACE,
    );
    let daemon = ScenarioDaemon::spawn(config, move |runtime| {
        let mut runtime = runtime.with_shared_telemetry(telemetry);
        runtime.epoch_clock = clock;
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
    let config = DaemonServerConfig::new(
        state_root.to_path_buf(),
        identity.clone(),
        SCENARIO_IDLE_GRACE,
    );
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
    keys: &[ReceiptKey],
) -> Result<Value, String> {
    let state = DaemonStateDirectory::open(state_root, identity)?;
    let config = DaemonServerConfig::new(
        state_root.to_path_buf(),
        identity.clone(),
        SCENARIO_IDLE_GRACE,
    );
    let runtime = V5ReceiptRuntime::open_with_epoch_clock(&state, &config, clock.clone())?;
    let snapshot = snapshot_with_actor(&runtime.receipt_ledger, &clock, telemetry, keys);
    drop(runtime);
    snapshot
}

fn snapshot_with_actor(
    actor: &ReceiptLedgerActor,
    clock: &ScenarioEpochClock,
    telemetry: &V5ReceiptRuntimeTelemetry,
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
        "sideEffectMarkers": 0,
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
    let response = exchange_once(state_root, identity, clock, telemetry, |owner| {
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
    identity: Option<Value>,
}

impl ScenarioReportBuilder {
    fn encode(self, events: Vec<V5ReceiptRuntimeEvent>) -> Result<String, String> {
        serde_json::to_string(&json!({
            "kind": "observed",
            "payload": {
                "checkpoints": self.checkpoints,
                "responses": self.responses,
                "taskReads": {},
                "events": events,
                "gateEvents": [],
                "operationEvents": [],
                "actorBindings": [],
                "actorAuthorizations": [],
                "taskPublicationCapacity": [],
                "taskStoreCapacityInvariantViolations": [],
                "stagedTerminalPreparations": [],
                "terminalPublications": [],
                "protocol": [],
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
}

impl ScenarioEpochClock {
    fn new(epoch_ms: u64) -> Self {
        Self {
            epoch_ms: AtomicU64::new(epoch_ms),
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
}

impl EpochMillisClock for ScenarioEpochClock {
    fn now_epoch_millis(&self) -> u64 {
        self.epoch_ms.load(Ordering::SeqCst)
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
    AdvanceEpoch {
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
                matches!(request, ScenarioRequest::Canonical)
                    && matches!(disconnect, ScenarioDisconnect::Never)
            }
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
            Self::AdvanceEpoch { .. }
            | Self::Restart
            | Self::Checkpoint { .. }
            | Self::Reset
            | Self::FillReceiptPool { .. }
            | Self::InstallBarrier { .. }
            | Self::WaitForEvent { .. }
            | Self::ReleaseBarrier { .. }
            | Self::CompareClientServerIdentity => true,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ScenarioRequest {
    Canonical,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ScenarioDisconnect {
    Never,
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
