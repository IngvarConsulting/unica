use crate::application::source_navigation::LocateRejection;
use crate::domain::cancellation::CancellationToken;
use crate::domain::diagnostics::{
    DiagnosticAction, DiagnosticContext, DiagnosticError, DiagnosticFocus, DiagnosticFocusKind,
    DiagnosticItem, DiagnosticMapError, DiagnosticObservation, DiagnosticObservationFocus,
    DiagnosticObservationLocation, DiagnosticProvider, DiagnosticProviderDescriptor,
    DiagnosticProviderOutcome, DiagnosticProviderRequest, DiagnosticProviderStatus,
    DiagnosticReadiness, DiagnosticReadinessState, DiagnosticRequest, DiagnosticRequestError,
    DiagnosticRuleObservation, DiagnosticSeverity, DiagnosticTag, MetadataFocus, ProviderDeadline,
    UnaddressableReason, BSL_ANALYZER_PROVIDER,
};
use crate::domain::metadata::{
    diagnostic_metadata_focus_route, diagnostic_metadata_property_is_canonical,
};
use crate::domain::project_sources::{SourceFormat, SourceSetKind};
use crate::domain::source_location::SourceLocation;
use crate::domain::source_roots::ResolvedSourceRoot;
use crate::domain::source_target::{
    ResolvedTarget, SourceTarget, SourceTargetErrorCode, TargetKind,
};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::bundled_tools::bundled_tool_version;
use crate::infrastructure::diagnostics_jsonl::AnalyzerDiagnosticsBatch;
use crate::infrastructure::internal_adapters::BslAnalyzerMcpAdapter;
use crate::infrastructure::platform_xml_source_targets::{
    locate_platform_xml_source_path_in, platform_xml_resource_evidence, portable_relative,
    resolve_platform_xml_read_target_in, resolve_platform_xml_target_in_diagnostic_context,
    source_set_relative_path, TargetKindPolicy,
};
use crate::infrastructure::plugin_runtime::find_plugin_root;
use crate::infrastructure::redaction::redactor;
use crate::infrastructure::source_roots::{resolve_named_source_set, NamedSourceSetErrorKind};
use crate::infrastructure::workspace_services::{WorkspaceServiceBslCall, WorkspaceServiceManager};
use roxmltree::Node;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use url::Url;

const MAX_METADATA_FOCUS_DESCRIPTOR_BYTES: u64 = 8 * 1024 * 1024;
const BSL_RESIDENT_FINDINGS_CAP: usize = 5_000;

static BSL_ANALYZER_DIAGNOSTIC_DESCRIPTOR: DiagnosticProviderDescriptor =
    DiagnosticProviderDescriptor {
        id: BSL_ANALYZER_PROVIDER,
        actions: &[
            DiagnosticAction::Analyze,
            DiagnosticAction::Findings,
            DiagnosticAction::Status,
            DiagnosticAction::Catalog,
        ],
        findings_target_kinds: &[TargetKind::Module],
        emits_focus_kinds: &[DiagnosticFocusKind::SourceRange],
    };

pub(crate) fn resolve_diagnostic_context(
    request: &DiagnosticRequest,
    workspace: &WorkspaceContext,
    cancellation: &CancellationToken,
) -> Result<DiagnosticContext, DiagnosticRequestError> {
    if cancellation.is_cancelled() {
        return Err(request_error(
            "cancelled",
            None,
            "diagnostics context resolution was cancelled",
        ));
    }
    let selected = resolve_named_source_set(workspace, &request.source_set).map_err(|error| {
        let (code, message) = match error.kind {
            NamedSourceSetErrorKind::NotFound => (
                "source_set_not_found",
                format!("sourceSet `{}` was not found", request.source_set),
            ),
            NamedSourceSetErrorKind::Ambiguous => (
                "source_set_ambiguous",
                format!("sourceSet `{}` is ambiguous", request.source_set),
            ),
            NamedSourceSetErrorKind::Containment => (
                "source_set_containment_denied",
                format!(
                    "sourceSet `{}` violates the workspace containment boundary",
                    request.source_set
                ),
            ),
            NamedSourceSetErrorKind::Discovery => (
                "source_set_discovery_failed",
                format!("sourceSet `{}` could not be discovered", request.source_set),
            ),
        };
        request_error(code, Some("sourceSet"), message)
    })?;
    let source_target = SourceTarget {
        source_set: selected.source_set.name.clone(),
        metadata_path: request.metadata_path.clone(),
    };
    // Whole-set diagnostics consume the selected directory as analyzer scope,
    // not as a readable metadata resource. External source sets deliberately
    // have no synthetic root resource: their resources start with the EPF/ERF
    // descriptors inside the directory. Keep that resource boundary closed
    // while still allowing actions whose contract addresses the whole set.
    let resolved_target = if request.metadata_path.is_none()
        && matches!(
            request.action,
            DiagnosticAction::Analyze | DiagnosticAction::Status | DiagnosticAction::Catalog
        )
        && matches!(
            selected.source_set.kind,
            SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport
        )
        && selected.source_set.source_format == SourceFormat::PlatformXml
    {
        ResolvedTarget {
            source_set: selected.source_set.name.clone(),
            metadata_path: None,
            target_kind: TargetKind::SourceRoot,
        }
    } else {
        resolve_platform_xml_read_target_in(
            workspace,
            &source_target,
            TargetKindPolicy::Any,
            selected.clone(),
        )
        .map_err(|error| source_target_request_error(error.code))?
        .resolved
    };
    Ok(DiagnosticContext::new(
        workspace.clone(),
        selected.source_set.clone(),
        ResolvedSourceRoot {
            source_set: Some(selected.source_set.name),
            path: selected.path,
        },
        resolved_target,
    ))
}

#[derive(Debug, Clone, PartialEq)]
enum BslDiagnosticBackendRequest {
    Analyze {
        source_root: PathBuf,
    },
    Resident {
        source_root: PathBuf,
        tool_name: &'static str,
        arguments: Value,
    },
}

