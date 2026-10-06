//! Тесты рантайма v5. Вынесены из `runtime_v5.rs`: производственный файл
//! перерос восемь тысяч строк, а модуль тестов — половина его объёма.
//! Путь модуля прежний (`infrastructure::daemon::runtime_v5::tests`),
//! поэтому выражения отбора nextest не меняются.

use super::*;
use crate::domain::operation_deadline::OperationDeadline;

use crate::infrastructure::daemon::protocol_v5::V5DaemonTaskSnapshot;
use crate::infrastructure::daemon::v13_workspace_bootstrap::test_control::HealthInspectionPause;

#[test]
fn losing_daemon_does_not_rewrite_capacity_snapshot_before_receipt_authority() {
    use crate::infrastructure::capacity_observation::{start_background_writer, CapacityObserver};

    let root = tempfile::tempdir().expect("temporary competing daemon state root");
    let state_root = std::fs::canonicalize(root.path()).expect("physical daemon state root");
    let identity = CoreIdentity::production_v5();
    let state = DaemonStateDirectory::open(&state_root, &identity).expect("open daemon state");
    let observer = Arc::new(CapacityObserver::default());
    observer.record_find(123, 1, true);
    let writer = start_background_writer(&state, observer, Arc::new(|| true))
        .expect("seed the capacity snapshot");
    drop(writer);
    let snapshot = state
        .path()
        .join("capacity-observation")
        .join("snapshot-v1.json");
    let before = std::fs::read(&snapshot).expect("read seeded snapshot");
    let authority = state
        .acquire_receipt_authority(Duration::from_millis(30))
        .expect("hold the receipt authority against the contender");

    let failure = run_daemon(DaemonServerConfig::new(
        state_root,
        identity,
        Duration::from_millis(50),
    ))
    .expect_err("contender cannot own the same receipt authority");
    assert!(failure.contains("stable receipt authority"), "{failure}");
    assert_eq!(
        std::fs::read(&snapshot).expect("read snapshot after losing startup"),
        before,
        "a losing daemon must not refresh or overwrite the owner's observation"
    );
    drop(authority);
}

struct InspectionClock {
    start: Instant,
    elapsed_ms: AtomicU64,
    listener: Mutex<Option<thread::ThreadId>>,
    watchdog_read: mpsc::Sender<()>,
}

impl Clock for InspectionClock {
    fn now(&self) -> Instant {
        let elapsed_ms = self.elapsed_ms.load(Ordering::SeqCst);
        if elapsed_ms >= 9_000 && *self.listener.lock().unwrap() == Some(thread::current().id()) {
            let _ = self.watchdog_read.send(());
        }
        self.start + Duration::from_millis(elapsed_ms)
    }
}

#[derive(Default)]
struct InspectionHooks {
    fail_stopped: AtomicBool,
}

impl V5RuntimeHooks for InspectionHooks {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn forced_process_exit(&self, _grace: Option<Duration>) {
        self.fail_stopped.store(true, Ordering::SeqCst);
    }

    fn releases_authority_on_fail_stop(&self) -> bool {
        true
    }
}

#[derive(Default)]
struct SourceAdmissionProbe {
    prepares: AtomicUsize,
}

impl CanonicalInvocationService for SourceAdmissionProbe {
    fn prepare(
        &self,
        _invocation: &crate::infrastructure::daemon::server::ActorBoundInvocation,
    ) -> Result<ExecutionClass, Box<DomainResult>> {
        self.prepares.fetch_add(1, Ordering::SeqCst);
        panic!("root inspection must not prepare a source operation");
    }

    fn execute(
        &self,
        _invocation: &crate::infrastructure::daemon::server::ActorBoundExecution,
        _cancellation: CancellationToken,
    ) -> Result<DomainResult, InvocationFailure> {
        panic!("root inspection must not execute a source operation");
    }
}

struct InspectionDaemon {
    pause: HealthInspectionPause,
    stop: Arc<AtomicBool>,
    server: Option<thread::JoinHandle<Result<(), String>>>,
}

impl Drop for InspectionDaemon {
    fn drop(&mut self) {
        self.pause.release();
        self.stop.store(true, Ordering::SeqCst);
        if let Some(server) = self.server.take() {
            let _ = server.join();
        }
    }
}

fn root_inspection_survives_response_cutoff(tool: V5ToolIdentity) {
    let state = tempfile::tempdir().unwrap();
    let state_root = std::fs::canonicalize(state.path()).unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let workspace_root = std::fs::canonicalize(workspace.path()).unwrap();
    std::fs::create_dir(workspace_root.join("src")).unwrap();
    let project_bytes =
        b"format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n";
    let source_bytes = br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Store</Name></Properties><ChildObjects/></Configuration></MetaDataObject>"#;
    std::fs::write(workspace_root.join("v8project.yaml"), project_bytes).unwrap();
    std::fs::write(workspace_root.join("src/Configuration.xml"), source_bytes).unwrap();
    let independent_workspace = tempfile::tempdir().unwrap();
    let pause = HealthInspectionPause::install(workspace_root.clone());
    let (watchdog_tx, watchdog_rx) = mpsc::channel();
    let clock = Arc::new(InspectionClock {
        start: Instant::now(),
        elapsed_ms: AtomicU64::new(0),
        listener: Mutex::new(None),
        watchdog_read: watchdog_tx,
    });
    let service = Arc::new(SourceAdmissionProbe::default());
    let canonical_runtime = Arc::new(V5CanonicalInvocationRuntime::new(
        service.clone(),
        clock.clone(),
    ));
    let hooks = Arc::new(InspectionHooks::default());
    let identity = CoreIdentity::production_v5();
    let config = DaemonServerConfig::new(
        state_root.clone(),
        identity.clone(),
        Duration::from_secs(30),
    )
    .with_canonical_runtime_for_test(canonical_runtime)
    .with_runtime_hooks_for_test(hooks.clone());
    let stop = Arc::new(AtomicBool::new(false));
    let server_stop = stop.clone();
    let server_clock = clock.clone();
    let server = thread::spawn(move || {
        *server_clock.listener.lock().unwrap() = Some(thread::current().id());
        run_daemon_configured_until(
            config,
            |runtime| runtime,
            || server_stop.load(Ordering::SeqCst),
        )
    });
    let daemon = InspectionDaemon {
        pause,
        stop,
        server: Some(server),
    };
    wait_for_v5_record(&state_root, &identity);
    let owner = V5DaemonProcessOwner::connect_or_spawn(
        &state_root,
        identity,
        std::path::PathBuf::from("unused-existing-v5-endpoint"),
        Duration::from_secs(2),
    )
    .unwrap();
    let task_id = TaskId::new();
    let invocation_id = InvocationId::new();
    let invocation = V5InvocationRequest::new(
        invocation_id,
        task_id,
        tool,
        serde_json::Map::new(),
        workspace_root.to_string_lossy().into_owned(),
        7_000,
    )
    .unwrap();
    let submit = thread::spawn(move || {
        let mut owner = owner;
        let response = owner.submit_invocation(invocation);
        (owner, response)
    });
    daemon.pause.wait_until_entered();
    clock.elapsed_ms.store(7_000, Ordering::SeqCst);
    let (mut owner, submitted) = submit.join().unwrap();
    let V5ServerResponse::Invocation {
        outcome: V5InvocationResponse::Task { snapshot },
    } = submitted.expect("root inspection returns its task at the response cutoff")
    else {
        panic!("root inspection did not hand off to a task");
    };
    assert_eq!(snapshot.task_id(), task_id);
    assert_eq!(snapshot.invocation_id(), invocation_id);

    clock.elapsed_ms.store(9_000, Ordering::SeqCst);
    watchdog_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("listener evaluated the two-second grace boundary");
    let independent = owner
        .connect_peer_before(Instant::now() + Duration::from_secs(2))
        .map_err(|error| error.to_string())
        .and_then(|mut peer| {
            peer.submit_invocation(
                V5InvocationRequest::new(
                    InvocationId::new(),
                    TaskId::new(),
                    V5ToolIdentity::View,
                    serde_json::Map::new(),
                    independent_workspace.path().to_string_lossy().into_owned(),
                    7_000,
                )
                .unwrap(),
            )
        });
    daemon.pause.release();
    assert!(
        independent.is_ok(),
        "slow root {tool:?} inspection stopped an independent daemon read; fail_stop={}: {independent:?}",
        hooks.fail_stopped.load(Ordering::SeqCst),
    );
    let V5ServerResponse::Invocation {
        outcome: V5InvocationResponse::Direct { receipt },
    } = independent.unwrap()
    else {
        panic!("independent root read did not complete directly");
    };
    assert!(
        matches!(receipt.terminal(), ReceiptTerminalOutcome::Completed { result } if result.ok)
    );

    let settled = owner
        .wait_task(task_id, 7_000)
        .expect("same root task remains reachable");
    let V5ServerResponse::Task {
        snapshot:
            V5DaemonTaskSnapshot::Completed {
                task_id: completed_task_id,
                invocation_id: completed_invocation_id,
                result,
                ..
            },
    } = settled
    else {
        panic!("root task did not terminalize: {settled:?}");
    };
    assert_eq!(completed_task_id, task_id);
    assert_eq!(completed_invocation_id, invocation_id);
    assert!(
        result.ok,
        "expired health inspection still reports workspace facts: {result:?}"
    );
    let data = result.data.unwrap();
    if tool == V5ToolIdentity::Check {
        assert_eq!(data["readinessState"], "incomplete");
        assert_eq!(data["ready"], false);
    } else {
        assert_eq!(data["sourceSets"][0]["name"], "main");
    }
    assert_eq!(
        daemon.pause.entries(),
        1,
        "handoff never replays inspection"
    );
    assert_eq!(service.prepares.load(Ordering::SeqCst), 0);
    assert!(!hooks.fail_stopped.load(Ordering::SeqCst));
    assert_eq!(
        std::fs::read(workspace_root.join("v8project.yaml")).unwrap(),
        project_bytes
    );
    assert_eq!(
        std::fs::read(workspace_root.join("src/Configuration.xml")).unwrap(),
        source_bytes
    );
}

