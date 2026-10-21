#[cfg(test)]
use unica_adapter_bsl_analyzer::adapter::{
    AnalyzerRunFailureKind, BslAnalyzerMcpAdapter, BslAnalyzerOutcome, BslMcpCommand, BslMcpOutput,
    BslMcpRunner,
};

use crate::application::RuntimeJobAction;
use crate::infrastructure::bundled_tools::resolve_bundled_tool;
use crate::infrastructure::plugin_runtime::find_plugin_root;
#[cfg(test)]
use unica_adapter_bsl_analyzer::jsonl::MAX_DIAGNOSTICS_JSONL_LINE_BYTES;
use unica_adapter_v8_runner::adapter::runtime_args;
#[cfg(test)]
use unica_adapter_v8_runner::adapter::{validate_runtime_mapper_payload, RuntimeAdapter};
use unica_adapter_v8_runner::preflight::RuntimeBuildPreflight;
use unica_adapter_v8_runner::request::RuntimeJobRequest;
use unica_artifact_delivery::engine::BundledTool;
use unica_execution::runtime_jobs;
use unica_execution_contract::adapter_outcome::cancelled_process_outcome;
use unica_execution_contract::adapter_outcome::merge_warnings;
use unica_execution_contract::adapter_outcome::AdapterOutcome;
use unica_process::arguments::process_timeout_error;
use unica_process::arguments::{kebab_case, reported_cli_value};
use unica_process::runner::{system_process_runner, ProcessCommand, ProcessRunner};
#[cfg(test)]
use unica_process::runner::{ProcessOutput, ProcessStreamFailure, ProcessStreamOutput};
use unica_redaction::redactor;
use unica_runner::operation::RuntimeJobOperation;
use unica_source_contract::workspace::WorkspaceContext;
#[cfg(test)]
use unica_workspace::paths::path_lock_identity;
type RuntimeJobService = runtime_jobs::RuntimeJobService<RuntimeJobRequest>;
#[cfg(test)]
use crate::infrastructure::source_roots::resolve_source_root;
#[cfg(test)]
use crate::infrastructure::workspace::discover_workspace;
use serde_json::{json, Map, Value};
use std::path::PathBuf;
use std::time::Duration;
#[cfg(test)]
use unica_process::StreamControl;
use unica_work_control::cancellation::{CancellationToken, CANCELLED_PREFIX};
#[cfg(test)]
use unica_workspace::paths::normalize_path_identity;

#[cfg(test)]
use unica_process::ProcessLineReply;

const DEFAULT_PROCESS_TIMEOUT: Duration = Duration::from_secs(120);

pub struct CliAdapter<'a> {
    tool_name: &'static str,
    default_command: &'static [&'static str],
    label: &'static str,
    runner: &'a dyn ProcessRunner,
    process_timeout: Duration,
}

/// Что показывает предпросмотр.
///
/// Если движка на диске нет, план не выдаёт себя за исполнимый: `ok` ложно, а
/// отказ назван кодом и следующим шагом. Команда при этом остаётся в ответе —
/// она объясняет, что было бы запущено, но уже не читается как готовая.
pub(crate) fn dry_run_outcome(
    tool_name: &str,
    label: &str,
    mutating: bool,
    bundled_tool: BundledTool,
    command: Vec<String>,
) -> AdapterOutcome {
    let changes = if mutating {
        vec!["no files changed because dryRun is true".to_string()]
    } else {
        Vec::new()
    };
    match bundled_tool.missing {
        Some(missing) => AdapterOutcome {
            ok: false,
            summary: format!(
                "dry run: {tool_name} cannot call internal {label} adapter — {} is not on this machine",
                missing.tool
            ),
            changes,
            warnings: bundled_tool.warnings,
            errors: vec![missing.message()],
            artifacts: Vec::new(),
            stdout: None,
            stderr: None,
            command: Some(command),
        },
        None => AdapterOutcome {
            ok: true,
            summary: format!("dry run: {tool_name} would call internal {label} adapter"),
            changes,
            warnings: bundled_tool.warnings,
            errors: Vec::new(),
            artifacts: Vec::new(),
            stdout: None,
            stderr: None,
            command: Some(command),
        },
    }
}

pub struct RuntimeJobAdapterOutcome {
    pub outcome: AdapterOutcome,
    pub job: Option<Value>,
}

pub struct RuntimeJobAdapter;

impl<'a> CliAdapter<'a> {
    pub fn new(
        tool_name: &'static str,
        default_command: &'static [&'static str],
        label: &'static str,
    ) -> Self {
        Self {
            tool_name,
            default_command,
            label,
            runner: system_process_runner(),
            process_timeout: DEFAULT_PROCESS_TIMEOUT,
        }
    }

    #[cfg(test)]
    fn with_runner(
        tool_name: &'static str,
        default_command: &'static [&'static str],
        label: &'static str,
        runner: &'a dyn ProcessRunner,
    ) -> Self {
        Self {
            tool_name,
            default_command,
            label,
            runner,
            process_timeout: DEFAULT_PROCESS_TIMEOUT,
        }
    }

    #[allow(dead_code)]
    pub fn invoke(
        &self,
        tool_name: &str,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
        dry_run: bool,
        mutating: bool,
    ) -> Result<AdapterOutcome, String> {
        self.invoke_cancellable(
            tool_name,
            args,
            context,
            dry_run,
            mutating,
            &CancellationToken::new(),
        )
    }

    pub fn invoke_cancellable(
        &self,
        tool_name: &str,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
        dry_run: bool,
        mutating: bool,
        cancellation: &CancellationToken,
    ) -> Result<AdapterOutcome, String> {
        if cancellation.is_cancelled() {
            return Ok(AdapterOutcome::cancelled(format!(
                "{tool_name} cancelled before adapter work"
            )));
        }
        let plugin_root = find_plugin_root(&context.cwd).ok_or_else(|| {
            "could not locate Unica plugin root for internal adapter lookup".to_string()
        })?;
        let reported_args = cli_args(args, true)?;
        let execution_args = cli_args(args, false)?;
        let bundled_tool = resolve_bundled_tool(&plugin_root, self.tool_name, !dry_run)?;
        let mut command = vec![bundled_tool.program.display().to_string()];
        command.extend(self.default_command.iter().map(|part| (*part).to_string()));
        command.extend(reported_args);

        if dry_run {
            return Ok(dry_run_outcome(
                tool_name,
                self.label,
                mutating,
                bundled_tool,
                command,
            ));
        }

        let mut process_args = self
            .default_command
            .iter()
            .map(|part| (*part).to_string())
            .collect::<Vec<_>>();
        process_args.extend(execution_args);
        let process_timeout = Some(self.process_timeout);
        let output = self.runner.run(&ProcessCommand {
            program: bundled_tool.program.clone(),
            args: process_args,
            cwd: context.cwd.clone(),
            env: Vec::new(),
            env_remove: Vec::new(),
            capture_limits: None,
            timeout: process_timeout,
            cancellation: cancellation.clone(),
        })?;
        if output.cancelled {
            return Ok(cancelled_process_outcome(
                tool_name,
                output.stdout,
                output.stderr,
                Some(command),
            ));
        }
        let ok = output.status_success;
        Ok(AdapterOutcome {
            ok,
            summary: if ok {
                format!(
                    "{tool_name} completed through internal {} adapter",
                    self.label
                )
            } else {
                format!("{tool_name} failed through internal {} adapter", self.label)
            },
            changes: if mutating {
                vec![format!("internal {} adapter executed", self.label)]
            } else {
                Vec::new()
            },
            warnings: if ok {
                Vec::new()
            } else if output.timed_out {
                vec![format!("internal {} adapter timed out", self.label)]
            } else {
                vec![format!(
                    "internal {} adapter exited with status {}",
                    self.label, output.status
                )]
            },
            errors: if ok {
                Vec::new()
            } else if output.stderr.trim().is_empty() && output.timed_out {
                vec![process_timeout_error(self.label, process_timeout)]
            } else {
                vec![output.stderr.trim().to_string()]
            },
            artifacts: Vec::new(),
            stdout: Some(output.stdout),
            stderr: Some(output.stderr),
            command: Some(command),
        })
    }
}

impl RuntimeJobAdapter {
    #[cfg(test)]
    pub fn invoke(
        action: RuntimeJobAction,
        tool_name: &str,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
        dry_run: bool,
    ) -> Result<RuntimeJobAdapterOutcome, String> {
        Self::invoke_cancellable(
            action,
            tool_name,
            args,
            context,
            dry_run,
            &CancellationToken::new(),
        )
    }

    pub(crate) fn invoke_cancellable(
        action: RuntimeJobAction,
        tool_name: &str,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
        dry_run: bool,
        cancellation: &CancellationToken,
    ) -> Result<RuntimeJobAdapterOutcome, String> {
        match action {
            RuntimeJobAction::Start => Self::start(tool_name, args, context, dry_run, cancellation),
            RuntimeJobAction::Status => Self::status(tool_name, args, context),
            RuntimeJobAction::Wait => Self::wait(tool_name, args, context),
            RuntimeJobAction::Logs => Self::logs(tool_name, args, context),
            RuntimeJobAction::Cancel => Self::cancel(tool_name, args, context, dry_run),
            RuntimeJobAction::List => Self::list(tool_name, context),
        }
    }

    fn start(
        tool_name: &str,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
        dry_run: bool,
        cancellation: &CancellationToken,
    ) -> Result<RuntimeJobAdapterOutcome, String> {
        for argument in ["waitForExit", "waitTimeoutMs", "stderrOutput"] {
            if args.contains_key(argument) {
                return Err(format!(
                    "{tool_name} does not support bounded external EPF argument `{argument}`; \
                     use `unica.runtime.execute`"
                ));
            }
        }

        let plugin_root = find_plugin_root(&context.cwd).ok_or_else(|| {
            "could not locate Unica plugin root for internal adapter lookup".to_string()
        })?;
        let invocation = unica_adapter_v8_runner::adapter::plan_runtime_invocation(
            args,
            context,
            super::runtime_composition::workspace_observer(),
        )?;
        if cancellation.is_cancelled() {
            return Ok(RuntimeJobAdapterOutcome {
                outcome: AdapterOutcome::cancelled(format!(
                    "{tool_name} cancelled during runtime build preflight"
                )),
                job: None,
            });
        }
        let reported_args = runtime_args(&invocation.args, true)?;
        let execution_args = runtime_args(&invocation.args, false)?;
        let build_preflight = invocation.build_preflight.clone();
        let bundled_tool = resolve_bundled_tool(&plugin_root, "v8-runner", !dry_run)?;
        let mut command = vec![bundled_tool.program.display().to_string()];
        command.extend(reported_args);

        if dry_run {
            let mut outcome = dry_run_outcome(
                tool_name,
                "v8-runner durable runtime job",
                true,
                bundled_tool,
                command,
            );
            outcome.changes = vec!["no runtime job started because dryRun is true".to_string()];
            outcome.warnings = merge_warnings(invocation.warnings, outcome.warnings);
            return Ok(RuntimeJobAdapterOutcome { outcome, job: None });
        }

        let warnings = merge_warnings(invocation.warnings, bundled_tool.warnings);

        if cancellation.is_cancelled() {
            return Ok(RuntimeJobAdapterOutcome {
                outcome: AdapterOutcome::cancelled(format!(
                    "{tool_name} cancelled before durable runtime job start"
                )),
                job: None,
            });
        }

        let request =
            runtime_job_start_request(tool_name, args, context, execution_args, build_preflight)?;
        match unica_adapter_v8_runner::worker::start_detached_worker(
            context.cache_root.clone(),
            bundled_tool.program,
            context.cwd.clone(),
            request,
            cancellation,
        ) {
            Ok(snapshot) => Ok(RuntimeJobAdapterOutcome {
                outcome: AdapterOutcome {
                    ok: true,
                    summary: format!("{tool_name} queued durable runtime job {}", snapshot.id),
                    changes: Vec::new(),
                    warnings,
                    errors: Vec::new(),
                    artifacts: Vec::new(),
                    stdout: None,
                    stderr: None,
                    command: Some(command),
                },
                job: Some(runtime_job_snapshot_value(&snapshot)),
            }),
            Err(error) => {
                if cancellation.is_cancelled() || error.starts_with(CANCELLED_PREFIX) {
                    return Ok(RuntimeJobAdapterOutcome {
                        outcome: AdapterOutcome::cancelled(format!(
                            "{tool_name} cancelled before durable runtime job launch"
                        )),
                        job: None,
                    });
                }
                let mut outcome = Self::failure(tool_name, error, Some(command));
                outcome.outcome.warnings = merge_warnings(warnings, outcome.outcome.warnings);
                Ok(outcome)
            }
        }
    }

    fn status(
        tool_name: &str,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
    ) -> Result<RuntimeJobAdapterOutcome, String> {
        let id = runtime_job_id(tool_name, args)?;
        match RuntimeJobService::status_at(context.cache_root.clone(), id) {
            Ok(snapshot) => Ok(Self::success(
                format!("{tool_name} read durable runtime job {id}"),
                runtime_job_snapshot_value(&snapshot),
            )),
            Err(error) => Ok(Self::failure(tool_name, error, None)),
        }
    }

    fn wait(
        tool_name: &str,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
    ) -> Result<RuntimeJobAdapterOutcome, String> {
        let id = runtime_job_id(tool_name, args)?;
        let timeout_seconds = args
            .get("timeoutSeconds")
            .and_then(Value::as_u64)
            .unwrap_or(30);
        match RuntimeJobService::wait_at(
            context.cache_root.clone(),
            id,
            Duration::from_secs(timeout_seconds),
        ) {
            Ok(snapshot) => Ok(Self::success(
                format!("{tool_name} observed durable runtime job {id}"),
                runtime_job_snapshot_value(&snapshot),
            )),
            Err(error) => Ok(Self::failure(tool_name, error, None)),
        }
    }

    fn logs(
        tool_name: &str,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
    ) -> Result<RuntimeJobAdapterOutcome, String> {
        let id = runtime_job_id(tool_name, args)?;
        let tail_chars = args
            .get("tailChars")
            .and_then(Value::as_u64)
            .map(|value| usize::try_from(value).unwrap_or(usize::MAX))
            .unwrap_or(4096);
        let snapshot = match RuntimeJobService::status_at(context.cache_root.clone(), id) {
            Ok(snapshot) => snapshot,
            Err(error) => return Ok(Self::failure(tool_name, error, None)),
        };
        match RuntimeJobService::logs_at(context.cache_root.clone(), id, tail_chars) {
            Ok(logs) => {
                let mut job = runtime_job_snapshot_value(&snapshot);
                if let Value::Object(ref mut object) = job {
                    object.insert("stdout".to_string(), Value::String(logs.stdout));
                    object.insert("stderr".to_string(), Value::String(logs.stderr));
                    object.insert("stdoutPath".to_string(), Value::String(logs.stdout_path));
                    object.insert("stderrPath".to_string(), Value::String(logs.stderr_path));
                }
                Ok(Self::success(
                    format!("{tool_name} read durable runtime job logs for {id}"),
                    job,
                ))
            }
            Err(error) => Ok(Self::failure(tool_name, error, None)),
        }
    }

    fn cancel(
        tool_name: &str,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
        dry_run: bool,
    ) -> Result<RuntimeJobAdapterOutcome, String> {
        let id = runtime_job_id(tool_name, args)?;
        if dry_run {
            return Ok(RuntimeJobAdapterOutcome {
                outcome: AdapterOutcome {
                    ok: true,
                    summary: format!("dry run: {tool_name} would request cancellation for {id}"),
                    changes: vec!["no cancellation requested because dryRun is true".to_string()],
                    warnings: Vec::new(),
                    errors: Vec::new(),
                    artifacts: Vec::new(),
                    stdout: None,
                    stderr: None,
                    command: None,
                },
                job: None,
            });
        }
        match RuntimeJobService::request_cancel_at(context.cache_root.clone(), id) {
            Ok(snapshot) => Ok(Self::success(
                format!("{tool_name} requested cancellation for durable runtime job {id}"),
                runtime_job_snapshot_value(&snapshot),
            )),
            Err(error) => Ok(Self::failure(tool_name, error, None)),
        }
    }

    fn list(
        tool_name: &str,
        context: &WorkspaceContext,
    ) -> Result<RuntimeJobAdapterOutcome, String> {
        let list = RuntimeJobService::list_at(context.cache_root.clone());
        let jobs = list
            .jobs
            .iter()
            .map(runtime_job_snapshot_value)
            .collect::<Vec<_>>();
        Ok(RuntimeJobAdapterOutcome {
            outcome: AdapterOutcome {
                ok: true,
                summary: format!("{tool_name} listed durable runtime jobs"),
                changes: Vec::new(),
                warnings: list.warnings,
                errors: Vec::new(),
                artifacts: Vec::new(),
                stdout: None,
                stderr: None,
                command: None,
            },
            job: Some(json!({ "jobs": jobs })),
        })
    }

    fn success(summary: String, job: Value) -> RuntimeJobAdapterOutcome {
        RuntimeJobAdapterOutcome {
            outcome: AdapterOutcome {
                ok: true,
                summary,
                changes: Vec::new(),
                warnings: Vec::new(),
                errors: Vec::new(),
                artifacts: Vec::new(),
                stdout: None,
                stderr: None,
                command: None,
            },
            job: Some(job),
        }
    }

    fn failure(
        tool_name: &str,
        error: String,
        command: Option<Vec<String>>,
    ) -> RuntimeJobAdapterOutcome {
        RuntimeJobAdapterOutcome {
            outcome: AdapterOutcome {
                ok: false,
                summary: format!("{tool_name} failed for durable runtime job lifecycle"),
                changes: Vec::new(),
                warnings: Vec::new(),
                errors: vec![redactor(&error)],
                artifacts: Vec::new(),
                stdout: None,
                stderr: Some(format!("{}\n", redactor(&error))),
                command,
            },
            job: None,
        }
    }
}

fn runtime_job_start_request(
    tool_name: &str,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    execution_args: Vec<String>,
    build_preflight: Option<RuntimeBuildPreflight>,
) -> Result<RuntimeJobRequest, String> {
    let operation_name = args
        .get("operation")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{tool_name} requires string `operation` argument"))?;
    let operation = RuntimeJobOperation::from_label(operation_name)?;
    Ok(RuntimeJobRequest::new(
        operation,
        execution_args,
        runtime_job_safe_target(context),
        args.get("output")
            .and_then(Value::as_str)
            .map(str::to_string),
    )
    .with_build_preflight(build_preflight))
}

fn runtime_job_id<'a>(tool_name: &str, args: &'a Map<String, Value>) -> Result<&'a str, String> {
    args.get("jobId")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{tool_name} requires string `jobId` argument"))
}

fn runtime_job_safe_target(context: &WorkspaceContext) -> String {
    let name = context
        .workspace_root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("workspace");
    format!("workspace:{name}")
}

