//! Юнит-тесты обвязки: разбор провода сценария и вспомогательные проверки.
//! Путь модуля прежний, отбор nextest не меняется.

use super::*;
use crate::application::receipt_ledger::{
    OriginalCutoffDescriptor, ACKNOWLEDGED_TOMBSTONE_TTL_MS, MAX_RECEIPT_ENTITLEMENT_BYTES,
};
use crate::infrastructure::daemon::protocol_v5::V5InvocationPhase as TestV5InvocationPhase;
use crate::infrastructure::receipt_ledger::ReceiptLedgerStore;

#[test]
fn bulk_key_index_keeps_exact_json_equality_with_collisions_and_missing_digests() {
    let first = json!({"keyDigest": "collision", "invocationId": "first"});
    let collision = json!({"keyDigest": "collision", "invocationId": "second"});
    let missing = json!({"invocationId": "third"});
    let non_string = json!({"keyDigest": 42, "invocationId": "fourth"});
    let mut index = vec![first.clone(), missing.clone(), non_string.clone()];
    let additions = [
        first.clone(),
        collision.clone(),
        collision.clone(),
        missing.clone(),
        non_string.clone(),
    ];

    extend_receipt_key_index(&mut index, additions.iter());

    assert_eq!(index, vec![first, missing, non_string, collision]);
}

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
        let clock = Arc::new(ScenarioEpochClock::new(SCENARIO_INITIAL_EPOCH_MS, false));
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

        let state =
            DaemonStateDirectory::open(state_root.path(), &identity).expect("open daemon state");
        let config = scenario_server_config_with_clock(state_root.path(), &identity, None, &clock)
            .expect("build scenario runtime configuration");
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
        let clock = Arc::new(ScenarioEpochClock::new(SCENARIO_INITIAL_EPOCH_MS, false));
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
        let state =
            DaemonStateDirectory::open(state_root.path(), &identity).expect("open daemon state");
        let config = scenario_server_config_with_clock(state_root.path(), &identity, None, &clock)
            .expect("build scenario runtime configuration");
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
        let clock = Arc::new(ScenarioEpochClock::new(SCENARIO_INITIAL_EPOCH_MS, false));
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
        let state =
            DaemonStateDirectory::open(state_root.path(), &identity).expect("open daemon state");
        let config = scenario_server_config_with_clock(state_root.path(), &identity, None, &clock)
            .expect("build scenario runtime configuration");
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
        .expect("run TaskBound checkpoint scenario");
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
        .expect("run production receipt-owned Task cancellation");
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
        .expect("run protocol-v5 acknowledgement scenario");
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
    let state_root = std::fs::canonicalize(root.path()).expect("physical failed-batch state root");
    let identity = CoreIdentity::production_v5();

    let error = exchange_batch(
        &state_root,
        &identity,
        Arc::new(ScenarioEpochClock::new(SCENARIO_INITIAL_EPOCH_MS, false)),
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
            OriginalCutoffDescriptor::new(SCENARIO_INITIAL_EPOCH_MS, 7_000).expect("valid cutoff"),
            Instant::now() + SCENARIO_OPERATION_TIMEOUT,
        )
        .expect("reserve ordinary receipt");

    let telemetry = V5ReceiptRuntimeTelemetry::new();
    let snapshot = snapshot_with_actor(
        &actor,
        &ScenarioEpochClock::new(SCENARIO_INITIAL_EPOCH_MS, false),
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
        client: Some(client),
        response: None,
        daemon: ScenarioDaemon {
            stop_requested: Arc::new(AtomicBool::new(false)),
            server: Some(server),
        },
    };

    pending
        .finish()
        .expect("actor/store must be released before daemon join");
}

