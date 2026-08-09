use crate::application::metadata::{MetaFailure, MetaInfoRequest, MetadataRequest};
use crate::application::ports::{
    ApplicationPorts, FormatGuardCheck, FormatGuardError, HandlerOutcome, MetaLocalInfo,
    MetaRelatedData, MetadataRead, MetadataValidationResult, MetadataValidationSubject,
    PreparedMetadataMutation, SupportGuardCheck,
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
use crate::infrastructure::metadata_operations::MetadataOperations;
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

    fn read_metadata_local(
        &self,
        request: &MetaInfoRequest,
        context: &WorkspaceContext,
        cancellation: &CancellationToken,
    ) -> Result<MetadataRead, MetaFailure> {
        MetadataOperations::read_local(request, context, cancellation)
    }

    fn read_metadata_related(
        &self,
        request: &MetaInfoRequest,
        local: &MetaLocalInfo,
        context: &WorkspaceContext,
        cancellation: &CancellationToken,
    ) -> MetaRelatedData {
        MetadataOperations::read_related(request, local, context, cancellation)
    }

    fn validate_metadata(
        &self,
        subject: &MetadataValidationSubject,
        context: &WorkspaceContext,
        cancellation: &CancellationToken,
    ) -> MetadataValidationResult {
        MetadataOperations::validate(subject, context, cancellation)
    }

    fn validate_metadata_read(
        &self,
        subject: &MetadataValidationSubject,
        context: &WorkspaceContext,
        cancellation: &CancellationToken,
    ) -> MetadataValidationResult {
        MetadataOperations::validate_read(subject, context, cancellation)
    }

    fn prepare_metadata_mutation(
        &self,
        request: &MetadataRequest,
        context: &WorkspaceContext,
        cancellation: &CancellationToken,
    ) -> Result<Box<dyn PreparedMetadataMutation>, MetaFailure> {
        MetadataOperations::prepare_mutation(request, context, cancellation)
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
                "{} must be dispatched through the provider-neutral metadata coordinator",
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
                diagnostics: None,
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
            ToolHandler::Documentation { operation } => {
                if operation != "search" {
                    return Err(format!("unknown documentation operation: {operation}"));
                }
                let request = crate::domain::documentation::DocumentationSearchRequest {
                    query: args
                        .get("query")
                        .and_then(Value::as_str)
                        .ok_or_else(|| "unica.documentation.search requires query".to_string())?
                        .to_string(),
                    source_kinds: Vec::new(),
                    limit: args
                        .get("limit")
                        .and_then(Value::as_u64)
                        .unwrap_or(20)
                        .min(200) as usize,
                    language: args
                        .get("language")
                        .and_then(Value::as_str)
                        .unwrap_or("ru")
                        .to_string(),
                };
                let requested_version = args.get("platformVersion").and_then(Value::as_str);
                // Порядок п.2 ADR-0029: явная версия в аргументе вызова,
                // затем версия, закреплённая проектом, затем численно
                // старшая найденная. Среднего уровня в коде не было, и
                // проект, закреплённый за 8.3.27, молча получал справку от
                // 8.5.4 — тот же вред, который запрещает п.3 записи.
                let project_version = project_platform_version(context);
                let installation_root = resolve_platform_installation_root(
                    requested_version,
                    project_version.as_deref(),
                );
                let context = crate::domain::documentation::DocumentationContext {
                    // Ограничение, по которому установку искали: поставщик
                    // называет его в отказе, когда установки не нашлось.
                    platform_version: requested_version.map(str::to_string).or(project_version),
                    installation_root,
                };
                let registry = documentation_registry()?;
                let data =
                    crate::application::documentation::search(&registry, &request, &context)?;
                Ok(HandlerOutcome::with_data(
                    AdapterOutcome::ok("unica.documentation.search completed"),
                    data,
                ))
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
        CodeIntelligenceReadRequest::Definition { .. } => {}
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

/// Composition root: the registry of documentation providers. Declaration
/// order here is the section order of the public result (ADR-0029 point 5),
/// and it is assembled here rather than in the domain layer so tests can
/// inject stand-in providers instead.
fn documentation_registry() -> Result<crate::domain::documentation::DocumentationRegistry, String> {
    use std::sync::Arc;

    crate::domain::documentation::DocumentationRegistry::new(vec![Arc::new(
        crate::infrastructure::platform_help::provider::PlatformSyntaxHelpProvider::new(),
    )
        as Arc<dyn crate::domain::documentation::DocumentationProvider>])
}

/// The platform version the project pins itself to: `tools.platform.version`
/// from `v8project.yaml`, overridden by the same key in the local
/// `v8project.local.yaml`. Same file, same key and same overlay order the
/// pinned runner already uses (`full_dump_publication`); a second platform
/// resolution mechanism must not appear in the project.
///
/// Absent, unreadable or malformed config means "no constraint", not an
/// error: documentation search is a read-only question about the platform,
/// and the runner is the place that refuses a broken project config loudly.
fn project_platform_version(context: &WorkspaceContext) -> Option<String> {
    let root = &context.workspace_root;
    configured_platform_version(&root.join("v8project.local.yaml"))
        .or_else(|| configured_platform_version(&root.join("v8project.yaml")))
}

fn configured_platform_version(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let yaml = serde_yaml::from_str::<serde_yaml::Value>(&text).ok()?;
    let version = yaml
        .as_mapping()?
        .get(serde_yaml::Value::String("tools".to_string()))?
        .as_mapping()?
        .get(serde_yaml::Value::String("platform".to_string()))?
        .as_mapping()?
        .get(serde_yaml::Value::String("version".to_string()))?
        .as_str()?;
    Some(version.to_string())
}

/// The platform installation root comes from the same root list the full-dump
/// publication adapter already uses: `default_platform_roots()` in
/// `infrastructure::platform::full_dump_publication`. A second installation
/// search mechanism must not appear in the project.
///
/// `UNICA_PLATFORM_HELP_DIR` is a test-only switch (see
/// `platform_help::installation`) and does not feed into this resolver.
fn resolve_platform_installation_root(
    requested: Option<&str>,
    project_version: Option<&str>,
) -> Option<std::path::PathBuf> {
    select_installation_root(
        &crate::infrastructure::platform::full_dump_publication::default_platform_roots(),
        requested,
        project_version,
    )
}

/// Pure root pick, split out of `resolve_platform_installation_root` so it can
/// be tested without the hard-coded platform roots. Roots are tried in the
/// declared order and the first one that answers closes the walk (ADR-0029
/// point 2).
///
/// The two constraints are not the same rule, because the two inputs do not
/// mean the same thing. The call argument is a *requested version* and must
/// match a directory name exactly: a caller asking for 8.3.27.2074 must never
/// be handed 8.3.27.2075. `tools.platform.version` is a *platform line* —
/// `references/tooling/runtime-build.md` documents it as constraining the
/// family, with a four-component value demanding an exact build — so it
/// selects the numerically newest installation under that line.
fn select_installation_root(
    roots: &[std::path::PathBuf],
    requested: Option<&str>,
    project_version: Option<&str>,
) -> Option<std::path::PathBuf> {
    for root in roots {
        let versions = version_directories(root);
        let selected = match (requested, project_version) {
            (Some(version), _) => select_platform_version(&versions, Some(version)),
            (None, Some(line)) => select_platform_line(&versions, line),
            (None, None) => select_platform_version(&versions, None),
        };
        if selected.is_some() {
            return selected;
        }
    }
    None
}

/// Subdirectories whose names have the shape of a platform version. Without
/// this filter every sibling of the version directories is a candidate: in
/// `/opt/1cv8` those are `1cv8`, `common` and `conf`, and since
/// `numeric_version_key` maps a non-numeric name to `vec![0]` while
/// `select_platform_version(_, None)` answers `Some` for any non-empty list,
/// the resolver used to return `/opt/1cv8/conf` — and, returning on the first
/// root that answers, never looked at the remaining roots. A single-component
/// name is refused for the same reason: `vec![9] > vec![8, 3, 27, 2074]`.
fn version_directories(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let Ok(children) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    children
        .flatten()
        .map(|child| child.path())
        .filter(|path| path.is_dir())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .and_then(version_components)
                .is_some()
        })
        .collect()
}

/// Numeric components of a version-shaped name, or `None` when the name is
/// not one: fewer than two dot-separated components, or a component that is
/// not a number.
fn version_components(name: &str) -> Option<Vec<u32>> {
    let parts = name
        .split('.')
        .map(|part| part.parse::<u32>().ok())
        .collect::<Option<Vec<u32>>>()?;
    (parts.len() >= 2).then_some(parts)
}

/// Newest installation under a platform line: the candidate's numeric
/// components must start with the configured ones. A four-component line
/// therefore demands the exact build, and a shorter one leaves the build free
/// — which is exactly how `tools.platform.version` is documented.
fn select_platform_line(versions: &[std::path::PathBuf], line: &str) -> Option<std::path::PathBuf> {
    let wanted = version_components(line)?;
    versions
        .iter()
        .filter(|path| numeric_version_key(path).starts_with(&wanted))
        .max_by_key(|path| numeric_version_key(path))
        .cloned()
}

/// Version-directory name as a numeric sort key: dot-separated components
/// compared as integers, not bytes. Byte/lexicographic comparison breaks the
/// moment a component's width changes — `"8.3.9.100"` sorts *after*
/// `"8.3.10.50"` under `str`/`PathBuf` ordering because `'1' < '9'` — and a
/// build-number digit rollover is a routine event over a machine's lifetime,
/// not a corner case. Silently answering from the wrong version is exactly
/// the "neighbouring version substituted" failure ADR-0029 point 3 forbids.
/// A non-numeric or missing component parses as 0; that only matters for a
/// directory name that is not a version at all, and `version_directories`
/// keeps those out of the listing this feeds from.
fn numeric_version_key(path: &std::path::Path) -> Vec<u32> {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| {
            name.split('.')
                .map(|part| part.parse::<u32>().unwrap_or(0))
                .collect()
        })
        .unwrap_or_default()
}

