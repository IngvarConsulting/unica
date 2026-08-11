use crate::domain::cache::{CacheAccess, CacheReport};
use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::{
    CodeIntelligenceReadRequest, ProviderDeadline, SearchRequest,
};
use crate::domain::events::{runtime_event_kind, DomainEvent, DomainEventKind};
use crate::domain::workspace::WorkspaceContext;
pub(crate) use operation_descriptors::SupportGuardRequirement;
pub(crate) use outcome::AdapterOutcome;
use ports::{ApplicationPorts, FormatGuardCheck, FormatGuardError, SupportGuardCheck};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
pub(crate) use tool_contracts::{
    DIAGNOSTICS_ANALYZE_TIMEOUT_MAX_SECONDS, DIAGNOSTICS_ANALYZE_TIMEOUT_MIN_SECONDS,
};

pub(crate) mod code_intelligence;
pub(crate) mod documentation;
pub(crate) mod metadata;
pub(crate) mod operation_descriptors;
pub(crate) mod operational_config;
mod outcome;
pub(crate) mod ports;
pub(crate) mod source_navigation;
pub(crate) mod source_resources;
pub(crate) mod tool_contracts;
pub use tool_contracts::input_schema_for_tool;

const PUBLIC_INVOCATION_DEADLINE: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolExecution {
    Read,
    Mutation,
}

impl ToolExecution {
    pub const fn is_mutating(self) -> bool {
        matches!(self, Self::Mutation)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvocationMode {
    Read,
    Preview,
    Apply,
}

impl InvocationMode {
    fn from_validated_args(spec: ToolSpec, args: &Map<String, Value>) -> Result<Self, String> {
        match spec.execution {
            ToolExecution::Read => Ok(Self::Read),
            ToolExecution::Mutation => match args.get("dryRun") {
                None | Some(Value::Bool(true)) => Ok(Self::Preview),
                Some(Value::Bool(false)) => Ok(Self::Apply),
                Some(_) => Err(format!("{} argument `dryRun` must be a boolean", spec.name)),
            },
        }
    }

    pub const fn is_preview(self) -> bool {
        matches!(self, Self::Preview)
    }

    pub const fn is_apply(self) -> bool {
        matches!(self, Self::Apply)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultContract {
    Typed,
    ExternalStream,
}

#[derive(Debug, Clone, Copy)]
pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub execution: ToolExecution,
    pub result_contract: ResultContract,
    pub cache_access: CacheAccess,
    pub handler: ToolHandler,
}

// ToolHandler remains inspectable for surface-contract tests, while the typed
// metadata operation enum is an application-internal dispatch detail.
#[allow(private_interfaces)]
#[derive(Debug, Clone, Copy)]
pub enum ToolHandler {
    Metadata {
        operation: metadata::MetadataOperation,
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
    RuntimeJob {
        action: RuntimeJobAction,
    },
    CodeIntelligence {
        operation: CodeIntelligenceOperation,
    },
    SourceNavigation {
        operation: SourceNavigationOperation,
    },
    SourceResources {
        operation: SourceResourceOperation,
    },
    CodeAdapter {
        command: &'static [&'static str],
    },
    StandardsAdapter {
        operation: &'static str,
    },
    Documentation {
        operation: &'static str,
    },
}

pub use source_navigation::SourceNavigationOperation;
pub use source_resources::SourceResourceOperation;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeJobAction {
    Start,
    Status,
    Wait,
    Logs,
    Cancel,
    List,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeIntelligenceOperation {
    Search,
    Definition,
    Outline,
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

/// Closed MCP envelope shared by the four typed Meta operations.
///
/// Operation-specific payloads deliberately remain unconstrained JSON values;
/// the stable envelope and cache report stay machine-checkable.
pub fn operation_result_output_schema() -> Value {
    let string_array = || json!({"type": "array", "items": {"type": "string"}});
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "ok": {"type": "boolean"},
            "summary": {"type": "string"},
            "changes": string_array(),
            "warnings": string_array(),
            "errors": string_array(),
            "artifacts": string_array(),
            "cache": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "mode": {"type": "string"},
                    "root": {"type": "string"},
                    "workspace_epoch": {"type": "integer", "minimum": 0},
                    "events": string_array(),
                    "invalidated": string_array(),
                    "refreshed": string_array(),
                    "lazy_rebuilt": string_array(),
                    "stale": string_array(),
                    "fresh": string_array()
                },
                "required": [
                    "mode", "root", "workspace_epoch", "events", "invalidated",
                    "refreshed", "lazy_rebuilt", "stale", "fresh"
                ]
            },
            "stdout": {"type": "string"},
            "stderr": {"type": "string"},
            "command": string_array(),
            "diagnostics": {},
            "data": {},
            "job": {}
        },
        "required": [
            "ok", "summary", "changes", "warnings", "errors", "artifacts", "cache"
        ]
    })
}

/// Closed transport schema for the logical typed `unica.role.edit` payload.
/// Valid calls always return the same required typed `data`, including guard
/// failures. The common cache envelope remains, but its physical root is
/// deliberately redacted for this logical-only API.
pub fn role_edit_output_schema() -> Value {
    let mut schema = operation_result_output_schema();
    if let Some(properties) = schema["properties"].as_object_mut() {
        for forbidden in ["stdout", "stderr", "command", "diagnostics", "job"] {
            properties.remove(forbidden);
        }
    }
    schema["properties"]["cache"]["properties"]["root"] = json!({"const": ""});
    schema["properties"]["data"] = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "metadataPath": {
                "type": "string",
                "pattern": crate::domain::role::ROLE_METADATA_PATH_PATTERN
            },
            "changed": {"type": "boolean"},
            "effects": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "operationIndex": {"type": "integer", "minimum": 0},
                        "operation": {"const": "setRight"},
                        "objectName": {"type": "string", "minLength": 3},
                        "right": {"type": "string", "minLength": 1},
                        "before": {"type": ["boolean", "null"]},
                        "after": {"type": "boolean"},
                        "action": {"type": "string", "enum": ["setRight", "removeObject"]},
                        "changed": {"type": "boolean"}
                    },
                    "required": [
                        "operationIndex", "operation", "objectName", "right", "before",
                        "after", "action", "changed"
                    ]
                }
            },
            "validation": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "status": {"type": "string", "enum": ["passed", "failed"]}
                },
                "required": ["status"]
            },
            "diagnostics": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "code": {"type": "string", "minLength": 1},
                        "severity": {"type": "string", "enum": ["error", "warning"]},
                        "message": {"type": "string"},
                        "operationIndex": {"type": "integer", "minimum": 0}
                    },
                    "required": ["code", "severity", "message"]
                }
            }
        },
        "required": ["metadataPath", "changed", "effects", "validation", "diagnostics"]
    });
    schema["required"]
        .as_array_mut()
        .expect("OperationResult required fields are an array")
        .push(json!("data"));
    schema
}

/// Project invalid Meta arguments into the stable operation envelope for an
/// MCP adapter without changing the direct application-call error contract.
pub fn metadata_argument_failure_result(
    name: &str,
    args: &Map<String, Value>,
) -> Option<OperationResult> {
    let spec = tools().into_iter().find(|tool| tool.name == name)?;
    let ToolHandler::Metadata { operation } = spec.handler else {
        return None;
    };
    metadata::parse_metadata_request(operation, args)
        .err()
        .map(invalid_metadata_arguments_result)
}

/// Preserve the typed role result for operation-level parser failures that
/// cannot be expressed by the host-visible owner-independent right union.
///
/// Top-level/address failures remain transport errors: they have no canonical
/// `metadataPath` that could satisfy the role output schema. An operation-level
/// error is only produced after the parser has accepted that logical address,
/// so it can be returned with its exact `operationIndex`.
pub fn role_edit_argument_failure_result(
    name: &str,
    args: &Map<String, Value>,
) -> Option<OperationResult> {
    if name != "unica.role.edit" {
        return None;
    }
    let error = crate::domain::role::parse_role_edit_request(args).err()?;
    let operation_index = error.operation_index?;
    let metadata_path = args.get("metadataPath")?.as_str()?.to_string();
    let message = error.message.clone();
    let data = crate::domain::role::RoleEditData::failed(
        metadata_path,
        error.code,
        message.clone(),
        Some(operation_index),
    );
    Some(OperationResult {
        ok: false,
        summary: "unica.role.edit rejected invalid operation".to_string(),
        changes: Vec::new(),
        warnings: Vec::new(),
        errors: vec![format!("{}: {message}", error.code)],
        artifacts: Vec::new(),
        cache: CacheReport {
            mode: "read".to_string(),
            root: String::new(),
            workspace_epoch: 0,
            events: Vec::new(),
            invalidated: Vec::new(),
            refreshed: Vec::new(),
            lazy_rebuilt: Vec::new(),
            stale: Vec::new(),
            fresh: Vec::new(),
            publication_warnings: Vec::new(),
        },
        stdout: None,
        stderr: None,
        command: None,
        diagnostics: None,
        data: Some(
            serde_json::to_value(data)
                .expect("typed role edit diagnostics are always serializable"),
        ),
        job: None,
    })
}

/// Public application entry point.
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
        let deadline = ProviderDeadline::from_budget(PUBLIC_INVOCATION_DEADLINE);
        let spec = tools()
            .into_iter()
            .find(|tool| tool.name == name)
            .ok_or_else(|| {
                if name == "unica.code.grep" {
                    "unica.code.grep was removed; use unica.code.search and inspect its git-grep section"
                        .to_string()
                } else {
                    format!("unknown unica tool: {name}")
                }
            })?;
        call_tool(spec, args, self.ports.as_ref(), &cancellation, deadline)
    }
}

#[cfg(test)]
mod meta_add_surface_tests;
#[cfg(test)]
mod meta_info_surface_tests;
#[cfg(test)]
mod meta_remove_surface_tests;

pub fn tools() -> Vec<ToolSpec> {
    let mut specs = configuration_tools();
    specs.extend([
        ToolSpec {
            name: "unica.project.status",
            description: "Inspect current Unica workspace, source set, and cache state.",
            execution: ToolExecution::Read,
            result_contract: ResultContract::Typed,
            cache_access: CacheAccess::default(),
            handler: ToolHandler::ProjectStatus,
        },
        ToolSpec {
            name: "unica.project.map",
            description:
                "Inspect configured source sets and effective source format per source set.",
            execution: ToolExecution::Read,
            result_contract: ResultContract::Typed,
            cache_access: CacheAccess {
                reads: &["workspace_graph"],
                writes: &[],
            },
            handler: ToolHandler::ProjectMap,
        },
        ToolSpec {
            name: "unica.source.resolve",
            description:
                "Resolve an exact or prefix logical metadata query inside one named source set.",
            execution: ToolExecution::Read,
            result_contract: ResultContract::Typed,
            cache_access: CacheAccess {
                reads: &["workspace_graph", "metadata_graph"],
                writes: &[],
            },
            handler: ToolHandler::SourceNavigation {
                operation: SourceNavigationOperation::Resolve,
            },
        },
        ToolSpec {
            name: "unica.source.children",
            description:
                "List exactly one level below a logical source-set root or metadata address.",
            execution: ToolExecution::Read,
            result_contract: ResultContract::Typed,
            cache_access: CacheAccess {
                reads: &["workspace_graph", "metadata_graph"],
                writes: &[],
            },
            handler: ToolHandler::SourceNavigation {
                operation: SourceNavigationOperation::Children,
            },
        },
        ToolSpec {
            name: "unica.source.locate",
            description:
                "Recover the logical metadata address that owns one source path inside a named source set.",
            execution: ToolExecution::Read,
            result_contract: ResultContract::Typed,
            cache_access: CacheAccess {
                reads: &["workspace_graph", "metadata_graph"],
                writes: &[],
            },
            handler: ToolHandler::SourceNavigation {
                operation: SourceNavigationOperation::Locate,
            },
        },
        ToolSpec {
            name: "unica.source.resources",
            description:
                "Open or page an immutable bounded manifest for one logical source target.",
            execution: ToolExecution::Read,
            result_contract: ResultContract::Typed,
            cache_access: CacheAccess::default(),
            handler: ToolHandler::SourceResources {
                operation: SourceResourceOperation::Resources,
            },
        },
        ToolSpec {
            name: "unica.source.read",
            description:
                "Read one bounded byte range from a resource in an issued immutable snapshot.",
            execution: ToolExecution::Read,
            result_contract: ResultContract::Typed,
            cache_access: CacheAccess::default(),
            handler: ToolHandler::SourceResources {
                operation: SourceResourceOperation::Read,
            },
        },
        ToolSpec {
            name: "unica.build.dump",
            description: "Dump source set through the internal build/runtime adapter.",
            execution: ToolExecution::Mutation,
            result_contract: ResultContract::ExternalStream,
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
            execution: ToolExecution::Mutation,
            result_contract: ResultContract::ExternalStream,
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
            execution: ToolExecution::Mutation,
            result_contract: ResultContract::ExternalStream,
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
            execution: ToolExecution::Mutation,
            result_contract: ResultContract::ExternalStream,
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
            execution: ToolExecution::Mutation,
            result_contract: ResultContract::ExternalStream,
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
            execution: ToolExecution::Mutation,
            result_contract: ResultContract::ExternalStream,
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
            execution: ToolExecution::Mutation,
            result_contract: ResultContract::ExternalStream,
            cache_access: CacheAccess::default(),
            handler: ToolHandler::RuntimeJob {
                action: RuntimeJobAction::Start,
            },
        },
        ToolSpec {
            name: "unica.runtime.job.status",
            description: "Read a durable runtime job snapshot by jobId.",
            execution: ToolExecution::Read,
            result_contract: ResultContract::ExternalStream,
            cache_access: CacheAccess::default(),
            handler: ToolHandler::RuntimeJob {
                action: RuntimeJobAction::Status,
            },
        },
        ToolSpec {
            name: "unica.runtime.job.wait",
            description: "Wait for a durable runtime job with a caller-side bounded timeout.",
            execution: ToolExecution::Read,
            result_contract: ResultContract::ExternalStream,
            cache_access: CacheAccess::default(),
            handler: ToolHandler::RuntimeJob {
                action: RuntimeJobAction::Wait,
            },
        },
        ToolSpec {
            name: "unica.runtime.job.logs",
            description: "Read bounded redacted stdout and stderr tails for a durable runtime job.",
            execution: ToolExecution::Read,
            result_contract: ResultContract::ExternalStream,
            cache_access: CacheAccess::default(),
            handler: ToolHandler::RuntimeJob {
                action: RuntimeJobAction::Logs,
            },
        },
        ToolSpec {
            name: "unica.runtime.job.cancel",
            description: "Request safe cancellation for a durable runtime job.",
            execution: ToolExecution::Mutation,
            result_contract: ResultContract::ExternalStream,
            cache_access: CacheAccess::default(),
            handler: ToolHandler::RuntimeJob {
                action: RuntimeJobAction::Cancel,
            },
        },
        ToolSpec {
            name: "unica.runtime.job.list",
            description: "List durable runtime job snapshots in the current workspace.",
            execution: ToolExecution::Read,
            result_contract: ResultContract::ExternalStream,
            cache_access: CacheAccess::default(),
            handler: ToolHandler::RuntimeJob {
                action: RuntimeJobAction::List,
            },
        },
        ToolSpec {
            name: "unica.code.search",
            description: "Search code concurrently through provider-local RLM, bsl-analyzer, and literal git-grep sections. Migration: use sourceDir instead of the former path/config fields and a per-provider limit from 1 to 50.",
            execution: ToolExecution::Read,
            result_contract: ResultContract::Typed,
            cache_access: CacheAccess {
                reads: &["bsl_index", "workspace_graph"],
                writes: &[],
            },
            handler: ToolHandler::CodeIntelligence {
                operation: CodeIntelligenceOperation::Search,
            },
        },
        ToolSpec {
            name: "unica.code.definition",
            description: "Find BSL method definitions through the typed Unica code index boundary.",
            execution: ToolExecution::Read,
            result_contract: ResultContract::Typed,
            cache_access: CacheAccess {
                reads: &["bsl_index"],
                writes: &[],
            },
            handler: ToolHandler::CodeIntelligence {
                operation: CodeIntelligenceOperation::Definition,
            },
        },
        ToolSpec {
            name: "unica.code.outline",
            description: "Read compact BSL module outline from the current source file.",
            execution: ToolExecution::Read,
            result_contract: ResultContract::Typed,
            // ADR-0020: the outline is parsed from the file on disk, so this tool
            // neither reads nor writes any workspace cache.
            cache_access: CacheAccess {
                reads: &[],
                writes: &[],
            },
            handler: ToolHandler::CodeIntelligence {
                operation: CodeIntelligenceOperation::Outline,
            },
        },
        ToolSpec {
            name: "unica.code.patch",
            description:
                "Insert or replace BSL in one logically addressed Platform XML Configuration or Extension module.",
            execution: ToolExecution::Mutation,
            result_contract: ResultContract::Typed,
            cache_access: cache_access_for("code-patch", Some(DomainEventKind::ModuleChanged)),
            handler: ToolHandler::NativeOperation {
                operation: "code-patch",
                event: Some(DomainEventKind::ModuleChanged),
            },
        },
        ToolSpec {
            name: "unica.xdto.info",
            description: "Inspect one logically addressed 1C XDTO package schema.",
            execution: ToolExecution::Read,
            result_contract: ResultContract::Typed,
            cache_access: CacheAccess::default(),
            handler: ToolHandler::NativeOperation { operation: "xdto-info", event: None },
        },
        ToolSpec {
            name: "unica.xdto.edit",
            description: "Preview or apply a safe targeted mutation to one logically addressed 1C XDTO package schema.",
            execution: ToolExecution::Mutation,
            result_contract: ResultContract::Typed,
            cache_access: cache_access_for("xdto-edit", Some(DomainEventKind::MetadataChanged)),
            handler: ToolHandler::NativeOperation { operation: "xdto-edit", event: Some(DomainEventKind::MetadataChanged) },
        },
        ToolSpec {
            name: "unica.code.graph",
            description: "Inspect BSL call graph through the typed Unica code analysis boundary.",
            execution: ToolExecution::Read,
            result_contract: ResultContract::Typed,
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
            execution: ToolExecution::Read,
            result_contract: ResultContract::Typed,
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
            execution: ToolExecution::Read,
            result_contract: ResultContract::Typed,
            cache_access: CacheAccess::default(),
            handler: ToolHandler::StandardsAdapter {
                operation: "search",
            },
        },
        ToolSpec {
            name: "unica.standards.explain",
            description:
                "Explain 1C diagnostics or standards through the internal standards adapter.",
            execution: ToolExecution::Read,
            result_contract: ResultContract::Typed,
            cache_access: CacheAccess::default(),
            handler: ToolHandler::StandardsAdapter {
                operation: "explain",
            },
        },
        ToolSpec {
            name: "unica.documentation.search",
            description:
                "Search the workspace configuration's embedded help, platform help, and development standards across documentation providers.",
            execution: ToolExecution::Read,
            result_contract: ResultContract::Typed,
            cache_access: CacheAccess::default(),
            handler: ToolHandler::Documentation {
                operation: "search",
            },
        },
        ToolSpec {
            name: "unica.documentation.get",
            description:
                "Fetch the full text of a documentation search hit by its documentId locator.",
            execution: ToolExecution::Read,
            result_contract: ResultContract::Typed,
            cache_access: CacheAccess::default(),
            handler: ToolHandler::Documentation { operation: "get" },
        },
    ]);
    specs
}

