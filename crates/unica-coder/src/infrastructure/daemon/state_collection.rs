//! Removal of the state directories that other builds' daemons left behind.
//!
//! Each build keys its own daemon directory (`daemon-p5-<identity>`), so the
//! shared state root keeps one directory per build ever started. A directory
//! is removed only when it is idle and its two ownership locks prove that no
//! daemon of that build is alive and none is starting.

use super::identity::{CoreIdentity, DaemonStateDirectory};
use crate::infrastructure::platform::filesystem::{
    open_absolute_directory_path_nofollow, ownership_locks_pin_their_directory,
    remove_directory_tree_child, rename_no_replace,
};
use std::ffi::OsStr;
use std::path::Path;
use std::str::FromStr;
use std::time::{Duration, SystemTime};

/// How long a directory must stay untouched before it is a candidate.
pub(crate) const STALE_DAEMON_STATE_IDLE: Duration = Duration::from_secs(7 * 24 * 60 * 60);

const STATE_PREFIX: &str = "daemon-p5-";
const TRASH_PREFIX: &str = ".trash-daemon-";

/// Remove the idle, unowned state directories of other builds and the trash
/// an interrupted collection left. Returns the names removed.
///
/// Only protocol-v5 directories are candidates: the locks of a retired
/// protocol are not this code's, so it cannot prove their daemon is gone.
pub(crate) fn collect_stale_daemon_states(
    state_root: &Path,
    own: &CoreIdentity,
    idle: Duration,
    now: SystemTime,
) -> Result<Vec<String>, String> {
    let names = std::fs::read_dir(state_root)
        .map_err(|error| format!("list daemon provider state root: {error}"))?
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .collect::<Vec<_>>();
    let mut removed = Vec::new();
    for name in names {
        if name.starts_with(TRASH_PREFIX) {
            if remove_child_tree(state_root, &name).is_ok() {
                removed.push(name);
            }
            continue;
        }
        let Some(identity) = foreign_identity(&name, own) else {
            continue;
        };
        // Measured before locking: taking the locks may itself create entries.
        if !is_idle(&state_root.join(&name), idle, now) {
            continue;
        }
        let Ok(Some(trash)) = retire(state_root, &identity, &name) else {
            continue;
        };
        if remove_child_tree(state_root, &trash).is_ok() {
            removed.push(name);
        }
    }
    Ok(removed)
}

/// The root is opened only for the removal: on Windows a listing handle on
/// the destination parent can refuse the preceding rename.
fn remove_child_tree(state_root: &Path, name: &str) -> std::io::Result<()> {
    let root = open_absolute_directory_path_nofollow(state_root)?;
    remove_directory_tree_child(&root, OsStr::new(name))
}

fn foreign_identity(name: &str, own: &CoreIdentity) -> Option<CoreIdentity> {
    let identity = CoreIdentity::from_str(name.strip_prefix(STATE_PREFIX)?).ok()?;
    (&identity != own).then_some(identity)
}

/// A real directory, not a link, whose newest own or direct-child change is
/// older than `idle`.
fn is_idle(path: &Path, idle: Duration, now: SystemTime) -> bool {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return false;
    };
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return false;
    }
    let Ok(mut newest) = metadata.modified() else {
        return false;
    };
    let Ok(entries) = std::fs::read_dir(path) else {
        return false;
    };
    for entry in entries {
        let Some(modified) = entry
            .ok()
            .and_then(|entry| std::fs::symlink_metadata(entry.path()).ok())
            .and_then(|metadata| metadata.modified().ok())
        else {
            return false;
        };
        newest = newest.max(modified);
    }
    now.duration_since(newest).is_ok_and(|age| age >= idle)
}

