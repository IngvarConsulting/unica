use super::*;

#[derive(Clone, Copy, Debug)]
enum LinkedState {
    Terminal,
    Bound,
    Handoff,
    Expired,
    Pending,
}

fn damage_task(path: &std::path::Path, oversized: bool) -> Vec<u8> {
    let bytes = if oversized {
        vec![b'x'; unica_execution_contract::result_size::MAX_TASK_RECORD_BYTES + 1]
    } else {
        b"{synthetic invalid Task JSON".to_vec()
    };
    std::fs::write(path, &bytes).unwrap();
    bytes
}

fn assert_isolated_task_error<T>(answer: Result<T, ReceiptLedgerError>) {
    let error = answer
        .err()
        .expect("the isolated Task must not produce a result");
    assert!(
        matches!(&error, ReceiptLedgerError::IsolatedTaskRecord { .. }),
        "unexpected non-payload failure: {error:?}"
    );
    assert!(
        !error.requires_reopen(),
        "a local payload cannot fail-stop every Task: {error:?}"
    );
    assert_eq!(daemon_error_code(&error), V5DaemonErrorCode::StoreFailed);
}

fn assert_linked_payload_isolation(oversized: bool, kind: LinkedState) {
    let root = tempfile::tempdir().unwrap();
    let state_root = std::fs::canonicalize(root.path()).unwrap();
    let identity = CoreIdentity::production_v5();
    let state = DaemonStateDirectory::open(&state_root, &identity).unwrap();
    let service = Arc::new(CountingService::default());
    let clock = Arc::new(ManualEpochClock::new(1_000));
    let config = DaemonServerConfig::new(state_root, identity.clone(), Duration::from_millis(50))
        .with_invocation_service(service.clone())
        .with_v5_epoch_clock_for_test(clock.clone());
    let runtime = V5ReceiptRuntime::open(&state, &config.receipt_runtime_config()).unwrap();
    let bad_key = match kind {
        LinkedState::Bound | LinkedState::Handoff => {
            let handoff = prepare_staged_startup_handoff(&runtime, &identity, AttemptPhase::Begun);
            let (_, bound) = materialize_bound_handoff_for_fixture(
                &runtime,
                &handoff,
                1_009,
                Instant::now() + TEST_STEP,
                runtime.hooks().as_ref(),
            )
            .unwrap_or_else(|failure| panic!("seed bound Task: {}", failure.error()));
            if matches!(kind, LinkedState::Bound) {
                receipt_actor_for_fixture(&runtime)
                    .complete_bound_task_handoff(
                        handoff.key().clone(),
                        handoff.record_version(),
                        bound,
                        Instant::now() + TEST_STEP,
                    )
                    .unwrap();
            }
            handoff.key().clone()
        }
        _ => materialize_completed_task_key(
            &runtime,
            &identity,
            &clock,
            "workspace-a",
            1_000,
            2_000,
            "bad result",
        ),
    };
    let bad_id = bad_key.reserved_task_id();
    if matches!(kind, LinkedState::Pending) {
        let Some(TaskLifecycleLinkRecord::TaskTerminalBound(terminal)) =
            retirement_link(&runtime, bad_id)
        else {
            panic!("terminal fixture")
        };
        lifecycle_links_for_fixture(&runtime)
            .begin_task_retirement(
                &terminal,
                64,
                64,
                unica_work_control::provider_deadline::ProviderDeadline::new(
                    Instant::now() + TEST_STEP,
                ),
            )
            .unwrap();
    }
    let now = if matches!(kind, LinkedState::Expired | LinkedState::Pending) {
        RETIREMENT_TEST_OBSERVED_EPOCH_MS
    } else {
        10_000
    };
    let good = materialize_completed_task(
        &runtime,
        &identity,
        &clock,
        "workspace-good",
        now - 1_000,
        now - 500,
        "live result",
    );
    clock.set(now);
    let link_before = retirement_link(&runtime, bad_id).unwrap();
    let task_path = state.path().join("tasks").join(format!("{bad_id}.json"));
    drop(runtime);
    let damaged = damage_task(&task_path, oversized);
    let reopened = Arc::new(
        V5ReceiptRuntime::open(&state, &config.receipt_runtime_config()).unwrap_or_else(|error| {
            panic!("{kind:?} one damaged Task prevented runtime startup: {error}")
        }),
    );
    for answer in [
        reopened.resolve_task(bad_id, Instant::now() + TEST_STEP),
        reopened.wait_task(bad_id, 0, Instant::now() + TEST_STEP),
        reopened.cancel_task(bad_id, Instant::now() + TEST_STEP),
    ] {
        assert_isolated_task_error(answer);
        assert_live_task_answers(
            &reopened,
            good,
            &format!("after {kind:?} isolated Task read"),
        );
    }
    let before = receipt_actor_for_fixture(&reopened)
        .generation(Instant::now() + TEST_STEP)
        .unwrap();
    let invocation = V5InvocationRequest::new(
        bad_key.invocation_id(),
        bad_id,
        V5ToolIdentity::View,
        serde_json::Map::new(),
        unica_bootstrap::ResolvedWorkspace::launch_cwd("workspace-a".to_owned()),
        7_000,
    )
    .unwrap();
    let decoded = decode_v5_request_frame(
        serde_json::to_vec(&V5ClientRequest::SubmitInvocation { invocation }).unwrap(),
    )
    .unwrap();
    assert_isolated_task_error(reopened.submit_invocation(
        unica_daemon::protocol_v5::DecodedSubmitAdapter::new(decoded, identity),
        now,
        Instant::now() + TEST_STEP,
        reopened.capture_submit_deadline(7_000),
    ));
    assert_eq!(
        receipt_actor_for_fixture(&reopened)
            .generation(Instant::now() + TEST_STEP)
            .unwrap(),
        before,
        "an exact repeat cannot reserve or recover the isolated attempt again"
    );
    assert_eq!(
        retirement_link(&reopened, bad_id),
        Some(link_before),
        "isolation cannot release a link or retirement intent"
    );
    assert_eq!(std::fs::read(task_path).unwrap(), damaged);
    assert_eq!(service.prepares.load(Ordering::SeqCst), 0);
    assert_eq!(service.executions.load(Ordering::SeqCst), 0);
    assert!(!reopened.restart_required());
}

