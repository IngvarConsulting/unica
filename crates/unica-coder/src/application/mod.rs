use crate::domain::cache::{CacheAccess, CacheReport};
use crate::domain::events::{DomainEvent, DomainEventKind};
use crate::domain::project_sources::discover_project_source_map;
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::native_operations::common::{
    absolutize, path_arg, required_string, support_guard_violation, SupportGuardRequirement,
    SupportGuardViolation,
};
use crate::infrastructure::native_operations::{meta, template};
use crate::infrastructure::AdapterOutcome;
use operation_descriptors::SupportGuardPolicy;
use ports::{ApplicationPorts, DefaultApplicationPorts};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::env;
use std::path::{Path, PathBuf};

mod operation_descriptors;
mod ports;
mod tool_contracts;
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
    CodeAdapter {
        command: &'static [&'static str],
    },
    StandardsAdapter {
        operation: &'static str,
    },
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
}

pub struct UnicaApplication {
    ports: Box<dyn ApplicationPorts>,
}

impl UnicaApplication {
    pub fn new() -> Self {
        Self {
            ports: Box::new(DefaultApplicationPorts),
        }
    }

    #[cfg(test)]
    fn with_ports(ports: Box<dyn ApplicationPorts>) -> Self {
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
        let spec = tools()
            .into_iter()
            .find(|tool| tool.name == name)
            .ok_or_else(|| format!("unknown unica tool: {name}"))?;
        call_tool(spec, args, self.ports.as_ref())
    }
}

impl Default for UnicaApplication {
    fn default() -> Self {
        Self::new()
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
) -> Result<OperationResult, String> {
    let dry_run = args
        .get("dryRun")
        .and_then(Value::as_bool)
        .unwrap_or(spec.mutating);
    tool_contracts::validate_tool_arguments(spec, args, dry_run)?;
    let cwd = args
        .get("cwd")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .unwrap_or(
            env::current_dir().map_err(|err| format!("failed to read current directory: {err}"))?,
        );
    let context = ports.discover_workspace(cwd)?;
    tool_contracts::validate_workspace_paths(spec, args, dry_run, &context)?;
    tool_contracts::validate_native_source_set_format(spec, args, dry_run, &context)?;
    let index_report = crate::infrastructure::workspace_index::IndexStartReport::default();
    let support_guard_warning = if spec.mutating && !dry_run {
        match support_guard_check(spec, args, &context)? {
            SupportGuardCheck::Allow => None,
            SupportGuardCheck::Warn(warning) => Some(warning),
            SupportGuardCheck::Block(mut outcome) => {
                outcome.warnings.extend(index_report.warnings);
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
                });
            }
        }
    } else {
        None
    };

    let mut outcome = ports.invoke_handler(spec, args, &context, dry_run)?;
    if let Some(warning) = support_guard_warning {
        outcome.warnings.insert(0, warning);
    }
    outcome.warnings.extend(index_report.warnings);

    let events = if should_emit_events(spec, dry_run, &outcome) {
        domain_events(spec, args)
    } else {
        Vec::new()
    };
    let cache = ports.cache_report(&context, &events, dry_run, spec.cache_access)?;
    if spec.mutating && !dry_run && outcome.ok && !events.is_empty() {
        ports.notify_invalidation(&context, &events);
    }
    let diagnostics = runtime_result_diagnostics(spec, args, &context, &outcome);

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
    })
}

fn should_emit_events(spec: ToolSpec, dry_run: bool, outcome: &AdapterOutcome) -> bool {
    spec.mutating && (dry_run || (outcome.ok && !outcome.changes.is_empty()))
}

