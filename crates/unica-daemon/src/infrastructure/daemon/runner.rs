//! The pinned v8-runner: its version, the lock that keeps two invocations for
//! one working copy from overlapping, and how the declared `origin` infobase is
//! read.
//!
//! The runner reads `v8project.yaml` and `v8project.local.yaml` of the working
//! copy in place, from the `--config` argument; Unica neither copies nor
//! rewrites them. The runner records the directory of that config as the
//! working copy that holds a file infobase, so Unica and a person running the
//! runner by hand in the same working copy are one holder.
use super::v13_workspace_bootstrap::read_yaml_config;
use serde_yaml::Value;
use std::path::{Path, PathBuf};
use unica_process::runner::{
    ProcessCommand, ProcessOutput, ProcessRunner, ProcessStreamFailure, SystemProcessRunner,
};
use unica_process::PendingProcessHandoff;
use unica_work_control::cancellation::{CancellationToken, CANCELLED_PREFIX};
use unica_workspace::paths::normalize_path_identity;
use unica_workspace::runner_lock::{RunnerLock, RunnerLockFailure};

pub(super) const VERSION: &str = "0.15.3";
pub(super) fn check_version(version: &str) -> Result<(), String> {
    if version == VERSION {
        Ok(())
    } else {
        Err(format!(
            "no verified Unica runner adapter for version {version}; expected {VERSION}"
        ))
    }
}

const BASE_NAME: &str = "v8project.yaml";
const LOCAL_NAME: &str = "v8project.local.yaml";
const ORIGIN: &str = "origin";

/// Runs the pinned runner on the working-copy config, one invocation per
/// working copy at a time.
pub(super) struct PinnedRunner;
impl ProcessRunner for PinnedRunner {
    fn run(&self, command: &ProcessCommand) -> Result<ProcessOutput, String> {
        let _lock = lock_for(command)?;
        SystemProcessRunner.run(command)
    }

    /// Waiting for the lock happens before any process exists, but it is not
    /// a failed launch of the runner: it is reported as a failure after start,
    /// never as an unstartable engine.
    fn run_staged(&self, command: &ProcessCommand) -> Result<ProcessOutput, ProcessStreamFailure> {
        let _lock = lock_for(command).map_err(ProcessStreamFailure::Run)?;
        SystemProcessRunner.run_staged(command)
    }

    fn run_pending_handoff(
        &self,
        command: &ProcessCommand,
    ) -> Result<(ProcessOutput, PendingProcessHandoff), String> {
        let _lock = lock_for(command)?;
        SystemProcessRunner.run_pending_handoff(command)
    }

    /// As [`Self::run_staged`]: waiting for the lock is not a failed launch.
    fn run_pending_handoff_staged(
        &self,
        command: &ProcessCommand,
    ) -> Result<(ProcessOutput, PendingProcessHandoff), ProcessStreamFailure> {
        let _lock = lock_for(command).map_err(ProcessStreamFailure::Run)?;
        SystemProcessRunner.run_pending_handoff_staged(command)
    }
}

fn lock_for(command: &ProcessCommand) -> Result<RunnerLock, String> {
    let config = command
        .args
        .iter()
        .position(|value| value == "--config")
        .and_then(|position| command.args.get(position + 1))
        .ok_or("runner adapter requires an explicit project config")?;
    let root = Path::new(config)
        .parent()
        .ok_or("project config has no parent")?;
    lock_working_copy(root, &command.cancellation).map_err(|error| match error {
        RunnerLockFailure::Cancelled => {
            format!("{CANCELLED_PREFIX} while waiting for the working copy runner lock")
        }
        RunnerLockFailure::InvalidState(message) => message,
    })
}

pub(super) fn lock_working_copy(
    root: &Path,
    cancellation: &CancellationToken,
) -> Result<RunnerLock, RunnerLockFailure> {
    let root = normalize_path_identity(root).map_err(RunnerLockFailure::InvalidState)?;
    RunnerLock::acquire(&lock_directory(&root), cancellation)
}

