//! The real protocol-v5 runtime in a thread with an injected canonical
//! service, for the MCP door tests that need a live daemon.
use super::prelude::*;
use crate::infrastructure::daemon::server::DaemonServerConfig;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::thread;
use unica_daemon_protocol::identity::{CoreIdentity, DaemonStateDirectory};
use unica_execution_contract::invocation::DomainResult;

pub(super) const WORKSPACE_MANIFEST: &str =
    "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: .\n";

// --- scripted fake daemon -------------------------------------------------

pub struct ScriptedService {
    pub delay: Duration,
    pub known_long: bool,
    pub outcome: Mutex<
        Option<Result<DomainResult, unica_execution_contract::invocation::InvocationFailure>>,
    >,
    pub executions: AtomicUsize,
}

impl crate::infrastructure::daemon::server::CanonicalInvocationService for ScriptedService {
    fn prepare(
        &self,
        _invocation: &crate::infrastructure::daemon::server::ActorBoundInvocation,
    ) -> Result<unica_execution::receipt_runtime::ports::ExecutionClass, Box<DomainResult>> {
        use unica_execution::receipt_runtime::ports::{ExecutionClass, KnownLongReason};
        Ok(if self.known_long {
            ExecutionClass::KnownLong(KnownLongReason::ExternalProcess)
        } else {
            ExecutionClass::InlineCandidate
        })
    }

    fn execute(
        &self,
        _invocation: &crate::infrastructure::daemon::server::ActorBoundExecution,
        _cancellation: CancellationToken,
    ) -> Result<DomainResult, unica_execution_contract::invocation::InvocationFailure> {
        self.executions.fetch_add(1, Ordering::SeqCst);
        thread::sleep(self.delay);
        self.outcome
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(|| Ok(DomainResult::success("executed")))
    }
}

pub struct LiveDaemon {
    _state: Option<tempfile::TempDir>,
    _workspace: Option<tempfile::TempDir>,
    pub workspace_hint: String,
    pub state_root: std::path::PathBuf,
    thread: Option<thread::JoinHandle<Result<(), String>>>,
    hooks: Option<
        Arc<
            dyn unica_execution::receipt_runtime::hooks::V5RuntimeHooks<
                crate::infrastructure::daemon::server::CanonicalInvocationHost,
            >,
        >,
    >,
    epoch_clock: Option<Arc<dyn unica_execution::invocation_store::EpochMillisClock>>,
}

impl LiveDaemon {
    pub fn start(
        service: Arc<dyn crate::infrastructure::daemon::server::CanonicalInvocationService>,
    ) -> Self {
        Self::start_with_hooks(service, None)
    }

    pub fn start_with_hooks(
        service: Arc<dyn crate::infrastructure::daemon::server::CanonicalInvocationService>,
        hooks: Option<
            Arc<
                dyn unica_execution::receipt_runtime::hooks::V5RuntimeHooks<
                    crate::infrastructure::daemon::server::CanonicalInvocationHost,
                >,
            >,
        >,
    ) -> Self {
        let workspace = tempfile::tempdir().unwrap();
        std::fs::write(workspace.path().join("v8project.yaml"), WORKSPACE_MANIFEST).unwrap();
        Self::start_over(workspace, service, hooks, None)
    }

    /// A workspace whose sources live in `src/` and whose files are older
    /// than the join freshness margin by the time the first call starts:
    /// an identical read may join only while nothing changed since the
    /// walk it joins began.
    pub fn start_with_aged_sources(
        service: Arc<dyn crate::infrastructure::daemon::server::CanonicalInvocationService>,
        hooks: Option<
            Arc<
                dyn unica_execution::receipt_runtime::hooks::V5RuntimeHooks<
                    crate::infrastructure::daemon::server::CanonicalInvocationHost,
                >,
            >,
        >,
    ) -> Self {
        Self::start_with_aged_sources_and_clock(service, hooks, None)
    }