#[test]
fn invalid_restart_joins_its_held_operation_before_returning_without_a_successor() {
    struct RedSafetyCleanup {
        control: Arc<ReceiptScenarioControl>,
        life: Arc<ScenarioProcessLife>,
    }
    impl Drop for RedSafetyCleanup {
        fn drop(&mut self) {
            self.life.close_permissions();
            if self
                .control
                .operation_events()
                .iter()
                .any(|event| event["label"] == "held-mark" && event["state"] == "spawned")
            {
                let _ = self.control.wait_for_operation_event(
                    "held-mark",
                    "completed",
                    Instant::now() + Duration::from_secs(5),
                );
            }
        }
    }
    let control = Arc::new(ReceiptScenarioControl::new());
    let original_life = control.process_life();
    let cleanup = RedSafetyCleanup {
        control: control.clone(),
        life: original_life.clone(),
    };
    let request = json!({
        "clock": "fake",
        "actions": [
            {"action": "seed_receipt", "state": "reserved_actor_bound",
             "cancel_requested": false, "staged_terminal": null},
            {"action": "install_barrier", "point": "before_mark_reserved_begun_gate_acquire"},
            {"action": "spawn_mark_reserved_begun", "proof": "exact", "label": "held-mark"},
            {"action": "wait_for_operation", "label": "held-mark", "state": "blocked"},
            {"action": "restart"}
        ]
    });
    let refused =
        dispatch::run_receipt_scenario_with_control_for_test(&request.to_string(), control.clone());
    // Capture the outcome before the test's own RED safety cleanup. A worker
    // completing after this cleanup is not evidence that the runner joined it.
    let events_before_cleanup = control.operation_events();
    let joined_before_return = events_before_cleanup
        .iter()
        .any(|event| event["label"] == "held-mark" && event["state"] == "joined");
    let owner_released_before_return = control.runtime().is_none();
    let hooks = ScenarioHooks::install(
        Arc::new(V5ReceiptRuntimeTelemetry::new()),
        Some(control.clone()),
    );
    let permission_after_return = hooks.wait_for_gate_cancel(Instant::now());
    let unchanged_life = Arc::ptr_eq(&original_life, &control.process_life());
    let process_exited = control.process_exited();
    let elapsed = control.process_exit_elapsed_ms();
    drop(cleanup);
    assert!(
        refused.is_err(),
        "Restart must still refuse an unjoined operation"
    );
    assert!(
        joined_before_return,
        "invalid Restart returned before actually joining its held operation"
    );
    assert!(owner_released_before_return);
    assert_eq!(
        permission_after_return,
        Err(ReceiptLedgerError::StoreUnavailable)
    );
    assert!(unchanged_life);
    assert!(!process_exited);
    assert_eq!(elapsed, None);
}

#[test]
fn no_elapsed_fail_stop_cannot_rotate_while_its_actual_runtime_is_retained() {
    let state = ScenarioStateRoot::new().expect("retained fail-stop owner state");
    let identity = CoreIdentity::production_v5();
    let clock = Arc::new(ScenarioEpochClock::new(SCENARIO_INITIAL_EPOCH_MS, false));
    let control = Arc::new(ReceiptScenarioControl::new());
    let telemetry = Arc::new(V5ReceiptRuntimeTelemetry::new());
    let (runtime, _, _) =
        open_scenario_operation_runtime(state.path(), &identity, &clock, &control, &telemetry)
            .expect("open actual retained runtime");
    let original_life = control.process_life();
    runtime.hooks.forced_process_exit(None);
    assert!(!control.process_exited());
    assert_eq!(control.process_exit_elapsed_ms(), None);
    let refused = control.prepare_process_open();
    let same_life = Arc::ptr_eq(&original_life, &control.process_life());
    drop(runtime);
    assert!(control.runtime().is_none());
    assert!(
        refused.is_err(),
        "a fail-stop request must not authorize opening while its actual runtime remains owned"
    );
    assert!(same_life);
    assert!(!control.process_exited());
    assert_eq!(control.process_exit_elapsed_ms(), None);
}