/// The lock directory of one working copy, in the Unica cache:
/// `<workspace>/.build/unica/runner-lock` by default. A cache moved out by
/// `UNICA_CACHE_DIR` is shared by workspaces, so there the directory is keyed
/// by the workspace root.
pub(super) fn lock_directory(root: &Path) -> PathBuf {
    lock_directory_in(root, std::env::var_os("UNICA_CACHE_DIR").map(PathBuf::from))
}

fn lock_directory_in(root: &Path, cache_override: Option<PathBuf>) -> PathBuf {
    match cache_override {
        None => root.join(".build").join("unica").join("runner-lock"),
        Some(cache) => {
            use sha2::{Digest, Sha256};
            // A relative cache is read from the current directory, as the
            // Unica cache itself reads it.
            let cache = std::env::current_dir()
                .map(|current| current.join(&cache))
                .unwrap_or(cache);
            let digest = Sha256::digest(root.as_os_str().as_encoded_bytes());
            let key: String = digest[..8]
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            cache.join("runner-lock").join(key)
        }
    }
}

/// Removes the infobase description from a layer and returns its `origin`.
fn take_origin(layer: &mut Value, file: &str) -> Result<Option<Value>, String> {
    let map = layer
        .as_mapping_mut()
        .ok_or_else(|| format!("{file} must be a mapping"))?;
    let legacy = map.remove(Value::from("infobase"));
    let target = map
        .remove(Value::from("infobases"))
        .and_then(|bases| bases.as_mapping()?.get(Value::from(ORIGIN)).cloned());
    match legacy.or(target) {
        Some(section) if section.is_mapping() => Ok(Some(section)),
        Some(_) => Err(format!("the {ORIGIN} infobase in {file} must be a mapping")),
        None => Ok(None),
    }
}

/// The kind of the `origin` infobase, read the way the runner reads it: the
/// section of both layers merged (local wins), `standalone` declares a
/// standalone server, otherwise a `File=` connection a file infobase, otherwise
/// a cluster.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OriginKind {
    Undeclared,
    File,
    Cluster,
    Standalone,
}

pub(super) fn origin_kind(root: &Path) -> Result<OriginKind, String> {
    let mut origin: Option<Value> = None;
    for file in [BASE_NAME, LOCAL_NAME] {
        let Some(mut layer) = read_yaml_config(root, file)? else {
            continue;
        };
        if let Some(section) = take_origin(&mut layer, file)? {
            match origin.as_mut() {
                Some(merged) => merge_yaml_values(merged, section),
                None => origin = Some(section),
            }
        }
    }
    let Some(origin) = origin else {
        return Ok(OriginKind::Undeclared);
    };
    if origin
        .get("standalone")
        .is_some_and(|value| !value.is_null())
    {
        return Ok(OriginKind::Standalone);
    }
    let file = origin
        .get("connection")
        .and_then(Value::as_str)
        .is_some_and(declares_a_file_infobase);
    Ok(if file {
        OriginKind::File
    } else {
        OriginKind::Cluster
    })
}

/// The runner's reading of a declared connection (`declared_parameters`):
/// non-empty `;` parts, each `key=value` with the key trimmed and compared
/// without case; a part without `=` makes the string undeclared, and a file
/// infobase has a non-empty `File` value. Raw argv names it with `/F`.
fn declares_a_file_infobase(connection: &str) -> bool {
    let text = connection.trim();
    if text.starts_with(['/', '-']) {
        return text
            .split_whitespace()
            .any(|token| token.eq_ignore_ascii_case("/F"));
    }
    let parts: Option<Vec<(String, &str)>> = text
        .split(';')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| {
            part.split_once('=')
                .map(|(key, value)| (key.trim().to_ascii_lowercase(), value.trim()))
        })
        .collect();
    parts.is_some_and(|parts| {
        parts
            .iter()
            .any(|(key, value)| key == "file" && !value.is_empty())
    })
}

