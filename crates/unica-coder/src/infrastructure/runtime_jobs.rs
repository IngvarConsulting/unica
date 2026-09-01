//! Durable runtime-job state used by the runtime-job worker and transport adapter.

use super::redaction;
#[cfg(test)]
use super::runtime_build_fallback::FallbackRejection;
use super::runtime_build_fallback::{
    classify_partial_platform_failure, full_rebuild_argv, BuildAttempt, PARTIAL_FALLBACK_WARNING,
};
use super::runtime_build_preflight::RuntimeBuildPreflight;
use crate::application::shared_work::{
    LongWorkFailure, SharedWork, SharedWorkKey, SharedWorkLease, SharedWorkLifetime,
    SharedWorkProducer,
};
use crate::domain::cache::CacheAccess;
use crate::domain::cancellation::{cancelled_error, CancellationToken};
use crate::domain::events::{runtime_event_kind, DomainEvent};
use crate::infrastructure::platform::filesystem::{
    metadata_is_link_or_reparse_point, replace_file_atomically, sync_parent_directory,
    RetainedDirectoryCapability, RetainedRegularFileCapability,
};
use crate::infrastructure::platform::{
    RuntimeProcessTreeHandle, RuntimeProcessTreeState, STDOUT_CAPTURE_LIMIT,
};
use crate::infrastructure::workspace::discover_workspace;
use crate::infrastructure::workspace_services::WorkspaceServiceManager;
use crate::infrastructure::workspace_state::WorkspaceStateRepository;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
#[cfg(test)]
use std::collections::HashSet;
use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{mpsc, Arc, Mutex},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

const RECORD_SCHEMA_VERSION: u8 = 1;
const RUNTIME_RECORD_BYTES: usize = 256 * 1024;
const OUTPUT_TAIL_BYTES: usize = 16 * 1024;
const FALLBACK_RECEIPT_BYTES: usize = STDOUT_CAPTURE_LIMIT;
const DEFAULT_STALE_AFTER: Duration = Duration::from_secs(5 * 60);
const LIFECYCLE_LOCK_WAIT: Duration = Duration::from_secs(30);
const LIFECYCLE_LOCK_RETRY: Duration = Duration::from_millis(20);
const STREAM_FINISH_TIMEOUT: Duration = Duration::from_secs(10);
const RUNTIME_STARTUP_CLEANUP_TIMEOUT: Duration = Duration::from_millis(500);
#[cfg(test)]
thread_local! {
    static STREAM_FINISH_ATTEMPTS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}
#[cfg(test)]
static INJECT_RUNTIME_RECORD_WRITE_FAILURE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
const RUNTIME_SECRET_VALUE_FLAGS: &[&str] =
    &["password", "pwd", "token", "secret", "connection", "c"];
const RUNTIME_CONNECTION_MARKERS: &[&str] = &[
    "file=", "srvr=", "ref=", "usr=", "pwd=", "dbsrvr=", "dbname=",
];

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
            Some(fallback)
                if self.operation != RuntimeJobOperation::Build
                    || full_rebuild
                    || !self
                        .raw_argv
                        .iter()
                        .any(|argument| argument == "--json-message")
                    || full_rebuild_argv(&self.raw_argv).as_ref() != Some(fallback) =>
            {
                return Err(redacted_error(
                    "runtime job full rebuild fallback arguments are inconsistent",
                ));
            }
            None if self.operation == RuntimeJobOperation::Build && !full_rebuild => {
                return Err(redacted_error(
                    "normal build worker request is missing its full rebuild fallback",
                ));
            }
            Some(_) | None => {}
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
    /// A reader that had to be abandoned before EOF, so the persisted tail may
    /// be missing the end of the process output.
    pub(crate) output_incomplete: bool,
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
    fn output_tails_until(
        &mut self,
        max_bytes: usize,
        _deadline: Instant,
    ) -> JobResult<RuntimeJobOutput> {
        self.output_tails(max_bytes)
    }
    /// Binds all cleanup work, including Drop, to the caller's one absolute
    /// monotonic deadline. Generic fakes need no extra state.
    fn bind_cleanup_deadline(&mut self, _deadline: Instant) {}
    #[cfg(test)]
    fn leader_exited_for_test(&self) -> bool {
        false
    }
    /// Test-owned fakes may provide a controlled terminal+complete-output
    /// transition before the real quarantine supervisor accepts them. Real and
    /// deliberately non-cooperative authorities keep the fail-closed default.
    #[cfg(test)]
    fn prepare_controlled_test_supervision(&mut self) -> bool {
        false
    }
}

/// Runner boundary. `attach` reconnects to an existing process; it never starts it again.
pub(crate) trait RuntimeJobRunner: Send + Sync {
    fn spawn(&self, request: &RuntimeJobRequest) -> RuntimeJobSpawnResult;
    fn attach(&self, process_id: u32) -> JobResult<Box<dyn RuntimeJobProcess>>;
}

pub(crate) enum RuntimeJobSpawnFailure {
    /// No child ever existed, or the spawned tree was proven terminal before return.
    ProvenChildless(String),
    /// A child may still exist. Its retained capability must stay owned and the
    /// resource lease must remain quarantined.
    OwnershipRetained {
        error: String,
        process: Box<dyn RuntimeJobProcess>,
    },
}

impl std::fmt::Debug for RuntimeJobSpawnFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ProvenChildless(error) => formatter
                .debug_tuple("ProvenChildless")
                .field(error)
                .finish(),
            Self::OwnershipRetained { error, .. } => formatter
                .debug_struct("OwnershipRetained")
                .field("error", error)
                .finish_non_exhaustive(),
        }
    }
}

type RuntimeJobSpawnResult = Result<Box<dyn RuntimeJobProcess>, RuntimeJobSpawnFailure>;

#[cfg(test)]
struct CoordinationOnlyRunner;

#[cfg(test)]
impl RuntimeJobRunner for CoordinationOnlyRunner {
    fn spawn(&self, _request: &RuntimeJobRequest) -> RuntimeJobSpawnResult {
        Err(RuntimeJobSpawnFailure::ProvenChildless(redacted_error(
            "coordination-only runner cannot spawn",
        )))
    }

