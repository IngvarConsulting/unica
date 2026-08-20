use crate::application::{
    AdapterOutcome, RuntimeJobAction, DIAGNOSTICS_ANALYZE_TIMEOUT_MAX_SECONDS,
    DIAGNOSTICS_ANALYZE_TIMEOUT_MIN_SECONDS,
};
use crate::domain::cancellation::{CancellationToken, CANCELLED_PREFIX};
use crate::domain::operational_config::OperationalConfig;
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::bundled_tools::resolve_bundled_tool;
use crate::infrastructure::code_intelligence::is_provider_unavailable_error;
use crate::infrastructure::diagnostics_jsonl::{
    AnalyzerDiagnosticsBatch, DiagnosticsJsonlParser, MAX_DIAGNOSTICS_JSONL_LINE_BYTES,
};
use crate::infrastructure::platform::filesystem::path_lock_identity;
use crate::infrastructure::platform::{
    ensure_truncation_diagnostics, ManagedChild, ManagedCommand, ManagedLineOutput, ManagedOutput,
    StreamControl,
};
use crate::infrastructure::plugin_runtime::{find_plugin_root, value_to_cli_string};
use crate::infrastructure::redaction::{is_secret_key, redactor};
use crate::infrastructure::runtime_build_preflight::{
    RuntimeBuildPreflight, RuntimeInvocationPlan,
};
use crate::infrastructure::runtime_jobs::{
    self, RuntimeJobOperation, RuntimeJobRequest, RuntimeJobService,
};
use crate::infrastructure::source_roots::normalize_path_identity;
use crate::infrastructure::source_roots::resolve_source_root;
use crate::infrastructure::workspace::discover_workspace;
use crate::infrastructure::workspace_services::WorkspaceServiceManager;
use serde_json::{json, Map, Value};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

const DEFAULT_PROCESS_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone)]
pub struct ProcessCommand {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(OsString, OsString)>,
    pub env_remove: Vec<OsString>,
    pub capture_limits: Option<(usize, usize)>,
    pub timeout: Option<Duration>,
    pub cancellation: CancellationToken,
}

#[derive(Debug, Clone)]
pub struct ProcessOutput {
    pub status_success: bool,
    pub status: String,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
    pub cancelled: bool,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub stdout_had_invalid_utf8: bool,
    pub stderr_had_invalid_utf8: bool,
}

#[derive(Debug, Clone)]
pub struct ProcessStreamOutput {
    pub status_success: bool,
    pub status: String,
    pub stderr: String,
    pub timed_out: bool,
    pub cancelled: bool,
    pub stopped_by_consumer: bool,
    pub line_error: Option<(usize, String)>,
}

pub trait ProcessRunner {
    fn run(&self, command: &ProcessCommand) -> Result<ProcessOutput, String>;

    fn run_with_input(
        &self,
        command: &ProcessCommand,
        input: &[u8],
    ) -> Result<ProcessOutput, String> {
        if input.is_empty() {
            self.run(command)
        } else {
            Err("process_failed: process runner does not support stdin".to_string())
        }
    }

    fn run_streaming(
        &self,
        command: &ProcessCommand,
        max_line_bytes: usize,
        on_line: &mut dyn FnMut(usize, &[u8]) -> StreamControl,
    ) -> Result<ProcessStreamOutput, String> {
        let output = self.run(command)?;
        let mut line_error = None;
        let logical_line_count = output.stdout.lines().count();
        for (index, bytes) in output
            .stdout
            .as_bytes()
            .split(|byte| *byte == b'\n')
            .enumerate()
        {
            if bytes.is_empty() && index + 1 == logical_line_count + 1 {
                continue;
            }
            if bytes.len() > max_line_bytes {
                line_error.get_or_insert_with(|| {
                    (index + 1, "line exceeds configured byte limit".to_string())
                });
            } else {
                if on_line(index + 1, bytes) == StreamControl::Stop {
                    return Ok(ProcessStreamOutput {
                        status_success: false,
                        status: "stopped by consumer".to_string(),
                        stderr: output.stderr,
                        timed_out: false,
                        cancelled: output.cancelled,
                        stopped_by_consumer: true,
                        line_error,
                    });
                }
            }
        }
        Ok(ProcessStreamOutput {
            status_success: output.status_success,
            status: output.status,
            stderr: output.stderr,
            timed_out: output.timed_out,
            cancelled: output.cancelled,
            stopped_by_consumer: false,
            line_error,
        })
    }
}

#[derive(Debug, Clone)]
pub struct BslMcpCommand {
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub source_dir: PathBuf,
    pub timeout: Duration,
    pub tool_name: &'static str,
    pub tool_args: Value,
    pub cancellation: CancellationToken,
}

#[derive(Debug, Clone)]
pub struct BslMcpOutput {
    pub result_text: String,
    pub stderr: String,
}

pub trait BslMcpRunner {
    fn call(&self, command: &BslMcpCommand) -> Result<BslMcpOutput, String>;
}

struct SystemProcessRunner;
struct SystemBslMcpRunner;

static SYSTEM_PROCESS_RUNNER: SystemProcessRunner = SystemProcessRunner;
static SYSTEM_BSL_MCP_RUNNER: SystemBslMcpRunner = SystemBslMcpRunner;

pub(crate) fn system_process_runner() -> &'static (dyn ProcessRunner + Send + Sync) {
    &SYSTEM_PROCESS_RUNNER
}

pub struct CliAdapter<'a> {
    tool_name: &'static str,
    default_command: &'static [&'static str],
    label: &'static str,
    runner: &'a dyn ProcessRunner,
    process_timeout: Duration,
}

pub struct RuntimeAdapter<'a> {
    runner: &'a dyn ProcessRunner,
}

pub struct RuntimeAdapterOutcome {
    pub outcome: AdapterOutcome,
    pub data: Option<Value>,
}

impl RuntimeAdapterOutcome {
    fn plain(outcome: AdapterOutcome) -> Self {
        Self {
            outcome,
            data: None,
        }
    }
}

pub struct RuntimeJobAdapterOutcome {
    pub outcome: AdapterOutcome,
    pub job: Option<Value>,
}

pub struct RuntimeJobAdapter;

pub(crate) struct RuntimeInvocation<'a> {
    pub(crate) tool_name: &'a str,
    pub(crate) args: &'a Map<String, Value>,
    pub(crate) context: &'a WorkspaceContext,
    pub(crate) dry_run: bool,
    pub(crate) mutating: bool,
}
pub struct BslAnalyzerMcpAdapter<'a> {
    runner: &'a dyn BslMcpRunner,
    process_runner: &'a dyn ProcessRunner,
}

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
            runner: &SYSTEM_PROCESS_RUNNER,
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
            return Ok(AdapterOutcome {
                ok: true,
                summary: format!(
                    "dry run: {tool_name} would call internal {} adapter",
                    self.label
                ),
                changes: if mutating {
                    vec!["no files changed because dryRun is true".to_string()]
                } else {
                    Vec::new()
                },
                warnings: bundled_tool.warnings,
                errors: Vec::new(),
                artifacts: Vec::new(),
                stdout: None,
                stderr: None,
                command: Some(command),
            });
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

