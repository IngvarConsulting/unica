use crate::application::ports::{
    ApplicationPorts, FormatGuardCheck, HandlerOutcome, SupportGuardCheck,
};
use crate::application::{project_map, project_status, AdapterOutcome, ToolHandler, ToolSpec};
use crate::domain::cache::{CacheAccess, CacheReport};
use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::{
    CodeIntelligenceContext, CodeIntelligenceProvider, CodeIntelligenceReadRequest,
    CodeIntelligenceRegistry,
};
use crate::domain::events::DomainEvent;
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::bundled_tools::resolve_bundled_tool;
use crate::infrastructure::code_intelligence::{BslAnalyzerProvider, GitGrepProvider, RlmProvider};
use crate::infrastructure::internal_adapters::{
    system_process_runner, BslAnalyzerMcpAdapter, CliAdapter, ConfigDumpInfoGitCheck,
    GitTrackingAdapter, ProcessCommand, RuntimeAdapter,
    RuntimeJobAdapter, StandardsAdapter,
};
use crate::infrastructure::native_operations::NativeOperationAdapter;
use crate::infrastructure::plugin_runtime::find_plugin_root;
use crate::infrastructure::workspace_services::WorkspaceServiceManager;
use crate::infrastructure::workspace_state::WorkspaceStateRepository;
use serde_json::{Map, Value};
use std::borrow::Cow;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use unica_adapter_platform_xml::PlatformXmlAdapterFactory;
use unica_application::OperationalPolicyService;
use unica_format_core::ports::{
    FormatDiagnosticCode, PublicationArtifact, PublicationChange, PublicationCleanup,
    PublicationInvocation, PublicationRecovery, PublicationRequest, PublicationResult,
    PublicationStatus,
};
pub(crate) struct InfrastructureApplicationPorts;

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
    ) -> Result<FormatGuardCheck, String> {
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
            return invoke_verified_full_dump(spec.name, invocation, args, context, cancellation);
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
                    cancellation,
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
            }),
            ToolHandler::CodeIntelligence { .. } => Err(format!(
                "{} must be dispatched through the provider-neutral code intelligence registry",
                spec.name
            )),
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

fn invoke_verified_full_dump(
    operation_name: &str,
    invocation: PublicationInvocation,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    cancellation: &CancellationToken,
) -> Result<HandlerOutcome, String> {
    let factory = PlatformXmlAdapterFactory::new();
    let session = factory.capture_publication_session(
        operation_name,
        args,
        &context.workspace_root,
        &context.cwd,
        |program, args, cwd, timeout, cancellation| {
            system_process_runner()
                .run(&ProcessCommand {
                    program: program.to_path_buf(),
                    args: args.to_vec(),
                    cwd: cwd.to_path_buf(),
                    timeout,
                    cancellation: cancellation.clone(),
                })
                .map(|output| {
                    (
                        output.status_success,
                        output.status,
                        output.stdout,
                        output.stderr,
                        output.timed_out,
                        output.cancelled,
                        output.stdout_truncated,
                    )
                })
        },
        |cwd, tool, require_executable| {
            let plugin_root = find_plugin_root(cwd)
                .ok_or_else(|| "publication runtime is unavailable".to_string())?;
            resolve_bundled_tool(&plugin_root, tool, require_executable)
                .map(|resolved| (resolved.program, resolved.warnings))
        },
    );
    let request = PublicationRequest::new(session, invocation, cancellation.clone());
    let registration = factory.operational_registration();
    let result = OperationalPolicyService::publish(registration.publication(), &request)
        .map_err(|_| "publication adapter operation failed".to_string())?;
    let data = serde_json::json!({ "publication": result.lifecycle() });
    Ok(HandlerOutcome::with_data(
        publication_outcome(&result),
        data,
    ))
}