#[test]
fn genuine_no_elapsed_fail_stop_releases_its_owner_before_a_fresh_native_success() {
    let state = ScenarioStateRoot::new().expect("native fail-stop state");
    let workspace = ScenarioWorkspace::new().expect("native fail-stop workspace");
    let identity = CoreIdentity::production_v5();
    let clock = Arc::new(ScenarioEpochClock::new(SCENARIO_INITIAL_EPOCH_MS, false));
    let control = Arc::new(ReceiptScenarioControl::new());
    let telemetry = Arc::new(V5ReceiptRuntimeTelemetry::new());
    control.set_provider(ScenarioProviderFixture {
        execution_class: ScenarioExecutionClass::Direct,
        terminal: ScenarioTerminalFixture::Success {
            payload: "fresh native result".into(),
        },
        precomputed_terminal: None,
        cooperative_cancel: true,
        side_effect_marker: true,
    });
    control.configure_admission(Some(ScenarioWorkspaceAdmissionFailure::RegistryFailed));
    let mut arguments = Map::new();
    arguments.insert("at".into(), Value::String("main:Configuration".into()));
    let old_key = fresh_key_for_workspace(&identity, &arguments, workspace.hint())
        .expect("old native request identity");
    let old_invocation = V5InvocationRequest::new(
        old_key.invocation_id(),
        old_key.reserved_task_id(),
        V5ToolIdentity::View,
        arguments.clone(),
        unica_bootstrap::ResolvedWorkspace::launch_cwd(workspace.hint().to_owned()),
        7_000,
    )
    .expect("old native invocation");
    let failed = exchange_once(
        state.path(),
        &identity,
        clock.clone(),
        telemetry.clone(),
        Some(control.clone()),
        |owner| owner.submit_invocation(old_invocation),
    )
    .expect("real registry failure returns before actual daemon join");
    let V5ServerResponse::Invocation {
        outcome: V5InvocationResponse::Direct { receipt },
    } = &failed
    else {
        panic!("native admission failure did not preserve its direct receipt");
    };
    assert_eq!(receipt.receipt_key(), &old_key);
    assert_eq!(
        receipt.terminal(),
        &ReceiptTerminalOutcome::Failed {
            reason: V5SafeFailureReason::WorkspaceRegistryFailed,
        }
    );
    assert!(control.current_process_requires_stop());
    assert!(!control.process_exited());
    assert_eq!(control.process_exit_elapsed_ms(), None);
    assert!(
        control.runtime().is_none(),
        "exchange_once must have joined and dropped the real owner"
    );
    // Reclaim only the endpoint of the genuinely joined old daemon. Otherwise
    // the next native exchange could connect to its stale record instead of
    // exercising the new process life.
    quiesce_promoted_continuation(state.path(), &identity, &control, &telemetry)
        .expect("reclaim the endpoint only after its actual failed owner was joined");
    let old_life = control.process_life();
    control.configure_admission(None);
    let (fresh_runtime, _, _) =
        open_scenario_operation_runtime(state.path(), &identity, &clock, &control, &telemetry)
            .expect("open successor only after the failed native owner is gone");
    let changed_life = !Arc::ptr_eq(&old_life, &control.process_life());
    let inherited_stop = control.current_process_requires_stop();
    drop(fresh_runtime);
    assert!(control.runtime().is_none());
    assert!(
        changed_life,
        "implicit runtime open reused the genuine failed process's life"
    );
    assert!(
        !inherited_stop,
        "new runtime inherited the old no-elapsed fail-stop request"
    );
    assert_eq!(control.process_exit_elapsed_ms(), None);
    let fresh_key = fresh_key_for_workspace(&identity, &arguments, workspace.hint())
        .expect("fresh native request identity");
    let fresh_invocation = V5InvocationRequest::new(
        fresh_key.invocation_id(),
        fresh_key.reserved_task_id(),
        V5ToolIdentity::View,
        arguments,
        unica_bootstrap::ResolvedWorkspace::launch_cwd(workspace.hint().to_owned()),
        7_000,
    )
    .expect("fresh native invocation");
    let succeeded = exchange_once(
        state.path(),
        &identity,
        clock.clone(),
        telemetry.clone(),
        Some(control.clone()),
        |owner| owner.submit_invocation(fresh_invocation),
    )
    .expect("fresh native submit succeeds after no-elapsed fail-stop");
    let V5ServerResponse::Invocation {
        outcome: V5InvocationResponse::Direct { receipt },
    } = succeeded
    else {
        panic!("fresh native submit did not return its exact direct receipt");
    };
    assert_eq!(receipt.receipt_key(), &fresh_key);
    assert_eq!(
        receipt.terminal(),
        &ReceiptTerminalOutcome::Completed {
            result: Box::new(DomainResult::success("fresh native result")),
        }
    );
    let recovered = exchange_once(
        state.path(),
        &identity,
        clock,
        telemetry.clone(),
        Some(control.clone()),
        |owner| owner.recover_invocation_receipt(old_key.clone()),
    )
    .expect("recover original failed attempt without replay");
    assert_eq!(recovered, failed);
    assert_eq!(telemetry.snapshot().callbacks.execute, 1);
    assert_eq!(control.side_effect_markers(), 1);
}

#[test]
fn gated_runtime_registers_its_actual_owner_until_that_runtime_is_released() {
    let state = ScenarioStateRoot::new().expect("gated owner state root");
    let identity = CoreIdentity::production_v5();
    let clock = Arc::new(ScenarioEpochClock::new(SCENARIO_INITIAL_EPOCH_MS, false));
    let control = Arc::new(ReceiptScenarioControl::new());
    let telemetry = Arc::new(V5ReceiptRuntimeTelemetry::new());
    let (runtime, _, _) =
        open_scenario_operation_runtime(state.path(), &identity, &clock, &control, &telemetry)
            .expect("open actual gated runtime");
    let observed = control
        .runtime()
        .expect("gated runtime must be an observed owner");
    assert!(Arc::ptr_eq(&observed, &runtime));
    drop(observed);
    assert!(
        control.begin_successor_process().is_err(),
        "a retained gated runtime cannot authorize a successor"
    );
    drop(runtime);
    assert!(control.runtime().is_none());
    control
        .begin_successor_process()
        .expect("released gated runtime no longer owns the process");
}

