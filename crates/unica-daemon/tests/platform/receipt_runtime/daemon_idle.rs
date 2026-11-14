use super::*;

/// rule: INV.RUNTIME.DAEMON-ACTIVE-OWNERS
#[test]
fn daemon_idle_saved_apply_plan_preserves_instance_and_exact_replay_after_owner_release() {
    let workspace =
        crate::infrastructure::daemon::server::actor_capacity_tests::subsystem_picture_workspace(
            "", false,
        );
    let descriptor = workspace.path().join("src/Subsystems/Sales.xml");
    let before = std::fs::read(&descriptor).unwrap();
    let idle = Duration::from_millis(80);
    let mut daemon = PromisedWorkIdleDaemon::start(idle);
    let mut owner = daemon.owner();
    let token = promised_work_preview_over_tcp(&mut owner, workspace.path());
    assert_eq!(
        before,
        std::fs::read(&descriptor).unwrap(),
        "preview must not write"
    );
    release_promised_work_owner(owner);

    let released = Instant::now();
    assert_held_by(
        &daemon.held_without_clients_since(released),
        |owners| owners.retained_workspace_work && !owners.task_cancellations,
        "the saved plan, as retained workspace work",
    );
    assert_eq!(
        daemon.endpoint(),
        Some(daemon.record.clone()),
        "idle cleanup discarded an unexecuted apply plan and its daemon instance"
    );
    let mut successor = daemon.owner();
    let arguments = json!({"executionToken": token});
    let applied = promised_work_apply_over_tcp(&mut successor, workspace.path(), arguments.clone());
    assert!(applied.ok, "{applied:?}");
    let written = std::fs::read(&descriptor).unwrap();
    assert_ne!(before, written);
    release_promised_work_owner(successor);
    let released = Instant::now();
    assert_held_by(
        &daemon.held_without_clients_since(released),
        |owners| owners.retained_workspace_work && !owners.task_cancellations,
        "the completed plan's replay result, as retained workspace work",
    );
    assert_eq!(
        daemon.endpoint(),
        Some(daemon.record.clone()),
        "idle cleanup discarded a completed plan's replay result"
    );
    let mut replay_owner = daemon.owner();
    assert_eq!(
        promised_work_apply_over_tcp(&mut replay_owner, workspace.path(), arguments),
        applied
    );
    assert_eq!(
        written,
        std::fs::read(&descriptor).unwrap(),
        "replay must not mutate again"
    );
    release_promised_work_owner(replay_owner);
}

/// rule: INV.RUNTIME.DAEMON-ACTIVE-OWNERS
#[test]
fn daemon_idle_empty_runtime_still_exits_after_owner_release() {
    let idle = Duration::from_millis(80);
    let mut daemon = PromisedWorkIdleDaemon::start(idle);
    let owner = daemon.owner();
    release_promised_work_owner(owner);
    daemon.await_idle_exit();
    daemon.join();
    assert!(daemon.endpoint().is_none());
}

/// rule: INV.RUNTIME.DAEMON-ACTIVE-OWNERS
#[test]
fn daemon_idle_saved_apply_plan_does_not_prevent_explicit_stop() {
    let workspace =
        crate::infrastructure::daemon::server::actor_capacity_tests::subsystem_picture_workspace(
            "", false,
        );
    let mut daemon = PromisedWorkIdleDaemon::start(Duration::from_secs(30));
    let mut owner = daemon.owner();
    let token = promised_work_preview_over_tcp(&mut owner, workspace.path());
    assert!(token.as_str().is_some_and(|value| !value.is_empty()));
    release_promised_work_owner(owner);
    daemon.guard.stop.store(true, Ordering::SeqCst);
    daemon
        .exited
        .recv_timeout(TEST_STEP)
        .expect("explicit stop must exit despite a pending apply plan");
    assert!(daemon.guard.stop.load(Ordering::SeqCst));
    daemon.join();
    assert!(daemon.endpoint().is_none());
}