fn runtime_job_snapshot_value(snapshot: &runtime_jobs::RuntimeJobSnapshot) -> Value {
    json!({
        "jobId": snapshot.id,
        "phase": snapshot.phase,
        "operation": snapshot.operation,
        "safeTarget": snapshot.safe_target,
        "createdAt": snapshot.created_at_ms,
        "startedAt": snapshot.started_at_ms,
        "heartbeatAt": snapshot.heartbeat_at_ms,
        "finishedAt": snapshot.finished_at_ms,
        "pid": snapshot.pid,
        "pidIdentity": snapshot.pid_identity,
        "exitCode": snapshot.exit_code,
        "cancelled": snapshot.cancelled,
        "cancelDeferred": snapshot.cancel_deferred,
        "unsafePhase": snapshot.unsafe_phase,
        "timeoutReason": snapshot.timeout_reason,
        "artifactPath": snapshot.artifact_path,
        "stdoutPath": snapshot.stdout_path,
        "stderrPath": snapshot.stderr_path,
        "warnings": snapshot.warnings,
        "waitTimedOut": snapshot.wait_timed_out,
    })
}

#[cfg(test)]
fn process_exit_code_is(status: &str, code: i32) -> bool {
    let status = status.trim();
    status == code.to_string() || status.ends_with(&format!(": {code}"))
}

#[cfg(test)]
fn resolve_source_dir(
    context: &WorkspaceContext,
    args: &Map<String, Value>,
) -> Result<PathBuf, String> {
    resolve_source_root(context, args.get("sourceDir").and_then(Value::as_str))
        .map(|resolved| resolved.path)
}

pub(crate) fn cli_args(args: &Map<String, Value>, redact: bool) -> Result<Vec<String>, String> {
    if args.contains_key("args") {
        return Err(
            "raw args are not accepted by internal adapters; use typed tool arguments".to_string(),
        );
    }

    let mut result = Vec::new();
    for (key, value) in args {
        if matches!(key.as_str(), "dryRun" | "cwd" | "confirm") {
            continue;
        }
        let flag = format!("--{}", kebab_case(key));
        match value {
            Value::Bool(true) => result.push(flag),
            Value::Bool(false) | Value::Null => {}
            Value::Array(items) => {
                for item in items {
                    result.push(flag.clone());
                    result.push(reported_cli_value(key, item, redact));
                }
            }
            other => {
                result.push(flag);
                result.push(reported_cli_value(key, other, redact));
            }
        }
    }
    Ok(result)
}

