use crate::domain::source_sync::{BuildStepMode, BuildTerminalEntry, SourceSetName};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::bundled_tools::resolve_bundled_tool;
use crate::infrastructure::plugin_runtime::{find_plugin_root, value_to_cli_string};
use crate::infrastructure::workspace_index::{
    IndexReadiness, IndexRunner, WorkspaceIndexService, SYSTEM_INDEX_RUNNER,
};
use crate::infrastructure::workspace_services::WorkspaceServiceManager;
use crate::infrastructure::AdapterOutcome;
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;
use std::env;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const DEFAULT_PROCESS_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone)]
pub struct ProcessCommand {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub timeout: Option<Duration>,
}

#[derive(Debug, Clone)]
pub struct ProcessOutput {
    pub status_success: bool,
    pub status: String,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
}

pub trait ProcessRunner {
    fn run(&self, command: &ProcessCommand) -> Result<ProcessOutput, String>;
}

#[derive(Debug, Clone)]
pub struct BslMcpCommand {
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub source_dir: PathBuf,
    pub timeout: Duration,
    pub tool_name: &'static str,
    pub tool_args: Value,
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

pub struct CliAdapter<'a> {
    tool_name: &'static str,
    default_command: &'static [&'static str],
    label: &'static str,
    runner: &'a dyn ProcessRunner,
}

pub struct RuntimeAdapter<'a> {
    runner: &'a dyn ProcessRunner,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuntimeBuildReport {
    pub envelope_ok: bool,
    pub steps: Vec<BuildTerminalEntry>,
}

#[derive(Deserialize)]
struct RawRuntimeBuildEnvelope {
    ok: bool,
    command: String,
    data: RawRuntimeBuildData,
}

#[derive(Deserialize)]
struct RawRuntimeBuildData {
    steps: Vec<RawRuntimeBuildStep>,
}

#[derive(Deserialize)]
struct RawRuntimeBuildStep {
    source_set: String,
    mode: Value,
    ok: bool,
    #[serde(default)]
    message: Option<String>,
}

pub(crate) fn parse_runtime_build_report(stdout: &str) -> Result<RuntimeBuildReport, String> {
    let envelope = serde_json::from_str::<RawRuntimeBuildEnvelope>(stdout.trim())
        .map_err(|error| format!("invalid v8-runner build JSON envelope: {error}"))?;
    if envelope.command != "build" {
        return Err(format!(
            "invalid v8-runner build JSON envelope: expected command `build`, got `{}`",
            envelope.command
        ));
    }

    let mut source_sets = BTreeSet::new();
    let mut steps = Vec::with_capacity(envelope.data.steps.len());
    for raw in envelope.data.steps {
        if raw.source_set.trim().is_empty() {
            return Err(
                "invalid v8-runner build JSON envelope: step source_set is blank".to_string(),
            );
        }
        if !source_sets.insert(raw.source_set.clone()) {
            return Err(format!(
                "invalid v8-runner build JSON envelope: duplicate source_set `{}`",
                raw.source_set
            ));
        }
        steps.push(BuildTerminalEntry {
            source_set: SourceSetName::new(raw.source_set)?,
            mode: parse_runtime_build_mode(&raw.mode)?,
            ok: raw.ok,
            message: raw.message,
        });
    }

    Ok(RuntimeBuildReport {
        envelope_ok: envelope.ok,
        steps,
    })
}

fn parse_runtime_build_mode(value: &Value) -> Result<BuildStepMode, String> {
    match value {
        Value::String(mode) => match mode.as_str() {
            "edt_export" => Ok(BuildStepMode::EdtExport),
            "full" => Ok(BuildStepMode::Full),
            "skipped" => Ok(BuildStepMode::Skipped),
            other => Err(format!(
                "invalid v8-runner build JSON envelope: unknown build mode `{other}`"
            )),
        },
        Value::Object(mode) if mode.len() == 1 && mode.contains_key("partial") => mode
            .get("partial")
            .and_then(Value::as_object)
            .filter(|payload| {
                payload.len() == 1 && payload.get("file_count").and_then(Value::as_u64).is_some()
            })
            .and_then(|payload| {
                payload
                    .get("file_count")
                    .and_then(Value::as_u64)
                    .and_then(|count| usize::try_from(count).ok())
                    .map(|file_count| BuildStepMode::Partial { file_count })
            })
            .ok_or_else(|| {
                format!(
                    "invalid v8-runner build JSON envelope: malformed partial build mode {value}"
                )
            }),
        _ => Err(format!(
            "invalid v8-runner build JSON envelope: unsupported build mode {value}"
        )),
    }
}

struct RuntimeInvocation<'a> {
    tool_name: &'a str,
    context: &'a WorkspaceContext,
    plugin_root: &'a Path,
    report_args: Vec<String>,
    execution_args: Vec<String>,
    dry_run: bool,
    mutating: bool,
}

pub struct CodeSearchAdapter<'a> {
    bsl_runner: &'a dyn ProcessRunner,
    grep_runner: &'a dyn ProcessRunner,
    index_runner: &'a dyn IndexRunner,
    use_workspace_service: bool,
}

pub struct CodeNavigationAdapter<'a> {
    index_runner: &'a dyn IndexRunner,
    grep_runner: &'a dyn ProcessRunner,
    use_workspace_service: bool,
}

pub struct BslAnalyzerMcpAdapter<'a> {
    runner: &'a dyn BslMcpRunner,
}