#[test]
fn root_view_keeps_the_same_task_and_daemon_past_the_admission_grace() {
    root_inspection_survives_response_cutoff(V5ToolIdentity::View);
}

#[test]
fn root_check_keeps_the_same_task_and_original_inspection_deadline() {
    root_inspection_survives_response_cutoff(V5ToolIdentity::Check);
}

#[test]
fn protected_mutation_preserves_success_and_failure_after_cancel_request() {
    let cancellation = CancellationToken::new();
    let protected = cancellation.protect_process_on_spawn();
    protected.spawn_with_gate(|| Ok(())).unwrap();
    cancellation.cancel();
    assert!(cancellation.protected_process_started());

    let succeeded = ReceiptTerminalOutcome::Completed {
        result: Box::new(DomainResult::success("provider confirmed database change")),
    };
    assert!(matches!(
        task_outcome_after_cancel(succeeded, true, true),
        ReceiptTerminalOutcome::Completed { .. }
    ));
    let failed = ReceiptTerminalOutcome::Failed {
        reason: V5SafeFailureReason::InvocationFailed,
    };
    assert!(matches!(
        task_outcome_after_cancel(failed, true, true),
        ReceiptTerminalOutcome::Failed { .. }
    ));
    let mut refusal = DomainResult::success("provider result could not be verified");
    refusal.ok = false;
    let refused = ReceiptTerminalOutcome::Completed {
        result: Box::new(refusal),
    };
    assert!(matches!(
        task_outcome_after_cancel(refused, true, true),
        ReceiptTerminalOutcome::Completed { .. }
    ));
    assert!(matches!(
        task_outcome_after_cancel(
            ReceiptTerminalOutcome::Completed {
                result: Box::new(DomainResult::success("no dispatch")),
            },
            true,
            false,
        ),
        ReceiptTerminalOutcome::Cancelled
    ));
}

#[test]
fn protected_mutation_does_not_arm_the_two_second_cancel_watchdog() {
    let root = tempfile::tempdir().unwrap();
    let state_root = std::fs::canonicalize(root.path()).unwrap();
    let identity = CoreIdentity::production_v5();
    let state = DaemonStateDirectory::open(&state_root, &identity).unwrap();
    let config = DaemonServerConfig::new(state_root, identity, Duration::from_millis(50));
    let runtime = V5ReceiptRuntime::open(&state, &config).unwrap();
    let task_id = TaskId::new();
    let record = V5StoredInvocationRecord {
        schema_version: crate::application::invocation_store_v5::V5StoredInvocationSchemaVersion,
        task_id,
        invocation_id: InvocationId::new(),
        receipt_key_digest: "44".repeat(32).parse().unwrap(),
        tool: V5ToolIdentity::Run,
        normalized_arguments_hash: normalized_arguments_hash(&serde_json::Map::new()),
        workspace_identity_hash: SafeIdentityHash::from_sha256(Sha256::digest(b"workspace").into()),
        created_at_epoch_ms: 1_000,
        updated_at_epoch_ms: 1_000,
        ttl_ms: 3_600_000,
        poll_interval_ms: 250,
        version: 2,
        cancel_requested: true,
        task: V5StoredTask::Working,
    };
    let (cancellation, _guard) = runtime.active_task_cancellations.register(task_id).unwrap();
    cancellation
        .protect_process_on_spawn()
        .spawn_with_gate(|| Ok(()))
        .unwrap();
    cancellation.cancel();
    runtime.arm_cancel_grace(&record);
    assert!(runtime
        .fail_stop_watchdogs
        .due(runtime.invocation_executor.now() + FAIL_STOP_GRACE + Duration::from_secs(1))
        .is_none());

    let unprotected = V5StoredInvocationRecord {
        task_id: TaskId::new(),
        receipt_key_digest: "55".repeat(32).parse().unwrap(),
        ..record
    };
    runtime.arm_cancel_grace(&unprotected);
    assert!(runtime
        .fail_stop_watchdogs
        .due(runtime.invocation_executor.now() + FAIL_STOP_GRACE + Duration::from_secs(1))
        .is_some());
}

#[test]
fn inline_protected_mutation_cancel_does_not_arm_fail_stop_watchdog() {
    let root = tempfile::tempdir().unwrap();
    let state_root = std::fs::canonicalize(root.path()).unwrap();
    let identity = CoreIdentity::production_v5();
    let state = DaemonStateDirectory::open(&state_root, &identity).unwrap();
    let config = DaemonServerConfig::new(state_root, identity.clone(), Duration::from_millis(50));
    let runtime = V5ReceiptRuntime::open(&state, &config).unwrap();
    let key = ReceiptKey::new(
        InvocationId::new(),
        TaskId::new(),
        RequestIdentity::new(
            identity.digest().clone(),
            V5ToolIdentity::Run,
            normalized_arguments_hash(&serde_json::Map::new()),
            request_scope_hash("workspace-a").unwrap(),
        ),
    );
    let deadline = Instant::now() + Duration::from_secs(2);
    let reserved = runtime
        .receipt_ledger
        .reserve(
            key.clone(),
            OriginalCutoffDescriptor::new(1_000, 7_000).unwrap(),
            deadline,
        )
        .unwrap()
        .into_reservation()
        .unwrap();
    let bound = runtime
        .receipt_ledger
        .bind_reserved_actor(
            key.clone(),
            reserved.record_version(),
            SafeIdentityHash::from_sha256(Sha256::digest(b"workspace-a").into()),
            deadline,
        )
        .unwrap();
    runtime
        .receipt_ledger
        .mark_reserved_begun(key.clone(), bound.record_version(), deadline)
        .unwrap();
    let (cancellation, _guard) = runtime
        .active_task_cancellations
        .register(key.reserved_task_id())
        .unwrap();
    cancellation
        .protect_process_on_spawn()
        .spawn_with_gate(|| Ok(()))
        .unwrap();

    runtime
        .cancel_invocation(key, 2_000, deadline)
        .expect("cancel started inline attempt");

    assert!(cancellation.is_cancelled());
    assert!(runtime
        .fail_stop_watchdogs
        .due(runtime.invocation_executor.now() + FAIL_STOP_GRACE + Duration::from_secs(1))
        .is_none());
}
use crate::application::invocation::normalized_arguments_hash;
use crate::application::operation_descriptors::{ExecutionClass, KnownLongReason};
use crate::application::receipt_ledger::{
    receipt_key_digest, request_scope_hash, CancelResolution, CommittedDirectPublication,
    OriginalCutoffDescriptor, ReceiptKey, ReceiptLedgerPort, ReceiptRecordHeader, ReceiptState,
    ReceiptTaskProjection, ReceiptTerminalOutcome, ReceiptVersion, RequestIdentity, ReserveOutcome,
    ReservedPhase, ReservedReceipt, V5CanonicalTerminal, V5ToolIdentity,
    MAX_RECEIPT_ENTITLEMENT_BYTES,
};
use crate::domain::invocation::{DomainResult, InvocationFailure};
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

struct CooperativeKnownLongService {
    entered: mpsc::Sender<()>,
}

impl CanonicalInvocationService for CooperativeKnownLongService {
    fn prepare(
        &self,
        _invocation: &super::super::server::ActorBoundInvocation,
    ) -> Result<ExecutionClass, Box<DomainResult>> {
        Ok(ExecutionClass::KnownLong(KnownLongReason::ExternalProcess))
    }

    fn execute(
        &self,
        _invocation: &super::super::server::ActorBoundExecution,
        cancellation: CancellationToken,
    ) -> Result<DomainResult, InvocationFailure> {
        self.entered.send(()).expect("report task execution");
        while !cancellation.is_cancelled() {
            thread::yield_now();
        }
        Err(InvocationFailure::new(
            "cancelled",
            "cooperative task observed cancellation",
        ))
    }
}

#[test]
fn cancel_task_signals_the_running_v5_canonical_execution() {
    let root = tempfile::tempdir().expect("temporary cancellation state root");
    let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
    let workspace = tempfile::tempdir().expect("temporary cancellation workspace");
    let source = workspace.path().join("src");
    std::fs::create_dir_all(&source).expect("create source root");
    std::fs::write(
        workspace.path().join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .expect("write workspace descriptor");
    std::fs::write(
        source.join("Configuration.xml"),
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Store</Name></Properties><ChildObjects/></Configuration></MetaDataObject>"#,
    )
    .expect("write configuration root");
    let workspace = std::fs::canonicalize(workspace.path()).expect("physical workspace");
    let identity = CoreIdentity::production_v5();
    let (entered_tx, entered_rx) = mpsc::channel();
    let config = DaemonServerConfig::new(
        state_root.clone(),
        identity.clone(),
        Duration::from_millis(80),
    )
    .with_invocation_service(Arc::new(CooperativeKnownLongService {
        entered: entered_tx,
    }));
    let server = thread::spawn(move || run_daemon(config));
    let _record = wait_for_v5_record(&state_root, &identity);
    let invocation = V5InvocationRequest::new(
        InvocationId::new(),
        TaskId::new(),
        V5ToolIdentity::View,
        serde_json::Map::from_iter([(
            "at".to_owned(),
            serde_json::Value::String("main:Catalog.Items".to_owned()),
        )]),
        workspace.to_string_lossy().into_owned(),
        7_000,
    )
    .expect("valid known-long invocation");
    let mut owner = V5DaemonProcessOwner::connect_or_spawn(
        &state_root,
        identity,
        std::path::PathBuf::from("unused-existing-v5-endpoint"),
        Duration::from_millis(300),
    )
    .expect("connect v5 owner");
    let submitted = owner
        .submit_invocation(invocation)
        .expect("submit known-long invocation");
    let task_id = match submitted {
        V5ServerResponse::Invocation {
            outcome:
                V5InvocationResponse::Task {
                    snapshot:
                        super::super::protocol_v5::V5DaemonTaskSnapshot::Working { task_id, .. },
                },
        } => task_id,
        other => panic!("known-long submission did not return Working: {other:?}"),
    };
    if entered_rx.recv_timeout(Duration::from_secs(10)).is_err() {
        let snapshot = owner.get_task(task_id).expect("inspect stalled Task");
        panic!("canonical execution did not enter: {snapshot:?}");
    }

    owner.cancel_task(task_id).expect("cancel running Task");
    let terminal = owner
        .wait_task(task_id, 7_000)
        .expect("wait for cancellation");
    assert!(matches!(
        terminal,
        V5ServerResponse::Task {
            snapshot: super::super::protocol_v5::V5DaemonTaskSnapshot::Cancelled { .. }
        }
    ));

    drop(owner);
    server
        .join()
        .expect("v5 cancellation daemon did not panic")
        .expect("v5 cancellation daemon exited cleanly");
}

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
        observed_deadline: mpsc::Sender<OperationDeadline>,
    },
}