#[test]
fn gated_cancel_after_begun_crash_opens_a_fresh_process_without_explicit_restart() {
    let request = json!({
        "clock": "fake",
        "actions": [
            {"action": "configure_provider", "execution_class": "direct",
             "terminal": {"terminal": "success", "payload": "must-not-execute"},
             "cooperative_cancel": true, "side_effect_marker": true},
            {"action": "install_barrier", "point": "before_prepare"},
            {"action": "submit", "request": "canonical", "response_budget_ms": 6000,
             "disconnect": "never", "label": "crashed-submit"},
            {"action": "wait_for_event", "event": "receipt_begun_committed"},
            {"action": "crash", "point": "reserved_begun"},
            {"action": "spawn_cancel", "key": "exact", "lazy_session": false,
             "label": "successor-cancel"},
            {"action": "join_operation", "label": "successor-cancel"},
            {"action": "checkpoint", "label": "cancelled-intent"}
        ]
    });
    let encoded = run_supported_receipt_scenario_for_test(&request.to_string())
        .expect("new gated runtime must not inherit the crashed process's closed permissions");
    let report: Value = serde_json::from_str(&encoded).expect("decode gated successor report");
    let snapshot = &report["payload"]["checkpoints"]["cancelled-intent"];
    assert_eq!(snapshot["callbacks"]["prepare"], 0);
    assert_eq!(snapshot["callbacks"]["execute"], 0);
    assert_eq!(snapshot["sideEffectMarkers"], 0);
    let receipts = snapshot["receipts"]
        .as_array()
        .expect("retained receipt rows");
    assert_eq!(receipts.len(), 1);
    assert_eq!(receipts[0]["state"], "reserved_begun");
    assert_eq!(receipts[0]["cancelRequested"], true);
    assert_eq!(snapshot["processExitElapsedMs"], 1);
}

#[test]
fn restart_releases_a_joined_gated_operation_runtime_before_opening_its_successor() {
    let request = json!({
        "clock": "fake",
        "actions": [
            {"action": "seed_receipt", "state": "reserved_actor_bound",
             "cancel_requested": false, "staged_terminal": null},
            {"action": "spawn_mark_reserved_begun", "proof": "exact", "label": "mark"},
            {"action": "join_operation", "label": "mark"},
            {"action": "restart"},
            {"action": "checkpoint", "label": "after-restart"},
            {"action": "recover", "key": "exact", "label": "recovered"}
        ]
    });
    let encoded = run_supported_receipt_scenario_for_test(&request.to_string())
        .expect("joined gated operation must not retain the old runtime across Restart");
    let report: Value = serde_json::from_str(&encoded).expect("decode joined runtime report");
    let payload = &report["payload"];
    assert_eq!(
        payload["responses"]["recovered"]["error"],
        "outcome_uncertain"
    );
    let snapshot = &payload["checkpoints"]["after-restart"];
    assert_eq!(snapshot["callbacks"]["prepare"], 0);
    assert_eq!(snapshot["callbacks"]["execute"], 0);
    assert_eq!(snapshot["sideEffectMarkers"], 0);
}

#[test]
fn restart_gives_a_fresh_submit_live_process_and_preserves_its_future_barrier() {
    let request = json!({
        "clock": "fake",
        "actions": [
            {"action": "configure_provider", "execution_class": "direct",
             "terminal": {"terminal": "success", "payload": "fresh-process-result"},
             "cooperative_cancel": true, "side_effect_marker": true},
            {"action": "install_barrier", "point": "before_prepare"},
            {"action": "submit", "request": "canonical", "response_budget_ms": 6000,
             "disconnect": "never", "label": "old-submit"},
            {"action": "wait_for_event", "event": "receipt_begun_committed"},
            {"action": "crash", "point": "reserved_begun"},
            // This barrier belongs to the successor, even if old-owner cleanup
            // releases its own barriers once again during Restart.
            {"action": "install_barrier", "point": "before_prepare"},
            {"action": "restart"},
            {"action": "recover", "key": "exact", "label": "old-recover"},
            {"action": "submit", "request": {"fresh": 1}, "response_budget_ms": 6000,
             "disconnect": "never", "label": "new-submit"},
            {"action": "wait_for_event_count", "event": "receipt_begun_committed", "count": 2},
            {"action": "checkpoint", "label": "new-held"},
            {"action": "release_barrier", "point": "before_prepare"},
            {"action": "checkpoint", "label": "new-completed"}
        ]
    });
    let encoded = run_supported_receipt_scenario_for_test(&request.to_string())
        .expect("real Restart must admit a fresh process-owned submit");
    let report: Value = serde_json::from_str(&encoded).expect("decode process-life report");
    let payload = &report["payload"];
    let held = &payload["checkpoints"]["new-held"];
    assert_eq!(
        held["callbacks"]["prepare"], 0,
        "future barrier was released by old cleanup"
    );
    assert_eq!(held["callbacks"]["execute"], 0);
    assert_eq!(held["sideEffectMarkers"], 0);
    let completed = &payload["checkpoints"]["new-completed"];
    assert_eq!(completed["callbacks"]["prepare"], 1);
    assert_eq!(completed["callbacks"]["execute"], 1);
    assert_eq!(completed["sideEffectMarkers"], 1);
    assert_eq!(
        payload["responses"]["old-recover"]["error"],
        "outcome_uncertain"
    );
    assert_eq!(payload["responses"]["new-submit"]["kind"], "direct");
    assert!(payload["responses"]["new-submit"]["error"].is_null());
    assert_eq!(
        held["processExitElapsedMs"], 1,
        "historical exit was cleared"
    );
    assert_eq!(completed["processExitElapsedMs"], 1);
}

