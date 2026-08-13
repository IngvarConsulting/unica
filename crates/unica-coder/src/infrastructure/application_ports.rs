use crate::application::metadata::{MetaFailure, MetaInfoRequest, MetadataRequest};
use crate::application::ports::{
    ApplicationPorts, FormatGuardCheck, FormatGuardError, HandlerOutcome, MetaLocalInfo,
    MetaRelatedData, MetadataRead, MetadataValidationResult, MetadataValidationSubject,
    PreparedMetadataMutation, PreparedToolInvocation, SupportGuardCheck,
};
use crate::application::source_navigation::{
    SourceChildrenRequest, SourceChildrenResult, SourceLocateRequest, SourceLocateResult,
    SourceResolveRequest, SourceResolveResult,
};
use crate::application::source_resources::{SourceReadRequest, SourceResourcesRequest};
use crate::application::{
    project_map, project_status, AdapterOutcome, InvocationMode, ToolExecution, ToolHandler,
    ToolSpec, TypedReadOutcome,
};
use crate::domain::cache::{CacheAccess, CacheReport};
use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::{
    CodeIntelligenceContext, CodeIntelligenceProvider, CodeIntelligenceReadRequest,
    CodeIntelligenceRegistry, ProviderDeadline,
};
use crate::domain::events::DomainEvent;
use crate::domain::operational_config::{OperationalConfig, OperationalConfigDiagnostic};
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
use crate::infrastructure::native_operations::subsystem;
use crate::infrastructure::native_operations::typed_result::NativeInvocationContext;
use crate::infrastructure::native_operations::NativeOperationAdapter;
use crate::infrastructure::platform::full_dump_publication::{
    FullDumpInvocation, VerifiedFullDumpAdapter,
};
use crate::infrastructure::support_state::{
    SupportStateReaderFactory, WorkspaceSupportStateReaderFactory,
};
use crate::infrastructure::workspace_services::WorkspaceServiceManager;
use crate::infrastructure::workspace_state::WorkspaceStateRepository;
use serde_json::{json, Map, Value};
use std::borrow::Cow;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

const NATIVE_TYPED_INVOCATION_DEADLINE: Duration = Duration::from_secs(5);

fn adapter_dry_run(spec: ToolSpec, mode: InvocationMode) -> Result<bool, String> {
    match (spec.execution, mode) {
        (ToolExecution::Mutation, InvocationMode::Preview) => Ok(true),
        (ToolExecution::Mutation, InvocationMode::Apply) => Ok(false),
        (ToolExecution::Read, InvocationMode::Read) => Ok(false),
        _ => Err(format!("invalid invocation mode for {}", spec.name)),
    }
}

pub(crate) struct InfrastructureApplicationPorts {
    source_resources: crate::infrastructure::platform_xml_resources::PlatformXmlResourceProvider,
    support_state_readers: Arc<dyn SupportStateReaderFactory>,
}

impl InfrastructureApplicationPorts {
    pub(crate) fn new() -> Self {
        Self {
            source_resources:
                crate::infrastructure::platform_xml_resources::PlatformXmlResourceProvider::new(),
            support_state_readers: Arc::new(WorkspaceSupportStateReaderFactory),
        }
    }

