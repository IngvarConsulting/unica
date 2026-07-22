use crate::domain::cancellation::CancellationToken;
use crate::domain::discovery::DiscoveryError;
use crate::domain::project_sources::SourceSetKind;
use crate::domain::source_roots::{select_default_source_set, ResolvedSourceRoot};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::platform::filesystem::strip_windows_extended_length_prefix;
use crate::infrastructure::platform::verified_directory::{
    read_verified_contained_directory_bounded_cancellable,
    read_verified_contained_directory_with_expected_identity_bounded_cancellable,
    VerifiedDirectoryEntry, VerifiedDirectoryEntryKind, VerifiedDirectoryError,
};
use crate::infrastructure::project_sources::{
    discover_project_source_declarations_cancellable, discover_project_source_map,
};
use std::fs;
use std::path::{Component, Path, PathBuf};

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

pub(crate) fn resolve_discovery_source_root(
    context: &WorkspaceContext,
    explicit: Option<&Path>,
    max_files: u32,
    cancellation: &CancellationToken,
) -> Result<ResolvedSourceRoot, DiscoveryError> {
    if cancellation.is_cancelled() {
        return Err(DiscoveryError::Cancelled);
    }
    let selected_result = (|| -> Result<ResolvedSourceRoot, DiscoveryError> {
        match explicit {
            Some(path) => {
                validate_discovery_relative_path(path)?;
                let resolved = normalize_contained_source_root(&context.workspace_root, path)
                    .map_err(DiscoveryError::InvalidSourceRoot)?;
                Ok(ResolvedSourceRoot {
                    source_set: None,
                    path: resolved,
                })
            }
            None => {
                let declarations = discover_project_source_declarations_cancellable(
                    &context.workspace_root,
                    cancellation,
                )?;
                let configurations = declarations
                    .iter()
                    .filter(|source_set| source_set.kind == SourceSetKind::Configuration)
                    .collect::<Vec<_>>();
                let source_set = match configurations.as_slice() {
                    [source_set] => *source_set,
                    [] => return Err(DiscoveryError::NoConfigurationSource),
                    multiple => {
                        let mut candidates = multiple
                            .iter()
                            .map(|source_set| source_set.name.clone())
                            .collect::<Vec<_>>();
                        candidates.sort();
                        return Err(DiscoveryError::AmbiguousConfigurationSources(candidates));
                    }
                };
                if !source_set.discovery_path_is_safe {
                    return Err(DiscoveryError::InvalidSourceRoot(
                    "configured source path must be workspace-relative and must not contain parent components"
                        .to_string(),
                ));
                }
                validate_discovery_relative_path(Path::new(&source_set.path))?;
                let path = normalize_contained_source_root(
                    &context.workspace_root,
                    Path::new(&source_set.path),
                )
                .map_err(DiscoveryError::InvalidSourceRoot)?;
                Ok(ResolvedSourceRoot {
                    source_set: Some(source_set.name.clone()),
                    path,
                })
            }
        }
    })();
    let selected = prefer_discovery_cancellation(selected_result, cancellation)?;
    let format = classify_discovery_source_format_observing(&selected.path, max_files, || {
        cancellation.is_cancelled()
    });
    match prefer_discovery_cancellation(format, cancellation)? {
        DiscoverySelectedFormat::PlatformXml => Ok(selected),
        DiscoverySelectedFormat::Edt => {
            Err(DiscoveryError::UnsupportedSourceFormat("edt".to_string()))
        }
        DiscoverySelectedFormat::Conflict => {
            Err(DiscoveryError::InvalidSourceFormat("conflict".to_string()))
        }
        DiscoverySelectedFormat::Unknown => {
            Err(DiscoveryError::InvalidSourceFormat("unknown".to_string()))
        }
    }
}

