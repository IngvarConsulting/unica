//! Removal of RLM index generations an earlier RLM version left behind.
//!
//! An incompatible RLM index format gets its own generation directory
//! (`index-v<builder>`) under three places of the per-workspace provider
//! state: the data root `rlm-bsl/`, `caches/rlm-bsl/` and `locks/rlm-bsl/`.
//! After an RLM update the previous generation is never read again by this
//! build, yet it keeps the whole index on disk. A generation is removed only
//! when it is idle, its index lock is free, and none of its directories is a
//! link.

use crate::infrastructure::platform::filesystem::{
    metadata_is_link_or_reparse_point, open_absolute_directory_path_nofollow,
    ownership_locks_pin_their_directory, remove_directory_tree_child, rename_no_replace,
};
use fs2::FileExt;
use std::collections::{BTreeSet, HashMap};
use std::ffi::OsStr;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

/// How long every directory of a generation must stay untouched. A build of
/// an older Unica still using that generation keeps touching it; reverting to
/// that build later is allowed to rebuild the removed index.
pub(crate) const STALE_RLM_GENERATION_IDLE: Duration = Duration::from_secs(24 * 60 * 60);

/// A pair root is examined at most this often by one process.
const COLLECTION_RETRY_AFTER: Duration = Duration::from_secs(60 * 60);

const PRODUCT_DIR: &str = "rlm-bsl";
const GENERATION_PREFIX: &str = "index-v";
const TRASH_PREFIX: &str = ".trash-rlm-";
const LOCK_FILE_NAME: &str = "bsl_index.lock";
/// The index layout is shallower than this. A directory at this depth that
/// still has entries is not walked, and unwalked content cannot prove the
/// generation idle, so such a generation is kept.
const MAX_IDLE_WALK_DEPTH: usize = 6;

/// Parents of generation directories, relative to the pair root.
fn generation_parents(pair_root: &Path) -> [PathBuf; 3] {
    [
        pair_root.join(PRODUCT_DIR),
        pair_root.join("caches").join(PRODUCT_DIR),
        pair_root.join("locks").join(PRODUCT_DIR),
    ]
}

/// Start a best-effort background collection for `pair_root` unless this
/// process examined it recently. Never fails and never blocks the caller.
pub(crate) fn spawn_stale_rlm_generation_collection(pair_root: PathBuf, current: &'static str) {
    static LAST_RUN: OnceLock<Mutex<HashMap<PathBuf, Instant>>> = OnceLock::new();
    let due = LAST_RUN
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map(|mut runs| {
            let now = Instant::now();
            let due = runs
                .get(&pair_root)
                .is_none_or(|last| now.duration_since(*last) >= COLLECTION_RETRY_AFTER);
            if due {
                runs.insert(pair_root.clone(), now);
            }
            due
        })
        .unwrap_or(false);
    if !due {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("unica-rlm-generation-collection".to_string())
        .spawn(move || {
            let _ = collect_stale_rlm_generations(
                &pair_root,
                current,
                STALE_RLM_GENERATION_IDLE,
                SystemTime::now(),
            );
        });
}

/// Remove idle, unlocked generations other than `current` and the trash an
/// interrupted collection left. Returns the generation and trash names
/// removed. A busy, recent or linked generation stays for a later attempt.
pub(crate) fn collect_stale_rlm_generations(
    pair_root: &Path,
    current: &str,
    idle: Duration,
    now: SystemTime,
) -> Result<Vec<String>, String> {
    let parents = generation_parents(pair_root);
    let mut removed = Vec::new();
    let mut generations = BTreeSet::new();
    for parent in &parents {
        // A route through a link or reparse point is not this code's to walk.
        if !route_is_real(pair_root, parent) || !parent.is_dir() {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(parent) else {
            continue;
        };
        for name in entries.filter_map(|entry| entry.ok()?.file_name().into_string().ok()) {
            if name.starts_with(TRASH_PREFIX) {
                if remove_child_tree(parent, &name).is_ok() {
                    removed.push(name);
                }
            } else if is_generation_name(&name) && name != current {
                generations.insert(name);
            }
        }
    }
    for generation in generations {
        if retire_generation(pair_root, &parents, &generation, idle, now) {
            removed.push(generation);
        }
    }
    Ok(removed)
}

fn is_generation_name(name: &str) -> bool {
    name.strip_prefix(GENERATION_PREFIX).is_some_and(|digits| {
        !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
    })
}

/// Every existing component from the pair root down to `path` is a plain
/// directory entry, never a link or reparse point. A missing tail is allowed.
fn route_is_real(pair_root: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(pair_root) else {
        return false;
    };
    let mut current = pair_root.to_path_buf();
    for component in std::iter::once(None).chain(relative.components().map(Some)) {
        if let Some(component) = component {
            current.push(component.as_os_str());
        }
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if metadata_is_link_or_reparse_point(&metadata) => return false,
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return false,
        }
    }
    true
}

/// Move every directory of one generation into trash, then remove the trash.
/// `false` leaves the generation as it was found, or for a later attempt.
fn retire_generation(
    pair_root: &Path,
    parents: &[PathBuf; 3],
    generation: &str,
    idle: Duration,
    now: SystemTime,
) -> bool {
    let mut present = Vec::new();
    for parent in parents {
        let path = parent.join(generation);
        if !route_is_real(pair_root, &path) {
            // A link anywhere on the route would take the move outside the
            // pair root; the whole generation stays.
            return false;
        }
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_dir() => {
                present.push(parent.clone());
            }
            // A link or a file under a generation name is left untouched, and
            // so is the rest of that generation.
            Ok(_) => return false,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return false,
        }
    }
    if present.is_empty() {
        return false;
    }
    // Measured before locking: opening the lock may itself touch the tree.
    if !present
        .iter()
        .all(|parent| is_idle(&parent.join(generation), idle, now))
    {
        return false;
    }
    let lock = match try_generation_lock(&parents[2].join(generation)) {
        Ok(lock) => lock,
        Err(()) => return false,
    };
    let held = if ownership_locks_pin_their_directory() {
        // Windows: a held handle would refuse the move of its own directory,
        // so the lock is released first. A build that takes it in between
        // can lose the data and cache parts moved before `locks/`; the order
        // data, caches, locks makes an open index database refuse the very
        // first move instead, leaving the generation whole.
        drop(lock);
        None
    } else {
        lock
    };
    let mut retired = Vec::new();
    for parent in &present {
        let trash = format!("{TRASH_PREFIX}{}", uuid::Uuid::new_v4());
        if rename_no_replace(&parent.join(generation), &parent.join(&trash)).is_err() {
            // The already moved parts are removed below; the rest stays and is
            // retried later. The generation is never read again either way.
            break;
        }
        retired.push((parent, trash));
    }
    drop(held);
    let complete = retired.len() == present.len();
    for (parent, trash) in retired {
        let _ = remove_child_tree(parent, &trash);
    }
    complete
}

