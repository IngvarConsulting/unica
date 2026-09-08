//! The contract harness's view of the protocol-v5 runtime: telemetry that
//! records every hook, leases that count listeners and actors, and the
//! writers the scenario runner still applies to the ledger itself.

use super::super::hooks::{
    V5AdmissionRejection, V5PausePoint, V5ReceiptRuntimeEventKind, V5RuntimeHooks, V5Stage,
    V5StoreFaultPoint,
};
use super::super::V5ReceiptRuntime;
use super::{ReceiptScenarioControl, SCENARIO_BULK_OPERATION_TIMEOUT};
use crate::application::invocation_store_v5::V5StoredInvocationRecord;
use crate::application::receipt_ledger::{
    AcknowledgedTombstoneReceipt, CommittedDirectPublication, ReceiptKey, ReceiptLedgerError,
    TaskBoundReceipt, TaskHandoffActorBoundReceipt, TaskPromisedActorBoundReceipt,
    TaskPromisedUnboundReceipt, TaskTerminalBoundReceipt, TaskTerminalReceiptBackedReceipt,
    TerminalDigest, V5CanonicalTerminal,
};
use crate::application::receipt_ledger_actor::ReceiptLedgerActor;
use crate::domain::invocation::{DomainResult, InvocationId, SafeIdentityHash};
use crate::infrastructure::platform::filesystem::RetainedDirectoryCapability;
use crate::infrastructure::receipt_ledger::{
    canonical_staged_transfer_certificate, ReceiptBackedTaskTerminalSeed, ReceiptLedgerStore,
};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::any::Any;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct V5ReceiptRuntimeEvent {
    pub(super) sequence: u64,
    pub(super) monotonic_ms: u64,
    pub(super) epoch_ms: u64,
    pub(super) event: V5ReceiptRuntimeEventKind,
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct V5ReceiptRuntimeCallbackCounts {
    pub(super) validation: u64,
    pub(super) admission: u64,
    pub(super) prepare: u64,
    pub(super) execute: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum V5ReceiptRuntimeListenerState {
    NotPublished,
    Listening,
    Closed,
}

pub(super) struct V5ReceiptRuntimeTelemetryState {
    pub(super) next_sequence: u64,
    pub(super) wait_floor_sequence: u64,
    pub(super) events: Vec<V5ReceiptRuntimeEvent>,
    pub(super) callbacks: V5ReceiptRuntimeCallbackCounts,
    pub(super) listener: V5ReceiptRuntimeListenerState,
    pub(super) active_listeners: u64,
    pub(super) daemon_running: bool,
    pub(super) restart_requested: bool,
    pub(super) actor_leases: u64,
    pub(super) terminal_publications: Vec<Value>,
    pub(super) next_preflight_sequence: u64,
    pub(super) task_store_create_attempts: u64,
    pub(super) store_fault: Option<V5StoreFaultPoint>,
}

#[derive(Clone)]
pub(super) struct V5ReceiptRuntimeTelemetrySnapshot {
    pub(super) events: Vec<V5ReceiptRuntimeEvent>,
    pub(super) callbacks: V5ReceiptRuntimeCallbackCounts,
    pub(super) listener: V5ReceiptRuntimeListenerState,
    pub(super) daemon_running: bool,
    pub(super) restart_requested: bool,
    pub(super) actor_leases: u64,
    pub(super) terminal_publications: Vec<Value>,
    pub(super) task_store_create_attempts: u64,
}

pub(super) struct V5ReceiptRuntimeTelemetry {
    pub(super) started_at: Instant,
    pub(super) state: Mutex<V5ReceiptRuntimeTelemetryState>,
    pub(super) changed: Condvar,
}

impl V5ReceiptRuntimeTelemetry {
    pub(super) fn new() -> Self {
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
                store_fault: None,
            }),
            changed: Condvar::new(),
        }
    }

    pub(super) fn lock_state(&self) -> MutexGuard<'_, V5ReceiptRuntimeTelemetryState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(super) fn record_event(&self, event: V5ReceiptRuntimeEventKind, epoch_ms: u64) -> u64 {
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

    pub(super) fn record_prepare(&self) {
        let mut state = self.lock_state();
        state.callbacks.prepare = state.callbacks.prepare.saturating_add(1);
        self.changed.notify_all();
    }

    pub(super) fn record_validation(&self) {
        let mut state = self.lock_state();
        state.callbacks.validation = state.callbacks.validation.saturating_add(1);
        self.changed.notify_all();
    }

    pub(super) fn record_admission(&self) {
        let mut state = self.lock_state();
        state.callbacks.admission = state.callbacks.admission.saturating_add(1);
        self.changed.notify_all();
    }

    pub(super) fn record_execute(&self) {
        let mut state = self.lock_state();
        state.callbacks.execute = state.callbacks.execute.saturating_add(1);
        self.changed.notify_all();
    }

    pub(super) fn record_task_store_create_attempt(&self) {
        let mut state = self.lock_state();
        state.task_store_create_attempts = state.task_store_create_attempts.saturating_add(1);
        self.changed.notify_all();
    }

    pub(super) fn record_restart_requested(&self) {
        let mut state = self.lock_state();
        state.restart_requested = true;
        self.changed.notify_all();
    }

    pub(super) fn record_forced_process_exit(&self) {
        let mut state = self.lock_state();
        if state.restart_requested
            && state.listener == V5ReceiptRuntimeListenerState::Closed
            && !state.daemon_running
        {
            return;
        }
        let epoch_ms = state.events.last().map_or(1, |event| event.epoch_ms.max(1));
        state.restart_requested = true;
        state.listener = V5ReceiptRuntimeListenerState::Closed;
        state.daemon_running = false;
        self.changed.notify_all();
        drop(state);
        self.record_event(V5ReceiptRuntimeEventKind::ListenerClosed, epoch_ms);
    }

    pub(super) fn reset_for_scenario(&self) {
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
        state.store_fault = None;
        self.changed.notify_all();
    }

    pub(super) fn arm_store_fault(&self, point: V5StoreFaultPoint) {
        self.lock_state().store_fault = Some(point);
    }

    pub(super) fn take_store_fault(&self, point: V5StoreFaultPoint) -> bool {
        let mut state = self.lock_state();
        if state.store_fault == Some(point) {
            state.store_fault = None;
            true
        } else {
            false
        }
    }

    pub(super) fn snapshot(&self) -> V5ReceiptRuntimeTelemetrySnapshot {
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

    pub(super) fn record_direct_publication(
        &self,
        publication: &CommittedDirectPublication,
        response_kind: &'static str,
        origin: &'static str,
        candidate_result_override: Option<Value>,
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
                    "candidateResult": candidate_result_override
                        .or_else(|| candidate_result_value(record.terminal())),
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

    pub(super) fn record_receipt_backed_publication(
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

    pub(super) fn wait_for_event(
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

    pub(super) fn wait_for_event_count(
        &self,
        event: V5ReceiptRuntimeEventKind,
        count: usize,
        deadline: Instant,
    ) -> Result<(), String> {
        let mut state = self.lock_state();
        let observed_count = |state: &V5ReceiptRuntimeTelemetryState| {
            state
                .events
                .iter()
                .filter(|record| {
                    record.sequence >= state.wait_floor_sequence && record.event == event
                })
                .count()
        };
        while observed_count(&state) < count {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .ok_or_else(|| {
                    format!(
                    "protocol-v5 runtime event {event:?} was observed {} times, expected {count}",
                    observed_count(&state)
                )
                })?;
            let (next, timeout) = self
                .changed
                .wait_timeout(state, remaining)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state = next;
            if timeout.timed_out() && observed_count(&state) < count {
                return Err(format!(
                    "protocol-v5 runtime event {event:?} was observed {} times, expected {count}",
                    observed_count(&state)
                ));
            }
        }
        Ok(())
    }

    pub(super) fn wait_for_either_event(
        &self,
        first: V5ReceiptRuntimeEventKind,
        second: V5ReceiptRuntimeEventKind,
        deadline: Instant,
    ) -> Result<(), String> {
        let mut state = self.lock_state();
        let observed = |state: &V5ReceiptRuntimeTelemetryState| {
            state.events.iter().any(|record| {
                record.sequence >= state.wait_floor_sequence
                    && (record.event == first || record.event == second)
            })
        };
        while !observed(&state) {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .ok_or_else(|| {
                    format!(
                        "neither protocol-v5 runtime event {first:?} nor {second:?} was observed"
                    )
                })?;
            let (next, timeout) = self
                .changed
                .wait_timeout(state, remaining)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state = next;
            if timeout.timed_out() && !observed(&state) {
                return Err(format!(
                    "neither protocol-v5 runtime event {first:?} nor {second:?} was observed"
                ));
            }
        }
        Ok(())
    }

    pub(super) fn listener_lease(self: &Arc<Self>) -> V5ReceiptRuntimeListenerLease {
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

    pub(super) fn actor_lease(self: &Arc<Self>) -> V5ReceiptRuntimeActorLease {
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

pub(super) fn receipt_key_observation_value(key: &ReceiptKey) -> Value {
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

pub(super) fn artifact_value(
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

pub(super) fn artifact_from_bytes(bytes: &[u8]) -> Value {
    artifact_value(
        bytes,
        u64::try_from(bytes.len()).expect("artifact length fits u64"),
        &crate::application::receipt_ledger::ArtifactSha256::from_sha256(
            Sha256::digest(bytes).into(),
        ),
    )
}

pub(super) fn terminal_observation_value(
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

pub(super) fn candidate_result_value(
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

pub(super) fn lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

pub(super) struct V5ReceiptRuntimeListenerLease {
    telemetry: Arc<V5ReceiptRuntimeTelemetry>,
}

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

pub(super) struct V5ReceiptRuntimeActorLease {
    telemetry: Arc<V5ReceiptRuntimeTelemetry>,
}

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

pub(super) fn open_receipt_actor_for_scenario(
    receipts: RetainedDirectoryCapability,
    context: &'static str,
) -> Result<ReceiptLedgerActor, String> {
    // Opening a fixture after a horizon/full-pool action performs the same bounded
    // retained-store recovery as daemon startup. Keep that test-only I/O inside the
    // bulk fixture budget instead of accidentally applying the ordinary 5-second
    // operation timeout to thousands of durable rows.
    let store = ReceiptLedgerStore::open_retained_directory_before(
        receipts,
        Instant::now() + SCENARIO_BULK_OPERATION_TIMEOUT,
    )
    .map_err(|error| format!("{context}: {error}"))?;
    Ok(ReceiptLedgerActor::spawn(store))
}

pub(super) fn inject_receipt_identity_collision_for_scenario(
    receipts: RetainedDirectoryCapability,
    collide_on_invocation_id: bool,
    deadline: Instant,
) -> Result<(), String> {
    let store = ReceiptLedgerStore::open_retained_directory(receipts)
        .map_err(|error| format!("reopen identity-collision fixture store: {error}"))?;
    store
        .inject_identity_index_collision_for_test(collide_on_invocation_id, deadline)
        .map_err(|error| format!("inject persisted identity collision: {error}"))
}

pub(super) fn seed_receipt_backed_task_terminal_for_scenario(
    receipts: RetainedDirectoryCapability,
    seed: ReceiptBackedTaskTerminalSeed,
    deadline: Instant,
) -> Result<TaskTerminalReceiptBackedReceipt, String> {
    let store = ReceiptLedgerStore::open_retained_directory(receipts)
        .map_err(|error| format!("open receipt-backed Task fixture ledger: {error}"))?;
    store
        .seed_task_terminal_receipt_backed_for_test(seed, deadline)
        .map_err(|error| format!("seed receipt-backed Task terminal: {error}"))
}

pub(super) fn seed_receipt_tombstones_for_scenario(
    receipts: RetainedDirectoryCapability,
    keys: Vec<ReceiptKey>,
    acknowledged_at_epoch_ms: u64,
    terminal_digest: TerminalDigest,
    deadline: Instant,
) -> Result<(ReceiptLedgerActor, Vec<AcknowledgedTombstoneReceipt>), String> {
    let store = ReceiptLedgerStore::open_retained_directory(receipts)
        .map_err(|error| format!("open tombstone fixture ledger: {error}"))?;
    let seeded = store
        .seed_tombstones_for_test(keys, acknowledged_at_epoch_ms, terminal_digest, deadline)
        .map_err(|error| format!("seed tombstone fixture pool: {error}"))?;
    Ok((ReceiptLedgerActor::spawn(store), seeded))
}

pub(super) fn stage_bound_handoff_terminal_for_scenario(
    actor: &ReceiptLedgerActor,
    handoff: TaskHandoffActorBoundReceipt,
    epoch_ms: u64,
    terminal: crate::application::receipt_ledger::V5CanonicalTerminal,
    deadline: Instant,
    telemetry: &V5ReceiptRuntimeTelemetry,
) -> Result<TaskHandoffActorBoundReceipt, ReceiptLedgerError> {
    telemetry.record_execute();
    telemetry.record_event(V5ReceiptRuntimeEventKind::ExecuteEntered, epoch_ms);
    telemetry.record_event(V5ReceiptRuntimeEventKind::ResultSerialized, epoch_ms);
    let certificate = canonical_staged_transfer_certificate(
        handoff.key(),
        handoff.key_digest(),
        handoff.link(),
        epoch_ms,
        &terminal,
    )?;
    let staged = actor.stage_bound_task_handoff_terminal(
        handoff.key().clone(),
        handoff.record_version(),
        epoch_ms,
        terminal,
        certificate,
        deadline,
    )?;
    telemetry.record_event(
        V5ReceiptRuntimeEventKind::BoundHandoffTerminalStaged,
        epoch_ms,
    );
    Ok(staged)
}

pub(super) fn acknowledge_direct_for_scenario(
    actor: &ReceiptLedgerActor,
    key: ReceiptKey,
    terminal_digest: TerminalDigest,
    acknowledged_at_epoch_ms: u64,
    deadline: Instant,
    telemetry: &V5ReceiptRuntimeTelemetry,
) -> Result<AcknowledgedTombstoneReceipt, ReceiptLedgerError> {
    let acknowledgement =
        actor.acknowledge_direct(key, terminal_digest, acknowledged_at_epoch_ms, deadline)?;
    telemetry.record_event(
        V5ReceiptRuntimeEventKind::AcknowledgementCommitted,
        acknowledged_at_epoch_ms,
    );
    Ok(acknowledgement)
}

/// The harness's observer of one runtime: telemetry records every hook, the
/// scenario control pauses the runtime and answers its injected decisions.
pub(super) struct ScenarioHooks {
    pub(super) telemetry: Arc<V5ReceiptRuntimeTelemetry>,
    pub(super) control: Option<Arc<ReceiptScenarioControl>>,
}

impl ScenarioHooks {
    pub(super) fn install(
        telemetry: Arc<V5ReceiptRuntimeTelemetry>,
        control: Option<Arc<ReceiptScenarioControl>>,
    ) -> Arc<dyn V5RuntimeHooks> {
        Arc::new(Self { telemetry, control })
    }

    fn bulk_deadline() -> Instant {
        Instant::now() + SCENARIO_BULK_OPERATION_TIMEOUT
    }
}

fn evidence_error(context: &'static str) -> ReceiptLedgerError {
    ReceiptLedgerError::Corrupt(context)
}

impl V5RuntimeHooks for ScenarioHooks {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn event(&self, event: V5ReceiptRuntimeEventKind, epoch_ms: u64) {
        self.telemetry.record_event(event, epoch_ms);
    }

    fn stage_entered(&self, stage: V5Stage) {
        match stage {
            V5Stage::Validation => self.telemetry.record_validation(),
            V5Stage::Admission => self.telemetry.record_admission(),
            V5Stage::Prepare => self.telemetry.record_prepare(),
            V5Stage::Execute => self.telemetry.record_execute(),
        }
    }

    fn task_store_create_attempted(&self) {
        self.telemetry.record_task_store_create_attempt();
    }

    fn restart_requested(&self) {
        self.telemetry.record_restart_requested();
    }

    fn forced_process_exit(&self, grace: Option<Duration>) {
        self.telemetry.record_forced_process_exit();
        if let (Some(grace), Some(control)) = (grace, &self.control) {
            control.record_process_exit(u64::try_from(grace.as_millis()).unwrap_or(u64::MAX));
        }
    }

    fn listener_lease(&self) -> Option<Box<dyn Any + Send>> {
        Some(Box::new(self.telemetry.listener_lease()))
    }

    fn actor_lease(&self) -> Option<Box<dyn Any + Send>> {
        Some(Box::new(self.telemetry.actor_lease()))
    }

    fn runtime_opened(&self, runtime: &Arc<V5ReceiptRuntime>) {
        if let Some(control) = &self.control {
            control.record_runtime(runtime);
        }
    }

    fn direct_publication(
        &self,
        publication: &CommittedDirectPublication,
        response_kind: &'static str,
        origin: &'static str,
        oversized_candidate: Option<&DomainResult>,
    ) {
        let candidate = oversized_candidate.and_then(|result| {
            serde_json::to_vec(result)
                .ok()
                .map(|bytes| artifact_from_bytes(&bytes))
        });
        self.telemetry
            .record_direct_publication(publication, response_kind, origin, candidate);
    }

    fn receipt_backed_terminal(
        &self,
        committed: &TaskTerminalReceiptBackedReceipt,
    ) -> Result<(), ReceiptLedgerError> {
        match &self.control {
            Some(control) => control
                .record_receipt_backed_terminal(committed.clone())
                .map_err(|_| evidence_error("capture receipt-backed terminal evidence failed")),
            None => Ok(()),
        }
    }

    fn actor_workspace_identity(&self, identity: &SafeIdentityHash) {
        if let Some(control) = &self.control {
            control.record_actor_workspace_identity(identity.clone());
        }
    }

    fn callback_invocation_id(&self, invocation_id: InvocationId) {
        if let Some(control) = &self.control {
            control.record_callback_invocation_id(invocation_id);
        }
    }

    fn bound_task(&self, record: &V5StoredInvocationRecord, bound: &TaskBoundReceipt) {
        if let Some(control) = &self.control {
            control.record_bound_task(record.clone(), bound.clone());
        }
    }

    fn terminal_bound_task(
        &self,
        record: &V5StoredInvocationRecord,
        link: &TaskTerminalBoundReceipt,
    ) {
        if let Some(control) = &self.control {
            control.record_terminal_bound_task(record.clone(), link.clone());
        }
    }

    fn promised_actor_binding(
        &self,
        promised: &TaskPromisedUnboundReceipt,
        actor_promised: &TaskPromisedActorBoundReceipt,
        bound: &TaskBoundReceipt,
    ) {
        if let Some(control) = &self.control {
            control.record_promised_actor_binding(promised, actor_promised, bound);
        }
    }

    fn bound_task_start_authorization(
        &self,
        authorized: &TaskBoundReceipt,
        record: &V5StoredInvocationRecord,
        begun: &TaskBoundReceipt,
    ) {
        if let Some(control) = &self.control {
            control.record_bound_task_start_authorization(authorized, record, begun);
        }
    }

    fn staged_terminal_preparation(
        &self,
        staged: &TaskHandoffActorBoundReceipt,
    ) -> Result<(), ReceiptLedgerError> {
        match &self.control {
            Some(control) => control
                .record_staged_terminal_preparation(staged)
                .map_err(|_| evidence_error("record staged terminal preparation")),
            None => Ok(()),
        }
    }

    fn staged_terminal_publication(
        &self,
        handoff: &TaskHandoffActorBoundReceipt,
        provisional: &V5StoredInvocationRecord,
        terminal_record: &V5StoredInvocationRecord,
        link: &TaskTerminalBoundReceipt,
    ) -> Result<(), ReceiptLedgerError> {
        match &self.control {
            Some(control) => control
                .record_staged_terminal_publication(handoff, provisional, terminal_record, link)
                .map_err(|_| evidence_error("record staged terminal publication")),
            None => Ok(()),
        }
    }

    fn bound_terminal_publication(
        &self,
        bound: &TaskBoundReceipt,
        provisional: &V5StoredInvocationRecord,
        terminal_record: &V5StoredInvocationRecord,
        link: &TaskTerminalBoundReceipt,
        terminal: &V5CanonicalTerminal,
        epoch_ms: u64,
    ) -> Result<(), ReceiptLedgerError> {
        match &self.control {
            Some(control) => control
                .record_bound_terminal_publication(
                    bound,
                    provisional,
                    terminal_record,
                    link,
                    terminal,
                    epoch_ms,
                )
                .map_err(|_| evidence_error("record bound terminal publication")),
            None => Ok(()),
        }
    }

    fn pause(&self, point: V5PausePoint, deadline: Instant) -> Result<(), ReceiptLedgerError> {
        match &self.control {
            Some(control) => control.pause(point, deadline),
            None => Ok(()),
        }
    }

    fn holds(&self, point: V5PausePoint) -> bool {
        self.control
            .as_ref()
            .is_some_and(|control| control.is_barrier_installed(point))
    }

    fn commit_deadline_at(&self, point: V5PausePoint, deadline: Instant) -> Instant {
        if self.holds(point) {
            Self::bulk_deadline()
        } else {
            deadline
        }
    }

    fn process_exited(&self) -> bool {
        self.control
            .as_ref()
            .is_some_and(|control| control.process_exited())
    }

    fn observing(&self) -> bool {
        self.control.is_some()
    }

    fn releases_authority_on_fail_stop(&self) -> bool {
        true
    }

    fn acquire_lifecycle_gate(
        &self,
        label: &'static str,
        deadline: Instant,
    ) -> Result<(), ReceiptLedgerError> {
        match &self.control {
            Some(control) => control.acquire_lifecycle_gate(label, deadline),
            None => Ok(()),
        }
    }

    fn release_lifecycle_gate(&self, label: &'static str) {
        if let Some(control) = &self.control {
            control.release_lifecycle_gate(label);
        }
    }

    fn wait_for_gate_cancel(&self, deadline: Instant) -> Result<(), ReceiptLedgerError> {
        match &self.control {
            Some(control) => control.wait_for_gate_cancel(deadline),
            None => Ok(()),
        }
    }

    fn release_pre_actor_pauses(&self) {
        if let Some(control) = &self.control {
            control.release_pre_actor_barriers();
        }
    }

    fn validation_rejects(&self) -> bool {
        self.control
            .as_ref()
            .is_some_and(|control| control.validation_rejects())
    }

    fn admission_rejection(&self) -> Option<V5AdmissionRejection> {
        self.control
            .as_ref()
            .and_then(|control| control.admission_rejection())
    }

    fn prepare_rejects(&self) -> bool {
        self.control
            .as_ref()
            .is_some_and(|control| control.prepare_rejects())
    }

    fn crash_after_side_effect(&self) -> bool {
        self.control
            .as_ref()
            .is_some_and(|control| control.take_crash_after_side_effect())
    }

    fn store_fault(&self, point: V5StoreFaultPoint) -> bool {
        self.telemetry.take_store_fault(point)
    }

    fn submit_response_disconnect(&self) -> bool {
        self.control
            .as_ref()
            .is_some_and(|control| control.take_submit_response_disconnect())
    }

    fn ack_response_disconnect(&self) -> bool {
        self.control
            .as_ref()
            .is_some_and(|control| control.take_ack_response_disconnect())
    }

    fn session_deadline_override(&self) -> Option<Instant> {
        self.control
            .as_ref()
            .filter(|control| control.has_precomputed_terminal())
            .map(|_| Self::bulk_deadline())
    }

    fn staged_handoff(&self) -> Option<TaskHandoffActorBoundReceipt> {
        self.control
            .as_ref()
            .and_then(|control| control.staged_handoff())
    }

    fn bound_task_override(&self) -> Option<(V5StoredInvocationRecord, TaskBoundReceipt)> {
        self.control
            .as_ref()
            .and_then(|control| control.bound_task())
            .map(|bound_task| (bound_task.record, bound_task.bound))
    }
}
