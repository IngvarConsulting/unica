use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::bundled_tools::resolve_bundled_tool;
use crate::infrastructure::plugin_runtime::find_plugin_root;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const INDEX_TIMEOUT: Duration = Duration::from_secs(30);
const LOCK_STALE_AFTER: Duration = Duration::from_secs(10 * 60);
const LOCK_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
const LOCK_SCHEMA_VERSION: u32 = 1;
const RLM_INDEX_DIR_NAME: &str = "rlm-tools-bsl";
const STATUS_FILE_NAME: &str = "bsl_index_status.json";
const LOCK_FILE_NAME: &str = "bsl_index.lock";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexReadiness {
    Ready { db_path: PathBuf },
    Missing,
    Stale,
    Building,
    Failed(String),
    Unavailable(String),
}

#[derive(Debug, Clone, Default)]
pub struct IndexStartReport {
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BslIndexStatus {
    pub status: String,
    pub source_root: Option<String>,
    pub db_path: Option<String>,
    pub message: Option<String>,
    pub updated_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_run: Option<BslIndexRunMetrics>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BslIndexRunMetrics {
    pub action: String,
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
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
    pub timeout: Duration,
}

#[derive(Debug, Clone)]
pub struct IndexOutput {
    pub status_success: bool,
    pub status: String,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
    pub duration_ms: u64,
}

#[derive(Debug)]
pub struct IndexBackgroundJob {
    pub action: String,
    pub source_root: PathBuf,
    pub primary: IndexCommand,
    pub info: IndexCommand,
    pub status_path: PathBuf,
    #[cfg(test)]
    pub lock_path: PathBuf,
    pub lock_lease: IndexLockLease,
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
    fn run(&self, command: &IndexCommand) -> Result<IndexOutput, String>;

    fn start_background(&self, job: IndexBackgroundJob) -> Result<(), String>;
}

pub struct SystemIndexRunner;

pub static SYSTEM_INDEX_RUNNER: SystemIndexRunner = SystemIndexRunner;

pub struct WorkspaceIndexService<'a> {
    runner: &'a dyn IndexRunner,
}

impl<'a> WorkspaceIndexService<'a> {
    pub fn new() -> Self {
        Self {
            runner: &SYSTEM_INDEX_RUNNER,
        }
    }

    pub fn with_runner(runner: &'a dyn IndexRunner) -> Self {
        Self { runner }
    }

    pub fn start_for_workspace(
        &self,
        context: &WorkspaceContext,
        args: &Map<String, Value>,
        dry_run: bool,
    ) -> IndexStartReport {
        if dry_run {
            return IndexStartReport::default();
        }

        let Some(source_root) = resolve_source_root(context, args) else {
            let _ = write_status(
                context,
                BslIndexStatus::unavailable("could not resolve 1C source root", None),
            );
            return IndexStartReport::default();
        };

        if active_lock(context, &source_root) {
            return IndexStartReport {
                warnings: vec!["rlm index building".to_string()],
            };
        }

        let commands = match self.commands(context, &source_root) {
            Ok(commands) => commands,
            Err(error) => {
                let _ = write_status(
                    context,
                    BslIndexStatus::unavailable(error.as_str(), Some(&source_root)),
                );
                return IndexStartReport::default();
            }
        };

        let info = match self.runner.run(&commands.info) {
            Ok(output) => output,
            Err(error) => {
                let _ = write_status(
                    context,
                    BslIndexStatus::unavailable(error.as_str(), Some(&source_root)),
                );
                return IndexStartReport::default();
            }
        };

        let readiness = readiness_from_info(&info);
        match readiness {
            IndexReadiness::Ready { db_path } => {
                let _ = write_status(
                    context,
                    ready_status_preserving_last_run(context, &source_root, &db_path),
                );
                IndexStartReport::default()
            }
            IndexReadiness::Missing => self.start_background(
                context,
                "build",
                source_root,
                commands.build,
                commands.info,
                "rlm index build started",
            ),
            IndexReadiness::Stale => self.start_background(
                context,
                "update",
                source_root,
                commands.update,
                commands.info,
                "rlm index building",
            ),
            IndexReadiness::Building => IndexStartReport {
                warnings: vec!["rlm index building".to_string()],
            },
            IndexReadiness::Failed(message) | IndexReadiness::Unavailable(message) => {
                let _ = write_status(
                    context,
                    BslIndexStatus::unavailable(message.as_str(), Some(&source_root)),
                );
                IndexStartReport::default()
            }
        }
    }

