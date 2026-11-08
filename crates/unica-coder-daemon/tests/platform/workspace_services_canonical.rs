//! Сохранённая правка через демон и живые помощники службы рабочей
//! области: инвалидация, отмена и замена анализатора. Служба, помощник и
//! запросы — из `workspace_service_fixture` нижнего слоя.

use crate::infrastructure::analysis_composition;
use crate::infrastructure::workspace_service_fixture::*;
use serde_json::{json, Value};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{atomic::Ordering, mpsc, Arc, Condvar, Mutex};
use std::time::Duration;
use unica_analysis_services::protocol::*;
use unica_analysis_services::test_support::*;
use unica_analysis_services::{WorkspaceServiceIdentity, WorkspaceServiceRecord};
use unica_source_contract::events::{DomainEvent, DomainEventKind};
use unica_source_contract::workspace::WorkspaceContext;
use unica_work_control::cancellation::CancellationToken;
use unica_work_control::provider_deadline::ProviderDeadline;

fn write_record(identity: &WorkspaceServiceIdentity, record: WorkspaceServiceRecord) {
    unica_analysis_services::test_support::write_record(identity, &record).unwrap();
}

// The real service prepares and executes every operation. Held executions
// are declared known-long only for this fixture's scheduling; no process
// starts and the original cancellation token retains ordinary safe points.
struct HeldApplyTaskService {
    inner: Arc<dyn crate::infrastructure::daemon::server::CanonicalInvocationService>,
}

impl crate::infrastructure::daemon::server::CanonicalInvocationService for HeldApplyTaskService {
    fn prepare(
        &self,
        invocation: &crate::infrastructure::daemon::server::ActorBoundInvocation,
    ) -> Result<
        unica_execution::receipt_runtime::ports::ExecutionClass,
        Box<unica_execution_contract::invocation::DomainResult>,
    > {
        let class = self.inner.prepare(invocation)?;
        Ok(
            if invocation.tool() == unica_execution::invocation_store::ToolIdentity::Apply
                && invocation.arguments().contains_key("executionToken")
            {
                unica_execution::receipt_runtime::ports::ExecutionClass::KnownLong(
                    unica_execution::receipt_runtime::ports::KnownLongReason::ExternalProcess,
                )
            } else {
                class
            },
        )
    }

    fn execute(
        &self,
        invocation: &crate::infrastructure::daemon::server::ActorBoundExecution,
        cancellation: CancellationToken,
    ) -> Result<
        unica_execution_contract::invocation::DomainResult,
        unica_execution_contract::invocation::InvocationFailure,
    > {
        assert!(
            !cancellation.protected_process_started(),
            "fixture classification must not protect a process"
        );
        let result = self.inner.execute(invocation, cancellation.clone());
        assert!(
            !cancellation.protected_process_started(),
            "canonical XML Apply must not acquire process authority"
        );
        result
    }
}

struct CanonicalInvalidationFixture {
    context: WorkspaceContext,
    executable: PathBuf,
    daemon: Option<crate::infrastructure::daemon::server::actor_capacity_tests::LiveV5Daemon>,
    owner: Option<unica_daemon::client_v5::V5DaemonProcessOwner>,
    _directory: tempfile::TempDir,
}

impl CanonicalInvalidationFixture {
    fn new() -> Self {
        Self::with_held_apply_schedule(false)
    }

    fn new_for_held_apply() -> Self {
        Self::with_held_apply_schedule(true)
    }

