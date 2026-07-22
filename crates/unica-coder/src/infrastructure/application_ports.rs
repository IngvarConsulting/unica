use crate::application::discovery::contract::DiscoverRequest;
use crate::application::discovery::ports::{DefinitionPort, DiscoveryPorts, SourceInventoryPort};
use crate::application::discovery::use_case::DiscoverExtensionPointsUseCase;
use crate::application::ports::{ApplicationPorts, HandlerOutcome, SupportGuardCheck};
use crate::application::{project_map, project_status, AdapterOutcome, ToolHandler, ToolSpec};
use crate::domain::cache::{CacheAccess, CacheReport};
use crate::domain::cancellation::CancellationToken;
use crate::domain::discovery::{
    DefinitionFact, DiscoveryEnvironment, DiscoveryError, DiscoveryQuery, DiscoveryReport,
    FactBatch, MappingFingerprint, ProviderDiagnostic, ProviderOutcome, SourceInventory,
};
use crate::domain::events::DomainEvent;
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::discovery::bsl::{
    ExistingIndexDefinitionProvider, InventoryBslSearchProvider, UnavailableRuntimeFlowProvider,
};
use crate::infrastructure::discovery::forms::ManagedFormProvider;
use crate::infrastructure::discovery::inventory::ContainedSourceInventoryPort;
use crate::infrastructure::discovery::metadata::PlatformXmlMetadataProvider;
use crate::infrastructure::discovery::support::SupportStateProvider;
use crate::infrastructure::internal_adapters::{
    BslAnalyzerMcpAdapter, CliAdapter, CodeNavigationAdapter, CodeSearchAdapter,
    ConfigDumpInfoGitCheck, GitTrackingAdapter, RuntimeAdapter, RuntimeJobAdapter,
    StandardsAdapter,
};
use crate::infrastructure::native_operations::NativeOperationAdapter;
use crate::infrastructure::workspace_services::WorkspaceServiceManager;
use crate::infrastructure::workspace_state::WorkspaceStateRepository;
use serde_json::{Map, Value};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
pub(crate) struct InfrastructureApplicationPorts;

struct CapturingSourceInventoryPort {
    inner: ContainedSourceInventoryPort,
    captured: RefCell<Option<SourceInventory>>,
}

impl CapturingSourceInventoryPort {
    fn new(canonical_root: PathBuf) -> Self {
        Self {
            inner: ContainedSourceInventoryPort::new(canonical_root),
            captured: RefCell::new(None),
        }
    }
}

impl SourceInventoryPort for CapturingSourceInventoryPort {
    fn inventory(&self, query: &DiscoveryQuery<'_>) -> ProviderOutcome<SourceInventory> {
        let outcome = self.inner.inventory(query);
        let captured = match &outcome {
            ProviderOutcome::Complete(inventory)
            | ProviderOutcome::Bounded {
                data: inventory,
                diagnostic: _,
            } => Some(inventory.clone()),
            ProviderOutcome::Unavailable(_)
            | ProviderOutcome::Failed(_)
            | ProviderOutcome::ContractViolation(_) => None,
        };
        self.captured.replace(captured);
        outcome
    }
}

struct CapturedDefinitionPort<'a> {
    selected_root: &'a Path,
    cache_root: &'a Path,
    inventory: &'a RefCell<Option<SourceInventory>>,
    status: Option<&'a crate::infrastructure::workspace_index::BslIndexStatus>,
    status_error: Option<CapturedDefinitionStatusError>,
}

enum CapturedDefinitionStatusError {
    Unavailable(ProviderDiagnostic),
    Invalid(ProviderDiagnostic),
}

impl DefinitionPort for CapturedDefinitionPort<'_> {
    fn definitions(
        &self,
        query: &DiscoveryQuery<'_>,
    ) -> ProviderOutcome<FactBatch<DefinitionFact>> {
        if let Some(outcome) = crate::infrastructure::discovery::cancellation_outcome(query) {
            return outcome;
        }
        if let Some(error) = &self.status_error {
            return match error {
                CapturedDefinitionStatusError::Unavailable(diagnostic) => {
                    ProviderOutcome::Unavailable(diagnostic.clone())
                }
                CapturedDefinitionStatusError::Invalid(diagnostic) => {
                    ProviderOutcome::ContractViolation(diagnostic.clone())
                }
            };
        }
        let captured = self.inventory.borrow();
        match captured.as_ref() {
            Some(inventory) => ExistingIndexDefinitionProvider::new(
                self.selected_root,
                self.cache_root,
                inventory,
                self.status,
            )
            .definitions(query),
            None => ProviderOutcome::Unavailable(ProviderDiagnostic::material(
                "source_inventory_unavailable",
                "definition provider could not run because source inventory is unavailable",
            )),
        }
    }
}

