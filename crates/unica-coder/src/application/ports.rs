use super::{project_map, project_status, ToolHandler, ToolSpec};
use crate::domain::cache::{CacheAccess, CacheReport};
use crate::domain::events::DomainEvent;
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::extension_point_discovery::ExtensionPointDiscoveryAdapter;
use crate::infrastructure::internal_adapters::{
    BslAnalyzerMcpAdapter, CliAdapter, CodeNavigationAdapter, CodeSearchAdapter, RuntimeAdapter,
    StandardsAdapter,
};
use crate::infrastructure::native_operations::NativeOperationAdapter;
use crate::infrastructure::workspace_services::WorkspaceServiceManager;
use crate::infrastructure::workspace_state::WorkspaceStateRepository;
use crate::infrastructure::AdapterOutcome;
use serde_json::{Map, Value};
use std::path::PathBuf;

pub(crate) trait ApplicationPorts {
    fn discover_workspace(&self, cwd: PathBuf) -> Result<WorkspaceContext, String>;

    fn invoke_handler(
        &self,
        spec: ToolSpec,
        args: &Map<String, Value>,
        context: &WorkspaceContext,
        dry_run: bool,
    ) -> Result<AdapterOutcome, String>;

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
    ) -> Result<AdapterOutcome, String> {
        match spec.handler {
            ToolHandler::NativeOperation { operation, .. } => NativeOperationAdapter::invoke(
                operation,
                spec.name,
                args,
                context,
                dry_run,
                spec.mutating,
            ),
            ToolHandler::ProjectStatus => Ok(project_status(context)),
            ToolHandler::ProjectMap => Ok(project_map(context)),
            ToolHandler::ProjectDiscovery => {
                ExtensionPointDiscoveryAdapter::new().invoke(spec.name, args, context, dry_run)
            }
            ToolHandler::BuildRuntime { command, .. } => CliAdapter::new(
                "v8-runner",
                command,
                "build/runtime",
            )
            .invoke(spec.name, args, context, dry_run, spec.mutating),
            ToolHandler::RuntimeAdapter => {
                RuntimeAdapter::new().invoke(spec.name, args, context, dry_run, spec.mutating)
            }
            ToolHandler::CodeAdapter { command } if command == ["search"] => {
                CodeSearchAdapter::new().invoke(spec.name, args, context, dry_run)
            }
            ToolHandler::CodeAdapter {
                command: ["definition"] | ["outline"] | ["grep"] | ["meta-profile"],
            } => CodeNavigationAdapter::new().invoke(spec.name, args, context, dry_run),
            ToolHandler::CodeAdapter {
                command: ["graph"] | ["analyze"],
            } => BslAnalyzerMcpAdapter::new().invoke(spec.name, args, context, dry_run),
            ToolHandler::CodeAdapter { command } => CliAdapter::new(
                "bsl-analyzer",
                command,
                "code analysis",
            )
            .invoke(spec.name, args, context, dry_run, spec.mutating),
            ToolHandler::StandardsAdapter { operation } => {
                Ok(StandardsAdapter::invoke(operation, args))
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
