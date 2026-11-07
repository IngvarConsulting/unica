use crate::application::ports::{FormatGuardCheck, FormatGuardError, SupportGuardCheck};
use crate::application::*;
use crate::composition::testing::{
    create_file_link_fixture_for_test, file_identity_for_test, prepare_file_for_removal,
    set_unix_mode_for_test, unix_mode_for_test, with_publication_lock_contention_signal,
    with_publication_lock_pause, CompileTransaction, FileLinkFixtureOutcome,
};
use serde_json::{json, Value};
use std::path::PathBuf;
use unica_code_intelligence::model::CodeIntelligenceReadRequest;
use unica_code_intelligence::model::SearchRequest;
use unica_execution_contract::adapter_outcome::AdapterOutcome;
use unica_navigation::deferred_delivery;
use unica_runner::admission as runtime_admission;
use unica_source_contract::events::{DomainEvent, DomainEventKind};
use unica_source_contract::workspace::WorkspaceContext;
use unica_work_control::cancellation::CancellationToken;
use unica_work_control::progress::{NoopProgressSink, ProgressSink};
use unica_work_control::provider_deadline::ProviderDeadline;
use unica_workspace::cache::{CacheAccess, CacheReport};

use serde_json::Map;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::sync::{mpsc, Arc, Barrier};
use std::thread;
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