/// `Ok(None)` when the generation has no lock file; `Err` when a live process
/// holds it or its state cannot be proven.
fn try_generation_lock(lock_dir: &Path) -> Result<Option<File>, ()> {
    let path = lock_dir.join(LOCK_FILE_NAME);
    match std::fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Ok(metadata) if metadata.is_file() => {}
        _ => return Err(()),
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|_| ())?;
    file.try_lock_exclusive().map_err(|_| ())?;
    Ok(Some(file))
}

/// Every directory and file of the tree, links themselves included but not
/// followed, changed no later than `now - idle`.
fn is_idle(path: &Path, idle: Duration, now: SystemTime) -> bool {
    fn newest_change(path: &Path, depth: usize) -> Option<SystemTime> {
        let metadata = std::fs::symlink_metadata(path).ok()?;
        let mut newest = metadata.modified().ok()?;
        if metadata.is_dir() && !metadata_is_link_or_reparse_point(&metadata) {
            if depth >= MAX_IDLE_WALK_DEPTH {
                return std::fs::read_dir(path)
                    .ok()?
                    .next()
                    .is_none()
                    .then_some(newest);
            }
            for entry in std::fs::read_dir(path).ok()? {
                newest = newest.max(newest_change(&entry.ok()?.path(), depth + 1)?);
            }
        }
        Some(newest)
    }
    newest_change(path, 0)
        .is_some_and(|newest| now.duration_since(newest).is_ok_and(|age| age >= idle))
}

