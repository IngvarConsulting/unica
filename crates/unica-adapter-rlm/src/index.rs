use crate::environment::RlmEnvironment;
use unica_analysis_services::backend::BackgroundTaskTracker;
use unica_analysis_services::protocol::IndexMaintenanceStatus;
use unica_workspace::paths::{
    metadata_is_link_or_reparse_point, path_starts_with_host_root, provider_state_path_identity,
};
#[cfg(test)]
use unica_workspace::state_fs::create_new_state_stage;
use unica_workspace::state_fs::{
    is_file_lock_contended, replace_file_atomically, sync_parent_directory,
};

use crate::generation_collection::spawn_stale_rlm_generation_collection;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
#[cfg(any(test, feature = "test-support"))]
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::io::{ErrorKind, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use unica_code_intelligence::model::IndexMaintenanceEvidence;
use unica_process::{ensure_truncation_diagnostics, ManagedChild, ManagedCommand, ManagedOutput};
use unica_source_contract::workspace::WorkspaceContext;
use unica_work_control::cancellation::{cancelled_error, CancellationToken, CANCELLED_PREFIX};
use unica_workspace::actor::ProviderSourceLease;
use unica_workspace::paths::normalize_path_identity;
use unica_workspace::state_fs::{
    create_state_stage, open_state_lock, open_state_reader, state_file_identity,
    StateHandle as File,
};
use unica_workspace::state_scope::WorkspaceStateScope;

#[cfg(any(test, feature = "test-support"))]
type IsolatedBuildStartObserver = Box<dyn Fn(&Path)>;

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static ISOLATED_BUILD_START_OBSERVER: RefCell<Option<IsolatedBuildStartObserver>> = RefCell::new(None);
}

#[cfg(any(test, feature = "test-support"))]
pub fn set_isolated_build_start_observer_for_test(observer: Option<IsolatedBuildStartObserver>) {
    ISOLATED_BUILD_START_OBSERVER.with(|slot| {
        *slot.borrow_mut() = observer;
    });
}

#[cfg(any(test, feature = "test-support"))]
fn observe_isolated_build_start(source_root: &Path) {
    ISOLATED_BUILD_START_OBSERVER.with(|slot| {
        if let Some(observer) = slot.borrow().as_ref() {
            observer(source_root);
        }
    });
}

const INDEX_TIMEOUT: Duration = Duration::from_secs(30);
const LOCK_STALE_AFTER: Duration = Duration::from_secs(10 * 60);
const LOCK_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
const LOCK_SCHEMA_VERSION: u32 = 1;
const RLM_PRODUCT_DIR: &str = "rlm-bsl";
const RLM_INDEX_GENERATION: &str = "index-v17";
const RLM_PYTHON_UTF8: &str = "1";
const RLM_PYTHON_IO_ENCODING: &str = "utf-8:surrogateescape";
const STATUS_FILE_NAME: &str = "bsl_index_status.json";
const LOCK_FILE_NAME: &str = "bsl_index.lock";

pub fn rlm_provider_state_root(
    context: &WorkspaceContext,
    source_root: &Path,
) -> Result<PathBuf, String> {
    rlm_provider_state_root_with(context, source_root, neutral_provider_state_root())
}

pub fn rlm_process_environment(generation_root: PathBuf) -> Vec<(OsString, OsString)> {
    vec![
        (
            OsString::from("RLM_INDEX_DIR"),
            generation_root.into_os_string(),
        ),
        (
            OsString::from("PYTHONUTF8"),
            OsString::from(RLM_PYTHON_UTF8),
        ),
        (
            OsString::from("PYTHONIOENCODING"),
            OsString::from(RLM_PYTHON_IO_ENCODING),
        ),
    ]
}

fn rlm_provider_state_root_with(
    context: &WorkspaceContext,
    source_root: &Path,
    external_base: Option<PathBuf>,
) -> Result<PathBuf, String> {
    let preferred = normalize_path_identity(&context.cache_root)?;
    let workspace = normalize_path_identity(&context.workspace_root)?;
    let source = normalize_path_identity(source_root)?;
    let base = if !path_starts_with_host_root(&preferred, &source) {
        preferred.join("provider-state")
    } else {
        external_base.ok_or_else(|| {
            "UNICA_PROVIDER_STATE_DIR, HOME, or USERPROFILE is required for RLM state outside sourceRoot".to_string()
        })?
    };
    let mut hasher = Sha256::new();
    for component in [&workspace, &source] {
        hasher.update(provider_state_path_identity(component));
        hasher.update([0]);
    }
    let identity = format!("{:x}", hasher.finalize());
    let root = normalize_path_identity(&base.join(format!("rlm-{identity}")))?;
    if path_starts_with_host_root(&root, &source) {
        return Err("failed to place RLM state outside the indexed source tree".to_string());
    }
    Ok(root)
}

fn neutral_provider_state_root() -> Option<PathBuf> {
    let root = neutral_provider_state_root_with(|name| std::env::var_os(name))?;
    Some(unica_workspace::state_fs::state_root_with_resolved_ancestors(&root).unwrap_or(root))
}

fn neutral_provider_state_root_with(
    read_env: impl Fn(&str) -> Option<std::ffi::OsString>,
) -> Option<PathBuf> {
    read_env("UNICA_PROVIDER_STATE_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            read_env("HOME")
                .filter(|value| !value.is_empty())
                .or_else(|| read_env("USERPROFILE").filter(|value| !value.is_empty()))
                .map(PathBuf::from)
                .map(|home| home.join(".unica").join("provider-state"))
        })
}

pub fn rlm_generation_root(
    context: &WorkspaceContext,
    source_root: &Path,
) -> Result<PathBuf, String> {
    let pair_root = rlm_provider_state_root(context, source_root)?;
    checked_generation_route(
        &pair_root,
        pair_root.join(RLM_PRODUCT_DIR).join(RLM_INDEX_GENERATION),
    )
}

fn rlm_build_root(
    context: &WorkspaceContext,
    source_root: &Path,
    build_id: &str,
) -> Result<PathBuf, String> {
    if build_id.len() != 32 || !build_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("invalid RLM index build identifier".to_string());
    }
    let pair_root = rlm_provider_state_root(context, source_root)?;
    checked_generation_route(
        &pair_root,
        pair_root
            .join(RLM_PRODUCT_DIR)
            .join(RLM_INDEX_GENERATION)
            .join("builds")
            .join(build_id),
    )
}

/// The active marker names a usable, immutable build. It is not evidence that
/// the indexed source bytes still match the working tree.
pub fn usable_index_build(
    context: &WorkspaceContext,
    source_root: &Path,
) -> Result<Option<UsableIndexBuild>, String> {
    let Some(status) = read_bsl_index_status(context, source_root)? else {
        return Ok(None);
    };
    if status.status != "ready" || !stored_path_matches(status.source_root.as_deref(), source_root)
    {
        return Ok(None);
    }
    // Legacy markers point to a mutable generation directory. They cannot
    // pin a build for readers; an isolated build must replace the marker.
    let Some(build_id) = status.build_id else {
        return Ok(None);
    };
    let directory = rlm_build_root(context, source_root, &build_id)?;
    let Some(db_path) = status.db_path.map(PathBuf::from) else {
        return Ok(None);
    };
    if !usable_db_in_generation(&db_path, &directory) {
        return Ok(None);
    }
    Ok(Some(UsableIndexBuild {
        id: build_id,
        directory,
        db_path,
    }))
}