struct SearchBackendResult {
    section: String,
    ok: bool,
    diagnostics: Vec<String>,
    artifacts: Vec<String>,
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
        }
    }

    #[cfg(test)]
    pub fn with_runner(
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
        }
    }

    pub fn invoke(
        &self,
        tool_name: &str,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
        dry_run: bool,
        mutating: bool,
    ) -> Result<AdapterOutcome, String> {
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
        let process_timeout = Some(DEFAULT_PROCESS_TIMEOUT);
        let output = self.runner.run(&ProcessCommand {
            program: bundled_tool.program.clone(),
            args: process_args,
            cwd: context.cwd.clone(),
            timeout: process_timeout,
        })?;
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

    pub fn invoke(
        &self,
        tool_name: &str,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
        dry_run: bool,
        mutating: bool,
    ) -> Result<AdapterOutcome, String> {
        let plugin_root = find_plugin_root(&context.cwd).ok_or_else(|| {
            "could not locate Unica plugin root for internal adapter lookup".to_string()
        })?;
        let report_args = runtime_args(args, true)?;
        let execution_args = runtime_args(args, false)?;
        self.invoke_runtime(RuntimeInvocation {
            tool_name,
            context,
            plugin_root: &plugin_root,
            report_args,
            execution_args,
            dry_run,
            mutating,
        })
    }

    pub fn invoke_syntax_json(
        &self,
        tool_name: &str,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
    ) -> Result<AdapterOutcome, String> {
        let plugin_root = find_plugin_root(&context.cwd).ok_or_else(|| {
            "could not locate Unica plugin root for internal adapter lookup".to_string()
        })?;
        let report_args = runtime_syntax_json_args(args, true)?;
        let execution_args = runtime_syntax_json_args(args, false)?;
        self.invoke_runtime(RuntimeInvocation {
            tool_name,
            context,
            plugin_root: &plugin_root,
            report_args,
            execution_args,
            dry_run: false,
            mutating: false,
        })
    }

    fn invoke_runtime(&self, invocation: RuntimeInvocation<'_>) -> Result<AdapterOutcome, String> {
        let RuntimeInvocation {
            tool_name,
            context,
            plugin_root,
            report_args,
            execution_args,
            dry_run,
            mutating,
        } = invocation;
        let bundled_tool = resolve_bundled_tool(plugin_root, "v8-runner", !dry_run)?;
        let mut command = vec![bundled_tool.program.display().to_string()];
        command.extend(report_args);

        if dry_run {
            return Ok(AdapterOutcome {
                ok: true,
                summary: format!(
                    "dry run: {tool_name} would call internal v8-runner runtime adapter"
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

        let process_timeout = None;
        let process_command = ProcessCommand {
            program: bundled_tool.program.clone(),
            args: execution_args,
            cwd: context.cwd.clone(),
            timeout: process_timeout,
        };
        let output = match self.runner.run(&process_command) {
            Ok(output) => output,
            Err(error) => {
                let error = redact_sensitive_text(&error);
                return Ok(AdapterOutcome {
                    ok: false,
                    summary: format!(
                        "{tool_name} failed through internal v8-runner runtime adapter"
                    ),
                    changes: Vec::new(),
                    warnings: vec![
                        "internal v8-runner runtime adapter failed to spawn process".to_string()
                    ],
                    errors: vec![error.clone()],
                    artifacts: Vec::new(),
                    stdout: None,
                    stderr: Some(format!("{error}\n")),
                    command: Some(command),
                });
            }
        };
        let ok = output.status_success;
        let stdout = redact_sensitive_text(&output.stdout);
        let stderr = redact_sensitive_text(&output.stderr);
        Ok(AdapterOutcome {
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
            warnings: if ok {
                Vec::new()
            } else if output.timed_out {
                vec!["internal v8-runner runtime adapter timed out".to_string()]
            } else {
                vec![format!(
                    "internal v8-runner runtime adapter exited with status {}",
                    output.status
                )]
            },
            errors: if ok {
                Vec::new()
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
            artifacts: Vec::new(),
            stdout: Some(stdout),
            stderr: Some(stderr),
            command: Some(command),
        })
    }
}

impl<'a> Default for RuntimeAdapter<'a> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> CodeSearchAdapter<'a> {
    pub fn new() -> Self {
        Self {
            bsl_runner: &SYSTEM_PROCESS_RUNNER,
            grep_runner: &SYSTEM_PROCESS_RUNNER,
            index_runner: &SYSTEM_INDEX_RUNNER,
            use_workspace_service: true,
        }
    }

    #[cfg(test)]
    pub fn with_runners(
        analyzer_runner: &'a dyn ProcessRunner,
        index_runner: &'a dyn IndexRunner,
    ) -> Self {
        Self {
            bsl_runner: analyzer_runner,
            grep_runner: analyzer_runner,
            index_runner,
            use_workspace_service: false,
        }
    }

    #[cfg(test)]
    pub fn with_backend_runners(
        bsl_runner: &'a dyn ProcessRunner,
        grep_runner: &'a dyn ProcessRunner,
        index_runner: &'a dyn IndexRunner,
    ) -> Self {
        Self {
            bsl_runner,
            grep_runner,
            index_runner,
            use_workspace_service: false,
        }
    }

    pub fn invoke(
        &self,
        tool_name: &str,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
        dry_run: bool,
    ) -> Result<AdapterOutcome, String> {
        if dry_run {
            return Ok(AdapterOutcome {
                ok: true,
                summary: format!("dry run: {tool_name} would use typed code search"),
                changes: Vec::new(),
                warnings: Vec::new(),
                errors: Vec::new(),
                artifacts: Vec::new(),
                stdout: None,
                stderr: None,
                command: None,
            });
        }

        let sections = [
            self.bsl_search(context, args),
            self.rlm_search(context, args),
            self.git_grep_search(tool_name, args, context),
        ];
        let ok = sections.iter().any(|section| section.ok);
        let warnings = sections
            .iter()
            .flat_map(|section| section.diagnostics.clone())
            .collect::<Vec<_>>();
        let errors = if ok { Vec::new() } else { warnings.clone() };
        let artifacts = sections
            .iter()
            .flat_map(|section| section.artifacts.clone())
            .collect::<Vec<_>>();
        let stdout = sections
            .iter()
            .map(|section| section.section.clone())
            .collect::<Vec<_>>()
            .join("\n\n");
        Ok(AdapterOutcome {
            ok,
            summary: if ok {
                format!("{tool_name} completed through typed code search")
            } else {
                format!("{tool_name} failed through typed code search")
            },
            changes: Vec::new(),
            warnings,
            errors,
            artifacts,
            stdout: Some(stdout),
            stderr: None,
            command: None,
        })
    }

    fn bsl_search(
        &self,
        context: &WorkspaceContext,
        args: &Map<String, Value>,
    ) -> SearchBackendResult {
        let plugin_root = match find_plugin_root(&context.cwd) {
            Some(plugin_root) => plugin_root,
            None => {
                return unavailable_backend(
                    "bsl-analyzer",
                    "could not locate Unica plugin root for bsl-analyzer search",
                )
            }
        };
        let bundled_tool = match resolve_bundled_tool(&plugin_root, "bsl-analyzer", true) {
            Ok(bundled_tool) => bundled_tool,
            Err(error) => return unavailable_backend("bsl-analyzer", error),
        };
        let mut process_args = vec!["search".to_string()];
        match cli_args(args, false) {
            Ok(args) => process_args.extend(args),
            Err(error) => return failed_backend("bsl-analyzer", error),
        }
        match self.bsl_runner.run(&ProcessCommand {
            program: bundled_tool.program,
            args: process_args,
            cwd: context.cwd.clone(),
            timeout: Some(DEFAULT_PROCESS_TIMEOUT),
        }) {
            Ok(output) if output.status_success => {
                let body = non_empty_body(&output.stdout, "No bsl-analyzer matches.");
                successful_backend("bsl-analyzer", body, Vec::new())
            }
            Ok(output) => {
                let reason = failed_process_reason("bsl-analyzer search", &output);
                failed_backend("bsl-analyzer", reason)
            }
            Err(error) => unavailable_backend("bsl-analyzer", error),
        }
    }

    fn rlm_search(
        &self,
        context: &WorkspaceContext,
        args: &Map<String, Value>,
    ) -> SearchBackendResult {
        match self.rlm_readiness(context, args) {
            IndexReadiness::Ready { db_path } => match search_rlm_index(&db_path, args) {
                Ok(Some(rlm_stdout)) => {
                    successful_backend("rlm", rlm_stdout, vec![db_path.display().to_string()])
                }
                Ok(None) => unavailable_backend("rlm", "missing required `query` argument"),
                Err(error) => failed_backend("rlm", error),
            },
            other => unavailable_backend("rlm", readiness_warning(other)),
        }
    }

    fn git_grep_search(
        &self,
        tool_name: &str,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
    ) -> SearchBackendResult {
        let grep_adapter = CodeNavigationAdapter {
            index_runner: self.index_runner,
            grep_runner: self.grep_runner,
            use_workspace_service: self.use_workspace_service,
        };
        match grep_adapter.grep(tool_name, args, context) {
            Ok(grep) if grep.ok => {
                let body = grep
                    .stdout
                    .as_deref()
                    .map(|stdout| section_body(stdout, "git-grep"))
                    .filter(|body| !body.trim().is_empty())
                    .unwrap_or_else(|| "No git grep matches.".to_string());
                let mut result = successful_backend("git grep", body, Vec::new());
                result.diagnostics.extend(grep.warnings);
                result
            }
            Ok(grep) => {
                let reason = if grep.errors.is_empty() {
                    "git grep failed".to_string()
                } else {
                    grep.errors.join("; ")
                };
                failed_backend("git grep", reason)
            }
            Err(error) => unavailable_backend("git grep", error),
        }
    }

    fn rlm_readiness(
        &self,
        context: &WorkspaceContext,
        args: &Map<String, Value>,
    ) -> IndexReadiness {
        if self.use_workspace_service {
            match resolve_source_dir(context, args).and_then(|source_dir| {
                WorkspaceServiceManager::new().rlm_readiness(context, &source_dir, args)
            }) {
                Ok(readiness) => readiness,
                Err(error) => IndexReadiness::Unavailable(error),
            }
        } else {
            WorkspaceIndexService::with_runner(self.index_runner).ready_index(context, args)
        }
    }
}

impl Default for CodeSearchAdapter<'_> {
    fn default() -> Self {
        Self::new()
    }
}

fn format_section(name: &str, text: &str) -> String {
    let body = text.trim_end();
    if body.is_empty() {
        format!("=== {name} ===")
    } else {
        format!("=== {name} ===\n{body}")
    }
}

fn successful_backend(
    name: &'static str,
    body: impl Into<String>,
    artifacts: Vec<String>,
) -> SearchBackendResult {
    SearchBackendResult {
        section: format_section(name, &body.into()),
        ok: true,
        diagnostics: Vec::new(),
        artifacts,
    }
}

fn unavailable_backend(name: &'static str, reason: impl Into<String>) -> SearchBackendResult {
    let reason = reason.into();
    SearchBackendResult {
        section: format_section(name, &format!("unavailable: {reason}")),
        ok: false,
        diagnostics: vec![format!("{name} unavailable: {reason}")],
        artifacts: Vec::new(),
    }
}

fn failed_backend(name: &'static str, reason: impl Into<String>) -> SearchBackendResult {
    let reason = reason.into();
    SearchBackendResult {
        section: format_section(name, &format!("failed: {reason}")),
        ok: false,
        diagnostics: vec![format!("{name} failed: {reason}")],
        artifacts: Vec::new(),
    }
}

fn non_empty_body(stdout: &str, empty_message: &'static str) -> String {
    let body = stdout.trim();
    if body.is_empty() {
        empty_message.to_string()
    } else {
        body.to_string()
    }
}

fn section_body(stdout: &str, section_name: &str) -> String {
    let expected_header = format!("=== {section_name} ===");
    let text = stdout.trim();
    text.strip_prefix(&expected_header)
        .map(str::trim)
        .filter(|body| !body.is_empty())
        .unwrap_or(text)
        .to_string()
}

fn failed_process_reason(label: &str, output: &ProcessOutput) -> String {
    if output.timed_out {
        return process_timeout_error(label, Some(DEFAULT_PROCESS_TIMEOUT));
    }
    let stderr = output.stderr.trim();
    if stderr.is_empty() {
        format!("{label} exited with status {}", output.status)
    } else {
        stderr.to_string()
    }
}

fn process_exit_code_is(status: &str, code: i32) -> bool {
    let status = status.trim();
    status == code.to_string() || status.ends_with(&format!(": {code}"))
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

fn redact_sensitive_text(text: &str) -> String {
    let chars = text.chars().collect::<Vec<_>>();
    let mut output = String::with_capacity(text.len());
    let mut index = 0;
    while index < chars.len() {
        if secret_key_char(chars[index]) {
            let key_start = index;
            index += 1;
            while index < chars.len() && secret_key_char(chars[index]) {
                index += 1;
            }
            let key_end = index;
            let mut separator = index;
            while separator < chars.len() && chars[separator].is_whitespace() {
                separator += 1;
            }
            if separator < chars.len() && matches!(chars[separator], '=' | ':') {
                let mut value_start = separator + 1;
                while value_start < chars.len() && chars[value_start].is_whitespace() {
                    value_start += 1;
                }
                let key = chars[key_start..key_end].iter().collect::<String>();
                if is_secret_key(&key) {
                    for ch in &chars[key_start..value_start] {
                        output.push(*ch);
                    }
                    output.push_str("<redacted>");
                    index = value_start;
                    while index < chars.len() && !secret_value_delimiter(chars[index]) {
                        index += 1;
                    }
                    continue;
                }
            }
            for ch in &chars[key_start..key_end] {
                output.push(*ch);
            }
            continue;
        }
        output.push(chars[index]);
        index += 1;
    }
    output
}

fn secret_key_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-')
}

fn secret_value_delimiter(ch: char) -> bool {
    matches!(ch, ';' | '&' | ',' | '\n' | '\r' | '}')
}

fn search_rlm_index(
    db_path: &PathBuf,
    args: &Map<String, Value>,
) -> Result<Option<String>, String> {
    let Some(query) = args.get("query").and_then(Value::as_str) else {
        return Ok(None);
    };
    let query = query.trim();
    if query.is_empty() {
        return Ok(None);
    }
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(20);
    let conn = Connection::open(db_path).map_err(|error| error.to_string())?;
    let fts_query = format!("\"{}\"", query.replace('"', "\"\""));
    let mut stmt = conn
        .prepare(
            "SELECT \
               m.name, m.type, m.is_export, m.line, m.end_line, m.params, \
               mod.rel_path AS module_path, mod.object_name, methods_fts.rank \
             FROM methods_fts \
             JOIN methods m ON m.id = methods_fts.rowid \
             JOIN modules mod ON mod.id = m.module_id \
             WHERE methods_fts MATCH ? \
             ORDER BY methods_fts.rank \
             LIMIT ?",
        )
        .map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map(params![fts_query, limit as i64], |row| {
            let method_type: String = row.get(1)?;
            let is_export: i64 = row.get(2)?;
            let params: Option<String> = row.get(5)?;
            let params = params.unwrap_or_default();
            let signature_params = format!("({})", params.trim());
            Ok(format!(
                "- {}:{} {} {}{}{}",
                row.get::<_, String>(6)?,
                row.get::<_, i64>(3)?,
                method_type,
                row.get::<_, String>(0)?,
                signature_params,
                if is_export != 0 { " export" } else { "" }
            ))
        })
        .map_err(|error| error.to_string())?;

    let mut lines = Vec::new();
    for row in rows {
        lines.push(row.map_err(|error| error.to_string())?);
    }
    if lines.is_empty() {
        Ok(Some("No RLM method matches.".to_string()))
    } else {
        Ok(Some(lines.join("\n")))
    }
}

impl<'a> CodeNavigationAdapter<'a> {
    pub fn new() -> Self {
        Self {
            index_runner: &SYSTEM_INDEX_RUNNER,
            grep_runner: &SYSTEM_PROCESS_RUNNER,
            use_workspace_service: true,
        }
    }

    #[cfg(test)]
    pub fn with_runners(
        index_runner: &'a dyn IndexRunner,
        grep_runner: &'a dyn ProcessRunner,
    ) -> Self {
        Self {
            index_runner,
            grep_runner,
            use_workspace_service: false,
        }
    }

    pub fn invoke(
        &self,
        tool_name: &str,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
        dry_run: bool,
    ) -> Result<AdapterOutcome, String> {
        if dry_run {
            return Ok(AdapterOutcome {
                ok: true,
                summary: format!("dry run: {tool_name} would use typed code navigation"),
                changes: Vec::new(),
                warnings: Vec::new(),
                errors: Vec::new(),
                artifacts: Vec::new(),
                stdout: None,
                stderr: None,
                command: None,
            });
        }

        match tool_name {
            "unica.code.definition" => self.definition(tool_name, args, context),
            "unica.code.outline" => self.outline(tool_name, args, context),
            "unica.code.grep" => self.grep(tool_name, args, context),
            "unica.meta.profile" => self.meta_profile(tool_name, args, context),
            _ => Err(format!("unsupported code navigation tool: {tool_name}")),
        }
    }

    fn definition(
        &self,
        tool_name: &str,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
    ) -> Result<AdapterOutcome, String> {
        let readiness = self.rlm_readiness(context, args);
        let db_path = match readiness {
            IndexReadiness::Ready { db_path } => db_path,
            other => return Ok(index_unavailable_outcome(tool_name, other)),
        };
        let body = find_definitions(&db_path, args)?;
        Ok(AdapterOutcome {
            ok: true,
            summary: format!("{tool_name} completed through internal RLM index"),
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: Vec::new(),
            artifacts: vec![db_path.display().to_string()],
            stdout: Some(format_section("rlm-definition", &body)),
            stderr: None,
            command: None,
        })
    }

    fn outline(
        &self,
        tool_name: &str,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
    ) -> Result<AdapterOutcome, String> {
        let candidates = index_path_candidates(context, args, "path")?;
        let readiness = self.rlm_readiness(context, args);
        let db_path = match readiness {
            IndexReadiness::Ready { db_path } => db_path,
            other => return Ok(index_unavailable_outcome(tool_name, other)),
        };
        let include_methods = args
            .get("includeMethods")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let body = module_outline(&db_path, &candidates, include_methods)?;
        Ok(AdapterOutcome {
            ok: true,
            summary: format!("{tool_name} completed through internal RLM index"),
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: Vec::new(),
            artifacts: vec![db_path.display().to_string()],
            stdout: Some(format_section("rlm-outline", &body)),
            stderr: None,
            command: None,
        })
    }

    fn grep(
        &self,
        tool_name: &str,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
    ) -> Result<AdapterOutcome, String> {
        let query = required_string(args, "query")?;
        let mode = args.get("mode").and_then(Value::as_str).unwrap_or("lines");
        if !matches!(mode, "lines" | "files") {
            return Err(format!(
                "{tool_name} argument `mode` must be one of: lines, files"
            ));
        }
        let limit = read_limit(args, 200);

        let mut git_args = vec!["grep".to_string()];
        if mode == "files" {
            git_args.push("--name-only".to_string());
        } else {
            git_args.push("-n".to_string());
            git_args.push("-m".to_string());
            git_args.push(limit.to_string());
        }
        if !args.get("regex").and_then(Value::as_bool).unwrap_or(false) {
            git_args.push("-F".to_string());
        }
        if args
            .get("ignoreCase")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            git_args.push("-i".to_string());
        }
        git_args.push("-e".to_string());
        git_args.push(query.to_string());

        let pathspecs = grep_pathspecs(context, args)?;
        if !pathspecs.is_empty() {
            git_args.push("--".to_string());
            git_args.extend(pathspecs);
        }

        let output = self.grep_runner.run(&ProcessCommand {
            program: PathBuf::from("git"),
            args: git_args.clone(),
            cwd: context.workspace_root.clone(),
            timeout: Some(DEFAULT_PROCESS_TIMEOUT),
        })?;
        let body = grep_body(&output.stdout, mode, limit);
        let no_matches = body.is_empty()
            && !output.status_success
            && output.stderr.trim().is_empty()
            && !output.timed_out
            && process_exit_code_is(&output.status, 1);
        let partial_matches = output.timed_out && !body.is_empty();
        if !output.status_success && !no_matches && !partial_matches {
            let error = output.stderr.trim();
            let error = if error.is_empty() {
                format!("git grep exited with status {}", output.status)
            } else {
                error.to_string()
            };
            return Ok(AdapterOutcome {
                ok: false,
                summary: format!("{tool_name} failed through git grep"),
                changes: Vec::new(),
                warnings: vec![format!("git grep exited with status {}", output.status)],
                errors: vec![error],
                artifacts: Vec::new(),
                stdout: Some(format_section("git-grep", &body)),
                stderr: Some(output.stderr),
                command: Some(std::iter::once("git".to_string()).chain(git_args).collect()),
            });
        }

        let stdout = if no_matches {
            "No git grep matches.".to_string()
        } else {
            body
        };
        Ok(AdapterOutcome {
            ok: true,
            summary: format!("{tool_name} completed through git grep"),
            changes: Vec::new(),
            warnings: if output.timed_out {
                vec!["git grep timed out".to_string()]
            } else {
                Vec::new()
            },
            errors: Vec::new(),
            artifacts: Vec::new(),
            stdout: Some(format_section("git-grep", &stdout)),
            stderr: if output.stderr.trim().is_empty() {
                None
            } else {
                Some(output.stderr)
            },
            command: Some(std::iter::once("git".to_string()).chain(git_args).collect()),
        })
    }

    fn meta_profile(
        &self,
        tool_name: &str,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
    ) -> Result<AdapterOutcome, String> {
        let readiness = self.rlm_readiness(context, args);
        let db_path = match readiness {
            IndexReadiness::Ready { db_path } => db_path,
            other => return Ok(index_unavailable_outcome(tool_name, other)),
        };
        match metadata_profile(&db_path, args) {
            Ok(body) => Ok(AdapterOutcome {
                ok: true,
                summary: format!("{tool_name} completed through internal RLM metadata index"),
                changes: Vec::new(),
                warnings: Vec::new(),
                errors: Vec::new(),
                artifacts: vec![db_path.display().to_string()],
                stdout: Some(format_section("rlm-meta-profile", &body)),
                stderr: None,
                command: None,
            }),
            Err(error) if is_metadata_profile_schema_error(&error) => Ok(
                metadata_profile_unavailable_outcome(tool_name, &db_path, &error),
            ),
            Err(error) => Err(error),
        }
    }

    fn rlm_readiness(
        &self,
        context: &WorkspaceContext,
        args: &Map<String, Value>,
    ) -> IndexReadiness {
        if self.use_workspace_service {
            match resolve_source_dir(context, args).and_then(|source_dir| {
                WorkspaceServiceManager::new().rlm_readiness(context, &source_dir, args)
            }) {
                Ok(readiness) => readiness,
                Err(error) => IndexReadiness::Unavailable(error),
            }
        } else {
            WorkspaceIndexService::with_runner(self.index_runner).ready_index(context, args)
        }
    }
}

impl Default for CodeNavigationAdapter<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> BslAnalyzerMcpAdapter<'a> {
    pub fn new() -> Self {
        Self {
            runner: &SYSTEM_BSL_MCP_RUNNER,
        }
    }

    #[cfg(test)]
    pub fn with_runner(runner: &'a dyn BslMcpRunner) -> Self {
        Self { runner }
    }

    pub fn invoke(
        &self,
        tool_name: &str,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
        dry_run: bool,
    ) -> Result<AdapterOutcome, String> {
        if tool_name == "unica.code.diagnostics" && diagnostics_mode(args) == "analyze" {
            let cli_args = diagnostics_analyze_args(args);
            return CliAdapter::new("bsl-analyzer", &["analyze"], "code analysis")
                .invoke(tool_name, &cli_args, context, dry_run, false);
        }

        let plugin_root = find_plugin_root(&context.cwd).ok_or_else(|| {
            "could not locate Unica plugin root for bsl-analyzer MCP adapter lookup".to_string()
        })?;
        let source_dir = resolve_source_dir(context, args)?;
        let (remote_tool, tool_args) = bsl_mcp_tool_request(tool_name, args)?;
        let bundled_tool = resolve_bundled_tool(&plugin_root, "bsl-analyzer", !dry_run)?;
        let command = bsl_mcp_command(&source_dir, context, remote_tool, tool_args);
        let mut reported_command = vec![bundled_tool.program.display().to_string()];
        reported_command.extend(command.args.clone());

        if dry_run {
            return Ok(AdapterOutcome {
                ok: true,
                summary: format!("dry run: {tool_name} would call typed bsl-analyzer MCP adapter"),
                changes: Vec::new(),
                warnings: bundled_tool.warnings,
                errors: Vec::new(),
                artifacts: vec![source_dir.display().to_string()],
                stdout: None,
                stderr: None,
                command: Some(reported_command),
            });
        }

        let output = self.runner.call(&command)?;
        let section = if command.tool_name == "graph" {
            "bsl-analyzer-graph"
        } else {
            "bsl-analyzer-diagnostics"
        };
        Ok(AdapterOutcome {
            ok: true,
            summary: format!("{tool_name} completed through typed bsl-analyzer MCP adapter"),
            changes: Vec::new(),
            warnings: bsl_mcp_readiness_warnings(&output.result_text),
            errors: Vec::new(),
            artifacts: vec![
                source_dir.display().to_string(),
                command.tool_name.to_string(),
            ],
            stdout: Some(format_section(section, &output.result_text)),
            stderr: if output.stderr.trim().is_empty() {
                None
            } else {
                Some(output.stderr)
            },
            command: Some(reported_command),
        })
    }
}

impl Default for BslAnalyzerMcpAdapter<'_> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
struct ModuleRecord {
    id: i64,
    rel_path: String,
    category: Option<String>,
    object_name: Option<String>,
    module_type: Option<String>,
}

#[derive(Debug, Clone)]
struct ProfileIdentity {
    category: Option<String>,
    object_name: String,
}

impl ProfileIdentity {
    fn object_ref(&self) -> String {
        match self.category.as_deref().filter(|value| !value.is_empty()) {
            Some(category) => format!("{category}.{}", self.object_name),
            None => self.object_name.clone(),
        }
    }
}

fn find_definitions(db_path: &PathBuf, args: &Map<String, Value>) -> Result<String, String> {
    let name = required_string(args, "name")?;
    let limit = read_limit(args, 50);
    let conn = Connection::open(db_path).map_err(|error| error.to_string())?;
    let mut lines = Vec::new();
    if let Some(module_hint) = args.get("moduleHint").and_then(Value::as_str) {
        let hint = format!("%{}%", module_hint.trim());
        let mut stmt = conn
            .prepare(
                "SELECT \
                   m.name, m.type, m.is_export, m.line, m.end_line, m.params, \
                   mod.rel_path, mod.category, mod.object_name, mod.module_type \
                 FROM methods m \
                 JOIN modules mod ON mod.id = m.module_id \
                 WHERE m.name = ? COLLATE NOCASE \
                   AND (mod.rel_path LIKE ? OR mod.object_name LIKE ?) \
                 ORDER BY m.is_export DESC, mod.rel_path, m.line \
                 LIMIT ?",
            )
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map(params![name, hint, hint, limit as i64], definition_line)
            .map_err(|error| error.to_string())?;
        for row in rows {
            lines.push(row.map_err(|error| error.to_string())?);
        }
    } else {
        let mut stmt = conn
            .prepare(
                "SELECT \
                   m.name, m.type, m.is_export, m.line, m.end_line, m.params, \
                   mod.rel_path, mod.category, mod.object_name, mod.module_type \
                 FROM methods m \
                 JOIN modules mod ON mod.id = m.module_id \
                 WHERE m.name = ? COLLATE NOCASE \
                 ORDER BY m.is_export DESC, mod.rel_path, m.line \
                 LIMIT ?",
            )
            .map_err(|error| error.to_string())?;
        let rows = stmt
            .query_map(params![name, limit as i64], definition_line)
            .map_err(|error| error.to_string())?;
        for row in rows {
            lines.push(row.map_err(|error| error.to_string())?);
        }
    }

    if lines.is_empty() {
        Ok(format!("No RLM definitions found for `{name}`."))
    } else {
        Ok(lines.join("\n"))
    }
}

fn definition_line(row: &Row<'_>) -> rusqlite::Result<String> {
    let method_type: String = row.get(1)?;
    let is_export: i64 = row.get(2)?;
    let params: Option<String> = row.get(5)?;
    let category: Option<String> = row.get(7)?;
    let object_name: Option<String> = row.get(8)?;
    let module_type: Option<String> = row.get(9)?;
    let mut meta = Vec::new();
    if let Some(category) = category.filter(|value| !value.is_empty()) {
        meta.push(format!("category={category}"));
    }
    if let Some(object_name) = object_name.filter(|value| !value.is_empty()) {
        meta.push(format!("object={object_name}"));
    }
    if let Some(module_type) = module_type.filter(|value| !value.is_empty()) {
        meta.push(format!("moduleType={module_type}"));
    }
    let signature_params = format!("({})", params.unwrap_or_default().trim());
    let suffix = if meta.is_empty() {
        String::new()
    } else {
        format!(" [{}]", meta.join(", "))
    };
    Ok(format!(
        "- {}:{} {} {}{}{}{}",
        row.get::<_, String>(6)?,
        row.get::<_, i64>(3)?,
        method_type,
        row.get::<_, String>(0)?,
        signature_params,
        if is_export != 0 { " export" } else { "" },
        suffix
    ))
}

fn module_outline(
    db_path: &PathBuf,
    candidates: &[String],
    include_methods: bool,
) -> Result<String, String> {
    let conn = Connection::open(db_path).map_err(|error| error.to_string())?;
    let mut module = None;
    for candidate in candidates {
        module = conn
            .query_row(
                "SELECT id, rel_path, category, object_name, module_type \
                 FROM modules WHERE rel_path = ?",
                params![candidate],
                |row| {
                    Ok(ModuleRecord {
                        id: row.get(0)?,
                        rel_path: row.get(1)?,
                        category: row.get(2)?,
                        object_name: row.get(3)?,
                        module_type: row.get(4)?,
                    })
                },
            )
            .optional()
            .map_err(|error| error.to_string())?;
        if module.is_some() {
            break;
        }
    }
    let Some(module) = module else {
        return Ok(format!(
            "No RLM module found for path candidates: {}",
            candidates.join(", ")
        ));
    };

    let mut lines = vec![format!("module: {}", module.rel_path)];
    if let Some(object_name) = module.object_name.filter(|value| !value.is_empty()) {
        lines.push(format!("object: {object_name}"));
    }
    if let Some(category) = module.category.filter(|value| !value.is_empty()) {
        lines.push(format!("category: {category}"));
    }
    if let Some(module_type) = module.module_type.filter(|value| !value.is_empty()) {
        lines.push(format!("moduleType: {module_type}"));
    }

    let header = conn
        .query_row(
            "SELECT header_comment FROM module_headers WHERE module_id = ?",
            params![module.id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    if let Some(header) = header.filter(|value| !value.trim().is_empty()) {
        lines.push(format!("header: {}", header.trim()));
    }

    let mut region_stmt = conn
        .prepare("SELECT name, line, end_line FROM regions WHERE module_id = ? ORDER BY line")
        .map_err(|error| error.to_string())?;
    let regions = region_stmt
        .query_map(params![module.id], |row| {
            let name: String = row.get(0)?;
            let line: i64 = row.get(1)?;
            let end_line: Option<i64> = row.get(2)?;
            Ok(match end_line {
                Some(end_line) => format!("region {name}: {line}-{end_line}"),
                None => format!("region {name}: {line}-?"),
            })
        })
        .map_err(|error| error.to_string())?;
    for region in regions {
        lines.push(region.map_err(|error| error.to_string())?);
    }

    if include_methods {
        let mut method_stmt = conn
            .prepare(
                "SELECT name, type, is_export, params, line, end_line \
                 FROM methods WHERE module_id = ? ORDER BY line",
            )
            .map_err(|error| error.to_string())?;
        let methods = method_stmt
            .query_map(params![module.id], |row| {
                let name: String = row.get(0)?;
                let method_type: String = row.get(1)?;
                let is_export: i64 = row.get(2)?;
                let params: Option<String> = row.get(3)?;
                let line: i64 = row.get(4)?;
                let end_line: Option<i64> = row.get(5)?;
                let range = match end_line {
                    Some(end_line) => format!("{line}-{end_line}"),
                    None => format!("{line}-?"),
                };
                let params = params.unwrap_or_default();
                Ok(format!(
                    "{} {}({}){} at {}",
                    method_type,
                    name,
                    params.trim(),
                    if is_export != 0 { " export" } else { "" },
                    range
                ))
            })
            .map_err(|error| error.to_string())?;
        for method in methods {
            lines.push(method.map_err(|error| error.to_string())?);
        }
    }

    Ok(lines.join("\n"))
}

fn metadata_profile(db_path: &PathBuf, args: &Map<String, Value>) -> Result<String, String> {
    let requested_name = required_string(args, "name")?;
    let limit = read_limit(args, 20);
    let sections = profile_sections(args)?;
    let conn = Connection::open(db_path).map_err(|error| error.to_string())?;
    let identity = resolve_profile_identity(&conn, requested_name)?;

    let mut lines = vec![format!("object: {}", identity.object_ref())];
    if let Some(category) = identity
        .category
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        lines.push(format!("category: {category}"));
    }
    lines.push(format!("name: {}", identity.object_name));

    for section in sections {
        let items = match section.as_str() {
            "structure" => profile_structure(&conn, &identity)?,
            "modules" => profile_modules(&conn, &identity)?,
            "roles" => profile_roles(&conn, &identity)?,
            "subscriptions" => profile_subscriptions(&conn, &identity)?,
            "functionalOptions" => profile_functional_options(&conn, &identity)?,
            "predefinedItems" => profile_predefined_items(&conn, &identity)?,
            other => return Err(format!("unsupported metadata profile section: {other}")),
        };
        lines.extend(format_profile_section(&section, items, limit));
    }

    Ok(lines.join("\n"))
}

fn profile_sections(args: &Map<String, Value>) -> Result<Vec<String>, String> {
    let Some(raw_sections) = args.get("sections") else {
        return Ok(vec![
            "structure".to_string(),
            "modules".to_string(),
            "roles".to_string(),
            "subscriptions".to_string(),
            "functionalOptions".to_string(),
        ]);
    };
    let Some(items) = raw_sections.as_array() else {
        return Err("unica.meta.profile argument `sections` must be array".to_string());
    };
    let mut sections = Vec::new();
    for item in items {
        let Some(section) = item.as_str() else {
            return Err("unica.meta.profile argument `sections` must contain strings".to_string());
        };
        match section {
            "structure" | "modules" | "roles" | "subscriptions" | "functionalOptions"
            | "predefinedItems" => sections.push(section.to_string()),
            other => return Err(format!("unsupported metadata profile section: {other}")),
        }
    }
    Ok(sections)
}

fn resolve_profile_identity(
    conn: &Connection,
    requested_name: &str,
) -> Result<ProfileIdentity, String> {
    let (category_hint, object_name) = split_profile_name(requested_name);
    if let Some(identity) = query_profile_identity(
        conn,
        "SELECT DISTINCT category, object_name FROM modules \
         WHERE object_name = ? COLLATE NOCASE \
           AND (? IS NULL OR category = ? COLLATE NOCASE) \
         ORDER BY category, object_name LIMIT 1",
        category_hint.as_deref(),
        &object_name,
    )? {
        return Ok(identity);
    }
    if let Some(identity) = query_profile_identity(
        conn,
        "SELECT DISTINCT category, object_name FROM object_attributes \
         WHERE object_name = ? COLLATE NOCASE \
           AND (? IS NULL OR category = ? COLLATE NOCASE) \
         ORDER BY category, object_name LIMIT 1",
        category_hint.as_deref(),
        &object_name,
    )? {
        return Ok(identity);
    }
    if let Some(category) = category_hint {
        Ok(ProfileIdentity {
            category: Some(category),
            object_name,
        })
    } else {
        Err(format!(
            "No RLM metadata object found for `{requested_name}`."
        ))
    }
}

fn query_profile_identity(
    conn: &Connection,
    sql: &str,
    category_hint: Option<&str>,
    object_name: &str,
) -> Result<Option<ProfileIdentity>, String> {
    conn.query_row(
        sql,
        params![object_name, category_hint, category_hint],
        |row| {
            Ok(ProfileIdentity {
                category: row.get(0)?,
                object_name: row.get(1)?,
            })
        },
    )
    .optional()
    .map_err(|error| error.to_string())
}

fn split_profile_name(raw: &str) -> (Option<String>, String) {
    let trimmed = raw.trim();
    let Some((prefix, name)) = trimmed.split_once('.') else {
        return (None, trimmed.to_string());
    };
    let category = match prefix {
        "Document" | "Документ" => "Document",
        "Catalog" | "Справочник" => "Catalog",
        "CommonModule" | "CommonModules" | "ОбщийМодуль" | "ОбщиеМодули" => {
            "CommonModule"
        }
        "InformationRegister" | "РегистрСведений" => "InformationRegister",
        "AccumulationRegister" | "РегистрНакопления" => "AccumulationRegister",
        "Enum" | "Перечисление" => "Enum",
        other => other,
    };
    (Some(category.to_string()), name.trim().to_string())
}

fn format_profile_section(section: &str, items: Vec<String>, limit: usize) -> Vec<String> {
    let total = items.len();
    let returned = total.min(limit);
    let status = if total == 0 { "empty" } else { "ok" };
    let mut lines = vec![format!(
        "section {section}: {status} total={total} returned={returned}"
    )];
    lines.extend(items.into_iter().take(limit));
    lines
}

fn category_filter(identity: &ProfileIdentity) -> Option<&str> {
    identity
        .category
        .as_deref()
        .filter(|value| !value.is_empty())
}

fn profile_structure(conn: &Connection, identity: &ProfileIdentity) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT attr_kind, attr_name, attr_type, ts_name \
             FROM object_attributes \
             WHERE object_name = ? COLLATE NOCASE \
               AND (? IS NULL OR category = ? COLLATE NOCASE) \
             ORDER BY attr_kind, ts_name, attr_name",
        )
        .map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map(
            params![
                identity.object_name,
                category_filter(identity),
                category_filter(identity)
            ],
            |row| {
                let kind: String = row.get(0)?;
                let name: String = row.get(1)?;
                let attr_type: Option<String> = row.get(2)?;
                let ts_name: Option<String> = row.get(3)?;
                let table = ts_name
                    .filter(|value| !value.is_empty())
                    .map(|value| format!(" table={value}"))
                    .unwrap_or_default();
                let type_text = attr_type
                    .filter(|value| !value.is_empty())
                    .map(|value| format!(" type={value}"))
                    .unwrap_or_default();
                Ok(format!("- {kind} {name}{type_text}{table}"))
            },
        )
        .map_err(|error| error.to_string())?;
    collect_rows(rows)
}