fn plan_runtime_invocation(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<RuntimeInvocationPlan, String> {
    if args.get("operation").and_then(Value::as_str) == Some("build") {
        validate_runtime_mapper_payload("build", args)?;
    }
    crate::infrastructure::runtime_build_preflight::plan_runtime_invocation(args, context)
}
fn merge_warnings(mut left: Vec<String>, right: Vec<String>) -> Vec<String> {
    left.extend(right);
    left
}
impl<'a> RuntimeAdapter<'a> {
    pub fn new() -> Self {
        Self {
            runner: &SYSTEM_PROCESS_RUNNER,
        }
    }

    #[cfg(test)]
    pub fn with_runner(runner: &'a dyn ProcessRunner) -> Self {
        Self { runner }
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
        self.invoke_with_data(tool_name, args, context, dry_run, mutating)
            .map(|outcome| outcome.outcome)
    }

    pub fn invoke_with_data(
        &self,
        tool_name: &str,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
        dry_run: bool,
        mutating: bool,
    ) -> Result<RuntimeAdapterOutcome, String> {
        self.invoke_cancellable_with_data(
            tool_name,
            args,
            context,
            dry_run,
            mutating,
            &CancellationToken::new(),
        )
    }

    pub fn invoke_cancellable_with_data(
        &self,
        tool_name: &str,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
        dry_run: bool,
        mutating: bool,
        cancellation: &CancellationToken,
    ) -> Result<RuntimeAdapterOutcome, String> {
        self.invoke_cancellable(
            RuntimeInvocation {
                tool_name,
                args,
                context,
                dry_run,
                mutating,
            },
            cancellation,
        )
    }

    pub(crate) fn invoke_cancellable(
        &self,
        invocation: RuntimeInvocation<'_>,
        cancellation: &CancellationToken,
    ) -> Result<RuntimeAdapterOutcome, String> {
        let RuntimeInvocation {
            tool_name,
            args,
            context,
            dry_run,
            mutating,
        } = invocation;
        if cancellation.is_cancelled() {
            return Ok(RuntimeAdapterOutcome::plain(AdapterOutcome::cancelled(
                format!("{tool_name} cancelled before adapter work"),
            )));
        }
        let invocation = plan_runtime_invocation(args, context)?;
        reject_missing_client_mcp_extension(&invocation.args, context)?;
        if let Some(outcome) = bind_external_processor_config(args, context, dry_run)? {
            return Ok(RuntimeAdapterOutcome::plain(outcome));
        }
        let plugin_root = find_plugin_root(&context.cwd).ok_or_else(|| {
            "could not locate Unica plugin root for internal adapter lookup".to_string()
        })?;
        if cancellation.is_cancelled() {
            return Ok(RuntimeAdapterOutcome::plain(AdapterOutcome::cancelled(
                format!("{tool_name} cancelled during runtime build preflight"),
            )));
        }
        let report_args = runtime_args(&invocation.args, true)?;
        let execution_args = runtime_args(&invocation.args, false)?;
        validate_bounded_external_epf_artifact_paths(&invocation.args, &context.cwd)?;
        let bundled_tool = resolve_bundled_tool(&plugin_root, "v8-runner", !dry_run)?;
        let mut command = vec![bundled_tool.program.display().to_string()];
        command.extend(report_args.iter().cloned());

        if dry_run {
            return Ok(RuntimeAdapterOutcome::plain(AdapterOutcome {
                ok: true,
                summary: format!(
                    "dry run: {tool_name} would call internal v8-runner runtime adapter"
                ),
                changes: if mutating {
                    vec!["no files changed because dryRun is true".to_string()]
                } else {
                    Vec::new()
                },
                warnings: merge_warnings(invocation.warnings, bundled_tool.warnings),
                errors: Vec::new(),
                artifacts: Vec::new(),
                stdout: None,
                stderr: None,
                command: Some(command),
            }));
        }

        if cancellation.is_cancelled() {
            return Ok(RuntimeAdapterOutcome::plain(AdapterOutcome::cancelled(
                format!("{tool_name} cancelled before v8-runner launch"),
            )));
        }

        let process_timeout = None;
        let process_command = ProcessCommand {
            program: bundled_tool.program.clone(),
            args: execution_args,
            cwd: context.cwd.clone(),
            env: Vec::new(),
            env_remove: Vec::new(),
            capture_limits: None,
            timeout: process_timeout,
            cancellation: cancellation.clone(),
        };
        let runtime_warnings = invocation.warnings;
        let output = match self.runner.run(&process_command) {
            Ok(output) => output,
            Err(error) => {
                let error = redactor(&error);
                let mut warnings = runtime_warnings;
                warnings
                    .push("internal v8-runner runtime adapter failed to spawn process".to_string());
                return Ok(RuntimeAdapterOutcome::plain(AdapterOutcome {
                    ok: false,
                    summary: format!(
                        "{tool_name} failed through internal v8-runner runtime adapter"
                    ),
                    changes: Vec::new(),
                    warnings,
                    errors: vec![error.clone()],
                    artifacts: Vec::new(),
                    stdout: None,
                    stderr: Some(format!("{error}\n")),
                    command: Some(command),
                }));
            }
        };
        // The one full retry belongs to the durable entry point alone. An
        // applied `unica.runtime.execute` is refused by INV-MCP-RUNTIME-RECEIPT
        // before it reaches this adapter, so there is no first attempt here to
        // classify and nothing to retry (ADR-0066, ADR-0067).
        let mut runner_error = if args.get("operation").and_then(Value::as_str) == Some("build")
            && !output.status_success
        {
            parse_runner_json_envelope(&output.stdout)
                .ok()
                .and_then(|envelope| runner_error_message(&envelope).map(redactor))
        } else {
            None
        };
        let mut ok = output.status_success;
        let stdout = redactor(&output.stdout);
        let stderr = redactor(&output.stderr);
        if output.cancelled {
            let mut outcome = cancelled_process_outcome(tool_name, stdout, stderr, Some(command));
            outcome.warnings = merge_warnings(runtime_warnings, outcome.warnings);
            return Ok(RuntimeAdapterOutcome::plain(outcome));
        }
        let waited_epf = args
            .get("waitForExit")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let mut artifacts = Vec::new();
        let mut wait_error = None;
        let mut data = None;
        if waited_epf {
            match parse_runner_json_envelope(&output.stdout) {
                Ok(envelope) => {
                    runner_error = runner_error_message(&envelope).map(redactor);
                    match parse_external_epf_wait(&envelope) {
                        Ok(Some(wait)) => {
                            match validate_external_epf_wait_receipt(&wait, args, &context.cwd) {
                                Ok(()) => {
                                    artifacts.extend(requested_external_epf_artifacts(args));
                                    data = Some(json!({
                                        "external_epf_wait": external_epf_wait_value(&wait)
                                    }));
                                    if wait.timed_out {
                                        ok = false;
                                        wait_error = Some(
                                            "bounded external EPF launch timed out".to_string(),
                                        );
                                    } else if wait.exit_code != Some(0) {
                                        ok = false;
                                        wait_error = Some(match wait.exit_code {
                                            Some(code) => {
                                                format!(
                                                    "bounded external EPF exited with code {code}"
                                                )
                                            }
                                            None => "bounded external EPF exit code is missing"
                                                .to_string(),
                                        });
                                    }
                                }
                                Err(error) => {
                                    ok = false;
                                    wait_error = Some(error);
                                }
                            }
                        }
                        Ok(None) if output.status_success => {
                            ok = false;
                            wait_error = Some(
                                "bounded external EPF runner result is missing `data.external_epf_wait`"
                                    .to_string(),
                            );
                        }
                        Ok(None) => {}
                        Err(error) => {
                            ok = false;
                            wait_error = Some(error);
                        }
                    }
                }
                Err(error) if output.status_success => {
                    ok = false;
                    wait_error = Some(error);
                }
                Err(_) => {}
            }
        }
        let wait_failed = wait_error.is_some();
        Ok(RuntimeAdapterOutcome {
            outcome: AdapterOutcome {
                ok,
                summary: if ok {
                    format!("{tool_name} completed through internal v8-runner runtime adapter")
                } else {
                    format!("{tool_name} failed through internal v8-runner runtime adapter")
                },
                changes: if mutating && ok {
                    vec!["internal v8-runner runtime adapter executed".to_string()]
                } else {
                    Vec::new()
                },
                warnings: if ok || wait_failed {
                    runtime_warnings
                } else if output.timed_out {
                    merge_warnings(
                        runtime_warnings,
                        vec!["internal v8-runner runtime adapter timed out".to_string()],
                    )
                } else {
                    merge_warnings(
                        runtime_warnings,
                        vec![format!(
                            "internal v8-runner runtime adapter exited with status {}",
                            output.status
                        )],
                    )
                },
                errors: if let Some(error) = wait_error {
                    vec![error]
                } else if ok {
                    Vec::new()
                } else if let Some(error) = runner_error {
                    vec![error]
                } else if stderr.trim().is_empty() && output.timed_out {
                    vec![process_timeout_error("v8-runner runtime", process_timeout)]
                } else if stderr.trim().is_empty() {
                    vec![format!(
                        "internal v8-runner runtime adapter exited with status {}",
                        output.status
                    )]
                } else {
                    vec![stderr.trim().to_string()]
                },
                artifacts,
                stdout: Some(stdout),
                stderr: Some(stderr),
                command: Some(command),
            },
            data,
        })
    }
}

fn validate_bounded_external_epf_artifact_paths(
    args: &Map<String, Value>,
    cwd: &Path,
) -> Result<(), String> {
    if args.get("waitForExit").and_then(Value::as_bool) != Some(true) {
        return Ok(());
    }
    // The mapper rejects identical path strings; this resolved host identity check also
    // catches aliases such as relative/absolute, symlinked, and case-only spellings.
    let resolve = |key: &str| {
        args.get(key)
            .and_then(Value::as_str)
            .ok_or_else(|| format!("bounded external EPF launch requires `{key}`"))
            .and_then(|path| runtime_path_identity(path, cwd))
    };
    if resolve("output")? == resolve("stderrOutput")? {
        return Err(
            "bounded external EPF `output` and `stderrOutput` resolve to the same path".to_string(),
        );
    }
    Ok(())
}

fn runtime_path_identity(path: &str, cwd: &Path) -> Result<String, String> {
    let path = Path::new(path);
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    normalize_path_identity(&candidate).map(|path| path_lock_identity(&path))
}

struct ExternalEpfWait {
    pid: u64,
    execute_path: String,
    exit_code: Option<i64>,
    timed_out: bool,
    output_path: String,
    stderr_path: String,
}

fn parse_runner_json_envelope(stdout: &str) -> Result<Value, String> {
    serde_json::from_str(stdout).map_err(|error| {
        format!("bounded external EPF runner returned invalid JSON result: {error}")
    })
}

fn runner_error_message(envelope: &Value) -> Option<&str> {
    envelope
        .pointer("/error/message")
        .and_then(Value::as_str)
        .or_else(|| envelope.pointer("/data/message").and_then(Value::as_str))
}

fn parse_external_epf_wait(envelope: &Value) -> Result<Option<ExternalEpfWait>, String> {
    let Some(wait) = envelope.pointer("/data/external_epf_wait") else {
        return Ok(None);
    };
    let wait = wait.as_object().ok_or_else(|| {
        "bounded external EPF runner result has invalid `data.external_epf_wait`".to_string()
    })?;
    let path = |key: &str| {
        wait.get(key)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .ok_or_else(|| format!("bounded external EPF runner result is missing `{key}`"))
    };
    if !wait.contains_key("exit_code") {
        return Err("bounded external EPF runner result is missing `exit_code`".to_string());
    }
    Ok(Some(ExternalEpfWait {
        pid: wait
            .get("pid")
            .and_then(Value::as_u64)
            .filter(|pid| *pid > 0)
            .ok_or_else(|| {
                "bounded external EPF runner result is missing positive `pid`".to_string()
            })?,
        execute_path: path("execute_path")?,
        exit_code: wait.get("exit_code").and_then(Value::as_i64),
        timed_out: wait
            .get("timed_out")
            .and_then(Value::as_bool)
            .ok_or_else(|| {
                "bounded external EPF runner result is missing `timed_out`".to_string()
            })?,
        output_path: path("output_path")?,
        stderr_path: path("stderr_path")?,
    }))
}

fn validate_external_epf_wait_receipt(
    wait: &ExternalEpfWait,
    args: &Map<String, Value>,
    cwd: &Path,
) -> Result<(), String> {
    for (receipt_key, returned, argument_key) in [
        ("execute_path", wait.execute_path.as_str(), "execute"),
        ("output_path", wait.output_path.as_str(), "output"),
        ("stderr_path", wait.stderr_path.as_str(), "stderrOutput"),
    ] {
        let requested = args
            .get(argument_key)
            .and_then(Value::as_str)
            .ok_or_else(|| format!("bounded external EPF launch requires `{argument_key}`"))?;
        if runtime_path_identity(returned, cwd)? != runtime_path_identity(requested, cwd)? {
            return Err(format!(
                "bounded external EPF receipt `{receipt_key}` does not match requested `{argument_key}`"
            ));
        }
    }
    Ok(())
}

fn requested_external_epf_artifacts(args: &Map<String, Value>) -> [String; 2] {
    ["output", "stderrOutput"].map(|key| {
        redactor(
            args.get(key)
                .and_then(Value::as_str)
                .expect("bounded external EPF arguments were validated"),
        )
    })
}

fn external_epf_wait_value(wait: &ExternalEpfWait) -> Value {
    json!({
        "pid": wait.pid,
        "execute_path": redactor(&wait.execute_path),
        "exit_code": wait.exit_code,
        "timed_out": wait.timed_out,
        "output_path": redactor(&wait.output_path),
        "stderr_path": redactor(&wait.stderr_path),
    })
}

impl<'a> Default for RuntimeAdapter<'a> {
    fn default() -> Self {
        Self::new()
    }
}

fn bind_external_processor_config(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    dry_run: bool,
) -> Result<Option<AdapterOutcome>, String> {
    if args.get("operation").and_then(Value::as_str) != Some("config-init")
        || !args.contains_key("sourceSet")
    {
        return Ok(None);
    }
    validate_runtime_mapper_payload("config-init", args)?;
    for key in ["format", "builder", "force"] {
        if args.contains_key(key) {
            return Err(format!(
                "operation `config-init` with `sourceSet` does not accept `{key}`"
            ));
        }
    }
    let source_set_name = required_non_empty_runtime_string(args, "sourceSet")?;
    let connection = required_non_empty_runtime_string(args, "connection")?;
    let config_arg = required_non_empty_runtime_string(args, "config")?;
    let unresolved_config = context.cwd.join(config_arg);
    let config_path = unresolved_config.canonicalize().map_err(|error| {
        format!(
            "external source-set bind requires an existing config `{}`: {error}",
            unresolved_config.display()
        )
    })?;
    let workspace_root = context.workspace_root.canonicalize().map_err(|error| {
        format!(
            "failed to resolve workspace root `{}`: {error}",
            context.workspace_root.display()
        )
    })?;
    if !config_path.starts_with(&workspace_root) {
        return Err(format!(
            "external source-set config `{}` is outside workspace root `{}`",
            config_path.display(),
            workspace_root.display()
        ));
    }
    let config_text = std::fs::read_to_string(&config_path)
        .map_err(|error| format!("failed to read {}: {error}", config_path.display()))?;
    let config: serde_yaml::Value = serde_yaml::from_str(&config_text)
        .map_err(|error| format!("failed to parse {}: {error}", config_path.display()))?;
    validate_external_processor_source_set(&config, source_set_name, &config_path)?;

    let local_path = config_path
        .parent()
        .expect("canonical config path has a parent")
        .join("v8project.local.yaml");
    if local_path.exists() {
        return Err(format!(
            "external source-set bind refuses to overwrite existing local overlay `{}`",
            local_path.display()
        ));
    }
    let mut infobase = serde_yaml::Mapping::new();
    infobase.insert(
        serde_yaml::Value::String("connection".to_string()),
        serde_yaml::Value::String(connection.to_string()),
    );
    let mut overlay = serde_yaml::Mapping::new();
    overlay.insert(
        serde_yaml::Value::String("infobase".to_string()),
        serde_yaml::Value::Mapping(infobase),
    );
    let overlay_text = serde_yaml::to_string(&overlay)
        .map_err(|error| format!("failed to serialize local runtime config: {error}"))?;

    if dry_run {
        return Ok(Some(AdapterOutcome {
            ok: true,
            summary: "dry run: unica.runtime.execute would bind an external processor source-set to a local infobase".to_string(),
            changes: vec!["no files changed because dryRun is true".to_string()],
            warnings: Vec::new(),
            errors: Vec::new(),
            artifacts: vec![local_path.display().to_string()],
            stdout: None,
            stderr: None,
            command: None,
        }));
    }

    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    let mut file = options.open(&local_path).map_err(|error| {
        format!(
            "failed to create local runtime config `{}`: {error}",
            local_path.display()
        )
    })?;
    use std::io::Write as _;
    file.write_all(overlay_text.as_bytes()).map_err(|error| {
        format!(
            "failed to write local runtime config `{}`: {error}",
            local_path.display()
        )
    })?;

    Ok(Some(AdapterOutcome {
        ok: true,
        summary: "unica.runtime.execute bound an external processor source-set to a local infobase"
            .to_string(),
        changes: vec![format!("created {}", local_path.display())],
        warnings: Vec::new(),
        errors: Vec::new(),
        artifacts: vec![local_path.display().to_string()],
        stdout: None,
        stderr: None,
        command: None,
    }))
}

fn required_non_empty_runtime_string<'a>(
    args: &'a Map<String, Value>,
    key: &str,
) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            format!("operation `config-init` with `sourceSet` requires non-empty `{key}`")
        })
}

fn validate_external_processor_source_set(
    config: &serde_yaml::Value,
    selected_name: &str,
    config_path: &Path,
) -> Result<(), String> {
    let source_sets = config
        .as_mapping()
        .and_then(|mapping| mapping.get(serde_yaml::Value::String("source-set".to_string())))
        .ok_or_else(|| format!("{} has no `source-set`", config_path.display()))?;
    let mut matches = Vec::new();
    match source_sets {
        serde_yaml::Value::Sequence(entries) => {
            for entry in entries {
                let Some(mapping) = entry.as_mapping() else {
                    continue;
                };
                if yaml_mapping_string(mapping, "name") == Some(selected_name) {
                    matches.push(mapping);
                }
            }
        }
        serde_yaml::Value::Mapping(entries) => {
            if let Some(entry) = entries.get(serde_yaml::Value::String(selected_name.to_string())) {
                if let Some(mapping) = entry.as_mapping() {
                    matches.push(mapping);
                }
            }
        }
        _ => {
            return Err(format!(
                "{} field `source-set` must be a list or mapping",
                config_path.display()
            ));
        }
    }
    if matches.len() != 1 {
        return Err(format!(
            "{} must contain exactly one source-set named `{selected_name}`",
            config_path.display()
        ));
    }
    let source_set = matches[0];
    if yaml_mapping_string(source_set, "type") != Some("EXTERNAL_DATA_PROCESSORS") {
        return Err(format!(
            "source-set `{selected_name}` must have type `EXTERNAL_DATA_PROCESSORS`"
        ));
    }
    if yaml_mapping_string(source_set, "path").is_none_or(|path| path.trim().is_empty()) {
        return Err(format!(
            "source-set `{selected_name}` must have a non-empty `path`"
        ));
    }
    Ok(())
}

