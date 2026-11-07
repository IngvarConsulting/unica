pub use operation_descriptors::SupportGuardRequirement;
use ports::{ApplicationPorts, FormatGuardCheck, FormatGuardError, SupportGuardCheck};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use unica_code_intelligence::coordinator as code_intelligence;
use unica_code_intelligence::model::CodeIntelligenceReadRequest;
use unica_code_intelligence::model::SearchRequest;
use unica_execution_contract::adapter_outcome::AdapterOutcome;
use unica_runner::operation::runtime_event_kind;
use unica_source_contract::events::{DomainEvent, DomainEventKind};
use unica_source_contract::workspace::WorkspaceContext;
use unica_work_control::cancellation::CancellationToken;
use unica_work_control::progress::{NoopProgressSink, ProgressSink};
use unica_work_control::provider_deadline::ProviderDeadline;
use unica_workspace::cache::{CacheAccess, CacheReport};

use unica_navigation::deferred_delivery;
pub(crate) mod diagnostics;
pub(crate) mod metadata;
pub mod operation_descriptors;
pub(crate) mod operational_config;
pub mod ports;
pub(crate) use unica_runner::admission as runtime_admission;
#[cfg(test)]
mod medium_application_tests;
#[cfg(test)]
mod runtime_admission_integration;

pub mod tool_contracts;
// The catalog is compiled for hidden canonical routing, while most semantic
// descriptors remain unused until the atomic Task 22 public cutover.
#[allow(dead_code)]
pub mod v13;
pub use tool_contracts::{input_schema_for_tool, strip_schema_descriptions};

#[doc(hidden)]
pub const PUBLIC_INVOCATION_DEADLINE: Duration = Duration::from_secs(5);

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

#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeAdmissionPolicy {
    Enforce,
    #[cfg(any(test, feature = "test-support"))]
    AlreadyAdmittedForDownstreamContractTest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultContract {
    Typed,
    ExternalStream,
}

/// What a mutating tool can honestly show before it writes.
///
/// #290 found the preview path decided by a growing set of exceptions keyed on
/// operation name: most typed mutation handlers ran only inside `!dry_run`, and
/// `code.patch`, `form.edit`, `meta.edit`, `role.edit` and `xdto.edit` were
/// carved out one at a time. A caller could not tell from the surface which
/// behaviour it would get.
///
/// The strategy is derived from the registry rather than restated per tool, so
/// a new mutator cannot be added without falling into one of these classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewStrategy {
    /// The tool computes the post-image it would write and answers with it.
    /// Every file mutator belongs here: the bytes are derivable without
    /// touching the workspace.
    PostImage,
    /// The tool answers with the external command it would run. A build or a
    /// runtime job cannot show its result without doing the work, but it can
    /// show exactly what it would launch.
    PlannedCommand,
}

/// Закрытый переходный список — типизированные мутаторы, чей
/// предпросмотр ещё отвечает общим успехом без предметных данных. Список
/// только сокращается; новый мутатор обязан родиться с честным предпросмотром.
/// Судьбы: `subsystem-edit` — #380, `interface-edit` —
/// #382, `cf-edit`/`cfe-borrow`/`form-*`/`support-edit` — последующие срезы
/// волны и вехи. Снятые срезом #375 `template-*` и `help-add` список уже
/// покинули; инициализаторы и `cfe-patch-method` ушли вместе с инструментами.
pub const PREVIEW_GATED_OPERATIONS: &[&str] =
    &["cf-edit", "cfe-borrow", "interface-edit", "subsystem-edit"];

