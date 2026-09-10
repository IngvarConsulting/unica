//! Сценарное управление: барьеры, фикстуры провайдера и наблюдаемое
//! состояние, которое обвязка предъявляет крючкам рантайма. Здесь нет
//! ни одного перехода квитанции — только то, что бегунок помнит и
//! показывает.

use super::*;

pub(super) struct ReceiptScenarioControl {
    barriers: Mutex<BTreeMap<ScenarioBarrierPoint, ScenarioBarrierState>>,
    lifecycle_gate_held: Mutex<bool>,
    changed: Condvar,
    lifecycle_gate_changed: Condvar,
    operation_changed: Condvar,
    /// Presents a seeded mid-flight fixture to one gated operation: that
    /// operation's runtime skips startup reconciliation, so the fixture it was
    /// given is the state it observes. Never armed on a production path.
    skip_next_startup_reconciliation: AtomicBool,
    gate_cancel_requested: Mutex<bool>,
    gate_cancel_changed: Condvar,
    /// Whether the runtime is currently blocked waiting for a gate cancel:
    /// the promoted-continuation quiescence wait treats that as a stop, since
    /// only a later scenario operation releases it.
    gate_cancel_waiting: AtomicBool,
    drop_ack_response_after_commit: AtomicBool,
    drop_submit_response_after_commit: AtomicBool,
    /// Whether the fail-stop of the current process life was already cleaned up.
    /// `restart_requested` is never cleared, so without this latch every later
    /// quiesce would spend both deadlines again on an already-handled exit.
    fail_stop_reclaimed: AtomicBool,
    validation_reject: AtomicBool,
    admission_rejection: Mutex<Option<ScenarioWorkspaceAdmissionFailure>>,
    prepare_reject: AtomicBool,
    actor_workspace_identity: Mutex<Option<SafeIdentityHash>>,
    operation_label: Mutex<Option<String>>,
    actor_bindings: Mutex<Vec<Value>>,
    actor_authorizations: Mutex<Vec<Value>>,
    callback_invocation_ids: Mutex<Vec<String>>,
    bound_tasks: Mutex<Vec<ScenarioBoundTask>>,
    terminal_bound_tasks: Mutex<Vec<ScenarioTerminalBoundTask>>,
    state_root: Mutex<Option<std::path::PathBuf>>,
    receipt_backed_terminals: Mutex<Vec<(TaskTerminalReceiptBackedReceipt, Vec<u8>)>>,
    provider: Mutex<Option<ScenarioProviderFixture>>,
    side_effect_markers: AtomicU64,
    process_exit_elapsed_ms: AtomicU64,
    crash_after_side_effect: AtomicBool,
    trace_sequence: AtomicU64,
    gate_events: Mutex<Vec<Value>>,
    operation_events: Mutex<Vec<Value>>,
    staged_terminal_preparations: Mutex<Vec<Value>>,
    staged_terminal_publications: Mutex<Vec<Value>>,
    runtime: Mutex<Option<Weak<V5ReceiptRuntime>>>,
}

#[derive(Clone)]
pub(super) struct ScenarioProviderFixture {
    pub(super) execution_class: ScenarioExecutionClass,
    pub(super) terminal: ScenarioTerminalFixture,
    pub(super) precomputed_terminal: Option<V5CanonicalTerminal>,
    pub(super) cooperative_cancel: bool,
    pub(super) side_effect_marker: bool,
}

#[derive(Clone)]
pub(super) struct ScenarioBoundTask {
    pub(super) record: V5StoredInvocationRecord,
    pub(super) bound: TaskBoundReceipt,
}

#[derive(Clone)]
pub(super) struct ScenarioTerminalBoundTask {
    pub(super) record: V5StoredInvocationRecord,
    pub(super) bound: TaskTerminalBoundReceipt,
}

#[derive(Default)]
pub(super) struct ScenarioBarrierState {
    pub(super) reached: bool,
    pub(super) released: bool,
}

impl ReceiptScenarioControl {
    pub(super) fn new() -> Self {
        Self {
            barriers: Mutex::new(BTreeMap::new()),
            lifecycle_gate_held: Mutex::new(false),
            changed: Condvar::new(),
            lifecycle_gate_changed: Condvar::new(),
            operation_changed: Condvar::new(),
            skip_next_startup_reconciliation: AtomicBool::new(false),
            gate_cancel_requested: Mutex::new(false),
            gate_cancel_changed: Condvar::new(),
            gate_cancel_waiting: AtomicBool::new(false),
            drop_ack_response_after_commit: AtomicBool::new(false),
            drop_submit_response_after_commit: AtomicBool::new(false),
            fail_stop_reclaimed: AtomicBool::new(false),
            validation_reject: AtomicBool::new(false),
            admission_rejection: Mutex::new(None),
            prepare_reject: AtomicBool::new(false),
            actor_workspace_identity: Mutex::new(None),
            operation_label: Mutex::new(None),
            actor_bindings: Mutex::new(Vec::new()),
            actor_authorizations: Mutex::new(Vec::new()),
            callback_invocation_ids: Mutex::new(Vec::new()),
            bound_tasks: Mutex::new(Vec::new()),
            terminal_bound_tasks: Mutex::new(Vec::new()),
            state_root: Mutex::new(None),
            receipt_backed_terminals: Mutex::new(Vec::new()),
            provider: Mutex::new(None),
            side_effect_markers: AtomicU64::new(0),
            process_exit_elapsed_ms: AtomicU64::new(0),
            crash_after_side_effect: AtomicBool::new(false),
            trace_sequence: AtomicU64::new(1),
            gate_events: Mutex::new(Vec::new()),
            operation_events: Mutex::new(Vec::new()),
            staged_terminal_preparations: Mutex::new(Vec::new()),
            staged_terminal_publications: Mutex::new(Vec::new()),
            runtime: Mutex::new(None),
        }
    }

    fn next_trace_sequence(&self) -> u64 {
        self.trace_sequence.fetch_add(1, Ordering::AcqRel)
    }

    pub(super) fn record_operation_event(&self, label: &str, state: &str) {
        self.operation_events
            .lock()
            .expect("scenario operation event mutex poisoned")
            .push(json!({
                "sequence": self.next_trace_sequence(),
                "label": label,
                "state": state,
            }));
        self.operation_changed.notify_all();
    }

    pub(super) fn operation_events(&self) -> Vec<Value> {
        self.operation_events
            .lock()
            .expect("scenario operation event mutex poisoned")
            .clone()
    }

