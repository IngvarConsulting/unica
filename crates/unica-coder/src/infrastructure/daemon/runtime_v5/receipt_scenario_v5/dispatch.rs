//! Диспетчер действий сценария: разбирает провод, поднимает демона под
//! каждое действие и записывает в отчёт то, что рантайм ответил. Сам он
//! не делает ни одного durable-перехода квитанции — правило
//! `INV.TEST.LEDGER-HARNESS-OBSERVES`, страж
//! `scripts/ci/check-receipt-harness-boundary.py`.

use super::*;

pub(super) fn unsupported_shape(shape: &str) -> String {
    format!("protocol-v5 receipt scenario shape is not supported: {shape}")
}

pub(crate) fn run_supported_receipt_scenario_for_test(request: &str) -> Result<String, String> {
    let scenario = match serde_json::from_str::<ReceiptScenario>(request) {
        Ok(scenario) => scenario,
        Err(error) => return Err(format!("decode protocol-v5 receipt scenario: {error}")),
    };
    if scenario.actions.is_empty() {
        return Err("protocol-v5 receipt scenario has no actions".to_owned());
    }
    if scenario
        .actions
        .iter()
        .all(|action| matches!(action, ReceiptScenarioAction::ProbeProtocol { .. }))
    {
        return run_protocol_probe_scenario(scenario);
    }
    let mut state = ScenarioStateRoot::new()?;
    let workspace = ScenarioWorkspace::new()?;
    let workspace_hint = workspace.hint().to_owned();
    let identity = CoreIdentity::production_v5();
    let clock = Arc::new(ScenarioEpochClock::new(
        SCENARIO_INITIAL_EPOCH_MS,
        matches!(scenario.clock, ScenarioClock::Wall),
    ));
    let mut arguments = Map::new();
    arguments.insert(
        "at".to_owned(),
        Value::String("main:Configuration".to_owned()),
    );
    let mut invocation_id = InvocationId::new();
    let mut reserved_task_id = TaskId::new();
    let mut exact_key = ReceiptKey::new(
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
    let mut mismatched_arguments_key = {
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
    for action in &scenario.actions {
        if let ReceiptScenarioAction::InvalidateActorProof { point, .. } = action {
            control.install(*point);
        }
        if matches!(
            action,
            ReceiptScenarioAction::Crash {
                point: ScenarioCrashPoint::AfterTaskStoreTerminalBeforeLifecycleLinkTerminal
            }
        ) {
            control
                .install(ScenarioBarrierPoint::AfterTaskStoreTerminalBeforeLifecycleLinkTerminal);
        }
    }
    let initial_daemon_state = DaemonStateDirectory::open(state.path(), &identity)?;
    let initial_receipts = initial_daemon_state.create_private_retained_subdirectory("receipts")?;
    control.set_state_root(initial_receipts.path());
    let telemetry = Arc::new(V5ReceiptRuntimeTelemetry::new());
    let mut known_keys = Vec::new();
    let mut submit_keys: HashMap<String, ReceiptKey> = HashMap::new();
    let mut original_submit_cutoff: Option<(u64, u64)> = None;
    let mut pending_submit: Option<PendingSubmit> = None;
    let mut spawned_submit_clients: HashMap<String, SpawnedSubmitClient> = HashMap::new();
    let mut spawn_submit_labels: HashSet<String> = HashSet::new();
    let mut live_daemon: Option<ScenarioDaemon> = None;
    let mut live_actor: Option<ReceiptLedgerActor> = None;
    let mut live_task_projection: Option<(TaskProjectionObservation, u64)> = None;
    let mut pending_duplicate_labels = Vec::new();
    let mut deferred_task_bound: Option<ScenarioSeedReceiptState> = None;
    let mut startup_failed = false;
    let mut listener_published = false;
    let mut startup_listener_override = false;
    let mut seeded_task_versions = HashMap::new();
    let mut corrupted_identity_snapshot: Option<Value> = None;
    let mut bulk_task_projection: Option<TaskProjectionObservation> = None;
    let mut bulk_receipt_snapshot: Option<Value> = None;
    let mut bulk_receipt_catalog: Option<BulkReceiptCatalogObservation> = None;
    let mut operation_runtime: Option<Arc<V5ReceiptRuntime>> = None;
    let mut operations: HashMap<String, ScenarioOperation> = HashMap::new();
    let mut inject_task_store_capacity_invariant_once = false;
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
                let precomputed_terminal = matches!(
                    terminal,
                    ScenarioTerminalFixture::NearLimitWithMaximumMetadata { .. }
                )
                .then(|| {
                    let result = domain_result_for_fixture(&terminal)?;
                    canonical_v5_terminal(&ReceiptTerminalOutcome::Completed {
                        result: Box::new(result),
                    })
                    .map_err(|error| format!("precompute scenario cutoff terminal: {error}"))
                })
                .transpose()?;
                control.set_provider(ScenarioProviderFixture {
                    execution_class,
                    terminal: terminal.clone(),
                    precomputed_terminal,
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
                        | ScenarioSeedReceiptState::TaskTerminalBound
                ) {
                    let valid = match seed_state {
                        ScenarioSeedReceiptState::TaskTerminalBound => {
                            !cancel_requested
                                && staged_terminal.as_ref().is_some_and(|terminal| {
                                    matches!(terminal, ScenarioTerminalFixture::Success { .. })
                                })
                        }
                        _ => !cancel_requested && staged_terminal.is_none(),
                    };
                    if !valid {
                        return Err(unsupported_shape("seed_receipt fixture combination"));
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
                        return Err(unsupported_shape("seed_receipt fixture state"));
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
            ReceiptScenarioAction::AttemptStagedTerminalAgainstProvisional {
                mismatch,
                repeat_same_terminal,
                label,
            } => {
                control.arm_skip_next_startup_reconciliation();
                let daemon_state = DaemonStateDirectory::open(state.path(), &identity)?;
                let config = scenario_server_config_with_clock(
                    state.path(),
                    &identity,
                    Some(&control),
                    &clock,
                );
                let runtime =
                    V5ReceiptRuntime::open_with_epoch_clock(&daemon_state, &config, clock.clone())?
                        .with_hooks_for_test(ScenarioHooks::install(
                            Arc::clone(&telemetry),
                            Some(Arc::clone(&control)),
                        ));
                let deadline = Instant::now() + SCENARIO_OPERATION_TIMEOUT;
                let staged = match runtime.receipt_ledger.recover(exact_key.clone(), deadline) {
                    Ok(ReceiptState::TaskHandoffActorBound(staged))
                        if matches!(
                            staged.terminal_stage(),
                            HandoffTerminalStage::Staged { .. }
                        ) =>
                    {
                        staged
                    }
                    Ok(other) => {
                        return Err(format!(
                            "staged provisional transfer found {}",
                            other.kind().diagnostic_name()
                        ))
                    }
                    Err(error) => {
                        return Err(format!("recover staged provisional transfer: {error}"))
                    }
                };
                control.record_staged_terminal_preparation(&staged)?;
                let reservation = runtime
                    .task_projection
                    .exact_task_link_reservation(&exact_key, deadline)
                    .map_err(|failure| {
                        format!("read staged provisional reservation: {}", failure.error)
                    })?;
                let mut expected = runtime
                    .task_projection
                    .exact_task_record(&exact_key, deadline)
                    .map_err(|failure| {
                        format!("read staged provisional Task: {}", failure.error)
                    })?;
                let mut expected_link_digest = staged.link().digest().clone();
                match mismatch {
                    Some(ScenarioProvisionalMismatchField::TaskId) => {
                        expected.task_id = TaskId::new();
                    }
                    Some(ScenarioProvisionalMismatchField::InvocationId) => {
                        expected.invocation_id = InvocationId::new();
                    }
                    Some(ScenarioProvisionalMismatchField::Status) => {
                        expected.task = match expected.task {
                            V5StoredTask::Queued => V5StoredTask::Working,
                            V5StoredTask::Working => V5StoredTask::Queued,
                            _ => {
                                return Err(
                                    "staged provisional mismatch target is terminal".to_owned()
                                )
                            }
                        };
                    }
                    Some(ScenarioProvisionalMismatchField::Version) => {
                        expected.version = expected.version.saturating_add(1);
                    }
                    Some(ScenarioProvisionalMismatchField::CancelRequested) => {
                        expected.cancel_requested = !expected.cancel_requested;
                    }
                    Some(ScenarioProvisionalMismatchField::TaskLinkDigest) => {
                        expected_link_digest = "f".repeat(64).parse().map_err(|error| {
                            format!("construct mismatched TaskLink digest: {error}")
                        })?;
                    }
                    None => {}
                }
                let provisional = expected.clone();
                let publication = runtime
                    .task_projection
                    .publish_staged_terminal_against_exact_provisional(
                        &staged,
                        &reservation,
                        &expected,
                        &expected_link_digest,
                        deadline,
                    );
                let (terminal_record, terminal_link) = match publication {
                    Ok(committed) => committed,
                    Err(failure) => {
                        let _ = runtime.project_task_failure(failure);
                        control.record_operation_event(&label, "completed");
                        telemetry.record_forced_process_exit();
                        drop(runtime);
                        continue;
                    }
                };
                runtime
                    .receipt_ledger
                    .complete_staged_task_handoff(
                        staged.key().clone(),
                        staged.record_version(),
                        terminal_link.clone(),
                        deadline,
                    )
                    .map_err(|error| {
                        format!("complete exact provisional staged transfer: {error}")
                    })?;
                control.record_staged_terminal_publication(
                    &staged,
                    &provisional,
                    &terminal_record,
                    &terminal_link,
                )?;
                control.record_terminal_bound_task(terminal_record.clone(), terminal_link);
                if repeat_same_terminal {
                    let generation_before = terminal_record.version;
                    let repeated = runtime
                        .task_projection
                        .task_store
                        .publish_staged_terminal_against_exact_provisional(
                            &provisional,
                            staged_terminal_publication(&staged)?,
                            crate::domain::code_intelligence::ProviderDeadline::new(deadline),
                        )
                        .map_err(|error| {
                            format!("repeat exact staged terminal publication: {error}")
                        })?;
                    control.record_staged_terminal_idempotent_repeat(
                        &exact_key,
                        &repeated,
                        generation_before,
                        repeated.version,
                    )?;
                }
                push_known_key(&mut known_keys, exact_key.clone());
                control.record_operation_event(&label, "completed");
                drop(runtime);
            }
            ReceiptScenarioAction::InjectStoreFault { point } => {
                telemetry.arm_store_fault(point);
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
            ReceiptScenarioAction::InjectPersistedIdentityCollision { index } => {
                seed_identity_collision_receipt(
                    state.path(),
                    &identity,
                    exact_key.clone(),
                    clock.now_epoch_millis(),
                )?;
                push_known_key(&mut known_keys, exact_key.clone());
                let snapshot = snapshot_from_state(
                    state.path(),
                    &identity,
                    Arc::clone(&clock),
                    &telemetry,
                    &control,
                    &known_keys,
                )?;
                corrupt_receipt_identity_index(
                    state.path(),
                    &identity,
                    matches!(index, ScenarioIdentityIndex::InvocationId),
                )?;
                corrupted_identity_snapshot = Some(snapshot);
            }
            ReceiptScenarioAction::ReconcileStartup => {
                startup_listener_override = true;
                let daemon_state = DaemonStateDirectory::open(state.path(), &identity)?;
                let config =
                    scenario_server_config_with_clock(state.path(), &identity, None, &clock);
                match V5ReceiptRuntime::open_with_epoch_clock(
                    &daemon_state,
                    &config
                        .clone()
                        .with_runtime_hooks_for_test(ScenarioHooks::install(
                            Arc::clone(&telemetry),
                            Some(Arc::clone(&control)),
                        )),
                    clock.clone(),
                ) {
                    Ok(runtime) => {
                        bulk_receipt_snapshot = match &bulk_receipt_catalog {
                            Some(catalog) => Some(snapshot_with_actor_and_bulk_catalog(
                                &runtime.receipt_ledger,
                                &clock,
                                &telemetry,
                                control.side_effect_markers(),
                                catalog,
                            )?),
                            None => None,
                        };
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
                    publish_listener_once(
                        state.path(),
                        &identity,
                        Arc::clone(&clock),
                        Arc::clone(&telemetry),
                    )?;
                    listener_published = true;
                }
            }
            ReceiptScenarioAction::SpawnCancel {
                key,
                _lazy_session: _,
                label,
            } => {
                if !matches!(key, ScenarioKey::Exact) || operations.contains_key(&label) {
                    return Err(unsupported_shape(
                        "spawn_cancel needs an exact key and a fresh label",
                    ));
                }
                if pending_submit.is_some() || live_daemon.is_some() {
                    let worker_label = label.clone();
                    let worker_control = Arc::clone(&control);
                    let operation =
                        spawn_scenario_operation(label.clone(), Arc::clone(&control), move || {
                            worker_control
                                .acquire_lifecycle_gate(
                                    &worker_label,
                                    Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                                )
                                .map_err(|error| {
                                    format!("acquire live cancel lifecycle gate: {error}")
                                })?;
                            worker_control.request_gate_cancel();
                            worker_control.release_lifecycle_gate(&worker_label);
                            Ok(())
                        });
                    operations.insert(label, operation);
                    continue;
                }
                let runtime = match &operation_runtime {
                    Some(runtime) => Arc::clone(runtime),
                    None => {
                        let (runtime, projection, attempts) = open_scenario_operation_runtime(
                            state.path(),
                            &identity,
                            &clock,
                            &control,
                            &telemetry,
                        )?;
                        live_task_projection = Some((projection, attempts));
                        live_actor = Some(runtime.receipt_ledger.clone());
                        operation_runtime = Some(Arc::clone(&runtime));
                        runtime
                    }
                };
                let key = exact_key.clone();
                let worker_label = label.clone();
                let operation =
                    spawn_scenario_operation(label.clone(), Arc::clone(&control), move || {
                        runtime.cancel_under_gate_for_test(
                            &key,
                            &worker_label,
                            Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                        )
                    });
                operations.insert(label, operation);
            }
            ReceiptScenarioAction::SpawnMarkReservedBegun { proof, label } => {
                if !matches!(proof, ScenarioActorProof::Exact) || operations.contains_key(&label) {
                    return Err(unsupported_shape(
                        "spawn_mark_reserved_begun needs an exact actor proof and a fresh label",
                    ));
                }
                let runtime = match &operation_runtime {
                    Some(runtime) => Arc::clone(runtime),
                    None => {
                        let (runtime, projection, attempts) = open_scenario_operation_runtime(
                            state.path(),
                            &identity,
                            &clock,
                            &control,
                            &telemetry,
                        )?;
                        live_task_projection = Some((projection, attempts));
                        live_actor = Some(runtime.receipt_ledger.clone());
                        operation_runtime = Some(Arc::clone(&runtime));
                        runtime
                    }
                };
                let key = exact_key.clone();
                let worker_label = label.clone();
                let operation =
                    spawn_scenario_operation(label.clone(), Arc::clone(&control), move || {
                        runtime.mark_reserved_begun_under_gate_for_test(
                            &key,
                            &worker_label,
                            Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                        )
                    });
                operations.insert(label, operation);
            }
            ReceiptScenarioAction::SpawnTaskStoreCreateAndBindUnderGate { label } => {
                if operations.contains_key(&label) {
                    return Err(unsupported_shape(
                        "spawn_task_store_create_and_bind_under_gate label is already in use",
                    ));
                }
                let runtime = match &operation_runtime {
                    Some(runtime) => Arc::clone(runtime),
                    None => {
                        let (runtime, projection, attempts) = open_scenario_operation_runtime(
                            state.path(),
                            &identity,
                            &clock,
                            &control,
                            &telemetry,
                        )?;
                        live_task_projection = Some((projection, attempts));
                        live_actor = Some(runtime.receipt_ledger.clone());
                        operation_runtime = Some(Arc::clone(&runtime));
                        runtime
                    }
                };
                let key = exact_key.clone();
                let worker_label = label.clone();
                let operation =
                    spawn_scenario_operation(label.clone(), Arc::clone(&control), move || {
                        runtime.bind_task_under_gate_for_test(
                            &key,
                            &worker_label,
                            Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                        )
                    });
                operations.insert(label, operation);
            }
            ReceiptScenarioAction::SpawnStageBoundHandoffTerminal { terminal, label } => {
                control.arm_skip_next_startup_reconciliation();
                if operations.contains_key(&label) {
                    return Err(unsupported_shape(
                        "spawn_stage_bound_handoff_terminal label is already in use",
                    ));
                }
                let daemon_state = DaemonStateDirectory::open(state.path(), &identity)?;
                let config = scenario_server_config_with_clock(
                    state.path(),
                    &identity,
                    Some(&control),
                    &clock,
                );
                let runtime =
                    V5ReceiptRuntime::open_with_epoch_clock(&daemon_state, &config, clock.clone())?
                        .with_hooks_for_test(ScenarioHooks::install(
                            Arc::clone(&telemetry),
                            Some(Arc::clone(&control)),
                        ));
                let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Completed {
                    result: Box::new(domain_result_for_fixture(&terminal)?),
                })
                .map_err(|error| format!("encode staged scenario terminal: {error}"))?;
                control.record_operation_event(&label, "spawned");
                runtime.stage_bound_handoff_terminal_for_test(
                    &exact_key,
                    terminal,
                    Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                )?;
                control.record_operation_event(&label, "completed");
                operations.insert(
                    label,
                    ScenarioOperation {
                        completed: Arc::new(AtomicBool::new(true)),
                        handle: thread::spawn(|| Ok(())),
                    },
                );
            }
            ReceiptScenarioAction::WaitForOperation { label, state } => {
                let Some(operation) = operations.get(&label) else {
                    return Err(format!("unknown scenario operation {label}"));
                };
                let expected = match state {
                    ScenarioOperationState::Blocked => "blocked",
                    ScenarioOperationState::Completed => "completed",
                };
                control.wait_for_operation_event(
                    &label,
                    expected,
                    Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                )?;
                if matches!(state, ScenarioOperationState::Completed)
                    && !operation.completed.load(Ordering::Acquire)
                {
                    return Err(format!("operation {label} reported completion before exit"));
                }
            }
            ReceiptScenarioAction::Cancel {
                key,
                label,
                _lazy_session: lazy_session,
            } => {
                let is_exact = matches!(&key, ScenarioKey::Exact);
                let wire_key = match key {
                    ScenarioKey::Exact => exact_key.clone(),
                    ScenarioKey::Unknown => {
                        fresh_key_for_workspace(&identity, &arguments, &workspace_hint)?
                    }
                    ScenarioKey::Mismatch(ScenarioIdentityField::NormalizedArgumentsHash) => {
                        mismatched_arguments_key.clone()
                    }
                    _ => return Err(unsupported_shape("cancel key shape")),
                };
                let response = if let Some(pending) = &pending_submit {
                    pending.cancel_additional(state.path(), &identity, wire_key.clone())?
                } else if live_daemon.is_some() {
                    cancel_on_live_daemon(state.path(), &identity, wire_key.clone())?
                } else {
                    // A session that owns the attempt cancels against the
                    // process holding it: the seeded fixture is handed to that
                    // one owner, which is why its startup does not reconcile.
                    // A lazy session owns nothing, so it meets what a successor
                    // left behind — production reconciles the orphan first.
                    if !lazy_session {
                        control.arm_skip_next_startup_reconciliation();
                    }
                    exchange_once(
                        state.path(),
                        &identity,
                        Arc::clone(&clock),
                        Arc::clone(&telemetry),
                        Some(Arc::clone(&control)),
                        |owner| owner.cancel_invocation(wire_key.clone()),
                    )?
                };
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
                        response_observation_with_exact_task(
                            &submit_response,
                            Some((accepted_epoch_ms, response_budget_ms)),
                            &exact_key,
                            state.path(),
                            &identity,
                            Some(None),
                        )?,
                    );
                    for duplicate_label in pending_duplicate_labels.drain(..) {
                        let duplicate_response =
                            recover_from_live_daemon(state.path(), &identity, exact_key.clone())?;
                        report.responses.insert(
                            duplicate_label,
                            response_observation_with_exact_task(
                                &duplicate_response,
                                Some((accepted_epoch_ms, response_budget_ms)),
                                &exact_key,
                                state.path(),
                                &identity,
                                Some(None),
                            )?,
                        );
                    }
                    for (spawned_label, spawned) in spawned_submit_clients.drain() {
                        let response = spawned.client.join().map_err(|_| {
                            format!("spawned submit {spawned_label} client panicked")
                        })??;
                        let workspace = control.actor_workspace_identity().and_then(|identity| {
                            serde_json::to_value(identity)
                                .ok()
                                .and_then(|value| value.as_str().map(str::to_owned))
                        });
                        report.responses.insert(
                            spawned_label,
                            response_observation_with_exact_task(
                                &response,
                                Some((spawned.accepted_epoch_ms, spawned.response_budget_ms)),
                                &spawned.key,
                                state.path(),
                                &identity,
                                Some(workspace),
                            )?,
                        );
                    }
                }
            }
            ReceiptScenarioAction::CancelTask {
                api,
                task,
                _lazy_session: _,
                label,
            } => {
                let task_id = match task {
                    ScenarioTaskSelector::ExactProjected => exact_key.reserved_task_id(),
                    ScenarioTaskSelector::ForReadLabel(read_label) => report
                        .task_reads
                        .get(&read_label)
                        .and_then(|task| task.get("taskId"))
                        .and_then(Value::as_str)
                        .ok_or_else(|| {
                            format!("Task cancel selector references missing read {read_label}")
                        })?
                        .parse()
                        .map_err(|error| {
                            format!("parse Task cancel selector from {read_label}: {error}")
                        })?,
                };
                // Cancelling by a projected Task observes the fixture too.
                control.arm_skip_next_startup_reconciliation();
                let response = exchange_once(
                    state.path(),
                    &identity,
                    Arc::clone(&clock),
                    Arc::clone(&telemetry),
                    Some(Arc::clone(&control)),
                    |owner| match api {
                        ScenarioTaskCancelApi::Native | ScenarioTaskCancelApi::Compatibility => {
                            owner.cancel_task(task_id)
                        }
                    },
                )?;
                let observation = match &response {
                    V5ServerResponse::Task { .. } => {
                        let task = task_observation_from_response_with_workspace(
                            response,
                            &exact_key,
                            state.path(),
                            &identity,
                            None,
                        )?;
                        json!({
                            "kind": "task",
                            "error": null,
                            "terminal": task.get("terminal").cloned().unwrap_or(Value::Null),
                            "key": receipt_key_observation(&exact_key),
                            "task": task,
                            "acknowledgement": null,
                            "cutoffEpochMs": null,
                            "originalBudgetMs": null,
                            "latencyMs": 0,
                        })
                    }
                    _ => response_observation(&response, None)?,
                };
                report.responses.insert(label, observation);
            }
            ReceiptScenarioAction::SpawnSubmit {
                request,
                response_budget_ms,
                disconnect,
                label,
            } => {
                if !matches!(disconnect, ScenarioDisconnect::Never)
                    || spawned_submit_clients.contains_key(&label)
                    || spawn_submit_labels.contains(&label)
                {
                    return Err(unsupported_shape(
                        "spawn_submit needs disconnect=never and a fresh label",
                    ));
                }
                let submit_key = match request {
                    ScenarioRequest::Fresh(sequence) => {
                        let _ = sequence;
                        fresh_key_for_workspace(&identity, &arguments, &workspace_hint)?
                    }
                    ScenarioRequest::Canonical | ScenarioRequest::SameIdentity => exact_key.clone(),
                    ScenarioRequest::Mismatch(_) => {
                        return Err(unsupported_shape("spawn_submit with a mismatched request"))
                    }
                };
                if pending_submit.is_none() && known_keys.is_empty() {
                    invocation_id = submit_key.invocation_id();
                    reserved_task_id = submit_key.reserved_task_id();
                    exact_key = submit_key.clone();
                    mismatched_arguments_key = ReceiptKey::new(
                        invocation_id,
                        reserved_task_id,
                        RequestIdentity::new(
                            identity.digest().clone(),
                            V5ToolIdentity::View,
                            normalized_arguments_hash(&Map::from_iter([(
                                "mismatch".to_owned(),
                                Value::Bool(true),
                            )])),
                            request_scope_hash(&workspace_hint).map_err(|error| {
                                format!(
                                    "construct mismatched receipt scenario request scope: {error}"
                                )
                            })?,
                        ),
                    );
                }
                let invocation = V5InvocationRequest::new(
                    submit_key.invocation_id(),
                    submit_key.reserved_task_id(),
                    V5ToolIdentity::View,
                    arguments.clone(),
                    workspace_hint.clone(),
                    response_budget_ms,
                )
                .map_err(|error| format!("construct spawned receipt scenario submit: {error}"))?;
                push_known_key(&mut known_keys, submit_key.clone());
                submit_keys.insert(label.clone(), submit_key.clone());
                spawn_submit_labels.insert(label.clone());
                if pending_submit.is_none() {
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
                    let client = spawn_additional_submit_client(
                        state.path(),
                        &identity,
                        invocation,
                        submit_key,
                        clock.now_epoch_millis(),
                        response_budget_ms,
                    );
                    spawned_submit_clients.insert(label, client);
                }
            }
            ReceiptScenarioAction::Submit {
                request,
                response_budget_ms,
                disconnect,
                label,
            } => {
                if let ScenarioRequest::Mismatch(field) = request {
                    let mismatch_key =
                        scenario_mismatch_key(&exact_key, field, &mismatched_arguments_key)?;
                    let outcome = attempt_mismatched_reserve(
                        state.path(),
                        &identity,
                        mismatch_key,
                        clock.now_epoch_millis(),
                        response_budget_ms,
                    )?;
                    let response = match outcome {
                        Err(error) => V5ServerResponse::Error {
                            code: daemon_error_code(&error),
                        },
                        Ok(_) => {
                            return Err(
                                "mismatched receipt identity unexpectedly mutated the ledger"
                                    .to_owned(),
                            )
                        }
                    };
                    report.responses.insert(
                        label,
                        response_observation(&response, original_submit_cutoff)?,
                    );
                    continue;
                }
                let submit_key = match request {
                    ScenarioRequest::Fresh(sequence) => {
                        let _ = sequence;
                        fresh_key_for_workspace(&identity, &arguments, &workspace_hint)?
                    }
                    ScenarioRequest::Canonical | ScenarioRequest::SameIdentity => exact_key.clone(),
                    ScenarioRequest::Mismatch(_) => unreachable!("mismatch handled above"),
                };
                let invocation = V5InvocationRequest::new(
                    submit_key.invocation_id(),
                    submit_key.reserved_task_id(),
                    V5ToolIdentity::View,
                    arguments.clone(),
                    workspace_hint.clone(),
                    response_budget_ms,
                )
                .map_err(|error| format!("construct receipt scenario submit: {error}"))?;
                push_known_key(&mut known_keys, submit_key.clone());
                submit_keys.insert(label.clone(), submit_key.clone());
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
                                response_observation_with_exact_task(
                                    &response,
                                    Some((pending.accepted_epoch_ms, pending.response_budget_ms)),
                                    &submit_key,
                                    state.path(),
                                    &identity,
                                    Some(None),
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
                        ScenarioDisconnect::Never => {
                            if bulk_receipt_catalog.is_some() {
                                let (response, actor) = exchange_once_retaining_actor(
                                    state.path(),
                                    &identity,
                                    Arc::clone(&clock),
                                    Arc::clone(&telemetry),
                                    Some(Arc::clone(&control)),
                                    |owner| owner.submit_invocation(invocation),
                                )?;
                                live_actor = Some(actor);
                                live_task_projection = bulk_task_projection
                                    .as_ref()
                                    .cloned()
                                    .map(|projection| (projection, 0));
                                Some(response)
                            } else {
                                Some(exchange_once(
                                    state.path(),
                                    &identity,
                                    Arc::clone(&clock),
                                    Arc::clone(&telemetry),
                                    Some(Arc::clone(&control)),
                                    |owner| owner.submit_invocation(invocation),
                                )?)
                            }
                        }
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
                                "key": receipt_key_observation(&submit_key),
                                "task": task_observation_from_response(
                                    V5ServerResponse::Task {
                                        snapshot: snapshot.clone(),
                                    },
                                    &submit_key,
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
            ReceiptScenarioAction::ProbeProtocol { .. } => {
                return Err(unsupported_shape(
                    "probe_protocol mixed with receipt actions",
                ))
            }
            ReceiptScenarioAction::Recover { key, label } => {
                let ScenarioKey::Exact = key else {
                    return Err(unsupported_shape("recover needs the exact key"));
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
                let mut observation = if matches!(
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
                        Some(None),
                    )?;
                    json!({
                        "kind": "task",
                        "error": null,
                        "terminal": task.get("terminal").cloned().unwrap_or(Value::Null),
                        "key": receipt_key_observation(&exact_key),
                        "task": task,
                        "acknowledgement": null,
                        "cutoffEpochMs": null,
                        "originalBudgetMs": null,
                        "latencyMs": 0,
                    })
                } else {
                    response_observation(&response, original_submit_cutoff)?
                };
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
                let acknowledge_key = match key {
                    ScenarioKey::Exact => exact_key.clone(),
                    ScenarioKey::ForSubmitLabel(submit_label) => {
                        submit_keys.get(&submit_label).cloned().ok_or_else(|| {
                            format!("acknowledgement references unknown submit {submit_label}")
                        })?
                    }
                    ScenarioKey::Unknown | ScenarioKey::Mismatch(_) => {
                        return Err(unsupported_shape("acknowledge key shape"))
                    }
                };
                let terminal_digest = match digest {
                    ScenarioDigest::ExactTerminal => match &live_actor {
                        Some(actor) => exact_terminal_digest_from_actor(
                            actor,
                            &acknowledge_key,
                            Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                        )?,
                        None => exact_terminal_digest(
                            state.path(),
                            &identity,
                            Arc::clone(&clock),
                            Arc::clone(&telemetry),
                            &acknowledge_key,
                        )?,
                    },
                    ScenarioDigest::Mismatched => TerminalDigest::from_str(&"a5".repeat(32))
                        .expect("fixed mismatch digest is normalized"),
                    ScenarioDigest::WellFormedCandidate => {
                        TerminalDigest::from_str(&"00".repeat(32))
                            .expect("fixed candidate digest is normalized")
                    }
                    ScenarioDigest::TaskTerminal => {
                        task_terminal_digest_from_store(state.path(), &identity, &acknowledge_key)?
                    }
                };
                match disconnect {
                    ScenarioAckDisconnect::Never => {
                        let response = match &live_actor {
                            // A live daemon has a listener: acknowledge over the wire,
                            // the way production does. Without one the harness itself
                            // holds the sole writer, so no listener can exist and the
                            // retained actor is the only owner there is.
                            Some(_) if live_daemon.is_some() => acknowledge_on_live_daemon(
                                state.path(),
                                &identity,
                                acknowledge_key.clone(),
                                terminal_digest,
                            )?,
                            Some(actor) => acknowledge_on_retained_actor(
                                actor,
                                acknowledge_key.clone(),
                                terminal_digest,
                                clock.now_epoch_millis(),
                                &telemetry,
                            ),
                            None => {
                                // Production acknowledges against a daemon that is
                                // already up; the scenario has to start one for the
                                // request. That extra startup must not reconcile the
                                // fixture the acknowledgement is aimed at — it owns it.
                                control.arm_skip_next_startup_reconciliation();
                                exchange_once(
                                    state.path(),
                                    &identity,
                                    Arc::clone(&clock),
                                    Arc::clone(&telemetry),
                                    Some(Arc::clone(&control)),
                                    |owner| {
                                        owner.acknowledge_invocation_receipt(
                                            acknowledge_key.clone(),
                                            terminal_digest,
                                        )
                                    },
                                )?
                            }
                        };
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
                            acknowledge_key,
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
                        snapshot: super::super::task_store_snapshot(
                            &super::super::project_bound_task_for_read(
                                bound_task.record,
                                mask_working_as_queued,
                            ),
                        ),
                    }
                });
                let controlled_terminal_response =
                    control
                        .terminal_bound_task()
                        .map(|bound_task| V5ServerResponse::Task {
                            snapshot: super::super::task_store_snapshot(&bound_task.record),
                        });
                let available_response = match promised_response {
                    Some(response) => Some(response),
                    None if controlled_terminal_response.is_some() => controlled_terminal_response,
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
                    None => {
                        // A read observes the fixture it was given; it does not
                        // stand in for the successor that would reconcile it.
                        control.arm_skip_next_startup_reconciliation();
                        exchange_once(
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
                        )?
                    }
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
            ReceiptScenarioAction::AttemptBoundTaskStart { proof, label } => {
                let daemon_state = DaemonStateDirectory::open(state.path(), &identity)?;
                let projection = V5TaskProjection::open(
                    &daemon_state,
                    clock.clone(),
                    Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                )?;
                let deadline = crate::domain::code_intelligence::ProviderDeadline::new(
                    Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                );
                let link = projection
                    .lifecycle_links
                    .read_by_task_id(exact_key.reserved_task_id(), deadline)
                    .map_err(|error| format!("read bound-start lifecycle proof: {error}"))?;
                let TaskLifecycleLinkRecord::TaskBound(bound) = link else {
                    return Err(
                        "bound-start proof requires one active TaskBound lifecycle link".to_owned(),
                    );
                };
                let record = projection
                    .task_store
                    .get(exact_key.reserved_task_id(), deadline)
                    .map_err(|error| format!("read bound-start Task proof: {error}"))?;
                control.record_rejected_bound_task_start_authorization(
                    label.clone(),
                    proof,
                    &bound,
                    &record,
                );
                report.responses.insert(
                    label,
                    json!({
                        "kind": "rejected",
                        "error": "unauthorized",
                        "terminal": null,
                        "key": null,
                        "task": null,
                        "acknowledgement": null,
                        "cutoffEpochMs": null,
                        "originalBudgetMs": null,
                        "latencyMs": 0,
                    }),
                );
            }
            ReceiptScenarioAction::InvalidateActorProof {
                proof: ScenarioActorProof::Stale,
                point,
                label,
            } => {
                control.wait_until_reached(point, Instant::now() + SCENARIO_OPERATION_TIMEOUT)?;
                let bound_task = control.bound_task().ok_or_else(|| {
                    "post-Working proof invalidation has no bound Task readback".to_owned()
                })?;
                control.record_stale_post_working_authorization(
                    label,
                    &bound_task.bound,
                    &bound_task.record,
                );
                telemetry.record_forced_process_exit();
                control.record_process_exit(1);
            }
            ReceiptScenarioAction::InvalidateActorProof { .. } => {
                return Err(unsupported_shape("invalidate_actor_proof"))
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
                        telemetry.wait_for_event(
                            V5ReceiptRuntimeEventKind::ReceiptReserved,
                            Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                        )?;
                        let workspace = control.actor_workspace_identity().and_then(|identity| {
                            serde_json::to_value(identity)
                                .ok()
                                .and_then(|value| value.as_str().map(str::to_owned))
                        });
                        // The runtime owns the cutoff and its reply is the
                        // projection the report records. When the observer
                        // holds the Task's store create, that reply waits
                        // behind the barrier: the durable handoff intent is
                        // the projection then.
                        let held_handoff = if control
                            .is_barrier_installed(ScenarioBarrierPoint::BeforeTaskStoreCreate)
                        {
                            pending.await_handoff_intent(
                                &exact_key,
                                Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                            )?
                        } else {
                            None
                        };
                        let response = match &held_handoff {
                            Some(handoff) => V5ServerResponse::Invocation {
                                outcome: V5InvocationResponse::Task {
                                    snapshot: super::super::queued_receipt_task_snapshot(
                                        handoff.task(),
                                        handoff.key_digest().clone(),
                                        handoff.cancel_requested(),
                                    ),
                                },
                            },
                            None => pending
                                .await_response(Instant::now() + SCENARIO_OPERATION_TIMEOUT)?,
                        };
                        report.responses.insert(
                            pending.label.clone(),
                            response_observation_with_exact_task(
                                &response,
                                Some((pending.accepted_epoch_ms, pending.response_budget_ms)),
                                &exact_key,
                                state.path(),
                                &identity,
                                Some(workspace),
                            )?,
                        );
                        pending.response_projected = true;
                        if let Some(handoff) = held_handoff {
                            stage_terminal_as_second_owner(
                                &pending.actor,
                                handoff,
                                clock.now_epoch_millis(),
                                &control,
                                &telemetry,
                            )?;
                        }
                    }
                }
                // A watchdog due on the runtime's clock exits the process on
                // its own: the observer waits for that exit and cleans up.
                let fail_stop_due = control
                    .runtime()
                    .is_some_and(|runtime| runtime.fail_stop_due_for_test());
                if fail_stop_due {
                    telemetry.wait_for_event(
                        V5ReceiptRuntimeEventKind::ListenerClosed,
                        Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                    )?;
                    if pending_submit.is_some() {
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
                        if !matches!(
                            response,
                            V5ServerResponse::Invocation {
                                outcome: V5InvocationResponse::ReceiptPending { .. }
                            }
                        ) {
                            report.responses.entry(label).or_insert(
                                response_observation_with_exact_task(
                                    &response,
                                    Some((accepted, budget)),
                                    &exact_key,
                                    state.path(),
                                    &identity,
                                    Some(None),
                                )?,
                            );
                        }
                    }
                    if let Some(daemon) = live_daemon.take() {
                        live_actor = None;
                        live_task_projection = None;
                        daemon.stop_and_join(
                            "protocol-v5 receipt scenario live daemon panicked after fail-stop",
                        )?;
                    }
                }
            }
            ReceiptScenarioAction::Crash { point } => {
                if matches!(
                    point,
                    ScenarioCrashPoint::AfterTaskStoreTerminalBeforeLifecycleLinkTerminal
                ) {
                    control.wait_until_reached(
                        ScenarioBarrierPoint::AfterTaskStoreTerminalBeforeLifecycleLinkTerminal,
                        Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                    )?;
                    telemetry.record_forced_process_exit();
                    control.record_process_exit(1);
                    control.release_all_barriers();
                    let pending = pending_submit
                        .take()
                        .ok_or_else(|| "Task terminal crash has no live submit".to_owned())?;
                    let PendingSubmit {
                        actor,
                        client,
                        daemon,
                        ..
                    } = pending;
                    if let Some(client) = client {
                        let _ = client.join();
                    }
                    drop(actor);
                    daemon.stop_and_join(
                        "protocol-v5 receipt scenario daemon panicked during Task terminal crash",
                    )?;
                    control.clear_task_projections();
                    continue;
                }
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
                    continue;
                }
                if matches!(point, ScenarioCrashPoint::ReservedBegun) {
                    // A process that dies with a `Reserved(Begun)` receipt leaves it
                    // Begun: nobody inside it terminalizes the attempt. The successor's
                    // startup reconciliation is what turns it into an uncertain outcome.
                    // Declaring the exit before the release is what makes the crash a
                    // crash: the released attempt sees a dead process and abandons.
                    telemetry.record_forced_process_exit();
                    control.record_process_exit(1);
                    control.release(ScenarioBarrierPoint::BeforePrepare);
                    let pending = pending_submit
                        .take()
                        .expect("crashed begun submit was checked immediately before take");
                    // A crashed process delivers no response: whatever the abandoned
                    // attempt replied dies with it, so the scenario projects none.
                    let (_, _, _, _, actor, _, _, daemon) = pending.finish()?;
                    drop(actor);
                    daemon.stop_and_join(
                        "protocol-v5 receipt scenario daemon panicked during begun crash",
                    )?;
                } else if matches!(point, ScenarioCrashPoint::TaskPromisedUnbound) {
                    let pending = pending_submit.as_ref().ok_or_else(|| {
                        "protocol-v5 TaskPromisedUnbound crash has no live submit".to_owned()
                    })?;
                    match pending.actor.recover(
                        exact_key.clone(),
                        Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                    ) {
                        Ok(ReceiptState::TaskPromisedUnbound(_)) => {}
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
                    }
                    // The promise a crashed process left behind is the successor's
                    // to reconcile; declaring the exit first is what makes the crash
                    // stop the attempt instead of letting it run on.
                    telemetry.record_forced_process_exit();
                    control.record_process_exit(1);
                    control.release_pre_actor_barriers();
                    let pending = pending_submit
                        .take()
                        .expect("crashed pending submit was checked immediately before take");
                    // A crashed process delivers no response.
                    let (_, _, _, _, actor, _, _, daemon) = pending.finish()?;
                    drop(actor);
                    daemon.stop_and_join(
                        "protocol-v5 receipt scenario daemon panicked during promised crash",
                    )?;
                } else if pending_submit.is_some() {
                    return Err(unsupported_shape(
                        "fail-stop with a live submit at this step",
                    ));
                }
            }
            ReceiptScenarioAction::Restart => {
                control.clear_fail_stop_reclaimed();
                startup_listener_override = true;
                if pending_submit.is_some() {
                    if !control.process_exited() {
                        return Err(
                            "protocol-v5 receipt scenario cannot restart a live submit".to_owned()
                        );
                    }
                    // The receipt a fail-stopped process left behind is the
                    // successor's to reconcile at startup. Terminalizing it here
                    // would be the harness doing the restart's own work early.
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
                        .or_insert(response_observation_with_exact_task(
                            &response,
                            Some((accepted, budget)),
                            &exact_key,
                            state.path(),
                            &identity,
                            Some(None),
                        )?);
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
                match V5ReceiptRuntime::open_with_epoch_clock(
                    &daemon_state,
                    &config
                        .clone()
                        .with_runtime_hooks_for_test(ScenarioHooks::install(
                            Arc::clone(&telemetry),
                            Some(Arc::clone(&control)),
                        )),
                    clock.clone(),
                ) {
                    Ok(runtime) => {
                        bulk_receipt_snapshot = match &bulk_receipt_catalog {
                            Some(catalog) => Some(snapshot_with_actor_and_bulk_catalog(
                                &runtime.receipt_ledger,
                                &clock,
                                &telemetry,
                                control.side_effect_markers(),
                                catalog,
                            )?),
                            None => None,
                        };
                        if let Some(projection) = &mut bulk_task_projection {
                            merge_exact_runtime_task_projection(
                                projection,
                                &runtime,
                                &exact_key,
                                state.path(),
                                &identity,
                            )?;
                        }
                        drop(runtime);
                        startup_failed = false;
                        // This branch proves startup/reconciliation by opening
                        // and immediately dropping a runtime; it does not spawn
                        // a daemon listener.  The next wire action must publish
                        // a real endpoint instead of trusting this probe.
                        listener_published = false;
                    }
                    Err(_) => {
                        startup_failed = true;
                        listener_published = false;
                    }
                }
            }
            ReceiptScenarioAction::Checkpoint { label } => {
                quiesce_promoted_continuation(state.path(), &identity, &control, &telemetry)?;
                let (mut snapshot, live_task_projection) = if let Some(snapshot) =
                    &corrupted_identity_snapshot
                {
                    (snapshot.clone(), None)
                } else {
                    match &pending_submit {
                        Some(pending) => (
                            match &bulk_receipt_catalog {
                                Some(catalog) => snapshot_with_actor_and_bulk_catalog(
                                    &pending.actor,
                                    &clock,
                                    &telemetry,
                                    control.side_effect_markers(),
                                    catalog,
                                )?,
                                None => snapshot_with_actor(
                                    &pending.actor,
                                    &clock,
                                    &telemetry,
                                    control.side_effect_markers(),
                                    &known_keys,
                                )?,
                            },
                            Some((&pending.task_projection, pending.task_store_create_attempts)),
                        ),
                        None => match &live_actor {
                            Some(actor) => (
                                match &bulk_receipt_catalog {
                                    Some(catalog) => snapshot_with_actor_and_bulk_catalog(
                                        actor,
                                        &clock,
                                        &telemetry,
                                        control.side_effect_markers(),
                                        catalog,
                                    )?,
                                    None => snapshot_with_actor(
                                        actor,
                                        &clock,
                                        &telemetry,
                                        control.side_effect_markers(),
                                        &known_keys,
                                    )?,
                                },
                                live_task_projection
                                    .as_ref()
                                    .map(|(projection, attempts)| (projection, *attempts)),
                            ),
                            None => {
                                let snapshot = match &bulk_receipt_snapshot {
                                    Some(snapshot) => snapshot.clone(),
                                    None => snapshot_from_state(
                                        state.path(),
                                        &identity,
                                        Arc::clone(&clock),
                                        &telemetry,
                                        &control,
                                        &known_keys,
                                    )?,
                                };
                                (
                                    snapshot,
                                    bulk_task_projection.as_ref().map(|projection| {
                                        (
                                            projection,
                                            telemetry.snapshot().task_store_create_attempts,
                                        )
                                    }),
                                )
                            }
                        },
                    }
                };
                if corrupted_identity_snapshot.is_some() {
                    snapshot["listener"] = Value::String("not_published".to_owned());
                    snapshot["restartRequested"] = Value::Bool(startup_failed);
                    snapshot["daemonRunning"] = Value::Bool(!startup_failed);
                }
                match live_task_projection {
                    Some((projection, expected_create_attempts)) => {
                        if known_keys.len() > 1 {
                            let current = controlled_task_projection_observation(
                                &control,
                                state.path(),
                                &identity,
                            )?;
                            if bulk_task_projection.is_some() {
                                apply_task_projection(&mut snapshot, projection)?;
                            } else {
                                apply_task_projection(
                                    &mut snapshot,
                                    current.as_ref().unwrap_or(projection),
                                )?;
                            }
                        } else if let Some(terminal_task) = control.terminal_bound_task() {
                            if let Some(bulk_projection) = &bulk_task_projection {
                                apply_task_projection(&mut snapshot, bulk_projection)?;
                            } else {
                                let projection = terminal_bound_task_projection_observation(
                                    terminal_task,
                                    state.path(),
                                    &identity,
                                )?;
                                apply_task_projection(&mut snapshot, &projection)?;
                            }
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
                                    format!(
                                        "live protocol-v5 checkpoint {label} crossed an unobserved TaskStore mutation: expected {expected_create_attempts}, observed {}",
                                        telemetry.snapshot().task_store_create_attempts
                                    ),
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
                if telemetry.snapshot().restart_requested {
                    let mut linked_task_ids = snapshot
                        .get("taskLinks")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(|link| link.get("taskId").cloned())
                        .collect::<Vec<_>>();
                    if snapshot
                        .get("taskLinkReservedCount")
                        .and_then(Value::as_u64)
                        .is_some_and(|count| count > 0)
                    {
                        linked_task_ids.extend(
                            snapshot
                                .get("receipts")
                                .and_then(Value::as_array)
                                .into_iter()
                                .flatten()
                                .filter(|receipt| {
                                    matches!(
                                        receipt.get("state").and_then(Value::as_str),
                                        Some(
                                            "task_handoff_actor_bound_not_begun"
                                                | "task_handoff_actor_bound_begun"
                                        )
                                    )
                                })
                                .filter_map(|receipt| {
                                    receipt
                                        .get("key")
                                        .and_then(|key| key.get("reservedTaskId"))
                                        .cloned()
                                }),
                        );
                    }
                    if let Some(tasks) = snapshot.get_mut("tasks").and_then(Value::as_array_mut) {
                        tasks.retain(|task| {
                            task.get("taskId")
                                .is_some_and(|task_id| linked_task_ids.contains(task_id))
                        });
                    }
                }
                if startup_listener_override {
                    let runtime_listener = telemetry.snapshot().listener;
                    snapshot["listener"] = Value::String(
                        if startup_failed {
                            if runtime_listener == V5ReceiptRuntimeListenerState::Closed {
                                "closed"
                            } else {
                                "not_published"
                            }
                        } else if listener_published {
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
                let telemetry_snapshot = telemetry.snapshot();
                let staged_sequence = telemetry_snapshot
                    .events
                    .iter()
                    .rev()
                    .find(|event| {
                        event.event == V5ReceiptRuntimeEventKind::BoundHandoffTerminalStaged
                    })
                    .map(|event| event.sequence);
                let created_sequence = telemetry_snapshot
                    .events
                    .iter()
                    .rev()
                    .find(|event| event.event == V5ReceiptRuntimeEventKind::TaskStoreCreated)
                    .map(|event| event.sequence);
                let link_capacity_rejected_sequence = telemetry_snapshot
                    .events
                    .iter()
                    .rev()
                    .find(|event| {
                        event.event == V5ReceiptRuntimeEventKind::TaskLinkCapacityRejected
                    })
                    .map(|event| event.sequence);
                let link_reserved_sequence = telemetry_snapshot
                    .events
                    .iter()
                    .rev()
                    .find(|event| {
                        event.event == V5ReceiptRuntimeEventKind::TaskLinkCapacityReserved
                    })
                    .map(|event| event.sequence);
                if staged_sequence.is_some()
                    && link_reserved_sequence.is_some()
                    && created_sequence.is_none_or(|created| staged_sequence > Some(created))
                    && link_capacity_rejected_sequence
                        .is_none_or(|rejected| staged_sequence > Some(rejected))
                {
                    snapshot["taskLinkReservedCount"] = Value::from(1_u64);
                    snapshot["taskLinkReservedBytes"] = Value::from(1_024_u64);
                }
                merge_unique_values(
                    &mut report.terminal_publications,
                    telemetry_snapshot.terminal_publications,
                );
                report.checkpoints.insert(label, snapshot);
            }
            ReceiptScenarioAction::Reset => {
                if pending_submit.is_some() || !operations.is_empty() {
                    return Err(
                        "protocol-v5 receipt scenario cannot reset a live operation".to_owned()
                    );
                }
                operation_runtime = None;
                live_actor = None;
                if let Some(daemon) = live_daemon.take() {
                    live_actor = None;
                    live_task_projection = None;
                    daemon.stop_and_join(
                        "protocol-v5 receipt scenario live daemon panicked before reset",
                    )?;
                }
                for (receipt, record_bytes) in control.receipt_backed_terminals() {
                    telemetry.record_receipt_backed_publication(&receipt, &record_bytes);
                }
                merge_unique_values(
                    &mut report.terminal_publications,
                    telemetry.snapshot().terminal_publications,
                );
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
                bulk_task_projection = None;
                bulk_receipt_snapshot = None;
                bulk_receipt_catalog = None;
                inject_task_store_capacity_invariant_once = false;
                exact_key = fresh_key_for_workspace(&identity, &arguments, &workspace_hint)?;
                invocation_id = exact_key.invocation_id();
                reserved_task_id = exact_key.reserved_task_id();
                let mut mismatched = Map::new();
                mismatched.insert("mismatch".to_owned(), Value::Bool(true));
                mismatched_arguments_key = ReceiptKey::new(
                    invocation_id,
                    reserved_task_id,
                    RequestIdentity::new(
                        identity.digest().clone(),
                        V5ToolIdentity::View,
                        normalized_arguments_hash(&mismatched),
                        request_scope_hash(&workspace_hint).map_err(|error| {
                            format!("construct reset mismatched request scope: {error}")
                        })?,
                    ),
                );
            }
            ReceiptScenarioAction::FillReceiptPool { state: fill, count } => {
                if !matches!(
                    fill,
                    ScenarioSeedReceiptState::CancelReserved
                        | ScenarioSeedReceiptState::ReservedUnbound
                        | ScenarioSeedReceiptState::TaskTerminalReceiptBacked
                ) {
                    return Err(unsupported_shape("fill_receipt_pool state"));
                }
                let daemon_state = DaemonStateDirectory::open(state.path(), &identity)?;
                let config =
                    scenario_server_config_with_clock(state.path(), &identity, None, &clock);
                let runtime =
                    V5ReceiptRuntime::open_with_epoch_clock(&daemon_state, &config, clock.clone())?
                        .with_hooks_for_test(ScenarioHooks::install(Arc::clone(&telemetry), None));
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
                    match fill {
                        ScenarioSeedReceiptState::CancelReserved => {
                            runtime
                                .seed_cancel_reserved_pool_entry_for_test(
                                    key.clone(),
                                    epoch_ms,
                                    Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                                )
                                .map_err(|error| format!("fill CancelReserved pool: {error}"))?;
                        }
                        ScenarioSeedReceiptState::ReservedUnbound => {
                            runtime
                                .seed_reserved_pool_entry_for_test(
                                    key.clone(),
                                    OriginalCutoffDescriptor::new(epoch_ms, 7_000).map_err(
                                        |error| format!("construct filled receipt cutoff: {error}"),
                                    )?,
                                    epoch_ms,
                                    Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                                )
                                .map_err(|error| format!("fill ReservedUnbound pool: {error}"))?;
                        }
                        ScenarioSeedReceiptState::TaskTerminalReceiptBacked => {
                            let terminal =
                                canonical_v5_terminal(&ReceiptTerminalOutcome::Completed {
                                    result: Box::new(DomainResult::success("receipt-pool")),
                                })
                                .map_err(|error| format!("encode filled Task terminal: {error}"))?;
                            runtime
                                .seed_receipt_backed_terminal_pool_entry_for_test(
                                    key.clone(),
                                    epoch_ms,
                                    SCENARIO_TASK_TTL_MS,
                                    SCENARIO_TASK_POLL_INTERVAL_MS,
                                    terminal,
                                    Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                                )
                                .map_err(|error| {
                                    format!("publish filled receipt-backed Task: {error}")
                                })?;
                        }
                        _ => unreachable!("receipt pool support guard narrowed the seed state"),
                    }
                    push_known_key(&mut known_keys, key);
                }
            }
            ReceiptScenarioAction::FillTaskLinks => {
                let (keys, projection) = fill_linked_task_pool(
                    state.path(),
                    &identity,
                    Arc::clone(&clock),
                    &arguments,
                    &workspace_hint,
                    4_096,
                )?;
                for key in keys {
                    push_known_key(&mut known_keys, key);
                }
                bulk_task_projection = Some(projection);
            }
            ReceiptScenarioAction::FillTaskLinksLeavingOneReservationSlot => {
                let (keys, projection) = fill_linked_task_pool(
                    state.path(),
                    &identity,
                    Arc::clone(&clock),
                    &arguments,
                    &workspace_hint,
                    4_095,
                )?;
                for key in keys {
                    push_known_key(&mut known_keys, key);
                }
                bulk_task_projection = Some(projection);
            }
            ReceiptScenarioAction::FillTombstones => {
                let (actor, catalog) = fill_tombstone_pool(
                    state.path(),
                    &identity,
                    &clock,
                    &arguments,
                    &workspace_hint,
                )?;
                bulk_receipt_snapshot = Some(snapshot_with_actor_and_bulk_catalog(
                    &actor,
                    &clock,
                    &telemetry,
                    control.side_effect_markers(),
                    &catalog,
                )?);
                bulk_receipt_catalog = Some(catalog);
            }
            ReceiptScenarioAction::AttemptUnstagedTaskBindAgainstStagedTerminal { label } => {
                control.arm_skip_next_startup_reconciliation();
                let daemon_state = DaemonStateDirectory::open(state.path(), &identity)?;
                let config = scenario_server_config_with_clock(
                    state.path(),
                    &identity,
                    Some(&control),
                    &clock,
                );
                let runtime =
                    V5ReceiptRuntime::open_with_epoch_clock(&daemon_state, &config, clock.clone())?
                        .with_hooks_for_test(ScenarioHooks::install(
                            Arc::clone(&telemetry),
                            Some(Arc::clone(&control)),
                        ));
                control.record_operation_event(&label, "spawned");
                let refused = runtime.attempt_unstaged_task_bind_against_staged_terminal_for_test(
                    &exact_key,
                    Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                )?;
                control
                    .record_operation_event(&label, if refused { "refused" } else { "completed" });
            }
            ReceiptScenarioAction::AttemptTaskStoreBindUnderGate { label } => {
                control.arm_skip_next_startup_reconciliation();
                let daemon_state = DaemonStateDirectory::open(state.path(), &identity)?;
                let config = scenario_server_config_with_clock(
                    state.path(),
                    &identity,
                    Some(&control),
                    &clock,
                );
                let runtime =
                    V5ReceiptRuntime::open_with_epoch_clock(&daemon_state, &config, clock.clone())?
                        .with_hooks_for_test(ScenarioHooks::install(
                            Arc::clone(&telemetry),
                            Some(Arc::clone(&control)),
                        ));
                if inject_task_store_capacity_invariant_once {
                    inject_task_store_capacity_invariant_once = false;
                    let violation = runtime.inject_task_store_capacity_invariant_for_test(
                        &exact_key,
                        &label,
                        Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                    )?;
                    control.record_operation_event(&label, "spawned");
                    control.record_operation_event(&label, "completed");
                    operations.insert(
                        label,
                        ScenarioOperation {
                            completed: Arc::new(AtomicBool::new(true)),
                            handle: thread::spawn(|| Ok(())),
                        },
                    );
                    report
                        .task_store_capacity_invariant_violations
                        .push(violation);
                    startup_failed = true;
                    listener_published = false;
                    live_task_projection = None;
                    continue;
                }
                let (response, capacity) = runtime.attempt_task_store_bind_under_gate_for_test(
                    &exact_key,
                    &label,
                    Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                )?;
                control.record_operation_event(&label, "spawned");
                control.record_operation_event(&label, "completed");
                operations.insert(
                    label.clone(),
                    ScenarioOperation {
                        completed: Arc::new(AtomicBool::new(true)),
                        handle: thread::spawn(|| Ok(())),
                    },
                );
                report.responses.insert(label, response);
                report.task_publication_capacity.push(capacity);
            }
            ReceiptScenarioAction::InjectTaskStoreCapacityInvariantViolationOnce => {
                inject_task_store_capacity_invariant_once = true;
            }
            ReceiptScenarioAction::ContinueReceiptOwnedAttempt { terminal, label } => {
                control.arm_skip_next_startup_reconciliation();
                let daemon_state = DaemonStateDirectory::open(state.path(), &identity)?;
                let config = scenario_server_config_with_clock(
                    state.path(),
                    &identity,
                    Some(&control),
                    &clock,
                );
                let runtime =
                    V5ReceiptRuntime::open_with_epoch_clock(&daemon_state, &config, clock.clone())?
                        .with_hooks_for_test(ScenarioHooks::install(
                            Arc::clone(&telemetry),
                            Some(Arc::clone(&control)),
                        ));
                let response = runtime.continue_receipt_owned_attempt_for_test(
                    &exact_key,
                    domain_result_for_fixture(&terminal)?,
                    Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                )?;
                report.responses.insert(label, response);
            }
            ReceiptScenarioAction::RunCrossStoreCrashWorkload { cases } => {
                let (crash_cases, preparations, publications) = run_cross_store_crash_cases(
                    &identity,
                    &clock,
                    &arguments,
                    &workspace_hint,
                    cases,
                )?;
                report.crash_cases.extend(crash_cases);
                merge_unique_values(&mut report.staged_terminal_preparations, preparations);
                merge_unique_values(&mut report.terminal_publications, publications);
            }
            ReceiptScenarioAction::RunTaskRetirementWorkload { cases } => {
                report
                    .task_retirement_cases
                    .extend(run_task_retirement_cases(
                        &identity,
                        &clock,
                        &arguments,
                        &workspace_hint,
                        cases,
                    )?);
            }
            ReceiptScenarioAction::RunDirectLoad {
                calls,
                duration_ms,
                concurrency,
                retained_receipt_terminals,
                immediate_ack,
                label,
            } => {
                let (load, retained_keys) = run_direct_load(
                    state.path(),
                    &identity,
                    Arc::clone(&clock),
                    Arc::clone(&control),
                    Arc::clone(&telemetry),
                    &arguments,
                    &workspace_hint,
                    calls,
                    duration_ms,
                    concurrency,
                    retained_receipt_terminals,
                    immediate_ack,
                )?;
                retained_keys
                    .into_iter()
                    .for_each(|key| push_known_key(&mut known_keys, key));
                report.load_runs.insert(label, load);
                listener_published = true;
            }
            ReceiptScenarioAction::RunLazyCancelStorm {
                submits,
                cancels,
                per_cancel_deadline_ms,
                label,
            } => {
                let (load, keys) = run_lazy_cancel_storm(
                    state.path(),
                    &identity,
                    Arc::clone(&clock),
                    Arc::clone(&control),
                    Arc::clone(&telemetry),
                    &arguments,
                    &workspace_hint,
                    submits,
                    cancels,
                    per_cancel_deadline_ms,
                )?;
                keys.into_iter()
                    .for_each(|key| push_known_key(&mut known_keys, key));
                report.load_runs.insert(label, load);
                listener_published = true;
            }
            ReceiptScenarioAction::ReclaimExpiredEvidence => {
                let observed_at_epoch_ms = clock.now_epoch_millis();
                let reclaim = |actor: &ReceiptLedgerActor| {
                    actor
                        .reclaim_expired_tombstones(
                            observed_at_epoch_ms,
                            Instant::now() + SCENARIO_BULK_OPERATION_TIMEOUT,
                        )
                        .map_err(|error| format!("reclaim explicit receipt evidence: {error}"))
                };
                let reclaimed = match &live_actor {
                    Some(actor) => reclaim(actor)?,
                    None => {
                        let daemon_state = DaemonStateDirectory::open(state.path(), &identity)?;
                        let receipts =
                            daemon_state.create_private_retained_subdirectory("receipts")?;
                        let actor = open_receipt_actor_for_scenario(
                            receipts,
                            "open explicit receipt retention coordinator",
                        )?;
                        reclaim(&actor)?
                    }
                };
                if let Some(catalog) = &mut bulk_receipt_catalog {
                    let removed = catalog.retain_unexpired(observed_at_epoch_ms)?;
                    if removed > reclaimed {
                        return Err(
                            "retention reported fewer removals than the expired bulk evidence"
                                .to_owned(),
                        );
                    }
                }
            }
            ReceiptScenarioAction::RotateReceiptSegments => {
                rotate_receipt_generation(
                    state.path(),
                    &identity,
                    live_actor.as_ref(),
                    Instant::now() + SCENARIO_BULK_OPERATION_TIMEOUT,
                )?;
            }
            ReceiptScenarioAction::JoinOperation { label } => {
                if spawn_submit_labels.remove(&label) {
                    if !report.responses.contains_key(&label) {
                        return Err(format!(
                            "spawned submit {label} was joined before its response was projected"
                        ));
                    }
                    control.record_operation_event(&label, "joined");
                    continue;
                }
                let operation = operations
                    .remove(&label)
                    .ok_or_else(|| format!("unknown scenario operation {label}"))?;
                let result = operation
                    .handle
                    .join()
                    .map_err(|_| format!("scenario operation {label} panicked"))?;
                result?;
                control.record_operation_event(&label, "joined");
            }
            ReceiptScenarioAction::InstallBarrier { point } => {
                control.install(point);
            }
            ReceiptScenarioAction::WaitForEventCount { event, count } => {
                let count = usize::try_from(count)
                    .map_err(|_| "scenario event count exceeds usize".to_owned())?;
                telemetry.wait_for_event_count(
                    scenario_runtime_event_kind(event),
                    count,
                    Instant::now() + SCENARIO_BULK_OPERATION_TIMEOUT,
                )?;
            }
            ReceiptScenarioAction::WaitForEvent { event } => {
                quiesce_promoted_continuation(state.path(), &identity, &control, &telemetry)?;
                match event {
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
                    ScenarioEvent::BoundHandoffTerminalStaged => {
                        telemetry.wait_for_event(
                            V5ReceiptRuntimeEventKind::BoundHandoffTerminalStaged,
                            Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                        )?;
                        telemetry.wait_for_either_event(
                            V5ReceiptRuntimeEventKind::TaskLinkCapacityReserved,
                            V5ReceiptRuntimeEventKind::TaskLinkCapacityRejected,
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
                    ScenarioEvent::FalseCancelObservationReached => {
                        telemetry.wait_for_event(
                            V5ReceiptRuntimeEventKind::FalseCancelObservationReached,
                            Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                        )?;
                    }
                    ScenarioEvent::TaskStoreTerminalCommitted => {
                        telemetry.wait_for_event(
                            V5ReceiptRuntimeEventKind::TaskStoreTerminalCommitted,
                            Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                        )?;
                    }
                    ScenarioEvent::TaskStoreTerminalReadback => {
                        telemetry.wait_for_event(
                            V5ReceiptRuntimeEventKind::TaskStoreTerminalReadback,
                            Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                        )?;
                    }
                    ScenarioEvent::TaskTerminalBoundCommitted => {
                        let timeout = if control.has_precomputed_terminal() {
                            SCENARIO_BULK_SNAPSHOT_TIMEOUT
                        } else {
                            SCENARIO_OPERATION_TIMEOUT
                        };
                        telemetry.wait_for_event(
                            V5ReceiptRuntimeEventKind::TaskTerminalBoundCommitted,
                            Instant::now() + timeout,
                        )?;
                    }
                    ScenarioEvent::TokenSignalled => {
                        telemetry.wait_for_event(
                            V5ReceiptRuntimeEventKind::TokenSignalled,
                            Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                        )?;
                    }
                    ScenarioEvent::MarkReservedBegunBlocked => {
                        telemetry.wait_for_event(
                            V5ReceiptRuntimeEventKind::MarkReservedBegunBlocked,
                            Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                        )?;
                    }
                    ScenarioEvent::CancelCommitBlocked => {
                        telemetry.wait_for_event(
                            V5ReceiptRuntimeEventKind::CancelCommitBlocked,
                            Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                        )?;
                    }
                    ScenarioEvent::TaskStoreReadbackBeforeBind => {
                        telemetry.wait_for_event(
                            V5ReceiptRuntimeEventKind::TaskStoreReadbackBeforeBind,
                            Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                        )?;
                    }
                    ScenarioEvent::CancelCommitted => {
                        telemetry.wait_for_event(
                            V5ReceiptRuntimeEventKind::CancelCommitted,
                            Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                        )?;
                    }
                    ScenarioEvent::OperationCompleted => {
                        telemetry.wait_for_event(
                            V5ReceiptRuntimeEventKind::OperationCompleted,
                            Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                        )?;
                    }
                    ScenarioEvent::LeaseReleased => {
                        telemetry.wait_for_event(
                            V5ReceiptRuntimeEventKind::LeaseReleased,
                            Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                        )?;
                    }
                    ScenarioEvent::ListenerClosed => {
                        telemetry.wait_for_event(
                            V5ReceiptRuntimeEventKind::ListenerClosed,
                            Instant::now() + SCENARIO_OPERATION_TIMEOUT,
                        )?;
                    }
                }
            }
            ReceiptScenarioAction::ReleaseBarrier { point } => {
                control.release(point);
                if !control.has_unreleased_barriers() {
                    let Some(pending) = pending_submit.take() else {
                        if !operations.is_empty() {
                            continue;
                        }
                        if control.process_exited() {
                            continue;
                        }
                        return Err(
                            "protocol-v5 receipt scenario has no submit at the barrier".to_owned()
                        );
                    };
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
                    if let std::collections::btree_map::Entry::Vacant(entry) =
                        report.responses.entry(label)
                    {
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
                        entry.insert(observation);
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
                    for (spawned_label, spawned) in spawned_submit_clients.drain() {
                        let response = spawned.client.join().map_err(|_| {
                            format!("spawned submit {spawned_label} client panicked")
                        })??;
                        let workspace = control.actor_workspace_identity().and_then(|identity| {
                            serde_json::to_value(identity)
                                .ok()
                                .and_then(|value| value.as_str().map(str::to_owned))
                        });
                        report.responses.insert(
                            spawned_label,
                            response_observation_with_exact_task(
                                &response,
                                Some((spawned.accepted_epoch_ms, spawned.response_budget_ms)),
                                &spawned.key,
                                state.path(),
                                &identity,
                                Some(workspace),
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

    if pending_submit.is_some()
        || !operations.is_empty()
        || !spawned_submit_clients.is_empty()
        || !spawn_submit_labels.is_empty()
    {
        return Err("protocol-v5 receipt scenario ended with a blocked operation".to_owned());
    }
    for (receipt, record_bytes) in control.receipt_backed_terminals() {
        telemetry.record_receipt_backed_publication(&receipt, &record_bytes);
    }
    merge_unique_values(
        &mut report.terminal_publications,
        telemetry.snapshot().terminal_publications,
    );
    merge_unique_values(
        &mut report.terminal_publications,
        control.staged_terminal_publications(),
    );
    report.actor_bindings = control.actor_bindings();
    report.actor_authorizations = control.actor_authorizations();
    merge_unique_values(
        &mut report.staged_terminal_preparations,
        control.staged_terminal_preparations(),
    );
    report.gate_events = control.gate_events();
    report.operation_events = control.operation_events();
    quiesce_promoted_continuation(state.path(), &identity, &control, &telemetry)?;
    let encoded = report.encode(telemetry.snapshot().events);
    drop(live_actor.take());
    let cleanup = match live_daemon.take() {
        Some(daemon) => daemon.stop_and_join(
            "protocol-v5 receipt scenario live daemon panicked during final cleanup",
        ),
        None => Ok(()),
    };
    finish_with_daemon_cleanup(encoded, cleanup)
}