/// Move a provably unowned directory out of its name into trash. `None` when
/// a daemon of that build is alive or starting, or the move was refused.
fn retire(
    state_root: &Path,
    identity: &CoreIdentity,
    name: &str,
) -> Result<Option<String>, String> {
    let Some(state) = DaemonStateDirectory::open_existing(state_root, identity)? else {
        return Ok(None);
    };
    // The order matches a starting frontend: spawn lock first, then the
    // authority a live daemon holds for its whole lifetime.
    let Some(spawn) = state.try_spawn_lock()? else {
        return Ok(None);
    };
    let Some(authority) = state.try_receipt_authority()? else {
        return Ok(None);
    };
    let trash = format!("{TRASH_PREFIX}{}", uuid::Uuid::new_v4());
    let source = state_root.join(name);
    let target = state_root.join(&trash);
    if !ownership_locks_pin_their_directory() {
        // The locks move with the directory object. A frontend or daemon of
        // that build waiting on them finds its name gone once they are free
        // and stops instead of using the trash.
        let moved = rename_no_replace(&source, &target);
        drop((authority, spawn, state));
        return Ok(moved.ok().map(|()| trash));
    }
    // Held locks pin the directory here, so they are released first. A
    // process that opens a lock in between pins it again and the move fails.
    drop((authority, spawn, state));
    Ok(rename_no_replace(&source, &target).ok().map(|()| trash))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::platform::filesystem::create_dir_symlink_for_test;

    fn identity(digit: char) -> CoreIdentity {
        CoreIdentity::from_str(&digit.to_string().repeat(64)).unwrap()
    }

    fn state_name(identity: &CoreIdentity) -> String {
        format!("{STATE_PREFIX}{}", identity.as_str())
    }

    fn root() -> (tempfile::TempDir, std::path::PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let path = std::fs::canonicalize(directory.path()).unwrap();
        (directory, path)
    }

    fn collect(root: &Path, own: &CoreIdentity, idle: Duration) -> Vec<String> {
        let mut removed = collect_stale_daemon_states(root, own, idle, SystemTime::now()).unwrap();
        removed.sort();
        removed
    }

    /// Удаляется только простаивающий каталог чужой сборки, у которого ни одна
    /// из двух блокировок не занята: живой демон держит полномочие квитанций,
    /// запускающий его frontend — блокировку запуска.
    #[test]
    fn only_an_idle_unowned_state_of_another_build_is_removed() {
        let (_guard, root) = root();
        let own = identity('0');
        let dead = identity('a');
        let alive = identity('b');
        let starting = identity('c');
        for state in [&own, &dead, &alive, &starting] {
            DaemonStateDirectory::open(&root, state).unwrap();
        }
        let alive_state = DaemonStateDirectory::open(&root, &alive).unwrap();
        let _authority = alive_state
            .acquire_receipt_authority(Duration::from_millis(50))
            .unwrap();
        let starting_state = DaemonStateDirectory::open(&root, &starting).unwrap();
        let _spawn = starting_state
            .acquire_spawn_lock(Duration::from_millis(50))
            .unwrap();
        let retired_protocol = root.join(format!("daemon-p3-{}", "d".repeat(64)));
        std::fs::create_dir(&retired_protocol).unwrap();
        let unrelated = root.join("rlm-index");
        std::fs::create_dir(&unrelated).unwrap();
        let interrupted = root.join(format!("{TRASH_PREFIX}left-over"));
        std::fs::create_dir_all(interrupted.join("receipts")).unwrap();

        // Recent activity keeps even an unowned directory.
        assert_eq!(
            collect(&root, &own, Duration::from_secs(3600)),
            vec![format!("{TRASH_PREFIX}left-over")]
        );
        assert!(root.join(state_name(&dead)).is_dir());

        assert_eq!(
            collect(&root, &own, Duration::ZERO),
            vec![state_name(&dead)]
        );
        for kept in [&own, &alive, &starting] {
            assert!(root.join(state_name(kept)).is_dir(), "{}", state_name(kept));
        }
        assert!(retired_protocol.is_dir() && unrelated.is_dir());
        assert!(!interrupted.exists());
        let leftovers = std::fs::read_dir(&root)
            .unwrap()
            .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
            .filter(|name| name.starts_with(TRASH_PREFIX))
            .collect::<Vec<_>>();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    /// Ссылка с именем каталога состояния — не каталог: уборка её не открывает
    /// и не удаляет то, на что она указывает.
    #[test]
    fn a_link_named_like_a_state_is_neither_followed_nor_removed() {
        let (_guard, root) = root();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("keep"), b"data").unwrap();
        let link = root.join(state_name(&identity('e')));
        if !matches!(
            create_dir_symlink_for_test(outside.path(), &link),
            Some(Ok(()))
        ) {
            return; // The host forbids creating links; nothing to prove here.
        }

        assert!(collect(&root, &identity('0'), Duration::ZERO).is_empty());
        assert!(outside.path().join("keep").is_file());
        assert!(std::fs::symlink_metadata(&link).is_ok());
    }

    /// Каталог, исчезнувший между листингом и открытием, не воссоздаётся.
    #[test]
    fn inspecting_a_vanished_state_does_not_recreate_it() {
        let (_guard, root) = root();
        assert!(DaemonStateDirectory::open_existing(&root, &identity('f'))
            .unwrap()
            .is_none());
        assert!(!root.join(state_name(&identity('f'))).exists());
    }
}
