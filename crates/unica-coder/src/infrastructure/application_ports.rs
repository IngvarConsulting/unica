use crate::application::ports::{
    ApplicationPorts, FormatGuardCheck, FormatGuardError, HandlerOutcome, SupportGuardCheck,
};
use crate::application::source_navigation::{
    SourceChildrenRequest, SourceChildrenResult, SourceLocateRequest, SourceLocateResult,
    SourceResolveRequest, SourceResolveResult,
};
use crate::application::source_resources::{SourceReadRequest, SourceResourcesRequest};
use crate::application::{
    project_map, project_status, AdapterOutcome, ToolHandler, ToolSpec, TypedReadOutcome,
};
use crate::domain::cache::{CacheAccess, CacheReport};
use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::{
    CodeIntelligenceContext, CodeIntelligenceProvider, CodeIntelligenceReadRequest,
    CodeIntelligenceRegistry,
};
use crate::domain::events::DomainEvent;
use crate::domain::source_resources::{
    ResourceManifestPage, SourceReadResult, SourceResourceError,
};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::code_intelligence::{BslAnalyzerProvider, GitGrepProvider, RlmProvider};
use crate::infrastructure::internal_adapters::{
    BslAnalyzerMcpAdapter, CliAdapter, ConfigDumpInfoGitCheck, GitTrackingAdapter, RuntimeAdapter,
    RuntimeJobAdapter, StandardsAdapter,
};
use crate::infrastructure::native_operations::NativeOperationAdapter;
use crate::infrastructure::platform::full_dump_publication::{
    FullDumpInvocation, VerifiedFullDumpAdapter,
};
use crate::infrastructure::workspace_services::WorkspaceServiceManager;
use crate::infrastructure::workspace_state::WorkspaceStateRepository;
use serde_json::{Map, Value};
use std::borrow::Cow;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
pub(crate) struct InfrastructureApplicationPorts {
    source_resources: crate::infrastructure::platform_xml_resources::PlatformXmlResourceProvider,
}