/// rule: INV.RUNTIME.DAEMON-ACTIVE-OWNERS
#[test]
fn daemon_idle_running_index_producer_without_rpc_or_lease_preserves_instance_then_retires() {
    let workspace =
        crate::infrastructure::daemon::server::actor_capacity_tests::subsystem_picture_workspace(
            "", false,
        );
    let producers = Arc::new(AtomicUsize::new(0));
    let release = Arc::new((Mutex::new(false), Condvar::new()));
    let (entered, started) = mpsc::channel();
    let (exited, finished) = mpsc::channel();
    let (follower, joined) = mpsc::channel();
    let service = Arc::new(IdleIndexService {
        calls: AtomicUsize::new(0),
        keep_first_lease: false,
        producers: Arc::clone(&producers),
        entered,
        exited,
        release: Arc::clone(&release),
        follower,
    });
    let idle = Duration::from_millis(80);
    let mut daemon = PromisedWorkIdleDaemon::with_service(idle, service);
    // Declared after the daemon so failure releases the producer before the
    // daemon's stop/join guard runs. No SharedWork lease is retained here.
    let cleanup = IdleIndexProducerCleanup {
        release,
        exited: finished,
    };
    let mut owner = daemon.owner();
    let call = |owner: &mut (TcpStream, BufReader<TcpStream>)| {
        promised_work_call_over_tcp(
            owner,
            workspace.path(),
            V5ToolIdentity::View,
            json!({"at": "main:Subsystem.Sales"}),
        )
    };
    assert!(call(&mut owner).ok);
    started
        .recv_timeout(TEST_STEP)
        .expect("the real SharedWork producer starts");
    release_promised_work_owner(owner);
    let released = Instant::now();
    let held = daemon.held_without_clients_since(released);
    assert_eq!(
        daemon.endpoint(),
        Some(daemon.record.clone()),
        "idle shutdown lost a running index producer after the RPC and its lease ended"
    );
    assert_held_by(
        &held,
        |owners| owners.retained_workspace_work && !owners.task_cancellations,
        "the running producer, as retained workspace work",
    );
    let mut successor = daemon.owner();
    assert!(call(&mut successor).ok);
    let lease = joined
        .recv_timeout(TEST_STEP)
        .expect("second real RPC attaches to the same exact index work");
    assert_eq!(producers.load(Ordering::SeqCst), 1);
    release_promised_work_owner(successor);
    cleanup.release();
    assert!(
        matches!(
            lease.wait_timeout(TEST_STEP),
            unica_work_control::shared_work::SharedWorkSnapshot::Ready(_)
        ),
        "producer must actually settle before its last lease is released"
    );
    drop(lease);
    daemon.await_idle_exit();
    daemon.join();
    assert!(daemon.endpoint().is_none());
}

/// rule: INV.RUNTIME.DAEMON-ACTIVE-OWNERS
#[test]
fn daemon_idle_live_task_after_client_release_preserves_attempt_then_retires() {
    let (_workspace, workspace) = catalog_workspace();
    let (entered, executing) = mpsc::channel();
    let release = Arc::new((Mutex::new(false), Condvar::new()));
    let service = Arc::new(IdleInvocationService {
        known_long: true,
        prepares: AtomicUsize::new(0),
        executions: AtomicUsize::new(0),
        entered,
        release: Arc::clone(&release),
    });
    let idle = Duration::from_millis(80);
    let mut daemon = PromisedWorkIdleDaemon::with_service(idle, service.clone());
    // Release before the daemon's panic cleanup joins the held execution.
    let cleanup = IdleInvocationRelease(release);
    let invocation = view_catalog_request(TaskId::new(), &workspace);
    let mut owner = daemon.owner();
    let initial = submitted_task_snapshot(Ok(idle_request_over_tcp(
        &mut owner,
        V5ClientRequest::SubmitInvocation {
            invocation: invocation.clone(),
        },
    )));
    assert_eq!(initial.task_id(), invocation.reserved_task_id());
    assert_eq!(initial.invocation_id(), invocation.invocation_id());
    executing.recv_timeout(TEST_STEP).unwrap();
    release_promised_work_owner(owner);
    let released = Instant::now();

    let held = daemon.held_without_clients_since(released);
    assert_eq!(daemon.endpoint(), Some(daemon.record.clone()));
    assert_held_by(
        &held,
        |owners| owners.task_cancellations,
        "the live Task, through its cancellation registration",
    );
    let mut successor = daemon.owner();
    let V5ServerResponse::Task { snapshot: running } = idle_request_over_tcp(
        &mut successor,
        V5ClientRequest::GetTask {
            task_id: invocation.reserved_task_id(),
        },
    ) else {
        panic!("successor must see the original live Task");
    };
    assert_eq!(running.task_id(), initial.task_id());
    assert_eq!(running.invocation_id(), initial.invocation_id());
    assert!(!task_snapshot_is_terminal(&running));
    cleanup.release();
    let terminal = idle_task_terminal_over_tcp(&mut successor, &invocation);
    assert!(
        matches!(&terminal, V5DaemonTaskSnapshot::Completed { result, .. }
        if result.ok && result.data == Some(json!({"attempt": 1})))
    );
    let replay = submitted_task_snapshot(Ok(idle_request_over_tcp(
        &mut successor,
        V5ClientRequest::SubmitInvocation { invocation },
    )));
    assert_eq!(replay, terminal);
    assert_eq!(service.prepares.load(Ordering::SeqCst), 1);
    assert_eq!(service.executions.load(Ordering::SeqCst), 1);
    release_promised_work_owner(successor);
    daemon.await_idle_exit();
    daemon.join();
    assert!(daemon.endpoint().is_none());
}