/// Pure version pick, split out of `resolve_platform_installation_root` so it
/// can be tested without touching the filesystem or the hard-coded platform
/// roots: an explicit version must match a directory name exactly (a
/// three-component prefix like `8.3.27` must not silently resolve to
/// `8.3.27.2074`, since a patch mismatch changes hundreds of API names), and
/// without one the numerically newest entry wins — order-independent, so the
/// caller does not need to pre-sort.
fn select_platform_version(
    versions: &[std::path::PathBuf],
    requested: Option<&str>,
) -> Option<std::path::PathBuf> {
    match requested {
        Some(version) => versions
            .iter()
            .find(|path| path.file_name().and_then(|name| name.to_str()) == Some(version))
            .cloned(),
        None => versions
            .iter()
            .max_by_key(|path| numeric_version_key(path))
            .cloned(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        documentation_registry, normalize_code_intelligence_read_request, project_platform_version,
        select_installation_root, select_platform_version, verified_full_dump_invocation,
    };
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

    #[test]
    fn documentation_registry_wires_the_platform_syntax_help_provider() {
        // The composition root is the only place a documentation provider is
        // constructed; a registry left empty (forgot to wire the provider) or
        // wired to the wrong provider must fail this, not merely compile.
        let registry = documentation_registry().expect("registry constructs");
        let providers: Vec<_> = registry.providers().collect();
        assert_eq!(providers.len(), 1, "exactly one provider is wired today");
        assert_eq!(providers[0].id().to_string(), "platform-syntax-help");
        assert_eq!(
            providers[0].corpora().len(),
            2,
            "syntax-context and platform-guides"
        );
    }

    #[test]
    fn select_platform_version_matches_the_requested_directory_name_exactly() {
        let versions = vec![
            PathBuf::from("/opt/1cv8/8.3.24.1234"),
            PathBuf::from("/opt/1cv8/8.3.27.2074"),
            PathBuf::from("/opt/1cv8/8.5.1.1451"),
        ];
        assert_eq!(
            select_platform_version(&versions, Some("8.3.27.2074")),
            Some(PathBuf::from("/opt/1cv8/8.3.27.2074"))
        );
    }

    #[test]
    fn select_platform_version_requires_an_exact_directory_name_match() {
        // A three-component prefix of a real directory must not resolve: a
        // substring/starts_with implementation would wrongly accept it, and a
        // patch mismatch changes hundreds of API names (ADR-0029 point 3).
        let versions = vec![PathBuf::from("/opt/1cv8/8.3.27.2074")];
        assert_eq!(select_platform_version(&versions, Some("8.3.27")), None);
    }

    #[test]
    fn select_platform_version_returns_none_when_the_requested_version_is_absent() {
        let versions = vec![PathBuf::from("/opt/1cv8/8.3.24.1234")];
        assert_eq!(select_platform_version(&versions, Some("9.9.9.9999")), None);
    }

    #[test]
    fn select_platform_version_without_a_request_picks_the_numerically_newest_entry() {
        // Byte/lexicographic order breaks the moment a dot-separated
        // component's width changes: "8.3.10.50" < "8.3.9.100" as raw
        // strings ('1' < '9' at the third component, the first place they
        // differ), even though 10 > 9 numerically. A build-number digit
        // rollover is a routine event over a machine's lifetime, not a
        // corner case. Fed in the order a byte sort would actually produce
        // (ascending lexicographically: "8.3.10.50" sorts first), a
        // `.last()`-over-byte-order pick would silently return the OLDER
        // version here — exactly the "neighbouring version substituted"
        // failure ADR-0029 point 3 forbids.
        let versions = vec![
            PathBuf::from("/opt/1cv8/8.3.10.50"),
            PathBuf::from("/opt/1cv8/8.3.9.100"),
        ];
        assert_eq!(
            select_platform_version(&versions, None),
            Some(PathBuf::from("/opt/1cv8/8.3.10.50"))
        );
    }

    #[test]
    fn select_platform_version_returns_none_for_an_empty_list() {
        assert_eq!(select_platform_version(&[], None), None);
        assert_eq!(select_platform_version(&[], Some("8.3.27.2074")), None);
    }

    /// `select_platform_version` was the only tested half; the resolver that
    /// builds its candidate list was covered by nothing. Every sibling of the
    /// version directories used to qualify: in `/opt/1cv8` those are `1cv8`,
    /// `common` and `conf`, and a single-component `9` outranks every real
    /// version because `vec![9] > vec![8, 3, 27, 2074]`. Since the walk
    /// returns on the first root that answers, one such name under the first
    /// root hid every later root as well.
    #[test]
    fn installation_root_skips_names_that_are_not_versions_and_keeps_walking() {
        let dir = tempfile::tempdir().expect("каталог");
        let noise = dir.path().join("noise");
        for name in ["1cv8", "common", "conf", "9"] {
            std::fs::create_dir_all(noise.join(name)).expect("служебный каталог");
        }
        let installed = dir.path().join("installed");
        std::fs::create_dir_all(installed.join("8.3.27.2074")).expect("каталог версии");

        assert_eq!(
            select_installation_root(&[noise, installed.clone()], None, None),
            Some(installed.join("8.3.27.2074")),
            "корень без версий не должен закрывать перебор своим служебным каталогом"
        );
    }

    /// ADR-0029 point 2 orders the three inputs: the explicit call argument,
    /// then the version the project pins itself to, then the numerically
    /// newest installation. The middle level did not exist, so a project
    /// pinned to 8.3.27 was answered from 8.5.4 without a diagnostic.
    #[test]
    fn project_platform_line_sits_between_the_call_argument_and_the_newest_install() {
        let dir = tempfile::tempdir().expect("каталог");
        let root = dir.path().join("1cv8");
        for version in ["8.3.27.2074", "8.5.4.1306"] {
            std::fs::create_dir_all(root.join(version)).expect("каталог версии");
        }
        let roots = [root.clone()];

        assert_eq!(
            select_installation_root(&roots, None, None),
            Some(root.join("8.5.4.1306")),
            "без ограничений побеждает численно старшая"
        );
        assert_eq!(
            select_installation_root(&roots, None, Some("8.3.27")),
            Some(root.join("8.3.27.2074")),
            "версия проекта ограничивает семейство"
        );
        assert_eq!(
            select_installation_root(&roots, Some("8.5.4.1306"), Some("8.3.27")),
            Some(root.join("8.5.4.1306")),
            "явный аргумент вызова сильнее версии проекта"
        );
        assert_eq!(
            select_installation_root(&roots, None, Some("8.4")),
            None,
            "закреплённой семьи нет — отказ, а не подстановка соседней (ADR-0029 point 3)"
        );
    }

    /// The project's own platform pin lives in `v8project.yaml` under
    /// `tools.platform.version`, and the machine-specific
    /// `v8project.local.yaml` overrides it — the same file, key and overlay
    /// order the pinned runner already reads.
    #[test]
    fn project_platform_version_reads_the_config_and_prefers_the_local_overlay() {
        let dir = tempfile::tempdir().expect("каталог");
        let workspace = dir.path().to_path_buf();
        let context = WorkspaceContext {
            cwd: workspace.clone(),
            workspace_root: workspace.clone(),
            cache_root: workspace.join(".build/unica"),
            workspace_epoch: 1,
        };

        assert_eq!(
            project_platform_version(&context),
            None,
            "без конфигурации проект ничего не закрепляет"
        );

        std::fs::write(
            workspace.join("v8project.yaml"),
            "tools:\n  platform:\n    version: \"8.3.27\"\n",
        )
        .expect("конфигурация проекта");
        assert_eq!(
            project_platform_version(&context).as_deref(),
            Some("8.3.27"),
            "tools.platform.version читается из v8project.yaml"
        );

        std::fs::write(
            workspace.join("v8project.local.yaml"),
            "tools:\n  platform:\n    version: \"8.3.27.2074\"\n",
        )
        .expect("локальное перекрытие");
        assert_eq!(
            project_platform_version(&context).as_deref(),
            Some("8.3.27.2074"),
            "локальное перекрытие сильнее основной конфигурации"
        );
    }
}