    pub fn ready_index(
        &self,
        context: &WorkspaceContext,
        args: &Map<String, Value>,
    ) -> IndexReadiness {
        let Some(source_root) = resolve_source_root(context, args) else {
            return IndexReadiness::Unavailable("could not resolve 1C source root".to_string());
        };

        if active_lock(context, &source_root) {
            return IndexReadiness::Building;
        }

        let commands = match self.commands(context, &source_root) {
            Ok(commands) => commands,
            Err(error) => return IndexReadiness::Unavailable(error),
        };

        let output = match self.runner.run(&commands.info) {
            Ok(output) => output,
            Err(error) => return IndexReadiness::Unavailable(error),
        };

        match readiness_from_info(&output) {
            IndexReadiness::Ready { db_path } => {
                let _ = write_status(
                    context,
                    ready_status_preserving_last_run(context, &source_root, &db_path),
                );
                IndexReadiness::Ready { db_path }
            }
            other => other,
        }
    }

    fn commands(
        &self,
        context: &WorkspaceContext,
        source_root: &Path,
    ) -> Result<IndexCommands, String> {
        let plugin_root = find_plugin_root(&context.cwd).ok_or_else(|| {
            "could not locate Unica plugin root for internal RLM index adapter lookup".to_string()
        })?;
        let program = resolve_bundled_tool(&plugin_root, "rlm-bsl-index", true)?.program;
        let env = vec![(
            "RLM_INDEX_DIR".to_string(),
            context
                .cache_root
                .join(RLM_INDEX_DIR_NAME)
                .display()
                .to_string(),
        )];
        let root = source_root.display().to_string();
        Ok(IndexCommands {
            info: IndexCommand {
                program: program.clone(),
                args: vec!["index".to_string(), "info".to_string(), root.clone()],
                cwd: context.cwd.clone(),
                env: env.clone(),
                timeout: INDEX_TIMEOUT,
            },
            build: IndexCommand {
                program: program.clone(),
                args: vec!["index".to_string(), "build".to_string(), root.clone()],
                cwd: context.cwd.clone(),
                env: env.clone(),
                timeout: Duration::from_secs(24 * 60 * 60),
            },
            update: IndexCommand {
                program,
                args: vec!["index".to_string(), "update".to_string(), root],
                cwd: context.cwd.clone(),
                env,
                timeout: Duration::from_secs(24 * 60 * 60),
            },
        })
    }

    fn start_background(
        &self,
        context: &WorkspaceContext,
        action: &str,
        source_root: PathBuf,
        primary: IndexCommand,
        info: IndexCommand,
        warning: &str,
    ) -> IndexStartReport {
        let lock = lock_path(context);
        if let Some(parent) = lock.parent() {
            if let Err(error) = fs::create_dir_all(parent) {
                let message = format!("failed to create RLM index lock directory: {error}");
                let _ = write_status(
                    context,
                    BslIndexStatus::failed(message.as_str(), Some(&source_root)),
                );
                return IndexStartReport::default();
            }
        }

        let lock_lease = match acquire_index_lock(&lock, action, &source_root) {
            Ok(Some(lock_lease)) => lock_lease,
            Ok(None) => {
                return IndexStartReport {
                    warnings: vec!["rlm index building".to_string()],
                };
            }
            Err(error) => {
                let _ = write_status(
                    context,
                    BslIndexStatus::failed(error.as_str(), Some(&source_root)),
                );
                return IndexStartReport::default();
            }
        };
        let status_path = status_path(context);
        let _ = write_status_path(
            &status_path,
            BslIndexStatus::building(action, Some(&source_root)),
        );

        let job = IndexBackgroundJob {
            action: action.to_string(),
            source_root,
            primary,
            info,
            status_path,
            #[cfg(test)]
            lock_path: lock.clone(),
            lock_lease,
        };
        if let Err(error) = self.runner.start_background(job) {
            let _ = write_status(context, BslIndexStatus::failed(error.as_str(), None));
            return IndexStartReport::default();
        }

        IndexStartReport {
            warnings: vec![warning.to_string()],
        }
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
    update: IndexCommand,
}

impl BslIndexStatus {
    fn ready(source_root: &Path, db_path: &Path) -> Self {
        Self {
            status: "ready".to_string(),
            source_root: Some(source_root.display().to_string()),
            db_path: Some(db_path.display().to_string()),
            message: None,
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
            updated_at: now_secs(),
            last_run: None,
        }
    }

    fn unavailable(message: &str, source_root: Option<&Path>) -> Self {
        Self {
            status: "unavailable".to_string(),
            source_root: source_root.map(|path| path.display().to_string()),
            db_path: None,
            message: Some(message.to_string()),
            updated_at: now_secs(),
            last_run: None,
        }
    }

    fn with_last_run(mut self, metrics: BslIndexRunMetrics) -> Self {
        self.last_run = Some(metrics);
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
    file: File,
    lock: BslIndexLock,
    released: bool,
}

impl IndexLockLease {
    fn lock_id(&self) -> &str {
        self.lock.lock_id.as_str()
    }

    fn refresh(&mut self, child_pid: u32) {
        if !self.current_file_still_owned() {
            return;
        }
        self.lock.updated_at = now_secs();
        self.lock.child_pid = Some(child_pid);
        let _ = write_lock_file_to_open(&mut self.file, &self.lock);
    }

