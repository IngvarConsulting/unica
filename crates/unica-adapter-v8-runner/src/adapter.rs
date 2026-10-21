//! v8 command planning and synchronous adapter implementation.
use crate::preflight::RuntimeInvocationPlan;
use serde_json::{json, Map, Value};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use unica_artifact_delivery::engine::BundledTool;
use unica_execution_contract::adapter_outcome::merge_warnings;
use unica_execution_contract::adapter_outcome::AdapterOutcome;
use unica_process::arguments::{append_arg, append_array_args, append_bool_flag};
use unica_process::runner::{system_process_runner, ProcessCommand, ProcessRunner};
use unica_redaction::redactor;
use unica_runner::attempt::RuntimeWorkspaceObservation;
use unica_source_contract::workspace::WorkspaceContext;
use unica_work_control::cancellation::CancellationToken;
use unica_workspace::paths::{normalize_path_identity, path_lock_identity};

/// Composition resolves installation and owns product-facing preview mapping;
/// command/schema/config/result semantics remain in this adapter.
pub trait RuntimeEnvironment: Send + Sync {
    fn workspace(&self) -> &dyn RuntimeWorkspaceObservation;
    fn plugin_root(&self, cwd: &Path) -> Option<PathBuf>;
    fn resolve_v8_tool(
        &self,
        plugin_root: &Path,
        require_exists: bool,
    ) -> Result<BundledTool, String>;
    fn preview(
        &self,
        tool_name: &str,
        label: &str,
        mutating: bool,
        tool: BundledTool,
        command: Vec<String>,
    ) -> AdapterOutcome;
}

use unica_execution_contract::adapter_outcome::cancelled_process_outcome;
use unica_process::arguments::process_timeout_error;
pub struct RuntimeAdapter<'a> {
    runner: &'a dyn ProcessRunner,
    environment: &'a dyn RuntimeEnvironment,
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

pub struct RuntimeInvocation<'a> {
    pub tool_name: &'a str,
    pub args: &'a Map<String, Value>,
    pub context: &'a WorkspaceContext,
    pub dry_run: bool,
    pub mutating: bool,
}

pub fn plan_runtime_invocation(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    observer: &dyn RuntimeWorkspaceObservation,
) -> Result<RuntimeInvocationPlan, String> {
    if args.get("operation").and_then(Value::as_str) == Some("build") {
        validate_runtime_mapper_payload("build", args)?;
    }
    crate::preflight::plan_runtime_invocation(args, context, observer)
}
impl<'a> RuntimeAdapter<'a> {
    pub fn new(environment: &'a dyn RuntimeEnvironment) -> Self {
        Self {
            runner: system_process_runner(),
            environment,
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_runner(
        runner: &'a dyn ProcessRunner,
        environment: &'a dyn RuntimeEnvironment,
    ) -> Self {
        Self {
            runner,
            environment,
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

    pub fn invoke_cancellable(
        &self,
        invocation: RuntimeInvocation<'_>,
        cancellation: &CancellationToken,
    ) -> Result<RuntimeAdapterOutcome, String> {
        self.invoke_cancellable_with_limits(invocation, cancellation, None, None)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn invoke_cancellable_with_limits_for_test(
        &self,
        invocation: RuntimeInvocation<'_>,
        cancellation: &CancellationToken,
        process_timeout: Option<Duration>,
        capture_limits: Option<(usize, usize)>,
    ) -> Result<RuntimeAdapterOutcome, String> {
        self.invoke_cancellable_with_limits(
            invocation,
            cancellation,
            process_timeout,
            capture_limits,
        )
    }

    fn invoke_cancellable_with_limits(
        &self,
        invocation: RuntimeInvocation<'_>,
        cancellation: &CancellationToken,
        process_timeout: Option<Duration>,
        capture_limits: Option<(usize, usize)>,
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
        let invocation = plan_runtime_invocation(args, context, self.environment.workspace())?;
        reject_missing_client_mcp_extension(&invocation.args, context)?;
        if let Some(outcome) = bind_external_processor_config(args, context, dry_run)? {
            return Ok(RuntimeAdapterOutcome::plain(outcome));
        }
        let plugin_root = self.environment.plugin_root(&context.cwd).ok_or_else(|| {
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
        let bundled_tool = self.environment.resolve_v8_tool(&plugin_root, !dry_run)?;
        let mut command = vec![bundled_tool.program.display().to_string()];
        command.extend(report_args.iter().cloned());

        if dry_run {
            let mut outcome = self.environment.preview(
                tool_name,
                "v8-runner runtime",
                mutating,
                bundled_tool,
                command,
            );
            outcome.warnings = merge_warnings(invocation.warnings, outcome.warnings);
            return Ok(RuntimeAdapterOutcome::plain(outcome));
        }

        if cancellation.is_cancelled() {
            return Ok(RuntimeAdapterOutcome::plain(AdapterOutcome::cancelled(
                format!("{tool_name} cancelled before v8-runner launch"),
            )));
        }

        let process_command = ProcessCommand {
            program: bundled_tool.program.clone(),
            args: execution_args,
            cwd: context.cwd.clone(),
            env: Vec::new(),
            env_remove: Vec::new(),
            capture_limits,
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
        // This synchronous adapter runs one attempt. A full retry belongs
        // to the durable worker, which owns both attempts.
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

pub fn validate_bounded_external_epf_artifact_paths(
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

pub fn runtime_args(args: &Map<String, Value>, redact: bool) -> Result<Vec<String>, String> {
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
            if let Some(mode) = unica_process::arguments::string_arg(args, "mode", redact) {
                result.push(mode);
            }
            append_syntax_args(&mut result, args, redact);
        }
        "test" => {
            result.push("test".to_string());
            if let Some(test_runner) =
                unica_process::arguments::string_arg(args, "testRunner", redact)
            {
                result.push(test_runner);
            }
            append_bool_flag(&mut result, "--full", args, "fullOutput");
            if let Some(test_scope) =
                unica_process::arguments::string_arg(args, "testScope", redact)
            {
                result.push(test_scope);
            }
            if let Some(module) = unica_process::arguments::string_arg(args, "module", redact) {
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
        "project declares `tools.client_mcp.extension.artifact.path` = `{declared}` but the artifact is missing; put the built `.cfe` at that path before `build` — the product does not fetch it, delivery of client-mcp is the toolchain of 0.14 (#870)"
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

pub fn validate_runtime_mapper_payload(
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

#[cfg(any(test, feature = "test-support"))]
pub mod test_support {
    pub fn reject_missing_client_mcp_extension(
        args: &serde_json::Map<String, serde_json::Value>,
        context: &unica_source_contract::workspace::WorkspaceContext,
    ) -> Result<(), String> {
        super::reject_missing_client_mcp_extension(args, context)
    }
}
