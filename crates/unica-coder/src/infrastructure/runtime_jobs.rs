//! Durable runtime-job state used by the runtime-job worker and transport adapter.

use super::redaction;
use super::runtime_build_fallback::{
    classify_partial_platform_failure, full_rebuild_argv, BuildAttempt, PARTIAL_FALLBACK_WARNING,
};
use super::runtime_build_preflight::RuntimeBuildPreflight;
use crate::domain::cache::CacheAccess;
use crate::domain::cancellation::{cancelled_error, CancellationToken};
use crate::domain::events::{runtime_event_kind, DomainEvent};
use crate::infrastructure::platform::filesystem::{replace_file_atomically, sync_parent_directory};
use crate::infrastructure::platform::{
    cancel_runtime_job_process_tree, configure_runtime_job_command, STDOUT_CAPTURE_LIMIT,
};
use crate::infrastructure::workspace::discover_workspace;
use crate::infrastructure::workspace_services::WorkspaceServiceManager;
use crate::infrastructure::workspace_state::WorkspaceStateRepository;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

const RECORD_SCHEMA_VERSION: u8 = 1;
const OUTPUT_TAIL_BYTES: usize = 16 * 1024;
const FALLBACK_RECEIPT_BYTES: usize = STDOUT_CAPTURE_LIMIT;
const DEFAULT_STALE_AFTER: Duration = Duration::from_secs(5 * 60);

type JobResult<T> = Result<T, String>;

/// Classifies whether interrupting an operation can leave its workspace inconsistent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum CancelPolicy {
    Safe,
    Critical,
}

/// The operation classes deliberately accepted by the durable core.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum RuntimeJobOperation {
    Make,
    Syntax,
    Test,
    ToolsDownload,
    ConfigInit,
    Init,
    Build,
    Dump,
    Convert,
    Load,
    Launch,
    Extensions,
}

impl RuntimeJobOperation {
    pub(crate) fn cancel_policy(self) -> CancelPolicy {
        match self {
            Self::Make | Self::Syntax | Self::Test | Self::ToolsDownload => CancelPolicy::Safe,
            Self::ConfigInit
            | Self::Init
            | Self::Build
            | Self::Dump
            | Self::Convert
            | Self::Load
            | Self::Launch
            | Self::Extensions => CancelPolicy::Critical,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Make => "make",
            Self::Syntax => "syntax",
            Self::Test => "test",
            Self::ToolsDownload => "tools-download",
            Self::ConfigInit => "config-init",
            Self::Init => "init",
            Self::Build => "build",
            Self::Dump => "dump",
            Self::Convert => "convert",
            Self::Load => "load",
            Self::Launch => "launch",
            Self::Extensions => "extensions",
        }
    }

    pub(crate) fn from_label(label: &str) -> JobResult<Self> {
        match label {
            "make" => Ok(Self::Make),
            "syntax" => Ok(Self::Syntax),
            "test" => Ok(Self::Test),
            "tools-download" => Ok(Self::ToolsDownload),
            "config-init" => Ok(Self::ConfigInit),
            "init" => Ok(Self::Init),
            "build" => Ok(Self::Build),
            "dump" => Ok(Self::Dump),
            "convert" => Ok(Self::Convert),
            "load" => Ok(Self::Load),
            "launch" => Ok(Self::Launch),
            "extensions" => Ok(Self::Extensions),
            _ => Err(redacted_error("unsupported runtime job operation")),
        }
    }
}

/// A request may carry raw arguments only in memory.  They are never persisted verbatim.
#[derive(Debug, Clone)]
pub(crate) struct RuntimeJobRequest {
    operation: RuntimeJobOperation,
    raw_argv: Vec<String>,
    safe_target: String,
    artifact_path: Option<String>,
    timeout_reason: Option<String>,
    build_preflight: Option<RuntimeBuildPreflight>,
    full_rebuild_fallback_argv: Option<Vec<String>>,
}

impl RuntimeJobRequest {
    pub(crate) fn new(
        operation: RuntimeJobOperation,
        raw_argv: Vec<String>,
        safe_target: impl Into<String>,
        artifact_path: Option<String>,
    ) -> Self {
        let full_rebuild_fallback_argv = (operation == RuntimeJobOperation::Build)
            .then(|| full_rebuild_argv(&raw_argv))
            .flatten();
        Self {
            operation,
            raw_argv,
            safe_target: safe_target.into(),
            artifact_path,
            timeout_reason: None,
            build_preflight: None,
            full_rebuild_fallback_argv,
        }
    }

    pub(crate) fn with_build_preflight(
        mut self,
        build_preflight: Option<RuntimeBuildPreflight>,
    ) -> Self {
        self.build_preflight = build_preflight;
        self
    }

    fn validate_build_preflight(&self) -> JobResult<()> {
        let full_rebuild = self
            .raw_argv
            .iter()
            .any(|argument| argument == "--full-rebuild");
        if self.operation == RuntimeJobOperation::Build
            && !full_rebuild
            && self.build_preflight.is_none()
        {
            return Err(redacted_error(
                "normal build worker request is missing prelaunch authorization; retry with \
                 `fullRebuild: true`",
            ));
        }
        match &self.full_rebuild_fallback_argv {
            Some(fallback) => {
                if self.operation != RuntimeJobOperation::Build
                    || full_rebuild
                    || !self
                        .raw_argv
                        .iter()
                        .any(|argument| argument == "--json-message")
                    || full_rebuild_argv(&self.raw_argv).as_ref() != Some(fallback)
                {
                    return Err(redacted_error(
                        "runtime job full rebuild fallback arguments are inconsistent",
                    ));
                }
            }
            None if self.operation == RuntimeJobOperation::Build && !full_rebuild => {
                return Err(redacted_error(
                    "normal build worker request is missing its full rebuild fallback",
                ));
            }
            None => {}
        }
        Ok(())
    }

    fn take_full_rebuild_fallback(&mut self) -> Option<Self> {
        let raw_argv = self.full_rebuild_fallback_argv.take()?;
        let mut fallback = self.clone();
        fallback.raw_argv = raw_argv;
        fallback.full_rebuild_fallback_argv = None;
        Some(fallback)
    }

    #[cfg(test)]
    fn without_build_preflight_for_test(mut self) -> Self {
        self.build_preflight = None;
        self
    }

    #[allow(dead_code)]
    pub(crate) fn with_timeout_reason(mut self, timeout_reason: impl Into<String>) -> Self {
        self.timeout_reason = Some(timeout_reason.into());
        self
    }

    #[cfg(test)]
    pub(crate) fn raw_argv(&self) -> &[String] {
        &self.raw_argv
    }

    #[cfg(test)]
    pub(crate) fn build_preflight(&self) -> Option<&RuntimeBuildPreflight> {
        self.build_preflight.as_ref()
    }

