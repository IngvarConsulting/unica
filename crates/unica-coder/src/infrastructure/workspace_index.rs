use crate::domain::cancellation::{cancelled_error, CancellationToken, CANCELLED_PREFIX};
use crate::domain::code_intelligence::IndexMaintenanceEvidence;
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::bundled_tools::resolve_bundled_tool;
use crate::infrastructure::platform::filesystem::{
    file_identity, metadata_is_link_or_reparse_point, path_starts_with_host_root,
    provider_state_path_identity, replace_file_atomically, sync_parent_directory,
    RetainedDirectoryCapability,
};
use crate::infrastructure::platform::{
    ensure_truncation_diagnostics, ManagedChild, ManagedCommand, ManagedOutput,
};
use crate::infrastructure::plugin_runtime::find_plugin_root;
use crate::infrastructure::rlm_generation_collection::spawn_stale_rlm_generation_collection;
use crate::infrastructure::source_roots::{normalize_path_identity, resolve_source_root};
use crate::infrastructure::workspace_state_scope::WorkspaceStateScope;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
#[cfg(test)]
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(test)]
type IsolatedBuildStartObserver = Box<dyn Fn(&Path)>;

#[cfg(test)]
thread_local! {
    static ISOLATED_BUILD_START_OBSERVER: RefCell<Option<IsolatedBuildStartObserver>> = RefCell::new(None);
}

#[cfg(test)]
pub(crate) fn set_isolated_build_start_observer_for_test(
    observer: Option<IsolatedBuildStartObserver>,
) {
    ISOLATED_BUILD_START_OBSERVER.with(|slot| {
        *slot.borrow_mut() = observer;
    });
}

#[cfg(test)]
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

pub(crate) fn rlm_provider_state_root(
    context: &WorkspaceContext,
    source_root: &Path,
) -> Result<PathBuf, String> {
    rlm_provider_state_root_with(context, source_root, neutral_provider_state_root())
}