/// The preview class of a mutating tool, or `None` for a reader.
///
/// Totality over the mutating registry is the point: the table-driven contract
/// test fails when a new mutator appears without a class, which is what turns
/// "growing set of exceptions" into a declared decision.
pub fn preview_strategy(tool: &ToolSpec) -> Option<PreviewStrategy> {
    if !tool.execution.is_mutating() {
        return None;
    }
    Some(match tool.handler {
        ToolHandler::BuildRuntime { .. }
        | ToolHandler::RuntimeAdapter
        | ToolHandler::RuntimeJob { .. } => PreviewStrategy::PlannedCommand,
        _ => PreviewStrategy::PostImage,
    })
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
        operation: unica_metadata::request::MetadataOperation,
    },
    NativeOperation {
        operation: &'static str,
        event: Option<DomainEventKind>,
    },
    BuildRuntime {
        command: &'static [&'static str],
        event: Option<DomainEventKind>,
    },
    RuntimeAdapter,
    Documentation {
        operation: &'static str,
    },
    RuntimeJob {
        action: RuntimeJobAction,
    },
    CodeIntelligence {
        operation: CodeIntelligenceOperation,
    },
    Diagnostics,
    CodeAdapter {
        command: &'static [&'static str],
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeIntelligenceOperation {
    Search,
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
    /// Состояние работы, не успевшей кончиться внутри вызова. Пустое не
    /// сериализуется: обычный вызов платит за этот слот ноль байтов.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub work: Option<unica_work_control::long_work::WorkState>,
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
                "pattern": unica_roles::ROLE_METADATA_PATH_PATTERN
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

/// Closed MCP schema for the provider-neutral code-search result.
///
/// Provider-specific attributes remain an intentionally open JSON object, but
/// the envelope, role sections, terminal reasons, completeness claims, counts,
/// hits, and logical location algebra are all machine-checkable. Once a search reaches the tool
/// result boundary it always carries `data`, including the three failed or
/// unavailable role sections when no provider served the request. Failures
/// before that boundary remain JSON-RPC errors and do not use this schema.
pub fn code_search_output_schema() -> Value {
    let string_array = || json!({"type": "array", "items": {"type": "string"}});
    let logical_location = json!({
        "oneOf": [
            {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "kind": {"const": "addressed"},
                    "sourceSet": {"type": "string", "minLength": 1},
                    "metadataPath": {"type": "string", "minLength": 1},
                    "targetKind": {
                        "type": "string",
                        "enum": ["sourceRoot", "metadataObject", "module"]
                    }
                },
                "required": ["kind", "sourceSet", "targetKind"]
            },
            {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "kind": {"const": "unaddressable"},
                    "sourceSet": {"type": "string", "minLength": 1},
                    "ownerMetadataPath": {"type": "string", "minLength": 1},
                    "path": {"type": "string", "minLength": 1}
                },
                "required": ["kind", "sourceSet", "path"]
            }
        ]
    });
    let hit = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "rank": {"type": "integer", "minimum": 1},
            "providerScore": {"type": "number"},
            "location": logical_location,
            "line": {"type": "integer", "minimum": 1},
            "endLine": {"type": ["integer", "null"], "minimum": 1},
            "symbol": {"type": ["string", "null"]},
            "kind": {"type": ["string", "null"]},
            "snippet": {"type": "string"},
            "attributes": {
                "type": "object",
                "description": "Provider-specific evidence; consumers must not branch on it."
            }
        },
        "required": [
            "location", "line", "endLine", "symbol", "kind", "snippet", "attributes"
        ]
    });
    let section = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "role": {"type": "string", "enum": ["semantic", "symbol", "lexical"]},
            "provider": {"type": "string", "minLength": 1},
            "status": {
                "type": "string",
                "enum": ["ok", "empty", "partial", "limitReached", "timedOut", "unavailable", "failed"]
            },
            "termination": {
                "oneOf": [
                    {"type": "null"},
                    {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {
                            "code": {
                                "type": "string",
                                "enum": [
                                    "limitReached", "deadlineExceeded", "dependencyPending",
                                    "unsupportedScope", "capacityExhausted",
                                    "providerUnavailable", "providerFailed"
                                ]
                            },
                            "retryable": {"type": "boolean"},
                            "detailCode": {"type": "string", "minLength": 1}
                        },
                        "required": ["code", "retryable"]
                    }
                ]
            },
            "searchComplete": {"type": "boolean"},
            "ranking": {"type": "string", "enum": ["provider", "none"]},
            "ordering": {"type": "string", "enum": ["provider", "providerTraversal"]},
            "matches": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "returned": {"type": "integer", "minimum": 0},
                    "total": {"type": "integer", "minimum": 0},
                    "relation": {"type": "string", "enum": ["exact", "lowerBound", "unknown"]}
                },
                "required": ["returned", "relation"]
            },
            "hits": {"type": "array", "items": hit},
            "diagnostics": string_array(),
            "artifacts": string_array()
        },
        "required": [
            "role", "provider", "status", "termination", "searchComplete", "ranking", "ordering",
            "matches", "hits", "diagnostics"
        ]
    });
    let mut schema = operation_result_output_schema();
    if let Some(properties) = schema["properties"].as_object_mut() {
        for forbidden in ["stdout", "stderr", "command", "job"] {
            properties.remove(forbidden);
        }
    }
    schema["properties"]["data"] = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "coverage": {"type": "string", "enum": ["complete", "partial", "none"]},
            "elapsedMs": {"type": "integer", "minimum": 0},
            "sections": {
                "type": "array",
                "minItems": 3,
                "maxItems": 3,
                "items": section
            }
        },
        "required": ["coverage", "elapsedMs", "sections"]
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
    unica_metadata::request::parse_metadata_request(operation, args)
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
    let error = unica_roles::parse_role_edit_request(args).err()?;
    let operation_index = error.operation_index?;
    let metadata_path = args.get("metadataPath")?.as_str()?.to_string();
    let message = error.message.clone();
    let data = unica_roles::RoleEditData::failed(
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
        work: None,
    })
}