fn yaml_mapping_string<'a>(mapping: &'a serde_yaml::Mapping, key: &str) -> Option<&'a str> {
    mapping
        .get(serde_yaml::Value::String(key.to_string()))
        .and_then(serde_yaml::Value::as_str)
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
        let invocation = plan_runtime_invocation(args, context)?;
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
        let warnings = merge_warnings(invocation.warnings, bundled_tool.warnings);

        if dry_run {
            return Ok(RuntimeJobAdapterOutcome {
                outcome: AdapterOutcome {
                    ok: true,
                    summary: format!("dry run: {tool_name} would start a durable runtime job"),
                    changes: vec!["no runtime job started because dryRun is true".to_string()],
                    warnings,
                    errors: Vec::new(),
                    artifacts: Vec::new(),
                    stdout: None,
                    stderr: None,
                    command: Some(command),
                },
                job: None,
            });
        }

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
        match runtime_jobs::start_detached_worker(
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

fn format_section(name: &str, text: &str) -> String {
    let body = text.trim_end();
    if body.is_empty() {
        format!("=== {name} ===")
    } else {
        format!("=== {name} ===\n{body}")
    }
}

fn cancelled_process_outcome(
    tool_name: &str,
    stdout: String,
    stderr: String,
    command: Option<Vec<String>>,
) -> AdapterOutcome {
    let mut outcome = AdapterOutcome::cancelled(format!("{tool_name} process stopped"));
    outcome.stdout = Some(stdout);
    outcome.stderr = Some(stderr);
    outcome.command = command;
    outcome
}

fn process_timeout_error(label: &str, timeout: Option<Duration>) -> String {
    match timeout {
        Some(timeout) => format!(
            "internal {label} adapter timed out after {} seconds",
            timeout.as_secs()
        ),
        None => format!("internal {label} adapter timed out"),
    }
}

#[cfg(test)]
fn process_exit_code_is(status: &str, code: i32) -> bool {
    let status = status.trim();
    status == code.to_string() || status.ends_with(&format!(": {code}"))
}

impl<'a> BslAnalyzerMcpAdapter<'a> {
    pub fn new() -> Self {
        Self {
            runner: &SYSTEM_BSL_MCP_RUNNER,
            process_runner: &SYSTEM_PROCESS_RUNNER,
        }
    }

    #[cfg(test)]
    pub fn with_runner(runner: &'a dyn BslMcpRunner) -> Self {
        Self {
            runner,
            process_runner: &SYSTEM_PROCESS_RUNNER,
        }
    }

    #[cfg(test)]
    pub fn with_process_runner(process_runner: &'a dyn ProcessRunner) -> Self {
        Self {
            runner: &SYSTEM_BSL_MCP_RUNNER,
            process_runner,
        }
    }

    pub(crate) fn analyze_diagnostic_batch(
        &self,
        context: &WorkspaceContext,
        source_root: &Path,
        timeout: Duration,
        cancellation: &CancellationToken,
    ) -> Result<AnalyzerDiagnosticsBatch, String> {
        let mut args = Map::new();
        args.insert(
            "sourceDir".to_string(),
            Value::String(source_root.display().to_string()),
        );
        if let Some(timeout_seconds) = analyzer_analyze_timeout_seconds(timeout) {
            args.insert("timeoutSeconds".to_string(), json!(timeout_seconds));
        }
        let result = self.invoke_diagnostics_analyze(
            "unica.code.diagnostics",
            &args,
            context,
            false,
            None,
            cancellation,
        )?;
        result.diagnostics.ok_or_else(|| {
            result
                .outcome
                .errors
                .first()
                .cloned()
                .unwrap_or(result.outcome.summary)
        })
    }

    #[allow(dead_code)]
    pub fn invoke(
        &self,
        tool_name: &str,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
        dry_run: bool,
    ) -> Result<BslAnalyzerOutcome, String> {
        self.invoke_cancellable(tool_name, args, context, dry_run, &CancellationToken::new())
    }

    pub fn invoke_cancellable(
        &self,
        tool_name: &str,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
        dry_run: bool,
        cancellation: &CancellationToken,
    ) -> Result<BslAnalyzerOutcome, String> {
        self.invoke_cancellable_with_operational_config(
            tool_name,
            args,
            context,
            dry_run,
            None,
            cancellation,
        )
    }

    pub(crate) fn invoke_cancellable_with_operational_config(
        &self,
        tool_name: &str,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
        dry_run: bool,
        _operational_config: Option<&OperationalConfig>,
        cancellation: &CancellationToken,
    ) -> Result<BslAnalyzerOutcome, String> {
        if cancellation.is_cancelled() {
            return Ok(BslAnalyzerOutcome::plain(AdapterOutcome::cancelled(
                format!("{tool_name} cancelled before adapter work"),
            )));
        }
        let plugin_root = match find_plugin_root(&context.cwd) {
            Some(plugin_root) => plugin_root,
            None => {
                return Ok(provider_unavailable_outcome(
                    tool_name,
                    "could not locate Unica plugin root for bsl-analyzer MCP adapter lookup"
                        .to_string(),
                ))
            }
        };
        let source_dir = resolve_source_dir(context, args)?;
        let (remote_tool, tool_args) = bsl_mcp_tool_request(tool_name, args)?;
        // A workspace that has not downloaded the tools has no analyzer in its
        // manifest. `code.search` already answers that state with an
        // unavailable section and a working result (ADR-0017); answering it
        // here with a failed call made the same workstation look broken (#275).
        // A provider that ran and failed is a different case and still fails.
        let bundled_tool = match resolve_bundled_tool(&plugin_root, "bsl-analyzer", !dry_run) {
            Ok(bundled_tool) => bundled_tool,
            Err(error) if is_provider_unavailable_error(&error) => {
                return Ok(provider_unavailable_outcome(tool_name, error))
            }
            Err(error) => return Err(error),
        };
        let command = bsl_mcp_command(
            &source_dir,
            context,
            remote_tool,
            tool_args,
            cancellation.clone(),
        );
        let mut reported_command = vec![bundled_tool.program.display().to_string()];
        reported_command.extend(command.args.clone());

        if dry_run {
            return Ok(BslAnalyzerOutcome::plain(AdapterOutcome {
                ok: true,
                summary: format!("dry run: {tool_name} would call typed bsl-analyzer MCP adapter"),
                changes: Vec::new(),
                warnings: bundled_tool.warnings,
                errors: Vec::new(),
                artifacts: vec![source_dir.display().to_string()],
                stdout: None,
                stderr: None,
                command: Some(reported_command),
            }));
        }

        let output = self.runner.call(&command)?;
        let section = "bsl-analyzer-graph";
        let readiness_warnings = bsl_mcp_readiness_warnings(&output.result_text);
        let summary = format!("{tool_name} completed through typed bsl-analyzer MCP adapter");
        // ADR-0023: the analyzer answers this tool with JSON, and wrapping that
        // JSON in a section header made the caller unwrap a string to reach it.
        // `analyze` is the analyzer MCP tool name, not a spawned 1C process, so
        // `code.diagnostics` returns the same kind of reply as `code.graph` and
        // the ADR-0023 §4 carve-out for external process streams does not reach
        // it.
        // A reply that is not JSON must still reach the caller: failing here
        // dropped the analyzer text, its stderr and the reported command, so a
        // plain-text diagnostic became "unparsable reply" and nothing else.
        let mut parse_error = None;
        let data = match serde_json::from_str::<Value>(output.result_text.trim()) {
            Ok(value) => Some(value),
            Err(error) => {
                parse_error = Some(format!(
                    "{tool_name} received an unparsable bsl-analyzer reply: {error}"
                ));
                None
            }
        };
        let mut errors = Vec::new();
        if let Some(parse_error) = parse_error {
            errors.push(parse_error);
        }
        let unparsable = data.is_none();
        Ok(BslAnalyzerOutcome {
            outcome: AdapterOutcome {
                ok: !unparsable,
                summary,
                changes: Vec::new(),
                warnings: readiness_warnings,
                errors,
                artifacts: vec![
                    source_dir.display().to_string(),
                    command.tool_name.to_string(),
                ],
                // The raw reply survives only when it could not be typed;
                // ADR-0023 keeps `stdout` empty for a well-formed answer.
                stdout: data
                    .is_none()
                    .then(|| format_section(section, &output.result_text)),
                stderr: if output.stderr.trim().is_empty() {
                    None
                } else {
                    Some(output.stderr)
                },
                command: Some(reported_command),
            },
            data,
            diagnostics: None,
        })
    }

    fn invoke_diagnostics_analyze(
        &self,
        tool_name: &str,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
        dry_run: bool,
        operational_config: Option<&OperationalConfig>,
        cancellation: &CancellationToken,
    ) -> Result<BslAnalyzerOutcome, String> {
        let plugin_root = find_plugin_root(&context.cwd).ok_or_else(|| {
            "could not locate Unica plugin root for diagnostics adapter lookup".to_string()
        })?;
        let source_dir = resolve_source_dir(context, args)?;
        let normalized_args = diagnostics_analyze_args(args);
        let process_timeout = diagnostics_analyze_timeout(args, operational_config)?;
        let bundled_tool = resolve_bundled_tool(&plugin_root, "bsl-analyzer", !dry_run)?;
        let reported_args = cli_args(&normalized_args, true)?;
        let execution_args = cli_args(&normalized_args, false)?;
        let mut reported_command = vec![bundled_tool.program.display().to_string()];
        reported_command.push("analyze".to_string());
        reported_command.extend(reported_args);

        if dry_run {
            return Ok(BslAnalyzerOutcome::plain(AdapterOutcome {
                ok: true,
                summary: format!("dry run: {tool_name} would call internal code analysis adapter"),
                changes: Vec::new(),
                warnings: bundled_tool.warnings,
                errors: Vec::new(),
                artifacts: vec![source_dir.display().to_string()],
                stdout: None,
                stderr: None,
                command: Some(reported_command),
            }));
        }

        let mut process_args = vec!["analyze".to_string()];
        process_args.extend(execution_args);
        let mut parser = DiagnosticsJsonlParser::new(&source_dir)?;
        let mut consume = |line_number, bytes: &[u8]| {
            parser.push_line(line_number, bytes);
            StreamControl::Continue
        };
        let output = self.process_runner.run_streaming(
            &ProcessCommand {
                program: bundled_tool.program,
                args: process_args,
                cwd: context.cwd.clone(),
                env: Vec::new(),
                env_remove: Vec::new(),
                capture_limits: None,
                timeout: Some(process_timeout),
                cancellation: cancellation.clone(),
            },
            MAX_DIAGNOSTICS_JSONL_LINE_BYTES,
            &mut consume,
        )?;
        if let Some((line_number, reason)) = &output.line_error {
            parser.reject_line(*line_number, reason);
        }
        let stderr = redactor(&output.stderr);
        if output.cancelled {
            let mut outcome = AdapterOutcome::cancelled(format!("{tool_name} process stopped"));
            outcome.stderr = (!stderr.trim().is_empty()).then_some(stderr);
            outcome.command = Some(reported_command);
            return Ok(BslAnalyzerOutcome::plain(outcome));
        }
        if !output.status_success {
            let timeout_error = output
                .timed_out
                .then(|| process_timeout_error("code analysis", Some(process_timeout)));
            let mut errors = timeout_error.iter().cloned().collect::<Vec<_>>();
            if !stderr.trim().is_empty() {
                errors.push(stderr.trim().to_string());
            }
            if errors.is_empty() {
                errors.push(format!(
                    "internal code analysis adapter exited with status {}",
                    output.status
                ));
            }
            return Ok(BslAnalyzerOutcome::plain(AdapterOutcome {
                ok: false,
                summary: format!("{tool_name} failed through internal code analysis adapter"),
                changes: Vec::new(),
                warnings: if output.timed_out {
                    timeout_error.into_iter().collect()
                } else {
                    vec![format!(
                        "internal code analysis adapter exited with status {}",
                        output.status
                    )]
                },
                errors,
                artifacts: vec![source_dir.display().to_string()],
                stdout: None,
                stderr: (!stderr.trim().is_empty()).then_some(stderr),
                command: Some(reported_command),
            }));
        }

        let batch = parser.finish();
        let protocol_error = batch.outcome.error.as_ref();
        let outcome = AdapterOutcome {
            ok: protocol_error.is_none(),
            summary: if let Some(error) = protocol_error {
                format!(
                    "{tool_name} finished with {} {}",
                    error.code.trim_end_matches(':'),
                    if error.retryable {
                        "retryable state"
                    } else {
                        "protocol failure"
                    }
                )
            } else {
                format!("{tool_name} completed through typed JSONL diagnostics adapter")
            },
            changes: Vec::new(),
            warnings: bundled_tool.warnings,
            errors: protocol_error
                .map(|error| vec![format!("{} {}", error.code, error.message)])
                .unwrap_or_default(),
            artifacts: vec![source_dir.display().to_string()],
            stdout: None,
            stderr: (!stderr.trim().is_empty()).then_some(stderr),
            command: Some(reported_command),
        };
        Ok(BslAnalyzerOutcome {
            outcome,
            data: None,
            diagnostics: Some(batch),
        })
    }
}

/// An analyzer answer plus the typed payload, for the tools whose contract is
/// already the analyzer's own JSON.
#[derive(Debug)]
pub struct BslAnalyzerOutcome {
    pub outcome: AdapterOutcome,
    pub data: Option<Value>,
    pub(crate) diagnostics: Option<AnalyzerDiagnosticsBatch>,
}

impl BslAnalyzerOutcome {
    fn plain(outcome: AdapterOutcome) -> Self {
        Self {
            outcome,
            data: None,
            diagnostics: None,
        }
    }
}

/// The answer for a provider that is not present in this workspace: a normal
/// tool result that states the cause, not a failed call. The
/// `provider_unavailable:` prefix is the machine-readable part — it separates
/// "the analyzer is not here" from "the analyzer ran and failed", which the
/// caller has to tell apart to decide whether retrying is worth anything.
fn provider_unavailable_outcome(tool_name: &str, error: String) -> BslAnalyzerOutcome {
    BslAnalyzerOutcome::plain(AdapterOutcome {
        ok: false,
        summary: format!(
            "{tool_name} is unavailable: bsl-analyzer is not bundled in this workspace"
        ),
        changes: Vec::new(),
        warnings: Vec::new(),
        errors: vec![format!("provider_unavailable: {error}")],
        artifacts: Vec::new(),
        stdout: None,
        stderr: None,
        command: None,
    })
}

impl Default for BslAnalyzerMcpAdapter<'_> {
    fn default() -> Self {
        Self::new()
    }
}