impl InfrastructureApplicationPorts {
    pub(crate) fn new() -> Self {
        Self {
            source_resources:
                crate::infrastructure::platform_xml_resources::PlatformXmlResourceProvider::new(),
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

    fn resolve_code_intelligence_context(
        &self,
        context: &WorkspaceContext,
        args: &Map<String, Value>,
    ) -> Result<CodeIntelligenceContext, String> {
        let source_root = crate::infrastructure::source_roots::resolve_source_root(
            context,
            args.get("sourceDir").and_then(Value::as_str),
        )?;
        // `resolve_source_root` hands back a canonical path while the discovered
        // workspace keeps whatever the caller was standing in, so a symlinked cwd
        // (or the macOS `/var` -> `/private/var` alias) would make the two
        // incomparable and reject paths that live inside the source root. Put all
        // three into the same identity class before the application layer folds
        // them lexically.
        let mut workspace = context.clone();
        workspace.workspace_root = crate::infrastructure::source_roots::normalize_path_identity(
            &workspace.workspace_root,
        )?;
        workspace.cwd =
            crate::infrastructure::source_roots::normalize_path_identity(&workspace.cwd)?;
        Ok(CodeIntelligenceContext::new(workspace, source_root))
    }

    fn normalize_code_intelligence_read_request(
        &self,
        request: CodeIntelligenceReadRequest,
        context: &CodeIntelligenceContext,
    ) -> Result<CodeIntelligenceReadRequest, String> {
        normalize_code_intelligence_read_request(request, context)
    }

    fn code_intelligence_registry(&self) -> Result<CodeIntelligenceRegistry, String> {
        let providers: Vec<Arc<dyn CodeIntelligenceProvider>> = vec![
            Arc::new(RlmProvider::new()),
            Arc::new(BslAnalyzerProvider::new()),
            Arc::new(GitGrepProvider::new()),
        ];
        CodeIntelligenceRegistry::new(providers)
    }

    fn resolve_source_navigation(
        &self,
        request: SourceResolveRequest,
        context: &WorkspaceContext,
        cancellation: &CancellationToken,
    ) -> Result<SourceResolveResult, String> {
        crate::infrastructure::platform_xml_source_targets::resolve_platform_xml_source_navigation(
            context,
            &request,
            cancellation,
        )
    }

    fn children_source_navigation(
        &self,
        request: SourceChildrenRequest,
        context: &WorkspaceContext,
        cancellation: &CancellationToken,
    ) -> Result<SourceChildrenResult, String> {
        crate::infrastructure::platform_xml_source_targets::children_platform_xml_source_navigation(
            context,
            &request,
            cancellation,
        )
    }

    fn locate_source_navigation(
        &self,
        request: SourceLocateRequest,
        context: &WorkspaceContext,
        cancellation: &CancellationToken,
    ) -> Result<SourceLocateResult, String> {
        crate::infrastructure::platform_xml_source_targets::locate_platform_xml_source_path(
            context,
            &request,
            cancellation,
        )
    }

    fn source_resources(
        &self,
        request: SourceResourcesRequest,
        context: &WorkspaceContext,
        cancellation: &CancellationToken,
    ) -> Result<ResourceManifestPage, SourceResourceError> {
        self.source_resources
            .resources(request, context, cancellation)
    }

    fn read_source_resource(
        &self,
        request: SourceReadRequest,
        context: &WorkspaceContext,
        cancellation: &CancellationToken,
    ) -> Result<SourceReadResult, SourceResourceError> {
        self.source_resources.read(request, context, cancellation)
    }

    fn evaluate_support_guard(
        &self,
        spec: ToolSpec,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
    ) -> Result<SupportGuardCheck, String> {
        crate::infrastructure::support_guard::evaluate_support_guard(spec, args, context)
    }

    fn evaluate_format_guard(
        &self,
        spec: ToolSpec,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
    ) -> Result<FormatGuardCheck, FormatGuardError> {
        crate::infrastructure::format_guard::evaluate_format_guard(spec, args, context)
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
        if let Some(invocation) = verified_full_dump_invocation(spec, args, dry_run) {
            return VerifiedFullDumpAdapter::new()
                .invoke(spec.name, invocation, args, context, cancellation)
                .map(HandlerOutcome::plain);
        }
        match spec.handler {
            ToolHandler::Metadata { .. } => Err(format!(
                "{} metadata handler is not registered for infrastructure dispatch yet",
                spec.name
            )),
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
                Ok(typed_read(project_status(context, source_map, warning)))
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
                Ok(typed_read(project_map(source_map, warning)))
            }
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
                .invoke_cancellable_with_data(
                    spec.name,
                    args,
                    context,
                    dry_run,
                    spec.mutating,
                    cancellation,
                )
                .map(|outcome| match outcome.data {
                    Some(data) => HandlerOutcome::with_data(outcome.outcome, data),
                    None => HandlerOutcome::plain(outcome.outcome),
                }),
            ToolHandler::RuntimeJob { action } => RuntimeJobAdapter::invoke(
                action, spec.name, args, context, dry_run,
            )
            .map(|outcome| HandlerOutcome {
                adapter: outcome.outcome,
                data: None,
                job: outcome.job,
                events: Vec::new(),
                projected_events: Vec::new(),
                recorded_cache: None,
            }),
            ToolHandler::CodeIntelligence { .. } => Err(format!(
                "{} must be dispatched through the provider-neutral code intelligence registry",
                spec.name
            )),
            ToolHandler::SourceNavigation { .. } => Err(format!(
                "{} must be dispatched through the provider-neutral source navigation port",
                spec.name
            )),
            ToolHandler::SourceResources { .. } => Err(format!(
                "{} must be dispatched through the provider-neutral source resource port",
                spec.name
            )),
            ToolHandler::CodeAdapter {
                command: ["graph"] | ["analyze"],
            } => BslAnalyzerMcpAdapter::new()
                .invoke_cancellable(spec.name, args, context, dry_run, cancellation)
                .map(|analyzer| match analyzer.data {
                    Some(data) => HandlerOutcome::with_data(analyzer.outcome, data),
                    None => HandlerOutcome::plain(analyzer.outcome),
                }),
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
            ToolHandler::StandardsAdapter { operation } => {
                let standards = StandardsAdapter::invoke(operation, args);
                Ok(match standards.data {
                    Some(data) => HandlerOutcome::with_data(standards.outcome, data),
                    None => HandlerOutcome::plain(standards.outcome),
                })
            }
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

fn normalize_code_intelligence_read_request(
    mut request: CodeIntelligenceReadRequest,
    context: &CodeIntelligenceContext,
) -> Result<CodeIntelligenceReadRequest, String> {
    match &mut request {
        CodeIntelligenceReadRequest::Definition { module_hint, .. }
            if Path::new(module_hint).is_absolute()
                || module_hint.contains('/')
                || module_hint.contains('\\') =>
        {
            *module_hint = normalize_code_intelligence_path(module_hint, context)?;
        }
        CodeIntelligenceReadRequest::Outline { path, .. } => {
            *path = normalize_code_intelligence_path(path, context)?;
        }
        CodeIntelligenceReadRequest::Definition { .. }
        | CodeIntelligenceReadRequest::ObjectProfile { .. } => {}
    }
    Ok(request)
}

fn normalize_code_intelligence_path(
    raw: &str,
    context: &CodeIntelligenceContext,
) -> Result<String, String> {
    let source_root =
        crate::infrastructure::source_roots::normalize_path_identity(&context.source_root.path)?;
    let workspace_root = crate::infrastructure::source_roots::normalize_path_identity(
        &context.workspace.workspace_root,
    )?;
    let cwd = crate::infrastructure::source_roots::normalize_path_identity(&context.workspace.cwd)?;

    // Callers address 1C sources with either separator regardless of the host, so
    // the argument has to become host-neutral components before candidate
    // selection. Filesystem identity is resolved only after choosing the base:
    // this follows symlink components and keeps non-existent suffixes attached to
    // their nearest existing canonical ancestor.
    let host_neutral = separator_neutral_path(raw);
    let raw_path = Path::new(host_neutral.as_ref());
    let candidate = if raw_path.is_absolute() {
        normalize_lexical_path(raw_path)
    } else {
        let from_cwd = normalize_lexical_path(&cwd.join(raw_path));
        let from_workspace = normalize_lexical_path(&workspace_root.join(raw_path));
        if from_cwd.starts_with(&source_root) {
            from_cwd
        } else if from_workspace.starts_with(&source_root) {
            from_workspace
        } else if raw_path
            .components()
            .any(|component| component == Component::ParentDir)
        {
            from_cwd
        } else {
            normalize_lexical_path(&source_root.join(raw_path))
        }
    };
    let resolved = crate::infrastructure::source_roots::normalize_path_identity(&candidate)
        .map_err(|error| format!("failed to normalize code intelligence path `{raw}`: {error}"))?;
    if !resolved.starts_with(&workspace_root) || !resolved.starts_with(&source_root) {
        return Err(format!(
            "path `{raw}` resolves outside resolved source root {}",
            context.source_root.path.display()
        ));
    }
    let relative = resolved
        .strip_prefix(&source_root)
        .map_err(|error| format!("failed to normalize code intelligence path `{raw}`: {error}"))?;
    if relative.as_os_str().is_empty() {
        return Err(format!(
            "path `{raw}` resolves to the source root rather than a source file"
        ));
    }
    Ok(relative.to_string_lossy().replace('\\', "/"))
}

/// Renders a caller-supplied path so that both separators split components on
/// the running host. Windows already treats `\` as a separator, so the argument
/// is returned untouched there and only foreign backslashes are folded.
fn separator_neutral_path(raw: &str) -> Cow<'_, str> {
    if std::path::MAIN_SEPARATOR == '\\' || !raw.contains('\\') {
        Cow::Borrowed(raw)
    } else {
        Cow::Owned(raw.replace('\\', "/"))
    }
}

fn normalize_lexical_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

fn is_applied_full_dump(args: &Map<String, Value>, dry_run: bool) -> bool {
    !dry_run && args.get("mode").and_then(Value::as_str) == Some("full")
}

fn verified_full_dump_invocation(
    spec: ToolSpec,
    args: &Map<String, Value>,
    dry_run: bool,
) -> Option<FullDumpInvocation> {
    if !is_applied_full_dump(args, dry_run) {
        return None;
    }
    match spec.handler {
        ToolHandler::BuildRuntime { command, .. } if command == ["dump"] => {
            Some(FullDumpInvocation::BuildDump)
        }
        ToolHandler::RuntimeAdapter
            if args.get("operation").and_then(Value::as_str) == Some("dump") =>
        {
            Some(FullDumpInvocation::RuntimeExecute)
        }
        _ => None,
    }
}

/// Publishes a typed read through the envelope: `data` when the handler proved
/// a payload, plain outcome when it refused.
fn typed_read(read: TypedReadOutcome) -> HandlerOutcome {
    match read.data {
        Some(data) => HandlerOutcome::with_data(read.outcome, data),
        None => HandlerOutcome::plain(read.outcome),
    }
}

#[cfg(test)]
mod tests {
    use super::{normalize_code_intelligence_read_request, verified_full_dump_invocation};
    use crate::application::{RuntimeJobAction, ToolHandler, ToolSpec};
    use crate::domain::cache::CacheAccess;
    use crate::domain::code_intelligence::{CodeIntelligenceContext, CodeIntelligenceReadRequest};
    use crate::domain::source_roots::ResolvedSourceRoot;
    use crate::domain::workspace::WorkspaceContext;
    use crate::infrastructure::platform::full_dump_publication::FullDumpInvocation;
    use serde_json::{json, Map};
    use std::path::PathBuf;

    fn spec(name: &'static str, handler: ToolHandler) -> ToolSpec {
        ToolSpec {
            name,
            description: "test",
            mutating: true,
            cache_access: CacheAccess::default(),
            handler,
        }
    }

    #[test]
    fn applied_full_dump_routes_only_synchronous_public_entry_points_to_verified_adapter() {
        let build = spec(
            "unica.build.dump",
            ToolHandler::BuildRuntime {
                command: &["dump"],
                event: None,
            },
        );
        let runtime = spec("unica.runtime.execute", ToolHandler::RuntimeAdapter);
        let job = spec(
            "unica.runtime.job.start",
            ToolHandler::RuntimeJob {
                action: RuntimeJobAction::Start,
            },
        );
        let mut build_args = Map::new();
        build_args.insert("mode".to_string(), json!("full"));
        let mut runtime_args = build_args.clone();
        runtime_args.insert("operation".to_string(), json!("dump"));

        assert_eq!(
            verified_full_dump_invocation(build, &build_args, false),
            Some(FullDumpInvocation::BuildDump)
        );
        assert_eq!(
            verified_full_dump_invocation(runtime, &runtime_args, false),
            Some(FullDumpInvocation::RuntimeExecute)
        );
        assert_eq!(
            verified_full_dump_invocation(job, &runtime_args, false),
            None
        );
        assert_eq!(
            verified_full_dump_invocation(runtime, &runtime_args, true),
            None
        );
        runtime_args.insert("mode".to_string(), json!("partial"));
        assert_eq!(
            verified_full_dump_invocation(runtime, &runtime_args, false),
            None
        );
    }

    fn code_intelligence_context_for_paths() -> (PathBuf, CodeIntelligenceContext) {
        let root = std::env::temp_dir().join(format!(
            "unica-code-intelligence-paths-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let workspace = root.join("workspace");
        let source_root = workspace.join("src/cf");
        std::fs::create_dir_all(&source_root).unwrap();
        (
            root,
            CodeIntelligenceContext::new(
                WorkspaceContext {
                    cwd: workspace.clone(),
                    workspace_root: workspace.clone(),
                    cache_root: workspace.join(".build/unica"),
                    workspace_epoch: 1,
                },
                ResolvedSourceRoot {
                    source_set: Some("main".to_string()),
                    path: source_root,
                },
            ),
        )
    }

    #[test]
    fn code_intelligence_read_paths_are_normalized_to_the_resolved_source_root() {
        let (root, context) = code_intelligence_context_for_paths();
        let absolute = context
            .source_root
            .path
            .join("CommonModules/X/Ext/Module.bsl")
            .display()
            .to_string();
        for raw in [
            "CommonModules/X/Ext/Module.bsl".to_string(),
            "src/cf/CommonModules/X/Ext/Module.bsl".to_string(),
            absolute,
        ] {
            let normalized = normalize_code_intelligence_read_request(
                CodeIntelligenceReadRequest::Outline {
                    path: raw,
                    include_methods: true,
                },
                &context,
            )
            .unwrap();

            assert_eq!(
                normalized,
                CodeIntelligenceReadRequest::Outline {
                    path: "CommonModules/X/Ext/Module.bsl".to_string(),
                    include_methods: true,
                }
            );
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn code_intelligence_read_path_cannot_escape_through_either_separator() {
        let (root, context) = code_intelligence_context_for_paths();
        for raw in [
            "../../other/Module.bsl",
            r"..\..\other\Module.bsl",
            r"..\../other/Module.bsl",
            r"CommonModules\..\..\..\other\Module.bsl",
        ] {
            let error = normalize_code_intelligence_read_request(
                CodeIntelligenceReadRequest::Outline {
                    path: raw.to_string(),
                    include_methods: true,
                },
                &context,
            )
            .unwrap_err();

            assert!(
                error.contains("outside resolved source root"),
                "{raw}: {error}"
            );
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn code_intelligence_definition_module_hint_uses_the_same_path_identity() {
        let (root, context) = code_intelligence_context_for_paths();
        let error = normalize_code_intelligence_read_request(
            CodeIntelligenceReadRequest::Definition {
                name: "ОбщегоНазначения".to_string(),
                module_hint: r"..\..\other\Module.bsl".to_string(),
                limit: 50,
            },
            &context,
        )
        .unwrap_err();

        assert!(error.contains("outside resolved source root"), "{error}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn code_intelligence_read_path_accepts_workspace_cwd_and_windows_forms() {
        let (root, mut context) = code_intelligence_context_for_paths();
        context.workspace.cwd = context.workspace.workspace_root.join("tools");
        std::fs::create_dir_all(&context.workspace.cwd).unwrap();

        for raw in [
            r"CommonModules\X\Ext\Module.bsl",
            "src/cf/CommonModules/X/Ext/Module.bsl",
            "../src/cf/CommonModules/X/Ext/Module.bsl",
        ] {
            let normalized = normalize_code_intelligence_read_request(
                CodeIntelligenceReadRequest::Outline {
                    path: raw.to_string(),
                    include_methods: true,
                },
                &context,
            )
            .unwrap();
            assert_eq!(
                normalized,
                CodeIntelligenceReadRequest::Outline {
                    path: "CommonModules/X/Ext/Module.bsl".to_string(),
                    include_methods: true,
                }
            );
        }
        let _ = std::fs::remove_dir_all(root);
    }
}