/// The parent is opened without following links only for the removal.
fn remove_child_tree(parent: &Path, name: &str) -> std::io::Result<()> {
    let parent = open_absolute_directory_path_nofollow(parent)?;
    remove_directory_tree_child(&parent, OsStr::new(name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::platform::filesystem::create_dir_symlink_for_test;

    const CURRENT: &str = "index-v17";

    fn root() -> (tempfile::TempDir, PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let path = std::fs::canonicalize(directory.path()).unwrap();
        (directory, path)
    }

    fn seed_generation(pair_root: &Path, generation: &str) {
        let data = pair_root.join(PRODUCT_DIR).join(generation);
        std::fs::create_dir_all(data.join("builds/0123")).unwrap();
        std::fs::write(data.join("builds/0123/bsl_index.db"), b"db").unwrap();
        let caches = pair_root.join("caches").join(PRODUCT_DIR).join(generation);
        std::fs::create_dir_all(&caches).unwrap();
        std::fs::write(caches.join("bsl_index_status.json"), b"{}").unwrap();
        let locks = pair_root.join("locks").join(PRODUCT_DIR).join(generation);
        std::fs::create_dir_all(&locks).unwrap();
        std::fs::write(locks.join(LOCK_FILE_NAME), b"{}").unwrap();
    }

    fn generation_paths(pair_root: &Path, generation: &str) -> [PathBuf; 3] {
        generation_parents(pair_root).map(|parent| parent.join(generation))
    }

    fn collect(pair_root: &Path, idle: Duration) -> Vec<String> {
        let mut removed =
            collect_stale_rlm_generations(pair_root, CURRENT, idle, SystemTime::now()).unwrap();
        removed.sort();
        removed
    }

    fn no_trash_left(pair_root: &Path) -> bool {
        generation_parents(pair_root).iter().all(|parent| {
            std::fs::read_dir(parent).map_or(true, |entries| {
                entries
                    .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
                    .all(|name| !name.starts_with(TRASH_PREFIX))
            })
        })
    }

    /// Прежнее поколение (данные, кэш и блокировка) удаляется целиком,
    /// текущее и посторонние каталоги остаются; недавняя активность сохраняет
    /// и прежнее поколение.
    #[test]
    fn only_an_idle_previous_generation_is_removed_in_every_location() {
        let (_guard, pair_root) = root();
        seed_generation(&pair_root, "index-v15");
        seed_generation(&pair_root, CURRENT);
        let unrelated = [
            pair_root.join(PRODUCT_DIR).join("index-vnext"),
            pair_root.join(PRODUCT_DIR).join("index-v15-backup"),
            pair_root.join("caches").join(PRODUCT_DIR).join("notes"),
            pair_root.join("rlm-tools-bsl"),
        ];
        for path in &unrelated {
            std::fs::create_dir_all(path).unwrap();
        }
        let interrupted = pair_root
            .join(PRODUCT_DIR)
            .join(format!("{TRASH_PREFIX}left-over"));
        std::fs::create_dir_all(interrupted.join("builds")).unwrap();

        assert_eq!(
            collect(&pair_root, Duration::from_secs(3600)),
            vec![format!("{TRASH_PREFIX}left-over")]
        );
        assert!(generation_paths(&pair_root, "index-v15")
            .iter()
            .all(|path| path.is_dir()));

        assert_eq!(
            collect(&pair_root, Duration::ZERO),
            vec!["index-v15".to_string()]
        );
        for path in generation_paths(&pair_root, "index-v15") {
            assert!(!path.exists(), "{}", path.display());
        }
        for path in generation_paths(&pair_root, CURRENT) {
            assert!(path.is_dir(), "{}", path.display());
        }
        for path in &unrelated {
            assert!(path.is_dir(), "{}", path.display());
        }
        assert!(!interrupted.exists());
        assert!(no_trash_left(&pair_root));
    }

    /// Поколение, чью блокировку держит живой процесс, остаётся до следующей
    /// попытки и удаляется после её освобождения.
    #[test]
    fn a_generation_whose_lock_is_held_is_kept_until_released() {
        let (_guard, pair_root) = root();
        seed_generation(&pair_root, "index-v15");
        let lock_path = pair_root
            .join("locks")
            .join(PRODUCT_DIR)
            .join("index-v15")
            .join(LOCK_FILE_NAME);
        let holder = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&lock_path)
            .unwrap();
        holder.lock_exclusive().unwrap();

        assert!(collect(&pair_root, Duration::ZERO).is_empty());
        assert!(generation_paths(&pair_root, "index-v15")
            .iter()
            .all(|path| path.is_dir()));

        holder.unlock().unwrap();
        drop(holder);
        assert_eq!(
            collect(&pair_root, Duration::ZERO),
            vec!["index-v15".to_string()]
        );
        assert!(no_trash_left(&pair_root));
    }

    /// Ссылка с именем поколения не открывается и не удаляется, вместе с ней
    /// остаётся и остальная часть этого поколения; цель ссылки цела.
    #[test]
    fn a_link_named_like_a_generation_is_neither_followed_nor_removed() {
        let (_guard, pair_root) = root();
        seed_generation(&pair_root, "index-v15");
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("keep"), b"data").unwrap();
        let data = pair_root.join(PRODUCT_DIR).join("index-v15");
        std::fs::remove_dir_all(&data).unwrap();
        if !matches!(
            create_dir_symlink_for_test(outside.path(), &data),
            Some(Ok(()))
        ) {
            return; // The host forbids creating links; nothing to prove here.
        }

        assert!(collect(&pair_root, Duration::ZERO).is_empty());
        assert!(outside.path().join("keep").is_file());
        assert!(std::fs::symlink_metadata(&data)
            .unwrap()
            .file_type()
            .is_symlink());
        for path in &generation_paths(&pair_root, "index-v15")[1..] {
            assert!(path.is_dir(), "{}", path.display());
        }
    }

    /// Ссылка на любом уровне маршрута к поколению — `caches` или
    /// `caches/rlm-bsl` — не обходится: уборка не выходит по ней за пределы
    /// корня пары, чужой каталог с именем поколения не переименовывается.
    #[test]
    fn a_linked_generation_parent_is_not_walked() {
        for linked in ["caches", "caches/rlm-bsl"] {
            let (_guard, pair_root) = root();
            let outside = tempfile::tempdir().unwrap();
            let target = if linked == "caches" {
                outside.path().join(PRODUCT_DIR)
            } else {
                outside.path().to_path_buf()
            };
            let foreign = target.join("index-v15");
            std::fs::create_dir_all(&foreign).unwrap();
            std::fs::write(foreign.join("keep"), b"data").unwrap();
            let link = pair_root.join(linked);
            std::fs::create_dir_all(link.parent().unwrap()).unwrap();
            let link_target = if linked == "caches" {
                outside.path()
            } else {
                target.as_path()
            };
            if !matches!(
                create_dir_symlink_for_test(link_target, &link),
                Some(Ok(()))
            ) {
                return; // The host forbids creating links; nothing to prove here.
            }
            seed_generation_data_only(&pair_root, "index-v15");

            assert!(collect(&pair_root, Duration::ZERO).is_empty(), "{linked}");
            assert!(foreign.join("keep").is_file(), "{linked}");
            let leftovers = std::fs::read_dir(&target)
                .unwrap()
                .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
                .filter(|name| name.starts_with(TRASH_PREFIX))
                .count();
            assert_eq!(leftovers, 0, "{linked}");
            assert!(
                pair_root.join(PRODUCT_DIR).join("index-v15").is_dir(),
                "{linked}"
            );
        }
    }

    /// Свежий файл глубоко в дереве (база сборки) удерживает всё поколение,
    /// даже когда сами каталоги давно не менялись.
    #[test]
    fn a_recent_file_deep_in_a_generation_keeps_it() {
        let (_guard, pair_root) = root();
        seed_generation(&pair_root, "index-v15");
        let day = Duration::from_secs(24 * 60 * 60);
        let old = SystemTime::now() - 2 * day;
        if age_tree(&pair_root, old).is_err() {
            // Windows cannot open a directory this way to set its time: the
            // idle check is proven on Unix only.
            return;
        }
        let db = pair_root
            .join(PRODUCT_DIR)
            .join("index-v15/builds/0123/bsl_index.db");
        File::options()
            .write(true)
            .open(&db)
            .unwrap()
            .set_modified(SystemTime::now())
            .unwrap();

        assert!(collect(&pair_root, day).is_empty());
        assert!(db.is_file());

        age_tree(&pair_root, old).unwrap();
        assert_eq!(collect(&pair_root, day), vec!["index-v15".to_string()]);
    }

    /// Содержимое глубже предела обхода не просматривается и потому не
    /// доказывает простой: такое поколение остаётся.
    #[test]
    fn content_below_the_walk_limit_keeps_the_generation() {
        let (_guard, pair_root) = root();
        seed_generation(&pair_root, "index-v15");
        let mut deep = pair_root.join(PRODUCT_DIR).join("index-v15");
        for level in 0..MAX_IDLE_WALK_DEPTH {
            deep = deep.join(format!("d{level}"));
        }
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(deep.join("unseen.db"), b"x").unwrap();
        let day = Duration::from_secs(24 * 60 * 60);
        if age_tree(&pair_root, SystemTime::now() - 2 * day).is_err() {
            // Windows cannot open a directory this way to set its time.
            return;
        }

        assert!(collect(&pair_root, day).is_empty());
        assert!(deep.join("unseen.db").is_file());

        std::fs::remove_file(deep.join("unseen.db")).unwrap();
        age_tree(&pair_root, SystemTime::now() - 2 * day).unwrap();
        assert_eq!(collect(&pair_root, day), vec!["index-v15".to_string()]);
    }

    /// Sets the modification time of every entry under `root`, deepest first.
    fn age_tree(root: &Path, time: SystemTime) -> std::io::Result<()> {
        for entry in std::fs::read_dir(root)? {
            let path = entry?.path();
            if path.is_dir() {
                age_tree(&path, time)?;
            } else {
                File::options()
                    .write(true)
                    .open(&path)?
                    .set_modified(time)?;
            }
        }
        File::open(root)?.set_modified(time)
    }

    fn seed_generation_data_only(pair_root: &Path, generation: &str) {
        let data = pair_root.join(PRODUCT_DIR).join(generation);
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("bsl_index.db"), b"db").unwrap();
    }
}