fn required_string<'a>(args: &'a Map<String, Value>, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("missing required `{key}` argument"))
}

/// Caller budget the analyzer run is allowed to take, in whole seconds.
///
/// The remaining budget is always a fraction below the configured one because
/// the deadline starts before the provider worker does, so truncating would
/// push the documented minimum of 30 seconds out of the accepted band and drop
/// the caller budget entirely.
fn analyzer_analyze_timeout_seconds(timeout: Duration) -> Option<u64> {
    let seconds = timeout.as_secs() + u64::from(timeout.subsec_nanos() > 0);
    Some(seconds.clamp(
        DIAGNOSTICS_ANALYZE_TIMEOUT_MIN_SECONDS,
        DIAGNOSTICS_ANALYZE_TIMEOUT_MAX_SECONDS,
    ))
}

fn diagnostics_analyze_args(args: &Map<String, Value>) -> Map<String, Value> {
    let mut filtered = Map::new();
    for key in ["cwd", "confirm", "sourceDir", "config"] {
        if let Some(value) = args.get(key) {
            filtered.insert(key.to_string(), value.clone());
        }
    }
    filtered.insert("format".to_string(), json!("jsonl"));
    filtered
}

fn diagnostics_analyze_timeout(
    args: &Map<String, Value>,
    operational_config: Option<&OperationalConfig>,
) -> Result<Duration, String> {
    let Some(value) = args.get("timeoutSeconds") else {
        return Ok(
            operational_config.map_or(DEFAULT_PROCESS_TIMEOUT, |config| {
                config.code_diagnostics().analyze_timeout()
            }),
        );
    };
    let Some(seconds) = value.as_u64() else {
        return Err("unica.code.diagnostics argument `timeoutSeconds` must be integer".to_string());
    };
    if !(DIAGNOSTICS_ANALYZE_TIMEOUT_MIN_SECONDS..=DIAGNOSTICS_ANALYZE_TIMEOUT_MAX_SECONDS)
        .contains(&seconds)
    {
        return Err(format!(
            "unica.code.diagnostics argument `timeoutSeconds` must be between {} and {}",
            DIAGNOSTICS_ANALYZE_TIMEOUT_MIN_SECONDS, DIAGNOSTICS_ANALYZE_TIMEOUT_MAX_SECONDS
        ));
    }
    Ok(Duration::from_secs(seconds))
}

fn bsl_mcp_command(
    source_dir: &Path,
    context: &WorkspaceContext,
    remote_tool: &'static str,
    tool_args: Value,
    cancellation: CancellationToken,
) -> BslMcpCommand {
    BslMcpCommand {
        args: vec![
            "mcp".to_string(),
            "serve".to_string(),
            "--profile".to_string(),
            "workspace".to_string(),
            "--source-dir".to_string(),
            source_dir.display().to_string(),
            "--mode".to_string(),
            "stdio".to_string(),
        ],
        cwd: context.cwd.clone(),
        source_dir: source_dir.to_path_buf(),
        timeout: DEFAULT_PROCESS_TIMEOUT,
        tool_name: remote_tool,
        tool_args,
        cancellation,
    }
}

fn bsl_mcp_tool_request(
    tool_name: &str,
    args: &Map<String, Value>,
) -> Result<(&'static str, Value), String> {
    match tool_name {
        "unica.code.graph" => {
            let mode = required_string(args, "mode")?;
            let mut payload = Map::new();
            payload.insert("action".to_string(), json!(mode));
            copy_json_arg(&mut payload, args, "id", "id");
            copy_json_arg(&mut payload, args, "ids", "ids");
            copy_json_arg(&mut payload, args, "query", "query");
            copy_json_arg(&mut payload, args, "dir", "dir");
            copy_json_arg(&mut payload, args, "detail", "detail");
            copy_json_arg(&mut payload, args, "edgeKinds", "edge_kinds");
            copy_json_arg(&mut payload, args, "provenance", "provenance");
            copy_json_arg(&mut payload, args, "limit", "max_nodes");
            copy_json_arg(&mut payload, args, "maxOutputTokens", "max_output_tokens");
            Ok(("graph", Value::Object(payload)))
        }
        _ => Err(format!("unsupported bsl-analyzer MCP tool: {tool_name}")),
    }
}

fn copy_json_arg(
    payload: &mut Map<String, Value>,
    args: &Map<String, Value>,
    from: &str,
    to: &str,
) {
    if let Some(value) = args.get(from).filter(|value| !value.is_null()) {
        payload.insert(to.to_string(), value.clone());
    }
}

fn resolve_source_dir(
    context: &WorkspaceContext,
    args: &Map<String, Value>,
) -> Result<PathBuf, String> {
    resolve_source_root(context, args.get("sourceDir").and_then(Value::as_str))
        .map(|resolved| resolved.path)
}

fn bsl_mcp_readiness_warnings(text: &str) -> Vec<String> {
    if bsl_mcp_reply_is_pending(text) {
        vec![
            "bsl-analyzer workspace model is not ready yet; retry status or the request after reload completes"
                .to_string(),
        ]
    } else {
        Vec::new()
    }
}

const BSL_MCP_READINESS_STATES: &[(&str, &str)] = &[
    ("reload", "running"),
    ("state", "loading"),
    ("status", "loading"),
];

const BSL_MCP_READINESS_MESSAGE_FIELDS: &[&str] = &["error", "message", "state", "status"];

fn bsl_mcp_reply_is_pending(text: &str) -> bool {
    let Ok(Value::Object(reply)) = serde_json::from_str::<Value>(text) else {
        return names_not_ready(text);
    };
    BSL_MCP_READINESS_STATES
        .iter()
        .any(|(field, pending)| reply.get(*field).and_then(Value::as_str) == Some(*pending))
        || BSL_MCP_READINESS_MESSAGE_FIELDS.iter().any(|field| {
            reply
                .get(*field)
                .and_then(Value::as_str)
                .is_some_and(names_not_ready)
        })
}

fn names_not_ready(text: &str) -> bool {
    text.contains("not_ready") || text.contains("not ready")
}

impl ProcessRunner for SystemProcessRunner {
    fn run(&self, command: &ProcessCommand) -> Result<ProcessOutput, String> {
        let output = ManagedChild::run(ManagedCommand {
            program: command.program.clone(),
            args: command.args.iter().map(Into::into).collect(),
            cwd: command.cwd.clone(),
            env: command.env.clone(),
            env_remove: command.env_remove.clone(),
            capture_limits: command.capture_limits,
            timeout: command.timeout,
            cancellation: command.cancellation.clone(),
        })?;
        Ok(map_managed_process_output(output))
    }

    fn run_with_input(
        &self,
        command: &ProcessCommand,
        input: &[u8],
    ) -> Result<ProcessOutput, String> {
        let output = ManagedChild::run_with_input(
            ManagedCommand {
                program: command.program.clone(),
                args: command.args.iter().map(Into::into).collect(),
                cwd: command.cwd.clone(),
                env: command.env.clone(),
                env_remove: command.env_remove.clone(),
                capture_limits: command.capture_limits,
                timeout: command.timeout,
                cancellation: command.cancellation.clone(),
            },
            input.to_vec(),
        )?;
        Ok(map_managed_process_output(output))
    }

    fn run_streaming(
        &self,
        command: &ProcessCommand,
        max_line_bytes: usize,
        on_line: &mut dyn FnMut(usize, &[u8]) -> StreamControl,
    ) -> Result<ProcessStreamOutput, String> {
        let mut child = ManagedChild::spawn(ManagedCommand {
            program: command.program.clone(),
            args: command.args.iter().map(Into::into).collect(),
            cwd: command.cwd.clone(),
            env: command.env.clone(),
            env_remove: command.env_remove.clone(),
            capture_limits: command.capture_limits,
            timeout: command.timeout,
            cancellation: command.cancellation.clone(),
        })?;
        let output = child.wait_for_line_output(max_line_bytes, on_line)?;
        Ok(map_managed_line_output(output))
    }
}

fn map_managed_process_output(mut output: ManagedOutput) -> ProcessOutput {
    let stdout_truncated = output.stdout_truncated;
    let stderr_truncated = output.stderr_truncated;
    let stdout_had_invalid_utf8 = output.stdout_had_invalid_utf8;
    let stderr_had_invalid_utf8 = output.stderr_had_invalid_utf8;
    ensure_truncation_diagnostics(&mut output);
    let output = ProcessOutput {
        status_success: output.status_success,
        status: output.status,
        stdout: output.stdout,
        stderr: output.stderr,
        timed_out: output.timed_out,
        cancelled: output.cancelled,
        stdout_truncated,
        stderr_truncated,
        stdout_had_invalid_utf8,
        stderr_had_invalid_utf8,
    };
    debug_assert!(!(output.timed_out && output.cancelled));
    output
}

fn map_managed_line_output(output: ManagedLineOutput) -> ProcessStreamOutput {
    ProcessStreamOutput {
        status_success: output.status_success,
        status: output.status,
        stderr: output.stderr,
        timed_out: output.timed_out,
        cancelled: output.cancelled,
        stopped_by_consumer: output.stopped_by_consumer,
        line_error: output.line_error,
    }
}

impl BslMcpRunner for SystemBslMcpRunner {
    fn call(&self, command: &BslMcpCommand) -> Result<BslMcpOutput, String> {
        let context = discover_workspace(Some(command.cwd.clone()))?;
        let output = WorkspaceServiceManager::new().call_bsl_mcp_cancellable(
            &context,
            &command.source_dir,
            command.tool_name,
            command.tool_args.clone(),
            command.timeout,
            &command.cancellation,
        )?;
        Ok(BslMcpOutput {
            result_text: output.result_text,
            stderr: output.stderr,
        })
    }
}

pub struct StandardsAdapter;

#[derive(Debug, Clone, PartialEq)]
pub struct StandardsRequest {
    pub method: &'static str,
    pub params: Value,
}

pub trait HttpClient {
    fn post_json(&self, endpoint: &str, payload: &Value) -> Result<String, String>;
}

struct UreqHttpClient;

static UREQ_HTTP_CLIENT: UreqHttpClient = UreqHttpClient;

/// Общий продовый HTTP-клиент для потребителей за пределами модуля:
/// поставщик `v8std` реестра документации держит его в `Arc`, а не через
/// статическую ссылку, чтобы тесты подменяли транспорт значением.
pub(crate) fn shared_http_client() -> std::sync::Arc<dyn HttpClient + Send + Sync> {
    std::sync::Arc::new(UreqHttpClient)
}

impl StandardsAdapter {
    const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

    pub fn request_for(
        operation: &str,
        args: &Map<String, Value>,
    ) -> Result<StandardsRequest, String> {
        match operation {
            "search" => Ok(StandardsRequest {
                method: "v8std_search",
                params: select_params(args, &["query", "limit", "types", "mode"]),
            }),
            "explain" if args.contains_key("codes") => Ok(StandardsRequest {
                method: "v8std_explain_diagnostics",
                params: select_params(args, &["codes"]),
            }),
            "explain" if args.contains_key("snippet") => Ok(StandardsRequest {
                method: "v8std_explain_snippet",
                params: select_params(args, &["snippet", "language", "limit"]),
            }),
            "explain" if args.contains_key("id") || args.contains_key("idOrAliasOrUrl") => {
                let id = args
                    .get("idOrAliasOrUrl")
                    .or_else(|| args.get("id"))
                    .cloned()
                    .ok_or_else(|| "missing id".to_string())?;
                let mut params = Map::new();
                params.insert("id_or_alias_or_url".to_string(), id);
                if let Some(limit) = args.get("bodyLimit").or_else(|| args.get("body_limit")) {
                    params.insert("body_limit".to_string(), limit.clone());
                }
                Ok(StandardsRequest {
                    method: "v8std_get_page",
                    params: Value::Object(params),
                })
            }
            "explain" if args.contains_key("query") => Ok(StandardsRequest {
                method: "v8std_search",
                params: select_params(args, &["query", "limit", "types", "mode"]),
            }),
            "explain" => Err(
                "unica.standards.explain requires one of: codes, snippet, id, idOrAliasOrUrl, query"
                    .to_string(),
            ),
            other => Err(format!("unknown standards operation: {other}")),
        }
    }

    /// Endpoint приходит от вызывающего: цепочку разрешения — политика
    /// `unica.toml`, окружение, встроенное умолчание — знает
    /// `standards_documentation::resolve_standards_endpoint`, и она одна на
    /// фасады и поставщика реестра (ADR-0032 п.4).
    pub fn invoke(operation: &str, args: &Map<String, Value>, endpoint: &str) -> StandardsOutcome {
        Self::invoke_with_client(operation, args, endpoint, &UREQ_HTTP_CLIENT)
    }

