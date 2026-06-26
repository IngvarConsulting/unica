use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::legacy_scripts::find_plugin_root;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
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

#[derive(Debug, Clone)]
pub struct IndexBackgroundJob {
    pub action: String,
    pub source_root: PathBuf,
    pub primary: IndexCommand,
    pub info: IndexCommand,
    pub status_path: PathBuf,
    pub lock_path: PathBuf,
    pub lock_id: String,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    child_pid: Option<u32>,
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
        let launcher = plugin_root.join("scripts").join("run-rlm-bsl-index.sh");
        if !launcher.exists() {
            return Err(format!(
                "internal RLM index launcher not found: {}",
                launcher.display()
            ));
        }
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
                program: launcher.clone(),
                args: vec!["index".to_string(), "info".to_string(), root.clone()],
                cwd: context.cwd.clone(),
                env: env.clone(),
                timeout: INDEX_TIMEOUT,
            },
            build: IndexCommand {
                program: launcher.clone(),
                args: vec!["index".to_string(), "build".to_string(), root.clone()],
                cwd: context.cwd.clone(),
                env: env.clone(),
                timeout: Duration::from_secs(24 * 60 * 60),
            },
            update: IndexCommand {
                program: launcher,
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

        let index_lock = BslIndexLock::new(action, &source_root);
        let lock_id = index_lock.lock_id.clone();
        match OpenOptions::new().create_new(true).write(true).open(&lock) {
            Ok(mut file) => {
                if let Err(error) = write_lock_file(&mut file, &index_lock) {
                    let _ = fs::remove_file(&lock);
                    let _ = write_status(
                        context,
                        BslIndexStatus::failed(error.as_str(), Some(&source_root)),
                    );
                    return IndexStartReport::default();
                }
            }
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                return IndexStartReport {
                    warnings: vec!["rlm index building".to_string()],
                };
            }
            Err(error) => {
                let message = format!("failed to acquire RLM index lock: {error}");
                let _ = write_status(
                    context,
                    BslIndexStatus::failed(message.as_str(), Some(&source_root)),
                );
                return IndexStartReport::default();
            }
        }

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
            lock_path: lock.clone(),
            lock_id,
        };
        if let Err(error) = self.runner.start_background(job) {
            let _ = fs::remove_file(lock);
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
            child_pid: None,
        }
    }

    fn is_fresh(&self) -> bool {
        self.schema_version == LOCK_SCHEMA_VERSION
            && now_secs().saturating_sub(self.updated_at) <= LOCK_STALE_AFTER.as_secs()
    }
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

