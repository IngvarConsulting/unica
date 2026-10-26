//! BSL analyzer resident protocol and substantive diagnostics interpretation.
use crate::adapter::{AnalyzerRunFailure, AnalyzerRunFailureKind, BslAnalyzerMcpAdapter};
use crate::baseline::{
    baseline_verdict, BaselineFacts, BaselineIssue, BaselineState, BaselineVerdict,
};
use crate::environment::BslEnvironment;
use crate::jsonl::AnalyzerDiagnosticsBatch;
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;
use unica_address::source_target::TargetKind;
use unica_analysis_services::{WorkspaceServiceBslCall, WorkspaceServiceManager};
use unica_diagnostics::model::*;
use unica_redaction::redactor;
use unica_work_control::cancellation::CancellationToken;
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

const BSL_RESIDENT_FINDINGS_CAP: usize = 5_000;

#[derive(Debug, Clone, PartialEq)]
pub enum BslDiagnosticBackendRequest {
    Analyze {
        source_root: PathBuf,
        module: Option<PathBuf>,
    },
    Resident {
        source_root: PathBuf,
        tool_name: &'static str,
        arguments: Value,
    },
}

impl BslDiagnosticBackendRequest {
    #[cfg(any(test, feature = "test-support"))]
    pub fn arguments(&self) -> &Value {
        match self {
            Self::Resident { arguments, .. } => arguments,
            Self::Analyze { .. } => &Value::Null,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BslDiagnosticResidentReply {
    pub result_text: String,
    pub stderr: String,
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BslDiagnosticBackendReply {
    Analyze(Box<AnalyzerDiagnosticsBatch>),
    Resident(BslDiagnosticResidentReply),
}

pub trait BslDiagnosticBackend: Send + Sync {
    fn invoke(
        &self,
        context: &DiagnosticContext,
        request: BslDiagnosticBackendRequest,
        timeout: Option<Duration>,
        cancellation: &CancellationToken,
    ) -> Result<BslDiagnosticBackendReply, AnalyzerRunFailure>;
}

pub struct WorkspaceBslDiagnosticBackend {
    pub environment: &'static dyn BslEnvironment,
}

impl BslDiagnosticBackend for WorkspaceBslDiagnosticBackend {
    fn invoke(
        &self,
        context: &DiagnosticContext,
        request: BslDiagnosticBackendRequest,
        timeout: Option<Duration>,
        cancellation: &CancellationToken,
    ) -> Result<BslDiagnosticBackendReply, AnalyzerRunFailure> {
        match request {
            BslDiagnosticBackendRequest::Analyze {
                source_root,
                module,
            } => BslAnalyzerMcpAdapter::new(self.environment)
                .analyze_diagnostic_batch(
                    &context.workspace,
                    &source_root,
                    module.as_deref(),
                    timeout,
                    cancellation,
                )
                .map(|batch| BslDiagnosticBackendReply::Analyze(Box::new(batch))),
            BslDiagnosticBackendRequest::Resident {
                source_root,
                tool_name,
                arguments,
            } => {
                let timeout = timeout.ok_or_else(|| {
                    AnalyzerRunFailure::from(
                        unica_work_control::provider_deadline::ProviderDeadlineError::Unsupported
                            .to_string(),
                    )
                })?;
                WorkspaceServiceManager::new(self.environment.service_environment())
                    .call_bsl_mcp_cancellable_with_budget(
                        &context.workspace,
                        &source_root,
                        WorkspaceServiceBslCall::new(tool_name, arguments, timeout, timeout),
                        cancellation,
                    )
                    .map_err(|error| resident_failure(self.environment, context, error))
                    .map(|output| {
                        let version = self
                            .environment
                            .find_plugin_root(&context.workspace.cwd)
                            .and_then(|root| {
                                self.environment
                                    .bundled_tool_version(&root, "bsl-analyzer")
                                    .ok()
                            });
                        BslDiagnosticBackendReply::Resident(BslDiagnosticResidentReply {
                            result_text: output.result_text,
                            stderr: output.stderr,
                            version,
                        })
                    })
            }
        }
    }
}

/// A resident session fails with prose only. Whether the engine is absent is
/// read from the disk through the typed [`MissingEngine`], never from that
/// prose: a transient "no such file" is not a missing engine (#1338).
///
/// [`MissingEngine`]: unica_artifact_delivery::engine::MissingEngine
fn resident_failure(
    environment: &dyn BslEnvironment,
    context: &DiagnosticContext,
    error: String,
) -> AnalyzerRunFailure {
    let missing = environment
        .find_plugin_root(&context.workspace.cwd)
        .and_then(|root| environment.missing_engine(&root, BSL_ANALYZER_ENGINE));
    match missing {
        Some(missing) => AnalyzerRunFailure::engine_missing(Some(missing), error),
        None => AnalyzerRunFailure::from(error),
    }
}

const BSL_ANALYZER_ENGINE: &str = "bsl-analyzer";

/// The real bsl-analyzer provider over a backend that records the process
/// timeout it would hand the analyzer, for tests that drive `check` through
/// the daemon's invocation service.
#[cfg(any(test, feature = "test-support"))]
pub mod analyze_timeout_probe {
    use super::*;

    pub struct RecordingAnalyzeBackend {
        timeouts: std::sync::Mutex<Vec<Option<Duration>>>,
    }

    impl RecordingAnalyzeBackend {
        pub fn timeouts(&self) -> Vec<Option<Duration>> {
            self.timeouts.lock().unwrap().clone()
        }
    }

    impl BslDiagnosticBackend for RecordingAnalyzeBackend {
        fn invoke(
            &self,
            _context: &DiagnosticContext,
            request: BslDiagnosticBackendRequest,
            timeout: Option<Duration>,
            _cancellation: &CancellationToken,
        ) -> Result<BslDiagnosticBackendReply, AnalyzerRunFailure> {
            self.timeouts.lock().unwrap().push(timeout);
            match request {
                BslDiagnosticBackendRequest::Analyze { .. } => Ok(
                    BslDiagnosticBackendReply::Analyze(Box::new(AnalyzerDiagnosticsBatch {
                        outcome: DiagnosticProviderOutcome {
                            status: DiagnosticProviderStatus::Empty,
                            complete: true,
                            version: Some("probe".to_string()),
                            observations: Vec::new(),
                            rules: Vec::new(),
                            readiness: None,
                            error: None,
                            suppressions: Vec::new(),
                        },
                        files: crate::jsonl::AnalyzerDiagnosticsFileTotals {
                            discovered: Some(1),
                            processed: Some(1),
                            failed: Some(0),
                        },
                        diagnostics_reported: Some(0),
                        elapsed_seconds: Some(0.0),
                    })),
                ),
                BslDiagnosticBackendRequest::Resident { .. } => {
                    Err("the analyze probe answers only analyze".to_string().into())
                }
            }
        }
    }

    /// A backend whose run stops at a closed place, for `check` paths that
    /// must name the cause instead of "provider is unavailable" (#1338).
    struct FailingAnalyzeBackend(AnalyzerRunFailureKind);

    impl BslDiagnosticBackend for FailingAnalyzeBackend {
        fn invoke(
            &self,
            _context: &DiagnosticContext,
            _request: BslDiagnosticBackendRequest,
            _timeout: Option<Duration>,
            _cancellation: &CancellationToken,
        ) -> Result<BslDiagnosticBackendReply, AnalyzerRunFailure> {
            Err(failure_for_test(self.0))
        }
    }

    /// A failure of the given kind; an absent engine is described as a
    /// source checkout misses it, with a private expected path.
    pub fn failure_for_test(kind: AnalyzerRunFailureKind) -> AnalyzerRunFailure {
        let message = "private-token /private/engine/bsl-analyzer stderr";
        if kind == AnalyzerRunFailureKind::EngineMissing {
            AnalyzerRunFailure::engine_missing(
                Some(unica_artifact_delivery::engine::MissingEngine::new(
                    "bsl-analyzer",
                    "darwin-arm64",
                    "/private/engine/bin/darwin-arm64/bsl-analyzer",
                    Some("0.2.86".to_string()),
                    unica_artifact_delivery::engine::InstallMode::Source,
                )),
                message,
            )
        } else {
            AnalyzerRunFailure::new(kind, message)
        }
    }

    pub fn failing_provider(
        kind: AnalyzerRunFailureKind,
    ) -> std::sync::Arc<dyn DiagnosticProvider> {
        let backend: &'static FailingAnalyzeBackend =
            Box::leak(Box::new(FailingAnalyzeBackend(kind)));
        std::sync::Arc::new(BslAnalyzerDiagnosticProvider::new(
            backend,
            analyze_probe_findings_unavailable,
        ))
    }

    pub fn recording_provider() -> (
        std::sync::Arc<dyn DiagnosticProvider>,
        &'static RecordingAnalyzeBackend,
    ) {
        let backend: &'static RecordingAnalyzeBackend =
            Box::leak(Box::new(RecordingAnalyzeBackend {
                timeouts: std::sync::Mutex::new(Vec::new()),
            }));
        (
            std::sync::Arc::new(BslAnalyzerDiagnosticProvider::new(
                backend,
                analyze_probe_findings_unavailable,
            )),
            backend,
        )
    }
}

pub type FindingsModuleResolver =
    fn(&DiagnosticProviderRequest, &DiagnosticContext) -> Result<PathBuf, String>;
pub struct BslAnalyzerDiagnosticProvider<'a> {
    findings_module: FindingsModuleResolver,
    backend: &'a (dyn BslDiagnosticBackend + Send + Sync),
}

impl<'a> BslAnalyzerDiagnosticProvider<'a> {
    pub fn new(
        backend: &'a (dyn BslDiagnosticBackend + Send + Sync),
        findings_module: FindingsModuleResolver,
    ) -> Self {
        Self {
            backend,
            findings_module,
        }
    }

    fn execute_inner(
        &self,
        request: &DiagnosticProviderRequest,
        context: &DiagnosticContext,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<DiagnosticProviderOutcome, AnalyzerRunFailure> {
        if cancellation.is_cancelled() {
            return Err(AnalyzerRunFailure::new(
                AnalyzerRunFailureKind::Cancelled,
                "cancelled: diagnostics provider stopped before request",
            ));
        }
        // Анализ модуля для `check` идёт без срока, если пользователь его
        // не задал (#1251). Резидентные действия прежней поверхности
        // по-прежнему требуют конечного срока.
        let timeout = match (request.action, deadline.finite_remaining()) {
            (_, Ok(timeout)) => Some(timeout),
            (DiagnosticAction::Analyze, Err(_)) => None,
            (_, Err(error)) => {
                return Ok(provider_failed_with_status(
                    DiagnosticProviderStatus::Unsupported,
                    None,
                    "provider_deadline_unsupported",
                    error.to_string(),
                    false,
                ))
            }
        };
        if timeout.is_some_and(|timeout| timeout.is_zero()) {
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
                module: request
                    .module_scope
                    .as_ref()
                    .map(|scope| scope.module_path.clone()),
            },
            DiagnosticAction::Findings => {
                let module = (self.findings_module)(request, context)?;
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

    fn editor_finding(
        &self,
        code: &str,
        severity: unica_diagnostics::model::DiagnosticSeverity,
    ) -> unica_diagnostics::model::EditorFinding {
        crate::editor_codes::editor_finding(code, severity)
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
            Err(failure) => backend_failure_outcome(failure),
        }
    }
}

pub fn findings_arguments(module_path: &Path) -> Value {
    json!({
        "action": "file",
        "path": module_path,
        // These are fixed widening hints, not public filters. The coordinator
        // owns the authoritative filter and limit after logical mapping.
        "min_severity": "hint",
        "max_findings": BSL_RESIDENT_FINDINGS_CAP,
    })
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
    out_of_scope: bool,
    findings_ignored_by_author: Option<usize>,
    #[serde(default)]
    findings: Vec<ResidentFinding>,
    error: Option<String>,
    detail: Option<String>,
    /// bsl-analyzer 0.2.86+: the `[diagnostics.baseline]` that filtered this
    /// reply. The resident reply adds bounded partition fields to the summary,
    /// so only the fields the verdict reads are typed; the state is closed.
    baseline: Option<ResidentBaseline>,
}

#[derive(Debug, Deserialize)]
struct ResidentBaseline {
    state: BaselineState,
    known: Option<usize>,
    new: Option<usize>,
    error_code: Option<String>,
    detail: Option<String>,
    #[serde(default)]
    errors: Vec<ResidentBaselineError>,
}

#[derive(Debug, Deserialize)]
struct ResidentBaselineError {
    code: String,
    detail: String,
}

impl ResidentFindingsResult {
    fn baseline_verdict(&self) -> BaselineVerdict {
        self.baseline
            .as_ref()
            .map_or_else(BaselineVerdict::disabled, |baseline| {
                baseline_verdict(BaselineFacts {
                    state: baseline.state,
                    known: baseline.known,
                    new: baseline.new,
                    error_code: baseline.error_code.as_deref(),
                    detail: baseline.detail.as_deref(),
                    first_error: baseline.errors.first().map(|error| BaselineIssue {
                        code: &error.code,
                        detail: &error.detail,
                    }),
                })
            })
    }
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

pub fn parse_resident_findings(
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
            suppressions: Vec::new(),
        });
    }
    let baseline = envelope.result.baseline_verdict();
    let mut suppressions: Vec<_> = baseline.suppression.into_iter().collect();
    if let Some(count) = envelope
        .result
        .findings_ignored_by_author
        .and_then(std::num::NonZeroUsize::new)
    {
        suppressions.push(DiagnosticSuppression::Authors { count });
    }
    let mut observations = Vec::with_capacity(envelope.result.findings.len());
    for finding in envelope.result.findings {
        observations.push(DiagnosticObservation::Diagnostic {
            provider: BSL_ANALYZER_PROVIDER,
            location: DiagnosticObservationLocation::Resource {
                handle: handle.clone(),
            },
            focus: DiagnosticObservationFocus::SourceRange(
                unica_diagnostics::model::DiagnosticRange {
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
    if envelope.result.out_of_scope {
        observations.push(DiagnosticObservation::ResourceFailure {
            provider: BSL_ANALYZER_PROVIDER,
            location: DiagnosticObservationLocation::Resource { handle },
            error: DiagnosticError {
                code: "analysis_out_of_scope".to_string(),
                message: "selected resource is outside the analyzer's configured analysis scope"
                    .to_string(),
                retryable: false,
            },
        });
    }
    Ok(DiagnosticProviderOutcome {
        // A configured baseline is named with the findings, so a list it
        // emptied reads as filtered rather than as a payload-free "clean".
        status: if observations.is_empty() && !envelope.result.truncated && suppressions.is_empty()
        {
            DiagnosticProviderStatus::Empty
        } else {
            DiagnosticProviderStatus::Completed
        },
        complete: !envelope.result.truncated && baseline.intact && !envelope.result.out_of_scope,
        version: reply.version,
        observations,
        rules: Vec::new(),
        readiness: None,
        error: None,
        suppressions,
    })
}

#[derive(Debug, Deserialize)]
struct ResidentStatusReply {
    state: String,
    #[serde(default)]
    reload: String,
    error: Option<String>,
}

pub fn parse_resident_status(
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
        suppressions: Vec::new(),
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

pub fn parse_resident_catalog(
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
        suppressions: Vec::new(),
    })
}

pub fn parse_common_severity(value: &str) -> Result<DiagnosticSeverity, String> {
    match value {
        "error" | "Blocker" | "Critical" | "Major" | "Error" => Ok(DiagnosticSeverity::Error),
        "warning" | "Warning" => Ok(DiagnosticSeverity::Warning),
        "info" | "Information" => Ok(DiagnosticSeverity::Info),
        "hint" | "Hint" => Ok(DiagnosticSeverity::Hint),
        _ => Err("bsl-analyzer returned an unknown diagnostic severity".to_string()),
    }
}

pub fn parse_common_tags(tags: &[String]) -> Vec<DiagnosticTag> {
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

/// The closed place a backend run stopped becomes the section's code; the
/// backend's own text never leaves the adapter (#1338). The public wording of
/// each code belongs to the diagnostics coordinator.
pub fn backend_failure_outcome(failure: AnalyzerRunFailure) -> DiagnosticProviderOutcome {
    let message = failure.missing.as_deref().map_or_else(
        || "bsl-analyzer diagnostics provider did not produce a result".to_string(),
        unica_diagnostics::coordinator::missing_engine_public_reason,
    );
    let (status, code, retryable) = match failure.kind {
        AnalyzerRunFailureKind::Cancelled => {
            return provider_failed(
                None,
                "cancelled",
                "bsl-analyzer diagnostics request was cancelled",
                false,
            )
        }
        AnalyzerRunFailureKind::EngineMissing => (
            DiagnosticProviderStatus::Unavailable,
            unica_artifact_delivery::engine::BUNDLED_TOOL_MISSING,
            false,
        ),
        AnalyzerRunFailureKind::EngineInvalid => (
            DiagnosticProviderStatus::Unavailable,
            "provider_install_invalid",
            false,
        ),
        AnalyzerRunFailureKind::StartFailed => (
            DiagnosticProviderStatus::Unavailable,
            "provider_start_failed",
            true,
        ),
        AnalyzerRunFailureKind::TimedOut => {
            (DiagnosticProviderStatus::Failed, "provider_timeout", true)
        }
        AnalyzerRunFailureKind::Exited => {
            (DiagnosticProviderStatus::Failed, "provider_exited", true)
        }
        AnalyzerRunFailureKind::Other => (
            DiagnosticProviderStatus::Failed,
            "provider_unavailable",
            true,
        ),
    };
    provider_failed_with_status(status, None, code, message, retryable)
}

pub fn provider_not_ready(
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

pub fn provider_failed(
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

pub fn provider_failed_with_status(
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
        suppressions: Vec::new(),
    }
}

#[cfg(any(test, feature = "test-support"))]
fn analyze_probe_findings_unavailable(
    _: &DiagnosticProviderRequest,
    _: &DiagnosticContext,
) -> Result<PathBuf, String> {
    Err("analyze probe has no findings source binding".to_string())
}