fn publication_outcome(result: &PublicationResult) -> AdapterOutcome {
    let artifacts = result
        .artifacts()
        .iter()
        .map(|artifact| match artifact {
            PublicationArtifact::PublishedSource => "publishedSource".to_string(),
            PublicationArtifact::RecoveryState => "recoveryState".to_string(),
        })
        .collect::<Vec<_>>();
    let errors = result
        .diagnostics()
        .iter()
        .map(|diagnostic| publication_diagnostic_message(diagnostic.code()).to_string())
        .collect::<Vec<_>>();
    let summary = match result.status() {
        PublicationStatus::Published => "Full source publication completed.",
        PublicationStatus::DryRun => "Full source publication preview completed.",
        PublicationStatus::Cancelled => "Full source publication was cancelled.",
        PublicationStatus::Failed if result.recovery() == PublicationRecovery::Required => {
            "Full source publication failed and requires recovery."
        }
        PublicationStatus::Failed if result.cleanup() == PublicationCleanup::Failed => {
            "Full source publication failed during private cleanup."
        }
        PublicationStatus::Failed => "Full source publication failed.",
    };
    AdapterOutcome {
        ok: matches!(
            result.status(),
            PublicationStatus::Published | PublicationStatus::DryRun
        ),
        summary: summary.to_string(),
        changes: result
            .changes()
            .iter()
            .map(|change| match change {
                PublicationChange::FullSourceReplaced => "full source replaced".to_string(),
            })
            .collect(),
        warnings: Vec::new(),
        errors: errors.clone(),
        artifacts,
        stdout: None,
        stderr: (!errors.is_empty()).then(|| format!("{}\n", errors.join("\n"))),
        command: None,
    }
}

fn publication_diagnostic_message(code: FormatDiagnosticCode) -> &'static str {
    match code {
        FormatDiagnosticCode::PublicationCancelled => "Publication was cancelled.",
        FormatDiagnosticCode::PublicationRecoveryRequired => {
            "Publication requires recovery before another write."
        }
        FormatDiagnosticCode::PublicationCleanupFailed => {
            "Publication could not complete private cleanup."
        }
        FormatDiagnosticCode::PublicationFailed => {
            "Publication failed before a verified source replacement completed."
        }
        _ => "Publication could not be completed safely.",
    }
}

fn verified_full_dump_invocation(
    spec: ToolSpec,
    args: &Map<String, Value>,
    dry_run: bool,
) -> Option<PublicationInvocation> {
    if !is_applied_full_dump(args, dry_run) {
        return None;
    }
    match spec.handler {
        ToolHandler::BuildRuntime { command, .. } if command == ["dump"] => {
            Some(PublicationInvocation::BuildDump)
        }
        ToolHandler::RuntimeAdapter
            if args.get("operation").and_then(Value::as_str) == Some("dump") =>
        {
            Some(PublicationInvocation::RuntimeExecute)
        }
        _ => None,
    }
}

