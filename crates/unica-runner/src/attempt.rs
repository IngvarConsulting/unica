//! A prepared attempt is bound to one backend and one immutable admission scope.
use crate::operation::RuntimeJobOperation;
use std::{
    fmt::Debug,
    path::{Path, PathBuf},
};
use unica_process::runtime_job::{RuntimeJobOutput, RuntimeJobProcess, RuntimeJobSpawnResult};
use unica_source_contract::workspace::WorkspaceContext;

/// Read-only workspace observation; it conveys no source publication authority.
pub trait RuntimeWorkspaceObservation: Send + Sync {
    fn discover(&self, cwd: &Path) -> Result<WorkspaceContext, String>;
}

/// Equality evidence captured by a backend for one admission. The authorization
/// bytes are opaque to execution; only their exact equality is meaningful here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeAttemptScope {
    pub operation: RuntimeJobOperation,
    pub cwd: PathBuf,
    pub safe_target: String,
    pub artifact_path: Option<String>,
    pub authorization: Vec<u8>,
}

pub struct CompletedAttempt<'a> {
    pub process_exit_code: Option<i32>,
    pub status_success: bool,
    pub timed_out: bool,
    pub cancelled: bool,
    pub stdout_truncated: bool,
    pub stdout_had_invalid_utf8: bool,
    pub stdout: &'a str,
}

#[derive(Debug, Clone, Copy)]
pub enum RetryStartFailure {
    Authorization,
    Spawn,
}

pub trait RuntimeAttempt: Clone + Debug + Send + Sync + 'static {
    fn operation(&self) -> RuntimeJobOperation;
    fn safe_target(&self) -> &str;
    fn artifact_path(&self) -> Option<&str>;
    fn timeout_reason(&self) -> Option<&str>;
    fn reportable_argv(&self) -> Vec<String>;
    fn validate(&self) -> Result<(), String>;
    fn take_retry(&mut self) -> Option<Self>;
    fn classify_retry(&self, observed: CompletedAttempt<'_>) -> Result<(), Option<String>>;
    fn retry_start_warning(&self, stage: RetryStartFailure, error: &str) -> String;
    fn retry_warning(&self) -> &'static str;
    fn combine_outputs(initial: &RuntimeJobOutput, retry: &RuntimeJobOutput) -> RuntimeJobOutput;
}

/// The associated request prevents cross-backend payload substitution. Process
/// ownership always arrives from process; this port never defines a second child.
pub trait RuntimeJobRunner: Send + Sync + 'static {
    type Request: RuntimeAttempt;
    fn scope(&self, request: &Self::Request) -> RuntimeAttemptScope;
    fn reauthorize(&self, request: &Self::Request) -> Result<(), String>;
    fn spawn(&self, request: &Self::Request) -> RuntimeJobSpawnResult;
    fn attach(&self, process_id: u32) -> Result<Box<dyn RuntimeJobProcess>, String>;
}