fn profile_modules(conn: &Connection, identity: &ProfileIdentity) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT rel_path, module_type \
             FROM modules \
             WHERE object_name = ? COLLATE NOCASE \
               AND (? IS NULL OR category = ? COLLATE NOCASE) \
             ORDER BY rel_path",
        )
        .map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map(
            params![
                identity.object_name,
                category_filter(identity),
                category_filter(identity)
            ],
            |row| {
                let rel_path: String = row.get(0)?;
                let module_type: Option<String> = row.get(1)?;
                let suffix = module_type
                    .filter(|value| !value.is_empty())
                    .map(|value| format!(" {value}"))
                    .unwrap_or_default();
                Ok(format!("- module {rel_path}{suffix}"))
            },
        )
        .map_err(|error| error.to_string())?;
    collect_rows(rows)
}

fn profile_roles(conn: &Connection, identity: &ProfileIdentity) -> Result<Vec<String>, String> {
    let object_ref = identity.object_ref();
    let mut stmt = conn
        .prepare(
            "SELECT role_name, GROUP_CONCAT(right_name, ', ') \
             FROM ( \
               SELECT role_name, right_name, id FROM role_rights \
               WHERE object_name = ? COLLATE NOCASE OR object_name = ? COLLATE NOCASE \
               ORDER BY role_name, id \
             ) \
             GROUP BY role_name ORDER BY role_name",
        )
        .map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map(params![object_ref, identity.object_name], |row| {
            let role_name: String = row.get(0)?;
            let rights: Option<String> = row.get(1)?;
            Ok(format!(
                "- role {role_name} rights={}",
                rights.unwrap_or_default()
            ))
        })
        .map_err(|error| error.to_string())?;
    collect_rows(rows)
}