/// rule: INV.APP.DAEMON-TASK-PERSISTENCE
/// rule: INV.APP.DAEMON-TASK-RECOVERY
#[test]
fn corrupt_linked_task_is_isolated_through_startup_reads_and_exact_repeat() {
    for kind in [
        LinkedState::Terminal,
        LinkedState::Bound,
        LinkedState::Handoff,
        LinkedState::Expired,
        LinkedState::Pending,
    ] {
        assert_linked_payload_isolation(false, kind);
    }
}

/// rule: INV.APP.DAEMON-TASK-PERSISTENCE
/// rule: INV.APP.DAEMON-TASK-RECOVERY
#[test]
fn oversized_linked_task_is_isolated_through_startup_reads_and_exact_repeat() {
    for kind in [
        LinkedState::Terminal,
        LinkedState::Bound,
        LinkedState::Handoff,
        LinkedState::Expired,
        LinkedState::Pending,
    ] {
        assert_linked_payload_isolation(true, kind);
    }
}

/// rule: INV.APP.DAEMON-TASK-RECOVERY
#[test]
fn corrupt_staged_task_preserves_handoff_and_does_not_replay_the_saved_terminal() {
    let StagedStartupCrashFixture {
        _root,
        state,
        config,
        clock,
        service,
        key,
        runtime,
        task_path,
        ..
    } = staged_startup_crash_fixture();
    let receipt = receipt_actor_for_fixture(&runtime)
        .recover(key.clone(), Instant::now() + TEST_STEP)
        .unwrap();
    let link = retirement_link(&runtime, key.reserved_task_id()).unwrap();
    drop(runtime);
    let bytes = damage_task(&task_path, false);
    let reopened =
        V5ReceiptRuntime::open_with_epoch_clock(&state, &config.receipt_runtime_config(), clock)
            .unwrap();
    assert_isolated_task_error(
        reopened.resolve_task(key.reserved_task_id(), Instant::now() + TEST_STEP),
    );
    assert_eq!(
        receipt_actor_for_fixture(&reopened)
            .recover(key.clone(), Instant::now() + TEST_STEP)
            .unwrap(),
        receipt
    );
    assert_eq!(
        retirement_link(&reopened, key.reserved_task_id()),
        Some(link)
    );
    assert_eq!(std::fs::read(task_path).unwrap(), bytes);
    assert_eq!(service.prepares.load(Ordering::SeqCst), 0);
    assert!(!reopened.restart_required());
}