    fn release(&mut self) {
        if self.released {
            return;
        }
        let still_owned = self.current_file_still_owned();
        unregister_active_lock(&self.path, self.lock_id());
        if still_owned {
            self.lock.mark_released();
            let _ = write_lock_file_to_open(&mut self.file, &self.lock);
        }
        let _ = self.file.unlock();
        self.released = true;
    }

    fn current_file_still_owned(&self) -> bool {
        match read_lock_path(&self.path) {
            Ok(index_lock) => index_lock.lock_id == self.lock.lock_id,
            Err(_) => active_index_locks()
                .lock()
                .ok()
                .and_then(|locks| locks.get(&self.path).cloned())
                .map(|lock_id| lock_id == self.lock.lock_id)
                .unwrap_or(false),
        }
    }
}

impl Drop for IndexLockLease {
    fn drop(&mut self) {
        self.release();
    }
}

fn active_index_locks() -> &'static Mutex<HashMap<PathBuf, String>> {
    static ACTIVE_INDEX_LOCKS: OnceLock<Mutex<HashMap<PathBuf, String>>> = OnceLock::new();
    ACTIVE_INDEX_LOCKS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn register_active_lock(path: &Path, lock_id: &str) {
    if let Ok(mut locks) = active_index_locks().lock() {
        locks.insert(path.to_path_buf(), lock_id.to_string());
    }
}

fn unregister_active_lock(path: &Path, lock_id: &str) {
    if let Ok(mut locks) = active_index_locks().lock() {
        if locks
            .get(path)
            .map(|current| current == lock_id)
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
    fn run(&self, command: &IndexCommand) -> Result<IndexOutput, String> {
        run_index_command(command)
    }

    fn start_background(&self, job: IndexBackgroundJob) -> Result<(), String> {
        thread::Builder::new()
            .name("unica-rlm-index".to_string())
            .spawn(move || run_background_job(job))
            .map(|_| ())
            .map_err(|error| format!("failed to start RLM index background worker: {error}"))
    }
}

fn run_background_job(mut job: IndexBackgroundJob) {
    let started_at = now_secs();
    let result = run_index_command_with_heartbeat(&job.primary, Some(&mut job.lock_lease));
    let finished_at = now_secs();
    match result {
        Ok(output) if output.status_success => {
            let metrics =
                BslIndexRunMetrics::from_output(&job.action, started_at, finished_at, &output);
            match run_index_command(&job.info) {
                Ok(info) => match readiness_from_info(&info) {
                    IndexReadiness::Ready { db_path } => {
                        let _ = write_status_path(
                            &job.status_path,
                            BslIndexStatus::ready(&job.source_root, &db_path)
                                .with_last_run(metrics),
                        );
                    }
                    other => {
                        let _ = write_status_path(
                            &job.status_path,
                            BslIndexStatus::failed(
                                format!("rlm index {} finished but info is {other:?}", job.action)
                                    .as_str(),
                                Some(&job.source_root),
                            )
                            .with_last_run(metrics),
                        );
                    }
                },
                Err(error) => {
                    let _ = write_status_path(
                        &job.status_path,
                        BslIndexStatus::failed(error.as_str(), Some(&job.source_root))
                            .with_last_run(metrics),
                    );
                }
            }
        }
        Ok(output) => {
            let metrics =
                BslIndexRunMetrics::from_output(&job.action, started_at, finished_at, &output);
            let message = if output.timed_out {
                format!("rlm index {} timed out", job.action)
            } else {
                format!(
                    "rlm index {} failed: {} {}",
                    job.action,
                    output.status,
                    output.stderr.trim()
                )
            };
            let _ = write_status_path(
                &job.status_path,
                BslIndexStatus::failed(message.as_str(), Some(&job.source_root))
                    .with_last_run(metrics),
            );
        }
        Err(error) => {
            let _ = write_status_path(
                &job.status_path,
                BslIndexStatus::failed(error.as_str(), Some(&job.source_root)),
            );
        }
    }
}

fn run_index_command(command: &IndexCommand) -> Result<IndexOutput, String> {
    run_index_command_with_heartbeat(command, None)
}

fn run_index_command_with_heartbeat(
    command: &IndexCommand,
    mut heartbeat: Option<&mut IndexLockLease>,
) -> Result<IndexOutput, String> {
    let mut child = Command::new(&command.program)
        .args(&command.args)
        .current_dir(&command.cwd)
        .envs(command.env.iter().map(|(key, value)| (key, value)))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("failed to execute RLM index process: {error}"))?;

    let started = Instant::now();
    let mut last_heartbeat = Instant::now();
    if let Some(lease) = heartbeat.as_mut() {
        (*lease).refresh(child.id());
    }
    loop {
        if child
            .try_wait()
            .map_err(|error| format!("failed to poll RLM index process: {error}"))?
            .is_some()
        {
            let output = child
                .wait_with_output()
                .map_err(|error| format!("failed to collect RLM index output: {error}"))?;
            return Ok(IndexOutput {
                status_success: output.status.success(),
                status: output.status.to_string(),
                stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                timed_out: false,
                duration_ms: duration_ms(started.elapsed()),
            });
        }

        if let Some(lease) = heartbeat.as_mut() {
            if last_heartbeat.elapsed() >= LOCK_HEARTBEAT_INTERVAL {
                (*lease).refresh(child.id());
                last_heartbeat = Instant::now();
            }
        }

        if started.elapsed() >= command.timeout {
            let _ = child.kill();
            let output = child.wait_with_output().map_err(|error| {
                format!("failed to collect timed-out RLM index output: {error}")
            })?;
            return Ok(IndexOutput {
                status_success: false,
                status: "timeout".to_string(),
                stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                timed_out: true,
                duration_ms: duration_ms(started.elapsed()),
            });
        }

        thread::sleep(Duration::from_millis(50));
    }
}

fn readiness_from_info(output: &IndexOutput) -> IndexReadiness {
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
        Some(value) if value.starts_with("stale") => IndexReadiness::Stale,
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

pub fn read_bsl_index_status(context: &WorkspaceContext) -> Option<BslIndexStatus> {
    let text = fs::read_to_string(status_path(context)).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn bsl_index_is_ready(context: &WorkspaceContext) -> bool {
    let Some(status) = read_bsl_index_status(context) else {
        return false;
    };
    if status.status != "ready" {
        return false;
    }
    match status.db_path {
        Some(db_path) => Path::new(&db_path).is_file(),
        None => false,
    }
}

pub fn status_path(context: &WorkspaceContext) -> PathBuf {
    context.cache_root.join("caches").join(STATUS_FILE_NAME)
}

fn lock_path(context: &WorkspaceContext) -> PathBuf {
    context.cache_root.join("locks").join(LOCK_FILE_NAME)
}

fn active_lock(context: &WorkspaceContext, source_root: &Path) -> bool {
    let lock = lock_path(context);
    if !lock.is_file() {
        return false;
    }
    if active_lock_registered(&lock) {
        return true;
    }
    match read_lock_path(&lock) {
        Ok(index_lock) if !index_lock.is_active() => false,
        Ok(index_lock) if index_lock.is_fresh() => true,
        Ok(index_lock) => {
            if lock_is_held_by_other_process(&lock) {
                return true;
            }
            !recover_stale_lock(
                context,
                source_root,
                format!(
                    "RLM index {action} lock is stale",
                    action = index_lock.action
                )
                .as_str(),
                Some(index_lock.lock_id.as_str()),
            )
        }
        Err(error) => {
            if invalid_lock_may_be_active(context, &lock) {
                return true;
            }
            !recover_stale_lock(
                context,
                source_root,
                format!("RLM index lock is invalid: {error}").as_str(),
                None,
            )
        }
    }
}

fn invalid_lock_may_be_active(context: &WorkspaceContext, lock: &Path) -> bool {
    if active_lock_registered(lock) || lock_is_held_by_other_process(lock) {
        return true;
    }
    let lock_updated_at = file_modified_secs(lock).unwrap_or_else(now_secs);
    if now_secs().saturating_sub(lock_updated_at) <= LOCK_STALE_AFTER.as_secs() {
        return true;
    }
    if let Some(status) = read_bsl_index_status(context) {
        if status.status == "building" {
            return now_secs().saturating_sub(status.updated_at) <= LOCK_STALE_AFTER.as_secs();
        }
    }
    false
}

fn recover_stale_lock(
    context: &WorkspaceContext,
    source_root: &Path,
    reason: &str,
    lock_id: Option<&str>,
) -> bool {
    let lock = lock_path(context);
    if !mark_lock_recovered(&lock, lock_id, source_root, reason) {
        return false;
    }
    if read_bsl_index_status(context)
        .map(|status| status.status == "building")
        .unwrap_or(false)
    {
        let _ = write_status(
            context,
            BslIndexStatus::failed(
                format!("stale RLM index build marker recovered: {reason}").as_str(),
                Some(source_root),
            ),
        );
    }
    true
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
    register_active_lock(path, index_lock.lock_id.as_str());
    Ok(Some(IndexLockLease {
        path: path.to_path_buf(),
        file,
        lock: index_lock,
        released: false,
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

fn write_status(context: &WorkspaceContext, status: BslIndexStatus) -> Result<(), String> {
    write_status_path(&status_path(context), status)
}

fn ready_status_preserving_last_run(
    context: &WorkspaceContext,
    source_root: &Path,
    db_path: &Path,
) -> BslIndexStatus {
    let mut status = BslIndexStatus::ready(source_root, db_path);
    let source_root_display = source_root.display().to_string();
    let db_path_display = db_path.display().to_string();
    status.last_run = read_bsl_index_status(context).and_then(|existing| {
        let same_index = existing.source_root.as_deref() == Some(source_root_display.as_str())
            && existing.db_path.as_deref() == Some(db_path_display.as_str());
        if same_index {
            existing.last_run
        } else {
            None
        }
    });
    status
}

fn write_status_path(path: &Path, status: BslIndexStatus) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create Unica cache status directory: {error}"))?;
    }
    let text = serde_json::to_string_pretty(&status).map_err(|error| error.to_string())?;
    fs::write(path, text + "\n")
        .map_err(|error| format!("failed to write RLM index status: {error}"))
}

fn resolve_source_root(context: &WorkspaceContext, args: &Map<String, Value>) -> Option<PathBuf> {
    for key in ["sourceDir", "path"] {
        if let Some(value) = args.get(key).and_then(Value::as_str) {
            let candidate = resolve_path(&context.cwd, value);
            if looks_like_1c_source_root(&candidate) {
                return Some(candidate);
            }
        }
    }

    [
        context.workspace_root.join("src"),
        context.workspace_root.join("src").join("cf"),
        context.workspace_root.clone(),
    ]
    .into_iter()
    .find(|candidate| looks_like_1c_source_root(candidate))
}

fn resolve_path(cwd: &Path, value: &str) -> PathBuf {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    }
}

fn looks_like_1c_source_root(path: &Path) -> bool {
    if !path.is_dir() {
        return false;
    }
    let source_dirs = [
        "CommonModules",
        "Catalogs",
        "Documents",
        "DataProcessors",
        "Reports",
        "InformationRegisters",
        "AccumulationRegisters",
    ];
    path.join("Configuration.xml").is_file()
        || source_dirs.iter().any(|name| path.join(name).is_dir())
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
    use std::cell::RefCell;

    #[test]
    fn dry_run_does_not_start_indexing_or_write_state() {
        let context = test_context("dry-run");
        fs::create_dir_all(context.workspace_root.join("src/CommonModules")).unwrap();
        let runner = RecordingIndexRunner::default();
        let service = WorkspaceIndexService::with_runner(&runner);

        let report = service.start_for_workspace(&context, &Map::new(), true);

        assert!(report.warnings.is_empty());
        assert!(runner.commands.borrow().is_empty());
        assert!(!status_path(&context).exists());
        cleanup(&context);
    }

    #[test]
    fn first_non_dry_run_starts_background_build_when_index_is_missing() {
        let context = test_context("missing");
        fs::create_dir_all(context.workspace_root.join("src/CommonModules")).unwrap();
        let runner = RecordingIndexRunner {
            outputs: RefCell::new(vec![IndexOutput::success(
                "Index not found: /tmp/bsl_index.db",
            )]),
            ..Default::default()
        };
        let service = WorkspaceIndexService::with_runner(&runner);

        let report = service.start_for_workspace(&context, &Map::new(), false);

        assert_eq!(report.warnings, vec!["rlm index build started".to_string()]);
        assert_eq!(runner.commands.borrow()[0].args[0..2], ["index", "info"]);
        let backgrounds = runner.backgrounds.borrow();
        assert_eq!(backgrounds[0].primary.args[0..2], ["index", "build"]);
        assert_eq!(backgrounds[0].primary.env[0].0, "RLM_INDEX_DIR");
        assert_eq!(
            PathBuf::from(&backgrounds[0].primary.env[0].1),
            context.cache_root.join(RLM_INDEX_DIR_NAME)
        );
        assert!(status_path(&context).is_file());
        cleanup(&context);
    }

    #[test]
    fn repeated_detect_does_not_start_duplicate_indexing_while_lock_exists() {
        let context = test_context("lock");
        fs::create_dir_all(context.workspace_root.join("src/CommonModules")).unwrap();
        write_fresh_lock(&context, "build");
        let runner = RecordingIndexRunner::default();
        let service = WorkspaceIndexService::with_runner(&runner);

        let report = service.start_for_workspace(&context, &Map::new(), false);

        assert_eq!(report.warnings, vec!["rlm index building".to_string()]);
        assert!(runner.commands.borrow().is_empty());
        assert!(runner.backgrounds.borrow().is_empty());
        cleanup(&context);
    }

    #[test]
    fn stale_legacy_lock_is_recovered_and_starts_missing_index_build() {
        let context = test_context("stale-legacy-lock");
        fs::create_dir_all(context.workspace_root.join("src/CommonModules")).unwrap();
        fs::create_dir_all(lock_path(&context).parent().unwrap()).unwrap();
        fs::write(lock_path(&context), "").unwrap();
        write_old_building_status(&context, "build");
        make_lock_file_old(&context);
        let runner = RecordingIndexRunner {
            outputs: RefCell::new(vec![IndexOutput::success(
                "Index not found: /tmp/bsl_index.db",
            )]),
            ..Default::default()
        };
        let service = WorkspaceIndexService::with_runner(&runner);

        let report = service.start_for_workspace(&context, &Map::new(), false);

        assert_eq!(report.warnings, vec!["rlm index build started".to_string()]);
        assert_eq!(runner.commands.borrow()[0].args[0..2], ["index", "info"]);
        assert_eq!(
            runner.backgrounds.borrow()[0].primary.args[0..2],
            ["index", "build"]
        );
        cleanup(&context);
    }

    #[test]
    fn invalid_lock_without_building_status_is_treated_as_active() {
        let context = test_context("invalid-lock-active");
        fs::create_dir_all(context.workspace_root.join("src/CommonModules")).unwrap();
        fs::create_dir_all(lock_path(&context).parent().unwrap()).unwrap();
        fs::write(lock_path(&context), "").unwrap();
        let runner = RecordingIndexRunner::default();
        let service = WorkspaceIndexService::with_runner(&runner);

        let report = service.start_for_workspace(&context, &Map::new(), false);

        assert_eq!(report.warnings, vec!["rlm index building".to_string()]);
        assert!(runner.commands.borrow().is_empty());
        assert!(runner.backgrounds.borrow().is_empty());
        cleanup(&context);
    }

    #[test]
    fn fresh_invalid_lock_with_stale_status_is_treated_as_active() {
        let context = test_context("invalid-lock-with-stale-status");
        fs::create_dir_all(context.workspace_root.join("src/CommonModules")).unwrap();
        fs::create_dir_all(lock_path(&context).parent().unwrap()).unwrap();
        fs::write(lock_path(&context), "").unwrap();
        write_old_building_status(&context, "build");
        let runner = RecordingIndexRunner::default();
        let service = WorkspaceIndexService::with_runner(&runner);

        let report = service.start_for_workspace(&context, &Map::new(), false);

        assert_eq!(report.warnings, vec!["rlm index building".to_string()]);
        assert!(runner.commands.borrow().is_empty());
        assert!(runner.backgrounds.borrow().is_empty());
        cleanup(&context);
    }

    #[test]
    fn stale_structured_lock_is_recovered_and_starts_missing_index_build() {
        let context = test_context("stale-structured-lock");
        fs::create_dir_all(context.workspace_root.join("src/CommonModules")).unwrap();
        write_stale_lock(&context, "build");
        write_old_building_status(&context, "build");
        let runner = RecordingIndexRunner {
            outputs: RefCell::new(vec![IndexOutput::success(
                "Index not found: /tmp/bsl_index.db",
            )]),
            ..Default::default()
        };
        let service = WorkspaceIndexService::with_runner(&runner);

        let report = service.start_for_workspace(&context, &Map::new(), false);

        assert_eq!(report.warnings, vec!["rlm index build started".to_string()]);
        assert_eq!(runner.commands.borrow()[0].args[0..2], ["index", "info"]);
        assert_eq!(
            runner.backgrounds.borrow()[0].primary.args[0..2],
            ["index", "build"]
        );
        cleanup(&context);
    }

    #[test]
    fn ready_index_recovers_stale_lock_and_reads_fresh_info() {
        let context = test_context("stale-lock-ready");
        fs::create_dir_all(context.workspace_root.join("src/CommonModules")).unwrap();
        fs::create_dir_all(lock_path(&context).parent().unwrap()).unwrap();
        fs::write(lock_path(&context), "").unwrap();
        write_old_building_status(&context, "build");
        make_lock_file_old(&context);
        let db_path = context.cache_root.join("rlm-tools-bsl/a/bsl_index.db");
        fs::create_dir_all(db_path.parent().unwrap()).unwrap();
        fs::write(&db_path, "").unwrap();
        let runner = RecordingIndexRunner {
            outputs: RefCell::new(vec![IndexOutput::success(format!(
                "Index: {}\n  Status:   fresh\n",
                db_path.display()
            ))]),
            ..Default::default()
        };
        let service = WorkspaceIndexService::with_runner(&runner);

        let readiness = service.ready_index(&context, &Map::new());

        assert_eq!(readiness, IndexReadiness::Ready { db_path });
        assert_eq!(runner.commands.borrow()[0].args[0..2], ["index", "info"]);
        cleanup(&context);
    }

    #[test]
    fn ready_info_writes_ready_status_and_does_not_start_background_job() {
        let context = test_context("ready");
        fs::create_dir_all(context.workspace_root.join("src/CommonModules")).unwrap();
        let db_path = context.cache_root.join("rlm-tools-bsl/a/bsl_index.db");
        fs::create_dir_all(db_path.parent().unwrap()).unwrap();
        fs::write(&db_path, "").unwrap();
        let runner = RecordingIndexRunner {
            outputs: RefCell::new(vec![IndexOutput::success(format!(
                "Index: {}\n  Status:   fresh\n",
                db_path.display()
            ))]),
            ..Default::default()
        };
        let service = WorkspaceIndexService::with_runner(&runner);

        let report = service.start_for_workspace(&context, &Map::new(), false);

        assert!(report.warnings.is_empty());
        assert!(runner.backgrounds.borrow().is_empty());
        assert!(bsl_index_is_ready(&context));
        cleanup(&context);
    }

    #[test]
    fn ready_info_preserves_existing_last_run_metrics() {
        let context = test_context("ready-metrics");
        fs::create_dir_all(context.workspace_root.join("src/CommonModules")).unwrap();
        let db_path = context.cache_root.join("rlm-tools-bsl/a/bsl_index.db");
        fs::create_dir_all(db_path.parent().unwrap()).unwrap();
        fs::write(&db_path, "").unwrap();
        write_status(
            &context,
            BslIndexStatus::ready(&context.workspace_root.join("src"), &db_path).with_last_run(
                BslIndexRunMetrics {
                    action: "build".to_string(),
                    duration_ms: 1234,
                    started_at: 10,
                    finished_at: 11,
                    timed_out: false,
                    index_version: Some("v14".to_string()),
                    modules: Some(24),
                    methods: Some(617),
                    db_size: Some("1.3 MB".to_string()),
                },
            ),
        )
        .unwrap();
        let runner = RecordingIndexRunner {
            outputs: RefCell::new(vec![IndexOutput::success(format!(
                "Index: {}\n  Status:   fresh\n",
                db_path.display()
            ))]),
            ..Default::default()
        };
        let service = WorkspaceIndexService::with_runner(&runner);

        let report = service.start_for_workspace(&context, &Map::new(), false);

        assert!(report.warnings.is_empty());
        let status = read_bsl_index_status(&context).unwrap();
        let metrics = status
            .last_run
            .expect("fresh info should not erase existing index metrics");
        assert_eq!(metrics.action, "build");
        assert_eq!(metrics.duration_ms, 1234);
        assert_eq!(metrics.index_version.as_deref(), Some("v14"));
        cleanup(&context);
    }

    #[test]
    fn stale_index_starts_background_update() {
        let context = test_context("stale");
        fs::create_dir_all(context.workspace_root.join("src/CommonModules")).unwrap();
        let runner = RecordingIndexRunner {
            outputs: RefCell::new(vec![IndexOutput::success(
                "Index: /tmp/bsl_index.db\n  Status:   stale (age)\n",
            )]),
            ..Default::default()
        };
        let service = WorkspaceIndexService::with_runner(&runner);

        let report = service.start_for_workspace(&context, &Map::new(), false);

        assert_eq!(report.warnings, vec!["rlm index building".to_string()]);
        assert_eq!(
            runner.backgrounds.borrow()[0].primary.args[0..2],
            ["index", "update"]
        );
        cleanup(&context);
    }

    #[test]
    fn successful_background_job_records_last_run_metrics_in_status() {
        let context = test_context("metrics");
        let db_path = context.cache_root.join("rlm-tools-bsl/a/bsl_index.db");
        fs::create_dir_all(db_path.parent().unwrap()).unwrap();
        fs::write(&db_path, "").unwrap();
        let status = status_path(&context);
        let lock = lock_path(&context);
        fs::create_dir_all(lock.parent().unwrap()).unwrap();
        let lock_lease = acquire_index_lock(&lock, "build", &context.workspace_root.join("src"))
            .unwrap()
            .expect("lock should be acquired for background job");

        run_background_job(IndexBackgroundJob {
            action: "build".to_string(),
            source_root: context.workspace_root.join("src"),
            primary: print_lines_command(
                &context.workspace_root,
                true,
                &[
                    "Index built in 1.2s".to_string(),
                    "  Index:    v14".to_string(),
                    "  Modules:  24".to_string(),
                    "  Methods:  617".to_string(),
                    "  DB size:  1.3 MB".to_string(),
                ],
            ),
            info: print_lines_command(
                &context.workspace_root,
                false,
                &[
                    format!("Index: {}", db_path.display()),
                    "  Status:   fresh".to_string(),
                ],
            ),
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
        cleanup(&context);
    }

    #[test]
    fn released_lock_does_not_block_next_index_build() {
        let context = test_context("released-lock");
        fs::create_dir_all(context.workspace_root.join("src/CommonModules")).unwrap();
        write_released_lock(&context, "build");
        write_old_building_status(&context, "build");
        let runner = RecordingIndexRunner {
            outputs: RefCell::new(vec![IndexOutput::success(
                "Index not found: /tmp/bsl_index.db",
            )]),
            ..Default::default()
        };
        let service = WorkspaceIndexService::with_runner(&runner);

        let report = service.start_for_workspace(&context, &Map::new(), false);

        assert_eq!(report.warnings, vec!["rlm index build started".to_string()]);
        assert_eq!(
            runner.backgrounds.borrow()[0].primary.args[0..2],
            ["index", "build"]
        );
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
        let runner = RecordingIndexRunner::default();
        let service = WorkspaceIndexService::with_runner(&runner);

        let readiness = service.ready_index(&context, &Map::new());

        assert_eq!(readiness, IndexReadiness::Building);
        assert!(runner.commands.borrow().is_empty());
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

        let report = service.start_for_workspace(&context, &Map::new(), false);

        assert!(report.warnings.is_empty());
        let current = read_lock_path(&lock).expect("replacement lock should remain");
        assert_eq!(current.lock_id, "new-owner");
        cleanup(&context);
    }

    #[test]
    fn stale_structured_lock_is_marked_recovered_before_rebuild() {
        let context = test_context("stale-structured-recovered");
        fs::create_dir_all(context.workspace_root.join("src/CommonModules")).unwrap();
        write_stale_lock(&context, "build");
        write_old_building_status(&context, "build");

        assert!(!active_lock(&context, &context.workspace_root.join("src")));

        let current =
            read_lock_path(&lock_path(&context)).expect("stale lock should remain as marker");
        assert_eq!(current.state, "recovered");
        cleanup(&context);
    }

    #[derive(Default)]
    struct RecordingIndexRunner {
        outputs: RefCell<Vec<IndexOutput>>,
        commands: RefCell<Vec<IndexCommand>>,
        backgrounds: RefCell<Vec<IndexBackgroundJob>>,
    }

    impl IndexRunner for RecordingIndexRunner {
        fn run(&self, command: &IndexCommand) -> Result<IndexOutput, String> {
            self.commands.borrow_mut().push(command.clone());
            if self.outputs.borrow().is_empty() {
                return Ok(IndexOutput::success("Index not found: /tmp/bsl_index.db"));
            }
            Ok(self.outputs.borrow_mut().remove(0))
        }

        fn start_background(&self, job: IndexBackgroundJob) -> Result<(), String> {
            self.backgrounds.borrow_mut().push(job);
            Ok(())
        }
    }

    struct FailingReplacingIndexRunner {
        replacement_lock_id: String,
    }

    impl IndexRunner for FailingReplacingIndexRunner {
        fn run(&self, _command: &IndexCommand) -> Result<IndexOutput, String> {
            Ok(IndexOutput::success("Index not found: /tmp/bsl_index.db"))
        }

        fn start_background(&self, job: IndexBackgroundJob) -> Result<(), String> {
            let mut replacement = BslIndexLock::new("build", &job.source_root);
            replacement.lock_id = self.replacement_lock_id.clone();
            write_lock_path(&job.lock_path, replacement).unwrap();
            Err("simulated background start failure".to_string())
        }
    }

    fn force_lock_updated_at(lease: &mut IndexLockLease, updated_at: u64) {
        lease.lock.updated_at = updated_at;
        write_lock_file_to_open(&mut lease.file, &lease.lock).unwrap();
    }

    impl IndexOutput {
        fn success(stdout: impl Into<String>) -> Self {
            Self {
                status_success: true,
                status: "exit status: 0".to_string(),
                stdout: stdout.into(),
                stderr: String::new(),
                timed_out: false,
                duration_ms: 0,
            }
        }
    }

    fn print_lines_command(cwd: &Path, sleep_first: bool, lines: &[String]) -> IndexCommand {
        #[cfg(windows)]
        {
            let mut script = String::new();
            if sleep_first {
                script.push_str("Start-Sleep -Milliseconds 20; ");
            }
            for line in lines {
                script.push_str("[Console]::Out.WriteLine('");
                script.push_str(&line.replace('\'', "''"));
                script.push_str("'); ");
            }
            return IndexCommand {
                program: PathBuf::from("powershell"),
                args: vec!["-NoProfile".to_string(), "-Command".to_string(), script],
                cwd: cwd.to_path_buf(),
                env: Vec::new(),
                timeout: Duration::from_secs(5),
            };
        }

        #[cfg(not(windows))]
        {
            let mut script = String::new();
            if sleep_first {
                script.push_str("sleep 0.01; ");
            }
            script.push_str("printf '%s\\n'");
            for line in lines {
                script.push_str(" '");
                script.push_str(&line.replace('\'', "'\\''"));
                script.push('\'');
            }
            IndexCommand {
                program: PathBuf::from("/bin/sh"),
                args: vec!["-c".to_string(), script],
                cwd: cwd.to_path_buf(),
                env: Vec::new(),
                timeout: Duration::from_secs(5),
            }
        }
    }

    fn make_lock_file_old(context: &WorkspaceContext) {
        #[cfg(windows)]
        let status = {
            let path = lock_path(context).display().to_string().replace('\'', "''");
            let script = format!(
                "(Get-Item -LiteralPath '{path}').LastWriteTimeUtc = [datetime]'2000-01-01T00:00:00Z'"
            );
            std::process::Command::new("powershell")
                .args(["-NoProfile", "-Command"])
                .arg(script)
                .status()
                .unwrap()
        };
        #[cfg(not(windows))]
        let status = std::process::Command::new("touch")
            .args(["-t", "200001010000"])
            .arg(lock_path(context))
            .status()
            .unwrap();
        assert!(status.success());
    }

    fn test_context(name: &str) -> WorkspaceContext {
        let root = std::env::temp_dir().join(format!("unica-index-{name}-{}", now_nanos()));
        fs::create_dir_all(&root).unwrap();
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

    fn cleanup(context: &WorkspaceContext) {
        let _ = fs::remove_dir_all(&context.workspace_root);
    }
}
