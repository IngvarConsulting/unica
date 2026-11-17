use crate::preflight::RuntimeBuildPreflight;
use unica_execution::runner::{
    attempt::{CompletedAttempt, RuntimeAttempt},
    operation::RuntimeJobOperation,
};
use unica_process::runtime_job::RuntimeJobOutput;
type JobResult<T> = Result<T, String>;
const RUNTIME_SECRET_VALUE_FLAGS: &[&str] =
    &["password", "pwd", "token", "secret", "connection", "c"];
const RUNTIME_CONNECTION_MARKERS: &[&str] = &[
    "file=", "srvr=", "ref=", "usr=", "pwd=", "dbsrvr=", "dbname=",
];
fn redact_text(text: &str) -> String {
    unica_redaction::redactor(text)
}
fn redacted_error(text: &str) -> String {
    redact_text(text)
}

/// A request may carry raw arguments only in memory.  They are never persisted verbatim.
#[derive(Debug, Clone)]
pub struct RuntimeJobRequest {
    pub(crate) operation: RuntimeJobOperation,
    pub(crate) raw_argv: Vec<String>,
    pub(crate) safe_target: String,
    pub(crate) artifact_path: Option<String>,
    pub(crate) timeout_reason: Option<String>,
    pub(crate) build_preflight: Option<RuntimeBuildPreflight>,
}

impl RuntimeJobRequest {
    pub fn new(
        operation: RuntimeJobOperation,
        raw_argv: Vec<String>,
        safe_target: impl Into<String>,
        artifact_path: Option<String>,
    ) -> Self {
        Self {
            operation,
            raw_argv,
            safe_target: safe_target.into(),
            artifact_path,
            timeout_reason: None,
            build_preflight: None,
        }
    }

    pub fn with_build_preflight(mut self, build_preflight: Option<RuntimeBuildPreflight>) -> Self {
        self.build_preflight = build_preflight;
        self
    }

    pub fn validate_build_preflight(&self) -> JobResult<()> {
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
        Ok(())
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn without_build_preflight_for_test(mut self) -> Self {
        self.build_preflight = None;
        self
    }

    #[allow(dead_code)]
    pub fn with_timeout_reason(mut self, timeout_reason: impl Into<String>) -> Self {
        self.timeout_reason = Some(timeout_reason.into());
        self
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn raw_argv(&self) -> &[String] {
        &self.raw_argv
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn build_preflight(&self) -> Option<&RuntimeBuildPreflight> {
        self.build_preflight.as_ref()
    }
}

pub fn redact_argv(argv: &[String]) -> Vec<String> {
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

#[cfg(any(test, feature = "test-support"))]
pub fn production_runtime_secret_flags() -> &'static [&'static str] {
    RUNTIME_SECRET_VALUE_FLAGS
}

#[cfg(any(test, feature = "test-support"))]
pub fn production_runtime_connection_markers() -> &'static [&'static str] {
    RUNTIME_CONNECTION_MARKERS
}

impl RuntimeAttempt for RuntimeJobRequest {
    fn operation(&self) -> RuntimeJobOperation {
        self.operation
    }
    fn safe_target(&self) -> &str {
        &self.safe_target
    }
    fn artifact_path(&self) -> Option<&str> {
        self.artifact_path.as_deref()
    }
    fn timeout_reason(&self) -> Option<&str> {
        self.timeout_reason.as_deref()
    }
    fn reportable_argv(&self) -> Vec<String> {
        redact_argv(&self.raw_argv)
    }
    fn validate(&self) -> JobResult<()> {
        self.validate_build_preflight()
    }
    // A build runs once: v8-runner chooses the import mode itself, and Unica
    // never repeats a failed build with `--full-rebuild` on its own.
    fn take_retry(&mut self) -> Option<Self> {
        None
    }
    fn classify_retry(&self, _observed: CompletedAttempt<'_>) -> Result<(), Option<String>> {
        Err(None)
    }
    fn retry_start_warning(
        &self,
        _stage: unica_execution::runner::attempt::RetryStartFailure,
        error: &str,
    ) -> String {
        redacted_error(error)
    }
    fn retry_warning(&self) -> &'static str {
        ""
    }
    fn combine_outputs(_initial: &RuntimeJobOutput, retry: &RuntimeJobOutput) -> RuntimeJobOutput {
        retry.clone()
    }
}

impl RuntimeJobRequest {
    pub fn bound_scope(
        &self,
        cwd: std::path::PathBuf,
    ) -> unica_execution::runner::attempt::RuntimeAttemptScope {
        unica_execution::runner::attempt::RuntimeAttemptScope {
            operation: self.operation,
            cwd,
            safe_target: self.safe_target.clone(),
            artifact_path: self.artifact_path.clone(),
            authorization: self
                .build_preflight
                .as_ref()
                .map(RuntimeBuildPreflight::scope_evidence)
                .unwrap_or_default(),
        }
    }
}

impl RuntimeJobRequest {
    #[cfg(any(test, feature = "test-support"))]
    pub fn raw_argv_mut_for_test(&mut self) -> &mut Vec<String> {
        &mut self.raw_argv
    }
}