/// The runner's own layer merge: mappings merge key by key, anything else is
/// replaced by the overlay.
fn merge_yaml_values(base: &mut Value, overlay: Value) {
    match (base, overlay) {
        (Value::Mapping(base), Value::Mapping(overlay)) => {
            for (key, overlay_value) in overlay {
                match base.get_mut(&key) {
                    Some(base_value) => merge_yaml_values(base_value, overlay_value),
                    None => {
                        base.insert(key, overlay_value);
                    }
                }
            }
        }
        (base, overlay) => *base = overlay,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace(base: &str, local: Option<&str>) -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join(BASE_NAME), base).unwrap();
        if let Some(local) = local {
            std::fs::write(root.path().join(LOCAL_NAME), local).unwrap();
        }
        root
    }

    /// Свой замок у каждой рабочей копии: по умолчанию в её кеше, а в общем
    /// кеше `UNICA_CACHE_DIR` — под ключом корня копии.
    #[test]
    fn each_working_copy_has_its_own_lock_directory() {
        assert_eq!(
            lock_directory_in(Path::new("/work/a"), None),
            Path::new("/work/a/.build/unica/runner-lock")
        );
        let cache = std::env::temp_dir().join("cache");
        let a = lock_directory_in(Path::new("/work/a"), Some(cache.clone()));
        let b = lock_directory_in(Path::new("/work/b"), Some(cache.clone()));
        assert!(a.starts_with(cache.join("runner-lock")), "{a:?}");
        assert_ne!(a, b);
        assert_eq!(a, lock_directory_in(Path::new("/work/a"), Some(cache)));
    }

    /// Вид базы `origin` читается так же, как его читает раннер: секции обоих
    /// слоёв сливаются (местный побеждает), `standalone` — автономный сервер,
    /// строка с `File=` — файловая база, иначе кластер.
    /// rule: INV.WIRE.INFOBASE-CREATE-ONLY-CREATES-AN-ABSENT-ONE
    #[test]
    fn the_origin_kind_follows_the_merged_infobase_section() {
        for (base, local, kind) in [
            ("workPath: build\n", None, OriginKind::Undeclared),
            (
                "infobase: {connection: 'File=ib'}\n",
                None,
                OriginKind::File,
            ),
            (
                "workPath: build\n",
                Some("infobases: {origin: {connection: 'File=\"/abs/ib\"'}}\n"),
                OriginKind::File,
            ),
            (
                "infobase: {connection: 'File=ib'}\n",
                Some("infobases: {origin: {connection: 'Srvr=srv;Ref=demo'}}\n"),
                OriginKind::Cluster,
            ),
            (
                "workPath: build\n",
                Some(
                    "infobases: {origin: {connection: 'Srvr=srv:1541;Ref=demo', standalone: {}}}\n",
                ),
                OriginKind::Standalone,
            ),
        ] {
            let root = workspace(base, local);
            assert_eq!(origin_kind(root.path()).unwrap(), kind, "{base} {local:?}");
        }
        // Разбор строки — как у раннера: пробелы вокруг ключа и регистр не
        // важны, часть без `=` делает строку не файловой.
        for (connection, file) in [
            ("File = build/ib", true),
            ("fIlE  =  'db'", true),
            ("File=ib;", true),
            ("File=ib;junk", false),
            ("File=", false),
            ("Srvr=srv;Ref=demo", false),
            ("/F /abs/ib", true),
        ] {
            assert_eq!(declares_a_file_infobase(connection), file, "{connection}");
        }
    }

    #[test]
    fn unknown_runner_versions_are_not_probed_by_executing_an_operation() {
        assert!(check_version(VERSION).is_ok());
        assert!(check_version("0.15.3").is_ok());
        for version in [
            "0.9.0",
            "0.11.3",
            "0.11.4",
            "0.12.0",
            "0.13.0",
            "0.14.0",
            "0.15.0",
            "0.15.1",
            "0.15.2",
            "0.15.3-rc.1",
            "1.0.0",
            "1.0.0-rc.1",
            "",
        ] {
            assert!(check_version(version).is_err());
        }
    }
}