/// rule: INV.APP.DAEMON-TASK-RECOVERY
#[test]
fn direct_reservation_recovers_even_when_its_unmaterialized_task_id_has_an_orphan_payload() {
    let root = tempfile::tempdir().unwrap();
    let state_root = std::fs::canonicalize(root.path()).unwrap();
    let identity = CoreIdentity::production_v5();
    let state = DaemonStateDirectory::open(&state_root, &identity).unwrap();
    let config = DaemonServerConfig::new(state_root, identity.clone(), Duration::from_millis(50));
    let runtime = V5ReceiptRuntime::open(&state, &config.receipt_runtime_config()).unwrap();
    let key = ReceiptKey::new(
        InvocationId::new(),
        TaskId::new(),
        RequestIdentity::new(
            identity.digest().clone(),
            V5ToolIdentity::View,
            normalized_arguments_hash(&serde_json::Map::new()),
            request_scope_hash("workspace-a").unwrap(),
        ),
    );
    receipt_actor_for_fixture(&runtime)
        .reserve(
            key.clone(),
            OriginalCutoffDescriptor::new(1_000, 7_000).unwrap(),
            Instant::now() + TEST_STEP,
        )
        .unwrap();
    let tasks =
        unica_workspace::state_fs::open_state_directory_nofollow(&state.path().join("tasks"))
            .unwrap();
    let name = format!("{}.json", key.reserved_task_id());
    let mut file =
        unica_workspace::state_fs::create_state_file_child(&tasks, std::ffi::OsStr::new(&name))
            .unwrap();
    unica_workspace::state_fs::restrict_state_stage_to_owner(&file).unwrap();
    file.write_all(b"{orphan invalid Task JSON").unwrap();
    drop(file);
    drop(runtime);
    let reopened = V5ReceiptRuntime::open(&state, &config.receipt_runtime_config()).unwrap();
    assert!(
        matches!(
            receipt_actor_for_fixture(&reopened)
                .recover(key, Instant::now() + TEST_STEP)
                .unwrap(),
            ReceiptState::DirectTerminalUnacked(_)
        ),
        "an unrelated orphan must not suspend ordinary direct recovery"
    );
    assert!(!reopened.restart_required());
}

/// rule: INV.APP.DAEMON-TASK-RECOVERY
#[test]
fn corrupt_task_does_not_hide_independent_staged_handoff_authority_mismatch() {
    let StagedStartupCrashFixture {
        _root,
        state,
        config,
        clock,
        key,
        runtime,
        task_path,
        ..
    } = staged_startup_crash_fixture();
    drop(runtime);
    let link_path = state
        .path()
        .join("task-lifecycle-links/task-lifecycle-links-v1.json");
    let mut links: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&link_path).unwrap()).unwrap();
    assert_eq!(links["entries"][0]["phase"], json!("begun"));
    links["entries"][0]["phase"] = json!("not_begun");
    let changed = serde_json::to_vec(&links).unwrap();
    std::fs::write(&link_path, &changed).unwrap();
    {
        let parsed = TaskLifecycleLinkStoreV5::open(
            state.path().join("task-lifecycle-links"),
            unica_work_control::provider_deadline::ProviderDeadline::new(
                Instant::now() + TEST_STEP,
            ),
        )
        .unwrap();
        let TaskLifecycleLinkRecord::TaskBound(link) = parsed
            .read_by_task_id(
                key.reserved_task_id(),
                unica_work_control::provider_deadline::ProviderDeadline::new(
                    Instant::now() + TEST_STEP,
                ),
            )
            .unwrap()
        else {
            panic!("valid TaskBound fixture")
        };
        assert_eq!(link.phase(), AttemptPhase::NotBegun);
    }
    for damaged in [false, true] {
        if damaged {
            damage_task(&task_path, false);
        }
        let error = V5ReceiptRuntime::open_with_epoch_clock(
            &state,
            &config.receipt_runtime_config(),
            clock.clone(),
        )
        .err()
        .expect("independent staged authority mismatch must still refuse startup");
        assert!(
            error.contains("staged startup TaskBound authority differs"),
            "{error}"
        );
        assert_eq!(std::fs::read(&link_path).unwrap(), changed);
    }
}