/// Public application entry point.
pub struct UnicaApplication {
    pub ports: Arc<dyn ApplicationPorts + Send + Sync>,
    pub deferred: Arc<deferred_delivery::DeferredDelivery>,
}

impl UnicaApplication {
    pub fn with_ports(ports: Arc<dyn ApplicationPorts + Send + Sync>) -> Self {
        Self {
            ports,
            deferred: Arc::new(deferred_delivery::DeferredDelivery::default()),
        }
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
        self.call_tool_observed(name, args, cancellation, Arc::new(NoopProgressSink))
    }

    pub fn call_tool_observed(
        &self,
        name: &str,
        args: &Map<String, Value>,
        cancellation: CancellationToken,
        progress: Arc<dyn ProgressSink>,
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
        call_tool_observed(
            spec,
            args,
            self.ports.as_ref(),
            &cancellation,
            deadline,
            progress.as_ref(),
            self.deferred.as_ref(),
        )
    }
}

pub fn tools() -> Vec<ToolSpec> {
    let mut specs = configuration_tools();
    specs.extend([
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
            description: "Export a CF/CFE artifact out of the current infobase through the internal build/runtime adapter; it does not load source changes into that infobase first.",
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
                "Preview typed v8-runner workflows, or run a classified applied operation and answer with its terminal result plus a named risk warning; an unclassified operation still fails closed before workspace discovery or process spawn.",
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
            description: "Search one logical code scope concurrently through semantic, symbol, and lexical roles. Results preserve role-local ranking, explicit completeness and retryability, logical locations, and observable progress; sourceDir remains a mutually exclusive migration selector.",
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
            description: "Read provider-neutral diagnostics addressed by logical 1C source targets.",
            execution: ToolExecution::Read,
            result_contract: ResultContract::Typed,
            cache_access: CacheAccess {
                reads: &["bsl_diagnostics"],
                writes: &[],
            },
            handler: ToolHandler::Diagnostics,
        },
    ]);
    specs
}

#[cfg(any(test, feature = "test-support"))]
pub fn call_tool(
    spec: ToolSpec,
    args: &Map<String, Value>,
    ports: &dyn ApplicationPorts,
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
) -> Result<OperationResult, String> {
    call_tool_observed(
        spec,
        args,
        ports,
        cancellation,
        deadline,
        &NoopProgressSink,
        &deferred_delivery::DeferredDelivery::default(),
    )
}

