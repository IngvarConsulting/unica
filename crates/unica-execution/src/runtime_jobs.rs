//! Durable runtime state, exact resource leases and retained child supervision.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
#[cfg(any(test, feature = "test-support"))]
use std::collections::HashSet;
use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use unica_process::runtime_job::{
    cancel_and_reap, RuntimeJobOutput, RuntimeJobProcess, RuntimeJobProcessState,
    RuntimeJobSpawnFailure,
};
use unica_redaction::{self as redaction, bounded_char_tail, bounded_redacted_tail};
use unica_runner::{
    attempt::{CompletedAttempt, RuntimeAttempt, RuntimeAttemptScope, RuntimeJobRunner},
    operation::{CancelPolicy, RuntimeJobOperation},
};
use unica_work_control::{
    cancellation::{cancelled_error, CancellationToken},
    shared_work::{SharedWork, SharedWorkLease, SharedWorkLifetime, SharedWorkProducer},
};
use unica_workspace::{
    paths::metadata_is_link_or_reparse_point,
    state_fs::{
        replace_file_atomically, sync_parent_directory, HeldStateLock, StateDirectory, StateFile,
        StateLockError,
    },
};
use uuid::Uuid;
type JobResult<T> = Result<T, String>;
const RECORD_SCHEMA_VERSION: u8 = 1;
const RUNTIME_RECORD_BYTES: usize = 256 * 1024;
const CANCEL_MARKER_BYTES: usize = 4 * 1024;
const OUTPUT_TAIL_BYTES: usize = 16 * 1024;
const DEFAULT_STALE_AFTER: Duration = Duration::from_secs(5 * 60);
const LIFECYCLE_LOCK_WAIT: Duration = Duration::from_secs(30);
const LIFECYCLE_LOCK_RETRY: Duration = Duration::from_millis(20);
#[cfg(any(test, feature = "test-support"))]
static INJECT_RUNTIME_RECORD_WRITE_FAILURE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeWorkFailure {
    Invalidated,
    Unavailable,
}

/// The trusted adapter has decoded and validated its own protocol exactly once.
pub struct RuntimeWorkerRun<R: RuntimeAttempt> {
    pub cache_root: PathBuf,
    pub job_id: String,
    pub cwd: PathBuf,
    pub request: R,
}

/// Only execution mints this after durable Succeeded and process/lease release.
pub struct TerminalRuntimeSuccess {
    cwd: PathBuf,
    operation: RuntimeJobOperation,
    job_id: String,
}
impl TerminalRuntimeSuccess {
    pub fn cwd(&self) -> &Path {
        &self.cwd
    }
    pub fn operation(&self) -> RuntimeJobOperation {
        self.operation
    }
    pub fn job_id(&self) -> &str {
        &self.job_id
    }
}
pub trait RuntimeLifecycleObserver: Send + Sync {
    fn after_success(&self, success: &TerminalRuntimeSuccess) -> JobResult<()>;
}