/// A canonical regular record can be unreadable before its payload is inspected.
#[cfg_attr(
    not(windows),
    ignore = "host capability Windows file sharing: requires an exclusive native read handle"
)]
/// rule: INV.APP.DAEMON-TASK-PERSISTENCE
#[test]
fn windows_record_sharing_failure_is_storage_even_for_an_isolated_payload() {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        let root = tempfile::tempdir().unwrap();
        let state_root = std::fs::canonicalize(root.path()).unwrap();
        let identity = CoreIdentity::production_v5();
        let state = DaemonStateDirectory::open(&state_root, &identity).unwrap();
        let clock = Arc::new(ManualEpochClock::new(1_000));
        let config =
            DaemonServerConfig::new(state_root, identity.clone(), Duration::from_millis(50))
                .with_v5_epoch_clock_for_test(clock.clone());
        let runtime = V5ReceiptRuntime::open(&state, &config.receipt_runtime_config()).unwrap();
        let bad = materialize_completed_task(
            &runtime,
            &identity,
            &clock,
            "bad",
            1_000,
            2_000,
            "bad result",
        );
        let good = materialize_completed_task(
            &runtime,
            &identity,
            &clock,
            "good",
            2_001,
            2_002,
            "good result",
        );
        let tasks = state.path().join("tasks");
        let path = tasks.join(format!("{bad}.json"));
        drop(runtime);
        damage_task(&path, false);
        let held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .unwrap();
        let open = || {
            FileInvocationStoreV5::open_retained_directory_inspect_only(
                unica_workspace::state_fs::StateDirectory::open(&tasks).unwrap(),
                clock.clone(),
                unica_work_control::provider_deadline::ProviderDeadline::new(
                    Instant::now() + TEST_STEP,
                ),
            )
        };
        assert!(matches!(
            open(),
            Err(V5TaskStoreError::Storage {
                operation: "open protocol-v5 task record",
                ..
            })
        ));
        drop(held);
        let (store, catalog) = open().unwrap();
        assert_eq!(catalog.entries().len(), 1);
        let held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .unwrap();
        assert!(
            matches!(
                store.get(
                    bad,
                    unica_work_control::provider_deadline::ProviderDeadline::new(
                        Instant::now() + TEST_STEP
                    )
                ),
                Err(V5TaskStoreError::Storage {
                    operation: "open protocol-v5 task record",
                    ..
                })
            ),
            "the immutable quarantine must not mask a new I/O failure"
        );
        drop(held);
        assert!(matches!(
            store.get(
                bad,
                unica_work_control::provider_deadline::ProviderDeadline::new(
                    Instant::now() + TEST_STEP
                )
            ),
            Err(V5TaskStoreError::IsolatedRecord { .. })
        ));
        assert_eq!(
            store
                .get(
                    good,
                    unica_work_control::provider_deadline::ProviderDeadline::new(
                        Instant::now() + TEST_STEP
                    )
                )
                .unwrap()
                .task
                .status(),
            V5TaskStatus::Completed
        );
    }
}

/// The authenticated transport must answer a second request in the same session.
/// rule: INV.APP.DAEMON-TASK-RECOVERY
#[test]
fn wire_session_reads_a_healthy_task_after_each_isolated_payload_without_stopping() {
    for oversized in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let state_root = std::fs::canonicalize(root.path()).unwrap();
        let identity = CoreIdentity::production_v5();
        let state = DaemonStateDirectory::open(&state_root, &identity).unwrap();
        let clock = Arc::new(ManualEpochClock::new(1_000));
        let service = Arc::new(CountingService::default());
        let config = DaemonServerConfig::new(
            state_root.clone(),
            identity.clone(),
            Duration::from_millis(50),
        )
        .with_v5_epoch_clock_for_test(clock.clone())
        .with_invocation_service(service.clone());
        let runtime = V5ReceiptRuntime::open(&state, &config.receipt_runtime_config()).unwrap();
        let bad = materialize_completed_task(
            &runtime,
            &identity,
            &clock,
            "bad",
            1_000,
            2_000,
            "bad result",
        );
        let good = materialize_completed_task(
            &runtime,
            &identity,
            &clock,
            "good",
            2_001,
            2_002,
            "live result",
        );
        let path = state.path().join("tasks").join(format!("{bad}.json"));
        drop(runtime);
        let bytes = damage_task(&path, oversized);
        let stop = Arc::new(AtomicBool::new(false));
        let server_stop = stop.clone();
        let server = thread::spawn(move || {
            run_daemon_configured_until(
                config.into_transport_config(),
                |runtime| runtime,
                || server_stop.load(Ordering::SeqCst),
            )
        });
        let mut daemon = HandlerSpawnDaemon {
            stop,
            streams: Vec::new(),
            server: Some(server),
        };
        let record = wait_for_v5_record(&state_root, &identity);
        let mut owner = connect_v5_owner(&record, &identity, &Uuid::new_v4().to_string());
        daemon.streams.push(owner.0.try_clone().unwrap());
        for _ in 0..2 {
            assert_eq!(
                idle_request_over_tcp(&mut owner, V5ClientRequest::GetTask { task_id: bad }),
                V5ServerResponse::Error {
                    code: V5DaemonErrorCode::StoreFailed
                }
            );
            let answer =
                idle_request_over_tcp(&mut owner, V5ClientRequest::GetTask { task_id: good });
            assert!(
                matches!(answer,V5ServerResponse::Task{snapshot:V5DaemonTaskSnapshot::Completed{result,..}}
                if result.ok&&result.summary=="live result")
            );
            assert_eq!(
                state.read_v5_endpoint_record().unwrap(),
                Some(record.clone())
            );
            assert!(!daemon.server.as_ref().unwrap().is_finished());
        }
        release_promised_work_owner(owner);
        for stream in &daemon.streams {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
        let until = Instant::now() + TEST_STEP;
        while !daemon.server.as_ref().unwrap().is_finished() {
            assert!(
                Instant::now() < until,
                "the healthy daemon did not exit through its ordinary idle path"
            );
            thread::sleep(Duration::from_millis(5));
        }
        daemon.server.take().unwrap().join().unwrap().unwrap();
        assert_eq!(std::fs::read(path).unwrap(), bytes);
        assert_eq!(service.prepares.load(Ordering::SeqCst), 0);
        assert_eq!(service.executions.load(Ordering::SeqCst), 0);
    }
}

