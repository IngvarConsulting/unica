use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct WorkspaceContext {
    pub cwd: PathBuf,
    pub workspace_root: PathBuf,
    pub cache_root: PathBuf,
    pub workspace_epoch: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSourceRoot {
    pub source_set: Option<String>,
    pub path: PathBuf,
}