    #[cfg(test)]
    fn with_support_reader_factory(
        support_state_readers: Arc<dyn SupportStateReaderFactory>,
    ) -> Self {
        Self {
            source_resources:
                crate::infrastructure::platform_xml_resources::PlatformXmlResourceProvider::new(),
            support_state_readers,
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
        mode: InvocationMode,
        context: &WorkspaceContext,
    ) -> Result<(), String> {
        let dry_run = adapter_dry_run(spec, mode)?;
        crate::infrastructure::tool_context::validate_tool_context(spec, args, dry_run, context)
    }

    fn load_operational_config(
        &self,
        context: &WorkspaceContext,
    ) -> Result<OperationalConfig, OperationalConfigDiagnostic> {
        crate::infrastructure::operational_config::load_operational_config(&context.workspace_root)
    }

    fn read_metadata_local(
        &self,
        request: &MetaInfoRequest,
        context: &WorkspaceContext,
        cancellation: &CancellationToken,
    ) -> Result<MetadataRead, MetaFailure> {
        let support_reader = self.support_state_readers.create(context);
        MetadataOperations::read_local(request, context, cancellation, support_reader.as_ref())
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

    fn prepare_tool_invocation(
        &self,
        spec: ToolSpec,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
        mode: InvocationMode,
        cancellation: &CancellationToken,
        deadline: ProviderDeadline,
    ) -> Result<PreparedToolInvocation, String> {
        // Validate the execution/mode pair before any preparation work. The
        // preparation path itself does not branch on preview state.
        adapter_dry_run(spec, mode)?;
        let ToolHandler::NativeOperation {
            operation: "subsystem-info",
            ..
        } = spec.handler
        else {
            return Ok(PreparedToolInvocation::empty());
        };

        if let Err(error) = subsystem::ensure_subsystem_info_control(cancellation, deadline) {
            return prepared_subsystem_info_failure(error.to_string(), false);
        }
        let support_reader = self.support_state_readers.create(context);
        let prepared = match subsystem::prepare_subsystem_info(
            args,
            context,
            cancellation,
            deadline,
            support_reader.as_ref(),
        ) {
            Ok(prepared) => prepared,
            Err(error) if error.is_control() => {
                return prepared_subsystem_info_failure(error.to_string(), false);
            }
            Err(error) => return prepared_subsystem_info_failure(error.to_string(), true),
        };
        let format_guard =
            crate::infrastructure::format_guard::evaluate_prepared_subsystem_info_format_guard(
                spec,
                &prepared.format_documents,
            )
            .map_err(|error| error.to_string())?;
        if let Err(error) = subsystem::ensure_subsystem_info_control(cancellation, deadline) {
            let mut failure = prepared_subsystem_info_failure(error.to_string(), false)?;
            failure.format_guard = Some(format_guard);
            return Ok(failure);
        }
        let native = NativeOperationAdapter::prepared_subsystem_info_with_data(prepared.execution)?;
        Ok(PreparedToolInvocation {
            format_guard: Some(format_guard),
            handler: Some(native_handler_outcome(native)),
        })
    }

    fn invoke_handler(
        &self,
        spec: ToolSpec,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
        mode: InvocationMode,
        cancellation: &CancellationToken,
    ) -> Result<HandlerOutcome, String> {
        self.invoke_handler_with_operational_config(spec, args, context, mode, None, cancellation)
    }

    fn invoke_handler_with_operational_config(
        &self,
        spec: ToolSpec,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
        mode: InvocationMode,
        operational_config: Option<&OperationalConfig>,
        cancellation: &CancellationToken,
    ) -> Result<HandlerOutcome, String> {
        let dry_run = adapter_dry_run(spec, mode)?;
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
                if operation == "subsystem-info" {
                    return Err(format!(
                        "{} requires the controlled prepared invocation path",
                        spec.name
                    ));
                }
                let support_reader = self.support_state_readers.create(context);
                NativeOperationAdapter::invoke_with_data(
                    operation,
                    spec.name,
                    args,
                    context,
                    dry_run,
                    spec.execution.is_mutating(),
                    NativeInvocationContext::new(
                        support_reader.as_ref(),
                        cancellation,
                        ProviderDeadline::new(Instant::now() + NATIVE_TYPED_INVOCATION_DEADLINE),
                    ),
                )
                .map(|outcome| {
                    let mut handler = match outcome.data {
                        Some(data) => HandlerOutcome::with_data(outcome.adapter, data),
                        None => HandlerOutcome::plain(outcome.adapter),
                    };
                    handler.events = outcome.events;
                    handler.recorded_cache = outcome.recorded_cache;
                    handler
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
                        spec.execution.is_mutating(),
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
                    spec.execution.is_mutating(),
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
            ToolHandler::Diagnostics => Err(format!(
                "{} must be dispatched through the provider-neutral diagnostics coordinator",
                spec.name
            )),
            ToolHandler::CodeAdapter {
                command: ["graph"] | ["analyze"],
            } => BslAnalyzerMcpAdapter::new()
                .invoke_cancellable_with_operational_config(
                    spec.name,
                    args,
                    context,
                    dry_run,
                    operational_config,
                    cancellation,
                )
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
                        spec.execution.is_mutating(),
                        cancellation,
                    )
                    .map(HandlerOutcome::plain)
            }
            ToolHandler::StandardsAdapter { operation } => {
                // Фасады делят с поставщиком v8std движок И политику: запрет в
                // unica.toml выключает оба маршрута одним файлом (ADR-0032
                // п.4, следствие 2). Нечитаемая политика — жёсткий отказ, а не
                // молчаливое разрешение: это файл запрета.
                let policy =
                    crate::infrastructure::documentation_policy::DocumentationPolicy::load(
                        &context.workspace_root,
                        DOCUMENTATION_PROVIDER_IDS,
                    )
                    .map_err(|error| format!("{}: {error}", spec.name))?;
                if policy.network("v8std")
                    == crate::infrastructure::documentation_policy::NetworkAccess::Deny
                {
                    return Err(format!(
                        "{}: сетевой выход v8std запрещён политикой unica.toml (policy-denied)",
                        spec.name
                    ));
                }
                let endpoint =
                    crate::infrastructure::standards_documentation::resolve_standards_endpoint(
                        &policy,
                    );
                let standards = StandardsAdapter::invoke(operation, args, &endpoint);
                Ok(match standards.data {
                    Some(data) => HandlerOutcome::with_data(standards.outcome, data),
                    None => HandlerOutcome::plain(standards.outcome),
                })
            }
            ToolHandler::Documentation { operation: "get" } => {
                let document_id = args
                    .get("documentId")
                    .and_then(Value::as_str)
                    .ok_or_else(|| "unica.documentation.get requires documentId".to_string())?;
                if document_id.trim().is_empty() {
                    return Err(
                        "unica.documentation.get requires a non-blank documentId".to_string()
                    );
                }
                let language = args
                    .get("language")
                    .and_then(Value::as_str)
                    .unwrap_or("ru")
                    .to_string();
                let registry = documentation_registry(context, cancellation)?;
                let requested_version = args.get("platformVersion").and_then(Value::as_str);
                let context = documentation_context(
                    &crate::infrastructure::platform::full_dump_publication::default_platform_roots(
                    ),
                    requested_version,
                    context,
                );
                let data = crate::application::documentation::get(
                    &registry,
                    document_id,
                    &language,
                    &context,
                )?;
                Ok(HandlerOutcome::with_data(
                    AdapterOutcome::ok("unica.documentation.get completed"),
                    data,
                ))
            }
            ToolHandler::Documentation { operation } => {
                if operation != "search" {
                    return Err(format!("unknown documentation operation: {operation}"));
                }
                // Фильтр по смыслу источника (ADR-0032 п.5). Чужое значение —
                // отказ с перечнем допустимых: молча проигнорированный фильтр
                // отвечал бы стандартами на просьбу «только справка платформы».
                let source_kinds = match args.get("sourceKinds") {
                    None => Vec::new(),
                    Some(value) => value
                        .as_array()
                        .ok_or_else(|| {
                            "unica.documentation.search: sourceKinds must be an array".to_string()
                        })?
                        .iter()
                        .map(|entry| {
                            entry
                                .as_str()
                                .and_then(crate::domain::documentation::SourceKind::parse)
                                .ok_or_else(|| {
                                    // Нестроковое значение называется самим
                                    // JSON-значением, а не пустой строкой.
                                    format!(
                                        "unica.documentation.search: unknown sourceKinds value {entry}; allowed: platform-help, development-standard, configuration-documentation"
                                    )
                                })
                        })
                        .collect::<Result<Vec<_>, String>>()?,
                };
                let request = crate::domain::documentation::DocumentationSearchRequest {
                    query: args
                        .get("query")
                        .and_then(Value::as_str)
                        .ok_or_else(|| "unica.documentation.search requires query".to_string())?
                        .to_string(),
                    source_kinds,
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
                if request.query.trim().is_empty() {
                    return Err("unica.documentation.search requires a non-blank query".to_string());
                }
                // Реестр собирается по рабочему пространству ДО того, как имя
                // `context` затенит DocumentationContext: политика unica.toml —
                // файлы проекта, и нечитаемая политика — отказ вызова.
                let registry = documentation_registry(context, cancellation)?;
                let requested_version = args.get("platformVersion").and_then(Value::as_str);
                let context = documentation_context(
                    &crate::infrastructure::platform::full_dump_publication::default_platform_roots(
                    ),
                    requested_version,
                    context,
                );
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
        mode: InvocationMode,
        cache_access: CacheAccess,
    ) -> Result<CacheReport, String> {
        WorkspaceStateRepository::new(context).report(
            context,
            events,
            mode.is_preview(),
            cache_access,
        )
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

fn native_handler_outcome(
    outcome: crate::infrastructure::native_operations::typed_result::NativeOperationResult,
) -> HandlerOutcome {
    match outcome.data {
        Some(data) => HandlerOutcome::with_data(outcome.adapter, data),
        None => HandlerOutcome::plain(outcome.adapter),
    }
}

fn prepared_subsystem_info_failure(
    error: String,
    provider_unavailable: bool,
) -> Result<PreparedToolInvocation, String> {
    let native = NativeOperationAdapter::prepared_subsystem_info_with_data(
        subsystem::subsystem_info_failure(error),
    )?;
    let mut handler = native_handler_outcome(native);
    if provider_unavailable {
        handler.diagnostics = Some(json!([{
            "code": "provider_unavailable",
            "severity": "error",
            "message": "registered subsystem topology is unavailable"
        }]));
    }
    Ok(PreparedToolInvocation {
        format_guard: Some(FormatGuardCheck::Allow),
        handler: Some(handler),
    })
}

/// Идентификаторы поставщиков документации — единственный перечень, против
/// которого политика `unica.toml` проверяет свои секции `[providers.*]`.
const DOCUMENTATION_PROVIDER_IDS: &[&str] = &[
    "configuration-help",
    "platform-syntax-help",
    "kb-1ci",
    "v8std",
];

/// Composition root: the registry of documentation providers. Declaration
/// order here is the section order of the public result (ADR-0029 point 5):
/// локальная справка платформы раньше сетевых поставщиков, справка раньше
/// стандартов. Собирается здесь, а не в домене, чтобы тесты внедряли
/// подмены; политика читается на каждый вызов — она из файлов проекта.
fn documentation_registry(
    context: &WorkspaceContext,
    cancellation: &crate::domain::cancellation::CancellationToken,
) -> Result<crate::domain::documentation::DocumentationRegistry, String> {
    use std::sync::Arc;

    #[cfg(test)]
    {
        let _ = cancellation;
        if let Some(stand_in) = documentation_registry_stand_in() {
            return crate::domain::documentation::DocumentationRegistry::new(vec![stand_in]);
        }
    }
    let policy = crate::infrastructure::documentation_policy::DocumentationPolicy::load(
        &context.workspace_root,
        DOCUMENTATION_PROVIDER_IDS,
    )?;
    let endpoint =
        crate::infrastructure::standards_documentation::resolve_standards_endpoint(&policy);
    // Справка конфигурации читает source-set'ы рабочего пространства; битая
    // настройка проекта — отказ вызова, как и битая политика: неясность — отказ.
    let source_map = crate::infrastructure::project_sources::discover_project_source_map(
        &context.workspace_root,
    )?;
    let source_sets = source_map
        .source_sets
        .iter()
        .map(|source_set| {
            (
                source_set.name.clone(),
                context.workspace_root.join(&source_set.path),
            )
        })
        .collect();
    crate::domain::documentation::DocumentationRegistry::new(vec![
        Arc::new(
            crate::infrastructure::configuration_help::ConfigurationHelpProvider { source_sets },
        ) as Arc<dyn crate::domain::documentation::DocumentationProvider>,
        Arc::new(crate::infrastructure::platform_help::provider::PlatformSyntaxHelpProvider::new()),
        Arc::new(crate::infrastructure::kb_1ci::Kb1ciProvider {
            base: crate::infrastructure::kb_1ci::KB_BASE.to_string(),
            network: policy.network("kb-1ci"),
            transport: Arc::new(crate::infrastructure::kb_1ci::UreqKbTransport),
            // Токен вызова: сетевой обход обязан отменяться вместе с вызовом
            // MCP, поэтому реестр собирается на вызов, а не на процесс.
            cancellation: cancellation.clone(),
            cache_ttl: crate::infrastructure::kb_1ci::KB_CACHE_TTL,
            lexicon: Arc::new(crate::infrastructure::kb_1ci::InstallationLexiconSource),
        }),
        Arc::new(
            crate::infrastructure::standards_documentation::V8StdDocumentationProvider {
                search_cache_ttl:
                    crate::infrastructure::standards_documentation::V8STD_SEARCH_CACHE_TTL,
                endpoint,
                network: policy.network("v8std"),
                http: crate::infrastructure::internal_adapters::shared_http_client(),
                cancellation: cancellation.clone(),
            },
        ),
    ])
}

/// Подмена реестра для тестов — та самая, которую допускает п.5 ADR-0029
/// («реестр собирается в корне композиции и допускает внедрение подмен для
/// тестов»). Без неё ветку диспетчера `unica.documentation.search` не
/// проверить: настоящий поставщик отвечает по установкам МАШИНЫ, и тест не
/// выбирает ни их состав, ни их наличие, поэтому наблюдать через него, что
/// аргументы вызова дошли до запроса и контекста, нельзя.
#[cfg(test)]
static DOCUMENTATION_REGISTRY_STAND_IN: std::sync::Mutex<
    Option<std::sync::Arc<dyn crate::domain::documentation::DocumentationProvider>>,
> = std::sync::Mutex::new(None);

#[cfg(test)]
fn documentation_registry_stand_in(
) -> Option<std::sync::Arc<dyn crate::domain::documentation::DocumentationProvider>> {
    DOCUMENTATION_REGISTRY_STAND_IN
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
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
    configured_platform_key(path, "version")
}

/// The second half of the runner's platform pin: `tools.platform.path` from
/// the same files, with the same per-key local overlay. The reference config
/// (`references/tooling/runtime-build.md`) stores it in
/// `v8project.local.yaml` because the path is machine-specific.
fn project_platform_path(context: &WorkspaceContext) -> Option<std::path::PathBuf> {
    let root = &context.workspace_root;
    let configured = configured_platform_key(&root.join("v8project.local.yaml"), "path")
        .or_else(|| configured_platform_key(&root.join("v8project.yaml"), "path"))?;
    let path = std::path::PathBuf::from(configured);
    // Относительный путь читается от корня проекта — так же absolutize
    // раннера читает его от каталога конфигурации.
    Some(if path.is_absolute() {
        path
    } else {
        root.join(path)
    })
}

fn configured_platform_key(path: &Path, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let yaml = serde_yaml::from_str::<serde_yaml::Value>(&text).ok()?;
    let value = yaml
        .as_mapping()?
        .get(serde_yaml::Value::String("tools".to_string()))?
        .as_mapping()?
        .get(serde_yaml::Value::String("platform".to_string()))?
        .as_mapping()?
        .get(serde_yaml::Value::String(key.to_string()))?
        .as_str()?;
    Some(value.to_string())
}

/// Installation named directly by the project's `tools.platform.path`. The
/// pin replaces the roots walk — same as the runner, where the hint replaces
/// the default candidate list — so a mismatch is a refusal, not a silent
/// fall-through to a neighbouring installation (ADR-0029 point 3).
///
/// The reference config points the pin at `<version>/bin` (executables live
/// there on Windows); the version directory is its parent, and the version is
/// that directory's name. A pinned directory whose name is not version-shaped
/// cannot prove which version answered, so it is refused rather than trusted.
fn pinned_installation_root(
    pin: &Path,
    requested: Option<&str>,
    project_version: Option<&str>,
) -> Option<std::path::PathBuf> {
    let candidate = if pin.file_name() == Some(std::ffi::OsStr::new("bin")) {
        pin.parent()?.to_path_buf()
    } else {
        pin.to_path_buf()
    };
    let name = candidate.file_name()?.to_str()?;
    let components = version_components(name)?;
    if let Some(version) = requested {
        if name != version {
            return None;
        }
    }
    if let Some(line) = project_version {
        let wanted = version_components(line)?;
        if !components.starts_with(&wanted) {
            return None;
        }
    }
    Some(candidate)
}

/// The documentation context the dispatcher hands to the provider registry:
/// which installation to read and which version constraint that search was
/// made under.
///
/// Split out of the dispatcher and taking `roots` as an argument so the
/// project-pin wiring can be tested without the hard-coded platform roots.
/// Deleting the `project_platform_version` call is caught by the compiler and
/// by clippy; passing `None` in its place is not, and that mutation is exactly
/// the ADR-0029 point 3 harm — a project pinned to 8.3.27 silently answered
/// from 8.5.4 while the reply still said 8.3.27.
///
/// Precedence is ADR-0029 point 2: the explicit call argument, then the
/// project's own pin, then the numerically newest installation found. A
/// `tools.platform.path` pin names the installation directly and replaces the
/// roots walk; the version constraints still apply to it.
///
/// `UNICA_PLATFORM_HELP_DIR` is a test-only switch (see
/// `platform_help::real_installation`) and does not feed into this resolver.
fn documentation_context(
    roots: &[std::path::PathBuf],
    requested: Option<&str>,
    workspace: &WorkspaceContext,
) -> crate::domain::documentation::DocumentationContext {
    let project_version = project_platform_version(workspace);
    let installation_root = match project_platform_path(workspace) {
        Some(pin) => pinned_installation_root(&pin, requested, project_version.as_deref()),
        None => select_installation_root(roots, requested, project_version.as_deref()),
    };
    crate::domain::documentation::DocumentationContext {
        installation_root,
        // Ограничение, по которому установку искали: поставщик называет его в
        // отказе, когда установки не нашлось.
        platform_version: requested.map(str::to_string).or(project_version),
    }
}

/// Pure root pick, split out of `documentation_context`'s resolution so it can
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

/// Pure version pick, split out of `select_installation_root` so it can be
/// tested without touching the filesystem or the hard-coded platform
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
        documentation_context, documentation_registry, normalize_code_intelligence_read_request,
        project_platform_version, select_installation_root, select_platform_version,
        verified_full_dump_invocation,
    };
    use crate::application::metadata::MetaInfoRequest;
    use crate::application::{InvocationMode, RuntimeJobAction, ToolHandler, ToolSpec};
    use crate::domain::cancellation::CancellationToken;
    use crate::domain::code_intelligence::{
        CodeIntelligenceContext, CodeIntelligenceReadRequest, ProviderDeadline,
    };
    use crate::domain::source_roots::ResolvedSourceRoot;
    use crate::domain::source_target::{
        MetadataAddress, ResolvedTarget, TargetKind, PLATFORM_XML_8_3_27_FORMAT_2_20,
    };
    use crate::domain::support_state::{
        ConfigurationSupportData, ConfigurationSupportState, ObjectSupportData, ObjectSupportState,
        ResolvedSubsystemTarget, SupportReadError, SupportStateReader,
    };
    use crate::domain::workspace::WorkspaceContext;
    use crate::infrastructure::native_operations::typed_result::NativeInvocationContext;
    use crate::infrastructure::native_operations::NativeOperationAdapter;
    use crate::infrastructure::platform::full_dump_publication::FullDumpInvocation;
    use crate::infrastructure::platform::secure_read::{
        with_secure_tree_test_hook, SecureTreePhase,
    };
    use crate::infrastructure::support_state::SupportStateReaderFactory;
    use serde_json::{json, Map};
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    fn spec(name: &'static str, handler: ToolHandler) -> ToolSpec {
        let mut spec = crate::application::tools()
            .into_iter()
            .find(|spec| spec.name == name)
            .unwrap_or_else(|| panic!("{name} must be registered"));
        spec.handler = handler;
        spec
    }

    #[derive(Clone)]
    struct RecordingSupportStateReaderFactory {
        calls: Arc<Mutex<Vec<(&'static str, ResolvedTarget)>>>,
        subsystem_calls: Arc<Mutex<Vec<ResolvedSubsystemTarget>>>,
    }

    struct RecordingSupportStateReader {
        calls: Arc<Mutex<Vec<(&'static str, ResolvedTarget)>>>,
        subsystem_calls: Arc<Mutex<Vec<ResolvedSubsystemTarget>>>,
    }

    struct FailingSupportStateReaderFactory;

    struct FailingSupportStateReader;

    #[derive(Clone, Copy)]
    struct StaticObjectSupportStateReaderFactory(ObjectSupportState);

    struct StaticObjectSupportStateReader(ObjectSupportState);

    impl SupportStateReaderFactory for RecordingSupportStateReaderFactory {
        fn create<'a>(
            &'a self,
            _context: &'a WorkspaceContext,
        ) -> Box<dyn SupportStateReader + 'a> {
            Box::new(RecordingSupportStateReader {
                calls: Arc::clone(&self.calls),
                subsystem_calls: Arc::clone(&self.subsystem_calls),
            })
        }
    }

    impl SupportStateReader for RecordingSupportStateReader {
        fn configuration_support(
            &self,
            target: &ResolvedTarget,
        ) -> Result<ConfigurationSupportData, SupportReadError> {
            self.calls
                .lock()
                .unwrap()
                .push(("configuration", target.clone()));
            Ok(ConfigurationSupportData {
                state: ConfigurationSupportState::Removed,
                editing_enabled: None,
                objects: None,
            })
        }

        fn object_support(
            &self,
            target: &ResolvedTarget,
        ) -> Result<ObjectSupportData, SupportReadError> {
            self.calls.lock().unwrap().push(("object", target.clone()));
            Ok(ObjectSupportData {
                state: ObjectSupportState::Locked,
                direct_edit_safe: Some(false),
            })
        }

        fn subsystem_support(
            &self,
            target: &ResolvedSubsystemTarget,
        ) -> Result<ObjectSupportData, SupportReadError> {
            self.subsystem_calls.lock().unwrap().push(target.clone());
            Ok(ObjectSupportData {
                state: ObjectSupportState::Locked,
                direct_edit_safe: Some(false),
            })
        }
    }

    impl SupportStateReaderFactory for FailingSupportStateReaderFactory {
        fn create<'a>(
            &'a self,
            _context: &'a WorkspaceContext,
        ) -> Box<dyn SupportStateReader + 'a> {
            Box::new(FailingSupportStateReader)
        }
    }

    impl SupportStateReader for FailingSupportStateReader {
        fn configuration_support(
            &self,
            _target: &ResolvedTarget,
        ) -> Result<ConfigurationSupportData, SupportReadError> {
            Err(SupportReadError::new(
                crate::domain::support_state::SupportReadErrorCode::ProviderUnavailable,
                "support-state provider is unavailable",
            ))
        }

        fn object_support(
            &self,
            _target: &ResolvedTarget,
        ) -> Result<ObjectSupportData, SupportReadError> {
            Err(SupportReadError::new(
                crate::domain::support_state::SupportReadErrorCode::ProviderUnavailable,
                "support-state provider is unavailable",
            ))
        }

        fn subsystem_support(
            &self,
            _target: &crate::domain::support_state::ResolvedSubsystemTarget,
        ) -> Result<ObjectSupportData, SupportReadError> {
            Err(SupportReadError::new(
                crate::domain::support_state::SupportReadErrorCode::ProviderUnavailable,
                "support-state provider is unavailable",
            ))
        }
    }

    impl SupportStateReaderFactory for StaticObjectSupportStateReaderFactory {
        fn create<'a>(
            &'a self,
            _context: &'a WorkspaceContext,
        ) -> Box<dyn SupportStateReader + 'a> {
            Box::new(StaticObjectSupportStateReader(self.0))
        }
    }

