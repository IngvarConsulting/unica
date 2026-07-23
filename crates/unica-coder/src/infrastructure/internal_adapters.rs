use crate::application::{
    AdapterOutcome, RuntimeJobAction, DIAGNOSTICS_ANALYZE_TIMEOUT_MAX_SECONDS,
    DIAGNOSTICS_ANALYZE_TIMEOUT_MIN_SECONDS,
};
use crate::domain::cancellation::{CancellationToken, CANCELLED_PREFIX};
use crate::domain::project_sources::{config_dump_info_xml_kind, ConfigDumpInfoXmlKind};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::bundled_tools::resolve_bundled_tool;
use crate::infrastructure::metadata_kinds::{metadata_kind, metadata_kind_by_directory};
use crate::infrastructure::platform::{
    ensure_truncation_diagnostics, ManagedChild, ManagedCommand, ManagedOutput,
};
use crate::infrastructure::plugin_runtime::{find_plugin_root, value_to_cli_string};
use crate::infrastructure::redaction::{is_secret_key, redactor};
use crate::infrastructure::runtime_jobs::{
    self, RuntimeJobOperation, RuntimeJobRequest, RuntimeJobService,
};
#[cfg(test)]
use crate::infrastructure::source_roots::normalize_path_identity;
use crate::infrastructure::source_roots::resolve_source_root;
use crate::infrastructure::workspace::discover_workspace;
use crate::infrastructure::workspace_index::{
    IndexReadiness, IndexRunner, WorkspaceIndexService, SYSTEM_INDEX_RUNNER,
};
use crate::infrastructure::workspace_services::WorkspaceServiceManager;
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

const DEFAULT_PROCESS_TIMEOUT: Duration = Duration::from_secs(120);
const GIT_TRACKING_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub struct ProcessCommand {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
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

pub struct CliAdapter<'a> {
    tool_name: &'static str,
    default_command: &'static [&'static str],
    label: &'static str,
    runner: &'a dyn ProcessRunner,
    process_timeout: Duration,
    report_timeout_seconds: bool,
}

pub struct RuntimeAdapter<'a> {
    runner: &'a dyn ProcessRunner,
}

pub struct RuntimeJobAdapterOutcome {
    pub outcome: AdapterOutcome,
    pub job: Option<Value>,
}

pub struct RuntimeJobAdapter;