    fn with_held_apply_schedule(held: bool) -> Self {
        use crate::infrastructure::daemon::server::actor_capacity_tests::{
            canonical_v13_service, LiveV5Daemon,
        };
        let directory = tempfile::tempdir().unwrap();
        let workspace =
            unica_workspace::test_support::canonical_path(directory.path()).join("workspace");
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/acceptance/workspace-symbol");
        for relative in [
            "v8project.yaml",
            "src/Configuration.xml",
            "src/CommonModules/SearchProbe.xml",
            "src/CommonModules/SearchProbe/Ext/Module.bsl",
        ] {
            let target = workspace.join(relative);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::copy(fixture.join(relative), target).unwrap();
        }
        let context =
            crate::infrastructure::workspace::discover_workspace(Some(workspace)).unwrap();
        assert_eq!(
            context.cache_root,
            context.workspace_root.join(".build/unica")
        );
        let executable = unica_process::test_support::fixture_executable_path(
            directory.path(),
            "snapshot-provider",
        );
        let output = Command::new("rustc")
            .arg("--edition=2021")
            .arg(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../tests/fixtures/analyzer/workspace-snapshot-producer.rs"),
            )
            .arg("-o")
            .arg(&executable)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "compile controlled resident provider"
        );
        let inner = canonical_v13_service();
        let service: Arc<dyn crate::infrastructure::daemon::server::CanonicalInvocationService> =
            if held {
                Arc::new(HeldApplyTaskService { inner })
            } else {
                inner
            };
        let daemon = LiveV5Daemon::start_with_owned_stop(service);
        let owner = daemon.owner();
        Self {
            context,
            executable,
            daemon: Some(daemon),
            owner: Some(owner),
            _directory: directory,
        }
    }

    fn apply(&self, arguments: Value) -> unica_execution_contract::invocation::DomainResult {
        use crate::infrastructure::daemon::server::actor_capacity_tests::V5Submission;
        use unica_execution::receipt_runtime::response::V5DaemonTaskSnapshot;
        let request = crate::infrastructure::daemon::protocol::InvocationRequest::new(
            unica_execution::invocation_store::ToolIdentity::Apply,
            arguments,
            self.context.workspace_root.to_string_lossy(),
            7_000,
        )
        .unwrap();
        let daemon = self.daemon.as_ref().unwrap();
        let owner = self.owner.as_ref().unwrap();
        match daemon.submit(owner, &request) {
            V5Submission::Direct(result) => *result,
            V5Submission::Task(task) => {
                match daemon.wait_terminal(owner, task, Duration::from_secs(15)) {
                    V5DaemonTaskSnapshot::Completed { result, .. } => *result,
                    other => panic!("canonical apply did not complete: {other:?}"),
                }
            }
        }
    }

    fn plan(&self, text: &str) -> unica_execution_contract::invocation::DomainResult {
        self.apply(json!({
            "at": "main:CommonModule.SearchProbe",
            "ops": [{"op": "code.replace", "args": {
                "at": "main:CommonModule.SearchProbe.Body", "text": text,
            }}],
        }))
    }

    fn prepare_snapshot_module(&self, line_ending: &str) -> (PathBuf, Vec<u8>) {
        assert!(matches!(line_ending, "\n" | "\r\n"));
        let module = self
            .context
            .workspace_root
            .join("src")
            .join(SNAPSHOT_MODULE);
        let lf_before = SNAPSHOT_BEFORE.as_bytes();
        let crlf_before = SNAPSHOT_BEFORE.replace('\n', "\r\n");
        let fixture_bytes = fs::read(&module).unwrap();
        assert!(
            fixture_bytes == lf_before || fixture_bytes == crlf_before.as_bytes(),
            "the controlled module must match its known LF or CRLF source"
        );
        let chosen_before = SNAPSHOT_BEFORE.replace('\n', line_ending);
        fs::write(&module, &chosen_before).unwrap();
        let before = fs::read(&module).unwrap();
        assert_eq!(before, chosen_before.as_bytes());
        let expected = if before == lf_before {
            SNAPSHOT_AFTER.as_bytes().to_vec()
        } else if before == crlf_before.as_bytes() {
            SNAPSHOT_AFTER.replace('\n', "\r\n").into_bytes()
        } else {
            panic!("the controlled source must retain its exact chosen line ending")
        };
        (module, expected)
    }

    fn helper(&self) -> CanonicalInvalidationHelper {
        CanonicalInvalidationHelper::start(
            &self.context,
            &self.context.workspace_root.join("src"),
            &self.executable,
        )
    }
}

impl Drop for CanonicalInvalidationFixture {
    fn drop(&mut self) {
        if let (Some(mut daemon), Some(owner)) = (self.daemon.take(), self.owner.take()) {
            daemon.stop(owner);
        }
    }
}

struct InvalidationDispatchRegistration(PathBuf);