#[test]
fn implicit_successor_submit_after_joined_owner_quiescence_preserves_its_future_barrier() {
    let request = json!({
        "clock": "fake",
        "actions": [
            {"action": "configure_provider", "execution_class": "direct",
             "terminal": {"terminal": "success", "payload": "fresh-process-result"},
             "cooperative_cancel": true, "side_effect_marker": true},
            {"action": "install_barrier", "point": "before_prepare"},
            {"action": "submit", "request": "canonical", "response_budget_ms": 6000,
             "disconnect": "never", "label": "old-submit"},
            {"action": "wait_for_event", "event": "receipt_begun_committed"},
            {"action": "checkpoint", "label": "old-held"},
            {"action": "crash", "point": "reserved_begun"},
            // The real joined old owner is quiesced before the fresh wire open.
            // This existing event checkpoint does not rotate the process life.
            {"action": "wait_for_event", "event": "receipt_begun_committed"},
            // This configuration must choose a pending worker before implicit open.
            {"action": "install_barrier", "point": "before_prepare"},
            {"action": "submit", "request": {"fresh": 1}, "response_budget_ms": 6000,
             "disconnect": "never", "label": "new-submit"},
            {"action": "wait_for_event_count", "event": "receipt_begun_committed", "count": 2},
            {"action": "checkpoint", "label": "new-held"},
            {"action": "release_barrier", "point": "before_prepare"},
            {"action": "checkpoint", "label": "new-completed"}
        ]
    });
    let encoded = run_supported_receipt_scenario_for_test(&request.to_string())
        .expect("implicit open must select a pending worker for the future barrier");
    let report: Value = serde_json::from_str(&encoded).expect("decode process-life report");
    let payload = &report["payload"];
    let held = &payload["checkpoints"]["new-held"];
    assert_eq!(
        held["callbacks"]["prepare"], 0,
        "future barrier was released by old cleanup"
    );
    assert_eq!(held["callbacks"]["execute"], 0);
    assert_eq!(held["sideEffectMarkers"], 0);
    let completed = &payload["checkpoints"]["new-completed"];
    assert_eq!(completed["callbacks"]["prepare"], 1);
    assert_eq!(completed["callbacks"]["execute"], 1);
    assert_eq!(completed["sideEffectMarkers"], 1);
    let old_rows = payload["checkpoints"]["old-held"]["receipts"]
        .as_array()
        .expect("original held receipt rows");
    assert_eq!(old_rows.len(), 1);
    let original = &old_rows[0];
    assert_eq!(original["state"], "reserved_begun");
    let retained = completed["receipts"]
        .as_array()
        .expect("completed receipt rows")
        .iter()
        .find(|row| row["key"] == original["key"])
        .expect("the exact original receipt remains present");
    assert_eq!(
        retained, original,
        "the crashed attempt was replayed or rewritten"
    );
    assert_ne!(payload["responses"]["new-submit"]["key"], original["key"]);
    assert_eq!(
        payload["responses"]["new-submit"]["terminal"]["result"],
        serde_json::to_value(DomainResult::success("fresh-process-result")).unwrap()
    );
    assert_eq!(payload["responses"]["new-submit"]["kind"], "direct");
    assert!(payload["responses"]["new-submit"]["error"].is_null());
    assert_eq!(
        held["processExitElapsedMs"], 1,
        "historical exit was cleared"
    );
    assert_eq!(completed["processExitElapsedMs"], 1);
}