fn run_background_job(job: IndexBackgroundJob) {
    let _lock_cleanup = LockCleanup {
        path: job.lock_path.clone(),
        lock_id: job.lock_id.clone(),
    };
    let started_at = now_secs();
    let result =
        run_index_command_with_heartbeat(&job.primary, Some((&job.lock_path, &job.lock_id)));
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
    heartbeat: Option<(&Path, &str)>,
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
    if let Some((path, lock_id)) = heartbeat {
        refresh_lock_heartbeat(path, lock_id, child.id());
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

        if let Some((path, lock_id)) = heartbeat {
            if last_heartbeat.elapsed() >= LOCK_HEARTBEAT_INTERVAL {
                refresh_lock_heartbeat(path, lock_id, child.id());
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

struct LockCleanup {
    path: PathBuf,
    lock_id: String,
}

impl Drop for LockCleanup {
    fn drop(&mut self) {
        remove_owned_lock(&self.path, &self.lock_id);
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
    match read_lock_path(&lock) {
        Ok(index_lock) if index_lock.is_fresh() => true,
        Ok(index_lock) => {
            recover_stale_lock(
                context,
                source_root,
                format!(
                    "RLM index {action} lock is stale",
                    action = index_lock.action
                )
                .as_str(),
                Some(index_lock.lock_id.as_str()),
            );
            false
        }
        Err(error) => {
            if invalid_lock_may_be_active(context, &lock) {
                return true;
            }
            recover_stale_lock(
                context,
                source_root,
                format!("RLM index lock is invalid: {error}").as_str(),
                None,
            );
            false
        }
    }
}

fn invalid_lock_may_be_active(context: &WorkspaceContext, lock: &Path) -> bool {
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
) {
    let lock = lock_path(context);
    match lock_id {
        Some(lock_id) => remove_owned_lock(&lock, lock_id),
        None => {
            let _ = fs::remove_file(lock);
        }
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
}

fn read_lock_path(path: &Path) -> Result<BslIndexLock, String> {
    let text = fs::read_to_string(path).map_err(|error| error.to_string())?;
    serde_json::from_str(&text).map_err(|error| error.to_string())
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

fn write_owned_lock_path(
    path: &Path,
    index_lock: BslIndexLock,
    expected_lock_id: &str,
) -> Result<bool, String> {
    let temp_path = lock_temp_path(path);
    {
        let mut temp = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp_path)
            .map_err(|error| format!("failed to create temporary RLM index lock: {error}"))?;
        write_lock_file(&mut temp, &index_lock)?;
    }
    match read_lock_path(path) {
        Ok(current) if current.lock_id == expected_lock_id => {
            fs::rename(&temp_path, path).map_err(|error| {
                let _ = fs::remove_file(&temp_path);
                format!("failed to replace RLM index lock atomically: {error}")
            })?;
            Ok(true)
        }
        _ => {
            let _ = fs::remove_file(&temp_path);
            Ok(false)
        }
    }
}

fn refresh_lock_heartbeat(path: &Path, lock_id: &str, child_pid: u32) {
    let Ok(mut index_lock) = read_lock_path(path) else {
        return;
    };
    if index_lock.lock_id != lock_id {
        return;
    }
    index_lock.updated_at = now_secs();
    index_lock.child_pid = Some(child_pid);
    let _ = write_owned_lock_path(path, index_lock, lock_id);
}

fn remove_owned_lock(path: &Path, lock_id: &str) {
    if read_lock_path(path)
        .map(|index_lock| index_lock.lock_id == lock_id)
        .unwrap_or(false)
    {
        let _ = fs::remove_file(path);
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
        assert_eq!(
            runner.backgrounds.borrow()[0].primary.args[0..2],
            ["index", "build"]
        );
        assert_eq!(
            runner.backgrounds.borrow()[0].primary.env[0].0,
            "RLM_INDEX_DIR"
        );
        assert!(runner.backgrounds.borrow()[0].primary.env[0]
            .1
            .contains(".build/unica/rlm-tools-bsl"));
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
        let index_lock = BslIndexLock::new("build", &context.workspace_root.join("src"));
        let lock_id = index_lock.lock_id.clone();
        write_lock_path(&lock, index_lock).unwrap();

        run_background_job(IndexBackgroundJob {
            action: "build".to_string(),
            source_root: context.workspace_root.join("src"),
            primary: shell_command(
                &context.workspace_root,
                "sleep 0.01; printf '%s\n' 'Index built in 1.2s' '  Index:    v14' '  Modules:  24' '  Methods:  617' '  DB size:  1.3 MB'",
            ),
            info: shell_command(
                &context.workspace_root,
                format!(
                    "printf '%s\n' 'Index: {}' '  Status:   fresh'",
                    db_path.display()
                ),
            ),
            status_path: status.clone(),
            lock_path: lock.clone(),
            lock_id,
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
        assert!(!lock.exists());
        cleanup(&context);
    }

    #[test]
    fn cleanup_does_not_remove_lock_replaced_by_new_owner() {
        let context = test_context("cleanup-owner");
        let lock = lock_path(&context);
        fs::create_dir_all(lock.parent().unwrap()).unwrap();
        let mut old_lock = BslIndexLock::new("build", &context.workspace_root.join("src"));
        old_lock.lock_id = "old-owner".to_string();
        let mut new_lock = BslIndexLock::new("build", &context.workspace_root.join("src"));
        new_lock.lock_id = "new-owner".to_string();
        write_lock_path(&lock, new_lock.clone()).unwrap();

        drop(LockCleanup {
            path: lock.clone(),
            lock_id: old_lock.lock_id,
        });

        let current = read_lock_path(&lock).expect("replacement lock should remain");
        assert_eq!(current.lock_id, new_lock.lock_id);
        cleanup(&context);
    }

    #[test]
    fn heartbeat_does_not_overwrite_lock_replaced_by_new_owner() {
        let context = test_context("heartbeat-owner");
        let lock = lock_path(&context);
        fs::create_dir_all(lock.parent().unwrap()).unwrap();
        let mut old_lock = BslIndexLock::new("build", &context.workspace_root.join("src"));
        old_lock.lock_id = "old-owner".to_string();
        let mut new_lock = BslIndexLock::new("build", &context.workspace_root.join("src"));
        new_lock.lock_id = "new-owner".to_string();
        write_lock_path(&lock, new_lock.clone()).unwrap();

        refresh_lock_heartbeat(&lock, &old_lock.lock_id, 42);

        let current = read_lock_path(&lock).expect("replacement lock should remain readable");
        assert_eq!(current.lock_id, new_lock.lock_id);
        assert_eq!(current.child_pid, new_lock.child_pid);
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

    fn shell_command(cwd: &Path, script: impl Into<String>) -> IndexCommand {
        IndexCommand {
            program: PathBuf::from("/bin/sh"),
            args: vec!["-c".to_string(), script.into()],
            cwd: cwd.to_path_buf(),
            env: Vec::new(),
            timeout: Duration::from_secs(5),
        }
    }

    fn test_context(name: &str) -> WorkspaceContext {
        let root = std::env::temp_dir().join(format!("unica-index-{name}-{}", now_secs()));
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
        fs::create_dir_all(plugin_root.join("scripts")).unwrap();
        fs::write(plugin_root.join("scripts").join("run-rlm-bsl-index.sh"), "").unwrap();
    }

    fn write_fresh_lock(context: &WorkspaceContext, action: &str) {
        fs::create_dir_all(lock_path(context).parent().unwrap()).unwrap();
        let text = serde_json::json!({
            "schema_version": 1,
            "lock_id": new_lock_id(),
            "owner_pid": std::process::id(),
            "action": action,
            "source_root": context.workspace_root.join("src").display().to_string(),
            "started_at": now_secs(),
            "updated_at": now_secs()
        });
        fs::write(
            lock_path(context),
            serde_json::to_string_pretty(&text).unwrap() + "\n",
        )
        .unwrap();
    }

    fn write_stale_lock(context: &WorkspaceContext, action: &str) {
        fs::create_dir_all(lock_path(context).parent().unwrap()).unwrap();
        let stale = now_secs().saturating_sub(LOCK_STALE_AFTER.as_secs() + 1);
        let text = serde_json::json!({
            "schema_version": 1,
            "lock_id": new_lock_id(),
            "owner_pid": std::process::id(),
            "action": action,
            "source_root": context.workspace_root.join("src").display().to_string(),
            "started_at": stale,
            "updated_at": stale
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

    fn make_lock_file_old(context: &WorkspaceContext) {
        let status = std::process::Command::new("touch")
            .args(["-t", "200001010000"])
            .arg(lock_path(context))
            .status()
            .unwrap();
        assert!(status.success());
    }

    fn cleanup(context: &WorkspaceContext) {
        let _ = fs::remove_dir_all(&context.workspace_root);
    }
}
