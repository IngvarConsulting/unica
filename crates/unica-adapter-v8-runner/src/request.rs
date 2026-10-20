use crate::fallback::{
    classify_partial_platform_failure, full_rebuild_argv, BuildAttempt, PARTIAL_FALLBACK_WARNING,
};
use crate::preflight::RuntimeBuildPreflight;
use unica_process::runtime_job::RuntimeJobOutput;
use unica_runner::{
    attempt::{CompletedAttempt, RuntimeAttempt},
    operation::RuntimeJobOperation,
};
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
    pub(crate) full_rebuild_fallback_argv: Option<Vec<String>>,
}

impl RuntimeJobRequest {
    pub fn new(
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

    pub fn take_full_rebuild_fallback(&mut self) -> Option<Self> {
        let raw_argv = self.full_rebuild_fallback_argv.take()?;
        let mut fallback = self.clone();
        fallback.raw_argv = raw_argv;
        fallback.full_rebuild_fallback_argv = None;
        Some(fallback)
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

    #[cfg(any(test, feature = "test-support"))]
    pub fn full_rebuild_fallback_argv(&self) -> Option<&[String]> {
        self.full_rebuild_fallback_argv.as_deref()
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
    fn take_retry(&mut self) -> Option<Self> {
        self.take_full_rebuild_fallback()
    }
    fn classify_retry(&self, observed: CompletedAttempt<'_>) -> Result<(), Option<String>> {
        classify_partial_platform_failure(&BuildAttempt {
            argv: &self.raw_argv,
            process_exit_code: observed.process_exit_code,
            status_success: observed.status_success,
            timed_out: observed.timed_out,
            cancelled: observed.cancelled,
            stdout_truncated: observed.stdout_truncated,
            stdout_had_invalid_utf8: observed.stdout_had_invalid_utf8,
            stdout: observed.stdout,
        })
        .map(|_| ())
        .map_err(|refusal| refusal.warning())
    }
    fn retry_start_warning(
        &self,
        stage: unica_runner::attempt::RetryStartFailure,
        error: &str,
    ) -> String {
        match stage {
            unica_runner::attempt::RetryStartFailure::Authorization => format!(
                "full rebuild fallback was not started because build identity changed: {error}"
            ),
            unica_runner::attempt::RetryStartFailure::Spawn => format!(
                "full rebuild fallback was not started because v8-runner failed to spawn: {error}"
            ),
        }
    }
    fn retry_warning(&self) -> &'static str {
        PARTIAL_FALLBACK_WARNING
    }
    fn combine_outputs(initial: &RuntimeJobOutput, retry: &RuntimeJobOutput) -> RuntimeJobOutput {
        combine_runtime_job_outputs(initial, retry)
    }
}

impl RuntimeJobRequest {
    pub fn bound_scope(
        &self,
        cwd: std::path::PathBuf,
    ) -> unica_runner::attempt::RuntimeAttemptScope {
        unica_runner::attempt::RuntimeAttemptScope {
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
    #[cfg(any(test, feature = "test-support"))]
    pub fn replace_fallback_for_test(&mut self, fallback: Option<Vec<String>>) {
        self.full_rebuild_fallback_argv = fallback;
    }
}