#[test]
fn fresh_task_listener_after_restart_survives_its_prepare_barrier_release() {
    let request = json!({
        "clock": "fake",
        "actions": [
            {"action": "configure_provider", "execution_class": "direct",
             "terminal": {"terminal": "success", "payload": "fresh-task-result"},
             "cooperative_cancel": true, "side_effect_marker": true},
            {"action": "install_barrier", "point": "before_prepare"},
            {"action": "submit", "request": "canonical", "response_budget_ms": 6000,
             "disconnect": "never", "label": "old-submit"},
            {"action": "wait_for_event", "event": "receipt_begun_committed"},
            {"action": "crash", "point": "reserved_begun"},
            {"action": "install_barrier", "point": "before_prepare"},
            {"action": "restart"},
            {"action": "configure_provider", "execution_class": "known_long",
             "terminal": {"terminal": "success", "payload": "fresh-task-result"},
             "cooperative_cancel": true, "side_effect_marker": true},
            {"action": "submit", "request": {"fresh": 1}, "response_budget_ms": 6000,
             "disconnect": "never", "label": "fresh-task"},
            {"action": "wait_for_event_count", "event": "receipt_begun_committed", "count": 2},
            {"action": "checkpoint", "label": "fresh-held"},
            {"action": "release_barrier", "point": "before_prepare"},
            {"action": "wait_for_event_count", "event": "task_store_terminal_readback", "count": 1},
            {"action": "checkpoint", "label": "fresh-completed"}
        ]
    });
    let mut observed_task = None;
    let mut observer = |record: &crate::infrastructure::daemon::protocol_v5::V5EndpointRecord,
                        keys: &HashMap<String, ReceiptKey>| {
        let key = keys
            .get("fresh-task")
            .ok_or_else(|| "fresh Task key was not submitted".to_owned())?;
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut owner = V5DaemonProcessOwner::connect_before(record.clone(), deadline)
            .map_err(|error| format!("fresh anchored listener closed before cleanup: {error}"))?;
        let snapshot = owner
            .get_task_before(key.reserved_task_id(), deadline)
            .map_err(|error| format!("read fresh Task through anchored listener: {error:?}"))?;
        observed_task = Some((key.clone(), snapshot));
        Ok(())
    };
    let encoded = dispatch::run_receipt_scenario_with_completion_observer(
        &request.to_string(),
        Some(&mut observer),
    )
    .expect("historical crash must not close the fresh Task listener");
    let report: Value = serde_json::from_str(&encoded).expect("decode fresh Task report");
    let payload = &report["payload"];
    assert_eq!(
        payload["checkpoints"]["fresh-held"]["callbacks"]["execute"],
        0
    );
    assert_eq!(
        payload["checkpoints"]["fresh-completed"]["callbacks"]["execute"],
        1
    );
    assert_eq!(
        payload["checkpoints"]["fresh-completed"]["sideEffectMarkers"],
        1
    );
    assert_eq!(payload["responses"]["fresh-task"]["kind"], "task");
    let (key, task) = observed_task.expect("public TCP observer ran");
    assert_eq!(task.receipt_key_digest(), &receipt_key_digest(&key));
    assert_eq!(task.task_id(), key.reserved_task_id());
    assert_eq!(
        task.completed_result(),
        Some(&DomainResult::success("fresh-task-result")),
        "actual public Task snapshot: {task:?}"
    );
    assert!(!task.cancel_requested());
    assert_eq!(
        payload["checkpoints"]["fresh-completed"]["processExitElapsedMs"],
        1
    );
}

fn assert_successor_barrier_configuration(
    control: &Arc<ReceiptScenarioControl>,
    old: &Arc<dyn super::super::hooks::V5RuntimeHooks>,
    telemetry: &Arc<V5ReceiptRuntimeTelemetry>,
) {
    let point = ScenarioBarrierPoint::BeforePrepare;
    let old_life = control.process_life();
    control.install(point);
    assert!(
        control.is_installed(),
        "Submit must select the pending worker for future barriers"
    );
    assert!(control.is_barrier_installed(point));
    assert!(control.has_unreleased_barriers());
    assert!(
        !old.holds(point),
        "a future barrier was installed into the stopped owner"
    );
    control.release_all_barriers();
    assert!(
        control.has_unreleased_barriers(),
        "old cleanup released future configuration"
    );
    assert!(
        !control.any_barrier_awaiting_release(),
        "future configuration is not a paused old worker"
    );
    control
        .prepare_process_open()
        .expect("no retained Runtime owner");
    assert!(!Arc::ptr_eq(&old_life, &control.process_life()));
    let fresh = ScenarioHooks::install(telemetry.clone(), Some(control.clone()));
    assert!(fresh.holds(point));
    assert!(!old.holds(point));
    assert_eq!(
        fresh.pause(point, Instant::now()),
        Err(ReceiptLedgerError::DeadlineExceeded)
    );
    control.release(point);
    assert_eq!(fresh.pause(point, Instant::now()), Ok(()));
    assert!(!control.has_unreleased_barriers());
}

#[test]
fn future_barriers_after_no_elapsed_fail_stop_belong_to_the_next_process() {
    let control = Arc::new(ReceiptScenarioControl::new());
    let telemetry = Arc::new(V5ReceiptRuntimeTelemetry::new());
    let old = ScenarioHooks::install(telemetry.clone(), Some(control.clone()));
    old.forced_process_exit(None);
    assert!(!control.process_exited());
    assert_successor_barrier_configuration(&control, &old, &telemetry);
}

#[test]
fn future_barriers_after_permission_close_belong_to_the_next_process() {
    let control = Arc::new(ReceiptScenarioControl::new());
    let telemetry = Arc::new(V5ReceiptRuntimeTelemetry::new());
    let old = ScenarioHooks::install(telemetry.clone(), Some(control.clone()));
    control.process_life().close_permissions();
    assert!(!control.process_exited());
    assert_successor_barrier_configuration(&control, &old, &telemetry);
}

#[test]
fn future_barrier_queries_after_exit_select_the_next_process_configuration() {
    let control = Arc::new(ReceiptScenarioControl::new());
    let telemetry = Arc::new(V5ReceiptRuntimeTelemetry::new());
    let old = ScenarioHooks::install(telemetry.clone(), Some(control.clone()));
    old.forced_process_exit(Some(Duration::from_millis(1)));
    assert!(control.process_exited());
    assert_successor_barrier_configuration(&control, &old, &telemetry);
}

