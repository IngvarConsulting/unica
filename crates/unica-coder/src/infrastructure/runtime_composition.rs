//! Application bindings for read-only runtime observation and cache lifecycle.
use crate::infrastructure::workspace::discover_workspace;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use unica_execution::runtime_jobs::{RuntimeLifecycleObserver, TerminalRuntimeSuccess};
use unica_runner::attempt::RuntimeWorkspaceObservation;
use unica_source_contract::{events::DomainEvent, workspace::WorkspaceContext};
use unica_workspace::{cache::CacheAccess, state::WorkspaceStateRepository};
type JobResult<T> = Result<T, String>;

pub struct WorkspaceObservation;
impl RuntimeWorkspaceObservation for WorkspaceObservation {
    fn discover(&self, cwd: &Path) -> Result<WorkspaceContext, String> {
        discover_workspace(Some(cwd.to_path_buf()))
    }
}
static WORKSPACE_OBSERVATION: WorkspaceObservation = WorkspaceObservation;
pub fn workspace_observer() -> &'static dyn RuntimeWorkspaceObservation {
    &WORKSPACE_OBSERVATION
}
pub fn shared_workspace_observer() -> Arc<dyn RuntimeWorkspaceObservation> {
    Arc::new(WorkspaceObservation)
}

pub struct RuntimeCacheLifecycle;
impl RuntimeLifecycleObserver for RuntimeCacheLifecycle {
    fn after_success(&self, success: &TerminalRuntimeSuccess) -> Result<(), String> {
        apply_runtime_success_effects(success.cwd(), success.operation().label(), success.job_id())
    }
}

pub fn run_worker_from_args(_args: &[String]) -> Result<(), String> {
    unica_adapter_v8_runner::worker::run_worker_from_reader(
        std::io::stdin().lock(),
        shared_workspace_observer(),
        &RuntimeCacheLifecycle,
    )
}

fn apply_runtime_success_effects(cwd: &Path, operation: &str, job_id: &str) -> JobResult<()> {
    let Some(event_kind) = runtime_event_kind(operation) else {
        return Ok(());
    };
    let context = discover_workspace(Some(cwd.to_path_buf()))?;
    let events = vec![DomainEvent::new(
        event_kind,
        format!("runtime-job:{job_id}"),
    )];
    let report = WorkspaceStateRepository::new(&context).report(
        &context,
        &events,
        false,
        CacheAccess {
            reads: &[],
            writes: &["workspace_graph", "metadata_graph"],
        },
    )?;
    super::analysis_composition::service_manager().notify_invalidation(&context, &events);
    if report.publication_warnings.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "runtime cache state committed with cleanup warning: {}",
            report.publication_warnings.join("; ")
        ))
    }
}

impl unica_adapter_v8_runner::adapter::RuntimeEnvironment for WorkspaceObservation {
    fn workspace(&self) -> &dyn RuntimeWorkspaceObservation {
        self
    }
    fn plugin_root(&self, cwd: &Path) -> Option<PathBuf> {
        super::plugin_runtime::find_plugin_root(cwd)
    }
    fn resolve_v8_tool(
        &self,
        plugin_root: &Path,
        require_exists: bool,
    ) -> Result<unica_artifact_delivery::engine::BundledTool, String> {
        super::bundled_tools::resolve_bundled_tool(plugin_root, "v8-runner", require_exists)
            .map_err(Into::into)
    }
    fn preview(
        &self,
        tool_name: &str,
        label: &str,
        mutating: bool,
        tool: unica_artifact_delivery::engine::BundledTool,
        command: Vec<String>,
    ) -> unica_execution_contract::adapter_outcome::AdapterOutcome {
        super::internal_adapters::dry_run_outcome(tool_name, label, mutating, tool, command)
    }
}
pub fn runtime_environment() -> &'static dyn unica_adapter_v8_runner::adapter::RuntimeEnvironment {
    &WORKSPACE_OBSERVATION
}

use unica_runner::operation::runtime_event_kind;

/// Bind the Designer codec to the existing cross-format inventory classifier.
pub fn full_dump_semantics(
) -> std::sync::Arc<dyn unica_source_contract::read::full_dump::DumpSemanticValidator> {
    std::sync::Arc::new(
        unica_source_designer::full_dump_validation::DesignerDumpValidator::new(|kind, files| {
            unica_source_designer::inventory::classify_physical_source_inventory(
                kind,
                files.iter().copied(),
            )
        }),
    )
}