impl InvalidationDispatchRegistration {
    fn install(context: &WorkspaceContext, hook: Arc<InvalidationDispatchHook>) -> Self {
        assert!(invalidation_dispatch_hooks()
            .lock()
            .unwrap()
            .insert(context.workspace_root.clone(), hook)
            .is_none());
        Self(context.workspace_root.clone())
    }
}

impl Drop for InvalidationDispatchRegistration {
    fn drop(&mut self) {
        invalidation_dispatch_hooks()
            .lock()
            .unwrap()
            .remove(&self.0);
    }
}

struct InvalidationTestRelease(Arc<(Mutex<bool>, Condvar)>);

impl Drop for InvalidationTestRelease {
    fn drop(&mut self) {
        *self.0 .0.lock().unwrap() = true;
        self.0 .1.notify_all();
    }
}

/// rule: INV.RUNTIME.SERVICE-MUTATION-INVALIDATION
#[test]
fn canonical_saved_apply_cancel_finishes_before_the_real_helper_ack_is_released() {
    use crate::infrastructure::daemon::server::actor_capacity_tests::V5Submission;
    use unica_execution::receipt_runtime::response::V5DaemonTaskSnapshot;

    let fixture = CanonicalInvalidationFixture::new_for_held_apply();
    let (module, expected) = fixture.prepare_snapshot_module("\n");
    let helper = fixture.helper();
    assert!(helper.snapshot().contains("AcceptanceExistingSymbol1306"));
    let preview = fixture.plan(SNAPSHOT_AFTER);
    assert!(preview.ok, "{preview:?}");
    let token = preview.data.as_ref().unwrap()["executionToken"]
        .as_str()
        .unwrap();
    let (entered, observed) = mpsc::channel();
    let release = Arc::new((Mutex::new(false), Condvar::new()));
    helper
        .runtime
        .test_set_invalidation_response_hook(Some(Arc::new({
            let release = Arc::clone(&release);
            move || {
                entered.send(()).unwrap();
                let mut released = release.0.lock().unwrap();
                while !*released {
                    released = release.1.wait(released).unwrap();
                }
            }
        })));
    // Declared after both owners: unwind releases the real helper before shutdown.
    let cleanup = InvalidationTestRelease(Arc::clone(&release));
    let request = crate::infrastructure::daemon::protocol::InvocationRequest::new(
        unica_execution::invocation_store::ToolIdentity::Apply,
        json!({"executionToken": token}),
        fixture.context.workspace_root.to_string_lossy(),
        7_000,
    )
    .unwrap();
    let daemon = fixture.daemon.as_ref().unwrap();
    let owner = fixture.owner.as_ref().unwrap();
    let V5Submission::Task(task) = daemon.submit(owner, &request) else {
        panic!("an unacknowledged helper must keep the canonical apply in a Task");
    };
    observed.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(helper
        .runtime
        .test_analyzer_invalidated()
        .load(Ordering::Acquire));
    assert_eq!(fs::read(&module).unwrap(), expected);
    let source_identity =
        unica_workspace::state_fs::test_support::file_identity(&fs::File::open(&module).unwrap())
            .unwrap();
    let state = fs::read(fixture.context.cache_root.join("state.json")).unwrap();
    let cache: Value = serde_json::from_slice(&state).unwrap();
    assert_eq!(cache["caches"]["bsl_index"]["status"], "stale");
    assert_eq!(cache["caches"]["bsl_diagnostics"]["status"], "stale");
    daemon.cancel(owner, task);
    let terminal = daemon.wait_terminal(owner, task, Duration::from_secs(2));
    assert!(
        !*release.0.lock().unwrap(),
        "terminal must precede ACK release"
    );
    let V5DaemonTaskSnapshot::Completed { result, .. } = terminal else {
        panic!("committed apply must retain its Completed receipt: {terminal:?}");
    };
    assert!(result.ok, "{result:?}");
    assert!(result.rev.is_some());
    assert!(!result.changed.is_empty());
    assert_eq!(
        result.data.as_ref().unwrap()["planHash"],
        preview.data.as_ref().unwrap()["planHash"]
    );
    let notifications = helper.notifications();
    assert_eq!(notifications.len(), 1);
    assert_eq!(fixture.apply(json!({"executionToken": token})), *result);
    assert_eq!(
        helper.notifications(),
        notifications,
        "saved replay must not notify again"
    );
    assert_eq!(fs::read(&module).unwrap(), expected);
    assert_eq!(
        unica_workspace::state_fs::test_support::file_identity(&fs::File::open(&module).unwrap(),)
            .unwrap(),
        source_identity
    );
    assert_eq!(
        fs::read(fixture.context.cache_root.join("state.json")).unwrap(),
        state
    );
    drop(cleanup);
    assert!(helper.snapshot().contains("AcceptanceChangedSymbol978"));
    helper.stop_and_verify();
}