/// rule: INV.RUNTIME.DAEMON-ACTIVE-OWNERS
#[test]
fn daemon_idle_disconnected_inline_invocation_preserves_attempt_then_retires() {
    let (_workspace, workspace) = catalog_workspace();
    let (entered, executing) = mpsc::channel();
    let release = Arc::new((Mutex::new(false), Condvar::new()));
    let service = Arc::new(IdleInvocationService {
        known_long: false,
        prepares: AtomicUsize::new(0),
        executions: AtomicUsize::new(0),
        entered,
        release: Arc::clone(&release),
    });
    let idle = Duration::from_millis(80);
    let mut daemon = PromisedWorkIdleDaemon::with_service(idle, service.clone());
    let cleanup = IdleInvocationRelease(release);
    let invocation = view_catalog_request(TaskId::new(), &workspace);
    let mut owner = daemon.owner();
    write_json_line(
        &mut owner.0,
        &serde_json::to_value(V5ClientRequest::SubmitInvocation {
            invocation: invocation.clone(),
        })
        .unwrap(),
    );
    executing.recv_timeout(TEST_STEP).unwrap();
    owner.0.shutdown(std::net::Shutdown::Both).unwrap();
    drop(owner);
    let disconnected = Instant::now();

    // The client has left, but the synchronous handler still owns its lease
    // until this invocation returns. Do not call its registry empty.
    let held = daemon.decisions_since(disconnected, |_| true);
    assert_eq!(daemon.endpoint(), Some(daemon.record.clone()));
    // Besides that lease, the begun attempt holds through its own
    // cancellation registration.
    assert_held_by(
        &held,
        |owners| owners.task_cancellations,
        "the begun inline attempt, through its cancellation registration",
    );
    let mut successor = daemon.owner();
    cleanup.release();
    // The released handler may not have published its receipt when the repeat
    // arrives; the daemon then answers `ReceiptPending` for the same key, and a
    // client repeats the exact call, as after `receipt_pending`. The repeat
    // still has to recover this attempt, not run a second one.
    let settles_by = Instant::now() + TEST_STEP;
    let recovered = loop {
        let response = idle_request_over_tcp(
            &mut successor,
            V5ClientRequest::SubmitInvocation {
                invocation: invocation.clone(),
            },
        );
        let V5ServerResponse::Invocation {
            outcome:
                V5InvocationResponse::ReceiptPending {
                    receipt_key,
                    cancel_requested,
                    ..
                },
        } = &response
        else {
            break response;
        };
        assert_eq!(receipt_key.invocation_id(), invocation.invocation_id());
        assert_eq!(
            receipt_key.reserved_task_id(),
            invocation.reserved_task_id()
        );
        assert!(
            !cancel_requested,
            "nobody cancelled the admitted invocation"
        );
        assert!(
            Instant::now() < settles_by,
            "the released invocation never settled: {response:?}"
        );
        thread::sleep(Duration::from_millis(10));
    };
    let result = match recovered {
        V5ServerResponse::Invocation {
            outcome: V5InvocationResponse::Direct { receipt },
        } => {
            assert_eq!(
                receipt.receipt_key().invocation_id(),
                invocation.invocation_id()
            );
            assert_eq!(
                receipt.receipt_key().reserved_task_id(),
                invocation.reserved_task_id()
            );
            let ReceiptTerminalOutcome::Completed { result } = receipt.terminal() else {
                panic!("disconnected invocation did not complete: {receipt:?}");
            };
            *result.clone()
        }
        V5ServerResponse::Invocation {
            outcome: V5InvocationResponse::Task { snapshot },
        } => {
            assert_eq!(snapshot.task_id(), invocation.reserved_task_id());
            assert_eq!(snapshot.invocation_id(), invocation.invocation_id());
            let terminal = idle_task_terminal_over_tcp(&mut successor, &invocation);
            let V5DaemonTaskSnapshot::Completed { result, .. } = terminal else {
                panic!("disconnected invocation did not complete: {terminal:?}");
            };
            *result
        }
        other => panic!("exact repeat must recover the admitted invocation: {other:?}"),
    };
    assert!(result.ok);
    assert_eq!(result.data, Some(json!({"attempt": 1})));
    assert_eq!(service.prepares.load(Ordering::SeqCst), 1);
    assert_eq!(service.executions.load(Ordering::SeqCst), 1);
    release_promised_work_owner(successor);
    daemon.await_idle_exit();
    daemon.join();
    assert!(daemon.endpoint().is_none());
}