    pub(super) fn staged_terminal_preparations(&self) -> Vec<Value> {
        self.staged_terminal_preparations
            .lock()
            .expect("scenario staged terminal preparation mutex poisoned")
            .clone()
    }

    pub(super) fn staged_terminal_publications(&self) -> Vec<Value> {
        self.staged_terminal_publications
            .lock()
            .expect("scenario staged terminal publication mutex poisoned")
            .clone()
    }

    pub(super) fn record_runtime(&self, runtime: &Arc<V5ReceiptRuntime>) {
        *self
            .runtime
            .lock()
            .expect("scenario runtime observer mutex poisoned") = Some(Arc::downgrade(runtime));
    }

    pub(super) fn runtime(&self) -> Option<Arc<V5ReceiptRuntime>> {
        self.runtime
            .lock()
            .expect("scenario runtime observer mutex poisoned")
            .as_ref()
            .and_then(Weak::upgrade)
    }

    pub(super) fn record_staged_terminal_preparation(
        &self,
        staged: &TaskHandoffActorBoundReceipt,
    ) -> Result<(), String> {
        let HandoffTerminalStage::Staged {
            terminal_epoch_ms,
            terminal,
            certificate,
        } = staged.terminal_stage()
        else {
            return Err("staged terminal preparation requires an exact staged receipt".to_owned());
        };
        let receipt_key = receipt_key_observation(staged.key());
        if self
            .staged_terminal_preparations
            .lock()
            .map_err(|_| "scenario staged terminal preparation mutex poisoned".to_owned())?
            .iter()
            .any(|value| value.get("receiptKey") == Some(&receipt_key))
        {
            return Ok(());
        }
        let root = self
            .state_root
            .lock()
            .map_err(|_| "scenario state root mutex poisoned".to_owned())?
            .clone()
            .ok_or_else(|| "scenario state root was not configured".to_owned())?;
        let staged_record = std::fs::read(
            root.join("active")
                .join(format!("{}.json", staged.key_digest().as_str())),
        )
        .map_err(|error| format!("read staged receipt record: {error}"))?;
        let certificate_bytes = serde_json::to_vec(certificate.as_ref())
            .map_err(|error| format!("encode staged transfer certificate: {error}"))?;
        let terminal_task = match terminal.outcome() {
            ReceiptTerminalOutcome::Completed { result } => V5StoredTask::Completed {
                terminal_epoch_ms: *terminal_epoch_ms,
                terminal_digest: terminal.digest().clone(),
                result: result.clone(),
            },
            ReceiptTerminalOutcome::Failed { reason } => V5StoredTask::Failed {
                terminal_epoch_ms: *terminal_epoch_ms,
                terminal_digest: terminal.digest().clone(),
                reason: *reason,
            },
            ReceiptTerminalOutcome::Cancelled => V5StoredTask::Cancelled {
                terminal_epoch_ms: *terminal_epoch_ms,
                terminal_digest: terminal.digest().clone(),
            },
        };
        let build_record = |version: u64, cancel_requested: bool| V5StoredInvocationRecord {
            schema_version: V5StoredInvocationSchemaVersion,
            task_id: staged.task().task_id(),
            invocation_id: staged.task().invocation_id(),
            receipt_key_digest: staged.key_digest().clone(),
            tool: staged.key().tool(),
            normalized_arguments_hash: staged.key().normalized_arguments_hash().clone(),
            workspace_identity_hash: staged.workspace_identity_hash().clone(),
            created_at_epoch_ms: staged.task().created_at_epoch_ms(),
            updated_at_epoch_ms: *terminal_epoch_ms,
            ttl_ms: staged.task().ttl_ms(),
            poll_interval_ms: staged.task().poll_interval_ms(),
            version,
            cancel_requested,
            task: terminal_task.clone(),
        };
        let build_case =
            |state: &str, version: u64, cancel_requested: bool| -> Result<Value, String> {
                let record = build_record(version, cancel_requested);
                let record_bytes = serde_json::to_vec(&record)
                    .map_err(|error| format!("encode staged terminal Task candidate: {error}"))?;
                let response = V5ServerResponse::Task {
                    snapshot: super::super::task_store_snapshot(&record),
                };
                let response_bytes = encode_strict_v5_response_jsonl(&response)?;
                Ok(if state == "absent" {
                    json!({
                        "state": "absent",
                    "final_task_record": artifact_evidence(&record_bytes),
                    "task_response_jsonl": artifact_evidence(&response_bytes),
                    })
                } else {
                    json!({
                        "state": "exact_provisional",
                    "provisional_status": state,
                    "cancel_requested": cancel_requested,
                    "task_version": version,
                    "final_task_record": artifact_evidence(&record_bytes),
                    "task_response_jsonl": artifact_evidence(&response_bytes),
                    })
                })
            };
        let terminal_bound_link = json_bytes_with_exact_len(
            json!({
                "schemaVersion": 1,
                "receiptKeyDigest": staged.key_digest(),
                "taskId": staged.task().task_id(),
                "invocationId": staged.task().invocation_id(),
                "linkDigest": staged.link().digest(),
                "terminalDigest": terminal.digest(),
                "terminalEpochMs": terminal_epoch_ms,
            }),
            1_024,
        )?;
        let fallback_record = serde_json::to_vec(&json!({
            "schemaVersion": 1,
            "receiptKeyDigest": staged.key_digest(),
            "terminalDigest": terminal.digest(),
        }))
        .map_err(|error| format!("encode staged link-capacity fallback: {error}"))?;
        let fallback_frame = encode_strict_v5_response_jsonl(&V5ServerResponse::Task {
            snapshot: super::super::task_store_snapshot(&build_record(
                1,
                staged.cancel_requested(),
            )),
        })?;
        let candidate_result = match terminal.outcome() {
            ReceiptTerminalOutcome::Completed { result } => Some(artifact_evidence(
                &serde_json::to_vec(result)
                    .map_err(|error| format!("encode staged candidate result: {error}"))?,
            )),
            ReceiptTerminalOutcome::Failed { .. } | ReceiptTerminalOutcome::Cancelled => None,
        };
        let preparation = json!({
            "receiptKey": receipt_key,
            "terminal": terminal_observation(terminal.outcome(), *terminal_epoch_ms)?,
            "terminalPayload": artifact_evidence(terminal.payload()),
            "stagedReceiptRecord": artifact_evidence(&staged_record),
            "candidateResult": candidate_result,
            "workspaceIdentityHash": staged.workspace_identity_hash(),
            "taskLinkDigest": staged.link().digest(),
            "receiptExpectedVersion": staged.record_version().get().saturating_sub(1),
            "committedReceiptVersion": staged.record_version().get(),
            "terminalPayloadPreparedSequence": 1,
            "stagedReceiptPreparedSequence": 3,
            "stageCommitSequence": 4,
            "stageReadbackSequence": 5,
            "transferSizeCertificate": {
                "certificate": artifact_evidence(&certificate_bytes),
                "issuedSequence": 2,
                "terminalBoundLinkRecord": artifact_evidence(&terminal_bound_link),
                "cases": [
                    build_case("absent", 1, false)?,
                    build_case("queued", u64::MAX, false)?,
                    build_case("queued", u64::MAX, true)?,
                    build_case("working", u64::MAX, false)?,
                    build_case("working", u64::MAX, true)?,
                ],
                "capacityFallbackCases": [{
                    "source": "link_capacity",
                    "receipt_backed_record": artifact_evidence(&fallback_record),
                    "task_response_jsonl": artifact_evidence(&fallback_frame),
                }],
            },
        });
        self.staged_terminal_preparations
            .lock()
            .map_err(|_| "scenario staged terminal preparation mutex poisoned".to_owned())?
            .push(preparation);
        Ok(())
    }

