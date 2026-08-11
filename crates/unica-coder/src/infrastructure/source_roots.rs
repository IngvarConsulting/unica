use crate::domain::project_sources::ProjectSourceSet;
use crate::domain::source_roots::{select_default_source_set, ResolvedSourceRoot};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::platform::filesystem::strip_windows_extended_length_prefix;
use crate::infrastructure::project_sources::discover_project_source_map;
use std::collections::hash_map::DefaultHasher;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs;
use std::hash::{Hash, Hasher};
use std::num::NonZeroUsize;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct ResolvedNamedSourceSet {
    pub(crate) source_set: ProjectSourceSet,
    pub(crate) lexical_path: PathBuf,
    pub(crate) path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NamedSourceSetErrorKind {
    NotFound,
    Ambiguous,
    Containment,
    Discovery,
}

#[derive(Debug)]
pub(crate) struct NamedSourceSetError {
    pub(crate) kind: NamedSourceSetErrorKind,
    detail: String,
}

impl NamedSourceSetError {
    fn new(kind: NamedSourceSetErrorKind, detail: impl Into<String>) -> Self {
        Self {
            kind,
            detail: detail.into(),
        }
    }
}

impl fmt::Display for NamedSourceSetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.detail)
    }
}

impl std::error::Error for NamedSourceSetError {}

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
    let lexical_path = lexical_contained_source_root(&context.workspace_root, &source_set.path)
        .map_err(|error| NamedSourceSetError::new(NamedSourceSetErrorKind::Containment, error))?;
    reject_linked_source_root_route(&context.workspace_root, &lexical_path)?;
    let path = normalize_contained_source_root(&context.workspace_root, &source_set.path)
        .map_err(|error| NamedSourceSetError::new(NamedSourceSetErrorKind::Containment, error))?;
    Ok(ResolvedNamedSourceSet {
        source_set,
        lexical_path,
        path,
    })
}

fn lexical_contained_source_root(
    workspace_root: &Path,
    configured_path: impl AsRef<Path>,
) -> Result<PathBuf, String> {
    let workspace_root = absolute_lexical(workspace_root)?;
    let configured_path = configured_path.as_ref();
    let candidate = if configured_path.is_absolute() {
        normalize_lexically(configured_path)
    } else {
        normalize_lexically(&workspace_root.join(configured_path))
    };
    // The same host-case and verbatim-prefix policy `WorkspacePathPolicy` uses,
    // so a source root it would accept is not rejected here as uncontained.
    if !crate::infrastructure::platform::filesystem::path_starts_with_host_root(
        &candidate,
        &workspace_root,
    ) {
        return Err(format!(
            "configured source root is outside workspace root {}: {}",
            workspace_root.display(),
            candidate.display()
        ));
    }
    Ok(candidate)
}

fn reject_linked_source_root_route(
    workspace_root: &Path,
    source_root: &Path,
) -> Result<(), NamedSourceSetError> {
    let containment =
        |detail: String| NamedSourceSetError::new(NamedSourceSetErrorKind::Containment, detail);
    let workspace_root = absolute_lexical(workspace_root).map_err(containment)?;
    let relative = source_root.strip_prefix(&workspace_root).map_err(|_| {
        containment(format!(
            "configured source root is outside workspace root {}",
            workspace_root.display()
        ))
    })?;
    let mut current = workspace_root;
    for component in relative.components() {
        current.push(component.as_os_str());
        let metadata = fs::symlink_metadata(&current).map_err(|error| {
            // A declared source root whose directory does not exist yet is a
            // discovery condition. Reporting it as a containment denial sends
            // the caller looking for a symbolic link that is not there.
            let kind = if error.kind() == std::io::ErrorKind::NotFound {
                NamedSourceSetErrorKind::NotFound
            } else {
                NamedSourceSetErrorKind::Discovery
            };
            NamedSourceSetError::new(
                kind,
                format!(
                    "failed to inspect configured source-root route {}: {error}",
                    current.display()
                ),
            )
        })?;
        if crate::infrastructure::platform::filesystem::metadata_is_link_or_reparse_point(&metadata)
        {
            return Err(containment(format!(
                "configured source-root route contains a symbolic link or reparse point: {}",
                current.display()
            )));
        }
    }
    Ok(())
}