fn runtime_result_diagnostics(
    spec: ToolSpec,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    outcome: &AdapterOutcome,
) -> Option<Value> {
    if !matches!(spec.handler, ToolHandler::RuntimeAdapter) || outcome.ok {
        return None;
    }
    let operation = args
        .get("operation")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
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

enum SupportGuardCheck {
    Allow,
    Warn(String),
    Block(AdapterOutcome),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SupportGuardMode {
    Deny,
    Warn,
    Off,
}

fn support_guard_check(
    spec: ToolSpec,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<SupportGuardCheck, String> {
    let Some((target_path, requirement)) = support_guard_target(spec, args, context) else {
        return Ok(SupportGuardCheck::Allow);
    };
    let Some(violation) = support_guard_violation(&target_path, requirement) else {
        return Ok(SupportGuardCheck::Allow);
    };

    Ok(match support_guard_mode(&violation.config_dir, context) {
        SupportGuardMode::Off => SupportGuardCheck::Allow,
        SupportGuardMode::Warn => SupportGuardCheck::Warn(format!(
            "[support guard] ПРЕДУПРЕЖДЕНИЕ: {}. Цель: {}",
            violation.reason,
            violation.target_path.display()
        )),
        SupportGuardMode::Deny => {
            SupportGuardCheck::Block(support_guard_blocked_outcome(spec, &violation, requirement))
        }
    })
}

fn support_guard_target(
    spec: ToolSpec,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Option<(PathBuf, SupportGuardRequirement)> {
    let ToolHandler::NativeOperation { operation, .. } = spec.handler else {
        return None;
    };
    let policy = operation_descriptors::native_operation_descriptor(operation)?.support_guard?;
    match policy {
        SupportGuardPolicy::PathArgs { names, requirement } => {
            support_guard_path_arg(args, context, names, requirement)
        }
        SupportGuardPolicy::MetaRemove { requirement } => {
            support_guard_meta_remove_target(args, context).map(|path| (path, requirement))
        }
        SupportGuardPolicy::ObjectName { requirement } => {
            support_guard_object_name_target(args, context).map(|path| (path, requirement))
        }
    }
}

fn support_guard_path_arg(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    names: &[&str],
    requirement: SupportGuardRequirement,
) -> Option<(PathBuf, SupportGuardRequirement)> {
    path_arg(args, names).map(|path| (absolutize(path, &context.cwd), requirement))
}

fn support_guard_meta_remove_target(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Option<PathBuf> {
    let config_dir = path_arg(args, &["configDir", "ConfigDir"])?;
    let object = required_string(args, &["object", "Object"], "Object").ok()?;
    let (object_type, object_name) = object.split_once('.')?;
    let type_dir = meta::meta_remove_type_plural(object_type)?;
    Some(
        absolutize(config_dir, &context.cwd)
            .join(type_dir)
            .join(format!("{object_name}.xml")),
    )
}

fn support_guard_object_name_target(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Option<PathBuf> {
    let object_name = required_string(
        args,
        &["objectName", "ObjectName", "processorName", "ProcessorName"],
        "ObjectName",
    )
    .ok()?;
    let src_dir = path_arg(args, &["srcDir", "SrcDir"]).unwrap_or_else(|| PathBuf::from("src"));
    let src_dir = absolutize(src_dir, &context.cwd);
    let direct = src_dir.join(format!("{object_name}.xml"));
    if direct.exists() {
        return Some(direct);
    }
    for folder in template::template_add_object_type_folders() {
        let candidate = src_dir.join(folder).join(format!("{object_name}.xml"));
        if candidate.exists() {
            return Some(candidate);
        }
    }
    Some(direct)
}

fn support_guard_mode(config_dir: &Path, context: &WorkspaceContext) -> SupportGuardMode {
    let Some(project_file) = find_v8_project_file(&context.cwd)
        .or_else(|| find_v8_project_file(config_dir))
        .or_else(|| find_v8_project_file(&context.workspace_root))
    else {
        return SupportGuardMode::Deny;
    };
    let Ok(text) = std::fs::read_to_string(&project_file) else {
        return SupportGuardMode::Deny;
    };
    let Ok(project) = serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')) else {
        return SupportGuardMode::Deny;
    };
    let project_dir = project_file.parent().unwrap_or_else(|| Path::new(""));
    let config_dir = normalize_guard_path(config_dir);

    if let Some(databases) = project.get("databases").and_then(Value::as_array) {
        for database in databases {
            let Some(config_src) = database.get("configSrc").and_then(Value::as_str) else {
                continue;
            };
            let config_src = PathBuf::from(config_src);
            let config_src = if config_src.is_absolute() {
                config_src
            } else {
                project_dir.join(config_src)
            };
            let config_src = normalize_guard_path(&config_src);
            if (config_dir == config_src || config_dir.starts_with(&config_src))
                && database
                    .get("editingAllowedCheck")
                    .and_then(Value::as_str)
                    .is_some()
            {
                return support_guard_mode_value(
                    database
                        .get("editingAllowedCheck")
                        .and_then(Value::as_str)
                        .expect("checked above"),
                );
            }
        }
    }

    project
        .get("editingAllowedCheck")
        .and_then(Value::as_str)
        .map(support_guard_mode_value)
        .unwrap_or(SupportGuardMode::Deny)
}

fn find_v8_project_file(start: &Path) -> Option<PathBuf> {
    let mut current = if start.is_dir() {
        start.to_path_buf()
    } else {
        start.parent()?.to_path_buf()
    };
    for _ in 0..20 {
        let candidate = current.join(".v8-project.json");
        if candidate.is_file() {
            return Some(candidate);
        }
        let Some(parent) = current.parent() else {
            break;
        };
        if parent == current {
            break;
        }
        current = parent.to_path_buf();
    }
    None
}

fn support_guard_mode_value(value: &str) -> SupportGuardMode {
    match value {
        "warn" => SupportGuardMode::Warn,
        "off" => SupportGuardMode::Off,
        _ => SupportGuardMode::Deny,
    }
}

fn normalize_guard_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn support_guard_blocked_outcome(
    spec: ToolSpec,
    violation: &SupportGuardViolation,
    requirement: SupportGuardRequirement,
) -> AdapterOutcome {
    let target = violation.target_path.display();
    let head = "[support-guard] Редактирование отклонено: это объект типовой конфигурации на поддержке поставщика, прямое редактирование молча сломает будущие обновления.";
    let cfe = "Рекомендуемый путь: внести доработку в расширение (навыки cfe-borrow / cfe-patch-method) — состояние поддержки менять не нужно, обновления вендора сохраняются.";
    let off_note =
        "Снять проверку для этой базы: editingAllowedCheck = warn|off в .v8-project.json.";
    let (state, fix) = match violation.code {
        "capability-off" => (
            format!(
                "Состояние: у всей конфигурации выключена возможность изменения (режим read-only «из коробки») — поэтому объект «{target}» редактировать нельзя."
            ),
            format!(
                "Либо снять защиту явно (навык support-edit, два шага):\n  support-edit -Path \"{}\" -Capability on — включить возможность изменения (объекты пока остаются на замке);\n  support-edit -Path \"{target}\" -Set editable — открыть этот объект для редактирования.\n  Изменение применяется в базу полной загрузкой выгрузки и обходит механизм обновлений вендора.",
                violation.config_dir.display()
            ),
        ),
        "not-removed" if requirement == SupportGuardRequirement::Removed => (
            format!(
                "Состояние: объект «{target}» на поддержке (не снят с поддержки) — его удаление разорвёт обновления вендора."
            ),
            format!(
                "Либо сначала снять объект с поддержки, затем удалять:\n  support-edit -Path \"{target}\" -Set off-support — объект уходит из-под обновлений, после этого удаление безопасно."
            ),
        ),
        _ => (
            format!(
                "Состояние: объект «{target}» на замке (возможность изменения конфигурации включена, но сам объект не редактируется)."
            ),
            format!(
                "Либо разрешить редактирование этого объекта (навык support-edit, выбрать одно):\n  support-edit -Path \"{target}\" -Set editable — редактировать и дальше получать обновления вендора (возможны конфликты слияния);\n  support-edit -Path \"{target}\" -Set off-support — снять с поддержки: обновления по объекту больше не приходят."
            ),
        ),
    };
    let message = format!("{head}\n{state}\n{cfe}\n{fix}\n{off_note}");
    AdapterOutcome {
        ok: false,
        summary: format!("{} blocked by support guard", spec.name),
        changes: Vec::new(),
        warnings: Vec::new(),
        errors: vec![message.clone()],
        artifacts: vec![violation.target_path.display().to_string()],
        stdout: None,
        stderr: Some(format!("{message}\n")),
        command: None,
    }
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
        _ => Vec::new(),
    }
}

fn runtime_event(args: &Map<String, Value>) -> Option<DomainEventKind> {
    match args.get("operation").and_then(Value::as_str)? {
        "config-init" | "init" | "convert" | "dump" => Some(DomainEventKind::SourceSetChanged),
        "build" | "load" | "extensions" | "test" => Some(DomainEventKind::BuildCompleted),
        "make" | "syntax" | "launch" => None,
        _ => None,
    }
}

fn project_status(context: &WorkspaceContext) -> AdapterOutcome {
    let source_map = discover_project_source_map(&context.workspace_root);
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
    outcome
}

fn project_map(context: &WorkspaceContext) -> AdapterOutcome {
    match discover_project_source_map(&context.workspace_root) {
        Ok(source_map) => {
            let mut outcome = AdapterOutcome::ok(format!(
                "project map discovered {} source set(s)",
                source_map.source_sets.len()
            ));
            outcome.stdout =
                Some(serde_json::to_string_pretty(&source_map).expect("source map serializes"));
            outcome
        }
        Err(error) => AdapterOutcome {
            ok: false,
            summary: "project map discovery failed".to_string(),
            changes: Vec::new(),
            warnings: Vec::new(),
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
            description: "Generate a CFE method interceptor.",
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
            description: "Edit managed Form.xml elements, attributes, and commands.",
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
            name: "unica.skd.compile",
            description: "Compile Data Composition Schema XML from JSON DSL.",
            mutating: true,
            cache_access: cache_access_for("skd-compile", Some(DomainEventKind::SkdChanged)),
            handler: ToolHandler::NativeOperation {
                operation: "skd-compile",
                event: Some(DomainEventKind::SkdChanged),
            },
        },
        ToolSpec {
            name: "unica.skd.edit",
            description: "Edit Data Composition Schema Template.xml.",
            mutating: true,
            cache_access: cache_access_for("skd-edit", Some(DomainEventKind::SkdChanged)),
            handler: ToolHandler::NativeOperation {
                operation: "skd-edit",
                event: Some(DomainEventKind::SkdChanged),
            },
        },
        ToolSpec {
            name: "unica.skd.info",
            description: "Inspect Data Composition Schema Template.xml.",
            mutating: false,
            cache_access: cache_access_for("skd-info", None),
            handler: ToolHandler::NativeOperation {
                operation: "skd-info",
                event: None,
            },
        },
        ToolSpec {
            name: "unica.skd.validate",
            description: "Validate Data Composition Schema Template.xml.",
            mutating: false,
            cache_access: cache_access_for("skd-validate", None),
            handler: ToolHandler::NativeOperation {
                operation: "skd-validate",
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
    } else if operation.starts_with("skd-") {
        CacheAccess {
            reads: &["metadata_graph", "skd_graph"],
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
    use serde_json::Map;
    use std::collections::HashSet;

    #[test]
    fn lists_unica_orchestrator_scope() {
        let names = tools().iter().map(|tool| tool.name).collect::<Vec<_>>();
        assert!(names.contains(&"unica.project.status"));
        assert!(names.contains(&"unica.project.map"));
        assert!(names.contains(&"unica.form.validate"));
        assert!(names.contains(&"unica.skd.edit"));
        assert!(names.contains(&"unica.mxl.compile"));
        assert!(names.contains(&"unica.role.validate"));
        assert!(names.contains(&"unica.support.edit"));
        assert!(names.contains(&"unica.epf.init"));
        assert!(names.contains(&"unica.erf.init"));
        assert!(names.contains(&"unica.build.load"));
        assert!(names.contains(&"unica.runtime.execute"));
        assert!(names.contains(&"unica.code.definition"));
        assert!(names.contains(&"unica.code.outline"));
        assert!(names.contains(&"unica.code.grep"));
        assert!(names.contains(&"unica.code.graph"));
        assert!(names.contains(&"unica.meta.profile"));
        assert!(names.contains(&"unica.standards.explain"));
        assert!(!names.contains(&"unica-coder"));
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

        assert!(!should_emit_events(spec, false, &outcome));

        outcome
            .changes
            .push("updated Configuration.xml".to_string());
        assert!(should_emit_events(spec, false, &outcome));
        assert!(should_emit_events(
            spec,
            true,
            &AdapterOutcome::ok("dry run")
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
            "unica.skd.compile",
            "unica.skd.edit",
            "unica.skd.info",
            "unica.skd.validate",
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
                && !tool.name.starts_with("unica.skd.")
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
    fn mutating_native_tools_have_registered_mutation_handlers() {
        let args = Map::new();
        for tool in tools() {
            if !tool.mutating {
                continue;
            }
            let ToolHandler::NativeOperation { operation, .. } = tool.handler else {
                continue;
            };
            let context = mutation_probe_context(operation);
            assert!(
                crate::infrastructure::native_operations::registry::invoke_mutation(
                    operation, tool.name, &args, &context
                )
                .is_some(),
                "{} routes to native mutation operation `{}` without a registered handler",
                tool.name,
                operation
            );
        }
    }

    fn mutation_probe_context(operation: &str) -> WorkspaceContext {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "unica-mutation-probe-{operation}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(root.join("src")).unwrap();
        WorkspaceContext::discover(root).unwrap()
    }

    #[test]
    fn form_and_skd_tools_route_through_native_handlers() {
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
                "unica.skd.compile",
                "skd-compile",
                Some(DomainEventKind::SkdChanged),
            ),
            (
                "unica.skd.edit",
                "skd-edit",
                Some(DomainEventKind::SkdChanged),
            ),
            ("unica.skd.info", "skd-info", None),
            ("unica.skd.validate", "skd-validate", None),
        ];
        for (tool_name, expected_operation, expected_event) in expected {
            let tool = tools()
                .into_iter()
                .find(|tool| tool.name == tool_name)
                .expect("form/SKD tool exists");

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
            let sidecar_before = b"sidecar content before failed batch";
            std::fs::write(&sidecar_path, sidecar_before).unwrap();

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
                "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.17\">\r\n",
                "\t<Configuration uuid=\"aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa\">\r\n",
                "\t\t<Properties>\r\n",
                "\t\t\t<Name>Issue55</Name>\r\n",
                "\t\t</Properties>\r\n",
                "\t\t<ChildObjects>\r\n",
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
    fn native_operation_descriptors_drive_required_schema() {
        for tool in tools() {
            let ToolHandler::NativeOperation { operation, .. } = tool.handler else {
                continue;
            };
            let descriptor = operation_descriptors::native_operation_descriptor(operation).unwrap();
            let schema = input_schema_for_tool(&tool);
            let required = schema["required"]
                .as_array()
                .expect("schema required is array")
                .iter()
                .map(|value| value.as_str().expect("required item is string"))
                .collect::<Vec<_>>();
            assert_eq!(required, descriptor.required_args, "{operation}");
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
    fn source_format_sensitive_descriptors_name_source_paths() {
        for operation in ["cf-info", "form-edit", "skd-edit", "role-info"] {
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
    fn application_dispatches_workspace_cache_and_handlers_through_ports() {
        use std::cell::RefCell;
        use std::rc::Rc;

        #[derive(Default)]
        struct RecordingPorts {
            discovered: RefCell<Vec<PathBuf>>,
            invoked: RefCell<Vec<&'static str>>,
            reported: RefCell<Vec<&'static str>>,
            invalidated: RefCell<Vec<String>>,
        }

        impl ports::ApplicationPorts for Rc<RecordingPorts> {
            fn discover_workspace(&self, cwd: PathBuf) -> Result<WorkspaceContext, String> {
                self.discovered.borrow_mut().push(cwd.clone());
                WorkspaceContext::discover(cwd)
            }

            fn invoke_handler(
                &self,
                spec: ToolSpec,
                _args: &Map<String, Value>,
                _context: &WorkspaceContext,
                _dry_run: bool,
            ) -> Result<AdapterOutcome, String> {
                self.invoked.borrow_mut().push(spec.name);
                Ok(AdapterOutcome::ok("fake port outcome"))
            }

            fn cache_report(
                &self,
                context: &WorkspaceContext,
                events: &[DomainEvent],
                dry_run: bool,
                cache_access: CacheAccess,
            ) -> Result<CacheReport, String> {
                self.reported.borrow_mut().extend(cache_access.writes);
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
                    .borrow_mut()
                    .extend(events.iter().map(|event| format!("{:?}", event.kind)));
            }
        }

        let root = std::env::temp_dir().join(format!("unica-ports-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let mut args = Map::new();
        args.insert("cwd".to_string(), Value::String(root.display().to_string()));
        let ports = Rc::new(RecordingPorts::default());
        let app = UnicaApplication::with_ports(Box::new(ports.clone()));

        let result = app.call_tool("unica.build.load", &args).unwrap();

        assert!(result.ok);
        assert_eq!(ports.invoked.borrow().as_slice(), ["unica.build.load"]);
        assert_eq!(
            ports.reported.borrow().as_slice(),
            ["workspace_graph", "metadata_graph"]
        );
        assert!(ports.invalidated.borrow().is_empty());
        assert_eq!(ports.discovered.borrow().len(), 1);

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
        assert!(bin_text.contains(",1,0,aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"));
        assert!(bin_text.contains(",1,0,bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"));
        assert!(bin_text.contains(",1,0,cccccccc-cccc-cccc-cccc-cccccccccccc"));

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
        let root = temp_meta_compile_workspace("unica-meta-compile-child-format");
        let workspace = root.join("workspace");
        let src = workspace.join("src");
        let config_path = src.join("Configuration.xml");
        std::fs::write(
            &config_path,
            concat!(
                "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n",
                "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.17\">\r\n",
                "\t<Configuration uuid=\"aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa\">\r\n",
                "\t\t<Properties>\r\n",
                "\t\t\t<Name>Demo</Name>\r\n",
                "\t\t</Properties>\r\n",
                "\t\t<ChildObjects>\r\n",
                "\t\t\t<Catalog>Items</Catalog>\r\n",
                "\t\t</ChildObjects>\r\n",
                "\t</Configuration>\r\n",
                "</MetaDataObject>"
            ),
        )
        .unwrap();
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
            "\t\t\t<Report>MetaCompileFormatReport</Report>\n",
            "\t\t</ChildObjects>"
        )));
        assert!(!config_text.contains("\t\t\t\t\t<Report>MetaCompileFormatReport</Report>"));
        assert!(!config_text
            .contains("<Report>MetaCompileFormatReport</Report>\r\n\t\t</ChildObjects>"));

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

        let regenerated_xml =
            std::fs::read_to_string(workspace.join("src/Roles/SampleReader.xml")).unwrap();
        let regenerated_uuid = metadata_root_uuid(&regenerated_xml, "Role");
        assert_eq!(first_uuid, regenerated_uuid);

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
                    "ChartOfCharacteristicTypesCharacteristic.MetaCompileCharacteristics",
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
        std::fs::create_dir_all(&ext).unwrap();
        std::fs::create_dir_all(&forms).unwrap();
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
        assert!(std::fs::read_to_string(&help_xml)
            .unwrap()
            .contains("<Page>ru</Page>"));
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

    struct FixedOutcomePorts {
        outcome: AdapterOutcome,
    }

    impl ports::ApplicationPorts for FixedOutcomePorts {
        fn discover_workspace(&self, cwd: PathBuf) -> Result<WorkspaceContext, String> {
            Ok(WorkspaceContext {
                cwd: cwd.clone(),
                workspace_root: cwd.clone(),
                cache_root: cwd.join(".build").join("unica"),
                workspace_epoch: 1,
            })
        }

        fn invoke_handler(
            &self,
            _spec: ToolSpec,
            _args: &Map<String, Value>,
            _context: &WorkspaceContext,
            _dry_run: bool,
        ) -> Result<AdapterOutcome, String> {
            Ok(self.outcome.clone())
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
        UnicaApplication::with_ports(Box::new(FixedOutcomePorts { outcome }))
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
            crate::infrastructure::native_operations::meta::is_guid(&uuid),
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
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.17">
  <Configuration uuid="{uuid}">
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
    <ChildObjects><Catalog>Items</Catalog></ChildObjects>
  </Configuration>
</MetaDataObject>"#
        )
    }

    fn support_test_catalog_xml(uuid: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.17">
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
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.17">
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
        let context = WorkspaceContext::discover(workspace.clone()).unwrap();
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

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(workspace.join("erf"), workspace.join("epf-link")).unwrap();
            args.insert(
                "OutputDir".to_string(),
                Value::String("epf-link".to_string()),
            );
            let error = UnicaApplication::new()
                .call_tool("unica.epf.init", &args)
                .unwrap_err();
            assert!(error.contains("must not traverse symlink"), "{error}");
        }

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