fn redact_reported_argv(argv: &[String]) -> Vec<String> {
    argv.iter().map(|arg| redact_text(arg)).collect()
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeJobPhase {
    Queued,
    Running,
    CancelRequested,
    Succeeded,
    Failed,
    Cancelled,
    TimedOut,
    Lost,
}

impl RuntimeJobPhase {
    pub fn is_terminal(self) -> bool {
        match self {
            Self::Queued | Self::Running | Self::CancelRequested => false,
            Self::Succeeded | Self::Failed | Self::Cancelled | Self::TimedOut | Self::Lost => true,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeJobSnapshot {
    pub id: String,
    pub phase: RuntimeJobPhase,
    pub operation: String,
    pub safe_target: String,
    pub redacted_argv: Vec<String>,
    pub created_at_ms: u64,
    pub started_at_ms: Option<u64>,
    pub heartbeat_at_ms: Option<u64>,
    pub finished_at_ms: Option<u64>,
    pub pid: Option<u32>,
    pub pid_identity: Option<String>,
    pub exit_code: Option<i32>,
    pub cancelled: bool,
    pub cancel_deferred: bool,
    pub unsafe_phase: Option<String>,
    pub timeout_reason: Option<String>,
    pub artifact_path: Option<String>,
    pub stdout_path: String,
    pub stderr_path: String,
    pub warnings: Vec<String>,
    pub wait_timed_out: bool,
}

#[derive(Debug, Clone)]
pub struct RuntimeJobLogs {
    pub stdout: String,
    pub stderr: String,
    pub stdout_path: String,
    pub stderr_path: String,
}

#[derive(Debug, Clone)]
pub struct RuntimeJobList {
    pub jobs: Vec<RuntimeJobSnapshot>,
    pub warnings: Vec<String>,
}

/// Durable job worker harness. A public transport adapter is deliberately outside this module.
/// One observation of the active process: its state, the outputs to persist and
/// the evidence a full rebuild decision needs.
struct RuntimeJobObservation<R: RuntimeAttempt> {
    state: RuntimeJobProcessState,
    output: RuntimeJobOutput,
    attempt_argv: Vec<String>,
    /// Whether a full rebuild is still available for this job. Only then can a
    /// refused receipt mean anything, so only then is it worth reporting.
    fallback_pending: bool,
    retry_request: Option<R>,
}

pub struct RuntimeJobService<R: RuntimeAttempt> {
    store: RuntimeJobStore,
    runner: Arc<dyn RuntimeJobRunner<Request = R>>,
    processes: Mutex<HashMap<String, ActiveRuntimeJobProcess<R>>>,
    #[cfg(any(test, feature = "test-support"))]
    test_admission: Option<TestServiceAdmission>,
    /// Lets a test act exactly at one directory-authority boundary of a poll.
    #[cfg(any(test, feature = "test-support"))]
    directory_hook: Mutex<Option<DirectoryAuthorityHook>>,
}

/// Boundaries inside a poll where the retained job directory is relied on
/// again after an earlier proof. Production code never observes them.
#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DirectoryAuthorityHookPoint {
    /// Logs are written; the fallback or finish decision has not started.
    LogsWritten,
    /// `record.json` was replaced, its post-publication proof has not run.
    RecordReplaced,
    /// The terminal record is confirmed; `active.lock` is still held.
    TerminalPublished,
}

#[cfg(any(test, feature = "test-support"))]
type DirectoryAuthorityHook = Box<dyn FnMut(DirectoryAuthorityHookPoint) + Send>;

impl<R: RuntimeAttempt> Drop for RuntimeJobService<R> {
    fn drop(&mut self) {
        let Ok(processes) = self.processes.get_mut() else {
            return;
        };
        for (job_id, active) in processes.drain() {
            let store = self.store.clone();
            #[cfg(any(test, feature = "test-support"))]
            let (active, controlled_test_supervision) = {
                let mut active = active;
                let controlled = active.process.prepare_controlled_test_supervision();
                (active, controlled)
            };
            let retained = Arc::new(Mutex::new(Some(active)));
            match spawn_quarantine_supervisor(store, job_id.clone(), Arc::clone(&retained)) {
                Ok(supervisor) => {
                    #[cfg(any(test, feature = "test-support"))]
                    {
                        let admission = self.test_admission.as_ref().expect(
                            "retained test process must have an explicit fixture owner admission",
                        );
                        register_test_quarantine_supervisor(
                            admission.owner,
                            admission.service_root.clone(),
                            job_id,
                            controlled_test_supervision,
                            supervisor,
                        );
                    }
                    #[cfg(not(any(test, feature = "test-support")))]
                    drop(supervisor);
                }
                Err(_) => {
                    // Thread creation failed before the clone could assume
                    // ownership. Intentionally retain the last in-process OS
                    // capability forever; active.lock is the durable companion
                    // quarantine. Dropping it would falsely make replacement safe.
                    std::mem::forget(retained);
                }
            }
        }
    }
}

type QuarantinedRuntimeProcess<R> = Arc<Mutex<Option<ActiveRuntimeJobProcess<R>>>>;

#[cfg(any(test, feature = "test-support"))]
static INJECT_QUARANTINE_THREAD_SPAWN_FAILURE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[cfg(any(test, feature = "test-support"))]
struct QuarantineSupervisor {
    handle: thread::JoinHandle<()>,
    finished: std::sync::mpsc::Receiver<()>,
}

#[cfg(not(any(test, feature = "test-support")))]
type QuarantineSupervisor = thread::JoinHandle<()>;

fn spawn_quarantine_supervisor<R: RuntimeAttempt>(
    store: RuntimeJobStore,
    job_id: String,
    retained: QuarantinedRuntimeProcess<R>,
) -> io::Result<QuarantineSupervisor> {
    #[cfg(any(test, feature = "test-support"))]
    if INJECT_QUARANTINE_THREAD_SPAWN_FAILURE.swap(false, std::sync::atomic::Ordering::AcqRel) {
        return Err(io::Error::other(
            "injected runtime quarantine thread spawn failure",
        ));
    }

    #[cfg(any(test, feature = "test-support"))]
    let (finished_tx, finished_rx) = std::sync::mpsc::sync_channel(1);
    let handle = thread::Builder::new()
        .name("unica-runtime-quarantine".to_string())
        .spawn(move || {
            let active = retained
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            if let Some(active) = active {
                supervise_owned_quarantine(store, job_id, active);
            }
            #[cfg(any(test, feature = "test-support"))]
            let _ = finished_tx.send(());
        })?;
    #[cfg(any(test, feature = "test-support"))]
    {
        Ok(QuarantineSupervisor {
            handle,
            finished: finished_rx,
        })
    }
    #[cfg(not(any(test, feature = "test-support")))]
    {
        Ok(handle)
    }
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TestFixtureOwnerToken(Uuid);

#[cfg(any(test, feature = "test-support"))]
#[derive(Default)]
pub struct TestFixtureRegistry {
    pub owners: HashMap<TestFixtureOwnerToken, TestFixtureOwnerState>,
    pub exact_roots: HashMap<PathBuf, TestFixtureOwnerToken>,
}

#[cfg(any(test, feature = "test-support"))]
pub struct TestFixtureOwnerState {
    pub outer_root: PathBuf,
    pub admission_open: bool,
    pub live_admissions: usize,
    pub exact_service_roots: HashSet<PathBuf>,
    pub supervisors: Vec<TestQuarantineSupervisor>,
}

#[cfg(any(test, feature = "test-support"))]
static TEST_FIXTURE_REGISTRY: std::sync::OnceLock<Mutex<TestFixtureRegistry>> =
    std::sync::OnceLock::new();

#[cfg(any(test, feature = "test-support"))]
pub fn test_fixture_registry() -> &'static Mutex<TestFixtureRegistry> {
    TEST_FIXTURE_REGISTRY.get_or_init(|| Mutex::new(TestFixtureRegistry::default()))
}

#[cfg(any(test, feature = "test-support"))]
pub struct TestServiceAdmission {
    pub owner: TestFixtureOwnerToken,
    pub service_root: PathBuf,
}

#[cfg(any(test, feature = "test-support"))]
impl Drop for TestServiceAdmission {
    fn drop(&mut self) {
        let mut registry = test_fixture_registry()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let state = registry
            .owners
            .get_mut(&self.owner)
            .expect("test fixture owner outlived its service admission");
        assert!(
            state.live_admissions > 0,
            "test fixture service admission count underflow"
        );
        state.live_admissions -= 1;
    }
}

#[cfg(any(test, feature = "test-support"))]
pub struct TestQuarantineSupervisor {
    pub service_root: PathBuf,
    pub job_id: String,
    pub controlled: bool,
    supervisor: QuarantineSupervisor,
}

#[cfg(any(test, feature = "test-support"))]
fn register_test_quarantine_supervisor(
    owner: TestFixtureOwnerToken,
    service_root: PathBuf,
    job_id: String,
    controlled: bool,
    supervisor: QuarantineSupervisor,
) {
    let mut registry = test_fixture_registry()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let state = registry
        .owners
        .get_mut(&owner)
        .expect("test fixture owner disappeared before supervisor registration");
    assert!(
        state.exact_service_roots.contains(&service_root),
        "test supervisor service root is not bound to its fixture owner"
    );
    assert!(
        state.live_admissions > 0,
        "test supervisor registration must precede service admission release"
    );
    state.supervisors.push(TestQuarantineSupervisor {
        service_root,
        job_id,
        controlled,
        supervisor,
    });
}

#[cfg(any(test, feature = "test-support"))]
pub fn create_test_fixture_owner(outer_root: PathBuf) -> TestFixtureOwnerToken {
    let owner = TestFixtureOwnerToken(Uuid::new_v4());
    let mut registry = test_fixture_registry()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert!(
        !registry.exact_roots.contains_key(&outer_root),
        "test fixture root already has an owner"
    );
    registry.exact_roots.insert(outer_root.clone(), owner);
    registry.owners.insert(
        owner,
        TestFixtureOwnerState {
            outer_root: outer_root.clone(),
            admission_open: true,
            live_admissions: 0,
            exact_service_roots: HashSet::from([outer_root]),
            supervisors: Vec::new(),
        },
    );
    owner
}

#[cfg(any(test, feature = "test-support"))]
pub fn bind_test_service_root(owner: TestFixtureOwnerToken, service_root: PathBuf) -> PathBuf {
    let mut registry = test_fixture_registry()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(existing) = registry.exact_roots.get(&service_root) {
        assert_eq!(
            *existing, owner,
            "test service root is already bound to another fixture owner"
        );
    }
    let admission_open = registry
        .owners
        .get(&owner)
        .expect("bind service root to live test fixture owner")
        .admission_open;
    if !admission_open {
        drop(registry);
        panic!("test fixture supervisor admission is already closed");
    }
    let state = registry
        .owners
        .get_mut(&owner)
        .expect("bind service root to live test fixture owner");
    if state.exact_service_roots.contains(&service_root) {
        return service_root;
    }
    state.exact_service_roots.insert(service_root.clone());
    registry.exact_roots.insert(service_root.clone(), owner);
    service_root
}

#[cfg(any(test, feature = "test-support"))]
pub fn acquire_test_service_admission(service_root: &Path) -> Option<TestServiceAdmission> {
    let mut registry = test_fixture_registry()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let owner = *registry.exact_roots.get(service_root)?;
    let state = registry
        .owners
        .get_mut(&owner)
        .expect("test service root points to a live fixture owner");
    assert!(
        state.admission_open,
        "test fixture supervisor admission is already closed"
    );
    state.live_admissions = state
        .live_admissions
        .checked_add(1)
        .expect("test fixture service admission count overflow");
    Some(TestServiceAdmission {
        owner,
        service_root: service_root.to_path_buf(),
    })
}

#[cfg(any(test, feature = "test-support"))]
pub fn drain_test_fixture_owner(owner: TestFixtureOwnerToken) {
    let (outer_root, mut supervisors) = {
        let mut registry = test_fixture_registry()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(state) = registry.owners.get_mut(&owner) else {
            return;
        };
        state.admission_open = false;
        if state.live_admissions != 0 {
            let live = state.live_admissions;
            let roots = state.exact_service_roots.clone();
            drop(registry);
            panic!(
                "test fixture owner still has {live} live runtime service admission(s): {roots:?}"
            );
        }
        (
            state.outer_root.clone(),
            std::mem::take(&mut state.supervisors),
        )
    };
    let deadline = Instant::now() + Duration::from_secs(2);
    while let Some(supervisor) = supervisors.pop() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let finished = supervisor
            .supervisor
            .finished
            .recv_timeout(remaining)
            .is_ok();
        if !finished {
            let service_root = supervisor.service_root.clone();
            let job_id = supervisor.job_id.clone();
            let controlled = supervisor.controlled;
            supervisors.push(supervisor);
            test_fixture_registry()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .owners
                .get_mut(&owner)
                .expect("restore unfinished test supervisor to fixture owner")
                .supervisors
                .append(&mut supervisors);
            panic!(
                "test quarantine supervisor remains active for owner root {} service root {} job {} (controlled={controlled}); fixture must prove terminal ownership and drain it explicitly",
                outer_root.display(),
                service_root.display(),
                job_id,
            );
        }
        supervisor
            .supervisor
            .handle
            .join()
            .expect("test quarantine supervisor panicked");
    }

    let mut registry = test_fixture_registry()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let state = registry
        .owners
        .remove(&owner)
        .expect("remove drained test fixture owner");
    assert_eq!(state.live_admissions, 0);
    assert!(state.supervisors.is_empty());
    for root in state.exact_service_roots {
        assert_eq!(registry.exact_roots.remove(&root), Some(owner));
    }
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static TEST_SUPERVISOR_SCOPES: std::cell::RefCell<Vec<Arc<Mutex<Vec<TestFixtureOwnerToken>>>>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(any(test, feature = "test-support"))]
pub struct TestSupervisorScope {
    pub owners: Arc<Mutex<Vec<TestFixtureOwnerToken>>>,
}

#[cfg(any(test, feature = "test-support"))]
impl TestSupervisorScope {
    pub fn enter() -> Self {
        let owners = Arc::new(Mutex::new(Vec::new()));
        TEST_SUPERVISOR_SCOPES.with(|scopes| scopes.borrow_mut().push(Arc::clone(&owners)));
        Self { owners }
    }

    pub fn assert_drained(&self) {
        let owners = self
            .owners
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let registry = test_fixture_registry()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for owner in owners {
            assert!(
                !registry.owners.contains_key(&owner),
                "test fixture owner {owner:?} remains registered after teardown"
            );
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Drop for TestSupervisorScope {
    fn drop(&mut self) {
        TEST_SUPERVISOR_SCOPES.with(|scopes| {
            let popped = scopes.borrow_mut().pop().expect("test supervisor scope");
            assert!(Arc::ptr_eq(&popped, &self.owners));
        });
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn register_test_supervisor_owner(owner: TestFixtureOwnerToken) {
    TEST_SUPERVISOR_SCOPES.with(|scopes| {
        for scope in scopes.borrow().iter() {
            scope
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(owner);
        }
    });
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct RuntimeWorkKey {
    resource_identity: [u8; 32],
    lease_identity: Uuid,
}

#[cfg(test)]
mod shared_identity_tests {
    use super::{RuntimeWorkKey, Uuid};

    #[test]
    fn typed_runtime_keys_remain_exact() {
        assert_ne!(
            RuntimeWorkKey {
                resource_identity: [8; 32],
                lease_identity: Uuid::new_v4(),
            },
            RuntimeWorkKey {
                resource_identity: [8; 32],
                lease_identity: Uuid::new_v4(),
            }
        );
    }
}

/// Daemon-owned in-process coordinator. Only `RuntimeJobService` can derive a
/// key, while the store's lifecycle guard and retained `active.lock`
/// capability are held. A canonical service can therefore ask to join an
/// exact active lease but cannot manufacture or cache a movable authority.
pub struct RuntimeResourceOwner {
    shared: SharedWork<RuntimeWorkKey, (), RuntimeWorkFailure>,
}

impl Default for RuntimeResourceOwner {
    fn default() -> Self {
        Self {
            shared: SharedWork::new(SharedWorkLifetime::OwnerBound),
        }
    }
}

impl RuntimeResourceOwner {
    fn join_or_start<W>(
        &self,
        key: RuntimeWorkKey,
        work: W,
    ) -> SharedWorkLease<RuntimeWorkKey, (), RuntimeWorkFailure>
    where
        W: FnOnce(SharedWorkProducer) -> Result<(), RuntimeWorkFailure> + Send + 'static,
    {
        self.shared.join_or_start(key, work)
    }
}

impl<R: RuntimeAttempt> RuntimeJobService<R> {
    pub fn new(
        cache_root: impl Into<PathBuf>,
        runner: Arc<dyn RuntimeJobRunner<Request = R>>,
    ) -> Self {
        Self::with_stale_after(cache_root, runner, DEFAULT_STALE_AFTER)
    }

    /// Join exact runtime work while the same lifecycle authority that owns
    /// `active.lock` is held. No authority key leaves this method.
    #[allow(dead_code)] // Consumed by the canonical runtime handler at Task 22 cutover.
    pub fn join_shared_work<W>(
        &self,
        id: &str,
        owner: &RuntimeResourceOwner,
        work: W,
    ) -> JobResult<SharedWorkLease<RuntimeWorkKey, (), RuntimeWorkFailure>>
    where
        W: FnOnce(SharedWorkProducer) -> Result<(), RuntimeWorkFailure> + Send + 'static,
    {
        self.join_shared_work_after_capability(id, owner, work, || {})
    }

    fn join_shared_work_after_capability<W>(
        &self,
        id: &str,
        owner: &RuntimeResourceOwner,
        work: W,
        after_capability: impl FnOnce(),
    ) -> JobResult<SharedWorkLease<RuntimeWorkKey, (), RuntimeWorkFailure>>
    where
        W: FnOnce(SharedWorkProducer) -> Result<(), RuntimeWorkFailure> + Send + 'static,
    {
        let id = canonical_job_id(id)?;
        let lifecycle = self.store.acquire_active_lifecycle_lock_bounded()?;
        after_capability();
        let active = lifecycle
            .retain_active_lock()
            .map_err(|error| io_error("retain runtime active lease", &error))?;
        let retained_lease = String::from_utf8(
            active
                .read_bounded(64)
                .map_err(|error| io_error("read retained runtime active lease", &error))?,
        )
        .map_err(|_| redacted_error("runtime active lease is not canonical UTF-8"))?;
        if retained_lease != id {
            return Err(redacted_error(
                "runtime shared work lease does not own the active resource",
            ));
        }
        lifecycle
            .validate()
            .and_then(|()| active.validate_named_identity())
            .map_err(|error| io_error("revalidate runtime active lease", &error))?;
        let revalidated_lease = String::from_utf8(
            active
                .read_bounded(64)
                .map_err(|error| io_error("reread retained runtime active lease", &error))?,
        )
        .map_err(|_| redacted_error("runtime active lease is not canonical UTF-8"))?;
        if revalidated_lease != id {
            return Err(redacted_error(
                "runtime active lease changed before shared-work admission",
            ));
        }
        let lease_identity =
            Uuid::parse_str(&id).map_err(|_| redacted_error("derive runtime lease identity"))?;
        let mut digest = Sha256::new();
        digest.update(b"unica-runtime-resource-v1\0");
        digest.update(lifecycle.jobs_root.identity().stable_bytes());
        digest.update(active.identity().stable_bytes());
        let resource_identity: [u8; 32] = digest.finalize().into();
        Ok(owner.join_or_start(
            RuntimeWorkKey {
                resource_identity,
                lease_identity,
            },
            work,
        ))
    }

    fn with_stale_after(
        cache_root: impl Into<PathBuf>,
        runner: Arc<dyn RuntimeJobRunner<Request = R>>,
        stale_after: Duration,
    ) -> Self {
        let cache_root = cache_root.into();
        Self {
            #[cfg(any(test, feature = "test-support"))]
            test_admission: acquire_test_service_admission(&cache_root),
            #[cfg(any(test, feature = "test-support"))]
            directory_hook: Mutex::new(None),
            store: RuntimeJobStore::new(cache_root, stale_after),
            runner,
            processes: Mutex::new(HashMap::new()),
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    fn require_test_fixture_admission(&self) -> JobResult<()> {
        self.test_admission.as_ref().map(|_| ()).ok_or_else(|| {
            redacted_error(
                "test runtime service root has no explicit fixture owner binding; use TestCache::service_root for nested roots",
            )
        })
    }

    #[cfg(any(test, feature = "test-support"))]
    fn start(&self, request: R) -> JobResult<RuntimeJobSnapshot> {
        let id = Uuid::new_v4().to_string();
        self.store.enqueue(&id, &request)?;
        self.activate_enqueued(&id, &request)
    }

    pub fn enqueue(cache_root: impl Into<PathBuf>, request: &R) -> JobResult<RuntimeJobSnapshot> {
        let store = RuntimeJobStore::new(cache_root, DEFAULT_STALE_AFTER);
        let id = Uuid::new_v4().to_string();
        let record = store.enqueue(&id, request)?;
        Ok(record.snapshot(false))
    }

    fn activate_enqueued(&self, id: &str, request: &R) -> JobResult<RuntimeJobSnapshot> {
        self.activate_enqueued_after_preflight(id, request, || {})
    }
    /// The hook runs after the first build reauthorization, so a test can let
    /// recovery or cancellation win that window. Production passes a no-op.
    fn activate_enqueued_after_preflight(
        &self,
        id: &str,
        request: &R,
        after_preflight: impl FnOnce(),
    ) -> JobResult<RuntimeJobSnapshot> {
        self.activate_enqueued_after_hooks(id, request, after_preflight, || {})
    }
    /// The second hook runs just before the activation guard is claimed, so a
    /// test can order itself against it. Production passes no-ops.
    fn activate_enqueued_after_hooks(
        &self,
        id: &str,
        request: &R,
        after_preflight: impl FnOnce(),
        before_activation_guard: impl FnOnce(),
    ) -> JobResult<RuntimeJobSnapshot> {
        #[cfg(any(test, feature = "test-support"))]
        self.require_test_fixture_admission()?;
        let mut record = self.store.read_record(id)?;
        // No platform process has started while the record is still queued, so
        // every operation can honor cancellation safely at this boundary. The
        // critical policy applies only after the worker owns a running process.
        if let Some(cancelled) = self.cancel_enqueued_before_start(&mut record)? {
            return Ok(cancelled);
        }
        if record.phase != RuntimeJobPhase::Queued {
            return Err(redacted_error("runtime job worker expected a queued job"));
        }
        if record.operation != request.operation().label() {
            return Err(redacted_error(
                "runtime job worker operation does not match queued job",
            ));
        }
        let preflight_result = self.runner.reauthorize(request);
        after_preflight();
        before_activation_guard();

        // Recovery and cancellation are separate processes. Linearize the
        // queued-to-running claim with both of them, then re-read durable state
        // under the same guard that is held through spawn and publication.
        let lifecycle_lock = self.store.acquire_active_lifecycle_lock()?;
        record = self.store.read_record(id)?;
        if let Some(cancelled) = self.cancel_enqueued_before_start_guarded(&mut record)? {
            return Ok(cancelled);
        }
        if record.phase != RuntimeJobPhase::Queued {
            return Err(redacted_error("runtime job worker expected a queued job"));
        }
        if record.operation != request.operation().label() {
            return Err(redacted_error(
                "runtime job worker operation does not match queued job",
            ));
        }
        let job_directory = lifecycle_lock
            .retain_job_directory(&record.id)
            .map_err(|error| io_error("retain activated runtime job directory", &error))?;
        let retained_authority = RetainedRuntimeJobAuthority {
            jobs_root: lifecycle_lock.jobs_root.clone(),
            job_directory,
        };
        // The first reauthorization may have completed while cancellation or
        // recovery owned the lifecycle guard. Repeat it after claiming the
        // activation boundary so stale evidence cannot cross into spawn.
        let preflight_result = preflight_result.and_then(|()| self.runner.reauthorize(request));
        if let Err(error) = preflight_result {
            let error = redacted_error(&error);
            let _ = self.fail_start_with_authority_guarded(
                &mut record,
                &error,
                &retained_authority,
                &lifecycle_lock,
            );
            return Err(error);
        }
        if let Err(error) = retained_authority.job_directory.validate_named_identity() {
            return Err(io_error(
                "revalidate activated runtime job directory",
                &error,
            ));
        }
        // The child must never be able to exist before the record admits it. A
        // worker killed between the spawn and the `Running` write would otherwise
        // leave a queued record that wrongly proves the workspace is childless.
        record.child_spawn_attempted = Some(true);
        if let Err(error) = self.publish_record_with_authority(&retained_authority, &record) {
            let _ = self.fail_start_with_authority_guarded(
                &mut record,
                &error,
                &retained_authority,
                &lifecycle_lock,
            );
            return Err(error);
        }
        let process = match self.runner.spawn(request) {
            Ok(process) => process,
            Err(RuntimeJobSpawnFailure::ProvenChildless(error)) => {
                let error = redacted_error(&error);
                let _ = self.fail_start_with_authority_guarded(
                    &mut record,
                    &error,
                    &retained_authority,
                    &lifecycle_lock,
                );
                return Err(error);
            }
            Err(RuntimeJobSpawnFailure::OwnershipRetained { error, process }) => {
                return Err(self.retain_uncertain_spawn_guarded(
                    &mut record,
                    request,
                    process,
                    retained_authority.clone(),
                    None,
                    &error,
                ));
            }
        };
        record.pid = Some(process.id());
        record.pid_identity = Some(format!("pid:{}", process.id()));
        record.started_at_ms = Some(now_millis());
        record.heartbeat_at_ms = Some(now_millis());
        record.transition(RuntimeJobPhase::Running)?;
        if let Err(error) = self.publish_record_with_authority(&retained_authority, &record) {
            self.cleanup_activation_failure_guarded(
                &mut record,
                process,
                retained_authority.clone(),
                request.reportable_argv(),
                None,
                &error,
            );
            return Err(error);
        }
        let mut processes = match self.lock_processes() {
            Ok(processes) => processes,
            Err(error) => {
                self.cleanup_activation_failure_guarded(
                    &mut record,
                    process,
                    retained_authority.clone(),
                    request.reportable_argv(),
                    None,
                    &error,
                );
                return Err(error);
            }
        };
        let mut request_with_fallback = request.clone();
        let fallback = request_with_fallback.take_retry();
        processes.insert(
            id.to_string(),
            ActiveRuntimeJobProcess {
                process,
                jobs_root: retained_authority.jobs_root,
                job_directory: retained_authority.job_directory,
                attempt_argv: request.reportable_argv(),
                attempt: Some(request.clone()),
                scope: Some(self.runner.scope(request)),
                fallback,
                previous_output: None,
            },
        );
        Ok(record.snapshot(false))
    }

    fn cancel_enqueued_before_start(
        &self,
        record: &mut RuntimeJobRecord,
    ) -> JobResult<Option<RuntimeJobSnapshot>> {
        let _lifecycle_lock = self.store.acquire_active_lifecycle_lock()?;
        *record = self.store.read_record(&record.id)?;
        self.cancel_enqueued_before_start_guarded(record)
    }

    fn cancel_enqueued_before_start_guarded(
        &self,
        record: &mut RuntimeJobRecord,
    ) -> JobResult<Option<RuntimeJobSnapshot>> {
        let id = record.id.clone();
        if record.phase != RuntimeJobPhase::CancelRequested && !self.store.has_cancel_marker(&id)? {
            return Ok(None);
        }
        record.cancelled = true;
        record.finished_at_ms = Some(now_millis());
        record.heartbeat_at_ms = Some(now_millis());
        record.transition(RuntimeJobPhase::Cancelled)?;
        self.store.write_record(record)?;
        self.store.release_active_lock_guarded(&id, || {})?;
        Ok(Some(record.snapshot(false)))
    }

    #[cfg(any(test, feature = "test-support"))]
    fn status(&self, id: &str) -> JobResult<RuntimeJobSnapshot> {
        Self::status_at(&self.store.cache_root, id)
    }

    pub fn status_at(cache_root: impl Into<PathBuf>, id: &str) -> JobResult<RuntimeJobSnapshot> {
        let store = RuntimeJobStore::new(cache_root, DEFAULT_STALE_AFTER);
        let _ = store.recover_stale_active()?;
        let record = store.read_record(id)?;
        store.snapshot_with_cancel_intent(&record, false)
    }

    /// Once this service owns the job's process, every read and write of the
    /// poll goes through the job directory retained at process admission. A
    /// directory installed later under `jobs/<id>` is never read, written or
    /// unlocked; losing the name keeps the process under observation.
    pub fn poll(&self, id: &str) -> JobResult<RuntimeJobSnapshot> {
        let lifecycle_lock = self.store.acquire_active_lifecycle_lock()?;
        let admitted = self.retained_authority(id)?;
        if let Some(authority) = &admitted {
            self.confirm_retained_authority(id, authority, &lifecycle_lock)?;
        }
        let mut record = match &admitted {
            Some(authority) => self
                .store
                .read_record_in_retained_job(id, &authority.job_directory)?,
            None => self.store.read_record(id)?,
        };
        if record.phase.is_terminal() {
            return Ok(record.snapshot(false));
        }
        if self.store.stale(&record) {
            record.transition(RuntimeJobPhase::Lost)?;
            record.finished_at_ms = Some(now_millis());
            record.warnings.push("stale heartbeat".to_string());
            self.write_poll_record(admitted.as_ref(), &record)?;
            return Ok(record.snapshot(false));
        }

        let cancel_requested = match &admitted {
            Some(authority) => self
                .store
                .has_cancel_marker_in_retained_job(&authority.job_directory)?,
            None => self.store.has_cancel_marker(id)?,
        };
        let request_safe_cancel = cancel_requested
            && record.cancel_policy == CancelPolicy::Safe
            && !record.cancel_attempted;
        if request_safe_cancel {
            if record.phase == RuntimeJobPhase::Running {
                record.transition(RuntimeJobPhase::CancelRequested)?;
            }
            record.cancel_attempted = true;
            record.heartbeat_at_ms = Some(now_millis());
            self.write_poll_record(admitted.as_ref(), &record)?;
        }
        if cancel_requested && record.cancel_policy == CancelPolicy::Critical {
            match record.phase {
                RuntimeJobPhase::Queued | RuntimeJobPhase::Running => {
                    record.transition(RuntimeJobPhase::CancelRequested)?;
                }
                RuntimeJobPhase::CancelRequested => {}
                RuntimeJobPhase::Succeeded
                | RuntimeJobPhase::Failed
                | RuntimeJobPhase::Cancelled
                | RuntimeJobPhase::TimedOut
                | RuntimeJobPhase::Lost => {
                    return Err(redacted_error(
                        "terminal runtime job was observed as active",
                    ));
                }
            }
            record.cancel_deferred = true;
            record.unsafe_phase = Some(record.operation.clone());
            record.heartbeat_at_ms = Some(now_millis());
            self.write_poll_record(admitted.as_ref(), &record)?;
        }
        let RuntimeJobObservation {
            state: process_state,
            output,
            attempt_argv: _,
            fallback_pending,
            retry_request,
        } = match self.observe_process(&record, request_safe_cancel, &lifecycle_lock) {
            Ok(observation) => observation,
            Err(error) => {
                return Err(self.quarantine_observation_failure_guarded(&mut record, &error));
            }
        };
        // An attach admits the process and its directory inside the observation;
        // from here on the poll holds that exact directory.
        let authority = match admitted {
            Some(authority) => authority,
            None => self.retained_authority(id)?.ok_or_else(|| {
                redacted_error(&format!("runtime job {id} process is unavailable"))
            })?,
        };
        self.confirm_retained_authority(id, &authority, &lifecycle_lock)?;
        self.store
            .write_logs_in_retained_job(&authority.job_directory, &output)?;
        #[cfg(any(test, feature = "test-support"))]
        self.directory_hook(DirectoryAuthorityHookPoint::LogsWritten);
        if output.output_incomplete {
            record.warnings.push(
                "runtime job output reader was abandoned before end of stream; the persisted tail \
                 may be incomplete and no full rebuild can be authorized from it"
                    .to_string(),
            );
        }

        match process_state {
            RuntimeJobProcessState::Running => {
                record.heartbeat_at_ms = Some(now_millis());
                self.publish_record_with_authority(&authority, &record)?;
                Ok(record.snapshot(false))
            }
            RuntimeJobProcessState::Exited { exit_code } => {
                if exit_code != 0 && !cancel_requested && fallback_pending {
                    match retry_request
                        .as_ref()
                        .expect("retry candidate retains original typed request")
                        .classify_retry(CompletedAttempt {
                            process_exit_code: Some(exit_code),
                            status_success: false,
                            timed_out: false,
                            cancelled: false,
                            stdout_truncated: output.fallback_receipt_truncated,
                            stdout_had_invalid_utf8: output.fallback_receipt.is_none()
                                && !output.fallback_receipt_truncated,
                            stdout: output.fallback_receipt.as_deref().unwrap_or_default(),
                        }) {
                        Ok(_) => {
                            if let Some(snapshot) = self.start_full_build_fallback_guarded(
                                &mut record,
                                &output,
                                exit_code,
                                &lifecycle_lock,
                            )? {
                                return Ok(snapshot);
                            }
                        }
                        // A receipt that reached the pinned failure code and was
                        // still refused has to say so, or a drifted receipt is
                        // indistinguishable from a runtime that never tried.
                        Err(warning) => record.warnings.extend(warning),
                    }
                }
                let phase = if cancel_requested && record.cancel_policy == CancelPolicy::Safe {
                    record.cancelled = true;
                    RuntimeJobPhase::Cancelled
                } else if exit_code == 0 {
                    RuntimeJobPhase::Succeeded
                } else {
                    RuntimeJobPhase::Failed
                };
                self.finish_guarded(
                    &mut record,
                    &authority,
                    &lifecycle_lock,
                    phase,
                    Some(exit_code),
                    None,
                )
            }
            RuntimeJobProcessState::TimedOut { reason } => self.finish_guarded(
                &mut record,
                &authority,
                &lifecycle_lock,
                RuntimeJobPhase::TimedOut,
                None,
                Some(redact_text(&reason)),
            ),
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    fn set_directory_hook(&self, hook: impl FnMut(DirectoryAuthorityHookPoint) + Send + 'static) {
        *self.directory_hook.lock().unwrap() = Some(Box::new(hook));
    }

    #[cfg(any(test, feature = "test-support"))]
    fn directory_hook(&self, point: DirectoryAuthorityHookPoint) {
        if let Some(hook) = self.directory_hook.lock().unwrap().as_mut() {
            hook(point);
        }
    }

    /// The directory authority of the process this service owns for `id`, if
    /// any. Cloning shares the admitted handles; nothing is resolved by name.
    fn retained_authority(&self, id: &str) -> JobResult<Option<RetainedRuntimeJobAuthority>> {
        Ok(self
            .lock_processes()?
            .get(id)
            .map(|active| RetainedRuntimeJobAuthority {
                jobs_root: active.jobs_root.clone(),
                job_directory: active.job_directory.clone(),
            }))
    }

    /// Proves that `jobs/<id>` still names the directory admitted with the
    /// process and that the lifecycle guard covers the same jobs root. A
    /// failure leaves the process, its record and `active.lock` untouched: the
    /// caller keeps the process under observation instead of finishing it.
    fn confirm_retained_authority(
        &self,
        id: &str,
        authority: &RetainedRuntimeJobAuthority,
        lifecycle: &ActiveLifecycleLock,
    ) -> JobResult<()> {
        let confirmed = if lifecycle.jobs_root.identity() == authority.jobs_root.identity() {
            authority.job_directory.validate_named_identity()
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "lifecycle guard covers a different jobs directory",
            ))
        };
        confirmed.map_err(|error| {
            redacted_error(&format!(
                "runtime job {id} no longer owns its admitted job directory; the process stays \
                 under observation and active.lock is retained: {error}"
            ))
        })
    }

    /// Before the process is admitted the poll has no retained directory and
    /// writes by name; afterwards it never does.
    fn write_poll_record(
        &self,
        authority: Option<&RetainedRuntimeJobAuthority>,
        record: &RuntimeJobRecord,
    ) -> JobResult<()> {
        match authority {
            Some(authority) => self.publish_record_with_authority(authority, record),
            None => self.store.write_record(record),
        }
    }

    /// Publishes the record inside the retained directory and confirms that
    /// the published file is still `record.json` of that same directory.
    fn publish_record_with_authority(
        &self,
        authority: &RetainedRuntimeJobAuthority,
        record: &RuntimeJobRecord,
    ) -> JobResult<()> {
        authority
            .job_directory
            .validate_named_identity()
            .map_err(|error| io_error("validate retained runtime job directory", &error))?;
        let published = self
            .store
            .write_record_in_retained_job(&authority.job_directory, record)?;
        #[cfg(any(test, feature = "test-support"))]
        self.directory_hook(DirectoryAuthorityHookPoint::RecordReplaced);
        authority
            .job_directory
            .validate_named_identity()
            .and_then(|()| published.validate_named_identity())
            .map_err(|error| io_error("confirm retained runtime job record", &error))
    }

    #[cfg(any(test, feature = "test-support"))]
    fn wait(&self, id: &str, caller_timeout: Duration) -> JobResult<RuntimeJobSnapshot> {
        let started_at = Instant::now();
        let deadline = match started_at.checked_add(caller_timeout) {
            Some(deadline) => deadline,
            None => started_at,
        };
        loop {
            let snapshot = self.poll(id)?;
            if snapshot.phase.is_terminal() {
                return Ok(snapshot);
            }
            if Instant::now() >= deadline {
                let mut timed_out = snapshot;
                timed_out.wait_timed_out = true;
                return Ok(timed_out);
            }
            thread::sleep(Duration::from_millis(1));
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    fn logs(&self, id: &str) -> JobResult<RuntimeJobLogs> {
        Self::logs_at(&self.store.cache_root, id, OUTPUT_TAIL_BYTES)
    }

    pub fn logs_at(
        cache_root: impl Into<PathBuf>,
        id: &str,
        tail_chars: usize,
    ) -> JobResult<RuntimeJobLogs> {
        let store = RuntimeJobStore::new(cache_root, DEFAULT_STALE_AFTER);
        let record = store.read_record(id)?;
        let stdout = fs::read_to_string(store.stdout_path(&record.id)?)
            .map_err(|error| io_error("read runtime job stdout log", &error))?;
        let stderr = fs::read_to_string(store.stderr_path(&record.id)?)
            .map_err(|error| io_error("read runtime job stderr log", &error))?;
        Ok(RuntimeJobLogs {
            stdout: bounded_char_tail(&stdout, tail_chars),
            stderr: bounded_char_tail(&stderr, tail_chars),
            stdout_path: record.stdout_path,
            stderr_path: record.stderr_path,
        })
    }

    #[cfg(any(test, feature = "test-support"))]
    fn cancel(&self, id: &str) -> JobResult<RuntimeJobSnapshot> {
        let record = self.store.read_record(id)?;
        if record.phase.is_terminal() {
            return Ok(record.snapshot(false));
        }
        self.store.write_cancel_marker(id)?;
        self.poll(id)
    }

    #[cfg(any(test, feature = "test-support"))]
    fn list(&self) -> RuntimeJobList {
        self.store.list()
    }

    pub fn list_at(cache_root: impl Into<PathBuf>) -> RuntimeJobList {
        let store = RuntimeJobStore::new(cache_root, DEFAULT_STALE_AFTER);
        let recovery_warning = store.recover_stale_active().err();
        let mut list = store.list();
        if let Some(warning) = recovery_warning {
            list.warnings.push(warning);
        }
        list
    }

    pub fn request_cancel_at(
        cache_root: impl Into<PathBuf>,
        id: &str,
    ) -> JobResult<RuntimeJobSnapshot> {
        let store = RuntimeJobStore::new(cache_root, DEFAULT_STALE_AFTER);
        let _lifecycle_lock = store.acquire_active_lifecycle_lock_bounded()?;
        let record = store.read_record(id)?;
        if record.phase.is_terminal() {
            return Ok(record.snapshot(false));
        }
        store.write_cancel_marker(id)?;
        // The worker and guarded lifecycle recovery publish transitions in
        // record.json. Re-read after the marker so a concurrently committed
        // terminal result always wins over this cancellation request.
        let current = store.read_record(id)?;
        store.snapshot_with_cancel_intent(&current, false)
    }

    pub fn wait_at(
        cache_root: impl Into<PathBuf>,
        id: &str,
        caller_timeout: Duration,
    ) -> JobResult<RuntimeJobSnapshot> {
        let cache_root = cache_root.into();
        let started_at = Instant::now();
        let deadline = started_at.checked_add(caller_timeout).unwrap_or(started_at);
        loop {
            let snapshot = Self::status_at(cache_root.clone(), id)?;
            if snapshot.phase.is_terminal() {
                return Ok(snapshot);
            }
            if Instant::now() >= deadline {
                let mut timed_out = snapshot;
                timed_out.wait_timed_out = true;
                return Ok(timed_out);
            }
            thread::sleep(Duration::from_millis(25));
        }
    }

    fn observe_process(
        &self,
        record: &RuntimeJobRecord,
        request_safe_cancel: bool,
        lifecycle: &ActiveLifecycleLock,
    ) -> JobResult<RuntimeJobObservation<R>> {
        #[cfg(any(test, feature = "test-support"))]
        self.require_test_fixture_admission()?;
        let mut processes = self.lock_processes()?;
        if !processes.contains_key(&record.id) {
            // A compatibility attach cannot precede the historical spawn, but
            // it must retain the exact directory before accepting ownership of
            // the process. From this point every quarantine retry carries this
            // capability instead of resolving the job name again.
            let job_directory = lifecycle
                .retain_job_directory(&record.id)
                .map_err(|error| io_error("retain attached runtime job directory", &error))?;
            let process_id = record.pid.ok_or_else(|| {
                redacted_error(&format!(
                    "runtime job {} has no persisted process id",
                    record.id
                ))
            })?;
            let process = self.runner.attach(process_id).map_err(|error| {
                redacted_error(&format!("attach runtime job {}: {error}", record.id))
            })?;
            processes.insert(
                record.id.clone(),
                ActiveRuntimeJobProcess {
                    process,
                    jobs_root: lifecycle.jobs_root.clone(),
                    job_directory,
                    attempt_argv: record.redacted_argv.clone(),
                    attempt: None,
                    scope: None,
                    fallback: None,
                    previous_output: None,
                },
            );
        }
        let active = processes.get_mut(&record.id).ok_or_else(|| {
            redacted_error(&format!("runtime job {} process is unavailable", record.id))
        })?;

        if request_safe_cancel {
            active.process.cancel().map_err(|error| {
                redacted_error(&format!("cancel runtime job {}: {error}", record.id))
            })?;
        }
        let state = active.process.try_wait().map_err(|error| {
            redacted_error(&format!("observe runtime job {}: {error}", record.id))
        })?;
        let mut output = active
            .process
            .output_tails(OUTPUT_TAIL_BYTES)
            .map_err(|error| {
                redacted_error(&format!("read runtime job {} output: {error}", record.id))
            })?;
        if let Some(previous) = &active.previous_output {
            output = R::combine_outputs(previous, &output);
        }
        Ok(RuntimeJobObservation {
            state,
            output,
            attempt_argv: active.attempt_argv.clone(),
            fallback_pending: active.fallback.is_some(),
            retry_request: active.attempt.clone(),
        })
    }

    fn start_full_build_fallback_guarded(
        &self,
        record: &mut RuntimeJobRecord,
        first_output: &RuntimeJobOutput,
        first_exit_code: i32,
        lifecycle: &ActiveLifecycleLock,
    ) -> JobResult<Option<RuntimeJobSnapshot>> {
        let (fallback, retained_authority) = {
            let mut processes = self.lock_processes()?;
            let active = processes.get_mut(&record.id).ok_or_else(|| {
                redacted_error(&format!("runtime job {} process is unavailable", record.id))
            })?;
            (
                active.fallback.take(),
                RetainedRuntimeJobAuthority {
                    jobs_root: active.jobs_root.clone(),
                    job_directory: active.job_directory.clone(),
                },
            )
        };
        let Some(fallback) = fallback else {
            return Ok(None);
        };
        // The full rebuild belongs to the same job directory. Without proof that
        // the name still holds it, no second process is started; the first one
        // stays under observation with its directory and lock.
        if let Err(error) =
            self.confirm_retained_authority(&record.id, &retained_authority, lifecycle)
        {
            if let Ok(mut processes) = self.lock_processes() {
                if let Some(active) = processes.get_mut(&record.id) {
                    active.fallback = Some(fallback);
                }
            }
            return Err(error);
        }

        let bound_scope = self
            .lock_processes()?
            .get(&record.id)
            .and_then(|active| active.scope.clone());
        if bound_scope.as_ref() != Some(&self.runner.scope(&fallback))
            || fallback.operation().label() != record.operation
        {
            record.warnings.push(redacted_error("runtime retry was refused because its prepared scope does not match the original admission"));
            return self
                .finish_guarded(
                    record,
                    &retained_authority,
                    lifecycle,
                    RuntimeJobPhase::Failed,
                    Some(first_exit_code),
                    None,
                )
                .map(Some);
        }
        {
            if let Err(error) = self.runner.reauthorize(&fallback) {
                record
                    .warnings
                    .push(redact_text(&fallback.retry_start_warning(
                        unica_runner::attempt::RetryStartFailure::Authorization,
                        &error,
                    )));
                return self
                    .finish_guarded(
                        record,
                        &retained_authority,
                        lifecycle,
                        RuntimeJobPhase::Failed,
                        Some(first_exit_code),
                        None,
                    )
                    .map(Some);
            }
        }

        let next = match self.runner.spawn(&fallback) {
            Ok(process) => process,
            Err(RuntimeJobSpawnFailure::ProvenChildless(error)) => {
                record
                    .warnings
                    .push(redacted_error(&fallback.retry_start_warning(
                        unica_runner::attempt::RetryStartFailure::Spawn,
                        &error,
                    )));
                return self
                    .finish_guarded(
                        record,
                        &retained_authority,
                        lifecycle,
                        RuntimeJobPhase::Failed,
                        Some(first_exit_code),
                        None,
                    )
                    .map(Some);
            }
            Err(RuntimeJobSpawnFailure::OwnershipRetained { error, process }) => {
                let error = self.retain_uncertain_spawn_guarded(
                    record,
                    &fallback,
                    process,
                    retained_authority,
                    Some(first_output.clone()),
                    &error,
                );
                return Err(error);
            }
        };
        record.pid = Some(next.id());
        record.pid_identity = Some(format!("pid:{}", next.id()));
        record.redacted_argv = redact_reported_argv(&fallback.reportable_argv());
        record.heartbeat_at_ms = Some(now_millis());
        record.warnings.push(fallback.retry_warning().to_string());
        if let Err(error) = self.publish_record_with_authority(&retained_authority, record) {
            self.cleanup_activation_failure_guarded(
                record,
                next,
                retained_authority.clone(),
                fallback.reportable_argv(),
                Some(first_output.clone()),
                &error,
            );
            return Err(error);
        }

        let mut processes = match self.lock_processes() {
            Ok(processes) => processes,
            Err(error) => {
                self.cleanup_activation_failure_guarded(
                    record,
                    next,
                    retained_authority.clone(),
                    fallback.reportable_argv(),
                    Some(first_output.clone()),
                    &error,
                );
                return Err(error);
            }
        };
        let Some(active) = processes.get_mut(&record.id) else {
            let error =
                redacted_error(&format!("runtime job {} process is unavailable", record.id));
            drop(processes);
            self.cleanup_activation_failure_guarded(
                record,
                next,
                retained_authority,
                fallback.reportable_argv(),
                Some(first_output.clone()),
                &error,
            );
            return Err(error);
        };
        active.process = next;
        active.attempt_argv = fallback.reportable_argv();
        active.attempt = Some(fallback.clone());
        let mut previous_output = first_output.clone();
        previous_output.fallback_receipt = None;
        previous_output.fallback_receipt_truncated = false;
        // The poll that observed the first attempt already recorded whatever it
        // had to say about its reader. Carrying the flag forward would repeat
        // that warning on every poll of the full attempt.
        previous_output.output_incomplete = false;
        active.previous_output = Some(previous_output);
        Ok(Some(record.snapshot(false)))
    }

    fn retain_uncertain_spawn_guarded(
        &self,
        record: &mut RuntimeJobRecord,
        request: &R,
        process: Box<dyn RuntimeJobProcess>,
        retained_authority: RetainedRuntimeJobAuthority,
        previous_output: Option<RuntimeJobOutput>,
        error: &str,
    ) -> String {
        let process_id = process.id();
        record.pid = Some(process_id);
        record.pid_identity = Some(format!("pid:{process_id}"));
        record.redacted_argv = redact_reported_argv(&request.reportable_argv());
        let _ = record.transition(RuntimeJobPhase::Lost);
        record.finished_at_ms = Some(now_millis());
        record.heartbeat_at_ms = Some(now_millis());
        record.warnings.push(redacted_error(&format!(
            "runtime process ownership is uncertain after spawn: {error}; active.lock remains quarantined"
        )));
        let persist = self.publish_record_with_authority(&retained_authority, record);
        let retain = self.lock_processes().map(|mut processes| {
            processes.insert(
                record.id.clone(),
                ActiveRuntimeJobProcess {
                    process,
                    jobs_root: retained_authority.jobs_root,
                    job_directory: retained_authority.job_directory,
                    attempt_argv: request.reportable_argv(),
                    attempt: Some(request.clone()),
                    scope: Some(self.runner.scope(request)),
                    fallback: None,
                    previous_output,
                },
            );
        });
        match (persist, retain) {
            (Ok(()), Ok(())) => redacted_error(error),
            (persist, retain) => redacted_error(&format!(
                "{error}; retain uncertain runtime ownership: persist={}; memory={}",
                persist.err().unwrap_or_else(|| "ok".to_string()),
                retain.err().unwrap_or_else(|| "ok".to_string())
            )),
        }
    }

    fn quarantine_observation_failure_guarded(
        &self,
        record: &mut RuntimeJobRecord,
        error: &str,
    ) -> String {
        if matches!(
            record.phase,
            RuntimeJobPhase::Running | RuntimeJobPhase::CancelRequested
        ) {
            let _ = record.transition(RuntimeJobPhase::Lost);
        }
        record.finished_at_ms = Some(now_millis());
        record.heartbeat_at_ms = Some(now_millis());
        record.warnings.push(redacted_error(&format!(
            "runtime process observation is uncertain; ownership remains quarantined: {error}"
        )));
        // An attach may have admitted the directory before the observation
        // failed; the quarantine record then belongs to that directory only.
        let persisted = match self.retained_authority(&record.id) {
            Ok(Some(authority)) => self.publish_record_with_authority(&authority, record),
            Ok(None) => self.store.write_record(record),
            Err(error) => Err(error),
        };
        match persisted {
            Ok(()) => redacted_error(error),
            Err(persist) => redacted_error(&format!(
                "{error}; persist retained runtime quarantine: {persist}"
            )),
        }
    }

    /// Publishes the terminal record in the retained job directory and only
    /// then releases `active.lock` through the retained jobs root. Any failure
    /// keeps the process in this service and the lock in place.
    fn finish_guarded(
        &self,
        record: &mut RuntimeJobRecord,
        authority: &RetainedRuntimeJobAuthority,
        lifecycle: &ActiveLifecycleLock,
        phase: RuntimeJobPhase,
        exit_code: Option<i32>,
        timeout_reason: Option<String>,
    ) -> JobResult<RuntimeJobSnapshot> {
        record.transition(phase)?;
        record.exit_code = exit_code;
        if timeout_reason.is_some() {
            record.timeout_reason = timeout_reason;
        }
        record.finished_at_ms = Some(now_millis());
        record.heartbeat_at_ms = Some(now_millis());
        self.publish_record_with_authority(authority, record)?;
        #[cfg(any(test, feature = "test-support"))]
        self.directory_hook(DirectoryAuthorityHookPoint::TerminalPublished);
        self.confirm_retained_authority(&record.id, authority, lifecycle)?;
        lifecycle
            .release_active_lock_for(&record.id, || {})
            .map_err(|error| io_error("release finished runtime job lock", &error))?;
        self.remove_process(&record.id)?;
        Ok(record.snapshot(false))
    }

    fn fail_start_with_authority_guarded(
        &self,
        record: &mut RuntimeJobRecord,
        error: &str,
        authority: &RetainedRuntimeJobAuthority,
        lifecycle: &ActiveLifecycleLock,
    ) -> JobResult<()> {
        record.transition(RuntimeJobPhase::Failed)?;
        record.finished_at_ms = Some(now_millis());
        record.warnings.push(redact_text(error));
        self.publish_record_with_authority(authority, record)?;
        authority
            .job_directory
            .validate_named_identity()
            .map_err(|error| io_error("revalidate failed runtime job directory", &error))?;
        lifecycle
            .release_active_lock_for(&record.id, || {})
            .map_err(|error| io_error("release failed runtime job lock", &error))
    }

    fn cleanup_activation_failure_guarded(
        &self,
        record: &mut RuntimeJobRecord,
        mut process: Box<dyn RuntimeJobProcess>,
        retained_authority: RetainedRuntimeJobAuthority,
        attempt_argv: Vec<String>,
        previous_output: Option<RuntimeJobOutput>,
        activation_error: &str,
    ) {
        match cancel_and_reap(&mut *process) {
            Ok(()) => {
                if record.phase == RuntimeJobPhase::Running {
                    let _ = record.transition(RuntimeJobPhase::Failed);
                }
                record.finished_at_ms = Some(now_millis());
                record.heartbeat_at_ms = Some(now_millis());
                record.warnings.push(redact_text(&format!(
                    "worker activation failed after child spawn: {activation_error}"
                )));
                if self
                    .publish_record_with_authority(&retained_authority, record)
                    .is_ok()
                {
                    let _ = self
                        .store
                        .acquire_active_lifecycle_lock_for_root_bounded(
                            retained_authority.jobs_root.clone(),
                        )
                        .and_then(|lifecycle| {
                            retained_authority
                                .job_directory
                                .validate_named_identity()
                                .map_err(|error| {
                                    io_error(
                                        "validate retained runtime activation directory",
                                        &error,
                                    )
                                })?;
                            lifecycle
                                .release_active_lock_for(&record.id, || {})
                                .map_err(|error| {
                                    io_error("release retained runtime activation lock", &error)
                                })
                        });
                }
            }
            Err(cleanup_error) => {
                if record.phase == RuntimeJobPhase::Running {
                    let _ = record.transition(RuntimeJobPhase::Lost);
                }
                record.finished_at_ms = Some(now_millis());
                record.heartbeat_at_ms = Some(now_millis());
                record.warnings.push(redact_text(&format!(
                    "worker activation lost child ownership: {activation_error}; cleanup: {cleanup_error}"
                )));
                // The lock intentionally remains: the child tree may still be mutating.
                let _ = self.publish_record_with_authority(&retained_authority, record);
                // Ownership moves to the same map consumed by canonical worker
                // quarantine supervision. Drop is not allowed to become the
                // last owner after an uncertain cleanup.
                if let Ok(mut processes) = self.lock_processes() {
                    processes.insert(
                        record.id.clone(),
                        ActiveRuntimeJobProcess {
                            process,
                            jobs_root: retained_authority.jobs_root,
                            job_directory: retained_authority.job_directory,
                            attempt_argv,
                            attempt: None,
                            scope: None,
                            fallback: None,
                            previous_output,
                        },
                    );
                } else {
                    // `lock_processes` recovers poison below; this is only a
                    // fail-safe for an impossible future implementation error.
                    std::mem::forget(process);
                }
            }
        }
    }

    /// Adds a late warning to the terminal record of the directory the job
    /// finished in; a directory installed later under the job name is refused.
    fn append_warning(
        &self,
        authority: &RetainedRuntimeJobAuthority,
        id: &str,
        warning: &str,
    ) -> JobResult<()> {
        authority
            .job_directory
            .validate_named_identity()
            .map_err(|error| io_error("validate finished runtime job directory", &error))?;
        let mut record = self
            .store
            .read_record_in_retained_job(id, &authority.job_directory)?;
        if record.phase.is_terminal() {
            record.warnings.push(redact_text(warning));
            self.publish_record_with_authority(authority, &record)?;
        }
        Ok(())
    }

    fn lock_processes(
        &self,
    ) -> JobResult<std::sync::MutexGuard<'_, HashMap<String, ActiveRuntimeJobProcess<R>>>> {
        Ok(self
            .processes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner))
    }

    fn remove_process(&self, id: &str) -> JobResult<()> {
        let mut processes = self.lock_processes()?;
        processes.remove(id);
        Ok(())
    }

    fn has_retained_process(&self, id: &str) -> bool {
        self.processes
            .lock()
            .map(|processes| processes.contains_key(id))
            .unwrap_or(true)
    }

    fn supervise_quarantine(&self, id: &str) -> JobResult<()> {
        let active = self
            .lock_processes()?
            .remove(id)
            .ok_or_else(|| redacted_error("quarantined runtime process is unavailable"))?;
        supervise_owned_quarantine(self.store.clone(), id.to_string(), active);
        Ok(())
    }
}

fn supervise_owned_quarantine<R: RuntimeAttempt>(
    store: RuntimeJobStore,
    job_id: String,
    mut active: ActiveRuntimeJobProcess<R>,
) {
    loop {
        if try_release_owned_quarantine_once(&store, &job_id, &mut active).is_ok_and(|done| done) {
            return;
        }
        thread::sleep(Duration::from_millis(25));
    }
}

/// Tries one terminal-proof/release transition for a retained process. The
/// retained jobs-directory capability is the authority for both lifecycle
/// admission and `active.lock` removal: an ambient path replacement must turn
/// this into quarantine, never into a release against the replacement tree.
fn try_release_owned_quarantine_once<R: RuntimeAttempt>(
    store: &RuntimeJobStore,
    job_id: &str,
    active: &mut ActiveRuntimeJobProcess<R>,
) -> JobResult<bool> {
    try_release_owned_quarantine_once_after_terminal(store, job_id, active, || {})
}

/// The hook lets the physical-root replacement race be scheduled exactly
/// after terminal/output proof and before lifecycle admission.
fn try_release_owned_quarantine_once_after_terminal<R: RuntimeAttempt>(
    store: &RuntimeJobStore,
    job_id: &str,
    active: &mut ActiveRuntimeJobProcess<R>,
    after_terminal_proof: impl FnOnce(),
) -> JobResult<bool> {
    let mut after_terminal_proof = Some(after_terminal_proof);
    try_release_owned_quarantine_once_after_record_hooks(store, job_id, active, |point| {
        if point == QuarantineReleaseHookPoint::AfterTerminalProof {
            if let Some(hook) = after_terminal_proof.take() {
                hook();
            }
        }
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QuarantineReleaseHookPoint {
    AfterTerminalProof,
    AfterFinalValidationBeforeRead,
    AfterRecordRead,
    ImmediatelyBeforeRecordPublish,
    AfterRecordPublishBeforeConfirmation,
}

/// The typed hook exposes each record-I/O boundary without widening the
/// production transition's argument surface. Production wrappers ignore every
/// point.
fn try_release_owned_quarantine_once_after_record_hooks<R: RuntimeAttempt>(
    store: &RuntimeJobStore,
    job_id: &str,
    active: &mut ActiveRuntimeJobProcess<R>,
    mut hook: impl FnMut(QuarantineReleaseHookPoint),
) -> JobResult<bool> {
    let terminal = matches!(
        active.process.try_wait(),
        Ok(RuntimeJobProcessState::Exited { .. } | RuntimeJobProcessState::TimedOut { .. })
    );
    if !terminal {
        return Ok(false);
    }
    let output_complete = active
        .process
        .output_tails_until(OUTPUT_TAIL_BYTES, Instant::now())
        .is_ok_and(|output| !output.output_incomplete);
    if !output_complete {
        return Ok(false);
    }
    hook(QuarantineReleaseHookPoint::AfterTerminalProof);

    let lifecycle =
        store.acquire_active_lifecycle_lock_for_root_bounded(active.jobs_root.clone())?;
    lifecycle
        .validate()
        .map_err(|error| io_error("revalidate retained runtime quarantine root", &error))?;
    let job_directory = active.job_directory.clone();
    let record_file = job_directory
        .retain_regular_child(std::ffi::OsStr::new("record.json"))
        .map_err(|error| io_error("retain runtime quarantine record", &error))?;
    job_directory
        .validate_named_identity()
        .and_then(|()| record_file.validate_named_identity())
        .map_err(|error| io_error("validate retained runtime quarantine record", &error))?;
    hook(QuarantineReleaseHookPoint::AfterFinalValidationBeforeRead);
    let mut record = store.read_retained_record(job_id, &record_file)?;
    hook(QuarantineReleaseHookPoint::AfterRecordRead);
    let publish_lost = if matches!(
        record.phase,
        RuntimeJobPhase::Queued | RuntimeJobPhase::Running | RuntimeJobPhase::CancelRequested
    ) {
        record.transition(RuntimeJobPhase::Lost)?;
        true
    } else if record.phase == RuntimeJobPhase::Lost {
        true
    } else if record.phase.is_terminal() {
        false
    } else {
        return Err(redacted_error(
            "runtime quarantine record has an unexpected terminal phase",
        ));
    };
    if publish_lost {
        record.finished_at_ms.get_or_insert_with(now_millis);
        record.heartbeat_at_ms = Some(now_millis());
        lifecycle
            .validate()
            .map_err(|error| io_error("revalidate retained runtime quarantine root", &error))?;
        job_directory
            .validate_named_identity()
            .and_then(|()| record_file.validate_named_identity())
            .map_err(|error| io_error("revalidate retained runtime quarantine record", &error))?;
        hook(QuarantineReleaseHookPoint::ImmediatelyBeforeRecordPublish);
        let published_record = store.write_record_in_retained_job(&job_directory, &record)?;
        hook(QuarantineReleaseHookPoint::AfterRecordPublishBeforeConfirmation);
        lifecycle
            .validate()
            .and_then(|()| job_directory.validate_named_identity())
            .and_then(|()| published_record.validate_named_identity())
            .map_err(|error| io_error("confirm retained runtime quarantine publication", &error))?;
    } else {
        // Another exact service instance may have published the terminal
        // result and released the lease while this instance still retained an
        // attachment. Terminal process+EOF proof allows this duplicate OS
        // capability to converge, but the already-published result is never
        // rewritten as Lost.
        lifecycle
            .validate()
            .and_then(|()| job_directory.validate_named_identity())
            .and_then(|()| record_file.validate_named_identity())
            .map_err(|error| io_error("confirm retained terminal runtime record", &error))?;
    }
    lifecycle
        .release_active_lock_for(job_id, || {})
        .map_err(|error| io_error("release retained runtime quarantine lock", &error))?;
    Ok(true)
}

#[cfg(any(test, feature = "test-support"))]
fn worker_start_result(
    cache_root: &Path,
    queued: RuntimeJobSnapshot,
    cancellation_observed: bool,
) -> JobResult<RuntimeJobSnapshot> {
    let store = RuntimeJobStore::new(cache_root.to_path_buf(), DEFAULT_STALE_AFTER);
    let _lifecycle_lock = store.acquire_active_lifecycle_lock()?;
    worker_start_result_guarded(&store, queued, cancellation_observed)
}

fn worker_start_result_guarded(
    store: &RuntimeJobStore,
    queued: RuntimeJobSnapshot,
    cancellation_observed: bool,
) -> JobResult<RuntimeJobSnapshot> {
    if cancellation_observed {
        cancel_queued_job_guarded(store, &queued.id, || {})?;
        Err(cancelled_error(
            "runtime job start stopped before detached worker activation",
        ))
    } else {
        Ok(queued)
    }
}

fn settle_worker_handoff_guarded(
    store: &RuntimeJobStore,
    queued: RuntimeJobSnapshot,
    handoff_result: JobResult<bool>,
    terminate_worker: impl FnOnce(),
) -> JobResult<RuntimeJobSnapshot> {
    match handoff_result {
        Ok(cancellation_observed) => {
            let result = worker_start_result_guarded(store, queued, cancellation_observed);
            if result.is_err() {
                // A false commit may already be readable by the worker. If
                // publishing Cancelled fails, it must still be stopped before
                // the activation guard is released.
                terminate_worker();
            }
            result
        }
        Err(error) => {
            // Even a delimiter/flush error can leave a complete false commit in
            // the pipe. Stop that worker first, while activation remains
            // guarded, and make the queued record terminal before unlock.
            terminate_worker();
            fail_queued_job_guarded(store, &queued.id, &error)?;
            Err(error)
        }
    }
}

pub fn run_worker_request<R: RuntimeAttempt>(
    handoff: RuntimeWorkerRun<R>,
    runner: Arc<dyn RuntimeJobRunner<Request = R>>,
    observer: &dyn RuntimeLifecycleObserver,
) -> Result<(), String> {
    run_worker_request_with_lifecycle_hooks(handoff, runner, observer, None::<fn()>, None::<fn()>)
}

/// The hook runs after the worker has durably published `Lost` and immediately
/// before it starts supervising the retained process. Tests use the event
/// instead of racing the worker through the ambient filesystem.
#[cfg(any(test, feature = "test-support"))]
pub fn run_worker_request_after_quarantine_persisted<R: RuntimeAttempt>(
    handoff: RuntimeWorkerRun<R>,
    runner: Arc<dyn RuntimeJobRunner<Request = R>>,
    observer: &dyn RuntimeLifecycleObserver,
    after_quarantine_persisted: impl FnMut(),
) -> Result<(), String> {
    run_worker_request_with_lifecycle_hooks(
        handoff,
        runner,
        observer,
        None::<fn()>,
        Some(after_quarantine_persisted),
    )
}

#[cfg(any(test, feature = "test-support"))]
pub fn run_worker_request_after_activation_and_quarantine_persisted<R: RuntimeAttempt>(
    handoff: RuntimeWorkerRun<R>,
    runner: Arc<dyn RuntimeJobRunner<Request = R>>,
    observer: &dyn RuntimeLifecycleObserver,
    after_activation_persisted: impl FnMut(),
    after_quarantine_persisted: impl FnMut(),
) -> Result<(), String> {
    run_worker_request_with_lifecycle_hooks(
        handoff,
        runner,
        observer,
        Some(after_activation_persisted),
        Some(after_quarantine_persisted),
    )
}

fn run_worker_request_with_lifecycle_hooks<R: RuntimeAttempt>(
    handoff: RuntimeWorkerRun<R>,
    runner: Arc<dyn RuntimeJobRunner<Request = R>>,
    observer: &dyn RuntimeLifecycleObserver,
    mut after_activation_persisted: Option<impl FnMut()>,
    mut after_quarantine_persisted: Option<impl FnMut()>,
) -> Result<(), String> {
    let job_id = canonical_job_id(&handoff.job_id)?;
    let request = match handoff.request.validate().map(|()| handoff.request.clone()) {
        Ok(request) => request,
        Err(error) => {
            fail_queued_job(&handoff.cache_root, &job_id, &error)?;
            return Err(error);
        }
    };
    let worker_cwd = handoff.cwd.clone();
    let service = RuntimeJobService::new(handoff.cache_root, runner);
    if let Err(error) = service.activate_enqueued(&job_id, &request) {
        if service.has_retained_process(&job_id) {
            notify_after_durable_quarantine(&service, &job_id, &mut after_quarantine_persisted)?;
            return service.supervise_quarantine(&job_id);
        }
        return Err(error);
    }
    if let Some(hook) = &mut after_activation_persisted {
        hook();
    }

    loop {
        // The poll that finishes the job releases its process; keep the
        // directory it was admitted with for the success effects below.
        let authority = service.retained_authority(&job_id)?;
        let snapshot = match service.poll(&job_id) {
            Ok(snapshot) => snapshot,
            Err(_error) if service.has_retained_process(&job_id) => {
                notify_after_durable_quarantine(
                    &service,
                    &job_id,
                    &mut after_quarantine_persisted,
                )?;
                return service.supervise_quarantine(&job_id);
            }
            Err(error) => return Err(error),
        };
        if snapshot.phase.is_terminal() && service.has_retained_process(&job_id) {
            notify_after_durable_quarantine(&service, &job_id, &mut after_quarantine_persisted)?;
            return service.supervise_quarantine(&job_id);
        }
        if snapshot.phase.is_terminal() {
            if snapshot.phase == RuntimeJobPhase::Succeeded {
                if let Err(error) = observer.after_success(&TerminalRuntimeSuccess {
                    cwd: worker_cwd.clone(),
                    operation: request.operation(),
                    job_id: job_id.clone(),
                }) {
                    if let Some(authority) = &authority {
                        let _ = service.append_warning(authority, &job_id, &error);
                    }
                }
            }
            return Ok(());
        }
        thread::sleep(Duration::from_millis(25));
    }
}

fn notify_after_durable_quarantine<R: RuntimeAttempt>(
    service: &RuntimeJobService<R>,
    job_id: &str,
    hook: &mut Option<impl FnMut()>,
) -> JobResult<()> {
    let Some(hook) = hook.as_mut() else {
        return Ok(());
    };
    let record = service.store.read_record(job_id)?;
    if record.phase != RuntimeJobPhase::Lost {
        return Err(redacted_error(
            "runtime worker entered retained supervision before durable quarantine publication",
        ));
    }
    hook();
    Ok(())
}

pub fn fail_queued_job(cache_root: &Path, id: &str, error: &str) -> JobResult<()> {
    let store = RuntimeJobStore::new(cache_root.to_path_buf(), DEFAULT_STALE_AFTER);
    let _lifecycle_lock = store.acquire_active_lifecycle_lock()?;
    fail_queued_job_guarded(&store, id, error)
}

fn fail_queued_job_guarded(store: &RuntimeJobStore, id: &str, error: &str) -> JobResult<()> {
    let mut record = store.read_record(id)?;
    if record.phase != RuntimeJobPhase::Queued {
        return Ok(());
    }
    record.transition(RuntimeJobPhase::Failed)?;
    record.finished_at_ms = Some(now_millis());
    record.heartbeat_at_ms = Some(now_millis());
    record.warnings.push(redact_text(error));
    store.write_record(&record)?;
    store.release_active_lock_guarded(id, || {})
}

pub fn cancel_queued_job(cache_root: &Path, id: &str) -> JobResult<()> {
    cancel_queued_job_after_guard(cache_root, id, || {})
}
/// The hook runs under the lifecycle guard once the record is known to be
/// queued, so a test can race the terminal write. Production passes a no-op.
fn cancel_queued_job_after_guard(
    cache_root: &Path,
    id: &str,
    after_guard: impl FnOnce(),
) -> JobResult<()> {
    let store = RuntimeJobStore::new(cache_root.to_path_buf(), DEFAULT_STALE_AFTER);
    let _lifecycle_lock = store.acquire_active_lifecycle_lock()?;
    cancel_queued_job_guarded(&store, id, after_guard)
}

fn cancel_queued_job_guarded(
    store: &RuntimeJobStore,
    id: &str,
    after_guard: impl FnOnce(),
) -> JobResult<()> {
    let mut record = store.read_record(id)?;
    if record.phase != RuntimeJobPhase::Queued {
        return Ok(());
    }
    after_guard();
    record.cancelled = true;
    record.transition(RuntimeJobPhase::Cancelled)?;
    record.finished_at_ms = Some(now_millis());
    record.heartbeat_at_ms = Some(now_millis());
    store.write_record(&record)?;
    store.release_active_lock_guarded(id, || {})
}

fn enqueue_cancellable_runtime_job<R: RuntimeAttempt>(
    cache_root: &Path,
    request: &R,
    cancellation: &CancellationToken,
) -> JobResult<RuntimeJobSnapshot> {
    enqueue_cancellable_runtime_job_after_hook(cache_root, request, cancellation, || {})
}
/// The hook runs after the durable enqueue and before the cancellation is
/// re-read, so a test can flip the token in that window. Production passes a
/// no-op.
fn enqueue_cancellable_runtime_job_after_hook<R: RuntimeAttempt>(
    cache_root: &Path,
    request: &R,
    cancellation: &CancellationToken,
    after_enqueue: impl FnOnce(),
) -> JobResult<RuntimeJobSnapshot> {
    let queued = RuntimeJobService::enqueue(cache_root.to_path_buf(), request)?;
    after_enqueue();
    if cancellation.is_cancelled() {
        cancel_queued_job(cache_root, &queued.id)?;
        return Err(cancelled_error(
            "runtime job start stopped before detached worker launch",
        ));
    }
    Ok(queued)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> JobResult<()> {
    let parent = path
        .parent()
        .ok_or_else(|| redacted_error("atomic runtime job path has no parent"))?;
    let temporary = parent.join(format!(".{}.{}.tmp", path_file_name(path), Uuid::new_v4()));
    let mut file = File::create(&temporary)
        .map_err(|error| io_error("create temporary runtime job file", &error))?;
    file.write_all(bytes)
        .map_err(|error| io_error("write temporary runtime job file", &error))?;
    file.sync_data()
        .map_err(|error| io_error("sync temporary runtime job file", &error))?;
    let replace_result = replace_file_atomically(&temporary, path)
        .map_err(|error| io_error("atomically replace runtime job file", &error));
    if let Err(error) = replace_result {
        let cleanup = fs::remove_file(&temporary);
        return match cleanup {
            Ok(()) => Err(error),
            Err(cleanup_error) if cleanup_error.kind() == io::ErrorKind::NotFound => Err(error),
            Err(cleanup_error) => Err(redacted_error(&format!(
                "{error}; remove failed runtime job staging file: {cleanup_error}"
            ))),
        };
    }
    sync_parent_directory(parent).map_err(|error| io_error("sync runtime job directory", &error))
}

fn path_file_name(path: &Path) -> String {
    match path.file_name().and_then(|name| name.to_str()) {
        Some(name) => name.to_string(),
        None => "runtime-job".to_string(),
    }
}

fn canonical_job_id(id: &str) -> JobResult<String> {
    Uuid::parse_str(id)
        .map(|uuid| uuid.to_string())
        .map_err(|_| redacted_error("runtime job id must be a UUID"))
}

fn now_millis() -> u64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration_millis(duration),
        Err(_) => 0,
    }
}

fn duration_millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn redact_text(text: &str) -> String {
    redaction::redactor(text)
}

fn redacted_error(message: &str) -> String {
    redact_text(message)
}

fn io_error(context: &str, error: &std::io::Error) -> String {
    redacted_error(&format!("{context}: {error}"))
}

pub fn start_detached_worker<R: RuntimeAttempt>(
    cache_root: PathBuf,
    request: R,
    cancellation: &CancellationToken,
    write_handoff: impl FnOnce(
        &mut dyn Write,
        &RuntimeJobSnapshot,
        &R,
        &CancellationToken,
    ) -> JobResult<bool>,
) -> JobResult<RuntimeJobSnapshot> {
    request.validate()?;
    if cancellation.is_cancelled() {
        return Err(cancelled_error(
            "runtime job start stopped before durable enqueue",
        ));
    }
    let queued = enqueue_cancellable_runtime_job(&cache_root, &request, cancellation)?;
    if cancellation.is_cancelled() {
        cancel_queued_job(&cache_root, &queued.id)?;
        return Err(cancelled_error(
            "runtime job start stopped before detached worker launch",
        ));
    }
    let mut worker = match Command::new(std::env::current_exe().map_err(|error| {
        redacted_error(&format!("resolve runtime job worker executable: {error}"))
    })?)
    .arg("--runtime-job-worker")
    .stdin(Stdio::piped())
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .spawn()
    {
        Ok(worker) => worker,
        Err(error) => {
            let error = redacted_error(&format!("spawn runtime job worker: {error}"));
            fail_queued_job(&cache_root, &queued.id, &error)?;
            return Err(error);
        }
    };
    let store = RuntimeJobStore::new(cache_root.clone(), DEFAULT_STALE_AFTER);
    // Hold the same inter-process guard used by worker activation while the
    // commit frame is published. A cancellation observed during that write is
    // then terminalized before a worker carrying the older serialized snapshot
    // can claim Queued -> Running.
    let lifecycle_lock = match store.acquire_active_lifecycle_lock_bounded() {
        Ok(lifecycle_lock) => lifecycle_lock,
        Err(error) => {
            let _ = worker.kill();
            let _ = worker.wait();
            fail_queued_job(&cache_root, &queued.id, &error)?;
            return Err(error);
        }
    };
    let write_result = worker
        .stdin
        .take()
        .ok_or_else(|| redacted_error("runtime job worker stdin is unavailable"))
        .and_then(|mut stdin| write_handoff(&mut stdin, &queued, &request, cancellation));
    let result = settle_worker_handoff_guarded(&store, queued, write_result, || {
        let _ = worker.kill();
    });
    let reap_worker = result.is_err();
    drop(lifecycle_lock);
    if reap_worker {
        // The worker may have been waiting for the lifecycle guard. Reap only
        // after releasing it; termination was already requested while guarded.
        let _ = worker.wait();
    }
    result
}

mod owner_state {
    use super::*;
    /// Compatibility rule for this record. `schema_version` is an exact-match gate,
    /// so a bump makes every build that does not know the new number reject the
    /// record outright. Additive fields therefore keep the version and carry
    /// `#[serde(default)]`, and unknown fields are tolerated on read so that a field
    /// added by a later build does not make this build fail `read_record`. That
    /// failure is not cosmetic: an unreadable record named by `active.lock` pins the
    /// lock with no automatic recovery. The version is reserved for changes that
    /// genuinely cannot be read by an older build.
    #[derive(Debug, Clone, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct RuntimeJobRecord {
        #[serde(rename = "schemaVersion")]
        pub schema_version: u8,
        pub id: String,
        pub phase: RuntimeJobPhase,
        pub operation: String,
        pub safe_target: String,
        pub redacted_argv: Vec<String>,
        pub created_at_ms: u64,
        pub started_at_ms: Option<u64>,
        pub heartbeat_at_ms: Option<u64>,
        pub finished_at_ms: Option<u64>,
        pub pid: Option<u32>,
        pub pid_identity: Option<String>,
        /// Persisted immediately before the worker asks the runner for a child, so a
        /// record can prove whether an orphaned child process may exist. Absent means
        /// unknown, not "no child": a build that predates this field could die after
        /// `runner.spawn()` and before persisting `pid`, leaving a live child behind
        /// a queued record with no trace of it. Only an explicit `false` is proof.
        #[serde(default)]
        pub child_spawn_attempted: Option<bool>,
        pub exit_code: Option<i32>,
        pub cancel_policy: CancelPolicy,
        pub cancelled: bool,
        pub cancel_deferred: bool,
        pub cancel_attempted: bool,
        pub unsafe_phase: Option<String>,
        pub timeout_reason: Option<String>,
        pub artifact_path: Option<String>,
        pub stdout_path: String,
        pub stderr_path: String,
        pub warnings: Vec<String>,
    }

    impl RuntimeJobRecord {
        pub fn queued<R: RuntimeAttempt>(id: String, request: &R) -> Self {
            let now = now_millis();
            Self {
                schema_version: RECORD_SCHEMA_VERSION,
                id: id.clone(),
                phase: RuntimeJobPhase::Queued,
                operation: request.operation().label().to_string(),
                safe_target: redact_text(request.safe_target()),
                redacted_argv: redact_reported_argv(&request.reportable_argv()),
                created_at_ms: now,
                // The public start contract reports when the durable job lifecycle
                // began. The worker replaces this with the child-start timestamp
                // once it has successfully claimed the queued record.
                started_at_ms: Some(now),
                heartbeat_at_ms: Some(now),
                finished_at_ms: None,
                pid: None,
                pid_identity: None,
                child_spawn_attempted: Some(false),
                exit_code: None,
                cancel_policy: request.operation().cancel_policy(),
                cancelled: false,
                cancel_deferred: false,
                cancel_attempted: false,
                unsafe_phase: None,
                timeout_reason: request.timeout_reason().map(redact_text),
                artifact_path: request.artifact_path().map(redact_text),
                stdout_path: format!("jobs/{id}/stdout.log"),
                stderr_path: format!("jobs/{id}/stderr.log"),
                warnings: Vec::new(),
            }
        }

        pub fn snapshot(&self, wait_timed_out: bool) -> RuntimeJobSnapshot {
            RuntimeJobSnapshot {
                id: self.id.clone(),
                phase: self.phase,
                operation: self.operation.clone(),
                safe_target: self.safe_target.clone(),
                redacted_argv: self.redacted_argv.clone(),
                created_at_ms: self.created_at_ms,
                started_at_ms: self.started_at_ms,
                heartbeat_at_ms: self.heartbeat_at_ms,
                finished_at_ms: self.finished_at_ms,
                pid: self.pid,
                pid_identity: self.pid_identity.clone(),
                exit_code: self.exit_code,
                cancelled: self.cancelled,
                cancel_deferred: self.cancel_deferred,
                unsafe_phase: self.unsafe_phase.clone(),
                timeout_reason: self.timeout_reason.clone(),
                artifact_path: self.artifact_path.clone(),
                stdout_path: self.stdout_path.clone(),
                stderr_path: self.stderr_path.clone(),
                warnings: self.warnings.clone(),
                wait_timed_out,
            }
        }

        /// Whether an orphaned child process of this job may still be running, and
        /// therefore whether releasing `active.lock` could admit a second job into a
        /// workspace the first one is still mutating. Releasing requires proof that
        /// no child exists: an explicit `child_spawn_attempted: false`, which this
        /// build persists before it asks the runner for a child. An absent flag is
        /// unknown and keeps the lock, as does any recorded `pid`.
        pub fn may_have_orphan_child(&self) -> bool {
            self.child_spawn_attempted.unwrap_or(true) || self.pid.is_some()
        }

        pub fn transition(&mut self, next: RuntimeJobPhase) -> JobResult<()> {
            let allowed = match self.phase {
                RuntimeJobPhase::Queued => matches!(
                    next,
                    RuntimeJobPhase::Running
                        | RuntimeJobPhase::CancelRequested
                        | RuntimeJobPhase::Failed
                        | RuntimeJobPhase::Cancelled
                        | RuntimeJobPhase::Lost
                ),
                RuntimeJobPhase::Running => matches!(
                    next,
                    RuntimeJobPhase::CancelRequested
                        | RuntimeJobPhase::Succeeded
                        | RuntimeJobPhase::Failed
                        | RuntimeJobPhase::Cancelled
                        | RuntimeJobPhase::TimedOut
                        | RuntimeJobPhase::Lost
                ),
                RuntimeJobPhase::CancelRequested => matches!(
                    next,
                    RuntimeJobPhase::Succeeded
                        | RuntimeJobPhase::Failed
                        | RuntimeJobPhase::Cancelled
                        | RuntimeJobPhase::TimedOut
                        | RuntimeJobPhase::Lost
                ),
                RuntimeJobPhase::Succeeded
                | RuntimeJobPhase::Failed
                | RuntimeJobPhase::Cancelled
                | RuntimeJobPhase::TimedOut
                | RuntimeJobPhase::Lost => false,
            };

            if allowed {
                self.phase = next;
                Ok(())
            } else {
                Err(redacted_error(&format!(
                    "runtime job {} cannot transition from {:?} to {:?}",
                    self.id, self.phase, next
                )))
            }
        }
    }

    #[derive(Debug, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub struct CancelMarker {
        pub requested_at_ms: u64,
    }

    #[derive(Debug)]
    pub struct ActiveLifecycleLock {
        pub _held: HeldStateLock,
        pub jobs_root: StateDirectory,
        pub lifecycle: StateFile,
    }

    impl ActiveLifecycleLock {
        pub fn validate(&self) -> io::Result<()> {
            self.jobs_root.validate_named_identity()?;
            self.lifecycle.validate_named_identity()
        }

        pub fn retain_active_lock(&self) -> io::Result<StateFile> {
            self.jobs_root
                .retain_regular_child(std::ffi::OsStr::new("active.lock"))
        }

        pub fn retain_job_directory(&self, id: &str) -> io::Result<StateDirectory> {
            self.validate()?;
            let directory = self
                .jobs_root
                .retain_directory_child(std::ffi::OsStr::new(id))?;
            directory.validate_named_identity()?;
            Ok(directory)
        }

        pub fn release_active_lock_for(
            &self,
            id: &str,
            after_guarded_observation: impl FnOnce(),
        ) -> io::Result<()> {
            self.validate()?;
            let active = match self.retain_active_lock() {
                Ok(active) => active,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
                Err(error) => return Err(error),
            };
            let current = String::from_utf8(active.read_bounded(128)?).map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "runtime active lease is not canonical UTF-8",
                )
            })?;
            if current.trim() != id {
                return Ok(());
            }
            after_guarded_observation();
            self.validate()?;
            active.validate_named_identity()?;
            active.remove_named_identity()
        }
    }

    pub fn lock_is_contended(error: &io::Error) -> bool {
        let expected = fs2::lock_contended_error();
        error.kind() == io::ErrorKind::WouldBlock
            || error
                .raw_os_error()
                .zip(expected.raw_os_error())
                .is_some_and(|(actual, expected)| actual == expected)
    }

    #[derive(Debug, Clone)]
    pub struct RuntimeJobStore {
        pub cache_root: PathBuf,
        pub stale_after: Duration,
    }

    impl RuntimeJobStore {
        pub fn new(cache_root: impl Into<PathBuf>, stale_after: Duration) -> Self {
            Self {
                cache_root: cache_root.into(),
                stale_after,
            }
        }

        pub fn jobs_root(&self) -> PathBuf {
            self.cache_root.join("jobs")
        }

        pub fn retain_jobs_root(&self) -> JobResult<StateDirectory> {
            fs::create_dir_all(&self.cache_root)
                .map_err(|error| io_error("create runtime cache directory", &error))?;
            let canonical_cache = fs::canonicalize(&self.cache_root)
                .map_err(|error| io_error("resolve runtime cache directory", &error))?;
            StateDirectory::open_or_create(&canonical_cache.join("jobs"))
                .map_err(|error| io_error("retain runtime jobs directory", &error))
        }

        pub fn active_lock_path(&self) -> PathBuf {
            self.jobs_root().join("active.lock")
        }

        #[cfg(any(test, feature = "test-support"))]
        pub fn active_lifecycle_lock_path(&self) -> PathBuf {
            self.jobs_root().join("active.lifecycle.lock")
        }

        pub fn recovery_lock_path(&self) -> PathBuf {
            self.jobs_root().join("active.recovery.lock")
        }

        pub fn job_dir(&self, id: &str) -> JobResult<PathBuf> {
            let id = canonical_job_id(id)?;
            Ok(self.jobs_root().join(id))
        }

        pub fn record_path(&self, id: &str) -> JobResult<PathBuf> {
            Ok(self.job_dir(id)?.join("record.json"))
        }

        pub fn stdout_path(&self, id: &str) -> JobResult<PathBuf> {
            Ok(self.job_dir(id)?.join("stdout.log"))
        }

        pub fn stderr_path(&self, id: &str) -> JobResult<PathBuf> {
            Ok(self.job_dir(id)?.join("stderr.log"))
        }

        pub fn cancel_path(&self, id: &str) -> JobResult<PathBuf> {
            Ok(self.job_dir(id)?.join("cancel.json"))
        }

        pub fn acquire_active_lock(&self, id: &str) -> JobResult<()> {
            self.acquire_active_lock_after_lifecycle(id, || {})
        }
        /// The hook runs after the lifecycle guard is held and before the active
        /// lock is claimed, so a test can interleave another process in that window.
        /// Production passes a no-op.
        pub fn acquire_active_lock_after_lifecycle(
            &self,
            id: &str,
            after_lifecycle_lock: impl FnOnce(),
        ) -> JobResult<()> {
            let _lifecycle_lock = self.acquire_active_lifecycle_lock_bounded()?;
            after_lifecycle_lock();
            self.acquire_active_lock_guarded(id)
        }

        pub fn acquire_active_lifecycle_lock(&self) -> JobResult<ActiveLifecycleLock> {
            self.open_active_lifecycle_lock(false)?.ok_or_else(|| {
                redacted_error("active runtime job lifecycle lock unexpectedly remained contended")
            })
        }

        /// Bounded acquisition for the paths that answer a caller: starting a job
        /// and requesting cancellation. The guard is held across process
        /// observation, so an owner that wedges must not turn those calls into an
        /// unbounded wait — a caller is owed a diagnosable refusal it can retry,
        /// and cancellation above all must not depend on the health of the worker
        /// it is trying to stop. The worker's own loop keeps the blocking form,
        /// since it owns the transition it is guarding.
        pub fn acquire_active_lifecycle_lock_bounded(&self) -> JobResult<ActiveLifecycleLock> {
            let deadline = Instant::now()
                .checked_add(LIFECYCLE_LOCK_WAIT)
                .ok_or_else(|| {
                    redacted_error("runtime job lifecycle deadline is unrepresentable")
                })?;
            self.acquire_active_lifecycle_lock_until(deadline)
        }

        pub fn acquire_active_lifecycle_lock_until(
            &self,
            deadline: Instant,
        ) -> JobResult<ActiveLifecycleLock> {
            loop {
                if let Some(lifecycle_lock) = self.try_acquire_active_lifecycle_lock()? {
                    return Ok(lifecycle_lock);
                }
                if Instant::now() >= deadline {
                    return Err(redacted_error(
                    "another runtime job lifecycle transition still holds the workspace guard; \
                     retry once the active job reports progress",
                ));
                }
                thread::sleep(LIFECYCLE_LOCK_RETRY);
            }
        }

        pub fn acquire_active_lifecycle_lock_for_root_bounded(
            &self,
            jobs_root: StateDirectory,
        ) -> JobResult<ActiveLifecycleLock> {
            let deadline = Instant::now()
                .checked_add(LIFECYCLE_LOCK_WAIT)
                .ok_or_else(|| {
                    redacted_error("runtime job lifecycle deadline is unrepresentable")
                })?;
            loop {
                if let Some(lifecycle) =
                    self.open_active_lifecycle_lock_in(jobs_root.clone(), true)?
                {
                    return Ok(lifecycle);
                }
                if Instant::now() >= deadline {
                    return Err(redacted_error(
                        "retained runtime lifecycle transition remained contended",
                    ));
                }
                thread::sleep(LIFECYCLE_LOCK_RETRY);
            }
        }

        pub fn try_acquire_active_lifecycle_lock(&self) -> JobResult<Option<ActiveLifecycleLock>> {
            self.open_active_lifecycle_lock(true)
        }

        pub fn open_active_lifecycle_lock(
            &self,
            nonblocking: bool,
        ) -> JobResult<Option<ActiveLifecycleLock>> {
            let jobs_root = self.retain_jobs_root()?;
            self.open_active_lifecycle_lock_in(jobs_root, nonblocking)
        }

        pub fn open_active_lifecycle_lock_in(
            &self,
            jobs_root: StateDirectory,
            nonblocking: bool,
        ) -> JobResult<Option<ActiveLifecycleLock>> {
            jobs_root
                .validate_named_identity()
                .map_err(|error| io_error("revalidate retained runtime jobs directory", &error))?;
            let lifecycle = jobs_root
                .retain_or_create_regular_child(std::ffi::OsStr::new("active.lifecycle.lock"))
                .map_err(|error| io_error("open active runtime job lifecycle lock", &error))?;
            match lifecycle.lock_exclusive(nonblocking) {
                Ok(held) => Ok(Some(ActiveLifecycleLock {
                    _held: held,
                    jobs_root,
                    lifecycle,
                })),
                Err(StateLockError::Clone(error)) => {
                    Err(io_error("clone active runtime job lifecycle lock", &error))
                }
                Err(StateLockError::Lock(error)) if nonblocking && lock_is_contended(&error) => {
                    Ok(None)
                }
                Err(StateLockError::Lock(error)) => Err(io_error(
                    "acquire active runtime job lifecycle lock",
                    &error,
                )),
            }
        }

        pub fn acquire_active_lock_guarded(&self, id: &str) -> JobResult<()> {
            let mut lock = match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(self.active_lock_path())
            {
                Ok(lock) => lock,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    return Err(self.active_job_conflict());
                }
                Err(error) => return Err(io_error("create active runtime job lock", &error)),
            };
            lock.write_all(id.as_bytes())
                .map_err(|error| io_error("write active runtime job lock", &error))?;
            lock.sync_data()
                .map_err(|error| io_error("sync active runtime job lock", &error))
        }

        pub fn active_job_conflict(&self) -> String {
            match fs::read_to_string(self.active_lock_path()) {
                Ok(id) => {
                    let id = id.trim();
                    let existing = if id.is_empty() { "unknown" } else { id };
                    redacted_error(&format!(
                        "workspace already has active runtime job {existing}{}",
                        self.orphan_retention_hint(existing)
                    ))
                }
                Err(error) => io_error("read active runtime job lock", &error),
            }
        }

        /// A lock held by a `lost` job that reached its child spawn is never released
        /// automatically: whether that child is still mutating the workspace cannot
        /// be decided from the record. Say so in the conflict, otherwise the state is
        /// indistinguishable from a job that is simply still running and the caller
        /// waits for a heartbeat that will never come.
        pub fn orphan_retention_hint(&self, id: &str) -> String {
            match self.read_record(id) {
                Ok(record)
                    if record.phase == RuntimeJobPhase::Lost && record.may_have_orphan_child() =>
                {
                    "; it is lost and holds the lock because its child process may still be running"
                        .to_string()
                }
                _ => String::new(),
            }
        }

        pub fn release_active_lock_for(&self, id: &str) -> JobResult<()> {
            self.release_active_lock_for_after_hooks(id, || {}, || {})
        }

        /// The hook parameters exist so a test can interleave another process at the
        /// two observation points this release has. Production always passes no-ops.
        pub fn release_active_lock_for_after_hooks(
            &self,
            id: &str,
            after_observation: impl FnOnce(),
            after_guarded_observation: impl FnOnce(),
        ) -> JobResult<()> {
            let lock_path = self.active_lock_path();
            let contents = match fs::read_to_string(&lock_path) {
                Ok(contents) => contents,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(error) => return Err(io_error("read active runtime job lock", &error)),
            };
            if contents.trim() == id {
                after_observation();
            } else {
                return Ok(());
            }

            let _lifecycle_lock = self.acquire_active_lifecycle_lock()?;
            self.release_active_lock_guarded(id, after_guarded_observation)
        }

        pub fn release_active_lock_guarded(
            &self,
            id: &str,
            after_guarded_observation: impl FnOnce(),
        ) -> JobResult<()> {
            let lock_path = self.active_lock_path();
            let current = match fs::read_to_string(&lock_path) {
                Ok(contents) => contents,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(error) => return Err(io_error("read active runtime job lock", &error)),
            };
            if current.trim() != id {
                return Ok(());
            }
            after_guarded_observation();
            match fs::remove_file(lock_path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(io_error("remove active runtime job lock", &error)),
            }
        }

        pub fn create_record<R: RuntimeAttempt>(
            &self,
            id: &str,
            request: &R,
        ) -> JobResult<RuntimeJobRecord> {
            let id = canonical_job_id(id)?;
            let directory = self.job_dir(&id)?;
            fs::create_dir_all(&directory)
                .map_err(|error| io_error("create runtime job directory", &error))?;
            let record = RuntimeJobRecord::queued(id.clone(), request);
            self.write_record(&record)?;
            fs::write(directory.join("stdout.log"), "")
                .map_err(|error| io_error("create runtime job stdout log", &error))?;
            fs::write(directory.join("stderr.log"), "")
                .map_err(|error| io_error("create runtime job stderr log", &error))?;
            Ok(record)
        }

        pub fn enqueue<R: RuntimeAttempt>(
            &self,
            id: &str,
            request: &R,
        ) -> JobResult<RuntimeJobRecord> {
            if let Err(error) = self.acquire_active_lock(id) {
                if !self.recover_stale_active()? {
                    return Err(error);
                }
                self.acquire_active_lock(id)?;
            }
            match self.create_record(id, request) {
                Ok(record) => Ok(record),
                Err(error) => {
                    let _ = self.release_active_lock_for(id);
                    Err(error)
                }
            }
        }

        pub fn recover_stale_active(&self) -> JobResult<bool> {
            fs::create_dir_all(self.jobs_root())
                .map_err(|error| io_error("create runtime jobs directory", &error))?;
            let recovery_lock = match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(self.recovery_lock_path())
            {
                Ok(lock) => lock,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return Ok(false),
                Err(error) => return Err(io_error("create runtime job recovery lock", &error)),
            };
            let result = match self.try_acquire_active_lifecycle_lock() {
                Ok(Some(_lifecycle_lock)) => self.recover_stale_active_locked(),
                Ok(None) => Ok(false),
                Err(error) => Err(error),
            };
            drop(recovery_lock);
            match fs::remove_file(self.recovery_lock_path()) {
                Ok(()) => result,
                Err(error) if error.kind() == io::ErrorKind::NotFound => result,
                Err(error) => Err(io_error("remove runtime job recovery lock", &error)),
            }
        }

        pub fn recover_stale_active_locked(&self) -> JobResult<bool> {
            let id = match fs::read_to_string(self.active_lock_path()) {
                Ok(id) => id.trim().to_string(),
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
                Err(error) => return Err(io_error("read active runtime job lock", &error)),
            };
            if self.record_is_provably_absent(&id) {
                // The lock is created before the record it names. A parent that died
                // inside that window never reached the spawn, so no child exists and
                // no worker will ever claim this id. A parent still inside that
                // window keeps its lock and is not reported as recoverable.
                if !self.active_lock_outlived_stale_window()? {
                    return Ok(false);
                }
                self.release_active_lock_guarded(&id, || {})?;
                return Ok(true);
            }
            let mut record = self.read_record(&id)?;
            if record.phase.is_terminal() {
                // A terminal write can succeed while the final lock removal fails.
                // Its own id makes this cleanup safe. A lost job that reached its
                // child spawn is the exception: that child may still be alive after
                // a worker crash, so it keeps the lock it owns.
                if record.phase == RuntimeJobPhase::Lost && record.may_have_orphan_child() {
                    return Ok(false);
                }
                self.release_active_lock_guarded(&id, || {})?;
                return Ok(true);
            }
            if !self.stale(&record) {
                return Ok(false);
            }
            record.transition(RuntimeJobPhase::Lost)?;
            record.finished_at_ms = Some(now_millis());
            record.warnings.push("stale worker heartbeat".to_string());
            self.write_record(&record)?;
            if record.may_have_orphan_child() {
                return Ok(false);
            }
            // Nothing ever spawned for this job, so the workspace it was holding is
            // provably idle. The release is scoped to this id and leaves a lock that
            // a replacement owner has already claimed untouched.
            self.release_active_lock_guarded(&id, || {})?;
            Ok(true)
        }

        /// Whether the durable record named by an id is *provably* absent. Only a
        /// successful negative answer counts: an id that is not a job id, and a path
        /// whose metadata cannot be read at all — a permission error, an I/O error,
        /// a broken link — prove nothing and must never be mistaken for proof that
        /// no child exists. `Path::exists` cannot express that difference because it
        /// reports every such failure as `false`. A record that is present but
        /// corrupt or of an unknown schema is likewise not absent.
        pub fn record_is_provably_absent(&self, id: &str) -> bool {
            let jobs_root = self.jobs_root();
            let jobs_root_metadata = match fs::symlink_metadata(&jobs_root) {
                Ok(metadata) => metadata,
                Err(_) => return false,
            };
            if metadata_is_link_or_reparse_point(&jobs_root_metadata)
                || !jobs_root_metadata.is_dir()
            {
                return false;
            }

            let Ok(job_dir) = self.job_dir(id) else {
                return false;
            };
            match fs::symlink_metadata(&job_dir) {
                Ok(metadata)
                    if metadata.is_dir() && !metadata_is_link_or_reparse_point(&metadata) => {}
                Ok(_) => return false,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return true,
                Err(_) => return false,
            }

            match fs::symlink_metadata(job_dir.join("record.json")) {
                Ok(_) => false,
                Err(error) => error.kind() == io::ErrorKind::NotFound,
            }
        }

        /// Whether `active.lock` itself has outlived the staleness window. The lock
        /// file carries its own creation time, which is the only clock available for
        /// a job that never got far enough to write a heartbeat.
        pub fn active_lock_outlived_stale_window(&self) -> JobResult<bool> {
            let metadata = match fs::metadata(self.active_lock_path()) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
                Err(error) => return Err(io_error("read active runtime job lock", &error)),
            };
            let Ok(created) = metadata.modified() else {
                return Ok(false);
            };
            Ok(created
                .elapsed()
                .map(|age| age > self.stale_after)
                .unwrap_or(false))
        }

        pub fn read_record(&self, id: &str) -> JobResult<RuntimeJobRecord> {
            let path = self.record_path(id)?;
            let contents = fs::read(&path).map_err(|error| {
                if error.kind() == std::io::ErrorKind::NotFound {
                    redacted_error(&format!("runtime job {id} record is missing"))
                } else {
                    io_error("read runtime job record", &error)
                }
            })?;
            self.decode_record(id, &contents)
        }

        pub fn read_retained_record(
            &self,
            id: &str,
            record: &StateFile,
        ) -> JobResult<RuntimeJobRecord> {
            let contents = record
                .read_bounded(RUNTIME_RECORD_BYTES)
                .map_err(|error| io_error("read retained runtime job record", &error))?;
            self.decode_record(id, &contents)
        }

        pub fn decode_record(&self, id: &str, contents: &[u8]) -> JobResult<RuntimeJobRecord> {
            let record: RuntimeJobRecord = serde_json::from_slice(contents).map_err(|error| {
                redacted_error(&format!("runtime job {id} record is corrupt: {error}"))
            })?;
            if record.schema_version != RECORD_SCHEMA_VERSION {
                return Err(redacted_error(&format!(
                    "runtime job {id} has unsupported schema version {}",
                    record.schema_version
                )));
            }
            let canonical_id = canonical_job_id(&record.id)?;
            if canonical_id != canonical_job_id(id)? {
                return Err(redacted_error(&format!(
                    "runtime job {id} record id is corrupt"
                )));
            }
            Ok(record)
        }

        pub fn write_record_in_retained_job(
            &self,
            job_directory: &StateDirectory,
            record: &RuntimeJobRecord,
        ) -> JobResult<StateFile> {
            #[cfg(any(test, feature = "test-support"))]
            if INJECT_RUNTIME_RECORD_WRITE_FAILURE.swap(false, std::sync::atomic::Ordering::AcqRel)
            {
                return Err(redacted_error("injected runtime record write failure"));
            }
            let bytes = serde_json::to_vec_pretty(record).map_err(|error| {
                redacted_error(&format!("serialize runtime job record: {error}"))
            })?;
            if bytes.len() > RUNTIME_RECORD_BYTES {
                return Err(redacted_error(
                    "runtime job record exceeds its bounded size",
                ));
            }
            let stage = format!(".record.json.{}.tmp", Uuid::new_v4());
            job_directory
                .replace_regular_child_atomically(
                    std::ffi::OsStr::new(&stage),
                    std::ffi::OsStr::new("record.json"),
                    &bytes,
                )
                .map_err(|error| io_error("publish retained runtime job record", error.io_error()))
        }

        pub fn write_record(&self, record: &RuntimeJobRecord) -> JobResult<()> {
            #[cfg(any(test, feature = "test-support"))]
            if INJECT_RUNTIME_RECORD_WRITE_FAILURE.swap(false, std::sync::atomic::Ordering::AcqRel)
            {
                return Err(redacted_error("injected runtime record write failure"));
            }
            let path = self.record_path(&record.id)?;
            let parent = path
                .parent()
                .ok_or_else(|| redacted_error("runtime job record path has no parent"))?;
            fs::create_dir_all(parent)
                .map_err(|error| io_error("create runtime job record directory", &error))?;
            let bytes = serde_json::to_vec_pretty(record).map_err(|error| {
                redacted_error(&format!("serialize runtime job record: {error}"))
            })?;
            atomic_write(&path, &bytes)
        }

        pub fn read_record_in_retained_job(
            &self,
            id: &str,
            job_directory: &StateDirectory,
        ) -> JobResult<RuntimeJobRecord> {
            let record = job_directory
                .retain_regular_child(std::ffi::OsStr::new("record.json"))
                .map_err(|error| {
                    if error.kind() == io::ErrorKind::NotFound {
                        redacted_error(&format!("runtime job {id} record is missing"))
                    } else {
                        io_error("retain runtime job record", &error)
                    }
                })?;
            self.read_retained_record(id, &record)
        }

        /// Rewrites both log tails through the retained job directory handle, so a
        /// directory installed later under the job name never receives them.
        pub fn write_logs_in_retained_job(
            &self,
            job_directory: &StateDirectory,
            output: &RuntimeJobOutput,
        ) -> JobResult<()> {
            for (name, text, context) in [
                ("stdout.log", &output.stdout, "write runtime job stdout log"),
                ("stderr.log", &output.stderr, "write runtime job stderr log"),
            ] {
                let tail = bounded_redacted_tail(text, OUTPUT_TAIL_BYTES);
                job_directory
                    .retain_or_create_regular_child(std::ffi::OsStr::new(name))
                    .and_then(|log| log.overwrite_retained(tail.as_bytes()))
                    .map_err(|error| io_error(context, &error))?;
            }
            Ok(())
        }

        pub fn has_cancel_marker_in_retained_job(
            &self,
            job_directory: &StateDirectory,
        ) -> JobResult<bool> {
            let marker =
                match job_directory.retain_regular_child(std::ffi::OsStr::new("cancel.json")) {
                    Ok(marker) => marker,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
                    Err(error) => return Err(io_error("read runtime job cancellation", &error)),
                };
            let contents = marker
                .read_bounded(CANCEL_MARKER_BYTES)
                .map_err(|error| io_error("read runtime job cancellation", &error))?;
            serde_json::from_slice::<CancelMarker>(&contents).map_err(|error| {
                redacted_error(&format!("runtime job cancellation is corrupt: {error}"))
            })?;
            Ok(true)
        }

        pub fn write_cancel_marker(&self, id: &str) -> JobResult<()> {
            let marker = CancelMarker {
                requested_at_ms: now_millis(),
            };
            let bytes = serde_json::to_vec(&marker).map_err(|error| {
                redacted_error(&format!("serialize runtime job cancellation: {error}"))
            })?;
            atomic_write(&self.cancel_path(id)?, &bytes)
        }

        pub fn has_cancel_marker(&self, id: &str) -> JobResult<bool> {
            let path = self.cancel_path(id)?;
            let contents = match fs::read_to_string(path) {
                Ok(contents) => contents,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
                Err(error) => return Err(io_error("read runtime job cancellation", &error)),
            };
            serde_json::from_str::<CancelMarker>(&contents).map_err(|error| {
                redacted_error(&format!("runtime job cancellation is corrupt: {error}"))
            })?;
            Ok(true)
        }

        pub fn snapshot_with_cancel_intent(
            &self,
            record: &RuntimeJobRecord,
            wait_timed_out: bool,
        ) -> JobResult<RuntimeJobSnapshot> {
            let mut snapshot = record.snapshot(wait_timed_out);
            if !record.phase.is_terminal() && self.has_cancel_marker(&record.id)? {
                snapshot.phase = RuntimeJobPhase::CancelRequested;
                if record.cancel_policy == CancelPolicy::Critical {
                    snapshot.cancel_deferred = true;
                    snapshot.unsafe_phase = Some(record.operation.clone());
                }
            }
            Ok(snapshot)
        }

        pub fn list(&self) -> RuntimeJobList {
            let entries = match fs::read_dir(self.jobs_root()) {
                Ok(entries) => entries,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return RuntimeJobList {
                        jobs: Vec::new(),
                        warnings: Vec::new(),
                    };
                }
                Err(error) => {
                    return RuntimeJobList {
                        jobs: Vec::new(),
                        warnings: vec![io_error("list runtime jobs", &error)],
                    };
                }
            };

            let mut jobs = Vec::new();
            let mut warnings = Vec::new();
            for entry in entries {
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(error) => {
                        warnings.push(io_error("read runtime jobs entry", &error));
                        continue;
                    }
                };
                let file_type = match entry.file_type() {
                    Ok(file_type) => file_type,
                    Err(error) => {
                        warnings.push(io_error("read runtime job entry type", &error));
                        continue;
                    }
                };
                if !file_type.is_dir() {
                    continue;
                }
                let name = entry.file_name();
                let Some(id) = name.to_str() else {
                    warnings.push(redacted_error(
                        "runtime job directory name is not valid UTF-8",
                    ));
                    continue;
                };
                match self
                    .read_record(id)
                    .and_then(|record| self.snapshot_with_cancel_intent(&record, false))
                {
                    Ok(snapshot) => jobs.push(snapshot),
                    Err(error) => warnings.push(error),
                }
            }
            jobs.sort_by_key(|job| job.created_at_ms);
            RuntimeJobList { jobs, warnings }
        }

        pub fn stale(&self, record: &RuntimeJobRecord) -> bool {
            let Some(heartbeat) = record.heartbeat_at_ms else {
                return false;
            };
            let threshold = duration_millis(self.stale_after);
            now_millis().saturating_sub(heartbeat) > threshold
        }
    }

    #[derive(Clone)]
    pub struct RetainedRuntimeJobAuthority {
        pub jobs_root: StateDirectory,
        pub job_directory: StateDirectory,
    }

    pub struct ActiveRuntimeJobProcess<R: RuntimeAttempt> {
        pub process: Box<dyn RuntimeJobProcess>,
        pub jobs_root: StateDirectory,
        /// Exact descriptor authority admitted before the process could start.
        /// Quarantine retries must never re-resolve `jobs/<id>` from its name.
        pub job_directory: StateDirectory,
        pub attempt_argv: Vec<String>,
        pub attempt: Option<R>,
        pub scope: Option<RuntimeAttemptScope>,
        pub fallback: Option<R>,
        pub previous_output: Option<RuntimeJobOutput>,
    }
}
use owner_state::*;

impl<R: RuntimeAttempt> RuntimeJobService<R> {
    #[cfg(any(test, feature = "test-support"))]
    pub fn test_store(&self) -> &test_support::RuntimeJobStore {
        &self.store
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn test_processes(
        &self,
    ) -> &Mutex<HashMap<String, test_support::ActiveRuntimeJobProcess<R>>> {
        &self.processes
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn test_join_shared_work_after_capability<W>(
        &self,
        id: &str,
        owner: &RuntimeResourceOwner,
        work: W,
        after_capability: impl FnOnce(),
    ) -> JobResult<SharedWorkLease<RuntimeWorkKey, (), RuntimeWorkFailure>>
    where
        W: FnOnce(SharedWorkProducer) -> Result<(), RuntimeWorkFailure> + Send + 'static,
    {
        self.join_shared_work_after_capability(id, owner, work, after_capability)
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn test_with_stale_after(
        cache_root: impl Into<PathBuf>,
        runner: Arc<dyn RuntimeJobRunner<Request = R>>,
        stale_after: Duration,
    ) -> Self {
        Self::with_stale_after(cache_root, runner, stale_after)
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn test_start(&self, request: R) -> JobResult<RuntimeJobSnapshot> {
        self.start(request)
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn test_activate_enqueued(&self, id: &str, request: &R) -> JobResult<RuntimeJobSnapshot> {
        self.activate_enqueued(id, request)
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn test_activate_enqueued_after_preflight(
        &self,
        id: &str,
        request: &R,
        after_preflight: impl FnOnce(),
    ) -> JobResult<RuntimeJobSnapshot> {
        self.activate_enqueued_after_preflight(id, request, after_preflight)
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn test_activate_enqueued_after_hooks(
        &self,
        id: &str,
        request: &R,
        after_preflight: impl FnOnce(),
        before_activation_guard: impl FnOnce(),
    ) -> JobResult<RuntimeJobSnapshot> {
        self.activate_enqueued_after_hooks(id, request, after_preflight, before_activation_guard)
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn test_status(&self, id: &str) -> JobResult<RuntimeJobSnapshot> {
        self.status(id)
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn test_set_directory_hook(
        &self,
        hook: impl FnMut(DirectoryAuthorityHookPoint) + Send + 'static,
    ) {
        self.set_directory_hook(hook)
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn test_wait(&self, id: &str, caller_timeout: Duration) -> JobResult<RuntimeJobSnapshot> {
        self.wait(id, caller_timeout)
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn test_logs(&self, id: &str) -> JobResult<RuntimeJobLogs> {
        self.logs(id)
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn test_cancel(&self, id: &str) -> JobResult<RuntimeJobSnapshot> {
        self.cancel(id)
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn test_list(&self) -> RuntimeJobList {
        self.list()
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn test_lock_processes(
        &self,
    ) -> JobResult<std::sync::MutexGuard<'_, HashMap<String, ActiveRuntimeJobProcess<R>>>> {
        self.lock_processes()
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn test_has_retained_process(&self, id: &str) -> bool {
        self.has_retained_process(id)
    }
}

/// Explicit owner-state observations and fault injection for backend integration.
/// Enabled only by dev dependencies; no source writer or descriptor escape.
#[cfg(any(test, feature = "test-support"))]
pub mod test_support {
    pub fn register_pending_quarantine_fixture(
        owner: super::TestFixtureOwnerToken,
        service_root: std::path::PathBuf,
        job_id: String,
        handle: std::thread::JoinHandle<()>,
        finished: std::sync::mpsc::Receiver<()>,
    ) {
        super::register_test_quarantine_supervisor(
            owner,
            service_root,
            job_id,
            false,
            super::QuarantineSupervisor { handle, finished },
        );
    }
    pub use super::owner_state::{
        ActiveLifecycleLock, ActiveRuntimeJobProcess, CancelMarker, RetainedRuntimeJobAuthority,
        RuntimeJobRecord, RuntimeJobStore,
    };
    use super::*;
    pub use super::{
        acquire_test_service_admission, bind_test_service_root, create_test_fixture_owner,
        drain_test_fixture_owner, register_test_supervisor_owner, test_fixture_registry,
        DirectoryAuthorityHookPoint, QuarantineReleaseHookPoint, TestFixtureOwnerState,
        TestFixtureOwnerToken, TestFixtureRegistry, TestQuarantineSupervisor, TestServiceAdmission,
        TestSupervisorScope,
    };
    pub const RECORD_SCHEMA_VERSION: u8 = super::RECORD_SCHEMA_VERSION;
    pub const DEFAULT_STALE_AFTER: Duration = super::DEFAULT_STALE_AFTER;
    pub const LIFECYCLE_LOCK_WAIT: Duration = super::LIFECYCLE_LOCK_WAIT;
    pub const OUTPUT_TAIL_BYTES: usize = super::OUTPUT_TAIL_BYTES;
    pub const RUNTIME_RECORD_BYTES: usize = super::RUNTIME_RECORD_BYTES;
    pub const CANCEL_MARKER_BYTES: usize = super::CANCEL_MARKER_BYTES;
    pub static RECORD_WRITE_FAILURE: &std::sync::atomic::AtomicBool =
        &super::INJECT_RUNTIME_RECORD_WRITE_FAILURE;
    pub static QUARANTINE_THREAD_SPAWN_FAILURE: &std::sync::atomic::AtomicBool =
        &super::INJECT_QUARANTINE_THREAD_SPAWN_FAILURE;
    pub fn try_release_owned_quarantine_once<R: RuntimeAttempt>(
        store: &RuntimeJobStore,
        job_id: &str,
        active: &mut ActiveRuntimeJobProcess<R>,
    ) -> JobResult<bool> {
        super::try_release_owned_quarantine_once(store, job_id, active)
    }
    pub fn try_release_owned_quarantine_once_after_terminal<R: RuntimeAttempt>(
        store: &RuntimeJobStore,
        job_id: &str,
        active: &mut ActiveRuntimeJobProcess<R>,
        after_terminal_proof: impl FnOnce(),
    ) -> JobResult<bool> {
        super::try_release_owned_quarantine_once_after_terminal(
            store,
            job_id,
            active,
            after_terminal_proof,
        )
    }
    pub fn try_release_owned_quarantine_once_after_record_hooks<R: RuntimeAttempt>(
        store: &RuntimeJobStore,
        job_id: &str,
        active: &mut ActiveRuntimeJobProcess<R>,
        hook: impl FnMut(QuarantineReleaseHookPoint),
    ) -> JobResult<bool> {
        super::try_release_owned_quarantine_once_after_record_hooks(store, job_id, active, hook)
    }
    pub fn worker_start_result(
        cache_root: &Path,
        queued: RuntimeJobSnapshot,
        cancellation_observed: bool,
    ) -> JobResult<RuntimeJobSnapshot> {
        super::worker_start_result(cache_root, queued, cancellation_observed)
    }
    pub fn worker_start_result_guarded(
        store: &RuntimeJobStore,
        queued: RuntimeJobSnapshot,
        cancellation_observed: bool,
    ) -> JobResult<RuntimeJobSnapshot> {
        super::worker_start_result_guarded(store, queued, cancellation_observed)
    }
    pub fn settle_worker_handoff_guarded(
        store: &RuntimeJobStore,
        queued: RuntimeJobSnapshot,
        handoff_result: JobResult<bool>,
        terminate_worker: impl FnOnce(),
    ) -> JobResult<RuntimeJobSnapshot> {
        super::settle_worker_handoff_guarded(store, queued, handoff_result, terminate_worker)
    }
    pub fn cancel_queued_job_after_guard(
        cache_root: &Path,
        id: &str,
        after_guard: impl FnOnce(),
    ) -> JobResult<()> {
        super::cancel_queued_job_after_guard(cache_root, id, after_guard)
    }
    pub fn enqueue_cancellable_runtime_job_after_hook<R: RuntimeAttempt>(
        cache_root: &Path,
        request: &R,
        cancellation: &CancellationToken,
        after_enqueue: impl FnOnce(),
    ) -> JobResult<RuntimeJobSnapshot> {
        super::enqueue_cancellable_runtime_job_after_hook(
            cache_root,
            request,
            cancellation,
            after_enqueue,
        )
    }
    pub fn atomic_write(path: &Path, bytes: &[u8]) -> JobResult<()> {
        super::atomic_write(path, bytes)
    }
    pub fn now_millis() -> u64 {
        super::now_millis()
    }
    pub fn redacted_error(message: &str) -> String {
        super::redacted_error(message)
    }
}

#[cfg(test)]
mod retry_scope_tests;

#[cfg(any(test, feature = "test-support"))]
impl TestQuarantineSupervisor {
    pub fn test_thread_id(&self) -> std::thread::ThreadId {
        self.supervisor.handle.thread().id()
    }
    pub fn test_join(self) -> std::thread::Result<()> {
        self.supervisor.handle.join()
    }
}
