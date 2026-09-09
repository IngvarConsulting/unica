//! Runtime and projection operations the contract harness drives directly:
//! seeds, gated probes and the writers the runner still applies itself.

use super::super::*;
use super::scenario_hooks::{
    receipt_key_observation_value, terminal_observation_value, ScenarioHooks,
    V5ReceiptRuntimeTelemetry,
};
use super::ScenarioBarrierPoint;
use crate::application::receipt_ledger::{receipt_key_digest, ProvenTaskLinkCapacity};
use crate::infrastructure::daemon::protocol_v5::V5PendingDirectReceipt;
use serde_json::json;

impl V5TaskProjection {
    pub(super) fn publish_staged_terminal_against_exact_provisional(
        &self,
        staged: &TaskHandoffActorBoundReceipt,
        reservation: &TaskLinkReservation,
        expected: &V5StoredInvocationRecord,
        expected_link_digest: &crate::application::receipt_ledger::TaskLinkDigest,
        deadline: Instant,
    ) -> Result<(V5StoredInvocationRecord, TaskTerminalBoundReceipt), V5TaskProjectionFailure> {
        if reservation.key() != staged.key()
            || reservation.link() != staged.link()
            || reservation.link().digest() != expected_link_digest
            || staged.link().digest() != expected_link_digest
        {
            return Err(V5TaskProjectionFailure::fail_stop(
                ReceiptLedgerError::TaskBoundMismatch,
            ));
        }
        let HandoffTerminalStage::Staged {
            terminal_epoch_ms,
            terminal,
            ..
        } = staged.terminal_stage()
        else {
            return Err(V5TaskProjectionFailure::fail_stop(
                ReceiptLedgerError::TaskBoundMismatch,
            ));
        };
        let provider_deadline = crate::domain::code_intelligence::ProviderDeadline::new(deadline);
        let observed = self
            .task_store
            .get(expected.task_id, provider_deadline)
            .map_err(|error| {
                V5TaskProjectionFailure::from_task_store(error, staged.key_digest().clone(), true)
            })?;
        if observed != *expected {
            return Err(V5TaskProjectionFailure::fail_stop(
                ReceiptLedgerError::TaskBoundMismatch,
            ));
        }
        let task = receipt_task_projection_from_store(&observed)?;
        let bound = self
            .lifecycle_links
            .materialize_task_bound(
                reservation,
                task,
                observed.version,
                *terminal_epoch_ms,
                staged.phase(),
                provider_deadline,
            )
            .map_err(V5TaskProjectionFailure::from_link_store)?;
        let (publication, terminal_status) = match terminal.outcome() {
            ReceiptTerminalOutcome::Completed { result } => (
                V5TerminalPublication::Completed {
                    terminal_epoch_ms: *terminal_epoch_ms,
                    terminal_digest: terminal.digest().clone(),
                    result: result.clone(),
                },
                ClosedTerminalStatus::Completed,
            ),
            ReceiptTerminalOutcome::Failed { reason } => (
                V5TerminalPublication::Failed {
                    terminal_epoch_ms: *terminal_epoch_ms,
                    terminal_digest: terminal.digest().clone(),
                    reason: *reason,
                },
                ClosedTerminalStatus::Failed,
            ),
            ReceiptTerminalOutcome::Cancelled => (
                V5TerminalPublication::Cancelled {
                    terminal_epoch_ms: *terminal_epoch_ms,
                    terminal_digest: terminal.digest().clone(),
                },
                ClosedTerminalStatus::Cancelled,
            ),
        };
        let terminal_record = self
            .task_store
            .publish_staged_terminal_against_exact_provisional(
                expected,
                publication,
                provider_deadline,
            )
            .map_err(|error| {
                V5TaskProjectionFailure::from_task_store(error, staged.key_digest().clone(), true)
            })?;
        let terminal_task = receipt_task_projection_from_store(&terminal_record)?;
        let terminal_link = self
            .lifecycle_links
            .publish_task_terminal_bound(
                &bound,
                terminal_task,
                terminal_record.version,
                terminal_status,
                terminal.digest().clone(),
                *terminal_epoch_ms,
                provider_deadline,
            )
            .map_err(V5TaskProjectionFailure::from_link_store)?;
        Ok((terminal_record, terminal_link))
    }