/// rule: INV.APP.DAEMON-TASK-RECOVERY
#[test]
fn corrupt_unstaged_task_does_not_hide_handoff_projection_mismatch() {
    for field in ["pollIntervalMs", "ttlMs", "createdAtEpochMs"] {
        let root = tempfile::tempdir().unwrap();
        let state_root = std::fs::canonicalize(root.path()).unwrap();
        let identity = CoreIdentity::production_v5();
        let state = DaemonStateDirectory::open(&state_root, &identity).unwrap();
        let config =
            DaemonServerConfig::new(state_root, identity.clone(), Duration::from_millis(50));
        let runtime = V5ReceiptRuntime::open(&state, &config.receipt_runtime_config()).unwrap();
        let handoff = prepare_staged_startup_handoff(&runtime, &identity, AttemptPhase::Begun);
        materialize_bound_handoff_for_fixture(
            &runtime,
            &handoff,
            1_009,
            Instant::now() + TEST_STEP,
            runtime.hooks().as_ref(),
        )
        .unwrap_or_else(|failure| panic!("seed handoff: {}", failure.error()));
        let key = handoff.key().clone();
        drop(runtime);
        let link_path = state
            .path()
            .join("task-lifecycle-links/task-lifecycle-links-v1.json");
        let mut links: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&link_path).unwrap()).unwrap();
        let before = links["entries"][0]["task"][field].as_u64().unwrap();
        let changed = if field == "createdAtEpochMs" {
            before - 1
        } else {
            before + 1
        };
        links["entries"][0]["task"][field] = json!(changed);
        let bytes = serde_json::to_vec(&links).unwrap();
        std::fs::write(&link_path, &bytes).unwrap();
        {
            let parsed = TaskLifecycleLinkStoreV5::open(
                state.path().join("task-lifecycle-links"),
                unica_work_control::provider_deadline::ProviderDeadline::new(
                    Instant::now() + TEST_STEP,
                ),
            )
            .unwrap();
            assert!(
                matches!(
                    parsed
                        .read_by_task_id(
                            key.reserved_task_id(),
                            unica_work_control::provider_deadline::ProviderDeadline::new(
                                Instant::now() + TEST_STEP
                            )
                        )
                        .unwrap(),
                    TaskLifecycleLinkRecord::TaskBound(_)
                ),
                "the altered metadata is valid link schema"
            );
        }
        for damaged in [false, true] {
            if damaged {
                damage_task(
                    &state
                        .path()
                        .join("tasks")
                        .join(format!("{}.json", key.reserved_task_id())),
                    false,
                );
            }
            let error = V5ReceiptRuntime::open(&state, &config.receipt_runtime_config())
                .err()
                .unwrap_or_else(|| {
                    panic!("{field}: a damaged payload waived the handoff/link projection mismatch")
                });
            assert!(error.contains("handoff"), "{error}");
            assert_eq!(std::fs::read(&link_path).unwrap(), bytes);
        }
    }
}
