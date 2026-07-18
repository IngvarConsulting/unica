use super::project_sources::discover_project_source_map;
use crate::domain::project_sources::{ProjectSourceSet, SourceSetKind};
use crate::domain::workspace::WorkspaceContext;
use std::fs;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedSourceRoot {
    pub source_set: Option<String>,
    pub path: PathBuf,
}

pub fn select_default_source_set(
    source_sets: &[ProjectSourceSet],
) -> Result<&ProjectSourceSet, String> {
    if let Some(main) = source_sets
        .iter()
        .find(|source_set| source_set.name == "main")
    {
        return Ok(main);
    }

    let configurations = source_sets
        .iter()
        .filter(|source_set| source_set.kind == SourceSetKind::Configuration)
        .collect::<Vec<_>>();

    match configurations.as_slice() {
        [source_set] => Ok(source_set),
        [] => {
            Err("sourceDir is required because no configuration source set was found".to_string())
        }
        _ => Err(format!(
            "sourceDir is required because configuration source sets are ambiguous: {}",
            configurations
                .iter()
                .map(|source_set| source_set.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

pub fn resolve_source_root(
    context: &WorkspaceContext,
    explicit: Option<&str>,
) -> Result<ResolvedSourceRoot, String> {
    let result = if let Some(raw) = explicit.filter(|value| !value.trim().is_empty()) {
        resolve_explicit(context, raw)
    } else {
        resolve_default(context)
    };
    result.map_err(invalid_source_root)
}

pub fn normalize_path_identity(path: &Path) -> Result<PathBuf, String> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|err| format!("failed to determine current directory: {err}"))?
            .join(path)
    };
    let prepared = strip_windows_extended_length_prefix(&absolute);
    let canonical = canonicalize_existing_ancestor(&prepared)?;
    Ok(strip_windows_extended_length_prefix(&canonical))
}

fn canonicalize_existing_ancestor(path: &Path) -> Result<PathBuf, String> {
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(_) => {
                let canonical = fs::canonicalize(ancestor).map_err(|err| {
                    format!(
                        "failed to resolve existing path ancestor {}: {err}",
                        ancestor.display()
                    )
                })?;
                let remainder = path.strip_prefix(ancestor).map_err(|err| {
                    format!(
                        "failed to preserve path suffix for {}: {err}",
                        path.display()
                    )
                })?;
                return Ok(normalize_lexically(&canonical.join(remainder)));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(format!(
                    "failed to inspect path ancestor {}: {error}",
                    ancestor.display()
                ));
            }
        }
    }

    Ok(normalize_lexically(path))
}

#[cfg(windows)]
fn strip_windows_extended_length_prefix(path: &Path) -> PathBuf {
    let path = path.as_os_str().to_string_lossy();
    if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{unc}"));
    }
    if let Some(regular) = path.strip_prefix(r"\\?\") {
        let bytes = regular.as_bytes();
        if bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && matches!(bytes[2], b'\\' | b'/')
        {
            return PathBuf::from(regular);
        }
    }
    PathBuf::from(path.as_ref())
}

#[cfg(not(windows))]
fn strip_windows_extended_length_prefix(path: &Path) -> PathBuf {
    path.to_path_buf()
}

fn resolve_default(context: &WorkspaceContext) -> Result<ResolvedSourceRoot, String> {
    let map = discover_project_source_map(&context.workspace_root)?;
    let selected = select_default_source_set(&map.source_sets)?;
    let path = normalize_contained_source_root(&context.workspace_root, &selected.path)?;
    Ok(ResolvedSourceRoot {
        source_set: Some(selected.name.clone()),
        path,
    })
}

pub(crate) fn normalize_contained_source_root(
    workspace_root: &Path,
    configured_path: impl AsRef<Path>,
) -> Result<PathBuf, String> {
    let workspace_root = normalize_path_identity(workspace_root)?;
    let configured_path = configured_path.as_ref();
    let candidate = if configured_path.is_absolute() {
        configured_path.to_path_buf()
    } else {
        workspace_root.join(configured_path)
    };
    let path = normalize_path_identity(&candidate)?;
    ensure_inside_workspace(&path, &workspace_root)?;
    Ok(path)
}

