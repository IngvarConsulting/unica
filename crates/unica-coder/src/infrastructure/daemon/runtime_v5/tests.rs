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

/// The daemon's invocation clock, moved by the test: it decides when the
/// handler hands an unfinished attempt off to its Task.
struct InspectionClock {
    start: Instant,
    elapsed_ms: AtomicU64,
}

impl Clock for InspectionClock {
    fn now(&self) -> Instant {
        self.start + Duration::from_millis(self.elapsed_ms.load(Ordering::SeqCst))
    }
}

/// Far past the former 120 s operation budget and the former 2 s fail-stop
/// grace: no automatic cutoff may fire at this reading (#1251).
const FAR_PAST_FORMER_DEADLINES_MS: u64 = 600_000;

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
    let clock = Arc::new(InspectionClock {
        start: Instant::now(),
        elapsed_ms: AtomicU64::new(0),
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
    let server = thread::spawn(move || {
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
        unica_bootstrap::ResolvedWorkspace::launch_cwd(
            workspace_root.to_string_lossy().into_owned(),
        ),
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

    clock
        .elapsed_ms
        .store(FAR_PAST_FORMER_DEADLINES_MS, Ordering::SeqCst);
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
                    unica_bootstrap::ResolvedWorkspace::launch_cwd(
                        independent_workspace.path().to_string_lossy().into_owned(),
                    ),
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
        "a slow health inspection still reports workspace facts: {result:?}"
    );
    let data = result.data.unwrap();
    if tool == V5ToolIdentity::Check {
        // The inspection ran to its end inside the Task: no portion cut at
        // the handoff window and no continuation to call for (#1251).
        assert_eq!(data["readinessState"], "complete", "{data}");
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
fn root_view_keeps_the_same_task_and_daemon_past_the_former_deadlines() {
    root_inspection_survives_response_cutoff(V5ToolIdentity::View);
}

#[test]
fn root_check_completes_the_whole_inspection_in_its_task() {
    root_inspection_survives_response_cutoff(V5ToolIdentity::Check);
}

#[test]
fn cancel_during_actor_admission_after_handoff_never_begins_and_keeps_the_daemon() {
    let state = tempfile::tempdir().unwrap();
    let state_root = std::fs::canonicalize(state.path()).unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let workspace_root = std::fs::canonicalize(workspace.path()).unwrap();
    std::fs::create_dir(workspace_root.join("src")).unwrap();
    std::fs::write(
        workspace_root.join("v8project.yaml"),
        b"format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    std::fs::write(
        workspace_root.join("src/Configuration.xml"),
        br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Store</Name></Properties><ChildObjects/></Configuration></MetaDataObject>"#,
    )
    .unwrap();
    let independent_workspace = tempfile::tempdir().unwrap();
    let admission =
        crate::infrastructure::daemon::server::admission_test_control::AdmissionPause::install(
            workspace_root.clone(),
        );
    let clock = Arc::new(InspectionClock {
        start: Instant::now(),
        elapsed_ms: AtomicU64::new(0),
    });
    // Its prepare panics: a cancelled admission must never reach it.
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
    let server = thread::spawn(move || {
        run_daemon_configured_until(
            config,
            |runtime| runtime,
            || server_stop.load(Ordering::SeqCst),
        )
    });
    wait_for_v5_record(&state_root, &identity);
    let owner = V5DaemonProcessOwner::connect_or_spawn(
        &state_root,
        identity,
        std::path::PathBuf::from("unused-existing-v5-endpoint"),
        Duration::from_secs(2),
    )
    .unwrap();
    let task_id = TaskId::new();
    let invocation = V5InvocationRequest::new(
        InvocationId::new(),
        task_id,
        V5ToolIdentity::View,
        serde_json::Map::from_iter([(
            "at".to_owned(),
            serde_json::Value::String("main:Catalog.Items".to_owned()),
        )]),
        unica_bootstrap::ResolvedWorkspace::launch_cwd(
            workspace_root.to_string_lossy().into_owned(),
        ),
        7_000,
    )
    .unwrap();
    let submit = thread::spawn(move || {
        let mut owner = owner;
        let response = owner.submit_invocation(invocation);
        (owner, response)
    });
    admission.wait_until_entered();
    clock.elapsed_ms.store(7_000, Ordering::SeqCst);
    let (mut owner, submitted) = submit.join().unwrap();
    let V5ServerResponse::Invocation {
        outcome: V5InvocationResponse::Task { snapshot },
    } = submitted.expect("an admission past the handoff moment returns its Task")
    else {
        panic!("slow admission did not hand off to a Task");
    };
    assert_eq!(snapshot.task_id(), task_id);

    // The admission keeps running past every former deadline; only the
    // explicit cancel below ends it.
    clock
        .elapsed_ms
        .store(FAR_PAST_FORMER_DEADLINES_MS, Ordering::SeqCst);
    owner
        .cancel_task(task_id)
        .expect("cancel the Task whose admission is still running");
    admission.release();
    let settled = owner
        .wait_task(task_id, 7_000)
        .expect("the cancelled Task stays reachable");
    assert!(
        matches!(
            settled,
            V5ServerResponse::Task {
                snapshot: V5DaemonTaskSnapshot::Cancelled { .. }
            }
        ),
        "a cancel during admission must end the Task as cancelled: {settled:?}"
    );

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
                    unica_bootstrap::ResolvedWorkspace::launch_cwd(
                        independent_workspace.path().to_string_lossy().into_owned(),
                    ),
                    7_000,
                )
                .unwrap(),
            )
        });
    let V5ServerResponse::Invocation {
        outcome: V5InvocationResponse::Direct { receipt },
    } = independent.expect("the daemon serves an independent call after the cancel")
    else {
        panic!("independent read did not complete directly");
    };
    assert!(
        matches!(receipt.terminal(), ReceiptTerminalOutcome::Completed { result } if result.ok)
    );
    assert_eq!(
        service.prepares.load(Ordering::SeqCst),
        0,
        "a cancelled admission must not begin the source operation"
    );
    // Read after the independent call, so the woken admission had time to
    // reach its next checkpoint and stop there.
    assert_eq!(
        admission.entries(),
        1,
        "the admission walk must stop at its first checkpoint after the cancel"
    );
    assert!(!hooks.fail_stopped.load(Ordering::SeqCst));
    drop(owner);
    drop(admission);
    stop.store(true, Ordering::SeqCst);
    server
        .join()
        .expect("daemon thread did not panic")
        .expect("daemon exits cleanly");
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
use crate::domain::invocation::{InvocationId, NormalizedArgumentsHash, SafeIdentityHash, TaskId};
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
        unica_bootstrap::ResolvedWorkspace::launch_cwd(workspace.to_string_lossy().into_owned()),
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
        unica_bootstrap::ResolvedWorkspace::launch_cwd("workspace-a".to_owned()),
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

/// #1360: the predecessor died after the lifecycle link became `TaskBound`
/// and before the handoff receipt was completed. The successor must adopt
/// that exact link, finish the Task with a recovery terminal and start; it
/// must neither refuse to start nor replay the work. The second crash point
/// is inside that recovery: the receipt already handed the Task to its link,
/// the recovery terminal is not written yet.
#[test]
fn startup_completes_handoff_whose_link_is_already_task_bound() {
    for (phase, cancel_requested, receipt_completed_before_crash) in [
        (AttemptPhase::NotBegun, false, false),
        (AttemptPhase::NotBegun, true, false),
        (AttemptPhase::Begun, false, false),
        (AttemptPhase::Begun, true, false),
        (AttemptPhase::NotBegun, false, true),
        (AttemptPhase::NotBegun, true, true),
        (AttemptPhase::Begun, false, true),
        (AttemptPhase::Begun, true, true),
    ] {
        let root = tempfile::tempdir().expect("temporary bound handoff recovery root");
        let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
        let identity = CoreIdentity::production_v5();
        let state = DaemonStateDirectory::open(&state_root, &identity)
            .expect("open bound handoff recovery daemon state");
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
            .expect("reserve bound handoff receipt")
            .into_reservation()
            .expect("new bound handoff receipt");
        let bound = runtime
            .receipt_ledger
            .bind_reserved_actor(
                key.clone(),
                reserved.record_version(),
                SafeIdentityHash::from_sha256(Sha256::digest(b"bound-handoff").into()),
                deadline,
            )
            .expect("bind bound handoff actor");
        let version = match phase {
            AttemptPhase::NotBegun => bound.record_version(),
            AttemptPhase::Begun => runtime
                .receipt_ledger
                .mark_reserved_begun(key.clone(), bound.record_version(), deadline)
                .expect("mark bound handoff begun")
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
            .expect("persist bound Task handoff");
        if cancel_requested {
            runtime
                .receipt_ledger
                .request_task_cancel(
                    key.clone(),
                    TaskCancellationReceipt::HandoffActorBound(handoff),
                    deadline,
                )
                .expect("persist bound handoff cancellation");
        }
        let ReceiptState::TaskHandoffActorBound(handoff) = runtime
            .receipt_ledger
            .recover(key.clone(), deadline)
            .expect("read bound handoff receipt")
        else {
            panic!("bound handoff receipt changed state");
        };
        // The crash window: TaskStore row and TaskBound link are durable,
        // the receipt still holds the handoff intent.
        let (created, task_bound) = runtime
            .task_projection
            .materialize_bound_handoff(&handoff, 1_010, deadline, runtime.hooks.as_ref())
            .unwrap_or_else(|failure| panic!("materialize bound handoff: {:?}", failure.error));
        assert_eq!(task_bound.phase(), phase);
        if receipt_completed_before_crash {
            runtime
                .receipt_ledger
                .complete_bound_task_handoff(
                    key.clone(),
                    handoff.record_version(),
                    task_bound,
                    deadline,
                )
                .expect("hand the bound Task over before the recovery terminal");
        }
        drop(runtime);

        let service = Arc::new(SourceAdmissionProbe::default());
        let config = config.clone().with_invocation_service(service.clone());
        let reopened = V5ReceiptRuntime::open(&state, &config).unwrap_or_else(|error| {
            panic!(
                "{phase:?}/{cancel_requested}/{receipt_completed_before_crash}: \
                 startup must recover a TaskBound handoff: {error}"
            )
        });
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
            .expect("resolve recovered bound handoff Task");
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
            (_, _, other) => panic!("unexpected recovered bound handoff Task: {other:?}"),
        }
        let terminal = reopened
            .task_projection
            .task_store
            .get(
                key.reserved_task_id(),
                crate::domain::code_intelligence::ProviderDeadline::new(
                    Instant::now() + Duration::from_secs(2),
                ),
            )
            .expect("read recovered bound handoff Task");
        assert_eq!(terminal.version, created.version + 1, "one recovery step");
        assert_eq!(
            service.prepares.load(Ordering::SeqCst),
            0,
            "recovery must not replay the work"
        );
        drop(reopened);

        // The recovered state is final: the next process starts as well.
        V5ReceiptRuntime::open(&state, &config).expect("second restart after recovery");
    }
}

/// A NotBegun handoff never starts its Task before the receipt hands it to
/// the link. A Working row under such a link is not a crash state: startup
/// refuses it by name and changes nothing.
#[test]
fn startup_refuses_not_begun_handoff_whose_bound_task_is_working() {
    let root = tempfile::tempdir().expect("temporary contradictory bound handoff root");
    let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
    let identity = CoreIdentity::production_v5();
    let state = DaemonStateDirectory::open(&state_root, &identity)
        .expect("open contradictory bound handoff daemon state");
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
        .expect("reserve contradictory handoff receipt")
        .into_reservation()
        .expect("new contradictory handoff receipt");
    let bound = runtime
        .receipt_ledger
        .bind_reserved_actor(
            key.clone(),
            reserved.record_version(),
            SafeIdentityHash::from_sha256(Sha256::digest(b"contradictory-handoff").into()),
            deadline,
        )
        .expect("bind contradictory handoff actor");
    let handoff = runtime
        .receipt_ledger
        .begin_bound_task_handoff(
            key.clone(),
            bound.record_version(),
            1_009,
            3_600_000,
            V5_TASK_POLL_INTERVAL_MS,
            deadline,
        )
        .expect("persist NotBegun handoff");
    let (queued, _task_bound) = runtime
        .task_projection
        .materialize_bound_handoff(&handoff, 1_010, deadline, runtime.hooks.as_ref())
        .unwrap_or_else(|failure| panic!("materialize NotBegun handoff: {:?}", failure.error));
    let provider_deadline = crate::domain::code_intelligence::ProviderDeadline::new(deadline);
    runtime
        .task_projection
        .task_store
        .start_working_if_not_cancel_requested(
            &queued.identity(),
            queued.version,
            provider_deadline,
        )
        .expect("start the Task behind its link");
    let working = runtime
        .task_projection
        .task_store
        .get(key.reserved_task_id(), provider_deadline)
        .expect("read Working Task");
    assert_eq!(working.task, V5StoredTask::Working);
    drop(runtime);

    let error = match V5ReceiptRuntime::open(&state, &config) {
        Ok(_) => panic!("a NotBegun handoff with a Working Task must not be recovered"),
        Err(error) => error,
    };
    assert!(
        error.contains("preexisting handoff Task contradicts its TaskBound link"),
        "{error}"
    );
    let task_root = RetainedDirectoryCapability::open(&state.path().join("tasks"))
        .expect("retain TaskStore after refused startup");
    let (store, _) = FileInvocationStoreV5::open_retained_directory_inspect_only(
        task_root,
        Arc::new(SystemEpochMillisClock),
        crate::domain::code_intelligence::ProviderDeadline::new(
            Instant::now() + Duration::from_secs(2),
        ),
    )
    .expect("inspect TaskStore after refused startup");
    assert_eq!(
        store
            .get(
                key.reserved_task_id(),
                crate::domain::code_intelligence::ProviderDeadline::new(
                    Instant::now() + Duration::from_secs(2),
                ),
            )
            .expect("read unchanged Task"),
        working
    );
}

#[derive(Default)]
struct StagedTerminalPostStoreCrash {
    exited: AtomicBool,
}

impl V5RuntimeHooks for StagedTerminalPostStoreCrash {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn holds(&self, point: V5PausePoint) -> bool {
        point == V5PausePoint::AfterTaskStoreTerminalBeforeLifecycleLinkTerminal
    }

    fn pause(&self, point: V5PausePoint, _deadline: Instant) -> Result<(), ReceiptLedgerError> {
        if point == V5PausePoint::AfterTaskStoreTerminalBeforeLifecycleLinkTerminal {
            self.exited.store(true, Ordering::SeqCst);
        }
        Ok(())
    }

    fn process_exited(&self) -> bool {
        self.exited.load(Ordering::SeqCst)
    }
}

#[derive(Default)]
struct StagedStartupObservation {
    provisional_publications: AtomicUsize,
    confirmed_terminals: AtomicUsize,
}

impl V5RuntimeHooks for StagedStartupObservation {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn staged_terminal_publication(
        &self,
        _handoff: &TaskHandoffActorBoundReceipt,
        _provisional: &V5StoredInvocationRecord,
        _terminal_record: &V5StoredInvocationRecord,
        _link: &TaskTerminalBoundReceipt,
    ) -> Result<(), ReceiptLedgerError> {
        self.provisional_publications.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn terminal_bound_task(
        &self,
        _record: &V5StoredInvocationRecord,
        _link: &TaskTerminalBoundReceipt,
    ) {
        self.confirmed_terminals.fetch_add(1, Ordering::SeqCst);
    }
}

struct StagedStartupCrashFixture {
    _root: tempfile::TempDir,
    state: DaemonStateDirectory,
    config: DaemonServerConfig,
    clock: Arc<ManualEpochClock>,
    service: Arc<SourceAdmissionProbe>,
    key: ReceiptKey,
    runtime: V5ReceiptRuntime,
    before: V5StoredInvocationRecord,
    task_path: std::path::PathBuf,
    task_bytes: Vec<u8>,
}

fn prepare_staged_startup_handoff(
    runtime: &V5ReceiptRuntime,
    identity: &CoreIdentity,
    phase: AttemptPhase,
) -> TaskHandoffActorBoundReceipt {
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
    let deadline = Instant::now() + Duration::from_secs(5);
    let reserved = runtime
        .receipt_ledger
        .reserve(
            key.clone(),
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid cutoff"),
            deadline,
        )
        .expect("reserve staged receipt")
        .into_reservation()
        .expect("new staged receipt");
    let bound = runtime
        .receipt_ledger
        .bind_reserved_actor(
            key.clone(),
            reserved.record_version(),
            SafeIdentityHash::from_sha256(Sha256::digest(b"staged-startup").into()),
            deadline,
        )
        .expect("bind staged actor");
    let version = match phase {
        AttemptPhase::NotBegun => bound.record_version(),
        AttemptPhase::Begun => runtime
            .receipt_ledger
            .mark_reserved_begun(key.clone(), bound.record_version(), deadline)
            .expect("persist Begun before staged handoff")
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
        .expect("persist Begun handoff");
    assert_eq!(handoff.phase(), phase);
    handoff
}

fn staged_startup_crash_fixture() -> StagedStartupCrashFixture {
    let root = tempfile::tempdir().expect("temporary staged startup root");
    let state_root = std::fs::canonicalize(root.path()).expect("physical staged startup root");
    let identity = CoreIdentity::production_v5();
    let state = DaemonStateDirectory::open(&state_root, &identity).expect("open staged state");
    let service = Arc::new(SourceAdmissionProbe::default());
    let config = DaemonServerConfig::new(state_root, identity.clone(), Duration::from_millis(50))
        .with_invocation_service(service.clone());
    // A single epoch isolates recovery from delayed materialization and clock jumps.
    let clock = Arc::new(ManualEpochClock::new(1_009));
    let crash = Arc::new(StagedTerminalPostStoreCrash::default());
    let runtime = V5ReceiptRuntime::open_with_epoch_clock(
        &state,
        &config.clone().with_runtime_hooks_for_test(crash.clone()),
        clock.clone(),
    )
    .expect("open original staged runtime");
    let handoff = prepare_staged_startup_handoff(&runtime, &identity, AttemptPhase::Begun);
    let key = handoff.key().clone();
    let deadline = Instant::now() + Duration::from_secs(5);
    let winner = canonical_v5_terminal(&ReceiptTerminalOutcome::Completed {
        result: Box::new(DomainResult::success("durable staged winner")),
    })
    .expect("canonical saved winner");
    assert!(matches!(
        runtime.publish_staged_handoff_terminal_reply(handoff, winner.clone(), 1_009, deadline),
        Err(ReceiptLedgerError::StoreUnavailable)
    ));
    assert!(
        crash.process_exited(),
        "the real post-store boundary was not reached"
    );
    let provider_deadline = crate::domain::code_intelligence::ProviderDeadline::new(deadline);
    let before = runtime
        .task_projection
        .task_store
        .get(key.reserved_task_id(), provider_deadline)
        .expect("read committed terminal before crash cleanup");
    assert!(matches!(
        &before.task,
        V5StoredTask::Completed { terminal_epoch_ms: 1_009, terminal_digest, .. }
            if terminal_digest == winner.digest()
    ));
    let receipt = runtime
        .receipt_ledger
        .recover(key.clone(), deadline)
        .expect("staged owner remains active after post-store crash");
    let ReceiptState::TaskHandoffActorBound(staged) = receipt else {
        panic!("post-store crash lost the staged owner");
    };
    assert!(matches!(
        staged.terminal_stage(),
        HandoffTerminalStage::Staged { terminal, terminal_epoch_ms: 1_009, .. }
            if terminal == &winner
    ));
    assert!(matches!(
        runtime
            .task_projection
            .lifecycle_links
            .read_by_task_id(key.reserved_task_id(), provider_deadline)
            .expect("read unfinished TaskBound link"),
        TaskLifecycleLinkRecord::TaskBound(_)
    ));
    let task_path = state
        .path()
        .join("tasks")
        .join(format!("{}.json", key.reserved_task_id()));
    let task_bytes = std::fs::read(&task_path).expect("read exact terminal bytes");
    assert_eq!(service.prepares.load(Ordering::SeqCst), 0);
    StagedStartupCrashFixture {
        _root: root,
        state,
        config,
        clock,
        service,
        key,
        runtime,
        before,
        task_path,
        task_bytes,
    }
}

fn assert_staged_startup_preserves_exact_winner(fixture: StagedStartupCrashFixture) {
    let StagedStartupCrashFixture {
        _root,
        state,
        config,
        clock,
        service,
        key,
        runtime,
        before,
        task_path,
        task_bytes,
    } = fixture;
    drop(runtime);
    let observation = Arc::new(StagedStartupObservation::default());
    let config = config.with_runtime_hooks_for_test(observation.clone());
    for _ in 0..2 {
        let reopened = V5ReceiptRuntime::open_with_epoch_clock(&state, &config, clock.clone())
            .unwrap_or_else(|error| panic!("startup refused the committed staged winner: {error}"));
        let deadline = Instant::now() + Duration::from_secs(5);
        let provider_deadline = crate::domain::code_intelligence::ProviderDeadline::new(deadline);
        assert_eq!(
            reopened
                .task_projection
                .task_store
                .get(key.reserved_task_id(), provider_deadline)
                .expect("read recovered terminal"),
            before
        );
        assert_eq!(
            std::fs::read(&task_path).expect("read recovered bytes"),
            task_bytes
        );
        assert_eq!(
            reopened.receipt_ledger.recover(key.clone(), deadline),
            Err(ReceiptLedgerError::ReceiptNotFound)
        );
        let link = reopened
            .task_projection
            .lifecycle_links
            .read_by_task_id(key.reserved_task_id(), provider_deadline)
            .expect("read finished terminal link");
        let TaskLifecycleLinkRecord::TaskTerminalBound(terminal_link) = link else {
            panic!("startup did not complete terminal ownership");
        };
        assert!(task_terminal_bound_matches_record(&terminal_link, &before)
            .unwrap_or_else(|failure| panic!("confirm recovered link: {}", failure.error)));
        assert_eq!(terminal_link.terminal_epoch_ms(), 1_009);
        assert_eq!(service.prepares.load(Ordering::SeqCst), 0);
        drop(reopened);
    }
    assert_eq!(
        observation.provisional_publications.load(Ordering::SeqCst),
        0,
        "readback of an existing winner must not claim a provisional replacement"
    );
    assert_eq!(observation.confirmed_terminals.load(Ordering::SeqCst), 1);
}

#[test]
fn startup_completes_exact_staged_terminal_after_post_store_crash_without_replay() {
    assert_staged_startup_preserves_exact_winner(staged_startup_crash_fixture());
}

fn finish_staged_crash_lifecycle_link(fixture: &StagedStartupCrashFixture) {
    let deadline = Instant::now() + Duration::from_secs(5);
    let provider_deadline = crate::domain::code_intelligence::ProviderDeadline::new(deadline);
    let link = fixture
        .runtime
        .task_projection
        .lifecycle_links
        .read_by_task_id(fixture.key.reserved_task_id(), provider_deadline)
        .expect("read actual unfinished link");
    let TaskLifecycleLinkRecord::TaskBound(bound) = link else {
        panic!("the crash fixture must retain TaskBound");
    };
    // The retained owner finishes the actual link transition, then exits before
    // completing its active receipt. No terminal record or link is synthesized.
    fixture
        .runtime
        .task_projection
        .finish_task_terminal_publication(&bound, fixture.before.clone(), deadline, &NoHooks)
        .unwrap_or_else(|failure| panic!("publish actual terminal link: {}", failure.error));
    assert!(matches!(
        fixture
            .runtime
            .receipt_ledger
            .recover(fixture.key.clone(), deadline),
        Ok(ReceiptState::TaskHandoffActorBound(_))
    ));
    assert_eq!(
        std::fs::read(&fixture.task_path).unwrap(),
        fixture.task_bytes
    );
}

#[test]
fn startup_completes_exact_staged_receipt_after_terminal_link_commit_without_replay() {
    let fixture = staged_startup_crash_fixture();
    finish_staged_crash_lifecycle_link(&fixture);
    assert_staged_startup_preserves_exact_winner(fixture);
}

fn staged_startup_persisted_bytes(
    root: &std::path::Path,
) -> std::collections::BTreeMap<std::path::PathBuf, Vec<u8>> {
    let mut files = std::collections::BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if entry.file_type().unwrap().is_dir() {
                pending.push(path);
            } else {
                files.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    std::fs::read(path).unwrap(),
                );
            }
        }
    }
    files
}