struct FailingCancelPort {
    failure: CancelPortFailure,
}

impl ReceiptLedgerPort for FailingCancelPort {
    fn generation(&mut self, _deadline: OperationDeadline) -> Result<u64, ReceiptLedgerError> {
        Ok(0)
    }

    fn reserve(
        &mut self,
        _key: ReceiptKey,
        _original_cutoff: OriginalCutoffDescriptor,
        _deadline: OperationDeadline,
    ) -> Result<ReserveOutcome, ReceiptLedgerError> {
        Err(ReceiptLedgerError::StoreUnavailable)
    }

    fn request_cancel_or_reserve(
        &mut self,
        key: ReceiptKey,
        _cancel_reserved_at_epoch_ms: u64,
        deadline: OperationDeadline,
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
                    deadline
                        .remaining_at(Instant::now())
                        .expect("finite cancellation fixture")
                        + Duration::from_millis(10),
                );
                Err(ReceiptLedgerError::StoreUnavailable)
            }
        }
    }

    fn publish_direct_terminal(
        &mut self,
        _key: &ReceiptKey,
        _expected_version: ReceiptVersion,
        _terminal_epoch_ms: u64,
        _terminal: V5CanonicalTerminal,
        _deadline: OperationDeadline,
    ) -> Result<CommittedDirectPublication, ReceiptLedgerError> {
        Err(ReceiptLedgerError::StoreUnavailable)
    }

    fn recover(
        &mut self,
        _key: &ReceiptKey,
        _deadline: OperationDeadline,
    ) -> Result<ReceiptState, ReceiptLedgerError> {
        Err(ReceiptLedgerError::StoreUnavailable)
    }
}

struct SlowReservePort {
    delay: Duration,
}

impl ReceiptLedgerPort for SlowReservePort {
    fn generation(&mut self, _deadline: OperationDeadline) -> Result<u64, ReceiptLedgerError> {
        Ok(0)
    }

    fn reserve(
        &mut self,
        key: ReceiptKey,
        original_cutoff: OriginalCutoffDescriptor,
        _deadline: OperationDeadline,
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
        _deadline: OperationDeadline,
    ) -> Result<CancelResolution, ReceiptLedgerError> {
        Err(ReceiptLedgerError::StoreUnavailable)
    }

    fn publish_direct_terminal(
        &mut self,
        _key: &ReceiptKey,
        _expected_version: ReceiptVersion,
        _terminal_epoch_ms: u64,
        _terminal: V5CanonicalTerminal,
        _deadline: OperationDeadline,
    ) -> Result<CommittedDirectPublication, ReceiptLedgerError> {
        Err(ReceiptLedgerError::StoreUnavailable)
    }

    fn recover(
        &mut self,
        _key: &ReceiptKey,
        _deadline: OperationDeadline,
    ) -> Result<ReceiptState, ReceiptLedgerError> {
        Err(ReceiptLedgerError::StoreUnavailable)
    }
}