impl BslDiagnosticBackendRequest {
    #[cfg(test)]
    fn arguments(&self) -> &Value {
        match self {
            Self::Resident { arguments, .. } => arguments,
            Self::Analyze { .. } => &Value::Null,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct BslDiagnosticResidentReply {
    result_text: String,
    stderr: String,
    version: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
enum BslDiagnosticBackendReply {
    Analyze(AnalyzerDiagnosticsBatch),
    Resident(BslDiagnosticResidentReply),
}

trait BslDiagnosticBackend: Send + Sync {
    fn invoke(
        &self,
        context: &DiagnosticContext,
        request: BslDiagnosticBackendRequest,
        timeout: Duration,
        cancellation: &CancellationToken,
    ) -> Result<BslDiagnosticBackendReply, String>;
}

struct WorkspaceBslDiagnosticBackend;

impl BslDiagnosticBackend for WorkspaceBslDiagnosticBackend {
    fn invoke(
        &self,
        context: &DiagnosticContext,
        request: BslDiagnosticBackendRequest,
        timeout: Duration,
        cancellation: &CancellationToken,
    ) -> Result<BslDiagnosticBackendReply, String> {
        match request {
            BslDiagnosticBackendRequest::Analyze { source_root } => BslAnalyzerMcpAdapter::new()
                .analyze_diagnostic_batch(&context.workspace, &source_root, timeout, cancellation)
                .map(BslDiagnosticBackendReply::Analyze),
            BslDiagnosticBackendRequest::Resident {
                source_root,
                tool_name,
                arguments,
            } => WorkspaceServiceManager::new()
                .call_bsl_mcp_cancellable_with_budget(
                    &context.workspace,
                    &source_root,
                    WorkspaceServiceBslCall::new(tool_name, arguments, timeout, timeout),
                    cancellation,
                )
                .map(|output| {
                    let version = find_plugin_root(&context.workspace.cwd)
                        .and_then(|root| bundled_tool_version(&root, "bsl-analyzer").ok());
                    BslDiagnosticBackendReply::Resident(BslDiagnosticResidentReply {
                        result_text: output.result_text,
                        stderr: output.stderr,
                        version,
                    })
                }),
        }
    }
}

static WORKSPACE_BSL_DIAGNOSTIC_BACKEND: WorkspaceBslDiagnosticBackend =
    WorkspaceBslDiagnosticBackend;

pub(crate) struct BslAnalyzerDiagnosticProvider<'a> {
    backend: &'a (dyn BslDiagnosticBackend + Send + Sync),
}

impl BslAnalyzerDiagnosticProvider<'static> {
    pub(crate) fn new() -> Self {
        Self {
            backend: &WORKSPACE_BSL_DIAGNOSTIC_BACKEND,
        }
    }
}

impl<'a> BslAnalyzerDiagnosticProvider<'a> {
    #[cfg(test)]
    fn with_backend(backend: &'a (dyn BslDiagnosticBackend + Send + Sync)) -> Self {
        Self { backend }
    }

    fn execute_inner(
        &self,
        request: &DiagnosticProviderRequest,
        context: &DiagnosticContext,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<DiagnosticProviderOutcome, String> {
        if cancellation.is_cancelled() {
            return Err("cancelled: diagnostics provider stopped before request".to_string());
        }
        let timeout = deadline.remaining();
        if timeout.is_zero() {
            return Ok(provider_failed(
                None,
                "provider_timeout",
                "bsl-analyzer provider deadline exceeded",
                true,
            ));
        }
        let mut findings_module = None;
        let backend_request = match request.action {
            DiagnosticAction::Analyze => BslDiagnosticBackendRequest::Analyze {
                source_root: context.source_root.path.clone(),
            },
            DiagnosticAction::Findings => {
                let module = findings_module_path(request, context)?;
                let arguments = findings_arguments(&module);
                findings_module = Some(module);
                BslDiagnosticBackendRequest::Resident {
                    source_root: context.source_root.path.clone(),
                    tool_name: "diagnostics",
                    arguments,
                }
            }
            DiagnosticAction::Status => BslDiagnosticBackendRequest::Resident {
                source_root: context.source_root.path.clone(),
                tool_name: "diagnostics",
                arguments: json!({"action": "status"}),
            },
            DiagnosticAction::Catalog => BslDiagnosticBackendRequest::Resident {
                source_root: context.source_root.path.clone(),
                tool_name: "diagnostics",
                arguments: json!({"action": "catalog", "locale": "en"}),
            },
        };
        let reply = self
            .backend
            .invoke(context, backend_request, timeout, cancellation)?;
        match (request.action, reply) {
            (DiagnosticAction::Analyze, BslDiagnosticBackendReply::Analyze(batch)) => {
                Ok(batch.outcome)
            }
            (DiagnosticAction::Findings, BslDiagnosticBackendReply::Resident(reply)) => {
                let version = reply.version.clone();
                let module =
                    findings_module.expect("findings action proves a module before invoke");
                Ok(
                    parse_resident_findings(reply, module).unwrap_or_else(|error| {
                        provider_failed(version, "provider_reply_invalid", error, false)
                    }),
                )
            }
            (DiagnosticAction::Status, BslDiagnosticBackendReply::Resident(reply)) => {
                let version = reply.version.clone();
                Ok(parse_resident_status(reply).unwrap_or_else(|error| {
                    provider_failed(version, "provider_reply_invalid", error, false)
                }))
            }
            (DiagnosticAction::Catalog, BslDiagnosticBackendReply::Resident(reply)) => {
                let version = reply.version.clone();
                Ok(parse_resident_catalog(reply).unwrap_or_else(|error| {
                    provider_failed(version, "provider_reply_invalid", error, false)
                }))
            }
            _ => Ok(provider_failed(
                None,
                "provider_contract_invalid",
                "bsl-analyzer backend returned a reply for another action",
                false,
            )),
        }
    }
}

impl DiagnosticProvider for BslAnalyzerDiagnosticProvider<'_> {
    fn descriptor(&self) -> &'static DiagnosticProviderDescriptor {
        &BSL_ANALYZER_DIAGNOSTIC_DESCRIPTOR
    }

    fn execute(
        &self,
        request: &DiagnosticProviderRequest,
        context: &DiagnosticContext,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> DiagnosticProviderOutcome {
        match self.execute_inner(request, context, deadline, cancellation) {
            Ok(outcome) => outcome,
            Err(error) if error.starts_with("cancelled:") => provider_failed(
                None,
                "cancelled",
                "bsl-analyzer diagnostics request was cancelled",
                false,
            ),
            Err(_) => provider_failed(
                None,
                "provider_unavailable",
                "bsl-analyzer diagnostics provider is unavailable",
                true,
            ),
        }
    }
}

fn findings_arguments(module_path: &Path) -> Value {
    json!({
        "action": "file",
        "path": module_path,
        // These are fixed widening hints, not public filters. The coordinator
        // owns the authoritative filter and limit after logical mapping.
        "min_severity": "hint",
        "max_findings": BSL_RESIDENT_FINDINGS_CAP,
    })
}

fn findings_module_path(
    request: &DiagnosticProviderRequest,
    context: &DiagnosticContext,
) -> Result<PathBuf, String> {
    let metadata_path = request
        .metadata_path
        .clone()
        .ok_or_else(|| "findings request requires metadataPath".to_string())?;
    let target = SourceTarget {
        source_set: request.source_set.clone(),
        metadata_path: Some(metadata_path),
    };
    let resolution = resolve_platform_xml_target_in_diagnostic_context(
        &context.workspace,
        &target,
        TargetKindPolicy::ModuleOnly,
        &context.source_set,
        &context.source_root.path,
    )
    .map_err(|error| format!("could not prove findings module: {:?}", error.code))?;
    platform_xml_resource_evidence(&context.workspace, &resolution.handle)
        .map(|evidence| evidence.target_path)
        .map_err(|error| format!("could not prove findings module resource: {error}"))
}

#[derive(Debug, Deserialize)]
struct ResidentFindingsEnvelope {
    #[serde(default)]
    stale: bool,
    #[serde(default)]
    reload: String,
    result: ResidentFindingsResult,
}

#[derive(Debug, Deserialize)]
struct ResidentFindingsResult {
    #[serde(default)]
    truncated: bool,
    #[serde(default)]
    findings: Vec<ResidentFinding>,
    error: Option<String>,
    detail: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ResidentFinding {
    code: String,
    severity: String,
    message: String,
    range: ResidentRange,
    #[serde(default)]
    tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ResidentRange {
    start_line: usize,
    start_column: usize,
    end_line: usize,
    end_column: usize,
}

fn parse_resident_findings(
    reply: BslDiagnosticResidentReply,
    module: PathBuf,
) -> Result<DiagnosticProviderOutcome, String> {
    let value: Value = serde_json::from_str(&reply.result_text).map_err(|_| {
        "bsl-analyzer diagnostics reply did not match the typed protocol".to_string()
    })?;
    // A findings reply can carry thousands of entries; probing two optional
    // fields must not deep-copy the tree it arrived in.
    let field = |name: &str| value.get(name).and_then(Value::as_str);
    if field("status") == Some("loading") || field("state") == Some("loading") {
        let detail = field("detail")
            .map(str::to_string)
            .unwrap_or_else(|| "diagnostics database is building".to_string());
        return Ok(provider_not_ready(reply.version, detail));
    }
    let envelope: ResidentFindingsEnvelope = serde_json::from_value(value)
        .map_err(|_| "bsl-analyzer findings reply did not match the typed protocol".to_string())?;
    if envelope.stale || matches!(envelope.reload.as_str(), "running" | "failed") {
        return Ok(provider_not_ready(
            reply.version,
            "diagnostic findings are stale while the provider reloads",
        ));
    }
    let handle = module.display().to_string();
    if let Some(code) = envelope.result.error {
        return Ok(DiagnosticProviderOutcome {
            status: DiagnosticProviderStatus::Completed,
            complete: false,
            version: reply.version,
            observations: vec![DiagnosticObservation::ResourceFailure {
                provider: BSL_ANALYZER_PROVIDER,
                location: DiagnosticObservationLocation::Resource { handle },
                error: DiagnosticError {
                    code: "source_analysis_failed".to_string(),
                    message: redactor(
                        &envelope
                            .result
                            .detail
                            .unwrap_or_else(|| format!("bsl-analyzer resource error: {code}")),
                    ),
                    retryable: false,
                },
            }],
            rules: Vec::new(),
            readiness: None,
            error: None,
        });
    }
    let mut observations = Vec::with_capacity(envelope.result.findings.len());
    for finding in envelope.result.findings {
        observations.push(DiagnosticObservation::Diagnostic {
            provider: BSL_ANALYZER_PROVIDER,
            location: DiagnosticObservationLocation::Resource {
                handle: handle.clone(),
            },
            focus: DiagnosticObservationFocus::SourceRange(
                crate::domain::diagnostics::DiagnosticRange {
                    start_line: finding.range.start_line,
                    start_column: finding.range.start_column,
                    end_line: finding.range.end_line,
                    end_column: finding.range.end_column,
                },
            ),
            code: finding.code,
            severity: parse_common_severity(&finding.severity)?,
            message: finding.message,
            tags: parse_common_tags(&finding.tags),
        });
    }
    Ok(DiagnosticProviderOutcome {
        status: if observations.is_empty() && !envelope.result.truncated {
            DiagnosticProviderStatus::Empty
        } else {
            DiagnosticProviderStatus::Completed
        },
        complete: !envelope.result.truncated,
        version: reply.version,
        observations,
        rules: Vec::new(),
        readiness: None,
        error: None,
    })
}

#[derive(Debug, Deserialize)]
struct ResidentStatusReply {
    state: String,
    #[serde(default)]
    reload: String,
    error: Option<String>,
}

fn parse_resident_status(
    reply: BslDiagnosticResidentReply,
) -> Result<DiagnosticProviderOutcome, String> {
    let status: ResidentStatusReply = serde_json::from_str(&reply.result_text)
        .map_err(|_| "bsl-analyzer status reply did not match the typed protocol".to_string())?;
    if status.state == "failed" {
        return Ok(provider_failed(
            reply.version,
            "provider_not_ready",
            status
                .error
                .unwrap_or_else(|| "diagnostics database failed to build".to_string()),
            true,
        ));
    }
    let state = match status.state.as_str() {
        "disabled" | "idle" => DiagnosticReadinessState::NotStarted,
        "loading" => DiagnosticReadinessState::Building,
        "ready" if matches!(status.reload.as_str(), "running" | "failed") => {
            DiagnosticReadinessState::Stale
        }
        "ready" => DiagnosticReadinessState::Ready,
        _ => {
            return Ok(provider_failed(
                reply.version,
                "provider_reply_invalid",
                "bsl-analyzer returned an unknown diagnostics readiness state",
                false,
            ));
        }
    };
    Ok(DiagnosticProviderOutcome {
        status: DiagnosticProviderStatus::Completed,
        complete: true,
        version: reply.version,
        observations: Vec::new(),
        rules: Vec::new(),
        readiness: Some(DiagnosticReadiness {
            state,
            retryable: matches!(
                state,
                DiagnosticReadinessState::NotStarted | DiagnosticReadinessState::Building
            ),
        }),
        error: None,
    })
}

#[derive(Debug, Deserialize)]
struct ResidentCatalogReply {
    entries: Vec<ResidentCatalogEntry>,
}

#[derive(Debug, Deserialize)]
struct ResidentCatalogEntry {
    code: String,
    title: String,
    default_severity: String,
    #[serde(default)]
    tags: Vec<String>,
}

fn parse_resident_catalog(
    reply: BslDiagnosticResidentReply,
) -> Result<DiagnosticProviderOutcome, String> {
    let catalog: ResidentCatalogReply = serde_json::from_str(&reply.result_text)
        .map_err(|_| "bsl-analyzer catalog reply did not match the typed protocol".to_string())?;
    let mut rules = Vec::with_capacity(catalog.entries.len());
    for entry in catalog.entries {
        if entry.code.trim().is_empty() || entry.title.trim().is_empty() {
            return Ok(provider_failed(
                reply.version,
                "provider_reply_invalid",
                "bsl-analyzer catalog contains an empty code or title",
                false,
            ));
        }
        rules.push(DiagnosticRuleObservation {
            provider: BSL_ANALYZER_PROVIDER,
            code: entry.code,
            default_severity: parse_common_severity(&entry.default_severity)?,
            title: entry.title,
            description: None,
            tags: parse_common_tags(&entry.tags),
        });
    }
    Ok(DiagnosticProviderOutcome {
        status: DiagnosticProviderStatus::Completed,
        complete: true,
        version: reply.version,
        observations: Vec::new(),
        rules,
        readiness: None,
        error: None,
    })
}

fn parse_common_severity(value: &str) -> Result<DiagnosticSeverity, String> {
    match value {
        "error" | "Blocker" | "Critical" | "Major" | "Error" => Ok(DiagnosticSeverity::Error),
        "warning" | "Warning" => Ok(DiagnosticSeverity::Warning),
        "info" | "Information" => Ok(DiagnosticSeverity::Info),
        "hint" | "Hint" => Ok(DiagnosticSeverity::Hint),
        _ => Err("bsl-analyzer returned an unknown diagnostic severity".to_string()),
    }
}

fn parse_common_tags(tags: &[String]) -> Vec<DiagnosticTag> {
    let mut mapped = Vec::new();
    for tag in tags {
        let tag = match tag.as_str() {
            "Unnecessary" | "unnecessary" | "unused" => Some(DiagnosticTag::Unnecessary),
            "Deprecated" | "deprecated" => Some(DiagnosticTag::Deprecated),
            _ => None,
        };
        if let Some(tag) = tag.filter(|tag| !mapped.contains(tag)) {
            mapped.push(tag);
        }
    }
    mapped
}

fn provider_not_ready(
    version: Option<String>,
    message: impl Into<String>,
) -> DiagnosticProviderOutcome {
    provider_failed_with_status(
        DiagnosticProviderStatus::Unavailable,
        version,
        "provider_not_ready",
        message,
        true,
    )
}

fn provider_failed(
    version: Option<String>,
    code: &str,
    message: impl Into<String>,
    retryable: bool,
) -> DiagnosticProviderOutcome {
    provider_failed_with_status(
        DiagnosticProviderStatus::Failed,
        version,
        code,
        message,
        retryable,
    )
}

fn provider_failed_with_status(
    status: DiagnosticProviderStatus,
    version: Option<String>,
    code: &str,
    message: impl Into<String>,
    retryable: bool,
) -> DiagnosticProviderOutcome {
    DiagnosticProviderOutcome {
        status,
        complete: false,
        version,
        observations: Vec::new(),
        rules: Vec::new(),
        readiness: None,
        error: Some(DiagnosticError {
            code: code.to_string(),
            message: message.into(),
            retryable,
        }),
    }
}

#[cfg(test)]
mod bsl_diagnostics_provider_tests {
    use super::*;
    use crate::domain::diagnostics::{
        DiagnosticAction, DiagnosticFilter, DiagnosticObservation, DiagnosticProvider,
        DiagnosticProviderOutcome, DiagnosticProviderRequest, DiagnosticProviderStatus,
        DiagnosticReadinessState, DiagnosticSeverity, DiagnosticTag, ProviderDeadline,
        BSL_ANALYZER_PROVIDER,
    };
    use crate::domain::project_sources::{ProjectSourceSet, SourceFormat, SourceSetKind};
    use crate::domain::source_roots::ResolvedSourceRoot;
    use crate::domain::source_target::{
        MetadataAddress, ResolvedTarget, TargetKind, PLATFORM_XML_8_3_27_FORMAT_2_20,
    };
    use crate::infrastructure::diagnostics_jsonl::{
        AnalyzerDiagnosticsBatch, AnalyzerDiagnosticsFileTotals,
    };
    use serde_json::{json, Value};
    use std::collections::VecDeque;
    use std::fs;
    use std::sync::Mutex;
    use std::time::Duration;
    use tempfile::TempDir;

    struct ProviderFixture {
        _temp: TempDir,
        context: DiagnosticContext,
        module: PathBuf,
    }

    impl ProviderFixture {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            // macOS exposes tempfile paths through `/var` while canonical file
            // identities use `/private/var`. Keep the hand-built context on
            // the same normalized boundary as production application ports.
            let root =
                crate::infrastructure::source_roots::normalize_path_identity(temp.path()).unwrap();
            let source = root.join("src");
            let module = source.join("CommonModules/Smoke/Ext/Module.bsl");
            fs::create_dir_all(module.parent().unwrap()).unwrap();
            fs::write(
                source.join("Configuration.xml"),
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Fixture</Name></Properties><ChildObjects><CommonModule>Smoke</CommonModule></ChildObjects></Configuration></MetaDataObject>"#,
            )
            .unwrap();
            fs::create_dir_all(source.join("CommonModules")).unwrap();
            fs::write(
                source.join("CommonModules/Smoke.xml"),
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><CommonModule><Properties><Name>Smoke</Name></Properties></CommonModule></MetaDataObject>"#,
            )
            .unwrap();
            fs::write(&module, "Procedure Probe()\nEndProcedure").unwrap();
            let module =
                crate::infrastructure::source_roots::normalize_path_identity(&module).unwrap();
            let metadata_path = MetadataAddress::parse(
                PLATFORM_XML_8_3_27_FORMAT_2_20,
                "CommonModule.Smoke.Module",
            )
            .unwrap();
            let workspace = WorkspaceContext {
                cwd: root.clone(),
                workspace_root: root.clone(),
                cache_root: root.join(".build/unica"),
                workspace_epoch: 7,
            };
            let source_set = ProjectSourceSet {
                name: "main".to_string(),
                kind: SourceSetKind::Configuration,
                path: "src".to_string(),
                source_format: SourceFormat::PlatformXml,
                format_evidence: Vec::new(),
                format_probe_error: None,
            };
            Self {
                context: DiagnosticContext::new(
                    workspace,
                    source_set,
                    ResolvedSourceRoot {
                        source_set: Some("main".to_string()),
                        path: source,
                    },
                    ResolvedTarget {
                        source_set: "main".to_string(),
                        metadata_path: Some(metadata_path),
                        target_kind: TargetKind::Module,
                    },
                ),
                module,
                _temp: temp,
            }
        }

        fn request(&self, action: DiagnosticAction) -> DiagnosticProviderRequest {
            DiagnosticProviderRequest {
                action,
                source_set: "main".to_string(),
                metadata_path: (action == DiagnosticAction::Findings)
                    .then(|| self.context.target.metadata_path.clone().unwrap()),
                target_kind: if action == DiagnosticAction::Findings {
                    TargetKind::Module
                } else {
                    TargetKind::SourceRoot
                },
                filter: DiagnosticFilter {
                    min_severity: Some(DiagnosticSeverity::Error),
                    codes: vec![crate::domain::diagnostics::DiagnosticCodeFilter {
                        provider: "bsl-analyzer".to_string(),
                        code: "LineLength".to_string(),
                    }],
                },
                range: Some(crate::domain::diagnostics::DiagnosticRange {
                    start_line: 2,
                    start_column: 0,
                    end_line: 8,
                    end_column: 0,
                }),
            }
        }
    }

    struct FakeBackend {
        calls: Mutex<Vec<BslDiagnosticBackendRequest>>,
        replies: Mutex<VecDeque<Result<BslDiagnosticBackendReply, String>>>,
    }

    impl FakeBackend {
        fn new(replies: Vec<BslDiagnosticBackendReply>) -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                replies: Mutex::new(replies.into_iter().map(Ok).collect()),
            }
        }

        fn resident(text: Value) -> BslDiagnosticBackendReply {
            BslDiagnosticBackendReply::Resident(BslDiagnosticResidentReply {
                result_text: text.to_string(),
                stderr: String::new(),
                version: Some("0.2.62".to_string()),
            })
        }
    }

    impl BslDiagnosticBackend for FakeBackend {
        fn invoke(
            &self,
            _context: &DiagnosticContext,
            request: BslDiagnosticBackendRequest,
            _timeout: Duration,
            _cancellation: &CancellationToken,
        ) -> Result<BslDiagnosticBackendReply, String> {
            self.calls.lock().unwrap().push(request);
            self.replies
                .lock()
                .unwrap()
                .pop_front()
                .expect("fake reply")
        }
    }

    fn empty_analyze() -> BslDiagnosticBackendReply {
        BslDiagnosticBackendReply::Analyze(AnalyzerDiagnosticsBatch {
            outcome: DiagnosticProviderOutcome {
                status: DiagnosticProviderStatus::Empty,
                complete: true,
                version: Some("0.2.62".to_string()),
                observations: Vec::new(),
                rules: Vec::new(),
                readiness: None,
                error: None,
            },
            files: AnalyzerDiagnosticsFileTotals {
                discovered: Some(0),
                processed: Some(0),
                failed: Some(0),
            },
            diagnostics_reported: Some(0),
            elapsed_seconds: Some(0.1),
        })
    }

    fn execute(
        provider: &BslAnalyzerDiagnosticProvider<'_>,
        fixture: &ProviderFixture,
        action: DiagnosticAction,
    ) -> DiagnosticProviderOutcome {
        provider.execute(
            &fixture.request(action),
            &fixture.context,
            ProviderDeadline::from_budget(Duration::from_secs(30)),
            &CancellationToken::new(),
        )
    }

    #[test]
    fn bsl_diagnostics_provider_request_maps_all_actions_without_public_filters() {
        let fixture = ProviderFixture::new();
        let backend = FakeBackend::new(vec![
            empty_analyze(),
            FakeBackend::resident(json!({
                "revision": 1,
                "stale": false,
                "reload": "none",
                "result": {"kind": "full", "truncated": false, "findings": []}
            })),
            FakeBackend::resident(json!({"state":"ready","generation":1,"reload":"none"})),
            FakeBackend::resident(json!({"action":"catalog","locale":"en","count":0,"entries":[]})),
        ]);
        let provider = BslAnalyzerDiagnosticProvider::with_backend(&backend);

        for action in [
            DiagnosticAction::Analyze,
            DiagnosticAction::Findings,
            DiagnosticAction::Status,
            DiagnosticAction::Catalog,
        ] {
            let outcome = execute(&provider, &fixture, action);
            assert!(
                matches!(
                    outcome.status,
                    DiagnosticProviderStatus::Completed | DiagnosticProviderStatus::Empty
                ),
                "{action:?}: {outcome:?}"
            );
        }

        let calls = backend.calls.lock().unwrap();
        assert_eq!(
            calls[0],
            BslDiagnosticBackendRequest::Analyze {
                source_root: fixture.context.source_root.path.clone()
            }
        );
        assert_eq!(
            calls[1],
            BslDiagnosticBackendRequest::Resident {
                source_root: fixture.context.source_root.path.clone(),
                tool_name: "diagnostics",
                arguments: json!({
                    "action": "file",
                    "path": fixture.module,
                    "min_severity": "hint",
                    "max_findings": 5000
                }),
            }
        );
        assert_eq!(
            calls[2],
            BslDiagnosticBackendRequest::Resident {
                source_root: fixture.context.source_root.path.clone(),
                tool_name: "diagnostics",
                arguments: json!({"action":"status"}),
            }
        );
        assert_eq!(
            calls[3],
            BslDiagnosticBackendRequest::Resident {
                source_root: fixture.context.source_root.path.clone(),
                tool_name: "diagnostics",
                arguments: json!({"action":"catalog","locale":"en"}),
            }
        );
        let wire = serde_json::to_string(&calls[1].arguments()).unwrap();
        for forbidden in ["LineLength", "range_start", "range_end", "limit"] {
            assert!(!wire.contains(forbidden), "forwarded public filter: {wire}");
        }
    }

    #[test]
    fn truncated_empty_resident_findings_are_incomplete_not_empty() {
        let fixture = ProviderFixture::new();
        let backend = FakeBackend::new(vec![FakeBackend::resident(json!({
            "revision": 1,
            "stale": false,
            "reload": "none",
            "result": {"kind": "full", "truncated": true, "findings": []}
        }))]);
        let provider = BslAnalyzerDiagnosticProvider::with_backend(&backend);

        let outcome = execute(&provider, &fixture, DiagnosticAction::Findings);

        assert_eq!(outcome.status, DiagnosticProviderStatus::Completed);
        assert!(!outcome.complete);
        assert!(outcome.observations.is_empty());
    }

    #[test]
    fn bsl_diagnostics_provider_maps_findings_and_loading_to_common_outcomes() {
        let fixture = ProviderFixture::new();
        let backend = FakeBackend::new(vec![
            FakeBackend::resident(json!({
                "revision": 3,
                "stale": false,
                "reload": "none",
                "result": {
                    "kind": "full",
                    "truncated": false,
                    "findings": [{
                        "code": "LineLength",
                        "severity": "warning",
                        "message": "Line too long",
                        "range": {"start_line":0,"start_column":10,"end_line":0,"end_column":18},
                        "tags": ["Unnecessary"],
                        "has_fix": false,
                        "future_field": "ignored"
                    }]
                },
                "future_envelope": true
            })),
            FakeBackend::resident(json!({
                "status":"loading","detail":"building","state":"loading","generation":4
            })),
        ]);
        let provider = BslAnalyzerDiagnosticProvider::with_backend(&backend);

        let complete = execute(&provider, &fixture, DiagnosticAction::Findings);
        assert_eq!(complete.status, DiagnosticProviderStatus::Completed);
        assert_eq!(complete.version.as_deref(), Some("0.2.62"));
        assert!(matches!(
            &complete.observations[0],
            DiagnosticObservation::Diagnostic {
                provider,
                location: DiagnosticObservationLocation::Resource { handle },
                focus: DiagnosticObservationFocus::SourceRange(range),
                code,
                severity: DiagnosticSeverity::Warning,
                tags,
                ..
            } if *provider == BSL_ANALYZER_PROVIDER
                && handle == &fixture.module.display().to_string()
                && code == "LineLength"
                && *range == crate::domain::diagnostics::DiagnosticRange {
                    start_line: 0, start_column: 10, end_line: 0, end_column: 18
                }
                && tags == &vec![DiagnosticTag::Unnecessary]
        ));

        let loading = execute(&provider, &fixture, DiagnosticAction::Findings);
        assert_eq!(loading.status, DiagnosticProviderStatus::Unavailable);
        assert!(loading.observations.is_empty());
        let error = loading.error.unwrap();
        assert_eq!(error.code, "provider_not_ready");
        assert!(error.retryable);
    }

    #[test]
    fn bsl_diagnostics_provider_maps_readiness_catalog_failures_and_resource_errors() {
        let fixture = ProviderFixture::new();
        let replies = vec![
            FakeBackend::resident(json!({"state":"idle","generation":0,"reload":"none"})),
            FakeBackend::resident(json!({"state":"loading","generation":1,"reload":"running"})),
            FakeBackend::resident(
                json!({"state":"ready","generation":2,"reload":"none","files":3}),
            ),
            FakeBackend::resident(
                json!({"state":"ready","generation":2,"reload":"running","files":3}),
            ),
            FakeBackend::resident(json!({
                "action":"catalog","locale":"en","count":1,
                "entries":[{
                    "code":"LineLength","title":"Line length","default_severity":"warning",
                    "type":"code_smell","activated_by_default":true,"tags":["unused"]
                }]
            })),
            FakeBackend::resident(json!({
                "revision": 1, "stale": false, "reload": "none",
                "result":{"error":"not_in_workspace","detail":"module is not resident"}
            })),
            BslDiagnosticBackendReply::Resident(BslDiagnosticResidentReply {
                result_text: "not-json C:\\secret\\Module.bsl".to_string(),
                stderr: "private stderr".to_string(),
                version: Some("0.2.62".to_string()),
            }),
        ];
        let backend = FakeBackend::new(replies);
        let provider = BslAnalyzerDiagnosticProvider::with_backend(&backend);

        for expected in [
            DiagnosticReadinessState::NotStarted,
            DiagnosticReadinessState::Building,
            DiagnosticReadinessState::Ready,
            DiagnosticReadinessState::Stale,
        ] {
            let status = execute(&provider, &fixture, DiagnosticAction::Status);
            assert_eq!(status.status, DiagnosticProviderStatus::Completed);
            assert_eq!(status.readiness.unwrap().state, expected);
        }

        let catalog = execute(&provider, &fixture, DiagnosticAction::Catalog);
        assert_eq!(catalog.status, DiagnosticProviderStatus::Completed);
        assert_eq!(catalog.rules.len(), 1);
        assert_eq!(catalog.rules[0].code, "LineLength");
        assert_eq!(
            catalog.rules[0].default_severity,
            DiagnosticSeverity::Warning
        );
        assert_eq!(catalog.rules[0].tags, vec![DiagnosticTag::Unnecessary]);

        let resource = execute(&provider, &fixture, DiagnosticAction::Findings);
        assert!(matches!(
            &resource.observations[0],
            DiagnosticObservation::ResourceFailure { error, .. }
                if error.code == "source_analysis_failed"
                    && error.message == "module is not resident"
        ));

        let malformed = execute(&provider, &fixture, DiagnosticAction::Findings);
        assert_eq!(malformed.status, DiagnosticProviderStatus::Failed);
        assert!(malformed.observations.is_empty());
        let error = malformed.error.unwrap();
        assert_eq!(error.code, "provider_reply_invalid");
        assert!(!error.message.contains("secret"));
        assert!(!error.message.contains("stderr"));
    }
}

pub(crate) fn map_diagnostic_observation(
    observation: DiagnosticObservation,
    context: &DiagnosticContext,
    cancellation: &CancellationToken,
) -> Result<DiagnosticItem, DiagnosticMapError> {
    map_diagnostic_observation_cached(
        observation,
        context,
        cancellation,
        &mut DiagnosticMappingCache::default(),
    )
}

/// One result per observation: a resource the mapper cannot prove is that
/// observation's own failure, not proof against the batch it arrived in.
pub(crate) fn map_diagnostic_observations(
    observations: Vec<DiagnosticObservation>,
    context: &DiagnosticContext,
    cancellation: &CancellationToken,
) -> Vec<Result<DiagnosticItem, DiagnosticMapError>> {
    let mut cache = DiagnosticMappingCache::default();
    observations
        .into_iter()
        .map(|observation| {
            map_diagnostic_observation_cached(observation, context, cancellation, &mut cache)
        })
        .collect()
}

fn map_diagnostic_observation_cached(
    observation: DiagnosticObservation,
    context: &DiagnosticContext,
    cancellation: &CancellationToken,
    cache: &mut DiagnosticMappingCache,
) -> Result<DiagnosticItem, DiagnosticMapError> {
    if cancellation.is_cancelled() {
        return Err(map_error(
            "cancelled",
            "diagnostic observation mapping was cancelled",
        ));
    }
    match observation {
        DiagnosticObservation::Diagnostic {
            provider,
            location,
            focus,
            code,
            severity,
            message,
            tags,
        } => {
            let mapped = map_location_cached(location, context, cancellation, cache)?;
            let focus = map_focus(focus, &mapped.location, context, cancellation, cache);
            Ok(DiagnosticItem::Diagnostic {
                provider: provider.as_str(),
                location: mapped.location,
                location_reason: mapped.reason,
                focus,
                code,
                severity,
                message,
                tags,
            })
        }
        DiagnosticObservation::ResourceFailure {
            provider,
            location,
            error,
        } => {
            let mapped = map_location_cached(location, context, cancellation, cache)?;
            Ok(DiagnosticItem::ResourceFailure {
                provider: provider.as_str(),
                location: mapped.location,
                location_reason: mapped.reason,
                error,
            })
        }
    }
}

#[derive(Debug, Clone)]
struct MappedDiagnosticLocation {
    location: SourceLocation,
    reason: Option<UnaddressableReason>,
}

/// Per-batch memo of everything the mapper would otherwise re-derive for every
/// observation of the same logical resource: the proven location and the object
/// descriptor a metadata focus is checked against.
#[derive(Debug, Default)]
struct DiagnosticMappingCache {
    resource_locations: HashMap<String, Result<MappedDiagnosticLocation, DiagnosticMapError>>,
    descriptors: HashMap<String, Option<String>>,
}

impl DiagnosticMappingCache {
    fn descriptor_text(
        &mut self,
        key: &str,
        load: impl FnOnce() -> Option<String>,
    ) -> Option<&str> {
        self.descriptors
            .entry(key.to_string())
            .or_insert_with(load)
            .as_deref()
    }
}

fn map_location(
    location: DiagnosticObservationLocation,
    context: &DiagnosticContext,
    cancellation: &CancellationToken,
) -> Result<MappedDiagnosticLocation, DiagnosticMapError> {
    match location {
        DiagnosticObservationLocation::Logical { metadata_path } => {
            let target = SourceTarget {
                source_set: context.target.source_set.clone(),
                metadata_path,
            };
            let resolution = resolve_platform_xml_target_in_diagnostic_context(
                &context.workspace,
                &target,
                TargetKindPolicy::Any,
                &context.source_set,
                &context.source_root.path,
            )
            .map_err(|error| map_source_target_error(error.code))?;
            Ok(MappedDiagnosticLocation {
                location: SourceLocation::Addressed {
                    source_set: resolution.resolved.source_set,
                    metadata_path: resolution.resolved.metadata_path,
                    target_kind: resolution.resolved.target_kind,
                },
                reason: None,
            })
        }
        DiagnosticObservationLocation::Resource { handle } => {
            map_resource_location(&handle, context, cancellation)
        }
    }
}

fn map_location_cached(
    location: DiagnosticObservationLocation,
    context: &DiagnosticContext,
    cancellation: &CancellationToken,
    cache: &mut DiagnosticMappingCache,
) -> Result<MappedDiagnosticLocation, DiagnosticMapError> {
    match location {
        DiagnosticObservationLocation::Resource { handle } => {
            if let Some(mapped) = cache.resource_locations.get(&handle) {
                return mapped.clone();
            }
            let mapped = map_resource_location(&handle, context, cancellation);
            cache.resource_locations.insert(handle, mapped.clone());
            mapped
        }
        logical => map_location(logical, context, cancellation),
    }
}

fn map_resource_location(
    handle: &str,
    context: &DiagnosticContext,
    cancellation: &CancellationToken,
) -> Result<MappedDiagnosticLocation, DiagnosticMapError> {
    let path = provider_resource_path(handle)?;
    let relative = source_set_relative_path(
        &context.workspace,
        &context.source_root.path,
        path.as_path(),
    )
    .ok_or_else(|| {
        map_error(
            "location_outside_source_set",
            "provider resource is outside the selected sourceSet",
        )
    })?;
    let observed_path = portable_relative(&relative);
    if context.source_set.source_format != SourceFormat::PlatformXml {
        return Ok(MappedDiagnosticLocation {
            location: SourceLocation::Unaddressable {
                source_set: context.target.source_set.clone(),
                owner_metadata_path: None,
                path: observed_path,
            },
            reason: Some(UnaddressableReason::SourceFormatUnsupported),
        });
    }
    let located = locate_platform_xml_source_path_in(
        &context.workspace,
        &context.source_set,
        &context.source_root.path,
        &path.to_string_lossy(),
        cancellation,
    )
    .map_err(|_| {
        map_error(
            "location_mapping_failed",
            "provider resource could not be mapped safely",
        )
    })?;
    match located.rejection {
        None => Ok(MappedDiagnosticLocation {
            location: SourceLocation::Addressed {
                source_set: located.source_set,
                metadata_path: located.metadata_path,
                target_kind: located.target_kind.ok_or_else(|| {
                    map_error(
                        "provider_contract_invalid",
                        "addressed provider resource has no target kind",
                    )
                })?,
            },
            reason: None,
        }),
        Some(LocateRejection::NotAddressable) => Ok(MappedDiagnosticLocation {
            location: SourceLocation::Unaddressable {
                source_set: located.source_set,
                owner_metadata_path: located.owner_metadata_path,
                path: located.relative_path,
            },
            reason: Some(UnaddressableReason::ResourceNotAddressable),
        }),
        Some(LocateRejection::OwnerUnproven) => Ok(MappedDiagnosticLocation {
            location: SourceLocation::Unaddressable {
                source_set: located.source_set,
                owner_metadata_path: located.owner_metadata_path,
                path: located.relative_path,
            },
            reason: Some(UnaddressableReason::OwnerUnproven),
        }),
        Some(LocateRejection::OutsideSourceSet) => Err(map_error(
            "location_outside_source_set",
            "provider resource is outside the selected sourceSet",
        )),
    }
}

fn provider_resource_path(handle: &str) -> Result<PathBuf, DiagnosticMapError> {
    let handle = handle.trim();
    if handle.is_empty() {
        return Err(map_error(
            "provider_contract_invalid",
            "provider resource handle is empty",
        ));
    }
    if looks_like_windows_drive_path(handle) {
        return Ok(PathBuf::from(handle.replace('\\', "/")));
    }
    if handle.starts_with("file:") || handle.contains("://") {
        let url = Url::parse(handle).map_err(|_| {
            map_error(
                "provider_contract_invalid",
                "provider resource URI is invalid",
            )
        })?;
        if url.scheme() != "file" {
            return Err(map_error(
                "resource_scheme_unsupported",
                "provider resource URI scheme is unsupported",
            ));
        }
        return url.to_file_path().map_err(|_| {
            map_error(
                "provider_contract_invalid",
                "provider file URI does not identify a local path",
            )
        });
    }
    Ok(PathBuf::from(handle.replace('\\', "/")))
}

fn looks_like_windows_drive_path(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'/' | b'\\')
}

