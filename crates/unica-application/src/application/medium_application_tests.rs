use crate::application::ports::{FormatGuardCheck, FormatGuardError, SupportGuardCheck};
use crate::application::*;
use serde_json::{json, Value};
use std::path::PathBuf;
use unica_code_intelligence::model::CodeIntelligenceReadRequest;
use unica_code_intelligence::model::SearchRequest;
use unica_execution::runner::admission as runtime_admission;
use unica_execution_contract::adapter_outcome::AdapterOutcome;
use unica_navigation::deferred_delivery;
use unica_source_contract::events::{DomainEvent, DomainEventKind};
use unica_source_contract::workspace::WorkspaceContext;
use unica_work_control::cancellation::CancellationToken;
use unica_work_control::progress::{NoopProgressSink, ProgressSink};
use unica_work_control::provider_deadline::ProviderDeadline;
use unica_workspace::cache::{CacheAccess, CacheReport};

use serde_json::Map;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

fn call_tool_after_runtime_admission_for_test(
    app: &UnicaApplication,
    name: &str,
    args: &Map<String, Value>,
) -> Result<OperationResult, String> {
    let spec = tools()
        .into_iter()
        .find(|tool| tool.name == name)
        .ok_or_else(|| format!("unknown unica tool: {name}"))?;
    call_tool_with_runtime_admission(
        spec,
        args,
        app.ports.as_ref(),
        &CancellationToken::new(),
        ProviderDeadline::from_budget(PUBLIC_INVOCATION_DEADLINE),
        &NoopProgressSink,
        RuntimeAdmissionPolicy::AlreadyAdmittedForDownstreamContractTest,
        app.deferred.as_ref(),
    )
}

fn object_schema_property_maps(schema: &Value) -> Vec<&Map<String, Value>> {
    if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
        return vec![properties];
    }
    schema["oneOf"]
        .as_array()
        .expect("tool schema publishes properties or closed object oneOf branches")
        .iter()
        .map(|branch| {
            branch["properties"]
                .as_object()
                .expect("oneOf branch properties are an object")
        })
        .collect()
}

fn call_tool_after_runtime_admission_for_downstream_test(
    app: &UnicaApplication,
    name: &str,
    args: &Map<String, Value>,
) -> Result<OperationResult, String> {
    if name == "unica.runtime.execute" {
        call_tool_after_runtime_admission_for_test(app, name, args)
    } else {
        app.call_tool(name, args)
    }
}

#[test]
fn code_search_schema_requires_a_machine_readable_section_termination() {
    let schema = code_search_output_schema();
    let section = &schema["properties"]["data"]["properties"]["sections"]["items"];

    assert!(section["required"]
        .as_array()
        .is_some_and(|required| required.iter().any(|field| field == "termination")));
    assert_eq!(
        section["properties"]["termination"]["oneOf"][1]["properties"]["code"]["enum"],
        json!([
            "limitReached",
            "deadlineExceeded",
            "dependencyPending",
            "unsupportedScope",
            "capacityExhausted",
            "providerUnavailable",
            "providerFailed"
        ])
    );
    assert_eq!(
        section["properties"]["termination"]["oneOf"][1]["properties"]["retryable"]["type"],
        "boolean"
    );
}

