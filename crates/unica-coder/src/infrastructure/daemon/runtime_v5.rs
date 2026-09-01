use super::identity::{CoreIdentity, DaemonStateDirectory, ReceiptAuthorityLock};
use super::protocol_v5::{
    decode_v5_request_frame, read_bounded_v5_request_frame_before, DecodedV5Request,
    V5AcknowledgedReceipt, V5ClientRequest, V5ClientRequestKind, V5DaemonErrorCode,
    V5EndpointRecord, V5HandshakeServerResponse, V5InvocationPhase, V5InvocationRequest,
    V5InvocationResponse, V5ProbeServerResponse, V5RequestFrameError, V5ServerResponse,
    DAEMON_PROTOCOL_VERSION, MAX_V5_RESPONSE_LINE_BYTES,
};
#[cfg(feature = "receipt-ledger-test-support")]
use super::protocol_v5::{decode_v5_server_response, read_bounded_v5_probe_response_frame};
use super::server::{
    CanonicalInvocationService, DaemonServerConfig, V5ActorBoundCanonicalInvocation,
    V5CanonicalInvocationRuntime, V5CanonicalPrepareError,
};
use crate::application::invocation::RESPONSE_SERIALIZATION_MARGIN;
use crate::application::invocation_store::{EpochMillisClock, ToolIdentity};
use crate::application::invocation_store_v5::{
    InvocationStoreV5, TaskStoreRecoveryCatalog, V5SafeFailureReason,
};
use crate::application::invocation_v5::{
    classify_cancel_reserved_expiry_outcome, classify_recovered_receipt,
    decide_cancel_reserved_submit, decide_cancel_resolution, CancelInvocationDecision,
    CancelReservedExpiryDecision, CancelReservedRecoveryDecision, CancelReservedSubmitDecision,
};
use crate::application::ports::Clock;
#[cfg(feature = "receipt-ledger-test-support")]
use crate::application::receipt_ledger::CommittedDirectPublication;
use crate::application::receipt_ledger::{
    OriginalCutoffDescriptor, PreparedWireFrame, ReceiptKey, ReceiptLedgerError, ReceiptState,
    ReserveOutcome, ReservedPhase, TerminalDigest,
};
use crate::application::receipt_ledger_actor::ReceiptLedgerActor;
use crate::infrastructure::platform::filesystem::RetainedDirectoryCapability;
use crate::infrastructure::receipt_ledger::ReceiptLedgerStore;
#[cfg(feature = "receipt-ledger-test-support")]
use crate::infrastructure::receipt_ledger::StableReceiptLedgerObservation;
#[cfg(feature = "receipt-ledger-test-support")]
use crate::infrastructure::receipt_ledger_test_evidence::ProductionMissingTransitionEvidence;
use crate::infrastructure::task_store::SystemEpochMillisClock;
use crate::infrastructure::task_store_v5::FileInvocationStoreV5;
use serde::Serialize;
#[cfg(feature = "receipt-ledger-test-support")]
use serde_json::json;
use serde_json::Value;
#[cfg(feature = "receipt-ledger-test-support")]
use sha2::{Digest, Sha256};
use std::io::{self, BufReader, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
#[cfg(feature = "receipt-ledger-test-support")]
use std::sync::mpsc::{sync_channel, SyncSender};
use std::sync::Arc;
#[cfg(feature = "receipt-ledger-test-support")]
use std::sync::{Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(feature = "receipt-ledger-test-support")]
mod receipt_scenario_v5;
#[cfg(feature = "receipt-ledger-test-support")]
pub(crate) use receipt_scenario_v5::run_supported_receipt_scenario_for_test;

const ACCEPT_POLL_INTERVAL: Duration = Duration::from_millis(10);
const AUTHORITY_ACQUIRE_TIMEOUT: Duration = Duration::from_secs(2);
const HANDSHAKE_READ_TIMEOUT: Duration = Duration::from_secs(2);
const SESSION_READ_TIMEOUT: Duration = Duration::from_secs(2);
const OWNER_RESPONSE_WRITE_TIMEOUT: Duration = Duration::from_secs(10);

#[cfg(feature = "receipt-ledger-test-support")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum V5ReceiptRuntimeEventKind {
    V5ReceiptRuntimeEntered,
    CanonicalV13ServiceEntered,
    CancelReservationConverted,
    ReceiptTerminalCommitted,
    AcknowledgementCommitted,
}

#[cfg(feature = "receipt-ledger-test-support")]
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct V5ReceiptRuntimeEvent {
    sequence: u64,
    monotonic_ms: u64,
    epoch_ms: u64,
    event: V5ReceiptRuntimeEventKind,
}

#[cfg(feature = "receipt-ledger-test-support")]
#[derive(Debug, Clone, Copy, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct V5ReceiptRuntimeCallbackCounts {
    validation: u64,
    admission: u64,
    prepare: u64,
    execute: u64,
}

#[cfg(feature = "receipt-ledger-test-support")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum V5ReceiptRuntimeListenerState {
    NotPublished,
    Listening,
    Closed,
}

#[cfg(feature = "receipt-ledger-test-support")]
struct V5ReceiptRuntimeTelemetryState {
    next_sequence: u64,
    events: Vec<V5ReceiptRuntimeEvent>,
    callbacks: V5ReceiptRuntimeCallbackCounts,
    listener: V5ReceiptRuntimeListenerState,
    active_listeners: u64,
    daemon_running: bool,
    actor_leases: u64,
    terminal_publications: Vec<Value>,
    next_preflight_sequence: u64,
}

#[cfg(feature = "receipt-ledger-test-support")]
#[derive(Clone)]
struct V5ReceiptRuntimeTelemetrySnapshot {
    events: Vec<V5ReceiptRuntimeEvent>,
    callbacks: V5ReceiptRuntimeCallbackCounts,
    listener: V5ReceiptRuntimeListenerState,
    daemon_running: bool,
    actor_leases: u64,
    terminal_publications: Vec<Value>,
}

#[cfg(feature = "receipt-ledger-test-support")]
struct V5ReceiptRuntimeTelemetry {
    started_at: Instant,
    state: Mutex<V5ReceiptRuntimeTelemetryState>,
    changed: Condvar,
}

#[cfg(feature = "receipt-ledger-test-support")]
impl V5ReceiptRuntimeTelemetry {
    fn new() -> Self {
        Self {
            started_at: Instant::now(),
            state: Mutex::new(V5ReceiptRuntimeTelemetryState {
                next_sequence: 1,
                events: Vec::new(),
                callbacks: V5ReceiptRuntimeCallbackCounts::default(),
                listener: V5ReceiptRuntimeListenerState::NotPublished,
                active_listeners: 0,
                daemon_running: false,
                actor_leases: 0,
                terminal_publications: Vec::new(),
                next_preflight_sequence: 1,
            }),
            changed: Condvar::new(),
        }
    }

    fn lock_state(&self) -> MutexGuard<'_, V5ReceiptRuntimeTelemetryState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn record_event(&self, event: V5ReceiptRuntimeEventKind, epoch_ms: u64) {
        let monotonic_ms = u64::try_from(self.started_at.elapsed().as_millis()).unwrap_or(u64::MAX);
        let mut state = self.lock_state();
        let sequence = state.next_sequence;
        state.next_sequence = state
            .next_sequence
            .checked_add(1)
            .expect("protocol-v5 runtime telemetry sequence exhausted u64");
        state.events.push(V5ReceiptRuntimeEvent {
            sequence,
            monotonic_ms,
            epoch_ms,
            event,
        });
        self.changed.notify_all();
    }

    fn record_prepare(&self) {
        let mut state = self.lock_state();
        state.callbacks.prepare = state.callbacks.prepare.saturating_add(1);
        self.changed.notify_all();
    }

    fn record_execute(&self) {
        let mut state = self.lock_state();
        state.callbacks.execute = state.callbacks.execute.saturating_add(1);
        self.changed.notify_all();
    }

    fn snapshot(&self) -> V5ReceiptRuntimeTelemetrySnapshot {
        let state = self.lock_state();
        V5ReceiptRuntimeTelemetrySnapshot {
            events: state.events.clone(),
            callbacks: state.callbacks,
            listener: state.listener,
            daemon_running: state.daemon_running,
            actor_leases: state.actor_leases,
            terminal_publications: state.terminal_publications.clone(),
        }
    }

    fn record_direct_publication(
        &self,
        publication: &CommittedDirectPublication,
        response_kind: &'static str,
        origin: &'static str,
    ) {
        let mut state = self.lock_state();
        let receipt_key = receipt_key_observation_value(publication.receipt().key());
        let response_prepared_sequence = state.next_preflight_sequence;
        let response_write_sequence = response_prepared_sequence
            .checked_add(1)
            .expect("protocol-v5 preflight sequence exhausted u64");
        if let Some(existing) = state
            .terminal_publications
            .iter_mut()
            .find(|value| value.get("receiptKey") == Some(&receipt_key))
        {
            if let Some(frames) = existing
                .get_mut("responseFrames")
                .and_then(Value::as_array_mut)
            {
                frames.push(json!({
                    "responseKind": response_kind,
                    "origin": origin,
                    "responseJsonl": artifact_value(
                        publication.wire_frame().jsonl(),
                        publication.wire_frame().encoded_bytes(),
                        publication.wire_frame().sha256(),
                    ),
                    "preparedSequence": response_prepared_sequence,
                    "writeSequence": response_write_sequence,
                }));
            }
            state.next_preflight_sequence = response_write_sequence
                .checked_add(1)
                .expect("protocol-v5 preflight sequence exhausted u64");
            return;
        }

        let Some(record) = publication.prepared_record() else {
            return;
        };
        let terminal_payload_sequence = response_prepared_sequence;
        let receipt_record_sequence = terminal_payload_sequence
            .checked_add(1)
            .expect("protocol-v5 preflight sequence exhausted u64");
        let receipt_commit_sequence = receipt_record_sequence
            .checked_add(1)
            .expect("protocol-v5 preflight sequence exhausted u64");
        state.next_preflight_sequence = receipt_commit_sequence
            .checked_add(1)
            .expect("protocol-v5 preflight sequence exhausted u64");
        state.terminal_publications.push(json!({
            "receiptKey": receipt_key,
            "terminal": terminal_observation_value(
                publication.receipt().terminal(),
                publication.receipt().terminal_epoch_ms(),
            ),
            "commit": {
                "owner": "direct_receipt_ledger",
                "receipt": {
                    "terminalPayload": artifact_value(
                        record.terminal().payload(),
                        u64::try_from(record.terminal().payload().len())
                            .expect("terminal payload length fits u64"),
                        &crate::application::receipt_ledger::ArtifactSha256::from_sha256(
                            Sha256::digest(record.terminal().payload()).into(),
                        ),
                    ),
                    "receiptRecord": artifact_value(
                        record.bytes(),
                        record.encoded_bytes(),
                        record.sha256(),
                    ),
                    "candidateResult": candidate_result_value(record.terminal()),
                    "terminalPayloadPreparedSequence": terminal_payload_sequence,
                    "receiptRecordPreparedSequence": receipt_record_sequence,
                    "receiptCommitSequence": receipt_commit_sequence,
                    "receiptExpectedVersion": record.binding().expected_version().get(),
                }
            },
            "responseFrames": [{
                "responseKind": response_kind,
                "origin": origin,
                "responseJsonl": artifact_value(
                    publication.wire_frame().jsonl(),
                    publication.wire_frame().encoded_bytes(),
                    publication.wire_frame().sha256(),
                ),
                "preparedSequence": response_prepared_sequence,
                "writeSequence": Value::Null,
            }],
        }));
    }

    fn wait_for_event(
        &self,
        event: V5ReceiptRuntimeEventKind,
        deadline: Instant,
    ) -> Result<(), String> {
        let mut state = self.lock_state();
        while !state.events.iter().any(|record| record.event == event) {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .ok_or_else(|| format!("protocol-v5 runtime event {event:?} was not observed"))?;
            let (next, timeout) = self
                .changed
                .wait_timeout(state, remaining)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state = next;
            if timeout.timed_out() && !state.events.iter().any(|record| record.event == event) {
                return Err(format!(
                    "protocol-v5 runtime event {event:?} was not observed"
                ));
            }
        }
        Ok(())
    }

    fn listener_lease(self: &Arc<Self>) -> V5ReceiptRuntimeListenerLease {
        let mut state = self.lock_state();
        state.active_listeners = state
            .active_listeners
            .checked_add(1)
            .expect("protocol-v5 runtime listener telemetry exhausted u64");
        state.listener = V5ReceiptRuntimeListenerState::Listening;
        state.daemon_running = true;
        self.changed.notify_all();
        V5ReceiptRuntimeListenerLease {
            telemetry: Arc::clone(self),
        }
    }

    fn actor_lease(self: &Arc<Self>) -> V5ReceiptRuntimeActorLease {
        let mut state = self.lock_state();
        state.actor_leases = state
            .actor_leases
            .checked_add(1)
            .expect("protocol-v5 runtime actor-lease telemetry exhausted u64");
        self.changed.notify_all();
        V5ReceiptRuntimeActorLease {
            telemetry: Arc::clone(self),
        }
    }
}