#[test]
fn startup_refuses_changed_staged_winner_metadata_before_any_store_mutation() {
    for terminal_link_committed in [false, true] {
        for field in [
            "invocation",
            "receipt",
            "tool",
            "arguments",
            "workspace",
            "created",
            "updated",
            "ttl",
            "poll",
            "cancel",
            "version",
            "terminal",
        ] {
            let fixture = staged_startup_crash_fixture();
            if terminal_link_committed {
                finish_staged_crash_lifecycle_link(&fixture);
            }
            let mut changed = fixture.before.clone();
            match field {
                "invocation" => changed.invocation_id = InvocationId::new(),
                "receipt" => {
                    changed.receipt_key_digest = "42"
                        .repeat(32)
                        .parse()
                        .expect("valid changed receipt digest")
                }
                "tool" => changed.tool = V5ToolIdentity::Apply,
                "arguments" => {
                    changed.normalized_arguments_hash =
                        NormalizedArgumentsHash::from_sha256([0x43; 32])
                }
                "workspace" => {
                    changed.workspace_identity_hash = SafeIdentityHash::from_sha256([0x44; 32])
                }
                "created" => changed.created_at_epoch_ms -= 1,
                "updated" => changed.updated_at_epoch_ms += 1,
                "ttl" => changed.ttl_ms += 1,
                "poll" => changed.poll_interval_ms += 1,
                "cancel" => changed.cancel_requested = true,
                "version" => changed.version += 1,
                "terminal" => {
                    let V5StoredTask::Completed { result, .. } = &mut changed.task else {
                        panic!("fixture must contain a completed winner");
                    };
                    **result = DomainResult::success("different result with the same digest");
                }
                _ => unreachable!(),
            }
            let bytes = serde_json::to_vec(&changed).unwrap();
            std::fs::write(&fixture.task_path, bytes).unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            assert_eq!(
                fixture
                    .runtime
                    .task_projection
                    .task_store
                    .get(
                        fixture.key.reserved_task_id(),
                        crate::domain::code_intelligence::ProviderDeadline::new(deadline)
                    )
                    .expect("tamper fixture must be a valid parsed Task"),
                changed
            );
            drop(fixture.runtime);
            let before = staged_startup_persisted_bytes(fixture.state.path());
            let reopened = V5ReceiptRuntime::open_with_epoch_clock(
                &fixture.state,
                &fixture.config,
                fixture.clock.clone(),
            );
            assert!(
                reopened.is_err(),
                "startup accepted changed {field}, terminal link={terminal_link_committed}"
            );
            assert_eq!(staged_startup_persisted_bytes(fixture.state.path()), before,
                "refusal changed persisted bytes for {field}, terminal link={terminal_link_committed}");
            assert_eq!(fixture.service.prepares.load(Ordering::SeqCst), 0);
        }
    }
}

fn materialize_saved_staged_startup_owner(
    runtime: &V5ReceiptRuntime,
    handoff: TaskHandoffActorBoundReceipt,
    winner: &crate::application::receipt_ledger::V5CanonicalTerminal,
) -> (
    TaskHandoffActorBoundReceipt,
    V5StoredInvocationRecord,
    TaskBoundReceipt,
) {
    let deadline = Instant::now() + Duration::from_secs(5);
    let certificate = canonical_staged_transfer_certificate(
        handoff.key(),
        handoff.key_digest(),
        handoff.link(),
        1_009,
        winner,
    )
    .unwrap();
    let staged = runtime
        .receipt_ledger
        .stage_bound_task_handoff_terminal(
            handoff.key().clone(),
            handoff.record_version(),
            1_009,
            winner.clone(),
            certificate,
            deadline,
        )
        .unwrap();
    let reservation = runtime
        .task_projection
        .reserve_bound_handoff_link(&staged, 1_009, deadline, &NoHooks)
        .unwrap_or_else(|failure| panic!("reserve staged fixture link: {}", failure.error));
    let (record, bound) = runtime
        .task_projection
        .materialize_staged_bound_handoff(&staged, &reservation, 1_009, deadline, &NoHooks)
        .unwrap_or_else(|failure| panic!("materialize staged fixture owner: {}", failure.error));
    (staged, record, bound)
}

