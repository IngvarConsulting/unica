use crate::infrastructure::project_sources::discover_project_source_map;

use std::path::PathBuf;
use unica_source_contract::discovery::{NamedSourceSetError, NamedSourceSetErrorKind};
use unica_source_contract::workspace::ResolvedSourceRoot;
use unica_source_contract::workspace::WorkspaceContext;
use unica_source_contract::ProjectSourceSet;
use unica_source_designer::selection::select_default_source_set;
use unica_workspace::discovery::{
    ensure_inside_workspace, inspect_declared_source_root_route, lexical_contained_source_root,
    normalize_contained_source_root, reject_linked_source_root_route,
};
use unica_workspace::paths::normalize_path_identity;

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct ResolvedNamedSourceSet {
    pub(crate) source_set: ProjectSourceSet,
    pub(crate) lexical_path: PathBuf,
    pub(crate) path: PathBuf,
}

pub(crate) fn resolve_source_root(
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

#[cfg(test)]
thread_local! {
    /// Counts how many times a call re-reads the project config. A scan that
    /// resolves the source set once per candidate instead of once per call is
    /// what made a bare-kind prefix query cost seconds (#277).
    pub(crate) static NAMED_SOURCE_SET_RESOLUTIONS: std::cell::Cell<usize> =
        const { std::cell::Cell::new(0) };
}

pub(crate) fn resolve_named_source_set(
    context: &WorkspaceContext,
    name: &str,
) -> Result<ResolvedNamedSourceSet, NamedSourceSetError> {
    #[cfg(test)]
    NAMED_SOURCE_SET_RESOLUTIONS.with(|count| count.set(count.get() + 1));
    if name.is_empty() {
        return Err(NamedSourceSetError::new(
            NamedSourceSetErrorKind::NotFound,
            "source set name must not be empty",
        ));
    }
    let map = discover_project_source_map(&context.workspace_root)
        .map_err(|error| NamedSourceSetError::new(NamedSourceSetErrorKind::Discovery, error))?;
    let mut matches = map
        .source_sets
        .into_iter()
        .filter(|source_set| source_set.name == name)
        .collect::<Vec<_>>();
    if matches.len() > 1 {
        return Err(NamedSourceSetError::new(
            NamedSourceSetErrorKind::Ambiguous,
            format!(
                "project source set name `{name}` is ambiguous across {} exact entries",
                matches.len()
            ),
        ));
    }
    let source_set = matches.pop().ok_or_else(|| {
        NamedSourceSetError::new(
            NamedSourceSetErrorKind::NotFound,
            format!("project source set `{name}` was not found"),
        )
    })?;
    let containment =
        |detail: String| NamedSourceSetError::new(NamedSourceSetErrorKind::Containment, detail);
    let lexical_path = lexical_contained_source_root(&context.workspace_root, &source_set.path)
        .map_err(containment)?;
    reject_linked_source_root_route(&context.workspace_root, &lexical_path)?;
    let route = inspect_declared_source_root_route(&context.workspace_root, &source_set.path)?;
    Ok(ResolvedNamedSourceSet {
        source_set,
        lexical_path: route.lexical_path,
        path: route.identity_path,
    })
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

/// Stable prefix every source-root rejection carries. Consumers that classify a
/// readiness failure as caller-fixable match on this instead of a literal.
pub(crate) const INVALID_SOURCE_ROOT_PREFIX: &str = "invalid_source_root:";

fn invalid_source_root(error: String) -> String {
    format!("{INVALID_SOURCE_ROOT_PREFIX} {error}")
}

#[cfg(test)]
mod tests {
    use super::{normalize_path_identity, resolve_named_source_set, resolve_source_root};
    use crate::infrastructure::workspace::discover_workspace;
    #[cfg(test)]
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    use unica_source_contract::workspace::WorkspaceContext;

    static TEMP_WORKSPACE_NONCE: AtomicU64 = AtomicU64::new(0);

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

    #[test]
    fn rejects_nonexistent_source_dir_through_symlink_outside_workspace() {
        let context = fixture(&[("main", "CONFIGURATION", "src/cf")]);
        let outside = temp_workspace("unica-source-roots-outside");
        let symlink_result = unica_workspace::test_support::create_dir_symlink_for_test(
            &outside,
            context.workspace_root.join("external"),
        )
        .expect("symlink fixtures exist on Unix and Windows");
        symlink_result.unwrap();

        let escaped = fs::canonicalize(&context.workspace_root)
            .unwrap()
            .join("external/new-source");
        let error = resolve_source_root(&context, escaped.to_str()).unwrap_err();

        assert!(error.starts_with("invalid_source_root:"));
        assert!(error.contains("workspace"));
        cleanup(&context);
        let _ = fs::remove_dir_all(outside);
    }

    #[test]
    fn declared_source_root_reports_link_before_normalized_containment() {
        let context = fixture(&[("main", "CONFIGURATION", "src/cf")]);
        fs::write(
            context.workspace_root.join("v8project.yaml"),
            "source-set:\n  - name: main\n    type: CONFIGURATION\n    path: external/new-source\n",
        )
        .unwrap();
        let outside = temp_workspace("unica-declared-source-root-outside");
        let symlink_result = unica_workspace::test_support::create_dir_symlink_for_test(
            &outside,
            context.workspace_root.join("external"),
        )
        .expect("symlink fixtures exist on Unix and Windows");
        symlink_result.unwrap();

        let error = resolve_named_source_set(&context, "main").unwrap_err();

        assert!(
            error.to_string().contains("symbolic link")
                || error.to_string().contains("reparse point"),
            "{error}"
        );
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
        assert!(error.contains("workspace"));
        cleanup(&context);
    }

    #[test]
    fn rejects_sole_configuration_configured_outside_the_workspace() {
        let context = fixture(&[("app", "CONFIGURATION", "../outside")]);
        let error = resolve_source_root(&context, None).unwrap_err();

        assert!(error.starts_with("invalid_source_root:"));
        assert!(error.contains("workspace"));
        cleanup(&context);
    }

    #[test]
    fn prefixes_project_discovery_errors() {
        let context = fixture(&[("main", "UNKNOWN", "src")]);
        let error = resolve_source_root(&context, None).unwrap_err();

        assert!(error.starts_with("invalid_source_root:"));
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
        discover_workspace(Some(root)).unwrap()
    }

    fn temp_workspace(prefix: &str) -> PathBuf {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let nonce = TEMP_WORKSPACE_NONCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "{prefix}-{}-{timestamp}-{nonce}",
            std::process::id()
        ));
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