/// rule: INV.RUNTIME.SERVICE-MUTATION-INVALIDATION
#[test]
fn canonical_saved_apply_cancel_before_dispatch_or_between_records_keeps_unknown_old_results() {
    use crate::infrastructure::daemon::server::actor_capacity_tests::V5Submission;
    use unica_code_intelligence::model::CodeIntelligenceContext;
    use unica_code_intelligence::model::CodeIntelligenceProvider;
    use unica_code_intelligence::model::SearchRequest;
    use unica_execution::receipt_runtime::response::V5DaemonTaskSnapshot;

    for stop_after in [0, 1] {
        let fixture = CanonicalInvalidationFixture::new_for_held_apply();
        let (module, expected) = fixture.prepare_snapshot_module("\n");
        let helper = fixture.helper();
        let replica = CanonicalInvalidationHelper::start_in_directory(
            &fixture.context,
            &fixture.context.workspace_root.join("src"),
            &fixture.executable,
            None,
            Some(
                fixture
                    .context
                    .cache_root
                    .join("services/controlled-second-record"),
            ),
        );
        let source_root = fixture.context.workspace_root.join("src");
        let context = CodeIntelligenceContext::new(
            fixture.context.clone(),
            unica_source_contract::workspace::ResolvedSourceRoot {
                source_set: Some("main".into()),
                path: source_root.clone(),
            },
            std::sync::Arc::new(unica_workspace::discovery::ProjectDiscoveryReader::new(
                &fixture.context.workspace_root,
            )),
        );
        let symbol = || {
            analysis_composition::bsl_provider().search(
                &SearchRequest {
                    query: "Acceptance".into(),
                    limit: 20,
                },
                &context,
                ProviderDeadline::from_budget(Duration::from_secs(10)),
                &CancellationToken::new(),
            )
        };
        let primary_identity = helper.runtime.test_identity();
        let mut initial = Vec::new();
        for current in [&helper, &replica] {
            // Select this controlled resident using the ordinary provider adapter.
            write_record(primary_identity, current.record.clone());
            let bsl = symbol();
            assert_eq!(bsl.hits.len(), 1, "{bsl:?}");
            assert_eq!(bsl.index_freshness.as_deref(), Some("unknown"));
            initial.push((bsl.hits, current.pids()));
        }
        write_record(primary_identity, helper.record.clone());
        let preview = fixture.plan(SNAPSHOT_AFTER);
        assert!(preview.ok, "{preview:?}");
        let token = preview.data.as_ref().unwrap()["executionToken"]
            .as_str()
            .unwrap();
        let (entered, observed) = mpsc::channel();
        let release = Arc::new((Mutex::new(false), Condvar::new()));
        let registration = InvalidationDispatchRegistration::install(
            &fixture.context,
            Arc::new({
                let release = Arc::clone(&release);
                move |dispatched| {
                    if dispatched == stop_after {
                        entered.send(()).unwrap();
                        let mut released = release.0.lock().unwrap();
                        while !*released {
                            released = release.1.wait(released).unwrap();
                        }
                    }
                }
            }),
        );
        let cleanup = InvalidationTestRelease(Arc::clone(&release));
        let request = crate::infrastructure::daemon::protocol::InvocationRequest::new(
            unica_execution::invocation_store::ToolIdentity::Apply,
            json!({"executionToken": token}),
            fixture.context.workspace_root.to_string_lossy(),
            7_000,
        )
        .unwrap();
        let daemon = fixture.daemon.as_ref().unwrap();
        let owner = fixture.owner.as_ref().unwrap();
        let V5Submission::Task(task) = daemon.submit(owner, &request) else {
            panic!("dispatch gate must retain the canonical apply Task");
        };
        observed.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(fs::read(&module).unwrap(), expected);
        let state = fs::read(fixture.context.cache_root.join("state.json")).unwrap();
        let cache: Value = serde_json::from_slice(&state).unwrap();
        assert_eq!(cache["caches"]["bsl_index"]["status"], "stale");
        assert_eq!(cache["caches"]["bsl_diagnostics"]["status"], "stale");
        daemon.cancel(owner, task);
        drop(cleanup);
        let terminal = daemon.wait_terminal(owner, task, Duration::from_secs(2));
        let V5DaemonTaskSnapshot::Completed { result, .. } = terminal else {
            panic!("committed publication must remain Completed: {terminal:?}");
        };
        assert!(result.ok, "{result:?}");
        drop(registration);
        assert_eq!(
            helper.notifications().len() + replica.notifications().len(),
            stop_after
        );
        let notifications = [helper.notifications(), replica.notifications()];
        assert_eq!(fixture.apply(json!({"executionToken": token})), *result);
        assert_eq!(
            [helper.notifications(), replica.notifications()],
            notifications
        );
        assert_eq!(
            fs::read(fixture.context.cache_root.join("state.json")).unwrap(),
            state
        );
        for (current, (old_hits, pids)) in [&helper, &replica].into_iter().zip(initial) {
            if !current.notifications().is_empty() {
                continue;
            }
            write_record(primary_identity, current.record.clone());
            let old_bsl = symbol();
            assert_eq!(old_bsl.hits, old_hits);
            assert_eq!(old_bsl.index_freshness.as_deref(), Some("unknown"));
            assert_eq!(current.pids(), pids);
        }
        write_record(primary_identity, helper.record.clone());
        replica.stop_and_verify();
        helper.stop_and_verify();
    }
}

