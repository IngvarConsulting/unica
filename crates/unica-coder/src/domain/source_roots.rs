use crate::domain::project_sources::{
    discover_project_source_map, ProjectSourceSet, SourceSetKind,
};
use crate::domain::workspace::WorkspaceContext;
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
    if let Some(raw) = explicit.filter(|value| !value.trim().is_empty()) {
        return resolve_explicit(context, raw);
    }
    let map = discover_project_source_map(&context.workspace_root)?;
    let selected = select_default_source_set(&map.source_sets)?;
    Ok(ResolvedSourceRoot {
        source_set: Some(selected.name.clone()),
        path: normalize_path_identity(&context.workspace_root.join(&selected.path))?,
    })
}

pub fn normalize_path_identity(path: &Path) -> Result<PathBuf, String> {
    Ok(normalize_lexically(path))
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
    if !path.starts_with(&workspace_root) {
        return Err(format!(
            "sourceDir must be inside workspace root {}: {}",
            workspace_root.display(),
            path.display()
        ));
    }

    let map = discover_project_source_map(&context.workspace_root)?;
    let source_set = map
        .source_sets
        .iter()
        .find(|source_set| {
            normalize_path_identity(&context.workspace_root.join(&source_set.path))
                .map(|configured_path| configured_path == path)
                .unwrap_or(false)
        })
        .map(|source_set| source_set.name.clone());
    Ok(ResolvedSourceRoot { source_set, path })
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
    use super::resolve_source_root;
    use crate::domain::workspace::WorkspaceContext;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn uses_explicit_source_dir_relative_to_cwd() {
        let context = fixture(&[("main", "CONFIGURATION", "src/cf")]);
        let selected = resolve_source_root(&context, Some("src/cf")).unwrap();

        assert_eq!(selected.source_set.as_deref(), Some("main"));
        assert_eq!(selected.path, context.workspace_root.join("src/cf"));
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
        assert_eq!(selected.path, context.workspace_root.join("src/cf"));
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
            context.workspace_root.join("extensions/main")
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
        assert_eq!(selected.path, context.workspace_root.join("app"));
        cleanup(&context);
    }

    #[test]
    fn rejects_ambiguous_configurations_without_main() {
        let context = fixture(&[
            ("app", "CONFIGURATION", "app"),
            ("tests", "CONFIGURATION", "tests"),
        ]);
        let error = resolve_source_root(&context, None).unwrap_err();

        assert!(error.contains("sourceDir"));
        assert!(error.contains("app"));
        assert!(error.contains("tests"));
        cleanup(&context);
    }

    #[test]
    fn rejects_explicit_source_dir_outside_the_workspace() {
        let context = fixture(&[("main", "CONFIGURATION", "src/cf")]);
        let error = resolve_source_root(&context, Some("../outside")).unwrap_err();

        assert!(error.contains("workspace"));
        cleanup(&context);
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