/// The client leaves before the response window closes, and the begun
/// inline attempt passes into its Task while nobody is connected. Every owner
/// set read from the client's departure through the handoff holds the
/// daemon by the attempt's cancellation registration, so the attempt never
/// stands without an owner; its Task then ends once and the daemon retires.
/// rule: INV.RUNTIME.DAEMON-ACTIVE-OWNERS
#[test]
fn daemon_idle_inline_attempt_handed_over_at_the_cutoff_keeps_an_owner_throughout() {
    let (_workspace, workspace) = catalog_workspace();
    let (entered, executing) = mpsc::channel();
    let release = Arc::new((Mutex::new(false), Condvar::new()));
    let service = Arc::new(IdleInvocationService {
        known_long: false,
        prepares: AtomicUsize::new(0),
        executions: AtomicUsize::new(0),
        entered,
        release: Arc::clone(&release),
    });
    let idle = Duration::from_millis(80);
    let budget = Duration::from_millis(600);
    // The invocation clock stands still until the test moves it past the
    // window, so the client leaves before the cutoff however slowly the
    // machine schedules the execution.
    let clock = Arc::new(InspectionClock {
        start: Instant::now(),
        elapsed_ms: AtomicU64::new(0),
    });
    let mut daemon =
        PromisedWorkIdleDaemon::with_service_and_clock(idle, service.clone(), Arc::clone(&clock));
    let cleanup = IdleInvocationRelease(release);
    let invocation =
        view_catalog_request_within(TaskId::new(), &workspace, budget.as_millis() as u64);
    let mut owner = daemon.owner();
    write_json_line(
        &mut owner.0,
        &serde_json::to_value(V5ClientRequest::SubmitInvocation {
            invocation: invocation.clone(),
        })
        .unwrap(),
    );
    executing.recv_timeout(TEST_STEP).unwrap();
    owner.0.shutdown(std::net::Shutdown::Both).unwrap();
    drop(owner);
    let disconnected = Instant::now();
    daemon.decisions_since(disconnected, |_| true);
    let past_cutoff = Instant::now();
    clock
        .elapsed_ms
        .store(budget.as_millis() as u64 + 1, Ordering::SeqCst);
    // After the handoff no client lease is left: the Task alone holds.
    assert_held_by(
        &daemon.held_without_clients_since(past_cutoff),
        |owners| owners.task_cancellations,
        "the handed-over Task, through its cancellation registration",
    );
    assert_eq!(daemon.endpoint(), Some(daemon.record.clone()));
    assert_held_by(
        &daemon.owners_since(disconnected),
        |owners| owners.task_cancellations,
        "the inline attempt and then its Task, through one cancellation registration",
    );
    let mut successor = daemon.owner();
    let V5ServerResponse::Task { snapshot: handed } = idle_request_over_tcp(
        &mut successor,
        V5ClientRequest::GetTask {
            task_id: invocation.reserved_task_id(),
        },
    ) else {
        panic!("the attempt must have passed into its reserved Task");
    };
    assert_eq!(handed.invocation_id(), invocation.invocation_id());
    assert!(!task_snapshot_is_terminal(&handed));
    cleanup.release();
    let terminal = idle_task_terminal_over_tcp(&mut successor, &invocation);
    assert!(
        matches!(&terminal, V5DaemonTaskSnapshot::Completed { result, .. }
        if result.ok && result.data == Some(json!({"attempt": 1}))),
        "{terminal:?}"
    );
    assert_eq!(service.prepares.load(Ordering::SeqCst), 1);
    assert_eq!(service.executions.load(Ordering::SeqCst), 1);
    release_promised_work_owner(successor);
    daemon.await_idle_exit();
    daemon.join();
    assert!(daemon.endpoint().is_none());
}