fn absolute_lexical(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        Ok(normalize_lexically(path))
    } else {
        std::env::current_dir()
            .map(|cwd| normalize_lexically(&cwd.join(path)))
            .map_err(|error| format!("failed to determine current directory: {error}"))
    }
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

/// Backstop for the source walk. Symlinks are never followed, so a real tree
/// cannot cycle and this limit only bounds pathological inputs. A real 8.3.27
/// configuration bottoms out around depth 10 — nested subsystem XML reaches it —
/// so the limit does not fire in practice. It must stay far above any real
/// layout: a walk that stops early reports an unchanged generation for edits
/// below the cut, which is exactly the stale read this fingerprint prevents.
const MAX_SOURCE_DEPTH: usize = 64;

/// Directory frames keep `A/` + `B` from hashing like a sibling pair `A`, `B`.
const ENTER_DIRECTORY: u8 = 1;
const LEAVE_DIRECTORY: u8 = 2;
/// Emitted where the walk knows something exists but cannot read it, so losing
/// access changes the generation instead of reading as "nothing changed".
const UNREADABLE_ENTRY: u8 = 3;

const GENERATED_DIR_NAME: &str = ".build";

/// The walk is bound by metadata syscalls, not CPU, so it stops scaling early:
/// on a 43k-file 8.3.27 configuration (APFS, 14 cores) one worker takes 883 ms,
/// four take 438 ms, and sixteen fall back to 610 ms on filesystem contention.
/// Four keeps the win without paying for the regression.
const MAX_SOURCE_WALK_WORKERS: usize = 4;

/// The walk is `stat`-bound — one call per source file, tens of thousands on a
/// vendor-class configuration — and every RLM-backed read pays for it. Each
/// top-level child is fingerprinted independently, so the children fan out
/// across worker threads and fold back in sorted order for a stable result.
pub(crate) fn source_generation(source_root: &Path) -> u64 {
    let normalized_root =
        normalize_path_identity(source_root).unwrap_or_else(|_| source_root.to_path_buf());
    let mut hasher = DefaultHasher::new();
    let Some(children) = read_source_children(&normalized_root) else {
        UNREADABLE_ENTRY.hash(&mut hasher);
        return hasher.finish();
    };
    for (child, digest) in children.iter().zip(child_digests(&children)) {
        child.name.as_encoded_bytes().hash(&mut hasher);
        digest.hash(&mut hasher);
    }
    hasher.finish()
}

struct SourceChild {
    name: OsString,
    entry: fs::DirEntry,
    is_directory: bool,
}

/// Lists the entries of one directory that take part in the fingerprint, in a
/// stable order. `None` means the directory itself could not be read.
fn read_source_children(directory: &Path) -> Option<Vec<SourceChild>> {
    let mut children = Vec::new();
    for entry in fs::read_dir(directory).ok()?.flatten() {
        // `file_type` comes from the directory enumeration, so this neither
        // follows symlinks nor pays for the extra stat the old `is_dir` did.
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        let name = entry.file_name();
        let is_directory = file_type.is_dir();
        if is_directory {
            if name.as_encoded_bytes() == GENERATED_DIR_NAME.as_bytes() {
                continue;
            }
        } else if !is_source_file_name(&name) {
            continue;
        }
        children.push(SourceChild {
            name,
            entry,
            is_directory,
        });
    }
    children.sort_by(|left, right| left.name.cmp(&right.name));
    Some(children)
}

fn child_digests(children: &[SourceChild]) -> Vec<u64> {
    let workers = thread::available_parallelism()
        .map(NonZeroUsize::get)
        .unwrap_or(1)
        .min(children.len())
        .min(MAX_SOURCE_WALK_WORKERS);
    if workers <= 1 {
        return children.iter().map(child_digest).collect();
    }
    let mut digests = vec![0_u64; children.len()];
    let cursor = AtomicUsize::new(0);
    thread::scope(|scope| {
        let workers = (0..workers)
            .map(|_| {
                scope.spawn(|| {
                    let mut computed = Vec::new();
                    loop {
                        let index = cursor.fetch_add(1, Ordering::Relaxed);
                        let Some(child) = children.get(index) else {
                            return computed;
                        };
                        computed.push((index, child_digest(child)));
                    }
                })
            })
            .collect::<Vec<_>>();
        for worker in workers {
            // A panic here would be a bug in the walk itself; propagating it
            // beats folding a silently wrong fingerprint into the generation.
            for (index, digest) in worker.join().expect("source generation worker panicked") {
                digests[index] = digest;
            }
        }
    });
    digests
}

