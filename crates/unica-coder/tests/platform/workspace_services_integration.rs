use crate::infrastructure::analysis_composition;
use crate::infrastructure::workspace_service_fixture::new_runtime;

use serde_json::{json, Value};
use std::env;
use std::fs;
use std::io::{self, BufRead, BufReader, ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    mpsc, Arc, Condvar, Mutex,
};
use std::thread;
use std::time::{Duration, Instant};
use unica_adapter_bsl_analyzer::session::BslWorkspaceBackend;
use unica_adapter_bsl_analyzer::session::*;

use unica_analysis_services::protocol::*;
use unica_analysis_services::test_support::WorkspaceServiceOperationExecutor as GenericWorkspaceServiceOperationExecutor;
use unica_analysis_services::test_support::*;
use unica_analysis_services::{
    checked_persistent_provider_working_directory, persistent_provider_working_directory,
};
use unica_analysis_services::{
    WorkspaceServiceBslCall, WorkspaceServiceConfig, WorkspaceServiceIdentity,
    WorkspaceServiceManager, WorkspaceServiceRecord, WorkspaceServices,
};
use unica_daemon::identity::CoreIdentity;
use unica_process::mcp_session::PersistentMcpSession;

use unica_process::ManagedStartupChild;
use unica_source_contract::events::{DomainEvent, DomainEventKind};
use unica_source_contract::workspace::WorkspaceContext;

use unica_work_control::cancellation::{cancelled_error, CancellationToken};
use unica_work_control::elapsed_budget::ElapsedBudgetDeadline;
use unica_work_control::operation_deadline::OperationDeadline;
use unica_work_control::provider_deadline::ProviderDeadline;
use unica_workspace::actor::WorkspaceActor;
use unica_workspace::paths::normalize_path_identity;
use unica_workspace::state_fs::short_private_runtime_dir;
use uuid::Uuid;
type WorkspaceServiceRuntime = WorkspaceServices<BslWorkspaceBackend>;
type WorkspaceServiceRuntimeState =
    unica_analysis_services::test_support::WorkspaceServicesState<BslWorkspaceBackend>;
#[path = "persistent_provider_cwd.rs"]
mod medium_persistent_provider_cwd_native_tests;
#[path = "../workspace_service_support/private_failure_tests.rs"]
mod medium_private_failure_tests;
#[path = "../workspace_service_support/persistent_provider_cwd_command_tests.rs"]
mod persistent_provider_cwd_command_tests;
#[path = "../workspace_service_support/persistent_provider_cwd_selection_tests.rs"]
mod persistent_provider_cwd_selection_tests;
macro_rules! assert_not_impl_production {
    ($type:ty: $trait:path) => {
        const _: fn() = || {
            trait AmbiguousIfImpl<Marker> {
                fn check() {}
            }
            struct ImplementsTrait;
            impl<T: ?Sized> AmbiguousIfImpl<()> for T {}
            impl<T: ?Sized + $trait> AmbiguousIfImpl<ImplementsTrait> for T {}
            let _ = <$type as AmbiguousIfImpl<_>>::check;
        };
    };
}
assert_not_impl_production!(WorkspaceServiceRuntime: std::ops::Deref);
assert_not_impl_production!(WorkspaceServiceRuntime: std::ops::DerefMut);
assert_not_impl_production!(WorkspaceActor<WorkspaceServiceRuntimeState>: std::ops::Deref);
assert_not_impl_production!(WorkspaceActor<WorkspaceServiceRuntimeState>: std::ops::DerefMut);
// Actual provider service implementation moved to unica-analysis-services.
fn workspace_service_command(
    exe: &Path,
    identity: &WorkspaceServiceIdentity,
    config: WorkspaceServiceConfig,
    token: &str,
    build: &CoreIdentity,
) -> Result<Command, String> {
    unica_analysis_services::test_support::workspace_service_command(
        exe,
        identity,
        config,
        token,
        build.as_str(),
        analysis_composition::service_environment().as_ref(),
    )
}
fn wait_for_record_with_connector(
    identity: &WorkspaceServiceIdentity,
    connector: &dyn ServiceConnector,
    pid: u32,
    token: &str,
    timeout: Duration,
    cancel: &CancellationToken,
) -> Result<WorkspaceServiceRecord, String> {
    unica_analysis_services::test_support::wait_for_record_with_connector(
        identity,
        connector,
        CoreIdentity::production().as_str(),
        pid,
        token,
        timeout,
        cancel,
    )
}

// Shared helpers keep their former name for the modules that import them.
#[cfg(test)]
use medium_tests as tests;
#[cfg(test)]
mod medium_tests {
    use super::*;
    pub(super) use crate::infrastructure::workspace_service_fixture::test_record;
    use crate::infrastructure::workspace_service_fixture::{
        now_secs_for_test, open_test_request, read_test_response, send_test_request, wait_for_file,
        wait_for_process_exit, CanonicalInvalidationHelper, SNAPSHOT_BEFORE, SNAPSHOT_MODULE,
    };
    use std::fs;
    use std::path::Path;
    use std::sync::OnceLock;
    use unica_analysis_services::provider_host::ProviderHostKey;
    use unica_source_contract::workspace::WorkspaceContext;

    #[test]
    fn legacy_workspace_service_runtime_is_owned_by_the_workspace_actor_adapter() {
        let context = test_context("legacy-runtime-actor-owner");
        let source_root = context.workspace_root.join("src");
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        let record = test_record(&identity, 1, CoreIdentity::production().as_str());
        let runtime = new_runtime(identity, &record);

        runtime.test_assert_actor_owned();
        assert_eq!(
            runtime.test_workspace_identity().provider_profile(),
            WORKSPACE_SERVICE_PROVIDER_PROFILE
        );
        cleanup(&context);
    }

    #[test]
    fn provider_host_identity_is_rootless_while_route_state_remains_root_bound() {
        let context = test_context("provider-host-root-split");
        let first_root = context.workspace_root.join("src-a");
        let second_root = context.workspace_root.join("src-b");
        fs::create_dir_all(&first_root).unwrap();
        fs::create_dir_all(&second_root).unwrap();
        let capabilities = std::collections::BTreeSet::from([
            "diagnostics".to_string(),
            "navigation".to_string(),
            "search".to_string(),
        ]);
        let first_host = ProviderHostKey::new(
            "bsl-analyzer",
            crate::infrastructure::platform::current_target_id().unwrap(),
            capabilities.clone(),
        )
        .unwrap();
        let second_host = ProviderHostKey::new(
            "bsl-analyzer",
            crate::infrastructure::platform::current_target_id().unwrap(),
            capabilities,
        )
        .unwrap();

        assert_eq!(first_host, second_host, "host readiness is rootless");
        assert_ne!(
            WorkspaceServiceIdentity::new(&context, &first_root).unwrap(),
            WorkspaceServiceIdentity::new(&context, &second_root).unwrap(),
            "root-bound compatibility sessions must not be reused across roots"
        );
        assert_ne!(
            bsl_analyzer_cache_dir(&context, &first_root).unwrap(),
            bsl_analyzer_cache_dir(&context, &second_root).unwrap(),
            "code intelligence and diagnostics caches remain root-bound"
        );
        cleanup(&context);
    }