    pub(super) fn exact_task_link_reservation(
        &self,
        key: &ReceiptKey,
        deadline: Instant,
    ) -> Result<TaskLinkReservation, V5TaskProjectionFailure> {
        let catalog = self
            .lifecycle_links
            .catalog_snapshot(crate::domain::code_intelligence::ProviderDeadline::new(
                deadline,
            ))
            .map_err(V5TaskProjectionFailure::from_link_store)?;
        catalog
            .entries()
            .iter()
            .find_map(|entry| match entry {
                TaskLifecycleLinkCatalogEntry::Reservation(reservation)
                    if reservation.key_digest() == &receipt_key_digest(key) =>
                {
                    Some(reservation.clone())
                }
                TaskLifecycleLinkCatalogEntry::Reservation(_)
                | TaskLifecycleLinkCatalogEntry::Record(_) => None,
            })
            .ok_or_else(|| {
                V5TaskProjectionFailure::fail_stop(ReceiptLedgerError::TaskBoundMismatch)
            })
    }

    pub(super) fn exact_task_record(
        &self,
        key: &ReceiptKey,
        deadline: Instant,
    ) -> Result<V5StoredInvocationRecord, V5TaskProjectionFailure> {
        self.task_store
            .get(
                key.reserved_task_id(),
                crate::domain::code_intelligence::ProviderDeadline::new(deadline),
            )
            .map_err(|error| {
                V5TaskProjectionFailure::from_task_store(error, receipt_key_digest(key), true)
            })
    }
}

impl V5ReceiptRuntime {
    /// Публикация терминала поверх точного provisional и ретирование
    /// собственной квитанции — та же пара, что делает боевой
    /// `publish_staged_handoff_terminal_reply`. Владелец один: попытка,
    /// которую проба и доводит; `Ok(None)` — отказ проекции, его наблюдает
    /// диспетчер.
    pub(super) fn publish_staged_terminal_against_provisional_for_test(
        &self,
        staged: &TaskHandoffActorBoundReceipt,
        reservation: &TaskLinkReservation,
        expected: &V5StoredInvocationRecord,
        expected_link_digest: &crate::application::receipt_ledger::TaskLinkDigest,
        deadline: Instant,
    ) -> Result<Option<(V5StoredInvocationRecord, TaskTerminalBoundReceipt)>, String> {
        let (terminal_record, terminal_link) = match self
            .task_projection
            .publish_staged_terminal_against_exact_provisional(
                staged,
                reservation,
                expected,
                expected_link_digest,
                deadline,
            ) {
            Ok(committed) => committed,
            Err(failure) => {
                let _ = self.project_task_failure(failure);
                return Ok(None);
            }
        };
        self.receipt_ledger
            .complete_staged_task_handoff(
                staged.key().clone(),
                staged.record_version(),
                terminal_link.clone(),
                deadline,
            )
            .map_err(|error| format!("complete exact provisional staged transfer: {error}"))?;
        Ok(Some((terminal_record, terminal_link)))
    }