fn normalized_path(path: &std::path::Path) -> std::path::PathBuf {
    unica_workspace::test_support::canonical_path(path)
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

fn call_public_tool_from_workspace(
    workspace: &std::path::Path,
    name: &str,
    args: &Map<String, Value>,
) -> Result<OperationResult, String> {
    let _cwd = unica_workspace::test_support::ProcessCwdGuard::enter(workspace)?;
    crate::composition::testing::application().call_tool(name, args)
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

fn path_text(path: &std::path::Path) -> String {
    path.display().to_string().replace('\\', "/")
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
    // #549 просил, чтобы отказ допуска не маскировал отсутствие бинаря.
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

        let mutation = crate::composition::testing::application()
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
fn numeric_equivalent_noncanonical_format_warns_on_read_and_blocks_public_mutator() {
    for (index, raw) in ["2.20.0", "02.20", "2.020"].into_iter().enumerate() {
        let (root, workspace, config_path) = cf_edit_mutation_workspace(
            &format!("unica-noncanonical-format-{index}"),
            support_test_configuration_xml("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa")
                .replacen(r#"version="2.20""#, &format!(r#"version="{raw}""#), 1)
                .as_bytes(),
        );
        let before = std::fs::read(&config_path).unwrap();

        let mutation = crate::composition::testing::application()
            .call_tool(
                "unica.cf.edit",
                &cf_edit_args(&workspace, "modify-property", "Version=2.0"),
            )
            .unwrap();

        assert!(!mutation.ok, "{raw}: {mutation:?}");
        let mutation_diagnostic = &mutation.diagnostics.as_ref().unwrap()["formatCompatibility"];
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
    let app = crate::composition::testing::application();
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
    let app = crate::composition::testing::application();
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

    let result = crate::composition::testing::application()
        .call_tool("unica.form.edit", &args)
        .unwrap();

    assert!(!result.ok, "{result:?}");
    assert!(result.data.is_none(), "{result:?}");
    assert!(result.cache.events.is_empty(), "{result:?}");
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.contains("AlreadyInvalid") && error.contains("missing companion")),
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
    let app = crate::composition::testing::application();

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
pub(crate) fn code_patch_locked_support_blocks_preview_and_apply_before_handler() {
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
    let mut args = json!({
        "cwd": workspace,
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

    let mut results = Vec::new();
    for dry_run in [false, true] {
        args.insert("dryRun".to_string(), json!(dry_run));
        let result = crate::composition::testing::application()
            .call_tool("unica.code.patch", &args)
            .unwrap();

        assert!(!result.ok, "dryRun={dry_run}: {result:?}");
        assert!(result.summary.contains("support guard"));
        assert!(result.errors.join("\n").contains("на замке"));
        assert!(result.data.is_none());
        assert!(result.cache.events.is_empty());
        assert_eq!(std::fs::read(&module).unwrap(), before);
        results.push(result);
    }
    assert_support_guard_block_parity(&results[0], &results[1]);

    std::fs::remove_dir_all(root).unwrap();
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
        let result = call_public_tool_from_workspace(&workspace, "unica.role.edit", &args).unwrap();
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
    let applied = call_public_tool_from_workspace(&workspace, "unica.role.edit", &args).unwrap();
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

    let repeated = call_public_tool_from_workspace(&workspace, "unica.role.edit", &args).unwrap();
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
fn mutating_cf_edit_blocks_locked_configuration_directory_target() {
    let root = std::env::temp_dir().join(format!(
        "unica-cf-guard-dir-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
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

    let result = crate::composition::testing::application()
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
    let root = std::env::temp_dir().join(format!(
        "unica-cf-crlf-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
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

    let result = crate::composition::testing::application()
        .call_tool("unica.cf.edit", &args)
        .unwrap();

    assert!(result.ok, "{result:?}");
    let after = std::fs::read_to_string(&config_path).unwrap();
    assert!(after.contains("<Version>2.0</Version>"));
    assert!(!after.contains("&#13;"));

    let _ = std::fs::remove_dir_all(root);
}

fn cf_edit_args(workspace: &std::path::Path, operation: &str, value: &str) -> Map<String, Value> {
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

fn cf_edit_mutation_workspace(prefix: &str, configuration: &[u8]) -> (PathBuf, PathBuf, PathBuf) {
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

    let result = crate::composition::testing::application()
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

    let result = crate::composition::testing::application()
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

    let result = crate::composition::testing::application()
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

    let result = crate::composition::testing::application()
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
pub(crate) fn cf_edit_equal_serialized_result_is_a_public_noop_and_preserves_identity() {
    let before = cf_edit_configuration_bytes();
    let (root, workspace, config_path) =
        cf_edit_mutation_workspace("unica-cf-edit-equal-noop", &before);
    let identity = file_identity_for_test(&config_path).unwrap();

    let result = crate::composition::testing::application()
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
    assert_eq!(file_identity_for_test(&config_path).unwrap(), identity);
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
        .register_canonical_child(
            &config_path,
            "Role",
            "Reader",
            unica_source_designer::registration::cf_edit_add_child_object_text,
        )
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
            crate::composition::testing::application()
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
        let definition_path = workspace.join(format!("{sidecar_operation}-{child_operation}.json"));
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

        let result = crate::composition::testing::application()
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

        let result = crate::composition::testing::application()
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
        let (root, workspace, config_path) =
            cf_edit_mutation_workspace(&format!("unica-cf-edit-external-success-{case}"), &before);
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

        let result = crate::composition::testing::application()
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

    let result = crate::composition::testing::application()
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

    let result = crate::composition::testing::application()
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

    let result = crate::composition::testing::application()
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
    include_str!("../../../tests/fixtures/unica_mcp_script_parity/cf-validate/Configuration.xml")
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
        include_str!(
            "../../../tests/fixtures/unica_mcp_script_parity/cf-validate/Languages/Русский.xml"
        ),
    )
    .unwrap();
    if include_bot {
        std::fs::write(src.join("Bots/Assistant.xml"), "<MetaDataObject/>").unwrap();
    }
    (root, workspace, config_path)
}

#[test]
fn cf_edit_adds_removes_and_noops_bot_through_registry() {
    let (root, workspace, config_path) = bot_cf_workspace("unica-cf-bot-edit", false);
    let src = workspace.join("src");
    std::fs::write(src.join("Bots/Assistant.xml"), "<MetaDataObject/>").unwrap();
    let before = std::fs::read_to_string(&config_path).unwrap();

    let add = crate::composition::testing::application()
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

    let duplicate = crate::composition::testing::application()
        .call_tool(
            "unica.cf.edit",
            &cf_edit_args(&workspace, "add-childObject", "Bot.Assistant"),
        )
        .unwrap();
    assert!(duplicate.ok, "{duplicate:?}");
    assert!(duplicate.changes.is_empty(), "{duplicate:?}");
    assert!(duplicate.cache.events.is_empty(), "{duplicate:?}");
    assert_eq!(std::fs::read_to_string(&config_path).unwrap(), after_add);

    let remove = crate::composition::testing::application()
        .call_tool(
            "unica.cf.edit",
            &cf_edit_args(&workspace, "remove-childObject", "Bot.Assistant"),
        )
        .unwrap();
    assert!(remove.ok, "{remove:?}");
    assert_eq!(std::fs::read_to_string(&config_path).unwrap(), before);

    let missing = crate::composition::testing::application()
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

    let unknown = crate::composition::testing::application()
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
    let root = std::env::temp_dir().join(format!(
        "unica-cf-child-crlf-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
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

    let result = crate::composition::testing::application()
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
    let root = std::env::temp_dir().join(format!(
        "unica-cf-issue55-roundtrip-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
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

    let remove = crate::composition::testing::application()
        .call_tool(
            "unica.cf.edit",
            &cf_edit_args(&workspace, "remove-childObject", "Catalog.Валюты"),
        )
        .unwrap();
    assert!(remove.ok, "{remove:?}");

    let add = crate::composition::testing::application()
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

    let root = std::env::temp_dir().join(format!(
        "unica-cf-issue55-trailer-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
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

    let remove = crate::composition::testing::application()
        .call_tool(
            "unica.cf.edit",
            &cf_edit_args(&workspace, "remove-childObject", "Catalog.Валюты"),
        )
        .unwrap();
    assert!(remove.ok, "{remove:?}");
    let after_remove = std::fs::read_to_string(&config_path).unwrap();
    assert_eq!(trailer_after_root(&after_remove), "\r\n\r\n\r\n");

    let add = crate::composition::testing::application()
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
    let root = std::env::temp_dir().join(format!(
        "unica-cf-issue55-noop-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
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

    let result = crate::composition::testing::application()
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
fn tool_specs_match_reviewed_result_contracts() {
    let review: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/v013/tool-surface-review.json"
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
pub(crate) fn incompatible_format_blocks_before_native_handler() {
    let root = test_workspace_root("unica-application-format-guard");
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

    let result = crate::composition::testing::application()
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
pub(crate) fn public_metadata_mutators_refuse_old_and_new_profiles_without_side_effects() {
    for version in ["2.19", "2.21"] {
        for tool in ["unica.meta.add", "unica.meta.edit"] {
            let root = test_workspace_root(&format!(
                "unica-meta-common-format-gate-{}-{version}",
                tool.replace('.', "-")
            ));
            let workspace = root.join("workspace");
            let source = workspace.join("src");
            std::fs::create_dir_all(source.join("Catalogs")).unwrap();
            std::fs::write(
                workspace.join("v8project.yaml"),
                "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
            )
            .unwrap();
            std::fs::write(
                source.join("Configuration.xml"),
                format!(
                    r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="{version}"><Configuration><Properties><Name>Test</Name></Properties><ChildObjects><Catalog>Items</Catalog></ChildObjects></Configuration></MetaDataObject>"#
                ),
            )
            .unwrap();
            std::fs::write(
                source.join("Catalogs/Items.xml"),
                support_test_catalog_xml("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"),
            )
            .unwrap();
            let before = unica_workspace::test_support::tree_snapshot(&source);
            let mut args = match tool {
                "unica.meta.add" => Map::from_iter([
                    ("sourceSet".to_string(), json!("main")),
                    ("kind".to_string(), json!("Catalog")),
                    ("name".to_string(), json!("Future")),
                ]),
                "unica.meta.edit" => Map::from_iter([
                    ("sourceSet".to_string(), json!("main")),
                    ("metadataPath".to_string(), json!("Catalog.Items")),
                    (
                        "operations".to_string(),
                        json!([{
                            "op": "setProperties",
                            "values": {"Comment": "changed"}
                        }]),
                    ),
                ]),
                _ => unreachable!(),
            };
            args.insert(
                "cwd".to_string(),
                Value::String(workspace.display().to_string()),
            );
            args.insert("dryRun".to_string(), Value::Bool(false));

            let result = crate::composition::testing::application()
                .call_tool(tool, &args)
                .unwrap();

            assert!(!result.ok, "{tool}, version={version}: {result:?}");
            let diagnostic = result
                .diagnostics
                .as_ref()
                .and_then(Value::as_array)
                .and_then(|diagnostics| diagnostics.first())
                .expect("metadata refusal has one typed diagnostic");
            assert_eq!(diagnostic["code"], "capability_unavailable");
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(&source),
                before,
                "{tool}, version={version} changed source bytes"
            );
            assert!(result.changes.is_empty(), "{tool}: {result:?}");
            assert!(result.artifacts.is_empty(), "{tool}: {result:?}");
            assert!(result.cache.events.is_empty(), "{tool}: {result:?}");
            std::fs::remove_dir_all(root).unwrap();
        }
    }
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

    let result = crate::composition::testing::application()
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
        let root = test_workspace_root(&format!("unica-form-existing-newer-nonstandard-{index}"));
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

        let result = crate::composition::testing::application()
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

        let result = crate::composition::testing::application()
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

    let result = crate::composition::testing::application()
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
    let root = std::env::temp_dir().join(format!(
        "{prefix}-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
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
    let root = std::env::temp_dir().join(format!(
        "unica-xml-tool-edt-guard-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
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
        "TemplatePath".to_string(),
        Value::String("src/Reports/Sales/Templates/Sheet/Ext/Template.xml".to_string()),
    );

    let error = match crate::composition::testing::application().call_tool("unica.mxl.info", &args)
    {
        Ok(result) => panic!("expected EDT source-set guard, got {}", result.summary),
        Err(error) => error,
    };

    assert!(error.contains("sourceFormat=edt"));
    assert!(error.contains("platform_xml"));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn native_xml_metadata_tools_reject_ambiguous_source_set_targets() {
    let root = std::env::temp_dir().join(format!(
        "unica-xml-tool-ambiguous-guard-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let workspace = root.join("workspace");
    std::fs::create_dir_all(workspace.join("src")).unwrap();
    std::fs::write(
        workspace.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    std::fs::write(workspace.join("src/.project"), "<projectDescription/>").unwrap();
    std::fs::write(workspace.join("src/Configuration.xml"), "not XML").unwrap();
    let args = Map::from_iter([
        (
            "cwd".to_string(),
            Value::String(workspace.display().to_string()),
        ),
        (
            "TemplatePath".to_string(),
            Value::String("src/Reports/Sales/Templates/Sheet/Ext/Template.xml".to_string()),
        ),
    ]);

    let error = crate::composition::testing::application()
        .call_tool("unica.mxl.info", &args)
        .expect_err("ambiguous source format must fail before XML parsing");

    assert!(error.contains("invalid/ambiguous format"), "{error}");
    assert!(error.contains("platform_xml"), "{error}");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn read_only_native_outfile_is_rejected_before_any_write() {
    let root = std::env::temp_dir().join(format!(
        "unica-read-outfile-write-guard-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
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
        "TemplatePath".to_string(),
        Value::String("src/Reports/Sales/Templates/Sheet/Ext/Template.xml".to_string()),
    );
    args.insert(
        "OutFile".to_string(),
        Value::String(outside.display().to_string()),
    );

    let error = match crate::composition::testing::application().call_tool("unica.mxl.info", &args)
    {
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
    let root = std::env::temp_dir().join(format!(
        "unica-cfe-borrow-edt-guard-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
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

    let error =
        match crate::composition::testing::application().call_tool("unica.cfe.borrow", &args) {
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
    let root = std::env::temp_dir().join(format!(
        "unica-cfe-borrow-mutation-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
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

    let result = crate::composition::testing::application()
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
        created
            .iter()
            .any(|path| normalized_path(std::path::Path::new(path)) == normalized_path(&borrowed)),
        "{created:?}"
    );
    assert!(
        updated.iter().any(|path| {
            normalized_path(std::path::Path::new(path)) == normalized_path(&extension_owner)
        }),
        "{updated:?}"
    );
    for path in &created {
        assert!(
            !updated.iter().any(|updated_path| {
                normalized_path(std::path::Path::new(updated_path))
                    == normalized_path(std::path::Path::new(path))
            }),
            "{path} is created and updated"
        );
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
    let root = std::env::temp_dir().join(format!(
        "unica-app-path-policy-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let mut args = Map::new();
    args.insert(
        "cwd".to_string(),
        Value::String(workspace.display().to_string()),
    );
    args.insert("dryRun".to_string(), Value::Bool(false));
    args.insert(
        "OutputPath".to_string(),
        Value::String("../outside/Form.xml".to_string()),
    );

    let error =
        match crate::composition::testing::application().call_tool("unica.form.compile", &args) {
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

    let error = crate::composition::testing::application()
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

    let error = crate::composition::testing::application()
        .call_tool("unica.form.compile", &args)
        .expect_err("form preview must enforce the same source-format guard as apply");

    assert!(error.contains("sourceFormat=edt"), "{error}");
    assert!(error.contains("platform_xml"), "{error}");
    assert!(!workspace.join("src/Form.xml").exists());
    let _ = std::fs::remove_dir_all(root);
}

fn metadata_test_object(value: Value) -> Map<String, Value> {
    value
        .as_object()
        .expect("test arguments are an object")
        .clone()
}

#[test]
fn preview_effects_follow_operation_order() {
    let workspace = std::env::temp_dir().join(format!(
        "unica-meta-effect-order-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    // Файловая фикстура пустой конфигурации 8.3.27 вместо снятого cf.init.
    fn copy_tree(from: &std::path::Path, to: &std::path::Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_tree(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), target).unwrap();
            }
        }
    }
    copy_tree(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/platform_8_3_27/empty_configuration"),
        &workspace,
    );
    let cwd = unica_workspace::test_support::ProcessCwdGuard::enter(&workspace).unwrap();
    let application = crate::composition::testing::application();
    for name in ["Owner", "Subject"] {
        let added = application
            .call_tool(
                "unica.meta.add",
                &metadata_test_object(json!({
                    "sourceSet": "main",
                    "kind": "Catalog",
                    "name": name,
                    "dryRun": false
                })),
            )
            .unwrap();
        assert!(added.ok, "{name}: {:?}", added.errors);
    }
    let operations = json!([
        {"op": "setProperties", "values": {"Comment": "ordered"}},
        {"op": "add", "collection": "attributes", "elements": [{"name": "Transient"}]},
        {"op": "update", "collection": "attributes", "elements": [{"name": "Transient", "newName": "Updated"}]},
        {"op": "remove", "collection": "attributes", "names": ["Updated"]},
        {
            "op": "add",
            "collection": "attributes",
            "elements": [{
                "name": "SearchKey",
                "type": {"variants": [{
                    "kind": "string",
                    "length": 20,
                    "allowedLength": "variable"
                }]},
                "fillValue": {"kind": "string", "value": "ready"}
            }]
        },
        {"op": "editRelations", "relation": "owners", "mode": "replace", "targets": [{"metadataPath": "Catalog.Owner"}]},
        {"op": "editRelations", "relation": "inputByString", "mode": "replace", "targets": [{"fieldPath": "Catalog.Subject.Attribute.SearchKey"}]}
    ]);
    let preview_call = json!({
        "sourceSet": "main",
        "metadataPath": "Catalog.Subject",
        "operations": operations,
        "dryRun": true
    });
    let missing_final_field_call = json!({
        "sourceSet": "main",
        "metadataPath": "Catalog.Subject",
        "operations": [{
            "op": "editRelations",
            "relation": "inputByString",
            "mode": "replace",
            "targets": [{
                "fieldPath": "Catalog.Subject.Attribute.Missing"
            }]
        }],
        "dryRun": true
    });
    let edit_schema = unica_metadata::request::metadata_input_schema(
        unica_metadata::request::MetadataOperation::Edit,
    );
    let edit_validator = jsonschema::validator_for(&edit_schema).unwrap();
    assert!(edit_validator.is_valid(&preview_call));
    assert!(edit_validator.is_valid(&missing_final_field_call));
    let preview = application
        .call_tool("unica.meta.edit", &metadata_test_object(preview_call))
        .unwrap();
    let missing_final_field = application
        .call_tool(
            "unica.meta.edit",
            &metadata_test_object(missing_final_field_call),
        )
        .unwrap();
    let template = application
        .call_tool(
            "unica.meta.add",
            &metadata_test_object(json!({
                "sourceSet": "main",
                "kind": "Catalog",
                "name": "TemplateOnly",
                "dryRun": true
            })),
        )
        .unwrap();
    drop(cwd);
    let _ = std::fs::remove_dir_all(&workspace);

    assert!(preview.ok, "{:?}", preview.errors);
    assert!(!missing_final_field.ok);
    assert!(missing_final_field
        .errors
        .iter()
        .any(|error| error.contains("does not exist in the owner post-image")));
    assert_eq!(
        missing_final_field.diagnostics.unwrap()[0]["field"],
        "operations[0].targets[0]"
    );
    let effects = preview.data.unwrap()["effects"].clone();
    assert_eq!(
        effects
            .as_array()
            .unwrap()
            .iter()
            .map(|effect| effect["operation"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec![
            "setProperties",
            "add",
            "update",
            "remove",
            "add",
            "editRelations",
            "editRelations",
        ]
    );
    assert!(effects
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .all(|(index, effect)| effect["operationIndex"] == index as u64));
    assert_eq!(
        template.data.unwrap()["effects"][0]["operation"],
        "createTemplate"
    );
}

// Call on a cloned schema before existing write_canonical. Only schema nodes
// lose annotations; const/default/enum values and property names are data.
fn remove_schema_descriptions(schema: &mut Value) {
    let Value::Object(object) = schema else {
        return;
    };
    object.remove("description");
    for (keyword, value) in object {
        match keyword.as_str() {
            "properties" | "patternProperties" | "$defs" | "definitions" | "dependentSchemas" => {
                if let Value::Object(schemas) = value {
                    for child in schemas.values_mut() {
                        remove_schema_descriptions(child);
                    }
                }
            }
            "allOf" | "anyOf" | "oneOf" | "prefixItems" => {
                if let Value::Array(schemas) = value {
                    for child in schemas {
                        remove_schema_descriptions(child);
                    }
                }
            }
            "items" => match value {
                Value::Array(schemas) => {
                    for child in schemas {
                        remove_schema_descriptions(child);
                    }
                }
                child => remove_schema_descriptions(child),
            },
            "additionalProperties"
            | "unevaluatedProperties"
            | "additionalItems"
            | "unevaluatedItems"
            | "contains"
            | "propertyNames"
            | "not"
            | "if"
            | "then"
            | "else"
            | "contentSchema" => remove_schema_descriptions(value),
            "dependencies" => {
                if let Value::Object(dependencies) = value {
                    for dependency in dependencies.values_mut() {
                        if dependency.is_object() {
                            remove_schema_descriptions(dependency);
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

#[test]
fn native_schema_fingerprint_ignores_annotations_but_keeps_description_fields() {
    let schema = json!({
        "type": "object", "description": "Old wording",
        "properties": {
            "description": {"type": "string", "description": "Explain the field"},
            "payload": {"const": {"description": "literal data"}}
        },
        "required": ["description"],
        "allOf": [{"description": "Nested wording", "minProperties": 1}]
    });
    let mut edited = schema.clone();
    edited["description"] = json!("Clear wording");
    edited["properties"]["description"]["description"] = json!("Clear field wording");
    edited["allOf"][0]["description"] = json!("Clear nested wording");
    assert_eq!(
        native_mutation_schema_signature(&schema),
        native_mutation_schema_signature(&edited)
    );

    let mut type_changed = schema.clone();
    type_changed["properties"]["description"]["type"] = json!("integer");
    assert_ne!(
        native_mutation_schema_signature(&schema),
        native_mutation_schema_signature(&type_changed)
    );
    let mut removed = schema.clone();
    removed["properties"]
        .as_object_mut()
        .unwrap()
        .remove("description");
    assert_ne!(
        native_mutation_schema_signature(&schema),
        native_mutation_schema_signature(&removed)
    );
    let mut constant_changed = schema.clone();
    constant_changed["properties"]["payload"]["const"]["description"] = json!("changed data");
    assert_ne!(
        native_mutation_schema_signature(&schema),
        native_mutation_schema_signature(&constant_changed)
    );
}

/// The compared form of a mutating tool's input schema: annotations
/// removed, keys sorted, one value per indented line. Snapshot files use
/// it so independent additions to one schema merge as separate lines.
fn native_mutation_schema_signature(schema: &Value) -> String {
    let mut contract = schema.clone();
    remove_schema_descriptions(&mut contract);
    canonical_pretty_json(&contract)
}

/// Deterministic pretty JSON: the workspace enables `preserve_order`, so
/// object keys are sorted here instead of relying on map order.
fn canonical_pretty_json(value: &Value) -> String {
    fn pad(depth: usize, output: &mut String) {
        for _ in 0..depth {
            output.push_str("  ");
        }
    }
    fn write(value: &Value, depth: usize, output: &mut String) {
        match value {
            Value::Array(items) if !items.is_empty() => {
                output.push_str("[\n");
                for (index, item) in items.iter().enumerate() {
                    pad(depth + 1, output);
                    write(item, depth + 1, output);
                    if index + 1 != items.len() {
                        output.push(',');
                    }
                    output.push('\n');
                }
                pad(depth, output);
                output.push(']');
            }
            Value::Object(object) if !object.is_empty() => {
                output.push_str("{\n");
                let mut keys = object.keys().collect::<Vec<_>>();
                keys.sort_unstable();
                for (index, key) in keys.iter().enumerate() {
                    pad(depth + 1, output);
                    output.push_str(&serde_json::to_string(key).expect("schema key serializes"));
                    output.push_str(": ");
                    write(&object[key.as_str()], depth + 1, output);
                    if index + 1 != keys.len() {
                        output.push(',');
                    }
                    output.push('\n');
                }
                pad(depth, output);
                output.push('}');
            }
            _ => output.push_str(&serde_json::to_string(value).expect("schema scalar serializes")),
        }
    }
    let mut output = String::new();
    write(value, 0, &mut output);
    output.push('\n');
    output
}

#[test]
fn native_mutation_schema_fingerprint_detects_nested_contract_changes() {
    let tool = tools()
        .into_iter()
        .find(|tool| tool.name == "unica.cf.edit")
        .expect("cf.edit is registered");
    let schema = input_schema_for_tool(&tool);
    let mut changed = schema.clone();
    changed["properties"]["dryRun"]["type"] = json!("string");

    assert_ne!(
        native_mutation_schema_signature(&schema),
        native_mutation_schema_signature(&changed),
        "a nested schema type change must alter the complete fingerprint",
    );
}

/// The snapshot file of one tool: its operation and the compared schema.
fn native_mutation_snapshot(operation: &str, schema: &Value) -> String {
    let mut contract = schema.clone();
    remove_schema_descriptions(&mut contract);
    canonical_pretty_json(&json!({"operation": operation, "inputSchema": contract}))
}

#[test]
pub(crate) fn native_mutation_surface_has_exact_operations_and_schemas() {
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    const UPDATE: &str = "UNICA_UPDATE_SCHEMA_SNAPSHOTS";
    let actual = tools()
        .into_iter()
        .filter(|tool| tool.execution.is_mutating())
        .filter(|tool| {
            matches!(
                tool.handler,
                ToolHandler::NativeOperation { .. } | ToolHandler::Metadata { .. }
            )
        })
        .map(|tool| {
            let operation = match tool.handler {
                ToolHandler::NativeOperation { operation, .. } => operation.to_string(),
                ToolHandler::Metadata { operation } => format!("metadata:{operation:?}"),
                _ => unreachable!(),
            };
            let snapshot = native_mutation_snapshot(&operation, &input_schema_for_tool(&tool));
            (tool.name.to_string(), snapshot)
        })
        .collect::<BTreeMap<_, _>>();
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mutation-schemas");
    let recorded = |directory: &Path| -> BTreeMap<String, (PathBuf, String)> {
        let Ok(entries) = std::fs::read_dir(directory) else {
            return BTreeMap::new();
        };
        entries
            .map(|entry| entry.expect("snapshot entry is readable").path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
            .map(|path| {
                let name = path
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .expect("snapshot name is UTF-8")
                    .to_string();
                // A Windows checkout may convert the line endings.
                let text = std::fs::read_to_string(&path)
                    .expect("snapshot is UTF-8")
                    .replace("\r\n", "\n");
                (name, (path, text))
            })
            .collect()
    };
    if std::env::var(UPDATE).as_deref() == Ok("1") {
        std::fs::create_dir_all(&directory).expect("create the snapshot directory");
        for (name, (path, _)) in recorded(&directory) {
            if !actual.contains_key(&name) {
                std::fs::remove_file(path).expect("remove a stale snapshot");
            }
        }
        for (name, snapshot) in &actual {
            // Thread id and clock keep parallel updates in one run apart.
            let staged = directory.join(format!(
                ".{name}.{:?}.{}.json.tmp",
                std::thread::current().id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |elapsed| elapsed.as_nanos())
            ));
            std::fs::write(&staged, snapshot).expect("stage a snapshot");
            std::fs::rename(&staged, directory.join(format!("{name}.json")))
                .expect("publish a snapshot");
        }
    }
    let recorded = recorded(&directory);
    let mut problems = Vec::new();
    for name in actual.keys().filter(|name| !recorded.contains_key(*name)) {
        problems.push(format!("{name}: snapshot {name}.json is missing"));
    }
    for name in recorded.keys().filter(|name| !actual.contains_key(*name)) {
        problems.push(format!(
            "{name}: snapshot exists, but no mutating native tool has this name"
        ));
    }
    for (name, snapshot) in &actual {
        let Some((_, text)) = recorded.get(name) else {
            continue;
        };
        if text == snapshot {
            continue;
        }
        if text.trim_end() == snapshot.trim_end() {
            problems.push(format!(
                "{name}: {name}.json differs only in its final line ending"
            ));
            continue;
        }
        let expected = text.lines().collect::<Vec<_>>();
        let observed = snapshot.lines().collect::<Vec<_>>();
        let line = expected
            .iter()
            .zip(&observed)
            .position(|(left, right)| left != right)
            .unwrap_or(expected.len().min(observed.len()));
        problems.push(format!(
            "{name}: schema differs from {name}.json at line {}\n  snapshot: {}\n  actual:   {}",
            line + 1,
            expected.get(line).unwrap_or(&"<end of file>"),
            observed.get(line).unwrap_or(&"<end of file>"),
        ));
    }
    assert!(
        problems.is_empty(),
        "mutating tool contract differs from tests/fixtures/mutation-schemas:\n{}\n\
         If the change is intended, run `{UPDATE}=1 cargo test -p unica-coder --lib \
         native_mutation_surface_has_exact_operations_and_schemas` and review the snapshot diff.",
        problems.join("\n")
    );
}