fn call_tool_observed(
    spec: ToolSpec,
    args: &Map<String, Value>,
    ports: &dyn ApplicationPorts,
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
    progress: &dyn ProgressSink,
    deferred: &deferred_delivery::DeferredDelivery,
) -> Result<OperationResult, String> {
    call_tool_with_runtime_admission(
        spec,
        args,
        ports,
        cancellation,
        deadline,
        progress,
        RuntimeAdmissionPolicy::Enforce,
        deferred,
    )
}

// The dispatcher wires every cross-cutting invocation concern; a parameter
// struct here would only rename the coupling, so the lint is acknowledged.
#[allow(clippy::too_many_arguments)]
#[doc(hidden)]
pub fn call_tool_with_runtime_admission(
    spec: ToolSpec,
    args: &Map<String, Value>,
    ports: &dyn ApplicationPorts,
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
    progress: &dyn ProgressSink,
    runtime_admission: RuntimeAdmissionPolicy,
    deferred: &deferred_delivery::DeferredDelivery,
) -> Result<OperationResult, String> {
    let normalized_args = tool_contracts::normalize_native_path_aliases(spec, args)?;
    let args = &normalized_args;
    tool_contracts::validate_tool_argument_shape(spec, args)?;
    let mode = InvocationMode::from_validated_args(spec, args)?;
    tool_contracts::validate_tool_argument_semantics(spec, args, mode)?;
    let dry_run = mode.is_preview();
    // a continuation call is served from the immutable snapshot and
    // must not re-read the source, so it short-circuits before workspace
    // discovery and the reader dispatch.
    if mode == InvocationMode::Read && supports_deferred_delivery(&spec) {
        if let Some(result) = try_deferred_continuation(spec, args, deferred) {
            return Ok(result);
        }
    }
    // A classified applied operation executes and carries its named
    // risk into the result; only an unclassified one still fails closed.
    let mut applied_risk = None;
    if runtime_admission == RuntimeAdmissionPolicy::Enforce
        && matches!(spec.handler, ToolHandler::RuntimeAdapter)
        && !dry_run
    {
        match runtime_admission::runtime_risk_notice(spec.name, args)? {
            runtime_admission::RuntimeRiskOutcome::Refused(failure) => {
                // #549: отказ, назвавший одну причину из двух, уводит агента
                // рассуждать про непрерываемые фазы, когда ставить нечего.
                let missing = ports.missing_engine(
                    spec,
                    args.get("cwd")
                        .and_then(Value::as_str)
                        .map(std::path::Path::new),
                );
                return Ok(runtime_receipt_admission_result(spec, failure, missing));
            }
            runtime_admission::RuntimeRiskOutcome::Warned(notice) => {
                applied_risk = Some(notice);
            }
        }
    }
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
                work: None,
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
            work: None,
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
                    work: None,
                });
            }
        }
    } else {
        None
    };

    let requires_operational_config = operational_config::requires_snapshot(spec, args);
    if requires_operational_config && cancellation.is_cancelled() {
        return Err(unica_work_control::cancellation::cancelled_error(
            "operational config resolution stopped before reading workspace files",
        ));
    }
    let resolved_operational_config =
        operational_config::resolve_for_call(ports, spec, args, &context);
    if requires_operational_config && cancellation.is_cancelled() {
        return Err(unica_work_control::cancellation::cancelled_error(
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
                work: None,
            });
        }
    };
    // Движок доставляется до того, как обработчик пойдёт его искать: отказ по
    // недоставленному движку вызывающий починить сам не может, а ожидание — это
    // работа, которая ещё едет. Отказ самой доставки вызов не отменяет:
    // обработчик скажет про отсутствующий движок ровно то же, что говорил до
    // сих пор. Предпросмотр ничего не исполняет и потому ничего не качает.
    if !dry_run {
        match ports.deliver_engine_if_missing(spec, &context, cancellation, progress) {
            unica_artifact_delivery::shared::EngineDeliveryState::NotRequired
            | unica_artifact_delivery::shared::EngineDeliveryState::Ready(_) => {}
            unica_artifact_delivery::shared::EngineDeliveryState::Working {
                artifact,
                received,
                total,
                poll_interval_ms,
            } => {
                let state = unica_work_control::long_work::WorkState {
                    status: unica_work_control::long_work::WorkStatus::Working,
                    status_message: match total {
                        Some(total) => {
                            format!("delivering {artifact}: {received} of {total} bytes on disk")
                        }
                        None => format!("delivering {artifact}: {received} bytes on disk"),
                    },
                    poll_interval_ms,
                };
                return Ok(long_work_result(spec, &context, mode, state));
            }
            unica_artifact_delivery::shared::EngineDeliveryState::Failed { artifact, failure } => {
                let state = unica_work_control::long_work::WorkState {
                    status: unica_work_control::long_work::WorkStatus::Failed,
                    status_message: format!(
                        "delivery of {artifact} failed: {}",
                        failure.legacy_diagnostic()
                    ),
                    poll_interval_ms: None,
                };
                return Ok(long_work_result(spec, &context, mode, state));
            }
        }
    }

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
                progress,
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
            ToolHandler::Diagnostics => diagnostics::invoke(
                ports,
                args,
                &context,
                operational_config.as_ref(),
                cancellation,
            )?,
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
    let handler_outcome =
        defer_oversized_typed_read(spec, args, mode, &context, deferred, handler_outcome);
    let mut outcome = handler_outcome.adapter;
    if let Some(notice) = applied_risk {
        outcome
            .warnings
            .push(format!("{}: {}", notice.code, notice.message));
    }
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
            serde_json::to_value(unica_roles::RoleEditData::failed(
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
    let diagnostics_typed = matches!(spec.handler, ToolHandler::Diagnostics);
    if role_typed || diagnostics_typed {
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
        // Обработчик отработал целиком: незаконченной работы здесь не бывает.
        work: None,
    })
}

