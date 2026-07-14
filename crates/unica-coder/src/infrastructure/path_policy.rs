use crate::domain::workspace::WorkspaceContext;
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
        let cwd = canonical_or_lexical(&self.context.cwd);
        let raw = if path.is_absolute() {
            path
        } else {
            cwd.join(path)
        };
        let normalized = normalize_lexically(&raw);
        let canonical_root = canonical_or_lexical(&self.context.workspace_root);

        let existing_ancestor = nearest_existing_ancestor(&normalized);
        let canonical_ancestor = existing_ancestor
            .canonicalize()
            .map_err(|err| format!("failed to inspect {}: {err}", existing_ancestor.display()))?;
        if !canonical_ancestor.starts_with(&canonical_root) {
            return Err(format!(
                "refusing to {operation} through symlink outside workspace root: {}",
                normalized.display()
            ));
        }

        Ok(normalized)
    }
}

fn canonical_or_lexical(path: &Path) -> PathBuf {
    path.canonicalize()
        .map(|path| normalize_lexically(&path))
        .unwrap_or_else(|_| normalize_lexically(path))
}

fn nearest_existing_ancestor(path: &Path) -> PathBuf {
    for ancestor in path.ancestors() {
        if ancestor.exists() {
            return ancestor.to_path_buf();
        }
    }
    PathBuf::from("/")
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