#[test]
fn code_search_schema_accepts_a_real_partial_provider_section() {
    use unica_code_intelligence::model::ProviderId;
    use unica_code_intelligence::model::ProviderSearchHit;
    use unica_code_intelligence::model::ProviderSearchSection;
    use unica_code_intelligence::model::SearchOrdering;
    use unica_code_intelligence::model::SearchRanking;
    use unica_source_contract::location::SourceLocation;

    let section = ProviderSearchSection::partial(
        ProviderId::SemanticFixture.identity(),
        SearchRanking::Provider,
        SearchOrdering::Provider,
        vec![ProviderSearchHit {
            rank: Some(1),
            provider_score: None,
            location: SourceLocation::Unaddressable {
                source_set: "main".to_string(),
                owner_metadata_path: None,
                path: "CommonModules/Sales/Ext/Module.bsl".to_string(),
            },
            line: 1,
            end_line: None,
            symbol: None,
            kind: None,
            snippet: "Post".to_string(),
            attributes: Map::new(),
        }],
        vec!["ignored malformed provider result #1".to_string()],
    )
    .unwrap();
    let schema = code_search_output_schema();
    let item_schema = &schema["properties"]["data"]["properties"]["sections"]["items"];
    let serialized = serde_json::to_value(section).unwrap();
    assert!(
        jsonschema::validator_for(item_schema)
            .unwrap()
            .is_valid(&serialized),
        "partial provider section must match the published output schema: {serialized}"
    );
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
    fn notify_committed_changes(
        &self,
        context: &WorkspaceContext,
        events: &[unica_source_contract::events::DomainEvent],
        _cancellation: &CancellationToken,
        _deadline: unica_work_control::operation_deadline::OperationDeadline,
    ) {
        self.notify_invalidation(context, events);
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

static DIAGNOSTICS_BOUNDARY_DESCRIPTOR: unica_diagnostics::model::DiagnosticProviderDescriptor =
    unica_diagnostics::model::DiagnosticProviderDescriptor {
        id: unica_diagnostics::model::BSL_ANALYZER_PROVIDER,
        actions: &[
            unica_diagnostics::model::DiagnosticAction::Analyze,
            unica_diagnostics::model::DiagnosticAction::Status,
        ],
        findings_target_kinds: &[],
        emits_focus_kinds: &[],
    };

struct DiagnosticsBoundaryProvider {
    calls: Arc<AtomicUsize>,
    observed_budget: Arc<Mutex<Option<Duration>>>,
    fail: bool,
}

impl unica_diagnostics::model::DiagnosticProvider for DiagnosticsBoundaryProvider {
    fn descriptor(&self) -> &'static unica_diagnostics::model::DiagnosticProviderDescriptor {
        &DIAGNOSTICS_BOUNDARY_DESCRIPTOR
    }

    fn execute(
        &self,
        request: &unica_diagnostics::model::DiagnosticProviderRequest,
        _context: &unica_diagnostics::model::DiagnosticContext,
        deadline: unica_diagnostics::model::ProviderDeadline,
        _cancellation: &CancellationToken,
    ) -> unica_diagnostics::model::DiagnosticProviderOutcome {
        self.calls.fetch_add(1, Ordering::SeqCst);
        *self.observed_budget.lock().unwrap() = deadline.remaining();
        if self.fail {
            return unica_diagnostics::model::DiagnosticProviderOutcome {
                status: unica_diagnostics::model::DiagnosticProviderStatus::Failed,
                complete: false,
                version: Some("test".to_string()),
                observations: Vec::new(),
                rules: Vec::new(),
                readiness: None,
                error: Some(unica_diagnostics::model::DiagnosticError {
                    code: "provider_unavailable".to_string(),
                    message: "test provider is unavailable".to_string(),
                    retryable: true,
                }),
                suppressions: Vec::new(),
            };
        }
        unica_diagnostics::model::DiagnosticProviderOutcome {
            status: if request.action == unica_diagnostics::model::DiagnosticAction::Analyze {
                unica_diagnostics::model::DiagnosticProviderStatus::Empty
            } else {
                unica_diagnostics::model::DiagnosticProviderStatus::Completed
            },
            complete: true,
            version: Some("test".to_string()),
            observations: Vec::new(),
            rules: Vec::new(),
            readiness: (request.action == unica_diagnostics::model::DiagnosticAction::Status)
                .then_some(unica_diagnostics::model::DiagnosticReadiness {
                    state: unica_diagnostics::model::DiagnosticReadinessState::Ready,
                    retryable: false,
                    progress: None,
                }),
            error: None,
            suppressions: Vec::new(),
        }
    }
}

#[derive(Default)]
struct DiagnosticsBoundaryPorts {
    handler_calls: AtomicUsize,
    provider_calls: Arc<AtomicUsize>,
    observed_budget: Arc<Mutex<Option<Duration>>>,
    fail_provider: bool,
}

impl ports::ApplicationPorts for DiagnosticsBoundaryPorts {
    fn discover_workspace(
        &self,
        requested_cwd: Option<PathBuf>,
    ) -> Result<WorkspaceContext, String> {
        let root = requested_cwd.unwrap_or_else(|| PathBuf::from("workspace"));
        Ok(WorkspaceContext {
            cwd: root.clone(),
            workspace_root: root.clone(),
            cache_root: root.join(".build/unica"),
            workspace_epoch: 7,
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

    fn diagnostic_provider_registry(
        &self,
    ) -> Result<unica_diagnostics::model::DiagnosticProviderRegistry, String> {
        unica_diagnostics::model::DiagnosticProviderRegistry::new(vec![Arc::new(
            DiagnosticsBoundaryProvider {
                calls: Arc::clone(&self.provider_calls),
                observed_budget: Arc::clone(&self.observed_budget),
                fail: self.fail_provider,
            },
        )])
        .map_err(|error| error.to_string())
    }

    fn resolve_diagnostic_context(
        &self,
        request: &unica_diagnostics::model::DiagnosticRequest,
        workspace: &WorkspaceContext,
        _cancellation: &CancellationToken,
    ) -> Result<
        unica_diagnostics::model::DiagnosticContext,
        unica_diagnostics::model::DiagnosticRequestError,
    > {
        Ok(unica_diagnostics::model::DiagnosticContext::new(
            workspace.clone(),
            unica_source_contract::ProjectSourceSet {
                name: request.source_set.clone(),
                kind: unica_source_contract::SourceSetKind::Configuration,
                path: "src".to_string(),
                source_format: unica_source_contract::SourceFormat::PlatformXml,
                source_state: unica_source_contract::SourceSetState::Supported,
                format_evidence: Vec::new(),
                format_probe_error: None,
            },
            unica_source_contract::workspace::ResolvedSourceRoot {
                source_set: Some(request.source_set.clone()),
                path: workspace.workspace_root.join("src"),
            },
            unica_address::source_target::ResolvedTarget {
                source_set: request.source_set.clone(),
                metadata_path: request.metadata_path.clone(),
                target_kind: unica_address::source_target::TargetKind::SourceRoot,
            },
        ))
    }

    fn map_diagnostic_observation(
        &self,
        _observation: unica_diagnostics::model::DiagnosticObservation,
        _context: &unica_diagnostics::model::DiagnosticContext,
        _cancellation: &CancellationToken,
    ) -> Result<
        unica_diagnostics::model::DiagnosticItem,
        unica_diagnostics::model::DiagnosticMapError,
    > {
        unreachable!("status providers do not emit observations")
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
        self.handler_calls.fetch_add(1, Ordering::SeqCst);
        Err("diagnostics was misrouted to the generic handler".to_string())
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
    fn notify_committed_changes(
        &self,
        context: &WorkspaceContext,
        events: &[unica_source_contract::events::DomainEvent],
        _cancellation: &CancellationToken,
        _deadline: unica_work_control::operation_deadline::OperationDeadline,
    ) {
        self.notify_invalidation(context, events);
    }

    fn notify_invalidation(&self, _context: &WorkspaceContext, _events: &[DomainEvent]) {}
}

#[test]
fn diagnostics_application_boundary_routes_only_through_coordinator_and_data() {
    fn contains_string(value: &Value, needle: &str) -> bool {
        match value {
            Value::Object(object) => object.values().any(|value| contains_string(value, needle)),
            Value::Array(items) => items.iter().any(|value| contains_string(value, needle)),
            Value::String(text) => text.contains(needle),
            _ => false,
        }
    }

    let ports = Arc::new(DiagnosticsBoundaryPorts::default());
    let app = UnicaApplication::with_ports(ports.clone());
    let workspace = std::env::temp_dir().join("unica-diagnostics-private-workspace");
    let args = json!({
        "action": "status",
        "sourceSet": "main",
        "cwd": workspace
    })
    .as_object()
    .unwrap()
    .clone();

    let result = app.call_tool("unica.code.diagnostics", &args).unwrap();
    let wire = serde_json::to_value(&result).unwrap();

    assert!(result.ok);
    assert_eq!(result.data.as_ref().unwrap()["state"], "completed");
    assert_eq!(wire["cache"]["root"], "");
    assert!(
        !contains_string(&wire, &workspace.display().to_string()),
        "serialized diagnostics result leaked workspace path: {wire}"
    );
    assert!(result.stdout.is_none());
    assert!(result.stderr.is_none());
    assert!(result.command.is_none());
    assert!(result.diagnostics.is_none());
    assert!(result.artifacts.is_empty());
    assert_eq!(ports.provider_calls.load(Ordering::SeqCst), 1);
    assert_eq!(ports.handler_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn diagnostics_application_boundary_preserves_failed_useful_result_semantics() {
    let ports = Arc::new(DiagnosticsBoundaryPorts {
        fail_provider: true,
        ..Default::default()
    });
    let app = UnicaApplication::with_ports(ports.clone());
    let args = json!({"action": "status", "sourceSet": "main"})
        .as_object()
        .unwrap()
        .clone();

    let result = app.call_tool("unica.code.diagnostics", &args).unwrap();

    assert!(!result.ok);
    assert_eq!(result.data.as_ref().unwrap()["state"], "failed");
    assert!(result.diagnostics.is_none());
    assert!(result.stdout.is_none());
    assert_eq!(ports.provider_calls.load(Ordering::SeqCst), 1);
    assert_eq!(ports.handler_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn diagnostics_analyze_timeout_override_reaches_provider_deadline() {
    let ports = Arc::new(DiagnosticsBoundaryPorts::default());
    let app = UnicaApplication::with_ports(ports.clone());
    let args = json!({
        "action": "analyze",
        "sourceSet": "main",
        "timeoutSeconds": 900
    })
    .as_object()
    .unwrap()
    .clone();

    let result = app.call_tool("unica.code.diagnostics", &args).unwrap();
    let observed = ports.observed_budget.lock().unwrap().unwrap();

    assert!(result.ok, "{result:?}");
    // The override is proven by the band it lands in, not by the second it
    // lands on: the deadline starts before the provider reads it, and a
    // loaded runner can spend more than a second in between.
    assert!(observed <= Duration::from_secs(900), "{observed:?}");
    assert!(observed > Duration::from_secs(880), "{observed:?}");
}

#[test]
fn diagnostics_application_boundary_cancellation_publishes_no_partial_data() {
    let ports = Arc::new(DiagnosticsBoundaryPorts::default());
    let app = UnicaApplication::with_ports(ports.clone());
    let args = json!({"action": "status", "sourceSet": "main"})
        .as_object()
        .unwrap()
        .clone();
    let cancellation = CancellationToken::new();
    cancellation.cancel();

    let error = app
        .call_tool_cancellable("unica.code.diagnostics", &args, cancellation)
        .unwrap_err();

    assert!(error.contains("cancel"), "{error}");
    assert_eq!(ports.provider_calls.load(Ordering::SeqCst), 0);
    assert_eq!(ports.handler_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn applied_runtime_carries_its_named_risk_into_the_result() {
    let args = Map::from_iter([
        ("operation".to_string(), json!("build")),
        ("dryRun".to_string(), Value::Bool(false)),
    ]);

    let result = UnicaApplication::with_ports(Arc::new(FixedOutcomePorts {
        outcome: AdapterOutcome::ok("v8-runner finished the applied build"),
        data: None,
    }))
    .call_tool("unica.runtime.execute", &args)
    .expect("an applied runtime call answers with an OperationResult");

    assert!(result.ok, "{result:?}");
    assert!(
        result
            .warnings
            .iter()
            .any(|warning| warning.starts_with("runtime_risk_critical_non_abortable:")),
        "{:?}",
        result.warnings
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

impl unica_code_intelligence::model::CodeIntelligenceProvider for FullRangeReadProvider {
    fn identity(&self) -> unica_code_intelligence::model::ProviderIdentity {
        unica_code_intelligence::model::ProviderId::SemanticFixture.identity()
    }

    fn capabilities(&self) -> &[unica_code_intelligence::model::ProviderCapability] {
        &[
            unica_code_intelligence::model::ProviderCapability::Search,
            unica_code_intelligence::model::ProviderCapability::Outline,
        ]
    }

    fn search(
        &self,
        _request: &SearchRequest,
        _context: &unica_code_intelligence::model::CodeIntelligenceContext,
        _deadline: ProviderDeadline,
        _cancellation: &CancellationToken,
    ) -> unica_code_intelligence::model::ProviderSearchSection {
        unica_code_intelligence::model::ProviderSearchSection::complete(
            unica_code_intelligence::model::ProviderId::SemanticFixture.identity(),
            unica_code_intelligence::model::SearchRanking::Provider,
            unica_code_intelligence::model::SearchOrdering::Provider,
            Vec::new(),
            Vec::new(),
        )
        .unwrap()
    }

    fn read(
        &self,
        request: &CodeIntelligenceReadRequest,
        _context: &unica_code_intelligence::model::CodeIntelligenceContext,
        _deadline: ProviderDeadline,
        _cancellation: &CancellationToken,
    ) -> Result<unica_code_intelligence::model::ProviderReadOutcome, String> {
        let data = match request {
            CodeIntelligenceReadRequest::Outline { path, .. } => {
                unica_code_intelligence::model::CodeIntelligenceReadData::Outline(
                    unica_bsl::outline::CodeOutlineResult {
                        module: path.clone(),
                        identity: Default::default(),
                        totals: unica_bsl::outline::CodeOutlineTotals {
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
            // Этот двойник отвечает за навигацию индекса; граф вызовов —
            // предмет анализатора BSL, и подставлять за него пустоту
            // значило бы учить проверку неправде.
            CodeIntelligenceReadRequest::CallGraph { .. } => {
                return Err("the navigation stub answers no call graph".to_string())
            }
        };
        Ok(unica_code_intelligence::model::ProviderReadOutcome {
            provider: unica_code_intelligence::model::ProviderId::SemanticFixture.identity(),
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
        unica_configuration::operational::OperationalConfig,
        unica_configuration::operational::OperationalConfigDiagnostic,
    > {
        self.load_calls.fetch_add(1, Ordering::SeqCst);
        if let Some(cancellation) = &self.cancellation_on_load {
            cancellation.cancel();
        }
        if self.fail_load {
            return Err(
                unica_configuration::operational::OperationalConfigDiagnostic::new(
                    unica_configuration::operational::OperationalConfigDiagnosticCode::InvalidToml,
                    unica_configuration::operational::OperationalConfigDiagnosticSource::Shared,
                    "$",
                ),
            );
        }
        if self.full_range_workspace.is_some() {
            let mut layer = unica_configuration::operational::OperationalConfigLayer::default();
            layer.set_timeout_seconds(
                unica_configuration::operational::OperationalConfigField::ProviderRead,
                i64::MAX,
                unica_configuration::operational::OperationalConfigDiagnosticSource::Shared,
            )?;
            return unica_configuration::operational::OperationalConfig::from_layers(
                Some(&layer),
                None,
            );
        }
        Ok(unica_configuration::operational::OperationalConfig::compiled_defaults())
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
        if matches!(spec.handler, ToolHandler::Diagnostics) {
            return Ok(ports::PreparedToolInvocation {
                format_guard: None,
                handler: Some(ports::HandlerOutcome::with_data(
                    AdapterOutcome::ok("prepared diagnostics"),
                    json!({"state": "completed"}),
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
    ) -> Result<unica_code_intelligence::model::CodeIntelligenceContext, String> {
        self.code_context_calls.fetch_add(1, Ordering::SeqCst);
        if self.full_range_workspace.is_some() {
            return Ok(
                unica_code_intelligence::model::CodeIntelligenceContext::new(
                    context.clone(),
                    unica_source_contract::workspace::ResolvedSourceRoot {
                        source_set: Some("main".to_string()),
                        path: context.workspace_root.join("src"),
                    },
                    std::sync::Arc::new(unica_workspace::discovery::ProjectDiscoveryReader::new(
                        &context.workspace_root,
                    )),
                ),
            );
        }
        Err("code intelligence context should not be resolved in this test".to_string())
    }

    fn resolve_code_search_context(
        &self,
        context: &WorkspaceContext,
        args: &Map<String, Value>,
    ) -> Result<
        (
            unica_code_intelligence::model::CodeIntelligenceContext,
            unica_code_intelligence::model::CodeSearchScope,
        ),
        String,
    > {
        let provider_context = self.resolve_code_intelligence_context(context, args)?;
        let scope = unica_code_intelligence::model::CodeSearchScope::all(
            "main".to_string(),
            provider_context.source_root.path.clone(),
            false,
        );
        Ok((provider_context.with_search_scope(scope.clone()), scope))
    }

    fn normalize_code_intelligence_read_request(
        &self,
        request: CodeIntelligenceReadRequest,
        _context: &unica_code_intelligence::model::CodeIntelligenceContext,
    ) -> Result<CodeIntelligenceReadRequest, String> {
        if self.full_range_workspace.is_some() {
            return Ok(request);
        }
        Err("code intelligence request should not be normalized in this test".to_string())
    }

    fn code_intelligence_registry(
        &self,
    ) -> Result<unica_code_intelligence::model::CodeIntelligenceRegistry, String> {
        if self.full_range_workspace.is_some() {
            return unica_code_intelligence::model::CodeIntelligenceRegistry::new(vec![Arc::new(
                FullRangeReadProvider,
            )]);
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
        operational_config: Option<&unica_configuration::operational::OperationalConfig>,
        _cancellation: &CancellationToken,
    ) -> Result<ports::HandlerOutcome, String> {
        self.handler_calls.fetch_add(1, Ordering::SeqCst);
        *self.observed_analyze_timeout.lock().unwrap() =
            operational_config.and_then(|config| config.code_diagnostics().analyze_timeout());
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
    fn notify_committed_changes(
        &self,
        context: &WorkspaceContext,
        events: &[unica_source_contract::events::DomainEvent],
        _cancellation: &CancellationToken,
        _deadline: unica_work_control::operation_deadline::OperationDeadline,
    ) {
        self.notify_invalidation(context, events);
    }

    fn notify_invalidation(&self, _context: &WorkspaceContext, _events: &[DomainEvent]) {}
}

/// Порты, запоминающие порядок: доставка движка должна случиться до того,
/// как обработчик пойдёт его искать.
#[derive(Default)]
struct DeliveryRecordingPorts {
    steps: std::sync::Mutex<Vec<&'static str>>,
    missing: Option<unica_artifact_delivery::engine::MissingEngine>,
    delivery: unica_artifact_delivery::shared::EngineDeliveryState,
}

impl ports::ApplicationPorts for DeliveryRecordingPorts {
    fn discover_workspace(
        &self,
        requested_cwd: Option<PathBuf>,
    ) -> Result<WorkspaceContext, String> {
        let cwd = requested_cwd.unwrap_or_else(|| PathBuf::from("/workspace"));
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

    fn missing_engine(
        &self,
        _spec: ToolSpec,
        _cwd: Option<&std::path::Path>,
    ) -> Option<unica_artifact_delivery::engine::MissingEngine> {
        self.missing.clone()
    }

    fn deliver_engine_if_missing(
        &self,
        _spec: ToolSpec,
        _context: &WorkspaceContext,
        _cancellation: &CancellationToken,
        _progress: &dyn ProgressSink,
    ) -> unica_artifact_delivery::shared::EngineDeliveryState {
        self.steps.lock().expect("steps").push("delivery");
        self.delivery.clone()
    }

    fn invoke_handler_with_operational_config(
        &self,
        _spec: ToolSpec,
        _args: &Map<String, Value>,
        _context: &WorkspaceContext,
        _mode: InvocationMode,
        _operational_config: Option<&unica_configuration::operational::OperationalConfig>,
        _cancellation: &CancellationToken,
    ) -> Result<ports::HandlerOutcome, String> {
        self.steps.lock().expect("steps").push("handler");
        let outcome = AdapterOutcome::ok("handled");
        Ok(if _spec.result_contract == ResultContract::Typed {
            ports::HandlerOutcome::with_data(outcome, json!({"fixture": true}))
        } else {
            ports::HandlerOutcome::plain(outcome)
        })
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
        args: &Map<String, Value>,
        context: &WorkspaceContext,
        mode: InvocationMode,
        cancellation: &CancellationToken,
    ) -> Result<ports::HandlerOutcome, String> {
        self.invoke_handler_with_operational_config(spec, args, context, mode, None, cancellation)
    }

    fn cache_report(
        &self,
        context: &WorkspaceContext,
        _events: &[DomainEvent],
        mode: InvocationMode,
        _cache_access: CacheAccess,
    ) -> Result<CacheReport, String> {
        Ok(CacheReport {
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
        })
    }
    fn notify_committed_changes(
        &self,
        context: &WorkspaceContext,
        events: &[unica_source_contract::events::DomainEvent],
        _cancellation: &CancellationToken,
        _deadline: unica_work_control::operation_deadline::OperationDeadline,
    ) {
        self.notify_invalidation(context, events);
    }

    fn notify_invalidation(&self, _context: &WorkspaceContext, _events: &[DomainEvent]) {}
}

fn runtime_execute_args(dry_run: bool) -> Map<String, Value> {
    let mut args = Map::new();
    args.insert("cwd".to_string(), json!("/workspace"));
    args.insert("dryRun".to_string(), json!(dry_run));
    args.insert("operation".to_string(), json!("dump"));
    args.insert("mode".to_string(), json!("incremental"));
    args
}

#[test]
fn an_admission_refusal_names_the_missing_engine_too() {
    // Отказ допуска не должен маскировать отсутствие бинаря.
    // Сам маршрут из дефекта закрыт раньше: теперь
    // классифицированные операции исполняются с названным риском, а
    // неклассифицированную аргументы до допуска не доносят. Отказ остаётся
    // достижим изнутри, и вторая причина в нём названа.
    let missing = unica_artifact_delivery::engine::MissingEngine::new(
        "v8-runner",
        "darwin-arm64",
        "/plugins/unica/bin/darwin-arm64/v8-runner",
        Some("0.5.1".to_string()),
        unica_artifact_delivery::engine::InstallMode::Source,
    );
    let spec = tools()
        .into_iter()
        .find(|tool| tool.name == "unica.runtime.execute")
        .expect("runtime.execute");

    let result = runtime_receipt_admission_result(
        spec,
        runtime_admission::RuntimeAdmissionFailure {
            code: "runtime_operation_unbounded",
            message: "operation `syntax` has no bounded terminal-receipt contract".to_string(),
        },
        Some(missing),
    );

    assert!(!result.ok);
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.contains("runtime_operation_unbounded")),
        "причина допуска названа: {:?}",
        result.errors
    );
    let missing = result
        .diagnostics
        .as_ref()
        .and_then(|diagnostics| diagnostics.get("missingEngine"))
        .expect("отсутствие движка названо полями");
    assert_eq!(missing["code"], "bundled_tool_missing");
    assert_eq!(missing["stage"], "delivery");
    assert!(
        missing["nextStep"]
            .as_str()
            .is_some_and(|step| step.contains("build-unica-tools.py")),
        "следующий шаг назван: {missing}"
    );
    assert_eq!(
        result
            .diagnostics
            .as_ref()
            .and_then(|diagnostics| diagnostics.get("missingEngine"))
            .and_then(|missing| missing.get("pinnedVersion"))
            .and_then(Value::as_str),
        Some("0.5.1"),
        "поля отказа машиночитаемы: {:?}",
        result.diagnostics
    );
}

#[test]
fn a_missing_engine_is_delivered_before_the_handler_looks_for_it() {
    // Вызов, которому нужен ещё не доставленный движок, не отказывает: он
    // дожидается доставки и исполняется.
    let ports = Arc::new(DeliveryRecordingPorts::default());
    let app = UnicaApplication::with_ports(ports.clone());

    let mut args = Map::new();
    args.insert("cwd".to_string(), json!("/workspace"));
    args.insert("mode".to_string(), json!("status"));
    call_tool_after_runtime_admission_for_downstream_test(&app, "unica.code.graph", &args)
        .expect("call");

    assert_eq!(
        *ports.steps.lock().expect("steps"),
        vec!["delivery", "handler"]
    );
}

#[test]
fn a_call_that_outran_the_window_answers_with_state_instead_of_working_blindly() {
    // Срез хоста уносит наш ответ целиком, поэтому отвечаем сами: успешный
    // результат с состоянием, а обработчик не зовётся — работать нечем.
    let ports = Arc::new(DeliveryRecordingPorts {
        delivery: unica_artifact_delivery::shared::EngineDeliveryState::Working {
            artifact: "bsl-analyzer".to_owned(),
            received: 12,
            total: Some(69),
            poll_interval_ms: Some(9_000),
        },
        ..Default::default()
    });
    let app = UnicaApplication::with_ports(ports.clone());
    let mut args = Map::new();
    args.insert("cwd".to_string(), json!("/workspace"));
    args.insert("mode".to_string(), json!("status"));

    let result =
        call_tool_after_runtime_admission_for_downstream_test(&app, "unica.code.graph", &args)
            .expect("call");

    assert!(result.ok, "идущая работа — состояние, а не неудача");
    assert_eq!(
        result.work.as_ref().map(|work| work.status),
        Some(unica_work_control::long_work::WorkStatus::Working)
    );
    assert_eq!(
        result.work.as_ref().and_then(|work| work.poll_interval_ms),
        Some(9_000)
    );
    assert_eq!(
        result
            .work
            .as_ref()
            .map(|work| work.status_message.as_str()),
        Some("delivering bsl-analyzer: 12 of 69 bytes on disk")
    );
    assert_eq!(
        *ports.steps.lock().expect("steps"),
        vec!["delivery"],
        "обработчик не зовётся: движка ещё нет"
    );
}

#[test]
fn a_delivery_that_failed_is_not_dressed_up_as_progress() {
    // `working` — состояние, а не неудача, и потому `ok`. Отказ доставки —
    // неудача, и притворяться идущей работой ему нечем.
    let ports = Arc::new(DeliveryRecordingPorts {
        delivery: unica_artifact_delivery::shared::EngineDeliveryState::Failed {
            artifact: "bsl-analyzer".to_owned(),
            failure: Arc::new(unica_artifact_delivery::shared::DeliveryFailure::new(
                unica_artifact_delivery::shared::DeliveryFailureClass::Network,
                "connection refused",
            )),
        },
        ..Default::default()
    });
    let app = UnicaApplication::with_ports(ports.clone());
    let mut args = Map::new();
    args.insert("cwd".to_string(), json!("/workspace"));
    args.insert("mode".to_string(), json!("status"));

    let result =
        call_tool_after_runtime_admission_for_downstream_test(&app, "unica.code.graph", &args)
            .expect("call");

    assert!(!result.ok, "отказ доставки — неудача");
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.contains("connection refused")),
        "причина названа: {:?}",
        result.errors
    );
    assert!(result
        .errors
        .iter()
        .any(|error| { error == "delivery of bsl-analyzer failed: connection refused" }));
    assert_eq!(
        result.work.as_ref().map(|work| work.status),
        Some(unica_work_control::long_work::WorkStatus::Failed)
    );
}

#[test]
fn a_preview_does_not_fetch_an_engine_it_will_not_run() {
    // Предпросмотр ничего не исполняет. Скачать ради него семьдесят
    // мегабайт значит заплатить за работу, которой не будет.
    let ports = Arc::new(DeliveryRecordingPorts::default());
    let app = UnicaApplication::with_ports(ports.clone());

    call_tool_after_runtime_admission_for_downstream_test(
        &app,
        "unica.runtime.execute",
        &runtime_execute_args(true),
    )
    .expect("call");

    assert_eq!(*ports.steps.lock().expect("steps"), vec!["handler"]);
}

#[test]
fn operational_config_is_loaded_once_only_for_affected_calls() {
    let ports = Arc::new(OperationalConfigRecordingPorts::default());
    let app = UnicaApplication::with_ports(ports.clone());

    let mut status = Map::new();
    status.insert("action".to_string(), json!("status"));
    status.insert("sourceSet".to_string(), json!("main"));
    app.call_tool("unica.code.diagnostics", &status).unwrap();
    assert_eq!(ports.load_calls.load(Ordering::SeqCst), 0);

    let mut analyze = Map::new();
    analyze.insert("action".to_string(), json!("analyze"));
    analyze.insert("sourceSet".to_string(), json!("main"));
    analyze.insert("timeoutSeconds".to_string(), json!(900));
    app.call_tool("unica.code.diagnostics", &analyze).unwrap();
    assert_eq!(ports.load_calls.load(Ordering::SeqCst), 1);

    {
        let (tool_name, args) = (
            "unica.code.search",
            json!({"query": "needle", "sourceDir": "."}),
        );
        let error = app
            .call_tool(tool_name, args.as_object().unwrap())
            .unwrap_err();
        assert!(error.contains("should not be resolved"), "{error}");
    }
    assert_eq!(ports.load_calls.load(Ordering::SeqCst), 2);
    assert_eq!(ports.code_context_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn prepared_code_search_handler_wins_after_required_operational_config_resolution() {
    let ports = Arc::new(OperationalConfigRecordingPorts::with_prepared_code_search_handler());
    let app = UnicaApplication::with_ports(ports.clone());

    let result = app
        .call_tool(
            "unica.code.search",
            json!({"query": "needle", "sourceDir": "."})
                .as_object()
                .unwrap(),
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
        (
            "unica.code.search",
            json!({"query": "needle", "sourceDir": "."}),
        ),
        (
            "unica.code.diagnostics",
            json!({"action": "analyze", "sourceSet": "main", "timeoutSeconds": 900}),
        ),
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
            json!({"query": "needle", "sourceDir": "."})
                .as_object()
                .unwrap(),
            token,
        )
        .expect_err("cancellation must win over invalid config");

    assert!(
        error.starts_with(unica_work_control::cancellation::CANCELLED_PREFIX),
        "{error}"
    );
}

#[test]
fn public_search_accepts_full_positive_i64_config_budget() {
    let workspace = std::env::temp_dir().join(format!(
        "unica-public-full-range-read-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let app = UnicaApplication::with_ports(Arc::new(
        OperationalConfigRecordingPorts::with_full_range_code_provider(workspace.clone()),
    ));

    let tool_name = "unica.code.search";
    let result = app
        .call_tool(
            tool_name,
            json!({"query": "Needle", "sourceSet": "main"})
                .as_object()
                .unwrap(),
        )
        .expect("valid full-range config must not panic public dispatch");
    assert!(
        result.ok,
        "{tool_name} must answer under a full-range budget"
    );
}

#[test]
fn xml_dsl_tools_route_to_parity_covered_native_handlers() {
    // `unica.cf.info` left the parity stand when it started answering with
    // typed data: there is no prose left to compare (ADR-0023).
    const PARITY_COVERED_TOOLS: &[&str] = &["unica.form.compile"];
    const REPO_OWNED_NATIVE_TOOLS: &[&str] = &[];
    // A tool that answers with typed data has no prose left for the parity
    // stand to compare, so it is covered by its own crate tests instead
    // (ADR-0023).
    const TYPED_RESULT_TOOLS: &[&str] = &[
        "unica.mxl.info",
        "unica.role.edit",
        "unica.meta.add",
        "unica.meta.edit",
        "unica.interface.edit",
        "unica.cf.edit",
        "unica.cfe.borrow",
        "unica.subsystem.edit",
        "unica.form.edit",
        "unica.meta.info",
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
                    "unica.meta.info" | "unica.meta.add" | "unica.meta.edit" | "unica.meta.remove"
                ),
                "{} unexpectedly routes through the typed Metadata handler",
                tool.name
            ),
            _ => panic!("{} routes through unexpected handler", tool.name),
        }
    }
}

/// DEC.2026-08-21.SOURCE-READ-ONLY-SURFACE: чтение исходников остаётся
/// read-only. Предметом правила были снятые `unica.source.*`; клятва
/// сохраняется на читателях, которые ещё стоят в реестре, и на отсутствии
/// пишущего входа рядом с ними.
#[test]
fn source_resource_tools_are_read_only_and_have_no_cache_or_event_effects() {
    let readers = tools()
        .into_iter()
        .filter(|tool| !tool.execution.is_mutating())
        .collect::<Vec<_>>();
    assert!(!readers.is_empty(), "the registry still declares readers");

    for tool in readers {
        assert!(
            tool.cache_access.writes.is_empty(),
            "{} is a reader and must not invalidate cache",
            tool.name
        );
        assert!(
            !matches!(
                tool.handler,
                ToolHandler::NativeOperation { event: Some(_), .. }
            ),
            "{} is a reader and must not publish a domain event",
            tool.name
        );
    }

    // Правка BSL принадлежит `unica.code.patch`, который меняет выбранный
    // метод или якорь, а не переписывает модуль целиком: пишущего входа
    // рядом с чтением исходников нет.
    assert!(tools()
        .into_iter()
        .all(|tool| tool.name != "unica.source.apply"));
}

#[test]
fn provider_neutral_tools_use_typed_code_intelligence_handlers() {
    let expected = [("unica.code.search", CodeIntelligenceOperation::Search)];

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
            work: None,
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
fn preview_result_contains_cache_shape_without_second_call() {
    let app = UnicaApplication::with_ports(Arc::new(FixedOutcomePorts {
        outcome: AdapterOutcome::ok("planned preview"),
        data: None,
    }));
    let result = app.call_tool("unica.build.load", &Map::new()).unwrap();
    let wire = serde_json::to_value(&result).unwrap();

    assert_eq!(result.cache.mode, "dry-run");
    assert_eq!(wire["cache"]["mode"], "dry-run");
    assert_eq!(
        wire["cache"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from([
            "events",
            "fresh",
            "invalidated",
            "lazy_rebuilt",
            "mode",
            "refreshed",
            "root",
            "stale",
            "workspace_epoch",
        ])
    );
    for field in [
        "events",
        "invalidated",
        "refreshed",
        "lazy_rebuilt",
        "stale",
        "fresh",
    ] {
        assert!(wire["cache"][field].is_array(), "missing cache.{field}");
    }
}

#[test]
fn runtime_execute_defaults_to_dry_run_and_maps_cache_event_by_operation() {
    let mut args = Map::new();
    args.insert("operation".to_string(), Value::String("dump".to_string()));
    let mut outcome = AdapterOutcome::ok("dry run: runtime adapter invoked");
    outcome.command = Some(vec!["v8-runner".to_string(), "dump".to_string()]);

    let result = UnicaApplication::with_ports(Arc::new(FixedOutcomePorts {
        outcome,
        data: None,
    }))
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
        fn notify_committed_changes(
            &self,
            context: &WorkspaceContext,
            events: &[unica_source_contract::events::DomainEvent],
            _cancellation: &CancellationToken,
            _deadline: unica_work_control::operation_deadline::OperationDeadline,
        ) {
            self.notify_invalidation(context, events);
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

    let result = call_tool_after_runtime_admission_for_test(
        &UnicaApplication::with_ports(Arc::new(FixedOutcomePorts {
            outcome: AdapterOutcome::ok("runtime adapter must not be invoked"),
            data: None,
        })),
        "unica.runtime.execute",
        &args,
    )
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

        let result =
            call_tool_after_runtime_admission_for_downstream_test(&app, tool, &args).unwrap();
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

        let result =
            call_tool_after_runtime_admission_for_downstream_test(&app, tool, &args).unwrap();
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

    let result = call_tool_with_runtime_admission(
        tools()
            .into_iter()
            .find(|tool| tool.name == "unica.runtime.execute")
            .unwrap(),
        &args,
        app.ports.as_ref(),
        &cancellation,
        ProviderDeadline::from_budget(PUBLIC_INVOCATION_DEADLINE),
        &NoopProgressSink,
        RuntimeAdmissionPolicy::AlreadyAdmittedForDownstreamContractTest,
        &deferred_delivery::DeferredDelivery::default(),
    )
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

    for (tool, include_operation) in [("unica.build.dump", false), ("unica.runtime.execute", true)]
    {
        let mut args = Map::new();
        args.insert("cwd".to_string(), json!(root));
        args.insert("dryRun".to_string(), json!(false));
        args.insert("mode".to_string(), json!("full"));
        if include_operation {
            args.insert("operation".to_string(), json!("dump"));
        }

        let result =
            call_tool_after_runtime_admission_for_downstream_test(&app, tool, &args).unwrap();
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
    assert!(!errors.contains("unica.build.dump"), "{job:?}");
    assert!(!errors.contains("unica.runtime.execute"), "{job:?}");
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
        let result =
            call_tool_after_runtime_admission_for_downstream_test(&app, tool, &convert).unwrap();
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
            let result =
                call_tool_after_runtime_admission_for_downstream_test(&app, tool, &launch).unwrap();
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

        let applied =
            call_tool_after_runtime_admission_for_downstream_test(&app, tool, &args).unwrap();
        assert!(applied.ok, "{tool}: {applied:?}");
        assert_eq!(applied.summary, "runtime adapter invoked");
    }

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn runtime_job_start_defaults_to_dry_run_without_runtime_cache_invalidation() {
    let mut args = Map::new();
    args.insert("operation".to_string(), Value::String("dump".to_string()));

    let result = UnicaApplication::with_ports(Arc::new(FixedOutcomePorts {
        outcome: AdapterOutcome::ok("dry run: runtime job adapter invoked"),
        data: None,
    }))
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

    let result = UnicaApplication::with_ports(Arc::new(FixedOutcomePorts {
        outcome: AdapterOutcome::ok("dry run: runtime adapter invoked"),
        data: None,
    }))
    .call_tool("unica.runtime.execute", &args)
    .unwrap();

    assert!(result.ok);
    assert!(result.cache.events.is_empty());
    assert_eq!(result.cache.mode, "read");
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
    let impact = unica_workspace::cache::CacheImpact::from_events(&[DomainEvent::new(
        DomainEventKind::RoleChanged,
        "main + Role.Demo",
    )]);
    assert!(impact.invalidated.contains("rights_graph"));
    assert!(impact.eager_refresh.contains("rights_graph"));
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
                "internal v8-runner runtime adapter exited with status exit status: 1".to_string(),
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
            warnings: vec!["internal v8-runner runtime adapter failed to spawn process".to_string()],
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
fn runtime_execute_terminal_result_is_returned_in_original_call() {
    let root = test_workspace_root("runtime-bounded-success-diagnostics");
    let result = call_runtime_with_outcome_and_data(
        &root,
        AdapterOutcome {
            ok: true,
            summary: "unica.runtime.execute completed through internal v8-runner runtime adapter"
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
fn reader_schemas_never_publish_dry_run_and_mutations_keep_it() {
    for tool in tools() {
        let schema = input_schema_for_tool(&tool);
        for properties in object_schema_property_maps(&schema) {
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
pub(crate) fn mutating_native_support_guard_coverage_is_explicit() {
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
                            matches!(operation, "code-patch" | "role-edit"),
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
                    }
                    SupportGuardPolicy::ObjectName { requirement } => {
                        assert!(
                            matches!(operation, "form-remove"),
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
            "form-compile",
            "form-edit",
            "interface-edit",
            "role-edit",
            "subsystem-edit",
        ],
        "guarded platform-XML mutations changed without updating the support contract"
    );
    let expected_exemptions = [("cfe-borrow", "writes only into an extension")];
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
fn mutating_tools_have_typed_cache_event_or_explicit_non_cache_effect() {
    let mut explicit_non_cache_effect = Vec::new();
    for tool in tools()
        .into_iter()
        .filter(|tool| tool.execution.is_mutating())
    {
        match tool.handler {
            ToolHandler::NativeOperation { event, .. } => {
                assert!(event.is_some(), "{} has no typed event", tool.name);
            }
            ToolHandler::Metadata { operation } => {
                assert!(
                    !matches!(operation, unica_metadata::request::MetadataOperation::Info),
                    "{} uses a read-only metadata operation as a mutation",
                    tool.name
                );
            }
            ToolHandler::BuildRuntime { event: Some(_), .. } | ToolHandler::RuntimeAdapter => {}
            ToolHandler::BuildRuntime { event: None, .. } | ToolHandler::RuntimeJob { .. } => {
                assert!(tool.cache_access.writes.is_empty(), "{}", tool.name);
                explicit_non_cache_effect.push(tool.name);
            }
            _ => panic!("{} has an unclassified mutating handler", tool.name),
        }
    }
    explicit_non_cache_effect.sort_unstable();
    assert_eq!(
        explicit_non_cache_effect,
        [
            "unica.build.make",
            "unica.build.run",
            "unica.runtime.job.start",
        ]
    );
}

#[test]
fn mutating_platform_xml_operations_declare_effective_format_paths() {
    let expected = [
        ("code-patch", &[][..], "HandlerResolved"),
        ("role-edit", &[][..], "HandlerResolved"),
        (
            "cf-edit",
            &["ConfigPath", "configPath", "Path", "path"][..],
            "HandlerResolved",
        ),
        (
            "cfe-borrow",
            &["ExtensionPath", "ConfigPath", "extensionPath", "configPath"][..],
            "DeclaredArgs",
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
        (
            "interface-edit",
            &["CIPath", "ciPath", "path", "Path"][..],
            "DeclaredArgs",
        ),
        (
            "subsystem-edit",
            &["SubsystemPath", "subsystemPath", "Path", "path"][..],
            "HandlerResolved",
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
}

#[test]
fn source_format_sensitive_descriptors_name_source_paths() {
    for operation in ["cf-info", "form-edit", "role-info"] {
        let descriptor = operation_descriptors::native_operation_descriptor(operation).unwrap();
        assert!(
            !descriptor.source_path_args.is_empty(),
            "{operation} should declare source path args for source-set format validation"
        );
    }
}

#[test]
fn native_descriptors_expose_required_adapter_arguments() {
    let required_by_operation = [(
        "role-edit",
        &["sourceSet", "metadataPath", "operations"][..],
    )];

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
        fn notify_committed_changes(
            &self,
            context: &WorkspaceContext,
            events: &[unica_source_contract::events::DomainEvent],
            _cancellation: &CancellationToken,
            _deadline: unica_work_control::operation_deadline::OperationDeadline,
        ) {
            self.notify_invalidation(context, events);
        }

        fn notify_invalidation(&self, _context: &WorkspaceContext, _events: &[DomainEvent]) {}
    }

    let cases = [
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
        fn notify_committed_changes(
            &self,
            context: &WorkspaceContext,
            events: &[unica_source_contract::events::DomainEvent],
            _cancellation: &CancellationToken,
            _deadline: unica_work_control::operation_deadline::OperationDeadline,
        ) {
            self.notify_invalidation(context, events);
        }

        fn notify_invalidation(&self, _context: &WorkspaceContext, events: &[DomainEvent]) {
            self.invalidated
                .lock()
                .unwrap()
                .extend(events.iter().map(|event| format!("{:?}", event.kind)));
        }
    }

    let root = std::env::temp_dir().join(format!(
        "unica-ports-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
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
        fn notify_committed_changes(
            &self,
            context: &WorkspaceContext,
            events: &[unica_source_contract::events::DomainEvent],
            _cancellation: &CancellationToken,
            _deadline: unica_work_control::operation_deadline::OperationDeadline,
        ) {
            self.notify_invalidation(context, events);
        }

        fn notify_invalidation(&self, _context: &WorkspaceContext, _events: &[DomainEvent]) {}
    }

    let root = std::env::temp_dir().join(format!(
        "unica-pre-recorded-cache-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
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
                "failed to inspect /private/provider/workspace/src/Configuration.xml".to_string(),
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
        Err(
            "failed to inspect /private/provider/workspace/src/XDTOPackages/Sample/Ext/Package.bin"
                .to_string(),
        )
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
    fn notify_committed_changes(
        &self,
        context: &WorkspaceContext,
        events: &[unica_source_contract::events::DomainEvent],
        _cancellation: &CancellationToken,
        _deadline: unica_work_control::operation_deadline::OperationDeadline,
    ) {
        self.notify_invalidation(context, events);
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
    fn notify_committed_changes(
        &self,
        context: &WorkspaceContext,
        events: &[unica_source_contract::events::DomainEvent],
        _cancellation: &CancellationToken,
        _deadline: unica_work_control::operation_deadline::OperationDeadline,
    ) {
        self.notify_invalidation(context, events);
    }

    fn notify_invalidation(&self, _context: &WorkspaceContext, _events: &[DomainEvent]) {}
}

/// Живой типизированный читатель для контрактных проб: `project.map`
/// снят вместе с остальными читателями партии 3, а типизированное чтение
/// метаданных идёт мимо `invoke_handler`, который подменяют эти порты.
fn typed_reader_args() -> Map<String, Value> {
    Map::from_iter([(
        "TemplatePath".to_string(),
        Value::String("src/Reports/Sales/Templates/Sheet/Ext/Template.xml".to_string()),
    )])
}

#[test]
fn successful_typed_reader_without_data_fails_closed() {
    let error = UnicaApplication::with_ports(Arc::new(FixedOutcomePorts {
        outcome: AdapterOutcome::ok("reader omitted its typed payload"),
        data: None,
    }))
    .call_tool("unica.mxl.info", &typed_reader_args())
    .expect_err("successful typed reader without data must fail closed");

    assert_eq!(
        error,
        "typed_result_missing: unica.mxl.info returned ok without OperationResult.data"
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
    .call_tool("unica.mxl.info", &typed_reader_args())
    .expect_err("successful typed reader must not duplicate data in stdout");

    assert_eq!(
        error,
        "typed_result_textual: unica.mxl.info returned ok with a stdout duplicate"
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
    .call_tool("unica.mxl.info", &typed_reader_args())
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
    call_tool_after_runtime_admission_for_test(
        &UnicaApplication::with_ports(Arc::new(FixedOutcomePorts {
            outcome,
            data: None,
        })),
        "unica.runtime.execute",
        &args,
    )
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
    call_tool_after_runtime_admission_for_test(
        &UnicaApplication::with_ports(Arc::new(FixedOutcomePorts { outcome, data })),
        "unica.runtime.execute",
        &args,
    )
    .unwrap()
}

fn test_workspace_root(prefix: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "{prefix}-{}-{nanos}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&root).unwrap();
    root
}

#[test]
fn tool_specs_match_reviewed_result_contracts() {
    let review: Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/v013/tool-surface-review.json"
    ))
    .expect("tool-surface review is valid JSON");
    let review = review
        .as_object()
        .expect("tool-surface review is a tool-name object");
    let catalog = crate::application::v13::tool_catalog::catalog_for(
        crate::application::tool_contracts::SurfaceRelease::V13,
    )
    .expect("canonical v0.13 catalog exists");
    let registered = catalog
        .tools
        .iter()
        .map(|tool| format!("unica.{}", tool.name))
        .chain(
            crate::application::v13::task_tools::compatibility_tool_contracts()
                .into_iter()
                .map(|tool| format!("unica.{}", tool.name)),
        )
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(review.len(), registered.len());
    assert_eq!(
        review
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>(),
        registered
    );

    for name in registered {
        let entry = review
            .get(&name)
            .unwrap_or_else(|| panic!("{name} has no tool-surface review"));
        assert_eq!(entry["scope"], "in", "{name}");
        assert_eq!(entry["result"]["contract"], "typed", "{name}");
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