/// rule: INV.RUNTIME.SERVICE-MUTATION-INVALIDATION
#[test]
fn canonical_saved_apply_notifies_live_helpers_once_and_restarts_the_warmed_analyzer() {
    for line_ending in ["\n", "\r\n"] {
        assert_canonical_saved_apply_notifies_live_helpers_once_and_restarts_the_warmed_analyzer(
            line_ending,
        );
    }
}

fn assert_canonical_saved_apply_notifies_live_helpers_once_and_restarts_the_warmed_analyzer(
    line_ending: &str,
) {
    let fixture = CanonicalInvalidationFixture::new();
    let (module, expected) = fixture.prepare_snapshot_module(line_ending);
    let before = fs::read(&module).unwrap();
    let helper = fixture.helper();
    assert!(helper.snapshot().contains("AcceptanceExistingSymbol1306"));
    let first_pid = helper.pids()[0];

    let mut neighbor_context = fixture.context.clone();
    neighbor_context.workspace_root = fixture._directory.path().join("neighbor");
    neighbor_context.cwd = neighbor_context.workspace_root.clone();
    let neighbor_source = neighbor_context.workspace_root.join("src");
    fs::create_dir_all(neighbor_source.join(SNAPSHOT_MODULE).parent().unwrap()).unwrap();
    fs::write(neighbor_source.join(SNAPSHOT_MODULE), &before).unwrap();
    let neighbor = CanonicalInvalidationHelper::start(
        &neighbor_context,
        &neighbor_source,
        &fixture.executable,
    );
    assert!(neighbor.snapshot().contains("AcceptanceExistingSymbol1306"));

    let preview = fixture.plan(SNAPSHOT_AFTER);
    assert!(preview.ok, "{preview:?}");
    assert_eq!(fs::read(&module).unwrap(), before);
    assert!(helper.notifications().is_empty());
    assert!(helper.snapshot().contains("AcceptanceExistingSymbol1306"));
    assert_eq!(helper.pids(), [first_pid]);

    let token = preview.data.as_ref().unwrap()["executionToken"]
        .as_str()
        .unwrap();
    let published = fixture.apply(json!({"executionToken": token}));
    assert!(published.ok, "{published:?}");
    assert_eq!(fs::read(&module).unwrap(), expected);
    let notifications = helper.notifications();
    assert_eq!(
        notifications.len(),
        1,
        "the canonical commit must reach the actual helper"
    );
    assert_eq!(notifications[0].len(), 1, "{notifications:?}");
    assert_eq!(
        notifications[0][0],
        DomainEvent::new(
            DomainEventKind::ModuleChanged,
            "main:CommonModule.SearchProbe"
        )
    );
    assert!(helper
        .runtime
        .test_analyzer_invalidated()
        .load(Ordering::Acquire));
    assert!(
        neighbor.notifications().is_empty(),
        "a neighbor sharing services must stay isolated"
    );
    assert!(helper.snapshot().contains("AcceptanceChangedSymbol978"));
    let pids = helper.pids();
    assert_eq!(pids.len(), 2);
    assert_ne!(pids[0], pids[1]);
    assert!(wait_for_process_exit(first_pid, Duration::from_secs(5)));

    let replay = fixture.apply(json!({"executionToken": token}));
    assert_eq!(
        serde_json::to_value(replay).unwrap(),
        serde_json::to_value(&published).unwrap()
    );
    assert_eq!(helper.notifications(), notifications);
    assert!(helper.snapshot().contains("AcceptanceChangedSymbol978"));
    assert_eq!(helper.pids(), pids);

    let no_op = fixture.plan(SNAPSHOT_AFTER);
    assert!(no_op.ok, "{no_op:?}");
    let no_op_token = no_op.data.as_ref().unwrap()["executionToken"]
        .as_str()
        .unwrap();
    assert!(fixture.apply(json!({"executionToken": no_op_token})).ok);
    assert_eq!(
        helper.notifications(),
        notifications,
        "empty effects must not invalidate"
    );

    let stale = fixture.plan("Procedure NeverPublished978() Export\nEndProcedure\n");
    assert!(stale.ok, "{stale:?}");
    let stale_token = stale.data.as_ref().unwrap()["executionToken"]
        .as_str()
        .unwrap();
    fs::write(
        &module,
        "Procedure ExternalEdit978() Export\nEndProcedure\n",
    )
    .unwrap();
    let state_before = fs::read(fixture.context.cache_root.join("state.json")).unwrap();
    let refused = fixture.apply(json!({"executionToken": stale_token}));
    assert!(!refused.ok, "{refused:?}");
    assert!(fs::read_to_string(&module)
        .unwrap()
        .contains("ExternalEdit978"));
    assert_eq!(
        fs::read(fixture.context.cache_root.join("state.json")).unwrap(),
        state_before
    );
    assert_eq!(helper.notifications(), notifications);
    assert_eq!(
        serde_json::to_value(fixture.apply(json!({"executionToken": stale_token}))).unwrap(),
        serde_json::to_value(refused).unwrap()
    );
    let invalid = fixture.apply(json!({"at": "main:CommonModule.SearchProbe", "ops": [{"op": "code.replace", "args": {"text": 42}}]}));
    assert!(!invalid.ok);
    assert_eq!(helper.notifications(), notifications);
    assert_eq!(helper.pids(), pids);
    neighbor.stop_and_verify();
    helper.stop_and_verify();
}