    fn attach(&self, _process_id: u32) -> JobResult<Box<dyn RuntimeJobProcess>> {
        Err(redacted_error("coordination-only runner cannot attach"))
    }
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
    fn spawn(&self, request: &RuntimeJobRequest) -> RuntimeJobSpawnResult {
        let mut command = Command::new(&self.program);
        command
            .args(&request.raw_argv)
            .current_dir(&self.cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let process_tree = RuntimeProcessTreeHandle::prepare(&mut command).map_err(|error| {
            RuntimeJobSpawnFailure::ProvenChildless(redacted_error(&format!(
                "prepare runtime job process: {error}"
            )))
        })?;
        let mut child = command.spawn().map_err(|error| {
            RuntimeJobSpawnFailure::ProvenChildless(redacted_error(&format!(
                "spawn runtime job process: {error}"
            )))
        })?;
        let stdout = child.stdout.take().map(|stdout| {
            if request.full_rebuild_fallback_argv.is_some() {
                StreamTail::spawn_with_receipt(stdout)
            } else {
                StreamTail::spawn(stdout)
            }
        });
        let stderr = child.stderr.take().map(StreamTail::spawn);
        let mut process = SystemRuntimeJobProcess {
            id: child.id(),
            child,
            process_tree,
            stdout,
            stderr,
            exited: false,
            cleanup_deadline: None,
            cleanup_complete: false,
        };
        if let Err(error) = process.process_tree.attach(&mut process.child) {
            return Err(process.classify_failed_start(redacted_error(&format!(
                "attach runtime job process tree: {error}"
            ))));
        }
        if process.stdout.is_none() || process.stderr.is_none() {
            return Err(process.classify_failed_start(redacted_error(
                "runtime job process output pipes are incomplete",
            )));
        }
        Ok(Box::new(process))
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
    process_tree: RuntimeProcessTreeHandle,
    stdout: Option<StreamTail>,
    stderr: Option<StreamTail>,
    exited: bool,
    cleanup_deadline: Option<Instant>,
    cleanup_complete: bool,
}

impl SystemRuntimeJobProcess {
    fn classify_failed_start(mut self, error: String) -> RuntimeJobSpawnFailure {
        let cleanup_deadline = self.begin_cleanup_deadline();
        match self.cleanup_until(cleanup_deadline) {
            Ok(()) => RuntimeJobSpawnFailure::ProvenChildless(error),
            Err(cleanup) => RuntimeJobSpawnFailure::OwnershipRetained {
                error: redacted_error(&format!("{error}; startup cleanup uncertain: {cleanup}")),
                process: Box::new(self),
            },
        }
    }

    fn begin_cleanup_deadline(&mut self) -> Instant {
        *self.cleanup_deadline.get_or_insert_with(|| {
            let started = Instant::now();
            started
                .checked_add(RUNTIME_STARTUP_CLEANUP_TIMEOUT)
                .unwrap_or(started)
        })
    }

    fn cleanup_until(&mut self, cleanup_deadline: Instant) -> JobResult<()> {
        let tree_result = if self.exited {
            Ok(())
        } else {
            self.process_tree
                .terminate_and_reap_until(&mut self.child, cleanup_deadline)
                .map_err(|error| io_error("cleanup runtime process tree", &error))
        };
        if tree_result.is_ok() {
            self.exited = true;
        }
        // Reader accounting is finally-style: even a tree/probe error must not
        // skip either reader and let Drop open another cleanup window.
        let output_result = self.output_tails_until(OUTPUT_TAIL_BYTES, cleanup_deadline);
        if let Err(tree_error) = tree_result {
            let output = output_result
                .err()
                .map(|error| format!("; output cleanup: {error}"))
                .unwrap_or_default();
            return Err(redacted_error(&format!("{tree_error}{output}")));
        }
        let output = output_result?;
        if output.output_incomplete {
            return Err(redacted_error(
                "runtime output cleanup deadline elapsed before both readers reached EOF",
            ));
        }
        self.cleanup_complete = true;
        Ok(())
    }
}

impl RuntimeJobProcess for SystemRuntimeJobProcess {
    fn id(&self) -> u32 {
        self.id
    }

    fn try_wait(&mut self) -> JobResult<RuntimeJobProcessState> {
        match self
            .process_tree
            .poll(&mut self.child)
            .map_err(|error| redacted_error(&format!("poll runtime job process: {error}")))?
        {
            RuntimeProcessTreeState::Exited(status) => {
                self.exited = true;
                Ok(RuntimeJobProcessState::Exited {
                    exit_code: status.code().unwrap_or(1),
                })
            }
            RuntimeProcessTreeState::Running => Ok(RuntimeJobProcessState::Running),
        }
    }

    fn cancel(&mut self) -> JobResult<()> {
        self.process_tree
            .terminate(&mut self.child)
            .map_err(|error| redacted_error(&format!("cancel runtime job process tree: {error}")))
    }

    fn output_tails(&mut self, max_bytes: usize) -> JobResult<RuntimeJobOutput> {
        let deadline = Instant::now()
            .checked_add(STREAM_FINISH_TIMEOUT)
            .unwrap_or_else(Instant::now);
        self.output_tails_until(max_bytes, deadline)
    }

    fn output_tails_until(
        &mut self,
        max_bytes: usize,
        deadline: Instant,
    ) -> JobResult<RuntimeJobOutput> {
        // The receipt only decides anything once the process is gone, and it is
        // the whole retained buffer. Reading it on every 25 ms poll of a running
        // build would clone and re-validate it for nothing.
        let (output_incomplete, fallback_receipt, fallback_receipt_truncated) =
            if self.exited || self.cleanup_deadline.is_some() {
                let stdout_incomplete = self
                    .stdout
                    .as_mut()
                    .map(|stream| stream.finish_until(deadline))
                    .transpose()?
                    .unwrap_or(true);
                let stderr_incomplete = self
                    .stderr
                    .as_mut()
                    .map(|stream| stream.finish_until(deadline))
                    .transpose()?
                    .unwrap_or(true);
                let (receipt, receipt_truncated) = self
                    .stdout
                    .as_ref()
                    .map(StreamTail::receipt)
                    .transpose()?
                    .unwrap_or((None, true));
                (
                    stdout_incomplete || stderr_incomplete,
                    receipt,
                    receipt_truncated,
                )
            } else {
                (false, None, false)
            };
        let output = RuntimeJobOutput {
            stdout: self
                .stdout
                .as_ref()
                .map(|stream| stream.tail(max_bytes))
                .transpose()?
                .unwrap_or_default(),
            stderr: self
                .stderr
                .as_ref()
                .map(|stream| stream.tail(max_bytes))
                .transpose()?
                .unwrap_or_default(),
            output_incomplete,
            fallback_receipt,
            fallback_receipt_truncated,
        };
        if self.exited && !output.output_incomplete {
            self.cleanup_complete = true;
        }
        Ok(output)
    }

    fn bind_cleanup_deadline(&mut self, deadline: Instant) {
        self.cleanup_deadline = Some(
            self.cleanup_deadline
                .map(|current| current.min(deadline))
                .unwrap_or(deadline),
        );
    }

    #[cfg(test)]
    fn leader_exited_for_test(&self) -> bool {
        self.process_tree.leader_exited()
    }
}

impl Drop for SystemRuntimeJobProcess {
    fn drop(&mut self) {
        if self.cleanup_complete {
            return;
        }
        if self.cleanup_deadline.is_none() {
            let cleanup_deadline = self.begin_cleanup_deadline();
            let _ = self.cleanup_until(cleanup_deadline);
        }
    }
}

struct StreamTail {
    text: Arc<Mutex<String>>,
    receipt: Option<Arc<Mutex<Vec<u8>>>>,
    receipt_truncated: Arc<std::sync::atomic::AtomicBool>,
    state: StreamTailState,
    finished: mpsc::Receiver<()>,
}

enum StreamTailState {
    Reading(Option<thread::JoinHandle<io::Result<()>>>),
    Eof,
    Failed(String),
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
        let receipt = retain_receipt.then(|| Arc::new(Mutex::new(Vec::new())));
        let captured_receipt = receipt.clone();
        let receipt_truncated = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let capture_receipt_truncated = Arc::clone(&receipt_truncated);
        let (finished_sender, finished) = mpsc::channel::<()>();
        let reader = thread::spawn(move || {
            // Dropped on every exit path, which is what tells `finish` the
            // reader is joinable without blocking on it.
            let _finished = finished_sender;
            let mut buffer = [0_u8; 4096];
            let mut redactor = redaction::StreamRedactor::new();
            loop {
                let count = stream.read(&mut buffer)?;
                if count == 0 {
                    append_tail(&captured, &redactor.finish())?;
                    return Ok(());
                }
                if let Some(receipt) = &captured_receipt {
                    append_byte_tail(receipt, &capture_receipt_truncated, &buffer[..count])?;
                }
                let chunk = String::from_utf8_lossy(&buffer[..count]);
                append_tail(&captured, &redactor.push(&chunk))?;
            }
        });
        Self {
            text,
            receipt,
            receipt_truncated,
            state: StreamTailState::Reading(Some(reader)),
            finished,
        }
    }

    /// Drains the reader for a bounded time and reports whether it had to be
    /// abandoned. A child that exits while a grandchild still holds the
    /// inherited pipe never produces EOF, and this join runs inside the
    /// workspace lifecycle guard: waiting on it forever would wedge the guard
    /// and with it every other job transition, including cancellation.
    #[cfg(test)]
    fn finish(&mut self) -> JobResult<bool> {
        self.finish_within(STREAM_FINISH_TIMEOUT)
    }

    fn finish_until(&mut self, deadline: Instant) -> JobResult<bool> {
        #[cfg(test)]
        STREAM_FINISH_ATTEMPTS.with(|slot| slot.set(slot.get().saturating_add(1)));
        self.finish_within(deadline.saturating_duration_since(Instant::now()))
    }

    fn finish_within(&mut self, timeout: Duration) -> JobResult<bool> {
        match &self.state {
            StreamTailState::Eof => return Ok(false),
            StreamTailState::Failed(error) => return Err(error.clone()),
            StreamTailState::Reading(_) => {}
        }
        match self.finished.recv_timeout(timeout) {
            Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                let reader = match &mut self.state {
                    StreamTailState::Reading(reader) => reader
                        .take()
                        .expect("reading stream retains exactly one reader handle"),
                    StreamTailState::Eof | StreamTailState::Failed(_) => unreachable!(),
                };
                let result = reader
                    .join()
                    .map_err(|_| redacted_error("join runtime job output reader"))
                    .and_then(|result| {
                        result.map_err(|error| io_error("read runtime job output", &error))
                    });
                match result {
                    Ok(()) => {
                        self.state = StreamTailState::Eof;
                        Ok(false)
                    }
                    Err(error) => {
                        self.state = StreamTailState::Failed(error.clone());
                        Err(error)
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // Preserve the reader capability so quarantine supervision can
                // later prove EOF. The partial bytes remain readable, but the
                // receipt cannot authorize a fallback until then.
                self.receipt_truncated
                    .store(true, std::sync::atomic::Ordering::Release);
                Ok(true)
            }
        }
    }

    fn tail(&self, max_bytes: usize) -> JobResult<String> {
        let text = self
            .text
            .lock()
            .map_err(|error| redacted_error(&format!("lock runtime job output: {error}")))?;
        Ok(bounded_tail(&text, max_bytes))
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
    /// Persisted immediately before the worker asks the runner for a child, so a
    /// record can prove whether an orphaned child process may exist. Absent means
    /// unknown, not "no child": a build that predates this field could die after
    /// `runner.spawn()` and before persisting `pid`, leaving a live child behind
    /// a queued record with no trace of it. Only an explicit `false` is proof.
    #[serde(default)]
    child_spawn_attempted: Option<bool>,
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
            child_spawn_attempted: Some(false),
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

    /// Whether an orphaned child process of this job may still be running, and
    /// therefore whether releasing `active.lock` could admit a second job into a
    /// workspace the first one is still mutating. Releasing requires proof that
    /// no child exists: an explicit `child_spawn_attempted: false`, which this
    /// build persists before it asks the runner for a child. An absent flag is
    /// unknown and keeps the lock, as does any recorded `pid`.
    fn may_have_orphan_child(&self) -> bool {
        self.child_spawn_attempted.unwrap_or(true) || self.pid.is_some()
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

#[derive(Debug)]
struct ActiveLifecycleLock {
    file: File,
    jobs_root: RetainedDirectoryCapability,
    lifecycle: RetainedRegularFileCapability,
}

impl ActiveLifecycleLock {
    fn validate(&self) -> io::Result<()> {
        self.jobs_root.validate_named_identity()?;
        self.lifecycle.validate_named_identity()
    }

    fn retain_active_lock(&self) -> io::Result<RetainedRegularFileCapability> {
        self.jobs_root
            .retain_regular_child(std::ffi::OsStr::new("active.lock"))
    }

    fn retain_job_directory(&self, id: &str) -> io::Result<RetainedDirectoryCapability> {
        self.validate()?;
        let directory = self
            .jobs_root
            .retain_directory_child(std::ffi::OsStr::new(id))?;
        directory.validate_named_identity()?;
        Ok(directory)
    }

    fn release_active_lock_for(
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

    fn retain_jobs_root(&self) -> JobResult<RetainedDirectoryCapability> {
        fs::create_dir_all(&self.cache_root)
            .map_err(|error| io_error("create runtime cache directory", &error))?;
        let canonical_cache = fs::canonicalize(&self.cache_root)
            .map_err(|error| io_error("resolve runtime cache directory", &error))?;
        RetainedDirectoryCapability::open_or_create(&canonical_cache.join("jobs"))
            .map_err(|error| io_error("retain runtime jobs directory", &error))
    }

    fn active_lock_path(&self) -> PathBuf {
        self.jobs_root().join("active.lock")
    }

    #[cfg(test)]
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
    /// The hook runs after the lifecycle guard is held and before the active
    /// lock is claimed, so a test can interleave another process in that window.
    /// Production passes a no-op.
    fn acquire_active_lock_after_lifecycle(
        &self,
        id: &str,
        after_lifecycle_lock: impl FnOnce(),
    ) -> JobResult<()> {
        let _lifecycle_lock = self.acquire_active_lifecycle_lock_bounded()?;
        after_lifecycle_lock();
        self.acquire_active_lock_guarded(id)
    }

    fn acquire_active_lifecycle_lock(&self) -> JobResult<ActiveLifecycleLock> {
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
    fn acquire_active_lifecycle_lock_bounded(&self) -> JobResult<ActiveLifecycleLock> {
        let deadline = Instant::now()
            .checked_add(LIFECYCLE_LOCK_WAIT)
            .ok_or_else(|| redacted_error("runtime job lifecycle deadline is unrepresentable"))?;
        self.acquire_active_lifecycle_lock_until(deadline)
    }

    fn acquire_active_lifecycle_lock_until(
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

    fn acquire_active_lifecycle_lock_for_root_bounded(
        &self,
        jobs_root: RetainedDirectoryCapability,
    ) -> JobResult<ActiveLifecycleLock> {
        let deadline = Instant::now()
            .checked_add(LIFECYCLE_LOCK_WAIT)
            .ok_or_else(|| redacted_error("runtime job lifecycle deadline is unrepresentable"))?;
        loop {
            if let Some(lifecycle) = self.open_active_lifecycle_lock_in(jobs_root.clone(), true)? {
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

    fn try_acquire_active_lifecycle_lock(&self) -> JobResult<Option<ActiveLifecycleLock>> {
        self.open_active_lifecycle_lock(true)
    }

    fn open_active_lifecycle_lock(
        &self,
        nonblocking: bool,
    ) -> JobResult<Option<ActiveLifecycleLock>> {
        let jobs_root = self.retain_jobs_root()?;
        self.open_active_lifecycle_lock_in(jobs_root, nonblocking)
    }

    fn open_active_lifecycle_lock_in(
        &self,
        jobs_root: RetainedDirectoryCapability,
        nonblocking: bool,
    ) -> JobResult<Option<ActiveLifecycleLock>> {
        jobs_root
            .validate_named_identity()
            .map_err(|error| io_error("revalidate retained runtime jobs directory", &error))?;
        let lifecycle = jobs_root
            .retain_or_create_regular_child(std::ffi::OsStr::new("active.lifecycle.lock"))
            .map_err(|error| io_error("open active runtime job lifecycle lock", &error))?;
        let file = lifecycle
            .try_clone_file()
            .map_err(|error| io_error("clone active runtime job lifecycle lock", &error))?;
        let lock_result = if nonblocking {
            FileExt::try_lock_exclusive(&file)
        } else {
            FileExt::lock_exclusive(&file)
        };
        match lock_result {
            Ok(()) => Ok(Some(ActiveLifecycleLock {
                file,
                jobs_root,
                lifecycle,
            })),
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
    fn orphan_retention_hint(&self, id: &str) -> String {
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

    fn release_active_lock_for(&self, id: &str) -> JobResult<()> {
        self.release_active_lock_for_after_hooks(id, || {}, || {})
    }

    /// The hook parameters exist so a test can interleave another process at the
    /// two observation points this release has. Production always passes no-ops.
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
    fn record_is_provably_absent(&self, id: &str) -> bool {
        let jobs_root = self.jobs_root();
        let jobs_root_metadata = match fs::symlink_metadata(&jobs_root) {
            Ok(metadata) => metadata,
            Err(_) => return false,
        };
        if metadata_is_link_or_reparse_point(&jobs_root_metadata) || !jobs_root_metadata.is_dir() {
            return false;
        }

        let Ok(job_dir) = self.job_dir(id) else {
            return false;
        };
        match fs::symlink_metadata(&job_dir) {
            Ok(metadata) if metadata.is_dir() && !metadata_is_link_or_reparse_point(&metadata) => {}
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
    fn active_lock_outlived_stale_window(&self) -> JobResult<bool> {
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

    fn read_record(&self, id: &str) -> JobResult<RuntimeJobRecord> {
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

    fn read_retained_record(
        &self,
        id: &str,
        record: &RetainedRegularFileCapability,
    ) -> JobResult<RuntimeJobRecord> {
        let contents = record
            .read_bounded(RUNTIME_RECORD_BYTES)
            .map_err(|error| io_error("read retained runtime job record", &error))?;
        self.decode_record(id, &contents)
    }

    fn decode_record(&self, id: &str, contents: &[u8]) -> JobResult<RuntimeJobRecord> {
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

    fn write_record_in_retained_job(
        &self,
        job_directory: &RetainedDirectoryCapability,
        record: &RuntimeJobRecord,
    ) -> JobResult<RetainedRegularFileCapability> {
        #[cfg(test)]
        if INJECT_RUNTIME_RECORD_WRITE_FAILURE.swap(false, std::sync::atomic::Ordering::AcqRel) {
            return Err(redacted_error("injected runtime record write failure"));
        }
        let bytes = serde_json::to_vec_pretty(record)
            .map_err(|error| redacted_error(&format!("serialize runtime job record: {error}")))?;
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

    fn write_record(&self, record: &RuntimeJobRecord) -> JobResult<()> {
        #[cfg(test)]
        if INJECT_RUNTIME_RECORD_WRITE_FAILURE.swap(false, std::sync::atomic::Ordering::AcqRel) {
            return Err(redacted_error("injected runtime record write failure"));
        }
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
#[derive(Clone)]
struct RetainedRuntimeJobAuthority {
    jobs_root: RetainedDirectoryCapability,
    job_directory: RetainedDirectoryCapability,
}

struct ActiveRuntimeJobProcess {
    process: Box<dyn RuntimeJobProcess>,
    jobs_root: RetainedDirectoryCapability,
    /// Exact descriptor authority admitted before the process could start.
    /// Quarantine retries must never re-resolve `jobs/<id>` from its name.
    job_directory: RetainedDirectoryCapability,
    attempt_argv: Vec<String>,
    fallback: Option<RuntimeJobRequest>,
    previous_output: Option<RuntimeJobOutput>,
}

/// One observation of the active process: its state, the outputs to persist and
/// the evidence a full rebuild decision needs.
struct RuntimeJobObservation {
    state: RuntimeJobProcessState,
    output: RuntimeJobOutput,
    attempt_argv: Vec<String>,
    /// Whether a full rebuild is still available for this job. Only then can a
    /// refused receipt mean anything, so only then is it worth reporting.
    fallback_pending: bool,
}

pub(crate) struct RuntimeJobService {
    store: RuntimeJobStore,
    runner: Arc<dyn RuntimeJobRunner>,
    processes: Mutex<HashMap<String, ActiveRuntimeJobProcess>>,
    #[cfg(test)]
    test_admission: Option<TestServiceAdmission>,
}

impl Drop for RuntimeJobService {
    fn drop(&mut self) {
        let Ok(processes) = self.processes.get_mut() else {
            return;
        };
        for (job_id, active) in processes.drain() {
            let store = self.store.clone();
            #[cfg(test)]
            let (active, controlled_test_supervision) = {
                let mut active = active;
                let controlled = active.process.prepare_controlled_test_supervision();
                (active, controlled)
            };
            let retained = Arc::new(Mutex::new(Some(active)));
            match spawn_quarantine_supervisor(store, job_id.clone(), Arc::clone(&retained)) {
                Ok(supervisor) => {
                    #[cfg(test)]
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
                    #[cfg(not(test))]
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

type QuarantinedRuntimeProcess = Arc<Mutex<Option<ActiveRuntimeJobProcess>>>;

#[cfg(test)]
static INJECT_QUARANTINE_THREAD_SPAWN_FAILURE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[cfg(test)]
struct QuarantineSupervisor {
    handle: thread::JoinHandle<()>,
    finished: std::sync::mpsc::Receiver<()>,
}

#[cfg(not(test))]
type QuarantineSupervisor = thread::JoinHandle<()>;

fn spawn_quarantine_supervisor(
    store: RuntimeJobStore,
    job_id: String,
    retained: QuarantinedRuntimeProcess,
) -> io::Result<QuarantineSupervisor> {
    #[cfg(test)]
    if INJECT_QUARANTINE_THREAD_SPAWN_FAILURE.swap(false, std::sync::atomic::Ordering::AcqRel) {
        return Err(io::Error::other(
            "injected runtime quarantine thread spawn failure",
        ));
    }

    #[cfg(test)]
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
            #[cfg(test)]
            let _ = finished_tx.send(());
        })?;
    #[cfg(test)]
    {
        Ok(QuarantineSupervisor {
            handle,
            finished: finished_rx,
        })
    }
    #[cfg(not(test))]
    {
        Ok(handle)
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct TestFixtureOwnerToken(Uuid);

#[cfg(test)]
#[derive(Default)]
struct TestFixtureRegistry {
    owners: HashMap<TestFixtureOwnerToken, TestFixtureOwnerState>,
    exact_roots: HashMap<PathBuf, TestFixtureOwnerToken>,
}

#[cfg(test)]
struct TestFixtureOwnerState {
    outer_root: PathBuf,
    admission_open: bool,
    live_admissions: usize,
    exact_service_roots: HashSet<PathBuf>,
    supervisors: Vec<TestQuarantineSupervisor>,
}

#[cfg(test)]
static TEST_FIXTURE_REGISTRY: std::sync::OnceLock<Mutex<TestFixtureRegistry>> =
    std::sync::OnceLock::new();

#[cfg(test)]
fn test_fixture_registry() -> &'static Mutex<TestFixtureRegistry> {
    TEST_FIXTURE_REGISTRY.get_or_init(|| Mutex::new(TestFixtureRegistry::default()))
}

#[cfg(test)]
struct TestServiceAdmission {
    owner: TestFixtureOwnerToken,
    service_root: PathBuf,
}

#[cfg(test)]
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

#[cfg(test)]
struct TestQuarantineSupervisor {
    service_root: PathBuf,
    job_id: String,
    controlled: bool,
    supervisor: QuarantineSupervisor,
}

#[cfg(test)]
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

#[cfg(test)]
fn create_test_fixture_owner(outer_root: PathBuf) -> TestFixtureOwnerToken {
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

#[cfg(test)]
fn bind_test_service_root(owner: TestFixtureOwnerToken, service_root: PathBuf) -> PathBuf {
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

#[cfg(test)]
fn acquire_test_service_admission(service_root: &Path) -> Option<TestServiceAdmission> {
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

#[cfg(test)]
fn drain_test_fixture_owner(owner: TestFixtureOwnerToken) {
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

#[cfg(test)]
thread_local! {
    static TEST_SUPERVISOR_SCOPES: std::cell::RefCell<Vec<Arc<Mutex<Vec<TestFixtureOwnerToken>>>>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(test)]
struct TestSupervisorScope {
    owners: Arc<Mutex<Vec<TestFixtureOwnerToken>>>,
}

#[cfg(test)]
impl TestSupervisorScope {
    fn new() -> Self {
        let owners = Arc::new(Mutex::new(Vec::new()));
        TEST_SUPERVISOR_SCOPES.with(|scopes| scopes.borrow_mut().push(Arc::clone(&owners)));
        Self { owners }
    }

    fn assert_drained(&self) {
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

#[cfg(test)]
impl Drop for TestSupervisorScope {
    fn drop(&mut self) {
        TEST_SUPERVISOR_SCOPES.with(|scopes| {
            let popped = scopes.borrow_mut().pop().expect("test supervisor scope");
            assert!(Arc::ptr_eq(&popped, &self.owners));
        });
    }
}

#[cfg(test)]
fn register_test_supervisor_owner(owner: TestFixtureOwnerToken) {
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
struct RuntimeWorkKey {
    resource_identity: [u8; 32],
    lease_identity: Uuid,
}

/// Daemon-owned in-process coordinator. Only `RuntimeJobService` can derive a
/// key, while the store's lifecycle guard and retained `active.lock`
/// capability are held. A canonical service can therefore ask to join an
/// exact active lease but cannot manufacture or cache a movable authority.
pub(crate) struct RuntimeResourceOwner {
    shared: SharedWork<(), LongWorkFailure>,
}

impl Default for RuntimeResourceOwner {
    fn default() -> Self {
        Self {
            shared: SharedWork::new(SharedWorkLifetime::OwnerBound),
        }
    }
}

impl RuntimeResourceOwner {
    fn join_or_start<W>(&self, key: RuntimeWorkKey, work: W) -> SharedWorkLease<(), LongWorkFailure>
    where
        W: FnOnce(SharedWorkProducer) -> Result<(), LongWorkFailure> + Send + 'static,
    {
        self.shared.join_or_start(
            SharedWorkKey::Runtime {
                resource_identity: key.resource_identity,
                lease_identity: key.lease_identity,
            },
            work,
        )
    }
}

impl RuntimeJobService {
    pub(crate) fn new(cache_root: impl Into<PathBuf>, runner: Arc<dyn RuntimeJobRunner>) -> Self {
        Self::with_stale_after(cache_root, runner, DEFAULT_STALE_AFTER)
    }

    #[cfg(test)]
    pub(crate) fn coordination_only_for_test(cache_root: impl Into<PathBuf>) -> Self {
        Self::new(cache_root, Arc::new(CoordinationOnlyRunner))
    }

    /// Join exact runtime work while the same lifecycle authority that owns
    /// `active.lock` is held. No authority key leaves this method.
    #[allow(dead_code)] // Consumed by the canonical runtime handler at Task 22 cutover.
    pub(crate) fn join_shared_work<W>(
        &self,
        id: &str,
        owner: &RuntimeResourceOwner,
        work: W,
    ) -> JobResult<SharedWorkLease<(), LongWorkFailure>>
    where
        W: FnOnce(SharedWorkProducer) -> Result<(), LongWorkFailure> + Send + 'static,
    {
        self.join_shared_work_after_capability(id, owner, work, || {})
    }

    fn join_shared_work_after_capability<W>(
        &self,
        id: &str,
        owner: &RuntimeResourceOwner,
        work: W,
        after_capability: impl FnOnce(),
    ) -> JobResult<SharedWorkLease<(), LongWorkFailure>>
    where
        W: FnOnce(SharedWorkProducer) -> Result<(), LongWorkFailure> + Send + 'static,
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
        runner: Arc<dyn RuntimeJobRunner>,
        stale_after: Duration,
    ) -> Self {
        let cache_root = cache_root.into();
        Self {
            #[cfg(test)]
            test_admission: acquire_test_service_admission(&cache_root),
            store: RuntimeJobStore::new(cache_root, stale_after),
            runner,
            processes: Mutex::new(HashMap::new()),
        }
    }

    #[cfg(test)]
    fn require_test_fixture_admission(&self) -> JobResult<()> {
        self.test_admission.as_ref().map(|_| ()).ok_or_else(|| {
            redacted_error(
                "test runtime service root has no explicit fixture owner binding; use TestCache::service_root for nested roots",
            )
        })
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
    /// The hook runs after the first build reauthorization, so a test can let
    /// recovery or cancellation win that window. Production passes a no-op.
    fn activate_enqueued_after_preflight(
        &self,
        id: &str,
        request: &RuntimeJobRequest,
        after_preflight: impl FnOnce(),
    ) -> JobResult<RuntimeJobSnapshot> {
        self.activate_enqueued_after_hooks(id, request, after_preflight, || {})
    }
    /// The second hook runs just before the activation guard is claimed, so a
    /// test can order itself against it. Production passes no-ops.
    fn activate_enqueued_after_hooks(
        &self,
        id: &str,
        request: &RuntimeJobRequest,
        after_preflight: impl FnOnce(),
        before_activation_guard: impl FnOnce(),
    ) -> JobResult<RuntimeJobSnapshot> {
        #[cfg(test)]
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
        let lifecycle_lock = self.store.acquire_active_lifecycle_lock()?;
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
        let preflight_result = preflight_result.and_then(|()| match &request.build_preflight {
            Some(build_preflight) => build_preflight.reauthorize_current_workspace(),
            None => Ok(()),
        });
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
        if let Err(error) = self.write_record_with_authority(&retained_authority, &record) {
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
        if let Err(error) = self.write_record_with_authority(&retained_authority, &record) {
            self.cleanup_activation_failure_guarded(
                &mut record,
                process,
                retained_authority.clone(),
                request.raw_argv.clone(),
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
                    request.raw_argv.clone(),
                    None,
                    &error,
                );
                return Err(error);
            }
        };
        let mut request_with_fallback = request.clone();
        let fallback = request_with_fallback.take_full_rebuild_fallback();
        processes.insert(
            id.to_string(),
            ActiveRuntimeJobProcess {
                process,
                jobs_root: retained_authority.jobs_root,
                job_directory: retained_authority.job_directory,
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
        let lifecycle_lock = self.store.acquire_active_lifecycle_lock()?;
        let mut record = self.store.read_record(id)?;
        if record.phase.is_terminal() {
            return Ok(record.snapshot(false));
        }
        if self.store.stale(&record) {
            record.transition(RuntimeJobPhase::Lost)?;
            record.finished_at_ms = Some(now_millis());
            record.warnings.push("stale heartbeat".to_string());
            self.store.write_record(&record)?;
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
        let RuntimeJobObservation {
            state: process_state,
            output,
            attempt_argv,
            fallback_pending,
        } = match self.observe_process(&record, request_safe_cancel, &lifecycle_lock) {
            Ok(observation) => observation,
            Err(error) => {
                return Err(self.quarantine_observation_failure_guarded(&mut record, &error));
            }
        };
        self.store.write_logs(&record.id, &output)?;
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
                self.store.write_record(&record)?;
                Ok(record.snapshot(false))
            }
            RuntimeJobProcessState::Exited { exit_code } => {
                if exit_code != 0 && !cancel_requested && fallback_pending {
                    match classify_partial_platform_failure(&BuildAttempt {
                        argv: &attempt_argv,
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
                            )? {
                                return Ok(snapshot);
                            }
                        }
                        // A receipt that reached the pinned failure code and was
                        // still refused has to say so, or a drifted receipt is
                        // indistinguishable from a runtime that never tried.
                        Err(rejection) => record.warnings.extend(rejection.warning()),
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
        lifecycle: &ActiveLifecycleLock,
    ) -> JobResult<RuntimeJobObservation> {
        #[cfg(test)]
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
        Ok(RuntimeJobObservation {
            state,
            output,
            attempt_argv: active.attempt_argv.clone(),
            fallback_pending: active.fallback.is_some(),
        })
    }

    fn start_full_build_fallback_guarded(
        &self,
        record: &mut RuntimeJobRecord,
        first_output: &RuntimeJobOutput,
        first_exit_code: i32,
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

        let next = match self.runner.spawn(&fallback) {
            Ok(process) => process,
            Err(RuntimeJobSpawnFailure::ProvenChildless(error)) => {
                record.warnings.push(redacted_error(&format!(
                    "full rebuild fallback was not started because v8-runner failed to spawn: {error}"
                )));
                return self
                    .finish_guarded(record, RuntimeJobPhase::Failed, Some(first_exit_code), None)
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
        record.redacted_argv = redact_argv(&fallback.raw_argv);
        record.heartbeat_at_ms = Some(now_millis());
        record.warnings.push(PARTIAL_FALLBACK_WARNING.to_string());
        if let Err(error) = self.write_record_with_authority(&retained_authority, record) {
            self.cleanup_activation_failure_guarded(
                record,
                next,
                retained_authority.clone(),
                fallback.raw_argv.clone(),
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
                    fallback.raw_argv.clone(),
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
                fallback.raw_argv.clone(),
                Some(first_output.clone()),
                &error,
            );
            return Err(error);
        };
        active.process = next;
        active.attempt_argv = fallback.raw_argv.clone();
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
        request: &RuntimeJobRequest,
        process: Box<dyn RuntimeJobProcess>,
        retained_authority: RetainedRuntimeJobAuthority,
        previous_output: Option<RuntimeJobOutput>,
        error: &str,
    ) -> String {
        let process_id = process.id();
        record.pid = Some(process_id);
        record.pid_identity = Some(format!("pid:{process_id}"));
        record.redacted_argv = redact_argv(&request.raw_argv);
        let _ = record.transition(RuntimeJobPhase::Lost);
        record.finished_at_ms = Some(now_millis());
        record.heartbeat_at_ms = Some(now_millis());
        record.warnings.push(redacted_error(&format!(
            "runtime process ownership is uncertain after spawn: {error}; active.lock remains quarantined"
        )));
        let persist = self.write_record_with_authority(&retained_authority, record);
        let retain = self.lock_processes().map(|mut processes| {
            processes.insert(
                record.id.clone(),
                ActiveRuntimeJobProcess {
                    process,
                    jobs_root: retained_authority.jobs_root,
                    job_directory: retained_authority.job_directory,
                    attempt_argv: request.raw_argv.clone(),
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
        match self.store.write_record(record) {
            Ok(()) => redacted_error(error),
            Err(persist) => redacted_error(&format!(
                "{error}; persist retained runtime quarantine: {persist}"
            )),
        }
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

    fn write_record_with_authority(
        &self,
        authority: &RetainedRuntimeJobAuthority,
        record: &RuntimeJobRecord,
    ) -> JobResult<()> {
        authority
            .job_directory
            .validate_named_identity()
            .map_err(|error| io_error("validate retained runtime job directory", &error))?;
        self.store
            .write_record_in_retained_job(&authority.job_directory, record)
            .map(|_| ())
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
        self.write_record_with_authority(authority, record)?;
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
                    .write_record_with_authority(&retained_authority, record)
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
                let _ = self.write_record_with_authority(&retained_authority, record);
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

fn supervise_owned_quarantine(
    store: RuntimeJobStore,
    job_id: String,
    mut active: ActiveRuntimeJobProcess,
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
fn try_release_owned_quarantine_once(
    store: &RuntimeJobStore,
    job_id: &str,
    active: &mut ActiveRuntimeJobProcess,
) -> JobResult<bool> {
    try_release_owned_quarantine_once_after_terminal(store, job_id, active, || {})
}

/// The hook lets the physical-root replacement race be scheduled exactly
/// after terminal/output proof and before lifecycle admission.
fn try_release_owned_quarantine_once_after_terminal(
    store: &RuntimeJobStore,
    job_id: &str,
    active: &mut ActiveRuntimeJobProcess,
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
enum QuarantineReleaseHookPoint {
    AfterTerminalProof,
    AfterFinalValidationBeforeRead,
    AfterRecordRead,
    ImmediatelyBeforeRecordPublish,
    AfterRecordPublishBeforeConfirmation,
}

/// The typed hook exposes each record-I/O boundary without widening the
/// production transition's argument surface. Production wrappers ignore every
/// point.
fn try_release_owned_quarantine_once_after_record_hooks(
    store: &RuntimeJobStore,
    job_id: &str,
    active: &mut ActiveRuntimeJobProcess,
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

pub(crate) fn run_worker_from_args(_args: &[String]) -> Result<(), String> {
    run_worker_from_reader(io::stdin().lock())
}

/// Decodes the handoff frames from an arbitrary source so that EOF, malformed and
/// truncated first frames are deterministically reproducible without a real pipe.
fn run_worker_from_reader(reader: impl Read) -> Result<(), String> {
    let mut reader = io::BufReader::new(reader);
    let handoff: WorkerStartRequest = read_worker_frame(&mut reader, "runtime job worker request")?;
    let commit = read_worker_commit(&mut reader, &handoff)?;
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
/// The hook runs between the request and commit frames, so a test can cancel
/// while exactly one frame is readable. Production passes a no-op.
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
    run_worker_request_with_lifecycle_hooks(handoff, runner, None::<fn()>, None::<fn()>)
}

/// The hook runs after the worker has durably published `Lost` and immediately
/// before it starts supervising the retained process. Tests use the event
/// instead of racing the worker through the ambient filesystem.
#[cfg(test)]
fn run_worker_request_after_quarantine_persisted(
    handoff: WorkerStartRequest,
    runner: Arc<dyn RuntimeJobRunner>,
    after_quarantine_persisted: impl FnMut(),
) -> Result<(), String> {
    run_worker_request_with_lifecycle_hooks(
        handoff,
        runner,
        None::<fn()>,
        Some(after_quarantine_persisted),
    )
}

#[cfg(test)]
fn run_worker_request_after_activation_and_quarantine_persisted(
    handoff: WorkerStartRequest,
    runner: Arc<dyn RuntimeJobRunner>,
    after_activation_persisted: impl FnMut(),
    after_quarantine_persisted: impl FnMut(),
) -> Result<(), String> {
    run_worker_request_with_lifecycle_hooks(
        handoff,
        runner,
        Some(after_activation_persisted),
        Some(after_quarantine_persisted),
    )
}

fn run_worker_request_with_lifecycle_hooks(
    handoff: WorkerStartRequest,
    runner: Arc<dyn RuntimeJobRunner>,
    mut after_activation_persisted: Option<impl FnMut()>,
    mut after_quarantine_persisted: Option<impl FnMut()>,
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
                if let Err(error) = apply_runtime_success_effects(&worker_cwd, operation, &job_id) {
                    let _ = service.append_warning(&job_id, &error);
                }
            }
            return Ok(());
        }
        thread::sleep(Duration::from_millis(25));
    }
}

fn notify_after_durable_quarantine(
    service: &RuntimeJobService,
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

fn enqueue_cancellable_runtime_job(
    cache_root: &Path,
    request: &RuntimeJobRequest,
    cancellation: &CancellationToken,
) -> JobResult<RuntimeJobSnapshot> {
    enqueue_cancellable_runtime_job_after_hook(cache_root, request, cancellation, || {})
}
/// The hook runs after the durable enqueue and before the cancellation is
/// re-read, so a test can flip the token in that window. Production passes a
/// no-op.
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

fn cancel_and_reap_with_budget(
    process: &mut dyn RuntimeJobProcess,
    budget: Duration,
) -> JobResult<()> {
    let started = Instant::now();
    let deadline = started.checked_add(budget).unwrap_or(started);
    process.bind_cleanup_deadline(deadline);
    let process_result = (|| {
        process.cancel()?;
        loop {
            match process.try_wait()? {
                RuntimeJobProcessState::Exited { .. } | RuntimeJobProcessState::TimedOut { .. } => {
                    return Ok(());
                }
                RuntimeJobProcessState::Running if Instant::now() >= deadline => {
                    return Err(redacted_error(
                        "runtime job process did not exit after cancellation request",
                    ));
                }
                RuntimeJobProcessState::Running => thread::sleep(
                    Duration::from_millis(10)
                        .min(deadline.saturating_duration_since(Instant::now())),
                ),
            }
        }
    })();
    // Reader accounting is mandatory on both success and failure and consumes
    // the same absolute deadline. System processes remember that deadline, so
    // their Drop cannot open a second cleanup window.
    let output_result = process.output_tails_until(OUTPUT_TAIL_BYTES, deadline);
    match process_result {
        Err(error) => Err(error),
        Ok(()) => {
            let output = output_result?;
            if output.output_incomplete {
                Err(redacted_error(
                    "runtime output cleanup deadline elapsed before both readers reached EOF",
                ))
            } else {
                Ok(())
            }
        }
    }
}

fn cancel_and_reap(process: &mut dyn RuntimeJobProcess) -> JobResult<()> {
    cancel_and_reap_with_budget(process, Duration::from_secs(5))
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
            redact_next = RUNTIME_SECRET_VALUE_FLAGS.contains(&lower.as_str());
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
    RUNTIME_CONNECTION_MARKERS
        .iter()
        .any(|marker| lower.contains(marker))
}

#[cfg(test)]
pub(crate) fn production_runtime_secret_flags() -> &'static [&'static str] {
    RUNTIME_SECRET_VALUE_FLAGS
}

#[cfg(test)]
pub(crate) fn production_runtime_connection_markers() -> &'static [&'static str] {
    RUNTIME_CONNECTION_MARKERS
}

fn bounded_redacted_tail(text: &str, max_bytes: usize) -> String {
    bounded_tail(&redact_text(text), max_bytes)
}

fn append_tail(target: &Arc<Mutex<String>>, addition: &str) -> io::Result<()> {
    let mut text = target
        .lock()
        .map_err(|_| io::Error::other("runtime job output lock is poisoned"))?;
    text.push_str(addition);
    if text.len() > OUTPUT_TAIL_BYTES {
        *text = bounded_tail(&text, OUTPUT_TAIL_BYTES);
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
        output_incomplete: initial.output_incomplete || fallback.output_incomplete,
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
pub(crate) mod tests {
    use super::*;
    use crate::infrastructure::platform::testing::{
        attempt_retained_directory_replacement_for_test, create_directory_link_fixture_for_test,
        create_file_link_fixture_for_test, path_identity_for_test, FileLinkFixtureOutcome,
        RetainedDirectoryReplacementOutcome,
    };
    use serde_json::{json, Map};
    use std::{
        collections::HashMap,
        io::Cursor,
        sync::{
            atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering},
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
        let terminal_record = fs::read(
            reconnected
                .store
                .record_path(&job.id)
                .expect("terminal record path"),
        )
        .expect("read terminal record");
        drop(reconnected);
        drop(service);
        cache.drain_supervisors();
        assert_eq!(
            fs::read(
                RuntimeJobStore::new(cache.path(), DEFAULT_STALE_AFTER)
                    .record_path(&job.id)
                    .expect("converged terminal record path"),
            )
            .expect("read converged terminal record"),
            terminal_record,
            "a duplicate retained authority rewrote the published terminal result",
        );
        assert!(
            !cache.path().join("jobs/active.lock").exists(),
            "a duplicate retained authority recreated or retained the released lease",
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

    /// A pipe whose write end outlived the child: readable, never at EOF.
    struct NeverEndingStream {
        released: Arc<std::sync::atomic::AtomicBool>,
    }

    enum FaultyStream {
        Error,
        Panic,
    }

    impl Read for FaultyStream {
        fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
            match self {
                Self::Error => Err(io::Error::other("injected stream read failure")),
                Self::Panic => panic!("injected stream reader panic"),
            }
        }
    }

    struct TerminalFaultyTailProcess {
        tail: StreamTail,
    }

    impl RuntimeJobProcess for TerminalFaultyTailProcess {
        fn id(&self) -> u32 {
            7332
        }

        fn try_wait(&mut self) -> JobResult<RuntimeJobProcessState> {
            Ok(RuntimeJobProcessState::Exited { exit_code: 1 })
        }

        fn cancel(&mut self) -> JobResult<()> {
            Ok(())
        }

        fn output_tails(&mut self, max_bytes: usize) -> JobResult<RuntimeJobOutput> {
            self.output_tails_until(max_bytes, Instant::now())
        }

        fn output_tails_until(
            &mut self,
            max_bytes: usize,
            _deadline: Instant,
        ) -> JobResult<RuntimeJobOutput> {
            let output_incomplete = self.tail.finish_within(Duration::from_secs(1))?;
            Ok(RuntimeJobOutput {
                stdout: self.tail.tail(max_bytes)?,
                stderr: String::new(),
                output_incomplete,
                fallback_receipt: None,
                fallback_receipt_truncated: false,
            })
        }
    }

    struct TerminalTreeWithTwoHeldReaders {
        stdout: StreamTail,
        stderr: StreamTail,
        cancelled: bool,
        output_incomplete: bool,
    }

    impl RuntimeJobProcess for TerminalTreeWithTwoHeldReaders {
        fn id(&self) -> u32 {
            7331
        }

        fn try_wait(&mut self) -> JobResult<RuntimeJobProcessState> {
            Ok(if self.cancelled {
                RuntimeJobProcessState::Exited { exit_code: 143 }
            } else {
                RuntimeJobProcessState::Running
            })
        }

        fn cancel(&mut self) -> JobResult<()> {
            self.cancelled = true;
            Ok(())
        }

        fn output_tails(&mut self, _max_bytes: usize) -> JobResult<RuntimeJobOutput> {
            panic!("cleanup must use the absolute-deadline output seam")
        }

        fn output_tails_until(
            &mut self,
            max_bytes: usize,
            deadline: Instant,
        ) -> JobResult<RuntimeJobOutput> {
            let stdout_incomplete = self.stdout.finish_until(deadline)?;
            let stderr_incomplete = self.stderr.finish_until(deadline)?;
            self.output_incomplete = stdout_incomplete || stderr_incomplete;
            Ok(RuntimeJobOutput {
                stdout: self.stdout.tail(max_bytes)?,
                stderr: self.stderr.tail(max_bytes)?,
                output_incomplete: self.output_incomplete,
                fallback_receipt: None,
                fallback_receipt_truncated: false,
            })
        }
    }

    impl Read for NeverEndingStream {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            while !self.released.load(std::sync::atomic::Ordering::Acquire) {
                thread::sleep(Duration::from_millis(5));
            }
            buffer[0] = b'x';
            Ok(0)
        }
    }

    #[test]
    fn a_reader_that_never_reaches_eof_is_abandoned_instead_of_wedging_the_guard() {
        let released = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut tail = StreamTail::spawn_with_receipt(NeverEndingStream {
            released: Arc::clone(&released),
        });

        let abandoned = tail
            .finish_within(Duration::from_millis(50))
            .expect("an unfinished reader must not fail the observation");

        assert!(
            abandoned,
            "a reader still waiting on an inherited pipe must be abandoned"
        );
        let (receipt, receipt_truncated) = tail.receipt().expect("read classification receipt");
        assert!(receipt.is_none());
        assert!(
            receipt_truncated,
            "an abandoned reader cannot prove a whole receipt, so no fallback may be authorized"
        );
        released.store(true, std::sync::atomic::Ordering::Release);
    }

    fn assert_faulty_reader_stays_quarantined(stream: FaultyStream) {
        let cache = TestCache::new();
        let service = RuntimeJobService::new(cache.path(), Arc::new(FakeRunner::success_after(0)));
        let started = service
            .start(fake_request(RuntimeJobOperation::Test))
            .expect("start retained process");
        let mut record = service.store.read_record(&started.id).unwrap();
        record.transition(RuntimeJobPhase::Lost).unwrap();
        service.store.write_record(&record).unwrap();
        let mut active = service
            .lock_processes()
            .unwrap()
            .remove(&started.id)
            .expect("take retained process");
        active.process = Box::new(TerminalFaultyTailProcess {
            tail: StreamTail::spawn(stream),
        });

        assert!(
            !try_release_owned_quarantine_once(&service.store, &started.id, &mut active,)
                .expect("first failed reader probe remains quarantined")
        );
        assert!(
            !try_release_owned_quarantine_once(&service.store, &started.id, &mut active,)
                .expect("second failed reader probe remains quarantined")
        );
        assert!(service.store.active_lock_path().exists());
        assert_eq!(
            service.store.read_record(&started.id).unwrap().phase,
            RuntimeJobPhase::Lost,
            "a failed reader must not publish success or authorize fallback"
        );
    }

    #[test]
    fn stream_tail_read_failure_is_sticky_across_quarantine_probes() {
        assert_faulty_reader_stays_quarantined(FaultyStream::Error);
    }

    #[test]
    fn stream_tail_panic_is_sticky_across_quarantine_probes() {
        assert_faulty_reader_stays_quarantined(FaultyStream::Panic);
    }

    #[test]
    fn two_nonterminating_output_readers_share_one_absolute_cleanup_deadline() {
        let released = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut process = TerminalTreeWithTwoHeldReaders {
            stdout: StreamTail::spawn(NeverEndingStream {
                released: Arc::clone(&released),
            }),
            stderr: StreamTail::spawn(NeverEndingStream {
                released: Arc::clone(&released),
            }),
            cancelled: false,
            output_incomplete: false,
        };
        let started = Instant::now();
        cancel_and_reap_with_budget(&mut process, Duration::from_millis(80))
            .expect_err("unfinished readers keep cleanup ownership uncertain");

        assert!(process.output_incomplete);
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "two inherited readers spent more than one cleanup budget"
        );
        released.store(true, std::sync::atomic::Ordering::Release);
    }

    #[test]
    fn a_held_lifecycle_guard_refuses_a_caller_instead_of_blocking_it() {
        let cache = TestCache::new();
        let store = RuntimeJobStore::new(cache.path(), DEFAULT_STALE_AFTER);
        let _held = store
            .acquire_active_lifecycle_lock()
            .expect("hold the lifecycle guard");

        let error = store
            .acquire_active_lifecycle_lock_until(Instant::now())
            .expect_err("a bounded acquisition must refuse a held guard");

        assert!(error.contains("still holds the workspace guard"), "{error}");
    }

    #[test]
    fn worker_stream_tail_redacts_output_before_retaining_it() {
        let mut tail = StreamTail::spawn(Cursor::new(
            b"build started\nPwd=stream-secret\ncompleted\n".to_vec(),
        ));
        assert!(!tail.finish().expect("finish output reader"));
        let output = tail.tail(OUTPUT_TAIL_BYTES).expect("read output tail");
        let (receipt, receipt_truncated) =
            tail.receipt().expect("inspect disabled receipt capture");

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
        assert!(!tail.finish().expect("finish output reader"));
        let output = tail.tail(OUTPUT_TAIL_BYTES).expect("read output tail");
        let (receipt, receipt_truncated) = tail.receipt().expect("read classification receipt");
        let argv = vec!["--json-message".to_string(), "build".to_string()];

        assert_eq!(output.len(), OUTPUT_TAIL_BYTES);
        assert!(receipt.is_none());
        assert!(receipt_truncated);
        assert_eq!(
            classify_partial_platform_failure(&BuildAttempt {
                argv: &argv,
                process_exit_code: Some(4),
                status_success: false,
                timed_out: false,
                cancelled: false,
                stdout_truncated: receipt_truncated,
                stdout_had_invalid_utf8: false,
                stdout: receipt.as_deref().unwrap_or_default(),
            }),
            Err(FallbackRejection::Receipt(
                "the captured output was truncated"
            )),
            "a valid JSON tail must not hide discarded stdout bytes"
        );
    }

    #[test]
    fn redacted_worker_logs_do_not_corrupt_transient_fallback_evidence() {
        let receipt = completed_partial_failure_json()
            .replace("/tmp/partial.lst", "/tmp/token=durable-secret/partial.lst");
        let mut tail = StreamTail::spawn_with_receipt(Cursor::new(receipt.into_bytes()));
        assert!(!tail.finish().expect("finish output reader"));
        let output = tail.tail(OUTPUT_TAIL_BYTES).expect("read output tail");
        let (receipt, receipt_truncated) = tail.receipt().expect("read classification receipt");
        let receipt = receipt.expect("complete raw receipt");
        let argv = vec!["--json-message".to_string(), "build".to_string()];

        assert!(!output.contains("durable-secret"));
        assert!(!receipt_truncated);
        assert!(
            classify_partial_platform_failure(&BuildAttempt {
                argv: &argv,
                process_exit_code: Some(4),
                status_success: false,
                timed_out: false,
                cancelled: false,
                stdout_truncated: receipt_truncated,
                stdout_had_invalid_utf8: false,
                stdout: &receipt,
            })
            .is_ok(),
            "redaction for persisted logs must not change transient retry evidence"
        );
    }

    #[test]
    fn redacted_tail_truncation_does_not_reject_a_complete_raw_receipt() {
        let expanded_log = "token=x,".repeat(600);
        let receipt = completed_partial_failure_json().replace("sanitized", &expanded_log);
        assert!(receipt.len() <= OUTPUT_TAIL_BYTES);
        let mut tail = StreamTail::spawn_with_receipt(Cursor::new(receipt.into_bytes()));
        assert!(!tail.finish().expect("finish output reader"));
        let redacted = tail.tail(OUTPUT_TAIL_BYTES).expect("read redacted tail");
        let (receipt, receipt_truncated) = tail.receipt().expect("read classification receipt");
        let receipt = receipt.expect("complete raw receipt");
        let argv = vec!["--json-message".to_string(), "build".to_string()];

        // Redaction expanded the persisted tail past its cap while the raw
        // receipt stayed whole, which is exactly the split the classifier reads.
        assert_eq!(redacted.len(), OUTPUT_TAIL_BYTES);
        assert!(!receipt_truncated);
        assert!(
            classify_partial_platform_failure(&BuildAttempt {
                argv: &argv,
                process_exit_code: Some(4),
                status_success: false,
                timed_out: false,
                cancelled: false,
                stdout_truncated: receipt_truncated,
                stdout_had_invalid_utf8: false,
                stdout: &receipt,
            })
            .is_ok(),
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
                stdout_had_invalid_utf8: false,
                stdout: &receipt,
            })
            .is_ok(),
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
            stdout_had_invalid_utf8: false,
            stdout: &receipt,
        })
        .is_ok());
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
    fn stale_job_that_reached_its_child_spawn_is_lost_and_retains_the_active_lock() {
        let cache = TestCache::new();
        let request = fake_request(RuntimeJobOperation::Build);
        let queued = RuntimeJobService::enqueue(cache.path(), &request).expect("queue stale job");
        let store = RuntimeJobStore::new(cache.path(), DEFAULT_STALE_AFTER);
        let mut record = store.read_record(&queued.id).expect("read queued record");
        record.heartbeat_at_ms = Some(0);
        // The worker persisted its intent to spawn, so an orphaned child tree may
        // still be mutating this workspace.
        record.child_spawn_attempted = Some(true);
        store.write_record(&record).expect("age queued record");

        let service = RuntimeJobService::new(cache.path(), Arc::new(FakeRunner::success_after(2)));
        let recovered = RuntimeJobService::status_at(cache.path(), &queued.id)
            .expect("status recovers stale job");
        let error = service
            .start(fake_request(RuntimeJobOperation::Test))
            .expect_err("a possibly live orphan must continue to block replacement work");

        assert_eq!(recovered.phase, RuntimeJobPhase::Lost);
        assert!(error.contains(&queued.id), "{error}");
        assert!(
            error.contains("child process may still be running"),
            "a lock that is never released automatically must say why: {error}"
        );
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

    /// Reproduces the parent that acquired `active.lock` and created the durable
    /// record, then died without ever terminalizing it. The five minute staleness
    /// window has already elapsed.
    fn abandoned_queued_job(cache: &TestCache) -> (RuntimeJobStore, String) {
        let store = RuntimeJobStore::new(cache.path(), DEFAULT_STALE_AFTER);
        let queued =
            RuntimeJobService::enqueue(cache.path(), &fake_request(RuntimeJobOperation::Build))
                .expect("queue job before the parent dies");
        let mut record = store.read_record(&queued.id).expect("read queued record");
        record.heartbeat_at_ms = Some(0);
        store.write_record(&record).expect("age queued record");
        (store, queued.id)
    }

    fn assert_childless_handoff_failure_frees_the_workspace(frame: &[u8]) {
        let cache = TestCache::new();
        let (store, abandoned) = abandoned_queued_job(&cache);

        // The worker cannot decode its identity, so it exits without touching
        // the durable record or the lock it never learned about.
        run_worker_from_reader(frame)
            .expect_err("a first frame that never decodes cannot claim a job");
        assert_eq!(
            store
                .read_record(&abandoned)
                .expect("record survives the failed handoff")
                .phase,
            RuntimeJobPhase::Queued
        );

        let service = RuntimeJobService::new(cache.path(), Arc::new(FakeRunner::success_after(2)));
        let replacement = service
            .start(fake_request(RuntimeJobOperation::Test))
            .expect("a job whose child never existed must not wedge the workspace");

        assert_eq!(
            store
                .read_record(&abandoned)
                .expect("read recovered record")
                .phase,
            RuntimeJobPhase::Lost,
            "the abandoned record must reach a terminal phase"
        );
        assert_eq!(
            fs::read_to_string(store.active_lock_path())
                .expect("read active lock")
                .trim(),
            replacement.id,
            "the released lock must be owned by the replacement job"
        );
        assert!(!store.recovery_lock_path().exists());
    }

    #[test]
    fn eof_before_the_first_handoff_frame_frees_a_childless_workspace() {
        assert_childless_handoff_failure_frees_the_workspace(b"");
    }

    #[test]
    fn malformed_first_handoff_frame_frees_a_childless_workspace() {
        assert_childless_handoff_failure_frees_the_workspace(b"{not json");
    }

    #[test]
    fn truncated_first_handoff_frame_frees_a_childless_workspace() {
        let cache = TestCache::new();
        let request = fake_request(RuntimeJobOperation::Build);
        let frame = serde_json::to_vec(&worker_request(
            &cache,
            &Uuid::new_v4().to_string(),
            &request,
        ))
        .expect("serialize handoff frame");
        let truncated = &frame[..frame.len() / 2];

        assert_childless_handoff_failure_frees_the_workspace(truncated);
    }

    #[test]
    fn parent_that_died_before_spawning_a_worker_frees_a_childless_workspace() {
        // No worker exists at all, so no worker-side identity could ever recover
        // this. Only the durable record can prove the workspace is free.
        let cache = TestCache::new();
        let (store, abandoned) = abandoned_queued_job(&cache);

        let service = RuntimeJobService::new(cache.path(), Arc::new(FakeRunner::success_after(2)));
        let replacement = service
            .start(fake_request(RuntimeJobOperation::Test))
            .expect("an unspawned worker must not wedge the workspace");

        assert_eq!(
            store
                .read_record(&abandoned)
                .expect("read recovered record")
                .phase,
            RuntimeJobPhase::Lost
        );
        assert_eq!(
            fs::read_to_string(store.active_lock_path())
                .expect("read active lock")
                .trim(),
            replacement.id
        );
    }

    #[test]
    fn recovery_never_releases_a_lock_owned_by_another_job() {
        let cache = TestCache::new();
        let (store, abandoned) = abandoned_queued_job(&cache);

        // A replacement owner claimed the lock while the abandoned record was
        // still non-terminal. Recovery is scoped to the id the lock names, so the
        // childless record next to it never becomes a reason to free somebody
        // else's workspace.
        let replacement = Uuid::new_v4().to_string();
        store
            .create_record(&replacement, &fake_request(RuntimeJobOperation::Test))
            .expect("create replacement record");
        fs::write(store.active_lock_path(), &replacement).expect("replace active lock");

        RuntimeJobService::status_at(cache.path(), &abandoned).expect("recover stale job");

        assert_eq!(
            fs::read_to_string(store.active_lock_path())
                .expect("read replacement active lock")
                .trim(),
            replacement
        );
        assert_eq!(
            store
                .read_record(&abandoned)
                .expect("read untouched record")
                .phase,
            RuntimeJobPhase::Queued,
            "recovery must not terminalize a record the lock does not name"
        );
    }

    #[test]
    fn a_lock_taken_before_its_record_existed_frees_a_childless_workspace() {
        // The parent died between acquire_active_lock and create_record, so no
        // worker was ever spawned and the durable record does not exist at all.
        let cache = TestCache::new();
        let store = RuntimeJobStore::new(cache.path(), Duration::from_millis(1));
        let abandoned = Uuid::new_v4().to_string();
        store
            .acquire_active_lock(&abandoned)
            .expect("take a lock the parent never backed with a record");
        thread::sleep(Duration::from_millis(5));

        assert!(
            store
                .recover_stale_active()
                .expect("recover a record-less lock"),
            "a lock whose job never reached a record must be recoverable"
        );
        assert!(!store.active_lock_path().exists());
    }

    #[test]
    fn a_lock_taken_before_its_record_existed_is_kept_inside_the_staleness_window() {
        // A parent that is still between acquire_active_lock and create_record
        // must not be recovered out from underneath.
        let cache = TestCache::new();
        let store = RuntimeJobStore::new(cache.path(), DEFAULT_STALE_AFTER);
        let live = Uuid::new_v4().to_string();
        store.acquire_active_lock(&live).expect("take a fresh lock");

        assert!(
            !store
                .recover_stale_active()
                .expect("probe a fresh record-less lock"),
            "a fresh lock must survive recovery"
        );
        assert_eq!(
            fs::read_to_string(store.active_lock_path())
                .expect("read retained active lock")
                .trim(),
            live
        );
    }

    #[test]
    fn a_missing_jobs_root_is_not_proof_that_a_record_is_absent() {
        let cache = TestCache::new();
        let store = RuntimeJobStore::new(cache.path(), Duration::from_millis(1));
        let live = Uuid::new_v4().to_string();

        assert!(
            !store.record_is_provably_absent(&live),
            "absence is unproven until the jobs root is a real directory"
        );
    }

    #[test]
    fn a_linked_job_directory_is_not_proof_that_a_record_is_absent() {
        let cache = TestCache::new();
        let store = RuntimeJobStore::new(cache.path(), Duration::from_millis(1));
        let live = Uuid::new_v4().to_string();
        fs::create_dir_all(store.jobs_root()).expect("create jobs root");
        let linked_target = cache.path().join("linked-job-target");
        fs::create_dir_all(&linked_target).expect("create linked job target");
        let linked_job_dir = store.job_dir(&live).expect("job directory path");
        let outcome = create_directory_link_fixture_for_test(&linked_target, &linked_job_dir)
            .expect("create linked job directory fixture");
        if outcome != FileLinkFixtureOutcome::Created {
            return;
        }

        assert!(
            !store.record_is_provably_absent(&live),
            "a linked job directory cannot prove where the record would live"
        );
    }

    #[test]
    fn a_broken_record_link_is_not_proof_that_a_record_is_absent() {
        let cache = TestCache::new();
        let store = RuntimeJobStore::new(cache.path(), Duration::from_millis(1));
        let live = Uuid::new_v4().to_string();
        let record_path = store.record_path(&live).expect("record path");
        fs::create_dir_all(record_path.parent().expect("record parent"))
            .expect("create job directory");
        let missing_target = cache.path().join("missing-record-target.json");
        let outcome = create_file_link_fixture_for_test(&missing_target, &record_path)
            .expect("create broken record link fixture");
        if outcome != FileLinkFixtureOutcome::Created {
            return;
        }

        assert!(
            !store.record_is_provably_absent(&live),
            "a broken record link is unknown rather than absent"
        );
    }

    #[test]
    fn a_record_path_that_cannot_be_read_is_never_treated_as_absent() {
        // "I cannot tell" must never become the proof that no child exists: the
        // lock would be released while a job is still mutating the workspace.
        let cache = TestCache::new();
        let store = RuntimeJobStore::new(cache.path(), Duration::from_millis(1));
        let live = Uuid::new_v4().to_string();
        store.acquire_active_lock(&live).expect("take a lock");
        let job_dir = store.job_dir(&live).expect("job directory path");
        fs::write(&job_dir, "not a directory").expect("block the record path");
        thread::sleep(Duration::from_millis(5));

        assert!(
            !store.record_is_provably_absent(&live),
            "an unreadable record path is not proof of absence"
        );
        store
            .recover_stale_active()
            .expect_err("an unreadable record cannot prove the workspace is idle");
        assert!(
            store.active_lock_path().exists(),
            "the lock must be retained while the record cannot be read"
        );

        // The other side of the same rule: a job directory that is genuinely
        // missing does prove absence, on every platform. Without this the
        // guard above could be satisfied by never proving absence at all.
        let never_started = Uuid::new_v4().to_string();
        assert!(
            store.record_is_provably_absent(&never_started),
            "an absent job directory proves the record is absent"
        );
    }

    #[test]
    fn a_lock_naming_something_that_is_not_a_job_id_is_never_treated_as_absent() {
        let cache = TestCache::new();
        let store = RuntimeJobStore::new(cache.path(), Duration::from_millis(1));
        fs::create_dir_all(store.jobs_root()).expect("create jobs root");
        fs::write(store.active_lock_path(), "not-a-uuid").expect("write a corrupt lock");
        thread::sleep(Duration::from_millis(5));

        assert!(!store.record_is_provably_absent("not-a-uuid"));
        store
            .recover_stale_active()
            .expect_err("a lock that names no job proves nothing about a child");
        assert!(store.active_lock_path().exists());
    }

    #[test]
    fn a_legacy_record_without_the_spawn_flag_is_lost_and_retains_the_active_lock() {
        // A record written before `childSpawnAttempted` existed proves nothing:
        // the previous worker could die after `runner.spawn()` and before it
        // persisted `pid`, leaving a live child behind a queued record. Absence
        // of the flag is unknown, never "no child was spawned".
        let cache = TestCache::new();
        let (store, abandoned) = abandoned_queued_job(&cache);
        let path = store.record_path(&abandoned).expect("record path");
        let mut value: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).expect("read record"))
                .expect("parse record");
        value
            .as_object_mut()
            .expect("record object")
            .remove("childSpawnAttempted");
        assert_eq!(
            value["pid"],
            serde_json::Value::Null,
            "the legacy shape under test carries no pid either"
        );
        fs::write(
            &path,
            serde_json::to_vec(&value).expect("serialize legacy record"),
        )
        .expect("write legacy record");

        let service = RuntimeJobService::new(cache.path(), Arc::new(FakeRunner::success_after(2)));
        let error = service
            .start(fake_request(RuntimeJobOperation::Test))
            .expect_err("an unprovable legacy record must not free the workspace");

        assert!(error.contains(&abandoned), "{error}");
        assert_eq!(
            store
                .read_record(&abandoned)
                .expect("read recovered record")
                .phase,
            RuntimeJobPhase::Lost,
            "the record must still be terminalized"
        );
        assert_eq!(
            fs::read_to_string(store.active_lock_path())
                .expect("read retained active lock")
                .trim(),
            abandoned,
            "the lock must be retained while the child cannot be ruled out"
        );
    }

    #[test]
    fn a_record_written_by_a_later_build_stays_readable_and_recoverable() {
        let cache = TestCache::new();
        let (store, abandoned) = abandoned_queued_job(&cache);

        // A later build added a field this build does not know. Rejecting the
        // record would pin active.lock with no automatic recovery at all.
        let path = store.record_path(&abandoned).expect("record path");
        let mut value: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).expect("read record"))
                .expect("parse record");
        value["fieldFromALaterBuild"] = serde_json::Value::Bool(true);
        fs::write(&path, serde_json::to_vec(&value).expect("serialize record"))
            .expect("write forward-compatible record");

        assert_eq!(
            store
                .read_record(&abandoned)
                .expect("a later build's record must stay readable")
                .phase,
            RuntimeJobPhase::Queued
        );

        let service = RuntimeJobService::new(cache.path(), Arc::new(FakeRunner::success_after(2)));
        service
            .start(fake_request(RuntimeJobOperation::Test))
            .expect("recovery must still free a provably childless workspace");
    }

    pub(crate) fn assert_system_cancellation_reaps_process_tree() {
        let cache = TestCache::new();
        fs::create_dir_all(cache.path()).expect("create worker cwd");
        let scenario = crate::infrastructure::platform::runtime_process_tree_test_scenario_for_test(
            &cache.path(),
        );
        let runner = SystemRuntimeJobRunner {
            program: scenario.program(),
            cwd: cache.path().to_path_buf(),
        };
        let request = RuntimeJobRequest::new(
            RuntimeJobOperation::Test,
            scenario.long_lived_args(),
            "workspace:test".to_string(),
            None,
        );
        let mut process = runner.spawn(&request).expect("spawn process group");
        let descendant = scenario
            .wait_for_descendant(Duration::from_secs(5))
            .expect("observe runtime-job descendant");

        cancel_and_reap(&mut *process).expect("cancel and reap process group");

        assert!(
            !descendant.is_alive().expect("probe runtime-job descendant"),
            "the process tree must no longer be alive"
        );
    }

    #[test]
    fn system_runtime_job_keeps_resource_owned_after_leader_exit_until_descendant_dies() {
        let cache = TestCache::new();
        fs::create_dir_all(cache.path()).expect("create runtime cwd");
        let scenario = crate::infrastructure::platform::runtime_process_tree_test_scenario_for_test(
            &cache.path(),
        );
        let runner = Arc::new(SystemRuntimeJobRunner {
            program: scenario.program(),
            cwd: cache.path().to_path_buf(),
        });
        let service = RuntimeJobService::new(cache.path(), runner);
        let request = RuntimeJobRequest::new(
            RuntimeJobOperation::Test,
            scenario.leader_with_descendant_args(),
            "workspace:test".to_string(),
            None,
        );
        let started = service.start(request).expect("start runtime process tree");
        let descendant = scenario
            .wait_for_descendant(Duration::from_secs(5))
            .expect("observe runtime descendant");

        // Poll until the platform handle has reaped the leader. Reaching
        // Running with `leader_exited` proves the retained process group/Job
        // Object query still observes the descendant.
        let observation_started = Instant::now();
        let observed = loop {
            let snapshot = service.poll(&started.id).expect("observe process tree");
            let leader_exited = service
                .lock_processes()
                .ok()
                .and_then(|processes| {
                    processes
                        .get(&started.id)
                        .map(|process| process.process.leader_exited_for_test())
                })
                .unwrap_or(false);
            if leader_exited || snapshot.phase.is_terminal() {
                break snapshot;
            }
            assert!(
                observation_started.elapsed() < Duration::from_secs(5),
                "leader did not exit"
            );
            thread::sleep(Duration::from_millis(10));
        };
        let active_lock_held = service.store.active_lock_path().exists();

        assert_eq!(observed.phase, RuntimeJobPhase::Running);
        assert!(
            descendant
                .is_alive()
                .expect("probe descendant after leader exit"),
            "descendant was not alive after leader exit"
        );
        assert!(
            active_lock_held,
            "active.lock was released before tree death"
        );

        // Cancellation owns and reaps the complete retained tree before the
        // terminal record can release the workspace resource.
        let mut cancelled = service.cancel(&started.id).expect("request cancellation");
        let cancel_started = Instant::now();
        while !cancelled.phase.is_terminal() && cancel_started.elapsed() < Duration::from_secs(5) {
            thread::sleep(Duration::from_millis(10));
            cancelled = service.poll(&started.id).expect("poll cancellation");
        }

        assert!(
            cancelled.phase.is_terminal(),
            "cancelled tree stayed active"
        );
        assert!(!service.store.active_lock_path().exists());
        assert!(
            !descendant
                .is_alive()
                .expect("probe descendant after cancellation"),
            "cancel returned before the owned descendant died"
        );
    }

    #[test]
    fn runtime_shared_work_joins_only_the_exact_active_resource_and_lease() {
        let cache = TestCache::new();
        let service = RuntimeJobService::new(cache.path(), Arc::new(FakeRunner::success_after(2)));
        let started = service
            .start(fake_request(RuntimeJobOperation::Test))
            .expect("start active runtime lease");
        let owner = RuntimeResourceOwner::default();
        let first = service
            .join_shared_work(&started.id, &owner, |_| Ok(()))
            .expect("join exact active runtime work");
        let second = service
            .join_shared_work(&started.id, &owner, |_| panic!("same lease ran twice"))
            .expect("join the same exact active runtime work");

        assert!(first.started_here());
        assert!(!second.started_here());
        assert!(service
            .join_shared_work(&Uuid::new_v4().to_string(), &owner, |_| Ok(()))
            .is_err());
        first.wait().expect("exact producer succeeds");
        second.wait().expect("exact follower succeeds");
    }

    #[test]
    fn runtime_shared_work_separates_physical_resources_and_distinct_v4_leases() {
        let cache_a = TestCache::new();
        let cache_b = TestCache::new();
        let service_a =
            RuntimeJobService::new(cache_a.path(), Arc::new(FakeRunner::success_after(2)));
        let service_b =
            RuntimeJobService::new(cache_b.path(), Arc::new(FakeRunner::success_after(2)));
        let lease_a = service_a
            .start(fake_request(RuntimeJobOperation::Test))
            .expect("start resource A");
        let lease_b = service_b
            .start(fake_request(RuntimeJobOperation::Test))
            .expect("start resource B");
        let owner = RuntimeResourceOwner::default();
        let producers = Arc::new(AtomicUsize::new(0));
        let work = || {
            let producers = Arc::clone(&producers);
            move |_| {
                producers.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        };

        let physical_a = service_a
            .join_shared_work(&lease_a.id, &owner, work())
            .expect("join physical resource A");
        let physical_b = service_b
            .join_shared_work(&lease_b.id, &owner, work())
            .expect("join physical resource B");
        let replacement_lease = Uuid::new_v4().to_string();
        fs::write(service_a.store.active_lock_path(), &replacement_lease)
            .expect("replace exact lease in the same physical lock");
        let different_lease = service_a
            .join_shared_work(&replacement_lease, &owner, work())
            .expect("join different exact v4 lease");

        assert!(physical_a.started_here());
        assert!(physical_b.started_here());
        assert!(different_lease.started_here());
        physical_a.wait().unwrap();
        physical_b.wait().unwrap();
        different_lease.wait().unwrap();
        assert_eq!(producers.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn stale_runtime_shared_work_authority_starts_no_producer_after_lock_replacement() {
        let cache = TestCache::new();
        let service = RuntimeJobService::new(cache.path(), Arc::new(FakeRunner::success_after(2)));
        let started = service
            .start(fake_request(RuntimeJobOperation::Test))
            .expect("start lease A");
        let replacement = Uuid::new_v4().to_string();
        let owner = RuntimeResourceOwner::default();
        let executions = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let executions_for_work = Arc::clone(&executions);

        let replacement_result = service.join_shared_work_after_capability(
            &started.id,
            &owner,
            move |_| {
                executions_for_work.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            },
            || {
                fs::remove_file(service.store.active_lock_path()).expect("remove lease A lock");
                fs::write(service.store.active_lock_path(), &replacement)
                    .expect("install lease B lock");
            },
        );
        assert!(
            replacement_result.is_err(),
            "replacement must invalidate retained authority before join"
        );

        assert_eq!(
            executions.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "a stale movable key admitted destructive work after lease replacement"
        );
    }

    #[test]
    fn replaced_jobs_directory_between_lifecycle_lock_and_admission_starts_no_producer() {
        let cache = TestCache::new();
        let service = RuntimeJobService::new(cache.path(), Arc::new(FakeRunner::success_after(2)));
        let started = service
            .start(fake_request(RuntimeJobOperation::Test))
            .expect("start lease in physical jobs root A");
        let owner = RuntimeResourceOwner::default();
        let executions = Arc::new(AtomicUsize::new(0));
        let executions_for_work = Arc::clone(&executions);
        let jobs = service.store.jobs_root();
        let original = cache.path().join("jobs-original");
        let replacement_lease = started.id.clone();
        let retained_identity = path_identity_for_test(&jobs)
            .unwrap()
            .expect("jobs root identity must be available on supported CI platforms");
        let replacement = std::cell::Cell::new(None);

        let result = service.join_shared_work_after_capability(
            &started.id,
            &owner,
            move |_| {
                executions_for_work.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
            || {
                let outcome = attempt_retained_directory_replacement_for_test(&jobs, &original)
                    .expect("attempt to move physical jobs root A");
                if outcome == RetainedDirectoryReplacementOutcome::Replaced {
                    fs::create_dir(&jobs).expect("install physical jobs root B");
                    fs::write(jobs.join("active.lock"), replacement_lease)
                        .expect("install matching-looking lease in B");
                }
                replacement.set(Some(outcome));
            },
        );

        match replacement.get().expect("replacement hook must execute") {
            RetainedDirectoryReplacementOutcome::Replaced => {
                assert!(
                    result.is_err(),
                    "a lifecycle lock for A admitted physical B"
                );
                assert_eq!(executions.load(Ordering::SeqCst), 0);

                fs::remove_dir_all(&jobs).expect("remove rejected physical jobs root B");
                fs::rename(&original, &jobs).expect("restore exact jobs root A before teardown");
            }
            RetainedDirectoryReplacementOutcome::PreventedByRetainedHandle => {
                assert_eq!(
                    path_identity_for_test(&jobs).unwrap().as_deref(),
                    Some(retained_identity.as_str()),
                    "a prevented replacement changed the retained jobs root identity"
                );
                assert!(!original.exists());
                result
                    .expect("the unchanged retained jobs root must remain admissible")
                    .wait()
                    .unwrap();
                assert_eq!(executions.load(Ordering::SeqCst), 1);
            }
        }
    }

    #[test]
    fn symlinked_jobs_directory_between_lifecycle_lock_and_admission_starts_no_producer() {
        if !crate::infrastructure::platform::unix_runtime_authority_tests_supported() {
            return;
        }

        let cache = TestCache::new();
        let service = RuntimeJobService::new(cache.path(), Arc::new(FakeRunner::success_after(2)));
        let started = service
            .start(fake_request(RuntimeJobOperation::Test))
            .expect("start lease in physical jobs root A");
        let owner = RuntimeResourceOwner::default();
        let executions = Arc::new(AtomicUsize::new(0));
        let executions_for_work = Arc::clone(&executions);
        let jobs = service.store.jobs_root();
        let original = cache.path().join("jobs-original");
        let replacement = cache.path().join("jobs-replacement");
        fs::create_dir(&replacement).expect("create replacement B");
        fs::write(replacement.join("active.lock"), &started.id)
            .expect("install matching-looking lease in B");

        let result = service.join_shared_work_after_capability(
            &started.id,
            &owner,
            move |_| {
                executions_for_work.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
            || {
                fs::rename(&jobs, &original).expect("move physical jobs root A");
                crate::infrastructure::platform::create_runtime_directory_link_for_test(
                    &replacement,
                    &jobs,
                )
                .expect("redirect jobs name to B");
            },
        );

        assert!(
            result.is_err(),
            "a lifecycle lock for A followed a link to B"
        );
        assert_eq!(executions.load(Ordering::SeqCst), 0);

        fs::remove_file(&jobs).expect("remove rejected jobs symlink to B");
        fs::rename(&original, &jobs).expect("restore exact jobs root A before teardown");
    }

    #[test]
    fn quarantine_release_never_removes_active_lock_from_replacement_jobs_root() {
        let cache = TestCache::new();
        let service = RuntimeJobService::new(cache.path(), Arc::new(FakeRunner::success_after(0)));
        let started = service
            .start(fake_request(RuntimeJobOperation::Test))
            .expect("start retained process in physical jobs root A");
        let mut record = service
            .store
            .read_record(&started.id)
            .expect("read running record");
        record
            .transition(RuntimeJobPhase::Lost)
            .expect("enter quarantine");
        service
            .store
            .write_record(&record)
            .expect("persist quarantine");
        let mut active = service
            .lock_processes()
            .expect("lock retained processes")
            .remove(&started.id)
            .expect("take retained process");
        let jobs = service.store.jobs_root();
        let original = cache.path().join("jobs-original");
        let replacement_id = started.id.clone();
        let retained_identity = path_identity_for_test(&jobs)
            .unwrap()
            .expect("jobs root identity must be available on supported CI platforms");
        let replacement = std::cell::Cell::new(None);

        let result = try_release_owned_quarantine_once_after_terminal(
            &service.store,
            &started.id,
            &mut active,
            || {
                replacement.set(Some(install_matching_replacement_jobs_root(
                    &jobs,
                    &original,
                    &replacement_id,
                )));
            },
        );

        match replacement.get().expect("replacement hook must execute") {
            RetainedDirectoryReplacementOutcome::Replaced => {
                assert!(
                    result.is_err(),
                    "root replacement must keep the terminal process quarantined"
                );
                assert_eq!(
                    fs::read_to_string(jobs.join("active.lock")).expect("read B active lock"),
                    started.id,
                    "quarantine release followed the ambient path and removed B"
                );
                assert!(
                    original.join("active.lock").exists(),
                    "failed exact validation must retain A's durable quarantine"
                );
            }
            RetainedDirectoryReplacementOutcome::PreventedByRetainedHandle => {
                assert_eq!(
                    path_identity_for_test(&jobs).unwrap().as_deref(),
                    Some(retained_identity.as_str()),
                    "a prevented replacement changed the retained jobs root identity"
                );
                assert!(!original.exists());
                assert!(result.expect("exact retained quarantine release"));
                assert!(
                    !service.store.active_lock_path().exists(),
                    "successful release left the exact original active lock behind"
                );
            }
        }
    }

    fn lost_quarantine_fixture() -> (
        TestCache,
        RuntimeJobService,
        String,
        ActiveRuntimeJobProcess,
    ) {
        let cache = TestCache::new();
        let service = RuntimeJobService::new(cache.path(), Arc::new(FakeRunner::success_after(0)));
        let started = service
            .start(fake_request(RuntimeJobOperation::Test))
            .expect("start retained process in physical jobs root A");
        let mut record = service.store.read_record(&started.id).unwrap();
        record.transition(RuntimeJobPhase::Lost).unwrap();
        service.store.write_record(&record).unwrap();
        let active = service
            .lock_processes()
            .unwrap()
            .remove(&started.id)
            .expect("take retained process");
        (cache, service, started.id, active)
    }

    fn install_matching_replacement_jobs_root(
        jobs: &Path,
        original: &Path,
        job_id: &str,
    ) -> RetainedDirectoryReplacementOutcome {
        let replacement = attempt_retained_directory_replacement_for_test(jobs, original)
            .expect("attempt to move retained jobs root A");
        if replacement == RetainedDirectoryReplacementOutcome::Replaced {
            fs::create_dir(jobs).expect("install replacement jobs root B");
            let replacement_job = jobs.join(job_id);
            fs::create_dir(&replacement_job).expect("install matching-looking job in B");
            fs::copy(
                original.join(job_id).join("record.json"),
                replacement_job.join("record.json"),
            )
            .expect("copy byte-identical quarantine record into B");
            fs::write(jobs.join("active.lock"), job_id)
                .expect("install matching-looking active lock in B");
        }
        replacement
    }

    fn install_matching_replacement_job_directory(
        jobs: &Path,
        job_id: &str,
        displaced_name: &str,
    ) -> (RetainedDirectoryReplacementOutcome, Vec<u8>) {
        let canonical = jobs.join(job_id);
        let displaced = jobs.join(displaced_name);
        let bytes = fs::read(canonical.join("record.json")).expect("save canonical record bytes");
        let replacement = attempt_retained_directory_replacement_for_test(&canonical, &displaced)
            .expect("attempt to move retained job directory A");
        if replacement == RetainedDirectoryReplacementOutcome::Replaced {
            fs::create_dir(&canonical).expect("install replacement job directory B");
            fs::write(canonical.join("record.json"), &bytes)
                .expect("install matching-looking record in B");
        }
        (replacement, bytes)
    }

    #[test]
    fn quarantine_record_read_never_follows_replacement_after_retained_validation() {
        let (_cache, service, job_id, mut active) = lost_quarantine_fixture();
        let jobs = service.store.jobs_root();
        let original = service.store.cache_root.join("jobs-original-read");
        let saved_b = service.store.cache_root.join("jobs-saved-b-read");
        let retained_identity = path_identity_for_test(&jobs)
            .unwrap()
            .expect("jobs root identity must be available on supported CI platforms");
        let replacement = std::cell::Cell::new(None);
        let expected_b = Arc::new(Mutex::new(Vec::new()));
        let expected_b_for_hook = Arc::clone(&expected_b);

        let result = try_release_owned_quarantine_once_after_record_hooks(
            &service.store,
            &job_id,
            &mut active,
            |point| match point {
                QuarantineReleaseHookPoint::AfterFinalValidationBeforeRead => {
                    let outcome = install_matching_replacement_jobs_root(&jobs, &original, &job_id);
                    replacement.set(Some(outcome));
                    if outcome == RetainedDirectoryReplacementOutcome::Replaced {
                        let path = jobs.join(&job_id).join("record.json");
                        let mut replacement: RuntimeJobRecord =
                            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
                        replacement.warnings.push("record-from-root-b".to_string());
                        let bytes = serde_json::to_vec_pretty(&replacement).unwrap();
                        fs::write(&path, &bytes).unwrap();
                        *expected_b_for_hook.lock().unwrap() = bytes;
                    }
                }
                QuarantineReleaseHookPoint::AfterRecordRead
                    if replacement.get() == Some(RetainedDirectoryReplacementOutcome::Replaced) =>
                {
                    fs::rename(&jobs, &saved_b).expect("retain replacement B for inspection");
                    fs::rename(&original, &jobs).expect("restore original jobs root A");
                }
                _ => {}
            },
        );

        match replacement.get().expect("replacement hook must execute") {
            RetainedDirectoryReplacementOutcome::Replaced => {
                assert!(result.expect("descriptor-relative A transition"));
                assert_eq!(
                    fs::read(saved_b.join(&job_id).join("record.json")).unwrap(),
                    *expected_b.lock().unwrap(),
                    "reading A followed the replacement namespace into B"
                );
            }
            RetainedDirectoryReplacementOutcome::PreventedByRetainedHandle => {
                assert!(result.expect("exact retained A transition"));
                assert_eq!(
                    path_identity_for_test(&jobs).unwrap().as_deref(),
                    Some(retained_identity.as_str()),
                    "a prevented replacement changed the retained jobs root identity"
                );
                assert!(!original.exists());
                assert!(!saved_b.exists());
                assert!(!service.store.active_lock_path().exists());
            }
        }
        assert!(
            !service
                .store
                .read_record(&job_id)
                .unwrap()
                .warnings
                .iter()
                .any(|warning| warning == "record-from-root-b"),
            "B's durable record was read and republished into A"
        );
    }

    #[test]
    fn quarantine_record_publish_never_writes_replacement_root() {
        let (_cache, service, job_id, mut active) = lost_quarantine_fixture();
        let jobs = service.store.jobs_root();
        let original = service.store.cache_root.join("jobs-original-publish");
        let expected_b = fs::read(service.store.record_path(&job_id).unwrap()).unwrap();
        let retained_identity = path_identity_for_test(&jobs)
            .unwrap()
            .expect("jobs root identity must be available on supported CI platforms");
        let replacement = std::cell::Cell::new(None);

        let result = try_release_owned_quarantine_once_after_record_hooks(
            &service.store,
            &job_id,
            &mut active,
            |point| {
                if point == QuarantineReleaseHookPoint::ImmediatelyBeforeRecordPublish {
                    replacement.set(Some(install_matching_replacement_jobs_root(
                        &jobs, &original, &job_id,
                    )));
                }
            },
        );

        match replacement.get().expect("replacement hook must execute") {
            RetainedDirectoryReplacementOutcome::Replaced => {
                assert!(
                    result.is_err(),
                    "replacement before publish must quarantine A"
                );
                assert_eq!(
                    fs::read(jobs.join(&job_id).join("record.json")).unwrap(),
                    expected_b,
                    "A-derived terminal state was published into B"
                );
                assert!(
                    original.join("active.lock").exists(),
                    "A was falsely released"
                );
            }
            RetainedDirectoryReplacementOutcome::PreventedByRetainedHandle => {
                assert!(result.expect("exact retained A publication"));
                assert_eq!(
                    path_identity_for_test(&jobs).unwrap().as_deref(),
                    Some(retained_identity.as_str()),
                    "a prevented replacement changed the retained jobs root identity"
                );
                assert!(!original.exists());
                assert!(!service.store.active_lock_path().exists());
                assert!(
                    service
                        .store
                        .read_record(&job_id)
                        .unwrap()
                        .finished_at_ms
                        .is_some(),
                    "the exact original record was not terminalized"
                );
            }
        }
    }

    #[test]
    fn quarantine_publication_never_releases_after_same_root_job_directory_replacement() {
        let (_cache, service, job_id, mut active) = lost_quarantine_fixture();
        let jobs = service.store.jobs_root();
        let canonical = jobs.join(&job_id);
        let displaced_name = format!("{job_id}-retained");
        let displaced = jobs.join(&displaced_name);
        let retained_identity = path_identity_for_test(&canonical)
            .unwrap()
            .expect("job directory identity must be available on supported CI platforms");
        let replacement = std::cell::Cell::new(None);
        let expected_b = Arc::new(Mutex::new(Vec::new()));
        let expected_b_for_hook = Arc::clone(&expected_b);

        let result = try_release_owned_quarantine_once_after_record_hooks(
            &service.store,
            &job_id,
            &mut active,
            |point| {
                if point == QuarantineReleaseHookPoint::ImmediatelyBeforeRecordPublish {
                    let (outcome, bytes) =
                        install_matching_replacement_job_directory(&jobs, &job_id, &displaced_name);
                    replacement.set(Some(outcome));
                    if outcome == RetainedDirectoryReplacementOutcome::Replaced {
                        *expected_b_for_hook.lock().unwrap() = bytes;
                    }
                }
            },
        );

        match replacement.get().expect("replacement hook must execute") {
            RetainedDirectoryReplacementOutcome::Replaced => {
                assert!(
                    result.is_err(),
                    "same-root job-directory replacement falsely confirmed publication"
                );
                assert_eq!(
                    fs::read(jobs.join(&job_id).join("record.json")).unwrap(),
                    *expected_b.lock().unwrap(),
                    "publication modified replacement job-directory B"
                );
                assert!(
                    service.store.active_lock_path().exists(),
                    "unconfirmed publication removed active.lock"
                );
                assert!(matches!(
                    active.process.try_wait(),
                    Ok(RuntimeJobProcessState::Exited { .. })
                ));

                let retry = try_release_owned_quarantine_once_after_record_hooks(
                    &service.store,
                    &job_id,
                    &mut active,
                    |_| {},
                );
                assert!(
                    retry.is_err(),
                    "second quarantine probe accepted replacement job-directory B"
                );
                assert_eq!(
                    fs::read(jobs.join(&job_id).join("record.json")).unwrap(),
                    *expected_b.lock().unwrap(),
                    "later quarantine probe modified replacement job-directory B"
                );
                assert!(
                    service.store.active_lock_path().exists(),
                    "later quarantine probe removed active.lock"
                );
            }
            RetainedDirectoryReplacementOutcome::PreventedByRetainedHandle => {
                assert!(result.expect("exact retained job publication"));
                assert_eq!(
                    path_identity_for_test(&canonical).unwrap().as_deref(),
                    Some(retained_identity.as_str()),
                    "a prevented replacement changed the retained job directory identity"
                );
                assert!(!displaced.exists());
                assert!(!service.store.active_lock_path().exists());
                assert!(
                    service
                        .store
                        .read_record(&job_id)
                        .unwrap()
                        .finished_at_ms
                        .is_some(),
                    "the exact original record was not terminalized"
                );
            }
        }
        assert!(matches!(
            active.process.try_wait(),
            Ok(RuntimeJobProcessState::Exited { .. })
        ));
    }

    #[test]
    fn quarantine_post_rename_flush_failure_never_releases_active_lock() {
        let (_cache, service, job_id, mut active) = lost_quarantine_fixture();
        let before = fs::read(service.store.record_path(&job_id).unwrap()).unwrap();
        crate::infrastructure::platform::filesystem::inject_post_rename_sync_failure_for_test();

        let result = try_release_owned_quarantine_once_after_record_hooks(
            &service.store,
            &job_id,
            &mut active,
            |_| {},
        );

        assert!(
            result.is_err(),
            "post-rename durability failure was reported as successful publication"
        );
        assert_ne!(
            fs::read(service.store.record_path(&job_id).unwrap()).unwrap(),
            before,
            "failure injection ran before the descriptor-relative rename"
        );
        assert!(
            service.store.active_lock_path().exists(),
            "post-rename durability failure removed active.lock"
        );
        assert!(matches!(
            active.process.try_wait(),
            Ok(RuntimeJobProcessState::Exited { .. })
        ));

        assert!(try_release_owned_quarantine_once_after_record_hooks(
            &service.store,
            &job_id,
            &mut active,
            |_| {},
        )
        .expect("transient descriptor flush failure remains retryable"));
        assert!(
            !service.store.active_lock_path().exists(),
            "successful retry did not release the exact retained active.lock"
        );
    }

    #[test]
    fn quarantine_post_publication_confirmation_rejects_same_root_job_directory_swap() {
        let (_cache, service, job_id, mut active) = lost_quarantine_fixture();
        let jobs = service.store.jobs_root();
        let canonical = jobs.join(&job_id);
        let displaced_name = format!("{job_id}-published");
        let displaced = jobs.join(&displaced_name);
        let retained_identity = path_identity_for_test(&canonical)
            .unwrap()
            .expect("job directory identity must be available on supported CI platforms");
        let replacement = std::cell::Cell::new(None);
        let expected_b = Arc::new(Mutex::new(Vec::new()));
        let expected_b_for_hook = Arc::clone(&expected_b);

        let result = try_release_owned_quarantine_once_after_record_hooks(
            &service.store,
            &job_id,
            &mut active,
            |point| {
                if point == QuarantineReleaseHookPoint::AfterRecordPublishBeforeConfirmation {
                    let (outcome, bytes) =
                        install_matching_replacement_job_directory(&jobs, &job_id, &displaced_name);
                    replacement.set(Some(outcome));
                    if outcome == RetainedDirectoryReplacementOutcome::Replaced {
                        *expected_b_for_hook.lock().unwrap() = bytes;
                    }
                }
            },
        );

        match replacement.get().expect("replacement hook must execute") {
            RetainedDirectoryReplacementOutcome::Replaced => {
                assert!(
                    result.is_err(),
                    "post-publication job-directory swap bypassed final confirmation"
                );
                assert_eq!(
                    fs::read(jobs.join(&job_id).join("record.json")).unwrap(),
                    *expected_b.lock().unwrap(),
                    "final confirmation modified replacement job-directory B"
                );
                assert!(service.store.active_lock_path().exists());

                let retry = try_release_owned_quarantine_once_after_record_hooks(
                    &service.store,
                    &job_id,
                    &mut active,
                    |_| {},
                );
                assert!(
                    retry.is_err(),
                    "second post-publication probe accepted replacement job-directory B"
                );
                assert_eq!(
                    fs::read(jobs.join(&job_id).join("record.json")).unwrap(),
                    *expected_b.lock().unwrap(),
                    "later post-publication probe modified replacement job-directory B"
                );
                assert!(service.store.active_lock_path().exists());
            }
            RetainedDirectoryReplacementOutcome::PreventedByRetainedHandle => {
                assert!(result.expect("exact retained job confirmation"));
                assert_eq!(
                    path_identity_for_test(&canonical).unwrap().as_deref(),
                    Some(retained_identity.as_str()),
                    "a prevented replacement changed the retained job directory identity"
                );
                assert!(!displaced.exists());
                assert!(!service.store.active_lock_path().exists());
                assert!(
                    service
                        .store
                        .read_record(&job_id)
                        .unwrap()
                        .finished_at_ms
                        .is_some(),
                    "the exact original record was not terminalized"
                );
            }
        }
        assert!(matches!(
            active.process.try_wait(),
            Ok(RuntimeJobProcessState::Exited { .. })
        ));
    }

    #[test]
    fn quarantine_record_transition_completes_only_in_exact_retained_root() {
        let (_cache, service, job_id, mut active) = lost_quarantine_fixture();

        assert!(try_release_owned_quarantine_once_after_record_hooks(
            &service.store,
            &job_id,
            &mut active,
            |_| {},
        )
        .expect("exact retained transition"));
        assert!(!service.store.active_lock_path().exists());
        let record = service.store.read_record(&job_id).unwrap();
        assert_eq!(record.phase, RuntimeJobPhase::Lost);
        assert!(record.finished_at_ms.is_some());
    }

    #[test]
    fn uncertain_post_spawn_failure_retains_active_lock_for_initial_attempt() {
        let cache = TestCache::new();
        let runner = Arc::new(UncertainSpawnRunner::fail_on_attempt(0, None));
        let service = RuntimeJobService::new(cache.path(), runner);

        service
            .start(fake_request(RuntimeJobOperation::Test))
            .expect_err("post-spawn ownership uncertainty must fail closed");

        assert!(
            service.store.active_lock_path().exists(),
            "post-spawn uncertainty released active.lock and admitted a replacement"
        );
        let active = fs::read_to_string(service.store.active_lock_path()).unwrap();
        assert_eq!(
            service.store.read_record(&active).unwrap().phase,
            RuntimeJobPhase::Lost
        );
    }

    #[test]
    fn retained_test_fake_supervisor_is_drained_before_authority_root_teardown() {
        let scope = TestSupervisorScope::new();
        {
            let cache = TestCache::new();
            let runner = Arc::new(UncertainSpawnRunner::fail_on_attempt(0, None));
            let service = RuntimeJobService::new(cache.path(), runner);

            service
                .start(fake_request(RuntimeJobOperation::Test))
                .expect_err("post-spawn ownership uncertainty must fail closed");
        }

        scope.assert_drained();
    }

    #[test]
    fn nested_retained_supervisor_is_not_hidden_from_outer_fixture_owner() {
        let scope = TestSupervisorScope::new();
        let nested_root;
        {
            let cache = TestCache::new();
            nested_root = cache.service_root("state");
            let runner = Arc::new(UncertainSpawnRunner::fail_on_attempt(0, None));
            let service = RuntimeJobService::new(nested_root.clone(), runner);

            service
                .start(fake_request(RuntimeJobOperation::Test))
                .expect_err("post-spawn ownership uncertainty must fail closed");
        }

        scope.assert_drained();
        let registry = test_fixture_registry()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(
            !registry.exact_roots.contains_key(&nested_root),
            "nested service root remained bound after owner teardown"
        );
    }

    #[test]
    fn nested_externally_terminal_uncontrolled_supervisor_is_owner_drained() {
        struct ExternallyTerminalRunner {
            dropped: Arc<AtomicBool>,
        }

        struct ExternallyTerminalProcess {
            dropped: Arc<AtomicBool>,
        }

        impl Drop for ExternallyTerminalProcess {
            fn drop(&mut self) {
                self.dropped.store(true, Ordering::Release);
            }
        }

        impl RuntimeJobProcess for ExternallyTerminalProcess {
            fn id(&self) -> u32 {
                93_001
            }

            fn try_wait(&mut self) -> JobResult<RuntimeJobProcessState> {
                Ok(RuntimeJobProcessState::Exited { exit_code: 1 })
            }

            fn cancel(&mut self) -> JobResult<()> {
                Ok(())
            }

            fn output_tails(&mut self, _max_bytes: usize) -> JobResult<RuntimeJobOutput> {
                Ok(RuntimeJobOutput::default())
            }
        }

        impl RuntimeJobRunner for ExternallyTerminalRunner {
            fn spawn(&self, _request: &RuntimeJobRequest) -> RuntimeJobSpawnResult {
                Err(RuntimeJobSpawnFailure::OwnershipRetained {
                    error: redacted_error("externally terminal retained test process"),
                    process: Box::new(ExternallyTerminalProcess {
                        dropped: Arc::clone(&self.dropped),
                    }),
                })
            }

            fn attach(&self, _process_id: u32) -> JobResult<Box<dyn RuntimeJobProcess>> {
                Err(redacted_error("externally terminal runner cannot attach"))
            }
        }

        let scope = TestSupervisorScope::new();
        let dropped = Arc::new(AtomicBool::new(false));
        {
            let cache = TestCache::new();
            let service = RuntimeJobService::new(
                cache.service_root("state"),
                Arc::new(ExternallyTerminalRunner {
                    dropped: Arc::clone(&dropped),
                }),
            );
            service
                .start(fake_request(RuntimeJobOperation::Test))
                .expect_err("retained external process must fail closed");
        }

        assert!(
            dropped.load(Ordering::Acquire),
            "owner teardown did not join the externally terminal production supervisor"
        );
        scope.assert_drained();
    }

    #[test]
    fn unbound_nested_test_service_fails_before_process_admission() {
        let cache = TestCache::new();
        let runner = Arc::new(FakeRunner::success_after(1));
        let service = RuntimeJobService::new(cache.path().join("unbound-state"), runner.clone());

        let error = service
            .start(fake_request(RuntimeJobOperation::Test))
            .expect_err("unbound nested fixture must fail closed before spawn");

        assert!(error.contains("explicit fixture owner binding"), "{error}");
        assert!(
            runner
                .processes
                .lock()
                .expect("lock fake processes")
                .is_empty(),
            "unbound nested fixture admitted a producer"
        );
    }

    #[test]
    fn fixture_owner_close_blocks_late_nested_admission() {
        let scope = TestSupervisorScope::new();
        let cache = TestCache::new();
        let service = RuntimeJobService::new(cache.path(), Arc::new(FakeRunner::success_after(1)));

        let live_admission = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            cache.drain_supervisors();
        }));
        assert!(
            live_admission.is_err(),
            "fixture teardown accepted a live runtime service admission"
        );
        let late_binding = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            cache.service_root("late-state");
        }));
        assert!(
            late_binding.is_err(),
            "closed fixture owner accepted a late nested service root"
        );

        drop(service);
        cache.drain_supervisors();
        drop(cache);
        scope.assert_drained();
    }

    #[test]
    fn unfinished_nested_supervisor_is_restored_with_same_owner_root_and_handle() {
        let scope = TestSupervisorScope::new();
        let cache = TestCache::new();
        let nested_root = cache.service_root("state");
        let admission = acquire_test_service_admission(&nested_root)
            .expect("acquire nested test service admission");
        let (release_tx, release_rx) = mpsc::sync_channel::<()>(1);
        let handle = thread::Builder::new()
            .name("unica-test-unfinished-quarantine".to_string())
            .spawn(move || {
                let _ = release_rx.recv();
            })
            .expect("spawn unfinished supervisor probe");
        let expected_thread = handle.thread().id();
        let (finished_tx, finished_rx) = mpsc::sync_channel::<()>(1);
        drop(finished_tx);
        register_test_quarantine_supervisor(
            cache.owner,
            nested_root.clone(),
            "00000000-0000-4000-8000-000000000001".to_string(),
            false,
            QuarantineSupervisor {
                handle,
                finished: finished_rx,
            },
        );
        drop(admission);

        let drain = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            cache.drain_supervisors();
        }));
        assert!(
            drain.is_err(),
            "unfinished nested supervisor was silently detached"
        );
        let restored = {
            let mut registry = test_fixture_registry()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let state = registry
                .owners
                .get_mut(&cache.owner)
                .expect("unfinished owner remains registered");
            assert_eq!(state.supervisors.len(), 1);
            let restored = state.supervisors.pop().expect("restored supervisor");
            assert_eq!(restored.service_root, nested_root);
            assert_eq!(restored.supervisor.handle.thread().id(), expected_thread);
            restored
        };
        release_tx.send(()).expect("release unfinished probe");
        restored
            .supervisor
            .handle
            .join()
            .expect("join restored supervisor handle");

        cache.drain_supervisors();
        drop(cache);
        scope.assert_drained();
    }

    #[test]
    fn ownership_retained_transition_never_writes_replacement_job_directory() {
        let cache = TestCache::new();
        let jobs_root = cache.path().join("jobs");
        let expected_replacement = Arc::new(Mutex::new(Vec::new()));
        let service = RuntimeJobService::new(
            cache.path(),
            Arc::new(JobDirectorySwapRunner {
                jobs_root: jobs_root.clone(),
                expected_replacement: Arc::clone(&expected_replacement),
                outcome: JobDirectorySwapOutcome::OwnershipRetained,
            }),
        );

        service
            .start(fake_request(RuntimeJobOperation::Test))
            .expect_err("ownership uncertainty after a namespace swap must fail closed");
        let job_id = fs::read_to_string(jobs_root.join("active.lock")).unwrap();
        assert_eq!(
            fs::read(jobs_root.join(&job_id).join("record.json")).unwrap(),
            *expected_replacement.lock().unwrap(),
            "ownership-retained transition wrote A's Lost state into replacement B"
        );
        assert!(service.has_retained_process(&job_id));
        assert!(service.store.active_lock_path().exists());

        service.lock_processes().unwrap().remove(&job_id);
    }

    #[test]
    fn failed_activation_cleanup_never_writes_replacement_job_directory() {
        let cache = TestCache::new();
        let jobs_root = cache.path().join("jobs");
        let expected_replacement = Arc::new(Mutex::new(Vec::new()));
        let service = RuntimeJobService::new(
            cache.path(),
            Arc::new(JobDirectorySwapRunner {
                jobs_root: jobs_root.clone(),
                expected_replacement: Arc::clone(&expected_replacement),
                outcome: JobDirectorySwapOutcome::ActivationCleanupFailure,
            }),
        );

        service
            .start(fake_request(RuntimeJobOperation::Test))
            .expect_err("failed activation cleanup after a namespace swap must quarantine");
        let job_id = fs::read_to_string(jobs_root.join("active.lock")).unwrap();
        assert_eq!(
            fs::read(jobs_root.join(&job_id).join("record.json")).unwrap(),
            *expected_replacement.lock().unwrap(),
            "failed activation cleanup wrote A's Lost state into replacement B"
        );
        assert!(service.has_retained_process(&job_id));
        assert!(service.store.active_lock_path().exists());

        service.lock_processes().unwrap().remove(&job_id);
    }

    #[test]
    fn quarantine_thread_spawn_failure_retains_process_authority_and_active_lock() {
        struct RetentionProbeRunner {
            dropped: Arc<AtomicBool>,
        }

        struct RetentionProbeProcess {
            dropped: Arc<AtomicBool>,
        }

        impl Drop for RetentionProbeProcess {
            fn drop(&mut self) {
                self.dropped.store(true, Ordering::Release);
            }
        }

        impl RuntimeJobProcess for RetentionProbeProcess {
            fn id(&self) -> u32 {
                77_331
            }

            fn try_wait(&mut self) -> JobResult<RuntimeJobProcessState> {
                Ok(RuntimeJobProcessState::Running)
            }

            fn cancel(&mut self) -> JobResult<()> {
                Err(redacted_error("injected retained ownership"))
            }

            fn output_tails(&mut self, _max_bytes: usize) -> JobResult<RuntimeJobOutput> {
                Ok(RuntimeJobOutput {
                    stdout: String::new(),
                    stderr: String::new(),
                    output_incomplete: false,
                    fallback_receipt: None,
                    fallback_receipt_truncated: false,
                })
            }
        }

        impl RuntimeJobRunner for RetentionProbeRunner {
            fn spawn(&self, _request: &RuntimeJobRequest) -> RuntimeJobSpawnResult {
                Err(RuntimeJobSpawnFailure::OwnershipRetained {
                    error: redacted_error("injected retained ownership"),
                    process: Box::new(RetentionProbeProcess {
                        dropped: Arc::clone(&self.dropped),
                    }),
                })
            }

            fn attach(&self, _process_id: u32) -> JobResult<Box<dyn RuntimeJobProcess>> {
                Err(redacted_error("probe runner cannot attach"))
            }
        }

        let cache = TestCache::new();
        let dropped = Arc::new(AtomicBool::new(false));
        let service = RuntimeJobService::new(
            cache.path(),
            Arc::new(RetentionProbeRunner {
                dropped: Arc::clone(&dropped),
            }),
        );
        service
            .start(fake_request(RuntimeJobOperation::Test))
            .expect_err("ownership uncertainty must return a closed failure");
        INJECT_QUARANTINE_THREAD_SPAWN_FAILURE.store(true, Ordering::Release);
        drop(service);

        assert!(
            !dropped.load(Ordering::Acquire),
            "thread spawn failure dropped the last retained process authority"
        );
        assert!(cache.path().join("jobs/active.lock").exists());
    }

    #[test]
    fn system_process_drop_uses_no_second_window_after_cleanup_deadline_is_consumed() {
        if !crate::infrastructure::platform::unix_runtime_authority_tests_supported() {
            return;
        }
        let released = Arc::new(AtomicBool::new(false));
        let (mut process, id) = system_process_for_authority_failure("exec sleep 30");
        process.stdout = Some(StreamTail::spawn(NeverEndingStream {
            released: Arc::clone(&released),
        }));
        process.stderr = Some(StreamTail::spawn(NeverEndingStream {
            released: Arc::clone(&released),
        }));
        crate::infrastructure::platform::reset_runtime_tree_cleanup_calls_for_test();
        crate::infrastructure::platform::inject_runtime_tree_cleanup_timeout_for_test();
        STREAM_FINISH_ATTEMPTS.with(|slot| slot.set(0));

        let retained = process.classify_failed_start(redacted_error("injected startup failure"));
        assert!(matches!(
            &retained,
            RuntimeJobSpawnFailure::OwnershipRetained { .. }
        ));
        assert_eq!(
            crate::infrastructure::platform::runtime_tree_cleanup_calls_for_test(),
            1
        );
        assert_eq!(
            STREAM_FINISH_ATTEMPTS.with(std::cell::Cell::get),
            2,
            "startup tree failure skipped one of the two output readers"
        );
        let drop_started = Instant::now();
        drop(retained);
        assert_eq!(
            crate::infrastructure::platform::runtime_tree_cleanup_calls_for_test(),
            1,
            "Drop opened a second process-tree cleanup attempt"
        );
        assert_eq!(
            STREAM_FINISH_ATTEMPTS.with(std::cell::Cell::get),
            2,
            "Drop opened a second output-reader cleanup attempt"
        );
        assert!(
            drop_started.elapsed() < Duration::from_millis(100),
            "Drop waited after the absolute cleanup deadline was consumed"
        );
        released.store(true, Ordering::Release);
        reap_raw_test_child(id);
    }

    #[test]
    fn external_cancel_error_binds_system_drop_to_the_same_cleanup_deadline() {
        if !crate::infrastructure::platform::unix_runtime_authority_tests_supported() {
            return;
        }
        let (mut process, id) = system_process_for_authority_failure("exec sleep 30");
        STREAM_FINISH_ATTEMPTS.with(|slot| slot.set(0));
        crate::infrastructure::platform::reset_runtime_tree_cleanup_calls_for_test();
        crate::infrastructure::platform::inject_unix_waitid_error_for_test();

        cancel_and_reap_with_budget(&mut process, Duration::from_millis(80))
            .expect_err("injected waitid failure must retain uncertain ownership");
        let reader_attempts = STREAM_FINISH_ATTEMPTS.with(std::cell::Cell::get);
        let tree_attempts = crate::infrastructure::platform::runtime_tree_cleanup_calls_for_test();
        assert_eq!(reader_attempts, 2);
        drop(process);
        assert_eq!(
            STREAM_FINISH_ATTEMPTS.with(std::cell::Cell::get),
            reader_attempts,
            "Drop repeated reader cleanup after external cancellation failed"
        );
        assert_eq!(
            crate::infrastructure::platform::runtime_tree_cleanup_calls_for_test(),
            tree_attempts,
            "Drop opened a second tree cleanup after external cancellation failed"
        );
        reap_raw_test_child(id);
    }

    fn system_process_for_authority_failure(script: &str) -> (SystemRuntimeJobProcess, u32) {
        let mut command = Command::new("/bin/sh");
        command
            .arg("-c")
            .arg(script)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let process_tree = RuntimeProcessTreeHandle::prepare(&mut command).unwrap();
        let mut child = command.spawn().unwrap();
        let id = child.id();
        let stdout = child.stdout.take().map(StreamTail::spawn);
        let stderr = child.stderr.take().map(StreamTail::spawn);
        let mut process = SystemRuntimeJobProcess {
            id,
            child,
            process_tree,
            stdout,
            stderr,
            exited: false,
            cleanup_deadline: None,
            cleanup_complete: false,
        };
        process.process_tree.attach(&mut process.child).unwrap();
        (process, id)
    }

    fn reap_raw_test_child(process_id: u32) {
        crate::infrastructure::platform::reap_runtime_authority_test_child(process_id);
    }

    #[test]
    fn waitid_authority_loss_makes_cancel_and_drop_send_zero_group_signals() {
        if !crate::infrastructure::platform::unix_runtime_authority_tests_supported() {
            return;
        }
        let (mut process, id) = system_process_for_authority_failure("exec sleep 30");
        crate::infrastructure::platform::reset_unix_signal_count_for_test();
        crate::infrastructure::platform::inject_unix_waitid_error_for_test();
        process
            .try_wait()
            .expect_err("injected waitid failure loses generation authority");
        process
            .cancel()
            .expect_err("cancel must fail closed after authority loss");
        drop(process);
        assert_eq!(
            crate::infrastructure::platform::unix_signal_count_for_test(),
            0,
            "cancel or Drop signalled after waitid authority loss"
        );
        reap_raw_test_child(id);
    }

    #[test]
    fn reap_authority_loss_makes_cancel_and_drop_send_zero_group_signals() {
        if !crate::infrastructure::platform::unix_runtime_authority_tests_supported() {
            return;
        }
        let gate = std::env::temp_dir().join(format!("unica-reap-gate-{}", Uuid::new_v4()));
        let script = format!("while [ ! -e '{}' ]; do :; done", gate.display());
        let (mut process, id) = system_process_for_authority_failure(&script);
        crate::infrastructure::platform::reset_unix_signal_count_for_test();
        crate::infrastructure::platform::inject_unix_reap_error_for_test();
        fs::write(&gate, b"go").unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match process.try_wait() {
                Err(error) => {
                    assert!(error.contains("injected Unix reap failure"), "{error}");
                    break;
                }
                Ok(RuntimeJobProcessState::Running) if Instant::now() < deadline => {
                    thread::yield_now();
                }
                other => panic!("reap failure was not observed: {other:?}"),
            }
        }
        process
            .cancel()
            .expect_err("cancel must fail closed after reap authority loss");
        drop(process);
        assert_eq!(
            crate::infrastructure::platform::unix_signal_count_for_test(),
            0,
            "cancel or Drop signalled after reap authority loss"
        );
        reap_raw_test_child(id);
        let _ = fs::remove_file(gate);
    }

    #[test]
    fn uncertain_post_spawn_failure_retains_active_lock_for_full_build_fallback() {
        let cache = TestCache::new();
        let first = FakeProcessState {
            polls_until_exit: 1,
            result: FakeResult::Exit(4),
            stdout: completed_partial_failure_json(),
            stderr: String::new(),
            cancel_calls: 0,
        };
        let runner = Arc::new(UncertainSpawnRunner::fail_on_attempt(1, Some(first)));
        let service = RuntimeJobService::new(cache.path(), runner);
        let started = service
            .start(fallback_build_request(&cache.path()))
            .expect("start first partial attempt");

        service
            .poll(&started.id)
            .expect_err("uncertain fallback ownership must fail closed");

        assert!(
            service.store.active_lock_path().exists(),
            "uncertain fallback ownership released active.lock"
        );
        assert_eq!(
            service.store.read_record(&started.id).unwrap().phase,
            RuntimeJobPhase::Lost
        );
    }

    fn complete_uncertain_process(runner: &UncertainSpawnRunner) {
        let state = runner
            .processes
            .lock()
            .expect("lock uncertain processes")
            .iter()
            .max_by_key(|(id, _)| *id)
            .map(|(_, state)| Arc::clone(state))
            .expect("retained uncertain process");
        state
            .lock()
            .expect("lock retained process")
            .polls_until_exit = 0;
    }

    fn wait_for_lost_record(store: &RuntimeJobStore, job_id: &str) {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if store
                .read_record(job_id)
                .is_ok_and(|record| record.phase == RuntimeJobPhase::Lost)
            {
                return;
            }
            assert!(Instant::now() < deadline, "worker never entered quarantine");
            thread::yield_now();
        }
    }

    fn assert_replacement_refused(cache: &TestCache, request: &RuntimeJobRequest) {
        RuntimeJobService::enqueue(cache.path(), request)
            .expect_err("retained process authority must refuse replacement admission");
    }

    fn assert_replacement_available(cache: &TestCache, request: &RuntimeJobRequest) {
        let replacement = RuntimeJobService::enqueue(cache.path(), request)
            .expect("terminal proof releases replacement admission");
        RuntimeJobStore::new(cache.path(), DEFAULT_STALE_AFTER)
            .release_active_lock_for(&replacement.id)
            .expect("release replacement fixture lock");
    }

    #[test]
    fn worker_supervises_initial_retained_ownership_until_proven_terminal() {
        let cache = TestCache::new();
        let request = fake_request(RuntimeJobOperation::Test);
        let queued = RuntimeJobService::enqueue(cache.path(), &request).expect("queue job");
        let handoff = worker_request(&cache, &queued.id, &request);
        let runner = Arc::new(UncertainSpawnRunner::fail_on_attempt(0, None));
        let worker_runner = Arc::clone(&runner);
        let (finished_tx, finished_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            finished_tx
                .send(run_worker_request(handoff, worker_runner))
                .expect("send worker result");
        });
        let store = RuntimeJobStore::new(cache.path(), DEFAULT_STALE_AFTER);
        wait_for_lost_record(&store, &queued.id);
        assert!(
            finished_rx.try_recv().is_err(),
            "worker abandoned retained authority"
        );
        assert!(store.active_lock_path().exists());
        assert_replacement_refused(&cache, &request);

        complete_uncertain_process(&runner);
        finished_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("worker did not finish supervision")
            .expect("supervisor exits after proof");
        worker.join().expect("join worker");
        assert!(!store.active_lock_path().exists());
        assert_replacement_available(&cache, &request);
    }

    #[test]
    fn worker_supervises_fallback_retained_ownership_until_proven_terminal() {
        let cache = TestCache::new();
        let first = FakeProcessState {
            polls_until_exit: 1,
            result: FakeResult::Exit(4),
            stdout: completed_partial_failure_json(),
            stderr: String::new(),
            cancel_calls: 0,
        };
        let request = fallback_build_request(&cache.path());
        let queued = RuntimeJobService::enqueue(cache.path(), &request).expect("queue build");
        let handoff = worker_request(&cache, &queued.id, &request);
        let runner = Arc::new(UncertainSpawnRunner::fail_on_attempt(1, Some(first)));
        let worker_runner = Arc::clone(&runner);
        let (finished_tx, finished_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            finished_tx
                .send(run_worker_request(handoff, worker_runner))
                .expect("send worker result");
        });
        let store = RuntimeJobStore::new(cache.path(), DEFAULT_STALE_AFTER);
        wait_for_lost_record(&store, &queued.id);
        assert!(
            finished_rx.try_recv().is_err(),
            "worker abandoned fallback authority"
        );
        assert!(store.active_lock_path().exists());
        assert_replacement_refused(&cache, &request);

        complete_uncertain_process(&runner);
        finished_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("fallback supervisor did not finish")
            .expect("supervisor exits after proof");
        worker.join().expect("join worker");
        assert!(!store.active_lock_path().exists());
        assert_replacement_available(&cache, &request);
    }

    struct ObservationFailureState {
        fail_poll: AtomicBool,
        fail_output: AtomicBool,
        terminal: AtomicBool,
        dropped: AtomicBool,
    }

    struct ObservationFailureRunner {
        state: Arc<ObservationFailureState>,
    }

    struct ActivationWriteFailureRunner {
        attempts: AtomicU32,
        first: Mutex<Option<FakeProcessState>>,
        retained: Arc<ObservationFailureState>,
    }

    struct ObservationFailureProcess {
        state: Arc<ObservationFailureState>,
    }

    impl Drop for ObservationFailureProcess {
        fn drop(&mut self) {
            self.state.dropped.store(true, Ordering::Release);
        }
    }

    impl RuntimeJobRunner for ObservationFailureRunner {
        fn spawn(&self, _request: &RuntimeJobRequest) -> RuntimeJobSpawnResult {
            Ok(Box::new(ObservationFailureProcess {
                state: Arc::clone(&self.state),
            }))
        }

        fn attach(&self, _process_id: u32) -> JobResult<Box<dyn RuntimeJobProcess>> {
            Err(redacted_error("observation-failure runner cannot attach"))
        }
    }

    impl RuntimeJobRunner for ActivationWriteFailureRunner {
        fn spawn(&self, _request: &RuntimeJobRequest) -> RuntimeJobSpawnResult {
            let attempt = self.attempts.fetch_add(1, Ordering::SeqCst);
            if attempt == 0 {
                if let Some(first) = self.first.lock().expect("lock first state").take() {
                    return Ok(Box::new(FakeProcess {
                        id: 88_880,
                        state: Arc::new(Mutex::new(first)),
                    }));
                }
            }
            INJECT_RUNTIME_RECORD_WRITE_FAILURE.store(true, Ordering::Release);
            Ok(Box::new(ObservationFailureProcess {
                state: Arc::clone(&self.retained),
            }))
        }

        fn attach(&self, _process_id: u32) -> JobResult<Box<dyn RuntimeJobProcess>> {
            Err(redacted_error(
                "activation-write-failure runner cannot attach",
            ))
        }
    }

    impl RuntimeJobProcess for ObservationFailureProcess {
        fn id(&self) -> u32 {
            88_881
        }

        fn try_wait(&mut self) -> JobResult<RuntimeJobProcessState> {
            if self.state.fail_poll.load(Ordering::Acquire) {
                return Err(redacted_error("injected retained poll failure"));
            }
            Ok(if self.state.terminal.load(Ordering::Acquire) {
                RuntimeJobProcessState::Exited { exit_code: 1 }
            } else {
                RuntimeJobProcessState::Running
            })
        }

        fn cancel(&mut self) -> JobResult<()> {
            Err(redacted_error("injected retained cancellation failure"))
        }

        fn output_tails(&mut self, _max_bytes: usize) -> JobResult<RuntimeJobOutput> {
            if self.state.fail_output.load(Ordering::Acquire) {
                return Err(redacted_error("injected retained output failure"));
            }
            Ok(RuntimeJobOutput::default())
        }

        fn prepare_controlled_test_supervision(&mut self) -> bool {
            self.state.fail_poll.store(false, Ordering::Release);
            self.state.fail_output.store(false, Ordering::Release);
            self.state.terminal.store(true, Ordering::Release);
            true
        }
    }

    struct ObservationFailureTerminalProof<'state> {
        state: &'state ObservationFailureState,
    }

    impl Drop for ObservationFailureTerminalProof<'_> {
        fn drop(&mut self) {
            self.state.fail_poll.store(false, Ordering::Release);
            self.state.fail_output.store(false, Ordering::Release);
            self.state.terminal.store(true, Ordering::Release);
        }
    }

    fn assert_worker_supervises_observation_failure(fail_poll: bool, fail_output: bool) {
        let cache = TestCache::new();
        let request = fake_request(RuntimeJobOperation::Test);
        let queued = RuntimeJobService::enqueue(cache.path(), &request).expect("queue job");
        let handoff = worker_request(&cache, &queued.id, &request);
        let state = Arc::new(ObservationFailureState {
            fail_poll: AtomicBool::new(fail_poll),
            fail_output: AtomicBool::new(fail_output),
            terminal: AtomicBool::new(fail_output),
            dropped: AtomicBool::new(false),
        });
        let runner = Arc::new(ObservationFailureRunner {
            state: Arc::clone(&state),
        });
        let (finished_tx, finished_rx) = mpsc::channel();
        let (worker_started_tx, worker_started_rx) = mpsc::sync_channel(1);
        let (quarantine_tx, quarantine_rx) = mpsc::sync_channel(1);
        thread::scope(|scope| {
            // If a Windows timing failure trips an assertion before the explicit
            // terminal proof below, release the controlled fake and join its
            // worker before `TestCache::drop` audits the supervisor registry.
            // This preserves the primary failure instead of aborting on a second
            // teardown panic.
            let _terminal_proof_on_unwind = ObservationFailureTerminalProof { state: &state };
            let worker = scope.spawn(move || {
                worker_started_tx.send(()).expect("announce worker start");
                finished_tx
                    .send(run_worker_request_after_quarantine_persisted(
                        handoff,
                        runner,
                        || quarantine_tx.send(()).expect("announce durable quarantine"),
                    ))
                    .expect("send worker result");
            });
            worker_started_rx
                .recv_timeout(Duration::from_secs(2))
                .expect("worker did not start");
            quarantine_rx
                .recv_timeout(Duration::from_secs(2))
                .expect("worker did not publish durable quarantine");
            let store = RuntimeJobStore::new(cache.path(), DEFAULT_STALE_AFTER);
            assert_eq!(
                store
                    .read_record(&queued.id)
                    .expect("read durable quarantine")
                    .phase,
                RuntimeJobPhase::Lost,
                "quarantine event preceded the durable Lost publication"
            );
            assert!(
                finished_rx.try_recv().is_err(),
                "worker exited with retained ownership"
            );
            assert!(store.active_lock_path().exists());
            assert_replacement_refused(&cache, &request);

            state.fail_poll.store(false, Ordering::Release);
            state.fail_output.store(false, Ordering::Release);
            state.terminal.store(true, Ordering::Release);
            finished_rx
                .recv_timeout(Duration::from_secs(2))
                .expect("worker did not finish retained supervision")
                .expect("worker finishes after exact terminal proof");
            worker.join().expect("join worker");
            assert!(!store.active_lock_path().exists());
            assert_replacement_available(&cache, &request);
        });
    }

    #[test]
    fn worker_quarantines_poll_failure_until_later_terminal_proof() {
        assert_worker_supervises_observation_failure(true, false);
    }

    #[test]
    fn worker_quarantines_output_failure_until_later_eof_proof() {
        assert_worker_supervises_observation_failure(false, true);
    }

    #[test]
    fn stale_local_worker_retains_process_until_later_terminal_proof() {
        let cache = TestCache::new();
        let request = fake_request(RuntimeJobOperation::Test);
        let queued = RuntimeJobService::enqueue(cache.path(), &request).expect("queue job");
        let handoff = worker_request(&cache, &queued.id, &request);
        let state = Arc::new(ObservationFailureState {
            fail_poll: AtomicBool::new(false),
            fail_output: AtomicBool::new(false),
            terminal: AtomicBool::new(false),
            dropped: AtomicBool::new(false),
        });
        let runner = Arc::new(ObservationFailureRunner {
            state: Arc::clone(&state),
        });
        let (finished_tx, finished_rx) = mpsc::channel();
        let (activation_tx, activation_rx) = mpsc::sync_channel(1);
        let (quarantine_tx, quarantine_rx) = mpsc::sync_channel(1);
        thread::scope(|scope| {
            // Preserve the primary assertion on slow Windows runners: any
            // unwind first makes the controlled process terminal, and the
            // scoped thread is joined before TestCache audits its owner.
            let _terminal_proof_on_unwind = ObservationFailureTerminalProof { state: &state };
            let worker = scope.spawn(move || {
                finished_tx
                    .send(
                        run_worker_request_after_activation_and_quarantine_persisted(
                            handoff,
                            runner,
                            || activation_tx.send(()).expect("announce durable activation"),
                            || quarantine_tx.send(()).expect("announce durable quarantine"),
                        ),
                    )
                    .expect("send worker result");
            });
            activation_rx
                .recv_timeout(Duration::from_secs(10))
                .expect("worker did not publish durable activation");
            let store = RuntimeJobStore::new(cache.path(), DEFAULT_STALE_AFTER);
            {
                let _lifecycle = store.acquire_active_lifecycle_lock().unwrap();
                let mut record = store.read_record(&queued.id).unwrap();
                assert_eq!(record.phase, RuntimeJobPhase::Running);
                record.heartbeat_at_ms = Some(0);
                store.write_record(&record).unwrap();
            }
            quarantine_rx
                .recv_timeout(Duration::from_secs(10))
                .expect("worker did not publish durable stale quarantine");
            assert_eq!(
                store.read_record(&queued.id).unwrap().phase,
                RuntimeJobPhase::Lost,
                "quarantine event preceded the durable Lost publication"
            );

            assert!(
                matches!(
                    finished_rx.recv_timeout(Duration::from_millis(100)),
                    Err(mpsc::RecvTimeoutError::Timeout)
                ),
                "stale polling let the canonical worker return with live authority"
            );
            assert!(
                !state.dropped.load(Ordering::Acquire),
                "stale polling dropped the retained process capability"
            );
            assert_replacement_refused(&cache, &request);

            state.terminal.store(true, Ordering::Release);
            finished_rx
                .recv_timeout(Duration::from_secs(10))
                .expect("worker did not finish stale-process supervision")
                .expect("terminal proof completes stale-process supervision");
            worker.join().expect("join worker");
            assert!(state.dropped.load(Ordering::Acquire));
            assert!(!store.active_lock_path().exists());
            assert_replacement_available(&cache, &request);
        });
    }

    fn assert_worker_supervises_post_spawn_record_failure(
        request: RuntimeJobRequest,
        first: Option<FakeProcessState>,
    ) {
        let cache = TestCache::new();
        let queued = RuntimeJobService::enqueue(cache.path(), &request).expect("queue job");
        let handoff = worker_request(&cache, &queued.id, &request);
        let retained = Arc::new(ObservationFailureState {
            fail_poll: AtomicBool::new(true),
            fail_output: AtomicBool::new(false),
            terminal: AtomicBool::new(false),
            dropped: AtomicBool::new(false),
        });
        let runner = Arc::new(ActivationWriteFailureRunner {
            attempts: AtomicU32::new(0),
            first: Mutex::new(first),
            retained: Arc::clone(&retained),
        });
        let (finished_tx, finished_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            finished_tx
                .send(run_worker_request(handoff, runner))
                .expect("send worker result");
        });
        let store = RuntimeJobStore::new(cache.path(), DEFAULT_STALE_AFTER);
        wait_for_lost_record(&store, &queued.id);
        assert!(finished_rx.try_recv().is_err());
        assert_replacement_refused(&cache, &request);

        retained.fail_poll.store(false, Ordering::Release);
        retained.terminal.store(true, Ordering::Release);
        finished_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("worker did not finish post-spawn quarantine")
            .expect("terminal proof releases quarantine");
        worker.join().expect("join worker");
        assert_replacement_available(&cache, &request);
    }

    #[test]
    fn worker_retains_initial_process_when_running_record_write_fails() {
        assert_worker_supervises_post_spawn_record_failure(
            fake_request(RuntimeJobOperation::Test),
            None,
        );
    }

    #[test]
    fn worker_retains_fallback_process_when_running_record_write_fails() {
        let cache = TestCache::new();
        let request = fallback_build_request(&cache.path());
        // Recreate the fixture inside the helper's cache-independent request
        // while keeping its captured workspace alive for reauthorization.
        let first = FakeProcessState {
            polls_until_exit: 1,
            result: FakeResult::Exit(4),
            stdout: completed_partial_failure_json(),
            stderr: String::new(),
            cancel_calls: 0,
        };
        assert_worker_supervises_post_spawn_record_failure(request, Some(first));
    }

    #[test]
    fn poisoned_process_registry_recovers_before_post_spawn_ownership_transfer() {
        let cache = TestCache::new();
        let service = RuntimeJobService::new(cache.path(), Arc::new(FakeRunner::success_after(1)));
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = service.processes.lock().expect("lock process registry");
            panic!("inject process registry poison");
        }));

        let started = service
            .start(fake_request(RuntimeJobOperation::Test))
            .expect("poison recovery must retain the spawned process");
        let terminal = service
            .poll(&started.id)
            .expect("observe recovered process");
        assert_eq!(terminal.phase, RuntimeJobPhase::Succeeded);
        assert!(!service.store.active_lock_path().exists());
    }

    #[test]
    fn proven_childless_spawn_failure_releases_initial_active_lock() {
        let cache = TestCache::new();
        let runner = Arc::new(UncertainSpawnRunner::proven_childless_on_attempt(0, None));
        let service = RuntimeJobService::new(cache.path(), runner);

        service
            .start(fake_request(RuntimeJobOperation::Test))
            .expect_err("proven terminal startup still reports the start failure");

        assert!(
            !service.store.active_lock_path().exists(),
            "only proven childless startup may release active.lock"
        );
    }

    #[test]
    fn proven_childless_fallback_spawn_failure_releases_active_lock() {
        let cache = TestCache::new();
        let first = FakeProcessState {
            polls_until_exit: 1,
            result: FakeResult::Exit(4),
            stdout: completed_partial_failure_json(),
            stderr: String::new(),
            cancel_calls: 0,
        };
        let runner = Arc::new(UncertainSpawnRunner::proven_childless_on_attempt(
            1,
            Some(first),
        ));
        let service = RuntimeJobService::new(cache.path(), runner);
        let started = service
            .start(fallback_build_request(&cache.path()))
            .expect("start partial attempt");

        let failed = service.poll(&started.id).expect("publish proven failure");

        assert_eq!(failed.phase, RuntimeJobPhase::Failed);
        assert!(!service.store.active_lock_path().exists());
    }

    #[test]
    fn dropping_system_runtime_process_reaps_the_owned_tree_within_one_budget() {
        let cache = TestCache::new();
        fs::create_dir_all(cache.path()).expect("create runtime cwd");
        let scenario = crate::infrastructure::platform::runtime_process_tree_test_scenario_for_test(
            &cache.path(),
        );
        let runner = SystemRuntimeJobRunner {
            program: scenario.program(),
            cwd: cache.path().to_path_buf(),
        };
        let request = RuntimeJobRequest::new(
            RuntimeJobOperation::Test,
            scenario.long_lived_args(),
            "workspace:test".to_string(),
            None,
        );
        let process = runner.spawn(&request).expect("spawn owned process tree");
        let descendant = scenario
            .wait_for_descendant(Duration::from_secs(5))
            .expect("observe drop-test descendant");
        assert!(
            descendant.is_alive().expect("probe live process tree"),
            "runtime descendant did not start"
        );

        drop(process);
        let started = Instant::now();
        while descendant.is_alive().expect("probe dropped process tree")
            && started.elapsed() < Duration::from_secs(1)
        {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(
            !descendant.is_alive().expect("probe reaped process tree"),
            "dropping runtime process left its owned tree alive"
        );
    }

    #[test]
    pub(crate) fn runtime_resource_tree_lease_contract() {
        let supervisor_scope = TestSupervisorScope::new();
        runtime_shared_work_joins_only_the_exact_active_resource_and_lease();
        runtime_shared_work_separates_physical_resources_and_distinct_v4_leases();
        stale_runtime_shared_work_authority_starts_no_producer_after_lock_replacement();
        replaced_jobs_directory_between_lifecycle_lock_and_admission_starts_no_producer();
        symlinked_jobs_directory_between_lifecycle_lock_and_admission_starts_no_producer();
        quarantine_release_never_removes_active_lock_from_replacement_jobs_root();
        quarantine_record_read_never_follows_replacement_after_retained_validation();
        quarantine_record_publish_never_writes_replacement_root();
        quarantine_publication_never_releases_after_same_root_job_directory_replacement();
        quarantine_post_rename_flush_failure_never_releases_active_lock();
        quarantine_post_publication_confirmation_rejects_same_root_job_directory_swap();
        quarantine_record_transition_completes_only_in_exact_retained_root();
        ownership_retained_transition_never_writes_replacement_job_directory();
        failed_activation_cleanup_never_writes_replacement_job_directory();
        retained_test_fake_supervisor_is_drained_before_authority_root_teardown();
        nested_retained_supervisor_is_not_hidden_from_outer_fixture_owner();
        nested_externally_terminal_uncontrolled_supervisor_is_owner_drained();
        unbound_nested_test_service_fails_before_process_admission();
        fixture_owner_close_blocks_late_nested_admission();
        unfinished_nested_supervisor_is_restored_with_same_owner_root_and_handle();
        uncertain_post_spawn_failure_retains_active_lock_for_initial_attempt();
        uncertain_post_spawn_failure_retains_active_lock_for_full_build_fallback();
        worker_supervises_initial_retained_ownership_until_proven_terminal();
        worker_supervises_fallback_retained_ownership_until_proven_terminal();
        worker_quarantines_poll_failure_until_later_terminal_proof();
        worker_quarantines_output_failure_until_later_eof_proof();
        stale_local_worker_retains_process_until_later_terminal_proof();
        stream_tail_read_failure_is_sticky_across_quarantine_probes();
        stream_tail_panic_is_sticky_across_quarantine_probes();
        worker_retains_initial_process_when_running_record_write_fails();
        worker_retains_fallback_process_when_running_record_write_fails();
        quarantine_thread_spawn_failure_retains_process_authority_and_active_lock();
        poisoned_process_registry_recovers_before_post_spawn_ownership_transfer();
        proven_childless_spawn_failure_releases_initial_active_lock();
        proven_childless_fallback_spawn_failure_releases_active_lock();
        two_nonterminating_output_readers_share_one_absolute_cleanup_deadline();
        system_process_drop_uses_no_second_window_after_cleanup_deadline_is_consumed();
        external_cancel_error_binds_system_drop_to_the_same_cleanup_deadline();
        waitid_authority_loss_makes_cancel_and_drop_send_zero_group_signals();
        reap_authority_loss_makes_cancel_and_drop_send_zero_group_signals();
        crate::infrastructure::platform::assert_runtime_generation_authority_for_test();
        system_runtime_job_keeps_resource_owned_after_leader_exit_until_descendant_dies();
        dropping_system_runtime_process_reaps_the_owned_tree_within_one_budget();
        assert_system_cancellation_reaps_process_tree();
        supervisor_scope.assert_drained();
    }

    #[test]
    pub(crate) fn terminal_snapshot_and_persistence_are_redacted_and_keep_log_artifacts() {
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
    pub(crate) fn production_secret_key_matrix_is_redacted_from_runtime_surfaces() {
        let keys = crate::infrastructure::redaction::production_secret_key_matrix();
        assert_eq!(keys, ["connection", "pwd", "password", "token", "secret"]);
        assert_eq!(
            production_runtime_secret_flags(),
            ["password", "pwd", "token", "secret", "connection", "c"]
        );
        assert_eq!(
            production_runtime_connection_markers(),
            ["file=", "srvr=", "ref=", "usr=", "pwd=", "dbsrvr=", "dbname="]
        );
        let stdout = keys
            .iter()
            .map(|key| format!("{key}=stdout-value-{key}"))
            .collect::<Vec<_>>()
            .join("\n");
        let stderr = keys
            .iter()
            .map(|key| format!("{key}=error-value-{key}"))
            .collect::<Vec<_>>()
            .join("\n");
        let argv = std::iter::once("runner".to_string())
            .chain(
                keys.iter()
                    .flat_map(|key| [format!("--{key}"), format!("argv-value-{key}")]),
            )
            .collect::<Vec<_>>();
        let target = keys
            .iter()
            .map(|key| format!("{key}=target-value-{key}"))
            .collect::<Vec<_>>()
            .join("&");
        let artifact = keys
            .iter()
            .map(|key| format!("{key}=artifact-value-{key}"))
            .collect::<Vec<_>>()
            .join("&");

        let cache = TestCache::new();
        let runner = Arc::new(FakeRunner::exits_after(1, 17, &stdout, &stderr));
        let service = RuntimeJobService::new(cache.path(), runner);
        let job = service
            .start(RuntimeJobRequest::new(
                RuntimeJobOperation::Test,
                argv,
                target,
                Some(artifact),
            ))
            .expect("start redaction matrix job");
        let terminal = service.poll(&job.id).expect("finish redaction matrix job");
        let logs = service.logs(&job.id).expect("read redaction matrix logs");
        let snapshot = serde_json::to_string(&terminal).expect("serialize terminal snapshot");
        let persistence = fs::read_to_string(
            service
                .store
                .record_path(&job.id)
                .expect("runtime record path"),
        )
        .expect("read runtime record");

        assert_eq!(terminal.redacted_argv.len(), keys.len() * 2 + 1);
        assert_eq!(terminal.redacted_argv[0], "runner");
        assert!(terminal
            .artifact_path
            .as_deref()
            .unwrap()
            .contains("<redacted>"));
        for key in &keys {
            assert!(
                terminal
                    .redacted_argv
                    .windows(2)
                    .any(|pair| { pair == [format!("--{key}"), "<redacted>".to_string()] }),
                "argv lost the redacted {key} field: {:?}",
                terminal.redacted_argv
            );
            assert!(logs.stdout.contains(&format!("{key}=<redacted>")));
            assert!(logs.stderr.contains(&format!("{key}=<redacted>")));
            assert!(persistence.contains(&format!("--{key}")));
        }

        for key in keys {
            for secret in [
                format!("stdout-value-{key}"),
                format!("error-value-{key}"),
                format!("argv-value-{key}"),
                format!("target-value-{key}"),
                format!("artifact-value-{key}"),
            ] {
                for (surface, rendered) in [
                    ("snapshot", snapshot.as_str()),
                    ("persistence", persistence.as_str()),
                    ("stdout", logs.stdout.as_str()),
                    ("stderr", logs.stderr.as_str()),
                ] {
                    assert!(!rendered.contains(&secret), "{surface} leaked {secret}");
                }
            }
        }

        for flag in production_runtime_secret_flags() {
            let secret = format!("flag-secret-{flag}");
            let raw = vec!["runner".to_string(), format!("--{flag}"), secret.clone()];
            let redacted = redact_argv(&raw);
            assert_eq!(redacted.len(), raw.len());
            assert_eq!(redacted[1], format!("--{flag}"));
            assert_eq!(redacted[2], "<redacted>");
            assert!(!redacted.join(" ").contains(&secret));
        }
        for marker in production_runtime_connection_markers() {
            let secret = format!("{marker}connection-secret");
            let redacted = redact_argv(&["runner".to_string(), "--c".to_string(), secret.clone()]);
            assert_eq!(redacted, ["runner", "--c", "<redacted>"]);
            assert!(!redacted.join(" ").contains(&secret));
        }
    }

    #[test]
    pub(crate) fn direct_status_rejects_corrupt_unknown_schema_and_non_uuid_without_touching_active_lock(
    ) {
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
    pub(crate) fn list_skips_a_corrupt_record_and_redacts_its_warning() {
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

        drop(service);
        {
            let registry = test_fixture_registry()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let state = registry
                .owners
                .get(&cache.owner)
                .expect("current fallback fixture owner remains registered before drain");
            assert_eq!(state.supervisors.len(), 1);
            assert_eq!(state.supervisors[0].service_root, cache.path());
        }
        cache.drain_supervisors();
        assert!(
            !test_fixture_registry()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .owners
                .contains_key(&cache.owner),
            "current fallback fixture owner remained after exact supervisor join"
        );
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
    fn runtime_job_lifecycle_and_log_bounds_are_complete() {
        detached_worker_owns_the_queued_record_until_terminal_state();
        let cache = TestCache::new();
        let runner = Arc::new(FakeRunner::success_after(50));
        let service = RuntimeJobService::new(cache.path(), runner);
        let started = service
            .start(fake_request(RuntimeJobOperation::Test))
            .expect("start runtime job");

        assert_eq!(service.status(&started.id).unwrap().id, started.id);
        let waited = service.wait(&started.id, Duration::ZERO).unwrap();
        assert!(waited.wait_timed_out);
        assert_eq!(
            service
                .list()
                .jobs
                .iter()
                .filter(|job| job.id == started.id)
                .count(),
            1
        );

        fs::write(
            service.store.stdout_path(&started.id).unwrap(),
            "stdout-абвгд",
        )
        .unwrap();
        fs::write(
            service.store.stderr_path(&started.id).unwrap(),
            "stderr-12345",
        )
        .unwrap();
        let logs = RuntimeJobService::logs_at(cache.path(), &started.id, 3).unwrap();
        assert_eq!(logs.stdout, "вгд");
        assert_eq!(logs.stderr, "345");

        let cancelled = service.cancel(&started.id).unwrap();
        assert_eq!(cancelled.phase, RuntimeJobPhase::Cancelled);
        assert!(cancelled.cancelled);
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
        let state_root = cache.service_root("state");
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
        let state_root = cache.service_root("state");
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
        // The recovered job was still queued, so its record proves no child was
        // ever spawned and the workspace is released with it. The worker below
        // must still refuse to resurrect that terminal state.
        assert!(!service.store.active_lock_path().exists());
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
        let state_root = cache.service_root("state");
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
        let state_root = cache.service_root("state");
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
    pub(crate) fn worker_handoff_never_persists_actual_argv_or_output_secrets() {
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
    pub(crate) fn persisted_command_redacts_a_launch_connection_string_completely() {
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
        owner: TestFixtureOwnerToken,
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
            let root = std::env::temp_dir().join(format!("unica-runtime-jobs-{}", Uuid::new_v4()));
            let owner = create_test_fixture_owner(root.clone());
            register_test_supervisor_owner(owner);
            Self { root, owner }
        }

        fn path(&self) -> PathBuf {
            self.root.clone()
        }

        fn service_root(&self, relative: impl AsRef<Path>) -> PathBuf {
            bind_test_service_root(self.owner, self.root.join(relative))
        }

        fn drain_supervisors(&self) {
            drain_test_fixture_owner(self.owner);
        }
    }

    impl Drop for TestCache {
        fn drop(&mut self) {
            drain_test_fixture_owner(self.owner);
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

    struct UncertainSpawnRunner {
        next_id: AtomicU32,
        attempt: AtomicU32,
        fail_on: u32,
        retain_on_failure: bool,
        first: Mutex<Option<FakeProcessState>>,
        processes: Arc<Mutex<HashMap<u32, Arc<Mutex<FakeProcessState>>>>>,
    }

    #[derive(Clone, Copy)]
    enum JobDirectorySwapOutcome {
        OwnershipRetained,
        ActivationCleanupFailure,
    }

    struct JobDirectorySwapRunner {
        jobs_root: PathBuf,
        expected_replacement: Arc<Mutex<Vec<u8>>>,
        outcome: JobDirectorySwapOutcome,
    }

    impl RuntimeJobRunner for JobDirectorySwapRunner {
        fn spawn(&self, _request: &RuntimeJobRequest) -> RuntimeJobSpawnResult {
            let job_id = fs::read_to_string(self.jobs_root.join("active.lock"))
                .expect("read active runtime job id");
            let canonical = self.jobs_root.join(&job_id);
            let displaced = self.jobs_root.join(format!("{job_id}-retained"));
            let bytes =
                fs::read(canonical.join("record.json")).expect("capture admitted job record");
            fs::rename(&canonical, displaced).expect("displace admitted job directory A");
            fs::create_dir(&canonical).expect("install replacement job directory B");
            fs::write(canonical.join("record.json"), &bytes)
                .expect("install byte-identical replacement record");
            *self.expected_replacement.lock().unwrap() = bytes;

            let process: Box<dyn RuntimeJobProcess> = Box::new(ObservationFailureProcess {
                state: Arc::new(ObservationFailureState {
                    fail_poll: AtomicBool::new(false),
                    fail_output: AtomicBool::new(false),
                    terminal: AtomicBool::new(false),
                    dropped: AtomicBool::new(false),
                }),
            });
            match self.outcome {
                JobDirectorySwapOutcome::OwnershipRetained => {
                    Err(RuntimeJobSpawnFailure::OwnershipRetained {
                        error: redacted_error("injected retained ownership after namespace swap"),
                        process,
                    })
                }
                JobDirectorySwapOutcome::ActivationCleanupFailure => Ok(process),
            }
        }

        fn attach(&self, _process_id: u32) -> JobResult<Box<dyn RuntimeJobProcess>> {
            Err(redacted_error("replacement runner cannot attach"))
        }
    }

    impl UncertainSpawnRunner {
        fn fail_on_attempt(fail_on: u32, first: Option<FakeProcessState>) -> Self {
            Self {
                next_id: AtomicU32::new(500),
                attempt: AtomicU32::new(0),
                fail_on,
                retain_on_failure: true,
                first: Mutex::new(first),
                processes: Arc::new(Mutex::new(HashMap::new())),
            }
        }

        fn proven_childless_on_attempt(fail_on: u32, first: Option<FakeProcessState>) -> Self {
            Self {
                next_id: AtomicU32::new(500),
                attempt: AtomicU32::new(0),
                fail_on,
                retain_on_failure: false,
                first: Mutex::new(first),
                processes: Arc::new(Mutex::new(HashMap::new())),
            }
        }
    }

    impl RuntimeJobRunner for UncertainSpawnRunner {
        fn spawn(&self, _request: &RuntimeJobRequest) -> RuntimeJobSpawnResult {
            let attempt = self.attempt.fetch_add(1, Ordering::SeqCst);
            if attempt == self.fail_on {
                if !self.retain_on_failure {
                    return Err(RuntimeJobSpawnFailure::ProvenChildless(redacted_error(
                        "post-spawn cleanup proved the process tree terminal",
                    )));
                }
                let id = self.next_id.fetch_add(1, Ordering::SeqCst);
                let state = Arc::new(Mutex::new(FakeProcessState {
                    polls_until_exit: u32::MAX,
                    result: FakeResult::Exit(143),
                    stdout: String::new(),
                    stderr: String::new(),
                    cancel_calls: 0,
                }));
                self.processes
                    .lock()
                    .map_err(|error| {
                        RuntimeJobSpawnFailure::ProvenChildless(redacted_error(&format!(
                            "lock uncertain processes: {error}"
                        )))
                    })?
                    .insert(id, Arc::clone(&state));
                return Err(RuntimeJobSpawnFailure::OwnershipRetained {
                    error: redacted_error("post-spawn process-tree ownership is uncertain"),
                    process: Box::new(FakeProcess { id, state }),
                });
            }
            (|| -> JobResult<Box<dyn RuntimeJobProcess>> {
                let id = self.next_id.fetch_add(1, Ordering::SeqCst);
                let state = Arc::new(Mutex::new(
                    self.first
                        .lock()
                        .map_err(|error| redacted_error(&format!("lock first process: {error}")))?
                        .take()
                        .ok_or_else(|| redacted_error("missing first process state"))?,
                ));
                self.processes
                    .lock()
                    .map_err(|error| redacted_error(&format!("lock uncertain processes: {error}")))?
                    .insert(id, Arc::clone(&state));
                Ok(Box::new(FakeProcess { id, state }))
            })()
            .map_err(RuntimeJobSpawnFailure::ProvenChildless)
        }

        fn attach(&self, process_id: u32) -> JobResult<Box<dyn RuntimeJobProcess>> {
            let state = self
                .processes
                .lock()
                .map_err(|error| redacted_error(&format!("lock uncertain processes: {error}")))?
                .get(&process_id)
                .cloned()
                .ok_or_else(|| redacted_error("uncertain process unavailable"))?;
            Ok(Box::new(FakeProcess {
                id: process_id,
                state,
            }))
        }
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
        fn spawn(&self, request: &RuntimeJobRequest) -> RuntimeJobSpawnResult {
            (|| -> JobResult<Box<dyn RuntimeJobProcess>> {
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
            })()
            .map_err(RuntimeJobSpawnFailure::ProvenChildless)
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
        fn spawn(&self, _request: &RuntimeJobRequest) -> RuntimeJobSpawnResult {
            (|| -> JobResult<Box<dyn RuntimeJobProcess>> {
                let id = self.next_id.fetch_add(1, Ordering::Relaxed);
                let state = Arc::new(Mutex::new(self.initial.clone()));
                self.processes
                    .lock()
                    .map_err(|error| redacted_error(&format!("lock fake runner: {error}")))?
                    .insert(id, state.clone());
                Ok(Box::new(FakeProcess { id, state }))
            })()
            .map_err(RuntimeJobSpawnFailure::ProvenChildless)
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
                output_incomplete: false,
                fallback_receipt: Some(state.stdout.clone()),
                fallback_receipt_truncated: false,
            })
        }

        fn prepare_controlled_test_supervision(&mut self) -> bool {
            self.state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .polls_until_exit = 0;
            true
        }
    }
}
