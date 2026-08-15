use serde::Deserialize;
use serde_json::Value;

pub(crate) const PARTIAL_FALLBACK_WARNING: &str =
    "v8-runner reported a completed partial load failure; Unica retried once with `--full-rebuild`";

/// A build that reached the pinned failure code but whose receipt refused the
/// classification must say so. Silence here is indistinguishable from a runtime
/// that never considered the retry, so a receipt that drifts away from the
/// pinned shape would look exactly like #404 being unfixed (ADR-0062).
const PARTIAL_FALLBACK_REJECTED: &str =
    "v8-runner exited with the pinned partial-load failure code, but its structured result did not \
     prove a completed partial load, so no full rebuild was retried";

/// Why an attempt did not authorize the one full retry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FallbackRejection {
    /// The attempt was never a candidate: an explicit full rebuild, a success,
    /// a cancellation, a timeout, or any exit code other than the pinned one.
    /// Ordinary results must stay quiet.
    NotACandidate,
    /// The attempt carried the pinned exit code, so the caller expected a
    /// retry. The reason names the first check that refused the receipt.
    Receipt(&'static str),
}

impl FallbackRejection {
    /// The public warning for a rejection, or `None` when the attempt was never
    /// a candidate and reporting it would be noise on every ordinary failure.
    pub(crate) fn warning(&self) -> Option<String> {
        match self {
            Self::NotACandidate => None,
            Self::Receipt(reason) => Some(format!("{PARTIAL_FALLBACK_REJECTED}: {reason}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BuildAttempt<'a> {
    pub(crate) argv: &'a [String],
    pub(crate) process_exit_code: Option<i32>,
    pub(crate) status_success: bool,
    pub(crate) timed_out: bool,
    pub(crate) cancelled: bool,
    pub(crate) stdout_truncated: bool,
    pub(crate) stdout_had_invalid_utf8: bool,
    pub(crate) stdout: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PartialBuildFailure {
    pub(crate) source_set: String,
    pub(crate) file_count: usize,
    pub(crate) inner_exit_code: i32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FailureEnvelope {
    ok: bool,
    command: String,
    duration_ms: u64,
    data: BuildData,
    warnings: Vec<String>,
    steps: Vec<Value>,
    error: RunnerError,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RunnerError {
    code: String,
    kind: String,
    message: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BuildData {
    ok: bool,
    steps: Vec<BuildStep>,
    duration_ms: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BuildStep {
    source_set: String,
    mode: BuildMode,
    ok: bool,
    message: Option<String>,
    duration_ms: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum BuildMode {
    EdtExport,
    Full,
    Partial { file_count: usize },
    Skipped,
}

pub(crate) fn full_rebuild_argv(argv: &[String]) -> Option<Vec<String>> {
    if argv.iter().any(|argument| argument == "--full-rebuild") {
        return None;
    }
    let build_index = build_subcommand_index(argv)?;
    let mut full = argv.to_vec();
    full.insert(build_index.saturating_add(1), "--full-rebuild".to_string());
    Some(full)
}

pub(crate) fn process_exit_code(status: &str) -> Option<i32> {
    let status = status.trim();
    status.parse::<i32>().ok().or_else(|| {
        ["exit status: ", "exit code: "]
            .iter()
            .find_map(|prefix| status.strip_prefix(prefix))
            .and_then(|code| code.parse::<i32>().ok())
    })
}

fn build_subcommand_index(argv: &[String]) -> Option<usize> {
    let mut index = 0;
    while index < argv.len() {
        match argv[index].as_str() {
            "--json-message" => index = index.saturating_add(1),
            "--config" | "--workdir" if index.saturating_add(1) < argv.len() => {
                index = index.saturating_add(2);
            }
            "build" => return Some(index),
            _ => return None,
        }
    }
    None
}

pub(crate) fn classify_partial_platform_failure(
    attempt: &BuildAttempt<'_>,
) -> Result<PartialBuildFailure, FallbackRejection> {
    use FallbackRejection::{NotACandidate, Receipt};

    if !attempt
        .argv
        .iter()
        .any(|argument| argument == "--json-message")
        || attempt
            .argv
            .iter()
            .any(|argument| argument == "--full-rebuild")
        || attempt.status_success
        || attempt.timed_out
        || attempt.cancelled
        || attempt.process_exit_code != Some(4)
    {
        return Err(NotACandidate);
    }
    // Past this point the attempt looked like the pinned failure, so every
    // refusal below is worth reporting instead of being swallowed.
    if attempt.stdout_truncated {
        return Err(Receipt("the captured output was truncated"));
    }
    if attempt.stdout_had_invalid_utf8 {
        return Err(Receipt("the captured output was not valid UTF-8"));
    }

    let envelope: FailureEnvelope = serde_json::from_str(attempt.stdout)
        .map_err(|_| Receipt("the output is not a closed v8-runner build failure envelope"))?;
    if envelope.ok || envelope.data.ok {
        return Err(Receipt("the envelope does not report an overall failure"));
    }
    if envelope.command != "build" {
        return Err(Receipt("the envelope does not report the `build` command"));
    }
    if envelope.error.code != "platform_failure" || envelope.error.kind != "platform" {
        return Err(Receipt(
            "the error is not a `platform_failure` of kind `platform`",
        ));
    }
    if envelope.error.message.is_empty() {
        return Err(Receipt("the error carries no message"));
    }
    if envelope.duration_ms != envelope.data.duration_ms {
        return Err(Receipt("the envelope and build durations disagree"));
    }
    if !envelope.warnings.is_empty() || !envelope.steps.is_empty() {
        return Err(Receipt(
            "the envelope carries top-level warnings or steps that the pinned failure never has",
        ));
    }

    let mut failed_partial = None;
    for step in &envelope.data.steps {
        match (&step.mode, step.ok, failed_partial.is_some()) {
            (_, true, false) => {}
            (BuildMode::Partial { file_count }, false, false) if *file_count > 0 => {
                failed_partial = Some(step);
            }
            (BuildMode::Skipped, false, true)
                if step.duration_ms == 0
                    && step.message.as_deref() == Some("aborted after previous failure") => {}
            _ => {
                return Err(Receipt(
                    "the failed step is not a single partial load followed by canonical skips",
                ))
            }
        }
    }
    let Some(step) = failed_partial else {
        return Err(Receipt("no failed partial load step is present"));
    };
    let BuildMode::Partial { file_count } = step.mode else {
        return Err(Receipt("no failed partial load step is present"));
    };
    let step_error = step
        .message
        .as_deref()
        .and_then(|message| message.strip_prefix("platform error: "))
        .ok_or(Receipt("the failed step carries no platform error message"))?;
    if step_error != envelope.error.message {
        return Err(Receipt("the step and envelope error messages disagree"));
    }
    let prefix = format!(
        "load failed for source-set '{}' with exit code ",
        step.source_set
    );
    let inner_exit_code = envelope
        .error
        .message
        .strip_prefix(&prefix)
        .and_then(|rest| rest.split_once("; "))
        .and_then(|(exit_code, _)| exit_code.parse::<i32>().ok())
        .filter(|inner_exit_code| *inner_exit_code > 0)
        .ok_or(Receipt(
            "the error does not report a positive platform exit code for the failed source-set",
        ))?;
    let (_, partial_list_path) = envelope
        .error
        .message
        .rsplit_once("; partial load list path: ")
        .ok_or(Receipt(
            "the error does not report a partial load list path",
        ))?;
    if partial_list_path.trim().is_empty() || partial_list_path.contains("; ") {
        return Err(Receipt("the reported partial load list path is not final"));
    }
    if requested_source_set(attempt.argv)
        .is_some_and(|requested| requested != step.source_set.as_str())
    {
        return Err(Receipt(
            "the failed source-set is not the one the build requested",
        ));
    }

    Ok(PartialBuildFailure {
        source_set: step.source_set.clone(),
        file_count,
        inner_exit_code,
    })
}

fn requested_source_set(argv: &[String]) -> Option<&str> {
    argv.windows(2)
        .find_map(|window| (window[0] == "--source-set").then_some(window[1].as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn argv() -> Vec<String> {
        vec!["--json-message".to_string(), "build".to_string()]
    }

    fn failure(mode: Value, code: &str, message: &str) -> String {
        serde_json::to_string(&json!({
            "ok": false,
            "command": "build",
            "duration_ms": 12,
            "data": {
                "ok": false,
                "steps": [{
                    "source_set": "main",
                    "mode": mode,
                    "ok": false,
                    "message": format!("platform error: {message}"),
                    "duration_ms": 0
                }],
                "duration_ms": 12
            },
            "warnings": [],
            "steps": [],
            "error": {
                "code": code,
                "kind": if code == "platform_failure" { "platform" } else { "runtime" },
                "message": message
            }
        }))
        .unwrap()
    }

    fn attempt<'a>(argv: &'a [String], stdout: &'a str) -> BuildAttempt<'a> {
        BuildAttempt {
            argv,
            process_exit_code: Some(4),
            status_success: false,
            timed_out: false,
            cancelled: false,
            stdout_truncated: false,
            stdout_had_invalid_utf8: false,
            stdout,
        }
    }

    #[test]
    fn completed_designer_partial_load_failure_is_retryable() {
        let argv = argv();
        let message = "load failed for source-set 'main' with exit code 1; platform log: sanitized; platform log path: /tmp/out.log; partial load list path: /tmp/partial.lst";
        let stdout = failure(
            json!({"partial": {"file_count": 4}}),
            "platform_failure",
            message,
        );

        let evidence = classify_partial_platform_failure(&attempt(&argv, &stdout))
            .expect("completed partial load failure");

        assert_eq!(evidence.source_set, "main");
        assert_eq!(evidence.file_count, 4);
        assert_eq!(evidence.inner_exit_code, 1);
    }

    #[test]
    fn completed_partial_failure_accepts_canonical_neighbor_steps() {
        let argv = argv();
        let message = "load failed for source-set 'main' with exit code 1; platform log: sanitized; partial load list path: /tmp/partial.lst";
        let stdout = serde_json::to_string(&json!({
            "ok": false,
            "command": "build",
            "duration_ms": 12,
            "data": {
                "ok": false,
                "steps": [
                    {
                        "source_set": "base",
                        "mode": "full",
                        "ok": true,
                        "message": null,
                        "duration_ms": 7
                    },
                    {
                        "source_set": "main",
                        "mode": {"partial": {"file_count": 4}},
                        "ok": false,
                        "message": format!("platform error: {message}"),
                        "duration_ms": 0
                    },
                    {
                        "source_set": "extension",
                        "mode": "skipped",
                        "ok": false,
                        "message": "aborted after previous failure",
                        "duration_ms": 0
                    }
                ],
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
        .unwrap();

        assert!(classify_partial_platform_failure(&attempt(&argv, &stdout)).is_ok());
    }

    #[test]
    fn a_refused_receipt_reports_the_check_that_rejected_it() {
        let argv = argv();
        let message = "load failed for source-set 'main' with exit code 1; platform log: sanitized; partial load list path: /tmp/partial.lst";
        let warned = serde_json::from_str::<Value>(&failure(
            json!({"partial": {"file_count": 4}}),
            "platform_failure",
            message,
        ))
        .map(|mut envelope| {
            envelope["warnings"] = json!(["platform version was pinned"]);
            envelope.to_string()
        })
        .unwrap();

        let rejection = classify_partial_platform_failure(&attempt(&argv, &warned))
            .expect_err("a receipt carrying envelope warnings is not the pinned failure");

        let reported = rejection
            .warning()
            .expect("a receipt that reached the pinned exit code must be reported");
        assert!(
            reported.contains("no full rebuild was retried"),
            "{reported}"
        );
        assert!(
            reported.contains("top-level warnings or steps"),
            "the report must name the check that refused it: {reported}"
        );
    }

    #[test]
    fn an_ordinary_failure_is_never_reported_as_a_refused_receipt() {
        let argv = argv();
        let mut ordinary = attempt(&argv, "");
        ordinary.process_exit_code = Some(1);

        let rejection = classify_partial_platform_failure(&ordinary)
            .expect_err("an unrelated exit code is not a candidate");

        assert_eq!(rejection, FallbackRejection::NotACandidate);
        assert_eq!(
            rejection.warning(),
            None,
            "every ordinary build failure would otherwise carry a fallback warning"
        );
    }

    #[test]
    fn unrelated_platform_failures_and_interruption_are_not_retryable() {
        let argv = argv();
        let partial_mode = json!({"partial": {"file_count": 1}});
        let spawn = failure(
            partial_mode.clone(),
            "platform_failure",
            "partial load list path: /tmp/partial.lst; failed to spawn process 1cv8",
        );
        let update = failure(
            partial_mode.clone(),
            "platform_failure",
            "update_db_cfg failed for source-set 'main' with exit code 1; platform log: sanitized; partial load list path: /tmp/partial.lst",
        );
        let full = failure(json!("full"), "platform_failure", "full failed");
        let runtime = failure(partial_mode, "runtime_failure", "runtime failed");

        assert!(classify_partial_platform_failure(&attempt(&argv, &spawn)).is_err());
        assert!(classify_partial_platform_failure(&attempt(&argv, &update)).is_err());
        assert!(classify_partial_platform_failure(&attempt(&argv, &full)).is_err());
        assert!(classify_partial_platform_failure(&attempt(&argv, &runtime)).is_err());

        let valid = failure(
            json!({"partial": {"file_count": 1}}),
            "platform_failure",
            "load failed for source-set 'main' with exit code 1; platform log: sanitized; partial load list path: /tmp/partial.lst",
        );
        let mut interrupted = attempt(&argv, &valid);
        interrupted.cancelled = true;
        assert!(classify_partial_platform_failure(&interrupted).is_err());
        interrupted.cancelled = false;
        interrupted.timed_out = true;
        assert!(classify_partial_platform_failure(&interrupted).is_err());
        interrupted.timed_out = false;
        interrupted.stdout_truncated = true;
        assert!(classify_partial_platform_failure(&interrupted).is_err());

        interrupted.stdout_truncated = false;
        interrupted.process_exit_code = Some(137);
        assert!(
            classify_partial_platform_failure(&interrupted).is_err(),
            "a killed process must not authorize fallback from a stale receipt"
        );
        interrupted.process_exit_code = None;
        assert!(
            classify_partial_platform_failure(&interrupted).is_err(),
            "an unknown process exit must fail closed"
        );
    }

    #[test]
    fn malformed_or_mismatched_receipts_fail_closed() {
        let argv = argv();
        assert!(classify_partial_platform_failure(&attempt(&argv, "exit code 4")).is_err());

        let message = "load failed for source-set 'main' with exit code 1; partial load list path: /tmp/partial.lst";
        let valid = failure(
            json!({"partial": {"file_count": 1}}),
            "platform_failure",
            message,
        );
        let duplicated = format!("{valid}\n{valid}");
        assert!(classify_partial_platform_failure(&attempt(&argv, &duplicated)).is_err());

        let selected = vec![
            "--json-message".to_string(),
            "build".to_string(),
            "--source-set".to_string(),
            "other".to_string(),
        ];
        assert!(classify_partial_platform_failure(&attempt(&selected, &valid)).is_err());

        let no_json_flag = vec!["build".to_string()];
        assert!(classify_partial_platform_failure(&attempt(&no_json_flag, &valid)).is_err());

        let trailing_diagnostic = failure(
            json!({"partial": {"file_count": 1}}),
            "platform_failure",
            "load failed for source-set 'main' with exit code 1; platform log: sanitized; partial load list path: /tmp/partial.lst; failed to spawn cleanup",
        );
        assert!(classify_partial_platform_failure(&attempt(&argv, &trailing_diagnostic)).is_err());

        let unknown_partial_field =
            valid.replace("\"file_count\":1", "\"file_count\":1,\"unknown\":true");
        assert!(
            classify_partial_platform_failure(&attempt(&argv, &unknown_partial_field)).is_err(),
            "the closed receipt schema must reject unknown partial-mode fields"
        );
    }

    #[test]
    fn full_rebuild_is_added_once_after_the_build_subcommand() {
        let partial = vec![
            "--json-message".to_string(),
            "--config".to_string(),
            "/workspace/v8project.yaml".to_string(),
            "build".to_string(),
            "--source-set".to_string(),
            "main".to_string(),
        ];

        let full = full_rebuild_argv(&partial).expect("fallback argv");

        assert_eq!(full[4], "--full-rebuild");
        assert!(full_rebuild_argv(&full).is_none());

        let colliding_global_values = vec![
            "--json-message".to_string(),
            "--config".to_string(),
            "build".to_string(),
            "--workdir".to_string(),
            "build".to_string(),
            "build".to_string(),
            "--source-set".to_string(),
            "build".to_string(),
        ];
        let full = full_rebuild_argv(&colliding_global_values).expect("fallback argv");
        assert_eq!(full[5], "build");
        assert_eq!(full[6], "--full-rebuild");

        assert_eq!(process_exit_code("exit status: 4"), Some(4));
        assert_eq!(process_exit_code("exit code: 4"), Some(4));
        assert_eq!(process_exit_code("4"), Some(4));
        assert_eq!(process_exit_code("signal: 9 (SIGKILL)"), None);
    }
}
