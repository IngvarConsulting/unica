use std::path::{Path, PathBuf};
use unica_artifact_delivery::engine::{BundledTool, BundledToolError};
use unica_source_contract::workspace::{ResolvedSourceRoot, WorkspaceContext};
pub trait RlmEnvironment: Send + Sync {
    fn find_plugin_root(&self, cwd: &Path) -> Option<PathBuf>;
    fn resolve_bundled_tool(
        &self,
        root: &Path,
        name: &str,
        allow_download: bool,
    ) -> Result<BundledTool, BundledToolError>;
    fn resolve_source_root(&self, context: &WorkspaceContext)
        -> Result<ResolvedSourceRoot, String>;
}
