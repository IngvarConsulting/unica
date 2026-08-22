use crate::domain::workspace::WorkspaceContext;
use std::collections::hash_map::DefaultHasher;
use std::env;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

pub(crate) fn discover_workspace(
    requested_cwd: Option<PathBuf>,
) -> Result<WorkspaceContext, String> {
    discover_workspace_with_current_dir(requested_cwd, env::current_dir)
}

fn discover_workspace_with_current_dir(
    requested_cwd: Option<PathBuf>,
    current_dir: impl FnOnce() -> std::io::Result<PathBuf>,
) -> Result<WorkspaceContext, String> {
    let cwd = match requested_cwd {
        Some(cwd) if cwd.is_absolute() => cwd,
        Some(cwd) => current_dir()
            .map_err(|err| {
                format!(
                    "failed to resolve relative requested workspace `{}`: launch current directory is unavailable: {err}",
                    cwd.display()
                )
            })?
            .join(cwd),
        None => current_dir().map_err(|err| {
            format!(
                "failed to resolve requested workspace: no `cwd` was provided and launch current directory is unavailable: {err}"
            )
        })?,
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
        if base.join(".git").is_file() {
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
    ] {
        hash_path(&mut hasher, root, rel);
    }
    hash_head(&mut hasher, root);
    hasher.finish()
}

/// Hashes the HEAD that actually moves when the workspace is checked out.
///
/// In a plain checkout `.git` is a directory and HEAD sits inside it. In a
/// linked worktree `.git` is a file pointing at the per-worktree git directory,
/// so `root/.git/HEAD` does not exist and the pointer file itself never changes
/// on checkout. Hashing the literal `.git/HEAD` path therefore freezes the
/// epoch across branch switches inside a worktree and serves stale caches.
fn hash_head(hasher: &mut DefaultHasher, root: &Path) {
    "git-head".hash(hasher);
    let Some(git_dir) = resolve_git_dir(root) else {
        0_u8.hash(hasher);
        return;
    };
    // HEAD is small and its contents are the branch identity, so hash the bytes
    // instead of size plus second-resolution mtime.
    match std::fs::read(git_dir.join("HEAD")) {
        Ok(contents) => contents.hash(hasher),
        Err(_) => 0_u8.hash(hasher),
    }
}

fn resolve_git_dir(root: &Path) -> Option<PathBuf> {
    let dot_git = root.join(".git");
    let metadata = std::fs::symlink_metadata(&dot_git).ok()?;
    if metadata.is_dir() {
        return Some(dot_git);
    }
    let pointer = std::fs::read_to_string(&dot_git).ok()?;
    let target = pointer.lines().next()?.strip_prefix("gitdir:")?.trim();
    if target.is_empty() {
        return None;
    }
    let target = PathBuf::from(target);
    Some(if target.is_absolute() {
        target
    } else {
        root.join(target)
    })
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
    use super::{discover_workspace, discover_workspace_with_current_dir};
    use std::io;
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
    fn absolute_requested_cwd_does_not_read_process_cwd() {
        let root = temp_root("unica-workspace-absolute-cwd");
        std::fs::create_dir_all(&root).unwrap();
        assert!(root.is_absolute());

        let context = discover_workspace_with_current_dir(Some(root.clone()), || {
            panic!("absolute requested cwd must not read the process cwd")
        })
        .unwrap();

        assert_eq!(context.cwd, root);
        let _ = std::fs::remove_dir_all(context.cwd);
    }

    #[test]
    fn relative_requested_cwd_reports_launch_directory_failure() {
        let error =
            discover_workspace_with_current_dir(Some(PathBuf::from("relative-workspace")), || {
                Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "launch cwd missing",
                ))
            })
            .unwrap_err();

        assert!(
            error.starts_with(
                "failed to resolve relative requested workspace `relative-workspace`: launch current directory is unavailable:"
            ),
            "{error}"
        );
    }

    #[test]
    fn missing_requested_cwd_reports_launch_directory_failure() {
        let error = discover_workspace_with_current_dir(None, || {
            Err(io::Error::new(
                io::ErrorKind::NotFound,
                "launch cwd missing",
            ))
        })
        .unwrap_err();

        assert!(
            error.starts_with(
                "failed to resolve requested workspace: no `cwd` was provided and launch current directory is unavailable:"
            ),
            "{error}"
        );
    }

    #[test]
    fn relative_requested_cwd_resolves_from_launch_directory() {
        let root = temp_root("unica-workspace-relative-cwd");
        let workspace = root.join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();

        let context = discover_workspace_with_current_dir(Some(PathBuf::from("workspace")), || {
            Ok(root.clone())
        })
        .unwrap();

        assert_eq!(context.cwd, workspace);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn git_worktree_boundary_prevents_parent_workspace_discovery() {
        let root = temp_root("unica-workspace-worktree-boundary");
        let primary = root.join("primary");
        let worktree = primary.join("worktrees/feature");
        let nested = worktree.join("src/catalogs");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(primary.join("v8project.yaml"), "format: DESIGNER\n").unwrap();
        std::fs::write(
            worktree.join(".git"),
            "gitdir: ../../.git/worktrees/feature\n",
        )
        .unwrap();

        let context = discover_workspace(Some(nested)).unwrap();

        assert_eq!(context.workspace_root, worktree);
        assert_ne!(context.workspace_root, primary);

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

    #[test]
    fn workspace_epoch_tracks_head_inside_a_linked_worktree() {
        let root = temp_root("unica-workspace-worktree-epoch");
        let primary_git = root.join("primary/.git");
        let worktree = root.join("worktrees/feature");
        let linked_git = primary_git.join("worktrees/feature");
        std::fs::create_dir_all(&linked_git).unwrap();
        std::fs::create_dir_all(&worktree).unwrap();
        std::fs::write(worktree.join("v8project.yaml"), "format: DESIGNER\n").unwrap();
        std::fs::write(
            worktree.join(".git"),
            format!("gitdir: {}\n", linked_git.display()),
        )
        .unwrap();
        std::fs::write(linked_git.join("HEAD"), "ref: refs/heads/feature-a\n").unwrap();

        let before = discover_workspace(Some(worktree.clone())).unwrap();
        std::fs::write(linked_git.join("HEAD"), "ref: refs/heads/feature-b\n").unwrap();
        let after = discover_workspace(Some(worktree.clone())).unwrap();

        assert_ne!(
            before.workspace_epoch, after.workspace_epoch,
            "a checkout inside a linked worktree must invalidate workspace caches"
        );

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