fn profile_subscriptions(
    conn: &Connection,
    identity: &ProfileIdentity,
) -> Result<Vec<String>, String> {
    let object_ref = identity.object_ref();
    let like_ref = format!("%{object_ref}%");
    let like_name = format!("%{}%", identity.object_name);
    let mut stmt = conn
        .prepare(
            "SELECT name, event, handler_module, handler_procedure \
             FROM event_subscriptions \
             WHERE source_types LIKE ? OR source_types LIKE ? OR name = ? COLLATE NOCASE \
             ORDER BY name",
        )
        .map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map(params![like_ref, like_name, identity.object_name], |row| {
            let name: String = row.get(0)?;
            let event: Option<String> = row.get(1)?;
            let handler_module: Option<String> = row.get(2)?;
            let handler_procedure: Option<String> = row.get(3)?;
            let handler = match (handler_module, handler_procedure) {
                (Some(module), Some(procedure)) if !module.is_empty() && !procedure.is_empty() => {
                    format!("{module}.{procedure}")
                }
                (Some(module), _) if !module.is_empty() => module,
                (_, Some(procedure)) if !procedure.is_empty() => procedure,
                _ => "<unknown>".to_string(),
            };
            Ok(format!(
                "- subscription {name} event={} handler={handler}",
                event.unwrap_or_default()
            ))
        })
        .map_err(|error| error.to_string())?;
    collect_rows(rows)
}

fn profile_functional_options(
    conn: &Connection,
    identity: &ProfileIdentity,
) -> Result<Vec<String>, String> {
    let object_ref = identity.object_ref();
    let like_ref = format!("%{object_ref}%");
    let like_name = format!("%{}%", identity.object_name);
    let mut stmt = conn
        .prepare(
            "SELECT name \
             FROM functional_options \
             WHERE location LIKE ? OR content LIKE ? OR location LIKE ? OR content LIKE ? \
             ORDER BY name",
        )
        .map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map(params![like_ref, like_ref, like_name, like_name], |row| {
            let name: String = row.get(0)?;
            Ok(format!("- option {name}"))
        })
        .map_err(|error| error.to_string())?;
    collect_rows(rows)
}

fn profile_predefined_items(
    conn: &Connection,
    identity: &ProfileIdentity,
) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT item_name, item_code \
             FROM predefined_items \
             WHERE object_name = ? COLLATE NOCASE \
               AND (? IS NULL OR category = ? COLLATE NOCASE) \
             ORDER BY item_name",
        )
        .map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map(
            params![
                identity.object_name,
                category_filter(identity),
                category_filter(identity)
            ],
            |row| {
                let item_name: String = row.get(0)?;
                let item_code: Option<String> = row.get(1)?;
                let code = item_code
                    .filter(|value| !value.is_empty())
                    .map(|value| format!(" code={value}"))
                    .unwrap_or_default();
                Ok(format!("- predefined {item_name}{code}"))
            },
        )
        .map_err(|error| error.to_string())?;
    collect_rows(rows)
}

fn collect_rows(
    rows: impl Iterator<Item = rusqlite::Result<String>>,
) -> Result<Vec<String>, String> {
    let mut lines = Vec::new();
    for row in rows {
        lines.push(row.map_err(|error| error.to_string())?);
    }
    Ok(lines)
}

fn is_metadata_profile_schema_error(error: &str) -> bool {
    error.contains("no such table:") || error.contains("no such column:")
}

fn metadata_profile_unavailable_outcome(
    tool_name: &str,
    db_path: &Path,
    error: &str,
) -> AdapterOutcome {
    let warning = format!(
        "RLM metadata profile schema is unavailable in the ready index: {error}; rebuild the RLM index with current tools."
    );
    AdapterOutcome {
        ok: true,
        summary: format!("{tool_name} could not read metadata profile from current RLM index"),
        changes: Vec::new(),
        warnings: vec![warning.clone()],
        errors: Vec::new(),
        artifacts: vec![db_path.display().to_string()],
        stdout: Some(format_section(
            "rlm-meta-profile",
            &format!("metadata profile unavailable\nwarning: {warning}"),
        )),
        stderr: None,
        command: None,
    }
}

fn index_unavailable_outcome(tool_name: &str, readiness: IndexReadiness) -> AdapterOutcome {
    AdapterOutcome {
        ok: true,
        summary: format!("{tool_name} could not read RLM index"),
        changes: Vec::new(),
        warnings: vec![readiness_warning(readiness)],
        errors: Vec::new(),
        artifacts: Vec::new(),
        stdout: None,
        stderr: None,
        command: None,
    }
}

fn readiness_warning(readiness: IndexReadiness) -> String {
    match readiness {
        IndexReadiness::Ready { .. } => "rlm index ready".to_string(),
        IndexReadiness::Missing => "rlm index unavailable: index is missing".to_string(),
        IndexReadiness::Stale | IndexReadiness::Building => "rlm index building".to_string(),
        IndexReadiness::Failed(error) | IndexReadiness::Unavailable(error) => {
            format!("rlm index unavailable: {error}")
        }
    }
}

fn required_string<'a>(args: &'a Map<String, Value>, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("missing required `{key}` argument"))
}

fn read_limit(args: &Map<String, Value>, default: usize) -> usize {
    args.get("limit")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn index_path_candidates(
    context: &WorkspaceContext,
    args: &Map<String, Value>,
    key: &str,
) -> Result<Vec<String>, String> {
    let raw = required_string(args, key)?;
    let mut candidates = BTreeSet::new();
    let rel = safe_workspace_rel(context, raw)?;
    if !rel.is_empty() {
        candidates.insert(rel.clone());
    }
    if let Some(source_dir) = args.get("sourceDir").and_then(Value::as_str) {
        let source_rel = safe_workspace_rel(context, source_dir)?;
        if rel_under(&rel, &source_rel) {
            let stripped = strip_rel_prefix(&rel, &source_rel);
            if !stripped.is_empty() {
                candidates.insert(stripped);
            }
        } else if !PathBuf::from(raw).is_absolute() {
            candidates.insert(join_rel(&source_rel, &rel));
        }
    }
    if candidates.is_empty() {
        candidates.insert(rel);
    }
    Ok(candidates.into_iter().collect())
}

fn grep_pathspecs(
    context: &WorkspaceContext,
    args: &Map<String, Value>,
) -> Result<Vec<String>, String> {
    let source_rel = args
        .get("sourceDir")
        .and_then(Value::as_str)
        .map(|value| safe_workspace_rel(context, value))
        .transpose()?;
    let include_rel = match args.get("path").and_then(Value::as_str) {
        Some(raw_path) => {
            let rel = safe_workspace_rel(context, raw_path)?;
            if let Some(source_rel) = &source_rel {
                if !PathBuf::from(raw_path).is_absolute() && !rel_under(&rel, source_rel) {
                    join_rel(source_rel, &rel)
                } else {
                    rel
                }
            } else {
                rel
            }
        }
        None => source_rel.unwrap_or_default(),
    };

    let mut pathspecs = Vec::new();
    let file_types = parse_file_types(args.get("fileTypes").and_then(Value::as_str))?;
    if file_types.is_empty() {
        if !include_rel.is_empty() {
            pathspecs.push(include_rel.clone());
        }
    } else {
        for extension in file_types {
            if include_rel.is_empty() {
                pathspecs.push(format!(":(glob)**/*.{extension}"));
            } else {
                pathspecs.push(format!(
                    ":(glob){}/**/*.{}",
                    include_rel.trim_end_matches('/'),
                    extension
                ));
            }
        }
    }

    if let Some(raw_exclude) = args.get("excludePath").and_then(Value::as_str) {
        let mut exclude_rel = safe_workspace_rel(context, raw_exclude)?;
        if let Some(source_dir) = args.get("sourceDir").and_then(Value::as_str) {
            let source_rel = safe_workspace_rel(context, source_dir)?;
            if !PathBuf::from(raw_exclude).is_absolute() && !rel_under(&exclude_rel, &source_rel) {
                exclude_rel = join_rel(&source_rel, &exclude_rel);
            }
        }
        if pathspecs.is_empty() {
            pathspecs.push(".".to_string());
        }
        pathspecs.push(format!(":(exclude){exclude_rel}"));
    }

    Ok(pathspecs)
}

fn parse_file_types(raw: Option<&str>) -> Result<Vec<String>, String> {
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };
    let mut types = Vec::new();
    for part in raw.split(|ch: char| ch == ',' || ch == ';' || ch.is_whitespace()) {
        let extension = part.trim().trim_start_matches('.');
        if extension.is_empty() {
            continue;
        }
        if !extension.chars().all(|ch| ch.is_ascii_alphanumeric()) {
            return Err(format!(
                "fileTypes contains unsupported extension `{extension}`"
            ));
        }
        types.push(extension.to_string());
    }
    Ok(types)
}

fn grep_body(stdout: &str, mode: &str, limit: usize) -> String {
    let mut lines = Vec::new();
    let mut seen = BTreeSet::new();
    for line in stdout
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty())
    {
        if mode == "files" && !seen.insert(line.to_string()) {
            continue;
        }
        lines.push(line.to_string());
        if lines.len() >= limit {
            break;
        }
    }
    lines.join("\n")
}

