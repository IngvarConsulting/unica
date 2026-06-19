use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::legacy_scripts::{find_plugin_root, value_to_cli_string};
use crate::infrastructure::workspace_index::{
    IndexReadiness, IndexRunner, WorkspaceIndexService, SYSTEM_INDEX_RUNNER,
};
use crate::infrastructure::AdapterOutcome;
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;
use std::env;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

const DEFAULT_PROCESS_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone)]
pub struct ProcessCommand {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub timeout: Duration,
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
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
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
    launcher: &'static str,
    default_command: &'static [&'static str],
    label: &'static str,
    runner: &'a dyn ProcessRunner,
}

pub struct RuntimeAdapter<'a> {
    runner: &'a dyn ProcessRunner,
}

pub struct CodeSearchAdapter<'a> {
    analyzer_runner: &'a dyn ProcessRunner,
    index_runner: &'a dyn IndexRunner,
}

pub struct CodeNavigationAdapter<'a> {
    index_runner: &'a dyn IndexRunner,
    grep_runner: &'a dyn ProcessRunner,
}

pub struct BslAnalyzerMcpAdapter<'a> {
    runner: &'a dyn BslMcpRunner,
}

impl<'a> CliAdapter<'a> {
    pub fn new(
        launcher: &'static str,
        default_command: &'static [&'static str],
        label: &'static str,
    ) -> Self {
        Self {
            launcher,
            default_command,
            label,
            runner: &SYSTEM_PROCESS_RUNNER,
        }
    }

    pub fn with_runner(
        launcher: &'static str,
        default_command: &'static [&'static str],
        label: &'static str,
        runner: &'a dyn ProcessRunner,
    ) -> Self {
        Self {
            launcher,
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
        let launcher = plugin_root.join("scripts").join(self.launcher);
        let mut command = vec![launcher.display().to_string()];
        command.extend(self.default_command.iter().map(|part| (*part).to_string()));
        command.extend(cli_args(args, true)?);
        let execution_args = cli_args(args, false)?;

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
                warnings: if launcher.exists() {
                    Vec::new()
                } else {
                    vec![format!(
                        "internal adapter launcher not found: {}",
                        launcher.display()
                    )]
                },
                errors: Vec::new(),
                artifacts: Vec::new(),
                stdout: None,
                stderr: None,
                command: Some(command),
            });
        }

        if !launcher.exists() {
            return Err(format!(
                "internal adapter launcher not found: {}",
                launcher.display()
            ));
        }

        let mut process_args = self
            .default_command
            .iter()
            .map(|part| (*part).to_string())
            .collect::<Vec<_>>();
        process_args.extend(execution_args);
        let output = self.runner.run(&ProcessCommand {
            program: launcher.clone(),
            args: process_args,
            cwd: context.cwd.clone(),
            timeout: DEFAULT_PROCESS_TIMEOUT,
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
                vec![format!(
                    "internal {} adapter timed out after {} seconds",
                    self.label,
                    DEFAULT_PROCESS_TIMEOUT.as_secs()
                )]
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
        let launcher = plugin_root.join("scripts").join("run-v8-runner.sh");
        let report_args = runtime_args(args, true)?;
        let execution_args = runtime_args(args, false)?;
        let mut command = vec![launcher.display().to_string()];
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
                warnings: if launcher.exists() {
                    Vec::new()
                } else {
                    vec![format!(
                        "internal adapter launcher not found: {}",
                        launcher.display()
                    )]
                },
                errors: Vec::new(),
                artifacts: Vec::new(),
                stdout: None,
                stderr: None,
                command: Some(command),
            });
        }

        if !launcher.exists() {
            return Err(format!(
                "internal adapter launcher not found: {}",
                launcher.display()
            ));
        }

        let output = self.runner.run(&ProcessCommand {
            program: launcher.clone(),
            args: execution_args,
            cwd: context.cwd.clone(),
            timeout: DEFAULT_PROCESS_TIMEOUT,
        })?;
        let ok = output.status_success;
        Ok(AdapterOutcome {
            ok,
            summary: if ok {
                format!("{tool_name} completed through internal v8-runner runtime adapter")
            } else {
                format!("{tool_name} failed through internal v8-runner runtime adapter")
            },
            changes: if mutating {
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
            } else if output.stderr.trim().is_empty() && output.timed_out {
                vec![format!(
                    "internal v8-runner runtime adapter timed out after {} seconds",
                    DEFAULT_PROCESS_TIMEOUT.as_secs()
                )]
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

impl<'a> Default for RuntimeAdapter<'a> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> CodeSearchAdapter<'a> {
    pub fn new() -> Self {
        Self {
            analyzer_runner: &SYSTEM_PROCESS_RUNNER,
            index_runner: &SYSTEM_INDEX_RUNNER,
        }
    }

    pub fn with_runners(
        analyzer_runner: &'a dyn ProcessRunner,
        index_runner: &'a dyn IndexRunner,
    ) -> Self {
        Self {
            analyzer_runner,
            index_runner,
        }
    }

    pub fn invoke(
        &self,
        tool_name: &str,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
        dry_run: bool,
    ) -> Result<AdapterOutcome, String> {
        let mut analyzer = CliAdapter::with_runner(
            "run-bsl-analyzer.sh",
            &["search"],
            "code analysis",
            self.analyzer_runner,
        )
        .invoke(tool_name, args, context, dry_run, false)?;

        if dry_run {
            return Ok(analyzer);
        }

        let analyzer_stdout = analyzer.stdout.take().unwrap_or_default();
        let analyzer_stderr = analyzer.stderr.take();
        let mut stdout = format_section("bsl-analyzer", &analyzer_stdout);

        match WorkspaceIndexService::with_runner(self.index_runner).ready_index(context, args) {
            IndexReadiness::Ready { db_path } => match search_rlm_index(&db_path, args) {
                Ok(Some(rlm_stdout)) => {
                    stdout.push_str("\n\n");
                    stdout.push_str(&format_section("rlm", &rlm_stdout));
                }
                Ok(None) => {}
                Err(error) => analyzer
                    .warnings
                    .push(format!("rlm search failed: {error}")),
            },
            IndexReadiness::Building => analyzer.warnings.push("rlm index building".to_string()),
            IndexReadiness::Missing => analyzer
                .warnings
                .push("rlm index unavailable: index is missing".to_string()),
            IndexReadiness::Stale => analyzer.warnings.push("rlm index building".to_string()),
            IndexReadiness::Failed(error) => analyzer
                .warnings
                .push(format!("rlm index unavailable: {error}")),
            IndexReadiness::Unavailable(error) => analyzer
                .warnings
                .push(format!("rlm index unavailable: {error}")),
        }

        analyzer.stdout = Some(stdout);
        analyzer.stderr = analyzer_stderr;
        Ok(analyzer)
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
        }
    }

    pub fn with_runners(
        index_runner: &'a dyn IndexRunner,
        grep_runner: &'a dyn ProcessRunner,
    ) -> Self {
        Self {
            index_runner,
            grep_runner,
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
            _ => Err(format!("unsupported code navigation tool: {tool_name}")),
        }
    }

    fn definition(
        &self,
        tool_name: &str,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
    ) -> Result<AdapterOutcome, String> {
        let readiness =
            WorkspaceIndexService::with_runner(self.index_runner).ready_index(context, args);
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
        let readiness =
            WorkspaceIndexService::with_runner(self.index_runner).ready_index(context, args);
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

        let mut git_args = vec!["grep".to_string()];
        if mode == "files" {
            git_args.push("--name-only".to_string());
        } else {
            git_args.push("-n".to_string());
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
            timeout: DEFAULT_PROCESS_TIMEOUT,
        })?;
        let limit = read_limit(args, 200);
        let body = grep_body(&output.stdout, mode, limit);
        let no_matches = body.is_empty() && !output.status_success;
        if !output.status_success && !no_matches {
            return Ok(AdapterOutcome {
                ok: false,
                summary: format!("{tool_name} failed through git grep"),
                changes: Vec::new(),
                warnings: vec![format!("git grep exited with status {}", output.status)],
                errors: vec![output.stderr.trim().to_string()],
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
            return CliAdapter::new("run-bsl-analyzer.sh", &["analyze"], "code analysis")
                .invoke(tool_name, &cli_args, context, dry_run, false);
        }

        let plugin_root = find_plugin_root(&context.cwd).ok_or_else(|| {
            "could not locate Unica plugin root for bsl-analyzer MCP adapter lookup".to_string()
        })?;
        let launcher = plugin_root.join("scripts").join("run-bsl-analyzer.sh");
        let source_dir = resolve_source_dir(context, args)?;
        let command = bsl_mcp_command(&launcher, &source_dir, context, tool_name, args)?;
        let mut reported_command = vec![launcher.display().to_string()];
        reported_command.extend(command.args.clone());

        if dry_run {
            return Ok(AdapterOutcome {
                ok: true,
                summary: format!("dry run: {tool_name} would call typed bsl-analyzer MCP adapter"),
                changes: Vec::new(),
                warnings: if launcher.exists() {
                    Vec::new()
                } else {
                    vec![format!(
                        "internal adapter launcher not found: {}",
                        launcher.display()
                    )]
                },
                errors: Vec::new(),
                artifacts: vec![source_dir.display().to_string()],
                stdout: None,
                stderr: None,
                command: Some(reported_command),
            });
        }

        if !launcher.exists() {
            return Err(format!(
                "internal adapter launcher not found: {}",
                launcher.display()
            ));
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
                Ok(format!(
                    "{} {}{}{} at {}",
                    method_type,
                    name,
                    format!("({})", params.unwrap_or_default().trim()),
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
        if mode == "files" {
            if !seen.insert(line.to_string()) {
                continue;
            }
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
            filtered.insert(key.to_string(), value.clone());
        }
    }
    filtered
}

fn bsl_mcp_command(
    launcher: &Path,
    source_dir: &Path,
    context: &WorkspaceContext,
    tool_name: &str,
    args: &Map<String, Value>,
) -> Result<BslMcpCommand, String> {
    let (remote_tool, tool_args) = bsl_mcp_tool_request(tool_name, args)?;
    Ok(BslMcpCommand {
        program: launcher.to_path_buf(),
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
        timeout: DEFAULT_PROCESS_TIMEOUT,
        tool_name: remote_tool,
        tool_args,
    })
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

            if started.elapsed() >= command.timeout {
                let _ = child.kill();
                let output = child
                    .wait_with_output()
                    .map_err(|err| format!("failed to collect timed-out process output: {err}"))?;
                return Ok(ProcessOutput {
                    status_success: false,
                    status: "timeout".to_string(),
                    stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                    stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                    timed_out: true,
                });
            }

            std::thread::sleep(Duration::from_millis(25));
        }
    }
}

impl BslMcpRunner for SystemBslMcpRunner {
    fn call(&self, command: &BslMcpCommand) -> Result<BslMcpOutput, String> {
        let mut child = Command::new(&command.program)
            .args(&command.args)
            .current_dir(&command.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| format!("failed to execute bsl-analyzer MCP process: {err}"))?;

        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| "failed to open bsl-analyzer MCP stdin".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "failed to open bsl-analyzer MCP stdout".to_string())?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| "failed to open bsl-analyzer MCP stderr".to_string())?;

        let (tx, rx) = mpsc::channel::<String>();
        let stdout_reader = thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) => break,
                    Ok(_) => {
                        if tx.send(line).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        let stderr_reader = thread::spawn(move || {
            let mut reader = BufReader::new(stderr);
            let mut text = String::new();
            let _ = reader.read_to_string(&mut text);
            text
        });

        let result = (|| {
            send_mcp_json(
                &mut stdin,
                &json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "initialize",
                    "params": {
                        "protocolVersion": "2025-03-26",
                        "capabilities": {},
                        "clientInfo": {
                            "name": "unica",
                            "version": env!("CARGO_PKG_VERSION")
                        }
                    }
                }),
            )?;
            let _ = read_mcp_response(&rx, 1, command.timeout)?;
            send_mcp_json(
                &mut stdin,
                &json!({
                    "jsonrpc": "2.0",
                    "method": "notifications/initialized"
                }),
            )?;
            send_mcp_json(
                &mut stdin,
                &json!({
                    "jsonrpc": "2.0",
                    "id": 2,
                    "method": "tools/call",
                    "params": {
                        "name": command.tool_name,
                        "arguments": command.tool_args
                    }
                }),
            )?;
            let response = read_mcp_response(&rx, 2, command.timeout)?;
            mcp_tool_text(&response)
        })();

        let _ = child.kill();
        let _ = child.wait();
        let _ = stdout_reader.join();
        let stderr = stderr_reader.join().unwrap_or_default();

        result.map(|result_text| BslMcpOutput {
            result_text,
            stderr,
        })
    }
}