/// The response window closes while the call still waits for its
/// workspace actor, before the attempt begins. Its Task is promised without
/// an actor and the client leaves. With no workspace work retained, the Task
/// alone holds the daemon through its cancellation registration; once bound,
/// the same Task runs once and the daemon retires.
/// rule: INV.RUNTIME.DAEMON-ACTIVE-OWNERS
#[test]
fn daemon_idle_task_promised_during_admission_holds_without_an_actor_then_retires() {
    let (_workspace, workspace) = catalog_workspace();
    let (entered, _executing) = mpsc::channel();
    let service = Arc::new(IdleInvocationService {
        known_long: false,
        prepares: AtomicUsize::new(0),
        executions: AtomicUsize::new(0),
        entered,
        release: Arc::new((Mutex::new(true), Condvar::new())),
    });
    let idle = Duration::from_millis(80);
    let budget = Duration::from_millis(600);
    // The invocation clock stands still until the test closes the response
    // window, so the call is promised while it waits for its actor however
    // slowly the machine runs.
    let clock = Arc::new(InspectionClock {
        start: Instant::now(),
        elapsed_ms: AtomicU64::new(0),
    });
    let mut daemon =
        PromisedWorkIdleDaemon::with_service_and_clock(idle, service.clone(), Arc::clone(&clock));
    // The attempt waits inside its real actor bind.
    let admission =
        crate::infrastructure::daemon::server::admission_test_control::AdmissionPause::install(
            workspace.clone(),
        );
    let invocation =
        view_catalog_request_within(TaskId::new(), &workspace, budget.as_millis() as u64);
    let mut owner = daemon.owner();
    write_json_line(
        &mut owner.0,
        &serde_json::to_value(V5ClientRequest::SubmitInvocation {
            invocation: invocation.clone(),
        })
        .unwrap(),
    );
    admission.wait_until_entered();
    clock
        .elapsed_ms
        .store(budget.as_millis() as u64 + 1, Ordering::SeqCst);
    // The answer names the Task promised at the closed window. The clock
    // stands inside the write margin, so the answer is not lost.
    let frame = read_bounded_v5_probe_response_frame(&mut owner.1)
        .expect("the promise of the waiting call is answered");
    let V5ServerResponse::Invocation {
        outcome: V5InvocationResponse::Task { snapshot: answered },
    } = decode_v5_server_response(&frame).unwrap()
    else {
        panic!("the call waiting for its actor must be promised as its Task");
    };
    assert_eq!(answered.task_id(), invocation.reserved_task_id());
    owner.0.shutdown(std::net::Shutdown::Both).unwrap();
    drop(owner);
    let released = Instant::now();

    assert_held_by(
        &daemon.held_without_clients_since(released),
        |owners| owners.task_cancellations && !owners.retained_workspace_work,
        "the Task promised during admission, through its cancellation registration alone",
    );
    assert_eq!(daemon.endpoint(), Some(daemon.record.clone()));
    let mut successor = daemon.owner();
    let V5ServerResponse::Task { snapshot: promised } = idle_request_over_tcp(
        &mut successor,
        V5ClientRequest::GetTask {
            task_id: invocation.reserved_task_id(),
        },
    ) else {
        panic!("the call must have been promised as its reserved Task");
    };
    assert_eq!(promised.invocation_id(), invocation.invocation_id());
    assert!(
        matches!(promised, V5DaemonTaskSnapshot::Queued { .. }),
        "a Task promised before the attempt begins is queued: {promised:?}"
    );
    assert_eq!(service.prepares.load(Ordering::SeqCst), 0);
    admission.release();
    let terminal = idle_task_terminal_over_tcp(&mut successor, &invocation);
    assert!(
        matches!(&terminal, V5DaemonTaskSnapshot::Completed { result, .. }
        if result.ok && result.data == Some(json!({"attempt": 1}))),
        "{terminal:?}"
    );
    assert_eq!(service.prepares.load(Ordering::SeqCst), 1);
    assert_eq!(service.executions.load(Ordering::SeqCst), 1);
    release_promised_work_owner(successor);
    daemon.await_idle_exit();
    daemon.join();
    assert!(daemon.endpoint().is_none());
}

