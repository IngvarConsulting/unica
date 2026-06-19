use crate::domain::cache::{CacheAccess, CacheReport};
use crate::domain::events::{DomainEvent, DomainEventKind};
use crate::domain::project_sources::discover_project_source_map;
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::internal_adapters::{
    CliAdapter, CodeNavigationAdapter, CodeSearchAdapter, RuntimeAdapter, StandardsAdapter,
};
use crate::infrastructure::legacy_scripts::LegacyScriptAdapter;
use crate::infrastructure::native_operations::NativeOperationAdapter;
use crate::infrastructure::workspace_index::WorkspaceIndexService;
use crate::infrastructure::workspace_state::WorkspaceStateRepository;
use crate::infrastructure::AdapterOutcome;
use serde::Serialize;
use serde_json::{Map, Value};
use std::env;
use std::path::PathBuf;

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
    LegacyScript {
        skill: &'static str,
        script: &'static str,
        event: Option<DomainEventKind>,
    },
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
}

pub struct UnicaApplication;

impl UnicaApplication {
    pub fn new() -> Self {
        Self
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
        call_tool(spec, args)
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
            description: "Search BSL code through the internal code index adapter.",
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

fn call_tool(spec: ToolSpec, args: &Map<String, Value>) -> Result<OperationResult, String> {
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
    let context = WorkspaceContext::discover(cwd)?;
    tool_contracts::validate_workspace_paths(spec, args, dry_run, &context)?;
    tool_contracts::validate_native_source_set_format(spec, args, dry_run, &context)?;
    let state_repo = WorkspaceStateRepository::new(&context);
    let index_report = if uses_bsl_index(spec) {
        WorkspaceIndexService::new().start_for_workspace(&context, args, dry_run)
    } else {
        Default::default()
    };

    let mut outcome = match spec.handler {
        ToolHandler::LegacyScript { skill, script, .. } => LegacyScriptAdapter::invoke(
            skill,
            script,
            spec.name,
            args,
            &context,
            dry_run,
            spec.mutating,
        )?,
        ToolHandler::NativeOperation { operation, .. } => NativeOperationAdapter::invoke(
            operation,
            spec.name,
            args,
            &context,
            dry_run,
            spec.mutating,
        )?,
        ToolHandler::ProjectStatus => project_status(&context),
        ToolHandler::ProjectMap => project_map(&context),
        ToolHandler::BuildRuntime { command, .. } => CliAdapter::new(
            "run-v8-runner.sh",
            command,
            "build/runtime",
        )
        .invoke(spec.name, args, &context, dry_run, spec.mutating)?,
        ToolHandler::RuntimeAdapter => {
            RuntimeAdapter::new().invoke(spec.name, args, &context, dry_run, spec.mutating)?
        }
        ToolHandler::CodeAdapter { command } if command == ["search"] => {
            CodeSearchAdapter::new().invoke(spec.name, args, &context, dry_run)?
        }
        ToolHandler::CodeAdapter { command }
            if matches!(command, ["definition"] | ["outline"] | ["grep"]) =>
        {
            CodeNavigationAdapter::new().invoke(spec.name, args, &context, dry_run)?
        }
        ToolHandler::CodeAdapter { command } => CliAdapter::new(
            "run-bsl-analyzer.sh",
            command,
            "code analysis",
        )
        .invoke(spec.name, args, &context, dry_run, spec.mutating)?,
        ToolHandler::StandardsAdapter { operation } => StandardsAdapter::invoke(operation, args),
    };
    outcome.warnings.extend(index_report.warnings);

    let events = if should_emit_events(spec, dry_run, &outcome) {
        domain_events(spec, args)
    } else {
        Vec::new()
    };
    let cache = state_repo.report(&context, &events, dry_run, spec.cache_access)?;

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
    })
}

fn should_emit_events(spec: ToolSpec, dry_run: bool, outcome: &AdapterOutcome) -> bool {
    spec.mutating && (dry_run || outcome.ok)
}

fn uses_bsl_index(spec: ToolSpec) -> bool {
    spec.cache_access.reads.contains(&"bsl_index")
        || spec.cache_access.writes.contains(&"bsl_index")
}

fn domain_events(spec: ToolSpec, args: &Map<String, Value>) -> Vec<DomainEvent> {
    match spec.handler {
        ToolHandler::LegacyScript {
            event: Some(event), ..
        } => vec![DomainEvent::new(event, spec.name)],
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

    #[test]
    fn lists_unica_orchestrator_scope() {
        let names = tools().iter().map(|tool| tool.name).collect::<Vec<_>>();
        assert!(names.contains(&"unica.project.status"));
        assert!(names.contains(&"unica.project.map"));
        assert!(names.contains(&"unica.form.validate"));
        assert!(names.contains(&"unica.skd.edit"));
        assert!(names.contains(&"unica.mxl.compile"));
        assert!(names.contains(&"unica.role.validate"));
        assert!(names.contains(&"unica.build.load"));
        assert!(names.contains(&"unica.runtime.execute"));
        assert!(names.contains(&"unica.code.definition"));
        assert!(names.contains(&"unica.code.outline"));
        assert!(names.contains(&"unica.code.grep"));
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
        assert!(result.command.is_none());
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
    fn xml_dsl_tools_route_to_existing_script_or_parity_covered_native_handler() {
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

        for tool in tools() {
            if !tool.name.starts_with("unica.cf.")
                && !tool.name.starts_with("unica.cfe.")
                && !tool.name.starts_with("unica.meta.")
                && !tool.name.starts_with("unica.form.")
                && !tool.name.starts_with("unica.interface.")
                && !tool.name.starts_with("unica.subsystem.")
                && !tool.name.starts_with("unica.template.")
                && !tool.name.starts_with("unica.skd.")
                && !tool.name.starts_with("unica.mxl.")
                && !tool.name.starts_with("unica.role.")
            {
                continue;
            }

            match tool.handler {
                ToolHandler::LegacyScript { skill, script, .. } => {
                    let script_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                        .join("../..")
                        .join("plugins")
                        .join("unica")
                        .join("skills")
                        .join(skill)
                        .join("scripts")
                        .join(script);
                    assert!(
                        script_path.is_file(),
                        "{} routes to missing transitional script {}",
                        tool.name,
                        script_path.display()
                    );
                }
                ToolHandler::NativeOperation { operation, .. } => {
                    assert!(
                        PARITY_COVERED_TOOLS.contains(&tool.name),
                        "{} routes to native operation {} without a parity fixture proving script-equivalent behavior",
                        tool.name,
                        operation
                    );
                }
                _ => panic!("{} routes through unexpected handler", tool.name),
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
}