    pub(super) fn attempt_task_store_bind_under_gate_for_test(
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
        let attempts_before = self
            .scenario_hooks()
            .telemetry
            .snapshot()
            .task_store_create_attempts;
        let checked_sequence = self
            .scenario_hooks()
            .telemetry
            .record_event(V5ReceiptRuntimeEventKind::V5ReceiptRuntimeEntered, epoch_ms);
        let failure = match self.task_projection.materialize_bound_handoff(
            &handoff,
            epoch_ms,
            deadline,
            self.hooks.as_ref(),
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
        let rejected_sequence = self.scenario_hooks().telemetry.record_event(
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
        let staged = match handoff.terminal_stage() {
            HandoffTerminalStage::NoTerminal => None,
            HandoffTerminalStage::Staged {
                terminal,
                certificate,
                ..
            } => Some((terminal.clone(), certificate.clone())),
        };
        let terminal = if let Some((staged_terminal, _)) = &staged {
            let committed = self
                .receipt_ledger
                .publish_receipt_backed_task_terminal(
                    handoff.key().clone(),
                    TaskCancellationReceipt::HandoffActorBound(handoff.clone()),
                    epoch_ms,
                    staged_terminal.clone(),
                    deadline,
                )
                .map_err(|error| {
                    format!("preserve staged Task terminal after link capacity: {error}")
                })?;
            if let Some(control) = &self.scenario_hooks().control {
                control
                    .record_staged_capacity_fallback(&committed)
                    .map_err(|error| format!("record staged capacity fallback: {error}"))?;
                control
                    .record_receipt_backed_terminal(committed.clone())
                    .map_err(|error| format!("record staged link-capacity terminal: {error}"))?;
            }
            self.scenario_hooks().telemetry.record_event(
                V5ReceiptRuntimeEventKind::ReceiptTerminalCommitted,
                epoch_ms,
            );
            Some(committed.terminal().clone())
        } else if handoff.phase() == AttemptPhase::Begun {
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
            if let Some(control) = &self.scenario_hooks().control {
                control
                    .record_receipt_backed_terminal(committed.clone())
                    .map_err(|error| format!("record Task capacity terminal: {error}"))?;
            }
            self.scenario_hooks().telemetry.record_event(
                V5ReceiptRuntimeEventKind::ReceiptTerminalCommitted,
                epoch_ms,
            );
            Some(committed.terminal().clone())
        };
        let attempts_after = self
            .scenario_hooks()
            .telemetry
            .snapshot()
            .task_store_create_attempts;
        let terminal_observation = terminal
            .as_ref()
            .map(|terminal| terminal_observation_value(terminal, epoch_ms));
        let staged_certificate_sha256 = staged
            .as_ref()
            .map(|(_, certificate)| {
                serde_json::to_vec(certificate.as_ref())
                    .map(|bytes| format!("{:x}", Sha256::digest(bytes)))
                    .map_err(|error| format!("encode staged transfer certificate: {error}"))
            })
            .transpose()?;
        let response = json!({
            "kind": if terminal.is_some() { "task" } else { "rejected" },
            "error": if staged.is_some() { Value::Null } else { Value::String("task_capacity".to_owned()) },
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
            "stagedTransferCertificateSha256": staged_certificate_sha256,
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

    pub(super) fn stage_bound_handoff_terminal_for_test(
        &self,
        key: &ReceiptKey,
        terminal: crate::application::receipt_ledger::V5CanonicalTerminal,
        deadline: Instant,
    ) -> Result<(), String> {
        let epoch_ms = self.epoch_clock.now_epoch_millis();
        let handoff = match self
            .receipt_ledger
            .recover(key.clone(), deadline)
            .map_err(|error| format!("recover handoff before staging terminal: {error}"))?
        {
            ReceiptState::TaskHandoffActorBound(handoff) => handoff,
            other => {
                return Err(format!(
                    "staging terminal requires actor-bound Task handoff, found {}",
                    other.kind().diagnostic_name()
                ))
            }
        };
        let certificate = canonical_staged_transfer_certificate(
            handoff.key(),
            handoff.key_digest(),
            handoff.link(),
            epoch_ms,
            &terminal,
        )
        .map_err(|error| format!("prepare staged transfer certificate: {error}"))?;
        let staged = self
            .receipt_ledger
            .stage_bound_task_handoff_terminal(
                handoff.key().clone(),
                handoff.record_version(),
                epoch_ms,
                terminal,
                certificate,
                deadline,
            )
            .map_err(|error| format!("stage bound Task handoff terminal: {error}"))?;
        if let Some(control) = &self.scenario_hooks().control {
            control
                .record_staged_terminal_preparation(&staged)
                .map_err(|error| format!("record staged terminal preparation: {error}"))?;
        }
        self.scenario_hooks().telemetry.record_event(
            V5ReceiptRuntimeEventKind::BoundHandoffTerminalStaged,
            epoch_ms,
        );
        Ok(())
    }

    /// Drives the unstaged completed-handoff bind against a predecessor that already
    /// carries a certified staged terminal — the shape every caller that routes on the
    /// `ReceiptState` variant alone produces. Reports whether the ledger refused it.
    pub(super) fn attempt_unstaged_task_bind_against_staged_terminal_for_test(
        &self,
        key: &ReceiptKey,
        deadline: Instant,
    ) -> Result<bool, String> {
        let handoff = match self
            .receipt_ledger
            .recover(key.clone(), deadline)
            .map_err(|error| format!("recover staged handoff before unstaged bind: {error}"))?
        {
            ReceiptState::TaskHandoffActorBound(handoff)
                if matches!(
                    handoff.terminal_stage(),
                    HandoffTerminalStage::Staged { .. }
                ) =>
            {
                handoff
            }
            other => {
                return Err(format!(
                    "unstaged bind attempt requires a staged Task handoff, found {}",
                    other.kind().diagnostic_name()
                ))
            }
        };
        let (_, bound) = self
            .task_projection
            .materialize_bound_handoff(&handoff, self.epoch_ms(), deadline, self.hooks.as_ref())
            .map_err(|failure| format!("materialize unstaged bound Task: {}", failure.error))?;
        match self.receipt_ledger.complete_bound_task_handoff(
            handoff.key().clone(),
            handoff.record_version(),
            bound,
            deadline,
        ) {
            Ok(_) => Ok(false),
            Err(ReceiptLedgerError::ReceiptRowPresentUnsupported) => Ok(true),
            Err(error) => Err(format!("attempt unstaged Task handoff completion: {error}")),
        }
    }

    pub(super) fn inject_task_store_capacity_invariant_for_test(
        &self,
        key: &ReceiptKey,
        operation_label: &str,
        deadline: Instant,
    ) -> Result<Value, String> {
        let epoch_ms = self.epoch_clock.now_epoch_millis();
        let handoff = match self
            .receipt_ledger
            .recover(key.clone(), deadline)
            .map_err(|error| format!("recover staged handoff before capacity invariant: {error}"))?
        {
            ReceiptState::TaskHandoffActorBound(handoff) => handoff,
            other => {
                return Err(format!(
                    "capacity invariant injection requires actor-bound Task handoff, found {}",
                    other.kind().diagnostic_name()
                ))
            }
        };
        let (terminal_epoch_ms, staged_terminal, certificate) = match handoff.terminal_stage() {
            HandoffTerminalStage::Staged {
                terminal_epoch_ms,
                terminal,
                certificate,
            } => (*terminal_epoch_ms, terminal, certificate),
            HandoffTerminalStage::NoTerminal => {
                return Err("capacity invariant injection requires a staged terminal".to_owned())
            }
        };
        let attempts_before = self
            .scenario_hooks()
            .telemetry
            .snapshot()
            .task_store_create_attempts;
        let task_records = u64::try_from(self.task_projection.recovery.entries().len())
            .map_err(|_| "TaskStore record count does not fit u64".to_owned())?;
        let reservation = self
            .task_projection
            .reserve_bound_handoff_link(&handoff, epoch_ms, deadline, self.hooks.as_ref())
            .map_err(|failure| format!("reserve invariant lifecycle link: {}", failure.error))?;
        let reserved_sequence = self
            .scenario_hooks()
            .telemetry
            .snapshot()
            .events
            .last()
            .filter(|event| event.event == V5ReceiptRuntimeEventKind::TaskLinkCapacityReserved)
            .map(|event| event.sequence)
            .ok_or_else(|| "capacity invariant reservation event is missing".to_owned())?;
        self.scenario_hooks()
            .telemetry
            .record_task_store_create_attempt();
        let create_sequence = self.scenario_hooks().telemetry.record_event(
            V5ReceiptRuntimeEventKind::TaskStoreCreateAttempted,
            epoch_ms,
        );
        let injected = V5TaskProjectionFailure::from_task_store(
            V5TaskStoreError::Capacity {
                max_records: crate::application::invocation_store_v5::MAX_V5_TASK_RECORDS,
            },
            handoff.key_digest().clone(),
            true,
        );
        if !injected.fail_stop {
            return Err(
                "injected post-reservation TaskStore Capacity did not fail-stop".to_owned(),
            );
        }
        let capacity_sequence = self.scenario_hooks().telemetry.record_event(
            V5ReceiptRuntimeEventKind::TaskStoreCapacityInvariantViolation,
            epoch_ms,
        );
        self.scenario_hooks().telemetry.record_forced_process_exit();
        let listener_closed_sequence = self
            .scenario_hooks()
            .telemetry
            .snapshot()
            .events
            .last()
            .filter(|event| event.event == V5ReceiptRuntimeEventKind::ListenerClosed)
            .map(|event| event.sequence)
            .ok_or_else(|| "capacity invariant listener-close event is missing".to_owned())?;
        let restart_requested_sequence = listener_closed_sequence.saturating_add(1);
        let daemon_stopped_sequence = listener_closed_sequence.saturating_add(2);
        let lifecycle = self
            .task_projection
            .lifecycle_links
            .catalog_snapshot(crate::domain::code_intelligence::ProviderDeadline::new(
                deadline,
            ))
            .map_err(|error| format!("inspect invariant lifecycle reservation: {error}"))?;
        let certificate_bytes = serde_json::to_vec(certificate.as_ref())
            .map_err(|error| format!("encode invariant staged certificate: {error}"))?;
        let reservation_fingerprint = format!(
            "{:x}",
            Sha256::digest(format!(
                "{}:{}:{}:{}",
                reservation.key_digest(),
                reservation.link().digest(),
                reservation.reservation_version(),
                reservation.mutation_sequence()
            ))
        );
        let materialized_links = lifecycle.count().saturating_sub(lifecycle.reserved_count());
        Ok(json!({
            "operationLabel": operation_label,
            "receiptKey": receipt_key_observation_value(key),
            "stagedTerminal": terminal_observation_value(staged_terminal, terminal_epoch_ms),
            "stagedTransferCertificateSha256": format!("{:x}", Sha256::digest(certificate_bytes)),
            "liveTaskLinkReservationFingerprint": reservation_fingerprint,
            "taskLinkReservedSequence": reserved_sequence,
            "taskStoreCreateSequence": create_sequence,
            "capacityObservedSequence": capacity_sequence,
            "listenerClosedSequence": listener_closed_sequence,
            "restartRequestedSequence": restart_requested_sequence,
            "daemonStoppedSequence": daemon_stopped_sequence,
            "taskStoreRecordCountBefore": task_records,
            "taskStoreRecordCountAfter": task_records,
            "materializedLifecycleLinkCountBefore": materialized_links,
            "materializedLifecycleLinkCountAfter": materialized_links,
            "liveLinkReservationCountBefore": lifecycle.reserved_count(),
            "liveLinkReservationCountAfter": lifecycle.reserved_count(),
            "taskStoreGenerationBefore": task_records,
            "taskStoreGenerationAfter": task_records,
            "taskStoreCreateAttemptsBefore": attempts_before,
            "taskStoreCreateAttemptsAfter": attempts_before.saturating_add(1),
        }))
    }

    pub(super) fn seed_cancel_reserved_pool_entry_for_test(
        &self,
        key: ReceiptKey,
        epoch_ms: u64,
        deadline: Instant,
    ) -> Result<(), ReceiptLedgerError> {
        self.scenario_hooks()
            .telemetry
            .record_event(V5ReceiptRuntimeEventKind::V5ReceiptRuntimeEntered, epoch_ms);
        self.receipt_ledger
            .request_cancel_or_reserve(key, epoch_ms, deadline)?;
        Ok(())
    }

    pub(super) fn submit_direct_batch_for_load(
        &self,
        work: Vec<(ReceiptKey, V5InvocationRequest, u64)>,
        deadline: Instant,
    ) -> Result<Vec<V5PendingDirectReceipt>, ReceiptLedgerError> {
        if work.is_empty() || work.len() > 32 {
            return Err(ReceiptLedgerError::ReceiptRowPresentUnsupported);
        }
        let reservations = self.receipt_ledger.reserve_batch(
            work.iter()
                .map(|(key, _, epoch_ms)| {
                    OriginalCutoffDescriptor::new(*epoch_ms, 7_000)
                        .map(|cutoff| (key.clone(), cutoff))
                        .map_err(|_| ReceiptLedgerError::TimestampOverflow)
                })
                .collect::<Result<Vec<_>, _>>()?,
            deadline,
        )?;
        let reservations = reservations
            .into_iter()
            .map(|outcome| {
                outcome.into_reservation().map_err(|_| {
                    ReceiptLedgerError::Corrupt("direct load batch reserve was not newly created")
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let batch_epoch_ms =
            work.first()
                .map(|(_, _, epoch_ms)| *epoch_ms)
                .ok_or(ReceiptLedgerError::Corrupt(
                    "direct load batch unexpectedly had no work",
                ))?;
        // The load report retains per-invocation lifecycle and callback counts. Record the
        // repeated phase boundaries once per durable writer batch so the diagnostic event trace
        // remains bounded at the full retention horizon.
        self.scenario_hooks()
            .telemetry
            .record_event(V5ReceiptRuntimeEventKind::V5ExecutorEntered, batch_epoch_ms);
        self.scenario_hooks().telemetry.record_event(
            V5ReceiptRuntimeEventKind::V5ReceiptRuntimeEntered,
            batch_epoch_ms,
        );
        self.scenario_hooks()
            .telemetry
            .record_event(V5ReceiptRuntimeEventKind::ValidationEntered, batch_epoch_ms);
        self.scenario_hooks()
            .telemetry
            .record_event(V5ReceiptRuntimeEventKind::AdmissionEntered, batch_epoch_ms);
        for _ in &work {
            self.scenario_hooks().telemetry.record_validation();
            self.scenario_hooks().telemetry.record_admission();
        }
        // The transport admits these calls concurrently. Keep every invocation's own validation
        // and actor binding, but do not serialize that admission ahead of the durable writer
        // batch. The scope joins every binding before the ledger can advance to ActorBound.
        let actor_bounds = thread::scope(|scope| {
            let bindings = work
                .iter()
                .map(|(_, invocation, _)| {
                    scope.spawn(|| {
                        let response_deadline = self
                            .invocation_executor
                            .capture_response_deadline(invocation.response_budget_ms());
                        self.invocation_executor
                            .bind(invocation.clone(), response_deadline)
                    })
                })
                .collect::<Vec<_>>();
            bindings
                .into_iter()
                .map(|binding| {
                    binding
                        .join()
                        .map_err(|_| {
                            ReceiptLedgerError::Corrupt(
                                "direct load batch invocation binding panicked",
                            )
                        })?
                        .map_err(|_| {
                            ReceiptLedgerError::Corrupt("direct load batch invocation did not bind")
                        })
                })
                .collect::<Result<Vec<_>, _>>()
        })?;
        let bound = self.receipt_ledger.bind_reserved_actor_batch(
            reservations
                .iter()
                .zip(&actor_bounds)
                .map(|(reservation, actor_bound)| {
                    (
                        reservation.key().clone(),
                        reservation.record_version(),
                        actor_bound.workspace_identity_hash().clone(),
                    )
                })
                .collect(),
            deadline,
        )?;
        let begun = self.receipt_ledger.mark_reserved_begun_batch(
            bound
                .iter()
                .zip(&actor_bounds)
                .map(|(bound, actor_bound)| {
                    (
                        bound.key().clone(),
                        bound.record_version(),
                        actor_bound.workspace_identity_hash().clone(),
                    )
                })
                .collect(),
            deadline,
        )?;
        drop(actor_bounds);
        self.scenario_hooks()
            .telemetry
            .record_event(V5ReceiptRuntimeEventKind::PrepareEntered, batch_epoch_ms);
        self.scenario_hooks()
            .telemetry
            .record_event(V5ReceiptRuntimeEventKind::ExecuteEntered, batch_epoch_ms);
        let mut terminal_requests = Vec::with_capacity(work.len());
        for ((key, _, epoch_ms), begun) in work.iter().zip(&begun) {
            self.scenario_hooks().telemetry.record_prepare();
            self.scenario_hooks().telemetry.record_execute();
            let outcome = match self
                .scenario_hooks()
                .control
                .as_ref()
                .ok_or(ReceiptLedgerError::Corrupt(
                    "direct load callback owner is unavailable",
                ))?
                .execute_direct_load_callback(key.invocation_id())
            {
                Ok(result) => ReceiptTerminalOutcome::Completed {
                    result: Box::new(result),
                },
                Err(_) => ReceiptTerminalOutcome::Failed {
                    reason: V5SafeFailureReason::InvocationFailed,
                },
            };
            let terminal = canonical_v5_terminal(&outcome)
                .map_err(|_| ReceiptLedgerError::Corrupt("canonical v5 terminal failed"))?;
            terminal_requests.push((
                begun.key().clone(),
                begun.record_version(),
                *epoch_ms,
                terminal,
            ));
        }
        let publications = self
            .receipt_ledger
            .publish_direct_terminal_batch(terminal_requests, deadline)?;
        publications
            .into_iter()
            .zip(work)
            .map(|(publication, (key, _, epoch_ms))| {
                let receipt = publication.receipt();
                self.scenario_hooks().telemetry.record_event(
                    V5ReceiptRuntimeEventKind::ReceiptTerminalCommitted,
                    epoch_ms,
                );
                Ok(V5PendingDirectReceipt::new(
                    key,
                    receipt.terminal().outcome().clone(),
                    receipt.terminal().digest().clone(),
                    receipt.terminal_epoch_ms(),
                ))
            })
            .collect()
    }

    pub(super) fn seed_reserved_pool_entry_for_test(
        &self,
        key: ReceiptKey,
        cutoff: OriginalCutoffDescriptor,
        epoch_ms: u64,
        deadline: Instant,
    ) -> Result<(), ReceiptLedgerError> {
        self.scenario_hooks()
            .telemetry
            .record_event(V5ReceiptRuntimeEventKind::V5ReceiptRuntimeEntered, epoch_ms);
        self.receipt_ledger.reserve(key, cutoff, deadline)?;
        Ok(())
    }

    pub(super) fn seed_receipt_backed_terminal_pool_entry_for_test(
        &self,
        key: ReceiptKey,
        epoch_ms: u64,
        task_ttl_ms: u64,
        task_poll_interval_ms: u64,
        terminal: crate::application::receipt_ledger::V5CanonicalTerminal,
        deadline: Instant,
    ) -> Result<(), ReceiptLedgerError> {
        self.scenario_hooks()
            .telemetry
            .record_event(V5ReceiptRuntimeEventKind::V5ReceiptRuntimeEntered, epoch_ms);
        let reserved = self
            .receipt_ledger
            .reserve(
                key.clone(),
                OriginalCutoffDescriptor::new(epoch_ms, 7_000)
                    .map_err(|_| ReceiptLedgerError::TimestampOverflow)?,
                deadline,
            )?
            .into_reservation()
            .map_err(|_| ReceiptLedgerError::Corrupt("fixture receipt already exists"))?;
        let promised = self.receipt_ledger.promise_task_unbound(
            key.clone(),
            reserved.record_version(),
            epoch_ms,
            task_ttl_ms,
            task_poll_interval_ms,
            deadline,
        )?;
        self.receipt_ledger.publish_receipt_backed_task_terminal(
            key,
            TaskCancellationReceipt::PromisedUnbound(promised),
            epoch_ms,
            terminal,
            deadline,
        )?;
        Ok(())
    }

    pub(super) fn mark_reserved_begun_under_gate_for_test(
        &self,
        key: &ReceiptKey,
        operation_label: &str,
        deadline: Instant,
    ) -> Result<(), String> {
        self.scenario_hooks().telemetry.record_event(
            V5ReceiptRuntimeEventKind::V5ReceiptRuntimeEntered,
            self.epoch_ms(),
        );
        let control = self
            .scenario_hooks()
            .control
            .as_ref()
            .ok_or_else(|| "reserved-begin scenario control is unavailable".to_owned())?;
        if control.is_barrier_installed(ScenarioBarrierPoint::BeforeMarkReservedBegunGateAcquire) {
            control.record_operation_event(operation_label, "blocked");
            self.scenario_hooks().telemetry.record_event(
                V5ReceiptRuntimeEventKind::MarkReservedBegunBlocked,
                self.epoch_ms(),
            );
            control
                .pause(
                    ScenarioBarrierPoint::BeforeMarkReservedBegunGateAcquire,
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
            self.scenario_hooks().telemetry.record_event(
                V5ReceiptRuntimeEventKind::ReceiptBegunCommitted,
                self.epoch_ms(),
            );
            self.scenario_hooks()
                .telemetry
                .record_event(V5ReceiptRuntimeEventKind::TokenSignalled, self.epoch_ms());
            Ok(())
        })();
        control.release_lifecycle_gate(operation_label);
        self.scenario_hooks().telemetry.record_event(
            V5ReceiptRuntimeEventKind::OperationCompleted,
            self.epoch_ms(),
        );
        result
    }

    pub(super) fn cancel_under_gate_for_test(
        &self,
        key: &ReceiptKey,
        operation_label: &str,
        deadline: Instant,
    ) -> Result<(), String> {
        self.scenario_hooks().telemetry.record_event(
            V5ReceiptRuntimeEventKind::V5ReceiptRuntimeEntered,
            self.epoch_ms(),
        );
        let control = self
            .scenario_hooks()
            .control
            .as_ref()
            .ok_or_else(|| "cancel scenario control is unavailable".to_owned())?;
        if control.is_barrier_installed(ScenarioBarrierPoint::BeforeCancelGateAcquire) {
            control.record_operation_event(operation_label, "blocked");
            self.scenario_hooks().telemetry.record_event(
                V5ReceiptRuntimeEventKind::CancelCommitBlocked,
                self.epoch_ms(),
            );
            control
                .pause(ScenarioBarrierPoint::BeforeCancelGateAcquire, deadline)
                .map_err(|error| format!("wait before cancel lifecycle gate: {error}"))?;
        }
        control
            .acquire_lifecycle_gate(operation_label, deadline)
            .map_err(|error| format!("acquire cancel lifecycle gate: {error}"))?;
        let result = (|| {
            self.cancel_invocation(key.clone(), self.epoch_ms(), deadline)
                .map_err(|error| format!("cancel under lifecycle gate: {error}"))?;
            match self.receipt_ledger.recover(key.clone(), deadline) {
                Ok(ReceiptState::Reserved(reserved))
                    if matches!(reserved.phase(), ReservedPhase::ActorBound { .. })
                        && reserved.cancel_requested() =>
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
                    self.scenario_hooks().telemetry.record_event(
                        V5ReceiptRuntimeEventKind::ReceiptTerminalCommitted,
                        self.epoch_ms(),
                    );
                }
                Ok(_) | Err(ReceiptLedgerError::ReceiptNotFound) => {}
                Err(error) => return Err(format!("recover cancel winner: {error}")),
            }
            Ok(())
        })();
        if result.is_ok() {
            self.scenario_hooks()
                .telemetry
                .record_event(V5ReceiptRuntimeEventKind::CancelCommitted, self.epoch_ms());
        }
        control.release_lifecycle_gate(operation_label);
        self.scenario_hooks().telemetry.record_event(
            V5ReceiptRuntimeEventKind::OperationCompleted,
            self.epoch_ms(),
        );
        result
    }

    pub(super) fn bind_task_under_gate_for_test(
        &self,
        key: &ReceiptKey,
        operation_label: &str,
        deadline: Instant,
    ) -> Result<(), String> {
        self.scenario_hooks().telemetry.record_event(
            V5ReceiptRuntimeEventKind::V5ReceiptRuntimeEntered,
            self.epoch_ms(),
        );
        let control = self
            .scenario_hooks()
            .control
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
                .materialize_bound_handoff(&handoff, self.epoch_ms(), deadline, self.hooks.as_ref())
                .map_err(|failure| format!("materialize actor-bound Task: {}", failure.error))?;
            control.record_bound_task(record.clone(), bound.clone());
            self.scenario_hooks().telemetry.record_event(
                V5ReceiptRuntimeEventKind::TaskStoreReadbackBeforeBind,
                self.epoch_ms(),
            );
            if control
                .is_barrier_installed(ScenarioBarrierPoint::AfterTaskStoreReadbackBeforeTaskBound)
            {
                control.record_operation_event(operation_label, "blocked");
                control
                    .pause(
                        ScenarioBarrierPoint::AfterTaskStoreReadbackBeforeTaskBound,
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
            self.scenario_hooks().telemetry.record_event(
                V5ReceiptRuntimeEventKind::TaskBoundCommitted,
                self.epoch_ms(),
            );
            Ok(())
        })();
        control.release_lifecycle_gate(operation_label);
        self.scenario_hooks().telemetry.record_event(
            V5ReceiptRuntimeEventKind::OperationCompleted,
            self.epoch_ms(),
        );
        result
    }

    pub(super) fn continue_receipt_owned_attempt_for_test(
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
        if let Some(control) = &self.scenario_hooks().control {
            control
                .record_receipt_backed_terminal(committed.clone())
                .map_err(|error| format!("record receipt-owned Task terminal: {error}"))?;
        }
        self.scenario_hooks().telemetry.record_event(
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

    pub(super) fn with_hooks_for_test(mut self, hooks: Arc<dyn V5RuntimeHooks>) -> Self {
        self.hooks = hooks;
        self
    }

    /// The telemetry of the installed observer, for writers the harness
    /// still applies itself.
    pub(super) fn scenario_telemetry(&self) -> &V5ReceiptRuntimeTelemetry {
        &self.scenario_hooks().telemetry
    }

    /// Whether a fail-stop watchdog is due on the runtime's clock: the accept
    /// loop exits on it, and the harness waits for that exit.
    /// How many promoted attempts the owner handed to a worker are still
    /// running their continuation off the reply thread.
    pub(super) fn promoted_continuation_in_flight_for_test(&self) -> usize {
        self.promoted_continuations
            .load(std::sync::atomic::Ordering::Acquire)
    }

    pub(super) fn fail_stop_due_for_test(&self) -> bool {
        self.fail_stop_watchdogs
            .due(self.invocation_executor.now())
            .is_some()
    }

    /// The observer the harness installed; probes read its telemetry and
    /// control directly.
    fn scenario_hooks(&self) -> &ScenarioHooks {
        self.hooks
            .as_any()
            .downcast_ref::<ScenarioHooks>()
            .expect("scenario probes run under scenario hooks")
    }
}

pub(super) fn receipt_state_task_snapshot_for_test(
    state: ReceiptState,
) -> Result<crate::infrastructure::daemon::protocol_v5::V5DaemonTaskSnapshot, ReceiptLedgerError> {
    receipt_state_task_snapshot(state)
}