#[allow(dead_code)]
fn _path_list(paths: &[PathBuf]) -> Vec<String> {
    paths
        .iter()
        .map(|path| path.display().to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::cell::RefCell;
    use std::ffi::{OsStr, OsString};
    use std::fs;
    use std::io::Write;
    use std::path::Path;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};
    use unica_adapter_v8_runner::adapter::test_support::reject_missing_client_mcp_extension;

    #[test]
    fn build_runtime_adapter_dry_run_builds_v8_runner_command() {
        let context = temp_context("build-runtime-dry-run");
        let mut args = Map::new();
        args.insert("sourceSet".to_string(), json!("main"));

        let outcome = CliAdapter::new("v8-runner", &["build"], "build/runtime")
            .invoke("unica.build.load", &args, &context, true, true)
            .unwrap();

        let command = outcome.command.unwrap().join(" ");
        assert!(command.contains("bin/"));
        assert!(command.contains("v8-runner"));
        assert!(!command.contains("run-v8-runner.sh"));
        assert!(command.contains("build"));
        assert!(command.contains("--source-set main"));
        cleanup_context(&context);
    }

    #[test]
    fn runtime_execute_preview_fails_honestly_when_v8_runner_is_missing() {
        let context = temp_context("runtime-preview-missing-v8-runner");
        remove_fake_tool_binaries(&context, "v8-runner");
        let mut args = Map::new();
        args.insert("operation".to_string(), json!("build"));

        let outcome =
            RuntimeAdapter::new(crate::infrastructure::runtime_composition::runtime_environment())
                .invoke("unica.runtime.execute", &args, &context, true, true)
                .expect("preview remains structured");

        assert!(!outcome.ok);
        assert!(outcome.errors.iter().any(|error| {
            error.contains(unica_artifact_delivery::engine::BUNDLED_TOOL_MISSING)
        }));
        cleanup_context(&context);
    }

    #[test]
    fn runtime_adapter_maps_build_to_allowlisted_v8_runner_argv() {
        let context = temp_context("runtime-build-argv");
        let runner = RecordingProcessRunner {
            commands: RefCell::new(Vec::new()),
            output: ProcessOutput {
                status_success: true,
                status: "exit status: 0".to_string(),
                stdout: "ok".to_string(),
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
                stderr_truncated: false,
                stdout_had_invalid_utf8: false,
                stderr_had_invalid_utf8: false,
            },
        };
        let mut args = Map::new();
        args.insert("operation".to_string(), json!("build"));
        args.insert("sourceSet".to_string(), json!("main"));
        args.insert("fullRebuild".to_string(), json!(true));

        let outcome = RuntimeAdapter::with_runner(
            &runner,
            crate::infrastructure::runtime_composition::runtime_environment(),
        )
        .invoke("unica.runtime.execute", &args, &context, false, true)
        .unwrap();

        assert!(outcome.ok);
        let commands = runner.commands.borrow();
        assert_runtime_args_with_primary(
            &context,
            &commands[0].args,
            &["build", "--full-rebuild", "--source-set", "main"],
        );
        assert!(commands[0].timeout.is_none());
        assert!(commands[0].program.to_string_lossy().contains("bin/"));
        assert!(!commands[0]
            .program
            .to_string_lossy()
            .contains("run-v8-runner.sh"));
        drop(commands);
        cleanup_context(&context);
    }

    /// This synchronous adapter must not start a second process after a
    /// partial-failure receipt. The durable worker owns the retry (#404).
    #[test]
    fn runtime_adapter_never_retries_a_failed_partial_build() {
        let mut context = temp_context("runtime-partial-no-sync-retry");
        configure_supported_designer_source(&mut context);
        let partial_message = "load failed for source-set 'main' with exit code 1; platform log: sanitized; platform log path: /tmp/out.log; partial load list path: /tmp/partial.lst";
        let runner = SequenceProcessRunner {
            commands: RefCell::new(Vec::new()),
            outputs: RefCell::new(vec![ProcessOutput {
                status_success: false,
                status: "exit status: 4".to_string(),
                stdout: build_failure_json(
                    json!({ "partial": { "file_count": 1 } }),
                    partial_message,
                ),
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
                stderr_truncated: false,
                stdout_had_invalid_utf8: false,
                stderr_had_invalid_utf8: false,
            }]),
        };
        let args = Map::from_iter([("operation".to_string(), json!("build"))]);

        let outcome = RuntimeAdapter::with_runner(
            &runner,
            crate::infrastructure::runtime_composition::runtime_environment(),
        )
        .invoke("unica.runtime.execute", &args, &context, false, true)
        .unwrap();

        assert!(!outcome.ok);
        assert_eq!(
            runner.commands.borrow().len(),
            1,
            "the synchronous adapter owns exactly one process"
        );
        assert!(
            !outcome
                .warnings
                .iter()
                .any(|warning| warning.contains("retried once")),
            "{:?}",
            outcome.warnings
        );
        // The structured error still reaches the caller; only the retry is gone.
        assert!(
            outcome
                .errors
                .iter()
                .any(|error| error.contains("load failed for source-set")),
            "{:?}",
            outcome.errors
        );
        cleanup_context(&context);
    }

    #[test]
    fn runtime_adapter_trims_selector_for_the_initial_build() {
        let mut context = temp_context("runtime-supported-configuration-trimmed-selector");
        configure_supported_designer_source(&mut context);
        let args = Map::from_iter([
            ("operation".to_string(), json!("build")),
            ("sourceSet".to_string(), json!(" main ")),
        ]);

        let invocation = unica_adapter_v8_runner::adapter::plan_runtime_invocation(
            &args,
            &context,
            crate::infrastructure::runtime_composition::workspace_observer(),
        )
        .expect("runner-compatible selector must reach v8-runner");

        assert_runtime_args_with_primary(
            &context,
            &runtime_args(&invocation.args, false).unwrap(),
            &["build", "--source-set", "main"],
        );
        assert!(invocation.build_preflight.is_some());
        cleanup_context(&context);
    }

    #[test]
    fn runtime_adapter_ignores_invalid_unselected_configuration_format() {
        let mut context = temp_context("runtime-unselected-invalid-configuration-format");
        configure_designer_source(&mut context);
        fs::write(
            context.workspace_root.join("src/.project"),
            "<projectDescription/>",
        )
        .unwrap();
        fs::create_dir_all(context.workspace_root.join("extension")).unwrap();
        fs::write(
            context.workspace_root.join("v8project.yaml"),
            concat!(
                "format: DESIGNER\n",
                "source-set:\n",
                "  - name: main\n",
                "    type: CONFIGURATION\n",
                "    path: src\n",
                "  - name: extension\n",
                "    type: EXTENSION\n",
                "    path: extension\n",
            ),
        )
        .unwrap();
        refresh_test_context(&mut context);
        let args = Map::from_iter([
            ("operation".to_string(), json!("build")),
            ("sourceSet".to_string(), json!("extension")),
        ]);

        let invocation = unica_adapter_v8_runner::adapter::plan_runtime_invocation(
            &args,
            &context,
            crate::infrastructure::runtime_composition::workspace_observer(),
        )
        .expect("unselected contradictory evidence must not block an extension build");

        assert_runtime_args_with_primary(
            &context,
            &runtime_args(&invocation.args, false).unwrap(),
            &["build", "--source-set", "extension"],
        );
        assert!(invocation.build_preflight.is_some());
        cleanup_context(&context);
    }

    /// The durable path gets the same initial/default build and one fallback
    /// plan as the synchronous runtime entry point.
    #[test]
    fn runtime_job_dry_run_reports_the_initial_default_build() {
        let mut context = temp_context("runtime-job-supported-configuration-build");
        configure_supported_designer_source(&mut context);
        let mut args = Map::new();
        args.insert("operation".to_string(), json!("build"));

        let outcome = RuntimeJobAdapter::invoke(
            RuntimeJobAction::Start,
            "unica.runtime.job.start",
            &args,
            &context,
            true,
        )
        .unwrap();

        let command = outcome.outcome.command.unwrap();
        assert_runtime_args_with_primary(&context, &command[1..], &["build"]);
        assert!(outcome
            .outcome
            .warnings
            .iter()
            .any(|warning| warning.contains("retry once")));
        cleanup_context(&context);
    }

    #[test]
    fn runtime_job_preview_fails_honestly_when_v8_runner_is_missing() {
        let context = temp_context("runtime-job-preview-missing-v8-runner");
        remove_fake_tool_binaries(&context, "v8-runner");
        let mut args = Map::new();
        args.insert("operation".to_string(), json!("build"));

        let outcome = RuntimeJobAdapter::invoke(
            RuntimeJobAction::Start,
            "unica.runtime.job.start",
            &args,
            &context,
            true,
        )
        .expect("preview remains structured")
        .outcome;

        assert!(!outcome.ok);
        assert!(outcome.errors.iter().any(|error| {
            error.contains(unica_artifact_delivery::engine::BUNDLED_TOOL_MISSING)
        }));
        cleanup_context(&context);
    }

    #[test]
    fn runtime_job_request_carries_one_full_fallback() {
        let mut context = temp_context("runtime-job-request-supported-configuration");
        configure_supported_designer_source(&mut context);
        let mut args = Map::new();
        args.insert("operation".to_string(), json!("build"));

        let invocation = unica_adapter_v8_runner::adapter::plan_runtime_invocation(
            &args,
            &context,
            crate::infrastructure::runtime_composition::workspace_observer(),
        )
        .unwrap();
        let execution_args = runtime_args(&invocation.args, false).unwrap();
        let request = runtime_job_start_request(
            "unica.runtime.job.start",
            &args,
            &context,
            execution_args,
            invocation.build_preflight,
        )
        .unwrap();

        assert_runtime_args_with_primary(&context, request.raw_argv(), &["build"]);
        let fallback = request
            .full_rebuild_fallback_argv()
            .expect("durable full fallback");
        assert_runtime_args_with_primary(&context, fallback, &["build", "--full-rebuild"]);
        cleanup_context(&context);
    }

    #[test]
    fn runtime_adapter_keeps_incremental_build_for_unsupported_configuration() {
        let mut context = temp_context("runtime-unsupported-configuration-build");
        configure_designer_source(&mut context);
        let runner = RecordingProcessRunner {
            commands: RefCell::new(Vec::new()),
            output: ProcessOutput {
                status_success: true,
                status: "exit status: 0".to_string(),
                stdout: "incremental build completed".to_string(),
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
                stderr_truncated: false,
                stdout_had_invalid_utf8: false,
                stderr_had_invalid_utf8: false,
            },
        };
        let mut args = Map::new();
        args.insert("operation".to_string(), json!("build"));

        let outcome = RuntimeAdapter::with_runner(
            &runner,
            crate::infrastructure::runtime_composition::runtime_environment(),
        )
        .invoke("unica.runtime.execute", &args, &context, false, true)
        .unwrap();

        assert!(outcome.ok);
        assert_runtime_args_with_primary(&context, &runner.commands.borrow()[0].args, &["build"]);
        assert!(outcome
            .warnings
            .iter()
            .any(|warning| warning.contains("retry once")));
        cleanup_context(&context);
    }

    #[test]
    fn runtime_adapter_binds_preflight_config_against_runner_environment_override() {
        let _environment_lock = crate::infrastructure::V8TR_CONFIG_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut context = temp_context("runtime-preflight-config-binding");
        configure_designer_source(&mut context);
        let alternate = context.workspace_root.join("alternate.yaml");
        fs::write(&alternate, "format: DESIGNER\nsource-set: []\n")
            .expect("write alternate runner config");
        let _environment = EnvVarGuard::set("V8TR_CONFIG", &alternate);
        let runner = RecordingProcessRunner {
            commands: RefCell::new(Vec::new()),
            output: ProcessOutput {
                status_success: true,
                status: "exit status: 0".to_string(),
                stdout: "incremental build completed".to_string(),
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
                stderr_truncated: false,
                stdout_had_invalid_utf8: false,
                stderr_had_invalid_utf8: false,
            },
        };
        let args = Map::from_iter([("operation".to_string(), json!("build"))]);

        let outcome = RuntimeAdapter::with_runner(
            &runner,
            crate::infrastructure::runtime_composition::runtime_environment(),
        )
        .invoke("unica.runtime.execute", &args, &context, false, true)
        .unwrap();

        assert!(outcome.ok);
        assert_runtime_args_with_primary(&context, &runner.commands.borrow()[0].args, &["build"]);
        cleanup_context(&context);
    }

    #[test]
    fn runtime_adapter_keeps_incremental_build_for_removed_configuration_support() {
        let mut context = temp_context("runtime-removed-configuration-support");
        configure_designer_source(&mut context);
        let marker = context
            .workspace_root
            .join("src/Ext/ParentConfigurations.bin");
        fs::create_dir_all(marker.parent().expect("support marker parent")).unwrap();
        fs::write(marker, []).expect("write removed support marker");
        let args = Map::from_iter([("operation".to_string(), json!("build"))]);

        let invocation = unica_adapter_v8_runner::adapter::plan_runtime_invocation(
            &args,
            &context,
            crate::infrastructure::runtime_composition::workspace_observer(),
        )
        .unwrap();

        assert_runtime_args_with_primary(
            &context,
            &runtime_args(&invocation.args, false).unwrap(),
            &["build"],
        );
        assert!(invocation
            .warnings
            .iter()
            .any(|warning| warning.contains("retry once")));
        cleanup_context(&context);
    }

    #[test]
    fn runtime_adapter_keeps_the_default_strategy_for_edt_configuration() {
        let mut context = temp_context("runtime-edt-configuration-build");
        fs::create_dir_all(context.workspace_root.join("src")).unwrap();
        fs::write(
            context.workspace_root.join("v8project.yaml"),
            concat!(
                "format: EDT\n",
                "source-set:\n",
                "  - name: main\n",
                "    type: CONFIGURATION\n",
                "    path: src\n",
            ),
        )
        .unwrap();
        refresh_test_context(&mut context);
        let args = Map::from_iter([
            ("operation".to_string(), json!("build")),
            ("fullRebuild".to_string(), json!(false)),
        ]);

        let invocation = unica_adapter_v8_runner::adapter::plan_runtime_invocation(
            &args,
            &context,
            crate::infrastructure::runtime_composition::workspace_observer(),
        )
        .unwrap();

        assert_runtime_args_with_primary(
            &context,
            &runtime_args(&invocation.args, false).unwrap(),
            &["build"],
        );
        assert!(invocation.build_preflight.is_some());
        cleanup_context(&context);
    }

    #[test]
    fn runtime_adapter_does_not_force_nonselected_supported_configuration() {
        let mut context = temp_context("runtime-nonselected-supported-configuration");
        configure_supported_designer_source(&mut context);
        fs::create_dir_all(context.workspace_root.join("extension")).unwrap();
        fs::write(
            context.workspace_root.join("extension/Configuration.xml"),
            include_bytes!(
                "../../../../tests/fixtures/platform_8_3_27/support-edit-bin-only/src/Configuration.xml"
            ),
        )
        .unwrap();
        fs::write(
            context.workspace_root.join("v8project.yaml"),
            concat!(
                "format: DESIGNER\n",
                "source-set:\n",
                "  - name: main\n",
                "    type: CONFIGURATION\n",
                "    path: src\n",
                "  - name: extension\n",
                "    type: EXTENSION\n",
                "    path: extension\n",
            ),
        )
        .unwrap();
        refresh_test_context(&mut context);
        let runner = RecordingProcessRunner {
            commands: RefCell::new(Vec::new()),
            output: ProcessOutput {
                status_success: true,
                status: "exit status: 0".to_string(),
                stdout: "extension build completed".to_string(),
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
                stderr_truncated: false,
                stdout_had_invalid_utf8: false,
                stderr_had_invalid_utf8: false,
            },
        };
        let mut args = Map::new();
        args.insert("operation".to_string(), json!("build"));
        args.insert("sourceSet".to_string(), json!("extension"));

        let outcome = RuntimeAdapter::with_runner(
            &runner,
            crate::infrastructure::runtime_composition::runtime_environment(),
        )
        .invoke("unica.runtime.execute", &args, &context, false, true)
        .unwrap();

        assert!(outcome.ok);
        assert_runtime_args_with_primary(
            &context,
            &runner.commands.borrow()[0].args,
            &["build", "--source-set", "extension"],
        );
        assert!(outcome
            .warnings
            .iter()
            .any(|warning| warning.contains("retry once")));
        cleanup_context(&context);
    }

    #[test]
    fn explicit_full_build_bypasses_unreadable_support_state() {
        let mut context = temp_context("runtime-explicit-full-unreadable-support-state");
        configure_supported_designer_source(&mut context);
        fs::write(
            context
                .workspace_root
                .join("src/Ext/ParentConfigurations.bin"),
            b"not a support-state marker",
        )
        .unwrap();
        let runner = RecordingProcessRunner {
            commands: RefCell::new(Vec::new()),
            output: ProcessOutput {
                status_success: true,
                status: "exit status: 0".to_string(),
                stdout: "full build completed".to_string(),
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
                stderr_truncated: false,
                stdout_had_invalid_utf8: false,
                stderr_had_invalid_utf8: false,
            },
        };
        let mut args = Map::new();
        args.insert("operation".to_string(), json!("build"));
        args.insert("fullRebuild".to_string(), json!(true));

        let outcome = RuntimeAdapter::with_runner(
            &runner,
            crate::infrastructure::runtime_composition::runtime_environment(),
        )
        .invoke("unica.runtime.execute", &args, &context, false, true)
        .unwrap();

        assert!(outcome.ok);
        assert_runtime_args_with_primary(
            &context,
            &runner.commands.borrow()[0].args,
            &["build", "--full-rebuild"],
        );
        assert!(outcome.warnings.is_empty());
        cleanup_context(&context);
    }

    #[test]
    fn incremental_build_with_explicit_config_uses_the_same_fallback_policy() {
        let context = temp_context("runtime-nonprimary-build-config");
        let alternate = context.workspace_root.join("alternate");
        fs::create_dir_all(&alternate).unwrap();
        fs::write(
            alternate.join("v8project.yaml"),
            "source-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        let runner = RecordingProcessRunner {
            commands: RefCell::new(Vec::new()),
            output: ProcessOutput {
                status_success: true,
                status: "exit status: 0".to_string(),
                stdout: String::new(),
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
                stderr_truncated: false,
                stdout_had_invalid_utf8: false,
                stderr_had_invalid_utf8: false,
            },
        };
        let mut args = Map::new();
        args.insert("operation".to_string(), json!("build"));
        args.insert("config".to_string(), json!("alternate/v8project.yaml"));

        let outcome = RuntimeAdapter::with_runner(
            &runner,
            crate::infrastructure::runtime_composition::runtime_environment(),
        )
        .invoke("unica.runtime.execute", &args, &context, false, true)
        .expect("fallback policy must not depend on the primary support reader");

        assert!(outcome.ok);
        let commands = runner.commands.borrow();
        assert_eq!(commands.len(), 1);
        assert!(commands[0]
            .args
            .iter()
            .any(|argument| argument == "--json-message"));
        assert!(!commands[0]
            .args
            .iter()
            .any(|argument| argument == "--full-rebuild"));
        cleanup_context(&context);
    }

    #[test]
    fn incremental_build_keeps_a_case_distinct_explicit_config() {
        let mut context = temp_context("runtime-case-distinct-nonprimary-config");
        configure_designer_source(&mut context);
        let alternate = context.workspace_root.join("V8PROJECT.YAML");
        fs::write(
            &alternate,
            "format: DESIGNER\nsource-set:\n  - name: alternate\n    type: CONFIGURATION\n    path: other\n",
        )
        .unwrap();
        if fs::canonicalize(&alternate).unwrap()
            == fs::canonicalize(context.workspace_root.join("v8project.yaml")).unwrap()
        {
            cleanup_context(&context);
            return;
        }
        let args = Map::from_iter([
            ("operation".to_string(), json!("build")),
            (
                "config".to_string(),
                json!(alternate.file_name().unwrap().to_string_lossy()),
            ),
        ]);

        let plan = unica_adapter_v8_runner::adapter::plan_runtime_invocation(
            &args,
            &context,
            crate::infrastructure::runtime_composition::workspace_observer(),
        )
        .expect("an explicit config has its own build identity");

        assert_eq!(
            plan.args.get("config").and_then(Value::as_str),
            alternate.to_str()
        );
        assert!(plan.build_preflight.is_some());
        cleanup_context(&context);
    }

    #[test]
    fn runtime_adapter_uses_global_edt_mode_despite_platform_xml_evidence() {
        let mut context = temp_context("runtime-edt-mode-with-platform-evidence");
        fs::create_dir_all(context.workspace_root.join("src")).unwrap();
        fs::write(
            context.workspace_root.join("v8project.yaml"),
            concat!(
                "format: EDT\n",
                "source-set:\n",
                "  - name: main\n",
                "    type: CONFIGURATION\n",
                "    path: src\n",
            ),
        )
        .unwrap();
        fs::write(
            context.workspace_root.join("src/Configuration.xml"),
            "<MetaDataObject/>",
        )
        .unwrap();
        refresh_test_context(&mut context);
        let args = Map::from_iter([
            ("operation".to_string(), json!("build")),
            ("sourceSet".to_string(), json!("main")),
        ]);

        let invocation = unica_adapter_v8_runner::adapter::plan_runtime_invocation(
            &args,
            &context,
            crate::infrastructure::runtime_composition::workspace_observer(),
        )
        .expect("global EDT mode reaches v8-runner unchanged");

        assert_runtime_args_with_primary(
            &context,
            &runtime_args(&invocation.args, false).unwrap(),
            &["build", "--source-set", "main"],
        );
        assert!(invocation.build_preflight.is_some());
        cleanup_context(&context);
    }

    #[test]
    fn runtime_adapter_delegates_successful_build_without_wrapper_timeout() {
        let mut context = temp_context("runtime-build-success");
        configure_designer_source(&mut context);
        let runner = RecordingProcessRunner {
            commands: RefCell::new(Vec::new()),
            output: ProcessOutput {
                status_success: true,
                status: "exit status: 0".to_string(),
                stdout: "Designer build completed after 240 seconds".to_string(),
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
                stderr_truncated: false,
                stdout_had_invalid_utf8: false,
                stderr_had_invalid_utf8: false,
            },
        };
        let mut args = Map::new();
        args.insert("operation".to_string(), json!("build"));

        let outcome = RuntimeAdapter::with_runner(
            &runner,
            crate::infrastructure::runtime_composition::runtime_environment(),
        )
        .invoke("unica.runtime.execute", &args, &context, false, true)
        .unwrap();

        assert!(outcome.ok);
        assert_eq!(
            outcome.stdout.as_deref(),
            Some("Designer build completed after 240 seconds")
        );
        assert!(runner.commands.borrow()[0].timeout.is_none());
        cleanup_context(&context);
    }

    /// #343. Credentials written into the connection string were carried into
    /// the project config and then silently dropped: the run reported a wrong
    /// password while the same connection worked in Designer. The safe answer
    /// is an early refusal naming where credentials belong — and it must not
    /// echo the secret it just refused.
    #[test]
    fn runtime_adapter_refuses_credentials_hidden_in_the_connection_string() {
        for connection in [
            "File=build/ib;Usr=Админ;Pwd=с3кр3т;",
            "File=build/ib;usr=Админ;pwd=с3кр3т;",
        ] {
            let mut args = Map::new();
            args.insert("operation".to_string(), json!("config-init"));
            args.insert("config".to_string(), json!("./v8project.yaml"));
            args.insert("connection".to_string(), json!(connection));
            args.insert("format".to_string(), json!("edt"));
            args.insert("builder".to_string(), json!("IBCMD"));

            let error = validate_runtime_mapper_payload("config-init", &args)
                .expect_err("credentials in the connection string are refused, not dropped");

            assert!(error.contains("connection"), "{error}");
            assert!(error.contains("user"), "{error}");
            assert!(
                !error.contains("с3кр3т"),
                "the refusal must not echo the secret: {error}"
            );
        }
    }

    /// A connection without credentials stays accepted.
    #[test]
    fn runtime_adapter_accepts_a_connection_without_credentials() {
        let mut args = Map::new();
        args.insert("operation".to_string(), json!("config-init"));
        args.insert("config".to_string(), json!("./v8project.yaml"));
        args.insert("connection".to_string(), json!("File=build/ib"));
        args.insert("format".to_string(), json!("edt"));
        args.insert("builder".to_string(), json!("IBCMD"));

        validate_runtime_mapper_payload("config-init", &args).unwrap();
    }

    /// #408. A project that declares a client MCP extension but has no
    /// artifact used to fail deep inside `build`, on a cause the caller could
    /// have been told before the run started.
    #[test]
    fn runtime_adapter_refuses_build_when_the_declared_client_mcp_artifact_is_missing() {
        let context = temp_context("client-mcp-preflight");
        std::fs::write(
            context.cwd.join("v8project.yaml"),
            "format: DESIGNER\ntools:\n  client_mcp:\n    extension:\n      artifact:\n        path: .build/client_mcp.cfe\n",
        )
        .unwrap();
        let mut args = Map::new();
        args.insert("operation".to_string(), json!("build"));

        let error = reject_missing_client_mcp_extension(&args, &context)
            .expect_err("a declared artifact that is absent is refused before the run");

        assert!(!error.contains("tools-download"), "{error}");
        assert!(error.contains("#870"), "{error}");
        assert!(error.contains("client_mcp.cfe"), "{error}");

        // Present artifact, and a project that declares none, both pass.
        std::fs::create_dir_all(context.cwd.join(".build")).unwrap();
        std::fs::write(context.cwd.join(".build/client_mcp.cfe"), b"cfe").unwrap();
        reject_missing_client_mcp_extension(&args, &context).unwrap();

        std::fs::write(context.cwd.join("v8project.yaml"), "format: DESIGNER\n").unwrap();
        reject_missing_client_mcp_extension(&args, &context).unwrap();
        cleanup_context(&context);
    }

    #[test]
    fn runtime_adapter_rejects_external_config_before_reading_its_declared_artifacts() {
        let context = temp_context("external-runtime-config-containment");
        let outside = tempfile::tempdir().unwrap();
        let outside_config = outside.path().join("v8project.yaml");
        std::fs::write(
            &outside_config,
            "format: DESIGNER\ntools:\n  client_mcp:\n    extension:\n      artifact:\n        path: outside.cfe\n",
        )
        .unwrap();
        let runner = RecordingProcessRunner {
            commands: RefCell::new(Vec::new()),
            output: ProcessOutput {
                status_success: true,
                status: "exit status: 0".to_string(),
                stdout: String::new(),
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
                stderr_truncated: false,
                stdout_had_invalid_utf8: false,
                stderr_had_invalid_utf8: false,
            },
        };
        let args = Map::from_iter([
            ("operation".to_string(), json!("build")),
            (
                "config".to_string(),
                json!(outside_config.to_string_lossy().into_owned()),
            ),
        ]);

        let error = RuntimeAdapter::with_runner(
            &runner,
            crate::infrastructure::runtime_composition::runtime_environment(),
        )
        .invoke("unica.runtime.execute", &args, &context, false, true)
        .expect_err("external config must be rejected before its YAML is inspected");

        assert!(error.contains("outside the workspace"), "{error}");
        assert!(!error.contains("outside.cfe"), "{error}");
        assert!(runner.commands.borrow().is_empty());
        cleanup_context(&context);
    }

    #[test]
    fn runtime_adapter_maps_config_init_config_to_output_arg() {
        let mut args = Map::new();
        args.insert("operation".to_string(), json!("config-init"));
        args.insert("config".to_string(), json!("./v8project.yaml"));
        args.insert("connection".to_string(), json!("File=build/ib"));
        args.insert("format".to_string(), json!("edt"));
        args.insert("builder".to_string(), json!("IBCMD"));

        let argv = runtime_args(&args, false).unwrap();

        assert_eq!(
            argv,
            vec![
                "config",
                "init",
                "--output",
                "./v8project.yaml",
                "--connection",
                "File=build/ib",
                "--format",
                "edt",
                "--builder",
                "IBCMD"
            ]
        );
    }

    #[test]
    fn runtime_adapter_binds_existing_external_processor_config_without_running_v8_runner() {
        let context = temp_context("runtime-external-config-bind");
        let primary = concat!(
            "format: DESIGNER\n",
            "source-set:\n",
            "  - name: external-processors\n",
            "    type: EXTERNAL_DATA_PROCESSORS\n",
            "    path: epf\n",
        );
        std::fs::write(context.cwd.join("v8project.yaml"), primary).unwrap();
        let runner = RecordingProcessRunner {
            commands: RefCell::new(Vec::new()),
            output: ProcessOutput {
                status_success: true,
                status: "exit status: 0".to_string(),
                stdout: "runner must not execute".to_string(),
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
                stderr_truncated: false,
                stdout_had_invalid_utf8: false,
                stderr_had_invalid_utf8: false,
            },
        };
        let mut args = Map::new();
        args.insert("operation".to_string(), json!("config-init"));
        args.insert("config".to_string(), json!("v8project.yaml"));
        args.insert("sourceSet".to_string(), json!("external-processors"));
        args.insert(
            "connection".to_string(),
            json!("File=/private/local/epf-harness"),
        );

        let outcome = RuntimeAdapter::with_runner(
            &runner,
            crate::infrastructure::runtime_composition::runtime_environment(),
        )
        .invoke("unica.runtime.execute", &args, &context, false, true)
        .unwrap();

        assert!(outcome.ok, "{outcome:?}");
        assert!(runner.commands.borrow().is_empty());
        assert_eq!(
            std::fs::read_to_string(context.cwd.join("v8project.local.yaml")).unwrap(),
            "infobase:\n  connection: File=/private/local/epf-harness\n"
        );
        assert_eq!(
            std::fs::read_to_string(context.cwd.join("v8project.yaml")).unwrap(),
            primary
        );
        assert!(!serde_json::to_string(&outcome)
            .unwrap()
            .contains("/private/local/epf-harness"));
        cleanup_context(&context);
    }

    #[test]
    fn runtime_external_processor_bind_dry_run_validates_without_writing_or_running() {
        let context = temp_context("runtime-external-config-bind-preview");
        std::fs::write(
            context.cwd.join("v8project.yaml"),
            "source-set:\n  external-processors:\n    type: EXTERNAL_DATA_PROCESSORS\n    path: epf\n",
        )
        .unwrap();
        let runner = RecordingProcessRunner {
            commands: RefCell::new(Vec::new()),
            output: ProcessOutput {
                status_success: true,
                status: "exit status: 0".to_string(),
                stdout: String::new(),
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
                stderr_truncated: false,
                stdout_had_invalid_utf8: false,
                stderr_had_invalid_utf8: false,
            },
        };
        let mut args = Map::new();
        args.insert("operation".to_string(), json!("config-init"));
        args.insert("config".to_string(), json!("v8project.yaml"));
        args.insert("sourceSet".to_string(), json!("external-processors"));
        args.insert("connection".to_string(), json!("File=build/ib"));

        let outcome = RuntimeAdapter::with_runner(
            &runner,
            crate::infrastructure::runtime_composition::runtime_environment(),
        )
        .invoke("unica.runtime.execute", &args, &context, true, true)
        .unwrap();

        assert!(outcome.ok, "{outcome:?}");
        assert!(outcome.summary.contains("dry run"));
        assert!(outcome.command.is_none());
        assert!(runner.commands.borrow().is_empty());
        assert!(!context.cwd.join("v8project.local.yaml").exists());
        cleanup_context(&context);
    }

    #[test]
    fn runtime_external_processor_bind_rejects_unsafe_or_ambiguous_inputs() {
        let context = temp_context("runtime-external-config-bind-guards");
        let mut args = Map::new();
        args.insert("operation".to_string(), json!("config-init"));
        args.insert("config".to_string(), json!("v8project.yaml"));
        args.insert("sourceSet".to_string(), json!("external-processors"));
        args.insert("connection".to_string(), json!("File=build/ib"));

        for (config, expected) in [
            (
                "source-set:\n  - name: external-processors\n    type: CONFIGURATION\n    path: src\n",
                "must have type `EXTERNAL_DATA_PROCESSORS`",
            ),
            (
                "source-set:\n  - name: external-processors\n    type: EXTERNAL_DATA_PROCESSORS\n    path: ''\n",
                "must have a non-empty `path`",
            ),
            (
                "source-set:\n  - name: external-processors\n    type: EXTERNAL_DATA_PROCESSORS\n    path: one\n  - name: external-processors\n    type: EXTERNAL_DATA_PROCESSORS\n    path: two\n",
                "exactly one source-set",
            ),
        ] {
            std::fs::write(context.cwd.join("v8project.yaml"), config).unwrap();
            let error = RuntimeAdapter::new(crate::infrastructure::runtime_composition::runtime_environment())
                .invoke("unica.runtime.execute", &args, &context, false, true)
                .unwrap_err();
            assert!(error.contains(expected), "{error}");
            assert!(!context.cwd.join("v8project.local.yaml").exists());
        }

        std::fs::write(
            context.cwd.join("v8project.yaml"),
            "source-set:\n  - name: external-processors\n    type: EXTERNAL_DATA_PROCESSORS\n    path: epf\n",
        )
        .unwrap();
        for key in ["format", "builder", "force"] {
            args.insert(
                key.to_string(),
                if key == "force" {
                    json!(false)
                } else {
                    json!("x")
                },
            );
            let error = RuntimeAdapter::new(
                crate::infrastructure::runtime_composition::runtime_environment(),
            )
            .invoke("unica.runtime.execute", &args, &context, false, true)
            .unwrap_err();
            assert!(
                error.contains(&format!("does not accept `{key}`")),
                "{error}"
            );
            args.remove(key);
        }
        std::fs::write(context.cwd.join("v8project.local.yaml"), "infobase: {}\n").unwrap();
        let error =
            RuntimeAdapter::new(crate::infrastructure::runtime_composition::runtime_environment())
                .invoke("unica.runtime.execute", &args, &context, false, true)
                .unwrap_err();
        assert!(error.contains("refuses to overwrite"), "{error}");
        cleanup_context(&context);
    }

    #[test]
    fn runtime_ordinary_config_init_does_not_read_existing_config() {
        let context = temp_context("runtime-ordinary-config-init-delegation");
        std::fs::write(context.cwd.join("v8project.yaml"), "not: [valid").unwrap();
        let runner = RecordingProcessRunner {
            commands: RefCell::new(Vec::new()),
            output: ProcessOutput {
                status_success: true,
                status: "exit status: 0".to_string(),
                stdout: "created".to_string(),
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
                stderr_truncated: false,
                stdout_had_invalid_utf8: false,
                stderr_had_invalid_utf8: false,
            },
        };
        let mut args = Map::new();
        args.insert("operation".to_string(), json!("config-init"));
        args.insert("config".to_string(), json!("v8project.yaml"));
        args.insert("connection".to_string(), json!("File=build/ib"));

        let outcome = RuntimeAdapter::with_runner(
            &runner,
            crate::infrastructure::runtime_composition::runtime_environment(),
        )
        .invoke("unica.runtime.execute", &args, &context, false, true)
        .unwrap();

        assert!(outcome.ok, "{outcome:?}");
        assert_eq!(runner.commands.borrow().len(), 1);
        cleanup_context(&context);
    }

    #[test]
    fn runtime_adapter_maps_test_and_launch_mcp_va() {
        let mut test_args = Map::new();
        test_args.insert("operation".to_string(), json!("test"));
        test_args.insert("testRunner".to_string(), json!("yaxunit"));
        test_args.insert("fullOutput".to_string(), json!(true));
        test_args.insert("testScope".to_string(), json!("module"));
        test_args.insert("module".to_string(), json!("CommonModule.Тесты"));

        assert_eq!(
            runtime_args(&test_args, false).unwrap(),
            vec!["test", "yaxunit", "--full", "module", "CommonModule.Тесты"]
        );

        let mut launch_args = Map::new();
        launch_args.insert("operation".to_string(), json!("launch"));
        launch_args.insert("clientMode".to_string(), json!("mcp-va"));
        launch_args.insert("mode".to_string(), json!("thin"));
        launch_args.insert("mcpPort".to_string(), json!(1550));

        assert_eq!(
            runtime_args(&launch_args, false).unwrap(),
            vec![
                "launch",
                "mcp",
                "va",
                "--mode",
                "thin",
                "--mcp-port",
                "1550"
            ]
        );
    }

    #[test]
    fn runtime_adapter_maps_bounded_external_epf_launch() {
        let args = json!({
            "operation": "launch",
            "clientMode": "thin",
            "execute": "tests/Smoke.epf",
            "output": "build/smoke.stdout.log",
            "stderrOutput": "build/smoke.stderr.log",
            "waitForExit": true,
            "waitTimeoutMs": 30_000,
        })
        .as_object()
        .unwrap()
        .clone();

        assert_eq!(
            runtime_args(&args, false).unwrap(),
            vec![
                "--json-message",
                "launch",
                "thin",
                "--execute",
                "tests/Smoke.epf",
                "--output",
                "build/smoke.stdout.log",
                "--stderr-output",
                "build/smoke.stderr.log",
                "--wait-for-exit",
                "--wait-timeout-ms",
                "30000",
            ]
        );
    }

    #[test]
    fn runtime_adapter_rejects_invalid_bounded_external_epf_launch() {
        let invalid = [
            json!({
                "operation": "launch",
                "clientMode": "thick",
                "execute": "tests/Smoke.epf",
                "output": "build/out.log",
                "stderrOutput": "build/err.log",
                "waitForExit": true,
                "waitTimeoutMs": 30_000,
            }),
            json!({
                "operation": "launch",
                "clientMode": "thin",
                "execute": "tests/Smoke.epf",
                "output": "build/out.log",
                "stderrOutput": "build/out.log",
                "waitForExit": true,
                "waitTimeoutMs": 30_000,
            }),
            json!({
                "operation": "launch",
                "clientMode": "thin",
                "execute": "tests/Smoke.epf",
                "output": "build/out.log",
                "stderrOutput": "build/err.log",
                "waitForExit": true,
                "waitTimeoutMs": 0,
            }),
            json!({
                "operation": "launch",
                "clientMode": "thin",
                "execute": "tests/Smoke.epf",
                "output": "build/out.log",
                "stderrOutput": "build/err.log",
                "waitForExit": true,
                "waitTimeoutMs": 30_000,
                "rawKeys": ["/Execute tests/Other.epf"],
            }),
        ];

        for input in invalid {
            let error = runtime_args(input.as_object().unwrap(), false).unwrap_err();
            assert!(error.contains("bounded external EPF"), "{error}");
        }
    }

    #[test]
    fn runtime_job_start_rejects_bounded_external_epf_arguments() {
        let context = temp_context("runtime-job-rejects-bounded-external-epf");
        let bounded_arguments = [
            ("waitForExit", json!(true)),
            ("waitTimeoutMs", json!(30_000)),
            ("stderrOutput", json!("build/smoke.stderr.log")),
        ];

        for (argument, value) in bounded_arguments {
            let mut args = json!({
                "operation": "launch",
                "clientMode": "thin",
            })
            .as_object()
            .unwrap()
            .clone();
            args.insert(argument.to_string(), value);

            let error = match RuntimeJobAdapter::invoke(
                RuntimeJobAction::Start,
                "unica.runtime.job.start",
                &args,
                &context,
                true,
            ) {
                Ok(_) => panic!("runtime.job.start accepted bounded argument `{argument}`"),
                Err(error) => error,
            };

            assert!(error.contains(argument), "{error}");
            assert!(error.contains("unica.runtime.execute"), "{error}");
        }

        cleanup_context(&context);
    }

    #[test]
    fn runtime_adapter_reports_waited_external_epf_exit_and_artifacts() {
        let context = temp_context("runtime-waited-external-epf");
        let runner = FakeProcessRunner {
            output: ProcessOutput {
                status_success: true,
                status: "exit status: 0".to_string(),
                stdout: json!({
                    "ok": true,
                    "data": {
                        "external_epf_wait": {
                            "pid": 42,
                            "execute_path": "tests/Smoke.epf",
                            "exit_code": 7,
                            "timed_out": false,
                            "output_path": "build/smoke.stdout.log",
                            "stderr_path": "build/smoke.stderr.log"
                        }
                    }
                })
                .to_string(),
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
                stderr_truncated: false,
                stdout_had_invalid_utf8: false,
                stderr_had_invalid_utf8: false,
            },
        };
        let args = json!({
            "operation": "launch",
            "clientMode": "thin",
            "execute": "tests/Smoke.epf",
            "output": "build/smoke.stdout.log",
            "stderrOutput": "build/smoke.stderr.log",
            "waitForExit": true,
            "waitTimeoutMs": 30_000,
        })
        .as_object()
        .unwrap()
        .clone();

        let result = RuntimeAdapter::with_runner(
            &runner,
            crate::infrastructure::runtime_composition::runtime_environment(),
        )
        .invoke_with_data("unica.runtime.execute", &args, &context, false, true)
        .unwrap();
        let outcome = result.outcome;

        assert!(!outcome.ok);
        assert!(outcome.errors.iter().any(|error| error.contains("code 7")));
        assert_eq!(
            outcome.artifacts,
            vec![
                "build/smoke.stdout.log".to_string(),
                "build/smoke.stderr.log".to_string()
            ]
        );
        let wait = &result.data.unwrap()["external_epf_wait"];
        assert_eq!(wait["pid"], 42);
        assert_eq!(wait["execute_path"], "tests/Smoke.epf");
        assert_eq!(wait["exit_code"], 7);
        assert_eq!(wait["timed_out"], false);
        cleanup_context(&context);
    }

    #[test]
    fn runtime_adapter_parses_wait_envelope_before_redacting_public_output() {
        let context = temp_context("runtime-waited-external-epf-unredacted-json");
        let runner = FakeProcessRunner {
            output: ProcessOutput {
                status_success: true,
                status: "exit status: 0".to_string(),
                stdout: json!({
                    "ok": true,
                    "data": {
                        "external_epf_wait": {
                            "pid": 42,
                            "execute_path": "tests/Smoke.epf",
                            "exit_code": 0,
                            "timed_out": false,
                            "output_path": "build/token=private/out.log",
                            "stderr_path": "build/smoke.stderr.log"
                        }
                    }
                })
                .to_string(),
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
                stderr_truncated: false,
                stdout_had_invalid_utf8: false,
                stderr_had_invalid_utf8: false,
            },
        };
        let args = json!({
            "operation": "launch",
            "clientMode": "thin",
            "execute": "tests/Smoke.epf",
            "output": "build/token=private/out.log",
            "stderrOutput": "build/smoke.stderr.log",
            "waitForExit": true,
            "waitTimeoutMs": 30_000,
        })
        .as_object()
        .unwrap()
        .clone();

        let result = RuntimeAdapter::with_runner(
            &runner,
            crate::infrastructure::runtime_composition::runtime_environment(),
        )
        .invoke_with_data("unica.runtime.execute", &args, &context, false, true)
        .unwrap();
        let outcome = result.outcome;

        assert!(outcome.ok, "{:?}", outcome.errors);
        assert_eq!(outcome.artifacts[1], "build/smoke.stderr.log");
        let artifacts = outcome.artifacts.join("\n");
        assert!(!artifacts.contains("private"), "{artifacts}");
        assert!(artifacts.contains("<redacted>"), "{artifacts}");
        assert!(!outcome.stdout.unwrap().contains("private"));
        let command = outcome.command.unwrap().join("\n");
        assert!(!command.contains("private"), "{command}");
        assert!(command.contains("<redacted>"), "{command}");
        let data = result.data.unwrap().to_string();
        assert!(!data.contains("private"), "{data}");
        assert!(data.contains("<redacted>"), "{data}");
        cleanup_context(&context);
    }

    #[test]
    fn runtime_adapter_preserves_prelaunch_runner_error_without_wait_payload() {
        let context = temp_context("runtime-waited-external-epf-prelaunch-error");
        let runner = FakeProcessRunner {
            output: ProcessOutput {
                status_success: false,
                status: "exit status: 2".to_string(),
                stdout: json!({
                    "ok": false,
                    "command": "launch",
                    "data": {
                        "message": "config file not found: v8project.yaml"
                    },
                    "error": {
                        "code": "invalid_argument",
                        "kind": "validation",
                        "message": "config file not found: v8project.yaml"
                    }
                })
                .to_string(),
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
                stderr_truncated: false,
                stdout_had_invalid_utf8: false,
                stderr_had_invalid_utf8: false,
            },
        };
        let args = bounded_external_epf_args();

        let outcome = RuntimeAdapter::with_runner(
            &runner,
            crate::infrastructure::runtime_composition::runtime_environment(),
        )
        .invoke("unica.runtime.execute", &args, &context, false, true)
        .unwrap();

        assert!(!outcome.ok);
        assert!(outcome
            .errors
            .iter()
            .any(|error| error.contains("config file not found")));
        assert!(outcome
            .errors
            .iter()
            .all(|error| !error.contains("external_epf_wait")));
        cleanup_context(&context);
    }

    #[test]
    fn runtime_adapter_rejects_mismatched_wait_artifact_receipt() {
        let context = temp_context("runtime-waited-external-epf-mismatched-receipt");
        let runner = FakeProcessRunner {
            output: ProcessOutput {
                status_success: true,
                status: "exit status: 0".to_string(),
                stdout: json!({
                    "ok": true,
                    "data": {
                        "external_epf_wait": {
                            "pid": 42,
                            "execute_path": "tests/Other.epf",
                            "exit_code": 0,
                            "timed_out": false,
                            "output_path": "build/smoke.stdout.log",
                            "stderr_path": "build/smoke.stderr.log"
                        }
                    }
                })
                .to_string(),
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
                stderr_truncated: false,
                stdout_had_invalid_utf8: false,
                stderr_had_invalid_utf8: false,
            },
        };
        let args = bounded_external_epf_args();

        let outcome = RuntimeAdapter::with_runner(
            &runner,
            crate::infrastructure::runtime_composition::runtime_environment(),
        )
        .invoke("unica.runtime.execute", &args, &context, false, true)
        .unwrap();

        assert!(!outcome.ok);
        assert!(outcome
            .errors
            .iter()
            .any(|error| error.contains("`execute_path` does not match")));
        assert!(outcome.artifacts.is_empty());
        cleanup_context(&context);
    }

    #[test]
    fn bounded_external_epf_rejects_case_only_output_aliases() {
        if path_lock_identity(Path::new("Out.log")) != path_lock_identity(Path::new("out.log")) {
            return;
        }

        let context = temp_context("runtime-waited-external-epf-case-alias");
        let args = json!({
            "waitForExit": true,
            "output": "build/Out.log",
            "stderrOutput": "build/out.log",
        })
        .as_object()
        .unwrap()
        .clone();

        let error = unica_adapter_v8_runner::adapter::validate_bounded_external_epf_artifact_paths(
            &args,
            &context.cwd,
        )
        .unwrap_err();

        assert!(error.contains("resolve to the same path"), "{error}");
        cleanup_context(&context);
    }

    fn bounded_external_epf_args() -> Map<String, Value> {
        json!({
            "operation": "launch",
            "clientMode": "thin",
            "execute": "tests/Smoke.epf",
            "output": "build/smoke.stdout.log",
            "stderrOutput": "build/smoke.stderr.log",
            "waitForExit": true,
            "waitTimeoutMs": 30_000,
        })
        .as_object()
        .unwrap()
        .clone()
    }

    #[test]
    fn runtime_adapter_maps_each_runtime_operation_to_expected_argv() {
        let cases = vec![
            (json!({"operation": "init"}), vec!["init"]),
            (
                json!({
                    "operation": "dump",
                    "mode": "partial",
                    "object": "Catalog:Номенклатура",
                    "sourceSet": "main",
                    "extension": "MyExtension",
                }),
                vec![
                    "dump",
                    "--mode",
                    "partial",
                    "--object",
                    "Catalog:Номенклатура",
                    "--source-set",
                    "main",
                    "--extension",
                    "MyExtension",
                ],
            ),
            (
                json!({
                    "operation": "convert",
                    "sourceSet": "main",
                    "output": "build/convert",
                }),
                vec![
                    "convert",
                    "--source-set",
                    "main",
                    "--output",
                    "build/convert",
                ],
            ),
            (
                json!({
                    "operation": "make",
                    "output": "build/config.cf",
                    "sourceSet": "main",
                }),
                vec![
                    "make",
                    "--output",
                    "build/config.cf",
                    "--source-set",
                    "main",
                ],
            ),
            (
                json!({
                    "operation": "load",
                    "path": "build/config.cf",
                    "mode": "merge",
                    "settings": "merge-settings.xml",
                }),
                vec![
                    "load",
                    "--path",
                    "build/config.cf",
                    "--mode",
                    "merge",
                    "--settings",
                    "merge-settings.xml",
                ],
            ),
            (
                json!({
                    "operation": "syntax",
                    "mode": "designer-modules",
                    "server": true,
                    "thinClient": true,
                }),
                vec!["syntax", "designer-modules", "--server", "--thin-client"],
            ),
            (
                json!({
                    "operation": "extensions",
                    "sourceSet": "MyExtension",
                }),
                vec!["extensions", "--name", "MyExtension"],
            ),
            (
                json!({
                    "operation": "dump",
                    "mode": "partial",
                    "objects": ["Catalog:Номенклатура", "Document:ЗаказПокупателя"],
                }),
                vec![
                    "dump",
                    "--mode",
                    "partial",
                    "--object",
                    "Catalog:Номенклатура",
                    "--object",
                    "Document:ЗаказПокупателя",
                ],
            ),
            (
                json!({
                    "operation": "syntax",
                    "mode": "edt",
                    "projects": ["Configuration", "Tests"],
                }),
                vec![
                    "syntax",
                    "edt",
                    "--project",
                    "Configuration",
                    "--project",
                    "Tests",
                ],
            ),
            (
                json!({
                    "operation": "test",
                    "testRunner": "va",
                    "fullOutput": true,
                    "features": ["features/smoke.feature"],
                    "filterTags": ["@smoke"],
                    "ignoreTags": ["@wip"],
                    "scenarioFilters": ["Open form"],
                }),
                vec![
                    "test",
                    "va",
                    "--full",
                    "--feature",
                    "features/smoke.feature",
                    "--filter-tag",
                    "@smoke",
                    "--ignore-tag",
                    "@wip",
                    "--scenario-filter",
                    "Open form",
                ],
            ),
            (
                json!({
                    "operation": "extensions",
                    "sourceSets": ["Sales", "Warehouse"],
                }),
                vec!["extensions", "--name", "Sales", "--name", "Warehouse"],
            ),
        ];

        for (input, expected) in cases {
            let args = input.as_object().unwrap().clone();
            assert_eq!(runtime_args(&args, false).unwrap(), expected);
        }

        // Маршрут загрузки зависимостей через раннер снят: продукт его не
        // маппит ни в какую команду (#871, B-2).
        let removed = json!({"operation": "tools-download", "tool": "client-mcp"});
        let error = runtime_args(removed.as_object().unwrap(), false).unwrap_err();
        assert!(error.contains("unknown runtime operation"), "{error}");
    }

    #[test]
    fn runtime_adapter_rejects_operation_specific_unsupported_args() {
        let cases = vec![
            (
                json!({"operation": "build", "extension": "MyExtension"}),
                "operation `build` does not accept `extension`",
            ),
            (
                json!({"operation": "convert", "path": "src"}),
                "operation `convert` does not accept `path`",
            ),
            (
                json!({"operation": "test", "testRunner": "yaxunit", "fullRebuild": true}),
                "operation `test` does not accept `fullRebuild`",
            ),
            (
                json!({"operation": "load", "path": "build/config.cf", "mode": "update"}),
                "load --mode update is not supported",
            ),
            (
                json!({"operation": "load", "path": "build/config.cf", "settings": "merge-settings.xml"}),
                "operation `load` accepts `settings` only with mode `merge`",
            ),
            (
                json!({"operation": "dump", "mode": "partial"}),
                "operation `dump` with mode `partial` requires `object` or `objects`",
            ),
        ];

        for (input, expected) in cases {
            let args = input.as_object().unwrap().clone();
            let error = runtime_args(&args, false).unwrap_err();
            assert!(
                error.contains(expected),
                "expected error containing {expected:?}, got {error:?}"
            );
        }
    }

    #[test]
    fn runtime_adapter_rejects_raw_args_vector() {
        let mut args = Map::new();
        args.insert("operation".to_string(), json!("build"));
        args.insert("args".to_string(), json!(["--unsafe", "../outside"]));

        let error = runtime_args(&args, false).unwrap_err();

        assert!(error.contains("raw args are not accepted"));
    }

    #[test]
    fn diagnostics_adapter_still_builds_bsl_analyzer_analyze_command() {
        let context = temp_context("diagnostics-analyze-dry-run");
        let mut args = Map::new();
        args.insert("sourceDir".to_string(), json!("src"));

        let outcome = CliAdapter::new("bsl-analyzer", &["analyze"], "code analysis")
            .invoke("unica.code.diagnostics", &args, &context, true, false)
            .unwrap();

        let command = outcome.command.unwrap().join(" ");
        assert!(command.contains("bin/"));
        assert!(command.contains("bsl-analyzer"));
        assert!(!command.contains("run-bsl-analyzer.sh"));
        assert!(command.contains("analyze"));
        assert!(command.contains("--source-dir src"));
        cleanup_context(&context);
    }

    #[test]
    fn diagnostics_preview_fails_honestly_when_bsl_analyzer_is_missing() {
        let context = temp_context("diagnostics-preview-missing-analyzer");
        remove_fake_tool_binaries(&context, "bsl-analyzer");
        let args = Map::new();

        let outcome = invoke_analyze(
            &BslAnalyzerMcpAdapter::new(crate::infrastructure::analysis_composition::environment()),
            &args,
            &context,
            true,
        )
        .outcome;

        assert!(!outcome.ok);
        assert!(outcome.errors.iter().any(|error| {
            error.contains(unica_artifact_delivery::engine::BUNDLED_TOOL_MISSING)
        }));
        cleanup_context(&context);
    }

    #[test]
    fn multi_source_set_resolve_source_dir_selects_main_configuration_root() {
        let context = temp_context("multi-source-set");
        fs::write(
            context.workspace_root.join("v8project.yaml"),
            r#"
source-set:
  - name: main
    type: CONFIGURATION
    path: src/cf
  - name: TESTS
    type: EXTENSION
    path: exts/TESTS
"#,
        )
        .unwrap();
        fs::create_dir_all(context.workspace_root.join("src/cf")).unwrap();
        fs::write(
            context.workspace_root.join("src/cf/Configuration.xml"),
            "<MetaDataObject/>",
        )
        .unwrap();

        let selected = resolve_source_dir(&context, &Map::new()).unwrap();

        assert_eq!(
            selected,
            normalize_path_identity(&context.workspace_root.join("src/cf")).unwrap()
        );
        cleanup_context(&context);
    }

    #[test]
    fn diagnostics_analyze_normalizes_json_format_and_keeps_limit_out_of_cli_args() {
        let context = temp_context("diagnostics-format-dry-run");
        let mut args = Map::new();
        args.insert("sourceDir".to_string(), json!("src/extensions/Smoke"));
        args.insert("format".to_string(), json!("json"));
        args.insert("limit".to_string(), json!(20));

        let outcome = invoke_analyze(
            &BslAnalyzerMcpAdapter::new(crate::infrastructure::analysis_composition::environment()),
            &args,
            &context,
            true,
        )
        .outcome;

        let command = outcome.command.unwrap().join(" ");
        assert!(command.contains("bin/"));
        assert!(command.contains("bsl-analyzer"));
        assert!(!command.contains("run-bsl-analyzer.sh"));
        assert!(command.contains("analyze"));
        assert!(command.contains("--source-dir src/extensions/Smoke"));
        assert!(command.contains("--format jsonl"));
        assert!(!command.contains("--limit"));
        assert!(!command.contains(" 20"));
        cleanup_context(&context);
    }

    #[test]
    fn diagnostics_analyze_default_forces_jsonl_and_returns_typed_data() {
        let context = temp_context("diagnostics-default-jsonl");
        let runner = RecordingProcessRunner {
            commands: RefCell::new(Vec::new()),
            output: analyze_process_output(concat!(
                "{\"type\":\"start\",\"total_files\":0,\"version\":\"0.2.62\"}\n",
                "{\"type\":\"done\",\"elapsed_secs\":0.1,\"total_files\":0,",
                "\"total_diagnostics\":0,\"failed_files\":0}\n",
            )),
        };

        let analyzer = invoke_analyze(
            &BslAnalyzerMcpAdapter::with_process_runner(
                &runner,
                crate::infrastructure::analysis_composition::environment(),
            ),
            &Map::new(),
            &context,
            false,
        );

        assert!(analyzer.outcome.ok, "{:?}", analyzer.outcome);
        assert!(analyzer.outcome.stdout.is_none());
        let batch = analyzer.diagnostics.as_ref().expect("typed diagnostics");
        assert_eq!(
            batch.outcome.status,
            unica_diagnostics::model::DiagnosticProviderStatus::Empty
        );
        assert!(batch.outcome.observations.is_empty());
        assert!(runner.commands.borrow()[0]
            .args
            .windows(2)
            .any(|pair| pair == ["--format", "jsonl"]));
        cleanup_context(&context);
    }

    #[test]
    fn diagnostics_analyze_preserves_cyrillic_paths_through_typed_jsonl() {
        let context = temp_context("diagnostics-cyrillic-jsonl-path");
        let expected_path = "CommonModules/РеактивныйКлиент/Ext/Module.bsl";
        let file = json!({
            "type": "file",
            "path": expected_path,
            "diagnostics": [{
                "code": "LineLength",
                "message": "Длина строки превышает максимальную",
                "severity": "Warning",
                "start_line": 10,
                "start_column": 0,
                "end_line": 10,
                "end_column": 150,
                "tags": []
            }]
        });
        let runner = RecordingProcessRunner {
            commands: RefCell::new(Vec::new()),
            output: analyze_process_output(&format!(
                concat!(
                    "{{\"type\":\"start\",\"total_files\":1,\"version\":\"0.2.62\"}}\n",
                    "{file}\n",
                    "{{\"type\":\"done\",\"elapsed_secs\":0.1,\"total_files\":1,",
                    "\"total_diagnostics\":1,\"failed_files\":0}}\n"
                ),
                file = file,
            )),
        };

        let analyzer = invoke_analyze(
            &BslAnalyzerMcpAdapter::with_process_runner(
                &runner,
                crate::infrastructure::analysis_composition::environment(),
            ),
            &Map::new(),
            &context,
            false,
        );

        assert!(analyzer.outcome.ok, "{:?}", analyzer.outcome);
        assert!(analyzer.outcome.stdout.is_none());
        let observation = &analyzer
            .diagnostics
            .as_ref()
            .expect("typed diagnostics")
            .outcome
            .observations[0];
        assert!(matches!(
            observation,
            unica_diagnostics::model::DiagnosticObservation::Diagnostic {
                location: unica_diagnostics::model::DiagnosticObservationLocation::Resource { handle },
                message,
                ..
            } if handle == expected_path && message == "Длина строки превышает максимальную"
        ));
        assert!(runner.commands.borrow()[0]
            .args
            .windows(2)
            .any(|pair| pair == ["--format", "jsonl"]));
        cleanup_context(&context);
    }

    #[test]
    fn diagnostics_analyze_uses_custom_timeout_without_forwarding_cli_argument() {
        let context = temp_context("diagnostics-custom-timeout");
        let runner = RecordingProcessRunner {
            commands: RefCell::new(Vec::new()),
            output: analyze_process_output(ANALYZE_JSONL_EMPTY),
        };
        let mut args = Map::new();
        args.insert("timeoutSeconds".to_string(), json!(900));

        let analyzer = invoke_analyze(
            &BslAnalyzerMcpAdapter::with_process_runner(
                &runner,
                crate::infrastructure::analysis_composition::environment(),
            ),
            &args,
            &context,
            false,
        );
        let outcome = analyzer.outcome;

        assert!(outcome.ok);
        let commands = runner.commands.borrow();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].timeout, Some(Duration::from_secs(900)));
        assert!(commands[0].args.iter().all(|arg| arg != "900"));
        assert!(commands[0]
            .args
            .iter()
            .all(|arg| arg != "--timeout-seconds"));
        assert!(outcome
            .command
            .unwrap()
            .iter()
            .all(|arg| arg != "900" && arg != "--timeout-seconds"));
        cleanup_context(&context);
    }

    #[test]
    fn diagnostics_analyze_without_configured_timeout_runs_without_process_timeout() {
        let context = temp_context("diagnostics-default-timeout");
        let runner = RecordingProcessRunner {
            commands: RefCell::new(Vec::new()),
            output: analyze_process_output(ANALYZE_JSONL_EMPTY),
        };
        let operational_config =
            crate::infrastructure::operational_config::load_operational_config(
                &context.workspace_root,
            )
            .unwrap();

        let outcome = BslAnalyzerMcpAdapter::with_process_runner(
            &runner,
            crate::infrastructure::analysis_composition::environment(),
        )
        .test_invoke_diagnostics_analyze_scoped(
            "unica.code.diagnostics",
            &Map::new(),
            &context,
            false,
            Some(&operational_config),
            &CancellationToken::new(),
            None,
        )
        .unwrap()
        .outcome;

        assert!(outcome.ok);
        assert_eq!(
            runner.commands.borrow()[0].timeout,
            None,
            "an unconfigured analysis must not receive a process timeout"
        );
        assert!(runner.commands.borrow()[0]
            .args
            .iter()
            .all(|arg| arg != "--timeout-seconds" && arg != "timeoutSeconds"));
        cleanup_context(&context);
    }

    #[test]
    fn diagnostics_analyze_uses_workspace_operational_config_default() {
        let context = temp_context("diagnostics-operational-config-timeout");
        fs::write(
            context.workspace_root.join("unica.toml"),
            r#"[operational.code_diagnostics]
analyze_timeout_seconds = 900
"#,
        )
        .unwrap();
        let runner = RecordingProcessRunner {
            commands: RefCell::new(Vec::new()),
            output: analyze_process_output(ANALYZE_JSONL_EMPTY),
        };
        let operational_config =
            crate::infrastructure::operational_config::load_operational_config(
                &context.workspace_root,
            )
            .unwrap();

        let outcome = BslAnalyzerMcpAdapter::with_process_runner(
            &runner,
            crate::infrastructure::analysis_composition::environment(),
        )
        .test_invoke_diagnostics_analyze_scoped(
            "unica.code.diagnostics",
            &Map::new(),
            &context,
            false,
            Some(&operational_config),
            &CancellationToken::new(),
            None,
        )
        .unwrap()
        .outcome;

        assert!(outcome.ok);
        assert_eq!(
            runner.commands.borrow()[0].timeout,
            Some(Duration::from_secs(900))
        );
        cleanup_context(&context);
    }

    #[test]
    fn diagnostics_analyze_timeout_reports_budget_and_preserves_stderr() {
        let context = temp_context("diagnostics-timeout-report");
        let runner = FakeProcessRunner {
            output: ProcessOutput {
                status_success: false,
                status: "timeout".to_string(),
                stdout: String::new(),
                stderr: "partial analyzer diagnostics".to_string(),
                timed_out: true,
                cancelled: false,
                stdout_truncated: false,
                stderr_truncated: false,
                stdout_had_invalid_utf8: false,
                stderr_had_invalid_utf8: false,
            },
        };
        let mut args = Map::new();
        args.insert("timeoutSeconds".to_string(), json!(900));

        let analyzer = invoke_analyze(
            &BslAnalyzerMcpAdapter::with_process_runner(
                &runner,
                crate::infrastructure::analysis_composition::environment(),
            ),
            &args,
            &context,
            false,
        );
        let outcome = analyzer.outcome;

        assert!(!outcome.ok);
        assert!(outcome
            .errors
            .iter()
            .any(|error| error.contains("timed out after 900 seconds")));
        assert!(outcome
            .warnings
            .iter()
            .any(|warning| warning.contains("timed out after 900 seconds")));
        assert!(outcome
            .errors
            .iter()
            .any(|error| error == "partial analyzer diagnostics"));
        assert_eq!(
            outcome.stderr.as_deref(),
            Some("partial analyzer diagnostics")
        );
        assert!(analyzer.data.is_none());
        cleanup_context(&context);
    }

    /// #275. A checkout that has not downloaded the tools has no
    /// `bsl-analyzer` in its manifest. `code.search` already answers that
    /// workstation state with an unavailable section and a working result;
    /// `code.graph` must not turn the same cause into a failed call.
    #[test]
    fn bsl_graph_adapter_degrades_when_the_analyzer_is_not_bundled() {
        let context = temp_context("graph-missing-analyzer");
        drop_tool_from_fake_manifest(&context.cwd, "bsl-analyzer");
        let runner = RecordingBslMcpRunner {
            commands: RefCell::new(Vec::new()),
            output: BslMcpOutput {
                result_text: String::new(),
                stderr: String::new(),
            },
        };
        let mut args = Map::new();
        args.insert("mode".to_string(), json!("resolve"));
        args.insert("query".to_string(), json!("ВидыНоменклатуры"));

        let analyzer = BslAnalyzerMcpAdapter::with_runner(
            &runner,
            crate::infrastructure::analysis_composition::environment(),
        )
        .invoke("unica.code.graph", &args, &context, false)
        .expect("an absent provider is a reportable state, not a failed call");

        assert!(!analyzer.outcome.ok, "{:?}", analyzer.outcome);
        assert!(
            analyzer
                .outcome
                .errors
                .iter()
                .any(|error| error.starts_with("provider_unavailable:")),
            "the answer names the cause in a machine-readable form: {:?}",
            analyzer.outcome.errors
        );
        assert!(
            analyzer
                .outcome
                .errors
                .iter()
                .any(|error| error.contains("bsl-analyzer")),
            "{:?}",
            analyzer.outcome.errors
        );
        assert!(
            runner.commands.borrow().is_empty(),
            "an absent provider is never invoked"
        );
        cleanup_context(&context);
    }

    #[test]
    fn bsl_graph_preview_fails_honestly_when_the_analyzer_binary_is_missing() {
        let context = temp_context("graph-preview-missing-analyzer");
        remove_fake_tool_binaries(&context, "bsl-analyzer");
        let mut args = Map::new();
        args.insert("mode".to_string(), json!("resolve"));
        args.insert("query".to_string(), json!("ВидыНоменклатуры"));

        let outcome =
            BslAnalyzerMcpAdapter::new(crate::infrastructure::analysis_composition::environment())
                .invoke("unica.code.graph", &args, &context, true)
                .expect("preview remains structured")
                .outcome;

        assert!(!outcome.ok);
        assert!(outcome.errors.iter().any(|error| {
            error.contains(unica_artifact_delivery::engine::BUNDLED_TOOL_MISSING)
        }));
        cleanup_context(&context);
    }

    #[test]
    fn bsl_graph_adapter_maps_typed_args_to_allowlisted_mcp_call() {
        let context = temp_context("graph-mcp");
        let runner = RecordingBslMcpRunner {
            commands: RefCell::new(Vec::new()),
            output: BslMcpOutput {
                result_text: "{\"action\":\"callers\",\"nodes\":[]}".to_string(),
                stderr: String::new(),
            },
        };
        let mut args = Map::new();
        args.insert("mode".to_string(), json!("callers"));
        args.insert("id".to_string(), json!("method:CommonModule.Smoke.Run"));
        args.insert("edgeKinds".to_string(), json!(["call"]));
        args.insert("provenance".to_string(), json!(["direct"]));
        args.insert("maxOutputTokens".to_string(), json!(1200));
        args.insert("limit".to_string(), json!(25));

        let analyzer = BslAnalyzerMcpAdapter::with_runner(
            &runner,
            crate::infrastructure::analysis_composition::environment(),
        )
        .invoke("unica.code.graph", &args, &context, false)
        .unwrap();

        assert!(analyzer.outcome.ok);
        // ADR-0023: the analyzer reply is the result, not a JSON string wrapped
        // in a section header.
        assert!(analyzer.outcome.stdout.is_none());
        assert_eq!(
            analyzer.data.unwrap(),
            json!({"action": "callers", "nodes": []})
        );
        let commands = runner.commands.borrow();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].tool_name, "graph");
        assert_eq!(commands[0].tool_args["action"], "callers");
        assert_eq!(commands[0].tool_args["edge_kinds"], json!(["call"]));
        assert_eq!(commands[0].tool_args["provenance"], json!(["direct"]));
        assert_eq!(commands[0].tool_args["max_output_tokens"], 1200);
        assert_eq!(commands[0].tool_args["max_nodes"], 25);
        assert!(commands[0].args.contains(&"mcp".to_string()));
        assert!(commands[0].args.contains(&"stdio".to_string()));
        assert!(commands[0].args.contains(
            &normalize_path_identity(&context.cwd)
                .unwrap()
                .display()
                .to_string()
        ));
        cleanup_context(&context);
    }

    #[test]
    fn legacy_diagnostics_paths_are_rejected_before_any_runner_call() {
        let context = temp_context("diagnostics-path-containment");
        fs::create_dir_all(context.workspace_root.join("src")).unwrap();
        let outside = context.workspace_root.with_file_name(format!(
            "{}-outside",
            context
                .workspace_root
                .file_name()
                .unwrap()
                .to_string_lossy()
        ));
        fs::create_dir_all(&outside).unwrap();
        let absolute_outside_path = outside.join("Module.bsl");

        let mut paths = vec![
            "../outside/Module.bsl".to_string(),
            absolute_outside_path.display().to_string(),
        ];
        if let Some(symlink_result) = unica_workspace::test_support::create_dir_symlink_for_test(
            &outside,
            context.workspace_root.join("src").join("escape"),
        ) {
            symlink_result.unwrap();
            paths.push("escape/Module.bsl".to_string());
        }

        for path in paths {
            let runner = RecordingBslMcpRunner {
                commands: RefCell::new(Vec::new()),
                output: BslMcpOutput {
                    result_text: "{\"action\":\"file\",\"findings\":[]}".to_string(),
                    stderr: String::new(),
                },
            };
            let mut args = Map::new();
            args.insert("mode".to_string(), json!("file"));
            args.insert("sourceDir".to_string(), json!("src"));
            args.insert("path".to_string(), json!(path));

            let error = BslAnalyzerMcpAdapter::with_runner(
                &runner,
                crate::infrastructure::analysis_composition::environment(),
            )
            .invoke("unica.code.diagnostics", &args, &context, false)
            .unwrap_err();
            assert!(
                error.starts_with("unsupported bsl-analyzer MCP tool:"),
                "{error}"
            );
            assert!(runner.commands.borrow().is_empty());
        }

        cleanup_context(&context);
        let _ = fs::remove_dir_all(outside);
    }

    #[test]
    fn bsl_mcp_adapter_reports_loading_as_non_fatal_warning() {
        let context = temp_context("graph-loading");
        let runner = RecordingBslMcpRunner {
            commands: RefCell::new(Vec::new()),
            output: BslMcpOutput {
                result_text: "{\"action\":\"status\",\"reload\":\"running\",\"state\":\"loading\"}"
                    .to_string(),
                stderr: String::new(),
            },
        };
        let mut args = Map::new();
        args.insert("mode".to_string(), json!("status"));

        let outcome = BslAnalyzerMcpAdapter::with_runner(
            &runner,
            crate::infrastructure::analysis_composition::environment(),
        )
        .invoke("unica.code.graph", &args, &context, false)
        .unwrap()
        .outcome;

        assert!(outcome.ok);
        assert!(outcome
            .warnings
            .iter()
            .any(|warning| warning.contains("not ready")));
        cleanup_context(&context);
    }

    const ANALYZE_JSONL_EMPTY: &str = concat!(
        "{\"type\":\"start\",\"total_files\":0,\"version\":\"0.2.62\"}\n",
        "{\"type\":\"done\",\"elapsed_secs\":0.1,\"total_files\":0,",
        "\"total_diagnostics\":0,\"failed_files\":0}\n",
    );

    fn analyze_process_output(stdout: &str) -> ProcessOutput {
        ProcessOutput {
            status_success: true,
            status: "exit status: 0".to_string(),
            stdout: stdout.to_string(),
            stderr: String::new(),
            timed_out: false,
            cancelled: false,
            stdout_truncated: false,
            stderr_truncated: false,
            stdout_had_invalid_utf8: false,
            stderr_had_invalid_utf8: false,
        }
    }

    fn invoke_analyze(
        adapter: &BslAnalyzerMcpAdapter<'_>,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
        dry_run: bool,
    ) -> BslAnalyzerOutcome {
        adapter
            .test_invoke_diagnostics_analyze_scoped(
                "bsl-analyzer diagnostics provider",
                args,
                context,
                dry_run,
                None,
                &CancellationToken::new(),
                None,
            )
            .unwrap()
    }

    fn analyze_outcome(format: Option<&str>, stdout: &str, label: &str) -> BslAnalyzerOutcome {
        let context = temp_context(label);
        let runner = FakeProcessRunner {
            output: analyze_process_output(stdout),
        };
        let mut args = Map::new();
        if let Some(format) = format {
            args.insert("format".to_string(), json!(format));
        }

        let outcome = invoke_analyze(
            &BslAnalyzerMcpAdapter::with_process_runner(
                &runner,
                crate::infrastructure::analysis_composition::environment(),
            ),
            &args,
            &context,
            false,
        );

        cleanup_context(&context);
        outcome
    }

    #[test]
    fn diagnostics_analyze_stream_without_findings_reports_pending() {
        // The analyzer emits the `start` opener before it builds its database and
        // batches every `file` event plus the closing `done` at the end of the
        // run, so a stream that stopped at `start` is a build that never reported.
        let outcome = analyze_outcome(
            Some("jsonl"),
            "{\"type\":\"start\",\"total_files\":812,\"version\":\"0.2.62\"}\n",
            "diagnostics-analyze-pending",
        );

        assert!(!outcome.outcome.ok);
        assert_eq!(
            outcome
                .diagnostics
                .as_ref()
                .expect("typed diagnostics")
                .outcome
                .status,
            unica_diagnostics::model::DiagnosticProviderStatus::Unavailable
        );
        assert!(outcome.outcome.stdout.is_none());
        assert!(outcome
            .outcome
            .errors
            .iter()
            .any(|error| error.starts_with("diagnostics_pending")
                && error.contains("did not report files")));
    }

    #[test]
    fn diagnostics_analyze_findings_quoting_the_stream_contract_stay_successful() {
        // Readiness is read off each event's own `type`, so findings that quote
        // the stream's vocabulary — including a source line spelling out a `done`
        // event — must not decide whether the run completed.
        let outcome = analyze_outcome(
            Some("jsonl"),
            concat!(
                "{\"type\":\"start\",\"total_files\":1,\"version\":\"0.2.62\"}\n",
                "{\"type\":\"file\",\"path\":\"CommonModules/Probe/Ext/Module.bsl\",",
                "\"diagnostics\":[{\"code\":\"LineLength\",",
                "\"message\":\"literal {\\\"type\\\":\\\"done\\\"} is not ready for review\",",
                "\"severity\":\"Warning\",\"start_line\":1,\"start_column\":0,",
                "\"end_line\":1,\"end_column\":1,\"tags\":[]}]}\n",
                "{\"type\":\"done\",\"elapsed_secs\":0.4,\"total_files\":1,",
                "\"total_diagnostics\":1,\"failed_files\":0}\n",
            ),
            "diagnostics-analyze-complete",
        );

        assert!(outcome.outcome.ok, "{:?}", outcome.outcome);
        assert!(outcome.outcome.errors.is_empty());
        assert!(outcome.outcome.stdout.is_none());
        assert_eq!(
            outcome
                .diagnostics
                .as_ref()
                .expect("typed diagnostics")
                .outcome
                .observations
                .len(),
            1
        );
    }

    #[test]
    fn diagnostics_analyze_format_aliases_have_the_same_typed_result() {
        let results = [None, Some("json"), Some("jsonl")]
            .into_iter()
            .enumerate()
            .map(|(index, format)| {
                analyze_outcome(
                    format,
                    ANALYZE_JSONL_EMPTY,
                    &format!("diagnostics-analyze-alias-{index}"),
                )
            })
            .collect::<Vec<_>>();
        for result in &results {
            assert!(result.outcome.ok, "{:?}", result.outcome);
            assert!(result.outcome.stdout.is_none());
        }
        assert_eq!(results[0].diagnostics, results[1].diagnostics);
        assert_eq!(results[1].diagnostics, results[2].diagnostics);
    }

    #[test]
    fn diagnostics_analyze_parses_more_than_the_legacy_stdout_capture_limit() {
        let files = 20_000usize;
        let mut stream =
            format!("{{\"type\":\"start\",\"total_files\":{files},\"version\":\"0.2.62\"}}\n");
        for index in 0..files {
            stream.push_str(&format!(
                "{{\"type\":\"file\",\"path\":\"Modules/{index}.bsl\",\"diagnostics\":[],\"error\":\"parse failed\"}}\n"
            ));
        }
        stream.push_str(&format!(
            "{{\"type\":\"done\",\"elapsed_secs\":1.0,\"total_files\":{files},\"total_diagnostics\":0,\"failed_files\":{files}}}\n"
        ));
        assert!(stream.len() > 1024 * 1024);

        let result = analyze_outcome(None, &stream, "diagnostics-analyze-large-stream");

        assert!(result.outcome.ok, "{:?}", result.outcome);
        assert!(result.outcome.stdout.is_none());
        let batch = result.diagnostics.unwrap();
        assert_eq!(batch.files.discovered, Some(files));
        assert_eq!(batch.files.failed, Some(files));
        assert_eq!(batch.outcome.observations.len(), files);
    }

    #[test]
    fn diagnostics_analyze_rejects_one_oversized_line_without_publishing_it() {
        let oversized = "x".repeat(MAX_DIAGNOSTICS_JSONL_LINE_BYTES + 1);

        let result = analyze_outcome(None, &oversized, "diagnostics-analyze-oversized-line");

        assert!(!result.outcome.ok);
        assert!(result.outcome.stdout.is_none());
        assert!(result.outcome.errors[0].starts_with("diagnostics_invalid"));
        assert_eq!(
            result.diagnostics.unwrap().outcome.error.unwrap().code,
            "diagnostics_invalid"
        );
    }

    #[test]
    fn diagnostics_analyze_reports_earliest_oversize_or_parse_failure() {
        let oversized = "private-token".repeat(MAX_DIAGNOSTICS_JSONL_LINE_BYTES / 13 + 1);
        for (index, (stream, expected)) in [
            (
                format!("{oversized}\ninvalid-event"),
                "line 1: line exceeds 8388608 bytes.",
            ),
            (
                format!("invalid-event\n{oversized}"),
                "line 1: invalid JSON or event schema.",
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let result =
                analyze_outcome(None, &stream, &format!("diagnostics-error-order-{index}"));
            assert!(!result.outcome.ok);
            assert!(result.outcome.stdout.is_none());
            assert!(!result.outcome.errors.join(" ").contains("private-token"));
            let batch = result.diagnostics.unwrap();
            assert!(batch.outcome.observations.is_empty());
            assert!(batch.outcome.error.unwrap().message.starts_with(expected));
        }
    }

    #[test]
    fn scoped_analysis_keeps_source_root_and_cleans_full_file_filter_on_every_exit() {
        struct ScopedRunner {
            root: PathBuf,
            module: PathBuf,
            filter: RefCell<Option<PathBuf>>,
            exit: &'static str,
        }
        impl ProcessRunner for ScopedRunner {
            fn run(&self, command: &ProcessCommand) -> Result<ProcessOutput, String> {
                let argument = |flag: &str| {
                    command
                        .args
                        .windows(2)
                        .find(|pair| pair[0] == flag)
                        .map(|pair| pair[1].clone())
                        .unwrap()
                };
                assert_eq!(command.args[0], "analyze");
                assert_eq!(
                    normalize_path_identity(Path::new(&argument("--source-dir"))).unwrap(),
                    self.root
                );
                assert_eq!(argument("--format"), "jsonl");
                assert!(!command.args.iter().any(|arg| arg == "--changed-files"));
                let filter = PathBuf::from(argument("--diff-filter"));
                assert!(!normalize_path_identity(&filter)
                    .unwrap()
                    .starts_with(&self.root));
                let document: Value = serde_json::from_slice(&fs::read(&filter).unwrap()).unwrap();
                assert_eq!(
                    document,
                    json!({"base_ref":"unica-check", "head_ref":"current", "files":{self.module.to_str().unwrap():{"hunks":null}}})
                );
                *self.filter.borrow_mut() = Some(filter);
                if self.exit == "transport_failure" {
                    return Err("test process failure".into());
                }
                let paths = if self.exit == "missing_target" {
                    Vec::new()
                } else {
                    vec![self.module.clone()]
                };
                let mut lines = vec![
                    json!({"type":"start", "total_files":paths.len(), "version":"test"})
                        .to_string(),
                ];
                lines.extend(
                    paths.iter().map(|path| {
                        json!({"type":"file", "path":path, "diagnostics":[]}).to_string()
                    }),
                );
                lines.push(json!({"type":"done", "elapsed_secs":0.1, "total_files":paths.len(), "total_diagnostics":0, "failed_files":0}).to_string());
                let mut output = analyze_process_output(&lines.join("\n"));
                output.cancelled = self.exit == "cancelled";
                Ok(output)
            }
        }
        let context = temp_context("scoped, анализ test");
        let root = normalize_path_identity(&context.cwd).unwrap();
        let module = root.join("CommonModules/Обмен/Ext/Module.bsl");
        fs::create_dir_all(module.parent().unwrap()).unwrap();
        let source = "Процедура Выполнить() Экспорт\nКонецПроцедуры\n";
        fs::write(&module, source).unwrap();
        for exit in [
            "success",
            "missing_target",
            "transport_failure",
            "cancelled",
        ] {
            let runner = ScopedRunner {
                root: root.clone(),
                module: module.clone(),
                filter: RefCell::new(None),
                exit,
            };
            let result = BslAnalyzerMcpAdapter::with_process_runner(
                &runner,
                crate::infrastructure::analysis_composition::environment(),
            )
            .analyze_diagnostic_batch(
                &context,
                &root,
                Some(&module),
                Some(Duration::from_secs(30)),
                &CancellationToken::new(),
            );
            let filter = runner
                .filter
                .borrow()
                .clone()
                .expect("runner must read filter while it exists");
            assert!(!filter.exists(), "filter leaked on {exit}");
            assert_eq!(fs::read_to_string(&module).unwrap(), source);
            match exit {
                "success" => assert!(result.unwrap().outcome.complete),
                "missing_target" => {
                    let outcome = result.unwrap().outcome;
                    assert!(!outcome.complete);
                    assert_eq!(outcome.error.unwrap().code, "diagnostics_incomplete");
                }
                _ => assert!(result.is_err(), "{exit}: {result:?}"),
            }
        }
        cleanup_context(&context);
    }

    /// bsl-analyzer v0.2.86 validates `[diagnostics.baseline]` before `start`
    /// and exits 1 with the cause on stderr (`analyze.rs`). The module check
    /// must see a named, incomplete section, not "provider is unavailable";
    /// any other failed run keeps failing as before.
    #[test]
    fn analyze_names_a_broken_baseline_the_cli_refused_before_start() {
        let context = temp_context("baseline refused, анализ");
        let root = normalize_path_identity(&context.cwd).unwrap();
        let module = root.join("CommonModules/Обмен/Ext/Module.bsl");
        fs::create_dir_all(module.parent().unwrap()).unwrap();
        fs::write(&module, "Процедура Выполнить() Экспорт\nКонецПроцедуры\n").unwrap();
        let run = |stderr: &str, timed_out: bool| {
            let mut output = analyze_process_output("");
            output.status_success = false;
            output.status = "exit status: 1".to_string();
            output.stderr = stderr.to_string();
            output.timed_out = timed_out;
            let runner = FakeProcessRunner { output };
            BslAnalyzerMcpAdapter::with_process_runner(
                &runner,
                crate::infrastructure::analysis_composition::environment(),
            )
            .analyze_diagnostic_batch(
                &context,
                &root,
                Some(&module),
                Some(Duration::from_secs(30)),
                &CancellationToken::new(),
            )
        };
        // The real stderr shape, with a spaced path outside the workspace.
        let stderr = "Error: \"cannot read diagnostics baseline /Users/dev/My Projects/base dir/\
                      missing.json: No such file or directory (os error 2)\"\n";

        let batch = run(stderr, false).expect("a broken baseline is a named result");
        let outcome = batch.outcome;
        assert_eq!(
            outcome.status,
            unica_diagnostics::model::DiagnosticProviderStatus::Completed
        );
        assert!(!outcome.complete);
        assert!(outcome.error.is_none());
        assert!(outcome.observations.is_empty());
        let reason = outcome
            .suppressions
            .into_iter()
            .next()
            .unwrap()
            .reason()
            .cloned()
            .expect("cause is named");
        assert_eq!(reason.code, "missing");
        assert_eq!(
            reason.detail.as_deref(),
            Some("cannot read diagnostics baseline missing.json: No such file or directory (os error 2)")
        );

        // A timeout is a timeout even if stderr ends with a baseline line, and
        // an unrelated failure stays an unexplained one.
        assert!(run(stderr, true).is_err());
        assert!(run("Error: \"metadata is unreadable\"\n", false).is_err());
        cleanup_context(&context);
    }

    /// A run that gives no diagnostics batch says where it stopped: the
    /// engine is absent, failed its install checks, could not start, ran out
    /// of time or exited with an error (#1338).
    #[test]
    fn analyze_failure_names_where_the_run_stopped() {
        struct StagedFailure(ProcessStreamFailure);
        impl ProcessRunner for StagedFailure {
            fn run(&self, _: &ProcessCommand) -> Result<ProcessOutput, String> {
                panic!("streaming runner expected")
            }

            fn run_streaming_staged(
                &self,
                _: &ProcessCommand,
                _: usize,
                _: &mut dyn FnMut(usize, &[u8]) -> StreamControl,
            ) -> Result<ProcessStreamOutput, ProcessStreamFailure> {
                Err(self.0.clone())
            }
        }
        let context = temp_context("run failure kinds, анализ");
        let root = normalize_path_identity(&context.cwd).unwrap();
        let analyze = |runner: &dyn ProcessRunner| {
            BslAnalyzerMcpAdapter::with_process_runner(
                runner,
                crate::infrastructure::analysis_composition::environment(),
            )
            .analyze_diagnostic_batch(
                &context,
                &root,
                None,
                Some(Duration::from_secs(30)),
                &CancellationToken::new(),
            )
            .expect_err("a failed run has no batch")
        };
        let failed = |timed_out: bool| {
            let mut output = analyze_process_output("");
            output.status_success = false;
            output.status = "exit status: 3".to_string();
            output.stderr = "panic at private-token".to_string();
            output.timed_out = timed_out;
            FakeProcessRunner { output }
        };

        assert_eq!(
            analyze(&StagedFailure(ProcessStreamFailure::Spawn(
                "Permission denied (os error 13)".to_string()
            )))
            .kind,
            AnalyzerRunFailureKind::StartFailed
        );
        // A failure while waiting for or stopping a started process before
        // its deadline is not a failed start.
        assert_eq!(
            analyze(&StagedFailure(ProcessStreamFailure::Run(
                "try_wait failed".to_string()
            )))
            .kind,
            AnalyzerRunFailureKind::Other
        );
        assert_eq!(
            analyze(&failed(true)).kind,
            AnalyzerRunFailureKind::TimedOut
        );
        assert_eq!(analyze(&failed(false)).kind, AnalyzerRunFailureKind::Exited);
        assert!(analyze(&failed(false)).missing.is_none());

        let plugin_bin = context.cwd.join("plugins/unica/bin");
        let analyzers = fs::read_dir(&plugin_bin)
            .unwrap()
            .flat_map(|target| fs::read_dir(target.unwrap().path()).unwrap())
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.file_stem().is_some_and(|stem| stem == "bsl-analyzer"))
            .collect::<Vec<_>>();
        assert!(!analyzers.is_empty());
        for analyzer in &analyzers {
            fs::write(analyzer, "tampered").unwrap();
        }
        let tampered = analyze(&failed(false));
        assert_eq!(
            tampered.kind,
            AnalyzerRunFailureKind::EngineInvalid,
            "a checksum mismatch is not an absent engine"
        );
        assert!(tampered.missing.is_none());
        for analyzer in &analyzers {
            fs::remove_file(analyzer).unwrap();
        }
        let absent = analyze(&failed(false));
        assert_eq!(absent.kind, AnalyzerRunFailureKind::EngineMissing);
        let missing = absent.missing.expect("an absent engine is described");
        assert_eq!(missing.tool, "bsl-analyzer");
        assert_eq!(
            missing.code,
            unica_artifact_delivery::engine::BUNDLED_TOOL_MISSING
        );
        cleanup_context(&context);
    }

    /// The real runner tells a process that never started from one that did.
    #[test]
    fn streaming_runner_reports_a_spawn_failure_as_such() {
        let failure = system_process_runner()
            .run_streaming_staged(
                &ProcessCommand {
                    program: std::env::temp_dir().join("unica-1338-no-such-engine"),
                    args: Vec::new(),
                    cwd: std::env::temp_dir(),
                    env: Vec::new(),
                    env_remove: Vec::new(),
                    capture_limits: None,
                    timeout: None,
                    cancellation: CancellationToken::new(),
                },
                1024,
                &mut |_, _| StreamControl::Continue,
            )
            .expect_err("an absent program cannot start");
        assert!(
            matches!(failure, ProcessStreamFailure::Spawn(_)),
            "{failure:?}"
        );
    }

    /// Ревью #549: рабочий каталог, которого нет, — не незапуск программы.
    /// Оба пути запуска раннера отвечают на него сбоем после старта, а не
    /// `Spawn`, которым маршрут назвал бы отказ «нет движка».
    #[test]
    fn a_missing_working_directory_is_not_a_failed_launch() {
        let command = ProcessCommand {
            program: std::env::current_exe().unwrap(),
            args: Vec::new(),
            cwd: std::env::temp_dir().join("unica-549-no-such-directory"),
            env: Vec::new(),
            env_remove: Vec::new(),
            capture_limits: None,
            timeout: None,
            cancellation: CancellationToken::new(),
        };
        let staged = system_process_runner()
            .run_staged(&command)
            .expect_err("a missing working directory cannot run");
        assert!(matches!(staged, ProcessStreamFailure::Run(_)), "{staged:?}");
        let streaming = system_process_runner()
            .run_streaming_staged(&command, 1024, &mut |_, _| StreamControl::Continue)
            .expect_err("a missing working directory cannot run");
        assert!(
            matches!(streaming, ProcessStreamFailure::Run(_)),
            "{streaming:?}"
        );
        let controlled = system_process_runner()
            .run_controlled_streaming_staged(&command, 1024, &mut |_, _| ProcessLineReply::Continue)
            .expect_err("a missing working directory cannot run");
        assert!(
            matches!(controlled, ProcessStreamFailure::Run(_)),
            "{controlled:?}"
        );
        let handoff = match system_process_runner().run_pending_handoff_staged(&command) {
            Ok(_) => panic!("a missing working directory cannot run"),
            Err(failure) => failure,
        };
        assert!(
            matches!(handoff, ProcessStreamFailure::Run(_)),
            "{handoff:?}"
        );
    }

    #[test]
    fn controlled_streaming_refuses_a_runner_without_live_pipe_support() {
        struct CapturedOnlyRunner;
        impl ProcessRunner for CapturedOnlyRunner {
            fn run(&self, _: &ProcessCommand) -> Result<ProcessOutput, String> {
                panic!("controlled streaming must never replay captured output")
            }
        }
        let command = ProcessCommand {
            program: PathBuf::from("unused-captured-runner"),
            args: Vec::new(),
            cwd: std::env::temp_dir(),
            env: Vec::new(),
            env_remove: Vec::new(),
            capture_limits: None,
            timeout: None,
            cancellation: CancellationToken::new(),
        };
        let failure = CapturedOnlyRunner
            .run_controlled_streaming_staged(&command, 1024, &mut |_, _| {
                panic!("unsupported runner must not invoke the live callback")
            })
            .expect_err("captured-only runner has no live decisions");
        assert!(matches!(failure, ProcessStreamFailure::Run(_)));
    }

    #[test]
    fn diagnostics_analyze_read_failure_is_safe_and_distinct_from_oversize() {
        struct ReadFailureRunner;
        impl ProcessRunner for ReadFailureRunner {
            fn run(&self, _: &ProcessCommand) -> Result<ProcessOutput, String> {
                panic!("streaming runner expected")
            }

            fn run_streaming(
                &self,
                _: &ProcessCommand,
                _: usize,
                on_line: &mut dyn FnMut(usize, &[u8]) -> StreamControl,
            ) -> Result<ProcessStreamOutput, String> {
                on_line(1, br#"{"type":"start","total_files":0,"version":"test"}"#);
                Ok(ProcessStreamOutput {
                    status_success: true,
                    status: "exit status: 0".to_string(),
                    stderr: String::new(),
                    timed_out: false,
                    cancelled: false,
                    stopped_by_consumer: false,
                    line_error: Some((
                        2,
                        "failed to read process stdout: private-token C:/private/Secret.bsl"
                            .to_string(),
                    )),
                })
            }
        }
        let context = temp_context("diagnostics-read-failure");
        let result = invoke_analyze(
            &BslAnalyzerMcpAdapter::with_process_runner(
                &ReadFailureRunner,
                crate::infrastructure::analysis_composition::environment(),
            ),
            &Map::new(),
            &context,
            false,
        );
        cleanup_context(&context);
        assert!(!result.outcome.ok);
        assert!(result.outcome.stdout.is_none());
        let error = result.diagnostics.unwrap().outcome.error.unwrap();
        assert_eq!(error.code, "diagnostics_invalid");
        assert_eq!(error.message, "line 2: failed to read diagnostics stream. Check compatibility between Unica and its bundled analyzer; if the problem persists, report this reason and line number.");
        assert_eq!(
            result.outcome.errors,
            vec![format!("diagnostics_invalid {}", error.message)]
        );
    }

    #[test]
    fn diagnostics_analyze_process_failure_takes_priority_over_invalid_stream() {
        for (index, (cancelled, timed_out, expected)) in [
            (true, false, "cancelled:"),
            (false, true, "timed out"),
            (false, false, "exit status: 2"),
        ]
        .into_iter()
        .enumerate()
        {
            let context = temp_context(&format!("diagnostics-process-priority-{index}"));
            let mut output = analyze_process_output("invalid-event");
            output.status_success = false;
            output.cancelled = cancelled;
            output.timed_out = timed_out;
            output.status = "exit status: 2".to_string();
            let runner = FakeProcessRunner { output };
            let result = invoke_analyze(
                &BslAnalyzerMcpAdapter::with_process_runner(
                    &runner,
                    crate::infrastructure::analysis_composition::environment(),
                ),
                &Map::new(),
                &context,
                false,
            );
            cleanup_context(&context);
            assert!(!result.outcome.ok);
            assert!(result.diagnostics.is_none());
            assert!(result.outcome.stdout.is_none());
            assert!(
                result
                    .outcome
                    .errors
                    .iter()
                    .any(|error| error.contains(expected)),
                "{:?}",
                result.outcome.errors
            );
            assert!(!result
                .outcome
                .errors
                .iter()
                .any(|error| error.contains("diagnostics_invalid")));
        }
    }

    #[test]
    fn cli_adapter_rejects_raw_args_vector() {
        let context = discover_workspace(Some(std::env::current_dir().unwrap())).unwrap();
        let mut args = Map::new();
        args.insert("args".to_string(), json!(["--unsafe", "../outside"]));

        let error = CliAdapter::new("v8-runner", &["build"], "build/runtime")
            .invoke("unica.build.load", &args, &context, true, true)
            .unwrap_err();

        assert!(error.contains("raw args are not accepted"));
    }

    #[test]
    fn cli_adapter_redacts_secret_values_from_reported_command() {
        let context = discover_workspace(Some(std::env::current_dir().unwrap())).unwrap();
        let mut args = Map::new();
        args.insert("dbPassword".to_string(), json!("super-secret"));
        args.insert("apiToken".to_string(), json!("token-secret"));

        let outcome = CliAdapter::new("v8-runner", &["build"], "build/runtime")
            .invoke("unica.build.load", &args, &context, true, true)
            .unwrap();

        let command = outcome.command.unwrap().join(" ");
        assert!(command.contains("--db-password <redacted>"));
        assert!(command.contains("--api-token <redacted>"));
        assert!(!command.contains("super-secret"));
        assert!(!command.contains("token-secret"));
    }

    #[test]
    fn runtime_adapter_redacts_connection_string_from_reported_command() {
        let context = discover_workspace(Some(std::env::current_dir().unwrap())).unwrap();
        let mut args = Map::new();
        args.insert("operation".to_string(), json!("config-init"));
        // Credentials in the connection string are refused outright (#343),
        // so the redaction case uses a connection the mapper accepts. What it
        // proves is unchanged: the connection value never reaches the reported
        // command, whatever it holds.
        args.insert(
            "connection".to_string(),
            json!("Srvr=prod-secret-host;Ref=ib"),
        );

        let outcome =
            RuntimeAdapter::new(crate::infrastructure::runtime_composition::runtime_environment())
                .invoke("unica.runtime.execute", &args, &context, true, true)
                .unwrap();

        let command = outcome.command.unwrap().join(" ");
        assert!(command.contains("--connection <redacted>"));
        assert!(!command.contains("prod-secret-host"));
    }

    #[test]
    fn cli_adapter_uses_fake_process_runner_for_status_and_output_contract() {
        let context = temp_context("cli-fake-runner-status");
        let runner = FakeProcessRunner {
            output: ProcessOutput {
                status_success: false,
                status: "exit status: 2".to_string(),
                stdout: "partial stdout".to_string(),
                stderr: "failure stderr".to_string(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
                stderr_truncated: false,
                stdout_had_invalid_utf8: false,
                stderr_had_invalid_utf8: false,
            },
        };

        let outcome = CliAdapter::with_runner("v8-runner", &["build"], "build/runtime", &runner)
            .invoke("unica.build.load", &Map::new(), &context, false, true)
            .unwrap();

        assert!(!outcome.ok);
        assert_eq!(outcome.stdout.as_deref(), Some("partial stdout"));
        assert_eq!(outcome.stderr.as_deref(), Some("failure stderr"));
        assert!(outcome.errors.contains(&"failure stderr".to_string()));
        assert!(outcome
            .warnings
            .iter()
            .any(|warning| warning.contains("exit status: 2")));
        cleanup_context(&context);
    }

    #[test]
    fn cancellation_prefix_is_stable_for_pre_cancelled_adapter_call() {
        let context = temp_context("cli-pre-cancelled");
        let runner = FakeProcessRunner {
            output: ProcessOutput {
                status_success: true,
                status: "exit status: 0".to_string(),
                stdout: String::new(),
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
                stderr_truncated: false,
                stdout_had_invalid_utf8: false,
                stderr_had_invalid_utf8: false,
            },
        };
        let cancellation = CancellationToken::new();
        cancellation.cancel();

        let outcome = CliAdapter::with_runner("v8-runner", &["build"], "build/runtime", &runner)
            .invoke_cancellable(
                "unica.build.load",
                &Map::new(),
                &context,
                false,
                true,
                &cancellation,
            )
            .unwrap();

        assert!(outcome.errors[0].starts_with("cancelled:"));
        cleanup_context(&context);
    }

    #[test]
    fn cancellation_prefix_is_stable_for_cancelled_cli_output() {
        let context = temp_context("cli-cancelled-output");
        let runner = FakeProcessRunner {
            output: ProcessOutput {
                status_success: false,
                status: "cancelled".to_string(),
                stdout: String::new(),
                stderr: String::new(),
                timed_out: false,
                cancelled: true,
                stdout_truncated: false,
                stderr_truncated: false,
                stdout_had_invalid_utf8: false,
                stderr_had_invalid_utf8: false,
            },
        };

        let outcome = CliAdapter::with_runner("v8-runner", &["build"], "build/runtime", &runner)
            .invoke("unica.build.load", &Map::new(), &context, false, true)
            .unwrap();

        assert!(outcome.errors[0].starts_with("cancelled:"));
        cleanup_context(&context);
    }

    #[test]
    fn cancellation_prefix_is_stable_for_cancelled_runtime_output() {
        let mut context = temp_context("runtime-cancelled-output");
        configure_designer_source(&mut context);
        let runner = FakeProcessRunner {
            output: ProcessOutput {
                status_success: false,
                status: "cancelled".to_string(),
                stdout: String::new(),
                stderr: String::new(),
                timed_out: false,
                cancelled: true,
                stdout_truncated: false,
                stderr_truncated: false,
                stdout_had_invalid_utf8: false,
                stderr_had_invalid_utf8: false,
            },
        };
        let mut args = Map::new();
        args.insert("operation".to_string(), json!("build"));

        let outcome = RuntimeAdapter::with_runner(
            &runner,
            crate::infrastructure::runtime_composition::runtime_environment(),
        )
        .invoke("unica.runtime.execute", &args, &context, false, true)
        .unwrap();

        assert!(outcome.errors[0].starts_with("cancelled:"));
        cleanup_context(&context);
    }

    #[test]
    fn cli_adapter_records_default_process_timeout() {
        let context = temp_context("cli-timeout-record");
        let runner = RecordingProcessRunner {
            commands: RefCell::new(Vec::new()),
            output: ProcessOutput {
                status_success: true,
                status: "exit status: 0".to_string(),
                stdout: String::new(),
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
                stderr_truncated: false,
                stdout_had_invalid_utf8: false,
                stderr_had_invalid_utf8: false,
            },
        };

        let outcome = CliAdapter::with_runner("v8-runner", &["build"], "build/runtime", &runner)
            .invoke("unica.build.load", &Map::new(), &context, false, true)
            .unwrap();

        assert!(outcome.ok);
        assert_eq!(
            runner.commands.borrow()[0].timeout,
            Some(DEFAULT_PROCESS_TIMEOUT)
        );
        assert!(runner.commands.borrow()[0]
            .program
            .to_string_lossy()
            .contains("bin/"));
        cleanup_context(&context);
    }

    #[test]
    fn cli_adapter_reports_fake_process_timeout() {
        let context = temp_context("cli-fake-timeout");
        let runner = FakeProcessRunner {
            output: ProcessOutput {
                status_success: false,
                status: "timeout".to_string(),
                stdout: String::new(),
                stderr: String::new(),
                timed_out: true,
                cancelled: false,
                stdout_truncated: false,
                stderr_truncated: false,
                stdout_had_invalid_utf8: false,
                stderr_had_invalid_utf8: false,
            },
        };

        let outcome = CliAdapter::with_runner("v8-runner", &["build"], "build/runtime", &runner)
            .invoke("unica.build.load", &Map::new(), &context, false, true)
            .unwrap();

        assert!(!outcome.ok);
        assert!(outcome
            .warnings
            .iter()
            .any(|warning| warning.contains("timed out")));
        assert!(outcome
            .errors
            .iter()
            .any(|error| error.contains("timed out after")));
        cleanup_context(&context);
    }

    #[test]
    fn unrelated_cli_timeout_with_stderr_keeps_existing_reporting() {
        let context = temp_context("cli-timeout-existing-reporting");
        let runner = FakeProcessRunner {
            output: ProcessOutput {
                status_success: false,
                status: "timeout".to_string(),
                stdout: String::new(),
                stderr: "runtime timeout details".to_string(),
                timed_out: true,
                cancelled: false,
                stdout_truncated: false,
                stderr_truncated: false,
                stdout_had_invalid_utf8: false,
                stderr_had_invalid_utf8: false,
            },
        };

        let outcome = CliAdapter::with_runner("v8-runner", &["build"], "build/runtime", &runner)
            .invoke("unica.build.load", &Map::new(), &context, false, true)
            .unwrap();

        assert_eq!(
            outcome.warnings,
            vec!["internal build/runtime adapter timed out"]
        );
        assert_eq!(outcome.errors, vec!["runtime timeout details"]);
        cleanup_context(&context);
    }

    #[test]
    fn runtime_adapter_does_not_report_wrapper_timeout_seconds_without_local_timeout() {
        let mut context = temp_context("runtime-timeout-no-local-budget");
        configure_designer_source(&mut context);
        let runner = FakeProcessRunner {
            output: ProcessOutput {
                status_success: false,
                status: "timeout".to_string(),
                stdout: String::new(),
                stderr: String::new(),
                timed_out: true,
                cancelled: false,
                stdout_truncated: false,
                stderr_truncated: false,
                stdout_had_invalid_utf8: false,
                stderr_had_invalid_utf8: false,
            },
        };
        let mut args = Map::new();
        args.insert("operation".to_string(), json!("build"));

        let outcome = RuntimeAdapter::with_runner(
            &runner,
            crate::infrastructure::runtime_composition::runtime_environment(),
        )
        .invoke("unica.runtime.execute", &args, &context, false, true)
        .unwrap();

        assert!(!outcome.ok);
        assert!(outcome
            .errors
            .iter()
            .any(|error| error == "internal v8-runner runtime adapter timed out"));
        assert!(outcome.errors.iter().all(|error| !error.contains("120")));
        cleanup_context(&context);
    }

    #[test]
    fn runtime_adapter_redacts_non_zero_process_output() {
        let mut context = temp_context("runtime-non-zero-diagnostics");
        configure_designer_source(&mut context);
        let runner = FakeProcessRunner {
            output: ProcessOutput {
                status_success: false,
                status: "exit status: 1".to_string(),
                stdout:
                    "prelude that should not matter\nstarted build\nUsr=admin;Pwd=stdout-secret\n"
                        .to_string(),
                stderr: "failed to load configuration: Pwd=stderr-secret\n".to_string(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
                stderr_truncated: false,
                stdout_had_invalid_utf8: false,
                stderr_had_invalid_utf8: false,
            },
        };
        let mut args = Map::new();
        args.insert("operation".to_string(), json!("build"));
        args.insert("sourceSet".to_string(), json!("main"));

        let outcome = RuntimeAdapter::with_runner(
            &runner,
            crate::infrastructure::runtime_composition::runtime_environment(),
        )
        .invoke("unica.runtime.execute", &args, &context, false, true)
        .unwrap();

        assert!(!outcome.ok);
        assert!(outcome
            .command
            .as_ref()
            .unwrap()
            .join(" ")
            .contains("build --source-set main"));
        assert!(outcome.stdout.as_deref().unwrap().contains("started build"));
        assert!(outcome
            .stderr
            .as_deref()
            .unwrap()
            .contains("failed to load configuration"));
        let serialized = serde_json::to_string(&outcome).unwrap();
        assert!(!serialized.contains("stdout-secret"));
        assert!(!serialized.contains("stderr-secret"));
        cleanup_context(&context);
    }

    #[test]
    fn runtime_adapter_reports_timeout_failure_without_wrapper_budget() {
        let context = temp_context("runtime-timeout-diagnostics");
        let runner = FakeProcessRunner {
            output: ProcessOutput {
                status_success: false,
                status: "timeout".to_string(),
                stdout: "started loading configuration...\n".to_string(),
                stderr: String::new(),
                timed_out: true,
                cancelled: false,
                stdout_truncated: false,
                stderr_truncated: false,
                stdout_had_invalid_utf8: false,
                stderr_had_invalid_utf8: false,
            },
        };
        let mut args = Map::new();
        args.insert("operation".to_string(), json!("load"));
        args.insert("path".to_string(), json!("build/config.cf"));

        let outcome = RuntimeAdapter::with_runner(
            &runner,
            crate::infrastructure::runtime_composition::runtime_environment(),
        )
        .invoke("unica.runtime.execute", &args, &context, false, true)
        .unwrap();

        assert!(!outcome.ok);
        assert!(outcome
            .warnings
            .iter()
            .any(|warning| warning.contains("timed out")));
        assert!(outcome
            .stdout
            .as_deref()
            .unwrap()
            .contains("started loading configuration"));
        assert!(outcome.errors.iter().all(|error| !error.contains("120")));
        cleanup_context(&context);
    }

    #[test]
    fn runtime_adapter_returns_failure_outcome_for_spawn_failure() {
        let mut context = temp_context("runtime-spawn-failure-diagnostics");
        configure_designer_source(&mut context);
        let runner = FailingProcessRunner {
            error: "failed to execute process: no such file or directory; apiToken=token-secret"
                .to_string(),
        };
        let mut args = Map::new();
        args.insert("operation".to_string(), json!("build"));

        let outcome = RuntimeAdapter::with_runner(
            &runner,
            crate::infrastructure::runtime_composition::runtime_environment(),
        )
        .invoke("unica.runtime.execute", &args, &context, false, true)
        .unwrap();

        assert!(!outcome.ok);
        assert!(outcome.summary.contains("failed"));
        assert!(outcome
            .errors
            .iter()
            .any(|error| error.contains("failed to execute process")));
        assert!(!serde_json::to_string(&outcome)
            .unwrap()
            .contains("token-secret"));
        cleanup_context(&context);
    }

    #[test]
    fn production_secret_redaction_surfaces_are_closed() {
        assert_eq!(
            unica_redaction::production_secret_key_matrix(),
            ["connection", "pwd", "password", "token", "secret"]
        );
        assert_eq!(
            unica_adapter_v8_runner::request::production_runtime_secret_flags(),
            ["password", "pwd", "token", "secret", "connection", "c"]
        );
        assert_eq!(
            unica_adapter_v8_runner::request::production_runtime_connection_markers(),
            ["file=", "srvr=", "ref=", "usr=", "pwd=", "dbsrvr=", "dbname="]
        );

        cli_adapter_redacts_secret_values_from_reported_command();
        runtime_adapter_redacts_connection_string_from_reported_command();
        runtime_adapter_redacts_non_zero_process_output();
        runtime_adapter_returns_failure_outcome_for_spawn_failure();
        crate::infrastructure::runtime_jobs_integration::tests::production_secret_key_matrix_is_redacted_from_runtime_surfaces();
        crate::infrastructure::runtime_jobs_integration::tests::terminal_snapshot_and_persistence_are_redacted_and_keep_log_artifacts();
        crate::infrastructure::runtime_jobs_integration::tests::persisted_command_redacts_a_launch_connection_string_completely();
        crate::infrastructure::runtime_jobs_integration::tests::worker_handoff_never_persists_actual_argv_or_output_secrets();
        crate::infrastructure::runtime_jobs_integration::tests::direct_status_rejects_corrupt_unknown_schema_and_non_uuid_without_touching_active_lock();
        crate::infrastructure::runtime_jobs_integration::tests::list_skips_a_corrupt_record_and_redacts_its_warning();
        crate::infrastructure::operational_config::tests::diagnostics_never_expose_absolute_paths_raw_toml_or_values();
        crate::infrastructure::operational_config::tests::read_errors_are_redacted_to_the_fixed_basename();
    }

    #[test]
    #[ignore = "helper process invoked by system_process_runner_drains_large_stderr_while_running"]
    fn system_process_runner_large_stderr_helper() {
        let chunk = [b'e'; 64 * 1024];
        let mut stderr = std::io::stderr().lock();
        for _ in 0..64 {
            stderr.write_all(&chunk).unwrap();
        }
        stderr.flush().unwrap();
        print!("large-stderr-complete");
        std::io::stdout().flush().unwrap();
    }

    #[test]
    #[ignore = "helper process invoked by system_process_runner_drains_large_stdout_while_running"]
    fn system_process_runner_large_stdout_helper() {
        let chunk = [b'o'; 64 * 1024];
        let mut stdout = std::io::stdout().lock();
        for _ in 0..64 {
            stdout.write_all(&chunk).unwrap();
        }
        stdout.write_all(b"large-stdout-complete").unwrap();
        stdout.flush().unwrap();
    }

    #[test]
    fn system_process_runner_drains_large_stdout_while_running() {
        let output = system_process_runner()
            .run(&ProcessCommand {
                program: std::env::current_exe().unwrap(),
                args: vec![
                    "--ignored".to_string(),
                    "--exact".to_string(),
                    "infrastructure::internal_adapters::tests::system_process_runner_large_stdout_helper"
                        .to_string(),
                    "--nocapture".to_string(),
                ],
                cwd: std::env::current_dir().unwrap(),
                env: Vec::new(),
                env_remove: Vec::new(),
                capture_limits: None,
                timeout: Some(Duration::from_secs(10)),
                cancellation: CancellationToken::new(),
            })
            .unwrap();

        assert!(
            !output.timed_out,
            "runner timed out after capturing {} stdout bytes",
            output.stdout.len()
        );
        assert!(
            !output.cancelled,
            "runner unexpectedly reported cancellation"
        );
        assert!(
            process_exit_code_is(&output.status, 0),
            "helper must exit successfully, got {}",
            output.status
        );
        assert!(
            !output.status_success,
            "truncated stdout must not be reported as parseable success"
        );
        assert!(
            output.stdout.contains("large-stdout-complete"),
            "expected bounded stdout tail to contain completion marker"
        );
        assert!(
            output.stderr.contains("stdout capture truncated"),
            "expected structured truncation diagnostic, got {:?}",
            output.stderr
        );
    }

    #[test]
    fn system_process_runner_honors_larger_bounded_capture_limit() {
        let output = system_process_runner()
            .run(&ProcessCommand {
                program: std::env::current_exe().unwrap(),
                args: vec![
                    "--ignored".to_string(),
                    "--exact".to_string(),
                    "infrastructure::internal_adapters::tests::system_process_runner_large_stdout_helper"
                        .to_string(),
                    "--nocapture".to_string(),
                ],
                cwd: std::env::current_dir().unwrap(),
                env: Vec::new(),
                env_remove: Vec::new(),
                capture_limits: Some((8 * 1024 * 1024, 256 * 1024)),
                timeout: Some(Duration::from_secs(10)),
                cancellation: CancellationToken::new(),
            })
            .unwrap();

        assert!(output.status_success, "{output:?}");
        assert!(!output.stdout_truncated);
        assert!(output.stdout.len() > 1024 * 1024);
        assert!(output.stdout.contains("large-stdout-complete"));
    }

    #[test]
    fn system_process_runner_drains_large_stderr_while_running() {
        let output = system_process_runner()
            .run(&ProcessCommand {
                program: std::env::current_exe().unwrap(),
                args: vec![
                    "--ignored".to_string(),
                    "--exact".to_string(),
                    "infrastructure::internal_adapters::tests::system_process_runner_large_stderr_helper"
                        .to_string(),
                    "--nocapture".to_string(),
                ],
                cwd: std::env::current_dir().unwrap(),
                env: Vec::new(),
                env_remove: Vec::new(),
                capture_limits: None,
                timeout: Some(Duration::from_secs(10)),
                cancellation: CancellationToken::new(),
            })
            .unwrap();

        assert!(
            !output.timed_out,
            "runner timed out after capturing {} stderr bytes",
            output.stderr.len()
        );
        assert!(output.status_success, "status was {}", output.status);
        assert!(
            output.stdout.contains("large-stderr-complete"),
            "{}",
            output.stdout
        );
        assert!(
            output.stderr.contains("earlier stderr diagnostics omitted"),
            "expected bounded stderr diagnostic, got {} bytes",
            output.stderr.len()
        );
    }

    #[test]
    fn system_process_runner_does_not_timeout_when_timeout_is_none() {
        let command = unica_process::test_support::command_writing_stdout("ok");

        let output = system_process_runner()
            .run(&ProcessCommand {
                program: command.program,
                args: command.args,
                cwd: std::env::current_dir().unwrap(),
                env: Vec::new(),
                env_remove: Vec::new(),
                capture_limits: None,
                timeout: None,
                cancellation: CancellationToken::new(),
            })
            .unwrap();

        assert!(output.status_success);
        assert_eq!(output.stdout, "ok");
        assert!(!output.timed_out);
    }

    #[test]
    fn cancelled_runner_stops_process_without_reporting_timeout() {
        let command = unica_process::test_support::long_running_command();
        let token = unica_work_control::cancellation::CancellationToken::new();
        token.cancel();

        let output = system_process_runner()
            .run(&ProcessCommand {
                program: command.program,
                args: command.args,
                cwd: std::env::current_dir().unwrap(),
                env: Vec::new(),
                env_remove: Vec::new(),
                capture_limits: None,
                timeout: Some(Duration::from_secs(10)),
                cancellation: token,
            })
            .unwrap();

        assert!(output.cancelled);
        assert!(!output.timed_out);
    }

    struct FakeProcessRunner {
        output: ProcessOutput,
    }

    impl ProcessRunner for FakeProcessRunner {
        fn run(&self, _command: &ProcessCommand) -> Result<ProcessOutput, String> {
            Ok(self.output.clone())
        }
    }

    #[test]
    fn default_process_runner_rejects_nonempty_stdin_without_running_command() {
        let runner = FakeProcessRunner {
            output: ProcessOutput {
                status_success: true,
                status: "exit status: 0".to_string(),
                stdout: String::new(),
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
                stderr_truncated: false,
                stdout_had_invalid_utf8: false,
                stderr_had_invalid_utf8: false,
            },
        };
        let command = ProcessCommand {
            program: PathBuf::from("unused"),
            args: Vec::new(),
            cwd: PathBuf::from("."),
            env: Vec::new(),
            env_remove: Vec::new(),
            capture_limits: None,
            timeout: None,
            cancellation: CancellationToken::new(),
        };

        let error = runner.run_with_input(&command, b"nonempty").unwrap_err();

        assert_eq!(
            error,
            "process_failed: process runner does not support stdin"
        );
    }

    struct FailingProcessRunner {
        error: String,
    }

    impl ProcessRunner for FailingProcessRunner {
        fn run(&self, _command: &ProcessCommand) -> Result<ProcessOutput, String> {
            Err(self.error.clone())
        }
    }

    struct RecordingProcessRunner {
        commands: RefCell<Vec<ProcessCommand>>,
        output: ProcessOutput,
    }

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<OsString>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: impl AsRef<OsStr>) -> Self {
            let previous = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(previous) = &self.previous {
                std::env::set_var(self.key, previous);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    impl ProcessRunner for RecordingProcessRunner {
        fn run(&self, command: &ProcessCommand) -> Result<ProcessOutput, String> {
            self.commands.borrow_mut().push(command.clone());
            Ok(self.output.clone())
        }
    }

    struct SequenceProcessRunner {
        commands: RefCell<Vec<ProcessCommand>>,
        outputs: RefCell<Vec<ProcessOutput>>,
    }
    fn build_failure_json(mode: Value, message: &str) -> String {
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
                "code": "platform_failure",
                "kind": "platform",
                "message": message
            }
        }))
        .expect("serialize build failure")
    }
    impl ProcessRunner for SequenceProcessRunner {
        fn run(&self, command: &ProcessCommand) -> Result<ProcessOutput, String> {
            self.commands.borrow_mut().push(command.clone());
            Ok(self.outputs.borrow_mut().remove(0))
        }
    }
    struct RecordingBslMcpRunner {
        commands: RefCell<Vec<BslMcpCommand>>,
        output: BslMcpOutput,
    }

    impl BslMcpRunner for RecordingBslMcpRunner {
        fn call(&self, command: &BslMcpCommand) -> Result<BslMcpOutput, String> {
            self.commands.borrow_mut().push(command.clone());
            Ok(self.output.clone())
        }
    }

    fn temp_context(name: &str) -> WorkspaceContext {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("unica-code-search-{name}-{nanos}"));
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("v8project.yaml"),
            "source-set:\n  - name: main\n    type: CONFIGURATION\n    path: .\n",
        )
        .unwrap();
        create_fake_plugin_root(&root);
        let mut context =
            discover_workspace(Some(root.clone())).expect("discover temporary workspace");
        context.cache_root = root.join(".build").join("unica");
        context
    }

    fn assert_runtime_args_with_primary(
        context: &WorkspaceContext,
        actual: &[String],
        expected_tail: &[&str],
    ) {
        let actual = if actual.first().map(String::as_str) == Some("--json-message") {
            &actual[1..]
        } else {
            actual
        };
        assert_eq!(actual.first().map(String::as_str), Some("--config"));
        let expected_config = unica_workspace::paths::strip_windows_extended_length_prefix(
            &context
                .workspace_root
                .join("v8project.yaml")
                .canonicalize()
                .expect("canonical primary runtime config"),
        );
        assert_eq!(
            actual.get(1).map(Path::new),
            Some(expected_config.as_path())
        );
        assert_eq!(
            actual.get(2..).expect("runtime command after config"),
            expected_tail
        );
    }

    fn configure_designer_source(context: &mut WorkspaceContext) {
        let source_root = context.workspace_root.join("src");
        fs::create_dir_all(&source_root).unwrap();
        fs::write(
            context.workspace_root.join("v8project.yaml"),
            concat!(
                "format: DESIGNER\n",
                "source-set:\n",
                "  - name: main\n",
                "    type: CONFIGURATION\n",
                "    path: src\n",
            ),
        )
        .unwrap();
        fs::write(
            source_root.join("Configuration.xml"),
            include_bytes!(
                "../../../../tests/fixtures/platform_8_3_27/support-edit-bin-only/src/Configuration.xml"
            ),
        )
        .unwrap();
        refresh_test_context(context);
    }

    fn configure_supported_designer_source(context: &mut WorkspaceContext) {
        configure_designer_source(context);
        let marker = context
            .workspace_root
            .join("src/Ext/ParentConfigurations.bin");
        fs::create_dir_all(marker.parent().unwrap()).unwrap();
        fs::write(
            marker,
            include_bytes!(
                "../../../../tests/fixtures/platform_8_3_27/support-edit-bin-only/src/Ext/ParentConfigurations.bin"
            ),
        )
        .unwrap();
        refresh_test_context(context);
    }

    fn refresh_test_context(context: &mut WorkspaceContext) {
        let cache_root = context.cache_root.clone();
        let mut current = discover_workspace(Some(context.cwd.clone()))
            .expect("rediscover configured test workspace");
        current.cache_root = cache_root;
        *context = current;
    }

    /// Rewrites the fake manifest without `tool_name`, the way a checkout that
    /// has not run the tools download looks.
    fn drop_tool_from_fake_manifest(root: &Path, tool_name: &str) {
        let manifest_path = root
            .join("plugins")
            .join("unica")
            .join("third-party")
            .join("manifest.json");
        let mut manifest: Value =
            serde_json::from_str(&fs::read_to_string(&manifest_path).unwrap()).unwrap();
        let tools = manifest["tools"].as_array_mut().unwrap();
        tools.retain(|tool| tool["name"] != json!(tool_name));
        fs::write(&manifest_path, serde_json::to_string(&manifest).unwrap()).unwrap();
    }

    fn remove_fake_tool_binaries(context: &WorkspaceContext, tool_name: &str) {
        let plugin_root = context.cwd.join("plugins/unica");
        for target in ["darwin-arm64", "linux-x64", "win-x64"] {
            let suffix = if target == "win-x64" { ".exe" } else { "" };
            let _ = fs::remove_file(
                plugin_root
                    .join("bin")
                    .join(target)
                    .join(format!("{tool_name}{suffix}")),
            );
        }
    }

    fn create_fake_plugin_root(root: &Path) {
        let plugin_root = root.join("plugins").join("unica");
        fs::create_dir_all(plugin_root.join("skills")).unwrap();
        fs::create_dir_all(plugin_root.join("third-party")).unwrap();
        for target in ["darwin-arm64", "linux-x64"] {
            fs::create_dir_all(plugin_root.join("bin").join(target)).unwrap();
            fs::write(
                plugin_root.join("bin").join(target).join("v8-runner"),
                "v8-runner",
            )
            .unwrap();
            fs::write(
                plugin_root.join("bin").join(target).join("bsl-analyzer"),
                "bsl-analyzer",
            )
            .unwrap();
            fs::write(
                plugin_root.join("bin").join(target).join("rlm-bsl-index"),
                "rlm-index",
            )
            .unwrap();
        }
        fs::create_dir_all(plugin_root.join("bin/win-x64")).unwrap();
        fs::write(
            plugin_root.join("bin/win-x64").join("v8-runner.exe"),
            "v8-runner",
        )
        .unwrap();
        fs::write(
            plugin_root.join("bin/win-x64").join("bsl-analyzer.exe"),
            "bsl-analyzer",
        )
        .unwrap();
        fs::write(
            plugin_root.join("bin/win-x64").join("rlm-bsl-index.exe"),
            "rlm-index",
        )
        .unwrap();
        fs::write(
            plugin_root.join("third-party/manifest.json"),
            r#"{
  "schemaVersion": 2,
  "tools": [
    {
      "name": "bsl-analyzer",
      "binaries": {
        "darwin-arm64": {"targetTriple": "aarch64-apple-darwin", "binaryPath": "bin/darwin-arm64/bsl-analyzer", "sha256": "e5121f9edee6abec4a7a34a3953521d89edb1cb14b871ea63a26f52d5697b05a"},
        "linux-x64": {"targetTriple": "x86_64-unknown-linux-gnu", "binaryPath": "bin/linux-x64/bsl-analyzer", "sha256": "e5121f9edee6abec4a7a34a3953521d89edb1cb14b871ea63a26f52d5697b05a"},
        "win-x64": {"targetTriple": "x86_64-pc-windows-msvc", "binaryPath": "bin/win-x64/bsl-analyzer.exe", "sha256": "e5121f9edee6abec4a7a34a3953521d89edb1cb14b871ea63a26f52d5697b05a"}
      }
    },
    {
      "name": "rlm-bsl-index",
      "binaries": {
        "darwin-arm64": {"targetTriple": "aarch64-apple-darwin", "binaryPath": "bin/darwin-arm64/rlm-bsl-index", "sha256": "fa6a77fa531fa57e7781010a7cec69b7be4b7b58903365153bf1f66e851ab213"},
        "linux-x64": {"targetTriple": "x86_64-unknown-linux-gnu", "binaryPath": "bin/linux-x64/rlm-bsl-index", "sha256": "fa6a77fa531fa57e7781010a7cec69b7be4b7b58903365153bf1f66e851ab213"},
        "win-x64": {"targetTriple": "x86_64-pc-windows-msvc", "binaryPath": "bin/win-x64/rlm-bsl-index.exe", "sha256": "fa6a77fa531fa57e7781010a7cec69b7be4b7b58903365153bf1f66e851ab213"}
      }
    },
    {
      "name": "v8-runner",
      "binaries": {
        "darwin-arm64": {"targetTriple": "aarch64-apple-darwin", "binaryPath": "bin/darwin-arm64/v8-runner", "sha256": "da3d869003da0bfb858de1160b3b1a7b92dee2374889909ee252cfd51a79e415"},
        "linux-x64": {"targetTriple": "x86_64-unknown-linux-gnu", "binaryPath": "bin/linux-x64/v8-runner", "sha256": "da3d869003da0bfb858de1160b3b1a7b92dee2374889909ee252cfd51a79e415"},
        "win-x64": {"targetTriple": "x86_64-pc-windows-msvc", "binaryPath": "bin/win-x64/v8-runner.exe", "sha256": "da3d869003da0bfb858de1160b3b1a7b92dee2374889909ee252cfd51a79e415"}
      }
    }
  ]
}"#,
        )
        .unwrap();
    }

    fn cleanup_context(context: &WorkspaceContext) {
        let _ = fs::remove_dir_all(&context.workspace_root);
    }
}
#[cfg(test)]
mod dry_run_honesty_tests {
    use super::*;
    use unica_artifact_delivery::engine::InstallMode;
    use unica_artifact_delivery::engine::MissingEngine;

