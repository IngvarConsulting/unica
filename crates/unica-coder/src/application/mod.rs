use crate::domain::cache::{CacheAccess, CacheReport};
use crate::domain::cancellation::CancellationToken;
use crate::domain::events::{runtime_event_kind, DomainEvent, DomainEventKind};
use crate::domain::workspace::WorkspaceContext;
pub(crate) use operation_descriptors::SupportGuardRequirement;
pub(crate) use outcome::AdapterOutcome;
use ports::{ApplicationPorts, FormatGuardCheck, SupportGuardCheck};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::path::PathBuf;
use std::sync::Arc;
pub(crate) use tool_contracts::{
    DIAGNOSTICS_ANALYZE_TIMEOUT_MAX_SECONDS, DIAGNOSTICS_ANALYZE_TIMEOUT_MIN_SECONDS,
};

pub(crate) mod operation_descriptors;
mod outcome;
pub(crate) mod ports;
pub(crate) mod tool_contracts;
pub use tool_contracts::input_schema_for_tool;

#[derive(Debug, Clone, Copy)]
pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub mutating: bool,
    pub cache_access: CacheAccess,
    pub handler: ToolHandler,
}

#[derive(Debug, Clone, Copy)]
pub enum ToolHandler {
    NativeOperation {
        operation: &'static str,
        event: Option<DomainEventKind>,
    },
    ProjectStatus,
    ProjectMap,
    BuildRuntime {
        command: &'static [&'static str],
        event: Option<DomainEventKind>,
    },
    RuntimeAdapter,
    RuntimeJob {
        action: RuntimeJobAction,
    },
    CodeAdapter {
        command: &'static [&'static str],
    },
    StandardsAdapter {
        operation: &'static str,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeJobAction {
    Start,
    Status,
    Wait,
    Logs,
    Cancel,
    List,
}

#[derive(Debug, Serialize)]
pub struct OperationResult {
    pub ok: bool,
    pub summary: String,
    pub changes: Vec<String>,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
    pub artifacts: Vec<String>,
    pub cache: CacheReport,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stdout: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stderr: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job: Option<Value>,
}

pub struct UnicaApplication {
    ports: Arc<dyn ApplicationPorts + Send + Sync>,
}

impl UnicaApplication {
    pub(crate) fn with_ports(ports: Arc<dyn ApplicationPorts + Send + Sync>) -> Self {
        Self { ports }
    }

    pub fn tools(&self) -> Vec<ToolSpec> {
        tools()
    }

    pub fn call_tool(
        &self,
        name: &str,
        args: &Map<String, Value>,
    ) -> Result<OperationResult, String> {
        self.call_tool_cancellable(name, args, CancellationToken::new())
    }

    pub fn call_tool_cancellable(
        &self,
        name: &str,
        args: &Map<String, Value>,
        cancellation: CancellationToken,
    ) -> Result<OperationResult, String> {
        let spec = tools()
            .into_iter()
            .find(|tool| tool.name == name)
            .ok_or_else(|| format!("unknown unica tool: {name}"))?;
        call_tool(spec, args, self.ports.as_ref(), &cancellation)
    }
}

pub fn tools() -> Vec<ToolSpec> {
    let mut specs = configuration_tools();
    specs.extend([
        ToolSpec {
            name: "unica.project.status",
            description: "Inspect current Unica workspace, source set, and cache state.",
            mutating: false,
            cache_access: CacheAccess::default(),
            handler: ToolHandler::ProjectStatus,
        },
        ToolSpec {
            name: "unica.project.map",
            description:
                "Inspect configured source sets and effective source format per source set.",
            mutating: false,
            cache_access: CacheAccess {
                reads: &["workspace_graph"],
                writes: &[],
            },
            handler: ToolHandler::ProjectMap,
        },
        ToolSpec {
            name: "unica.build.dump",
            description: "Dump source set through the internal build/runtime adapter.",
            mutating: true,
            cache_access: CacheAccess {
                reads: &[],
                writes: &["workspace_graph", "metadata_graph"],
            },
            handler: ToolHandler::BuildRuntime {
                command: &["dump"],
                event: Some(DomainEventKind::SourceSetChanged),
            },
        },
        ToolSpec {
            name: "unica.build.load",
            description: "Load/build XML source set through the internal build/runtime adapter.",
            mutating: true,
            cache_access: CacheAccess {
                reads: &[],
                writes: &["workspace_graph", "metadata_graph"],
            },
            handler: ToolHandler::BuildRuntime {
                command: &["build"],
                event: Some(DomainEventKind::BuildCompleted),
            },
        },
        ToolSpec {
            name: "unica.build.update",
            description:
                "Apply built configuration changes through the internal build/runtime adapter.",
            mutating: true,
            cache_access: CacheAccess {
                reads: &[],
                writes: &["workspace_graph", "metadata_graph"],
            },
            handler: ToolHandler::BuildRuntime {
                command: &["build", "--update"],
                event: Some(DomainEventKind::BuildCompleted),
            },
        },
        ToolSpec {
            name: "unica.build.make",
            description: "Create CF/CFE artifact through the internal build/runtime adapter.",
            mutating: true,
            cache_access: CacheAccess::default(),
            handler: ToolHandler::BuildRuntime {
                command: &["make"],
                event: None,
            },
        },
        ToolSpec {
            name: "unica.build.run",
            description:
                "Launch 1C runtime or Designer through the internal build/runtime adapter.",
            mutating: true,
            cache_access: CacheAccess::default(),
            handler: ToolHandler::BuildRuntime {
                command: &["launch"],
                event: None,
            },
        },
        ToolSpec {
            name: "unica.runtime.execute",
            description:
                "Execute typed v8-runner runtime workflows through the single Unica MCP boundary.",
            mutating: true,
            cache_access: CacheAccess {
                reads: &[],
                writes: &["workspace_graph", "metadata_graph"],
            },
            handler: ToolHandler::RuntimeAdapter,
        },
        ToolSpec {
            name: "unica.runtime.job.start",
            description:
                "Start a durable typed v8-runner runtime job without changing runtime.execute.",
            mutating: true,
            cache_access: CacheAccess::default(),
            handler: ToolHandler::RuntimeJob {
                action: RuntimeJobAction::Start,
            },
        },
        ToolSpec {
            name: "unica.runtime.job.status",
            description: "Read a durable runtime job snapshot by jobId.",
            mutating: false,
            cache_access: CacheAccess::default(),
            handler: ToolHandler::RuntimeJob {
                action: RuntimeJobAction::Status,
            },
        },
        ToolSpec {
            name: "unica.runtime.job.wait",
            description: "Wait for a durable runtime job with a caller-side bounded timeout.",
            mutating: false,
            cache_access: CacheAccess::default(),
            handler: ToolHandler::RuntimeJob {
                action: RuntimeJobAction::Wait,
            },
        },
        ToolSpec {
            name: "unica.runtime.job.logs",
            description: "Read bounded redacted stdout and stderr tails for a durable runtime job.",
            mutating: false,
            cache_access: CacheAccess::default(),
            handler: ToolHandler::RuntimeJob {
                action: RuntimeJobAction::Logs,
            },
        },
        ToolSpec {
            name: "unica.runtime.job.cancel",
            description: "Request safe cancellation for a durable runtime job.",
            mutating: true,
            cache_access: CacheAccess::default(),
            handler: ToolHandler::RuntimeJob {
                action: RuntimeJobAction::Cancel,
            },
        },
        ToolSpec {
            name: "unica.runtime.job.list",
            description: "List durable runtime job snapshots in the current workspace.",
            mutating: false,
            cache_access: CacheAccess::default(),
            handler: ToolHandler::RuntimeJob {
                action: RuntimeJobAction::List,
            },
        },
        ToolSpec {
            name: "unica.code.search",
            description: "Search BSL code through the internal RLM index.",
            mutating: false,
            cache_access: CacheAccess {
                reads: &["bsl_index"],
                writes: &[],
            },
            handler: ToolHandler::CodeAdapter {
                command: &["search"],
            },
        },
        ToolSpec {
            name: "unica.code.definition",
            description: "Find BSL method definitions through the typed Unica code index boundary.",
            mutating: false,
            cache_access: CacheAccess {
                reads: &["bsl_index"],
                writes: &[],
            },
            handler: ToolHandler::CodeAdapter {
                command: &["definition"],
            },
        },
        ToolSpec {
            name: "unica.code.outline",
            description: "Read compact BSL module outline from the internal code index.",
            mutating: false,
            cache_access: CacheAccess {
                reads: &["bsl_index"],
                writes: &[],
            },
            handler: ToolHandler::CodeAdapter {
                command: &["outline"],
            },
        },
        ToolSpec {
            name: "unica.code.grep",
            description: "Run safe typed git-grep search inside the Unica workspace.",
            mutating: false,
            cache_access: CacheAccess::default(),
            handler: ToolHandler::CodeAdapter { command: &["grep"] },
        },
        ToolSpec {
            name: "unica.code.patch",
            description: "Insert content into one selected existing BSL *Module.bsl file.",
            mutating: true,
            cache_access: cache_access_for("code-patch", Some(DomainEventKind::ModuleChanged)),
            handler: ToolHandler::NativeOperation {
                operation: "code-patch",
                event: Some(DomainEventKind::ModuleChanged),
            },
        },
        ToolSpec {
            name: "unica.code.graph",
            description: "Inspect BSL call graph through the typed Unica code analysis boundary.",
            mutating: false,
            cache_access: CacheAccess {
                reads: &["workspace_graph", "bsl_diagnostics"],
                writes: &[],
            },
            handler: ToolHandler::CodeAdapter {
                command: &["graph"],
            },
        },
        ToolSpec {
            name: "unica.code.diagnostics",
            description: "Run BSL diagnostics through the internal code analysis adapter.",
            mutating: false,
            cache_access: CacheAccess {
                reads: &["bsl_diagnostics"],
                writes: &[],
            },
            handler: ToolHandler::CodeAdapter {
                command: &["analyze"],
            },
        },
        ToolSpec {
            name: "unica.standards.search",
            description: "Search 1C standards through the internal standards adapter.",
            mutating: false,
            cache_access: CacheAccess::default(),
            handler: ToolHandler::StandardsAdapter {
                operation: "search",
            },
        },
        ToolSpec {
            name: "unica.standards.explain",
            description:
                "Explain 1C diagnostics or standards through the internal standards adapter.",
            mutating: false,
            cache_access: CacheAccess::default(),
            handler: ToolHandler::StandardsAdapter {
                operation: "explain",
            },
        },
    ]);
    specs
}

fn call_tool(
    spec: ToolSpec,
    args: &Map<String, Value>,
    ports: &dyn ApplicationPorts,
    cancellation: &CancellationToken,
) -> Result<OperationResult, String> {
    let normalized_args = tool_contracts::normalize_native_path_aliases(spec, args)?;
    let args = &normalized_args;
    let dry_run = args
        .get("dryRun")
        .and_then(Value::as_bool)
        .unwrap_or(spec.mutating);
    tool_contracts::validate_tool_arguments(spec, args, dry_run)?;
    let cwd = args.get("cwd").and_then(Value::as_str).map(PathBuf::from);
    let context = ports.discover_workspace(cwd)?;
    ports.validate_tool_context(spec, args, dry_run, &context)?;
    let mut format_guard_warning = None;
    let mut format_diagnostic = None;
    match ports.evaluate_format_guard(spec, args, &context)? {
        FormatGuardCheck::Allow => {}
        FormatGuardCheck::Warn {
            warning,
            diagnostic,
        } => {
            format_guard_warning = Some(warning);
            format_diagnostic = Some(diagnostic);
        }
        FormatGuardCheck::Block {
            outcome,
            diagnostic,
        } => {
            let cache = ports.cache_report(&context, &[], dry_run, spec.cache_access)?;
            return Ok(OperationResult {
                ok: outcome.ok,
                summary: outcome.summary,
                changes: outcome.changes,
                warnings: outcome.warnings,
                errors: outcome.errors,
                artifacts: outcome.artifacts,
                cache,
                stdout: outcome.stdout,
                stderr: outcome.stderr,
                command: outcome.command,
                diagnostics: Some(json!({"formatCompatibility": diagnostic})),
                data: None,
                job: None,
            });
        }
    }
    if let Some(outcome) = runtime_xml_route_guard(spec, args, dry_run, cancellation)
        .or_else(|| source_sync_dump_guard(spec, args, dry_run, cancellation))
    {
        let cache = ports.cache_report(&context, &[], dry_run, spec.cache_access)?;
        return Ok(OperationResult {
            ok: outcome.ok,
            summary: outcome.summary,
            changes: outcome.changes,
            warnings: outcome.warnings,
            errors: outcome.errors,
            artifacts: outcome.artifacts,
            cache,
            stdout: outcome.stdout,
            stderr: outcome.stderr,
            command: outcome.command,
            diagnostics: None,
            data: None,
            job: None,
        });
    }
    let mut support_guard_warning = if spec.mutating && !dry_run {
        match ports.evaluate_support_guard(spec, args, &context)? {
            SupportGuardCheck::Allow => None,
            SupportGuardCheck::Warn(warning) => Some(warning),
            SupportGuardCheck::Block(outcome) => {
                let cache = ports.cache_report(&context, &[], dry_run, spec.cache_access)?;
                return Ok(OperationResult {
                    ok: outcome.ok,
                    summary: outcome.summary,
                    changes: outcome.changes,
                    warnings: outcome.warnings,
                    errors: outcome.errors,
                    artifacts: outcome.artifacts,
                    cache,
                    stdout: outcome.stdout,
                    stderr: outcome.stderr,
                    command: outcome.command,
                    diagnostics: None,
                    data: None,
                    job: None,
                });
            }
        }
    } else {
        None
    };

    let handler_outcome = ports.invoke_handler(spec, args, &context, dry_run, cancellation)?;
    let mut outcome = handler_outcome.adapter;
    if is_successful_detailed_compile_preview(spec, dry_run, &outcome) {
        match ports.evaluate_support_guard(spec, args, &context)? {
            SupportGuardCheck::Allow => {}
            SupportGuardCheck::Warn(warning) => support_guard_warning = Some(warning),
            SupportGuardCheck::Block(blocked) => {
                let cache = ports.cache_report(&context, &[], dry_run, spec.cache_access)?;
                return Ok(OperationResult {
                    ok: blocked.ok,
                    summary: blocked.summary,
                    changes: blocked.changes,
                    warnings: blocked.warnings,
                    errors: blocked.errors,
                    artifacts: blocked.artifacts,
                    cache,
                    stdout: blocked.stdout,
                    stderr: blocked.stderr,
                    command: blocked.command,
                    diagnostics: None,
                    data: None,
                    job: None,
                });
            }
        }
    }
    if let Some(warning) = support_guard_warning {
        outcome.warnings.insert(0, warning);
    }
    if let Some(warning) = format_guard_warning {
        outcome.warnings.insert(0, warning);
    }
    let events = if should_emit_events(spec, args, dry_run, &outcome) {
        domain_events(spec, args)
    } else {
        Vec::new()
    };
    let cache = ports.cache_report(&context, &events, dry_run, spec.cache_access)?;
    if spec.mutating && !dry_run && outcome.ok && !events.is_empty() {
        ports.notify_invalidation(&context, &events);
    }
    let diagnostics = merge_diagnostics(
        runtime_result_diagnostics(
            spec,
            args,
            &context,
            &outcome,
            handler_outcome.data.as_ref(),
        ),
        format_diagnostic,
    );

    Ok(OperationResult {
        ok: outcome.ok,
        summary: outcome.summary,
        changes: outcome.changes,
        warnings: outcome.warnings,
        errors: outcome.errors,
        artifacts: outcome.artifacts,
        cache,
        stdout: outcome.stdout,
        stderr: outcome.stderr,
        command: outcome.command,
        diagnostics,
        data: handler_outcome.data,
        job: handler_outcome.job,
    })
}

fn runtime_xml_route_guard(
    spec: ToolSpec,
    args: &Map<String, Value>,
    dry_run: bool,
    cancellation: &CancellationToken,
) -> Option<AdapterOutcome> {
    if dry_run
        || !matches!(
            spec.handler,
            ToolHandler::RuntimeAdapter
                | ToolHandler::RuntimeJob {
                    action: RuntimeJobAction::Start
                }
        )
    {
        return None;
    }
    if cancellation.is_cancelled() {
        return Some(AdapterOutcome::cancelled(format!(
            "{} stopped before runtime XML route execution",
            spec.name
        )));
    }

    let operation = args.get("operation").and_then(Value::as_str);
    let message = if operation == Some("convert") {
        Some(
            "applied runtime convert is disabled because EDT-to-Designer conversion can publish platform XML without the verified platform 8.3.27 / exact export format 2.20 private-stage validation used by synchronous full dump; dryRun=true remains available"
                .to_string(),
        )
    } else if operation == Some("launch") && contains_reserved_designer_file_key(args) {
        Some(
            "Designer rawKeys containing DumpConfigToFiles or LoadConfigFromFiles are reserved and cannot bypass Unica's platform 8.3.27 / export format 2.20 source guards; use typed dump/build operations"
                .to_string(),
        )
    } else {
        None
    }?;

    Some(AdapterOutcome {
        ok: false,
        summary: format!("{} blocked by runtime XML route guard", spec.name),
        changes: Vec::new(),
        warnings: vec![
            "Git-visible platform XML was not created or consumed through an unverified route"
                .to_string(),
        ],
        errors: vec![message.clone()],
        artifacts: Vec::new(),
        stdout: None,
        stderr: Some(format!("{message}\n")),
        command: None,
    })
}

fn contains_reserved_designer_file_key(args: &Map<String, Value>) -> bool {
    args.get("rawKeys")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_ascii_lowercase)
        .any(|key| key.contains("dumpconfigtofiles") || key.contains("loadconfigfromfiles"))
}

fn merge_diagnostics(runtime: Option<Value>, format: Option<Value>) -> Option<Value> {
    match (runtime, format) {
        (None, None) => None,
        (Some(runtime), None) => Some(runtime),
        (None, Some(format)) => Some(json!({"formatCompatibility": format})),
        (Some(mut runtime), Some(format)) => {
            if let Some(object) = runtime.as_object_mut() {
                object.insert("formatCompatibility".to_string(), format);
                Some(runtime)
            } else {
                Some(json!({
                    "runtime": runtime,
                    "formatCompatibility": format,
                }))
            }
        }
    }
}

fn source_sync_dump_guard(
    spec: ToolSpec,
    args: &Map<String, Value>,
    dry_run: bool,
    cancellation: &CancellationToken,
) -> Option<AdapterOutcome> {
    if dry_run || !is_source_dump(spec, args) {
        return None;
    }
    if cancellation.is_cancelled() {
        return Some(AdapterOutcome::cancelled(format!(
            "{} dump stopped before execution",
            spec.name
        )));
    }
    let mode = args.get("mode").and_then(Value::as_str);
    if mode == Some("full") {
        if matches!(
            spec.handler,
            ToolHandler::RuntimeJob {
                action: RuntimeJobAction::Start
            }
        ) {
            let message = "asynchronous applied full dump is not supported yet because the background job boundary cannot return the private staged tree to Unica for the required platform 8.3.27 and exact export format 2.20 validation before publication; use synchronous unica.runtime.execute or unica.build.dump".to_string();
            return Some(AdapterOutcome {
                ok: false,
                summary: format!("{} blocked by source sync guard", spec.name),
                changes: Vec::new(),
                warnings: vec![
                    "dryRun=true remains available to inspect the planned v8-runner command"
                        .to_string(),
                ],
                errors: vec![message.clone()],
                artifacts: Vec::new(),
                stdout: None,
                stderr: Some(format!("{message}\n")),
                command: None,
            });
        }
        return None;
    }

    let requested_mode = mode
        .map(|mode| format!("mode={mode}"))
        .unwrap_or_else(|| "no explicit mode".to_string());
    let message = format!(
        "applied dump with {requested_mode} is disabled because only explicit mode=full declares whole-tree replacement and uses staging publication; pinned v8-runner cannot report exact processed paths/hashes or perform a divergence-safe merge; DESIGNER incremental/partial dumps also write directly into the source root, while EDT stages final publication but still lacks that merge receipt; use mode=full or wait for the shadow/staging receipt contract in alkoleft/v8-runner-rust#30"
    );
    Some(AdapterOutcome {
        ok: false,
        summary: format!("{} blocked by source sync guard", spec.name),
        changes: Vec::new(),
        warnings: vec![
            "dryRun=true remains available to inspect the planned v8-runner command".to_string(),
        ],
        errors: vec![message.clone()],
        artifacts: Vec::new(),
        stdout: None,
        stderr: Some(format!("{message}\n")),
        command: None,
    })
}

fn is_source_dump(spec: ToolSpec, args: &Map<String, Value>) -> bool {
    match spec.handler {
        ToolHandler::BuildRuntime { command, .. } => command == ["dump"],
        ToolHandler::RuntimeAdapter
        | ToolHandler::RuntimeJob {
            action: RuntimeJobAction::Start,
        } => args.get("operation").and_then(Value::as_str) == Some("dump"),
        _ => false,
    }
}

fn should_emit_events(
    spec: ToolSpec,
    args: &Map<String, Value>,
    dry_run: bool,
    outcome: &AdapterOutcome,
) -> bool {
    if !spec.mutating || !outcome.ok {
        return false;
    }
    if dry_run && spec.name == "unica.code.patch" {
        return false;
    }
    if !dry_run {
        return !outcome.changes.is_empty();
    }

    let is_semantic_form_edit_preview = spec.name == "unica.form.edit"
        && args.keys().any(|key| {
            matches!(
                key.as_str(),
                "FormPath" | "formPath" | "Path" | "path" | "JsonPath" | "jsonPath" | "definition"
            )
        });
    !is_semantic_form_edit_preview || !outcome.changes.is_empty()
}

fn is_successful_detailed_compile_preview(
    spec: ToolSpec,
    dry_run: bool,
    outcome: &AdapterOutcome,
) -> bool {
    dry_run
        && outcome.ok
        && outcome.summary.contains("planned native")
        && matches!(
            spec.handler,
            ToolHandler::NativeOperation {
                operation: "meta-compile" | "role-compile" | "subsystem-compile",
                ..
            }
        )
}

fn runtime_result_diagnostics(
    spec: ToolSpec,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    outcome: &AdapterOutcome,
    data: Option<&Value>,
) -> Option<Value> {
    if !matches!(spec.handler, ToolHandler::RuntimeAdapter) {
        return None;
    }
    let operation = args
        .get("operation")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    if let Some(wait) = data
        .and_then(|data| data.get("external_epf_wait"))
        .and_then(Value::as_object)
    {
        let timed_out = wait
            .get("timed_out")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let exit_code = wait.get("exit_code").cloned().unwrap_or(Value::Null);
        let outcome_kind = if timed_out {
            "timeout"
        } else if exit_code.as_i64() == Some(0) {
            "success"
        } else {
            "exit"
        };
        let failure_kind = (outcome_kind != "success").then_some(outcome_kind);
        let status = if timed_out {
            Some("timeout".to_string())
        } else {
            exit_code
                .as_i64()
                .map(|code| format!("exit status: {code}"))
        };
        let argv = outcome.command.clone().unwrap_or_default();
        let executable = argv.first().cloned();
        return Some(json!({
            "type": "process",
            "tool": "v8-runner",
            "operation": operation,
            "outcome_kind": outcome_kind,
            "failure_kind": failure_kind,
            "executable": executable,
            "argv": argv,
            "cwd": context.cwd.display().to_string(),
            "status": status,
            "exit_code": exit_code,
            "timed_out": timed_out,
            "timeout_ms": args.get("waitTimeoutMs"),
            "timeout_source": "v8-runner-external-epf",
            "stdout_tail": result_tail(outcome.stdout.as_deref().unwrap_or_default()),
            "stderr_tail": result_tail(outcome.stderr.as_deref().unwrap_or_default()),
            "error": outcome.errors.first(),
            "external_epf_wait": wait,
        }));
    }
    if outcome.ok {
        return None;
    }
    let failure_kind = runtime_failure_kind(outcome);
    let status = runtime_failure_status(outcome, failure_kind);
    let argv = outcome.command.clone().unwrap_or_default();
    let executable = argv.first().cloned();
    Some(json!({
        "type": "process",
        "tool": "v8-runner",
        "operation": operation,
        "failure_kind": failure_kind,
        "executable": executable,
        "argv": argv,
        "cwd": context.cwd.display().to_string(),
        "status": status,
        "exit_code": status.as_deref().and_then(process_exit_code),
        "timed_out": failure_kind == "timeout",
        "timeout_seconds": Option::<u64>::None,
        "timeout_source": "delegated-to-v8-runner",
        "stdout_tail": result_tail(outcome.stdout.as_deref().unwrap_or_default()),
        "stderr_tail": result_tail(outcome.stderr.as_deref().unwrap_or_default()),
        "error": outcome.errors.first(),
    }))
}

fn runtime_failure_kind(outcome: &AdapterOutcome) -> &'static str {
    if outcome
        .warnings
        .iter()
        .any(|warning| warning.contains("failed to spawn"))
    {
        "spawn"
    } else if outcome
        .warnings
        .iter()
        .any(|warning| warning.contains("timed out"))
    {
        "timeout"
    } else {
        "exit"
    }
}

fn runtime_failure_status(outcome: &AdapterOutcome, failure_kind: &str) -> Option<String> {
    if failure_kind == "spawn" {
        return None;
    }
    if failure_kind == "timeout" {
        return Some("timeout".to_string());
    }
    outcome.warnings.iter().find_map(|warning| {
        warning
            .strip_prefix("internal v8-runner runtime adapter exited with status ")
            .map(str::to_string)
    })
}

fn process_exit_code(status: &str) -> Option<i32> {
    let status = status.trim();
    if status == "timeout" {
        return None;
    }
    if let Ok(code) = status.parse::<i32>() {
        return Some(code);
    }
    status
        .rsplit_once(':')
        .and_then(|(_, tail)| tail.trim().parse::<i32>().ok())
}

fn result_tail(text: &str) -> String {
    const TAIL_CHARS: usize = 4096;
    let char_count = text.chars().count();
    if char_count <= TAIL_CHARS {
        return text.to_string();
    }
    text.chars().skip(char_count - TAIL_CHARS).collect()
}

fn domain_events(spec: ToolSpec, args: &Map<String, Value>) -> Vec<DomainEvent> {
    match spec.handler {
        ToolHandler::NativeOperation {
            event: Some(event), ..
        } => vec![DomainEvent::new(event, spec.name)],
        ToolHandler::BuildRuntime {
            event: Some(event), ..
        } => vec![DomainEvent::new(event, spec.name)],
        ToolHandler::RuntimeAdapter => runtime_event(args)
            .map(|event| vec![DomainEvent::new(event, spec.name)])
            .unwrap_or_default(),
        ToolHandler::RuntimeJob { .. } => Vec::new(),
        _ => Vec::new(),
    }
}

fn runtime_event(args: &Map<String, Value>) -> Option<DomainEventKind> {
    args.get("operation")
        .and_then(Value::as_str)
        .and_then(runtime_event_kind)
}

pub(crate) fn project_status(
    context: &WorkspaceContext,
    source_map: Result<crate::domain::project_sources::ProjectSourceMap, String>,
    tracked_config_dump_info_warning: Option<String>,
) -> AdapterOutcome {
    let mut outcome = AdapterOutcome::ok(format!(
        "workspace root: {}; cache root: {}",
        context.workspace_root.display(),
        context.cache_root.display()
    ));
    outcome
        .artifacts
        .push(context.workspace_root.display().to_string());
    outcome
        .artifacts
        .push(context.cache_root.display().to_string());
    match source_map {
        Ok(source_map) => {
            outcome
                .summary
                .push_str(&format!("; source sets: {}", source_map.source_sets.len()));
            if !source_map.source_sets.is_empty() {
                outcome.stdout = Some(source_set_summary(&source_map));
            }
        }
        Err(error) => outcome
            .warnings
            .push(format!("source-set discovery failed: {error}")),
    }
    if let Some(warning) = tracked_config_dump_info_warning {
        outcome.warnings.push(warning);
    }
    outcome
}

pub(crate) fn project_map(
    source_map: Result<crate::domain::project_sources::ProjectSourceMap, String>,
    tracked_config_dump_info_warning: Option<String>,
) -> AdapterOutcome {
    match source_map {
        Ok(source_map) => {
            let mut outcome = AdapterOutcome::ok(format!(
                "project map discovered {} source set(s)",
                source_map.source_sets.len()
            ));
            if let Some(error) = &source_map.source_selection_error {
                outcome.warnings.push(error.clone());
            }
            if let Some(warning) = tracked_config_dump_info_warning {
                outcome.warnings.push(warning);
            }
            outcome.stdout =
                Some(serde_json::to_string_pretty(&source_map).expect("source map serializes"));
            outcome
        }
        Err(error) => AdapterOutcome {
            ok: false,
            summary: "project map discovery failed".to_string(),
            changes: Vec::new(),
            warnings: tracked_config_dump_info_warning.into_iter().collect(),
            errors: vec![error],
            artifacts: Vec::new(),
            stdout: None,
            stderr: None,
            command: None,
        },
    }
}

