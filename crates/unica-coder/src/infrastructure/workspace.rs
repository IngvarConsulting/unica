use crate::domain::workspace::WorkspaceContext;
use std::collections::hash_map::DefaultHasher;
use std::env;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

pub(crate) fn discover_workspace(
    requested_cwd: Option<PathBuf>,
) -> Result<WorkspaceContext, String> {
    let cwd = requested_cwd.unwrap_or(
        env::current_dir().map_err(|err| format!("failed to read current directory: {err}"))?,
    );
    let cwd = if cwd.is_absolute() {
        cwd
    } else {
        env::current_dir()
            .map_err(|err| format!("failed to read current directory: {err}"))?
            .join(cwd)
    };
    let workspace_root = find_workspace_root(&cwd).unwrap_or_else(|| cwd.clone());
    let cache_root = env::var("UNICA_CACHE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| workspace_root.join(".build").join("unica"));
    let workspace_epoch = workspace_fingerprint(&workspace_root);
    Ok(WorkspaceContext {
        cwd,
        workspace_root,
        cache_root,
        workspace_epoch,
    })
}

fn find_workspace_root(cwd: &Path) -> Option<PathBuf> {
    for base in cwd.ancestors() {
        if base.join("v8project.yaml").is_file() {
            return Some(base.to_path_buf());
        }
    }
    None
}

fn workspace_fingerprint(root: &Path) -> u64 {
    let mut hasher = DefaultHasher::new();
    root.display().to_string().hash(&mut hasher);
    for rel in [
        "v8project.yaml",
        "Configuration.xml",
        "src/Configuration.xml",
        ".git/HEAD",
    ] {
        hash_path(&mut hasher, root, rel);
    }
    hasher.finish()
}

fn hash_path(hasher: &mut DefaultHasher, root: &Path, rel: &str) {
    rel.hash(hasher);
    let path = root.join(rel);
    let Ok(metadata) = path.metadata() else {
        0_u8.hash(hasher);
        return;
    };
    metadata.len().hash(hasher);
    if let Ok(modified) = metadata.modified() {
        let secs = modified
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or_default();
        secs.hash(hasher);
    }
}

#[cfg(test)]
mod tests {
    use super::discover_workspace;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn ignores_parent_git_without_v8project_yaml() {
        let root = temp_root("unica-workspace-parent-git");
        let workspace = root.join("workspace");
        let nested = workspace.join("src/catalogs");
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::create_dir_all(&nested).unwrap();

        let context = discover_workspace(Some(nested.clone())).unwrap();

        assert_eq!(context.workspace_root, nested);
        assert_ne!(context.workspace_root, root);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn falls_back_to_cwd_without_workspace_marker() {
        let root = temp_root("unica-workspace-no-marker");
        let cwd = root.join("workspace").join("src");
        std::fs::create_dir_all(&cwd).unwrap();

        let context = discover_workspace(Some(cwd.clone())).unwrap();

        assert_eq!(context.workspace_root, cwd);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn v8project_yaml_in_ancestor_defines_workspace_root() {
        let root = temp_root("unica-workspace-discovery");
        let workspace = root.join("workspace");
        let nested = workspace.join("src/catalogs");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(workspace.join("v8project.yaml"), "format: DESIGNER\n").unwrap();

        let context = discover_workspace(Some(nested)).unwrap();

        assert_eq!(context.workspace_root, workspace);
        assert_eq!(
            context.cache_root,
            context.workspace_root.join(".build/unica")
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn workspace_epoch_is_stable_until_a_fingerprint_marker_changes() {
        let root = temp_root("unica-workspace-epoch");
        std::fs::create_dir_all(&root).unwrap();
        let marker = root.join("v8project.yaml");
        std::fs::write(&marker, "format: DESIGNER\n").unwrap();

        let first = discover_workspace(Some(root.clone())).unwrap();
        let unchanged = discover_workspace(Some(root.clone())).unwrap();
        assert_eq!(first.workspace_epoch, unchanged.workspace_epoch);

        std::fs::write(&marker, "format: DESIGNER\nsource-set: []\n").unwrap();
        let changed = discover_workspace(Some(root.clone())).unwrap();
        assert_ne!(first.workspace_epoch, changed.workspace_epoch);

        let _ = std::fs::remove_dir_all(root);
    }

    fn temp_root(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{}-{nanos}", std::process::id()))
    }
}