#[test]
fn invalidated_analyzer_retirement_observes_cancellation_and_budget_before_replacement() {
    for cancel in [false, true] {
        let fixture = CanonicalInvalidationFixture::new();
        let cleanup_gate = fixture._directory.path().join("hold-analyzer-cleanup");
        fs::write(&cleanup_gate, b"hold").unwrap();
        let helper = CanonicalInvalidationHelper::start_with_cleanup_gate(
            &fixture.context,
            &fixture.context.workspace_root.join("src"),
            &fixture.executable,
            Some(cleanup_gate.clone()),
        );
        assert!(helper.snapshot().contains("AcceptanceExistingSymbol1306"));
        let first_pid = helper.pids()[0];
        let preview = fixture.plan(SNAPSHOT_AFTER);
        assert!(preview.ok);
        let token = preview.data.as_ref().unwrap()["executionToken"]
            .as_str()
            .unwrap();
        assert!(fixture.apply(json!({"executionToken": token})).ok);

        let mut response = open_test_request(
            &helper.record,
            ServiceRequestKind::BslMcp {
                operation_id: "waiting-for-analyzer-exit".into(),
                tool_name: "search".into(),
                tool_args: json!({"action": "search_code", "query": "Acceptance"}),
                timeout_seconds: if cancel { 10 } else { 0 },
                timeout_nanos: if cancel { 0 } else { 500_000_000 },
                yields: false,
            },
        );
        let closing = PathBuf::from(format!("{}.closing", cleanup_gate.display()));
        wait_for_file(&closing, Duration::from_secs(2));
        assert_eq!(
            helper.pids(),
            [first_pid],
            "replacement waits for old EOF cleanup"
        );
        assert!(send_test_request(&helper.record, ServiceRequestKind::Ping).ok);
        if cancel {
            assert!(
                send_test_request(
                    &helper.record,
                    ServiceRequestKind::Cancel {
                        operation_id: "waiting-for-analyzer-exit".into(),
                    },
                )
                .ok
            );
        }
        let response = read_test_response(&mut response);
        assert!(
            !response.ok,
            "unfinished retirement cannot claim a replacement"
        );
        let error = response.error.as_deref().unwrap();
        assert!(error.starts_with(if cancel { "cancelled:" } else { "timeout:" }));
        assert_eq!(helper.pids(), [first_pid]);
        assert!(wait_for_process_exit(first_pid, Duration::from_secs(5)));
        assert!(
            helper.pid_file.with_extension("lease").exists(),
            "forced stop does not claim the provider released its lease"
        );
        fs::remove_file(cleanup_gate).unwrap();
        // Recover only the fixture's witness after the owned process has
        // actually exited; no provider-private state is edited by Unica.
        fs::remove_file(helper.pid_file.with_extension("lease")).unwrap();
        assert!(helper.snapshot().contains("AcceptanceChangedSymbol978"));
        assert_eq!(helper.pids().len(), 2);
    }
}