    fn tool(missing: Option<MissingEngine>) -> BundledTool {
        BundledTool {
            program: PathBuf::from("/plugins/unica/bin/darwin-arm64/v8-runner"),
            warnings: Vec::new(),
            missing,
        }
    }

    #[test]
    fn a_preview_without_the_binary_is_not_a_runnable_plan() {
        // #549: `ok` и `command` вместе выглядели готовым к запуску планом,
        // хотя файла по этому пути нет. Худший из возможных ответов.
        let outcome = dry_run_outcome(
            "unica.build.load",
            "build/runtime",
            true,
            tool(Some(MissingEngine::new(
                "v8-runner",
                "darwin-arm64",
                "/plugins/unica/bin/darwin-arm64/v8-runner",
                Some("0.5.1".to_string()),
                InstallMode::Source,
            ))),
            vec!["/plugins/unica/bin/darwin-arm64/v8-runner".to_string()],
        );

        assert!(!outcome.ok, "план без бинаря не выдаёт себя за исполнимый");
        assert!(
            outcome
                .errors
                .iter()
                .any(|error| error.contains(unica_artifact_delivery::engine::BUNDLED_TOOL_MISSING)),
            "отказ назван кодом: {:?}",
            outcome.errors
        );
        assert!(
            outcome.summary.contains("v8-runner"),
            "чего не хватает, видно в сводке: {}",
            outcome.summary
        );
    }

    #[test]
    fn a_preview_with_the_binary_in_place_still_describes_the_plan() {
        let outcome = dry_run_outcome(
            "unica.build.load",
            "build/runtime",
            true,
            tool(None),
            vec!["/plugins/unica/bin/darwin-arm64/v8-runner".to_string()],
        );

        assert!(outcome.ok);
        assert!(outcome.errors.is_empty());
        assert!(outcome.command.is_some());
        assert_eq!(
            outcome.changes,
            vec!["no files changed because dryRun is true".to_string()]
        );
    }
}