/// Fingerprints one child on its own hasher so subtrees stay independent of the
/// order their workers happen to finish in.
fn child_digest(child: &SourceChild) -> u64 {
    let mut hasher = DefaultHasher::new();
    if child.is_directory {
        ENTER_DIRECTORY.hash(&mut hasher);
        hash_source_tree(&mut hasher, &child.entry.path(), 1);
        LEAVE_DIRECTORY.hash(&mut hasher);
    } else {
        hash_source_file(&mut hasher, &child.entry);
    }
    hasher.finish()
}

/// Hashes names relative to the walked directory rather than absolute paths:
/// the fingerprint answers "did these sources change", and the marker that
/// stores it is pinned to its source root separately.
fn hash_source_tree(hasher: &mut DefaultHasher, directory: &Path, depth: usize) {
    if depth > MAX_SOURCE_DEPTH {
        UNREADABLE_ENTRY.hash(hasher);
        return;
    }
    let Some(children) = read_source_children(directory) else {
        UNREADABLE_ENTRY.hash(hasher);
        return;
    };
    for child in children {
        child.name.as_encoded_bytes().hash(hasher);
        if child.is_directory {
            ENTER_DIRECTORY.hash(hasher);
            hash_source_tree(hasher, &child.entry.path(), depth + 1);
            LEAVE_DIRECTORY.hash(hasher);
        } else {
            hash_source_file(hasher, &child.entry);
        }
    }
}

fn is_source_file_name(name: &OsStr) -> bool {
    Path::new(name)
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|extension| matches!(extension, "bsl" | "xml" | "yaml" | "yml"))
}