fn map_focus(
    focus: DiagnosticObservationFocus,
    location: &SourceLocation,
    context: &DiagnosticContext,
    cancellation: &CancellationToken,
    cache: &mut DiagnosticMappingCache,
) -> DiagnosticFocus {
    match focus {
        DiagnosticObservationFocus::Target => DiagnosticFocus::Target,
        // A zero-width range is a caret, and weakening it to `target` would
        // both lose the position and make the finding answer every requested
        // range. `DiagnosticRange::intersects` owns the caret comparison.
        DiagnosticObservationFocus::SourceRange(range) => DiagnosticFocus::SourceRange { range },
        DiagnosticObservationFocus::Metadata(focus)
            if metadata_focus_is_proven(&focus, location, context, cancellation, cache) =>
        {
            focus.into()
        }
        DiagnosticObservationFocus::Metadata(_) => DiagnosticFocus::Target,
    }
}

fn metadata_focus_is_proven(
    focus: &MetadataFocus,
    location: &SourceLocation,
    context: &DiagnosticContext,
    cancellation: &CancellationToken,
    cache: &mut DiagnosticMappingCache,
) -> bool {
    if cancellation.is_cancelled() {
        return false;
    }
    let SourceLocation::Addressed {
        metadata_path: Some(metadata_path),
        target_kind: TargetKind::MetadataObject,
        ..
    } = location
    else {
        return false;
    };
    if focus
        .language
        .as_deref()
        .is_some_and(|value| value.trim().is_empty())
    {
        return false;
    }
    if focus
        .property
        .as_deref()
        .is_some_and(|property| !diagnostic_metadata_property_is_canonical(property))
    {
        return false;
    }
    let Some(route) = diagnostic_metadata_focus_route(&focus.element_path) else {
        return false;
    };
    // Every observation of the same object would otherwise re-resolve the
    // target and re-read a descriptor of up to
    // `MAX_METADATA_FOCUS_DESCRIPTOR_BYTES`.
    let Some(text) = cache.descriptor_text(metadata_path.as_str(), || {
        let target = SourceTarget {
            source_set: context.target.source_set.clone(),
            metadata_path: Some(metadata_path.clone()),
        };
        let resolution = resolve_platform_xml_target_in_diagnostic_context(
            &context.workspace,
            &target,
            TargetKindPolicy::Any,
            &context.source_set,
            &context.source_root.path,
        )
        .ok()?;
        let evidence =
            platform_xml_resource_evidence(&context.workspace, &resolution.handle).ok()?;
        let metadata = std::fs::metadata(&evidence.target_path).ok()?;
        if metadata.len() > MAX_METADATA_FOCUS_DESCRIPTOR_BYTES {
            return None;
        }
        let bytes = std::fs::read(&evidence.target_path).ok()?;
        String::from_utf8(bytes).ok()
    }) else {
        return false;
    };
    let Ok(document) = roxmltree::Document::parse(text.trim_start_matches('\u{feff}')) else {
        return false;
    };
    let Some(mut current) = document
        .root_element()
        .children()
        .find(|node| node.is_element())
    else {
        return false;
    };
    for (element, collection) in focus.element_path.iter().zip(route) {
        let Some(child_objects) = direct_child(current, "ChildObjects") else {
            return false;
        };
        let Some(found) = child_objects.children().find(|candidate| {
            candidate.is_element()
                && candidate.tag_name().name() == collection.xml_element_name()
                && direct_child(*candidate, "Properties")
                    .and_then(|properties| direct_child(properties, "Name"))
                    .and_then(|name| name.text())
                    == Some(element.name.as_str())
        }) else {
            return false;
        };
        current = found;
    }
    let Some(property) = focus.property.as_deref() else {
        return focus.language.is_none();
    };
    let Some(properties) = direct_child(current, "Properties") else {
        return false;
    };
    let Some(property_node) = direct_child(properties, property) else {
        return false;
    };
    focus.language.as_deref().is_none_or(|language| {
        property_node.descendants().any(|node| {
            node.is_element() && node.tag_name().name() == "lang" && node.text() == Some(language)
        })
    })
}