#[cfg(feature = "receipt-ledger-test-support")]
fn receipt_key_observation_value(key: &ReceiptKey) -> Value {
    json!({
        "invocationId": key.invocation_id(),
        "reservedTaskId": key.reserved_task_id(),
        "coreIdentityDigest": key.core_identity_digest(),
        "tool": key.tool(),
        "normalizedArgumentsHash": key.normalized_arguments_hash(),
        "requestScopeHash": key.request_scope_hash(),
        "keyDigest": crate::application::receipt_ledger::receipt_key_digest(key),
    })
}

#[cfg(feature = "receipt-ledger-test-support")]
fn artifact_value(
    bytes: &[u8],
    encoded_bytes: u64,
    sha256: &crate::application::receipt_ledger::ArtifactSha256,
) -> Value {
    json!({
        "rawHex": lower_hex(bytes),
        "encodedBytes": encoded_bytes,
        "sha256": sha256.to_string(),
    })
}

#[cfg(feature = "receipt-ledger-test-support")]
fn artifact_from_bytes(bytes: &[u8]) -> Value {
    artifact_value(
        bytes,
        u64::try_from(bytes.len()).expect("artifact length fits u64"),
        &crate::application::receipt_ledger::ArtifactSha256::from_sha256(
            Sha256::digest(bytes).into(),
        ),
    )
}

#[cfg(feature = "receipt-ledger-test-support")]
fn terminal_observation_value(
    terminal: &crate::application::receipt_ledger::V5CanonicalTerminal,
    terminal_epoch_ms: u64,
) -> Value {
    let mut value = serde_json::to_value(terminal.outcome())
        .expect("protocol-v5 terminal outcome must serialize");
    let object = value
        .as_object_mut()
        .expect("protocol-v5 terminal outcome must be an object");
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
    value
}

#[cfg(feature = "receipt-ledger-test-support")]
fn candidate_result_value(
    terminal: &crate::application::receipt_ledger::V5CanonicalTerminal,
) -> Option<Value> {
    match terminal.outcome() {
        crate::application::receipt_ledger::ReceiptTerminalOutcome::Completed { result } => {
            let bytes = serde_json::to_vec(result).expect("DomainResult must serialize");
            Some(artifact_from_bytes(&bytes))
        }
        crate::application::receipt_ledger::ReceiptTerminalOutcome::Failed { .. }
        | crate::application::receipt_ledger::ReceiptTerminalOutcome::Cancelled => None,
    }
}

#[cfg(feature = "receipt-ledger-test-support")]
fn lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

#[cfg(feature = "receipt-ledger-test-support")]
struct V5ReceiptRuntimeListenerLease {
    telemetry: Arc<V5ReceiptRuntimeTelemetry>,
}

#[cfg(feature = "receipt-ledger-test-support")]
impl Drop for V5ReceiptRuntimeListenerLease {
    fn drop(&mut self) {
        let mut state = self.telemetry.lock_state();
        state.active_listeners = state
            .active_listeners
            .checked_sub(1)
            .expect("protocol-v5 runtime listener telemetry lease released only once");
        if state.active_listeners == 0 {
            state.listener = V5ReceiptRuntimeListenerState::Closed;
            state.daemon_running = false;
        }
        self.telemetry.changed.notify_all();
    }
}

#[cfg(feature = "receipt-ledger-test-support")]
struct V5ReceiptRuntimeActorLease {
    telemetry: Arc<V5ReceiptRuntimeTelemetry>,
}

#[cfg(feature = "receipt-ledger-test-support")]
impl Drop for V5ReceiptRuntimeActorLease {
    fn drop(&mut self) {
        let mut state = self.telemetry.lock_state();
        state.actor_leases = state
            .actor_leases
            .checked_sub(1)
            .expect("protocol-v5 runtime actor telemetry lease released only once");
        self.telemetry.changed.notify_all();
    }
}

struct V5ReceiptRuntime {
    core_identity: CoreIdentity,
    // On healthy shutdown Rust drops fields in declaration order: the actor
    // joins and releases its store before named authority is released. A
    // fail-stopped runtime is retained until process death instead.
    receipt_ledger: ReceiptLedgerActor,
    _stable_authority: ReceiptAuthorityLock,
    epoch_clock: Arc<dyn EpochMillisClock>,
    #[cfg(feature = "receipt-ledger-test-support")]
    initial_receipt_observation: StableReceiptLedgerObservation,
    #[cfg_attr(not(feature = "receipt-ledger-test-support"), allow(dead_code))]
    invocation_executor: V5InvocationExecutor,
    #[cfg_attr(not(feature = "receipt-ledger-test-support"), allow(dead_code))]
    task_projection: V5TaskProjection,
    #[cfg(feature = "receipt-ledger-test-support")]
    evidence_capture: Option<SyncSender<ProductionMissingTransitionEvidence>>,
    #[cfg(feature = "receipt-ledger-test-support")]
    scenario_control: Option<Arc<receipt_scenario_v5::ReceiptScenarioControl>>,
    #[cfg(feature = "receipt-ledger-test-support")]
    telemetry: Arc<V5ReceiptRuntimeTelemetry>,
}

struct V5TaskProjection {
    #[cfg_attr(not(feature = "receipt-ledger-test-support"), allow(dead_code))]
    task_store_root: RetainedDirectoryCapability,
    #[allow(dead_code)]
    task_store: Arc<dyn InvocationStoreV5>,
    recovery: TaskStoreRecoveryCatalog,
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
            epoch_clock,
            crate::domain::code_intelligence::ProviderDeadline::new(deadline),
        )
        .map_err(|error| format!("open inspect-only protocol-v5 task store: {error}"))?;
        Ok(Self {
            task_store_root,
            task_store: Arc::new(store),
            recovery,
        })
    }

    #[cfg(feature = "receipt-ledger-test-support")]
    fn validate_named_identity(&self) -> Result<(), String> {
        self.task_store_root
            .validate_named_identity()
            .map_err(|error| format!("validate protocol-v5 task projection root: {error}"))
    }

    #[cfg(feature = "receipt-ledger-test-support")]
    fn observe_missing_seed_writer(
        &self,
        observation: StableReceiptLedgerObservation,
    ) -> V5TaskProjectionReachability {
        V5TaskProjectionReachability {
            _task_store_root: self.task_store_root.clone(),
            observation,
        }
    }
}

#[cfg(feature = "receipt-ledger-test-support")]
pub(in crate::infrastructure) struct V5TaskProjectionReachability {
    _task_store_root: RetainedDirectoryCapability,
    observation: StableReceiptLedgerObservation,
}

#[cfg(feature = "receipt-ledger-test-support")]
impl V5TaskProjectionReachability {
    pub(in crate::infrastructure) const fn observation(&self) -> &StableReceiptLedgerObservation {
        &self.observation
    }
}

struct V5InvocationExecutor {
    invocation_runtime: V5CanonicalInvocationRuntime,
}

impl V5InvocationExecutor {
    fn new(invocation_service: Arc<dyn CanonicalInvocationService>, clock: Arc<dyn Clock>) -> Self {
        Self {
            invocation_runtime: V5CanonicalInvocationRuntime::new(invocation_service, clock),
        }
    }

    fn bind(
        &self,
        invocation: V5InvocationRequest,
    ) -> Result<V5ActorBoundCanonicalInvocation, V5CanonicalPrepareError> {
        let tool = match invocation.tool() {
            crate::application::receipt_ledger::V5ToolIdentity::View => ToolIdentity::View,
            crate::application::receipt_ledger::V5ToolIdentity::Apply => ToolIdentity::Apply,
            crate::application::receipt_ledger::V5ToolIdentity::Find => ToolIdentity::Find,
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
                    "bad_value",
                    error,
                ),
            ))
        })?;
        self.invocation_runtime.bind(request)
    }

    #[cfg(feature = "receipt-ledger-test-support")]
    fn observe_missing_writer(
        &self,
        action: V5ExecutorReachabilityAction,
        observation: StableReceiptLedgerObservation,
    ) -> V5ExecutorReachability {
        V5ExecutorReachability {
            action,
            observation,
        }
    }
}

#[cfg(feature = "receipt-ledger-test-support")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::infrastructure) enum V5ExecutorReachabilityAction {
    SubmitInvocation,
    RunDirectLoad,
    RunLazyCancelStorm,
}

#[cfg(feature = "receipt-ledger-test-support")]
impl V5ExecutorReachabilityAction {
    pub(in crate::infrastructure) const fn wire_name(self) -> &'static str {
        match self {
            Self::SubmitInvocation => "submit",
            Self::RunDirectLoad => "run_direct_load",
            Self::RunLazyCancelStorm => "run_lazy_cancel_storm",
        }
    }
}

#[cfg(feature = "receipt-ledger-test-support")]
pub(in crate::infrastructure) struct V5ExecutorReachability {
    action: V5ExecutorReachabilityAction,
    observation: StableReceiptLedgerObservation,
}

#[cfg(feature = "receipt-ledger-test-support")]
impl V5ExecutorReachability {
    pub(in crate::infrastructure) const fn action(&self) -> V5ExecutorReachabilityAction {
        self.action
    }

    pub(in crate::infrastructure) const fn observation(&self) -> &StableReceiptLedgerObservation {
        &self.observation
    }
}

impl V5ReceiptRuntime {
    fn open(state: &DaemonStateDirectory, config: &DaemonServerConfig) -> Result<Self, String> {
        Self::open_with_epoch_clock(state, config, Arc::new(SystemEpochMillisClock))
    }

    fn open_with_epoch_clock(
        state: &DaemonStateDirectory,
        config: &DaemonServerConfig,
        epoch_clock: Arc<dyn EpochMillisClock>,
    ) -> Result<Self, String> {
        let stable_authority = state.acquire_receipt_authority(AUTHORITY_ACQUIRE_TIMEOUT)?;
        let receipts = state.create_private_retained_subdirectory("receipts")?;
        let receipt_ledger = ReceiptLedgerStore::open_retained_directory(receipts)
            .map_err(|error| format!("open protocol-v5 receipt ledger: {error}"))?;
        receipt_ledger
            .generation()
            .map_err(|error| format!("read protocol-v5 receipt generation: {error}"))?;
        #[cfg(feature = "receipt-ledger-test-support")]
        let initial_receipt_observation = receipt_ledger
            .observe_stable_generation()
            .map_err(|error| format!("observe initial protocol-v5 receipt generation: {error}"))?;
        let receipt_ledger = ReceiptLedgerActor::spawn(receipt_ledger);
        let task_projection = V5TaskProjection::open(
            state,
            Arc::clone(&epoch_clock),
            Instant::now() + AUTHORITY_ACQUIRE_TIMEOUT,
        )?;
        let _task_recovery_entries = task_projection.recovery.entries().len();
        Ok(Self {
            core_identity: config.core_identity.clone(),
            _stable_authority: stable_authority,
            receipt_ledger,
            epoch_clock,
            #[cfg(feature = "receipt-ledger-test-support")]
            initial_receipt_observation,
            invocation_executor: V5InvocationExecutor::new(
                config.invocation_service_for_v5(),
                config.invocation_clock_for_v5(),
            ),
            task_projection,
            #[cfg(feature = "receipt-ledger-test-support")]
            evidence_capture: None,
            #[cfg(feature = "receipt-ledger-test-support")]
            scenario_control: None,
            #[cfg(feature = "receipt-ledger-test-support")]
            telemetry: Arc::new(V5ReceiptRuntimeTelemetry::new()),
        })
    }

    #[cfg(feature = "receipt-ledger-test-support")]
    fn with_shared_telemetry(mut self, telemetry: Arc<V5ReceiptRuntimeTelemetry>) -> Self {
        self.telemetry = telemetry;
        self
    }

    fn ensure_named_authority(&self) -> Result<(), String> {
        self.ensure_named_authority_before(Instant::now() + AUTHORITY_ACQUIRE_TIMEOUT)
    }

    fn ensure_named_authority_before(&self, deadline: Instant) -> Result<(), String> {
        self.receipt_ledger
            .generation(deadline)
            .map(|_| ())
            .map_err(|error| format!("validate protocol-v5 receipt authority: {error}"))
    }