impl ApplicationPorts for InfrastructureApplicationPorts {
    fn discover_workspace(
        &self,
        requested_cwd: Option<PathBuf>,
    ) -> Result<WorkspaceContext, String> {
        crate::infrastructure::workspace::discover_workspace(requested_cwd)
    }

    fn validate_tool_context(
        &self,
        spec: ToolSpec,
        args: &Map<String, Value>,
        dry_run: bool,
        context: &WorkspaceContext,
    ) -> Result<(), String> {
        crate::infrastructure::tool_context::validate_tool_context(spec, args, dry_run, context)
    }

    fn discover_extension_points(
        &self,
        request: &DiscoverRequest,
        context: &WorkspaceContext,
        cancellation: &CancellationToken,
    ) -> Result<DiscoveryReport, DiscoveryError> {
        if cancellation.is_cancelled() {
            return Err(DiscoveryError::Cancelled);
        }
        let selected = crate::infrastructure::source_roots::resolve_discovery_source_root(
            context,
            request.source_dir(),
            request.limits().max_files().get(),
            cancellation,
        )?;
        if cancellation.is_cancelled() {
            return Err(DiscoveryError::Cancelled);
        }

        let source_set = selected.source_set.as_deref().unwrap_or("explicit");
        let mapping_identity = format!("configuration:{source_set}:{}", selected.path.display());
        let environment = DiscoveryEnvironment::new(
            selected.path.clone(),
            MappingFingerprint::from_identity(&mapping_identity),
        );
        let inventory = CapturingSourceInventoryPort::new(selected.path.clone());
        let (status, status_error) =
            match crate::infrastructure::workspace_index::read_bsl_index_status_for_discovery(
                context,
                cancellation,
            ) {
                Ok(status) => (status, None),
                Err(crate::infrastructure::workspace_index::BslIndexStatusReadError::Cancelled) => {
                    return Err(DiscoveryError::Cancelled)
                }
                Err(
                    crate::infrastructure::workspace_index::BslIndexStatusReadError::Unavailable(
                        message,
                    ),
                ) => (
                    None,
                    Some(CapturedDefinitionStatusError::Unavailable(
                        ProviderDiagnostic::material("bsl_index_status_unavailable", message),
                    )),
                ),
                Err(crate::infrastructure::workspace_index::BslIndexStatusReadError::Invalid(
                    message,
                )) => (
                    None,
                    Some(CapturedDefinitionStatusError::Invalid(
                        ProviderDiagnostic::material("bsl_index_status_invalid", message),
                    )),
                ),
            };
        let definitions = CapturedDefinitionPort {
            selected_root: &selected.path,
            cache_root: &context.cache_root,
            inventory: &inventory.captured,
            status: status.as_ref(),
            status_error,
        };
        let use_case = DiscoverExtensionPointsUseCase::new(DiscoveryPorts {
            source_inventory: &inventory,
            metadata_catalog: &PlatformXmlMetadataProvider,
            managed_forms: &ManagedFormProvider,
            bsl_search: &InventoryBslSearchProvider,
            definitions: &definitions,
            runtime_flow: &UnavailableRuntimeFlowProvider,
            support_state: &SupportStateProvider,
        });
        let report = use_case.execute_cancellable(request, &environment, cancellation)?;
        if cancellation.is_cancelled() {
            return Err(DiscoveryError::Cancelled);
        }
        Ok(report)
    }

    fn evaluate_support_guard(
        &self,
        spec: ToolSpec,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
    ) -> Result<SupportGuardCheck, String> {
        crate::infrastructure::support_guard::evaluate_support_guard(spec, args, context)
    }

