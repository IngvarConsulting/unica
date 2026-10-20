//! Strict existing worker handoff codec, owned by the v8 adapter.
use crate::{
    backend::SystemRuntimeJobRunner, preflight::RuntimeBuildPreflight, request::RuntimeJobRequest,
};
use serde::{Deserialize, Serialize};
use std::{
    io::{self, Read, Write},
    path::PathBuf,
    sync::Arc,
};
use unica_execution::runtime_jobs::{
    cancel_queued_job, fail_queued_job, run_worker_request, RuntimeJobSnapshot,
    RuntimeLifecycleObserver, RuntimeWorkerRun,
};
use unica_runner::{attempt::RuntimeWorkspaceObservation, operation::RuntimeJobOperation};
use unica_work_control::cancellation::CancellationToken;
type JobResult<T> = Result<T, String>;
fn redacted_error(text: &str) -> String {
    unica_redaction::redactor(text)
}
fn io_error(context: &str, error: &io::Error) -> String {
    redacted_error(&format!("{context}: {error}"))
}
pub fn run_worker_from_reader(
    reader: impl Read,
    workspace: Arc<dyn RuntimeWorkspaceObservation>,
    observer: &dyn RuntimeLifecycleObserver,
) -> Result<(), String> {
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
        workspace,
    });
    let request = match handoff.runtime_request() {
        Ok(request) => request,
        Err(error) => {
            fail_queued_job(&handoff.cache_root, &handoff.job_id, &error)?;
            return Err(error);
        }
    };
    run_worker_request(
        RuntimeWorkerRun {
            cache_root: handoff.cache_root,
            job_id: handoff.job_id,
            cwd: handoff.cwd,
            request,
        },
        runner,
        observer,
    )
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

/// The hook runs between the request and commit frames, so a test can cancel
/// while exactly one frame is readable. Production passes a no-op.
fn write_worker_handoff_after_request(
    writer: &mut (impl Write + ?Sized),
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
    writer: &mut (impl Write + ?Sized),
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

pub fn start_detached_worker(
    cache_root: PathBuf,
    program: PathBuf,
    cwd: PathBuf,
    request: RuntimeJobRequest,
    cancellation: &CancellationToken,
) -> JobResult<RuntimeJobSnapshot> {
    let handoff_root = cache_root.clone();
    unica_execution::runtime_jobs::start_detached_worker(
        cache_root,
        request,
        cancellation,
        |writer, queued, request, cancellation| {
            let handoff =
                WorkerStartRequest::new(handoff_root, queued.id.clone(), program, cwd, request);
            write_worker_handoff_after_request(writer, &handoff, cancellation, || {})
        },
    )
}

mod codec {
    use super::*;
    #[derive(Debug, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub struct WorkerStartRequest {
        pub cache_root: PathBuf,
        pub job_id: String,
        pub program: PathBuf,
        pub cwd: PathBuf,
        pub operation: String,
        pub argv: Vec<String>,
        pub safe_target: String,
        pub artifact_path: Option<String>,
        pub timeout_reason: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub build_preflight: Option<RuntimeBuildPreflight>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub full_rebuild_fallback_argv: Option<Vec<String>>,
    }

    #[derive(Debug, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    pub struct WorkerStartCommit {
        pub cancelled: bool,
    }

    impl WorkerStartRequest {
        pub fn new(
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

        pub fn runtime_request(&self) -> JobResult<RuntimeJobRequest> {
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
}
use codec::*;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support {
    pub use super::codec::{WorkerStartCommit, WorkerStartRequest};
    use super::*;
    pub fn read_worker_frame<T: for<'de> Deserialize<'de>>(
        reader: &mut impl io::BufRead,
        label: &str,
    ) -> JobResult<T> {
        super::read_worker_frame(reader, label)
    }
    pub fn write_worker_frame(
        writer: &mut impl Write,
        value: &impl Serialize,
        label: &str,
    ) -> JobResult<()> {
        super::write_worker_frame(writer, value, label)
    }
    pub fn read_worker_commit(
        reader: &mut impl io::BufRead,
        handoff: &WorkerStartRequest,
    ) -> JobResult<WorkerStartCommit> {
        super::read_worker_commit(reader, handoff)
    }
    pub fn write_worker_handoff_after_request(
        writer: &mut impl Write,
        handoff: &WorkerStartRequest,
        cancellation: &CancellationToken,
        after_request: impl FnOnce(),
    ) -> JobResult<bool> {
        super::write_worker_handoff_after_request(writer, handoff, cancellation, after_request)
    }
    pub fn decoded_run(
        handoff: WorkerStartRequest,
    ) -> JobResult<RuntimeWorkerRun<RuntimeJobRequest>> {
        Ok(RuntimeWorkerRun {
            cache_root: handoff.cache_root.clone(),
            job_id: handoff.job_id.clone(),
            cwd: handoff.cwd.clone(),
            request: handoff.runtime_request()?,
        })
    }
}