#[cfg(test)]
mod task7_fix_round1_publication_tests {
    use super::{invoke_verified_full_dump, publication_outcome};
    use crate::domain::{cancellation::CancellationToken, workspace::WorkspaceContext};
    use serde_json::{json, Map};
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };
    use unica_format_core::ports::{
        FormatDiagnostic, FormatDiagnosticCode, FormatDiagnosticDetail, PublicationArtifact,
        PublicationCancellation, PublicationChange, PublicationCleanup, PublicationFailureKind,
        PublicationInvocation, PublicationIssueKind, PublicationLifecycle, PublicationRecovery,
        PublicationResult, PublicationRollback,
    };

    #[test]
    fn cancelled_publication_json_preserves_typed_state_without_paths_or_native_keys() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "unica-task7-public-json-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        let context = WorkspaceContext {
            cwd: root.clone(),
            workspace_root: root.clone(),
            cache_root: root.join(".cache"),
            workspace_epoch: 1,
        };
        let args = Map::from_iter([
            (
                "config".to_string(),
                json!("/private/unix/Configuration.xml"),
            ),
            ("workdir".to_string(), json!(r"C:\private\workspace")),
            ("nativeTag".to_string(), json!("MetaDataObject")),
        ]);
        let cancellation = CancellationToken::new();
        cancellation.cancel();

        let outcome = invoke_verified_full_dump(
            "alternate.publish",
            PublicationInvocation::BuildDump,
            &args,
            &context,
            &cancellation,
        )
        .unwrap();
        let public = serde_json::to_string(&outcome).unwrap();

        for forbidden in [
            root.to_string_lossy().as_ref(),
            "/private/unix",
            r"C:\private\workspace",
            "Configuration.xml",
            "MetaDataObject",
            "ParentConfigurations.bin",
            "8.3.27",
            "2.20",
        ] {
            assert!(!public.contains(forbidden), "leaked {forbidden}: {public}");
        }
        for expected in [
            r#""state":"cancelled""#,
            r#""cancellation":"beforeExecution""#,
            r#""rollback":"notNeeded""#,
            r#""cleanup":"completed""#,
            r#""recovery":"notRequired""#,
        ] {
            assert!(public.contains(expected), "missing {expected}: {public}");
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn every_publication_outcome_ignores_path_bearing_free_form_text() {
        let diagnostic = |code, issue| {
            FormatDiagnostic::new(code, FormatDiagnosticDetail::Publication(issue)).unwrap()
        };
        let cases = [
            PublicationResult::new(
                PublicationLifecycle::published(),
                Vec::new(),
                vec![PublicationChange::FullSourceReplaced],
                vec![PublicationArtifact::PublishedSource],
            )
            .unwrap(),
            PublicationResult::new(
                PublicationLifecycle::cancelled(
                    PublicationCancellation::BeforePublication,
                    PublicationRollback::NotNeeded,
                    PublicationCleanup::Completed,
                    PublicationRecovery::NotRequired,
                )
                .unwrap(),
                vec![diagnostic(
                    FormatDiagnosticCode::PublicationCancelled,
                    PublicationIssueKind::Cancelled,
                )],
                Vec::new(),
                Vec::new(),
            )
            .unwrap(),
            PublicationResult::new(
                PublicationLifecycle::failed(
                    PublicationFailureKind::Cleanup,
                    PublicationCancellation::NotRequested,
                    PublicationRollback::NotNeeded,
                    PublicationCleanup::Failed,
                    PublicationRecovery::Required,
                )
                .unwrap(),
                vec![
                    diagnostic(
                        FormatDiagnosticCode::PublicationFailed,
                        PublicationIssueKind::Failed,
                    ),
                    diagnostic(
                        FormatDiagnosticCode::PublicationCleanupFailed,
                        PublicationIssueKind::CleanupFailed,
                    ),
                    diagnostic(
                        FormatDiagnosticCode::PublicationRecoveryRequired,
                        PublicationIssueKind::RecoveryRequired,
                    ),
                ],
                Vec::new(),
                vec![PublicationArtifact::RecoveryState],
            )
            .unwrap(),
            PublicationResult::new(
                PublicationLifecycle::failed(
                    PublicationFailureKind::Publication,
                    PublicationCancellation::DuringPublication,
                    PublicationRollback::Failed,
                    PublicationCleanup::RetainedForRecovery,
                    PublicationRecovery::Required,
                )
                .unwrap(),
                vec![
                    diagnostic(
                        FormatDiagnosticCode::PublicationFailed,
                        PublicationIssueKind::Failed,
                    ),
                    diagnostic(
                        FormatDiagnosticCode::PublicationCancelled,
                        PublicationIssueKind::Cancelled,
                    ),
                    diagnostic(
                        FormatDiagnosticCode::PublicationRecoveryRequired,
                        PublicationIssueKind::RecoveryRequired,
                    ),
                ],
                Vec::new(),
                vec![PublicationArtifact::RecoveryState],
            )
            .unwrap(),
        ];

        for result in cases {
            let public = serde_json::to_string(&publication_outcome(&result)).unwrap();
            for forbidden in [
                "/private/source",
                r"C:\private\source",
                "Configuration.xml",
                "MetaDataObject",
                "ParentConfigurations.bin",
                "8.3.27",
                "2.20",
            ] {
                assert!(!public.contains(forbidden), "leaked {forbidden}: {public}");
            }
        }
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
    use serde_json::{json, Map};
    use std::path::PathBuf;
    use unica_format_core::ports::PublicationInvocation;

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
            Some(PublicationInvocation::BuildDump)
        );
        assert_eq!(
            verified_full_dump_invocation(runtime, &runtime_args, false),
            Some(PublicationInvocation::RuntimeExecute)
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
