//! Actual BSL analyzer MCP and JSONL diagnostics process implementation.
use crate::baseline::{broken_baseline_outcome, cli_baseline_failure};
use crate::environment::BslEnvironment;
use crate::jsonl::{
    AnalyzerDiagnosticsBatch, AnalyzerDiagnosticsFileTotals, DiagnosticsJsonlParser,
    MAX_DIAGNOSTICS_JSONL_LINE_BYTES,
};
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;
use unica_analysis_services::WorkspaceServiceManager;
use unica_artifact_delivery::engine::BundledToolError;
use unica_configuration::operational::OperationalConfig;
use unica_diagnostics::deadlines::{
    DIAGNOSTICS_ANALYZE_TIMEOUT_MAX_SECONDS, DIAGNOSTICS_ANALYZE_TIMEOUT_MIN_SECONDS,
};
use unica_execution_contract::adapter_outcome::AdapterOutcome;
use unica_process::arguments::process_timeout_error;
use unica_process::runner::{
    system_process_runner, ProcessCommand, ProcessRunner, ProcessStreamFailure,
};
use unica_process::{StreamControl, STREAM_LINE_TOO_LONG_ERROR};
use unica_redaction::redactor;
use unica_source_contract::workspace::WorkspaceContext;
use unica_work_control::cancellation::CancellationToken;
use unica_workspace::paths::normalize_path_identity;
const DEFAULT_PROCESS_TIMEOUT: Duration = Duration::from_secs(120);
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

struct SystemBslMcpRunner<'a> {
    environment: &'a dyn BslEnvironment,
}

pub struct BslAnalyzerMcpAdapter<'a> {
    environment: &'a dyn BslEnvironment,
    runner: Option<&'a dyn BslMcpRunner>,
    process_runner: &'a dyn ProcessRunner,
}

fn format_section(name: &str, text: &str) -> String {
    let body = text.trim_end();
    if body.is_empty() {
        format!("=== {name} ===")
    } else {
        format!("=== {name} ===\n{body}")
    }
}

/// Почему прогон анализатора не дал пакета диагностик.
///
/// Вид определяется местом, где прогон остановился, а не разбором текста
/// процесса: только так «движок не доставлен», «не запустился», «превысил
/// срок» и «завершился ошибкой» не схлопываются в одно «недоступен» (#1338).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalyzerRunFailureKind {
    /// Движка нет на машине: не доставлен, не собран, нет манифеста.
    EngineMissing,
    /// Движок на месте, но не прошёл проверку поставки: контрольная сумма,
    /// манифест, платформа.
    EngineInvalid,
    /// Процесс движка не удалось запустить.
    StartFailed,
    /// Процесс превысил заданный пользователем срок.
    TimedOut,
    /// Процесс запустился и завершился ошибкой до результата.
    Exited,
    Cancelled,
    /// Остальное: подготовка запроса, резидентный сеанс и прочее.
    Other,
}

/// Отказ прогона анализатора: закрытый вид и внутренний текст. Текст
/// остаётся внутри адаптера; наружу причина выходит только видом.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalyzerRunFailure {
    pub kind: AnalyzerRunFailureKind,
    pub message: String,
    /// Чего не хватает, когда движка нет: инструмент, цель, путь, версия
    /// и следующий шаг по режиму установки. Есть только у `EngineMissing`.
    pub missing: Option<Box<unica_artifact_delivery::engine::MissingEngine>>,
}

impl AnalyzerRunFailure {
    pub fn new(kind: AnalyzerRunFailureKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            missing: None,
        }
    }

    /// Движка нет на диске: вид и типизированное описание отсутствия.
    pub fn engine_missing(
        missing: Option<unica_artifact_delivery::engine::MissingEngine>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            missing: missing.map(Box::new),
            ..Self::new(AnalyzerRunFailureKind::EngineMissing, message)
        }
    }

    /// Ошибка разрешения поставляемого движка. Отсутствие определяется
    /// состоянием диска через типизированный [`MissingEngine`], а не текстом
    /// ошибки; поставка, которая есть, но не прошла проверку, — отдельный вид.
    ///
    /// [`MissingEngine`]: unica_artifact_delivery::engine::MissingEngine
    pub fn bundled_tool(
        environment: &dyn BslEnvironment,
        plugin_root: &Path,
        tool: &str,
        error: BundledToolError,
    ) -> Self {
        let error = error.to_string();
        match environment.missing_engine(plugin_root, tool) {
            Some(missing) => Self::engine_missing(Some(missing), error),
            None => Self::new(AnalyzerRunFailureKind::EngineInvalid, error),
        }
    }
}