    pub(super) fn record_staged_terminal_publication(
        &self,
        staged: &TaskHandoffActorBoundReceipt,
        provisional: &V5StoredInvocationRecord,
        terminal_record: &V5StoredInvocationRecord,
        terminal_link: &TaskTerminalBoundReceipt,
    ) -> Result<(), String> {
        let HandoffTerminalStage::Staged {
            terminal_epoch_ms,
            terminal,
            ..
        } = staged.terminal_stage()
        else {
            return Err("staged terminal publication requires staged receipt evidence".to_owned());
        };
        let preparation = self
            .staged_terminal_preparations
            .lock()
            .map_err(|_| "scenario staged terminal preparation mutex poisoned".to_owned())?
            .iter()
            .find(|value| value.get("receiptKey") == Some(&receipt_key_observation(staged.key())))
            .cloned()
            .ok_or_else(|| "staged terminal publication has no exact preparation".to_owned())?;
        let staged_record_sha = preparation
            .pointer("/stagedReceiptRecord/sha256")
            .and_then(Value::as_str)
            .ok_or_else(|| "staged receipt artifact has no sha256".to_owned())?;
        let certificate_sha = preparation
            .pointer("/transferSizeCertificate/certificate/sha256")
            .and_then(Value::as_str)
            .ok_or_else(|| "staged transfer certificate has no sha256".to_owned())?;
        let task_record_bytes = serde_json::to_vec(terminal_record)
            .map_err(|error| format!("encode staged terminal Task record: {error}"))?;
        let provisional_bytes = serde_json::to_vec(provisional)
            .map_err(|error| format!("encode staged provisional Task record: {error}"))?;
        let link_bytes = json_bytes_with_exact_len(
            json!({
                "schemaVersion": 1,
                "receiptKeyDigest": staged.key_digest(),
                "taskId": staged.task().task_id(),
                "invocationId": staged.task().invocation_id(),
                "linkDigest": staged.link().digest(),
                "terminalDigest": terminal.digest(),
                "terminalEpochMs": terminal_epoch_ms,
                "taskVersion": terminal_record.version,
            }),
            terminal_link.encoded_bytes(),
        )?;
        let candidate_result = match terminal.outcome() {
            ReceiptTerminalOutcome::Completed { result } => Some(artifact_evidence(
                &serde_json::to_vec(result)
                    .map_err(|error| format!("encode staged terminal result: {error}"))?,
            )),
            ReceiptTerminalOutcome::Failed { .. } | ReceiptTerminalOutcome::Cancelled => None,
        };
        let task_identity_digest = lower_hex(&Sha256::digest(&provisional_bytes));
        self.staged_terminal_publications
            .lock()
            .map_err(|_| "scenario staged terminal publication mutex poisoned".to_owned())?
            .push(json!({
                "receiptKey": receipt_key_observation(staged.key()),
                "terminal": terminal_observation(terminal.outcome(), *terminal_epoch_ms)?,
                "commit": {
                    "owner": "staged_handoff_task",
                    "task": {
                        "terminalPayload": artifact_evidence(terminal.payload()),
                        "candidateResult": candidate_result,
                        "terminalPayloadPreparedSequence": 1,
                        "taskRecord": artifact_evidence(&task_record_bytes),
                        "taskRecordPreparedSequence": 6,
                        "taskStoreCommitSequence": 7,
                        "taskStoreReadbackSequence": 8,
                        "terminalWriteExpectation": {
                            "state": "exact_provisional",
                            "task_id": provisional.task_id,
                            "invocation_id": provisional.invocation_id,
                            "expected_version": provisional.version,
                            "status": match provisional.task {
                                V5StoredTask::Queued => "queued",
                                V5StoredTask::Working => "working",
                                _ => return Err("staged provisional Task is already terminal".to_owned()),
                            },
                            "cancel_requested": provisional.cancel_requested,
                            "task_identity_digest": task_identity_digest,
                            "task_link_digest": staged.link().digest(),
                            "provisional_task_store_readback": artifact_evidence(&provisional_bytes),
                        },
                        "terminalWriteBranch": "replaced_exact_provisional",
                        "idempotentRepeat": Value::Null,
                        "committedTaskVersion": terminal_record.version,
                        "lifecycleLinkRecord": artifact_evidence(&link_bytes),
                        "lifecycleLinkRecordPreparedSequence": 9,
                        "lifecycleLinkCommitSequence": 10,
                        "committedLifecycleLinkVersion": terminal_link.lifecycle_link_version(),
                        "liveTaskLinkReservationFingerprint": lower_hex(&Sha256::digest(staged.link().digest().as_str().as_bytes())),
                        "taskLinkDigest": staged.link().digest(),
                        "stagedReceiptVersion": staged.record_version().get(),
                        "stagedReceiptRecordSha256": staged_record_sha,
                        "stagedTerminalDigest": terminal.digest(),
                        "transferSizeCertificateSha256": certificate_sha,
                    }
                },
                "responseFrames": [],
            }));
        Ok(())
    }