#[test]
fn successor_hooks_keep_old_process_dead_and_future_barriers_owned_by_the_new_life() {
    let control = Arc::new(ReceiptScenarioControl::new());
    let telemetry = Arc::new(V5ReceiptRuntimeTelemetry::new());
    let old_point = ScenarioBarrierPoint::BeforePrepare;
    let future_point = ScenarioBarrierPoint::BeforeTaskStoreCreate;
    control.install(old_point);
    let old = ScenarioHooks::install(telemetry.clone(), Some(control.clone()));
    old.forced_process_exit(Some(Duration::from_millis(1)));
    control.release_all_barriers();
    control.install(future_point);
    control.install(ScenarioBarrierPoint::AdmissionEntered);
    // Old-owner cleanup must not release a separately installed future barrier.
    control.release_all_barriers();
    let before = telemetry.snapshot();
    control
        .begin_successor_process()
        .expect("no old runtime owner");
    let new = ScenarioHooks::install(telemetry.clone(), Some(control.clone()));

    assert!(old.process_exited(), "retained old hooks were revived");
    assert!(
        !new.process_exited(),
        "successor inherited the old death bit"
    );
    assert!(old.holds(old_point));
    assert_eq!(
        old.pause(old_point, Instant::now()),
        Err(ReceiptLedgerError::StoreUnavailable),
        "released old barriers are not permission to resume a closed process"
    );
    assert!(!old.holds(future_point));
    assert!(!new.holds(old_point));
    assert!(new.holds(future_point));
    let exact_deadline = Instant::now();
    assert_eq!(
        old.commit_deadline_at(future_point, exact_deadline),
        exact_deadline
    );
    assert_eq!(
        new.commit_deadline_at(old_point, exact_deadline),
        exact_deadline
    );
    assert_eq!(
        new.pause(future_point, exact_deadline),
        Err(ReceiptLedgerError::DeadlineExceeded)
    );
    old.release_pre_actor_pauses();
    assert_eq!(
        new.pause(ScenarioBarrierPoint::AdmissionEntered, exact_deadline),
        Err(ReceiptLedgerError::DeadlineExceeded),
        "old cleanup released the successor admission barrier"
    );
    new.release_pre_actor_pauses();
    assert_eq!(
        new.pause(ScenarioBarrierPoint::AdmissionEntered, exact_deadline),
        Ok(())
    );
    assert_eq!(control.process_exit_elapsed_ms(), Some(1));
    assert_eq!(telemetry.snapshot().events.len(), before.events.len());
    assert_eq!(
        telemetry.snapshot().restart_requested,
        before.restart_requested
    );
    // A delayed notification from the retained old hook cannot kill the new life.
    old.forced_process_exit(Some(Duration::from_millis(2)));
    assert_eq!(
        control.process_exit_elapsed_ms(),
        Some(1),
        "a retained old hook must not rewrite the successor's exit-time observation"
    );
    assert!(old.process_exited());
    assert!(!new.process_exited());
    assert!(!control.process_exited());
    control.release(future_point);
    assert_eq!(new.pause(future_point, exact_deadline), Ok(()));
}

#[test]
fn successor_gate_permission_does_not_inherit_the_old_process_cancellation_request() {
    let control = Arc::new(ReceiptScenarioControl::new());
    let telemetry = Arc::new(V5ReceiptRuntimeTelemetry::new());
    let old = ScenarioHooks::install(telemetry.clone(), Some(control.clone()));
    control.request_gate_cancel();
    control.record_process_exit(1);
    control
        .begin_successor_process()
        .expect("old process has no runtime owner");
    let new = ScenarioHooks::install(telemetry, Some(control.clone()));
    let deadline = Instant::now();
    assert_eq!(
        new.wait_for_gate_cancel(deadline),
        Err(ReceiptLedgerError::DeadlineExceeded),
        "old cancellation would authorize cancelling a fresh Task"
    );
    assert_eq!(
        old.wait_for_gate_cancel(deadline),
        Err(ReceiptLedgerError::StoreUnavailable),
        "historical Cancel is not permission for a closed process"
    );
    control.request_gate_cancel();
    assert_eq!(new.wait_for_gate_cancel(deadline), Ok(()));
}

#[test]
fn retained_old_hook_cannot_release_the_successor_lifecycle_gate() {
    let control = Arc::new(ReceiptScenarioControl::new());
    let telemetry = Arc::new(V5ReceiptRuntimeTelemetry::new());
    let old = ScenarioHooks::install(telemetry.clone(), Some(control.clone()));
    control.record_process_exit(1);
    control
        .begin_successor_process()
        .expect("old process has no runtime owner");
    let new = ScenarioHooks::install(telemetry, Some(control));
    new.acquire_lifecycle_gate("new-owner", Instant::now() + Duration::from_secs(1))
        .expect("fresh process gate must be available");
    old.release_lifecycle_gate("old-owner");
    assert_eq!(
        new.acquire_lifecycle_gate("new-contender", Instant::now()),
        Err(ReceiptLedgerError::DeadlineExceeded),
        "old hook released the fresh owner gate"
    );
    new.release_lifecycle_gate("new-owner");
    new.acquire_lifecycle_gate("new-contender", Instant::now() + Duration::from_secs(1))
        .expect("fresh owner releases its own gate");
    new.release_lifecycle_gate("new-contender");
}