fn call_tool(
    spec: ToolSpec,
    args: &Map<String, Value>,
    ports: &dyn ApplicationPorts,
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
) -> Result<OperationResult, String> {
    let normalized_args = tool_contracts::normalize_native_path_aliases(spec, args)?;
    let args = &normalized_args;
    tool_contracts::validate_tool_argument_shape(spec, args)?;
    let mode = InvocationMode::from_validated_args(spec, args)?;
    tool_contracts::validate_tool_argument_semantics(spec, args, mode)?;
    let dry_run = mode.is_preview();
    let cwd = args.get("cwd").and_then(Value::as_str).map(PathBuf::from);
    let context = ports.discover_workspace(cwd)?;
    ports.validate_tool_context(spec, args, mode, &context)?;
    let mut prepared =
        ports.prepare_tool_invocation(spec, args, &context, mode, cancellation, deadline)?;
    let xdto_target = XdtoLogicalTarget::from_call(spec, args);
    let role_target = RoleEditLogicalTarget::from_call(spec, args);
    let mut format_guard_warning = None;
    let mut format_diagnostic = None;
    let format_guard = match prepared.format_guard.take() {
        Some(check) => check,
        None => match ports.evaluate_format_guard(spec, args, &context) {
            Ok(check) => check,
            Err(error) => {
                if let Some(target) = role_target.as_ref() {
                    let public = error.to_string();
                    let code = role_guard_error_code(&public, "format_guard_failed");
                    let cache = match role_cache_report(
                        ports,
                        &context,
                        target,
                        &[],
                        mode,
                        spec.cache_access,
                    ) {
                        Ok(cache) => cache,
                        Err(result) => return Ok(*result),
                    };
                    return Ok(target.failed_result(
                        cache,
                        dry_run,
                        code,
                        role_guard_failure_reason(code),
                    ));
                }
                return Err(project_xdto_format_guard_error(xdto_target.as_ref(), error));
            }
        },
    };
    match format_guard {
        FormatGuardCheck::Allow => {}
        FormatGuardCheck::Warn {
            warning,
            diagnostic,
        } => {
            if let Some(target) = role_target.as_ref() {
                format_guard_warning = Some(target.warning(
                    "format_guard_warning",
                    "the role export is outside the supported platform 8.3.27 / format 2.20 profile",
                ));
            } else if let Some(target) = xdto_target.as_ref() {
                format_guard_warning = Some(target.warning(
                    "format_guard_warning",
                    "the source export format is outside the supported profile",
                ));
                format_diagnostic = Some(target.format_diagnostic(&diagnostic));
            } else {
                format_guard_warning = Some(warning);
                format_diagnostic = Some(diagnostic);
            }
        }
        FormatGuardCheck::Block {
            mut outcome,
            diagnostic,
        } => {
            if let Some(target) = role_target.as_ref() {
                let code = diagnostic
                    .get("code")
                    .and_then(Value::as_str)
                    .unwrap_or("format_incompatible");
                let cache = match role_cache_report(
                    ports,
                    &context,
                    target,
                    &[],
                    mode,
                    spec.cache_access,
                ) {
                    Ok(cache) => cache,
                    Err(result) => return Ok(*result),
                };
                return Ok(target.failed_result(
                    cache,
                    dry_run,
                    code,
                    role_guard_failure_reason(code),
                ));
            }
            let diagnostic = if let Some(target) = xdto_target.as_ref() {
                let code = diagnostic
                    .get("code")
                    .and_then(Value::as_str)
                    .unwrap_or("format_incompatible");
                outcome = target.blocked_outcome(
                    spec,
                    code,
                    "the source export format is outside the supported profile",
                );
                target.format_diagnostic(&diagnostic)
            } else {
                diagnostic
            };
            let cache = ports.cache_report(&context, &[], mode, spec.cache_access)?;
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
        let cache = ports.cache_report(&context, &[], mode, spec.cache_access)?;
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
    let support_guard_warning = if spec.execution.is_mutating() {
        let support_guard = match ports.evaluate_support_guard(spec, args, &context) {
            Ok(check) => check,
            Err(error) => {
                if let Some(target) = role_target.as_ref() {
                    let code = role_guard_error_code(&error, "support_guard_failed");
                    let cache = match role_cache_report(
                        ports,
                        &context,
                        target,
                        &[],
                        mode,
                        spec.cache_access,
                    ) {
                        Ok(cache) => cache,
                        Err(result) => return Ok(*result),
                    };
                    return Ok(target.failed_result(
                        cache,
                        dry_run,
                        code,
                        role_guard_failure_reason(code),
                    ));
                }
                return Err(project_xdto_guard_error(
                    xdto_target.as_ref(),
                    "support_guard_failed",
                    error,
                ));
            }
        };
        match support_guard {
            SupportGuardCheck::Allow => None,
            SupportGuardCheck::Warn(warning) => Some(if let Some(target) = role_target.as_ref() {
                target.warning(
                    "support_guard_warning",
                    "the role is protected by support policy; the operation continues in warn mode",
                )
            } else if let Some(target) = xdto_target.as_ref() {
                target.warning(
                        "support_guard_warning",
                        "the target is protected by support policy; the operation continues in warn mode",
                    )
            } else {
                warning
            }),
            SupportGuardCheck::Block(mut outcome) => {
                if let Some(target) = role_target.as_ref() {
                    let cache = match role_cache_report(
                        ports,
                        &context,
                        target,
                        &[],
                        mode,
                        spec.cache_access,
                    ) {
                        Ok(cache) => cache,
                        Err(result) => return Ok(*result),
                    };
                    return Ok(target.failed_result(
                        cache,
                        dry_run,
                        "support_locked",
                        role_guard_failure_reason("support_locked"),
                    ));
                }
                if let Some(target) = xdto_target.as_ref() {
                    outcome = target.blocked_outcome(
                        spec,
                        "support_locked",
                        "the target is protected by support policy",
                    );
                }
                if dry_run {
                    outcome.summary = format!("dry run: {}", outcome.summary);
                }
                let cache = ports.cache_report(&context, &[], mode, spec.cache_access)?;
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

    let requires_operational_config = operational_config::requires_snapshot(spec, args);
    if requires_operational_config && cancellation.is_cancelled() {
        return Err(crate::domain::cancellation::cancelled_error(
            "operational config resolution stopped before reading workspace files",
        ));
    }
    let resolved_operational_config =
        operational_config::resolve_for_call(ports, spec, args, &context);
    if requires_operational_config && cancellation.is_cancelled() {
        return Err(crate::domain::cancellation::cancelled_error(
            "operational config resolution stopped after reading workspace files",
        ));
    }
    let operational_config = match resolved_operational_config {
        Ok(config) => config,
        Err(diagnostic) => {
            let error = diagnostic.to_string();
            let diagnostic = serde_json::to_value(diagnostic).map_err(|serialize| {
                format!("failed to serialize operational config diagnostic: {serialize}")
            })?;
            let cache = ports.cache_report(&context, &[], mode, spec.cache_access)?;
            return Ok(OperationResult {
                ok: false,
                summary: format!("{} operational configuration is invalid", spec.name),
                changes: Vec::new(),
                warnings: Vec::new(),
                errors: vec![error],
                artifacts: Vec::new(),
                cache,
                stdout: None,
                stderr: None,
                command: None,
                diagnostics: Some(json!({"operationalConfig": diagnostic})),
                data: None,
                job: None,
            });
        }
    };
    let handler_outcome = match prepared.handler.take() {
        Some(handler) => handler,
        None => match spec.handler {
            ToolHandler::Metadata { operation } => {
                metadata::invoke(operation, ports, args, &context, cancellation)?
            }
            ToolHandler::CodeIntelligence {
                operation: CodeIntelligenceOperation::Search,
            } => invoke_code_intelligence_search(
                ports,
                args,
                &context,
                operational_config.as_ref().ok_or_else(|| {
                    "code intelligence call is missing operational config".to_string()
                })?,
                cancellation,
            )?,
            ToolHandler::CodeIntelligence { operation } => {
                invoke_code_intelligence_read(CodeIntelligenceReadInvocation {
                    ports,
                    tool_name: spec.name,
                    operation,
                    args,
                    workspace: &context,
                    operational_config: operational_config.as_ref().ok_or_else(|| {
                        "code intelligence call is missing operational config".to_string()
                    })?,
                    cancellation,
                })?
            }
            ToolHandler::SourceNavigation { operation } => {
                source_navigation::invoke(operation, ports, args, &context, cancellation)?
            }
            ToolHandler::SourceResources { operation } => {
                source_resources::invoke(operation, ports, args, &context, cancellation)?
            }
            _ => ports.invoke_handler_with_operational_config(
                spec,
                args,
                &context,
                mode,
                operational_config.as_ref(),
                cancellation,
            )?,
        },
    };
    enforce_result_contract(spec, mode, &handler_outcome)?;
    let mut outcome = handler_outcome.adapter;
    let handler_events = handler_outcome.events;
    let projected_events = handler_outcome.projected_events;
    let recorded_cache = handler_outcome.recorded_cache;
    let handler_diagnostics = handler_outcome.diagnostics;
    let mut handler_data = handler_outcome.data;
    if let Some(target) = role_target.as_ref().filter(|_| handler_data.is_none()) {
        let code = "handler_contract_failed";
        outcome = AdapterOutcome {
            ok: false,
            summary: "unica.role.edit handler violated its typed result contract".to_string(),
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: vec![format!(
                "{code}: {} — the role mutation returned no typed data",
                target.identity()
            )],
            artifacts: Vec::new(),
            stdout: None,
            stderr: None,
            command: None,
        };
        handler_data = Some(
            serde_json::to_value(crate::domain::role::RoleEditData::failed(
                target.metadata_path.clone(),
                code,
                "the role mutation returned no typed data",
                None,
            ))
            .expect("typed role edit diagnostics are always serializable"),
        );
    }
    if let Some(warning) = support_guard_warning {
        if !outcome
            .warnings
            .iter()
            .any(|existing| existing.starts_with("support_guard_warning:"))
        {
            outcome.warnings.insert(0, warning);
        }
    }
    if let Some(warning) = format_guard_warning {
        outcome.warnings.insert(0, warning);
    }
    let events = if dry_run && !projected_events.is_empty() {
        projected_events
    } else if !dry_run && spec.execution.is_mutating() && outcome.ok && !handler_events.is_empty() {
        handler_events
    } else if should_emit_events(spec, args, dry_run, &outcome, handler_data.as_ref()) {
        if handler_events.is_empty() {
            domain_events(spec, args)
        } else {
            handler_events
        }
    } else {
        Vec::new()
    };
    let mut cache = if let Some(cache) = recorded_cache {
        if dry_run || events.is_empty() {
            return Err(format!(
                "{} returned a persisted cache report without an applied event",
                spec.name
            ));
        }
        cache
    } else if let Some(target) = role_target.as_ref() {
        match role_cache_report(ports, &context, target, &events, mode, spec.cache_access) {
            Ok(cache) => cache,
            Err(result) => return Ok(*result),
        }
    } else {
        ports.cache_report(&context, &events, mode, spec.cache_access)?
    };
    outcome.warnings.append(&mut cache.publication_warnings);
    if spec.execution.is_mutating() && !dry_run && outcome.ok && !events.is_empty() {
        ports.notify_invalidation(&context, &events);
    }
    let diagnostics = merge_handler_diagnostics(
        handler_diagnostics,
        merge_diagnostics(
            runtime_result_diagnostics(spec, args, &context, &outcome, handler_data.as_ref()),
            format_diagnostic,
        ),
    );

    let role_typed = role_target.is_some();
    if role_typed {
        cache.root.clear();
    }
    let artifacts = if let Some(target) = role_target.as_ref() {
        if outcome.ok && !outcome.artifacts.is_empty() {
            vec![target.identity()]
        } else {
            Vec::new()
        }
    } else {
        outcome.artifacts
    };
    Ok(OperationResult {
        ok: outcome.ok,
        summary: outcome.summary,
        changes: outcome.changes,
        warnings: outcome.warnings,
        errors: outcome.errors,
        artifacts,
        cache,
        stdout: if role_typed { None } else { outcome.stdout },
        stderr: if role_typed { None } else { outcome.stderr },
        command: if role_typed { None } else { outcome.command },
        diagnostics: if role_typed { None } else { diagnostics },
        data: handler_data,
        job: if role_typed {
            None
        } else {
            handler_outcome.job
        },
    })
}

fn enforce_result_contract(
    spec: ToolSpec,
    mode: InvocationMode,
    outcome: &ports::HandlerOutcome,
) -> Result<(), String> {
    if mode == InvocationMode::Read
        && spec.result_contract == ResultContract::Typed
        && outcome.adapter.ok
    {
        if outcome.data.is_none() {
            return Err(format!(
                "typed_result_missing: {} returned ok without OperationResult.data",
                spec.name
            ));
        }
        if outcome.adapter.stdout.is_some() {
            return Err(format!(
                "typed_result_textual: {} returned ok with a stdout duplicate",
                spec.name
            ));
        }
    }
    Ok(())
}

fn invalid_metadata_arguments_result(failure: metadata::MetaFailure) -> OperationResult {
    let errors = failure
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.message.clone())
        .collect();
    let diagnostics = serde_json::to_value(&failure.diagnostics)
        .expect("metadata diagnostics are always serializable");
    OperationResult {
        ok: false,
        summary: "metadata arguments are invalid".to_string(),
        changes: Vec::new(),
        warnings: Vec::new(),
        errors,
        artifacts: Vec::new(),
        cache: CacheReport {
            mode: "read".to_string(),
            root: String::new(),
            workspace_epoch: 0,
            events: Vec::new(),
            invalidated: Vec::new(),
            refreshed: Vec::new(),
            lazy_rebuilt: Vec::new(),
            stale: Vec::new(),
            fresh: Vec::new(),
            publication_warnings: Vec::new(),
        },
        stdout: None,
        stderr: None,
        command: None,
        diagnostics: Some(diagnostics),
        data: None,
        job: None,
    }
}

#[derive(Clone, Debug)]
struct XdtoLogicalTarget {
    source_set: String,
    metadata_path: String,
}

#[derive(Clone, Debug)]
struct RoleEditLogicalTarget {
    source_set: String,
    metadata_path: String,
}

impl RoleEditLogicalTarget {
    fn from_call(spec: ToolSpec, args: &Map<String, Value>) -> Option<Self> {
        if spec.name != "unica.role.edit" {
            return None;
        }
        Some(Self {
            source_set: args.get("sourceSet")?.as_str()?.to_string(),
            metadata_path: args.get("metadataPath")?.as_str()?.to_string(),
        })
    }

    fn identity(&self) -> String {
        format!("{} + {}", self.source_set, self.metadata_path)
    }

    fn warning(&self, code: &str, reason: &str) -> String {
        format!("{code}: {} — {reason}", self.identity())
    }

    fn failed_result(
        &self,
        mut cache: CacheReport,
        dry_run: bool,
        code: &str,
        reason: &str,
    ) -> OperationResult {
        cache.root.clear();
        let message = format!("{code}: {} — {reason}", self.identity());
        let data = crate::domain::role::RoleEditData::failed(
            self.metadata_path.clone(),
            code,
            reason,
            None,
        );
        OperationResult {
            ok: false,
            summary: format!(
                "{}unica.role.edit blocked for {} ({code})",
                if dry_run { "dry run: " } else { "" },
                self.identity()
            ),
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: vec![message],
            artifacts: Vec::new(),
            cache,
            stdout: None,
            stderr: None,
            command: None,
            diagnostics: None,
            data: Some(
                serde_json::to_value(data)
                    .expect("typed role edit diagnostics are always serializable"),
            ),
            job: None,
        }
    }
}

fn role_cache_report(
    ports: &dyn ApplicationPorts,
    context: &WorkspaceContext,
    target: &RoleEditLogicalTarget,
    events: &[DomainEvent],
    mode: InvocationMode,
    access: CacheAccess,
) -> Result<CacheReport, Box<OperationResult>> {
    let dry_run = mode.is_preview();
    ports
        .cache_report(context, events, mode, access)
        .map_err(|_| {
            Box::new(target.failed_result(
                CacheReport {
                    mode: if dry_run { "dry-run" } else { "read" }.to_string(),
                    root: String::new(),
                    workspace_epoch: context.workspace_epoch,
                    events: Vec::new(),
                    invalidated: Vec::new(),
                    refreshed: Vec::new(),
                    lazy_rebuilt: Vec::new(),
                    stale: Vec::new(),
                    fresh: Vec::new(),
                    publication_warnings: Vec::new(),
                },
                dry_run,
                "cache_unavailable",
                role_guard_failure_reason("cache_unavailable"),
            ))
        })
}

fn role_guard_error_code(error: &str, fallback: &'static str) -> &'static str {
    [
        "source_set_unknown",
        "target_not_found",
        "not_a_role",
        "provider_unavailable",
        "containment_denied",
        "profile_unsupported",
    ]
    .into_iter()
    .find(|code| error.starts_with(&format!("{code}:")))
    .unwrap_or(fallback)
}

fn role_guard_failure_reason(code: &str) -> &'static str {
    match code {
        "source_set_unknown" => "the requested source set is unavailable",
        "target_not_found" => "the logical role target was not found",
        "not_a_role" => "metadataPath does not identify a role",
        "provider_unavailable" => "the logical source provider is unavailable",
        "containment_denied" => "the logical role target failed containment checks",
        "profile_unsupported" => "the logical address profile is unsupported",
        "support_locked" => "the logical role target is protected by support policy",
        "support_guard_failed" => "the support policy could not be evaluated safely",
        "cache_unavailable" => "the logical cache projection is unavailable",
        "formatMigrationAvailable" | "platformVersionUnsupported" | "formatVersionInvalid" => {
            "the role export is outside the supported platform 8.3.27 / format 2.20 profile"
        }
        _ => "the role mutation could not pass its preflight checks",
    }
}

impl XdtoLogicalTarget {
    fn from_call(spec: ToolSpec, args: &Map<String, Value>) -> Option<Self> {
        if !matches!(spec.name, "unica.xdto.info" | "unica.xdto.edit") {
            return None;
        }
        let source_set = args.get("sourceSet")?.as_str()?.to_string();
        let raw_metadata_path = args.get("metadataPath")?.as_str()?;
        let name = raw_metadata_path
            .strip_prefix("XDTOPackage.")
            .or_else(|| raw_metadata_path.strip_prefix("ПакетXDTO."))?;
        Some(Self {
            source_set,
            metadata_path: format!("XDTOPackage.{name}"),
        })
    }

    fn identity(&self) -> String {
        format!("{} + {}", self.source_set, self.metadata_path)
    }

    fn warning(&self, code: &str, reason: &str) -> String {
        format!("{code}: {} — {reason}", self.identity())
    }

    fn blocked_outcome(&self, spec: ToolSpec, code: &str, reason: &str) -> AdapterOutcome {
        let message = format!("{code}: {} — {reason}", self.identity());
        AdapterOutcome {
            ok: false,
            summary: format!("{} blocked for {} ({code})", spec.name, self.identity()),
            changes: Vec::new(),
            warnings: (code != "support_locked")
                .then(|| message.clone())
                .into_iter()
                .collect(),
            errors: vec![message.clone()],
            artifacts: vec![self.identity()],
            stdout: None,
            stderr: Some(format!("{message}\n")),
            command: None,
        }
    }

    fn format_diagnostic(&self, diagnostic: &Value) -> Value {
        let mut projected = Map::new();
        if let Some(source) = diagnostic.as_object() {
            for key in [
                "code",
                "actualFormat",
                "targetFormat",
                "targetPlatform",
                "compatibility",
                "ownerKind",
            ] {
                if let Some(value) = source.get(key) {
                    projected.insert(key.to_string(), value.clone());
                }
            }
        }
        projected.insert("sourceSet".to_string(), json!(self.source_set));
        projected.insert("metadataPath".to_string(), json!(self.metadata_path));
        projected.insert("targetKind".to_string(), json!("metadataObject"));
        Value::Object(projected)
    }
}

fn project_xdto_guard_error(
    target: Option<&XdtoLogicalTarget>,
    code: &str,
    internal_error: String,
) -> String {
    target.map_or(internal_error, |target| {
        format!("{code}: guard evaluation failed for {}", target.identity())
    })
}

fn project_xdto_format_guard_error(
    target: Option<&XdtoLogicalTarget>,
    error: FormatGuardError,
) -> String {
    let Some(target) = target else {
        return error.into_internal_cause();
    };
    match error.public_projection() {
        Some((code, message)) => {
            format!("{}: {} — {message}", code.as_str(), target.identity())
        }
        None => format!(
            "format_guard_failed: guard evaluation failed for {}",
            target.identity()
        ),
    }
}

fn invoke_code_intelligence_search(
    ports: &dyn ApplicationPorts,
    args: &Map<String, Value>,
    workspace: &WorkspaceContext,
    operational_config: &crate::domain::operational_config::OperationalConfig,
    cancellation: &CancellationToken,
) -> Result<ports::HandlerOutcome, String> {
    let context = ports.resolve_code_intelligence_context(workspace, args)?;
    let request = SearchRequest {
        query: args
            .get("query")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        limit: args
            .get("limit")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or(20),
    };
    let execution = code_intelligence::CodeSearchCoordinator::with_deadlines(
        ports.code_intelligence_registry()?,
        operational_config.code_intelligence(),
    )
    .search(&request, &context, cancellation)?;
    let artifacts = execution
        .result
        .sections
        .iter()
        .flat_map(|section| section.artifacts.clone())
        .collect();
    let data = serde_json::to_value(&execution.result)
        .map_err(|error| format!("failed to serialize code search result: {error}"))?;
    Ok(ports::HandlerOutcome::with_data(
        AdapterOutcome {
            ok: execution.ok,
            summary: if execution.ok {
                "unica.code.search completed through provider-neutral code intelligence".to_string()
            } else {
                "unica.code.search failed because no provider served the request".to_string()
            },
            changes: Vec::new(),
            warnings: execution.warnings,
            errors: execution.errors,
            artifacts,
            // ADR-0023: the sections are published as data, so a rendered copy
            // of them in stdout would be the second representation the decision
            // removes.
            stdout: None,
            stderr: None,
            command: None,
        },
        data,
    ))
}

struct CodeIntelligenceReadInvocation<'a> {
    ports: &'a dyn ApplicationPorts,
    tool_name: &'a str,
    operation: CodeIntelligenceOperation,
    args: &'a Map<String, Value>,
    workspace: &'a WorkspaceContext,
    operational_config: &'a crate::domain::operational_config::OperationalConfig,
    cancellation: &'a CancellationToken,
}

fn invoke_code_intelligence_read(
    invocation: CodeIntelligenceReadInvocation<'_>,
) -> Result<ports::HandlerOutcome, String> {
    let CodeIntelligenceReadInvocation {
        ports,
        tool_name,
        operation,
        args,
        workspace,
        operational_config,
        cancellation,
    } = invocation;
    let context = ports.resolve_code_intelligence_context(workspace, args)?;
    let request = ports.normalize_code_intelligence_read_request(
        code_intelligence_read_request(operation, args)?,
        &context,
    )?;
    let registry = ports.code_intelligence_registry()?;
    let provider = registry.provider_for(request.capability()).ok_or_else(|| {
        format!(
            "no code intelligence provider implements {:?} for {tool_name}",
            request.capability()
        )
    })?;
    let provider_id = provider.id();
    let mut outcome = code_intelligence::execute_provider_read(
        provider,
        request,
        context,
        operational_config
            .code_intelligence()
            .provider_read_timeout(),
        cancellation,
    )?;
    if outcome.provider != provider_id {
        outcome.warnings.insert(
            0,
            format!(
                "provider registry selected {}, but the response identified {}",
                provider_id.as_str(),
                outcome.provider.as_str()
            ),
        );
    }
    let data = outcome
        .data
        .take()
        .map(serde_json::to_value)
        .transpose()
        .map_err(|error| format!("failed to serialize code intelligence read result: {error}"))?;
    let adapter = AdapterOutcome {
        ok: outcome.ok,
        summary: outcome.summary,
        changes: Vec::new(),
        warnings: outcome.warnings,
        errors: outcome.errors,
        artifacts: outcome.artifacts,
        stdout: outcome.stdout,
        stderr: outcome.stderr,
        command: None,
    };
    Ok(match data {
        Some(data) => ports::HandlerOutcome::with_data(adapter, data),
        None => ports::HandlerOutcome::plain(adapter),
    })
}

fn code_intelligence_read_request(
    operation: CodeIntelligenceOperation,
    args: &Map<String, Value>,
) -> Result<CodeIntelligenceReadRequest, String> {
    match operation {
        CodeIntelligenceOperation::Definition => Ok(CodeIntelligenceReadRequest::Definition {
            name: required_code_intelligence_string(args, "name")?.to_string(),
            module_hint: args
                .get("moduleHint")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string(),
            limit: code_intelligence_limit(args, 50),
        }),
        CodeIntelligenceOperation::Outline => Ok(CodeIntelligenceReadRequest::Outline {
            path: required_code_intelligence_string(args, "path")?.to_string(),
            include_methods: args
                .get("includeMethods")
                .and_then(Value::as_bool)
                .unwrap_or(true),
        }),
        CodeIntelligenceOperation::Search => {
            Err("search cannot be built as a code intelligence read request".to_string())
        }
    }
}

fn required_code_intelligence_string<'a>(
    args: &'a Map<String, Value>,
    name: &str,
) -> Result<&'a str, String> {
    args.get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("missing required `{name}` argument"))
}