    fn invoke_handler(
        &self,
        spec: ToolSpec,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
        dry_run: bool,
        cancellation: &CancellationToken,
    ) -> Result<HandlerOutcome, String> {
        if cancellation.is_cancelled() {
            return Ok(HandlerOutcome::plain(AdapterOutcome::cancelled(format!(
                "{} stopped before adapter execution",
                spec.name
            ))));
        }
        match spec.handler {
            ToolHandler::NativeOperation { operation, .. } => {
                NativeOperationAdapter::invoke_with_data(
                    operation,
                    spec.name,
                    args,
                    context,
                    dry_run,
                    spec.mutating,
                )
                .map(|outcome| match outcome.data {
                    Some(data) => HandlerOutcome::with_data(outcome.adapter, data),
                    None => HandlerOutcome::plain(outcome.adapter),
                })
            }
            ToolHandler::ProjectStatus => {
                let source_map =
                    crate::infrastructure::project_sources::discover_project_source_map(
                        &context.workspace_root,
                    );
                if cancellation.is_cancelled() {
                    return Ok(HandlerOutcome::plain(AdapterOutcome::cancelled(
                        "unica.project.status source-set discovery stopped",
                    )));
                }
                let warning = match GitTrackingAdapter::new()
                    .config_dump_info_warning(context, cancellation)
                {
                    ConfigDumpInfoGitCheck::Complete(warning) => warning,
                    ConfigDumpInfoGitCheck::Cancelled => {
                        return Ok(HandlerOutcome::plain(AdapterOutcome::cancelled(
                            "unica.project.status Git tracking check stopped",
                        )));
                    }
                };
                Ok(HandlerOutcome::plain(project_status(
                    context, source_map, warning,
                )))
            }
            ToolHandler::ProjectMap => {
                let source_map =
                    crate::infrastructure::project_sources::discover_project_source_map(
                        &context.workspace_root,
                    );
                if cancellation.is_cancelled() {
                    return Ok(HandlerOutcome::plain(AdapterOutcome::cancelled(
                        "unica.project.map source-set discovery stopped",
                    )));
                }
                let warning = match GitTrackingAdapter::new()
                    .config_dump_info_warning(context, cancellation)
                {
                    ConfigDumpInfoGitCheck::Complete(warning) => warning,
                    ConfigDumpInfoGitCheck::Cancelled => {
                        return Ok(HandlerOutcome::plain(AdapterOutcome::cancelled(
                            "unica.project.map Git tracking check stopped",
                        )));
                    }
                };
                Ok(HandlerOutcome::plain(project_map(source_map, warning)))
            }
            ToolHandler::ProjectDiscover => Err(
                "internal dispatch error: unica.project.discover requires typed application dispatch"
                    .to_string(),
            ),
            ToolHandler::BuildRuntime { command, .. } => {
                CliAdapter::new("v8-runner", command, "build/runtime")
                    .invoke_cancellable(
                        spec.name,
                        args,
                        context,
                        dry_run,
                        spec.mutating,
                        cancellation,
                    )
                    .map(HandlerOutcome::plain)
            }
            ToolHandler::RuntimeAdapter => RuntimeAdapter::new()
                .invoke_cancellable(
                    spec.name,
                    args,
                    context,
                    dry_run,
                    spec.mutating,
                    cancellation,
                )
                .map(HandlerOutcome::plain),
            ToolHandler::RuntimeJob { action } => RuntimeJobAdapter::invoke(
                action, spec.name, args, context, dry_run,
            )
            .map(|outcome| HandlerOutcome {
                adapter: outcome.outcome,
                data: None,
                job: outcome.job,
            }),
            ToolHandler::CodeAdapter { command } if command == ["search"] => {
                CodeSearchAdapter::new()
                    .invoke_cancellable(spec.name, args, context, dry_run, cancellation)
                    .map(HandlerOutcome::plain)
            }
            ToolHandler::CodeAdapter {
                command: ["definition"] | ["outline"] | ["grep"] | ["meta-profile"],
            } => CodeNavigationAdapter::new()
                .invoke_cancellable(spec.name, args, context, dry_run, cancellation)
                .map(HandlerOutcome::plain),
            ToolHandler::CodeAdapter {
                command: ["graph"] | ["analyze"],
            } => BslAnalyzerMcpAdapter::new()
                .invoke_cancellable(spec.name, args, context, dry_run, cancellation)
                .map(HandlerOutcome::plain),
            ToolHandler::CodeAdapter { command } => {
                CliAdapter::new("bsl-analyzer", command, "code analysis")
                    .invoke_cancellable(
                        spec.name,
                        args,
                        context,
                        dry_run,
                        spec.mutating,
                        cancellation,
                    )
                    .map(HandlerOutcome::plain)
            }
            ToolHandler::StandardsAdapter { operation } => Ok(HandlerOutcome::plain(
                StandardsAdapter::invoke(operation, args),
            )),
        }
    }