impl From<String> for AnalyzerRunFailure {
    fn from(message: String) -> Self {
        let kind = if message.starts_with(unica_work_control::cancellation::CANCELLED_PREFIX) {
            AnalyzerRunFailureKind::Cancelled
        } else {
            AnalyzerRunFailureKind::Other
        };
        Self::new(kind, message)
    }
}

impl<'a> BslAnalyzerMcpAdapter<'a> {
    pub fn new(environment: &'a dyn BslEnvironment) -> Self {
        Self {
            runner: None,
            environment,
            process_runner: system_process_runner(),
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_runner(runner: &'a dyn BslMcpRunner, environment: &'a dyn BslEnvironment) -> Self {
        Self {
            runner: Some(runner),
            environment,
            process_runner: system_process_runner(),
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_process_runner(
        process_runner: &'a dyn ProcessRunner,
        environment: &'a dyn BslEnvironment,
    ) -> Self {
        Self {
            runner: None,
            environment,
            process_runner,
        }
    }

    pub fn analyze_diagnostic_batch(
        &self,
        context: &WorkspaceContext,
        source_root: &Path,
        module: Option<&Path>,
        timeout: Option<Duration>,
        cancellation: &CancellationToken,
    ) -> Result<AnalyzerDiagnosticsBatch, AnalyzerRunFailure> {
        let mut args = Map::new();
        args.insert(
            "sourceDir".to_string(),
            Value::String(source_root.display().to_string()),
        );
        // Без заданного срока процесс анализатора идёт до конца (#1251).
        if let Some(timeout_seconds) = timeout.map(analyzer_analyze_timeout_seconds) {
            args.insert("timeoutSeconds".to_string(), json!(timeout_seconds));
        }
        let result = self.invoke_diagnostics_analyze_scoped(
            "unica.code.diagnostics",
            &args,
            context,
            false,
            None,
            cancellation,
            module,
        )?;
        result.diagnostics.ok_or_else(|| {
            AnalyzerRunFailure::new(
                result.run_failure.unwrap_or(AnalyzerRunFailureKind::Other),
                result
                    .outcome
                    .errors
                    .first()
                    .cloned()
                    .unwrap_or(result.outcome.summary),
            )
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

    pub fn invoke_cancellable_with_operational_config(
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
        let plugin_root = match self.environment.find_plugin_root(&context.cwd) {
            Some(plugin_root) => plugin_root,
            None => {
                return Ok(provider_unavailable_outcome(
                    tool_name,
                    "could not locate Unica plugin root for bsl-analyzer MCP adapter lookup"
                        .to_string(),
                ))
            }
        };
        let source_dir = self
            .environment
            .resolve_source_root(context, args.get("sourceDir").and_then(Value::as_str))?
            .path;
        let (remote_tool, tool_args) = bsl_mcp_tool_request(tool_name, args)?;
        // A workspace that has not downloaded the tools has no analyzer in its
        // manifest. `code.search` already answers that state with an
        // unavailable section and a working result; answering it
        // here with a failed call made the same workstation look broken (#275).
        // A provider that ran and failed is a different case and still fails.
        let bundled_tool =
            match self
                .environment
                .resolve_bundled_tool(&plugin_root, "bsl-analyzer", !dry_run)
            {
                Ok(bundled_tool) => bundled_tool,
                Err(error @ BundledToolError::Missing(_)) => {
                    return Ok(provider_unavailable_outcome(tool_name, error.into()))
                }
                Err(error) => return Err(error.into()),
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
            let mut outcome = self.environment.dry_run_outcome(
                tool_name,
                "typed bsl-analyzer MCP",
                false,
                bundled_tool,
                reported_command,
            );
            outcome.artifacts = vec![source_dir.display().to_string()];
            return Ok(BslAnalyzerOutcome::plain(outcome));
        }

        let system = SystemBslMcpRunner {
            environment: self.environment,
        };
        let output = self.runner.unwrap_or(&system).call(&command)?;
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
            run_failure: None,
        })
    }

    #[cfg(any(test, feature = "test-support"))]
    #[allow(clippy::too_many_arguments)]
    pub fn test_invoke_diagnostics_analyze_scoped(
        &self,
        tool_name: &str,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
        dry_run: bool,
        operational_config: Option<&OperationalConfig>,
        cancellation: &CancellationToken,
        module: Option<&Path>,
    ) -> Result<BslAnalyzerOutcome, AnalyzerRunFailure> {
        self.invoke_diagnostics_analyze_scoped(
            tool_name,
            args,
            context,
            dry_run,
            operational_config,
            cancellation,
            module,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn invoke_diagnostics_analyze_scoped(
        &self,
        tool_name: &str,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
        dry_run: bool,
        operational_config: Option<&OperationalConfig>,
        cancellation: &CancellationToken,
        module: Option<&Path>,
    ) -> Result<BslAnalyzerOutcome, AnalyzerRunFailure> {
        let plugin_root = self
            .environment
            .find_plugin_root(&context.cwd)
            .ok_or_else(|| {
                AnalyzerRunFailure::engine_missing(
                    None,
                    "could not locate Unica plugin root for diagnostics adapter lookup",
                )
            })?;
        let source_dir = self
            .environment
            .resolve_source_root(context, args.get("sourceDir").and_then(Value::as_str))?
            .path;
        let mut normalized_args = diagnostics_analyze_args(args);
        // Keep the full configuration root for metadata and cross-module facts.
        // A JSON file preserves commas and spaces in physical module paths.
        let _scope_file = if let Some(module) = module {
            if !module.is_absolute() {
                return Err("diagnostics module path must be absolute"
                    .to_string()
                    .into());
            }
            let module = module
                .to_str()
                .ok_or_else(|| "diagnostics module path must be UTF-8".to_string())?;
            let scratch_root = normalize_path_identity(&std::env::temp_dir())?;
            if scratch_root.starts_with(normalize_path_identity(&source_dir)?) {
                return Err("diagnostics scope file would be inside source root"
                    .to_string()
                    .into());
            }
            let mut scope = tempfile::NamedTempFile::new_in(scratch_root)
                .map_err(|_| "cannot create diagnostics scope file".to_string())?;
            serde_json::to_writer(
                scope.as_file_mut(),
                &json!({
                    "base_ref": "unica-check",
                    "head_ref": "current",
                    "files": {module: {"hunks": null}},
                }),
            )
            .map_err(|_| "cannot write diagnostics scope file".to_string())?;
            let filter_path = scope
                .path()
                .to_str()
                .ok_or_else(|| "diagnostics scope path must be UTF-8".to_string())?;
            normalized_args.insert(
                "diffFilter".to_string(),
                Value::String(filter_path.to_string()),
            );
            Some(scope)
        } else {
            None
        };
        let process_timeout = diagnostics_analyze_timeout(args, operational_config)?;
        let bundled_tool = self
            .environment
            .resolve_bundled_tool(&plugin_root, "bsl-analyzer", !dry_run)
            .map_err(|error| {
                AnalyzerRunFailure::bundled_tool(
                    self.environment,
                    &plugin_root,
                    "bsl-analyzer",
                    error,
                )
            })?;
        let reported_args = self.environment.cli_args(&normalized_args, true)?;
        let execution_args = self.environment.cli_args(&normalized_args, false)?;
        let mut reported_command = vec![bundled_tool.program.display().to_string()];
        reported_command.push("analyze".to_string());
        reported_command.extend(reported_args);

        if dry_run {
            let mut outcome = self.environment.dry_run_outcome(
                tool_name,
                "bsl-analyzer code analysis",
                false,
                bundled_tool,
                reported_command,
            );
            outcome.artifacts = vec![source_dir.display().to_string()];
            return Ok(BslAnalyzerOutcome::plain(outcome));
        }

        let mut process_args = vec!["analyze".to_string()];
        process_args.extend(execution_args);
        let mut parser = match module {
            Some(module) => DiagnosticsJsonlParser::for_module(&source_dir, module)?,
            None => DiagnosticsJsonlParser::new(&source_dir)?,
        };
        let mut consume = |line_number, bytes: &[u8]| {
            parser.push_line(line_number, bytes);
            StreamControl::Continue
        };
        let explicit_config = normalized_args
            .get("config")
            .and_then(Value::as_str)
            .map(|path| {
                let path = PathBuf::from(path);
                if path.is_absolute() {
                    path
                } else {
                    context.cwd.join(path)
                }
            });
        let mut filter_errors = crate::filters::cli_filter_errors(
            &source_dir,
            explicit_config.as_deref(),
            module.is_some(),
        );
        let started = std::time::Instant::now();
        let output = self
            .process_runner
            .run_streaming_staged(
                &ProcessCommand {
                    program: bundled_tool.program,
                    args: process_args,
                    cwd: context.cwd.clone(),
                    env: Vec::new(),
                    env_remove: Vec::new(),
                    capture_limits: None,
                    timeout: process_timeout,
                    cancellation: cancellation.clone(),
                },
                MAX_DIAGNOSTICS_JSONL_LINE_BYTES,
                &mut consume,
            )
            .map_err(|failure| match failure {
                // The process never existed: the engine could not be started.
                ProcessStreamFailure::Spawn(error) => {
                    AnalyzerRunFailure::new(AnalyzerRunFailureKind::StartFailed, error)
                }
                // Waiting for or stopping a started process failed. Past the
                // user's deadline that is a timeout whose stop failed;
                // otherwise the run failed for a reason Unica cannot name.
                ProcessStreamFailure::Run(error) => AnalyzerRunFailure::new(
                    if process_timeout.is_some_and(|limit| started.elapsed() >= limit) {
                        AnalyzerRunFailureKind::TimedOut
                    } else {
                        AnalyzerRunFailureKind::Other
                    },
                    error,
                ),
            })?;
        if let Some((line_number, reason)) = &output.line_error {
            use unica_diagnostics::stream_error::StreamErrorKind;
            let kind = if reason == STREAM_LINE_TOO_LONG_ERROR {
                StreamErrorKind::LineTooLong
            } else {
                StreamErrorKind::ReadFailure
            };
            parser.reject_line(*line_number, kind);
        }
        let stderr = redactor(&output.stderr);
        if output.cancelled {
            let mut outcome = AdapterOutcome::cancelled(format!("{tool_name} process stopped"));
            outcome.stderr = (!stderr.trim().is_empty()).then_some(stderr);
            outcome.command = Some(reported_command);
            let mut outcome = BslAnalyzerOutcome::plain(outcome);
            outcome.run_failure = Some(AnalyzerRunFailureKind::Cancelled);
            return Ok(outcome);
        }
        if !output.status_success {
            // A broken `[diagnostics.baseline]` stops the analyzer before
            // `start`, so only its stderr names the cause. That run is a
            // named incompleteness, not an unavailable provider.
            let baseline_failure = (!output.timed_out)
                .then(|| cli_baseline_failure(&output.stderr))
                .flatten();
            let timeout_error = output
                .timed_out
                .then(|| process_timeout_error("code analysis", process_timeout));
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
            let outcome = AdapterOutcome {
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
            };
            let kind = if output.timed_out {
                AnalyzerRunFailureKind::TimedOut
            } else {
                AnalyzerRunFailureKind::Exited
            };
            return Ok(BslAnalyzerOutcome {
                outcome,
                data: None,
                diagnostics: baseline_failure.map(|reason| AnalyzerDiagnosticsBatch {
                    outcome: broken_baseline_outcome(reason),
                    files: AnalyzerDiagnosticsFileTotals {
                        discovered: None,
                        processed: None,
                        failed: None,
                    },
                    diagnostics_reported: None,
                    elapsed_seconds: None,
                }),
                run_failure: Some(kind),
            });
        }

        let mut batch = parser.finish();
        // A removed filter must not hide that it may have applied while the
        // process ran; conversely a newly observed filter is also unproven.
        for error in crate::filters::cli_filter_errors(
            &source_dir,
            explicit_config.as_deref(),
            module.is_some(),
        ) {
            if !filter_errors.iter().any(|before| before.code == error.code) {
                filter_errors.push(error);
            }
        }
        crate::filters::mark_cli_filter_limits(&mut batch.outcome, module, filter_errors);
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
            run_failure: None,
        })
    }
}

/// An analyzer answer plus the typed payload, for the tools whose contract is
/// already the analyzer's own JSON.
#[derive(Debug)]
pub struct BslAnalyzerOutcome {
    pub outcome: AdapterOutcome,
    pub data: Option<Value>,
    pub diagnostics: Option<AnalyzerDiagnosticsBatch>,
    /// Where the analyze run stopped when it produced no diagnostics batch.
    pub run_failure: Option<AnalyzerRunFailureKind>,
}

impl BslAnalyzerOutcome {
    fn plain(outcome: AdapterOutcome) -> Self {
        Self {
            outcome,
            data: None,
            diagnostics: None,
            run_failure: None,
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

/// Caller budget the analyzer run is allowed to take, in whole seconds.
///
/// The remaining budget is always a fraction below the configured one because
/// the deadline starts before the provider worker does, so truncating would
/// push the documented minimum of 30 seconds out of the accepted band and drop
/// the caller budget entirely.
fn analyzer_analyze_timeout_seconds(timeout: Duration) -> u64 {
    let seconds = timeout.as_secs() + u64::from(timeout.subsec_nanos() > 0);
    seconds.clamp(
        DIAGNOSTICS_ANALYZE_TIMEOUT_MIN_SECONDS,
        DIAGNOSTICS_ANALYZE_TIMEOUT_MAX_SECONDS,
    )
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

/// `None` — срок анализа никто не задал, и процесс идёт до конечного исхода.
fn diagnostics_analyze_timeout(
    args: &Map<String, Value>,
    operational_config: Option<&OperationalConfig>,
) -> Result<Option<Duration>, String> {
    let Some(value) = args.get("timeoutSeconds") else {
        return Ok(
            operational_config.and_then(|config| config.code_diagnostics().analyze_timeout())
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
    Ok(Some(Duration::from_secs(seconds)))
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
            let mode = unica_process::arguments::required_string(args, "mode")?;
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

impl BslMcpRunner for SystemBslMcpRunner<'_> {
    fn call(&self, command: &BslMcpCommand) -> Result<BslMcpOutput, String> {
        let context = self
            .environment
            .discover_workspace(Some(command.cwd.clone()))?;
        let output = WorkspaceServiceManager::new(self.environment.service_environment())
            .call_bsl_mcp_cancellable(
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn analyzer_analyze_budget_survives_the_deadline_that_already_started() {
        // The provider deadline starts before the worker thread does, so the
        // smallest configured budget always arrives here a fraction short.
        let almost_minimum =
            Duration::from_secs(DIAGNOSTICS_ANALYZE_TIMEOUT_MIN_SECONDS) - Duration::from_millis(1);
        assert_eq!(
            analyzer_analyze_timeout_seconds(almost_minimum),
            DIAGNOSTICS_ANALYZE_TIMEOUT_MIN_SECONDS,
            "the caller budget must reach the analyzer instead of being dropped"
        );

        let almost_maximum =
            Duration::from_secs(DIAGNOSTICS_ANALYZE_TIMEOUT_MAX_SECONDS) - Duration::from_millis(1);
        assert_eq!(
            analyzer_analyze_timeout_seconds(almost_maximum),
            DIAGNOSTICS_ANALYZE_TIMEOUT_MAX_SECONDS
        );

        // An exhausted deadline still bounds the analyzer by the smallest
        // accepted value rather than handing it its own default.
        assert_eq!(
            analyzer_analyze_timeout_seconds(Duration::from_secs(1)),
            DIAGNOSTICS_ANALYZE_TIMEOUT_MIN_SECONDS
        );
        assert_eq!(
            analyzer_analyze_timeout_seconds(Duration::from_secs(7_200)),
            DIAGNOSTICS_ANALYZE_TIMEOUT_MAX_SECONDS
        );
    }
}