    #[cfg(test)]
    pub(crate) fn full_rebuild_fallback_argv(&self) -> Option<&[String]> {
        self.full_rebuild_fallback_argv.as_deref()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum RuntimeJobPhase {
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
    fn is_terminal(self) -> bool {
        match self {
            Self::Queued | Self::Running | Self::CancelRequested => false,
            Self::Succeeded | Self::Failed | Self::Cancelled | Self::TimedOut | Self::Lost => true,
        }
    }
}

/// A process exit is intentionally nonblocking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RuntimeJobProcessState {
    Running,
    Exited {
        exit_code: i32,
    },
    // SystemRuntimeJobRunner delegates execution timeouts to v8-runner, while
    // the runner boundary still models timeout-capable implementations.
    #[allow(dead_code)]
    TimedOut {
        reason: String,
    },
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RuntimeJobOutput {
    pub(crate) stdout: String,
    pub(crate) stderr: String,
    pub(crate) stdout_truncated: bool,
    fallback_receipt: Option<String>,
    fallback_receipt_truncated: bool,
}

/// Process boundary for the core. Implementations must not expose shell snippets.
pub(crate) trait RuntimeJobProcess: Send {
    fn id(&self) -> u32;
    fn try_wait(&mut self) -> JobResult<RuntimeJobProcessState>;
    fn cancel(&mut self) -> JobResult<()>;
    /// Return at most `max_bytes` of each stream. The core redacts the retained tails again.
    fn output_tails(&mut self, max_bytes: usize) -> JobResult<RuntimeJobOutput>;
}

/// Runner boundary. `attach` reconnects to an existing process; it never starts it again.
pub(crate) trait RuntimeJobRunner: Send + Sync {
    fn spawn(&self, request: &RuntimeJobRequest) -> JobResult<Box<dyn RuntimeJobProcess>>;
    fn attach(&self, process_id: u32) -> JobResult<Box<dyn RuntimeJobProcess>>;
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WorkerStartRequest {
    cache_root: PathBuf,
    job_id: String,
    program: PathBuf,
    cwd: PathBuf,
    operation: String,
    argv: Vec<String>,
    safe_target: String,
    artifact_path: Option<String>,
    timeout_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    build_preflight: Option<RuntimeBuildPreflight>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    full_rebuild_fallback_argv: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WorkerStartCommit {
    cancelled: bool,
}

impl WorkerStartRequest {
    fn new(
        cache_root: PathBuf,
        job_id: String,
        program: PathBuf,
        cwd: PathBuf,
        request: &RuntimeJobRequest,
    ) -> Self {
        Self {
            cache_root,
            job_id,
            program,
            cwd,
            operation: request.operation.label().to_string(),
            argv: request.raw_argv.clone(),
            safe_target: request.safe_target.clone(),
            artifact_path: request.artifact_path.clone(),
            timeout_reason: request.timeout_reason.clone(),
            build_preflight: request.build_preflight.clone(),
            full_rebuild_fallback_argv: request.full_rebuild_fallback_argv.clone(),
        }
    }

    fn runtime_request(&self) -> JobResult<RuntimeJobRequest> {
        let operation = RuntimeJobOperation::from_label(&self.operation)?;
        let mut request = RuntimeJobRequest::new(
            operation,
            self.argv.clone(),
            self.safe_target.clone(),
            self.artifact_path.clone(),
        );
        request.timeout_reason = self.timeout_reason.clone();
        request.build_preflight = self.build_preflight.clone();
        request.full_rebuild_fallback_argv = self.full_rebuild_fallback_argv.clone();
        request.validate_build_preflight()?;
        Ok(request)
    }
}

struct SystemRuntimeJobRunner {
    program: PathBuf,
    cwd: PathBuf,
}

impl RuntimeJobRunner for SystemRuntimeJobRunner {
    fn spawn(&self, request: &RuntimeJobRequest) -> JobResult<Box<dyn RuntimeJobProcess>> {
        let mut command = Command::new(&self.program);
        command
            .args(&request.raw_argv)
            .current_dir(&self.cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        configure_runtime_job_command(&mut command);
        let mut child = command
            .spawn()
            .map_err(|error| redacted_error(&format!("spawn runtime job process: {error}")))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| redacted_error("runtime job process has no stdout pipe"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| redacted_error("runtime job process has no stderr pipe"))?;
        let stdout = if request.full_rebuild_fallback_argv.is_some() {
            StreamTail::spawn_with_receipt(stdout)
        } else {
            StreamTail::spawn(stdout)
        };
        Ok(Box::new(SystemRuntimeJobProcess {
            id: child.id(),
            child,
            stdout,
            stderr: StreamTail::spawn(stderr),
            exited: false,
        }))
    }

    fn attach(&self, _process_id: u32) -> JobResult<Box<dyn RuntimeJobProcess>> {
        Err(redacted_error(
            "runtime job worker cannot attach to an unowned process",
        ))
    }
}

struct SystemRuntimeJobProcess {
    id: u32,
    child: Child,
    stdout: StreamTail,
    stderr: StreamTail,
    exited: bool,
}

impl RuntimeJobProcess for SystemRuntimeJobProcess {
    fn id(&self) -> u32 {
        self.id
    }

    fn try_wait(&mut self) -> JobResult<RuntimeJobProcessState> {
        match self
            .child
            .try_wait()
            .map_err(|error| redacted_error(&format!("poll runtime job process: {error}")))?
        {
            Some(status) => {
                self.exited = true;
                Ok(RuntimeJobProcessState::Exited {
                    exit_code: status.code().unwrap_or(1),
                })
            }
            None => Ok(RuntimeJobProcessState::Running),
        }
    }

    fn cancel(&mut self) -> JobResult<()> {
        cancel_runtime_job_process_tree(self.id)
    }

    fn output_tails(&mut self, max_bytes: usize) -> JobResult<RuntimeJobOutput> {
        if self.exited {
            self.stdout.finish()?;
            self.stderr.finish()?;
        }
        let (stdout, stdout_truncated) = self.stdout.tail(max_bytes)?;
        let (fallback_receipt, receipt_truncated) = self.stdout.receipt()?;
        let (stderr, _) = self.stderr.tail(max_bytes)?;
        Ok(RuntimeJobOutput {
            stdout,
            stderr,
            stdout_truncated,
            fallback_receipt,
            fallback_receipt_truncated: receipt_truncated,
        })
    }
}

struct StreamTail {
    text: Arc<Mutex<String>>,
    truncated: Arc<std::sync::atomic::AtomicBool>,
    receipt: Option<Arc<Mutex<Vec<u8>>>>,
    receipt_truncated: Arc<std::sync::atomic::AtomicBool>,
    reader: Option<thread::JoinHandle<io::Result<()>>>,
}

impl StreamTail {
    fn spawn<R>(stream: R) -> Self
    where
        R: Read + Send + 'static,
    {
        Self::spawn_inner(stream, false)
    }

    fn spawn_with_receipt<R>(stream: R) -> Self
    where
        R: Read + Send + 'static,
    {
        Self::spawn_inner(stream, true)
    }

    fn spawn_inner<R>(mut stream: R, retain_receipt: bool) -> Self
    where
        R: Read + Send + 'static,
    {
        let text = Arc::new(Mutex::new(String::new()));
        let captured = Arc::clone(&text);
        let truncated = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let capture_truncated = Arc::clone(&truncated);
        let receipt = retain_receipt.then(|| Arc::new(Mutex::new(Vec::new())));
        let captured_receipt = receipt.clone();
        let receipt_truncated = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let capture_receipt_truncated = Arc::clone(&receipt_truncated);
        let reader = thread::spawn(move || {
            let mut buffer = [0_u8; 4096];
            let mut redactor = redaction::StreamRedactor::new();
            loop {
                let count = stream.read(&mut buffer)?;
                if count == 0 {
                    append_tail(&captured, &capture_truncated, &redactor.finish())?;
                    return Ok(());
                }
                if let Some(receipt) = &captured_receipt {
                    append_byte_tail(receipt, &capture_receipt_truncated, &buffer[..count])?;
                }
                let chunk = String::from_utf8_lossy(&buffer[..count]);
                append_tail(&captured, &capture_truncated, &redactor.push(&chunk))?;
            }
        });
        Self {
            text,
            truncated,
            receipt,
            receipt_truncated,
            reader: Some(reader),
        }
    }

    fn finish(&mut self) -> JobResult<()> {
        let Some(reader) = self.reader.take() else {
            return Ok(());
        };
        reader
            .join()
            .map_err(|_| redacted_error("join runtime job output reader"))?
            .map_err(|error| io_error("read runtime job output", &error))
    }

    fn tail(&self, max_bytes: usize) -> JobResult<(String, bool)> {
        let text = self
            .text
            .lock()
            .map_err(|error| redacted_error(&format!("lock runtime job output: {error}")))?;
        let truncated =
            self.truncated.load(std::sync::atomic::Ordering::Acquire) || text.len() > max_bytes;
        Ok((bounded_tail(&text, max_bytes), truncated))
    }

    fn receipt(&self) -> JobResult<(Option<String>, bool)> {
        let Some(receipt) = &self.receipt else {
            return Ok((None, false));
        };
        let bytes = receipt
            .lock()
            .map_err(|error| redacted_error(&format!("lock runtime job receipt: {error}")))?;
        let truncated = self
            .receipt_truncated
            .load(std::sync::atomic::Ordering::Acquire)
            || bytes.len() > FALLBACK_RECEIPT_BYTES;
        if truncated {
            return Ok((None, true));
        }
        Ok((String::from_utf8(bytes.clone()).ok(), false))
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeJobSnapshot {
    pub(crate) id: String,
    pub(crate) phase: RuntimeJobPhase,
    pub(crate) operation: String,
    pub(crate) safe_target: String,
    pub(crate) redacted_argv: Vec<String>,
    pub(crate) created_at_ms: u64,
    pub(crate) started_at_ms: Option<u64>,
    pub(crate) heartbeat_at_ms: Option<u64>,
    pub(crate) finished_at_ms: Option<u64>,
    pub(crate) pid: Option<u32>,
    pub(crate) pid_identity: Option<String>,
    pub(crate) exit_code: Option<i32>,
    pub(crate) cancelled: bool,
    pub(crate) cancel_deferred: bool,
    pub(crate) unsafe_phase: Option<String>,
    pub(crate) timeout_reason: Option<String>,
    pub(crate) artifact_path: Option<String>,
    pub(crate) stdout_path: String,
    pub(crate) stderr_path: String,
    pub(crate) warnings: Vec<String>,
    pub(crate) wait_timed_out: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct RuntimeJobLogs {
    pub(crate) stdout: String,
    pub(crate) stderr: String,
    pub(crate) stdout_path: String,
    pub(crate) stderr_path: String,
}

#[derive(Debug, Clone)]
pub(crate) struct RuntimeJobList {
    pub(crate) jobs: Vec<RuntimeJobSnapshot>,
    pub(crate) warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RuntimeJobRecord {
    #[serde(rename = "schemaVersion")]
    schema_version: u8,
    id: String,
    phase: RuntimeJobPhase,
    operation: String,
    safe_target: String,
    redacted_argv: Vec<String>,
    created_at_ms: u64,
    started_at_ms: Option<u64>,
    heartbeat_at_ms: Option<u64>,
    finished_at_ms: Option<u64>,
    pid: Option<u32>,
    pid_identity: Option<String>,
    exit_code: Option<i32>,
    cancel_policy: CancelPolicy,
    cancelled: bool,
    cancel_deferred: bool,
    cancel_attempted: bool,
    unsafe_phase: Option<String>,
    timeout_reason: Option<String>,
    artifact_path: Option<String>,
    stdout_path: String,
    stderr_path: String,
    warnings: Vec<String>,
}

impl RuntimeJobRecord {
    fn queued(id: String, request: &RuntimeJobRequest) -> Self {
        let now = now_millis();
        Self {
            schema_version: RECORD_SCHEMA_VERSION,
            id: id.clone(),
            phase: RuntimeJobPhase::Queued,
            operation: request.operation.label().to_string(),
            safe_target: redact_text(&request.safe_target),
            redacted_argv: redact_argv(&request.raw_argv),
            created_at_ms: now,
            // The public start contract reports when the durable job lifecycle
            // began. The worker replaces this with the child-start timestamp
            // once it has successfully claimed the queued record.
            started_at_ms: Some(now),
            heartbeat_at_ms: Some(now),
            finished_at_ms: None,
            pid: None,
            pid_identity: None,
            exit_code: None,
            cancel_policy: request.operation.cancel_policy(),
            cancelled: false,
            cancel_deferred: false,
            cancel_attempted: false,
            unsafe_phase: None,
            timeout_reason: request.timeout_reason.as_deref().map(redact_text),
            artifact_path: request.artifact_path.as_deref().map(redact_text),
            stdout_path: format!("jobs/{id}/stdout.log"),
            stderr_path: format!("jobs/{id}/stderr.log"),
            warnings: Vec::new(),
        }
    }

    fn snapshot(&self, wait_timed_out: bool) -> RuntimeJobSnapshot {
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

    fn transition(&mut self, next: RuntimeJobPhase) -> JobResult<()> {
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
struct CancelMarker {
    requested_at_ms: u64,
}

struct ActiveLifecycleLock {
    file: File,
}

impl Drop for ActiveLifecycleLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

fn lock_is_contended(error: &io::Error) -> bool {
    let expected = fs2::lock_contended_error();
    error.kind() == io::ErrorKind::WouldBlock
        || error
            .raw_os_error()
            .zip(expected.raw_os_error())
            .is_some_and(|(actual, expected)| actual == expected)
}

#[derive(Debug, Clone)]
struct RuntimeJobStore {
    cache_root: PathBuf,
    stale_after: Duration,
}

impl RuntimeJobStore {
    fn new(cache_root: impl Into<PathBuf>, stale_after: Duration) -> Self {
        Self {
            cache_root: cache_root.into(),
            stale_after,
        }
    }

    fn jobs_root(&self) -> PathBuf {
        self.cache_root.join("jobs")
    }

    fn active_lock_path(&self) -> PathBuf {
        self.jobs_root().join("active.lock")
    }

    fn active_lifecycle_lock_path(&self) -> PathBuf {
        self.jobs_root().join("active.lifecycle.lock")
    }

    fn recovery_lock_path(&self) -> PathBuf {
        self.jobs_root().join("active.recovery.lock")
    }

    fn job_dir(&self, id: &str) -> JobResult<PathBuf> {
        let id = canonical_job_id(id)?;
        Ok(self.jobs_root().join(id))
    }

    fn record_path(&self, id: &str) -> JobResult<PathBuf> {
        Ok(self.job_dir(id)?.join("record.json"))
    }

    fn stdout_path(&self, id: &str) -> JobResult<PathBuf> {
        Ok(self.job_dir(id)?.join("stdout.log"))
    }

    fn stderr_path(&self, id: &str) -> JobResult<PathBuf> {
        Ok(self.job_dir(id)?.join("stderr.log"))
    }

    fn cancel_path(&self, id: &str) -> JobResult<PathBuf> {
        Ok(self.job_dir(id)?.join("cancel.json"))
    }

    fn acquire_active_lock(&self, id: &str) -> JobResult<()> {
        self.acquire_active_lock_after_lifecycle(id, || {})
    }

    fn acquire_active_lock_after_lifecycle(
        &self,
        id: &str,
        after_lifecycle_lock: impl FnOnce(),
    ) -> JobResult<()> {
        let _lifecycle_lock = self.acquire_active_lifecycle_lock()?;
        after_lifecycle_lock();
        self.acquire_active_lock_guarded(id)
    }

    fn acquire_active_lifecycle_lock(&self) -> JobResult<ActiveLifecycleLock> {
        self.open_active_lifecycle_lock(false)?.ok_or_else(|| {
            redacted_error("active runtime job lifecycle lock unexpectedly remained contended")
        })
    }

    fn try_acquire_active_lifecycle_lock(&self) -> JobResult<Option<ActiveLifecycleLock>> {
        self.open_active_lifecycle_lock(true)
    }

    fn open_active_lifecycle_lock(
        &self,
        nonblocking: bool,
    ) -> JobResult<Option<ActiveLifecycleLock>> {
        fs::create_dir_all(self.jobs_root())
            .map_err(|error| io_error("create runtime jobs directory", &error))?;
        let path = self.active_lifecycle_lock_path();
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|error| io_error("open active runtime job lifecycle lock", &error))?;
        let lock_result = if nonblocking {
            FileExt::try_lock_exclusive(&file)
        } else {
            FileExt::lock_exclusive(&file)
        };
        match lock_result {
            Ok(()) => Ok(Some(ActiveLifecycleLock { file })),
            Err(error) if nonblocking && lock_is_contended(&error) => Ok(None),
            Err(error) => Err(io_error(
                "acquire active runtime job lifecycle lock",
                &error,
            )),
        }
    }

    fn acquire_active_lock_guarded(&self, id: &str) -> JobResult<()> {
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

    fn active_job_conflict(&self) -> String {
        match fs::read_to_string(self.active_lock_path()) {
            Ok(id) => {
                let id = id.trim();
                let existing = if id.is_empty() { "unknown" } else { id };
                redacted_error(&format!(
                    "workspace already has active runtime job {existing}"
                ))
            }
            Err(error) => io_error("read active runtime job lock", &error),
        }
    }

    fn release_active_lock_for(&self, id: &str) -> JobResult<()> {
        self.release_active_lock_for_after_observation(id, || {})
    }

    fn release_active_lock_for_after_observation(
        &self,
        id: &str,
        after_observation: impl FnOnce(),
    ) -> JobResult<()> {
        self.release_active_lock_for_after_hooks(id, after_observation, || {})
    }

    fn release_active_lock_for_after_hooks(
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

    fn release_active_lock_guarded(
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

    fn create_record(&self, id: &str, request: &RuntimeJobRequest) -> JobResult<RuntimeJobRecord> {
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

    fn enqueue(&self, id: &str, request: &RuntimeJobRequest) -> JobResult<RuntimeJobRecord> {
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

    fn recover_stale_active(&self) -> JobResult<bool> {
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

    fn recover_stale_active_locked(&self) -> JobResult<bool> {
        let id = match fs::read_to_string(self.active_lock_path()) {
            Ok(id) => id.trim().to_string(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(io_error("read active runtime job lock", &error)),
        };
        let mut record = self.read_record(&id)?;
        if record.phase.is_terminal() {
            // A terminal write can succeed while the final lock removal fails.
            // Its own id makes this cleanup safe. Lost is deliberately excluded:
            // its child may still be alive after a worker crash.
            if record.phase != RuntimeJobPhase::Lost {
                self.release_active_lock_guarded(&id, || {})?;
                return Ok(true);
            }
            return Ok(false);
        }
        if !self.stale(&record) {
            return Ok(false);
        }
        record.transition(RuntimeJobPhase::Lost)?;
        record.finished_at_ms = Some(now_millis());
        record.warnings.push("stale worker heartbeat".to_string());
        self.write_record(&record)?;
        Ok(true)
    }

    fn read_record(&self, id: &str) -> JobResult<RuntimeJobRecord> {
        let path = self.record_path(id)?;
        let contents = fs::read_to_string(&path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                redacted_error(&format!("runtime job {id} record is missing"))
            } else {
                io_error("read runtime job record", &error)
            }
        })?;
        let record: RuntimeJobRecord = serde_json::from_str(&contents).map_err(|error| {
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

    fn write_record(&self, record: &RuntimeJobRecord) -> JobResult<()> {
        let path = self.record_path(&record.id)?;
        let parent = path
            .parent()
            .ok_or_else(|| redacted_error("runtime job record path has no parent"))?;
        fs::create_dir_all(parent)
            .map_err(|error| io_error("create runtime job record directory", &error))?;
        let bytes = serde_json::to_vec_pretty(record)
            .map_err(|error| redacted_error(&format!("serialize runtime job record: {error}")))?;
        atomic_write(&path, &bytes)
    }

    fn write_logs(&self, id: &str, output: &RuntimeJobOutput) -> JobResult<()> {
        let stdout = bounded_redacted_tail(&output.stdout, OUTPUT_TAIL_BYTES);
        let stderr = bounded_redacted_tail(&output.stderr, OUTPUT_TAIL_BYTES);
        fs::write(self.stdout_path(id)?, stdout)
            .map_err(|error| io_error("write runtime job stdout log", &error))?;
        fs::write(self.stderr_path(id)?, stderr)
            .map_err(|error| io_error("write runtime job stderr log", &error))
    }

    fn write_cancel_marker(&self, id: &str) -> JobResult<()> {
        let marker = CancelMarker {
            requested_at_ms: now_millis(),
        };
        let bytes = serde_json::to_vec(&marker).map_err(|error| {
            redacted_error(&format!("serialize runtime job cancellation: {error}"))
        })?;
        atomic_write(&self.cancel_path(id)?, &bytes)
    }

    fn has_cancel_marker(&self, id: &str) -> JobResult<bool> {
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

    fn snapshot_with_cancel_intent(
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

    fn list(&self) -> RuntimeJobList {
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

    fn stale(&self, record: &RuntimeJobRecord) -> bool {
        let Some(heartbeat) = record.heartbeat_at_ms else {
            return false;
        };
        let threshold = duration_millis(self.stale_after);
        now_millis().saturating_sub(heartbeat) > threshold
    }
}

/// Durable job worker harness. A public transport adapter is deliberately outside this module.
struct ActiveRuntimeJobProcess {
    process: Box<dyn RuntimeJobProcess>,
    attempt_argv: Vec<String>,
    fallback: Option<RuntimeJobRequest>,
    previous_output: Option<RuntimeJobOutput>,
}

pub(crate) struct RuntimeJobService {
    store: RuntimeJobStore,
    runner: Arc<dyn RuntimeJobRunner>,
    processes: Mutex<HashMap<String, ActiveRuntimeJobProcess>>,
}

impl RuntimeJobService {
    pub(crate) fn new(cache_root: impl Into<PathBuf>, runner: Arc<dyn RuntimeJobRunner>) -> Self {
        Self::with_stale_after(cache_root, runner, DEFAULT_STALE_AFTER)
    }

    fn with_stale_after(
        cache_root: impl Into<PathBuf>,
        runner: Arc<dyn RuntimeJobRunner>,
        stale_after: Duration,
    ) -> Self {
        Self {
            store: RuntimeJobStore::new(cache_root, stale_after),
            runner,
            processes: Mutex::new(HashMap::new()),
        }
    }

    #[cfg(test)]
    fn start(&self, request: RuntimeJobRequest) -> JobResult<RuntimeJobSnapshot> {
        let id = Uuid::new_v4().to_string();
        self.store.enqueue(&id, &request)?;
        self.activate_enqueued(&id, &request)
    }

    pub(crate) fn enqueue(
        cache_root: impl Into<PathBuf>,
        request: &RuntimeJobRequest,
    ) -> JobResult<RuntimeJobSnapshot> {
        let store = RuntimeJobStore::new(cache_root, DEFAULT_STALE_AFTER);
        let id = Uuid::new_v4().to_string();
        let record = store.enqueue(&id, request)?;
        Ok(record.snapshot(false))
    }

    fn activate_enqueued(
        &self,
        id: &str,
        request: &RuntimeJobRequest,
    ) -> JobResult<RuntimeJobSnapshot> {
        self.activate_enqueued_after_preflight(id, request, || {})
    }

    fn activate_enqueued_after_preflight(
        &self,
        id: &str,
        request: &RuntimeJobRequest,
        after_preflight: impl FnOnce(),
    ) -> JobResult<RuntimeJobSnapshot> {
        self.activate_enqueued_after_hooks(id, request, after_preflight, || {})
    }

    fn activate_enqueued_after_hooks(
        &self,
        id: &str,
        request: &RuntimeJobRequest,
        after_preflight: impl FnOnce(),
        before_activation_guard: impl FnOnce(),
    ) -> JobResult<RuntimeJobSnapshot> {
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
        if record.operation != request.operation.label() {
            return Err(redacted_error(
                "runtime job worker operation does not match queued job",
            ));
        }
        let preflight_result = match &request.build_preflight {
            Some(build_preflight) => build_preflight.reauthorize_current_workspace(),
            None => Ok(()),
        };
        after_preflight();
        before_activation_guard();

        // Recovery and cancellation are separate processes. Linearize the
        // queued-to-running claim with both of them, then re-read durable state
        // under the same guard that is held through spawn and publication.
        let _lifecycle_lock = self.store.acquire_active_lifecycle_lock()?;
        record = self.store.read_record(id)?;
        if let Some(cancelled) = self.cancel_enqueued_before_start_guarded(&mut record)? {
            return Ok(cancelled);
        }
        if record.phase != RuntimeJobPhase::Queued {
            return Err(redacted_error("runtime job worker expected a queued job"));
        }
        if record.operation != request.operation.label() {
            return Err(redacted_error(
                "runtime job worker operation does not match queued job",
            ));
        }
        // The first reauthorization may have completed while cancellation or
        // recovery owned the lifecycle guard. Repeat it after claiming the
        // activation boundary so stale evidence cannot cross into spawn.
        let preflight_result = preflight_result.and_then(|()| match &request.build_preflight {
            Some(build_preflight) => build_preflight.reauthorize_current_workspace(),
            None => Ok(()),
        });
        let spawn_result = preflight_result.and_then(|()| self.runner.spawn(request));
        let mut process = match spawn_result {
            Ok(process) => process,
            Err(error) => {
                let error = redacted_error(&error);
                let _ = self.fail_start_guarded(&mut record, &error);
                return Err(error);
            }
        };
        record.pid = Some(process.id());
        record.pid_identity = Some(format!("pid:{}", process.id()));
        record.started_at_ms = Some(now_millis());
        record.heartbeat_at_ms = Some(now_millis());
        record.transition(RuntimeJobPhase::Running)?;
        if let Err(error) = self.store.write_record(&record) {
            self.cleanup_activation_failure_guarded(&mut record, &mut *process, &error);
            return Err(error);
        }
        let mut processes = match self.lock_processes() {
            Ok(processes) => processes,
            Err(error) => {
                self.cleanup_activation_failure_guarded(&mut record, &mut *process, &error);
                return Err(error);
            }
        };
        let mut request_with_fallback = request.clone();
        let fallback = request_with_fallback.take_full_rebuild_fallback();
        processes.insert(
            id.to_string(),
            ActiveRuntimeJobProcess {
                process,
                attempt_argv: request.raw_argv.clone(),
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

    #[cfg(test)]
    fn status(&self, id: &str) -> JobResult<RuntimeJobSnapshot> {
        Self::status_at(&self.store.cache_root, id)
    }

    pub(crate) fn status_at(
        cache_root: impl Into<PathBuf>,
        id: &str,
    ) -> JobResult<RuntimeJobSnapshot> {
        let store = RuntimeJobStore::new(cache_root, DEFAULT_STALE_AFTER);
        let _ = store.recover_stale_active()?;
        let record = store.read_record(id)?;
        store.snapshot_with_cancel_intent(&record, false)
    }

    pub(crate) fn poll(&self, id: &str) -> JobResult<RuntimeJobSnapshot> {
        let _lifecycle_lock = self.store.acquire_active_lifecycle_lock()?;
        let mut record = self.store.read_record(id)?;
        if record.phase.is_terminal() {
            return Ok(record.snapshot(false));
        }
        if self.store.stale(&record) {
            record.transition(RuntimeJobPhase::Lost)?;
            record.finished_at_ms = Some(now_millis());
            record.warnings.push("stale heartbeat".to_string());
            self.store.write_record(&record)?;
            self.remove_process(&record.id)?;
            return Ok(record.snapshot(false));
        }

        let cancel_requested = self.store.has_cancel_marker(id)?;
        let request_safe_cancel = cancel_requested
            && record.cancel_policy == CancelPolicy::Safe
            && !record.cancel_attempted;
        if request_safe_cancel {
            if record.phase == RuntimeJobPhase::Running {
                record.transition(RuntimeJobPhase::CancelRequested)?;
            }
            record.cancel_attempted = true;
            record.heartbeat_at_ms = Some(now_millis());
            self.store.write_record(&record)?;
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
            self.store.write_record(&record)?;
        }
        let (process_state, output, attempt_argv) =
            self.observe_process(&record, request_safe_cancel)?;
        self.store.write_logs(&record.id, &output)?;

        match process_state {
            RuntimeJobProcessState::Running => {
                record.heartbeat_at_ms = Some(now_millis());
                self.store.write_record(&record)?;
                Ok(record.snapshot(false))
            }
            RuntimeJobProcessState::Exited { exit_code } => {
                if exit_code != 0
                    && !cancel_requested
                    && output.fallback_receipt.as_deref().is_some_and(|stdout| {
                        classify_partial_platform_failure(&BuildAttempt {
                            argv: &attempt_argv,
                            process_exit_code: Some(exit_code),
                            status_success: false,
                            timed_out: false,
                            cancelled: false,
                            stdout_truncated: output.fallback_receipt_truncated,
                            stdout,
                        })
                        .is_some()
                    })
                {
                    if let Some(snapshot) =
                        self.start_full_build_fallback_guarded(&mut record, &output, exit_code)?
                    {
                        return Ok(snapshot);
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
                self.finish_guarded(&mut record, phase, Some(exit_code), None)
            }
            RuntimeJobProcessState::TimedOut { reason } => self.finish_guarded(
                &mut record,
                RuntimeJobPhase::TimedOut,
                None,
                Some(redact_text(&reason)),
            ),
        }
    }

    #[cfg(test)]
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

    #[cfg(test)]
    fn logs(&self, id: &str) -> JobResult<RuntimeJobLogs> {
        Self::logs_at(&self.store.cache_root, id, OUTPUT_TAIL_BYTES)
    }

    pub(crate) fn logs_at(
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

    #[cfg(test)]
    fn cancel(&self, id: &str) -> JobResult<RuntimeJobSnapshot> {
        let record = self.store.read_record(id)?;
        if record.phase.is_terminal() {
            return Ok(record.snapshot(false));
        }
        self.store.write_cancel_marker(id)?;
        self.poll(id)
    }

    #[cfg(test)]
    fn list(&self) -> RuntimeJobList {
        self.store.list()
    }

    pub(crate) fn list_at(cache_root: impl Into<PathBuf>) -> RuntimeJobList {
        let store = RuntimeJobStore::new(cache_root, DEFAULT_STALE_AFTER);
        let recovery_warning = store.recover_stale_active().err();
        let mut list = store.list();
        if let Some(warning) = recovery_warning {
            list.warnings.push(warning);
        }
        list
    }

    pub(crate) fn request_cancel_at(
        cache_root: impl Into<PathBuf>,
        id: &str,
    ) -> JobResult<RuntimeJobSnapshot> {
        let store = RuntimeJobStore::new(cache_root, DEFAULT_STALE_AFTER);
        let _lifecycle_lock = store.acquire_active_lifecycle_lock()?;
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

    pub(crate) fn wait_at(
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
    ) -> JobResult<(RuntimeJobProcessState, RuntimeJobOutput, Vec<String>)> {
        let mut processes = self.lock_processes()?;
        if !processes.contains_key(&record.id) {
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
                    attempt_argv: record.redacted_argv.clone(),
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
            output = combine_runtime_job_outputs(previous, &output);
        }
        Ok((state, output, active.attempt_argv.clone()))
    }

    fn start_full_build_fallback_guarded(
        &self,
        record: &mut RuntimeJobRecord,
        first_output: &RuntimeJobOutput,
        first_exit_code: i32,
    ) -> JobResult<Option<RuntimeJobSnapshot>> {
        let fallback = {
            let mut processes = self.lock_processes()?;
            let active = processes.get_mut(&record.id).ok_or_else(|| {
                redacted_error(&format!("runtime job {} process is unavailable", record.id))
            })?;
            active.fallback.take()
        };
        let Some(fallback) = fallback else {
            return Ok(None);
        };

        if let Some(preflight) = &fallback.build_preflight {
            if let Err(error) = preflight.reauthorize_current_workspace() {
                record.warnings.push(redact_text(&format!(
                    "full rebuild fallback was not started because build identity changed: {error}"
                )));
                return self
                    .finish_guarded(record, RuntimeJobPhase::Failed, Some(first_exit_code), None)
                    .map(Some);
            }
        }

        let mut next = match self.runner.spawn(&fallback) {
            Ok(process) => process,
            Err(error) => {
                record.warnings.push(redacted_error(&format!(
                    "full rebuild fallback was not started because v8-runner failed to spawn: {error}"
                )));
                return self
                    .finish_guarded(record, RuntimeJobPhase::Failed, Some(first_exit_code), None)
                    .map(Some);
            }
        };
        record.pid = Some(next.id());
        record.pid_identity = Some(format!("pid:{}", next.id()));
        record.redacted_argv = redact_argv(&fallback.raw_argv);
        record.heartbeat_at_ms = Some(now_millis());
        record.warnings.push(PARTIAL_FALLBACK_WARNING.to_string());
        if let Err(error) = self.store.write_record(record) {
            self.cleanup_activation_failure_guarded(record, &mut *next, &error);
            return Err(error);
        }

        let mut processes = match self.lock_processes() {
            Ok(processes) => processes,
            Err(error) => {
                self.cleanup_activation_failure_guarded(record, &mut *next, &error);
                return Err(error);
            }
        };
        let Some(active) = processes.get_mut(&record.id) else {
            let error =
                redacted_error(&format!("runtime job {} process is unavailable", record.id));
            drop(processes);
            self.cleanup_activation_failure_guarded(record, &mut *next, &error);
            return Err(error);
        };
        active.process = next;
        active.attempt_argv = fallback.raw_argv.clone();
        let mut previous_output = first_output.clone();
        previous_output.fallback_receipt = None;
        previous_output.fallback_receipt_truncated = false;
        active.previous_output = Some(previous_output);
        Ok(Some(record.snapshot(false)))
    }

    fn finish_guarded(
        &self,
        record: &mut RuntimeJobRecord,
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
        self.store.write_record(record)?;
        self.store.release_active_lock_guarded(&record.id, || {})?;
        self.remove_process(&record.id)?;
        Ok(record.snapshot(false))
    }

    fn fail_start_guarded(&self, record: &mut RuntimeJobRecord, error: &str) -> JobResult<()> {
        record.transition(RuntimeJobPhase::Failed)?;
        record.finished_at_ms = Some(now_millis());
        record.warnings.push(redact_text(error));
        self.store.write_record(record)?;
        self.store.release_active_lock_guarded(&record.id, || {})
    }

    fn cleanup_activation_failure_guarded(
        &self,
        record: &mut RuntimeJobRecord,
        process: &mut dyn RuntimeJobProcess,
        activation_error: &str,
    ) {
        match cancel_and_reap(process) {
            Ok(()) => {
                if record.phase == RuntimeJobPhase::Running {
                    let _ = record.transition(RuntimeJobPhase::Failed);
                }
                record.finished_at_ms = Some(now_millis());
                record.heartbeat_at_ms = Some(now_millis());
                record.warnings.push(redact_text(&format!(
                    "worker activation failed after child spawn: {activation_error}"
                )));
                if self.store.write_record(record).is_ok() {
                    let _ = self.store.release_active_lock_guarded(&record.id, || {});
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
                let _ = self.store.write_record(record);
            }
        }
    }

    fn append_warning(&self, id: &str, warning: &str) -> JobResult<()> {
        let mut record = self.store.read_record(id)?;
        if record.phase.is_terminal() {
            record.warnings.push(redact_text(warning));
            self.store.write_record(&record)?;
        }
        Ok(())
    }

    fn lock_processes(
        &self,
    ) -> JobResult<std::sync::MutexGuard<'_, HashMap<String, ActiveRuntimeJobProcess>>> {
        self.processes
            .lock()
            .map_err(|error| redacted_error(&format!("lock runtime job processes: {error}")))
    }

    fn remove_process(&self, id: &str) -> JobResult<()> {
        let mut processes = self.lock_processes()?;
        processes.remove(id);
        Ok(())
    }
}

pub(crate) fn run_worker_from_args(_args: &[String]) -> Result<(), String> {
    let mut stdin = io::stdin().lock();
    let handoff: WorkerStartRequest = read_worker_frame(&mut stdin, "runtime job worker request")?;
    let commit = read_worker_commit(&mut stdin, &handoff)?;
    if commit.cancelled {
        cancel_queued_job(&handoff.cache_root, &handoff.job_id)?;
        return Ok(());
    }
    let runner = Arc::new(SystemRuntimeJobRunner {
        program: handoff.program.clone(),
        cwd: handoff.cwd.clone(),
    });
    run_worker_request(handoff, runner)
}

fn read_worker_commit(
    reader: &mut impl std::io::BufRead,
    handoff: &WorkerStartRequest,
) -> JobResult<WorkerStartCommit> {
    match read_worker_frame(reader, "runtime job worker commit") {
        Ok(commit) => Ok(commit),
        Err(error) => {
            fail_queued_job(&handoff.cache_root, &handoff.job_id, &error)?;
            Err(error)
        }
    }
}

pub(crate) fn start_detached_worker(
    cache_root: PathBuf,
    program: PathBuf,
    cwd: PathBuf,
    request: RuntimeJobRequest,
    cancellation: &CancellationToken,
) -> JobResult<RuntimeJobSnapshot> {
    request.validate_build_preflight()?;
    if cancellation.is_cancelled() {
        return Err(cancelled_error(
            "runtime job start stopped before durable enqueue",
        ));
    }
    let queued = enqueue_cancellable_runtime_job(&cache_root, &request, cancellation)?;
    let handoff = WorkerStartRequest::new(
        cache_root.clone(),
        queued.id.clone(),
        program,
        cwd,
        &request,
    );
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
    let lifecycle_lock = match store.acquire_active_lifecycle_lock() {
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
        .and_then(|mut stdin| {
            write_worker_handoff_after_request(&mut stdin, &handoff, cancellation, || {})
        });
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

fn write_worker_handoff_after_request(
    writer: &mut impl Write,
    handoff: &WorkerStartRequest,
    cancellation: &CancellationToken,
    after_request: impl FnOnce(),
) -> JobResult<bool> {
    write_worker_frame(writer, handoff, "runtime job worker request")?;
    after_request();
    let commit_cancelled = cancellation.is_cancelled();
    write_worker_frame(
        writer,
        &WorkerStartCommit {
            cancelled: commit_cancelled,
        },
        "runtime job worker commit",
    )?;
    // The token may flip while serde or the pipe flush is in progress. The
    // caller holds the activation lifecycle guard and will publish this late
    // observation durably before releasing the worker.
    Ok(commit_cancelled || cancellation.is_cancelled())
}

fn write_worker_frame(
    writer: &mut impl Write,
    value: &impl Serialize,
    label: &str,
) -> JobResult<()> {
    serde_json::to_writer(&mut *writer, value)
        .map_err(|error| redacted_error(&format!("write {label}: {error}")))?;
    writer
        .write_all(b"\n")
        .map_err(|error| io_error(&format!("write {label} delimiter"), &error))?;
    writer
        .flush()
        .map_err(|error| io_error(&format!("flush {label}"), &error))
}

#[cfg(test)]
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

fn read_worker_frame<T: for<'de> Deserialize<'de>>(
    reader: &mut impl std::io::BufRead,
    label: &str,
) -> JobResult<T> {
    let mut frame = String::new();
    reader
        .read_line(&mut frame)
        .map_err(|error| io_error(&format!("read {label}"), &error))?;
    if frame.is_empty() {
        return Err(redacted_error(&format!("read {label}: unexpected EOF")));
    }
    serde_json::from_str(&frame).map_err(|error| redacted_error(&format!("read {label}: {error}")))
}

fn run_worker_request(
    handoff: WorkerStartRequest,
    runner: Arc<dyn RuntimeJobRunner>,
) -> Result<(), String> {
    let job_id = canonical_job_id(&handoff.job_id)?;
    let request = match handoff.runtime_request() {
        Ok(request) => request,
        Err(error) => {
            fail_queued_job(&handoff.cache_root, &job_id, &error)?;
            return Err(error);
        }
    };
    let worker_cwd = handoff.cwd.clone();
    let operation = request.operation.label();
    let service = RuntimeJobService::new(handoff.cache_root, runner);
    service.activate_enqueued(&job_id, &request)?;

    loop {
        let snapshot = service.poll(&job_id)?;
        if snapshot.phase.is_terminal() {
            if snapshot.phase == RuntimeJobPhase::Succeeded {
                if let Err(error) = apply_runtime_success_effects(&worker_cwd, operation, &job_id) {
                    let _ = service.append_warning(&job_id, &error);
                }
            }
            return Ok(());
        }
        thread::sleep(Duration::from_millis(25));
    }
}

fn apply_runtime_success_effects(cwd: &Path, operation: &str, job_id: &str) -> JobResult<()> {
    let Some(event_kind) = runtime_event_kind(operation) else {
        return Ok(());
    };
    let context = discover_workspace(Some(cwd.to_path_buf()))?;
    let events = vec![DomainEvent::new(
        event_kind,
        format!("runtime-job:{job_id}"),
    )];
    let report = WorkspaceStateRepository::new(&context).report(
        &context,
        &events,
        false,
        CacheAccess {
            reads: &[],
            writes: &["workspace_graph", "metadata_graph"],
        },
    )?;
    WorkspaceServiceManager::new().notify_invalidation(&context, &events);
    if report.publication_warnings.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "runtime cache state committed with cleanup warning: {}",
            report.publication_warnings.join("; ")
        ))
    }
}

fn fail_queued_job(cache_root: &Path, id: &str, error: &str) -> JobResult<()> {
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

fn cancel_queued_job(cache_root: &Path, id: &str) -> JobResult<()> {
    cancel_queued_job_after_guard(cache_root, id, || {})
}

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

fn enqueue_cancellable_runtime_job(
    cache_root: &Path,
    request: &RuntimeJobRequest,
    cancellation: &CancellationToken,
) -> JobResult<RuntimeJobSnapshot> {
    enqueue_cancellable_runtime_job_after_hook(cache_root, request, cancellation, || {})
}

fn enqueue_cancellable_runtime_job_after_hook(
    cache_root: &Path,
    request: &RuntimeJobRequest,
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

fn cancel_and_reap(process: &mut dyn RuntimeJobProcess) -> JobResult<()> {
    const REAP_TIMEOUT: Duration = Duration::from_secs(5);

    process.cancel()?;
    let deadline = Instant::now()
        .checked_add(REAP_TIMEOUT)
        .unwrap_or_else(Instant::now);
    loop {
        match process.try_wait()? {
            RuntimeJobProcessState::Exited { .. } | RuntimeJobProcessState::TimedOut { .. } => {
                // The process has been reaped. Reader failures cannot revive it,
                // so they do not make lock release unsafe.
                let _ = process.output_tails(OUTPUT_TAIL_BYTES);
                return Ok(());
            }
            RuntimeJobProcessState::Running if Instant::now() >= deadline => {
                return Err(redacted_error(
                    "runtime job process did not exit after cancellation request",
                ));
            }
            RuntimeJobProcessState::Running => thread::sleep(Duration::from_millis(10)),
        }
    }
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

fn redact_argv(argv: &[String]) -> Vec<String> {
    let mut redact_next = false;
    argv.iter()
        .map(|argument| {
            let lower = argument.trim_start_matches('-').to_ascii_lowercase();
            let redact_argument = redact_next;
            redact_next = matches!(
                lower.as_str(),
                "password" | "pwd" | "token" | "secret" | "connection" | "c"
            );
            if redact_argument || looks_like_connection_string(argument) {
                "<redacted>".to_string()
            } else {
                redact_text(argument)
            }
        })
        .collect()
}

fn looks_like_connection_string(argument: &str) -> bool {
    let lower = argument.to_ascii_lowercase();
    [
        "file=", "srvr=", "ref=", "usr=", "pwd=", "dbsrvr=", "dbname=",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

fn bounded_redacted_tail(text: &str, max_bytes: usize) -> String {
    bounded_tail(&redact_text(text), max_bytes)
}

fn append_tail(
    target: &Arc<Mutex<String>>,
    truncated: &std::sync::atomic::AtomicBool,
    addition: &str,
) -> io::Result<()> {
    let mut text = target
        .lock()
        .map_err(|_| io::Error::other("runtime job output lock is poisoned"))?;
    text.push_str(addition);
    if text.len() > OUTPUT_TAIL_BYTES {
        *text = bounded_tail(&text, OUTPUT_TAIL_BYTES);
        truncated.store(true, std::sync::atomic::Ordering::Release);
    }
    Ok(())
}

fn append_byte_tail(
    target: &Arc<Mutex<Vec<u8>>>,
    truncated: &std::sync::atomic::AtomicBool,
    addition: &[u8],
) -> io::Result<()> {
    let mut bytes = target
        .lock()
        .map_err(|_| io::Error::other("runtime job receipt lock is poisoned"))?;
    bytes.extend_from_slice(addition);
    if bytes.len() > FALLBACK_RECEIPT_BYTES {
        let keep_from = bytes.len().saturating_sub(FALLBACK_RECEIPT_BYTES);
        bytes.drain(..keep_from);
        truncated.store(true, std::sync::atomic::Ordering::Release);
    }
    Ok(())
}

fn combine_runtime_job_outputs(
    initial: &RuntimeJobOutput,
    fallback: &RuntimeJobOutput,
) -> RuntimeJobOutput {
    RuntimeJobOutput {
        stdout: format!(
            "--- initial partial attempt ---\n{}\n--- full rebuild fallback ---\n{}",
            initial.stdout, fallback.stdout
        ),
        stderr: format!(
            "--- initial partial attempt ---\n{}\n--- full rebuild fallback ---\n{}",
            initial.stderr, fallback.stderr
        ),
        stdout_truncated: initial.stdout_truncated || fallback.stdout_truncated,
        fallback_receipt: fallback.fallback_receipt.clone(),
        fallback_receipt_truncated: fallback.fallback_receipt_truncated,
    }
}

fn bounded_tail(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    let mut start = text.len().saturating_sub(max_bytes);
    while start < text.len() && !text.is_char_boundary(start) {
        start = start.saturating_add(1);
    }
    text[start..].to_string()
}

fn bounded_char_tail(text: &str, max_chars: usize) -> String {
    let char_count = text.chars().count();
    if char_count <= max_chars {
        return text.to_string();
    }
    text.chars()
        .skip(char_count.saturating_sub(max_chars))
        .collect()
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

#[cfg(test)]
pub(crate) use tests::assert_system_cancellation_reaps_process_tree;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Map};
    use std::{
        collections::HashMap,
        io::Cursor,
        sync::{
            atomic::{AtomicU32, Ordering},
            mpsc,
        },
    };

    #[test]
    fn atomic_write_replaces_an_existing_runtime_job_record() {
        let cache = TestCache::new();
        let path = cache.path().join("record.json");
        fs::create_dir_all(cache.path()).expect("create cache");

        atomic_write(&path, br#"{"phase":"queued"}"#).expect("create record");
        atomic_write(&path, br#"{"phase":"running"}"#).expect("replace record");

        assert_eq!(
            fs::read(&path).expect("read replaced record"),
            br#"{"phase":"running"}"#
        );
    }

    #[test]
    fn long_success_survives_reconnect_from_a_new_service_instance() {
        let cache = TestCache::new();
        let runner = Arc::new(FakeRunner::success_after(2));
        let service = RuntimeJobService::new(cache.path(), runner.clone());

        let job = service
            .start(fake_request(RuntimeJobOperation::Test))
            .expect("start job");
        assert_eq!(
            service.poll(&job.id).expect("first poll").phase,
            RuntimeJobPhase::Running
        );

        let reconnected = RuntimeJobService::new(cache.path(), runner);
        assert_eq!(
            reconnected.poll(&job.id).expect("reconnected poll").phase,
            RuntimeJobPhase::Succeeded
        );
    }

    #[test]
    fn detached_worker_owns_the_queued_record_until_terminal_state() {
        let cache = TestCache::new();
        let request = fake_request(RuntimeJobOperation::Test);
        let queued = RuntimeJobService::enqueue(cache.path(), &request).expect("queue job");
        assert_eq!(queued.phase, RuntimeJobPhase::Queued);

        run_worker_request(
            worker_request(&cache, &queued.id, &request),
            Arc::new(FakeRunner::success_after(2)),
        )
        .expect("worker completes job");

        let reconnected =
            RuntimeJobService::new(cache.path(), Arc::new(FakeRunner::success_after(1)));
        let terminal = reconnected.status(&queued.id).expect("read terminal job");
        assert_eq!(terminal.phase, RuntimeJobPhase::Succeeded);
        assert_eq!(terminal.operation, "test");
        assert!(terminal.started_at_ms.is_some());
        assert!(terminal.finished_at_ms.is_some());
        assert!(!reconnected.store.active_lock_path().exists());
    }

    #[test]
    fn worker_stream_tail_redacts_output_before_retaining_it() {
        let mut tail = StreamTail::spawn(Cursor::new(
            b"build started\nPwd=stream-secret\ncompleted\n".to_vec(),
        ));
        tail.finish().expect("finish output reader");
        let (output, truncated) = tail.tail(OUTPUT_TAIL_BYTES).expect("read output tail");
        let (receipt, receipt_truncated) =
            tail.receipt().expect("inspect disabled receipt capture");

        assert!(!truncated);
        assert!(receipt.is_none());
        assert!(!receipt_truncated);
        assert!(output.contains("Pwd=<redacted>"));
        assert!(!output.contains("stream-secret"));
    }

    #[test]
    fn truncated_worker_stdout_cannot_authorize_full_fallback() {
        let receipt = completed_partial_failure_json();
        let mut padded_receipt = receipt.clone();
        padded_receipt.push_str(&" ".repeat(FALLBACK_RECEIPT_BYTES - receipt.len()));
        let mut bytes = b"discarded-prefix".to_vec();
        bytes.extend_from_slice(padded_receipt.as_bytes());
        let mut tail = StreamTail::spawn_with_receipt(Cursor::new(bytes));
        tail.finish().expect("finish output reader");
        let (output, truncated) = tail.tail(OUTPUT_TAIL_BYTES).expect("read output tail");
        let (receipt, receipt_truncated) = tail.receipt().expect("read classification receipt");
        let argv = vec!["--json-message".to_string(), "build".to_string()];

        assert_eq!(output.len(), OUTPUT_TAIL_BYTES);
        assert!(truncated);
        assert!(receipt.is_none());
        assert!(receipt_truncated);
        assert!(
            classify_partial_platform_failure(&BuildAttempt {
                argv: &argv,
                process_exit_code: Some(4),
                status_success: false,
                timed_out: false,
                cancelled: false,
                stdout_truncated: truncated,
                stdout: &output,
            })
            .is_none(),
            "a valid JSON tail must not hide discarded stdout bytes"
        );
    }

    #[test]
    fn redacted_worker_logs_do_not_corrupt_transient_fallback_evidence() {
        let receipt = completed_partial_failure_json()
            .replace("/tmp/partial.lst", "/tmp/token=durable-secret/partial.lst");
        let mut tail = StreamTail::spawn_with_receipt(Cursor::new(receipt.into_bytes()));
        tail.finish().expect("finish output reader");
        let (output, truncated) = tail.tail(OUTPUT_TAIL_BYTES).expect("read output tail");
        let (receipt, receipt_truncated) = tail.receipt().expect("read classification receipt");
        let receipt = receipt.expect("complete raw receipt");
        let argv = vec!["--json-message".to_string(), "build".to_string()];

        assert!(!output.contains("durable-secret"));
        assert!(!truncated);
        assert!(!receipt_truncated);
        assert!(
            classify_partial_platform_failure(&BuildAttempt {
                argv: &argv,
                process_exit_code: Some(4),
                status_success: false,
                timed_out: false,
                cancelled: false,
                stdout_truncated: receipt_truncated,
                stdout: &receipt,
            })
            .is_some(),
            "redaction for persisted logs must not change transient retry evidence"
        );
    }

    #[test]
    fn redacted_tail_truncation_does_not_reject_a_complete_raw_receipt() {
        let expanded_log = "token=x,".repeat(600);
        let receipt = completed_partial_failure_json().replace("sanitized", &expanded_log);
        assert!(receipt.len() <= OUTPUT_TAIL_BYTES);
        let mut tail = StreamTail::spawn_with_receipt(Cursor::new(receipt.into_bytes()));
        tail.finish().expect("finish output reader");
        let (_, redacted_truncated) = tail.tail(OUTPUT_TAIL_BYTES).expect("read redacted tail");
        let (receipt, receipt_truncated) = tail.receipt().expect("read classification receipt");
        let receipt = receipt.expect("complete raw receipt");
        let argv = vec!["--json-message".to_string(), "build".to_string()];

        assert!(redacted_truncated);
        assert!(!receipt_truncated);
        assert!(
            classify_partial_platform_failure(&BuildAttempt {
                argv: &argv,
                process_exit_code: Some(4),
                status_success: false,
                timed_out: false,
                cancelled: false,
                stdout_truncated: receipt_truncated,
                stdout: &receipt,
            })
            .is_some(),
            "only truncation of the raw receipt may reject retry evidence"
        );
    }

    #[test]
    fn durable_fallback_accepts_a_receipt_within_the_sync_capture_limit() {
        let expanded_log = "x".repeat(OUTPUT_TAIL_BYTES);
        let receipt = completed_partial_failure_json().replace("sanitized", &expanded_log);
        assert!(receipt.len() > OUTPUT_TAIL_BYTES);
        assert!(receipt.len() < FALLBACK_RECEIPT_BYTES);

        let mut tail = StreamTail::spawn_with_receipt(Cursor::new(receipt.into_bytes()));
        tail.finish().expect("finish output reader");
        let (receipt, receipt_truncated) = tail.receipt().expect("read classification receipt");
        let receipt = receipt.expect("complete raw receipt");
        let argv = vec!["--json-message".to_string(), "build".to_string()];

        assert!(!receipt_truncated);
        assert!(
            classify_partial_platform_failure(&BuildAttempt {
                argv: &argv,
                process_exit_code: Some(4),
                status_success: false,
                timed_out: false,
                cancelled: false,
                stdout_truncated: receipt_truncated,
                stdout: &receipt,
            })
            .is_some(),
            "durable classification must accept every complete receipt that sync capture accepts"
        );
    }

    #[test]
    fn raw_receipt_preserves_utf8_split_across_stream_chunks() {
        let template = completed_partial_failure_json();
        let marker = template.find("sanitized").expect("platform log marker");
        let padding = 4095usize.saturating_sub(marker);
        let receipt = template.replace("sanitized", &format!("{}я", "a".repeat(padding)));
        assert!(receipt.len() <= OUTPUT_TAIL_BYTES);
        let mut tail = StreamTail::spawn_with_receipt(Cursor::new(receipt.into_bytes()));
        tail.finish().expect("finish output reader");
        let (receipt, truncated) = tail.receipt().expect("read classification receipt");
        let receipt = receipt.expect("complete UTF-8 receipt");
        let argv = vec!["--json-message".to_string(), "build".to_string()];

        assert!(!truncated);
        assert!(receipt.contains('я'));
        assert!(classify_partial_platform_failure(&BuildAttempt {
            argv: &argv,
            process_exit_code: Some(4),
            status_success: false,
            timed_out: false,
            cancelled: false,
            stdout_truncated: false,
            stdout: &receipt,
        })
        .is_some());
    }

    #[test]
    fn second_start_reports_the_active_job_id_as_a_conflict() {
        let cache = TestCache::new();
        let runner = Arc::new(FakeRunner::success_after(50));
        let service = RuntimeJobService::new(cache.path(), runner);
        let first = service
            .start(fake_request(RuntimeJobOperation::Test))
            .expect("start first job");

        let error = service
            .start(fake_request(RuntimeJobOperation::Test))
            .expect_err("second active job must be rejected");

        assert!(error.contains(&first.id), "{error}");
        assert_eq!(
            service.status(&first.id).expect("read first job").phase,
            RuntimeJobPhase::Running
        );
    }

    #[test]
    fn stale_job_becomes_lost_without_removing_a_replacement_active_lock() {
        let cache = TestCache::new();
        let runner = Arc::new(FakeRunner::success_after(50));
        let service =
            RuntimeJobService::with_stale_after(cache.path(), runner, Duration::from_millis(1));
        let stale = service
            .start(fake_request(RuntimeJobOperation::Test))
            .expect("start stale job");
        let mut record = service
            .store
            .read_record(&stale.id)
            .expect("read stale record");
        record.heartbeat_at_ms = Some(0);
        service.store.write_record(&record).expect("age record");

        let replacement = Uuid::new_v4().to_string();
        fs::write(service.store.active_lock_path(), &replacement).expect("replace active lock");

        let lost = service.poll(&stale.id).expect("poll stale job");

        assert_eq!(lost.phase, RuntimeJobPhase::Lost);
        assert_eq!(
            fs::read_to_string(service.store.active_lock_path())
                .expect("read replacement active lock")
                .trim(),
            replacement
        );
        assert_eq!(
            service
                .store
                .read_record(&stale.id)
                .expect("read lost record")
                .snapshot(false)
                .phase,
            RuntimeJobPhase::Lost
        );
    }

    #[test]
    fn stale_queued_job_is_lost_but_retains_the_active_lock() {
        let cache = TestCache::new();
        let request = fake_request(RuntimeJobOperation::Build);
        let queued = RuntimeJobService::enqueue(cache.path(), &request).expect("queue stale job");
        let store = RuntimeJobStore::new(cache.path(), DEFAULT_STALE_AFTER);
        let mut record = store.read_record(&queued.id).expect("read queued record");
        record.heartbeat_at_ms = Some(0);
        store.write_record(&record).expect("age queued record");

        let service = RuntimeJobService::new(cache.path(), Arc::new(FakeRunner::success_after(2)));
        let recovered = RuntimeJobService::status_at(cache.path(), &queued.id)
            .expect("status recovers stale job");
        let error = service
            .start(fake_request(RuntimeJobOperation::Test))
            .expect_err("a possibly live orphan must continue to block replacement work");

        assert_eq!(recovered.phase, RuntimeJobPhase::Lost);
        assert!(error.contains(&queued.id), "{error}");
        assert_eq!(
            fs::read_to_string(store.active_lock_path())
                .expect("read retained active lock")
                .trim(),
            queued.id
        );
        assert!(!store.recovery_lock_path().exists());
    }

    #[test]
    fn recovery_releases_its_own_lock_for_a_persisted_terminal_job() {
        let cache = TestCache::new();
        let request = fake_request(RuntimeJobOperation::Test);
        let queued = RuntimeJobService::enqueue(cache.path(), &request).expect("queue job");
        let store = RuntimeJobStore::new(cache.path(), DEFAULT_STALE_AFTER);
        let mut record = store.read_record(&queued.id).expect("read queued record");
        record
            .transition(RuntimeJobPhase::Failed)
            .expect("mark terminal record");
        record.finished_at_ms = Some(now_millis());
        store
            .write_record(&record)
            .expect("persist terminal record");

        let terminal =
            RuntimeJobService::status_at(cache.path(), &queued.id).expect("recover terminal lock");

        assert_eq!(terminal.phase, RuntimeJobPhase::Failed);
        assert!(!store.active_lock_path().exists());
    }

    pub(crate) fn assert_system_cancellation_reaps_process_tree(
        program: PathBuf,
        args: Vec<String>,
        process_tree_is_alive: impl FnOnce(u32) -> bool,
    ) {
        let cache = TestCache::new();
        fs::create_dir_all(cache.path()).expect("create worker cwd");
        let runner = SystemRuntimeJobRunner {
            program,
            cwd: cache.path().to_path_buf(),
        };
        let request = RuntimeJobRequest::new(
            RuntimeJobOperation::Test,
            args,
            "workspace:test".to_string(),
            None,
        );
        let mut process = runner.spawn(&request).expect("spawn process group");
        let process_id = process.id();

        cancel_and_reap(&mut *process).expect("cancel and reap process group");

        assert!(
            !process_tree_is_alive(process_id),
            "the process tree must no longer be alive"
        );
    }

    #[test]
    fn terminal_snapshot_and_persistence_are_redacted_and_keep_log_artifacts() {
        const ARGV_SECRET: &str = "argv-secret";
        const TARGET_SECRET: &str = "target-secret";
        const ARTIFACT_SECRET: &str = "artifact-secret";
        const STDOUT_SECRET: &str = "stdout-secret";
        const STDERR_SECRET: &str = "stderr-secret";

        let cache = TestCache::new();
        let runner = Arc::new(FakeRunner::exits_after(
            1,
            17,
            "stdout token=stdout-secret\n",
            "stderr password=stderr-secret\n",
        ));
        let service = RuntimeJobService::new(cache.path(), runner);
        let request = RuntimeJobRequest::new(
            RuntimeJobOperation::Test,
            vec![
                "runner".to_string(),
                "--token".to_string(),
                ARGV_SECRET.to_string(),
            ],
            "workspace:token=target-secret",
            Some("artifacts/token=artifact-secret".to_string()),
        );
        let job = service.start(request).expect("start job");

        let terminal = service.poll(&job.id).expect("finish job");
        let repeated = service.poll(&job.id).expect("read terminal job again");
        let logs = service.logs(&job.id).expect("read redacted logs");
        let snapshot_json = serde_json::to_string(&terminal).expect("serialize snapshot");
        let record_json =
            fs::read_to_string(service.store.record_path(&job.id).expect("record path"))
                .expect("read serialized record");

        assert_eq!(terminal.phase, RuntimeJobPhase::Failed);
        assert_eq!(terminal.operation, "test");
        assert_eq!(terminal.exit_code, Some(17));
        assert!(terminal.started_at_ms.is_some());
        assert_eq!(repeated.phase, terminal.phase);
        assert_eq!(repeated.exit_code, terminal.exit_code);
        assert_eq!(repeated.finished_at_ms, terminal.finished_at_ms);
        assert!(terminal.artifact_path.is_some());
        assert!(terminal.stdout_path.ends_with("stdout.log"));
        assert!(terminal.stderr_path.ends_with("stderr.log"));
        assert!(terminal.redacted_argv.iter().any(|arg| arg == "<redacted>"));
        assert!(logs.stdout.contains("<redacted>"));
        assert!(logs.stderr.contains("<redacted>"));

        for secret in [
            ARGV_SECRET,
            TARGET_SECRET,
            ARTIFACT_SECRET,
            STDOUT_SECRET,
            STDERR_SECRET,
        ] {
            assert!(!snapshot_json.contains(secret), "snapshot leaked {secret}");
            assert!(!record_json.contains(secret), "record leaked {secret}");
            assert!(!logs.stdout.contains(secret), "stdout leaked {secret}");
            assert!(!logs.stderr.contains(secret), "stderr leaked {secret}");
        }
    }

    #[test]
    fn direct_status_rejects_corrupt_unknown_schema_and_non_uuid_without_touching_active_lock() {
        let cache = TestCache::new();
        let runner = Arc::new(FakeRunner::success_after(50));
        let service = RuntimeJobService::new(cache.path(), runner);
        let fresh = service
            .start(fake_request(RuntimeJobOperation::Test))
            .expect("start fresh job");

        let corrupt_id = Uuid::new_v4().to_string();
        let corrupt_path = service
            .store
            .record_path(&corrupt_id)
            .expect("corrupt record path");
        fs::create_dir_all(corrupt_path.parent().expect("corrupt record directory"))
            .expect("create corrupt record directory");
        fs::write(&corrupt_path, "{ token=corrupt-secret").expect("write corrupt record");

        let schema_id = Uuid::new_v4().to_string();
        let mut unsupported = service
            .store
            .read_record(&fresh.id)
            .expect("read fresh record");
        unsupported.id = schema_id.clone();
        unsupported.schema_version = RECORD_SCHEMA_VERSION.saturating_add(1);
        service
            .store
            .write_record(&unsupported)
            .expect("write unsupported schema record");

        let corrupt_error = service
            .status(&corrupt_id)
            .expect_err("corrupt status must fail");
        let schema_error = service
            .status(&schema_id)
            .expect_err("unknown schema status must fail");
        let id_error = service
            .status("not-a-uuid")
            .expect_err("non-UUID status must fail");

        assert!(corrupt_error.contains("corrupt"), "{corrupt_error}");
        assert!(!corrupt_error.contains("corrupt-secret"), "{corrupt_error}");
        assert!(
            schema_error.contains("unsupported schema version"),
            "{schema_error}"
        );
        assert!(id_error.contains("UUID"), "{id_error}");
        assert_eq!(
            fs::read_to_string(service.store.active_lock_path())
                .expect("read fresh active lock")
                .trim(),
            fresh.id
        );
    }

    #[test]
    fn list_skips_a_corrupt_record_and_redacts_its_warning() {
        let cache = TestCache::new();
        let service = RuntimeJobService::new(cache.path(), Arc::new(FakeRunner::success_after(1)));
        let corrupt_id = Uuid::new_v4().to_string();
        let corrupt_path = service
            .store
            .record_path(&corrupt_id)
            .expect("corrupt record path");
        fs::create_dir_all(corrupt_path.parent().expect("corrupt record directory"))
            .expect("create corrupt record directory");
        fs::write(&corrupt_path, "{ token=list-secret").expect("write corrupt record");

        let list = service.list();

        assert!(list.jobs.is_empty(), "{list:?}");
        assert_eq!(list.warnings.len(), 1, "{list:?}");
        assert!(list.warnings[0].contains("corrupt"), "{list:?}");
        assert!(!list.warnings[0].contains("list-secret"), "{list:?}");
    }

    #[test]
    fn long_failure_is_persisted_with_its_exit_code() {
        let cache = TestCache::new();
        let runner = Arc::new(FakeRunner::exits_after(2, 23, "", "compile failed"));
        let service = RuntimeJobService::new(cache.path(), runner);
        let job = service
            .start(fake_request(RuntimeJobOperation::Test))
            .expect("start job");

        assert_eq!(
            service.poll(&job.id).expect("first poll").phase,
            RuntimeJobPhase::Running
        );
        let terminal = service.poll(&job.id).expect("terminal poll");

        assert_eq!(terminal.phase, RuntimeJobPhase::Failed);
        assert_eq!(terminal.exit_code, Some(23));
        assert!(terminal.finished_at_ms.is_some());
    }

    #[test]
    fn failed_partial_build_restarts_once_as_full_under_the_same_job() {
        let cache = TestCache::new();
        let request = fallback_build_request(&cache.path());
        let runner = Arc::new(SequenceRunner::new(vec![
            FakeProcessState {
                polls_until_exit: 1,
                result: FakeResult::Exit(4),
                stdout: completed_partial_failure_json(),
                stderr: String::new(),
                cancel_calls: 0,
            },
            FakeProcessState {
                polls_until_exit: 1,
                result: FakeResult::Exit(0),
                stdout: "full build completed".to_string(),
                stderr: String::new(),
                cancel_calls: 0,
            },
        ]));
        let service = RuntimeJobService::new(cache.path(), runner.clone());

        let started = service.start(request).expect("start partial build");
        let retried = service.poll(&started.id).expect("start full fallback");

        assert_eq!(retried.phase, RuntimeJobPhase::Running);
        assert_ne!(retried.pid, started.pid);
        assert!(service.store.active_lock_path().exists());
        let completed = service.poll(&started.id).expect("finish full fallback");
        assert_eq!(completed.phase, RuntimeJobPhase::Succeeded);
        assert!(!service.store.active_lock_path().exists());
        let requests = runner.requests.lock().expect("lock recorded requests");
        assert_eq!(requests.len(), 2);
        assert!(!requests[0].iter().any(|arg| arg == "--full-rebuild"));
        assert!(requests[1].iter().any(|arg| arg == "--full-rebuild"));
        drop(requests);
        let logs = service.logs(&started.id).expect("read combined logs");
        assert!(logs.stdout.contains("--- initial partial attempt ---"));
        assert!(logs.stdout.contains("partial load list path"));
        assert!(logs.stdout.contains("--- full rebuild fallback ---"));
        assert!(logs.stdout.contains("full build completed"));
        assert!(completed
            .warnings
            .iter()
            .any(|warning| warning == PARTIAL_FALLBACK_WARNING));
    }

    #[test]
    fn durable_fallback_classifies_with_the_unredacted_source_set() {
        let cache = TestCache::new();
        let mut request = fallback_build_request(&cache.path());
        request
            .raw_argv
            .extend(["--source-set".to_string(), "file=configuration".to_string()]);
        request.full_rebuild_fallback_argv = full_rebuild_argv(&request.raw_argv);
        let receipt = completed_partial_failure_json()
            .replace("source-set 'main'", "source-set 'file=configuration'")
            .replace(
                "\"source_set\":\"main\"",
                "\"source_set\":\"file=configuration\"",
            );
        let runner = Arc::new(SequenceRunner::new(vec![
            FakeProcessState {
                polls_until_exit: 1,
                result: FakeResult::Exit(4),
                stdout: receipt,
                stderr: String::new(),
                cancel_calls: 0,
            },
            FakeProcessState {
                polls_until_exit: 1,
                result: FakeResult::Exit(0),
                stdout: "full build completed".to_string(),
                stderr: String::new(),
                cancel_calls: 0,
            },
        ]));
        let service = RuntimeJobService::new(cache.path(), runner.clone());

        let started = service.start(request).expect("start partial build");
        let retried = service.poll(&started.id).expect("start full fallback");

        assert_eq!(retried.phase, RuntimeJobPhase::Running);
        assert_eq!(runner.requests.lock().expect("lock requests").len(), 2);
    }

    #[test]
    fn durable_fallback_never_persists_or_retains_the_raw_receipt() {
        const RECEIPT_SECRET: &str = "durable-receipt-secret";

        let cache = TestCache::new();
        let receipt = completed_partial_failure_json().replace(
            "/tmp/partial.lst",
            &format!("/tmp/token={RECEIPT_SECRET}/partial.lst"),
        );
        let runner = Arc::new(SequenceRunner::new(vec![
            FakeProcessState {
                polls_until_exit: 1,
                result: FakeResult::Exit(4),
                stdout: receipt,
                stderr: String::new(),
                cancel_calls: 0,
            },
            FakeProcessState {
                polls_until_exit: 1,
                result: FakeResult::Exit(0),
                stdout: "full build completed".to_string(),
                stderr: String::new(),
                cancel_calls: 0,
            },
        ]));
        let service = RuntimeJobService::new(cache.path(), runner.clone());

        let started = service
            .start(fallback_build_request(&cache.path()))
            .expect("start partial build");
        assert_eq!(
            service.poll(&started.id).expect("start fallback").phase,
            RuntimeJobPhase::Running
        );
        {
            let processes = service.processes.lock().expect("lock active processes");
            let previous = processes
                .get(&started.id)
                .and_then(|active| active.previous_output.as_ref())
                .expect("retain redacted first attempt");
            assert!(previous.fallback_receipt.is_none());
        }
        let record =
            fs::read_to_string(service.store.record_path(&started.id).expect("record path"))
                .expect("read record");
        let logs = service.logs(&started.id).expect("read logs");

        assert_eq!(runner.requests.lock().expect("lock requests").len(), 2);
        assert!(!record.contains(RECEIPT_SECRET));
        assert!(!logs.stdout.contains(RECEIPT_SECRET));
        assert!(logs.stdout.contains("<redacted>"));
    }

    #[test]
    fn durable_fallback_rejects_config_change_between_attempts() {
        let cache = TestCache::new();
        let request = fallback_build_request(&cache.path());
        let config = cache.path().join("workspace/v8project.yaml");
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let calls_for_hook = Arc::clone(&calls);
        let _hook = crate::infrastructure::runtime_build_preflight::set_before_reauthorization_hook_for_test(
            move || {
                if calls_for_hook.fetch_add(1, Ordering::SeqCst) == 2 {
                    fs::write(
                        &config,
                        "format: DESIGNER\nsource-set: []\n# changed before fallback\n",
                    )
                    .expect("change config before fallback reauthorization");
                }
            },
        );
        let runner = Arc::new(SequenceRunner::new(vec![FakeProcessState {
            polls_until_exit: 1,
            result: FakeResult::Exit(4),
            stdout: completed_partial_failure_json(),
            stderr: String::new(),
            cancel_calls: 0,
        }]));
        let service = RuntimeJobService::new(cache.path(), runner.clone());

        let started = service.start(request).expect("start partial build");
        let terminal = service
            .poll(&started.id)
            .expect("identity change terminalizes the original attempt");

        assert_eq!(terminal.phase, RuntimeJobPhase::Failed);
        assert_eq!(terminal.exit_code, Some(4));
        assert_eq!(runner.requests.lock().expect("lock requests").len(), 1);
        assert!(!service.store.active_lock_path().exists());
        assert!(terminal
            .warnings
            .iter()
            .any(|warning| warning.contains("identity changed")));
    }

    #[test]
    fn failed_full_fallback_is_not_retried_a_third_time() {
        let cache = TestCache::new();
        let runner = Arc::new(SequenceRunner::new(vec![
            FakeProcessState {
                polls_until_exit: 1,
                result: FakeResult::Exit(4),
                stdout: completed_partial_failure_json(),
                stderr: String::new(),
                cancel_calls: 0,
            },
            FakeProcessState {
                polls_until_exit: 1,
                result: FakeResult::Exit(4),
                stdout: completed_partial_failure_json(),
                stderr: String::new(),
                cancel_calls: 0,
            },
        ]));
        let service = RuntimeJobService::new(cache.path(), runner.clone());

        let started = service
            .start(fallback_build_request(&cache.path()))
            .expect("start partial build");
        assert_eq!(
            service.poll(&started.id).expect("start fallback").phase,
            RuntimeJobPhase::Running
        );
        assert_eq!(
            service.poll(&started.id).expect("finish fallback").phase,
            RuntimeJobPhase::Failed
        );
        assert_eq!(
            service
                .poll(&started.id)
                .expect("read terminal state")
                .phase,
            RuntimeJobPhase::Failed
        );
        assert_eq!(runner.requests.lock().expect("lock requests").len(), 2);
        assert!(!service.store.active_lock_path().exists());
    }

    #[test]
    fn unrelated_build_failure_does_not_start_full_fallback() {
        let cache = TestCache::new();
        let unrelated = completed_partial_failure_json().replace(
            "load failed for source-set",
            "update_db_cfg failed for source-set",
        );
        let runner = Arc::new(SequenceRunner::new(vec![FakeProcessState {
            polls_until_exit: 1,
            result: FakeResult::Exit(4),
            stdout: unrelated,
            stderr: String::new(),
            cancel_calls: 0,
        }]));
        let service = RuntimeJobService::new(cache.path(), runner.clone());

        let started = service
            .start(fallback_build_request(&cache.path()))
            .expect("start partial build");
        let completed = service.poll(&started.id).expect("observe failure");

        assert_eq!(completed.phase, RuntimeJobPhase::Failed);
        assert_eq!(runner.requests.lock().expect("lock requests").len(), 1);
        assert!(!service.store.active_lock_path().exists());
    }

    #[test]
    fn deferred_cancel_prevents_full_fallback_after_partial_failure() {
        let cache = TestCache::new();
        let runner = Arc::new(SequenceRunner::new(vec![FakeProcessState {
            polls_until_exit: 1,
            result: FakeResult::Exit(4),
            stdout: completed_partial_failure_json(),
            stderr: String::new(),
            cancel_calls: 0,
        }]));
        let service = RuntimeJobService::new(cache.path(), runner.clone());

        let started = service
            .start(fallback_build_request(&cache.path()))
            .expect("start partial build");
        let completed = service
            .cancel(&started.id)
            .expect("cancel marker must win before fallback starts");

        assert_eq!(completed.phase, RuntimeJobPhase::Failed);
        assert!(completed.cancel_deferred);
        assert_eq!(runner.requests.lock().expect("lock requests").len(), 1);
        assert!(!service.store.active_lock_path().exists());
    }

    #[test]
    fn fallback_spawn_failure_terminalizes_the_same_job() {
        let cache = TestCache::new();
        let runner = Arc::new(SequenceRunner::new(vec![FakeProcessState {
            polls_until_exit: 1,
            result: FakeResult::Exit(4),
            stdout: completed_partial_failure_json(),
            stderr: String::new(),
            cancel_calls: 0,
        }]));
        let service = RuntimeJobService::new(cache.path(), runner.clone());

        let started = service
            .start(fallback_build_request(&cache.path()))
            .expect("start partial build");
        let completed = service
            .poll(&started.id)
            .expect("fallback spawn error is a terminal job outcome");

        assert_eq!(completed.phase, RuntimeJobPhase::Failed);
        assert!(completed
            .warnings
            .iter()
            .any(|warning| warning.contains("failed to spawn")));
        assert!(completed
            .warnings
            .iter()
            .any(|warning| warning.contains("fallback was not started")));
        assert!(!completed
            .warnings
            .iter()
            .any(|warning| warning == PARTIAL_FALLBACK_WARNING));
        assert_eq!(runner.requests.lock().expect("lock requests").len(), 1);
        assert!(!service.store.active_lock_path().exists());
    }

    #[test]
    fn runner_timeout_becomes_terminal_without_a_process_exit_code() {
        let cache = TestCache::new();
        let runner = Arc::new(FakeRunner::times_out_after(1, "runner timeout"));
        let service = RuntimeJobService::new(cache.path(), runner);
        let job = service
            .start(fake_request(RuntimeJobOperation::Test))
            .expect("start job");

        let terminal = service.poll(&job.id).expect("observe timeout");

        assert_eq!(terminal.phase, RuntimeJobPhase::TimedOut);
        assert_eq!(terminal.exit_code, None);
        assert_eq!(terminal.timeout_reason.as_deref(), Some("runner timeout"));
        assert!(!service.store.active_lock_path().exists());
    }

    #[test]
    fn caller_wait_timeout_does_not_stop_the_active_job() {
        let cache = TestCache::new();
        let runner = Arc::new(FakeRunner::success_after(3));
        let service = RuntimeJobService::new(cache.path(), runner);
        let job = service
            .start(fake_request(RuntimeJobOperation::Test))
            .expect("start job");

        let waiting = service.wait(&job.id, Duration::ZERO).expect("wait once");

        assert_eq!(waiting.phase, RuntimeJobPhase::Running);
        assert!(waiting.wait_timed_out);
        assert_eq!(
            service.status(&job.id).expect("status").phase,
            RuntimeJobPhase::Running
        );
        assert_eq!(
            service.poll(&job.id).expect("second poll").phase,
            RuntimeJobPhase::Running
        );
        assert_eq!(
            service.poll(&job.id).expect("third poll").phase,
            RuntimeJobPhase::Succeeded
        );
    }

    #[test]
    fn safe_cancel_calls_the_process_and_becomes_cancelled() {
        let cache = TestCache::new();
        let runner = Arc::new(FakeRunner::success_after(50));
        let service = RuntimeJobService::new(cache.path(), runner.clone());
        let job = service
            .start(fake_request(RuntimeJobOperation::Test))
            .expect("start job");

        let cancelled = service.cancel(&job.id).expect("cancel job");

        assert_eq!(cancelled.phase, RuntimeJobPhase::Cancelled);
        assert!(cancelled.cancelled);
        assert_eq!(cancelled.unsafe_phase, None);
        let process_id = job.pid.expect("persisted fake pid");
        assert_eq!(runner.cancel_calls(process_id).expect("cancel calls"), 1);
        assert!(!service.store.active_lock_path().exists());
    }

    #[test]
    fn critical_cancel_is_deferred_and_the_process_keeps_being_observed() {
        let cache = TestCache::new();
        let runner = Arc::new(FakeRunner::success_after(2));
        let service = RuntimeJobService::new(cache.path(), runner.clone());
        let job = service
            .start(fake_request(RuntimeJobOperation::Build))
            .expect("start job");

        let deferred = service.cancel(&job.id).expect("request cancel");

        assert_eq!(deferred.phase, RuntimeJobPhase::CancelRequested);
        assert!(deferred.cancel_deferred);
        assert_eq!(deferred.unsafe_phase.as_deref(), Some("build"));
        let process_id = job.pid.expect("persisted fake pid");
        assert_eq!(runner.cancel_calls(process_id).expect("cancel calls"), 0);
        assert_eq!(
            service.poll(&job.id).expect("observe completion").phase,
            RuntimeJobPhase::Succeeded
        );
    }

    #[test]
    fn detached_critical_cancel_publishes_deferred_state_before_a_worker_polls() {
        let cache = TestCache::new();
        let request = fake_request(RuntimeJobOperation::Build);
        let queued = RuntimeJobService::enqueue(cache.path(), &request).expect("queue job");

        let deferred = RuntimeJobService::request_cancel_at(cache.path(), &queued.id)
            .expect("request durable cancellation");

        assert_eq!(deferred.phase, RuntimeJobPhase::CancelRequested);
        assert!(deferred.cancel_deferred);
        assert_eq!(deferred.unsafe_phase.as_deref(), Some("build"));
        assert!(!deferred.cancelled);
    }

    #[test]
    fn terminal_record_wins_over_a_preexisting_cancel_marker() {
        let cache = TestCache::new();
        let request = fake_request(RuntimeJobOperation::Build);
        let queued = RuntimeJobService::enqueue(cache.path(), &request).expect("queue job");
        let store = RuntimeJobStore::new(cache.path(), DEFAULT_STALE_AFTER);

        store
            .write_cancel_marker(&queued.id)
            .expect("publish cancellation intent");
        let mut terminal = store.read_record(&queued.id).expect("read queued record");
        terminal
            .transition(RuntimeJobPhase::Running)
            .expect("start job");
        terminal
            .transition(RuntimeJobPhase::Succeeded)
            .expect("finish job");
        terminal.finished_at_ms = Some(now_millis());
        store
            .write_record(&terminal)
            .expect("persist terminal result");
        store
            .release_active_lock_for(&queued.id)
            .expect("release active lock");

        let observed = RuntimeJobService::request_cancel_at(cache.path(), &queued.id)
            .expect("terminal cancellation is a no-op");
        assert_eq!(observed.phase, RuntimeJobPhase::Succeeded);
        assert_eq!(
            store
                .read_record(&queued.id)
                .expect("read terminal record")
                .phase,
            RuntimeJobPhase::Succeeded
        );
    }

    #[test]
    fn worker_does_not_spawn_a_job_cancelled_while_queued() {
        let cache = TestCache::new();
        let runner = Arc::new(FakeRunner::success_after(2));
        let service = RuntimeJobService::new(cache.path(), runner.clone());
        let request = fake_request(RuntimeJobOperation::Test);
        let queued = RuntimeJobService::enqueue(cache.path(), &request).expect("queue job");
        RuntimeJobService::request_cancel_at(cache.path(), &queued.id).expect("cancel queued job");

        let cancelled = service
            .activate_enqueued(&queued.id, &request)
            .expect("observe queued cancellation");

        assert_eq!(cancelled.phase, RuntimeJobPhase::Cancelled);
        assert!(cancelled.cancelled);
        assert!(cancelled.pid.is_none());
        assert!(!service.store.active_lock_path().exists());
        assert!(runner
            .processes
            .lock()
            .expect("lock fake processes")
            .is_empty());
    }

    #[test]
    fn worker_does_not_spawn_a_critical_job_cancelled_while_queued() {
        let cache = TestCache::new();
        let runner = Arc::new(FakeRunner::success_after(2));
        let service = RuntimeJobService::new(cache.path(), runner.clone());
        let request = fake_request(RuntimeJobOperation::Build);
        let queued = RuntimeJobService::enqueue(cache.path(), &request).expect("queue build job");
        service
            .store
            .write_cancel_marker(&queued.id)
            .expect("cancel queued critical job");

        let cancelled = service
            .activate_enqueued(&queued.id, &request)
            .expect("worker observes pre-start cancellation");

        assert_eq!(cancelled.phase, RuntimeJobPhase::Cancelled);
        assert!(cancelled.cancelled);
        assert!(cancelled.pid.is_none());
        assert!(!service.store.active_lock_path().exists());
        assert!(runner
            .processes
            .lock()
            .expect("lock fake processes")
            .is_empty());
    }

    #[test]
    fn worker_observes_cancellation_after_build_reauthorization_before_spawn() {
        let cache = TestCache::new();
        let workspace = cache.path().join("workspace");
        fs::create_dir_all(&workspace).expect("create workspace");
        fs::write(
            workspace.join("v8project.yaml"),
            "format: DESIGNER\nsource-set: []\n",
        )
        .expect("write workspace config");
        let context = discover_workspace(Some(workspace)).expect("discover workspace");
        let args = Map::from_iter([("operation".to_string(), json!("build"))]);
        let request = RuntimeJobRequest::new(
            RuntimeJobOperation::Build,
            vec!["build".to_string()],
            "workspace:test",
            None,
        )
        .with_build_preflight(Some(
            RuntimeBuildPreflight::capture(&args, &context).unwrap(),
        ));
        let state_root = cache.path().join("state");
        let runner = Arc::new(FakeRunner::success_after(1));
        let service = RuntimeJobService::new(state_root.clone(), runner.clone());
        let queued = RuntimeJobService::enqueue(&state_root, &request).expect("queue build job");

        let cancelled = service
            .activate_enqueued_after_preflight(&queued.id, &request, || {
                service
                    .store
                    .write_cancel_marker(&queued.id)
                    .expect("cancel after build reauthorization");
            })
            .expect("worker observes pre-spawn cancellation");

        assert_eq!(cancelled.phase, RuntimeJobPhase::Cancelled);
        assert!(cancelled.cancelled);
        assert!(cancelled.pid.is_none());
        assert!(!service.store.active_lock_path().exists());
        assert!(runner
            .processes
            .lock()
            .expect("lock fake processes")
            .is_empty());
    }

    #[test]
    fn worker_does_not_spawn_when_cancellation_wins_the_activation_race() {
        let cache = TestCache::new();
        let runner = Arc::new(FakeRunner::success_after(1));
        let service = RuntimeJobService::new(cache.path(), runner.clone());
        let request = fake_request(RuntimeJobOperation::Build);
        let queued = RuntimeJobService::enqueue(cache.path(), &request).expect("queue build job");

        let cancelled = service
            .activate_enqueued_after_hooks(
                &queued.id,
                &request,
                || {},
                || {
                    service
                        .store
                        .write_cancel_marker(&queued.id)
                        .expect("cancel before the worker claims activation");
                },
            )
            .expect("cancellation must win before process activation");

        assert_eq!(cancelled.phase, RuntimeJobPhase::Cancelled);
        assert!(cancelled.cancelled);
        assert!(cancelled.pid.is_none());
        assert!(!service.store.active_lock_path().exists());
        assert!(runner
            .processes
            .lock()
            .expect("lock fake processes")
            .is_empty());
    }

    #[test]
    fn worker_does_not_resurrect_job_recovered_during_build_reauthorization() {
        let cache = TestCache::new();
        let workspace = cache.path().join("workspace");
        fs::create_dir_all(&workspace).expect("create workspace");
        fs::write(
            workspace.join("v8project.yaml"),
            "format: DESIGNER\nsource-set: []\n",
        )
        .expect("write workspace config");
        let context = discover_workspace(Some(workspace)).expect("discover workspace");
        let args = Map::from_iter([("operation".to_string(), json!("build"))]);
        let request = RuntimeJobRequest::new(
            RuntimeJobOperation::Build,
            vec!["build".to_string()],
            "workspace:test",
            None,
        )
        .with_build_preflight(Some(
            RuntimeBuildPreflight::capture(&args, &context).unwrap(),
        ));
        let state_root = cache.path().join("state");
        let runner = Arc::new(FakeRunner::success_after(1));
        let service = RuntimeJobService::new(state_root.clone(), runner.clone());
        let queued = RuntimeJobService::enqueue(&state_root, &request).expect("queue build job");

        let error = service
            .activate_enqueued_after_preflight(&queued.id, &request, || {
                let mut stale = service
                    .store
                    .read_record(&queued.id)
                    .expect("read queued record during recovery");
                stale.heartbeat_at_ms = Some(0);
                service
                    .store
                    .write_record(&stale)
                    .expect("age queued record during preflight");
                assert!(service
                    .store
                    .recover_stale_active()
                    .expect("recover stale queued job"));
            })
            .expect_err("worker must not overwrite a recovered terminal state");

        assert!(error.contains("expected a queued job"), "{error}");
        assert_eq!(
            service
                .store
                .read_record(&queued.id)
                .expect("read recovered job")
                .phase,
            RuntimeJobPhase::Lost
        );
        assert!(service.store.active_lock_path().exists());
        assert!(runner
            .processes
            .lock()
            .expect("lock fake processes")
            .is_empty());
    }

    #[test]
    fn worker_rechecks_workspace_epoch_after_waiting_for_activation_guard() {
        let cache = TestCache::new();
        let workspace = cache.path().join("workspace");
        fs::create_dir_all(workspace.join(".git")).expect("create git metadata");
        fs::write(
            workspace.join("v8project.yaml"),
            "format: DESIGNER\nsource-set: []\n",
        )
        .expect("write workspace config");
        let head = workspace.join(".git/HEAD");
        fs::write(&head, "ref: refs/heads/feature-a\n").expect("write initial HEAD");
        let context = discover_workspace(Some(workspace)).expect("discover workspace");
        let args = Map::from_iter([("operation".to_string(), json!("build"))]);
        let request = RuntimeJobRequest::new(
            RuntimeJobOperation::Build,
            vec!["build".to_string()],
            "workspace:test",
            None,
        )
        .with_build_preflight(Some(
            RuntimeBuildPreflight::capture(&args, &context).unwrap(),
        ));
        let state_root = cache.path().join("state");
        let runner = Arc::new(FakeRunner::success_after(1));
        let service = RuntimeJobService::new(state_root.clone(), runner.clone());
        let queued = RuntimeJobService::enqueue(&state_root, &request).expect("queue build job");

        let error = service
            .activate_enqueued_after_preflight(&queued.id, &request, || {
                fs::write(&head, "ref: refs/heads/feature-b\n")
                    .expect("switch HEAD after the first reauthorization");
            })
            .expect_err("worker must recheck authorization at activation");

        assert!(error.contains("workspace identity changed"), "{error}");
        assert_eq!(
            service
                .store
                .read_record(&queued.id)
                .expect("read failed job")
                .phase,
            RuntimeJobPhase::Failed
        );
        assert!(!service.store.active_lock_path().exists());
        assert!(runner
            .processes
            .lock()
            .expect("lock fake processes")
            .is_empty());
    }

    #[test]
    fn worker_build_authorization_does_not_depend_on_support_state_changes() {
        let cache = TestCache::new();
        let workspace = cache.path().join("workspace");
        let source_root = workspace.join("src");
        fs::create_dir_all(&source_root).expect("create source root");
        fs::write(
            workspace.join("v8project.yaml"),
            concat!(
                "format: DESIGNER\n",
                "source-set:\n",
                "  - name: main\n",
                "    type: CONFIGURATION\n",
                "    path: src\n",
            ),
        )
        .expect("write project config");
        fs::write(
            source_root.join("Configuration.xml"),
            include_bytes!(
                "../../../../tests/fixtures/platform_8_3_27/support-edit-bin-only/src/Configuration.xml"
            ),
        )
        .expect("write configuration root");
        let context = discover_workspace(Some(workspace.clone())).expect("discover workspace");
        let args = Map::from_iter([("operation".to_string(), json!("build"))]);
        let build_preflight = RuntimeBuildPreflight::capture(&args, &context).unwrap();
        build_preflight
            .reauthorize_current_workspace()
            .expect("support state must not affect normal build authorization");
        let marker = source_root.join("Ext/ParentConfigurations.bin");
        fs::create_dir_all(marker.parent().unwrap()).expect("create support directory");
        fs::write(
            marker,
            include_bytes!(
                "../../../../tests/fixtures/platform_8_3_27/support-edit-bin-only/src/Ext/ParentConfigurations.bin"
            ),
        )
        .expect("activate configuration support");
        let runner = Arc::new(FakeRunner::success_after(1));
        let state_root = cache.path().join("state");
        let service = RuntimeJobService::new(state_root.clone(), runner.clone());
        let request = RuntimeJobRequest::new(
            RuntimeJobOperation::Build,
            vec!["--json-message".to_string(), "build".to_string()],
            "workspace:test",
            None,
        )
        .with_build_preflight(Some(build_preflight));

        let running = service
            .start(request)
            .expect("support state must not select the build strategy");

        assert_eq!(running.phase, RuntimeJobPhase::Running);
        assert_eq!(
            runner.processes.lock().expect("lock fake processes").len(),
            1
        );
        assert_eq!(
            service.poll(&running.id).unwrap().phase,
            RuntimeJobPhase::Succeeded
        );
    }

    #[test]
    fn worker_handoff_preserves_incremental_build_authorization() {
        let cache = TestCache::new();
        let workspace = cache.path().join("workspace");
        fs::create_dir_all(&workspace).expect("create workspace");
        fs::write(
            workspace.join("v8project.yaml"),
            "format: DESIGNER\nsource-set: []\n",
        )
        .expect("write workspace config");
        let local_config = workspace.join("v8project.local.yaml");
        fs::write(
            &local_config,
            "workPath: local\ninfobase:\n  password: handoff-secret\n",
        )
        .expect("write local workspace config");
        let context = discover_workspace(Some(workspace)).expect("discover workspace");
        let args = Map::from_iter([
            ("operation".to_string(), json!("build")),
            ("sourceSet".to_string(), json!("main")),
        ]);
        let request = RuntimeJobRequest::new(
            RuntimeJobOperation::Build,
            vec![
                "--json-message".to_string(),
                "build".to_string(),
                "--source-set".to_string(),
                "main".to_string(),
            ],
            "workspace:test",
            None,
        )
        .with_build_preflight(Some(
            RuntimeBuildPreflight::capture(&args, &context).unwrap(),
        ));
        let handoff = worker_request(&cache, &Uuid::new_v4().to_string(), &request);

        let encoded = serde_json::to_vec(&handoff).expect("serialize handoff");
        assert!(!encoded
            .windows(b"handoff-secret".len())
            .any(|window| { window == b"handoff-secret" }));
        let decoded: WorkerStartRequest =
            serde_json::from_slice(&encoded).expect("deserialize handoff");
        let restored = decoded.runtime_request().expect("restore runtime request");

        assert_eq!(restored.build_preflight(), request.build_preflight());
        fs::write(&local_config, "workPath: changed\n").expect("change local workspace config");
        let error = restored
            .build_preflight()
            .expect("restored build preflight")
            .reauthorize_current_workspace()
            .expect_err("serialized authorization must retain the local overlay digest");
        assert!(error.contains("local project config changed"), "{error}");
    }

    #[test]
    fn worker_handoff_waits_for_commit_and_cancels_before_activation() {
        let cache = TestCache::new();
        let runner = Arc::new(FakeRunner::success_after(1));
        let request = fake_request(RuntimeJobOperation::Build);
        let queued = RuntimeJobService::enqueue(cache.path(), &request).expect("queue build job");
        let handoff = worker_request(&cache, &queued.id, &request);

        let mut framed = Vec::new();
        write_worker_frame(&mut framed, &handoff, "test worker request").unwrap();
        write_worker_frame(
            &mut framed,
            &WorkerStartCommit { cancelled: true },
            "test worker commit",
        )
        .unwrap();
        let mut reader = io::BufReader::new(framed.as_slice());
        let decoded: WorkerStartRequest =
            read_worker_frame(&mut reader, "test worker request").unwrap();
        let commit: WorkerStartCommit =
            read_worker_frame(&mut reader, "test worker commit").unwrap();
        if commit.cancelled {
            cancel_queued_job(&decoded.cache_root, &decoded.job_id).unwrap();
        } else {
            run_worker_request(decoded, runner.clone()).unwrap();
        }

        let store = RuntimeJobStore::new(cache.path(), DEFAULT_STALE_AFTER);
        let record = store.read_record(&queued.id).expect("read cancelled job");
        assert_eq!(record.phase, RuntimeJobPhase::Cancelled);
        assert!(record.cancelled);
        assert!(!store.active_lock_path().exists());
        assert!(runner
            .processes
            .lock()
            .expect("lock fake processes")
            .is_empty());
        assert!(worker_start_result(&cache.path(), queued, true)
            .expect_err("cancelled commit must not report a queued success")
            .starts_with("cancelled:"));
    }

    #[test]
    fn parent_terminalizes_a_cancelled_commit_without_worker_acknowledgement() {
        let cache = TestCache::new();
        let request = fake_request(RuntimeJobOperation::Build);
        let queued = RuntimeJobService::enqueue(cache.path(), &request).expect("queue build job");

        let error = worker_start_result(&cache.path(), queued.clone(), true)
            .expect_err("cancelled commit must not report a queued success");

        assert!(error.starts_with("cancelled:"), "{error}");
        let store = RuntimeJobStore::new(cache.path(), DEFAULT_STALE_AFTER);
        let record = store.read_record(&queued.id).expect("read cancelled job");
        assert_eq!(record.phase, RuntimeJobPhase::Cancelled);
        assert!(record.cancelled);
        assert!(!store.active_lock_path().exists());
    }

    #[test]
    fn cancel_cleanup_serializes_with_replacement_admission() {
        let cache = TestCache::new();
        let request = fake_request(RuntimeJobOperation::Build);
        let cancelled =
            RuntimeJobService::enqueue(cache.path(), &request).expect("queue cancelled job");
        let (guarded_tx, guarded_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();

        let cancel_cache = cache.path();
        let cancel_id = cancelled.id.clone();
        let cancellation = thread::spawn(move || {
            cancel_queued_job_after_guard(&cancel_cache, &cancel_id, || {
                guarded_tx.send(()).expect("signal guarded cancellation");
                release_rx.recv().expect("release guarded cancellation");
            })
        });

        guarded_rx.recv().expect("wait for guarded cancellation");
        let replacement_cache = cache.path();
        let replacement_request = request.clone();
        let (replacement_tx, replacement_rx) = mpsc::channel();
        let replacement = thread::spawn(move || {
            let result = RuntimeJobService::enqueue(replacement_cache, &replacement_request);
            replacement_tx
                .send(result)
                .expect("send replacement result");
        });

        assert!(replacement_rx
            .recv_timeout(Duration::from_millis(50))
            .is_err());
        release_tx.send(()).expect("resume guarded cancellation");
        cancellation
            .join()
            .expect("join cancellation")
            .expect("finish cancellation");
        let replacement_job = replacement_rx
            .recv()
            .expect("receive replacement result")
            .expect("queue replacement job");
        replacement.join().expect("join replacement admission");

        let error = RuntimeJobService::enqueue(cache.path(), &request)
            .expect_err("replacement job must retain exclusive admission");
        assert!(error.contains(&replacement_job.id), "{error}");
        let store = RuntimeJobStore::new(cache.path(), DEFAULT_STALE_AFTER);
        assert_eq!(
            fs::read_to_string(store.active_lock_path())
                .expect("read replacement active lock")
                .trim(),
            replacement_job.id
        );
    }

    #[test]
    fn active_lock_admission_and_release_hold_the_same_lifecycle_guard() {
        fn assert_contended(path: &Path) {
            let contender = OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)
                .expect("open lifecycle lock contender");
            let error = FileExt::try_lock_exclusive(&contender)
                .expect_err("lifecycle lock must remain held");
            assert!(lock_is_contended(&error), "unexpected lock error: {error}");
        }

        let cache = TestCache::new();
        let request = fake_request(RuntimeJobOperation::Build);
        let queued = RuntimeJobService::enqueue(cache.path(), &request).expect("queue job");
        let store = RuntimeJobStore::new(cache.path(), DEFAULT_STALE_AFTER);
        let lifecycle_path = store.active_lifecycle_lock_path();
        let (release_guarded_tx, release_guarded_rx) = mpsc::channel();
        let (resume_release_tx, resume_release_rx) = mpsc::channel();
        let release_store = store.clone();
        let release_id = queued.id.clone();
        let release = thread::spawn(move || {
            release_store.release_active_lock_for_after_hooks(
                &release_id,
                || {},
                || {
                    release_guarded_tx
                        .send(())
                        .expect("signal guarded release observation");
                    resume_release_rx.recv().expect("resume guarded release");
                },
            )
        });

        release_guarded_rx
            .recv()
            .expect("wait for guarded release observation");
        assert_contended(&lifecycle_path);
        resume_release_tx.send(()).expect("resume release");
        release
            .join()
            .expect("join guarded release")
            .expect("release active lock");

        let replacement_id = Uuid::new_v4().to_string();
        let (acquire_guarded_tx, acquire_guarded_rx) = mpsc::channel();
        let (resume_acquire_tx, resume_acquire_rx) = mpsc::channel();
        let acquire_store = store.clone();
        let acquired_id = replacement_id.clone();
        let acquire = thread::spawn(move || {
            acquire_store.acquire_active_lock_after_lifecycle(&acquired_id, || {
                acquire_guarded_tx
                    .send(())
                    .expect("signal guarded admission");
                resume_acquire_rx.recv().expect("resume guarded admission");
            })
        });

        acquire_guarded_rx
            .recv()
            .expect("wait for guarded admission");
        assert_contended(&lifecycle_path);
        assert!(!store.active_lock_path().exists());
        resume_acquire_tx.send(()).expect("resume admission");
        acquire
            .join()
            .expect("join guarded admission")
            .expect("acquire replacement active lock");
        assert_eq!(
            fs::read_to_string(store.active_lock_path())
                .expect("read replacement active lock")
                .trim(),
            replacement_id
        );
        store
            .release_active_lock_for(&replacement_id)
            .expect("release replacement active lock");
    }

    #[test]
    fn status_and_list_skip_recovery_while_lifecycle_transition_is_guarded() {
        let cache = TestCache::new();
        let request = fake_request(RuntimeJobOperation::Build);
        let queued = RuntimeJobService::enqueue(cache.path(), &request).expect("queue job");
        let store = RuntimeJobStore::new(cache.path(), DEFAULT_STALE_AFTER);
        let _lifecycle_lock = store
            .acquire_active_lifecycle_lock()
            .expect("guard lifecycle transition");

        let status_cache = cache.path();
        let status_id = queued.id.clone();
        let (status_tx, status_rx) = mpsc::channel();
        let status = thread::spawn(move || {
            status_tx
                .send(RuntimeJobService::status_at(status_cache, &status_id))
                .expect("send status result");
        });
        let observed = status_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("status must not wait for the lifecycle guard")
            .expect("read queued status");
        status.join().expect("join status reader");

        let list_cache = cache.path();
        let (list_tx, list_rx) = mpsc::channel();
        let list = thread::spawn(move || {
            list_tx
                .send(RuntimeJobService::list_at(list_cache))
                .expect("send list result");
        });
        let listed = list_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("list must not wait for the lifecycle guard");
        list.join().expect("join list reader");

        assert_eq!(observed.phase, RuntimeJobPhase::Queued);
        assert!(listed.warnings.is_empty());
        assert_eq!(listed.jobs.len(), 1);
        assert_eq!(listed.jobs[0].id, queued.id);
    }

    #[test]
    fn worker_request_without_commit_fails_the_queued_record() {
        let cache = TestCache::new();
        let request = fake_request(RuntimeJobOperation::Build);
        let queued = RuntimeJobService::enqueue(cache.path(), &request).expect("queue build job");
        let handoff = worker_request(&cache, &queued.id, &request);
        let mut framed = Vec::new();
        write_worker_frame(&mut framed, &handoff, "test worker request").unwrap();
        let mut reader = io::BufReader::new(framed.as_slice());
        let decoded: WorkerStartRequest =
            read_worker_frame(&mut reader, "test worker request").unwrap();

        let error = read_worker_commit(&mut reader, &decoded)
            .expect_err("EOF before commit must fail the queued job");

        assert!(error.contains("unexpected EOF"), "{error}");
        let store = RuntimeJobStore::new(cache.path(), DEFAULT_STALE_AFTER);
        let record = store.read_record(&queued.id).expect("read failed job");
        assert_eq!(record.phase, RuntimeJobPhase::Failed);
        assert!(!store.active_lock_path().exists());
    }

    #[test]
    fn worker_handoff_observes_cancellation_after_request_frame() {
        let cache = TestCache::new();
        let request = fake_request(RuntimeJobOperation::Build);
        let handoff = worker_request(&cache, &Uuid::new_v4().to_string(), &request);
        let cancellation = CancellationToken::new();
        let mut framed = Vec::new();

        let commit_cancelled =
            write_worker_handoff_after_request(&mut framed, &handoff, &cancellation, || {
                cancellation.cancel()
            })
            .expect("write worker handoff");

        assert!(commit_cancelled);
        let mut reader = io::BufReader::new(framed.as_slice());
        let _: WorkerStartRequest = read_worker_frame(&mut reader, "test worker request").unwrap();
        let commit: WorkerStartCommit =
            read_worker_frame(&mut reader, "test worker commit").unwrap();
        assert!(commit.cancelled);
    }

    #[test]
    fn worker_handoff_observes_cancellation_during_commit_write() {
        let cache = TestCache::new();
        let request = fake_request(RuntimeJobOperation::Build);
        let queued = RuntimeJobService::enqueue(cache.path(), &request).expect("queue build job");
        let handoff = worker_request(&cache, &queued.id, &request);
        let cancellation = CancellationToken::new();
        let mut framed = CancelDuringCommitWriter::new(cancellation.clone());
        let store = RuntimeJobStore::new(cache.path(), DEFAULT_STALE_AFTER);
        let lifecycle_lock = store
            .acquire_active_lifecycle_lock()
            .expect("guard activation while publishing commit");

        let cancellation_observed =
            write_worker_handoff_after_request(&mut framed, &handoff, &cancellation, || {})
                .expect("write worker handoff");

        assert!(
            cancellation_observed,
            "cancellation published while the commit frame was written must be observed"
        );
        let error = worker_start_result_guarded(&store, queued.clone(), cancellation_observed)
            .expect_err("late cancellation must terminalize the queued job");
        assert!(error.starts_with("cancelled:"), "{error}");
        drop(lifecycle_lock);

        let mut reader = io::BufReader::new(framed.bytes.as_slice());
        let decoded: WorkerStartRequest =
            read_worker_frame(&mut reader, "test worker request").unwrap();
        let commit: WorkerStartCommit =
            read_worker_frame(&mut reader, "test worker commit").unwrap();
        assert!(
            !commit.cancelled,
            "the regression must exercise cancellation after the serialized snapshot"
        );

        let runner = Arc::new(FakeRunner::success_after(1));
        run_worker_request(decoded, runner.clone())
            .expect_err("the stale worker commit must not reactivate a cancelled job");
        let record = store.read_record(&queued.id).expect("read cancelled job");
        assert_eq!(record.phase, RuntimeJobPhase::Cancelled);
        assert!(record.cancelled);
        assert!(!store.active_lock_path().exists());
        assert!(runner
            .processes
            .lock()
            .expect("lock fake processes")
            .is_empty());
    }

    #[test]
    fn handoff_write_error_after_commit_does_not_release_a_queued_worker() {
        let cache = TestCache::new();
        let request = fake_request(RuntimeJobOperation::Build);
        let queued = RuntimeJobService::enqueue(cache.path(), &request).expect("queue build job");
        let handoff = worker_request(&cache, &queued.id, &request);
        let cancellation = CancellationToken::new();
        let mut framed = FailSecondFlushWriter::default();
        let store = RuntimeJobStore::new(cache.path(), DEFAULT_STALE_AFTER);
        let lifecycle_lock = store
            .acquire_active_lifecycle_lock()
            .expect("guard activation while publishing commit");

        let handoff_result =
            write_worker_handoff_after_request(&mut framed, &handoff, &cancellation, || {});
        assert!(handoff_result.is_err());
        let mut terminated = false;
        settle_worker_handoff_guarded(&store, queued.clone(), handoff_result, || {
            terminated = true;
        })
        .expect_err("a handoff write error must fail the queued job");
        assert!(
            terminated,
            "the worker must be terminated before the activation guard is released"
        );
        drop(lifecycle_lock);

        // The complete false commit is still decodable. Durable terminalization
        // must prevent those stale bytes from activating a provider process.
        let mut reader = io::BufReader::new(framed.bytes.as_slice());
        let decoded: WorkerStartRequest =
            read_worker_frame(&mut reader, "test worker request").unwrap();
        let commit: WorkerStartCommit =
            read_worker_frame(&mut reader, "test worker commit").unwrap();
        assert!(!commit.cancelled);
        let runner = Arc::new(FakeRunner::success_after(1));

        let activation = run_worker_request(decoded, runner.clone());

        assert!(
            activation.is_err(),
            "a worker whose handoff failed must not activate from serialized bytes"
        );
        assert_eq!(runner.next_id.load(Ordering::Acquire), 100);
        let record = store.read_record(&queued.id).expect("read failed job");
        assert_eq!(record.phase, RuntimeJobPhase::Failed);
        assert!(!store.active_lock_path().exists());
    }

    #[test]
    fn handoff_terminalization_error_terminates_worker_before_unlock() {
        let cache = TestCache::new();
        let request = fake_request(RuntimeJobOperation::Build);
        let queued = RuntimeJobService::enqueue(cache.path(), &request).expect("queue build job");
        let store = RuntimeJobStore::new(cache.path(), DEFAULT_STALE_AFTER);
        let lifecycle_lock = store
            .acquire_active_lifecycle_lock()
            .expect("guard activation while terminalizing commit");
        fs::remove_file(store.record_path(&queued.id).expect("job record path"))
            .expect("inject terminalization failure");
        let mut terminated = false;

        settle_worker_handoff_guarded(&store, queued, Ok(true), || {
            terminated = true;
        })
        .expect_err("missing durable record must fail terminalization");

        assert!(
            terminated,
            "terminalization failure must stop the stale commit worker before unlock"
        );
        drop(lifecycle_lock);
    }

    #[test]
    fn normal_build_handoff_without_authorization_is_rejected() {
        let cache = TestCache::new();
        let request = RuntimeJobRequest::new(
            RuntimeJobOperation::Build,
            vec!["build".to_string()],
            "workspace:test",
            None,
        )
        .without_build_preflight_for_test();
        let handoff = worker_request(&cache, &Uuid::new_v4().to_string(), &request);

        let error = handoff
            .runtime_request()
            .expect_err("normal build handoff must fail closed");

        assert!(error.contains("missing prelaunch authorization"), "{error}");
        assert!(error.contains("fullRebuild: true"), "{error}");
    }

    #[test]
    fn invalid_normal_build_handoff_fails_the_queued_record() {
        let cache = TestCache::new();
        let request = RuntimeJobRequest::new(
            RuntimeJobOperation::Build,
            vec!["build".to_string()],
            "workspace:test",
            None,
        )
        .without_build_preflight_for_test();
        let queued = RuntimeJobService::enqueue(cache.path(), &request).expect("queue invalid job");
        let handoff = worker_request(&cache, &queued.id, &request);
        let runner = Arc::new(FakeRunner::success_after(1));

        let error = run_worker_request(handoff, runner.clone())
            .expect_err("invalid handoff must fail before process spawn");

        assert!(error.contains("missing prelaunch authorization"), "{error}");
        let store = RuntimeJobStore::new(cache.path(), DEFAULT_STALE_AFTER);
        let record = store.read_record(&queued.id).expect("read failed job");
        assert_eq!(record.phase, RuntimeJobPhase::Failed);
        assert!(!store.active_lock_path().exists());
        assert!(runner
            .processes
            .lock()
            .expect("lock fake processes")
            .is_empty());
    }

    #[test]
    fn cancellation_after_enqueue_terminalizes_job_before_worker_launch() {
        let cache = TestCache::new();
        let request = fake_request(RuntimeJobOperation::Build);
        let cancellation = CancellationToken::new();

        let error = enqueue_cancellable_runtime_job_after_hook(
            &cache.path(),
            &request,
            &cancellation,
            || cancellation.cancel(),
        )
        .expect_err("cancelled handoff must not launch a worker");

        assert!(error.starts_with("cancelled:"), "{error}");
        let store = RuntimeJobStore::new(cache.path(), DEFAULT_STALE_AFTER);
        let id = fs::read_dir(store.jobs_root())
            .expect("list job directory")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .find(|name| Uuid::parse_str(name).is_ok())
            .expect("cancelled job id");
        let record = store.read_record(&id).expect("read cancelled job record");
        assert_eq!(record.phase, RuntimeJobPhase::Cancelled);
        assert!(record.cancelled);
        assert!(!store.active_lock_path().exists());
    }

    #[test]
    fn worker_handoff_never_persists_actual_argv_or_output_secrets() {
        const REQUEST_SECRET: &str = "request-secret";
        const OUTPUT_SECRET: &str = "output-secret";

        let cache = TestCache::new();
        let request = RuntimeJobRequest::new(
            RuntimeJobOperation::Make,
            vec![
                "make".to_string(),
                "--connection".to_string(),
                format!("Pwd={REQUEST_SECRET}"),
            ],
            "workspace:test".to_string(),
            None,
        );
        let queued = RuntimeJobService::enqueue(cache.path(), &request).expect("queue job");
        let handoff = WorkerStartRequest::new(
            cache.path(),
            queued.id.clone(),
            PathBuf::from("fake-v8-runner"),
            cache.path(),
            &request,
        );
        let runner = Arc::new(FakeRunner::exits_after(
            0,
            0,
            &format!("token={OUTPUT_SECRET}\\n"),
            "",
        ));

        run_worker_request(handoff, runner).expect("run worker handoff");

        let store = RuntimeJobStore::new(cache.path(), DEFAULT_STALE_AFTER);
        let persisted = [
            fs::read_to_string(store.record_path(&queued.id).expect("record path"))
                .expect("read record"),
            fs::read_to_string(store.stdout_path(&queued.id).expect("stdout path"))
                .expect("read stdout"),
            fs::read_to_string(store.stderr_path(&queued.id).expect("stderr path"))
                .expect("read stderr"),
        ]
        .join("\\n");
        assert!(!persisted.contains(REQUEST_SECRET), "{persisted}");
        assert!(!persisted.contains(OUTPUT_SECRET), "{persisted}");
        assert!(persisted.contains("<redacted>"), "{persisted}");
    }

    #[test]
    fn persisted_command_redacts_a_launch_connection_string_completely() {
        let cache = TestCache::new();
        let service = RuntimeJobService::new(cache.path(), Arc::new(FakeRunner::success_after(1)));
        let job = service
            .start(RuntimeJobRequest::new(
                RuntimeJobOperation::Launch,
                vec![
                    "v8-runner".to_string(),
                    "launch".to_string(),
                    "--c".to_string(),
                    "Srvr=prod;Ref=finance;Usr=svc;Pwd=secret".to_string(),
                ],
                "workspace:demo",
                None,
            ))
            .expect("start launch job");
        let snapshot_json = serde_json::to_string(&job).expect("serialize snapshot");
        let record_json =
            fs::read_to_string(service.store.record_path(&job.id).expect("record path"))
                .expect("read record");

        for value in ["prod", "finance", "svc", "secret", "Srvr=", "Ref="] {
            assert!(!snapshot_json.contains(value), "snapshot leaked {value}");
            assert!(!record_json.contains(value), "record leaked {value}");
        }
        assert_eq!(job.redacted_argv[3], "<redacted>");
    }

    fn fake_request(operation: RuntimeJobOperation) -> RuntimeJobRequest {
        let mut argv = vec!["unica".to_string(), "test".to_string()];
        if operation == RuntimeJobOperation::Build {
            argv.push("--full-rebuild".to_string());
        }
        RuntimeJobRequest::new(operation, argv, "workspace:demo", None)
    }

    fn worker_request(
        cache: &TestCache,
        job_id: &str,
        request: &RuntimeJobRequest,
    ) -> WorkerStartRequest {
        WorkerStartRequest::new(
            cache.path(),
            job_id.to_string(),
            PathBuf::from("fake-runtime"),
            cache.path(),
            request,
        )
    }

    struct TestCache {
        root: PathBuf,
    }

    struct CancelDuringCommitWriter {
        bytes: Vec<u8>,
        cancellation: CancellationToken,
        completed_flushes: usize,
        cancelled: bool,
    }

    impl CancelDuringCommitWriter {
        fn new(cancellation: CancellationToken) -> Self {
            Self {
                bytes: Vec::new(),
                cancellation,
                completed_flushes: 0,
                cancelled: false,
            }
        }
    }

    impl io::Write for CancelDuringCommitWriter {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            if self.completed_flushes > 0 && !self.cancelled {
                self.cancellation.cancel();
                self.cancelled = true;
            }
            self.bytes.extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            self.completed_flushes += 1;
            Ok(())
        }
    }

    #[derive(Default)]
    struct FailSecondFlushWriter {
        bytes: Vec<u8>,
        completed_flushes: usize,
    }

    impl io::Write for FailSecondFlushWriter {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            self.bytes.extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            self.completed_flushes += 1;
            if self.completed_flushes == 2 {
                Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "injected commit flush failure",
                ))
            } else {
                Ok(())
            }
        }
    }

    impl TestCache {
        fn new() -> Self {
            Self {
                root: std::env::temp_dir().join(format!("unica-runtime-jobs-{}", Uuid::new_v4())),
            }
        }

        fn path(&self) -> PathBuf {
            self.root.clone()
        }
    }

    impl Drop for TestCache {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[derive(Clone)]
    struct FakeRunner {
        next_id: Arc<AtomicU32>,
        processes: Arc<Mutex<HashMap<u32, Arc<Mutex<FakeProcessState>>>>>,
        initial: FakeProcessState,
    }

    struct SequenceRunner {
        next_id: AtomicU32,
        states: Mutex<std::collections::VecDeque<FakeProcessState>>,
        processes: Arc<Mutex<HashMap<u32, Arc<Mutex<FakeProcessState>>>>>,
        requests: Mutex<Vec<Vec<String>>>,
    }

    impl SequenceRunner {
        fn new(states: Vec<FakeProcessState>) -> Self {
            Self {
                next_id: AtomicU32::new(100),
                states: Mutex::new(states.into()),
                processes: Arc::new(Mutex::new(HashMap::new())),
                requests: Mutex::new(Vec::new()),
            }
        }
    }

    impl RuntimeJobRunner for SequenceRunner {
        fn spawn(&self, request: &RuntimeJobRequest) -> JobResult<Box<dyn RuntimeJobProcess>> {
            let id = self.next_id.fetch_add(1, Ordering::Relaxed);
            let state = Arc::new(Mutex::new(
                self.states
                    .lock()
                    .map_err(|error| redacted_error(&format!("lock sequence: {error}")))?
                    .pop_front()
                    .ok_or_else(|| redacted_error("sequence runner exhausted"))?,
            ));
            self.requests
                .lock()
                .map_err(|error| redacted_error(&format!("lock requests: {error}")))?
                .push(request.raw_argv.clone());
            self.processes
                .lock()
                .map_err(|error| redacted_error(&format!("lock sequence processes: {error}")))?
                .insert(id, state.clone());
            Ok(Box::new(FakeProcess { id, state }))
        }

        fn attach(&self, process_id: u32) -> JobResult<Box<dyn RuntimeJobProcess>> {
            let state = self
                .processes
                .lock()
                .map_err(|error| redacted_error(&format!("lock sequence processes: {error}")))?
                .get(&process_id)
                .cloned()
                .ok_or_else(|| redacted_error("sequence process unavailable"))?;
            Ok(Box::new(FakeProcess {
                id: process_id,
                state,
            }))
        }
    }

    fn completed_partial_failure_json() -> String {
        let message = "load failed for source-set 'main' with exit code 1; platform log: sanitized; platform log path: /tmp/out.log; partial load list path: /tmp/partial.lst";
        serde_json::to_string(&json!({
            "ok": false,
            "command": "build",
            "duration_ms": 12,
            "data": {
                "ok": false,
                "steps": [{
                    "source_set": "main",
                    "mode": { "partial": { "file_count": 1 } },
                    "ok": false,
                    "message": format!("platform error: {message}"),
                    "duration_ms": 0
                }],
                "duration_ms": 12
            },
            "warnings": [],
            "steps": [],
            "error": {
                "code": "platform_failure",
                "kind": "platform",
                "message": message
            }
        }))
        .expect("serialize partial failure")
    }

    fn fallback_build_request(cache_root: &Path) -> RuntimeJobRequest {
        let workspace = cache_root.join("workspace");
        fs::create_dir_all(&workspace).expect("create workspace");
        fs::write(
            workspace.join("v8project.yaml"),
            "format: DESIGNER\nsource-set: []\n",
        )
        .expect("write config");
        let context = discover_workspace(Some(workspace)).expect("discover workspace");
        let args = Map::from_iter([("operation".to_string(), json!("build"))]);
        RuntimeJobRequest::new(
            RuntimeJobOperation::Build,
            vec![
                "--json-message".to_string(),
                "--config".to_string(),
                context
                    .workspace_root
                    .join("v8project.yaml")
                    .display()
                    .to_string(),
                "build".to_string(),
            ],
            "workspace:test",
            None,
        )
        .with_build_preflight(Some(
            RuntimeBuildPreflight::capture(&args, &context).expect("capture build identity"),
        ))
    }

    impl FakeRunner {
        fn success_after(polls: u32) -> Self {
            Self::exits_after(polls, 0, "done", "")
        }

        fn exits_after(polls: u32, exit_code: i32, stdout: &str, stderr: &str) -> Self {
            Self {
                next_id: Arc::new(AtomicU32::new(100)),
                processes: Arc::new(Mutex::new(HashMap::new())),
                initial: FakeProcessState {
                    polls_until_exit: polls,
                    result: FakeResult::Exit(exit_code),
                    stdout: stdout.to_string(),
                    stderr: stderr.to_string(),
                    cancel_calls: 0,
                },
            }
        }

        fn times_out_after(polls: u32, reason: &str) -> Self {
            Self {
                next_id: Arc::new(AtomicU32::new(100)),
                processes: Arc::new(Mutex::new(HashMap::new())),
                initial: FakeProcessState {
                    polls_until_exit: polls,
                    result: FakeResult::TimedOut(reason.to_string()),
                    stdout: String::new(),
                    stderr: String::new(),
                    cancel_calls: 0,
                },
            }
        }

        fn cancel_calls(&self, process_id: u32) -> JobResult<u32> {
            let process = self
                .processes
                .lock()
                .map_err(|error| redacted_error(&format!("lock fake runner: {error}")))?
                .get(&process_id)
                .cloned()
                .ok_or_else(|| redacted_error("fake process is unavailable"))?;
            let calls = process
                .lock()
                .map_err(|error| redacted_error(&format!("lock fake process: {error}")))?
                .cancel_calls;
            Ok(calls)
        }
    }

    impl RuntimeJobRunner for FakeRunner {
        fn spawn(&self, _request: &RuntimeJobRequest) -> JobResult<Box<dyn RuntimeJobProcess>> {
            let id = self.next_id.fetch_add(1, Ordering::Relaxed);
            let state = Arc::new(Mutex::new(self.initial.clone()));
            self.processes
                .lock()
                .map_err(|error| redacted_error(&format!("lock fake runner: {error}")))?
                .insert(id, state.clone());
            Ok(Box::new(FakeProcess { id, state }))
        }

        fn attach(&self, process_id: u32) -> JobResult<Box<dyn RuntimeJobProcess>> {
            let state = self
                .processes
                .lock()
                .map_err(|error| redacted_error(&format!("lock fake runner: {error}")))?
                .get(&process_id)
                .cloned()
                .ok_or_else(|| redacted_error("fake process is unavailable"))?;
            Ok(Box::new(FakeProcess {
                id: process_id,
                state,
            }))
        }
    }

    #[derive(Clone)]
    struct FakeProcessState {
        polls_until_exit: u32,
        result: FakeResult,
        stdout: String,
        stderr: String,
        cancel_calls: u32,
    }

    #[derive(Clone)]
    enum FakeResult {
        Exit(i32),
        TimedOut(String),
    }

    struct FakeProcess {
        id: u32,
        state: Arc<Mutex<FakeProcessState>>,
    }

    impl RuntimeJobProcess for FakeProcess {
        fn id(&self) -> u32 {
            self.id
        }

        fn try_wait(&mut self) -> JobResult<RuntimeJobProcessState> {
            let mut state = self
                .state
                .lock()
                .map_err(|error| redacted_error(&format!("lock fake process: {error}")))?;
            if state.polls_until_exit > 1 {
                state.polls_until_exit -= 1;
                return Ok(RuntimeJobProcessState::Running);
            }
            match &state.result {
                FakeResult::Exit(exit_code) => Ok(RuntimeJobProcessState::Exited {
                    exit_code: *exit_code,
                }),
                FakeResult::TimedOut(reason) => Ok(RuntimeJobProcessState::TimedOut {
                    reason: reason.clone(),
                }),
            }
        }

        fn cancel(&mut self) -> JobResult<()> {
            let mut state = self
                .state
                .lock()
                .map_err(|error| redacted_error(&format!("lock fake process: {error}")))?;
            state.cancel_calls = state.cancel_calls.saturating_add(1);
            state.polls_until_exit = 0;
            state.result = FakeResult::Exit(143);
            Ok(())
        }

        fn output_tails(&mut self, _max_bytes: usize) -> JobResult<RuntimeJobOutput> {
            let state = self
                .state
                .lock()
                .map_err(|error| redacted_error(&format!("lock fake process: {error}")))?;
            Ok(RuntimeJobOutput {
                stdout: state.stdout.clone(),
                stderr: state.stderr.clone(),
                stdout_truncated: false,
                fallback_receipt: Some(state.stdout.clone()),
                fallback_receipt_truncated: false,
            })
        }
    }
}