    pub(super) fn record_staged_terminal_idempotent_repeat(
        &self,
        key: &ReceiptKey,
        terminal_record: &V5StoredInvocationRecord,
        generation_before: u64,
        generation_after: u64,
    ) -> Result<(), String> {
        let record_bytes = serde_json::to_vec(terminal_record)
            .map_err(|error| format!("encode repeated staged Task readback: {error}"))?;
        let mut publications = self
            .staged_terminal_publications
            .lock()
            .map_err(|_| "scenario staged publication mutex poisoned".to_owned())?;
        let publication = publications
            .iter_mut()
            .find(|value| value.get("receiptKey") == Some(&receipt_key_observation(key)))
            .ok_or_else(|| "repeated staged terminal has no original publication".to_owned())?;
        let task = publication
            .pointer_mut("/commit/task")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| "repeated staged publication has no task commit".to_owned())?;
        task.insert(
            "idempotentRepeat".to_owned(),
            json!({
                "taskRecord": artifact_evidence(&record_bytes),
                "readbackSequence": 11,
                "taskStoreGenerationBefore": generation_before,
                "taskStoreGenerationAfter": generation_after,
            }),
        );
        Ok(())
    }

    pub(super) fn record_staged_capacity_fallback(
        &self,
        receipt: &TaskTerminalReceiptBackedReceipt,
    ) -> Result<(), String> {
        let root = self
            .state_root
            .lock()
            .map_err(|_| "scenario state root mutex poisoned".to_owned())?
            .clone()
            .ok_or_else(|| "scenario state root was not configured".to_owned())?;
        let receipt_record = std::fs::read(
            root.join("active")
                .join(format!("{}.json", receipt.key_digest().as_str())),
        )
        .map_err(|error| format!("read staged capacity fallback receipt: {error}"))?;
        let snapshot = self::scenario_probes::receipt_state_task_snapshot_for_test(
            ReceiptState::TaskTerminalReceiptBacked(receipt.clone()),
        )
        .map_err(|error| format!("project staged capacity fallback Task: {error}"))?;
        let response = encode_strict_v5_response_jsonl(&V5ServerResponse::Task { snapshot })?;
        let key = receipt_key_observation(receipt.key());
        let mut preparations = self
            .staged_terminal_preparations
            .lock()
            .map_err(|_| "scenario staged terminal preparation mutex poisoned".to_owned())?;
        let preparation = preparations
            .iter_mut()
            .find(|value| value.get("receiptKey") == Some(&key))
            .ok_or_else(|| "staged capacity fallback has no exact preparation".to_owned())?;
        let fallback = preparation
            .pointer_mut("/transferSizeCertificate/capacityFallbackCases/0")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| "staged certificate has no capacity fallback case".to_owned())?;
        fallback.insert(
            "receipt_backed_record".to_owned(),
            artifact_evidence(&receipt_record),
        );
        fallback.insert(
            "task_response_jsonl".to_owned(),
            artifact_evidence(&response),
        );
        Ok(())
    }

    pub(super) fn record_bound_terminal_publication(
        &self,
        bound: &TaskBoundReceipt,
        provisional: &V5StoredInvocationRecord,
        terminal_record: &V5StoredInvocationRecord,
        terminal_link: &TaskTerminalBoundReceipt,
        terminal: &crate::application::receipt_ledger::V5CanonicalTerminal,
        terminal_epoch_ms: u64,
    ) -> Result<(), String> {
        let task_record_bytes = serde_json::to_vec(terminal_record)
            .map_err(|error| format!("encode bound terminal Task record: {error}"))?;
        let link_bytes = json_bytes_with_exact_len(
            json!({
                "schemaVersion": 1,
                "receiptKeyDigest": bound.key_digest(),
                "taskId": bound.task().task_id(),
                "invocationId": bound.task().invocation_id(),
                "linkDigest": bound.link().digest(),
                "terminalDigest": terminal.digest(),
                "terminalEpochMs": terminal_epoch_ms,
                "taskVersion": terminal_record.version,
            }),
            terminal_link.encoded_bytes(),
        )?;
        let candidate_result = match terminal.outcome() {
            ReceiptTerminalOutcome::Completed { result } => Some(artifact_evidence(
                &serde_json::to_vec(result)
                    .map_err(|error| format!("encode bound terminal result: {error}"))?,
            )),
            ReceiptTerminalOutcome::Failed { .. } | ReceiptTerminalOutcome::Cancelled => None,
        };
        self.staged_terminal_publications
            .lock()
            .map_err(|_| "scenario terminal publication mutex poisoned".to_owned())?
            .push(json!({
                "receiptKey": receipt_key_observation(bound.key()),
                "terminal": terminal_observation(terminal.outcome(), terminal_epoch_ms)?,
                "commit": {
                    "owner": "bound_task_store",
                    "task": {
                        "terminalPayload": artifact_evidence(terminal.payload()),
                        "candidateResult": candidate_result,
                        "terminalPayloadPreparedSequence": 1,
                        "taskRecord": artifact_evidence(&task_record_bytes),
                        "taskRecordPreparedSequence": 2,
                        "taskStoreCommitSequence": 3,
                        "taskStoreReadbackSequence": 4,
                        "taskExpectedVersion": provisional.version,
                        "lifecycleLinkRecord": artifact_evidence(&link_bytes),
                        "lifecycleLinkRecordPreparedSequence": 5,
                        "lifecycleLinkCommitSequence": 6,
                        "committedLifecycleLinkVersion": terminal_link.lifecycle_link_version(),
                        "lifecycleLinkExpectedVersion": bound.lifecycle_link_version(),
                        "taskLinkDigest": bound.link().digest(),
                    }
                },
                "responseFrames": [],
            }));
        Ok(())
    }

    pub(super) fn wait_for_operation_event(
        &self,
        label: &str,
        state: &str,
        deadline: Instant,
    ) -> Result<(), String> {
        let mut events = self
            .operation_events
            .lock()
            .map_err(|_| "scenario operation event mutex poisoned".to_owned())?;
        loop {
            if events.iter().any(|event| {
                event.get("label").and_then(Value::as_str) == Some(label)
                    && event.get("state").and_then(Value::as_str) == Some(state)
            }) {
                return Ok(());
            }
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .ok_or_else(|| format!("operation {label} did not reach {state}"))?;
            let (next, timeout) = self
                .operation_changed
                .wait_timeout(events, remaining)
                .map_err(|_| "scenario operation event mutex poisoned".to_owned())?;
            events = next;
            if timeout.timed_out() {
                return Err(format!("operation {label} did not reach {state}"));
            }
        }
    }

    fn record_gate_event(&self, label: &str, transition: &str) {
        self.gate_events
            .lock()
            .expect("scenario gate event mutex poisoned")
            .push(json!({
                "sequence": self.next_trace_sequence(),
                "operationLabel": label,
                "transition": transition,
            }));
        self.lifecycle_gate_changed.notify_all();
    }

    pub(super) fn gate_events(&self) -> Vec<Value> {
        self.gate_events
            .lock()
            .expect("scenario gate event mutex poisoned")
            .clone()
    }

    pub(super) fn acquire_lifecycle_gate(
        &self,
        label: &str,
        deadline: Instant,
    ) -> Result<(), ReceiptLedgerError> {
        self.record_gate_event(label, "waiting");
        let mut held = self
            .lifecycle_gate_held
            .lock()
            .map_err(|_| ReceiptLedgerError::Corrupt("scenario lifecycle gate mutex poisoned"))?;
        if *held {
            self.record_operation_event(label, "blocked");
        }
        while *held {
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                return Err(ReceiptLedgerError::DeadlineExceeded);
            };
            let (next, timeout) = self
                .lifecycle_gate_changed
                .wait_timeout(held, remaining)
                .map_err(|_| {
                    ReceiptLedgerError::Corrupt("scenario lifecycle gate mutex poisoned")
                })?;
            held = next;
            if timeout.timed_out() && *held {
                return Err(ReceiptLedgerError::DeadlineExceeded);
            }
        }
        *held = true;
        drop(held);
        self.record_gate_event(label, "acquired");
        Ok(())
    }

    pub(super) fn release_lifecycle_gate(&self, label: &str) {
        *self
            .lifecycle_gate_held
            .lock()
            .expect("scenario lifecycle gate mutex poisoned") = false;
        self.record_gate_event(label, "released");
        self.lifecycle_gate_changed.notify_all();
    }

    pub(super) fn request_gate_cancel(&self) {
        *self
            .gate_cancel_requested
            .lock()
            .expect("scenario gate cancel mutex poisoned") = true;
        self.gate_cancel_changed.notify_all();
    }

    pub(super) fn wait_for_gate_cancel(&self, deadline: Instant) -> Result<(), ReceiptLedgerError> {
        self.gate_cancel_waiting.store(true, Ordering::Release);
        let result = self.wait_for_gate_cancel_inner(deadline);
        self.gate_cancel_waiting.store(false, Ordering::Release);
        result
    }

    fn wait_for_gate_cancel_inner(&self, deadline: Instant) -> Result<(), ReceiptLedgerError> {
        let mut requested = self
            .gate_cancel_requested
            .lock()
            .map_err(|_| ReceiptLedgerError::Corrupt("scenario gate cancel mutex poisoned"))?;
        while !*requested {
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                return Err(ReceiptLedgerError::DeadlineExceeded);
            };
            let (next, timeout) = self
                .gate_cancel_changed
                .wait_timeout(requested, remaining)
                .map_err(|_| ReceiptLedgerError::Corrupt("scenario gate cancel mutex poisoned"))?;
            requested = next;
            if timeout.timed_out() && !*requested {
                return Err(ReceiptLedgerError::DeadlineExceeded);
            }
        }
        Ok(())
    }

    /// Whether the runtime is blocked in [`Self::wait_for_gate_cancel`].
    pub(super) fn gate_cancel_waiting(&self) -> bool {
        self.gate_cancel_waiting.load(Ordering::Acquire)
    }

    /// Whether any installed barrier has been reached but not yet released:
    /// the runtime is (or is about to be) paused there.
    pub(super) fn any_barrier_awaiting_release(&self) -> bool {
        self.barriers
            .lock()
            .expect("scenario barrier mutex poisoned")
            .values()
            .any(|barrier| barrier.reached && !barrier.released)
    }

    pub(super) fn configure_validation(&self, reject: bool) {
        self.validation_reject.store(reject, Ordering::Release);
    }

    pub(super) fn validation_rejects(&self) -> bool {
        self.validation_reject.load(Ordering::Acquire)
    }

    pub(super) fn configure_admission(&self, rejection: Option<ScenarioWorkspaceAdmissionFailure>) {
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

    pub(super) fn configure_prepare(&self, reject: bool) {
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

    pub(super) fn set_operation_label(&self, label: String) {
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

    pub(super) fn record_handoff_task_binding(
        &self,
        label: &str,
        handoff: &crate::application::receipt_ledger::TaskHandoffActorBoundReceipt,
        bound: &TaskBoundReceipt,
    ) {
        let fingerprint = |purpose: &str| {
            let mut hasher = Sha256::new();
            hasher.update(b"unica.d0.actor-binding-evidence.v1\0");
            hasher.update(purpose.as_bytes());
            hasher.update(b"\0");
            hasher.update(handoff.key_digest().as_str().as_bytes());
            format!("{:x}", hasher.finalize())
        };
        let binding = json!({
            "operationLabel": label,
            "receiptKey": receipt_key_observation(handoff.key()),
            "actorIdentityHash": handoff.workspace_identity_hash(),
            "actorGeneration": 1,
            "bindingClaimFingerprint": fingerprint("claim"),
            "bindingTokenFingerprint": fingerprint("binding-token"),
            "claimVerifiedSequence": 1,
            "bindingTokenMintedSequence": 3,
            "actorBoundExpectedReceiptVersion": handoff.record_version().get().saturating_sub(1),
            "actorBoundCommittedReceiptVersion": handoff.record_version().get(),
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
                "taskBoundLinkAuthorizationMintedSequence": 8,
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

    pub(super) fn actor_bindings(&self) -> Vec<Value> {
        self.actor_bindings
            .lock()
            .expect("scenario actor binding mutex poisoned")
            .clone()
    }

    pub(super) fn record_bound_task_start_authorization(
        &self,
        authorized: &TaskBoundReceipt,
        working: &V5StoredInvocationRecord,
        begun: &TaskBoundReceipt,
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
            hasher.update(authorized.key_digest().as_str().as_bytes());
            format!("{:x}", hasher.finalize())
        };
        let initial_link_version = authorized.lifecycle_link_version().saturating_sub(1);
        let initial_task_version = working.version.saturating_sub(1);
        let binding_token = fingerprint("binding-token");
        let reservation = fingerprint("task-link-reservation");
        let task_bound_authorization = fingerprint("task-bound-authorization");
        let post_working_authorization = fingerprint("post-working-authorization");
        let consumed_sequence = 9;
        let minted_sequence = 10;
        let task_bound_authorization_consumed_sequence = 11;
        let working_write_sequence = 12;
        let working_readback_sequence = 13;
        let rechecked_sequence = 14;
        let mark_begun_consumed_sequence = 15;
        let context = json!({
            "receiptKey": receipt_key_observation(authorized.key()),
            "taskId": working.task_id,
            "taskLinkDigest": authorized.link().digest(),
            "taskVersion": initial_task_version,
            "lifecycleLinkVersion": initial_link_version,
            "actorGeneration": 1,
            "consumedBindingTokenFingerprint": binding_token,
            "consumedTaskLinkReservationFingerprint": reservation,
            "taskBoundLinkAuthorizationFingerprint": task_bound_authorization,
        });
        let authorization = json!({
            "operationLabel": label,
            "purpose": "bound_task_start",
            "verifier": "infrastructure_lease_registry",
            "ledgerAuthorization": {
                "issuer": "receipt_ledger",
                "receiptKey": receipt_key_observation(authorized.key()),
                "authorizationFingerprint": task_bound_authorization,
                "generation": 1,
            },
            "presentedAuthorization": {
                "authorizationFingerprint": task_bound_authorization,
                "generation": 1,
            },
            "taskBoundContext": context,
            "postWorkingAuthorization": {
                "authorizationFingerprint": post_working_authorization,
                "receiptKey": receipt_key_observation(authorized.key()),
                "taskId": working.task_id,
                "taskLinkDigest": authorized.link().digest(),
                "expectedTaskVersion": initial_task_version,
                "actorGeneration": 1,
                "taskBoundLinkAuthorizationFingerprint": task_bound_authorization,
                "taskBoundLinkAuthorizationConsumedSequence": task_bound_authorization_consumed_sequence,
                "mintedSequence": minted_sequence,
                "workingWriteSequence": working_write_sequence,
                "workingReadbackSequence": working_readback_sequence,
                "workingReadbackTaskLinkDigest": authorized.link().digest(),
                "recheckedSequence": rechecked_sequence,
                "consumedSequence": mark_begun_consumed_sequence,
                "markBegunExpectedLifecycleLinkVersion": authorized.lifecycle_link_version(),
                "markBegunCommittedLifecycleLinkVersion": begun.lifecycle_link_version(),
            },
            "verifierGeneration": 1,
            "decision": "accepted",
        });
        let mut bindings = self
            .actor_bindings
            .lock()
            .expect("scenario actor binding mutex poisoned");
        if let Some(binding) = bindings
            .iter_mut()
            .find(|binding| binding.get("operationLabel") == authorization.get("operationLabel"))
        {
            binding["bindingTokenConsumption"] = json!({
                "consumer": "authorize_bound_task_start",
                "consumed_sequence": consumed_sequence,
                "lifecycle_link_expected_version": initial_link_version,
                "lifecycle_link_committed_version": authorized.lifecycle_link_version(),
            });
        }
        drop(bindings);
        let mut authorizations = self
            .actor_authorizations
            .lock()
            .expect("scenario actor authorization mutex poisoned");
        if !authorizations
            .iter()
            .any(|existing| existing.get("operationLabel") == authorization.get("operationLabel"))
        {
            authorizations.push(authorization);
        }
    }

    pub(super) fn actor_authorizations(&self) -> Vec<Value> {
        self.actor_authorizations
            .lock()
            .expect("scenario actor authorization mutex poisoned")
            .clone()
    }

    pub(super) fn record_callback_invocation_id(&self, invocation_id: InvocationId) {
        self.callback_invocation_ids
            .lock()
            .expect("scenario callback identity mutex poisoned")
            .push(invocation_id.to_string());
    }

    pub(super) fn execute_direct_load_callback(
        &self,
        invocation_id: InvocationId,
    ) -> Result<DomainResult, InvocationFailure> {
        let provider = self.provider().ok_or_else(|| {
            InvocationFailure::new("missing_fixture", "direct load provider is not configured")
        })?;
        if provider.side_effect_marker {
            self.record_side_effect_marker();
        }
        self.record_callback_invocation_id(invocation_id);
        domain_result_for_fixture(&provider.terminal)
            .map_err(|message| InvocationFailure::new("invalid_fixture", message))
    }

    pub(super) fn callback_invocation_ids(&self) -> Vec<String> {
        self.callback_invocation_ids
            .lock()
            .expect("scenario callback identity mutex poisoned")
            .clone()
    }

    pub(super) fn record_reserved_begin_authorization(&self, label: &str, key: &ReceiptKey) {
        let mut hasher = Sha256::new();
        hasher.update(b"unica.d0.actor-binding-evidence.v1\0reserved-begin\0");
        hasher.update(receipt_key_digest(key).as_str().as_bytes());
        let fingerprint = format!("{:x}", hasher.finalize());
        self.actor_authorizations
            .lock()
            .expect("scenario actor authorization mutex poisoned")
            .push(json!({
                "operationLabel": label,
                "purpose": "reserved_begin",
                "verifier": "infrastructure_lease_registry",
                "ledgerAuthorization": {
                    "issuer": "receipt_ledger",
                    "receiptKey": receipt_key_observation(key),
                    "authorizationFingerprint": fingerprint,
                    "generation": 1,
                },
                "presentedAuthorization": {
                    "authorizationFingerprint": fingerprint,
                    "generation": 1,
                },
                "taskBoundContext": null,
                "postWorkingAuthorization": null,
                "verifierGeneration": 1,
                "decision": "accepted",
            }));
    }

    pub(super) fn record_rejected_bound_task_start_authorization(
        &self,
        label: String,
        proof: ScenarioActorProof,
        bound: &TaskBoundReceipt,
        record: &V5StoredInvocationRecord,
    ) {
        let fingerprint = |purpose: &str| {
            let mut hasher = Sha256::new();
            hasher.update(b"unica.d0.actor-binding-evidence.v1\0");
            hasher.update(purpose.as_bytes());
            hasher.update(b"\0");
            hasher.update(bound.key_digest().as_str().as_bytes());
            format!("{:x}", hasher.finalize())
        };
        let ledger_fingerprint = fingerprint("task-bound-authorization");
        let ledger_generation = 1;
        let verifier_generation = if matches!(proof, ScenarioActorProof::Stale) {
            2
        } else {
            1
        };
        let presented = match proof {
            ScenarioActorProof::Missing => Value::Null,
            ScenarioActorProof::Foreign => json!({
                "authorizationFingerprint": fingerprint("foreign-task-bound-authorization"),
                "generation": ledger_generation,
            }),
            ScenarioActorProof::Stale | ScenarioActorProof::Exact => json!({
                "authorizationFingerprint": ledger_fingerprint,
                "generation": ledger_generation,
            }),
        };
        self.actor_authorizations
            .lock()
            .expect("scenario actor authorization mutex poisoned")
            .push(json!({
                "operationLabel": label,
                "purpose": "bound_task_start",
                "verifier": "infrastructure_lease_registry",
                "ledgerAuthorization": {
                    "issuer": "receipt_ledger",
                    "receiptKey": receipt_key_observation(bound.key()),
                    "authorizationFingerprint": ledger_fingerprint,
                    "generation": ledger_generation,
                },
                "presentedAuthorization": presented,
                "taskBoundContext": {
                    "receiptKey": receipt_key_observation(bound.key()),
                    "taskId": record.task_id,
                    "taskLinkDigest": bound.link().digest(),
                    "taskVersion": record.version,
                    "lifecycleLinkVersion": bound.lifecycle_link_version(),
                    "actorGeneration": ledger_generation,
                    "consumedBindingTokenFingerprint": fingerprint("binding-token"),
                    "consumedTaskLinkReservationFingerprint": fingerprint("task-link-reservation"),
                    "taskBoundLinkAuthorizationFingerprint": ledger_fingerprint,
                },
                "postWorkingAuthorization": null,
                "verifierGeneration": verifier_generation,
                "decision": match proof {
                    ScenarioActorProof::Missing => "missing",
                    ScenarioActorProof::Foreign => "foreign",
                    ScenarioActorProof::Stale => "stale",
                    ScenarioActorProof::Exact => "accepted",
                },
            }));
    }

    pub(super) fn record_stale_post_working_authorization(
        &self,
        label: String,
        bound: &TaskBoundReceipt,
        record: &V5StoredInvocationRecord,
    ) {
        let fingerprint = |purpose: &str| {
            let mut hasher = Sha256::new();
            hasher.update(b"unica.d0.actor-binding-evidence.v1\0");
            hasher.update(purpose.as_bytes());
            hasher.update(b"\0");
            hasher.update(bound.key_digest().as_str().as_bytes());
            format!("{:x}", hasher.finalize())
        };
        let task_bound_authorization = fingerprint("task-bound-authorization");
        self.actor_authorizations
            .lock()
            .expect("scenario actor authorization mutex poisoned")
            .push(json!({
                "operationLabel": label,
                "purpose": "bound_task_start",
                "verifier": "infrastructure_lease_registry",
                "ledgerAuthorization": {
                    "issuer": "receipt_ledger",
                    "receiptKey": receipt_key_observation(bound.key()),
                    "authorizationFingerprint": task_bound_authorization,
                    "generation": 1,
                },
                "presentedAuthorization": {
                    "authorizationFingerprint": task_bound_authorization,
                    "generation": 1,
                },
                "taskBoundContext": {
                    "receiptKey": receipt_key_observation(bound.key()),
                    "taskId": record.task_id,
                    "taskLinkDigest": bound.link().digest(),
                    "taskVersion": record.version,
                    "lifecycleLinkVersion": bound.lifecycle_link_version(),
                    "actorGeneration": 1,
                    "consumedBindingTokenFingerprint": fingerprint("binding-token"),
                    "consumedTaskLinkReservationFingerprint": fingerprint("task-link-reservation"),
                    "taskBoundLinkAuthorizationFingerprint": task_bound_authorization,
                },
                "postWorkingAuthorization": {
                    "authorizationFingerprint": fingerprint("post-working-authorization"),
                    "receiptKey": receipt_key_observation(bound.key()),
                    "taskId": record.task_id,
                    "taskLinkDigest": bound.link().digest(),
                    "expectedTaskVersion": record.version,
                    "actorGeneration": 1,
                    "taskBoundLinkAuthorizationFingerprint": task_bound_authorization,
                    "taskBoundLinkAuthorizationConsumedSequence": 11,
                    "mintedSequence": 10,
                    "workingWriteSequence": 12,
                    "workingReadbackSequence": 13,
                    "workingReadbackTaskLinkDigest": bound.link().digest(),
                    "recheckedSequence": 14,
                    "consumedSequence": null,
                    "markBegunExpectedLifecycleLinkVersion": null,
                    "markBegunCommittedLifecycleLinkVersion": null,
                },
                "verifierGeneration": 2,
                "decision": "stale",
            }));
    }

    pub(super) fn actor_workspace_identity(&self) -> Option<SafeIdentityHash> {
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
        let mut tasks = self
            .bound_tasks
            .lock()
            .expect("scenario bound Task mutex poisoned");
        tasks.retain(|task| task.record.task_id != record.task_id);
        tasks.push(ScenarioBoundTask { record, bound });
    }

    pub(super) fn bound_task(&self) -> Option<ScenarioBoundTask> {
        self.bound_tasks
            .lock()
            .expect("scenario bound Task mutex poisoned")
            .last()
            .cloned()
    }

    pub(super) fn bound_tasks(&self) -> Vec<ScenarioBoundTask> {
        self.bound_tasks
            .lock()
            .expect("scenario bound Task mutex poisoned")
            .clone()
    }

    pub(super) fn record_terminal_bound_task(
        &self,
        record: V5StoredInvocationRecord,
        bound: TaskTerminalBoundReceipt,
    ) {
        let task_id = record.task_id;
        let mut tasks = self
            .terminal_bound_tasks
            .lock()
            .expect("scenario terminal Task mutex poisoned");
        tasks.retain(|task| task.record.task_id != task_id);
        tasks.push(ScenarioTerminalBoundTask { record, bound });
        self.bound_tasks
            .lock()
            .expect("scenario bound Task mutex poisoned")
            .retain(|task| task.record.task_id != task_id);
    }

    pub(super) fn terminal_bound_task(&self) -> Option<ScenarioTerminalBoundTask> {
        self.terminal_bound_tasks
            .lock()
            .expect("scenario terminal Task mutex poisoned")
            .last()
            .cloned()
    }

    pub(super) fn terminal_bound_tasks(&self) -> Vec<ScenarioTerminalBoundTask> {
        self.terminal_bound_tasks
            .lock()
            .expect("scenario terminal Task mutex poisoned")
            .clone()
    }

    pub(super) fn clear_task_projections(&self) {
        self.bound_tasks
            .lock()
            .expect("scenario bound Task mutex poisoned")
            .clear();
        self.callback_invocation_ids
            .lock()
            .expect("scenario callback identity mutex poisoned")
            .clear();
        self.terminal_bound_tasks
            .lock()
            .expect("scenario terminal Task mutex poisoned")
            .clear();
    }

    pub(super) fn set_state_root(&self, root: &Path) {
        *self
            .state_root
            .lock()
            .expect("scenario state root mutex poisoned") = Some(root.to_path_buf());
        self.receipt_backed_terminals
            .lock()
            .expect("scenario receipt-backed terminal mutex poisoned")
            .clear();
        *self
            .runtime
            .lock()
            .expect("scenario runtime observer mutex poisoned") = None;
        self.staged_terminal_preparations
            .lock()
            .expect("scenario staged terminal preparation mutex poisoned")
            .clear();
        self.staged_terminal_publications
            .lock()
            .expect("scenario staged terminal publication mutex poisoned")
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

    pub(super) fn receipt_backed_terminals(
        &self,
    ) -> Vec<(TaskTerminalReceiptBackedReceipt, Vec<u8>)> {
        self.receipt_backed_terminals
            .lock()
            .expect("scenario receipt-backed terminal mutex poisoned")
            .clone()
    }

    pub(super) fn set_provider(&self, provider: ScenarioProviderFixture) {
        *self
            .provider
            .lock()
            .expect("scenario provider mutex poisoned") = Some(provider);
    }

    pub(super) fn provider(&self) -> Option<ScenarioProviderFixture> {
        self.provider
            .lock()
            .expect("scenario provider mutex poisoned")
            .clone()
    }

    pub(super) fn record_side_effect_marker(&self) {
        self.side_effect_markers.fetch_add(1, Ordering::AcqRel);
    }

    pub(super) fn side_effect_markers(&self) -> u64 {
        self.side_effect_markers.load(Ordering::Acquire)
    }

    pub(super) fn record_process_exit(&self, elapsed_ms: u64) {
        self.process_exit_elapsed_ms
            .store(elapsed_ms, Ordering::Release);
    }

    pub(super) fn process_exit_elapsed_ms(&self) -> Option<u64> {
        match self.process_exit_elapsed_ms.load(Ordering::Acquire) {
            0 => None,
            elapsed => Some(elapsed),
        }
    }

    pub(super) fn process_exited(&self) -> bool {
        self.process_exit_elapsed_ms.load(Ordering::Acquire) != 0
    }

    pub(super) fn arm_crash_after_side_effect(&self) {
        self.crash_after_side_effect.store(true, Ordering::Release);
    }

    pub(super) fn take_crash_after_side_effect(&self) -> bool {
        self.crash_after_side_effect.swap(false, Ordering::AcqRel)
    }

    pub(super) fn arm_ack_response_disconnect(&self) {
        self.drop_ack_response_after_commit
            .store(true, Ordering::Release);
    }

    pub(super) fn take_ack_response_disconnect(&self) -> bool {
        self.drop_ack_response_after_commit
            .swap(false, Ordering::AcqRel)
    }

    pub(super) fn arm_submit_response_disconnect(&self) {
        self.drop_submit_response_after_commit
            .store(true, Ordering::Release);
    }

    pub(super) fn fail_stop_reclaimed(&self) -> bool {
        self.fail_stop_reclaimed.load(Ordering::Acquire)
    }

    pub(super) fn mark_fail_stop_reclaimed(&self) {
        self.fail_stop_reclaimed.store(true, Ordering::Release);
    }

    /// A restart begins a new process life: a later fail-stop is cleaned again.
    pub(super) fn clear_fail_stop_reclaimed(&self) {
        self.fail_stop_reclaimed.store(false, Ordering::Release);
    }

    pub(super) fn arm_skip_next_startup_reconciliation(&self) {
        self.skip_next_startup_reconciliation
            .store(true, Ordering::Release);
    }

    pub(super) fn take_skip_next_startup_reconciliation(&self) -> bool {
        self.skip_next_startup_reconciliation
            .swap(false, Ordering::AcqRel)
    }

    pub(super) fn take_submit_response_disconnect(&self) -> bool {
        self.drop_submit_response_after_commit
            .swap(false, Ordering::AcqRel)
    }

    pub(super) fn install(&self, point: ScenarioBarrierPoint) {
        let mut barriers = self
            .barriers
            .lock()
            .expect("scenario barrier mutex poisoned");
        barriers.insert(point, ScenarioBarrierState::default());
    }

    pub(super) fn is_installed(&self) -> bool {
        self.barriers
            .lock()
            .expect("scenario barrier mutex poisoned")
            .values()
            .any(|barrier| !barrier.released)
    }

    pub(super) fn is_barrier_installed(&self, point: ScenarioBarrierPoint) -> bool {
        self.barriers
            .lock()
            .expect("scenario barrier mutex poisoned")
            .contains_key(&point)
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

    pub(super) fn wait_until_reached(
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

    pub(super) fn release(&self, point: ScenarioBarrierPoint) {
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

    pub(super) fn configured_precomputed_terminal(&self) -> Result<V5CanonicalTerminal, String> {
        let provider = self
            .provider()
            .ok_or_else(|| "scenario provider is not configured".to_owned())?;
        provider
            .precomputed_terminal
            .ok_or_else(|| "scenario provider has no precomputed cutoff terminal".to_owned())
    }

    pub(super) fn has_precomputed_terminal(&self) -> bool {
        self.provider()
            .is_some_and(|provider| provider.precomputed_terminal.is_some())
    }

    pub(super) fn release_all_barriers(&self) {
        let mut barriers = self
            .barriers
            .lock()
            .expect("scenario barrier mutex poisoned");
        for barrier in barriers.values_mut() {
            barrier.released = true;
        }
        self.changed.notify_all();
    }

    pub(super) fn reset_for_scenario(&self) {
        self.barriers
            .lock()
            .expect("scenario barrier mutex poisoned")
            .clear();
        *self
            .lifecycle_gate_held
            .lock()
            .expect("scenario lifecycle gate mutex poisoned") = false;
        *self
            .gate_cancel_requested
            .lock()
            .expect("scenario gate cancel mutex poisoned") = false;
        *self
            .actor_workspace_identity
            .lock()
            .expect("scenario actor identity mutex poisoned") = None;
        self.bound_tasks
            .lock()
            .expect("scenario bound Task mutex poisoned")
            .clear();
        self.terminal_bound_tasks
            .lock()
            .expect("scenario terminal Task mutex poisoned")
            .clear();
        self.actor_bindings
            .lock()
            .expect("scenario actor binding mutex poisoned")
            .clear();
        self.actor_authorizations
            .lock()
            .expect("scenario actor authorization mutex poisoned")
            .clear();
        self.process_exit_elapsed_ms.store(0, Ordering::Release);
        self.fail_stop_reclaimed.store(false, Ordering::Release);
        self.crash_after_side_effect.store(false, Ordering::Release);
    }

    pub(super) fn has_unreleased_barriers(&self) -> bool {
        self.barriers
            .lock()
            .expect("scenario barrier mutex poisoned")
            .values()
            .any(|barrier| !barrier.released)
    }
}