fn usable_db_in_generation(db_path: &Path, directory: &Path) -> bool {
    fs::symlink_metadata(db_path)
        .ok()
        .is_some_and(|metadata| metadata.is_file() && !metadata_is_link_or_reparse_point(&metadata))
        && normalized_path_is_within(db_path, directory)
        && fs::canonicalize(db_path)
            .ok()
            .zip(fs::canonicalize(directory).ok())
            .is_some_and(|(db, root)| db.starts_with(root))
}

fn checked_generation_route(pair_root: &Path, route: PathBuf) -> Result<PathBuf, String> {
    let relative = route.strip_prefix(pair_root).map_err(|error| {
        format!(
            "RLM generation route is outside provider state root {}: {error}",
            pair_root.display()
        )
    })?;
    let mut current = pair_root.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata_is_link_or_reparse_point(&metadata) => {
                return Err(format!(
                    "RLM generation route contains a symbolic link or reparse point: {}",
                    current.display()
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "failed to inspect RLM generation route {}: {error}",
                    current.display()
                ));
            }
        }
    }
    Ok(route)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexReadiness {
    Ready { db_path: PathBuf },
    Missing,
    Stale { status: String },
    Building,
    Queued,
    Incomplete,
    MaintenanceUnavailable(IndexMaintenanceEvidence),
    Failed(String),
    Unavailable(String),
}

impl IndexReadiness {
    pub fn maintenance_evidence(&self) -> Option<IndexMaintenanceEvidence> {
        match self {
            Self::Queued => Some(IndexMaintenanceEvidence::Queued),
            Self::Building => Some(IndexMaintenanceEvidence::Running),
            Self::MaintenanceUnavailable(evidence) => Some(*evidence),
            _ => None,
        }
    }
}

#[derive(Debug)]
enum MaintenanceObservation {
    Prepared,
    Observed(IndexMaintenanceStatus),
}

type MaintenanceObserver = Arc<Mutex<MaintenanceObservation>>;

fn observed_status(observer: &MaintenanceObserver) -> IndexMaintenanceStatus {
    match &*observer.lock().unwrap_or_else(|error| error.into_inner()) {
        MaintenanceObservation::Prepared => IndexMaintenanceStatus::Unknown,
        MaintenanceObservation::Observed(status) => status.clone(),
    }
}

#[derive(Debug, Clone, Default)]
pub struct IndexStartReport {
    pub warnings: Vec<String>,
    pub maintenance: IndexMaintenanceStatus,
}

