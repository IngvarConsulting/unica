use serde::Deserialize;
use serde_json::Value;

pub(crate) const PARTIAL_FALLBACK_WARNING: &str =
    "v8-runner reported a completed partial load failure; Unica retried once with `--full-rebuild`";

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
) -> Option<PartialBuildFailure> {
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
        || attempt.stdout_truncated
        || attempt.stdout_had_invalid_utf8
        || attempt.process_exit_code != Some(4)
    {
        return None;
    }

    let envelope: FailureEnvelope = serde_json::from_str(attempt.stdout).ok()?;
    if envelope.ok
        || envelope.command != "build"
        || envelope.data.ok
        || envelope.error.code != "platform_failure"
        || envelope.error.kind != "platform"
        || envelope.error.message.is_empty()
        || envelope.duration_ms != envelope.data.duration_ms
        || !envelope.warnings.is_empty()
        || !envelope.steps.is_empty()
    {
        return None;
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
            _ => return None,
        }
    }
    let step = failed_partial?;
    let BuildMode::Partial { file_count } = step.mode else {
        return None;
    };
    let step_error = step.message.as_deref()?.strip_prefix("platform error: ")?;
    if step_error != envelope.error.message {
        return None;
    }
    let prefix = format!(
        "load failed for source-set '{}' with exit code ",
        step.source_set
    );
    let (exit_code, _) = envelope
        .error
        .message
        .strip_prefix(&prefix)?
        .split_once("; ")?;
    let inner_exit_code = exit_code.parse::<i32>().ok()?;
    if inner_exit_code <= 0 {
        return None;
    }
    let (_, partial_list_path) = envelope
        .error
        .message
        .rsplit_once("; partial load list path: ")?;
    if partial_list_path.trim().is_empty() || partial_list_path.contains("; ") {
        return None;
    }
    if requested_source_set(attempt.argv)
        .is_some_and(|requested| requested != step.source_set.as_str())
    {
        return None;
    }

    Some(PartialBuildFailure {
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

        assert!(classify_partial_platform_failure(&attempt(&argv, &stdout)).is_some());
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

        assert!(classify_partial_platform_failure(&attempt(&argv, &spawn)).is_none());
        assert!(classify_partial_platform_failure(&attempt(&argv, &update)).is_none());
        assert!(classify_partial_platform_failure(&attempt(&argv, &full)).is_none());
        assert!(classify_partial_platform_failure(&attempt(&argv, &runtime)).is_none());

        let valid = failure(
            json!({"partial": {"file_count": 1}}),
            "platform_failure",
            "load failed for source-set 'main' with exit code 1; platform log: sanitized; partial load list path: /tmp/partial.lst",
        );
        let mut interrupted = attempt(&argv, &valid);
        interrupted.cancelled = true;
        assert!(classify_partial_platform_failure(&interrupted).is_none());
        interrupted.cancelled = false;
        interrupted.timed_out = true;
        assert!(classify_partial_platform_failure(&interrupted).is_none());
        interrupted.timed_out = false;
        interrupted.stdout_truncated = true;
        assert!(classify_partial_platform_failure(&interrupted).is_none());

        interrupted.stdout_truncated = false;
        interrupted.process_exit_code = Some(137);
        assert!(
            classify_partial_platform_failure(&interrupted).is_none(),
            "a killed process must not authorize fallback from a stale receipt"
        );
        interrupted.process_exit_code = None;
        assert!(
            classify_partial_platform_failure(&interrupted).is_none(),
            "an unknown process exit must fail closed"
        );
    }

    #[test]
    fn malformed_or_mismatched_receipts_fail_closed() {
        let argv = argv();
        assert!(classify_partial_platform_failure(&attempt(&argv, "exit code 4")).is_none());

        let message = "load failed for source-set 'main' with exit code 1; partial load list path: /tmp/partial.lst";
        let valid = failure(
            json!({"partial": {"file_count": 1}}),
            "platform_failure",
            message,
        );
        let duplicated = format!("{valid}\n{valid}");
        assert!(classify_partial_platform_failure(&attempt(&argv, &duplicated)).is_none());

        let selected = vec![
            "--json-message".to_string(),
            "build".to_string(),
            "--source-set".to_string(),
            "other".to_string(),
        ];
        assert!(classify_partial_platform_failure(&attempt(&selected, &valid)).is_none());

        let no_json_flag = vec!["build".to_string()];
        assert!(classify_partial_platform_failure(&attempt(&no_json_flag, &valid)).is_none());

        let trailing_diagnostic = failure(
            json!({"partial": {"file_count": 1}}),
            "platform_failure",
            "load failed for source-set 'main' with exit code 1; platform log: sanitized; partial load list path: /tmp/partial.lst; failed to spawn cleanup",
        );
        assert!(classify_partial_platform_failure(&attempt(&argv, &trailing_diagnostic)).is_none());

        let unknown_partial_field =
            valid.replace("\"file_count\":1", "\"file_count\":1,\"unknown\":true");
        assert!(
            classify_partial_platform_failure(&attempt(&argv, &unknown_partial_field)).is_none(),
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