#[test]
fn invalidated_analyzer_preserves_its_owner_after_exit_observation_is_lost() {
    use crate::infrastructure::{
        inject_unix_waitid_error_for_test, reap_runtime_authority_test_child,
    };

    if !unica_process::test_support::unix_runtime_authority_tests_supported() {
        unica_process::test_support::skip_without_host_capability(
            unica_process::test_support::HostCapability::UnixProcessGroups,
            "the exit-observation fault exists only on Unix",
        );
        return;
    }

    struct ExactChildCleanup(u32);
    impl Drop for ExactChildCleanup {
        fn drop(&mut self) {
            reap_runtime_authority_test_child(self.0);
        }
    }

    let fixture = CanonicalInvalidationFixture::new();
    let cleanup_gate = fixture._directory.path().join("hold-lost-owner-cleanup");
    fs::write(&cleanup_gate, b"hold").unwrap();
    let helper = CanonicalInvalidationHelper::start_with_cleanup_gate(
        &fixture.context,
        &fixture.context.workspace_root.join("src"),
        &fixture.executable,
        Some(cleanup_gate.clone()),
    );
    assert!(helper.snapshot().contains("AcceptanceExistingSymbol1306"));
    let first_pid = helper.pids()[0];
    let preview = fixture.plan(SNAPSHOT_AFTER);
    assert!(preview.ok);
    let token = preview.data.as_ref().unwrap()["executionToken"]
        .as_str()
        .unwrap();
    assert!(fixture.apply(json!({"executionToken": token})).ok);

    // The fault is thread-local: call the real component in this thread,
    // after the public TCP apply delivered the actual invalidation.
    inject_unix_waitid_error_for_test();
    let cleanup = ExactChildCleanup(first_pid);
    let call = || {
        helper.runtime.test_handle_bsl_mcp(
            "search",
            json!({"action": "search_code", "query": "Acceptance"}),
            10,
            0,
            &CancellationToken::new(),
        )
    };
    let first = call();
    assert!(!first.ok);
    assert!(first
        .error
        .as_deref()
        .unwrap()
        .contains("injected Unix waitid failure"));
    for _ in 0..2 {
        let retained = helper.runtime.test_analyzer().lock().unwrap();
        assert_eq!(retained.as_ref().unwrap().test_child_pid(), first_pid);
        assert!(helper
            .runtime
            .test_analyzer_invalidated()
            .load(Ordering::Acquire));
        drop(retained);
        let next = call();
        assert!(
            !next.ok,
            "an unknown old owner cannot authorize replacement"
        );
        assert!(next
            .error
            .as_deref()
            .unwrap()
            .contains("authority was lost"));
        assert_eq!(helper.pids(), [first_pid]);
    }
    fs::remove_file(cleanup_gate).unwrap();
    drop(cleanup);
}