pub(crate) fn rlm_process_environment(generation_root: PathBuf) -> Vec<(OsString, OsString)> {
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
    neutral_provider_state_root_with(|name| std::env::var_os(name))
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

pub(crate) fn rlm_generation_root(
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
pub(crate) fn usable_index_build(
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
    pub(crate) fn maintenance_evidence(&self) -> Option<IndexMaintenanceEvidence> {
        match self {
            Self::Queued => Some(IndexMaintenanceEvidence::Queued),
            Self::Building => Some(IndexMaintenanceEvidence::Running),
            Self::MaintenanceUnavailable(evidence) => Some(*evidence),
            _ => None,
        }
    }
}

/// Observed work, independent of the persisted index's readiness.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", content = "reason", rename_all = "snake_case")]
pub(crate) enum IndexMaintenanceStatus {
    #[default]
    Unknown,
    Queued,
    Running,
    Completed,
    StartFailed(String),
    Failed(String),
    Cancelled(String),
}

impl IndexMaintenanceStatus {
    pub(crate) fn summary(&self) -> String {
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

    pub(crate) fn evidence(&self) -> IndexMaintenanceEvidence {
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

    pub(crate) fn readiness(&self) -> IndexReadiness {
        match self {
            Self::Queued => IndexReadiness::Queued,
            Self::Running => IndexReadiness::Building,
            _ => IndexReadiness::MaintenanceUnavailable(self.evidence()),
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
    pub(crate) maintenance: IndexMaintenanceStatus,
}

impl IndexStartReport {
    pub(crate) fn observed(maintenance: IndexMaintenanceStatus) -> Self {
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
pub(crate) struct UsableIndexBuild {
    pub(crate) id: String,
    pub(crate) directory: PathBuf,
    pub(crate) db_path: PathBuf,
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
    pub(crate) root_capability: Option<Arc<RetainedDirectoryCapability>>,
    pub primary: IndexCommand,
    pub info: IndexCommand,
    pub status_path: PathBuf,
    #[cfg(test)]
    pub lock_path: PathBuf,
    pub lock_lease: IndexLockLease,
}

#[cfg(test)]
impl IndexBackgroundJob {
    pub(crate) fn run_with_for_test(
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

pub trait IndexBackgroundTaskTracker: Send + Sync {
    fn track(&self, handle: thread::JoinHandle<()>);
}

pub struct SystemIndexRunner {
    tracker: Option<Arc<dyn IndexBackgroundTaskTracker>>,
}

impl SystemIndexRunner {
    pub fn tracked(tracker: Arc<dyn IndexBackgroundTaskTracker>) -> Self {
        Self {
            tracker: Some(tracker),
        }
    }
}

pub static SYSTEM_INDEX_RUNNER: SystemIndexRunner = SystemIndexRunner { tracker: None };

pub struct WorkspaceIndexService<'a> {
    runner: &'a dyn IndexRunner,
    bound_source_root: Option<Arc<RetainedDirectoryCapability>>,
    state_scope: WorkspaceStateScope,
}

impl<'a> WorkspaceIndexService<'a> {
    pub(crate) fn active_build(
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

    pub(crate) fn maintenance_status(
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
    pub(crate) fn start_isolated_build(
        &self,
        context: &WorkspaceContext,
        source_root: &Path,
        cancellation: &CancellationToken,
    ) -> IndexStartReport {
        #[cfg(test)]
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

    pub fn new() -> Self {
        Self {
            runner: &SYSTEM_INDEX_RUNNER,
            bound_source_root: None,
            state_scope: WorkspaceStateScope::LegacyPhysical,
        }
    }

    pub(crate) fn with_runner(runner: &'a dyn IndexRunner) -> Self {
        Self {
            runner,
            bound_source_root: None,
            state_scope: WorkspaceStateScope::LegacyPhysical,
        }
    }

    pub(crate) fn with_bound_source_root(
        mut self,
        source_root: Arc<RetainedDirectoryCapability>,
    ) -> Self {
        self.bound_source_root = Some(source_root);
        self
    }

    pub(crate) fn with_state_scope(mut self, state_scope: WorkspaceStateScope) -> Self {
        self.state_scope = state_scope;
        self
    }

    fn commands(
        &self,
        context: &WorkspaceContext,
        source_root: &Path,
        cancellation: &CancellationToken,
    ) -> Result<IndexCommands, String> {
        let plugin_root = find_plugin_root(&context.cwd).ok_or_else(|| {
            "could not locate Unica plugin root for internal RLM index adapter lookup".to_string()
        })?;
        let program = resolve_bundled_tool(&plugin_root, "rlm-bsl-index", true)?.program;
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
        if let Some(bound) = &self.bound_source_root {
            if bound.path() != source_root {
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

    #[cfg(test)]
    pub(crate) fn provider_state_root_for_test(
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
            root_capability: self.bound_source_root.clone(),
            primary,
            info,
            status_path,
            #[cfg(test)]
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

impl Default for WorkspaceIndexService<'_> {
    fn default() -> Self {
        Self::new()
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
        let named = File::open(path)?;
        Ok(file_identity(&named)? == file_identity(&self.file)?)
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
        let position = self.file.stream_position()?;
        let mut text = String::new();
        let read = self
            .file
            .seek(SeekFrom::Start(0))
            .and_then(|_| self.file.read_to_string(&mut text));
        let restored = self.file.seek(SeekFrom::Start(position));
        read?;
        restored?;
        let current: BslIndexLock = serde_json::from_str(&text)
            .map_err(|error| std::io::Error::new(ErrorKind::InvalidData, error))?;
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
    if !job.lock_lease.validate_ownership()
        || !root_capability_is_current(job.root_capability.as_deref())
    {
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
        job.root_capability.as_deref(),
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
        job.root_capability.as_deref(),
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
    root_capability: Option<&RetainedDirectoryCapability>,
    run: &mut F,
) -> Option<Result<IndexOutput, String>>
where
    F: FnMut(&IndexCommand, &mut IndexLockLease) -> Result<IndexOutput, String>,
{
    if !lease.validate_ownership() || !root_capability_is_current(root_capability) {
        return None;
    }
    let result = run(command, lease);
    (lease.validate_ownership() && root_capability_is_current(root_capability)).then_some(result)
}

fn root_capability_is_current(capability: Option<&RetainedDirectoryCapability>) -> bool {
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
    if !job.lock_lease.validate_ownership()
        || !root_capability_is_current(job.root_capability.as_deref())
    {
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

pub fn bsl_index_is_ready(context: &WorkspaceContext) -> bool {
    let Ok(source_root) = resolve_source_root(context, None).map(|resolved| resolved.path) else {
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

pub(crate) fn status_path(
    context: &WorkspaceContext,
    source_root: &Path,
) -> Result<PathBuf, String> {
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

fn acquire_index_lock(
    path: &Path,
    action: &str,
    source_root: &Path,
) -> Result<Option<IndexLockLease>, String> {
    if active_lock_registered(path) {
        return Ok(None);
    }
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
        .map_err(|error| format!("failed to open RLM index lock: {error}"))?;
    match file.try_lock_exclusive() {
        Ok(()) => {}
        Err(error) if lock_error_is_contended(&error) => return Ok(None),
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
        let mut temp = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp_path)
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
    let Ok(mut file) = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
    else {
        return false;
    };
    match file.try_lock_exclusive() {
        Ok(()) => {}
        Err(error) if lock_error_is_contended(&error) => return false,
        Err(_) => return false,
    }

    let recovered = match read_lock_path(path) {
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
        Err(_) => BslIndexLock::recovered(reason, source_root),
    };
    let result = write_lock_file_to_open(&mut file, &recovered).is_ok();
    let _ = file.unlock();
    result
}

fn lock_is_held_by_other_process(path: &Path) -> bool {
    let Ok(file) = OpenOptions::new().read(true).write(true).open(path) else {
        return false;
    };
    match file.try_lock_exclusive() {
        Ok(()) => {
            let _ = file.unlock();
            false
        }
        Err(error) if lock_error_is_contended(&error) => true,
        Err(_) => true,
    }
}

fn lock_error_is_contended(error: &std::io::Error) -> bool {
    error.kind() == ErrorKind::WouldBlock
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
        let mut file = File::create(&temporary)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::cancellation::CancellationToken;
    use crate::infrastructure::platform::{filesystem::path_starts_with_host_root, testing};
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::ffi::OsString;
    use std::sync::Arc;

    fn environment(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let entries: BTreeMap<String, OsString> = pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), OsString::from(*value)))
            .collect();
        move |name: &str| entries.get(name).cloned()
    }

    fn index_command_env<'a>(command: &'a IndexCommand, name: &str) -> &'a std::ffi::OsStr {
        command
            .env
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_os_str())
            .unwrap_or_else(|| panic!("index command must carry {name}"))
    }

    fn default_source_root(context: &WorkspaceContext) -> PathBuf {
        context.workspace_root.join("src")
    }

    fn generation_db_path(context: &WorkspaceContext, relative: &str) -> PathBuf {
        rlm_generation_root(context, &default_source_root(context))
            .unwrap()
            .join(relative)
    }

    fn status_path(context: &WorkspaceContext) -> PathBuf {
        super::status_path(context, &default_source_root(context)).unwrap()
    }

    fn lock_path(context: &WorkspaceContext) -> PathBuf {
        super::lock_path(context, &default_source_root(context)).unwrap()
    }

    fn read_bsl_index_status(context: &WorkspaceContext) -> Option<BslIndexStatus> {
        super::read_bsl_index_status(context, &default_source_root(context)).unwrap()
    }

    fn write_status(context: &WorkspaceContext, status: BslIndexStatus) -> Result<(), String> {
        super::write_status(context, &default_source_root(context), status)
    }

    #[test]
    fn neutral_provider_state_root_valid_override_wins() {
        let root = neutral_provider_state_root_with(environment(&[
            ("UNICA_PROVIDER_STATE_DIR", "/state/unica"),
            ("HOME", "/home/user"),
        ]));

        assert_eq!(root, Some(PathBuf::from("/state/unica")));
    }

    #[test]
    fn neutral_provider_state_root_empty_override_falls_through_to_home() {
        let root = neutral_provider_state_root_with(environment(&[
            ("UNICA_PROVIDER_STATE_DIR", ""),
            ("HOME", "/home/user"),
        ]));

        assert_eq!(
            root,
            Some(
                PathBuf::from("/home/user")
                    .join(".unica")
                    .join("provider-state")
            )
        );
    }

    #[test]
    fn neutral_provider_state_root_empty_home_falls_through_to_userprofile() {
        let root = neutral_provider_state_root_with(environment(&[
            ("HOME", ""),
            ("USERPROFILE", "C:/Users/user"),
        ]));

        assert_eq!(
            root,
            Some(
                PathBuf::from("C:/Users/user")
                    .join(".unica")
                    .join("provider-state")
            )
        );
    }

    #[test]
    fn neutral_provider_state_root_all_empty_values_report_missing_root() {
        let mut context = test_context("empty-direct-runtime-environment");
        context.cache_root = context.workspace_root.join(".build/unica");
        let external_base = neutral_provider_state_root_with(environment(&[
            ("UNICA_PROVIDER_STATE_DIR", ""),
            ("HOME", ""),
            ("USERPROFILE", ""),
        ]));

        assert_eq!(external_base, None);
        let error = rlm_provider_state_root_with(&context, &context.workspace_root, external_base)
            .unwrap_err();
        assert_eq!(
            error,
            "UNICA_PROVIDER_STATE_DIR, HOME, or USERPROFILE is required for RLM state outside sourceRoot"
        );
        cleanup(&context);
    }

    #[test]
    fn safe_root_failure_never_falls_back_to_generation_markers_inside_sources() {
        let mut context = test_context("safe-root-no-source-fallback");
        context.cache_root = context.workspace_root.join(".build/unica");
        let source_root = context.workspace_root.clone();

        let error = rlm_provider_state_root_with(&context, &source_root, None).unwrap_err();

        assert!(error.contains("required for RLM state outside sourceRoot"));
        assert!(!source_root.join("rlm-bsl/index-v17/bsl_index.db").exists());
        assert!(!source_root
            .join("caches/rlm-bsl/index-v17/bsl_index_status.json")
            .exists());
        assert!(!source_root
            .join("locks/rlm-bsl/index-v17/bsl_index.lock")
            .exists());
        cleanup(&context);
    }

    #[test]
    fn safe_root_failure_is_unavailable_at_the_start_boundary() {
        let mut context = test_context("safe-root-boundaries");
        let source_root = default_source_root(&context);
        fs::create_dir_all(&source_root).unwrap();
        let first = context.workspace_root.join("provider-state-cycle-a");
        let second = context.workspace_root.join("provider-state-cycle-b");
        let Some(first_link) = testing::create_dir_symlink_for_test(&second, &first) else {
            cleanup(&context);
            return;
        };
        first_link.unwrap();
        testing::create_dir_symlink_for_test(&first, &second)
            .expect("the platform that created the first link must expose the second fixture")
            .unwrap();
        context.cache_root = first;
        let runner = RecordingIndexRunner::default();
        let service = WorkspaceIndexService::with_runner(&runner);

        let report =
            service.start_isolated_build(&context, &source_root, &CancellationToken::new());

        assert!(report.warnings.iter().any(|warning| {
            warning.starts_with("rlm index unavailable:")
                && warning.contains("failed to resolve existing path ancestor")
        }));
        assert!(runner.backgrounds.borrow().is_empty());
        assert!(!source_root.join("rlm-bsl/index-v17/bsl_index.db").exists());
        assert!(!source_root
            .join("caches/rlm-bsl/index-v17/bsl_index_status.json")
            .exists());
        assert!(!source_root
            .join("locks/rlm-bsl/index-v17/bsl_index.lock")
            .exists());
        testing::remove_dir_symlink_for_test(&second).unwrap();
        testing::remove_dir_symlink_for_test(&context.cache_root).unwrap();
        cleanup(&context);
    }

    #[test]
    fn rlm_provider_state_scopes_the_existing_safe_cache_layout() {
        let context = test_context("safe-provider-root");
        let source_root = context.workspace_root.join("src");
        fs::create_dir_all(&source_root).unwrap();

        let actual = rlm_provider_state_root_with(
            &context,
            &source_root,
            Some(context.workspace_root.parent().unwrap().join("host-data")),
        )
        .unwrap();

        let safe_parent =
            normalize_path_identity(&context.cache_root.join("provider-state")).unwrap();
        assert!(actual.starts_with(&safe_parent));
        assert_ne!(
            actual,
            normalize_path_identity(&context.cache_root).unwrap()
        );
        let identity = actual.file_name().unwrap().to_string_lossy();
        let digest = identity.strip_prefix("rlm-").unwrap();
        assert_eq!(digest.len(), 64);
        assert!(digest.bytes().all(|byte| byte.is_ascii_hexdigit()));
        cleanup(&context);
    }

    #[test]
    fn rlm_provider_state_moves_outside_a_workspace_wide_source_root() {
        let mut context = test_context("unsafe-provider-root");
        context.cache_root = context.workspace_root.join(".build/unica");
        let external = context.workspace_root.parent().unwrap().join("host-data");

        let first =
            rlm_provider_state_root_with(&context, &context.workspace_root, Some(external.clone()))
                .unwrap();
        let second =
            rlm_provider_state_root_with(&context, &context.workspace_root, Some(external))
                .unwrap();

        assert_eq!(first, second);
        assert!(!path_starts_with_host_root(&first, &context.workspace_root));
        let identity = first.file_name().unwrap().to_string_lossy();
        let digest = identity.strip_prefix("rlm-").unwrap();
        assert_eq!(digest.len(), 64);
        assert!(digest.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_eq!(
            first.join(RLM_PRODUCT_DIR).join(RLM_INDEX_GENERATION),
            first.join("rlm-bsl/index-v17")
        );
        cleanup(&context);
    }

    #[test]
    fn rlm_provider_state_separates_source_roots() {
        let context = test_context("separate-provider-roots");
        let external = context.workspace_root.parent().unwrap().join("host-data");
        let first_source = context.workspace_root.join("src/configuration");
        let second_source = context.workspace_root.join("src/extension");
        fs::create_dir_all(&first_source).unwrap();
        fs::create_dir_all(&second_source).unwrap();

        let first =
            rlm_provider_state_root_with(&context, &first_source, Some(external.clone())).unwrap();
        let second =
            rlm_provider_state_root_with(&context, &second_source, Some(external)).unwrap();

        assert_ne!(first, second);
        cleanup(&context);
    }

    #[test]
    fn rlm_provider_state_identity_preserves_unix_path_case() {
        // INV-CACHE-PROVIDER-STATE-OUTSIDE-SOURCE: distinct normalized pairs
        // must not share persistent provider state on case-sensitive Unix filesystems.
        let fixture = test_context("provider-state-case-identity");
        let parent = fixture.workspace_root.parent().unwrap();
        let external = parent.join("host-data");
        let Some((upper_workspace, lower_workspace)) =
            testing::case_distinct_provider_paths_for_test(parent)
        else {
            cleanup(&fixture);
            return;
        };
        let context_for = |workspace_root: PathBuf| WorkspaceContext {
            cwd: workspace_root.clone(),
            cache_root: workspace_root.join(".build/unica"),
            workspace_root,
            workspace_epoch: 1,
        };
        let upper = rlm_provider_state_root_with(
            &context_for(upper_workspace.clone()),
            &upper_workspace,
            Some(external.clone()),
        )
        .unwrap();
        let lower = rlm_provider_state_root_with(
            &context_for(lower_workspace.clone()),
            &lower_workspace,
            Some(external),
        )
        .unwrap();

        assert_ne!(upper, lower);
        cleanup(&fixture);
    }

    #[test]
    fn rlm_provider_state_identity_preserves_distinct_non_utf8_unix_paths() {
        // INV-CACHE-PROVIDER-STATE-OUTSIDE-SOURCE: distinct normalized pairs
        // must not share persistent provider state when their path text is not UTF-8.
        let fixture = test_context("provider-state-non-utf8-identity");
        let Some((first_workspace, second_workspace)) =
            testing::distinct_non_utf8_provider_paths_for_test(&fixture.workspace_root)
        else {
            cleanup(&fixture);
            return;
        };
        fs::create_dir_all(&first_workspace).unwrap();
        fs::create_dir_all(&second_workspace).unwrap();
        assert_ne!(
            fs::canonicalize(&first_workspace).unwrap(),
            fs::canonicalize(&second_workspace).unwrap()
        );
        let external = fixture.workspace_root.join("host-data");
        let context_for = |workspace_root: PathBuf| WorkspaceContext {
            cwd: workspace_root.clone(),
            cache_root: workspace_root.join(".build/unica"),
            workspace_root,
            workspace_epoch: 1,
        };
        let first = rlm_provider_state_root_with(
            &context_for(first_workspace.clone()),
            &first_workspace,
            Some(external.clone()),
        )
        .unwrap();
        let second = rlm_provider_state_root_with(
            &context_for(second_workspace.clone()),
            &second_workspace,
            Some(external),
        )
        .unwrap();

        assert_ne!(first, second);
        cleanup(&fixture);
    }

    #[test]
    fn rlm_coordination_paths_separate_source_roots_under_the_pair_root() {
        use crate::domain::project_sources::{SourceFormat, SourceProfile, SourceSetKind};
        use crate::infrastructure::workspace_actor::{
            WorkspaceActor, WorkspaceIdentity, WorkspaceSourceSetInput,
        };

        let context = test_context("separate-coordination-roots");
        let first_source = context.workspace_root.join("src/configuration");
        let second_source = context.workspace_root.join("src/extension");
        fs::create_dir_all(&first_source).unwrap();
        fs::create_dir_all(&second_source).unwrap();

        let first_root = rlm_provider_state_root(&context, &first_source).unwrap();
        let second_root = rlm_provider_state_root(&context, &second_source).unwrap();
        let first_status = super::status_path(&context, &first_source).unwrap();
        let second_status = super::status_path(&context, &second_source).unwrap();
        let first_lock = super::lock_path(&context, &first_source).unwrap();
        let second_lock = super::lock_path(&context, &second_source).unwrap();

        assert_eq!(
            first_status,
            first_root.join("caches/rlm-bsl/index-v17/bsl_index_status.json")
        );
        assert_eq!(
            first_lock,
            first_root.join("locks/rlm-bsl/index-v17/bsl_index.lock")
        );
        assert_eq!(
            second_status,
            second_root.join("caches/rlm-bsl/index-v17/bsl_index_status.json")
        );
        assert_eq!(
            second_lock,
            second_root.join("locks/rlm-bsl/index-v17/bsl_index.lock")
        );
        assert_ne!(first_status, second_status);
        assert_ne!(first_lock, second_lock);

        let runner = RecordingIndexRunner::default();
        for (index, (name, profile)) in [
            ("main", "program"),
            ("renamed", "program"),
            ("main", "program-and-service"),
        ]
        .into_iter()
        .enumerate()
        {
            let identity = WorkspaceIdentity::new(
                &context,
                [WorkspaceSourceSetInput::new(
                    name,
                    &first_source,
                    SourceSetKind::Configuration,
                    SourceFormat::PlatformXml,
                    SourceProfile::platform_xml_8_3_27_format_2_20(),
                )],
                profile,
            )
            .unwrap();
            let actor = WorkspaceActor::new(identity, context.clone()).unwrap();
            let binding = actor.bind_provider_root(name, &first_source).unwrap();
            let service = actor.index_service(&binding, &runner).unwrap();
            let admitted_source = binding.source_root().to_path_buf();
            let report =
                service.start_isolated_build(&context, &admitted_source, &CancellationToken::new());

            assert_eq!(
                runner.backgrounds.borrow().len(),
                index + 1,
                "another actor scope blocked index start for {name}/{profile}: {report:?}"
            );
        }

        let backgrounds = runner.backgrounds.borrow();
        for (index, job) in backgrounds.iter().enumerate() {
            assert!(job.lock_path.is_file());
            assert!(!job.status_path.exists());
            for previous in &backgrounds[..index] {
                assert_ne!(job.lock_path, previous.lock_path);
                assert_ne!(job.status_path, previous.status_path);
                assert_ne!(
                    index_command_env(&job.primary, "RLM_INDEX_DIR"),
                    index_command_env(&previous.primary, "RLM_INDEX_DIR")
                );
            }
        }
        drop(backgrounds);
        drop(runner);
        cleanup(&context);
    }

    #[test]
    fn worker_terminal_observation_tracks_real_failure_cancellation_and_publication() {
        for case in [
            "failure",
            "cancelled",
            "timeout",
            "completed",
            "publication-failed",
        ] {
            let context = test_context(&format!("maintenance-terminal-{case}"));
            let source_root = context.workspace_root.join("src");
            fs::create_dir_all(&source_root).unwrap();
            let runner = RecordingIndexRunner::default();
            let service = WorkspaceIndexService::with_runner(&runner);
            let report =
                service.start_isolated_build(&context, &source_root, &CancellationToken::new());
            assert_eq!(report.maintenance, IndexMaintenanceStatus::Queued);
            let job = runner.backgrounds.borrow_mut().pop().unwrap();
            let observation = Arc::clone(&job.lock_lease.observation);
            let status_path = job.status_path.clone();
            run_background_job_with(job, |command, _| {
                assert_eq!(
                    observed_status(&observation),
                    IndexMaintenanceStatus::Running
                );
                match case {
                    "failure" => Err("controlled execution failure".to_string()),
                    "cancelled" | "timeout" => {
                        let mut output = IndexOutput::success("");
                        output.cancelled = case == "cancelled";
                        output.timed_out = case == "timeout";
                        Ok(output)
                    }
                    _ if command.args.get(1).is_some_and(|arg| arg == "info") => {
                        let db = PathBuf::from(index_command_env(command, "RLM_INDEX_DIR"))
                            .join("bsl_index.db");
                        fs::create_dir_all(db.parent().unwrap()).unwrap();
                        fs::write(&db, b"usable candidate").unwrap();
                        if case == "publication-failed" {
                            fs::create_dir_all(&status_path).unwrap();
                        }
                        Ok(IndexOutput::success(format!(
                            "Index: {}\nStatus: fresh",
                            db.display()
                        )))
                    }
                    _ => Ok(IndexOutput::success("build complete")),
                }
            });
            let observed = observed_status(&observation);
            match case {
                "completed" => {
                    assert_eq!(observed, IndexMaintenanceStatus::Completed);
                    assert!(service
                        .active_build(&context, &source_root)
                        .unwrap()
                        .is_some());
                }
                "cancelled" => assert!(
                    matches!(observed, IndexMaintenanceStatus::Cancelled(_)),
                    "{observed:?}"
                ),
                _ => assert!(
                    matches!(observed, IndexMaintenanceStatus::Failed(_)),
                    "{case}: {observed:?}"
                ),
            }
            assert!(
                service.maintenance_status(&context, &source_root).is_none(),
                "a terminal job no longer advertises active work"
            );
            cleanup(&context);
        }
    }

    #[test]
    fn registered_owner_observes_maintenance_while_its_file_is_exclusively_locked() {
        let context = test_context("maintenance-own-locked-observation");
        let source_root = context.workspace_root.join("src");
        fs::create_dir_all(&source_root).unwrap();
        let path = super::lock_path(&context, &source_root).unwrap();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let lease = acquire_index_lock(&path, "build", &source_root)
            .unwrap()
            .expect("own the actual index lock");
        lease
            .owner
            .lock()
            .unwrap()
            .file
            .seek(SeekFrom::Start(7))
            .unwrap();

        for state in [
            IndexMaintenanceStatus::Queued,
            IndexMaintenanceStatus::Running,
        ] {
            *lease.observation.lock().unwrap() = MaintenanceObservation::Observed(state.clone());
            assert!(lease.validate_ownership());
            assert_eq!(
                observed_maintenance(&path),
                Some(state),
                "the registered owner must observe its state without reopening locked bytes"
            );
            assert_eq!(
                lease.owner.lock().unwrap().file.stream_position().unwrap(),
                7
            );
        }
        drop(lease);
        assert!(observed_maintenance(&path).is_none());
        cleanup(&context);
    }

    #[test]
    fn registered_owner_rejects_in_place_record_substitution_without_overwriting_it() {
        for case in ["foreign-id", "inactive", "wrong-schema", "invalid-json"] {
            let context = test_context(&format!("maintenance-in-place-{case}"));
            let source_root = context.workspace_root.join("src");
            fs::create_dir_all(&source_root).unwrap();
            let path = super::lock_path(&context, &source_root).unwrap();
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            let mut lease = acquire_index_lock(&path, "build", &source_root)
                .unwrap()
                .unwrap();
            *lease.observation.lock().unwrap() =
                MaintenanceObservation::Observed(IndexMaintenanceStatus::Queued);
            let mut owner = lease.owner.lock().unwrap();
            let identity = file_identity(&owner.file).unwrap();
            let mut foreign = owner.lock.clone();
            match case {
                "foreign-id" => foreign.lock_id = "foreign-owner".to_string(),
                "inactive" => foreign.mark_released(),
                "wrong-schema" => foreign.schema_version += 1,
                _ => {}
            }
            let bytes = if case == "invalid-json" {
                b"broken-json".to_vec()
            } else {
                serde_json::to_vec(&foreign).unwrap()
            };
            owner.file.set_len(0).unwrap();
            owner.file.seek(SeekFrom::Start(0)).unwrap();
            owner.file.write_all(&bytes).unwrap();
            owner.file.flush().unwrap();
            assert_eq!(file_identity(&owner.file).unwrap(), identity);
            drop(owner);
            assert!(observed_maintenance(&path).is_none(), "{case}");
            assert!(!lease.validate_ownership(), "{case}");
            assert!(!lease.refresh(123), "{case}");
            drop(lease);
            assert_eq!(
                fs::read(&path).unwrap(),
                bytes,
                "{case}: stale owner wrote the lock"
            );
            cleanup(&context);
        }
    }

    #[test]
    fn poisoned_owner_never_proves_maintenance_or_writes_a_release_record() {
        let context = test_context("maintenance-poisoned-owner");
        let source_root = context.workspace_root.join("src");
        fs::create_dir_all(&source_root).unwrap();
        let path = super::lock_path(&context, &source_root).unwrap();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut lease = acquire_index_lock(&path, "build", &source_root)
            .unwrap()
            .unwrap();
        let owner = Arc::clone(&lease.owner);
        assert!(std::panic::catch_unwind(move || {
            let _guard = owner.lock().unwrap();
            panic!("controlled owner mutex poison");
        })
        .is_err());
        assert!(observed_maintenance(&path).is_none());
        assert!(!lease.validate_ownership());
        assert!(!lease.refresh(123));
        drop(lease);
        assert!(
            read_lock_path(&path).unwrap().is_active(),
            "poisoned owner must not publish a release"
        );
        cleanup(&context);
    }

    #[test]
    fn foreign_or_replaced_lock_never_proves_queued_or_running_maintenance() {
        use crate::infrastructure::platform::filesystem::file_identity;
        use std::io::Read;

        let context = test_context("maintenance-ownership");
        let source_root = context.workspace_root.join("src");
        fs::create_dir_all(&source_root).unwrap();
        let runner = RecordingIndexRunner::default();
        let service = WorkspaceIndexService::with_runner(&runner);
        let path = super::lock_path(&context, &source_root).unwrap();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        write_lock_path(&path, BslIndexLock::new("build", &source_root)).unwrap();
        let before = fs::read(&path).unwrap();
        let mut foreign = File::options().read(true).write(true).open(&path).unwrap();
        let held_identity = file_identity(&foreign).unwrap();
        foreign.lock_exclusive().unwrap();
        let report =
            service.start_isolated_build(&context, &source_root, &CancellationToken::new());
        assert_eq!(report.maintenance, IndexMaintenanceStatus::Unknown);
        assert!(runner.backgrounds.borrow().is_empty());
        assert_eq!(
            file_identity(&File::open(&path).unwrap()).unwrap(),
            held_identity,
            "the foreign owner's named file must not be replaced"
        );
        // Windows enforces the byte-range lock against a newly opened reader,
        // including another handle in this process. Inspect through its owner
        // while keeping that lock held and restoring the owner's file cursor.
        let position = foreign.stream_position().unwrap();
        foreign.seek(SeekFrom::Start(0)).unwrap();
        let mut after = Vec::new();
        foreign.read_to_end(&mut after).unwrap();
        foreign.seek(SeekFrom::Start(position)).unwrap();
        assert_eq!(after, before);
        foreign.unlock().unwrap();
        drop(foreign);
        assert_eq!(
            service
                .start_isolated_build(&context, &source_root, &CancellationToken::new())
                .maintenance,
            IndexMaintenanceStatus::Queued
        );
        let job = runner.backgrounds.borrow_mut().pop().unwrap();
        let observation = Arc::clone(&job.lock_lease.observation);
        write_lock_path(&path, BslIndexLock::new("build", &source_root)).unwrap();
        let successor = fs::read(&path).unwrap();
        assert!(service.maintenance_status(&context, &source_root).is_none());
        let report =
            service.start_isolated_build(&context, &source_root, &CancellationToken::new());
        assert_eq!(report.maintenance, IndexMaintenanceStatus::Unknown);
        run_background_job_with(job, |_, _| panic!("a replaced owner cannot execute"));
        assert_eq!(
            observed_status(&observation),
            IndexMaintenanceStatus::Unknown
        );
        assert_eq!(fs::read(&path).unwrap(), successor);
        cleanup(&context);
    }

    #[test]
    fn builder_17_uses_a_new_generation_and_leaves_older_generations_untouched() {
        let context = test_context("generation-cutover");
        let source_root = context.workspace_root.join("src");
        fs::create_dir_all(&source_root).unwrap();
        let state_root = rlm_provider_state_root(&context, &source_root).unwrap();
        let legacy = state_root.join("rlm-tools-bsl");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(legacy.join("builder-14-sentinel"), "keep").unwrap();
        // RLM 1.42 writes builder-17 schema; a builder-15 index of RLM 1.33
        // stays where it was and is never handed to the new reader.
        let builder_15 = state_root.join("rlm-bsl/index-v15");
        fs::create_dir_all(&builder_15).unwrap();
        fs::write(builder_15.join("bsl_index.db"), "builder-15-db").unwrap();

        let commands = WorkspaceIndexService::with_runner(&RecordingIndexRunner::default())
            .commands(&context, &source_root, &CancellationToken::new())
            .unwrap();
        let actual = PathBuf::from(index_command_env(&commands.info, "RLM_INDEX_DIR"));

        assert_eq!(actual, state_root.join("rlm-bsl/index-v17"));
        assert_eq!(
            fs::read_to_string(builder_15.join("bsl_index.db")).unwrap(),
            "builder-15-db"
        );
        assert_eq!(index_command_env(&commands.info, "PYTHONUTF8"), "1");
        assert_eq!(
            index_command_env(&commands.info, "PYTHONIOENCODING"),
            "utf-8:surrogateescape"
        );
        assert_eq!(
            fs::read_to_string(legacy.join("builder-14-sentinel")).unwrap(),
            "keep"
        );
        cleanup(&context);
    }

    /// Чтение активной сборки запускает уборку прежнего поколения той же пары:
    /// давно не тронутое index-v15 исчезает, текущее поколение остаётся.
    #[test]
    fn active_build_collects_an_idle_previous_generation_but_not_the_current_one() {
        let context = test_context("generation-collection-wiring");
        let source_root = default_source_root(&context);
        fs::create_dir_all(&source_root).unwrap();
        let pair_root = rlm_provider_state_root(&context, &source_root).unwrap();
        let previous = [
            pair_root.join("rlm-bsl/index-v15"),
            pair_root.join("caches/rlm-bsl/index-v15"),
            pair_root.join("locks/rlm-bsl/index-v15"),
        ];
        for directory in &previous {
            fs::create_dir_all(directory).unwrap();
            fs::write(directory.join("payload"), b"builder-15").unwrap();
        }
        let current = pair_root.join("rlm-bsl").join(RLM_INDEX_GENERATION);
        fs::create_dir_all(&current).unwrap();
        fs::write(current.join("payload"), b"current").unwrap();
        let old = SystemTime::now() - Duration::from_secs(2 * 24 * 60 * 60);
        let aged = previous.iter().chain([&current]).try_for_each(|directory| {
            File::options()
                .write(true)
                .open(directory.join("payload"))?
                .set_modified(old)?;
            File::open(directory)?.set_modified(old)
        });
        if aged.is_err() {
            cleanup(&context);
            // Windows cannot open a directory this way to set its time: the
            // wiring is proven on Unix only.
            return;
        }

        let _ = WorkspaceIndexService::with_runner(&RecordingIndexRunner::default())
            .active_build(&context, &source_root);
        let deadline = Instant::now() + Duration::from_secs(10);
        while previous.iter().any(|directory| directory.exists()) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }

        for directory in &previous {
            assert!(!directory.exists(), "{}", directory.display());
        }
        assert_eq!(fs::read(current.join("payload")).unwrap(), b"current");
        cleanup(&context);
    }

    #[test]
    fn builder_generation_environment_preserves_native_path_bytes() {
        let mut context = test_context("native-generation-environment");
        let Some(invalid_component) = testing::non_utf8_relative_path_for_test() else {
            cleanup(&context);
            return;
        };
        context.cache_root = context.workspace_root.join(invalid_component);
        let source_root = default_source_root(&context);
        fs::create_dir_all(&source_root).unwrap();
        let expected_reader_environment = rlm_generation_root(&context, &source_root).unwrap();

        let commands = WorkspaceIndexService::with_runner(&RecordingIndexRunner::default())
            .commands(&context, &source_root, &CancellationToken::new())
            .unwrap();

        assert_eq!(
            PathBuf::from(index_command_env(&commands.info, "RLM_INDEX_DIR")),
            expected_reader_environment
        );
        assert_eq!(index_command_env(&commands.info, "PYTHONUTF8"), "1");
        assert_eq!(
            index_command_env(&commands.info, "PYTHONIOENCODING"),
            "utf-8:surrogateescape"
        );
        assert_eq!(PathBuf::from(&commands.info.args[2]), source_root);
        cleanup(&context);
    }

    #[test]
    fn builder_command_preserves_native_source_root_argument_bytes() {
        let context = test_context("native-source-argument");
        let Some(invalid_component) = testing::non_utf8_relative_path_for_test() else {
            cleanup(&context);
            return;
        };
        let source_root = context.workspace_root.join(invalid_component);

        let commands = WorkspaceIndexService::with_runner(&RecordingIndexRunner::default())
            .commands(&context, &source_root, &CancellationToken::new())
            .unwrap();

        assert_eq!(PathBuf::from(&commands.info.args[2]), source_root);
        cleanup(&context);
    }

    #[test]
    fn builder_15_rejects_a_generation_alias_to_builder_14_storage() {
        let context = test_context("generation-alias-to-builder-14");
        let source_root = default_source_root(&context);
        fs::create_dir_all(&source_root).unwrap();
        let pair_root = rlm_provider_state_root(&context, &source_root).unwrap();
        let legacy = pair_root.join("rlm-tools-bsl/index");
        fs::create_dir_all(&legacy).unwrap();
        let legacy_db = legacy.join("bsl_index.db");
        fs::write(&legacy_db, b"builder-14-db").unwrap();
        let alias = pair_root.join("rlm-bsl/index-v17");
        fs::create_dir_all(alias.parent().unwrap()).unwrap();
        let Some(link) = testing::create_dir_symlink_for_test(&legacy, &alias) else {
            cleanup(&context);
            return;
        };
        link.unwrap();

        let error = WorkspaceIndexService::with_runner(&RecordingIndexRunner::default())
            .commands(&context, &source_root, &CancellationToken::new())
            .unwrap_err();

        assert!(error.contains("symbolic link or reparse point"));
        assert_eq!(fs::read(&legacy_db).unwrap(), b"builder-14-db");
        assert!(!legacy.join(STATUS_FILE_NAME).exists());
        cleanup(&context);
    }

    #[test]
    fn legacy_status_and_lock_do_not_gate_builder_15() {
        let context = test_context("legacy-markers");
        let source_root = context.workspace_root.join("src");
        fs::create_dir_all(&source_root).unwrap();
        fs::create_dir_all(context.cache_root.join("caches")).unwrap();
        fs::create_dir_all(context.cache_root.join("locks")).unwrap();
        fs::write(
            context.cache_root.join("caches/bsl_index_status.json"),
            "legacy",
        )
        .unwrap();
        fs::write(context.cache_root.join("locks/bsl_index.lock"), "legacy").unwrap();

        assert!(!super::status_path(&context, &source_root)
            .unwrap()
            .ends_with("caches/bsl_index_status.json"));
        assert!(!super::lock_path(&context, &source_root)
            .unwrap()
            .ends_with("locks/bsl_index.lock"));
        assert_eq!(
            fs::read_to_string(context.cache_root.join("caches/bsl_index_status.json")).unwrap(),
            "legacy"
        );
        assert_eq!(
            fs::read_to_string(context.cache_root.join("locks/bsl_index.lock")).unwrap(),
            "legacy"
        );
        cleanup(&context);
    }

    #[test]
    fn current_pair_scoped_builder_14_bytes_are_ignored_and_preserved() {
        let context = test_context("pair-scoped-legacy-markers");
        let source_root = context.workspace_root.join("src");
        fs::create_dir_all(&source_root).unwrap();
        let pair_root = rlm_provider_state_root(&context, &source_root).unwrap();
        let legacy_db = pair_root.join("rlm-tools-bsl/index/bsl_index.db");
        let legacy_status = pair_root.join("caches/bsl_index_status.json");
        let legacy_lock = pair_root.join("locks/bsl_index.lock");
        for path in [&legacy_db, &legacy_status, &legacy_lock] {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
        }
        fs::write(&legacy_db, b"builder-14-db").unwrap();
        fs::write(&legacy_status, b"builder-14-status").unwrap();
        fs::write(&legacy_lock, b"builder-14-lock").unwrap();
        let before = [
            fs::read(&legacy_db).unwrap(),
            fs::read(&legacy_status).unwrap(),
            fs::read(&legacy_lock).unwrap(),
        ];
        let runner = RecordingIndexRunner::default();

        let report = WorkspaceIndexService::with_runner(&runner).start_isolated_build(
            &context,
            &source_root,
            &CancellationToken::new(),
        );

        assert_eq!(report.maintenance, IndexMaintenanceStatus::Queued);
        assert_eq!(runner.backgrounds.borrow()[0].action, "build");
        let candidate = PathBuf::from(index_command_env(
            &runner.backgrounds.borrow()[0].primary,
            "RLM_INDEX_DIR",
        ));
        assert!(
            candidate.starts_with(
                rlm_generation_root(&context, &source_root)
                    .unwrap()
                    .join("builds")
            ),
            "{candidate:?}"
        );
        assert_eq!(fs::read(&legacy_db).unwrap(), before[0]);
        assert_eq!(fs::read(&legacy_status).unwrap(), before[1]);
        assert_eq!(fs::read(&legacy_lock).unwrap(), before[2]);
        cleanup(&context);
    }

    #[test]
    fn active_build_rejects_db_outside_declared_build_directory() {
        enum InvalidDbLocation {
            Builder14,
            SiblingBuild,
        }

        for (name, location) in [
            ("builder-14", InvalidDbLocation::Builder14),
            ("sibling-build", InvalidDbLocation::SiblingBuild),
        ] {
            let context = test_context(&format!("active-build-{name}"));
            let source_root = default_source_root(&context);
            fs::create_dir_all(&source_root).unwrap();
            let build_id = "a".repeat(32);
            let db_path = match location {
                InvalidDbLocation::Builder14 => rlm_provider_state_root(&context, &source_root)
                    .unwrap()
                    .join("rlm-tools-bsl/index/bsl_index.db"),
                InvalidDbLocation::SiblingBuild => {
                    rlm_build_root(&context, &source_root, &"b".repeat(32))
                        .unwrap()
                        .join("bsl_index.db")
                }
            };
            fs::create_dir_all(db_path.parent().unwrap()).unwrap();
            fs::write(&db_path, name).unwrap();
            write_status(
                &context,
                BslIndexStatus::ready(&source_root, &db_path).with_build_id(build_id.clone()),
            )
            .unwrap();

            assert!(db_path.is_file(), "{name}: fixture DB must exist");
            assert_eq!(
                read_bsl_index_status(&context).unwrap().build_id.as_deref(),
                Some(build_id.as_str()),
                "{name}: marker must reach the build-directory check"
            );
            let active = WorkspaceIndexService::with_runner(&RecordingIndexRunner::default())
                .active_build(&context, &source_root)
                .unwrap();
            assert_eq!(active, None, "{name}: DB outside its build became active");
            cleanup(&context);
        }
    }

    #[test]
    fn active_builder_15_lock_beats_ready_marker_for_cache_readiness() {
        let context = test_context("cache-readiness-active-lock");
        let source_root = context.workspace_root.join("src");
        fs::create_dir_all(&source_root).unwrap();
        let db_path = generation_db_path(&context, "ready/bsl_index.db");
        fs::create_dir_all(db_path.parent().unwrap()).unwrap();
        fs::write(&db_path, b"builder-15-db").unwrap();
        write_status(&context, BslIndexStatus::ready(&source_root, &db_path)).unwrap();
        write_fresh_lock(&context, "update");

        assert!(!bsl_index_is_ready(&context));
        cleanup(&context);
    }

    #[test]
    fn background_worker_never_publishes_ready_for_db_outside_builder_15_generation() {
        let context = test_context("worker-legacy-db-never-readable");
        let source_root = context.workspace_root.join("src");
        fs::create_dir_all(&source_root).unwrap();
        let legacy_db = rlm_provider_state_root(&context, &source_root)
            .unwrap()
            .join("rlm-tools-bsl/index/bsl_index.db");
        fs::create_dir_all(legacy_db.parent().unwrap()).unwrap();
        fs::write(&legacy_db, b"builder-14-db").unwrap();
        let job = test_background_job(&context, "build");

        run_background_job_with(job, |command, _lease| {
            if command.args.get(1).is_some_and(|arg| arg == "info") {
                Ok(IndexOutput::success(format!(
                    "Index: {}\n  Status:   fresh\n",
                    legacy_db.display()
                )))
            } else {
                Ok(IndexOutput::success("Index built"))
            }
        });

        assert_ne!(read_bsl_index_status(&context).unwrap().status, "ready");
        assert_eq!(fs::read(&legacy_db).unwrap(), b"builder-14-db");
        cleanup(&context);
    }

    #[test]
    fn cancellation_prefix_is_stable_for_cancelled_index_output() {
        let readiness = readiness_from_info(&IndexOutput {
            status_success: false,
            status: "cancelled".to_string(),
            stdout: String::new(),
            stderr: String::new(),
            timed_out: false,
            cancelled: true,
            duration_ms: 0,
        });

        assert!(matches!(
            readiness,
            IndexReadiness::Unavailable(error) if error.starts_with("cancelled:")
        ));
    }

    #[test]
    fn info_parser_preserves_exact_stale_status() {
        for status in [
            "stale (content)",
            "stale (age)",
            "stale (structure changed)",
        ] {
            let readiness = readiness_from_info(&IndexOutput::success(format!(
                "Index: /tmp/bsl_index.db\n  Status:   {status}\n"
            )));
            assert_eq!(
                readiness,
                IndexReadiness::Stale {
                    status: status.to_string()
                }
            );
        }
    }

    #[test]
    fn incomplete_info_is_retryable_but_never_readable() {
        let readiness = readiness_from_info(&IndexOutput::success(
            "Index: /tmp/bsl_index.db\n  Status:   incomplete\n",
        ));

        assert_eq!(readiness, IndexReadiness::Incomplete);
    }

    #[test]
    fn path_normalization_failures_do_not_match_index_identity() {
        let context = test_context("invalid-path-identity");
        let dangling = context.workspace_root.join("dangling");
        let Some(symlink) = testing::create_file_symlink_for_test(
            context.workspace_root.join("missing"),
            &dangling,
        ) else {
            cleanup(&context);
            return;
        };
        symlink.unwrap();
        let dangling_text = dangling.display().to_string();

        assert!(!stored_path_matches(Some(&dangling_text), &dangling));
        cleanup(&context);
    }

    #[test]
    fn registered_ownership_check_detects_replacement_without_disk_validation() {
        let context = test_context("registered-ownership");
        let job = test_background_job(&context, "update");
        let lock = lock_path(&context);

        assert!(job.lock_lease.registered_ownership_is_current());
        register_active_lock(&lock, "replacement-owner", &job.lock_lease.owner);
        assert!(!job.lock_lease.registered_ownership_is_current());

        unregister_active_lock(&lock, "replacement-owner");
        drop(job);
        cleanup(&context);
    }

    #[test]
    fn running_command_is_cancelled_when_registered_ownership_is_replaced() {
        let context = test_context("running-command-displaced");
        let mut job = test_background_job(&context, "update");
        let command = long_running_index_command(&context);
        let lock = lock_path(&context);
        let owner = Arc::clone(&job.lock_lease.owner);
        let replacement = thread::spawn(move || {
            thread::sleep(Duration::from_millis(500));
            register_active_lock(&lock, "replacement-owner", &owner);
        });

        let started = Instant::now();
        let output = run_index_command_with_heartbeat(&command, Some(&mut job.lock_lease))
            .expect("ownership loss should return a managed child output");
        replacement.join().unwrap();

        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(output.cancelled);
        unregister_active_lock(&lock_path(&context), "replacement-owner");
        drop(job);
        cleanup(&context);
    }

    #[test]
    fn successful_background_job_records_last_run_metrics_in_status() {
        let context = test_context("metrics");
        let source_root = context.workspace_root.join("src");
        let build_id = "0123456789abcdef0123456789abcdef";
        let build_dir = rlm_build_root(&context, &source_root, build_id).unwrap();
        let db_path = build_dir.join("bsl_index.db");
        fs::create_dir_all(db_path.parent().unwrap()).unwrap();
        fs::write(&db_path, "").unwrap();
        let status = status_path(&context);
        let lock = lock_path(&context);
        fs::create_dir_all(lock.parent().unwrap()).unwrap();
        let lock_lease = acquire_index_lock(&lock, "build", &context.workspace_root.join("src"))
            .unwrap()
            .expect("lock should be acquired for background job");

        let mut primary = print_lines_command(
            &context.workspace_root,
            true,
            &[
                "Index built in 1.2s".to_string(),
                "  Index:    v14".to_string(),
                "  Modules:  24".to_string(),
                "  Methods:  617".to_string(),
                "  DB size:  1.3 MB".to_string(),
            ],
            CancellationToken::new(),
        );
        let mut info = print_lines_command(
            &context.workspace_root,
            false,
            &[
                format!("Index: {}", db_path.display()),
                "  Status:   fresh".to_string(),
            ],
            CancellationToken::new(),
        );
        for command in [&mut primary, &mut info] {
            command.env = rlm_process_environment(build_dir.clone());
        }
        run_background_job(IndexBackgroundJob {
            action: "build".to_string(),
            context: context.clone(),
            source_root,
            root_capability: None,
            primary,
            info,
            status_path: status.clone(),
            lock_path: lock.clone(),
            lock_lease,
        });

        let value: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&status).unwrap()).unwrap();
        let metrics = value
            .get("last_run")
            .expect("ready status should include last_run metrics");
        assert_eq!(metrics["action"], "build");
        assert_eq!(metrics["timed_out"], false);
        assert!(metrics["duration_ms"].as_u64().unwrap() > 0);
        assert!(
            metrics["finished_at"].as_u64().unwrap() >= metrics["started_at"].as_u64().unwrap()
        );
        assert_eq!(metrics["index_version"], "v14");
        assert_eq!(metrics["modules"], 24);
        assert_eq!(metrics["methods"], 617);
        assert_eq!(metrics["db_size"], "1.3 MB");
        let current = read_lock_path(&lock).expect("completed job should leave a marker");
        assert_eq!(current.state, "released");
        assert!(current.child_pid.is_some());
        cleanup(&context);
    }

    #[test]
    fn actor_bound_background_job_publishes_no_result_after_mid_command_root_swap() {
        let context = test_context("actor-bound-background-root-replacement");
        let source_root = context.workspace_root.join("src");
        fs::create_dir_all(&source_root).unwrap();
        let source_root = fs::canonicalize(source_root).unwrap();
        let capability = Arc::new(RetainedDirectoryCapability::open(&source_root).unwrap());
        let mut job = test_background_job(&context, "build");
        job.root_capability = Some(capability);
        write_status_path(
            &job.status_path,
            BslIndexStatus::building("build", Some(&source_root)),
        )
        .unwrap();
        let before = fs::read(&job.status_path).unwrap();
        let observation = Arc::clone(&job.lock_lease.observation);
        let displaced = context.workspace_root.join("src-displaced");
        let replacement = source_root.clone();

        run_background_job_with(job, move |_command, _lease| {
            fs::rename(&replacement, &displaced).unwrap();
            fs::create_dir_all(&replacement).unwrap();
            Ok(IndexOutput::success("Index built"))
        });

        assert_eq!(fs::read(status_path(&context)).unwrap(), before);
        assert_eq!(
            observed_status(&observation),
            IndexMaintenanceStatus::Unknown
        );
        cleanup(&context);
    }

    #[test]
    fn actor_bound_index_discards_background_start_error_after_root_replacement() {
        struct ReplacingStartRunner {
            source_root: PathBuf,
            displaced: PathBuf,
        }

        impl IndexRunner for ReplacingStartRunner {
            fn start_background(&self, _job: IndexBackgroundJob) -> Result<(), String> {
                fs::rename(&self.source_root, &self.displaced).unwrap();
                fs::create_dir_all(&self.source_root).unwrap();
                Err("FOREIGN-REPLACEMENT-DIAGNOSTICS".to_string())
            }
        }

        let context = test_context("actor-bound-background-start-root-replacement");
        let source_root = context.workspace_root.join("src");
        fs::create_dir_all(&source_root).unwrap();
        let source_root = normalize_path_identity(&source_root).unwrap();
        let capability = Arc::new(RetainedDirectoryCapability::open(&source_root).unwrap());
        let runner = ReplacingStartRunner {
            source_root: source_root.clone(),
            displaced: context.workspace_root.join("src-displaced"),
        };
        let service =
            WorkspaceIndexService::with_runner(&runner).with_bound_source_root(capability);
        let report =
            service.start_isolated_build(&context, &source_root, &CancellationToken::new());

        assert!(
            report
                .warnings
                .iter()
                .any(|warning| warning.contains("changed after admission")),
            "{report:?}"
        );
        assert!(read_bsl_index_status(&context).is_none());
        cleanup(&context);
    }

    #[test]
    fn cancelled_index_info_returns_promptly() {
        let context = test_context("cancelled-info");
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let command = print_lines_command(
            &context.workspace_root,
            true,
            &["Index: /tmp/bsl_index.db".to_string()],
            cancellation,
        );

        let started = Instant::now();
        let output = run_index_command(&command).expect("cancelled command should return output");

        assert!(output.cancelled);
        assert!(!output.status_success);
        assert!(started.elapsed() < Duration::from_secs(2));
        cleanup(&context);
    }

    #[test]
    fn timed_out_index_info_returns_promptly_without_cancellation() {
        let context = test_context("timed-out-info");
        let mut command = print_lines_command(
            &context.workspace_root,
            true,
            &["Index: /tmp/bsl_index.db".to_string()],
            CancellationToken::new(),
        );
        command.timeout = Duration::ZERO;

        let started = Instant::now();
        let output = run_index_command(&command).expect("timed-out command should return output");

        assert!(output.timed_out);
        assert!(!output.cancelled);
        assert!(!output.status_success);
        assert!(started.elapsed() < Duration::from_secs(2));
        cleanup(&context);
    }

    #[test]
    fn managed_cancelled_output_never_maps_to_success() {
        let output = map_managed_output(
            crate::infrastructure::platform::ManagedOutput {
                status_success: true,
                status: "exit status: 0".to_string(),
                stdout: String::new(),
                stderr: String::new(),
                timed_out: false,
                cancelled: true,
                stdout_truncated: false,
                stderr_truncated: false,
                stdout_had_invalid_utf8: false,
                stderr_had_invalid_utf8: false,
            },
            Duration::from_millis(1),
        );

        assert!(!output.status_success);
        assert!(output.cancelled);
        assert!(!output.timed_out);
    }

    #[test]
    fn managed_timed_out_output_never_maps_to_success() {
        let output = map_managed_output(
            crate::infrastructure::platform::ManagedOutput {
                status_success: true,
                status: "exit status: 0".to_string(),
                stdout: String::new(),
                stderr: String::new(),
                timed_out: true,
                cancelled: false,
                stdout_truncated: false,
                stderr_truncated: false,
                stdout_had_invalid_utf8: false,
                stderr_had_invalid_utf8: false,
            },
            Duration::from_millis(1),
        );

        assert!(!output.status_success);
        assert!(output.timed_out);
        assert!(!output.cancelled);
    }

    #[test]
    fn managed_truncation_is_visible_at_index_boundary() {
        let output = map_managed_output(
            crate::infrastructure::platform::ManagedOutput {
                status_success: false,
                status: "exit status: 0".into(),
                stdout: "tail".into(),
                stderr: "diagnostic tail".into(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: true,
                stderr_truncated: true,
                stdout_had_invalid_utf8: false,
                stderr_had_invalid_utf8: false,
            },
            Duration::from_millis(1),
        );
        assert!(output.stderr.contains("stdout capture truncated"));
        assert!(output.stderr.contains("earlier stderr diagnostics omitted"));
    }

    #[test]
    fn cancelled_background_job_records_failure_and_releases_lock() {
        let context = test_context("cancelled-background");
        let status = status_path(&context);
        let lock = lock_path(&context);
        fs::create_dir_all(lock.parent().unwrap()).unwrap();
        let lock_lease = acquire_index_lock(&lock, "build", &context.workspace_root.join("src"))
            .unwrap()
            .expect("lock should be acquired for background job");
        let cancellation = CancellationToken::new();
        cancellation.cancel();

        run_background_job(IndexBackgroundJob {
            action: "build".to_string(),
            context: context.clone(),
            source_root: context.workspace_root.join("src"),
            root_capability: None,
            primary: print_lines_command(
                &context.workspace_root,
                true,
                &["Index built".to_string()],
                cancellation,
            ),
            info: print_lines_command(
                &context.workspace_root,
                false,
                &["Index not found: /tmp/bsl_index.db".to_string()],
                CancellationToken::new(),
            ),
            status_path: status.clone(),
            lock_path: lock.clone(),
            lock_lease,
        });

        let current_status: BslIndexStatus =
            serde_json::from_str(&fs::read_to_string(&status).unwrap()).unwrap();
        assert_eq!(current_status.status, "failed");
        assert!(current_status
            .message
            .as_deref()
            .is_some_and(|message| message.starts_with("cancelled:")));
        assert!(current_status.last_run.is_some());
        let current_lock = read_lock_path(&lock).expect("cancelled job should leave a marker");
        assert_eq!(current_lock.state, "released");
        cleanup(&context);
    }

    #[test]
    fn released_lock_does_not_block_next_index_build() {
        let context = test_context("released-lock");
        fs::create_dir_all(context.workspace_root.join("src/CommonModules")).unwrap();
        write_released_lock(&context, "build");
        write_old_building_status(&context, "build");
        let runner = RecordingIndexRunner::default();
        let service = WorkspaceIndexService::with_runner(&runner);

        let report = service.start_isolated_build(
            &context,
            &context.workspace_root.join("src"),
            &CancellationToken::new(),
        );

        assert_eq!(report.maintenance, IndexMaintenanceStatus::Queued);
        assert_eq!(
            runner.backgrounds.borrow()[0].primary.args[0..2],
            ["index", "build"]
        );
        cleanup(&context);
    }

    #[test]
    fn invalid_lock_without_building_status_is_treated_as_active() {
        let context = test_context("invalid-lock-active");
        let source_root = default_source_root(&context);
        fs::create_dir_all(&source_root).unwrap();
        write_usable_build(&context, &"c".repeat(32));
        let lock = lock_path(&context);
        fs::create_dir_all(lock.parent().unwrap()).unwrap();
        fs::write(&lock, b"").unwrap();
        let lock_before = fs::read(&lock).unwrap();

        assert!(!bsl_index_is_ready(&context));
        assert_eq!(
            fs::read(&lock).unwrap(),
            lock_before,
            "fresh invalid lock must not be recovered"
        );
        cleanup(&context);
    }

    #[test]
    fn fresh_invalid_lock_with_stale_status_is_treated_as_active() {
        let context = test_context("invalid-lock-with-stale-status");
        let source_root = default_source_root(&context);
        fs::create_dir_all(&source_root).unwrap();
        let lock = lock_path(&context);
        fs::create_dir_all(lock.parent().unwrap()).unwrap();
        fs::write(&lock, b"").unwrap();
        write_old_building_status(&context, "build");
        let lock_before = fs::read(&lock).unwrap();
        let status_before = fs::read(status_path(&context)).unwrap();

        assert!(active_lock(&context, &source_root).unwrap());
        assert_eq!(fs::read(&lock).unwrap(), lock_before);
        assert_eq!(fs::read(status_path(&context)).unwrap(), status_before);
        assert_eq!(read_bsl_index_status(&context).unwrap().status, "building");
        cleanup(&context);
    }

    #[test]
    fn stale_legacy_lock_is_recovered_by_active_lock() {
        let context = test_context("stale-legacy-lock");
        let source_root = default_source_root(&context);
        fs::create_dir_all(&source_root).unwrap();
        let lock = lock_path(&context);
        fs::create_dir_all(lock.parent().unwrap()).unwrap();
        fs::write(&lock, b"").unwrap();
        write_old_building_status(&context, "build");
        make_lock_file_old(&context);

        assert!(!active_lock(&context, &source_root).unwrap());
        assert_eq!(read_lock_path(&lock).unwrap().state, "recovered");
        assert_eq!(read_bsl_index_status(&context).unwrap().status, "failed");
        cleanup(&context);
    }

    #[test]
    fn stale_lock_held_by_current_process_is_still_active() {
        let context = test_context("stale-held-lock");
        fs::create_dir_all(context.workspace_root.join("src/CommonModules")).unwrap();
        let lock = lock_path(&context);
        fs::create_dir_all(lock.parent().unwrap()).unwrap();
        let mut lease = acquire_index_lock(&lock, "build", &context.workspace_root.join("src"))
            .unwrap()
            .expect("lock should be acquired");
        force_lock_updated_at(
            &mut lease,
            now_secs().saturating_sub(LOCK_STALE_AFTER.as_secs() + 1),
        );
        assert!(active_lock(&context, &context.workspace_root.join("src")).unwrap());
        drop(lease);
        cleanup(&context);
    }

    #[test]
    fn cleanup_does_not_remove_lock_replaced_by_new_owner() {
        let context = test_context("cleanup-owner");
        let lock = lock_path(&context);
        fs::create_dir_all(lock.parent().unwrap()).unwrap();
        let lease = acquire_index_lock(&lock, "build", &context.workspace_root.join("src"))
            .unwrap()
            .expect("old owner should acquire lock");
        let mut new_lock = BslIndexLock::new("build", &context.workspace_root.join("src"));
        new_lock.lock_id = "new-owner".to_string();
        write_lock_path(&lock, new_lock.clone()).unwrap();

        drop(lease);

        let current = read_lock_path(&lock).expect("replacement lock should remain");
        assert_eq!(current.lock_id, new_lock.lock_id);
        cleanup(&context);
    }

    #[test]
    fn heartbeat_does_not_overwrite_lock_replaced_by_new_owner() {
        let context = test_context("heartbeat-owner");
        let lock = lock_path(&context);
        fs::create_dir_all(lock.parent().unwrap()).unwrap();
        let mut lease = acquire_index_lock(&lock, "build", &context.workspace_root.join("src"))
            .unwrap()
            .expect("old owner should acquire lock");
        let mut new_lock = BslIndexLock::new("build", &context.workspace_root.join("src"));
        new_lock.lock_id = "new-owner".to_string();
        write_lock_path(&lock, new_lock.clone()).unwrap();

        lease.refresh(42);

        let current = read_lock_path(&lock).expect("replacement lock should remain readable");
        assert_eq!(current.lock_id, new_lock.lock_id);
        assert_eq!(current.child_pid, new_lock.child_pid);
        drop(lease);
        cleanup(&context);
    }

    #[test]
    fn failed_background_start_does_not_remove_lock_replaced_by_new_owner() {
        let context = test_context("start-background-owner");
        fs::create_dir_all(context.workspace_root.join("src/CommonModules")).unwrap();
        let lock = lock_path(&context);
        let runner = FailingReplacingIndexRunner {
            replacement_lock_id: "new-owner".to_string(),
        };
        let service = WorkspaceIndexService::with_runner(&runner);

        let report = service.start_isolated_build(
            &context,
            &context.workspace_root.join("src"),
            &CancellationToken::new(),
        );

        assert!(report
            .warnings
            .iter()
            .any(|warning| warning.contains("simulated background start failure")));
        let current = read_lock_path(&lock).expect("replacement lock should remain");
        assert_eq!(current.lock_id, "new-owner");
        cleanup(&context);
    }

    #[test]
    fn stale_structured_lock_is_recovered_by_active_lock() {
        let context = test_context("stale-structured-recovered");
        let source_root = default_source_root(&context);
        fs::create_dir_all(&source_root).unwrap();
        write_stale_lock(&context, "build");
        write_old_building_status(&context, "build");

        assert!(!active_lock(&context, &source_root).unwrap());

        let current =
            read_lock_path(&lock_path(&context)).expect("stale lock should remain as marker");
        assert_eq!(current.state, "recovered");
        assert_eq!(read_bsl_index_status(&context).unwrap().status, "failed");
        cleanup(&context);
    }

    #[derive(Default)]
    struct RecordingIndexRunner {
        backgrounds: RefCell<Vec<IndexBackgroundJob>>,
    }

    impl IndexRunner for RecordingIndexRunner {
        fn start_background(&self, job: IndexBackgroundJob) -> Result<(), String> {
            self.backgrounds.borrow_mut().push(job);
            Ok(())
        }
    }

    struct FailingReplacingIndexRunner {
        replacement_lock_id: String,
    }

    impl IndexRunner for FailingReplacingIndexRunner {
        fn start_background(&self, job: IndexBackgroundJob) -> Result<(), String> {
            let mut replacement = BslIndexLock::new("build", &job.source_root);
            replacement.lock_id = self.replacement_lock_id.clone();
            write_lock_path(&job.lock_path, replacement).unwrap();
            Err("simulated background start failure".to_string())
        }
    }

    fn force_lock_updated_at(lease: &mut IndexLockLease, updated_at: u64) {
        let mut owner = lease.owner.lock().unwrap();
        owner.lock.updated_at = updated_at;
        owner.write_record().unwrap();
    }

    impl IndexOutput {
        fn success(stdout: impl Into<String>) -> Self {
            Self {
                status_success: true,
                status: "exit status: 0".to_string(),
                stdout: stdout.into(),
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                duration_ms: 0,
            }
        }
    }

    fn inert_index_command(context: &WorkspaceContext, verb: &str) -> IndexCommand {
        IndexCommand {
            program: PathBuf::from("unused-by-scripted-runner"),
            args: vec![
                "index".into(),
                verb.into(),
                context.workspace_root.join("src").into_os_string(),
            ],
            cwd: context.workspace_root.clone(),
            env: vec![(
                "RLM_INDEX_DIR".into(),
                rlm_generation_root(context, &default_source_root(context))
                    .unwrap()
                    .into_os_string(),
            )],
            // These fixtures assert on a finished run, not on how fast it
            // finished. The Windows fixture shells out to `powershell`, whose
            // cold start alone can take seconds on a loaded runner, so a tight
            // budget reports `timed_out` for a command that did its job.
            timeout: Duration::from_secs(30),
            cancellation: CancellationToken::new(),
        }
    }

    fn long_running_index_command(context: &WorkspaceContext) -> IndexCommand {
        let command = testing::long_running_command();
        IndexCommand {
            program: command.program,
            args: command.args.into_iter().map(Into::into).collect(),
            cwd: context.workspace_root.clone(),
            env: Vec::new(),
            timeout: Duration::from_secs(30),
            cancellation: CancellationToken::new(),
        }
    }

    fn test_background_job(context: &WorkspaceContext, action: &str) -> IndexBackgroundJob {
        let lock = lock_path(context);
        fs::create_dir_all(lock.parent().unwrap()).unwrap();
        let lock_lease = acquire_index_lock(&lock, action, &context.workspace_root.join("src"))
            .unwrap()
            .expect("test background job should acquire lock");
        IndexBackgroundJob {
            action: action.to_string(),
            context: context.clone(),
            source_root: context.workspace_root.join("src"),
            root_capability: None,
            primary: inert_index_command(context, action),
            info: inert_index_command(context, "info"),
            status_path: status_path(context),
            lock_path: lock,
            lock_lease,
        }
    }

    fn print_lines_command(
        cwd: &Path,
        sleep_first: bool,
        lines: &[String],
        cancellation: CancellationToken,
    ) -> IndexCommand {
        let command = testing::line_printing_command(sleep_first, lines);
        let context = WorkspaceContext {
            cwd: cwd.to_path_buf(),
            workspace_root: cwd.to_path_buf(),
            cache_root: cwd.join(".build/unica"),
            workspace_epoch: 1,
        };
        IndexCommand {
            program: command.program,
            args: command.args.into_iter().map(Into::into).collect(),
            cwd: cwd.to_path_buf(),
            env: vec![(
                "RLM_INDEX_DIR".into(),
                rlm_generation_root(&context, &default_source_root(&context))
                    .unwrap()
                    .into_os_string(),
            )],
            // See `index_command`: the budget bounds a hung fixture, it does
            // not assert how quickly `powershell` starts.
            timeout: Duration::from_secs(30),
            cancellation,
        }
    }

    fn test_context(name: &str) -> WorkspaceContext {
        let root = std::env::temp_dir().join(format!("unica-index-{name}-{}", now_nanos()));
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("v8project.yaml"),
            "source-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        create_fake_plugin_root(&root);
        WorkspaceContext {
            cwd: root.clone(),
            workspace_root: root.clone(),
            cache_root: root.join(".build").join("unica"),
            workspace_epoch: 1,
        }
    }

    fn create_fake_plugin_root(root: &Path) {
        let plugin_root = root.join("plugins").join("unica");
        fs::create_dir_all(plugin_root.join("skills")).unwrap();
        fs::create_dir_all(plugin_root.join("third-party")).unwrap();
        for target in ["darwin-arm64", "linux-x64"] {
            fs::create_dir_all(plugin_root.join("bin").join(target)).unwrap();
            fs::write(
                plugin_root.join("bin").join(target).join("rlm-bsl-index"),
                "rlm-index",
            )
            .unwrap();
        }
        fs::create_dir_all(plugin_root.join("bin/win-x64")).unwrap();
        fs::write(
            plugin_root.join("bin/win-x64").join("rlm-bsl-index.exe"),
            "rlm-index",
        )
        .unwrap();
        fs::write(
            plugin_root.join("third-party/manifest.json"),
            r#"{
  "schemaVersion": 2,
  "tools": [
    {
      "name": "rlm-bsl-index",
      "binaries": {
        "darwin-arm64": {"targetTriple": "aarch64-apple-darwin", "binaryPath": "bin/darwin-arm64/rlm-bsl-index", "sha256": "fa6a77fa531fa57e7781010a7cec69b7be4b7b58903365153bf1f66e851ab213"},
        "linux-x64": {"targetTriple": "x86_64-unknown-linux-gnu", "binaryPath": "bin/linux-x64/rlm-bsl-index", "sha256": "fa6a77fa531fa57e7781010a7cec69b7be4b7b58903365153bf1f66e851ab213"},
        "win-x64": {"targetTriple": "x86_64-pc-windows-msvc", "binaryPath": "bin/win-x64/rlm-bsl-index.exe", "sha256": "fa6a77fa531fa57e7781010a7cec69b7be4b7b58903365153bf1f66e851ab213"}
      }
    }
  ]
}"#,
        )
        .unwrap();
    }

    fn write_stale_lock(context: &WorkspaceContext, action: &str) {
        fs::create_dir_all(lock_path(context).parent().unwrap()).unwrap();
        let mut lock = BslIndexLock::new(action, &context.workspace_root.join("src"));
        lock.started_at = now_secs().saturating_sub(LOCK_STALE_AFTER.as_secs() + 1);
        lock.updated_at = lock.started_at;
        write_lock_path(&lock_path(context), lock).unwrap();
    }

    fn write_fresh_lock(context: &WorkspaceContext, action: &str) {
        fs::create_dir_all(lock_path(context).parent().unwrap()).unwrap();
        let lock = BslIndexLock::new(action, &context.workspace_root.join("src"));
        write_lock_path(&lock_path(context), lock).unwrap();
    }

    fn write_released_lock(context: &WorkspaceContext, action: &str) {
        fs::create_dir_all(lock_path(context).parent().unwrap()).unwrap();
        let now = now_secs();
        let text = serde_json::json!({
            "schema_version": LOCK_SCHEMA_VERSION,
            "lock_id": "released",
            "owner_pid": 999999,
            "action": action,
            "source_root": context.workspace_root.join("src").display().to_string(),
            "started_at": now,
            "updated_at": now,
            "state": "released",
            "released_at": now
        });
        fs::write(
            lock_path(context),
            serde_json::to_string_pretty(&text).unwrap() + "\n",
        )
        .unwrap();
    }

    fn write_old_building_status(context: &WorkspaceContext, action: &str) {
        let mut status =
            BslIndexStatus::building(action, Some(&context.workspace_root.join("src")));
        status.updated_at = now_secs().saturating_sub(LOCK_STALE_AFTER.as_secs() + 1);
        write_status(context, status).unwrap();
    }

    fn write_usable_build(context: &WorkspaceContext, build_id: &str) -> PathBuf {
        let source_root = default_source_root(context);
        let build_root = rlm_build_root(context, &source_root, build_id).unwrap();
        let db_path = build_root.join("bsl_index.db");
        fs::create_dir_all(&build_root).unwrap();
        fs::write(&db_path, b"usable index").unwrap();
        write_status(
            context,
            BslIndexStatus::ready(&source_root, &db_path).with_build_id(build_id.to_string()),
        )
        .unwrap();
        db_path
    }

    fn make_lock_file_old(context: &WorkspaceContext) {
        use std::fs::FileTimes;

        const JANUARY_1_2000_UTC: Duration = Duration::from_secs(946_684_800);
        let file = OpenOptions::new()
            .write(true)
            .open(lock_path(context))
            .unwrap();
        file.set_times(FileTimes::new().set_modified(UNIX_EPOCH + JANUARY_1_2000_UTC))
            .unwrap();
    }

    #[test]
    fn isolated_build_keeps_last_usable_index_until_candidate_is_ready() {
        let context = test_context("isolated-build-active-cutover");
        let source_root = default_source_root(&context);
        fs::create_dir_all(&source_root).unwrap();
        let old_id = "1".repeat(32);
        let old_directory = rlm_build_root(&context, &source_root, &old_id).unwrap();
        let old_db = old_directory.join("bsl_index.db");
        fs::create_dir_all(&old_directory).unwrap();
        fs::write(&old_db, b"old usable database").unwrap();
        write_status(
            &context,
            BslIndexStatus::ready(&source_root, &old_db).with_build_id(old_id.clone()),
        )
        .unwrap();
        let old_marker = fs::read(status_path(&context)).unwrap();
        let runner = RecordingIndexRunner::default();
        let service = WorkspaceIndexService::with_runner(&runner);

        let report =
            service.start_isolated_build(&context, &source_root, &CancellationToken::new());
        assert_eq!(report.maintenance, IndexMaintenanceStatus::Queued);
        let failed_job = runner.backgrounds.borrow_mut().pop().unwrap();
        let failed_directory = PathBuf::from(index_command_env(&failed_job.info, "RLM_INDEX_DIR"));
        assert_ne!(failed_directory, old_directory);
        assert_eq!(fs::read(status_path(&context)).unwrap(), old_marker);
        assert_eq!(
            service
                .active_build(&context, &source_root)
                .unwrap()
                .unwrap()
                .id,
            old_id
        );
        run_background_job_with(failed_job, |_command, _lease| {
            Err("candidate build failed".to_string())
        });
        assert_eq!(fs::read(status_path(&context)).unwrap(), old_marker);
        assert_eq!(
            service
                .active_build(&context, &source_root)
                .unwrap()
                .unwrap()
                .db_path,
            old_db
        );

        let report =
            service.start_isolated_build(&context, &source_root, &CancellationToken::new());
        assert_eq!(report.maintenance, IndexMaintenanceStatus::Queued);
        let ready_job = runner.backgrounds.borrow_mut().pop().unwrap();
        let candidate_directory =
            PathBuf::from(index_command_env(&ready_job.info, "RLM_INDEX_DIR"));
        let candidate_id = candidate_directory
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert_ne!(candidate_directory, failed_directory);
        let candidate_db = candidate_directory.join("bsl_index.db");
        fs::create_dir_all(&candidate_directory).unwrap();
        fs::write(&candidate_db, b"new usable database").unwrap();
        let source_file = source_root.join("Module.bsl");
        run_background_job_with(ready_job, |command, _lease| {
            if command.args.get(1).is_some_and(|verb| verb == "build") {
                fs::write(&source_file, b"source edited while building").unwrap();
                Ok(IndexOutput::success("Index built"))
            } else {
                Ok(IndexOutput::success(format!(
                    "Index: {}\n  Status: fresh\n",
                    candidate_db.display()
                )))
            }
        });

        let active = service
            .active_build(&context, &source_root)
            .unwrap()
            .unwrap();
        assert_eq!(active.id, candidate_id);
        assert_eq!(active.db_path, candidate_db);
        assert_eq!(fs::read(&old_db).unwrap(), b"old usable database");
        let marker = read_bsl_index_status(&context).unwrap();
        assert_eq!(marker.build_id.as_deref(), Some(candidate_id.as_str()));
        cleanup(&context);
    }

    #[test]
    fn legacy_ready_marker_requires_an_isolated_build_before_becoming_active() {
        let context = test_context("legacy-ready-isolated-migration");
        let source_root = default_source_root(&context);
        fs::create_dir_all(&source_root).unwrap();
        let legacy_directory = rlm_generation_root(&context, &source_root).unwrap();
        let legacy_db = legacy_directory.join("bsl_index.db");
        fs::create_dir_all(&legacy_directory).unwrap();
        fs::write(&legacy_db, b"legacy mutable database").unwrap();
        // Older ready markers have no build_id and point into the shared
        // generation directory, which an older builder may still overwrite.
        write_status(&context, BslIndexStatus::ready(&source_root, &legacy_db)).unwrap();
        let legacy_marker = fs::read(status_path(&context)).unwrap();
        let runner = RecordingIndexRunner::default();
        let service = WorkspaceIndexService::with_runner(&runner);
        assert_eq!(service.active_build(&context, &source_root).unwrap(), None);
        assert!(!bsl_index_is_ready(&context));

        let report =
            service.start_isolated_build(&context, &source_root, &CancellationToken::new());
        assert_eq!(report.maintenance, IndexMaintenanceStatus::Queued);
        let job = runner.backgrounds.borrow_mut().pop().unwrap();
        let candidate_directory = PathBuf::from(index_command_env(&job.info, "RLM_INDEX_DIR"));
        let candidate_id = isolated_build_id(&job.info).unwrap();
        assert_eq!(
            candidate_directory,
            legacy_directory.join("builds").join(&candidate_id)
        );
        assert_eq!(
            PathBuf::from(index_command_env(&job.primary, "RLM_INDEX_DIR")),
            candidate_directory
        );
        assert_eq!(fs::read(status_path(&context)).unwrap(), legacy_marker);
        assert_eq!(service.active_build(&context, &source_root).unwrap(), None);
        let candidate_db = candidate_directory.join("bsl_index.db");
        run_background_job_with(job, |command, _lease| {
            if command.args.get(1).is_some_and(|verb| verb == "build") {
                fs::create_dir_all(&candidate_directory).unwrap();
                fs::write(&candidate_db, b"isolated database").unwrap();
                Ok(IndexOutput::success("Index built"))
            } else {
                Ok(IndexOutput::success(format!(
                    "Index: {}\n  Status: fresh\n",
                    candidate_db.display()
                )))
            }
        });
        let active = service
            .active_build(&context, &source_root)
            .unwrap()
            .unwrap();
        assert_eq!(active.id, candidate_id);
        assert_eq!(active.directory, candidate_directory);
        assert_eq!(active.db_path, candidate_db);
        assert!(bsl_index_is_ready(&context));
        assert_eq!(fs::read(&legacy_db).unwrap(), b"legacy mutable database");
        fs::write(&legacy_db, b"rewritten by legacy builder").unwrap();
        assert_eq!(fs::read(&active.db_path).unwrap(), b"isolated database");
        assert_eq!(
            service.active_build(&context, &source_root).unwrap(),
            Some(active)
        );
        cleanup(&context);
    }

    #[test]
    fn active_build_rejects_foreign_root_and_invalid_build_route() {
        let context = test_context("active-build-route");
        let source_root = default_source_root(&context);
        fs::create_dir_all(&source_root).unwrap();
        let build_id = "2".repeat(32);
        let directory = rlm_build_root(&context, &source_root, &build_id).unwrap();
        let db_path = directory.join("bsl_index.db");
        fs::create_dir_all(&directory).unwrap();
        fs::write(&db_path, b"usable").unwrap();
        write_status(
            &context,
            BslIndexStatus::ready(&context.workspace_root.join("other"), &db_path)
                .with_build_id(build_id.clone()),
        )
        .unwrap();
        assert_eq!(usable_index_build(&context, &source_root).unwrap(), None);

        write_status(
            &context,
            BslIndexStatus::ready(&source_root, &db_path).with_build_id("../outside".to_string()),
        )
        .unwrap();
        assert!(usable_index_build(&context, &source_root)
            .unwrap_err()
            .contains("invalid RLM index build identifier"));
        cleanup(&context);
    }

    #[test]
    fn candidate_symlink_route_never_replaces_last_usable_marker() {
        let context = test_context("candidate-symlink-route");
        let source_root = default_source_root(&context);
        fs::create_dir_all(&source_root).unwrap();
        let old_id = "3".repeat(32);
        let old_directory = rlm_build_root(&context, &source_root, &old_id).unwrap();
        let old_db = old_directory.join("bsl_index.db");
        fs::create_dir_all(&old_directory).unwrap();
        fs::write(&old_db, b"old usable database").unwrap();
        write_status(
            &context,
            BslIndexStatus::ready(&source_root, &old_db).with_build_id(old_id.clone()),
        )
        .unwrap();
        let old_marker = fs::read(status_path(&context)).unwrap();
        let runner = RecordingIndexRunner::default();
        let service = WorkspaceIndexService::with_runner(&runner);
        service.start_isolated_build(&context, &source_root, &CancellationToken::new());
        let job = runner.backgrounds.borrow_mut().pop().unwrap();
        let candidate_directory = PathBuf::from(index_command_env(&job.info, "RLM_INDEX_DIR"));
        let outside = context.workspace_root.join("foreign-db");
        fs::create_dir_all(&candidate_directory).unwrap();
        fs::create_dir_all(&outside).unwrap();
        let Some(link) =
            testing::create_dir_symlink_for_test(&outside, candidate_directory.join("sub"))
        else {
            cleanup(&context);
            return;
        };
        link.unwrap();
        fs::write(outside.join("index.db"), b"foreign DB").unwrap();
        let escaped_db = candidate_directory.join("sub/index.db");
        run_background_job_with(job, |command, _lease| {
            if command.args.get(1).is_some_and(|verb| verb == "build") {
                Ok(IndexOutput::success("Index built"))
            } else {
                Ok(IndexOutput::success(format!(
                    "Index: {}\n  Status: fresh\n",
                    escaped_db.display()
                )))
            }
        });
        assert_eq!(fs::read(status_path(&context)).unwrap(), old_marker);
        assert_eq!(
            service
                .active_build(&context, &source_root)
                .unwrap()
                .unwrap()
                .id,
            old_id
        );

        service.start_isolated_build(&context, &source_root, &CancellationToken::new());
        let job = runner.backgrounds.borrow_mut().pop().unwrap();
        let replaced_directory = PathBuf::from(index_command_env(&job.info, "RLM_INDEX_DIR"));
        fs::create_dir_all(replaced_directory.parent().unwrap()).unwrap();
        testing::create_dir_symlink_for_test(&outside, &replaced_directory)
            .expect("symlinks were supported for the first candidate")
            .unwrap();
        let escaped_db = replaced_directory.join("index.db");
        run_background_job_with(job, |command, _lease| {
            if command.args.get(1).is_some_and(|verb| verb == "build") {
                Ok(IndexOutput::success("Index built"))
            } else {
                Ok(IndexOutput::success(format!(
                    "Index: {}\n  Status: fresh\n",
                    escaped_db.display()
                )))
            }
        });
        assert_eq!(fs::read(status_path(&context)).unwrap(), old_marker);
        assert_eq!(
            service
                .active_build(&context, &source_root)
                .unwrap()
                .unwrap()
                .id,
            old_id
        );
        cleanup(&context);
    }

    fn cleanup(context: &WorkspaceContext) {
        let _ = fs::remove_dir_all(&context.workspace_root);
    }
}
