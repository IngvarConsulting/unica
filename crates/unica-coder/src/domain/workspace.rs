use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct WorkspaceContext {
    pub cwd: PathBuf,
    pub workspace_root: PathBuf,
    pub cache_root: PathBuf,
    pub workspace_epoch: u64,
}