fn prefer_discovery_cancellation<T>(
    result: Result<T, DiscoveryError>,
    cancellation: &CancellationToken,
) -> Result<T, DiscoveryError> {
    if cancellation.is_cancelled() {
        Err(DiscoveryError::Cancelled)
    } else {
        result
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiscoverySelectedFormat {
    PlatformXml,
    Edt,
    Conflict,
    Unknown,
}

fn classify_discovery_source_format_observing(
    source_root: &Path,
    max_files: u32,
    mut is_cancelled: impl FnMut() -> bool,
) -> Result<DiscoverySelectedFormat, DiscoveryError> {
    let mut budget = MarkerScanBudget {
        limit: max_files,
        observed: 0,
    };
    let root_entries = read_marker_directory(
        source_root,
        source_root,
        None,
        &mut budget,
        &mut is_cancelled,
    )?;
    let platform = marker_file(&root_entries, "Configuration.xml")?;
    let mut edt = marker_file(&root_entries, ".project")?;

    if let Some(entry) = marker_directory(&root_entries, "DT-INF")? {
        let entries = read_marker_directory(
            source_root,
            &entry.path,
            Some(entry.identity),
            &mut budget,
            &mut is_cancelled,
        )?;
        edt |= marker_file(&entries, "PROJECT.PMF")?;
    }
    if let Some(entry) = marker_directory(&root_entries, "Configuration")? {
        let entries = read_marker_directory(
            source_root,
            &entry.path,
            Some(entry.identity),
            &mut budget,
            &mut is_cancelled,
        )?;
        edt |= marker_file(&entries, "Configuration.mdo")?;
    }
    if let Some(src) = marker_directory(&root_entries, "src")? {
        let src_entries = read_marker_directory(
            source_root,
            &src.path,
            Some(src.identity),
            &mut budget,
            &mut is_cancelled,
        )?;
        if let Some(configuration) = marker_directory(&src_entries, "Configuration")? {
            let entries = read_marker_directory(
                source_root,
                &configuration.path,
                Some(configuration.identity),
                &mut budget,
                &mut is_cancelled,
            )?;
            edt |= marker_file(&entries, "Configuration.mdo")?;
        }
    }
    Ok(match (platform, edt) {
        (true, false) => DiscoverySelectedFormat::PlatformXml,
        (false, true) => DiscoverySelectedFormat::Edt,
        (true, true) => DiscoverySelectedFormat::Conflict,
        (false, false) => DiscoverySelectedFormat::Unknown,
    })
}

struct MarkerScanBudget {
    limit: u32,
    observed: u32,
}

fn read_marker_directory(
    source_root: &Path,
    directory: &Path,
    expected_identity: Option<crate::infrastructure::platform::contained_file::VerifiedIdentity>,
    budget: &mut MarkerScanBudget,
    is_cancelled: &mut dyn FnMut() -> bool,
) -> Result<Vec<VerifiedDirectoryEntry>, DiscoveryError> {
    if is_cancelled() {
        return Err(DiscoveryError::Cancelled);
    }
    let remaining =
        usize::try_from(budget.limit.saturating_sub(budget.observed)).unwrap_or(usize::MAX);
    let root_snapshot = expected_identity.is_none();
    let result = match expected_identity {
        Some(identity) => {
            read_verified_contained_directory_with_expected_identity_bounded_cancellable(
                source_root,
                directory,
                identity,
                remaining,
                &mut *is_cancelled,
            )
        }
        None => read_verified_contained_directory_bounded_cancellable(
            source_root,
            directory,
            remaining,
            &mut *is_cancelled,
        ),
    };
    if matches!(&result, Err(VerifiedDirectoryError::Cancelled)) {
        return Err(DiscoveryError::Cancelled);
    }
    if is_cancelled() {
        return Err(DiscoveryError::Cancelled);
    }
    let entries =
        result.map_err(|error| map_marker_directory_error(error, budget.limit, root_snapshot))?;
    budget.observed = budget
        .observed
        .saturating_add(u32::try_from(entries.len()).unwrap_or(u32::MAX));
    Ok(entries)
}

fn map_marker_directory_error(
    error: VerifiedDirectoryError,
    limit: u32,
    root_snapshot: bool,
) -> DiscoveryError {
    let root_establishment_error = match &error {
        VerifiedDirectoryError::RootNotCanonical
        | VerifiedDirectoryError::RootNotDirectory
        | VerifiedDirectoryError::NotDirectory => true,
        VerifiedDirectoryError::Io { operation, .. } => {
            matches!(*operation, "resolve source root" | "inspect source root")
        }
        _ => false,
    };
    if root_snapshot && root_establishment_error {
        return DiscoveryError::InvalidSourceRoot(format!(
            "could not establish selected source root: {error}"
        ));
    }
    match error {
        VerifiedDirectoryError::Cancelled => DiscoveryError::Cancelled,
        VerifiedDirectoryError::EntryLimitExceeded { .. } => {
            DiscoveryError::SourceFormatBound { limit }
        }
        error => DiscoveryError::InvalidSourceFormat(format!(
            "unsafe selected source marker boundary: {error}"
        )),
    }
}

fn marker_file(entries: &[VerifiedDirectoryEntry], name: &str) -> Result<bool, DiscoveryError> {
    match marker_entry(entries, name) {
        Some(entry) if entry.kind == VerifiedDirectoryEntryKind::RegularFile => Ok(true),
        Some(_) => Err(DiscoveryError::InvalidSourceFormat(format!(
            "selected source marker has an invalid filesystem kind: {name}"
        ))),
        None => Ok(false),
    }
}

fn marker_directory<'a>(
    entries: &'a [VerifiedDirectoryEntry],
    name: &str,
) -> Result<Option<&'a VerifiedDirectoryEntry>, DiscoveryError> {
    match marker_entry(entries, name) {
        Some(entry) if entry.kind == VerifiedDirectoryEntryKind::Directory => Ok(Some(entry)),
        Some(_) => Err(DiscoveryError::InvalidSourceFormat(format!(
            "selected source marker has an invalid filesystem kind: {name}"
        ))),
        None => Ok(None),
    }
}