    #[test]
    fn persistent_transport_deadline_handles_full_configured_range_without_panicking() {
        let context = test_context("persistent-transport-full-range-deadline");
        let fixture = compile_exit_after_call_fixture(&context);
        let pid_file = context.cache_root.join("transport-full-range-pids.txt");
        let completion_file = context
            .cache_root
            .join("transport-full-range-completed.txt");
        let mut command = Command::new(&fixture);
        command.args([&pid_file, &completion_file]);
        let cancellation = CancellationToken::new();
        let mut session = PersistentMcpSession::start_with_command(command, &cancellation).unwrap();

        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            session.call(
                "status",
                json!({}),
                Duration::from_secs(i64::MAX as u64),
                &cancellation,
            )
        }));

        drop(session);
        cleanup(&context);
        let output = outcome.expect("full configured timeout must not overflow Instant");
        assert_eq!(output.unwrap().result_text, "ok");
    }

    #[test]
    fn bsl_protocol_roundtrips_full_positive_i64_seconds() {
        let wire = json!({
            "type": "bsl-mcp",
            "operation_id": "max-timeout",
            "tool_name": "search",
            "tool_args": {"query": "needle"},
            "timeout_seconds": i64::MAX,
            "timeout_nanos": 0
        });

        let request: ServiceRequestKind = serde_json::from_value(wire.clone()).unwrap();

        assert_eq!(serde_json::to_value(request).unwrap(), wire);
    }

    #[test]
    fn service_timeout_parts_reject_seconds_outside_positive_i64_config_domain() {
        let error = duration_from_timeout_parts(u64::MAX, 1)
            .expect_err("wire timeout must stay inside the accepted config domain");

        assert!(error.contains("exceeds"), "{error}");
        assert!(error.contains(&i64::MAX.to_string()), "{error}");
    }

    /// rule: INV.APP.PERSISTENT-PROVIDER-CWD
    #[test]
    fn bsl_analyzer_child_does_not_run_from_inside_the_workspace() {
        let context = test_context("bsl-analyzer-external-cwd");
        let source_root = context.workspace_root.join("src");
        let command =
            bsl_analyzer_command(Path::new("bsl-analyzer"), &context, &source_root).unwrap();
        let directory = command.get_current_dir().expect("explicit analyzer cwd");
        assert!(directory.is_dir());
        for root in [&context.workspace_root, &source_root] {
            assert!(!unica_workspace::paths::path_starts_with_host_root(
                &normalize_path_identity(directory).unwrap(),
                &normalize_path_identity(root).unwrap(),
            ));
        }
        cleanup(&context);
    }

    #[cfg_attr(
        not(unix),
        ignore = "host capability: a private runtime directory and Unix socket exist only on Unix"
    )]
    #[test]
    fn bsl_analyzer_runtime_directory_is_passed_to_the_child_and_fits_socket_budget() {
        use std::ffi::OsStr;

        let expected = short_private_runtime_dir().unwrap();
        let mut command = Command::new("bsl-analyzer");
        configure_bsl_analyzer_runtime_dir(&mut command).unwrap();
        let actual = command
            .get_envs()
            .find(|(key, _)| *key == OsStr::new("XDG_RUNTIME_DIR"))
            .and_then(|(_, value)| value.map(PathBuf::from));

        assert_eq!(actual, expected);
        let runtime_dir = actual.expect("a Unix host passes a private runtime directory");

        let socket = runtime_dir
            .join("bsl-mcp")
            .join("0123456789abcdef0123456789abcdef.sock");

        assert_eq!(runtime_dir.parent(), Some(Path::new("/tmp")));
        assert!(socket.as_os_str().as_encoded_bytes().len() < 104);
    }

    #[test]
    fn bsl_analyzer_child_uses_source_specific_cache_outside_the_source_tree() {
        let context = test_context("bsl-analyzer-cache");
        let first_source = context.workspace_root.join("src/main");
        let second_source = context.workspace_root.join("src/extension");
        fs::create_dir_all(&first_source).unwrap();
        fs::create_dir_all(&second_source).unwrap();

        let first_cache = bsl_analyzer_cache_dir(&context, &first_source).unwrap();
        let second_cache = bsl_analyzer_cache_dir(&context, &second_source).unwrap();
        let cache_root = normalize_path_identity(&context.cache_root).unwrap();
        let first_source = normalize_path_identity(&first_source).unwrap();

        assert!(first_cache.starts_with(cache_root));
        assert!(!first_cache.starts_with(&first_source));
        assert_ne!(first_cache, second_cache);
    }

    /// rule: INV.CACHE.STATE-OUTSIDE-SOURCE
    #[test]
    fn bsl_analyzer_cache_stays_outside_a_workspace_wide_source_root() {
        let mut context = test_context("bsl-analyzer-workspace-cache");
        context.cache_root = context.workspace_root.join(".build/unica");

        let cache = bsl_analyzer_cache_dir(&context, &context.workspace_root).unwrap();
        let workspace_root = normalize_path_identity(&context.workspace_root).unwrap();
        let expected_key = provider_state_key(&workspace_root, &workspace_root).unwrap();

        assert!(!cache.starts_with(&workspace_root));
        assert_eq!(
            cache.file_name(),
            Some(std::ffi::OsStr::new(&expected_key)),
            "the fallback must preserve the normalized source identity"
        );
        assert_eq!(
            cache.parent().unwrap().file_name(),
            Some(std::ffi::OsStr::new("owned-stdio"))
        );

        cleanup(&context);
    }

    #[test]
    fn bsl_analyzer_command_passes_external_provider_cache() {
        use std::ffi::OsString;

        let context = test_context("bsl-provider-command");
        let source_root = context.workspace_root.join("src");
        let expected_cache = bsl_analyzer_cache_dir(&context, &source_root).unwrap();
        let expected_key = provider_state_key(
            &normalize_path_identity(&context.workspace_root).unwrap(),
            &normalize_path_identity(&source_root).unwrap(),
        )
        .unwrap();
        assert_eq!(
            expected_cache,
            normalize_path_identity(
                &context
                    .cache_root
                    .join("providers/bsl-analyzer/owned-stdio")
                    .join(expected_key)
            )
            .unwrap()
        );
        let command =
            bsl_analyzer_command(Path::new("bsl-analyzer"), &context, &source_root).unwrap();
        let args = command.get_args().map(OsString::from).collect::<Vec<_>>();

        assert_eq!(
            args,
            vec![
                "mcp".into(),
                "serve".into(),
                "--profile".into(),
                "workspace".into(),
                "--source-dir".into(),
                normalize_path_identity(&source_root)
                    .unwrap()
                    .into_os_string(),
                "--cache-dir".into(),
                expected_cache.into_os_string(),
                "--mode".into(),
                "stdio".into(),
            ]
        );
        assert_eq!(
            command.get_envs().find(|(key, _)| *key == "BSL_MCP_BROKER"),
            Some((
                std::ffi::OsStr::new("BSL_MCP_BROKER"),
                Some(std::ffi::OsStr::new("0"))
            )),
            "retiring the persistent session must retire its backend, not only a broker proxy"
        );

        cleanup(&context);
    }

    #[derive(Default)]
    struct BlockingWorkspaceState {
        started: Vec<String>,
        cancelled: Vec<String>,
        release_cancelled: bool,
    }

    #[derive(Default)]
    struct BlockingWorkspaceExecutor {
        state: Mutex<BlockingWorkspaceState>,
        wake: std::sync::Condvar,
    }

    impl BlockingWorkspaceExecutor {
        fn wait_started(&self, expected: usize) {
            let deadline = Instant::now() + Duration::from_secs(2);
            let mut state = self.state.lock().unwrap();
            while state.started.len() < expected {
                let remaining = deadline.saturating_duration_since(Instant::now());
                assert!(
                    !remaining.is_zero(),
                    "work did not start before test deadline"
                );
                let (next, timeout) = self.wake.wait_timeout(state, remaining).unwrap();
                state = next;
                assert!(!timeout.timed_out() || state.started.len() >= expected);
            }
        }

        fn wait_cancelled(&self, expected: usize) {
            let deadline = Instant::now() + Duration::from_secs(2);
            let mut state = self.state.lock().unwrap();
            while state.cancelled.len() < expected {
                let remaining = deadline.saturating_duration_since(Instant::now());
                assert!(
                    !remaining.is_zero(),
                    "operation was not cancelled before test deadline"
                );
                let (next, timeout) = self.wake.wait_timeout(state, remaining).unwrap();
                state = next;
                assert!(!timeout.timed_out() || state.cancelled.len() >= expected);
            }
        }

        fn release_cancelled(&self) {
            self.state.lock().unwrap().release_cancelled = true;
            self.wake.notify_all();
        }
    }

    impl GenericWorkspaceServiceOperationExecutor<BslWorkspaceBackend> for BlockingWorkspaceExecutor {
        fn execute(
            &self,
            _runtime: &WorkspaceServiceRuntime,
            kind: ServiceRequestKind,
            cancellation: &CancellationToken,
        ) -> ServiceResponse {
            let operation_id = kind.operation_id().unwrap_or("missing").to_string();
            if operation_id.starts_with("panic-worker") {
                panic!("intentional workspace worker panic");
            }
            {
                let mut state = self.state.lock().unwrap();
                state.started.push(operation_id.clone());
                self.wake.notify_all();
            }
            if operation_id.starts_with("success") {
                return ServiceResponse {
                    ok: true,
                    status: Some("ready".to_string()),
                    ..ServiceResponse::default()
                };
            }
            let deadline = Instant::now() + Duration::from_secs(2);
            while !cancellation.is_cancelled() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(10));
            }
            if cancellation.is_cancelled() {
                let mut state = self.state.lock().unwrap();
                state.cancelled.push(operation_id.clone());
                self.wake.notify_all();
                if operation_id.starts_with("held-after-cancel") {
                    while !state.release_cancelled {
                        state = self.wake.wait(state).unwrap();
                    }
                }
                unica_analysis_services::test_support::error_response(cancelled_error(
                    "workspace operation stopped",
                ))
            } else {
                unica_analysis_services::test_support::error_response(
                    "test operation was not cancelled",
                )
            }
        }
    }

    #[derive(Default)]
    struct AnalyzerLaneExecutor {
        holder_started: Mutex<bool>,
        wake: std::sync::Condvar,
    }

    impl AnalyzerLaneExecutor {
        fn wait_holder(&self) {
            let deadline = Instant::now() + Duration::from_secs(2);
            let mut started = self.holder_started.lock().unwrap();
            while !*started {
                let remaining = deadline.saturating_duration_since(Instant::now());
                assert!(!remaining.is_zero(), "analyzer lane holder did not start");
                (started, _) = self.wake.wait_timeout(started, remaining).unwrap();
            }
        }
    }

    impl GenericWorkspaceServiceOperationExecutor<BslWorkspaceBackend> for AnalyzerLaneExecutor {
        fn execute(
            &self,
            runtime: &WorkspaceServiceRuntime,
            kind: ServiceRequestKind,
            cancellation: &CancellationToken,
        ) -> ServiceResponse {
            let operation_id = kind.operation_id().unwrap().to_string();
            let _lane = match runtime.test_acquire_analyzer_lane(cancellation) {
                Ok(lane) => lane,
                Err(error) => return unica_analysis_services::test_support::error_response(error),
            };
            if operation_id.starts_with("analyzer-holder") {
                *self.holder_started.lock().unwrap() = true;
                self.wake.notify_all();
                while !cancellation.is_cancelled() {
                    thread::sleep(Duration::from_millis(10));
                }
                return unica_analysis_services::test_support::error_response(cancelled_error(
                    "analyzer holder stopped",
                ));
            }
            ServiceResponse {
                ok: true,
                status: Some("analyzer-lane-acquired".to_string()),
                ..ServiceResponse::default()
            }
        }
    }

    type WorkspaceControlTestServer = (
        WorkspaceContext,
        WorkspaceServiceRecord,
        Arc<WorkspaceServiceRuntime>,
        Arc<BlockingWorkspaceExecutor>,
        thread::JoinHandle<Result<(), String>>,
    );

    fn workspace_control_test_server(name: &str) -> WorkspaceControlTestServer {
        workspace_control_test_server_with_options(name, SERVICE_REQUEST_HEADER_TIMEOUT, None)
    }

    fn workspace_control_test_server_with_header_timeout(
        name: &str,
        request_header_timeout: Duration,
        handler_started_hook: Arc<dyn Fn() + Send + Sync>,
    ) -> WorkspaceControlTestServer {
        workspace_control_test_server_with_options(
            name,
            request_header_timeout,
            Some(handler_started_hook),
        )
    }

    fn workspace_control_test_server_with_options(
        name: &str,
        request_header_timeout: Duration,
        handler_started_hook: Option<Arc<dyn Fn() + Send + Sync>>,
    ) -> WorkspaceControlTestServer {
        let context = test_context(name);
        let identity =
            WorkspaceServiceIdentity::new(&context, &context.workspace_root.join("src")).unwrap();
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let record = test_record(&identity, port, CoreIdentity::production().as_str());
        write_record(&identity, record.clone());
        let runtime = new_runtime(identity, &record);
        *runtime.test_handler_started_hook().lock().unwrap() = handler_started_hook;
        let runtime = Arc::new(runtime);
        let executor = Arc::new(BlockingWorkspaceExecutor::default());
        let server_runtime = Arc::clone(&runtime);
        let server_executor = Arc::clone(&executor);
        let server = thread::spawn(move || {
            serve_workspace_service(
                listener,
                server_runtime,
                server_executor,
                Duration::from_secs(30),
                request_header_timeout,
                SERVICE_SHUTDOWN_GRACE,
            )
        });
        (context, record, runtime, executor, server)
    }

    fn workspace_test_server_with_executor(
        name: &str,
        executor: Arc<dyn GenericWorkspaceServiceOperationExecutor<BslWorkspaceBackend>>,
        shutdown_grace: Duration,
    ) -> (
        WorkspaceContext,
        WorkspaceServiceRecord,
        Arc<WorkspaceServiceRuntime>,
        thread::JoinHandle<Result<(), String>>,
    ) {
        let context = test_context(name);
        let identity =
            WorkspaceServiceIdentity::new(&context, &context.workspace_root.join("src")).unwrap();
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let record = test_record(
            &identity,
            listener.local_addr().unwrap().port(),
            CoreIdentity::production().as_str(),
        );
        write_record(&identity, record.clone());
        let runtime = Arc::new(new_runtime(identity, &record));
        let server_runtime = Arc::clone(&runtime);
        let server = thread::spawn(move || {
            serve_workspace_service(
                listener,
                server_runtime,
                executor,
                Duration::from_secs(30),
                SERVICE_REQUEST_HEADER_TIMEOUT,
                shutdown_grace,
            )
        });
        (context, record, runtime, server)
    }

    /// Answers after `delay`, unless the service stops it first.
    struct SlowCancellableExecutor {
        delay: Duration,
    }

    impl GenericWorkspaceServiceOperationExecutor<BslWorkspaceBackend> for SlowCancellableExecutor {
        fn execute(
            &self,
            _runtime: &WorkspaceServiceRuntime,
            _kind: ServiceRequestKind,
            cancellation: &CancellationToken,
        ) -> ServiceResponse {
            let until = Instant::now() + self.delay;
            while Instant::now() < until {
                if cancellation.is_cancelled() {
                    return unica_analysis_services::test_support::error_response(cancelled_error(
                        "slow operation stopped",
                    ));
                }
                thread::sleep(Duration::from_millis(10));
            }
            ServiceResponse {
                ok: true,
                status: Some("slow-done".to_string()),
                ..ServiceResponse::default()
            }
        }
    }

    /// rule: INV.RUNTIME.SERVICE-LIFETIME
    #[test]
    fn workspace_service_never_stops_under_a_request_longer_than_its_idle_time() {
        let context = test_context("idle-under-request");
        let identity =
            WorkspaceServiceIdentity::new(&context, &context.workspace_root.join("src")).unwrap();
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let record = test_record(
            &identity,
            listener.local_addr().unwrap().port(),
            CoreIdentity::production().as_str(),
        );
        write_record(&identity, record.clone());
        let runtime = Arc::new(new_runtime(identity, &record));
        let server_runtime = Arc::clone(&runtime);
        let idle_timeout = Duration::from_millis(200);
        let server = thread::spawn(move || {
            serve_workspace_service(
                listener,
                server_runtime,
                Arc::new(SlowCancellableExecutor {
                    delay: Duration::from_millis(1_200),
                }),
                idle_timeout,
                SERVICE_REQUEST_HEADER_TIMEOUT,
                SERVICE_SHUTDOWN_GRACE,
            )
        });
        let response = send_test_request(
            &record,
            ServiceRequestKind::BslMcp {
                operation_id: "slow-1".to_string(),
                tool_name: "diagnostics".to_string(),
                tool_args: json!({}),
                timeout_seconds: 5,
                timeout_nanos: 0,
                yields: false,
            },
        );
        assert!(
            response.ok,
            "a request six times longer than the idle time completes: {response:?}"
        );
        assert_eq!(response.status.as_deref(), Some("slow-done"));
        // Once the request is done the service is idle and leaves on its own.
        server
            .join()
            .expect("service thread did not panic")
            .expect("service exits cleanly after idling");
        assert!(runtime.test_shutting_down().load(Ordering::Acquire));
        let _ = fs::remove_dir_all(context.cache_root.parent().unwrap());
    }

    #[test]
    fn workspace_service_control_path_ping_cancel_and_recover() {
        let (context, record, runtime, executor, server) =
            workspace_control_test_server("control-ping-cancel");
        let mut work = open_test_request(
            &record,
            ServiceRequestKind::BslMcp {
                operation_id: "blocked-1".to_string(),
                tool_name: "diagnostics".to_string(),
                tool_args: json!({}),
                timeout_seconds: 5,
                timeout_nanos: 0,
                yields: false,
            },
        );
        executor.wait_started(1);

        let ping_started = Instant::now();
        let ping = send_test_request(&record, ServiceRequestKind::Ping);
        assert!(ping.ok);
        assert!(ping_started.elapsed() < Duration::from_millis(500));

        let cancel = send_test_request(
            &record,
            ServiceRequestKind::Cancel {
                operation_id: "blocked-1".to_string(),
            },
        );
        assert!(cancel.ok);
        let cancelled = read_test_response(&mut work);
        assert!(!cancelled.ok);
        assert!(cancelled.error.unwrap().starts_with("cancelled:"));
        assert!(runtime.test_operation_ids().is_empty());

        let recovered = send_test_request(
            &record,
            ServiceRequestKind::BslMcp {
                operation_id: "success-2".to_string(),
                tool_name: "diagnostics".to_string(),
                tool_args: json!({}),
                timeout_seconds: 5,
                timeout_nanos: 0,
                yields: false,
            },
        );
        assert!(recovered.ok);
        assert!(runtime.test_operation_ids().is_empty());

        assert!(send_test_request(&record, ServiceRequestKind::Shutdown).ok);
        server.join().unwrap().unwrap();
        let identity =
            WorkspaceServiceIdentity::new(&context, &context.workspace_root.join("src")).unwrap();
        assert!(!identity.test_record_path().exists());
        cleanup(&context);
    }

    /// rule: INV.RUNTIME.SERVICE-REQUEST-FRAMING
    #[test]
    fn workspace_service_request_lines_are_bounded_and_resynchronize() {
        let mut bytes = vec![b'x'; SERVICE_REQUEST_LINE_LIMIT + 1];
        bytes.push(b'\n');
        bytes.extend_from_slice(b"{}\n");
        let mut reader = BufReader::new(std::io::Cursor::new(bytes));
        assert!(matches!(
            read_bounded_service_line(&mut reader).unwrap(),
            Some(BoundedServiceLine::TooLarge)
        ));
        assert!(
            matches!(read_bounded_service_line(&mut reader).unwrap(), Some(BoundedServiceLine::Line(line)) if line == b"{}")
        );
    }

    /// rule: INV.RUNTIME.SERVICE-REQUEST-FRAMING
    #[test]
    fn workspace_service_header_deadline_is_aggregate_across_drip_bytes() {
        struct DripReader {
            bytes: Vec<u8>,
            offset: usize,
            clock: ManualClock,
        }
        impl Read for DripReader {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                if self.offset == self.bytes.len() {
                    return Ok(0);
                }
                self.clock.advance(Duration::from_millis(1));
                buf[0] = self.bytes[self.offset];
                self.offset += 1;
                Ok(1)
            }
        }
        let clock = ManualClock::default();
        let mut reader = BufReader::with_capacity(
            1,
            DripReader {
                bytes: b"{}\n".to_vec(),
                offset: 0,
                clock: clock.clone(),
            },
        );
        let deadline = Deadline::new(&clock, Duration::from_millis(2));
        let error =
            read_bounded_service_line_with_deadline(&mut reader, &deadline, &clock, |_| Ok(()))
                .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::TimedOut);
    }

    /// rule: INV.RUNTIME.SERVICE-REQUEST-FRAMING
    #[test]
    fn workspace_service_header_timeout_is_connection_local() {
        let (handler_started_tx, handler_started_rx) = mpsc::sync_channel(0);
        let handler_started_tx = Mutex::new(Some(handler_started_tx));
        let handler_started_hook = Arc::new(move || {
            if let Some(sender) = handler_started_tx.lock().unwrap().take() {
                let _ = sender.send(());
            }
        });
        let (context, record, _runtime, _executor, server) =
            workspace_control_test_server_with_header_timeout(
                "header-timeout-recovery",
                Duration::from_millis(50),
                handler_started_hook,
            );
        let mut stalled = TcpStream::connect(("127.0.0.1", record.port)).unwrap();
        stalled.write_all(b"{").unwrap();
        stalled.flush().unwrap();

        handler_started_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("stalled connection handler did not start");
        stalled
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let closed = stalled.read(&mut [0_u8]);
        assert!(
            matches!(closed, Ok(0))
                || matches!(closed, Err(ref error) if matches!(error.kind(), ErrorKind::ConnectionReset | ErrorKind::ConnectionAborted)),
            "timed-out connection stayed open: {closed:?}"
        );

        assert!(send_test_request(&record, ServiceRequestKind::Ping).ok);
        assert!(send_test_request(&record, ServiceRequestKind::Shutdown).ok);
        drop(stalled);
        server.join().unwrap().unwrap();
        cleanup(&context);
    }

    fn prepared_begin(id: &str) -> ServiceRequestKind {
        ServiceRequestKind::BslMcpBegin {
            operation_id: id.into(),
            call: PreparedBslCall {
                tool_name: "symbol_info".into(),
                tool_args: json!({}),
                yields: false,
            },
            deadline: BslOperationDeadline::Absent {},
        }
    }

    fn read_prepared_reply(reader: &mut BufReader<TcpStream>) -> PreparedBslReply {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    }

    fn write_prepared_phase(
        reader: &mut BufReader<TcpStream>,
        record: &WorkspaceServiceRecord,
        kind: ServiceRequestKind,
    ) {
        let frame = ServiceRequest {
            token: record.token.clone(),
            kind,
        };
        writeln!(
            reader.get_mut(),
            "{}",
            serde_json::to_string(&frame).unwrap()
        )
        .unwrap();
        reader.get_mut().flush().unwrap();
    }

    fn wait_operation_retired(runtime: &WorkspaceServiceRuntime, id: &str) {
        let watchdog = Instant::now() + Duration::from_secs(3);
        while runtime
            .test_operation_ids()
            .iter()
            .any(|active| active == id)
        {
            assert!(
                Instant::now() < watchdog,
                "operation owner was not collected: {id}"
            );
            thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn prepared_bsl_symbol_absence_reaches_the_actual_persistent_mcp_session() {
        use unica_adapter_bsl_analyzer::provider::WorkspaceBslToolClient;
        use unica_adapter_bsl_analyzer::symbols::BslAnalyzerSymbols;
        use unica_code_intelligence::model::CodeIntelligenceContext;
        use unica_code_intelligence::symbols::{
            SymbolPosition, SymbolProvider, SymbolQuery, SymbolRequestContext,
        };
        use unica_source_contract::workspace::ResolvedSourceRoot;

        for finite in [false, true] {
            let context = test_context(&format!("prepared-symbol-full-route-{finite}"));
            let source_root = context.workspace_root.join("src");
            fs::create_dir_all(source_root.join("CommonModules/SearchProbe/Ext")).unwrap();
            fs::write(source_root.join(SNAPSHOT_MODULE), SNAPSHOT_BEFORE).unwrap();
            fs::create_dir_all(&context.cache_root).unwrap();
            let source = context.cache_root.join("symbol-session.rs");
            let executable = unica_process::test_support::fixture_executable_path(
                &context.cache_root,
                "symbol-session",
            );
            let answer = json!({"status":"ok", "symbol":"DeadlineProbe", "kind":"procedure", "definitions":[],
            "freshness":{"stale":false,"completeness":{"status":"complete"}}});
            let reply = json!({"jsonrpc":"2.0", "id":2, "result":{"content":[{"type":"text","text":answer.to_string()}]}}).to_string();
            let program = r#"use std::{env, fs::OpenOptions, io::{self, BufRead, Write}};
fn main() {
    let pid_file = env::args().nth(2).unwrap();
    writeln!(OpenOptions::new().create(true).append(true).open(pid_file).unwrap(), "{}", std::process::id()).unwrap();
    for (index, line) in io::stdin().lock().lines().enumerate() {
        let line = line.unwrap();
        match index {
            0 => { std::thread::sleep(std::time::Duration::from_millis(150)); println!("{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{{}}}}"); },
            2 => { assert!(line.contains("symbol_info"), "{}", line); println!("{}", __REPLY__); }
            _ => {}
        }
        io::stdout().flush().unwrap();
    }
}"#.replace("__REPLY__", &format!("{reply:?}"));
            fs::write(&source, program).unwrap();
            assert!(Command::new("rustc")
                .arg(&source)
                .arg("-o")
                .arg(&executable)
                .status()
                .unwrap()
                .success());
            let helper = CanonicalInvalidationHelper::start(&context, &source_root, &executable);
            let dispatch_observed_at = Arc::new(Mutex::new(None));
            if finite {
                let dispatch_observed_at = dispatch_observed_at.clone();
                helper
                    .runtime
                    .test_set_prepared_dispatch_hook(Some(Arc::new(move || {
                        *dispatch_observed_at.lock().unwrap() = Some(Instant::now());
                        // Budget consumption before System execution, not a readiness wait.
                        thread::sleep(Duration::from_millis(100));
                    })));
            }
            let (session_observed, session_received) = mpsc::channel();
            helper
                .runtime
                .test_set_bsl_session_deadline_hook(Some(Arc::new(move |remaining, elapsed| {
                    session_observed
                        .send((remaining, elapsed, Instant::now()))
                        .unwrap();
                })));
            let (observed, received) = mpsc::channel();
            helper
                .runtime
                .test_set_authenticated_request_hook(Some(Arc::new(move |_, request| {
                    if let ServiceRequestKind::BslMcpBegin { deadline, call, .. } = request {
                        observed
                            .send((deadline.clone(), call.tool_name.clone()))
                            .unwrap();
                    }
                })));
            let client = WorkspaceBslToolClient {
                environment: analysis_composition::service_environment,
            };
            let symbols = BslAnalyzerSymbols::new(
                &client,
                CodeIntelligenceContext::new(
                    context.clone(),
                    ResolvedSourceRoot {
                        source_set: Some("main".into()),
                        path: source_root,
                    },
                    Arc::new(unica_workspace::discovery::ProjectDiscoveryReader::new(
                        &context.workspace_root,
                    )),
                ),
            );
            let cancellation = CancellationToken::new();
            let symbol = symbols
                .symbol(
                    &SymbolQuery::At {
                        path: SNAPSHOT_MODULE.into(),
                        position: SymbolPosition { line: 0, column: 1 },
                    },
                    &SymbolRequestContext {
                        deadline: if finite {
                            ProviderDeadline::from_budget(Duration::from_secs(5))
                        } else {
                            ProviderDeadline::no_deadline()
                        },
                        cancellation: &cancellation,
                    },
                )
                .unwrap()
                .unwrap();
            assert_eq!(symbol.name, "DeadlineProbe");
            let (remaining, elapsed, session_observed_at) = session_received
                .recv_timeout(Duration::from_secs(3))
                .unwrap();
            assert!(!elapsed);
            if finite {
                let remaining = remaining.expect("finite must not become absence");
                let dispatch_observed_at = dispatch_observed_at.lock().unwrap().unwrap();
                let consumed = session_observed_at.saturating_duration_since(dispatch_observed_at);
                let upper = Duration::from_secs(5)
                    .saturating_sub(consumed)
                    .saturating_add(Duration::from_millis(5));
                assert!(remaining > Duration::ZERO && remaining <= upper,
                    "same receiver anchor must account for all actual dispatch/initialization delay: remaining={remaining:?}, consumed={consumed:?}");
            } else {
                assert_eq!(remaining, None, "the actual session deadline port must retain absence, not a finite replacement");
            }
            let (deadline, tool) = received.recv_timeout(Duration::from_secs(3)).unwrap();
            assert_eq!(matches!(deadline, BslOperationDeadline::Absent {}), !finite);
            assert_eq!(tool, "symbol_info");
            assert_eq!(
                helper.pids().len(),
                1,
                "one real MCP child handled the symbol request"
            );
            helper.stop_and_verify();
            cleanup(&context);
        }
    }

    #[test]
    fn prepared_bsl_cancel_and_expire_before_start_preserve_the_registered_decision() {
        // Two admissible orders before Start, each fixed by an event:
        // the handler finishes on the recorded stop without waiting for Start,
        // or a Start already consumed by the held handler meets the stop.
        // A Start written after the handler has finished is not a scenario:
        // the closed reservation may reset it, which proves nothing.
        for start_delivered in [false, true] {
            for expire in [false, true] {
                let mut server =
                    WorkspaceAdmissionTestServer::start("prepared-control-before-start");
                let gate = Arc::new((Mutex::new(false), Condvar::new()));
                let (entered, start_received) = mpsc::channel();
                if start_delivered {
                    server.gates.push(gate.clone());
                    server
                        .runtime
                        .test_set_prepared_start_received_hook(Some(Arc::new({
                            let gate = gate.clone();
                            move || {
                                entered.send(()).unwrap();
                                wait_admission_test_gate(&gate);
                            }
                        })));
                }
                let id = "held-before-start";
                let mut work = open_test_request(&server.record, prepared_begin(id));
                assert!(matches!(read_prepared_reply(&mut work),
                    PreparedBslReply::Admitted { operation_id } if operation_id == id));
                let token = server.token(id);
                assert!(!token.is_cancelled());
                if start_delivered {
                    write_prepared_phase(
                        &mut work,
                        &server.record,
                        ServiceRequestKind::BslMcpStart {
                            operation_id: id.into(),
                            deadline: BslOperationDeadline::Absent {},
                        },
                    );
                    start_received
                        .recv_timeout(Duration::from_secs(3))
                        .expect("the handler consumed Start");
                }
                let control = if expire {
                    ServiceRequestKind::Expire {
                        operation_id: id.into(),
                    }
                } else {
                    ServiceRequestKind::Cancel {
                        operation_id: id.into(),
                    }
                };
                assert!(send_test_request(&server.record, control).ok);
                assert!(token.is_cancelled());
                if start_delivered {
                    *gate.0.lock().unwrap() = true;
                    gate.1.notify_all();
                }
                let PreparedBslReply::Finished { response } = read_prepared_reply(&mut work) else {
                    panic!("a stopped reservation was admitted again");
                };
                assert!(!response.ok);
                let error = response.error.unwrap_or_default();
                assert!(
                    error.starts_with(if expire { "timeout:" } else { "cancelled:" }),
                    "start_delivered={start_delivered}, expire={expire}: {error}"
                );
                assert!(
                    server.executor.started.lock().unwrap().is_empty(),
                    "Start dispatched stopped work"
                );
                wait_operation_retired(&server.runtime, id);
                assert!(
                    send_test_request(
                        &server.record,
                        ServiceRequestKind::BslMcp {
                            operation_id: "success-neighbor".into(),
                            tool_name: "diagnostics".to_string(),
                            tool_args: json!({}),
                            timeout_seconds: 5,
                            timeout_nanos: 0,
                            yields: false
                        }
                    )
                    .ok
                );
            }
        }
    }

    #[test]
    fn prepared_bsl_cancellation_before_registration_cannot_create_late_work() {
        thread::scope(|scope| {
            let mut server = WorkspaceAdmissionTestServer::start("prepared-before-registration");
            let gate = Arc::new((Mutex::new(false), Condvar::new()));
            server.gates.push(gate.clone());
            let (entered, observed) = mpsc::channel();
            server
                .runtime
                .test_set_authenticated_request_hook(Some(Arc::new({
                    let gate = gate.clone();
                    move |_, kind| {
                        if matches!(kind, ServiceRequestKind::BslMcpBegin { .. }) {
                            entered.send(()).unwrap();
                            wait_admission_test_gate(&gate);
                        }
                    }
                })));
            let (finished, collected) = mpsc::channel();
            server
                .runtime
                .test_set_operation_retired_hook(Some(Arc::new(move |id| {
                    if id == "late-never-started" {
                        let _ = finished.send(());
                    }
                })));
            let cancellation = CancellationToken::new();
            let worker_cancellation = cancellation.clone();
            let record = server.record.clone();
            let sender = scope.spawn(move || {
                SYSTEM_SERVICE_CONNECTOR.send_operation_before(
                    &record,
                    ServiceRequest {
                        token: record.token.clone(),
                        kind: prepared_begin("late-never-started"),
                    },
                    &worker_cancellation,
                    ProviderDeadline::no_deadline(),
                )
            });
            observed.recv_timeout(Duration::from_secs(3)).unwrap();
            assert!(server.runtime.test_operation_ids().is_empty());
            cancellation.cancel();
            *gate.0.lock().unwrap() = true;
            gate.1.notify_all();
            let error = sender.join().unwrap().unwrap_err();
            assert!(error.starts_with("cancelled:"));
            collected
                .recv_timeout(Duration::from_secs(3))
                .expect("the late Begin reservation was registered and retired");
            wait_operation_retired(&server.runtime, "late-never-started");
            assert!(server.executor.started.lock().unwrap().is_empty());
            assert!(
                send_test_request(
                    &server.record,
                    ServiceRequestKind::BslMcp {
                        operation_id: "success-independent".into(),
                        tool_name: "diagnostics".to_string(),
                        tool_args: json!({}),
                        timeout_seconds: 5,
                        timeout_nanos: 0,
                        yields: false
                    }
                )
                .ok
            );
        });
    }

    #[test]
    fn prepared_bsl_accepted_eof_retains_owner_until_separate_expiry() {
        let server = WorkspaceAdmissionTestServer::start("prepared-accepted-eof");
        let id = "held-after-start";
        let mut work = open_test_request(&server.record, prepared_begin(id));
        assert!(matches!(read_prepared_reply(&mut work),
            PreparedBslReply::Admitted { operation_id } if operation_id == id));
        write_prepared_phase(
            &mut work,
            &server.record,
            ServiceRequestKind::BslMcpStart {
                operation_id: id.into(),
                deadline: BslOperationDeadline::Absent {},
            },
        );
        server.executor.wait_started(1);
        let token = server.token(id);
        let (detached_tx, detached_rx) = mpsc::channel();
        server
            .runtime
            .test_set_caller_detached_hook(Some(Arc::new(move |token| {
                let _ = detached_tx.send(token.is_cancelled());
            })));
        drop(work);
        assert!(
            !detached_rx.recv_timeout(Duration::from_secs(3)).unwrap(),
            "the real EOF monitor cancelled accepted provider work"
        );
        assert!(
            !token.is_cancelled(),
            "EOF cancelled accepted provider work"
        );
        assert!(server
            .runtime
            .test_operation_ids()
            .iter()
            .any(|active| active == id));
        assert!(
            send_test_request(
                &server.record,
                ServiceRequestKind::Expire {
                    operation_id: id.into()
                }
            )
            .ok
        );
        wait_operation_retired(&server.runtime, id);
        assert!(token.is_cancelled());
        assert_eq!(
            server.executor.started.lock().unwrap().as_slice(),
            &[id.to_owned()]
        );
        let unknown = send_test_request(
            &server.record,
            ServiceRequestKind::Expire {
                operation_id: "not-observed".into(),
            },
        );
        assert!(!unknown.ok);
        assert!(unknown
            .error
            .unwrap()
            .contains("termination is unconfirmed"));
        assert!(
            send_test_request(
                &server.record,
                ServiceRequestKind::BslMcp {
                    operation_id: "success-after-expiry".into(),
                    tool_name: "diagnostics".to_string(),
                    tool_args: json!({}),
                    timeout_seconds: 5,
                    timeout_nanos: 0,
                    yields: false
                }
            )
            .ok
        );
    }

    #[test]
    fn prepared_bsl_receiver_anchor_includes_delayed_worker_without_expiry_delivery() {
        for finite in [false, true] {
            let mut server = WorkspaceAdmissionTestServer::start("prepared-delayed-worker");
            let gate = Arc::new((Mutex::new(false), Condvar::new()));
            server.gates.push(gate.clone());
            let (entered, observed) = mpsc::channel();
            server
                .runtime
                .test_set_prepared_dispatch_hook(Some(Arc::new({
                    let gate = gate.clone();
                    move || {
                        entered.send(()).unwrap();
                        wait_admission_test_gate(&gate);
                    }
                })));
            let id = "success-delayed-worker";
            let mut begin = prepared_begin(id);
            let deadline = if finite {
                BslOperationDeadline::Finite {
                    seconds: 0,
                    nanos: 30_000_000,
                }
            } else {
                BslOperationDeadline::Absent {}
            };
            if let ServiceRequestKind::BslMcpBegin {
                deadline: offered, ..
            } = &mut begin
            {
                *offered = deadline.clone();
            }
            let mut work = open_test_request(&server.record, begin);
            assert!(matches!(
                read_prepared_reply(&mut work),
                PreparedBslReply::Admitted { .. }
            ));
            write_prepared_phase(
                &mut work,
                &server.record,
                ServiceRequestKind::BslMcpStart {
                    operation_id: id.into(),
                    deadline,
                },
            );
            observed.recv_timeout(Duration::from_secs(3)).unwrap();
            // No Expire control is sent. Receiver must account for the delay itself.
            thread::sleep(Duration::from_millis(60));
            *gate.0.lock().unwrap() = true;
            gate.1.notify_all();
            let PreparedBslReply::Finished { response } = read_prepared_reply(&mut work) else {
                panic!("missing final result")
            };
            if finite {
                assert!(!response.ok);
                assert!(response.error.unwrap().starts_with("timeout:"));
                assert!(server.executor.started.lock().unwrap().is_empty());
            } else {
                assert!(response.ok);
                assert_eq!(
                    server.executor.started.lock().unwrap().as_slice(),
                    &[id.to_owned()]
                );
            }
        }
    }

    #[test]
    fn prepared_bsl_failed_separate_stop_delivery_retains_the_accepted_owner() {
        struct FailControlIo(AtomicUsize);
        impl ConnectorIo for FailControlIo {
            fn connect(
                &self,
                port: u16,
                timeout: Duration,
            ) -> io::Result<Box<dyn ConnectorStream>> {
                if self.0.fetch_add(1, Ordering::AcqRel) == 0 {
                    SystemConnectorIo.connect(port, timeout)
                } else {
                    Err(io::Error::new(
                        ErrorKind::ConnectionRefused,
                        "controlled control connection failure",
                    ))
                }
            }
        }
        static START: OnceLock<Instant> = OnceLock::new();
        static ADVANCE: AtomicUsize = AtomicUsize::new(0);
        fn now() -> Instant {
            *START.get().unwrap() + Duration::from_secs(ADVANCE.load(Ordering::Acquire) as u64)
        }
        let start = *START.get_or_init(Instant::now);
        for expiry in [false, true] {
            ADVANCE.store(0, Ordering::Release);
            thread::scope(|scope| {
                let server = WorkspaceAdmissionTestServer::start("prepared-undelivered-stop");
                let id = "held-with-unconfirmed-stop";
                let cancellation = CancellationToken::new();
                let deadline = if expiry {
                    ProviderDeadline::with_clock(start + Duration::from_secs(10), now)
                } else {
                    ProviderDeadline::no_deadline()
                };
                let mut begin = prepared_begin(id);
                if let ServiceRequestKind::BslMcpBegin {
                    deadline: offered, ..
                } = &mut begin
                {
                    *offered = BslOperationDeadline::from_remaining(deadline.remaining());
                }
                let io = Arc::new(FailControlIo(AtomicUsize::new(0)));
                let record = server.record.clone();
                let token = cancellation.clone();
                let transport = io.clone();
                let sender = scope.spawn(move || {
                    SYSTEM_SERVICE_CONNECTOR.send_prepared_with(
                        &record,
                        connector_test_request(begin),
                        &token,
                        deadline,
                        transport.as_ref(),
                        &SystemClock::start(),
                    )
                });
                server.executor.wait_started(1);
                let accepted = server.token(id);
                if expiry {
                    ADVANCE.store(20, Ordering::Release);
                } else {
                    cancellation.cancel();
                }
                let error = sender.join().unwrap().unwrap_err();
                assert!(
                    error.starts_with(if expiry { "timeout:" } else { "cancelled:" }),
                    "{error}"
                );
                assert!(error.contains("stop delivery is unconfirmed"), "{error}");
                assert_eq!(
                    io.0.load(Ordering::Acquire),
                    2,
                    "one work and one separate stop attempt, no replay"
                );
                assert!(
                    !accepted.is_cancelled(),
                    "failed control delivery is not a terminal stop"
                );
                assert!(server
                    .runtime
                    .test_operation_ids()
                    .iter()
                    .any(|active| active == id));
                assert_eq!(
                    server.executor.started.lock().unwrap().as_slice(),
                    &[id.to_owned()]
                );
                assert!(
                    send_test_request(
                        &server.record,
                        ServiceRequestKind::Expire {
                            operation_id: id.into()
                        }
                    )
                    .ok
                );
                wait_operation_retired(&server.runtime, id);
            });
        }
    }

    #[test]
    fn prepared_bsl_manager_retains_original_finite_cutoff_through_reuse_spawn_and_result() {
        static START: OnceLock<Instant> = OnceLock::new();
        static ADVANCE: AtomicUsize = AtomicUsize::new(0);
        fn now() -> Instant {
            *START.get().unwrap() + Duration::from_secs(ADVANCE.load(Ordering::Acquire) as u64)
        }
        struct PreparedConnector {
            original: ProviderDeadline,
            late: bool,
            exhaust: bool,
            expected_remaining: Duration,
            calls: AtomicUsize,
        }
        impl ServiceConnector for PreparedConnector {
            fn send(
                &self,
                _: &WorkspaceServiceRecord,
                _: ServiceRequest,
                _: &CancellationToken,
                _: Duration,
            ) -> Result<ServiceResponse, String> {
                unreachable!("typed operations must not fall back to legacy durations")
            }
            fn send_before(
                &self,
                _: &WorkspaceServiceRecord,
                _: ServiceRequest,
                _: &CancellationToken,
                _: OperationDeadline,
            ) -> Result<ServiceResponse, String> {
                unreachable!("typed operations must retain their original provider cutoff")
            }
            fn send_operation_before(
                &self,
                _: &WorkspaceServiceRecord,
                request: ServiceRequest,
                _: &CancellationToken,
                deadline: ProviderDeadline,
            ) -> Result<ServiceResponse, String> {
                assert_eq!(deadline, self.original);
                match request.kind {
                    ServiceRequestKind::Ping => {
                        ADVANCE.store(if self.exhaust { 11 } else { 3 }, Ordering::Release);
                        Ok(ServiceResponse {
                            ok: true,
                            status: Some("alive".into()),
                            ..ServiceResponse::default()
                        })
                    }
                    ServiceRequestKind::BslMcpBegin {
                        operation_id,
                        deadline: offered,
                        ..
                    } => {
                        Uuid::parse_str(&operation_id).unwrap();
                        self.calls.fetch_add(1, Ordering::AcqRel);
                        assert_eq!(offered.remaining().unwrap(), Some(self.expected_remaining));
                        assert_eq!(deadline.remaining(), Some(self.expected_remaining));
                        if self.late {
                            ADVANCE.store(11, Ordering::Release);
                        }
                        Ok(ServiceResponse {
                            ok: true,
                            result_text: Some("actual-result".into()),
                            ..ServiceResponse::default()
                        })
                    }
                    other => panic!("unexpected typed request: {other:?}"),
                }
            }
        }
        struct PreparedSpawner {
            original: ProviderDeadline,
            exhaust: bool,
            calls: AtomicUsize,
        }
        impl ServiceSpawner for PreparedSpawner {
            fn spawn(
                &self,
                _: &WorkspaceServiceIdentity,
                _: WorkspaceServiceConfig,
                _: &str,
                _: &CancellationToken,
                _: Duration,
            ) -> Result<WorkspaceServiceRecord, String> {
                unreachable!("typed startup must not rearm a duration")
            }
            fn spawn_operation_before(
                &self,
                identity: &WorkspaceServiceIdentity,
                _: WorkspaceServiceConfig,
                token: &str,
                _: &CancellationToken,
                deadline: ProviderDeadline,
            ) -> Result<WorkspaceServiceRecord, String> {
                assert_eq!(deadline, self.original);
                self.calls.fetch_add(1, Ordering::AcqRel);
                ADVANCE.store(if self.exhaust { 11 } else { 2 }, Ordering::Release);
                let mut record = test_record(identity, 34567, CoreIdentity::production().as_str());
                record.token = token.into();
                Ok(record)
            }
        }
        let start = *START.get_or_init(Instant::now);
        for reuse in [false, true] {
            for (late, exhaust) in [(false, false), (true, false), (false, true)] {
                ADVANCE.store(0, Ordering::Release);
                let context =
                    test_context(&format!("prepared-manager-cutoff-{reuse}-{late}-{exhaust}"));
                let root = context.workspace_root.join("src");
                let identity = WorkspaceServiceIdentity::new(&context, &root).unwrap();
                if reuse {
                    write_record(
                        &identity,
                        test_record(&identity, 34567, CoreIdentity::production().as_str()),
                    );
                }
                let original = ProviderDeadline::with_clock(start + Duration::from_secs(10), now);
                let connector = PreparedConnector {
                    original,
                    late,
                    exhaust,
                    expected_remaining: Duration::from_secs(if reuse { 7 } else { 8 }),
                    calls: AtomicUsize::new(0),
                };
                let spawner = PreparedSpawner {
                    original,
                    exhaust,
                    calls: AtomicUsize::new(0),
                };
                let manager = WorkspaceServiceManager::with_io(
                    &connector,
                    &spawner,
                    analysis_composition::service_environment(),
                );
                let result = manager.call_bsl_mcp_before(
                    &context,
                    &root,
                    PreparedBslCall {
                        tool_name: "symbol_info".into(),
                        tool_args: json!({}),
                        yields: false,
                    },
                    &CancellationToken::new(),
                    original,
                );
                if late || exhaust {
                    assert!(result.unwrap_err().starts_with("timeout:"));
                } else {
                    assert_eq!(result.unwrap().result_text, "actual-result");
                }
                assert_eq!(
                    connector.calls.load(Ordering::Acquire),
                    usize::from(!exhaust)
                );
                assert_eq!(spawner.calls.load(Ordering::Acquire), usize::from(!reuse));
                cleanup(&context);
            }
        }
    }

    #[test]
    fn prepared_bsl_control_steps_are_finite_without_replacing_the_business_deadline() {
        static START: OnceLock<Instant> = OnceLock::new();
        static ADVANCE: AtomicUsize = AtomicUsize::new(0);
        fn now() -> Instant {
            *START.get().unwrap() + Duration::from_secs(ADVANCE.load(Ordering::Acquire) as u64)
        }
        struct AckIo;
        struct AckStream;
        impl ConnectorIo for AckIo {
            fn connect(&self, _: u16, _: Duration) -> io::Result<Box<dyn ConnectorStream>> {
                Ok(Box::new(AckStream))
            }
        }
        impl Read for AckStream {
            fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
                ADVANCE.store(3, Ordering::Release);
                let reply = b"{\"ok\":true,\"status\":\"alive\"}\n";
                bytes[..reply.len()].copy_from_slice(reply);
                Ok(reply.len())
            }
        }
        impl Write for AckStream {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        impl ConnectorStream for AckStream {
            fn set_read_timeout(&self, _: Option<Duration>) -> io::Result<()> {
                Ok(())
            }
            fn set_write_timeout(&self, _: Option<Duration>) -> io::Result<()> {
                Ok(())
            }
        }
        struct Connector {
            original: ProviderDeadline,
            controls: AtomicUsize,
            begins: AtomicUsize,
        }
        impl ServiceConnector for Connector {
            fn send(
                &self,
                _: &WorkspaceServiceRecord,
                _: ServiceRequest,
                _: &CancellationToken,
                _: Duration,
            ) -> Result<ServiceResponse, String> {
                panic!("a control step must retain typed deadline dispatch");
            }
            fn send_before(
                &self,
                _: &WorkspaceServiceRecord,
                _: ServiceRequest,
                _: &CancellationToken,
                _: OperationDeadline,
            ) -> Result<ServiceResponse, String> {
                panic!("a prepared call must retain its provider deadline");
            }
            fn send_operation_before(
                &self,
                record: &WorkspaceServiceRecord,
                request: ServiceRequest,
                cancellation: &CancellationToken,
                deadline: ProviderDeadline,
            ) -> Result<ServiceResponse, String> {
                assert_eq!(deadline, self.original, "the manager must not replace an absent/long business deadline with a control deadline");
                match &request.kind {
                    ServiceRequestKind::Ping | ServiceRequestKind::Shutdown => {
                        self.controls.fetch_add(1, Ordering::AcqRel);
                        let result = SYSTEM_SERVICE_CONNECTOR.test_send_operation_with(
                            record,
                            request,
                            cancellation,
                            deadline,
                            &AckIo,
                            &ManualClock::default(),
                        );
                        if self.original.remaining() == Some(Duration::from_secs(7)) {
                            assert_eq!(deadline.remaining(), Some(Duration::from_secs(7)), "the selected original deadline must retain its clock through control I/O");
                        }
                        result
                    }
                    ServiceRequestKind::BslMcpBegin {
                        deadline: offered, ..
                    } => {
                        assert_eq!(offered.remaining().unwrap(), self.original.remaining());
                        self.begins.fetch_add(1, Ordering::AcqRel);
                        Ok(ServiceResponse {
                            ok: true,
                            result_text: Some("business-result".into()),
                            ..ServiceResponse::default()
                        })
                    }
                    other => panic!("unexpected request: {other:?}"),
                }
            }
        }
        struct Spawner {
            original: ProviderDeadline,
        }
        impl ServiceSpawner for Spawner {
            fn spawn(
                &self,
                _: &WorkspaceServiceIdentity,
                _: WorkspaceServiceConfig,
                _: &str,
                _: &CancellationToken,
                _: Duration,
            ) -> Result<WorkspaceServiceRecord, String> {
                panic!("typed startup required");
            }
            fn spawn_operation_before(
                &self,
                identity: &WorkspaceServiceIdentity,
                _: WorkspaceServiceConfig,
                token: &str,
                _: &CancellationToken,
                deadline: ProviderDeadline,
            ) -> Result<WorkspaceServiceRecord, String> {
                assert_eq!(deadline, self.original);
                let mut record = test_record(identity, 34567, CoreIdentity::production().as_str());
                record.token = token.into();
                Ok(record)
            }
        }
        let start = *START.get_or_init(Instant::now);
        let current_build = CoreIdentity::production();
        for mismatch in [false, true] {
            for budget in [
                None,
                Some(Duration::from_secs(240)),
                Some(Duration::from_secs(10)),
            ] {
                for stopped in ["running", "cancelled", "expired"] {
                    ADVANCE.store(0, Ordering::Release);
                    let context = test_context(&format!(
                        "prepared-control-step-{mismatch}-{budget:?}-{stopped}"
                    ));
                    let root = context.workspace_root.join("src");
                    let identity = WorkspaceServiceIdentity::new(&context, &root).unwrap();
                    write_record(
                        &identity,
                        test_record(
                            &identity,
                            34567,
                            if mismatch {
                                "replaced-build"
                            } else {
                                current_build.as_str()
                            },
                        ),
                    );
                    let original = if stopped == "expired" {
                        ProviderDeadline::with_clock(start, now)
                    } else {
                        budget.map_or(ProviderDeadline::no_deadline(), |budget| {
                            ProviderDeadline::with_clock(start + budget, now)
                        })
                    };
                    let cancellation = CancellationToken::new();
                    if stopped == "cancelled" {
                        cancellation.cancel();
                    }
                    let connector = Connector {
                        original,
                        controls: AtomicUsize::new(0),
                        begins: AtomicUsize::new(0),
                    };
                    let spawner = Spawner { original };
                    let manager = WorkspaceServiceManager::with_io(
                        &connector,
                        &spawner,
                        analysis_composition::service_environment(),
                    );
                    let result = manager.call_bsl_mcp_before(
                        &context,
                        &root,
                        PreparedBslCall {
                            tool_name: "symbol_info".into(),
                            tool_args: json!({}),
                            yields: false,
                        },
                        &cancellation,
                        original,
                    );
                    if stopped == "running" {
                        assert_eq!(result.unwrap().result_text, "business-result");
                        assert_eq!(connector.controls.load(Ordering::Acquire), 1);
                        assert_eq!(connector.begins.load(Ordering::Acquire), 1);
                    } else {
                        let error = result.unwrap_err();
                        assert!(
                            error.starts_with(if stopped == "cancelled" {
                                "cancelled:"
                            } else {
                                "timeout:"
                            }),
                            "{error}"
                        );
                        assert_eq!(connector.controls.load(Ordering::Acquire), 0);
                        assert_eq!(connector.begins.load(Ordering::Acquire), 0);
                    }
                    cleanup(&context);
                }
            }
        }
    }

    #[test]
    fn prepared_bsl_control_transport_observes_the_selected_original_clock() {
        static START: OnceLock<Instant> = OnceLock::new();
        static ADVANCE: AtomicUsize = AtomicUsize::new(0);
        fn now() -> Instant {
            *START.get().unwrap() + Duration::from_secs(ADVANCE.load(Ordering::Acquire) as u64)
        }
        #[derive(Clone, Copy, Debug)]
        enum Reply {
            AfterPoll,
            StepExpired,
            CallerExpired,
            Late,
            Cancel,
        }
        struct ProbeIo {
            clock: ManualClock,
            reads: Arc<AtomicUsize>,
            reply: Reply,
            cancellation: CancellationToken,
        }
        struct ProbeStream {
            clock: ManualClock,
            reads: Arc<AtomicUsize>,
            reply: Reply,
            cancellation: CancellationToken,
        }
        impl ConnectorIo for ProbeIo {
            fn connect(&self, _: u16, timeout: Duration) -> io::Result<Box<dyn ConnectorStream>> {
                assert!(timeout > Duration::ZERO && timeout <= SERVICE_REQUEST_TIMEOUT);
                Ok(Box::new(ProbeStream {
                    clock: self.clock.clone(),
                    reads: self.reads.clone(),
                    reply: self.reply,
                    cancellation: self.cancellation.clone(),
                }))
            }
        }
        impl Read for ProbeStream {
            fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
                let read = self.reads.fetch_add(1, Ordering::AcqRel);
                let reply = if read == 0 {
                    b"{\"ok\":".as_slice()
                } else {
                    assert!(
                        read == 1 || (read == 2 && matches!(self.reply, Reply::AfterPoll)),
                        "the actual control-step deadline must stop further polling"
                    );
                    if read == 1 {
                        let (step_elapsed, caller_elapsed) = match self.reply {
                            Reply::AfterPoll => (3, 3),
                            Reply::CallerExpired => (0, 11),
                            _ => (121, 121),
                        };
                        self.clock.advance(Duration::from_secs(step_elapsed));
                        ADVANCE.store(caller_elapsed, Ordering::Release);
                        if matches!(self.reply, Reply::Cancel) {
                            self.cancellation.cancel();
                        }
                        if !matches!(self.reply, Reply::Late) {
                            return Err(io::Error::new(
                                ErrorKind::TimedOut,
                                "control polling slice",
                            ));
                        }
                    }
                    b"true}\n".as_slice()
                };
                bytes[..reply.len()].copy_from_slice(reply);
                Ok(reply.len())
            }
        }
        impl Write for ProbeStream {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        impl ConnectorStream for ProbeStream {
            fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
                assert!(timeout.is_some_and(
                    |timeout| timeout > Duration::ZERO && timeout <= Duration::from_millis(100)
                ));
                Ok(())
            }
            fn set_write_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
                assert!(timeout.is_some_and(
                    |timeout| timeout > Duration::ZERO && timeout <= Duration::from_millis(100)
                ));
                Ok(())
            }
        }
        let start = *START.get_or_init(Instant::now);
        for shutdown in [false, true] {
            for budget in [
                None,
                Some(Duration::from_secs(240)),
                Some(Duration::from_secs(10)),
            ] {
                for reply in [
                    Reply::StepExpired,
                    Reply::CallerExpired,
                    Reply::AfterPoll,
                    Reply::Late,
                    Reply::Cancel,
                ] {
                    if matches!(reply, Reply::CallerExpired)
                        && budget != Some(Duration::from_secs(10))
                    {
                        continue;
                    }
                    ADVANCE.store(0, Ordering::Release);
                    let original = budget.map_or(ProviderDeadline::no_deadline(), |budget| {
                        ProviderDeadline::with_clock(start + budget, now)
                    });
                    let cancellation = CancellationToken::new();
                    let clock = ManualClock::default();
                    let reads = Arc::new(AtomicUsize::new(0));
                    let io = ProbeIo {
                        clock: clock.clone(),
                        reads: reads.clone(),
                        reply,
                        cancellation: cancellation.clone(),
                    };
                    let kind = if shutdown {
                        ServiceRequestKind::Shutdown
                    } else {
                        ServiceRequestKind::Ping
                    };
                    let result = SYSTEM_SERVICE_CONNECTOR.test_send_operation_with(
                        &connector_test_record(),
                        connector_test_request(kind),
                        &cancellation,
                        original,
                        &io,
                        &clock,
                    );
                    if matches!(reply, Reply::AfterPoll) {
                        assert!(
                            result.unwrap().ok,
                            "the prefix before a polling timeout must survive"
                        );
                        assert_eq!(reads.load(Ordering::Acquire), 3);
                    } else {
                        let error = result.unwrap_err();
                        if matches!(reply, Reply::Cancel) {
                            assert!(error.starts_with("cancelled:"), "{error}");
                        } else if budget == Some(Duration::from_secs(10)) {
                            assert!(
                                error.contains("operation deadline elapsed"),
                                "the original caller clock must win: {error}"
                            );
                        } else {
                            assert_eq!(
                                error,
                                workspace_service_request_timeout_error(SERVICE_REQUEST_TIMEOUT),
                                "the transport step, not the business operation, expired"
                            );
                        }
                        assert_eq!(reads.load(Ordering::Acquire), 2);
                        if matches!(reply, Reply::CallerExpired) {
                            assert_eq!(
                                clock.elapsed(),
                                Duration::ZERO,
                                "the transport step is still live"
                            );
                            assert_eq!(
                                original.remaining(),
                                Some(Duration::ZERO),
                                "only the independent caller clock expired"
                            );
                        }
                    }
                    assert_eq!(
                        original.remaining().is_none(),
                        budget.is_none(),
                        "the control step must never overwrite the caller's absence"
                    );
                }
            }
        }
    }

    #[test]
    fn prepared_bsl_ping_step_timeout_preserves_the_live_service_and_accepted_neighbor() {
        struct SilentIo {
            clock: ManualClock,
        }
        struct SilentStream {
            clock: ManualClock,
            read: bool,
        }
        impl ConnectorIo for SilentIo {
            fn connect(&self, _: u16, _: Duration) -> io::Result<Box<dyn ConnectorStream>> {
                Ok(Box::new(SilentStream {
                    clock: self.clock.clone(),
                    read: false,
                }))
            }
        }
        impl Read for SilentStream {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                assert!(
                    !self.read,
                    "the actual control step must expire after this polling slice"
                );
                self.read = true;
                self.clock.advance(Duration::from_secs(121));
                Err(io::Error::new(ErrorKind::TimedOut, "silent Ping peer"))
            }
        }
        impl Write for SilentStream {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        impl ConnectorStream for SilentStream {
            fn set_read_timeout(&self, _: Option<Duration>) -> io::Result<()> {
                Ok(())
            }
            fn set_write_timeout(&self, _: Option<Duration>) -> io::Result<()> {
                Ok(())
            }
        }
        struct Connector {
            stall_ping: std::cell::Cell<bool>,
            shutdowns: AtomicUsize,
            begins: AtomicUsize,
        }
        impl ServiceConnector for Connector {
            fn send(
                &self,
                _: &WorkspaceServiceRecord,
                _: ServiceRequest,
                _: &CancellationToken,
                _: Duration,
            ) -> Result<ServiceResponse, String> {
                panic!("typed calls required");
            }
            fn send_before(
                &self,
                _: &WorkspaceServiceRecord,
                _: ServiceRequest,
                _: &CancellationToken,
                _: OperationDeadline,
            ) -> Result<ServiceResponse, String> {
                panic!("provider deadline required");
            }
            fn send_operation_before(
                &self,
                record: &WorkspaceServiceRecord,
                request: ServiceRequest,
                cancellation: &CancellationToken,
                deadline: ProviderDeadline,
            ) -> Result<ServiceResponse, String> {
                assert_eq!(deadline, ProviderDeadline::no_deadline());
                match &request.kind {
                    ServiceRequestKind::Ping if self.stall_ping.get() => {
                        let clock = ManualClock::default();
                        SYSTEM_SERVICE_CONNECTOR.test_send_operation_with(
                            record,
                            request,
                            cancellation,
                            deadline,
                            &SilentIo {
                                clock: clock.clone(),
                            },
                            &clock,
                        )
                    }
                    ServiceRequestKind::Ping => SYSTEM_SERVICE_CONNECTOR.send_operation_before(
                        record,
                        request,
                        cancellation,
                        deadline,
                    ),
                    ServiceRequestKind::Shutdown => {
                        self.shutdowns.fetch_add(1, Ordering::AcqRel);
                        // A forbidden shutdown reaches the real helper and its real held worker.
                        SYSTEM_SERVICE_CONNECTOR.send_operation_before(
                            record,
                            request,
                            cancellation,
                            deadline,
                        )
                    }
                    ServiceRequestKind::BslMcpBegin {
                        deadline: offered, ..
                    } => {
                        assert_eq!(offered.remaining().unwrap(), None);
                        self.begins.fetch_add(1, Ordering::AcqRel);
                        Ok(ServiceResponse {
                            ok: true,
                            result_text: Some("reused-service".into()),
                            ..ServiceResponse::default()
                        })
                    }
                    other => panic!("unexpected request {other:?}"),
                }
            }
        }
        struct Spawner {
            record: WorkspaceServiceRecord,
            calls: AtomicUsize,
        }
        impl ServiceSpawner for Spawner {
            fn spawn(
                &self,
                _: &WorkspaceServiceIdentity,
                _: WorkspaceServiceConfig,
                _: &str,
                _: &CancellationToken,
                _: Duration,
            ) -> Result<WorkspaceServiceRecord, String> {
                panic!("typed startup required");
            }
            fn spawn_operation_before(
                &self,
                _: &WorkspaceServiceIdentity,
                _: WorkspaceServiceConfig,
                _: &str,
                _: &CancellationToken,
                _: ProviderDeadline,
            ) -> Result<WorkspaceServiceRecord, String> {
                self.calls.fetch_add(1, Ordering::AcqRel);
                Ok(self.record.clone())
            }
        }
        let mut server = WorkspaceAdmissionTestServer::start("prepared-probe-neighbor");
        let mut neighbor_reader = server.work("held-probe-neighbor");
        server.executor.wait_started(1);
        let neighbor = server.token("held-probe-neighbor");
        let source_root = server.context.workspace_root.join("src");
        let identity = WorkspaceServiceIdentity::new(&server.context, &source_root).unwrap();
        let connector = Connector {
            stall_ping: std::cell::Cell::new(true),
            shutdowns: AtomicUsize::new(0),
            begins: AtomicUsize::new(0),
        };
        let spawner = Spawner {
            record: server.record.clone(),
            calls: AtomicUsize::new(0),
        };
        let manager = WorkspaceServiceManager::with_io(
            &connector,
            &spawner,
            analysis_composition::service_environment(),
        );
        let deadline = ProviderDeadline::no_deadline();
        let call = || PreparedBslCall {
            tool_name: "symbol_info".into(),
            tool_args: json!({}),
            yields: false,
        };
        let result = manager.call_bsl_mcp_before(
            &server.context,
            &source_root,
            call(),
            &CancellationToken::new(),
            deadline,
        );
        assert!(!neighbor.is_cancelled(), "a Ping step timeout must not send Shutdown and cancel an accepted neighboring operation");
        assert!(!server.runtime.test_shutting_down().load(Ordering::Acquire));
        assert_eq!(
            result.unwrap_err(),
            workspace_service_request_timeout_error(SERVICE_REQUEST_TIMEOUT)
        );
        assert!(
            !deadline.is_elapsed(),
            "transport failure is not expiry of this caller's absent operation deadline"
        );
        assert_eq!(connector.shutdowns.load(Ordering::Acquire), 0);
        assert_eq!(connector.begins.load(Ordering::Acquire), 0);
        assert_eq!(spawner.calls.load(Ordering::Acquire), 0);
        assert!(server
            .runtime
            .test_owns_record(&read_record(&identity).unwrap()));
        assert!(server
            .runtime
            .test_operation_ids()
            .iter()
            .any(|id| id == "held-probe-neighbor"));
        connector.stall_ping.set(false);
        assert_eq!(
            manager
                .call_bsl_mcp_before(
                    &server.context,
                    &source_root,
                    call(),
                    &CancellationToken::new(),
                    deadline
                )
                .unwrap()
                .result_text,
            "reused-service"
        );
        assert_eq!(connector.shutdowns.load(Ordering::Acquire), 0);
        assert_eq!(spawner.calls.load(Ordering::Acquire), 0);
        assert_eq!(connector.begins.load(Ordering::Acquire), 1);
        assert!(!neighbor.is_cancelled());
        assert!(server
            .runtime
            .test_owns_record(&read_record(&identity).unwrap()));
        assert!(
            send_test_request(
                &server.record,
                ServiceRequestKind::Cancel {
                    operation_id: "held-probe-neighbor".into()
                }
            )
            .ok
        );
        assert!(!read_test_response(&mut neighbor_reader).ok);
        assert!(
            neighbor.is_cancelled(),
            "explicit exact cancellation still owns the retained operation"
        );
    }

    #[test]
    fn prepared_bsl_admitted_delay_shrinks_start_and_original_cutoff_rejects_late_result() {
        static START: OnceLock<Instant> = OnceLock::new();
        static ADVANCE: AtomicUsize = AtomicUsize::new(0);
        fn now() -> Instant {
            *START.get().unwrap() + Duration::from_secs(ADVANCE.load(Ordering::Acquire) as u64)
        }
        fn accept(listener: &TcpListener) -> TcpStream {
            let fuse = Instant::now() + Duration::from_secs(3);
            loop {
                match listener.accept() {
                    Ok((stream, _)) => {
                        stream.set_nonblocking(false).unwrap();
                        stream
                            .set_read_timeout(Some(Duration::from_secs(3)))
                            .unwrap();
                        return stream;
                    }
                    Err(error) if error.kind() == ErrorKind::WouldBlock => {
                        assert!(
                            Instant::now() < fuse,
                            "expected prepared connection was not opened"
                        );
                        thread::yield_now();
                    }
                    Err(error) => panic!("accept prepared peer: {error}"),
                }
            }
        }
        let start = *START.get_or_init(Instant::now);
        for (expired_admission, late_result) in [(false, false), (true, false), (false, true)] {
            ADVANCE.store(0, Ordering::Release);
            let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
            listener.set_nonblocking(true).unwrap();
            let mut record = connector_test_record();
            record.port = listener.local_addr().unwrap().port();
            thread::scope(|scope| {
                let peer = scope.spawn(move || {
                    let mut stream = accept(&listener);
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    let begin: ServiceRequest = serde_json::from_str(&line).unwrap();
                    let ServiceRequestKind::BslMcpBegin { operation_id, .. } = begin.kind else { panic!("not Begin") };
                    serde_json::to_writer(&mut stream, &PreparedBslReply::Admitted { operation_id: operation_id.clone() }).unwrap();
                    writeln!(stream).unwrap();
                    stream.flush().unwrap();
                    line.clear();
                    let count = match reader.read_line(&mut line) {
                        Ok(count) => count,
                        Err(error) if expired_admission && error.kind() == ErrorKind::ConnectionReset => 0,
                        Err(error) => panic!("read prepared phase: {error}"),
                    };
                    if expired_admission {
                        if count != 0 {
                            let stop: ServiceRequest = serde_json::from_str(&line).unwrap();
                            assert!(matches!(stop.kind, ServiceRequestKind::BslMcpStop { operation_id: id, cause: BslStopCause::Expire } if id == operation_id));
                        }
                        return;
                    }
                    let start_request: ServiceRequest = serde_json::from_str(&line).unwrap();
                    let ServiceRequestKind::BslMcpStart { operation_id: id, deadline } = start_request.kind else { panic!("not Start") };
                    assert_eq!(id, operation_id);
                    assert_eq!(deadline.remaining().unwrap(), Some(Duration::from_secs(7)), "admission latency must consume the original budget");
                    serde_json::to_writer(&mut stream, &PreparedBslReply::Finished { response: Box::new(ServiceResponse { ok: true, result_text: Some("observed-provider-result".into()), ..ServiceResponse::default() }) }).unwrap();
                    writeln!(stream).unwrap();
                    stream.flush().unwrap();
                    if late_result {
                        let mut control = BufReader::new(accept(&listener));
                        line.clear();
                        control.read_line(&mut line).unwrap();
                        let stop: ServiceRequest = serde_json::from_str(&line).unwrap();
                        assert!(matches!(stop.kind, ServiceRequestKind::Expire { operation_id: id } if id == operation_id));
                    }
                });
                let original = ProviderDeadline::with_clock(start + Duration::from_secs(10), now);
                let mut request = prepared_begin("prepared-delayed-admission");
                if let ServiceRequestKind::BslMcpBegin { deadline, .. } = &mut request {
                    *deadline = BslOperationDeadline::from_remaining(original.remaining());
                }
                let trace = Arc::new(Mutex::new(Vec::new()));
                let transport = PreparedTestWireIo {
                    trace: trace.clone(),
                    disconnect_after_admitted: None,
                    after_reply: Some(Arc::new(move |reply| {
                        // Advance only after the actual bytes were delivered, before sender admission.
                        if reply == 1 {
                            ADVANCE
                                .store(if expired_admission { 11 } else { 3 }, Ordering::Release);
                        } else if late_result {
                            ADVANCE.store(11, Ordering::Release);
                        }
                    })),
                };
                let result = SYSTEM_SERVICE_CONNECTOR.send_prepared_with(
                    &record,
                    connector_test_request(request),
                    &CancellationToken::new(),
                    original,
                    &transport,
                    &SystemClock::start(),
                );
                peer.join().unwrap();
                let bytes = trace.lock().unwrap().clone();
                let sent: Vec<ServiceRequest> = bytes
                    .split(|byte| *byte == b'\n')
                    .filter(|line| !line.is_empty())
                    .map(|line| serde_json::from_slice(line).unwrap())
                    .collect();
                assert_eq!(
                    sent.iter()
                        .filter(|request| matches!(
                            request.kind,
                            ServiceRequestKind::BslMcpStart { .. }
                        ))
                        .count(),
                    usize::from(!expired_admission),
                    "expired admission must never submit Start, even when peer observes TCP reset"
                );

                if expired_admission || late_result {
                    assert!(result.unwrap_err().starts_with("timeout:"));
                } else {
                    assert_eq!(
                        result.unwrap().result_text.as_deref(),
                        Some("observed-provider-result")
                    );
                }
            });
        }
    }

    type PreparedReplyReadObserver = dyn Fn(usize) + Send + Sync;

    struct PreparedTestWireIo {
        trace: Arc<Mutex<Vec<u8>>>,
        disconnect_after_admitted: Option<Arc<AdmissionTestExecutor>>,
        after_reply: Option<Arc<PreparedReplyReadObserver>>,
    }
    struct PreparedTestWireStream {
        stream: TcpStream,
        admitted: bool,
        replies: usize,
        trace: Arc<Mutex<Vec<u8>>>,
        disconnect_after_admitted: Option<Arc<AdmissionTestExecutor>>,
        after_reply: Option<Arc<PreparedReplyReadObserver>>,
    }
    impl Read for PreparedTestWireStream {
        fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            if self.admitted {
                if let Some(executor) = &self.disconnect_after_admitted {
                    executor.wait_started(1);
                    self.stream.shutdown(std::net::Shutdown::Both)?;
                    return Ok(0);
                }
            }
            let count = self.stream.read(bytes)?;
            let complete = bytes[..count].iter().filter(|byte| **byte == b'\n').count();
            self.admitted |= complete != 0;
            if complete != 0 {
                self.replies += complete;
                if let Some(observe) = &self.after_reply {
                    observe(self.replies);
                }
            }
            Ok(count)
        }
    }
    impl Write for PreparedTestWireStream {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let written = self.stream.write(&bytes[..bytes.len().min(7)])?;
            self.trace
                .lock()
                .unwrap()
                .extend_from_slice(&bytes[..written]);
            Ok(written)
        }
        fn flush(&mut self) -> io::Result<()> {
            self.stream.flush()
        }
    }
    impl ConnectorStream for PreparedTestWireStream {
        fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
            self.stream.set_read_timeout(timeout)
        }
        fn set_write_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
            self.stream.set_write_timeout(timeout)
        }
    }
    impl ConnectorIo for PreparedTestWireIo {
        fn connect(&self, port: u16, timeout: Duration) -> io::Result<Box<dyn ConnectorStream>> {
            let stream = TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), timeout)?;
            Ok(Box::new(PreparedTestWireStream {
                stream,
                admitted: false,
                replies: 0,
                after_reply: self.after_reply.clone(),
                trace: self.trace.clone(),
                disconnect_after_admitted: self.disconnect_after_admitted.clone(),
            }))
        }
    }

    #[test]
    fn prepared_bsl_ambiguous_accepted_start_is_not_replayed_or_implicitly_cancelled() {
        struct AmbiguousConnector {
            executor: Arc<AdmissionTestExecutor>,
            trace: Arc<Mutex<Vec<u8>>>,
            attempts: AtomicUsize,
        }
        impl ServiceConnector for AmbiguousConnector {
            fn send(
                &self,
                _: &WorkspaceServiceRecord,
                _: ServiceRequest,
                _: &CancellationToken,
                _: Duration,
            ) -> Result<ServiceResponse, String> {
                unreachable!("no legacy fallback")
            }
            fn send_before(
                &self,
                _: &WorkspaceServiceRecord,
                _: ServiceRequest,
                _: &CancellationToken,
                _: OperationDeadline,
            ) -> Result<ServiceResponse, String> {
                unreachable!("no legacy fallback")
            }
            fn send_operation_before(
                &self,
                record: &WorkspaceServiceRecord,
                request: ServiceRequest,
                cancellation: &CancellationToken,
                deadline: ProviderDeadline,
            ) -> Result<ServiceResponse, String> {
                if matches!(request.kind, ServiceRequestKind::BslMcpBegin { .. }) {
                    assert_eq!(
                        self.attempts.fetch_add(1, Ordering::AcqRel),
                        0,
                        "ambiguous accepted Start must not be replayed"
                    );
                    SYSTEM_SERVICE_CONNECTOR.send_prepared_with(
                        record,
                        request,
                        cancellation,
                        deadline,
                        &PreparedTestWireIo {
                            trace: self.trace.clone(),
                            disconnect_after_admitted: Some(self.executor.clone()),
                            after_reply: None,
                        },
                        &SystemClock::start(),
                    )
                } else {
                    SYSTEM_SERVICE_CONNECTOR.send_operation_before(
                        record,
                        request,
                        cancellation,
                        deadline,
                    )
                }
            }
        }
        let server = WorkspaceAdmissionTestServer::start("prepared-ambiguous-accepted-start");
        let trace = Arc::new(Mutex::new(Vec::new()));
        let connector = AmbiguousConnector {
            executor: server.executor.clone(),
            trace: trace.clone(),
            attempts: AtomicUsize::new(0),
        };
        let spawner = RecordingSpawner::default();
        let manager = WorkspaceServiceManager::with_io(
            &connector,
            &spawner,
            analysis_composition::service_environment(),
        );
        let result = manager.call_bsl_mcp_before(
            &server.context,
            &server.context.workspace_root.join("src"),
            PreparedBslCall {
                tool_name: "diagnostics".into(),
                tool_args: json!({}),
                yields: false,
            },
            &CancellationToken::new(),
            ProviderDeadline::no_deadline(),
        );
        assert!(result
            .unwrap_err()
            .contains("disconnected before responding"));
        let bytes = trace.lock().unwrap().clone();
        let phases: Vec<ServiceRequest> = bytes
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_slice(line).unwrap())
            .collect();
        assert_eq!(phases.len(), 2);
        let ServiceRequestKind::BslMcpBegin {
            operation_id: id, ..
        } = &phases[0].kind
        else {
            panic!("not Begin")
        };
        assert!(
            matches!(&phases[1].kind, ServiceRequestKind::BslMcpStart { operation_id, .. } if operation_id == id)
        );
        Uuid::parse_str(id).unwrap();
        assert_eq!(
            server.executor.started.lock().unwrap().as_slice(),
            std::slice::from_ref(id)
        );
        assert!(
            !server.token(id).is_cancelled(),
            "an ambiguous transport outcome must not invent explicit cancellation"
        );
        assert_eq!(connector.attempts.load(Ordering::Acquire), 1);
        assert!(
            send_test_request(
                &server.record,
                ServiceRequestKind::Expire {
                    operation_id: id.clone()
                }
            )
            .ok
        );
        wait_operation_retired(&server.runtime, id);
    }

    #[test]
    fn prepared_bsl_connect_failure_preserves_original_expiry_and_cancel_precedence() {
        static START: OnceLock<Instant> = OnceLock::new();
        static ADVANCE: AtomicUsize = AtomicUsize::new(0);
        fn now() -> Instant {
            *START.get().unwrap() + Duration::from_secs(ADVANCE.load(Ordering::Acquire) as u64)
        }
        struct FailingConnect {
            cancellation: CancellationToken,
            cancel: bool,
            expire: bool,
            connections: AtomicUsize,
        }
        impl ConnectorIo for FailingConnect {
            fn connect(&self, _: u16, _: Duration) -> io::Result<Box<dyn ConnectorStream>> {
                self.connections.fetch_add(1, Ordering::AcqRel);
                if self.expire {
                    ADVANCE.store(10, Ordering::Release);
                }
                if self.cancel {
                    self.cancellation.cancel();
                }
                Err(io::Error::new(
                    ErrorKind::ConnectionRefused,
                    "controlled prepared connect failure",
                ))
            }
        }
        let start = *START.get_or_init(Instant::now);
        for (cancel, expire) in [(false, false), (false, true), (true, false), (true, true)] {
            ADVANCE.store(0, Ordering::Release);
            let cancellation = CancellationToken::new();
            let io = FailingConnect {
                cancellation: cancellation.clone(),
                cancel,
                expire,
                connections: AtomicUsize::new(0),
            };
            let original = ProviderDeadline::with_clock(start + Duration::from_secs(5), now);
            let error = SYSTEM_SERVICE_CONNECTOR
                .send_prepared_with(
                    &connector_test_record(),
                    connector_test_request(prepared_begin("connect-never-submitted")),
                    &cancellation,
                    original,
                    &io,
                    &SystemClock::start(),
                )
                .unwrap_err();
            let expected = if cancel {
                "cancelled:"
            } else if expire {
                "timeout:"
            } else {
                "failed to connect workspace service:"
            };
            assert!(
                error.starts_with(expected),
                "cancel={cancel}, expire={expire}: {error}"
            );
            assert_eq!(io.connections.load(Ordering::Acquire), 1);
        }
    }

    #[test]
    fn prepared_bsl_sender_rejects_success_before_start() {
        for wrong_admitted in [false, true] {
            let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
            let mut record = connector_test_record();
            record.port = listener.local_addr().unwrap().port();
            thread::scope(|scope| {
                let peer = scope.spawn(move || {
                    let (mut stream, _) = listener.accept().unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(3)))
                        .unwrap();
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut begin = String::new();
                    reader.read_line(&mut begin).unwrap();
                    let request: ServiceRequest = serde_json::from_str(&begin).unwrap();
                    assert!(matches!(
                        request.kind,
                        ServiceRequestKind::BslMcpBegin { .. }
                    ));
                    let reply = if wrong_admitted {
                        PreparedBslReply::Admitted {
                            operation_id: "foreign-admission".into(),
                        }
                    } else {
                        PreparedBslReply::Finished {
                            response: Box::new(ServiceResponse {
                                ok: true,
                                ..ServiceResponse::default()
                            }),
                        }
                    };
                    serde_json::to_writer(&mut stream, &reply).unwrap();
                    writeln!(stream).unwrap();
                    stream.flush().unwrap();
                    let mut phase = String::new();
                    assert_eq!(
                        reader.read_line(&mut phase).unwrap(),
                        0,
                        "a refused admission must never submit Start: {phase}"
                    );
                });
                let result = SYSTEM_SERVICE_CONNECTOR.send_operation_before(
                    &record,
                    connector_test_request(prepared_begin("not-executed")),
                    &CancellationToken::new(),
                    ProviderDeadline::no_deadline(),
                );
                peer.join().unwrap();
                assert!(
                    result.is_err(),
                    "fabricated success or foreign admission was accepted"
                );
            });
        }
    }

    #[test]
    fn prepared_bsl_phase_uuid_cannot_dispatch_or_stop_a_held_neighbor() {
        for stop in [false, true] {
            let mut server = WorkspaceAdmissionTestServer::start("prepared-foreign-phase");
            let _neighbor_work = server.work("held-neighbor");
            server.executor.wait_started(1);
            let neighbor = server.token("held-neighbor");
            let mut work =
                open_test_request(&server.record, prepared_begin("pending-own-admission"));
            assert!(
                matches!(read_prepared_reply(&mut work), PreparedBslReply::Admitted { operation_id } if operation_id == "pending-own-admission")
            );
            let phase = if stop {
                ServiceRequestKind::BslMcpStop {
                    operation_id: "held-neighbor".into(),
                    cause: BslStopCause::Cancel,
                }
            } else {
                ServiceRequestKind::BslMcpStart {
                    operation_id: "held-neighbor".into(),
                    deadline: BslOperationDeadline::Absent {},
                }
            };
            write_prepared_phase(&mut work, &server.record, phase);
            let PreparedBslReply::Finished { response } = read_prepared_reply(&mut work) else {
                panic!("missing foreign-phase refusal")
            };
            assert!(!response.ok);
            wait_operation_retired(&server.runtime, "pending-own-admission");
            assert!(!neighbor.is_cancelled());
            assert!(server
                .runtime
                .test_operation_ids()
                .iter()
                .any(|id| id == "held-neighbor"));
            assert_eq!(
                server.executor.started.lock().unwrap().as_slice(),
                &["held-neighbor"]
            );
            assert!(
                send_test_request(
                    &server.record,
                    ServiceRequestKind::Expire {
                        operation_id: "held-neighbor".into()
                    }
                )
                .ok
            );
            wait_operation_retired(&server.runtime, "held-neighbor");
        }
    }

    #[test]
    fn prepared_bsl_late_stop_does_not_replace_a_factual_provider_failure() {
        for id in ["factual-error-after-stop", "late-success-after-stop"] {
            let server = WorkspaceAdmissionTestServer::start(id);
            let mut work = open_test_request(&server.record, prepared_begin(id));
            assert!(matches!(
                read_prepared_reply(&mut work),
                PreparedBslReply::Admitted { .. }
            ));
            write_prepared_phase(
                &mut work,
                &server.record,
                ServiceRequestKind::BslMcpStart {
                    operation_id: id.into(),
                    deadline: BslOperationDeadline::Absent {},
                },
            );
            server.executor.wait_started(1);
            assert!(
                send_test_request(
                    &server.record,
                    ServiceRequestKind::Expire {
                        operation_id: id.into()
                    }
                )
                .ok
            );
            let PreparedBslReply::Finished { response } = read_prepared_reply(&mut work) else {
                panic!("missing failure")
            };
            if id.starts_with("late-success") {
                assert!(response.ok);
                assert_eq!(response.result_text.as_deref(), Some("actual-late-success"));
                assert!(response.error.is_none());
            } else {
                assert!(!response.ok);
                assert_eq!(
                    response.error.as_deref(),
                    Some("controlled provider protocol failure")
                );
            }
        }
    }

    #[test]
    fn workspace_service_handler_failures_have_stable_diagnostics() {
        assert_eq!(workspace_service_handler_diagnostic(Ok(Ok(()))), None);
        assert_eq!(
            workspace_service_handler_diagnostic(Ok(Err("header timed out".to_string()))),
            Some("workspace service connection failed: header timed out".to_string())
        );
        let panic_result: thread::Result<Result<(), String>> = Err(Box::new("panic"));
        assert_eq!(
            workspace_service_handler_diagnostic(panic_result),
            Some("workspace service connection handler panicked".to_string())
        );

        report_workspace_service_handler_result_to(
            &mut FailingWriter,
            Ok(Err("header timed out".to_string())),
        );
    }

    /// rule: INV.RUNTIME.SERVICE-CONTROL-CAPACITY
    #[test]
    fn slow_general_headers_cannot_exhaust_control_capacity() {
        let mut server = WorkspaceAdmissionTestServer::start("header-control-reserve");
        server.hold_headers(4);
        let started = Instant::now();
        let mut work = server.work("success-with-slow-headers");
        assert!(read_test_response(&mut work).ok);
        assert!(send_test_request(&server.record, ServiceRequestKind::Ping).ok);
        assert!(
            send_test_request(
                &server.record,
                ServiceRequestKind::Cancel {
                    operation_id: "none".into()
                }
            )
            .ok
        );
        assert!(send_test_request(&server.record, ServiceRequestKind::Shutdown).ok);
        assert!(started.elapsed() < SERVICE_CONTROL_CONNECT_TIMEOUT);
        let identity = WorkspaceServiceIdentity::new(
            &server.context,
            &server.context.workspace_root.join("src"),
        )
        .unwrap();
        drop(server);
        assert!(!identity.test_record_path().exists());
    }

    #[derive(Default)]
    struct AdmissionTestExecutor {
        started: Mutex<Vec<String>>,
        changed: std::sync::Condvar,
    }

    impl AdmissionTestExecutor {
        fn wait_started(&self, expected: usize) {
            let watchdog = Instant::now() + Duration::from_secs(3);
            let mut started = self.started.lock().unwrap();
            while started.len() < expected {
                let remaining = watchdog.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    let observed = started.clone();
                    drop(started);
                    panic!("only {observed:?} reached the executor");
                }
                (started, _) = self.changed.wait_timeout(started, remaining).unwrap();
            }
        }
    }

    impl GenericWorkspaceServiceOperationExecutor<BslWorkspaceBackend> for AdmissionTestExecutor {
        fn execute(
            &self,
            _runtime: &WorkspaceServiceRuntime,
            kind: ServiceRequestKind,
            cancellation: &CancellationToken,
        ) -> ServiceResponse {
            let id = kind.operation_id().unwrap().to_owned();
            let mut started = self.started.lock().unwrap();
            started.push(id.clone());
            self.changed.notify_all();
            if id.starts_with("success") {
                return ServiceResponse {
                    ok: true,
                    status: Some(id),
                    ..ServiceResponse::default()
                };
            }
            while !cancellation.is_cancelled() {
                (started, _) = self
                    .changed
                    .wait_timeout(started, Duration::from_millis(10))
                    .unwrap();
            }
            if id.starts_with("late-success") {
                return ServiceResponse {
                    ok: true,
                    result_text: Some("actual-late-success".into()),
                    ..ServiceResponse::default()
                };
            }
            if id.starts_with("factual-error") {
                return unica_analysis_services::test_support::error_response(
                    "controlled provider protocol failure",
                );
            }
            unica_analysis_services::test_support::error_response(cancelled_error(&id))
        }
    }

    type AdmissionTestGate = Arc<(Mutex<bool>, std::sync::Condvar)>;

    fn wait_admission_test_gate(gate: &AdmissionTestGate) {
        let (released, changed) = &**gate;
        let mut released = released.lock().unwrap();
        while !*released {
            released = changed.wait(released).unwrap();
        }
    }

    struct WorkspaceAdmissionTestServer {
        context: WorkspaceContext,
        record: WorkspaceServiceRecord,
        runtime: Arc<WorkspaceServiceRuntime>,
        executor: Arc<AdmissionTestExecutor>,
        server: Option<thread::JoinHandle<Result<(), String>>>,
        streams: Vec<TcpStream>,
        gates: Vec<AdmissionTestGate>,
    }

    impl WorkspaceAdmissionTestServer {
        fn start(name: &str) -> Self {
            let executor = Arc::new(AdmissionTestExecutor::default());
            let (context, record, runtime, server) =
                workspace_test_server_with_executor(name, executor.clone(), SERVICE_SHUTDOWN_GRACE);
            Self {
                context,
                record,
                runtime,
                executor,
                server: Some(server),
                streams: Vec::new(),
                gates: Vec::new(),
            }
        }

        fn retain(&mut self, stream: &TcpStream) {
            self.streams.push(stream.try_clone().unwrap());
        }

        fn work(&mut self, id: &str) -> BufReader<TcpStream> {
            let reader = open_test_request(&self.record, admission_test_work(id));
            self.retain(reader.get_ref());
            reader
        }

        fn hold_headers(&mut self, count: usize) {
            let gate = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
            self.gates.push(gate.clone());
            let seen = AtomicUsize::new(0);
            let (entered, observed) = mpsc::channel();
            *self.runtime.test_handler_started_hook().lock().unwrap() = Some(Arc::new(move || {
                let index = seen.fetch_add(1, Ordering::AcqRel) + 1;
                let _ = entered.send(index);
                if index <= count {
                    wait_admission_test_gate(&gate);
                }
            }));
            for expected in 1..=count {
                let mut stream = TcpStream::connect(("127.0.0.1", self.record.port)).unwrap();
                self.retain(&stream);
                stream.write_all(b"{").unwrap();
                assert_eq!(
                    observed
                        .recv_timeout(Duration::from_secs(2))
                        .expect("real partial-header handler starts"),
                    expected
                );
            }
        }

        fn token(&self, id: &str) -> CancellationToken {
            self.runtime
                .test_operation_cancellation(id)
                .expect("operation remains registered")
        }
    }

    impl Drop for WorkspaceAdmissionTestServer {
        fn drop(&mut self) {
            for gate in &self.gates {
                let (released, changed) = &**gate;
                *released.lock().unwrap() = true;
                changed.notify_all();
            }
            for stream in &self.streams {
                let _ = stream.shutdown(std::net::Shutdown::Both);
            }
            self.runtime.test_begin_shutdown();
            if let Some(server) = self.server.take() {
                let _ = server.join();
            }
            cleanup(&self.context);
        }
    }

    fn admission_test_work(id: &str) -> ServiceRequestKind {
        ServiceRequestKind::BslMcp {
            operation_id: id.to_owned(),
            tool_name: "diagnostics".to_string(),
            tool_args: json!({}),
            timeout_seconds: 30,
            timeout_nanos: 0,
            yields: false,
        }
    }

    /// rule: INV.RUNTIME.SERVICE-CONTROL-CAPACITY
    #[test]
    fn workspace_service_admission_ninth_work_runs_with_exact_cancel_ping_and_shutdown() {
        let mut server = WorkspaceAdmissionTestServer::start("admission-nine-workers");
        let mut work = Vec::new();
        for index in 0..8 {
            work.push(server.work(&format!("held-{index}")));
        }
        server.executor.wait_started(8);
        let mut ninth = server.work("held-ninth");
        server.executor.wait_started(9);
        let target = server.token("held-ninth");
        let other = server.token("held-0");
        assert!(send_test_request(&server.record, ServiceRequestKind::Ping).ok);
        assert!(
            send_test_request(
                &server.record,
                ServiceRequestKind::Cancel {
                    operation_id: "held-ninth".into()
                }
            )
            .ok
        );
        assert!(!read_test_response(&mut ninth).ok);
        assert!(target.is_cancelled());
        assert!(
            !other.is_cancelled(),
            "exact cancel must preserve the other work"
        );
        assert!(send_test_request(&server.record, ServiceRequestKind::Shutdown).ok);
        for reader in &mut work {
            assert!(!read_test_response(reader).ok);
        }
        assert!(other.is_cancelled());
    }

    /// rule: INV.RUNTIME.SERVICE-CONTROL-CAPACITY
    #[test]
    fn workspace_service_admission_sixty_fifth_partial_header_reaches_handler_and_executes_work() {
        let mut server = WorkspaceAdmissionTestServer::start("admission-65-headers");
        server.hold_headers(64);
        let (entered, observed) = mpsc::channel();
        // The first 64 real handlers already hold their previous hook's gate.
        *server.runtime.test_handler_started_hook().lock().unwrap() = Some(Arc::new(move || {
            let _ = entered.send(());
        }));
        let mut stream = TcpStream::connect(("127.0.0.1", server.record.port)).unwrap();
        server.retain(&stream);
        stream.write_all(b"{").unwrap();
        observed
            .recv_timeout(Duration::from_secs(2))
            .expect("the 65th actual handler must enter while the other headers are held");
        let bytes = serde_json::to_vec(&ServiceRequest {
            token: server.record.token.clone(),
            kind: admission_test_work("success-after-64-headers"),
        })
        .unwrap();
        stream.write_all(&bytes[1..]).unwrap();
        stream.write_all(b"\n").unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let result = read_test_response(&mut BufReader::new(stream));
        assert!(result.ok, "{}", result.error.unwrap_or_default());
        assert_eq!(result.status.as_deref(), Some("success-after-64-headers"));
        server.executor.wait_started(1);
        assert!(send_test_request(&server.record, ServiceRequestKind::Shutdown).ok);
    }

    /// rule: INV.RUNTIME.SERVICE-CONTROL-CAPACITY
    #[test]
    fn workspace_service_admission_ninth_control_delivers_exact_cancel_with_all_old_slots_held() {
        let mut server = WorkspaceAdmissionTestServer::start("admission-nine-controls");
        let mut target_work = server.work("control-target");
        let other_work = server.work("control-other");
        server.executor.wait_started(2);
        let target = server.token("control-target");
        let other = server.token("control-other");
        // These two real work handlers and 62 incomplete headers hold all 64
        // original general slots; no fake counter or reduced fixture capacity.
        server.hold_headers(62);
        let gate = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
        server.gates.push(gate.clone());
        let held_ports = Arc::new(Mutex::new(Vec::<u16>::new()));
        let selected = held_ports.clone();
        let (entered, observed) = mpsc::channel();
        let (cancel_entered, cancel_observed) = mpsc::channel();
        server.runtime.test_set_authenticated_request_hook(Some(Arc::new(move |port, kind| {
            if matches!(kind, ServiceRequestKind::Cancel { operation_id } if operation_id == "control-target")
            {
                let _ = cancel_entered.send(());
            }
            if matches!(kind, ServiceRequestKind::Ping) && selected.lock().unwrap().contains(&port)
            {
                let _ = entered.send(port);
                wait_admission_test_gate(&gate);
            }
        })));
        for _ in 0..8 {
            let mut stream = TcpStream::connect(("127.0.0.1", server.record.port)).unwrap();
            server.retain(&stream);
            let port = stream.local_addr().unwrap().port();
            held_ports.lock().unwrap().push(port);
            let request = ServiceRequest {
                token: server.record.token.clone(),
                kind: ServiceRequestKind::Ping,
            };
            writeln!(stream, "{}", serde_json::to_string(&request).unwrap()).unwrap();
            assert_eq!(
                observed
                    .recv_timeout(Duration::from_secs(2))
                    .expect("authenticated control handler enters pre-dispatch gate"),
                port
            );
        }
        let mut cancel_response = open_test_request(
            &server.record,
            ServiceRequestKind::Cancel {
                operation_id: "control-target".into(),
            },
        );
        cancel_observed
            .recv_timeout(Duration::from_secs(2))
            .expect("ninth real control handler must authenticate before exact cancellation");
        let cancel = read_test_response(&mut cancel_response);
        assert!(cancel.ok, "{:?}", cancel.error);
        assert!(!read_test_response(&mut target_work).ok);
        assert!(target.is_cancelled());
        assert!(
            !other.is_cancelled(),
            "ninth control must only cancel the addressed work"
        );
        assert!(send_test_request(&server.record, ServiceRequestKind::Ping).ok);
        assert!(send_test_request(&server.record, ServiceRequestKind::Shutdown).ok);
        drop(other_work);
    }

    /// rule: INV.RUNTIME.SERVICE-CONTROL-CAPACITY
    #[test]
    fn workspace_service_concurrent_work_preserves_control_path() {
        let (context, record, _runtime, executor, server) =
            workspace_control_test_server("work-saturation");
        let mut work = Vec::new();
        for index in 0..8 {
            work.push(open_test_request(
                &record,
                ServiceRequestKind::BslMcp {
                    operation_id: format!("blocked-{index}"),
                    tool_name: "diagnostics".to_string(),
                    tool_args: json!({}),
                    timeout_seconds: 5,
                    timeout_nanos: 0,
                    yields: false,
                },
            ));
        }
        executor.wait_started(8);
        let additional = send_test_request(
            &record,
            ServiceRequestKind::BslMcp {
                operation_id: "success-ninth".into(),
                tool_name: "diagnostics".to_string(),
                tool_args: json!({}),
                timeout_seconds: 5,
                timeout_nanos: 0,
                yields: false,
            },
        );
        assert!(additional.ok, "{:?}", additional.error);
        executor.wait_started(9);
        assert!(send_test_request(&record, ServiceRequestKind::Ping).ok);
        for index in 0..8 {
            assert!(
                send_test_request(
                    &record,
                    ServiceRequestKind::Cancel {
                        operation_id: format!("blocked-{index}")
                    }
                )
                .ok
            );
        }
        for reader in &mut work {
            assert!(!read_test_response(reader).ok);
        }
        assert!(send_test_request(&record, ServiceRequestKind::Shutdown).ok);
        server.join().unwrap().unwrap();
        let identity =
            WorkspaceServiceIdentity::new(&context, &context.workspace_root.join("src")).unwrap();
        assert!(!identity.test_record_path().exists());
    }

    /// rule: INV.RUNTIME.SERVICE-LIFETIME
    /// rule: INV.RUNTIME.SERVICE-SHUTDOWN
    #[test]
    fn workspace_service_control_path_shutdown_cancels_all_and_rejects_new_work() {
        let (context, record, runtime, executor, server) =
            workspace_control_test_server("control-shutdown");
        let mut first = open_test_request(
            &record,
            ServiceRequestKind::BslMcp {
                operation_id: "blocked-first".to_string(),
                tool_name: "diagnostics".to_string(),
                tool_args: json!({}),
                timeout_seconds: 5,
                timeout_nanos: 0,
                yields: false,
            },
        );
        let mut second = open_test_request(
            &record,
            ServiceRequestKind::BslMcp {
                operation_id: "blocked-second".to_string(),
                tool_name: "diagnostics".to_string(),
                tool_args: json!({}),
                timeout_seconds: 5,
                timeout_nanos: 0,
                yields: false,
            },
        );
        executor.wait_started(2);
        let shutdown = send_test_request(&record, ServiceRequestKind::Shutdown);
        assert!(shutdown.ok);
        for response in [
            read_test_response(&mut first),
            read_test_response(&mut second),
        ] {
            assert!(response.error.unwrap().starts_with("cancelled:"));
        }

        let rejected = match runtime.test_register_operation("late-work".to_string()) {
            Ok(_) => panic!("workspace service accepted work after shutdown"),
            Err(error) => error,
        };
        assert!(rejected.contains("shutting down"));
        executor.wait_cancelled(2);

        server.join().unwrap().unwrap();
        assert!(runtime.test_operation_ids().is_empty());
        cleanup(&context);
    }

    #[test]
    fn workspace_service_record_outlives_late_session_retirement() {
        struct BlockingRetiredSession {
            drop_started: mpsc::Sender<()>,
            drop_release: Arc<Mutex<mpsc::Receiver<()>>>,
        }

        impl Drop for BlockingRetiredSession {
            fn drop(&mut self) {
                self.drop_started.send(()).unwrap();
                self.drop_release.lock().unwrap().recv().unwrap();
            }
        }

        struct LateSessionRetirementExecutor {
            before_retirement: mpsc::Sender<()>,
            allow_retirement: Mutex<mpsc::Receiver<()>>,
            drop_started: mpsc::Sender<()>,
            drop_release: Arc<Mutex<mpsc::Receiver<()>>>,
        }

        impl GenericWorkspaceServiceOperationExecutor<BslWorkspaceBackend>
            for LateSessionRetirementExecutor
        {
            fn execute(
                &self,
                runtime: &WorkspaceServiceRuntime,
                _kind: ServiceRequestKind,
                _cancellation: &CancellationToken,
            ) -> ServiceResponse {
                self.before_retirement.send(()).unwrap();
                self.allow_retirement.lock().unwrap().recv().unwrap();
                runtime.test_retire_session(BlockingRetiredSession {
                    drop_started: self.drop_started.clone(),
                    drop_release: Arc::clone(&self.drop_release),
                });
                ServiceResponse {
                    ok: true,
                    status: Some("late-session-retired".to_string()),
                    ..ServiceResponse::default()
                }
            }
        }

        let (before_retirement_tx, before_retirement_rx) = mpsc::channel();
        let (allow_retirement_tx, allow_retirement_rx) = mpsc::channel();
        let (drop_started_tx, drop_started_rx) = mpsc::channel();
        let (drop_release_tx, drop_release_rx) = mpsc::channel();
        let executor = Arc::new(LateSessionRetirementExecutor {
            before_retirement: before_retirement_tx,
            allow_retirement: Mutex::new(allow_retirement_rx),
            drop_started: drop_started_tx,
            drop_release: Arc::new(Mutex::new(drop_release_rx)),
        });
        let (context, record, runtime, server) = workspace_test_server_with_executor(
            "late-session-record",
            executor,
            SERVICE_SHUTDOWN_GRACE,
        );
        let (providers_drained_tx, providers_drained_rx) = mpsc::channel();
        *runtime
            .test_provider_sessions_drained_hook()
            .lock()
            .unwrap() = Some(Arc::new(move || providers_drained_tx.send(()).unwrap()));
        let mut request = open_test_request(
            &record,
            ServiceRequestKind::BslMcp {
                operation_id: "late-session".to_string(),
                tool_name: "diagnostics".to_string(),
                tool_args: json!({}),
                timeout_seconds: 5,
                timeout_nanos: 0,
                yields: false,
            },
        );
        before_retirement_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap();

        assert!(send_test_request(&record, ServiceRequestKind::Shutdown).ok);
        providers_drained_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap();
        let record_path =
            WorkspaceServiceIdentity::new(&context, &context.workspace_root.join("src"))
                .unwrap()
                .test_record_path();
        let (server_finished_tx, server_finished_rx) = mpsc::channel();
        let joiner = thread::spawn(move || {
            server_finished_tx.send(server.join().unwrap()).unwrap();
        });
        allow_retirement_tx.send(()).unwrap();
        drop_started_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap();

        assert!(matches!(
            server_finished_rx.recv_timeout(Duration::from_millis(500)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        assert!(record_path.exists());

        drop_release_tx.send(()).unwrap();
        assert!(read_test_response(&mut request).ok);
        server_finished_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .unwrap();
        joiner.join().unwrap();
        assert!(!record_path.exists());
        cleanup(&context);
    }

    /// rule: INV.RUNTIME.SERVICE-OPERATION-CANCELLATION
    #[test]
    fn workspace_service_detached_caller_keeps_work_and_exact_cancel_ownership() {
        let mut server = WorkspaceAdmissionTestServer::start("detached-exact-control");
        let disconnected = server.work("held-disconnected");
        let mut other = server.work("held-other");
        server.executor.wait_started(2);
        let target_token = server.token("held-disconnected");
        let other_token = server.token("held-other");
        let (detached_tx, detached_rx) = mpsc::channel();
        server
            .runtime
            .test_set_caller_detached_hook(Some(Arc::new(move |token| {
                let _ = detached_tx.send(token.is_cancelled());
            })));
        disconnected
            .get_ref()
            .shutdown(std::net::Shutdown::Both)
            .unwrap();
        drop(disconnected);
        assert!(
            !detached_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            "the real EOF monitor must detach without cancelling accepted work"
        );
        assert!(!target_token.is_cancelled());
        assert!(!other_token.is_cancelled());
        assert!(server
            .runtime
            .test_operation_ids()
            .iter()
            .any(|id| id == "held-disconnected"));
        assert!(send_test_request(&server.record, ServiceRequestKind::Ping).ok);
        assert!(
            send_test_request(
                &server.record,
                ServiceRequestKind::Cancel {
                    operation_id: "held-disconnected".into()
                }
            )
            .ok
        );
        assert!(target_token.is_cancelled());
        assert!(!other_token.is_cancelled());
        assert!(send_test_request(&server.record, ServiceRequestKind::Shutdown).ok);
        assert!(!read_test_response(&mut other).ok);
        assert!(other_token.is_cancelled());
    }

    /// rule: INV.RUNTIME.SERVICE-OPERATION-CANCELLATION
    #[test]
    fn workspace_service_half_closed_caller_receives_late_success_without_cancel() {
        assert_workspace_caller_completion(None);
    }

    /// rule: INV.RUNTIME.SERVICE-OPERATION-CANCELLATION
    #[test]
    fn workspace_service_monitor_failures_preserve_worker_and_report_real_error_after_completion() {
        for point in [
            WorkspaceCallerMonitorPoint::Setup,
            WorkspaceCallerMonitorPoint::Peek,
        ] {
            assert_workspace_caller_completion(Some(point));
        }
    }

    fn assert_workspace_caller_completion(fault: Option<WorkspaceCallerMonitorPoint>) {
        struct FinishingExecutor {
            started: mpsc::Sender<()>,
            release: Mutex<mpsc::Receiver<()>>,
        }
        impl GenericWorkspaceServiceOperationExecutor<BslWorkspaceBackend> for FinishingExecutor {
            fn execute(
                &self,
                _runtime: &WorkspaceServiceRuntime,
                _kind: ServiceRequestKind,
                cancellation: &CancellationToken,
            ) -> ServiceResponse {
                self.started.send(()).unwrap();
                self.release.lock().unwrap().recv().unwrap();
                if cancellation.is_cancelled() {
                    unica_analysis_services::test_support::error_response(cancelled_error(
                        "half-closed work cancelled",
                    ))
                } else {
                    ServiceResponse {
                        ok: true,
                        status: Some("late-complete-result".into()),
                        ..ServiceResponse::default()
                    }
                }
            }
        }
        struct Cleanup {
            context: WorkspaceContext,
            runtime: Arc<WorkspaceServiceRuntime>,
            server: Option<thread::JoinHandle<Result<(), String>>>,
            release: mpsc::Sender<()>,
        }
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = self.release.send(());
                self.runtime.test_begin_shutdown();
                if let Some(server) = self.server.take() {
                    let _ = server.join();
                }
                cleanup(&self.context);
            }
        }
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let executor = Arc::new(FinishingExecutor {
            started: started_tx,
            release: Mutex::new(release_rx),
        });
        let (context, record, runtime, server) = workspace_test_server_with_executor(
            "half-closed-late-result",
            executor,
            SERVICE_SHUTDOWN_GRACE,
        );
        let fixture = Cleanup {
            context,
            runtime,
            server: Some(server),
            release: release_tx.clone(),
        };
        fixture.runtime.test_set_caller_monitor_fault(fault);
        let (finished_tx, finished_rx) = mpsc::channel();
        fixture
            .runtime
            .test_set_handler_finished_hook(Some(Arc::new(move |result| {
                let error = result
                    .as_ref()
                    .expect("actual handler must not panic")
                    .as_ref()
                    .err()
                    .cloned();
                let _ = finished_tx.send(error);
            })));
        let (detached_tx, detached_rx) = mpsc::channel();
        fixture
            .runtime
            .test_set_caller_detached_hook(Some(Arc::new(move |token| {
                let _ = detached_tx.send(token.is_cancelled());
                let _ = release_tx.send(());
            })));
        let mut request = open_test_request(&record, admission_test_work("half-closed-work"));
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        request
            .get_ref()
            .shutdown(std::net::Shutdown::Write)
            .unwrap();
        let observed_cancel = detached_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(!observed_cancel, "inbound FIN must preserve accepted work");
        let response = read_test_response(&mut request);
        assert!(response.ok, "{response:?}");
        assert_eq!(response.status.as_deref(), Some("late-complete-result"));
        assert!(fixture.runtime.test_operation_ids().is_empty());
        let handler_error = finished_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        match fault {
            None => assert!(handler_error.is_none(), "{handler_error:?}"),
            Some(WorkspaceCallerMonitorPoint::Setup) => assert!(handler_error
                .unwrap()
                .contains("injected caller monitor setup failure")),
            Some(WorkspaceCallerMonitorPoint::Peek) => assert!(handler_error
                .unwrap()
                .contains("injected caller monitor peek failure")),
        }
    }

    #[test]
    fn workspace_service_control_path_rejects_duplicate_active_operation_id() {
        let (context, record, _runtime, executor, server) =
            workspace_control_test_server("control-duplicate");
        let mut first = open_test_request(
            &record,
            ServiceRequestKind::BslMcp {
                operation_id: "duplicate-id".to_string(),
                tool_name: "diagnostics".to_string(),
                tool_args: json!({}),
                timeout_seconds: 5,
                timeout_nanos: 0,
                yields: false,
            },
        );
        executor.wait_started(1);

        let duplicate = send_test_request(
            &record,
            ServiceRequestKind::BslMcp {
                operation_id: "duplicate-id".to_string(),
                tool_name: "diagnostics".to_string(),
                tool_args: json!({}),
                timeout_seconds: 5,
                timeout_nanos: 0,
                yields: false,
            },
        );
        assert!(!duplicate.ok);
        assert!(duplicate.error.unwrap().contains("already active"));

        assert!(
            send_test_request(
                &record,
                ServiceRequestKind::Cancel {
                    operation_id: "duplicate-id".to_string(),
                },
            )
            .ok
        );
        assert!(read_test_response(&mut first)
            .error
            .unwrap()
            .starts_with("cancelled:"));
        assert!(send_test_request(&record, ServiceRequestKind::Shutdown).ok);
        server.join().unwrap().unwrap();
        cleanup(&context);
    }

    /// rule: INV.RUNTIME.SERVICE-OPERATION-CANCELLATION
    #[test]
    fn workspace_service_control_path_drains_disconnected_worker_before_cleanup() {
        let (context, record, runtime, executor, server) =
            workspace_control_test_server("control-disconnected-drain");
        let disconnected = open_test_request(
            &record,
            ServiceRequestKind::BslMcp {
                operation_id: "held-after-cancel-1".to_string(),
                tool_name: "diagnostics".to_string(),
                tool_args: json!({}),
                timeout_seconds: 5,
                timeout_nanos: 0,
                yields: false,
            },
        );
        executor.wait_started(1);
        drop(disconnected);
        assert!(send_test_request(&record, ServiceRequestKind::Shutdown).ok);
        executor.wait_cancelled(1);

        let (joined_tx, joined_rx) = mpsc::channel();
        let joiner = thread::spawn(move || {
            let result = server.join().unwrap();
            let _ = joined_tx.send(result);
        });
        assert!(joined_rx.recv_timeout(Duration::from_millis(150)).is_err());
        executor.release_cancelled();
        joined_rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap();
        joiner.join().unwrap();

        assert!(runtime.test_operation_ids().is_empty());
        cleanup(&context);
    }

    #[test]
    fn workspace_service_control_path_cancels_analyzer_waiter_and_shutdown_releases_lane() {
        let executor = Arc::new(AnalyzerLaneExecutor::default());
        let (context, record, runtime, server) = workspace_test_server_with_executor(
            "analyzer-lane-cancel",
            executor.clone(),
            Duration::from_secs(2),
        );
        let mut holder = open_test_request(
            &record,
            ServiceRequestKind::BslMcp {
                operation_id: "analyzer-holder-1".to_string(),
                tool_name: "hold".to_string(),
                tool_args: json!({}),
                timeout_seconds: 30,
                timeout_nanos: 0,
                yields: false,
            },
        );
        executor.wait_holder();
        let mut waiter = open_test_request(
            &record,
            ServiceRequestKind::BslMcp {
                operation_id: "analyzer-waiter-2".to_string(),
                tool_name: "wait".to_string(),
                tool_args: json!({}),
                timeout_seconds: 30,
                timeout_nanos: 0,
                yields: false,
            },
        );
        let queued_deadline = Instant::now() + Duration::from_secs(2);
        while runtime.test_analyzer_lane().test_waiters_is_empty() {
            assert!(
                Instant::now() < queued_deadline,
                "analyzer waiter did not enter the lane queue"
            );
            thread::yield_now();
        }
        let cancel_started = Instant::now();
        assert!(
            send_test_request(
                &record,
                ServiceRequestKind::Cancel {
                    operation_id: "analyzer-waiter-2".to_string(),
                },
            )
            .ok
        );
        let cancelled_waiter = read_test_response(&mut waiter);
        assert!(cancelled_waiter.error.unwrap().starts_with("cancelled:"));
        assert!(cancel_started.elapsed() < Duration::from_millis(500));

        assert!(send_test_request(&record, ServiceRequestKind::Shutdown).ok);
        assert!(read_test_response(&mut holder)
            .error
            .unwrap()
            .starts_with("cancelled:"));
        server.join().unwrap().unwrap();
        assert!(runtime.test_operation_ids().is_empty());
        cleanup(&context);
    }

    #[test]
    fn bsl_operation_times_out_while_analyzer_lane_remains_held() {
        let context = test_context("bsl-lane-deadline");
        let source_root = context.workspace_root.join("src");
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        let record = test_record(&identity, 34567, CoreIdentity::production().as_str());
        let runtime = Arc::new(new_runtime(identity, &record));
        let holder = runtime
            .test_analyzer_lane()
            .acquire(&CancellationToken::new())
            .unwrap();
        let waiter_runtime = Arc::clone(&runtime);
        let (response_tx, response_rx) = mpsc::channel();
        let waiter = thread::spawn(move || {
            response_tx
                .send(waiter_runtime.test_handle_bsl_mcp(
                    "diagnostics_status",
                    json!({}),
                    0,
                    50_000_000,
                    &CancellationToken::new(),
                ))
                .unwrap();
        });

        let response = match response_rx.recv_timeout(Duration::from_secs(2)) {
            Ok(response) => response,
            Err(error) => {
                drop(holder);
                let late_response = response_rx.recv_timeout(Duration::from_secs(2)).ok();
                waiter.join().unwrap();
                drop(runtime);
                cleanup(&context);
                panic!("analyzer lane wait exceeded its budget: {error}; {late_response:?}");
            }
        };

        assert!(
            response
                .error
                .as_deref()
                .is_some_and(|error| error.starts_with("timeout:")),
            "{response:?}"
        );
        drop(holder);
        waiter.join().unwrap();
        drop(runtime);
        cleanup(&context);
    }

    #[test]
    fn runtime_tracks_session_teardown_until_shutdown_drain() {
        struct DropSignal(Option<mpsc::Sender<()>>);
        impl Drop for DropSignal {
            fn drop(&mut self) {
                self.0.take().unwrap().send(()).unwrap();
            }
        }

        let context = test_context("tracked-session-teardown");
        let identity =
            WorkspaceServiceIdentity::new(&context, &context.workspace_root.join("src")).unwrap();
        let record = test_record(&identity, 34567, CoreIdentity::production().as_str());
        let runtime = new_runtime(identity, &record);
        let (dropped_tx, dropped_rx) = mpsc::channel();

        runtime.test_retire_session(DropSignal(Some(dropped_tx)));

        assert!(runtime.test_shutdown_provider_sessions());
        dropped_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        cleanup(&context);
    }

    #[test]
    fn workspace_service_worker_panic_cleans_registry_and_next_request_succeeds() {
        let (context, record, runtime, _executor, server) =
            workspace_control_test_server("worker-panic");
        let panic_response = send_test_request(
            &record,
            ServiceRequestKind::BslMcp {
                operation_id: "panic-worker-1".to_string(),
                tool_name: "diagnostics".to_string(),
                tool_args: json!({}),
                timeout_seconds: 5,
                timeout_nanos: 0,
                yields: false,
            },
        );
        assert!(!panic_response.ok);
        assert!(panic_response
            .error
            .unwrap()
            .contains("worker disconnected"));
        assert!(runtime.test_operation_ids().is_empty());
        assert!(
            send_test_request(
                &record,
                ServiceRequestKind::BslMcp {
                    operation_id: "success-after-panic".to_string(),
                    tool_name: "diagnostics".to_string(),
                    tool_args: json!({}),
                    timeout_seconds: 5,
                    timeout_nanos: 0,
                    yields: false
                },
            )
            .ok
        );
        assert!(send_test_request(&record, ServiceRequestKind::Shutdown).ok);
        server.join().unwrap().unwrap();
        cleanup(&context);
    }

    /// rule: INV.RUNTIME.SERVICE-RECORD-OWNERSHIP
    #[test]
    fn workspace_service_cleanup_preserves_replacement_record() {
        let context = test_context("cleanup-owner-race");
        let identity =
            WorkspaceServiceIdentity::new(&context, &context.workspace_root.join("src")).unwrap();
        let original = test_record(&identity, 31001, CoreIdentity::production().as_str());
        write_record(&identity, original.clone());
        let runtime = new_runtime(identity.clone(), &original);
        let mut replacement = original;
        replacement.token = "replacement-token".to_string();
        replacement.pid = replacement.pid.saturating_add(1);
        replacement.started_at = replacement.started_at.saturating_add(1);
        write_record(&identity, replacement.clone());

        remove_service_record_if_owned(&runtime);

        assert_eq!(read_record(&identity).unwrap().token, replacement.token);
        cleanup(&context);
    }

    /// rule: INV.RUNTIME.SERVICE-RECORD-OWNERSHIP
    #[test]
    fn workspace_service_record_cleanup_serializes_concurrent_replacement() {
        let context = test_context("cleanup-serialized-race");
        let identity =
            WorkspaceServiceIdentity::new(&context, &context.workspace_root.join("src")).unwrap();
        let original = test_record(&identity, 31002, CoreIdentity::production().as_str());
        write_record(&identity, original.clone());
        let runtime = Arc::new(new_runtime(identity.clone(), &original));
        let mut replacement = original;
        replacement.token = "serialized-replacement".to_string();
        replacement.started_at = replacement.started_at.saturating_add(1);
        let (inside_tx, inside_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let cleanup_runtime = runtime.clone();
        let cleanup_thread = thread::spawn(move || {
            remove_service_record_if_owned_with_hook(&cleanup_runtime, || {
                inside_tx.send(()).unwrap();
                release_rx.recv().unwrap();
            });
        });
        inside_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let replacement_identity = identity.clone();
        let expected = replacement.clone();
        let (writer_inside_tx, writer_inside_rx) = mpsc::channel();
        let writer = thread::spawn(move || {
            write_record_with_hook(&replacement_identity, &replacement, || {
                writer_inside_tx.send(()).unwrap();
            })
            .unwrap();
        });
        assert!(writer_inside_rx
            .recv_timeout(Duration::from_millis(100))
            .is_err());
        release_tx.send(()).unwrap();
        cleanup_thread.join().unwrap();
        writer_inside_rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
        writer.join().unwrap();

        assert_eq!(read_record(&identity).unwrap(), expected);
        assert!(identity
            .service_dir
            .join(SERVICE_RECORD_LOCK_FILE)
            .is_file());
        cleanup(&context);
    }

    #[test]
    fn workspace_service_record_lock_survives_holder_panic_and_is_reusable() {
        let context = test_context("record-lock-panic");
        let identity =
            WorkspaceServiceIdentity::new(&context, &context.workspace_root.join("src")).unwrap();
        let panic_identity = identity.clone();
        let _ = thread::spawn(move || {
            let _lock = acquire_record_lock(&panic_identity).unwrap();
            panic!("intentional record lock holder panic");
        })
        .join();
        let record = test_record(&identity, 31004, CoreIdentity::production().as_str());

        super::write_record(&identity, &record).unwrap();

        assert_eq!(read_record(&identity).unwrap(), record);
        assert!(identity
            .service_dir
            .join(SERVICE_RECORD_LOCK_FILE)
            .is_file());
        cleanup(&context);
    }

    /// rule: INV.RUNTIME.SERVICE-RECORD-OWNERSHIP
    #[test]
    fn workspace_service_last_access_update_cannot_overwrite_replacement() {
        let context = test_context("last-access-serialized-race");
        let identity =
            WorkspaceServiceIdentity::new(&context, &context.workspace_root.join("src")).unwrap();
        let original = test_record(&identity, 31003, CoreIdentity::production().as_str());
        write_record(&identity, original.clone());
        let runtime = Arc::new(new_runtime(identity.clone(), &original));
        let mut replacement = original;
        replacement.token = "last-access-replacement".to_string();
        replacement.started_at = replacement.started_at.saturating_add(1);
        let expected = replacement.clone();
        let (inside_tx, inside_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let update_runtime = runtime.clone();
        let updater = thread::spawn(move || {
            update_service_record_last_access_with_hook(&update_runtime, 999, || {
                inside_tx.send(()).unwrap();
                release_rx.recv().unwrap();
            });
        });
        inside_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let replacement_identity = identity.clone();
        let (writer_inside_tx, writer_inside_rx) = mpsc::channel();
        let writer = thread::spawn(move || {
            write_record_with_hook(&replacement_identity, &replacement, || {
                writer_inside_tx.send(()).unwrap();
            })
            .unwrap();
        });
        assert!(writer_inside_rx
            .recv_timeout(Duration::from_millis(100))
            .is_err());
        release_tx.send(()).unwrap();
        updater.join().unwrap();
        writer_inside_rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
        writer.join().unwrap();

        assert_eq!(read_record(&identity).unwrap(), expected);
        cleanup(&context);
    }

    #[test]
    fn analyzer_lane_is_fifo_and_cancelled_middle_ticket_advances_queue() {
        let lane = Arc::new(AnalyzerLane::default());
        let holder = lane.acquire(&CancellationToken::new()).unwrap();
        let (queued_tx, queued_rx) = mpsc::channel();
        let (acquired_tx, acquired_rx) = mpsc::channel();
        let middle_cancellation = CancellationToken::new();
        let mut waiters = Vec::new();
        for id in [1_u8, 2, 3] {
            let lane = lane.clone();
            let queued_tx = queued_tx.clone();
            let acquired_tx = acquired_tx.clone();
            let cancellation = if id == 2 {
                middle_cancellation.clone()
            } else {
                CancellationToken::new()
            };
            waiters.push(thread::spawn(move || {
                let result = lane.acquire_with_hook(&cancellation, || {
                    queued_tx.send(id).unwrap();
                });
                match result {
                    Ok(_permit) => acquired_tx.send((id, "acquired")).unwrap(),
                    Err(error) => {
                        assert!(error.starts_with("cancelled:"));
                        acquired_tx.send((id, "cancelled")).unwrap();
                    }
                }
            }));
            assert_eq!(queued_rx.recv_timeout(Duration::from_secs(2)).unwrap(), id);
        }
        middle_cancellation.cancel();
        assert_eq!(
            acquired_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            (2, "cancelled")
        );
        drop(holder);
        assert_eq!(
            acquired_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            (1, "acquired")
        );
        assert_eq!(
            acquired_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            (3, "acquired")
        );
        for waiter in waiters {
            waiter.join().unwrap();
        }
    }

    #[test]
    fn analyzer_lane_recovers_from_poison_without_losing_progress() {
        let lane = Arc::new(AnalyzerLane::default());
        let poison_lane = lane.clone();
        let _ = thread::spawn(move || {
            poison_lane.test_poison_state();
        })
        .join();

        let permit = lane.acquire(&CancellationToken::new()).unwrap();
        drop(permit);
        let second = lane.acquire(&CancellationToken::new()).unwrap();
        drop(second);
    }

    #[test]
    fn workspace_service_zero_grace_cutoff_preserves_new_owner_record() {
        let executor = Arc::new(BlockingWorkspaceExecutor::default());
        let (context, record, runtime, server) = workspace_test_server_with_executor(
            "zero-grace-owner",
            executor.clone(),
            Duration::ZERO,
        );
        let disconnected = open_test_request(
            &record,
            ServiceRequestKind::BslMcp {
                operation_id: "held-after-cancel-zero-grace".to_string(),
                tool_name: "diagnostics".to_string(),
                tool_args: json!({}),
                timeout_seconds: 5,
                timeout_nanos: 0,
                yields: false,
            },
        );
        executor.wait_started(1);
        drop(disconnected);
        let identity = runtime.test_identity().clone();
        let mut replacement = record.clone();
        replacement.token = "new-owner".to_string();
        replacement.pid = replacement.pid.saturating_add(1);
        replacement.started_at = replacement.started_at.saturating_add(1);
        write_record(&identity, replacement.clone());
        assert!(send_test_request(&record, ServiceRequestKind::Shutdown).ok);
        executor.wait_cancelled(1);

        let started = Instant::now();
        server.join().unwrap().unwrap();
        assert!(started.elapsed() < Duration::from_millis(500));
        assert_eq!(read_record(&identity).unwrap().token, replacement.token);
        executor.release_cancelled();
        let retired_by = Instant::now() + Duration::from_secs(2);
        while !runtime.test_operation_ids().is_empty() {
            assert!(
                Instant::now() < retired_by,
                "cancelled worker did not retire its guard"
            );
            thread::sleep(Duration::from_millis(5));
        }
        cleanup(&context);
    }

    #[test]
    fn bsl_session_initialize_uses_operation_cancellation_for_stuck_and_fragmented_output() {
        let context = test_context("cancelled-initialize");
        let fixture = compile_initialize_fixture(&context);
        for mode in ["stuck", "fragmented"] {
            let mut command = Command::new(&fixture);
            command.arg(mode);
            let cancellation = CancellationToken::new();
            let cancel = cancellation.clone();
            let canceller = thread::spawn(move || {
                thread::sleep(Duration::from_millis(100));
                cancel.cancel();
            });
            let started = Instant::now();
            let error = match PersistentMcpSession::start_with_command(command, &cancellation) {
                Ok(_) => panic!("initialize unexpectedly completed"),
                Err(error) => error,
            };
            canceller.join().unwrap();
            assert!(error.starts_with("cancelled:"), "{error}");
            assert!(started.elapsed() < Duration::from_secs(2));
        }
        let pid_file = context.cache_root.join("incomplete-eof-child-pid.txt");
        let read_marker = context
            .cache_root
            .join("incomplete-eof-initialize-read.txt");
        let mut command = Command::new(&fixture);
        command.args([
            "incomplete-eof",
            pid_file.to_str().unwrap(),
            read_marker.to_str().unwrap(),
        ]);
        let started = Instant::now();
        let error = PersistentMcpSession::start_with_command(command, &CancellationToken::new())
            .err()
            .expect("incomplete initialize response must fail");
        let process_pid = wait_for_recorded_pid(&pid_file);
        assert_eq!(fs::read_to_string(&read_marker).unwrap(), "initialize-read");
        assert!(error.contains("incomplete JSON line"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(wait_for_process_exit(process_pid, Duration::from_secs(2)));
        cleanup(&context);
    }

    #[test]
    fn cancelled_bsl_session_terminates_process_tree_without_blocking_drop() {
        let context = test_context("cancelled-session-tree");
        let fixture = compile_session_tree_fixture(&context);
        let pid_file = context.cache_root.join("session-tree-pids.txt");
        let mut command = Command::new(&fixture);
        command.arg(&pid_file);
        let cancellation = CancellationToken::new();
        let mut session = PersistentMcpSession::start_with_command(command, &cancellation).unwrap();
        let parent_pid = session.test_child_pid();
        let child_pid = wait_for_recorded_pid(&pid_file);
        let cancel = cancellation.clone();
        let canceller = thread::spawn(move || {
            thread::sleep(Duration::from_millis(100));
            cancel.cancel();
        });

        let error = session
            .call(
                "unica.code.search",
                json!({"query": "x".repeat(16 * 1024 * 1024)}),
                Duration::from_secs(30),
                &cancellation,
            )
            .unwrap_err();
        assert!(error.starts_with("cancelled:"), "{error}");
        let drop_started = Instant::now();
        drop(session);
        canceller.join().unwrap();

        assert!(drop_started.elapsed() < Duration::from_secs(2));
        assert!(wait_for_process_exit(parent_pid, Duration::from_secs(2)));
        assert!(wait_for_process_exit(child_pid, Duration::from_secs(2)));
        cleanup(&context);
    }

    fn stalling_runtime(
        name: &str,
    ) -> (WorkspaceContext, WorkspaceServiceRuntime, PathBuf, PathBuf) {
        let context = test_context(&format!("{name}-{}", Uuid::new_v4()));
        let fixture = compile_stall_first_call_fixture(&context);
        let pid_file = context.cache_root.join("stall-session-pids.txt");
        let log_file = context.cache_root.join("stall-session-log.txt");
        let source_root = context.workspace_root.join("src");
        fs::create_dir_all(&source_root).unwrap();
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        let record = test_record(&identity, 1, CoreIdentity::production().as_str());
        let mut runtime = new_runtime(identity, &record);
        let (pids, log) = (pid_file.clone(), log_file.clone());
        runtime.test_set_analyzer_starter(Arc::new(move |_context, _source_root, cancellation| {
            let mut command = Command::new(&fixture);
            command.args([&pids, &log]);
            PersistentMcpSession::start_with_command(command, cancellation)
        }));
        (context, runtime, pid_file, log_file)
    }

    /// #2189: a call out of time is cancelled at the analyzer, and the next
    /// call is answered by the same warm process.
    /// rule: INV.LSP.MCP-PRIORITY
    #[test]
    fn an_expired_analyzer_call_keeps_the_warm_process_for_the_next_one() {
        let (context, runtime, pid_file, log_file) = stalling_runtime("expired-call");
        let expired = runtime.test_handle_yielding_bsl_mcp(
            "symbol_info",
            json!({}),
            Duration::from_millis(500),
            &CancellationToken::new(),
        );
        assert!(!expired.ok, "{expired:?}");
        let next = runtime.test_handle_bsl_mcp(
            "unica.code.search",
            json!({}),
            SERVICE_REQUEST_TIMEOUT.as_secs(),
            0,
            &CancellationToken::new(),
        );
        assert!(next.ok, "{:?}", next.error);
        assert_eq!(fs::read_to_string(&pid_file).unwrap().lines().count(), 1);
        assert_eq!(
            fs::read_to_string(&log_file).unwrap(),
            "holding\ncancelled\n"
        );
        cleanup(&context);
    }

    /// #2189, mcp-priority: an editor's call holding the analyzer gives way to
    /// an MCP call, which is answered without waiting for it to finish.
    /// rule: INV.LSP.MCP-PRIORITY
    #[test]
    fn an_mcp_call_cancels_a_yielding_call_that_holds_the_analyzer() {
        let (context, runtime, pid_file, log_file) = stalling_runtime("yielding-call");
        thread::scope(|scope| {
            let editor = scope.spawn(|| {
                runtime.test_handle_yielding_bsl_mcp(
                    "symbol_info",
                    json!({}),
                    SERVICE_REQUEST_TIMEOUT,
                    &CancellationToken::new(),
                )
            });
            let deadline = Instant::now() + Duration::from_secs(10);
            while !fs::read_to_string(&log_file).is_ok_and(|log| log.contains("holding")) {
                assert!(
                    Instant::now() < deadline,
                    "the editor call never reached the analyzer"
                );
                thread::sleep(Duration::from_millis(10));
            }
            let started = Instant::now();
            let agent = runtime.test_handle_bsl_mcp(
                "unica.code.search",
                json!({}),
                SERVICE_REQUEST_TIMEOUT.as_secs(),
                0,
                &CancellationToken::new(),
            );
            assert!(agent.ok, "{:?}", agent.error);
            assert!(started.elapsed() < Duration::from_secs(5));
            let editor = editor.join().unwrap();
            assert!(!editor.ok);
            assert!(
                editor.error.as_deref().is_some_and(
                    |error| error.starts_with("cancelled:") && error.contains("gave way")
                ),
                "{editor:?}"
            );
        });
        assert_eq!(fs::read_to_string(&pid_file).unwrap().lines().count(), 1);
        assert_eq!(
            fs::read_to_string(&log_file).unwrap(),
            "holding\ncancelled\n"
        );
        cleanup(&context);
    }

    #[test]
    fn bsl_session_replaces_dead_warm_process_before_next_request() {
        let context = test_context("dead-warm-session");
        let fixture = compile_exit_after_call_fixture(&context);
        let pid_file = context.cache_root.join("warm-session-pids.txt");
        let completion_file = context.cache_root.join("warm-session-completed.txt");
        let source_root = context.workspace_root.join("src");
        fs::create_dir_all(&source_root).unwrap();
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        let record = test_record(&identity, 1, CoreIdentity::production().as_str());
        let mut runtime = new_runtime(identity, &record);
        let fixture_for_start = fixture.clone();
        let pid_file_for_start = pid_file.clone();
        let completion_file_for_start = completion_file.clone();
        runtime.test_set_analyzer_starter(Arc::new(move |_context, _source_root, cancellation| {
            let mut command = Command::new(&fixture_for_start);
            command.args([&pid_file_for_start, &completion_file_for_start]);
            PersistentMcpSession::start_with_command(command, cancellation)
        }));
        // Each runtime request includes cold startup and initialize. The two-second
        // checks below cover completion, reader EOF and old-process termination.
        let first = runtime.test_handle_bsl_mcp(
            "unica.code.search",
            json!({}),
            SERVICE_REQUEST_TIMEOUT.as_secs(),
            0,
            &CancellationToken::new(),
        );
        assert!(first.ok, "{:?}", first.error);
        let first_pid = wait_for_recorded_pid(&pid_file);
        wait_for_file(&completion_file, Duration::from_secs(2));
        wait_for_runtime_reader_terminal(&runtime, Duration::from_secs(2));

        let second = runtime.test_handle_bsl_mcp(
            "unica.code.search",
            json!({}),
            SERVICE_REQUEST_TIMEOUT.as_secs(),
            0,
            &CancellationToken::new(),
        );
        assert!(second.ok, "{:?}", second.error);
        assert!(wait_for_process_exit(first_pid, Duration::from_secs(2)));
        let pids = fs::read_to_string(&pid_file).unwrap();
        let pids = pids.lines().collect::<Vec<_>>();
        assert_eq!(pids.len(), 2, "{pids:?}");
        assert_ne!(pids[0], pids[1]);
        cleanup(&context);
    }

    #[test]
    fn bsl_provider_discards_output_after_mid_call_source_root_replacement() {
        let context = test_context("bsl-provider-root-replacement");
        let fixture = compile_exit_after_call_fixture(&context);
        let pid_file = context.cache_root.join("root-swap-session-pids.txt");
        let completion_file = context.cache_root.join("root-swap-session-completed.txt");
        let source_root = context.workspace_root.join("src");
        fs::write(
            source_root.join("Module.bsl"),
            "Процедура Тест()\nКонецПроцедуры\n",
        )
        .unwrap();
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        let record = test_record(&identity, 1, CoreIdentity::production().as_str());
        let mut runtime = new_runtime(identity, &record);
        let displaced = context.workspace_root.join("src-displaced");
        let replacement = fs::canonicalize(&source_root).unwrap();
        runtime.test_set_analyzer_starter(Arc::new(move |_context, root, cancellation| {
            fs::rename(root, &displaced).unwrap();
            fs::create_dir_all(&replacement).unwrap();
            let mut command = Command::new(&fixture);
            command.args([&pid_file, &completion_file]);
            PersistentMcpSession::start_with_command(command, cancellation)
        }));

        let response = runtime.test_handle_bsl_mcp(
            "unica.code.search",
            json!({}),
            2,
            0,
            &CancellationToken::new(),
        );

        assert!(!response.ok, "{response:?}");
        assert!(response.result_text.is_none(), "{response:?}");
        assert!(
            response.error.as_deref().is_some_and(|error| error
                .contains("workspace actor source-set physical identity changed after admission")),
            "{response:?}"
        );
        cleanup(&context);
    }

    #[test]
    fn bsl_provider_root_replacement_masks_error_diagnostics_from_the_replacement() {
        let context = test_context("bsl-provider-root-replacement-error");
        let source_root = context.workspace_root.join("src");
        fs::write(
            source_root.join("Module.bsl"),
            "Процедура Тест()\nКонецПроцедуры\n",
        )
        .unwrap();
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        let record = test_record(&identity, 1, CoreIdentity::production().as_str());
        let mut runtime = new_runtime(identity, &record);
        let displaced = context.workspace_root.join("src-displaced");
        let replacement = fs::canonicalize(&source_root).unwrap();
        runtime.test_set_analyzer_starter(Arc::new(
            move |_context, root, _cancellation| -> Result<PersistentMcpSession, String> {
                fs::rename(root, &displaced).unwrap();
                fs::create_dir_all(&replacement).unwrap();
                Err("timeout: FOREIGN-REPLACEMENT-DIAGNOSTICS".to_string())
            },
        ));

        let response = runtime.test_handle_bsl_mcp(
            "unica.code.search",
            json!({}),
            2,
            0,
            &CancellationToken::new(),
        );

        let error = response.error.as_deref().unwrap_or_default();
        assert!(!response.ok, "{response:?}");
        assert!(error.contains("physical identity changed"), "{response:?}");
        assert!(!error.contains("FOREIGN-REPLACEMENT"), "{response:?}");
        cleanup(&context);
    }

    #[test]
    fn bsl_session_replaces_reader_terminal_session_before_next_request() {
        let context = test_context(&format!("terminal-warm-session-{}", Uuid::new_v4()));
        let fixture = compile_terminal_after_call_fixture(&context);
        let pid_file = context.cache_root.join("terminal-session-pids.txt");
        let terminal_marker = context.cache_root.join("terminal-session-marker.txt");
        let source_root = context.workspace_root.join("src");
        fs::create_dir_all(&source_root).unwrap();
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        let record = test_record(&identity, 1, CoreIdentity::production().as_str());
        let mut runtime = new_runtime(identity, &record);
        runtime.test_set_analyzer_starter(Arc::new({
            let fixture = fixture.clone();
            let pid_file = pid_file.clone();
            let terminal_marker = terminal_marker.clone();
            move |_context, _source_root, cancellation| {
                let mut command = Command::new(&fixture);
                command.args([&pid_file, &terminal_marker]);
                PersistentMcpSession::start_with_command(command, cancellation)
            }
        }));

        let first = runtime.test_handle_bsl_mcp(
            "unica.code.search",
            json!({}),
            2,
            0,
            &CancellationToken::new(),
        );
        assert!(first.ok, "{:?}", first.error);
        let first_pid = wait_for_recorded_pid(&pid_file);
        wait_for_file(&terminal_marker, Duration::from_secs(5));
        wait_for_runtime_reader_terminal(&runtime, Duration::from_secs(5));

        let second = runtime.test_handle_bsl_mcp(
            "unica.code.search",
            json!({}),
            2,
            0,
            &CancellationToken::new(),
        );
        assert!(second.ok, "{:?}", second.error);
        assert!(wait_for_process_exit(first_pid, Duration::from_secs(2)));
        let pids = fs::read_to_string(&pid_file).unwrap();
        let pids = pids.lines().collect::<Vec<_>>();
        assert_eq!(pids.len(), 2, "{pids:?}");
        assert_ne!(pids[0], pids[1]);
        cleanup(&context);
    }

    fn compile_terminal_after_call_fixture(context: &WorkspaceContext) -> PathBuf {
        let source = context.cache_root.join("terminal-after-call-fixture.rs");
        let executable = unica_process::test_support::fixture_executable_path(
            &context.cache_root,
            "terminal-after-call-fixture",
        );
        fs::create_dir_all(&context.cache_root).unwrap();
        fs::write(
            &source,
            r#"use std::{env, fs::{self, OpenOptions}, io::{self, BufRead, Write}, thread, time::Duration};
fn main() {
    let pid_file = env::args().nth(1).unwrap();
    let marker = env::args().nth(2).unwrap();
    let first = fs::read_to_string(&pid_file).unwrap_or_default().lines().count() == 0;
    writeln!(OpenOptions::new().create(true).append(true).open(&pid_file).unwrap(), "{}", std::process::id()).unwrap();
    for (index, line) in io::stdin().lock().lines().enumerate() {
        line.unwrap();
        match index {
            0 => println!("{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{{}}}}"),
            2 => {
                println!("{{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{{\"content\":[{{\"type\":\"text\",\"text\":\"ok\"}}]}}}}");
                io::stdout().flush().unwrap();
                if first {
                    println!("malformed-after-success");
                    io::stdout().flush().unwrap();
                    fs::write(&marker, b"malformed-written").unwrap();
                    thread::sleep(Duration::from_secs(30));
                }
            }
            _ => {}
        }
        io::stdout().flush().unwrap();
    }
}"#,
        )
        .unwrap();
        let status = Command::new("rustc")
            .arg(&source)
            .arg("-o")
            .arg(&executable)
            .status()
            .unwrap();
        assert!(status.success());
        executable
    }

    /// An analyzer that never answers its first call and records the
    /// cancellation it receives for it; later calls are answered by id.
    fn compile_stall_first_call_fixture(context: &WorkspaceContext) -> PathBuf {
        let source = context.cache_root.join("stall-first-call-fixture.rs");
        let executable = unica_process::test_support::fixture_executable_path(
            &context.cache_root,
            "stall-first-call-fixture",
        );
        fs::create_dir_all(&context.cache_root).unwrap();
        fs::write(
            &source,
            r#"use std::{env, fs::OpenOptions, io::{self, BufRead, Write}};
fn main() {
    let pid_file = env::args().nth(1).unwrap();
    let log_file = env::args().nth(2).unwrap();
    let log = |entry: &str| writeln!(OpenOptions::new().create(true).append(true).open(&log_file).unwrap(), "{entry}").unwrap();
    writeln!(OpenOptions::new().create(true).append(true).open(pid_file).unwrap(), "{}", std::process::id()).unwrap();
    let mut calls = 0;
    for line in io::stdin().lock().lines() {
        let line = line.unwrap();
        if line.contains("\"method\":\"initialize\"") {
            println!("{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{{}}}}");
        } else if line.contains("\"method\":\"notifications/cancelled\"") {
            log("cancelled");
        } else if line.contains("\"method\":\"tools/call\"") {
            calls += 1;
            if calls == 1 {
                log("holding");
                continue;
            }
            let id: String = line.split("\"id\":").nth(1).unwrap().chars().take_while(char::is_ascii_digit).collect();
            println!("{{\"jsonrpc\":\"2.0\",\"id\":{id},\"result\":{{\"content\":[{{\"type\":\"text\",\"text\":\"ok\"}}]}}}}");
        }
        io::stdout().flush().unwrap();
    }
}"#,
        )
        .unwrap();
        let status = Command::new("rustc")
            .arg(&source)
            .arg("-o")
            .arg(&executable)
            .status()
            .unwrap();
        assert!(status.success());
        executable
    }

    fn compile_exit_after_call_fixture(context: &WorkspaceContext) -> PathBuf {
        let source = context.cache_root.join("exit-after-call-fixture.rs");
        let executable = unica_process::test_support::fixture_executable_path(
            &context.cache_root,
            "exit-after-call-fixture",
        );
        fs::create_dir_all(&context.cache_root).unwrap();
        fs::write(
            &source,
            r#"use std::{env, fs::OpenOptions, io::{self, BufRead, Write}};
fn main() {
    let pid_file = env::args().nth(1).unwrap();
    let completion_file = env::args().nth(2).unwrap();
    writeln!(OpenOptions::new().create(true).append(true).open(pid_file).unwrap(), "{}", std::process::id()).unwrap();
    for (index, line) in io::stdin().lock().lines().enumerate() {
        line.unwrap();
        match index {
            0 => println!("{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{{}}}}"),
            2 => { println!("{{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{{\"content\":[{{\"type\":\"text\",\"text\":\"ok\"}}]}}}}"); io::stdout().flush().unwrap(); std::fs::write(completion_file, b"completed").unwrap(); break; }
            _ => {}
        }
        io::stdout().flush().unwrap();
    }
}"#,
        )
        .unwrap();
        let status = Command::new("rustc")
            .arg(&source)
            .arg("-o")
            .arg(&executable)
            .status()
            .unwrap();
        assert!(status.success());
        executable
    }

    fn compile_session_tree_fixture(context: &WorkspaceContext) -> PathBuf {
        let source = context.cache_root.join("session-tree-fixture.rs");
        let executable = unica_process::test_support::fixture_executable_path(
            &context.cache_root,
            "session-tree-fixture",
        );
        fs::create_dir_all(&context.cache_root).unwrap();
        fs::write(
            &source,
            r#"use std::{env, fs, io::{self, BufRead, Write}, process::Command, thread, time::Duration};
fn main() {
    if env::args().nth(1).as_deref() == Some("child") {
        thread::sleep(Duration::from_secs(5));
        return;
    }
    let pid_file = env::args().nth(1).unwrap();
    let child = Command::new(env::current_exe().unwrap()).arg("child").spawn().unwrap();
    fs::write(pid_file, child.id().to_string()).unwrap();
    let stdin = io::stdin();
    for (index, line) in stdin.lock().lines().enumerate() {
        line.unwrap();
        if index == 0 {
            println!("{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{{}}}}");
            io::stdout().flush().unwrap();
        } else {
            thread::sleep(Duration::from_secs(30));
        }
    }
}"#,
        )
        .unwrap();
        let status = Command::new("rustc")
            .arg(&source)
            .arg("-o")
            .arg(&executable)
            .status()
            .unwrap();
        assert!(status.success());
        executable
    }

    fn wait_for_recorded_pid(path: &Path) -> u32 {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Ok(text) = fs::read_to_string(path) {
                if let Some(pid) = text.lines().find_map(|line| line.trim().parse().ok()) {
                    return pid;
                }
            }
            assert!(
                Instant::now() < deadline,
                "fixture did not record child pid"
            );
            thread::sleep(Duration::from_millis(25));
        }
    }

    fn wait_for_runtime_reader_terminal(runtime: &WorkspaceServiceRuntime, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        loop {
            let closed = runtime
                .test_analyzer()
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|session| session.test_reader_terminal().load(Ordering::Acquire));
            if closed {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "analyzer stdout reader terminal state was not observed"
            );
            thread::yield_now();
        }
    }

    fn compile_initialize_fixture(context: &WorkspaceContext) -> PathBuf {
        let source = context.cache_root.join("initialize-fixture.rs");
        let executable = unica_process::test_support::fixture_executable_path(
            &context.cache_root,
            "initialize-fixture",
        );
        fs::create_dir_all(&context.cache_root).unwrap();
        fs::write(
            &source,
            r#"use std::{env, fs, io::{self, BufRead, Write}, thread, time::Duration};
fn main() {
    let mode = env::args().nth(1).unwrap_or_default();
    if mode == "incomplete-eof" {
        fs::write(env::args().nth(2).unwrap(), std::process::id().to_string()).unwrap();
        let mut request = String::new();
        io::stdin().lock().read_line(&mut request).unwrap();
        assert!(request.contains("\"method\":\"initialize\""));
        fs::write(env::args().nth(3).unwrap(), "initialize-read").unwrap();
        print!("{{\"jsonrpc\":\"2.0\",\"id\":1");
        io::stdout().flush().unwrap();
        return;
    }
    if mode == "fragmented" {
        print!("{{\"jsonrpc\":\"2.0\",\"id\":1");
        io::stdout().flush().unwrap();
    }
    thread::sleep(Duration::from_secs(30));
}"#,
        )
        .unwrap();
        let status = Command::new("rustc")
            .arg(&source)
            .arg("-o")
            .arg(&executable)
            .status()
            .unwrap();
        assert!(status.success());
        executable
    }

    #[derive(Clone, Default)]
    struct ManualClock(Arc<Mutex<Duration>>);

    impl ManualClock {
        fn advance(&self, duration: Duration) {
            *self.0.lock().unwrap() += duration;
        }
    }

    impl ConnectorClock for ManualClock {
        fn elapsed(&self) -> Duration {
            *self.0.lock().unwrap()
        }
    }

    #[derive(Clone, Copy)]
    enum FailureStage {
        Connect,
        Write,
        Read,
        Eof,
    }

    struct RacingIo {
        cancellation: CancellationToken,
        stage: FailureStage,
        connections: Mutex<u32>,
        connect_timeouts: Mutex<Vec<Duration>>,
    }

    impl RacingIo {
        fn new(cancellation: CancellationToken, stage: FailureStage) -> Self {
            Self {
                cancellation,
                stage,
                connections: Mutex::new(0),
                connect_timeouts: Mutex::new(Vec::new()),
            }
        }
    }

    impl ConnectorIo for RacingIo {
        fn connect(&self, _port: u16, timeout: Duration) -> io::Result<Box<dyn ConnectorStream>> {
            self.connect_timeouts.lock().unwrap().push(timeout);
            let mut connections = self.connections.lock().unwrap();
            *connections += 1;
            if *connections == 1 && matches!(self.stage, FailureStage::Connect) {
                self.cancellation.cancel();
                return Err(io::Error::new(ErrorKind::ConnectionRefused, "connect race"));
            }
            Ok(Box::new(RacingStream {
                cancellation: self.cancellation.clone(),
                stage: (*connections == 1).then_some(self.stage),
                writes: 0,
            }))
        }
    }

    struct RacingStream {
        cancellation: CancellationToken,
        stage: Option<FailureStage>,
        writes: usize,
    }

    struct BudgetIo {
        clock: ManualClock,
        write_timeouts: Arc<Mutex<Vec<Duration>>>,
        expected_connect_timeout: Duration,
        connect_advance: Duration,
        first_write_advance: Duration,
        flush_advance: Duration,
    }

    impl ConnectorIo for BudgetIo {
        fn connect(&self, _port: u16, timeout: Duration) -> io::Result<Box<dyn ConnectorStream>> {
            assert_eq!(timeout, self.expected_connect_timeout);
            self.clock.advance(self.connect_advance);
            Ok(Box::new(BudgetStream {
                clock: self.clock.clone(),
                write_timeouts: Arc::clone(&self.write_timeouts),
                first_write_advance: self.first_write_advance,
                flush_advance: self.flush_advance,
                writes: 0,
            }))
        }
    }

    struct BudgetStream {
        clock: ManualClock,
        write_timeouts: Arc<Mutex<Vec<Duration>>>,
        first_write_advance: Duration,
        flush_advance: Duration,
        writes: usize,
    }

    impl Read for BudgetStream {
        fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
            Ok(0)
        }
    }

    impl Write for BudgetStream {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.writes += 1;
            if self.writes == 1 {
                self.clock.advance(self.first_write_advance);
            }
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            self.clock.advance(self.flush_advance);
            Ok(())
        }
    }

    impl ConnectorStream for BudgetStream {
        fn set_read_timeout(&self, _timeout: Option<Duration>) -> io::Result<()> {
            Ok(())
        }

        fn set_write_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
            self.write_timeouts.lock().unwrap().push(timeout.unwrap());
            Ok(())
        }
    }

    struct ChunkStream {
        chunks: std::collections::VecDeque<Vec<u8>>,
        chunk_offset: usize,
        clock: ManualClock,
        advance_per_read: Duration,
        cancel_after_reads: Option<(CancellationToken, usize)>,
        reads: usize,
    }

    impl Read for ChunkStream {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.clock.advance(self.advance_per_read);
            self.reads += 1;
            if let Some((token, target)) = &self.cancel_after_reads {
                if self.reads == *target {
                    token.cancel();
                }
            }
            let Some(chunk) = self.chunks.front() else {
                return Err(io::Error::new(ErrorKind::TimedOut, "poll"));
            };
            let remaining = &chunk[self.chunk_offset..];
            let count = remaining.len().min(buf.len());
            buf[..count].copy_from_slice(&remaining[..count]);
            self.chunk_offset += count;
            if self.chunk_offset == chunk.len() {
                self.chunks.pop_front();
                self.chunk_offset = 0;
            }
            Ok(count)
        }
    }

    impl Write for ChunkStream {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl ConnectorStream for ChunkStream {
        fn set_read_timeout(&self, _timeout: Option<Duration>) -> io::Result<()> {
            Ok(())
        }

        fn set_write_timeout(&self, _timeout: Option<Duration>) -> io::Result<()> {
            Ok(())
        }
    }

    enum PartialWriteFailure {
        None,
        TimeoutAfterBudget,
        CancelAfterBudget,
        WouldBlockThenCancel,
    }

    struct PartialWriteStream {
        clock: ManualClock,
        cancellation: CancellationToken,
        max_write: usize,
        advance_per_write: Duration,
        failure: PartialWriteFailure,
        writes: Vec<u8>,
        timeouts: Mutex<Vec<Duration>>,
    }

    impl Read for PartialWriteStream {
        fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
            Ok(0)
        }
    }

    impl Write for PartialWriteStream {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.clock.advance(self.advance_per_write);
            match self.failure {
                PartialWriteFailure::TimeoutAfterBudget => {
                    return Err(io::Error::new(ErrorKind::TimedOut, "write timeout"));
                }
                PartialWriteFailure::CancelAfterBudget => {
                    self.cancellation.cancel();
                    return Err(io::Error::new(ErrorKind::BrokenPipe, "write cancelled"));
                }
                PartialWriteFailure::WouldBlockThenCancel => {
                    self.cancellation.cancel();
                    return Err(io::Error::new(ErrorKind::WouldBlock, "blocked"));
                }
                PartialWriteFailure::None => {}
            }
            let count = self.max_write.min(buf.len());
            self.writes.extend_from_slice(&buf[..count]);
            Ok(count)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl ConnectorStream for PartialWriteStream {
        fn set_read_timeout(&self, _timeout: Option<Duration>) -> io::Result<()> {
            Ok(())
        }

        fn set_write_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
            self.timeouts.lock().unwrap().push(timeout.unwrap());
            Ok(())
        }
    }

    struct DeadlineFlushStream {
        clock: ManualClock,
        advance: Duration,
    }

    impl Read for DeadlineFlushStream {
        fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
            Ok(0)
        }
    }

    impl Write for DeadlineFlushStream {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            self.clock.advance(self.advance);
            Ok(())
        }
    }

    impl ConnectorStream for DeadlineFlushStream {
        fn set_read_timeout(&self, _timeout: Option<Duration>) -> io::Result<()> {
            Ok(())
        }
        fn set_write_timeout(&self, _timeout: Option<Duration>) -> io::Result<()> {
            Ok(())
        }
    }

    impl Read for RacingStream {
        fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
            match self.stage {
                Some(FailureStage::Read) => {
                    self.cancellation.cancel();
                    Err(io::Error::new(ErrorKind::ConnectionReset, "read race"))
                }
                Some(FailureStage::Eof) => {
                    self.cancellation.cancel();
                    Ok(0)
                }
                _ => Ok(0),
            }
        }
    }

    impl Write for RacingStream {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.writes += 1;
            if self.writes == 1 && matches!(self.stage, Some(FailureStage::Write)) {
                self.cancellation.cancel();
                return Err(io::Error::new(ErrorKind::BrokenPipe, "write race"));
            }
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl ConnectorStream for RacingStream {
        fn set_read_timeout(&self, _timeout: Option<Duration>) -> io::Result<()> {
            Ok(())
        }

        fn set_write_timeout(&self, _timeout: Option<Duration>) -> io::Result<()> {
            Ok(())
        }
    }

    fn connector_test_record() -> WorkspaceServiceRecord {
        WorkspaceServiceRecord {
            schema_version: SERVICE_SCHEMA_VERSION,
            pid: std::process::id(),
            port: 1,
            token: "secret".to_string(),
            build: CoreIdentity::production().as_str().to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            workspace_root: "workspace".to_string(),
            source_root: "source".to_string(),
            started_at: now_secs_for_test(),
            last_access_at: now_secs_for_test(),
        }
    }

    fn connector_test_request(kind: ServiceRequestKind) -> ServiceRequest {
        ServiceRequest {
            token: "secret".to_string(),
            kind,
        }
    }

    struct AnchoredConnectorClock {
        started: Instant,
        elapsed: ManualClock,
    }

    impl ConnectorClock for AnchoredConnectorClock {
        fn elapsed(&self) -> Duration {
            self.elapsed.elapsed()
        }
        fn now(&self) -> Instant {
            self.started + self.elapsed.elapsed()
        }
    }

    struct TimedAckIo {
        clock: ManualClock,
        advance_per_read: Duration,
        connect_timeouts: Arc<Mutex<Vec<Duration>>>,
        cancel_after_reads: Option<(CancellationToken, usize)>,
    }

    impl ConnectorIo for TimedAckIo {
        fn connect(&self, _port: u16, timeout: Duration) -> io::Result<Box<dyn ConnectorStream>> {
            self.connect_timeouts.lock().unwrap().push(timeout);
            Ok(Box::new(ChunkStream {
                chunks: [b"{\"ok\":".to_vec(), b"true}\n".to_vec()].into(),
                chunk_offset: 0,
                clock: self.clock.clone(),
                advance_per_read: self.advance_per_read,
                cancel_after_reads: self.cancel_after_reads.clone(),
                reads: 0,
            }))
        }
    }

    #[test]
    fn committed_invalidation_records_share_the_exact_finite_deadline_through_ack() {
        struct DeadlineConnector {
            clock: AnchoredConnectorClock,
            io: TimedAckIo,
            seen: std::cell::RefCell<Vec<OperationDeadline>>,
            succeeded: std::cell::RefCell<Vec<bool>>,
        }
        impl ServiceConnector for DeadlineConnector {
            fn send(
                &self,
                _: &WorkspaceServiceRecord,
                _: ServiceRequest,
                _: &CancellationToken,
                _: Duration,
            ) -> Result<ServiceResponse, String> {
                panic!("committed invalidation must retain the caller deadline");
            }
            fn send_before(
                &self,
                record: &WorkspaceServiceRecord,
                request: ServiceRequest,
                cancellation: &CancellationToken,
                deadline: OperationDeadline,
            ) -> Result<ServiceResponse, String> {
                self.seen.borrow_mut().push(deadline);
                let result = SYSTEM_SERVICE_CONNECTOR.send_with_deadline(
                    record,
                    request,
                    cancellation,
                    &deadline,
                    &self.io,
                    &self.clock,
                );
                self.succeeded.borrow_mut().push(result.is_ok());
                result
            }

            fn send_operation_before(
                &self,
                _record: &WorkspaceServiceRecord,
                _request: ServiceRequest,
                _cancellation: &CancellationToken,
                _deadline: ProviderDeadline,
            ) -> Result<ServiceResponse, String> {
                unreachable!("this legacy fixture does not use the prepared BSL route")
            }
        }
        let context = test_context("committed-invalidation-aggregate");
        for source in ["src", "extension"] {
            let identity =
                WorkspaceServiceIdentity::new(&context, &context.workspace_root.join(source))
                    .unwrap();
            write_record(
                &identity,
                test_record(&identity, 1, CoreIdentity::production().as_str()),
            );
        }
        let started = Instant::now();
        let deadline = OperationDeadline::Finite(started + Duration::from_secs(60));
        let elapsed = ManualClock::default();
        let connect_timeouts = Arc::new(Mutex::new(Vec::new()));
        let connector = DeadlineConnector {
            clock: AnchoredConnectorClock {
                started,
                elapsed: elapsed.clone(),
            },
            io: TimedAckIo {
                clock: elapsed,
                advance_per_read: Duration::from_secs(18),
                connect_timeouts: Arc::clone(&connect_timeouts),
                cancel_after_reads: None,
            },
            seen: Default::default(),
            succeeded: Default::default(),
        };
        let spawner = RecordingSpawner::default();
        WorkspaceServiceManager::with_io(
            &connector,
            &spawner,
            analysis_composition::service_environment(),
        )
        .notify_committed_changes(
            &context,
            &[DomainEvent::new(
                DomainEventKind::ModuleChanged,
                "main:CommonModule.SearchProbe",
            )],
            &CancellationToken::new(),
            deadline,
        );
        assert_eq!(*connector.seen.borrow(), [deadline, deadline]);
        assert_eq!(
            *connector.succeeded.borrow(),
            [true, false],
            "the second ACK must not get a replenished budget"
        );
        assert_eq!(
            *connect_timeouts.lock().unwrap(),
            [
                SERVICE_CONTROL_CONNECT_TIMEOUT,
                SERVICE_CONTROL_CONNECT_TIMEOUT
            ]
        );
        cleanup(&context);
    }

    #[test]
    fn no_deadline_invalidation_keeps_late_fragmented_ack_and_observes_original_cancellation() {
        for cancel in [false, true] {
            let elapsed = ManualClock::default();
            let clock = AnchoredConnectorClock {
                started: Instant::now(),
                elapsed: elapsed.clone(),
            };
            let cancellation = CancellationToken::new();
            let connect_timeouts = Arc::new(Mutex::new(Vec::new()));
            let io = TimedAckIo {
                clock: elapsed,
                advance_per_read: Duration::from_secs(180),
                connect_timeouts: Arc::clone(&connect_timeouts),
                cancel_after_reads: cancel.then(|| (cancellation.clone(), 2)),
            };
            let result = SYSTEM_SERVICE_CONNECTOR.send_with_deadline(
                &connector_test_record(),
                connector_test_request(ServiceRequestKind::Invalidate { events: vec![] }),
                &cancellation,
                &OperationDeadline::NoDeadline,
                &io,
                &clock,
            );
            assert_eq!(clock.elapsed(), Duration::from_secs(360));
            assert_eq!(
                *connect_timeouts.lock().unwrap(),
                [SERVICE_CONTROL_CONNECT_TIMEOUT]
            );
            if cancel {
                assert!(result.unwrap_err().starts_with("cancelled:"));
            } else {
                assert!(
                    result.unwrap().ok,
                    "absence must survive the old120s cutoff and a fragmented ACK"
                );
            }
        }
    }

    #[test]
    fn cancellable_connector_deadline_is_aggregate() {
        let clock = ManualClock::default();
        let deadline = Deadline::new(&clock, Duration::from_millis(500));
        clock.advance(Duration::from_millis(300));
        assert_eq!(deadline.remaining(&clock), Some(Duration::from_millis(200)));
        clock.advance(Duration::from_millis(200));
        assert_eq!(deadline.remaining(&clock), None);
    }

    /// rule: INV.PERF.SERVICE-OPERATION-DEADLINE
    #[test]
    fn cancellable_connector_cancel_control_uses_one_aggregate_500ms_budget() {
        let clock = ManualClock::default();
        let write_timeouts = Arc::new(Mutex::new(Vec::new()));
        let io = BudgetIo {
            clock: clock.clone(),
            write_timeouts: Arc::clone(&write_timeouts),
            expected_connect_timeout: SERVICE_CONTROL_CONNECT_TIMEOUT,
            connect_advance: Duration::from_millis(300),
            first_write_advance: Duration::from_millis(100),
            flush_advance: Duration::ZERO,
        };
        SYSTEM_SERVICE_CONNECTOR
            .send_control_with(
                &connector_test_record(),
                ServiceRequestKind::Cancel {
                    operation_id: "budget-operation".to_string(),
                },
                &io,
                &clock,
            )
            .unwrap();

        assert_eq!(
            write_timeouts.lock().unwrap().as_slice(),
            &[
                Duration::from_millis(200),
                Duration::from_millis(100),
                Duration::from_millis(100)
            ]
        );
    }

    /// rule: INV.PERF.SERVICE-OPERATION-DEADLINE
    #[test]
    fn control_flush_success_cannot_cross_aggregate_500ms_budget() {
        let clock = ManualClock::default();
        let io = BudgetIo {
            clock: clock.clone(),
            write_timeouts: Arc::new(Mutex::new(Vec::new())),
            expected_connect_timeout: SERVICE_CONTROL_CONNECT_TIMEOUT,
            connect_advance: Duration::from_millis(300),
            first_write_advance: Duration::from_millis(100),
            flush_advance: Duration::from_millis(100),
        };
        let error = SYSTEM_SERVICE_CONNECTOR
            .send_control_with(
                &connector_test_record(),
                ServiceRequestKind::Cancel {
                    operation_id: "flush-deadline".into(),
                },
                &io,
                &clock,
            )
            .unwrap_err();
        assert!(error.contains("timed out"), "{error}");
    }

    #[test]
    fn cancellable_connector_work_write_uses_budget_remaining_after_connect() {
        let clock = ManualClock::default();
        let write_timeouts = Arc::new(Mutex::new(Vec::new()));
        let io = BudgetIo {
            clock: clock.clone(),
            write_timeouts: Arc::clone(&write_timeouts),
            expected_connect_timeout: SERVICE_CONNECT_TIMEOUT,
            connect_advance: Duration::from_secs(3),
            first_write_advance: Duration::ZERO,
            flush_advance: Duration::ZERO,
        };
        let cancellation = CancellationToken::new();
        let _ = SYSTEM_SERVICE_CONNECTOR.send_with(
            &connector_test_record(),
            connector_test_request(ServiceRequestKind::BslMcp {
                operation_id: "work-budget".to_string(),
                tool_name: "search".to_string(),
                tool_args: json!({}),
                timeout_seconds: 120,
                timeout_nanos: 0,
                yields: false,
            }),
            &cancellation,
            SERVICE_REQUEST_TIMEOUT,
            &io,
            &clock,
        );

        assert_eq!(
            write_timeouts.lock().unwrap().first().copied(),
            Some(Duration::from_millis(100))
        );
    }

    #[test]
    fn cancellable_connector_reads_fragmented_response_and_ignores_bytes_after_newline() {
        let clock = ManualClock::default();
        let deadline = Deadline::new(&clock, Duration::from_secs(1));
        let mut stream = ChunkStream {
            chunks: [b"{\"ok\":".to_vec(), b"true}\nignored".to_vec()].into(),
            chunk_offset: 0,
            clock: clock.clone(),
            advance_per_read: Duration::from_millis(1),
            cancel_after_reads: None,
            reads: 0,
        };
        let response =
            read_service_response(&mut stream, &deadline, &clock, &CancellationToken::new())
                .unwrap();
        assert!(response.ok);
    }

    #[test]
    fn cancellable_connector_partial_response_cannot_bypass_deadline() {
        let clock = ManualClock::default();
        let deadline = Deadline::new(&clock, Duration::from_millis(3));
        let mut stream = ChunkStream {
            chunks: [b"a".to_vec(), b"b".to_vec(), b"c".to_vec(), b"d".to_vec()].into(),
            chunk_offset: 0,
            clock: clock.clone(),
            advance_per_read: Duration::from_millis(1),
            cancel_after_reads: None,
            reads: 0,
        };
        let error =
            read_service_response(&mut stream, &deadline, &clock, &CancellationToken::new())
                .unwrap_err();
        assert!(error.starts_with("timeout:"), "{error}");
    }

    #[test]
    fn cancellable_connector_partial_response_cannot_bypass_cancellation() {
        let clock = ManualClock::default();
        let cancellation = CancellationToken::new();
        let deadline = Deadline::new(&clock, Duration::from_secs(1));
        let mut stream = ChunkStream {
            chunks: [b"partial".to_vec(), b"still-partial".to_vec()].into(),
            chunk_offset: 0,
            clock: clock.clone(),
            advance_per_read: Duration::ZERO,
            cancel_after_reads: Some((cancellation.clone(), 2)),
            reads: 0,
        };
        let error =
            read_service_response(&mut stream, &deadline, &clock, &cancellation).unwrap_err();
        assert!(error.starts_with("cancelled:"), "{error}");
    }

    #[test]
    fn cancellable_connector_rejects_oversized_response_line() {
        let clock = ManualClock::default();
        let deadline = Deadline::new(&clock, Duration::from_secs(1));
        let mut stream = ChunkStream {
            chunks: [vec![b'x'; SERVICE_RESPONSE_LINE_LIMIT + 1]].into(),
            chunk_offset: 0,
            clock: clock.clone(),
            advance_per_read: Duration::ZERO,
            cancel_after_reads: None,
            reads: 0,
        };
        let error =
            read_service_response(&mut stream, &deadline, &clock, &CancellationToken::new())
                .unwrap_err();
        assert!(error.contains("response line exceeds"), "{error}");
    }

    #[test]
    fn cancellable_connector_partial_writes_recompute_remaining_budget() {
        let clock = ManualClock::default();
        let cancellation = CancellationToken::new();
        let deadline = Deadline::new(&clock, Duration::from_millis(100));
        let mut stream = PartialWriteStream {
            clock: clock.clone(),
            cancellation: cancellation.clone(),
            max_write: 2,
            advance_per_write: Duration::from_millis(10),
            failure: PartialWriteFailure::None,
            writes: Vec::new(),
            timeouts: Mutex::new(Vec::new()),
        };
        write_with_deadline(&mut stream, b"abcdef", &deadline, &clock, &cancellation).unwrap();
        assert_eq!(stream.writes, b"abcdef");
        assert_eq!(
            *stream.timeouts.lock().unwrap(),
            vec![
                Duration::from_millis(100),
                Duration::from_millis(90),
                Duration::from_millis(80)
            ]
        );
    }

    #[test]
    fn cancellable_connector_successful_final_write_cannot_cross_deadline() {
        let clock = ManualClock::default();
        let cancellation = CancellationToken::new();
        let deadline = Deadline::new(&clock, Duration::from_millis(10));
        let mut stream = PartialWriteStream {
            clock: clock.clone(),
            cancellation: cancellation.clone(),
            max_write: 1,
            advance_per_write: Duration::from_millis(10),
            failure: PartialWriteFailure::None,
            writes: Vec::new(),
            timeouts: Mutex::new(Vec::new()),
        };
        let error =
            write_with_deadline(&mut stream, b"x", &deadline, &clock, &cancellation).unwrap_err();
        assert!(error.starts_with("timeout:"), "{error}");
    }

    #[test]
    fn cancellable_connector_successful_flush_cannot_cross_deadline() {
        let clock = ManualClock::default();
        let deadline = Deadline::new(&clock, Duration::from_millis(10));
        let mut stream = DeadlineFlushStream {
            clock: clock.clone(),
            advance: Duration::from_millis(10),
        };
        let error = flush_with_deadline(&mut stream, &deadline, &clock, &CancellationToken::new())
            .unwrap_err();
        assert!(error.starts_with("timeout:"), "{error}");
    }

    #[test]
    fn cancellable_connector_rechecks_cancel_after_blocked_write_slice() {
        let clock = ManualClock::default();
        let cancellation = CancellationToken::new();
        let deadline = Deadline::new(&clock, Duration::from_secs(5));
        let mut stream = PartialWriteStream {
            clock: clock.clone(),
            cancellation: cancellation.clone(),
            max_write: 1,
            advance_per_write: Duration::from_millis(100),
            failure: PartialWriteFailure::WouldBlockThenCancel,
            writes: Vec::new(),
            timeouts: Mutex::new(Vec::new()),
        };
        let error =
            write_with_deadline(&mut stream, b"x", &deadline, &clock, &cancellation).unwrap_err();
        assert!(error.starts_with("cancelled:"), "{error}");
        assert_eq!(
            stream.timeouts.lock().unwrap().as_slice(),
            &[Duration::from_millis(100)]
        );
    }

    #[test]
    fn cancellable_connector_write_error_uses_timeout_unless_cancelled() {
        for (failure, expected) in [
            (PartialWriteFailure::TimeoutAfterBudget, "timeout:"),
            (PartialWriteFailure::CancelAfterBudget, "cancelled:"),
        ] {
            let clock = ManualClock::default();
            let cancellation = CancellationToken::new();
            let deadline = Deadline::new(&clock, Duration::from_millis(10));
            let mut stream = PartialWriteStream {
                clock: clock.clone(),
                cancellation: cancellation.clone(),
                max_write: 1,
                advance_per_write: Duration::from_millis(10),
                failure,
                writes: Vec::new(),
                timeouts: Mutex::new(Vec::new()),
            };
            let error = write_with_deadline(&mut stream, b"x", &deadline, &clock, &cancellation)
                .unwrap_err();
            assert!(error.starts_with(expected), "{error}");
        }
    }

    #[test]
    fn cancellable_connector_rejects_write_zero() {
        let clock = ManualClock::default();
        let cancellation = CancellationToken::new();
        let deadline = Deadline::new(&clock, Duration::from_secs(1));
        let mut stream = PartialWriteStream {
            clock: clock.clone(),
            cancellation: cancellation.clone(),
            max_write: 0,
            advance_per_write: Duration::ZERO,
            failure: PartialWriteFailure::None,
            writes: Vec::new(),
            timeouts: Mutex::new(Vec::new()),
        };
        let error =
            write_with_deadline(&mut stream, b"x", &deadline, &clock, &cancellation).unwrap_err();
        assert!(error.contains("failed to write workspace service request"));
    }

    #[test]
    fn cancellable_connector_prioritizes_cancel_over_transport_races() {
        for stage in [
            FailureStage::Connect,
            FailureStage::Write,
            FailureStage::Read,
            FailureStage::Eof,
        ] {
            let cancellation = CancellationToken::new();
            let io = RacingIo::new(cancellation.clone(), stage);
            let clock = ManualClock::default();
            let error = SYSTEM_SERVICE_CONNECTOR
                .send_with(
                    &connector_test_record(),
                    connector_test_request(ServiceRequestKind::BslMcp {
                        operation_id: "race-operation".to_string(),
                        tool_name: "search".to_string(),
                        tool_args: json!({}),
                        timeout_seconds: 120,
                        timeout_nanos: 0,
                        yields: false,
                    }),
                    &cancellation,
                    SERVICE_REQUEST_TIMEOUT,
                    &io,
                    &clock,
                )
                .unwrap_err();
            assert!(error.starts_with("cancelled:"), "{error}");
        }
    }

    #[test]
    fn cancellable_connector_uses_short_connect_budget_for_every_control_kind() {
        for kind in [
            ServiceRequestKind::Expire {
                operation_id: "expiry".into(),
            },
            ServiceRequestKind::Ping,
            ServiceRequestKind::Cancel {
                operation_id: "cancel-budget".to_string(),
            },
            ServiceRequestKind::Invalidate { events: vec![] },
            ServiceRequestKind::Shutdown,
        ] {
            let cancellation = CancellationToken::new();
            let io = RacingIo::new(cancellation.clone(), FailureStage::Eof);
            let _ = SYSTEM_SERVICE_CONNECTOR.send_with(
                &connector_test_record(),
                connector_test_request(kind),
                &cancellation,
                SERVICE_REQUEST_TIMEOUT,
                &io,
                &ManualClock::default(),
            );
            assert_eq!(
                io.connect_timeouts.lock().unwrap().as_slice(),
                &[SERVICE_CONTROL_CONNECT_TIMEOUT]
            );
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum ServiceDeadlineClass {
        Work,
        Control,
        AdmissionPhase,
    }

    fn service_deadline_class(kind: &ServiceRequestKind) -> ServiceDeadlineClass {
        match kind {
            ServiceRequestKind::BslMcp { .. } | ServiceRequestKind::BslMcpBegin { .. } => {
                ServiceDeadlineClass::Work
            }
            ServiceRequestKind::Ping
            | ServiceRequestKind::Cancel { .. }
            | ServiceRequestKind::Expire { .. }
            | ServiceRequestKind::Invalidate { .. }
            | ServiceRequestKind::Shutdown => ServiceDeadlineClass::Control,
            ServiceRequestKind::BslMcpStart { .. } | ServiceRequestKind::BslMcpStop { .. } => {
                ServiceDeadlineClass::AdmissionPhase
            }
        }
    }

    /// rule: INV.PERF.SERVICE-OPERATION-DEADLINE
    #[test]
    fn service_request_kind_deadline_matrix_is_exhaustive() {
        let work = [
            prepared_begin("prepared-matrix"),
            ServiceRequestKind::BslMcp {
                operation_id: "bsl".into(),
                tool_name: "search".into(),
                tool_args: json!({}),
                timeout_seconds: 120,
                timeout_nanos: 0,
                yields: false,
            },
            ServiceRequestKind::BslMcp {
                operation_id: "ready".into(),
                tool_name: "diagnostics".to_string(),
                tool_args: json!({}),
                timeout_seconds: 120,
                timeout_nanos: 0,
                yields: false,
            },
        ];
        let phases = [
            ServiceRequestKind::BslMcpStart {
                operation_id: "phase".into(),
                deadline: BslOperationDeadline::Absent {},
            },
            ServiceRequestKind::BslMcpStop {
                operation_id: "phase".into(),
                cause: BslStopCause::Cancel,
            },
        ];
        let control = [
            ServiceRequestKind::Expire {
                operation_id: "expiry".into(),
            },
            ServiceRequestKind::Ping,
            ServiceRequestKind::Cancel {
                operation_id: "cancel".into(),
            },
            ServiceRequestKind::Invalidate { events: vec![] },
            ServiceRequestKind::Shutdown,
        ];
        let tags = work
            .iter()
            .chain(control.iter())
            .chain(phases.iter())
            .map(|kind| {
                serde_json::to_value(kind).unwrap()["type"]
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            tags,
            [
                "bsl-mcp",
                "bsl-mcp-begin",
                "bsl-mcp-start",
                "bsl-mcp-stop",
                "expire",
                "cancel",
                "invalidate",
                "ping",
                "shutdown"
            ]
            .into_iter()
            .map(str::to_string)
            .collect()
        );

        for kind in work {
            assert_eq!(service_deadline_class(&kind), ServiceDeadlineClass::Work);
            assert!(!kind.is_control());
            let cancellation = CancellationToken::new();
            let io = RacingIo::new(cancellation.clone(), FailureStage::Eof);
            let _ = SYSTEM_SERVICE_CONNECTOR.send_with(
                &connector_test_record(),
                connector_test_request(kind),
                &cancellation,
                SERVICE_REQUEST_TIMEOUT,
                &io,
                &ManualClock::default(),
            );
            assert_eq!(
                io.connect_timeouts.lock().unwrap().as_slice(),
                &[SERVICE_CONNECT_TIMEOUT, SERVICE_CONTROL_CONNECT_TIMEOUT],
                "work request must use its full connect cap and a separately bounded cancellation path"
            );
        }
        for kind in control {
            assert_eq!(service_deadline_class(&kind), ServiceDeadlineClass::Control);
            assert!(kind.is_control());
        }
        for phase in phases {
            assert_eq!(
                service_deadline_class(&phase),
                ServiceDeadlineClass::AdmissionPhase
            );
            assert!(
                !phase.is_control(),
                "admission phases use only their existing data connection"
            );
            assert!(phase.operation_id().is_none());
        }
        cancellable_connector_uses_short_connect_budget_for_every_control_kind();
        cancellable_connector_partial_response_cannot_bypass_deadline();
        manager_shares_one_deadline_across_late_transport_retry();
    }

    #[test]
    fn cancellable_connector_protocol_roundtrips_work_and_cancel_shapes() {
        let bsl = ServiceRequestKind::BslMcp {
            operation_id: Uuid::new_v4().to_string(),
            tool_name: "search".to_string(),
            tool_args: json!({"query": "needle"}),
            timeout_seconds: 5,
            timeout_nanos: 0,
            yields: false,
        };
        for (kind, expected_tag) in [
            (bsl, "bsl-mcp"),
            (
                ServiceRequestKind::Cancel {
                    operation_id: "cancel-id".to_string(),
                },
                "cancel",
            ),
        ] {
            let json = serde_json::to_value(&kind).unwrap();
            assert_eq!(json.get("type").and_then(Value::as_str), Some(expected_tag));
            assert!(json.get("operation_id").is_some());
            assert_eq!(
                serde_json::from_value::<ServiceRequestKind>(json).unwrap(),
                kind
            );
        }

        let invalidation = ServiceRequestKind::Invalidate {
            events: vec![DomainEvent::source_resources_replaced(
                unica_source_contract::events::SourceResourcesReplaced {
                    source_set: "main".to_string(),
                    owner: "CommonModule.Shared".to_string(),
                    roles: vec![unica_source_contract::resources::ResourceRole::BslModule],
                    preimage_hashes: vec!["sha256:before".to_string()],
                    postimage_hashes: vec!["sha256:after".to_string()],
                    affected_targets: vec!["CommonModule.Shared.Module".to_string()],
                },
            )],
        };
        let json = serde_json::to_value(&invalidation).unwrap();
        assert_eq!(json["type"], "invalidate");
        assert_eq!(
            json["events"][0]["details"]["affectedTargets"],
            json!(["CommonModule.Shared.Module"])
        );
        assert_eq!(
            serde_json::from_value::<ServiceRequestKind>(json).unwrap(),
            invalidation
        );
    }

    struct FailingWriter;

    impl Write for FailingWriter {
        fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(ErrorKind::BrokenPipe, "caller disconnected"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::new(ErrorKind::BrokenPipe, "caller disconnected"))
        }
    }

    #[test]
    fn cancel_response_disconnect_is_non_fatal_and_service_record_remains() {
        let context = test_context("cancel-disconnect");
        let identity =
            WorkspaceServiceIdentity::new(&context, &context.workspace_root.join("src")).unwrap();
        let record = test_record(&identity, 34567, CoreIdentity::production().as_str());
        write_record(&identity, record.clone());
        let runtime = new_runtime(identity.clone(), &record);
        let response = runtime.test_cancel_operation("gone-caller");

        assert!(!write_service_response(FailingWriter, &response, true).unwrap());
        assert!(read_record(&identity).is_some());
        let ping = runtime.test_ping();
        assert!(ping.ok);
        cleanup(&context);
    }

    /// rule: INV.RUNTIME.SERVICE-OPERATION-CANCELLATION
    #[test]
    fn cancellable_connector_sends_cancel_on_a_separate_connection() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let (request_tx, request_rx) = mpsc::channel();
        let server = thread::spawn(move || {
            let (work_stream, _) = listener.accept().unwrap();
            let mut work_reader = BufReader::new(work_stream);
            let mut work_line = String::new();
            work_reader.read_line(&mut work_line).unwrap();
            request_tx
                .send(serde_json::from_str::<ServiceRequest>(work_line.trim()).unwrap())
                .unwrap();

            let (cancel_stream, _) = listener.accept().unwrap();
            let mut cancel_line = String::new();
            BufReader::new(cancel_stream)
                .read_line(&mut cancel_line)
                .unwrap();
            request_tx
                .send(serde_json::from_str::<ServiceRequest>(cancel_line.trim()).unwrap())
                .unwrap();
        });
        let record = WorkspaceServiceRecord {
            schema_version: SERVICE_SCHEMA_VERSION,
            pid: std::process::id(),
            port,
            token: "secret".to_string(),
            build: CoreIdentity::production().as_str().to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            workspace_root: "workspace".to_string(),
            source_root: "source".to_string(),
            started_at: now_secs_for_test(),
            last_access_at: now_secs_for_test(),
        };
        let operation_id = "operation-1".to_string();
        let cancellation = CancellationToken::new();
        let caller_token = cancellation.clone();
        let caller = thread::spawn(move || {
            SYSTEM_SERVICE_CONNECTOR.send(
                &record,
                ServiceRequest {
                    token: "secret".to_string(),
                    kind: ServiceRequestKind::BslMcp {
                        operation_id: operation_id.clone(),
                        tool_name: "search".to_string(),
                        tool_args: json!({}),
                        timeout_seconds: 120,
                        timeout_nanos: 0,
                        yields: false,
                    },
                },
                &caller_token,
                SERVICE_REQUEST_TIMEOUT,
            )
        });

        let work = request_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        let work_id = match work.kind {
            ServiceRequestKind::BslMcp { operation_id, .. } => operation_id,
            other => panic!("expected bsl work request, got {other:?}"),
        };
        let cancelled_at = Instant::now();
        cancellation.cancel();
        let cancel = request_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let error = caller.join().unwrap().unwrap_err();

        assert_eq!(
            cancel.kind,
            ServiceRequestKind::Cancel {
                operation_id: work_id
            }
        );
        assert!(error.starts_with("cancelled:"));
        assert!(cancelled_at.elapsed() < Duration::from_secs(2));
        server.join().unwrap();
    }

    #[test]
    fn cancellation_prefix_is_stable_for_pre_cancelled_manager_call() {
        let context = test_context("pre-cancelled-manager");
        let cancellation = CancellationToken::new();
        cancellation.cancel();

        let error = WorkspaceServiceManager::new(analysis_composition::service_environment())
            .ensure_service_cancellable(
                &context,
                &context.workspace_root.join("src"),
                &cancellation,
            )
            .unwrap_err();

        assert!(error.starts_with("cancelled:"));
        cleanup(&context);
    }

    #[test]
    fn bsl_generation_exhausts_operation_budget_before_provider_start() {
        let context = test_context("bsl-generation-budget");
        let source_root = context.workspace_root.join("src");
        fs::write(
            source_root.join("Module.bsl"),
            "Процедура Тест()\nКонецПроцедуры\n",
        )
        .unwrap();
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        let record = test_record(&identity, 1, CoreIdentity::production().as_str());
        let mut runtime = new_runtime(identity, &record);
        let starts = Arc::new(AtomicUsize::new(0));
        runtime.test_set_analyzer_starter(Arc::new({
            let starts = Arc::clone(&starts);
            move |_context, _source_root, _cancellation| {
                starts.fetch_add(1, Ordering::AcqRel);
                Err("provider started after operation budget expired".to_string())
            }
        }));

        let response = runtime.test_handle_bsl_mcp(
            "diagnostics_status",
            json!({}),
            0,
            1,
            &CancellationToken::new(),
        );

        assert_eq!(starts.load(Ordering::Acquire), 0, "{response:?}");
        assert!(
            response
                .error
                .as_deref()
                .is_some_and(|error| error.starts_with("timeout:")),
            "{response:?}"
        );
        cleanup(&context);
    }

    #[test]
    fn bsl_analyzer_cache_is_stable_and_separated_by_workspace_and_source() {
        let context = test_context("bsl-provider-cache");
        let source_root = context.workspace_root.join("src");
        let repeated = bsl_analyzer_cache_dir(&context, &source_root).unwrap();
        let dotted = bsl_analyzer_cache_dir(&context, &source_root.join(".")).unwrap();
        let other_source =
            bsl_analyzer_cache_dir(&context, &context.workspace_root.join("extension")).unwrap();
        let mut other_workspace = test_context("bsl-provider-cache-other");
        let other_workspace_cleanup = other_workspace.clone();
        other_workspace.cache_root = context.cache_root.clone();
        let other_workspace_cache = bsl_analyzer_cache_dir(
            &other_workspace,
            &other_workspace.workspace_root.join("src"),
        )
        .unwrap();

        assert_eq!(repeated, dotted);
        assert_ne!(repeated, other_source);
        assert_ne!(repeated, other_workspace_cache);
        assert!(repeated.starts_with(
            normalize_path_identity(&context.cache_root)
                .unwrap()
                .join("providers/bsl-analyzer/owned-stdio")
        ));
        assert!(!repeated.starts_with(&source_root));
        assert!(!repeated.starts_with(context.cache_root.join("services")));
        assert_eq!(
            repeated.file_name().unwrap().to_string_lossy().len(),
            "source-".len() + 64
        );

        cleanup(&context);
        cleanup(&other_workspace_cleanup);
    }

    #[test]
    fn provider_state_key_has_a_stable_domain_separated_sha256_contract() {
        assert_eq!(
            provider_state_key(Path::new("/workspace"), Path::new("/workspace/src/cf")).unwrap(),
            "source-156ee86cec5364248d3b62f5fd5ba2ddf820f63e50dabeef868d5cb18600d790"
        );
    }

    #[test]
    fn provider_state_key_keeps_distinct_non_utf8_path_identities_separate() {
        let (first, second) = unica_workspace::test_support::distinct_non_unicode_paths_for_test()
            .expect("non-Unicode paths exist on Unix and Windows");
        let workspace = Path::new("/workspace");

        assert_ne!(first, second);
        assert!(first.to_str().is_none());
        assert!(second.to_str().is_none());
        assert_eq!(
            first.display().to_string(),
            second.display().to_string(),
            "the reproducer requires lossy display collision"
        );
        assert_ne!(
            provider_state_key(workspace, &first).unwrap(),
            provider_state_key(workspace, &second).unwrap()
        );
    }

    #[test]
    fn bsl_analyzer_cache_absolutizes_a_relative_unica_cache_root() {
        let mut context = test_context("bsl-provider-relative-cache");
        let cleanup_context = context.clone();
        context.cache_root = PathBuf::from("relative-unica-cache");
        let source_root = context.workspace_root.join("src");

        let cache_dir = bsl_analyzer_cache_dir(&context, &source_root).unwrap();

        assert!(cache_dir.is_absolute(), "{cache_dir:?}");
        assert!(cache_dir.ends_with(
            Path::new("relative-unica-cache")
                .join("providers")
                .join("bsl-analyzer")
                .join("owned-stdio")
                .join(cache_dir.file_name().unwrap())
        ));

        cleanup(&cleanup_context);
    }

    #[test]
    fn service_identity_reuses_same_workspace_source_root_and_separates_other_roots() {
        let context = test_context("identity");
        let source_root = context.workspace_root.join("src");
        let same = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        let repeated = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        let other_source =
            WorkspaceServiceIdentity::new(&context, &context.workspace_root.join("extension"))
                .unwrap();
        let other_workspace = test_context("identity-other");
        let other_workspace_identity =
            WorkspaceServiceIdentity::new(&other_workspace, &other_workspace.workspace_root)
                .unwrap();

        assert_eq!(same.key, repeated.key);
        assert_ne!(same.key, other_source.key);
        assert_ne!(same.key, other_workspace_identity.key);
        assert!(same
            .service_dir
            .ends_with(Path::new("services").join(&same.key)));

        cleanup(&context);
        cleanup(&other_workspace);
    }

    /// rule: INV.APP.HIDDEN-SERVICES
    #[test]
    fn hidden_service_identity_distinguishes_user_core_daemon_from_workspace_helpers() {
        use unica_daemon::identity::{CoreIdentity, DaemonStateDirectory};

        let first = test_context("hidden-topology-first");
        let second = test_context("hidden-topology-second");
        let first_helper =
            WorkspaceServiceIdentity::new(&first, &first.workspace_root.join("src")).unwrap();
        let second_helper =
            WorkspaceServiceIdentity::new(&second, &second.workspace_root.join("src")).unwrap();
        let daemon_fixture = tempfile::tempdir().unwrap();
        let provider_state = std::fs::canonicalize(daemon_fixture.path()).unwrap();
        let core = CoreIdentity::production();
        let incompatible = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
            .parse::<CoreIdentity>()
            .unwrap();
        let daemon = DaemonStateDirectory::open(&provider_state, &core).unwrap();
        let incompatible_daemon =
            DaemonStateDirectory::open(&provider_state, &incompatible).unwrap();

        assert_ne!(first_helper.key, second_helper.key);
        assert_ne!(daemon.path(), incompatible_daemon.path());
        let daemon_name = daemon.path().file_name().unwrap().to_string_lossy();
        assert!(daemon_name.contains(core.as_str()));
        assert!(!daemon_name.contains(&first_helper.key));
        assert!(!daemon_name.contains(&second_helper.key));

        cleanup(&first);
        cleanup(&second);
    }

    #[test]
    fn service_identity_reuses_normalized_paths() {
        let context = test_context("normalized-identity");
        let plain =
            WorkspaceServiceIdentity::new(&context, &context.workspace_root.join("src")).unwrap();
        let dotted =
            WorkspaceServiceIdentity::new(&context, &context.workspace_root.join("src/./"))
                .unwrap();

        assert_eq!(plain.key, dotted.key);
        cleanup(&context);
    }

    /// rule: INV.PERF.WORKSPACE-SERVICE-REUSE
    #[test]
    fn service_record_is_reusable_only_for_matching_live_build_and_paths() {
        let context = test_context("record");
        let source_root = context.workspace_root.join("src");
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        let record = WorkspaceServiceRecord {
            schema_version: SERVICE_SCHEMA_VERSION,
            pid: std::process::id(),
            port: 34567,
            token: "token".to_string(),
            build: CoreIdentity::production().as_str().to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            workspace_root: identity.workspace_root.clone(),
            source_root: identity.source_root.clone(),
            started_at: now_secs_for_test(),
            last_access_at: now_secs_for_test(),
        };

        assert!(record.matches(&identity, CoreIdentity::production().as_str()));

        let mut mismatched_schema = record.clone();
        mismatched_schema.schema_version = SERVICE_SCHEMA_VERSION - 1;
        assert!(!mismatched_schema.matches(&identity, CoreIdentity::production().as_str()));

        let mut mismatched_build = record.clone();
        mismatched_build.build = "another-build".to_string();
        assert!(!mismatched_build.matches(&identity, CoreIdentity::production().as_str()));

        let mut mismatched_source = record;
        mismatched_source.source_root = context.workspace_root.join("other").display().to_string();
        assert!(!mismatched_source.matches(&identity, CoreIdentity::production().as_str()));

        cleanup(&context);
    }

    #[test]
    fn service_record_v2_is_not_reused_after_typed_invalidation_protocol() {
        let context = test_context("typed-invalidation-record-version");
        let source_root = context.workspace_root.join("src");
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        let mut record = test_record(&identity, 34567, CoreIdentity::production().as_str());
        record.schema_version = 2;

        assert!(
            !record.matches(&identity, CoreIdentity::production().as_str()),
            "the pre-typed-invalidation service protocol must not be reused"
        );
        cleanup(&context);
    }

    #[test]
    fn service_record_v3_is_not_reused_after_lossless_worker_timeout_protocol() {
        let context = test_context("lossless-worker-timeout-record-version");
        let source_root = context.workspace_root.join("src");
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        let mut record = test_record(&identity, 34567, CoreIdentity::production().as_str());
        record.schema_version = 3;

        assert!(
            !record.matches(&identity, CoreIdentity::production().as_str()),
            "the millisecond-saturated worker timeout protocol must not be reused"
        );
        cleanup(&context);
    }

    #[test]
    fn service_record_with_shutting_down_ping_is_not_reusable() {
        struct ShuttingDownConnector;
        impl ServiceConnector for ShuttingDownConnector {
            fn send_before(
                &self,
                _record: &WorkspaceServiceRecord,
                _request: ServiceRequest,
                _cancellation: &CancellationToken,
                _deadline: OperationDeadline,
            ) -> Result<ServiceResponse, String> {
                unreachable!("this fixture exercises only finite legacy requests")
            }

            fn send(
                &self,
                _record: &WorkspaceServiceRecord,
                _request: ServiceRequest,
                _cancellation: &CancellationToken,
                _budget: Duration,
            ) -> Result<ServiceResponse, String> {
                Ok(ServiceResponse {
                    ok: true,
                    status: Some("shutting-down".to_string()),
                    ..ServiceResponse::default()
                })
            }

            fn send_operation_before(
                &self,
                _record: &WorkspaceServiceRecord,
                _request: ServiceRequest,
                _cancellation: &CancellationToken,
                _deadline: ProviderDeadline,
            ) -> Result<ServiceResponse, String> {
                unreachable!("this legacy fixture does not use the prepared BSL route")
            }
        }

        let context = test_context("shutting-down-record");
        let identity =
            WorkspaceServiceIdentity::new(&context, &context.workspace_root.join("src")).unwrap();
        let record = test_record(&identity, 34567, CoreIdentity::production().as_str());
        write_record(&identity, record.clone());
        let spawner = RecordingSpawner::default();
        let manager = WorkspaceServiceManager::with_io(
            &ShuttingDownConnector,
            &spawner,
            analysis_composition::service_environment(),
        );

        let replacement = manager.ensure_service(&context, Path::new(&identity.source_root));

        assert_eq!(replacement.unwrap().port, 45678);
        assert_eq!(*spawner.spawns.borrow(), 1);
        cleanup(&context);
    }

    #[test]
    fn spawn_wait_rejects_shutting_down_record() {
        struct ShuttingDownConnector;
        impl ServiceConnector for ShuttingDownConnector {
            fn send_before(
                &self,
                _record: &WorkspaceServiceRecord,
                _request: ServiceRequest,
                _cancellation: &CancellationToken,
                _deadline: OperationDeadline,
            ) -> Result<ServiceResponse, String> {
                unreachable!("this fixture exercises only finite legacy requests")
            }

            fn send(
                &self,
                _record: &WorkspaceServiceRecord,
                _request: ServiceRequest,
                _cancellation: &CancellationToken,
                _budget: Duration,
            ) -> Result<ServiceResponse, String> {
                Ok(ServiceResponse {
                    ok: true,
                    status: Some("shutting-down".to_string()),
                    ..ServiceResponse::default()
                })
            }

            fn send_operation_before(
                &self,
                _record: &WorkspaceServiceRecord,
                _request: ServiceRequest,
                _cancellation: &CancellationToken,
                _deadline: ProviderDeadline,
            ) -> Result<ServiceResponse, String> {
                unreachable!("this legacy fixture does not use the prepared BSL route")
            }
        }
        let context = test_context("spawn-wait-shutting-down");
        let identity =
            WorkspaceServiceIdentity::new(&context, &context.workspace_root.join("src")).unwrap();
        let record = test_record(&identity, 34567, CoreIdentity::production().as_str());
        write_record(&identity, record.clone());

        let result = wait_for_record_with_connector(
            &identity,
            &ShuttingDownConnector,
            record.pid,
            &record.token,
            Duration::from_millis(75),
            &CancellationToken::new(),
        );

        assert!(result.is_err());
        cleanup(&context);
    }

    /// Проводка продуктового запуска: сервисом стартует этот тестовый бинарь,
    /// libtest не понимает `--workspace-service` и сразу выходит. Запуск обязан
    /// назвать выход до готовности, не досиживая срок готовности (#1920).
    #[test]
    fn spawned_service_that_exits_is_reported_before_the_readiness_timeout() {
        assert!(
            std::env::var_os("UNICA_TEST_WORKSPACE_SERVICE_EXE").is_none(),
            "сервисом должен стартовать этот же тестовый бинарь"
        );
        let context = test_context("spawn-exits-before-readiness");
        let manager = WorkspaceServiceManager::new(analysis_composition::service_environment());
        let started = Instant::now();

        let error = manager
            .ensure_service(&context, &context.workspace_root.join("src"))
            .unwrap_err();

        assert!(error.contains("did not become ready"), "{error}");
        assert!(error.contains("exited before readiness"), "{error}");
        assert!(
            started.elapsed() < SERVICE_CONNECT_TIMEOUT,
            "запуск ждал срок готовности после выхода сервиса: {:?}",
            started.elapsed()
        );
        cleanup(&context);
    }

    /// Сервис, умерший до готовности, записи не напишет: ожидание обязано
    /// прекратиться по его выходу, а не досиживать весь срок (#1920).
    #[test]
    fn spawn_wait_stops_once_the_spawned_service_has_exited() {
        let context = test_context("spawn-wait-exited");
        let identity =
            WorkspaceServiceIdentity::new(&context, &context.workspace_root.join("src")).unwrap();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "infrastructure::workspace_services::medium_tests::startup_failure_child_fixture",
                "--ignored",
                "--nocapture",
            ])
            .env("UNICA_STARTUP_FAILURE_CHILD_FIXTURE", "1")
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut child = ManagedStartupChild::spawn_configured(command).unwrap();
        let pid = child.id();
        let started = Instant::now();

        let error = unica_analysis_services::test_support::wait_for_record_while(
            &identity,
            &RecordingConnector::default(),
            CoreIdentity::production().as_str(),
            pid,
            "secret",
            Duration::from_secs(30),
            &CancellationToken::new(),
            &mut || matches!(child.try_wait_status(), Ok(Some(_))),
        )
        .unwrap_err();

        assert!(error.contains("did not become ready"), "{error}");
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "ожидание пережило выход сервиса: {:?}",
            started.elapsed()
        );
        assert!(matches!(child.try_wait_status(), Ok(Some(_))));
        cleanup(&context);
    }

    #[test]
    fn spawn_wait_prioritizes_cancellation_over_its_timeout() {
        let context = test_context("spawn-wait-cancelled");
        let identity =
            WorkspaceServiceIdentity::new(&context, &context.workspace_root.join("src")).unwrap();
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let started = Instant::now();

        let error = wait_for_record_with_connector(
            &identity,
            &RecordingConnector::default(),
            34567,
            "secret",
            SERVICE_CONNECT_TIMEOUT,
            &cancellation,
        )
        .unwrap_err();

        assert!(error.starts_with("cancelled:"), "{error}");
        assert!(started.elapsed() < Duration::from_millis(100));
        cleanup(&context);
    }

    #[test]
    fn failed_spawn_cleanup_reaps_child_and_preserves_replacement_record() {
        let context = test_context("failed-spawn-child-cleanup");
        let source_root = context.workspace_root.join("src");
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();

        let mut owned_command = Command::new(std::env::current_exe().unwrap());
        owned_command
            .args([
                "--exact",
                "infrastructure::workspace_services::medium_tests::spawn_cleanup_child_fixture",
                "--ignored",
                "--nocapture",
            ])
            .env("UNICA_SPAWN_CLEANUP_CHILD_FIXTURE", "1")
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut owned_child = ManagedStartupChild::spawn_configured(owned_command).unwrap();
        thread::sleep(Duration::from_millis(75));
        assert!(owned_child.is_running().unwrap());
        let mut owned_record = test_record(&identity, 34567, CoreIdentity::production().as_str());
        owned_record.pid = owned_child.id();
        owned_record.token = "owned-spawn-token".to_string();
        write_record(&identity, owned_record);

        terminate_failed_workspace_service_spawn(
            &mut owned_child,
            &identity,
            "owned-spawn-token",
            Duration::from_secs(1),
        )
        .unwrap();

        assert!(read_record(&identity).is_none());

        let mut replaced_command = Command::new(std::env::current_exe().unwrap());
        replaced_command
            .args([
                "--exact",
                "infrastructure::workspace_services::medium_tests::spawn_cleanup_child_fixture",
                "--ignored",
                "--nocapture",
            ])
            .env("UNICA_SPAWN_CLEANUP_CHILD_FIXTURE", "1")
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut replaced_child = ManagedStartupChild::spawn_configured(replaced_command).unwrap();
        thread::sleep(Duration::from_millis(75));
        assert!(replaced_child.is_running().unwrap());
        let replacement = test_record(&identity, 45678, CoreIdentity::production().as_str());
        write_record(&identity, replacement.clone());

        terminate_failed_workspace_service_spawn(
            &mut replaced_child,
            &identity,
            "different-spawn-token",
            Duration::from_secs(1),
        )
        .unwrap();

        assert_eq!(read_record(&identity), Some(replacement));
        cleanup(&context);
    }

    /// rule: INV.APP.WORKSPACE-SERVICE-STARTUP-DIAGNOSTIC
    #[test]
    fn startup_failure_reports_exit_status_and_bounded_stderr_tail() {
        let context = test_context("startup-failure-diagnostic");
        let identity =
            WorkspaceServiceIdentity::new(&context, &context.workspace_root.join("src")).unwrap();
        let stderr_path = identity.service_dir.join("service.stderr.log");
        fs::create_dir_all(&identity.service_dir).unwrap();
        let stderr = fs::File::create(&stderr_path).unwrap();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "infrastructure::workspace_services::medium_tests::startup_failure_child_fixture",
                "--ignored",
                "--nocapture",
            ])
            .env("UNICA_STARTUP_FAILURE_CHILD_FIXTURE", "1")
            .stdout(Stdio::null())
            .stderr(Stdio::from(stderr));
        let mut child = ManagedStartupChild::spawn_configured(command).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while child.is_running().unwrap() {
            assert!(Instant::now() < deadline, "fixture did not exit");
            thread::yield_now();
        }

        let error = workspace_service_startup_failure(
            "workspace service did not become ready",
            &mut child,
            &stderr_path,
        );

        assert!(error.contains("exited before readiness"), "{error}");
        assert!(error.contains("23"), "{error}");
        assert!(error.contains("issue-339-startup-tail-marker"), "{error}");
        assert!(!error.contains("issue-339-startup-head-marker"), "{error}");
        assert!(error.len() <= SERVICE_STARTUP_STDERR_TAIL_LIMIT as usize + 512);
        child.terminate_bounded(Duration::from_secs(1)).unwrap();
        cleanup(&context);
    }

    /// rule: INV.APP.WORKSPACE-SERVICE-STARTUP-DIAGNOSTIC
    #[test]
    fn startup_stderr_tail_stays_bounded_after_lossy_utf8_decoding() {
        let context = test_context("startup-stderr-invalid-utf8");
        let stderr_path = context.cache_root.join("service.stderr.log");
        fs::create_dir_all(&context.cache_root).unwrap();
        let marker = b"issue-339-startup-tail-marker";
        let mut stderr = vec![0xff; SERVICE_STARTUP_STDERR_TAIL_LIMIT as usize];
        stderr.extend_from_slice(marker);
        fs::write(&stderr_path, stderr).unwrap();

        let rendered = read_workspace_service_stderr_tail(&stderr_path).unwrap();

        assert!(rendered.contains("issue-339-startup-tail-marker"));
        assert!(
            rendered.len() <= SERVICE_STARTUP_STDERR_TAIL_LIMIT as usize,
            "lossy UTF-8 expanded the diagnostic to {} bytes",
            rendered.len()
        );
        cleanup(&context);
    }

    /// rule: INV.APP.WORKSPACE-SERVICE-STARTUP-DIAGNOSTIC
    #[test]
    fn startup_failure_reports_live_child_without_assuming_deadline_expired() {
        let context = test_context("startup-failure-live-child");
        let identity =
            WorkspaceServiceIdentity::new(&context, &context.workspace_root.join("src")).unwrap();
        let stderr_path = identity.service_dir.join("service.stderr.log");
        fs::create_dir_all(&identity.service_dir).unwrap();
        let stderr = fs::File::create(&stderr_path).unwrap();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "infrastructure::workspace_services::medium_tests::spawn_cleanup_child_fixture",
                "--ignored",
                "--nocapture",
            ])
            .env("UNICA_SPAWN_CLEANUP_CHILD_FIXTURE", "1")
            .stdout(Stdio::null())
            .stderr(Stdio::from(stderr));
        let mut child = ManagedStartupChild::spawn_configured(command).unwrap();
        thread::sleep(Duration::from_millis(75));
        assert!(child.is_running().unwrap());

        let error = workspace_service_startup_failure(
            "cancelled: workspace service readiness was cancelled",
            &mut child,
            &stderr_path,
        );

        assert!(
            error.contains("was still running when readiness failed"),
            "{error}"
        );
        assert!(!error.contains("readiness deadline"), "{error}");
        child.terminate_bounded(Duration::from_secs(1)).unwrap();
        cleanup(&context);
    }

    #[test]
    #[ignore = "helper process: spawned by the startup failure tests with UNICA_STARTUP_FAILURE_CHILD_FIXTURE; not a standalone test"]
    fn startup_failure_child_fixture() {
        std::env::var_os("UNICA_STARTUP_FAILURE_CHILD_FIXTURE")
            .expect("helper process: the parent test sets the fixture marker");
        eprintln!("issue-339-startup-head-marker");
        eprintln!(
            "{}",
            "x".repeat(SERVICE_STARTUP_STDERR_TAIL_LIMIT as usize + 1024)
        );
        eprintln!("issue-339-startup-tail-marker");
        std::process::exit(23);
    }

    #[test]
    #[ignore = "helper process: spawned by the spawn cleanup tests with UNICA_SPAWN_CLEANUP_CHILD_FIXTURE; not a standalone test"]
    fn spawn_cleanup_child_fixture() {
        std::env::var_os("UNICA_SPAWN_CLEANUP_CHILD_FIXTURE")
            .expect("helper process: the parent test sets the fixture marker");
        thread::sleep(Duration::from_secs(30));
    }

    /// rule: INV.RUNTIME.SERVICE-LIFETIME
    #[test]
    fn service_config_uses_defaults_and_env_overrides() {
        std::env::remove_var("UNICA_WORKSPACE_SERVICE_IDLE_SECS");
        let defaults = WorkspaceServiceConfig::from_env();
        assert_eq!(defaults.idle_secs, 7200);

        std::env::set_var("UNICA_WORKSPACE_SERVICE_IDLE_SECS", "10");
        let configured = WorkspaceServiceConfig::from_env();
        assert_eq!(configured.idle_secs, 10);

        std::env::remove_var("UNICA_WORKSPACE_SERVICE_IDLE_SECS");
    }

    /// rule: INV.RUNTIME.SERVICE-TOKEN
    #[test]
    fn service_protocol_rejects_invalid_token_and_accepts_ping() {
        let context = test_context("protocol");
        let identity =
            WorkspaceServiceIdentity::new(&context, &context.workspace_root.join("src")).unwrap();
        let record = test_record(&identity, 34567, CoreIdentity::production().as_str());
        let runtime = new_runtime(identity, &record);

        let invalid = unica_analysis_services::test_support::error_response(
            runtime.test_authenticate("wrong").unwrap_err(),
        );
        assert!(!invalid.ok);
        assert_eq!(
            invalid.error.as_deref(),
            Some("invalid workspace service token")
        );

        runtime.test_authenticate("secret").unwrap();
        let valid = runtime.test_ping();
        assert!(valid.ok);
        assert_eq!(valid.status.as_deref(), Some("alive"));

        cleanup(&context);
    }

    /// rule: INV.PERF.WORKSPACE-SERVICE-REUSE
    #[test]
    fn manager_reuses_matching_live_record_without_spawning() {
        let context = test_context("reuse");
        let source_root = context.workspace_root.join("src");
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        write_record(
            &identity,
            test_record(&identity, 34567, CoreIdentity::production().as_str()),
        );
        let connector = RecordingConnector {
            ping_ok: true,
            ..Default::default()
        };
        let spawner = RecordingSpawner::default();
        let manager = WorkspaceServiceManager::with_io(
            &connector,
            &spawner,
            analysis_composition::service_environment(),
        );

        let record = manager.ensure_service(&context, &source_root).unwrap();

        assert_eq!(record.port, 34567);
        assert_eq!(*connector.pings.borrow(), 1);
        assert_eq!(*spawner.spawns.borrow(), 0);
        cleanup(&context);
    }

    #[test]
    fn manager_spawns_when_record_is_unreachable_or_version_mismatched() {
        let context = test_context("spawn");
        let source_root = context.workspace_root.join("src");
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        write_record(&identity, test_record(&identity, 34567, "older"));
        let connector = RecordingConnector::default();
        let spawner = RecordingSpawner::default();
        let manager = WorkspaceServiceManager::with_io(
            &connector,
            &spawner,
            analysis_composition::service_environment(),
        );

        let record = manager.ensure_service(&context, &source_root).unwrap();

        assert_eq!(record.port, 45678);
        assert_eq!(*connector.pings.borrow(), 0);
        assert_eq!(*spawner.spawns.borrow(), 1);
        cleanup(&context);
    }

    /// Одна служба на проект: служба другой сборки — и запись прежней схемы,
    /// опознанная по версии пакета, — гасится и заменяется своей, а не
    /// переиспользуется и не остаётся жить рядом с новой.
    #[test]
    fn manager_replaces_a_service_of_another_build_and_shuts_it_down() {
        let context = test_context("another-build");
        let source_root = context.workspace_root.join("src");
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        let package_version_record = serde_json::json!({
            "schema_version": 4,
            "pid": std::process::id(),
            "port": 34567,
            "token": "secret",
            "version": env!("CARGO_PKG_VERSION"),
            "workspace_root": identity.workspace_root,
            "source_root": identity.source_root,
            "started_at": now_secs_for_test(),
            "last_access_at": now_secs_for_test(),
        });
        // Same package version, another build: the version used to match.
        let same_version_record = test_record(&identity, 34567, "another-build");
        assert_eq!(same_version_record.version, env!("CARGO_PKG_VERSION"));
        for foreign in [
            serde_json::to_value(same_version_record).unwrap(),
            package_version_record,
        ] {
            fs::create_dir_all(&identity.service_dir).unwrap();
            fs::write(identity.test_record_path(), foreign.to_string()).unwrap();
            let connector = RecordingConnector {
                ping_ok: true,
                ..Default::default()
            };
            let spawner = RecordingSpawner::default();
            let manager = WorkspaceServiceManager::with_io(
                &connector,
                &spawner,
                analysis_composition::service_environment(),
            );

            let record = manager.ensure_service(&context, &source_root).unwrap();

            assert_eq!(record.port, 45678, "{foreign}");
            assert_eq!(*spawner.spawns.borrow(), 1, "{foreign}");
            assert!(
                connector
                    .requests
                    .borrow()
                    .iter()
                    .any(|kind| matches!(kind, ServiceRequestKind::Shutdown)),
                "the foreign service must be told to stop: {foreign}"
            );
        }
        cleanup(&context);
    }

    /// Запись новой схемы читается и бинарём схемы 4: иначе он не сможет ни
    /// переиспользовать, ни погасить этот сервис и оставит его жить рядом.
    #[test]
    fn a_schema_four_binary_can_still_read_and_reject_the_record() {
        #[derive(serde::Deserialize)]
        #[allow(dead_code)]
        struct SchemaFourRecord {
            schema_version: u32,
            pid: u32,
            port: u16,
            token: String,
            version: String,
            workspace_root: String,
            source_root: String,
            started_at: u64,
            last_access_at: u64,
        }
        let context = test_context("schema-four-reader");
        let source_root = context.workspace_root.join("src");
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        let record = test_record(&identity, 34567, CoreIdentity::production().as_str());

        let parsed: SchemaFourRecord =
            serde_json::from_value(serde_json::to_value(&record).unwrap()).unwrap();

        assert_ne!(parsed.schema_version, 4, "a schema-4 reader must reject it");
        assert!(!parsed.token.is_empty() && parsed.port > 0);
        cleanup(&context);
    }

    /// rule: INV.RUNTIME.SERVICE-STARTUP-OWNERSHIP
    #[test]
    fn manager_waits_for_peer_spawn_lock_release_before_reusing_record() {
        let context = test_context("peer-lock");
        let source_root = context.workspace_root.join("src");
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        let spawn_lock = acquire_spawn_lock(&identity).unwrap().unwrap();
        write_record(
            &identity,
            test_record(&identity, 34567, CoreIdentity::production().as_str()),
        );
        let releaser = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(150));
            drop(spawn_lock);
        });
        let connector = RecordingConnector {
            ping_ok: true,
            ..Default::default()
        };
        let spawner = RecordingSpawner::default();
        let manager = WorkspaceServiceManager::with_io(
            &connector,
            &spawner,
            analysis_composition::service_environment(),
        );
        let started = Instant::now();

        let record = manager.ensure_service(&context, &source_root).unwrap();

        releaser.join().unwrap();
        assert!(started.elapsed() >= Duration::from_millis(125));
        assert_eq!(record.port, 34567);
        assert_eq!(*connector.pings.borrow(), 1);
        assert_eq!(*spawner.spawns.borrow(), 0);
        cleanup(&context);
    }

    /// rule: INV.RUNTIME.SERVICE-STARTUP-OWNERSHIP
    #[test]
    fn manager_spawn_lock_wait_observes_cancellation_and_shared_deadline() {
        let context = test_context("peer-lock-deadline");
        let source_root = context.workspace_root.join("src");
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        let spawn_lock = acquire_spawn_lock(&identity).unwrap().unwrap();
        let connector = RecordingConnector::default();
        let spawner = RecordingSpawner::default();
        let manager = WorkspaceServiceManager::with_io(
            &connector,
            &spawner,
            analysis_composition::service_environment(),
        );

        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let cancelled = manager
            .ensure_service_cancellable(&context, &source_root, &cancellation)
            .unwrap_err();
        assert!(cancelled.starts_with("cancelled:"), "{cancelled}");

        let started = Instant::now();
        let timed_out = manager
            .test_ensure_service_cancellable_with_deadline(
                &context,
                &source_root,
                &CancellationToken::new(),
                &WorkspaceServiceCallDeadline::new(Duration::from_millis(75)),
            )
            .unwrap_err();
        assert!(timed_out.starts_with("timeout:"), "{timed_out}");
        assert!(started.elapsed() < Duration::from_millis(300));
        assert!(connector.requests.borrow().is_empty());
        assert_eq!(*spawner.spawns.borrow(), 0);

        drop(spawn_lock);
        cleanup(&context);
    }

    #[test]
    fn workspace_service_deadline_reports_actual_request_budget() {
        let deadline =
            WorkspaceServiceCallDeadline::test_with_started(Instant::now(), Duration::ZERO);

        let error = deadline.remaining(&CancellationToken::new()).unwrap_err();

        assert_eq!(
            error,
            "timeout: workspace service request exceeded 0 seconds"
        );
    }

    #[test]
    fn workspace_service_timeout_reports_a_subsecond_budget_in_milliseconds() {
        assert_eq!(
            workspace_service_request_timeout_error(Duration::from_millis(750)),
            "timeout: workspace service request exceeded 750 milliseconds"
        );
    }

    #[test]
    fn spawn_lock_contention_classifier_uses_fs2_platform_error() {
        assert!(spawn_lock_is_contended(&fs2::lock_contended_error()));
        assert!(spawn_lock_is_contended(&io::Error::from(
            ErrorKind::WouldBlock
        )));
        assert!(!spawn_lock_is_contended(&io::Error::from(
            ErrorKind::PermissionDenied
        )));
    }

    /// rule: INV.RUNTIME.SERVICE-STARTUP-OWNERSHIP
    #[test]
    fn spawn_wait_ignores_live_record_not_owned_by_spawned_child() {
        let context = test_context("spawn-wait-owned-record");
        let identity =
            WorkspaceServiceIdentity::new(&context, &context.workspace_root.join("src")).unwrap();
        write_record(
            &identity,
            test_record(&identity, 34567, CoreIdentity::production().as_str()),
        );
        let connector = RecordingConnector {
            ping_ok: true,
            ..RecordingConnector::default()
        };

        let result = wait_for_record_with_connector(
            &identity,
            &connector,
            45678,
            "new-spawn-token",
            Duration::from_millis(75),
            &CancellationToken::new(),
        );

        assert!(result.is_err());
        assert!(connector.requests.borrow().is_empty());
        cleanup(&context);
    }

    /// rule: INV.RUNTIME.SERVICE-OPERATION-CANCELLATION
    #[test]
    fn manager_generates_unique_uuid_operation_ids() {
        let context = test_context("operation-ids");
        let source_root = context.workspace_root.join("src");
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        write_record(
            &identity,
            test_record(&identity, 34567, CoreIdentity::production().as_str()),
        );
        let connector = RecordingConnector {
            ping_ok: true,
            ..Default::default()
        };
        let spawner = RecordingSpawner::default();
        let manager = WorkspaceServiceManager::with_io(
            &connector,
            &spawner,
            analysis_composition::service_environment(),
        );

        manager
            .call_bsl_mcp(
                &context,
                &source_root,
                "search",
                json!({}),
                Duration::from_secs(1),
            )
            .unwrap();
        manager
            .call_bsl_mcp(
                &context,
                &source_root,
                "search",
                json!({}),
                Duration::from_secs(1),
            )
            .unwrap();

        let ids = connector
            .requests
            .borrow()
            .iter()
            .filter_map(ServiceRequestKind::operation_id)
            .map(str::to_string)
            .collect::<Vec<_>>();
        assert_eq!(ids.len(), 2);
        assert_eq!(
            ids.iter().collect::<std::collections::HashSet<_>>().len(),
            2
        );
        assert!(ids
            .iter()
            .map(|id| Uuid::parse_str(id).unwrap())
            .all(|id| id.get_version_num() == 4));
        cleanup(&context);
    }

    /// rule: INV.PERF.SERVICE-CALLER-BUDGET
    #[test]
    fn bsl_request_preserves_budget_beyond_u64_max_milliseconds() {
        let context = test_context("bsl-caller-budget-beyond-u64-millis");
        let source_root = context.workspace_root.join("src");
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        write_record(
            &identity,
            test_record(&identity, 34567, CoreIdentity::production().as_str()),
        );
        let connector = RecordingConnector {
            ping_ok: true,
            ..RecordingConnector::default()
        };
        let spawner = RecordingSpawner::default();
        let manager = WorkspaceServiceManager::with_io(
            &connector,
            &spawner,
            analysis_composition::service_environment(),
        );

        manager
            .call_bsl_mcp_cancellable_with_budget(
                &context,
                &source_root,
                WorkspaceServiceBslCall::new(
                    "search",
                    json!({"query": "needle"}),
                    Duration::from_secs(i64::MAX as u64),
                    Duration::from_secs(i64::MAX as u64),
                ),
                &CancellationToken::new(),
            )
            .unwrap();

        let budget = *connector.budgets.borrow().last().unwrap();
        let request = serde_json::to_value(connector.requests.borrow().last().unwrap()).unwrap();
        let wire_timeout = Duration::new(
            request["timeout_seconds"].as_u64().unwrap(),
            request["timeout_nanos"].as_u64().unwrap() as u32,
        );
        cleanup(&context);
        assert!(wire_timeout > Duration::from_millis(u64::MAX));
        assert_eq!(wire_timeout, budget);
    }

    /// rule: INV.RUNTIME.SERVICE-READ-RETRY
    #[test]
    fn manager_retries_bsl_request_once_after_transport_reset() {
        let context = test_context("bsl-transport-reset-retry");
        let source_root = context.workspace_root.join("src");
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        write_record(
            &identity,
            test_record(&identity, 34567, CoreIdentity::production().as_str()),
        );
        let connector = ResetBslConnector {
            recover: true,
            ..ResetBslConnector::default()
        };
        let spawner = RecordingSpawner::default();
        let manager = WorkspaceServiceManager::with_io(
            &connector,
            &spawner,
            analysis_composition::service_environment(),
        );

        let output = manager
            .call_bsl_mcp(
                &context,
                &source_root,
                "diagnostics",
                json!({"mode": "file", "path": "CommonModules/Example/Module.bsl"}),
                Duration::from_secs(1),
            )
            .unwrap();

        assert_eq!(output.result_text, "recovered");
        assert_eq!(*connector.pings.borrow(), 2);
        assert_eq!(*connector.bsl_calls.borrow(), 2);
        let operation_ids = connector.operation_ids.borrow();
        assert_eq!(operation_ids.len(), 2);
        assert_ne!(operation_ids[0], operation_ids[1]);
        assert_eq!(*spawner.spawns.borrow(), 0);
        cleanup(&context);
    }

    #[test]
    fn manager_shares_one_deadline_across_late_transport_retry() {
        let context = test_context("bsl-transport-shared-deadline");
        let source_root = context.workspace_root.join("src");
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        write_record(
            &identity,
            test_record(&identity, 34567, CoreIdentity::production().as_str()),
        );
        let connector = ResetBslConnector {
            recover: true,
            first_bsl_delay: Duration::from_millis(250),
            ..ResetBslConnector::default()
        };
        let spawner = RecordingSpawner::default();
        let manager = WorkspaceServiceManager::with_io(
            &connector,
            &spawner,
            analysis_composition::service_environment(),
        );

        let output = manager
            .call_bsl_mcp_cancellable_with_budget(
                &context,
                &source_root,
                WorkspaceServiceBslCall::new(
                    "diagnostics",
                    json!({"mode": "file"}),
                    Duration::from_secs(120),
                    Duration::from_secs(1),
                ),
                &CancellationToken::new(),
            )
            .unwrap();

        assert_eq!(output.result_text, "recovered");
        let ping_budgets = connector.ping_budgets.borrow();
        let bsl_budgets = connector.bsl_budgets.borrow();
        assert_eq!(ping_budgets.len(), 2);
        assert_eq!(bsl_budgets.len(), 2);
        assert!(bsl_budgets[0] <= ping_budgets[0]);
        assert!(ping_budgets[1] < bsl_budgets[0]);
        assert!(bsl_budgets[1] <= ping_budgets[1]);
        let timeouts = connector.bsl_timeouts.borrow();
        assert_eq!(timeouts.len(), 2);
        assert!(timeouts
            .iter()
            .all(|timeout| *timeout <= Duration::from_secs(1)));
        assert!(timeouts[1] < timeouts[0]);
        assert_eq!(*spawner.spawns.borrow(), 0);
        cleanup(&context);
    }

    /// rule: INV.RUNTIME.SERVICE-READ-RETRY
    #[test]
    fn manager_passes_remaining_budget_and_fresh_token_to_retry_spawn() {
        let context = test_context("bsl-retry-spawn-budget-token");
        let source_root = context.workspace_root.join("src");
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        write_record(
            &identity,
            test_record(&identity, 34567, CoreIdentity::production().as_str()),
        );
        let connector = ResetBslConnector {
            recover: true,
            restart_after_reset: true,
            first_bsl_delay: Duration::from_millis(250),
            ..ResetBslConnector::default()
        };
        let spawner = RecordingSpawner::default();
        let manager = WorkspaceServiceManager::with_io(
            &connector,
            &spawner,
            analysis_composition::service_environment(),
        );

        let output = manager
            .call_bsl_mcp_cancellable_with_budget(
                &context,
                &source_root,
                WorkspaceServiceBslCall::new(
                    "diagnostics",
                    json!({"mode": "file"}),
                    Duration::from_secs(120),
                    Duration::from_secs(1),
                ),
                &CancellationToken::new(),
            )
            .unwrap();

        assert_eq!(output.result_text, "recovered");
        assert_eq!(*spawner.spawns.borrow(), 1);
        let spawn_budgets = spawner.budgets.borrow();
        assert_eq!(spawn_budgets.len(), 1);
        assert!(spawn_budgets[0] < Duration::from_secs(1));
        let spawned_tokens = spawner.tokens.borrow();
        let bsl_record_tokens = connector.bsl_record_tokens.borrow();
        let bsl_request_tokens = connector.bsl_request_tokens.borrow();
        assert_eq!(bsl_record_tokens.len(), 2);
        assert_eq!(bsl_record_tokens[0], "secret");
        assert_eq!(bsl_record_tokens[1], spawned_tokens[0]);
        assert_eq!(*bsl_request_tokens, *bsl_record_tokens);
        cleanup(&context);
    }

    #[test]
    fn manager_bounds_delayed_retry_spawn_by_remaining_deadline() {
        let context = test_context("bsl-retry-delayed-spawn-budget");
        let source_root = context.workspace_root.join("src");
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        write_record(
            &identity,
            test_record(&identity, 34567, CoreIdentity::production().as_str()),
        );
        let connector = ResetBslConnector {
            restart_after_reset: true,
            first_bsl_delay: Duration::from_millis(100),
            ..ResetBslConnector::default()
        };
        let spawner = RecordingSpawner {
            delay: Duration::from_secs(2),
            ..RecordingSpawner::default()
        };
        let manager = WorkspaceServiceManager::with_io(
            &connector,
            &spawner,
            analysis_composition::service_environment(),
        );
        let started = Instant::now();

        let error = manager
            .call_bsl_mcp_cancellable_with_budget(
                &context,
                &source_root,
                WorkspaceServiceBslCall::new(
                    "diagnostics",
                    json!({"mode": "file"}),
                    Duration::from_secs(120),
                    Duration::from_secs(1),
                ),
                &CancellationToken::new(),
            )
            .unwrap_err();

        assert!(error.starts_with("timeout:"), "{error}");
        assert!(started.elapsed() < Duration::from_millis(1_500));
        assert_eq!(*spawner.spawns.borrow(), 1);
        assert!(spawner.budgets.borrow()[0] < Duration::from_secs(1));
        cleanup(&context);
    }

    #[test]
    fn manager_does_not_spawn_when_retry_shutdown_observes_cancellation() {
        let context = test_context("bsl-retry-cancel-before-spawn");
        let source_root = context.workspace_root.join("src");
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        write_record(
            &identity,
            test_record(&identity, 34567, CoreIdentity::production().as_str()),
        );
        let connector = ResetBslConnector {
            restart_after_reset: true,
            cancel_on_shutdown: true,
            ..ResetBslConnector::default()
        };
        let spawner = RecordingSpawner::default();
        let manager = WorkspaceServiceManager::with_io(
            &connector,
            &spawner,
            analysis_composition::service_environment(),
        );
        let cancellation = CancellationToken::new();

        let error = manager
            .call_bsl_mcp_cancellable_with_budget(
                &context,
                &source_root,
                WorkspaceServiceBslCall::new(
                    "diagnostics",
                    json!({}),
                    Duration::from_secs(120),
                    Duration::from_secs(1),
                ),
                &cancellation,
            )
            .unwrap_err();

        assert!(error.starts_with("cancelled:"), "{error}");
        assert_eq!(*spawner.spawns.borrow(), 0);
        cleanup(&context);
    }

    #[test]
    fn manager_prioritizes_cancellation_over_exhausted_retry_budget() {
        let context = test_context("bsl-cancel-over-zero-budget");
        let source_root = context.workspace_root.join("src");
        let connector = ResetBslConnector::default();
        let spawner = RecordingSpawner::default();
        let manager = WorkspaceServiceManager::with_io(
            &connector,
            &spawner,
            analysis_composition::service_environment(),
        );
        let cancellation = CancellationToken::new();
        cancellation.cancel();

        let error = manager
            .call_bsl_mcp_cancellable_with_budget(
                &context,
                &source_root,
                WorkspaceServiceBslCall::new(
                    "diagnostics",
                    json!({}),
                    Duration::from_secs(120),
                    Duration::ZERO,
                ),
                &cancellation,
            )
            .unwrap_err();

        assert!(error.starts_with("cancelled:"), "{error}");
        assert_eq!(*connector.pings.borrow(), 0);
        assert_eq!(*connector.bsl_calls.borrow(), 0);
        assert_eq!(*spawner.spawns.borrow(), 0);
        cleanup(&context);
    }

    #[test]
    fn manager_rejects_exhausted_budget_without_resetting_it() {
        let context = test_context("bsl-zero-budget");
        let source_root = context.workspace_root.join("src");
        let connector = ResetBslConnector::default();
        let spawner = RecordingSpawner::default();
        let manager = WorkspaceServiceManager::with_io(
            &connector,
            &spawner,
            analysis_composition::service_environment(),
        );

        let error = manager
            .call_bsl_mcp_cancellable_with_budget(
                &context,
                &source_root,
                WorkspaceServiceBslCall::new(
                    "diagnostics",
                    json!({}),
                    Duration::from_secs(120),
                    Duration::ZERO,
                ),
                &CancellationToken::new(),
            )
            .unwrap_err();

        assert!(error.starts_with("timeout:"), "{error}");
        assert_eq!(*connector.pings.borrow(), 0);
        assert_eq!(*connector.bsl_calls.borrow(), 0);
        assert_eq!(*spawner.spawns.borrow(), 0);
        cleanup(&context);
    }

    /// rule: INV.RUNTIME.SERVICE-READ-RETRY
    #[test]
    fn manager_stops_after_one_transport_retry() {
        let context = test_context("bsl-transport-reset-twice");
        let source_root = context.workspace_root.join("src");
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        write_record(
            &identity,
            test_record(&identity, 34567, CoreIdentity::production().as_str()),
        );
        let connector = ResetBslConnector::default();
        let spawner = RecordingSpawner::default();
        let manager = WorkspaceServiceManager::with_io(
            &connector,
            &spawner,
            analysis_composition::service_environment(),
        );

        let error = manager
            .call_bsl_mcp(
                &context,
                &source_root,
                "diagnostics",
                json!({"mode": "file"}),
                Duration::from_secs(1),
            )
            .unwrap_err();

        assert_eq!(
            error,
            "failed to read workspace service response: Connection reset by peer (os error 54)"
        );
        assert_eq!(*connector.pings.borrow(), 2);
        assert_eq!(*connector.bsl_calls.borrow(), 2);
        assert_eq!(*spawner.spawns.borrow(), 0);
        cleanup(&context);
    }

    /// rule: INV.RUNTIME.SERVICE-READ-RETRY
    #[test]
    fn manager_does_not_retry_unknown_bsl_tool_after_ambiguous_reset() {
        let context = test_context("unknown-bsl-tool-no-retry");
        let source_root = context.workspace_root.join("src");
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        write_record(
            &identity,
            test_record(&identity, 34567, CoreIdentity::production().as_str()),
        );
        let connector = ResetBslConnector::default();
        let spawner = RecordingSpawner::default();
        let manager = WorkspaceServiceManager::with_io(
            &connector,
            &spawner,
            analysis_composition::service_environment(),
        );

        let error = manager
            .call_bsl_mcp(
                &context,
                &source_root,
                "future_mutation",
                json!({}),
                Duration::from_secs(1),
            )
            .unwrap_err();

        assert!(error.starts_with("failed to read workspace service response:"));
        assert_eq!(*connector.pings.borrow(), 1);
        assert_eq!(*connector.bsl_calls.borrow(), 1);
        assert_eq!(*spawner.spawns.borrow(), 0);
        cleanup(&context);
    }

    /// rule: INV.RUNTIME.SERVICE-READ-RETRY
    #[test]
    fn manager_does_not_retry_typed_bsl_failure() {
        let context = test_context("typed-bsl-failure-no-retry");
        let source_root = context.workspace_root.join("src");
        let identity = WorkspaceServiceIdentity::new(&context, &source_root).unwrap();
        write_record(
            &identity,
            test_record(&identity, 34567, CoreIdentity::production().as_str()),
        );
        let connector = TypedFailureBslConnector::default();
        let spawner = RecordingSpawner::default();
        let manager = WorkspaceServiceManager::with_io(
            &connector,
            &spawner,
            analysis_composition::service_environment(),
        );

        let error = manager
            .call_bsl_mcp(
                &context,
                &source_root,
                "diagnostics",
                json!({"mode": "file"}),
                Duration::from_secs(1),
            )
            .unwrap_err();

        assert_eq!(error, "typed analyzer failure");
        assert_eq!(*connector.pings.borrow(), 1);
        assert_eq!(*connector.bsl_calls.borrow(), 1);
        assert_eq!(*spawner.spawns.borrow(), 0);
        cleanup(&context);
    }

    /// rule: INV.RUNTIME.SERVICE-READ-RETRY
    #[test]
    fn transport_retry_classifier_excludes_terminal_errors() {
        assert_eq!(duration_timeout_millis(Duration::ZERO), 1);
        assert_eq!(duration_timeout_millis(Duration::from_nanos(1)), 1);
        assert_eq!(duration_timeout_millis(Duration::from_secs(1)), 1_000);
        assert_eq!(
            duration_timeout_millis(Duration::from_secs(1) + Duration::from_nanos(1)),
            1_001
        );
        assert!(is_retry_safe_bsl_mcp_tool("diagnostics"));
        assert!(is_retry_safe_bsl_mcp_tool("graph"));
        assert!(!is_retry_safe_bsl_mcp_tool("future_mutation"));
        for error in [
            "failed to connect workspace service: refused",
            "failed to set workspace service read timeout: invalid argument",
            "failed to set workspace service write timeout: invalid argument",
            "failed to write workspace service request: broken pipe",
            "failed to flush workspace service request: broken pipe",
            "workspace service disconnected before responding",
            "failed to read workspace service response: connection reset",
        ] {
            assert!(
                is_retryable_workspace_service_transport_error(error),
                "{error}"
            );
        }
        assert!(!is_retryable_workspace_service_transport_error(
            "timeout: workspace service request exceeded 60 seconds"
        ));
        assert!(!is_retryable_workspace_service_transport_error(
            "cancelled: workspace service operation stopped"
        ));
        assert!(!is_retryable_workspace_service_transport_error(
            "invalid workspace service response: expected value"
        ));
    }

    pub(super) fn test_context(name: &str) -> WorkspaceContext {
        let root = std::env::temp_dir().join(format!(
            "unica-workspace-service-{name}-{}",
            std::process::id()
        ));
        let workspace = root.join("workspace");
        fs::create_dir_all(workspace.join("src/CommonModules")).unwrap();
        fs::create_dir_all(workspace.join("extension/CommonModules")).unwrap();
        WorkspaceContext {
            cwd: workspace.clone(),
            workspace_root: workspace.clone(),
            cache_root: root.join("cache"),
            workspace_epoch: 1,
        }
    }

    pub(super) fn cleanup(context: &WorkspaceContext) {
        let _ = fs::remove_dir_all(context.cache_root.parent().unwrap_or(&context.cache_root));
    }

    fn write_record(identity: &WorkspaceServiceIdentity, record: WorkspaceServiceRecord) {
        super::write_record(identity, &record).unwrap();
    }

    #[derive(Default)]
    struct RecordingConnector {
        ping_ok: bool,
        pings: std::cell::RefCell<u32>,
        requests: std::cell::RefCell<Vec<ServiceRequestKind>>,
        budgets: std::cell::RefCell<Vec<Duration>>,
    }

    #[derive(Default)]
    struct ResetBslConnector {
        recover: bool,
        restart_after_reset: bool,
        cancel_on_shutdown: bool,
        first_bsl_delay: Duration,
        pings: std::cell::RefCell<u32>,
        bsl_calls: std::cell::RefCell<u32>,
        operation_ids: std::cell::RefCell<Vec<String>>,
        ping_budgets: std::cell::RefCell<Vec<Duration>>,
        bsl_budgets: std::cell::RefCell<Vec<Duration>>,
        bsl_timeouts: std::cell::RefCell<Vec<Duration>>,
        bsl_record_tokens: std::cell::RefCell<Vec<String>>,
        bsl_request_tokens: std::cell::RefCell<Vec<String>>,
    }

    impl ServiceConnector for ResetBslConnector {
        fn send_before(
            &self,
            _record: &WorkspaceServiceRecord,
            _request: ServiceRequest,
            _cancellation: &CancellationToken,
            _deadline: OperationDeadline,
        ) -> Result<ServiceResponse, String> {
            unreachable!("this fixture exercises only finite legacy requests")
        }

        fn send(
            &self,
            record: &WorkspaceServiceRecord,
            request: ServiceRequest,
            cancellation: &CancellationToken,
            budget: Duration,
        ) -> Result<ServiceResponse, String> {
            let request_token = request.token;
            match request.kind {
                ServiceRequestKind::Ping => {
                    *self.pings.borrow_mut() += 1;
                    self.ping_budgets.borrow_mut().push(budget);
                    if self.restart_after_reset && *self.bsl_calls.borrow() > 0 {
                        return Err(
                            "failed to connect workspace service: connection refused".to_string()
                        );
                    }
                    Ok(ServiceResponse {
                        ok: true,
                        status: Some("alive".to_string()),
                        ..ServiceResponse::default()
                    })
                }
                ServiceRequestKind::BslMcp {
                    operation_id,
                    timeout_seconds,
                    timeout_nanos,
                    ..
                } => {
                    self.operation_ids.borrow_mut().push(operation_id);
                    self.bsl_budgets.borrow_mut().push(budget);
                    self.bsl_timeouts
                        .borrow_mut()
                        .push(Duration::new(timeout_seconds, timeout_nanos));
                    self.bsl_record_tokens
                        .borrow_mut()
                        .push(record.token.clone());
                    self.bsl_request_tokens.borrow_mut().push(request_token);
                    let mut calls = self.bsl_calls.borrow_mut();
                    *calls += 1;
                    if *calls == 1 && !self.first_bsl_delay.is_zero() {
                        thread::sleep(self.first_bsl_delay);
                    }
                    if *calls > 1 && self.recover {
                        Ok(ServiceResponse {
                            ok: true,
                            result_text: Some("recovered".to_string()),
                            ..ServiceResponse::default()
                        })
                    } else {
                        Err(
                            "failed to read workspace service response: Connection reset by peer (os error 54)"
                                .to_string(),
                        )
                    }
                }
                ServiceRequestKind::Shutdown => {
                    if self.cancel_on_shutdown {
                        cancellation.cancel();
                    }
                    Ok(ServiceResponse {
                        ok: true,
                        shutdown: true,
                        ..ServiceResponse::default()
                    })
                }
                _ => panic!("unexpected request kind"),
            }
        }

        fn send_operation_before(
            &self,
            _record: &WorkspaceServiceRecord,
            _request: ServiceRequest,
            _cancellation: &CancellationToken,
            _deadline: ProviderDeadline,
        ) -> Result<ServiceResponse, String> {
            unreachable!("this legacy fixture does not use the prepared BSL route")
        }
    }

    #[derive(Default)]
    struct TypedFailureBslConnector {
        pings: std::cell::RefCell<u32>,
        bsl_calls: std::cell::RefCell<u32>,
    }

    impl ServiceConnector for TypedFailureBslConnector {
        fn send_before(
            &self,
            _record: &WorkspaceServiceRecord,
            _request: ServiceRequest,
            _cancellation: &CancellationToken,
            _deadline: OperationDeadline,
        ) -> Result<ServiceResponse, String> {
            unreachable!("this fixture exercises only finite legacy requests")
        }

        fn send(
            &self,
            _record: &WorkspaceServiceRecord,
            request: ServiceRequest,
            _cancellation: &CancellationToken,
            _budget: Duration,
        ) -> Result<ServiceResponse, String> {
            match request.kind {
                ServiceRequestKind::Ping => {
                    *self.pings.borrow_mut() += 1;
                    Ok(ServiceResponse {
                        ok: true,
                        status: Some("alive".to_string()),
                        ..ServiceResponse::default()
                    })
                }
                ServiceRequestKind::BslMcp { .. } => {
                    *self.bsl_calls.borrow_mut() += 1;
                    Ok(unica_analysis_services::test_support::error_response(
                        "typed analyzer failure",
                    ))
                }
                _ => panic!("unexpected request kind"),
            }
        }

        fn send_operation_before(
            &self,
            _record: &WorkspaceServiceRecord,
            _request: ServiceRequest,
            _cancellation: &CancellationToken,
            _deadline: ProviderDeadline,
        ) -> Result<ServiceResponse, String> {
            unreachable!("this legacy fixture does not use the prepared BSL route")
        }
    }

    impl ServiceConnector for RecordingConnector {
        fn send_before(
            &self,
            _record: &WorkspaceServiceRecord,
            _request: ServiceRequest,
            _cancellation: &CancellationToken,
            _deadline: OperationDeadline,
        ) -> Result<ServiceResponse, String> {
            unreachable!("this fixture exercises only finite legacy requests")
        }

        fn send(
            &self,
            _record: &WorkspaceServiceRecord,
            request: ServiceRequest,
            _cancellation: &CancellationToken,
            budget: Duration,
        ) -> Result<ServiceResponse, String> {
            self.requests.borrow_mut().push(request.kind.clone());
            self.budgets.borrow_mut().push(budget);
            if matches!(request.kind, ServiceRequestKind::Ping) {
                *self.pings.borrow_mut() += 1;
            }
            if self.ping_ok {
                Ok(ServiceResponse {
                    ok: true,
                    status: Some("alive".to_string()),
                    ..ServiceResponse::default()
                })
            } else {
                Err("connection refused".to_string())
            }
        }

        fn send_operation_before(
            &self,
            _record: &WorkspaceServiceRecord,
            _request: ServiceRequest,
            _cancellation: &CancellationToken,
            _deadline: ProviderDeadline,
        ) -> Result<ServiceResponse, String> {
            unreachable!("this legacy fixture does not use the prepared BSL route")
        }
    }

    #[derive(Default)]
    struct RecordingSpawner {
        spawns: std::cell::RefCell<u32>,
        delay: Duration,
        budgets: std::cell::RefCell<Vec<Duration>>,
        tokens: std::cell::RefCell<Vec<String>>,
    }

    impl ServiceSpawner for RecordingSpawner {
        fn spawn(
            &self,
            identity: &WorkspaceServiceIdentity,
            _config: WorkspaceServiceConfig,
            token: &str,
            cancellation: &CancellationToken,
            budget: Duration,
        ) -> Result<WorkspaceServiceRecord, String> {
            cancellation_error(cancellation)?;
            *self.spawns.borrow_mut() += 1;
            self.budgets.borrow_mut().push(budget);
            self.tokens.borrow_mut().push(token.to_string());
            let started = Instant::now();
            while started.elapsed() < self.delay {
                cancellation_error(cancellation)?;
                let Some(remaining) = budget
                    .checked_sub(started.elapsed())
                    .filter(|remaining| !remaining.is_zero())
                else {
                    return Err(workspace_service_request_timeout_error(budget));
                };
                thread::sleep(
                    Duration::from_millis(10)
                        .min(remaining)
                        .min(self.delay.saturating_sub(started.elapsed())),
                );
            }
            cancellation_error(cancellation)?;
            if started.elapsed() >= budget {
                return Err(workspace_service_request_timeout_error(budget));
            }
            let mut record = test_record(identity, 45678, CoreIdentity::production().as_str());
            record.token = token.to_string();
            Ok(record)
        }

        fn spawn_operation_before(
            &self,
            _identity: &WorkspaceServiceIdentity,
            _config: WorkspaceServiceConfig,
            _token: &str,
            _cancellation: &CancellationToken,
            _deadline: ProviderDeadline,
        ) -> Result<WorkspaceServiceRecord, String> {
            unreachable!("this legacy fixture does not use typed provider startup")
        }
    }
}