    fn cache_report(
        &self,
        context: &WorkspaceContext,
        events: &[DomainEvent],
        dry_run: bool,
        cache_access: CacheAccess,
    ) -> Result<CacheReport, String> {
        WorkspaceStateRepository::new(context).report(context, events, dry_run, cache_access)
    }

    fn notify_invalidation(&self, context: &WorkspaceContext, events: &[DomainEvent]) {
        WorkspaceServiceManager::new().notify_invalidation(context, events);
    }
}

#[cfg(test)]
mod tests {
    use super::InfrastructureApplicationPorts;
    use crate::application::discovery::contract::parse_discover_request;
    use crate::application::ports::ApplicationPorts;
    use crate::domain::cancellation::CancellationToken;
    use crate::domain::discovery::{
        DiscoveryStatus, ProviderKind, ProviderOutcomeKind, SOURCE_INVENTORY_TRAVERSAL_BOUND_CODE,
    };
    use crate::domain::workspace::WorkspaceContext;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    #[test]
    fn irrelevant_fanout_remains_a_public_bounded_partial_inventory() {
        let fixture = Fixture::new("traversal-bound-public");
        let source_root = fixture.root.join("src");
        let ignored = source_root.join("ignored");
        fs::create_dir_all(&ignored).expect("ignored fixture directory");
        fs::write(
            fixture.root.join("v8project.yaml"),
            "source-set:\n  - name: app\n    type: CONFIGURATION\n    path: src\n",
        )
        .expect("project manifest");
        fs::write(
            source_root.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><Configuration uuid="00000000-0000-0000-0000-000000000001"><Properties><Name>Configuration</Name></Properties></Configuration></MetaDataObject>"#,
        )
        .expect("configuration marker");
        // maxFiles=2 derives 1,040 traversal entries. The source root consumes
        // two, so 1,039 irrelevant children exercise the deterministic N+1.
        for index in 0..1_039 {
            fs::write(ignored.join(format!("ignored-{index:04}.txt")), b"ignored")
                .expect("irrelevant fanout entry");
        }
        let serde_json::Value::Object(args) = serde_json::json!({
            "mode": "explore",
            "task": "discover configuration extension points",
            "limits": { "maxFiles": 2 }
        }) else {
            unreachable!("static request object")
        };
        let request = parse_discover_request(&args).expect("bounded request");
        let context = WorkspaceContext {
            cwd: fixture.root.clone(),
            workspace_root: fixture.root.clone(),
            cache_root: fixture.root.join(".build/unica"),
            workspace_epoch: 1,
        };

        let report = InfrastructureApplicationPorts
            .discover_extension_points(&request, &context, &CancellationToken::new())
            .expect("partial discovery report");

        assert_eq!(report.status, DiscoveryStatus::Partial);
        let inventory = report
            .provider_outcomes
            .iter()
            .find(|outcome| outcome.provider == ProviderKind::SourceInventory)
            .expect("source inventory report");
        assert_eq!(inventory.outcome, ProviderOutcomeKind::Bounded);
        assert_eq!(
            inventory
                .diagnostic
                .as_ref()
                .map(|diagnostic| diagnostic.code.as_str()),
            Some(SOURCE_INVENTORY_TRAVERSAL_BOUND_CODE)
        );
        assert!(report.missing_checks.iter().any(|check| {
            check.provider == ProviderKind::SourceInventory
                && check.code == SOURCE_INVENTORY_TRAVERSAL_BOUND_CODE
        }));
    }

    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn new(label: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "unica-{label}-{}-{nanos}-{}",
                std::process::id(),
                TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(root.join("src")).expect("fixture source root");
            Self {
                root: fs::canonicalize(root).expect("canonical fixture root"),
            }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.root).expect("fixture cleanup");
        }
    }
}