/// rule: INV.RUNTIME.DAEMON-ACTIVE-OWNERS
#[test]
fn daemon_idle_completed_index_lease_preserves_same_result_then_retires() {
    let workspace =
        crate::infrastructure::daemon::server::actor_capacity_tests::subsystem_picture_workspace(
            "", false,
        );
    let producers = Arc::new(AtomicUsize::new(0));
    let release = Arc::new((Mutex::new(false), Condvar::new()));
    let (entered, started) = mpsc::channel();
    let (exited, finished) = mpsc::channel();
    let (follower, joined) = mpsc::channel();
    let service = Arc::new(IdleIndexService {
        calls: AtomicUsize::new(0),
        keep_first_lease: true,
        producers: Arc::clone(&producers),
        entered,
        exited,
        release: Arc::clone(&release),
        follower,
    });
    let idle = Duration::from_millis(80);
    let mut daemon = PromisedWorkIdleDaemon::with_service(idle, service);
    let cleanup = IdleIndexProducerCleanup {
        release,
        exited: finished,
    };
    let mut owner = daemon.owner();
    let call = |owner: &mut (TcpStream, BufReader<TcpStream>)| {
        promised_work_call_over_tcp(
            owner,
            workspace.path(),
            V5ToolIdentity::View,
            json!({"at": "main:Subsystem.Sales"}),
        )
    };
    assert!(call(&mut owner).ok);
    started.recv_timeout(TEST_STEP).unwrap();
    let lease = joined.recv_timeout(TEST_STEP).unwrap();
    release_promised_work_owner(owner);
    cleanup.release();
    let unica_work_control::shared_work::SharedWorkSnapshot::Ready(original) =
        lease.wait_timeout(TEST_STEP)
    else {
        panic!("the producer must finish before the lease-only idle interval");
    };

    let released = Instant::now();
    assert_held_by(
        &daemon.held_without_clients_since(released),
        |owners| owners.retained_workspace_work && !owners.task_cancellations,
        "the needed lease, as retained workspace work",
    );
    assert_eq!(daemon.endpoint(), Some(daemon.record.clone()));
    let mut successor = daemon.owner();
    assert!(call(&mut successor).ok);
    let follower = joined.recv_timeout(TEST_STEP).unwrap();
    let unica_work_control::shared_work::SharedWorkSnapshot::Ready(reused) = follower.snapshot()
    else {
        panic!("successor must receive the completed original shared work");
    };
    assert!(Arc::ptr_eq(&original, &reused));
    assert_eq!(producers.load(Ordering::SeqCst), 1);
    release_promised_work_owner(successor);
    drop(follower);
    let released = Instant::now();
    assert_held_by(
        &daemon.held_without_clients_since(released),
        |owners| owners.retained_workspace_work && !owners.task_cancellations,
        "the original lease, as retained workspace work",
    );
    drop(lease);
    daemon.await_idle_exit();
    daemon.join();
    assert!(daemon.endpoint().is_none());
}
