use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::platform::filesystem::strip_windows_extended_length_prefix;
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
        let lexical_workspace = normalize_lexically(&strip_windows_extended_length_prefix(
            &self.context.workspace_root,
        ));
        let lexical_path = normalize_lexically(&strip_windows_extended_length_prefix(&normalized));
        let workspace_identity = normalize_path_identity(&self.context.workspace_root)
            .map_err(|error| format!("failed to inspect workspace root: {error}"))?;

        if !path_starts_with_host_policy(&lexical_path, &lexical_workspace)
            && !path_starts_with_host_policy(&lexical_path, &workspace_identity)
        {
            return Err(format!(
                "refusing to {operation} outside workspace root: {}",
                normalized.display()
            ));
        }

        let path_identity = normalize_path_identity(&normalized)
            .map_err(|error| format!("failed to inspect workspace path: {error}"))?;
        if !path_starts_with_host_policy(&path_identity, &workspace_identity) {
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

#[cfg(windows)]
fn path_starts_with_host_policy(path: &Path, base: &Path) -> bool {
    let path_components = path.components().collect::<Vec<_>>();
    let base_components = base.components().collect::<Vec<_>>();
    path_components.len() >= base_components.len()
        && path_components
            .iter()
            .zip(base_components.iter())
            .all(|(left, right)| {
                left.as_os_str().to_string_lossy().to_lowercase()
                    == right.as_os_str().to_string_lossy().to_lowercase()
            })
}

#[cfg(not(windows))]
fn path_starts_with_host_policy(path: &Path, base: &Path) -> bool {
    path.starts_with(base)
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

    #[cfg(unix)]
    #[test]
    fn rejects_lexically_external_symlink_that_points_into_workspace() {
        use std::os::unix::fs::symlink;
        use std::time::{SystemTime, UNIX_EPOCH};

        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp = std::env::temp_dir().join(format!(
            "unica-path-policy-inbound-link-{}-{nanos}",
            std::process::id()
        ));
        let workspace = temp.join("workspace");
        let outside = temp.join("outside");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(workspace.join("Configuration.xml"), "<MetaDataObject/>").unwrap();
        symlink(&workspace, outside.join("workspace-alias")).unwrap();
        let context = WorkspaceContext {
            cwd: workspace.clone(),
            workspace_root: workspace.clone(),
            cache_root: workspace.join(".build").join("unica"),
            workspace_epoch: 1,
        };
        let policy = WorkspacePathPolicy::new(&context);

        let error = policy
            .resolve_write(outside.join("workspace-alias/Configuration.xml"))
            .unwrap_err();

        assert!(error.contains("outside workspace root"));
        let _ = std::fs::remove_dir_all(temp);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_lexically_internal_symlink_that_points_outside_workspace() {
        use std::os::unix::fs::symlink;
        use std::time::{SystemTime, UNIX_EPOCH};

        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp = std::env::temp_dir().join(format!(
            "unica-path-policy-outbound-link-{}-{nanos}",
            std::process::id()
        ));
        let workspace = temp.join("workspace");
        let outside = temp.join("outside");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("Configuration.xml"), "<MetaDataObject/>").unwrap();
        symlink(&outside, workspace.join("outside-alias")).unwrap();
        let context = WorkspaceContext {
            cwd: workspace.clone(),
            workspace_root: workspace.clone(),
            cache_root: workspace.join(".build").join("unica"),
            workspace_epoch: 1,
        };
        let policy = WorkspacePathPolicy::new(&context);

        let error = policy
            .resolve_write(workspace.join("outside-alias/Configuration.xml"))
            .unwrap_err();

        assert!(error.contains("outside workspace root"));
        let _ = std::fs::remove_dir_all(temp);
    }

    #[cfg(windows)]
    #[test]
    fn accepts_identity_normalized_child_of_verbatim_workspace_root() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let regular_root = std::env::temp_dir().join(format!(
            "unica-path-policy-verbatim-{}-{nanos}",
            std::process::id()
        ));
        let child = regular_root.join("src/CommonModules/Example.xml");
        std::fs::create_dir_all(child.parent().unwrap()).unwrap();
        std::fs::write(&child, "<MetaDataObject/>").unwrap();
        let verbatim_root = PathBuf::from(format!(r"\\?\{}", regular_root.display()));
        let context = WorkspaceContext {
            cwd: verbatim_root.clone(),
            workspace_root: verbatim_root.clone(),
            cache_root: verbatim_root.join(".build").join("unica"),
            workspace_epoch: 1,
        };
        let policy = WorkspacePathPolicy::new(&context);
        let normalized_child = normalize_path_identity(&child).unwrap();

        assert_eq!(
            policy.resolve_write(normalized_child.clone()).unwrap(),
            normalized_child
        );

        let _ = std::fs::remove_dir_all(regular_root);
    }

    #[cfg(windows)]
    #[test]
    fn lexical_containment_follows_windows_case_policy() {
        assert!(path_starts_with_host_policy(
            Path::new(r"C:\WORKSPACE\src\Module.bsl"),
            Path::new(r"c:\workspace")
        ));
    }
}
