//! v8 launch binding. The process owner retains the actual child and streams.
use crate::request::RuntimeJobRequest;
use std::{path::PathBuf, sync::Arc};
use unica_process::runtime_job::{spawn_runtime_job, RuntimeJobProcess, RuntimeJobSpawnResult};
use unica_runner::attempt::{RuntimeAttemptScope, RuntimeJobRunner, RuntimeWorkspaceObservation};

pub struct SystemRuntimeJobRunner {
    pub program: PathBuf,
    pub cwd: PathBuf,
    pub workspace: Arc<dyn RuntimeWorkspaceObservation>,
}

impl RuntimeJobRunner for SystemRuntimeJobRunner {
    type Request = RuntimeJobRequest;
    fn scope(&self, request: &RuntimeJobRequest) -> RuntimeAttemptScope {
        request.bound_scope(self.cwd.clone())
    }
    fn reauthorize(&self, request: &RuntimeJobRequest) -> Result<(), String> {
        match &request.build_preflight {
            Some(preflight) => preflight.reauthorize_current_workspace(self.workspace.as_ref()),
            None => Ok(()),
        }
    }
    fn spawn(&self, request: &RuntimeJobRequest) -> RuntimeJobSpawnResult {
        spawn_runtime_job(
            &self.program,
            &self.cwd,
            &request.raw_argv,
            request.full_rebuild_fallback_argv.is_some(),
        )
    }
    fn attach(&self, _process_id: u32) -> Result<Box<dyn RuntimeJobProcess>, String> {
        Err(unica_redaction::redactor(
            "runtime job worker cannot attach to an unowned process",
        ))
    }
}

/// A fixture can bind an exact persisted lease without admitting a child.
#[cfg(any(test, feature = "test-support"))]
pub struct CoordinationOnlyRunner;
#[cfg(any(test, feature = "test-support"))]
impl RuntimeJobRunner for CoordinationOnlyRunner {
    type Request = RuntimeJobRequest;
    fn scope(&self, request: &RuntimeJobRequest) -> RuntimeAttemptScope {
        request.bound_scope(PathBuf::new())
    }
    fn reauthorize(&self, _request: &RuntimeJobRequest) -> Result<(), String> {
        Err("coordination-only runner cannot authorize launch".to_string())
    }
    fn spawn(&self, _request: &RuntimeJobRequest) -> RuntimeJobSpawnResult {
        Err(
            unica_process::runtime_job::RuntimeJobSpawnFailure::ProvenChildless(
                "coordination-only runner cannot spawn".to_string(),
            ),
        )
    }
    fn attach(&self, _process_id: u32) -> Result<Box<dyn RuntimeJobProcess>, String> {
        Err("coordination-only runner cannot attach".to_string())
    }
}
