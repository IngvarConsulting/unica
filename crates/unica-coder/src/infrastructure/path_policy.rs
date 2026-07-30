use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::platform::filesystem::path_starts_with_host_root;
use crate::infrastructure::source_roots::normalize_path_identity;
use std::path::{Component, Path, PathBuf};

pub struct WorkspacePathPolicy<'a> {
    context: &'a WorkspaceContext,
}

impl<'a> WorkspacePathPolicy<'a> {
    pub fn new(context: &'a WorkspaceContext) -> Self {
        Self { context }
    }

    pub fn resolve_write(&self, path: impl Into<PathBuf>) -> Result<PathBuf, String> {
        self.resolve_workspace_path(path.into(), "write")
    }

    fn resolve_workspace_path(&self, path: PathBuf, operation: &str) -> Result<PathBuf, String> {
        let cwd = normalize_path_identity(&self.context.cwd)
            .map_err(|error| format!("failed to inspect workspace cwd: {error}"))?;
        let raw = if path.is_absolute() {
            path
        } else {
            cwd.join(path)
        };
        let normalized = normalize_lexically(&raw);
        let lexical_workspace = normalize_lexically(&self.context.workspace_root);
        let lexical_path = normalize_lexically(&normalized);
        let workspace_identity = normalize_path_identity(&self.context.workspace_root)
            .map_err(|error| format!("failed to inspect workspace root: {error}"))?;

        if !path_starts_with_host_root(&lexical_path, &lexical_workspace)
            && !path_starts_with_host_root(&lexical_path, &workspace_identity)
        {
            return Err(format!(
                "refusing to {operation} outside workspace root: {}",
                normalized.display()
            ));
        }

        let path_identity = normalize_path_identity(&normalized)
            .map_err(|error| format!("failed to inspect workspace path: {error}"))?;
        if !path_starts_with_host_root(&path_identity, &workspace_identity) {
            return Err(format!(
                "refusing to {operation} through symlink outside workspace root: {}",
                normalized.display()
            ));
        }

        Ok(normalized)
    }
}

fn normalize_lexically(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_write_escape_outside_workspace_root() {
        let temp = std::env::temp_dir().join(format!("unica-path-policy-{}", std::process::id()));
        std::fs::create_dir_all(temp.join("workspace")).unwrap();
        let context = WorkspaceContext {
            cwd: temp.join("workspace"),
            workspace_root: temp.join("workspace"),
            cache_root: temp.join("workspace").join(".build").join("unica"),
            workspace_epoch: 1,
        };
        let policy = WorkspacePathPolicy::new(&context);

        let error = policy
            .resolve_write("../outside/Configuration.xml")
            .unwrap_err();

        assert!(error.contains("outside workspace root"));

        let _ = std::fs::remove_dir_all(temp);
    }
}