fn hash_source_file(hasher: &mut DefaultHasher, entry: &fs::DirEntry) {
    let Ok(metadata) = entry.metadata() else {
        UNREADABLE_ENTRY.hash(hasher);
        return;
    };
    metadata.len().hash(hasher);
    if let Ok(modified) = metadata.modified() {
        if let Ok(duration) = modified.duration_since(std::time::UNIX_EPOCH) {
            duration.as_secs().hash(hasher);
            duration.subsec_nanos().hash(hasher);
        }
    }
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

pub(crate) fn select_unique_deepest_source_set_match<'a>(
    target: &Path,
    matches: Vec<(&'a ProjectSourceSet, PathBuf)>,
) -> Result<Option<(&'a ProjectSourceSet, PathBuf)>, String> {
    let mut deepest = deepest_source_set_matches(matches);
    if deepest.len() > 1 {
        let descriptions = deepest
            .iter()
            .map(|(source_set, root)| {
                format!(
                    "`{}` ({:?}, {})",
                    source_set.name,
                    source_set.kind,
                    root.display()
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!(
            "ambiguous source-set ownership for {}: {} equally specific matches: {descriptions}",
            target.display(),
            deepest.len()
        ));
    }
    Ok(deepest.pop())
}

pub(crate) fn deepest_source_set_matches(
    matches: Vec<(&ProjectSourceSet, PathBuf)>,
) -> Vec<(&ProjectSourceSet, PathBuf)> {
    let Some(deepest_depth) = matches
        .iter()
        .map(|(_, root)| root.components().count())
        .max()
    else {
        return Vec::new();
    };
    matches
        .into_iter()
        .filter(|(_, root)| root.components().count() == deepest_depth)
        .collect()
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

/// Stable prefix every source-root rejection carries. Consumers that classify a
/// readiness failure as caller-fixable match on this instead of a literal.
pub(crate) const INVALID_SOURCE_ROOT_PREFIX: &str = "invalid_source_root:";

fn invalid_source_root(error: String) -> String {
    format!("{INVALID_SOURCE_ROOT_PREFIX} {error}")
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
    use super::{normalize_path_identity, resolve_source_root, source_generation};
    use crate::domain::workspace::WorkspaceContext;
    use crate::infrastructure::workspace::discover_workspace;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEMP_WORKSPACE_NONCE: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn source_generation_ignores_build_cache_and_tracks_bsl_changes() {
        let context = fixture(&[("main", "CONFIGURATION", "src")]);
        let source_root = context.workspace_root.join("src");
        let module = source_root.join("CommonModules/SmokeModule.bsl");
        fs::create_dir_all(module.parent().unwrap()).unwrap();
        fs::write(&module, "Процедура Тест() Экспорт\nКонецПроцедуры\n").unwrap();
        let baseline = source_generation(&source_root);

        let generated = source_root.join(".build/bsl-graph.db");
        fs::create_dir_all(generated.parent().unwrap()).unwrap();
        fs::write(&generated, "generated cache").unwrap();
        assert_eq!(source_generation(&source_root), baseline);

        fs::write(
            &module,
            "Процедура Тест(НовыйПараметр = Неопределено) Экспорт\nКонецПроцедуры\n",
        )
        .unwrap();
        assert_ne!(source_generation(&source_root), baseline);
        cleanup(&context);
    }

    /// A real 8.3.27 configuration puts nested subsystem XML at depth 10, and
    /// `Ext/Form/Module.bsl` under a nested source set reaches the same range.
    /// The walk used to stop at depth 8, so edits below the cut left the
    /// generation unchanged and a stale index still read as fresh.
    #[test]
    fn source_generation_tracks_changes_below_the_former_depth_limit() {
        let context = fixture(&[("main", "CONFIGURATION", "src")]);
        let source_root = context.workspace_root.join("src");
        let nested = source_root
            .join("Subsystems/Обмен/Subsystems/ЭДО/Subsystems/Форматы/Subsystems/Служебный");
        fs::create_dir_all(&nested).unwrap();
        let deep = nested.join("Ext/Module.bsl");
        fs::create_dir_all(deep.parent().unwrap()).unwrap();
        fs::write(&deep, "Процедура Тест() Экспорт\nКонецПроцедуры\n").unwrap();
        let baseline = source_generation(&source_root);

        fs::write(
            &deep,
            "Процедура Тест(НовыйПараметр) Экспорт\nКонецПроцедуры\n",
        )
        .unwrap();

        assert_ne!(
            source_generation(&source_root),
            baseline,
            "a change below the former depth limit must change the source generation"
        );
        cleanup(&context);
    }

    #[test]
    fn source_generation_is_stable_across_repeated_walks() {
        let context = fixture(&[("main", "CONFIGURATION", "src")]);
        let source_root = context.workspace_root.join("src");
        for index in 0..64 {
            let module = source_root.join(format!("CommonModules/Module{index}/Ext/Module.bsl"));
            fs::create_dir_all(module.parent().unwrap()).unwrap();
            fs::write(
                &module,
                format!("Процедура Тест{index}()\nКонецПроцедуры\n"),
            )
            .unwrap();
        }

        let first = source_generation(&source_root);

        assert_eq!(
            source_generation(&source_root),
            first,
            "the fan-out walk must fold worker results back in a stable order"
        );
        cleanup(&context);
    }

    #[test]
    fn source_generation_does_not_follow_symlinks_into_a_cycle() {
        let context = fixture(&[("main", "CONFIGURATION", "src")]);
        let source_root = context.workspace_root.join("src");
        let module = source_root.join("CommonModules/SmokeModule.bsl");
        fs::create_dir_all(module.parent().unwrap()).unwrap();
        fs::write(&module, "Процедура Тест()\nКонецПроцедуры\n").unwrap();
        let baseline = source_generation(&source_root);

        // Windows agents without the privilege cannot create one at all, which
        // is reported as `None` rather than a failure.
        let Some(symlink_result) =
            crate::infrastructure::platform::filesystem::create_dir_symlink_for_test(
                &source_root,
                source_root.join("CommonModules/loop"),
            )
        else {
            cleanup(&context);
            return;
        };
        symlink_result.unwrap();

        assert_eq!(
            source_generation(&source_root),
            baseline,
            "symlinks must be skipped so the walk cannot cycle"
        );
        cleanup(&context);
    }

    #[test]
    fn source_generation_uses_normalized_source_root_identity() {
        let context = fixture(&[("main", "CONFIGURATION", "src")]);
        let source_root = context.workspace_root.join("src");
        let module = source_root.join("CommonModules/SmokeModule.bsl");
        fs::create_dir_all(module.parent().unwrap()).unwrap();
        fs::write(&module, "Процедура Тест() Экспорт\nКонецПроцедуры\n").unwrap();
        let equivalent_root = source_root.join("CommonModules").join("..");

        assert_eq!(
            source_generation(&source_root),
            source_generation(&equivalent_root),
            "equivalent source-root spellings must identify one source generation"
        );
        cleanup(&context);
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