    impl SupportStateReader for StaticObjectSupportStateReader {
        fn configuration_support(
            &self,
            _target: &ResolvedTarget,
        ) -> Result<ConfigurationSupportData, SupportReadError> {
            unreachable!("meta.info reads object support")
        }

        fn object_support(
            &self,
            _target: &ResolvedTarget,
        ) -> Result<ObjectSupportData, SupportReadError> {
            Ok(ObjectSupportData {
                state: self.0,
                direct_edit_safe: None,
            })
        }

        fn subsystem_support(
            &self,
            _target: &ResolvedSubsystemTarget,
        ) -> Result<ObjectSupportData, SupportReadError> {
            unreachable!("meta.info reads object support")
        }
    }

    fn support_reader_fixture() -> (tempfile::TempDir, WorkspaceContext) {
        let root = tempfile::Builder::new()
            .prefix("unica-support-reader-routes")
            .tempdir()
            .unwrap();
        let workspace = root.path().canonicalize().unwrap();
        let source = workspace.join("src");
        for directory in [
            "Roles/Reader/Ext",
            "Catalogs/Items/Forms/Order/Ext",
            "Reports/Sales/Templates/Sheet/Ext",
            "Reports/Sales/Templates/Dcs/Ext",
        ] {
            std::fs::create_dir_all(source.join(directory)).unwrap();
        }
        std::fs::write(
            workspace.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(
            source.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration uuid="11111111-1111-1111-1111-111111111111"><Properties><Name>Demo</Name></Properties><ChildObjects><Role>Reader</Role><Catalog>Items</Catalog><Report>Sales</Report></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(
            source.join("Roles/Reader.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Role uuid="22222222-2222-2222-2222-222222222222"><Properties><Name>Reader</Name></Properties></Role></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(
            source.join("Roles/Reader/Ext/Rights.xml"),
            r#"<Rights xmlns="http://v8.1c.ru/8.2/roles" setForNewObjects="false" setForAttributesByDefault="true" independentRightsOfChildObjects="false"/>"#,
        )
        .unwrap();
        std::fs::write(
            source.join("Catalogs/Items.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog uuid="33333333-3333-3333-3333-333333333333"><Properties><Name>Items</Name></Properties><ChildObjects><Form>Order</Form></ChildObjects></Catalog></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(
            source.join("Catalogs/Items/Forms/Order.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Form uuid="44444444-4444-4444-4444-444444444444"><Properties><Name>Order</Name></Properties></Form></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(
            source.join("Catalogs/Items/Forms/Order/Ext/Form.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?><Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"><AutoCommandBar name="ФормаКоманднаяПанель" id="-1"/></Form>"#,
        )
        .unwrap();
        std::fs::write(
            source.join("Reports/Sales.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Report uuid="55555555-5555-5555-5555-555555555555"><Properties><Name>Sales</Name></Properties><ChildObjects><Template>Sheet</Template><Template>Dcs</Template></ChildObjects></Report></MetaDataObject>"#,
        )
        .unwrap();
        for (name, uuid, template_type) in [
            (
                "Sheet",
                "66666666-6666-6666-6666-666666666666",
                "SpreadsheetDocument",
            ),
            (
                "Dcs",
                "77777777-7777-7777-7777-777777777777",
                "DataCompositionSchema",
            ),
        ] {
            std::fs::write(
                source.join(format!("Reports/Sales/Templates/{name}.xml")),
                format!(
                    r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Template uuid="{uuid}"><Properties><Name>{name}</Name><TemplateType>{template_type}</TemplateType></Properties></Template></MetaDataObject>"#
                ),
            )
            .unwrap();
        }
        std::fs::write(
            source.join("Reports/Sales/Templates/Sheet/Ext/Template.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?><document xmlns="http://v8.1c.ru/8.2/data/spreadsheet" xmlns:v8="http://v8.1c.ru/8.1/data/core"><columnsID>0</columnsID><format><formatIndex>0</formatIndex></format></document>"#,
        )
        .unwrap();
        std::fs::write(
            source.join("Reports/Sales/Templates/Dcs/Ext/Template.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?><DataCompositionSchema xmlns="http://v8.1c.ru/8.1/data-composition-system/schema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"><dataSource><name>Source</name><dataSourceType>Local</dataSourceType></dataSource></DataCompositionSchema>"#,
        )
        .unwrap();
        let context = WorkspaceContext {
            cwd: workspace.clone(),
            workspace_root: workspace.clone(),
            cache_root: workspace.join(".build/unica"),
            workspace_epoch: 1,
        };
        (root, context)
    }

    fn metadata_address(raw: &str) -> MetadataAddress {
        MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, raw).unwrap()
    }

    #[test]
    fn native_typed_readers_receive_logical_support_targets() {
        use crate::application::ports::ApplicationPorts;

        let (_root, context) = support_reader_fixture();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let ports = super::InfrastructureApplicationPorts::with_support_reader_factory(Arc::new(
            RecordingSupportStateReaderFactory {
                calls: Arc::clone(&calls),
                subsystem_calls: Arc::new(Mutex::new(Vec::new())),
            },
        ));
        let cases = [
            (
                "unica.cf.info",
                Map::from_iter([("ConfigPath".to_string(), json!("src"))]),
                "removed",
            ),
            (
                "unica.role.info",
                Map::from_iter([(
                    "RightsPath".to_string(),
                    json!("src/Roles/Reader/Ext/Rights.xml"),
                )]),
                "locked",
            ),
            (
                "unica.mxl.info",
                Map::from_iter([(
                    "TemplatePath".to_string(),
                    json!("src/Reports/Sales/Templates/Sheet/Ext/Template.xml"),
                )]),
                "locked",
            ),
            (
                "unica.dcs.info",
                Map::from_iter([(
                    "TemplatePath".to_string(),
                    json!("src/Reports/Sales/Templates/Dcs/Ext/Template.xml"),
                )]),
                "locked",
            ),
            (
                "unica.form.info",
                Map::from_iter([(
                    "FormPath".to_string(),
                    json!("src/Catalogs/Items/Forms/Order/Ext/Form.xml"),
                )]),
                "locked",
            ),
        ];
        for (name, args, expected_state) in cases {
            let tool = crate::application::tools()
                .into_iter()
                .find(|tool| tool.name == name)
                .unwrap();
            let outcome = ports
                .invoke_handler(
                    tool,
                    &args,
                    &context,
                    InvocationMode::Read,
                    &CancellationToken::new(),
                )
                .unwrap();
            assert!(outcome.adapter.ok, "{name}: {:?}", outcome.adapter);
            assert_eq!(
                outcome.data.as_ref().unwrap()["support"]["state"],
                expected_state,
                "{name} must publish the value returned by the injected reader"
            );
        }

        assert_eq!(
            *calls.lock().unwrap(),
            vec![
                (
                    "configuration",
                    ResolvedTarget {
                        source_set: "main".to_string(),
                        metadata_path: None,
                        target_kind: TargetKind::SourceRoot,
                    },
                ),
                (
                    "object",
                    ResolvedTarget {
                        source_set: "main".to_string(),
                        metadata_path: Some(metadata_address("Role.Reader")),
                        target_kind: TargetKind::MetadataObject,
                    },
                ),
                (
                    "object",
                    ResolvedTarget {
                        source_set: "main".to_string(),
                        metadata_path: Some(metadata_address("Report.Sales.Template.Sheet")),
                        target_kind: TargetKind::MetadataObject,
                    },
                ),
                (
                    "object",
                    ResolvedTarget {
                        source_set: "main".to_string(),
                        metadata_path: Some(metadata_address("Report.Sales.Template.Dcs")),
                        target_kind: TargetKind::MetadataObject,
                    },
                ),
                (
                    "object",
                    ResolvedTarget {
                        source_set: "main".to_string(),
                        metadata_path: Some(metadata_address("Catalog.Items.Form.Order")),
                        target_kind: TargetKind::MetadataObject,
                    },
                ),
            ]
        );
    }

    fn meta_info_request() -> MetaInfoRequest {
        MetaInfoRequest {
            source_set: "main".to_string(),
            metadata_path: metadata_address("Catalog.Items"),
            sections: Vec::new(),
            limit: 20,
        }
    }

    #[test]
    fn meta_info_passes_its_resolved_target_to_support_reader() {
        use crate::application::ports::ApplicationPorts;

        let (_root, context) = support_reader_fixture();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let ports = super::InfrastructureApplicationPorts::with_support_reader_factory(Arc::new(
            RecordingSupportStateReaderFactory {
                calls: Arc::clone(&calls),
                subsystem_calls: Arc::new(Mutex::new(Vec::new())),
            },
        ));

        let read = ports
            .read_metadata_local(&meta_info_request(), &context, &CancellationToken::new())
            .unwrap();

        assert_eq!(
            read.local.support,
            crate::domain::metadata::MetaSupportStatus::Locked
        );
        assert_eq!(
            *calls.lock().unwrap(),
            vec![(
                "object",
                ResolvedTarget {
                    source_set: "main".to_string(),
                    metadata_path: Some(metadata_address("Catalog.Items")),
                    target_kind: TargetKind::MetadataObject,
                },
            )]
        );
    }

    #[test]
    fn meta_info_maps_support_provider_failure_to_logical_diagnostic() {
        use crate::application::ports::ApplicationPorts;

        let (_root, context) = support_reader_fixture();
        let ports = super::InfrastructureApplicationPorts::with_support_reader_factory(Arc::new(
            FailingSupportStateReaderFactory,
        ));

        let failure = match ports.read_metadata_local(
            &meta_info_request(),
            &context,
            &CancellationToken::new(),
        ) {
            Ok(_) => panic!("support provider failure must fail the local read"),
            Err(failure) => failure,
        };

        assert_eq!(failure.diagnostics.len(), 1);
        let diagnostic = &failure.diagnostics[0];
        assert_eq!(
            diagnostic.code,
            crate::domain::metadata::MetaDiagnosticCode::ProviderUnavailable
        );
        assert_eq!(
            diagnostic.metadata_path.as_ref(),
            Some(&metadata_address("Catalog.Items"))
        );
        assert!(diagnostic.message.contains("provider_unavailable"));
        assert!(!diagnostic
            .message
            .contains(&context.workspace_root.display().to_string()));
        assert!(!diagnostic.message.contains('/'));
        assert!(!diagnostic.message.contains('\\'));
    }

    #[test]
    fn meta_info_preserves_the_existing_support_projection() {
        use crate::application::ports::ApplicationPorts;
        use crate::domain::metadata::MetaSupportStatus;

        let (_root, context) = support_reader_fixture();
        for (state, expected) in [
            (ObjectSupportState::Locked, MetaSupportStatus::Locked),
            (
                ObjectSupportState::ConfigurationReadOnly,
                MetaSupportStatus::Locked,
            ),
            (
                ObjectSupportState::RemovedFromSupport,
                MetaSupportStatus::Unsupported,
            ),
            (
                ObjectSupportState::EditableWithSupport,
                MetaSupportStatus::Supported,
            ),
            (
                ObjectSupportState::NotSupported,
                MetaSupportStatus::Supported,
            ),
        ] {
            let ports = super::InfrastructureApplicationPorts::with_support_reader_factory(
                Arc::new(StaticObjectSupportStateReaderFactory(state)),
            );
            let read = ports
                .read_metadata_local(&meta_info_request(), &context, &CancellationToken::new())
                .unwrap();
            assert_eq!(read.local.support, expected, "state {state:?}");
        }
    }

    #[test]
    fn support_readers_cannot_bypass_the_logical_port() {
        const READERS: &[(&str, &str, &[&str])] = &[
            (
                "cf",
                include_str!("native_operations/cf.rs"),
                &[".configuration_support("],
            ),
            (
                "role",
                include_str!("native_operations/role.rs"),
                &[".object_support("],
            ),
            (
                "mxl",
                include_str!("native_operations/mxl.rs"),
                &[".object_support("],
            ),
            (
                "dcs",
                include_str!("native_operations/dcs.rs"),
                &[".object_support("],
            ),
            (
                "form",
                include_str!("native_operations/form.rs"),
                &[".object_support("],
            ),
            (
                "subsystem",
                include_str!("native_operations/subsystem.rs"),
                &[".object_support(", ".subsystem_support("],
            ),
            (
                "meta",
                include_str!("native_operations/meta/info.rs"),
                &[".object_support("],
            ),
        ];
        const COORDINATOR: &str = include_str!("metadata_operations.rs");
        const FORBIDDEN: &[&str] = &[
            "object_support_state(",
            "support_state_data(",
            "support_status_for_path(",
        ];

        for (name, source, required_calls) in READERS {
            for forbidden in FORBIDDEN {
                assert!(
                    !source.contains(forbidden),
                    "{name} bypasses SupportStateReader through {forbidden}"
                );
            }
            for required_call in *required_calls {
                assert!(
                    source.contains(required_call),
                    "{name} must route support through {required_call}"
                );
            }
        }
        for forbidden in FORBIDDEN {
            assert!(
                !COORDINATOR.contains(forbidden),
                "metadata coordinator bypasses SupportStateReader through {forbidden}"
            );
        }

        let domain_port = include_str!("../domain/support_state.rs");
        assert!(!domain_port.contains("std::path"));
        assert!(!domain_port.contains("&Path"));
        assert!(!domain_port.contains("PathBuf"));
    }

    fn prepared_subsystem_fixture() -> (
        tempfile::TempDir,
        WorkspaceContext,
        Map<String, serde_json::Value>,
        Map<String, serde_json::Value>,
    ) {
        let root = tempfile::Builder::new()
            .prefix("unica-prepared-subsystem-support")
            .tempdir()
            .unwrap();
        let workspace = root.path().canonicalize().unwrap();
        let source = workspace.join("src");
        std::fs::create_dir_all(source.join("Subsystems")).unwrap();
        std::fs::write(
            workspace.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        std::fs::write(
            source.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Demo</Name></Properties><ChildObjects><Subsystem>Sales</Subsystem></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(
            source.join("Subsystems/Sales.xml"),
            crate::infrastructure::native_operations::subsystem::child_subsystem_stub_xml(
                "Sales", "2.20",
            ),
        )
        .unwrap();
        let context = WorkspaceContext {
            cwd: workspace.clone(),
            workspace_root: workspace.clone(),
            cache_root: workspace.join(".build/unica"),
            workspace_epoch: 1,
        };
        (
            root,
            context,
            Map::from_iter([(
                "SubsystemPath".to_string(),
                json!("src/Subsystems/Sales.xml"),
            )]),
            Map::from_iter([("SubsystemPath".to_string(), json!("src/Subsystems"))]),
        )
    }

    fn prepare_subsystem(
        ports: &super::InfrastructureApplicationPorts,
        args: &Map<String, serde_json::Value>,
        context: &WorkspaceContext,
    ) -> crate::application::ports::PreparedToolInvocation {
        use crate::application::ports::ApplicationPorts;

        let tool = crate::application::tools()
            .into_iter()
            .find(|tool| tool.name == "unica.subsystem.info")
            .unwrap();
        ports
            .prepare_tool_invocation(
                tool,
                args,
                context,
                InvocationMode::Read,
                &CancellationToken::new(),
                ProviderDeadline::new(Instant::now() + Duration::from_secs(5)),
            )
            .unwrap()
    }

    #[test]
    fn prepared_subsystem_info_records_the_descriptor_target() {
        let (_root, context, object_args, _tree_args) = prepared_subsystem_fixture();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let subsystem_calls = Arc::new(Mutex::new(Vec::new()));
        let ports = super::InfrastructureApplicationPorts::with_support_reader_factory(Arc::new(
            RecordingSupportStateReaderFactory {
                calls: Arc::clone(&calls),
                subsystem_calls: Arc::clone(&subsystem_calls),
            },
        ));

        let prepared = prepare_subsystem(&ports, &object_args, &context);
        let handler = prepared.handler.expect("prepared handler");

        assert!(handler.adapter.ok, "{:?}", handler.adapter);
        assert_eq!(handler.data.unwrap()["support"]["state"], "locked");
        assert_eq!(
            *calls.lock().unwrap(),
            vec![(
                "object",
                ResolvedTarget {
                    source_set: "main".to_string(),
                    metadata_path: Some(metadata_address("Subsystem.Sales")),
                    target_kind: TargetKind::MetadataObject,
                },
            )]
        );
        assert!(subsystem_calls.lock().unwrap().is_empty());
    }

    #[test]
    fn prepared_nested_subsystem_keeps_the_dedicated_subsystem_address() {
        let (_root, context, _object_args, _tree_args) = prepared_subsystem_fixture();
        let source = context.workspace_root.join("src");
        std::fs::write(
            source.join("Subsystems/Sales.xml"),
            crate::infrastructure::native_operations::subsystem::child_subsystem_stub_xml(
                "Sales", "2.20",
            )
            .replace(
                "<ChildObjects/>",
                "<ChildObjects><Subsystem>Online</Subsystem></ChildObjects>",
            ),
        )
        .unwrap();
        std::fs::create_dir_all(source.join("Subsystems/Sales/Subsystems")).unwrap();
        std::fs::write(
            source.join("Subsystems/Sales/Subsystems/Online.xml"),
            crate::infrastructure::native_operations::subsystem::child_subsystem_stub_xml(
                "Online", "2.20",
            ),
        )
        .unwrap();
        let args = Map::from_iter([(
            "SubsystemPath".to_string(),
            json!("src/Subsystems/Sales/Subsystems/Online.xml"),
        )]);
        let calls = Arc::new(Mutex::new(Vec::new()));
        let subsystem_calls = Arc::new(Mutex::new(Vec::new()));
        let ports = super::InfrastructureApplicationPorts::with_support_reader_factory(Arc::new(
            RecordingSupportStateReaderFactory {
                calls: Arc::clone(&calls),
                subsystem_calls: Arc::clone(&subsystem_calls),
            },
        ));

        let prepared = prepare_subsystem(&ports, &args, &context);
        let handler = prepared.handler.expect("prepared handler");

        assert!(handler.adapter.ok, "{:?}", handler.adapter);
        assert_eq!(handler.data.unwrap()["support"]["state"], "locked");
        assert!(calls.lock().unwrap().is_empty());
        assert_eq!(
            *subsystem_calls.lock().unwrap(),
            vec![ResolvedSubsystemTarget {
                source_set: "main".to_string(),
                address: crate::domain::subsystem::SubsystemAddress::parse("Sales.Online").unwrap(),
            }]
        );
    }

    #[test]
    fn subsystem_tree_does_not_invent_object_support() {
        let (_root, context, _object_args, tree_args) = prepared_subsystem_fixture();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let subsystem_calls = Arc::new(Mutex::new(Vec::new()));
        let ports = super::InfrastructureApplicationPorts::with_support_reader_factory(Arc::new(
            RecordingSupportStateReaderFactory {
                calls: Arc::clone(&calls),
                subsystem_calls: Arc::clone(&subsystem_calls),
            },
        ));

        let prepared = prepare_subsystem(&ports, &tree_args, &context);

        assert!(prepared.handler.unwrap().adapter.ok);
        assert!(calls.lock().unwrap().is_empty());
        assert!(subsystem_calls.lock().unwrap().is_empty());
    }

    #[test]
    fn subsystem_support_failure_publishes_no_partial_data() {
        let (_root, context, object_args, _tree_args) = prepared_subsystem_fixture();
        let ports = super::InfrastructureApplicationPorts::with_support_reader_factory(Arc::new(
            FailingSupportStateReaderFactory,
        ));

        let prepared = prepare_subsystem(&ports, &object_args, &context);
        let handler = prepared.handler.expect("prepared failure handler");

        assert!(!handler.adapter.ok, "{:?}", handler.adapter);
        assert_eq!(
            handler.adapter.errors,
            vec!["provider_unavailable: support-state provider is unavailable"]
        );
        assert!(handler.data.is_none());
    }

    #[test]
    fn infrastructure_rejects_non_read_modes_for_a_reader() {
        use crate::application::ports::ApplicationPorts;

        let root = tempfile::tempdir().unwrap();
        let context = WorkspaceContext {
            cwd: root.path().to_path_buf(),
            workspace_root: root.path().to_path_buf(),
            cache_root: root.path().join(".build/unica"),
            workspace_epoch: 1,
        };
        for mode in [InvocationMode::Preview, InvocationMode::Apply] {
            let error = match super::InfrastructureApplicationPorts::new().invoke_handler(
                spec("unica.project.status", ToolHandler::ProjectStatus),
                &Map::new(),
                &context,
                mode,
                &CancellationToken::new(),
            ) {
                Ok(_) => panic!("reader unexpectedly accepted {mode:?}"),
                Err(error) => error,
            };
            assert_eq!(error, "invalid invocation mode for unica.project.status");
        }
    }

    #[test]
    fn infrastructure_rejects_read_mode_for_a_mutation() {
        use crate::application::ports::ApplicationPorts;

        let root = tempfile::tempdir().unwrap();
        let context = WorkspaceContext {
            cwd: root.path().to_path_buf(),
            workspace_root: root.path().to_path_buf(),
            cache_root: root.path().join(".build/unica"),
            workspace_epoch: 1,
        };
        let error = match super::InfrastructureApplicationPorts::new().invoke_handler(
            spec("unica.cf.edit", ToolHandler::ProjectStatus),
            &Map::new(),
            &context,
            InvocationMode::Read,
            &CancellationToken::new(),
        ) {
            Ok(_) => panic!("mutation unexpectedly accepted Read"),
            Err(error) => error,
        };
        assert_eq!(error, "invalid invocation mode for unica.cf.edit");
    }

    fn subsystem_info_fixture(
        label: &str,
    ) -> (
        tempfile::TempDir,
        WorkspaceContext,
        Map<String, serde_json::Value>,
    ) {
        let root = tempfile::Builder::new().prefix(label).tempdir().unwrap();
        let physical_root = root.path().canonicalize().unwrap();
        std::fs::create_dir_all(physical_root.join("Subsystems")).unwrap();
        std::fs::write(
            physical_root.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Test</Name></Properties><ChildObjects><Subsystem>Sales</Subsystem></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        std::fs::write(
            physical_root.join("Subsystems/Sales.xml"),
            crate::infrastructure::native_operations::subsystem::child_subsystem_stub_xml(
                "Sales", "2.20",
            ),
        )
        .unwrap();
        let context = WorkspaceContext {
            cwd: physical_root.clone(),
            workspace_root: physical_root.clone(),
            cache_root: physical_root.join(".build/unica"),
            workspace_epoch: 1,
        };
        let args = Map::from_iter([(
            "SubsystemPath".to_string(),
            serde_json::Value::String("Subsystems".to_string()),
        )]);
        (root, context, args)
    }

    #[test]
    fn subsystem_info_native_path_observes_mid_read_cancellation() {
        let (_root, context, args) = subsystem_info_fixture("unica-subsystem-mid-read");
        let cancellation = CancellationToken::new();
        let hook_cancellation = cancellation.clone();
        let outcome = with_secure_tree_test_hook(
            move |phase| {
                if phase == &SecureTreePhase::AfterRebindEntry(PathBuf::from("Configuration.xml")) {
                    hook_cancellation.cancel();
                }
            },
            || {
                NativeOperationAdapter::invoke_with_data(
                    "subsystem-info",
                    "unica.subsystem.info",
                    &args,
                    &context,
                    false,
                    false,
                    NativeInvocationContext::new(
                        &crate::infrastructure::support_state::WorkspaceSupportStateReader::new(
                            &context,
                        ),
                        &cancellation,
                        ProviderDeadline::new(Instant::now() + Duration::from_secs(5)),
                    ),
                )
            },
        )
        .unwrap();

        assert!(!outcome.adapter.ok, "{:?}", outcome.adapter);
        assert!(
            outcome
                .adapter
                .errors
                .iter()
                .any(|error| error.starts_with("cancelled:")),
            "{:?}",
            outcome.adapter
        );
        assert!(outcome.data.is_none());
    }

    #[test]
    fn subsystem_info_native_path_observes_terminal_cancellation_after_identity_proofs() {
        let (_root, context, mut args) = subsystem_info_fixture("unica-subsystem-terminal-cancel");
        args.insert(
            "SubsystemPath".to_string(),
            serde_json::Value::String("Subsystems/Sales.xml".to_string()),
        );
        let cancellation = CancellationToken::new();
        let hook_cancellation = cancellation.clone();
        let outcome = with_secure_tree_test_hook(
            move |phase| {
                if phase == &SecureTreePhase::AfterFinalIdentityProofs {
                    hook_cancellation.cancel();
                }
            },
            || {
                NativeOperationAdapter::invoke_with_data(
                    "subsystem-info",
                    "unica.subsystem.info",
                    &args,
                    &context,
                    false,
                    false,
                    NativeInvocationContext::new(
                        &crate::infrastructure::support_state::WorkspaceSupportStateReader::new(
                            &context,
                        ),
                        &cancellation,
                        ProviderDeadline::new(Instant::now() + Duration::from_secs(5)),
                    ),
                )
            },
        )
        .unwrap();

        assert!(!outcome.adapter.ok, "{:?}", outcome.adapter);
        assert!(
            outcome
                .adapter
                .errors
                .iter()
                .any(|error| error.starts_with("cancelled:")),
            "{:?}",
            outcome.adapter
        );
        assert!(outcome.data.is_none());
    }

    #[test]
    fn subsystem_info_typed_invocation_rejects_an_exhausted_deadline() {
        let (_root, context, args) = subsystem_info_fixture("unica-subsystem-expired-deadline");
        let cancellation = CancellationToken::new();

        let outcome = super::NativeOperationAdapter::invoke_with_data(
            "subsystem-info",
            "unica.subsystem.info",
            &args,
            &context,
            false,
            false,
            NativeInvocationContext::new(
                &crate::infrastructure::support_state::WorkspaceSupportStateReader::new(&context),
                &cancellation,
                ProviderDeadline::new(Instant::now() - Duration::from_millis(1)),
            ),
        )
        .unwrap();

        assert!(!outcome.adapter.ok, "{:?}", outcome.adapter);
        assert!(
            outcome
                .adapter
                .errors
                .iter()
                .any(|error| error.contains("provider deadline exceeded")),
            "{:?}",
            outcome.adapter
        );
        assert!(outcome.data.is_none());
    }

    #[test]
    fn standalone_subsystem_info_rejects_an_already_cancelled_invocation_before_read() {
        let (_root, context, mut args) = subsystem_info_fixture("unica-standalone-cancelled");
        std::fs::remove_file(context.cwd.join("Configuration.xml")).unwrap();
        args.insert(
            "SubsystemPath".to_string(),
            serde_json::Value::String("Subsystems/Sales.xml".to_string()),
        );
        let cancellation = CancellationToken::new();
        cancellation.cancel();

        let outcome = super::NativeOperationAdapter::invoke_with_data(
            "subsystem-info",
            "unica.subsystem.info",
            &args,
            &context,
            false,
            false,
            NativeInvocationContext::new(
                &crate::infrastructure::support_state::WorkspaceSupportStateReader::new(&context),
                &cancellation,
                ProviderDeadline::new(Instant::now() + Duration::from_secs(5)),
            ),
        )
        .unwrap();

        assert!(!outcome.adapter.ok, "{:?}", outcome.adapter);
        assert!(
            outcome
                .adapter
                .errors
                .iter()
                .any(|error| error.starts_with("cancelled:")),
            "{:?}",
            outcome.adapter
        );
        assert!(outcome.data.is_none());
    }

    #[test]
    fn standalone_subsystem_info_rejects_an_exhausted_deadline_before_read() {
        let (_root, context, mut args) = subsystem_info_fixture("unica-standalone-deadline");
        std::fs::remove_file(context.cwd.join("Configuration.xml")).unwrap();
        args.insert(
            "SubsystemPath".to_string(),
            serde_json::Value::String("Subsystems/Sales.xml".to_string()),
        );
        let cancellation = CancellationToken::new();

        let outcome = super::NativeOperationAdapter::invoke_with_data(
            "subsystem-info",
            "unica.subsystem.info",
            &args,
            &context,
            false,
            false,
            NativeInvocationContext::new(
                &crate::infrastructure::support_state::WorkspaceSupportStateReader::new(&context),
                &cancellation,
                ProviderDeadline::new(Instant::now() - Duration::from_millis(1)),
            ),
        )
        .unwrap();

        assert!(!outcome.adapter.ok, "{:?}", outcome.adapter);
        assert!(
            outcome
                .adapter
                .errors
                .iter()
                .any(|error| error.contains("provider deadline exceeded")),
            "{:?}",
            outcome.adapter
        );
        assert!(outcome.data.is_none());
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
    fn documentation_registry_wires_providers_in_the_declared_order() {
        // The composition root is the only place documentation providers are
        // constructed; a registry left short (forgot to wire one) or ordered
        // differently must fail this, not merely compile. Порядок реестра —
        // порядок секций публичного ответа, специфичность убывает: справка
        // самой конфигурации раньше справки платформы, локальные поставщики
        // раньше сетевых, справка раньше стандартов.
        // Замок общий со стенд-тестами: без него параллельный прогон подменил
        // бы реестр на стенд прямо под этой проверкой.
        let _serial = documentation_registry_serial();
        let dir = tempfile::tempdir().expect("каталог");
        let workspace = dir.path().to_path_buf();
        let context = WorkspaceContext {
            cwd: workspace.clone(),
            workspace_root: workspace.clone(),
            cache_root: workspace.join(".build/unica"),
            workspace_epoch: 1,
        };
        let registry = documentation_registry(
            &context,
            &crate::domain::cancellation::CancellationToken::default(),
        )
        .expect("registry constructs");
        let ids: Vec<String> = registry
            .providers()
            .map(|provider| provider.id().to_string())
            .collect();
        assert_eq!(
            ids,
            vec![
                "configuration-help".to_string(),
                "platform-syntax-help".to_string(),
                "kb-1ci".to_string(),
                "v8std".to_string()
            ],
            "состав и порядок реестра"
        );
        let providers: Vec<_> = registry.providers().collect();
        assert_eq!(providers[0].corpora().len(), 1, "configuration-help");
        assert_eq!(
            providers[0].corpora()[0].source_kind,
            crate::domain::documentation::SourceKind::ConfigurationDocumentation
        );
        assert_eq!(
            providers[1].corpora().len(),
            2,
            "syntax-context and platform-guides"
        );
        assert_eq!(
            providers[2].corpora().len(),
            2,
            "kb-developer-guide and kb-administrator-guide"
        );
        assert_eq!(providers[3].corpora().len(), 1, "public-standards");
        assert_eq!(
            providers[3].corpora()[0].source_kind,
            crate::domain::documentation::SourceKind::DevelopmentStandard
        );
    }

    /// Источник поставщика справки конфигурации — source-set'ы рабочего
    /// пространства: выгрузка с `Configuration.xml` в корне обязана дать
    /// секцию с попаданием без какой-либо настройки проекта.
    #[test]
    fn documentation_registry_feeds_configuration_help_with_workspace_sources() {
        let _serial = documentation_registry_serial();
        let dir = tempfile::tempdir().expect("каталог");
        let workspace = dir.path().to_path_buf();
        std::fs::write(
            workspace.join("Configuration.xml"),
            "<?xml version=\"1.0\"?><MetaDataObject><Configuration><Properties>\
             <Version>1.0.0.1</Version></Properties></Configuration></MetaDataObject>",
        )
        .expect("configuration");
        let help = workspace.join("Catalogs/Товары/Ext/Help");
        std::fs::create_dir_all(&help).expect("help dir");
        std::fs::write(
            help.join("ru.html"),
            "<html><body><h1>Товары</h1><p>Справочник товаров.</p></body></html>",
        )
        .expect("help page");
        let context = WorkspaceContext {
            cwd: workspace.clone(),
            workspace_root: workspace.clone(),
            cache_root: workspace.join(".build/unica"),
            workspace_epoch: 1,
        };
        let registry = documentation_registry(
            &context,
            &crate::domain::cancellation::CancellationToken::default(),
        )
        .expect("registry constructs");
        let provider = registry.providers().next().expect("первый поставщик");
        let sections = provider.search(
            &crate::domain::documentation::DocumentationSearchRequest {
                query: "Товары".to_string(),
                source_kinds: Vec::new(),
                limit: 5,
                language: "ru".to_string(),
            },
            &crate::domain::documentation::DocumentationContext {
                platform_version: None,
                installation_root: None,
            },
        );
        assert_eq!(
            sections[0].hits[0].document_id,
            "configuration-help:main:Catalogs/Товары/Ext/Help/ru.html",
            "source-set найден автодетектом и назван в локаторе"
        );
        assert_eq!(sections[0].hits[0].applicable_version, "1.0.0.1");
    }

    /// Фасады `unica.standards.*` делят с поставщиком движок и политику
    /// (ADR-0032 п.4, следствие 2: «оба выключаются одним файлом настройки»).
    /// Запрет в `unica.toml` обязан отказывать фасаду ДО транспорта, называя
    /// политику; endpoint на закрытый локальный порт делает и красное
    /// состояние герметичным — сеть за пределы машины не выходит.
    /// Различающая сила доказана мутацией: со снятой проверкой запрета
    /// (`if false`) тест падает.
    #[test]
    fn the_standards_facades_refuse_when_policy_denies_v8std() {
        use crate::application::ports::ApplicationPorts;

        struct EnvGuard {
            previous: Option<String>,
        }
        impl Drop for EnvGuard {
            fn drop(&mut self) {
                match self.previous.take() {
                    Some(value) => std::env::set_var("UNICA_STANDARDS_MCP_URL", value),
                    None => std::env::remove_var("UNICA_STANDARDS_MCP_URL"),
                }
            }
        }
        let _env = EnvGuard {
            previous: std::env::var("UNICA_STANDARDS_MCP_URL").ok(),
        };
        std::env::set_var("UNICA_STANDARDS_MCP_URL", "http://127.0.0.1:9/mcp");

        let dir = tempfile::tempdir().expect("каталог");
        let workspace = dir.path().to_path_buf();
        std::fs::write(
            workspace.join("unica.toml"),
            "[providers.v8std]\nnetwork = \"deny\"\n",
        )
        .expect("политика");
        let context = WorkspaceContext {
            cwd: workspace.clone(),
            workspace_root: workspace.clone(),
            cache_root: workspace.join(".build/unica"),
            workspace_epoch: 1,
        };

        let mut args = Map::new();
        args.insert("query".to_string(), json!("ссылка"));
        let error = match super::InfrastructureApplicationPorts::new().invoke_handler(
            spec(
                "unica.standards.search",
                ToolHandler::StandardsAdapter {
                    operation: "search",
                },
            ),
            &args,
            &context,
            InvocationMode::Read,
            &crate::domain::cancellation::CancellationToken::default(),
        ) {
            Ok(_) => panic!("запрет политики обязан отказывать фасаду"),
            Err(error) => error,
        };
        assert!(
            error.contains("unica.toml"),
            "отказ обязан назвать политику, получено {error}"
        );
    }

    /// Описание публичного инструмента обещало «development standards», хотя
    /// ни один поставщик реестра не объявляет корпуса этого смысла источника:
    /// секция `development-standard` в ответе недостижима до ADR-0032, и
    /// агент, ищущий стандарт, получил бы `empty` вместо маршрута в
    /// `unica.standards.search`. Проверка структурная: описание вправе
    /// назвать стандарты в тот момент, когда реестр действительно несёт
    /// такой корпус, — тогда это утверждение перестаёт быть переобещанием и
    /// тест перестаёт его запрещать.
    #[test]
    fn the_documentation_tool_description_promises_only_declared_source_kinds() {
        let description = crate::application::tools()
            .into_iter()
            .find(|tool| tool.name == "unica.documentation.search")
            .expect("инструмент объявлен")
            .description;
        // Замок общий со стенд-тестами: настоящий реестр, а не стенд соседа.
        let _serial = documentation_registry_serial();
        let dir = tempfile::tempdir().expect("каталог");
        let workspace = dir.path().to_path_buf();
        let context = WorkspaceContext {
            cwd: workspace.clone(),
            workspace_root: workspace.clone(),
            cache_root: workspace.join(".build/unica"),
            workspace_epoch: 1,
        };
        let registry = documentation_registry(
            &context,
            &crate::domain::cancellation::CancellationToken::default(),
        )
        .expect("реестр собран");
        let declares_standards = registry.providers().any(|provider| {
            provider.corpora().iter().any(|corpus| {
                corpus.source_kind == crate::domain::documentation::SourceKind::DevelopmentStandard
            })
        });
        if !declares_standards {
            assert!(
                !description.to_lowercase().contains("standard"),
                "описание обещает стандарты, которых нет ни у одного поставщика: {description}"
            );
        }
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

    /// Прочитать конфигурацию проекта умеет
    /// `project_platform_version_reads_the_config_and_prefers_the_local_overlay`,
    /// выбрать установку по закреплённой семье — `select_installation_root_*`.
    /// Между ними была дыра: ничто не проверяло, что диспетчер
    /// `unica.documentation.search` СОЕДИНЯЕТ одно с другим. Удаление вызова
    /// ловит компилятор, а подстановка `None` на его место — нет: ревью
    /// применило именно её, и все 2021 тест остались зелёными. Вред — п.3
    /// ADR-0029: проект, закреплённый за 8.3.27, читает справку 8.5.4, а ответ
    /// продолжает называть 8.3.27.
    #[test]
    fn the_dispatcher_constrains_the_installation_by_the_projects_own_platform_pin() {
        let machine = tempfile::tempdir().expect("каталог установок");
        for version in ["8.3.27.2074", "8.5.4.1306"] {
            std::fs::create_dir_all(machine.path().join(version)).expect("каталог версии");
        }
        let roots = vec![machine.path().to_path_buf()];

        let project = tempfile::tempdir().expect("каталог проекта");
        let workspace = project.path().to_path_buf();
        let context = WorkspaceContext {
            cwd: workspace.clone(),
            workspace_root: workspace.clone(),
            cache_root: workspace.join(".build/unica"),
            workspace_epoch: 1,
        };

        // Без закрепления побеждает численно старшая — эта половина и
        // остаётся верной при мутации, поэтому одной её недостаточно.
        let unpinned = documentation_context(&roots, None, &context);
        assert_eq!(
            unpinned.installation_root,
            Some(machine.path().join("8.5.4.1306")),
            "без закрепления читается численно старшая установка"
        );
        assert_eq!(unpinned.platform_version, None);

        std::fs::write(
            workspace.join("v8project.yaml"),
            "tools:\n  platform:\n    version: \"8.3.27\"\n",
        )
        .expect("конфигурация проекта");

        let pinned = documentation_context(&roots, None, &context);
        assert_eq!(
            pinned.installation_root,
            Some(machine.path().join("8.3.27.2074")),
            "закрепление проекта обязано сужать выбор установки, а не только попадать в ответ"
        );
        assert_eq!(
            pinned.platform_version.as_deref(),
            Some("8.3.27"),
            "ограничение, по которому искали установку, обязано попасть в ответ"
        );

        // Явный аргумент вызова сильнее закрепления проекта (п.2 ADR-0029).
        let requested = documentation_context(&roots, Some("8.5.4.1306"), &context);
        assert_eq!(
            requested.installation_root,
            Some(machine.path().join("8.5.4.1306"))
        );
        assert_eq!(requested.platform_version.as_deref(), Some("8.5.4.1306"));
    }

    /// Записывающий поставщик: единственный способ увидеть, ЧТО именно ветка
    /// диспетчера передала слою application. Возвращает `Empty`, чтобы вызов
    /// завершался успехом и проверялся заодно и его результат.
    #[derive(Default)]
    struct RecordingProvider {
        seen: std::sync::Mutex<
            Vec<(
                crate::domain::documentation::DocumentationSearchRequest,
                crate::domain::documentation::DocumentationContext,
            )>,
        >,
        seen_gets: std::sync::Mutex<
            Vec<(
                String,
                String,
                crate::domain::documentation::DocumentationContext,
            )>,
        >,
    }

    impl crate::domain::documentation::DocumentationProvider for RecordingProvider {
        fn id(&self) -> crate::domain::documentation::DocumentationProviderId {
            crate::domain::documentation::DocumentationProviderId::new("recording")
        }
        fn corpora(&self) -> Vec<crate::domain::documentation::DocumentationCorpus> {
            // Смысл корпуса объявлен, чтобы фильтр sourceKinds считал стенд
            // применимым и его опрос был наблюдаем.
            vec![crate::domain::documentation::DocumentationCorpus {
                id: "syntax-context".to_string(),
                source_kind: crate::domain::documentation::SourceKind::PlatformHelp,
                authority: crate::domain::documentation::Authority::Vendor,
            }]
        }
        fn needs_network(&self) -> bool {
            false
        }
        fn search(
            &self,
            request: &crate::domain::documentation::DocumentationSearchRequest,
            context: &crate::domain::documentation::DocumentationContext,
        ) -> Vec<crate::domain::documentation::DocumentationSection> {
            self.seen
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push((request.clone(), context.clone()));
            vec![crate::domain::documentation::DocumentationSection::empty(
                self.id(),
                "syntax-context",
                crate::domain::documentation::SourceKind::PlatformHelp,
                crate::domain::documentation::Authority::Vendor,
                &request.language,
            )]
        }

        fn get(
            &self,
            document_id: &str,
            language: &str,
            context: &crate::domain::documentation::DocumentationContext,
        ) -> Option<Result<crate::domain::documentation::DocumentationDocument, String>> {
            self.seen_gets
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push((
                    document_id.to_string(),
                    language.to_string(),
                    context.clone(),
                ));
            Some(Ok(crate::domain::documentation::DocumentationDocument {
                provider: self.id(),
                corpus: "syntax-context".to_string(),
                source_kind: crate::domain::documentation::SourceKind::PlatformHelp,
                authority: crate::domain::documentation::Authority::Vendor,
                language: language.to_string(),
                document_id: document_id.to_string(),
                title: "Заголовок".to_string(),
                signature: None,
                applicable_version: "8.3.27.2074".to_string(),
                text: "Полный текст.".to_string(),
            }))
        }
    }

    /// Слот подмены один на процесс, поэтому тесты, которые его пишут, идут по
    /// одному: страж несёт замок сериализации и держит его до конца теста, а
    /// подмена снимается на выходе даже при панике теста. Без замка два
    /// стенд-теста под параллельным прогоном перезаписывали бы слот друг
    /// друга — тот же приём, что и `index_test_lock` у поставщика.
    struct StandInGuard {
        _serial: std::sync::MutexGuard<'static, ()>,
    }

    impl Drop for StandInGuard {
        fn drop(&mut self) {
            *super::DOCUMENTATION_REGISTRY_STAND_IN
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        }
    }

    /// Один замок на писателей слота подмены И на читателей настоящего
    /// реестра: `documentation_registry()` под параллельным прогоном иначе
    /// видел бы стенд соседнего теста вместо настоящего поставщика.
    fn documentation_registry_serial() -> std::sync::MutexGuard<'static, ()> {
        static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
        SERIAL
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn install_stand_in(provider: std::sync::Arc<RecordingProvider>) -> StandInGuard {
        let serial = documentation_registry_serial();
        *super::DOCUMENTATION_REGISTRY_STAND_IN
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(
            provider as std::sync::Arc<dyn crate::domain::documentation::DocumentationProvider>,
        );
        StandInGuard { _serial: serial }
    }

    /// `tools.platform.path` — вторая половина закрепления платформы у
    /// раннера, и до правки она не читалась вовсе: проект, закрепивший один
    /// путь (пример в `references/tooling/runtime-build.md` именно таков —
    /// `path` в `v8project.local.yaml`), выгоды не получал, а справка
    /// продолжала идти из численно старшей установки стандартных корней. Пин
    /// пути называет установку напрямую и заменяет перебор корней, поэтому
    /// проверяется на ПУСТОМ списке корней: разрешение не вправе зависеть от
    /// них. Путь из документации указывает на `<версия>/bin` — он обязан
    /// сводиться к каталогу версии, потому что имя версии несёт именно он.
    #[test]
    fn the_projects_platform_path_pin_names_the_installation_directly() {
        let machine = tempfile::tempdir().expect("каталог установок");
        let install = machine.path().join("8.3.27.2074");
        std::fs::create_dir_all(install.join("bin")).expect("каталог версии с bin");

        let project = tempfile::tempdir().expect("каталог проекта");
        let workspace = project.path().to_path_buf();
        let context = WorkspaceContext {
            cwd: workspace.clone(),
            workspace_root: workspace.clone(),
            cache_root: workspace.join(".build/unica"),
            workspace_epoch: 1,
        };

        // Одинарные кавычки YAML: путь Windows несёт обратные слэши, а в
        // двойных кавычках `\U` и `\8` — невалидные escape-последовательности,
        // и конфигурация молча не разбиралась бы именно на той ОС, ради
        // которой пример в references и хранит путь в локальном файле.
        std::fs::write(
            workspace.join("v8project.local.yaml"),
            format!("tools:\n  platform:\n    path: '{}'\n", install.display()),
        )
        .expect("локальное закрепление пути");

        let pinned = documentation_context(&[], None, &context);
        assert_eq!(
            pinned.installation_root,
            Some(install.clone()),
            "пин пути обязан называть установку без перебора корней"
        );

        std::fs::write(
            workspace.join("v8project.local.yaml"),
            format!(
                "tools:\n  platform:\n    path: '{}'\n",
                install.join("bin").display()
            ),
        )
        .expect("закрепление пути на bin");
        let via_bin = documentation_context(&[], None, &context);
        assert_eq!(
            via_bin.installation_root,
            Some(install.clone()),
            "путь на bin обязан сводиться к каталогу версии — имя версии несёт он"
        );
    }

    /// Ограничения версий действуют и при пине пути: явный аргумент вызова
    /// обязан совпасть с именем закреплённого каталога точно, а семейство
    /// `tools.platform.version` — быть его префиксом. Несовпадение — отказ,
    /// а не тихий переход к стандартным корням: пин заменяет перебор, как и
    /// у раннера, и подстановка соседней установки здесь была бы тем же
    /// вредом п.3 ADR-0029.
    #[test]
    fn version_constraints_still_apply_to_a_path_pinned_installation() {
        let machine = tempfile::tempdir().expect("каталог установок");
        let install = machine.path().join("8.3.27.2074");
        std::fs::create_dir_all(&install).expect("каталог версии");
        // Соседняя установка в стандартном корне: тихий переход к перебору
        // корней подставил бы её и остался бы незамеченным без этой приманки.
        let decoy_root = machine.path().join("standard");
        std::fs::create_dir_all(decoy_root.join("8.5.4.1306")).expect("каталог приманки");

        let project = tempfile::tempdir().expect("каталог проекта");
        let workspace = project.path().to_path_buf();
        let context = WorkspaceContext {
            cwd: workspace.clone(),
            workspace_root: workspace.clone(),
            cache_root: workspace.join(".build/unica"),
            workspace_epoch: 1,
        };
        // Одинарные кавычки: см. соседний тест — двойные ломают Windows-пути.
        std::fs::write(
            workspace.join("v8project.yaml"),
            format!("tools:\n  platform:\n    path: '{}'\n", install.display()),
        )
        .expect("закрепление пути");

        let roots = [decoy_root];
        assert_eq!(
            documentation_context(&roots, Some("8.3.27.2074"), &context).installation_root,
            Some(install.clone()),
            "совпавший аргумент вызова принимает закреплённую установку"
        );
        assert_eq!(
            documentation_context(&roots, Some("8.5.4.1306"), &context).installation_root,
            None,
            "аргумент вызова, не совпавший с пином, — отказ, а не перебор корней"
        );

        std::fs::write(
            workspace.join("v8project.local.yaml"),
            "tools:\n  platform:\n    version: \"8.4\"\n",
        )
        .expect("несовместимое семейство");
        assert_eq!(
            documentation_context(&roots, None, &context).installation_root,
            None,
            "семейство, которому пин не принадлежит, — отказ, а не перебор корней"
        );
    }

    /// `the_dispatcher_constrains_the_installation_by_the_projects_own_platform_pin`
    /// проверяет ПОМОЩНИКА, а не ветку, и три мутации самой ветки оставляли
    /// всё дерево зелёным: игнорировать аргумент `language`, игнорировать
    /// `platformVersion` и обойти `documentation_context` пустым контекстом.
    /// Вторая кусает пользователя молча наполовину: ответ приходит из другой
    /// установки, чем спросили. Здесь прогоняется сама ветка
    /// `ToolHandler::Documentation`, а поставщик записывает то, что до него
    /// дошло.
    ///
    /// Версия намеренно невозможная: она не разрешается ни в какую установку
    /// ни на одной машине, поэтому проверка не зависит от того, какие
    /// платформы стоят на машине сборки.
    #[test]
    fn the_documentation_branch_carries_its_arguments_into_the_request_and_the_context() {
        use crate::application::ports::ApplicationPorts;

        let recorder = std::sync::Arc::new(RecordingProvider::default());
        let _stand_in = install_stand_in(std::sync::Arc::clone(&recorder));

        let dir = tempfile::tempdir().expect("каталог");
        let workspace = dir.path().to_path_buf();
        let context = WorkspaceContext {
            cwd: workspace.clone(),
            workspace_root: workspace.clone(),
            cache_root: workspace.join(".build/unica"),
            workspace_epoch: 1,
        };

        let mut args = Map::new();
        args.insert("query".to_string(), json!("GetURL"));
        args.insert("language".to_string(), json!("en"));
        args.insert("platformVersion".to_string(), json!("9.9.9.9999"));
        args.insert("limit".to_string(), json!(7));

        let outcome = super::InfrastructureApplicationPorts::new()
            .invoke_handler(
                spec(
                    "unica.documentation.search",
                    ToolHandler::Documentation {
                        operation: "search",
                    },
                ),
                &args,
                &context,
                InvocationMode::Read,
                &crate::domain::cancellation::CancellationToken::default(),
            )
            .expect("ветка обязана ответить");

        let seen = recorder
            .seen
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert_eq!(seen.len(), 1, "поставщик обязан быть опрошен ровно раз");
        let (request, documentation_context) = &seen[0];
        assert_eq!(request.query, "GetURL");
        assert_eq!(
            request.language, "en",
            "аргумент language обязан дойти до запроса, а не подменяться константой"
        );
        assert_eq!(request.limit, 7, "аргумент limit обязан дойти до запроса");
        assert_eq!(
            documentation_context.platform_version.as_deref(),
            Some("9.9.9.9999"),
            "аргумент platformVersion обязан дойти до контекста: иначе ответ придёт из другой установки, чем спросили"
        );
        assert_eq!(
            documentation_context.installation_root, None,
            "несуществующая версия не разрешается ни в какую установку"
        );

        let data = outcome
            .data
            .expect("ветка обязана отвечать типизированным data");
        assert_eq!(
            data["sections"][0]["language"], "en",
            "локаль ответа обязана дойти обратно до публичного результата"
        );
    }

    /// Reader не имеет invocation switch: штатный вызов обязан дойти до
    /// provider registry и вернуть его типизированные секции.
    #[test]
    fn documentation_reader_polls_provider_without_an_invocation_switch() {
        use crate::application::ports::ApplicationPorts;

        let recorder = std::sync::Arc::new(RecordingProvider::default());
        let _stand_in = install_stand_in(std::sync::Arc::clone(&recorder));

        let dir = tempfile::tempdir().expect("каталог");
        let workspace = dir.path().to_path_buf();
        let context = WorkspaceContext {
            cwd: workspace.clone(),
            workspace_root: workspace.clone(),
            cache_root: workspace.join(".build/unica"),
            workspace_epoch: 1,
        };

        let mut args = Map::new();
        args.insert("query".to_string(), json!("СтрНайти"));

        let outcome = super::InfrastructureApplicationPorts::new()
            .invoke_handler(
                spec(
                    "unica.documentation.search",
                    ToolHandler::Documentation {
                        operation: "search",
                    },
                ),
                &args,
                &context,
                InvocationMode::Read,
                &crate::domain::cancellation::CancellationToken::default(),
            )
            .expect("reader обязан ответить успехом");

        let seen = recorder
            .seen
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert_eq!(seen.len(), 1, "reader обязан опросить provider ровно раз");
        assert!(outcome.adapter.ok, "reader обязан вернуть успешный ответ");
        assert!(
            !outcome.adapter.summary.contains("dry run"),
            "reader не должен называться сухим прогоном: {}",
            outcome.adapter.summary
        );
        assert!(
            outcome.data.is_some(),
            "успешный documentation reader обязан вернуть data"
        );
    }

    /// Обычный reader без обязательного `query` отказывается до provider-а.
    #[test]
    fn documentation_reader_requires_the_query_argument() {
        use crate::application::ports::ApplicationPorts;

        let recorder = std::sync::Arc::new(RecordingProvider::default());
        let _stand_in = install_stand_in(std::sync::Arc::clone(&recorder));

        let dir = tempfile::tempdir().expect("каталог");
        let workspace = dir.path().to_path_buf();
        let context = WorkspaceContext {
            cwd: workspace.clone(),
            workspace_root: workspace.clone(),
            cache_root: workspace.join(".build/unica"),
            workspace_epoch: 1,
        };

        // `HandlerOutcome` не несёт `Debug`, поэтому `expect_err` не
        // компилируется — тот же приём, что и в
        // `domain::documentation::tests::duplicate_provider_ids_are_rejected`.
        let error = match super::InfrastructureApplicationPorts::new().invoke_handler(
            spec(
                "unica.documentation.search",
                ToolHandler::Documentation {
                    operation: "search",
                },
            ),
            &Map::new(),
            &context,
            InvocationMode::Read,
            &crate::domain::cancellation::CancellationToken::default(),
        ) {
            Ok(_) => panic!("reader без query обязан отказывать"),
            Err(error) => error,
        };
        assert!(
            error.contains("query"),
            "отказ обязан назвать недостающий аргумент, получено {error}"
        );
    }

    /// Аргумент `sourceKinds` (ADR-0032 п.5) фильтрует по смыслу источника, а
    /// не по идентификатору поставщика. Разобранные значения обязаны дойти до
    /// запроса, а чужое значение — отказ с перечнем допустимых: молча
    /// проигнорированный фильтр отвечал бы стандартами на просьбу «только
    /// справка платформы».
    #[test]
    fn the_documentation_branch_parses_source_kinds_and_refuses_unknown_values() {
        use crate::application::ports::ApplicationPorts;

        let recorder = std::sync::Arc::new(RecordingProvider::default());
        let _stand_in = install_stand_in(std::sync::Arc::clone(&recorder));

        let dir = tempfile::tempdir().expect("каталог");
        let workspace = dir.path().to_path_buf();
        let context = WorkspaceContext {
            cwd: workspace.clone(),
            workspace_root: workspace.clone(),
            cache_root: workspace.join(".build/unica"),
            workspace_epoch: 1,
        };

        let mut args = Map::new();
        args.insert("query".to_string(), json!("СтрНайти"));
        args.insert("sourceKinds".to_string(), json!(["platform-help"]));
        super::InfrastructureApplicationPorts::new()
            .invoke_handler(
                spec(
                    "unica.documentation.search",
                    ToolHandler::Documentation {
                        operation: "search",
                    },
                ),
                &args,
                &context,
                InvocationMode::Read,
                &crate::domain::cancellation::CancellationToken::default(),
            )
            .expect("вызов с применимым фильтром обязан пройти");
        {
            let seen = recorder
                .seen
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            assert_eq!(seen.len(), 1, "применимый стенд опрошен ровно раз");
            assert_eq!(
                seen[0].0.source_kinds,
                vec![crate::domain::documentation::SourceKind::PlatformHelp],
                "разобранный фильтр обязан дойти до запроса, а не подменяться пустым"
            );
        }

        let mut alien = Map::new();
        alien.insert("query".to_string(), json!("СтрНайти"));
        alien.insert("sourceKinds".to_string(), json!(["standards"]));
        let error = match super::InfrastructureApplicationPorts::new().invoke_handler(
            spec(
                "unica.documentation.search",
                ToolHandler::Documentation {
                    operation: "search",
                },
            ),
            &alien,
            &context,
            InvocationMode::Read,
            &crate::domain::cancellation::CancellationToken::default(),
        ) {
            Ok(_) => panic!("чужое значение sourceKinds обязано отклоняться"),
            Err(error) => error,
        };
        assert!(
            error.contains("standards") && error.contains("platform-help"),
            "отказ обязан назвать чужое значение и допустимые, получено {error}"
        );

        // Нестроковое значение — тот же отказ, но с самим значением, а не с
        // пустой строкой: «unknown value \"\"» не говорит автору вызова ничего.
        let mut non_string = Map::new();
        non_string.insert("query".to_string(), json!("СтрНайти"));
        non_string.insert("sourceKinds".to_string(), json!([42]));
        let error = match super::InfrastructureApplicationPorts::new().invoke_handler(
            spec(
                "unica.documentation.search",
                ToolHandler::Documentation {
                    operation: "search",
                },
            ),
            &non_string,
            &context,
            InvocationMode::Read,
            &crate::domain::cancellation::CancellationToken::default(),
        ) {
            Ok(_) => panic!("нестроковое значение sourceKinds обязано отклоняться"),
            Err(error) => error,
        };
        assert!(
            error.contains("42"),
            "отказ обязан назвать само значение, получено {error}"
        );
    }

    /// Ветка `unica.documentation.get`: аргументы обязаны дойти до владельца
    /// локатора — `documentId` и `language` в вызов `get`, `platformVersion`
    /// в контекст, — а документ владельца обязан дойти обратно типизированным
    /// `data.document`. Версия намеренно невозможная: разрешение установки не
    /// должно зависеть от машин сборки.
    #[test]
    fn the_documentation_get_branch_carries_arguments_and_returns_the_document() {
        use crate::application::ports::ApplicationPorts;

        let recorder = std::sync::Arc::new(RecordingProvider::default());
        let _stand_in = install_stand_in(std::sync::Arc::clone(&recorder));

        let dir = tempfile::tempdir().expect("каталог");
        let workspace = dir.path().to_path_buf();
        let context = WorkspaceContext {
            cwd: workspace.clone(),
            workspace_root: workspace.clone(),
            cache_root: workspace.join(".build/unica"),
            workspace_epoch: 1,
        };

        let mut args = Map::new();
        args.insert(
            "documentId".to_string(),
            json!("platform-syntax-help:syntax-context:page.html"),
        );
        args.insert("language".to_string(), json!("en"));
        args.insert("platformVersion".to_string(), json!("9.9.9.9999"));

        let outcome = super::InfrastructureApplicationPorts::new()
            .invoke_handler(
                spec(
                    "unica.documentation.get",
                    ToolHandler::Documentation { operation: "get" },
                ),
                &args,
                &context,
                InvocationMode::Read,
                &crate::domain::cancellation::CancellationToken::default(),
            )
            .expect("ветка get обязана ответить");

        let seen = recorder
            .seen_gets
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert_eq!(seen.len(), 1, "владелец опрошен ровно раз");
        let (document_id, language, documentation_context) = &seen[0];
        assert_eq!(document_id, "platform-syntax-help:syntax-context:page.html");
        assert_eq!(language, "en", "language обязан дойти до get");
        assert_eq!(
            documentation_context.platform_version.as_deref(),
            Some("9.9.9.9999"),
            "platformVersion обязан дойти до контекста"
        );

        let data = outcome.data.expect("типизированный data");
        assert_eq!(
            data["document"]["documentId"],
            "platform-syntax-help:syntax-context:page.html"
        );
        assert_eq!(data["document"]["text"], "Полный текст.");
    }

    /// `get` reader опрашивает владельца локатора и требует `documentId`.
    #[test]
    fn documentation_get_reader_polls_owner_and_requires_document_id() {
        use crate::application::ports::ApplicationPorts;

        let recorder = std::sync::Arc::new(RecordingProvider::default());
        let _stand_in = install_stand_in(std::sync::Arc::clone(&recorder));

        let dir = tempfile::tempdir().expect("каталог");
        let workspace = dir.path().to_path_buf();
        let context = WorkspaceContext {
            cwd: workspace.clone(),
            workspace_root: workspace.clone(),
            cache_root: workspace.join(".build/unica"),
            workspace_epoch: 1,
        };

        let mut args = Map::new();
        args.insert(
            "documentId".to_string(),
            json!("platform-syntax-help:syntax-context:page.html"),
        );
        let outcome = super::InfrastructureApplicationPorts::new()
            .invoke_handler(
                spec(
                    "unica.documentation.get",
                    ToolHandler::Documentation { operation: "get" },
                ),
                &args,
                &context,
                InvocationMode::Read,
                &crate::domain::cancellation::CancellationToken::default(),
            )
            .expect("reader обязан ответить успехом");
        assert_eq!(
            recorder
                .seen_gets
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .len(),
            1,
            "reader обязан опросить владельца ровно раз"
        );
        assert!(!outcome.adapter.summary.contains("dry run"));
        assert!(
            outcome.data.is_some(),
            "успешный get reader обязан вернуть data"
        );

        let error = match super::InfrastructureApplicationPorts::new().invoke_handler(
            spec(
                "unica.documentation.get",
                ToolHandler::Documentation { operation: "get" },
            ),
            &Map::new(),
            &context,
            InvocationMode::Read,
            &crate::domain::cancellation::CancellationToken::default(),
        ) {
            Ok(_) => panic!("reader без documentId обязан отказывать"),
            Err(error) => error,
        };
        assert!(
            error.contains("documentId"),
            "отказ обязан назвать аргумент, получено {error}"
        );
    }

    /// Пустой-но-присутствующий `query` отклоняется до provider-а.
    #[test]
    fn documentation_reader_refuses_a_blank_query() {
        use crate::application::ports::ApplicationPorts;

        let recorder = std::sync::Arc::new(RecordingProvider::default());
        let _stand_in = install_stand_in(std::sync::Arc::clone(&recorder));

        let dir = tempfile::tempdir().expect("каталог");
        let workspace = dir.path().to_path_buf();
        let context = WorkspaceContext {
            cwd: workspace.clone(),
            workspace_root: workspace.clone(),
            cache_root: workspace.join(".build/unica"),
            workspace_epoch: 1,
        };

        let mut args = Map::new();
        args.insert("query".to_string(), json!("   "));

        let error = match super::InfrastructureApplicationPorts::new().invoke_handler(
            spec(
                "unica.documentation.search",
                ToolHandler::Documentation {
                    operation: "search",
                },
            ),
            &args,
            &context,
            InvocationMode::Read,
            &crate::domain::cancellation::CancellationToken::default(),
        ) {
            Ok(_) => panic!("reader обязан отклонять пробельный query"),
            Err(error) => error,
        };
        assert!(
            error.contains("query"),
            "отказ обязан назвать аргумент, получено {error}"
        );
    }
}