    pub fn start_with_aged_sources_and_clock(
        service: Arc<dyn crate::infrastructure::daemon::server::CanonicalInvocationService>,
        hooks: Option<
            Arc<
                dyn unica_execution::receipt_runtime::hooks::V5RuntimeHooks<
                    crate::infrastructure::daemon::server::CanonicalInvocationHost,
                >,
            >,
        >,
        epoch_clock: Option<Arc<dyn unica_execution::invocation_store::EpochMillisClock>>,
    ) -> Self {
        let workspace = tempfile::tempdir().unwrap();
        std::fs::write(
                workspace.path().join("v8project.yaml"),
                "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
            )
            .unwrap();
        std::fs::create_dir_all(workspace.path().join("src/CommonModules")).unwrap();
        std::fs::write(
            workspace.path().join("src/CommonModules/Module.bsl"),
            "// v1\n",
        )
        .unwrap();
        thread::sleep(
            unica_execution::receipt_runtime::test_support::TIMESTAMP_MARGIN
                + Duration::from_millis(200),
        );
        Self::start_over(workspace, service, hooks, epoch_clock)
    }

    fn start_over(
        workspace: tempfile::TempDir,
        service: Arc<dyn crate::infrastructure::daemon::server::CanonicalInvocationService>,
        hooks: Option<
            Arc<
                dyn unica_execution::receipt_runtime::hooks::V5RuntimeHooks<
                    crate::infrastructure::daemon::server::CanonicalInvocationHost,
                >,
            >,
        >,
        epoch_clock: Option<Arc<dyn unica_execution::invocation_store::EpochMillisClock>>,
    ) -> Self {
        let state = tempfile::tempdir().unwrap();
        let state_root = std::fs::canonicalize(state.path()).unwrap();
        let workspace_hint = std::fs::canonicalize(workspace.path())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let mut daemon = Self {
            _state: Some(state),
            _workspace: Some(workspace),
            workspace_hint,
            state_root,
            thread: None,
            hooks,
            epoch_clock,
        };
        daemon.restart(service, Duration::from_millis(400));
        daemon
    }

    /// Start (again) the daemon over the same state root: the second life
    /// of a restart scenario reads what the first one left durable.
    pub fn restart(
        &mut self,
        service: Arc<dyn crate::infrastructure::daemon::server::CanonicalInvocationService>,
        idle_grace: Duration,
    ) {
        assert!(
            self.thread.is_none(),
            "the previous daemon life must be finished first"
        );
        let config = DaemonServerConfig::new(
            self.state_root.clone(),
            CoreIdentity::production_v5(),
            idle_grace,
        )
        .with_invocation_service(service);
        let config = match &self.epoch_clock {
            Some(clock) => config.with_v5_epoch_clock_for_test(clock.clone()),
            None => config,
        };
        let config = match &self.hooks {
            Some(hooks) => config.with_runtime_hooks_for_test(hooks.clone()),
            None => config,
        };
        self.thread = Some(thread::spawn(move || {
            unica_daemon_protocol::transport::run_daemon(config.into_transport_config())
        }));
    }

    pub fn owner(&self) -> V5DaemonProcessOwner {
        let identity = CoreIdentity::production_v5();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let state = DaemonStateDirectory::open(&self.state_root, &identity).unwrap();
            if let Some(record) = state.read_v5_endpoint_record().unwrap() {
                return V5DaemonProcessOwner::connect_before(record, deadline)
                    .expect("connect live v5 daemon");
            }
            assert!(Instant::now() < deadline, "v5 endpoint was not published");
            thread::sleep(Duration::from_millis(5));
        }
    }

    /// Wait for the daemon to exit on its idle grace; the state root stays.
    pub fn stop(&mut self) {
        self.thread
            .take()
            .expect("a running daemon life")
            .join()
            .expect("join v5 daemon thread")
            .expect("v5 daemon exits cleanly after its idle grace");
    }

    pub fn finish(mut self) {
        self.stop();
    }
}
