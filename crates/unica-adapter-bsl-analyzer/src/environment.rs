//! Trusted composition observations; lookup and delivery algorithms have one owner.
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use unica_analysis_services::backend::ServiceEnvironment;
use unica_artifact_delivery::engine::{BundledTool, BundledToolError, MissingEngine};
use unica_execution_contract::adapter_outcome::AdapterOutcome;
use unica_source_contract::workspace::{ResolvedSourceRoot, WorkspaceContext};
pub trait BslEnvironment: Send + Sync {
    fn find_plugin_root(&self, cwd: &Path) -> Option<PathBuf>;
    fn resolve_bundled_tool(
        &self,
        root: &Path,
        tool: &str,
        allow_download: bool,
    ) -> Result<BundledTool, BundledToolError>;
    fn missing_engine(&self, root: &Path, tool: &str) -> Option<MissingEngine>;
    fn bundled_tool_version(&self, root: &Path, tool: &str) -> Result<String, String>;
    fn resolve_source_root(
        &self,
        context: &WorkspaceContext,
        explicit: Option<&str>,
    ) -> Result<ResolvedSourceRoot, String>;
    fn discover_workspace(&self, cwd: Option<PathBuf>) -> Result<WorkspaceContext, String>;
    fn service_environment(&self) -> Arc<dyn ServiceEnvironment>;
    fn dry_run_outcome(
        &self,
        tool_name: &str,
        label: &str,
        mutating: bool,
        bundled_tool: BundledTool,
        command: Vec<String>,
    ) -> AdapterOutcome;
    fn cli_args(&self, args: &Map<String, Value>, redact: bool) -> Result<Vec<String>, String>;
}