fn direct_child<'a, 'input>(node: Node<'a, 'input>, name: &str) -> Option<Node<'a, 'input>> {
    node.children()
        .find(|child| child.is_element() && child.tag_name().name() == name)
}

fn request_error(
    code: &'static str,
    field: Option<&'static str>,
    message: impl Into<String>,
) -> DiagnosticRequestError {
    DiagnosticRequestError {
        code,
        field,
        message: message.into(),
        retryable: false,
    }
}

fn source_target_request_error(code: SourceTargetErrorCode) -> DiagnosticRequestError {
    match code {
        SourceTargetErrorCode::SourceSetRequired => request_error(
            "source_set_required",
            Some("sourceSet"),
            "sourceSet must name an exact project source set",
        ),
        SourceTargetErrorCode::SourceSetNotFound => request_error(
            "source_set_not_found",
            Some("sourceSet"),
            "sourceSet was not found",
        ),
        SourceTargetErrorCode::MetadataAddressInvalid => request_error(
            "metadata_address_invalid",
            Some("metadataPath"),
            "metadataPath is not a valid logical address",
        ),
        SourceTargetErrorCode::MetadataAddressNotFound => request_error(
            "metadata_address_not_found",
            Some("metadataPath"),
            "metadataPath was not found in the selected sourceSet",
        ),
        SourceTargetErrorCode::TargetKindMismatch => request_error(
            "target_kind_mismatch",
            Some("metadataPath"),
            "metadataPath does not identify a supported diagnostic target",
        ),
        SourceTargetErrorCode::SourceRootNotAddressable
        | SourceTargetErrorCode::AddressProfileUnsupported => request_error(
            "source_format_unsupported",
            Some("sourceSet"),
            "sourceSet is outside the supported logical address profile",
        ),
        SourceTargetErrorCode::ContainmentDenied => request_error(
            "source_set_containment_denied",
            Some("sourceSet"),
            "sourceSet violates the workspace containment boundary",
        ),
    }
}