/// rule: INV.RUNTIME.SERVICE-MUTATION-INVALIDATION
#[test]
fn failed_helper_invalidation_preserves_the_published_receipt_and_unknown_old_results() {
    for line_ending in ["\n", "\r\n"] {
        assert_failed_helper_invalidation_preserves_the_published_receipt_and_unknown_old_results(
            line_ending,
        );
    }
}

fn assert_failed_helper_invalidation_preserves_the_published_receipt_and_unknown_old_results(
    line_ending: &str,
) {
    use unica_code_intelligence::model::CodeIntelligenceContext;
    use unica_code_intelligence::model::CodeIntelligenceProvider;
    use unica_code_intelligence::model::SearchRequest;
    let fixture = CanonicalInvalidationFixture::new();
    let (module, expected) = fixture.prepare_snapshot_module(line_ending);
    let helper = fixture.helper();
    let source_root = fixture.context.workspace_root.join("src");
    let context = CodeIntelligenceContext::new(
        fixture.context.clone(),
        unica_source_contract::workspace::ResolvedSourceRoot {
            source_set: Some("main".into()),
            path: source_root.clone(),
        },
        std::sync::Arc::new(unica_workspace::discovery::ProjectDiscoveryReader::new(
            &fixture.context.workspace_root,
        )),
    );
    let symbol = || {
        analysis_composition::bsl_provider().search(
            &SearchRequest {
                query: "Acceptance".into(),
                limit: 20,
            },
            &context,
            ProviderDeadline::from_budget(Duration::from_secs(10)),
            &CancellationToken::new(),
        )
    };
    let before = symbol();
    assert_eq!(before.hits.len(), 1, "{before:?}");
    assert_eq!(before.index_freshness.as_deref(), Some("unknown"));
    let first_pid = helper.pids()[0];

    let preview = fixture.plan(SNAPSHOT_AFTER);
    assert!(preview.ok, "{preview:?}");
    let token = preview.data.as_ref().unwrap()["executionToken"]
        .as_str()
        .unwrap();
    let mut wrong_token = helper.record.clone();
    wrong_token.token = "rejected-invalidation-token".into();
    write_record(helper.runtime.test_identity(), wrong_token);
    let connections_before = helper.connections.load(Ordering::Acquire);
    let published = fixture.apply(json!({"executionToken": token}));
    write_record(helper.runtime.test_identity(), helper.record.clone());
    assert!(
        published.ok,
        "delivery failure must not change a committed apply: {published:?}"
    );
    assert_eq!(fs::read(&module).unwrap(), expected);
    assert!(
        helper.notifications().is_empty(),
        "the original helper rejects the token before invalidation"
    );
    assert_eq!(
        helper.connections.load(Ordering::Acquire),
        connections_before + 1,
        "the committed apply makes one real notification attempt even when authentication rejects it"
    );
    let state = fs::read_to_string(fixture.context.cache_root.join("state.json")).unwrap();
    let state: Value = serde_json::from_str(&state).unwrap();
    assert_eq!(state["caches"]["bsl_index"]["status"], "stale");
    assert_eq!(state["caches"]["bsl_diagnostics"]["status"], "stale");
    assert_eq!(
        serde_json::to_value(fixture.apply(json!({"executionToken": token}))).unwrap(),
        serde_json::to_value(published).unwrap()
    );
    assert!(
        helper.notifications().is_empty(),
        "replay must not retry a publication's notification"
    );
    assert_eq!(
        helper.connections.load(Ordering::Acquire),
        connections_before + 1
    );
    let old_symbol = symbol();
    assert_eq!(old_symbol.hits, before.hits);
    assert_eq!(old_symbol.index_freshness.as_deref(), Some("unknown"));
    assert_eq!(helper.pids(), [first_pid]);
    helper.stop_and_verify();
}