pub(crate) struct GitTrackingAdapter<'a> {
    runner: &'a dyn ProcessRunner,
    timeout: Duration,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ConfigDumpInfoGitCheck {
    Complete(Option<String>),
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GitIndexPath {
    path: String,
    blob_oid: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GitBlobClassification {
    Classified(ConfigDumpInfoXmlKind),
    Inconclusive,
    Cancelled,
}

pub struct CodeSearchAdapter<'a> {
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
    process_runner: &'a dyn ProcessRunner,
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
            process_timeout: DEFAULT_PROCESS_TIMEOUT,
            report_timeout_seconds: false,
        }
    }

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
            report_timeout_seconds: false,
        }
    }

    fn with_process_timeout(mut self, timeout: Duration) -> Self {
        self.process_timeout = timeout;
        self.report_timeout_seconds = true;
        self
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
                vec![if self.report_timeout_seconds {
                    process_timeout_error(self.label, process_timeout)
                } else {
                    format!("internal {} adapter timed out", self.label)
                }]
            } else {
                vec![format!(
                    "internal {} adapter exited with status {}",
                    self.label, output.status
                )]
            },
            errors: if ok {
                Vec::new()
            } else if output.timed_out && self.report_timeout_seconds {
                let mut errors = vec![process_timeout_error(self.label, process_timeout)];
                if !output.stderr.trim().is_empty() {
                    errors.push(output.stderr.trim().to_string());
                }
                errors
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

impl<'a> GitTrackingAdapter<'a> {
    pub(crate) fn new() -> Self {
        Self {
            runner: &SYSTEM_PROCESS_RUNNER,
            timeout: GIT_TRACKING_TIMEOUT,
        }
    }

    #[cfg(test)]
    fn with_runner(runner: &'a dyn ProcessRunner) -> Self {
        Self {
            runner,
            timeout: GIT_TRACKING_TIMEOUT,
        }
    }

    pub(crate) fn config_dump_info_warning(
        &self,
        context: &WorkspaceContext,
        cancellation: &CancellationToken,
    ) -> ConfigDumpInfoGitCheck {
        if cancellation.is_cancelled() {
            return ConfigDumpInfoGitCheck::Cancelled;
        }
        let started = Instant::now();
        let deadline = started.checked_add(self.timeout).unwrap_or(started);

        let output = match self.runner.run(&ProcessCommand {
            program: PathBuf::from("git"),
            args: [
                "ls-files",
                "--cached",
                "--stage",
                "-z",
                "--",
                ":(icase)ConfigDumpInfo.xml",
                ":(icase,glob)**/ConfigDumpInfo.xml",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
            cwd: context.workspace_root.clone(),
            timeout: Some(self.timeout),
            cancellation: cancellation.clone(),
        }) {
            Ok(output) => output,
            Err(error) if cancellation.is_cancelled() || error.starts_with(CANCELLED_PREFIX) => {
                return ConfigDumpInfoGitCheck::Cancelled;
            }
            Err(_) => return ConfigDumpInfoGitCheck::Complete(None),
        };

        if output.cancelled || cancellation.is_cancelled() {
            return ConfigDumpInfoGitCheck::Cancelled;
        }
        if output.timed_out {
            return ConfigDumpInfoGitCheck::Complete(Some(format!(
                "ConfigDumpInfo.xml Git tracking check timed out after {} seconds; project inspection continued without tracking diagnostics",
                self.timeout.as_secs()
            )));
        }
        if output.stdout_truncated {
            return ConfigDumpInfoGitCheck::Complete(Some(
                "ConfigDumpInfo.xml Git tracking check exceeded its bounded output capture; inspect the Git index manually because the tracked-path list is incomplete"
                    .to_string(),
            ));
        }
        if output.stdout.contains('\u{fffd}') {
            return ConfigDumpInfoGitCheck::Complete(Some(
                "ConfigDumpInfo.xml Git tracking check returned non-UTF-8 paths; inspect the Git index manually because matching paths cannot be classified safely"
                    .to_string(),
            ));
        }
        if !output.status_success {
            return ConfigDumpInfoGitCheck::Complete(None);
        }

        let Some(index_paths) = parse_git_index_paths(&output.stdout) else {
            return ConfigDumpInfoGitCheck::Complete(Some(
                "ConfigDumpInfo.xml Git tracking check returned an unrecognized index record; inspect matching tracked paths manually"
                    .to_string(),
            ));
        };
        if index_paths.is_empty() {
            return ConfigDumpInfoGitCheck::Complete(None);
        }

        let mut runtime_paths = Vec::new();
        let mut ambiguous_paths = Vec::new();
        let mut blob_cache = BTreeMap::new();
        let mut entries = index_paths.into_iter();
        while let Some(entry) = entries.next() {
            if cancellation.is_cancelled() {
                return ConfigDumpInfoGitCheck::Cancelled;
            }
            if Instant::now() >= deadline {
                ambiguous_paths.push(entry.path);
                ambiguous_paths.extend(entries.map(|remaining| remaining.path));
                break;
            }
            let Some(oid) = entry.blob_oid else {
                ambiguous_paths.push(entry.path);
                continue;
            };
            let classification = if let Some(cached) = blob_cache.get(&oid) {
                *cached
            } else {
                let remaining = deadline.saturating_duration_since(Instant::now());
                let Some(remaining) = (!remaining.is_zero()).then_some(remaining) else {
                    ambiguous_paths.push(entry.path);
                    continue;
                };
                let classification = self.classify_git_blob(context, &oid, remaining, cancellation);
                if classification != GitBlobClassification::Cancelled {
                    blob_cache.insert(oid, classification);
                }
                classification
            };
            match classification {
                GitBlobClassification::Cancelled => {
                    return ConfigDumpInfoGitCheck::Cancelled;
                }
                GitBlobClassification::Classified(ConfigDumpInfoXmlKind::RuntimeSidecar) => {
                    runtime_paths.push(entry.path);
                }
                GitBlobClassification::Classified(
                    ConfigDumpInfoXmlKind::ExternalProcessor
                    | ConfigDumpInfoXmlKind::ExternalReport
                    | ConfigDumpInfoXmlKind::MetadataDescriptor,
                ) => {}
                GitBlobClassification::Classified(ConfigDumpInfoXmlKind::Other)
                | GitBlobClassification::Inconclusive => {
                    ambiguous_paths.push(entry.path);
                }
            }
        }

        ConfigDumpInfoGitCheck::Complete(config_dump_info_warnings(runtime_paths, ambiguous_paths))
    }

    fn classify_git_blob(
        &self,
        context: &WorkspaceContext,
        oid: &str,
        timeout: Duration,
        cancellation: &CancellationToken,
    ) -> GitBlobClassification {
        let output = match self.runner.run(&ProcessCommand {
            program: PathBuf::from("git"),
            args: ["--no-replace-objects", "cat-file", "blob", oid]
                .into_iter()
                .map(str::to_string)
                .collect(),
            cwd: context.workspace_root.clone(),
            timeout: Some(timeout),
            cancellation: cancellation.clone(),
        }) {
            Ok(output) => output,
            Err(error) if cancellation.is_cancelled() || error.starts_with(CANCELLED_PREFIX) => {
                return GitBlobClassification::Cancelled;
            }
            Err(_) => return GitBlobClassification::Inconclusive,
        };
        if output.cancelled || cancellation.is_cancelled() {
            return GitBlobClassification::Cancelled;
        }
        if output.timed_out
            || output.stdout_truncated
            || output.stdout.contains('\u{fffd}')
            || !output.status_success
        {
            return GitBlobClassification::Inconclusive;
        }
        GitBlobClassification::Classified(config_dump_info_xml_kind(output.stdout.as_bytes()))
    }
}

fn parse_git_index_paths(stdout: &str) -> Option<Vec<GitIndexPath>> {
    #[derive(Default)]
    struct EntryState {
        records: usize,
        blob_oid: Option<String>,
    }

    let mut entries = BTreeMap::<String, EntryState>::new();
    for record in stdout.split('\0').filter(|record| !record.is_empty()) {
        let (metadata, path) = record.split_once('\t')?;
        if path.is_empty() {
            return None;
        }
        let fields = metadata.split_whitespace().collect::<Vec<_>>();
        if fields.len() != 3 {
            return None;
        }
        let mode = fields[0];
        let oid = fields[1];
        let stage = fields[2];
        let usable_blob = matches!(mode, "100644" | "100755")
            && stage == "0"
            && !oid.is_empty()
            && oid.bytes().all(|byte| byte.is_ascii_hexdigit())
            && oid.bytes().any(|byte| byte != b'0');
        let entry = entries.entry(path.to_string()).or_default();
        entry.records += 1;
        if entry.records == 1 && usable_blob {
            entry.blob_oid = Some(oid.to_string());
        } else {
            entry.blob_oid = None;
        }
    }
    Some(
        entries
            .into_iter()
            .map(|(path, state)| GitIndexPath {
                path,
                blob_oid: state.blob_oid,
            })
            .collect(),
    )
}

fn config_dump_info_warnings(
    mut runtime_paths: Vec<String>,
    mut ambiguous_paths: Vec<String>,
) -> Option<String> {
    runtime_paths.sort();
    runtime_paths.dedup();
    ambiguous_paths.sort();
    ambiguous_paths.dedup();
    let mut warnings = Vec::new();
    if !runtime_paths.is_empty() {
        warnings.push(format!(
            "per-infobase ConfigDumpInfo.xml runtime state is tracked by Git at {}; from the workspace root, remove only these paths with `git rm --cached -- <path>` and add the same workspace-relative paths to that workspace's .gitignore",
            format_git_paths(runtime_paths.iter().map(String::as_str))
        ));
    }
    if !ambiguous_paths.is_empty() {
        warnings.push(manual_config_dump_info_warning(
            ambiguous_paths.iter().map(String::as_str),
            "the staged blob classification is inconclusive",
        ));
    }
    (!warnings.is_empty()).then(|| warnings.join("; "))
}

fn manual_config_dump_info_warning<'a>(
    paths: impl Iterator<Item = &'a str>,
    reason: &str,
) -> String {
    format!(
        "tracked ConfigDumpInfo.xml paths require manual review at {} because {reason}; keep platform-generated runtime sidecars out of Git, but do not untrack legitimate metadata object descriptors with the same filename",
        format_git_paths(paths)
    )
}

fn format_git_paths<'a>(paths: impl Iterator<Item = &'a str>) -> String {
    paths
        .map(|path| serde_json::to_string(path).expect("Git path serializes as JSON string"))
        .collect::<Vec<_>>()
        .join(", ")
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
        if let Some(outcome) = bind_external_processor_config(args, context, dry_run)? {
            return Ok(outcome);
        }
        let plugin_root = find_plugin_root(&context.cwd).ok_or_else(|| {
            "could not locate Unica plugin root for internal adapter lookup".to_string()
        })?;
        let report_args = runtime_args(args, true)?;
        let execution_args = runtime_args(args, false)?;
        let bundled_tool = resolve_bundled_tool(&plugin_root, "v8-runner", !dry_run)?;
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
            cancellation: cancellation.clone(),
        };
        let output = match self.runner.run(&process_command) {
            Ok(output) => output,
            Err(error) => {
                let error = redactor(&error);
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
        let stdout = redactor(&output.stdout);
        let stderr = redactor(&output.stderr);
        if output.cancelled {
            return Ok(cancelled_process_outcome(
                tool_name,
                stdout,
                stderr,
                Some(command),
            ));
        }
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
    pub fn invoke(
        action: RuntimeJobAction,
        tool_name: &str,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
        dry_run: bool,
    ) -> Result<RuntimeJobAdapterOutcome, String> {
        match action {
            RuntimeJobAction::Start => Self::start(tool_name, args, context, dry_run),
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
    ) -> Result<RuntimeJobAdapterOutcome, String> {
        let plugin_root = find_plugin_root(&context.cwd).ok_or_else(|| {
            "could not locate Unica plugin root for internal adapter lookup".to_string()
        })?;
        let reported_args = runtime_args(args, true)?;
        let execution_args = runtime_args(args, false)?;
        let bundled_tool = resolve_bundled_tool(&plugin_root, "v8-runner", !dry_run)?;
        let mut command = vec![bundled_tool.program.display().to_string()];
        command.extend(reported_args);

        if dry_run {
            return Ok(RuntimeJobAdapterOutcome {
                outcome: AdapterOutcome {
                    ok: true,
                    summary: format!("dry run: {tool_name} would start a durable runtime job"),
                    changes: vec!["no runtime job started because dryRun is true".to_string()],
                    warnings: bundled_tool.warnings,
                    errors: Vec::new(),
                    artifacts: Vec::new(),
                    stdout: None,
                    stderr: None,
                    command: Some(command),
                },
                job: None,
            });
        }

        let operation_name = args
            .get("operation")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("{tool_name} requires string `operation` argument"))?;
        let operation = RuntimeJobOperation::from_label(operation_name)?;
        let request = RuntimeJobRequest::new(
            operation,
            execution_args,
            runtime_job_safe_target(context),
            args.get("output")
                .and_then(Value::as_str)
                .map(str::to_string),
        );
        match runtime_jobs::start_detached_worker(
            context.cache_root.clone(),
            bundled_tool.program,
            context.cwd.clone(),
            request,
        ) {
            Ok(snapshot) => Ok(RuntimeJobAdapterOutcome {
                outcome: AdapterOutcome {
                    ok: true,
                    summary: format!("{tool_name} queued durable runtime job {}", snapshot.id),
                    changes: Vec::new(),
                    warnings: bundled_tool.warnings,
                    errors: Vec::new(),
                    artifacts: Vec::new(),
                    stdout: None,
                    stderr: None,
                    command: Some(command),
                },
                job: Some(runtime_job_snapshot_value(&snapshot)),
            }),
            Err(error) => Ok(Self::failure(tool_name, error, Some(command))),
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

impl<'a> CodeSearchAdapter<'a> {
    pub fn new() -> Self {
        Self {
            grep_runner: &SYSTEM_PROCESS_RUNNER,
            index_runner: &SYSTEM_INDEX_RUNNER,
            use_workspace_service: true,
        }
    }

    #[cfg(test)]
    pub fn with_runners(
        grep_runner: &'a dyn ProcessRunner,
        index_runner: &'a dyn IndexRunner,
    ) -> Self {
        Self {
            grep_runner,
            index_runner,
            use_workspace_service: false,
        }
    }

    #[allow(dead_code)]
    pub fn invoke(
        &self,
        tool_name: &str,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
        dry_run: bool,
    ) -> Result<AdapterOutcome, String> {
        self.invoke_cancellable(tool_name, args, context, dry_run, &CancellationToken::new())
    }

    pub fn invoke_cancellable(
        &self,
        tool_name: &str,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
        dry_run: bool,
        cancellation: &CancellationToken,
    ) -> Result<AdapterOutcome, String> {
        if cancellation.is_cancelled() {
            return Ok(AdapterOutcome::cancelled(format!(
                "{tool_name} cancelled before adapter work"
            )));
        }
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
            self.rlm_search(context, args, cancellation),
            self.git_grep_search(tool_name, args, context, cancellation),
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
        if let Some(error) = warnings
            .iter()
            .find(|error| error.starts_with(CANCELLED_PREFIX))
        {
            let mut outcome = AdapterOutcome::cancelled(
                error.strip_prefix(CANCELLED_PREFIX).unwrap_or(error).trim(),
            );
            outcome.stdout = Some(stdout);
            return Ok(outcome);
        }
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

    fn rlm_search(
        &self,
        context: &WorkspaceContext,
        args: &Map<String, Value>,
        cancellation: &CancellationToken,
    ) -> SearchBackendResult {
        match self.rlm_readiness(context, args, cancellation) {
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
        cancellation: &CancellationToken,
    ) -> SearchBackendResult {
        let grep_adapter = CodeNavigationAdapter {
            index_runner: self.index_runner,
            grep_runner: self.grep_runner,
            use_workspace_service: self.use_workspace_service,
        };
        match grep_adapter.grep(tool_name, args, context, cancellation) {
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
        cancellation: &CancellationToken,
    ) -> IndexReadiness {
        if self.use_workspace_service {
            match resolve_source_dir(context, args).and_then(|source_dir| {
                WorkspaceServiceManager::new().rlm_readiness_cancellable(
                    context,
                    &source_dir,
                    args,
                    cancellation,
                )
            }) {
                Ok(readiness) => readiness,
                Err(error) => IndexReadiness::Unavailable(error),
            }
        } else {
            WorkspaceIndexService::with_runner(self.index_runner).ready_index_cancellable(
                context,
                args,
                cancellation,
            )
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
    let diagnostics = if reason.starts_with(CANCELLED_PREFIX) {
        vec![reason.clone()]
    } else {
        vec![format!("{name} unavailable: {reason}")]
    };
    SearchBackendResult {
        section: format_section(name, &format!("unavailable: {reason}")),
        ok: false,
        diagnostics,
        artifacts: Vec::new(),
    }
}

fn failed_backend(name: &'static str, reason: impl Into<String>) -> SearchBackendResult {
    let reason = reason.into();
    let diagnostics = if reason.starts_with(CANCELLED_PREFIX) {
        vec![reason.clone()]
    } else {
        vec![format!("{name} failed: {reason}")]
    };
    SearchBackendResult {
        section: format_section(name, &format!("failed: {reason}")),
        ok: false,
        diagnostics,
        artifacts: Vec::new(),
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

    #[allow(dead_code)]
    pub fn invoke(
        &self,
        tool_name: &str,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
        dry_run: bool,
    ) -> Result<AdapterOutcome, String> {
        self.invoke_cancellable(tool_name, args, context, dry_run, &CancellationToken::new())
    }

    pub fn invoke_cancellable(
        &self,
        tool_name: &str,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
        dry_run: bool,
        cancellation: &CancellationToken,
    ) -> Result<AdapterOutcome, String> {
        if cancellation.is_cancelled() {
            return Ok(AdapterOutcome::cancelled(format!(
                "{tool_name} cancelled before adapter work"
            )));
        }
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
            "unica.code.definition" => self.definition(tool_name, args, context, cancellation),
            "unica.code.outline" => self.outline(tool_name, args, context, cancellation),
            "unica.code.grep" => self.grep(tool_name, args, context, cancellation),
            "unica.meta.profile" => self.meta_profile(tool_name, args, context, cancellation),
            _ => Err(format!("unsupported code navigation tool: {tool_name}")),
        }
    }

    fn definition(
        &self,
        tool_name: &str,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
        cancellation: &CancellationToken,
    ) -> Result<AdapterOutcome, String> {
        let readiness = self.rlm_readiness(context, args, cancellation);
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
        cancellation: &CancellationToken,
    ) -> Result<AdapterOutcome, String> {
        let candidates = index_path_candidates(context, args, "path")?;
        let readiness = self.rlm_readiness(context, args, cancellation);
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
        cancellation: &CancellationToken,
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
            cancellation: cancellation.clone(),
        })?;
        if output.cancelled {
            let body = grep_body(&output.stdout, mode, limit);
            return Ok(cancelled_process_outcome(
                tool_name,
                format_section("git-grep", &body),
                output.stderr,
                Some(std::iter::once("git".to_string()).chain(git_args).collect()),
            ));
        }
        let body = grep_body(&output.stdout, mode, limit);
        if output.timed_out {
            let timeout_error = process_timeout_error("git grep", Some(DEFAULT_PROCESS_TIMEOUT));
            let stderr_diagnostic = output.stderr.trim().to_string();
            let mut warnings = vec![timeout_error.clone()];
            if !body.is_empty() {
                warnings.push("partial git grep matches are incomplete".to_string());
            }
            let mut errors = vec![timeout_error];
            if !stderr_diagnostic.is_empty() {
                errors.push(stderr_diagnostic);
            }
            return Ok(AdapterOutcome {
                ok: false,
                summary: format!("{tool_name} timed out through git grep"),
                changes: Vec::new(),
                warnings,
                errors,
                artifacts: Vec::new(),
                stdout: Some(format_section("git-grep", &body)),
                stderr: if output.stderr.trim().is_empty() {
                    None
                } else {
                    Some(output.stderr)
                },
                command: Some(std::iter::once("git".to_string()).chain(git_args).collect()),
            });
        }
        let no_matches = body.is_empty()
            && !output.status_success
            && output.stderr.trim().is_empty()
            && process_exit_code_is(&output.status, 1);
        if !output.status_success && !no_matches {
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
            warnings: Vec::new(),
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
        cancellation: &CancellationToken,
    ) -> Result<AdapterOutcome, String> {
        let readiness = self.rlm_readiness(context, args, cancellation);
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
        cancellation: &CancellationToken,
    ) -> IndexReadiness {
        if self.use_workspace_service {
            match resolve_source_dir(context, args).and_then(|source_dir| {
                WorkspaceServiceManager::new().rlm_readiness_cancellable(
                    context,
                    &source_dir,
                    args,
                    cancellation,
                )
            }) {
                Ok(readiness) => readiness,
                Err(error) => IndexReadiness::Unavailable(error),
            }
        } else {
            WorkspaceIndexService::with_runner(self.index_runner).ready_index_cancellable(
                context,
                args,
                cancellation,
            )
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

    #[allow(dead_code)]
    pub fn invoke(
        &self,
        tool_name: &str,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
        dry_run: bool,
    ) -> Result<AdapterOutcome, String> {
        self.invoke_cancellable(tool_name, args, context, dry_run, &CancellationToken::new())
    }

    pub fn invoke_cancellable(
        &self,
        tool_name: &str,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
        dry_run: bool,
        cancellation: &CancellationToken,
    ) -> Result<AdapterOutcome, String> {
        if cancellation.is_cancelled() {
            return Ok(AdapterOutcome::cancelled(format!(
                "{tool_name} cancelled before adapter work"
            )));
        }
        if tool_name == "unica.code.diagnostics" && diagnostics_mode(args) == "analyze" {
            let cli_args = diagnostics_analyze_args(args);
            let process_timeout = diagnostics_analyze_timeout(args)?;
            return CliAdapter::with_runner(
                "bsl-analyzer",
                &["analyze"],
                "code analysis",
                self.process_runner,
            )
            .with_process_timeout(process_timeout)
            .invoke_cancellable(
                tool_name,
                &cli_args,
                context,
                dry_run,
                false,
                cancellation,
            );
        }

        let plugin_root = find_plugin_root(&context.cwd).ok_or_else(|| {
            "could not locate Unica plugin root for bsl-analyzer MCP adapter lookup".to_string()
        })?;
        let source_dir = resolve_source_dir(context, args)?;
        let (remote_tool, tool_args) = bsl_mcp_tool_request(tool_name, args)?;
        let bundled_tool = resolve_bundled_tool(&plugin_root, "bsl-analyzer", !dry_run)?;
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
    let category = metadata_kind(prefix)
        .or_else(|| metadata_kind_by_directory(prefix))
        .map(|kind| kind.tag)
        .unwrap_or_else(|| match prefix {
            "Документ" => "Document",
            "Справочник" => "Catalog",
            "ОбщийМодуль" | "ОбщиеМодули" => "CommonModule",
            "РегистрСведений" => "InformationRegister",
            "РегистрНакопления" => "AccumulationRegister",
            "Перечисление" => "Enum",
            other => other,
        });
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
    let warning = readiness_warning(readiness);
    if warning.starts_with(CANCELLED_PREFIX) {
        return AdapterOutcome::cancelled(
            warning
                .strip_prefix(CANCELLED_PREFIX)
                .unwrap_or(&warning)
                .trim(),
        );
    }
    AdapterOutcome {
        ok: true,
        summary: format!("{tool_name} could not read RLM index"),
        changes: Vec::new(),
        warnings: vec![warning],
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
        IndexReadiness::Stale { .. } | IndexReadiness::Building => "rlm index building".to_string(),
        IndexReadiness::Failed(error) | IndexReadiness::Unavailable(error)
            if error.starts_with(CANCELLED_PREFIX) =>
        {
            error
        }
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

fn diagnostics_analyze_timeout(args: &Map<String, Value>) -> Result<Duration, String> {
    let Some(value) = args.get("timeoutSeconds") else {
        return Ok(DEFAULT_PROCESS_TIMEOUT);
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
    resolve_source_root(context, args.get("sourceDir").and_then(Value::as_str))
        .map(|resolved| resolved.path)
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
        let output = ManagedChild::run(ManagedCommand {
            program: command.program.clone(),
            args: command.args.clone(),
            cwd: command.cwd.clone(),
            env: Vec::new(),
            timeout: command.timeout,
            cancellation: command.cancellation.clone(),
        })?;
        Ok(map_managed_process_output(output))
    }
}

fn map_managed_process_output(mut output: ManagedOutput) -> ProcessOutput {
    let stdout_truncated = output.stdout_truncated;
    ensure_truncation_diagnostics(&mut output);
    let output = ProcessOutput {
        status_success: output.status_success,
        status: output.status,
        stdout: output.stdout,
        stderr: output.stderr,
        timed_out: output.timed_out,
        cancelled: output.cancelled,
        stdout_truncated,
    };
    debug_assert!(!(output.timed_out && output.cancelled));
    output
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
    use crate::application::UnicaApplication;
    use crate::infrastructure::metadata_kinds::METADATA_KINDS;
    use crate::infrastructure::platform::testing;
    use crate::infrastructure::workspace_index::{IndexBackgroundJob, IndexCommand, IndexOutput};
    use rusqlite::Connection;
    use serde_json::json;
    use std::cell::RefCell;
    use std::fs;
    use std::io::Write;
    use std::path::Path;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn code_grep_does_not_start_rlm_index_side_effect() {
        let root = std::env::temp_dir().join(format!("unica-code-grep-{}", std::process::id()));
        let workspace = root.join("workspace");
        let module_dir = workspace.join("CommonModules/SmokeModule/Ext");
        std::fs::create_dir_all(&module_dir).unwrap();
        std::fs::write(
            module_dir.join("Module.bsl"),
            "Процедура SmokeProcedure() Экспорт\nКонецПроцедуры\n",
        )
        .unwrap();
        std::process::Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&workspace)
            .status()
            .unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(&workspace)
            .status()
            .unwrap();
        let mut args = Map::new();
        args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        args.insert(
            "query".to_string(),
            Value::String("SmokeProcedure".to_string()),
        );
        args.insert(
            "path".to_string(),
            Value::String("CommonModules".to_string()),
        );

        let result = UnicaApplication::new()
            .call_tool("unica.code.grep", &args)
            .unwrap();

        assert!(result.ok);
        assert!(result.stdout.unwrap().contains("SmokeProcedure"));
        let context = discover_workspace(Some(workspace.clone())).unwrap();
        assert!(
            !crate::infrastructure::workspace_index::status_path(&context).exists(),
            "unica.code.grep must not start or mark RLM index state"
        );
        assert!(
            !context.cache_root.join("services").exists(),
            "unica.code.grep must not start workspace analyzer services"
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn config_dump_info_git_check_uses_bounded_cancellable_process() {
        let context = temp_context("tracked-config-dump-info");
        let runner = RecordingProcessRunner {
            commands: RefCell::new(Vec::new()),
            output: ProcessOutput {
                status_success: true,
                status: "exit status: 0".to_string(),
                stdout: concat!(
                    "100644 0000000000000000000000000000000000000000 0\tnested/ConfigDumpInfo.xml\0",
                    "100644 0000000000000000000000000000000000000000 0\tsrc/ConfigDumpInfo.xml\0",
                )
                .to_string(),
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
            },
        };
        let cancellation = CancellationToken::new();

        let result = GitTrackingAdapter::with_runner(&runner)
            .config_dump_info_warning(&context, &cancellation);

        assert_eq!(
            result,
            ConfigDumpInfoGitCheck::Complete(Some(
                "tracked ConfigDumpInfo.xml paths require manual review at \"nested/ConfigDumpInfo.xml\", \"src/ConfigDumpInfo.xml\" because the staged blob classification is inconclusive; keep platform-generated runtime sidecars out of Git, but do not untrack legitimate metadata object descriptors with the same filename"
                    .to_string()
            ))
        );
        let commands = runner.commands.borrow();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].program, PathBuf::from("git"));
        assert_eq!(
            commands[0].args,
            [
                "ls-files",
                "--cached",
                "--stage",
                "-z",
                "--",
                ":(icase)ConfigDumpInfo.xml",
                ":(icase,glob)**/ConfigDumpInfo.xml",
            ]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>()
        );
        assert_eq!(commands[0].cwd, context.workspace_root);
        assert_eq!(commands[0].timeout, Some(GIT_TRACKING_TIMEOUT));
        assert!(!commands[0].cancellation.is_cancelled());

        let _ = fs::remove_dir_all(context.workspace_root);
    }

    #[test]
    fn config_dump_info_git_check_reports_truncated_index_output_as_incomplete() {
        let context = temp_context("tracked-config-dump-info-truncated");
        let runner = RecordingProcessRunner {
            commands: RefCell::new(Vec::new()),
            output: ProcessOutput {
                status_success: false,
                status: "exit status: 0".to_string(),
                stdout: "100644 aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa 0\tConfigDumpInfo.xml"
                    .to_string(),
                stderr: "stdout capture truncated".to_string(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: true,
            },
        };

        let result = GitTrackingAdapter::with_runner(&runner)
            .config_dump_info_warning(&context, &CancellationToken::new());

        let ConfigDumpInfoGitCheck::Complete(Some(warning)) = result else {
            panic!("truncated Git output must remain visible");
        };
        assert!(warning.contains("tracked-path list is incomplete"));
        assert!(!warning.contains("git rm --cached"));

        let _ = fs::remove_dir_all(context.workspace_root);
    }

    #[test]
    fn config_dump_info_git_check_does_not_suggest_removal_when_blob_is_truncated() {
        let context = temp_context("tracked-config-dump-info-truncated-blob");
        fs::create_dir_all(context.workspace_root.join("epf")).unwrap();
        fs::write(
            context.workspace_root.join("v8project.yaml"),
            concat!(
                "format: DESIGNER\n",
                "source-set:\n",
                "  - name: processors\n",
                "    type: EXTERNAL_DATA_PROCESSORS\n",
                "    path: epf\n",
            ),
        )
        .unwrap();
        let runner = SequenceProcessRunner {
            commands: RefCell::new(Vec::new()),
            outputs: RefCell::new(vec![
                ProcessOutput {
                    status_success: true,
                    status: "exit status: 0".to_string(),
                    stdout: "100644 aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa 0\tepf/ConfigDumpInfo.xml\0"
                        .to_string(),
                    stderr: String::new(),
                    timed_out: false,
                    cancelled: false,
                    stdout_truncated: false,
                },
                ProcessOutput {
                    status_success: false,
                    status: "exit status: 0".to_string(),
                    stdout: "<MetaDataObject>".to_string(),
                    stderr: "stdout capture truncated".to_string(),
                    timed_out: false,
                    cancelled: false,
                    stdout_truncated: true,
                },
            ]),
        };

        let result = GitTrackingAdapter::with_runner(&runner)
            .config_dump_info_warning(&context, &CancellationToken::new());

        let ConfigDumpInfoGitCheck::Complete(Some(warning)) = result else {
            panic!("truncated index blob must require manual review");
        };
        assert!(warning.contains("manual review"));
        assert!(!warning.contains("git rm --cached"));
        assert_eq!(runner.commands.borrow().len(), 2);
        assert_eq!(
            runner.commands.borrow()[1].args,
            [
                "--no-replace-objects",
                "cat-file",
                "blob",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            ]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>()
        );

        let lossy_runner = SequenceProcessRunner {
            commands: RefCell::new(Vec::new()),
            outputs: RefCell::new(vec![
                ProcessOutput {
                    status_success: true,
                    status: "exit status: 0".to_string(),
                    stdout: "100644 aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa 0\tepf/ConfigDumpInfo.xml\0"
                        .to_string(),
                    stderr: String::new(),
                    timed_out: false,
                    cancelled: false,
                    stdout_truncated: false,
                },
                ProcessOutput {
                    status_success: true,
                    status: "exit status: 0".to_string(),
                    stdout: "<MetaDataObject><ExternalDataProcessor><Comment>\u{fffd}</Comment></ExternalDataProcessor></MetaDataObject>"
                        .to_string(),
                    stderr: String::new(),
                    timed_out: false,
                    cancelled: false,
                    stdout_truncated: false,
                },
            ]),
        };

        let result = GitTrackingAdapter::with_runner(&lossy_runner)
            .config_dump_info_warning(&context, &CancellationToken::new());

        let ConfigDumpInfoGitCheck::Complete(Some(warning)) = result else {
            panic!("lossy index blob must require manual review");
        };
        assert!(warning.contains("manual review"));
        assert!(!warning.contains("git rm --cached"));

        let _ = fs::remove_dir_all(context.workspace_root);
    }

    #[test]
    fn config_dump_info_index_parser_marks_unmerged_and_intent_to_add_as_ambiguous() {
        let entries = parse_git_index_paths(concat!(
            "100644 aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa 1\tconflict/ConfigDumpInfo.xml\0",
            "100644 bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb 2\tconflict/ConfigDumpInfo.xml\0",
            "100644 0000000000000000000000000000000000000000 0\tnew/ConfigDumpInfo.xml\0",
            "100644 cccccccccccccccccccccccccccccccccccccccc 0\tvalid/ConfigDumpInfo.xml\0",
        ))
        .unwrap();

        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].path, "conflict/ConfigDumpInfo.xml");
        assert_eq!(entries[0].blob_oid, None);
        assert_eq!(entries[1].path, "new/ConfigDumpInfo.xml");
        assert_eq!(entries[1].blob_oid, None);
        assert_eq!(
            entries[2].blob_oid.as_deref(),
            Some("cccccccccccccccccccccccccccccccccccccccc")
        );
    }

    #[test]
    fn config_dump_info_warning_escapes_unusual_git_paths() {
        assert_eq!(
            format_git_paths(
                [
                    "line\nbreak/ConfigDumpInfo.xml",
                    "comma,path/ConfigDumpInfo.xml"
                ]
                .into_iter()
            ),
            r#""line\nbreak/ConfigDumpInfo.xml", "comma,path/ConfigDumpInfo.xml""#
        );
    }

    #[test]
    fn config_dump_info_git_check_keeps_unmerged_runtime_path_non_destructive() {
        let context = temp_context("tracked-config-dump-info-unmerged-runtime");
        fs::create_dir_all(context.workspace_root.join("src")).unwrap();
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
            context.workspace_root.join("src/Configuration.xml"),
            "<MetaDataObject/>",
        )
        .unwrap();
        let runner = RecordingProcessRunner {
            commands: RefCell::new(Vec::new()),
            output: ProcessOutput {
                status_success: true,
                status: "exit status: 0".to_string(),
                stdout: concat!(
                    "100644 aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa 1\tsrc/ConfigDumpInfo.xml\0",
                    "100644 bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb 2\tsrc/ConfigDumpInfo.xml\0",
                )
                .to_string(),
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
            },
        };

        let result = GitTrackingAdapter::with_runner(&runner)
            .config_dump_info_warning(&context, &CancellationToken::new());

        let ConfigDumpInfoGitCheck::Complete(Some(warning)) = result else {
            panic!("unmerged index stages must require manual review");
        };
        assert!(warning.contains("manual review"));
        assert!(warning.contains("src/ConfigDumpInfo.xml"));
        assert!(!warning.contains("git rm --cached"));
        assert_eq!(runner.commands.borrow().len(), 1);

        let _ = fs::remove_dir_all(context.workspace_root);
    }

    #[test]
    fn config_dump_info_git_check_rejects_lossy_index_paths() {
        let context = temp_context("tracked-config-dump-info-lossy-path");
        let runner = RecordingProcessRunner {
            commands: RefCell::new(Vec::new()),
            output: ProcessOutput {
                status_success: true,
                status: "exit status: 0".to_string(),
                stdout: "100644 aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa 0\tbad\u{fffd}/ConfigDumpInfo.xml\0"
                    .to_string(),
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
            },
        };

        let result = GitTrackingAdapter::with_runner(&runner)
            .config_dump_info_warning(&context, &CancellationToken::new());

        let ConfigDumpInfoGitCheck::Complete(Some(warning)) = result else {
            panic!("lossy Git paths must remain visible");
        };
        assert!(warning.contains("non-UTF-8 paths"));
        assert!(!warning.contains("git rm --cached"));

        let _ = fs::remove_dir_all(context.workspace_root);
    }

    #[test]
    fn config_dump_info_git_check_propagates_process_cancellation() {
        let context = temp_context("tracked-config-dump-info-cancelled");
        let runner = RecordingProcessRunner {
            commands: RefCell::new(Vec::new()),
            output: ProcessOutput {
                status_success: false,
                status: "cancelled".to_string(),
                stdout: String::new(),
                stderr: String::new(),
                timed_out: false,
                cancelled: true,
                stdout_truncated: false,
            },
        };

        let result = GitTrackingAdapter::with_runner(&runner)
            .config_dump_info_warning(&context, &CancellationToken::new());

        assert_eq!(result, ConfigDumpInfoGitCheck::Cancelled);
        assert_eq!(
            runner.commands.borrow()[0].timeout,
            Some(GIT_TRACKING_TIMEOUT)
        );

        let _ = fs::remove_dir_all(context.workspace_root);
    }

    #[test]
    fn config_dump_info_git_check_reports_timeout_without_failing_inspection() {
        let context = temp_context("tracked-config-dump-info-timeout");
        let runner = RecordingProcessRunner {
            commands: RefCell::new(Vec::new()),
            output: ProcessOutput {
                status_success: false,
                status: "timed out".to_string(),
                stdout: String::new(),
                stderr: String::new(),
                timed_out: true,
                cancelled: false,
                stdout_truncated: false,
            },
        };

        let result = GitTrackingAdapter::with_runner(&runner)
            .config_dump_info_warning(&context, &CancellationToken::new());

        let ConfigDumpInfoGitCheck::Complete(Some(warning)) = result else {
            panic!("timeout should remain a non-fatal project warning");
        };
        assert!(warning.contains("timed out after 5 seconds"));

        let _ = fs::remove_dir_all(context.workspace_root);
    }

    #[test]
    fn metadata_profile_selector_normalizes_every_registry_tag_and_directory() {
        for kind in METADATA_KINDS {
            let tag_selector = format!("{}.ObjectName", kind.tag);
            assert_eq!(
                split_profile_name(&tag_selector),
                (Some(kind.tag.to_string()), "ObjectName".to_string()),
                "canonical tag selector must use the registry: {tag_selector}"
            );

            let directory_selector = format!("{}.ObjectName", kind.directory);
            assert_eq!(
                split_profile_name(&directory_selector),
                (Some(kind.tag.to_string()), "ObjectName".to_string()),
                "plural directory selector must use the registry: {directory_selector}"
            );
        }

        for (alias, expected) in [
            ("Документ", "Document"),
            ("Справочник", "Catalog"),
            ("ОбщийМодуль", "CommonModule"),
            ("ОбщиеМодули", "CommonModule"),
            ("РегистрСведений", "InformationRegister"),
            ("РегистрНакопления", "AccumulationRegister"),
            ("Перечисление", "Enum"),
        ] {
            let selector = format!("{alias}.ObjectName");
            assert_eq!(
                split_profile_name(&selector),
                (Some(expected.to_string()), "ObjectName".to_string()),
                "Russian alias must remain supported: {selector}"
            );
        }
        assert_eq!(
            split_profile_name("SyntheticMetadata.Unknown"),
            (Some("SyntheticMetadata".to_string()), "Unknown".to_string())
        );
    }

    #[test]
    fn metadata_profile_selector_has_no_local_english_kind_table() {
        let source = include_str!("internal_adapters.rs");
        let selector_body = source
            .split_once("fn split_profile_name")
            .and_then(|(_, tail)| tail.split_once("fn format_profile_section"))
            .map(|(body, _)| body)
            .expect("split_profile_name source must be present");
        let local_match_patterns = selector_body
            .lines()
            .filter_map(|line| line.split_once("=>").map(|(pattern, _)| pattern))
            .collect::<Vec<_>>()
            .join("\n");

        for kind in METADATA_KINDS {
            assert!(
                !local_match_patterns.contains(&format!("\"{}\"", kind.tag)),
                "{} must be resolved through the shared registry",
                kind.tag
            );
            assert!(
                !local_match_patterns.contains(&format!("\"{}\"", kind.directory)),
                "{} must be resolved through the shared registry",
                kind.directory
            );
        }
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
            vec!["build", "--full-rebuild", "--source-set", "main"]
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
                cancelled: false,
                stdout_truncated: false,
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
    fn code_search_adapter_dry_run_reports_typed_code_search() {
        let context = discover_workspace(Some(std::env::current_dir().unwrap())).unwrap();
        let grep = FakeProcessRunner {
            output: ProcessOutput {
                status_success: true,
                status: "exit status: 0".to_string(),
                stdout: "ignored".to_string(),
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
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
                cancelled: false,
                stdout_truncated: false,
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
    fn code_search_adapter_returns_rlm_then_git_grep_without_analyzer_search() {
        let context = temp_context("search-three-backends");
        fs::create_dir_all(context.workspace_root.join("src/CommonModules")).unwrap();
        let db_path = context.cache_root.join("rlm-tools-bsl/test/bsl_index.db");
        create_rlm_search_db(&db_path);
        let runner = RecordingProcessRunner {
            commands: RefCell::new(Vec::new()),
            output: ProcessOutput {
                status_success: true,
                status: "exit status: 0".to_string(),
                stdout: "CommonModules/Проведение.bsl:42:ОбработкаПроведения\n".to_string(),
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
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
        assert!(!stdout.contains("=== bsl-analyzer ==="));
        assert!(stdout.find("=== rlm ===").unwrap() < stdout.find("=== git grep ===").unwrap());
        assert!(stdout.contains("=== rlm ==="));
        assert!(stdout.contains("=== git grep ==="));
        assert!(stdout.contains("CommonModules/Проведение.bsl:42"));
        assert!(stdout.contains("Procedure ОбработкаПроведения() export"));
        assert!(!stdout.contains("=== git-grep ==="));
        cleanup_context(&context);
    }

    #[test]
    fn code_search_adapter_keeps_unavailable_sections_when_git_grep_succeeds() {
        let context = temp_context("search-unavailable");
        let grep = FakeProcessRunner {
            output: ProcessOutput {
                status_success: true,
                status: "exit status: 0".to_string(),
                stdout: "CommonModules/SmokeModule/Ext/Module.bsl:2:ОбработкаПроведения\n"
                    .to_string(),
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
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
        assert!(!stdout.contains("=== bsl-analyzer ==="));
        assert!(stdout.contains("=== rlm ===\nunavailable: rlm index unavailable"));
        assert!(stdout.contains("=== git grep ==="));
        assert!(stdout.contains("CommonModules/SmokeModule/Ext/Module.bsl:2"));
        cleanup_context(&context);
    }

    #[test]
    fn code_search_adapter_returns_two_failed_sections_when_no_backend_runs() {
        let context = temp_context("search-all-failed");
        let grep = FakeProcessRunner {
            output: ProcessOutput {
                status_success: false,
                status: "exit status: 128".to_string(),
                stdout: String::new(),
                stderr: "fatal: not a git repository (or any of the parent directories): .git\n"
                    .to_string(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
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
        assert!(!stdout.contains("=== bsl-analyzer ==="));
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
                cancelled: false,
                stdout_truncated: false,
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
                cancelled: false,
                stdout_truncated: false,
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
                cancelled: false,
                stdout_truncated: false,
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
                cancelled: false,
                stdout_truncated: false,
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
                cancelled: false,
                stdout_truncated: false,
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
                cancelled: false,
                stdout_truncated: false,
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
                cancelled: false,
                stdout_truncated: false,
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
    fn code_grep_adapter_applies_limit_globally_and_sets_process_timeout() {
        let context = temp_context("grep-global-limit");
        let index = FakeIndexRunner::default();
        let grep = RecordingProcessRunner {
            commands: RefCell::new(Vec::new()),
            output: ProcessOutput {
                status_success: true,
                status: "exit status: 0".to_string(),
                stdout: (0..25)
                    .map(|index| format!("CommonModules/Module{index}/Ext/Module.bsl:1:Needle"))
                    .collect::<Vec<_>>()
                    .join("\n"),
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
            },
        };
        let mut args = Map::new();
        args.insert("query".to_string(), json!("Needle"));
        args.insert("limit".to_string(), json!(5));

        let outcome = CodeNavigationAdapter::with_runners(&index, &grep)
            .invoke("unica.code.grep", &args, &context, false)
            .unwrap();

        assert!(outcome.ok);
        let stdout = outcome.stdout.unwrap();
        let matches = stdout
            .strip_prefix("=== git-grep ===\n")
            .unwrap()
            .lines()
            .collect::<Vec<_>>();
        assert_eq!(matches.len(), 5);
        assert!(matches[4].contains("Module4"));
        assert!(!stdout.contains("Module5"));
        let commands = grep.commands.borrow();
        assert_eq!(commands[0].timeout, Some(DEFAULT_PROCESS_TIMEOUT));
        cleanup_context(&context);
    }

    #[test]
    fn code_grep_adapter_reports_partial_timeout_as_failure() {
        let context = temp_context("grep-partial-timeout");
        let index = FakeIndexRunner::default();
        let grep = FakeProcessRunner {
            output: ProcessOutput {
                status_success: false,
                status: "timeout".to_string(),
                stdout: "CommonModules/One.bsl:1:Needle\nCommonModules/Two.bsl:1:Needle\n"
                    .to_string(),
                stderr: "[unica: stdout capture truncated; result is not parseable]\n".to_string(),
                timed_out: true,
                cancelled: false,
                stdout_truncated: false,
            },
        };
        let mut args = Map::new();
        args.insert("query".to_string(), json!("Needle"));
        args.insert("limit".to_string(), json!(5));

        let outcome = CodeNavigationAdapter::with_runners(&index, &grep)
            .invoke("unica.code.grep", &args, &context, false)
            .unwrap();

        assert!(!outcome.ok);
        assert!(outcome
            .errors
            .iter()
            .any(|error| error.contains("timed out after 120 seconds")));
        assert!(outcome
            .errors
            .iter()
            .any(|error| error.contains("stdout capture truncated")));
        assert!(outcome
            .warnings
            .iter()
            .any(|warning| warning.contains("partial") && warning.contains("incomplete")));
        assert!(outcome
            .stdout
            .as_deref()
            .is_some_and(|stdout| stdout.contains("CommonModules/One.bsl")));
        assert!(outcome
            .stderr
            .as_deref()
            .is_some_and(|stderr| stderr.contains("stdout capture truncated")));
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
                cancelled: false,
                stdout_truncated: false,
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
                cancelled: false,
                stdout_truncated: false,
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
    fn diagnostics_analyze_uses_custom_timeout_without_forwarding_cli_argument() {
        let context = temp_context("diagnostics-custom-timeout");
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
            },
        };
        let mut args = Map::new();
        args.insert("timeoutSeconds".to_string(), json!(900));

        let outcome = BslAnalyzerMcpAdapter::with_process_runner(&runner)
            .invoke("unica.code.diagnostics", &args, &context, false)
            .unwrap();

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
            output: ProcessOutput {
                status_success: true,
                status: "exit status: 0".to_string(),
                stdout: String::new(),
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
            },
        };

        let outcome = BslAnalyzerMcpAdapter::with_process_runner(&runner)
            .invoke("unica.code.diagnostics", &Map::new(), &context, false)
            .unwrap();

        assert!(outcome.ok);
        assert_eq!(
            runner.commands.borrow()[0].timeout,
            Some(DEFAULT_PROCESS_TIMEOUT)
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
            },
        };
        let mut args = Map::new();
        args.insert("timeoutSeconds".to_string(), json!(900));

        let outcome = BslAnalyzerMcpAdapter::with_process_runner(&runner)
            .invoke("unica.code.diagnostics", &args, &context, false)
            .unwrap();

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
        assert!(commands[0].args.contains(
            &normalize_path_identity(&context.cwd)
                .unwrap()
                .display()
                .to_string()
        ));
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
                cancelled: false,
                stdout_truncated: false,
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
        let context = temp_context("runtime-cancelled-output");
        let runner = FakeProcessRunner {
            output: ProcessOutput {
                status_success: false,
                status: "cancelled".to_string(),
                stdout: String::new(),
                stderr: String::new(),
                timed_out: false,
                cancelled: true,
                stdout_truncated: false,
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
    fn cancellation_prefix_is_stable_for_cancelled_git_grep_output() {
        let context = temp_context("grep-cancelled-output");
        let index = FakeIndexRunner::default();
        let grep = FakeProcessRunner {
            output: ProcessOutput {
                status_success: false,
                status: "cancelled".to_string(),
                stdout: String::new(),
                stderr: String::new(),
                timed_out: false,
                cancelled: true,
                stdout_truncated: false,
            },
        };
        let mut args = Map::new();
        args.insert("query".to_string(), json!("SmokeProcedure"));

        let outcome = CodeNavigationAdapter::with_runners(&index, &grep)
            .invoke("unica.code.grep", &args, &context, false)
            .unwrap();

        assert!(outcome.errors[0].starts_with("cancelled:"));
        cleanup_context(&context);
    }

    #[test]
    fn cancellation_prefix_is_stable_when_code_search_backend_is_cancelled() {
        let context = temp_context("search-cancelled-output");
        let index = FakeIndexRunner::default();
        let grep = FakeProcessRunner {
            output: ProcessOutput {
                status_success: false,
                status: "cancelled".to_string(),
                stdout: String::new(),
                stderr: String::new(),
                timed_out: false,
                cancelled: true,
                stdout_truncated: false,
            },
        };
        let mut args = Map::new();
        args.insert("query".to_string(), json!("SmokeProcedure"));

        let outcome = CodeSearchAdapter::with_runners(&grep, &index)
            .invoke("unica.code.search", &args, &context, false)
            .unwrap();

        assert!(!outcome.ok);
        assert!(outcome.errors[0].starts_with("cancelled:"));
        cleanup_context(&context);
    }

    #[test]
    fn cancellation_prefix_is_stable_when_navigation_index_is_cancelled() {
        let context = temp_context("navigation-index-cancelled");
        let index = FakeIndexRunner {
            outputs: RefCell::new(vec![IndexOutput {
                status_success: false,
                status: "cancelled".to_string(),
                stdout: String::new(),
                stderr: String::new(),
                timed_out: false,
                cancelled: true,
                duration_ms: 0,
            }]),
            ..Default::default()
        };
        let grep = FakeProcessRunner {
            output: ProcessOutput {
                status_success: true,
                status: "exit status: 0".to_string(),
                stdout: String::new(),
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
            },
        };
        let mut args = Map::new();
        args.insert("name".to_string(), json!("SmokeProcedure"));

        let outcome = CodeNavigationAdapter::with_runners(&index, &grep)
            .invoke("unica.code.definition", &args, &context, false)
            .unwrap();

        assert!(!outcome.ok);
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
        let context = temp_context("runtime-timeout-no-local-budget");
        let runner = FakeProcessRunner {
            output: ProcessOutput {
                status_success: false,
                status: "timeout".to_string(),
                stdout: String::new(),
                stderr: String::new(),
                timed_out: true,
                cancelled: false,
                stdout_truncated: false,
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
                cancelled: false,
                stdout_truncated: false,
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

    struct SequenceProcessRunner {
        commands: RefCell<Vec<ProcessCommand>>,
        outputs: RefCell<Vec<ProcessOutput>>,
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
            cancelled: false,
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
        fs::write(
            root.join("v8project.yaml"),
            "source-set:\n  - name: main\n    type: CONFIGURATION\n    path: .\n",
        )
        .unwrap();
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
    });
    assert!(output.stderr.contains("stdout capture truncated"));
    assert!(output.stderr.contains("earlier stderr diagnostics omitted"));
}