#[test]
fn successor_closes_and_joins_old_gate_waiters_without_cancelling_the_new_life() {
    let control = Arc::new(ReceiptScenarioControl::new());
    let telemetry = Arc::new(V5ReceiptRuntimeTelemetry::new());
    let old = ScenarioHooks::install(telemetry.clone(), Some(control.clone()));
    let deadline = Instant::now() + Duration::from_secs(5);
    old.acquire_lifecycle_gate("old-owner", deadline)
        .expect("hold old gate");
    let gate_control = control.clone();
    let old_life = control.process_life();
    let gate_waiter = thread::spawn(move || {
        gate_control.acquire_lifecycle_gate_for(&old_life, "old-waiter", deadline)
    });
    control
        .wait_for_operation_event("old-waiter", "blocked", deadline)
        .expect("old gate waiter entered its actual blocked loop");
    let old_cancel = old.clone();
    let cancel_waiter = thread::spawn(move || old_cancel.wait_for_gate_cancel(deadline));
    while !control.gate_cancel_waiting() {
        assert!(
            Instant::now() < deadline,
            "old cancel waiter did not enter its wait"
        );
        thread::yield_now();
    }
    // Actual explicit-exit fixture boundary: no Runtime owner is released early.
    // External waiters retain only their old process Life, not a Runtime Arc.
    control.record_process_exit(1);
    control
        .begin_successor_process()
        .expect("no old runtime owner remains");
    let new = ScenarioHooks::install(telemetry, Some(control));
    new.acquire_lifecycle_gate("new-owner", deadline)
        .expect("fresh gate is independent");
    assert_eq!(
        gate_waiter.join().expect("old gate waiter must join"),
        Err(ReceiptLedgerError::StoreUnavailable),
        "old wait was abandoned until its deadline"
    );
    assert_eq!(
        cancel_waiter.join().expect("old cancel waiter must join"),
        Err(ReceiptLedgerError::StoreUnavailable),
        "closing a life must not fabricate Cancel"
    );
    assert_eq!(
        new.acquire_lifecycle_gate("new-contender", Instant::now()),
        Err(ReceiptLedgerError::DeadlineExceeded),
        "old teardown released the fresh gate"
    );
    assert_eq!(
        new.wait_for_gate_cancel(Instant::now()),
        Err(ReceiptLedgerError::DeadlineExceeded),
        "old teardown invented fresh cancellation"
    );
    new.release_lifecycle_gate("new-owner");
}

#[test]
fn closing_permissions_does_not_claim_process_exit() {
    let control = Arc::new(ReceiptScenarioControl::new());
    let telemetry = Arc::new(V5ReceiptRuntimeTelemetry::new());
    let hooks = ScenarioHooks::install(telemetry.clone(), Some(control.clone()));
    control.install(ScenarioBarrierPoint::BeforePrepare);
    control.process_life().close_permissions();
    assert!(!hooks.process_exited());
    assert!(!control.process_exited());
    assert!(!control.current_process_requires_stop());
    assert_eq!(control.process_exit_elapsed_ms(), None);
    assert!(!telemetry.snapshot().restart_requested);
    assert_eq!(
        hooks.acquire_lifecycle_gate("closed", Instant::now()),
        Err(ReceiptLedgerError::StoreUnavailable)
    );
    assert_eq!(
        hooks.wait_for_gate_cancel(Instant::now()),
        Err(ReceiptLedgerError::StoreUnavailable)
    );
    assert_eq!(
        hooks.pause(ScenarioBarrierPoint::BeforePrepare, Instant::now()),
        Err(ReceiptLedgerError::StoreUnavailable)
    );
}

#[test]
fn fail_stop_request_without_exit_belongs_only_to_its_captured_process_life() {
    let control = Arc::new(ReceiptScenarioControl::new());
    let telemetry = Arc::new(V5ReceiptRuntimeTelemetry::new());
    let old = ScenarioHooks::install(telemetry.clone(), Some(control.clone()));
    old.restart_requested();
    assert!(control.current_process_requires_stop());
    assert!(!old.process_exited());
    assert_eq!(control.process_exit_elapsed_ms(), None);
    control
        .begin_successor_process()
        .expect("no old Runtime owner");
    let fresh = ScenarioHooks::install(telemetry, Some(control.clone()));
    assert!(!control.current_process_requires_stop());
    old.forced_process_exit(None);
    assert!(
        !control.current_process_requires_stop(),
        "old request closed fresh life"
    );
    fresh.forced_process_exit(None);
    assert!(control.current_process_requires_stop());
    assert!(!fresh.process_exited(), "a stop request is not actual exit");
    assert_eq!(control.process_exit_elapsed_ms(), None);
}