#[test]
fn startup_publishes_saved_staged_winner_from_exact_provisional_without_replay() {
    for phase in [AttemptPhase::NotBegun, AttemptPhase::Begun] {
        let root = tempfile::tempdir().unwrap();
        let identity = CoreIdentity::production_v5();
        let state =
            DaemonStateDirectory::open(&root.path().canonicalize().unwrap(), &identity).unwrap();
        let service = Arc::new(SourceAdmissionProbe::default());
        let config = DaemonServerConfig::new(
            root.path().canonicalize().unwrap(),
            identity.clone(),
            Duration::from_millis(50),
        )
        .with_invocation_service(service.clone());
        let clock = Arc::new(ManualEpochClock::new(1_009));
        let runtime =
            V5ReceiptRuntime::open_with_epoch_clock(&state, &config, clock.clone()).unwrap();
        let handoff = prepare_staged_startup_handoff(&runtime, &identity, phase);
        let winner = canonical_v5_terminal(&ReceiptTerminalOutcome::Completed {
            result: Box::new(DomainResult::success("saved before terminal publication")),
        })
        .unwrap();
        let (staged, provisional, _) =
            materialize_saved_staged_startup_owner(&runtime, handoff, &winner);
        assert!(!provisional.task.is_terminal());
        let key = staged.key().clone();
        drop(runtime);
        let observation = Arc::new(StagedStartupObservation::default());
        let config = config.with_runtime_hooks_for_test(observation.clone());
        let reopened =
            V5ReceiptRuntime::open_with_epoch_clock(&state, &config, clock.clone()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let provider_deadline = crate::domain::code_intelligence::ProviderDeadline::new(deadline);
        let stored = reopened
            .task_projection
            .task_store
            .get(key.reserved_task_id(), provider_deadline)
            .unwrap();
        let mut expected = provisional;
        expected.version += 1;
        expected.updated_at_epoch_ms = 1_009;
        expected.task = V5TaskProjection::terminal_publication(&winner, 1_009)
            .0
            .into_stored_task();
        assert_eq!(stored, expected);
        assert_eq!(
            reopened.receipt_ledger.recover(key.clone(), deadline),
            Err(ReceiptLedgerError::ReceiptNotFound)
        );
        let path = state
            .path()
            .join("tasks")
            .join(format!("{}.json", key.reserved_task_id()));
        let bytes = std::fs::read(&path).unwrap();
        drop(reopened);
        let reopened = V5ReceiptRuntime::open_with_epoch_clock(&state, &config, clock).unwrap();
        assert_eq!(
            reopened
                .task_projection
                .task_store
                .get(key.reserved_task_id(), provider_deadline)
                .unwrap(),
            expected
        );
        assert_eq!(std::fs::read(path).unwrap(), bytes);
        assert_eq!(service.prepares.load(Ordering::SeqCst), 0);
        assert_eq!(
            observation.provisional_publications.load(Ordering::SeqCst),
            1
        );
        assert_eq!(observation.confirmed_terminals.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn startup_validates_all_staged_owners_before_completing_any_receipt_or_link() {
    let root = tempfile::tempdir().unwrap();
    let identity = CoreIdentity::production_v5();
    let state =
        DaemonStateDirectory::open(&root.path().canonicalize().unwrap(), &identity).unwrap();
    let service = Arc::new(SourceAdmissionProbe::default());
    let config = DaemonServerConfig::new(
        root.path().canonicalize().unwrap(),
        identity.clone(),
        Duration::from_millis(50),
    )
    .with_invocation_service(service.clone());
    let clock = Arc::new(ManualEpochClock::new(1_009));
    let runtime = V5ReceiptRuntime::open_with_epoch_clock(&state, &config, clock.clone()).unwrap();
    let handoffs = (0..2)
        .map(|_| prepare_staged_startup_handoff(&runtime, &identity, AttemptPhase::Begun))
        .collect::<Vec<_>>();
    let winner = canonical_v5_terminal(&ReceiptTerminalOutcome::Completed {
        result: Box::new(DomainResult::success("captured winner before crash")),
    })
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let provider_deadline = crate::domain::code_intelligence::ProviderDeadline::new(deadline);
    let mut committed = Vec::new();
    for handoff in handoffs {
        let (staged, provisional, _) =
            materialize_saved_staged_startup_owner(&runtime, handoff, &winner);
        // Actual store CAS, intentionally stop before the following link/receipt
        // transitions. Both captured owners were admitted before this boundary.
        let terminal = runtime
            .task_projection
            .task_store
            .publish_staged_terminal_against_exact_provisional(
                &provisional,
                V5TaskProjection::terminal_publication(&winner, 1_009).0,
                provider_deadline,
            )
            .unwrap();
        committed.push((
            staged.key_digest().as_str().to_owned(),
            staged.key().clone(),
            terminal,
        ));
    }
    committed.sort_by(|left, right| left.0.cmp(&right.0));
    let (_, key, mut changed) = committed.pop().unwrap();
    changed.poll_interval_ms += 1;
    let path = state
        .path()
        .join("tasks")
        .join(format!("{}.json", key.reserved_task_id()));
    std::fs::write(path, serde_json::to_vec(&changed).unwrap()).unwrap();
    assert_eq!(
        runtime
            .task_projection
            .task_store
            .get(key.reserved_task_id(), provider_deadline)
            .unwrap(),
        changed
    );
    drop(runtime);
    let before = staged_startup_persisted_bytes(state.path());
    assert!(V5ReceiptRuntime::open_with_epoch_clock(&state, &config, clock).is_err());
    assert_eq!(
        staged_startup_persisted_bytes(state.path()),
        before,
        "a later invalid owner must prevent completion of the earlier exact staged owner"
    );
    assert_eq!(service.prepares.load(Ordering::SeqCst), 0);
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
                    runtime.hooks.as_ref(),
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

    // The daemon process records the refusal for its frontend (#1360).
    let error = run_daemon(config.clone()).expect_err("daemon refuses the persisted state");
    assert!(error.contains("preexisting handoff Task has no exact prior link reservation"));
    assert_eq!(
        state.read_startup_failure(std::process::id()).as_deref(),
        Some(error.as_str())
    );

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
fn started_daemon_removes_an_earlier_startup_failure_record() {
    let root = tempfile::tempdir().expect("temporary startup failure root");
    let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
    let identity = CoreIdentity::production_v5();
    let state = DaemonStateDirectory::open(&state_root, &identity)
        .expect("open startup failure daemon state");
    let earlier_pid = std::process::id();
    state
        .publish_startup_failure(earlier_pid, "an earlier refusal")
        .expect("record an earlier refusal");
    let config = DaemonServerConfig::new(state_root.clone(), identity, Duration::from_millis(50));
    run_daemon(config).expect("daemon starts and leaves after its idle grace");
    assert_eq!(state.read_startup_failure(earlier_pid), None);
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
        unica_bootstrap::ResolvedWorkspace::launch_cwd("workspace-a".to_owned()),
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
        unica_bootstrap::ResolvedWorkspace::launch_cwd("workspace-a".to_string()),
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
        unica_bootstrap::ResolvedWorkspace::launch_cwd(
            std::fs::canonicalize(workspace)
                .unwrap()
                .display()
                .to_string(),
        ),
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

/// The first frame is read before the token is checked and keeps the small
/// handshake bound; an authenticated session reads far larger frames (#1309).
#[test]
fn handshake_keeps_its_small_bound_while_an_authenticated_frame_may_be_far_larger() {
    use crate::infrastructure::daemon::protocol_v5::{
        MAX_V5_HANDSHAKE_LINE_BYTES, MAX_V5_REQUEST_LINE_BYTES,
    };
    // JSON whitespace pads a frame without changing its meaning.
    fn padded_line(value: &serde_json::Value, line_bytes: usize) -> Vec<u8> {
        let mut bytes = serde_json::to_vec(value).expect("serialize v5 frame");
        bytes.resize(line_bytes - 1, b' ');
        bytes.push(b'\n');
        bytes
    }
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
    let hello = json!({
        "kind": "hello",
        "protocolVersion": 5,
        "token": record.token(),
        "coreIdentity": identity.as_str(),
        "ownerLease": "44444444-4444-4444-8444-444444444444"
    });
    let connect = || {
        let stream = TcpStream::connect(record.loopback_addr().expect("v5 loopback address"))
            .expect("connect v5 daemon");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("bound v5 response read");
        let reader = BufReader::new(stream.try_clone().expect("clone v5 stream"));
        (stream, reader)
    };

    // One byte above the handshake bound: every byte is consumed before the
    // refusal, so the answer is not lost to a reset.
    let (mut stream, mut reader) = connect();
    stream
        .write_all(&padded_line(&hello, MAX_V5_HANDSHAKE_LINE_BYTES + 1))
        .expect("write oversized handshake");
    let refused = read_bounded_v5_probe_response_frame(&mut reader).expect("read refusal");
    assert_eq!(
        decode_v5_server_response(&refused),
        Ok(V5ServerResponse::Error {
            code: V5DaemonErrorCode::InvalidRequest
        })
    );
    drop((stream, reader));

    let (mut stream, mut reader) = connect();
    write_json_line(&mut stream, &hello);
    read_bounded_v5_probe_response_frame(&mut reader).expect("read v5 ready");
    let large = MAX_V5_HANDSHAKE_LINE_BYTES * 64;
    assert!(large < MAX_V5_REQUEST_LINE_BYTES);
    stream
        .write_all(&padded_line(&json!({"kind": "ping"}), large))
        .expect("write large authenticated frame");
    let pong = read_bounded_v5_probe_response_frame(&mut reader).expect("read v5 pong");
    let pong: V5ProbeServerResponse = serde_json::from_slice(&pong).expect("decode strict v5 pong");
    assert_eq!(pong.kind(), V5ProbeResponseKind::Pong);
    write_json_line(&mut stream, &json!({"kind": "release"}));
    read_bounded_v5_probe_response_frame(&mut reader).expect("read v5 release response");
    drop((stream, reader));
    server.join().expect("join v5 runtime").expect("v5 runtime");
}

/// A large frame that begins just before an idle session poll expires keeps
/// its consumed prefix: the frame has its own deadline from its first byte,
/// and its tail is never read as a new request (#1309).
#[test]
fn a_frame_straddling_the_idle_session_poll_is_read_whole() {
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
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("bound v5 response read");
    let mut reader = BufReader::new(stream.try_clone().expect("clone v5 stream"));
    write_json_line(
        &mut stream,
        &json!({
            "kind": "hello",
            "protocolVersion": 5,
            "token": record.token(),
            "coreIdentity": identity.as_str(),
            "ownerLease": "55555555-5555-4555-8555-555555555555"
        }),
    );
    read_bounded_v5_probe_response_frame(&mut reader).expect("read v5 ready");
    let ready_at = Instant::now();

    let mut frame = serde_json::to_vec(&json!({"kind": "ping"})).expect("serialize ping");
    frame.resize(1024 * 1024, b' ');
    frame.push(b'\n');
    let parts: Vec<&[u8]> = frame.chunks(frame.len() / 4 + 1).collect();
    // The first part lands shortly before the idle poll ends, the rest after.
    thread::sleep(SESSION_READ_TIMEOUT.saturating_sub(Duration::from_millis(500)));
    for part in parts {
        stream.write_all(part).expect("write frame part");
        thread::sleep(Duration::from_millis(250));
    }
    assert!(ready_at.elapsed() > SESSION_READ_TIMEOUT);
    let answer = read_bounded_v5_probe_response_frame(&mut reader).expect("read v5 answer");
    let answer: V5ProbeServerResponse =
        serde_json::from_slice(&answer).expect("decode strict v5 answer");
    assert_eq!(answer.kind(), V5ProbeResponseKind::Pong);
    write_json_line(&mut stream, &json!({"kind": "release"}));
    read_bounded_v5_probe_response_frame(&mut reader).expect("read v5 release response");
    drop((stream, reader));
    server.join().expect("join v5 runtime").expect("v5 runtime");
}

/// A session frame that misses its own deadline ends the session: its tail
/// is never answered as another request.
#[test]
fn a_session_frame_missing_its_deadline_closes_the_session() {
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
        .set_read_timeout(Some(SESSION_FRAME_READ_TIMEOUT + Duration::from_secs(3)))
        .expect("bound v5 response read");
    let mut reader = BufReader::new(stream.try_clone().expect("clone v5 stream"));
    write_json_line(
        &mut stream,
        &json!({
            "kind": "hello",
            "protocolVersion": 5,
            "token": record.token(),
            "coreIdentity": identity.as_str(),
            "ownerLease": "66666666-6666-4666-8666-666666666666"
        }),
    );
    read_bounded_v5_probe_response_frame(&mut reader).expect("read v5 ready");
    // `{"kind":"ping"}` split so that its tail alone is a valid frame.
    stream.write_all(b"{\"a\":").expect("write frame head");
    thread::sleep(SESSION_FRAME_READ_TIMEOUT + Duration::from_millis(300));
    let _ = stream.write_all(b"1}\n{\"kind\":\"ping\"}\n");
    let mut rest = Vec::new();
    let _ = reader.read_to_end(&mut rest);
    assert!(
        rest.is_empty(),
        "an expired frame must close the session without an answer ({} bytes)",
        rest.len()
    );
    drop((stream, reader));
    server.join().expect("join v5 runtime").expect("v5 runtime");
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

fn probe_peer_cannot_reply(stream: &TcpStream) -> io::Result<bool> {
    if let Some(error) = stream.take_error()? {
        return match error.kind() {
            io::ErrorKind::ConnectionReset | io::ErrorKind::NotConnected => Ok(true),
            _ => Err(error),
        };
    }
    stream.set_nonblocking(true)?;
    match stream.peek(&mut [0u8; 1]) {
        Ok(0) => Ok(true),
        Ok(_) => Ok(false),
        Err(error) => match error.kind() {
            io::ErrorKind::ConnectionReset | io::ErrorKind::NotConnected => Ok(true),
            io::ErrorKind::WouldBlock => Ok(false),
            _ => Err(error),
        },
    }
}

fn displaced_owner_probe(mut stream: TcpStream, hello: &serde_json::Value) -> io::Result<bool> {
    if let Err(setup_error) = stream.set_read_timeout(Some(Duration::from_secs(1))) {
        // Darwin can reject SO_RCVTIMEO after a real reset. Observe the
        // transport; this does not establish process death or release authority.
        return match probe_peer_cannot_reply(&stream) {
            Ok(true) => Ok(false),
            Ok(false) => Err(setup_error),
            Err(observation_error) => Err(io::Error::new(
                setup_error.kind(),
                format!("probe timeout setup failed: {setup_error}; closure observation failed: {observation_error}"),
            )),
        };
    }
    if serde_json::to_writer(&mut stream, hello).is_ok() && stream.write_all(b"\n").is_ok() {
        let mut reader = BufReader::new(stream.try_clone()?);
        Ok(read_bounded_v5_probe_response_frame(&mut reader).is_ok())
    } else {
        Ok(false)
    }
}

#[test]
fn displaced_owner_probe_rejects_a_connection_reset_by_listener_shutdown() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind backlog probe fixture");
    let stream = TcpStream::connect(listener.local_addr().unwrap()).expect("connect pending probe");
    stream
        .set_read_timeout(Some(Duration::from_secs(1)))
        .expect("bound closure observation");
    drop(listener);
    match stream.peek(&mut [0u8; 1]) {
        Ok(0) => {}
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::ConnectionReset | io::ErrorKind::NotConnected
            ) => {}
        other => panic!("listener shutdown must actually close this pending connection: {other:?}"),
    }
    assert!(
        !displaced_owner_probe(stream, &json!({"kind":"hello"})).expect("probe actual reset"),
        "a physically reset peer cannot become ready"
    );
}

#[test]
fn displaced_owner_probe_observes_a_live_peer_response() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind live probe fixture");
    let stream = TcpStream::connect(listener.local_addr().unwrap()).expect("connect live probe");
    let (accepted, _) = listener.accept().expect("accept live probe");
    let server = thread::spawn(move || {
        let mut reader = BufReader::new(accepted);
        reader
            .get_ref()
            .set_read_timeout(Some(Duration::from_secs(1)))
            .expect("bound fixture hello");
        let mut hello = String::new();
        reader
            .read_line(&mut hello)
            .expect("read actual probe hello");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&hello).unwrap(),
            json!({"kind":"hello"})
        );
        reader
            .get_mut()
            .write_all(b"{\"kind\":\"ready\"}\n")
            .expect("send actual probe response");
    });
    let ready = displaced_owner_probe(stream, &json!({"kind":"hello"}));
    server.join().expect("join actual live probe");
    assert!(
        ready.expect("probe live peer"),
        "live response must not be classified as a closed peer"
    );
}

#[test]
fn probe_peer_without_a_response_or_with_payload_is_not_observed_closed() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind live closure observation");
    let stream = TcpStream::connect(listener.local_addr().unwrap()).expect("connect live peer");
    let (mut accepted, _) = listener.accept().expect("accept live peer");
    assert!(!probe_peer_cannot_reply(&stream).expect("observe idle live peer"));
    stream
        .set_nonblocking(false)
        .expect("restore fixture observation");
    stream
        .set_read_timeout(Some(Duration::from_secs(1)))
        .expect("bound payload observation");
    accepted.write_all(b"x").expect("send actual peer payload");
    assert_eq!(
        stream
            .peek(&mut [0u8; 1])
            .expect("observe delivered payload"),
        1
    );
    assert!(!probe_peer_cannot_reply(&stream).expect("observe live peer payload"));
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
                    Ok(stream) => displaced_owner_probe(
                        stream,
                        &json!({
                            "kind": "hello",
                            "protocolVersion": 5,
                            "token": record.token(),
                            "coreIdentity": identity.as_str(),
                            "ownerLease": "33333333-3333-4333-8333-333333333333"
                        }),
                    ),
                    Err(_) => Ok(false),
                };
            let server_result = server.join().expect("join displaced v5 runtime");
            let displaced_still_ready = displaced_still_ready.expect("probe displaced daemon");
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

/// A workspace with one Platform XML configuration and a catalog to view.
fn catalog_workspace() -> (tempfile::TempDir, std::path::PathBuf) {
    let workspace = tempfile::tempdir().expect("temporary workspace");
    let root = std::fs::canonicalize(workspace.path()).expect("physical workspace");
    std::fs::create_dir_all(root.join("src/Catalogs")).expect("create source root");
    std::fs::write(
        root.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .expect("write workspace descriptor");
    std::fs::write(
        root.join("src/Configuration.xml"),
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Store</Name></Properties><ChildObjects><Catalog>Items</Catalog></ChildObjects></Configuration></MetaDataObject>"#,
    )
    .expect("write configuration root");
    std::fs::write(
        root.join("src/Catalogs/Items.xml"),
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog uuid="aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"><Properties><Name>Items</Name></Properties><ChildObjects/></Catalog></MetaDataObject>"#,
    )
    .expect("write catalog");
    (workspace, root)
}

fn view_catalog_request(task_id: TaskId, workspace: &std::path::Path) -> V5InvocationRequest {
    V5InvocationRequest::new(
        InvocationId::new(),
        task_id,
        V5ToolIdentity::View,
        serde_json::Map::from_iter([(
            "at".to_owned(),
            serde_json::Value::String("main:Catalog.Items".to_owned()),
        )]),
        unica_bootstrap::ResolvedWorkspace::launch_cwd(workspace.to_string_lossy().into_owned()),
        7_000,
    )
    .expect("valid view invocation")
}

/// Wait for a Task terminal in short client polls, up to `limit`.
fn wait_task_terminal(
    owner: &mut V5DaemonProcessOwner,
    task_id: TaskId,
    limit: Duration,
) -> V5DaemonTaskSnapshot {
    let until = Instant::now() + limit;
    loop {
        // A fresh session per poll: a session idle across a moved daemon
        // clock is not what this helper tests.
        let mut peer = owner
            .connect_peer_before(Instant::now() + Duration::from_secs(2))
            .expect("connect a polling session");
        let response = peer.wait_task(task_id, 1_000).expect("poll the Task");
        let V5ServerResponse::Task { snapshot } = response else {
            panic!("Task poll returned a non-Task response: {response:?}");
        };
        if !matches!(
            snapshot,
            V5DaemonTaskSnapshot::Queued { .. } | V5DaemonTaskSnapshot::Working { .. }
        ) {
            return snapshot;
        }
        assert!(Instant::now() < until, "Task stayed open past {limit:?}");
    }
}

/// The real canonical read service behind a provider that answers later than
/// every former automatic deadline: the 120 s logical read budget, the 30 s
/// terminal publication bound and the 2 s admission grace.
struct SlowRealReadService {
    inner: crate::infrastructure::daemon::v13_service::CanonicalV13ReadService,
    delay: Duration,
    executions: AtomicUsize,
    entered: Mutex<mpsc::Sender<()>>,
}

impl CanonicalInvocationService for SlowRealReadService {
    fn prepare(
        &self,
        invocation: &crate::infrastructure::daemon::server::ActorBoundInvocation,
    ) -> Result<ExecutionClass, Box<DomainResult>> {
        self.inner.prepare(invocation)
    }

    fn execute(
        &self,
        invocation: &crate::infrastructure::daemon::server::ActorBoundExecution,
        cancellation: CancellationToken,
    ) -> Result<DomainResult, InvocationFailure> {
        self.executions.fetch_add(1, Ordering::SeqCst);
        let _ = self.entered.lock().unwrap().send(());
        let until = Instant::now() + self.delay;
        while Instant::now() < until {
            if cancellation.is_cancelled() {
                return Err(InvocationFailure::new("cancelled", "slow read cancelled"));
            }
            thread::sleep(Duration::from_millis(50));
        }
        self.inner.execute(invocation, cancellation)
    }
}

/// #1251: a read that answers after 125 s of real time completes through its
/// Task on both continuation paths — an inline execution handed off at the
/// seventh second, and an admission that outlived the handoff moment and
/// runs prepare and execute on the promised Task's continuation. The
/// daemon's handoff clock is moved by the test; the delay itself is real, so
/// every former wall-clock budget of the read and of its publication passes.
#[test]
fn promoted_reads_run_past_former_deadlines_and_complete_through_their_tasks() {
    let state = tempfile::tempdir().expect("temporary daemon state");
    let state_root = std::fs::canonicalize(state.path()).expect("physical state root");
    let (_inline_workspace, inline_root) = catalog_workspace();
    let (_admitted_workspace, admitted_root) = catalog_workspace();
    let admission =
        crate::infrastructure::daemon::server::admission_test_control::AdmissionPause::install(
            admitted_root.clone(),
        );
    let clock = Arc::new(InspectionClock {
        start: Instant::now(),
        elapsed_ms: AtomicU64::new(0),
    });
    let (entered_tx, entered_rx) = mpsc::channel();
    let service = Arc::new(SlowRealReadService {
        inner: crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default(),
        delay: Duration::from_secs(125),
        executions: AtomicUsize::new(0),
        entered: Mutex::new(entered_tx),
    });
    let canonical_runtime = Arc::new(V5CanonicalInvocationRuntime::new(
        service.clone(),
        clock.clone(),
    ));
    let hooks = Arc::new(InspectionHooks::default());
    let identity = CoreIdentity::production_v5();
    let config = DaemonServerConfig::new(
        state_root.clone(),
        identity.clone(),
        Duration::from_secs(600),
    )
    .with_canonical_runtime_for_test(canonical_runtime)
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
    wait_for_v5_record(&state_root, &identity);
    let owner = V5DaemonProcessOwner::connect_or_spawn(
        &state_root,
        identity,
        std::path::PathBuf::from("unused-existing-v5-endpoint"),
        Duration::from_secs(2),
    )
    .expect("connect v5 owner");

    // Inline: the execution starts before the handoff moment and is handed
    // off to its Task when the daemon clock reaches the seventh second.
    let inline_task = TaskId::new();
    let inline_request = view_catalog_request(inline_task, &inline_root);
    let inline_submit = thread::spawn(move || {
        let mut owner = owner;
        let response = owner.submit_invocation(inline_request);
        (owner, response)
    });
    entered_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("the inline execution starts");
    clock.elapsed_ms.store(7_000, Ordering::SeqCst);
    let (mut owner, inline) = inline_submit.join().expect("inline submit thread");
    assert!(
        matches!(
            inline.expect("the inline read answers at the handoff moment"),
            V5ServerResponse::Invocation {
                outcome: V5InvocationResponse::Task { .. }
            }
        ),
        "an unfinished inline read hands off as a Task"
    );

    // Admitted: the admission itself outlives the handoff moment, and the
    // promised Task's continuation prepares and executes the read.
    let admitted_task = TaskId::new();
    let admitted_request = view_catalog_request(admitted_task, &admitted_root);
    let mut admitted_owner = owner
        .connect_peer_before(Instant::now() + Duration::from_secs(2))
        .expect("connect a second session");
    let admitted_submit = thread::spawn(move || {
        let response = admitted_owner.submit_invocation(admitted_request);
        (admitted_owner, response)
    });
    admission.wait_until_entered();
    clock.elapsed_ms.store(14_000, Ordering::SeqCst);
    let (_admitted_owner, admitted) = admitted_submit.join().expect("admitted submit thread");
    assert!(
        matches!(
            admitted.expect("the admission past the handoff moment answers"),
            V5ServerResponse::Invocation {
                outcome: V5InvocationResponse::Task { .. }
            }
        ),
        "an admission past the handoff moment returns its promised Task"
    );
    clock
        .elapsed_ms
        .store(FAR_PAST_FORMER_DEADLINES_MS, Ordering::SeqCst);
    admission.release();

    for (label, task_id) in [("inline", inline_task), ("admitted", admitted_task)] {
        let terminal = wait_task_terminal(&mut owner, task_id, Duration::from_secs(300));
        let V5DaemonTaskSnapshot::Completed { result, .. } = terminal else {
            panic!("{label} read did not complete: {terminal:?}");
        };
        assert!(result.ok, "{label} read failed after the delay: {result:?}");
        assert!(
            result.data.is_some(),
            "{label} read lost its source data: {result:?}"
        );
    }
    assert_eq!(service.executions.load(Ordering::SeqCst), 2);
    assert!(!hooks.fail_stopped.load(Ordering::SeqCst));
    drop(owner);
    drop(admission);
    stop.store(true, Ordering::SeqCst);
    server
        .join()
        .expect("daemon thread did not panic")
        .expect("daemon exits cleanly");
}

/// A known-long execution that ignores its cancellation for one address and
/// answers every other at once.
struct NoncooperativeStuckService {
    entered: mpsc::Sender<()>,
    release: Mutex<mpsc::Receiver<()>>,
}

impl CanonicalInvocationService for NoncooperativeStuckService {
    fn prepare(
        &self,
        _invocation: &crate::infrastructure::daemon::server::ActorBoundInvocation,
    ) -> Result<ExecutionClass, Box<DomainResult>> {
        Ok(ExecutionClass::KnownLong(KnownLongReason::ExternalProcess))
    }

    fn execute(
        &self,
        invocation: &crate::infrastructure::daemon::server::ActorBoundExecution,
        _cancellation: CancellationToken,
    ) -> Result<DomainResult, InvocationFailure> {
        if invocation.arguments().get("at") == Some(&json!("main:Catalog.Stuck")) {
            self.entered.send(()).expect("report the stuck execution");
            self.release
                .lock()
                .unwrap()
                .recv()
                .expect("release the stuck execution");
        }
        Ok(DomainResult::success("answered"))
    }
}

/// #1251, owner decision: an executor that does not answer its cancel keeps
/// only its own Task. The Task shows the requested cancel, the daemon keeps
/// serving other calls past the former two-second fail-stop, and the late
/// executor still ends as cancelled.
#[test]
fn noncooperative_cancel_keeps_only_its_task_while_the_daemon_serves_others() {
    let state = tempfile::tempdir().expect("temporary daemon state");
    let state_root = std::fs::canonicalize(state.path()).expect("physical state root");
    let (_workspace, root) = catalog_workspace();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let identity = CoreIdentity::production_v5();
    let config = DaemonServerConfig::new(
        state_root.clone(),
        identity.clone(),
        Duration::from_secs(60),
    )
    .with_invocation_service(Arc::new(NoncooperativeStuckService {
        entered: entered_tx,
        release: Mutex::new(release_rx),
    }));
    let stop = Arc::new(AtomicBool::new(false));
    let server_stop = stop.clone();
    let server = thread::spawn(move || {
        run_daemon_configured_until(
            config,
            |runtime| runtime,
            || server_stop.load(Ordering::SeqCst),
        )
    });
    wait_for_v5_record(&state_root, &identity);
    let mut owner = V5DaemonProcessOwner::connect_or_spawn(
        &state_root,
        identity,
        std::path::PathBuf::from("unused-existing-v5-endpoint"),
        Duration::from_secs(2),
    )
    .expect("connect v5 owner");
    let stuck_task = TaskId::new();
    let stuck = V5InvocationRequest::new(
        InvocationId::new(),
        stuck_task,
        V5ToolIdentity::View,
        serde_json::Map::from_iter([(
            "at".to_owned(),
            serde_json::Value::String("main:Catalog.Stuck".to_owned()),
        )]),
        unica_bootstrap::ResolvedWorkspace::launch_cwd(root.to_string_lossy().into_owned()),
        7_000,
    )
    .expect("valid stuck invocation");
    owner
        .submit_invocation(stuck)
        .expect("submit the stuck read");
    entered_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("the stuck execution starts");
    owner
        .cancel_task(stuck_task)
        .expect("cancel the stuck Task");
    // Past the former two-second grace after the cancel.
    thread::sleep(Duration::from_millis(2_500));

    let V5ServerResponse::Task { snapshot } =
        owner.get_task(stuck_task).expect("read the stuck Task")
    else {
        panic!("stuck Task is unreadable");
    };
    assert!(
        matches!(
            snapshot,
            V5DaemonTaskSnapshot::Working {
                cancel_requested: true,
                ..
            }
        ),
        "the stuck Task stays visible with its cancel requested: {snapshot:?}"
    );
    let other_task = TaskId::new();
    owner
        .submit_invocation(view_catalog_request(other_task, &root))
        .expect("the daemon accepts another call");
    let other = wait_task_terminal(&mut owner, other_task, Duration::from_secs(30));
    assert!(
        matches!(&other, V5DaemonTaskSnapshot::Completed { result, .. } if result.ok),
        "another Task completes while the stuck one ignores its cancel: {other:?}"
    );

    release_tx.send(()).expect("release the stuck execution");
    let stuck_terminal = wait_task_terminal(&mut owner, stuck_task, Duration::from_secs(30));
    assert!(
        matches!(stuck_terminal, V5DaemonTaskSnapshot::Cancelled { .. }),
        "the late executor still ends as cancelled: {stuck_terminal:?}"
    );
    drop(owner);
    stop.store(true, Ordering::SeqCst);
    server
        .join()
        .expect("daemon thread did not panic")
        .expect("daemon exits cleanly");
}

/// #1251: a cancel that reaches an inline call still in admission, before
/// the handoff moment, ends it. Without an admission deadline the token is
/// the only stop; the attempt publishes its own cancelled terminal.
#[test]
fn cancel_during_inline_admission_before_handoff_publishes_cancelled() {
    let state = tempfile::tempdir().unwrap();
    let state_root = std::fs::canonicalize(state.path()).unwrap();
    let (_workspace, workspace_root) = catalog_workspace();
    let admission =
        crate::infrastructure::daemon::server::admission_test_control::AdmissionPause::install(
            workspace_root.clone(),
        );
    let clock = Arc::new(InspectionClock {
        start: Instant::now(),
        elapsed_ms: AtomicU64::new(0),
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
    let server = thread::spawn(move || {
        run_daemon_configured_until(
            config,
            |runtime| runtime,
            || server_stop.load(Ordering::SeqCst),
        )
    });
    wait_for_v5_record(&state_root, &identity);
    let owner = V5DaemonProcessOwner::connect_or_spawn(
        &state_root,
        identity.clone(),
        std::path::PathBuf::from("unused-existing-v5-endpoint"),
        Duration::from_secs(2),
    )
    .unwrap();
    let invocation = view_catalog_request(TaskId::new(), &workspace_root);
    let key = ReceiptKey::new(
        invocation.invocation_id(),
        invocation.reserved_task_id(),
        RequestIdentity::new(
            identity.digest().clone(),
            V5ToolIdentity::View,
            normalized_arguments_hash(invocation.arguments()),
            request_scope_hash(invocation.workspace_hint()).unwrap(),
        ),
    );
    let mut canceller = owner
        .connect_peer_before(Instant::now() + Duration::from_secs(2))
        .unwrap();
    let submit = thread::spawn(move || {
        let mut owner = owner;
        let response = owner.submit_invocation(invocation);
        (owner, response)
    });
    admission.wait_until_entered();
    canceller
        .cancel_invocation(key)
        .expect("cancel the inline call while it is admitted");
    admission.release();
    let (owner, submitted) = submit.join().unwrap();
    let V5ServerResponse::Invocation {
        outcome: V5InvocationResponse::Direct { receipt },
    } = submitted.expect("the cancelled inline call answers directly")
    else {
        panic!("a cancelled inline call must end directly");
    };
    assert!(
        matches!(receipt.terminal(), ReceiptTerminalOutcome::Cancelled),
        "the inline call must end as cancelled: {:?}",
        receipt.terminal()
    );
    assert_eq!(
        admission.entries(),
        1,
        "the admission walk must stop at its first checkpoint after the cancel"
    );
    assert_eq!(service.prepares.load(Ordering::SeqCst), 0);
    assert!(!hooks.fail_stopped.load(Ordering::SeqCst));
    drop(owner);
    drop(canceller);
    drop(admission);
    stop.store(true, Ordering::SeqCst);
    server
        .join()
        .expect("daemon thread did not panic")
        .expect("daemon exits cleanly");
}

/// Fails one retirement step for one record, for as long as it is armed.
struct RetirementStepFault {
    step: V5RetirementStep,
    target: Mutex<Option<TaskId>>,
    armed: AtomicBool,
    injected: AtomicUsize,
    /// Runs in place of the failed step before it reports the failure.
    on_fault: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
}

impl RetirementStepFault {
    fn at(step: V5RetirementStep) -> Self {
        Self {
            step,
            target: Mutex::new(None),
            armed: AtomicBool::new(true),
            injected: AtomicUsize::new(0),
            on_fault: Mutex::new(None),
        }
    }
}

impl V5RuntimeHooks for RetirementStepFault {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn retirement_step_fault(&self, step: V5RetirementStep, task_id: TaskId) -> bool {
        let fails = step == self.step
            && self.armed.load(Ordering::SeqCst)
            && *self.target.lock().expect("retirement fault target") == Some(task_id);
        if fails {
            self.injected.fetch_add(1, Ordering::SeqCst);
            if let Some(on_fault) = &*self.on_fault.lock().expect("retirement fault action") {
                on_fault();
            }
        }
        fails
    }
}

const RETIREMENT_TEST_TTL_MS: u64 = 3_600_000;
/// Every Task created at the start of the test has outlived its TTL here,
/// while the live Task completed shortly before.
const RETIREMENT_TEST_OBSERVED_EPOCH_MS: u64 = 4_000_000;

/// Creates a Task of `workspace` through the real receipt and Task stores and
/// completes it with `summary` at `terminal_epoch_ms`.
fn materialize_completed_task(
    runtime: &V5ReceiptRuntime,
    identity: &CoreIdentity,
    clock: &ManualEpochClock,
    workspace: &str,
    created_epoch_ms: u64,
    terminal_epoch_ms: u64,
    summary: &str,
) -> TaskId {
    materialize_completed_task_key(
        runtime,
        identity,
        clock,
        workspace,
        created_epoch_ms,
        terminal_epoch_ms,
        summary,
    )
    .reserved_task_id()
}

/// [`materialize_completed_task`], answering with the call's full key.
fn materialize_completed_task_key(
    runtime: &V5ReceiptRuntime,
    identity: &CoreIdentity,
    clock: &ManualEpochClock,
    workspace: &str,
    created_epoch_ms: u64,
    terminal_epoch_ms: u64,
    summary: &str,
) -> ReceiptKey {
    clock.set(created_epoch_ms);
    let key = ReceiptKey::new(
        InvocationId::new(),
        TaskId::new(),
        RequestIdentity::new(
            identity.digest().clone(),
            V5ToolIdentity::View,
            normalized_arguments_hash(&serde_json::Map::new()),
            request_scope_hash(workspace).expect("request scope"),
        ),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    let reserved = runtime
        .receipt_ledger
        .reserve(
            key.clone(),
            OriginalCutoffDescriptor::new(created_epoch_ms, 6_000).expect("valid cutoff"),
            deadline,
        )
        .expect("reserve Task receipt")
        .into_reservation()
        .expect("new Task receipt");
    let actor_bound = runtime
        .receipt_ledger
        .bind_reserved_actor(
            key.clone(),
            reserved.record_version(),
            SafeIdentityHash::from_sha256(Sha256::digest(workspace.as_bytes()).into()),
            deadline,
        )
        .expect("bind Task actor");
    let begun = runtime
        .receipt_ledger
        .mark_reserved_begun(key.clone(), actor_bound.record_version(), deadline)
        .expect("mark Task attempt begun");
    let handoff = runtime
        .receipt_ledger
        .begin_bound_task_handoff(
            key.clone(),
            begun.record_version(),
            created_epoch_ms,
            RETIREMENT_TEST_TTL_MS,
            V5_TASK_POLL_INTERVAL_MS,
            deadline,
        )
        .expect("begin Task handoff");
    let (record, task_bound) = runtime
        .task_projection
        .materialize_bound_handoff(&handoff, created_epoch_ms, deadline, runtime.hooks.as_ref())
        .unwrap_or_else(|failure| panic!("materialize TaskBound: {}", failure.error));
    let task_bound = runtime
        .receipt_ledger
        .complete_bound_task_handoff(key.clone(), handoff.record_version(), task_bound, deadline)
        .expect("complete TaskBound ownership");
    clock.set(terminal_epoch_ms);
    let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Completed {
        result: Box::new(DomainResult::success(summary)),
    })
    .expect("canonical completed terminal");
    runtime
        .task_projection
        .publish_bound_task_terminal(
            &task_bound,
            &record,
            &terminal,
            terminal_epoch_ms,
            deadline,
            runtime.hooks.as_ref(),
        )
        .unwrap_or_else(|failure| panic!("publish completed Task: {}", failure.error));
    key
}

fn retirement_link(runtime: &V5ReceiptRuntime, task_id: TaskId) -> Option<TaskLifecycleLinkRecord> {
    let deadline = crate::domain::code_intelligence::ProviderDeadline::new(
        Instant::now() + Duration::from_secs(5),
    );
    match runtime
        .task_projection
        .lifecycle_links
        .read_by_task_id(task_id, deadline)
    {
        Ok(link) => Some(link),
        Err(TaskLifecycleLinkStoreError::NotFound { .. }) => None,
        Err(error) => panic!("read lifecycle link of {task_id}: {error}"),
    }
}

fn retirement_task_present(runtime: &V5ReceiptRuntime, task_id: TaskId) -> bool {
    let deadline = crate::domain::code_intelligence::ProviderDeadline::new(
        Instant::now() + Duration::from_secs(5),
    );
    match runtime.task_projection.task_store.get(task_id, deadline) {
        Ok(_) => true,
        Err(V5TaskStoreError::NotFound { .. }) => false,
        Err(error) => panic!("read Task {task_id}: {error}"),
    }
}

/// `tasks/get`, `tasks/result` (a wait) and `tasks/cancel` of the live Task
/// all answer its completed result, and the process keeps admitting work.
fn assert_live_task_answers(runtime: &V5ReceiptRuntime, live: TaskId, context: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    for (operation, answer) in [
        ("get", runtime.resolve_task(live, deadline)),
        ("wait", runtime.wait_task(live, 0, deadline)),
        ("cancel", runtime.cancel_task(live, deadline)),
    ] {
        match answer {
            Ok(V5DaemonTaskSnapshot::Completed { result, .. }) => assert!(
                result.ok && result.summary == "live result",
                "{context}: {operation} returned another result: {result:?}"
            ),
            other => panic!("{context}: {operation} of the live Task failed: {other:?}"),
        }
    }
    assert!(
        !runtime.restart_required(),
        "{context}: a retirement failure closed the daemon for every Task"
    );
}

/// An accumulated store: two expired Tasks of this workspace, one expired
/// Task of another workspace copy that cannot be retired, and a live Task.
struct AccumulatedRetirementStore {
    _root: tempfile::TempDir,
    runtime: Arc<V5ReceiptRuntime>,
    clock: Arc<ManualEpochClock>,
    expired: Vec<TaskId>,
    foreign: TaskId,
    live: TaskId,
}

fn accumulated_retirement_store(hooks: Arc<dyn V5RuntimeHooks>) -> AccumulatedRetirementStore {
    let root = tempfile::tempdir().expect("temporary retirement state root");
    let state_root = std::fs::canonicalize(root.path()).expect("physical retirement state root");
    let identity = CoreIdentity::production_v5();
    let clock = Arc::new(ManualEpochClock::new(1_000));
    let config = DaemonServerConfig::new(
        state_root.clone(),
        identity.clone(),
        Duration::from_millis(50),
    )
    .with_v5_epoch_clock_for_test(clock.clone())
    .with_runtime_hooks_for_test(hooks);
    let state =
        DaemonStateDirectory::open(&state_root, &identity).expect("open retirement daemon state");
    let runtime = V5ReceiptRuntime::open(&state, &config).expect("open retirement runtime");
    let first = materialize_completed_task(
        &runtime,
        &identity,
        &clock,
        "workspace-a",
        1_000,
        2_000,
        "expired a",
    );
    let foreign = materialize_completed_task(
        &runtime,
        &identity,
        &clock,
        "workspace-other-copy",
        1_100,
        2_100,
        "expired foreign",
    );
    let second = materialize_completed_task(
        &runtime,
        &identity,
        &clock,
        "workspace-a",
        1_200,
        2_200,
        "expired b",
    );
    let live = materialize_completed_task(
        &runtime,
        &identity,
        &clock,
        "workspace-a",
        3_900_000,
        3_950_000,
        "live result",
    );
    clock.set(RETIREMENT_TEST_OBSERVED_EPOCH_MS);
    AccumulatedRetirementStore {
        _root: root,
        runtime: Arc::new(runtime),
        clock,
        expired: vec![first, second],
        foreign,
        live,
    }
}

/// #863: a record whose retirement fails at `step` stays with its committed
/// state; the live Task is answered, the other expired records are retired,
/// the failed one is not retried on every poll and is retired after its pause.
fn assert_unretirable_record_stays_confined(step: V5RetirementStep) {
    let hooks = Arc::new(RetirementStepFault::at(step));
    let store = accumulated_retirement_store(hooks.clone());
    let runtime = &store.runtime;
    *hooks.target.lock().expect("retirement fault target") = Some(store.foreign);

    for poll in 0..3 {
        assert_live_task_answers(runtime, store.live, &format!("{step:?} poll {poll}"));
    }
    assert_eq!(
        hooks.injected.load(Ordering::SeqCst),
        1,
        "{step:?}: the failed record must wait for its pause instead of failing every poll"
    );
    for task_id in &store.expired {
        assert!(
            retirement_link(runtime, *task_id).is_none()
                && !retirement_task_present(runtime, *task_id),
            "{step:?}: an expired record next to the failed one was not retired"
        );
    }
    let link = retirement_link(runtime, store.foreign);
    match step {
        V5RetirementStep::Begin => assert!(
            matches!(link, Some(TaskLifecycleLinkRecord::TaskTerminalBound(_))),
            "{step:?}: the failed record lost its terminal link: {link:?}"
        ),
        _ => assert!(
            matches!(
                link,
                Some(TaskLifecycleLinkRecord::TaskRetirementPending(_))
            ),
            "{step:?}: the failed record lost its committed retirement intent: {link:?}"
        ),
    }
    assert_eq!(
        retirement_task_present(runtime, store.foreign),
        step != V5RetirementStep::Finalize,
        "{step:?}: the failed record changed its Task beyond the failed step"
    );
    // Reading the record that could not be retired does not close the daemon
    // either, even when its Task is already deleted and only the link remains.
    let foreign = runtime.resolve_task(store.foreign, Instant::now() + Duration::from_secs(5));
    if step == V5RetirementStep::Finalize {
        assert!(
            matches!(foreign, Err(ReceiptLedgerError::ReceiptNotFound)),
            "{step:?}: a deleted Task under its retirement intent must read as absent: {foreign:?}"
        );
    } else {
        assert!(
            matches!(&foreign, Ok(V5DaemonTaskSnapshot::Completed { result, .. })
                if result.summary == "expired foreign"),
            "{step:?}: the record that could not be retired lost its result: {foreign:?}"
        );
    }
    assert_live_task_answers(
        runtime,
        store.live,
        &format!("{step:?} after the failed record"),
    );

    hooks.armed.store(false, Ordering::SeqCst);
    store
        .clock
        .set(RETIREMENT_TEST_OBSERVED_EPOCH_MS + RETIREMENT_RETRY_INITIAL_DELAY_MS);
    assert_live_task_answers(runtime, store.live, &format!("{step:?} retry"));
    assert!(
        retirement_link(runtime, store.foreign).is_none()
            && !retirement_task_present(runtime, store.foreign),
        "{step:?}: the failed record was not retired after its pause"
    );
}

#[test]
fn retirement_begin_failure_stays_with_its_record() {
    assert_unretirable_record_stays_confined(V5RetirementStep::Begin);
}

#[test]
fn retirement_authorize_failure_stays_with_its_record() {
    assert_unretirable_record_stays_confined(V5RetirementStep::Authorize);
}

#[test]
fn retirement_delete_failure_stays_with_its_record() {
    assert_unretirable_record_stays_confined(V5RetirementStep::Delete);
}

#[test]
fn retirement_finalize_failure_stays_with_its_record() {
    assert_unretirable_record_stays_confined(V5RetirementStep::Finalize);
}

/// The real store fault: deleting an expired Task returns `CommitUncertain`
/// after the file is gone. The record keeps its intent, the live Task is
/// answered, and the retry reconciles the absent Task.
#[test]
fn uncertain_retirement_delete_stays_with_its_record_and_reconciles_on_retry() {
    let store = accumulated_retirement_store(Arc::new(NoHooks));
    let runtime = &store.runtime;
    runtime
        .task_projection
        .task_store
        .inject_next_publication_failure(PublicationFailure::AfterDeleteBeforeSync);

    assert_live_task_answers(runtime, store.live, "uncertain delete");
    let mut expired = store.expired.clone();
    expired.push(store.foreign);
    let left = expired
        .iter()
        .filter(|task_id| retirement_link(runtime, **task_id).is_some())
        .collect::<Vec<_>>();
    assert_eq!(left.len(), 1, "exactly the uncertain record keeps its link");
    assert!(
        matches!(
            retirement_link(runtime, *left[0]),
            Some(TaskLifecycleLinkRecord::TaskRetirementPending(_))
        ) && !retirement_task_present(runtime, *left[0]),
        "the uncertain delete keeps its committed intent over the deleted Task"
    );
    assert_live_task_answers(runtime, store.live, "uncertain delete, next poll");

    store
        .clock
        .set(RETIREMENT_TEST_OBSERVED_EPOCH_MS + RETIREMENT_RETRY_INITIAL_DELAY_MS);
    assert_live_task_answers(runtime, store.live, "uncertain delete retry");
    for task_id in expired {
        assert!(
            retirement_link(runtime, task_id).is_none()
                && !retirement_task_present(runtime, task_id),
            "the retry did not finish retiring {task_id}"
        );
    }
}

#[test]
fn retirement_retry_pause_doubles_up_to_an_hour_without_an_attempt_limit() {
    assert_eq!(retirement_retry_delay_ms(1), 60_000);
    assert_eq!(retirement_retry_delay_ms(2), 120_000);
    assert_eq!(retirement_retry_delay_ms(6), 1_920_000);
    assert_eq!(retirement_retry_delay_ms(7), RETIREMENT_RETRY_MAX_DELAY_MS);
    assert_eq!(
        retirement_retry_delay_ms(u32::MAX),
        RETIREMENT_RETRY_MAX_DELAY_MS
    );
}

/// The intent to retire reaches the disk, but its commit reports an uncertain
/// outcome and memory keeps the terminal link: what `publish_and_readback`
/// leaves when the directory sync or readback fails after the rename. The
/// process goes on serving the other Tasks from the durable catalog, so later
/// commits keep the intent instead of overwriting it with the stale copy.
#[test]
fn retirement_failure_after_uncertain_link_commit_serves_the_durable_catalog() {
    let hooks = Arc::new(RetirementStepFault::at(V5RetirementStep::Begin));
    let store = accumulated_retirement_store(hooks.clone());
    let runtime = &store.runtime;
    *hooks.target.lock().expect("retirement fault target") = Some(store.foreign);
    let Some(TaskLifecycleLinkRecord::TaskTerminalBound(terminal)) =
        retirement_link(runtime, store.foreign)
    else {
        panic!("the foreign expired Task must be terminal-bound")
    };
    let durable_intent = Arc::new(Mutex::new(None));
    let weak_runtime = Arc::downgrade(runtime);
    let committed = Arc::clone(&durable_intent);
    *hooks.on_fault.lock().expect("retirement fault action") = Some(Box::new(move || {
        let runtime = weak_runtime
            .upgrade()
            .expect("runtime is alive during its read");
        let links = &runtime.task_projection.lifecycle_links;
        let before_commit = links.memory_catalog_for_test();
        let intent = links
            .begin_task_retirement(
                &terminal,
                64,
                64,
                crate::domain::code_intelligence::ProviderDeadline::new(
                    Instant::now() + Duration::from_secs(5),
                ),
            )
            .expect("commit the intent to retire");
        links.restore_memory_catalog_for_test(before_commit);
        *committed.lock().expect("committed intent") = Some(intent);
    }));

    assert_live_task_answers(runtime, store.live, "uncertain link commit");
    assert_eq!(hooks.injected.load(Ordering::SeqCst), 1);
    let durable_intent = durable_intent
        .lock()
        .expect("committed intent")
        .clone()
        .expect("the fault committed the intent");
    assert_eq!(
        retirement_link(runtime, store.foreign),
        Some(TaskLifecycleLinkRecord::TaskRetirementPending(
            durable_intent
        )),
        "the durable intent to retire was replaced by the stale terminal link"
    );
    for task_id in &store.expired {
        assert!(
            retirement_link(runtime, *task_id).is_none()
                && !retirement_task_present(runtime, *task_id),
            "an expired record next to the failed one was not retired"
        );
    }

    hooks.armed.store(false, Ordering::SeqCst);
    store
        .clock
        .set(RETIREMENT_TEST_OBSERVED_EPOCH_MS + RETIREMENT_RETRY_INITIAL_DELAY_MS);
    assert_live_task_answers(runtime, store.live, "uncertain link commit retry");
    assert!(
        retirement_link(runtime, store.foreign).is_none()
            && !retirement_task_present(runtime, store.foreign),
        "the durable intent was not finished after its pause"
    );
}

/// A symbol provider whose index is cold: a search blocks until the test
/// opens the gate, so a call that reached it would cross the response cutoff
/// and come back as a task.
struct ColdIndexSearchProvider {
    calls: Arc<AtomicUsize>,
    warm: Arc<(Mutex<bool>, std::sync::Condvar)>,
}

impl crate::domain::code_intelligence::CodeIntelligenceProvider for ColdIndexSearchProvider {
    fn identity(&self) -> crate::domain::code_intelligence::ProviderIdentity {
        crate::domain::code_intelligence::ProviderIdentity::new(
            crate::domain::code_intelligence::ProviderRole::Symbol,
            "cold-symbol",
        )
    }

    fn capabilities(&self) -> &[crate::domain::code_intelligence::ProviderCapability] {
        &[crate::domain::code_intelligence::ProviderCapability::Search]
    }

    fn search(
        &self,
        _request: &crate::domain::code_intelligence::SearchRequest,
        _context: &crate::domain::code_intelligence::CodeIntelligenceContext,
        _deadline: crate::domain::code_intelligence::ProviderDeadline,
        _cancellation: &CancellationToken,
    ) -> crate::domain::code_intelligence::ProviderSearchSection {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let (warm, changed) = &*self.warm;
        let guard = warm.lock().unwrap();
        let _ = changed
            .wait_timeout_while(guard, Duration::from_secs(30), |warm| !*warm)
            .unwrap();
        crate::domain::code_intelligence::ProviderSearchSection::dependency_pending(
            self.identity(),
            crate::domain::code_intelligence::SearchRanking::Provider,
            crate::domain::code_intelligence::SearchOrdering::Provider,
            vec![],
            vec![],
            "buildingIndex",
        )
        .unwrap()
    }
}

/// #1216: through the daemon a search whose cursor is not a continuation of
/// the question, or whose limit is out of range, is answered directly with a
/// `fixCall` refusal. It creates no task and does not wait for the cold index
/// of the selected provider.
#[test]
fn search_argument_refusal_is_direct_while_the_provider_index_is_cold() {
    let root = tempfile::tempdir().expect("temporary search state root");
    let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
    let workspace = tempfile::tempdir().expect("temporary search workspace");
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
    let calls = Arc::new(AtomicUsize::new(0));
    let warm = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
    let config = DaemonServerConfig::new(
        state_root.clone(),
        identity.clone(),
        Duration::from_millis(80),
    )
    .with_invocation_service(Arc::new(
        crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::with_search_providers_for_test(
            vec![Arc::new(ColdIndexSearchProvider {
                calls: calls.clone(),
                warm: warm.clone(),
            })],
        ),
    ));
    let server = thread::spawn(move || run_daemon(config));
    let _record = wait_for_v5_record(&state_root, &identity);
    let mut owner = V5DaemonProcessOwner::connect_or_spawn(
        &state_root,
        identity,
        std::path::PathBuf::from("unused-existing-v5-endpoint"),
        Duration::from_millis(300),
    )
    .expect("connect v5 owner");
    for (arguments, code) in [
        (
            json!({"query": "Needle", "role": "symbol", "cursor": "не-наш-курсор"}),
            "invalid_cursor",
        ),
        (
            json!({"query": "Needle", "role": "symbol", "cursor": format!("sc2.{}", "0".repeat(244))}),
            "invalid_cursor",
        ),
        (
            json!({"query": "Needle", "role": "symbol", "limit": 51}),
            "bad_value",
        ),
    ] {
        let serde_json::Value::Object(arguments) = arguments else {
            unreachable!("search arguments are an object")
        };
        let invocation = V5InvocationRequest::new(
            InvocationId::new(),
            TaskId::new(),
            V5ToolIdentity::Search,
            arguments.clone(),
            unica_bootstrap::ResolvedWorkspace::launch_cwd(
                workspace.to_string_lossy().into_owned(),
            ),
            7_000,
        )
        .expect("valid search invocation");
        let submitted = owner
            .submit_invocation(invocation)
            .expect("submit search invocation");
        let V5ServerResponse::Invocation {
            outcome: V5InvocationResponse::Direct { receipt },
        } = submitted
        else {
            panic!("{arguments:?} must be answered directly, not as a task: {submitted:?}");
        };
        let ReceiptTerminalOutcome::Completed { result } = receipt.terminal() else {
            panic!("{arguments:?} must complete with a refusal: {receipt:?}");
        };
        assert!(!result.ok, "{result:?}");
        assert_eq!(result.diagnostics[0]["code"], code, "{result:?}");
        assert_eq!(result.diagnostics[0]["outcome"], "fixCall", "{result:?}");
    }
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "a refused search must not ask the cold provider"
    );

    *warm.0.lock().unwrap() = true;
    warm.1.notify_all();
    drop(owner);
    server
        .join()
        .expect("v5 search daemon did not panic")
        .expect("v5 search daemon exited cleanly");
}

/// A known-long read held open until released; counts its prepares and
/// executions.
struct CountedKnownLongService {
    prepares: AtomicUsize,
    executions: AtomicUsize,
    entered: Mutex<mpsc::Sender<()>>,
    release: Mutex<mpsc::Receiver<()>>,
}

impl CanonicalInvocationService for CountedKnownLongService {
    fn prepare(
        &self,
        _invocation: &crate::infrastructure::daemon::server::ActorBoundInvocation,
    ) -> Result<ExecutionClass, Box<DomainResult>> {
        self.prepares.fetch_add(1, Ordering::SeqCst);
        Ok(ExecutionClass::KnownLong(KnownLongReason::ExternalProcess))
    }

    fn execute(
        &self,
        _invocation: &crate::infrastructure::daemon::server::ActorBoundExecution,
        _cancellation: CancellationToken,
    ) -> Result<DomainResult, InvocationFailure> {
        self.executions.fetch_add(1, Ordering::SeqCst);
        let _ = self.entered.lock().unwrap().send(());
        self.release
            .lock()
            .unwrap()
            .recv()
            .expect("release the held execution");
        Ok(DomainResult::success("answered"))
    }
}

fn submitted_task_snapshot(response: Result<V5ServerResponse, String>) -> V5DaemonTaskSnapshot {
    match response {
        Ok(V5ServerResponse::Invocation {
            outcome: V5InvocationResponse::Task { snapshot },
        }) => snapshot,
        other => panic!("the submit must name its Task: {other:?}"),
    }
}

/// #1384: the exact repeat of a submit whose receipt already passed into its
/// Task reads that Task — while it runs and after it ends. The key is never
/// reserved again, nothing is prepared a second time, and the daemon keeps
/// serving. Another key that claims the same Task id is refused.
#[test]
fn exact_repeat_submit_reads_its_bound_task_without_a_second_attempt() {
    let state = tempfile::tempdir().expect("temporary daemon state");
    let state_root = std::fs::canonicalize(state.path()).expect("physical state root");
    let (_workspace, root) = catalog_workspace();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let service = Arc::new(CountedKnownLongService {
        prepares: AtomicUsize::new(0),
        executions: AtomicUsize::new(0),
        entered: Mutex::new(entered_tx),
        release: Mutex::new(release_rx),
    });
    let hooks = Arc::new(InspectionHooks::default());
    let identity = CoreIdentity::production_v5();
    let config = DaemonServerConfig::new(
        state_root.clone(),
        identity.clone(),
        Duration::from_secs(60),
    )
    .with_invocation_service(service.clone())
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
    wait_for_v5_record(&state_root, &identity);
    let mut owner = V5DaemonProcessOwner::connect_or_spawn(
        &state_root,
        identity,
        std::path::PathBuf::from("unused-existing-v5-endpoint"),
        Duration::from_secs(2),
    )
    .expect("connect v5 owner");
    let task_id = TaskId::new();
    let call = view_catalog_request(task_id, &root);

    let first = submitted_task_snapshot(owner.submit_invocation(call.clone()));
    assert_eq!(first.task_id(), task_id);
    entered_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("the bound Task starts its execution");

    let running = submitted_task_snapshot(owner.submit_invocation(call.clone()));
    assert_eq!(running.task_id(), task_id);
    assert_eq!(running.invocation_id(), call.invocation_id());
    assert!(
        !task_snapshot_is_terminal(&running),
        "the repeat reads the running Task: {running:?}"
    );

    let impostor = V5InvocationRequest::new(
        InvocationId::new(),
        task_id,
        V5ToolIdentity::View,
        call.arguments().clone(),
        unica_bootstrap::ResolvedWorkspace::launch_cwd(root.to_string_lossy().into_owned()),
        7_000,
    )
    .expect("valid impostor invocation");
    let impostor_key = ReceiptKey::new(
        impostor.invocation_id(),
        task_id,
        RequestIdentity::new(
            CoreIdentity::production_v5().digest().clone(),
            V5ToolIdentity::View,
            normalized_arguments_hash(impostor.arguments()),
            request_scope_hash(impostor.workspace_hint()).expect("request scope"),
        ),
    );
    assert_eq!(
        owner.submit_invocation(impostor),
        Ok(V5ServerResponse::Error {
            code: V5DaemonErrorCode::InvocationIdentityMismatch,
        }),
        "another key never takes over the Task id"
    );
    assert_eq!(
        owner.cancel_invocation(impostor_key),
        Ok(V5ServerResponse::Error {
            code: V5DaemonErrorCode::InvocationIdentityMismatch,
        }),
        "another key never cancels the Task either"
    );

    release_tx.send(()).expect("release the execution");
    let terminal = wait_task_terminal(&mut owner, task_id, Duration::from_secs(30));
    assert!(
        matches!(&terminal, V5DaemonTaskSnapshot::Completed { result, .. } if result.ok),
        "the single attempt completes: {terminal:?}"
    );
    let after = submitted_task_snapshot(owner.submit_invocation(call));
    assert_eq!(
        after, terminal,
        "the repeat after the end reads the same terminal"
    );

    assert_eq!(service.prepares.load(Ordering::SeqCst), 1);
    assert_eq!(service.executions.load(Ordering::SeqCst), 1);
    assert!(!hooks.fail_stopped.load(Ordering::SeqCst));
    drop(owner);
    stop.store(true, Ordering::SeqCst);
    server
        .join()
        .expect("daemon thread did not panic")
        .expect("daemon exits cleanly");
}

/// Records the bound every receipt-store command receives and refuses the
/// reservation with a capacity error, which closes nothing.
struct StoreDeadlineObservingPort {
    observed: mpsc::Sender<(&'static str, Option<Duration>)>,
    /// Once set, every generation probe is slow.
    slow_generation: Arc<AtomicBool>,
}

impl ReceiptLedgerPort for StoreDeadlineObservingPort {
    fn generation(&mut self, deadline: OperationDeadline) -> Result<u64, ReceiptLedgerError> {
        let _ = self
            .observed
            .send(("generation", deadline.remaining_at(Instant::now())));
        // A slow preceding step: the reservation begins well after the frame
        // arrived, so a budget counted from the frame would come up short.
        if self.slow_generation.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(400));
        }
        Ok(0)
    }

    fn reserve(
        &mut self,
        _key: ReceiptKey,
        _original_cutoff: OriginalCutoffDescriptor,
        deadline: OperationDeadline,
    ) -> Result<ReserveOutcome, ReceiptLedgerError> {
        let _ = self
            .observed
            .send(("reserve", deadline.remaining_at(Instant::now())));
        Err(ReceiptLedgerError::CapacityExceeded)
    }

    fn request_cancel_or_reserve(
        &mut self,
        _key: ReceiptKey,
        _cancel_reserved_at_epoch_ms: u64,
        _deadline: OperationDeadline,
    ) -> Result<CancelResolution, ReceiptLedgerError> {
        unreachable!("the submit test cancels nothing")
    }

    fn publish_direct_terminal(
        &mut self,
        _key: &ReceiptKey,
        _expected_version: ReceiptVersion,
        _terminal_epoch_ms: u64,
        _terminal: V5CanonicalTerminal,
        _deadline: OperationDeadline,
    ) -> Result<CommittedDirectPublication, ReceiptLedgerError> {
        unreachable!("a refused reservation publishes nothing")
    }

    fn recover(
        &mut self,
        _key: &ReceiptKey,
        _deadline: OperationDeadline,
    ) -> Result<ReceiptState, ReceiptLedgerError> {
        Err(ReceiptLedgerError::ReceiptNotFound)
    }
}

/// #1387: a submit whose caller budget is a single millisecond still gives
/// every receipt-store step of the request the daemon's own step budget,
/// counted from the step's start. A commit the store began is therefore not
/// cut short by the caller.
#[test]
fn tiny_submit_budget_gives_the_receipt_store_its_own_step_budget() {
    let root = tempfile::tempdir().expect("temporary state root");
    let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
    let identity = CoreIdentity::production_v5();
    let (observed_tx, observed_rx) = mpsc::channel();
    let slow_generation = Arc::new(AtomicBool::new(false));
    let port_slow = slow_generation.clone();
    let config = DaemonServerConfig::new(
        state_root.clone(),
        identity.clone(),
        Duration::from_millis(300),
    );
    let server = thread::spawn(move || {
        run_daemon_configured(config, move |mut runtime| {
            runtime.receipt_ledger = spawn_receipt_ledger_actor(StoreDeadlineObservingPort {
                observed: observed_tx,
                slow_generation: port_slow,
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
        unica_bootstrap::ResolvedWorkspace::launch_cwd("workspace-a".to_owned()),
        1,
    )
    .expect("valid one-millisecond submit");
    let mut owner = V5DaemonProcessOwner::connect_or_spawn(
        &state_root,
        identity,
        std::path::PathBuf::from("unused-existing-v5-endpoint"),
        Duration::from_secs(5),
    )
    .expect("connect owner");
    // Drop what the session admission observed: only the submit counts.
    while observed_rx.try_recv().is_ok() {}
    slow_generation.store(true, Ordering::SeqCst);
    let response = owner.submit_invocation(invocation);
    drop(owner);
    let server_result = server.join().expect("join runtime");
    let observed: Vec<_> = observed_rx.try_iter().collect();

    // The answer comes after the caller's millisecond, so the caller may
    // have stopped reading; what it would read is the store's own refusal.
    assert!(
        matches!(
            &response,
            Ok(V5ServerResponse::Error {
                code: V5DaemonErrorCode::ReceiptCapacity,
            })
        ) || matches!(&response, Err(error) if error.contains("deadline expired")),
        "{response:?}"
    );
    assert!(
        observed.iter().any(|(command, _)| *command == "reserve"),
        "the reservation reached the store: {observed:?}"
    );
    for (command, remaining) in &observed {
        // Read on entry to the port, right after the step began on the
        // actor thread: only that hand-over separates it from the full budget.
        let remaining = remaining.expect("a store step is bounded");
        assert!(
            remaining >= DURABLE_STEP_BUDGET - Duration::from_millis(100),
            "{command} got only {remaining:?} of the store's own budget"
        );
    }
    assert_eq!(server_result, Ok(()));
}

/// #1387 on the real receipt ledger: an early cancel and then a submit of the
/// same key with a budget far below one durable commit end as the cancelled
/// direct terminal, readable by recovery; the daemon never fail-stops.
#[test]
fn tiny_submit_budget_after_an_early_cancel_keeps_the_daemon() {
    let root = tempfile::tempdir().expect("temporary state root");
    let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
    let identity = CoreIdentity::production_v5();
    let hooks = Arc::new(InspectionHooks::default());
    let config = DaemonServerConfig::new(
        state_root.clone(),
        identity.clone(),
        Duration::from_secs(60),
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
    wait_for_v5_record(&state_root, &identity);
    let owner = V5DaemonProcessOwner::connect_or_spawn(
        &state_root,
        identity.clone(),
        std::path::PathBuf::from("unused-existing-v5-endpoint"),
        Duration::from_secs(2),
    )
    .expect("connect owner");
    for budget_ms in [1_u64, 50] {
        let mut session = owner
            .connect_peer_before(Instant::now() + Duration::from_secs(2))
            .expect("open a session for the call");
        let arguments = serde_json::Map::new();
        let invocation_id = InvocationId::new();
        let task_id = TaskId::new();
        let key = ReceiptKey::new(
            invocation_id,
            task_id,
            RequestIdentity::new(
                identity.digest().clone(),
                V5ToolIdentity::View,
                normalized_arguments_hash(&arguments),
                request_scope_hash("workspace-a").expect("request scope"),
            ),
        );
        let cancel = session
            .cancel_invocation(key.clone())
            .expect("reserve the early cancellation");
        assert!(matches!(
            cancel,
            V5ServerResponse::Invocation {
                outcome: V5InvocationResponse::ReceiptPending {
                    phase: V5InvocationPhase::CancelReserved,
                    ..
                }
            }
        ));
        let invocation = V5InvocationRequest::new(
            invocation_id,
            task_id,
            V5ToolIdentity::View,
            arguments,
            unica_bootstrap::ResolvedWorkspace::launch_cwd("workspace-a".to_owned()),
            budget_ms,
        )
        .expect("valid short submit");
        // The caller may give up on the answer before it is written; that
        // loses the answer, never the commit, and recovery reads it.
        let submit = session.submit_invocation(invocation);
        assert!(
            !matches!(
                submit,
                Ok(V5ServerResponse::Error {
                    code: V5DaemonErrorCode::StoreCommitUncertain
                })
            ),
            "a {budget_ms} ms submit made the commit uncertain"
        );
        let mut peer = owner
            .connect_peer_before(Instant::now() + Duration::from_secs(2))
            .expect("the daemon still admits sessions");
        let recovered = peer
            .recover_invocation_receipt(key.clone())
            .expect("the daemon still answers");
        assert!(
            matches!(
                &recovered,
                V5ServerResponse::Invocation {
                    outcome: V5InvocationResponse::Direct { receipt }
                } if matches!(receipt.terminal(), ReceiptTerminalOutcome::Cancelled)
                    && receipt.receipt_key() == &key
            ),
            "a {budget_ms} ms submit ends as the early cancellation: {recovered:?}"
        );
        if let Ok(answered) = submit {
            assert_eq!(answered, recovered, "recovery reads the answered terminal");
        }
    }
    assert!(!hooks.fail_stopped.load(Ordering::SeqCst));
    drop(owner);
    stop.store(true, Ordering::SeqCst);
    server
        .join()
        .expect("daemon thread did not panic")
        .expect("daemon exits cleanly");
}

/// Holds the first attempt's handoff before its Task exists, and lets the
/// exact repeat reach its reservation only once that handoff completed — or
/// after a second, when the completion waits for the repeat's key lock.
struct ExactRepeatRaceHooks {
    /// The point of the racing call and which pass through it races.
    race_point: V5PausePoint,
    race_pass: usize,
    handoff_held: Mutex<mpsc::Sender<()>>,
    handoff_gate: (Mutex<bool>, std::sync::Condvar),
    reservations: AtomicUsize,
    repeat_at_reservation: Mutex<mpsc::Sender<()>>,
    bound: (Mutex<bool>, std::sync::Condvar),
}

impl V5RuntimeHooks for ExactRepeatRaceHooks {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn holds(&self, point: V5PausePoint) -> bool {
        point == V5PausePoint::BeforeTaskStoreCreate || point == self.race_point
    }

    fn pause(&self, point: V5PausePoint, _deadline: Instant) -> Result<(), ReceiptLedgerError> {
        match point {
            V5PausePoint::BeforeTaskStoreCreate => {
                let _ = self.handoff_held.lock().unwrap().send(());
                let (lock, changed) = &self.handoff_gate;
                let mut released = lock.lock().unwrap();
                while !*released {
                    released = changed.wait(released).unwrap();
                }
            }
            point
                if point == self.race_point
                    && self.reservations.fetch_add(1, Ordering::SeqCst) == self.race_pass =>
            {
                let _ = self.repeat_at_reservation.lock().unwrap().send(());
                let (lock, changed) = &self.bound;
                let bound = lock.lock().unwrap();
                let _ = changed
                    .wait_timeout_while(bound, Duration::from_secs(1), |bound| !*bound)
                    .unwrap();
            }
            _ => {}
        }
        Ok(())
    }

    fn event(&self, event: V5ReceiptRuntimeEventKind, _epoch_ms: u64) {
        if event == V5ReceiptRuntimeEventKind::TaskBoundCommitted {
            let (lock, changed) = &self.bound;
            *lock.lock().unwrap() = true;
            changed.notify_all();
        }
    }
}

/// #1384: the receipt row leaves the ledger when the handoff completes, and
/// the Task link alone remembers the key from then on. An exact repeat that
/// found no link yet must not reserve the key after the row is gone: it reads
/// the one Task, and the attempt is prepared once.
#[test]
fn exact_repeat_racing_the_handoff_completion_reads_the_same_task() {
    let state = tempfile::tempdir().expect("temporary daemon state");
    let state_root = std::fs::canonicalize(state.path()).expect("physical state root");
    let (_workspace, root) = catalog_workspace();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let service = Arc::new(CountedKnownLongService {
        prepares: AtomicUsize::new(0),
        executions: AtomicUsize::new(0),
        entered: Mutex::new(entered_tx),
        release: Mutex::new(release_rx),
    });
    let (held_tx, held_rx) = mpsc::channel();
    let (repeat_tx, repeat_rx) = mpsc::channel();
    let hooks = Arc::new(ExactRepeatRaceHooks {
        race_point: V5PausePoint::BeforeSubmitReservation,
        race_pass: 1,
        handoff_held: Mutex::new(held_tx),
        handoff_gate: (Mutex::new(false), std::sync::Condvar::new()),
        reservations: AtomicUsize::new(0),
        repeat_at_reservation: Mutex::new(repeat_tx),
        bound: (Mutex::new(false), std::sync::Condvar::new()),
    });
    let identity = CoreIdentity::production_v5();
    let config = DaemonServerConfig::new(
        state_root.clone(),
        identity.clone(),
        Duration::from_secs(60),
    )
    .with_invocation_service(service.clone())
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
    wait_for_v5_record(&state_root, &identity);
    let mut owner = V5DaemonProcessOwner::connect_or_spawn(
        &state_root,
        identity,
        std::path::PathBuf::from("unused-existing-v5-endpoint"),
        Duration::from_secs(2),
    )
    .expect("connect v5 owner");
    let task_id = TaskId::new();
    let call = view_catalog_request(task_id, &root);

    let mut first_session = owner
        .connect_peer_before(Instant::now() + Duration::from_secs(2))
        .expect("connect the first session");
    let first_call = call.clone();
    let first = thread::spawn(move || first_session.submit_invocation(first_call));
    held_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("the handoff stops before its Task exists");
    let mut repeat_session = owner
        .connect_peer_before(Instant::now() + Duration::from_secs(2))
        .expect("connect the repeat session");
    let repeat_call = call.clone();
    let repeat = thread::spawn(move || repeat_session.submit_invocation(repeat_call));
    repeat_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("the repeat found no link and stands before its reservation");
    {
        let (lock, changed) = &hooks.handoff_gate;
        *lock.lock().unwrap() = true;
        changed.notify_all();
    }

    let repeated = submitted_task_snapshot(repeat.join().expect("repeat thread"));
    let answered = submitted_task_snapshot(first.join().expect("first thread"));
    assert_eq!(answered.task_id(), task_id);
    assert_eq!(repeated.task_id(), task_id);
    assert_eq!(repeated.invocation_id(), call.invocation_id());
    entered_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("the single attempt executes");
    release_tx.send(()).expect("release the execution");
    let terminal = wait_task_terminal(&mut owner, task_id, Duration::from_secs(30));
    assert!(
        matches!(&terminal, V5DaemonTaskSnapshot::Completed { result, .. } if result.ok),
        "the single attempt completes: {terminal:?}"
    );
    assert_eq!(service.prepares.load(Ordering::SeqCst), 1);
    assert_eq!(service.executions.load(Ordering::SeqCst), 1);
    drop(owner);
    stop.store(true, Ordering::SeqCst);
    server
        .join()
        .expect("daemon thread did not panic")
        .expect("daemon exits cleanly");
}

/// Counts every domain callback; none may run in these tests.
#[derive(Default)]
struct CountingService {
    prepares: AtomicUsize,
    executions: AtomicUsize,
}

impl CanonicalInvocationService for CountingService {
    fn prepare(
        &self,
        _invocation: &crate::infrastructure::daemon::server::ActorBoundInvocation,
    ) -> Result<ExecutionClass, Box<DomainResult>> {
        self.prepares.fetch_add(1, Ordering::SeqCst);
        Ok(ExecutionClass::KnownLong(KnownLongReason::ExternalProcess))
    }

    fn execute(
        &self,
        _invocation: &crate::infrastructure::daemon::server::ActorBoundExecution,
        _cancellation: CancellationToken,
    ) -> Result<DomainResult, InvocationFailure> {
        self.executions.fetch_add(1, Ordering::SeqCst);
        Ok(DomainResult::success("ran again"))
    }
}

/// #1384: the Task of a call is being retired by its TTL — the link holds a
/// committed retirement intent, the Task record is already deleted. The exact
/// repeat of the call finds no answer left to read: it is refused as not
/// found, the key is not reserved again and no domain code runs.
#[test]
fn exact_repeat_of_a_retiring_task_is_not_found_and_reserves_nothing() {
    let root = tempfile::tempdir().expect("temporary retirement state root");
    let state_root = std::fs::canonicalize(root.path()).expect("physical retirement state root");
    let identity = CoreIdentity::production_v5();
    let clock = Arc::new(ManualEpochClock::new(1_000));
    let service = Arc::new(CountingService::default());
    let config = DaemonServerConfig::new(
        state_root.clone(),
        identity.clone(),
        Duration::from_millis(50),
    )
    .with_invocation_service(service.clone())
    .with_v5_epoch_clock_for_test(clock.clone());
    let state =
        DaemonStateDirectory::open(&state_root, &identity).expect("open retirement daemon state");
    let runtime =
        Arc::new(V5ReceiptRuntime::open(&state, &config).expect("open retirement runtime"));
    let key = materialize_completed_task_key(
        &runtime,
        &identity,
        &clock,
        "workspace-a",
        1_000,
        2_000,
        "expired",
    );
    let task_id = key.reserved_task_id();
    clock.set(RETIREMENT_TEST_OBSERVED_EPOCH_MS);
    // The delete lands but its sync is uncertain: the record stays retiring,
    // its Task gone, until the retry pause ends.
    runtime
        .task_projection
        .task_store
        .inject_next_publication_failure(PublicationFailure::AfterDeleteBeforeSync);
    let _ = runtime.resolve_task(task_id, Instant::now() + Duration::from_secs(5));
    assert!(
        matches!(
            retirement_link(&runtime, task_id),
            Some(TaskLifecycleLinkRecord::TaskRetirementPending(_))
        ) && !retirement_task_present(&runtime, task_id),
        "the fixture leaves the Task retiring"
    );

    let invocation = V5InvocationRequest::new(
        key.invocation_id(),
        task_id,
        V5ToolIdentity::View,
        serde_json::Map::new(),
        unica_bootstrap::ResolvedWorkspace::launch_cwd("workspace-a".to_owned()),
        7_000,
    )
    .expect("valid exact repeat");
    let decoded = decode_v5_request_frame(
        serde_json::to_vec(&V5ClientRequest::SubmitInvocation { invocation })
            .expect("serialize the exact repeat"),
    )
    .expect("decode the exact repeat");
    let repeat = runtime.submit_invocation(
        decoded,
        RETIREMENT_TEST_OBSERVED_EPOCH_MS,
        Instant::now() + Duration::from_secs(5),
    );

    assert!(
        matches!(repeat, Err(ReceiptLedgerError::ReceiptNotFound)),
        "a retiring Task has no answer left: {:?}",
        repeat.as_ref().map(|_| "reply")
    );
    assert!(
        matches!(
            runtime
                .receipt_ledger
                .recover(key, Instant::now() + Duration::from_secs(5)),
            Err(ReceiptLedgerError::ReceiptNotFound)
        ),
        "the key was reserved again"
    );
    runtime.join_task_executions();
    assert_eq!(service.prepares.load(Ordering::SeqCst), 0);
    assert_eq!(service.executions.load(Ordering::SeqCst), 0);
    assert!(!runtime.restart_required());
}

/// #1384: the staged handoff completion removes the call's receipt row only
/// under the key's lock, the same lock an exact repeat holds between its Task
/// link lookup and its reservation. While the lock is held the row stays.
#[test]
fn staged_handoff_completion_waits_for_the_key_lock() {
    let root = tempfile::tempdir().expect("temporary staged root");
    let state_root = std::fs::canonicalize(root.path()).expect("physical staged root");
    let identity = CoreIdentity::production_v5();
    let state = DaemonStateDirectory::open(&state_root, &identity).expect("open staged state");
    let service = Arc::new(CountingService::default());
    let config = DaemonServerConfig::new(state_root, identity.clone(), Duration::from_millis(50))
        .with_invocation_service(service.clone());
    let clock = Arc::new(ManualEpochClock::new(1_009));
    let runtime = Arc::new(
        V5ReceiptRuntime::open_with_epoch_clock(&state, &config, clock)
            .expect("open staged runtime"),
    );
    let handoff = prepare_staged_startup_handoff(&runtime, &identity, AttemptPhase::Begun);
    let key = handoff.key().clone();
    let digest = crate::application::receipt_ledger::receipt_key_digest(&key);
    let winner = canonical_v5_terminal(&ReceiptTerminalOutcome::Completed {
        result: Box::new(DomainResult::success("staged winner")),
    })
    .expect("canonical staged winner");

    let lock = runtime.submit_fences.stripe(&digest);
    let publisher = {
        let runtime = Arc::clone(&runtime);
        thread::spawn(move || {
            runtime.publish_staged_handoff_terminal_reply(
                handoff,
                winner,
                1_009,
                Instant::now() + Duration::from_secs(10),
            )
        })
    };
    // Long enough for a publisher that ignored the lock to publish the Task,
    // its terminal and the link; one that waits for it publishes nothing yet.
    let until = Instant::now() + Duration::from_secs(2);
    while Instant::now() < until
        && !matches!(
            retirement_link(&runtime, key.reserved_task_id()),
            Some(TaskLifecycleLinkRecord::TaskTerminalBound(_))
        )
    {
        thread::sleep(Duration::from_millis(10));
    }
    thread::sleep(Duration::from_millis(200));
    let held = runtime
        .receipt_ledger
        .recover(key.clone(), Instant::now() + Duration::from_secs(5));
    assert!(
        matches!(held, Ok(ReceiptState::TaskHandoffActorBound(_))),
        "the receipt row left the ledger while its key was locked: {held:?}"
    );
    assert!(!publisher.is_finished());
    drop(lock);

    let reply = publisher
        .join()
        .expect("publisher thread")
        .expect("the staged handoff completes");
    assert!(matches!(
        reply,
        V5RuntimeReply::Json(V5ServerResponse::Invocation {
            outcome: V5InvocationResponse::Task { .. }
        })
    ));
    assert!(matches!(
        runtime
            .receipt_ledger
            .recover(key, Instant::now() + Duration::from_secs(5)),
        Err(ReceiptLedgerError::ReceiptNotFound)
    ));
    assert_eq!(service.prepares.load(Ordering::SeqCst), 0);
}

/// #1384, cancel side: a cancel of the exact key that found no Task link yet
/// must not write a cancel row after the handoff completion moved the key to
/// the link. It reaches the call itself: the Task sees the cancel, recovery
/// reads the Task, and no stray cancel row is left behind.
#[test]
fn exact_cancel_racing_the_handoff_completion_cancels_the_task() {
    let state = tempfile::tempdir().expect("temporary daemon state");
    let state_root = std::fs::canonicalize(state.path()).expect("physical state root");
    let (_workspace, root) = catalog_workspace();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let service = Arc::new(CountedKnownLongService {
        prepares: AtomicUsize::new(0),
        executions: AtomicUsize::new(0),
        entered: Mutex::new(entered_tx),
        release: Mutex::new(release_rx),
    });
    let (held_tx, held_rx) = mpsc::channel();
    let (cancel_tx, cancel_rx) = mpsc::channel();
    let hooks = Arc::new(ExactRepeatRaceHooks {
        race_point: V5PausePoint::BeforeCancelReservation,
        race_pass: 0,
        handoff_held: Mutex::new(held_tx),
        handoff_gate: (Mutex::new(false), std::sync::Condvar::new()),
        reservations: AtomicUsize::new(0),
        repeat_at_reservation: Mutex::new(cancel_tx),
        bound: (Mutex::new(false), std::sync::Condvar::new()),
    });
    let identity = CoreIdentity::production_v5();
    let config = DaemonServerConfig::new(
        state_root.clone(),
        identity.clone(),
        Duration::from_secs(60),
    )
    .with_invocation_service(service.clone())
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
    wait_for_v5_record(&state_root, &identity);
    let mut owner = V5DaemonProcessOwner::connect_or_spawn(
        &state_root,
        identity.clone(),
        std::path::PathBuf::from("unused-existing-v5-endpoint"),
        Duration::from_secs(2),
    )
    .expect("connect v5 owner");
    let task_id = TaskId::new();
    let call = view_catalog_request(task_id, &root);
    let key = ReceiptKey::new(
        call.invocation_id(),
        task_id,
        RequestIdentity::new(
            identity.digest().clone(),
            V5ToolIdentity::View,
            normalized_arguments_hash(call.arguments()),
            request_scope_hash(call.workspace_hint()).expect("request scope"),
        ),
    );

    let mut first_session = owner
        .connect_peer_before(Instant::now() + Duration::from_secs(2))
        .expect("connect the first session");
    let first_call = call.clone();
    let first = thread::spawn(move || first_session.submit_invocation(first_call));
    held_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("the handoff stops before its Task exists");
    let mut cancel_session = owner
        .connect_peer_before(Instant::now() + Duration::from_secs(2))
        .expect("connect the cancel session");
    let cancel_key = key.clone();
    let cancel = thread::spawn(move || cancel_session.cancel_invocation(cancel_key));
    cancel_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("the cancel found no link and stands before its ledger write");
    {
        let (lock, changed) = &hooks.handoff_gate;
        *lock.lock().unwrap() = true;
        changed.notify_all();
    }

    let cancelled = cancel
        .join()
        .expect("cancel thread")
        .expect("cancel answered");
    let answered = submitted_task_snapshot(first.join().expect("first thread"));
    assert_eq!(answered.task_id(), task_id);
    assert!(
        matches!(
            &cancelled,
            V5ServerResponse::Invocation {
                outcome: V5InvocationResponse::Task { snapshot }
            } if snapshot.task_id() == task_id && snapshot.cancel_requested()
        ),
        "the cancel reaches the Task: {cancelled:?}"
    );
    let recovered = owner
        .recover_invocation_receipt(key)
        .expect("recover the call");
    assert!(
        matches!(
            &recovered,
            V5ServerResponse::Invocation {
                outcome: V5InvocationResponse::Task { snapshot }
            } if snapshot.task_id() == task_id
        ),
        "recovery reads the Task, not a stray cancel row: {recovered:?}"
    );
    let _ = entered_rx.recv_timeout(Duration::from_secs(2));
    let _ = release_tx.send(());
    let terminal = wait_task_terminal(&mut owner, task_id, Duration::from_secs(30));
    assert!(
        terminal.cancel_requested(),
        "the Task keeps the cancel: {terminal:?}"
    );
    assert!(service.prepares.load(Ordering::SeqCst) <= 1);
    drop(owner);
    stop.store(true, Ordering::SeqCst);
    server
        .join()
        .expect("daemon thread did not panic")
        .expect("daemon exits cleanly");
}

/// #1384: every bound handoff completion that does not already hold the
/// key's lock takes it before the receipt row leaves the ledger.
#[test]
fn bound_handoff_completion_waits_for_the_key_lock() {
    let root = tempfile::tempdir().expect("temporary handoff root");
    let state_root = std::fs::canonicalize(root.path()).expect("physical handoff root");
    let identity = CoreIdentity::production_v5();
    let state = DaemonStateDirectory::open(&state_root, &identity).expect("open handoff state");
    let config = DaemonServerConfig::new(state_root, identity.clone(), Duration::from_millis(50))
        .with_invocation_service(Arc::new(CountingService::default()));
    let clock = Arc::new(ManualEpochClock::new(1_009));
    let runtime = Arc::new(
        V5ReceiptRuntime::open_with_epoch_clock(&state, &config, clock)
            .expect("open handoff runtime"),
    );
    let handoff = prepare_staged_startup_handoff(&runtime, &identity, AttemptPhase::Begun);
    let key = handoff.key().clone();
    let deadline = Instant::now() + Duration::from_secs(10);
    let (_record, task_bound) = runtime
        .task_projection
        .materialize_bound_handoff(&handoff, 1_009, deadline, runtime.hooks.as_ref())
        .unwrap_or_else(|failure| panic!("materialize TaskBound: {}", failure.error));
    let digest = crate::application::receipt_ledger::receipt_key_digest(&key);

    let lock = runtime.submit_fences.stripe(&digest);
    let completion = {
        let runtime = Arc::clone(&runtime);
        let key = key.clone();
        let version = handoff.record_version();
        thread::spawn(move || {
            runtime.complete_bound_task_handoff(key, version, task_bound, deadline)
        })
    };
    // A completion that ignored the lock would be done long before this.
    thread::sleep(Duration::from_millis(200));
    let held = runtime
        .receipt_ledger
        .recover(key.clone(), Instant::now() + Duration::from_secs(5));
    assert!(
        matches!(held, Ok(ReceiptState::TaskHandoffActorBound(_))),
        "the receipt row left the ledger while its key was locked: {held:?}"
    );
    assert!(!completion.is_finished());
    drop(lock);

    completion
        .join()
        .expect("completion thread")
        .expect("the handoff completes");
    assert!(matches!(
        runtime
            .receipt_ledger
            .recover(key, Instant::now() + Duration::from_secs(5)),
        Err(ReceiptLedgerError::ReceiptNotFound)
    ));
}

/// Hands the committed handoff an already elapsed deadline before its Task
/// is materialized: nothing after the commit may depend on what is left of
/// it, however long the materialization takes (#1395).
#[derive(Default)]
struct SpentHandoffDeadlineHooks {
    fail_stopped: AtomicBool,
}

impl V5RuntimeHooks for SpentHandoffDeadlineHooks {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn commit_deadline_at(&self, point: V5PausePoint, deadline: Instant) -> Instant {
        if point == V5PausePoint::BeforeTaskStoreCreate {
            let now = Instant::now();
            now.checked_sub(Duration::from_millis(1)).unwrap_or(now)
        } else {
            deadline
        }
    }

    fn forced_process_exit(&self, _grace: Option<Duration>) {
        self.fail_stopped.store(true, Ordering::SeqCst);
    }
}

/// Once the handoff of a known-long call is committed, materializing and
/// completing it get the daemon's own step budget. A deadline spent before
/// the Task exists leaves no Task unexecuted. (A begun handoff materializes
/// its Task already working, so no separate start step needs a budget here.)
#[test]
fn committed_handoff_starts_its_task_after_the_deadline_was_spent() {
    let state = tempfile::tempdir().expect("temporary daemon state");
    let state_root = std::fs::canonicalize(state.path()).expect("physical state root");
    let (_workspace, root) = catalog_workspace();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let service = Arc::new(CountedKnownLongService {
        prepares: AtomicUsize::new(0),
        executions: AtomicUsize::new(0),
        entered: Mutex::new(entered_tx),
        release: Mutex::new(release_rx),
    });
    let hooks = Arc::new(SpentHandoffDeadlineHooks::default());
    let identity = CoreIdentity::production_v5();
    let config = DaemonServerConfig::new(
        state_root.clone(),
        identity.clone(),
        Duration::from_secs(60),
    )
    .with_invocation_service(service.clone())
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
    wait_for_v5_record(&state_root, &identity);
    let mut owner = V5DaemonProcessOwner::connect_or_spawn(
        &state_root,
        identity,
        std::path::PathBuf::from("unused-existing-v5-endpoint"),
        Duration::from_secs(2),
    )
    .expect("connect v5 owner");
    let task_id = TaskId::new();

    let first =
        submitted_task_snapshot(owner.submit_invocation(view_catalog_request(task_id, &root)));
    assert_eq!(first.task_id(), task_id);
    entered_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("the Task starts its execution");
    release_tx.send(()).expect("release the execution");
    let terminal = wait_task_terminal(&mut owner, task_id, Duration::from_secs(30));
    assert!(
        matches!(&terminal, V5DaemonTaskSnapshot::Completed { result, .. } if result.ok),
        "the committed Task completes: {terminal:?}"
    );
    assert_eq!(service.prepares.load(Ordering::SeqCst), 1);
    assert_eq!(service.executions.load(Ordering::SeqCst), 1);
    assert!(!hooks.fail_stopped.load(Ordering::SeqCst));
    drop(owner);
    stop.store(true, Ordering::SeqCst);
    server
        .join()
        .expect("daemon thread did not panic")
        .expect("daemon exits cleanly");
}

/// Recovery terminalizes a bound Task it found interrupted even when the
/// startup deadline it inherited is already spent.
#[test]
fn recovery_terminalizes_a_bound_task_after_the_startup_deadline_was_spent() {
    let root = tempfile::tempdir().expect("temporary recovery root");
    let state_root = std::fs::canonicalize(root.path()).expect("physical recovery root");
    let identity = CoreIdentity::production_v5();
    let state = DaemonStateDirectory::open(&state_root, &identity).expect("open recovery state");
    let config = DaemonServerConfig::new(state_root, identity.clone(), Duration::from_millis(50))
        .with_invocation_service(Arc::new(CountingService::default()));
    let clock = Arc::new(ManualEpochClock::new(1_009));
    let runtime = V5ReceiptRuntime::open_with_epoch_clock(&state, &config, clock)
        .expect("open recovery runtime");
    let handoff = prepare_staged_startup_handoff(&runtime, &identity, AttemptPhase::Begun);
    let deadline = Instant::now() + Duration::from_secs(10);
    let (record, task_bound) = runtime
        .task_projection
        .materialize_bound_handoff(&handoff, 1_009, deadline, runtime.hooks.as_ref())
        .unwrap_or_else(|failure| panic!("materialize TaskBound: {}", failure.error));
    let task_bound = runtime
        .complete_bound_task_handoff(
            handoff.key().clone(),
            handoff.record_version(),
            task_bound,
            deadline,
        )
        .expect("complete the handoff");

    let spent = Instant::now();
    thread::sleep(Duration::from_millis(5));
    let terminal = runtime
        .task_projection
        .terminalize_recovered_bound(
            &task_bound,
            record,
            RecoveryTerminalReason::OutcomeUncertain,
            spent,
        )
        .unwrap_or_else(|failure| panic!("terminalize after a spent deadline: {}", failure.error));
    assert_eq!(terminal.key(), handoff.key());
    assert!(matches!(
        retirement_link(&runtime, handoff.key().reserved_task_id()),
        Some(TaskLifecycleLinkRecord::TaskTerminalBound(_))
    ));
}

/// The same on the continuation of a promised Task: an admission that
/// outlived the handoff moment commits the handoff with a spent deadline, and
/// the continuation still starts, executes and completes the one attempt.
#[test]
fn promised_continuation_runs_its_task_after_the_deadline_was_spent() {
    let state = tempfile::tempdir().expect("temporary daemon state");
    let state_root = std::fs::canonicalize(state.path()).expect("physical state root");
    let (_workspace, root) = catalog_workspace();
    let admission =
        crate::infrastructure::daemon::server::admission_test_control::AdmissionPause::install(
            root.clone(),
        );
    let clock = Arc::new(InspectionClock {
        start: Instant::now(),
        elapsed_ms: AtomicU64::new(0),
    });
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let service = Arc::new(CountedKnownLongService {
        prepares: AtomicUsize::new(0),
        executions: AtomicUsize::new(0),
        entered: Mutex::new(entered_tx),
        release: Mutex::new(release_rx),
    });
    let canonical_runtime = Arc::new(V5CanonicalInvocationRuntime::new(
        service.clone(),
        clock.clone(),
    ));
    let hooks = Arc::new(SpentHandoffDeadlineHooks::default());
    let identity = CoreIdentity::production_v5();
    let config = DaemonServerConfig::new(
        state_root.clone(),
        identity.clone(),
        Duration::from_secs(600),
    )
    .with_canonical_runtime_for_test(canonical_runtime)
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
    wait_for_v5_record(&state_root, &identity);
    let owner = V5DaemonProcessOwner::connect_or_spawn(
        &state_root,
        identity,
        std::path::PathBuf::from("unused-existing-v5-endpoint"),
        Duration::from_secs(2),
    )
    .expect("connect v5 owner");
    let task_id = TaskId::new();
    let request = view_catalog_request(task_id, &root);
    let submit = thread::spawn(move || {
        let mut owner = owner;
        let response = owner.submit_invocation(request);
        (owner, response)
    });
    admission.wait_until_entered();
    clock.elapsed_ms.store(7_000, Ordering::SeqCst);
    let (mut owner, promised) = submit.join().expect("submit thread");
    assert_eq!(
        submitted_task_snapshot(promised.map_err(|error| error.to_string())).task_id(),
        task_id
    );
    admission.release();
    entered_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("the continuation executes the Task");
    release_tx.send(()).expect("release the execution");
    let terminal = wait_task_terminal(&mut owner, task_id, Duration::from_secs(30));
    assert!(
        matches!(&terminal, V5DaemonTaskSnapshot::Completed { result, .. } if result.ok),
        "the promised Task completes: {terminal:?}"
    );
    assert_eq!(service.prepares.load(Ordering::SeqCst), 1);
    assert_eq!(service.executions.load(Ordering::SeqCst), 1);
    assert!(!hooks.fail_stopped.load(Ordering::SeqCst));
    drop(owner);
    drop(admission);
    stop.store(true, Ordering::SeqCst);
    server
        .join()
        .expect("daemon thread did not panic")
        .expect("daemon exits cleanly");
}

/// Which handoff materializes the Task while an exact cancel races it (#1395).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MaterializationRace {
    /// The handler promotes a begun attempt held before prepare and
    /// materializes its Task itself (`promote_at_cutoff`).
    PromotedBegun,
    /// The handler commits the handoff intent of an actor-bound attempt; the
    /// attempt materializes the Task as its continuation (`continue_handoff`).
    PromotedActorBound,
    /// The inline drive of a running execution reaches the cutoff and
    /// materializes the Task it hands the execution to (`commit_cutoff_handoff`).
    InlineCutoff,
    /// The same, but the cancel marks the committed handoff before the Task
    /// is materialized: the running execution must hear it all the same.
    InlineCutoffMarkedBeforeTask,
    /// An admission outlives the cutoff; its continuation binds the promised
    /// Task and materializes it (`continue_promised`).
    PromisedAdmission,
}

/// Holds the attempt before the cutoff where the race needs it, the handoff
/// between the Task store create and the Task link, a not-begun Task before
/// it starts, and, when asked, the cancel between its read of the Task and
/// its write.
struct MaterializationRaceHooks {
    hold_point: Option<V5PausePoint>,
    hold_before_task: bool,
    hold_cancel_write: bool,
    cancel_write_held: Mutex<mpsc::Sender<()>>,
    cancel_write_gate: (Mutex<bool>, std::sync::Condvar),
    started: Mutex<mpsc::Sender<()>>,
    attempt_held: Mutex<mpsc::Sender<()>>,
    attempt_gate: (Mutex<bool>, std::sync::Condvar),
    attempt_passes: AtomicUsize,
    materializing: Mutex<mpsc::Sender<()>>,
    materialize_gate: (Mutex<bool>, std::sync::Condvar),
    start_gate: (Mutex<bool>, std::sync::Condvar),
    creates: AtomicUsize,
    fail_stopped: AtomicBool,
}

impl MaterializationRaceHooks {
    fn wait_gate(gate: &(Mutex<bool>, std::sync::Condvar)) {
        let (lock, changed) = gate;
        let _ = changed
            .wait_timeout_while(lock.lock().unwrap(), Duration::from_secs(30), |open| !*open)
            .unwrap();
    }

    fn open_gate(gate: &(Mutex<bool>, std::sync::Condvar)) {
        let (lock, changed) = gate;
        *lock.lock().unwrap() = true;
        changed.notify_all();
    }
}

impl V5RuntimeHooks for MaterializationRaceHooks {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn pause(&self, point: V5PausePoint, _deadline: Instant) -> Result<(), ReceiptLedgerError> {
        if Some(point) == self.hold_point && self.attempt_passes.fetch_add(1, Ordering::SeqCst) == 0
        {
            let _ = self.attempt_held.lock().unwrap().send(());
            Self::wait_gate(&self.attempt_gate);
        }
        if self.hold_before_task
            && point == V5PausePoint::BeforeTaskStoreCreate
            && self.creates.fetch_add(1, Ordering::SeqCst) == 0
        {
            // The handoff is committed, its Task does not exist yet.
            let _ = self.materializing.lock().unwrap().send(());
            Self::wait_gate(&self.materialize_gate);
        }
        if point == V5PausePoint::AfterFalseCancelObservation {
            Self::wait_gate(&self.start_gate);
        }
        if self.hold_cancel_write
            && point == V5PausePoint::BeforeTaskCancelWrite
            && !*self.cancel_write_gate.0.lock().unwrap()
        {
            let _ = self.cancel_write_held.lock().unwrap().send(());
            Self::wait_gate(&self.cancel_write_gate);
        }
        Ok(())
    }

    fn event(&self, event: V5ReceiptRuntimeEventKind, _epoch_ms: u64) {
        if event == V5ReceiptRuntimeEventKind::TaskStoreWorkingReadback {
            let _ = self.started.lock().unwrap().send(());
        }
        // The Task row exists, its link does not yet.
        if !self.hold_before_task
            && event == V5ReceiptRuntimeEventKind::TaskStoreCreated
            && self.creates.fetch_add(1, Ordering::SeqCst) == 0
        {
            let _ = self.materializing.lock().unwrap().send(());
            Self::wait_gate(&self.materialize_gate);
        }
    }

    fn forced_process_exit(&self, _grace: Option<Duration>) {
        self.fail_stopped.store(true, Ordering::SeqCst);
    }
}

/// Executes inline or known-long and runs until its cancellation; counts
/// every domain callback.
struct CancelRaceService {
    class: ExecutionClass,
    prepares: AtomicUsize,
    executions: AtomicUsize,
    cancels_observed: AtomicUsize,
    entered: Mutex<mpsc::Sender<()>>,
}

impl CanonicalInvocationService for CancelRaceService {
    fn prepare(
        &self,
        _invocation: &crate::infrastructure::daemon::server::ActorBoundInvocation,
    ) -> Result<ExecutionClass, Box<DomainResult>> {
        self.prepares.fetch_add(1, Ordering::SeqCst);
        Ok(self.class)
    }

    fn execute(
        &self,
        _invocation: &crate::infrastructure::daemon::server::ActorBoundExecution,
        cancellation: CancellationToken,
    ) -> Result<DomainResult, InvocationFailure> {
        self.executions.fetch_add(1, Ordering::SeqCst);
        let _ = self.entered.lock().unwrap().send(());
        let until = Instant::now() + Duration::from_secs(30);
        while !cancellation.is_cancelled() && Instant::now() < until {
            thread::sleep(Duration::from_millis(5));
        }
        if cancellation.is_cancelled() {
            self.cancels_observed.fetch_add(1, Ordering::SeqCst);
        }
        Err(InvocationFailure::new(
            "cancelled",
            "race execution cancelled",
        ))
    }
}

/// #1395: an exact cancel arrives while a handoff has created the Task row
/// but not yet its link. Whatever the path, the cancel reaches that Task, no
/// cancel row is left beside it, the daemon keeps serving, and an attempt
/// that has not started yet never starts. With `cancel_races_start` the
/// cancel reads the continued Task, the Task starts, and only then the cancel
/// writes: it reads the Task again and ends the running attempt.
fn cancel_during_task_materialization(race: MaterializationRace, cancel_races_start: bool) {
    cancel_during_task_materialization_by(race, cancel_races_start, CancelBy::ExactKey);
}

/// How the racing cancel names the call (#1399).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CancelBy {
    /// The exact receipt key of the call.
    ExactKey,
    /// Only the `taskId` of its Task.
    TaskId,
}

fn cancel_during_task_materialization_by(
    race: MaterializationRace,
    cancel_races_start: bool,
    cancel_by: CancelBy,
) {
    let state = tempfile::tempdir().expect("temporary daemon state");
    let state_root = std::fs::canonicalize(state.path()).expect("physical state root");
    let (_workspace, root) = catalog_workspace();
    let admission = (race == MaterializationRace::PromisedAdmission).then(|| {
        crate::infrastructure::daemon::server::admission_test_control::AdmissionPause::install(
            root.clone(),
        )
    });
    let clock = Arc::new(InspectionClock {
        start: Instant::now(),
        elapsed_ms: AtomicU64::new(0),
    });
    let inline = matches!(
        race,
        MaterializationRace::InlineCutoff | MaterializationRace::InlineCutoffMarkedBeforeTask
    );
    let (entered_tx, entered_rx) = mpsc::channel();
    let service = Arc::new(CancelRaceService {
        class: if inline {
            ExecutionClass::InlineCandidate
        } else {
            ExecutionClass::KnownLong(KnownLongReason::ExternalProcess)
        },
        prepares: AtomicUsize::new(0),
        executions: AtomicUsize::new(0),
        cancels_observed: AtomicUsize::new(0),
        entered: Mutex::new(entered_tx),
    });
    let (held_tx, held_rx) = mpsc::channel();
    let (cancel_write_tx, cancel_write_rx) = mpsc::channel();
    let (started_tx, started_rx) = mpsc::channel();
    let (materializing_tx, materializing_rx) = mpsc::channel();
    let hooks = Arc::new(MaterializationRaceHooks {
        hold_point: match race {
            MaterializationRace::PromotedBegun => Some(V5PausePoint::PrepareEntered),
            MaterializationRace::PromotedActorBound => Some(V5PausePoint::ActorBound),
            MaterializationRace::InlineCutoff
            | MaterializationRace::InlineCutoffMarkedBeforeTask
            | MaterializationRace::PromisedAdmission => None,
        },
        hold_before_task: race == MaterializationRace::InlineCutoffMarkedBeforeTask,
        hold_cancel_write: cancel_races_start,
        cancel_write_held: Mutex::new(cancel_write_tx),
        cancel_write_gate: (Mutex::new(false), std::sync::Condvar::new()),
        started: Mutex::new(started_tx),
        attempt_held: Mutex::new(held_tx),
        attempt_gate: (Mutex::new(false), std::sync::Condvar::new()),
        attempt_passes: AtomicUsize::new(0),
        materializing: Mutex::new(materializing_tx),
        materialize_gate: (Mutex::new(false), std::sync::Condvar::new()),
        start_gate: (Mutex::new(false), std::sync::Condvar::new()),
        creates: AtomicUsize::new(0),
        fail_stopped: AtomicBool::new(false),
    });
    let canonical_runtime = Arc::new(V5CanonicalInvocationRuntime::new(
        service.clone(),
        clock.clone(),
    ));
    let identity = CoreIdentity::production_v5();
    let config = DaemonServerConfig::new(
        state_root.clone(),
        identity.clone(),
        Duration::from_secs(600),
    )
    .with_canonical_runtime_for_test(canonical_runtime)
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
    wait_for_v5_record(&state_root, &identity);
    let mut owner = V5DaemonProcessOwner::connect_or_spawn(
        &state_root,
        identity.clone(),
        std::path::PathBuf::from("unused-existing-v5-endpoint"),
        Duration::from_secs(2),
    )
    .expect("connect v5 owner");
    let task_id = TaskId::new();
    let call = view_catalog_request(task_id, &root);
    let key = ReceiptKey::new(
        call.invocation_id(),
        task_id,
        RequestIdentity::new(
            identity.digest().clone(),
            V5ToolIdentity::View,
            normalized_arguments_hash(call.arguments()),
            request_scope_hash(call.workspace_hint()).expect("request scope"),
        ),
    );

    let mut submit_session = owner
        .connect_peer_before(Instant::now() + Duration::from_secs(2))
        .expect("connect the submit session");
    let submit = thread::spawn(move || submit_session.submit_invocation(call));
    match race {
        MaterializationRace::PromotedBegun | MaterializationRace::PromotedActorBound => held_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("the attempt stops before the cutoff"),
        MaterializationRace::InlineCutoff | MaterializationRace::InlineCutoffMarkedBeforeTask => {
            entered_rx
                .recv_timeout(Duration::from_secs(10))
                .expect("the inline execution runs")
        }
        MaterializationRace::PromisedAdmission => admission
            .as_ref()
            .expect("admission pause")
            .wait_until_entered(),
    }
    clock.elapsed_ms.store(7_000, Ordering::SeqCst);
    // These two answer with the handoff intent; the attempt materializes the
    // Task as its continuation once released.
    let answered_early = matches!(
        race,
        MaterializationRace::PromotedActorBound | MaterializationRace::PromisedAdmission
    );
    let mut submit = Some(submit);
    let mut answered = None;
    if answered_early {
        answered = Some(submitted_task_snapshot(
            submit.take().unwrap().join().expect("submit thread"),
        ));
        match race {
            MaterializationRace::PromotedActorBound => {
                MaterializationRaceHooks::open_gate(&hooks.attempt_gate);
            }
            _ => admission.as_ref().expect("admission pause").release(),
        }
    }
    materializing_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("the handoff stands before its Task link");

    let mut cancel_session = owner
        .connect_peer_before(Instant::now() + Duration::from_secs(2))
        .expect("connect the cancel session");
    let cancel_key = key.clone();
    let cancel = thread::spawn(move || match cancel_by {
        CancelBy::ExactKey => cancel_session.cancel_invocation(cancel_key),
        CancelBy::TaskId => cancel_session.cancel_task(task_id),
    });
    // A cancel that does not wait for the handoff lands in the window now.
    thread::sleep(Duration::from_millis(200));
    MaterializationRaceHooks::open_gate(&hooks.materialize_gate);
    if cancel_races_start {
        cancel_write_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("the cancel read the Task");
        MaterializationRaceHooks::open_gate(&hooks.start_gate);
        started_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("the Task started under the cancel");
        MaterializationRaceHooks::open_gate(&hooks.cancel_write_gate);
    }

    let cancelled = cancel
        .join()
        .expect("cancel thread")
        .expect("cancel answered");
    if let Some(submit) = submit.take() {
        answered = Some(submitted_task_snapshot(
            submit.join().expect("submit thread"),
        ));
    }
    assert_eq!(answered.expect("submit answered").task_id(), task_id);
    MaterializationRaceHooks::open_gate(&hooks.attempt_gate);
    MaterializationRaceHooks::open_gate(&hooks.start_gate);
    assert!(
        matches!(
            &cancelled,
            V5ServerResponse::Invocation {
                outcome: V5InvocationResponse::Task { snapshot }
            } | V5ServerResponse::Task { snapshot }
                if snapshot.task_id() == task_id && snapshot.cancel_requested()
        ),
        "{race:?}/{cancel_by:?}: the cancel reaches the Task: {cancelled:?}"
    );
    let terminal = wait_task_terminal(&mut owner, task_id, Duration::from_secs(30));
    assert!(
        matches!(terminal, V5DaemonTaskSnapshot::Cancelled { .. }),
        "{race:?}/{cancel_by:?}: the Task ends cancelled: {terminal:?}"
    );
    let recovered = owner
        .recover_invocation_receipt(key)
        .expect("recover the call");
    assert!(
        matches!(
            &recovered,
            V5ServerResponse::Invocation {
                outcome: V5InvocationResponse::Task { snapshot }
            } if snapshot.task_id() == task_id
        ),
        "{race:?}: recovery reads the Task, not a stray cancel row: {recovered:?}"
    );
    if inline || cancel_races_start {
        // The execution was already running, or may be: the cancel stops it.
        assert_eq!(
            service.cancels_observed.load(Ordering::SeqCst),
            service.executions.load(Ordering::SeqCst),
            "{race:?}: an execution ran past the cancel"
        );
        assert!(service.prepares.load(Ordering::SeqCst) <= 1);
    } else {
        assert_eq!(
            service.prepares.load(Ordering::SeqCst),
            0,
            "{race:?}: the cancelled attempt was prepared"
        );
        assert_eq!(service.executions.load(Ordering::SeqCst), 0);
    }
    assert!(
        !hooks.fail_stopped.load(Ordering::SeqCst),
        "{race:?}/{cancel_by:?}: the daemon stopped"
    );
    drop(owner);
    drop(admission);
    stop.store(true, Ordering::SeqCst);
    server
        .join()
        .expect("daemon thread did not panic")
        .expect("daemon exits cleanly");
}

#[test]
fn exact_cancel_during_the_begun_promotion_cancels_the_unstarted_task() {
    cancel_during_task_materialization(MaterializationRace::PromotedBegun, false);
}

#[test]
fn exact_cancel_during_the_handoff_continuation_cancels_the_unstarted_task() {
    cancel_during_task_materialization(MaterializationRace::PromotedActorBound, false);
}

#[test]
fn exact_cancel_during_the_inline_cutoff_cancels_the_running_task() {
    cancel_during_task_materialization(MaterializationRace::InlineCutoff, false);
}

#[test]
fn exact_cancel_during_the_promised_continuation_cancels_the_unstarted_task() {
    cancel_during_task_materialization(MaterializationRace::PromisedAdmission, false);
}

#[test]
fn exact_cancel_marking_the_inline_handoff_cancels_the_running_task() {
    cancel_during_task_materialization(MaterializationRace::InlineCutoffMarkedBeforeTask, false);
}

/// The continued Task starts between the cancel's read and its write: the
/// moved Task neither fails the cancel nor stops the daemon.
#[test]
fn exact_cancel_racing_the_start_of_the_continued_task_ends_it_cancelled() {
    cancel_during_task_materialization(MaterializationRace::PromotedActorBound, true);
}

/// #1399: a cancel by `taskId` alone, landing in the same handoff windows,
/// waits for the handoff completion like the exact-key cancel: it reaches
/// the Task, an unstarted attempt never starts, and the daemon keeps serving.
#[test]
fn task_id_cancel_during_the_begun_promotion_cancels_the_unstarted_task() {
    cancel_during_task_materialization_by(
        MaterializationRace::PromotedBegun,
        false,
        CancelBy::TaskId,
    );
}

#[test]
fn task_id_cancel_during_the_handoff_continuation_cancels_the_unstarted_task() {
    cancel_during_task_materialization_by(
        MaterializationRace::PromotedActorBound,
        false,
        CancelBy::TaskId,
    );
}

#[test]
fn task_id_cancel_during_the_inline_cutoff_cancels_the_running_task() {
    cancel_during_task_materialization_by(
        MaterializationRace::InlineCutoff,
        false,
        CancelBy::TaskId,
    );
}

#[test]
fn task_id_cancel_marking_the_inline_handoff_cancels_the_running_task() {
    cancel_during_task_materialization_by(
        MaterializationRace::InlineCutoffMarkedBeforeTask,
        false,
        CancelBy::TaskId,
    );
}

#[test]
fn task_id_cancel_during_the_promised_continuation_cancels_the_unstarted_task() {
    cancel_during_task_materialization_by(
        MaterializationRace::PromisedAdmission,
        false,
        CancelBy::TaskId,
    );
}

#[test]
fn task_id_cancel_racing_the_start_of_the_continued_task_ends_it_cancelled() {
    cancel_during_task_materialization_by(
        MaterializationRace::PromotedActorBound,
        true,
        CancelBy::TaskId,
    );
}

/// Holds the staged handoff completion after its Task link is published and
/// before the staged terminal is: the window a `taskId` cancel used to land
/// in without the key's lock (#1399).
struct StagedLinkWindowHooks {
    armed: AtomicBool,
    linked: Mutex<mpsc::Sender<()>>,
    gate: (Mutex<bool>, std::sync::Condvar),
    fail_stopped: AtomicBool,
}

impl V5RuntimeHooks for StagedLinkWindowHooks {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn event(&self, event: V5ReceiptRuntimeEventKind, _epoch_ms: u64) {
        if event == V5ReceiptRuntimeEventKind::TaskLinkReservationConverted
            && self.armed.swap(false, Ordering::SeqCst)
        {
            let _ = self.linked.lock().unwrap().send(());
            MaterializationRaceHooks::wait_gate(&self.gate);
        }
    }

    fn forced_process_exit(&self, _grace: Option<Duration>) {
        self.fail_stopped.store(true, Ordering::SeqCst);
    }
}

/// #1399: a staged handoff has published its Task link, not yet its staged
/// terminal, and a cancel names only the `taskId`. The cancel waits for the
/// completion under the key's lock: the staged terminal wins, the cancel
/// reads it, and the daemon keeps serving. Without the lock the cancel ended
/// the not-begun Task itself and the staged publication found a moved Task.
#[test]
fn task_id_cancel_waits_for_the_staged_handoff_completion() {
    for phase in [AttemptPhase::NotBegun, AttemptPhase::Begun] {
        let root = tempfile::tempdir().expect("temporary staged root");
        let state_root = std::fs::canonicalize(root.path()).expect("physical staged root");
        let identity = CoreIdentity::production_v5();
        let state = DaemonStateDirectory::open(&state_root, &identity).expect("open staged state");
        let (linked_tx, linked_rx) = mpsc::channel();
        let hooks = Arc::new(StagedLinkWindowHooks {
            armed: AtomicBool::new(true),
            linked: Mutex::new(linked_tx),
            gate: (Mutex::new(false), std::sync::Condvar::new()),
            fail_stopped: AtomicBool::new(false),
        });
        let service = Arc::new(CountingService::default());
        let config =
            DaemonServerConfig::new(state_root, identity.clone(), Duration::from_millis(50))
                .with_invocation_service(service.clone())
                .with_runtime_hooks_for_test(hooks.clone());
        let clock = Arc::new(ManualEpochClock::new(1_009));
        let runtime = Arc::new(
            V5ReceiptRuntime::open_with_epoch_clock(&state, &config, clock)
                .expect("open staged runtime"),
        );
        let handoff = prepare_staged_startup_handoff(&runtime, &identity, phase);
        let task_id = handoff.key().reserved_task_id();
        let winner = canonical_v5_terminal(&ReceiptTerminalOutcome::Completed {
            result: Box::new(DomainResult::success("staged winner")),
        })
        .expect("canonical staged winner");

        let publisher = {
            let runtime = Arc::clone(&runtime);
            thread::spawn(move || {
                runtime.publish_staged_handoff_terminal_reply(
                    handoff,
                    winner,
                    1_009,
                    Instant::now() + Duration::from_secs(10),
                )
            })
        };
        linked_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("the staged Task link is published");
        let canceller = {
            let runtime = Arc::clone(&runtime);
            thread::spawn(move || {
                runtime.cancel_task(task_id, Instant::now() + Duration::from_secs(10))
            })
        };
        // A cancel that ignores the key's lock finishes in the window now.
        let until = Instant::now() + Duration::from_millis(500);
        while Instant::now() < until && !canceller.is_finished() {
            thread::sleep(Duration::from_millis(10));
        }
        MaterializationRaceHooks::open_gate(&hooks.gate);

        let published = publisher.join().expect("publisher thread");
        assert!(
            published.is_ok(),
            "{phase:?}: the staged handoff completes: {:?}",
            published.as_ref().map(|_| "reply")
        );
        let cancelled = canceller
            .join()
            .expect("cancel thread")
            .unwrap_or_else(|error| panic!("{phase:?}: the cancel is answered: {error}"));
        assert!(
            matches!(&cancelled, V5DaemonTaskSnapshot::Completed { .. })
                && cancelled.task_id() == task_id,
            "{phase:?}: the cancel reads the staged terminal: {cancelled:?}"
        );
        assert!(
            !runtime.restart_required() && !hooks.fail_stopped.load(Ordering::SeqCst),
            "{phase:?}: the daemon stopped"
        );
        assert_eq!(service.prepares.load(Ordering::SeqCst), 0);
        assert_eq!(service.executions.load(Ordering::SeqCst), 0);
    }
}

/// Holds a begun attempt after its early look for a Task the handler
/// materialized and before it claims the cutoff (#1399).
struct CutoffClaimRaceHooks {
    held: Mutex<mpsc::Sender<()>>,
    gate: (Mutex<bool>, std::sync::Condvar),
    passes: AtomicUsize,
    fail_stopped: AtomicBool,
}

impl V5RuntimeHooks for CutoffClaimRaceHooks {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn pause(&self, point: V5PausePoint, _deadline: Instant) -> Result<(), ReceiptLedgerError> {
        if point == V5PausePoint::BeforeCutoffClaim
            && self.passes.fetch_add(1, Ordering::SeqCst) == 0
        {
            let _ = self.held.lock().unwrap().send(());
            MaterializationRaceHooks::wait_gate(&self.gate);
        }
        Ok(())
    }

    fn forced_process_exit(&self, _grace: Option<Duration>) {
        self.fail_stopped.store(true, Ordering::SeqCst);
    }
}

/// #1399: the begun attempt looked for a materialized Task before the
/// handler made one, then the cutoff passed and the handler materialized
/// the working Task and answered with it. The attempt, claiming the cutoff
/// only now, takes that Task and executes it once; the daemon keeps serving.
#[test]
fn begun_promotion_after_the_early_look_executes_the_materialized_task() {
    let state = tempfile::tempdir().expect("temporary daemon state");
    let state_root = std::fs::canonicalize(state.path()).expect("physical state root");
    let (_workspace, root) = catalog_workspace();
    let clock = Arc::new(InspectionClock {
        start: Instant::now(),
        elapsed_ms: AtomicU64::new(0),
    });
    let service = Arc::new(CountingService::default());
    let (held_tx, held_rx) = mpsc::channel();
    let hooks = Arc::new(CutoffClaimRaceHooks {
        held: Mutex::new(held_tx),
        gate: (Mutex::new(false), std::sync::Condvar::new()),
        passes: AtomicUsize::new(0),
        fail_stopped: AtomicBool::new(false),
    });
    let canonical_runtime = Arc::new(V5CanonicalInvocationRuntime::new(
        service.clone(),
        clock.clone(),
    ));
    let identity = CoreIdentity::production_v5();
    let config = DaemonServerConfig::new(
        state_root.clone(),
        identity.clone(),
        Duration::from_secs(600),
    )
    .with_canonical_runtime_for_test(canonical_runtime)
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
    wait_for_v5_record(&state_root, &identity);
    let mut owner = V5DaemonProcessOwner::connect_or_spawn(
        &state_root,
        identity.clone(),
        std::path::PathBuf::from("unused-existing-v5-endpoint"),
        Duration::from_secs(2),
    )
    .expect("connect v5 owner");
    let task_id = TaskId::new();
    let call = view_catalog_request(task_id, &root);

    let mut submit_session = owner
        .connect_peer_before(Instant::now() + Duration::from_secs(2))
        .expect("connect the submit session");
    let submit = thread::spawn(move || submit_session.submit_invocation(call));
    held_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("the begun attempt passed its early look");
    clock.elapsed_ms.store(7_000, Ordering::SeqCst);
    // The handler promotes the begun attempt and answers with the working
    // Task it materialized while the attempt still stands before its claim.
    let answered = submitted_task_snapshot(submit.join().expect("submit thread"));
    assert_eq!(answered.task_id(), task_id);
    assert!(
        matches!(answered, V5DaemonTaskSnapshot::Working { .. }),
        "the handler materialized the working Task: {answered:?}"
    );
    MaterializationRaceHooks::open_gate(&hooks.gate);

    let terminal = wait_task_terminal(&mut owner, task_id, Duration::from_secs(30));
    assert!(
        matches!(&terminal, V5DaemonTaskSnapshot::Completed { result, .. } if result.ok),
        "the materialized Task runs to its outcome: {terminal:?}"
    );
    assert_eq!(service.prepares.load(Ordering::SeqCst), 1);
    assert_eq!(service.executions.load(Ordering::SeqCst), 1);
    assert!(
        !hooks.fail_stopped.load(Ordering::SeqCst),
        "the daemon stopped"
    );
    drop(owner);
    stop.store(true, Ordering::SeqCst);
    server
        .join()
        .expect("daemon thread did not panic")
        .expect("daemon exits cleanly");
}