fn source_set_summary(source_map: &crate::domain::project_sources::ProjectSourceMap) -> String {
    source_map
        .source_sets
        .iter()
        .map(|source_set| {
            format!(
                "{}: {:?} {:?} {}",
                source_set.name, source_set.kind, source_set.source_format, source_set.path
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn configuration_tools() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "unica.cf.edit",
            description:
                "Edit root Configuration.xml properties, ChildObjects, panels, and home page.",
            mutating: true,
            cache_access: cache_access_for("cf-edit", Some(DomainEventKind::ConfigXmlChanged)),
            handler: ToolHandler::NativeOperation {
                operation: "cf-edit",
                event: Some(DomainEventKind::ConfigXmlChanged),
            },
        },
        ToolSpec {
            name: "unica.cf.info",
            description: "Inspect root Configuration.xml.",
            mutating: false,
            cache_access: cache_access_for("cf-info", None),
            handler: ToolHandler::NativeOperation {
                operation: "cf-info",
                event: None,
            },
        },
        ToolSpec {
            name: "unica.cf.init",
            description: "Create empty 1C configuration XML scaffold.",
            mutating: true,
            cache_access: cache_access_for("cf-init", Some(DomainEventKind::ConfigXmlChanged)),
            handler: ToolHandler::NativeOperation {
                operation: "cf-init",
                event: Some(DomainEventKind::ConfigXmlChanged),
            },
        },
        ToolSpec {
            name: "unica.cf.validate",
            description: "Validate root configuration XML structure.",
            mutating: false,
            cache_access: cache_access_for("cf-validate", None),
            handler: ToolHandler::NativeOperation {
                operation: "cf-validate",
                event: None,
            },
        },
        ToolSpec {
            name: "unica.support.edit",
            description: "Toggle 1C vendor support editing capability or per-object support rule.",
            mutating: true,
            cache_access: cache_access_for("support-edit", Some(DomainEventKind::ConfigXmlChanged)),
            handler: ToolHandler::NativeOperation {
                operation: "support-edit",
                event: Some(DomainEventKind::ConfigXmlChanged),
            },
        },
        ToolSpec {
            name: "unica.cfe.borrow",
            description: "Borrow configuration objects/forms into an extension.",
            mutating: true,
            cache_access: cache_access_for("cfe-borrow", Some(DomainEventKind::CfeChanged)),
            handler: ToolHandler::NativeOperation {
                operation: "cfe-borrow",
                event: Some(DomainEventKind::CfeChanged),
            },
        },
        ToolSpec {
            name: "unica.cfe.diff",
            description: "Inspect extension contents and transferred insertion blocks.",
            mutating: false,
            cache_access: cache_access_for("cfe-diff", None),
            handler: ToolHandler::NativeOperation {
                operation: "cfe-diff",
                event: None,
            },
        },
        ToolSpec {
            name: "unica.cfe.init",
            description: "Create extension XML scaffold.",
            mutating: true,
            cache_access: cache_access_for("cfe-init", Some(DomainEventKind::CfeChanged)),
            handler: ToolHandler::NativeOperation {
                operation: "cfe-init",
                event: Some(DomainEventKind::CfeChanged),
            },
        },
        ToolSpec {
            name: "unica.epf.init",
            description:
                "Create a make-ready external data processor scaffold in a Designer/platform-XML external source-set, optionally with a managed form.",
            mutating: true,
            cache_access: cache_access_for(
                "epf-init",
                Some(DomainEventKind::SourceSetChanged),
            ),
            handler: ToolHandler::NativeOperation {
                operation: "epf-init",
                event: Some(DomainEventKind::SourceSetChanged),
            },
        },
        ToolSpec {
            name: "unica.erf.init",
            description:
                "Create a make-ready external report scaffold in a Designer/platform-XML external source-set, optionally with a managed form.",
            mutating: true,
            cache_access: cache_access_for(
                "erf-init",
                Some(DomainEventKind::SourceSetChanged),
            ),
            handler: ToolHandler::NativeOperation {
                operation: "erf-init",
                event: Some(DomainEventKind::SourceSetChanged),
            },
        },
        ToolSpec {
            name: "unica.cfe.patch_method",
            description:
                "Generate a CFE Before/After interceptor for a caller-verified existing parameterless procedure on a registered adopted object.",
            mutating: true,
            cache_access: cache_access_for(
                "cfe-patch-method",
                Some(DomainEventKind::ModuleChanged),
            ),
            handler: ToolHandler::NativeOperation {
                operation: "cfe-patch-method",
                event: Some(DomainEventKind::ModuleChanged),
            },
        },
        ToolSpec {
            name: "unica.cfe.validate",
            description: "Validate extension XML structure.",
            mutating: false,
            cache_access: cache_access_for("cfe-validate", None),
            handler: ToolHandler::NativeOperation {
                operation: "cfe-validate",
                event: None,
            },
        },
        ToolSpec {
            name: "unica.meta.compile",
            description: "Compile metadata object XML from JSON DSL.",
            mutating: true,
            cache_access: cache_access_for("meta-compile", Some(DomainEventKind::MetadataChanged)),
            handler: ToolHandler::NativeOperation {
                operation: "meta-compile",
                event: Some(DomainEventKind::MetadataChanged),
            },
        },
        ToolSpec {
            name: "unica.meta.edit",
            description: "Edit metadata object XML.",
            mutating: true,
            cache_access: cache_access_for("meta-edit", Some(DomainEventKind::MetadataChanged)),
            handler: ToolHandler::NativeOperation {
                operation: "meta-edit",
                event: Some(DomainEventKind::MetadataChanged),
            },
        },
        ToolSpec {
            name: "unica.meta.info",
            description: "Inspect metadata object XML.",
            mutating: false,
            cache_access: cache_access_for("meta-info", None),
            handler: ToolHandler::NativeOperation {
                operation: "meta-info",
                event: None,
            },
        },
        ToolSpec {
            name: "unica.meta.profile",
            description: "Read compact metadata object profile from the internal RLM index.",
            mutating: false,
            cache_access: CacheAccess {
                reads: &["bsl_index"],
                writes: &[],
            },
            handler: ToolHandler::CodeAdapter {
                command: &["meta-profile"],
            },
        },
        ToolSpec {
            name: "unica.meta.remove",
            description: "Remove metadata object XML and registration.",
            mutating: true,
            cache_access: cache_access_for("meta-remove", Some(DomainEventKind::MetadataChanged)),
            handler: ToolHandler::NativeOperation {
                operation: "meta-remove",
                event: Some(DomainEventKind::MetadataChanged),
            },
        },
        ToolSpec {
            name: "unica.meta.validate",
            description: "Validate metadata object XML.",
            mutating: false,
            cache_access: cache_access_for("meta-validate", None),
            handler: ToolHandler::NativeOperation {
                operation: "meta-validate",
                event: None,
            },
        },
        ToolSpec {
            name: "unica.help.add",
            description: "Add built-in help metadata and page to a 1C object.",
            mutating: true,
            cache_access: cache_access_for("help-add", Some(DomainEventKind::FormChanged)),
            handler: ToolHandler::NativeOperation {
                operation: "help-add",
                event: Some(DomainEventKind::FormChanged),
            },
        },
        ToolSpec {
            name: "unica.form.add",
            description: "Add managed form metadata and files.",
            mutating: true,
            cache_access: cache_access_for("form-add", Some(DomainEventKind::FormChanged)),
            handler: ToolHandler::NativeOperation {
                operation: "form-add",
                event: Some(DomainEventKind::FormChanged),
            },
        },
        ToolSpec {
            name: "unica.form.compile",
            description: "Compile managed Form.xml from JSON DSL or metadata.",
            mutating: true,
            cache_access: cache_access_for("form-compile", Some(DomainEventKind::FormChanged)),
            handler: ToolHandler::NativeOperation {
                operation: "form-compile",
                event: Some(DomainEventKind::FormChanged),
            },
        },
        ToolSpec {
            name: "unica.form.edit",
            description:
                "Edit managed Form.xml elements, attributes, commands, and validated events.",
            mutating: true,
            cache_access: cache_access_for("form-edit", Some(DomainEventKind::FormChanged)),
            handler: ToolHandler::NativeOperation {
                operation: "form-edit",
                event: Some(DomainEventKind::FormChanged),
            },
        },
        ToolSpec {
            name: "unica.form.info",
            description: "Inspect managed Form.xml.",
            mutating: false,
            cache_access: cache_access_for("form-info", None),
            handler: ToolHandler::NativeOperation {
                operation: "form-info",
                event: None,
            },
        },
        ToolSpec {
            name: "unica.form.remove",
            description: "Remove a managed form and registration.",
            mutating: true,
            cache_access: cache_access_for("form-remove", Some(DomainEventKind::FormChanged)),
            handler: ToolHandler::NativeOperation {
                operation: "form-remove",
                event: Some(DomainEventKind::FormChanged),
            },
        },
        ToolSpec {
            name: "unica.form.validate",
            description: "Validate managed Form.xml.",
            mutating: false,
            cache_access: cache_access_for("form-validate", None),
            handler: ToolHandler::NativeOperation {
                operation: "form-validate",
                event: None,
            },
        },
        ToolSpec {
            name: "unica.interface.edit",
            description: "Edit subsystem CommandInterface.xml.",
            mutating: true,
            cache_access: cache_access_for(
                "interface-edit",
                Some(DomainEventKind::SubsystemChanged),
            ),
            handler: ToolHandler::NativeOperation {
                operation: "interface-edit",
                event: Some(DomainEventKind::SubsystemChanged),
            },
        },
        ToolSpec {
            name: "unica.interface.validate",
            description: "Validate CommandInterface.xml.",
            mutating: false,
            cache_access: cache_access_for("interface-validate", None),
            handler: ToolHandler::NativeOperation {
                operation: "interface-validate",
                event: None,
            },
        },
        ToolSpec {
            name: "unica.subsystem.compile",
            description: "Compile subsystem XML from JSON DSL.",
            mutating: true,
            cache_access: cache_access_for(
                "subsystem-compile",
                Some(DomainEventKind::SubsystemChanged),
            ),
            handler: ToolHandler::NativeOperation {
                operation: "subsystem-compile",
                event: Some(DomainEventKind::SubsystemChanged),
            },
        },
        ToolSpec {
            name: "unica.subsystem.edit",
            description: "Edit subsystem XML content and hierarchy.",
            mutating: true,
            cache_access: cache_access_for(
                "subsystem-edit",
                Some(DomainEventKind::SubsystemChanged),
            ),
            handler: ToolHandler::NativeOperation {
                operation: "subsystem-edit",
                event: Some(DomainEventKind::SubsystemChanged),
            },
        },
        ToolSpec {
            name: "unica.subsystem.info",
            description: "Inspect subsystem XML and command interface.",
            mutating: false,
            cache_access: cache_access_for("subsystem-info", None),
            handler: ToolHandler::NativeOperation {
                operation: "subsystem-info",
                event: None,
            },
        },
        ToolSpec {
            name: "unica.subsystem.validate",
            description: "Validate subsystem XML.",
            mutating: false,
            cache_access: cache_access_for("subsystem-validate", None),
            handler: ToolHandler::NativeOperation {
                operation: "subsystem-validate",
                event: None,
            },
        },
        ToolSpec {
            name: "unica.template.add",
            description: "Add a template to an object and register it.",
            mutating: true,
            cache_access: cache_access_for("template-add", Some(DomainEventKind::TemplateChanged)),
            handler: ToolHandler::NativeOperation {
                operation: "template-add",
                event: Some(DomainEventKind::TemplateChanged),
            },
        },
        ToolSpec {
            name: "unica.template.remove",
            description: "Remove a template from an object.",
            mutating: true,
            cache_access: cache_access_for(
                "template-remove",
                Some(DomainEventKind::TemplateChanged),
            ),
            handler: ToolHandler::NativeOperation {
                operation: "template-remove",
                event: Some(DomainEventKind::TemplateChanged),
            },
        },
        ToolSpec {
            name: "unica.dcs.compile",
            description: "Compile Data Composition Schema XML from JSON DSL.",
            mutating: true,
            cache_access: cache_access_for("dcs-compile", Some(DomainEventKind::DcsChanged)),
            handler: ToolHandler::NativeOperation {
                operation: "dcs-compile",
                event: Some(DomainEventKind::DcsChanged),
            },
        },
        ToolSpec {
            name: "unica.dcs.edit",
            description: "Edit Data Composition Schema Template.xml.",
            mutating: true,
            cache_access: cache_access_for("dcs-edit", Some(DomainEventKind::DcsChanged)),
            handler: ToolHandler::NativeOperation {
                operation: "dcs-edit",
                event: Some(DomainEventKind::DcsChanged),
            },
        },
        ToolSpec {
            name: "unica.dcs.info",
            description: "Inspect Data Composition Schema Template.xml.",
            mutating: false,
            cache_access: cache_access_for("dcs-info", None),
            handler: ToolHandler::NativeOperation {
                operation: "dcs-info",
                event: None,
            },
        },
        ToolSpec {
            name: "unica.dcs.validate",
            description: "Validate Data Composition Schema Template.xml.",
            mutating: false,
            cache_access: cache_access_for("dcs-validate", None),
            handler: ToolHandler::NativeOperation {
                operation: "dcs-validate",
                event: None,
            },
        },
        ToolSpec {
            name: "unica.mxl.compile",
            description: "Compile spreadsheet Template.xml from JSON DSL.",
            mutating: true,
            cache_access: cache_access_for("mxl-compile", Some(DomainEventKind::MxlChanged)),
            handler: ToolHandler::NativeOperation {
                operation: "mxl-compile",
                event: Some(DomainEventKind::MxlChanged),
            },
        },
        ToolSpec {
            name: "unica.mxl.decompile",
            description: "Decompile spreadsheet Template.xml to JSON DSL.",
            mutating: false,
            cache_access: cache_access_for("mxl-decompile", None),
            handler: ToolHandler::NativeOperation {
                operation: "mxl-decompile",
                event: None,
            },
        },
        ToolSpec {
            name: "unica.mxl.info",
            description: "Inspect spreadsheet Template.xml.",
            mutating: false,
            cache_access: cache_access_for("mxl-info", None),
            handler: ToolHandler::NativeOperation {
                operation: "mxl-info",
                event: None,
            },
        },
        ToolSpec {
            name: "unica.mxl.validate",
            description: "Validate spreadsheet Template.xml.",
            mutating: false,
            cache_access: cache_access_for("mxl-validate", None),
            handler: ToolHandler::NativeOperation {
                operation: "mxl-validate",
                event: None,
            },
        },
        ToolSpec {
            name: "unica.role.compile",
            description: "Compile role metadata and Rights.xml from JSON DSL.",
            mutating: true,
            cache_access: cache_access_for("role-compile", Some(DomainEventKind::RoleChanged)),
            handler: ToolHandler::NativeOperation {
                operation: "role-compile",
                event: Some(DomainEventKind::RoleChanged),
            },
        },
        ToolSpec {
            name: "unica.role.info",
            description: "Inspect role Rights.xml.",
            mutating: false,
            cache_access: cache_access_for("role-info", None),
            handler: ToolHandler::NativeOperation {
                operation: "role-info",
                event: None,
            },
        },
        ToolSpec {
            name: "unica.role.validate",
            description: "Validate role Rights.xml.",
            mutating: false,
            cache_access: cache_access_for("role-validate", None),
            handler: ToolHandler::NativeOperation {
                operation: "role-validate",
                event: None,
            },
        },
    ]
}

fn cache_access_for(operation: &str, event: Option<DomainEventKind>) -> CacheAccess {
    if event.is_some() {
        return CacheAccess {
            reads: &[],
            writes: &["metadata_graph"],
        };
    }

    if operation.starts_with("form-") {
        CacheAccess {
            reads: &["metadata_graph", "form_graph"],
            writes: &[],
        }
    } else if operation.starts_with("role-") {
        CacheAccess {
            reads: &["metadata_graph", "rights_graph"],
            writes: &[],
        }
    } else if operation.starts_with("dcs-") {
        CacheAccess {
            reads: &["metadata_graph", "dcs_graph"],
            writes: &[],
        }
    } else if operation.starts_with("mxl-") {
        CacheAccess {
            reads: &["metadata_graph", "mxl_graph"],
            writes: &[],
        }
    } else if operation.starts_with("subsystem-") || operation.starts_with("interface-") {
        CacheAccess {
            reads: &[
                "metadata_graph",
                "subsystem_graph",
                "command_interface_graph",
            ],
            writes: &[],
        }
    } else {
        CacheAccess {
            reads: &["workspace_graph", "metadata_graph"],
            writes: &[],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::composition::testing::{
        create_file_link_fixture_for_test, prepare_file_for_removal, set_unix_mode_for_test,
        unix_mode_for_test, with_publication_lock_contention_signal, with_publication_lock_pause,
        CompileTransaction, FileLinkFixtureOutcome,
    };
    use serde_json::Map;
    use std::collections::HashSet;
    use std::sync::{mpsc, Arc, Barrier};
    use std::thread;
    use std::time::Duration;

    #[test]
    fn lists_unica_orchestrator_scope() {
        let names = tools().iter().map(|tool| tool.name).collect::<Vec<_>>();
        assert!(names.contains(&"unica.project.status"));
        assert!(names.contains(&"unica.project.map"));
        assert!(names.contains(&"unica.form.validate"));
        assert!(names.contains(&"unica.dcs.edit"));
        assert!(names.contains(&"unica.mxl.compile"));
        assert!(names.contains(&"unica.role.validate"));
        assert!(names.contains(&"unica.support.edit"));
        assert!(names.contains(&"unica.epf.init"));
        assert!(names.contains(&"unica.erf.init"));
        assert!(names.contains(&"unica.build.load"));
        assert!(names.contains(&"unica.runtime.execute"));
        for name in [
            "unica.runtime.job.start",
            "unica.runtime.job.status",
            "unica.runtime.job.wait",
            "unica.runtime.job.logs",
            "unica.runtime.job.cancel",
            "unica.runtime.job.list",
        ] {
            assert!(names.contains(&name), "missing {name}");
        }
        assert!(names.contains(&"unica.code.definition"));
        assert!(names.contains(&"unica.code.outline"));
        assert!(names.contains(&"unica.code.grep"));
        assert!(names.contains(&"unica.code.graph"));
        assert!(names.contains(&"unica.meta.profile"));
        assert!(names.contains(&"unica.standards.explain"));
        assert!(!names.contains(&"unica-coder"));
    }

    #[test]
    fn cfe_patch_method_public_description_states_the_v1_procedure_boundary() {
        let tool = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.cfe.patch_method")
            .expect("cfe.patch_method is public");

        assert!(tool.description.contains("Before/After"));
        assert!(tool.description.contains("caller-verified"));
        assert!(tool.description.contains("parameterless procedure"));
        assert!(!tool.description.contains("method interceptor"));
    }

    #[test]
    fn operation_result_serializes_typed_data_and_omits_absent_data() {
        fn result(data: Option<Value>) -> OperationResult {
            OperationResult {
                ok: true,
                summary: "test".to_string(),
                changes: Vec::new(),
                warnings: Vec::new(),
                errors: Vec::new(),
                artifacts: Vec::new(),
                cache: CacheReport {
                    mode: "read".to_string(),
                    root: ".build/unica".to_string(),
                    workspace_epoch: 1,
                    events: Vec::new(),
                    invalidated: Vec::new(),
                    refreshed: Vec::new(),
                    lazy_rebuilt: Vec::new(),
                    stale: Vec::new(),
                    fresh: Vec::new(),
                },
                stdout: None,
                stderr: None,
                command: None,
                diagnostics: None,
                data,
                job: None,
            }
        }

        let plain = serde_json::to_value(result(None)).expect("plain result must serialize");
        assert!(plain.get("data").is_none());

        let data = json!({"path": "src/Module.bsl", "noOp": false});
        let structured =
            serde_json::to_value(result(Some(data.clone()))).expect("typed result must serialize");
        assert_eq!(structured["data"], data);
        assert!(structured.get("stdout").is_none());
    }

    #[test]
    fn code_patch_public_result_is_typed_and_emits_only_applied_change_events() {
        let root = test_workspace_root("unica-code-patch-public-result");
        let workspace = root.join("workspace");
        let src = workspace.join("src");
        let module = src.join("CommonModules/Sample/Ext/Module.bsl");
        std::fs::create_dir_all(module.parent().unwrap()).unwrap();
        std::fs::write(
            workspace.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(
            src.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration/></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(src.join("CommonModules/Sample.xml"), "<MetaDataObject/>").unwrap();
        std::fs::write(
            &module,
            "Procedure Run()\n    Message(\"ok\");\nEndProcedure\n",
        )
        .unwrap();
        let app = UnicaApplication::new();
        let mut args = json!({
            "cwd": workspace,
            "sourceDir": "src",
            "path": "src/CommonModules/Sample/Ext/Module.bsl",
            "operation": "insert",
            "selector": {"method": "Run"},
            "content": "Procedure Added()\nEndProcedure",
            "position": "after"
        })
        .as_object()
        .unwrap()
        .clone();

        let preview = app.call_tool("unica.code.patch", &args).unwrap();
        assert!(preview.ok, "{:?}", preview.errors);
        assert!(preview.stdout.is_none());
        assert!(preview.cache.events.is_empty());
        assert_eq!(preview.data.as_ref().unwrap()["sourceSet"], "main");
        assert_eq!(
            preview.data.as_ref().unwrap()["affectedTarget"]["owner"],
            "CommonModule.Sample"
        );
        assert_eq!(
            preview.data.as_ref().unwrap()["validation"]["status"],
            "passed"
        );
        let serialized = serde_json::to_value(&preview).unwrap();
        assert!(serialized["data"].is_object());
        assert!(serialized.get("stdout").is_none());
        assert!(!std::fs::read_to_string(&module)
            .unwrap()
            .contains("Procedure Added"));

        args.insert("dryRun".to_string(), json!(false));
        let applied = app.call_tool("unica.code.patch", &args).unwrap();
        assert!(applied.ok, "{:?}", applied.errors);
        assert_eq!(applied.cache.events, vec!["ModuleChanged"]);
        assert_eq!(applied.cache.mode, "applied");

        let repeated = app.call_tool("unica.code.patch", &args).unwrap();
        assert!(repeated.ok, "{:?}", repeated.errors);
        assert!(repeated.cache.events.is_empty());
        assert_eq!(repeated.data.as_ref().unwrap()["noOp"], true);

        let before_invalid = std::fs::read(&module).unwrap();
        args.insert(
            "selector".to_string(),
            json!({"anchor": "Message(\"ok\");"}),
        );
        args.insert("content".to_string(), json!("    If True Then"));
        let rejected = app.call_tool("unica.code.patch", &args).unwrap();
        assert!(!rejected.ok);
        assert!(rejected.cache.events.is_empty());
        assert_eq!(
            rejected.data.as_ref().unwrap()["validation"]["status"],
            "failed"
        );
        assert_eq!(std::fs::read(&module).unwrap(), before_invalid);

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn code_patch_apply_is_blocked_for_a_locked_supported_object() {
        let root = test_workspace_root("unica-code-patch-support-guard");
        let workspace = root.join("workspace");
        let src = workspace.join("src");
        let module = src.join("Catalogs/Items/Ext/ObjectModule.bsl");
        std::fs::create_dir_all(module.parent().unwrap()).unwrap();
        std::fs::create_dir_all(src.join("Ext")).unwrap();
        std::fs::write(
            workspace.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(
            src.join("Configuration.xml"),
            support_test_configuration_xml("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"),
        )
        .unwrap();
        std::fs::write(
            src.join("Catalogs/Items.xml"),
            support_test_catalog_xml("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"),
        )
        .unwrap();
        std::fs::write(
            src.join("Ext/ParentConfigurations.bin"),
            support_test_parent_configurations_bin(
                "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
                "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb",
                "cccccccc-cccc-cccc-cccc-cccccccccccc",
            ),
        )
        .unwrap();
        let before = b"Procedure Run()\nEndProcedure\n";
        std::fs::write(&module, before).unwrap();
        let args = json!({
            "cwd": workspace,
            "dryRun": false,
            "sourceDir": "src",
            "path": "src/Catalogs/Items/Ext/ObjectModule.bsl",
            "operation": "insert",
            "selector": {"method": "Run"},
            "content": "Procedure Added()\nEndProcedure",
            "position": "after"
        })
        .as_object()
        .unwrap()
        .clone();

        let result = UnicaApplication::new()
            .call_tool("unica.code.patch", &args)
            .unwrap();

        assert!(!result.ok);
        assert!(result.summary.contains("support guard"));
        assert!(result.errors.join("\n").contains("на замке"));
        assert!(result.data.is_none());
        assert!(result.cache.events.is_empty());
        assert_eq!(std::fs::read(&module).unwrap(), before);

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn mutating_tool_defaults_to_dry_run_and_reports_cache() {
        let result = UnicaApplication::new()
            .call_tool("unica.form.edit", &Map::new())
            .unwrap();
        assert!(result.ok);
        assert!(result.summary.contains("dry run"));
        assert_eq!(result.command, None);
        assert_eq!(result.cache.mode, "dry-run");
        assert!(result.cache.events.contains(&"FormChanged".to_string()));
        assert!(result
            .cache
            .invalidated
            .contains(&"metadata_graph".to_string()));
    }

    #[test]
    fn runtime_execute_defaults_to_dry_run_and_maps_cache_event_by_operation() {
        let mut args = Map::new();
        args.insert("operation".to_string(), Value::String("dump".to_string()));

        let result = UnicaApplication::new()
            .call_tool("unica.runtime.execute", &args)
            .unwrap();

        assert!(result.ok);
        assert!(result.summary.contains("dry run"));
        assert_eq!(result.cache.mode, "dry-run");
        assert!(result
            .cache
            .events
            .contains(&"SourceSetChanged".to_string()));
        assert!(result.command.unwrap().join(" ").contains(" dump"));
    }

    #[test]
    fn applied_partial_dump_is_blocked_until_runner_can_publish_through_staging() {
        let root = test_workspace_root("runtime-partial-dump-guard");
        let mut args = Map::new();
        args.insert("cwd".to_string(), json!(root));
        args.insert("dryRun".to_string(), json!(false));
        args.insert("operation".to_string(), json!("dump"));
        args.insert("mode".to_string(), json!("partial"));
        args.insert("object".to_string(), json!("Catalog:Items"));

        let result = UnicaApplication::with_ports(Arc::new(FixedOutcomePorts {
            outcome: AdapterOutcome::ok("runtime adapter must not be invoked"),
            data: None,
        }))
        .call_tool("unica.runtime.execute", &args)
        .unwrap();

        assert!(!result.ok);
        assert!(result.summary.contains("source sync guard"));
        let errors = result.errors.join("\n");
        assert!(errors.contains("v8-runner-rust#30"));
        assert!(errors.contains("DESIGNER"));
        assert!(errors.contains("EDT"));
        assert!(errors.contains("divergence-safe merge"));
        assert!(result.changes.is_empty());

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn applied_incremental_dump_is_blocked_at_every_unica_runtime_entry_point() {
        let root = test_workspace_root("runtime-incremental-dump-guard");
        let app = UnicaApplication::with_ports(Arc::new(FixedOutcomePorts {
            outcome: AdapterOutcome::ok("runtime adapter must not be invoked"),
            data: None,
        }));

        for (tool, include_operation) in [
            ("unica.build.dump", false),
            ("unica.runtime.execute", true),
            ("unica.runtime.job.start", true),
        ] {
            let mut args = Map::new();
            args.insert("cwd".to_string(), json!(root));
            args.insert("dryRun".to_string(), json!(false));
            args.insert("mode".to_string(), json!("incremental"));
            if include_operation {
                args.insert("operation".to_string(), json!("dump"));
            }

            let result = app.call_tool(tool, &args).unwrap();
            assert!(!result.ok, "{tool} must be fail-closed");
            assert!(result.summary.contains("source sync guard"));
        }

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn applied_dump_requires_explicit_full_mode_at_every_runtime_entry_point() {
        let root = test_workspace_root("runtime-explicit-full-dump-guard");
        let app = UnicaApplication::with_ports(Arc::new(FixedOutcomePorts {
            outcome: AdapterOutcome::ok("runtime adapter must not be invoked"),
            data: None,
        }));

        for (tool, include_operation) in [
            ("unica.build.dump", false),
            ("unica.runtime.execute", true),
            ("unica.runtime.job.start", true),
        ] {
            let mut args = Map::new();
            args.insert("cwd".to_string(), json!(root));
            args.insert("dryRun".to_string(), json!(false));
            if include_operation {
                args.insert("operation".to_string(), json!("dump"));
            }

            let result = app.call_tool(tool, &args).unwrap();
            assert!(!result.ok, "{tool} must require explicit mode=full");
            assert!(result.summary.contains("source sync guard"));
        }

        let mut unknown_mode = Map::new();
        unknown_mode.insert("cwd".to_string(), json!(root));
        unknown_mode.insert("dryRun".to_string(), json!(false));
        unknown_mode.insert("mode".to_string(), json!("future-mode"));
        let result = app.call_tool("unica.build.dump", &unknown_mode).unwrap();
        assert!(!result.ok);
        assert!(result.summary.contains("source sync guard"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn cancelled_applied_dump_wins_over_source_sync_guard() {
        let root = test_workspace_root("runtime-cancelled-dump-guard");
        let app = UnicaApplication::with_ports(Arc::new(FixedOutcomePorts {
            outcome: AdapterOutcome::ok("runtime adapter must not be invoked"),
            data: None,
        }));
        let mut args = Map::new();
        args.insert("cwd".to_string(), json!(root));
        args.insert("dryRun".to_string(), json!(false));
        args.insert("operation".to_string(), json!("dump"));
        args.insert("mode".to_string(), json!("incremental"));
        let cancellation = CancellationToken::new();
        cancellation.cancel();

        let result = app
            .call_tool_cancellable("unica.runtime.execute", &args, cancellation)
            .unwrap();

        assert!(!result.ok);
        assert!(result.errors[0].starts_with("cancelled:"));
        assert!(!result.summary.contains("source sync guard"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn applied_full_dump_is_synchronous_only_until_jobs_can_validate_before_publication() {
        let root = test_workspace_root("runtime-full-dump-profile-guard");
        let app = UnicaApplication::with_ports(Arc::new(FixedOutcomePorts {
            outcome: AdapterOutcome::ok("verified synchronous dump adapter invoked"),
            data: None,
        }));

        for (tool, include_operation) in
            [("unica.build.dump", false), ("unica.runtime.execute", true)]
        {
            let mut args = Map::new();
            args.insert("cwd".to_string(), json!(root));
            args.insert("dryRun".to_string(), json!(false));
            args.insert("mode".to_string(), json!("full"));
            if include_operation {
                args.insert("operation".to_string(), json!("dump"));
            }

            let result = app.call_tool(tool, &args).unwrap();
            assert!(result.ok, "{tool}: {result:?}");
            assert_eq!(
                result.summary, "verified synchronous dump adapter invoked",
                "{tool}: {result:?}"
            );
        }

        let mut job_args = Map::new();
        job_args.insert("cwd".to_string(), json!(root));
        job_args.insert("dryRun".to_string(), json!(false));
        job_args.insert("mode".to_string(), json!("full"));
        job_args.insert("operation".to_string(), json!("dump"));
        let job = app.call_tool("unica.runtime.job.start", &job_args).unwrap();
        assert!(!job.ok, "{job:?}");
        assert!(job.summary.contains("source sync guard"), "{job:?}");
        let errors = job.errors.join("\n");
        assert!(errors.contains("asynchronous"), "{job:?}");
        assert!(errors.contains("8.3.27"), "{job:?}");
        assert!(errors.contains("2.20"), "{job:?}");
        assert!(errors.contains("unica.runtime.execute"), "{job:?}");
        assert!(job.changes.is_empty(), "{job:?}");
        assert!(job.job.is_none(), "{job:?}");

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn non_dump_platform_xml_routes_are_fail_closed_before_runtime_handlers() {
        let root = test_workspace_root("runtime-non-dump-xml-route-guard");
        let app = UnicaApplication::with_ports(Arc::new(FixedOutcomePorts {
            outcome: AdapterOutcome::ok("runtime adapter must not be invoked"),
            data: None,
        }));

        for tool in ["unica.runtime.execute", "unica.runtime.job.start"] {
            let mut convert = Map::new();
            convert.insert("cwd".to_string(), json!(root));
            convert.insert("dryRun".to_string(), json!(false));
            convert.insert("operation".to_string(), json!("convert"));
            convert.insert("output".to_string(), json!("designer-out"));
            let result = app.call_tool(tool, &convert).unwrap();
            assert!(!result.ok, "{tool}: {result:?}");
            assert!(
                result.summary.contains("runtime XML route guard"),
                "{tool}: {result:?}"
            );
            assert!(result.errors.join("\n").contains("EDT-to-Designer"));
            assert!(result.changes.is_empty());

            for reserved in ["/DumpConfigToFiles", "/LoadConfigFromFiles"] {
                let mut launch = Map::new();
                launch.insert("cwd".to_string(), json!(root));
                launch.insert("dryRun".to_string(), json!(false));
                launch.insert("operation".to_string(), json!("launch"));
                launch.insert("clientMode".to_string(), json!("designer"));
                launch.insert("rawKeys".to_string(), json!([reserved, "git-visible-src"]));
                let result = app.call_tool(tool, &launch).unwrap();
                assert!(!result.ok, "{tool} {reserved}: {result:?}");
                assert!(
                    result.summary.contains("runtime XML route guard"),
                    "{tool} {reserved}: {result:?}"
                );
                assert!(
                    result.errors.join("\n").contains("reserved"),
                    "{tool} {reserved}: {result:?}"
                );
                assert!(result.changes.is_empty());
            }
        }

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn non_dump_platform_xml_route_previews_remain_non_executing() {
        let root = test_workspace_root("runtime-non-dump-xml-route-preview");
        let app = UnicaApplication::with_ports(Arc::new(FixedOutcomePorts {
            outcome: AdapterOutcome::ok("runtime preview invoked"),
            data: None,
        }));
        for operation in ["convert", "launch"] {
            let mut args = Map::new();
            args.insert("cwd".to_string(), json!(root));
            args.insert("dryRun".to_string(), json!(true));
            args.insert("operation".to_string(), json!(operation));
            if operation == "launch" {
                args.insert("clientMode".to_string(), json!("designer"));
                args.insert(
                    "rawKeys".to_string(),
                    json!(["/DumpConfigToFiles", "ignored-preview"]),
                );
            }
            let result = app.call_tool("unica.runtime.execute", &args).unwrap();
            assert!(result.ok, "{operation}: {result:?}");
            assert_eq!(result.summary, "runtime preview invoked");
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn dump_previews_and_non_dump_runtime_operations_remain_available() {
        let root = test_workspace_root("runtime-profile-guard-scope");
        let app = UnicaApplication::with_ports(Arc::new(FixedOutcomePorts {
            outcome: AdapterOutcome::ok("runtime adapter invoked"),
            data: None,
        }));

        for (tool, include_operation) in [
            ("unica.build.dump", false),
            ("unica.runtime.execute", true),
            ("unica.runtime.job.start", true),
        ] {
            let mut args = Map::new();
            args.insert("cwd".to_string(), json!(root));
            args.insert("dryRun".to_string(), json!(true));
            args.insert("mode".to_string(), json!("full"));
            if include_operation {
                args.insert("operation".to_string(), json!("dump"));
            }

            let preview = app.call_tool(tool, &args).unwrap();
            assert!(preview.ok, "{tool}: {preview:?}");
            assert_eq!(preview.summary, "runtime adapter invoked");
        }

        for (tool, operation) in [
            ("unica.build.load", None),
            ("unica.runtime.execute", Some("build")),
            ("unica.runtime.job.start", Some("build")),
        ] {
            let mut args = Map::new();
            args.insert("cwd".to_string(), json!(root));
            args.insert("dryRun".to_string(), json!(false));
            if let Some(operation) = operation {
                args.insert("operation".to_string(), json!(operation));
            }

            let applied = app.call_tool(tool, &args).unwrap();
            assert!(applied.ok, "{tool}: {applied:?}");
            assert_eq!(applied.summary, "runtime adapter invoked");
        }

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn runtime_job_start_defaults_to_dry_run_without_runtime_cache_invalidation() {
        let mut args = Map::new();
        args.insert("operation".to_string(), Value::String("dump".to_string()));

        let result = UnicaApplication::new()
            .call_tool("unica.runtime.job.start", &args)
            .expect("dry-run job start succeeds");

        assert!(result.ok);
        assert!(result.summary.contains("dry run"));
        assert_eq!(result.job, None);
        assert_eq!(result.cache.mode, "read");
        assert!(result.cache.events.is_empty());
    }

    #[test]
    fn runtime_event_is_not_emitted_for_non_invalidating_operations() {
        let mut args = Map::new();
        args.insert("operation".to_string(), Value::String("launch".to_string()));
        args.insert("clientMode".to_string(), Value::String("thin".to_string()));

        let result = UnicaApplication::new()
            .call_tool("unica.runtime.execute", &args)
            .unwrap();

        assert!(result.ok);
        assert!(result.cache.events.is_empty());
        assert_eq!(result.cache.mode, "read");
    }

    #[test]
    fn mutating_native_noop_does_not_emit_cache_events() {
        let mut outcome = AdapterOutcome::ok("no changes");
        outcome.changes = Vec::new();
        let spec = ToolSpec {
            name: "unica.cf.edit",
            description: "test",
            mutating: true,
            cache_access: cache_access_for("cf-edit", Some(DomainEventKind::ConfigXmlChanged)),
            handler: ToolHandler::NativeOperation {
                operation: "cf-edit",
                event: Some(DomainEventKind::ConfigXmlChanged),
            },
        };

        let args = Map::new();
        assert!(!should_emit_events(spec, &args, false, &outcome));

        outcome
            .changes
            .push("updated Configuration.xml".to_string());
        assert!(should_emit_events(spec, &args, false, &outcome));
        assert!(should_emit_events(
            spec,
            &args,
            true,
            &AdapterOutcome::ok("generic dry run")
        ));

        let code_patch_spec = ToolSpec {
            name: "unica.code.patch",
            description: "test",
            mutating: true,
            cache_access: cache_access_for("code-patch", Some(DomainEventKind::ModuleChanged)),
            handler: ToolHandler::NativeOperation {
                operation: "code-patch",
                event: Some(DomainEventKind::ModuleChanged),
            },
        };
        assert!(!should_emit_events(
            code_patch_spec,
            &args,
            true,
            &AdapterOutcome::ok("code patch preview")
        ));

        let form_edit_spec = ToolSpec {
            name: "unica.form.edit",
            description: "test",
            mutating: true,
            cache_access: cache_access_for("form-edit", Some(DomainEventKind::FormChanged)),
            handler: ToolHandler::NativeOperation {
                operation: "form-edit",
                event: Some(DomainEventKind::FormChanged),
            },
        };
        let semantic_args = Map::from_iter([
            ("FormPath".to_string(), json!("Form.xml")),
            ("definition".to_string(), json!({"formEvents": []})),
        ]);
        assert!(!should_emit_events(
            form_edit_spec,
            &semantic_args,
            true,
            &AdapterOutcome::ok("semantic dry run no-op")
        ));

        let mut planned = AdapterOutcome::ok("dry run planned change");
        planned.changes.push("would update Form.xml".to_string());
        assert!(should_emit_events(
            form_edit_spec,
            &semantic_args,
            true,
            &planned
        ));

        let mut rejected = AdapterOutcome::ok("dry run rejected");
        rejected.ok = false;
        rejected.changes.push("would update Form.xml".to_string());
        assert!(!should_emit_events(
            form_edit_spec,
            &semantic_args,
            true,
            &rejected
        ));
    }

    #[test]
    fn runtime_failure_result_includes_structured_exit_diagnostics() {
        let root = test_workspace_root("runtime-exit-diagnostics");
        let result = call_runtime_with_outcome(
            &root,
            AdapterOutcome {
                ok: false,
                summary: "unica.runtime.execute failed through internal v8-runner runtime adapter"
                    .to_string(),
                changes: Vec::new(),
                warnings: vec![
                    "internal v8-runner runtime adapter exited with status exit status: 1"
                        .to_string(),
                ],
                errors: vec!["failed to load configuration: Pwd=<redacted>".to_string()],
                artifacts: Vec::new(),
                stdout: Some("started build\nPwd=<redacted>\n".to_string()),
                stderr: Some("failed to load configuration: Pwd=<redacted>\n".to_string()),
                command: Some(vec![
                    "/tmp/unica/plugins/unica/bin/darwin-arm64/v8-runner".to_string(),
                    "build".to_string(),
                    "--source-set".to_string(),
                    "main".to_string(),
                ]),
            },
            "build",
        );

        let diagnostics = result.diagnostics.unwrap();
        assert_eq!(diagnostics["tool"], "v8-runner");
        assert_eq!(diagnostics["operation"], "build");
        assert_eq!(diagnostics["failure_kind"], "exit");
        assert_eq!(diagnostics["exit_code"], 1);
        assert_eq!(diagnostics["timed_out"], false);
        assert_eq!(diagnostics["argv"][1], "build");
        assert_eq!(diagnostics["argv"][2], "--source-set");
        assert_eq!(diagnostics["argv"][3], "main");
        assert_eq!(diagnostics["cwd"], root.display().to_string());
        assert!(diagnostics["stdout_tail"]
            .as_str()
            .unwrap()
            .contains("started build"));
        assert!(!serde_json::to_string(&diagnostics)
            .unwrap()
            .contains("super-secret"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn runtime_failure_result_distinguishes_timeout_diagnostics() {
        let root = test_workspace_root("runtime-timeout-diagnostics");
        let result = call_runtime_with_outcome(
            &root,
            AdapterOutcome {
                ok: false,
                summary: "unica.runtime.execute failed through internal v8-runner runtime adapter"
                    .to_string(),
                changes: Vec::new(),
                warnings: vec!["internal v8-runner runtime adapter timed out".to_string()],
                errors: vec!["internal v8-runner runtime adapter timed out".to_string()],
                artifacts: Vec::new(),
                stdout: Some("started loading configuration...\n".to_string()),
                stderr: Some(String::new()),
                command: Some(vec![
                    "/tmp/unica/plugins/unica/bin/darwin-arm64/v8-runner".to_string(),
                    "load".to_string(),
                    "--path".to_string(),
                    "build/config.cf".to_string(),
                ]),
            },
            "load",
        );

        let diagnostics = result.diagnostics.unwrap();
        assert_eq!(diagnostics["failure_kind"], "timeout");
        assert_eq!(diagnostics["timed_out"], true);
        assert!(diagnostics["timeout_seconds"].is_null());
        assert_eq!(diagnostics["timeout_source"], "delegated-to-v8-runner");
        assert!(diagnostics["stdout_tail"]
            .as_str()
            .unwrap()
            .contains("started loading configuration"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn runtime_failure_result_distinguishes_spawn_diagnostics() {
        let root = test_workspace_root("runtime-spawn-diagnostics");
        let result = call_runtime_with_outcome(
            &root,
            AdapterOutcome {
                ok: false,
                summary: "unica.runtime.execute failed through internal v8-runner runtime adapter"
                    .to_string(),
                changes: Vec::new(),
                warnings: vec![
                    "internal v8-runner runtime adapter failed to spawn process".to_string()
                ],
                errors: vec!["failed to execute process: apiToken=<redacted>".to_string()],
                artifacts: Vec::new(),
                stdout: None,
                stderr: Some("failed to execute process: apiToken=<redacted>\n".to_string()),
                command: Some(vec![
                    "/tmp/unica/plugins/unica/bin/darwin-arm64/v8-runner".to_string(),
                    "build".to_string(),
                ]),
            },
            "build",
        );

        let diagnostics = result.diagnostics.unwrap();
        assert_eq!(diagnostics["failure_kind"], "spawn");
        assert_eq!(diagnostics["operation"], "build");
        assert!(diagnostics["exit_code"].is_null());
        assert_eq!(diagnostics["timed_out"], false);
        assert!(diagnostics["status"].is_null());
        assert!(diagnostics["error"]
            .as_str()
            .unwrap()
            .contains("failed to execute process"));
        assert!(!serde_json::to_string(&diagnostics)
            .unwrap()
            .contains("token-secret"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn bounded_runtime_success_exposes_typed_exit_and_execute_receipt() {
        let root = test_workspace_root("runtime-bounded-success-diagnostics");
        let result = call_runtime_with_outcome_and_data(
            &root,
            AdapterOutcome {
                ok: true,
                summary:
                    "unica.runtime.execute completed through internal v8-runner runtime adapter"
                        .to_string(),
                changes: Vec::new(),
                warnings: Vec::new(),
                errors: Vec::new(),
                artifacts: vec![
                    "build/smoke.out.log".to_string(),
                    "build/smoke.stderr.log".to_string(),
                ],
                stdout: Some("{\"ok\":true}".to_string()),
                stderr: Some(String::new()),
                command: Some(vec![
                    "/tmp/unica/plugins/unica/bin/darwin-arm64/v8-runner".to_string(),
                    "--json-message".to_string(),
                    "launch".to_string(),
                    "thin".to_string(),
                ]),
            },
            Some(json!({
                "external_epf_wait": {
                    "pid": 42,
                    "execute_path": "tests/Smoke.epf",
                    "exit_code": 0,
                    "timed_out": false,
                    "output_path": "build/smoke.out.log",
                    "stderr_path": "build/smoke.stderr.log"
                }
            })),
        );

        assert!(result.ok);
        assert_eq!(
            result.data.as_ref().unwrap()["external_epf_wait"]["pid"],
            42
        );
        let diagnostics = result.diagnostics.unwrap();
        assert_eq!(diagnostics["outcome_kind"], "success");
        assert!(diagnostics["failure_kind"].is_null());
        assert_eq!(diagnostics["exit_code"], 0);
        assert_eq!(diagnostics["timed_out"], false);
        assert_eq!(
            diagnostics["external_epf_wait"]["execute_path"],
            "tests/Smoke.epf"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn bounded_runtime_nonzero_exit_preserves_external_exit_code() {
        let root = test_workspace_root("runtime-bounded-nonzero-diagnostics");
        let result = call_runtime_with_outcome_and_data(
            &root,
            AdapterOutcome {
                ok: false,
                summary: "unica.runtime.execute failed through internal v8-runner runtime adapter"
                    .to_string(),
                changes: Vec::new(),
                warnings: Vec::new(),
                errors: vec!["bounded external EPF exited with code 7".to_string()],
                artifacts: vec![
                    "build/smoke.out.log".to_string(),
                    "build/smoke.stderr.log".to_string(),
                ],
                stdout: Some("{\"ok\":true}".to_string()),
                stderr: Some(String::new()),
                command: Some(vec![
                    "/tmp/unica/plugins/unica/bin/darwin-arm64/v8-runner".to_string(),
                    "--json-message".to_string(),
                    "launch".to_string(),
                    "thin".to_string(),
                ]),
            },
            Some(json!({
                "external_epf_wait": {
                    "pid": 42,
                    "execute_path": "tests/Smoke.epf",
                    "exit_code": 7,
                    "timed_out": false,
                    "output_path": "build/smoke.out.log",
                    "stderr_path": "build/smoke.stderr.log"
                }
            })),
        );

        assert!(!result.ok);
        let diagnostics = result.diagnostics.unwrap();
        assert_eq!(diagnostics["outcome_kind"], "exit");
        assert_eq!(diagnostics["failure_kind"], "exit");
        assert_eq!(diagnostics["exit_code"], 7);
        assert_eq!(diagnostics["timed_out"], false);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn bounded_runtime_timeout_uses_external_process_diagnostics() {
        let root = test_workspace_root("runtime-bounded-timeout-diagnostics");
        let result = call_runtime_with_outcome_and_data(
            &root,
            AdapterOutcome {
                ok: false,
                summary: "unica.runtime.execute failed through internal v8-runner runtime adapter"
                    .to_string(),
                changes: Vec::new(),
                warnings: Vec::new(),
                errors: vec!["bounded external EPF launch timed out".to_string()],
                artifacts: vec![
                    "build/smoke.out.log".to_string(),
                    "build/smoke.stderr.log".to_string(),
                ],
                stdout: Some("{\"ok\":false}".to_string()),
                stderr: Some(String::new()),
                command: Some(vec![
                    "/tmp/unica/plugins/unica/bin/darwin-arm64/v8-runner".to_string(),
                    "--json-message".to_string(),
                    "launch".to_string(),
                    "thin".to_string(),
                ]),
            },
            Some(json!({
                "external_epf_wait": {
                    "pid": 42,
                    "execute_path": "tests/Smoke.epf",
                    "exit_code": null,
                    "timed_out": true,
                    "output_path": "build/smoke.out.log",
                    "stderr_path": "build/smoke.stderr.log"
                }
            })),
        );

        assert!(!result.ok);
        let diagnostics = result.diagnostics.unwrap();
        assert_eq!(diagnostics["outcome_kind"], "timeout");
        assert_eq!(diagnostics["failure_kind"], "timeout");
        assert!(diagnostics["exit_code"].is_null());
        assert_eq!(diagnostics["timed_out"], true);
        assert_eq!(diagnostics["status"], "timeout");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn xml_dsl_tools_route_to_parity_covered_native_handlers() {
        const PARITY_COVERED_TOOLS: &[&str] = &[
            "unica.cf.edit",
            "unica.cf.info",
            "unica.cf.init",
            "unica.cf.validate",
            "unica.cfe.borrow",
            "unica.cfe.diff",
            "unica.cfe.init",
            "unica.cfe.patch_method",
            "unica.cfe.validate",
            "unica.meta.compile",
            "unica.meta.edit",
            "unica.meta.info",
            "unica.meta.remove",
            "unica.meta.validate",
            "unica.help.add",
            "unica.form.add",
            "unica.form.compile",
            "unica.form.edit",
            "unica.form.info",
            "unica.form.remove",
            "unica.form.validate",
            "unica.interface.edit",
            "unica.interface.validate",
            "unica.subsystem.compile",
            "unica.subsystem.edit",
            "unica.subsystem.info",
            "unica.subsystem.validate",
            "unica.template.add",
            "unica.template.remove",
            "unica.dcs.compile",
            "unica.dcs.edit",
            "unica.dcs.info",
            "unica.dcs.validate",
            "unica.mxl.compile",
            "unica.mxl.decompile",
            "unica.mxl.info",
            "unica.mxl.validate",
            "unica.role.compile",
            "unica.role.info",
            "unica.role.validate",
        ];
        const REPO_OWNED_NATIVE_TOOLS: &[&str] = &["unica.support.edit"];

        for tool in tools() {
            if !tool.name.starts_with("unica.cf.")
                && !tool.name.starts_with("unica.cfe.")
                && !tool.name.starts_with("unica.meta.")
                && !tool.name.starts_with("unica.help.")
                && !tool.name.starts_with("unica.form.")
                && !tool.name.starts_with("unica.interface.")
                && !tool.name.starts_with("unica.subsystem.")
                && !tool.name.starts_with("unica.template.")
                && !tool.name.starts_with("unica.dcs.")
                && !tool.name.starts_with("unica.mxl.")
                && !tool.name.starts_with("unica.role.")
                && !tool.name.starts_with("unica.support.")
            {
                continue;
            }
            if tool.name == "unica.meta.profile" {
                continue;
            }

            match tool.handler {
                ToolHandler::NativeOperation { operation, .. } => {
                    assert!(
                        PARITY_COVERED_TOOLS.contains(&tool.name)
                            || REPO_OWNED_NATIVE_TOOLS.contains(&tool.name),
                        "{} routes to native operation {} without a parity fixture or repo-owned native contract exception",
                        tool.name,
                        operation
                    );
                }
                _ => panic!("{} routes through unexpected handler", tool.name),
            }
        }
    }

    #[test]
    fn form_and_dcs_tools_route_through_native_handlers() {
        let expected = [
            (
                "unica.form.add",
                "form-add",
                Some(DomainEventKind::FormChanged),
            ),
            (
                "unica.form.compile",
                "form-compile",
                Some(DomainEventKind::FormChanged),
            ),
            (
                "unica.form.edit",
                "form-edit",
                Some(DomainEventKind::FormChanged),
            ),
            ("unica.form.info", "form-info", None),
            (
                "unica.form.remove",
                "form-remove",
                Some(DomainEventKind::FormChanged),
            ),
            ("unica.form.validate", "form-validate", None),
            (
                "unica.dcs.compile",
                "dcs-compile",
                Some(DomainEventKind::DcsChanged),
            ),
            (
                "unica.dcs.edit",
                "dcs-edit",
                Some(DomainEventKind::DcsChanged),
            ),
            ("unica.dcs.info", "dcs-info", None),
            ("unica.dcs.validate", "dcs-validate", None),
        ];
        for (tool_name, expected_operation, expected_event) in expected {
            let tool = tools()
                .into_iter()
                .find(|tool| tool.name == tool_name)
                .expect("form/DCS tool exists");

            match tool.handler {
                ToolHandler::NativeOperation { operation, event } => {
                    assert_eq!(operation, expected_operation);
                    assert_eq!(event, expected_event);
                }
                other => panic!("{tool_name} should route through native operation, got {other:?}"),
            }
        }
    }

    #[test]
    fn project_status_is_read_only_and_cache_aware() {
        let result = UnicaApplication::new()
            .call_tool("unica.project.status", &Map::new())
            .unwrap();
        assert!(result.ok);
        assert_eq!(result.cache.mode, "read");
        assert!(result.summary.contains("workspace root"));
    }

    #[test]
    fn project_map_reports_source_sets_as_read_only_json() {
        let root = std::env::temp_dir().join(format!("unica-project-map-{}", std::process::id()));
        let workspace = root.join("workspace");
        std::fs::create_dir_all(workspace.join("src")).unwrap();
        std::fs::write(
            workspace.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(workspace.join("src/Configuration.xml"), "<MetaDataObject/>").unwrap();
        let mut args = Map::new();
        args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );

        let result = UnicaApplication::new()
            .call_tool("unica.project.map", &args)
            .unwrap();

        assert!(result.ok);
        assert_eq!(result.cache.mode, "read");
        let stdout = result.stdout.unwrap();
        assert!(stdout.contains("\"sourceSets\""));
        assert!(stdout.contains("\"sourceFormat\": \"platform_xml\""));
        assert!(stdout.contains("\"kind\": \"configuration\""));
        assert!(stdout.contains(r#""effectiveSourceSet": "main""#));
        assert!(stdout.contains(r#""effectiveSourceRoot""#));
        assert!(!stdout.contains("sourceSelectionError"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn project_map_warns_when_config_dump_info_is_tracked_by_git() {
        let root = test_workspace_root("project-map-tracked-cdfi");
        let src = root.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(src.join("Configuration.xml"), "<MetaDataObject/>").unwrap();
        std::fs::write(src.join("configdumpinfo.xml"), "<ConfigDumpInfo/>").unwrap();
        std::process::Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&root)
            .status()
            .unwrap();
        std::process::Command::new("git")
            .args([
                "add",
                "v8project.yaml",
                "src/Configuration.xml",
                "src/configdumpinfo.xml",
            ])
            .current_dir(&root)
            .status()
            .unwrap();
        let mut args = Map::new();
        args.insert("cwd".to_string(), json!(root));

        let result = UnicaApplication::new()
            .call_tool("unica.project.map", &args)
            .unwrap();

        assert!(result.ok);
        assert!(result
            .warnings
            .iter()
            .any(|warning| warning.contains("src/configdumpinfo.xml")
                && warning.contains("git rm --cached")));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn project_map_does_not_warn_for_tracked_external_object_named_config_dump_info() {
        let root = test_workspace_root("project-map-external-object-named-cdfi");
        let epf = root.join("epf");
        let erf = root.join("erf");
        std::fs::create_dir_all(&epf).unwrap();
        std::fs::create_dir_all(&erf).unwrap();
        std::fs::write(
            root.join("v8project.yaml"),
            concat!(
                "format: DESIGNER\n",
                "source-set:\n",
                "  - name: processors\n",
                "    type: EXTERNAL_DATA_PROCESSORS\n",
                "    path: epf\n",
                "  - name: reports\n",
                "    type: EXTERNAL_REPORTS\n",
                "    path: erf\n",
            ),
        )
        .unwrap();
        std::fs::write(
            epf.join("ConfigDumpInfo.xml"),
            "<MetaDataObject><ExternalDataProcessor/></MetaDataObject>",
        )
        .unwrap();
        std::fs::write(
            erf.join("configdumpinfo.xml"),
            "<MetaDataObject><ExternalReport/></MetaDataObject>",
        )
        .unwrap();
        std::process::Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&root)
            .status()
            .unwrap();
        std::process::Command::new("git")
            .args([
                "add",
                "v8project.yaml",
                "epf/ConfigDumpInfo.xml",
                "erf/configdumpinfo.xml",
            ])
            .current_dir(&root)
            .status()
            .unwrap();
        let mut args = Map::new();
        args.insert("cwd".to_string(), json!(root));

        let result = UnicaApplication::new()
            .call_tool("unica.project.map", &args)
            .unwrap();

        assert!(result.ok);
        assert_eq!(
            result
                .stdout
                .as_deref()
                .map(|stdout| stdout.matches(r#""sourceFormat": "platform_xml""#).count()),
            Some(2)
        );
        assert!(
            result
                .warnings
                .iter()
                .all(|warning| !warning.contains("git rm --cached")),
            "valid external descriptor must not be treated as runtime state: {:?}",
            result.warnings
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn project_map_classifies_config_dump_info_from_git_index_not_worktree() {
        let runtime_index = test_workspace_root("project-map-cdfi-runtime-index");
        std::fs::create_dir_all(runtime_index.join("epf")).unwrap();
        std::fs::write(
            runtime_index.join("v8project.yaml"),
            concat!(
                "format: DESIGNER\n",
                "source-set:\n",
                "  - name: processors\n",
                "    type: EXTERNAL_DATA_PROCESSORS\n",
                "    path: epf\n",
            ),
        )
        .unwrap();
        std::fs::write(
            runtime_index.join("epf/ConfigDumpInfo.xml"),
            "<ConfigDumpInfo/>",
        )
        .unwrap();
        std::process::Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&runtime_index)
            .status()
            .unwrap();
        std::process::Command::new("git")
            .args(["add", "v8project.yaml", "epf/ConfigDumpInfo.xml"])
            .current_dir(&runtime_index)
            .status()
            .unwrap();
        std::fs::write(
            runtime_index.join("epf/ConfigDumpInfo.xml"),
            "<MetaDataObject><ExternalDataProcessor/></MetaDataObject>",
        )
        .unwrap();
        let mut args = Map::new();
        args.insert("cwd".to_string(), json!(runtime_index));

        let result = UnicaApplication::new()
            .call_tool("unica.project.map", &args)
            .unwrap();

        assert!(result.warnings.iter().any(|warning| {
            warning.contains("epf/ConfigDumpInfo.xml")
                && warning.contains("git rm --cached")
                && warning.contains("workspace-relative paths")
        }));

        let external_index = test_workspace_root("project-map-cdfi-external-index");
        std::fs::create_dir_all(external_index.join("epf")).unwrap();
        std::fs::write(
            external_index.join("v8project.yaml"),
            concat!(
                "format: DESIGNER\n",
                "source-set:\n",
                "  - name: processors\n",
                "    type: EXTERNAL_DATA_PROCESSORS\n",
                "    path: epf\n",
            ),
        )
        .unwrap();
        std::fs::write(
            external_index.join("epf/ConfigDumpInfo.xml"),
            "<MetaDataObject><ExternalDataProcessor/></MetaDataObject>",
        )
        .unwrap();
        std::process::Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&external_index)
            .status()
            .unwrap();
        std::process::Command::new("git")
            .args(["add", "v8project.yaml", "epf/ConfigDumpInfo.xml"])
            .current_dir(&external_index)
            .status()
            .unwrap();
        std::fs::write(
            external_index.join("epf/ConfigDumpInfo.xml"),
            "<ConfigDumpInfo/>",
        )
        .unwrap();
        let mut args = Map::new();
        args.insert("cwd".to_string(), json!(external_index));

        let result = UnicaApplication::new()
            .call_tool("unica.project.map", &args)
            .unwrap();

        assert!(result.warnings.iter().all(|warning| {
            !warning.contains("git rm --cached") && !warning.contains("manual review")
        }));

        let _ = std::fs::remove_dir_all(runtime_index);
        let _ = std::fs::remove_dir_all(external_index);
    }

    #[test]
    fn project_map_does_not_treat_nested_metadata_object_as_runtime_sidecar() {
        let root = test_workspace_root("project-map-nested-metadata-named-cdfi");
        std::fs::create_dir_all(root.join("src/Catalogs")).unwrap();
        std::fs::write(
            root.join("v8project.yaml"),
            concat!(
                "format: DESIGNER\n",
                "source-set:\n",
                "  - name: main\n",
                "    type: CONFIGURATION\n",
                "    path: src\n",
            ),
        )
        .unwrap();
        std::fs::write(root.join("src/Configuration.xml"), "<MetaDataObject/>").unwrap();
        std::fs::write(
            root.join("src/Catalogs/ConfigDumpInfo.xml"),
            "<MetaDataObject><Catalog/></MetaDataObject>",
        )
        .unwrap();
        std::process::Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&root)
            .status()
            .unwrap();
        std::process::Command::new("git")
            .args([
                "add",
                "v8project.yaml",
                "src/Configuration.xml",
                "src/Catalogs/ConfigDumpInfo.xml",
            ])
            .current_dir(&root)
            .status()
            .unwrap();
        let mut args = Map::new();
        args.insert("cwd".to_string(), json!(root));

        let result = UnicaApplication::new()
            .call_tool("unica.project.map", &args)
            .unwrap();

        assert!(result.ok);
        assert!(result.warnings.iter().all(|warning| {
            !warning.contains("git rm --cached") && !warning.contains("manual review")
        }));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn project_map_preserves_tracked_config_dump_info_warning_when_map_fails() {
        let root = test_workspace_root("project-map-invalid-with-tracked-cdfi");
        std::fs::write(root.join("v8project.yaml"), "source-set: [").unwrap();
        std::fs::write(root.join("ConfigDumpInfo.xml"), "<ConfigDumpInfo/>").unwrap();
        std::process::Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&root)
            .status()
            .unwrap();
        std::process::Command::new("git")
            .args(["add", "v8project.yaml", "ConfigDumpInfo.xml"])
            .current_dir(&root)
            .status()
            .unwrap();
        let mut args = Map::new();
        args.insert("cwd".to_string(), json!(root));

        let result = UnicaApplication::new()
            .call_tool("unica.project.map", &args)
            .unwrap();

        assert!(!result.ok);
        assert!(result
            .warnings
            .iter()
            .any(|warning| warning.contains("ConfigDumpInfo.xml")
                && warning.contains("git rm --cached")));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn project_map_reports_ambiguous_configuration_source_sets_without_failing() {
        let root = std::env::temp_dir().join(format!(
            "unica-project-map-ambiguous-{}",
            std::process::id()
        ));
        let workspace = root.join("workspace");
        std::fs::create_dir_all(workspace.join("app")).unwrap();
        std::fs::create_dir_all(workspace.join("tests")).unwrap();
        std::fs::write(
            workspace.join("v8project.yaml"),
            "source-set:\n  - name: app\n    type: CONFIGURATION\n    path: app\n  - name: tests\n    type: CONFIGURATION\n    path: tests\n",
        )
        .unwrap();
        let mut args = Map::new();
        args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );

        let result = UnicaApplication::new()
            .call_tool("unica.project.map", &args)
            .unwrap();

        assert!(result.ok);
        assert!(result.warnings.join("\n").contains("sourceDir"));
        let stdout = result.stdout.unwrap();
        assert!(stdout.contains(r#""name": "app""#));
        assert!(stdout.contains(r#""name": "tests""#));
        assert!(stdout.contains(r#""sourceSelectionError""#));
        assert!(stdout.contains("sourceDir"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn cf_info_reports_configuration_support_state_from_parent_configurations_bin() {
        let root = std::env::temp_dir().join(format!("unica-cf-support-{}", std::process::id()));
        let workspace = root.join("workspace");
        let src = workspace.join("src");
        let ext = src.join("Ext");
        std::fs::create_dir_all(&ext).unwrap();
        std::fs::write(
            workspace.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(
            src.join("Configuration.xml"),
            support_test_configuration_xml("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"),
        )
        .unwrap();
        std::fs::write(
            ext.join("ParentConfigurations.bin"),
            support_test_parent_configurations_bin(
                "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
                "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb",
                "cccccccc-cccc-cccc-cccc-cccccccccccc",
            ),
        )
        .unwrap();
        let mut args = Map::new();
        args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        args.insert("ConfigPath".to_string(), Value::String("src".to_string()));

        let result = UnicaApplication::new()
            .call_tool("unica.cf.info", &args)
            .unwrap();

        assert!(result.ok);
        let stdout = result.stdout.unwrap();
        assert!(stdout.contains("Поддержка:      на поддержке"));
        assert!(stdout.contains("Возможность изменения: включена"));
        assert!(stdout.contains("Объектов: на замке 1 / редактируется 1 / снято 1"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn mutating_cf_edit_blocks_locked_configuration_directory_target() {
        let root = std::env::temp_dir().join(format!("unica-cf-guard-dir-{}", std::process::id()));
        let workspace = root.join("workspace");
        let src = workspace.join("src");
        let ext = src.join("Ext");
        std::fs::create_dir_all(&ext).unwrap();
        std::fs::write(
            workspace.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        let config_path = src.join("Configuration.xml");
        std::fs::write(
            &config_path,
            support_test_configuration_xml("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"),
        )
        .unwrap();
        std::fs::write(
            ext.join("ParentConfigurations.bin"),
            support_test_parent_configurations_bin(
                "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
                "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
                "cccccccc-cccc-cccc-cccc-cccccccccccc",
            ),
        )
        .unwrap();
        let before = std::fs::read_to_string(&config_path).unwrap();
        let mut args = Map::new();
        args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        args.insert("dryRun".to_string(), Value::Bool(false));
        args.insert("ConfigPath".to_string(), Value::String("src".to_string()));
        args.insert(
            "Operation".to_string(),
            Value::String("modify-property".to_string()),
        );
        args.insert(
            "Value".to_string(),
            Value::String("Version=2.0".to_string()),
        );

        let result = UnicaApplication::new()
            .call_tool("unica.cf.edit", &args)
            .unwrap();

        assert!(!result.ok);
        assert!(result.summary.contains("support guard"));
        assert!(result.errors.join("\n").contains("на замке"));
        assert_eq!(std::fs::read_to_string(&config_path).unwrap(), before);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn cf_edit_normalizes_crlf_before_lxml_compatible_write() {
        let root = std::env::temp_dir().join(format!("unica-cf-crlf-{}", std::process::id()));
        let workspace = root.join("workspace");
        let src = workspace.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            workspace.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        let config_path = src.join("Configuration.xml");
        let crlf_config = support_test_configuration_xml("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa")
            .replace('\n', "\r\n");
        assert!(crlf_config.contains("\r\n"));
        std::fs::write(&config_path, crlf_config).unwrap();

        let mut args = Map::new();
        args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        args.insert("dryRun".to_string(), Value::Bool(false));
        args.insert("ConfigPath".to_string(), Value::String("src".to_string()));
        args.insert(
            "Operation".to_string(),
            Value::String("modify-property".to_string()),
        );
        args.insert(
            "Value".to_string(),
            Value::String("Version=2.0".to_string()),
        );
        args.insert("NoValidate".to_string(), Value::Bool(true));

        let result = UnicaApplication::new()
            .call_tool("unica.cf.edit", &args)
            .unwrap();

        assert!(result.ok, "{result:?}");
        let after = std::fs::read_to_string(&config_path).unwrap();
        assert!(after.contains("<Version>2.0</Version>"));
        assert!(!after.contains("&#13;"));

        let _ = std::fs::remove_dir_all(root);
    }

    fn cf_edit_args(
        workspace: &std::path::Path,
        operation: &str,
        value: &str,
    ) -> Map<String, Value> {
        let mut args = Map::new();
        args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        args.insert("dryRun".to_string(), Value::Bool(false));
        args.insert("ConfigPath".to_string(), Value::String("src".to_string()));
        args.insert(
            "Operation".to_string(),
            Value::String(operation.to_string()),
        );
        args.insert("Value".to_string(), Value::String(value.to_string()));
        args.insert("NoValidate".to_string(), Value::Bool(true));
        args
    }

    fn cf_edit_mutation_workspace(
        prefix: &str,
        configuration: &[u8],
    ) -> (PathBuf, PathBuf, PathBuf) {
        let root = test_workspace_root(prefix);
        let workspace = root.join("workspace");
        let src = workspace.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            workspace.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        let config_path = src.join("Configuration.xml");
        std::fs::write(&config_path, configuration).unwrap();
        (root, workspace, config_path)
    }

    fn cf_edit_configuration_bytes() -> Vec<u8> {
        let text = support_test_configuration_xml("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa");
        let mut bytes = b"\xef\xbb\xbf".to_vec();
        bytes.extend_from_slice(text.as_bytes());
        bytes
    }

    fn cf_edit_home_page_bytes() -> Vec<u8> {
        br#"<?xml version="1.0" encoding="UTF-8"?>
<HomePageWorkArea xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" version="2.20">
  <WorkingAreaTemplate>OneColumn</WorkingAreaTemplate>
</HomePageWorkArea>
"#
        .to_vec()
    }

    fn assert_no_cf_edit_stage_debris(config_path: &std::path::Path) {
        let parent = config_path.parent().unwrap();
        let debris = std::fs::read_dir(parent)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name())
            .filter(|name| name.to_string_lossy().contains(".unica-stage-"))
            .collect::<Vec<_>>();
        assert!(debris.is_empty(), "staging debris remains: {debris:?}");
    }

    #[test]
    fn cf_edit_preserves_unix_mode_0600() {
        let before = cf_edit_configuration_bytes();
        let (root, workspace, config_path) =
            cf_edit_mutation_workspace("unica-cf-edit-mode-0600", &before);
        if !set_unix_mode_for_test(&config_path, 0o600).unwrap() {
            eprintln!("[SKIPPED FIXTURE] Unix permission modes are unsupported on this host");
            std::fs::remove_dir_all(root).unwrap();
            return;
        }

        let result = UnicaApplication::new()
            .call_tool(
                "unica.cf.edit",
                &cf_edit_args(&workspace, "modify-property", "Version=2.0"),
            )
            .unwrap();

        assert!(result.ok, "{result:?}");
        assert_eq!(unix_mode_for_test(&config_path).unwrap(), Some(0o600));
        assert_ne!(std::fs::read(&config_path).unwrap(), before);
        assert_no_cf_edit_stage_debris(&config_path);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cf_edit_rejects_readonly_configuration_unchanged() {
        let before = cf_edit_configuration_bytes();
        let (root, workspace, config_path) =
            cf_edit_mutation_workspace("unica-cf-edit-readonly", &before);
        let exact_unix_mode = set_unix_mode_for_test(&config_path, 0o400).unwrap();
        if !exact_unix_mode {
            let mut permissions = std::fs::metadata(&config_path).unwrap().permissions();
            permissions.set_readonly(true);
            std::fs::set_permissions(&config_path, permissions).unwrap();
        }
        let mode_before = unix_mode_for_test(&config_path).unwrap();
        assert!(std::fs::metadata(&config_path)
            .unwrap()
            .permissions()
            .readonly());
        if exact_unix_mode {
            assert_eq!(mode_before, Some(0o400));
        } else {
            assert_eq!(mode_before, None);
        }

        let result = UnicaApplication::new()
            .call_tool(
                "unica.cf.edit",
                &cf_edit_args(&workspace, "modify-property", "Version=2.0"),
            )
            .unwrap();

        assert!(!result.ok, "{result:?}");
        assert!(result.errors.join("\n").contains("read-only"), "{result:?}");
        assert_eq!(std::fs::read(&config_path).unwrap(), before);
        assert!(std::fs::metadata(&config_path)
            .unwrap()
            .permissions()
            .readonly());
        assert_eq!(unix_mode_for_test(&config_path).unwrap(), mode_before);
        assert_no_cf_edit_stage_debris(&config_path);
        prepare_file_for_removal(&config_path).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cf_edit_rejects_symlink_configuration_without_touching_referent() {
        let before = cf_edit_configuration_bytes();
        let root = test_workspace_root("unica-cf-edit-symlink");
        let workspace = root.join("workspace");
        let src = workspace.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            workspace.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        let referent = root.join("real-Configuration.xml");
        let config_path = src.join("Configuration.xml");
        std::fs::write(&referent, &before).unwrap();
        let outcome = create_file_link_fixture_for_test(&referent, &config_path)
            .expect("unexpected file-link creation error must fail the fixture test");
        match outcome {
            FileLinkFixtureOutcome::Created => {}
            FileLinkFixtureOutcome::Unsupported => {
                eprintln!("[SKIPPED FIXTURE] file links are unsupported on this host");
                std::fs::remove_dir_all(root).unwrap();
                return;
            }
            FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => {
                eprintln!("[SKIPPED FIXTURE] Windows file-link privilege is unavailable");
                std::fs::remove_dir_all(root).unwrap();
                return;
            }
        }
        let link_before = std::fs::read_link(&config_path).unwrap();

        let result = UnicaApplication::new()
            .call_tool(
                "unica.cf.edit",
                &cf_edit_args(&workspace, "modify-property", "Version=2.0"),
            )
            .unwrap();

        assert!(!result.ok, "{result:?}");
        assert!(
            result.errors.join("\n").contains("link or reparse point"),
            "{result:?}"
        );
        assert_eq!(std::fs::read_link(&config_path).unwrap(), link_before);
        assert_eq!(std::fs::read(&referent).unwrap(), before);
        assert_no_cf_edit_stage_debris(&config_path);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cf_edit_rejects_hard_linked_configuration_unchanged() {
        let before = cf_edit_configuration_bytes();
        let (root, workspace, config_path) =
            cf_edit_mutation_workspace("unica-cf-edit-hard-link", &before);
        let alias = config_path
            .parent()
            .unwrap()
            .join("Configuration.alias.xml");
        std::fs::hard_link(&config_path, &alias).unwrap();

        let result = UnicaApplication::new()
            .call_tool(
                "unica.cf.edit",
                &cf_edit_args(&workspace, "modify-property", "Version=2.0"),
            )
            .unwrap();

        assert!(!result.ok, "{result:?}");
        assert!(
            result.errors.join("\n").contains("hard links"),
            "{result:?}"
        );
        assert_eq!(std::fs::read(&config_path).unwrap(), before);
        assert_eq!(std::fs::read(&alias).unwrap(), before);
        assert_no_cf_edit_stage_debris(&config_path);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cf_edit_equal_serialized_result_is_a_public_noop() {
        let before = cf_edit_configuration_bytes();
        let (root, workspace, config_path) =
            cf_edit_mutation_workspace("unica-cf-edit-equal-noop", &before);

        let result = UnicaApplication::new()
            .call_tool(
                "unica.cf.edit",
                &cf_edit_args(&workspace, "modify-property", "Version=1.0"),
            )
            .unwrap();

        assert!(result.ok, "{result:?}");
        assert!(result.changes.is_empty(), "{result:?}");
        assert!(result.cache.events.is_empty(), "{result:?}");
        let stdout = result.stdout.unwrap_or_default();
        assert!(
            stdout.contains("[INFO] No Configuration.xml changes"),
            "{stdout}"
        );
        assert!(!stdout.contains("[INFO] Saved:"), "{stdout}");
        assert_eq!(std::fs::read(&config_path).unwrap(), before);
        assert_no_cf_edit_stage_debris(&config_path);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn compile_transaction_and_cf_edit_share_target_lock() {
        let before = cf_edit_configuration_bytes();
        let (root, workspace, config_path) =
            cf_edit_mutation_workspace("unica-compile-cf-edit-lock", &before);
        let mut transaction = CompileTransaction::new();
        transaction
            .register_canonical_child(&config_path, "Role", "Reader")
            .expect("compile transaction must plan a registration");

        let acquired = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let acquired_in_compile = Arc::clone(&acquired);
        let release_in_compile = Arc::clone(&release);
        let compile_thread = thread::spawn(move || {
            with_publication_lock_pause(acquired_in_compile, release_in_compile, || {
                transaction.commit()
            })
        });
        acquired.wait();

        let (contended_sender, contended_receiver) = mpsc::channel();
        let workspace_in_edit = workspace.clone();
        let edit_thread = thread::spawn(move || {
            with_publication_lock_contention_signal(contended_sender, || {
                UnicaApplication::new()
                    .call_tool(
                        "unica.cf.edit",
                        &cf_edit_args(&workspace_in_edit, "modify-property", "Version=1.0"),
                    )
                    .unwrap()
            })
        });

        let contention = contended_receiver.recv_timeout(Duration::from_secs(2));
        release.wait();
        let compile_result = compile_thread
            .join()
            .expect("compile transaction thread must not panic");
        let edit_result = edit_thread.join().expect("cf-edit thread must not panic");

        contention.expect("cf-edit must contend on the shared publisher lock");
        compile_result.expect("compile transaction must commit");
        assert!(!edit_result.ok, "{edit_result:?}");
        assert!(
            edit_result
                .errors
                .join("\n")
                .contains("differs from the expected preimage"),
            "{edit_result:?}"
        );
        let after = std::fs::read(&config_path).unwrap();
        assert_ne!(after, before);
        assert!(
            String::from_utf8_lossy(&after).contains("<Role>Reader</Role>"),
            "{}",
            String::from_utf8_lossy(&after)
        );
        assert_no_cf_edit_stage_debris(&config_path);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cf_edit_definition_file_rejects_invalid_child_object_before_sidecar_writes() {
        let mut violations = Vec::new();

        for (sidecar_operation, sidecar_value, sidecar_name, child_operation, child_value, error) in [
            (
                "set-panels",
                json!({"top": ["open"]}),
                "ClientApplicationInterface.xml",
                "add-childObject",
                "SyntheticMetadata.Unknown",
                "Unknown type 'SyntheticMetadata'",
            ),
            (
                "set-panels",
                json!({"top": ["open"]}),
                "ClientApplicationInterface.xml",
                "remove-childObject",
                "SyntheticMetadata.Unknown",
                "Unknown type 'SyntheticMetadata'",
            ),
            (
                "set-home-page",
                json!({"template": "OneColumn", "left": ["CommonForm.Demo"]}),
                "HomePageWorkArea.xml",
                "add-childObject",
                "SyntheticMetadata.Unknown",
                "Unknown type 'SyntheticMetadata'",
            ),
            (
                "set-home-page",
                json!({"template": "OneColumn", "left": ["CommonForm.Demo"]}),
                "HomePageWorkArea.xml",
                "remove-childObject",
                "SyntheticMetadata.Unknown",
                "Unknown type 'SyntheticMetadata'",
            ),
            (
                "set-panels",
                json!({"top": ["open"]}),
                "ClientApplicationInterface.xml",
                "add-childObject",
                "Catalog.",
                "Invalid format 'Catalog.', expected 'Type.Name'",
            ),
            (
                "set-panels",
                json!({"top": ["open"]}),
                "ClientApplicationInterface.xml",
                "remove-childObject",
                "Catalog.",
                "Invalid format 'Catalog.', expected 'Type.Name'",
            ),
        ] {
            let (root, workspace, _) = support_test_workspace(
                &format!("unica-cf-edit-unknown-kind-atomic-{sidecar_operation}-{child_operation}"),
                String::new(),
            );
            let config_path = workspace.join("src/Configuration.xml");
            let definition_path =
                workspace.join(format!("{sidecar_operation}-{child_operation}.json"));
            std::fs::write(
                &definition_path,
                serde_json::to_string(&json!([
                    {"operation": sidecar_operation, "value": sidecar_value},
                    {"operation": child_operation, "value": child_value}
                ]))
                .unwrap(),
            )
            .unwrap();
            let config_before = std::fs::read(&config_path).unwrap();
            let definition_before = std::fs::read(&definition_path).unwrap();
            let sidecar_path = workspace.join("src/Ext").join(sidecar_name);
            let sidecar_before = if sidecar_name == "HomePageWorkArea.xml" {
                cf_edit_home_page_bytes()
            } else {
                b"sidecar content before failed batch".to_vec()
            };
            std::fs::write(&sidecar_path, &sidecar_before).unwrap();

            let mut args = Map::new();
            args.insert(
                "cwd".to_string(),
                Value::String(workspace.display().to_string()),
            );
            args.insert("dryRun".to_string(), Value::Bool(false));
            args.insert("ConfigPath".to_string(), Value::String("src".to_string()));
            args.insert(
                "DefinitionFile".to_string(),
                Value::String(definition_path.display().to_string()),
            );
            args.insert("NoValidate".to_string(), Value::Bool(true));

            let result = UnicaApplication::new()
                .call_tool("unica.cf.edit", &args)
                .unwrap();

            let case = format!("{sidecar_operation} -> {child_operation} {child_value}");
            if result.ok {
                violations.push(format!("{case}: batch unexpectedly succeeded"));
            }
            if !result.errors.join("\n").contains(error) {
                violations.push(format!("{case}: wrong error: {result:?}"));
            }
            if std::fs::read(&config_path).unwrap() != config_before {
                violations.push(format!("{case}: Configuration.xml changed"));
            }
            if std::fs::read(&definition_path).unwrap() != definition_before {
                violations.push(format!("{case}: definition file changed"));
            }
            if std::fs::read(&sidecar_path).unwrap() != sidecar_before {
                violations.push(format!(
                    "{case}: failed batch changed {}",
                    sidecar_path.display()
                ));
            }

            let _ = std::fs::remove_dir_all(root);
        }

        assert!(
            violations.is_empty(),
            "failed batches must leave all affected files byte-identical: {violations:#?}"
        );
    }

    #[test]
    fn cf_edit_definition_file_late_failure_is_atomic_for_external_files() {
        for preexisting_sidecars in [false, true] {
            let before = cf_edit_configuration_bytes();
            let case = if preexisting_sidecars {
                "existing-sidecars"
            } else {
                "new-sidecars"
            };
            let (root, workspace, config_path) =
                cf_edit_mutation_workspace(&format!("unica-cf-edit-late-failure-{case}"), &before);
            let definition_path = workspace.join("late-failure.json");
            std::fs::write(
                &definition_path,
                serde_json::to_vec(&json!([
                    {"operation": "set-panels", "value": {"top": ["open"]}},
                    {"operation": "set-home-page", "value": {"template": "OneColumn"}},
                    {"operation": "modify-property", "value": "MissingEquals"}
                ]))
                .unwrap(),
            )
            .unwrap();
            let definition_before = std::fs::read(&definition_path).unwrap();
            let ext = workspace.join("src/Ext");
            let panels = ext.join("ClientApplicationInterface.xml");
            let home_page = ext.join("HomePageWorkArea.xml");
            let existing_panels = b"panel bytes before failed batch";
            let existing_home_page = cf_edit_home_page_bytes();
            if preexisting_sidecars {
                std::fs::create_dir_all(&ext).unwrap();
                std::fs::write(&panels, existing_panels).unwrap();
                std::fs::write(&home_page, &existing_home_page).unwrap();
            }

            let mut args = Map::new();
            args.insert(
                "cwd".to_string(),
                Value::String(workspace.display().to_string()),
            );
            args.insert("dryRun".to_string(), Value::Bool(false));
            args.insert("ConfigPath".to_string(), Value::String("src".to_string()));
            args.insert(
                "DefinitionFile".to_string(),
                Value::String(definition_path.display().to_string()),
            );
            args.insert("NoValidate".to_string(), Value::Bool(true));

            let result = UnicaApplication::new()
                .call_tool("unica.cf.edit", &args)
                .unwrap();

            assert!(!result.ok, "{case}: {result:?}");
            assert!(
                result.errors.join("\n").contains("Invalid property format"),
                "{case}: {result:?}"
            );
            assert_eq!(std::fs::read(&config_path).unwrap(), before, "{case}");
            assert_eq!(
                std::fs::read(&definition_path).unwrap(),
                definition_before,
                "{case}"
            );
            if preexisting_sidecars {
                assert_eq!(std::fs::read(&panels).unwrap(), existing_panels, "{case}");
                assert_eq!(
                    std::fs::read(&home_page).unwrap(),
                    existing_home_page,
                    "{case}"
                );
            } else {
                assert!(!ext.exists(), "{case}: {} was created", ext.display());
            }

            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn cf_edit_external_files_publish_for_single_and_combined_batches() {
        for (case, operations, expected_files) in [
            (
                "panels",
                json!([
                    {"operation": "set-panels", "value": {"top": ["open"]}}
                ]),
                vec!["ClientApplicationInterface.xml"],
            ),
            (
                "home-page",
                json!([
                    {"operation": "set-home-page", "value": {"template": "OneColumn"}}
                ]),
                vec!["HomePageWorkArea.xml"],
            ),
            (
                "combined",
                json!([
                    {"operation": "set-panels", "value": {"top": ["open"]}},
                    {"operation": "set-home-page", "value": {"template": "OneColumn"}}
                ]),
                vec!["ClientApplicationInterface.xml", "HomePageWorkArea.xml"],
            ),
        ] {
            let before = cf_edit_configuration_bytes();
            let (root, workspace, config_path) = cf_edit_mutation_workspace(
                &format!("unica-cf-edit-external-success-{case}"),
                &before,
            );
            let definition_path = workspace.join(format!("{case}.json"));
            std::fs::write(&definition_path, serde_json::to_vec(&operations).unwrap()).unwrap();
            let mut args = Map::new();
            args.insert(
                "cwd".to_string(),
                Value::String(workspace.display().to_string()),
            );
            args.insert("dryRun".to_string(), Value::Bool(false));
            args.insert("ConfigPath".to_string(), Value::String("src".to_string()));
            args.insert(
                "DefinitionFile".to_string(),
                Value::String(definition_path.display().to_string()),
            );
            args.insert("NoValidate".to_string(), Value::Bool(true));

            let result = UnicaApplication::new()
                .call_tool("unica.cf.edit", &args)
                .unwrap();

            assert!(result.ok, "{case}: {result:?}");
            assert_eq!(std::fs::read(&config_path).unwrap(), before, "{case}");
            assert_eq!(
                result.changes.len(),
                expected_files.len(),
                "{case}: {result:?}"
            );
            assert_eq!(
                result.artifacts.len(),
                expected_files.len() + 1,
                "{case}: {result:?}"
            );
            for file_name in expected_files {
                let path = workspace.join("src/Ext").join(file_name);
                let bytes = std::fs::read(&path).unwrap();
                assert!(
                    bytes.starts_with(b"\xef\xbb\xbf"),
                    "{case}: {}",
                    path.display()
                );
                roxmltree::Document::parse(std::str::from_utf8(&bytes[3..]).unwrap())
                    .unwrap_or_else(|error| panic!("{case}: {}: {error}", path.display()));
                assert!(
                    result
                        .changes
                        .contains(&format!("updated {}", path.display())),
                    "{case}: {result:?}"
                );
            }

            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn cf_edit_combined_batch_replaces_external_files_and_updates_configuration() {
        let before = cf_edit_configuration_bytes();
        let (root, workspace, config_path) =
            cf_edit_mutation_workspace("unica-cf-edit-external-replace-combined", &before);
        let ext = workspace.join("src/Ext");
        std::fs::create_dir_all(&ext).unwrap();
        let panels = ext.join("ClientApplicationInterface.xml");
        let home_page = ext.join("HomePageWorkArea.xml");
        let panels_before = b"old panel bytes";
        let home_page_before = cf_edit_home_page_bytes();
        std::fs::write(&panels, panels_before).unwrap();
        std::fs::write(&home_page, &home_page_before).unwrap();
        let definition_path = workspace.join("combined-replace.json");
        std::fs::write(
            &definition_path,
            serde_json::to_vec(&json!([
                {"operation": "modify-property", "value": "Version=2.0"},
                {"operation": "set-panels", "value": {"top": ["open"]}},
                {"operation": "set-home-page", "value": {"template": "OneColumn"}}
            ]))
            .unwrap(),
        )
        .unwrap();
        let mut args = Map::new();
        args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        args.insert("dryRun".to_string(), Value::Bool(false));
        args.insert("ConfigPath".to_string(), Value::String("src".to_string()));
        args.insert(
            "DefinitionFile".to_string(),
            Value::String(definition_path.display().to_string()),
        );
        args.insert("NoValidate".to_string(), Value::Bool(true));

        let result = UnicaApplication::new()
            .call_tool("unica.cf.edit", &args)
            .unwrap();

        assert!(result.ok, "{result:?}");
        assert_eq!(result.changes.len(), 3, "{result:?}");
        let config_after = std::fs::read(&config_path).unwrap();
        assert_ne!(config_after, before);
        assert!(String::from_utf8_lossy(&config_after).contains("<Version>2.0</Version>"));
        for (path, old_bytes) in [
            (&panels, panels_before.as_slice()),
            (&home_page, home_page_before.as_slice()),
        ] {
            let bytes = std::fs::read(path).unwrap();
            assert_ne!(bytes, old_bytes, "{}", path.display());
            assert!(bytes.starts_with(b"\xef\xbb\xbf"), "{}", path.display());
            roxmltree::Document::parse(std::str::from_utf8(&bytes[3..]).unwrap()).unwrap();
        }

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cf_edit_late_config_publication_failure_leaves_external_files_absent() {
        let before = cf_edit_configuration_bytes();
        let (root, workspace, config_path) =
            cf_edit_mutation_workspace("unica-cf-edit-external-config-failure", &before);
        let alias = workspace.join("Configuration.alias.xml");
        std::fs::hard_link(&config_path, &alias).unwrap();
        let definition_path = workspace.join("config-failure.json");
        std::fs::write(
            &definition_path,
            serde_json::to_vec(&json!([
                {"operation": "set-panels", "value": {"top": ["open"]}},
                {"operation": "set-home-page", "value": {"template": "OneColumn"}},
                {"operation": "modify-property", "value": "Version=2.0"}
            ]))
            .unwrap(),
        )
        .unwrap();
        let mut args = Map::new();
        args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        args.insert("dryRun".to_string(), Value::Bool(false));
        args.insert("ConfigPath".to_string(), Value::String("src".to_string()));
        args.insert(
            "DefinitionFile".to_string(),
            Value::String(definition_path.display().to_string()),
        );
        args.insert("NoValidate".to_string(), Value::Bool(true));

        let result = UnicaApplication::new()
            .call_tool("unica.cf.edit", &args)
            .unwrap();

        assert!(!result.ok, "{result:?}");
        assert!(
            result.errors.join("\n").contains("hard links"),
            "{result:?}"
        );
        assert_eq!(std::fs::read(&config_path).unwrap(), before);
        assert_eq!(std::fs::read(&alias).unwrap(), before);
        assert!(!workspace.join("src/Ext").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cf_edit_definition_file_keeps_valid_ordered_child_object_batch() {
        let (root, workspace, _) =
            support_test_workspace("unica-cf-edit-known-kind-batch", String::new());
        let definition_path = workspace.join("ordered-batch.json");
        std::fs::write(
            &definition_path,
            serde_json::to_string(&json!([
                {"operation": "set-panels", "value": {"top": ["open"]}},
                {"operation": "remove-childObject", "value": "Catalog.Items"},
                {"operation": "add-childObject", "value": "Catalog.Items"}
            ]))
            .unwrap(),
        )
        .unwrap();
        let mut args = Map::new();
        args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        args.insert("dryRun".to_string(), Value::Bool(false));
        args.insert("ConfigPath".to_string(), Value::String("src".to_string()));
        args.insert(
            "DefinitionFile".to_string(),
            Value::String(definition_path.display().to_string()),
        );
        args.insert("NoValidate".to_string(), Value::Bool(true));

        let result = UnicaApplication::new()
            .call_tool("unica.cf.edit", &args)
            .unwrap();

        assert!(result.ok, "{result:?}");
        assert!(workspace
            .join("src/Ext/ClientApplicationInterface.xml")
            .is_file());
        assert!(
            std::fs::read_to_string(workspace.join("src/Configuration.xml"))
                .unwrap()
                .contains("<Catalog>Items</Catalog>")
        );

        let _ = std::fs::remove_dir_all(root);
    }

    fn cf_edit_issue55_config_xml(child_indent: &str) -> String {
        format!(
            concat!(
                "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n",
                "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\">\r\n",
                "\t<Configuration uuid=\"aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa\">\r\n",
                "\t\t<InternalInfo/>\r\n",
                "\t\t<Properties>\r\n",
                "\t\t\t<Name>Issue55</Name>\r\n",
                "\t\t\t<DefaultLanguage>Russian</DefaultLanguage>\r\n",
                "\t\t</Properties>\r\n",
                "\t\t<ChildObjects>\r\n",
                "{0}<Language>Russian</Language>\r\n",
                "{0}<StyleItem>НепринятаяВерсия</StyleItem>\r\n",
                "{0}<StyleItem>НеПринятыеКИсполнениюЗадачи</StyleItem>\r\n",
                "{0}<StyleItem>НерабочийПериодПроизводственногоКалендаряФон</StyleItem>\r\n",
                "{0}<CommonPicture>Минимум</CommonPicture>\r\n",
                "{0}<CommonPicture>МЧДАктивна</CommonPicture>\r\n",
                "{0}<Catalog>Валюты</Catalog>\r\n",
                "{0}<Catalog>ВариантыОтветовАнкет</Catalog>\r\n",
                "\t\t</ChildObjects>\r\n",
                "\t</Configuration>\r\n",
                "</MetaDataObject>\r\n"
            ),
            child_indent
        )
    }

    fn bot_configuration_xml(include_bot: bool) -> String {
        let children = if include_bot {
            concat!(
                "\t\t\t<Language>Русский</Language>\n",
                "\t\t\t<CommonModule>Core</CommonModule>\n",
                "\t\t\t<Bot>Assistant</Bot>\n",
                "\t\t\t<CommonAttribute>Shared</CommonAttribute>"
            )
        } else {
            concat!(
                "\t\t\t<Language>Русский</Language>\n",
                "\t\t\t<CommonModule>Core</CommonModule>\n",
                "\t\t\t<CommonAttribute>Shared</CommonAttribute>"
            )
        };
        include_str!(
            "../../../../tests/fixtures/unica_mcp_script_parity/cf-validate/Configuration.xml"
        )
        .replace("\r\n", "\n")
        .replace("version=\"2.17\"", "version=\"2.20\"")
        .replace("\t\t\t<Language>Русский</Language>", children)
    }

    fn bot_cf_workspace(prefix: &str, include_bot: bool) -> (PathBuf, PathBuf, PathBuf) {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("{prefix}-{nanos}"));
        let workspace = root.join("workspace");
        let src = workspace.join("src");
        for directory in ["Languages", "CommonModules", "Bots", "CommonAttributes"] {
            std::fs::create_dir_all(src.join(directory)).unwrap();
        }
        std::fs::write(
            workspace.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        let config_path = src.join("Configuration.xml");
        std::fs::write(
            &config_path,
            format!("\u{feff}{}", bot_configuration_xml(include_bot)),
        )
        .unwrap();
        std::fs::write(
            src.join("Languages/Русский.xml"),
            include_str!("../../../../tests/fixtures/unica_mcp_script_parity/cf-validate/Languages/Русский.xml"),
        )
        .unwrap();
        if include_bot {
            std::fs::write(src.join("Bots/Assistant.xml"), "<MetaDataObject/>").unwrap();
        }
        (root, workspace, config_path)
    }

    #[test]
    fn cf_info_and_validate_recognize_bot_in_canonical_order() {
        let (root, workspace, _config_path) = bot_cf_workspace("unica-cf-bot-read", true);
        let mut args = Map::new();
        args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        args.insert("ConfigPath".to_string(), Value::String("src".to_string()));

        let overview = UnicaApplication::new()
            .call_tool("unica.cf.info", &args)
            .unwrap();
        assert!(overview.ok, "{overview:?}");
        let overview_stdout = overview.stdout.unwrap();
        assert!(
            overview_stdout
                .lines()
                .any(|line| line.starts_with("  Боты") && line.ends_with('1')),
            "{overview_stdout}"
        );

        args.insert("Mode".to_string(), Value::String("full".to_string()));
        let full = UnicaApplication::new()
            .call_tool("unica.cf.info", &args)
            .unwrap();
        assert!(full.ok, "{full:?}");
        let full_stdout = full.stdout.unwrap();
        assert!(full_stdout.contains("Боты (Bot): 1"), "{full_stdout}");
        assert!(full_stdout.contains("    Assistant"), "{full_stdout}");

        args.remove("Mode");
        let validation = UnicaApplication::new()
            .call_tool("unica.cf.validate", &args)
            .unwrap();
        assert!(validation.ok, "{validation:?}");
        let validation_stdout = validation.stdout.unwrap_or_default();
        assert!(!validation_stdout.contains("Unknown type 'Bot'"));
        assert!(!validation_stdout.contains("out of canonical order"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn cf_edit_adds_removes_and_noops_bot_through_registry() {
        let (root, workspace, config_path) = bot_cf_workspace("unica-cf-bot-edit", false);
        let src = workspace.join("src");
        std::fs::write(src.join("Bots/Assistant.xml"), "<MetaDataObject/>").unwrap();
        let before = std::fs::read_to_string(&config_path).unwrap();

        let add = UnicaApplication::new()
            .call_tool(
                "unica.cf.edit",
                &cf_edit_args(&workspace, "add-childObject", "Bot.Assistant"),
            )
            .unwrap();
        assert!(add.ok, "{add:?}");
        let after_add = std::fs::read_to_string(&config_path).unwrap();
        assert!(
            after_add.find("<CommonModule>Core</CommonModule>").unwrap()
                < after_add.find("<Bot>Assistant</Bot>").unwrap()
        );
        assert!(
            after_add.find("<Bot>Assistant</Bot>").unwrap()
                < after_add
                    .find("<CommonAttribute>Shared</CommonAttribute>")
                    .unwrap()
        );

        let duplicate = UnicaApplication::new()
            .call_tool(
                "unica.cf.edit",
                &cf_edit_args(&workspace, "add-childObject", "Bot.Assistant"),
            )
            .unwrap();
        assert!(duplicate.ok, "{duplicate:?}");
        assert!(duplicate.changes.is_empty(), "{duplicate:?}");
        assert!(duplicate.cache.events.is_empty(), "{duplicate:?}");
        assert_eq!(std::fs::read_to_string(&config_path).unwrap(), after_add);

        let remove = UnicaApplication::new()
            .call_tool(
                "unica.cf.edit",
                &cf_edit_args(&workspace, "remove-childObject", "Bot.Assistant"),
            )
            .unwrap();
        assert!(remove.ok, "{remove:?}");
        assert_eq!(std::fs::read_to_string(&config_path).unwrap(), before);

        let missing = UnicaApplication::new()
            .call_tool(
                "unica.cf.edit",
                &cf_edit_args(&workspace, "add-childObject", "Bot.Missing"),
            )
            .unwrap();
        assert!(!missing.ok, "{missing:?}");
        let missing_errors = missing.errors.join("\n");
        assert!(missing_errors.contains("Bots/Missing.xml"), "{missing:?}");
        assert!(!missing_errors.contains("use meta-compile"), "{missing:?}");
        assert_eq!(std::fs::read_to_string(&config_path).unwrap(), before);

        let unknown = UnicaApplication::new()
            .call_tool(
                "unica.cf.edit",
                &cf_edit_args(
                    &workspace,
                    "remove-childObject",
                    "SyntheticMetadata.Unknown",
                ),
            )
            .unwrap();
        assert!(!unknown.ok, "{unknown:?}");
        assert!(
            unknown
                .errors
                .join("\n")
                .contains("Unknown type 'SyntheticMetadata'"),
            "{unknown:?}"
        );
        assert_eq!(std::fs::read_to_string(&config_path).unwrap(), before);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn cf_edit_add_child_object_does_not_escape_structural_crlf() {
        let root = std::env::temp_dir().join(format!("unica-cf-child-crlf-{}", std::process::id()));
        let workspace = root.join("workspace");
        let src = workspace.join("src");
        let catalogs = src.join("Catalogs");
        std::fs::create_dir_all(&catalogs).unwrap();
        std::fs::write(
            workspace.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        let config_path = src.join("Configuration.xml");
        let crlf_config = support_test_configuration_xml("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa")
            .replace('\n', "\r\n");
        std::fs::write(&config_path, crlf_config).unwrap();
        std::fs::write(
            catalogs.join("Extra.xml"),
            support_test_catalog_xml("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"),
        )
        .unwrap();

        let mut args = Map::new();
        args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        args.insert("dryRun".to_string(), Value::Bool(false));
        args.insert("ConfigPath".to_string(), Value::String("src".to_string()));
        args.insert(
            "Operation".to_string(),
            Value::String("add-childObject".to_string()),
        );
        args.insert(
            "Value".to_string(),
            Value::String("Catalog.Extra".to_string()),
        );
        args.insert("NoValidate".to_string(), Value::Bool(true));

        let result = UnicaApplication::new()
            .call_tool("unica.cf.edit", &args)
            .unwrap();

        assert!(result.ok, "{result:?}");
        let after_bytes = std::fs::read(&config_path).unwrap();
        let after = String::from_utf8(after_bytes.clone()).unwrap();
        assert!(after.starts_with('\u{feff}'));
        assert!(after.contains("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
        assert!(after.contains("<Catalog>Extra</Catalog>"));
        assert!(!after.contains("&#13;"), "{after}");
        assert!(
            after_bytes
                .iter()
                .enumerate()
                .filter(|(_, byte)| **byte == b'\n')
                .all(|(index, _)| index > 0 && after_bytes[index - 1] == b'\r'),
            "{after}"
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn cf_edit_remove_add_child_object_preserves_neighboring_childobjects() {
        let root =
            std::env::temp_dir().join(format!("unica-cf-issue55-roundtrip-{}", std::process::id()));
        let workspace = root.join("workspace");
        let src = workspace.join("src");
        let catalogs = src.join("Catalogs");
        std::fs::create_dir_all(&catalogs).unwrap();
        std::fs::write(
            workspace.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        let config_path = src.join("Configuration.xml");
        let before = cf_edit_issue55_config_xml("\t\t\t\t\t");
        std::fs::write(&config_path, before.as_bytes()).unwrap();
        std::fs::write(
            catalogs.join("Валюты.xml"),
            support_test_catalog_xml("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"),
        )
        .unwrap();

        let remove = UnicaApplication::new()
            .call_tool(
                "unica.cf.edit",
                &cf_edit_args(&workspace, "remove-childObject", "Catalog.Валюты"),
            )
            .unwrap();
        assert!(remove.ok, "{remove:?}");

        let add = UnicaApplication::new()
            .call_tool(
                "unica.cf.edit",
                &cf_edit_args(&workspace, "add-childObject", "Catalog.Валюты"),
            )
            .unwrap();
        assert!(add.ok, "{add:?}");

        let after = std::fs::read_to_string(&config_path).unwrap();
        assert_eq!(after, before);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn cf_edit_child_object_roundtrip_preserves_trailing_blank_lines() {
        fn trailer_after_root(text: &str) -> &str {
            let marker = "</MetaDataObject>";
            let root_end = text.rfind(marker).unwrap() + marker.len();
            &text[root_end..]
        }

        let root =
            std::env::temp_dir().join(format!("unica-cf-issue55-trailer-{}", std::process::id()));
        let workspace = root.join("workspace");
        let src = workspace.join("src");
        let catalogs = src.join("Catalogs");
        std::fs::create_dir_all(&catalogs).unwrap();
        std::fs::write(
            workspace.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        let config_path = src.join("Configuration.xml");
        let before = format!("{}\r\n\r\n", cf_edit_issue55_config_xml("\t\t\t\t\t"));
        assert_eq!(trailer_after_root(&before), "\r\n\r\n\r\n");
        std::fs::write(&config_path, before.as_bytes()).unwrap();
        std::fs::write(
            catalogs.join("Валюты.xml"),
            support_test_catalog_xml("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"),
        )
        .unwrap();

        let remove = UnicaApplication::new()
            .call_tool(
                "unica.cf.edit",
                &cf_edit_args(&workspace, "remove-childObject", "Catalog.Валюты"),
            )
            .unwrap();
        assert!(remove.ok, "{remove:?}");
        let after_remove = std::fs::read_to_string(&config_path).unwrap();
        assert_eq!(trailer_after_root(&after_remove), "\r\n\r\n\r\n");

        let add = UnicaApplication::new()
            .call_tool(
                "unica.cf.edit",
                &cf_edit_args(&workspace, "add-childObject", "Catalog.Валюты"),
            )
            .unwrap();
        assert!(add.ok, "{add:?}");

        let after = std::fs::read_to_string(&config_path).unwrap();
        assert_eq!(after, before);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn cf_edit_duplicate_add_child_object_does_not_rewrite_configuration() {
        let root =
            std::env::temp_dir().join(format!("unica-cf-issue55-noop-{}", std::process::id()));
        let workspace = root.join("workspace");
        let src = workspace.join("src");
        let catalogs = src.join("Catalogs");
        std::fs::create_dir_all(&catalogs).unwrap();
        std::fs::write(
            workspace.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        let config_path = src.join("Configuration.xml");
        let before = cf_edit_issue55_config_xml("\t\t\t\t\t");
        std::fs::write(&config_path, before.as_bytes()).unwrap();
        std::fs::write(
            catalogs.join("Валюты.xml"),
            support_test_catalog_xml("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"),
        )
        .unwrap();

        let result = UnicaApplication::new()
            .call_tool(
                "unica.cf.edit",
                &cf_edit_args(&workspace, "add-childObject", "Catalog.Валюты"),
            )
            .unwrap();

        assert!(result.ok, "{result:?}");
        assert!(result.changes.is_empty(), "{result:?}");
        assert!(result.cache.events.is_empty(), "{result:?}");
        let stdout = result.stdout.unwrap_or_default();
        assert!(
            stdout.contains("[WARN] Already exists: Catalog.Валюты"),
            "{stdout}"
        );
        assert!(!stdout.contains("[INFO] Saved:"), "{stdout}");
        assert_eq!(std::fs::read_to_string(&config_path).unwrap(), before);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn meta_info_reports_locked_vendor_support_state_through_unica_boundary() {
        let root = std::env::temp_dir().join(format!("unica-meta-support-{}", std::process::id()));
        let workspace = root.join("workspace");
        let src = workspace.join("src");
        let ext = src.join("Ext");
        let catalogs = src.join("Catalogs");
        std::fs::create_dir_all(&ext).unwrap();
        std::fs::create_dir_all(&catalogs).unwrap();
        std::fs::write(
            workspace.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(
            src.join("Configuration.xml"),
            support_test_configuration_xml("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"),
        )
        .unwrap();
        std::fs::write(
            catalogs.join("Items.xml"),
            support_test_catalog_xml("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"),
        )
        .unwrap();
        std::fs::write(
            ext.join("ParentConfigurations.bin"),
            support_test_parent_configurations_bin(
                "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
                "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb",
                "cccccccc-cccc-cccc-cccc-cccccccccccc",
            ),
        )
        .unwrap();
        let mut args = Map::new();
        args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        args.insert(
            "ObjectPath".to_string(),
            Value::String("src/Catalogs/Items.xml".to_string()),
        );

        let result = UnicaApplication::new()
            .call_tool("unica.meta.info", &args)
            .unwrap();

        assert!(result.ok);
        let stdout = result.stdout.unwrap();
        assert!(stdout.contains("Поддержка: на замке"));
        assert!(stdout.contains("cfe-*"));
        assert!(!stdout.contains("powershell.exe"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn support_edit_tool_is_mutating_native_operation() {
        let tool = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.support.edit")
            .expect("support-edit tool exists");

        assert!(tool.mutating);
        assert_eq!(tool.cache_access.writes, &["metadata_graph"]);
        match tool.handler {
            ToolHandler::NativeOperation { operation, event } => {
                assert_eq!(operation, "support-edit");
                assert_eq!(event, Some(DomainEventKind::ConfigXmlChanged));
            }
            other => {
                panic!("unica.support.edit should route through native operation, got {other:?}")
            }
        }
    }

    #[test]
    fn native_operation_descriptors_cover_all_native_tool_handlers() {
        for tool in tools() {
            let ToolHandler::NativeOperation { operation, .. } = tool.handler else {
                continue;
            };
            let descriptor = operation_descriptors::native_operation_descriptor(operation)
                .unwrap_or_else(|| panic!("{operation} has no OperationDescriptor"));
            assert_eq!(descriptor.operation, operation);
        }
    }

    #[test]
    fn native_operation_descriptors_drive_required_schema_and_path_alias_alternatives() {
        for tool in tools() {
            let ToolHandler::NativeOperation { operation, .. } = tool.handler else {
                continue;
            };
            let descriptor = operation_descriptors::native_operation_descriptor(operation).unwrap();
            let schema = input_schema_for_tool(&tool);
            let path_groups = operation_descriptors::native_path_alias_groups(operation);
            let expected_direct = descriptor
                .required_args
                .iter()
                .copied()
                .filter(|required| !path_groups.iter().any(|group| group.canonical == *required))
                .collect::<Vec<_>>();
            let required = schema["required"]
                .as_array()
                .expect("schema required is array")
                .iter()
                .map(|value| value.as_str().expect("required item is string"))
                .collect::<Vec<_>>();
            assert_eq!(required, expected_direct, "{operation} direct required");

            let expected_alias_requirements = descriptor
                .required_args
                .iter()
                .filter_map(|required| {
                    path_groups
                        .iter()
                        .find(|group| group.canonical == *required)
                        .map(|group| {
                            json!({
                                "anyOf": group
                                    .aliases
                                    .iter()
                                    .map(|alias| json!({"required": [alias]}))
                                    .collect::<Vec<_>>()
                            })
                        })
                })
                .collect::<Vec<_>>();
            let actual_alias_requirements = schema
                .get("allOf")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            assert_eq!(
                actual_alias_requirements, expected_alias_requirements,
                "{operation} path alias requirements"
            );
        }
    }

    #[test]
    fn mutating_native_descriptors_declare_write_path_policy() {
        for tool in tools() {
            if !tool.mutating {
                continue;
            }
            let ToolHandler::NativeOperation { operation, .. } = tool.handler else {
                continue;
            };
            let descriptor = operation_descriptors::native_operation_descriptor(operation).unwrap();
            assert!(
                !descriptor.write_path_args.is_empty(),
                "{operation} mutates workspace but has no descriptor write_path_args"
            );
        }
    }

    #[test]
    fn mutating_platform_xml_operations_declare_effective_format_paths() {
        use operation_descriptors::FormatGuardPolicy;

        let expected = [
            ("code-patch", &["path"][..], "DeclaredArgs"),
            (
                "cf-edit",
                &["ConfigPath", "configPath", "Path", "path"][..],
                "HandlerResolved",
            ),
            ("cf-init", &["OutputDir", "outputDir"][..], "DeclaredArgs"),
            (
                "support-edit",
                &["Path", "path", "TargetPath", "targetPath"][..],
                "DeclaredArgs",
            ),
            (
                "cfe-borrow",
                &["ExtensionPath", "ConfigPath", "extensionPath", "configPath"][..],
                "DeclaredArgs",
            ),
            (
                "cfe-init",
                &["ConfigPath", "configPath"][..],
                "DeclaredArgs",
            ),
            ("epf-init", &["OutputDir", "outputDir"][..], "DeclaredArgs"),
            ("erf-init", &["OutputDir", "outputDir"][..], "DeclaredArgs"),
            (
                "cfe-patch-method",
                &["ExtensionPath", "extensionPath"][..],
                "DeclaredArgs",
            ),
            (
                "meta-compile",
                &["OutputDir", "outputDir"][..],
                "DeclaredArgs",
            ),
            (
                "meta-edit",
                &["ObjectPath", "objectPath", "Path", "path"][..],
                "HandlerResolved",
            ),
            (
                "meta-remove",
                &["ConfigDir", "configDir"][..],
                "DeclaredArgs",
            ),
            ("help-add", &["SrcDir", "srcDir"][..], "DefaultSrcObject"),
            (
                "form-add",
                &["ObjectPath", "objectPath", "Path", "path"][..],
                "HandlerResolved",
            ),
            (
                "form-compile",
                &["OutputPath", "outputPath"][..],
                "FormCompile",
            ),
            (
                "form-edit",
                &["FormPath", "formPath", "Path", "path"][..],
                "DeclaredArgs",
            ),
            ("form-remove", &["SrcDir", "srcDir"][..], "DefaultSrcObject"),
            (
                "interface-edit",
                &["CIPath", "ciPath", "path", "Path"][..],
                "DeclaredArgs",
            ),
            (
                "subsystem-compile",
                &["OutputDir", "outputDir", "Parent", "parent"][..],
                "DeclaredArgs",
            ),
            (
                "subsystem-edit",
                &["SubsystemPath", "subsystemPath", "Path", "path"][..],
                "HandlerResolved",
            ),
            (
                "template-add",
                &["SrcDir", "srcDir"][..],
                "DefaultSrcObject",
            ),
            (
                "template-remove",
                &["SrcDir", "srcDir"][..],
                "DefaultSrcObject",
            ),
            (
                "dcs-compile",
                &["OutputPath", "outputPath"][..],
                "DeclaredArgs",
            ),
            (
                "dcs-edit",
                &["TemplatePath", "templatePath", "Path", "path"][..],
                "HandlerResolved",
            ),
            (
                "mxl-compile",
                &["OutputPath", "outputPath"][..],
                "DeclaredArgs",
            ),
            (
                "role-compile",
                &["OutputDir", "outputDir"][..],
                "DeclaredArgs",
            ),
        ];

        let mut actual_operations = tools()
            .into_iter()
            .filter(|tool| tool.mutating)
            .filter_map(|tool| {
                let ToolHandler::NativeOperation { operation, .. } = tool.handler else {
                    return None;
                };
                operation_descriptors::native_operation_descriptor(operation)?;
                Some(operation)
            })
            .collect::<Vec<_>>();
        actual_operations.sort_unstable();
        let mut expected_operations = expected
            .iter()
            .map(|(operation, _, _)| *operation)
            .collect::<Vec<_>>();
        expected_operations.sort_unstable();
        assert_eq!(
            actual_operations,
            expected_operations,
            "the handler-path contract table must cover every mutating platform-XML operation exactly once"
        );

        for (operation, aliases, policy) in expected {
            let descriptor = operation_descriptors::native_operation_descriptor(operation).unwrap();
            assert_eq!(descriptor.source_path_args, aliases, "{operation} aliases");
            assert_eq!(
                format!("{:?}", descriptor.format_path_policy),
                policy,
                "{operation} effective path policy"
            );
        }

        for tool in tools().into_iter().filter(|tool| tool.mutating) {
            let ToolHandler::NativeOperation { operation, .. } = tool.handler else {
                continue;
            };
            let descriptor = operation_descriptors::native_operation_descriptor(operation).unwrap();
            assert!(
                !descriptor.source_path_args.is_empty(),
                "{operation} must declare its platform-XML read/target path arguments"
            );
        }
        assert_eq!(
            operation_descriptors::native_operation_descriptor("mxl-compile")
                .unwrap()
                .format_guard,
            FormatGuardPolicy::ExistingDump,
            "mxl.compile must guard an existing owner while allowing standalone output"
        );
    }

    #[test]
    fn incompatible_format_blocks_before_native_handler() {
        let root = std::env::temp_dir().join(format!(
            "unica-application-format-guard-{}",
            std::process::id()
        ));
        let src = root.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        let config = src.join("Configuration.xml");
        let before = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.19"><Configuration/></MetaDataObject>"#;
        std::fs::write(&config, before).unwrap();
        let mut args = Map::new();
        args.insert("cwd".into(), Value::String(root.display().to_string()));
        args.insert(
            "ConfigPath".into(),
            Value::String(config.display().to_string()),
        );
        args.insert("dryRun".into(), Value::Bool(false));

        let result = UnicaApplication::new()
            .call_tool("unica.cf.edit", &args)
            .unwrap();

        assert!(!result.ok, "{result:?}");
        assert_eq!(
            result.diagnostics.as_ref().unwrap()["formatCompatibility"]["code"],
            "formatMigrationAvailable"
        );
        assert_eq!(std::fs::read_to_string(config).unwrap(), before);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn numeric_equivalent_noncanonical_format_warns_on_read_and_blocks_public_mutator() {
        for (index, raw) in ["2.20.0", "02.20", "2.020"].into_iter().enumerate() {
            let (root, workspace, config_path) = cf_edit_mutation_workspace(
                &format!("unica-noncanonical-format-{index}"),
                support_test_configuration_xml("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa")
                    .replacen(r#"version="2.20""#, &format!(r#"version="{raw}""#), 1)
                    .as_bytes(),
            );
            let before = std::fs::read(&config_path).unwrap();

            let mut read_args = Map::new();
            read_args.insert(
                "cwd".to_string(),
                Value::String(workspace.display().to_string()),
            );
            read_args.insert("ConfigPath".to_string(), Value::String("src".to_string()));
            let read = UnicaApplication::new()
                .call_tool("unica.cf.info", &read_args)
                .unwrap();

            assert!(
                !read.warnings.is_empty(),
                "{raw} must produce a read warning: {read:?}"
            );
            let read_diagnostic = &read.diagnostics.as_ref().unwrap()["formatCompatibility"];
            assert_eq!(read_diagnostic["code"], "formatVersionInvalid", "{raw}");
            assert_eq!(read_diagnostic["actualFormat"], raw, "{raw}");

            let mutation = UnicaApplication::new()
                .call_tool(
                    "unica.cf.edit",
                    &cf_edit_args(&workspace, "modify-property", "Version=2.0"),
                )
                .unwrap();

            assert!(!mutation.ok, "{raw}: {mutation:?}");
            let mutation_diagnostic =
                &mutation.diagnostics.as_ref().unwrap()["formatCompatibility"];
            assert_eq!(mutation_diagnostic["code"], "formatVersionInvalid", "{raw}");
            assert_eq!(mutation_diagnostic["actualFormat"], raw, "{raw}");
            assert_eq!(std::fs::read(&config_path).unwrap(), before, "{raw}");
            assert!(mutation.changes.is_empty(), "{raw}: {mutation:?}");
            assert!(mutation.artifacts.is_empty(), "{raw}: {mutation:?}");
            assert!(mutation.cache.events.is_empty(), "{raw}: {mutation:?}");
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn entity_spelled_supported_format_is_invalid_at_the_public_boundary() {
        for (index, raw) in ["2.&#50;0", "&#x32;.20", "2.2&#48;"]
            .into_iter()
            .enumerate()
        {
            let (root, workspace, config_path) = cf_edit_mutation_workspace(
                &format!("unica-entity-spelled-format-{index}"),
                support_test_configuration_xml("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa")
                    .replacen(r#"version="2.20""#, &format!(r#"version="{raw}""#), 1)
                    .as_bytes(),
            );
            let before = std::fs::read(&config_path).unwrap();

            let read_args = Map::from_iter([
                (
                    "cwd".to_string(),
                    Value::String(workspace.display().to_string()),
                ),
                ("ConfigPath".to_string(), Value::String("src".to_string())),
            ]);
            let read = UnicaApplication::new()
                .call_tool("unica.cf.info", &read_args)
                .unwrap();

            assert!(
                !read.warnings.is_empty(),
                "{raw} must produce a read warning: {read:?}"
            );
            let read_diagnostic = &read.diagnostics.as_ref().unwrap()["formatCompatibility"];
            assert_eq!(read_diagnostic["code"], "formatVersionInvalid", "{raw}");
            assert_eq!(read_diagnostic["actualFormat"], raw, "{raw}");

            let mutation = UnicaApplication::new()
                .call_tool(
                    "unica.cf.edit",
                    &cf_edit_args(&workspace, "modify-property", "Version=2.0"),
                )
                .unwrap();

            assert!(!mutation.ok, "{raw}: {mutation:?}");
            let diagnostic = &mutation.diagnostics.as_ref().unwrap()["formatCompatibility"];
            assert_eq!(diagnostic["code"], "formatVersionInvalid", "{raw}");
            assert_eq!(diagnostic["actualFormat"], raw, "{raw}");
            assert_eq!(std::fs::read(&config_path).unwrap(), before, "{raw}");
            assert!(mutation.changes.is_empty(), "{raw}: {mutation:?}");
            assert!(mutation.artifacts.is_empty(), "{raw}: {mutation:?}");
            assert!(mutation.cache.events.is_empty(), "{raw}: {mutation:?}");
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn declared_existing_mxl_output_rejects_wrong_root_before_handler() {
        let root = test_workspace_root("unica-mxl-existing-wrong-root");
        let workspace = root.join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let definition = workspace.join("mxl.json");
        std::fs::write(
            &definition,
            r#"{"columns":1,"areas":[{"name":"Area","rows":[{"cells":[{"col":1,"text":"value"}]}]}]}"#,
        )
        .unwrap();
        let output = workspace.join("Template.xml");
        std::fs::write(&output, b"<garbage/>").unwrap();
        let before = std::fs::read(&output).unwrap();
        let args = Map::from_iter([
            (
                "cwd".to_string(),
                Value::String(workspace.display().to_string()),
            ),
            (
                "JsonPath".to_string(),
                Value::String(definition.display().to_string()),
            ),
            (
                "OutputPath".to_string(),
                Value::String(output.display().to_string()),
            ),
            ("dryRun".to_string(), Value::Bool(false)),
        ]);

        let result = UnicaApplication::new()
            .call_tool("unica.mxl.compile", &args)
            .unwrap();

        assert!(!result.ok, "{result:?}");
        let diagnostic = &result.diagnostics.as_ref().unwrap()["formatCompatibility"];
        assert_eq!(diagnostic["code"], "formatVersionInvalid", "{result:?}");
        assert_eq!(diagnostic["compatibility"], "invalid", "{result:?}");
        assert_eq!(std::fs::read(&output).unwrap(), before);
        assert!(result.changes.is_empty(), "{result:?}");
        assert!(result.artifacts.is_empty(), "{result:?}");
        assert!(result.cache.events.is_empty(), "{result:?}");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn declared_existing_dcs_output_rejects_wrong_root_before_handler() {
        let root = test_workspace_root("unica-dcs-existing-wrong-root");
        let workspace = root.join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let output = workspace.join("Template.xml");
        std::fs::write(&output, b"<garbage/>").unwrap();
        let before = std::fs::read(&output).unwrap();
        let args = Map::from_iter([
            (
                "cwd".to_string(),
                Value::String(workspace.display().to_string()),
            ),
            (
                "Value".to_string(),
                Value::String(
                    json!({
                        "dataSets": [{
                            "name": "Data",
                            "query": "SELECT 1 AS Value",
                            "fields": ["Value"]
                        }]
                    })
                    .to_string(),
                ),
            ),
            (
                "OutputPath".to_string(),
                Value::String(output.display().to_string()),
            ),
            ("dryRun".to_string(), Value::Bool(false)),
        ]);

        let result = UnicaApplication::new()
            .call_tool("unica.dcs.compile", &args)
            .unwrap();

        assert!(!result.ok, "{result:?}");
        let diagnostic = &result.diagnostics.as_ref().unwrap()["formatCompatibility"];
        assert_eq!(diagnostic["code"], "formatVersionInvalid", "{result:?}");
        assert_eq!(diagnostic["compatibility"], "invalid", "{result:?}");
        assert_eq!(std::fs::read(&output).unwrap(), before);
        assert!(result.changes.is_empty(), "{result:?}");
        assert!(result.artifacts.is_empty(), "{result:?}");
        assert!(result.cache.events.is_empty(), "{result:?}");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn declared_existing_form_output_rejects_wrong_root_before_handler() {
        let root = test_workspace_root("unica-form-existing-wrong-root");
        let workspace = root.join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let definition = workspace.join("form.json");
        std::fs::write(&definition, "{}").unwrap();
        let output = workspace.join("Form.xml");
        std::fs::write(&output, b"<garbage/>").unwrap();
        let before = std::fs::read(&output).unwrap();
        let args = Map::from_iter([
            (
                "cwd".to_string(),
                Value::String(workspace.display().to_string()),
            ),
            (
                "JsonPath".to_string(),
                Value::String(definition.display().to_string()),
            ),
            (
                "OutputPath".to_string(),
                Value::String(output.display().to_string()),
            ),
            ("dryRun".to_string(), Value::Bool(false)),
        ]);

        let result = UnicaApplication::new()
            .call_tool("unica.form.compile", &args)
            .unwrap();

        assert!(!result.ok, "{result:?}");
        let diagnostic = &result.diagnostics.as_ref().unwrap()["formatCompatibility"];
        assert_eq!(diagnostic["code"], "formatVersionInvalid", "{result:?}");
        assert_eq!(diagnostic["compatibility"], "invalid", "{result:?}");
        assert_eq!(std::fs::read(&output).unwrap(), before);
        assert!(result.changes.is_empty(), "{result:?}");
        assert!(result.artifacts.is_empty(), "{result:?}");
        assert!(result.cache.events.is_empty(), "{result:?}");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn declared_form_output_with_nonstandard_suffix_still_blocks_newer_owner() {
        for (index, file_name) in ["Form.XML", "Form"].into_iter().enumerate() {
            let root =
                test_workspace_root(&format!("unica-form-existing-newer-nonstandard-{index}"));
            let workspace = root.join("workspace");
            std::fs::create_dir_all(&workspace).unwrap();
            let definition = workspace.join("form.json");
            std::fs::write(&definition, "{}").unwrap();
            let output = workspace.join(file_name);
            std::fs::write(
                &output,
                br#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.21"/>"#,
            )
            .unwrap();
            let before = std::fs::read(&output).unwrap();
            let args = Map::from_iter([
                (
                    "cwd".to_string(),
                    Value::String(workspace.display().to_string()),
                ),
                (
                    "JsonPath".to_string(),
                    Value::String(definition.display().to_string()),
                ),
                (
                    "OutputPath".to_string(),
                    Value::String(output.display().to_string()),
                ),
                ("dryRun".to_string(), Value::Bool(false)),
            ]);

            let result = UnicaApplication::new()
                .call_tool("unica.form.compile", &args)
                .unwrap();

            assert!(!result.ok, "{file_name}: {result:?}");
            let diagnostic = &result.diagnostics.as_ref().unwrap()["formatCompatibility"];
            assert_eq!(
                diagnostic["code"], "platformVersionUnsupported",
                "{file_name}: {result:?}"
            );
            assert_eq!(
                diagnostic["actualFormat"], "2.21",
                "{file_name}: {result:?}"
            );
            assert_eq!(std::fs::read(&output).unwrap(), before, "{file_name}");
            assert!(result.changes.is_empty(), "{file_name}: {result:?}");
            assert!(result.artifacts.is_empty(), "{file_name}: {result:?}");
            assert!(result.cache.events.is_empty(), "{file_name}: {result:?}");
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn ambiguous_source_set_owner_has_same_structured_failure_for_preview_and_apply() {
        let root = test_workspace_root("unica-ambiguous-source-set-owner-shape");
        let workspace = root.join("workspace");
        let src = workspace.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            workspace.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: external\n    type: EXTERNAL_DATA_PROCESSORS\n    path: src\n  - name: configuration\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        let configuration = src.join("Configuration.xml");
        let external = src.join("Demo.xml");
        std::fs::write(
            &configuration,
            br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration/></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(
            &external,
            br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.21"><ExternalDataProcessor/></MetaDataObject>"#,
        )
        .unwrap();
        let configuration_before = std::fs::read(&configuration).unwrap();
        let external_before = std::fs::read(&external).unwrap();
        let mut diagnostics = Vec::new();

        for dry_run in [true, false] {
            let args = Map::from_iter([
                (
                    "cwd".to_string(),
                    Value::String(workspace.display().to_string()),
                ),
                ("dryRun".to_string(), Value::Bool(dry_run)),
                (
                    "ObjectPath".to_string(),
                    Value::String("src/Demo.xml".to_string()),
                ),
                (
                    "Operation".to_string(),
                    Value::String("modify-property".to_string()),
                ),
                (
                    "Value".to_string(),
                    Value::String("Name=Changed".to_string()),
                ),
            ]);

            let result = UnicaApplication::new()
                .call_tool("unica.meta.edit", &args)
                .expect("ownership ambiguity must use the structured format guard result");

            assert!(!result.ok, "dryRun={dry_run}: {result:?}");
            let diagnostic = result.diagnostics.as_ref().unwrap()["formatCompatibility"].clone();
            assert_eq!(
                diagnostic["code"], "formatVersionInvalid",
                "dryRun={dry_run}: {result:?}"
            );
            assert_eq!(diagnostic["compatibility"], "invalid");
            assert!(diagnostic["actualFormat"].is_null());
            assert!(result.changes.is_empty(), "dryRun={dry_run}: {result:?}");
            assert!(result.artifacts.is_empty(), "dryRun={dry_run}: {result:?}");
            assert!(
                result.cache.events.is_empty(),
                "dryRun={dry_run}: {result:?}"
            );
            assert_eq!(std::fs::read(&configuration).unwrap(), configuration_before);
            assert_eq!(std::fs::read(&external).unwrap(), external_before);
            diagnostics.push(diagnostic);
        }

        assert_eq!(diagnostics[0], diagnostics[1]);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn meta_edit_rejects_ambiguous_or_empty_standalone_metadata_owner_before_handler() {
        let root = test_workspace_root("unica-invalid-standalone-metadata-owner");
        let workspace = root.join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let cases = [
            (
                "multiple",
                br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog/><Document/></MetaDataObject>"#
                    .as_slice(),
            ),
            (
                "empty",
                br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"></MetaDataObject>"#
                    .as_slice(),
            ),
            (
                "unknown",
                br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Garbage><Properties><Name>Editable</Name></Properties></Garbage></MetaDataObject>"#
                    .as_slice(),
            ),
        ];

        for (label, xml) in cases {
            let target = workspace.join(format!("{label}.xml"));
            std::fs::write(&target, xml).unwrap();
            let before = std::fs::read(&target).unwrap();
            let args = Map::from_iter([
                (
                    "cwd".to_string(),
                    Value::String(workspace.display().to_string()),
                ),
                ("dryRun".to_string(), Value::Bool(false)),
                (
                    "ObjectPath".to_string(),
                    Value::String(target.display().to_string()),
                ),
                (
                    "Operation".to_string(),
                    Value::String("modify-property".to_string()),
                ),
                (
                    "Value".to_string(),
                    Value::String("Name=Changed".to_string()),
                ),
            ]);

            let result = UnicaApplication::new()
                .call_tool("unica.meta.edit", &args)
                .expect("invalid standalone owner must use the structured format guard result");

            assert!(!result.ok, "{label}: {result:?}");
            let diagnostic = &result.diagnostics.as_ref().unwrap()["formatCompatibility"];
            assert_eq!(diagnostic["code"], "formatVersionInvalid", "{label}");
            assert_eq!(diagnostic["compatibility"], "invalid", "{label}");
            assert_eq!(
                diagnostic["root"],
                std::fs::canonicalize(&target)
                    .unwrap()
                    .display()
                    .to_string(),
                "{label}"
            );
            assert_eq!(std::fs::read(&target).unwrap(), before, "{label}");
            assert!(result.changes.is_empty(), "{label}: {result:?}");
            assert!(result.artifacts.is_empty(), "{label}: {result:?}");
            assert!(result.cache.events.is_empty(), "{label}: {result:?}");
        }

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cfe_patch_method_public_boundary_rejects_module_path_outside_extension() {
        let root = std::env::temp_dir().join(format!(
            "unica-cfe-patch-public-containment-{}",
            std::process::id()
        ));
        let workspace = root.join("workspace");
        let extension = workspace.join("ext");
        let outside = root.join("outside");
        let outside_module = outside.join("Ext/ObjectModule.bsl");
        std::fs::create_dir_all(&extension).unwrap();
        std::fs::write(
            workspace.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: extension\n    type: EXTENSION\n    path: ext\n",
        )
        .unwrap();
        std::fs::write(
            extension.join("Configuration.xml"),
            support_test_configuration_xml("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"),
        )
        .unwrap();
        let mut args = Map::new();
        args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        args.insert(
            "ExtensionPath".to_string(),
            Value::String("ext".to_string()),
        );
        args.insert("dryRun".to_string(), Value::Bool(false));
        args.insert(
            "ModulePath".to_string(),
            Value::String(format!("Catalog.{}.ObjectModule", outside.display())),
        );
        args.insert("MethodName".to_string(), Value::String("Run".to_string()));
        args.insert(
            "InterceptorType".to_string(),
            Value::String("Before".to_string()),
        );

        let result = UnicaApplication::new()
            .call_tool("unica.cfe.patch_method", &args)
            .unwrap();
        let escaped = outside_module.exists();
        let debug = format!("{result:?}");
        let errors = result.errors.join("\n");
        let ok = result.ok;
        let changes = result.changes.clone();
        let artifacts = result.artifacts.clone();
        let events = result.cache.events.clone();
        std::fs::remove_dir_all(root).unwrap();

        assert!(!ok, "{debug}");
        assert!(
            errors.contains("valid Unicode XML NCName and a single path component"),
            "{debug}"
        );
        assert!(!escaped, "{debug}");
        assert!(changes.is_empty(), "{debug}");
        assert!(artifacts.is_empty(), "{debug}");
        assert!(events.is_empty(), "{debug}");
    }

    #[test]
    fn cfe_patch_method_public_boundary_rejects_unborrowed_object_atomically() {
        let root = test_workspace_root("unica-cfe-patch-public-unborrowed");
        let workspace = root.join("workspace");
        let extension = workspace.join("ext");
        std::fs::create_dir_all(&extension).unwrap();
        std::fs::write(
            workspace.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: extension\n    type: EXTENSION\n    path: ext\n",
        )
        .unwrap();
        std::fs::write(
            extension.join("Configuration.xml"),
            support_test_configuration_xml("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"),
        )
        .unwrap();
        let module = extension.join("CommonModules/Orphan/Ext/Module.bsl");
        let mut args = Map::new();
        args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        args.insert(
            "ExtensionPath".to_string(),
            Value::String("ext".to_string()),
        );
        args.insert("dryRun".to_string(), Value::Bool(false));
        args.insert(
            "ModulePath".to_string(),
            Value::String("CommonModule.Orphan".to_string()),
        );
        args.insert("MethodName".to_string(), Value::String("Run".to_string()));
        args.insert(
            "InterceptorType".to_string(),
            Value::String("Before".to_string()),
        );

        let result = UnicaApplication::new()
            .call_tool("unica.cfe.patch_method", &args)
            .unwrap();
        let debug = format!("{result:?}");

        assert!(!result.ok, "{debug}");
        assert!(
            result
                .errors
                .join("\n")
                .contains("is not a borrowed extension object"),
            "{debug}"
        );
        assert!(!module.exists(), "{debug}");
        assert!(result.changes.is_empty(), "{debug}");
        assert!(result.artifacts.is_empty(), "{debug}");
        assert!(result.cache.events.is_empty(), "{debug}");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn create_only_initializers_prioritize_exact_newer_planned_xml_targets() {
        let root = test_workspace_root("unica-init-exact-newer-targets");
        let base = root.join("base/Configuration.xml");
        std::fs::create_dir_all(base.parent().unwrap()).unwrap();
        std::fs::write(
            &base,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.19"><Configuration><Properties><CompatibilityMode>Version8_3_24</CompatibilityMode><InterfaceCompatibilityMode>Taxi</InterfaceCompatibilityMode></Properties></Configuration></MetaDataObject>"#,
        )
        .unwrap();

        let mut cases = Vec::new();
        let cf_workspace = root.join("cf-default");
        let cf_target = cf_workspace.join("src/Languages/Русский.xml");
        cases.push((
            "unica.cf.init",
            cf_workspace,
            cf_target,
            Map::from_iter([("Name".to_string(), json!("ExactCf"))]),
            "src/Configuration.xml",
        ));

        let cfe_default_workspace = root.join("cfe-default");
        let cfe_default_target = cfe_default_workspace.join("src/Languages/Русский.xml");
        cases.push((
            "unica.cfe.init",
            cfe_default_workspace,
            cfe_default_target,
            Map::from_iter([
                ("Name".to_string(), json!("ExactCfeDefault")),
                ("ConfigPath".to_string(), json!(base.display().to_string())),
                ("NoRole".to_string(), json!(true)),
            ]),
            "src/Configuration.xml",
        ));

        let cfe_alias_workspace = root.join("cfe-alias");
        let cfe_alias_target = cfe_alias_workspace.join("extension/Languages/Русский.xml");
        cases.push((
            "unica.cfe.init",
            cfe_alias_workspace,
            cfe_alias_target,
            Map::from_iter([
                ("Name".to_string(), json!("ExactCfeAlias")),
                ("ConfigPath".to_string(), json!(base.display().to_string())),
                ("ExtensionPath".to_string(), json!("extension")),
                ("NoRole".to_string(), json!(true)),
            ]),
            "extension/Configuration.xml",
        ));

        for (tool, dir, artifact, output_dir) in [
            (
                "unica.epf.init",
                "epf",
                "ExactProcessor",
                "external/ExactProcessor.xml",
            ),
            (
                "unica.erf.init",
                "erf",
                "ExactReport",
                "external/ExactReport.xml",
            ),
        ] {
            let workspace = root.join(dir);
            let target = workspace
                .join("external")
                .join(artifact)
                .join("Forms/Main/Ext/Form.xml");
            cases.push((
                tool,
                workspace,
                target,
                Map::from_iter([
                    ("Name".to_string(), json!(artifact)),
                    ("OutputDir".to_string(), json!("external")),
                    ("FormName".to_string(), json!("Main")),
                ]),
                output_dir,
            ));
        }

        for (tool, workspace, target, mut args, missing_owner) in cases {
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            let newer = if target.ends_with("Form.xml") {
                br#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.21"/>"#.to_vec()
            } else {
                br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.21"><Language/></MetaDataObject>"#.to_vec()
            };
            std::fs::write(&target, &newer).unwrap();
            args.insert(
                "cwd".to_string(),
                Value::String(workspace.display().to_string()),
            );
            args.insert("dryRun".to_string(), Value::Bool(false));

            let result = UnicaApplication::new().call_tool(tool, &args).unwrap();

            assert!(!result.ok, "{tool}: {result:?}");
            let diagnostic = &result.diagnostics.as_ref().unwrap()["formatCompatibility"];
            assert_eq!(
                diagnostic["code"], "platformVersionUnsupported",
                "{tool}: {result:?}"
            );
            assert_eq!(diagnostic["actualFormat"], "2.21", "{tool}: {result:?}");
            assert_eq!(
                diagnostic["root"],
                std::fs::canonicalize(&target)
                    .unwrap()
                    .display()
                    .to_string(),
                "{tool}: {result:?}"
            );
            assert_eq!(std::fs::read(&target).unwrap(), newer, "{tool}");
            assert!(
                !workspace.join(missing_owner).exists(),
                "{tool}: {result:?}"
            );
            assert!(result.changes.is_empty(), "{tool}: {result:?}");
            assert!(result.artifacts.is_empty(), "{tool}: {result:?}");
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn create_only_initializers_ignore_unrelated_neighbor_xml() {
        let root = test_workspace_root("unica-init-unrelated-newer-neighbors");
        let cases = [
            (
                "unica.cf.init",
                "cf",
                Map::from_iter([("Name".to_string(), json!("NeighborCf"))]),
                "src/Catalogs/Unrelated.xml",
                "src/Configuration.xml",
            ),
            (
                "unica.cfe.init",
                "cfe",
                Map::from_iter([
                    ("Name".to_string(), json!("NeighborCfe")),
                    ("NoRole".to_string(), json!(true)),
                ]),
                "src/Catalogs/Unrelated.xml",
                "src/Configuration.xml",
            ),
        ];

        for (tool, label, mut args, neighbor_relative, expected_relative) in cases {
            let workspace = root.join(label);
            let neighbor = workspace.join(neighbor_relative);
            std::fs::create_dir_all(neighbor.parent().unwrap()).unwrap();
            let neighbor_bytes = br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.21"><Catalog/></MetaDataObject>"#.to_vec();
            std::fs::write(&neighbor, &neighbor_bytes).unwrap();
            args.insert(
                "cwd".to_string(),
                Value::String(workspace.display().to_string()),
            );
            args.insert("dryRun".to_string(), Value::Bool(false));

            let result = UnicaApplication::new().call_tool(tool, &args).unwrap();

            assert!(result.ok, "{tool}: {result:?}");
            assert!(workspace.join(expected_relative).is_file(), "{tool}");
            assert_eq!(std::fs::read(&neighbor).unwrap(), neighbor_bytes, "{tool}");
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn configuration_initializers_reject_external_source_set_roots_before_writes() {
        let root = test_workspace_root("unica-config-init-external-source-root");
        let cases = [
            (
                "unica.cf.init",
                "cf",
                Map::from_iter([
                    ("Name".to_string(), json!("WrongKindConfiguration")),
                    ("OutputDir".to_string(), json!("external")),
                ]),
                "Configuration",
            ),
            (
                "unica.cfe.init",
                "cfe",
                Map::from_iter([
                    ("Name".to_string(), json!("WrongKindExtension")),
                    ("OutputDir".to_string(), json!("external")),
                    ("NoRole".to_string(), json!(true)),
                ]),
                "Extension",
            ),
        ];

        for (tool, label, base_args, expected_kind) in cases {
            for nested in [false, true] {
                let workspace = root.join(format!(
                    "{label}-{}",
                    if nested { "nested" } else { "exact" }
                ));
                let external = workspace.join("external");
                std::fs::create_dir_all(&external).unwrap();
                std::fs::write(
                    workspace.join("v8project.yaml"),
                    "format: DESIGNER\nsource-set:\n  - name: external\n    type: EXTERNAL_DATA_PROCESSORS\n    path: external\n",
                )
                .unwrap();
                let mut args = base_args.clone();
                args.insert(
                    "cwd".to_string(),
                    Value::String(workspace.display().to_string()),
                );
                args.insert("dryRun".to_string(), Value::Bool(false));
                args.insert(
                    "OutputDir".to_string(),
                    json!(if nested {
                        "external/nested"
                    } else {
                        "external"
                    }),
                );

                let error = UnicaApplication::new().call_tool(tool, &args).expect_err(
                    "configuration initializer must reject an external source-set root",
                );

                assert!(error.contains("source-set `external`"), "{tool}: {error}");
                assert!(error.contains("ExternalProcessor"), "{tool}: {error}");
                assert!(error.contains(expected_kind), "{tool}: {error}");
                assert!(
                    std::fs::read_dir(&external).unwrap().next().is_none(),
                    "{tool}: wrong-kind validation must happen before writes"
                );
            }
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cfe_initializer_allows_nested_output_inside_configuration_source_set() {
        let root = test_workspace_root("unica-cfe-init-nested-configuration-source");
        let workspace = root.join("workspace");
        let src = workspace.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            workspace.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(
            src.join("Configuration.xml"),
            br#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration/></MetaDataObject>"#,
        )
        .unwrap();
        let args = Map::from_iter([
            (
                "cwd".to_string(),
                Value::String(workspace.display().to_string()),
            ),
            ("dryRun".to_string(), Value::Bool(false)),
            ("Name".to_string(), json!("NestedExtension")),
            ("OutputDir".to_string(), json!("src/extensions/MyExtension")),
            ("NoRole".to_string(), json!(true)),
        ]);

        let result = UnicaApplication::new()
            .call_tool("unica.cfe.init", &args)
            .unwrap();

        assert!(result.ok, "{result:?}");
        assert!(
            src.join("extensions/MyExtension/Configuration.xml")
                .is_file(),
            "{result:?}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn external_initializers_validate_every_existing_root_artifact_owner() {
        let root = test_workspace_root("unica-external-init-all-root-owners");
        let tool_cases = [
            (
                "unica.epf.init",
                "EXTERNAL_DATA_PROCESSORS",
                "ExternalDataProcessor",
            ),
            ("unica.erf.init", "EXTERNAL_REPORTS", "ExternalReport"),
        ];

        for (tool, source_type, artifact_tag) in tool_cases {
            for newer_config_dump_info in [false, true] {
                let label = format!(
                    "{}-{}",
                    tool.replace('.', "-"),
                    if newer_config_dump_info {
                        "mixed"
                    } else {
                        "compatible"
                    }
                );
                let workspace = root.join(label);
                let external = workspace.join("external");
                std::fs::create_dir_all(&external).unwrap();
                std::fs::write(
                    workspace.join("v8project.yaml"),
                    format!(
                        "format: DESIGNER\nsource-set:\n  - name: external\n    type: {source_type}\n    path: external\n"
                    ),
                )
                .unwrap();
                let first = external.join("First.xml");
                let second = external.join(if newer_config_dump_info {
                    "ConfigDumpInfo.xml"
                } else {
                    "Second.xml"
                });
                let owner_xml = |version: &str| {
                    format!(
                        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="{version}"><{artifact_tag}/></MetaDataObject>"#
                    )
                };
                std::fs::write(&first, owner_xml("2.20")).unwrap();
                std::fs::write(
                    &second,
                    owner_xml(if newer_config_dump_info {
                        "2.21"
                    } else {
                        "2.20"
                    }),
                )
                .unwrap();
                let first_before = std::fs::read(&first).unwrap();
                let second_before = std::fs::read(&second).unwrap();
                let args = Map::from_iter([
                    (
                        "cwd".to_string(),
                        Value::String(workspace.display().to_string()),
                    ),
                    ("dryRun".to_string(), Value::Bool(false)),
                    ("Name".to_string(), json!("Created")),
                    ("OutputDir".to_string(), json!("external")),
                ]);

                let result = UnicaApplication::new().call_tool(tool, &args).unwrap();

                if newer_config_dump_info {
                    assert!(!result.ok, "{tool}: {result:?}");
                    let diagnostic = &result.diagnostics.as_ref().unwrap()["formatCompatibility"];
                    assert_eq!(
                        diagnostic["code"], "platformVersionUnsupported",
                        "{tool}: {result:?}"
                    );
                    assert_eq!(diagnostic["actualFormat"], "2.21", "{tool}: {result:?}");
                    assert_eq!(
                        diagnostic["root"],
                        std::fs::canonicalize(&second)
                            .unwrap()
                            .display()
                            .to_string(),
                        "{tool}: {result:?}"
                    );
                    assert!(!external.join("Created.xml").exists(), "{tool}");
                    assert!(result.changes.is_empty(), "{tool}: {result:?}");
                    assert!(result.artifacts.is_empty(), "{tool}: {result:?}");
                    assert!(result.cache.events.is_empty(), "{tool}: {result:?}");
                } else {
                    assert!(result.ok, "{tool}: {result:?}");
                    assert!(external.join("Created.xml").is_file(), "{tool}");
                }
                assert_eq!(std::fs::read(&first).unwrap(), first_before, "{tool}");
                assert_eq!(std::fs::read(&second).unwrap(), second_before, "{tool}");
            }
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cf_edit_validation_dependencies_block_incompatible_home_page_file() {
        let newer_home_page = String::from_utf8(cf_edit_home_page_bytes())
            .unwrap()
            .replacen(r#"version="2.20""#, r#"version="2.21""#, 1)
            .into_bytes();
        let cases = [
            (
                "modify-newer",
                "modify-property",
                "Version=2.0",
                newer_home_page.as_slice(),
            ),
            (
                "modify-malformed",
                "modify-property",
                "Version=2.0",
                b"<not-valid-xml".as_slice(),
            ),
            (
                "panels-newer",
                "set-panels",
                r#"{"top":["open"]}"#,
                newer_home_page.as_slice(),
            ),
            (
                "panels-malformed",
                "set-panels",
                r#"{"top":["open"]}"#,
                b"<not-valid-xml".as_slice(),
            ),
        ];
        for (label, operation, value, home_page_bytes) in cases {
            let (root, workspace, config_path) = cf_edit_mutation_workspace(
                &format!("unica-cf-edit-unrelated-home-page-{label}"),
                &cf_edit_configuration_bytes(),
            );
            let home_page_path = config_path
                .parent()
                .unwrap()
                .join("Ext/HomePageWorkArea.xml");
            std::fs::create_dir_all(home_page_path.parent().unwrap()).unwrap();
            std::fs::write(&home_page_path, home_page_bytes).unwrap();
            let home_page_before = std::fs::read(&home_page_path).unwrap();
            let config_before = std::fs::read(&config_path).unwrap();
            let panels_path = config_path
                .parent()
                .unwrap()
                .join("Ext/ClientApplicationInterface.xml");

            let result = UnicaApplication::new()
                .call_tool("unica.cf.edit", &cf_edit_args(&workspace, operation, value))
                .unwrap();

            assert!(!result.ok, "{label}: {result:?}");
            let diagnostic = &result.diagnostics.as_ref().unwrap()["formatCompatibility"];
            let expected_code = if label.ends_with("newer") {
                "platformVersionUnsupported"
            } else {
                "formatVersionInvalid"
            };
            assert_eq!(diagnostic["code"], expected_code, "{label}: {result:?}");
            assert_eq!(
                std::fs::read(&home_page_path).unwrap(),
                home_page_before,
                "{label}"
            );
            assert_eq!(
                std::fs::read(&config_path).unwrap(),
                config_before,
                "{label}"
            );
            assert!(!panels_path.exists(), "{label}: {result:?}");
            assert!(result.changes.is_empty(), "{label}: {result:?}");
            assert!(result.artifacts.is_empty(), "{label}: {result:?}");
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn cf_edit_add_child_object_prioritizes_newer_existing_target_descriptor() {
        let (root, workspace, config_path) = cf_edit_mutation_workspace(
            "unica-cf-edit-add-child-newer-target",
            &cf_edit_configuration_bytes(),
        );
        let older_configuration = std::fs::read_to_string(&config_path).unwrap().replacen(
            r#"version="2.20""#,
            r#"version="2.19""#,
            1,
        );
        std::fs::write(&config_path, older_configuration).unwrap();
        let target_path = config_path.parent().unwrap().join("Catalogs/Future.xml");
        std::fs::create_dir_all(target_path.parent().unwrap()).unwrap();
        let newer_target = support_test_catalog_xml("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb")
            .replacen(r#"version="2.20""#, r#"version="2.21""#, 1)
            .replacen("<Name>Items</Name>", "<Name>Future</Name>", 1);
        std::fs::write(&target_path, newer_target).unwrap();
        let config_before = std::fs::read(&config_path).unwrap();
        let target_before = std::fs::read(&target_path).unwrap();

        let result = UnicaApplication::new()
            .call_tool(
                "unica.cf.edit",
                &cf_edit_args(&workspace, "add-childObject", "Catalog.Future"),
            )
            .unwrap();

        assert!(!result.ok, "{result:?}");
        let diagnostic = &result.diagnostics.as_ref().unwrap()["formatCompatibility"];
        assert_eq!(diagnostic["code"], "platformVersionUnsupported");
        assert_eq!(diagnostic["actualFormat"], "2.21");
        assert_eq!(diagnostic["compatibility"], "newer");
        assert_eq!(
            diagnostic["root"],
            std::fs::canonicalize(&target_path)
                .unwrap()
                .display()
                .to_string(),
            "{result:?}"
        );
        let errors = result.errors.join("\n");
        assert!(errors.contains("1С 8.5"), "{result:?}");
        assert!(!errors.contains("старше поддерживаемого"), "{result:?}");
        assert_eq!(std::fs::read(&config_path).unwrap(), config_before);
        assert_eq!(std::fs::read(&target_path).unwrap(), target_before);
        assert!(result.changes.is_empty(), "{result:?}");
        assert!(result.artifacts.is_empty(), "{result:?}");
        assert!(result.cache.events.is_empty(), "{result:?}");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn read_only_path_aliases_warn_for_older_directory_owned_inputs() {
        let root = std::env::temp_dir().join(format!(
            "unica-application-read-format-aliases-{}",
            std::process::id()
        ));
        let src = root.join("src");
        let extension = root.join("extension");
        let role_dir = src.join("Roles/Reader");
        let rights = role_dir.join("Ext/Rights.xml");
        std::fs::create_dir_all(rights.parent().unwrap()).unwrap();
        std::fs::create_dir_all(&extension).unwrap();
        std::fs::write(
            root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n  - name: extension\n    type: EXTENSION\n    path: extension\n",
        )
        .unwrap();
        let configuration = src.join("Configuration.xml");
        std::fs::write(
            &configuration,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.19"><Configuration><Properties><Name>Main</Name></Properties><ChildObjects><Role>Reader</Role></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        let extension_configuration = extension.join("Configuration.xml");
        std::fs::write(
            &extension_configuration,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.19"><Configuration><Properties><Name>Extension</Name><ConfigurationExtensionPurpose>Customization</ConfigurationExtensionPurpose></Properties><ChildObjects/></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(
            src.join("Roles/Reader.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Role><Properties><Name>Reader</Name></Properties></Role></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(
            &rights,
            r#"<Rights xmlns="http://v8.1c.ru/8.2/roles" version="2.20"/>"#,
        )
        .unwrap();
        let protected = [
            configuration.clone(),
            extension_configuration.clone(),
            rights.clone(),
        ];
        let before = protected
            .iter()
            .map(|path| std::fs::read(path).unwrap())
            .collect::<Vec<_>>();

        for (tool, alias, directory) in [
            ("unica.cf.info", "Path", src.clone()),
            ("unica.cf.validate", "path", src.clone()),
            ("unica.cfe.validate", "Path", extension.clone()),
            ("unica.role.info", "path", role_dir.clone()),
            ("unica.role.validate", "Path", role_dir.clone()),
        ] {
            let mut args = Map::new();
            args.insert("cwd".into(), Value::String(root.display().to_string()));
            args.insert(alias.into(), Value::String(directory.display().to_string()));
            args.insert("dryRun".into(), Value::Bool(true));

            let result = UnicaApplication::new().call_tool(tool, &args).unwrap();
            assert!(
                !result.warnings.is_empty(),
                "{tool} {alias} must preserve the old-format warning: {result:?}"
            );
            assert_eq!(
                result.diagnostics.as_ref().unwrap()["formatCompatibility"]["actualFormat"],
                "2.19",
                "{tool} {alias}"
            );
        }
        for (path, expected) in protected.iter().zip(before) {
            assert_eq!(std::fs::read(path).unwrap(), expected, "{}", path.display());
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn mxl_compile_blocks_write_inside_older_dump_with_structured_diagnostic() {
        let root = std::env::temp_dir().join(format!(
            "unica-application-format-guard-mxl-old-{}",
            std::process::id()
        ));
        let src = root.join("src");
        let output = src.join("Reports/Sales/Templates/Print/Ext/Template.xml");
        std::fs::create_dir_all(output.parent().unwrap()).unwrap();
        std::fs::write(
            root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(
            src.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.19"><Configuration/></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(
            &output,
            br#"<document xmlns="http://v8.1c.ru/8.2/data/spreadsheet"/>"#,
        )
        .unwrap();
        let before = std::fs::read(&output).unwrap();
        let json_path = root.join("mxl.json");
        std::fs::write(
            &json_path,
            r#"{"columns":1,"areas":[{"name":"A","rows":[{"cells":[{"col":1,"text":"x"}]}]}]}"#,
        )
        .unwrap();
        let mut args = Map::new();
        args.insert("cwd".into(), Value::String(root.display().to_string()));
        args.insert(
            "JsonPath".into(),
            Value::String(json_path.display().to_string()),
        );
        args.insert(
            "OutputPath".into(),
            Value::String(output.display().to_string()),
        );
        args.insert("dryRun".into(), Value::Bool(false));

        let result = UnicaApplication::new()
            .call_tool("unica.mxl.compile", &args)
            .unwrap();

        assert!(!result.ok, "{result:?}");
        assert_eq!(
            result.diagnostics.as_ref().unwrap()["formatCompatibility"]["actualFormat"],
            "2.19"
        );
        assert_eq!(std::fs::read(&output).unwrap(), before);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn mxl_compile_allows_new_standalone_output() {
        let root = std::env::temp_dir().join(format!(
            "unica-application-format-guard-mxl-standalone-{}",
            std::process::id()
        ));
        let src = root.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(
            src.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.19"><Configuration/></MetaDataObject>"#,
        )
        .unwrap();
        let json_path = root.join("mxl.json");
        std::fs::write(
            &json_path,
            r#"{"columns":1,"areas":[{"name":"A","rows":[{"cells":[{"col":1,"text":"x"}]}]}]}"#,
        )
        .unwrap();
        let output = root.join("generated/standalone.xml");
        let mut args = Map::new();
        args.insert("cwd".into(), Value::String(root.display().to_string()));
        args.insert(
            "JsonPath".into(),
            Value::String(json_path.display().to_string()),
        );
        args.insert(
            "OutputPath".into(),
            Value::String(output.display().to_string()),
        );
        args.insert("dryRun".into(), Value::Bool(false));

        let result = UnicaApplication::new()
            .call_tool("unica.mxl.compile", &args)
            .unwrap();

        assert!(result.ok, "{result:?}");
        assert!(output.is_file());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn source_format_sensitive_descriptors_name_source_paths() {
        for operation in ["cf-info", "form-edit", "dcs-edit", "role-info"] {
            let descriptor = operation_descriptors::native_operation_descriptor(operation).unwrap();
            assert!(
                !descriptor.source_path_args.is_empty(),
                "{operation} should declare source path args for source-set format validation"
            );
        }
    }

    #[test]
    fn native_descriptors_expose_required_adapter_arguments() {
        let required_by_operation = [
            ("meta-compile", &["JsonPath", "OutputDir"][..]),
            ("role-compile", &["JsonPath", "OutputDir"][..]),
            ("mxl-compile", &["JsonPath", "OutputPath"][..]),
        ];

        for (operation, expected_required) in required_by_operation {
            let descriptor = operation_descriptors::native_operation_descriptor(operation).unwrap();
            for expected in expected_required {
                assert!(
                    descriptor.required_args.contains(expected),
                    "{operation} descriptor should require {expected}"
                );
            }
        }
    }

    #[test]
    fn native_path_aliases_are_canonical_before_every_application_boundary() {
        use std::sync::Mutex;

        #[derive(Default)]
        struct AliasRecordingPorts {
            observed: Mutex<Vec<(&'static str, Map<String, Value>)>>,
        }

        impl AliasRecordingPorts {
            fn record(&self, stage: &'static str, args: &Map<String, Value>) {
                self.observed.lock().unwrap().push((stage, args.clone()));
            }
        }

        impl ports::ApplicationPorts for AliasRecordingPorts {
            fn discover_workspace(
                &self,
                requested_cwd: Option<PathBuf>,
            ) -> Result<WorkspaceContext, String> {
                let cwd = requested_cwd.unwrap_or_default();
                Ok(WorkspaceContext {
                    cwd: cwd.clone(),
                    workspace_root: cwd.clone(),
                    cache_root: cwd.join(".build").join("unica"),
                    workspace_epoch: 1,
                })
            }

            fn validate_tool_context(
                &self,
                _spec: ToolSpec,
                args: &Map<String, Value>,
                _dry_run: bool,
                _context: &WorkspaceContext,
            ) -> Result<(), String> {
                self.record("context", args);
                Ok(())
            }

            fn evaluate_format_guard(
                &self,
                _spec: ToolSpec,
                args: &Map<String, Value>,
                _context: &WorkspaceContext,
            ) -> Result<FormatGuardCheck, String> {
                self.record("format", args);
                Ok(FormatGuardCheck::Allow)
            }

            fn evaluate_support_guard(
                &self,
                _spec: ToolSpec,
                args: &Map<String, Value>,
                _context: &WorkspaceContext,
            ) -> Result<SupportGuardCheck, String> {
                self.record("support", args);
                Ok(SupportGuardCheck::Allow)
            }

            fn invoke_handler(
                &self,
                _spec: ToolSpec,
                args: &Map<String, Value>,
                _context: &WorkspaceContext,
                _dry_run: bool,
                _cancellation: &CancellationToken,
            ) -> Result<ports::HandlerOutcome, String> {
                self.record("handler", args);
                Ok(ports::HandlerOutcome::plain(AdapterOutcome::ok(
                    "alias recording",
                )))
            }

            fn cache_report(
                &self,
                context: &WorkspaceContext,
                _events: &[DomainEvent],
                _dry_run: bool,
                _cache_access: CacheAccess,
            ) -> Result<CacheReport, String> {
                Ok(CacheReport {
                    mode: "test".to_string(),
                    root: context.cache_root.display().to_string(),
                    workspace_epoch: context.workspace_epoch,
                    events: Vec::new(),
                    invalidated: Vec::new(),
                    refreshed: Vec::new(),
                    lazy_rebuilt: Vec::new(),
                    stale: Vec::new(),
                    fresh: Vec::new(),
                })
            }

            fn notify_invalidation(&self, _context: &WorkspaceContext, _events: &[DomainEvent]) {}
        }

        let cases = [
            (
                "unica.cf.info",
                json!({"configPath": "src", "dryRun": false}),
                &[("ConfigPath", "configPath")][..],
            ),
            (
                "unica.meta.edit",
                json!({
                    "objectPath": "src/Catalogs/Items.xml",
                    "Operation": "modify-property",
                    "dryRun": false
                }),
                &[("ObjectPath", "objectPath")][..],
            ),
            (
                "unica.form.edit",
                json!({
                    "formPath": "src/Catalogs/Items/Forms/Item/Ext/Form.xml",
                    "definition": {},
                    "dryRun": false
                }),
                &[("FormPath", "formPath")][..],
            ),
            (
                "unica.interface.edit",
                json!({
                    "ciPath": "src/Subsystems/Sales/Ext/CommandInterface.xml",
                    "dryRun": false
                }),
                &[("CIPath", "ciPath")][..],
            ),
            (
                "unica.subsystem.edit",
                json!({
                    "subsystemPath": "src/Subsystems/Sales.xml",
                    "dryRun": false
                }),
                &[("SubsystemPath", "subsystemPath")][..],
            ),
            (
                "unica.dcs.edit",
                json!({
                    "templatePath": "src/Reports/Sales/Templates/Main/Ext/Template.xml",
                    "dryRun": false
                }),
                &[("TemplatePath", "templatePath")][..],
            ),
            (
                "unica.form.compile",
                json!({
                    "OutputPath": "src/Catalogs/Items/Forms/Item/Ext/Form.xml",
                    "outputPath": "src/Catalogs/Items/Forms/Item/Ext/Form.xml",
                    "JsonPath": "form.json",
                    "jsonPath": "form.json",
                    "dryRun": false
                }),
                &[("OutputPath", "outputPath"), ("JsonPath", "jsonPath")][..],
            ),
        ];

        for (tool, raw, aliases) in cases {
            let ports = Arc::new(AliasRecordingPorts::default());
            let args = raw.as_object().unwrap();
            let result = UnicaApplication::with_ports(ports.clone())
                .call_tool(tool, args)
                .unwrap_or_else(|error| panic!("{tool} rejected a public path alias: {error}"));
            assert!(result.ok, "{tool}: {result:?}");

            let observed = ports.observed.lock().unwrap();
            assert!(
                !observed.is_empty(),
                "{tool} reached no application boundary"
            );
            for (stage, normalized) in observed.iter() {
                for (canonical, alias) in aliases {
                    assert_eq!(
                        normalized.get(*canonical),
                        args.get(*alias),
                        "{tool} {stage} did not receive canonical {canonical}"
                    );
                    assert!(
                        !normalized.contains_key(*alias),
                        "{tool} {stage} still received alias {alias}"
                    );
                }
            }
        }
    }

    #[test]
    fn native_path_alias_normalization_accepts_equal_or_empty_duplicates_but_rejects_conflicts() {
        let same = json!({
            "ConfigPath": "src",
            "configPath": "src",
            "dryRun": false
        });
        UnicaApplication::with_ports(Arc::new(FixedOutcomePorts {
            outcome: AdapterOutcome::ok("same aliases"),
            data: None,
        }))
        .call_tool("unica.cf.info", same.as_object().unwrap())
        .expect("equal path aliases must collapse to one canonical value");

        let empty_and_value = json!({
            "ConfigPath": "",
            "configPath": "src",
            "dryRun": false
        });
        UnicaApplication::with_ports(Arc::new(FixedOutcomePorts {
            outcome: AdapterOutcome::ok("empty alias ignored"),
            data: None,
        }))
        .call_tool("unica.cf.info", empty_and_value.as_object().unwrap())
        .expect("one non-empty path alias must win over empty aliases");

        let conflict = json!({
            "ConfigPath": "src-a",
            "configPath": "src-b",
            "dryRun": false
        });
        let error = UnicaApplication::with_ports(Arc::new(FixedOutcomePorts {
            outcome: AdapterOutcome::ok("must not run"),
            data: None,
        }))
        .call_tool("unica.cf.info", conflict.as_object().unwrap())
        .unwrap_err();
        assert!(error.contains("conflicting path aliases"), "{error}");
        assert!(error.contains("ConfigPath"), "{error}");
        assert!(error.contains("configPath"), "{error}");

        let form_compile_conflict = json!({
            "OutputPath": "src/Catalogs/Items/Forms/A/Ext/Form.xml",
            "outputPath": "src/Catalogs/Items/Forms/B/Ext/Form.xml",
            "JsonPath": "form.json",
            "dryRun": false
        });
        let error = UnicaApplication::with_ports(Arc::new(FixedOutcomePorts {
            outcome: AdapterOutcome::ok("form compile must not run"),
            data: None,
        }))
        .call_tool(
            "unica.form.compile",
            form_compile_conflict.as_object().unwrap(),
        )
        .unwrap_err();
        assert!(error.contains("conflicting path aliases"), "{error}");
        assert!(error.contains("OutputPath"), "{error}");
        assert!(error.contains("outputPath"), "{error}");
    }

    #[test]
    fn call_tool_cancellable_propagates_cancelled_token_to_ports() {
        use crate::domain::cancellation::CancellationToken;
        use std::sync::{Arc, Mutex};

        #[derive(Default)]
        struct CancellationRecordingPorts {
            observed_cancelled: Mutex<Option<bool>>,
        }

        impl ports::ApplicationPorts for CancellationRecordingPorts {
            fn discover_workspace(
                &self,
                requested_cwd: Option<PathBuf>,
            ) -> Result<WorkspaceContext, String> {
                let cwd = requested_cwd.unwrap_or_default();
                Ok(WorkspaceContext {
                    cwd: cwd.clone(),
                    workspace_root: cwd.clone(),
                    cache_root: cwd.join(".build").join("unica"),
                    workspace_epoch: 1,
                })
            }

            fn validate_tool_context(
                &self,
                _spec: ToolSpec,
                _args: &Map<String, Value>,
                _dry_run: bool,
                _context: &WorkspaceContext,
            ) -> Result<(), String> {
                Ok(())
            }

            fn evaluate_support_guard(
                &self,
                _spec: ToolSpec,
                _args: &Map<String, Value>,
                _context: &WorkspaceContext,
            ) -> Result<SupportGuardCheck, String> {
                Ok(SupportGuardCheck::Allow)
            }

            fn invoke_handler(
                &self,
                _spec: ToolSpec,
                _args: &Map<String, Value>,
                _context: &WorkspaceContext,
                _dry_run: bool,
                cancellation: &CancellationToken,
            ) -> Result<ports::HandlerOutcome, String> {
                *self.observed_cancelled.lock().unwrap() = Some(cancellation.is_cancelled());
                if cancellation.is_cancelled() {
                    return Ok(ports::HandlerOutcome::plain(AdapterOutcome::cancelled(
                        "recording port stopped",
                    )));
                }
                Ok(ports::HandlerOutcome::plain(AdapterOutcome::ok(
                    "recording port completed",
                )))
            }

            fn cache_report(
                &self,
                context: &WorkspaceContext,
                _events: &[DomainEvent],
                _dry_run: bool,
                _cache_access: CacheAccess,
            ) -> Result<CacheReport, String> {
                Ok(CacheReport {
                    mode: "read".to_string(),
                    root: context.cache_root.display().to_string(),
                    workspace_epoch: context.workspace_epoch,
                    events: Vec::new(),
                    invalidated: Vec::new(),
                    refreshed: Vec::new(),
                    lazy_rebuilt: Vec::new(),
                    stale: Vec::new(),
                    fresh: Vec::new(),
                })
            }

            fn notify_invalidation(&self, _context: &WorkspaceContext, _events: &[DomainEvent]) {}
        }

        let ports = Arc::new(CancellationRecordingPorts::default());
        let app = UnicaApplication::with_ports(ports.clone());
        let token = CancellationToken::new();
        token.cancel();

        let result = app
            .call_tool_cancellable("unica.code.search", &Map::new(), token)
            .unwrap();

        assert_eq!(*ports.observed_cancelled.lock().unwrap(), Some(true));
        assert!(result.errors[0].starts_with("cancelled:"));
    }

    #[test]
    fn call_tool_cancellable_default_ports_uses_stable_cancellation_prefix() {
        let token = CancellationToken::new();
        token.cancel();

        let result = UnicaApplication::new()
            .call_tool_cancellable("unica.project.status", &Map::new(), token)
            .unwrap();

        assert!(!result.ok);
        assert!(result.errors[0].starts_with("cancelled:"));
    }

    #[test]
    fn application_dispatches_workspace_cache_and_handlers_through_ports() {
        use std::sync::{Arc, Mutex};

        #[derive(Default)]
        struct RecordingPorts {
            discovered: Mutex<Vec<PathBuf>>,
            invoked: Mutex<Vec<&'static str>>,
            reported: Mutex<Vec<&'static str>>,
            invalidated: Mutex<Vec<String>>,
        }

        impl ports::ApplicationPorts for RecordingPorts {
            fn discover_workspace(
                &self,
                requested_cwd: Option<PathBuf>,
            ) -> Result<WorkspaceContext, String> {
                let cwd = requested_cwd.unwrap_or_default();
                self.discovered.lock().unwrap().push(cwd.clone());
                Ok(WorkspaceContext {
                    cwd: cwd.clone(),
                    workspace_root: cwd.clone(),
                    cache_root: cwd.join(".build").join("unica"),
                    workspace_epoch: 1,
                })
            }

            fn validate_tool_context(
                &self,
                _spec: ToolSpec,
                _args: &Map<String, Value>,
                _dry_run: bool,
                _context: &WorkspaceContext,
            ) -> Result<(), String> {
                Ok(())
            }

            fn evaluate_support_guard(
                &self,
                _spec: ToolSpec,
                _args: &Map<String, Value>,
                _context: &WorkspaceContext,
            ) -> Result<SupportGuardCheck, String> {
                Ok(SupportGuardCheck::Allow)
            }

            fn invoke_handler(
                &self,
                spec: ToolSpec,
                _args: &Map<String, Value>,
                _context: &WorkspaceContext,
                _dry_run: bool,
                _cancellation: &CancellationToken,
            ) -> Result<ports::HandlerOutcome, String> {
                self.invoked.lock().unwrap().push(spec.name);
                Ok(ports::HandlerOutcome::plain(AdapterOutcome::ok(
                    "fake port outcome",
                )))
            }

            fn cache_report(
                &self,
                context: &WorkspaceContext,
                events: &[DomainEvent],
                dry_run: bool,
                cache_access: CacheAccess,
            ) -> Result<CacheReport, String> {
                self.reported.lock().unwrap().extend(cache_access.writes);
                Ok(CacheReport {
                    mode: if dry_run { "dry-run" } else { "write" }.to_string(),
                    root: context.cache_root.display().to_string(),
                    workspace_epoch: context.workspace_epoch,
                    events: events
                        .iter()
                        .map(|event| format!("{:?}", event.kind))
                        .collect(),
                    invalidated: cache_access
                        .writes
                        .iter()
                        .map(|name| (*name).to_string())
                        .collect(),
                    refreshed: Vec::new(),
                    lazy_rebuilt: Vec::new(),
                    stale: Vec::new(),
                    fresh: Vec::new(),
                })
            }

            fn notify_invalidation(&self, _context: &WorkspaceContext, events: &[DomainEvent]) {
                self.invalidated
                    .lock()
                    .unwrap()
                    .extend(events.iter().map(|event| format!("{:?}", event.kind)));
            }
        }

        let root = std::env::temp_dir().join(format!("unica-ports-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let mut args = Map::new();
        args.insert("cwd".to_string(), Value::String(root.display().to_string()));
        let ports = Arc::new(RecordingPorts::default());
        let app = UnicaApplication::with_ports(ports.clone());

        let result = app.call_tool("unica.build.load", &args).unwrap();

        assert!(result.ok);
        assert_eq!(
            ports.invoked.lock().unwrap().as_slice(),
            ["unica.build.load"]
        );
        assert_eq!(
            ports.reported.lock().unwrap().as_slice(),
            ["workspace_graph", "metadata_graph"]
        );
        assert!(ports.invalidated.lock().unwrap().is_empty());
        assert_eq!(ports.discovered.lock().unwrap().len(), 1);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn support_edit_dry_run_does_not_change_parent_configurations() {
        let (root, workspace, bin_path) = support_test_workspace(
            "unica-support-edit-dry-run",
            support_test_parent_configurations_bin(
                "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
                "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb",
                "cccccccc-cccc-cccc-cccc-cccccccccccc",
            ),
        );
        let before = std::fs::read_to_string(&bin_path).unwrap();
        let mut args = Map::new();
        args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        args.insert("Path".to_string(), Value::String("src".to_string()));
        args.insert("Capability".to_string(), Value::String("off".to_string()));

        let result = UnicaApplication::new()
            .call_tool("unica.support.edit", &args)
            .unwrap();

        assert!(result.ok);
        assert!(result.summary.contains("dry run"));
        assert_eq!(std::fs::read_to_string(&bin_path).unwrap(), before);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn support_edit_capability_on_enables_global_editing() {
        let bin = support_test_parent_configurations_bin(
            "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
            "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb",
            "cccccccc-cccc-cccc-cccc-cccccccccccc",
        )
        .replace("{6,0,", "{6,1,");
        let (root, workspace, _bin_path) =
            support_test_workspace("unica-support-edit-capability-on", bin);
        let mut args = Map::new();
        args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        args.insert("dryRun".to_string(), Value::Bool(false));
        args.insert("Path".to_string(), Value::String("src".to_string()));
        args.insert("Capability".to_string(), Value::String("on".to_string()));

        let result = UnicaApplication::new()
            .call_tool("unica.support.edit", &args)
            .unwrap();

        assert!(result.ok, "{:?}", result.errors);
        assert!(result.summary.contains("Возможность изменения"));
        let mut info_args = Map::new();
        info_args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        info_args.insert("ConfigPath".to_string(), Value::String("src".to_string()));
        let info = UnicaApplication::new()
            .call_tool("unica.cf.info", &info_args)
            .unwrap();
        assert!(info
            .stdout
            .unwrap()
            .contains("Возможность изменения: включена"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn support_edit_capability_off_disables_global_editing_and_blocks_set() {
        let (root, workspace, bin_path) = support_test_workspace(
            "unica-support-edit-capability-off",
            support_test_parent_configurations_bin(
                "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
                "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb",
                "cccccccc-cccc-cccc-cccc-cccccccccccc",
            ),
        );
        let mut args = Map::new();
        args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        args.insert("dryRun".to_string(), Value::Bool(false));
        args.insert("Path".to_string(), Value::String("src".to_string()));
        args.insert("Capability".to_string(), Value::String("off".to_string()));

        let result = UnicaApplication::new()
            .call_tool("unica.support.edit", &args)
            .unwrap();

        assert!(result.ok, "{:?}", result.errors);
        assert!(result.summary.contains("ВЫКЛЮЧЕНА"));
        let bin_text = std::fs::read_to_string(&bin_path).unwrap();
        assert!(bin_text.contains("{6,1,"));
        assert!(bin_text.contains(
            "dddddddd-dddd-dddd-dddd-dddddddddddd,0,eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee"
        ));
        assert!(bin_text.contains(",0,0,aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"));
        assert!(bin_text.contains(",0,0,bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"));
        assert!(bin_text.contains(",0,0,cccccccc-cccc-cccc-cccc-cccccccccccc"));

        let mut info_args = Map::new();
        info_args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        info_args.insert("ConfigPath".to_string(), Value::String("src".to_string()));
        let info = UnicaApplication::new()
            .call_tool("unica.cf.info", &info_args)
            .unwrap();
        assert!(info
            .stdout
            .unwrap()
            .contains("Возможность изменения: выключена"));

        let mut set_args = Map::new();
        set_args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        set_args.insert("dryRun".to_string(), Value::Bool(false));
        set_args.insert(
            "Path".to_string(),
            Value::String("src/Catalogs/Items.xml".to_string()),
        );
        set_args.insert("Set".to_string(), Value::String("editable".to_string()));
        let set_result = UnicaApplication::new()
            .call_tool("unica.support.edit", &set_args)
            .unwrap();
        assert!(!set_result.ok);
        assert!(set_result.errors.join("\n").contains("Capability=on"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn support_edit_set_editable_updates_object_rule_and_meta_info() {
        let (root, workspace, _bin_path) = support_test_workspace(
            "unica-support-edit-set-editable",
            support_test_parent_configurations_bin(
                "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
                "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb",
                "cccccccc-cccc-cccc-cccc-cccccccccccc",
            ),
        );
        let mut args = Map::new();
        args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        args.insert("dryRun".to_string(), Value::Bool(false));
        args.insert(
            "Path".to_string(),
            Value::String("src/Catalogs/Items.xml".to_string()),
        );
        args.insert("Set".to_string(), Value::String("editable".to_string()));

        let result = UnicaApplication::new()
            .call_tool("unica.support.edit", &args)
            .unwrap();

        assert!(result.ok, "{:?}", result.errors);
        assert!(result.summary.contains("редактируется"));
        let mut info_args = Map::new();
        info_args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        info_args.insert(
            "ObjectPath".to_string(),
            Value::String("src/Catalogs/Items.xml".to_string()),
        );
        let info = UnicaApplication::new()
            .call_tool("unica.meta.info", &info_args)
            .unwrap();
        assert!(info
            .stdout
            .unwrap()
            .contains("редактируется с сохранением поддержки"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn support_edit_set_requires_global_capability_on() {
        let bin = support_test_parent_configurations_bin(
            "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
            "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb",
            "cccccccc-cccc-cccc-cccc-cccccccccccc",
        )
        .replace("{6,0,", "{6,1,");
        let (root, workspace, bin_path) =
            support_test_workspace("unica-support-edit-set-capability-off", bin);
        let before = std::fs::read_to_string(&bin_path).unwrap();
        let mut args = Map::new();
        args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        args.insert("dryRun".to_string(), Value::Bool(false));
        args.insert(
            "Path".to_string(),
            Value::String("src/Catalogs/Items.xml".to_string()),
        );
        args.insert("Set".to_string(), Value::String("editable".to_string()));

        let result = UnicaApplication::new()
            .call_tool("unica.support.edit", &args)
            .unwrap();

        assert!(!result.ok);
        assert!(result.errors.join("\n").contains("Capability=on"));
        assert_eq!(std::fs::read_to_string(&bin_path).unwrap(), before);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn support_edit_missing_parent_configurations_is_safe_noop() {
        let root =
            std::env::temp_dir().join(format!("unica-support-edit-no-bin-{}", std::process::id()));
        let workspace = root.join("workspace");
        let src = workspace.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            workspace.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(
            src.join("Configuration.xml"),
            support_test_configuration_xml("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"),
        )
        .unwrap();
        let mut args = Map::new();
        args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        args.insert("dryRun".to_string(), Value::Bool(false));
        args.insert("Path".to_string(), Value::String("src".to_string()));
        args.insert("Capability".to_string(), Value::String("on".to_string()));

        let result = UnicaApplication::new()
            .call_tool("unica.support.edit", &args)
            .unwrap();

        assert!(result.ok);
        assert!(result.changes.is_empty());
        assert!(result.summary.contains("не на поддержке"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn support_edit_set_editable_allows_follow_up_meta_edit() {
        let (root, workspace, _bin_path) = support_test_workspace(
            "unica-support-edit-unblocks-guard",
            support_test_parent_configurations_bin(
                "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
                "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb",
                "cccccccc-cccc-cccc-cccc-cccccccccccc",
            ),
        );
        let mut support_args = Map::new();
        support_args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        support_args.insert("dryRun".to_string(), Value::Bool(false));
        support_args.insert(
            "Path".to_string(),
            Value::String("src/Catalogs/Items.xml".to_string()),
        );
        support_args.insert("Set".to_string(), Value::String("editable".to_string()));
        let support_result = UnicaApplication::new()
            .call_tool("unica.support.edit", &support_args)
            .unwrap();
        assert!(support_result.ok, "{:?}", support_result.errors);

        let object_path = workspace.join("src").join("Catalogs").join("Items.xml");
        let before = std::fs::read_to_string(&object_path).unwrap();
        let mut edit_args = Map::new();
        edit_args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        edit_args.insert("dryRun".to_string(), Value::Bool(false));
        edit_args.insert(
            "ObjectPath".to_string(),
            Value::String("src/Catalogs/Items.xml".to_string()),
        );
        edit_args.insert(
            "Operation".to_string(),
            Value::String("modify-property".to_string()),
        );
        edit_args.insert(
            "Value".to_string(),
            Value::String("Name=Changed".to_string()),
        );

        let edit_result = UnicaApplication::new()
            .call_tool("unica.meta.edit", &edit_args)
            .unwrap();

        assert!(edit_result.ok, "{:?}", edit_result.errors);
        assert_ne!(std::fs::read_to_string(&object_path).unwrap(), before);
        assert!(std::fs::read_to_string(&object_path)
            .unwrap()
            .contains("<Name>Changed</Name>"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn meta_compile_dry_run_reports_exact_registration_diff_without_writes() {
        let root = temp_scaffolded_configuration_workspace("unica-meta-compile-dry-run-plan");
        let workspace = root.join("workspace");
        let src = workspace.join("src");
        let config_path = src.join("Configuration.xml");
        let config_before = write_scaffolded_configuration_fixture(
            &config_path,
            &["<Language>Русский</Language>"],
            "<!-- registrar-tail -->\n\n",
        );
        let json_path = workspace.join("common-module.json");
        std::fs::write(
            &json_path,
            r#"{
  "type": "CommonModule",
  "name": "SampleService",
  "synonym": "Sample service"
}"#,
        )
        .unwrap();

        let mut args = Map::new();
        args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        args.insert("dryRun".to_string(), Value::Bool(true));
        args.insert(
            "JsonPath".to_string(),
            Value::String(json_path.display().to_string()),
        );
        args.insert("OutputDir".to_string(), Value::String("src".to_string()));

        let result = UnicaApplication::new()
            .call_tool("unica.meta.compile", &args)
            .unwrap();

        assert!(result.ok, "{:?}", result.errors);
        assert!(result.summary.contains("dry run"), "{}", result.summary);
        assert!(result
            .changes
            .iter()
            .any(|change| change.contains("would create") && change.contains("SampleService.xml")));
        assert!(result
            .changes
            .iter()
            .any(|change| change.contains("would update") && change.contains("Configuration.xml")));
        let preview = result.stdout.unwrap_or_default();
        assert!(preview.contains("@@ bytes"), "{preview}");
        assert!(
            preview.contains("<CommonModule>SampleService</CommonModule>\\r\\n"),
            "{preview}"
        );
        assert!(result.artifacts.is_empty());
        assert_eq!(result.cache.mode, "dry-run");
        assert!(result.cache.events.contains(&"MetadataChanged".to_string()));
        assert_eq!(std::fs::read(&config_path).unwrap(), config_before);
        assert!(!src.join("CommonModules/SampleService.xml").exists());

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn repeated_meta_compile_is_a_byte_for_byte_noop() {
        let root = temp_meta_compile_workspace("unica-meta-compile-repeat-noop");
        let workspace = root.join("workspace");
        let src = workspace.join("src");
        let json_path = workspace.join("common-module.json");
        std::fs::write(
            &json_path,
            r#"{
  "type": "CommonModule",
  "name": "SampleService",
  "synonym": "Sample service"
}"#,
        )
        .unwrap();
        let first = call_meta_compile(&workspace, &json_path);
        assert!(first.ok, "{:?}", first.errors);
        let metadata_path = src.join("CommonModules/SampleService.xml");
        let module_path = src.join("CommonModules/SampleService/Ext/Module.bsl");
        let config_path = src.join("Configuration.xml");
        let metadata_before = std::fs::read(&metadata_path).unwrap();
        let module_before = std::fs::read(&module_path).unwrap();
        let config_before = std::fs::read(&config_path).unwrap();
        std::fs::write(
            &json_path,
            r#"{
  "type": "CommonModule",
  "name": "SampleService",
  "synonym": "A changed definition must not overwrite the object"
}"#,
        )
        .unwrap();

        let repeated = call_meta_compile(&workspace, &json_path);

        assert!(repeated.ok, "{:?}", repeated.errors);
        assert!(repeated.changes.is_empty(), "{:?}", repeated.changes);
        assert!(repeated.artifacts.is_empty(), "{:?}", repeated.artifacts);
        assert_eq!(std::fs::read(&metadata_path).unwrap(), metadata_before);
        assert_eq!(std::fs::read(&module_path).unwrap(), module_before);
        assert_eq!(std::fs::read(&config_path).unwrap(), config_before);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn meta_compile_preserves_single_configuration_bom() {
        let root = temp_meta_compile_workspace("unica-meta-compile-single-bom");
        let workspace = root.join("workspace");
        let src = workspace.join("src");
        let config_path = src.join("Configuration.xml");
        std::fs::write(
            &config_path,
            format!(
                "\u{feff}{}",
                support_test_configuration_xml("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa")
            ),
        )
        .unwrap();
        let json_path = workspace.join("report.json");
        std::fs::write(
            &json_path,
            r#"{
  "type": "Report",
  "name": "MetaCompileBomReport",
  "synonym": "MetaCompileBomReport"
}"#,
        )
        .unwrap();

        let result = call_meta_compile(&workspace, &json_path);

        assert!(result.ok, "{:?}", result.errors);
        let config_bytes = std::fs::read(&config_path).unwrap();
        assert_eq!(leading_utf8_bom_count(&config_bytes), 1);
        let config_text = String::from_utf8_lossy(&config_bytes).to_string();
        assert!(config_text.contains("<Report>MetaCompileBomReport</Report>"));
        roxmltree::Document::parse(config_text.trim_start_matches('\u{feff}')).unwrap();
        let generated =
            std::fs::read_to_string(src.join("Reports/MetaCompileBomReport.xml")).unwrap();
        assert!(generated.contains(r#"version="2.20""#), "{generated}");
        assert!(!generated.contains(r#"version="2.17""#), "{generated}");

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn meta_compile_keeps_bot_outside_its_narrow_capability_gate() {
        let root = temp_meta_compile_workspace("unica-meta-compile-bot-unsupported");
        let workspace = root.join("workspace");
        let json_path = workspace.join("bot.json");
        std::fs::write(
            &json_path,
            r#"{
  "type": "Bot",
  "name": "Assistant",
  "synonym": "Assistant"
}"#,
        )
        .unwrap();

        let result = call_meta_compile(&workspace, &json_path);

        assert!(!result.ok, "{result:?}");
        assert!(
            result.errors.join("\n").contains("Unsupported type: Bot"),
            "{result:?}"
        );
        assert!(!workspace.join("src/Bots").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn meta_compile_preserves_configuration_child_objects_formatting() {
        let root = temp_scaffolded_configuration_workspace("unica-meta-compile-child-format");
        let workspace = root.join("workspace");
        let src = workspace.join("src");
        let config_path = src.join("Configuration.xml");
        write_scaffolded_configuration_fixture(
            &config_path,
            &["<Language>Русский</Language>", "<Catalog>Items</Catalog>"],
            "",
        );
        let json_path = workspace.join("report.json");
        std::fs::write(
            &json_path,
            r#"{
  "type": "Report",
  "name": "MetaCompileFormatReport",
  "synonym": "MetaCompileFormatReport"
}"#,
        )
        .unwrap();

        let result = call_meta_compile(&workspace, &json_path);

        assert!(result.ok, "{:?}", result.errors);
        let config_text =
            String::from_utf8_lossy(&std::fs::read(&config_path).unwrap()).to_string();
        assert!(config_text.contains(concat!(
            "\r\n\t\t\t<Catalog>Items</Catalog>\r\n",
            "\t\t\t<Report>MetaCompileFormatReport</Report>\r\n",
            "\t\t</ChildObjects>"
        )));
        assert!(!config_text.contains("\t\t\t\t\t<Report>MetaCompileFormatReport</Report>"));
        assert!(
            !config_text.contains("<Report>MetaCompileFormatReport</Report>\n\t\t</ChildObjects>")
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn meta_compile_catalog_comment_emits_single_object_comment() {
        let root = temp_meta_compile_workspace("unica-meta-compile-catalog-comment");
        let workspace = root.join("workspace");
        let src = workspace.join("src");
        let fixtures = workspace.join("fixtures");
        std::fs::create_dir_all(&fixtures).unwrap();
        let json_path = fixtures.join("catalog-comment.json");
        std::fs::write(
            &json_path,
            r#"{
  "type": "Catalog",
  "name": "Issue67Catalog",
  "synonym": "Issue67Catalog",
  "comment": "TEST-COMMENT"
}"#,
        )
        .unwrap();

        let result = call_meta_compile(&workspace, &json_path);

        assert!(result.ok, "{:?}", result.stderr);
        let xml_path = src.join("Catalogs").join("Issue67Catalog.xml");
        assert!(xml_path.is_file());
        let xml = std::fs::read_to_string(&xml_path).unwrap();
        assert_eq!(xml.matches("<Comment>TEST-COMMENT</Comment>").count(), 1);
        let doc = roxmltree::Document::parse(xml.trim_start_matches('\u{feff}')).unwrap();
        let catalog = doc
            .root_element()
            .children()
            .find(|node| node.is_element() && node.tag_name().name() == "Catalog")
            .unwrap();
        let properties = catalog
            .children()
            .find(|node| node.is_element() && node.tag_name().name() == "Properties")
            .unwrap();
        let comments = properties
            .children()
            .filter(|node| node.is_element() && node.tag_name().name() == "Comment")
            .collect::<Vec<_>>();
        assert_eq!(comments.len(), 1, "{xml}");
        assert_eq!(comments[0].text(), Some("TEST-COMMENT"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn template_add_preserves_single_object_bom() {
        let root = temp_meta_compile_workspace("unica-template-add-single-bom");
        let workspace = root.join("workspace");
        let json_path = workspace.join("report.json");
        std::fs::write(
            &json_path,
            r#"{
  "type": "Report",
  "name": "TemplateBomReport",
  "synonym": "TemplateBomReport"
}"#,
        )
        .unwrap();
        let result = call_meta_compile(&workspace, &json_path);
        assert!(result.ok, "{:?}", result.errors);

        let report_path = workspace
            .join("src")
            .join("Reports")
            .join("TemplateBomReport.xml");
        let report_bytes = std::fs::read(&report_path).unwrap();
        assert_eq!(leading_utf8_bom_count(&report_bytes), 1);

        let mut args = Map::new();
        args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        args.insert("dryRun".to_string(), Value::Bool(false));
        args.insert(
            "ObjectName".to_string(),
            Value::String("TemplateBomReport".to_string()),
        );
        args.insert(
            "TemplateName".to_string(),
            Value::String("ОсновнаяСхемаКомпоновкиДанных".to_string()),
        );
        args.insert(
            "TemplateType".to_string(),
            Value::String("DataCompositionSchema".to_string()),
        );
        args.insert(
            "SrcDir".to_string(),
            Value::String("src/Reports".to_string()),
        );

        let template_result = UnicaApplication::new()
            .call_tool("unica.template.add", &args)
            .unwrap();

        assert!(template_result.ok, "{:?}", template_result.errors);
        let report_bytes = std::fs::read(&report_path).unwrap();
        assert_eq!(leading_utf8_bom_count(&report_bytes), 1);
        assert!(String::from_utf8_lossy(&report_bytes)
            .contains("<Template>ОсновнаяСхемаКомпоновкиДанных</Template>"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn template_add_repairs_repeated_object_bom() {
        let root = temp_meta_compile_workspace("unica-template-add-repeated-bom");
        let workspace = root.join("workspace");
        let json_path = workspace.join("report.json");
        std::fs::write(
            &json_path,
            r#"{
  "type": "Report",
  "name": "TemplateRepeatedBomReport",
  "synonym": "TemplateRepeatedBomReport"
}"#,
        )
        .unwrap();
        let result = call_meta_compile(&workspace, &json_path);
        assert!(result.ok, "{:?}", result.errors);

        let report_path = workspace
            .join("src")
            .join("Reports")
            .join("TemplateRepeatedBomReport.xml");
        let report_bytes = std::fs::read(&report_path).unwrap();
        assert_eq!(leading_utf8_bom_count(&report_bytes), 1);

        let mut damaged = b"\xef\xbb\xbf".to_vec();
        damaged.extend_from_slice(&report_bytes);
        std::fs::write(&report_path, damaged).unwrap();
        let report_bytes = std::fs::read(&report_path).unwrap();
        assert_eq!(leading_utf8_bom_count(&report_bytes), 2);

        let mut args = Map::new();
        args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        args.insert("dryRun".to_string(), Value::Bool(false));
        args.insert(
            "ObjectName".to_string(),
            Value::String("TemplateRepeatedBomReport".to_string()),
        );
        args.insert(
            "TemplateName".to_string(),
            Value::String("ОсновнаяСхемаКомпоновкиДанных".to_string()),
        );
        args.insert(
            "TemplateType".to_string(),
            Value::String("DataCompositionSchema".to_string()),
        );
        args.insert(
            "SrcDir".to_string(),
            Value::String("src/Reports".to_string()),
        );

        let template_result = UnicaApplication::new()
            .call_tool("unica.template.add", &args)
            .unwrap();

        assert!(template_result.ok, "{:?}", template_result.errors);
        let report_bytes = std::fs::read(&report_path).unwrap();
        assert_eq!(leading_utf8_bom_count(&report_bytes), 1);
        assert!(String::from_utf8_lossy(&report_bytes)
            .contains("<Template>ОсновнаяСхемаКомпоновкиДанных</Template>"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn meta_validate_supports_pipe_separated_batch_paths() {
        let root = std::env::temp_dir().join(format!("unica-meta-batch-{}", std::process::id()));
        let workspace = root.join("workspace");
        let src = workspace.join("src");
        let fixtures = workspace.join("fixtures");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&fixtures).unwrap();
        std::fs::write(
            workspace.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(
            src.join("Configuration.xml"),
            support_test_configuration_xml("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"),
        )
        .unwrap();
        let items_json = fixtures.join("items.json");
        let other_json = fixtures.join("other.json");
        std::fs::write(&items_json, support_test_catalog_definition("Items")).unwrap();
        std::fs::write(&other_json, support_test_catalog_definition("Other")).unwrap();
        for json_path in [&items_json, &other_json] {
            let mut compile_args = Map::new();
            compile_args.insert(
                "cwd".to_string(),
                Value::String(workspace.display().to_string()),
            );
            compile_args.insert("dryRun".to_string(), Value::Bool(false));
            compile_args.insert(
                "JsonPath".to_string(),
                Value::String(json_path.display().to_string()),
            );
            compile_args.insert("OutputDir".to_string(), Value::String("src".to_string()));
            let compile_result = UnicaApplication::new()
                .call_tool("unica.meta.compile", &compile_args)
                .unwrap();
            assert!(compile_result.ok, "{:?}", compile_result.stderr);
        }
        let mut args = Map::new();
        args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        args.insert(
            "ObjectPath".to_string(),
            Value::String("src/Catalogs/Items.xml|src/Catalogs/Other.xml".to_string()),
        );

        let result = UnicaApplication::new()
            .call_tool("unica.meta.validate", &args)
            .unwrap();

        assert!(result.ok);
        assert!(result
            .summary
            .contains("completed with native metadata validator"));
        let stdout = result.stdout.unwrap();
        assert!(stdout.contains("=== meta-validate batch summary ==="));
        assert!(stdout.contains("Validated: 2"));
        assert!(stdout.contains("src/Catalogs/Items.xml"));
        assert!(stdout.contains("src/Catalogs/Other.xml"));
        assert_eq!(result.artifacts.len(), 2);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn meta_validate_accepts_platform_hierarchy_of_items() {
        let (root, catalog_path) =
            compile_test_catalog_with_hierarchy_type("validate-platform", "HierarchyOfItems");
        let workspace = root.join("workspace");
        assert!(std::fs::read_to_string(&catalog_path)
            .unwrap()
            .contains("<HierarchyType>HierarchyOfItems</HierarchyType>"));

        let result = call_meta_validate(&workspace, "src/Catalogs/Items.xml");

        assert!(
            result.ok,
            "platform-valid HierarchyOfItems was rejected: {:?}\n{}",
            result.errors,
            result.stdout.unwrap_or_default()
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn meta_compile_normalizes_legacy_hierarchy_items_only() {
        let (root, catalog_path) =
            compile_test_catalog_with_hierarchy_type("compile-legacy", "HierarchyItemsOnly");

        let catalog_xml = std::fs::read_to_string(catalog_path).unwrap();
        assert!(catalog_xml.contains("<HierarchyType>HierarchyOfItems</HierarchyType>"));
        assert!(!catalog_xml.contains("HierarchyItemsOnly"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn meta_edit_normalizes_legacy_hierarchy_items_only() {
        let (root, catalog_path) =
            compile_test_catalog_with_hierarchy_type("edit-legacy", "HierarchyFoldersAndItems");
        let workspace = root.join("workspace");
        let mut args = Map::new();
        args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        args.insert("dryRun".to_string(), Value::Bool(false));
        args.insert(
            "ObjectPath".to_string(),
            Value::String("src/Catalogs/Items.xml".to_string()),
        );
        args.insert(
            "Operation".to_string(),
            Value::String("modify-property".to_string()),
        );
        args.insert(
            "Value".to_string(),
            Value::String("HierarchyType=HierarchyItemsOnly".to_string()),
        );

        let edit = UnicaApplication::new()
            .call_tool("unica.meta.edit", &args)
            .unwrap();

        assert!(edit.ok, "{:?}", edit.errors);
        let catalog_xml = std::fs::read_to_string(catalog_path).unwrap();
        assert!(catalog_xml.contains("<HierarchyType>HierarchyOfItems</HierarchyType>"));
        assert!(!catalog_xml.contains("HierarchyItemsOnly"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn meta_edit_sets_enum_fill_value_through_public_tool() {
        let root = temp_meta_compile_workspace("unica-meta-edit-enum-fill-value");
        let workspace = root.join("workspace");
        let fixtures = workspace.join("fixtures");
        std::fs::create_dir_all(&fixtures).unwrap();

        let enum_definition = fixtures.join("status-enum.json");
        std::fs::write(
            &enum_definition,
            r#"{
  "type": "Enum",
  "name": "SampleStatus",
  "values": ["Default"]
}"#,
        )
        .unwrap();
        let enum_compile = call_meta_compile(&workspace, &enum_definition);
        assert!(enum_compile.ok, "{:?}", enum_compile.errors);

        let catalog_definition = fixtures.join("items-catalog.json");
        std::fs::write(
            &catalog_definition,
            r#"{
  "type": "Catalog",
  "name": "Items",
  "attributes": [
    { "name": "Status", "type": "EnumRef.SampleStatus" }
  ]
}"#,
        )
        .unwrap();
        let catalog_compile = call_meta_compile(&workspace, &catalog_definition);
        assert!(catalog_compile.ok, "{:?}", catalog_compile.errors);
        let catalog_path = workspace.join("src/Catalogs/Items.xml");
        let catalog_before = std::fs::read_to_string(&catalog_path).unwrap();
        let catalog_expected = catalog_before.replacen(
            "<FillValue xsi:nil=\"true\"/>",
            "<FillValue xsi:type=\"xr:DesignTimeRef\">Enum.SampleStatus.EnumValue.Default</FillValue>",
            1,
        );
        assert_ne!(catalog_expected, catalog_before);

        let mut args = Map::new();
        args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        args.insert("dryRun".to_string(), Value::Bool(false));
        args.insert(
            "ObjectPath".to_string(),
            Value::String("src/Catalogs/Items.xml".to_string()),
        );
        args.insert(
            "Operation".to_string(),
            Value::String("modify-attribute".to_string()),
        );
        args.insert(
            "Value".to_string(),
            Value::String("Status: fillValue=Enum.SampleStatus.EnumValue.Default".to_string()),
        );

        let edit = UnicaApplication::new()
            .call_tool("unica.meta.edit", &args)
            .unwrap();

        assert!(edit.ok, "{:?}", edit.errors);
        let catalog_after = std::fs::read_to_string(catalog_path).unwrap();
        assert_eq!(catalog_after, catalog_expected);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn role_compile_registers_in_canonical_position_and_preserves_crlf() {
        let root =
            temp_scaffolded_configuration_workspace("unica-role-compile-canonical-registration");
        let workspace = root.join("workspace");
        let src = workspace.join("src");
        let config_path = src.join("Configuration.xml");
        write_scaffolded_configuration_fixture(
            &config_path,
            &[
                "<Language>Русский</Language>",
                "<SessionParameter>CurrentUser</SessionParameter>",
                "<CommonTemplate>Shared</CommonTemplate>",
            ],
            "<!-- registrar-tail -->\n\n",
        );
        let config_before = std::fs::read(&config_path).unwrap();
        let role_json = workspace.join("sample-user.json");
        std::fs::write(
            &role_json,
            r#"{
  "name": "SampleUser",
  "synonym": "Sample user",
  "objects": ["Catalog.Items: @view"]
}"#,
        )
        .unwrap();
        let mut args = Map::new();
        args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        args.insert("dryRun".to_string(), Value::Bool(true));
        args.insert(
            "JsonPath".to_string(),
            Value::String(role_json.display().to_string()),
        );
        args.insert("OutputDir".to_string(), Value::String("src".to_string()));

        let preview = UnicaApplication::new()
            .call_tool("unica.role.compile", &args)
            .unwrap();

        assert!(preview.ok, "{:?}", preview.errors);
        assert!(preview.summary.contains("dry run"));
        assert!(preview
            .changes
            .iter()
            .any(|change| change.contains("would create") && change.contains("SampleUser.xml")));
        assert!(preview
            .changes
            .iter()
            .any(|change| change.contains("would update") && change.contains("Configuration.xml")));
        assert!(preview.stdout.unwrap_or_default().contains("@@ bytes"));
        assert!(preview.artifacts.is_empty());
        assert_eq!(std::fs::read(&config_path).unwrap(), config_before);
        assert!(!src.join("Roles/SampleUser.xml").exists());

        args.insert("dryRun".to_string(), Value::Bool(false));
        let result = UnicaApplication::new()
            .call_tool("unica.role.compile", &args)
            .unwrap();

        assert!(result.ok, "{:?}", result.errors);
        let config = String::from_utf8(std::fs::read(&config_path).unwrap()).unwrap();
        assert!(config.contains(concat!(
            "\t\t\t<SessionParameter>CurrentUser</SessionParameter>\r\n",
            "\t\t\t<Role>SampleUser</Role>\r\n",
            "\t\t\t<CommonTemplate>Shared</CommonTemplate>\r\n"
        )));
        assert!(config.ends_with("<!-- registrar-tail -->\r\n\r\n"));
        assert!(!config.replace("\r\n", "").contains('\n'));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn role_compile_generates_distinct_non_placeholder_uuid_v4() {
        let root = temp_meta_compile_workspace("unica-role-compile-uuid-v4");
        let workspace = root.join("workspace");
        let fixtures = workspace.join("fixtures");
        std::fs::create_dir_all(&fixtures).unwrap();

        let reader_json = fixtures.join("sample-reader.json");
        std::fs::write(
            &reader_json,
            r#"{
  "name": "SampleReader",
  "synonym": "Sample reader",
  "comment": "Synthetic repro",
  "objects": ["Catalog.Items: @view"]
}"#,
        )
        .unwrap();
        let editor_json = fixtures.join("sample-editor.json");
        std::fs::write(
            &editor_json,
            r#"{
  "name": "SampleEditor",
  "synonym": "Sample editor",
  "comment": "Synthetic repro",
  "objects": ["Catalog.Items: @view @edit"]
}"#,
        )
        .unwrap();

        for json_path in [&reader_json, &editor_json] {
            let mut args = Map::new();
            args.insert(
                "cwd".to_string(),
                Value::String(workspace.display().to_string()),
            );
            args.insert("dryRun".to_string(), Value::Bool(false));
            args.insert(
                "JsonPath".to_string(),
                Value::String(json_path.display().to_string()),
            );
            args.insert("OutputDir".to_string(), Value::String("src".to_string()));
            let result = UnicaApplication::new()
                .call_tool("unica.role.compile", &args)
                .unwrap();

            assert!(result.ok, "{:?}", result.errors);
        }

        let reader_xml =
            std::fs::read_to_string(workspace.join("src/Roles/SampleReader.xml")).unwrap();
        let editor_xml =
            std::fs::read_to_string(workspace.join("src/Roles/SampleEditor.xml")).unwrap();
        assert_valid_root_uuid(&reader_xml, "Role");
        assert_valid_root_uuid(&editor_xml, "Role");
        let reader_uuid = metadata_root_uuid(&reader_xml, "Role");
        let editor_uuid = metadata_root_uuid(&editor_xml, "Role");
        assert_ne!(reader_uuid, editor_uuid);
        for uuid in [&reader_uuid, &editor_uuid] {
            assert!(
                !uuid.starts_with("00000000-0000-0000-"),
                "role.compile must not generate placeholder UUID: {uuid}"
            );
            assert_eq!(
                uuid.as_bytes().get(14),
                Some(&b'4'),
                "UUID must be v4: {uuid}"
            );
        }

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn role_compile_preserves_existing_uuid_when_regenerating_role() {
        let root = temp_meta_compile_workspace("unica-role-compile-idempotent-uuid");
        let workspace = root.join("workspace");
        let fixtures = workspace.join("fixtures");
        std::fs::create_dir_all(&fixtures).unwrap();

        let role_json = fixtures.join("sample-reader.json");
        std::fs::write(
            &role_json,
            r#"{
  "name": "SampleReader",
  "synonym": "Sample reader",
  "comment": "Synthetic repro",
  "objects": ["Catalog.Items: @view"]
}"#,
        )
        .unwrap();

        let mut args = Map::new();
        args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        args.insert("dryRun".to_string(), Value::Bool(false));
        args.insert(
            "JsonPath".to_string(),
            Value::String(role_json.display().to_string()),
        );
        args.insert("OutputDir".to_string(), Value::String("src".to_string()));
        let result = UnicaApplication::new()
            .call_tool("unica.role.compile", &args)
            .unwrap();

        assert!(result.ok, "{:?}", result.errors);

        let first_xml =
            std::fs::read_to_string(workspace.join("src/Roles/SampleReader.xml")).unwrap();
        let first_uuid = metadata_root_uuid(&first_xml, "Role");
        let metadata_path = workspace.join("src/Roles/SampleReader.xml");
        let rights_path = workspace.join("src/Roles/SampleReader/Ext/Rights.xml");
        let config_path = workspace.join("src/Configuration.xml");
        let metadata_before = std::fs::read(&metadata_path).unwrap();
        let rights_before = std::fs::read(&rights_path).unwrap();
        let config_before = std::fs::read(&config_path).unwrap();
        std::fs::write(
            &role_json,
            r#"{
  "name": "SampleReader",
  "synonym": "Changed definition must not overwrite",
  "comment": "Synthetic repro",
  "objects": ["Catalog.Items: @view @edit"]
}"#,
        )
        .unwrap();

        let mut args = Map::new();
        args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        args.insert("dryRun".to_string(), Value::Bool(false));
        args.insert(
            "JsonPath".to_string(),
            Value::String(role_json.display().to_string()),
        );
        args.insert("OutputDir".to_string(), Value::String("src".to_string()));
        let result = UnicaApplication::new()
            .call_tool("unica.role.compile", &args)
            .unwrap();

        assert!(result.ok, "{:?}", result.errors);
        assert!(result.changes.is_empty(), "{:?}", result.changes);
        assert!(result.artifacts.is_empty(), "{:?}", result.artifacts);

        let regenerated_xml =
            std::fs::read_to_string(workspace.join("src/Roles/SampleReader.xml")).unwrap();
        let regenerated_uuid = metadata_root_uuid(&regenerated_xml, "Role");
        assert_eq!(first_uuid, regenerated_uuid);
        assert_eq!(std::fs::read(&metadata_path).unwrap(), metadata_before);
        assert_eq!(std::fs::read(&rights_path).unwrap(), rights_before);
        assert_eq!(std::fs::read(&config_path).unwrap(), config_before);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn meta_compile_creates_constant_with_boolean_type() {
        let root = temp_meta_compile_workspace("unica-meta-compile-constant-bool");
        let workspace = root.join("workspace");
        let src = workspace.join("src");
        let fixtures = workspace.join("fixtures");
        std::fs::create_dir_all(&fixtures).unwrap();
        let json_path = fixtures.join("constant-bool.json");
        std::fs::write(
            &json_path,
            r#"{
  "type": "Constant",
  "name": "DemoFlag",
  "synonym": "Demo flag",
  "comment": "Synthetic repro",
  "valueType": "Boolean"
}"#,
        )
        .unwrap();

        let result = call_meta_compile(&workspace, &json_path);

        assert!(result.ok, "{:?}", result.stderr);
        let xml_path = src.join("Constants").join("DemoFlag.xml");
        assert!(xml_path.is_file());
        let xml = std::fs::read_to_string(&xml_path).unwrap();
        assert_valid_root_uuid(&xml, "Constant");
        assert!(xml.contains("<Name>DemoFlag</Name>"));
        assert!(xml.contains("<v8:Type>xs:boolean</v8:Type>"));
        assert!(xml.contains("ConstantManager.DemoFlag"));
        assert!(std::fs::read_to_string(src.join("Configuration.xml"))
            .unwrap()
            .contains("<Constant>DemoFlag</Constant>"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn meta_compile_creates_constant_with_catalog_ref_type() {
        let root = temp_meta_compile_workspace("unica-meta-compile-constant-ref");
        let workspace = root.join("workspace");
        let src = workspace.join("src");
        let fixtures = workspace.join("fixtures");
        std::fs::create_dir_all(&fixtures).unwrap();
        let json_path = fixtures.join("constant-ref.json");
        std::fs::write(
            &json_path,
            r#"{
  "type": "Constant",
  "name": "MainCurrency",
  "valueType": "CatalogRef.Currencies"
}"#,
        )
        .unwrap();

        let result = call_meta_compile(&workspace, &json_path);

        assert!(result.ok, "{:?}", result.stderr);
        let xml = std::fs::read_to_string(src.join("Constants").join("MainCurrency.xml")).unwrap();
        assert!(xml.contains("<v8:Type>cfg:CatalogRef.Currencies</v8:Type>"));
        assert!(std::fs::read_to_string(src.join("Configuration.xml"))
            .unwrap()
            .contains("<Constant>MainCurrency</Constant>"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn meta_compile_creates_common_module_with_server_context() {
        let root = temp_meta_compile_workspace("unica-meta-compile-common-module");
        let workspace = root.join("workspace");
        let src = workspace.join("src");
        let fixtures = workspace.join("fixtures");
        std::fs::create_dir_all(&fixtures).unwrap();
        let json_path = fixtures.join("common-module.json");
        std::fs::write(
            &json_path,
            r#"{
  "type": "CommonModule",
  "name": "DemoServerModule",
  "synonym": "Demo server module",
  "comment": "Synthetic repro",
  "context": "server",
  "returnValuesReuse": "DuringRequest"
}"#,
        )
        .unwrap();

        let result = call_meta_compile(&workspace, &json_path);

        assert!(result.ok, "{:?}", result.stderr);
        let xml_path = src.join("CommonModules").join("DemoServerModule.xml");
        let module_path = src
            .join("CommonModules")
            .join("DemoServerModule")
            .join("Ext")
            .join("Module.bsl");
        assert!(xml_path.is_file());
        assert!(module_path.is_file());
        let xml = std::fs::read_to_string(&xml_path).unwrap();
        assert_valid_root_uuid(&xml, "CommonModule");
        assert!(xml.contains("<Server>true</Server>"));
        assert!(xml.contains("<ServerCall>true</ServerCall>"));
        assert!(xml.contains("<ClientManagedApplication>false</ClientManagedApplication>"));
        assert!(xml.contains("<ReturnValuesReuse>DuringRequest</ReturnValuesReuse>"));
        assert!(std::fs::read_to_string(src.join("Configuration.xml"))
            .unwrap()
            .contains("<CommonModule>DemoServerModule</CommonModule>"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn meta_compile_creates_enum_and_defined_type() {
        let root = temp_meta_compile_workspace("unica-meta-compile-enum-defined");
        let workspace = root.join("workspace");
        let src = workspace.join("src");
        let fixtures = workspace.join("fixtures");
        std::fs::create_dir_all(&fixtures).unwrap();

        let enum_json = fixtures.join("enum.json");
        std::fs::write(
            &enum_json,
            r#"{
  "type": "Enum",
  "name": "DemoStatuses",
  "values": ["New", "Closed"]
}"#,
        )
        .unwrap();
        let enum_result = call_meta_compile(&workspace, &enum_json);
        assert!(enum_result.ok, "{:?}", enum_result.stderr);

        let defined_json = fixtures.join("defined.json");
        std::fs::write(
            &defined_json,
            r#"{
  "type": "DefinedType",
  "name": "DemoValue",
  "valueTypes": ["String(100)", "CatalogRef.Products"]
}"#,
        )
        .unwrap();
        let defined_result = call_meta_compile(&workspace, &defined_json);
        assert!(defined_result.ok, "{:?}", defined_result.stderr);

        let enum_xml = std::fs::read_to_string(src.join("Enums").join("DemoStatuses.xml")).unwrap();
        assert!(enum_xml.contains("<EnumValue uuid=\""));
        assert!(enum_xml.contains("<Name>New</Name>"));
        assert!(enum_xml.contains("<Name>Closed</Name>"));
        let defined_xml =
            std::fs::read_to_string(src.join("DefinedTypes").join("DemoValue.xml")).unwrap();
        assert_valid_root_uuid(&defined_xml, "DefinedType");
        assert!(defined_xml.contains("<v8:Type>xs:string</v8:Type>"));
        assert!(defined_xml.contains("<v8:Type>cfg:CatalogRef.Products</v8:Type>"));
        let config = std::fs::read_to_string(src.join("Configuration.xml")).unwrap();
        assert!(config.contains("<Enum>DemoStatuses</Enum>"));
        assert!(config.contains("<DefinedType>DemoValue</DefinedType>"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn meta_compile_event_subscription_uses_documented_object_source_type() {
        let root = temp_meta_compile_workspace("unica-meta-compile-event-source");
        let workspace = root.join("workspace");
        let src = workspace.join("src");
        let fixtures = workspace.join("fixtures");
        std::fs::create_dir_all(&fixtures).unwrap();
        let module_json = fixtures.join("event-handlers.json");
        std::fs::write(
            &module_json,
            r#"{
  "type": "CommonModule",
  "name": "EventHandlers",
  "context": "server"
}"#,
        )
        .unwrap();
        let module_result = call_meta_compile(&workspace, &module_json);
        assert!(module_result.ok, "{:?}", module_result.errors);
        std::fs::write(
            src.join("CommonModules/EventHandlers/Ext/Module.bsl"),
            "\u{feff}Procedure OnBeforeWrite(Source, Cancel, StandardProcessing) Export\nEndProcedure\n",
        )
        .unwrap();
        let document_json = fixtures.join("sales-order.json");
        std::fs::write(
            &document_json,
            r#"{
  "type": "Document",
  "name": "SalesOrder"
}"#,
        )
        .unwrap();
        let document_result = call_meta_compile(&workspace, &document_json);
        assert!(document_result.ok, "{:?}", document_result.errors);
        let json_path = fixtures.join("event-subscription.json");
        std::fs::write(
            &json_path,
            r#"{
  "type": "EventSubscription",
  "name": "BeforeDocumentWrite",
  "source": ["DocumentObject.SalesOrder"],
  "event": "BeforeWrite",
  "handler": "EventHandlers.OnBeforeWrite"
}"#,
        )
        .unwrap();

        let result = call_meta_compile(&workspace, &json_path);

        assert!(result.ok, "{:?}", result.stderr);
        let xml = std::fs::read_to_string(
            src.join("EventSubscriptions")
                .join("BeforeDocumentWrite.xml"),
        )
        .unwrap();
        assert!(xml.contains("<v8:Type>cfg:DocumentObject.SalesOrder</v8:Type>"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn meta_compile_supports_all_documented_pending_types() {
        struct Case {
            obj_type: &'static str,
            name: &'static str,
            plural: &'static str,
            json: &'static str,
            markers: &'static [&'static str],
            ext_files: &'static [&'static str],
        }

        let root = temp_meta_compile_workspace("unica-meta-compile-documented-types");
        let workspace = root.join("workspace");
        let src = workspace.join("src");
        let fixtures = workspace.join("fixtures");
        std::fs::create_dir_all(&fixtures).unwrap();

        let module_json = fixtures.join("event-handlers.json");
        std::fs::write(
            &module_json,
            r#"{
  "type": "CommonModule",
  "name": "EventHandlers",
  "context": "server"
}"#,
        )
        .unwrap();
        let module_result = call_meta_compile(&workspace, &module_json);
        assert!(module_result.ok, "{:?}", module_result.stderr);
        std::fs::write(
            src.join("CommonModules")
                .join("EventHandlers")
                .join("Ext")
                .join("Module.bsl"),
            "\u{feff}Procedure RunJob() Export\nEndProcedure\n\nProcedure OnBeforeWrite(Source, Cancel, StandardProcessing) Export\nEndProcedure\n",
        )
        .unwrap();

        let cases = [
            Case {
                obj_type: "Document",
                name: "MetaCompileDocument",
                plural: "Documents",
                json: r#"{
  "type": "Document",
  "name": "MetaCompileDocument",
  "numberLength": 8,
  "attributes": ["Partner:CatalogRef.Partners|req,index"],
  "tabularSections": {"Lines": ["Quantity:Number(10,2)"]}
}"#,
                markers: &[
                    "<Document uuid=\"",
                    "DocumentObject.MetaCompileDocument",
                    "<xr:StandardAttribute name=\"Posted\">",
                    "<Attribute uuid=\"",
                    "<TabularSection uuid=\"",
                ],
                ext_files: &["ObjectModule.bsl"],
            },
            Case {
                obj_type: "InformationRegister",
                name: "MetaCompileInfoRegister",
                plural: "InformationRegisters",
                json: r#"{
  "type": "InformationRegister",
  "name": "MetaCompileInfoRegister",
  "periodicity": "Month",
  "dimensions": ["Item:CatalogRef.Items|master,index"],
  "resources": ["Price:Number(15,2)"],
  "attributes": ["Comment:String(100)"]
}"#,
                markers: &[
                    "<InformationRegister uuid=\"",
                    "InformationRegisterRecordSet.MetaCompileInfoRegister",
                    "<InformationRegisterPeriodicity>Month</InformationRegisterPeriodicity>",
                    "<Dimension uuid=\"",
                    "<Resource uuid=\"",
                ],
                ext_files: &["RecordSetModule.bsl"],
            },
            Case {
                obj_type: "AccumulationRegister",
                name: "MetaCompileAccumulation",
                plural: "AccumulationRegisters",
                json: r#"{
  "type": "AccumulationRegister",
  "name": "MetaCompileAccumulation",
  "registerType": "Balances",
  "dimensions": ["Warehouse:CatalogRef.Warehouses|index"],
  "resources": ["Quantity:Number(15,3)"],
  "attributes": ["Batch:String(40)"]
}"#,
                markers: &[
                    "<AccumulationRegister uuid=\"",
                    "AccumulationRegisterRecordSet.MetaCompileAccumulation",
                    "<RegisterType>Balance</RegisterType>",
                    "<UseInTotals>true</UseInTotals>",
                ],
                ext_files: &["RecordSetModule.bsl"],
            },
            Case {
                obj_type: "AccountingRegister",
                name: "MetaCompileAccounting",
                plural: "AccountingRegisters",
                json: r#"{
  "type": "AccountingRegister",
  "name": "MetaCompileAccounting",
  "chartOfAccounts": "ChartOfAccounts.MetaCompileAccounts",
  "dimensions": ["Department:CatalogRef.Departments"],
  "resources": ["Amount:Number(15,2)"],
  "attributes": ["Description:String(50)"]
}"#,
                markers: &[
                    "<AccountingRegister uuid=\"",
                    "AccountingRegisterExtDimensions.MetaCompileAccounting",
                    "<ChartOfAccounts>ChartOfAccounts.MetaCompileAccounts</ChartOfAccounts>",
                    "<Resource uuid=\"",
                ],
                ext_files: &["RecordSetModule.bsl"],
            },
            Case {
                obj_type: "CalculationRegister",
                name: "MetaCompileCalculation",
                plural: "CalculationRegisters",
                json: r#"{
  "type": "CalculationRegister",
  "name": "MetaCompileCalculation",
  "chartOfCalculationTypes": "ChartOfCalculationTypes.MetaCompileCalcTypes",
  "periodicity": "Month",
  "dimensions": ["Employee:CatalogRef.Employees"],
  "resources": ["Result:Number(15,2)"],
  "attributes": ["Comment:String(50)"]
}"#,
                markers: &[
                    "<CalculationRegister uuid=\"",
                    "CalculationRegisterRecordSet.MetaCompileCalculation",
                    "<ChartOfCalculationTypes>ChartOfCalculationTypes.MetaCompileCalcTypes</ChartOfCalculationTypes>",
                    "<Periodicity>Month</Periodicity>",
                ],
                ext_files: &["RecordSetModule.bsl"],
            },
            Case {
                obj_type: "ChartOfAccounts",
                name: "MetaCompileAccounts",
                plural: "ChartsOfAccounts",
                json: r#"{
  "type": "ChartOfAccounts",
  "name": "MetaCompileAccounts",
  "extDimensionTypes": "ChartOfCharacteristicTypes.MetaCompileCharacteristics",
  "accountingFlags": ["Tax"],
  "extDimensionAccountingFlags": ["Department"],
  "attributes": ["ExternalCode:String(20)"]
}"#,
                markers: &[
                    "<ChartOfAccounts uuid=\"",
                    "ChartOfAccountsExtDimensionTypes.MetaCompileAccounts",
                    "<AccountingFlag uuid=\"",
                    "<ExtDimensionAccountingFlag uuid=\"",
                ],
                ext_files: &["ObjectModule.bsl"],
            },
            Case {
                obj_type: "ChartOfCharacteristicTypes",
                name: "MetaCompileCharacteristics",
                plural: "ChartsOfCharacteristicTypes",
                json: r#"{
  "type": "ChartOfCharacteristicTypes",
  "name": "MetaCompileCharacteristics",
  "valueTypes": ["String(50)", "Number(15,2)"],
  "attributes": ["Group:String(20)"]
}"#,
                markers: &[
                    "<ChartOfCharacteristicTypes uuid=\"",
                    "Characteristic.MetaCompileCharacteristics",
                    "<v8:Type>xs:string</v8:Type>",
                    "<Attribute uuid=\"",
                ],
                ext_files: &["ObjectModule.bsl"],
            },
            Case {
                obj_type: "ChartOfCalculationTypes",
                name: "MetaCompileCalcTypes",
                plural: "ChartsOfCalculationTypes",
                json: r#"{
  "type": "ChartOfCalculationTypes",
  "name": "MetaCompileCalcTypes",
  "dependenceOnCalculationTypes": "OnActionPeriod",
  "baseCalculationTypes": ["ChartOfCalculationTypes.BaseSalary"],
  "attributes": ["Kind:String(20)"]
}"#,
                markers: &[
                    "<ChartOfCalculationTypes uuid=\"",
                    "BaseCalculationTypes.MetaCompileCalcTypes",
                    "<DependenceOnCalculationTypes>OnActionPeriod</DependenceOnCalculationTypes>",
                    "<BaseCalculationTypes>",
                ],
                ext_files: &["ObjectModule.bsl"],
            },
            Case {
                obj_type: "BusinessProcess",
                name: "MetaCompileProcess",
                plural: "BusinessProcesses",
                json: r#"{
  "type": "BusinessProcess",
  "name": "MetaCompileProcess",
  "task": "Task.MetaCompileTask",
  "attributes": ["Subject:String(100)"]
}"#,
                markers: &[
                    "<BusinessProcess uuid=\"",
                    "BusinessProcessRoutePointRef.MetaCompileProcess",
                    "<Task>Task.MetaCompileTask</Task>",
                    "<Attribute uuid=\"",
                ],
                ext_files: &["ObjectModule.bsl", "Flowchart.xml"],
            },
            Case {
                obj_type: "Task",
                name: "MetaCompileTask",
                plural: "Tasks",
                json: r#"{
  "type": "Task",
  "name": "MetaCompileTask",
  "addressing": "CatalogRef.Users",
  "mainAddressingAttribute": "Performer",
  "addressingAttributes": [
    {"name": "Performer", "type": "CatalogRef.Users", "addressingDimension": "Catalog.Users"}
  ],
  "attributes": ["Priority:Number(3,0)"]
}"#,
                markers: &[
                    "<Task uuid=\"",
                    "TaskObject.MetaCompileTask",
                    "<AddressingAttribute uuid=\"",
                    "<MainAddressingAttribute>Performer</MainAddressingAttribute>",
                ],
                ext_files: &["ObjectModule.bsl"],
            },
            Case {
                obj_type: "ExchangePlan",
                name: "MetaCompileExchange",
                plural: "ExchangePlans",
                json: r#"{
  "type": "ExchangePlan",
  "name": "MetaCompileExchange",
  "distributedInfoBase": true,
  "includeConfigurationExtensions": true,
  "attributes": ["NodeKind:String(20)"]
}"#,
                markers: &[
                    "<ExchangePlan uuid=\"",
                    "<xr:ThisNode>",
                    "ExchangePlanObject.MetaCompileExchange",
                    "<DistributedInfoBase>true</DistributedInfoBase>",
                ],
                ext_files: &["ObjectModule.bsl", "Content.xml"],
            },
            Case {
                obj_type: "DocumentJournal",
                name: "MetaCompileJournal",
                plural: "DocumentJournals",
                json: r#"{
  "type": "DocumentJournal",
  "name": "MetaCompileJournal",
  "registeredDocuments": ["Document.MetaCompileDocument"],
  "columns": [
    {"name": "Partner", "references": ["Document.MetaCompileDocument"]}
  ]
}"#,
                markers: &[
                    "<DocumentJournal uuid=\"",
                    "DocumentJournalManager.MetaCompileJournal",
                    "<RegisteredDocuments>",
                    "<Column uuid=\"",
                    "<References>",
                ],
                ext_files: &[],
            },
            Case {
                obj_type: "Report",
                name: "MetaCompileReport",
                plural: "Reports",
                json: r#"{
  "type": "Report",
  "name": "MetaCompileReport",
  "attributes": ["Period:String(20)"],
  "tabularSections": {"Settings": ["Key:String(40)", "Value:String(100)"]}
}"#,
                markers: &[
                    "<Report uuid=\"",
                    "ReportObject.MetaCompileReport",
                    "<UseStandardCommands>true</UseStandardCommands>",
                    "<TabularSection uuid=\"",
                ],
                ext_files: &["ObjectModule.bsl", "ManagerModule.bsl"],
            },
            Case {
                obj_type: "DataProcessor",
                name: "MetaCompileProcessor",
                plural: "DataProcessors",
                json: r#"{
  "type": "DataProcessor",
  "name": "MetaCompileProcessor",
  "attributes": ["FileName:String(260)"],
  "tabularSections": {"Rows": ["Value:String(100)"]}
}"#,
                markers: &[
                    "<DataProcessor uuid=\"",
                    "DataProcessorManager.MetaCompileProcessor",
                    "<UseStandardCommands>false</UseStandardCommands>",
                    "<Attribute uuid=\"",
                ],
                ext_files: &["ObjectModule.bsl", "ManagerModule.bsl"],
            },
            Case {
                obj_type: "ScheduledJob",
                name: "MetaCompileScheduledJob",
                plural: "ScheduledJobs",
                json: r#"{
  "type": "ScheduledJob",
  "name": "MetaCompileScheduledJob",
  "methodName": "EventHandlers.RunJob",
  "description": "Smoke job",
  "key": "smoke",
  "use": true,
  "predefined": true
}"#,
                markers: &[
                    "<ScheduledJob uuid=\"",
                    "<MethodName>CommonModule.EventHandlers.RunJob</MethodName>",
                    "<Use>true</Use>",
                ],
                ext_files: &[],
            },
            Case {
                obj_type: "EventSubscription",
                name: "MetaCompileSubscription",
                plural: "EventSubscriptions",
                json: r#"{
  "type": "EventSubscription",
  "name": "MetaCompileSubscription",
  "source": ["DocumentObject.MetaCompileDocument"],
  "event": "BeforeWrite",
  "handler": "EventHandlers.OnBeforeWrite"
}"#,
                markers: &[
                    "<EventSubscription uuid=\"",
                    "<Source>",
                    "<v8:Type>cfg:DocumentObject.MetaCompileDocument</v8:Type>",
                    "<Event>BeforeWrite</Event>",
                    "<Handler>CommonModule.EventHandlers.OnBeforeWrite</Handler>",
                ],
                ext_files: &[],
            },
            Case {
                obj_type: "HTTPService",
                name: "MetaCompileHTTP",
                plural: "HTTPServices",
                json: r#"{
  "type": "HTTPService",
  "name": "MetaCompileHTTP",
  "rootURL": "meta",
  "reuseSessions": "AutoUse",
  "urlTemplates": {
    "Items": {"template": "/items/{id}", "methods": {"Get": "GET", "Post": "POST"}}
  }
}"#,
                markers: &[
                    "<HTTPService uuid=\"",
                    "<RootURL>meta</RootURL>",
                    "<URLTemplate uuid=\"",
                    "<Method uuid=\"",
                    "<HTTPMethod>GET</HTTPMethod>",
                ],
                ext_files: &["Module.bsl"],
            },
            Case {
                obj_type: "WebService",
                name: "MetaCompileWeb",
                plural: "WebServices",
                json: r#"{
  "type": "WebService",
  "name": "MetaCompileWeb",
  "namespace": "urn:meta-compile",
  "reuseSessions": "AutoUse",
  "operations": {
    "Ping": {
      "returnType": "xs:string",
      "parameters": {"Text": "xs:string"}
    }
  }
}"#,
                markers: &[
                    "<WebService uuid=\"",
                    "<Namespace>urn:meta-compile</Namespace>",
                    "<Operation uuid=\"",
                    "<Parameter uuid=\"",
                    "<ProcedureName>Ping</ProcedureName>",
                ],
                ext_files: &["Module.bsl"],
            },
        ];

        let mut root_uuids = HashSet::new();

        for case in cases {
            let json_path = fixtures.join(format!("{}.json", case.name));
            std::fs::write(&json_path, case.json).unwrap();

            let result = call_meta_compile(&workspace, &json_path);
            assert!(result.ok, "{} failed: {:?}", case.obj_type, result.stderr);

            let xml_path = src.join(case.plural).join(format!("{}.xml", case.name));
            assert!(xml_path.is_file(), "missing {}", xml_path.display());
            let xml = std::fs::read_to_string(&xml_path).unwrap();
            let root_uuid = metadata_root_uuid(&xml, case.obj_type);
            assert!(
                root_uuids.insert(root_uuid.clone()),
                "duplicate root uuid {root_uuid} for {}.{}",
                case.obj_type,
                case.name
            );
            for marker in case.markers {
                assert!(
                    xml.contains(marker),
                    "{} XML missing marker {}",
                    case.obj_type,
                    marker
                );
            }
            let config = std::fs::read_to_string(src.join("Configuration.xml")).unwrap();
            assert!(
                config.contains(&format!(
                    "<{}>{}</{}>",
                    case.obj_type, case.name, case.obj_type
                )),
                "Configuration.xml missing {}.{}",
                case.obj_type,
                case.name
            );
            for ext_file in case.ext_files {
                let ext_path = src
                    .join(case.plural)
                    .join(case.name)
                    .join("Ext")
                    .join(ext_file);
                assert!(ext_path.is_file(), "missing {}", ext_path.display());
            }

            let validate = call_meta_validate(
                &workspace,
                &format!("src/{}/{}.xml", case.plural, case.name),
            );
            assert!(
                validate.ok,
                "{} failed validation: {:?}\n{}",
                case.obj_type,
                validate.errors,
                validate.stdout.unwrap_or_default()
            );
        }

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn help_add_routes_through_unica_and_creates_help_files() {
        let root = std::env::temp_dir().join(format!("unica-help-add-{}", std::process::id()));
        let workspace = root.join("workspace");
        let src = workspace.join("src");
        let object_dir = src.join("Catalogs").join("Items");
        let ext = object_dir.join("Ext");
        let forms = object_dir.join("Forms");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            workspace.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(
            src.join("Configuration.xml"),
            support_test_configuration_xml("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"),
        )
        .unwrap();
        let catalog_definition = workspace.join("catalog.json");
        std::fs::write(
            &catalog_definition,
            r#"{"type":"Catalog","name":"Items","synonym":"Items"}"#,
        )
        .unwrap();
        let catalog_result = UnicaApplication::new()
            .call_tool(
                "unica.meta.compile",
                &Map::from_iter([
                    (
                        "cwd".to_string(),
                        Value::String(workspace.display().to_string()),
                    ),
                    ("dryRun".to_string(), Value::Bool(false)),
                    (
                        "JsonPath".to_string(),
                        Value::String(catalog_definition.display().to_string()),
                    ),
                    ("OutputDir".to_string(), Value::String("src".to_string())),
                ]),
            )
            .expect("catalog fixture must route through the public application boundary");
        assert!(catalog_result.ok, "{:?}", catalog_result.errors);
        std::fs::create_dir_all(&ext).unwrap();
        std::fs::create_dir_all(&forms).unwrap();
        let form_path = forms.join("Main.xml");
        std::fs::write(&form_path, support_test_form_xml()).unwrap();

        let mut args = Map::new();
        args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        args.insert("dryRun".to_string(), Value::Bool(false));
        args.insert(
            "ObjectName".to_string(),
            Value::String("Catalogs/Items".to_string()),
        );
        args.insert("SrcDir".to_string(), Value::String("src".to_string()));
        args.insert("Lang".to_string(), Value::String("ru".to_string()));

        let result = UnicaApplication::new()
            .call_tool("unica.help.add", &args)
            .unwrap();

        assert!(result.ok, "{} {:?}", result.summary, result.errors);
        let help_xml = ext.join("Help.xml");
        let help_page = ext.join("Help").join("ru.html");
        assert!(help_xml.is_file());
        assert!(help_page.is_file());
        let generated_help = std::fs::read_to_string(&help_xml).unwrap();
        assert!(generated_help.contains("<Page>ru</Page>"));
        assert!(
            generated_help.contains(r#"version="2.20""#),
            "{generated_help}"
        );
        assert!(
            !generated_help.contains(r#"version="2.17""#),
            "{generated_help}"
        );
        assert!(std::fs::read_to_string(&help_page)
            .unwrap()
            .contains("<h1>Catalogs/Items</h1>"));
        assert!(std::fs::read_to_string(&form_path)
            .unwrap()
            .contains("<IncludeHelpInContents>false</IncludeHelpInContents>"));
        assert!(result.cache.events.contains(&"FormChanged".to_string()));
        assert!(result.cache.invalidated.contains(&"form_graph".to_string()));
        assert!(result.command.is_none());

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn help_add_blocks_locked_vendor_object_before_writing_files() {
        let root =
            std::env::temp_dir().join(format!("unica-help-add-guard-{}", std::process::id()));
        let workspace = root.join("workspace");
        let src = workspace.join("src");
        let support_ext = src.join("Ext");
        let object_dir = src.join("Catalogs").join("Items");
        let ext = object_dir.join("Ext");
        std::fs::create_dir_all(&support_ext).unwrap();
        std::fs::create_dir_all(&ext).unwrap();
        std::fs::write(
            workspace.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(
            src.join("Configuration.xml"),
            support_test_configuration_xml("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"),
        )
        .unwrap();
        std::fs::create_dir_all(src.join("Catalogs")).unwrap();
        std::fs::write(
            src.join("Catalogs").join("Items.xml"),
            support_test_catalog_xml("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"),
        )
        .unwrap();
        std::fs::write(
            support_ext.join("ParentConfigurations.bin"),
            support_test_parent_configurations_bin(
                "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
                "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb",
                "cccccccc-cccc-cccc-cccc-cccccccccccc",
            ),
        )
        .unwrap();

        let mut args = Map::new();
        args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        args.insert("dryRun".to_string(), Value::Bool(false));
        args.insert(
            "ObjectName".to_string(),
            Value::String("Catalogs/Items".to_string()),
        );
        args.insert("SrcDir".to_string(), Value::String("src".to_string()));

        let result = UnicaApplication::new()
            .call_tool("unica.help.add", &args)
            .unwrap();

        assert!(!result.ok);
        assert!(result.summary.contains("support guard"));
        assert!(!ext.join("Help.xml").exists());
        assert!(result.cache.events.is_empty());

        let _ = std::fs::remove_dir_all(root);
    }

    fn support_test_catalog_definition(name: &str) -> String {
        format!(
            r#"{{
  "type": "Catalog",
  "name": "{name}",
  "synonym": "{name}",
  "codeLength": 9,
  "descriptionLength": 50,
  "attributes": [
    {{
      "name": "Article",
      "type": "String",
      "length": 32,
      "synonym": "Article"
    }}
  ]
}}"#
        )
    }

    fn compile_test_catalog_with_hierarchy_type(
        prefix: &str,
        hierarchy_type: &str,
    ) -> (std::path::PathBuf, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "unica-meta-hierarchy-{prefix}-{}",
            std::process::id()
        ));
        let workspace = root.join("workspace");
        let src = workspace.join("src");
        let fixtures = workspace.join("fixtures");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&fixtures).unwrap();
        std::fs::write(
            workspace.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(
            src.join("Configuration.xml"),
            support_test_configuration_xml("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"),
        )
        .unwrap();
        let definition_path = fixtures.join("items.json");
        std::fs::write(
            &definition_path,
            serde_json::to_string_pretty(&serde_json::json!({
                "type": "Catalog",
                "name": "Items",
                "synonym": "Items",
                "hierarchical": true,
                "hierarchyType": hierarchy_type,
            }))
            .unwrap(),
        )
        .unwrap();
        let compile = call_meta_compile(&workspace, &definition_path);
        assert!(compile.ok, "{:?}", compile.errors);

        let catalog_path = src.join("Catalogs").join("Items.xml");
        (root, catalog_path)
    }

    struct FixedOutcomePorts {
        outcome: AdapterOutcome,
        data: Option<Value>,
    }

    impl ports::ApplicationPorts for FixedOutcomePorts {
        fn discover_workspace(
            &self,
            requested_cwd: Option<PathBuf>,
        ) -> Result<WorkspaceContext, String> {
            let cwd = requested_cwd.unwrap_or_default();
            Ok(WorkspaceContext {
                cwd: cwd.clone(),
                workspace_root: cwd.clone(),
                cache_root: cwd.join(".build").join("unica"),
                workspace_epoch: 1,
            })
        }

        fn validate_tool_context(
            &self,
            _spec: ToolSpec,
            _args: &Map<String, Value>,
            _dry_run: bool,
            _context: &WorkspaceContext,
        ) -> Result<(), String> {
            Ok(())
        }

        fn evaluate_support_guard(
            &self,
            _spec: ToolSpec,
            _args: &Map<String, Value>,
            _context: &WorkspaceContext,
        ) -> Result<SupportGuardCheck, String> {
            Ok(SupportGuardCheck::Allow)
        }

        fn invoke_handler(
            &self,
            _spec: ToolSpec,
            _args: &Map<String, Value>,
            _context: &WorkspaceContext,
            _dry_run: bool,
            _cancellation: &CancellationToken,
        ) -> Result<ports::HandlerOutcome, String> {
            Ok(match self.data.clone() {
                Some(data) => ports::HandlerOutcome::with_data(self.outcome.clone(), data),
                None => ports::HandlerOutcome::plain(self.outcome.clone()),
            })
        }

        fn cache_report(
            &self,
            context: &WorkspaceContext,
            events: &[DomainEvent],
            dry_run: bool,
            _cache_access: CacheAccess,
        ) -> Result<CacheReport, String> {
            Ok(CacheReport {
                mode: if events.is_empty() {
                    "read".to_string()
                } else if dry_run {
                    "dry-run".to_string()
                } else {
                    "applied".to_string()
                },
                root: context.cache_root.display().to_string(),
                workspace_epoch: context.workspace_epoch,
                events: events
                    .iter()
                    .map(|event| event.name().to_string())
                    .collect(),
                invalidated: Vec::new(),
                refreshed: Vec::new(),
                lazy_rebuilt: Vec::new(),
                stale: Vec::new(),
                fresh: Vec::new(),
            })
        }

        fn notify_invalidation(&self, _context: &WorkspaceContext, _events: &[DomainEvent]) {}
    }

    fn call_runtime_with_outcome(
        workspace: &std::path::Path,
        outcome: AdapterOutcome,
        operation: &str,
    ) -> OperationResult {
        let mut args = Map::new();
        args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        args.insert("dryRun".to_string(), Value::Bool(false));
        args.insert(
            "operation".to_string(),
            Value::String(operation.to_string()),
        );
        if operation == "load" {
            args.insert(
                "path".to_string(),
                Value::String("build/config.cf".to_string()),
            );
        }
        UnicaApplication::with_ports(Arc::new(FixedOutcomePorts {
            outcome,
            data: None,
        }))
        .call_tool("unica.runtime.execute", &args)
        .unwrap()
    }

    fn call_runtime_with_outcome_and_data(
        workspace: &std::path::Path,
        outcome: AdapterOutcome,
        data: Option<Value>,
    ) -> OperationResult {
        let mut args = json!({
            "cwd": workspace,
            "dryRun": false,
            "operation": "launch",
            "clientMode": "thin",
            "execute": "tests/Smoke.epf",
            "output": "build/smoke.out.log",
            "stderrOutput": "build/smoke.stderr.log",
            "waitForExit": true,
            "waitTimeoutMs": 30_000
        })
        .as_object()
        .unwrap()
        .clone();
        args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        UnicaApplication::with_ports(Arc::new(FixedOutcomePorts { outcome, data }))
            .call_tool("unica.runtime.execute", &args)
            .unwrap()
    }

    fn test_workspace_root(prefix: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("{prefix}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn temp_meta_compile_workspace(prefix: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("{prefix}-{}-{nanos}", std::process::id()));
        let workspace = root.join("workspace");
        let src = workspace.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            workspace.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(
            src.join("Configuration.xml"),
            support_test_configuration_xml("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"),
        )
        .unwrap();
        root
    }

    fn temp_scaffolded_configuration_workspace(prefix: &str) -> std::path::PathBuf {
        let root = test_workspace_root(prefix);
        let workspace = root.join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(
            workspace.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        let init = UnicaApplication::new()
            .call_tool(
                "unica.cf.init",
                &Map::from_iter([
                    (
                        "cwd".to_string(),
                        Value::String(workspace.display().to_string()),
                    ),
                    ("dryRun".to_string(), Value::Bool(false)),
                    ("Name".to_string(), json!("Demo")),
                    ("OutputDir".to_string(), json!("src")),
                ]),
            )
            .expect("cf.init must route through the public application boundary");
        assert!(init.ok, "{:?}", init.errors);
        root
    }

    fn write_scaffolded_configuration_fixture(
        config_path: &std::path::Path,
        child_objects: &[&str],
        trailer: &str,
    ) -> Vec<u8> {
        let mut config = std::fs::read_to_string(config_path).unwrap();
        let start_marker = "\t\t<ChildObjects>\n";
        let start = config.find(start_marker).unwrap();
        let end_marker = "\t\t</ChildObjects>";
        let end = config[start..].find(end_marker).unwrap() + start + end_marker.len();
        let children = child_objects
            .iter()
            .map(|child| format!("\t\t\t{child}\n"))
            .collect::<String>();
        config.replace_range(start..end, &format!("{start_marker}{children}{end_marker}"));
        if !trailer.is_empty() {
            config = config.replacen(
                "</MetaDataObject>",
                &format!("</MetaDataObject>{trailer}"),
                1,
            );
        }
        let bytes = config
            .replace("\r\n", "\n")
            .replace('\n', "\r\n")
            .into_bytes();
        std::fs::write(config_path, &bytes).unwrap();
        bytes
    }

    fn call_meta_compile(
        workspace: &std::path::Path,
        json_path: &std::path::Path,
    ) -> OperationResult {
        let mut args = Map::new();
        args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        args.insert("dryRun".to_string(), Value::Bool(false));
        args.insert(
            "JsonPath".to_string(),
            Value::String(json_path.display().to_string()),
        );
        args.insert("OutputDir".to_string(), Value::String("src".to_string()));
        UnicaApplication::new()
            .call_tool("unica.meta.compile", &args)
            .unwrap()
    }

    fn call_meta_validate(workspace: &std::path::Path, object_path: &str) -> OperationResult {
        let mut args = Map::new();
        args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        args.insert(
            "ObjectPath".to_string(),
            Value::String(object_path.to_string()),
        );
        UnicaApplication::new()
            .call_tool("unica.meta.validate", &args)
            .unwrap()
    }

    fn leading_utf8_bom_count(bytes: &[u8]) -> usize {
        bytes
            .chunks_exact(3)
            .take_while(|chunk| *chunk == [0xEF, 0xBB, 0xBF])
            .count()
    }

    fn assert_valid_root_uuid(xml: &str, tag_name: &str) {
        let uuid = metadata_root_uuid(xml, tag_name);
        assert!(
            uuid::Uuid::parse_str(&uuid).is_ok(),
            "{tag_name} root uuid is invalid: {uuid}"
        );
    }

    fn metadata_root_uuid(xml: &str, tag_name: &str) -> String {
        let marker = format!("<{tag_name} uuid=\"");
        let start = xml
            .find(&marker)
            .unwrap_or_else(|| panic!("missing root marker {marker}"))
            + marker.len();
        let end = xml[start..]
            .find('"')
            .unwrap_or_else(|| panic!("{tag_name} root uuid is not terminated"))
            + start;
        xml[start..end].to_string()
    }

    #[test]
    fn mutating_meta_edit_blocks_locked_vendor_object_by_default() {
        let root = std::env::temp_dir().join(format!("unica-meta-guard-{}", std::process::id()));
        let workspace = root.join("workspace");
        let src = workspace.join("src");
        let ext = src.join("Ext");
        let catalogs = src.join("Catalogs");
        std::fs::create_dir_all(&ext).unwrap();
        std::fs::create_dir_all(&catalogs).unwrap();
        std::fs::write(
            workspace.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(
            src.join("Configuration.xml"),
            support_test_configuration_xml("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"),
        )
        .unwrap();
        let object_path = catalogs.join("Items.xml");
        std::fs::write(
            &object_path,
            support_test_catalog_xml("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"),
        )
        .unwrap();
        std::fs::write(
            ext.join("ParentConfigurations.bin"),
            support_test_parent_configurations_bin(
                "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
                "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb",
                "cccccccc-cccc-cccc-cccc-cccccccccccc",
            ),
        )
        .unwrap();
        let before = std::fs::read_to_string(&object_path).unwrap();
        let mut args = Map::new();
        args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        args.insert("dryRun".to_string(), Value::Bool(false));
        args.insert(
            "ObjectPath".to_string(),
            Value::String("src/Catalogs/Items.xml".to_string()),
        );
        args.insert(
            "Operation".to_string(),
            Value::String("modify-property".to_string()),
        );
        args.insert(
            "Value".to_string(),
            Value::String("Name=Changed".to_string()),
        );

        let result = UnicaApplication::new()
            .call_tool("unica.meta.edit", &args)
            .unwrap();

        assert!(!result.ok);
        assert!(result.summary.contains("support guard"));
        assert!(result.errors.join("\n").contains("на замке"));
        assert!(result.cache.events.is_empty());
        assert_eq!(std::fs::read_to_string(&object_path).unwrap(), before);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn mutating_meta_edit_warn_mode_allows_locked_vendor_object_with_warning() {
        let root =
            std::env::temp_dir().join(format!("unica-meta-guard-warn-{}", std::process::id()));
        let workspace = root.join("workspace");
        let src = workspace.join("src");
        let ext = src.join("Ext");
        let catalogs = src.join("Catalogs");
        std::fs::create_dir_all(&ext).unwrap();
        std::fs::create_dir_all(&catalogs).unwrap();
        std::fs::write(
            workspace.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(
            workspace.join(".v8-project.json"),
            r#"{"editingAllowedCheck":"warn"}"#,
        )
        .unwrap();
        std::fs::write(
            src.join("Configuration.xml"),
            support_test_configuration_xml("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"),
        )
        .unwrap();
        let object_path = catalogs.join("Items.xml");
        std::fs::write(
            &object_path,
            support_test_catalog_xml("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"),
        )
        .unwrap();
        std::fs::write(
            ext.join("ParentConfigurations.bin"),
            support_test_parent_configurations_bin(
                "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
                "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb",
                "cccccccc-cccc-cccc-cccc-cccccccccccc",
            ),
        )
        .unwrap();
        let mut args = Map::new();
        args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        args.insert("dryRun".to_string(), Value::Bool(false));
        args.insert(
            "ObjectPath".to_string(),
            Value::String("src/Catalogs/Items.xml".to_string()),
        );
        args.insert(
            "Operation".to_string(),
            Value::String("modify-property".to_string()),
        );
        args.insert(
            "Value".to_string(),
            Value::String("Name=Changed".to_string()),
        );

        let result = UnicaApplication::new()
            .call_tool("unica.meta.edit", &args)
            .unwrap();

        assert!(result.ok);
        assert!(result.warnings.join("\n").contains("support guard"));
        assert!(std::fs::read_to_string(&object_path)
            .unwrap()
            .contains("<Name>Changed</Name>"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn mutating_meta_remove_blocks_supported_object_until_off_support() {
        let root =
            std::env::temp_dir().join(format!("unica-meta-guard-remove-{}", std::process::id()));
        let workspace = root.join("workspace");
        let src = workspace.join("src");
        let ext = src.join("Ext");
        let catalogs = src.join("Catalogs");
        std::fs::create_dir_all(&ext).unwrap();
        std::fs::create_dir_all(&catalogs).unwrap();
        std::fs::write(
            workspace.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(
            src.join("Configuration.xml"),
            support_test_configuration_xml("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"),
        )
        .unwrap();
        let object_path = catalogs.join("Items.xml");
        std::fs::write(
            &object_path,
            support_test_catalog_xml("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"),
        )
        .unwrap();
        std::fs::write(
            ext.join("ParentConfigurations.bin"),
            support_test_parent_configurations_bin(
                "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
                "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb",
                "cccccccc-cccc-cccc-cccc-cccccccccccc",
            ),
        )
        .unwrap();
        let mut args = Map::new();
        args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        args.insert("dryRun".to_string(), Value::Bool(false));
        args.insert("ConfigDir".to_string(), Value::String("src".to_string()));
        args.insert(
            "Object".to_string(),
            Value::String("Catalog.Items".to_string()),
        );

        let result = UnicaApplication::new()
            .call_tool("unica.meta.remove", &args)
            .unwrap();

        assert!(!result.ok);
        assert!(result.summary.contains("support guard"));
        assert!(result.errors.join("\n").contains("не снят с поддержки"));
        assert!(object_path.exists());

        let _ = std::fs::remove_dir_all(root);
    }

    fn support_test_configuration_xml(uuid: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20">
  <Configuration uuid="{uuid}">
    <InternalInfo/>
    <Properties>
      <Name>Demo</Name>
      <Synonym><v8:item><v8:lang>ru</v8:lang><v8:content>Demo</v8:content></v8:item></Synonym>
      <Version>1.0</Version>
      <Vendor>Vendor</Vendor>
      <CompatibilityMode>Version8_3_24</CompatibilityMode>
      <DefaultRunMode>ManagedApplication</DefaultRunMode>
      <ScriptVariant>Russian</ScriptVariant>
      <DefaultLanguage>Russian</DefaultLanguage>
      <DataLockControlMode>Managed</DataLockControlMode>
      <ModalityUseMode>DontUse</ModalityUseMode>
      <InterfaceCompatibilityMode>Taxi</InterfaceCompatibilityMode>
    </Properties>
    <ChildObjects><Language>Russian</Language><Catalog>Items</Catalog></ChildObjects>
  </Configuration>
</MetaDataObject>"#
        )
    }

    fn support_test_catalog_xml(uuid: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20">
  <Catalog uuid="{uuid}">
    <Properties>
      <Name>Items</Name>
      <Synonym><v8:item><v8:lang>ru</v8:lang><v8:content>Items</v8:content></v8:item></Synonym>
    </Properties>
    <ChildObjects/>
  </Catalog>
</MetaDataObject>"#
        )
    }

    fn support_test_form_xml() -> &'static str {
        r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20">
  <Form uuid="dddddddd-dddd-dddd-dddd-dddddddddddd">
    <Properties>
      <Name>Main</Name>
      <FormType>Managed</FormType>
    </Properties>
  </Form>
</MetaDataObject>"#
    }

    fn support_test_workspace(
        prefix: &str,
        parent_configurations_bin: String,
    ) -> (PathBuf, PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("{prefix}-{}", std::process::id()));
        let workspace = root.join("workspace");
        let src = workspace.join("src");
        let ext = src.join("Ext");
        let catalogs = src.join("Catalogs");
        std::fs::create_dir_all(&ext).unwrap();
        std::fs::create_dir_all(&catalogs).unwrap();
        std::fs::write(
            workspace.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(
            src.join("Configuration.xml"),
            support_test_configuration_xml("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"),
        )
        .unwrap();
        std::fs::write(
            catalogs.join("Items.xml"),
            support_test_catalog_xml("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"),
        )
        .unwrap();
        let bin_path = ext.join("ParentConfigurations.bin");
        std::fs::write(&bin_path, parent_configurations_bin).unwrap();
        (root, workspace, bin_path)
    }

    fn support_test_parent_configurations_bin(
        config_uuid: &str,
        locked_uuid: &str,
        removed_uuid: &str,
    ) -> String {
        format!(
            "\u{feff}{{6,0,1,dddddddd-dddd-dddd-dddd-dddddddddddd,0,eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee,\"1.0\",\"Vendor\",\"VendorConf\",3,1,0,{config_uuid},{config_uuid},0,0,{locked_uuid},{locked_uuid},2,0,{removed_uuid},{removed_uuid}}}"
        )
    }

    #[test]
    fn native_xml_metadata_tools_reject_edt_source_set_targets() {
        let root =
            std::env::temp_dir().join(format!("unica-xml-tool-edt-guard-{}", std::process::id()));
        let workspace = root.join("workspace");
        std::fs::create_dir_all(workspace.join("src/Configuration")).unwrap();
        std::fs::write(
            workspace.join("v8project.yaml"),
            "format: EDT\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(workspace.join("src/.project"), "<projectDescription/>").unwrap();
        std::fs::write(
            workspace.join("src/Configuration/Configuration.mdo"),
            "<mdclass:Configuration/>",
        )
        .unwrap();
        let mut args = Map::new();
        args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        args.insert(
            "ConfigPath".to_string(),
            Value::String("src/Configuration.xml".to_string()),
        );

        let error = match UnicaApplication::new().call_tool("unica.cf.info", &args) {
            Ok(result) => panic!("expected EDT source-set guard, got {}", result.summary),
            Err(error) => error,
        };

        assert!(error.contains("sourceFormat=edt"));
        assert!(error.contains("platform_xml"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn read_only_native_outfile_is_workspace_write_guarded() {
        let root = std::env::temp_dir().join(format!(
            "unica-read-outfile-write-guard-{}",
            std::process::id()
        ));
        let workspace = root.join("workspace");
        let outside = root.join("outside").join("report.txt");
        std::fs::create_dir_all(workspace.join("src")).unwrap();
        std::fs::create_dir_all(outside.parent().unwrap()).unwrap();
        std::fs::write(
            workspace.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(
            workspace.join("src/Configuration.xml"),
            support_test_configuration_xml("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"),
        )
        .unwrap();
        let mut args = Map::new();
        args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        args.insert(
            "ConfigPath".to_string(),
            Value::String("src/Configuration.xml".to_string()),
        );
        args.insert(
            "OutFile".to_string(),
            Value::String(outside.display().to_string()),
        );

        let error = match UnicaApplication::new().call_tool("unica.cf.info", &args) {
            Ok(result) => panic!("expected OutFile write guard, got {}", result.summary),
            Err(error) => error,
        };

        assert!(error.contains("outside workspace root"), "{error}");
        assert!(!outside.exists());

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn cfe_borrow_rejects_edt_config_source_set_target() {
        let root =
            std::env::temp_dir().join(format!("unica-cfe-borrow-edt-guard-{}", std::process::id()));
        let workspace = root.join("workspace");
        std::fs::create_dir_all(workspace.join("cfg/Configuration")).unwrap();
        std::fs::create_dir_all(workspace.join("ext")).unwrap();
        std::fs::write(
            workspace.join("v8project.yaml"),
            "format: EDT\nsource-set:\n  - name: cfg\n    type: CONFIGURATION\n    path: cfg\n  - name: ext\n    type: EXTENSION\n    path: ext\n",
        )
        .unwrap();
        std::fs::write(workspace.join("cfg/.project"), "<projectDescription/>").unwrap();
        std::fs::write(
            workspace.join("cfg/Configuration/Configuration.mdo"),
            "<mdclass:Configuration/>",
        )
        .unwrap();
        std::fs::write(
            workspace.join("ext/Configuration.xml"),
            support_test_configuration_xml("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"),
        )
        .unwrap();
        let mut args = Map::new();
        args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        args.insert("dryRun".to_string(), Value::Bool(false));
        args.insert(
            "ExtensionPath".to_string(),
            Value::String("ext/Configuration.xml".to_string()),
        );
        args.insert(
            "ConfigPath".to_string(),
            Value::String("cfg/Configuration.xml".to_string()),
        );
        args.insert(
            "Object".to_string(),
            Value::String("Catalog.Items".to_string()),
        );

        let error = match UnicaApplication::new().call_tool("unica.cfe.borrow", &args) {
            Ok(result) => panic!("expected EDT source-set guard, got {}", result.summary),
            Err(error) => error,
        };

        assert!(error.contains("source-set `cfg`"), "{error}");
        assert!(error.contains("sourceFormat=edt"), "{error}");

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn native_operations_rs_is_thin_facade_not_xml_dsl_monolith() {
        let infrastructure_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("infrastructure");
        let path = infrastructure_dir.join("native_operations.rs");
        let text = std::fs::read_to_string(&path).unwrap();
        let line_count = text.lines().count();

        assert!(
            line_count < 200,
            "native_operations.rs must stay a thin facade; got {line_count} lines"
        );
        assert!(
            !text.contains("match operation"),
            "operation-specific XML/DSL dispatch belongs in backend modules"
        );
        assert!(
            !infrastructure_dir
                .join("native_operations_backend.rs")
                .exists(),
            "native_operations_backend.rs must not return; split operation logic by family under native_operations/"
        );
    }

    #[test]
    fn mutating_native_operation_rejects_output_escape_before_backend_execution() {
        let root =
            std::env::temp_dir().join(format!("unica-app-path-policy-{}", std::process::id()));
        let workspace = root.join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let mut args = Map::new();
        args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        args.insert("dryRun".to_string(), Value::Bool(false));
        args.insert("Name".to_string(), Value::String("PathPolicy".to_string()));
        args.insert(
            "OutputDir".to_string(),
            Value::String("../outside".to_string()),
        );

        let error = match UnicaApplication::new().call_tool("unica.cf.init", &args) {
            Ok(result) => panic!("expected path policy error, got {}", result.summary),
            Err(error) => error,
        };

        assert!(error.contains("outside workspace root"));
        assert!(!root.join("outside").exists());

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn detailed_compile_dry_run_rejects_output_escape_like_apply() {
        let root = temp_meta_compile_workspace("unica-compile-preview-path-policy");
        let workspace = root.join("workspace");
        let json_path = workspace.join("module.json");
        std::fs::write(
            &json_path,
            r#"{"type":"CommonModule","name":"PreviewPathPolicy"}"#,
        )
        .unwrap();
        let mut args = Map::new();
        args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        args.insert("dryRun".to_string(), Value::Bool(true));
        args.insert(
            "JsonPath".to_string(),
            Value::String(json_path.display().to_string()),
        );
        args.insert(
            "OutputDir".to_string(),
            Value::String("../outside".to_string()),
        );

        let error = UnicaApplication::new()
            .call_tool("unica.meta.compile", &args)
            .expect_err("preview must enforce the same output path policy as apply");

        assert!(error.contains("outside workspace root"), "{error}");
        assert!(!root.join("outside").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn form_compile_dry_run_rejects_output_escape_like_apply() {
        let root = test_workspace_root("unica-form-compile-preview-path-policy");
        let workspace = root.join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let json_path = workspace.join("form.json");
        std::fs::write(&json_path, "{}").unwrap();
        let mut args = Map::new();
        args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        args.insert("dryRun".to_string(), Value::Bool(true));
        args.insert(
            "JsonPath".to_string(),
            Value::String(json_path.display().to_string()),
        );
        args.insert(
            "OutputPath".to_string(),
            Value::String("../outside.xml".to_string()),
        );

        let error = UnicaApplication::new()
            .call_tool("unica.form.compile", &args)
            .expect_err("form preview must enforce the same output path policy as apply");

        assert!(error.contains("outside workspace root"), "{error}");
        assert!(!root.join("outside.xml").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn detailed_compile_dry_run_rejects_edt_source_set_like_apply() {
        let root = test_workspace_root("unica-compile-preview-edt-guard");
        let workspace = root.join("workspace");
        std::fs::create_dir_all(workspace.join("src/Configuration")).unwrap();
        std::fs::write(
            workspace.join("v8project.yaml"),
            "format: EDT\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(workspace.join("src/.project"), "<projectDescription/>").unwrap();
        std::fs::write(
            workspace.join("src/Configuration/Configuration.mdo"),
            "<mdclass:Configuration/>",
        )
        .unwrap();
        let json_path = workspace.join("module.json");
        std::fs::write(
            &json_path,
            r#"{"type":"CommonModule","name":"PreviewEdtGuard"}"#,
        )
        .unwrap();
        let mut args = Map::new();
        args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        args.insert("dryRun".to_string(), Value::Bool(true));
        args.insert(
            "JsonPath".to_string(),
            Value::String(json_path.display().to_string()),
        );
        args.insert("OutputDir".to_string(), Value::String("src".to_string()));

        let error = UnicaApplication::new()
            .call_tool("unica.meta.compile", &args)
            .expect_err("preview must enforce the same source-format guard as apply");

        assert!(error.contains("sourceFormat=edt"), "{error}");
        assert!(error.contains("platform_xml"), "{error}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn form_compile_dry_run_rejects_edt_source_set_like_apply() {
        let root = test_workspace_root("unica-form-compile-preview-edt-guard");
        let workspace = root.join("workspace");
        std::fs::create_dir_all(workspace.join("src/Configuration")).unwrap();
        std::fs::write(
            workspace.join("v8project.yaml"),
            "format: EDT\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(workspace.join("src/.project"), "<projectDescription/>").unwrap();
        std::fs::write(
            workspace.join("src/Configuration/Configuration.mdo"),
            "<mdclass:Configuration/>",
        )
        .unwrap();
        let json_path = workspace.join("form.json");
        std::fs::write(&json_path, "{}").unwrap();
        let mut args = Map::new();
        args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        args.insert("dryRun".to_string(), Value::Bool(true));
        args.insert(
            "JsonPath".to_string(),
            Value::String(json_path.display().to_string()),
        );
        args.insert(
            "OutputPath".to_string(),
            Value::String("src/Form.xml".to_string()),
        );

        let error = UnicaApplication::new()
            .call_tool("unica.form.compile", &args)
            .expect_err("form preview must enforce the same source-format guard as apply");

        assert!(error.contains("sourceFormat=edt"), "{error}");
        assert!(error.contains("platform_xml"), "{error}");
        assert!(!workspace.join("src/Form.xml").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn detailed_compile_dry_run_reports_planner_errors_instead_of_masking_them() {
        let root = temp_meta_compile_workspace("unica-compile-preview-error-parity");
        let workspace = root.join("workspace");
        let config_path = workspace.join("src/Configuration.xml");
        let config_before = std::fs::read(&config_path).unwrap();
        let json_path = workspace.join("invalid.json");
        std::fs::write(
            &json_path,
            r#"{"type":"UnknownMetadata","name":"InvalidPreview"}"#,
        )
        .unwrap();
        let mut args = Map::new();
        args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        args.insert("dryRun".to_string(), Value::Bool(true));
        args.insert(
            "JsonPath".to_string(),
            Value::String(json_path.display().to_string()),
        );
        args.insert("OutputDir".to_string(), Value::String("src".to_string()));

        let result = UnicaApplication::new()
            .call_tool("unica.meta.compile", &args)
            .unwrap();

        assert!(!result.ok, "{result:?}");
        assert!(result.summary.contains("dry run"), "{}", result.summary);
        assert!(
            result.errors.join("\n").contains("UnknownMetadata"),
            "{:?}",
            result.errors
        );
        assert!(result.changes.is_empty());
        assert!(result.artifacts.is_empty());
        assert!(result.cache.events.is_empty());
        assert_eq!(std::fs::read(&config_path).unwrap(), config_before);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn detailed_compile_dry_run_applies_the_same_support_guard_as_apply() {
        let root = temp_meta_compile_workspace("unica-compile-preview-support-guard");
        let workspace = root.join("workspace");
        let src = workspace.join("src");
        let ext = src.join("Ext");
        std::fs::create_dir_all(&ext).unwrap();
        std::fs::write(
            ext.join("ParentConfigurations.bin"),
            support_test_parent_configurations_bin(
                "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
                "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb",
                "cccccccc-cccc-cccc-cccc-cccccccccccc",
            )
            .replace("{6,0,", "{6,1,"),
        )
        .unwrap();
        let config_path = src.join("Configuration.xml");
        let config_before = std::fs::read(&config_path).unwrap();
        let json_path = workspace.join("module.json");
        std::fs::write(
            &json_path,
            r#"{"type":"CommonModule","name":"PreviewSupportGuard"}"#,
        )
        .unwrap();
        let mut args = Map::new();
        args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        args.insert("dryRun".to_string(), Value::Bool(true));
        args.insert(
            "JsonPath".to_string(),
            Value::String(json_path.display().to_string()),
        );
        args.insert("OutputDir".to_string(), Value::String("src".to_string()));

        let result = UnicaApplication::new()
            .call_tool("unica.meta.compile", &args)
            .unwrap();

        assert!(!result.ok, "{result:?}");
        assert!(result.summary.contains("support guard"), "{result:?}");
        assert!(result.cache.events.is_empty(), "{result:?}");
        assert_eq!(std::fs::read(&config_path).unwrap(), config_before);
        assert!(!src.join("CommonModules/PreviewSupportGuard.xml").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn external_init_preview_is_path_guarded_and_source_set_typed() {
        let root = std::env::temp_dir().join(format!(
            "unica-external-init-contract-{}",
            std::process::id()
        ));
        let workspace = root.join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(
            workspace.join("v8project.yaml"),
            concat!(
                "format: DESIGNER\n",
                "source-set:\n",
                "  - name: processors\n",
                "    type: EXTERNAL_DATA_PROCESSORS\n",
                "    path: epf\n",
                "  - name: reports\n",
                "    type: EXTERNAL_REPORTS\n",
                "    path: erf\n",
                "  - name: russian-processors\n",
                "    type: EXTERNAL_DATA_PROCESSORS\n",
                "    path: епф\n",
            ),
        )
        .unwrap();

        let mut args = Map::new();
        args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        args.insert("dryRun".to_string(), Value::Bool(true));
        args.insert("Name".to_string(), Value::String("Preview".to_string()));
        args.insert("OutputDir".to_string(), Value::String("epf".to_string()));

        let preview = UnicaApplication::new()
            .call_tool("unica.epf.init", &args)
            .unwrap();
        assert!(preview.ok, "{:?}", preview.errors);
        assert_eq!(preview.artifacts.len(), 2);
        assert!(!workspace.join("epf").exists());

        args.insert("OutputDir".to_string(), Value::String("EPF".to_string()));
        let error = UnicaApplication::new()
            .call_tool("unica.epf.init", &args)
            .unwrap_err();
        assert!(error.contains("exact source-set root"), "{error}");
        assert!(!workspace.join("EPF").exists());

        args.insert("OutputDir".to_string(), Value::String("ЕПФ".to_string()));
        let error = UnicaApplication::new()
            .call_tool("unica.epf.init", &args)
            .unwrap_err();
        assert!(error.contains("exact source-set root"), "{error}");
        assert!(!workspace.join("ЕПФ").exists());

        args.insert(
            "OutputDir".to_string(),
            Value::String("epf/nested".to_string()),
        );
        let error = UnicaApplication::new()
            .call_tool("unica.epf.init", &args)
            .unwrap_err();
        assert!(error.contains("source-set root"), "{error}");
        assert!(!workspace.join("epf").exists());

        args.insert("OutputDir".to_string(), Value::String("erf".to_string()));
        let error = UnicaApplication::new()
            .call_tool("unica.epf.init", &args)
            .unwrap_err();
        assert!(error.contains("source-set `reports`"), "{error}");
        assert!(error.contains("ExternalReport"), "{error}");
        assert!(!workspace.join("erf").exists());

        args.insert(
            "OutputDir".to_string(),
            Value::String("../outside".to_string()),
        );
        let error = UnicaApplication::new()
            .call_tool("unica.epf.init", &args)
            .unwrap_err();
        assert!(error.contains("outside workspace root"), "{error}");
        assert!(!root.join("outside").exists());

        std::fs::write(
            workspace.join("v8project.yaml"),
            concat!(
                "format: DESIGNER\n",
                "source-set:\n",
                "  - name: configuration\n",
                "    type: CONFIGURATION\n",
                "    path: .\n",
            ),
        )
        .unwrap();
        args.insert(
            "OutputDir".to_string(),
            Value::String("external/epf".to_string()),
        );
        let preview = UnicaApplication::new()
            .call_tool("unica.epf.init", &args)
            .unwrap();
        assert!(preview.ok, "{:?}", preview.errors);
        assert_eq!(preview.artifacts.len(), 2);
        assert!(!workspace.join("external").exists());

        args.insert("OutputDir".to_string(), Value::String(".".to_string()));
        let error = UnicaApplication::new()
            .call_tool("unica.epf.init", &args)
            .unwrap_err();
        assert!(error.contains("source-set `configuration`"), "{error}");
        assert!(error.contains("Configuration"), "{error}");

        std::fs::write(
            workspace.join("v8project.yaml"),
            concat!(
                "format: DESIGNER\n",
                "source-set:\n",
                "  - name: configuration\n",
                "    type: CONFIGURATION\n",
                "    path: src\n",
            ),
        )
        .unwrap();
        args.insert("OutputDir".to_string(), Value::String("SRC".to_string()));
        let error = UnicaApplication::new()
            .call_tool("unica.epf.init", &args)
            .unwrap_err();
        assert!(error.contains("exact source-set root"), "{error}");
        assert!(!workspace.join("SRC").exists());

        std::fs::write(
            workspace.join("v8project.yaml"),
            concat!(
                "format: EDT\n",
                "source-set:\n",
                "  - name: processors\n",
                "    type: EXTERNAL_DATA_PROCESSORS\n",
                "    path: epf\n",
            ),
        )
        .unwrap();
        std::fs::create_dir_all(workspace.join("epf")).unwrap();
        std::fs::write(
            workspace.join("epf/Existing.xml"),
            "<MetaDataObject><ExternalDataProcessor/></MetaDataObject>",
        )
        .unwrap();
        args.insert("OutputDir".to_string(), Value::String("epf".to_string()));
        let error = UnicaApplication::new()
            .call_tool("unica.epf.init", &args)
            .unwrap_err();
        assert!(error.contains("format=DESIGNER"), "{error}");
        assert!(!workspace.join("epf/Preview.xml").exists());

        std::fs::write(
            workspace.join("v8project.yaml"),
            concat!(
                "format: designer\n",
                "source-set:\n",
                "  - name: processors\n",
                "    type: EXTERNAL_DATA_PROCESSORS\n",
                "    path: epf\n",
            ),
        )
        .unwrap();
        let error = UnicaApplication::new()
            .call_tool("unica.epf.init", &args)
            .unwrap_err();
        assert!(error.contains("exact `DESIGNER`"), "{error}");
        assert!(!workspace.join("epf/Preview.xml").exists());

        std::fs::write(
            workspace.join("v8project.yaml"),
            concat!(
                "format: true\n",
                "source-set:\n",
                "  - name: processors\n",
                "    type: EXTERNAL_DATA_PROCESSORS\n",
                "    path: epf\n",
            ),
        )
        .unwrap();
        let error = UnicaApplication::new()
            .call_tool("unica.epf.init", &args)
            .unwrap_err();
        assert!(error.contains("field `format` must be a string"), "{error}");
        assert!(!workspace.join("epf/Preview.xml").exists());

        let _ = std::fs::remove_dir_all(root);
    }
}
