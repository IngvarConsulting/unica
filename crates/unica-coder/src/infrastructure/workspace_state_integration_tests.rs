use std::{fs, path::PathBuf};
use unica_diagnostics::model::*;
use unica_source_contract::events::{DomainEvent, DomainEventKind};
use unica_source_contract::{ProjectSourceSet, SourceFormat, SourceSetKind, SourceSetState};
use unica_work_control::provider_deadline::ProviderDeadline;
use unica_workspace::cache::CacheAccess;
use unica_workspace::state::WorkspaceStateRepository;

use crate::infrastructure::analysis_composition::index_is_ready as bsl_index_is_ready;
use serde_json::{json, Map, Value};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use unica_adapter_rlm::index::rlm_provider_state_root;
use unica_address::source_target::{MetadataAddress, ResolvedTarget, TargetKind};
use unica_source_contract::location::SourceLocation;
use unica_source_contract::workspace::{ResolvedSourceRoot, WorkspaceContext};
use unica_work_control::cancellation::CancellationToken;

const ANALYZER: DiagnosticProviderId = DiagnosticProviderId::new_const("bsl-analyzer");
static ANALYZER_DESCRIPTOR: DiagnosticProviderDescriptor = DiagnosticProviderDescriptor {
    id: ANALYZER,
    actions: &[
        DiagnosticAction::Analyze,
        DiagnosticAction::Findings,
        DiagnosticAction::Status,
        DiagnosticAction::Catalog,
    ],
    findings_target_kinds: &[TargetKind::Module],
    emits_focus_kinds: &[DiagnosticFocusKind::SourceRange],
};
struct CacheProvider(DiagnosticProviderOutcome);
impl DiagnosticProvider for CacheProvider {
    fn descriptor(&self) -> &'static DiagnosticProviderDescriptor {
        &ANALYZER_DESCRIPTOR
    }
    fn execute(
        &self,
        _: &DiagnosticProviderRequest,
        _: &DiagnosticContext,
        _: ProviderDeadline,
        _: &CancellationToken,
    ) -> DiagnosticProviderOutcome {
        self.0.clone()
    }
}
fn successful(observations: Vec<DiagnosticObservation>) -> DiagnosticProviderOutcome {
    DiagnosticProviderOutcome {
        status: if observations.is_empty() {
            DiagnosticProviderStatus::Empty
        } else {
            DiagnosticProviderStatus::Completed
        },
        complete: true,
        version: Some("test".to_string()),
        observations,
        rules: Vec::new(),
        readiness: None,
        error: None,
        suppressions: Vec::new(),
    }
}

struct CachePorts {
    workspace: WorkspaceContext,
    outcome: DiagnosticProviderOutcome,
}

