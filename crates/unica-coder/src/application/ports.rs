use super::{project_map, project_status, ToolHandler, ToolSpec};
use crate::domain::cache::{CacheAccess, CacheReport};
use crate::domain::cancellation::CancellationToken;
use crate::domain::events::DomainEvent;
use crate::domain::project_sources::discover_project_source_map;
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::internal_adapters::{
    BslAnalyzerMcpAdapter, CliAdapter, CodeNavigationAdapter, CodeSearchAdapter,
    ConfigDumpInfoGitCheck, GitTrackingAdapter, RuntimeAdapter, RuntimeJobAdapter,
    StandardsAdapter,
};
use crate::infrastructure::native_operations::NativeOperationAdapter;
use crate::infrastructure::workspace_services::WorkspaceServiceManager;
use crate::infrastructure::workspace_state::WorkspaceStateRepository;
use crate::infrastructure::AdapterOutcome;
use serde_json::{Map, Value};
use std::path::PathBuf;

pub(crate) struct HandlerOutcome {
    pub(crate) adapter: AdapterOutcome,
    pub(crate) job: Option<Value>,
}

impl HandlerOutcome {
    pub(crate) fn plain(adapter: AdapterOutcome) -> Self {
        Self { adapter, job: None }
    }
}

pub(crate) trait ApplicationPorts: Send + Sync {
    fn discover_workspace(&self, cwd: PathBuf) -> Result<WorkspaceContext, String>;

    fn invoke_handler(
        &self,
        spec: ToolSpec,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
        dry_run: bool,
        cancellation: &CancellationToken,
    ) -> Result<HandlerOutcome, String>;

    fn cache_report(
        &self,
        context: &WorkspaceContext,
        events: &[DomainEvent],
        dry_run: bool,
        cache_access: CacheAccess,
    ) -> Result<CacheReport, String>;

    fn notify_invalidation(&self, context: &WorkspaceContext, events: &[DomainEvent]);
}

pub(crate) struct DefaultApplicationPorts;

impl ApplicationPorts for DefaultApplicationPorts {
    fn discover_workspace(&self, cwd: PathBuf) -> Result<WorkspaceContext, String> {
        WorkspaceContext::discover(cwd)
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
            ToolHandler::NativeOperation { operation, .. } => NativeOperationAdapter::invoke(
                operation,
                spec.name,
                args,
                context,
                dry_run,
                spec.mutating,
            )
            .map(HandlerOutcome::plain),
            ToolHandler::ProjectStatus => {
                let source_map = discover_project_source_map(&context.workspace_root);
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
                let source_map = discover_project_source_map(&context.workspace_root);
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