fn map_source_target_error(code: SourceTargetErrorCode) -> DiagnosticMapError {
    let request = source_target_request_error(code);
    map_error(request.code, request.message)
}

fn map_error(code: &'static str, message: impl Into<String>) -> DiagnosticMapError {
    DiagnosticMapError {
        code,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::{map_diagnostic_observation, resolve_diagnostic_context};
    use crate::domain::cancellation::CancellationToken;
    use crate::domain::diagnostics::{
        DiagnosticAction, DiagnosticCodeFilter, DiagnosticFilter, DiagnosticFocus, DiagnosticItem,
        DiagnosticObservation, DiagnosticObservationFocus, DiagnosticObservationLocation,
        DiagnosticProviderId, DiagnosticRange, DiagnosticRequest, DiagnosticSeverity,
        DiagnosticTag, MetadataElement, MetadataFocus, UnaddressableReason,
    };
    use crate::domain::project_sources::SourceFormat;
    use crate::domain::source_location::SourceLocation;
    use crate::domain::source_target::{
        MetadataAddress, TargetKind, PLATFORM_XML_8_3_27_FORMAT_2_20,
    };
    use crate::domain::workspace::WorkspaceContext;
    use std::fs;
    use std::path::Path;
    use std::time::Duration;
    use tempfile::TempDir;

    const TEST_PROVIDER: DiagnosticProviderId = DiagnosticProviderId::new_const("test-provider");

    struct Fixture {
        _temp: TempDir,
        context: WorkspaceContext,
    }

    impl Fixture {
        fn platform_xml() -> Self {
            let temp = tempfile::tempdir().unwrap();
            // Match the normalized workspace identity supplied by production
            // ports; macOS otherwise mixes `/var` and `/private/var` aliases.
            let root =
                crate::infrastructure::source_roots::normalize_path_identity(temp.path()).unwrap();
            fs::write(
                root.join("v8project.yaml"),
                "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
            )
            .unwrap();
            let source = root.join("src");
            fs::create_dir_all(&source).unwrap();
            fs::write(
                source.join("Configuration.xml"),
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Diagnostics</Name></Properties><ChildObjects><Catalog>Items</Catalog><CommonModule>Shared</CommonModule><CommonModule>Модуль с пробелом</CommonModule></ChildObjects></Configuration></MetaDataObject>"#,
            )
            .unwrap();
            Self {
                context: WorkspaceContext {
                    cwd: root.clone(),
                    workspace_root: root.clone(),
                    cache_root: root.join(".build/unica"),
                    workspace_epoch: 1,
                },
                _temp: temp,
            }
        }

        fn source(&self) -> std::path::PathBuf {
            self.context.workspace_root.join("src")
        }

        fn write_catalog(&self, name: &str) {
            let directory = self.source().join("Catalogs");
            fs::create_dir_all(&directory).unwrap();
            fs::write(
                directory.join(format!("{name}.xml")),
                format!(
                    r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog><Properties><Name>{name}</Name><Synonym><v8:item xmlns:v8="http://v8.1c.ru/8.1/data/core"><v8:lang>ru</v8:lang><v8:content>Товары</v8:content></v8:item></Synonym><InternalField>secret</InternalField></Properties><ChildObjects><Attribute><Properties><Name>Code</Name><Type>string</Type></Properties></Attribute><TabularSection><Properties><Name>Lines</Name></Properties><ChildObjects><Attribute><Properties><Name>Price</Name><Type>number</Type></Properties></Attribute></ChildObjects></TabularSection><Form>Card</Form></ChildObjects></Catalog></MetaDataObject>"#
                ),
            )
            .unwrap();
        }

        fn write_common_module(&self, name: &str) -> std::path::PathBuf {
            let directory = self.source().join("CommonModules");
            fs::create_dir_all(directory.join(name).join("Ext")).unwrap();
            fs::write(
                directory.join(format!("{name}.xml")),
                format!(
                    r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><CommonModule><Properties><Name>{name}</Name></Properties></CommonModule></MetaDataObject>"#
                ),
            )
            .unwrap();
            let module = directory.join(name).join("Ext/Module.bsl");
            fs::write(&module, "Procedure Run()\nEndProcedure\n").unwrap();
            module
        }

        fn write_form_module(&self, catalog: &str, form: &str) -> std::path::PathBuf {
            let directory = self.source().join("Catalogs").join(catalog);
            fs::create_dir_all(directory.join("Forms").join(form).join("Ext/Form")).unwrap();
            fs::write(
                directory.join("Forms").join(format!("{form}.xml")),
                format!(
                    r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Form><Properties><Name>{form}</Name></Properties></Form></MetaDataObject>"#
                ),
            )
            .unwrap();
            let module = directory
                .join("Forms")
                .join(form)
                .join("Ext/Form/Module.bsl");
            fs::write(&module, "Procedure Open()\nEndProcedure\n").unwrap();
            module
        }
    }

    fn address(raw: &str) -> MetadataAddress {
        MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, raw).unwrap()
    }

    fn request(metadata_path: Option<&str>) -> DiagnosticRequest {
        DiagnosticRequest {
            action: DiagnosticAction::Findings,
            source_set: "main".to_string(),
            metadata_path: metadata_path.map(address),
            filter: DiagnosticFilter {
                min_severity: Some(DiagnosticSeverity::Warning),
                codes: Vec::<DiagnosticCodeFilter>::new(),
            },
            range: None,
            limit: 200,
            timeout: Some(Duration::from_secs(30)),
        }
    }

    #[test]
    fn diagnostics_context_accepts_external_modules_without_starting_a_provider() {
        for (source_type, directory, object_kind, name) in [
            (
                "EXTERNAL_DATA_PROCESSORS",
                "epf",
                "ExternalDataProcessor",
                "Review",
            ),
            ("EXTERNAL_REPORTS", "erf", "ExternalReport", "Analysis"),
        ] {
            let temp = tempfile::tempdir().unwrap();
            let root =
                crate::infrastructure::source_roots::normalize_path_identity(temp.path()).unwrap();
            fs::write(
                root.join("v8project.yaml"),
                format!(
                    "format: DESIGNER\nsource-set:\n  - name: external\n    type: {source_type}\n    path: {directory}\n"
                ),
            )
            .unwrap();
            fs::create_dir_all(root.join(format!("{directory}/{name}/Ext"))).unwrap();
            fs::write(
                root.join(format!("{directory}/{name}.xml")),
                format!(
                    r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><{object_kind}><Properties><Name>{name}</Name></Properties><ChildObjects/></{object_kind}></MetaDataObject>"#
                ),
            )
            .unwrap();
            fs::write(
                root.join(format!("{directory}/{name}/Ext/ObjectModule.bsl")),
                "Procedure Run()\nEndProcedure\n",
            )
            .unwrap();
            let context = WorkspaceContext {
                cwd: root.clone(),
                workspace_root: root.clone(),
                cache_root: root.join(".build/unica"),
                workspace_epoch: 1,
            };
            let metadata_path = format!("{object_kind}.{name}.ObjectModule");
            let mut request = request(Some(&metadata_path));
            request.source_set = "external".to_string();

            let resolved =
                resolve_diagnostic_context(&request, &context, &CancellationToken::new()).unwrap();

            assert_eq!(
                resolved.target.metadata_path.as_ref().unwrap().as_str(),
                metadata_path
            );
            assert_eq!(resolved.target.target_kind, TargetKind::Module);
        }
    }

    #[test]
    fn diagnostics_context_accepts_external_source_root_for_whole_set_actions() {
        for (source_type, directory) in [
            ("EXTERNAL_DATA_PROCESSORS", "epf"),
            ("EXTERNAL_REPORTS", "erf"),
        ] {
            let temp = tempfile::tempdir().unwrap();
            let root =
                crate::infrastructure::source_roots::normalize_path_identity(temp.path()).unwrap();
            fs::write(
                root.join("v8project.yaml"),
                format!(
                    "format: DESIGNER\nsource-set:\n  - name: external\n    type: {source_type}\n    path: {directory}\n"
                ),
            )
            .unwrap();
            fs::create_dir_all(root.join(directory)).unwrap();
            let context = WorkspaceContext {
                cwd: root.clone(),
                workspace_root: root.clone(),
                cache_root: root.join(".build/unica"),
                workspace_epoch: 1,
            };

            for action in [
                DiagnosticAction::Analyze,
                DiagnosticAction::Status,
                DiagnosticAction::Catalog,
            ] {
                let mut request = request(None);
                request.action = action;
                request.source_set = "external".to_string();

                let resolved =
                    resolve_diagnostic_context(&request, &context, &CancellationToken::new())
                        .unwrap();

                assert_eq!(resolved.target.source_set, "external");
                assert_eq!(resolved.target.metadata_path, None);
                assert_eq!(resolved.target.target_kind, TargetKind::SourceRoot);
                assert_eq!(resolved.source_root.path, root.join(directory));
            }
        }
    }

    fn diagnostic(
        location: DiagnosticObservationLocation,
        focus: DiagnosticObservationFocus,
    ) -> DiagnosticObservation {
        DiagnosticObservation::Diagnostic {
            provider: TEST_PROVIDER,
            location,
            focus,
            code: "TEST001".to_string(),
            severity: DiagnosticSeverity::Warning,
            message: "finding".to_string(),
            tags: vec![DiagnosticTag::Unnecessary],
        }
    }

    fn mapped_location(item: DiagnosticItem) -> (SourceLocation, Option<UnaddressableReason>) {
        match item {
            DiagnosticItem::Diagnostic {
                location,
                location_reason,
                ..
            } => (location, location_reason),
            item => panic!("expected diagnostic item, got {item:?}"),
        }
    }

    #[test]
    fn diagnostic_location_maps_root_object_module_and_nested_form_module() {
        let fixture = Fixture::platform_xml();
        fixture.write_catalog("Items");
        let module = fixture.write_common_module("Shared");
        let form_module = fixture.write_form_module("Items", "Card");
        let cancellation = CancellationToken::new();
        let context =
            resolve_diagnostic_context(&request(None), &fixture.context, &cancellation).unwrap();

        let root = map_diagnostic_observation(
            diagnostic(
                DiagnosticObservationLocation::Logical {
                    metadata_path: None,
                },
                DiagnosticObservationFocus::Target,
            ),
            &context,
            &cancellation,
        )
        .unwrap();
        assert!(matches!(
            mapped_location(root).0,
            SourceLocation::Addressed {
                target_kind: TargetKind::SourceRoot,
                ..
            }
        ));

        let object = map_diagnostic_observation(
            diagnostic(
                DiagnosticObservationLocation::Logical {
                    metadata_path: Some(address("Catalog.Items")),
                },
                DiagnosticObservationFocus::Target,
            ),
            &context,
            &cancellation,
        )
        .unwrap();
        assert!(matches!(
            mapped_location(object).0,
            SourceLocation::Addressed {
                target_kind: TargetKind::MetadataObject,
                ..
            }
        ));

        for (path, expected) in [
            (module, "CommonModule.Shared.Module"),
            (form_module, "Catalog.Items.Form.Card.FormModule"),
        ] {
            let item = map_diagnostic_observation(
                diagnostic(
                    DiagnosticObservationLocation::Resource {
                        handle: path.to_string_lossy().into_owned(),
                    },
                    DiagnosticObservationFocus::SourceRange(DiagnosticRange {
                        start_line: 0,
                        start_column: 0,
                        end_line: 0,
                        end_column: 1,
                    }),
                ),
                &context,
                &cancellation,
            )
            .unwrap();
            match mapped_location(item).0 {
                SourceLocation::Addressed {
                    metadata_path: Some(actual),
                    target_kind: TargetKind::Module,
                    ..
                } => assert_eq!(actual.as_str(), expected),
                location => panic!("unexpected module location: {location:?}"),
            }
        }
    }

    #[test]
    fn diagnostic_location_distinguishes_unaddressable_owner_and_unproven_owner() {
        let fixture = Fixture::platform_xml();
        fixture.write_catalog("Items");
        fs::create_dir_all(fixture.source().join("Catalogs/Items/Ext")).unwrap();
        fs::write(
            fixture.source().join("Catalogs/Items/Ext/Unknown.xml"),
            "<unknown/>",
        )
        .unwrap();
        fs::create_dir_all(fixture.source().join("CommonModules/Ghost/Ext")).unwrap();
        fs::write(
            fixture.source().join("CommonModules/Ghost/Ext/Module.bsl"),
            "Procedure Run()\nEndProcedure",
        )
        .unwrap();
        let cancellation = CancellationToken::new();
        let context =
            resolve_diagnostic_context(&request(None), &fixture.context, &cancellation).unwrap();

        let cases = [
            (
                "Catalogs/Items/Ext/Unknown.xml",
                UnaddressableReason::ResourceNotAddressable,
                Some("Catalog.Items"),
            ),
            (
                "CommonModules/Ghost/Ext/Module.bsl",
                UnaddressableReason::OwnerUnproven,
                None,
            ),
        ];
        for (handle, expected_reason, expected_owner) in cases {
            let item = map_diagnostic_observation(
                diagnostic(
                    DiagnosticObservationLocation::Resource {
                        handle: handle.to_string(),
                    },
                    DiagnosticObservationFocus::Target,
                ),
                &context,
                &cancellation,
            )
            .unwrap();
            let (location, reason) = mapped_location(item);
            match location {
                SourceLocation::Unaddressable {
                    owner_metadata_path,
                    path,
                    ..
                } => {
                    assert_eq!(reason, Some(expected_reason));
                    assert_eq!(
                        owner_metadata_path.as_ref().map(MetadataAddress::as_str),
                        expected_owner
                    );
                    assert!(!path.contains('\\'));
                    assert!(!Path::new(&path).is_absolute());
                }
                location => panic!("unexpected unaddressable location: {location:?}"),
            }
        }
    }

    #[test]
    fn diagnostic_location_preserves_exact_metadata_focus_and_weakens_unknown_elements() {
        let fixture = Fixture::platform_xml();
        fixture.write_catalog("Items");
        let cancellation = CancellationToken::new();
        let context = resolve_diagnostic_context(
            &request(Some("Catalog.Items")),
            &fixture.context,
            &cancellation,
        )
        .unwrap();

        let focuses = [
            MetadataFocus {
                element_path: Vec::new(),
                property: Some("Synonym".to_string()),
                language: Some("ru".to_string()),
            },
            MetadataFocus {
                element_path: vec![MetadataElement {
                    collection: "attributes".to_string(),
                    name: "Code".to_string(),
                }],
                property: Some("Type".to_string()),
                language: None,
            },
            MetadataFocus {
                element_path: vec![
                    MetadataElement {
                        collection: "tabularSections".to_string(),
                        name: "Lines".to_string(),
                    },
                    MetadataElement {
                        collection: "attributes".to_string(),
                        name: "Price".to_string(),
                    },
                ],
                property: Some("Type".to_string()),
                language: None,
            },
        ];
        for expected in focuses {
            let item = map_diagnostic_observation(
                diagnostic(
                    DiagnosticObservationLocation::Logical {
                        metadata_path: Some(address("Catalog.Items")),
                    },
                    DiagnosticObservationFocus::Metadata(expected.clone()),
                ),
                &context,
                &cancellation,
            )
            .unwrap();
            match item {
                DiagnosticItem::Diagnostic {
                    focus:
                        DiagnosticFocus::Metadata {
                            element_path,
                            property,
                            language,
                        },
                    ..
                } => {
                    assert_eq!(element_path, expected.element_path);
                    assert_eq!(property, expected.property);
                    assert_eq!(language, expected.language);
                }
                item => panic!("expected exact metadata focus, got {item:?}"),
            }
        }

        let unknown = map_diagnostic_observation(
            diagnostic(
                DiagnosticObservationLocation::Logical {
                    metadata_path: Some(address("Catalog.Items")),
                },
                DiagnosticObservationFocus::Metadata(MetadataFocus {
                    element_path: vec![MetadataElement {
                        collection: "attributes".to_string(),
                        name: "Missing".to_string(),
                    }],
                    property: Some("Type".to_string()),
                    language: None,
                }),
            ),
            &context,
            &cancellation,
        )
        .unwrap();
        assert!(matches!(
            unknown,
            DiagnosticItem::Diagnostic {
                focus: DiagnosticFocus::Target,
                ..
            }
        ));

        let private_xml_field = map_diagnostic_observation(
            diagnostic(
                DiagnosticObservationLocation::Logical {
                    metadata_path: Some(address("Catalog.Items")),
                },
                DiagnosticObservationFocus::Metadata(MetadataFocus {
                    element_path: Vec::new(),
                    property: Some("InternalField".to_string()),
                    language: None,
                }),
            ),
            &context,
            &cancellation,
        )
        .unwrap();
        assert!(matches!(
            private_xml_field,
            DiagnosticItem::Diagnostic {
                focus: DiagnosticFocus::Target,
                ..
            }
        ));
    }

    #[test]
    fn diagnostics_windows_normalizes_separators_unicode_file_uri_and_dot_segments() {
        let fixture = Fixture::platform_xml();
        let module = fixture.write_common_module("Модуль с пробелом");
        let cancellation = CancellationToken::new();
        let context =
            resolve_diagnostic_context(&request(None), &fixture.context, &cancellation).unwrap();
        let absolute_uri = url::Url::from_file_path(&module).unwrap().to_string();
        let handles = [
            "CommonModules\\Модуль с пробелом\\Ext\\Module.bsl".to_string(),
            "./CommonModules/Модуль с пробелом/Ext/./Module.bsl".to_string(),
            absolute_uri,
        ];

        for handle in handles {
            let item = map_diagnostic_observation(
                diagnostic(
                    DiagnosticObservationLocation::Resource { handle },
                    DiagnosticObservationFocus::Target,
                ),
                &context,
                &cancellation,
            )
            .unwrap();
            match mapped_location(item).0 {
                SourceLocation::Addressed {
                    metadata_path: Some(address),
                    ..
                } => assert_eq!(address.as_str(), "CommonModule.Модуль с пробелом.Module"),
                location => panic!("unexpected normalized location: {location:?}"),
            }
        }
    }

    #[test]
    fn diagnostic_location_rejects_escape_without_leaking_the_raw_handle() {
        let fixture = Fixture::platform_xml();
        let cancellation = CancellationToken::new();
        let context =
            resolve_diagnostic_context(&request(None), &fixture.context, &cancellation).unwrap();
        let raw = "../outside/secret.bsl";
        let error = map_diagnostic_observation(
            diagnostic(
                DiagnosticObservationLocation::Resource {
                    handle: raw.to_string(),
                },
                DiagnosticObservationFocus::Target,
            ),
            &context,
            &cancellation,
        )
        .unwrap_err();

        assert_eq!(error.code, "location_outside_source_set");
        assert!(!error.message.contains(raw));
        assert!(!error.message.contains("secret.bsl"));
    }

    #[test]
    fn diagnostic_location_reports_unsupported_source_format_without_a_physical_path() {
        let fixture = Fixture::platform_xml();
        let cancellation = CancellationToken::new();
        let mut context =
            resolve_diagnostic_context(&request(None), &fixture.context, &cancellation).unwrap();
        context.source_set.source_format = SourceFormat::Edt;
        let item = map_diagnostic_observation(
            diagnostic(
                DiagnosticObservationLocation::Resource {
                    handle: "src/CommonModules/Any/Ext/Module.bsl".to_string(),
                },
                DiagnosticObservationFocus::Target,
            ),
            &context,
            &cancellation,
        )
        .unwrap();

        let (location, reason) = mapped_location(item);
        assert_eq!(reason, Some(UnaddressableReason::SourceFormatUnsupported));
        assert!(matches!(
            location,
            SourceLocation::Unaddressable { path, .. }
                if path == "CommonModules/Any/Ext/Module.bsl"
        ));
    }

    #[test]
    fn zero_width_provider_range_stays_a_caret_focus() {
        let fixture = Fixture::platform_xml();
        let module = fixture.write_common_module("Shared");
        let cancellation = CancellationToken::new();
        let context =
            resolve_diagnostic_context(&request(None), &fixture.context, &cancellation).unwrap();
        let caret = DiagnosticRange {
            start_line: 40,
            start_column: 4,
            end_line: 40,
            end_column: 4,
        };

        let item = map_diagnostic_observation(
            diagnostic(
                DiagnosticObservationLocation::Resource {
                    handle: module.to_string_lossy().into_owned(),
                },
                DiagnosticObservationFocus::SourceRange(caret),
            ),
            &context,
            &cancellation,
        )
        .unwrap();

        assert!(
            matches!(
                &item,
                DiagnosticItem::Diagnostic {
                    focus: DiagnosticFocus::SourceRange { range },
                    ..
                } if *range == caret
            ),
            "a zero-width range must keep its position instead of weakening to target: {item:?}"
        );
    }

    #[test]
    fn metadata_focus_descriptor_is_read_once_per_mapped_batch() {
        let mut cache = super::DiagnosticMappingCache::default();
        let mut reads = 0usize;
        let mut load = |cache: &mut super::DiagnosticMappingCache| {
            cache
                .descriptor_text("Catalog.Items", || {
                    reads += 1;
                    Some("<MetaDataObject/>".to_string())
                })
                .map(str::to_string)
        };

        assert_eq!(load(&mut cache).as_deref(), Some("<MetaDataObject/>"));
        assert_eq!(load(&mut cache).as_deref(), Some("<MetaDataObject/>"));
        assert_eq!(reads, 1, "the descriptor must be read once per batch");

        let mut misses = 0usize;
        for _ in 0..2 {
            assert!(cache
                .descriptor_text("Catalog.Missing", || {
                    misses += 1;
                    None
                })
                .is_none());
        }
        assert_eq!(misses, 1, "an unreadable descriptor is not retried either");
    }

    mod platform_tests {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/platform/diagnostics_windows.rs"
        ));
    }
}
