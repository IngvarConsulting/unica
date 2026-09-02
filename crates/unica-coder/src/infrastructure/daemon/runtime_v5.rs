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
    InvocationStoreV5, NewV5InvocationRecord, RecoveryTerminalReason, TaskStoreRecoveryCatalog,
    V5SafeFailureReason, V5StartWorkingOutcome, V5StoredInvocationRecord, V5StoredTask,
    V5TaskIdentity, V5TaskStoreError, V5TerminalPublication,
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
    canonical_v5_terminal, AttemptPhase, ClosedTerminalStatus, HandoffTerminalStage,
    OriginalCutoffDescriptor, PreparedWireFrame, ProvenTaskLinkCapacity, ReceiptKey,
    ReceiptKeyDigest, ReceiptLedgerError, ReceiptState, ReceiptTaskProjection,
    ReceiptTerminalOutcome, ReserveOutcome, ReservedPhase, TaskBoundReceipt,
    TaskCancellationReceipt, TaskHandoffActorBoundReceipt, TaskPromisedActorBoundReceipt,
    TaskRetirementPendingReceipt, TaskTerminalBoundReceipt, TaskTerminalReceiptBackedReceipt,
    TerminalDigest, DIRECT_TERMINAL_RETENTION_MS,
};
use crate::application::receipt_ledger_actor::ReceiptLedgerActor;
use crate::infrastructure::platform::filesystem::RetainedDirectoryCapability;
use crate::infrastructure::receipt_ledger::ReceiptLedgerStore;
#[cfg(feature = "receipt-ledger-test-support")]
use crate::infrastructure::receipt_ledger::StableReceiptLedgerObservation;
#[cfg(feature = "receipt-ledger-test-support")]
use crate::infrastructure::receipt_ledger_test_evidence::ProductionMissingTransitionEvidence;
use crate::infrastructure::task_lifecycle_link_store_v5::{
    TaskLifecycleLinkCatalogEntry, TaskLifecycleLinkRecord, TaskLifecycleLinkStoreError,
    TaskLifecycleLinkStoreV5,
};
use crate::infrastructure::task_store::SystemEpochMillisClock;
use crate::infrastructure::task_store_v5::FileInvocationStoreV5;
use serde::Serialize;
#[cfg(feature = "receipt-ledger-test-support")]
use serde_json::json;
use serde_json::Value;
#[cfg(feature = "receipt-ledger-test-support")]
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::io::{self, BufReader, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
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
const STARTUP_RECONCILIATION_TIMEOUT: Duration = Duration::from_secs(30);
const HANDSHAKE_READ_TIMEOUT: Duration = Duration::from_secs(2);
const SESSION_READ_TIMEOUT: Duration = Duration::from_secs(2);
const OWNER_RESPONSE_WRITE_TIMEOUT: Duration = Duration::from_secs(10);
const V5_TASK_POLL_INTERVAL_MS: u64 = 100;

#[cfg(feature = "receipt-ledger-test-support")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum V5ReceiptRuntimeEventKind {
    StrictEnvelopeParsed,
    V5ReceiptRuntimeEntered,
    CanonicalV13ServiceEntered,
    ReceiptReserved,
    ValidationEntered,
    AdmissionEntered,
    ActorBoundCommitted,
    ReceiptBegunCommitted,
    PrepareEntered,
    ExecuteEntered,
    UnboundPromiseCommitted,
    BoundHandoffCommitted,
    BoundHandoffTerminalStaged,
    TaskBoundCommitted,
    TaskLinkCapacityReserved,
    TaskLinkCapacityRejected,
    TaskStoreCreated,
    TaskLinkReservationConverted,
    FalseCancelObservationReached,
    TaskStoreWorkingReadback,
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
    ListenerPublished,
    ListenerClosed,
    CancelReservationConverted,
    ResultSerialized,
    ReceiptTerminalCommitted,
    FinalResultProjected,
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
    wait_floor_sequence: u64,
    events: Vec<V5ReceiptRuntimeEvent>,
    callbacks: V5ReceiptRuntimeCallbackCounts,
    listener: V5ReceiptRuntimeListenerState,
    active_listeners: u64,
    daemon_running: bool,
    restart_requested: bool,
    actor_leases: u64,
    terminal_publications: Vec<Value>,
    next_preflight_sequence: u64,
    task_store_create_attempts: u64,
}

