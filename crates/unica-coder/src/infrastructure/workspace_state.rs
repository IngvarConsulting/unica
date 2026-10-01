use crate::domain::cache::{path_for_report, CacheAccess, CacheImpact, CacheReport, EagerCacheKey};
use crate::domain::events::DomainEvent;
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::native_operations::compile_transaction::CompileTransaction;
use crate::infrastructure::workspace_index::bsl_index_is_ready;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::Path;
use std::path::PathBuf;

use crate::infrastructure::native_operations::apply::{ApplyStagedState, ApplyStagingError};

pub(crate) const CACHE_STATE_CONFLICT_RETRIES: usize = 64;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WorkspaceState {
    workspace_root: String,
    workspace_epoch: u64,
    caches: BTreeMap<String, CacheEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheEntry {
    status: CacheStatus,
    epoch: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum CacheStatus {
    Fresh,
    Stale,
}

pub struct WorkspaceStateRepository {
    state_path: PathBuf,
}

struct PlannedWorkspaceStateFile {
    path: PathBuf,
    preimage: Option<Vec<u8>>,
    postimage: Vec<u8>,
}

struct PlannedWorkspaceReport {
    report: CacheReport,
    state: Option<PlannedWorkspaceStateFile>,
    metadata: Vec<PlannedWorkspaceStateFile>,
}

impl PlannedWorkspaceReport {
    fn publication_paths(&self) -> Vec<PathBuf> {
        self.metadata
            .iter()
            .chain(self.state.iter())
            .map(|file| file.path.clone())
            .collect()
    }

    fn stage(self, transaction: &mut CompileTransaction) -> Result<CacheReport, String> {
        for file in self.metadata.into_iter().chain(self.state) {
            let PlannedWorkspaceStateFile {
                path,
                preimage,
                postimage,
            } = file;
            let expected_existing = preimage.is_some();
            let staged = match preimage {
                Some(preimage) => transaction.replace_bytes(&path, preimage, postimage),
                None => transaction.create_bytes(&path, postimage),
            };
            if let Err(error) = staged {
                if expected_existing
                    && fs::symlink_metadata(&path)
                        .is_err_and(|source| source.kind() == ErrorKind::NotFound)
                {
                    return Err(format!(
                        "cache publication target disappeared while staging: {}",
                        path.display()
                    ));
                }
                return Err(error);
            }
        }
        Ok(self.report)
    }
}

impl WorkspaceStateRepository {
    pub fn new(context: &WorkspaceContext) -> Self {
        Self {
            state_path: context.cache_root.join("state.json"),
        }
    }

    pub fn report(
        &self,
        context: &WorkspaceContext,
        events: &[DomainEvent],
        dry_run: bool,
        cache_access: CacheAccess,
    ) -> Result<CacheReport, String> {
        for attempt in 0..=CACHE_STATE_CONFLICT_RETRIES {
            let plan = self.plan_report(context, events, dry_run, cache_access)?;
            if plan.state.is_none() && plan.metadata.is_empty() {
                return Ok(plan.report);
            }
            let publication_paths = plan.publication_paths();
            let result = (|| -> Result<CacheReport, String> {
                let mut transaction = CompileTransaction::new();
                let mut report = plan.stage(&mut transaction)?;
                let commit = transaction.commit()?;
                report.publication_warnings = cache_cleanup_warnings(commit.cleanup_warnings);
                Ok(report)
            })();
            match result {
                Ok(report) => return Ok(report),
                Err(error)
                    if attempt < CACHE_STATE_CONFLICT_RETRIES
                        && is_retryable_cache_state_conflict(&error, &publication_paths) =>
                {
                    std::thread::yield_now();
                }
                Err(error) => {
                    return Err(format!(
                        "failed to atomically publish Unica cache state: {error}"
                    ));
                }
            }
        }
        unreachable!("bounded cache state retry loop always returns")
    }

    /// Stage the cache report in a caller-owned source transaction.
    ///
    /// Mutations that already own their publication transaction use this path
    /// so source bytes and cache state either commit or roll back together.
    /// A cache preimage conflict is surfaced to that transaction's caller; it
    /// is intentionally not retried after source publication.
    pub(crate) fn stage_report_in_transaction(
        &self,
        transaction: &mut CompileTransaction,
        context: &WorkspaceContext,
        events: &[DomainEvent],
        dry_run: bool,
        cache_access: CacheAccess,
    ) -> Result<CacheReport, String> {
        self.plan_report(context, events, dry_run, cache_access)?
            .stage(transaction)
    }

    pub(crate) fn stage_report_in_retained_apply(
        &self,
        state: &mut ApplyStagedState,
        cache_prefix: &Path,
        context: &WorkspaceContext,
        events: &[DomainEvent],
        dry_run: bool,
        cache_access: CacheAccess,
    ) -> Result<CacheReport, ApplyStagingError> {
        let state_relative = cache_prefix.join("state.json");
        let state_preimage = state.read(&state_relative)?;
        let impact = CacheImpact::from_events(events);
        let mut metadata_preimages = BTreeMap::new();
        for name in &impact.eager_refresh {
            let key = EagerCacheKey::from_name(name).ok_or_else(|| {
                ApplyStagingError::new(
                    crate::infrastructure::native_operations::apply::ApplyStagingErrorKind::Invariant,
                    format!("eager cache key is outside the closed catalog: {name}"),
                )
            })?;
            let relative = cache_prefix
                .join("caches")
                .join(format!("{}.json", key.as_str()));
            metadata_preimages.insert(name.clone(), state.read(&relative)?);
        }
        let plan = self
            .plan_report_from_preimages(
                context,
                events,
                dry_run,
                cache_access,
                state_preimage,
                metadata_preimages,
            )
            .map_err(|error| {
                ApplyStagingError::new(
                    crate::infrastructure::native_operations::apply::ApplyStagingErrorKind::Invariant,
                    error,
                )
            })?;
        let report = plan.report.clone();
        for file in plan.metadata.into_iter().chain(plan.state) {
            let relative = file
                .path
                .strip_prefix(&context.cache_root)
                .map_err(|_| {
                    ApplyStagingError::new(
                        crate::infrastructure::native_operations::apply::ApplyStagingErrorKind::ContainmentIdentity,
                        "cache artifact escaped the actor-owned cache root",
                    )
                })?;
            let relative = cache_prefix.join(relative);
            match file.preimage {
                Some(preimage) => state.replace(relative, preimage, file.postimage)?,
                None => state.create(relative, file.postimage)?,
            }
        }
        Ok(report)
    }

    fn plan_report(
        &self,
        context: &WorkspaceContext,
        events: &[DomainEvent],
        dry_run: bool,
        cache_access: CacheAccess,
    ) -> Result<PlannedWorkspaceReport, String> {
        let (_, state_preimage) = self.load_snapshot(context)?;
        let impact = CacheImpact::from_events(events);
        let mut metadata_preimages = BTreeMap::new();
        for name in &impact.eager_refresh {
            let path = context
                .cache_root
                .join("caches")
                .join(format!("{name}.json"));
            let preimage = match fs::read(&path) {
                Ok(bytes) => Some(bytes),
                Err(error) if error.kind() == ErrorKind::NotFound => None,
                Err(error) => {
                    return Err(format!(
                        "failed to read Unica cache metadata {}: {error}",
                        path.display()
                    ))
                }
            };
            metadata_preimages.insert(name.clone(), preimage);
        }
        self.plan_report_from_preimages(
            context,
            events,
            dry_run,
            cache_access,
            state_preimage,
            metadata_preimages,
        )
    }

    fn plan_report_from_preimages(
        &self,
        context: &WorkspaceContext,
        events: &[DomainEvent],
        dry_run: bool,
        cache_access: CacheAccess,
        state_preimage: Option<Vec<u8>>,
        metadata_preimages: BTreeMap<String, Option<Vec<u8>>>,
    ) -> Result<PlannedWorkspaceReport, String> {
        let impact = CacheImpact::from_events(events);
        let mut state = state_preimage
            .as_deref()
            .and_then(|bytes| serde_json::from_slice(bytes).ok())
            .unwrap_or_else(|| default_state(context));
        let mut invalidated = sorted(impact.invalidated);
        let mut refreshed = sorted(impact.eager_refresh);
        let lazy_rebuilt = Vec::new();
        let mut metadata = Vec::new();
        let mut write_state = false;

        if !events.is_empty() && !dry_run {
            for name in &invalidated {
                state.caches.insert(
                    name.clone(),
                    CacheEntry {
                        status: CacheStatus::Stale,
                        epoch: context.workspace_epoch,
                    },
                );
            }
            for name in &refreshed {
                state.caches.insert(
                    name.clone(),
                    CacheEntry {
                        status: CacheStatus::Fresh,
                        epoch: context.workspace_epoch,
                    },
                );
                metadata.push(self.plan_cache_metadata(
                    context,
                    name,
                    "eager",
                    metadata_preimages.get(name).cloned().flatten(),
                )?);
            }
            state.workspace_epoch = context.workspace_epoch;
            write_state = true;
        }

        if dry_run {
            refreshed.clear();
        }
        if events.is_empty() {
            invalidated.clear();
            refreshed.clear();
        }

        if events.is_empty() && !dry_run {
            for name in cache_access.reads {
                if *name == "bsl_index" {
                    state.caches.insert(
                        (*name).to_string(),
                        CacheEntry {
                            status: if bsl_index_is_ready(context) {
                                CacheStatus::Fresh
                            } else {
                                CacheStatus::Stale
                            },
                            epoch: context.workspace_epoch,
                        },
                    );
                }
            }
        }

        let mut stale = Vec::new();
        let mut fresh = Vec::new();
        for (name, entry) in &state.caches {
            match entry.status {
                CacheStatus::Fresh => fresh.push(name.clone()),
                CacheStatus::Stale => stale.push(name.clone()),
            }
        }

        let report = CacheReport {
            mode: if events.is_empty() {
                "read".to_string()
            } else if dry_run {
                "dry-run".to_string()
            } else {
                "applied".to_string()
            },
            root: path_for_report(&context.cache_root),
            workspace_epoch: context.workspace_epoch,
            events: events
                .iter()
                .map(|event| event.name().to_string())
                .collect(),
            invalidated,
            refreshed,
            lazy_rebuilt,
            stale,
            fresh,
            publication_warnings: Vec::new(),
        };
        let state = if write_state {
            let mut postimage =
                serde_json::to_vec_pretty(&state).map_err(|error| error.to_string())?;
            postimage.push(b'\n');
            Some(PlannedWorkspaceStateFile {
                path: self.state_path.clone(),
                preimage: state_preimage,
                postimage,
            })
        } else {
            None
        };
        Ok(PlannedWorkspaceReport {
            report,
            state,
            metadata,
        })
    }

    fn load_snapshot(
        &self,
        context: &WorkspaceContext,
    ) -> Result<(WorkspaceState, Option<Vec<u8>>), String> {
        match fs::read(&self.state_path) {
            Ok(bytes) => {
                let state =
                    serde_json::from_slice(&bytes).unwrap_or_else(|_| default_state(context));
                Ok((state, Some(bytes)))
            }
            Err(error) if error.kind() == ErrorKind::NotFound => Ok((default_state(context), None)),
            Err(error) => Err(format!(
                "failed to read Unica cache state {}: {error}",
                self.state_path.display()
            )),
        }
    }

    fn plan_cache_metadata(
        &self,
        context: &WorkspaceContext,
        name: &str,
        mode: &str,
        preimage: Option<Vec<u8>>,
    ) -> Result<PlannedWorkspaceStateFile, String> {
        let path = context
            .cache_root
            .join("caches")
            .join(format!("{name}.json"));
        let text = serde_json::json!({
            "name": name,
            "mode": mode,
            "workspaceEpoch": context.workspace_epoch,
        });
        let mut postimage = serde_json::to_vec_pretty(&text).map_err(|error| error.to_string())?;
        postimage.push(b'\n');
        Ok(PlannedWorkspaceStateFile {
            path,
            preimage,
            postimage,
        })
    }
}

fn default_state(context: &WorkspaceContext) -> WorkspaceState {
    WorkspaceState {
        workspace_root: context.workspace_root.display().to_string(),
        workspace_epoch: context.workspace_epoch,
        caches: BTreeMap::new(),
    }
}

fn sorted(values: std::collections::BTreeSet<String>) -> Vec<String> {
    values.into_iter().collect()
}

fn cache_cleanup_warnings(cleanup_warnings: Vec<String>) -> Vec<String> {
    if cleanup_warnings.is_empty() {
        return Vec::new();
    }
    let mut stderr = std::io::stderr().lock();
    for warning in &cleanup_warnings {
        let _ = writeln!(
            stderr,
            "Unica cache state committed with cleanup warning: {warning}"
        );
    }
    vec![format!(
        "cache state committed, but {} recovery or staging artifact(s) require cleanup; inspect the Unica server log",
        cleanup_warnings.len()
    )]
}

pub(crate) fn is_retryable_cache_state_conflict(error: &str, paths: &[PathBuf]) -> bool {
    let error = error.trim_end();
    let Some(conflict_prefix) = paths.iter().find_map(|path| {
        let suffix = format!(": {}", path.to_string_lossy());
        error.strip_suffix(&suffix)
    }) else {
        return false;
    };
    if [
        "; rollback encountered:",
        "; cleanup encountered:",
        "; cleanup warnings:",
    ]
    .iter()
    .any(|marker| conflict_prefix.contains(marker))
    {
        return false;
    }
    [
        "create-only compile target is already",
        "create-only publication target already exists",
        "publication target differs from the expected preimage",
        "publication target metadata changed before commit",
        "replacement target changed while planning",
        "registration target changed after planning",
        "registration target metadata changed after planning",
        "registration target disappeared before commit",
        "cache publication target disappeared while staging",
    ]
    .iter()
    .any(|marker| conflict_prefix.contains(marker))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::cancellation::CancellationToken;
    use crate::domain::diagnostics::*;
    use crate::domain::events::{DomainEvent, DomainEventKind};
    use crate::domain::project_sources::{
        ProjectSourceSet, SourceFormat, SourceSetKind, SourceSetState,
    };
    use crate::domain::source_location::SourceLocation;
    use crate::domain::source_roots::ResolvedSourceRoot;
    use crate::domain::source_target::{MetadataAddress, ResolvedTarget, TargetKind};
    use crate::infrastructure::workspace_index::rlm_provider_state_root;
    use serde_json::{json, Map, Value};
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

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
            events: &[crate::domain::events::DomainEvent],
            mode: crate::application::InvocationMode,
            access: crate::domain::cache::CacheAccess,
        ) -> Result<crate::domain::cache::CacheReport, String> {
            crate::infrastructure::workspace_state::WorkspaceStateRepository::new(context).report(
                context,
                events,
                mode.is_preview(),
                access,
            )
        }
        fn notify_invalidation(
            &self,
            _: &WorkspaceContext,
            _: &[crate::domain::events::DomainEvent],
        ) {
        }
    }

    #[test]
    fn diagnostics_reads_preserve_workspace_cache_staleness() {
        use crate::domain::cache::CacheAccess;
        use crate::domain::events::{DomainEvent, DomainEventKind};
        use crate::infrastructure::workspace_state::WorkspaceStateRepository;
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
            let application =
                crate::application::UnicaApplication::with_ports(Arc::new(CachePorts {
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
        let repo = WorkspaceStateRepository::new(&context);

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

    #[test]
    fn cache_conflict_classifier_requires_an_exact_planned_cache_path() {
        let root = PathBuf::from("/workspace");
        let state = root.join("state.json");
        let source = root.join("src/CommonModules/Shared/Ext/Module.bsl");

        assert!(is_retryable_cache_state_conflict(
            &format!(
                "create-only compile target is already an existing file: {}",
                state.display()
            ),
            std::slice::from_ref(&state),
        ));
        let recovery_named_state = PathBuf::from("/workspace/recovery-project/state.json");
        assert!(is_retryable_cache_state_conflict(
            &format!(
                "create-only publication target already exists: {}",
                recovery_named_state.display()
            ),
            std::slice::from_ref(&recovery_named_state),
        ));
        assert!(!is_retryable_cache_state_conflict(
            &format!(
                "publication target differs from the expected preimage: {}",
                source.display()
            ),
            std::slice::from_ref(&state),
        ));
        assert!(!is_retryable_cache_state_conflict(
            &format!(
                "publication target differs from the expected preimage: {}.backup",
                state.display()
            ),
            std::slice::from_ref(&state),
        ));
        assert!(!is_retryable_cache_state_conflict(
            &format!(
                "publication target differs from the expected preimage: {}; rollback encountered: recovery failed",
                state.display()
            ),
            std::slice::from_ref(&state),
        ));
        let warnings =
            cache_cleanup_warnings(vec![format!("{} could not be removed", state.display())]);
        assert_eq!(warnings.len(), 1);
        assert!(!warnings[0].contains(state.to_string_lossy().as_ref()));
    }

    fn temp_root(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{nanos}"))
    }
}