#[test]
fn seven_second_submit_budget_is_not_truncated_by_transport_timeouts() {
    let root = tempfile::tempdir().expect("temporary long-submit state root");
    let state_root = std::fs::canonicalize(root.path()).expect("physical long-submit state root");
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
    let mut owner = V5DaemonProcessOwner::connect_or_spawn(
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
    let mut owner = V5DaemonProcessOwner::connect_or_spawn(
        &state_root,
        identity.clone(),
        std::path::PathBuf::from("unused-existing-v5-endpoint"),
        Duration::from_millis(300),
    )
    .expect("connect fail-stop owner");
    let response = owner.cancel_invocation(key);
    drop(owner);
    let server_result = server.join().expect("join fail-stop runtime");
    let state =
        DaemonStateDirectory::open(&state_root, &identity).expect("reopen fail-stop daemon state");
    let retained = state
        .read_v5_endpoint_record()
        .expect("read retained fail-stop endpoint");
    let competing_authority = state.acquire_receipt_authority(Duration::from_millis(30));

    assert_eq!(
        response,
        Ok(V5ServerResponse::Error {
            code: V5DaemonErrorCode::StoreCommitUncertain,
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
    let mut owner = V5DaemonProcessOwner::connect_or_spawn(
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
        .expect("observe the live cancel operation deadline")
        .as_instant()
        .expect("existing runtime deadline is finite");
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
            code: V5DaemonErrorCode::StoreCommitUncertain,
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
    let state_root = std::fs::canonicalize(root.path()).expect("physical store-failure state root");
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
    let mut owner = V5DaemonProcessOwner::connect_or_spawn(
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
    // Порядок полей проверяется в производственном файле — тесты живут рядом с ним.
    let source = include_str!("../runtime_v5.rs");
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
    write_json_line(&mut successor, &json!({"kind": "release"}));
    let successor_released = read_bounded_v5_probe_response_frame(&mut successor_reader)
        .expect("release successor session");
    assert_eq!(
        decode_v5_server_response(&successor_released),
        Ok(V5ServerResponse::Released)
    );
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
    service: Arc<SourceAdmissionProbe>,
    exchange: impl FnOnce(&mut V5DaemonProcessOwner) -> Result<V5ServerResponse, String>,
) -> V5ServerResponse {
    let config = DaemonServerConfig::new(
        state_root.to_path_buf(),
        identity.clone(),
        Duration::from_millis(80),
    )
    .with_invocation_service(service);
    let server = thread::spawn(move || {
        run_daemon_configured(config, move |mut runtime| {
            runtime.epoch_clock = clock;
            runtime
        })
    });
    let _record = wait_for_v5_record(state_root, identity);
    let mut owner = V5DaemonProcessOwner::connect_or_spawn(
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

fn assert_staged_handoff_terminal_transfer(phase: AttemptPhase, outcome: ReceiptTerminalOutcome) {
    let root = tempfile::tempdir().unwrap();
    let state_root = std::fs::canonicalize(root.path()).unwrap();
    let identity = CoreIdentity::production_v5();
    let state = DaemonStateDirectory::open(&state_root, &identity).unwrap();
    let service = Arc::new(SourceAdmissionProbe::default());
    let clock = Arc::new(ManualEpochClock::new(1_000));
    let config = DaemonServerConfig::new(state_root, identity.clone(), Duration::from_millis(50))
        .with_v5_epoch_clock_for_test(clock.clone())
        .with_invocation_service(service.clone());
    let runtime = V5ReceiptRuntime::open(&state, &config).unwrap();
    let key = ReceiptKey::new(
        InvocationId::new(),
        TaskId::new(),
        RequestIdentity::new(
            identity.digest().clone(),
            V5ToolIdentity::View,
            normalized_arguments_hash(&serde_json::Map::new()),
            request_scope_hash("staged-transfer-workspace").unwrap(),
        ),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    let reserved = runtime
        .receipt_ledger
        .reserve(
            key.clone(),
            OriginalCutoffDescriptor::new(1_000, 7_000).unwrap(),
            deadline,
        )
        .unwrap()
        .into_reservation()
        .unwrap();
    let bound = runtime
        .receipt_ledger
        .bind_reserved_actor(
            key.clone(),
            reserved.record_version(),
            SafeIdentityHash::from_sha256(Sha256::digest(b"staged-transfer-workspace").into()),
            deadline,
        )
        .unwrap();
    let version = match phase {
        AttemptPhase::NotBegun => bound.record_version(),
        AttemptPhase::Begun => runtime
            .receipt_ledger
            .mark_reserved_begun(key.clone(), bound.record_version(), deadline)
            .unwrap()
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
        .unwrap();
    let handoff = if matches!(outcome, ReceiptTerminalOutcome::Cancelled) {
        let cancelled = runtime
            .receipt_ledger
            .request_task_cancel(
                key.clone(),
                TaskCancellationReceipt::HandoffActorBound(handoff),
                deadline,
            )
            .unwrap();
        let TaskCancellationReceipt::HandoffActorBound(handoff) = cancelled else {
            panic!("cancellation must retain the same actor-bound handoff");
        };
        handoff
    } else {
        handoff
    };
    let terminal = canonical_v5_terminal(&outcome).unwrap();
    let certificate = canonical_staged_transfer_certificate(
        handoff.key(),
        handoff.key_digest(),
        handoff.link(),
        1_100,
        &terminal,
    )
    .unwrap();
    let staged = runtime
        .receipt_ledger
        .stage_bound_task_handoff_terminal(
            key.clone(),
            handoff.record_version(),
            1_100,
            terminal.clone(),
            certificate,
            deadline,
        )
        .unwrap();
    clock.set(if matches!(outcome, ReceiptTerminalOutcome::Cancelled) {
        1_100
    } else {
        2_000
    });
    let publication =
        runtime.publish_staged_handoff_terminal_reply(staged, terminal.clone(), 1_100, deadline);
    let stored = runtime
        .task_projection
        .task_store
        .get(
            key.reserved_task_id(),
            crate::domain::code_intelligence::ProviderDeadline::new(deadline),
        )
        .unwrap();
    assert!(
        publication.is_ok(),
        "saved {phase:?} {outcome:?} transfer failed: {:?}; actual TaskStore state {:?}",
        publication.err(),
        stored.task
    );
    assert_eq!(stored.updated_at_epoch_ms, 1_100);
    assert_eq!(stored.task.terminal_digest(), Some(terminal.digest()));
    assert!(matches!(
        runtime.receipt_ledger.recover(key.clone(), deadline),
        Err(ReceiptLedgerError::ReceiptNotFound)
    ));
    let snapshot = runtime
        .resolve_task(key.reserved_task_id(), deadline)
        .unwrap();
    assert_eq!(service.prepares.load(Ordering::SeqCst), 0);
    drop(runtime);
    clock.set(3_000);
    let reopened = V5ReceiptRuntime::open(&state, &config).unwrap();
    assert_eq!(
        reopened
            .resolve_task(key.reserved_task_id(), deadline)
            .unwrap(),
        snapshot
    );
    assert_eq!(
        reopened
            .task_projection
            .task_store
            .get(
                key.reserved_task_id(),
                crate::domain::code_intelligence::ProviderDeadline::new(deadline),
            )
            .unwrap(),
        stored
    );
    assert_eq!(service.prepares.load(Ordering::SeqCst), 0);
}

#[test]
fn queued_staged_completed_transfer_preserves_exact_winner_and_reopens() {
    assert_staged_handoff_terminal_transfer(
        AttemptPhase::NotBegun,
        ReceiptTerminalOutcome::Completed {
            result: Box::new(DomainResult::success("saved winner")),
        },
    );
}

#[test]
fn queued_staged_failed_transfer_preserves_exact_winner_and_reopens() {
    assert_staged_handoff_terminal_transfer(
        AttemptPhase::NotBegun,
        ReceiptTerminalOutcome::Failed {
            reason: V5SafeFailureReason::InvocationFailed,
        },
    );
}

#[test]
fn begun_staged_completed_transfer_preserves_exact_winner_and_reopens() {
    assert_staged_handoff_terminal_transfer(
        AttemptPhase::Begun,
        ReceiptTerminalOutcome::Completed {
            result: Box::new(DomainResult::success("saved winner")),
        },
    );
}

#[test]
fn queued_staged_cancelled_transfer_preserves_exact_winner_and_reopens() {
    assert_staged_handoff_terminal_transfer(
        AttemptPhase::NotBegun,
        ReceiptTerminalOutcome::Cancelled,
    );
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
        .materialize_bound_handoff(&handoff, 1_009, deadline, runtime.hooks.as_ref())
        .unwrap_or_else(|failure| panic!("materialize startup TaskBound: {}", failure.error));
    let task_bound = runtime
        .receipt_ledger
        .complete_bound_task_handoff(key.clone(), handoff.record_version(), task_bound, deadline)
        .expect("complete startup TaskBound ownership");
    (key, record, task_bound)
}

/// Two retirement passes meet at the snapshot: the hook holds each one
/// until the other arrives.
struct RetirementSnapshotBarrier {
    barrier: Arc<std::sync::Barrier>,
}

impl V5RuntimeHooks for RetirementSnapshotBarrier {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn pause(&self, point: V5PausePoint, _deadline: Instant) -> Result<(), ReceiptLedgerError> {
        if point == V5PausePoint::BeforeRetirementSnapshot {
            self.barrier.wait();
        }
        Ok(())
    }
}

#[test]
fn concurrent_terminal_retirement_is_idempotent() {
    let root = tempfile::tempdir().expect("temporary retirement-race state root");
    let state_root =
        std::fs::canonicalize(root.path()).expect("physical retirement-race state root");
    let identity = CoreIdentity::production_v5();
    let clock = Arc::new(ManualEpochClock::new(1_000));
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let config = DaemonServerConfig::new(
        state_root.clone(),
        identity.clone(),
        Duration::from_millis(50),
    )
    .with_v5_epoch_clock_for_test(clock.clone())
    .with_runtime_hooks_for_test(Arc::new(RetirementSnapshotBarrier {
        barrier: Arc::clone(&barrier),
    }));
    let state = DaemonStateDirectory::open(&state_root, &identity)
        .expect("open retirement-race daemon state");
    let runtime = V5ReceiptRuntime::open(&state, &config).expect("open retirement-race runtime");
    let (_key, queued, task_bound) =
        materialize_startup_task_bound(&runtime, &identity, AttemptPhase::NotBegun);
    clock.set(2_000);
    let provider_deadline = crate::domain::code_intelligence::ProviderDeadline::new(
        Instant::now() + Duration::from_secs(2),
    );
    let terminal = runtime
        .task_projection
        .task_store
        .terminalize_recovered_exact(
            &queued.identity(),
            queued.version,
            RecoveryTerminalReason::InterruptedBeforeExecution,
            provider_deadline,
        )
        .expect("terminalize retirement-race Task");
    let V5StoredTask::Failed {
        terminal_epoch_ms,
        terminal_digest,
        ..
    } = &terminal.task
    else {
        panic!("retirement-race Task must be terminal")
    };
    runtime
        .task_projection
        .lifecycle_links
        .publish_task_terminal_bound(
            &task_bound,
            receipt_task_projection_from_store(&terminal)
                .unwrap_or_else(|failure| panic!("project terminal Task: {}", failure.error)),
            terminal.version,
            ClosedTerminalStatus::Failed,
            terminal_digest.clone(),
            *terminal_epoch_ms,
            provider_deadline,
        )
        .expect("publish retirement-race terminal link");
    clock.set(4_000_000);

    let runtime = Arc::new(runtime);
    let workers = (0..2)
        .map(|_| {
            let runtime = Arc::clone(&runtime);
            thread::spawn(move || {
                runtime.task_projection.retire_expired_terminal_tasks(
                    Instant::now() + Duration::from_secs(2),
                    runtime.hooks.as_ref(),
                )
            })
        })
        .collect::<Vec<_>>();
    let outcomes = workers
        .into_iter()
        .map(|worker| worker.join().expect("join retirement worker"))
        .collect::<Vec<_>>();

    assert!(
        outcomes.iter().all(Result::is_ok),
        "ordinary concurrent retirement produced a fail-stop failure"
    );
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
    let (key, queued, task_bound) =
        materialize_startup_task_bound(&runtime, &identity, AttemptPhase::NotBegun);
    assert_eq!(queued.task, V5StoredTask::Queued);
    runtime
        .task_projection
        .lifecycle_links
        .mark_task_bound_begun(
            &task_bound,
            queued.version,
            queued.updated_at_epoch_ms,
            crate::domain::code_intelligence::ProviderDeadline::new(
                Instant::now() + Duration::from_secs(2),
            ),
        )
        .expect("mark lifecycle link begun without advancing the queued Task fixture");
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
    let state =
        DaemonStateDirectory::open(&state_root, &identity).expect("open orphan Task daemon state");
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
    assert!(
        error.contains("TaskStore Task has no exact lifecycle link"),
        "unexpected startup failure: {error}"
    );

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
            receipt_task_projection_from_store(&terminal)
                .unwrap_or_else(|failure| panic!("project exact terminal Task: {}", failure.error)),
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
fn cancel_reserved_survives_former_ttl_and_restart_before_late_submit_without_callback() {
    let root = tempfile::tempdir().expect("temporary restart-stable receipt root");
    let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
    let identity = CoreIdentity::production_v5();
    let clock = Arc::new(ManualEpochClock::new(1_000));
    let service = Arc::new(SourceAdmissionProbe::default());
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
    // Every exchange starts and fully stops a fresh daemon over the same durable ledger.
    let initial = exchange_once_with_epoch(
        &state_root,
        &identity,
        Arc::clone(&clock),
        Arc::clone(&service),
        |owner| owner.cancel_invocation(key.clone()),
    );
    assert!(matches!(
        initial,
        V5ServerResponse::Invocation {
            outcome: V5InvocationResponse::ReceiptPending {
                accepted_epoch_ms: 1_000,
                phase: V5InvocationPhase::CancelReserved,
                cancel_requested: true,
                ..
            }
        }
    ));
    let state = DaemonStateDirectory::open(&state_root, &identity).expect("open receipt state");
    let generation_path = state.path().join("receipts").join("generation");
    let original_generation = std::fs::read(&generation_path).expect("read initial generation");

    // Both observations are later than the former absolute expiry (1_000 + 7_125).
    clock.set(100_000);
    let duplicate = exchange_once_with_epoch(
        &state_root,
        &identity,
        Arc::clone(&clock),
        Arc::clone(&service),
        |owner| owner.cancel_invocation(key.clone()),
    );
    assert_eq!(
        duplicate, initial,
        "duplicate cancellation changed its original timestamp"
    );
    assert_eq!(
        std::fs::read(&generation_path).unwrap(),
        original_generation
    );
    clock.set(200_000);
    let recovered = exchange_once_with_epoch(
        &state_root,
        &identity,
        Arc::clone(&clock),
        Arc::clone(&service),
        |owner| owner.recover_invocation_receipt(key.clone()),
    );
    assert_eq!(recovered, initial, "restart forgot the early cancellation");
    assert_eq!(
        std::fs::read(&generation_path).unwrap(),
        original_generation
    );

    let invocation = V5InvocationRequest::new(
        invocation_id,
        reserved_task_id,
        V5ToolIdentity::View,
        arguments,
        "workspace-a".to_owned(),
        7_000,
    )
    .expect("strict late invocation");
    let submitted = exchange_once_with_epoch(
        &state_root,
        &identity,
        clock,
        Arc::clone(&service),
        |owner| owner.submit_invocation(invocation),
    );
    assert!(matches!(
        submitted,
        V5ServerResponse::Invocation {
            outcome: V5InvocationResponse::Direct { ref receipt }
        } if matches!(receipt.terminal(), ReceiptTerminalOutcome::Cancelled)
            && receipt.receipt_key() == &key
    ));
    assert_eq!(
        service.prepares.load(Ordering::SeqCst),
        0,
        "late pre-cancelled submission must not invoke the provider callback"
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

    let mut cancel_owner = V5DaemonProcessOwner::connect_or_spawn(
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

    let mut submit_owner = V5DaemonProcessOwner::connect_or_spawn(
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

    let mut recover_owner = V5DaemonProcessOwner::connect_or_spawn(
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
    drop(cancel_owner);
    drop(submit_owner);
    drop(recover_owner);

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

fn connect_v5_owner(
    record: &V5EndpointRecord,
    identity: &CoreIdentity,
    owner_lease: &str,
) -> (TcpStream, BufReader<TcpStream>) {
    let mut stream = TcpStream::connect(record.loopback_addr().expect("v5 loopback address"))
        .expect("connect v5 daemon owner");
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("bound v5 owner response read");
    let mut reader = BufReader::new(stream.try_clone().expect("clone v5 owner stream"));
    write_json_line(
        &mut stream,
        &json!({
            "kind": "hello",
            "protocolVersion": 5,
            "token": record.token(),
            "coreIdentity": identity.as_str(),
            "ownerLease": owner_lease
        }),
    );
    let ready = read_bounded_v5_probe_response_frame(&mut reader).expect("read v5 owner ready");
    assert!(matches!(
        decode_v5_server_response(&ready),
        Ok(V5ServerResponse::Ready { .. })
    ));
    (stream, reader)
}

#[test]
fn v5_owner_session_accepts_ping_then_release() {
    let root = tempfile::tempdir().expect("temporary session state root");
    let state_root = std::fs::canonicalize(root.path()).expect("physical session state root");
    let identity = CoreIdentity::production_v5();
    let config = DaemonServerConfig::new(
        state_root.clone(),
        identity.clone(),
        Duration::from_millis(100),
    );
    let server = thread::spawn(move || run_daemon(config));
    let record = wait_for_v5_record(&state_root, &identity);
    let (mut stream, mut reader) =
        connect_v5_owner(&record, &identity, "44444444-4444-4444-8444-444444444444");

    write_json_line(&mut stream, &json!({"kind": "ping"}));
    let pong = read_bounded_v5_probe_response_frame(&mut reader).expect("read v5 pong");
    assert_eq!(decode_v5_server_response(&pong), Ok(V5ServerResponse::Pong));

    write_json_line(&mut stream, &json!({"kind": "release"}));
    let released =
        read_bounded_v5_probe_response_frame(&mut reader).expect("read v5 release response");
    assert_eq!(
        decode_v5_server_response(&released),
        Ok(V5ServerResponse::Released)
    );
    drop(stream);

    server
        .join()
        .expect("join v5 session runtime")
        .expect("v5 session runtime");
}

struct HandlerSpawnFailureHooks {
    fail_next: AtomicBool,
    failed: mpsc::Sender<()>,
}

impl V5RuntimeHooks for HandlerSpawnFailureHooks {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn spawn_connection_handler(
        &self,
        handler: Box<dyn FnOnce() + Send + 'static>,
    ) -> io::Result<thread::JoinHandle<()>> {
        if self.fail_next.swap(false, Ordering::SeqCst) {
            drop(handler);
            self.failed
                .send(())
                .expect("report actual handler spawn failure");
            return Err(io::Error::from(io::ErrorKind::WouldBlock));
        }
        thread::Builder::new()
            .name("unica-daemon-connection".into())
            .spawn(handler)
    }
}

struct HandlerSpawnDaemon {
    stop: Arc<AtomicBool>,
    streams: Vec<TcpStream>,
    server: Option<thread::JoinHandle<Result<(), String>>>,
}

impl Drop for HandlerSpawnDaemon {
    fn drop(&mut self) {
        for stream in &self.streams {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
        self.stop.store(true, Ordering::SeqCst);
        if let Some(server) = self.server.take() {
            let _ = server.join();
        }
    }
}

#[test]
fn handler_spawn_failure_preserves_live_owner_listener_and_next_connection() {
    use std::io::Read;

    let root = tempfile::tempdir().expect("temporary handler-spawn state root");
    let state_root = std::fs::canonicalize(root.path()).unwrap();
    let identity = CoreIdentity::production_v5();
    let (failed, failed_wait) = mpsc::channel();
    let hooks = Arc::new(HandlerSpawnFailureHooks {
        fail_next: AtomicBool::new(false),
        failed,
    });
    let config = DaemonServerConfig::new(
        state_root.clone(),
        identity.clone(),
        Duration::from_millis(80),
    )
    .with_runtime_hooks_for_test(hooks.clone());
    let stop = Arc::new(AtomicBool::new(false));
    let server_stop = Arc::clone(&stop);
    let server = thread::spawn(move || {
        run_daemon_configured_until(
            config,
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
    let (mut owner, mut owner_reader) =
        connect_v5_owner(&record, &identity, &uuid::Uuid::new_v4().to_string());
    daemon.streams.push(owner.try_clone().unwrap());

    hooks.fail_next.store(true, Ordering::SeqCst);
    let mut rejected = TcpStream::connect(record.loopback_addr().unwrap()).unwrap();
    rejected
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    daemon.streams.push(rejected.try_clone().unwrap());
    failed_wait
        .recv_timeout(Duration::from_secs(2))
        .expect("the accepted connection reached the failing spawn");
    let closed = rejected.read(&mut [0_u8]);
    assert!(
        matches!(closed, Ok(0))
            || matches!(closed, Err(ref error) if matches!(error.kind(), io::ErrorKind::ConnectionReset | io::ErrorKind::ConnectionAborted)),
        "failed spawn left its transport open: {closed:?}"
    );

    write_json_line(&mut owner, &json!({"kind": "ping"}));
    let pong = read_bounded_v5_probe_response_frame(&mut owner_reader)
        .expect("existing owner remains responsive");
    assert_eq!(decode_v5_server_response(&pong), Ok(V5ServerResponse::Pong));
    assert!(
        !daemon.server.as_ref().unwrap().is_finished(),
        "handler spawn failure terminated the listener"
    );

    let (mut successor, mut successor_reader) =
        connect_v5_owner(&record, &identity, &uuid::Uuid::new_v4().to_string());
    daemon.streams.push(successor.try_clone().unwrap());
    write_json_line(&mut successor, &json!({"kind": "ping"}));
    let pong = read_bounded_v5_probe_response_frame(&mut successor_reader)
        .expect("next accepted owner remains responsive");
    assert_eq!(decode_v5_server_response(&pong), Ok(V5ServerResponse::Pong));
    let state = DaemonStateDirectory::open(&state_root, &identity).unwrap();
    let current = state
        .read_v5_endpoint_record()
        .unwrap()
        .expect("same listener remains published");
    assert_eq!(current.instance_id(), record.instance_id());
    assert_eq!(
        current.loopback_addr().unwrap(),
        record.loopback_addr().unwrap()
    );

    for (stream, reader) in [
        (&mut successor, &mut successor_reader),
        (&mut owner, &mut owner_reader),
    ] {
        write_json_line(stream, &json!({"kind": "release"}));
        let released =
            read_bounded_v5_probe_response_frame(reader).expect("release admitted owner");
        assert_eq!(
            decode_v5_server_response(&released),
            Ok(V5ServerResponse::Released)
        );
    }
    let stopped_before = Instant::now() + Duration::from_secs(5);
    while !daemon.server.as_ref().unwrap().is_finished() {
        assert!(
            Instant::now() < stopped_before,
            "failed spawn retained admission ownership after both owners released"
        );
        thread::sleep(Duration::from_millis(5));
    }
    daemon
        .server
        .take()
        .unwrap()
        .join()
        .expect("listener did not panic")
        .expect("listener shut down after releasing both owners");
    assert!(state.read_v5_endpoint_record().unwrap().is_none());
}

#[test]
fn seven_second_wait_task_keeps_its_requested_operation_window() {
    let task_id = TaskId::new();
    let frame = serde_json::to_vec(&json!({
        "kind": "wait_task",
        "taskId": task_id.to_string(),
        "waitMs": 7_000
    }))
    .expect("serialize v5 wait request");
    let decoded = decode_v5_request_frame(frame).expect("decode v5 wait request");
    let received_at = Instant::now();

    let deadlines = v5_request_deadlines(&decoded, received_at).expect("derive v5 wait deadlines");

    assert_eq!(
        deadlines.operation.duration_since(received_at),
        Duration::from_secs(9)
    );
}

#[test]
fn v5_duplicate_live_owner_lease_is_rejected() {
    let root = tempfile::tempdir().expect("temporary duplicate-lease state root");
    let state_root =
        std::fs::canonicalize(root.path()).expect("physical duplicate-lease state root");
    let identity = CoreIdentity::production_v5();
    let config = DaemonServerConfig::new(
        state_root.clone(),
        identity.clone(),
        Duration::from_millis(100),
    );
    let server = thread::spawn(move || run_daemon(config));
    let record = wait_for_v5_record(&state_root, &identity);
    let lease = "55555555-5555-4555-8555-555555555555";
    let (mut first, mut first_reader) = connect_v5_owner(&record, &identity, lease);

    let mut duplicate = TcpStream::connect(record.loopback_addr().expect("v5 loopback address"))
        .expect("connect duplicate v5 owner");
    duplicate
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("bound duplicate response read");
    let mut duplicate_reader =
        BufReader::new(duplicate.try_clone().expect("clone duplicate stream"));
    write_json_line(
        &mut duplicate,
        &json!({
            "kind": "hello",
            "protocolVersion": 5,
            "token": record.token(),
            "coreIdentity": identity.as_str(),
            "ownerLease": lease
        }),
    );
    let response = read_bounded_v5_probe_response_frame(&mut duplicate_reader)
        .expect("read duplicate owner rejection");
    assert_eq!(
        decode_v5_server_response(&response),
        Ok(V5ServerResponse::Error {
            code: V5DaemonErrorCode::DuplicateLease,
        })
    );

    write_json_line(&mut first, &json!({"kind": "release"}));
    let released =
        read_bounded_v5_probe_response_frame(&mut first_reader).expect("release original v5 owner");
    assert_eq!(
        decode_v5_server_response(&released),
        Ok(V5ServerResponse::Released)
    );
    drop(first);
    drop(duplicate);

    server
        .join()
        .expect("join duplicate-lease runtime")
        .expect("duplicate-lease runtime");
}

struct HeldOwnerHandshakes {
    started: mpsc::Sender<u16>,
    entered: AtomicUsize,
    released: Mutex<bool>,
    changed: Condvar,
    hold_count: usize,
}

impl HeldOwnerHandshakes {
    fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.changed.notify_all();
    }
}

impl V5RuntimeHooks for HeldOwnerHandshakes {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn before_owner_handshake(&self, peer_port: u16) {
        if self.entered.fetch_add(1, Ordering::SeqCst) >= self.hold_count {
            return;
        }
        self.started.send(peer_port).unwrap();
        let mut released = self.released.lock().unwrap();
        while !*released {
            released = self.changed.wait(released).unwrap();
        }
    }
}

struct AdmissionDaemon {
    stop: Arc<AtomicBool>,
    hooks: Arc<HeldOwnerHandshakes>,
    sockets: Vec<TcpStream>,
    server: Option<thread::JoinHandle<Result<(), String>>>,
}

impl Drop for AdmissionDaemon {
    fn drop(&mut self) {
        self.hooks.release();
        for socket in &self.sockets {
            let _ = socket.shutdown(std::net::Shutdown::Both);
        }
        self.stop.store(true, Ordering::SeqCst);
        if let Some(server) = self.server.take() {
            let _ = server.join();
        }
    }
}

fn admitted_owner_cancel_and_recover(
    record: &V5EndpointRecord,
    identity: &CoreIdentity,
    daemon: &mut AdmissionDaemon,
) -> Result<(V5ServerResponse, V5ServerResponse, ReceiptKey), String> {
    let mut stream =
        TcpStream::connect(record.loopback_addr().unwrap()).map_err(|error| error.to_string())?;
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|error| error.to_string())?;
    daemon
        .sockets
        .push(stream.try_clone().map_err(|error| error.to_string())?);
    let mut reader = BufReader::new(stream.try_clone().map_err(|error| error.to_string())?);
    write_json_line(
        &mut stream,
        &json!({
            "kind": "hello", "protocolVersion": 5, "token": record.token(),
            "coreIdentity": identity.as_str(), "ownerLease": Uuid::new_v4().to_string()
        }),
    );
    let ready =
        read_bounded_v5_probe_response_frame(&mut reader).map_err(|error| error.to_string())?;
    let ready = decode_v5_server_response(&ready).map_err(|error| error.to_string())?;
    if !matches!(ready, V5ServerResponse::Ready { .. }) {
        return Err(format!(
            "real cancellation owner handshake was refused: {ready:?}"
        ));
    }
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
    write_json_line(
        &mut stream,
        &serde_json::to_value(V5ClientRequest::CancelInvocation {
            receipt_key: key.clone(),
        })
        .unwrap(),
    );
    let cancel =
        read_bounded_v5_probe_response_frame(&mut reader).map_err(|error| error.to_string())?;
    let cancel = decode_v5_server_response(&cancel).map_err(|error| error.to_string())?;
    write_json_line(
        &mut stream,
        &serde_json::to_value(V5ClientRequest::RecoverInvocationReceipt {
            receipt_key: key.clone(),
        })
        .unwrap(),
    );
    let recovered =
        read_bounded_v5_probe_response_frame(&mut reader).map_err(|error| error.to_string())?;
    let recovered = decode_v5_server_response(&recovered).map_err(|error| error.to_string())?;
    Ok((cancel, recovered, key))
}

fn cancellation_owner_crosses_admission_count(held_handshakes: usize, held_owners: usize) {
    let root = tempfile::tempdir().unwrap();
    let state_root = std::fs::canonicalize(root.path()).unwrap();
    let identity = CoreIdentity::production_v5();
    let (started_tx, started_rx) = mpsc::channel();
    let hooks = Arc::new(HeldOwnerHandshakes {
        started: started_tx,
        entered: AtomicUsize::new(0),
        released: Mutex::new(false),
        changed: Condvar::new(),
        hold_count: held_handshakes,
    });
    let config = DaemonServerConfig::new(
        state_root.clone(),
        identity.clone(),
        Duration::from_secs(30),
    )
    .with_runtime_hooks_for_test(hooks.clone());
    let stop = Arc::new(AtomicBool::new(false));
    let server_stop = stop.clone();
    let server = thread::spawn(move || {
        run_daemon_configured_until(
            config,
            |runtime| runtime,
            || server_stop.load(Ordering::SeqCst),
        )
    });
    let mut daemon = AdmissionDaemon {
        stop,
        hooks,
        sockets: Vec::new(),
        server: Some(server),
    };
    let record = wait_for_v5_record(&state_root, &identity);
    let address = record.loopback_addr().unwrap();
    let mut expected_ports = HashSet::new();
    for _ in 0..held_handshakes {
        let stream = TcpStream::connect(address).unwrap();
        expected_ports.insert(stream.local_addr().unwrap().port());
        daemon.sockets.push(stream);
    }
    let mut observed_ports = HashSet::new();
    for _ in 0..held_handshakes {
        observed_ports.insert(
            started_rx
                .recv_timeout(Duration::from_secs(2))
                .expect("actual accepted handshake handler entered gate"),
        );
    }
    assert_eq!(
        observed_ports, expected_ports,
        "only the actual blocker sockets reached the held handshake gate"
    );
    for _ in 0..held_owners {
        let (stream, reader) = connect_v5_owner(&record, &identity, &Uuid::new_v4().to_string());
        daemon.sockets.push(stream);
        drop(reader);
    }
    let result = admitted_owner_cancel_and_recover(&record, &identity, &mut daemon);
    drop(daemon);
    let (cancel, recovered, key) =
        result.expect("a new real owner must remain able to deliver explicit cancellation");
    assert_eq!(
        recovered, cancel,
        "exact recovery must return the durably accepted cancellation"
    );
    assert!(matches!(cancel,
        V5ServerResponse::Invocation { outcome: V5InvocationResponse::ReceiptPending {
            receipt_key, phase: V5InvocationPhase::CancelReserved, cancel_requested: true, ..
        }} if receipt_key == key
    ));
}

#[test]
fn ninth_connection_delivers_cancel_while_eight_real_handshakes_are_held() {
    cancellation_owner_crosses_admission_count(8, 0);
}

#[test]
fn sixty_fifth_authenticated_owner_delivers_exact_cancellation() {
    cancellation_owner_crosses_admission_count(0, 64);
}

#[test]
fn connection_ownership_survives_former_limit_and_checked_overflow() {
    let admitted = Arc::new(AtomicUsize::new(0));
    let mut slots = (0..65)
        .map(|_| V5ConnectionSlot::acquire(Arc::clone(&admitted)).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(admitted.load(Ordering::Acquire), 65);
    let held = slots.pop().unwrap();
    drop(slots);
    assert_eq!(admitted.load(Ordering::Acquire), 1);
    drop(held);
    assert_eq!(admitted.load(Ordering::Acquire), 0);

    let maximum = Arc::new(AtomicUsize::new(usize::MAX));
    assert!(V5ConnectionSlot::acquire(Arc::clone(&maximum)).is_none());
    assert_eq!(maximum.load(Ordering::Acquire), usize::MAX);
}

#[test]
fn live_v5_owner_prevents_idle_listener_shutdown() {
    let root = tempfile::tempdir().expect("temporary owner-idle state root");
    let state_root = std::fs::canonicalize(root.path()).expect("physical owner-idle state root");
    let identity = CoreIdentity::production_v5();
    let config = DaemonServerConfig::new(
        state_root.clone(),
        identity.clone(),
        Duration::from_millis(80),
    );
    let server = thread::spawn(move || run_daemon(config));
    let record = wait_for_v5_record(&state_root, &identity);
    let (mut first, mut first_reader) =
        connect_v5_owner(&record, &identity, "66666666-6666-4666-8666-666666666666");

    thread::sleep(Duration::from_millis(200));
    let (mut successor, mut successor_reader) =
        connect_v5_owner(&record, &identity, "77777777-7777-4777-8777-777777777777");
    write_json_line(&mut successor, &json!({"kind": "release"}));
    let successor_released = read_bounded_v5_probe_response_frame(&mut successor_reader)
        .expect("release successor v5 owner");
    assert_eq!(
        decode_v5_server_response(&successor_released),
        Ok(V5ServerResponse::Released)
    );
    drop(successor);

    write_json_line(&mut first, &json!({"kind": "release"}));
    let first_released = read_bounded_v5_probe_response_frame(&mut first_reader)
        .expect("release original idle-fencing owner");
    assert_eq!(
        decode_v5_server_response(&first_released),
        Ok(V5ServerResponse::Released)
    );
    drop(first);

    server
        .join()
        .expect("join owner-idle runtime")
        .expect("owner-idle runtime");
}

struct PromisedWorkIdleHooks {
    observations: mpsc::Sender<(Instant, bool)>,
}

impl V5RuntimeHooks for PromisedWorkIdleHooks {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn after_idle_owner_leases_read(&self, empty: bool) {
        let _ = self.observations.send((Instant::now(), empty));
    }
}

struct PromisedWorkIdleDaemon {
    // Stop and join before removing the state directory, including on panic.
    guard: HandlerSpawnDaemon,
    _state: tempfile::TempDir,
    state_root: std::path::PathBuf,
    record: V5EndpointRecord,
    observations: mpsc::Receiver<(Instant, bool)>,
}

impl PromisedWorkIdleDaemon {
    fn start(idle_grace: Duration) -> Self {
        Self::with_service(
            idle_grace,
            super::super::server::actor_capacity_tests::canonical_v13_service(),
        )
    }

    fn with_service(
        idle_grace: Duration,
        service: Arc<dyn super::super::server::CanonicalInvocationService>,
    ) -> Self {
        let state = tempfile::tempdir().unwrap();
        let state_root = std::fs::canonicalize(state.path()).unwrap();
        let identity = CoreIdentity::production_v5();
        let (observations, observed) = mpsc::channel();
        let config = DaemonServerConfig::new(state_root.clone(), identity.clone(), idle_grace)
            .with_invocation_service(service)
            .with_runtime_hooks_for_test(Arc::new(PromisedWorkIdleHooks { observations }));
        let stop = Arc::new(AtomicBool::new(false));
        let server_stop = Arc::clone(&stop);
        let server = thread::spawn(move || {
            run_daemon_configured_until(
                config,
                |runtime| runtime,
                || server_stop.load(Ordering::SeqCst),
            )
        });
        let guard = HandlerSpawnDaemon {
            stop,
            streams: Vec::new(),
            server: Some(server),
        };
        let record = wait_for_v5_record(&state_root, &identity);
        Self {
            guard,
            _state: state,
            state_root,
            record,
            observations: observed,
        }
    }

    fn owner(&mut self) -> (TcpStream, BufReader<TcpStream>) {
        let owner = connect_v5_owner(
            &self.record,
            self.record.core_identity(),
            &uuid::Uuid::new_v4().to_string(),
        );
        self.guard.streams.push(owner.0.try_clone().unwrap());
        owner
    }

    fn endpoint(&self) -> Option<V5EndpointRecord> {
        DaemonStateDirectory::open(&self.state_root, self.record.core_identity())
            .unwrap()
            .read_v5_endpoint_record()
            .unwrap()
    }

    /// Observe the actual listener past the requested interval. Its exit drops
    /// the sole sender, so an early idle exit is also observed without sleeping.
    fn observe_until(&self, threshold: Instant) -> bool {
        let watchdog = Instant::now() + Duration::from_secs(3);
        loop {
            match self
                .observations
                .recv_timeout(watchdog.saturating_duration_since(Instant::now()))
            {
                Ok((observed, _)) if observed >= threshold => return true,
                Ok(_) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return false,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    panic!("listener did not reach the idle observation or exit")
                }
            }
        }
    }

    /// Begin the interval only at a fresh observation of the actual empty
    /// owner registry after Release, excluding startup and buffered events.
    fn observe_idle_interval(&self, interval: Duration) -> bool {
        let released_at = Instant::now();
        let watchdog = released_at + Duration::from_secs(3);
        let mut empty_since = None;
        loop {
            match self
                .observations
                .recv_timeout(watchdog.saturating_duration_since(Instant::now()))
            {
                Ok((observed, empty)) if observed >= released_at => {
                    if empty {
                        let start = *empty_since.get_or_insert(observed);
                        if observed.duration_since(start) >= interval {
                            return true;
                        }
                    } else {
                        empty_since = None;
                    }
                }
                Ok(_) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return false,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    panic!("listener neither crossed the owner-free idle interval nor exited")
                }
            }
        }
    }

    fn join(&mut self) {
        self.guard
            .server
            .take()
            .unwrap()
            .join()
            .expect("join promised-work daemon")
            .expect("promised-work daemon exits cleanly");
    }
}

fn promised_work_apply_over_tcp(
    owner: &mut (TcpStream, BufReader<TcpStream>),
    workspace: &std::path::Path,
    arguments: serde_json::Value,
) -> crate::domain::invocation::DomainResult {
    promised_work_call_over_tcp(owner, workspace, V5ToolIdentity::Apply, arguments)
}

fn promised_work_call_over_tcp(
    owner: &mut (TcpStream, BufReader<TcpStream>),
    workspace: &std::path::Path,
    tool: V5ToolIdentity,
    arguments: serde_json::Value,
) -> crate::domain::invocation::DomainResult {
    let invocation = V5InvocationRequest::new(
        InvocationId::new(),
        TaskId::new(),
        tool,
        arguments.as_object().unwrap().clone(),
        std::fs::canonicalize(workspace)
            .unwrap()
            .display()
            .to_string(),
        7_000,
    )
    .unwrap();
    write_json_line(
        &mut owner.0,
        &serde_json::to_value(V5ClientRequest::SubmitInvocation { invocation }).unwrap(),
    );
    let frame =
        read_bounded_v5_probe_response_frame(&mut owner.1).expect("read canonical apply response");
    match decode_v5_server_response(&frame).unwrap() {
        V5ServerResponse::Invocation {
            outcome: V5InvocationResponse::Direct { receipt },
        } => match receipt.terminal() {
            ReceiptTerminalOutcome::Completed { result } => *result.clone(),
            other => panic!("apply did not complete: {other:?}"),
        },
        other => panic!("apply did not return a direct result: {other:?}"),
    }
}

fn promised_work_preview_over_tcp(
    owner: &mut (TcpStream, BufReader<TcpStream>),
    workspace: &std::path::Path,
) -> serde_json::Value {
    let result = promised_work_apply_over_tcp(
        owner,
        workspace,
        json!({
            "at": "main:Subsystem.Sales",
            "ops": [{"op": "props.set", "args": {"values": {"Comment": "kept across idle"}}}]
        }),
    );
    assert!(result.ok, "{result:?}");
    result.data.unwrap()["executionToken"].clone()
}

fn release_promised_work_owner(mut owner: (TcpStream, BufReader<TcpStream>)) {
    write_json_line(&mut owner.0, &json!({"kind": "release"}));
    let released =
        read_bounded_v5_probe_response_frame(&mut owner.1).expect("release canonical apply owner");
    assert_eq!(
        decode_v5_server_response(&released),
        Ok(V5ServerResponse::Released)
    );
    // Released precedes the server lease guard drop. The idle oracle below
    // observes the actual empty registry; socket shutdown is best-effort cleanup.
    let _ = owner.0.shutdown(std::net::Shutdown::Both);
}

#[test]
fn daemon_idle_saved_apply_plan_preserves_instance_and_exact_replay_after_owner_release() {
    let workspace =
        super::super::server::actor_capacity_tests::subsystem_picture_workspace("", false);
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

    let observed = daemon.observe_idle_interval(idle * 3);
    assert_eq!(
        daemon.endpoint(),
        Some(daemon.record.clone()),
        "idle cleanup discarded an unexecuted apply plan and its daemon instance"
    );
    assert!(
        observed,
        "the original listener must remain alive past idle"
    );
    let mut successor = daemon.owner();
    let arguments = json!({"executionToken": token});
    let applied = promised_work_apply_over_tcp(&mut successor, workspace.path(), arguments.clone());
    assert!(applied.ok, "{applied:?}");
    let written = std::fs::read(&descriptor).unwrap();
    assert_ne!(before, written);
    release_promised_work_owner(successor);
    let observed = daemon.observe_idle_interval(idle * 3);
    assert_eq!(
        daemon.endpoint(),
        Some(daemon.record.clone()),
        "idle cleanup discarded a completed plan's replay result"
    );
    assert!(
        observed,
        "completed plan remains promised after all owners release"
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

#[test]
fn daemon_idle_empty_runtime_still_exits_after_owner_release() {
    let idle = Duration::from_millis(80);
    let mut daemon = PromisedWorkIdleDaemon::start(idle);
    let owner = daemon.owner();
    release_promised_work_owner(owner);
    assert!(
        !daemon.observe_idle_interval(idle * 3),
        "an empty daemon must still idle out"
    );
    daemon.join();
    assert!(daemon.endpoint().is_none());
}

#[test]
fn daemon_idle_saved_apply_plan_does_not_prevent_explicit_stop() {
    let workspace =
        super::super::server::actor_capacity_tests::subsystem_picture_workspace("", false);
    let mut daemon = PromisedWorkIdleDaemon::start(Duration::from_secs(30));
    let mut owner = daemon.owner();
    let token = promised_work_preview_over_tcp(&mut owner, workspace.path());
    assert!(token.as_str().is_some_and(|value| !value.is_empty()));
    release_promised_work_owner(owner);
    daemon.guard.stop.store(true, Ordering::SeqCst);
    assert!(
        !daemon.observe_until(Instant::now() + Duration::from_secs(1)),
        "explicit stop must exit despite a pending apply plan"
    );
    daemon.join();
    assert!(daemon.endpoint().is_none());
}

type IdleIndexLease = crate::application::shared_work::SharedWorkLease<
    (),
    crate::application::shared_work::LongWorkFailure,
>;
type IdleIndexGate = Arc<(Mutex<bool>, Condvar)>;

struct IdleIndexService {
    calls: AtomicUsize,
    producers: Arc<AtomicUsize>,
    entered: mpsc::Sender<()>,
    exited: mpsc::Sender<()>,
    release: IdleIndexGate,
    follower: mpsc::Sender<IdleIndexLease>,
}

impl super::super::server::CanonicalInvocationService for IdleIndexService {
    fn prepare(
        &self,
        _invocation: &super::super::server::ActorBoundInvocation,
    ) -> Result<
        crate::application::operation_descriptors::ExecutionClass,
        Box<crate::domain::invocation::DomainResult>,
    > {
        Ok(crate::application::operation_descriptors::ExecutionClass::InlineCandidate)
    }

    fn execute(
        &self,
        invocation: &super::super::server::ActorBoundExecution,
        _cancellation: CancellationToken,
    ) -> Result<crate::domain::invocation::DomainResult, crate::domain::invocation::InvocationFailure>
    {
        let producers = Arc::clone(&self.producers);
        let entered = self.entered.clone();
        let exited = self.exited.clone();
        let release = Arc::clone(&self.release);
        let (_, lease) = invocation
            .join_index_work(
                "rlm",
                "idle-test-profile",
                "idle-test-generation",
                move |_| {
                    producers.fetch_add(1, Ordering::SeqCst);
                    entered.send(()).unwrap();
                    let (released, changed) = &*release;
                    let mut released = released.lock().unwrap();
                    while !*released {
                        released = changed.wait(released).unwrap();
                    }
                    exited.send(()).unwrap();
                    Ok(())
                },
            )
            .map_err(|error| {
                crate::domain::invocation::InvocationFailure::new("index_failed", error)
            })?;
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            assert!(lease.started_here());
            drop(lease);
        } else {
            assert!(
                !lease.started_here(),
                "later RPC must attach to the original live producer"
            );
            self.follower.send(lease).unwrap();
        }
        Ok(crate::domain::invocation::DomainResult::success(
            "index joined",
        ))
    }
}

struct IdleIndexProducerCleanup {
    release: IdleIndexGate,
    exited: mpsc::Receiver<()>,
}

impl IdleIndexProducerCleanup {
    fn release(&self) {
        let (released, changed) = &*self.release;
        *released.lock().unwrap() = true;
        changed.notify_all();
    }
}

impl Drop for IdleIndexProducerCleanup {
    fn drop(&mut self) {
        self.release();
        // Also settle the cooperative producer on a RED assertion before the
        // follower is obtained; the normal path verifies actual Ready below.
        let _ = self.exited.recv_timeout(Duration::from_secs(3));
    }
}

#[test]
fn daemon_idle_running_index_producer_without_rpc_or_lease_preserves_instance_then_retires() {
    let workspace =
        super::super::server::actor_capacity_tests::subsystem_picture_workspace("", false);
    let producers = Arc::new(AtomicUsize::new(0));
    let release = Arc::new((Mutex::new(false), Condvar::new()));
    let (entered, started) = mpsc::channel();
    let (exited, finished) = mpsc::channel();
    let (follower, joined) = mpsc::channel();
    let service = Arc::new(IdleIndexService {
        calls: AtomicUsize::new(0),
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
        .recv_timeout(Duration::from_secs(2))
        .expect("the real SharedWork producer starts");
    release_promised_work_owner(owner);
    let observed = daemon.observe_idle_interval(idle * 3);
    assert_eq!(
        daemon.endpoint(),
        Some(daemon.record.clone()),
        "idle shutdown lost a running index producer after the RPC and its lease ended"
    );
    assert!(observed);
    let mut successor = daemon.owner();
    assert!(call(&mut successor).ok);
    let lease = joined
        .recv_timeout(Duration::from_secs(2))
        .expect("second real RPC attaches to the same exact index work");
    assert_eq!(producers.load(Ordering::SeqCst), 1);
    release_promised_work_owner(successor);
    cleanup.release();
    assert!(
        matches!(
            lease.wait_timeout(Duration::from_secs(2)),
            crate::application::shared_work::SharedWorkSnapshot::Ready(_)
        ),
        "producer must actually settle before its last lease is released"
    );
    drop(lease);
    assert!(
        !daemon.observe_idle_interval(idle * 3),
        "completed index work with no leases must allow empty daemon idle cleanup"
    );
    daemon.join();
    assert!(daemon.endpoint().is_none());
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
    let pong: V5ProbeServerResponse = serde_json::from_slice(&pong).expect("decode strict v5 pong");
    assert_eq!(pong.kind(), V5ProbeResponseKind::Pong);
    write_json_line(&mut stream, &json!({"kind": "release"}));
    let released =
        read_bounded_v5_probe_response_frame(&mut reader).expect("read v5 release response");
    assert_eq!(
        decode_v5_server_response(&released),
        Ok(V5ServerResponse::Released)
    );
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

#[test]
fn direct_runtime_entry_rejects_every_non_v5_identity_before_state_creation() {
    use std::str::FromStr;

    for identity in [
        CoreIdentity::from_str("2f4dd5713d11e5211a92c5fa01b1ec5722dc3a3160b9b1e0b667f8d8da3d9c28")
            .expect("the retired protocol-v3 digest still parses as a canonical identity"),
        CoreIdentity::from_str("eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee")
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
            Err("protocol-v5 runtime requires the exact production-v5 core identity".to_string())
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

struct IdleHandoffHooks {
    accepted: AtomicBool,
    paused: AtomicBool,
    reported: AtomicBool,
    handler_gate: Arc<(Mutex<bool>, std::sync::Condvar)>,
    listener_gate: Arc<(Mutex<bool>, std::sync::Condvar)>,
    owner_read: mpsc::Sender<bool>,
    decision: mpsc::Sender<bool>,
}

fn release_idle_handoff_gate(gate: &(Mutex<bool>, std::sync::Condvar)) {
    *gate.0.lock().unwrap() = true;
    gate.1.notify_all();
}

fn wait_idle_handoff_gate(gate: &(Mutex<bool>, std::sync::Condvar)) {
    let mut open = gate.0.lock().unwrap();
    while !*open {
        open = gate.1.wait(open).unwrap();
    }
}

impl V5RuntimeHooks for IdleHandoffHooks {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn before_owner_handshake(&self, _peer_port: u16) {
        self.accepted.store(true, Ordering::Release);
        wait_idle_handoff_gate(&self.handler_gate);
    }

    fn after_idle_owner_leases_read(&self, empty: bool) {
        if self.accepted.load(Ordering::Acquire) && !self.paused.swap(true, Ordering::AcqRel) {
            self.owner_read.send(empty).unwrap();
            wait_idle_handoff_gate(&self.listener_gate);
        }
    }

    fn listener_ownership_observed(&self, no_active_work: bool) {
        if self.paused.load(Ordering::Acquire) && !self.reported.swap(true, Ordering::AcqRel) {
            self.decision.send(no_active_work).unwrap();
        }
    }
}

struct IdleHandoffDaemon {
    hooks: Arc<IdleHandoffHooks>,
    stop: Arc<AtomicBool>,
    transports: Vec<TcpStream>,
    server: Option<thread::JoinHandle<Result<(), String>>>,
}

impl Drop for IdleHandoffDaemon {
    fn drop(&mut self) {
        release_idle_handoff_gate(&self.hooks.handler_gate);
        release_idle_handoff_gate(&self.hooks.listener_gate);
        for transport in &self.transports {
            let _ = transport.shutdown(std::net::Shutdown::Both);
        }
        self.stop.store(true, Ordering::Release);
        if let Some(server) = self.server.take() {
            let _ = server.join();
        }
    }
}

#[test]
fn owner_handoff_between_idle_reads_keeps_listener_and_exact_owner_alive() {
    let root = tempfile::tempdir().unwrap();
    let state_root = std::fs::canonicalize(root.path()).unwrap();
    let identity = CoreIdentity::production_v5();
    let (owner_read, owner_read_wait) = mpsc::channel();
    let (decision, decision_wait) = mpsc::channel();
    let hooks = Arc::new(IdleHandoffHooks {
        accepted: AtomicBool::new(false),
        paused: AtomicBool::new(false),
        reported: AtomicBool::new(false),
        handler_gate: Arc::new((Mutex::new(false), std::sync::Condvar::new())),
        listener_gate: Arc::new((Mutex::new(false), std::sync::Condvar::new())),
        owner_read,
        decision,
    });
    let config = DaemonServerConfig::new(
        state_root.clone(),
        identity.clone(),
        Duration::from_millis(500),
    )
    .with_runtime_hooks_for_test(hooks.clone());
    let stop = Arc::new(AtomicBool::new(false));
    let listener_stop = Arc::clone(&stop);
    let server = thread::spawn(move || {
        run_daemon_configured_until(
            config,
            |runtime| runtime,
            || listener_stop.load(Ordering::Acquire),
        )
    });
    let mut daemon = IdleHandoffDaemon {
        hooks: Arc::clone(&hooks),
        stop,
        transports: Vec::new(),
        server: Some(server),
    };
    let record = wait_for_v5_record(&state_root, &identity);
    let mut owner = TcpStream::connect(record.loopback_addr().unwrap()).unwrap();
    owner
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    daemon.transports.push(owner.try_clone().unwrap());
    let mut owner_reader = BufReader::new(owner.try_clone().unwrap());
    assert!(
        owner_read_wait
            .recv_timeout(Duration::from_secs(2))
            .unwrap(),
        "listener did not read the genuinely empty owner registry before handoff"
    );
    // Keep the real listener between its two ownership reads beyond its idle
    // grace. This sleep crosses the production Instant-based maintenance clock;
    // gates, rather than timing, determine the handoff interleaving.
    thread::sleep(Duration::from_millis(550));
    write_json_line(
        &mut owner,
        &json!({
            "kind": "hello", "protocolVersion": 5, "token": record.token(),
            "coreIdentity": identity.as_str(), "ownerLease": uuid::Uuid::new_v4().to_string(),
        }),
    );
    release_idle_handoff_gate(&hooks.handler_gate);
    let ready = read_bounded_v5_probe_response_frame(&mut owner_reader).unwrap();
    assert!(matches!(
        decode_v5_server_response(&ready),
        Ok(V5ServerResponse::Ready { .. })
    ));
    // Ready is written after lease acquisition and handshake-slot release.
    release_idle_handoff_gate(&hooks.listener_gate);
    assert!(
        !decision_wait.recv_timeout(Duration::from_secs(2)).unwrap(),
        "idle observation lost an authenticated owner during handshake handoff"
    );
    write_json_line(&mut owner, &json!({"kind": "ping"}));
    let pong = read_bounded_v5_probe_response_frame(&mut owner_reader).unwrap();
    assert_eq!(decode_v5_server_response(&pong), Ok(V5ServerResponse::Pong));
    let (mut next, mut next_reader) =
        connect_v5_owner(&record, &identity, &uuid::Uuid::new_v4().to_string());
    daemon.transports.push(next.try_clone().unwrap());
    write_json_line(&mut next, &json!({"kind": "ping"}));
    let pong = read_bounded_v5_probe_response_frame(&mut next_reader).unwrap();
    assert_eq!(decode_v5_server_response(&pong), Ok(V5ServerResponse::Pong));
    let state = DaemonStateDirectory::open(&state_root, &identity).unwrap();
    let current = state.read_v5_endpoint_record().unwrap().unwrap();
    assert_eq!(current.instance_id(), record.instance_id());
    assert_eq!(
        current.loopback_addr().unwrap(),
        record.loopback_addr().unwrap()
    );
}