#[cfg(feature = "receipt-ledger-test-support")]
#[derive(Clone)]
struct V5ReceiptRuntimeTelemetrySnapshot {
    events: Vec<V5ReceiptRuntimeEvent>,
    callbacks: V5ReceiptRuntimeCallbackCounts,
    listener: V5ReceiptRuntimeListenerState,
    daemon_running: bool,
    restart_requested: bool,
    actor_leases: u64,
    terminal_publications: Vec<Value>,
    task_store_create_attempts: u64,
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
                wait_floor_sequence: 1,
                events: Vec::new(),
                callbacks: V5ReceiptRuntimeCallbackCounts::default(),
                listener: V5ReceiptRuntimeListenerState::NotPublished,
                active_listeners: 0,
                daemon_running: false,
                restart_requested: false,
                actor_leases: 0,
                terminal_publications: Vec::new(),
                next_preflight_sequence: 1,
                task_store_create_attempts: 0,
            }),
            changed: Condvar::new(),
        }
    }

    fn lock_state(&self) -> MutexGuard<'_, V5ReceiptRuntimeTelemetryState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn record_event(&self, event: V5ReceiptRuntimeEventKind, epoch_ms: u64) -> u64 {
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
        sequence
    }

    fn record_prepare(&self) {
        let mut state = self.lock_state();
        state.callbacks.prepare = state.callbacks.prepare.saturating_add(1);
        self.changed.notify_all();
    }

    fn record_validation(&self) {
        let mut state = self.lock_state();
        state.callbacks.validation = state.callbacks.validation.saturating_add(1);
        self.changed.notify_all();
    }

    fn record_admission(&self) {
        let mut state = self.lock_state();
        state.callbacks.admission = state.callbacks.admission.saturating_add(1);
        self.changed.notify_all();
    }

    fn record_execute(&self) {
        let mut state = self.lock_state();
        state.callbacks.execute = state.callbacks.execute.saturating_add(1);
        self.changed.notify_all();
    }

    fn record_task_store_create_attempt(&self) {
        let mut state = self.lock_state();
        state.task_store_create_attempts = state.task_store_create_attempts.saturating_add(1);
        self.changed.notify_all();
    }

    fn record_restart_requested(&self) {
        let mut state = self.lock_state();
        state.restart_requested = true;
        self.changed.notify_all();
    }

    fn record_forced_process_exit(&self) {
        let mut state = self.lock_state();
        let epoch_ms = state.events.last().map_or(1, |event| event.epoch_ms.max(1));
        state.restart_requested = true;
        state.listener = V5ReceiptRuntimeListenerState::Closed;
        state.daemon_running = false;
        self.changed.notify_all();
        drop(state);
        self.record_event(V5ReceiptRuntimeEventKind::ListenerClosed, epoch_ms);
    }

    fn reset_for_scenario(&self) {
        let mut state = self.lock_state();
        state.wait_floor_sequence = state.next_sequence;
        state.callbacks = V5ReceiptRuntimeCallbackCounts::default();
        state.listener = V5ReceiptRuntimeListenerState::NotPublished;
        state.active_listeners = 0;
        state.daemon_running = false;
        state.restart_requested = false;
        state.actor_leases = 0;
        state.terminal_publications.clear();
        state.task_store_create_attempts = 0;
        self.changed.notify_all();
    }

    fn snapshot(&self) -> V5ReceiptRuntimeTelemetrySnapshot {
        let state = self.lock_state();
        V5ReceiptRuntimeTelemetrySnapshot {
            events: state.events.clone(),
            callbacks: state.callbacks,
            listener: state.listener,
            daemon_running: state.daemon_running,
            restart_requested: state.restart_requested,
            actor_leases: if state.restart_requested && !state.daemon_running {
                0
            } else {
                state.actor_leases
            },
            terminal_publications: state.terminal_publications.clone(),
            task_store_create_attempts: state.task_store_create_attempts,
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

    fn record_receipt_backed_publication(
        &self,
        receipt: &TaskTerminalReceiptBackedReceipt,
        record_bytes: &[u8],
    ) {
        let mut state = self.lock_state();
        let terminal_payload_sequence = state.next_preflight_sequence;
        let receipt_record_sequence = terminal_payload_sequence.saturating_add(1);
        let receipt_commit_sequence = receipt_record_sequence.saturating_add(1);
        state.next_preflight_sequence = receipt_commit_sequence.saturating_add(1);
        let record_sha = crate::application::receipt_ledger::ArtifactSha256::from_sha256(
            Sha256::digest(record_bytes).into(),
        );
        state.terminal_publications.push(json!({
            "receiptKey": receipt_key_observation_value(receipt.key()),
            "terminal": terminal_observation_value(
                receipt.terminal(),
                receipt.terminal_epoch_ms(),
            ),
            "commit": {
                "owner": "receipt_backed_task",
                "receipt": {
                    "terminalPayload": artifact_value(
                        receipt.terminal().payload(),
                        u64::try_from(receipt.terminal().payload().len()).unwrap_or(u64::MAX),
                        &crate::application::receipt_ledger::ArtifactSha256::from_sha256(
                            Sha256::digest(receipt.terminal().payload()).into(),
                        ),
                    ),
                    "receiptRecord": artifact_value(
                        record_bytes,
                        u64::try_from(record_bytes.len()).unwrap_or(u64::MAX),
                        &record_sha,
                    ),
                    "candidateResult": candidate_result_value(receipt.terminal()),
                    "terminalPayloadPreparedSequence": terminal_payload_sequence,
                    "receiptRecordPreparedSequence": receipt_record_sequence,
                    "receiptCommitSequence": receipt_commit_sequence,
                    "receiptExpectedVersion": receipt.record_version().get().saturating_sub(1),
                }
            },
            "responseFrames": [],
        }));
    }

    fn wait_for_event(
        &self,
        event: V5ReceiptRuntimeEventKind,
        deadline: Instant,
    ) -> Result<(), String> {
        let mut state = self.lock_state();
        while !state
            .events
            .iter()
            .any(|record| record.sequence >= state.wait_floor_sequence && record.event == event)
        {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .ok_or_else(|| format!("protocol-v5 runtime event {event:?} was not observed"))?;
            let (next, timeout) = self
                .changed
                .wait_timeout(state, remaining)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state = next;
            if timeout.timed_out()
                && !state.events.iter().any(|record| {
                    record.sequence >= state.wait_floor_sequence && record.event == event
                })
            {
                let observed = state
                    .events
                    .iter()
                    .filter(|record| record.sequence >= state.wait_floor_sequence)
                    .map(|record| format!("{:?}", record.event))
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(format!(
                    "protocol-v5 runtime event {event:?} was not observed; observed: [{observed}]"
                ));
            }
        }
        Ok(())
    }

    fn listener_lease(self: &Arc<Self>) -> V5ReceiptRuntimeListenerLease {
        let mut state = self.lock_state();
        let epoch_ms = state.events.last().map_or(1, |event| event.epoch_ms.max(1));
        state.active_listeners = state
            .active_listeners
            .checked_add(1)
            .expect("protocol-v5 runtime listener telemetry exhausted u64");
        state.listener = V5ReceiptRuntimeListenerState::Listening;
        state.daemon_running = true;
        self.changed.notify_all();
        drop(state);
        self.record_event(V5ReceiptRuntimeEventKind::ListenerPublished, epoch_ms);
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
        let epoch_ms = state.events.last().map_or(1, |event| event.epoch_ms.max(1));
        state.actor_leases = state
            .actor_leases
            .checked_sub(1)
            .expect("protocol-v5 runtime actor telemetry lease released only once");
        self.telemetry.changed.notify_all();
        drop(state);
        self.telemetry
            .record_event(V5ReceiptRuntimeEventKind::LeaseReleased, epoch_ms);
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
    external_store_fail_stop: AtomicBool,
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
    #[allow(dead_code)]
    lifecycle_link_root: RetainedDirectoryCapability,
    lifecycle_links: TaskLifecycleLinkStoreV5,
    recovery: TaskStoreRecoveryCatalog,
}

struct StartupTaskTerminalizationPlan {
    expected: TaskBoundReceipt,
    record: V5StoredInvocationRecord,
    reason: RecoveryTerminalReason,
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
        })
    }

    #[cfg(feature = "receipt-ledger-test-support")]
    fn validate_named_identity(&self) -> Result<(), String> {
        self.task_store_root
            .validate_named_identity()
            .map_err(|error| format!("validate protocol-v5 task projection root: {error}"))?;
        self.lifecycle_link_root
            .validate_named_identity()
            .map_err(|error| format!("validate protocol-v5 lifecycle-link root: {error}"))
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
                None => {
                    self.read_startup_linked_task(receipt_key_digest, task_id, provider_deadline)?
                }
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
            self.terminalize_recovered_bound(&plan.expected, plan.record, plan.reason, deadline)?;
        }
        Ok(())
    }

    fn read_startup_linked_task(
        &self,
        receipt_key_digest: &ReceiptKeyDigest,
        task_id: crate::domain::invocation::TaskId,
        deadline: crate::domain::code_intelligence::ProviderDeadline,
    ) -> Result<V5StoredInvocationRecord, V5TaskProjectionFailure> {
        self.task_store.get(task_id, deadline).map_err(|error| {
            V5TaskProjectionFailure::from_task_store(error, receipt_key_digest.clone(), true)
        })
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
            reason,
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
        #[cfg(feature = "receipt-ledger-test-support")] telemetry: &V5ReceiptRuntimeTelemetry,
    ) -> Result<(V5StoredInvocationRecord, TaskBoundReceipt), V5TaskProjectionFailure> {
        self.materialize_actor_bound_task(
            handoff.key(),
            handoff.link(),
            handoff.task(),
            handoff.workspace_identity_hash(),
            handoff.cancel_requested(),
            handoff.phase(),
            false,
            bind_epoch_ms,
            deadline,
            #[cfg(feature = "receipt-ledger-test-support")]
            telemetry,
        )
    }

    fn materialize_recovered_handoff(
        &self,
        handoff: &TaskHandoffActorBoundReceipt,
        bind_epoch_ms: u64,
        deadline: Instant,
        #[cfg(feature = "receipt-ledger-test-support")] telemetry: &V5ReceiptRuntimeTelemetry,
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
            #[cfg(feature = "receipt-ledger-test-support")]
            telemetry,
        )
    }

    fn materialize_promised_actor_bound(
        &self,
        promised: &TaskPromisedActorBoundReceipt,
        bind_epoch_ms: u64,
        deadline: Instant,
        #[cfg(feature = "receipt-ledger-test-support")] telemetry: &V5ReceiptRuntimeTelemetry,
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
            #[cfg(feature = "receipt-ledger-test-support")]
            telemetry,
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
        #[cfg(feature = "receipt-ledger-test-support")] telemetry: &V5ReceiptRuntimeTelemetry,
    ) -> Result<(V5StoredInvocationRecord, TaskBoundReceipt), V5TaskProjectionFailure> {
        let provider_deadline = crate::domain::code_intelligence::ProviderDeadline::new(deadline);
        let reservation = self
            .lifecycle_links
            .reserve_task_link(key.clone(), link.clone(), provider_deadline)
            .map_err(V5TaskProjectionFailure::from_link_store)?;
        #[cfg(feature = "receipt-ledger-test-support")]
        telemetry.record_event(
            V5ReceiptRuntimeEventKind::TaskLinkCapacityReserved,
            bind_epoch_ms,
        );
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
        #[cfg(feature = "receipt-ledger-test-support")]
        telemetry.record_task_store_create_attempt();
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
        #[cfg(feature = "receipt-ledger-test-support")]
        telemetry.record_event(V5ReceiptRuntimeEventKind::TaskStoreCreated, bind_epoch_ms);
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
                &reservation,
                task,
                readback.version,
                bind_epoch_ms,
                phase,
                provider_deadline,
            )
            .map_err(V5TaskProjectionFailure::from_link_store)?;
        #[cfg(feature = "receipt-ledger-test-support")]
        telemetry.record_event(
            V5ReceiptRuntimeEventKind::TaskLinkReservationConverted,
            bind_epoch_ms,
        );
        Ok((readback, bound))
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

    fn publish_bound_task_terminal(
        &self,
        expected: &TaskBoundReceipt,
        record: V5StoredInvocationRecord,
        terminal: &crate::application::receipt_ledger::V5CanonicalTerminal,
        terminal_epoch_ms: u64,
        deadline: Instant,
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
                && record.task == V5StoredTask::Working));
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
        #[cfg(feature = "receipt-ledger-test-support")]
        if let Some(epoch_clock) = config.epoch_clock_for_v5_test() {
            return Self::open_with_epoch_clock(state, config, epoch_clock);
        }
        Self::open_with_epoch_clock(state, config, Arc::new(SystemEpochMillisClock))
    }

    fn open_with_epoch_clock(
        state: &DaemonStateDirectory,
        config: &DaemonServerConfig,
        epoch_clock: Arc<dyn EpochMillisClock>,
    ) -> Result<Self, String> {
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
        #[cfg(feature = "receipt-ledger-test-support")]
        let initial_receipt_observation = receipt_ledger
            .observe_stable_generation()
            .map_err(|error| format!("observe initial protocol-v5 receipt generation: {error}"))?;
        let receipt_ledger = ReceiptLedgerActor::spawn(receipt_ledger);
        let task_projection =
            V5TaskProjection::open(state, Arc::clone(&epoch_clock), startup_deadline)?;
        let _task_recovery_entries = task_projection.recovery.entries().len();
        let runtime = Self {
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
            external_store_fail_stop: AtomicBool::new(false),
            #[cfg(feature = "receipt-ledger-test-support")]
            evidence_capture: None,
            #[cfg(feature = "receipt-ledger-test-support")]
            scenario_control: None,
            #[cfg(feature = "receipt-ledger-test-support")]
            telemetry: Arc::new(V5ReceiptRuntimeTelemetry::new()),
        };
        #[cfg(feature = "receipt-ledger-test-support")]
        let skip_reconciliation = config.skip_v5_startup_reconciliation_for_test();
        #[cfg(not(feature = "receipt-ledger-test-support"))]
        let skip_reconciliation = false;
        if !skip_reconciliation {
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

    #[cfg(feature = "receipt-ledger-test-support")]
    fn attempt_task_store_bind_under_gate_for_test(
        &self,
        key: &ReceiptKey,
        operation_label: &str,
        deadline: Instant,
    ) -> Result<(Value, Value), String> {
        let epoch_ms = self.epoch_clock.now_epoch_millis();
        let state = self
            .receipt_ledger
            .recover(key.clone(), deadline)
            .map_err(|error| format!("recover capacity handoff receipt: {error}"))?;
        let handoff = match state {
            ReceiptState::TaskPromisedActorBound(promised) => self
                .receipt_ledger
                .begin_bound_task_handoff(
                    promised.key().clone(),
                    promised.record_version(),
                    promised.task().created_at_epoch_ms(),
                    promised.task().ttl_ms(),
                    promised.task().poll_interval_ms(),
                    deadline,
                )
                .map_err(|error| format!("begin capacity handoff: {error}"))?,
            ReceiptState::TaskHandoffActorBound(handoff) => handoff,
            other => {
                return Err(format!(
                    "capacity bind requires actor-bound Task handoff, found {}",
                    other.kind().diagnostic_name()
                ))
            }
        };
        let task_store_generation = u64::try_from(self.task_projection.recovery.entries().len())
            .map_err(|_| "TaskStore generation does not fit capacity evidence".to_owned())?;
        let attempts_before = self.telemetry.snapshot().task_store_create_attempts;
        let checked_sequence = self
            .telemetry
            .record_event(V5ReceiptRuntimeEventKind::V5ReceiptRuntimeEntered, epoch_ms);
        let failure = match self.task_projection.materialize_bound_handoff(
            &handoff,
            epoch_ms,
            deadline,
            &self.telemetry,
        ) {
            Ok(_) => {
                return Err(
                    "full lifecycle-link pool unexpectedly admitted another Task".to_owned(),
                )
            }
            Err(failure) => failure,
        };
        if failure.fail_stop || failure.error != ReceiptLedgerError::CapacityExceeded {
            return Err(format!(
                "capacity bind failed for a non-capacity reason: {}",
                failure.error
            ));
        }
        let rejected_sequence = self.telemetry.record_event(
            V5ReceiptRuntimeEventKind::TaskLinkCapacityRejected,
            epoch_ms,
        );
        let proof = ProvenTaskLinkCapacity::Count {
            observed_live_links: u64::try_from(
                crate::infrastructure::task_lifecycle_link_store_v5::MAX_TASK_LIFECYCLE_LINK_RECORDS,
            )
            .expect("Task lifecycle-link limit fits u64"),
            maximum_live_links: u64::try_from(
                crate::infrastructure::task_lifecycle_link_store_v5::MAX_TASK_LIFECYCLE_LINK_RECORDS,
            )
            .expect("Task lifecycle-link limit fits u64"),
        };
        let terminal = if handoff.phase() == AttemptPhase::Begun {
            self.receipt_ledger
                .retain_begun_task_after_link_capacity(
                    handoff.key().clone(),
                    handoff.record_version(),
                    proof,
                    deadline,
                )
                .map_err(|error| format!("retain receipt-owned begun Task: {error}"))?;
            None
        } else {
            let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Failed {
                reason: V5SafeFailureReason::TaskCapacity,
            })
            .map_err(|error| format!("encode Task capacity terminal: {error}"))?;
            let committed = self
                .receipt_ledger
                .publish_receipt_backed_task_terminal(
                    handoff.key().clone(),
                    TaskCancellationReceipt::HandoffActorBound(handoff.clone()),
                    epoch_ms,
                    terminal,
                    deadline,
                )
                .map_err(|error| format!("publish Task capacity terminal: {error}"))?;
            if let Some(control) = &self.scenario_control {
                control
                    .record_receipt_backed_terminal(committed.clone())
                    .map_err(|error| format!("record Task capacity terminal: {error}"))?;
            }
            self.telemetry.record_event(
                V5ReceiptRuntimeEventKind::ReceiptTerminalCommitted,
                epoch_ms,
            );
            Some(committed.terminal().clone())
        };
        let attempts_after = self.telemetry.snapshot().task_store_create_attempts;
        let terminal_observation = terminal
            .as_ref()
            .map(|terminal| terminal_observation_value(terminal, epoch_ms));
        let response = json!({
            "kind": if terminal.is_some() { "task" } else { "rejected" },
            "error": "task_capacity",
            "terminal": terminal_observation,
            "key": receipt_key_observation_value(key),
            "task": null,
            "acknowledgement": null,
            "cutoffEpochMs": null,
            "originalBudgetMs": null,
            "latencyMs": 0,
        });
        let observation = json!({
            "operationLabel": operation_label,
            "receiptKey": receipt_key_observation_value(key),
            "terminal": terminal_observation,
            "stagedTransferCertificateSha256": null,
            "evidence": {
                "source": "link_capacity",
                "capacity_checked_sequence": checked_sequence,
                "capacity_rejected_sequence": rejected_sequence,
                "task_store_generation_before": task_store_generation,
                "task_store_generation_after": task_store_generation,
                "task_store_create_attempts_before": attempts_before,
                "task_store_create_attempts_after": attempts_after,
            }
        });
        Ok((response, observation))
    }

    #[cfg(feature = "receipt-ledger-test-support")]
    fn mark_reserved_begun_under_gate_for_test(
        &self,
        key: &ReceiptKey,
        operation_label: &str,
        deadline: Instant,
    ) -> Result<(), String> {
        self.telemetry.record_event(
            V5ReceiptRuntimeEventKind::V5ReceiptRuntimeEntered,
            self.epoch_ms(),
        );
        let control = self
            .scenario_control
            .as_ref()
            .ok_or_else(|| "reserved-begin scenario control is unavailable".to_owned())?;
        if control.is_barrier_installed(
            receipt_scenario_v5::ScenarioBarrierPoint::BeforeMarkReservedBegunGateAcquire,
        ) {
            control.record_operation_event(operation_label, "blocked");
            self.telemetry.record_event(
                V5ReceiptRuntimeEventKind::MarkReservedBegunBlocked,
                self.epoch_ms(),
            );
            control
                .pause(
                    receipt_scenario_v5::ScenarioBarrierPoint::BeforeMarkReservedBegunGateAcquire,
                    deadline,
                )
                .map_err(|error| format!("wait before reserved-begin lifecycle gate: {error}"))?;
        }
        control
            .acquire_lifecycle_gate(operation_label, deadline)
            .map_err(|error| format!("acquire reserved-begin lifecycle gate: {error}"))?;
        let result = (|| {
            let current = self
                .receipt_ledger
                .recover(key.clone(), deadline)
                .map_err(|error| format!("recover actor-bound receipt before begin: {error}"))?;
            let ReceiptState::Reserved(reserved) = current else {
                return Ok(());
            };
            if !matches!(reserved.phase(), ReservedPhase::ActorBound { .. }) {
                return Ok(());
            }
            let begun = match self.receipt_ledger.mark_reserved_begun(
                key.clone(),
                reserved.record_version(),
                deadline,
            ) {
                Ok(begun) => begun,
                Err(error) => {
                    let winner = self.receipt_ledger.recover(key.clone(), deadline).map_err(
                        |recover_error| {
                            format!(
                                "mark actor-bound receipt begun: {error}; recover winner: {recover_error}"
                            )
                        },
                    )?;
                    if !matches!(winner, ReceiptState::Reserved(_)) {
                        return Ok(());
                    }
                    return Err(format!("mark actor-bound receipt begun: {error}"));
                }
            };
            control.record_reserved_begin_authorization(operation_label, begun.key());
            self.telemetry.record_event(
                V5ReceiptRuntimeEventKind::ReceiptBegunCommitted,
                self.epoch_ms(),
            );
            self.telemetry
                .record_event(V5ReceiptRuntimeEventKind::TokenSignalled, self.epoch_ms());
            Ok(())
        })();
        control.release_lifecycle_gate(operation_label);
        result
    }

    #[cfg(feature = "receipt-ledger-test-support")]
    fn cancel_under_gate_for_test(
        &self,
        key: &ReceiptKey,
        operation_label: &str,
        deadline: Instant,
    ) -> Result<(), String> {
        self.telemetry.record_event(
            V5ReceiptRuntimeEventKind::V5ReceiptRuntimeEntered,
            self.epoch_ms(),
        );
        let control = self
            .scenario_control
            .as_ref()
            .ok_or_else(|| "cancel scenario control is unavailable".to_owned())?;
        if control.is_barrier_installed(
            receipt_scenario_v5::ScenarioBarrierPoint::BeforeCancelGateAcquire,
        ) {
            control.record_operation_event(operation_label, "blocked");
            self.telemetry.record_event(
                V5ReceiptRuntimeEventKind::CancelCommitBlocked,
                self.epoch_ms(),
            );
            control
                .pause(
                    receipt_scenario_v5::ScenarioBarrierPoint::BeforeCancelGateAcquire,
                    deadline,
                )
                .map_err(|error| format!("wait before cancel lifecycle gate: {error}"))?;
        }
        control
            .acquire_lifecycle_gate(operation_label, deadline)
            .map_err(|error| format!("acquire cancel lifecycle gate: {error}"))?;
        let result = (|| {
            self.cancel_invocation(key.clone(), self.epoch_ms(), deadline)
                .map_err(|error| format!("cancel under lifecycle gate: {error}"))?;
            if let ReceiptState::Reserved(reserved) = self
                .receipt_ledger
                .recover(key.clone(), deadline)
                .map_err(|error| format!("recover cancel winner: {error}"))?
            {
                if matches!(reserved.phase(), ReservedPhase::ActorBound { .. })
                    && reserved.cancel_requested()
                {
                    let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Cancelled)
                        .map_err(|error| format!("prepare cancel winner terminal: {error}"))?;
                    self.receipt_ledger
                        .publish_direct_terminal(
                            key.clone(),
                            reserved.record_version(),
                            self.epoch_ms(),
                            terminal,
                            deadline,
                        )
                        .map_err(|error| format!("publish cancel winner terminal: {error}"))?;
                    self.telemetry.record_event(
                        V5ReceiptRuntimeEventKind::ReceiptTerminalCommitted,
                        self.epoch_ms(),
                    );
                }
            }
            Ok(())
        })();
        if result.is_ok() {
            self.telemetry
                .record_event(V5ReceiptRuntimeEventKind::CancelCommitted, self.epoch_ms());
        }
        control.release_lifecycle_gate(operation_label);
        result
    }

    #[cfg(feature = "receipt-ledger-test-support")]
    fn bind_task_under_gate_for_test(
        &self,
        key: &ReceiptKey,
        operation_label: &str,
        deadline: Instant,
    ) -> Result<(), String> {
        self.telemetry.record_event(
            V5ReceiptRuntimeEventKind::V5ReceiptRuntimeEntered,
            self.epoch_ms(),
        );
        let control = self
            .scenario_control
            .as_ref()
            .ok_or_else(|| "Task bind scenario control is unavailable".to_owned())?;
        control
            .acquire_lifecycle_gate(operation_label, deadline)
            .map_err(|error| format!("acquire Task bind lifecycle gate: {error}"))?;
        let result = (|| {
            let current = self
                .receipt_ledger
                .recover(key.clone(), deadline)
                .map_err(|error| format!("recover actor-bound Task promise: {error}"))?;
            let handoff = match current {
                ReceiptState::TaskPromisedActorBound(promised) => self
                    .receipt_ledger
                    .begin_bound_task_handoff(
                        promised.key().clone(),
                        promised.record_version(),
                        promised.task().created_at_epoch_ms(),
                        promised.task().ttl_ms(),
                        promised.task().poll_interval_ms(),
                        deadline,
                    )
                    .map_err(|error| format!("begin Task handoff: {error}"))?,
                ReceiptState::TaskHandoffActorBound(handoff) => handoff,
                other => {
                    return Err(format!(
                        "Task bind requires actor-bound promise, found {}",
                        other.kind().diagnostic_name()
                    ))
                }
            };
            let (record, bound) = self
                .task_projection
                .materialize_bound_handoff(&handoff, self.epoch_ms(), deadline, &self.telemetry)
                .map_err(|failure| format!("materialize actor-bound Task: {}", failure.error))?;
            control.record_bound_task(record.clone(), bound.clone());
            self.telemetry.record_event(
                V5ReceiptRuntimeEventKind::TaskStoreReadbackBeforeBind,
                self.epoch_ms(),
            );
            if control.is_barrier_installed(
                receipt_scenario_v5::ScenarioBarrierPoint::AfterTaskStoreReadbackBeforeTaskBound,
            ) {
                control.record_operation_event(operation_label, "blocked");
                control
                    .pause(
                        receipt_scenario_v5::ScenarioBarrierPoint::AfterTaskStoreReadbackBeforeTaskBound,
                        deadline,
                    )
                    .map_err(|error| format!("wait after TaskStore readback: {error}"))?;
            }
            self.receipt_ledger
                .complete_bound_task_handoff(
                    handoff.key().clone(),
                    handoff.record_version(),
                    bound.clone(),
                    deadline,
                )
                .map_err(|error| format!("complete Task handoff: {error}"))?;
            control.record_handoff_task_binding(operation_label, &handoff, &bound);
            control.record_bound_task(record, bound);
            self.telemetry.record_event(
                V5ReceiptRuntimeEventKind::TaskBoundCommitted,
                self.epoch_ms(),
            );
            Ok(())
        })();
        control.release_lifecycle_gate(operation_label);
        result
    }

    #[cfg(feature = "receipt-ledger-test-support")]
    fn continue_receipt_owned_attempt_for_test(
        &self,
        key: &ReceiptKey,
        result: crate::domain::invocation::DomainResult,
        deadline: Instant,
    ) -> Result<Value, String> {
        let epoch_ms = self.epoch_clock.now_epoch_millis();
        let state = self
            .receipt_ledger
            .recover(key.clone(), deadline)
            .map_err(|error| format!("recover receipt-owned Task attempt: {error}"))?;
        let ReceiptState::TaskReceiptOwnedActorBound(receipt_owned) = state else {
            return Err("continued Task attempt is not receipt-owned".to_owned());
        };
        let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Completed {
            result: Box::new(result),
        })
        .map_err(|error| format!("encode receipt-owned Task terminal: {error}"))?;
        let committed = self
            .receipt_ledger
            .publish_receipt_backed_task_terminal(
                key.clone(),
                TaskCancellationReceipt::ReceiptOwnedActorBound(receipt_owned),
                epoch_ms,
                terminal,
                deadline,
            )
            .map_err(|error| format!("publish receipt-owned Task terminal: {error}"))?;
        if let Some(control) = &self.scenario_control {
            control
                .record_receipt_backed_terminal(committed.clone())
                .map_err(|error| format!("record receipt-owned Task terminal: {error}"))?;
        }
        self.telemetry.record_event(
            V5ReceiptRuntimeEventKind::ReceiptTerminalCommitted,
            epoch_ms,
        );
        Ok(json!({
            "kind": "task",
            "error": null,
            "terminal": terminal_observation_value(committed.terminal(), epoch_ms),
            "key": receipt_key_observation_value(key),
            "task": null,
            "acknowledgement": null,
            "cutoffEpochMs": null,
            "originalBudgetMs": null,
            "latencyMs": 0,
        }))
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
                            #[cfg(feature = "receipt-ledger-test-support")]
                            &self.telemetry,
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
                    if !matches!(handoff.terminal_stage(), HandoffTerminalStage::NoTerminal) {
                        return Err(
                            "reconcile protocol-v5 staged handoff terminal before listener"
                                .to_owned(),
                        );
                    }
                    let receipt_version = handoff.record_version();
                    let phase = handoff.phase();
                    let (record, task_bound) = self
                        .task_projection
                        .materialize_recovered_handoff(
                            &handoff,
                            terminal_epoch_ms,
                            deadline,
                            #[cfg(feature = "receipt-ledger-test-support")]
                            &self.telemetry,
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

    #[cfg(feature = "receipt-ledger-test-support")]
    fn with_shared_telemetry(mut self, telemetry: Arc<V5ReceiptRuntimeTelemetry>) -> Self {
        self.telemetry = telemetry;
        self
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
        if let Some(record) = self
            .task_projection
            .cancel_exact_bound_task(&key, deadline)
            .map_err(|failure| self.project_task_failure(failure))?
        {
            return Ok(V5RuntimeReply::Json(V5ServerResponse::Invocation {
                outcome: V5InvocationResponse::Task {
                    snapshot: task_store_snapshot(&record),
                },
            }));
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
                        #[cfg(feature = "receipt-ledger-test-support")]
                        {
                            if let Some(control) = &self.scenario_control {
                                control.record_receipt_backed_terminal(committed).map_err(
                                    |_| {
                                        ReceiptLedgerError::Corrupt(
                                            "capture receipt-backed terminal evidence failed",
                                        )
                                    },
                                )?;
                                control.release_pre_actor_barriers();
                            }
                            self.telemetry.record_event(
                                V5ReceiptRuntimeEventKind::ReceiptTerminalCommitted,
                                epoch_ms,
                            );
                        }
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
        #[cfg(feature = "receipt-ledger-test-support")]
        if matches!(&outcome, ReserveOutcome::Created(_)) {
            self.telemetry
                .record_event(V5ReceiptRuntimeEventKind::ReceiptReserved, epoch_ms);
        }
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
        #[cfg(feature = "receipt-ledger-test-support")]
        {
            self.telemetry.record_validation();
            self.telemetry
                .record_event(V5ReceiptRuntimeEventKind::ValidationEntered, epoch_ms);
            if let Some(control) = &self.scenario_control {
                control.pause(
                    receipt_scenario_v5::ScenarioBarrierPoint::ValidationEntered,
                    deadline,
                )?;
                let current = self
                    .receipt_ledger
                    .recover(reservation.key().clone(), deadline)?;
                if !matches!(
                    &current,
                    ReceiptState::Reserved(receipt)
                        if matches!(receipt.phase(), ReservedPhase::Unbound)
                ) && !matches!(&current, ReceiptState::TaskPromisedUnbound(_))
                {
                    return self.reply_for_existing_state(current, deadline);
                }
                if control.validation_rejects() {
                    let terminal = crate::application::receipt_ledger::canonical_v5_terminal(
                        &crate::application::receipt_ledger::ReceiptTerminalOutcome::Completed {
                            result: Box::new(
                                crate::domain::invocation::DomainResult::canonical_rejection(
                                    None,
                                    "bad_value",
                                    "scenario validation rejected invocation",
                                ),
                            ),
                        },
                    )
                    .map_err(|_| ReceiptLedgerError::Corrupt("canonical v5 terminal failed"))?;
                    return self.publish_pre_actor_terminal(
                        reservation,
                        epoch_ms,
                        terminal,
                        deadline,
                    );
                }
            }
            self.telemetry.record_admission();
            self.telemetry
                .record_event(V5ReceiptRuntimeEventKind::AdmissionEntered, epoch_ms);
            if let Some(control) = &self.scenario_control {
                control.pause(
                    receipt_scenario_v5::ScenarioBarrierPoint::AdmissionEntered,
                    deadline,
                )?;
                let current = self
                    .receipt_ledger
                    .recover(reservation.key().clone(), deadline)?;
                if control.process_exited() {
                    return self.reply_for_existing_state(current, deadline);
                }
                if !matches!(
                    &current,
                    ReceiptState::Reserved(receipt)
                        if matches!(receipt.phase(), ReservedPhase::Unbound)
                ) && !matches!(&current, ReceiptState::TaskPromisedUnbound(_))
                {
                    return self.reply_for_existing_state(current, deadline);
                }
                if let Some(rejection) = control.admission_rejection() {
                    let fail_stop = matches!(
                        rejection,
                        receipt_scenario_v5::ScenarioWorkspaceAdmissionFailure::RegistryFailed
                    );
                    let outcome = match rejection {
                        receipt_scenario_v5::ScenarioWorkspaceAdmissionFailure::Invalid => {
                            crate::application::receipt_ledger::ReceiptTerminalOutcome::Completed {
                                result: Box::new(
                                    crate::domain::invocation::DomainResult::canonical_rejection(
                                        None,
                                        "bad_value",
                                        "scenario workspace admission rejected invocation",
                                    ),
                                ),
                            }
                        }
                        receipt_scenario_v5::ScenarioWorkspaceAdmissionFailure::Capacity => {
                            crate::application::receipt_ledger::ReceiptTerminalOutcome::Failed {
                                reason: V5SafeFailureReason::WorkspaceCapacity,
                            }
                        }
                        receipt_scenario_v5::ScenarioWorkspaceAdmissionFailure::RegistryFailed => {
                            crate::application::receipt_ledger::ReceiptTerminalOutcome::Failed {
                                reason: V5SafeFailureReason::WorkspaceRegistryFailed,
                            }
                        }
                    };
                    let terminal =
                        crate::application::receipt_ledger::canonical_v5_terminal(&outcome)
                            .map_err(|_| {
                                ReceiptLedgerError::Corrupt("canonical v5 terminal failed")
                            })?;
                    let reply =
                        self.publish_pre_actor_terminal(reservation, epoch_ms, terminal, deadline);
                    if fail_stop {
                        self.telemetry.record_restart_requested();
                        self.external_store_fail_stop.store(true, Ordering::Release);
                    }
                    return match (fail_stop, reply) {
                        (true, Ok(V5RuntimeReply::Prepared(frame))) => {
                            Ok(V5RuntimeReply::PreparedFailStop(frame))
                        }
                        (true, Ok(V5RuntimeReply::Json(response))) => {
                            Ok(V5RuntimeReply::JsonFailStop(response))
                        }
                        (_, reply) => reply,
                    };
                }
            }
        }
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
                return self.publish_pre_actor_terminal(reservation, epoch_ms, terminal, deadline);
            }
        };
        let current = self
            .receipt_ledger
            .recover(reservation.key().clone(), deadline)?;
        if let ReceiptState::TaskPromisedUnbound(promised) = current {
            #[cfg(feature = "receipt-ledger-test-support")]
            if let Some(control) = &self.scenario_control {
                control
                    .record_actor_workspace_identity(actor_bound.workspace_identity_hash().clone());
            }
            let actor_promised = self.receipt_ledger.bind_promised_task_actor(
                promised.key().clone(),
                promised.record_version(),
                actor_bound.workspace_identity_hash().clone(),
                deadline,
            )?;
            #[cfg(feature = "receipt-ledger-test-support")]
            self.telemetry
                .record_event(V5ReceiptRuntimeEventKind::ActorBoundCommitted, epoch_ms);
            #[cfg(feature = "receipt-ledger-test-support")]
            if let Some(control) = &self.scenario_control {
                control.pause(
                    receipt_scenario_v5::ScenarioBarrierPoint::ActorBound,
                    deadline,
                )?;
            }
            let handoff = self.receipt_ledger.begin_bound_task_handoff(
                actor_promised.key().clone(),
                actor_promised.record_version(),
                actor_promised.task().created_at_epoch_ms(),
                actor_promised.task().ttl_ms(),
                actor_promised.task().poll_interval_ms(),
                deadline,
            )?;
            #[cfg(feature = "receipt-ledger-test-support")]
            self.telemetry
                .record_event(V5ReceiptRuntimeEventKind::BoundHandoffCommitted, epoch_ms);
            #[cfg(feature = "receipt-ledger-test-support")]
            if let Some(control) = &self.scenario_control {
                control.pause(
                    receipt_scenario_v5::ScenarioBarrierPoint::BeforeTaskStoreCreate,
                    deadline,
                )?;
            }
            let (task_record, bound) = self
                .task_projection
                .materialize_bound_handoff(
                    &handoff,
                    epoch_ms,
                    deadline,
                    #[cfg(feature = "receipt-ledger-test-support")]
                    &self.telemetry,
                )
                .map_err(|failure| self.project_task_failure(failure))?;
            #[cfg(feature = "receipt-ledger-test-support")]
            if let Some(control) = &self.scenario_control {
                control.record_promised_actor_binding(&promised, &actor_promised, &bound);
            }
            self.receipt_ledger.complete_bound_task_handoff(
                handoff.key().clone(),
                handoff.record_version(),
                bound.clone(),
                deadline,
            )?;
            #[cfg(feature = "receipt-ledger-test-support")]
            if let Some(control) = &self.scenario_control {
                control.record_bound_task(task_record.clone(), bound.clone());
            }
            #[cfg(feature = "receipt-ledger-test-support")]
            self.telemetry
                .record_event(V5ReceiptRuntimeEventKind::TaskBoundCommitted, epoch_ms);
            let authorized_bound =
                if !task_record.cancel_requested && task_record.task == V5StoredTask::Queued {
                    self.task_projection
                        .authorize_not_begun_bound_task_start(&bound, &task_record, deadline)
                        .map_err(|failure| self.project_task_failure(failure))?
                } else {
                    bound
                };
            #[cfg(feature = "receipt-ledger-test-support")]
            if let Some(control) = &self.scenario_control {
                control.record_bound_task(task_record.clone(), authorized_bound.clone());
            }
            #[cfg(feature = "receipt-ledger-test-support")]
            if !task_record.cancel_requested && task_record.task == V5StoredTask::Queued {
                self.telemetry.record_event(
                    V5ReceiptRuntimeEventKind::FalseCancelObservationReached,
                    epoch_ms,
                );
                if let Some(control) = &self.scenario_control {
                    control.pause(
                        receipt_scenario_v5::ScenarioBarrierPoint::AfterFalseCancelObservation,
                        deadline,
                    )?;
                }
            }
            #[cfg(feature = "receipt-ledger-test-support")]
            let scenario_gate_acquired = if let Some(control) = &self.scenario_control {
                if control.is_barrier_installed(
                    receipt_scenario_v5::ScenarioBarrierPoint::AfterWorkingReadback,
                ) {
                    control.acquire_lifecycle_gate("submit", deadline)?;
                    true
                } else {
                    false
                }
            } else {
                false
            };
            let (mut task_record, bound) = self
                .task_projection
                .start_not_begun_bound_task(&authorized_bound, task_record, deadline)
                .map_err(|failure| self.project_task_failure(failure))?;
            #[cfg(feature = "receipt-ledger-test-support")]
            if task_record.task == V5StoredTask::Working {
                self.telemetry.record_event(
                    V5ReceiptRuntimeEventKind::TaskStoreWorkingReadback,
                    epoch_ms,
                );
            }
            #[cfg(feature = "receipt-ledger-test-support")]
            if let Some(control) = &self.scenario_control {
                control.record_bound_task(task_record.clone(), bound.clone());
                if task_record.task == V5StoredTask::Working
                    && bound.phase() == AttemptPhase::NotBegun
                {
                    control.pause(
                        receipt_scenario_v5::ScenarioBarrierPoint::AfterWorkingReadback,
                        deadline,
                    )?;
                    control.pause(
                        receipt_scenario_v5::ScenarioBarrierPoint::BeforeReceiptBegun,
                        deadline,
                    )?;
                    if control.process_exited() {
                        let current = self.receipt_ledger.recover(bound.key().clone(), deadline)?;
                        return self.reply_for_existing_state(current, deadline);
                    }
                }
            }
            let bound = if task_record.task == V5StoredTask::Working
                && bound.phase() == AttemptPhase::NotBegun
            {
                let begun = self
                    .task_projection
                    .mark_not_begun_bound_task_begun(&bound, &task_record, deadline)
                    .map_err(|failure| self.project_task_failure(failure))?;
                #[cfg(feature = "receipt-ledger-test-support")]
                {
                    self.telemetry
                        .record_event(V5ReceiptRuntimeEventKind::ReceiptBegunCommitted, epoch_ms);
                    self.telemetry
                        .record_event(V5ReceiptRuntimeEventKind::TokenSignalled, epoch_ms);
                    if let Some(control) = &self.scenario_control {
                        control.record_bound_task_start_authorization(
                            &authorized_bound,
                            &task_record,
                            &begun,
                        );
                        control.record_bound_task(task_record.clone(), begun.clone());
                    }
                }
                begun
            } else {
                bound
            };
            #[cfg(feature = "receipt-ledger-test-support")]
            if scenario_gate_acquired {
                if let Some(control) = &self.scenario_control {
                    control.release_lifecycle_gate("submit");
                    control.wait_for_gate_cancel(deadline)?;
                    if let Some(cancelled) = self
                        .task_projection
                        .cancel_exact_bound_task(bound.key(), deadline)
                        .map_err(|failure| self.project_task_failure(failure))?
                    {
                        task_record = cancelled;
                        control.record_bound_task(task_record.clone(), bound.clone());
                    }
                }
            }
            if task_record.cancel_requested {
                let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Cancelled)
                    .map_err(|_| ReceiptLedgerError::Corrupt("canonical v5 terminal failed"))?;
                let (terminal_record, terminal_link) = self
                    .task_projection
                    .publish_bound_task_terminal(&bound, task_record, &terminal, epoch_ms, deadline)
                    .map_err(|failure| self.project_task_failure(failure))?;
                #[cfg(feature = "receipt-ledger-test-support")]
                {
                    self.telemetry.record_event(
                        V5ReceiptRuntimeEventKind::TaskStoreTerminalCommitted,
                        epoch_ms,
                    );
                    self.telemetry.record_event(
                        V5ReceiptRuntimeEventKind::TaskStoreTerminalReadback,
                        epoch_ms,
                    );
                    self.telemetry.record_event(
                        V5ReceiptRuntimeEventKind::TaskTerminalBoundCommitted,
                        epoch_ms,
                    );
                    if let Some(control) = &self.scenario_control {
                        control.record_terminal_bound_task(terminal_record.clone(), terminal_link);
                    }
                }
                return Ok(V5RuntimeReply::Json(V5ServerResponse::Invocation {
                    outcome: V5InvocationResponse::Task {
                        snapshot: task_store_snapshot(&terminal_record),
                    },
                }));
            }
            #[cfg(feature = "receipt-ledger-test-support")]
            {
                if let Some(control) = &self.scenario_control {
                    control.pause(
                        receipt_scenario_v5::ScenarioBarrierPoint::BeforePrepare,
                        deadline,
                    )?;
                }
                self.telemetry.record_prepare();
                self.telemetry
                    .record_event(V5ReceiptRuntimeEventKind::PrepareEntered, epoch_ms);
                if let Some(control) = &self.scenario_control {
                    control.pause(
                        receipt_scenario_v5::ScenarioBarrierPoint::PrepareEntered,
                        deadline,
                    )?;
                    if control.prepare_rejects() {
                        let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Completed {
                            result: Box::new(
                                crate::domain::invocation::DomainResult::canonical_rejection(
                                    None,
                                    "bad_value",
                                    "scenario prepare rejected invocation",
                                ),
                            ),
                        })
                        .map_err(|_| ReceiptLedgerError::Corrupt("canonical v5 terminal failed"))?;
                        return self.publish_bound_terminal_reply(
                            &bound,
                            task_record,
                            &terminal,
                            epoch_ms,
                            deadline,
                        );
                    }
                }
            }
            let prepared = match actor_bound.prepare() {
                Ok(prepared) => prepared,
                Err(result) => {
                    let terminal =
                        canonical_v5_terminal(&ReceiptTerminalOutcome::Completed { result })
                            .map_err(|_| {
                                ReceiptLedgerError::Corrupt("canonical v5 terminal failed")
                            })?;
                    return self.publish_bound_terminal_reply(
                        &bound,
                        task_record,
                        &terminal,
                        epoch_ms,
                        deadline,
                    );
                }
            };
            if matches!(
                prepared.execution_class(),
                crate::application::operation_descriptors::ExecutionClass::KnownLong(_)
            ) {
                return Ok(V5RuntimeReply::Json(V5ServerResponse::Invocation {
                    outcome: V5InvocationResponse::Task {
                        snapshot: task_store_snapshot(&task_record),
                    },
                }));
            }
            #[cfg(feature = "receipt-ledger-test-support")]
            {
                self.telemetry.record_execute();
                self.telemetry
                    .record_event(V5ReceiptRuntimeEventKind::ExecuteEntered, epoch_ms);
            }
            let outcome = match prepared.execute() {
                Ok(result) => ReceiptTerminalOutcome::Completed {
                    result: Box::new(result),
                },
                Err(_) => ReceiptTerminalOutcome::Failed {
                    reason: V5SafeFailureReason::InvocationFailed,
                },
            };
            #[cfg(feature = "receipt-ledger-test-support")]
            if self
                .scenario_control
                .as_ref()
                .is_some_and(|control| control.take_crash_after_side_effect())
            {
                return Err(ReceiptLedgerError::StoreUnavailable);
            }
            let terminal = canonical_v5_terminal(&outcome)
                .map_err(|_| ReceiptLedgerError::Corrupt("canonical v5 terminal failed"))?;
            #[cfg(feature = "receipt-ledger-test-support")]
            self.telemetry
                .record_event(V5ReceiptRuntimeEventKind::ResultSerialized, epoch_ms);
            return self.publish_bound_terminal_reply(
                &bound,
                task_record,
                &terminal,
                epoch_ms,
                deadline,
            );
        }
        #[cfg(feature = "receipt-ledger-test-support")]
        if let Some(control) = &self.scenario_control {
            control.record_actor_workspace_identity(actor_bound.workspace_identity_hash().clone());
        }
        let bound = self.receipt_ledger.bind_reserved_actor(
            reservation.key().clone(),
            reservation.record_version(),
            actor_bound.workspace_identity_hash().clone(),
            deadline,
        )?;
        #[cfg(feature = "receipt-ledger-test-support")]
        self.telemetry
            .record_event(V5ReceiptRuntimeEventKind::ActorBoundCommitted, epoch_ms);
        #[cfg(feature = "receipt-ledger-test-support")]
        if let Some(control) = &self.scenario_control {
            control.pause(
                receipt_scenario_v5::ScenarioBarrierPoint::ActorBound,
                deadline,
            )?;
            control.pause(
                receipt_scenario_v5::ScenarioBarrierPoint::BeforeReceiptBegun,
                deadline,
            )?;
            let current = self.receipt_ledger.recover(bound.key().clone(), deadline)?;
            if control.process_exited() {
                return self.reply_for_existing_state(current, deadline);
            }
            if let ReceiptState::TaskHandoffActorBound(handoff) = current {
                let (task_record, task_bound) = self
                    .task_projection
                    .materialize_bound_handoff(&handoff, epoch_ms, deadline, &self.telemetry)
                    .map_err(|failure| self.project_task_failure(failure))?;
                self.receipt_ledger.complete_bound_task_handoff(
                    handoff.key().clone(),
                    handoff.record_version(),
                    task_bound.clone(),
                    deadline,
                )?;
                control.record_bound_task(task_record.clone(), task_bound);
                self.telemetry
                    .record_event(V5ReceiptRuntimeEventKind::TaskBoundCommitted, epoch_ms);
                return Ok(V5RuntimeReply::Json(V5ServerResponse::Invocation {
                    outcome: V5InvocationResponse::Task {
                        snapshot: task_store_snapshot(&task_record),
                    },
                }));
            }
        }
        let begun = self.receipt_ledger.mark_reserved_begun(
            bound.key().clone(),
            bound.record_version(),
            deadline,
        )?;
        #[cfg(feature = "receipt-ledger-test-support")]
        self.telemetry
            .record_event(V5ReceiptRuntimeEventKind::ReceiptBegunCommitted, epoch_ms);
        #[cfg(feature = "receipt-ledger-test-support")]
        {
            if let Some(control) = &self.scenario_control {
                control.pause(
                    receipt_scenario_v5::ScenarioBarrierPoint::BeforePrepare,
                    deadline,
                )?;
                let current = self.receipt_ledger.recover(begun.key().clone(), deadline)?;
                if control.process_exited() {
                    return self.reply_for_existing_state(current, deadline);
                }
                if let ReceiptState::TaskHandoffActorBound(handoff) = current {
                    let (task_record, task_bound) = self
                        .task_projection
                        .materialize_bound_handoff(&handoff, epoch_ms, deadline, &self.telemetry)
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
                    control.record_bound_task(task_record.clone(), task_bound);
                    self.telemetry
                        .record_event(V5ReceiptRuntimeEventKind::TaskBoundCommitted, epoch_ms);
                    return Ok(V5RuntimeReply::Json(V5ServerResponse::Invocation {
                        outcome: V5InvocationResponse::Task {
                            snapshot: task_store_snapshot(&task_record),
                        },
                    }));
                }
                if !matches!(
                    &current,
                    ReceiptState::Reserved(receipt)
                        if matches!(receipt.phase(), ReservedPhase::Begun { .. })
                ) {
                    return self.reply_for_existing_state(current, deadline);
                }
            }
            self.telemetry.record_prepare();
            self.telemetry
                .record_event(V5ReceiptRuntimeEventKind::PrepareEntered, epoch_ms);
            if let Some(control) = &self.scenario_control {
                control.pause(
                    receipt_scenario_v5::ScenarioBarrierPoint::PrepareEntered,
                    deadline,
                )?;
                let current = self.receipt_ledger.recover(begun.key().clone(), deadline)?;
                if control.process_exited() {
                    return self.reply_for_existing_state(current, deadline);
                }
                if let ReceiptState::TaskHandoffActorBound(handoff) = current {
                    let (task_record, task_bound) = self
                        .task_projection
                        .materialize_bound_handoff(&handoff, epoch_ms, deadline, &self.telemetry)
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
                    control.record_bound_task(task_record.clone(), task_bound);
                    self.telemetry
                        .record_event(V5ReceiptRuntimeEventKind::TaskBoundCommitted, epoch_ms);
                    return Ok(V5RuntimeReply::Json(V5ServerResponse::Invocation {
                        outcome: V5InvocationResponse::Task {
                            snapshot: task_store_snapshot(&task_record),
                        },
                    }));
                }
                if !matches!(
                    &current,
                    ReceiptState::Reserved(receipt)
                        if matches!(receipt.phase(), ReservedPhase::Begun { .. })
                ) {
                    return self.reply_for_existing_state(current, deadline);
                }
                if control.prepare_rejects() {
                    let terminal = crate::application::receipt_ledger::canonical_v5_terminal(
                        &crate::application::receipt_ledger::ReceiptTerminalOutcome::Completed {
                            result: Box::new(
                                crate::domain::invocation::DomainResult::canonical_rejection(
                                    None,
                                    "bad_value",
                                    "scenario prepare rejected invocation",
                                ),
                            ),
                        },
                    )
                    .map_err(|_| ReceiptLedgerError::Corrupt("canonical v5 terminal failed"))?;
                    return self.publish_direct_terminal(begun, epoch_ms, terminal, deadline);
                }
            }
        }
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
            let handoff = self.receipt_ledger.begin_bound_task_handoff(
                begun.key().clone(),
                begun.record_version(),
                epoch_ms,
                DIRECT_TERMINAL_RETENTION_MS,
                V5_TASK_POLL_INTERVAL_MS,
                deadline,
            )?;
            #[cfg(feature = "receipt-ledger-test-support")]
            self.telemetry
                .record_event(V5ReceiptRuntimeEventKind::BoundHandoffCommitted, epoch_ms);
            let (task_record, bound) = self
                .task_projection
                .materialize_bound_handoff(
                    &handoff,
                    epoch_ms,
                    deadline,
                    #[cfg(feature = "receipt-ledger-test-support")]
                    &self.telemetry,
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
            #[cfg(feature = "receipt-ledger-test-support")]
            if let Some(control) = &self.scenario_control {
                control.record_bound_task(task_record.clone(), bound.clone());
            }
            #[cfg(feature = "receipt-ledger-test-support")]
            self.telemetry
                .record_event(V5ReceiptRuntimeEventKind::TaskBoundCommitted, epoch_ms);
            #[cfg(feature = "receipt-ledger-test-support")]
            if let Some(control) = &self.scenario_control {
                if control.is_barrier_installed(
                    receipt_scenario_v5::ScenarioBarrierPoint::BeforeTaskTerminalReceipt,
                ) {
                    control.pause(
                        receipt_scenario_v5::ScenarioBarrierPoint::BeforeTaskTerminalReceipt,
                        deadline,
                    )?;
                    self.telemetry.record_execute();
                    self.telemetry
                        .record_event(V5ReceiptRuntimeEventKind::ExecuteEntered, epoch_ms);
                    let outcome = match prepared.execute() {
                        Ok(result) => ReceiptTerminalOutcome::Completed {
                            result: Box::new(result),
                        },
                        Err(_) => ReceiptTerminalOutcome::Failed {
                            reason: V5SafeFailureReason::InvocationFailed,
                        },
                    };
                    let terminal = canonical_v5_terminal(&outcome)
                        .map_err(|_| ReceiptLedgerError::Corrupt("canonical v5 terminal failed"))?;
                    self.telemetry
                        .record_event(V5ReceiptRuntimeEventKind::ResultSerialized, epoch_ms);
                    return self.publish_bound_terminal_reply(
                        &bound,
                        task_record,
                        &terminal,
                        epoch_ms,
                        deadline,
                    );
                }
            }
            return Ok(V5RuntimeReply::Json(V5ServerResponse::Invocation {
                outcome: V5InvocationResponse::Task {
                    snapshot: task_store_snapshot(&task_record),
                },
            }));
        }
        #[cfg(feature = "receipt-ledger-test-support")]
        {
            self.telemetry.record_execute();
            self.telemetry
                .record_event(V5ReceiptRuntimeEventKind::ExecuteEntered, epoch_ms);
        }
        let outcome = match prepared.execute() {
            Ok(result) => crate::application::receipt_ledger::ReceiptTerminalOutcome::Completed {
                result: Box::new(result),
            },
            Err(_) => crate::application::receipt_ledger::ReceiptTerminalOutcome::Failed {
                reason: V5SafeFailureReason::InvocationFailed,
            },
        };
        #[cfg(feature = "receipt-ledger-test-support")]
        if self
            .scenario_control
            .as_ref()
            .is_some_and(|control| control.take_crash_after_side_effect())
        {
            return Err(ReceiptLedgerError::StoreUnavailable);
        }
        let terminal = crate::application::receipt_ledger::canonical_v5_terminal(&outcome)
            .map_err(|_| ReceiptLedgerError::Corrupt("canonical v5 terminal failed"))?;
        #[cfg(feature = "receipt-ledger-test-support")]
        self.telemetry
            .record_event(V5ReceiptRuntimeEventKind::ResultSerialized, epoch_ms);
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
            .record_event(V5ReceiptRuntimeEventKind::FinalResultProjected, epoch_ms);
        #[cfg(feature = "receipt-ledger-test-support")]
        self.telemetry
            .record_direct_publication(&publication, "direct", "immediate_publication");
        Ok(V5RuntimeReply::Prepared(publication.into_parts().1))
    }

    fn publish_bound_terminal_reply(
        &self,
        bound: &TaskBoundReceipt,
        task_record: V5StoredInvocationRecord,
        terminal: &crate::application::receipt_ledger::V5CanonicalTerminal,
        epoch_ms: u64,
        deadline: Instant,
    ) -> Result<V5RuntimeReply, ReceiptLedgerError> {
        let (terminal_record, terminal_link) = self
            .task_projection
            .publish_bound_task_terminal(bound, task_record, terminal, epoch_ms, deadline)
            .map_err(|failure| self.project_task_failure(failure))?;
        #[cfg(feature = "receipt-ledger-test-support")]
        {
            self.telemetry.record_event(
                V5ReceiptRuntimeEventKind::TaskStoreTerminalCommitted,
                epoch_ms,
            );
            self.telemetry.record_event(
                V5ReceiptRuntimeEventKind::TaskStoreTerminalReadback,
                epoch_ms,
            );
            self.telemetry.record_event(
                V5ReceiptRuntimeEventKind::TaskTerminalBoundCommitted,
                epoch_ms,
            );
            if let Some(control) = &self.scenario_control {
                control.record_terminal_bound_task(terminal_record.clone(), terminal_link);
            }
        }
        Ok(V5RuntimeReply::Json(V5ServerResponse::Invocation {
            outcome: V5InvocationResponse::Task {
                snapshot: task_store_snapshot(&terminal_record),
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
                #[cfg(feature = "receipt-ledger-test-support")]
                if let Some(control) = &self.scenario_control {
                    control
                        .record_receipt_backed_terminal(receipt)
                        .map_err(|_| {
                            ReceiptLedgerError::Corrupt(
                                "capture receipt-backed terminal evidence failed",
                            )
                        })?;
                }
                #[cfg(feature = "receipt-ledger-test-support")]
                self.telemetry.record_event(
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
                #[cfg(feature = "receipt-ledger-test-support")]
                self.telemetry.record_event(
                    V5ReceiptRuntimeEventKind::CancelReservationConverted,
                    epoch_ms,
                );
                #[cfg(feature = "receipt-ledger-test-support")]
                if let Some(control) = &self.scenario_control {
                    control.pause(
                        receipt_scenario_v5::ScenarioBarrierPoint::AfterCancelReservationConvertedBeforeTerminal,
                        deadline,
                    )?;
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
        if let Some(record) = self
            .task_projection
            .cancel_bound_task(task_id, deadline)
            .map_err(|failure| self.project_task_failure(failure))?
        {
            return Ok(task_store_snapshot(&record));
        }
        let state = self.receipt_ledger.resolve_task(task_id, deadline)?;
        let expected = match state {
            ReceiptState::TaskPromisedUnbound(receipt) => {
                TaskCancellationReceipt::PromisedUnbound(receipt)
            }
            ReceiptState::TaskPromisedActorBound(receipt) => {
                TaskCancellationReceipt::PromisedActorBound(receipt)
            }
            ReceiptState::TaskHandoffActorBound(receipt) => {
                TaskCancellationReceipt::HandoffActorBound(receipt)
            }
            ReceiptState::TaskTerminalReceiptBacked(receipt) => {
                return self.resolve_task(receipt.task().task_id(), deadline)
            }
            _ => return Err(ReceiptLedgerError::ReceiptRowPresentUnsupported),
        };
        let cancelled =
            self.receipt_ledger
                .request_task_cancel(expected.key().clone(), expected, deadline)?;
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
    JsonFailStop(V5ServerResponse),
    Prepared(PreparedWireFrame),
    PreparedFailStop(PreparedWireFrame),
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

#[cfg(feature = "receipt-ledger-test-support")]
pub(super) fn receipt_state_task_snapshot_for_test(
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
    let mut sessions = Vec::new();

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
                let session_runtime = Arc::clone(&runtime);
                let session_record = record.clone();
                sessions.push(thread::spawn(move || {
                    let _ = handle_probe_connection(stream, &session_record, &session_runtime);
                }));
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
    for session in sessions {
        let _ = session.join();
    }
    if restart_requested {
        // INV.APP.DAEMON-STORE-FAIL-STOP: keep both the PID-bound endpoint and
        // receipt authority alive until process death. A detached worker may
        // still be inside an uninterruptible adapter or syscall.
        #[cfg(not(feature = "receipt-ledger-test-support"))]
        std::mem::forget(runtime);
        #[cfg(feature = "receipt-ledger-test-support")]
        drop(runtime);
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
        V5ReceiptRuntimeEventKind::StrictEnvelopeParsed,
        runtime.epoch_ms(),
    );
    #[cfg(feature = "receipt-ledger-test-support")]
    runtime.telemetry.record_event(
        V5ReceiptRuntimeEventKind::V5ReceiptRuntimeEntered,
        runtime.epoch_ms(),
    );
    runtime.ensure_named_authority_before(deadlines.operation)?;
    let kind = decoded.request().kind();
    #[cfg(feature = "receipt-ledger-test-support")]
    let _actor_telemetry_lease =
        (kind == V5ClientRequestKind::SubmitInvocation).then(|| runtime.telemetry.actor_lease());
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
                    #[cfg(feature = "receipt-ledger-test-support")]
                    if runtime
                        .scenario_control
                        .as_ref()
                        .is_some_and(|control| control.take_submit_response_disconnect())
                    {
                        return Ok(());
                    }
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
        ReceiptLedgerError::CommitUncertain { .. } => V5DaemonErrorCode::DurabilityUncertain,
        ReceiptLedgerError::DeadlineExceeded => V5DaemonErrorCode::Overloaded,
        ReceiptLedgerError::AlreadyOwned => V5DaemonErrorCode::DuplicateLease,
        ReceiptLedgerError::TerminalMismatch | ReceiptLedgerError::ReceiptRowPresentUnsupported => {
            V5DaemonErrorCode::InvalidRequest
        }
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
        ReceiptRecordHeader, ReceiptState, ReceiptTaskProjection, ReceiptTerminalOutcome,
        ReceiptVersion, RequestIdentity, ReserveOutcome, ReservedPhase, ReservedReceipt,
        V5CanonicalTerminal, V5ToolIdentity, CANCEL_RESERVATION_TTL_MS,
        MAX_RECEIPT_ENTITLEMENT_BYTES,
    };
    use crate::domain::invocation::{InvocationId, SafeIdentityHash, TaskId};
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
    use sha2::{Digest, Sha256};
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpStream;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, Instant};

    #[test]
    fn promised_receipt_projects_the_exact_stable_queued_task() {
        let task = ReceiptTaskProjection::new(
            "11111111-1111-4111-8111-111111111111"
                .parse()
                .expect("valid TaskId"),
            "22222222-2222-4222-8222-222222222222"
                .parse()
                .expect("valid InvocationId"),
            1_000,
            1_000,
            3_600_000,
            250,
            1,
        )
        .expect("valid Task projection");
        let digest: crate::application::receipt_ledger::ReceiptKeyDigest =
            "33".repeat(32).parse().expect("valid receipt digest");

        let snapshot = queued_receipt_task_snapshot(&task, digest.clone(), true);

        assert_eq!(
            snapshot,
            super::super::protocol_v5::V5DaemonTaskSnapshot::Queued {
                task_id: task.task_id(),
                invocation_id: task.invocation_id(),
                receipt_key_digest: digest,
                created_at_epoch_ms: 1_000,
                updated_at_epoch_ms: 1_000,
                ttl_ms: 3_600_000,
                poll_interval_ms: 250,
                version: 1,
                cancel_requested: true,
            }
        );
    }

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

        assert!(
            matches!(
                response,
                Ok(V5ServerResponse::Invocation { .. })
                    | Ok(V5ServerResponse::Error {
                        code: V5DaemonErrorCode::StoreFailed
                    })
            ),
            "seven-second reserve must reach the next runtime transition: {response:?}"
        );
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
    fn fail_stop_transport_margin_starts_after_response_serialization() {
        struct DelayedResponse(V5ServerResponse);

        impl Serialize for DelayedResponse {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                thread::sleep(RESPONSE_SERIALIZATION_MARGIN + Duration::from_millis(10));
                self.0.serialize(serializer)
            }
        }

        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind response listener");
        let address = listener.local_addr().expect("response listener address");
        let reader = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept response stream");
            let mut line = String::new();
            BufReader::new(stream)
                .read_line(&mut line)
                .expect("read fail-stop response");
            line
        });
        let mut stream = TcpStream::connect(address).expect("connect response stream");
        let observed_at = Instant::now();
        let expired_response_deadline = observed_at
            .checked_sub(Duration::from_millis(1))
            .expect("response deadline can precede the observation");
        let response = DelayedResponse(V5ServerResponse::Error {
            code: V5DaemonErrorCode::DurabilityUncertain,
        });

        assert_eq!(
            fail_stop_response_write_timeout(expired_response_deadline, observed_at),
            RESPONSE_SERIALIZATION_MARGIN
        );
        write_fail_stop_json_line(&mut stream, &response, expired_response_deadline)
            .expect("serialized fail-stop response retains a fresh transport margin");
        assert_eq!(
            reader.join().expect("join response reader"),
            "{\"kind\":\"error\",\"code\":\"durability_uncertain\"}\n"
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
                    cancel_requested: true,
                },
            }) if receipt_key == key
        ));
    }

    #[test]
    fn startup_terminalizes_pre_task_receipts_without_replaying_domain_work() {
        for (phase, cancel_requested, expected) in [
            (
                ReservedPhase::Unbound,
                false,
                ReceiptTerminalOutcome::Failed {
                    reason: V5SafeFailureReason::Interrupted,
                },
            ),
            (
                ReservedPhase::ActorBound {
                    bound_workspace_identity: SafeIdentityHash::from_sha256(
                        Sha256::digest(b"startup-actor").into(),
                    ),
                },
                true,
                ReceiptTerminalOutcome::Cancelled,
            ),
            (
                ReservedPhase::Begun {
                    bound_workspace_identity: SafeIdentityHash::from_sha256(
                        Sha256::digest(b"startup-begun").into(),
                    ),
                },
                true,
                ReceiptTerminalOutcome::Failed {
                    reason: V5SafeFailureReason::OutcomeUncertain,
                },
            ),
        ] {
            let root = tempfile::tempdir().expect("temporary startup recovery root");
            let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
            let identity = CoreIdentity::production_v5();
            let state = DaemonStateDirectory::open(&state_root, &identity)
                .expect("open startup recovery daemon state");
            let config = DaemonServerConfig::new(
                state_root.clone(),
                identity.clone(),
                Duration::from_millis(50),
            );
            let runtime = V5ReceiptRuntime::open(&state, &config).expect("open initial runtime");
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
            let deadline = Instant::now() + Duration::from_secs(2);
            let reserved = runtime
                .receipt_ledger
                .reserve(
                    key.clone(),
                    OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid cutoff"),
                    deadline,
                )
                .expect("reserve startup receipt")
                .into_reservation()
                .expect("new startup receipt");
            let mut current_version = reserved.record_version();
            match phase {
                ReservedPhase::Unbound => {}
                ReservedPhase::ActorBound {
                    bound_workspace_identity,
                } => {
                    current_version = runtime
                        .receipt_ledger
                        .bind_reserved_actor(
                            key.clone(),
                            current_version,
                            bound_workspace_identity,
                            deadline,
                        )
                        .expect("bind startup actor")
                        .record_version();
                }
                ReservedPhase::Begun {
                    bound_workspace_identity,
                } => {
                    let bound = runtime
                        .receipt_ledger
                        .bind_reserved_actor(
                            key.clone(),
                            current_version,
                            bound_workspace_identity,
                            deadline,
                        )
                        .expect("bind begun startup actor");
                    runtime
                        .receipt_ledger
                        .mark_reserved_begun(key.clone(), bound.record_version(), deadline)
                        .expect("mark startup receipt begun");
                }
            }
            if cancel_requested {
                runtime
                    .receipt_ledger
                    .request_cancel_or_reserve(key.clone(), 2_000, deadline)
                    .expect("persist startup cancellation");
            }
            drop(runtime);

            let reopened = V5ReceiptRuntime::open(&state, &config).expect("reconcile startup");
            let recovered = reopened
                .receipt_ledger
                .recover(key, Instant::now() + Duration::from_secs(2))
                .expect("read reconciled startup receipt");
            let ReceiptState::DirectTerminalUnacked(receipt) = recovered else {
                panic!("startup must publish one direct terminal")
            };
            assert_eq!(receipt.terminal().outcome(), &expected);
        }
    }

    #[test]
    fn startup_terminalizes_unbound_promised_task_without_task_store_create() {
        for (cancel_requested, expected) in [
            (
                false,
                ReceiptTerminalOutcome::Failed {
                    reason: V5SafeFailureReason::Interrupted,
                },
            ),
            (true, ReceiptTerminalOutcome::Cancelled),
        ] {
            let root = tempfile::tempdir().expect("temporary promised recovery root");
            let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
            let identity = CoreIdentity::production_v5();
            let state = DaemonStateDirectory::open(&state_root, &identity)
                .expect("open promised recovery daemon state");
            let config = DaemonServerConfig::new(
                state_root.clone(),
                identity.clone(),
                Duration::from_millis(50),
            );
            let runtime = V5ReceiptRuntime::open(&state, &config).expect("open initial runtime");
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
            let deadline = Instant::now() + Duration::from_secs(2);
            let reserved = runtime
                .receipt_ledger
                .reserve(
                    key.clone(),
                    OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid cutoff"),
                    deadline,
                )
                .expect("reserve promised startup receipt")
                .into_reservation()
                .expect("new promised startup receipt");
            let promised = runtime
                .receipt_ledger
                .promise_task_unbound(
                    key.clone(),
                    reserved.record_version(),
                    1_007,
                    3_600_000,
                    V5_TASK_POLL_INTERVAL_MS,
                    deadline,
                )
                .expect("promise startup Task");
            if cancel_requested {
                runtime
                    .receipt_ledger
                    .request_task_cancel(
                        key.clone(),
                        TaskCancellationReceipt::PromisedUnbound(promised),
                        deadline,
                    )
                    .expect("persist promised Task cancellation");
            }
            drop(runtime);

            let reopened = V5ReceiptRuntime::open(&state, &config).expect("reconcile startup");
            let recovered = reopened
                .receipt_ledger
                .recover(key, Instant::now() + Duration::from_secs(2))
                .expect("read reconciled promised Task receipt");
            let ReceiptState::TaskTerminalReceiptBacked(receipt) = recovered else {
                panic!("startup must publish one receipt-backed Task terminal")
            };
            assert_eq!(receipt.terminal().outcome(), &expected);
            assert_eq!(reopened.task_projection.recovery.entries().len(), 0);
        }
    }

    #[test]
    fn startup_materializes_and_terminalizes_actor_bound_promised_task() {
        for cancel_requested in [false, true] {
            let root = tempfile::tempdir().expect("temporary actor-bound recovery root");
            let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
            let identity = CoreIdentity::production_v5();
            let state = DaemonStateDirectory::open(&state_root, &identity)
                .expect("open actor-bound recovery daemon state");
            let config = DaemonServerConfig::new(
                state_root.clone(),
                identity.clone(),
                Duration::from_millis(50),
            );
            let runtime = V5ReceiptRuntime::open(&state, &config).expect("open initial runtime");
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
            let deadline = Instant::now() + Duration::from_secs(2);
            let reserved = runtime
                .receipt_ledger
                .reserve(
                    key.clone(),
                    OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid cutoff"),
                    deadline,
                )
                .expect("reserve actor-bound startup receipt")
                .into_reservation()
                .expect("new actor-bound startup receipt");
            let promised = runtime
                .receipt_ledger
                .promise_task_unbound(
                    key.clone(),
                    reserved.record_version(),
                    1_007,
                    3_600_000,
                    V5_TASK_POLL_INTERVAL_MS,
                    deadline,
                )
                .expect("promise startup Task");
            let actor_bound = runtime
                .receipt_ledger
                .bind_promised_task_actor(
                    key.clone(),
                    promised.record_version(),
                    SafeIdentityHash::from_sha256(Sha256::digest(b"startup-actor").into()),
                    deadline,
                )
                .expect("bind promised startup Task actor");
            if cancel_requested {
                runtime
                    .receipt_ledger
                    .request_task_cancel(
                        key.clone(),
                        TaskCancellationReceipt::PromisedActorBound(actor_bound),
                        deadline,
                    )
                    .expect("persist actor-bound startup cancellation");
            }
            drop(runtime);

            let reopened = V5ReceiptRuntime::open(&state, &config).expect("reconcile startup");
            assert_eq!(
                reopened
                    .receipt_ledger
                    .recover(key.clone(), Instant::now() + Duration::from_secs(2)),
                Err(ReceiptLedgerError::ReceiptNotFound)
            );
            let snapshot = reopened
                .resolve_task(
                    key.reserved_task_id(),
                    Instant::now() + Duration::from_secs(2),
                )
                .expect("resolve recovered actor-bound Task");
            match (cancel_requested, snapshot) {
                (
                    false,
                    crate::infrastructure::daemon::protocol_v5::V5DaemonTaskSnapshot::Failed {
                        reason: V5SafeFailureReason::Interrupted,
                        cancel_requested: false,
                        ..
                    },
                )
                | (
                    true,
                    crate::infrastructure::daemon::protocol_v5::V5DaemonTaskSnapshot::Cancelled {
                        cancel_requested: true,
                        ..
                    },
                ) => {}
                (_, other) => panic!("unexpected recovered actor-bound Task: {other:?}"),
            }
        }
    }

    #[test]
    fn startup_materializes_handoff_without_replaying_begun_work() {
        for (phase, cancel_requested) in [
            (AttemptPhase::NotBegun, false),
            (AttemptPhase::NotBegun, true),
            (AttemptPhase::Begun, false),
            (AttemptPhase::Begun, true),
        ] {
            let root = tempfile::tempdir().expect("temporary handoff recovery root");
            let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
            let identity = CoreIdentity::production_v5();
            let state = DaemonStateDirectory::open(&state_root, &identity)
                .expect("open handoff recovery daemon state");
            let config = DaemonServerConfig::new(
                state_root.clone(),
                identity.clone(),
                Duration::from_millis(50),
            );
            let runtime = V5ReceiptRuntime::open(&state, &config).expect("open initial runtime");
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
            let deadline = Instant::now() + Duration::from_secs(2);
            let reserved = runtime
                .receipt_ledger
                .reserve(
                    key.clone(),
                    OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid cutoff"),
                    deadline,
                )
                .expect("reserve handoff startup receipt")
                .into_reservation()
                .expect("new handoff startup receipt");
            let bound = runtime
                .receipt_ledger
                .bind_reserved_actor(
                    key.clone(),
                    reserved.record_version(),
                    SafeIdentityHash::from_sha256(Sha256::digest(b"startup-handoff").into()),
                    deadline,
                )
                .expect("bind handoff startup actor");
            let version = match phase {
                AttemptPhase::NotBegun => bound.record_version(),
                AttemptPhase::Begun => runtime
                    .receipt_ledger
                    .mark_reserved_begun(key.clone(), bound.record_version(), deadline)
                    .expect("mark startup handoff begun")
                    .record_version(),
            };
            let handoff = runtime
                .receipt_ledger
                .begin_bound_task_handoff(
                    key.clone(),
                    version,
                    1_009,
                    3_600_000,
                    V5_TASK_POLL_INTERVAL_MS,
                    deadline,
                )
                .expect("persist startup Task handoff");
            if cancel_requested {
                runtime
                    .receipt_ledger
                    .request_task_cancel(
                        key.clone(),
                        TaskCancellationReceipt::HandoffActorBound(handoff),
                        deadline,
                    )
                    .expect("persist startup handoff cancellation");
            }
            drop(runtime);

            let reopened = V5ReceiptRuntime::open(&state, &config).expect("reconcile startup");
            assert_eq!(
                reopened
                    .receipt_ledger
                    .recover(key.clone(), Instant::now() + Duration::from_secs(2)),
                Err(ReceiptLedgerError::ReceiptNotFound)
            );
            let snapshot = reopened
                .resolve_task(
                    key.reserved_task_id(),
                    Instant::now() + Duration::from_secs(2),
                )
                .expect("resolve recovered handoff Task");
            match (phase, cancel_requested, snapshot) {
                (
                    AttemptPhase::NotBegun,
                    false,
                    crate::infrastructure::daemon::protocol_v5::V5DaemonTaskSnapshot::Failed {
                        reason: V5SafeFailureReason::Interrupted,
                        cancel_requested: false,
                        ..
                    },
                )
                | (
                    AttemptPhase::NotBegun,
                    true,
                    crate::infrastructure::daemon::protocol_v5::V5DaemonTaskSnapshot::Cancelled {
                        cancel_requested: true,
                        ..
                    },
                )
                | (
                    AttemptPhase::Begun,
                    false,
                    crate::infrastructure::daemon::protocol_v5::V5DaemonTaskSnapshot::Failed {
                        reason: V5SafeFailureReason::OutcomeUncertain,
                        cancel_requested: false,
                        ..
                    },
                )
                | (
                    AttemptPhase::Begun,
                    true,
                    crate::infrastructure::daemon::protocol_v5::V5DaemonTaskSnapshot::Failed {
                        reason: V5SafeFailureReason::OutcomeUncertain,
                        cancel_requested: true,
                        ..
                    },
                ) => {}
                (_, _, other) => panic!("unexpected recovered handoff Task: {other:?}"),
            }
        }
    }

    fn materialize_startup_task_bound(
        runtime: &V5ReceiptRuntime,
        identity: &CoreIdentity,
        phase: AttemptPhase,
    ) -> (ReceiptKey, V5StoredInvocationRecord, TaskBoundReceipt) {
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
        let deadline = Instant::now() + Duration::from_secs(2);
        let reserved = runtime
            .receipt_ledger
            .reserve(
                key.clone(),
                OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid cutoff"),
                deadline,
            )
            .expect("reserve materialized startup receipt")
            .into_reservation()
            .expect("new materialized startup receipt");
        let actor_bound = runtime
            .receipt_ledger
            .bind_reserved_actor(
                key.clone(),
                reserved.record_version(),
                SafeIdentityHash::from_sha256(Sha256::digest(b"materialized-startup").into()),
                deadline,
            )
            .expect("bind materialized startup actor");
        let receipt_version = match phase {
            AttemptPhase::NotBegun => actor_bound.record_version(),
            AttemptPhase::Begun => runtime
                .receipt_ledger
                .mark_reserved_begun(key.clone(), actor_bound.record_version(), deadline)
                .expect("mark materialized startup receipt begun")
                .record_version(),
        };
        let handoff = runtime
            .receipt_ledger
            .begin_bound_task_handoff(
                key.clone(),
                receipt_version,
                1_009,
                3_600_000,
                V5_TASK_POLL_INTERVAL_MS,
                deadline,
            )
            .expect("begin materialized startup handoff");
        let (record, task_bound) = runtime
            .task_projection
            .materialize_bound_handoff(
                &handoff,
                1_009,
                deadline,
                #[cfg(feature = "receipt-ledger-test-support")]
                &runtime.telemetry,
            )
            .unwrap_or_else(|failure| panic!("materialize startup TaskBound: {}", failure.error));
        let task_bound = runtime
            .receipt_ledger
            .complete_bound_task_handoff(
                key.clone(),
                handoff.record_version(),
                task_bound,
                deadline,
            )
            .expect("complete startup TaskBound ownership");
        (key, record, task_bound)
    }

    #[test]
    fn startup_terminalizes_already_materialized_not_begun_task_bound() {
        for cancel_requested in [false, true] {
            let root = tempfile::tempdir().expect("temporary materialized TaskBound root");
            let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
            let identity = CoreIdentity::production_v5();
            let state = DaemonStateDirectory::open(&state_root, &identity)
                .expect("open materialized TaskBound daemon state");
            let config = DaemonServerConfig::new(
                state_root.clone(),
                identity.clone(),
                Duration::from_millis(50),
            );
            let runtime = V5ReceiptRuntime::open(&state, &config).expect("open initial runtime");
            let (key, _record, _task_bound) =
                materialize_startup_task_bound(&runtime, &identity, AttemptPhase::NotBegun);
            if cancel_requested {
                runtime
                    .task_projection
                    .cancel_bound_task(
                        key.reserved_task_id(),
                        Instant::now() + Duration::from_secs(2),
                    )
                    .unwrap_or_else(|failure| {
                        panic!("request materialized Task cancellation: {}", failure.error)
                    })
                    .expect("materialized Task exists");
            }
            drop(runtime);

            let reopened = V5ReceiptRuntime::open(&state, &config)
                .expect("reconcile already materialized TaskBound");
            let snapshot = reopened
                .resolve_task(
                    key.reserved_task_id(),
                    Instant::now() + Duration::from_secs(2),
                )
                .expect("resolve reconciled materialized Task");
            match (cancel_requested, snapshot) {
                (
                    false,
                    crate::infrastructure::daemon::protocol_v5::V5DaemonTaskSnapshot::Failed {
                        reason: V5SafeFailureReason::Interrupted,
                        ..
                    },
                )
                | (
                    true,
                    crate::infrastructure::daemon::protocol_v5::V5DaemonTaskSnapshot::Cancelled {
                        cancel_requested: true,
                        ..
                    },
                ) => {}
                (_, other) => panic!("unexpected materialized TaskBound recovery: {other:?}"),
            }
        }
    }

    #[test]
    fn startup_terminalizes_exact_working_begun_task_bound_as_outcome_uncertain() {
        let root = tempfile::tempdir().expect("temporary begun TaskBound root");
        let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
        let identity = CoreIdentity::production_v5();
        let state = DaemonStateDirectory::open(&state_root, &identity)
            .expect("open begun TaskBound daemon state");
        let config = DaemonServerConfig::new(
            state_root.clone(),
            identity.clone(),
            Duration::from_millis(50),
        );
        let runtime = V5ReceiptRuntime::open(&state, &config).expect("open initial runtime");
        let (key, record, task_bound) =
            materialize_startup_task_bound(&runtime, &identity, AttemptPhase::Begun);
        let (_working, _working_bound) = runtime
            .task_projection
            .start_bound_task(&task_bound, record, Instant::now() + Duration::from_secs(2))
            .unwrap_or_else(|failure| panic!("start exact begun Task: {}", failure.error));
        drop(runtime);

        let reopened =
            V5ReceiptRuntime::open(&state, &config).expect("reconcile exact begun TaskBound");
        assert!(matches!(
            reopened
                .resolve_task(
                    key.reserved_task_id(),
                    Instant::now() + Duration::from_secs(2)
                )
                .expect("resolve reconciled begun Task"),
            crate::infrastructure::daemon::protocol_v5::V5DaemonTaskSnapshot::Failed {
                reason: V5SafeFailureReason::OutcomeUncertain,
                ..
            }
        ));
    }

    #[test]
    fn startup_rejects_queued_begun_task_bound_without_mutation() {
        let root = tempfile::tempdir().expect("temporary invalid begun TaskBound root");
        let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
        let identity = CoreIdentity::production_v5();
        let state = DaemonStateDirectory::open(&state_root, &identity)
            .expect("open invalid begun TaskBound daemon state");
        let config = DaemonServerConfig::new(
            state_root.clone(),
            identity.clone(),
            Duration::from_millis(50),
        );
        let runtime = V5ReceiptRuntime::open(&state, &config).expect("open initial runtime");
        let (key, queued, _task_bound) =
            materialize_startup_task_bound(&runtime, &identity, AttemptPhase::Begun);
        drop(runtime);

        let error = match V5ReceiptRuntime::open(&state, &config) {
            Ok(_) => panic!("queued begun TaskBound must fail-stop startup"),
            Err(error) => error,
        };
        assert!(error.contains("TaskBound Begun requires exact Working Task"));

        let task_root = RetainedDirectoryCapability::open(&state.path().join("tasks"))
            .expect("retain TaskStore after failed startup");
        let (store, recovery) = FileInvocationStoreV5::open_retained_directory_inspect_only(
            task_root,
            Arc::new(SystemEpochMillisClock),
            crate::domain::code_intelligence::ProviderDeadline::new(
                Instant::now() + Duration::from_secs(2),
            ),
        )
        .expect("inspect TaskStore after failed startup");
        assert_eq!(
            store
                .get(
                    key.reserved_task_id(),
                    crate::domain::code_intelligence::ProviderDeadline::new(
                        Instant::now() + Duration::from_secs(2),
                    ),
                )
                .expect("read unchanged queued Task"),
            queued
        );
        assert_eq!(recovery.entries().len(), 1);
    }

    #[test]
    fn startup_rejects_active_task_without_lifecycle_link_without_mutation() {
        let root = tempfile::tempdir().expect("temporary orphan Task root");
        let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
        let identity = CoreIdentity::production_v5();
        let state = DaemonStateDirectory::open(&state_root, &identity)
            .expect("open orphan Task daemon state");
        let config = DaemonServerConfig::new(
            state_root.clone(),
            identity.clone(),
            Duration::from_millis(50),
        );
        let runtime = V5ReceiptRuntime::open(&state, &config).expect("open initial runtime");
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
        let workspace_identity =
            SafeIdentityHash::from_sha256(Sha256::digest(b"orphan-startup").into());
        let orphan = runtime
            .task_projection
            .task_store
            .create_exact(
                NewV5InvocationRecord::new(
                    V5TaskIdentity::new(
                        key.reserved_task_id(),
                        key.invocation_id(),
                        receipt_key_digest(&key),
                    ),
                    key.tool(),
                    key.normalized_arguments_hash().clone(),
                    workspace_identity,
                    V5_TASK_POLL_INTERVAL_MS,
                    3_600_000,
                )
                .with_initial_epoch_ms(1_009),
                crate::domain::code_intelligence::ProviderDeadline::new(
                    Instant::now() + Duration::from_secs(2),
                ),
            )
            .expect("create orphan startup Task");
        drop(runtime);

        let error = match V5ReceiptRuntime::open(&state, &config) {
            Ok(_) => panic!("active Task without lifecycle link must fail-stop startup"),
            Err(error) => error,
        };
        assert!(error.contains("active Task has no exact lifecycle link"));

        let task_root = RetainedDirectoryCapability::open(&state.path().join("tasks"))
            .expect("retain orphan TaskStore after failed startup");
        let (store, recovery) = FileInvocationStoreV5::open_retained_directory_inspect_only(
            task_root,
            Arc::new(SystemEpochMillisClock),
            crate::domain::code_intelligence::ProviderDeadline::new(
                Instant::now() + Duration::from_secs(2),
            ),
        )
        .expect("inspect orphan TaskStore after failed startup");
        assert_eq!(
            store
                .get(
                    key.reserved_task_id(),
                    crate::domain::code_intelligence::ProviderDeadline::new(
                        Instant::now() + Duration::from_secs(2),
                    ),
                )
                .expect("read unchanged orphan Task"),
            orphan
        );
        assert_eq!(recovery.entries().len(), 1);
    }

    #[test]
    fn startup_receipt_loop_completes_exact_reserved_link_with_preexisting_queued_task() {
        let root = tempfile::tempdir().expect("temporary preexisting handoff root");
        let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
        let identity = CoreIdentity::production_v5();
        let state = DaemonStateDirectory::open(&state_root, &identity)
            .expect("open preexisting handoff daemon state");
        let config = DaemonServerConfig::new(
            state_root.clone(),
            identity.clone(),
            Duration::from_millis(50),
        );
        let runtime = V5ReceiptRuntime::open(&state, &config).expect("open initial runtime");
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
        let deadline = Instant::now() + Duration::from_secs(2);
        let reserved = runtime
            .receipt_ledger
            .reserve(
                key.clone(),
                OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid cutoff"),
                deadline,
            )
            .expect("reserve preexisting handoff receipt")
            .into_reservation()
            .expect("new preexisting handoff receipt");
        let workspace_identity =
            SafeIdentityHash::from_sha256(Sha256::digest(b"preexisting-handoff").into());
        let actor_bound = runtime
            .receipt_ledger
            .bind_reserved_actor(
                key.clone(),
                reserved.record_version(),
                workspace_identity.clone(),
                deadline,
            )
            .expect("bind preexisting handoff actor");
        let handoff = runtime
            .receipt_ledger
            .begin_bound_task_handoff(
                key.clone(),
                actor_bound.record_version(),
                1_009,
                3_600_000,
                V5_TASK_POLL_INTERVAL_MS,
                deadline,
            )
            .expect("begin preexisting handoff");
        runtime
            .task_projection
            .lifecycle_links
            .reserve_task_link(
                key.clone(),
                handoff.link().clone(),
                crate::domain::code_intelligence::ProviderDeadline::new(deadline),
            )
            .expect("reserve exact preexisting Task link");
        runtime
            .task_projection
            .task_store
            .create_exact(
                NewV5InvocationRecord::new(
                    V5TaskIdentity::new(
                        key.reserved_task_id(),
                        key.invocation_id(),
                        receipt_key_digest(&key),
                    ),
                    key.tool(),
                    key.normalized_arguments_hash().clone(),
                    workspace_identity,
                    V5_TASK_POLL_INTERVAL_MS,
                    3_600_000,
                )
                .with_initial_epoch_ms(1_009),
                crate::domain::code_intelligence::ProviderDeadline::new(deadline),
            )
            .expect("create exact preexisting queued Task");
        drop(runtime);

        let reopened = V5ReceiptRuntime::open(&state, &config)
            .expect("receipt loop completes preexisting handoff");
        assert!(matches!(
            reopened
                .resolve_task(
                    key.reserved_task_id(),
                    Instant::now() + Duration::from_secs(2)
                )
                .expect("resolve preexisting handoff Task"),
            crate::infrastructure::daemon::protocol_v5::V5DaemonTaskSnapshot::Failed {
                reason: V5SafeFailureReason::Interrupted,
                ..
            }
        ));
    }

    #[test]
    fn startup_rejects_preexisting_handoff_task_without_prior_link_reservation() {
        let root = tempfile::tempdir().expect("temporary missing handoff reservation root");
        let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
        let identity = CoreIdentity::production_v5();
        let state = DaemonStateDirectory::open(&state_root, &identity)
            .expect("open missing handoff reservation daemon state");
        let config = DaemonServerConfig::new(
            state_root.clone(),
            identity.clone(),
            Duration::from_millis(50),
        );
        let runtime = V5ReceiptRuntime::open(&state, &config).expect("open initial runtime");
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
        let deadline = Instant::now() + Duration::from_secs(2);
        let reserved = runtime
            .receipt_ledger
            .reserve(
                key.clone(),
                OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid cutoff"),
                deadline,
            )
            .expect("reserve missing-reservation handoff receipt")
            .into_reservation()
            .expect("new missing-reservation handoff receipt");
        let workspace_identity =
            SafeIdentityHash::from_sha256(Sha256::digest(b"missing-handoff-reservation").into());
        let actor_bound = runtime
            .receipt_ledger
            .bind_reserved_actor(
                key.clone(),
                reserved.record_version(),
                workspace_identity.clone(),
                deadline,
            )
            .expect("bind missing-reservation handoff actor");
        runtime
            .receipt_ledger
            .begin_bound_task_handoff(
                key.clone(),
                actor_bound.record_version(),
                1_009,
                3_600_000,
                V5_TASK_POLL_INTERVAL_MS,
                deadline,
            )
            .expect("begin missing-reservation handoff");
        let queued = runtime
            .task_projection
            .task_store
            .create_exact(
                NewV5InvocationRecord::new(
                    V5TaskIdentity::new(
                        key.reserved_task_id(),
                        key.invocation_id(),
                        receipt_key_digest(&key),
                    ),
                    key.tool(),
                    key.normalized_arguments_hash().clone(),
                    workspace_identity,
                    V5_TASK_POLL_INTERVAL_MS,
                    3_600_000,
                )
                .with_initial_epoch_ms(1_009),
                crate::domain::code_intelligence::ProviderDeadline::new(deadline),
            )
            .expect("create preexisting handoff Task without reservation");
        drop(runtime);

        let error = match V5ReceiptRuntime::open(&state, &config) {
            Ok(_) => panic!("handoff Task without prior reservation must fail-stop startup"),
            Err(error) => error,
        };
        assert!(error.contains("preexisting handoff Task has no exact prior link reservation"));

        let task_root = RetainedDirectoryCapability::open(&state.path().join("tasks"))
            .expect("retain handoff TaskStore after failed startup");
        let (store, _) = FileInvocationStoreV5::open_retained_directory_inspect_only(
            task_root,
            Arc::new(SystemEpochMillisClock),
            crate::domain::code_intelligence::ProviderDeadline::new(
                Instant::now() + Duration::from_secs(2),
            ),
        )
        .expect("inspect handoff TaskStore after failed startup");
        assert_eq!(
            store
                .get(
                    key.reserved_task_id(),
                    crate::domain::code_intelligence::ProviderDeadline::new(
                        Instant::now() + Duration::from_secs(2),
                    ),
                )
                .expect("read unchanged handoff Task"),
            queued
        );
        let links = TaskLifecycleLinkStoreV5::open(
            state.path().join("task-lifecycle-links"),
            crate::domain::code_intelligence::ProviderDeadline::new(
                Instant::now() + Duration::from_secs(2),
            ),
        )
        .expect("inspect lifecycle store after failed startup")
        .catalog_snapshot(crate::domain::code_intelligence::ProviderDeadline::new(
            Instant::now() + Duration::from_secs(2),
        ))
        .expect("snapshot unchanged lifecycle store");
        assert!(links.entries().is_empty());
    }

    #[test]
    fn startup_rejects_task_terminal_bound_that_does_not_confirm_exact_terminal_task() {
        let root = tempfile::tempdir().expect("temporary terminal mismatch root");
        let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
        let identity = CoreIdentity::production_v5();
        let state = DaemonStateDirectory::open(&state_root, &identity)
            .expect("open terminal mismatch daemon state");
        let config = DaemonServerConfig::new(
            state_root.clone(),
            identity.clone(),
            Duration::from_millis(50),
        );
        let runtime = V5ReceiptRuntime::open(&state, &config).expect("open initial runtime");
        let (key, queued, task_bound) =
            materialize_startup_task_bound(&runtime, &identity, AttemptPhase::NotBegun);
        let deadline = crate::domain::code_intelligence::ProviderDeadline::new(
            Instant::now() + Duration::from_secs(2),
        );
        let terminal = runtime
            .task_projection
            .task_store
            .terminalize_recovered_exact(
                &queued.identity(),
                queued.version,
                RecoveryTerminalReason::InterruptedBeforeExecution,
                deadline,
            )
            .expect("terminalize exact TaskStore record");
        let V5StoredTask::Failed {
            terminal_epoch_ms, ..
        } = &terminal.task
        else {
            panic!("recovery terminal must be Failed")
        };
        let wrong_digest: TerminalDigest = "ff".repeat(32).parse().expect("wrong terminal digest");
        runtime
            .task_projection
            .lifecycle_links
            .publish_task_terminal_bound(
                &task_bound,
                receipt_task_projection_from_store(&terminal).unwrap_or_else(|failure| {
                    panic!("project exact terminal Task: {}", failure.error)
                }),
                terminal.version,
                ClosedTerminalStatus::Failed,
                wrong_digest,
                *terminal_epoch_ms,
                deadline,
            )
            .expect("publish deliberately mismatched TaskTerminalBound");
        drop(runtime);

        let error = match V5ReceiptRuntime::open(&state, &config) {
            Ok(_) => panic!("mismatched TaskTerminalBound must fail-stop startup"),
            Err(error) => error,
        };
        assert!(error.contains("TaskTerminalBound does not confirm the exact terminal Task"));

        let task_root = RetainedDirectoryCapability::open(&state.path().join("tasks"))
            .expect("retain terminal TaskStore after failed startup");
        let (store, _) = FileInvocationStoreV5::open_retained_directory_inspect_only(
            task_root,
            Arc::new(SystemEpochMillisClock),
            crate::domain::code_intelligence::ProviderDeadline::new(
                Instant::now() + Duration::from_secs(2),
            ),
        )
        .expect("inspect terminal TaskStore after failed startup");
        assert_eq!(
            store
                .get(
                    key.reserved_task_id(),
                    crate::domain::code_intelligence::ProviderDeadline::new(
                        Instant::now() + Duration::from_secs(2),
                    ),
                )
                .expect("read unchanged terminal Task"),
            terminal
        );
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
        if let Err(error) = stream.read_to_end(&mut response) {
            assert_eq!(
                error.kind(),
                io::ErrorKind::ConnectionReset,
                "expired handshake must close the transport: {error}"
            );
        }

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