impl IndexStartReport {
    pub fn observed(maintenance: IndexMaintenanceStatus) -> Self {
        Self {
            warnings: vec![maintenance.summary()],
            maintenance,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BslIndexStatus {
    pub status: String,
    pub source_root: Option<String>,
    pub db_path: Option<String>,
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_class: Option<BslIndexFailureClass>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_action: Option<BslIndexNextAction>,
    pub updated_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_run: Option<BslIndexRunMetrics>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsableIndexBuild {
    pub id: String,
    pub directory: PathBuf,
    pub db_path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BslIndexNextAction {
    Update,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BslIndexFailureClass {
    Retryable,
    Terminal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BslIndexRunMetrics {
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recovery_reason: Option<String>,
    pub duration_ms: u64,
    pub started_at: u64,
    pub finished_at: u64,
    pub timed_out: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modules: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub methods: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub db_size: Option<String>,
}

#[derive(Debug, Clone)]
pub struct IndexCommand {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub cwd: PathBuf,
    pub env: Vec<(OsString, OsString)>,
    pub timeout: Duration,
    pub cancellation: CancellationToken,
}

#[derive(Debug, Clone)]
pub struct IndexOutput {
    pub status_success: bool,
    pub status: String,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
    pub cancelled: bool,
    pub duration_ms: u64,
}

#[derive(Debug)]
pub struct IndexBackgroundJob {
    pub action: String,
    pub context: WorkspaceContext,
    pub source_root: PathBuf,
    pub source_lease: Option<ProviderSourceLease>,
    pub primary: IndexCommand,
    pub info: IndexCommand,
    pub status_path: PathBuf,
    #[cfg(any(test, feature = "test-support"))]
    pub lock_path: PathBuf,
    pub lock_lease: IndexLockLease,
}

#[cfg(any(test, feature = "test-support"))]
impl IndexBackgroundJob {
    pub fn run_with_for_test(
        self,
        run: impl FnMut(&IndexCommand, &mut IndexLockLease) -> Result<IndexOutput, String>,
    ) {
        run_background_job_with(self, run);
    }
}

struct IndexStartSpec {
    action: &'static str,
    source_root: PathBuf,
    primary: IndexCommand,
    info: IndexCommand,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BslIndexLock {
    schema_version: u32,
    lock_id: String,
    owner_pid: u32,
    action: String,
    source_root: String,
    started_at: u64,
    updated_at: u64,
    #[serde(default = "default_lock_state")]
    state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    child_pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    released_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

pub trait IndexRunner {
    fn start_background(&self, job: IndexBackgroundJob) -> Result<(), String>;
}

pub struct SystemIndexRunner {
    tracker: Option<Arc<dyn BackgroundTaskTracker>>,
}

impl SystemIndexRunner {
    pub fn tracked(tracker: Arc<dyn BackgroundTaskTracker>) -> Self {
        Self {
            tracker: Some(tracker),
        }
    }
}

pub static SYSTEM_INDEX_RUNNER: SystemIndexRunner = SystemIndexRunner { tracker: None };

pub struct WorkspaceIndexService<'a> {
    runner: &'a dyn IndexRunner,
    source_lease: Option<ProviderSourceLease>,
    state_scope: WorkspaceStateScope,
    environment: &'a dyn RlmEnvironment,
}

impl<'a> WorkspaceIndexService<'a> {
    pub fn active_build(
        &self,
        context: &WorkspaceContext,
        source_root: &Path,
    ) -> Result<Option<UsableIndexBuild>, String> {
        self.validate_bound_source_root(source_root)?;
        let state_context = self.state_context(context, source_root)?;
        // Generations an earlier RLM left behind are removed in the background;
        // a busy or failing removal never affects this read.
        if let Ok(pair_root) = rlm_provider_state_root(&state_context, source_root) {
            spawn_stale_rlm_generation_collection(pair_root, RLM_INDEX_GENERATION);
        }
        usable_index_build(&state_context, source_root)
    }

    pub fn maintenance_status(
        &self,
        context: &WorkspaceContext,
        source_root: &Path,
    ) -> Option<IndexMaintenanceStatus> {
        self.validate_bound_source_root(source_root).ok()?;
        let context = self.state_context(context, source_root).ok()?;
        observed_maintenance(&lock_path(&context, source_root).ok()?)
    }

    /// Start a build in a new directory. The active marker is replaced only
    /// after the candidate reports a usable DB, so existing readers keep their
    /// own build while maintenance runs or fails.
    pub fn start_isolated_build(
        &self,
        context: &WorkspaceContext,
        source_root: &Path,
        cancellation: &CancellationToken,
    ) -> IndexStartReport {
        #[cfg(any(test, feature = "test-support"))]
        observe_isolated_build_start(source_root);
        if cancellation.is_cancelled() {
            return unavailable_start_report(cancelled_error("rlm index build stopped"));
        }
        if let Err(error) = self.validate_bound_source_root(source_root) {
            return unavailable_start_report(error);
        }
        let state_context = match self.state_context(context, source_root) {
            Ok(context) => context,
            Err(error) => return unavailable_start_report(error),
        };
        if let Some(maintenance) =
            observed_maintenance(&match lock_path(&state_context, source_root) {
                Ok(path) => path,
                Err(error) => return unavailable_start_report(error),
            })
        {
            return IndexStartReport::observed(maintenance);
        }
        let build_id = uuid::Uuid::new_v4().simple().to_string();
        let build_dir = match rlm_build_root(&state_context, source_root, &build_id) {
            Ok(directory) => directory,
            Err(error) => return unavailable_start_report(error),
        };
        let mut commands = match self.commands(&state_context, source_root, cancellation) {
            Ok(commands) => commands,
            Err(error) => return unavailable_start_report(error),
        };
        for command in [&mut commands.info, &mut commands.build] {
            let Some((_, directory)) = command
                .env
                .iter_mut()
                .find(|(name, _)| name == "RLM_INDEX_DIR")
            else {
                return unavailable_start_report(
                    "RLM index directory was not configured".to_string(),
                );
            };
            *directory = build_dir.as_os_str().to_os_string();
        }
        self.start_background(
            &state_context,
            IndexStartSpec {
                action: "build",
                source_root: source_root.to_path_buf(),
                primary: commands.build,
                info: commands.info,
            },
        )
    }

    pub fn new(environment: &'a dyn RlmEnvironment) -> Self {
        Self {
            runner: &SYSTEM_INDEX_RUNNER,
            source_lease: None,
            state_scope: WorkspaceStateScope::LegacyPhysical,
            environment,
        }
    }

    pub fn with_runner(runner: &'a dyn IndexRunner, environment: &'a dyn RlmEnvironment) -> Self {
        Self {
            runner,
            source_lease: None,
            state_scope: WorkspaceStateScope::LegacyPhysical,
            environment,
        }
    }

    pub fn with_source_lease(mut self, lease: ProviderSourceLease) -> Self {
        self.state_scope = lease.state_scope().clone();
        self.source_lease = Some(lease);
        self
    }

    fn commands(
        &self,
        context: &WorkspaceContext,
        source_root: &Path,
        cancellation: &CancellationToken,
    ) -> Result<IndexCommands, String> {
        let plugin_root = self
            .environment
            .find_plugin_root(&context.cwd)
            .ok_or_else(|| {
                "could not locate Unica plugin root for internal RLM index adapter lookup"
                    .to_string()
            })?;
        let program = self
            .environment
            .resolve_bundled_tool(&plugin_root, "rlm-bsl-index", true)?
            .program;
        let env = rlm_process_environment(rlm_generation_root(context, source_root)?);
        let root = source_root.as_os_str().to_os_string();
        Ok(IndexCommands {
            info: IndexCommand {
                program: program.clone(),
                args: vec!["index".into(), "info".into(), root.clone()],
                cwd: context.cwd.clone(),
                env: env.clone(),
                timeout: INDEX_TIMEOUT,
                cancellation: cancellation.clone(),
            },
            build: IndexCommand {
                program,
                args: vec!["index".into(), "build".into(), root],
                cwd: context.cwd.clone(),
                env,
                timeout: Duration::from_secs(24 * 60 * 60),
                cancellation: cancellation.clone(),
            },
        })
    }

    fn validate_bound_source_root(&self, source_root: &Path) -> Result<(), String> {
        if let Some(bound) = &self.source_lease {
            if bound.source_root() != source_root {
                return Err(format!(
                    "workspace index request escaped its actor-bound source root: {}",
                    source_root.display()
                ));
            }
            bound.validate_named_identity().map_err(|error| {
                format!(
                    "workspace index actor-bound source root changed after admission: {}: {error}",
                    source_root.display()
                )
            })?;
        }
        Ok(())
    }

    fn state_context(
        &self,
        context: &WorkspaceContext,
        source_root: &Path,
    ) -> Result<WorkspaceContext, String> {
        let Some(scope) = self.state_scope.scoped_digest() else {
            return Ok(context.clone());
        };
        let legacy_pair_root = rlm_provider_state_root(context, source_root)?;
        let cache_root = normalize_path_identity(
            &legacy_pair_root
                .join("actor-scopes")
                .join(scope)
                .join("cache"),
        )?;
        if path_starts_with_host_root(&cache_root, source_root) {
            return Err("actor-scoped RLM state resolved inside sourceRoot".to_string());
        }
        let mut scoped = context.clone();
        scoped.cache_root = cache_root;
        Ok(scoped)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn provider_state_root_for_test(
        &self,
        context: &WorkspaceContext,
        source_root: &Path,
    ) -> Result<PathBuf, String> {
        let scoped = self.state_context(context, source_root)?;
        rlm_provider_state_root(&scoped, source_root)
    }

    fn start_background(
        &self,
        context: &WorkspaceContext,
        spec: IndexStartSpec,
    ) -> IndexStartReport {
        let IndexStartSpec {
            action,
            source_root,
            primary,
            info,
        } = spec;
        let isolated_build = isolated_build_id(&primary).is_some();
        let lock = match lock_path(context, &source_root) {
            Ok(lock) => lock,
            Err(error) => return unavailable_start_report(error),
        };
        if let Err(error) = self.validate_bound_source_root(&source_root) {
            return unavailable_start_report(error);
        }
        if let Some(parent) = lock.parent() {
            if let Err(error) = fs::create_dir_all(parent) {
                let message = format!("failed to create RLM index lock directory: {error}");
                if !isolated_build {
                    let _ = write_status(
                        context,
                        &source_root,
                        BslIndexStatus::failed(message.as_str(), Some(&source_root)),
                    );
                }
                return unavailable_start_report(message);
            }
        }

        let lock_lease = match acquire_index_lock(&lock, action, &source_root) {
            Ok(Some(lock_lease)) => lock_lease,
            Ok(None) => {
                return IndexStartReport::observed(observed_maintenance(&lock).unwrap_or_default());
            }
            Err(error) => {
                if !isolated_build {
                    let _ = write_status(
                        context,
                        &source_root,
                        BslIndexStatus::failed(error.as_str(), Some(&source_root)),
                    );
                }
                return unavailable_start_report(error);
            }
        };
        let status_path = match status_path(context, &source_root) {
            Ok(status_path) => status_path,
            Err(error) => return unavailable_start_report(error),
        };
        if !isolated_build {
            let _ = write_status_path(
                &status_path,
                BslIndexStatus::building(action, Some(&source_root)),
            );
        }

        let observation = Arc::clone(&lock_lease.observation);
        let error_source_root = source_root.clone();
        let job = IndexBackgroundJob {
            action: action.to_string(),
            context: context.clone(),
            source_root,
            source_lease: self.source_lease.clone(),
            primary,
            info,
            status_path,
            #[cfg(any(test, feature = "test-support"))]
            lock_path: lock.clone(),
            lock_lease,
        };
        let start_result = self.runner.start_background(job);
        if let Err(error) = self.validate_bound_source_root(&error_source_root) {
            return unavailable_start_report(error);
        }
        if let Err(error) = start_result {
            if !isolated_build {
                let _ = write_status(
                    context,
                    &error_source_root,
                    BslIndexStatus::failed(error.as_str(), Some(&error_source_root)),
                );
            }
            return unavailable_start_report(error);
        }

        {
            let mut state = observation
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if matches!(*state, MaintenanceObservation::Prepared) {
                *state = MaintenanceObservation::Observed(IndexMaintenanceStatus::Queued);
            }
        }
        IndexStartReport::observed(observed_status(&observation))
    }
}

#[derive(Debug, Clone)]
struct IndexCommands {
    info: IndexCommand,
    build: IndexCommand,
}

impl BslIndexStatus {
    fn ready(source_root: &Path, db_path: &Path) -> Self {
        Self {
            status: "ready".to_string(),
            source_root: Some(source_root.display().to_string()),
            db_path: Some(db_path.display().to_string()),
            message: None,
            failure_class: None,
            build_id: None,
            next_action: None,
            updated_at: now_secs(),
            last_run: None,
        }
    }

    fn building(action: &str, source_root: Option<&Path>) -> Self {
        Self {
            status: "building".to_string(),
            source_root: source_root.map(|path| path.display().to_string()),
            db_path: None,
            message: Some(format!("rlm index {action} started")),
            failure_class: None,
            build_id: None,
            next_action: None,
            updated_at: now_secs(),
            last_run: None,
        }
    }

    fn failed(message: &str, source_root: Option<&Path>) -> Self {
        Self {
            status: "failed".to_string(),
            source_root: source_root.map(|path| path.display().to_string()),
            db_path: None,
            message: Some(message.to_string()),
            failure_class: Some(BslIndexFailureClass::Retryable),
            build_id: None,
            next_action: None,
            updated_at: now_secs(),
            last_run: None,
        }
    }

    fn with_last_run(mut self, metrics: BslIndexRunMetrics) -> Self {
        self.last_run = Some(metrics);
        self
    }

    fn with_build_id(mut self, build_id: String) -> Self {
        self.build_id = Some(build_id);
        self
    }
}

impl BslIndexLock {
    fn new(action: &str, source_root: &Path) -> Self {
        let now = now_secs();
        Self {
            schema_version: LOCK_SCHEMA_VERSION,
            lock_id: new_lock_id(),
            owner_pid: std::process::id(),
            action: action.to_string(),
            source_root: source_root.display().to_string(),
            started_at: now,
            updated_at: now,
            state: "active".to_string(),
            child_pid: None,
            released_at: None,
            message: None,
        }
    }

    fn recovered(reason: &str, source_root: &Path) -> Self {
        let now = now_secs();
        Self {
            schema_version: LOCK_SCHEMA_VERSION,
            lock_id: new_lock_id(),
            owner_pid: std::process::id(),
            action: "recover".to_string(),
            source_root: source_root.display().to_string(),
            started_at: now,
            updated_at: now,
            state: "recovered".to_string(),
            child_pid: None,
            released_at: Some(now),
            message: Some(reason.to_string()),
        }
    }

    fn is_active(&self) -> bool {
        self.schema_version == LOCK_SCHEMA_VERSION && self.state == "active"
    }

    fn is_fresh(&self) -> bool {
        self.is_active() && now_secs().saturating_sub(self.updated_at) <= LOCK_STALE_AFTER.as_secs()
    }

    fn mark_released(&mut self) {
        let now = now_secs();
        self.state = "released".to_string();
        self.updated_at = now;
        self.released_at = Some(now);
    }

    fn mark_recovered(&mut self, reason: &str) {
        let now = now_secs();
        self.state = "recovered".to_string();
        self.updated_at = now;
        self.released_at = Some(now);
        self.message = Some(reason.to_string());
    }
}

fn default_lock_state() -> String {
    "active".to_string()
}

#[derive(Debug)]
pub struct IndexLockLease {
    path: PathBuf,
    lock_id: String,
    owner: Arc<Mutex<OwnedIndexLock>>,
    observation: MaintenanceObserver,
}

#[derive(Debug)]
struct OwnedIndexLock {
    file: File,
    lock: BslIndexLock,
    released: bool,
}

impl OwnedIndexLock {
    fn named_identity_matches(&self, path: &Path) -> std::io::Result<bool> {
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.is_file() || metadata_is_link_or_reparse_point(&metadata) {
            return Ok(false);
        }
        let named = open_state_reader(path)?;
        Ok(named.identity()? == state_file_identity(&self.file)?)
    }

    fn current_record(&mut self, path: &Path, lock_id: &str) -> std::io::Result<BslIndexLock> {
        if self.released || !self.named_identity_matches(path)? {
            return Err(std::io::Error::other(
                "index lock owner no longer names its file",
            ));
        }
        // Windows mandatory locking rejects reads through a new File. Use the
        // actual lock owner, serialized with heartbeat/release writes. Cloned
        // handles and Windows seek_read share or alter the owner's cursor.
        let current = read_lock_from_open(&mut self.file)?;
        if current.lock_id != lock_id
            || !current.is_active()
            || !self.named_identity_matches(path)?
        {
            return Err(std::io::Error::other("index lock owner record changed"));
        }
        Ok(current)
    }

    fn write_record(&mut self) -> Result<(), String> {
        write_lock_file_to_open(&mut self.file, &self.lock)
    }
}

impl IndexLockLease {
    fn lock_id(&self) -> &str {
        &self.lock_id
    }

    fn registered_ownership_is_current(&self) -> bool {
        active_index_locks()
            .lock()
            .ok()
            .and_then(|locks| locks.get(&self.path).cloned())
            .is_some_and(|entry| {
                entry.lock_id == self.lock_id && entry.owner.ptr_eq(&Arc::downgrade(&self.owner))
            })
    }

    fn refresh(&mut self, child_pid: u32) -> bool {
        if !self.registered_ownership_is_current() {
            return false;
        }
        let Ok(mut owner) = self.owner.lock() else {
            return false;
        };
        let Ok(current) = owner.current_record(&self.path, &self.lock_id) else {
            return false;
        };
        owner.lock = current;
        owner.lock.updated_at = now_secs();
        owner.lock.child_pid = Some(child_pid);
        owner.write_record().is_ok() && owner.named_identity_matches(&self.path).unwrap_or(false)
    }

    fn release(&mut self) {
        let mut owner = match self.owner.lock() {
            Ok(owner) => owner,
            Err(poisoned) => {
                // Cleanup may unlock a poisoned owner, but must not write a
                // record whose authority could not be validated.
                unregister_active_lock(&self.path, self.lock_id());
                let mut owner = poisoned.into_inner();
                owner.released = true;
                let _ = owner.file.unlock();
                return;
            }
        };
        if owner.released {
            return;
        }
        {
            let mut observation = self
                .observation
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if matches!(
                *observation,
                MaintenanceObservation::Prepared
                    | MaintenanceObservation::Observed(
                        IndexMaintenanceStatus::Queued | IndexMaintenanceStatus::Running
                    )
            ) {
                *observation = MaintenanceObservation::Observed(IndexMaintenanceStatus::Unknown);
            }
        }
        let current = self
            .registered_ownership_is_current()
            .then(|| owner.current_record(&self.path, &self.lock_id).ok())
            .flatten();
        unregister_active_lock(&self.path, self.lock_id());
        if let Some(current) = current {
            owner.lock = current;
            owner.lock.mark_released();
            let _ = owner.write_record();
        }
        owner.released = true;
        let _ = owner.file.unlock();
    }

    fn validate_ownership(&self) -> bool {
        let registered = self.registered_ownership_is_current();
        if !registered {
            return false;
        }
        self.owner
            .lock()
            .ok()
            .is_some_and(|mut owner| owner.current_record(&self.path, &self.lock_id).is_ok())
    }
}

impl Drop for IndexLockLease {
    fn drop(&mut self) {
        self.release();
    }
}

#[derive(Clone)]
struct RegisteredIndexLock {
    lock_id: String,
    owner: Weak<Mutex<OwnedIndexLock>>,
    observation: MaintenanceObserver,
}

fn active_index_locks() -> &'static Mutex<HashMap<PathBuf, RegisteredIndexLock>> {
    static ACTIVE_INDEX_LOCKS: OnceLock<Mutex<HashMap<PathBuf, RegisteredIndexLock>>> =
        OnceLock::new();
    ACTIVE_INDEX_LOCKS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn register_active_lock(
    path: &Path,
    lock_id: &str,
    owner: &Arc<Mutex<OwnedIndexLock>>,
) -> MaintenanceObserver {
    let observation = Arc::new(Mutex::new(MaintenanceObservation::Prepared));
    if let Ok(mut locks) = active_index_locks().lock() {
        locks.insert(
            path.to_path_buf(),
            RegisteredIndexLock {
                lock_id: lock_id.to_string(),
                owner: Arc::downgrade(owner),
                observation: Arc::clone(&observation),
            },
        );
    }
    observation
}

fn observed_maintenance(path: &Path) -> Option<IndexMaintenanceStatus> {
    // Never hold the global registry mutex while waiting for an owner. Release
    // takes the owner first and unregisters before dropping its kernel lock.
    let entry = { active_index_locks().lock().ok()?.get(path).cloned()? };
    let owner = entry.owner.upgrade()?;
    let mut owner = owner.lock().ok()?;
    owner.current_record(path, &entry.lock_id).ok()?;
    let observation = entry.observation.lock().ok()?;
    Some(match &*observation {
        MaintenanceObservation::Prepared => IndexMaintenanceStatus::Unknown,
        MaintenanceObservation::Observed(status) => status.clone(),
    })
}

fn unregister_active_lock(path: &Path, lock_id: &str) {
    if let Ok(mut locks) = active_index_locks().lock() {
        if locks
            .get(path)
            .map(|current| current.lock_id == lock_id)
            .unwrap_or(false)
        {
            locks.remove(path);
        }
    }
}

fn active_lock_registered(path: &Path) -> bool {
    active_index_locks()
        .lock()
        .ok()
        .and_then(|locks| locks.get(path).cloned())
        .is_some()
}

impl BslIndexRunMetrics {
    fn from_output(action: &str, started_at: u64, finished_at: u64, output: &IndexOutput) -> Self {
        Self {
            action: action.to_string(),
            recovery_reason: None,
            duration_ms: output.duration_ms,
            started_at,
            finished_at,
            timed_out: output.timed_out,
            index_version: parse_info_value(&output.stdout, "Index")
                .filter(|value| value.starts_with('v')),
            modules: parse_u64_info_value(&output.stdout, "Modules"),
            methods: parse_u64_info_value(&output.stdout, "Methods"),
            db_size: parse_info_value(&output.stdout, "DB size"),
        }
    }
}

impl IndexRunner for SystemIndexRunner {
    fn start_background(&self, job: IndexBackgroundJob) -> Result<(), String> {
        let handle = thread::Builder::new()
            .name("unica-rlm-index".to_string())
            .spawn(move || run_background_job(job))
            .map_err(|error| format!("failed to start RLM index background worker: {error}"))?;
        if let Some(tracker) = &self.tracker {
            tracker.track(handle);
        }
        Ok(())
    }
}

fn run_background_job(job: IndexBackgroundJob) {
    run_background_job_with(job, |command, lease| {
        run_index_command_with_heartbeat(command, Some(lease))
    });
}

fn run_background_job_with<F>(job: IndexBackgroundJob, mut run: F)
where
    F: FnMut(&IndexCommand, &mut IndexLockLease) -> Result<IndexOutput, String>,
{
    let mut job = job;
    if !job.lock_lease.validate_ownership() || !source_lease_is_current(job.source_lease.as_ref()) {
        return;
    }
    *job.lock_lease
        .observation
        .lock()
        .unwrap_or_else(|error| error.into_inner()) =
        MaintenanceObservation::Observed(IndexMaintenanceStatus::Running);
    let started_at = now_secs();
    let Some(primary) = run_background_command(
        &job.primary,
        &mut job.lock_lease,
        job.source_lease.as_ref(),
        &mut run,
    ) else {
        return;
    };
    let primary = match primary {
        Ok(output) => output,
        Err(error) => {
            write_background_status(
                &job,
                BslIndexStatus::failed(error.as_str(), Some(&job.source_root)),
            );
            return;
        }
    };
    let primary_finished_at = now_secs();
    let primary_metrics =
        BslIndexRunMetrics::from_output(&job.action, started_at, primary_finished_at, &primary);
    if !primary.status_success || primary.cancelled || primary.timed_out {
        let message = command_failure_message(&job.action, &primary);
        write_background_status(
            &job,
            BslIndexStatus::failed(message.as_str(), Some(&job.source_root))
                .with_last_run(primary_metrics),
        );
        return;
    }

    let Some(post_primary) = run_background_command(
        &job.info,
        &mut job.lock_lease,
        job.source_lease.as_ref(),
        &mut run,
    ) else {
        return;
    };
    let post_primary = match post_primary {
        Ok(info) => readiness_from_info(&info),
        Err(error) => {
            write_background_status(
                &job,
                BslIndexStatus::failed(error.as_str(), Some(&job.source_root))
                    .with_last_run(primary_metrics),
            );
            return;
        }
    };
    match post_primary {
        IndexReadiness::Ready { db_path } => {
            if db_path_belongs_to_command_generation(&job.info, &db_path)
                && candidate_db_for_job_is_usable(&job, &db_path)
            {
                write_background_status(
                    &job,
                    ready_status_after_build(&job, &db_path, primary_metrics),
                );
            } else {
                write_background_status(
                    &job,
                    BslIndexStatus::failed(
                        "rlm index info reported a DB outside the active generation",
                        Some(&job.source_root),
                    )
                    .with_last_run(primary_metrics),
                );
            }
        }
        other => {
            write_background_status(
                &job,
                failed_status_from_readiness(
                    &other,
                    &job.source_root,
                    format!("rlm index {} finished but info is", job.action).as_str(),
                )
                .with_last_run(primary_metrics),
            );
        }
    }
}
fn ready_status_after_build(
    job: &IndexBackgroundJob,
    db_path: &Path,
    metrics: BslIndexRunMetrics,
) -> BslIndexStatus {
    let Some(build_id) = isolated_build_id(&job.info) else {
        return BslIndexStatus::failed(
            "RLM background build has no isolated build identifier",
            Some(&job.source_root),
        )
        .with_last_run(metrics);
    };
    BslIndexStatus::ready(&job.source_root, db_path)
        .with_build_id(build_id)
        .with_last_run(metrics)
}

fn failed_status_from_readiness(
    readiness: &IndexReadiness,
    source_root: &Path,
    context: &str,
) -> BslIndexStatus {
    let detail = match readiness {
        IndexReadiness::Missing => "missing".to_string(),
        IndexReadiness::Stale { status } => status.clone(),
        IndexReadiness::Building => "building".to_string(),
        IndexReadiness::Queued => "queued".to_string(),
        IndexReadiness::Incomplete => "incomplete".to_string(),
        IndexReadiness::MaintenanceUnavailable(evidence) => evidence.summary().to_string(),
        IndexReadiness::Failed(error) | IndexReadiness::Unavailable(error) => error.clone(),
        IndexReadiness::Ready { .. } => "fresh".to_string(),
    };
    let message = recovery_failure_message(context, &detail);
    BslIndexStatus::failed(message.as_str(), Some(source_root))
}

fn recovery_failure_message(context: &str, detail: &str) -> String {
    if detail.starts_with(CANCELLED_PREFIX) {
        format!("{detail}; {context}")
    } else {
        format!("{context}: {detail}")
    }
}

fn run_background_command<F>(
    command: &IndexCommand,
    lease: &mut IndexLockLease,
    source_lease: Option<&ProviderSourceLease>,
    run: &mut F,
) -> Option<Result<IndexOutput, String>>
where
    F: FnMut(&IndexCommand, &mut IndexLockLease) -> Result<IndexOutput, String>,
{
    if !lease.validate_ownership() || !source_lease_is_current(source_lease) {
        return None;
    }
    let result = run(command, lease);
    (lease.validate_ownership() && source_lease_is_current(source_lease)).then_some(result)
}

fn source_lease_is_current(capability: Option<&ProviderSourceLease>) -> bool {
    capability.is_none_or(|capability| capability.validate_named_identity().is_ok())
}

fn db_path_belongs_to_command_generation(command: &IndexCommand, db_path: &Path) -> bool {
    let Some((_, generation_root)) = command.env.iter().find(|(name, _)| name == "RLM_INDEX_DIR")
    else {
        return false;
    };
    normalized_path_is_within(db_path, Path::new(generation_root))
}

fn isolated_build_id(command: &IndexCommand) -> Option<String> {
    let (_, root) = command
        .env
        .iter()
        .find(|(name, _)| name == "RLM_INDEX_DIR")?;
    let root = Path::new(root);
    (root.parent()?.file_name()? == "builds")
        .then(|| root.file_name()?.to_str().map(ToString::to_string))
        .flatten()
        .filter(|id| id.len() == 32 && id.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn candidate_db_for_job_is_usable(job: &IndexBackgroundJob, db_path: &Path) -> bool {
    let Some(build_id) = isolated_build_id(&job.info) else {
        return false;
    };
    // Recompute the checked route at publication time: a candidate directory
    // may have been replaced by a symlink after the command was started.
    let Ok(directory) = rlm_build_root(&job.context, &job.source_root, &build_id) else {
        return false;
    };
    job.info
        .env
        .iter()
        .any(|(key, configured)| key == "RLM_INDEX_DIR" && Path::new(configured) == directory)
        && usable_db_in_generation(db_path, &directory)
}

fn write_background_status(job: &IndexBackgroundJob, status: BslIndexStatus) -> bool {
    if !job.lock_lease.validate_ownership() || !source_lease_is_current(job.source_lease.as_ref()) {
        *job.lock_lease
            .observation
            .lock()
            .unwrap_or_else(|error| error.into_inner()) =
            MaintenanceObservation::Observed(IndexMaintenanceStatus::Unknown);
        return false;
    }
    let published = if isolated_build_id(&job.info).is_none() || status.build_id.is_some() {
        write_status_path(&job.status_path, status.clone()).is_ok()
    } else {
        true
    };
    let observed = match status.status.as_str() {
        "ready" if published => IndexMaintenanceStatus::Completed,
        "ready" => {
            IndexMaintenanceStatus::Failed("could not publish the completed RLM index".to_string())
        }
        "failed" => {
            let reason = status
                .message
                .unwrap_or_else(|| "index worker failed".to_string());
            if reason.starts_with(CANCELLED_PREFIX) {
                IndexMaintenanceStatus::Cancelled(reason)
            } else {
                IndexMaintenanceStatus::Failed(reason)
            }
        }
        _ => IndexMaintenanceStatus::Unknown,
    };
    *job.lock_lease
        .observation
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = MaintenanceObservation::Observed(observed);
    published
}

fn command_failure_message(action: &str, output: &IndexOutput) -> String {
    if output.cancelled {
        cancelled_error(format!("rlm index {action} stopped"))
    } else if output.timed_out {
        format!("rlm index {action} timed out")
    } else {
        format!(
            "rlm index {action} failed: {} {}",
            output.status,
            output.stderr.trim()
        )
    }
}

#[cfg(test)]
fn run_index_command(command: &IndexCommand) -> Result<IndexOutput, String> {
    run_index_command_with_heartbeat(command, None)
}

fn run_index_command_with_heartbeat(
    command: &IndexCommand,
    mut heartbeat: Option<&mut IndexLockLease>,
) -> Result<IndexOutput, String> {
    if heartbeat
        .as_deref()
        .is_some_and(|lease| !lease.validate_ownership())
    {
        return Err("RLM index lock ownership lost before command start".to_string());
    }
    let started = Instant::now();
    let mut child = ManagedChild::spawn(ManagedCommand {
        program: command.program.clone(),
        args: command.args.clone(),
        cwd: command.cwd.clone(),
        env: command.env.clone(),
        env_remove: Vec::new(),
        capture_limits: None,
        timeout: Some(command.timeout),
        cancellation: command.cancellation.clone(),
    })
    .map_err(|error| format!("failed to execute RLM index process: {error}"))?;
    let child_pid = child.id();
    let mut last_heartbeat = Instant::now();
    let ownership_cancellation = command.cancellation.clone();
    let mut ownership_lost = false;
    if let Some(lease) = heartbeat.as_mut() {
        if !(*lease).refresh(child_pid) {
            ownership_lost = true;
            ownership_cancellation.cancel();
        }
    }
    let output = child
        .wait_for_output_with_poll(Duration::from_millis(50), || {
            if let Some(lease) = heartbeat.as_mut() {
                if !(*lease).registered_ownership_is_current() {
                    ownership_lost = true;
                    ownership_cancellation.cancel();
                    return;
                }
                if last_heartbeat.elapsed() >= LOCK_HEARTBEAT_INTERVAL {
                    if !(*lease).refresh(child_pid) {
                        ownership_lost = true;
                        ownership_cancellation.cancel();
                        return;
                    }
                    last_heartbeat = Instant::now();
                }
            }
        })
        .map_err(|error| format!("failed to collect RLM index output: {error}"))?;
    if ownership_lost && !output.cancelled {
        return Err("RLM index command stopped after lock ownership was lost".to_string());
    }
    Ok(map_managed_output(output, started.elapsed()))
}

fn map_managed_output(mut output: ManagedOutput, elapsed: Duration) -> IndexOutput {
    ensure_truncation_diagnostics(&mut output);
    IndexOutput {
        status_success: output.status_success && !output.cancelled && !output.timed_out,
        status: output.status,
        stdout: output.stdout,
        stderr: output.stderr,
        timed_out: output.timed_out,
        cancelled: output.cancelled,
        duration_ms: duration_ms(elapsed),
    }
}

fn readiness_from_info(output: &IndexOutput) -> IndexReadiness {
    if output.cancelled {
        return IndexReadiness::Unavailable(cancelled_error("rlm index info stopped"));
    }
    if !output.status_success {
        return IndexReadiness::Unavailable(output.stderr.trim().to_string());
    }
    if output.stdout.contains("Index not found") {
        return IndexReadiness::Missing;
    }
    let status = parse_info_value(&output.stdout, "Status");
    let db_path = parse_info_value(&output.stdout, "Index").map(PathBuf::from);
    match status.as_deref() {
        Some("fresh") => match db_path {
            Some(db_path) => IndexReadiness::Ready { db_path },
            None => {
                IndexReadiness::Unavailable("RLM index info did not report DB path".to_string())
            }
        },
        Some(value) if value.starts_with("stale") => IndexReadiness::Stale {
            status: value.to_string(),
        },
        Some("incomplete") => IndexReadiness::Incomplete,
        Some(value) => IndexReadiness::Unavailable(format!("RLM index status is {value}")),
        None => IndexReadiness::Unavailable("RLM index info did not report status".to_string()),
    }
}

fn parse_info_value(stdout: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}:");
    stdout.lines().find_map(|line| {
        let trimmed = line.trim();
        trimmed
            .strip_prefix(&prefix)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
    })
}

fn parse_u64_info_value(stdout: &str, key: &str) -> Option<u64> {
    let value = parse_info_value(stdout, key)?;
    let digits: String = value.chars().filter(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse().ok()
}

fn duration_ms(duration: Duration) -> u64 {
    duration.as_millis().try_into().unwrap_or(u64::MAX)
}

pub fn read_bsl_index_status(
    context: &WorkspaceContext,
    source_root: &Path,
) -> Result<Option<BslIndexStatus>, String> {
    let path = status_path(context, source_root)?;
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "failed to read RLM index status {}: {error}",
                path.display()
            ));
        }
    };
    serde_json::from_str(&text).map(Some).map_err(|error| {
        format!(
            "failed to parse RLM index status {}: {error}",
            path.display()
        )
    })
}

pub fn bsl_index_is_ready(context: &WorkspaceContext, environment: &dyn RlmEnvironment) -> bool {
    let Ok(source_root) = environment
        .resolve_source_root(context)
        .map(|resolved| resolved.path)
    else {
        return false;
    };
    if active_lock(context, &source_root).unwrap_or(true) {
        return false;
    }
    usable_index_build(context, &source_root).is_ok_and(|build| build.is_some())
}

fn unavailable_start_report(error: String) -> IndexStartReport {
    IndexStartReport {
        warnings: vec![format!("rlm index unavailable: {error}")],
        maintenance: IndexMaintenanceStatus::StartFailed(error),
    }
}

pub fn status_path(context: &WorkspaceContext, source_root: &Path) -> Result<PathBuf, String> {
    let pair_root = rlm_provider_state_root(context, source_root)?;
    checked_generation_route(
        &pair_root,
        pair_root
            .join("caches")
            .join(RLM_PRODUCT_DIR)
            .join(RLM_INDEX_GENERATION)
            .join(STATUS_FILE_NAME),
    )
}

fn lock_path(context: &WorkspaceContext, source_root: &Path) -> Result<PathBuf, String> {
    let pair_root = rlm_provider_state_root(context, source_root)?;
    checked_generation_route(
        &pair_root,
        pair_root
            .join("locks")
            .join(RLM_PRODUCT_DIR)
            .join(RLM_INDEX_GENERATION)
            .join(LOCK_FILE_NAME),
    )
}

fn active_lock(context: &WorkspaceContext, source_root: &Path) -> Result<bool, String> {
    let lock = lock_path(context, source_root)?;
    if !lock.is_file() {
        return Ok(false);
    }
    if active_lock_registered(&lock) {
        return Ok(true);
    }
    match read_lock_path(&lock) {
        Ok(index_lock) if !index_lock.is_active() => Ok(false),
        Ok(index_lock) if index_lock.is_fresh() => Ok(true),
        Ok(index_lock) => {
            if lock_is_held_by_other_process(&lock) {
                return Ok(true);
            }
            Ok(!recover_stale_lock(
                context,
                source_root,
                format!(
                    "RLM index {action} lock is stale",
                    action = index_lock.action
                )
                .as_str(),
                Some(index_lock.lock_id.as_str()),
            )?)
        }
        Err(error) => {
            if invalid_lock_may_be_active(context, source_root, &lock)? {
                return Ok(true);
            }
            Ok(!recover_stale_lock(
                context,
                source_root,
                format!("RLM index lock is invalid: {error}").as_str(),
                None,
            )?)
        }
    }
}

fn invalid_lock_may_be_active(
    context: &WorkspaceContext,
    source_root: &Path,
    lock: &Path,
) -> Result<bool, String> {
    if active_lock_registered(lock) || lock_is_held_by_other_process(lock) {
        return Ok(true);
    }
    let lock_updated_at = file_modified_secs(lock).unwrap_or_else(now_secs);
    if now_secs().saturating_sub(lock_updated_at) <= LOCK_STALE_AFTER.as_secs() {
        return Ok(true);
    }
    if let Some(status) = read_bsl_index_status(context, source_root)? {
        if status.status == "building" {
            return Ok(now_secs().saturating_sub(status.updated_at) <= LOCK_STALE_AFTER.as_secs());
        }
    }
    Ok(false)
}

fn recover_stale_lock(
    context: &WorkspaceContext,
    source_root: &Path,
    reason: &str,
    lock_id: Option<&str>,
) -> Result<bool, String> {
    let lock = lock_path(context, source_root)?;
    if !mark_lock_recovered(&lock, lock_id, source_root, reason) {
        return Ok(false);
    }
    if read_bsl_index_status(context, source_root)?
        .is_some_and(|status| status.status == "building")
    {
        let _ = write_status(
            context,
            source_root,
            BslIndexStatus::failed(
                format!("stale RLM index build marker recovered: {reason}").as_str(),
                None,
            ),
        );
    }
    Ok(true)
}

fn read_lock_path(path: &Path) -> Result<BslIndexLock, String> {
    let text = fs::read_to_string(path).map_err(|error| error.to_string())?;
    serde_json::from_str(&text).map_err(|error| error.to_string())
}

fn read_lock_from_open(file: &mut File) -> std::io::Result<BslIndexLock> {
    let position = file.stream_position()?;
    let mut text = String::new();
    let read = file
        .seek(SeekFrom::Start(0))
        .and_then(|_| file.read_to_string(&mut text));
    let restored = file.seek(SeekFrom::Start(position));
    read?;
    restored?;
    serde_json::from_str(&text).map_err(|error| std::io::Error::new(ErrorKind::InvalidData, error))
}

fn acquire_index_lock(
    path: &Path,
    action: &str,
    source_root: &Path,
) -> Result<Option<IndexLockLease>, String> {
    if active_lock_registered(path) {
        return Ok(None);
    }
    let mut file = open_state_lock(path, true)
        .map_err(|error| format!("failed to open RLM index lock: {error}"))?;
    match file.try_lock_exclusive() {
        Ok(()) => {}
        Err(error) if is_file_lock_contended(&error) => return Ok(None),
        Err(error) => return Err(format!("failed to lock RLM index lock: {error}")),
    }
    if active_lock_registered(path) {
        let _ = file.unlock();
        return Ok(None);
    }
    let index_lock = BslIndexLock::new(action, source_root);
    write_lock_file_to_open(&mut file, &index_lock)?;
    let lock_id = index_lock.lock_id.clone();
    let owner = Arc::new(Mutex::new(OwnedIndexLock {
        file,
        lock: index_lock,
        released: false,
    }));
    let observation = register_active_lock(path, &lock_id, &owner);
    Ok(Some(IndexLockLease {
        path: path.to_path_buf(),
        lock_id,
        owner,
        observation,
    }))
}

#[cfg(test)]
fn write_lock_path(path: &Path, index_lock: BslIndexLock) -> Result<(), String> {
    let temp_path = lock_temp_path(path);
    {
        let mut temp = create_new_state_stage(&temp_path)
            .map_err(|error| format!("failed to create temporary RLM index lock: {error}"))?;
        write_lock_file(&mut temp, &index_lock)?;
    }
    fs::rename(&temp_path, path).map_err(|error| {
        let _ = fs::remove_file(&temp_path);
        format!("failed to replace RLM index lock atomically: {error}")
    })
}

fn write_lock_file(file: &mut File, index_lock: &BslIndexLock) -> Result<(), String> {
    let text = serde_json::to_string_pretty(&index_lock).map_err(|error| error.to_string())?;
    file.write_all(text.as_bytes())
        .and_then(|_| file.write_all(b"\n"))
        .and_then(|_| file.flush())
        .map_err(|error| format!("failed to write RLM index lock: {error}"))
}

fn write_lock_file_to_open(file: &mut File, index_lock: &BslIndexLock) -> Result<(), String> {
    file.set_len(0)
        .and_then(|_| file.seek(SeekFrom::Start(0)).map(|_| ()))
        .map_err(|error| format!("failed to prepare RLM index lock for write: {error}"))?;
    write_lock_file(file, index_lock)
}

#[cfg(test)]
fn lock_temp_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("bsl_index.lock");
    path.with_file_name(format!(
        ".{file_name}.{}.{}.tmp",
        std::process::id(),
        now_nanos()
    ))
}

fn mark_lock_recovered(
    path: &Path,
    expected_lock_id: Option<&str>,
    source_root: &Path,
    reason: &str,
) -> bool {
    let Ok(mut file) = open_state_lock(path, expected_lock_id.is_none()) else {
        return false;
    };
    match file.try_lock_exclusive() {
        Ok(()) => {}
        Err(error) if is_file_lock_contended(&error) => return false,
        Err(_) => return false,
    }

    let recovered = match read_lock_from_open(&mut file) {
        Ok(mut current) => {
            if expected_lock_id
                .map(|lock_id| current.lock_id != lock_id)
                .unwrap_or(false)
            {
                let _ = file.unlock();
                return false;
            }
            current.mark_recovered(reason);
            current
        }
        Err(_) if expected_lock_id.is_some() => {
            let _ = file.unlock();
            return false;
        }
        Err(_) => BslIndexLock::recovered(reason, source_root),
    };
    let result = write_lock_file_to_open(&mut file, &recovered).is_ok();
    let _ = file.unlock();
    result
}

fn lock_is_held_by_other_process(path: &Path) -> bool {
    let Ok(file) = open_state_lock(path, false) else {
        return false;
    };
    match file.try_lock_exclusive() {
        Ok(()) => {
            let _ = file.unlock();
            false
        }
        Err(error) if is_file_lock_contended(&error) => true,
        Err(_) => true,
    }
}

fn file_modified_secs(path: &Path) -> Option<u64> {
    path.metadata()
        .ok()?
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs())
}

fn write_status(
    context: &WorkspaceContext,
    source_root: &Path,
    status: BslIndexStatus,
) -> Result<(), String> {
    write_status_path(&status_path(context, source_root)?, status)
}

fn normalized_path_is_within(path: &Path, root: &Path) -> bool {
    match (normalize_path_identity(path), normalize_path_identity(root)) {
        (Ok(path), Ok(root)) => path_starts_with_host_root(&path, &root),
        _ => false,
    }
}

fn stored_path_matches(stored: Option<&str>, current: &Path) -> bool {
    let Some(stored) = stored else {
        return false;
    };
    match (
        normalize_path_identity(Path::new(stored)),
        normalize_path_identity(current),
    ) {
        (Ok(stored), Ok(current)) => stored == current,
        _ => false,
    }
}

fn write_status_path(path: &Path, status: BslIndexStatus) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create Unica cache status directory: {error}"))?;
    }
    let text = serde_json::to_string_pretty(&status).map_err(|error| error.to_string())?;
    let temporary = path.with_file_name(format!(
        ".{}.{}.tmp",
        STATUS_FILE_NAME,
        uuid::Uuid::new_v4()
    ));
    let write_result = (|| {
        let mut file = create_state_stage(&temporary)?;
        file.write_all((text + "\n").as_bytes())?;
        file.sync_data()?;
        replace_file_atomically(&temporary, path)?;
        if let Some(parent) = path.parent() {
            sync_parent_directory(parent)?;
        }
        Ok::<(), std::io::Error>(())
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    write_result.map_err(|error| format!("failed to write RLM index status: {error}"))
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

fn now_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default()
}

fn new_lock_id() -> String {
    format!("{}-{}", std::process::id(), now_nanos())
}

pub trait IndexMaintenanceStatusExt {
    fn summary(&self) -> String;
    fn evidence(&self) -> IndexMaintenanceEvidence;
    fn readiness(&self) -> IndexReadiness;
}
impl IndexMaintenanceStatusExt for IndexMaintenanceStatus {
    fn summary(&self) -> String {
        match self {
            Self::Unknown => "RLM index maintenance state is unknown".to_string(),
            Self::Queued => "RLM index maintenance is queued".to_string(),
            Self::Running => "RLM index maintenance is running".to_string(),
            Self::Completed => "RLM index maintenance completed".to_string(),
            Self::StartFailed(error) => format!("RLM index maintenance start failed: {error}"),
            Self::Failed(error) => format!("RLM index maintenance failed: {error}"),
            Self::Cancelled(error) => error.clone(),
        }
    }
    fn evidence(&self) -> IndexMaintenanceEvidence {
        match self {
            Self::Queued => IndexMaintenanceEvidence::Queued,
            Self::Running => IndexMaintenanceEvidence::Running,
            Self::StartFailed(_) => IndexMaintenanceEvidence::StartFailed,
            Self::Failed(_) => IndexMaintenanceEvidence::ExecutionFailed,
            Self::Unknown => IndexMaintenanceEvidence::Unknown,
            Self::Completed => IndexMaintenanceEvidence::Completed,
            Self::Cancelled(_) => IndexMaintenanceEvidence::Cancelled,
        }
    }
    fn readiness(&self) -> IndexReadiness {
        match self {
            Self::Queued => IndexReadiness::Queued,
            Self::Running => IndexReadiness::Building,
            _ => IndexReadiness::MaintenanceUnavailable(self.evidence()),
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
#[path = "../tests/platform/index.rs"]
mod tests;
#[cfg(any(test, feature = "test-support"))]
pub use tests::rlm_coordination_contract;