fn code_intelligence_limit(args: &Map<String, Value>, default: usize) -> usize {
    args.get("limit")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(default)
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

fn merge_handler_diagnostics(handler: Option<Value>, orchestrator: Option<Value>) -> Option<Value> {
    match (handler, orchestrator) {
        (None, None) => None,
        (Some(handler), None) => Some(handler),
        (None, Some(orchestrator)) => Some(orchestrator),
        (Some(handler), Some(orchestrator)) => Some(json!({
            "handler": handler,
            "orchestrator": orchestrator,
        })),
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
    data: Option<&Value>,
) -> bool {
    if !spec.execution.is_mutating() || !outcome.ok {
        return false;
    }
    if !dry_run {
        return if spec.name == "unica.role.edit" {
            data.and_then(|data| data.get("changed"))
                .and_then(Value::as_bool)
                == Some(true)
        } else if spec.name == "unica.xdto.edit" {
            data.and_then(|data| data.get("noOp"))
                .and_then(Value::as_bool)
                == Some(false)
        } else {
            !outcome.changes.is_empty()
        };
    }

    if spec.name == "unica.role.edit" {
        return data
            .and_then(|data| data.get("changed"))
            .and_then(Value::as_bool)
            == Some(true);
    }

    if spec.name == "unica.xdto.edit" {
        return data
            .and_then(|data| data.get("noOp"))
            .and_then(Value::as_bool)
            == Some(false);
    }

    if spec.name == "unica.code.patch" {
        return false;
    }
    let is_semantic_form_edit_preview = spec.name == "unica.form.edit"
        && args.keys().any(|key| {
            matches!(
                key.as_str(),
                "FormPath" | "formPath" | "Path" | "path" | "JsonPath" | "jsonPath" | "definition"
            )
        });
    !is_semantic_form_edit_preview
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
        ToolHandler::SourceNavigation { .. } => Vec::new(),
        _ => Vec::new(),
    }
}

fn runtime_event(args: &Map<String, Value>) -> Option<DomainEventKind> {
    args.get("operation")
        .and_then(Value::as_str)
        .and_then(runtime_event_kind)
}

/// A read whose result is data: ADR-0023 keeps the typed payload out of
/// `stdout`, so the caller reads fields instead of parsing a rendered report.
pub(crate) struct TypedReadOutcome {
    pub(crate) outcome: AdapterOutcome,
    pub(crate) data: Option<Value>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProjectStatusData {
    workspace_root: String,
    cache_root: String,
    /// `null` when discovery failed: the caller must not read an empty list as
    /// a workspace that has no source sets.
    source_sets: Option<Vec<crate::domain::project_sources::ProjectSourceSet>>,
}

pub(crate) fn project_status(
    context: &WorkspaceContext,
    source_map: Result<crate::domain::project_sources::ProjectSourceMap, String>,
    tracked_config_dump_info_warning: Option<String>,
) -> TypedReadOutcome {
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
    let source_sets = match source_map {
        Ok(source_map) => {
            outcome
                .summary
                .push_str(&format!("; source sets: {}", source_map.source_sets.len()));
            Some(source_map.source_sets)
        }
        Err(error) => {
            outcome
                .warnings
                .push(format!("source-set discovery failed: {error}"));
            None
        }
    };
    if let Some(warning) = tracked_config_dump_info_warning {
        outcome.warnings.push(warning);
    }
    let data = serde_json::to_value(ProjectStatusData {
        workspace_root: context.workspace_root.display().to_string(),
        cache_root: context.cache_root.display().to_string(),
        source_sets,
    })
    .expect("project status data serializes");
    TypedReadOutcome {
        outcome,
        data: Some(data),
    }
}

pub(crate) fn project_map(
    source_map: Result<crate::domain::project_sources::ProjectSourceMap, String>,
    tracked_config_dump_info_warning: Option<String>,
) -> TypedReadOutcome {
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
            // The map used to be serialized into `stdout`, which put a JSON
            // string inside the JSON envelope -- exactly the shape ADR-0020
            // rejected.
            let data = serde_json::to_value(&source_map).expect("source map serializes");
            TypedReadOutcome {
                outcome,
                data: Some(data),
            }
        }
        Err(error) => TypedReadOutcome {
            outcome: AdapterOutcome {
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
            data: None,
        },
    }
}

fn configuration_tools() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "unica.cf.edit",
            description:
                "Edit root Configuration.xml properties, ChildObjects, panels, and home page.",
            execution: ToolExecution::Mutation,
            result_contract: ResultContract::Typed,
            cache_access: cache_access_for("cf-edit", Some(DomainEventKind::ConfigXmlChanged)),
            handler: ToolHandler::NativeOperation {
                operation: "cf-edit",
                event: Some(DomainEventKind::ConfigXmlChanged),
            },
        },
        ToolSpec {
            name: "unica.cf.info",
            description: "Inspect root Configuration.xml.",
            execution: ToolExecution::Read,
            result_contract: ResultContract::Typed,
            cache_access: cache_access_for("cf-info", None),
            handler: ToolHandler::NativeOperation {
                operation: "cf-info",
                event: None,
            },
        },
        ToolSpec {
            name: "unica.cf.init",
            description: "Create empty 1C configuration XML scaffold.",
            execution: ToolExecution::Mutation,
            result_contract: ResultContract::Typed,
            cache_access: cache_access_for("cf-init", Some(DomainEventKind::ConfigXmlChanged)),
            handler: ToolHandler::NativeOperation {
                operation: "cf-init",
                event: Some(DomainEventKind::ConfigXmlChanged),
            },
        },
        ToolSpec {
            name: "unica.cf.validate",
            description: "Validate root configuration XML structure.",
            execution: ToolExecution::Read,
            result_contract: ResultContract::ExternalStream,
            cache_access: cache_access_for("cf-validate", None),
            handler: ToolHandler::NativeOperation {
                operation: "cf-validate",
                event: None,
            },
        },
        ToolSpec {
            name: "unica.support.edit",
            description: "Toggle 1C vendor support editing capability or per-object support rule.",
            execution: ToolExecution::Mutation,
            result_contract: ResultContract::Typed,
            cache_access: cache_access_for("support-edit", Some(DomainEventKind::ConfigXmlChanged)),
            handler: ToolHandler::NativeOperation {
                operation: "support-edit",
                event: Some(DomainEventKind::ConfigXmlChanged),
            },
        },
        ToolSpec {
            name: "unica.cfe.borrow",
            description: "Borrow configuration objects/forms into an extension.",
            execution: ToolExecution::Mutation,
            result_contract: ResultContract::Typed,
            cache_access: cache_access_for("cfe-borrow", Some(DomainEventKind::CfeChanged)),
            handler: ToolHandler::NativeOperation {
                operation: "cfe-borrow",
                event: Some(DomainEventKind::CfeChanged),
            },
        },
        ToolSpec {
            name: "unica.cfe.diff",
            description: "Inspect extension contents and transferred insertion blocks.",
            execution: ToolExecution::Read,
            result_contract: ResultContract::Typed,
            cache_access: cache_access_for("cfe-diff", None),
            handler: ToolHandler::NativeOperation {
                operation: "cfe-diff",
                event: None,
            },
        },
        ToolSpec {
            name: "unica.cfe.init",
            description: "Create extension XML scaffold.",
            execution: ToolExecution::Mutation,
            result_contract: ResultContract::Typed,
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
            execution: ToolExecution::Mutation,
            result_contract: ResultContract::Typed,
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
            execution: ToolExecution::Mutation,
            result_contract: ResultContract::Typed,
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
            execution: ToolExecution::Mutation,
            result_contract: ResultContract::Typed,
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
            execution: ToolExecution::Read,
            result_contract: ResultContract::ExternalStream,
            cache_access: cache_access_for("cfe-validate", None),
            handler: ToolHandler::NativeOperation {
                operation: "cfe-validate",
                event: None,
            },
        },
        ToolSpec {
            name: "unica.meta.info",
            description: "Inspect one metadata object with validation, proven subsystem memberships, and source-tree usage.",
            execution: ToolExecution::Read,
            result_contract: ResultContract::Typed,
            cache_access: CacheAccess {
                reads: &["workspace_graph", "metadata_graph"],
                writes: &[],
            },
            handler: ToolHandler::Metadata {
                operation: metadata::MetadataOperation::Info,
            },
        },
        ToolSpec {
            name: "unica.meta.add",
            description: "Create one metadata object from a typed internal template and optionally configure it atomically with ordered operations.",
            execution: ToolExecution::Mutation,
            result_contract: ResultContract::Typed,
            cache_access: CacheAccess {
                reads: &[],
                writes: &["workspace_graph", "metadata_graph"],
            },
            handler: ToolHandler::Metadata {
                operation: metadata::MetadataOperation::Add,
            },
        },
        ToolSpec {
            name: "unica.meta.edit",
            description: "Apply ordered typed metadata edit operations atomically.",
            execution: ToolExecution::Mutation,
            result_contract: ResultContract::Typed,
            cache_access: CacheAccess {
                reads: &[],
                writes: &["workspace_graph", "metadata_graph"],
            },
            handler: ToolHandler::Metadata {
                operation: metadata::MetadataOperation::Edit,
            },
        },
        ToolSpec {
            name: "unica.meta.remove",
            description: "Remove one metadata object through a logical guarded target.",
            execution: ToolExecution::Mutation,
            result_contract: ResultContract::Typed,
            cache_access: CacheAccess {
                reads: &[],
                writes: &["workspace_graph", "metadata_graph"],
            },
            handler: ToolHandler::Metadata {
                operation: metadata::MetadataOperation::Remove,
            },
        },
        ToolSpec {
            name: "unica.help.add",
            description: "Add built-in help metadata and page to a 1C object.",
            execution: ToolExecution::Mutation,
            result_contract: ResultContract::Typed,
            cache_access: cache_access_for("help-add", Some(DomainEventKind::FormChanged)),
            handler: ToolHandler::NativeOperation {
                operation: "help-add",
                event: Some(DomainEventKind::FormChanged),
            },
        },
        ToolSpec {
            name: "unica.form.add",
            description: "Add managed form metadata and files.",
            execution: ToolExecution::Mutation,
            result_contract: ResultContract::Typed,
            cache_access: cache_access_for("form-add", Some(DomainEventKind::FormChanged)),
            handler: ToolHandler::NativeOperation {
                operation: "form-add",
                event: Some(DomainEventKind::FormChanged),
            },
        },
        ToolSpec {
            name: "unica.form.compile",
            description: "Compile managed Form.xml from JSON DSL or metadata.",
            execution: ToolExecution::Mutation,
            result_contract: ResultContract::ExternalStream,
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
            execution: ToolExecution::Mutation,
            result_contract: ResultContract::Typed,
            cache_access: cache_access_for("form-edit", Some(DomainEventKind::FormChanged)),
            handler: ToolHandler::NativeOperation {
                operation: "form-edit",
                event: Some(DomainEventKind::FormChanged),
            },
        },
        ToolSpec {
            name: "unica.form.info",
            description: "Inspect managed Form.xml.",
            execution: ToolExecution::Read,
            result_contract: ResultContract::Typed,
            cache_access: cache_access_for("form-info", None),
            handler: ToolHandler::NativeOperation {
                operation: "form-info",
                event: None,
            },
        },
        ToolSpec {
            name: "unica.form.remove",
            description: "Remove a managed form and registration.",
            execution: ToolExecution::Mutation,
            result_contract: ResultContract::Typed,
            cache_access: cache_access_for("form-remove", Some(DomainEventKind::FormChanged)),
            handler: ToolHandler::NativeOperation {
                operation: "form-remove",
                event: Some(DomainEventKind::FormChanged),
            },
        },
        ToolSpec {
            name: "unica.form.validate",
            description: "Validate managed Form.xml.",
            execution: ToolExecution::Read,
            result_contract: ResultContract::ExternalStream,
            cache_access: cache_access_for("form-validate", None),
            handler: ToolHandler::NativeOperation {
                operation: "form-validate",
                event: None,
            },
        },
        ToolSpec {
            name: "unica.interface.edit",
            description: "Edit subsystem CommandInterface.xml.",
            execution: ToolExecution::Mutation,
            result_contract: ResultContract::Typed,
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
            execution: ToolExecution::Read,
            result_contract: ResultContract::ExternalStream,
            cache_access: cache_access_for("interface-validate", None),
            handler: ToolHandler::NativeOperation {
                operation: "interface-validate",
                event: None,
            },
        },
        ToolSpec {
            name: "unica.subsystem.compile",
            description: "Compile subsystem XML from JSON DSL.",
            execution: ToolExecution::Mutation,
            result_contract: ResultContract::ExternalStream,
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
            execution: ToolExecution::Mutation,
            result_contract: ResultContract::Typed,
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
            description: "Inspect a registered subsystem tree from a directory, a focused registered tree from XML, or an unregistered XML locally.",
            execution: ToolExecution::Read,
            result_contract: ResultContract::Typed,
            cache_access: cache_access_for("subsystem-info", None),
            handler: ToolHandler::NativeOperation {
                operation: "subsystem-info",
                event: None,
            },
        },
        ToolSpec {
            name: "unica.subsystem.validate",
            description: "Validate subsystem XML.",
            execution: ToolExecution::Read,
            result_contract: ResultContract::ExternalStream,
            cache_access: cache_access_for("subsystem-validate", None),
            handler: ToolHandler::NativeOperation {
                operation: "subsystem-validate",
                event: None,
            },
        },
        ToolSpec {
            name: "unica.template.add",
            description: "Add a template to an object and register it.",
            execution: ToolExecution::Mutation,
            result_contract: ResultContract::Typed,
            cache_access: cache_access_for("template-add", Some(DomainEventKind::TemplateChanged)),
            handler: ToolHandler::NativeOperation {
                operation: "template-add",
                event: Some(DomainEventKind::TemplateChanged),
            },
        },
        ToolSpec {
            name: "unica.template.remove",
            description: "Remove a template from an object.",
            execution: ToolExecution::Mutation,
            result_contract: ResultContract::Typed,
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
            execution: ToolExecution::Mutation,
            result_contract: ResultContract::ExternalStream,
            cache_access: cache_access_for("dcs-compile", Some(DomainEventKind::DcsChanged)),
            handler: ToolHandler::NativeOperation {
                operation: "dcs-compile",
                event: Some(DomainEventKind::DcsChanged),
            },
        },
        ToolSpec {
            name: "unica.dcs.edit",
            description: "Edit Data Composition Schema Template.xml.",
            execution: ToolExecution::Mutation,
            result_contract: ResultContract::Typed,
            cache_access: cache_access_for("dcs-edit", Some(DomainEventKind::DcsChanged)),
            handler: ToolHandler::NativeOperation {
                operation: "dcs-edit",
                event: Some(DomainEventKind::DcsChanged),
            },
        },
        ToolSpec {
            name: "unica.dcs.info",
            description: "Inspect Data Composition Schema Template.xml.",
            execution: ToolExecution::Read,
            result_contract: ResultContract::Typed,
            cache_access: cache_access_for("dcs-info", None),
            handler: ToolHandler::NativeOperation {
                operation: "dcs-info",
                event: None,
            },
        },
        ToolSpec {
            name: "unica.dcs.validate",
            description: "Validate Data Composition Schema Template.xml.",
            execution: ToolExecution::Read,
            result_contract: ResultContract::ExternalStream,
            cache_access: cache_access_for("dcs-validate", None),
            handler: ToolHandler::NativeOperation {
                operation: "dcs-validate",
                event: None,
            },
        },
        ToolSpec {
            name: "unica.mxl.compile",
            description: "Compile spreadsheet Template.xml from JSON DSL.",
            execution: ToolExecution::Mutation,
            result_contract: ResultContract::ExternalStream,
            cache_access: cache_access_for("mxl-compile", Some(DomainEventKind::MxlChanged)),
            handler: ToolHandler::NativeOperation {
                operation: "mxl-compile",
                event: Some(DomainEventKind::MxlChanged),
            },
        },
        ToolSpec {
            name: "unica.mxl.decompile",
            description: "Decompile spreadsheet Template.xml to JSON DSL.",
            execution: ToolExecution::Read,
            result_contract: ResultContract::ExternalStream,
            cache_access: cache_access_for("mxl-decompile", None),
            handler: ToolHandler::NativeOperation {
                operation: "mxl-decompile",
                event: None,
            },
        },
        ToolSpec {
            name: "unica.mxl.info",
            description: "Inspect spreadsheet Template.xml.",
            execution: ToolExecution::Read,
            result_contract: ResultContract::Typed,
            cache_access: cache_access_for("mxl-info", None),
            handler: ToolHandler::NativeOperation {
                operation: "mxl-info",
                event: None,
            },
        },
        ToolSpec {
            name: "unica.mxl.validate",
            description: "Validate spreadsheet Template.xml.",
            execution: ToolExecution::Read,
            result_contract: ResultContract::ExternalStream,
            cache_access: cache_access_for("mxl-validate", None),
            handler: ToolHandler::NativeOperation {
                operation: "mxl-validate",
                event: None,
            },
        },
        ToolSpec {
            name: "unica.role.compile",
            description: "Compile role metadata and Rights.xml from JSON DSL.",
            execution: ToolExecution::Mutation,
            result_contract: ResultContract::ExternalStream,
            cache_access: cache_access_for("role-compile", Some(DomainEventKind::RoleChanged)),
            handler: ToolHandler::NativeOperation {
                operation: "role-compile",
                event: Some(DomainEventKind::RoleChanged),
            },
        },
        ToolSpec {
            name: "unica.role.edit",
            description: "Edit role rights through a closed logical typed contract.",
            execution: ToolExecution::Mutation,
            result_contract: ResultContract::Typed,
            cache_access: CacheAccess {
                reads: &[],
                writes: &["metadata_graph", "rights_graph"],
            },
            handler: ToolHandler::NativeOperation {
                operation: "role-edit",
                event: Some(DomainEventKind::RoleChanged),
            },
        },
        ToolSpec {
            name: "unica.role.info",
            description: "Inspect role Rights.xml.",
            execution: ToolExecution::Read,
            result_contract: ResultContract::Typed,
            cache_access: cache_access_for("role-info", None),
            handler: ToolHandler::NativeOperation {
                operation: "role-info",
                event: None,
            },
        },
        ToolSpec {
            name: "unica.role.validate",
            description: "Validate role Rights.xml.",
            execution: ToolExecution::Read,
            result_contract: ResultContract::ExternalStream,
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
        child_subsystem_stub_xml, create_file_link_fixture_for_test, file_identity_for_test,
        prepare_file_for_removal, set_unix_mode_for_test, unix_mode_for_test,
        with_publication_lock_contention_signal, with_publication_lock_pause,
        with_secure_tree_test_hook, CompileTransaction, FileLinkFixtureOutcome, SecureTreePhase,
    };
    use serde_json::Map;
    use std::cell::Cell;
    use std::rc::Rc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;
    use std::sync::{mpsc, Arc, Barrier};
    use std::thread;
    use std::time::Duration;

    fn normalized_path(path: &std::path::Path) -> std::path::PathBuf {
        let canonical = std::fs::canonicalize(path).expect("test path identity must canonicalize");
        if std::path::MAIN_SEPARATOR == '\\' {
            let display = canonical.to_string_lossy();
            if let Some(path) = display.strip_prefix(r"\\?\UNC\") {
                return std::path::PathBuf::from(format!(r"\\{path}"));
            }
            if let Some(path) = display.strip_prefix(r"\\?\") {
                return std::path::PathBuf::from(path);
            }
        }
        canonical
    }

    fn call_public_tool_from_workspace(
        workspace: &std::path::Path,
        name: &str,
        args: &Map<String, Value>,
    ) -> Result<OperationResult, String> {
        let _cwd = crate::test_support::ProcessCwdGuard::enter(workspace)?;
        UnicaApplication::new().call_tool(name, args)
    }

    fn path_text(path: &std::path::Path) -> String {
        path.display().to_string().replace('\\', "/")
    }

    #[derive(Default)]
    struct RejectDiscoveryPorts {
        discovery_calls: AtomicUsize,
    }

    impl ports::ApplicationPorts for RejectDiscoveryPorts {
        fn discover_workspace(
            &self,
            _requested_cwd: Option<PathBuf>,
        ) -> Result<WorkspaceContext, String> {
            self.discovery_calls.fetch_add(1, Ordering::SeqCst);
            panic!("reader argument validation must run before workspace discovery")
        }

        fn validate_tool_context(
            &self,
            _spec: ToolSpec,
            _args: &Map<String, Value>,
            _mode: InvocationMode,
            _context: &WorkspaceContext,
        ) -> Result<(), String> {
            unreachable!("workspace discovery must not run")
        }

        fn evaluate_support_guard(
            &self,
            _spec: ToolSpec,
            _args: &Map<String, Value>,
            _context: &WorkspaceContext,
        ) -> Result<SupportGuardCheck, String> {
            unreachable!("workspace discovery must not run")
        }

        fn invoke_handler(
            &self,
            _spec: ToolSpec,
            _args: &Map<String, Value>,
            _context: &WorkspaceContext,
            _mode: InvocationMode,
            _cancellation: &CancellationToken,
        ) -> Result<ports::HandlerOutcome, String> {
            unreachable!("workspace discovery must not run")
        }

        fn cache_report(
            &self,
            _context: &WorkspaceContext,
            _events: &[DomainEvent],
            _mode: InvocationMode,
            _cache_access: CacheAccess,
        ) -> Result<CacheReport, String> {
            unreachable!("workspace discovery must not run")
        }

        fn notify_invalidation(&self, _context: &WorkspaceContext, _events: &[DomainEvent]) {
            unreachable!("workspace discovery must not run")
        }
    }

    #[test]
    fn reader_rejects_dry_run_before_workspace_discovery() {
        let ports = RejectDiscoveryPorts::default();

        for spec in tools()
            .into_iter()
            .filter(|tool| tool.execution == ToolExecution::Read)
        {
            for value in [true, false] {
                let mut args = Map::new();
                args.insert("dryRun".to_string(), Value::Bool(value));

                let error = call_tool(
                    spec,
                    &args,
                    &ports,
                    &CancellationToken::new(),
                    ProviderDeadline::from_budget(Duration::from_secs(1)),
                )
                .expect_err("reader must reject dryRun");
                let expected = if matches!(spec.handler, ToolHandler::Metadata { .. }) {
                    "metadata operation does not accept argument `dryRun`"
                } else {
                    "does not accept argument `dryRun`"
                };
                assert!(error.contains(expected), "{}: {error}", spec.name);
                assert_eq!(
                    ports.discovery_calls.load(Ordering::SeqCst),
                    0,
                    "{} reached workspace discovery",
                    spec.name,
                );
            }
        }
    }

    #[test]
    fn invocation_mode_is_derived_from_validated_tool_execution() {
        let reader = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.project.status")
            .expect("project.status reader exists");
        let mutation = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.cf.edit")
            .expect("cf.edit mutation exists");

        assert_eq!(
            InvocationMode::from_validated_args(reader, &Map::new()).unwrap(),
            InvocationMode::Read,
        );
        assert_eq!(
            InvocationMode::from_validated_args(mutation, &Map::new()).unwrap(),
            InvocationMode::Preview,
        );
        assert_eq!(
            InvocationMode::from_validated_args(
                mutation,
                serde_json::json!({"dryRun": false}).as_object().unwrap(),
            )
            .unwrap(),
            InvocationMode::Apply,
        );
    }

    #[derive(Default)]
    struct OperationalConfigRecordingPorts {
        load_calls: AtomicUsize,
        prepare_calls: AtomicUsize,
        handler_calls: AtomicUsize,
        code_context_calls: AtomicUsize,
        fail_load: bool,
        cancellation_on_load: Option<CancellationToken>,
        prepared_code_search_handler: bool,
        full_range_workspace: Option<PathBuf>,
        observed_analyze_timeout: Mutex<Option<Duration>>,
    }

    impl OperationalConfigRecordingPorts {
        fn failing() -> Self {
            Self {
                fail_load: true,
                ..Self::default()
            }
        }

        fn failing_and_cancelling(cancellation: CancellationToken) -> Self {
            Self {
                fail_load: true,
                cancellation_on_load: Some(cancellation),
                ..Self::default()
            }
        }

        fn with_prepared_code_search_handler() -> Self {
            Self {
                prepared_code_search_handler: true,
                ..Self::default()
            }
        }

        fn with_full_range_code_provider(workspace: PathBuf) -> Self {
            Self {
                full_range_workspace: Some(workspace),
                ..Self::default()
            }
        }
    }

    struct FullRangeReadProvider;

    impl crate::domain::code_intelligence::CodeIntelligenceProvider for FullRangeReadProvider {
        fn id(&self) -> crate::domain::code_intelligence::ProviderId {
            crate::domain::code_intelligence::ProviderId::Rlm
        }

        fn capabilities(&self) -> &[crate::domain::code_intelligence::ProviderCapability] {
            &[
                crate::domain::code_intelligence::ProviderCapability::Definition,
                crate::domain::code_intelligence::ProviderCapability::Outline,
            ]
        }

        fn search(
            &self,
            _request: &SearchRequest,
            _context: &crate::domain::code_intelligence::CodeIntelligenceContext,
            _deadline: ProviderDeadline,
            _cancellation: &CancellationToken,
        ) -> crate::domain::code_intelligence::ProviderSearchSection {
            unreachable!("read-only fixture")
        }

        fn read(
            &self,
            request: &CodeIntelligenceReadRequest,
            _context: &crate::domain::code_intelligence::CodeIntelligenceContext,
            _deadline: ProviderDeadline,
            _cancellation: &CancellationToken,
        ) -> Result<crate::domain::code_intelligence::ProviderReadOutcome, String> {
            let data = match request {
                CodeIntelligenceReadRequest::Definition { name, .. } => {
                    crate::domain::code_intelligence::CodeIntelligenceReadData::Definition(
                        crate::domain::code_intelligence::CodeDefinitionResult {
                            name: name.clone(),
                            definitions: Vec::new(),
                        },
                    )
                }
                CodeIntelligenceReadRequest::Outline { path, .. } => {
                    crate::domain::code_intelligence::CodeIntelligenceReadData::Outline(
                        crate::domain::code_intelligence::CodeOutlineResult {
                            module: path.clone(),
                            identity: Default::default(),
                            totals: crate::domain::code_intelligence::CodeOutlineTotals {
                                methods: 0,
                                exports: 0,
                                regions: 0,
                                loc: 0,
                            },
                            regions: Vec::new(),
                            methods: Vec::new(),
                        },
                    )
                }
            };
            Ok(crate::domain::code_intelligence::ProviderReadOutcome {
                provider: crate::domain::code_intelligence::ProviderId::Rlm,
                ok: true,
                summary: "read".to_string(),
                warnings: Vec::new(),
                errors: Vec::new(),
                artifacts: Vec::new(),
                stdout: None,
                stderr: None,
                data: Some(data),
            })
        }
    }

    impl ports::ApplicationPorts for OperationalConfigRecordingPorts {
        fn discover_workspace(
            &self,
            requested_cwd: Option<PathBuf>,
        ) -> Result<WorkspaceContext, String> {
            let cwd = self
                .full_range_workspace
                .clone()
                .or(requested_cwd)
                .unwrap_or_else(|| PathBuf::from("/workspace"));
            Ok(WorkspaceContext {
                cwd: cwd.clone(),
                workspace_root: cwd.clone(),
                cache_root: cwd.join(".build/unica"),
                workspace_epoch: 1,
            })
        }

        fn validate_tool_context(
            &self,
            _spec: ToolSpec,
            _args: &Map<String, Value>,
            _mode: InvocationMode,
            _context: &WorkspaceContext,
        ) -> Result<(), String> {
            Ok(())
        }

        fn load_operational_config(
            &self,
            _context: &WorkspaceContext,
        ) -> Result<
            crate::domain::operational_config::OperationalConfig,
            crate::domain::operational_config::OperationalConfigDiagnostic,
        > {
            self.load_calls.fetch_add(1, Ordering::SeqCst);
            if let Some(cancellation) = &self.cancellation_on_load {
                cancellation.cancel();
            }
            if self.fail_load {
                return Err(
                    crate::domain::operational_config::OperationalConfigDiagnostic::new(
                        crate::domain::operational_config::OperationalConfigDiagnosticCode::InvalidToml,
                        crate::domain::operational_config::OperationalConfigDiagnosticSource::Shared,
                        "$",
                    ),
                );
            }
            if self.full_range_workspace.is_some() {
                let mut layer =
                    crate::domain::operational_config::OperationalConfigLayer::default();
                layer.set_timeout_seconds(
                    crate::domain::operational_config::OperationalConfigField::ProviderRead,
                    i64::MAX,
                    crate::domain::operational_config::OperationalConfigDiagnosticSource::Shared,
                )?;
                return crate::domain::operational_config::OperationalConfig::from_layers(
                    Some(&layer),
                    None,
                );
            }
            Ok(crate::domain::operational_config::OperationalConfig::compiled_defaults())
        }

        fn prepare_tool_invocation(
            &self,
            spec: ToolSpec,
            _args: &Map<String, Value>,
            _context: &WorkspaceContext,
            _mode: InvocationMode,
            _cancellation: &CancellationToken,
            _deadline: ProviderDeadline,
        ) -> Result<ports::PreparedToolInvocation, String> {
            self.prepare_calls.fetch_add(1, Ordering::SeqCst);
            if self.prepared_code_search_handler && spec.name == "unica.code.search" {
                return Ok(ports::PreparedToolInvocation {
                    format_guard: None,
                    handler: Some(ports::HandlerOutcome::with_data(
                        AdapterOutcome::ok("prepared code search"),
                        json!({"sections": []}),
                    )),
                });
            }
            Ok(ports::PreparedToolInvocation::empty())
        }

        fn evaluate_support_guard(
            &self,
            _spec: ToolSpec,
            _args: &Map<String, Value>,
            _context: &WorkspaceContext,
        ) -> Result<SupportGuardCheck, String> {
            Ok(SupportGuardCheck::Allow)
        }

        fn resolve_code_intelligence_context(
            &self,
            context: &WorkspaceContext,
            _args: &Map<String, Value>,
        ) -> Result<crate::domain::code_intelligence::CodeIntelligenceContext, String> {
            self.code_context_calls.fetch_add(1, Ordering::SeqCst);
            if self.full_range_workspace.is_some() {
                return Ok(
                    crate::domain::code_intelligence::CodeIntelligenceContext::new(
                        context.clone(),
                        crate::domain::source_roots::ResolvedSourceRoot {
                            source_set: Some("main".to_string()),
                            path: context.workspace_root.join("src"),
                        },
                    ),
                );
            }
            Err("code intelligence context should not be resolved in this test".to_string())
        }

        fn normalize_code_intelligence_read_request(
            &self,
            request: CodeIntelligenceReadRequest,
            _context: &crate::domain::code_intelligence::CodeIntelligenceContext,
        ) -> Result<CodeIntelligenceReadRequest, String> {
            if self.full_range_workspace.is_some() {
                return Ok(request);
            }
            Err("code intelligence request should not be normalized in this test".to_string())
        }

        fn code_intelligence_registry(
            &self,
        ) -> Result<crate::domain::code_intelligence::CodeIntelligenceRegistry, String> {
            if self.full_range_workspace.is_some() {
                return crate::domain::code_intelligence::CodeIntelligenceRegistry::new(vec![
                    Arc::new(FullRangeReadProvider),
                ]);
            }
            Err("code intelligence registry should not be read in this test".to_string())
        }

        fn invoke_handler(
            &self,
            spec: ToolSpec,
            _args: &Map<String, Value>,
            _context: &WorkspaceContext,
            _mode: InvocationMode,
            _cancellation: &CancellationToken,
        ) -> Result<ports::HandlerOutcome, String> {
            self.handler_calls.fetch_add(1, Ordering::SeqCst);
            let outcome = AdapterOutcome::ok("handled");
            Ok(
                if spec.execution == ToolExecution::Read
                    && spec.result_contract == ResultContract::Typed
                {
                    ports::HandlerOutcome::with_data(outcome, json!({"fixture": true}))
                } else {
                    ports::HandlerOutcome::plain(outcome)
                },
            )
        }

        fn invoke_handler_with_operational_config(
            &self,
            spec: ToolSpec,
            _args: &Map<String, Value>,
            _context: &WorkspaceContext,
            _mode: InvocationMode,
            operational_config: Option<&crate::domain::operational_config::OperationalConfig>,
            _cancellation: &CancellationToken,
        ) -> Result<ports::HandlerOutcome, String> {
            self.handler_calls.fetch_add(1, Ordering::SeqCst);
            *self.observed_analyze_timeout.lock().unwrap() =
                operational_config.map(|config| config.code_diagnostics().analyze_timeout());
            let outcome = AdapterOutcome::ok("handled");
            Ok(
                if spec.execution == ToolExecution::Read
                    && spec.result_contract == ResultContract::Typed
                {
                    ports::HandlerOutcome::with_data(outcome, json!({"fixture": true}))
                } else {
                    ports::HandlerOutcome::plain(outcome)
                },
            )
        }

        fn cache_report(
            &self,
            context: &WorkspaceContext,
            _events: &[DomainEvent],
            mode: InvocationMode,
            _cache_access: CacheAccess,
        ) -> Result<CacheReport, String> {
            Ok(CacheReport {
                mode: if mode.is_preview() { "dry-run" } else { "read" }.to_string(),
                root: context.cache_root.display().to_string(),
                workspace_epoch: context.workspace_epoch,
                events: Vec::new(),
                invalidated: Vec::new(),
                refreshed: Vec::new(),
                lazy_rebuilt: Vec::new(),
                stale: Vec::new(),
                fresh: Vec::new(),
                publication_warnings: Vec::new(),
            })
        }

        fn notify_invalidation(&self, _context: &WorkspaceContext, _events: &[DomainEvent]) {}
    }

    #[test]
    fn operational_config_is_loaded_once_only_for_affected_calls() {
        let ports = Arc::new(OperationalConfigRecordingPorts::default());
        let app = UnicaApplication::with_ports(ports.clone());

        let mut status = Map::new();
        status.insert("mode".to_string(), json!("status"));
        app.call_tool("unica.code.diagnostics", &status).unwrap();
        assert_eq!(ports.load_calls.load(Ordering::SeqCst), 0);

        let mut analyze = Map::new();
        analyze.insert("timeoutSeconds".to_string(), json!(900));
        app.call_tool("unica.code.diagnostics", &analyze).unwrap();
        assert_eq!(ports.load_calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            *ports.observed_analyze_timeout.lock().unwrap(),
            Some(Duration::from_secs(900))
        );

        app.call_tool("unica.project.status", &Map::new()).unwrap();
        assert_eq!(ports.load_calls.load(Ordering::SeqCst), 1);
        assert_eq!(ports.handler_calls.load(Ordering::SeqCst), 3);

        for (tool_name, args) in [
            ("unica.code.search", json!({"query": "needle"})),
            ("unica.code.definition", json!({"name": "Needle"})),
            ("unica.code.outline", json!({"path": "Module.bsl"})),
        ] {
            let error = app
                .call_tool(tool_name, args.as_object().unwrap())
                .unwrap_err();
            assert!(error.contains("should not be resolved"), "{error}");
        }
        assert_eq!(ports.load_calls.load(Ordering::SeqCst), 4);
        assert_eq!(ports.code_context_calls.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn prepared_code_search_handler_wins_after_required_operational_config_resolution() {
        let ports = Arc::new(OperationalConfigRecordingPorts::with_prepared_code_search_handler());
        let app = UnicaApplication::with_ports(ports.clone());

        let result = app
            .call_tool(
                "unica.code.search",
                json!({"query": "needle"}).as_object().unwrap(),
            )
            .expect("prepared handler must serve code search");

        assert!(result.ok, "{result:?}");
        assert_eq!(result.summary, "prepared code search");
        assert_eq!(ports.prepare_calls.load(Ordering::SeqCst), 1);
        assert_eq!(ports.load_calls.load(Ordering::SeqCst), 1);
        assert_eq!(ports.code_context_calls.load(Ordering::SeqCst), 0);
        assert_eq!(ports.handler_calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn invalid_operational_config_stops_before_handler_execution() {
        let cases = [
            ("unica.code.search", json!({"query": "needle"})),
            ("unica.code.definition", json!({"name": "Needle"})),
            ("unica.code.outline", json!({"path": "Module.bsl"})),
            ("unica.code.diagnostics", json!({"timeoutSeconds": 900})),
        ];

        for (tool_name, args) in cases {
            let ports = Arc::new(OperationalConfigRecordingPorts::failing());
            let app = UnicaApplication::with_ports(ports.clone());
            let result = app.call_tool(tool_name, args.as_object().unwrap()).unwrap();

            assert!(!result.ok, "{tool_name}");
            assert_eq!(ports.load_calls.load(Ordering::SeqCst), 1, "{tool_name}");
            assert_eq!(ports.handler_calls.load(Ordering::SeqCst), 0, "{tool_name}");
            assert_eq!(
                ports.code_context_calls.load(Ordering::SeqCst),
                0,
                "{tool_name}"
            );
            let diagnostic = &result.diagnostics.unwrap()["operationalConfig"];
            assert_eq!(diagnostic["source"], "unica.toml");
            assert_eq!(diagnostic["fieldPath"], "$");
        }
    }

    #[test]
    fn cancellation_during_failed_operational_config_load_wins() {
        let token = CancellationToken::new();
        let ports = Arc::new(OperationalConfigRecordingPorts::failing_and_cancelling(
            token.clone(),
        ));
        let app = UnicaApplication::with_ports(ports);

        let error = app
            .call_tool_cancellable(
                "unica.code.search",
                json!({"query": "needle"}).as_object().unwrap(),
                token,
            )
            .expect_err("cancellation must win over invalid config");

        assert!(
            error.starts_with(crate::domain::cancellation::CANCELLED_PREFIX),
            "{error}"
        );
    }

    #[test]
    fn public_definition_and_outline_accept_full_positive_i64_config_budget() {
        let workspace = std::env::temp_dir().join(format!(
            "unica-public-full-range-read-{}",
            std::process::id()
        ));
        let app = UnicaApplication::with_ports(Arc::new(
            OperationalConfigRecordingPorts::with_full_range_code_provider(workspace.clone()),
        ));

        for (tool_name, args) in [
            ("unica.code.definition", json!({"name": "Needle"})),
            ("unica.code.outline", json!({"path": "Module.bsl"})),
        ] {
            let result = app
                .call_tool(tool_name, args.as_object().unwrap())
                .expect("valid full-range config must not panic public dispatch");
            assert!(result.ok, "{tool_name}: {result:?}");
        }
    }

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
        assert!(!names.contains(&"unica.code.grep"));
        assert!(names.contains(&"unica.code.graph"));
        for name in [
            "unica.meta.info",
            "unica.meta.add",
            "unica.meta.edit",
            "unica.meta.remove",
        ] {
            assert!(names.contains(&name), "missing {name}");
        }
        for name in [
            "unica.meta.compile",
            "unica.meta.profile",
            "unica.meta.validate",
        ] {
            assert!(!names.contains(&name), "retired {name} is still public");
        }
        assert!(names.contains(&"unica.standards.explain"));
        assert!(!names.contains(&"unica-coder"));
    }

    #[test]
    fn retired_meta_routes_fail_as_unknown_tools() {
        for retired in [
            "unica.meta.compile",
            "unica.meta.profile",
            "unica.meta.validate",
        ] {
            let error = UnicaApplication::new()
                .call_tool(retired, &Map::new())
                .expect_err("retired Meta route must not dispatch");
            assert_eq!(error, format!("unknown unica tool: {retired}"));
        }
    }

    #[test]
    fn provider_neutral_tools_use_typed_code_intelligence_handlers() {
        let expected = [
            ("unica.code.search", CodeIntelligenceOperation::Search),
            (
                "unica.code.definition",
                CodeIntelligenceOperation::Definition,
            ),
            ("unica.code.outline", CodeIntelligenceOperation::Outline),
        ];

        for (name, operation) in expected {
            let tool = tools().into_iter().find(|tool| tool.name == name).unwrap();
            assert!(matches!(
                tool.handler,
                ToolHandler::CodeIntelligence {
                    operation: actual
                } if actual == operation
            ));
        }
    }

    #[test]
    fn source_navigation_tools_use_provider_neutral_application_handlers() {
        let expected = [
            ("unica.source.resolve", SourceNavigationOperation::Resolve),
            ("unica.source.children", SourceNavigationOperation::Children),
        ];

        for (name, operation) in expected {
            let tool = tools().into_iter().find(|tool| tool.name == name).unwrap();
            assert!(
                !tool.execution.is_mutating(),
                "{name} must remain read-only"
            );
            assert!(matches!(
                tool.handler,
                ToolHandler::SourceNavigation {
                    operation: actual
                } if actual == operation
            ));
        }
    }

    #[test]
    fn source_resource_tools_are_read_only_and_have_no_cache_or_event_effects() {
        let expected = [
            ("unica.source.resources", SourceResourceOperation::Resources),
            ("unica.source.read", SourceResourceOperation::Read),
        ];

        for (name, operation) in expected {
            let tool = tools().into_iter().find(|tool| tool.name == name).unwrap();
            assert!(
                !tool.execution.is_mutating(),
                "{name} must remain read-only"
            );
            assert!(
                tool.cache_access.reads.is_empty(),
                "{name} must not read cache"
            );
            assert!(
                tool.cache_access.writes.is_empty(),
                "{name} must not invalidate cache"
            );
            assert!(matches!(
                tool.handler,
                ToolHandler::SourceResources {
                    operation: actual
                } if actual == operation
            ));
        }

        // The bounded resource surface is read-only: BSL mutation belongs to
        // `unica.code.patch`, which edits the selected method or anchor instead
        // of rewriting a whole module.
        assert!(tools()
            .into_iter()
            .all(|tool| tool.name != "unica.source.apply"));
    }

    #[test]
    fn removed_code_grep_error_points_to_unified_search() {
        let error = UnicaApplication::new()
            .call_tool("unica.code.grep", &Map::new())
            .unwrap_err();

        assert!(error.contains("removed"), "{error}");
        assert!(error.contains("unica.code.search"), "{error}");
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
                    publication_warnings: Vec::new(),
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
    fn xdto_guards_project_support_deny_and_warn_to_the_logical_target() {
        let (deny_root, deny_workspace) =
            xdto_public_guard_workspace("unica-xdto-support-deny", "2.20", None);
        let deny_args = xdto_public_edit_args(&deny_workspace, "ПакетXDTO.Sample");
        let denied = UnicaApplication::new()
            .call_tool("unica.xdto.edit", &deny_args)
            .unwrap();

        assert!(!denied.ok, "{denied:?}");
        assert!(
            denied.errors.join("\n").contains("support_locked"),
            "{denied:?}"
        );
        assert_eq!(
            denied.artifacts,
            vec!["main + XDTOPackage.Sample".to_string()]
        );
        assert!(denied.data.is_none(), "{denied:?}");
        assert_xdto_public_fields_are_logical(&denied, &deny_workspace);

        let (warn_root, warn_workspace) =
            xdto_public_guard_workspace("unica-xdto-support-warn", "2.20", Some("warn"));
        let warned = UnicaApplication::new()
            .call_tool(
                "unica.xdto.edit",
                &xdto_public_edit_args(&warn_workspace, "XDTOPackage.Sample"),
            )
            .unwrap();

        assert!(warned.ok, "{warned:?}");
        assert!(
            warned
                .warnings
                .iter()
                .any(|warning| warning.contains("support_guard_warning")
                    && warning.contains("main + XDTOPackage.Sample")),
            "{warned:?}"
        );
        assert_eq!(
            warned.data.as_ref().unwrap()["metadataPath"],
            "XDTOPackage.Sample"
        );
        assert_xdto_public_fields_are_logical(&warned, &warn_workspace);

        std::fs::remove_dir_all(deny_root).unwrap();
        std::fs::remove_dir_all(warn_root).unwrap();
    }

    #[test]
    fn xdto_guards_project_format_warn_and_block_to_the_logical_target() {
        for (label, format_version, expected_code) in [
            ("invalid", "2.20.0", "formatVersionInvalid"),
            ("older", "2.19", "formatMigrationAvailable"),
            ("newer", "2.21", "platformVersionUnsupported"),
        ] {
            let (warn_root, warn_workspace) = xdto_public_guard_workspace(
                &format!("unica-xdto-format-warn-{label}"),
                format_version,
                None,
            );
            let read = UnicaApplication::new()
                .call_tool(
                    "unica.xdto.info",
                    &Map::from_iter([
                        (
                            "cwd".to_string(),
                            Value::String(warn_workspace.display().to_string()),
                        ),
                        ("sourceSet".to_string(), json!("main")),
                        ("metadataPath".to_string(), json!("ПакетXDTO.Sample")),
                    ]),
                )
                .unwrap();

            assert!(read.ok, "{label}: {read:?}");
            assert!(
                read.warnings.iter().any(|warning| {
                    warning.contains("format_guard_warning")
                        && warning.contains("main + XDTOPackage.Sample")
                }),
                "{label}: {read:?}"
            );
            let read_diagnostic = &read.diagnostics.as_ref().unwrap()["formatCompatibility"];
            assert_eq!(read_diagnostic["code"], expected_code, "{label}");
            assert_eq!(read_diagnostic["sourceSet"], "main", "{label}");
            assert_eq!(
                read_diagnostic["metadataPath"], "XDTOPackage.Sample",
                "{label}"
            );
            assert!(read_diagnostic.get("root").is_none(), "{label}: {read:?}");
            assert_xdto_public_fields_are_logical(&read, &warn_workspace);

            let (block_root, block_workspace) = xdto_public_guard_workspace(
                &format!("unica-xdto-format-block-{label}"),
                format_version,
                None,
            );
            let blocked = UnicaApplication::new()
                .call_tool(
                    "unica.xdto.edit",
                    &xdto_public_edit_args(&block_workspace, "XDTOPackage.Sample"),
                )
                .unwrap();

            assert!(!blocked.ok, "{label}: {blocked:?}");
            assert_eq!(
                blocked.artifacts,
                vec!["main + XDTOPackage.Sample".to_string()],
                "{label}"
            );
            let blocked_diagnostic = &blocked.diagnostics.as_ref().unwrap()["formatCompatibility"];
            assert_eq!(blocked_diagnostic["code"], expected_code, "{label}");
            assert_eq!(blocked_diagnostic["sourceSet"], "main", "{label}");
            assert_eq!(
                blocked_diagnostic["metadataPath"], "XDTOPackage.Sample",
                "{label}"
            );
            assert!(
                blocked_diagnostic.get("root").is_none(),
                "{label}: {blocked:?}"
            );
            assert!(blocked.data.is_none(), "{label}: {blocked:?}");
            assert_xdto_public_fields_are_logical(&blocked, &block_workspace);

            std::fs::remove_dir_all(warn_root).unwrap();
            std::fs::remove_dir_all(block_root).unwrap();
        }
    }

    #[test]
    fn xdto_guards_sanitize_format_and_support_evaluation_errors() {
        let workspace = PathBuf::from("/private/provider/workspace");
        let args = xdto_public_edit_args(&workspace, "ПакетXDTO.Sample");
        for (guard, expected_code) in [
            (FailingXdtoGuard::Format, "format_guard_failed"),
            (FailingXdtoGuard::Support, "support_guard_failed"),
        ] {
            let error = UnicaApplication::with_ports(Arc::new(FailingXdtoGuardPorts { guard }))
                .call_tool("unica.xdto.edit", &args)
                .expect_err("guard evaluation failure must remain an application error");

            assert!(error.starts_with(expected_code), "{guard:?}: {error}");
            assert!(
                error.contains("main + XDTOPackage.Sample"),
                "{guard:?}: {error}"
            );
            assert!(!error.contains("/private/provider"), "{guard:?}: {error}");
            assert!(!error.contains("Package.bin"), "{guard:?}: {error}");
        }
    }

    #[test]
    fn xdto_format_guard_preserves_known_resolver_codes_without_physical_handles() {
        let (root, workspace) =
            xdto_public_guard_workspace("unica-xdto-resolver-errors", "2.20", None);
        std::fs::create_dir_all(workspace.join("external")).unwrap();
        std::fs::write(
            workspace.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n  - name: external\n    type: EXTERNAL_DATA_PROCESSORS\n    path: external\n",
        )
        .unwrap();

        let cases = [
            (
                "source_set_unknown",
                "missing + XDTOPackage.Sample",
                UnicaApplication::new()
                    .call_tool(
                        "unica.xdto.info",
                        &xdto_public_info_args(&workspace, "missing", "XDTOPackage.Sample"),
                    )
                    .expect_err("unknown source set must remain an application error"),
            ),
            (
                "target_not_found",
                "main + XDTOPackage.Missing",
                UnicaApplication::new()
                    .call_tool(
                        "unica.xdto.info",
                        &xdto_public_info_args(&workspace, "main", "XDTOPackage.Missing"),
                    )
                    .expect_err("missing logical target must remain an application error"),
            ),
            (
                "not_an_xdto_package",
                "external + XDTOPackage.Sample",
                UnicaApplication::new()
                    .call_tool(
                        "unica.xdto.info",
                        &xdto_public_info_args(&workspace, "external", "XDTOPackage.Sample"),
                    )
                    .expect_err("unsupported source format must remain an application error"),
            ),
        ];

        std::fs::remove_file(workspace.join("src/XDTOPackages/Sample/Ext/Package.bin")).unwrap();
        let missing_resource = UnicaApplication::new()
            .call_tool(
                "unica.xdto.info",
                &xdto_public_info_args(&workspace, "main", "XDTOPackage.Sample"),
            )
            .expect_err("missing Package.bin must remain an application error");
        let expected_codes = [
            "source_set_unknown",
            "target_not_found",
            "not_an_xdto_package",
            "package_resource_missing",
        ];
        let actual_codes = cases
            .iter()
            .map(|(_, _, error)| {
                error
                    .split_once(':')
                    .map_or(error.as_str(), |(code, _)| code)
            })
            .chain(std::iter::once(
                missing_resource
                    .split_once(':')
                    .map_or(missing_resource.as_str(), |(code, _)| code),
            ))
            .collect::<Vec<_>>();
        assert_eq!(actual_codes, expected_codes);

        for (expected_code, expected_target, error) in &cases {
            assert_xdto_public_error_is_logical(error, expected_code, expected_target, &workspace);
        }
        assert_xdto_public_error_is_logical(
            &missing_resource,
            "package_resource_missing",
            "main + XDTOPackage.Sample",
            &workspace,
        );

        std::fs::remove_dir_all(root).unwrap();
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
        std::fs::write(
            src.join("CommonModules/Sample.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><CommonModule><Properties><Name>Sample</Name></Properties></CommonModule></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(
            &module,
            "Procedure Run()\n    Message(\"ok\");\nEndProcedure\n",
        )
        .unwrap();
        let app = UnicaApplication::new();
        let mut args = json!({
            "cwd": workspace,
            "sourceSet": "main",
            "metadataPath": "CommonModule.Sample.Module",
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
            preview.data.as_ref().unwrap()["metadataPath"],
            "CommonModule.Sample.Module"
        );
        assert_eq!(preview.data.as_ref().unwrap()["targetKind"], "module");
        assert!(preview.data.as_ref().unwrap().get("path").is_none());
        assert!(preview.data.as_ref().unwrap()["affectedTarget"]
            .get("path")
            .is_none());
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

        let empty_module = src.join("CommonModules/Empty/Ext/Module.bsl");
        std::fs::create_dir_all(empty_module.parent().unwrap()).unwrap();
        std::fs::write(
            src.join("CommonModules/Empty.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><CommonModule><Properties><Name>Empty</Name></Properties></CommonModule></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(&empty_module, b"\xef\xbb\xbf").unwrap();
        // A module holding no method yet takes a selector-less insert: the end
        // of the module is the one place it already has.
        let mut first_body = json!({
            "cwd": workspace,
            "sourceSet": "main",
            "metadataPath": "CommonModule.Empty.Module",
            "operation": "insert",
            "content": "Procedure Run()\nEndProcedure"
        })
        .as_object()
        .unwrap()
        .clone();
        let first_preview = app.call_tool("unica.code.patch", &first_body).unwrap();
        assert!(first_preview.ok, "{:?}", first_preview.errors);
        assert!(first_preview.cache.events.is_empty());
        assert_eq!(
            first_preview.data.as_ref().unwrap()["validation"]["status"],
            "passed"
        );
        assert_eq!(std::fs::read(&empty_module).unwrap(), b"\xef\xbb\xbf");

        first_body.insert("dryRun".to_string(), json!(false));
        let written = app.call_tool("unica.code.patch", &first_body).unwrap();
        assert!(written.ok, "{:?}", written.errors);
        assert_eq!(written.cache.events, vec!["ModuleChanged"]);
        assert_eq!(
            std::fs::read(&empty_module).unwrap(),
            b"\xef\xbb\xbfProcedure Run()\nEndProcedure\n"
        );

        // Unlike a dedicated initialize operation, the repeat stays a proven
        // no-op instead of failing after the write already landed.
        let repeat = app.call_tool("unica.code.patch", &first_body).unwrap();
        assert!(repeat.ok, "{:?}", repeat.errors);
        assert!(repeat.cache.events.is_empty());
        assert_eq!(
            std::fs::read(&empty_module).unwrap(),
            b"\xef\xbb\xbfProcedure Run()\nEndProcedure\n"
        );

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn form_edit_remove_returns_typed_data() {
        let root = test_workspace_root("unica-form-edit-remove-typed-data");
        let workspace = root.join("workspace");
        let form_path = workspace.join("Form.xml");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(
            &form_path,
            r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20">
	<AutoCommandBar name="FormCommandBar" id="-1"/>
	<ChildItems>
		<Group name="First" id="1">
			<ChildItems>
				<InputField name="FirstInput" id="2">
					<ContextMenu name="FirstInputContextMenu" id="3"/>
				</InputField>
			</ChildItems>
		</Group>
		<InputField name="Second" id="4">
			<ContextMenu name="SecondContextMenu" id="5"/>
			<ExtendedTooltip name="SecondExtendedTooltip" id="6"/>
		</InputField>
	</ChildItems>
	<Attributes/>
	<Commands/>
</Form>
"#,
        )
        .unwrap();
        let original = std::fs::read(&form_path).unwrap();
        let app = UnicaApplication::new();
        let mut args = json!({
            "cwd": workspace,
            "FormPath": form_path,
            "definition": {
                "removeElements": [{"name": "Second"}, {"name": "First"}]
            }
        })
        .as_object()
        .unwrap()
        .clone();
        let expected_data = json!({
            "changed": true,
            "removed": [
                {"name": "Second", "kind": "InputField", "reason": "requested"},
                {"name": "SecondContextMenu", "kind": "ContextMenu", "reason": "contained"},
                {"name": "SecondExtendedTooltip", "kind": "ExtendedTooltip", "reason": "contained"},
                {"name": "First", "kind": "Group", "reason": "requested"},
                {"name": "FirstInput", "kind": "InputField", "reason": "contained"},
                {"name": "FirstInputContextMenu", "kind": "ContextMenu", "reason": "contained"}
            ],
            "addedElements": [],
            "addedAttributes": [],
            "addedCommands": [],
            "addedEvents": [],
            "validation": "passed"
        });

        let preview = app.call_tool("unica.form.edit", &args).unwrap();
        assert!(preview.ok, "{:?}", preview.errors);
        assert_eq!(preview.data, Some(expected_data.clone()));
        assert!(preview.stdout.is_none(), "{preview:?}");
        assert!(preview.cache.events.is_empty());
        assert_eq!(std::fs::read(&form_path).unwrap(), original);

        args.insert("dryRun".to_string(), json!(false));
        let applied = app.call_tool("unica.form.edit", &args).unwrap();
        assert!(applied.ok, "{:?}", applied.errors);
        assert_eq!(applied.data, Some(expected_data));
        assert_eq!(applied.cache.events, vec!["FormChanged"]);
        assert!(applied.stdout.is_none(), "{applied:?}");

        let validation_args = json!({
            "cwd": workspace,
            "FormPath": form_path
        })
        .as_object()
        .unwrap()
        .clone();
        let validation = app
            .call_tool("unica.form.validate", &validation_args)
            .unwrap();
        assert!(validation.ok, "{:?}", validation.errors);
        assert!(validation.cache.events.is_empty());

        let non_removal_form_path = workspace.join("NonRemoval.xml");
        std::fs::write(
            &non_removal_form_path,
            r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20">
	<AutoCommandBar name="FormCommandBar" id="-1"/>
	<ChildItems/>
	<Attributes/>
	<Commands/>
</Form>
"#,
        )
        .unwrap();
        let non_removal_args = json!({
            "cwd": workspace,
            "dryRun": false,
            "FormPath": non_removal_form_path,
            "definition": {"elements": [{"input": "Added"}]}
        })
        .as_object()
        .unwrap()
        .clone();
        let non_removal = app.call_tool("unica.form.edit", &non_removal_args).unwrap();
        assert!(non_removal.ok, "{:?}", non_removal.errors);
        assert_eq!(
            non_removal.data,
            Some(json!({
                "changed": true,
                "removed": [],
                // The addition now shows up as data, not as a printed line.
                "addedElements": [{
                    "kind": "InputField",
                    "name": "Added",
                    "path": null,
                    "representation": null,
                    "autoInsertNewRow": null
                }],
                "addedAttributes": [],
                "addedCommands": [],
                "addedEvents": [],
                "validation": "passed"
            }))
        );
        assert_eq!(non_removal.cache.events, vec!["FormChanged"]);

        let no_op_form_path = workspace.join("NoOp.xml");
        let no_op_original = r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20">
	<AutoCommandBar name="FormCommandBar" id="-1"/>
	<ChildItems/>
	<Attributes/>
	<Commands/>
</Form>
"#;
        std::fs::write(&no_op_form_path, no_op_original).unwrap();
        let no_op_args = json!({
            "cwd": workspace,
            "dryRun": false,
            "FormPath": no_op_form_path,
            "definition": {}
        })
        .as_object()
        .unwrap()
        .clone();
        let no_op = app.call_tool("unica.form.edit", &no_op_args).unwrap();
        assert!(no_op.ok, "{:?}", no_op.errors);
        assert_eq!(
            no_op.data,
            Some(json!({
                "changed": false,
                "removed": [],
                "addedElements": [],
                "addedAttributes": [],
                "addedCommands": [],
                "addedEvents": [],
                "validation": "passed"
            }))
        );
        assert!(no_op.cache.events.is_empty());
        assert_eq!(
            std::fs::read(&no_op_form_path).unwrap(),
            no_op_original.as_bytes()
        );

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn form_edit_preview_rejects_an_invalid_projected_form_at_the_public_boundary() {
        let root = test_workspace_root("unica-form-edit-project-validation");
        let workspace = root.join("workspace");
        let form_path = workspace.join("Form.xml");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(
            &form_path,
            r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20">
	<AutoCommandBar name="FormCommandBar" id="-1"/>
	<ChildItems>
		<InputField name="RemoveMe" id="1">
			<ContextMenu name="RemoveMeContextMenu" id="2"/>
			<ExtendedTooltip name="RemoveMeExtendedTooltip" id="3"/>
		</InputField>
		<InputField name="AlreadyInvalid" id="4"/>
	</ChildItems>
	<Attributes/>
	<Commands/>
</Form>
"#,
        )
        .unwrap();
        let original = std::fs::read(&form_path).unwrap();
        let args = json!({
            "cwd": workspace,
            "FormPath": form_path,
            "definition": {"removeElements": [{"name": "RemoveMe"}]}
        })
        .as_object()
        .unwrap()
        .clone();

        let result = UnicaApplication::new()
            .call_tool("unica.form.edit", &args)
            .unwrap();

        assert!(!result.ok, "{result:?}");
        assert!(result.data.is_none(), "{result:?}");
        assert!(result.cache.events.is_empty(), "{result:?}");
        assert!(
            result.errors.iter().any(
                |error| error.contains("AlreadyInvalid") && error.contains("missing companion")
            ),
            "{result:?}"
        );
        assert_eq!(std::fs::read(&form_path).unwrap(), original);

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn form_edit_remove_json_path_uses_the_typed_public_contract() {
        let root = test_workspace_root("unica-form-edit-remove-json-path");
        let workspace = root.join("workspace");
        let form_path = workspace.join("Form.xml");
        let definition_path = workspace.join("remove.json");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(
            &form_path,
            r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20">
	<AutoCommandBar name="FormCommandBar" id="-1"/>
	<ChildItems>
		<InputField name="Target" id="1">
			<ContextMenu name="TargetContextMenu" id="2"/>
			<ExtendedTooltip name="TargetExtendedTooltip" id="3"/>
		</InputField>
	</ChildItems>
	<Attributes/>
	<Commands/>
</Form>
"#,
        )
        .unwrap();
        std::fs::write(
            &definition_path,
            r#"{"removeElements":[{"name":"Target"}]}"#,
        )
        .unwrap();
        let original_form = std::fs::read(&form_path).unwrap();
        let original_definition = std::fs::read(&definition_path).unwrap();
        let mut args = json!({
            "cwd": workspace,
            "FormPath": form_path,
            "JsonPath": definition_path
        })
        .as_object()
        .unwrap()
        .clone();
        let expected = json!({
            "changed": true,
            "removed": [
                {"name": "Target", "kind": "InputField", "reason": "requested"},
                {"name": "TargetContextMenu", "kind": "ContextMenu", "reason": "contained"},
                {"name": "TargetExtendedTooltip", "kind": "ExtendedTooltip", "reason": "contained"}
            ],
            "addedElements": [],
            "addedAttributes": [],
            "addedCommands": [],
            "addedEvents": [],
            "validation": "passed"
        });
        let app = UnicaApplication::new();

        let preview = app.call_tool("unica.form.edit", &args).unwrap();
        assert!(preview.ok, "{preview:?}");
        assert_eq!(preview.data, Some(expected.clone()));
        assert!(preview.cache.events.is_empty(), "{preview:?}");
        assert_eq!(std::fs::read(&form_path).unwrap(), original_form);

        args.insert("dryRun".to_string(), json!(false));
        let apply = app.call_tool("unica.form.edit", &args).unwrap();
        assert!(apply.ok, "{apply:?}");
        assert_eq!(apply.data, Some(expected));
        assert_eq!(apply.cache.events, vec!["FormChanged"]);
        assert!(!std::fs::read_to_string(&form_path)
            .unwrap()
            .contains("name=\"Target\""));
        assert_eq!(
            std::fs::read(&definition_path).unwrap(),
            original_definition
        );

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
            "sourceSet": "main",
            "metadataPath": "Catalog.Items.ObjectModule",
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
    fn legacy_dry_run_explicit_handler_event_reaches_preview_cache_unchanged() {
        struct ExplicitPreviewEventPorts;

        impl ports::ApplicationPorts for ExplicitPreviewEventPorts {
            fn discover_workspace(
                &self,
                requested_cwd: Option<PathBuf>,
            ) -> Result<WorkspaceContext, String> {
                let cwd = requested_cwd.unwrap_or_default();
                Ok(WorkspaceContext {
                    cwd: cwd.clone(),
                    workspace_root: cwd.clone(),
                    cache_root: cwd.join(".build/unica"),
                    workspace_epoch: 1,
                })
            }

            fn validate_tool_context(
                &self,
                _spec: ToolSpec,
                _args: &Map<String, Value>,
                _mode: InvocationMode,
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
                _mode: InvocationMode,
                _cancellation: &CancellationToken,
            ) -> Result<ports::HandlerOutcome, String> {
                Ok(ports::HandlerOutcome::with_data_and_events(
                    AdapterOutcome::ok("legacy preview with an explicit event"),
                    json!({"preview": true}),
                    vec![DomainEvent::new(
                        DomainEventKind::ModuleChanged,
                        "src/CommonModules/Preview/Ext/Module.bsl",
                    )],
                ))
            }

            fn cache_report(
                &self,
                context: &WorkspaceContext,
                events: &[DomainEvent],
                mode: InvocationMode,
                _cache_access: CacheAccess,
            ) -> Result<CacheReport, String> {
                Ok(CacheReport {
                    mode: if mode.is_preview() {
                        "dry-run"
                    } else {
                        "applied"
                    }
                    .to_string(),
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
                    publication_warnings: Vec::new(),
                })
            }

            fn notify_invalidation(&self, _context: &WorkspaceContext, _events: &[DomainEvent]) {}
        }

        let result = UnicaApplication::with_ports(Arc::new(ExplicitPreviewEventPorts))
            .call_tool("unica.build.load", &Map::new())
            .unwrap();

        assert!(result.ok);
        assert_eq!(result.cache.mode, "dry-run");
        assert_eq!(result.cache.events, ["ModuleChanged"]);
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
            execution: ToolExecution::Mutation,
            result_contract: ResultContract::Typed,
            cache_access: cache_access_for("cf-edit", Some(DomainEventKind::ConfigXmlChanged)),
            handler: ToolHandler::NativeOperation {
                operation: "cf-edit",
                event: Some(DomainEventKind::ConfigXmlChanged),
            },
        };

        let args = Map::new();
        assert!(!should_emit_events(spec, &args, false, &outcome, None));

        outcome
            .changes
            .push("updated Configuration.xml".to_string());
        assert!(should_emit_events(spec, &args, false, &outcome, None));
        assert!(should_emit_events(
            spec,
            &args,
            true,
            &AdapterOutcome::ok("generic dry run"),
            None,
        ));

        let code_patch_spec = ToolSpec {
            name: "unica.code.patch",
            description: "test",
            execution: ToolExecution::Mutation,
            result_contract: ResultContract::Typed,
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
            &AdapterOutcome::ok("code patch preview"),
            None,
        ));

        let form_edit_spec = ToolSpec {
            name: "unica.form.edit",
            description: "test",
            execution: ToolExecution::Mutation,
            result_contract: ResultContract::Typed,
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
            &AdapterOutcome::ok("semantic dry run no-op"),
            None,
        ));

        let mut planned = AdapterOutcome::ok("dry run planned change");
        planned.changes.push("would update Form.xml".to_string());
        assert!(!should_emit_events(
            form_edit_spec,
            &semantic_args,
            true,
            &planned,
            None,
        ));

        let mut rejected = AdapterOutcome::ok("dry run rejected");
        rejected.ok = false;
        rejected.changes.push("would update Form.xml".to_string());
        assert!(!should_emit_events(
            form_edit_spec,
            &semantic_args,
            true,
            &rejected,
            None,
        ));
    }

    #[test]
    fn xdto_event_selector_uses_typed_plan_state_without_presentation_changes() {
        let spec = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.xdto.edit")
            .unwrap();
        let args = Map::new();
        let changed = json!({"noOp": false});
        let no_op = json!({"noOp": true});
        let successful = AdapterOutcome::ok("presentation changes intentionally omitted");

        assert!(should_emit_events(
            spec,
            &args,
            false,
            &successful,
            Some(&changed),
        ));
        assert!(!should_emit_events(
            spec,
            &args,
            false,
            &successful,
            Some(&no_op),
        ));

        let mut failed = AdapterOutcome::ok("typed plan exists but the operation failed");
        failed.ok = false;
        assert!(!should_emit_events(
            spec,
            &args,
            false,
            &failed,
            Some(&changed),
        ));
    }

    #[test]
    fn role_edit_event_selector_uses_typed_changed_state_for_preview_apply_and_noop() {
        let spec = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.role.edit")
            .unwrap();
        let args = Map::new();
        let changed = json!({"changed": true});
        let no_op = json!({"changed": false});
        let successful = AdapterOutcome::ok("typed role edit");

        for dry_run in [true, false] {
            assert!(should_emit_events(
                spec,
                &args,
                dry_run,
                &successful,
                Some(&changed),
            ));
            assert!(!should_emit_events(
                spec,
                &args,
                dry_run,
                &successful,
                Some(&no_op),
            ));
        }
        let impact = crate::domain::cache::CacheImpact::from_events(&[DomainEvent::new(
            DomainEventKind::RoleChanged,
            "main + Role.Demo",
        )]);
        assert!(impact.invalidated.contains("rights_graph"));
        assert!(impact.eager_refresh.contains("rights_graph"));
    }

    fn role_edit_application_workspace(
        label: &str,
        descriptor_version: &str,
        support_locked: bool,
    ) -> (PathBuf, PathBuf, Map<String, Value>) {
        let root = test_workspace_root(label);
        let workspace = root.join("workspace");
        let src = workspace.join("src");
        let rights = src.join("Roles/Demo/Ext/Rights.xml");
        std::fs::create_dir_all(rights.parent().unwrap()).unwrap();
        std::fs::write(
            workspace.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(
            src.join("Configuration.xml"),
            format!(
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="{descriptor_version}"><Configuration uuid="aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"><Properties><Name>Main</Name></Properties><ChildObjects><Role>Demo</Role></ChildObjects></Configuration></MetaDataObject>"#
            ),
        )
        .unwrap();
        std::fs::write(
            src.join("Roles/Demo.xml"),
            format!(
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="{descriptor_version}"><Role uuid="bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"><Properties><Name>Demo</Name></Properties></Role></MetaDataObject>"#
            ),
        )
        .unwrap();
        std::fs::write(
            &rights,
            concat!(
                "<Rights xmlns=\"http://v8.1c.ru/8.2/roles\" ",
                "xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" ",
                "xsi:type=\"Rights\" version=\"2.20\">",
                "<setForNewObjects>false</setForNewObjects>",
                "<setForAttributesByDefault>true</setForAttributesByDefault>",
                "<independentRightsOfChildObjects>false</independentRightsOfChildObjects>",
                "<object><name>Catalog.Demo</name><right><name>Delete</name>",
                "<value>true</value></right></object></Rights>"
            ),
        )
        .unwrap();
        if support_locked {
            std::fs::create_dir_all(src.join("Ext")).unwrap();
            std::fs::write(
                src.join("Ext/ParentConfigurations.bin"),
                support_test_parent_configurations_bin(
                    "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
                    "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb",
                    "cccccccc-cccc-cccc-cccc-cccccccccccc",
                ),
            )
            .unwrap();
        }
        let args = json!({
            "sourceSet": "main",
            "metadataPath": "Role.Demo",
            "operations": [{
                "op": "setRight",
                "objectName": "Catalog.Demo",
                "right": "Delete",
                "value": false
            }]
        })
        .as_object()
        .unwrap()
        .clone();
        (root, workspace, args)
    }

    fn assert_typed_role_edit_failure(
        result: &OperationResult,
        workspace: &std::path::Path,
        expected_metadata_path: &str,
        expected_code: &str,
    ) {
        assert!(!result.ok, "{result:?}");
        let value = serde_json::to_value(result).unwrap();
        assert_eq!(
            value
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<std::collections::BTreeSet<_>>(),
            std::collections::BTreeSet::from([
                "artifacts",
                "cache",
                "changes",
                "data",
                "errors",
                "ok",
                "summary",
                "warnings",
            ])
        );
        assert_eq!(value["artifacts"], json!([]));
        assert_eq!(value["cache"]["root"], "");
        assert_eq!(value["data"]["metadataPath"], expected_metadata_path);
        assert_eq!(value["data"]["changed"], false);
        assert_eq!(value["data"]["effects"], json!([]));
        assert_eq!(value["data"]["validation"], json!({"status":"failed"}));
        assert_eq!(value["data"]["diagnostics"][0]["code"], expected_code);
        assert!(jsonschema::validator_for(&role_edit_output_schema())
            .unwrap()
            .is_valid(&value));
        let encoded = serde_json::to_string(&value).unwrap();
        assert!(
            !encoded.contains(&workspace.display().to_string()),
            "{encoded}"
        );
        assert!(!encoded.contains("Rights.xml"), "{encoded}");
    }

    #[test]
    fn role_edit_application_projects_2_19_and_2_21_format_blocks_to_typed_data() {
        for (version, code) in [
            ("2.19", "formatMigrationAvailable"),
            ("2.21", "platformVersionUnsupported"),
        ] {
            let (root, workspace, args) = role_edit_application_workspace(
                &format!("unica-role-edit-format-{version}"),
                version,
                false,
            );
            let before = std::fs::read(workspace.join("src/Roles/Demo/Ext/Rights.xml")).unwrap();
            let result =
                call_public_tool_from_workspace(&workspace, "unica.role.edit", &args).unwrap();
            assert_typed_role_edit_failure(&result, &workspace, "Role.Demo", code);
            assert_eq!(
                std::fs::read(workspace.join("src/Roles/Demo/Ext/Rights.xml")).unwrap(),
                before
            );
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn role_edit_application_uses_transactionally_recorded_cache_and_noop_does_not_republish() {
        let (root, workspace, mut args) =
            role_edit_application_workspace("unica-role-edit-recorded-cache", "2.20", false);
        args.insert("dryRun".to_string(), json!(false));
        let applied =
            call_public_tool_from_workspace(&workspace, "unica.role.edit", &args).unwrap();
        assert!(applied.ok, "{applied:?}");
        assert_eq!(applied.cache.root, "");
        assert_eq!(applied.cache.mode, "applied");
        assert_eq!(applied.cache.events, ["RoleChanged"]);
        assert!(applied
            .cache
            .invalidated
            .contains(&"rights_graph".to_string()));
        assert!(applied
            .cache
            .refreshed
            .contains(&"rights_graph".to_string()));
        assert_eq!(applied.data.as_ref().unwrap()["changed"], true);
        let state_path = workspace.join(".build/unica/state.json");
        let state_after_apply = std::fs::read(&state_path).unwrap();

        let repeated =
            call_public_tool_from_workspace(&workspace, "unica.role.edit", &args).unwrap();
        assert!(repeated.ok, "{repeated:?}");
        assert_eq!(repeated.cache.root, "");
        assert!(repeated.cache.events.is_empty());
        assert_eq!(repeated.data.as_ref().unwrap()["changed"], false);
        assert_eq!(std::fs::read(&state_path).unwrap(), state_after_apply);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn role_edit_application_projects_cache_read_failure_without_physical_paths() {
        let (root, workspace, args) =
            role_edit_application_workspace("unica-role-edit-cache-failure", "2.20", false);
        let rights = workspace.join("src/Roles/Demo/Ext/Rights.xml");
        let before = std::fs::read(&rights).unwrap();
        std::fs::create_dir_all(workspace.join(".build/unica/state.json")).unwrap();

        let result = call_public_tool_from_workspace(&workspace, "unica.role.edit", &args).unwrap();

        assert_typed_role_edit_failure(&result, &workspace, "Role.Demo", "cache_unavailable");
        assert_eq!(std::fs::read(&rights).unwrap(), before);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn role_edit_application_reports_one_stable_support_warning() {
        let (root, workspace, mut args) =
            role_edit_application_workspace("unica-role-edit-support-warn", "2.20", true);
        std::fs::write(
            workspace.join(".v8-project.json"),
            r#"{"editingAllowedCheck":"warn"}"#,
        )
        .unwrap();
        args.insert("dryRun".to_string(), json!(false));

        let result = call_public_tool_from_workspace(&workspace, "unica.role.edit", &args).unwrap();

        assert!(result.ok, "{result:?}");
        assert_eq!(
            result
                .warnings
                .iter()
                .filter(|warning| warning.starts_with("support_guard_warning:"))
                .count(),
            1,
            "{result:?}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn role_edit_application_projects_support_block_to_typed_data() {
        let (root, workspace, args) =
            role_edit_application_workspace("unica-role-edit-support", "2.20", true);
        let rights = workspace.join("src/Roles/Demo/Ext/Rights.xml");
        let before = std::fs::read(&rights).unwrap();
        let result = call_public_tool_from_workspace(&workspace, "unica.role.edit", &args).unwrap();
        assert_typed_role_edit_failure(&result, &workspace, "Role.Demo", "support_locked");
        assert_eq!(std::fs::read(&rights).unwrap(), before);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn role_edit_application_projects_missing_logical_target_to_typed_data() {
        let (root, workspace, mut args) =
            role_edit_application_workspace("unica-role-edit-missing", "2.20", false);
        args.insert("metadataPath".to_string(), json!("Role.Missing"));
        let result = call_public_tool_from_workspace(&workspace, "unica.role.edit", &args).unwrap();
        assert_typed_role_edit_failure(&result, &workspace, "Role.Missing", "target_not_found");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn role_edit_application_projects_format_and_support_guard_errors_to_typed_data() {
        let args = json!({
            "sourceSet": "main",
            "metadataPath": "Role.Demo",
            "operations": [{
                "op": "setRight",
                "objectName": "Catalog.Demo",
                "right": "Delete",
                "value": false
            }]
        })
        .as_object()
        .unwrap()
        .clone();
        let hidden = PathBuf::from("/private/provider/workspace");
        for (guard, code) in [
            (FailingXdtoGuard::Format, "format_guard_failed"),
            (FailingXdtoGuard::Support, "support_guard_failed"),
        ] {
            let result = UnicaApplication::with_ports(Arc::new(FailingXdtoGuardPorts { guard }))
                .call_tool("unica.role.edit", &args)
                .unwrap();
            assert_typed_role_edit_failure(&result, &hidden, "Role.Demo", code);
        }
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
        // `unica.cf.info` left the parity stand when it started answering with
        // typed data: there is no prose left to compare (ADR-0023).
        const PARITY_COVERED_TOOLS: &[&str] = &[
            "unica.cf.validate",
            "unica.cfe.validate",
            "unica.form.compile",
            "unica.form.validate",
            "unica.interface.validate",
            "unica.subsystem.compile",
            "unica.subsystem.validate",
            "unica.dcs.compile",
            "unica.dcs.validate",
            "unica.mxl.compile",
            "unica.mxl.decompile",
            "unica.mxl.validate",
            "unica.role.compile",
            "unica.role.validate",
        ];
        const REPO_OWNED_NATIVE_TOOLS: &[&str] = &["unica.support.edit"];
        // A tool that answers with typed data has no prose left for the parity
        // stand to compare, so it is covered by its own crate tests instead
        // (ADR-0023).
        const TYPED_RESULT_TOOLS: &[&str] = &[
            "unica.cf.info",
            "unica.role.info",
            "unica.role.edit",
            "unica.subsystem.info",
            "unica.mxl.info",
            "unica.cfe.diff",
            "unica.meta.add",
            "unica.meta.edit",
            "unica.template.add",
            "unica.template.remove",
            "unica.help.add",
            "unica.form.remove",
            "unica.interface.edit",
            "unica.meta.remove",
            "unica.cfe.init",
            "unica.cf.edit",
            "unica.cf.init",
            "unica.cfe.borrow",
            "unica.cfe.patch_method",
            "unica.subsystem.edit",
            "unica.form.add",
            "unica.dcs.edit",
            "unica.form.edit",
            "unica.form.info",
            "unica.meta.info",
            "unica.dcs.info",
        ];

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
            match tool.handler {
                ToolHandler::NativeOperation { operation, .. } => {
                    assert!(
                        PARITY_COVERED_TOOLS.contains(&tool.name)
                            || REPO_OWNED_NATIVE_TOOLS.contains(&tool.name)
                            || TYPED_RESULT_TOOLS.contains(&tool.name),
                        "{} routes to native operation {} without a parity fixture or repo-owned native contract exception",
                        tool.name,
                        operation
                    );
                }
                ToolHandler::Metadata { .. } => assert!(
                    matches!(
                        tool.name,
                        "unica.meta.info"
                            | "unica.meta.add"
                            | "unica.meta.edit"
                            | "unica.meta.remove"
                    ),
                    "{} unexpectedly routes through the typed Metadata handler",
                    tool.name
                ),
                _ => panic!("{} routes through unexpected handler", tool.name),
            }
        }
    }

    #[test]
    fn meta_info_declares_only_the_local_graphs_it_reads() {
        let tool = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.meta.info")
            .unwrap();

        // Nothing in a metadata read consults the code index any more, so
        // declaring `bsl_index` would report a dependency the tool does not
        // have and make its cache status answer for a provider it never calls.
        assert_eq!(
            tool.cache_access.reads,
            &["workspace_graph", "metadata_graph"]
        );
        assert!(tool.cache_access.writes.is_empty());
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
        let data = result.data.unwrap();
        assert!(data["workspaceRoot"].is_string());
        assert!(data["cacheRoot"].is_string());
        // Discovery either proves the sets or says it could not: an empty list
        // must never stand in for "we did not look".
        assert!(data["sourceSets"].is_array() || data["sourceSets"].is_null());
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
        // ADR-0023: the map is the result, so it rides in `data` instead of
        // being serialized into a JSON string inside the JSON envelope.
        assert!(result.stdout.is_none(), "{:?}", result.stdout);
        let data = result.data.unwrap();
        let source_sets = data["sourceSets"].as_array().unwrap();
        assert_eq!(source_sets.len(), 1);
        assert_eq!(source_sets[0]["kind"], "configuration");
        assert_eq!(source_sets[0]["sourceFormat"], "platform_xml");
        assert_eq!(data["effectiveSourceSet"], "main");
        assert!(data["effectiveSourceRoot"].is_string());
        assert!(data.get("sourceSelectionError").is_none());

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
        let source_sets = result.data.as_ref().unwrap()["sourceSets"]
            .as_array()
            .unwrap()
            .clone();
        assert_eq!(
            source_sets
                .iter()
                .filter(|entry| entry["sourceFormat"] == "platform_xml")
                .count(),
            2
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
        let data = result.data.unwrap();
        let names = data["sourceSets"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["name"].as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["app".to_string(), "tests".to_string()]);
        assert!(data["sourceSelectionError"]
            .as_str()
            .unwrap()
            .contains("sourceDir"));

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
        // ADR-0023: the support state is four typed values plus counts, not a
        // sentence a consumer has to match.
        let support = &result.data.unwrap()["support"];
        assert_eq!(support["state"], "supported");
        assert_eq!(support["editingEnabled"], true);
        assert_eq!(support["objects"]["locked"], 1);
        assert_eq!(support["objects"]["editable"], 1);
        assert_eq!(support["objects"]["removed"], 1);

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
        let data = result.data.as_ref().expect("cf.edit answers with data");
        assert_eq!(data["configUpdated"], serde_json::json!(false), "{data:?}");
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
                        .iter()
                        .map(|change| change.replace('\\', "/"))
                        .any(|change| change == format!("updated {}", path_text(&path))),
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
        let data = overview.data.unwrap();
        let bots = data["childObjects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["kind"] == "Bot")
            .expect("the configuration registers a Bot");
        assert_eq!(bots["count"], 1);

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
        let data = result.data.as_ref().expect("cf.edit answers with data");
        assert_eq!(data["configUpdated"], serde_json::json!(false), "{data:?}");
        let skipped = data["operations"]
            .as_array()
            .expect("operations is a list")
            .iter()
            .find(|item| item["target"] == serde_json::json!("Catalog.Валюты"))
            .expect("the duplicate add is reported");
        assert_eq!(skipped["applied"], serde_json::json!(false), "{skipped:?}");
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
        write_support_test_language(&src);
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
        args.insert("sourceSet".to_string(), Value::String("main".to_string()));
        args.insert(
            "metadataPath".to_string(),
            Value::String("Catalog.Items".to_string()),
        );

        let result = call_public_tool_from_workspace(&workspace, "unica.meta.info", &args).unwrap();

        assert!(
            !result.ok,
            "the legacy support fixture is intentionally incomplete"
        );
        assert_eq!(result.summary, "metadata validation failed");
        // The locked rule is a per-object fact: the configuration is on
        // support, but this object must not be edited directly.
        let data = result.data.as_ref().expect("meta.info answers with data");
        assert_eq!(data["support"], serde_json::json!("locked"), "{data:?}");
        assert!(result.stdout.is_none(), "{result:?}");

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn code_outline_tool_declares_no_cache_access() {
        // ADR-0020: the outline is parsed from the current file, so the envelope
        // must not claim `bsl_index` as an input of this tool — neither as read
        // nor as written.
        let tool = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.code.outline")
            .expect("code-outline tool exists");

        assert!(!tool.execution.is_mutating());
        assert!(tool.cache_access.reads.is_empty());
        assert!(tool.cache_access.writes.is_empty());
        assert!(!tool.description.contains("index"), "{}", tool.description);
    }

    #[test]
    fn support_edit_tool_is_mutating_native_operation() {
        let tool = tools()
            .into_iter()
            .find(|tool| tool.name == "unica.support.edit")
            .expect("support-edit tool exists");

        assert!(tool.execution.is_mutating());
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
    fn reader_schemas_never_publish_dry_run_and_mutations_keep_it() {
        for tool in tools() {
            let schema = input_schema_for_tool(&tool);
            let properties = schema["properties"]
                .as_object()
                .expect("tool input schema properties are an object");
            assert_eq!(
                properties.contains_key("dryRun"),
                tool.execution.is_mutating(),
                "{} publishes the wrong invocation switch",
                tool.name,
            );
            if tool.execution.is_mutating() {
                assert_eq!(
                    properties["dryRun"]["default"], true,
                    "{} publishes the wrong preview default",
                    tool.name,
                );
            }
        }
    }

    #[test]
    fn tool_specs_match_reviewed_result_contracts() {
        let review: Value = serde_json::from_str(include_str!(
            "../../../../spec/architecture/tool-surface-review.json"
        ))
        .expect("tool-surface review is valid JSON");
        let review = review
            .as_object()
            .expect("tool-surface review is a tool-name object");
        let registered = tools();
        assert_eq!(review.len(), registered.len());

        for tool in registered {
            let entry = review
                .get(tool.name)
                .unwrap_or_else(|| panic!("{} has no tool-surface review", tool.name));
            let expected = if entry["scope"] == "in" && entry["result"]["contract"] == "typed" {
                ResultContract::Typed
            } else {
                ResultContract::ExternalStream
            };
            assert_eq!(tool.result_contract, expected, "{}", tool.name);
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
    fn native_operation_descriptors_drive_canonical_required_schema_paths() {
        for tool in tools() {
            let ToolHandler::NativeOperation { operation, .. } = tool.handler else {
                continue;
            };
            let descriptor = operation_descriptors::native_operation_descriptor(operation).unwrap();
            let schema = input_schema_for_tool(&tool);
            let path_groups = operation_descriptors::native_path_alias_groups(operation);
            let required = schema["required"]
                .as_array()
                .expect("schema required is array")
                .iter()
                .map(|value| value.as_str().expect("required item is string"))
                .collect::<Vec<_>>();
            assert_eq!(
                required, descriptor.required_args,
                "{operation} canonical required arguments"
            );
            assert!(
                schema.get("allOf").is_none(),
                "{operation} must not hide required paths in schema composition"
            );
            let properties = schema["properties"].as_object().unwrap();
            for required in &required {
                assert!(
                    properties.contains_key(*required),
                    "{operation} requires unpublished argument {required}"
                );
            }
            for group in path_groups {
                assert!(properties.contains_key(group.canonical));
                for alias in group
                    .aliases
                    .iter()
                    .filter(|alias| **alias != group.canonical)
                {
                    assert!(
                        !properties.contains_key(*alias),
                        "{operation} publishes compatibility path alias {alias}"
                    );
                }
            }
        }
    }

    #[test]
    fn mutating_native_descriptors_declare_write_path_policy() {
        for tool in tools() {
            if !tool.execution.is_mutating() {
                continue;
            }
            let ToolHandler::NativeOperation { operation, .. } = tool.handler else {
                continue;
            };
            let descriptor = operation_descriptors::native_operation_descriptor(operation).unwrap();
            assert!(
                !descriptor.write_path_args.is_empty()
                    || format!("{:?}", descriptor.format_path_policy) == "HandlerResolved",
                "{operation} mutates workspace but has neither declared nor handler-resolved write targets"
            );
        }
    }

    #[test]
    fn mutating_native_support_guard_coverage_is_explicit() {
        use operation_descriptors::{SupportGuardPolicy, SupportGuardRequirement};

        let mut guarded = Vec::new();
        let mut exempt = Vec::new();
        for tool in tools()
            .into_iter()
            .filter(|tool| tool.execution.is_mutating())
        {
            let ToolHandler::NativeOperation { operation, .. } = tool.handler else {
                continue;
            };
            let descriptor = operation_descriptors::native_operation_descriptor(operation).unwrap();
            if operation == "code-patch" {
                assert_eq!(
                    format!("{:?}", descriptor.support_guard),
                    "Some(HandlerResolved { requirement: Editable })",
                    "code.patch must not silently lose its support guard with the public path"
                );
            }
            match descriptor.support_guard {
                Some(policy) => {
                    match policy {
                        SupportGuardPolicy::HandlerResolved { requirement } => {
                            assert!(
                                matches!(operation, "code-patch" | "xdto-edit" | "role-edit"),
                                "{operation} unexpectedly delegates support resolution"
                            );
                            assert_eq!(requirement, SupportGuardRequirement::Editable);
                        }
                        SupportGuardPolicy::PathArgs { names, requirement } => {
                            assert!(!names.is_empty(), "{operation} guard target is empty");
                            assert_eq!(
                                requirement,
                                SupportGuardRequirement::Editable,
                                "{operation} path mutation must require an editable owner"
                            );
                            if operation == "subsystem-compile" {
                                assert_eq!(
                                    names,
                                    &["Parent", "parent", "OutputDir", "outputDir"],
                                    "subsystem compilation must guard Parent first and retain OutputDir as the root fallback"
                                );
                            }
                        }
                        SupportGuardPolicy::ObjectName { requirement } => {
                            assert!(
                                matches!(
                                    operation,
                                    "help-add" | "form-remove" | "template-add" | "template-remove"
                                ),
                                "{operation} unexpectedly uses object-name guard resolution"
                            );
                            assert_eq!(requirement, SupportGuardRequirement::Editable);
                        }
                    }
                    guarded.push(operation);
                }
                None => exempt.push(operation),
            }
        }
        guarded.sort_unstable();
        exempt.sort_unstable();

        assert_eq!(
            guarded,
            [
                "cf-edit",
                "code-patch",
                "dcs-compile",
                "dcs-edit",
                "form-add",
                "form-compile",
                "form-edit",
                "form-remove",
                "help-add",
                "interface-edit",
                "mxl-compile",
                "role-compile",
                "role-edit",
                "subsystem-compile",
                "subsystem-edit",
                "template-add",
                "template-remove",
                "xdto-edit",
            ],
            "guarded platform-XML mutations changed without updating the support contract"
        );
        let expected_exemptions = [
            ("cf-init", "creates a new configuration tree"),
            ("cfe-borrow", "writes only into an extension"),
            ("cfe-init", "creates a new extension tree"),
            ("cfe-patch-method", "writes only into an extension"),
            ("epf-init", "creates a new external processor tree"),
            ("erf-init", "creates a new external report tree"),
            (
                "support-edit",
                "must remain available to change the support lock itself",
            ),
        ];
        assert!(expected_exemptions
            .iter()
            .all(|(_, reason)| !reason.is_empty()));
        assert_eq!(
            exempt,
            expected_exemptions
                .iter()
                .map(|(operation, _)| *operation)
                .collect::<Vec<_>>(),
            "every unguarded native mutation must remain an explicitly justified support-guard exception"
        );
    }

    #[test]
    fn mutating_platform_xml_operations_declare_effective_format_paths() {
        use operation_descriptors::FormatGuardPolicy;

        let expected = [
            ("code-patch", &[][..], "HandlerResolved"),
            ("xdto-edit", &[][..], "HandlerResolved"),
            ("role-edit", &[][..], "HandlerResolved"),
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
            .filter(|tool| tool.execution.is_mutating())
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

        for tool in tools()
            .into_iter()
            .filter(|tool| tool.execution.is_mutating())
        {
            let ToolHandler::NativeOperation { operation, .. } = tool.handler else {
                continue;
            };
            let descriptor = operation_descriptors::native_operation_descriptor(operation).unwrap();
            assert!(
                !descriptor.source_path_args.is_empty()
                    || format!("{:?}", descriptor.format_path_policy) == "HandlerResolved",
                "{operation} must declare path arguments or explicit handler resolution"
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

    fn subsystem_format_guard_workspace(
        prefix: &str,
    ) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
        let root = test_workspace_root(prefix);
        let workspace = root.join("workspace");
        let source = workspace.join("src");
        let child = source.join("Subsystems/Parent/Subsystems/Child.xml");
        std::fs::create_dir_all(child.parent().unwrap()).unwrap();
        std::fs::write(
            workspace.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(
            source.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Test</Name></Properties><ChildObjects><Subsystem>Parent</Subsystem></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(
            source.join("Subsystems/Parent.xml"),
            child_subsystem_stub_xml("Parent", "2.20").replacen(
                "<ChildObjects/>",
                "<ChildObjects><Subsystem>Child</Subsystem></ChildObjects>",
                1,
            ),
        )
        .unwrap();
        std::fs::write(&child, child_subsystem_stub_xml("Child", "2.21")).unwrap();
        let physical_workspace = workspace.canonicalize().unwrap();
        (root, physical_workspace, child)
    }

    fn assert_public_subsystem_format_warning(
        workspace: &std::path::Path,
        subsystem_path: &str,
        child: &std::path::Path,
    ) {
        let args = Map::from_iter([
            (
                "cwd".to_string(),
                Value::String(workspace.canonicalize().unwrap().display().to_string()),
            ),
            (
                "SubsystemPath".to_string(),
                Value::String(subsystem_path.to_string()),
            ),
        ]);

        let result = UnicaApplication::new()
            .call_tool("unica.subsystem.info", &args)
            .unwrap();

        assert!(result.ok, "{result:?}");
        assert!(!result.warnings.is_empty(), "{result:?}");
        let diagnostic = &result.diagnostics.as_ref().unwrap()["formatCompatibility"];
        assert_eq!(diagnostic["actualFormat"], "2.21", "{result:?}");
        assert_eq!(
            normalized_path(&std::path::PathBuf::from(
                diagnostic["root"].as_str().unwrap()
            )),
            normalized_path(child)
        );
    }

    #[test]
    fn public_subsystem_format_guard_covers_registered_descendants_for_a_directory_without_mode() {
        let (root, workspace, child) =
            subsystem_format_guard_workspace("unica-subsystem-format-directory");

        assert_public_subsystem_format_warning(&workspace, "src/Subsystems", &child);

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn public_subsystem_format_guard_covers_registered_descendants_for_a_file_without_mode() {
        let (root, workspace, child) =
            subsystem_format_guard_workspace("unica-subsystem-format-file");

        assert_public_subsystem_format_warning(&workspace, "src/Subsystems/Parent.xml", &child);

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn public_subsystem_info_projects_registered_dependency_errors_as_typed_failures() {
        let root = test_workspace_root("unica-subsystem-format-resolver-error");
        let workspace = root.join("workspace");
        let source = workspace.join("src");
        std::fs::create_dir_all(source.join("Subsystems")).unwrap();
        std::fs::write(
            workspace.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(
            source.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Test</Name></Properties><ChildObjects><Subsystem>Missing</Subsystem></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        let args = Map::from_iter([
            (
                "cwd".to_string(),
                Value::String(workspace.canonicalize().unwrap().display().to_string()),
            ),
            (
                "SubsystemPath".to_string(),
                Value::String("src/Subsystems".to_string()),
            ),
        ]);

        let result = UnicaApplication::new()
            .call_tool("unica.subsystem.info", &args)
            .expect("provider evidence failures stay inside the public tool envelope");

        assert!(!result.ok, "{result:?}");
        assert!(result.data.is_none(), "{result:?}");
        let diagnostics = result
            .diagnostics
            .as_ref()
            .and_then(Value::as_array)
            .expect("typed provider diagnostics");
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic["code"] == "provider_unavailable"),
            "{diagnostics:?}"
        );
        assert!(
            result.errors.iter().any(|error| {
                error.contains("registered subsystem descriptor")
                    && error.contains("Subsystems/Missing.xml")
            }),
            "{result:?}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn public_subsystem_info_does_not_classify_descriptor_text_as_a_control_error() {
        let root = test_workspace_root("unica-subsystem-provider-error-text");
        let workspace = root.join("workspace");
        let source = workspace.join("src");
        std::fs::create_dir_all(source.join("Subsystems")).unwrap();
        std::fs::write(
            workspace.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(
            source.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Test</Name></Properties><ChildObjects><Subsystem>Injected</Subsystem></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(
            source.join("Subsystems/Injected.xml"),
            child_subsystem_stub_xml("Injected", "2.20").replace(
                "<Name>Injected</Name>",
                "<Name>provider deadline exceeded</Name>",
            ),
        )
        .unwrap();
        let args = Map::from_iter([
            (
                "cwd".to_string(),
                Value::String(workspace.canonicalize().unwrap().display().to_string()),
            ),
            (
                "SubsystemPath".to_string(),
                Value::String("src/Subsystems".to_string()),
            ),
        ]);

        let result = UnicaApplication::new()
            .call_tool("unica.subsystem.info", &args)
            .expect("descriptor proof failures stay inside the public tool envelope");

        assert!(!result.ok, "{result:?}");
        assert!(result.data.is_none(), "{result:?}");
        let diagnostics = result
            .diagnostics
            .as_ref()
            .and_then(Value::as_array)
            .expect("typed provider diagnostics");
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic["code"] == "provider_unavailable"),
            "{diagnostics:?}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn public_subsystem_info_rejects_dry_run_before_reading_target() {
        let root = test_workspace_root("unica-subsystem-dry-run-missing-target");
        let workspace = root.join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(workspace.join("v8project.yaml"), "format: DESIGNER\n").unwrap();
        let missing = workspace.join("src/Subsystems/Продажи.xml");
        let args = Map::from_iter([
            (
                "cwd".to_string(),
                Value::String(workspace.canonicalize().unwrap().display().to_string()),
            ),
            (
                "SubsystemPath".to_string(),
                Value::String("src/Subsystems/Продажи.xml".to_string()),
            ),
            ("dryRun".to_string(), Value::Bool(true)),
        ]);

        assert!(!missing.exists());
        let error = UnicaApplication::new()
            .call_tool("unica.subsystem.info", &args)
            .expect_err("reader must reject dryRun before target discovery");

        assert!(
            error.contains("does not accept argument `dryRun`"),
            "{error}"
        );
        assert!(
            !missing.exists(),
            "argument rejection must not create the missing target"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn public_subsystem_validate_rejects_dry_run_before_reading_target() {
        let root = test_workspace_root("unica-subsystem-validate-dry-run-missing-target");
        let workspace = root.join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(workspace.join("v8project.yaml"), "format: DESIGNER\n").unwrap();
        let missing = workspace.join("Subsystems/Продажи.xml");
        let args = Map::from_iter([
            (
                "cwd".to_string(),
                Value::String(workspace.canonicalize().unwrap().display().to_string()),
            ),
            (
                "SubsystemPath".to_string(),
                Value::String("Subsystems/Продажи.xml".to_string()),
            ),
            ("dryRun".to_string(), Value::Bool(true)),
        ]);

        assert!(!missing.exists());
        let error = UnicaApplication::new()
            .call_tool("unica.subsystem.validate", &args)
            .expect_err("reader must reject dryRun before target discovery");

        assert!(
            error.contains("does not accept argument `dryRun`"),
            "{error}"
        );
        assert!(
            !missing.exists(),
            "argument rejection must not create the missing target"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn public_subsystem_validate_missing_target_is_a_normal_typed_failure() {
        let root = test_workspace_root("unica-subsystem-validate-missing-target");
        let workspace = root.join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(workspace.join("v8project.yaml"), "format: DESIGNER\n").unwrap();
        let args = Map::from_iter([
            (
                "cwd".to_string(),
                Value::String(workspace.canonicalize().unwrap().display().to_string()),
            ),
            (
                "SubsystemPath".to_string(),
                Value::String("missing/Subsystem.xml".to_string()),
            ),
        ]);

        let result = UnicaApplication::new()
            .call_tool("unica.subsystem.validate", &args)
            .expect("validation failures stay inside the public tool envelope");

        assert!(!result.ok, "{result:?}");
        assert!(
            result
                .errors
                .iter()
                .any(|error| error.contains("File not found")),
            "{result:?}"
        );
        assert!(result.data.is_none(), "{result:?}");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn public_subsystem_info_cancellation_stops_the_preflight_registered_capture() {
        let (root, workspace, _) =
            subsystem_format_guard_workspace("unica-subsystem-public-preflight-cancel");
        let args = Map::from_iter([
            (
                "cwd".to_string(),
                Value::String(workspace.display().to_string()),
            ),
            (
                "SubsystemPath".to_string(),
                Value::String("src/Subsystems".to_string()),
            ),
        ]);
        let cancellation = CancellationToken::new();
        let hook_cancellation = cancellation.clone();
        let read_after_cancellation = Rc::new(Cell::new(false));
        let hook_read_after_cancellation = Rc::clone(&read_after_cancellation);

        let result = with_secure_tree_test_hook(
            move |phase| match phase {
                SecureTreePhase::AfterRebindEntry(path)
                    if path == std::path::Path::new("Configuration.xml") =>
                {
                    hook_cancellation.cancel();
                }
                SecureTreePhase::AfterRebindEntry(path)
                    if path != std::path::Path::new("Configuration.xml") =>
                {
                    hook_read_after_cancellation.set(true);
                }
                _ => {}
            },
            || {
                UnicaApplication::new().call_tool_cancellable(
                    "unica.subsystem.info",
                    &args,
                    cancellation,
                )
            },
        )
        .unwrap();

        assert!(!result.ok, "{result:?}");
        assert!(
            result
                .errors
                .iter()
                .any(|error| error.starts_with("cancelled:")),
            "{result:?}"
        );
        assert!(
            !read_after_cancellation.get(),
            "public preflight continued into registered descriptors after cancellation"
        );
        assert!(
            result.diagnostics.as_ref().is_none_or(|diagnostics| {
                !diagnostics.to_string().contains("provider_unavailable")
            }),
            "cancellation must not be mislabeled as provider_unavailable: {result:?}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn public_subsystem_info_uses_one_registered_snapshot_for_guard_and_handler() {
        let (root, workspace, child) =
            subsystem_format_guard_workspace("unica-subsystem-single-public-snapshot");
        let args = Map::from_iter([
            (
                "cwd".to_string(),
                Value::String(workspace.display().to_string()),
            ),
            (
                "SubsystemPath".to_string(),
                Value::String("src/Subsystems".to_string()),
            ),
        ]);
        let completed_snapshots = Rc::new(Cell::new(0usize));
        let hook_completed_snapshots = Rc::clone(&completed_snapshots);
        let child_during_capture = child.clone();

        let result = with_secure_tree_test_hook(
            move |phase| {
                if phase == &SecureTreePhase::AfterFinalIdentityProofs {
                    let completed = hook_completed_snapshots.get();
                    hook_completed_snapshots.set(completed + 1);
                    if completed == 0 {
                        std::fs::write(
                            &child_during_capture,
                            child_subsystem_stub_xml("Child", "2.20"),
                        )
                        .unwrap();
                    }
                }
            },
            || UnicaApplication::new().call_tool("unica.subsystem.info", &args),
        )
        .unwrap();

        assert!(result.ok, "{result:?}");
        assert_eq!(
            completed_snapshots.get(),
            1,
            "format preflight and handler must consume one prepared registered snapshot"
        );
        assert_eq!(
            result.diagnostics.as_ref().unwrap()["formatCompatibility"]["actualFormat"],
            "2.21",
            "format warning must use the exact bytes captured before the descriptor changed"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn public_subsystem_info_deadline_covers_registered_preflight() {
        let (root, workspace, _) =
            subsystem_format_guard_workspace("unica-subsystem-public-preflight-deadline");
        let args = Map::from_iter([
            (
                "cwd".to_string(),
                Value::String(workspace.display().to_string()),
            ),
            (
                "SubsystemPath".to_string(),
                Value::String("src/Subsystems".to_string()),
            ),
        ]);
        let delayed = Rc::new(Cell::new(false));
        let hook_delayed = Rc::clone(&delayed);

        let result = with_secure_tree_test_hook(
            move |phase| {
                if phase == &SecureTreePhase::RootOpened && !hook_delayed.replace(true) {
                    std::thread::sleep(Duration::from_millis(5_100));
                }
            },
            || UnicaApplication::new().call_tool("unica.subsystem.info", &args),
        )
        .unwrap();

        assert!(!result.ok, "{result:?}");
        assert!(
            result
                .errors
                .iter()
                .any(|error| error.contains("provider deadline exceeded")),
            "{result:?}"
        );
        assert!(
            result.diagnostics.as_ref().is_none_or(|diagnostics| {
                !diagnostics.to_string().contains("provider_unavailable")
            }),
            "deadline expiry must not be mislabeled as provider_unavailable: {result:?}"
        );
        std::fs::remove_dir_all(root).unwrap();
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
                normalized_path(&target).display().to_string(),
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
                        normalized_path(&second).display().to_string(),
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
            normalized_path(&target_path).display().to_string(),
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
            ("role-compile", &["JsonPath", "OutputDir"][..]),
            (
                "role-edit",
                &["sourceSet", "metadataPath", "operations"][..],
            ),
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
                _mode: InvocationMode,
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
            ) -> Result<FormatGuardCheck, FormatGuardError> {
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
                spec: ToolSpec,
                args: &Map<String, Value>,
                _context: &WorkspaceContext,
                _mode: InvocationMode,
                _cancellation: &CancellationToken,
            ) -> Result<ports::HandlerOutcome, String> {
                self.record("handler", args);
                let outcome = AdapterOutcome::ok("alias recording");
                Ok(
                    if spec.execution == ToolExecution::Read
                        && spec.result_contract == ResultContract::Typed
                    {
                        ports::HandlerOutcome::with_data(outcome, json!({"fixture": true}))
                    } else {
                        ports::HandlerOutcome::plain(outcome)
                    },
                )
            }

            fn cache_report(
                &self,
                context: &WorkspaceContext,
                _events: &[DomainEvent],
                _mode: InvocationMode,
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
                    publication_warnings: Vec::new(),
                })
            }

            fn notify_invalidation(&self, _context: &WorkspaceContext, _events: &[DomainEvent]) {}
        }

        let cases = [
            (
                "unica.cf.info",
                json!({"configPath": "src"}),
                &[("ConfigPath", "configPath")][..],
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
            "configPath": "src"
        });
        UnicaApplication::with_ports(Arc::new(FixedOutcomePorts {
            outcome: AdapterOutcome::ok("same aliases"),
            data: Some(json!({"fixture": true})),
        }))
        .call_tool("unica.cf.info", same.as_object().unwrap())
        .expect("equal path aliases must collapse to one canonical value");

        let empty_and_value = json!({
            "ConfigPath": "",
            "configPath": "src"
        });
        UnicaApplication::with_ports(Arc::new(FixedOutcomePorts {
            outcome: AdapterOutcome::ok("empty alias ignored"),
            data: Some(json!({"fixture": true})),
        }))
        .call_tool("unica.cf.info", empty_and_value.as_object().unwrap())
        .expect("one non-empty path alias must win over empty aliases");

        let conflict = json!({
            "ConfigPath": "src-a",
            "configPath": "src-b"
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
                _mode: InvocationMode,
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
                _mode: InvocationMode,
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
                _mode: InvocationMode,
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
                    publication_warnings: Vec::new(),
                })
            }

            fn notify_invalidation(&self, _context: &WorkspaceContext, _events: &[DomainEvent]) {}
        }

        let ports = Arc::new(CancellationRecordingPorts::default());
        let app = UnicaApplication::with_ports(ports.clone());
        let token = CancellationToken::new();
        token.cancel();

        let result = app
            .call_tool_cancellable("unica.project.status", &Map::new(), token)
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
                _mode: InvocationMode,
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
                _mode: InvocationMode,
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
                mode: InvocationMode,
                cache_access: CacheAccess,
            ) -> Result<CacheReport, String> {
                self.reported.lock().unwrap().extend(cache_access.writes);
                Ok(CacheReport {
                    mode: if mode.is_preview() {
                        "dry-run"
                    } else {
                        "write"
                    }
                    .to_string(),
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
                    publication_warnings: Vec::new(),
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
    fn pre_recorded_cache_effect_skips_post_commit_cache_report() {
        struct PreRecordedCachePorts;

        impl ports::ApplicationPorts for PreRecordedCachePorts {
            fn discover_workspace(
                &self,
                requested_cwd: Option<PathBuf>,
            ) -> Result<WorkspaceContext, String> {
                let cwd = requested_cwd.unwrap_or_default();
                Ok(WorkspaceContext {
                    cwd: cwd.clone(),
                    workspace_root: cwd.clone(),
                    cache_root: cwd.join(".build/unica"),
                    workspace_epoch: 1,
                })
            }

            fn validate_tool_context(
                &self,
                _spec: ToolSpec,
                _args: &Map<String, Value>,
                _mode: InvocationMode,
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
                context: &WorkspaceContext,
                _mode: InvocationMode,
                _cancellation: &CancellationToken,
            ) -> Result<ports::HandlerOutcome, String> {
                let event = DomainEvent::new(DomainEventKind::ModuleChanged, "src/Module.bsl");
                let mut adapter = AdapterOutcome::ok("source and cache state committed");
                adapter.changes = vec!["updated src/Module.bsl".to_string()];
                let mut outcome = ports::HandlerOutcome::with_data_and_events(
                    adapter,
                    serde_json::json!({"postHash": "sha256:after"}),
                    vec![event],
                );
                outcome.recorded_cache = Some(CacheReport {
                    mode: "applied".to_string(),
                    root: context.cache_root.display().to_string(),
                    workspace_epoch: context.workspace_epoch,
                    events: vec!["ModuleChanged".to_string()],
                    invalidated: vec!["bsl_diagnostics".to_string(), "bsl_index".to_string()],
                    refreshed: Vec::new(),
                    lazy_rebuilt: Vec::new(),
                    stale: vec!["bsl_diagnostics".to_string(), "bsl_index".to_string()],
                    fresh: Vec::new(),
                    publication_warnings: vec![
                        "transaction committed with one cleanup warning".to_string()
                    ],
                });
                Ok(outcome)
            }

            fn cache_report(
                &self,
                _context: &WorkspaceContext,
                _events: &[DomainEvent],
                _mode: InvocationMode,
                _cache_access: CacheAccess,
            ) -> Result<CacheReport, String> {
                panic!("post-commit cache_report must not run after transactional persistence")
            }

            fn notify_invalidation(&self, _context: &WorkspaceContext, _events: &[DomainEvent]) {}
        }

        let root =
            std::env::temp_dir().join(format!("unica-pre-recorded-cache-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let mut args = Map::new();
        args.insert("cwd".to_string(), Value::String(root.display().to_string()));
        args.insert("dryRun".to_string(), Value::Bool(false));

        let result = UnicaApplication::with_ports(Arc::new(PreRecordedCachePorts))
            .call_tool("unica.build.load", &args)
            .unwrap();

        assert!(result.ok);
        assert_eq!(result.cache.mode, "applied");
        assert_eq!(result.cache.events, ["ModuleChanged"]);
        assert_eq!(
            result.warnings,
            ["transaction committed with one cleanup warning"]
        );
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
        assert_eq!(info.data.unwrap()["support"]["editingEnabled"], true);

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
        assert_eq!(info.data.unwrap()["support"]["editingEnabled"], false);

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
        info_args.insert("sourceSet".to_string(), Value::String("main".to_string()));
        info_args.insert(
            "metadataPath".to_string(),
            Value::String("Catalog.Items".to_string()),
        );
        let info =
            call_public_tool_from_workspace(&workspace, "unica.meta.info", &info_args).unwrap();
        let info_data = info.data.as_ref().expect("meta.info answers with data");
        assert_eq!(
            info_data["support"],
            serde_json::json!("supported"),
            "{info_data:?}"
        );

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
    fn template_add_preserves_single_object_bom() {
        let root = temp_meta_compile_workspace("unica-template-add-single-bom");
        let workspace = root.join("workspace");
        let result = call_typed_meta_add(&workspace, "Report", "TemplateBomReport");
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
        let result = call_typed_meta_add(&workspace, "Report", "TemplateRepeatedBomReport");
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
    fn help_add_routes_through_unica_and_creates_help_files() {
        let root = test_workspace_root("unica-help-add");
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
        let initialized = UnicaApplication::new()
            .call_tool(
                "unica.cf.init",
                &Map::from_iter([
                    (
                        "cwd".to_string(),
                        Value::String(workspace.display().to_string()),
                    ),
                    ("Name".to_string(), Value::String("HelpAdd".to_string())),
                    ("OutputDir".to_string(), Value::String("src".to_string())),
                    ("dryRun".to_string(), Value::Bool(false)),
                ]),
            )
            .unwrap();
        assert!(initialized.ok, "{:?}", initialized.errors);
        let catalog_result = call_typed_meta_add(&workspace, "Catalog", "Items");
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
        args.insert(
            "ObjectName".to_string(),
            Value::String("Catalogs/Items".to_string()),
        );
        args.insert("SrcDir".to_string(), Value::String("src".to_string()));

        let mut results = Vec::new();
        for dry_run in [false, true] {
            args.insert("dryRun".to_string(), Value::Bool(dry_run));
            let result = UnicaApplication::new()
                .call_tool("unica.help.add", &args)
                .unwrap();

            assert!(!result.ok, "dryRun={dry_run}: {result:?}");
            assert_eq!(
                result.summary,
                if dry_run {
                    "dry run: unica.help.add blocked by support guard"
                } else {
                    "unica.help.add blocked by support guard"
                }
            );
            assert!(!ext.join("Help.xml").exists());
            assert!(result.cache.events.is_empty(), "{result:?}");
            results.push(result);
        }
        assert_support_guard_block_parity(&results[0], &results[1]);

        let _ = std::fs::remove_dir_all(root);
    }

    struct FixedOutcomePorts {
        outcome: AdapterOutcome,
        data: Option<Value>,
    }

    #[derive(Clone, Copy, Debug)]
    enum FailingXdtoGuard {
        Format,
        Support,
    }

    struct FailingXdtoGuardPorts {
        guard: FailingXdtoGuard,
    }

    impl ports::ApplicationPorts for FailingXdtoGuardPorts {
        fn discover_workspace(
            &self,
            requested_cwd: Option<PathBuf>,
        ) -> Result<WorkspaceContext, String> {
            let cwd = requested_cwd.unwrap_or_default();
            Ok(WorkspaceContext {
                cwd: cwd.clone(),
                workspace_root: cwd.clone(),
                cache_root: cwd.join(".build/unica"),
                workspace_epoch: 1,
            })
        }

        fn validate_tool_context(
            &self,
            _spec: ToolSpec,
            _args: &Map<String, Value>,
            _mode: InvocationMode,
            _context: &WorkspaceContext,
        ) -> Result<(), String> {
            Ok(())
        }

        fn evaluate_format_guard(
            &self,
            _spec: ToolSpec,
            _args: &Map<String, Value>,
            _context: &WorkspaceContext,
        ) -> Result<FormatGuardCheck, FormatGuardError> {
            match self.guard {
                FailingXdtoGuard::Format => Err(FormatGuardError::internal(
                    "failed to inspect /private/provider/workspace/src/Configuration.xml"
                        .to_string(),
                )),
                FailingXdtoGuard::Support => Ok(FormatGuardCheck::Allow),
            }
        }

        fn evaluate_support_guard(
            &self,
            _spec: ToolSpec,
            _args: &Map<String, Value>,
            _context: &WorkspaceContext,
        ) -> Result<SupportGuardCheck, String> {
            Err("failed to inspect /private/provider/workspace/src/XDTOPackages/Sample/Ext/Package.bin".to_string())
        }

        fn invoke_handler(
            &self,
            _spec: ToolSpec,
            _args: &Map<String, Value>,
            _context: &WorkspaceContext,
            _mode: InvocationMode,
            _cancellation: &CancellationToken,
        ) -> Result<ports::HandlerOutcome, String> {
            Err("handler must not run after a guard evaluation error".to_string())
        }

        fn cache_report(
            &self,
            context: &WorkspaceContext,
            _events: &[DomainEvent],
            mode: InvocationMode,
            _cache_access: CacheAccess,
        ) -> Result<CacheReport, String> {
            Ok(CacheReport {
                mode: if mode.is_preview() {
                    "dry-run"
                } else {
                    "applied"
                }
                .to_string(),
                root: context.cache_root.display().to_string(),
                workspace_epoch: context.workspace_epoch,
                events: Vec::new(),
                invalidated: Vec::new(),
                refreshed: Vec::new(),
                lazy_rebuilt: Vec::new(),
                stale: Vec::new(),
                fresh: Vec::new(),
                publication_warnings: Vec::new(),
            })
        }

        fn notify_invalidation(&self, _context: &WorkspaceContext, _events: &[DomainEvent]) {}
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
            _mode: InvocationMode,
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
            _mode: InvocationMode,
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
            mode: InvocationMode,
            _cache_access: CacheAccess,
        ) -> Result<CacheReport, String> {
            Ok(CacheReport {
                mode: if events.is_empty() {
                    "read".to_string()
                } else if mode.is_preview() {
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
                publication_warnings: Vec::new(),
            })
        }

        fn notify_invalidation(&self, _context: &WorkspaceContext, _events: &[DomainEvent]) {}
    }

    #[test]
    fn successful_typed_reader_without_data_fails_closed() {
        let error = UnicaApplication::with_ports(Arc::new(FixedOutcomePorts {
            outcome: AdapterOutcome::ok("reader omitted its typed payload"),
            data: None,
        }))
        .call_tool("unica.project.status", &Map::new())
        .expect_err("successful typed reader without data must fail closed");

        assert_eq!(
            error,
            "typed_result_missing: unica.project.status returned ok without OperationResult.data"
        );
    }

    #[test]
    fn successful_typed_reader_with_stdout_duplicate_fails_closed() {
        let mut outcome = AdapterOutcome::ok("reader duplicated its typed payload");
        outcome.stdout = Some("rendered result".to_string());

        let error = UnicaApplication::with_ports(Arc::new(FixedOutcomePorts {
            outcome,
            data: Some(json!({"fixture": true})),
        }))
        .call_tool("unica.project.status", &Map::new())
        .expect_err("successful typed reader must not duplicate data in stdout");

        assert_eq!(
            error,
            "typed_result_textual: unica.project.status returned ok with a stdout duplicate"
        );
    }

    #[test]
    fn failed_typed_reader_may_omit_data() {
        let mut outcome = AdapterOutcome::ok("reader failed before producing data");
        outcome.ok = false;
        outcome.errors.push("invalid source input".to_string());

        let result = UnicaApplication::with_ports(Arc::new(FixedOutcomePorts {
            outcome,
            data: None,
        }))
        .call_tool("unica.project.status", &Map::new())
        .expect("typed reader failure may omit data");

        assert!(!result.ok);
        assert!(result.data.is_none());
    }

    #[test]
    fn successful_typed_mutation_may_omit_data() {
        let result = UnicaApplication::with_ports(Arc::new(FixedOutcomePorts {
            outcome: AdapterOutcome::ok("mutation completed without a typed receipt"),
            data: None,
        }))
        .call_tool("unica.cf.edit", &Map::new())
        .expect("typed mutation remains outside the reader postcondition");

        assert!(result.ok);
        assert!(result.data.is_none());
    }

    #[test]
    fn successful_external_stream_reader_may_omit_data() {
        let args = Map::from_iter([(
            "ConfigPath".to_string(),
            Value::String("src/Configuration.xml".to_string()),
        )]);
        let result = UnicaApplication::with_ports(Arc::new(FixedOutcomePorts {
            outcome: AdapterOutcome::ok("validator reported through its external stream"),
            data: None,
        }))
        .call_tool("unica.cf.validate", &args)
        .expect("external-stream reader remains outside the typed postcondition");

        assert!(result.ok);
        assert!(result.data.is_none());
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

    fn xdto_public_guard_workspace(
        prefix: &str,
        source_format_version: &str,
        support_mode: Option<&str>,
    ) -> (std::path::PathBuf, std::path::PathBuf) {
        let root = test_workspace_root(prefix);
        let workspace = root.join("workspace");
        let src = workspace.join("src");
        let package = src.join("XDTOPackages/Sample/Ext/Package.bin");
        std::fs::create_dir_all(package.parent().unwrap()).unwrap();
        std::fs::create_dir_all(src.join("Ext")).unwrap();
        std::fs::write(
            workspace.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        if let Some(mode) = support_mode {
            std::fs::write(
                workspace.join(".v8-project.json"),
                format!(r#"{{"editingAllowedCheck":"{mode}"}}"#),
            )
            .unwrap();
        }
        std::fs::write(
            src.join("Configuration.xml"),
            format!(
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="{source_format_version}"><Configuration uuid="aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"><Properties><Name>Main</Name></Properties><ChildObjects><XDTOPackage>Sample</XDTOPackage></ChildObjects></Configuration></MetaDataObject>"#
            ),
        )
        .unwrap();
        std::fs::write(
            src.join("XDTOPackages/Sample.xml"),
            format!(
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="{source_format_version}"><XDTOPackage uuid="bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"><Properties><Name>Sample</Name><Namespace>urn:test</Namespace></Properties></XDTOPackage></MetaDataObject>"#
            ),
        )
        .unwrap();
        std::fs::write(
            package,
            r#"<package xmlns="http://v8.1c.ru/8.1/xdto" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:tns="urn:test" targetNamespace="urn:test">
	<objectType name="Existing"/>
</package>"#,
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
        (root, workspace)
    }

    fn xdto_public_edit_args(
        workspace: &std::path::Path,
        metadata_path: &str,
    ) -> Map<String, Value> {
        Map::from_iter([
            (
                "cwd".to_string(),
                Value::String(workspace.display().to_string()),
            ),
            ("sourceSet".to_string(), json!("main")),
            ("metadataPath".to_string(), json!(metadata_path)),
            ("operation".to_string(), json!("add-object-type")),
            ("name".to_string(), json!("Added")),
        ])
    }

    fn xdto_public_info_args(
        workspace: &std::path::Path,
        source_set: &str,
        metadata_path: &str,
    ) -> Map<String, Value> {
        Map::from_iter([
            (
                "cwd".to_string(),
                Value::String(workspace.display().to_string()),
            ),
            ("sourceSet".to_string(), json!(source_set)),
            ("metadataPath".to_string(), json!(metadata_path)),
        ])
    }

    fn assert_xdto_public_error_is_logical(
        error: &str,
        expected_code: &str,
        expected_target: &str,
        workspace: &std::path::Path,
    ) {
        assert!(error.starts_with(expected_code), "{error}");
        assert!(error.contains(expected_target), "{error}");
        let serialized = serde_json::to_string(error).unwrap();
        for forbidden in [
            workspace.display().to_string(),
            workspace.join("src").display().to_string(),
            "XDTOPackages/Sample/Ext/Package.bin".to_string(),
            "XDTOPackages\\Sample\\Ext\\Package.bin".to_string(),
            "XDTOPackages/Sample.xml".to_string(),
            "XDTOPackages\\Sample.xml".to_string(),
            "Package.bin".to_string(),
            "Configuration.xml".to_string(),
            "provider".to_string(),
        ] {
            assert!(
                !serialized.contains(&forbidden),
                "leaked {forbidden:?}: {serialized}"
            );
        }
    }

    fn assert_xdto_public_fields_are_logical(
        result: &OperationResult,
        workspace: &std::path::Path,
    ) {
        let public_fields = serde_json::to_string(&json!({
            "summary": result.summary,
            "changes": result.changes,
            "warnings": result.warnings,
            "errors": result.errors,
            "artifacts": result.artifacts,
            "stdout": result.stdout,
            "stderr": result.stderr,
            "command": result.command,
            "diagnostics": result.diagnostics,
            "data": result.data,
        }))
        .unwrap();
        for forbidden in [
            workspace.display().to_string(),
            workspace.join("src").display().to_string(),
            "XDTOPackages/Sample/Ext/Package.bin".to_string(),
            "XDTOPackages\\Sample\\Ext\\Package.bin".to_string(),
            "XDTOPackages/Sample.xml".to_string(),
            "XDTOPackages\\Sample.xml".to_string(),
        ] {
            assert!(
                !public_fields.contains(&forbidden),
                "leaked {forbidden:?}: {public_fields}"
            );
        }
    }

    fn source_tree_snapshot(
        root: &std::path::Path,
    ) -> Vec<(std::path::PathBuf, &'static str, Vec<u8>, Option<String>)> {
        fn visit(
            root: &std::path::Path,
            current: &std::path::Path,
            snapshot: &mut Vec<(std::path::PathBuf, &'static str, Vec<u8>, Option<String>)>,
        ) {
            let mut entries = std::fs::read_dir(current)
                .unwrap()
                .map(Result::unwrap)
                .collect::<Vec<_>>();
            entries.sort_by_key(std::fs::DirEntry::file_name);
            for entry in entries {
                let path = entry.path();
                let relative = path.strip_prefix(root).unwrap().to_path_buf();
                let metadata = std::fs::symlink_metadata(&path).unwrap();
                if metadata.file_type().is_symlink() {
                    snapshot.push((
                        relative,
                        "symlink",
                        std::fs::read_link(&path)
                            .unwrap()
                            .as_os_str()
                            .to_string_lossy()
                            .as_bytes()
                            .to_vec(),
                        None,
                    ));
                } else if metadata.is_dir() {
                    snapshot.push((relative, "directory", Vec::new(), None));
                    visit(root, &path, snapshot);
                } else {
                    let identity = file_identity_for_test(&path).unwrap();
                    snapshot.push((relative, "file", std::fs::read(&path).unwrap(), identity));
                }
            }
        }

        let mut snapshot = Vec::new();
        visit(root, root, &mut snapshot);
        snapshot
    }

    #[test]
    fn legacy_read_only_output_sinks_cannot_change_source_path_aliases() {
        let root = test_workspace_root("unica-read-only-output-aliases");
        let workspace = root.join("workspace");
        let src = workspace.join("src");
        let protected = src.join("protected-source.xml");
        let hard_link = src.join("protected-hard-link.xml");
        let symlink = src.join("protected-symlink.xml");
        let outside = root.join("outside/protected-source.xml");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(outside.parent().unwrap()).unwrap();
        std::fs::write(
            workspace.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(&protected, b"source bytes").unwrap();
        std::fs::write(&outside, b"outside bytes").unwrap();
        std::fs::hard_link(&protected, &hard_link).unwrap();
        let symlink_created = match create_file_link_fixture_for_test(&protected, &symlink)
            .expect("unexpected file-link creation error must fail the fixture test")
        {
            FileLinkFixtureOutcome::Created => true,
            FileLinkFixtureOutcome::Unsupported => {
                eprintln!("[SKIPPED FIXTURE] file links are unsupported on this host");
                false
            }
            FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => {
                eprintln!("[SKIPPED FIXTURE] Windows file-link privilege is unavailable");
                false
            }
        };

        let mut sink_targets = vec![
            ("same source", protected.clone()),
            (
                "parent traversal",
                workspace.join("src/../../outside/protected-source.xml"),
            ),
            ("hard-link alias", hard_link),
        ];
        if symlink_created {
            sink_targets.push(("symlink alias", symlink));
        }

        let affected_tools = [
            ("unica.cf.info", "ConfigPath", "OutFile", "outFile"),
            ("unica.cf.validate", "ConfigPath", "OutFile", "outFile"),
            ("unica.cfe.validate", "ExtensionPath", "OutFile", "outFile"),
            // Retired metadata readers are covered by the exact unknown-tool
            // contract; typed meta.info has no output sink in its schema.
            ("unica.interface.validate", "CIPath", "OutFile", "outFile"),
            (
                "unica.subsystem.info",
                "SubsystemPath",
                "OutFile",
                "outFile",
            ),
            (
                "unica.subsystem.validate",
                "SubsystemPath",
                "OutFile",
                "outFile",
            ),
            ("unica.dcs.info", "TemplatePath", "OutFile", "outFile"),
            ("unica.dcs.validate", "TemplatePath", "OutFile", "outFile"),
            ("unica.role.info", "RightsPath", "OutFile", "outFile"),
            ("unica.role.validate", "RightsPath", "OutFile", "outFile"),
            (
                "unica.mxl.decompile",
                "TemplatePath",
                "OutputPath",
                "outputPath",
            ),
        ];

        for (tool, input_argument, sink_argument, sink_alias) in affected_tools {
            for argument in [sink_argument, sink_alias] {
                for (target_kind, target) in &sink_targets {
                    let mut args = Map::new();
                    args.insert(
                        "cwd".to_string(),
                        Value::String(workspace.display().to_string()),
                    );
                    args.insert(
                        input_argument.to_string(),
                        Value::String("src/protected-source.xml".to_string()),
                    );
                    args.insert(
                        argument.to_string(),
                        Value::String(target.display().to_string()),
                    );
                    let before = source_tree_snapshot(&root);

                    let error = UnicaApplication::new()
                        .call_tool(tool, &args)
                        .expect_err("legacy output sink must be rejected by the public contract");

                    assert!(
                        error.contains(&format!("does not accept argument `{argument}`")),
                        "{tool} {argument} {target_kind}: {error}"
                    );
                    assert_eq!(
                        source_tree_snapshot(&root),
                        before,
                        "{tool} {argument} must not change the tree through {target_kind}"
                    );
                }
            }
        }

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn dcs_format_warnings_leave_source_trees_unchanged() {
        let dcs_fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
            "../../tests/fixtures/unica_mcp_script_parity/bsp/dcs/DataProcessors__ВыгрузкаЗагрузкаEnterpriseData__СхемаКомпоновкиДанных/Template.xml",
        );

        for (format, compatibility) in [("2.19", "older"), ("2.21", "newer")] {
            let root = test_workspace_root(&format!("unica-read-only-format-{format}"));
            let workspace = root.join("workspace");
            let src = workspace.join("src");
            let object = src.join("Catalogs/Items.xml");
            let template = src.join("Reports/Sales/Templates/Main/Ext/Template.xml");
            std::fs::create_dir_all(object.parent().unwrap()).unwrap();
            std::fs::create_dir_all(template.parent().unwrap()).unwrap();
            std::fs::write(
                workspace.join("v8project.yaml"),
                "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
            )
            .unwrap();
            std::fs::write(
                src.join("Configuration.xml"),
                support_test_configuration_xml("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa").replacen(
                    r#"version="2.20""#,
                    &format!(r#"version="{format}""#),
                    1,
                ),
            )
            .unwrap();
            std::fs::write(
                &object,
                support_test_catalog_xml("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb").replacen(
                    r#"version="2.20""#,
                    &format!(r#"version="{format}""#),
                    1,
                ),
            )
            .unwrap();
            std::fs::copy(&dcs_fixture, &template).unwrap();

            let path_selector = |name: &str, path: &std::path::Path| {
                vec![(name.to_string(), Value::String(path.display().to_string()))]
            };
            for (tool, selector) in [
                ("unica.dcs.info", path_selector("TemplatePath", &template)),
                (
                    "unica.dcs.validate",
                    path_selector("TemplatePath", &template),
                ),
            ] {
                let mut args = Map::new();
                args.insert(
                    "cwd".to_string(),
                    Value::String(workspace.display().to_string()),
                );
                for (key, value) in selector {
                    args.insert(key, value);
                }
                let before = source_tree_snapshot(&src);

                let result = UnicaApplication::new().call_tool(tool, &args).unwrap();

                assert!(
                    !result.warnings.is_empty(),
                    "{tool} must preserve the {format} warning: {result:?}"
                );
                let diagnostic = &result.diagnostics.as_ref().unwrap()["formatCompatibility"];
                assert_eq!(diagnostic["actualFormat"], format, "{tool}: {result:?}");
                assert_eq!(
                    diagnostic["compatibility"], compatibility,
                    "{tool}: {result:?}"
                );
                assert_eq!(
                    source_tree_snapshot(&src),
                    before,
                    "{tool} must not change a {format} source tree"
                );
            }

            std::fs::remove_dir_all(root).unwrap();
        }
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
        write_support_test_language(&src);
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

    fn leading_utf8_bom_count(bytes: &[u8]) -> usize {
        bytes
            .chunks_exact(3)
            .take_while(|chunk| *chunk == [0xEF, 0xBB, 0xBF])
            .count()
    }

    fn call_typed_meta_add(workspace: &std::path::Path, kind: &str, name: &str) -> OperationResult {
        let _cwd = crate::test_support::ProcessCwdGuard::enter(workspace).unwrap();
        UnicaApplication::new()
            .call_tool(
                "unica.meta.add",
                &Map::from_iter([
                    ("sourceSet".to_string(), Value::String("main".to_string())),
                    ("kind".to_string(), Value::String(kind.to_string())),
                    ("name".to_string(), Value::String(name.to_string())),
                    ("dryRun".to_string(), Value::Bool(false)),
                ]),
            )
            .unwrap()
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

    fn write_support_test_language(src: &std::path::Path) {
        let languages = src.join("Languages");
        std::fs::create_dir_all(&languages).unwrap();
        std::fs::write(
            languages.join("Russian.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
  <Language uuid="eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee">
    <Properties>
      <Name>Russian</Name>
      <Synonym/>
      <Comment/>
      <LanguageCode>ru</LanguageCode>
    </Properties>
  </Language>
</MetaDataObject>"#,
        )
        .unwrap();
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

    fn support_test_subsystem_xml(uuid: &str, name: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
  <Subsystem uuid="{uuid}">
    <Properties>
      <Name>{name}</Name>
      <Synonym/>
      <Comment/>
      <IncludeHelpInContents>true</IncludeHelpInContents>
      <IncludeInCommandInterface>true</IncludeInCommandInterface>
      <UseOneCommand>false</UseOneCommand>
      <Explanation/>
      <Picture/>
      <Content/>
    </Properties>
    <ChildObjects/>
  </Subsystem>
</MetaDataObject>"#
        )
    }

    fn assert_support_guard_block_parity(applied: &OperationResult, preview: &OperationResult) {
        assert_eq!(preview.summary, format!("dry run: {}", applied.summary));
        assert_eq!(preview.ok, applied.ok);
        assert_eq!(preview.changes, applied.changes);
        assert_eq!(preview.warnings, applied.warnings);
        assert_eq!(preview.errors, applied.errors);
        assert_eq!(preview.artifacts, applied.artifacts);
        assert_eq!(preview.stdout, applied.stdout);
        assert_eq!(preview.stderr, applied.stderr);
        assert_eq!(preview.command, applied.command);
        assert_eq!(preview.diagnostics, applied.diagnostics);
        assert_eq!(preview.data, applied.data);
        assert_eq!(preview.job, applied.job);
        assert_eq!(preview.cache.mode, applied.cache.mode);
        assert_eq!(preview.cache.root, applied.cache.root);
        assert_eq!(preview.cache.workspace_epoch, applied.cache.workspace_epoch);
        assert_eq!(preview.cache.events, applied.cache.events);
        assert_eq!(preview.cache.invalidated, applied.cache.invalidated);
        assert_eq!(preview.cache.refreshed, applied.cache.refreshed);
        assert_eq!(preview.cache.lazy_rebuilt, applied.cache.lazy_rebuilt);
        assert_eq!(preview.cache.stale, applied.cache.stale);
        assert_eq!(preview.cache.fresh, applied.cache.fresh);
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
        write_support_test_language(&src);
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
    fn read_only_native_outfile_is_rejected_before_any_write() {
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
            Ok(result) => panic!(
                "expected OutFile contract rejection, got {}",
                result.summary
            ),
            Err(error) => error,
        };

        assert!(
            error.contains("does not accept argument `OutFile`"),
            "{error}"
        );
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

    /// The published `OperationResult` is where a consumer reads the effect of
    /// a borrow, so `data.mutation`, `changes` and the workspace itself must
    /// agree on which files were created and which were replaced.
    #[test]
    fn cfe_borrow_result_mutation_changes_and_workspace_agree() {
        let root =
            std::env::temp_dir().join(format!("unica-cfe-borrow-mutation-{}", std::process::id()));
        let workspace = root.join("workspace");
        std::fs::create_dir_all(workspace.join("src/Catalogs")).unwrap();
        std::fs::create_dir_all(workspace.join("ext")).unwrap();
        std::fs::write(
            workspace.join("src/Configuration.xml"),
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\">\n\t<Configuration uuid=\"55555555-5555-5555-5555-555555555555\"/>\n</MetaDataObject>\n",
        )
        .unwrap();
        std::fs::write(
            workspace.join("src/Catalogs/Items.xml"),
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\">\n\t<Catalog uuid=\"aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa\">\n\t\t<Properties><Name>Items</Name></Properties>\n\t\t<ChildObjects/>\n\t</Catalog>\n</MetaDataObject>\n",
        )
        .unwrap();
        let extension_owner = workspace.join("ext/Configuration.xml");
        std::fs::write(
            &extension_owner,
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\">\n\t<Configuration uuid=\"66666666-6666-6666-6666-666666666666\">\n\t\t<InternalInfo/>\n\t\t<Properties>\n\t\t\t<ObjectBelonging>Adopted</ObjectBelonging>\n\t\t\t<Name>SmokeExtension</Name>\n\t\t\t<ConfigurationExtensionPurpose>Customization</ConfigurationExtensionPurpose>\n\t\t\t<NamePrefix>SE_</NamePrefix>\n\t\t</Properties>\n\t\t<ChildObjects/>\n\t</Configuration>\n</MetaDataObject>\n",
        )
        .unwrap();
        let borrowed = workspace.join("ext/Catalogs/Items.xml");
        assert!(!borrowed.exists());

        let mut args = Map::new();
        args.insert(
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        );
        args.insert("dryRun".to_string(), Value::Bool(false));
        args.insert(
            "ExtensionPath".to_string(),
            Value::String("ext".to_string()),
        );
        args.insert("ConfigPath".to_string(), Value::String("src".to_string()));
        args.insert(
            "Object".to_string(),
            Value::String("Catalog.Items".to_string()),
        );

        let result = UnicaApplication::new()
            .call_tool("unica.cfe.borrow", &args)
            .expect("borrow must succeed");

        assert!(result.ok, "{:?}", result.errors);
        let mutation = &result
            .data
            .as_ref()
            .expect("cfe.borrow publishes typed data")["mutation"];
        let created = mutation["created"]
            .as_array()
            .expect("created is an array")
            .iter()
            .map(|value| value.as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        let updated = mutation["updated"]
            .as_array()
            .expect("updated is an array")
            .iter()
            .map(|value| value.as_str().unwrap().to_string())
            .collect::<Vec<_>>();

        assert!(borrowed.exists(), "the borrow must have created the object");
        assert!(
            created.contains(&borrowed.display().to_string()),
            "{created:?}"
        );
        assert!(
            updated.contains(&extension_owner.display().to_string()),
            "{updated:?}"
        );
        for path in &created {
            assert!(!updated.contains(path), "{path} is created and updated");
        }
        let expected_changes = created
            .iter()
            .map(|path| format!("created {path}"))
            .chain(updated.iter().map(|path| format!("updated {path}")))
            .collect::<Vec<_>>();
        assert_eq!(result.changes, expected_changes);

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
    fn subsystem_compile_guards_locked_parent_before_both_planners() {
        for dry_run in [false, true] {
            let root = test_workspace_root(if dry_run {
                "unica-subsystem-parent-guard-preview"
            } else {
                "unica-subsystem-parent-guard-apply"
            });
            let workspace = root.join("workspace");
            let src = workspace.join("src");
            let ext = src.join("Ext");
            let subsystems = src.join("Subsystems");
            std::fs::create_dir_all(&ext).unwrap();
            std::fs::create_dir_all(&subsystems).unwrap();
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
            let parent_path = subsystems.join("Parent.xml");
            std::fs::write(
                &parent_path,
                support_test_subsystem_xml("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb", "Parent"),
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
            let before = std::fs::read(&parent_path).unwrap();
            let args = json!({
                "cwd": workspace,
                "dryRun": dry_run,
                "OutputDir": "src",
                "Parent": "src/Subsystems/Parent.xml",
                "Value": r#"{"name":"Child"}"#
            })
            .as_object()
            .unwrap()
            .clone();

            let result = UnicaApplication::new()
                .call_tool("unica.subsystem.compile", &args)
                .unwrap();
            let after = std::fs::read(&parent_path).unwrap();
            let normalized_parent = normalized_path(&parent_path).display().to_string();
            let child_exists = src.join("Subsystems/Parent/Subsystems/Child.xml").exists();
            std::fs::remove_dir_all(root).unwrap();

            assert!(!result.ok, "dryRun={dry_run}: {result:?}");
            assert_eq!(
                result.summary,
                if dry_run {
                    "dry run: unica.subsystem.compile blocked by support guard"
                } else {
                    "unica.subsystem.compile blocked by support guard"
                }
            );
            assert!(result.errors.join("\n").contains("на замке"), "{result:?}");
            assert_eq!(result.artifacts, [normalized_parent]);
            assert!(result.cache.events.is_empty(), "{result:?}");
            assert_eq!(after, before, "dryRun={dry_run}");
            assert!(!child_exists, "dryRun={dry_run}");
        }
    }

    #[test]
    fn subsystem_compile_retains_locked_configuration_fallback_without_parent() {
        let config_uuid = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa";
        let (root, workspace, _bin_path) = support_test_workspace(
            "unica-subsystem-root-guard",
            support_test_parent_configurations_bin(
                config_uuid,
                config_uuid,
                "cccccccc-cccc-cccc-cccc-cccccccccccc",
            ),
        );
        let src = workspace.join("src");
        let config_path = src.join("Configuration.xml");
        let before = std::fs::read(&config_path).unwrap();
        let mut args = json!({
            "cwd": workspace,
            "OutputDir": "src",
            "Value": r#"{"name":"RootChild"}"#
        })
        .as_object()
        .unwrap()
        .clone();
        let mut results = Vec::new();

        for dry_run in [false, true] {
            args.insert("dryRun".to_string(), Value::Bool(dry_run));
            let result = UnicaApplication::new()
                .call_tool("unica.subsystem.compile", &args)
                .unwrap();

            assert!(!result.ok, "dryRun={dry_run}: {result:?}");
            assert_eq!(
                result.artifacts,
                [normalized_path(&src).display().to_string()]
            );
            assert!(result.errors.join("\n").contains("на замке"), "{result:?}");
            assert!(result.cache.events.is_empty(), "{result:?}");
            assert_eq!(std::fs::read(&config_path).unwrap(), before);
            assert!(!src.join("Subsystems/RootChild.xml").exists());
            results.push(result);
        }
        assert_support_guard_block_parity(&results[0], &results[1]);

        std::fs::remove_dir_all(root).unwrap();
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