fn resolve_explicit(context: &WorkspaceContext, raw: &str) -> Result<ResolvedSourceRoot, String> {
    let raw = PathBuf::from(raw.trim());
    let path = if raw.is_absolute() {
        raw
    } else {
        context.cwd.join(raw)
    };
    let path = normalize_path_identity(&path)?;
    let workspace_root = normalize_path_identity(&context.workspace_root)?;
    ensure_inside_workspace(&path, &workspace_root)?;

    let map = discover_project_source_map(&context.workspace_root)?;
    let mut source_set = None;
    for configured in &map.source_sets {
        let configured_path =
            normalize_path_identity(&context.workspace_root.join(&configured.path))?;
        if configured_path == path {
            source_set = Some(configured.name.clone());
            break;
        }
    }
    Ok(ResolvedSourceRoot { source_set, path })
}

fn ensure_inside_workspace(path: &Path, workspace_root: &Path) -> Result<(), String> {
    if path.starts_with(workspace_root) {
        return Ok(());
    }
    Err(format!(
        "sourceDir must be inside workspace root {}: {}",
        workspace_root.display(),
        path.display()
    ))
}

fn invalid_source_root(error: String) -> String {
    format!("invalid_source_root: {error}")
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
    use super::{normalize_path_identity, resolve_source_root};
    use crate::domain::workspace::WorkspaceContext;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn uses_explicit_source_dir_relative_to_cwd() {
        let context = fixture(&[("main", "CONFIGURATION", "src/cf")]);
        let selected = resolve_source_root(&context, Some("src/cf")).unwrap();

        assert_eq!(selected.source_set.as_deref(), Some("main"));
        assert_eq!(
            selected.path,
            normalize_path_identity(&context.workspace_root.join("src/cf")).unwrap()
        );
        cleanup(&context);
    }

    #[test]
    fn selects_main_before_other_configurations() {
        let context = fixture(&[
            ("main", "CONFIGURATION", "src/cf"),
            ("TESTS", "CONFIGURATION", "exts/TESTS"),
        ]);
        let selected = resolve_source_root(&context, None).unwrap();

        assert_eq!(selected.source_set.as_deref(), Some("main"));
        assert_eq!(
            selected.path,
            normalize_path_identity(&context.workspace_root.join("src/cf")).unwrap()
        );
        cleanup(&context);
    }

    #[test]
    fn selects_main_regardless_of_source_set_kind() {
        let context = fixture(&[
            ("main", "EXTENSION", "extensions/main"),
            ("app", "CONFIGURATION", "app"),
        ]);
        let selected = resolve_source_root(&context, None).unwrap();

        assert_eq!(selected.source_set.as_deref(), Some("main"));
        assert_eq!(
            selected.path,
            normalize_path_identity(&context.workspace_root.join("extensions/main")).unwrap()
        );
        cleanup(&context);
    }

    #[test]
    fn selects_the_sole_configuration() {
        let context = fixture(&[
            ("app", "CONFIGURATION", "app"),
            ("extension", "EXTENSION", "ext"),
        ]);
        let selected = resolve_source_root(&context, None).unwrap();

        assert_eq!(selected.source_set.as_deref(), Some("app"));
        assert_eq!(
            selected.path,
            normalize_path_identity(&context.workspace_root.join("app")).unwrap()
        );
        cleanup(&context);
    }

    #[test]
    fn rejects_ambiguous_configurations_without_main() {
        let context = fixture(&[
            ("app", "CONFIGURATION", "app"),
            ("tests", "CONFIGURATION", "tests"),
        ]);
        let error = resolve_source_root(&context, None).unwrap_err();

        assert!(error.starts_with("invalid_source_root:"));
        assert!(error.contains("sourceDir"));
        assert!(error.contains("app"));
        assert!(error.contains("tests"));
        cleanup(&context);
    }

    #[test]
    fn rejects_explicit_source_dir_outside_the_workspace() {
        let context = fixture(&[("main", "CONFIGURATION", "src/cf")]);
        let error = resolve_source_root(&context, Some("../outside")).unwrap_err();

        assert!(error.starts_with("invalid_source_root:"));
        assert!(error.contains("workspace"));
        cleanup(&context);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_nonexistent_source_dir_through_symlink_outside_workspace() {
        use std::os::unix::fs::symlink;

        let context = fixture(&[("main", "CONFIGURATION", "src/cf")]);
        let outside = temp_workspace("unica-source-roots-outside");
        symlink(&outside, context.workspace_root.join("external")).unwrap();

        let escaped = fs::canonicalize(&context.workspace_root)
            .unwrap()
            .join("external/new-source");
        let error = resolve_source_root(&context, escaped.to_str()).unwrap_err();

        assert!(error.starts_with("invalid_source_root:"));
        assert!(error.contains("workspace"));
        cleanup(&context);
        let _ = fs::remove_dir_all(outside);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_parent_traversal_after_symlink_outside_workspace() {
        use std::os::unix::fs::symlink;

        let context = fixture(&[("main", "CONFIGURATION", "src/cf")]);
        let outside = temp_workspace("unica-source-roots-parent-outside");
        symlink(&outside, context.workspace_root.join("external")).unwrap();
        let escaped = fs::canonicalize(&context.workspace_root)
            .unwrap()
            .join("external/../escaped-new");

        let error = resolve_source_root(&context, escaped.to_str()).unwrap_err();

        assert!(error.starts_with("invalid_source_root:"));
        assert!(error.contains("workspace"));
        cleanup(&context);
        let _ = fs::remove_dir_all(outside);
    }

    #[test]
    fn nonexistent_path_uses_canonical_identity_of_existing_parent() {
        let root = temp_workspace("unica-source-roots-nonexistent");
        let expected = normalize_path_identity(&root).unwrap().join("new/source");

        let actual = normalize_path_identity(&root.join("new/source")).unwrap();

        assert_eq!(actual, expected);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_main_source_set_configured_outside_the_workspace() {
        let context = fixture(&[("main", "CONFIGURATION", "../outside")]);
        let error = resolve_source_root(&context, None).unwrap_err();

        assert!(error.starts_with("invalid_source_root:"));
        assert!(error.contains("path_traversal"));
        cleanup(&context);
    }

    #[test]
    fn rejects_sole_configuration_configured_outside_the_workspace() {
        let context = fixture(&[("app", "CONFIGURATION", "../outside")]);
        let error = resolve_source_root(&context, None).unwrap_err();

        assert!(error.starts_with("invalid_source_root:"));
        assert!(error.contains("path_traversal"));
        cleanup(&context);
    }

    #[test]
    fn prefixes_project_discovery_errors() {
        let context = fixture(&[("main", "UNKNOWN", "src")]);
        let error = resolve_source_root(&context, None).unwrap_err();

        assert!(error.starts_with("invalid_source_root:"));
        cleanup(&context);
    }

    #[cfg(windows)]
    #[test]
    fn extended_length_and_regular_paths_have_same_identity() {
        let root = temp_workspace("path-identity");
        let regular = normalize_path_identity(&root).unwrap();
        let extended = PathBuf::from(format!(r"\\?\{}", root.display()));

        assert_eq!(regular, normalize_path_identity(&extended).unwrap());

        let _ = fs::remove_dir_all(root);
    }

    #[cfg(windows)]
    #[test]
    fn extended_length_unc_paths_use_regular_unc_identity() {
        let extended = PathBuf::from(r"\\?\UNC\server\share\source");

        assert_eq!(
            PathBuf::from(r"\\server\share\source"),
            normalize_path_identity(&extended).unwrap()
        );
    }

    #[cfg(windows)]
    #[test]
    fn preserves_non_drive_verbatim_path_namespaces() {
        let verbatim = PathBuf::from(r"\\?\Volume{01234567-89ab-cdef-0123-456789abcdef}\source");

        assert_eq!(verbatim, normalize_path_identity(&verbatim).unwrap());
    }

    fn fixture(source_sets: &[(&str, &str, &str)]) -> WorkspaceContext {
        let root = temp_workspace("unica-source-roots");
        let yaml = source_sets
            .iter()
            .map(|(name, kind, path)| {
                format!("  - name: {name}\n    type: {kind}\n    path: {path}")
            })
            .collect::<Vec<_>>()
            .join("\n");
        write(
            &root.join("v8project.yaml"),
            &format!("source-set:\n{yaml}\n"),
        );
        for (_, _, path) in source_sets {
            fs::create_dir_all(root.join(path)).unwrap();
        }
        fs::create_dir_all(root.join("outside")).unwrap();
        WorkspaceContext::discover(root).unwrap()
    }

    fn temp_workspace(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("{prefix}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn write(path: &Path, text: &str) {
        fs::write(path, text).unwrap();
    }

    fn cleanup(context: &WorkspaceContext) {
        let _ = fs::remove_dir_all(&context.workspace_root);
    }
}