/// Ответ вызова, чья работа не кончилась внутри него.
///
/// Идущая работа несёт `ok`: это состояние, а не неудача, и в задачах протокола
/// оно называется так же. Неудача остаётся неудачей и называет причину.
/// Обработчик при этом не звался, поэтому ни вывода, ни команды здесь нет.
fn supports_deferred_delivery(spec: &ToolSpec) -> bool {
    matches!(spec.handler, ToolHandler::NativeOperation { operation, .. } if deferred_delivery::supports_operation(operation))
}

fn long_work_result(
    spec: ToolSpec,
    context: &WorkspaceContext,
    mode: InvocationMode,
    state: unica_work_control::long_work::WorkState,
) -> OperationResult {
    use unica_work_control::long_work::WorkStatus;
    let ok = matches!(state.status, WorkStatus::Working | WorkStatus::Completed);
    OperationResult {
        ok,
        summary: format!("{}: {}", spec.name, state.status_message),
        changes: Vec::new(),
        warnings: Vec::new(),
        errors: if ok {
            Vec::new()
        } else {
            vec![state.status_message.clone()]
        },
        artifacts: Vec::new(),
        cache: CacheReport {
            mode: match mode {
                InvocationMode::Read => "read",
                InvocationMode::Preview => "preview",
                InvocationMode::Apply => "apply",
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
        },
        stdout: None,
        stderr: None,
        command: None,
        diagnostics: None,
        data: None,
        job: None,
        work: Some(state),
    }
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

fn deferred_cache_report(root: String, workspace_epoch: u64) -> CacheReport {
    CacheReport {
        mode: "read".to_string(),
        root,
        workspace_epoch,
        events: Vec::new(),
        invalidated: Vec::new(),
        refreshed: Vec::new(),
        lazy_rebuilt: Vec::new(),
        stale: Vec::new(),
        fresh: Vec::new(),
        publication_warnings: Vec::new(),
    }
}

fn deferred_failure_result(spec: ToolSpec, code: &str, message: &str) -> OperationResult {
    OperationResult {
        ok: false,
        summary: format!("{} continuation refused", spec.name),
        changes: Vec::new(),
        warnings: Vec::new(),
        errors: vec![format!("{code}: {message}")],
        artifacts: Vec::new(),
        cache: deferred_cache_report(String::new(), 0),
        stdout: None,
        stderr: None,
        command: None,
        diagnostics: None,
        data: None,
        job: None,
        work: None,
    }
}

/// serves a continuation call from the stored snapshot. `None`
/// means the call is an ordinary read and proceeds to the reader.
fn try_deferred_continuation(
    spec: ToolSpec,
    args: &Map<String, Value>,
    deferred: &deferred_delivery::DeferredDelivery,
) -> Option<OperationResult> {
    let selection = deferred_delivery::Selection::from_args(args);
    let result_ref = args.get("resultRef").and_then(Value::as_str);
    let Some(result_ref) = result_ref else {
        if selection.requests_anything() {
            return Some(deferred_failure_result(
                spec,
                "continuation_requires_result_ref",
                "section, filter, page and delivery are continuation selectors; pass the resultRef issued by a deferred manifest",
            ));
        }
        return None;
    };
    let identity = deferred_delivery::args_identity(args);
    let view = match deferred.store.read(result_ref, spec.name, &identity) {
        Ok(view) => view,
        Err(error) => {
            return Some(deferred_failure_result(
                spec,
                error.code(),
                &format!("continuation reference {result_ref} was not honored"),
            ));
        }
    };
    match deferred_delivery::slice(&view, result_ref, &selection) {
        Ok(data) => Some(OperationResult {
            ok: true,
            summary: format!(
                "{}: continuation served from the stored snapshot",
                spec.name
            ),
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: Vec::new(),
            artifacts: Vec::new(),
            cache: deferred_cache_report(
                view.snapshot.cache_root.clone(),
                view.snapshot.workspace_epoch,
            ),
            stdout: None,
            stderr: None,
            command: None,
            diagnostics: None,
            data: Some(data),
            job: None,
            work: None,
        }),
        Err(error) => Some(deferred_failure_result(spec, error.code, &error.message)),
    }
}

/// an oversized successful typed read is published as a deferred
/// manifest while the full snapshot goes to the bounded store.
fn defer_oversized_typed_read(
    spec: ToolSpec,
    args: &Map<String, Value>,
    mode: InvocationMode,
    context: &WorkspaceContext,
    deferred: &deferred_delivery::DeferredDelivery,
    mut handler_outcome: ports::HandlerOutcome,
) -> ports::HandlerOutcome {
    if mode != InvocationMode::Read
        || !supports_deferred_delivery(&spec)
        || !handler_outcome.adapter.ok
    {
        return handler_outcome;
    }
    let Some(data) = handler_outcome.data.take() else {
        return handler_outcome;
    };
    let bytes = serde_json::to_vec(&data)
        .map(|body| body.len())
        .unwrap_or(usize::MAX);
    if bytes <= deferred.threshold_bytes {
        handler_outcome.data = Some(data);
        return handler_outcome;
    }
    let snapshot = unica_navigation::result_store::SnapshotIdentity {
        workspace_epoch: context.workspace_epoch,
        cache_root: context.cache_root.display().to_string(),
        as_of_unix_ms: deferred_delivery::now_unix_ms(),
    };
    let identity = deferred_delivery::args_identity(args);
    match deferred
        .store
        .insert(spec.name, &identity, snapshot.clone(), data.clone(), bytes)
    {
        Some(reference) => {
            let expires_at = deferred_delivery::now_unix_ms()
                .saturating_add(deferred.store.ttl().as_millis() as u64);
            handler_outcome.data = Some(deferred_delivery::manifest(
                &handler_outcome.adapter.summary,
                &data,
                bytes,
                &reference,
                &snapshot,
                expires_at,
            ));
        }
        None => {
            // The store refused an oversized single result: deliver inline
            // rather than promise a continuation that cannot be honored.
            handler_outcome.data = Some(data);
        }
    }
    handler_outcome
}

#[doc(hidden)]
pub fn runtime_receipt_admission_result(
    spec: ToolSpec,
    failure: runtime_admission::RuntimeAdmissionFailure,
    missing_engine: Option<unica_artifact_delivery::engine::MissingEngine>,
) -> OperationResult {
    let mut errors = vec![format!("{}: {}", failure.code, failure.message)];
    if let Some(missing) = &missing_engine {
        errors.push(missing.message());
    }
    OperationResult {
        ok: false,
        summary: format!("{} refused before runtime process spawn", spec.name),
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
        diagnostics: missing_engine.map(|missing| json!({"missingEngine": missing})),
        data: None,
        job: None,
        work: None,
    }
}

fn invalid_metadata_arguments_result(
    failure: unica_metadata::request::MetaFailure,
) -> OperationResult {
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
        work: None,
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
        let data =
            unica_roles::RoleEditData::failed(self.metadata_path.clone(), code, reason, None);
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
            work: None,
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
        if spec.name != "unica.xdto.info" {
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
    operational_config: &unica_configuration::operational::OperationalConfig,
    cancellation: &CancellationToken,
    progress: &dyn ProgressSink,
) -> Result<ports::HandlerOutcome, String> {
    let (context, _scope) = ports.resolve_code_search_context(workspace, args)?;
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
    .search_observed(&request, &context, cancellation, progress)?;
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
    operational_config: &'a unica_configuration::operational::OperationalConfig,
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
    let provider_identity = provider.identity();
    let mut outcome = code_intelligence::execute_provider_read(
        provider,
        request,
        context,
        operational_config
            .code_intelligence()
            .provider_read_timeout(),
        cancellation,
    )?;
    if outcome.provider != provider_identity {
        outcome.warnings.insert(
            0,
            format!(
                "provider registry selected {}, but the response identified {}",
                provider_identity.provider, outcome.provider.provider
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
            let message = "asynchronous applied full dump is not supported yet because the background job boundary cannot return the private staged tree to Unica for the required platform 8.3.27 and exact export format 2.20 validation before publication; no applied runtime route is an admitted fallback, so inspect the command with dryRun=true and wait for a verified terminal-receipt contract".to_string();
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

#[doc(hidden)]
pub fn should_emit_events(
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
        _ => Vec::new(),
    }
}

fn runtime_event(args: &Map<String, Value>) -> Option<DomainEventKind> {
    args.get("operation")
        .and_then(Value::as_str)
        .and_then(runtime_event_kind)
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
        // `meta.info` остаётся до отдельного среза: 61 тест его набора
        // доказывает движок типизированного чтения, который зовёт и
        // канонический `view`. Снятие имени требует переноса этих тестов на
        // движок напрямую, и это отдельная работа.
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
                operation: unica_metadata::request::MetadataOperation::Info,
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
                operation: unica_metadata::request::MetadataOperation::Add,
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
                operation: unica_metadata::request::MetadataOperation::Edit,
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
        // Чтение макета остаётся за v0.12: канонический `view` на узле
        // `Template` отдаёт только адрес и заголовок, содержимого
        // табличного документа у него пока нет.
        // Канонический `docs` отдаёт `documentId` и сниппет, но не текст
        // страницы. Пока выборки документа по локатору у него нет, ответ
        // нечем доказать, поэтому этот вход остаётся.
        ToolSpec {
            name: "unica.documentation.get",
            description:
                "Fetch the full text of a documentation search hit by its documentId locator.",
            execution: ToolExecution::Read,
            result_contract: ResultContract::Typed,
            cache_access: CacheAccess::default(),
            handler: ToolHandler::Documentation { operation: "get" },
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
mod execution_catalog_tests;