fn send_mcp_json(stdin: &mut impl Write, payload: &Value) -> Result<(), String> {
    stdin
        .write_all(payload.to_string().as_bytes())
        .and_then(|_| stdin.write_all(b"\n"))
        .and_then(|_| stdin.flush())
        .map_err(|err| format!("failed to write bsl-analyzer MCP request: {err}"))
}

fn read_mcp_response(
    rx: &mpsc::Receiver<String>,
    id: i64,
    timeout: Duration,
) -> Result<Value, String> {
    let started = Instant::now();
    while started.elapsed() < timeout {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(line) => {
                let Ok(value) = serde_json::from_str::<Value>(line.trim()) else {
                    continue;
                };
                if value.get("id").and_then(Value::as_i64) == Some(id) {
                    return Ok(value);
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err("bsl-analyzer MCP stdout closed before response".to_string());
            }
        }
    }
    Err(format!("bsl-analyzer MCP request {id} timed out"))
}

fn mcp_tool_text(response: &Value) -> Result<String, String> {
    if let Some(error) = response.get("error") {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("bsl-analyzer MCP JSON-RPC error");
        return Err(message.to_string());
    }
    let result = response
        .get("result")
        .ok_or_else(|| "bsl-analyzer MCP response is missing result".to_string())?;
    if let Some(content) = result.get("content").and_then(Value::as_array) {
        let parts = content
            .iter()
            .filter_map(|item| item.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>();
        if !parts.is_empty() {
            return Ok(parts.join("\n"));
        }
    }
    Ok(result.to_string())
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
    let mut result = Vec::new();

    append_runtime_global_args(&mut result, operation, args, redact);

    match operation {
        "config-init" => {
            result.extend(["config".to_string(), "init".to_string()]);
            append_arg(&mut result, "--output", args, "config", redact);
            append_arg(&mut result, "--connection", args, "connection", redact);
            append_arg(&mut result, "--format", args, "format", redact);
            append_arg(&mut result, "--builder", args, "builder", redact);
        }
        "init" => result.push("init".to_string()),
        "build" => {
            result.push("build".to_string());
            append_bool_flag(&mut result, "--full-rebuild", args, "fullRebuild");
            append_arg(&mut result, "--source-set", args, "sourceSet", redact);
            append_arg(&mut result, "--extension", args, "extension", redact);
        }
        "dump" => {
            result.push("dump".to_string());
            append_arg(&mut result, "--mode", args, "mode", redact);
            append_arg(&mut result, "--object", args, "object", redact);
            append_arg(&mut result, "--source-set", args, "sourceSet", redact);
            append_arg(&mut result, "--extension", args, "extension", redact);
        }
        "convert" => {
            result.push("convert".to_string());
            append_arg(&mut result, "--source-set", args, "sourceSet", redact);
            append_arg(&mut result, "--output", args, "output", redact);
            append_arg(&mut result, "--path", args, "path", redact);
            append_arg(&mut result, "--format", args, "format", redact);
            append_arg(&mut result, "--extension", args, "extension", redact);
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
            append_bool_flag(&mut result, "--server", args, "server");
            append_bool_flag(&mut result, "--thin-client", args, "thinClient");
        }
        "test" => {
            result.push("test".to_string());
            if let Some(test_runner) = string_arg(args, "testRunner", redact) {
                result.push(test_runner);
            }
            append_bool_flag(&mut result, "--full", args, "fullRebuild");
            if let Some(test_scope) = string_arg(args, "testScope", redact) {
                result.push(test_scope);
            }
            if let Some(module) = string_arg(args, "module", redact) {
                result.push(module);
            }
            append_arg(&mut result, "--source-set", args, "sourceSet", redact);
            append_arg(&mut result, "--extension", args, "extension", redact);
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
                Some(client_mode) => result.push(client_mode.to_string()),
                None => {}
            }
        }
        "extensions" => {
            result.push("extensions".to_string());
            append_arg(&mut result, "--name", args, "sourceSet", redact);
            append_arg(&mut result, "--extension", args, "extension", redact);
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
    key.contains("password") || key.contains("token") || key.contains("secret")
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
        let context = WorkspaceContext::discover(std::env::current_dir().unwrap()).unwrap();
        let mut args = Map::new();
        args.insert("sourceSet".to_string(), json!("main"));

        let outcome = CliAdapter::new("run-v8-runner.sh", &["build"], "build/runtime")
            .invoke("unica.build.load", &args, &context, true, true)
            .unwrap();

        let command = outcome.command.unwrap().join(" ");
        assert!(command.contains("run-v8-runner.sh"));
        assert!(command.contains("build"));
        assert!(command.contains("--source-set main"));
    }

    #[test]
    fn runtime_adapter_maps_build_to_allowlisted_v8_runner_argv() {
        let context = WorkspaceContext::discover(std::env::current_dir().unwrap()).unwrap();
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
            vec!["build", "--full-rebuild", "--source-set", "main"]
        );
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
        test_args.insert("fullRebuild".to_string(), json!(true));
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
        ];

        for (input, expected) in cases {
            let args = input.as_object().unwrap().clone();
            assert_eq!(runtime_args(&args, false).unwrap(), expected);
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
        let context = WorkspaceContext::discover(std::env::current_dir().unwrap()).unwrap();
        let mut args = Map::new();
        args.insert("query".to_string(), json!("ОбщийМодуль"));

        let outcome = CliAdapter::new("run-bsl-analyzer.sh", &["search"], "code analysis")
            .invoke("unica.code.search", &args, &context, true, false)
            .unwrap();

        let command = outcome.command.unwrap().join(" ");
        assert!(command.contains("run-bsl-analyzer.sh"));
        assert!(command.contains("search"));
        assert!(command.contains("--query"));
        assert!(command.contains("ОбщийМодуль"));
    }

    #[test]
    fn code_search_adapter_dry_run_builds_bsl_analyzer_search_command() {
        let context = WorkspaceContext::discover(std::env::current_dir().unwrap()).unwrap();
        let analyzer = FakeProcessRunner {
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

        let outcome = CodeSearchAdapter::with_runners(&analyzer, &index)
            .invoke("unica.code.search", &args, &context, true)
            .unwrap();

        let command = outcome.command.unwrap().join(" ");
        assert!(command.contains("run-bsl-analyzer.sh"));
        assert!(command.contains("search"));
        assert!(command.contains("--query"));
        assert!(command.contains("ОбработкаПроведения"));
    }

    #[test]
    fn code_search_adapter_returns_analyzer_section_when_rlm_index_is_missing() {
        let context = temp_context("search-missing");
        fs::create_dir_all(context.workspace_root.join("src/CommonModules")).unwrap();
        let analyzer = FakeProcessRunner {
            output: ProcessOutput {
                status_success: true,
                status: "exit status: 0".to_string(),
                stdout: "analyzer hit".to_string(),
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

        let outcome = CodeSearchAdapter::with_runners(&analyzer, &index)
            .invoke("unica.code.search", &args, &context, false)
            .unwrap();

        assert!(outcome.ok);
        assert_eq!(
            outcome.stdout.as_deref(),
            Some("=== bsl-analyzer ===\nanalyzer hit")
        );
        assert!(outcome
            .warnings
            .iter()
            .any(|warning| warning.contains("rlm index unavailable")));
        cleanup_context(&context);
    }

    #[test]
    fn code_search_adapter_adds_rlm_section_when_index_is_ready() {
        let context = temp_context("search-ready");
        fs::create_dir_all(context.workspace_root.join("src/CommonModules")).unwrap();
        let db_path = context.cache_root.join("rlm-tools-bsl/test/bsl_index.db");
        create_rlm_search_db(&db_path);
        let analyzer = FakeProcessRunner {
            output: ProcessOutput {
                status_success: true,
                status: "exit status: 0".to_string(),
                stdout: "analyzer hit".to_string(),
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

        let outcome = CodeSearchAdapter::with_runners(&analyzer, &index)
            .invoke("unica.code.search", &args, &context, false)
            .unwrap();

        let stdout = outcome.stdout.unwrap();
        assert!(stdout.contains("=== bsl-analyzer ===\nanalyzer hit"));
        assert!(stdout.contains("=== rlm ==="));
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
    fn diagnostics_adapter_still_builds_bsl_analyzer_analyze_command() {
        let context = WorkspaceContext::discover(std::env::current_dir().unwrap()).unwrap();
        let mut args = Map::new();
        args.insert("sourceDir".to_string(), json!("src"));

        let outcome = CliAdapter::new("run-bsl-analyzer.sh", &["analyze"], "code analysis")
            .invoke("unica.code.diagnostics", &args, &context, true, false)
            .unwrap();

        let command = outcome.command.unwrap().join(" ");
        assert!(command.contains("run-bsl-analyzer.sh"));
        assert!(command.contains("analyze"));
        assert!(command.contains("--source-dir src"));
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

        let error = CliAdapter::new("run-v8-runner.sh", &["build"], "build/runtime")
            .invoke("unica.build.load", &args, &context, true, true)
            .unwrap_err();

        assert!(error.contains("raw args are not accepted"));
    }

    #[test]
    fn cli_adapter_redacts_secret_values_from_reported_command() {
        let context = WorkspaceContext::discover(std::env::current_dir().unwrap()).unwrap();
        let mut args = Map::new();
        args.insert("dbPassword".to_string(), json!("super-secret"));

        let outcome = CliAdapter::new("run-v8-runner.sh", &["build"], "build/runtime")
            .invoke("unica.build.load", &args, &context, true, true)
            .unwrap();

        let command = outcome.command.unwrap().join(" ");
        assert!(command.contains("--db-password <redacted>"));
        assert!(!command.contains("super-secret"));
    }

    #[test]
    fn cli_adapter_uses_fake_process_runner_for_status_and_output_contract() {
        let context = WorkspaceContext::discover(std::env::current_dir().unwrap()).unwrap();
        let runner = FakeProcessRunner {
            output: ProcessOutput {
                status_success: false,
                status: "exit status: 2".to_string(),
                stdout: "partial stdout".to_string(),
                stderr: "failure stderr".to_string(),
                timed_out: false,
            },
        };

        let outcome =
            CliAdapter::with_runner("run-v8-runner.sh", &["build"], "build/runtime", &runner)
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
    }

    #[test]
    fn cli_adapter_reports_fake_process_timeout() {
        let context = WorkspaceContext::discover(std::env::current_dir().unwrap()).unwrap();
        let runner = FakeProcessRunner {
            output: ProcessOutput {
                status_success: false,
                status: "timeout".to_string(),
                stdout: String::new(),
                stderr: String::new(),
                timed_out: true,
            },
        };

        let outcome =
            CliAdapter::with_runner("run-v8-runner.sh", &["build"], "build/runtime", &runner)
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
        fs::create_dir_all(plugin_root.join("scripts")).unwrap();
        fs::write(plugin_root.join("scripts").join("run-bsl-analyzer.sh"), "").unwrap();
        fs::write(plugin_root.join("scripts").join("run-rlm-bsl-index.sh"), "").unwrap();
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