    pub fn invoke_with_client(
        operation: &str,
        args: &Map<String, Value>,
        endpoint: &str,
        http: &dyn HttpClient,
    ) -> StandardsOutcome {
        let endpoint = endpoint.to_string();
        let request = match Self::request_for(operation, args) {
            Ok(request) => request,
            Err(error) => {
                return StandardsOutcome::plain(AdapterOutcome {
                    ok: false,
                    summary: format!("unica.standards.{operation} rejected invalid arguments"),
                    changes: Vec::new(),
                    warnings: Vec::new(),
                    errors: vec![error],
                    artifacts: vec![endpoint],
                    stdout: None,
                    stderr: None,
                    command: None,
                })
            }
        };

        let outcome = Self::call(operation, &endpoint, &request, http);

        // The server's own alias list for a standard never carries the bare
        // number (`"647"`), only `"std647"` and its other spellings — the
        // search fallback already names `std647` as the top `code_variant`
        // candidate on a miss, so a caller who typed the bare number the
        // codes are usually cited by (#591) gets an honest `found: false`
        // instead of the page. One bounded retry with the `std` prefix costs
        // nothing when the id was already resolvable and turns the common
        // case into a hit.
        if request.method == "v8std_get_page" && !get_page_found(&outcome.data) {
            if let Some(digits) = bare_numeric_standard_id(&request.params) {
                let retry = StandardsRequest {
                    method: "v8std_get_page",
                    params: with_prefixed_id(&request.params, &digits),
                };
                return Self::call(operation, &endpoint, &retry, http);
            }
        }

        outcome
    }

    fn call(
        operation: &str,
        endpoint: &str,
        request: &StandardsRequest,
        http: &dyn HttpClient,
    ) -> StandardsOutcome {
        let payload = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": request.method,
                "arguments": request.params,
            }
        });

        match http.post_json(endpoint, &payload) {
            Ok(text) => Self::outcome_from_http_body(operation, endpoint, request.method, &text),
            Err(err) => StandardsOutcome::plain(AdapterOutcome {
                ok: false,
                summary: format!(
                    "unica.standards.{operation} failed through internal v8std MCP proxy"
                ),
                changes: Vec::new(),
                warnings: Vec::new(),
                errors: vec![err.to_string()],
                artifacts: vec![endpoint.to_string(), request.method.to_string()],
                stdout: None,
                stderr: None,
                command: None,
            }),
        }
    }

    pub fn outcome_from_http_body(
        operation: &str,
        endpoint: &str,
        remote_method: &str,
        text: &str,
    ) -> StandardsOutcome {
        let normalized = match normalize_mcp_http_body(text) {
            Ok(text) => text,
            Err(error) => {
                return StandardsOutcome::plain(AdapterOutcome {
                    ok: false,
                    summary: format!(
                        "unica.standards.{operation} received invalid v8std MCP response"
                    ),
                    changes: Vec::new(),
                    warnings: Vec::new(),
                    errors: vec![error],
                    artifacts: vec![endpoint.to_string(), remote_method.to_string()],
                    stdout: None,
                    stderr: None,
                    command: None,
                })
            }
        };

        match serde_json::from_str::<Value>(&normalized) {
            Ok(Value::Object(object)) if object.contains_key("error") => {
                let message = object
                    .get("error")
                    .and_then(|error| error.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or("remote JSON-RPC error");
                StandardsOutcome::plain(AdapterOutcome {
                    ok: false,
                    summary: format!(
                        "unica.standards.{operation} failed through internal v8std MCP proxy"
                    ),
                    changes: Vec::new(),
                    warnings: Vec::new(),
                    errors: vec![message.to_string()],
                    artifacts: vec![endpoint.to_string(), remote_method.to_string()],
                    stdout: None,
                    stderr: None,
                    command: None,
                })
            }
            // ADR-0023: the JSON-RPC envelope is transport. The tool publishes
            // the `result` payload it carried, not the envelope as a string.
            Ok(Value::Object(mut object)) if object.contains_key("result") => StandardsOutcome {
                outcome: AdapterOutcome {
                    ok: true,
                    summary: format!(
                        "unica.standards.{operation} completed through internal v8std MCP proxy"
                    ),
                    changes: Vec::new(),
                    warnings: Vec::new(),
                    errors: Vec::new(),
                    artifacts: vec![endpoint.to_string(), remote_method.to_string()],
                    stdout: None,
                    stderr: None,
                    command: None,
                },
                data: object.remove("result"),
            },
            Ok(_) => StandardsOutcome::plain(AdapterOutcome {
                ok: false,
                summary: format!(
                    "unica.standards.{operation} received non-JSON-RPC v8std MCP response"
                ),
                changes: Vec::new(),
                warnings: Vec::new(),
                errors: vec!["missing JSON-RPC result or error".to_string()],
                artifacts: vec![endpoint.to_string(), remote_method.to_string()],
                stdout: None,
                stderr: None,
                command: None,
            }),
            Err(error) => StandardsOutcome::plain(AdapterOutcome {
                ok: false,
                summary: format!("unica.standards.{operation} received invalid v8std MCP JSON"),
                changes: Vec::new(),
                warnings: Vec::new(),
                errors: vec![error.to_string()],
                artifacts: vec![endpoint.to_string(), remote_method.to_string()],
                stdout: None,
                stderr: None,
                command: None,
            }),
        }
    }
}

/// The digits of `id_or_alias_or_url` when it is nothing but a standard's
/// bare number (`"647"`), the form skills and diagnostics cite it by but the
/// server's own alias list never carries. `None` for every other shape,
/// including an already-prefixed `"std647"` — that one already resolves and
/// gains nothing from a retry.
fn bare_numeric_standard_id(params: &Value) -> Option<String> {
    let id = params.get("id_or_alias_or_url")?.as_str()?;
    (!id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit())).then(|| id.to_string())
}