    fn restart_required(&self) -> bool {
        self.receipt_ledger.restart_required()
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
                self.reply_for_existing_state(rejection.into_state(), deadline)
            }
        }
    }

    fn submit_invocation(
        &self,
        decoded: DecodedV5Request,
        epoch_ms: u64,
        deadline: Instant,
    ) -> Result<V5RuntimeReply, ReceiptLedgerError> {
        let strict = decoded
            .into_strict_submit(&self.core_identity)
            .map_err(|_| ReceiptLedgerError::InvocationIdentityMismatch)?;
        let invocation = strict.invocation().clone();
        let (key, response_budget_ms) = strict.into_parts();
        let cutoff = OriginalCutoffDescriptor::new(epoch_ms, response_budget_ms)
            .map_err(|_| ReceiptLedgerError::TimestampOverflow)?;
        let outcome = self.receipt_ledger.reserve(key, cutoff, deadline)?;
        let decision = decide_cancel_reserved_submit(outcome).map_err(|_| {
            ReceiptLedgerError::Corrupt("canonical cancelled terminal could not be constructed")
        })?;
        match decision {
            CancelReservedSubmitDecision::ExecuteReserved(reservation) => {
                self.execute_reserved_invocation(reservation, invocation, epoch_ms, deadline)
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

    fn execute_reserved_invocation(
        &self,
        reservation: crate::application::receipt_ledger::ReservedReceipt,
        invocation: V5InvocationRequest,
        epoch_ms: u64,
        deadline: Instant,
    ) -> Result<V5RuntimeReply, ReceiptLedgerError> {
        #[cfg(feature = "receipt-ledger-test-support")]
        self.telemetry
            .record_event(V5ReceiptRuntimeEventKind::V5ReceiptRuntimeEntered, epoch_ms);
        #[cfg(feature = "receipt-ledger-test-support")]
        self.telemetry.record_event(
            V5ReceiptRuntimeEventKind::CanonicalV13ServiceEntered,
            epoch_ms,
        );
        let actor_bound = match self.invocation_executor.bind(invocation) {
            Ok(actor_bound) => actor_bound,
            Err(error) => {
                let outcome = match error {
                    V5CanonicalPrepareError::Rejected(result) => {
                        crate::application::receipt_ledger::ReceiptTerminalOutcome::Completed {
                            result,
                        }
                    }
                    V5CanonicalPrepareError::WorkspaceCapacity => {
                        crate::application::receipt_ledger::ReceiptTerminalOutcome::Failed {
                            reason: V5SafeFailureReason::WorkspaceCapacity,
                        }
                    }
                    V5CanonicalPrepareError::WorkspaceRegistryFailed => {
                        crate::application::receipt_ledger::ReceiptTerminalOutcome::Failed {
                            reason: V5SafeFailureReason::WorkspaceRegistryFailed,
                        }
                    }
                };
                let terminal = crate::application::receipt_ledger::canonical_v5_terminal(&outcome)
                    .map_err(|_| ReceiptLedgerError::Corrupt("canonical v5 terminal failed"))?;
                return self.publish_direct_terminal(reservation, epoch_ms, terminal, deadline);
            }
        };
        let bound = self.receipt_ledger.bind_reserved_actor(
            reservation.key().clone(),
            reservation.record_version(),
            actor_bound.workspace_identity_hash().clone(),
            deadline,
        )?;
        let begun = self.receipt_ledger.mark_reserved_begun(
            bound.key().clone(),
            bound.record_version(),
            deadline,
        )?;
        #[cfg(feature = "receipt-ledger-test-support")]
        self.telemetry.record_prepare();
        let prepared = match actor_bound.prepare() {
            Ok(prepared) => prepared,
            Err(result) => {
                let terminal = crate::application::receipt_ledger::canonical_v5_terminal(
                    &crate::application::receipt_ledger::ReceiptTerminalOutcome::Completed {
                        result,
                    },
                )
                .map_err(|_| ReceiptLedgerError::Corrupt("canonical v5 terminal failed"))?;
                return self.publish_direct_terminal(begun, epoch_ms, terminal, deadline);
            }
        };
        if matches!(
            prepared.execution_class(),
            crate::application::operation_descriptors::ExecutionClass::KnownLong(_)
        ) {
            let terminal = crate::application::receipt_ledger::canonical_v5_terminal(
                &crate::application::receipt_ledger::ReceiptTerminalOutcome::Failed {
                    reason: V5SafeFailureReason::InvocationFailed,
                },
            )
            .map_err(|_| ReceiptLedgerError::Corrupt("canonical v5 terminal failed"))?;
            return self.publish_direct_terminal(begun, epoch_ms, terminal, deadline);
        }
        #[cfg(feature = "receipt-ledger-test-support")]
        self.telemetry.record_execute();
        let outcome = match prepared.execute() {
            Ok(result) => crate::application::receipt_ledger::ReceiptTerminalOutcome::Completed {
                result: Box::new(result),
            },
            Err(_) => crate::application::receipt_ledger::ReceiptTerminalOutcome::Failed {
                reason: V5SafeFailureReason::InvocationFailed,
            },
        };
        let terminal = crate::application::receipt_ledger::canonical_v5_terminal(&outcome)
            .map_err(|_| ReceiptLedgerError::Corrupt("canonical v5 terminal failed"))?;
        self.publish_direct_terminal(begun, epoch_ms, terminal, deadline)
    }

    fn publish_direct_terminal(
        &self,
        reservation: crate::application::receipt_ledger::ReservedReceipt,
        epoch_ms: u64,
        terminal: crate::application::receipt_ledger::V5CanonicalTerminal,
        deadline: Instant,
    ) -> Result<V5RuntimeReply, ReceiptLedgerError> {
        let publication = self.receipt_ledger.publish_direct_terminal(
            reservation.key().clone(),
            reservation.record_version(),
            epoch_ms,
            terminal,
            deadline,
        )?;
        #[cfg(feature = "receipt-ledger-test-support")]
        self.telemetry.record_event(
            V5ReceiptRuntimeEventKind::ReceiptTerminalCommitted,
            epoch_ms,
        );
        #[cfg(feature = "receipt-ledger-test-support")]
        self.telemetry
            .record_direct_publication(&publication, "direct", "immediate_publication");
        Ok(V5RuntimeReply::Prepared(publication.into_parts().1))
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
                #[cfg(feature = "receipt-ledger-test-support")]
                self.telemetry.record_event(
                    V5ReceiptRuntimeEventKind::CancelReservationConverted,
                    epoch_ms,
                );
                #[cfg(feature = "receipt-ledger-test-support")]
                if let Some(control) = &self.scenario_control {
                    control.pause_after_cancel_conversion(deadline)?;
                }
                let publication = self.receipt_ledger.publish_direct_terminal(
                    intent.reservation().key().clone(),
                    intent.reservation().record_version(),
                    epoch_ms,
                    intent.terminal().clone(),
                    deadline,
                )?;
                #[cfg(feature = "receipt-ledger-test-support")]
                self.telemetry.record_event(
                    V5ReceiptRuntimeEventKind::ReceiptTerminalCommitted,
                    epoch_ms,
                );
                #[cfg(feature = "receipt-ledger-test-support")]
                self.telemetry.record_direct_publication(
                    &publication,
                    "cancelled",
                    "immediate_publication",
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
        let state = self.receipt_ledger.recover_at(key, epoch_ms, deadline)?;
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
        #[cfg(feature = "receipt-ledger-test-support")]
        self.telemetry.record_event(
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
        let state = self.receipt_ledger.resolve_task(task_id, deadline)?;
        let ReceiptState::TaskTerminalReceiptBacked(receipt) = state else {
            return Err(ReceiptLedgerError::ReceiptRowPresentUnsupported);
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
        #[cfg(not(feature = "receipt-ledger-test-support"))]
        let _ = (origin, response_kind);
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
                #[cfg(feature = "receipt-ledger-test-support")]
                self.telemetry
                    .record_direct_publication(&publication, response_kind, origin);
                Ok(V5RuntimeReply::Prepared(publication.into_parts().1))
            }
            ReceiptState::AcknowledgedTombstone(receipt) => {
                Ok(V5RuntimeReply::Json(V5ServerResponse::Invocation {
                    outcome: V5InvocationResponse::Acknowledged {
                        acknowledgement: V5AcknowledgedReceipt::from_receipt(&receipt),
                    },
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

    #[cfg(feature = "receipt-ledger-test-support")]
    fn observe_missing_executor_writer(
        &self,
        action: V5ExecutorReachabilityAction,
    ) -> Result<V5ExecutorReachability, String> {
        self.ensure_named_authority()?;
        let observation = self.initial_receipt_observation.clone();
        Ok(self
            .invocation_executor
            .observe_missing_writer(action, observation))
    }

    #[cfg(feature = "receipt-ledger-test-support")]
    fn observe_missing_task_projection_writer(
        &self,
    ) -> Result<V5TaskProjectionReachability, String> {
        self.ensure_named_authority()?;
        self.task_projection.validate_named_identity()?;
        let observation = self.initial_receipt_observation.clone();
        Ok(self
            .task_projection
            .observe_missing_seed_writer(observation))
    }

    #[cfg(feature = "receipt-ledger-test-support")]
    fn with_evidence_capture(
        mut self,
        capture: SyncSender<ProductionMissingTransitionEvidence>,
    ) -> Self {
        self.evidence_capture = Some(capture);
        self
    }

    #[cfg(feature = "receipt-ledger-test-support")]
    fn capture_protocol_transition_after_frame(
        &self,
        decoded: &DecodedV5Request,
    ) -> Result<(), String> {
        let Some(capture) = &self.evidence_capture else {
            return Ok(());
        };
        self.ensure_named_authority()?;
        let evidence = ProductionMissingTransitionEvidence::protocol_behavior_unavailable(decoded);
        capture
            .try_send(evidence)
            .map_err(|_| "capture protocol-v5 reachability evidence".to_string())
    }

    #[cfg(feature = "receipt-ledger-test-support")]
    fn capture_missing_submit_writer_after_reserve(
        &self,
        reply: &V5RuntimeReply,
        deadline: Instant,
    ) -> Result<(), String> {
        let Some(capture) = &self.evidence_capture else {
            return Ok(());
        };
        if !matches!(
            reply,
            V5RuntimeReply::Json(V5ServerResponse::Invocation {
                outcome: V5InvocationResponse::ReceiptPending {
                    phase: V5InvocationPhase::ReservedUnbound,
                    ..
                }
            })
        ) {
            return Ok(());
        }
        self.ensure_named_authority_before(deadline)?;
        let token = self.invocation_executor.observe_missing_writer(
            V5ExecutorReachabilityAction::SubmitInvocation,
            self.initial_receipt_observation.clone(),
        );
        let evidence = ProductionMissingTransitionEvidence::writer_path_unavailable(token);
        capture
            .try_send(evidence)
            .map_err(|_| "capture protocol-v5 submit writer evidence".to_string())
    }
}

enum V5RuntimeReply {
    Json(V5ServerResponse),
    Prepared(PreparedWireFrame),
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
    let runtime = configure_runtime(V5ReceiptRuntime::open(&state, &config)?);
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
    #[cfg(feature = "receipt-ledger-test-support")]
    let listener_telemetry = runtime.telemetry.listener_lease();
    let mut idle_since = Instant::now();
    let mut restart_requested = false;

    loop {
        if stop_requested() {
            break;
        }
        if let Err(error) = runtime.ensure_named_authority() {
            if runtime.restart_required() {
                restart_requested = true;
                break;
            }
            let _ = state.remove_v5_endpoint_if_owned(&published);
            return Err(error);
        }
        match listener.accept() {
            Ok((stream, address)) if address.ip().is_loopback() => {
                if let Err(error) = runtime.ensure_named_authority() {
                    drop(stream);
                    if runtime.restart_required() {
                        restart_requested = true;
                        break;
                    }
                    let _ = state.remove_v5_endpoint_if_owned(&published);
                    return Err(error);
                }
                idle_since = Instant::now();
                // This CR0 slice handles one bounded authenticated session synchronously,
                // including the partial submit/cancel/recover ReceiptLedger path. Full
                // owner/session concurrency and execution traffic remain later work.
                let _ = handle_probe_connection(stream, &record, &runtime);
                if runtime.restart_required() {
                    restart_requested = true;
                    break;
                }
            }
            Ok((_stream, _)) => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                if idle_since.elapsed() >= config.idle_grace {
                    break;
                }
                thread::sleep(ACCEPT_POLL_INTERVAL);
            }
            Err(error) => {
                let _ = state.remove_v5_endpoint_if_owned(&published);
                return Err(daemon_io_error("accept protocol-v5 connection", error));
            }
        }
    }

    drop(listener);
    #[cfg(feature = "receipt-ledger-test-support")]
    drop(listener_telemetry);
    if restart_requested {
        // INV.APP.DAEMON-STORE-FAIL-STOP: keep both the PID-bound endpoint and
        // receipt authority alive until process death. A detached worker may
        // still be inside an uninterruptible adapter or syscall.
        std::mem::forget(runtime);
        return Ok(());
    }
    state.remove_v5_endpoint_if_owned(&published)?;
    Ok(())
}

fn handle_probe_connection(
    mut stream: TcpStream,
    record: &V5EndpointRecord,
    runtime: &V5ReceiptRuntime,
) -> Result<(), String> {
    let handshake_deadline = Instant::now() + HANDSHAKE_READ_TIMEOUT;
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
    let Some((protocol_version, token, core_identity, _owner_lease)) =
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
    write_runtime_json_line_before(
        &mut stream,
        runtime,
        &V5HandshakeServerResponse::ready(record),
        handshake_deadline,
    )?;
    #[cfg(feature = "receipt-ledger-test-support")]
    let _actor_telemetry_lease = runtime.telemetry.actor_lease();
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
                return Ok(());
            }
            Err(V5RequestFrameError::Read(error)) if error.kind() == io::ErrorKind::InvalidData => {
                write_runtime_probe_error_before(
                    &mut stream,
                    runtime,
                    V5DaemonErrorCode::InvalidRequest,
                    session_read_deadline,
                )?;
                return Ok(());
            }
            Err(V5RequestFrameError::Read(_)) => return Ok(()),
        };
    let deadlines = v5_request_deadlines(&decoded, request_received_at)?;
    #[cfg(feature = "receipt-ledger-test-support")]
    runtime.telemetry.record_event(
        V5ReceiptRuntimeEventKind::V5ReceiptRuntimeEntered,
        runtime.epoch_ms(),
    );
    runtime.ensure_named_authority_before(deadlines.operation)?;
    let kind = decoded.request().kind();
    match kind {
        V5ClientRequestKind::Ping => {
            #[cfg(feature = "receipt-ledger-test-support")]
            runtime.capture_protocol_transition_after_frame(&decoded)?;
            write_runtime_json_line_before(
                &mut stream,
                runtime,
                &V5ProbeServerResponse::Pong {},
                deadlines.response,
            )
        }
        V5ClientRequestKind::SubmitInvocation => {
            let epoch_ms = runtime.epoch_ms();
            match runtime.submit_invocation(decoded, epoch_ms, deadlines.operation) {
                Ok(reply) => {
                    #[cfg(feature = "receipt-ledger-test-support")]
                    runtime
                        .capture_missing_submit_writer_after_reserve(&reply, deadlines.operation)?;
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
            let V5ClientRequest::CancelInvocation { receipt_key } = decoded.into_request() else {
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
            let V5ClientRequest::RecoverInvocationReceipt { receipt_key } = decoded.into_request()
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
                    #[cfg(feature = "receipt-ledger-test-support")]
                    if runtime
                        .scenario_control
                        .as_ref()
                        .is_some_and(|control| control.take_ack_response_disconnect())
                    {
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
        V5ClientRequestKind::GetTask
        | V5ClientRequestKind::WaitTask
        | V5ClientRequestKind::CancelTask => {
            let task_id = match decoded.into_request() {
                V5ClientRequest::GetTask { task_id }
                | V5ClientRequest::WaitTask { task_id, .. }
                | V5ClientRequest::CancelTask { task_id } => task_id,
                _ => unreachable!("request kind and decoded Task variant diverged"),
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
    }
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
        // The actor has already latched fail-stop, so asking it for another
        // generation check would turn the required closed response into EOF.
        // The runtime still owns the authenticated stream, PID endpoint,
        // listener and process-scoped authority at this point. The response
        // deadline is the operation cutoff plus its one transport margin.
        write_json_line_before(stream, &response, deadline)
    } else {
        write_runtime_json_line_before(stream, runtime, &response, deadline)
    }
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
        ReceiptLedgerError::CommitUncertain { .. } => V5DaemonErrorCode::DurabilityUncertain,
        ReceiptLedgerError::DeadlineExceeded => V5DaemonErrorCode::Overloaded,
        ReceiptLedgerError::AlreadyOwned => V5DaemonErrorCode::DuplicateLease,
        ReceiptLedgerError::RecordTooLarge
        | ReceiptLedgerError::TimestampOverflow
        | ReceiptLedgerError::ReceiptVersionMismatch { .. }
        | ReceiptLedgerError::ReceiptMutationSequenceMismatch { .. }
        | ReceiptLedgerError::TerminalMismatch
        | ReceiptLedgerError::ReceiptDigestCollision
        | ReceiptLedgerError::StoreUnavailable
        | ReceiptLedgerError::ConcurrentGenerationChange { .. }
        | ReceiptLedgerError::Corrupt(_)
        | ReceiptLedgerError::ReceiptRowPresentUnsupported
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
    let mut bytes = serde_json::to_vec(value)
        .map_err(|_| "protocol-v5 response could not be serialized".to_string())?;
    bytes.push(b'\n');
    if bytes.len() > MAX_V5_RESPONSE_LINE_BYTES {
        return Err("protocol-v5 response exceeds the byte limit".to_string());
    }
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

#[cfg(feature = "receipt-ledger-test-support")]
pub(crate) fn run_protocol_ping_reachability_probe_for_test(
) -> Result<ProductionMissingTransitionEvidence, String> {
    run_v5_reachability_probe_for_test(V5ClientRequest::Ping {}, ReachabilityExpectedResponse::Pong)
}

#[cfg(feature = "receipt-ledger-test-support")]
pub(crate) fn run_submit_reachability_probe_for_test(
) -> Result<ProductionMissingTransitionEvidence, String> {
    run_v5_reachability_probe_for_test(
        fixed_submit_request_for_test()?,
        ReachabilityExpectedResponse::ReceiptPending(V5InvocationPhase::ReservedUnbound),
    )
}

#[cfg(feature = "receipt-ledger-test-support")]
pub(crate) fn run_direct_load_reachability_probe_for_test(
) -> Result<ProductionMissingTransitionEvidence, String> {
    run_executor_reachability_probe_for_test(V5ExecutorReachabilityAction::RunDirectLoad)
}

#[cfg(feature = "receipt-ledger-test-support")]
pub(crate) fn run_lazy_cancel_storm_reachability_probe_for_test(
) -> Result<ProductionMissingTransitionEvidence, String> {
    run_executor_reachability_probe_for_test(V5ExecutorReachabilityAction::RunLazyCancelStorm)
}

#[cfg(feature = "receipt-ledger-test-support")]
pub(crate) fn run_seed_task_reachability_probe_for_test(
) -> Result<ProductionMissingTransitionEvidence, String> {
    let root =
        tempfile::tempdir().map_err(|error| format!("create v5 task projection state: {error}"))?;
    let state_root = std::fs::canonicalize(root.path())
        .map_err(|error| format!("canonicalize v5 task projection state: {error}"))?;
    let identity = CoreIdentity::production_v5();
    let state = DaemonStateDirectory::open(&state_root, &identity)?;
    let runtime = V5ReceiptRuntime::open(
        &state,
        &DaemonServerConfig::new(state_root, identity, Duration::from_millis(50)),
    )?;
    let token = runtime.observe_missing_task_projection_writer()?;
    Ok(ProductionMissingTransitionEvidence::task_projection_unavailable(token))
}

#[cfg(feature = "receipt-ledger-test-support")]
fn run_executor_reachability_probe_for_test(
    action: V5ExecutorReachabilityAction,
) -> Result<ProductionMissingTransitionEvidence, String> {
    let root = tempfile::tempdir().map_err(|error| format!("create v5 executor state: {error}"))?;
    let state_root = std::fs::canonicalize(root.path())
        .map_err(|error| format!("canonicalize v5 executor state: {error}"))?;
    let identity = CoreIdentity::production_v5();
    let state = DaemonStateDirectory::open(&state_root, &identity)?;
    let runtime = V5ReceiptRuntime::open(
        &state,
        &DaemonServerConfig::new(state_root, identity, Duration::from_millis(50)),
    )?;
    let token = runtime.observe_missing_executor_writer(action)?;
    Ok(ProductionMissingTransitionEvidence::writer_path_unavailable(token))
}

#[cfg(feature = "receipt-ledger-test-support")]
fn fixed_submit_request_for_test() -> Result<V5ClientRequest, String> {
    use crate::application::receipt_ledger::V5ToolIdentity;
    use crate::domain::invocation::{InvocationId, TaskId};
    use std::str::FromStr;

    let invocation = V5InvocationRequest::new(
        InvocationId::from_str("11111111-1111-4111-8111-111111111111")
            .map_err(|_| "invalid fixed v5 reachability invocation id".to_string())?,
        TaskId::from_str("22222222-2222-4222-8222-222222222222")
            .map_err(|_| "invalid fixed v5 reachability task id".to_string())?,
        V5ToolIdentity::View,
        serde_json::Map::new(),
        "workspace-a".to_string(),
        7_000,
    )?;
    Ok(V5ClientRequest::SubmitInvocation { invocation })
}

#[cfg(feature = "receipt-ledger-test-support")]
#[derive(Clone, Copy)]
enum ReachabilityExpectedResponse {
    Pong,
    ReceiptPending(V5InvocationPhase),
}

#[cfg(feature = "receipt-ledger-test-support")]
fn run_v5_reachability_probe_for_test(
    request: V5ClientRequest,
    expected_response: ReachabilityExpectedResponse,
) -> Result<ProductionMissingTransitionEvidence, String> {
    let root = tempfile::tempdir().map_err(|error| format!("create v5 probe state: {error}"))?;
    let state_root = std::fs::canonicalize(root.path())
        .map_err(|error| format!("canonicalize v5 probe state: {error}"))?;
    let identity = CoreIdentity::production_v5();
    let config = DaemonServerConfig::new(
        state_root.clone(),
        identity.clone(),
        Duration::from_millis(50),
    );
    let (evidence_tx, evidence_rx) = sync_channel(1);
    let server = thread::spawn(move || {
        run_daemon_configured(config, |runtime| runtime.with_evidence_capture(evidence_tx))
    });

    let startup_deadline = Instant::now() + Duration::from_secs(5);
    let record = loop {
        let state = DaemonStateDirectory::open(&state_root, &identity)?;
        if let Some(record) = state.read_v5_endpoint_record()? {
            break record;
        }
        if Instant::now() >= startup_deadline {
            return Err("protocol-v5 reachability endpoint was not published".to_string());
        }
        thread::sleep(Duration::from_millis(5));
    };

    let mut stream = TcpStream::connect(record.loopback_addr()?)
        .map_err(|error| daemon_io_error("connect protocol-v5 reachability endpoint", error))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|error| daemon_io_error("bound protocol-v5 reachability read", error))?;
    write_json_line_before(
        &mut stream,
        &V5ClientRequest::Hello {
            protocol_version: DAEMON_PROTOCOL_VERSION,
            token: record.token().to_string(),
            core_identity: identity,
            owner_lease: uuid::Uuid::new_v4().to_string(),
        },
        Instant::now() + Duration::from_secs(2),
    )?;
    let mut reader = BufReader::new(
        stream
            .try_clone()
            .map_err(|error| daemon_io_error("clone protocol-v5 reachability stream", error))?,
    );
    let ready_frame = read_bounded_v5_probe_response_frame(&mut reader)
        .map_err(|error| daemon_io_error("read protocol-v5 reachability ready", error))?;
    let ready: V5HandshakeServerResponse = serde_json::from_slice(&ready_frame)
        .map_err(|_| "protocol-v5 reachability ready is not strict JSON".to_string())?;
    if !ready.matches_record(&record) {
        return Err("protocol-v5 reachability ready does not match endpoint".to_string());
    }

    write_json_line_before(
        &mut stream,
        &request,
        Instant::now() + Duration::from_secs(2),
    )?;
    let response_frame = read_bounded_v5_probe_response_frame(&mut reader)
        .map_err(|error| daemon_io_error("read protocol-v5 reachability response", error))?;
    let response = decode_v5_server_response(&response_frame).map_err(|error| {
        format!("protocol-v5 reachability response is not strict JSON: {error}")
    })?;
    let response_matches = match expected_response {
        ReachabilityExpectedResponse::Pong => matches!(response, V5ServerResponse::Pong),
        ReachabilityExpectedResponse::ReceiptPending(expected_phase) => matches!(
            response,
            V5ServerResponse::Invocation {
                outcome: V5InvocationResponse::ReceiptPending { phase, .. }
            } if phase == expected_phase
        ),
    };
    if !response_matches {
        return Err("protocol-v5 reachability probe received an unexpected response".to_string());
    }
    let evidence = evidence_rx
        .recv_timeout(Duration::from_secs(2))
        .map_err(|_| "protocol-v5 reachability evidence was not captured".to_string())?;
    drop(stream);
    server
        .join()
        .map_err(|_| "protocol-v5 reachability daemon panicked".to_string())??;
    Ok(evidence)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::invocation::normalized_arguments_hash;
    use crate::application::receipt_ledger::{
        receipt_key_digest, request_scope_hash, CancelExpiryOutcome, CancelResolution,
        CommittedDirectPublication, OriginalCutoffDescriptor, ReceiptKey, ReceiptLedgerPort,
        ReceiptRecordHeader, ReceiptState, ReceiptTerminalOutcome, ReceiptVersion, RequestIdentity,
        ReserveOutcome, ReservedPhase, ReservedReceipt, V5CanonicalTerminal, V5ToolIdentity,
        CANCEL_RESERVATION_TTL_MS, MAX_RECEIPT_ENTITLEMENT_BYTES,
    };
    use crate::domain::invocation::{InvocationId, TaskId};
    use crate::infrastructure::daemon::client_v5::V5DaemonProcessOwner;
    use crate::infrastructure::daemon::identity::{CoreIdentity, DaemonStateDirectory};
    use crate::infrastructure::daemon::protocol_v5::V5InvocationRequest;
    use crate::infrastructure::daemon::protocol_v5::{
        decode_v5_server_response, read_bounded_v5_probe_response_frame, V5EndpointRecord,
        V5ProbeResponseKind, V5ProbeServerResponse,
    };
    use crate::infrastructure::platform::testing::{
        attempt_retained_directory_replacement_for_test, RetainedDirectoryReplacementOutcome,
    };
    use serde_json::json;
    use std::io::{BufReader, Read, Write};
    use std::net::TcpStream;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, Instant};

    fn write_json_line(stream: &mut TcpStream, value: &serde_json::Value) {
        let mut bytes = serde_json::to_vec(value).expect("serialize v5 frame");
        bytes.push(b'\n');
        stream.write_all(&bytes).expect("write v5 frame");
    }

    enum CancelPortFailure {
        ImmediateCommitUncertain,
        ImmediateStoreUnavailable,
        WaitPastOperationDeadline {
            observed_deadline: mpsc::Sender<Instant>,
        },
    }

    struct FailingCancelPort {
        failure: CancelPortFailure,
    }

    impl ReceiptLedgerPort for FailingCancelPort {
        fn generation(&mut self, _deadline: Instant) -> Result<u64, ReceiptLedgerError> {
            Ok(0)
        }

        fn reserve(
            &mut self,
            _key: ReceiptKey,
            _original_cutoff: OriginalCutoffDescriptor,
            _deadline: Instant,
        ) -> Result<ReserveOutcome, ReceiptLedgerError> {
            Err(ReceiptLedgerError::StoreUnavailable)
        }

        fn request_cancel_or_reserve(
            &mut self,
            key: ReceiptKey,
            _cancel_reserved_at_epoch_ms: u64,
            deadline: Instant,
        ) -> Result<CancelResolution, ReceiptLedgerError> {
            match self.failure {
                CancelPortFailure::ImmediateCommitUncertain => {
                    Err(ReceiptLedgerError::CommitUncertain {
                        receipt_key_digest: receipt_key_digest(&key),
                    })
                }
                CancelPortFailure::ImmediateStoreUnavailable => {
                    Err(ReceiptLedgerError::StoreUnavailable)
                }
                CancelPortFailure::WaitPastOperationDeadline {
                    ref observed_deadline,
                } => {
                    observed_deadline
                        .send(deadline)
                        .expect("publish live cancel operation deadline");
                    thread::sleep(
                        deadline.saturating_duration_since(Instant::now())
                            + Duration::from_millis(10),
                    );
                    Err(ReceiptLedgerError::StoreUnavailable)
                }
            }
        }

        fn expire_cancel_reserved(
            &mut self,
            _key: ReceiptKey,
            _expected_version: ReceiptVersion,
            _expected_mutation_sequence: u64,
            _observed_at_epoch_ms: u64,
            _deadline: Instant,
        ) -> Result<CancelExpiryOutcome, ReceiptLedgerError> {
            Err(ReceiptLedgerError::StoreUnavailable)
        }

        fn publish_direct_terminal(
            &mut self,
            _key: &ReceiptKey,
            _expected_version: ReceiptVersion,
            _terminal_epoch_ms: u64,
            _terminal: V5CanonicalTerminal,
            _deadline: Instant,
        ) -> Result<CommittedDirectPublication, ReceiptLedgerError> {
            Err(ReceiptLedgerError::StoreUnavailable)
        }

        fn recover(
            &mut self,
            _key: &ReceiptKey,
            _deadline: Instant,
        ) -> Result<ReceiptState, ReceiptLedgerError> {
            Err(ReceiptLedgerError::StoreUnavailable)
        }
    }

    struct SlowReservePort {
        delay: Duration,
    }

    impl ReceiptLedgerPort for SlowReservePort {
        fn generation(&mut self, _deadline: Instant) -> Result<u64, ReceiptLedgerError> {
            Ok(0)
        }

        fn reserve(
            &mut self,
            key: ReceiptKey,
            original_cutoff: OriginalCutoffDescriptor,
            _deadline: Instant,
        ) -> Result<ReserveOutcome, ReceiptLedgerError> {
            thread::sleep(self.delay);
            Ok(ReserveOutcome::Created(ReservedReceipt::new(
                ReceiptRecordHeader::new(
                    key.clone(),
                    receipt_key_digest(&key),
                    ReceiptVersion::initial(),
                    1,
                    512,
                ),
                original_cutoff.accepted_epoch_ms(),
                original_cutoff,
                ReservedPhase::Unbound,
                false,
                MAX_RECEIPT_ENTITLEMENT_BYTES - 512,
            )))
        }

        fn request_cancel_or_reserve(
            &mut self,
            _key: ReceiptKey,
            _cancel_reserved_at_epoch_ms: u64,
            _deadline: Instant,
        ) -> Result<CancelResolution, ReceiptLedgerError> {
            Err(ReceiptLedgerError::StoreUnavailable)
        }

        fn expire_cancel_reserved(
            &mut self,
            _key: ReceiptKey,
            _expected_version: ReceiptVersion,
            _expected_mutation_sequence: u64,
            _observed_at_epoch_ms: u64,
            _deadline: Instant,
        ) -> Result<CancelExpiryOutcome, ReceiptLedgerError> {
            Err(ReceiptLedgerError::StoreUnavailable)
        }

        fn publish_direct_terminal(
            &mut self,
            _key: &ReceiptKey,
            _expected_version: ReceiptVersion,
            _terminal_epoch_ms: u64,
            _terminal: V5CanonicalTerminal,
            _deadline: Instant,
        ) -> Result<CommittedDirectPublication, ReceiptLedgerError> {
            Err(ReceiptLedgerError::StoreUnavailable)
        }

        fn recover(
            &mut self,
            _key: &ReceiptKey,
            _deadline: Instant,
        ) -> Result<ReceiptState, ReceiptLedgerError> {
            Err(ReceiptLedgerError::StoreUnavailable)
        }
    }

    #[test]
    fn seven_second_submit_budget_is_not_truncated_by_transport_timeouts() {
        let root = tempfile::tempdir().expect("temporary long-submit state root");
        let state_root =
            std::fs::canonicalize(root.path()).expect("physical long-submit state root");
        let identity = CoreIdentity::production_v5();
        let config = DaemonServerConfig::new(
            state_root.clone(),
            identity.clone(),
            Duration::from_millis(80),
        );
        let server = thread::spawn(move || {
            run_daemon_configured(config, |mut runtime| {
                runtime.receipt_ledger = ReceiptLedgerActor::spawn(SlowReservePort {
                    delay: Duration::from_millis(5_100),
                });
                runtime
            })
        });
        let _record = wait_for_v5_record(&state_root, &identity);
        let invocation = V5InvocationRequest::new(
            InvocationId::new(),
            TaskId::new(),
            V5ToolIdentity::View,
            serde_json::Map::new(),
            "workspace-a".to_owned(),
            7_000,
        )
        .expect("valid long-submit request");
        let mut owner = V5DaemonProcessOwner::connect_or_spawn_for_protocol_test(
            &state_root,
            identity,
            std::path::PathBuf::from("unused-existing-v5-endpoint"),
            Duration::from_millis(300),
        )
        .expect("connect long-submit owner");
        let response = owner.submit_invocation(invocation);
        drop(owner);
        let server_result = server.join().expect("join long-submit runtime");

        assert!(matches!(
            response,
            Ok(V5ServerResponse::Invocation {
                outcome: V5InvocationResponse::ReceiptPending {
                    phase: V5InvocationPhase::ReservedUnbound,
                    ..
                }
            })
        ));
        assert_eq!(server_result, Ok(()));
    }

    #[test]
    fn commit_uncertain_is_returned_before_process_owned_fail_stop_retains_endpoint() {
        let root = tempfile::tempdir().expect("temporary fail-stop state root");
        let state_root = std::fs::canonicalize(root.path()).expect("physical fail-stop state root");
        let identity = CoreIdentity::production_v5();
        let config = DaemonServerConfig::new(
            state_root.clone(),
            identity.clone(),
            Duration::from_secs(30),
        );
        let server = thread::spawn(move || {
            run_daemon_configured(config, |mut runtime| {
                runtime.receipt_ledger = ReceiptLedgerActor::spawn(FailingCancelPort {
                    failure: CancelPortFailure::ImmediateCommitUncertain,
                });
                runtime
            })
        });
        let record = wait_for_v5_record(&state_root, &identity);
        let key = ReceiptKey::new(
            InvocationId::new(),
            TaskId::new(),
            RequestIdentity::new(
                identity.digest().clone(),
                V5ToolIdentity::View,
                normalized_arguments_hash(&serde_json::Map::new()),
                request_scope_hash("workspace-a").expect("request scope"),
            ),
        );
        let mut owner = V5DaemonProcessOwner::connect_or_spawn_for_protocol_test(
            &state_root,
            identity.clone(),
            std::path::PathBuf::from("unused-existing-v5-endpoint"),
            Duration::from_millis(300),
        )
        .expect("connect fail-stop owner");
        let response = owner.cancel_invocation(key);
        drop(owner);
        let server_result = server.join().expect("join fail-stop runtime");
        let state = DaemonStateDirectory::open(&state_root, &identity)
            .expect("reopen fail-stop daemon state");
        let retained = state
            .read_v5_endpoint_record()
            .expect("read retained fail-stop endpoint");
        let competing_authority = state.acquire_receipt_authority(Duration::from_millis(30));

        assert_eq!(
            response,
            Ok(V5ServerResponse::Error {
                code: V5DaemonErrorCode::DurabilityUncertain,
            })
        );
        assert_eq!(server_result, Ok(()));
        assert_eq!(retained, Some(record));
        assert!(
            competing_authority.is_err(),
            "fail-stop released receipt authority before process death"
        );
    }

    #[test]
    fn running_mutation_timeout_preserves_response_margin_or_closes_after_it() {
        let root = tempfile::tempdir().expect("temporary timeout fail-stop state root");
        let state_root =
            std::fs::canonicalize(root.path()).expect("physical timeout fail-stop state root");
        let identity = CoreIdentity::production_v5();
        let config = DaemonServerConfig::new(
            state_root.clone(),
            identity.clone(),
            Duration::from_secs(30),
        );
        let (operation_deadline_tx, operation_deadline_rx) = mpsc::channel();
        let server = thread::spawn(move || {
            run_daemon_configured(config, |mut runtime| {
                runtime.receipt_ledger = ReceiptLedgerActor::spawn(FailingCancelPort {
                    failure: CancelPortFailure::WaitPastOperationDeadline {
                        observed_deadline: operation_deadline_tx,
                    },
                });
                runtime
            })
        });
        let record = wait_for_v5_record(&state_root, &identity);
        let key = ReceiptKey::new(
            InvocationId::new(),
            TaskId::new(),
            RequestIdentity::new(
                identity.digest().clone(),
                V5ToolIdentity::View,
                normalized_arguments_hash(&serde_json::Map::new()),
                request_scope_hash("workspace-a").expect("request scope"),
            ),
        );
        let decoded = decode_v5_request_frame(
            serde_json::to_vec(&V5ClientRequest::CancelInvocation {
                receipt_key: key.clone(),
            })
            .expect("serialize timeout deadline fixture"),
        )
        .expect("decode timeout deadline fixture");
        let request_received_at = Instant::now();
        let deadlines = v5_request_deadlines(&decoded, request_received_at)
            .expect("derive timeout response deadlines");
        assert_eq!(
            deadlines.operation.duration_since(request_received_at),
            SESSION_READ_TIMEOUT,
            "cancel operation keeps its original bounded session budget"
        );
        assert_eq!(
            deadlines.response.duration_since(deadlines.operation),
            RESPONSE_SERIALIZATION_MARGIN,
            "response serialization gets exactly one non-renewable margin"
        );
        let mut owner = V5DaemonProcessOwner::connect_or_spawn_for_protocol_test(
            &state_root,
            identity.clone(),
            std::path::PathBuf::from("unused-existing-v5-endpoint"),
            Duration::from_millis(300),
        )
        .expect("connect timeout fail-stop owner");
        let response = owner.cancel_invocation(key);
        let response_completed_at = Instant::now();
        let operation_deadline = operation_deadline_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("observe the live cancel operation deadline");
        drop(owner);
        let server_result = server.join().expect("join timeout fail-stop runtime");
        let state = DaemonStateDirectory::open(&state_root, &identity)
            .expect("reopen timeout fail-stop daemon state");
        let retained = state
            .read_v5_endpoint_record()
            .expect("read retained timeout fail-stop endpoint");
        let competing_authority = state.acquire_receipt_authority(Duration::from_millis(30));

        match response {
            Ok(V5ServerResponse::Error {
                code: V5DaemonErrorCode::DurabilityUncertain,
            }) => {}
            Err(error) => {
                assert_eq!(
                    error, "read protocol-v5 cancel invocation: v5 JSON line ended before data",
                    "only expiry of the bounded response margin may replace the closed error"
                );
                let final_response_deadline = operation_deadline
                    .checked_add(RESPONSE_SERIALIZATION_MARGIN)
                    .expect("bounded response deadline");
                assert!(
                    response_completed_at >= final_response_deadline,
                    "transport closed before the live operation deadline and response margin expired"
                );
            }
            unexpected => panic!("unexpected timed-out mutation response: {unexpected:?}"),
        }
        assert_eq!(server_result, Ok(()));
        assert_eq!(retained, Some(record));
        assert!(
            competing_authority.is_err(),
            "timed-out mutation released receipt authority before process death"
        );
    }

    #[test]
    fn every_fail_stop_store_error_is_written_without_reentering_the_actor() {
        let root = tempfile::tempdir().expect("temporary store-failure state root");
        let state_root =
            std::fs::canonicalize(root.path()).expect("physical store-failure state root");
        let identity = CoreIdentity::production_v5();
        let config = DaemonServerConfig::new(
            state_root.clone(),
            identity.clone(),
            Duration::from_secs(30),
        );
        let server = thread::spawn(move || {
            run_daemon_configured(config, |mut runtime| {
                runtime.receipt_ledger = ReceiptLedgerActor::spawn(FailingCancelPort {
                    failure: CancelPortFailure::ImmediateStoreUnavailable,
                });
                runtime
            })
        });
        let record = wait_for_v5_record(&state_root, &identity);
        let key = ReceiptKey::new(
            InvocationId::new(),
            TaskId::new(),
            RequestIdentity::new(
                identity.digest().clone(),
                V5ToolIdentity::View,
                normalized_arguments_hash(&serde_json::Map::new()),
                request_scope_hash("workspace-a").expect("request scope"),
            ),
        );
        let mut owner = V5DaemonProcessOwner::connect_or_spawn_for_protocol_test(
            &state_root,
            identity.clone(),
            std::path::PathBuf::from("unused-existing-v5-endpoint"),
            Duration::from_millis(300),
        )
        .expect("connect store-failure owner");
        let response = owner.cancel_invocation(key);
        drop(owner);
        let server_result = server.join().expect("join store-failure runtime");
        let state = DaemonStateDirectory::open(&state_root, &identity)
            .expect("reopen store-failure daemon state");
        let retained = state
            .read_v5_endpoint_record()
            .expect("read retained store-failure endpoint");
        let competing_authority = state.acquire_receipt_authority(Duration::from_millis(30));

        assert_eq!(
            response,
            Ok(V5ServerResponse::Error {
                code: V5DaemonErrorCode::StoreFailed,
            })
        );
        assert_eq!(server_result, Ok(()));
        assert_eq!(retained, Some(record));
        assert!(
            competing_authority.is_err(),
            "store failure released receipt authority before process death"
        );
    }

    #[test]
    fn healthy_runtime_drops_the_actor_store_before_releasing_named_authority() {
        let source = include_str!("runtime_v5.rs");
        let start = source
            .find("struct V5ReceiptRuntime {")
            .expect("runtime owner declaration");
        let body = source[start..]
            .split_once("\n}")
            .expect("runtime owner declaration end")
            .0;

        assert!(
            body.find("receipt_ledger: ReceiptLedgerActor")
                < body.find("_stable_authority: ReceiptAuthorityLock"),
            "healthy Rust drop order must join actor/store before authority release"
        );
    }

    #[test]
    fn authenticated_release_closes_only_its_owner_session() {
        let root = tempfile::tempdir().expect("temporary release state root");
        let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
        let identity = CoreIdentity::production_v5();
        let config = DaemonServerConfig::new(
            state_root.clone(),
            identity.clone(),
            Duration::from_millis(500),
        );
        let server = thread::spawn(move || run_daemon(config));
        let record = wait_for_v5_record(&state_root, &identity);
        let mut stream = TcpStream::connect(record.loopback_addr().expect("loopback address"))
            .expect("connect release owner");
        let mut reader = BufReader::new(stream.try_clone().expect("clone release stream"));
        write_json_line(
            &mut stream,
            &json!({
                "kind": "hello",
                "protocolVersion": 5,
                "token": record.token(),
                "coreIdentity": identity.as_str(),
                "ownerLease": "77777777-7777-4777-8777-777777777777"
            }),
        );
        read_bounded_v5_probe_response_frame(&mut reader).expect("read release ready");

        write_json_line(&mut stream, &json!({"kind": "release"}));
        let released =
            read_bounded_v5_probe_response_frame(&mut reader).expect("read release response");
        let released = decode_v5_server_response(&released).expect("decode release response");
        drop(stream);

        let mut successor = TcpStream::connect(record.loopback_addr().expect("loopback address"))
            .expect("release must leave the daemon listener available");
        successor
            .set_read_timeout(Some(Duration::from_secs(1)))
            .expect("bound successor session read");
        let mut successor_reader =
            BufReader::new(successor.try_clone().expect("clone successor stream"));
        write_json_line(
            &mut successor,
            &json!({
                "kind": "hello",
                "protocolVersion": 5,
                "token": record.token(),
                "coreIdentity": identity.as_str(),
                "ownerLease": "88888888-8888-4888-8888-888888888888"
            }),
        );
        let successor_ready = read_bounded_v5_probe_response_frame(&mut successor_reader)
            .expect("released owner must not close successor admission");
        let successor_ready: V5HandshakeServerResponse =
            serde_json::from_slice(&successor_ready).expect("decode successor ready response");
        assert!(successor_ready.matches_record(&record));
        write_json_line(&mut successor, &json!({"kind": "ping"}));
        let successor_pong = read_bounded_v5_probe_response_frame(&mut successor_reader)
            .expect("successor session ping");
        let successor_pong: V5ProbeServerResponse =
            serde_json::from_slice(&successor_pong).expect("decode successor pong");
        assert_eq!(successor_pong.kind(), V5ProbeResponseKind::Pong);
        drop(successor);

        server
            .join()
            .expect("join released v5 runtime")
            .expect("released v5 runtime");

        assert_eq!(released, V5ServerResponse::Released);
    }

    struct ManualEpochClock {
        epoch_ms: AtomicU64,
    }

    impl ManualEpochClock {
        fn new(epoch_ms: u64) -> Self {
            Self {
                epoch_ms: AtomicU64::new(epoch_ms),
            }
        }

        fn set(&self, epoch_ms: u64) {
            self.epoch_ms.store(epoch_ms, Ordering::SeqCst);
        }
    }

    impl EpochMillisClock for ManualEpochClock {
        fn now_epoch_millis(&self) -> u64 {
            self.epoch_ms.load(Ordering::SeqCst)
        }
    }

    fn exchange_once_with_epoch(
        state_root: &std::path::Path,
        identity: &CoreIdentity,
        clock: Arc<ManualEpochClock>,
        exchange: impl FnOnce(&mut V5DaemonProcessOwner) -> Result<V5ServerResponse, String>,
    ) -> V5ServerResponse {
        let config = DaemonServerConfig::new(
            state_root.to_path_buf(),
            identity.clone(),
            Duration::from_millis(80),
        );
        let server = thread::spawn(move || {
            run_daemon_configured(config, move |mut runtime| {
                runtime.epoch_clock = clock;
                runtime
            })
        });
        let _record = wait_for_v5_record(state_root, identity);
        let mut owner = V5DaemonProcessOwner::connect_or_spawn_for_protocol_test(
            state_root,
            identity.clone(),
            std::path::PathBuf::from("unused-existing-v5-endpoint"),
            Duration::from_millis(300),
        )
        .expect("connect authenticated one-shot owner");
        let response = exchange(&mut owner).expect("exchange one protocol-v5 request");
        drop(owner);
        server
            .join()
            .expect("join one-shot v5 runtime")
            .expect("one-shot v5 runtime");
        response
    }

    #[test]
    fn receipt_digest_collision_is_a_fail_stop_store_error_not_caller_identity_mismatch() {
        let error = ReceiptLedgerError::ReceiptDigestCollision;

        assert!(error.requires_reopen());
        assert_eq!(daemon_error_code(&error), V5DaemonErrorCode::StoreFailed);
    }

    #[test]
    fn cancel_existing_reserved_receipt_returns_the_typed_pending_winner() {
        let root = tempfile::tempdir().expect("temporary existing-winner state root");
        let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
        let identity = CoreIdentity::production_v5();
        let state = DaemonStateDirectory::open(&state_root, &identity)
            .expect("open existing-winner daemon state");
        let runtime = V5ReceiptRuntime::open(
            &state,
            &DaemonServerConfig::new(state_root, identity.clone(), Duration::from_millis(50)),
        )
        .expect("open protocol-v5 runtime");
        let key = ReceiptKey::new(
            InvocationId::new(),
            TaskId::new(),
            RequestIdentity::new(
                identity.digest().clone(),
                V5ToolIdentity::View,
                normalized_arguments_hash(&serde_json::Map::new()),
                request_scope_hash("workspace-a").expect("request scope"),
            ),
        );
        let cutoff = OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid cutoff");
        runtime
            .receipt_ledger
            .reserve(key.clone(), cutoff, Instant::now() + Duration::from_secs(2))
            .expect("reserve exact receipt");

        let reply = runtime
            .cancel_invocation(key.clone(), 2_000, Instant::now() + Duration::from_secs(2))
            .expect("return the existing reserved winner");

        assert!(matches!(
            reply,
            V5RuntimeReply::Json(V5ServerResponse::Invocation {
                outcome: V5InvocationResponse::ReceiptPending {
                    receipt_key,
                    phase: V5InvocationPhase::ReservedUnbound,
                    accepted_epoch_ms: 1_000,
                    original_budget_ms: 7_000,
                    cancel_requested: false,
                },
            }) if receipt_key == key
        ));
    }

    #[test]
    fn cancel_reserved_reopens_with_the_original_absolute_7125ms_expiry() {
        let root = tempfile::tempdir().expect("temporary restart-stable receipt root");
        let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
        let identity = CoreIdentity::production_v5();
        let clock = Arc::new(ManualEpochClock::new(1_000));
        let arguments = serde_json::Map::new();
        let request_identity = RequestIdentity::new(
            identity.digest().clone(),
            V5ToolIdentity::View,
            normalized_arguments_hash(&arguments),
            request_scope_hash("workspace-a").expect("request scope"),
        );
        let key = ReceiptKey::new(InvocationId::new(), TaskId::new(), request_identity);

        let initial =
            exchange_once_with_epoch(&state_root, &identity, Arc::clone(&clock), |owner| {
                owner.cancel_invocation(key.clone())
            });
        assert!(matches!(
            initial,
            V5ServerResponse::Invocation {
                outcome: V5InvocationResponse::ReceiptPending {
                    accepted_epoch_ms: 1_000,
                    phase: V5InvocationPhase::CancelReserved,
                    ..
                }
            }
        ));

        clock.set(4_000);
        let duplicate =
            exchange_once_with_epoch(&state_root, &identity, Arc::clone(&clock), |owner| {
                owner.cancel_invocation(key.clone())
            });
        assert_eq!(duplicate, initial, "reopen extended the cancellation TTL");

        clock.set(1_000 + CANCEL_RESERVATION_TTL_MS - 1);
        let before_expiry =
            exchange_once_with_epoch(&state_root, &identity, Arc::clone(&clock), |owner| {
                owner.recover_invocation_receipt(key.clone())
            });
        assert_eq!(before_expiry, initial);

        clock.set(1_000 + CANCEL_RESERVATION_TTL_MS);
        let expired = exchange_once_with_epoch(&state_root, &identity, clock, |owner| {
            owner.recover_invocation_receipt(key)
        });
        assert_eq!(
            expired,
            V5ServerResponse::Error {
                code: V5DaemonErrorCode::ReceiptNotFound,
            }
        );
    }

    #[test]
    fn authenticated_pre_cancel_submit_and_recover_cross_the_actor_owned_runtime() {
        let root = tempfile::tempdir().expect("temporary state root");
        let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
        let identity = CoreIdentity::production_v5();
        let config = DaemonServerConfig::new(
            state_root.clone(),
            identity.clone(),
            Duration::from_millis(300),
        );
        let server = thread::spawn(move || run_daemon(config));
        let _record = wait_for_v5_record(&state_root, &identity);

        let arguments = serde_json::Map::new();
        let invocation_id = InvocationId::new();
        let reserved_task_id = TaskId::new();
        let request_identity = RequestIdentity::new(
            identity.digest().clone(),
            V5ToolIdentity::View,
            normalized_arguments_hash(&arguments),
            request_scope_hash("workspace-a").expect("request scope"),
        );
        let key = ReceiptKey::new(invocation_id, reserved_task_id, request_identity);
        let invocation = V5InvocationRequest::new(
            invocation_id,
            reserved_task_id,
            V5ToolIdentity::View,
            arguments,
            "workspace-a".to_string(),
            7_000,
        )
        .expect("strict invocation");
        let unused_executable = std::path::PathBuf::from("unused-existing-v5-endpoint");

        let mut cancel_owner = V5DaemonProcessOwner::connect_or_spawn_for_protocol_test(
            &state_root,
            identity.clone(),
            unused_executable.clone(),
            Duration::from_millis(300),
        )
        .expect("connect authenticated cancel owner");
        let cancel = cancel_owner
            .cancel_invocation(key.clone())
            .expect("durably reserve pre-submit cancellation");
        assert!(matches!(
            cancel,
            V5ServerResponse::Invocation {
                outcome: V5InvocationResponse::ReceiptPending {
                    phase: V5InvocationPhase::CancelReserved,
                    cancel_requested: true,
                    ..
                }
            }
        ));

        let mut submit_owner = V5DaemonProcessOwner::connect_or_spawn_for_protocol_test(
            &state_root,
            identity.clone(),
            unused_executable.clone(),
            Duration::from_millis(300),
        )
        .expect("connect authenticated submit owner");
        let submit = submit_owner
            .submit_invocation(invocation)
            .expect("terminalize exact pre-cancelled submit");
        assert!(matches!(
            &submit,
            V5ServerResponse::Invocation {
                outcome: V5InvocationResponse::Direct { receipt }
            } if matches!(receipt.terminal(), ReceiptTerminalOutcome::Cancelled)
                && receipt.receipt_key() == &key
        ));

        let mut recover_owner = V5DaemonProcessOwner::connect_or_spawn_for_protocol_test(
            &state_root,
            identity,
            unused_executable,
            Duration::from_millis(300),
        )
        .expect("connect authenticated recovery owner");
        let recovered = recover_owner
            .recover_invocation_receipt(key)
            .expect("recover committed direct terminal");
        assert_eq!(recovered, submit, "recovery changed the prepared response");

        server.join().expect("join v5 runtime").expect("v5 runtime");
    }

    fn wait_for_v5_record(
        state_root: &std::path::Path,
        core_identity: &CoreIdentity,
    ) -> V5EndpointRecord {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let state = DaemonStateDirectory::open(state_root, core_identity)
                .expect("open v5 daemon state while waiting");
            if let Some(record) = state
                .read_v5_endpoint_record()
                .expect("read v5 endpoint record")
            {
                return record;
            }
            assert!(Instant::now() < deadline, "v5 endpoint was not published");
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn exact_v5_runtime_opens_receipt_ledger_and_serves_real_handshake_and_ping() {
        let root = tempfile::tempdir().expect("temporary state root");
        let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
        let identity = CoreIdentity::production_v5();
        let config = DaemonServerConfig::new(
            state_root.clone(),
            identity.clone(),
            Duration::from_millis(80),
        );
        let server = thread::spawn(move || run_daemon(config));
        let record = wait_for_v5_record(&state_root, &identity);

        let mut stream = TcpStream::connect(record.loopback_addr().expect("v5 loopback address"))
            .expect("connect v5 daemon");
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("bound v5 response read");
        write_json_line(
            &mut stream,
            &json!({
                "kind": "hello",
                "protocolVersion": 5,
                "token": record.token(),
                "coreIdentity": identity.as_str(),
                "ownerLease": "33333333-3333-4333-8333-333333333333"
            }),
        );
        let mut reader = BufReader::new(stream.try_clone().expect("clone v5 stream"));
        let ready = read_bounded_v5_probe_response_frame(&mut reader).expect("read v5 ready");
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&ready).expect("decode v5 ready"),
            json!({
                "kind": "ready",
                "protocolVersion": 5,
                "coreIdentity": identity.as_str(),
                "daemonPid": std::process::id(),
                "instanceId": record.instance_id()
            })
        );

        write_json_line(&mut stream, &json!({"kind": "ping"}));
        let pong = read_bounded_v5_probe_response_frame(&mut reader).expect("read v5 pong");
        let pong: V5ProbeServerResponse =
            serde_json::from_slice(&pong).expect("decode strict v5 pong");
        assert_eq!(pong.kind(), V5ProbeResponseKind::Pong);
        drop(stream);

        server.join().expect("join v5 runtime").expect("v5 runtime");
        let state = DaemonStateDirectory::open(&state_root, &identity).expect("reopen v5 state");
        assert!(state.read_v5_endpoint_record().unwrap().is_none());
        let receipts = state
            .create_private_retained_subdirectory("receipts")
            .expect("retain production receipts directory");
        assert_eq!(
            std::fs::read(receipts.path().join("generation")).expect("read v5 generation"),
            b"0\n"
        );
    }

    #[cfg(feature = "receipt-ledger-test-support")]
    #[test]
    fn task_projection_evidence_changes_with_production_stable_receipt_observation() {
        let root = tempfile::tempdir().expect("temporary task projection state");
        let state_root =
            std::fs::canonicalize(root.path()).expect("physical task projection state");
        let identity = CoreIdentity::production_v5();
        let state = DaemonStateDirectory::open(&state_root, &identity)
            .expect("open task projection daemon state");
        let encode_projection = |runtime: &V5ReceiptRuntime| {
            let observation = runtime.initial_receipt_observation.clone();
            let token = runtime
                .observe_missing_task_projection_writer()
                .expect("observe production task projection boundary");
            let evidence = ProductionMissingTransitionEvidence::task_projection_unavailable(token);
            let encoded = evidence
                .encode_facade_envelope(0, "seed_task")
                .expect("encode task projection evidence");
            (
                observation,
                serde_json::from_str::<serde_json::Value>(&encoded)
                    .expect("decode task projection evidence"),
            )
        };

        let runtime = V5ReceiptRuntime::open(
            &state,
            &DaemonServerConfig::new(
                state_root.clone(),
                identity.clone(),
                Duration::from_millis(50),
            ),
        )
        .expect("open baseline production v5 receipt runtime");
        let (baseline_observation, baseline) = encode_projection(&runtime);
        drop(runtime);
        std::fs::write(state.path().join("receipts/generation"), b"7\n")
            .expect("advance production receipt generation fixture");
        let runtime = V5ReceiptRuntime::open(
            &state,
            &DaemonServerConfig::new(state_root, identity, Duration::from_millis(50)),
        )
        .expect("open advanced production v5 receipt runtime");
        let (advanced_observation, advanced) = encode_projection(&runtime);

        assert_eq!(baseline_observation.generation_before(), 0);
        assert_eq!(baseline_observation.generation_after(), 0);
        assert_eq!(advanced_observation.generation_before(), 7);
        assert_eq!(advanced_observation.generation_after(), 7);
        assert_eq!(baseline["payload"]["evidence"]["generationBefore"], 0);
        assert_eq!(baseline["payload"]["evidence"]["generationAfter"], 0);
        assert_eq!(advanced["payload"]["evidence"]["generationBefore"], 7);
        assert_eq!(advanced["payload"]["evidence"]["generationAfter"], 7);
        assert_ne!(
            baseline["payload"]["evidence"]["fingerprint"],
            advanced["payload"]["evidence"]["fingerprint"]
        );
    }

    #[cfg(feature = "receipt-ledger-test-support")]
    #[test]
    fn missing_executor_and_task_projection_writers_are_minted_only_after_v5_owner_entry() {
        type Probe = fn() -> Result<ProductionMissingTransitionEvidence, String>;
        for (action, boundary, code, event, probe) in [
            (
                "run_direct_load",
                "v5_executor",
                "writer_path_unavailable",
                Some("v5_executor_entered"),
                run_direct_load_reachability_probe_for_test as Probe,
            ),
            (
                "run_lazy_cancel_storm",
                "v5_executor",
                "writer_path_unavailable",
                Some("v5_executor_entered"),
                run_lazy_cancel_storm_reachability_probe_for_test as Probe,
            ),
            (
                "seed_task",
                "task_projection",
                "task_projection_unavailable",
                None,
                run_seed_task_reachability_probe_for_test as Probe,
            ),
        ] {
            let evidence = probe().unwrap_or_else(|error| panic!("{action} owner entry: {error}"));
            let encoded = evidence
                .encode_facade_envelope(0, action)
                .expect("owner evidence correlates");
            let encoded: serde_json::Value =
                serde_json::from_str(&encoded).expect("closed owner evidence envelope");
            assert_eq!(encoded["payload"]["reachedBoundary"], boundary);
            assert_eq!(encoded["payload"]["currentProtocol"], "v5");
            assert_eq!(encoded["payload"]["evidence"]["code"], code);
            match event {
                Some(event) => assert_eq!(encoded["payload"]["evidence"]["event"], event),
                None => assert!(encoded["payload"]["evidence"]["event"].is_null()),
            }
        }
    }

    #[test]
    fn direct_runtime_entry_rejects_every_non_v5_identity_before_state_creation() {
        use std::str::FromStr;

        for identity in [
            CoreIdentity::production(),
            CoreIdentity::from_str(
                "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
            )
            .expect("arbitrary accepted identity"),
        ] {
            let root = tempfile::tempdir().expect("temporary state root");
            let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
            let result = run_daemon(DaemonServerConfig::new(
                state_root,
                identity,
                Duration::from_millis(10),
            ));

            assert_eq!(
                result,
                Err(
                    "protocol-v5 runtime requires the exact production-v5 core identity"
                        .to_string()
                )
            );
            assert_eq!(
                std::fs::read_dir(root.path())
                    .expect("read untouched root")
                    .count(),
                0
            );
        }
    }

    #[test]
    fn partial_handshake_bytes_cannot_replenish_the_absolute_frame_deadline() {
        let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .expect("bind slowloris fixture");
        let address = listener.local_addr().expect("slowloris address");
        let (done_tx, done_rx) = mpsc::channel();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept slowloris fixture");
            stream
                .set_nonblocking(false)
                .expect("blocking fixture stream");
            let mut reader = BufReader::new(stream);
            let started = Instant::now();
            let result = read_v5_request_before(&mut reader, started + Duration::from_millis(60));
            done_tx
                .send((result.is_err(), started.elapsed()))
                .expect("report bounded read");
        });
        let mut client = TcpStream::connect(address).expect("connect slowloris fixture");
        for byte in b"{\"kind\":\"ping\"}\n" {
            if client.write_all(&[*byte]).is_err() {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        let (rejected, elapsed) = done_rx
            .recv_timeout(Duration::from_millis(250))
            .expect("absolute frame deadline must release the reader");
        assert!(rejected);
        assert!(elapsed < Duration::from_millis(180), "elapsed={elapsed:?}");
        server.join().expect("join slowloris fixture");
    }

    #[test]
    fn expired_partial_handshake_closes_transport_without_a_late_protocol_response() {
        let root = tempfile::tempdir().expect("temporary state root");
        let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
        let identity = CoreIdentity::production_v5();
        let config = DaemonServerConfig::new(
            state_root.clone(),
            identity.clone(),
            HANDSHAKE_READ_TIMEOUT + Duration::from_secs(1),
        );
        let server = thread::spawn(move || run_daemon(config));
        let record = wait_for_v5_record(&state_root, &identity);

        let mut stream = TcpStream::connect(record.loopback_addr().expect("v5 loopback address"))
            .expect("connect v5 daemon");
        stream
            .set_read_timeout(Some(HANDSHAKE_READ_TIMEOUT + Duration::from_secs(1)))
            .expect("bound expired-handshake read");
        stream.write_all(b"{").expect("write partial handshake");
        let started = Instant::now();
        let mut response = Vec::new();
        stream
            .read_to_end(&mut response)
            .expect("expired handshake must close the transport");

        assert!(
            response.is_empty(),
            "transport timeout was misclassified as protocol response: {}",
            String::from_utf8_lossy(&response)
        );
        assert!(
            started.elapsed() < HANDSHAKE_READ_TIMEOUT + Duration::from_millis(500),
            "expired handshake received a replenished response budget: {:?}",
            started.elapsed()
        );
        server.join().expect("join v5 runtime").expect("v5 runtime");
    }

    #[test]
    fn complete_v5_frame_near_cutoff_cannot_receive_a_fresh_response_budget() {
        let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .expect("bind response-deadline fixture");
        let address = listener.local_addr().expect("response-deadline address");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept response-deadline fixture");
            let original_deadline = Instant::now() + Duration::from_millis(30);
            thread::sleep(Duration::from_millis(45));
            let result = write_json_line_before(
                &mut stream,
                &V5ProbeServerResponse::Pong {},
                original_deadline,
            );
            drop(stream);
            result
        });
        let mut client = TcpStream::connect(address).expect("connect response-deadline fixture");
        client
            .set_read_timeout(Some(Duration::from_secs(1)))
            .expect("bound response-deadline read");
        let mut response = Vec::new();
        client
            .read_to_end(&mut response)
            .expect("expired response deadline closes transport");

        assert!(
            server
                .join()
                .expect("join response-deadline fixture")
                .is_err(),
            "expired original deadline granted a new response-write budget"
        );
        assert!(response.is_empty(), "late response escaped: {response:?}");
    }

    #[test]
    fn displaced_receipt_authority_fail_stops_until_process_death() {
        let root = tempfile::tempdir().expect("temporary state root");
        let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
        let identity = CoreIdentity::production_v5();
        let config = DaemonServerConfig::new(
            state_root.clone(),
            identity.clone(),
            Duration::from_millis(80),
        );
        let server = thread::spawn(move || run_daemon(config));
        let record = wait_for_v5_record(&state_root, &identity);
        let state = DaemonStateDirectory::open(&state_root, &identity).expect("open daemon state");
        let receipts = state.path().join("receipts");
        let displaced = state.path().join("receipts-displaced");

        match attempt_retained_directory_replacement_for_test(&receipts, &displaced)
            .expect("attempt receipt authority replacement")
        {
            RetainedDirectoryReplacementOutcome::PreventedByRetainedHandle => {
                server.join().expect("join v5 runtime").expect("v5 runtime");
            }
            RetainedDirectoryReplacementOutcome::Replaced => {
                let displaced_still_ready =
                    match TcpStream::connect(record.loopback_addr().expect("old v5 address")) {
                        Ok(mut stream) => {
                            stream
                                .set_read_timeout(Some(Duration::from_secs(1)))
                                .expect("bound displaced-daemon read");
                            let hello = json!({
                                "kind": "hello",
                                "protocolVersion": 5,
                                "token": record.token(),
                                "coreIdentity": identity.as_str(),
                                "ownerLease": "33333333-3333-4333-8333-333333333333"
                            });
                            if serde_json::to_writer(&mut stream, &hello).is_ok()
                                && stream.write_all(b"\n").is_ok()
                            {
                                let mut reader = BufReader::new(
                                    stream.try_clone().expect("clone displaced v5 stream"),
                                );
                                read_bounded_v5_probe_response_frame(&mut reader).is_ok()
                            } else {
                                false
                            }
                        }
                        Err(_) => false,
                    };
                let server_result = server.join().expect("join displaced v5 runtime");
                assert!(
                    !displaced_still_ready,
                    "displaced receipt owner still accepted a handshake"
                );
                assert!(
                    server_result.is_ok(),
                    "process-owned fail-stop is a controlled daemon shutdown: {server_result:?}"
                );
                let retained_record = state
                    .read_v5_endpoint_record()
                    .expect("read fail-stop endpoint")
                    .expect("fail-stop keeps the PID-bound endpoint until process death");
                assert_eq!(retained_record, record);
                assert!(
                    V5ReceiptRuntime::open(
                        &state,
                        &DaemonServerConfig::new(state_root, identity, Duration::from_millis(120),),
                    )
                    .is_err(),
                    "same-process successor bypassed the retained fail-stop authority"
                );
            }
        }
    }

    #[test]
    fn displaced_runtime_retains_stable_authority_until_the_old_owner_drops() {
        let root = tempfile::tempdir().expect("temporary state root");
        let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
        let identity = CoreIdentity::production_v5();
        let state = DaemonStateDirectory::open(&state_root, &identity).expect("open daemon state");
        let first = V5ReceiptRuntime::open(
            &state,
            &DaemonServerConfig::new(
                state_root.clone(),
                identity.clone(),
                Duration::from_millis(80),
            ),
        )
        .expect("open first runtime owner");
        let receipts = state.path().join("receipts");
        let displaced = state.path().join("receipts-displaced");

        match attempt_retained_directory_replacement_for_test(&receipts, &displaced)
            .expect("attempt receipt authority replacement")
        {
            RetainedDirectoryReplacementOutcome::PreventedByRetainedHandle => return,
            RetainedDirectoryReplacementOutcome::Replaced => {}
        }

        let successor_state =
            DaemonStateDirectory::open(&state_root, &identity).expect("open successor state");
        let successor_while_old_is_live = V5ReceiptRuntime::open(
            &successor_state,
            &DaemonServerConfig::new(
                state_root.clone(),
                identity.clone(),
                Duration::from_millis(80),
            ),
        );
        assert!(
            successor_while_old_is_live.is_err(),
            "replacement receipts directory created a second live runtime authority"
        );

        drop(first);
        V5ReceiptRuntime::open(
            &successor_state,
            &DaemonServerConfig::new(state_root, identity, Duration::from_millis(80)),
        )
        .expect("successor acquires the stable authority after old owner drops");
    }

    #[test]
    fn replacement_receipt_authority_directory_alone_cannot_create_a_successor_runtime() {
        let root = tempfile::tempdir().expect("temporary state root");
        let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
        let identity = CoreIdentity::production_v5();
        let state = DaemonStateDirectory::open(&state_root, &identity).expect("open daemon state");
        let first = V5ReceiptRuntime::open(
            &state,
            &DaemonServerConfig::new(
                state_root.clone(),
                identity.clone(),
                Duration::from_millis(80),
            ),
        )
        .expect("open first runtime owner");
        let authority = state.path().join(".receipt-authority");
        let displaced = state.path().join(".receipt-authority-displaced");

        match attempt_retained_directory_replacement_for_test(&authority, &displaced)
            .expect("attempt stable receipt-authority replacement")
        {
            RetainedDirectoryReplacementOutcome::PreventedByRetainedHandle => return,
            RetainedDirectoryReplacementOutcome::Replaced => {}
        }

        let successor_state =
            DaemonStateDirectory::open(&state_root, &identity).expect("open successor state");
        let successor_while_old_is_live = V5ReceiptRuntime::open(
            &successor_state,
            &DaemonServerConfig::new(
                state_root.clone(),
                identity.clone(),
                Duration::from_millis(80),
            ),
        );

        let error = match successor_while_old_is_live {
            Ok(_) => {
                panic!("replacement receipt-authority directory created a second live runtime")
            }
            Err(error) => error,
        };
        assert_eq!(
            error,
            "open protocol-v5 receipt ledger: receipt ledger is already owned"
        );
        first
            .ensure_named_authority()
            .expect("unchanged receipt ledger keeps the original runtime authoritative");

        drop(first);
        V5ReceiptRuntime::open(
            &successor_state,
            &DaemonServerConfig::new(state_root, identity, Duration::from_millis(80)),
        )
        .expect("successor acquires both authority layers after old owner drops");
    }

    #[test]
    fn displaced_runtime_cannot_write_a_response_after_the_final_authority_check() {
        let root = tempfile::tempdir().expect("temporary state root");
        let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
        let identity = CoreIdentity::production_v5();
        let state = DaemonStateDirectory::open(&state_root, &identity).expect("open daemon state");
        let runtime = V5ReceiptRuntime::open(
            &state,
            &DaemonServerConfig::new(state_root, identity, Duration::from_millis(80)),
        )
        .expect("open runtime owner");
        let receipts = state.path().join("receipts");
        let displaced = state.path().join("receipts-displaced");
        match attempt_retained_directory_replacement_for_test(&receipts, &displaced)
            .expect("attempt receipt authority replacement")
        {
            RetainedDirectoryReplacementOutcome::PreventedByRetainedHandle => return,
            RetainedDirectoryReplacementOutcome::Replaced => {}
        }

        let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .expect("bind displaced response fixture");
        let address = listener.local_addr().expect("displaced response address");
        let client = TcpStream::connect(address).expect("connect displaced response fixture");
        client
            .set_read_timeout(Some(Duration::from_secs(1)))
            .expect("bound displaced response read");
        let (mut server, _) = listener
            .accept()
            .expect("accept displaced response fixture");
        let result = write_runtime_json_line_before(
            &mut server,
            &runtime,
            &V5ProbeServerResponse::Pong {},
            Instant::now() + Duration::from_secs(1),
        );
        drop(server);
        let mut reader = BufReader::new(client);
        let mut response = Vec::new();
        reader
            .read_to_end(&mut response)
            .expect("read displaced response transport");

        assert!(result.is_err(), "displaced runtime wrote a response");
        assert!(
            response.is_empty(),
            "displaced response escaped: {response:?}"
        );
    }

    #[cfg(feature = "receipt-ledger-test-support")]
    #[test]
    fn real_tcp_ping_mints_protocol_evidence_only_after_runtime_frame_handling() {
        let evidence = run_protocol_ping_reachability_probe_for_test()
            .expect("run typed protocol-v5 reachability probe");
        let encoded = evidence
            .encode_facade_envelope(9, "probe_protocol")
            .expect("encode runtime-owned protocol evidence");
        let encoded: serde_json::Value =
            serde_json::from_str(&encoded).expect("decode evidence envelope");

        assert_eq!(encoded["kind"], "production_missing_transition");
        assert_eq!(encoded["payload"]["actionIndex"], 9);
        assert_eq!(encoded["payload"]["actionKind"], "probe_protocol");
        assert_eq!(
            encoded["payload"]["reachedBoundary"],
            "protocol_negotiation"
        );
        assert_eq!(encoded["payload"]["currentProtocol"], "v5");
        assert_eq!(
            encoded["payload"]["evidence"]["code"],
            "protocol_behavior_unavailable"
        );
        assert_eq!(
            encoded["payload"]["evidence"]["event"],
            "protocol_frame_read"
        );
    }

    #[cfg(feature = "receipt-ledger-test-support")]
    #[test]
    fn tcp_submit_reserves_before_reporting_the_missing_executor_writer() {
        let evidence =
            run_submit_reachability_probe_for_test().expect("run typed submit reachability probe");
        let encoded = evidence
            .encode_facade_envelope(3, "submit")
            .expect("encode runtime-owned receipt evidence");
        let encoded: serde_json::Value =
            serde_json::from_str(&encoded).expect("decode evidence envelope");

        assert_eq!(encoded["kind"], "production_missing_transition");
        assert_eq!(encoded["payload"]["actionIndex"], 3);
        assert_eq!(encoded["payload"]["actionKind"], "submit");
        assert_eq!(encoded["payload"]["reachedBoundary"], "v5_executor");
        assert_eq!(encoded["payload"]["currentProtocol"], "v5");
        assert_eq!(
            encoded["payload"]["evidence"]["code"],
            "writer_path_unavailable"
        );
        assert_eq!(
            encoded["payload"]["evidence"]["event"],
            "v5_executor_entered"
        );
        assert_eq!(encoded["payload"]["evidence"]["generationBefore"], 0);
        assert_eq!(encoded["payload"]["evidence"]["generationAfter"], 0);
    }
}