fn diagnostics_mode(args: &Map<String, Value>) -> &str {
    args.get("mode")
        .and_then(Value::as_str)
        .unwrap_or("analyze")
}

fn diagnostics_analyze_args(args: &Map<String, Value>) -> Map<String, Value> {
    let mut filtered = Map::new();
    for key in ["cwd", "dryRun", "confirm", "sourceDir", "config", "format"] {
        if let Some(value) = args.get(key) {
            let value = if key == "format" && value.as_str() == Some("json") {
                json!("jsonl")
            } else {
                value.clone()
            };
            filtered.insert(key.to_string(), value);
        }
    }
    filtered
}

fn bsl_mcp_command(
    source_dir: &Path,
    context: &WorkspaceContext,
    remote_tool: &'static str,
    tool_args: Value,
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
        "unica.code.diagnostics" => {
            let mut payload = Map::new();
            payload.insert("action".to_string(), json!(diagnostics_mode(args)));
            copy_json_arg(&mut payload, args, "codes", "codes");
            copy_json_arg(&mut payload, args, "path", "path");
            copy_json_arg(&mut payload, args, "detail", "detail");
            copy_json_arg(&mut payload, args, "minSeverity", "min_severity");
            copy_json_arg(&mut payload, args, "rangeStart", "range_start");
            copy_json_arg(&mut payload, args, "rangeEnd", "range_end");
            copy_json_arg(&mut payload, args, "limit", "max_findings");
            copy_json_arg(&mut payload, args, "maxFiles", "max_files");
            Ok(("diagnostics", Value::Object(payload)))
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
    match args.get("sourceDir").and_then(Value::as_str) {
        Some(raw) => {
            let rel = safe_workspace_rel(context, raw)?;
            Ok(context.workspace_root.join(rel))
        }
        None => Ok(context.cwd.clone()),
    }
}

fn bsl_mcp_readiness_warnings(text: &str) -> Vec<String> {
    if text.contains("\"reload\":\"running\"")
        || text.contains("\"state\":\"loading\"")
        || text.contains("\"status\":\"loading\"")
        || text.contains("not_ready")
        || text.contains("not ready")
    {
        vec![
            "bsl-analyzer workspace model is not ready yet; retry status or the request after reload completes"
                .to_string(),
        ]
    } else {
        Vec::new()
    }
}

fn safe_workspace_rel(context: &WorkspaceContext, raw: &str) -> Result<String, String> {
    let path = PathBuf::from(raw);
    let resolved = if path.is_absolute() {
        normalize_lexical_path(&path)
    } else {
        normalize_lexical_path(&context.cwd.join(path))
    };
    let workspace = normalize_lexical_path(&context.workspace_root);
    if !resolved.starts_with(&workspace) {
        return Err(format!(
            "path `{raw}` resolves outside workspace root {}",
            context.workspace_root.display()
        ));
    }
    let rel = resolved
        .strip_prefix(&workspace)
        .map_err(|error| format!("failed to relativize `{raw}`: {error}"))?;
    Ok(path_to_slash(rel))
}

fn normalize_lexical_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

fn path_to_slash(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn rel_under(rel: &str, base: &str) -> bool {
    base.is_empty() || rel == base || rel.starts_with(&format!("{base}/"))
}

fn strip_rel_prefix(rel: &str, base: &str) -> String {
    if base.is_empty() {
        rel.to_string()
    } else if rel == base {
        String::new()
    } else {
        rel.strip_prefix(&format!("{base}/"))
            .unwrap_or(rel)
            .to_string()
    }
}

fn join_rel(base: &str, rel: &str) -> String {
    match (base.is_empty(), rel.is_empty()) {
        (true, _) => rel.to_string(),
        (_, true) => base.to_string(),
        _ => format!(
            "{}/{}",
            base.trim_end_matches('/'),
            rel.trim_start_matches('/')
        ),
    }
}

impl ProcessRunner for SystemProcessRunner {
    fn run(&self, command: &ProcessCommand) -> Result<ProcessOutput, String> {
        let mut child = Command::new(&command.program)
            .args(&command.args)
            .current_dir(&command.cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| format!("failed to execute process: {err}"))?;

        let started = Instant::now();
        loop {
            if child
                .try_wait()
                .map_err(|err| format!("failed to poll process: {err}"))?
                .is_some()
            {
                let output = child
                    .wait_with_output()
                    .map_err(|err| format!("failed to collect process output: {err}"))?;
                return Ok(ProcessOutput {
                    status_success: output.status.success(),
                    status: output.status.to_string(),
                    stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                    stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                    timed_out: false,
                });
            }

            if let Some(timeout) = command.timeout {
                if started.elapsed() >= timeout {
                    let _ = child.kill();
                    let output = child.wait_with_output().map_err(|err| {
                        format!("failed to collect timed-out process output: {err}")
                    })?;
                    return Ok(ProcessOutput {
                        status_success: false,
                        status: "timeout".to_string(),
                        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                        timed_out: true,
                    });
                }
            }

            std::thread::sleep(Duration::from_millis(25));
        }
    }
}

impl BslMcpRunner for SystemBslMcpRunner {
    fn call(&self, command: &BslMcpCommand) -> Result<BslMcpOutput, String> {
        let context = WorkspaceContext::discover(command.cwd.clone())?;
        let output = WorkspaceServiceManager::new().call_bsl_mcp(
            &context,
            &command.source_dir,
            command.tool_name,
            command.tool_args.clone(),
            command.timeout,
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

    pub fn invoke(operation: &str, args: &Map<String, Value>) -> AdapterOutcome {
        Self::invoke_with_client(operation, args, &UREQ_HTTP_CLIENT)
    }

    pub fn invoke_with_client(
        operation: &str,
        args: &Map<String, Value>,
        http: &dyn HttpClient,
    ) -> AdapterOutcome {
        let endpoint = env::var("UNICA_STANDARDS_MCP_URL")
            .unwrap_or_else(|_| "https://ai.v8std.ru/mcp".to_string());
        let request = match Self::request_for(operation, args) {
            Ok(request) => request,
            Err(error) => {
                return AdapterOutcome {
                    ok: false,
                    summary: format!("unica.standards.{operation} rejected invalid arguments"),
                    changes: Vec::new(),
                    warnings: Vec::new(),
                    errors: vec![error],
                    artifacts: vec![endpoint],
                    stdout: None,
                    stderr: None,
                    command: None,
                }
            }
        };

        let payload = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": request.method,
                "arguments": request.params,
            }
        });

        match http.post_json(&endpoint, &payload) {
            Ok(text) => Self::outcome_from_http_body(operation, &endpoint, request.method, &text),
            Err(err) => AdapterOutcome {
                ok: false,
                summary: format!(
                    "unica.standards.{operation} failed through internal v8std MCP proxy"
                ),
                changes: Vec::new(),
                warnings: Vec::new(),
                errors: vec![err.to_string()],
                artifacts: vec![endpoint, request.method.to_string()],
                stdout: None,
                stderr: None,
                command: None,
            },
        }
    }

    pub fn outcome_from_http_body(
        operation: &str,
        endpoint: &str,
        remote_method: &str,
        text: &str,
    ) -> AdapterOutcome {
        let normalized = match normalize_mcp_http_body(text) {
            Ok(text) => text,
            Err(error) => {
                return AdapterOutcome {
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
                }
            }
        };

        match serde_json::from_str::<Value>(&normalized) {
            Ok(Value::Object(object)) if object.contains_key("error") => {
                let message = object
                    .get("error")
                    .and_then(|error| error.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or("remote JSON-RPC error");
                AdapterOutcome {
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
                }
            }
            Ok(Value::Object(object)) if object.contains_key("result") => AdapterOutcome {
                ok: true,
                summary: format!(
                    "unica.standards.{operation} completed through internal v8std MCP proxy"
                ),
                changes: Vec::new(),
                warnings: Vec::new(),
                errors: Vec::new(),
                artifacts: vec![endpoint.to_string(), remote_method.to_string()],
                stdout: Some(normalized),
                stderr: None,
                command: None,
            },
            Ok(_) => AdapterOutcome {
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
            },
            Err(error) => AdapterOutcome {
                ok: false,
                summary: format!("unica.standards.{operation} received invalid v8std MCP JSON"),
                changes: Vec::new(),
                warnings: Vec::new(),
                errors: vec![error.to_string()],
                artifacts: vec![endpoint.to_string(), remote_method.to_string()],
                stdout: None,
                stderr: None,
                command: None,
            },
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
    "force",
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

    if operation == "build" {
        result.push("--json-message".to_string());
    }

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

fn runtime_syntax_json_args(
    args: &Map<String, Value>,
    redact: bool,
) -> Result<Vec<String>, String> {
    if args.contains_key("operation") {
        return Err(
            "internal syntax invocation fixes `operation` to `syntax`; omit `operation`"
                .to_string(),
        );
    }

    let mut syntax_args = args.clone();
    syntax_args.insert("operation".to_string(), json!("syntax"));
    let mut result = runtime_args(&syntax_args, redact)?;
    result.insert(0, "--json-message".to_string());
    Ok(result)
}

fn append_runtime_global_args(
    result: &mut Vec<String>,
    operation: &str,
    args: &Map<String, Value>,
    redact: bool,
) {
    if operation != "config-init" {
        append_arg(result, "--config", args, "config", redact);
    }
    append_arg(result, "--workdir", args, "workdir", redact);
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

    match operation {
        "dump" => {
            validate_mapper_enum(args, "mode", RUNTIME_MAPPER_DUMP_MODES)?;
            if let Some(force) = args.get("force") {
                if !force.is_boolean() {
                    return Err("argument `force` must be boolean".to_string());
                }
                if args.get("mode").and_then(Value::as_str) != Some("partial") {
                    return Err(
                        "operation `dump` accepts `force` only with mode `partial`".to_string()
                    );
                }
            }
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
                    result.push(value_to_cli_string(item));
                }
            }
            other => {
                result.push(flag);
                result.push(if redact && is_secret_key(key) {
                    "<redacted>".to_string()
                } else {
                    value_to_cli_string(other)
                });
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
        result.push(if redact && is_secret_key(key) {
            "<redacted>".to_string()
        } else {
            value_to_cli_string(item)
        });
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
        if redact && is_secret_key(key) {
            Some("<redacted>".to_string())
        } else {
            Some(value_to_cli_string(value))
        }
    })
}

fn is_secret_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key == "connection"
        || key == "pwd"
        || key.contains("password")
        || key.contains("token")
        || key.contains("secret")
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
    use crate::infrastructure::workspace_index::{IndexBackgroundJob, IndexCommand, IndexOutput};
    use rusqlite::Connection;
    use serde_json::json;
    use std::cell::RefCell;
    use std::fs;
    use std::path::Path;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

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
        assert_eq!(
            commands[0].args,
            vec![
                "--json-message",
                "build",
                "--full-rebuild",
                "--source-set",
                "main"
            ]
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

    #[test]
    fn runtime_build_report_parses_terminal_steps_without_log_inference() {
        let report = parse_runtime_build_report(
            r#"{
                "ok": false,
                "command": "build",
                "duration_ms": 42,
                "data": {
                    "ok": false,
                    "steps": [
                        {"source_set":"main","mode":{"partial":{"file_count":2}},"ok":true,"message":null,"duration_ms":20},
                        {"source_set":"ext","mode":"skipped","ok":false,"message":"aborted","duration_ms":0}
                    ],
                    "duration_ms": 42
                },
                "warnings": [],
                "steps": [],
                "error": {"code":"build_failed","kind":"runtime","message":"failed"}
            }"#,
        )
        .unwrap();

        assert!(!report.envelope_ok);
        assert_eq!(report.steps.len(), 2);
        assert_eq!(report.steps[0].source_set.as_str(), "main");
        assert_eq!(
            report.steps[0].mode,
            BuildStepMode::Partial { file_count: 2 }
        );
        assert!(report.steps[0].ok);
        assert_eq!(report.steps[1].mode, BuildStepMode::Skipped);
    }

    #[test]
    fn runtime_build_report_rejects_invalid_or_ambiguous_terminal_json() {
        let arbitrary_log = parse_runtime_build_report("Designer build completed").unwrap_err();
        assert!(arbitrary_log.contains("invalid v8-runner build JSON envelope"));

        let wrong_command =
            parse_runtime_build_report(r#"{"ok":true,"command":"dump","data":{"steps":[]}}"#)
                .unwrap_err();
        assert!(wrong_command.contains("expected command `build`"));

        let duplicate = parse_runtime_build_report(
            r#"{"ok":true,"command":"build","data":{"steps":[
                {"source_set":"main","mode":"full","ok":true},
                {"source_set":"main","mode":"skipped","ok":false}
            ]}}"#,
        )
        .unwrap_err();
        assert!(duplicate.contains("duplicate source_set `main`"));

        let malformed_partial = parse_runtime_build_report(
            r#"{"ok":true,"command":"build","data":{"steps":[
                {"source_set":"main","mode":{"partial":null},"ok":true}
            ]}}"#,
        )
        .unwrap_err();
        assert!(malformed_partial.contains("malformed partial build mode"));
    }

    #[test]
    fn runtime_adapter_syntax_json_uses_only_typed_syntax_argv() {
        let context = temp_context("runtime-syntax-json-argv");
        let runner = RecordingProcessRunner {
            commands: RefCell::new(Vec::new()),
            output: ProcessOutput {
                status_success: true,
                status: "exit status: 0".to_string(),
                stdout: "{\"status\":\"success\"}".to_string(),
                stderr: String::new(),
                timed_out: false,
            },
        };
        let mut args = Map::new();
        args.insert("mode".to_string(), json!("designer-modules"));
        args.insert("server".to_string(), json!(true));

        let outcome = RuntimeAdapter::with_runner(&runner)
            .invoke_syntax_json("unica.code.patch", &args, &context)
            .unwrap();

        assert!(outcome.ok);
        let commands = runner.commands.borrow();
        assert_eq!(
            commands[0].args,
            vec!["--json-message", "syntax", "designer-modules", "--server"]
        );
        assert!(commands[0]
            .args
            .iter()
            .all(|arg| !matches!(arg.as_str(), "build" | "load" | "update")));
        assert!(commands[0].timeout.is_none());
        drop(commands);
        cleanup_context(&context);
    }

    #[test]
    fn runtime_adapter_syntax_json_rejects_raw_or_overridden_operation() {
        let context = temp_context("runtime-syntax-json-rejects-raw");
        let runner = RecordingProcessRunner {
            commands: RefCell::new(Vec::new()),
            output: ProcessOutput {
                status_success: true,
                status: "exit status: 0".to_string(),
                stdout: String::new(),
                stderr: String::new(),
                timed_out: false,
            },
        };
        let mut raw_args = Map::new();
        raw_args.insert("args".to_string(), json!(["build", "load", "update"]));

        let raw_error = RuntimeAdapter::with_runner(&runner)
            .invoke_syntax_json("unica.code.patch", &raw_args, &context)
            .unwrap_err();

        assert!(raw_error.contains("raw args are not accepted"));
        let mut overridden_args = Map::new();
        overridden_args.insert("operation".to_string(), json!("load"));

        let override_error = RuntimeAdapter::with_runner(&runner)
            .invoke_syntax_json("unica.code.patch", &overridden_args, &context)
            .unwrap_err();

        assert!(override_error.contains("fixes `operation` to `syntax`"));
        assert!(runner.commands.borrow().is_empty());
        cleanup_context(&context);
    }

    #[test]
    fn runtime_adapter_delegates_successful_build_without_wrapper_timeout() {
        let context = temp_context("runtime-build-success");
        let runner = RecordingProcessRunner {
            commands: RefCell::new(Vec::new()),
            output: ProcessOutput {
                status_success: true,
                status: "exit status: 0".to_string(),
                stdout: "Designer build completed after 240 seconds".to_string(),
                stderr: String::new(),
                timed_out: false,
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
    fn runtime_adapter_keeps_partial_dump_force_inside_unica_wrapper() {
        let args = json!({
            "operation": "dump",
            "mode": "partial",
            "object": "Catalog:Items",
            "force": true,
        })
        .as_object()
        .unwrap()
        .clone();

        let argv = runtime_args(&args, false).unwrap();

        assert_eq!(
            argv,
            vec!["dump", "--mode", "partial", "--object", "Catalog:Items"]
        );
        assert!(!argv.iter().any(|arg| arg == "--force"));

        let non_partial = json!({"operation": "dump", "mode": "full", "force": false})
            .as_object()
            .unwrap()
            .clone();
        assert!(runtime_args(&non_partial, false)
            .unwrap_err()
            .contains("only with mode `partial`"));
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
    fn code_adapter_dry_run_builds_bsl_analyzer_command() {
        let context = temp_context("code-adapter-dry-run");
        let mut args = Map::new();
        args.insert("query".to_string(), json!("ОбщийМодуль"));

        let outcome = CliAdapter::new("bsl-analyzer", &["search"], "code analysis")
            .invoke("unica.code.search", &args, &context, true, false)
            .unwrap();

        let command = outcome.command.unwrap().join(" ");
        assert!(command.contains("bin/"));
        assert!(command.contains("bsl-analyzer"));
        assert!(!command.contains("run-bsl-analyzer.sh"));
        assert!(command.contains("search"));
        assert!(command.contains("--query"));
        assert!(command.contains("ОбщийМодуль"));
        cleanup_context(&context);
    }

    #[test]
    fn code_search_adapter_dry_run_reports_typed_code_search() {
        let context = WorkspaceContext::discover(std::env::current_dir().unwrap()).unwrap();
        let grep = FakeProcessRunner {
            output: ProcessOutput {
                status_success: true,
                status: "exit status: 0".to_string(),
                stdout: "ignored".to_string(),
                stderr: String::new(),
                timed_out: false,
            },
        };
        let index = FakeIndexRunner::default();
        let mut args = Map::new();
        args.insert("query".to_string(), json!("ОбработкаПроведения"));

        let outcome = CodeSearchAdapter::with_runners(&grep, &index)
            .invoke("unica.code.search", &args, &context, true)
            .unwrap();

        assert!(outcome.ok);
        assert_eq!(
            outcome.summary,
            "dry run: unica.code.search would use typed code search"
        );
        assert!(outcome.command.is_none());
    }

    #[test]
    fn code_search_adapter_falls_back_to_git_grep_when_rlm_index_is_missing() {
        let context = temp_context("search-missing");
        fs::create_dir_all(context.workspace_root.join("src/CommonModules")).unwrap();
        let grep = FakeProcessRunner {
            output: ProcessOutput {
                status_success: true,
                status: "exit status: 0".to_string(),
                stdout: "CommonModules/SmokeModule/Ext/Module.bsl:2:ОбработкаПроведения\n"
                    .to_string(),
                stderr: String::new(),
                timed_out: false,
            },
        };
        let index = FakeIndexRunner {
            outputs: RefCell::new(vec![index_success("Index not found: /tmp/bsl_index.db")]),
            ..Default::default()
        };
        let mut args = Map::new();
        args.insert("query".to_string(), json!("ОбработкаПроведения"));

        let outcome = CodeSearchAdapter::with_runners(&grep, &index)
            .invoke("unica.code.search", &args, &context, false)
            .unwrap();

        assert!(outcome.ok);
        assert!(outcome
            .stdout
            .as_deref()
            .is_some_and(|stdout| stdout.contains("=== git grep ===")));
        assert!(outcome
            .warnings
            .iter()
            .any(|warning| warning.contains("rlm index unavailable")));
        cleanup_context(&context);
    }

    #[test]
    fn code_search_adapter_returns_three_backend_sections_in_order() {
        let context = temp_context("search-three-backends");
        fs::create_dir_all(context.workspace_root.join("src/CommonModules")).unwrap();
        let db_path = context.cache_root.join("rlm-tools-bsl/test/bsl_index.db");
        create_rlm_search_db(&db_path);
        let runner = FakeProcessRunner {
            output: ProcessOutput {
                status_success: true,
                status: "exit status: 0".to_string(),
                stdout: "CommonModules/Проведение.bsl:42:ОбработкаПроведения\n".to_string(),
                stderr: String::new(),
                timed_out: false,
            },
        };
        let index = FakeIndexRunner {
            outputs: RefCell::new(vec![index_success(format!(
                "Index: {}\n  Status:   fresh\n",
                db_path.display()
            ))]),
            ..Default::default()
        };
        let mut args = Map::new();
        args.insert("query".to_string(), json!("ОбработкаПроведения"));
        args.insert("limit".to_string(), json!(5));

        let outcome = CodeSearchAdapter::with_runners(&runner, &index)
            .invoke("unica.code.search", &args, &context, false)
            .unwrap();

        assert!(outcome.ok);
        assert!(outcome.command.is_none());
        let stdout = outcome.stdout.unwrap();
        let bsl_pos = stdout.find("=== bsl-analyzer ===").unwrap();
        let rlm_pos = stdout.find("=== rlm ===").unwrap();
        let grep_pos = stdout.find("=== git grep ===").unwrap();
        assert!(bsl_pos < rlm_pos);
        assert!(rlm_pos < grep_pos);
        assert!(stdout.contains("=== rlm ==="));
        assert!(stdout.contains("=== git grep ==="));
        assert!(stdout.contains("CommonModules/Проведение.bsl:42"));
        assert!(stdout.contains("Procedure ОбработкаПроведения() export"));
        assert!(!stdout.contains("=== git-grep ==="));
        cleanup_context(&context);
    }

    #[test]
    fn code_search_adapter_runs_bsl_analyzer_search_natively() {
        let context = temp_context("search-bsl-command");
        let bsl = RecordingProcessRunner {
            commands: RefCell::new(Vec::new()),
            output: ProcessOutput {
                status_success: true,
                status: "exit status: 0".to_string(),
                stdout: "bsl result\n".to_string(),
                stderr: String::new(),
                timed_out: false,
            },
        };
        let grep = RecordingProcessRunner {
            commands: RefCell::new(Vec::new()),
            output: ProcessOutput {
                status_success: true,
                status: "exit status: 0".to_string(),
                stdout: "CommonModules/Проведение.bsl:42:ОбработкаПроведения\n".to_string(),
                stderr: String::new(),
                timed_out: false,
            },
        };
        let index = FakeIndexRunner {
            outputs: RefCell::new(vec![index_success("Index not found: /tmp/bsl_index.db")]),
            ..Default::default()
        };
        let mut args = Map::new();
        args.insert("query".to_string(), json!("ОбработкаПроведения"));

        let outcome = CodeSearchAdapter::with_backend_runners(&bsl, &grep, &index)
            .invoke("unica.code.search", &args, &context, false)
            .unwrap();

        assert!(outcome.ok);
        assert!(outcome.command.is_none());
        let bsl_commands = bsl.commands.borrow();
        assert_eq!(bsl_commands.len(), 1);
        assert!(bsl_commands[0].program.to_string_lossy().contains("bin/"));
        assert!(!bsl_commands[0]
            .program
            .to_string_lossy()
            .contains("run-bsl-analyzer.sh"));
        assert_eq!(bsl_commands[0].args[0], "search");
        assert!(bsl_commands[0].args.contains(&"--query".to_string()));
        let grep_commands = grep.commands.borrow();
        assert_eq!(grep_commands[0].program, PathBuf::from("git"));
        cleanup_context(&context);
    }

    #[test]
    fn code_search_adapter_keeps_unavailable_sections_when_git_grep_succeeds() {
        let context = temp_context("search-unavailable");
        fs::remove_file(
            context
                .workspace_root
                .join("plugins/unica/bin/darwin-arm64/bsl-analyzer"),
        )
        .ok();
        fs::remove_file(
            context
                .workspace_root
                .join("plugins/unica/bin/linux-x64/bsl-analyzer"),
        )
        .ok();
        fs::remove_file(
            context
                .workspace_root
                .join("plugins/unica/bin/win-x64/bsl-analyzer.exe"),
        )
        .ok();
        let grep = FakeProcessRunner {
            output: ProcessOutput {
                status_success: true,
                status: "exit status: 0".to_string(),
                stdout: "CommonModules/SmokeModule/Ext/Module.bsl:2:ОбработкаПроведения\n"
                    .to_string(),
                stderr: String::new(),
                timed_out: false,
            },
        };
        let index = FakeIndexRunner {
            outputs: RefCell::new(vec![index_success("Index not found: /tmp/bsl_index.db")]),
            ..Default::default()
        };
        let mut args = Map::new();
        args.insert("query".to_string(), json!("ОбработкаПроведения"));

        let outcome = CodeSearchAdapter::with_runners(&grep, &index)
            .invoke("unica.code.search", &args, &context, false)
            .unwrap();

        assert!(outcome.ok);
        let stdout = outcome.stdout.unwrap();
        assert!(stdout.contains("=== bsl-analyzer ===\nunavailable:"));
        assert!(stdout.contains("=== rlm ===\nunavailable: rlm index unavailable"));
        assert!(stdout.contains("=== git grep ==="));
        assert!(stdout.contains("CommonModules/SmokeModule/Ext/Module.bsl:2"));
        cleanup_context(&context);
    }

    #[test]
    fn code_search_adapter_returns_three_failed_sections_when_no_backend_runs() {
        let context = temp_context("search-all-failed");
        fs::remove_file(
            context
                .workspace_root
                .join("plugins/unica/bin/darwin-arm64/bsl-analyzer"),
        )
        .ok();
        fs::remove_file(
            context
                .workspace_root
                .join("plugins/unica/bin/linux-x64/bsl-analyzer"),
        )
        .ok();
        fs::remove_file(
            context
                .workspace_root
                .join("plugins/unica/bin/win-x64/bsl-analyzer.exe"),
        )
        .ok();
        let grep = FakeProcessRunner {
            output: ProcessOutput {
                status_success: false,
                status: "exit status: 128".to_string(),
                stdout: String::new(),
                stderr: "fatal: not a git repository (or any of the parent directories): .git\n"
                    .to_string(),
                timed_out: false,
            },
        };
        let index = FakeIndexRunner {
            outputs: RefCell::new(vec![index_success("Index not found: /tmp/bsl_index.db")]),
            ..Default::default()
        };
        let mut args = Map::new();
        args.insert("query".to_string(), json!("SmokeProcedure"));

        let outcome = CodeSearchAdapter::with_runners(&grep, &index)
            .invoke("unica.code.search", &args, &context, false)
            .unwrap();

        assert!(!outcome.ok);
        let stdout = outcome.stdout.unwrap();
        assert!(stdout.contains("=== bsl-analyzer ===\nunavailable:"));
        assert!(stdout.contains("=== rlm ===\nunavailable: rlm index unavailable"));
        assert!(stdout.contains("=== git grep ===\nfailed: fatal: not a git repository"));
        cleanup_context(&context);
    }

    #[test]
    fn code_search_adapter_reports_git_grep_fatal_error_instead_of_no_matches() {
        let context = temp_context("search-grep-fatal");
        fs::create_dir_all(context.workspace_root.join("src/CommonModules")).unwrap();
        let grep = FakeProcessRunner {
            output: ProcessOutput {
                status_success: false,
                status: "exit status: 128".to_string(),
                stdout: String::new(),
                stderr: "fatal: not a git repository (or any of the parent directories): .git\n"
                    .to_string(),
                timed_out: false,
            },
        };
        let index = FakeIndexRunner {
            outputs: RefCell::new(vec![index_success("Index not found: /tmp/bsl_index.db")]),
            ..Default::default()
        };
        let mut args = Map::new();
        args.insert("query".to_string(), json!("SmokeProcedure"));

        let outcome = CodeSearchAdapter::with_runners(&grep, &index)
            .invoke("unica.code.search", &args, &context, false)
            .unwrap();

        assert!(!outcome.ok);
        assert!(outcome
            .errors
            .iter()
            .any(|error| error.contains("fatal: not a git repository")));
        assert!(!outcome
            .stdout
            .as_deref()
            .unwrap_or_default()
            .contains("No git grep matches."));
        cleanup_context(&context);
    }

    #[test]
    fn code_search_adapter_adds_rlm_section_when_index_is_ready() {
        let context = temp_context("search-ready");
        fs::create_dir_all(context.workspace_root.join("src/CommonModules")).unwrap();
        let db_path = context.cache_root.join("rlm-tools-bsl/test/bsl_index.db");
        create_rlm_search_db(&db_path);
        let grep = FakeProcessRunner {
            output: ProcessOutput {
                status_success: true,
                status: "exit status: 0".to_string(),
                stdout: "CommonModules/Проведение.bsl:42:ОбработкаПроведения\n".to_string(),
                stderr: String::new(),
                timed_out: false,
            },
        };
        let index = FakeIndexRunner {
            outputs: RefCell::new(vec![index_success(format!(
                "Index: {}\n  Status:   fresh\n",
                db_path.display()
            ))]),
            ..Default::default()
        };
        let mut args = Map::new();
        args.insert("query".to_string(), json!("ОбработкаПроведения"));
        args.insert("limit".to_string(), json!(5));

        let outcome = CodeSearchAdapter::with_runners(&grep, &index)
            .invoke("unica.code.search", &args, &context, false)
            .unwrap();

        let stdout = outcome.stdout.unwrap();
        assert!(stdout.contains("=== rlm ==="));
        assert!(stdout.contains("=== git grep ==="));
        assert!(stdout.contains("CommonModules/Проведение.bsl:42"));
        assert!(stdout.contains("Procedure ОбработкаПроведения() export"));
        cleanup_context(&context);
    }

    #[test]
    fn code_definition_adapter_returns_matches_from_ready_rlm_index() {
        let context = temp_context("definition-ready");
        fs::create_dir_all(context.workspace_root.join("src/CommonModules")).unwrap();
        let db_path = context.cache_root.join("rlm-tools-bsl/test/bsl_index.db");
        create_rlm_navigation_db(&db_path);
        let index = FakeIndexRunner {
            outputs: RefCell::new(vec![index_success(format!(
                "Index: {}\n  Status:   fresh\n",
                db_path.display()
            ))]),
            ..Default::default()
        };
        let grep = FakeProcessRunner {
            output: ProcessOutput {
                status_success: true,
                status: "exit status: 0".to_string(),
                stdout: String::new(),
                stderr: String::new(),
                timed_out: false,
            },
        };
        let mut args = Map::new();
        args.insert("name".to_string(), json!("SmokeProcedure"));
        args.insert("limit".to_string(), json!(5));

        let outcome = CodeNavigationAdapter::with_runners(&index, &grep)
            .invoke("unica.code.definition", &args, &context, false)
            .unwrap();

        let stdout = outcome.stdout.unwrap();
        assert!(stdout.contains("=== rlm-definition ==="));
        assert!(stdout.contains("CommonModules/SmokeModule/Ext/Module.bsl:2"));
        assert!(stdout.contains("Procedure SmokeProcedure() export"));
        assert!(stdout.contains("category=CommonModule"));
        cleanup_context(&context);
    }

    #[test]
    fn code_outline_adapter_returns_regions_headers_and_methods() {
        let context = temp_context("outline-ready");
        fs::create_dir_all(context.workspace_root.join("src/CommonModules")).unwrap();
        let db_path = context.cache_root.join("rlm-tools-bsl/test/bsl_index.db");
        create_rlm_navigation_db(&db_path);
        let index = FakeIndexRunner {
            outputs: RefCell::new(vec![index_success(format!(
                "Index: {}\n  Status:   fresh\n",
                db_path.display()
            ))]),
            ..Default::default()
        };
        let grep = FakeProcessRunner {
            output: ProcessOutput {
                status_success: true,
                status: "exit status: 0".to_string(),
                stdout: String::new(),
                stderr: String::new(),
                timed_out: false,
            },
        };
        let mut args = Map::new();
        args.insert(
            "path".to_string(),
            json!("CommonModules/SmokeModule/Ext/Module.bsl"),
        );

        let outcome = CodeNavigationAdapter::with_runners(&index, &grep)
            .invoke("unica.code.outline", &args, &context, false)
            .unwrap();

        let stdout = outcome.stdout.unwrap();
        assert!(stdout.contains("=== rlm-outline ==="));
        assert!(stdout.contains("module: CommonModules/SmokeModule/Ext/Module.bsl"));
        assert!(stdout.contains("header: Smoke module header"));
        assert!(stdout.contains("region PublicApi: 1-5"));
        assert!(stdout.contains("Procedure SmokeProcedure() export"));
        cleanup_context(&context);
    }

    #[test]
    fn meta_profile_adapter_returns_object_metadata_from_ready_rlm_index() {
        let context = temp_context("meta-profile-ready");
        fs::create_dir_all(context.workspace_root.join("src/Documents/SalesOrder")).unwrap();
        let db_path = context.cache_root.join("rlm-tools-bsl/test/bsl_index.db");
        create_rlm_profile_db(&db_path);
        let index = FakeIndexRunner {
            outputs: RefCell::new(vec![index_success(format!(
                "Index: {}\n  Status:   fresh\n",
                db_path.display()
            ))]),
            ..Default::default()
        };
        let grep = FakeProcessRunner {
            output: ProcessOutput {
                status_success: true,
                status: "exit status: 0".to_string(),
                stdout: String::new(),
                stderr: String::new(),
                timed_out: false,
            },
        };
        let mut args = Map::new();
        args.insert("name".to_string(), json!("Document.SalesOrder"));
        args.insert(
            "sections".to_string(),
            json!([
                "structure",
                "modules",
                "roles",
                "subscriptions",
                "functionalOptions"
            ]),
        );
        args.insert("limit".to_string(), json!(10));

        let outcome = CodeNavigationAdapter::with_runners(&index, &grep)
            .invoke("unica.meta.profile", &args, &context, false)
            .unwrap();

        assert!(outcome.ok);
        let stdout = outcome.stdout.unwrap();
        assert!(stdout.contains("=== rlm-meta-profile ==="));
        assert!(stdout.contains("object: Document.SalesOrder"));
        assert!(stdout.contains("section structure: ok total=1 returned=1"));
        assert!(stdout.contains("- attribute Customer type=CatalogRef.Customers"));
        assert!(stdout.contains("section modules: ok total=1 returned=1"));
        assert!(stdout.contains("- module Documents/SalesOrder/Ext/ObjectModule.bsl ObjectModule"));
        assert!(stdout.contains("section roles: ok total=1 returned=1"));
        assert!(stdout.contains("- role SalesManager rights=Read, Insert"));
        assert!(stdout.contains("section subscriptions: ok total=1 returned=1"));
        assert!(stdout.contains(
            "- subscription SalesOrderOnWrite event=OnWrite handler=SalesEvents.OnWrite"
        ));
        assert!(stdout.contains("section functionalOptions: ok total=1 returned=1"));
        assert!(stdout.contains("- option UseSalesOrders"));
        cleanup_context(&context);
    }

    #[test]
    fn meta_profile_adapter_warns_when_ready_index_lacks_profile_schema() {
        let context = temp_context("meta-profile-missing-schema");
        fs::create_dir_all(context.workspace_root.join("src/CommonModules")).unwrap();
        let db_path = context.cache_root.join("rlm-tools-bsl/test/bsl_index.db");
        create_rlm_navigation_db(&db_path);
        let index = FakeIndexRunner {
            outputs: RefCell::new(vec![index_success(format!(
                "Index: {}\n  Status:   fresh\n",
                db_path.display()
            ))]),
            ..Default::default()
        };
        let grep = FakeProcessRunner {
            output: ProcessOutput {
                status_success: true,
                status: "exit status: 0".to_string(),
                stdout: String::new(),
                stderr: String::new(),
                timed_out: false,
            },
        };
        let mut args = Map::new();
        args.insert("name".to_string(), json!("CommonModule.SmokeModule"));

        let outcome = CodeNavigationAdapter::with_runners(&index, &grep)
            .invoke("unica.meta.profile", &args, &context, false)
            .unwrap();

        assert!(outcome.ok);
        assert!(outcome
            .warnings
            .iter()
            .any(|warning| warning.contains("metadata profile schema")));
        let stdout = outcome.stdout.unwrap();
        assert!(stdout.contains("=== rlm-meta-profile ==="));
        assert!(stdout.contains("metadata profile unavailable"));
        assert!(stdout.contains("rebuild the RLM index"));
        cleanup_context(&context);
    }

    #[test]
    fn code_grep_adapter_maps_typed_args_to_safe_git_grep() {
        let context = temp_context("grep-command");
        let index = FakeIndexRunner::default();
        let grep = RecordingProcessRunner {
            commands: RefCell::new(Vec::new()),
            output: ProcessOutput {
                status_success: true,
                status: "exit status: 0".to_string(),
                stdout: "CommonModules/SmokeModule/Ext/Module.bsl:2:SmokeProcedure\n".to_string(),
                stderr: String::new(),
                timed_out: false,
            },
        };
        let mut args = Map::new();
        args.insert("query".to_string(), json!("SmokeProcedure"));
        args.insert("path".to_string(), json!("CommonModules"));
        args.insert("fileTypes".to_string(), json!("bsl"));
        args.insert("ignoreCase".to_string(), json!(true));
        args.insert("excludePath".to_string(), json!("CommonModules/Generated"));
        args.insert("limit".to_string(), json!(10));

        let outcome = CodeNavigationAdapter::with_runners(&index, &grep)
            .invoke("unica.code.grep", &args, &context, false)
            .unwrap();

        assert!(outcome.ok);
        assert_eq!(
            outcome.stdout.as_deref(),
            Some("=== git-grep ===\nCommonModules/SmokeModule/Ext/Module.bsl:2:SmokeProcedure")
        );
        let commands = grep.commands.borrow();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].program, PathBuf::from("git"));
        assert!(commands[0].args.contains(&"grep".to_string()));
        assert!(commands[0].args.contains(&"-F".to_string()));
        assert!(commands[0].args.contains(&"-i".to_string()));
        assert!(commands[0].args.contains(&"-m".to_string()));
        assert!(commands[0].args.contains(&"10".to_string()));
        assert!(commands[0]
            .args
            .contains(&":(glob)CommonModules/**/*.bsl".to_string()));
        assert!(commands[0]
            .args
            .contains(&":(exclude)CommonModules/Generated".to_string()));
        cleanup_context(&context);
    }

    #[test]
    fn code_grep_adapter_rejects_path_escape_before_git_execution() {
        let context = temp_context("grep-escape");
        let index = FakeIndexRunner::default();
        let grep = RecordingProcessRunner {
            commands: RefCell::new(Vec::new()),
            output: ProcessOutput {
                status_success: true,
                status: "exit status: 0".to_string(),
                stdout: String::new(),
                stderr: String::new(),
                timed_out: false,
            },
        };
        let mut args = Map::new();
        args.insert("query".to_string(), json!("SmokeProcedure"));
        args.insert("path".to_string(), json!("../outside"));

        let error = CodeNavigationAdapter::with_runners(&index, &grep)
            .invoke("unica.code.grep", &args, &context, false)
            .unwrap_err();

        assert!(error.contains("outside workspace root"));
        assert!(grep.commands.borrow().is_empty());
        cleanup_context(&context);
    }

    #[test]
    fn code_grep_adapter_reports_git_fatal_error_instead_of_no_matches() {
        let context = temp_context("grep-fatal");
        let index = FakeIndexRunner::default();
        let grep = RecordingProcessRunner {
            commands: RefCell::new(Vec::new()),
            output: ProcessOutput {
                status_success: false,
                status: "exit status: 128".to_string(),
                stdout: String::new(),
                stderr: "fatal: not a git repository (or any of the parent directories): .git\n"
                    .to_string(),
                timed_out: false,
            },
        };
        let mut args = Map::new();
        args.insert("query".to_string(), json!("SmokeProcedure"));

        let outcome = CodeNavigationAdapter::with_runners(&index, &grep)
            .invoke("unica.code.grep", &args, &context, false)
            .unwrap();

        assert!(!outcome.ok);
        assert!(outcome
            .errors
            .iter()
            .any(|error| error.contains("fatal: not a git repository")));
        assert!(!outcome
            .stdout
            .as_deref()
            .unwrap_or_default()
            .contains("No git grep matches."));
        cleanup_context(&context);
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
    fn diagnostics_analyze_normalizes_json_format_and_keeps_limit_out_of_cli_args() {
        let context = temp_context("diagnostics-format-dry-run");
        let mut args = Map::new();
        args.insert("sourceDir".to_string(), json!("src/extensions/Smoke"));
        args.insert("format".to_string(), json!("json"));
        args.insert("limit".to_string(), json!(20));

        let outcome = BslAnalyzerMcpAdapter::new()
            .invoke("unica.code.diagnostics", &args, &context, true)
            .unwrap();

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

        let outcome = BslAnalyzerMcpAdapter::with_runner(&runner)
            .invoke("unica.code.graph", &args, &context, false)
            .unwrap();

        assert!(outcome.ok);
        assert_eq!(
            outcome.stdout.as_deref(),
            Some("=== bsl-analyzer-graph ===\n{\"action\":\"callers\",\"nodes\":[]}")
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
        assert!(commands[0]
            .args
            .contains(&context.cwd.display().to_string()));
        cleanup_context(&context);
    }

    #[test]
    fn bsl_diagnostics_adapter_maps_file_mode_to_allowlisted_mcp_call() {
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

        let outcome = BslAnalyzerMcpAdapter::with_runner(&runner)
            .invoke("unica.code.diagnostics", &args, &context, false)
            .unwrap();

        assert!(outcome.ok);
        let commands = runner.commands.borrow();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].tool_name, "diagnostics");
        assert_eq!(commands[0].tool_args["action"], "file");
        assert_eq!(commands[0].tool_args["min_severity"], "warning");
        assert_eq!(commands[0].tool_args["range_start"], 3);
        assert_eq!(commands[0].tool_args["range_end"], 7);
        assert_eq!(commands[0].tool_args["max_findings"], 5);
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
            .unwrap();

        assert!(outcome.ok);
        assert!(outcome
            .warnings
            .iter()
            .any(|warning| warning.contains("not ready")));
        cleanup_context(&context);
    }

    #[test]
    fn cli_adapter_rejects_raw_args_vector() {
        let context = WorkspaceContext::discover(std::env::current_dir().unwrap()).unwrap();
        let mut args = Map::new();
        args.insert("args".to_string(), json!(["--unsafe", "../outside"]));

        let error = CliAdapter::new("v8-runner", &["build"], "build/runtime")
            .invoke("unica.build.load", &args, &context, true, true)
            .unwrap_err();

        assert!(error.contains("raw args are not accepted"));
    }

    #[test]
    fn cli_adapter_redacts_secret_values_from_reported_command() {
        let context = WorkspaceContext::discover(std::env::current_dir().unwrap()).unwrap();
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
        let context = WorkspaceContext::discover(std::env::current_dir().unwrap()).unwrap();
        let mut args = Map::new();
        args.insert("operation".to_string(), json!("config-init"));
        args.insert(
            "connection".to_string(),
            json!("Srvr=prod;Ref=ib;Usr=admin;Pwd=super-secret"),
        );

        let outcome = RuntimeAdapter::new()
            .invoke("unica.runtime.execute", &args, &context, true, true)
            .unwrap();

        let command = outcome.command.unwrap().join(" ");
        assert!(command.contains("--connection <redacted>"));
        assert!(!command.contains("super-secret"));
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
    fn runtime_adapter_does_not_report_wrapper_timeout_seconds_without_local_timeout() {
        let context = temp_context("runtime-timeout-no-local-budget");
        let runner = FakeProcessRunner {
            output: ProcessOutput {
                status_success: false,
                status: "timeout".to_string(),
                stdout: String::new(),
                stderr: String::new(),
                timed_out: true,
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
        let context = temp_context("runtime-non-zero-diagnostics");
        let runner = FakeProcessRunner {
            output: ProcessOutput {
                status_success: false,
                status: "exit status: 1".to_string(),
                stdout:
                    "prelude that should not matter\nstarted build\nUsr=admin;Pwd=stdout-secret\n"
                        .to_string(),
                stderr: "failed to load configuration: Pwd=stderr-secret\n".to_string(),
                timed_out: false,
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
        let context = temp_context("runtime-spawn-failure-diagnostics");
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
    fn system_process_runner_does_not_timeout_when_timeout_is_none() {
        #[cfg(windows)]
        let (program, args) = (
            PathBuf::from("powershell"),
            vec![
                "-NoProfile".to_string(),
                "-Command".to_string(),
                "[Console]::Write('ok')".to_string(),
            ],
        );
        #[cfg(not(windows))]
        let (program, args) = (
            PathBuf::from("sh"),
            vec!["-c".to_string(), "printf ok".to_string()],
        );

        let output = SYSTEM_PROCESS_RUNNER
            .run(&ProcessCommand {
                program,
                args,
                cwd: std::env::current_dir().unwrap(),
                timeout: None,
            })
            .unwrap();

        assert!(output.status_success);
        assert_eq!(output.stdout, "ok");
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

        assert!(!outcome.ok);
        assert!(outcome.errors.iter().any(|error| error.contains("bad id")));
        assert!(outcome.stdout.is_none());
    }

    #[test]
    fn standards_sse_body_extracts_structured_json_result() {
        let outcome = StandardsAdapter::outcome_from_http_body(
            "search",
            "https://example.test/mcp",
            "v8std_search",
            "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"ok\":true}}\n\n",
        );

        assert!(outcome.ok);
        assert_eq!(
            outcome.stdout.as_deref(),
            Some(r#"{"jsonrpc":"2.0","id":1,"result":{"ok":true}}"#)
        );
    }

    #[test]
    fn standards_protocol_mismatch_is_failure() {
        let outcome = StandardsAdapter::outcome_from_http_body(
            "search",
            "https://example.test/mcp",
            "v8std_search",
            r#"{"not":"json-rpc"}"#,
        );

        assert!(!outcome.ok);
        assert!(outcome
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

        let outcome = StandardsAdapter::invoke_with_client("search", &args, &client);

        assert!(outcome.ok);
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

    struct FakeProcessRunner {
        output: ProcessOutput,
    }

    impl ProcessRunner for FakeProcessRunner {
        fn run(&self, _command: &ProcessCommand) -> Result<ProcessOutput, String> {
            Ok(self.output.clone())
        }
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

    impl ProcessRunner for RecordingProcessRunner {
        fn run(&self, command: &ProcessCommand) -> Result<ProcessOutput, String> {
            self.commands.borrow_mut().push(command.clone());
            Ok(self.output.clone())
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

    #[derive(Default)]
    struct FakeIndexRunner {
        outputs: RefCell<Vec<IndexOutput>>,
        commands: RefCell<Vec<IndexCommand>>,
        backgrounds: RefCell<Vec<IndexBackgroundJob>>,
    }

    impl IndexRunner for FakeIndexRunner {
        fn run(&self, command: &IndexCommand) -> Result<IndexOutput, String> {
            self.commands.borrow_mut().push(command.clone());
            if self.outputs.borrow().is_empty() {
                return Ok(index_success("Index not found: /tmp/bsl_index.db"));
            }
            Ok(self.outputs.borrow_mut().remove(0))
        }

        fn start_background(&self, job: IndexBackgroundJob) -> Result<(), String> {
            self.backgrounds.borrow_mut().push(job);
            Ok(())
        }
    }

    fn index_success(stdout: impl Into<String>) -> IndexOutput {
        IndexOutput {
            status_success: true,
            status: "exit status: 0".to_string(),
            stdout: stdout.into(),
            stderr: String::new(),
            timed_out: false,
            duration_ms: 0,
        }
    }

    fn temp_context(name: &str) -> WorkspaceContext {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("unica-code-search-{name}-{nanos}"));
        fs::create_dir_all(&root).unwrap();
        create_fake_plugin_root(&root);
        WorkspaceContext {
            cwd: root.clone(),
            workspace_root: root.clone(),
            cache_root: root.join(".build").join("unica"),
            workspace_epoch: 1,
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

    fn create_rlm_search_db(db_path: &PathBuf) {
        fs::create_dir_all(db_path.parent().unwrap()).unwrap();
        let conn = Connection::open(db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE modules (
                id INTEGER PRIMARY KEY,
                rel_path TEXT NOT NULL,
                object_name TEXT NOT NULL
            );
            CREATE TABLE methods (
                id INTEGER PRIMARY KEY,
                module_id INTEGER NOT NULL,
                name TEXT NOT NULL,
                type TEXT NOT NULL,
                is_export INTEGER NOT NULL,
                line INTEGER NOT NULL,
                end_line INTEGER NOT NULL,
                params TEXT
            );
            CREATE VIRTUAL TABLE methods_fts USING fts5(name, object_name, tokenize='trigram');",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO modules (id, rel_path, object_name) VALUES (1, ?1, ?2)",
            ("CommonModules/Проведение.bsl", "Проведение"),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO methods (id, module_id, name, type, is_export, line, end_line, params)
             VALUES (1, 1, ?1, 'Procedure', 1, 42, 55, '')",
            ("ОбработкаПроведения",),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO methods_fts(rowid, name, object_name) VALUES (1, ?1, ?2)",
            ("ОбработкаПроведения", "Проведение"),
        )
        .unwrap();
    }

    fn create_rlm_navigation_db(db_path: &PathBuf) {
        fs::create_dir_all(db_path.parent().unwrap()).unwrap();
        let conn = Connection::open(db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE index_meta (
                key TEXT PRIMARY KEY,
                value TEXT
            );
            CREATE TABLE modules (
                id INTEGER PRIMARY KEY,
                rel_path TEXT NOT NULL,
                category TEXT,
                object_name TEXT,
                module_type TEXT
            );
            CREATE TABLE methods (
                id INTEGER PRIMARY KEY,
                module_id INTEGER NOT NULL,
                name TEXT NOT NULL,
                type TEXT NOT NULL,
                is_export INTEGER NOT NULL,
                params TEXT,
                line INTEGER NOT NULL,
                end_line INTEGER NOT NULL,
                loc INTEGER
            );
            CREATE VIRTUAL TABLE methods_fts USING fts5(name, object_name, tokenize='trigram');
            CREATE TABLE regions (
                id INTEGER PRIMARY KEY,
                module_id INTEGER NOT NULL,
                name TEXT NOT NULL,
                line INTEGER NOT NULL,
                end_line INTEGER
            );
            CREATE TABLE module_headers (
                module_id INTEGER PRIMARY KEY,
                header_comment TEXT NOT NULL
            );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO index_meta (key, value) VALUES ('builder_version', '14')",
            (),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO modules (id, rel_path, category, object_name, module_type)
             VALUES (1, ?1, 'CommonModule', 'SmokeModule', 'ManagerModule')",
            ("CommonModules/SmokeModule/Ext/Module.bsl",),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO methods (id, module_id, name, type, is_export, params, line, end_line, loc)
             VALUES (1, 1, 'SmokeProcedure', 'Procedure', 1, '', 2, 4, 3)",
            (),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO methods_fts(rowid, name, object_name) VALUES (1, 'SmokeProcedure', 'SmokeModule')",
            (),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO regions (id, module_id, name, line, end_line) VALUES (1, 1, 'PublicApi', 1, 5)",
            (),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO module_headers (module_id, header_comment) VALUES (1, 'Smoke module header')",
            (),
        )
        .unwrap();
    }

    fn create_rlm_profile_db(db_path: &PathBuf) {
        fs::create_dir_all(db_path.parent().unwrap()).unwrap();
        let conn = Connection::open(db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE index_meta (
                key TEXT PRIMARY KEY,
                value TEXT
            );
            CREATE TABLE modules (
                id INTEGER PRIMARY KEY,
                rel_path TEXT NOT NULL,
                category TEXT,
                object_name TEXT,
                module_type TEXT
            );
            CREATE TABLE methods (
                id INTEGER PRIMARY KEY,
                module_id INTEGER NOT NULL,
                name TEXT NOT NULL,
                type TEXT NOT NULL,
                is_export INTEGER NOT NULL,
                params TEXT,
                line INTEGER NOT NULL,
                end_line INTEGER NOT NULL,
                loc INTEGER
            );
            CREATE VIRTUAL TABLE methods_fts USING fts5(name, object_name, tokenize='trigram');
            CREATE TABLE regions (
                id INTEGER PRIMARY KEY,
                module_id INTEGER NOT NULL,
                name TEXT NOT NULL,
                line INTEGER NOT NULL,
                end_line INTEGER
            );
            CREATE TABLE module_headers (
                module_id INTEGER PRIMARY KEY,
                header_comment TEXT NOT NULL
            );
            CREATE TABLE object_attributes (
                id INTEGER PRIMARY KEY,
                object_name TEXT NOT NULL,
                category TEXT NOT NULL,
                attr_name TEXT NOT NULL,
                attr_synonym TEXT,
                attr_type TEXT,
                attr_kind TEXT NOT NULL,
                ts_name TEXT,
                source_file TEXT NOT NULL
            );
            CREATE TABLE role_rights (
                id INTEGER PRIMARY KEY,
                role_name TEXT NOT NULL,
                object_name TEXT NOT NULL,
                right_name TEXT NOT NULL,
                file TEXT
            );
            CREATE TABLE event_subscriptions (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                synonym TEXT,
                event TEXT,
                handler_module TEXT,
                handler_procedure TEXT,
                source_types TEXT,
                source_count INTEGER,
                file TEXT
            );
            CREATE TABLE functional_options (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                synonym TEXT,
                location TEXT,
                content TEXT,
                file TEXT
            );
            CREATE TABLE predefined_items (
                id INTEGER PRIMARY KEY,
                object_name TEXT NOT NULL,
                category TEXT NOT NULL,
                item_name TEXT NOT NULL,
                item_synonym TEXT,
                item_code TEXT,
                types_json TEXT,
                is_folder INTEGER DEFAULT 0,
                source_file TEXT NOT NULL
            );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO index_meta (key, value) VALUES ('builder_version', '14')",
            (),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO modules (id, rel_path, category, object_name, module_type)
             VALUES (1, 'Documents/SalesOrder/Ext/ObjectModule.bsl', 'Document', 'SalesOrder', 'ObjectModule')",
            (),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO object_attributes
             (object_name, category, attr_name, attr_synonym, attr_type, attr_kind, ts_name, source_file)
             VALUES ('SalesOrder', 'Document', 'Customer', 'Customer', 'CatalogRef.Customers', 'attribute', NULL, 'Documents/SalesOrder.xml')",
            (),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO role_rights (role_name, object_name, right_name, file)
             VALUES ('SalesManager', 'Document.SalesOrder', 'Read', 'Roles/SalesManager.xml')",
            (),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO role_rights (role_name, object_name, right_name, file)
             VALUES ('SalesManager', 'Document.SalesOrder', 'Insert', 'Roles/SalesManager.xml')",
            (),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO event_subscriptions
             (name, synonym, event, handler_module, handler_procedure, source_types, source_count, file)
             VALUES ('SalesOrderOnWrite', NULL, 'OnWrite', 'SalesEvents', 'OnWrite', 'Document.SalesOrder', 1, 'EventSubscriptions/SalesOrderOnWrite.xml')",
            (),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO functional_options (name, synonym, location, content, file)
             VALUES ('UseSalesOrders', NULL, 'Document.SalesOrder', 'Document.SalesOrder', 'FunctionalOptions/UseSalesOrders.xml')",
            (),
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