/// `v8std_get_page`'s own verdict, read the same way its `structuredContent`
/// reports it. Missing or unparseable reads as "not found": that only ever
/// widens the retry to a case where it is a harmless extra call, never masks
/// a real hit.
fn get_page_found(data: &Option<Value>) -> bool {
    data.as_ref()
        .and_then(|value| value.get("structuredContent"))
        .and_then(|value| value.get("found"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// `params` with `id_or_alias_or_url` replaced by the `std`-prefixed form of
/// `digits`. `params` is always the object `request_for` built for the
/// `explain`+id path, so the key is always present to replace.
fn with_prefixed_id(params: &Value, digits: &str) -> Value {
    let mut params = params.clone();
    if let Some(object) = params.as_object_mut() {
        object.insert(
            "id_or_alias_or_url".to_string(),
            Value::String(format!("std{digits}")),
        );
    }
    params
}

/// A standards answer plus the `result` payload the remote MCP returned.
#[derive(Debug)]
pub struct StandardsOutcome {
    pub outcome: AdapterOutcome,
    pub data: Option<Value>,
}

impl StandardsOutcome {
    fn plain(outcome: AdapterOutcome) -> Self {
        Self {
            outcome,
            data: None,
        }
    }
}

impl HttpClient for UreqHttpClient {
    fn post_json(&self, endpoint: &str, payload: &Value) -> Result<String, String> {
        ureq::AgentBuilder::new()
            .timeout(StandardsAdapter::DEFAULT_TIMEOUT)
            .build()
            .post(endpoint)
            .set("Content-Type", "application/json")
            .set("Accept", "application/json, text/event-stream")
            .send_string(&payload.to_string())
            .map_err(|err| err.to_string())?
            .into_string()
            .map_err(|err| err.to_string())
    }
}

fn select_params(args: &Map<String, Value>, keys: &[&str]) -> Value {
    let mut params = Map::new();
    for key in keys {
        if let Some(value) = args.get(*key) {
            params.insert((*key).to_string(), value.clone());
        }
    }
    Value::Object(params)
}

fn normalize_mcp_http_body(text: &str) -> Result<String, String> {
    let data_lines = text
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if data_lines.is_empty() {
        return Ok(text.trim().to_string());
    }
    let joined = data_lines.join("\n");
    serde_json::from_str::<Value>(&joined)
        .map_err(|err| format!("invalid JSON-RPC SSE data: {err}"))?;
    Ok(joined)
}

const RUNTIME_MAPPER_CONFIG_INIT_ARGS: &[&str] = &[
    "operation",
    "config",
    "workdir",
    "sourceSet",
    "connection",
    "format",
    "builder",
    "force",
];
const RUNTIME_MAPPER_INIT_ARGS: &[&str] = &["operation", "config", "workdir"];
const RUNTIME_MAPPER_BUILD_ARGS: &[&str] =
    &["operation", "config", "workdir", "sourceSet", "fullRebuild"];
const RUNTIME_MAPPER_DUMP_ARGS: &[&str] = &[
    "operation",
    "config",
    "workdir",
    "mode",
    "object",
    "objects",
    "sourceSet",
    "extension",
];
const RUNTIME_MAPPER_CONVERT_ARGS: &[&str] =
    &["operation", "config", "workdir", "sourceSet", "output"];
const RUNTIME_MAPPER_MAKE_ARGS: &[&str] = &[
    "operation",
    "config",
    "workdir",
    "output",
    "sourceSet",
    "extension",
];
const RUNTIME_MAPPER_LOAD_ARGS: &[&str] = &[
    "operation",
    "config",
    "workdir",
    "path",
    "mode",
    "settings",
    "extension",
];
const RUNTIME_MAPPER_SYNTAX_ARGS: &[&str] = &[
    "operation",
    "config",
    "workdir",
    "mode",
    "server",
    "thinClient",
    "webClient",
    "mobileClient",
    "externalConnection",
    "externalConnectionServer",
    "thickClientManagedApplication",
    "thickClientServerManagedApplication",
    "thickClientOrdinaryApplication",
    "thickClientServerOrdinaryApplication",
    "mobileAppClient",
    "mobileAppServer",
    "mobileClientDigiSign",
    "distributiveModules",
    "unreferenceProcedures",
    "handlersExistence",
    "emptyHandlers",
    "extendedModulesCheck",
    "checkUseSynchronousCalls",
    "checkUseModality",
    "unsupportedFunctional",
    "configLogIntegrity",
    "incorrectReferences",
    "extension",
    "allExtensions",
    "projects",
];
const RUNTIME_MAPPER_TEST_ARGS: &[&str] = &[
    "operation",
    "config",
    "workdir",
    "testRunner",
    "testScope",
    "module",
    "fullOutput",
    "features",
    "filterTags",
    "ignoreTags",
    "scenarioFilters",
];
const RUNTIME_MAPPER_LAUNCH_ARGS: &[&str] = &[
    "operation",
    "config",
    "workdir",
    "clientMode",
    "mode",
    "mcpConfig",
    "mcpPort",
    "c",
    "execute",
    "usePrivilegedMode",
    "output",
    "stderrOutput",
    "waitForExit",
    "waitTimeoutMs",
    "rawKeys",
];
const RUNTIME_MAPPER_EXTENSIONS_ARGS: &[&str] =
    &["operation", "config", "workdir", "sourceSet", "sourceSets"];
const RUNTIME_MAPPER_TOOLS_DOWNLOAD_ARGS: &[&str] =
    &["operation", "config", "workdir", "tool", "sources", "force"];
const RUNTIME_MAPPER_ARRAY_ARGS: &[&str] = &[
    "features",
    "filterTags",
    "ignoreTags",
    "objects",
    "projects",
    "rawKeys",
    "scenarioFilters",
    "sourceSets",
];
const RUNTIME_MAPPER_LOAD_MODES: &[&str] = &["load", "merge"];
const RUNTIME_MAPPER_DUMP_MODES: &[&str] = &["full", "incremental", "partial"];
const RUNTIME_MAPPER_TEST_RUNNERS: &[&str] = &["yaxunit", "va"];
const RUNTIME_MAPPER_TEST_SCOPES: &[&str] = &["all", "module"];
const RUNTIME_MAPPER_TOOLS: &[&str] = &["yaxunit", "vanessa", "client-mcp"];

fn runtime_args(args: &Map<String, Value>, redact: bool) -> Result<Vec<String>, String> {
    if args.contains_key("args") {
        return Err(
            "raw args are not accepted by internal adapters; use typed tool arguments".to_string(),
        );
    }

    let operation = args
        .get("operation")
        .and_then(Value::as_str)
        .ok_or_else(|| "unica.runtime.execute requires string `operation` argument".to_string())?;
    validate_runtime_mapper_payload(operation, args)?;
    let mut result = Vec::new();

    append_runtime_global_args(&mut result, operation, args, redact);

    match operation {
        "config-init" => {
            result.extend(["config".to_string(), "init".to_string()]);
            append_arg(&mut result, "--output", args, "config", redact);
            append_arg(&mut result, "--connection", args, "connection", redact);
            append_arg(&mut result, "--format", args, "format", redact);
            append_arg(&mut result, "--builder", args, "builder", redact);
            append_bool_flag(&mut result, "--force", args, "force");
        }
        "init" => result.push("init".to_string()),
        "build" => {
            result.push("build".to_string());
            append_bool_flag(&mut result, "--full-rebuild", args, "fullRebuild");
            append_arg(&mut result, "--source-set", args, "sourceSet", redact);
        }
        "dump" => {
            result.push("dump".to_string());
            append_arg(&mut result, "--mode", args, "mode", redact);
            append_arg(&mut result, "--object", args, "object", redact);
            append_array_args(&mut result, "--object", args, "objects", redact);
            append_arg(&mut result, "--source-set", args, "sourceSet", redact);
            append_arg(&mut result, "--extension", args, "extension", redact);
        }
        "convert" => {
            result.push("convert".to_string());
            append_arg(&mut result, "--source-set", args, "sourceSet", redact);
            append_arg(&mut result, "--output", args, "output", redact);
        }
        "make" => {
            result.push("make".to_string());
            append_arg(&mut result, "--output", args, "output", redact);
            append_arg(&mut result, "--source-set", args, "sourceSet", redact);
            append_arg(&mut result, "--extension", args, "extension", redact);
        }
        "load" => {
            result.push("load".to_string());
            append_arg(&mut result, "--path", args, "path", redact);
            append_arg(&mut result, "--mode", args, "mode", redact);
            append_arg(&mut result, "--settings", args, "settings", redact);
            append_arg(&mut result, "--extension", args, "extension", redact);
        }
        "syntax" => {
            result.push("syntax".to_string());
            if let Some(mode) = string_arg(args, "mode", redact) {
                result.push(mode);
            }
            append_syntax_args(&mut result, args, redact);
        }
        "test" => {
            result.push("test".to_string());
            if let Some(test_runner) = string_arg(args, "testRunner", redact) {
                result.push(test_runner);
            }
            append_bool_flag(&mut result, "--full", args, "fullOutput");
            if let Some(test_scope) = string_arg(args, "testScope", redact) {
                result.push(test_scope);
            }
            if let Some(module) = string_arg(args, "module", redact) {
                result.push(module);
            }
            append_array_args(&mut result, "--feature", args, "features", redact);
            append_array_args(&mut result, "--filter-tag", args, "filterTags", redact);
            append_array_args(&mut result, "--ignore-tag", args, "ignoreTags", redact);
            append_array_args(
                &mut result,
                "--scenario-filter",
                args,
                "scenarioFilters",
                redact,
            );
        }
        "launch" => {
            result.push("launch".to_string());
            match args.get("clientMode").and_then(Value::as_str) {
                Some("mcp-va") => {
                    result.extend(["mcp".to_string(), "va".to_string()]);
                    append_arg(&mut result, "--mode", args, "mode", redact);
                    append_arg(&mut result, "--mcp-port", args, "mcpPort", redact);
                    append_arg(&mut result, "--mcp-config", args, "mcpConfig", redact);
                }
                Some("mcp") => {
                    result.push("mcp".to_string());
                    append_arg(&mut result, "--mode", args, "mode", redact);
                    append_arg(&mut result, "--mcp-port", args, "mcpPort", redact);
                    append_arg(&mut result, "--mcp-config", args, "mcpConfig", redact);
                }
                Some(client_mode) => {
                    result.push(client_mode.to_string());
                    append_launch_direct_args(&mut result, args, redact);
                }
                None => {}
            }
        }
        "extensions" => {
            result.push("extensions".to_string());
            append_arg(&mut result, "--name", args, "sourceSet", redact);
            append_array_args(&mut result, "--name", args, "sourceSets", redact);
        }
        "tools-download" => {
            result.extend(["tools".to_string(), "download".to_string()]);
            if let Some(tool) = string_arg(args, "tool", redact) {
                result.push(tool);
            }
            append_bool_flag(&mut result, "--sources", args, "sources");
            append_bool_flag(&mut result, "--force", args, "force");
        }
        other => return Err(format!("unknown runtime operation: {other}")),
    }

    Ok(result)
}

fn append_runtime_global_args(
    result: &mut Vec<String>,
    operation: &str,
    args: &Map<String, Value>,
    redact: bool,
) {
    if args.get("waitForExit").and_then(Value::as_bool) == Some(true)
        || (operation == "build" && args.get("fullRebuild").and_then(Value::as_bool) != Some(true))
    {
        result.push("--json-message".to_string());
    }
    if operation != "config-init" {
        append_arg(result, "--config", args, "config", redact);
    }
    append_arg(result, "--workdir", args, "workdir", redact);
}

/// Credentials belong to `infobase.user`/`infobase.password`, not to the
/// connection string. Written into `connection` they reached the project
/// config and were dropped on the way to the platform, so the run reported a
/// wrong password while the same connection worked in Designer (#343).
///
/// The refusal names the field and never echoes the value: the argument it is
/// refusing is the one carrying the secret.
/// A project that declares a client MCP extension must have its artifact where
/// it says. Without this the run reached the platform and failed late, deep in
/// the build, on a cause the caller could have been told before it started
/// (#408).
fn reject_missing_client_mcp_extension(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<(), String> {
    if args.get("operation").and_then(Value::as_str) != Some("build") {
        return Ok(());
    }
    let config_arg = args
        .get("config")
        .and_then(Value::as_str)
        .unwrap_or("v8project.yaml");
    let config_path = context.cwd.join(config_arg);
    let Ok(text) = std::fs::read_to_string(&config_path) else {
        return Ok(());
    };
    let Ok(config) = serde_yaml::from_str::<serde_yaml::Value>(&text) else {
        return Ok(());
    };
    let declared = ["tools", "client_mcp", "extension", "artifact", "path"]
        .iter()
        .try_fold(&config, |node, key| node.get(key))
        .and_then(serde_yaml::Value::as_str);
    let Some(declared) = declared.filter(|path| !path.trim().is_empty()) else {
        return Ok(());
    };
    let artifact = config_path.parent().unwrap_or(&context.cwd).join(declared);
    if artifact.is_file() {
        return Ok(());
    }
    Err(format!(
        "project declares `tools.client_mcp.extension.artifact.path` = `{declared}` but the artifact is missing; download it with operation `tools-download` before `build`"
    ))
}

fn reject_credentials_in_connection(args: &Map<String, Value>) -> Result<(), String> {
    let Some(connection) = args.get("connection").and_then(Value::as_str) else {
        return Ok(());
    };
    let carries = |field: &str| {
        connection.split(';').any(|part| {
            part.trim().split_once('=').is_some_and(|(name, value)| {
                name.trim().eq_ignore_ascii_case(field) && !value.trim().is_empty()
            })
        })
    };
    if carries("Usr") || carries("Pwd") {
        return Err(
            "`connection` must not carry `Usr` or `Pwd`; the platform never receives them from it — put the credentials in infobase.user and infobase.password of v8project.local.yaml"
                .to_string(),
        );
    }
    Ok(())
}

fn validate_runtime_mapper_payload(
    operation: &str,
    args: &Map<String, Value>,
) -> Result<(), String> {
    let allowed = runtime_mapper_operation_args(operation)
        .ok_or_else(|| format!("unknown runtime operation: {operation}"))?;
    for key in args.keys() {
        if matches!(key.as_str(), "cwd" | "dryRun" | "confirm") {
            continue;
        }
        if !allowed.contains(&key.as_str()) {
            return Err(format!("operation `{operation}` does not accept `{key}`"));
        }
    }
    for key in RUNTIME_MAPPER_ARRAY_ARGS {
        validate_mapper_string_array(args, key)?;
    }
    reject_credentials_in_connection(args)?;

    match operation {
        "dump" => {
            validate_mapper_enum(args, "mode", RUNTIME_MAPPER_DUMP_MODES)?;
            if args
                .get("mode")
                .and_then(Value::as_str)
                .is_some_and(|mode| mode == "partial")
                && !args.contains_key("object")
                && !mapper_has_non_empty_array_arg(args, "objects")
            {
                return Err(
                    "operation `dump` with mode `partial` requires `object` or `objects`"
                        .to_string(),
                );
            }
        }
        "load" => {
            if args
                .get("mode")
                .and_then(Value::as_str)
                .is_some_and(|mode| mode == "update")
            {
                return Err(
                    "load --mode update is not supported; use `load` or `merge`".to_string()
                );
            }
            validate_mapper_enum(args, "mode", RUNTIME_MAPPER_LOAD_MODES)?;
            if args
                .get("mode")
                .and_then(Value::as_str)
                .is_some_and(|mode| mode == "merge")
                && !args.contains_key("settings")
            {
                return Err("operation `load` with mode `merge` requires `settings`".to_string());
            }
            if args.contains_key("settings")
                && args.get("mode").and_then(Value::as_str) != Some("merge")
            {
                return Err(
                    "operation `load` accepts `settings` only with mode `merge`".to_string()
                );
            }
        }
        "test" => {
            validate_mapper_enum(args, "testRunner", RUNTIME_MAPPER_TEST_RUNNERS)?;
            validate_mapper_enum(args, "testScope", RUNTIME_MAPPER_TEST_SCOPES)?;
        }
        "launch" => validate_bounded_external_epf_launch(args)?,
        "tools-download" => {
            validate_mapper_enum(args, "tool", RUNTIME_MAPPER_TOOLS)?;
            if args
                .get("sources")
                .and_then(Value::as_bool)
                .unwrap_or(false)
                && args
                    .get("tool")
                    .and_then(Value::as_str)
                    .is_some_and(|tool| tool == "vanessa")
            {
                return Err(
                    "operation `tools-download` accepts `sources` only for `yaxunit` or `client-mcp`"
                        .to_string(),
                );
            }
        }
        _ => {}
    }

    Ok(())
}

fn validate_bounded_external_epf_launch(args: &Map<String, Value>) -> Result<(), String> {
    let wait = args.get("waitForExit").and_then(Value::as_bool);
    if args.contains_key("waitForExit") && wait.is_none() {
        return Err("bounded external EPF `waitForExit` must be boolean".to_string());
    }
    if !wait.unwrap_or(false) {
        if args.contains_key("stderrOutput") || args.contains_key("waitTimeoutMs") {
            return Err(
                "bounded external EPF output and timeout require `waitForExit: true`".to_string(),
            );
        }
        return Ok(());
    }

    if args.get("clientMode").and_then(Value::as_str) != Some("thin") {
        return Err("bounded external EPF launch requires `clientMode: thin`".to_string());
    }
    let required_string = |key: &str| {
        args.get(key)
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| format!("bounded external EPF launch requires non-empty `{key}`"))
    };
    let execute = required_string("execute")?;
    if !execute.to_ascii_lowercase().ends_with(".epf") {
        return Err("bounded external EPF `execute` must name an .epf file".to_string());
    }
    let output = required_string("output")?;
    let stderr_output = required_string("stderrOutput")?;
    if Path::new(output) == Path::new(stderr_output) {
        return Err(
            "bounded external EPF `output` and `stderrOutput` must be distinct".to_string(),
        );
    }
    args.get("waitTimeoutMs")
        .and_then(Value::as_u64)
        .filter(|value| (1..=86_400_000).contains(value))
        .ok_or_else(|| {
            "bounded external EPF `waitTimeoutMs` must be an integer from 1 to 86400000".to_string()
        })?;

    if args
        .get("rawKeys")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .any(|key| {
            ["c", "execute", "out"]
                .iter()
                .any(|reserved| launch_key_alias_matches(key, reserved))
        })
    {
        return Err(
            "bounded external EPF rawKeys must not override /C, /Execute, or /Out".to_string(),
        );
    }
    Ok(())
}

fn launch_key_alias_matches(raw: &str, key: &str) -> bool {
    let trimmed = raw.trim_start();
    if !trimmed.starts_with(['/', '-']) {
        return false;
    }
    let normalized = trimmed
        .trim_start_matches(['/', '-'])
        .trim_end()
        .to_ascii_lowercase();
    if normalized == key {
        return true;
    }
    normalized
        .strip_prefix(key)
        .and_then(|rest| rest.chars().next())
        .is_some_and(|ch| matches!(ch, '"' | '=' | ':' | ' ' | '\t'))
}

fn runtime_mapper_operation_args(operation: &str) -> Option<&'static [&'static str]> {
    match operation {
        "config-init" => Some(RUNTIME_MAPPER_CONFIG_INIT_ARGS),
        "init" => Some(RUNTIME_MAPPER_INIT_ARGS),
        "build" => Some(RUNTIME_MAPPER_BUILD_ARGS),
        "dump" => Some(RUNTIME_MAPPER_DUMP_ARGS),
        "convert" => Some(RUNTIME_MAPPER_CONVERT_ARGS),
        "make" => Some(RUNTIME_MAPPER_MAKE_ARGS),
        "load" => Some(RUNTIME_MAPPER_LOAD_ARGS),
        "syntax" => Some(RUNTIME_MAPPER_SYNTAX_ARGS),
        "test" => Some(RUNTIME_MAPPER_TEST_ARGS),
        "launch" => Some(RUNTIME_MAPPER_LAUNCH_ARGS),
        "extensions" => Some(RUNTIME_MAPPER_EXTENSIONS_ARGS),
        "tools-download" => Some(RUNTIME_MAPPER_TOOLS_DOWNLOAD_ARGS),
        _ => None,
    }
}

fn validate_mapper_string_array(args: &Map<String, Value>, key: &str) -> Result<(), String> {
    let Some(value) = args.get(key) else {
        return Ok(());
    };
    let Some(items) = value.as_array() else {
        return Err(format!("argument `{key}` must be array"));
    };
    for item in items {
        if !item.is_string() {
            return Err(format!("argument `{key}` must contain strings"));
        }
    }
    Ok(())
}

fn validate_mapper_enum(
    args: &Map<String, Value>,
    key: &str,
    allowed: &[&str],
) -> Result<(), String> {
    let Some(value) = args.get(key) else {
        return Ok(());
    };
    let Some(value) = value.as_str() else {
        return Err(format!("argument `{key}` must be string"));
    };
    if !allowed.contains(&value) {
        return Err(format!(
            "argument `{key}` must be one of: {}",
            allowed.join(", ")
        ));
    }
    Ok(())
}

fn mapper_has_non_empty_array_arg(args: &Map<String, Value>, key: &str) -> bool {
    args.get(key)
        .and_then(Value::as_array)
        .is_some_and(|items| !items.is_empty())
}

fn cli_args(args: &Map<String, Value>, redact: bool) -> Result<Vec<String>, String> {
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

fn append_arg(
    result: &mut Vec<String>,
    flag: &str,
    args: &Map<String, Value>,
    key: &str,
    redact: bool,
) {
    if let Some(value) = string_arg(args, key, redact) {
        result.push(flag.to_string());
        result.push(value);
    }
}

fn append_array_args(
    result: &mut Vec<String>,
    flag: &str,
    args: &Map<String, Value>,
    key: &str,
    redact: bool,
) {
    let Some(items) = args.get(key).and_then(Value::as_array) else {
        return;
    };
    for item in items {
        result.push(flag.to_string());
        result.push(reported_cli_value(key, item, redact));
    }
}

fn append_syntax_args(result: &mut Vec<String>, args: &Map<String, Value>, redact: bool) {
    for (key, flag) in [
        ("server", "--server"),
        ("thinClient", "--thin-client"),
        ("webClient", "--web-client"),
        ("mobileClient", "--mobile-client"),
        ("externalConnection", "--external-connection"),
        ("externalConnectionServer", "--external-connection-server"),
        (
            "thickClientManagedApplication",
            "--thick-client-managed-application",
        ),
        (
            "thickClientServerManagedApplication",
            "--thick-client-server-managed-application",
        ),
        (
            "thickClientOrdinaryApplication",
            "--thick-client-ordinary-application",
        ),
        (
            "thickClientServerOrdinaryApplication",
            "--thick-client-server-ordinary-application",
        ),
        ("mobileAppClient", "--mobile-app-client"),
        ("mobileAppServer", "--mobile-app-server"),
        ("mobileClientDigiSign", "--mobile-client-digi-sign"),
        ("distributiveModules", "--distributive-modules"),
        ("unreferenceProcedures", "--unreference-procedures"),
        ("handlersExistence", "--handlers-existence"),
        ("emptyHandlers", "--empty-handlers"),
        ("extendedModulesCheck", "--extended-modules-check"),
        ("checkUseSynchronousCalls", "--check-use-synchronous-calls"),
        ("checkUseModality", "--check-use-modality"),
        ("unsupportedFunctional", "--unsupported-functional"),
        ("configLogIntegrity", "--config-log-integrity"),
        ("incorrectReferences", "--incorrect-references"),
        ("allExtensions", "--all-extensions"),
    ] {
        append_bool_flag(result, flag, args, key);
    }
    append_arg(result, "--extension", args, "extension", redact);
    append_array_args(result, "--project", args, "projects", redact);
}

fn append_launch_direct_args(result: &mut Vec<String>, args: &Map<String, Value>, redact: bool) {
    append_arg(result, "--c", args, "c", redact);
    append_arg(result, "--execute", args, "execute", redact);
    append_bool_flag(result, "--use-privileged-mode", args, "usePrivilegedMode");
    append_arg(result, "--output", args, "output", redact);
    append_arg(result, "--stderr-output", args, "stderrOutput", redact);
    append_bool_flag(result, "--wait-for-exit", args, "waitForExit");
    append_arg(result, "--wait-timeout-ms", args, "waitTimeoutMs", redact);
    append_array_args(result, "--raw-key", args, "rawKeys", redact);
}

fn append_bool_flag(result: &mut Vec<String>, flag: &str, args: &Map<String, Value>, key: &str) {
    if args.get(key).and_then(Value::as_bool).unwrap_or(false) {
        result.push(flag.to_string());
    }
}

fn string_arg(args: &Map<String, Value>, key: &str, redact: bool) -> Option<String> {
    args.get(key).and_then(|value| {
        if value.is_null() {
            return None;
        }
        Some(reported_cli_value(key, value, redact))
    })
}

fn reported_cli_value(key: &str, value: &Value, redact: bool) -> String {
    if !redact {
        return value_to_cli_string(value);
    }
    if is_secret_key(key) {
        return "<redacted>".to_string();
    }
    redactor(&value_to_cli_string(value))
}

fn kebab_case(key: &str) -> String {
    let mut out = String::new();
    for (index, ch) in key.chars().enumerate() {
        if ch == '_' {
            out.push('-');
        } else if ch.is_ascii_uppercase() {
            if index > 0 {
                out.push('-');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
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
    use crate::infrastructure::platform::testing;
    use serde_json::json;
    use std::cell::RefCell;
    use std::ffi::{OsStr, OsString};
    use std::fs;
    use std::io::Write;
    use std::path::Path;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn analyzer_analyze_budget_survives_the_deadline_that_already_started() {
        // The provider deadline starts before the worker thread does, so the
        // smallest configured budget always arrives here a fraction short.
        let almost_minimum =
            Duration::from_secs(DIAGNOSTICS_ANALYZE_TIMEOUT_MIN_SECONDS) - Duration::from_millis(1);
        assert_eq!(
            analyzer_analyze_timeout_seconds(almost_minimum),
            Some(DIAGNOSTICS_ANALYZE_TIMEOUT_MIN_SECONDS),
            "the caller budget must reach the analyzer instead of being dropped"
        );

        let almost_maximum =
            Duration::from_secs(DIAGNOSTICS_ANALYZE_TIMEOUT_MAX_SECONDS) - Duration::from_millis(1);
        assert_eq!(
            analyzer_analyze_timeout_seconds(almost_maximum),
            Some(DIAGNOSTICS_ANALYZE_TIMEOUT_MAX_SECONDS)
        );

        // An exhausted deadline still bounds the analyzer by the smallest
        // accepted value rather than handing it its own default.
        assert_eq!(
            analyzer_analyze_timeout_seconds(Duration::from_secs(1)),
            Some(DIAGNOSTICS_ANALYZE_TIMEOUT_MIN_SECONDS)
        );
        assert_eq!(
            analyzer_analyze_timeout_seconds(Duration::from_secs(7_200)),
            Some(DIAGNOSTICS_ANALYZE_TIMEOUT_MAX_SECONDS)
        );
    }

    #[test]
    fn standards_search_maps_to_v8std_search_request() {
        let mut args = Map::new();
        args.insert("query".to_string(), json!("modal windows"));
        args.insert("limit".to_string(), json!(3));

        let request = StandardsAdapter::request_for("search", &args).unwrap();

        assert_eq!(request.method, "v8std_search");
        assert_eq!(request.params["query"], "modal windows");
        assert_eq!(request.params["limit"], 3);
    }

    #[test]
    fn standards_explain_prefers_diagnostics_codes() {
        let mut args = Map::new();
        args.insert("codes".to_string(), json!(["acc:142"]));
        args.insert("query".to_string(), json!("ignored when codes are present"));

        let request = StandardsAdapter::request_for("explain", &args).unwrap();

        assert_eq!(request.method, "v8std_explain_diagnostics");
        assert_eq!(request.params["codes"][0], "acc:142");
    }

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

        let outcome = RuntimeAdapter::with_runner(&runner)
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

    /// #404 and ADR-0067. The one full retry belongs to the durable entry
    /// point. An applied `unica.runtime.execute` never reaches this adapter
    /// (INV-MCP-RUNTIME-RECEIPT), so a partial-failure receipt here must not
    /// start a second process: the retry would escape the one call that owns
    /// the lifecycle.
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

        let outcome = RuntimeAdapter::with_runner(&runner)
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

        let invocation = plan_runtime_invocation(&args, &context)
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

        let invocation = plan_runtime_invocation(&args, &context)
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
    fn runtime_job_request_carries_one_full_fallback() {
        let mut context = temp_context("runtime-job-request-supported-configuration");
        configure_supported_designer_source(&mut context);
        let mut args = Map::new();
        args.insert("operation".to_string(), json!("build"));

        let invocation = plan_runtime_invocation(&args, &context).unwrap();
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

        let outcome = RuntimeAdapter::with_runner(&runner)
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

        let outcome = RuntimeAdapter::with_runner(&runner)
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

        let invocation = plan_runtime_invocation(&args, &context).unwrap();

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

        let invocation = plan_runtime_invocation(&args, &context).unwrap();

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

        let outcome = RuntimeAdapter::with_runner(&runner)
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

        let outcome = RuntimeAdapter::with_runner(&runner)
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

        let outcome = RuntimeAdapter::with_runner(&runner)
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

        let plan = plan_runtime_invocation(&args, &context)
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

        let invocation = plan_runtime_invocation(&args, &context)
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

        let outcome = RuntimeAdapter::with_runner(&runner)
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

        assert!(error.contains("tools-download"), "{error}");
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

        let error = RuntimeAdapter::with_runner(&runner)
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

        let outcome = RuntimeAdapter::with_runner(&runner)
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

        let outcome = RuntimeAdapter::with_runner(&runner)
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
            let error = RuntimeAdapter::new()
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
            let error = RuntimeAdapter::new()
                .invoke("unica.runtime.execute", &args, &context, false, true)
                .unwrap_err();
            assert!(
                error.contains(&format!("does not accept `{key}`")),
                "{error}"
            );
            args.remove(key);
        }
        std::fs::write(context.cwd.join("v8project.local.yaml"), "infobase: {}\n").unwrap();
        let error = RuntimeAdapter::new()
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

        let outcome = RuntimeAdapter::with_runner(&runner)
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

        let result = RuntimeAdapter::with_runner(&runner)
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

        let result = RuntimeAdapter::with_runner(&runner)
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

        let outcome = RuntimeAdapter::with_runner(&runner)
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

        let outcome = RuntimeAdapter::with_runner(&runner)
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

        let error = validate_bounded_external_epf_artifact_paths(&args, &context.cwd).unwrap_err();

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
            (
                json!({
                    "operation": "tools-download",
                    "tool": "client-mcp",
                    "sources": true,
                    "force": true,
                }),
                vec!["tools", "download", "client-mcp", "--sources", "--force"],
            ),
        ];

        for (input, expected) in cases {
            let args = input.as_object().unwrap().clone();
            assert_eq!(runtime_args(&args, false).unwrap(), expected);
        }
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

        let outcome = invoke_analyze(&BslAnalyzerMcpAdapter::new(), &args, &context, true).outcome;

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
            &BslAnalyzerMcpAdapter::with_process_runner(&runner),
            &Map::new(),
            &context,
            false,
        );

        assert!(analyzer.outcome.ok, "{:?}", analyzer.outcome);
        assert!(analyzer.outcome.stdout.is_none());
        let batch = analyzer.diagnostics.as_ref().expect("typed diagnostics");
        assert_eq!(
            batch.outcome.status,
            crate::domain::diagnostics::DiagnosticProviderStatus::Empty
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
            &BslAnalyzerMcpAdapter::with_process_runner(&runner),
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
            crate::domain::diagnostics::DiagnosticObservation::Diagnostic {
                location: crate::domain::diagnostics::DiagnosticObservationLocation::Resource { handle },
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
            &BslAnalyzerMcpAdapter::with_process_runner(&runner),
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
    fn diagnostics_analyze_keeps_default_timeout() {
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

        let outcome = BslAnalyzerMcpAdapter::with_process_runner(&runner)
            .invoke_diagnostics_analyze(
                "unica.code.diagnostics",
                &Map::new(),
                &context,
                false,
                Some(&operational_config),
                &CancellationToken::new(),
            )
            .unwrap()
            .outcome;

        assert!(outcome.ok);
        assert_eq!(
            runner.commands.borrow()[0].timeout,
            Some(DEFAULT_PROCESS_TIMEOUT)
        );
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

        let outcome = BslAnalyzerMcpAdapter::with_process_runner(&runner)
            .invoke_diagnostics_analyze(
                "unica.code.diagnostics",
                &Map::new(),
                &context,
                false,
                Some(&operational_config),
                &CancellationToken::new(),
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
            &BslAnalyzerMcpAdapter::with_process_runner(&runner),
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

        let analyzer = BslAnalyzerMcpAdapter::with_runner(&runner)
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

        let analyzer = BslAnalyzerMcpAdapter::with_runner(&runner)
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
    fn legacy_diagnostics_file_route_is_not_available_through_graph_adapter() {
        let context = temp_context("diagnostics-mcp");
        let runner = RecordingBslMcpRunner {
            commands: RefCell::new(Vec::new()),
            output: BslMcpOutput {
                result_text: "{\"action\":\"file\",\"findings\":[]}".to_string(),
                stderr: String::new(),
            },
        };
        let mut args = Map::new();
        args.insert("mode".to_string(), json!("file"));
        args.insert(
            "path".to_string(),
            json!("CommonModules/SmokeModule/Ext/Module.bsl"),
        );
        args.insert("codes".to_string(), json!(["UnusedLocalVariable"]));
        args.insert("minSeverity".to_string(), json!("warning"));
        args.insert("rangeStart".to_string(), json!(3));
        args.insert("rangeEnd".to_string(), json!(7));
        args.insert("detail".to_string(), json!("detailed"));
        args.insert("limit".to_string(), json!(5));

        let error = BslAnalyzerMcpAdapter::with_runner(&runner)
            .invoke("unica.code.diagnostics", &args, &context, false)
            .unwrap_err();

        assert_eq!(
            error,
            "unsupported bsl-analyzer MCP tool: unica.code.diagnostics"
        );
        assert!(runner.commands.borrow().is_empty());
        cleanup_context(&context);
    }

    #[test]
    fn legacy_absolute_diagnostics_path_does_not_restore_adapter_route() {
        let context = temp_context("diagnostics-absolute-inside");
        let path = context
            .cwd
            .join("CommonModules/SmokeModule/Ext/Module.bsl")
            .display()
            .to_string();
        let runner = RecordingBslMcpRunner {
            commands: RefCell::new(Vec::new()),
            output: BslMcpOutput {
                result_text: "{\"action\":\"file\",\"findings\":[]}".to_string(),
                stderr: String::new(),
            },
        };
        let mut args = Map::new();
        args.insert("mode".to_string(), json!("file"));
        args.insert("path".to_string(), json!(path));

        let error = BslAnalyzerMcpAdapter::with_runner(&runner)
            .invoke("unica.code.diagnostics", &args, &context, false)
            .unwrap_err();

        assert!(error.starts_with("unsupported bsl-analyzer MCP tool:"));
        assert!(runner.commands.borrow().is_empty());
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
        if let Some(symlink_result) =
            crate::infrastructure::platform::filesystem::create_dir_symlink_for_test(
                &outside,
                context.workspace_root.join("src").join("escape"),
            )
        {
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

            let error = BslAnalyzerMcpAdapter::with_runner(&runner)
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
    fn legacy_non_string_diagnostics_paths_cannot_reach_runner() {
        let context = temp_context("diagnostics-non-string-path");

        for path in [Value::Null, json!(true), json!(1), json!([]), json!({})] {
            let runner = RecordingBslMcpRunner {
                commands: RefCell::new(Vec::new()),
                output: BslMcpOutput {
                    result_text: "{\"action\":\"file\",\"findings\":[]}".to_string(),
                    stderr: String::new(),
                },
            };
            let mut args = Map::new();
            args.insert("mode".to_string(), json!("file"));
            args.insert("path".to_string(), path.clone());

            let error = BslAnalyzerMcpAdapter::with_runner(&runner)
                .invoke("unica.code.diagnostics", &args, &context, false)
                .unwrap_err();
            assert!(
                error.starts_with("unsupported bsl-analyzer MCP tool:"),
                "{path}: {error}"
            );
            assert!(runner.commands.borrow().is_empty(), "{path}");
        }

        cleanup_context(&context);
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

        let outcome = BslAnalyzerMcpAdapter::with_runner(&runner)
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

    #[test]
    fn legacy_diagnostics_loading_route_is_not_available() {
        let context = temp_context("diagnostics-loading");
        let runner = RecordingBslMcpRunner {
            commands: RefCell::new(Vec::new()),
            output: BslMcpOutput {
                result_text: "{\"action\":\"file\",\"status\":\"loading\"}".to_string(),
                stderr: String::new(),
            },
        };
        let mut args = Map::new();
        args.insert("mode".to_string(), json!("file"));
        args.insert(
            "path".to_string(),
            json!("CommonModules/Probe/Ext/Module.bsl"),
        );

        let error = BslAnalyzerMcpAdapter::with_runner(&runner)
            .invoke("unica.code.diagnostics", &args, &context, false)
            .unwrap_err();

        assert!(error.starts_with("unsupported bsl-analyzer MCP tool:"));
        assert!(runner.commands.borrow().is_empty());
        cleanup_context(&context);
    }

    #[test]
    fn legacy_diagnostics_status_route_is_not_available() {
        let context = temp_context("diagnostics-status-loading");
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

        let error = BslAnalyzerMcpAdapter::with_runner(&runner)
            .invoke("unica.code.diagnostics", &args, &context, false)
            .unwrap_err();

        assert!(error.starts_with("unsupported bsl-analyzer MCP tool:"));
        assert!(runner.commands.borrow().is_empty());
        cleanup_context(&context);
    }

    #[test]
    fn legacy_diagnostics_findings_route_is_not_available() {
        let context = temp_context("diagnostics-quoted-loading");
        let runner = RecordingBslMcpRunner {
            commands: RefCell::new(Vec::new()),
            output: BslMcpOutput {
                result_text: "{\"action\":\"file\",\"findings\":[{\"code\":\"LineLength\",\"message\":\"literal \\\"status\\\":\\\"loading\\\" is not ready for review\"}]}"
                    .to_string(),
                stderr: String::new(),
            },
        };
        let mut args = Map::new();
        args.insert("mode".to_string(), json!("file"));
        args.insert(
            "path".to_string(),
            json!("CommonModules/Probe/Ext/Module.bsl"),
        );

        let error = BslAnalyzerMcpAdapter::with_runner(&runner)
            .invoke("unica.code.diagnostics", &args, &context, false)
            .unwrap_err();

        assert!(error.starts_with("unsupported bsl-analyzer MCP tool:"));
        assert!(runner.commands.borrow().is_empty());
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
            .invoke_diagnostics_analyze(
                "bsl-analyzer diagnostics provider",
                args,
                context,
                dry_run,
                None,
                &CancellationToken::new(),
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
            &BslAnalyzerMcpAdapter::with_process_runner(&runner),
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
            crate::domain::diagnostics::DiagnosticProviderStatus::Unavailable
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

        let outcome = RuntimeAdapter::new()
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

        let outcome = RuntimeAdapter::with_runner(&runner)
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

        let outcome = RuntimeAdapter::with_runner(&runner)
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

        let outcome = RuntimeAdapter::with_runner(&runner)
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

        let outcome = RuntimeAdapter::with_runner(&runner)
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

        let outcome = RuntimeAdapter::with_runner(&runner)
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
        let output = SYSTEM_PROCESS_RUNNER
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
        let output = SYSTEM_PROCESS_RUNNER
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
        let output = SYSTEM_PROCESS_RUNNER
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
        let command = testing::command_writing_stdout("ok");

        let output = SYSTEM_PROCESS_RUNNER
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
        let command = testing::long_running_command();
        let token = crate::domain::cancellation::CancellationToken::new();
        token.cancel();

        let output = SYSTEM_PROCESS_RUNNER
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

    #[test]
    fn standards_mcp_error_body_is_reported_as_failure() {
        let outcome = StandardsAdapter::outcome_from_http_body(
            "explain",
            "https://example.test/mcp",
            "v8std_get_page",
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32602,"message":"bad id"}}"#,
        );

        assert!(!outcome.outcome.ok);
        assert!(outcome
            .outcome
            .errors
            .iter()
            .any(|error| error.contains("bad id")));
        assert!(outcome.outcome.stdout.is_none());
        assert!(outcome.data.is_none());
    }

    #[test]
    fn standards_sse_body_extracts_structured_json_result() {
        let outcome = StandardsAdapter::outcome_from_http_body(
            "search",
            "https://example.test/mcp",
            "v8std_search",
            "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"ok\":true}}\n\n",
        );

        assert!(outcome.outcome.ok);
        // ADR-0023: the JSON-RPC envelope is transport; the tool publishes the
        // result it carried.
        assert!(outcome.outcome.stdout.is_none());
        assert_eq!(outcome.data.unwrap(), json!({"ok": true}));
    }

    #[test]
    fn standards_protocol_mismatch_is_failure() {
        let outcome = StandardsAdapter::outcome_from_http_body(
            "search",
            "https://example.test/mcp",
            "v8std_search",
            r#"{"not":"json-rpc"}"#,
        );

        assert!(!outcome.outcome.ok);
        assert!(outcome
            .outcome
            .errors
            .iter()
            .any(|error| error.contains("missing JSON-RPC")));
    }

    #[test]
    fn standards_adapter_uses_fake_http_client_for_json_rpc_mapping() {
        let client = FakeHttpClient {
            payloads: RefCell::new(Vec::new()),
            response: r#"{"jsonrpc":"2.0","id":1,"result":{"content":[]}}"#.to_string(),
        };
        let mut args = Map::new();
        args.insert("query".to_string(), json!("модальные окна"));
        args.insert("limit".to_string(), json!(2));

        let outcome = StandardsAdapter::invoke_with_client(
            "search",
            &args,
            "https://ai.v8std.ru/mcp",
            &client,
        );

        assert!(outcome.outcome.ok);
        assert_eq!(outcome.data.unwrap(), json!({"content": []}));
        let payloads = client.payloads.borrow();
        assert_eq!(payloads.len(), 1);
        assert_eq!(payloads[0]["method"], "tools/call");
        assert_eq!(payloads[0]["params"]["name"], "v8std_search");
        assert_eq!(
            payloads[0]["params"]["arguments"]["query"],
            "модальные окна"
        );
        assert_eq!(payloads[0]["params"]["arguments"]["limit"], 2);
    }

    /// #591 point 7: `unica.standards.explain` misses a standard when
    /// `idOrAliasOrUrl` is the bare number skills and users type naturally
    /// (`"647"`), even though the server's own alias list only ever carries
    /// the `std`-prefixed form (`"std647"`) and the search fallback already
    /// names it as the top `code_variant` candidate. Two calls, second one
    /// prefixed, proves the retry rather than a lucky first hit.
    struct BareNumericStandardIdHttpClient {
        payloads: RefCell<Vec<Value>>,
    }

    impl HttpClient for BareNumericStandardIdHttpClient {
        fn post_json(&self, _endpoint: &str, payload: &Value) -> Result<String, String> {
            self.payloads.borrow_mut().push(payload.clone());
            let id = payload["params"]["arguments"]["id_or_alias_or_url"]
                .as_str()
                .unwrap_or_default();
            let body = if id == "std647" {
                r#"{"jsonrpc":"2.0","id":1,"result":{
                    "content":[{"type":"text","text":"{\"found\":true,\"page\":{\"id\":\"std647\"}}"}],
                    "structuredContent":{"found":true,"page":{"id":"std647"}}
                }}"#
            } else {
                r#"{"jsonrpc":"2.0","id":1,"result":{
                    "content":[{"type":"text","text":"{\"found\":false,\"candidates\":[{\"id\":\"std647\"}]}"}],
                    "structuredContent":{"found":false,"candidates":[{"id":"std647"}]}
                }}"#
            };
            Ok(body.to_string())
        }
    }

    #[test]
    fn standards_explain_retries_a_bare_numeric_id_with_the_std_prefix() {
        let client = BareNumericStandardIdHttpClient {
            payloads: RefCell::new(Vec::new()),
        };
        let mut args = Map::new();
        args.insert("idOrAliasOrUrl".to_string(), json!("647"));

        let outcome = StandardsAdapter::invoke_with_client(
            "explain",
            &args,
            "https://ai.v8std.ru/mcp",
            &client,
        );

        assert!(outcome.outcome.ok);
        assert_eq!(
            outcome.data.unwrap()["structuredContent"]["found"],
            json!(true),
            "the retried std647 lookup should be what the caller sees"
        );

        let payloads = client.payloads.borrow();
        assert_eq!(
            payloads.len(),
            2,
            "a bare number must be retried, not answered from the first miss"
        );
        assert_eq!(
            payloads[0]["params"]["arguments"]["id_or_alias_or_url"],
            "647"
        );
        assert_eq!(
            payloads[1]["params"]["arguments"]["id_or_alias_or_url"],
            "std647"
        );
    }

    #[test]
    fn standards_explain_does_not_retry_an_id_that_is_already_std_prefixed() {
        let client = BareNumericStandardIdHttpClient {
            payloads: RefCell::new(Vec::new()),
        };
        let mut args = Map::new();
        args.insert("idOrAliasOrUrl".to_string(), json!("std647"));

        let outcome = StandardsAdapter::invoke_with_client(
            "explain",
            &args,
            "https://ai.v8std.ru/mcp",
            &client,
        );

        assert!(outcome.outcome.ok);
        let payloads = client.payloads.borrow();
        assert_eq!(payloads.len(), 1, "an already-resolvable id must not retry");
    }

    #[test]
    fn standards_explain_does_not_retry_a_non_numeric_miss() {
        struct AlwaysMisses;
        impl HttpClient for AlwaysMisses {
            fn post_json(&self, _endpoint: &str, _payload: &Value) -> Result<String, String> {
                Ok(r#"{"jsonrpc":"2.0","id":1,"result":{
                    "content":[{"type":"text","text":"{\"found\":false}"}],
                    "structuredContent":{"found":false}
                }}"#
                .to_string())
            }
        }
        let mut args = Map::new();
        args.insert("idOrAliasOrUrl".to_string(), json!("modal-windows"));

        let outcome = StandardsAdapter::invoke_with_client(
            "explain",
            &args,
            "https://ai.v8std.ru/mcp",
            &AlwaysMisses,
        );

        // A name that was never a bare number has no `std<N>` form to retry
        // with; the honest miss must reach the caller unchanged.
        assert!(outcome.outcome.ok);
        assert_eq!(
            outcome.data.unwrap()["structuredContent"]["found"],
            json!(false)
        );
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
        let expected_config =
            crate::infrastructure::platform::filesystem::strip_windows_extended_length_prefix(
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

    struct FakeHttpClient {
        payloads: RefCell<Vec<Value>>,
        response: String,
    }

    impl HttpClient for FakeHttpClient {
        fn post_json(&self, _endpoint: &str, payload: &Value) -> Result<String, String> {
            self.payloads.borrow_mut().push(payload.clone());
            Ok(self.response.clone())
        }
    }
}
#[test]
fn managed_truncation_is_visible_at_process_adapter_boundary() {
    let output = map_managed_process_output(ManagedOutput {
        status_success: false,
        status: "exit status: 0".into(),
        stdout: "tail".into(),
        stderr: "diagnostic tail".into(),
        timed_out: false,
        cancelled: false,
        stdout_truncated: true,
        stderr_truncated: true,
        stdout_had_invalid_utf8: true,
        stderr_had_invalid_utf8: true,
    });
    assert!(output.stderr.contains("stdout capture truncated"));
    assert!(output.stderr.contains("earlier stderr diagnostics omitted"));
    assert!(output.stdout_truncated);
    assert!(output.stderr_truncated);
    assert!(output.stdout_had_invalid_utf8);
    assert!(output.stderr_had_invalid_utf8);
}