impl crate::application::ports::ApplicationPorts for CachePorts {
    fn discover_workspace(&self, _: Option<PathBuf>) -> Result<WorkspaceContext, String> {
        Ok(self.workspace.clone())
    }
    fn validate_tool_context(
        &self,
        _: crate::application::ToolSpec,
        _: &Map<String, Value>,
        _: crate::application::InvocationMode,
        _: &WorkspaceContext,
    ) -> Result<(), String> {
        Ok(())
    }
    fn evaluate_support_guard(
        &self,
        _: crate::application::ToolSpec,
        _: &Map<String, Value>,
        _: &WorkspaceContext,
    ) -> Result<crate::application::ports::SupportGuardCheck, String> {
        Ok(crate::application::ports::SupportGuardCheck::Allow)
    }
    fn invoke_handler(
        &self,
        _: crate::application::ToolSpec,
        _: &Map<String, Value>,
        _: &WorkspaceContext,
        _: crate::application::InvocationMode,
        _: &CancellationToken,
    ) -> Result<crate::application::ports::HandlerOutcome, String> {
        unreachable!()
    }
    fn diagnostic_provider_registry(&self) -> Result<DiagnosticProviderRegistry, String> {
        DiagnosticProviderRegistry::new(vec![Arc::new(CacheProvider(self.outcome.clone()))])
            .map_err(|error| format!("{error:?}"))
    }
    fn resolve_diagnostic_context(
        &self,
        request: &DiagnosticRequest,
        workspace: &WorkspaceContext,
        _: &CancellationToken,
    ) -> Result<DiagnosticContext, DiagnosticRequestError> {
        Ok(DiagnosticContext::new(
            workspace.clone(),
            ProjectSourceSet {
                name: request.source_set.clone(),
                kind: SourceSetKind::Configuration,
                path: "src".to_string(),
                source_format: SourceFormat::PlatformXml,
                source_state: SourceSetState::Supported,
                format_evidence: Vec::new(),
                format_probe_error: None,
            },
            ResolvedSourceRoot {
                source_set: Some(request.source_set.clone()),
                path: workspace.workspace_root.join("src"),
            },
            ResolvedTarget {
                source_set: request.source_set.clone(),
                metadata_path: request.metadata_path.clone(),
                target_kind: request
                    .metadata_path
                    .as_ref()
                    .map_or(TargetKind::SourceRoot, MetadataAddress::target_kind),
            },
        ))
    }
    fn map_diagnostic_observation(
        &self,
        observation: DiagnosticObservation,
        context: &DiagnosticContext,
        _: &CancellationToken,
    ) -> Result<DiagnosticItem, DiagnosticMapError> {
        let location = SourceLocation::Addressed {
            source_set: context.target.source_set.clone(),
            metadata_path: context.target.metadata_path.clone(),
            target_kind: context.target.target_kind,
        };
        Ok(match observation {
            DiagnosticObservation::Diagnostic {
                provider,
                code,
                severity,
                message,
                tags,
                ..
            } => DiagnosticItem::Diagnostic {
                provider: provider.as_str(),
                location,
                location_reason: None,
                focus: DiagnosticFocus::Target,
                code,
                severity,
                message,
                tags,
            },
            DiagnosticObservation::ResourceFailure {
                provider, error, ..
            } => DiagnosticItem::ResourceFailure {
                provider: provider.as_str(),
                location,
                location_reason: None,
                error,
            },
        })
    }
    fn cache_report(
        &self,
        context: &WorkspaceContext,
        events: &[unica_source_contract::events::DomainEvent],
        mode: crate::application::InvocationMode,
        access: unica_workspace::cache::CacheAccess,
    ) -> Result<unica_workspace::cache::CacheReport, String> {
        unica_workspace::state::WorkspaceStateRepository::new(context).report(
            context,
            events,
            mode.is_preview(),
            access,
        )
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

    fn notify_invalidation(
        &self,
        _: &WorkspaceContext,
        _: &[unica_source_contract::events::DomainEvent],
    ) {
    }
}

#[test]
fn diagnostics_reads_preserve_workspace_cache_staleness() {
    use unica_source_contract::events::{DomainEvent, DomainEventKind};
    use unica_workspace::cache::CacheAccess;
    use unica_workspace::state::WorkspaceStateRepository;
    let root = tempfile::tempdir().unwrap();
    let context = WorkspaceContext {
        cwd: root.path().to_path_buf(),
        workspace_root: root.path().to_path_buf(),
        cache_root: root.path().join(".cache"),
        workspace_epoch: 1,
    };
    let repo = WorkspaceStateRepository::new(&context);
    repo.report(
        &context,
        &[DomainEvent::new(
            DomainEventKind::ModuleChanged,
            "Module.bsl",
        )],
        false,
        CacheAccess::default(),
    )
    .unwrap();
    let failed = DiagnosticProviderOutcome {
        status: DiagnosticProviderStatus::Unavailable,
        complete: false,
        error: Some(DiagnosticError {
            code: "provider_unavailable".to_string(),
            message: "provider is unavailable".to_string(),
            retryable: true,
        }),
        ..successful(Vec::new())
    };
    let partial = DiagnosticProviderOutcome {
        complete: false,
        ..successful(vec![DiagnosticObservation::Diagnostic {
            provider: ANALYZER,
            location: DiagnosticObservationLocation::Resource {
                handle: "selected".to_string(),
            },
            focus: DiagnosticObservationFocus::Target,
            code: "A001".to_string(),
            severity: DiagnosticSeverity::Warning,
            message: "warning".to_string(),
            tags: Vec::new(),
        }])
    };
    let resource_failure = successful(vec![DiagnosticObservation::ResourceFailure {
        provider: ANALYZER,
        location: DiagnosticObservationLocation::Resource {
            handle: "selected".to_string(),
        },
        error: DiagnosticError {
            code: "resource_failed".to_string(),
            message: "resource failed".to_string(),
            retryable: false,
        },
    }]);
    for (action, metadata_path, outcome, fresh) in [
        ("analyze", None, failed.clone(), false),
        ("analyze", None, partial, false),
        ("analyze", None, resource_failure, false),
        (
            "status",
            None,
            DiagnosticProviderOutcome {
                status: DiagnosticProviderStatus::Completed,
                readiness: Some(DiagnosticReadiness {
                    state: DiagnosticReadinessState::Ready,
                    retryable: false,
                }),
                ..successful(Vec::new())
            },
            false,
        ),
        ("catalog", None, successful(Vec::new()), false),
        (
            "findings",
            Some("CommonModule.Selected.Module"),
            successful(Vec::new()),
            false,
        ),
        ("analyze", None, successful(Vec::new()), false),
        ("analyze", None, failed, false),
    ] {
        let mut args = json!({"action": action, "sourceSet": "main"});
        if let Some(metadata_path) = metadata_path {
            args["metadataPath"] = json!(metadata_path);
        }
        let expected_ok = matches!(
            outcome.status,
            DiagnosticProviderStatus::Completed | DiagnosticProviderStatus::Empty
        );
        let application = crate::application::UnicaApplication::with_ports(Arc::new(CachePorts {
            workspace: context.clone(),
            outcome,
        }));
        let result = application
            .call_tool("unica.code.diagnostics", args.as_object().unwrap())
            .unwrap();
        assert_eq!(result.ok, expected_ok, "{action}");
        if !expected_ok {
            let data = result.data.as_ref().unwrap();
            assert_eq!(data["action"], "analyze");
            assert_eq!(
                data["providers"][0]["error"]["code"],
                "provider_unavailable"
            );
            assert_eq!(data["providers"][0]["error"]["retryable"], true);
        }
        let cache = result.cache;
        assert_eq!(
            cache.fresh.iter().any(|name| name == "bsl_diagnostics"),
            fresh,
            "{action}"
        );
        assert_eq!(
            cache.stale.iter().any(|name| name == "bsl_diagnostics"),
            !fresh,
            "{action}"
        );
        if !fresh {
            assert!(cache.lazy_rebuilt.is_empty(), "{action}");
        }
        let persisted = repo
            .report(&context, &[], false, CacheAccess::default())
            .unwrap();
        assert_eq!(
            persisted.fresh.iter().any(|name| name == "bsl_diagnostics"),
            fresh,
            "persisted {action}"
        );
    }
}

#[test]
fn bsl_index_read_reflects_real_index_status_instead_of_lazy_rebuild() {
    let root = temp_root("unica-cache-lazy");
    fs::create_dir_all(&root).unwrap();
    let context = WorkspaceContext {
        cwd: root.clone(),
        workspace_root: root.clone(),
        cache_root: root.join(".cache"),
        workspace_epoch: 1,
    };
    let repo = WorkspaceStateRepository::new(&context).with_index_readiness(bsl_index_is_ready);

    let invalidation = repo
        .report(
            &context,
            &[DomainEvent::new(
                DomainEventKind::ModuleChanged,
                "Module.bsl",
            )],
            false,
            CacheAccess::default(),
        )
        .unwrap();
    assert!(invalidation.stale.contains(&"bsl_index".to_string()));

    let reported = repo
        .report(
            &context,
            &[],
            false,
            CacheAccess {
                reads: &["bsl_index"],
                writes: &[],
            },
        )
        .unwrap();
    assert!(reported.lazy_rebuilt.is_empty());
    assert!(reported.stale.contains(&"bsl_index".to_string()));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn bsl_index_cache_report_ignores_pair_scoped_builder_14_marker() {
    let root = temp_root("unica-cache-builder14-ignored");
    let source_root = root.join("src");
    fs::create_dir_all(&source_root).unwrap();
    fs::write(
        root.join("v8project.yaml"),
        "source-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    let context = WorkspaceContext {
        cwd: root.clone(),
        workspace_root: root.clone(),
        cache_root: root.join(".cache"),
        workspace_epoch: 1,
    };
    let pair_root = rlm_provider_state_root(&context, &source_root).unwrap();
    let legacy_db = pair_root.join("rlm-tools-bsl/index/bsl_index.db");
    let legacy_status = pair_root.join("caches/bsl_index_status.json");
    fs::create_dir_all(legacy_db.parent().unwrap()).unwrap();
    fs::create_dir_all(legacy_status.parent().unwrap()).unwrap();
    fs::write(&legacy_db, b"builder-14-db").unwrap();
    fs::write(
        &legacy_status,
        serde_json::to_string_pretty(&serde_json::json!({
            "status": "ready",
            "source_root": source_root.display().to_string(),
            "db_path": legacy_db.display().to_string(),
            "message": null,
            "updated_at": 1
        }))
        .unwrap()
            + "\n",
    )
    .unwrap();
    let legacy_bytes = fs::read(&legacy_status).unwrap();

    let reported = WorkspaceStateRepository::new(&context)
        .with_index_readiness(bsl_index_is_ready)
        .report(
            &context,
            &[],
            false,
            CacheAccess {
                reads: &["bsl_index"],
                writes: &[],
            },
        )
        .unwrap();

    assert!(reported.stale.contains(&"bsl_index".to_string()));
    assert!(!reported.lazy_rebuilt.contains(&"bsl_index".to_string()));
    assert_eq!(fs::read(&legacy_status).unwrap(), legacy_bytes);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn diagnostics_reads_preserve_stale_cache_and_existing_metadata() {
    let root = temp_root("unica-cache-lazy-existing");
    fs::create_dir_all(&root).unwrap();
    let context = WorkspaceContext {
        cwd: root.clone(),
        workspace_root: root.clone(),
        cache_root: root.join(".cache"),
        workspace_epoch: 1,
    };
    let repo = WorkspaceStateRepository::new(&context);
    repo.report(
        &context,
        &[DomainEvent::new(
            DomainEventKind::ModuleChanged,
            "Module.bsl",
        )],
        false,
        CacheAccess::default(),
    )
    .unwrap();
    let metadata = context.cache_root.join("caches/bsl_diagnostics.json");
    fs::create_dir_all(metadata.parent().unwrap()).unwrap();
    fs::write(&metadata, b"existing-lazy-preimage").unwrap();

    for tool_name in ["unica.code.graph", "unica.code.diagnostics"] {
        let tool = crate::application::tools()
            .into_iter()
            .find(|tool| tool.name == tool_name)
            .unwrap();
        let report = repo
            .report(&context, &[], false, tool.cache_access)
            .unwrap();
        assert!(
            report.stale.iter().any(|name| name == "bsl_diagnostics"),
            "{tool_name}"
        );
        assert!(report.lazy_rebuilt.is_empty(), "{tool_name}");
        assert_eq!(fs::read(&metadata).unwrap(), b"existing-lazy-preimage");
    }
    let _ = fs::remove_dir_all(root);
}

fn temp_root(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("{prefix}-{nanos}"))
}