fn marker_entry<'a>(
    entries: &'a [VerifiedDirectoryEntry],
    name: &str,
) -> Option<&'a VerifiedDirectoryEntry> {
    entries.iter().find(|entry| {
        entry
            .path
            .file_name()
            .and_then(|entry_name| entry_name.to_str())
            .is_some_and(|entry_name| entry_name.eq_ignore_ascii_case(name))
    })
}

fn validate_discovery_relative_path(path: &Path) -> Result<(), DiscoveryError> {
    if path.is_absolute() {
        return Err(DiscoveryError::InvalidSourceRoot(
            "sourceDir must be relative to the workspace root".to_string(),
        ));
    }
    if path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err(DiscoveryError::InvalidSourceRoot(
            "sourceDir must not contain parent or root path components".to_string(),
        ));
    }
    Ok(())
}

pub(crate) fn normalize_path_identity(path: &Path) -> Result<PathBuf, String> {
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
    use super::{
        classify_discovery_source_format_observing, normalize_path_identity,
        resolve_discovery_source_root, resolve_source_root,
    };
    use crate::domain::cancellation::CancellationToken;
    use crate::domain::workspace::WorkspaceContext;
    use crate::infrastructure::platform::testing::{
        create_file_link_fixture_for_test, FileLinkFixtureOutcome,
    };
    use crate::infrastructure::workspace::discover_workspace;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEMP_WORKSPACE_NONCE: AtomicU64 = AtomicU64::new(0);

    fn resolve_discovery(
        context: &WorkspaceContext,
        explicit: Option<&Path>,
    ) -> Result<
        crate::domain::source_roots::ResolvedSourceRoot,
        crate::domain::discovery::DiscoveryError,
    > {
        resolve_discovery_source_root(context, explicit, 20_000, &CancellationToken::new())
    }

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
    fn discovery_selects_the_only_configuration_instead_of_main_extension() {
        let context = fixture(&[
            ("main", "EXTENSION", "extensions/main"),
            ("app", "CONFIGURATION", "src/configuration"),
        ]);

        let selected = resolve_discovery(&context, None).unwrap();

        assert_eq!(selected.source_set.as_deref(), Some("app"));
        assert_eq!(
            selected.path,
            normalize_path_identity(&context.workspace_root.join("src/configuration")).unwrap()
        );
        cleanup(&context);
    }

    #[test]
    fn discovery_rejects_zero_or_multiple_configuration_roots_with_typed_errors() {
        let no_configuration = fixture(&[("main", "EXTENSION", "extensions/main")]);
        assert_eq!(
            resolve_discovery(&no_configuration, None).unwrap_err(),
            crate::domain::discovery::DiscoveryError::NoConfigurationSource
        );
        cleanup(&no_configuration);

        let ambiguous = fixture(&[
            ("tests", "CONFIGURATION", "tests"),
            ("app", "CONFIGURATION", "app"),
        ]);
        assert_eq!(
            resolve_discovery(&ambiguous, None).unwrap_err(),
            crate::domain::discovery::DiscoveryError::AmbiguousConfigurationSources(vec![
                "app".to_string(),
                "tests".to_string(),
            ])
        );
        cleanup(&ambiguous);
    }

    #[test]
    fn discovery_explicit_source_dir_is_workspace_relative_and_contained() {
        let mut context = fixture(&[("main", "CONFIGURATION", "src/configuration")]);
        context.cwd = context.workspace_root.join("nested/working/directory");

        let selected = resolve_discovery(&context, Some(Path::new("src/configuration"))).unwrap();

        assert_eq!(
            selected.path,
            normalize_path_identity(&context.workspace_root.join("src/configuration")).unwrap()
        );
        assert!(matches!(
            resolve_discovery(&context, Some(Path::new("../outside"))),
            Err(crate::domain::discovery::DiscoveryError::InvalidSourceRoot(
                _
            ))
        ));
        cleanup(&context);
    }

    #[test]
    fn explicit_discovery_ignores_malformed_unrelated_project_declarations() {
        let context = fixture(&[("main", "CONFIGURATION", "src")]);
        write(
            &context.workspace_root.join("v8project.yaml"),
            "source-set: [",
        );

        let selected = resolve_discovery(&context, Some(Path::new("src")))
            .expect("explicit contained source must not depend on unrelated declarations");

        assert_eq!(selected.source_set, None);
        assert_eq!(
            selected.path,
            normalize_path_identity(&context.workspace_root.join("src")).unwrap()
        );
        cleanup(&context);
    }

    #[test]
    fn missing_explicit_source_dir_is_a_source_root_error_not_a_format_error() {
        let context = fixture(&[("main", "CONFIGURATION", "src")]);

        let error = resolve_discovery(&context, Some(Path::new("missing")))
            .expect_err("a missing explicit source root must fail selection");

        assert!(matches!(
            error,
            crate::domain::discovery::DiscoveryError::InvalidSourceRoot(_)
        ));
        cleanup(&context);
    }

    #[test]
    fn missing_configured_source_dir_is_a_source_root_error_not_a_format_error() {
        let context = fixture(&[("main", "CONFIGURATION", "src")]);
        fs::remove_dir_all(context.workspace_root.join("src")).expect("remove configured root");

        let error = resolve_discovery(&context, None)
            .expect_err("a missing configured source root must fail selection");

        assert!(matches!(
            error,
            crate::domain::discovery::DiscoveryError::InvalidSourceRoot(_)
        ));
        cleanup(&context);
    }

    #[test]
    fn discovery_format_snapshot_cancels_inside_a_large_flat_root() {
        let context = fixture(&[("main", "CONFIGURATION", "src")]);
        for index in 0..256 {
            write(
                &context
                    .workspace_root
                    .join(format!("src/ignored-{index:03}.txt")),
                "ignored",
            );
        }
        let token = CancellationToken::new();
        let mut entries_seen = 0_u16;

        let source_root = normalize_path_identity(&context.workspace_root.join("src")).unwrap();
        let result = classify_discovery_source_format_observing(&source_root, 512, || {
            entries_seen += 1;
            if entries_seen == 32 {
                token.cancel();
            }
            token.is_cancelled()
        });

        assert_eq!(
            result,
            Err(crate::domain::discovery::DiscoveryError::Cancelled)
        );
        assert_eq!(entries_seen, 32);
        cleanup(&context);
    }

    #[test]
    fn discovery_format_snapshot_stops_at_the_request_file_bound() {
        let context = fixture(&[("main", "CONFIGURATION", "src")]);
        write(&context.workspace_root.join("src/ignored-a.txt"), "ignored");
        write(&context.workspace_root.join("src/ignored-b.txt"), "ignored");

        let source_root = normalize_path_identity(&context.workspace_root.join("src")).unwrap();
        let error = classify_discovery_source_format_observing(&source_root, 2, || false)
            .expect_err("marker snapshot must honor maxFiles");

        assert_eq!(
            error,
            crate::domain::discovery::DiscoveryError::SourceFormatBound { limit: 2 }
        );
        cleanup(&context);
    }

    #[test]
    fn discovery_cancellation_wins_over_a_resolver_error() {
        let token = CancellationToken::new();
        token.cancel();

        let result = super::prefer_discovery_cancellation::<()>(
            Err(
                crate::domain::discovery::DiscoveryError::InvalidSourceFormat(
                    "unsafe marker".to_string(),
                ),
            ),
            &token,
        );

        assert_eq!(
            result,
            Err(crate::domain::discovery::DiscoveryError::Cancelled)
        );
    }

    #[test]
    fn discovery_rejects_edt_for_implicit_and_explicit_selection() {
        let context = fixture(&[("app", "CONFIGURATION", "src")]);
        fs::remove_file(context.workspace_root.join("src/Configuration.xml")).unwrap();
        write(
            &context.workspace_root.join("src/.project"),
            "<projectDescription/>",
        );
        fs::create_dir_all(context.workspace_root.join("src/Configuration")).unwrap();
        write(
            &context
                .workspace_root
                .join("src/Configuration/Configuration.mdo"),
            "<mdclass:Configuration/>",
        );

        for explicit in [None, Some(Path::new("src"))] {
            assert_eq!(
                resolve_discovery(&context, explicit).unwrap_err(),
                crate::domain::discovery::DiscoveryError::UnsupportedSourceFormat(
                    "edt".to_string()
                )
            );
        }
        cleanup(&context);
    }

    #[test]
    fn discovery_fails_closed_for_unknown_or_conflicting_selected_format() {
        let unknown = fixture(&[("app", "CONFIGURATION", "src")]);
        fs::remove_file(unknown.workspace_root.join("src/Configuration.xml")).unwrap();
        assert_eq!(
            resolve_discovery(&unknown, None).unwrap_err(),
            crate::domain::discovery::DiscoveryError::InvalidSourceFormat("unknown".to_string())
        );
        cleanup(&unknown);

        let conflict = fixture(&[("app", "CONFIGURATION", "src")]);
        write(
            &conflict.workspace_root.join("src/.project"),
            "<projectDescription/>",
        );
        assert_eq!(
            resolve_discovery(&conflict, None).unwrap_err(),
            crate::domain::discovery::DiscoveryError::InvalidSourceFormat("conflict".to_string())
        );
        cleanup(&conflict);
    }

    #[test]
    fn implicit_discovery_rejects_absolute_or_parented_declaration_paths_lexically() {
        let root = temp_workspace("unica-discovery-lexical-source-paths");
        fs::create_dir_all(root.join("src")).unwrap();
        write(&root.join("src/Configuration.xml"), "<MetaDataObject/>");
        write(
            &root.join("v8project.yaml"),
            &format!(
                "source-set:\n  - name: app\n    type: CONFIGURATION\n    path: {}\n",
                root.join("src").display()
            ),
        );
        let absolute = discover_workspace(Some(root.clone())).unwrap();
        assert!(matches!(
            resolve_discovery(&absolute, None),
            Err(crate::domain::discovery::DiscoveryError::InvalidSourceRoot(
                _
            ))
        ));

        write(
            &root.join("v8project.yaml"),
            "source-set:\n  - name: app\n    type: CONFIGURATION\n    path: src/../src\n",
        );
        let parented = discover_workspace(Some(root)).unwrap();
        assert!(matches!(
            resolve_discovery(&parented, None),
            Err(crate::domain::discovery::DiscoveryError::InvalidSourceRoot(
                _
            ))
        ));
        cleanup(&parented);
    }

    #[test]
    fn explicit_discovery_does_not_probe_an_unrelated_escaping_config_dump_link() {
        let context = fixture(&[
            ("app", "CONFIGURATION", "src"),
            ("external", "EXTERNAL_DATA_PROCESSORS", "epf"),
        ]);
        let outside = temp_workspace("unica-unrelated-config-dump-outside");
        write(&outside.join("ConfigDumpInfo.xml"), "<ConfigDumpInfo/>");
        let link = context.workspace_root.join("epf/ConfigDumpInfo.xml");
        match create_file_link_fixture_for_test(outside.join("ConfigDumpInfo.xml"), &link)
            .expect("escaping link fixture")
        {
            FileLinkFixtureOutcome::Created => {}
            FileLinkFixtureOutcome::Unsupported
            | FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => {
                let _ = fs::remove_dir_all(outside);
                cleanup(&context);
                return;
            }
        }

        let selected = resolve_discovery(&context, Some(Path::new("src"))).unwrap();

        assert_eq!(selected.source_set, None);
        assert_eq!(
            selected.path,
            normalize_path_identity(&context.workspace_root.join("src")).unwrap()
        );
        let _ = fs::remove_dir_all(outside);
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
        let Some(symlink_result) =
            crate::infrastructure::platform::filesystem::create_dir_symlink_for_test(
                &outside,
                context.workspace_root.join("external"),
            )
        else {
            cleanup(&context);
            let _ = fs::remove_dir_all(outside);
            return;
        };
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
        for (_, kind, path) in source_sets {
            if *kind == "CONFIGURATION" {
                write(
                    &root.join(path).join("Configuration.xml"),
                    "<MetaDataObject/>",
                );
            }
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
