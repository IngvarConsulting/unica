//! The private runner project of a working copy: owner-only directories, the projected configuration and the lock serializing runner invocations.
#![cfg_attr(windows, allow(dead_code))]
#[cfg(any(unix, windows))]
use super::filesystem::hard_link_count;
#[cfg(windows)]
use super::filesystem::{
    create_owner_only_directory_child, create_owner_only_file_child, delete_open_child,
    discard_created_child, open_any_child_for_delete, open_any_child_nofollow,
    open_directory_child_nofollow, open_directory_nofollow, open_regular_child_nofollow,
    opened_child_kind, verify_owner_only_acl, OpenedChildKind,
};
use super::filesystem::{file_identity, metadata_is_link_or_reparse_point, FileIdentity};
use crate::paths::{normalize_leaf_path, normalize_path_identity};
use sha2::{Digest, Sha256};
#[cfg(any(test, feature = "test-support"))]
use std::cell::RefCell;
#[cfg(any(unix, windows))]
use std::io::Read;
use std::{
    env,
    ffi::{OsStr, OsString},
    fs::{self, File},
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
    time::Duration,
};
#[cfg(unix)]
use std::{
    ffi::CString,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::ffi::{OsStrExt, OsStringExt},
    },
};
use unica_work_control::cancellation::CancellationToken;
use uuid::Uuid;
const EFFECTIVE_CONFIG_NAME: &str = "v8project.yaml";
const LOCAL_CONFIG_NAME: &str = "v8project.local.yaml";
/// A stable working-copy config, serialized across every runner invocation.
/// A retained sibling lock directory keeps the ownership namespace alive when
/// the projection is removed, independently for each working copy.
pub struct RunnerProjectStore {
    parent: DirectoryAnchor,
    directory: Option<DirectoryAnchor>,
    path: PathBuf,
    config_path: PathBuf,
    lock_anchor: DirectoryAnchor,
    _ownership: File,
    inputs: Vec<ConfigInputSnapshot>,
}

#[derive(Debug)]
pub enum RunnerProjectLockFailure {
    Cancelled,
    InvalidState(String),
}

impl From<String> for RunnerProjectLockFailure {
    fn from(message: String) -> Self {
        Self::InvalidState(message)
    }
}
impl From<&str> for RunnerProjectLockFailure {
    fn from(message: &str) -> Self {
        Self::InvalidState(message.into())
    }
}
impl std::fmt::Display for RunnerProjectLockFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => formatter
                .write_str("runner project preparation cancelled while waiting for ownership"),
            Self::InvalidState(message) => formatter.write_str(message),
        }
    }
}

impl RunnerProjectStore {
    pub fn open_and_lock(
        directory: &Path,
        cancellation: &CancellationToken,
    ) -> Result<Self, RunnerProjectLockFailure> {
        use fs2::FileExt;
        let directory = if directory.is_absolute() {
            directory.to_path_buf()
        } else {
            env::current_dir()
                .map_err(|error| error.to_string())?
                .join(directory)
        };
        let parent = ensure_private_directory_chain(
            directory.parent().ok_or("runner project has no parent")?,
        )?;
        let path = parent
            .path
            .join(directory.file_name().ok_or("runner project has no name")?);
        let lock_name = OsString::from(format!(
            ".runner-project-lock-{:x}",
            Sha256::digest(
                path.file_name()
                    .expect("runner project name was validated")
                    .as_encoded_bytes()
            )
        ));
        let lock_path = parent.path.join(&lock_name);
        let lock_anchor = ensure_private_directory_chain(&lock_path)?;
        verify_private_owner(&lock_anchor.directory).map_err(|error| error.to_string())?;
        let ownership = crate::platform::filesystem::open_directory_ownership_lock(
            &lock_anchor.directory,
            OsStr::new(".runner-project.lock"),
        )
        .map_err(|error| format!("failed to open runner project ownership: {error}"))?;
        loop {
            if cancellation.is_cancelled() {
                return Err(RunnerProjectLockFailure::Cancelled);
            }
            match ownership.try_lock_exclusive() {
                Ok(()) => break,
                Err(error) if crate::platform::filesystem::is_file_lock_contended(&error) => {
                    #[cfg(any(test, feature = "test-support"))]
                    TEST_PROJECT_LOCK_CONTENDED.with(|slot| {
                        if let Some(hook) = slot.borrow_mut().take() {
                            hook();
                        }
                    });
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => return Err(format!("failed to lock runner project: {error}").into()),
            }
        }
        parent.verify_path_binding()?;
        let mut store = Self {
            parent,
            directory: None,
            config_path: path.join(EFFECTIVE_CONFIG_NAME),
            lock_anchor,
            path,
            _ownership: ownership,
            inputs: Vec::new(),
        };
        if !secure_path_is_absent(&store.path)? {
            store.capture_directory()?;
        }
        Ok(store)
    }

    pub fn directory(&self) -> &Path {
        &self.path
    }

    pub fn config_path(&self) -> &Path {
        &self.config_path
    }

    fn capture_directory(&mut self) -> Result<(), String> {
        let name = self.path.file_name().ok_or("runner project has no name")?;
        let anchor = self
            .parent
            .capture_descendant(Path::new(name), &self.path)?;
        verify_private_owner(&anchor.directory)
            .map_err(|error| format!("runner project is not owner-only: {error}"))?;
        self.inputs = [EFFECTIVE_CONFIG_NAME, LOCAL_CONFIG_NAME]
            .into_iter()
            .map(|name| ConfigInputSnapshot::optional(self.path.join(name)))
            .collect::<Result<Vec<_>, _>>()?;
        self.directory = Some(anchor);
        Ok(())
    }

    fn ensure_directory(&mut self) -> Result<(), String> {
        if self.directory.is_some() {
            return self.verify();
        }
        self.parent.verify_path_binding()?;
        let name = self.path.file_name().ok_or("runner project has no name")?;
        let anchor = self.parent.create_child(name, &self.path)?;
        self.directory = Some(anchor);
        self.inputs = [EFFECTIVE_CONFIG_NAME, LOCAL_CONFIG_NAME]
            .into_iter()
            .map(|name| ConfigInputSnapshot::optional(self.path.join(name)))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(())
    }

    pub fn verify(&self) -> Result<(), String> {
        self.parent.verify_path_binding()?;
        self.lock_anchor.verify_path_binding()?;
        if let Some(anchor) = &self.directory {
            anchor.verify_path_binding()?;
            verify_private_owner(&anchor.directory)
                .map_err(|error| format!("runner project privacy changed: {error}"))?;
            for input in &self.inputs {
                input.recheck()?;
                if matches!(input.state, ConfigInputState::Exact(_)) {
                    let file = open_regular_child_nofollow(
                        &anchor.directory,
                        input.path.file_name().expect("tracked config has a name"),
                    )
                    .map_err(|error| error.to_string())?;
                    verify_private_owner(&file).map_err(|error| error.to_string())?;
                }
            }
        } else if !secure_path_is_absent(&self.path)? {
            return Err("runner project appeared while its ownership was held".into());
        }
        Ok(())
    }

    /// Publishes both layers as one owned projection. During a local-layer
    /// replacement/removal the primary is deliberately unreadable to the
    /// runner's conservative owner probe. Its reader also checks that the
    /// primary did not change while it inspected the local layer.
    pub fn write_serialized_projection(
        &mut self,
        projection: &SerializedDumpProjection,
    ) -> Result<(), String> {
        let primary = projection.primary.as_slice();
        let local = projection.local.as_deref();
        let local_bytes = local;
        self.ensure_directory()?;
        let previous_local = self
            .inputs
            .iter()
            .find(|input| input.path.file_name() == Some(OsStr::new(LOCAL_CONFIG_NAME)))
            .expect("both runner config names are tracked");
        let local_changed = previous_local.raw() != local_bytes;
        let guard_needed =
            local_changed && matches!(previous_local.state, ConfigInputState::Exact(_));
        if guard_needed {
            self.write_config(EFFECTIVE_CONFIG_NAME, &projection.unreadable_guard)?;
            run_runner_projection_checkpoint(
                RunnerProjectionCheckpoint::GuardInstalled,
                &self.path,
            )?;
        }
        if local_changed {
            match local {
                Some(local) => self.write_config(LOCAL_CONFIG_NAME, local)?,
                None => {
                    self.remove_config(LOCAL_CONFIG_NAME)?;
                    run_runner_projection_checkpoint(
                        RunnerProjectionCheckpoint::LocalGap,
                        &self.path,
                    )?;
                }
            }
        }
        // Any refusal above leaves the owned Null guard in place. In
        // particular, never restore a valid primary over an unconfirmed local.
        self.verify()?;
        run_runner_projection_checkpoint(RunnerProjectionCheckpoint::LocalConfirmed, &self.path)?;
        self.write_config(EFFECTIVE_CONFIG_NAME, primary)?;
        self.verify()
    }

    fn write_config(&mut self, name: &str, bytes: &[u8]) -> Result<(), String> {
        if ![EFFECTIVE_CONFIG_NAME, LOCAL_CONFIG_NAME].contains(&name) {
            return Err("runner project only stores its primary and local config".into());
        }
        self.ensure_directory()?;
        let anchor = self.directory.as_ref().expect("runner project was ensured");
        let stage_name = OsString::from(format!(".config-{}", Uuid::new_v4()));
        let stage_path = self.path.join(&stage_name);
        let mut file = create_regular_child_owner_only(&anchor.directory, &stage_name, &stage_path)
            .map_err(|error| format!("failed to create private runner config: {error}"))?;
        let identity = file_identity(&file).map_err(|error| error.to_string())?;
        let result = (|| {
            verify_private_owner(&file).map_err(|error| error.to_string())?;
            file.write_all(bytes).map_err(|error| error.to_string())?;
            file.sync_all().map_err(|error| error.to_string())?;
            self.verify()?;
            let expected = self
                .inputs
                .iter()
                .find(|input| input.path.file_name() == Some(OsStr::new(name)))
                .expect("both config names are tracked");
            publish_private_runner_config(
                anchor,
                &stage_name,
                identity,
                OsStr::new(name),
                expected,
            )?;
            Ok(())
        })();
        if let Err(error) = result {
            let cleanup = if secure_path_is_absent(&stage_path)? {
                Ok(())
            } else {
                unlink_bound_regular_child(&anchor.directory, &stage_name, identity)
            };
            return Err(match cleanup {
                Ok(()) => error,
                Err(cleanup) => format!("{error}; private runner config cleanup failed: {cleanup}"),
            });
        }
        let snapshot = ConfigInputSnapshot::required(self.path.join(name))?;
        *self
            .inputs
            .iter_mut()
            .find(|input| input.path.file_name() == Some(OsStr::new(name)))
            .expect("both runner config names are tracked") = snapshot;
        self.verify()
    }

    fn remove_config(&mut self, name: &str) -> Result<(), String> {
        if ![EFFECTIVE_CONFIG_NAME, LOCAL_CONFIG_NAME].contains(&name) {
            return Err("runner project only stores its primary and local config".into());
        }
        self.verify()?;
        let Some(anchor) = &self.directory else {
            return Ok(());
        };
        let input = self
            .inputs
            .iter_mut()
            .find(|input| input.path.file_name() == Some(OsStr::new(name)))
            .expect("both runner config names are tracked");
        if let ConfigInputState::Exact(snapshot) = &input.state {
            let directory =
                crate::platform::filesystem::RetainedDirectoryCapability::open(&anchor.path)
                    .map_err(|error| error.to_string())?;
            if directory.identity() != anchor.identity {
                return Err("runner project directory changed before config removal".into());
            }
            let file = directory
                .retain_regular_child(OsStr::new(name))
                .map_err(|error| error.to_string())?;
            if file.identity() != snapshot.identity {
                return Err(
                    "runner config changed before removal; replacement was left untouched".into(),
                );
            }
            file.remove_named_identity_relative_for_retained_apply(OsStr::new(&format!(
                ".config-cleanup-{}",
                Uuid::new_v4()
            )))
            .map_err(|error| format!("failed to remove retained runner config: {error}"))?;
            input.state = ConfigInputState::Absent;
        }
        self.verify()
    }

    pub fn remove_projection(&mut self) -> Result<(), String> {
        self.remove_config(EFFECTIVE_CONFIG_NAME)?;
        self.remove_config(LOCAL_CONFIG_NAME)?;
        let Some(anchor) = self.directory.as_ref() else {
            return Ok(());
        };
        if !matches!(TreeSnapshot::capture_target(&self.path)?, TreeSnapshot::Directory { entries, .. } if entries.is_empty())
        {
            return Err(
                "runner project contains an unexpected child; it was left untouched".into(),
            );
        }
        let parent =
            crate::platform::filesystem::RetainedDirectoryCapability::open(&self.parent.path)
                .map_err(|error| error.to_string())?;
        if parent.identity() != self.parent.identity {
            return Err("runner project parent changed before removal".into());
        }
        let directory = parent
            .retain_directory_child(self.path.file_name().ok_or("runner project has no name")?)
            .map_err(|error| error.to_string())?;
        if directory.identity() != anchor.identity {
            return Err(
                "runner project changed before removal; replacement was left untouched".into(),
            );
        }
        directory
            .remove_empty_named_identity_relative_for_retained_apply(OsStr::new(&format!(
                ".runner-project-cleanup-{}",
                Uuid::new_v4()
            )))
            .map_err(|error| format!("failed to remove retained runner project: {error}"))?;
        self.directory.take();
        self.inputs.clear();
        self.verify()
    }
}

fn publish_private_runner_config(
    anchor: &DirectoryAnchor,
    stage_name: &OsStr,
    stage_identity: FileIdentity,
    destination_name: &OsStr,
    expected: &ConfigInputSnapshot,
) -> Result<(), String> {
    use crate::platform::filesystem::RetainedDirectoryCapability;
    let directory =
        RetainedDirectoryCapability::open(&anchor.path).map_err(|error| error.to_string())?;
    if directory.identity() != anchor.identity {
        return Err("runner project directory changed before config publication".into());
    }
    let staged = directory
        .retain_regular_child(stage_name)
        .map_err(|error| error.to_string())?;
    if staged.identity() != stage_identity {
        return Err("private staged config identity changed before publication".into());
    }
    let recovery_name = OsString::from(format!(".config-recovery-{}", Uuid::new_v4()));
    let previous = if let ConfigInputState::Exact(snapshot) = &expected.state {
        let previous = directory
            .retain_regular_child(destination_name)
            .map_err(|error| error.to_string())?;
        if previous.identity() != snapshot.identity {
            return Err("runner config destination changed before publication; replacement was left untouched".into());
        }
        Some(
            directory
                .displace_regular_child_no_replace(destination_name, &previous, &recovery_name)
                .map_err(|error| {
                    format!(
                        "runner config destination could not be retained for replacement: {error}"
                    )
                })?,
        )
    } else {
        None
    };
    let installed = (|| {
        if destination_name == OsStr::new(LOCAL_CONFIG_NAME) && previous.is_some() {
            run_runner_projection_checkpoint(RunnerProjectionCheckpoint::LocalGap, &anchor.path)?;
        }
        directory
            .displace_regular_child_no_replace(stage_name, &staged, destination_name)
            .map_err(|error| error.to_string())
    })();
    if let Err(error) = installed {
        return Err(match previous {
            Some(previous) => match directory.restore_displaced_regular_child_no_replace(&previous, destination_name) {
                Ok(()) => format!("private runner config publication failed; previous config restored: {error}"),
                Err(restore) => format!("private runner config publication failed: {error}; original config retained at {} because restoration was refused: {restore}", anchor.path.join(&recovery_name).display()),
            },
            None => format!("private runner config publication failed: {error}"),
        });
    }
    if let Some(previous) = previous {
        previous.remove_named_identity_relative().map_err(|error| {
            format!(
                "private runner config replaced, but original config cleanup failed at {}: {error}",
                anchor.path.join(recovery_name).display()
            )
        })?;
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunnerProjectionCheckpoint {
    GuardInstalled,
    LocalGap,
    LocalConfirmed,
}

#[cfg(any(test, feature = "test-support"))]
type RunnerProjectionHook = (RunnerProjectionCheckpoint, Box<dyn FnOnce(&Path)>);

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static TEST_RUNNER_PROJECTION_HOOK: RefCell<Option<RunnerProjectionHook>> = const { RefCell::new(None) };
    static TEST_RUNNER_PROJECTION_FAILPOINT: RefCell<Option<RunnerProjectionCheckpoint>> = const { RefCell::new(None) };
}

fn run_runner_projection_checkpoint(
    checkpoint: RunnerProjectionCheckpoint,
    path: &Path,
) -> Result<(), String> {
    #[cfg(any(test, feature = "test-support"))]
    {
        let hook = TEST_RUNNER_PROJECTION_HOOK.with(|slot| {
            let mut slot = slot.borrow_mut();
            if slot
                .as_ref()
                .is_some_and(|(expected, _)| *expected == checkpoint)
            {
                slot.take().map(|(_, hook)| hook)
            } else {
                None
            }
        });
        if let Some(hook) = hook {
            hook(path);
        }
        if TEST_RUNNER_PROJECTION_FAILPOINT.with(|slot| slot.borrow().as_ref() == Some(&checkpoint))
        {
            return Err(format!(
                "injected runner projection failure at {checkpoint:?}"
            ));
        }
    }
    let _ = (checkpoint, path);
    Ok(())
}

fn ensure_private_directory_chain(path: &Path) -> Result<DirectoryAnchor, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata_is_link_or_reparse_point(&metadata) => {
            return Err(format!(
                "private directory resolves to a link or reparse point: {}",
                path.display()
            ));
        }
        Ok(_) => return DirectoryAnchor::capture_exact(&normalize_path_identity(path)?),
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "failed to inspect private directory ancestry: {error}"
            ))
        }
    }
    let parent =
        ensure_private_directory_chain(path.parent().ok_or("private directory has no parent")?)?;
    let path = parent
        .path
        .join(path.file_name().ok_or("private directory has no name")?);
    match parent.create_child(
        path.file_name().ok_or("private directory has no name")?,
        &path,
    ) {
        Ok(anchor) => Ok(anchor),
        Err(error) => match DirectoryAnchor::capture_exact(&path) {
            Ok(anchor) => Ok(anchor),
            Err(_) => Err(error),
        },
    }
}

fn verify_private_owner(file: &File) -> std::io::Result<()> {
    crate::platform::filesystem::verify_owner_only_acl(file)
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static TEST_PROJECT_LOCK_CONTENDED: RefCell<Option<Box<dyn FnOnce()>>> = const { RefCell::new(None) };
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ConfigInputState {
    Exact(SecureFileSnapshot),
    Absent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigInputSnapshot {
    path: PathBuf,
    state: ConfigInputState,
}

impl ConfigInputSnapshot {
    pub fn required(path: PathBuf) -> Result<Self, String> {
        let snapshot = secure_read_regular_file_snapshot(&path, "project config")?;
        Ok(Self {
            path,
            state: ConfigInputState::Exact(snapshot),
        })
    }

    pub fn optional(path: PathBuf) -> Result<Self, String> {
        match secure_read_regular_file_snapshot(&path, "local config overlay") {
            Ok(snapshot) => Ok(Self {
                state: ConfigInputState::Exact(snapshot),
                path,
            }),
            Err(read_error) => {
                if secure_path_is_absent(&path)? {
                    Ok(Self {
                        path: normalize_leaf_path(&path)?,
                        state: ConfigInputState::Absent,
                    })
                } else {
                    Err(read_error)
                }
            }
        }
    }

    pub fn raw(&self) -> Option<&[u8]> {
        match &self.state {
            ConfigInputState::Exact(snapshot) => Some(&snapshot.raw),
            ConfigInputState::Absent => None,
        }
    }

    pub fn recheck(&self) -> Result<(), String> {
        match &self.state {
            ConfigInputState::Exact(expected) => {
                let actual =
                    secure_read_regular_file_snapshot(&self.path, "project config preimage")?;
                if &actual != expected {
                    return Err(format!(
                        "project config changed during full dump; staged output was not published: {}",
                        self.path.display()
                    ));
                }
            }
            ConfigInputState::Absent => match secure_path_is_absent(&self.path) {
                Ok(true) => {}
                Ok(false) => {
                    return Err(format!(
                        "local config overlay appeared during full dump; staged output was not published: {}",
                        self.path.display()
                    ));
                }
                Err(error) => {
                    return Err(format!(
                        "failed to recheck absent local config overlay {}: {error}",
                        self.path.display()
                    ));
                }
            },
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SecureFileSnapshot {
    identity: FileIdentity,
    raw: Vec<u8>,
}

#[cfg(unix)]
fn secure_read_regular_file_snapshot(
    path: &Path,
    role: &str,
) -> Result<SecureFileSnapshot, String> {
    let normalized = normalize_leaf_path(path)?;
    let (parent_path, name) = split_parent_and_name(&normalized)?;
    let parent = open_directory_nofollow(&parent_path).map_err(|error| {
        format!(
            "failed to securely open parent for {role} {}: {error}",
            normalized.display()
        )
    })?;
    run_secure_read_hook(path);
    let mut file = open_regular_child_nofollow(&parent, &name).map_err(|error| {
        let detail = if error.raw_os_error() == Some(libc::ELOOP) {
            "symbolic link or reparse point".to_string()
        } else {
            error.to_string()
        };
        format!(
            "failed to securely open {role} {}: {detail}",
            path.display()
        )
    })?;
    let identity = file_identity(&file).map_err(|error| {
        format!(
            "failed to inspect opened {role} identity {}: {error}",
            path.display()
        )
    })?;
    if hard_link_count(&file).map_err(|error| {
        format!(
            "failed to inspect hard links for {role} {}: {error}",
            path.display()
        )
    })? != 1
    {
        return Err(format!(
            "{role} must have exactly one hard link: {}",
            path.display()
        ));
    }
    let opened = file.metadata().map_err(|error| {
        format!(
            "failed to inspect opened {role} {}: {error}",
            path.display()
        )
    })?;
    let mut raw = Vec::new();
    file.read_to_end(&mut raw)
        .map_err(|error| format!("failed to read {role} {}: {error}", path.display()))?;
    let after = file.metadata().map_err(|error| {
        format!(
            "failed to recheck opened {role} {}: {error}",
            path.display()
        )
    })?;
    if opened.len() != after.len() || opened.modified().ok() != after.modified().ok() {
        return Err(format!("{role} changed while reading: {}", path.display()));
    }
    let rebound = open_regular_child_nofollow(&parent, &name).map_err(|error| {
        format!(
            "failed to rebind {role} name after reading {}: {error}",
            path.display()
        )
    })?;
    let rebound_identity = file_identity(&rebound).map_err(|error| {
        format!(
            "failed to recheck {role} identity {}: {error}",
            path.display()
        )
    })?;
    if rebound_identity != identity {
        return Err(format!(
            "{role} identity changed while reading: {}",
            path.display()
        ));
    }
    run_secure_read_after_hook(path);
    Ok(SecureFileSnapshot { identity, raw })
}

#[cfg(windows)]
fn secure_read_regular_file_snapshot(
    path: &Path,
    role: &str,
) -> Result<SecureFileSnapshot, String> {
    let normalized = normalize_leaf_path(path)?;
    let (parent_path, name) = split_parent_and_name(&normalized)?;
    let parent = open_directory_nofollow(&parent_path).map_err(|error| {
        format!(
            "failed to securely open parent for {role} {}: {error}",
            normalized.display()
        )
    })?;
    let mut file = open_regular_child_nofollow(&parent, &name)
        .map_err(|error| format!("failed to securely open {role} {}: {error}", path.display()))?;
    let identity = file_identity(&file).map_err(|error| {
        format!(
            "failed to inspect opened {role} identity {}: {error}",
            path.display()
        )
    })?;
    if hard_link_count(&file).map_err(|error| {
        format!(
            "failed to inspect hard links for {role} {}: {error}",
            path.display()
        )
    })? != 1
    {
        return Err(format!(
            "{role} must have exactly one hard link: {}",
            path.display()
        ));
    }
    let opened = file.metadata().map_err(|error| {
        format!(
            "failed to inspect opened {role} {}: {error}",
            path.display()
        )
    })?;

    run_secure_read_hook(path);
    let rebound = open_regular_child_nofollow(&parent, &name).map_err(|error| {
        format!(
            "failed to rebind {role} name before reading {}: {error}",
            path.display()
        )
    })?;
    if file_identity(&rebound).map_err(|error| {
        format!(
            "failed to recheck {role} identity {}: {error}",
            path.display()
        )
    })? != identity
    {
        return Err(format!(
            "{role} identity changed before reading: {}",
            path.display()
        ));
    }

    let mut raw = Vec::new();
    file.read_to_end(&mut raw)
        .map_err(|error| format!("failed to read {role} {}: {error}", path.display()))?;
    run_secure_read_after_hook(path);
    let after = file.metadata().map_err(|error| {
        format!(
            "failed to recheck opened {role} {}: {error}",
            path.display()
        )
    })?;
    if opened.len() != after.len() || opened.modified().ok() != after.modified().ok() {
        return Err(format!("{role} changed while reading: {}", path.display()));
    }
    let rebound = open_regular_child_nofollow(&parent, &name).map_err(|error| {
        format!(
            "failed to rebind {role} name after reading {}: {error}",
            path.display()
        )
    })?;
    if file_identity(&rebound).map_err(|error| {
        format!(
            "failed to recheck {role} identity {}: {error}",
            path.display()
        )
    })? != identity
    {
        return Err(format!(
            "{role} identity changed while reading: {}",
            path.display()
        ));
    }
    Ok(SecureFileSnapshot { identity, raw })
}

#[cfg(not(any(unix, windows)))]
fn secure_read_regular_file_snapshot(
    path: &Path,
    role: &str,
) -> Result<SecureFileSnapshot, String> {
    Err(format!(
        "{role} secure no-follow reads are unavailable on this host: {}",
        path.display()
    ))
}

fn split_parent_and_name(path: &Path) -> Result<(PathBuf, OsString), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("path has no parent: {}", path.display()))?;
    let name = path
        .file_name()
        .ok_or_else(|| format!("path has no final component: {}", path.display()))?;
    Ok((parent.to_path_buf(), name.to_os_string()))
}

#[cfg(unix)]
fn secure_path_is_absent(path: &Path) -> Result<bool, String> {
    let normalized = normalize_leaf_path(path)?;
    let (parent_path, name) = split_parent_and_name(&normalized)?;
    let parent = open_directory_nofollow(&parent_path).map_err(|error| {
        format!(
            "failed to securely open parent while checking {}: {error}",
            normalized.display()
        )
    })?;
    let name = CString::new(name.as_bytes()).map_err(|error| {
        format!(
            "path contains an embedded NUL byte {}: {error}",
            normalized.display()
        )
    })?;
    // SAFETY: parent remains open for the call, name is NUL-terminated, and
    // metadata points to writable storage for the duration of fstatat.
    let mut metadata = unsafe { std::mem::zeroed::<libc::stat>() };
    let status = unsafe {
        libc::fstatat(
            parent.as_raw_fd(),
            name.as_ptr(),
            &mut metadata,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if status == 0 {
        return Ok(false);
    }
    let error = std::io::Error::last_os_error();
    if error.kind() == ErrorKind::NotFound {
        Ok(true)
    } else {
        Err(format!(
            "failed to securely inspect {}: {error}",
            normalized.display()
        ))
    }
}

#[cfg(windows)]
fn secure_path_is_absent(path: &Path) -> Result<bool, String> {
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND};

    let normalized = normalize_leaf_path(path)?;
    let (parent_path, name) = split_parent_and_name(&normalized)?;
    let parent = open_directory_nofollow(&parent_path).map_err(|error| {
        format!(
            "failed to securely open parent while checking {}: {error}",
            normalized.display()
        )
    })?;
    match open_any_child_nofollow(&parent, &name) {
        Ok((_child, OpenedChildKind::ReparsePoint)) => Ok(false),
        Ok((_child, _kind)) => Ok(false),
        Err(error)
            if matches!(
                error.raw_os_error(),
                Some(code)
                    if code == ERROR_FILE_NOT_FOUND as i32
                        || code == ERROR_PATH_NOT_FOUND as i32
            ) =>
        {
            Ok(true)
        }
        Err(error) => Err(format!(
            "failed to securely inspect {}: {error}",
            normalized.display()
        )),
    }
}

#[cfg(not(any(unix, windows)))]
fn secure_path_is_absent(path: &Path) -> Result<bool, String> {
    Err(format!(
        "secure no-follow absence checks are unavailable on this host: {}",
        path.display()
    ))
}

#[cfg(unix)]
fn open_directory_nofollow(path: &Path) -> std::io::Result<File> {
    use std::path::Component;

    if !path.is_absolute() {
        return Err(std::io::Error::new(
            ErrorKind::InvalidInput,
            "secure directory path must be absolute",
        ));
    }
    let root = CString::new("/")?;
    // SAFETY: the static root path is NUL-terminated and the returned descriptor is owned below.
    let root_fd = unsafe {
        libc::open(
            root.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if root_fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: root_fd is a newly owned descriptor from open.
    let mut current = unsafe { File::from_raw_fd(root_fd) };
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => {
                current = open_directory_child_nofollow(&current, name)?;
            }
            Component::CurDir => {}
            Component::ParentDir | Component::Prefix(_) => {
                return Err(std::io::Error::new(
                    ErrorKind::InvalidInput,
                    "secure directory path contains a non-normal component",
                ));
            }
        }
    }
    Ok(current)
}

#[cfg(any(unix, windows))]
fn open_directory_relative_nofollow(root: &File, relative: &Path) -> std::io::Result<File> {
    use std::path::Component;

    if relative.is_absolute() {
        return Err(std::io::Error::new(
            ErrorKind::InvalidInput,
            "secure descendant path must be relative",
        ));
    }
    let mut current = root.try_clone()?;
    for component in relative.components() {
        match component {
            Component::Normal(name) => {
                current = open_directory_child_nofollow(&current, name)?;
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(std::io::Error::new(
                    ErrorKind::InvalidInput,
                    "secure descendant path contains a non-normal component",
                ));
            }
        }
    }
    Ok(current)
}

#[cfg(unix)]
fn open_directory_child_nofollow(parent: &File, name: &OsStr) -> std::io::Result<File> {
    let name = CString::new(name.as_bytes())?;
    // SAFETY: parent remains open for the call and name is a live NUL-terminated string.
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        // SAFETY: fd is a newly owned descriptor from openat.
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}

#[cfg(unix)]
fn open_regular_child_nofollow(parent: &File, name: &OsStr) -> std::io::Result<File> {
    let name = CString::new(name.as_bytes())?;
    // SAFETY: parent remains open for the call and name is a live NUL-terminated string.
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: fd is a newly owned descriptor from openat.
    let file = unsafe { File::from_raw_fd(fd) };
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::new(
            ErrorKind::InvalidData,
            "entry is not a regular file",
        ));
    }
    Ok(file)
}

#[cfg(unix)]
fn create_regular_child_owner_only(
    parent: &File,
    name: &OsStr,
    _display_path: &Path,
) -> std::io::Result<File> {
    crate::platform::filesystem::create_owner_only_file_child(parent, name)
}

#[cfg(windows)]
fn create_regular_child_owner_only(
    parent: &File,
    name: &OsStr,
    display_path: &Path,
) -> std::io::Result<File> {
    let created = create_owner_only_file_child(parent, name)?;
    let cleanup_error = |primary: std::io::Error| {
        match discard_created_child(&created) {
        Ok(()) => primary,
        Err(cleanup) => std::io::Error::new(
            primary.kind(),
            format!(
                "{primary}; failed to remove unverified effective config through its created handle: {cleanup}"
            ),
        ),
    }
    };
    let expected_identity = file_identity(&created).map_err(cleanup_error)?;
    run_effective_config_create_hook(display_path);
    verify_owner_only_acl(&created).map_err(cleanup_error)?;
    let rebound = open_regular_child_nofollow(parent, name).map_err(cleanup_error)?;
    let rebound_identity = file_identity(&rebound).map_err(cleanup_error)?;
    if rebound_identity != expected_identity {
        return Err(cleanup_error(std::io::Error::new(
            ErrorKind::InvalidData,
            "private effective config identity changed before verified reopen",
        )));
    }
    Ok(created)
}

#[cfg(not(any(unix, windows)))]
fn create_regular_child_owner_only(
    _parent: &File,
    _name: &OsStr,
    _display_path: &Path,
) -> std::io::Result<File> {
    Err(std::io::Error::new(
        ErrorKind::Unsupported,
        "secure openat config creation is unavailable on this host",
    ))
}

#[cfg(unix)]
fn unlink_child_at(parent: &File, name: &OsStr, flags: libc::c_int) -> std::io::Result<()> {
    let name = CString::new(name.as_bytes())?;
    // SAFETY: parent remains open and name is a live NUL-terminated string.
    let status = unsafe { libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), flags) };
    if status == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(unix)]
fn unlink_directory_child_at(parent: &File, name: &OsStr) -> std::io::Result<()> {
    unlink_child_at(parent, name, libc::AT_REMOVEDIR)
}

#[cfg(unix)]
fn unlink_non_directory_child_at(parent: &File, name: &OsStr) -> std::io::Result<()> {
    unlink_child_at(parent, name, 0)
}

#[derive(Debug)]
struct DirectoryAnchor {
    path: PathBuf,
    identity: FileIdentity,
    directory: File,
}

impl DirectoryAnchor {
    #[cfg(any(unix, windows))]
    fn capture_exact(path: &Path) -> Result<Self, String> {
        if !path.is_absolute() {
            return Err(format!(
                "directory anchor path must already be absolute: {}",
                path.display()
            ));
        }
        let path = path.to_path_buf();
        let directory = open_directory_nofollow(&path).map_err(|error| {
            format!(
                "failed to securely open directory anchor {}: {error}",
                path.display()
            )
        })?;
        let identity = file_identity(&directory).map_err(|error| {
            format!(
                "failed to inspect directory anchor {}: {error}",
                path.display()
            )
        })?;
        Ok(Self {
            path,
            identity,
            directory,
        })
    }

    #[cfg(any(unix, windows))]
    fn capture_descendant(&self, relative: &Path, display_path: &Path) -> Result<Self, String> {
        self.verify_path_binding()?;
        let directory =
            open_directory_relative_nofollow(&self.directory, relative).map_err(|error| {
                format!(
                    "failed to bind dump target parent below retained workspace anchor {}: {error}",
                    display_path.display()
                )
            })?;
        let identity = file_identity(&directory).map_err(|error| {
            format!(
                "failed to inspect workspace-contained directory {}: {error}",
                display_path.display()
            )
        })?;
        self.verify_descendant_identity(relative, identity, display_path)?;
        Ok(Self {
            path: display_path.to_path_buf(),
            identity,
            directory,
        })
    }

    #[cfg(any(unix, windows))]
    fn verify_descendant_identity(
        &self,
        relative: &Path,
        expected_identity: FileIdentity,
        display_path: &Path,
    ) -> Result<(), String> {
        self.verify_path_binding()?;
        let rebound =
            open_directory_relative_nofollow(&self.directory, relative).map_err(|error| {
                format!(
                    "workspace-contained directory could not be rebound without following links {}: {error}",
                    display_path.display()
                )
            })?;
        let actual_identity = file_identity(&rebound).map_err(|error| {
            format!(
                "failed to recheck workspace-contained directory {}: {error}",
                display_path.display()
            )
        })?;
        if actual_identity != expected_identity {
            return Err(format!(
                "workspace-contained directory identity changed during full dump: {}",
                display_path.display()
            ));
        }
        self.verify_path_binding()
    }

    #[cfg(not(any(unix, windows)))]
    fn capture_descendant(&self, _relative: &Path, display_path: &Path) -> Result<Self, String> {
        Err(format!(
            "secure workspace-relative directory anchors are unavailable on this host: {}",
            display_path.display()
        ))
    }

    #[cfg(not(any(unix, windows)))]
    fn verify_descendant_identity(
        &self,
        _relative: &Path,
        _expected_identity: FileIdentity,
        display_path: &Path,
    ) -> Result<(), String> {
        Err(format!(
            "secure workspace-relative containment checks are unavailable on this host: {}",
            display_path.display()
        ))
    }

    #[cfg(not(any(unix, windows)))]
    fn try_clone(&self) -> Result<Self, String> {
        Err(format!(
            "secure directory anchors are unavailable on this host: {}",
            self.path.display()
        ))
    }

    #[cfg(unix)]
    fn create_child(&self, name: &OsStr, display_path: &Path) -> Result<Self, String> {
        self.verify_path_binding()?;
        let directory =
            crate::platform::filesystem::create_owner_only_directory_child(&self.directory, name)
                .map_err(|error| {
                format!(
                    "failed to create private directory {}: {error}",
                    display_path.display()
                )
            })?;
        let identity = file_identity(&directory).map_err(|error| {
            format!(
                "failed to inspect private directory identity {}: {error}",
                display_path.display()
            )
        })?;
        if let Err(error) = self.verify_path_binding() {
            let cleanup = unlink_directory_child_at(&self.directory, name);
            return Err(match cleanup {
                Ok(()) => error,
                Err(cleanup) => format!("{error}; private directory cleanup failed: {cleanup}"),
            });
        }
        Ok(Self {
            path: display_path.to_path_buf(),
            identity,
            directory,
        })
    }

    #[cfg(windows)]
    fn create_child(&self, name: &OsStr, display_path: &Path) -> Result<Self, String> {
        let expected_path = self.path.join(name);
        if display_path != expected_path {
            return Err(format!(
                "private directory display path does not match anchored child {}: {}",
                expected_path.display(),
                display_path.display()
            ));
        }
        self.verify_path_binding()?;
        let directory =
            create_owner_only_directory_child(&self.directory, name).map_err(|error| {
                format!(
                    "failed to create private directory {}: {error}",
                    display_path.display()
                )
            })?;
        let identity = file_identity(&directory).map_err(|error| {
            format!(
                "failed to inspect private directory identity {}; the unverified directory was left untouched: {error}",
                display_path.display()
            )
        })?;
        let cleanup_error = |error: String| {
            match remove_bound_directory_child(&self.directory, name, identity, display_path) {
                Ok(()) => error,
                Err(cleanup) => format!(
                    "{error}; failed to remove private directory created through the retained parent anchor {}: {cleanup}",
                    display_path.display()
                ),
            }
        };
        if let Err(error) = verify_owner_only_acl(&directory) {
            return Err(cleanup_error(format!(
                "failed to verify private directory ACL {}: {error}",
                display_path.display()
            )));
        }
        if let Err(error) = self.verify_path_binding() {
            return Err(cleanup_error(error));
        }
        let rebound = open_directory_child_nofollow(&self.directory, name).map_err(|error| {
            cleanup_error(format!(
                "failed to bind newly created private directory {}: {error}",
                display_path.display()
            ))
        })?;
        let rebound_identity = file_identity(&rebound).map_err(|error| {
            cleanup_error(format!(
                "failed to recheck private directory identity {}: {error}",
                display_path.display()
            ))
        })?;
        if rebound_identity != identity {
            return Err(cleanup_error(format!(
                "private directory identity changed during creation: {}",
                display_path.display()
            )));
        }
        Ok(Self {
            path: display_path.to_path_buf(),
            identity,
            directory,
        })
    }

    #[cfg(not(any(unix, windows)))]
    fn create_child(&self, _name: &OsStr, display_path: &Path) -> Result<Self, String> {
        Err(format!(
            "secure mkdirat private directories are unavailable on this host: {}",
            display_path.display()
        ))
    }

    #[cfg(not(any(unix, windows)))]
    fn capture_exact(path: &Path) -> Result<Self, String> {
        Err(format!(
            "secure directory anchors are unavailable on this host: {}",
            path.display()
        ))
    }

    #[cfg(any(unix, windows))]
    fn verify_path_binding(&self) -> Result<(), String> {
        let directory = open_directory_nofollow(&self.path).map_err(|error| {
            format!(
                "failed to securely reopen directory anchor {}: {error}",
                self.path.display()
            )
        })?;
        let identity = file_identity(&directory).map_err(|error| {
            format!(
                "failed to recheck directory anchor {}: {error}",
                self.path.display()
            )
        })?;
        if identity != self.identity {
            return Err(format!(
                "directory anchor identity changed during full dump: {}",
                self.path.display()
            ));
        }
        Ok(())
    }

    #[cfg(not(any(unix, windows)))]
    fn verify_path_binding(&self) -> Result<(), String> {
        Err(format!(
            "secure directory anchors are unavailable on this host: {}",
            self.path.display()
        ))
    }

    #[cfg(not(any(unix, windows)))]
    fn capture_child_root_identity(
        &self,
        _name: &OsStr,
        display_path: &Path,
    ) -> Result<Option<FileIdentity>, String> {
        Err(format!(
            "secure dump target root identity checks are unavailable on this host: {}",
            display_path.display()
        ))
    }
}

#[cfg(not(any(unix, windows)))]
fn open_bound_cleanup_directory(
    _parent: &File,
    _name: &OsStr,
    _expected_identity: FileIdentity,
    _path: &Path,
) -> Result<Option<DirectoryAnchor>, String> {
    Err("retained cleanup is unavailable on this host".into())
}

#[cfg(not(any(unix, windows)))]
fn remove_known_compartment(
    _parent: Option<&DirectoryAnchor>,
    _retained: &mut Option<DirectoryAnchor>,
    _name: &OsStr,
    _identity: FileIdentity,
    _path: &Path,
    _children: &BTreeMap<OsString, TreeSnapshot>,
) -> Result<(), String> {
    Err("retained cleanup is unavailable on this host".into())
}

#[cfg(not(any(unix, windows)))]
fn remove_empty_bound_directory_child(
    _parent: &File,
    _name: &OsStr,
    _expected_identity: FileIdentity,
    _path: &Path,
) -> Result<(), String> {
    Err("retained cleanup is unavailable on this host".into())
}

#[cfg(unix)]
fn unlink_bound_regular_child(
    parent: &File,
    name: &OsStr,
    expected_identity: FileIdentity,
) -> Result<(), String> {
    let rebound = open_regular_child_nofollow(parent, name)
        .map_err(|error| format!("failed to rebind the private file before cleanup: {error}"))?;
    let actual_identity = file_identity(&rebound)
        .map_err(|error| format!("failed to inspect the private file before cleanup: {error}"))?;
    if actual_identity != expected_identity {
        return Err(
            "private file identity changed before cleanup; the replacement was left untouched"
                .to_string(),
        );
    }
    unlink_non_directory_child_at(parent, name)
        .map_err(|error| format!("secure unlinkat failed: {error}"))
}

#[cfg(windows)]
fn unlink_bound_regular_child(
    parent: &File,
    name: &OsStr,
    expected_identity: FileIdentity,
) -> Result<(), String> {
    let child = open_any_child_for_delete(parent, name)
        .map_err(|error| format!("failed to rebind the private file before cleanup: {error}"))?;
    if opened_child_kind(&child)
        .map_err(|error| format!("failed to classify the private file before cleanup: {error}"))?
        != OpenedChildKind::RegularFile
    {
        return Err(
            "private file changed kind before cleanup; the replacement was left untouched"
                .to_string(),
        );
    }
    let actual_identity = file_identity(&child)
        .map_err(|error| format!("failed to inspect the private file before cleanup: {error}"))?;
    if actual_identity != expected_identity {
        return Err(
            "private file identity changed before cleanup; the replacement was left untouched"
                .to_string(),
        );
    }
    delete_open_child(&child)
        .map_err(|error| format!("secure handle-relative file cleanup failed: {error}"))
}

#[cfg(not(any(unix, windows)))]
fn unlink_bound_regular_child(
    _parent: &File,
    _name: &OsStr,
    _expected_identity: FileIdentity,
) -> Result<(), String> {
    Err("secure descriptor-relative file cleanup is unavailable on this host".to_string())
}

#[cfg(windows)]
fn remove_directory_contents_nofollow(directory: &File, display_path: &Path) -> Result<(), String> {
    for name in crate::platform::filesystem::read_directory_names(directory).map_err(|error| {
        format!(
            "failed to enumerate the retained private directory anchor {}: {error}",
            display_path.display()
        )
    })? {
        let child_path = display_path.join(&name);
        run_directory_cleanup_open_hook(&child_path);
        let child = match open_any_child_for_delete(directory, &name) {
            Ok(child) => child,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(format!(
                    "failed to inspect private entry {} before cleanup: {error}",
                    child_path.display()
                ));
            }
        };
        let identity = file_identity(&child).map_err(|error| {
            format!(
                "failed to inspect private entry identity {} before cleanup: {error}",
                child_path.display()
            )
        })?;
        let kind = opened_child_kind(&child).map_err(|error| {
            format!(
                "failed to classify private entry {} before cleanup: {error}",
                child_path.display()
            )
        })?;

        if kind == OpenedChildKind::Directory {
            let directory_child =
                open_directory_child_nofollow(directory, &name).map_err(|error| {
                    format!(
                        "failed to bind private directory {} before cleanup: {error}",
                        child_path.display()
                    )
                })?;
            if file_identity(&directory_child).map_err(|error| {
                format!(
                    "failed to inspect private directory {} before cleanup: {error}",
                    child_path.display()
                )
            })? != identity
            {
                return Err(format!(
                    "private directory identity changed before cleanup; replacement left untouched: {}",
                    child_path.display()
                ));
            }
            remove_directory_contents_nofollow(&directory_child, &child_path)?;
        } else if kind == OpenedChildKind::Unsupported {
            return Err(format!(
                "private tree contains an unsupported entry that cannot be securely removed: {}",
                child_path.display()
            ));
        }

        let rebound = open_any_child_for_delete(directory, &name).map_err(|error| {
            format!(
                "failed to rebind private entry {} before cleanup: {error}",
                child_path.display()
            )
        })?;
        let rebound_identity = file_identity(&rebound).map_err(|error| {
            format!(
                "failed to recheck private entry {} before cleanup: {error}",
                child_path.display()
            )
        })?;
        let rebound_kind = opened_child_kind(&rebound).map_err(|error| {
            format!(
                "failed to reclassify private entry {} before cleanup: {error}",
                child_path.display()
            )
        })?;
        if rebound_identity != identity || rebound_kind != kind {
            return Err(format!(
                "private entry identity or kind changed before cleanup; replacement left untouched: {}",
                child_path.display()
            ));
        }
        delete_open_child(&rebound).map_err(|error| {
            format!(
                "failed to remove private entry {} through its retained parent anchor: {error}",
                child_path.display()
            )
        })?;
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn remove_directory_contents_nofollow(
    _directory: &File,
    display_path: &Path,
) -> Result<(), String> {
    Err(format!(
        "secure descriptor-relative private cleanup is unavailable on this host: {}",
        display_path.display()
    ))
}

#[cfg(windows)]
fn remove_bound_directory_child(
    parent: &File,
    name: &OsStr,
    expected_identity: FileIdentity,
    display_path: &Path,
) -> Result<(), String> {
    let delete_handle = open_any_child_for_delete(parent, name).map_err(|error| {
        format!(
            "failed to rebind expected private directory {} before cleanup: {error}",
            display_path.display()
        )
    })?;
    if opened_child_kind(&delete_handle).map_err(|error| {
        format!(
            "failed to classify expected private directory {} before cleanup: {error}",
            display_path.display()
        )
    })? != OpenedChildKind::Directory
    {
        return Err(format!(
            "private directory changed kind before cleanup; replacement left untouched: {}",
            display_path.display()
        ));
    }
    if file_identity(&delete_handle).map_err(|error| {
        format!(
            "failed to inspect expected private directory {} before cleanup: {error}",
            display_path.display()
        )
    })? != expected_identity
    {
        return Err(format!(
            "private directory identity changed before cleanup; replacement left untouched: {}",
            display_path.display()
        ));
    }

    let directory = open_directory_child_nofollow(parent, name).map_err(|error| {
        format!(
            "failed to retain expected private directory {} before cleanup: {error}",
            display_path.display()
        )
    })?;
    if file_identity(&directory).map_err(|error| {
        format!(
            "failed to inspect retained private directory {} before cleanup: {error}",
            display_path.display()
        )
    })? != expected_identity
    {
        return Err(format!(
            "private directory identity changed before cleanup; replacement left untouched: {}",
            display_path.display()
        ));
    }
    remove_directory_contents_nofollow(&directory, display_path)?;

    let rebound = open_any_child_for_delete(parent, name).map_err(|error| {
        format!(
            "failed to rebind expected private directory {} after cleanup: {error}",
            display_path.display()
        )
    })?;
    if file_identity(&rebound).map_err(|error| {
        format!(
            "failed to recheck expected private directory {} after cleanup: {error}",
            display_path.display()
        )
    })? != expected_identity
        || opened_child_kind(&rebound).map_err(|error| {
            format!(
                "failed to reclassify expected private directory {} after cleanup: {error}",
                display_path.display()
            )
        })? != OpenedChildKind::Directory
    {
        return Err(format!(
            "private directory identity or kind changed after cleanup; replacement left untouched: {}",
            display_path.display()
        ));
    }
    delete_open_child(&rebound).map_err(|error| {
        format!(
            "failed to remove expected private directory {} through its retained parent anchor: {error}",
            display_path.display()
        )
    })
}

#[cfg(not(any(unix, windows)))]
fn remove_bound_directory_child(
    _parent: &File,
    _name: &OsStr,
    _expected_identity: FileIdentity,
    display_path: &Path,
) -> Result<(), String> {
    Err(format!(
        "secure descriptor-relative private cleanup is unavailable on this host: {}",
        display_path.display()
    ))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeSnapshot {
    Absent,
    Directory {
        identity: FileIdentity,
        entries: Vec<TreeEntrySnapshot>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeEntrySnapshot {
    pub relative_path: PathBuf,
    pub kind: TreeEntryKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeEntryKind {
    Directory {
        identity: FileIdentity,
    },
    File {
        identity: FileIdentity,
        size: u64,
        sha256: [u8; 32],
    },
}

impl TreeSnapshot {
    pub fn capture_target(path: &Path) -> Result<Self, String> {
        capture_tree_target_nofollow(path)
    }
}

#[cfg(unix)]
pub fn capture_tree_target_nofollow(path: &Path) -> Result<TreeSnapshot, String> {
    let normalized = normalize_leaf_path(path)?;
    let (parent_path, name) = split_parent_and_name(&normalized)?;
    let parent = open_directory_nofollow(&parent_path).map_err(|error| {
        format!(
            "failed to securely open dump target parent {}: {error}",
            parent_path.display()
        )
    })?;
    run_tree_open_hook(path);
    capture_tree_child_nofollow(&parent, &name, path)
}

#[cfg(unix)]
fn capture_tree_child_nofollow(
    parent: &File,
    name: &OsStr,
    display_path: &Path,
) -> Result<TreeSnapshot, String> {
    let directory = match open_directory_child_nofollow(parent, name) {
        Ok(directory) => directory,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(TreeSnapshot::Absent),
        Err(error) => {
            let detail = if matches!(
                error.raw_os_error(),
                Some(libc::ELOOP) | Some(libc::ENOTDIR)
            ) {
                "symbolic link, reparse point, or non-directory entry".to_string()
            } else {
                error.to_string()
            };
            return Err(format!(
                "dump target must be a real directory or absent {}: {detail}",
                display_path.display()
            ));
        }
    };
    let identity = file_identity(&directory).map_err(|error| {
        format!(
            "failed to inspect dump target directory identity {}: {error}",
            display_path.display()
        )
    })?;
    let mut entries = Vec::new();
    capture_directory_snapshot_recursive(&directory, Path::new(""), display_path, &mut entries)?;
    let rebound = open_directory_child_nofollow(parent, name).map_err(|error| {
        format!(
            "failed to rebind dump target directory {}: {error}",
            display_path.display()
        )
    })?;
    if file_identity(&rebound).map_err(|error| {
        format!(
            "failed to recheck dump target identity {}: {error}",
            display_path.display()
        )
    })? != identity
    {
        return Err(format!(
            "dump target directory identity changed while snapshotting: {}",
            display_path.display()
        ));
    }
    entries.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(TreeSnapshot::Directory { identity, entries })
}

#[cfg(windows)]
pub fn capture_tree_target_nofollow(path: &Path) -> Result<TreeSnapshot, String> {
    let normalized = normalize_leaf_path(path)?;
    let (parent_path, name) = split_parent_and_name(&normalized)?;
    let parent = open_directory_nofollow(&parent_path).map_err(|error| {
        format!(
            "failed to securely open dump target parent {}: {error}",
            parent_path.display()
        )
    })?;
    run_tree_open_hook(path);
    capture_tree_child_nofollow(&parent, &name, path)
}

#[cfg(windows)]
fn capture_tree_child_nofollow(
    parent: &File,
    name: &OsStr,
    display_path: &Path,
) -> Result<TreeSnapshot, String> {
    let directory = match open_directory_child_nofollow(parent, name) {
        Ok(directory) => directory,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(TreeSnapshot::Absent),
        Err(error) => {
            return Err(format!(
                "dump target must be a real directory or absent {}: {error}",
                display_path.display()
            ));
        }
    };
    let identity = file_identity(&directory).map_err(|error| {
        format!(
            "failed to inspect dump target directory identity {}: {error}",
            display_path.display()
        )
    })?;
    let mut entries = Vec::new();
    capture_directory_snapshot_recursive(&directory, Path::new(""), display_path, &mut entries)?;
    let rebound = open_directory_child_nofollow(parent, name).map_err(|error| {
        format!(
            "failed to rebind dump target directory {}: {error}",
            display_path.display()
        )
    })?;
    if file_identity(&rebound).map_err(|error| {
        format!(
            "failed to recheck dump target identity {}: {error}",
            display_path.display()
        )
    })? != identity
    {
        return Err(format!(
            "dump target directory identity changed while snapshotting: {}",
            display_path.display()
        ));
    }
    entries.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(TreeSnapshot::Directory { identity, entries })
}

#[cfg(not(any(unix, windows)))]
pub fn capture_tree_target_nofollow(path: &Path) -> Result<TreeSnapshot, String> {
    Err(format!(
        "secure no-follow directory snapshots are unavailable on this host: {}",
        path.display()
    ))
}

#[cfg(not(any(unix, windows)))]
fn capture_tree_child_nofollow(
    _parent: &File,
    _name: &OsStr,
    display_path: &Path,
) -> Result<TreeSnapshot, String> {
    Err(format!(
        "secure no-follow directory snapshots are unavailable on this host: {}",
        display_path.display()
    ))
}

#[cfg(windows)]
fn capture_directory_snapshot_recursive(
    directory: &File,
    relative_root: &Path,
    display_root: &Path,
    entries: &mut Vec<TreeEntrySnapshot>,
) -> Result<(), String> {
    let initial_names =
        crate::platform::filesystem::read_directory_names(directory).map_err(|error| {
            format!(
                "failed to securely enumerate {}: {error}",
                display_root.join(relative_root).display()
            )
        })?;
    for name in &initial_names {
        let relative = relative_root.join(name);
        let display_path = display_root.join(&relative);
        let (opened, opened_kind) = open_any_child_nofollow(directory, name).map_err(|error| {
            format!(
                "dump tree contains an unsupported entry {}: {error}",
                display_path.display()
            )
        })?;
        let identity = file_identity(&opened).map_err(|error| {
            format!(
                "failed to inspect entry identity {}: {error}",
                display_path.display()
            )
        })?;
        if opened_kind == OpenedChildKind::Directory {
            let child = open_directory_child_nofollow(directory, name).map_err(|error| {
                format!(
                    "failed to securely bind directory {}: {error}",
                    display_path.display()
                )
            })?;
            if file_identity(&child).map_err(|error| {
                format!(
                    "failed to inspect directory identity {}: {error}",
                    display_path.display()
                )
            })? != identity
            {
                return Err(format!(
                    "dump tree directory identity changed while opening: {}",
                    display_path.display()
                ));
            }
            entries.push(TreeEntrySnapshot {
                relative_path: relative.clone(),
                kind: TreeEntryKind::Directory { identity },
            });
            capture_directory_snapshot_recursive(&child, &relative, display_root, entries)?;
            let rebound = open_directory_child_nofollow(directory, name).map_err(|error| {
                format!(
                    "failed to rebind directory {}: {error}",
                    display_path.display()
                )
            })?;
            if file_identity(&rebound).map_err(|error| {
                format!(
                    "failed to recheck directory identity {}: {error}",
                    display_path.display()
                )
            })? != identity
            {
                return Err(format!(
                    "dump tree directory identity changed while snapshotting: {}",
                    display_path.display()
                ));
            }
            continue;
        }

        if opened_kind != OpenedChildKind::RegularFile {
            return Err(format!(
                "dump tree contains an unsupported entry kind: {}",
                display_path.display()
            ));
        }
        let mut file = open_regular_child_nofollow(directory, name).map_err(|error| {
            format!(
                "dump tree contains an unsupported entry {}: {error}",
                display_path.display()
            )
        })?;
        if file_identity(&file).map_err(|error| {
            format!(
                "failed to inspect file identity {}: {error}",
                display_path.display()
            )
        })? != identity
        {
            return Err(format!(
                "dump tree file identity changed while opening: {}",
                display_path.display()
            ));
        }
        if hard_link_count(&file).map_err(|error| {
            format!(
                "failed to inspect hard links for {}: {error}",
                display_path.display()
            )
        })? != 1
        {
            return Err(format!(
                "dump tree file must have exactly one hard link: {}",
                display_path.display()
            ));
        }
        let before = file.metadata().map_err(|error| {
            format!(
                "failed to inspect opened file {}: {error}",
                display_path.display()
            )
        })?;
        let mut hasher = Sha256::new();
        let mut size = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = file
                .read(&mut buffer)
                .map_err(|error| format!("failed to read {}: {error}", display_path.display()))?;
            if read == 0 {
                break;
            }
            size += read as u64;
            hasher.update(&buffer[..read]);
        }
        run_tree_open_hook(&display_path);
        let after = file.metadata().map_err(|error| {
            format!(
                "failed to recheck opened file {}: {error}",
                display_path.display()
            )
        })?;
        if before.len() != after.len()
            || before.modified().ok() != after.modified().ok()
            || after.len() != size
        {
            return Err(format!(
                "dump tree file changed while snapshotting: {}",
                display_path.display()
            ));
        }
        if hard_link_count(&file).map_err(|error| {
            format!(
                "failed to recheck hard links for {}: {error}",
                display_path.display()
            )
        })? != 1
        {
            return Err(format!(
                "dump tree file hard-link count changed while snapshotting: {}",
                display_path.display()
            ));
        }
        let rebound = open_regular_child_nofollow(directory, name).map_err(|error| {
            format!("failed to rebind file {}: {error}", display_path.display())
        })?;
        if file_identity(&rebound).map_err(|error| {
            format!(
                "failed to recheck file identity {}: {error}",
                display_path.display()
            )
        })? != identity
        {
            return Err(format!(
                "dump tree file identity changed while snapshotting: {}",
                display_path.display()
            ));
        }
        entries.push(TreeEntrySnapshot {
            relative_path: relative,
            kind: TreeEntryKind::File {
                identity,
                size,
                sha256: hasher.finalize().into(),
            },
        });
    }
    let final_names =
        crate::platform::filesystem::read_directory_names(directory).map_err(|error| {
            format!(
                "failed to securely re-enumerate {}: {error}",
                display_root.join(relative_root).display()
            )
        })?;
    if final_names != initial_names {
        return Err(format!(
            "dump tree directory name set changed while snapshotting: {}",
            display_root.join(relative_root).display()
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn capture_directory_snapshot_recursive(
    directory: &File,
    relative_root: &Path,
    display_root: &Path,
    entries: &mut Vec<TreeEntrySnapshot>,
) -> Result<(), String> {
    for name in read_directory_names(directory).map_err(|error| {
        format!(
            "failed to securely enumerate {}: {error}",
            display_root.join(relative_root).display()
        )
    })? {
        let relative = relative_root.join(&name);
        let display_path = display_root.join(&relative);
        match open_directory_child_nofollow(directory, &name) {
            Ok(child) => {
                let identity = file_identity(&child).map_err(|error| {
                    format!(
                        "failed to inspect directory identity {}: {error}",
                        display_path.display()
                    )
                })?;
                entries.push(TreeEntrySnapshot {
                    relative_path: relative.clone(),
                    kind: TreeEntryKind::Directory { identity },
                });
                capture_directory_snapshot_recursive(&child, &relative, display_root, entries)?;
                let rebound = open_directory_child_nofollow(directory, &name).map_err(|error| {
                    format!(
                        "failed to rebind directory {}: {error}",
                        display_path.display()
                    )
                })?;
                if file_identity(&rebound).map_err(|error| {
                    format!(
                        "failed to recheck directory identity {}: {error}",
                        display_path.display()
                    )
                })? != identity
                {
                    return Err(format!(
                        "dump tree directory identity changed while snapshotting: {}",
                        display_path.display()
                    ));
                }
            }
            Err(directory_error)
                if matches!(
                    directory_error.raw_os_error(),
                    Some(libc::ENOTDIR) | Some(libc::EINVAL)
                ) =>
            {
                let mut file = open_regular_child_nofollow(directory, &name).map_err(|error| {
                    let detail = if error.raw_os_error() == Some(libc::ELOOP) {
                        "symbolic link or reparse point".to_string()
                    } else {
                        error.to_string()
                    };
                    format!(
                        "dump tree contains an unsupported entry {}: {detail}",
                        display_path.display()
                    )
                })?;
                let identity = file_identity(&file).map_err(|error| {
                    format!(
                        "failed to inspect file identity {}: {error}",
                        display_path.display()
                    )
                })?;
                let links = hard_link_count(&file).map_err(|error| {
                    format!(
                        "failed to inspect hard links for {}: {error}",
                        display_path.display()
                    )
                })?;
                if links != 1 {
                    return Err(format!(
                        "dump tree file must have exactly one hard link: {}",
                        display_path.display()
                    ));
                }
                let before = file.metadata().map_err(|error| {
                    format!(
                        "failed to inspect opened file {}: {error}",
                        display_path.display()
                    )
                })?;
                let mut hasher = Sha256::new();
                let mut size = 0_u64;
                let mut buffer = [0_u8; 64 * 1024];
                loop {
                    let read = file.read(&mut buffer).map_err(|error| {
                        format!("failed to read {}: {error}", display_path.display())
                    })?;
                    if read == 0 {
                        break;
                    }
                    size += read as u64;
                    hasher.update(&buffer[..read]);
                }
                let after = file.metadata().map_err(|error| {
                    format!(
                        "failed to recheck opened file {}: {error}",
                        display_path.display()
                    )
                })?;
                if before.len() != after.len()
                    || before.modified().ok() != after.modified().ok()
                    || after.len() != size
                {
                    return Err(format!(
                        "dump tree file changed while snapshotting: {}",
                        display_path.display()
                    ));
                }
                let rebound = open_regular_child_nofollow(directory, &name).map_err(|error| {
                    format!("failed to rebind file {}: {error}", display_path.display())
                })?;
                if file_identity(&rebound).map_err(|error| {
                    format!(
                        "failed to recheck file identity {}: {error}",
                        display_path.display()
                    )
                })? != identity
                {
                    return Err(format!(
                        "dump tree file identity changed while snapshotting: {}",
                        display_path.display()
                    ));
                }
                entries.push(TreeEntrySnapshot {
                    relative_path: relative,
                    kind: TreeEntryKind::File {
                        identity,
                        size,
                        sha256: hasher.finalize().into(),
                    },
                });
            }
            Err(error) => {
                let detail = if error.raw_os_error() == Some(libc::ELOOP) {
                    "symbolic link or reparse point".to_string()
                } else {
                    error.to_string()
                };
                return Err(format!(
                    "dump tree contains an unsupported entry {}: {detail}",
                    display_path.display()
                ));
            }
        }
    }
    Ok(())
}

#[cfg(unix)]
fn read_directory_names(directory: &File) -> std::io::Result<Vec<OsString>> {
    // SAFETY: fcntl duplicates the live descriptor; fdopendir assumes ownership of that duplicate.
    let duplicate = unsafe { libc::fcntl(directory.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 0) };
    if duplicate < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: duplicate is a valid owned directory descriptor.
    let stream = unsafe { libc::fdopendir(duplicate) };
    if stream.is_null() {
        let error = std::io::Error::last_os_error();
        // SAFETY: fdopendir failed and did not take ownership of duplicate.
        unsafe {
            libc::close(duplicate);
        }
        return Err(error);
    }
    let mut names = Vec::new();
    loop {
        // SAFETY: stream remains valid until closed below.
        let entry = unsafe { libc::readdir(stream) };
        if entry.is_null() {
            break;
        }
        // SAFETY: d_name is a NUL-terminated array owned by the live dirent.
        let bytes = unsafe { std::ffi::CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
        if bytes == b"." || bytes == b".." {
            continue;
        }
        names.push(OsString::from_vec(bytes.to_vec()));
    }
    // SAFETY: stream is live and owns duplicate.
    if unsafe { libc::closedir(stream) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    names.sort();
    Ok(names)
}

#[cfg(not(any(unix, windows)))]
fn capture_directory_snapshot_recursive(
    _directory: &File,
    _relative_root: &Path,
    display_root: &Path,
    _entries: &mut Vec<TreeEntrySnapshot>,
) -> Result<(), String> {
    Err(format!(
        "secure no-follow directory snapshots are unavailable on this host: {}",
        display_root.display()
    ))
}

#[cfg(not(any(unix, windows)))]
fn rename_prechecked_directory_child_no_replace(
    _source_parent: &DirectoryAnchor,
    _source_name: &OsStr,
    _expected_identity: FileIdentity,
    _destination_parent: &DirectoryAnchor,
    _destination_name: &OsStr,
) -> Result<(), String> {
    Err("prechecked atomic no-clobber directory rename is unavailable on this host".to_string())
}

#[cfg(not(any(unix, windows)))]
fn rename_child_no_replace(
    _source_parent: &DirectoryAnchor,
    _source_name: &OsStr,
    _destination_parent: &DirectoryAnchor,
    _destination_name: &OsStr,
) -> Result<(), String> {
    Err("atomic no-clobber directory rename is unavailable on this host".to_string())
}

#[cfg(any(test, feature = "test-support"))]
type PathHook = Box<dyn FnOnce(&Path)>;
#[cfg(all(any(test, feature = "test-support"), windows))]
type CleanupOpenHook = Box<dyn FnMut(&Path)>;
#[cfg(any(test, feature = "test-support"))]
type TargetedReadHooks = (
    PathBuf,
    Option<Box<dyn FnOnce()>>,
    Option<Box<dyn FnOnce()>>,
);

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static TEST_SECURE_READ_HOOK: RefCell<Option<PathHook>> = const { RefCell::new(None) };
    static TEST_TARGETED_READ_HOOKS: RefCell<Option<TargetedReadHooks>> = const { RefCell::new(None) };
    static TEST_TREE_OPEN_HOOK: RefCell<Option<PathHook>> = const { RefCell::new(None) };
    #[cfg(windows)]
    static TEST_CLEANUP_OPEN_HOOK: RefCell<Option<CleanupOpenHook>> = const { RefCell::new(None) };
    static TEST_PRIVATE_CREATE_HOOK: RefCell<Option<PathHook>> = const { RefCell::new(None) };
    static TEST_EFFECTIVE_CONFIG_PREPARE_HOOK: RefCell<Option<PathHook>> = const { RefCell::new(None) };
    #[cfg(windows)]
    static TEST_EFFECTIVE_CONFIG_CREATE_HOOK: RefCell<Option<PathHook>> = const { RefCell::new(None) };
    static TEST_TARGET_PARENT_CAPTURE_HOOK: RefCell<Option<PathHook>> = const { RefCell::new(None) };
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static TEST_SEMANTIC_VALIDATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(all(any(test, feature = "test-support"), windows))]
pub fn with_effective_config_create_hook<T>(
    hook: impl FnOnce(&Path) + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<PathHook>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_EFFECTIVE_CONFIG_CREATE_HOOK.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }
    let previous =
        TEST_EFFECTIVE_CONFIG_CREATE_HOOK.with(|slot| slot.replace(Some(Box::new(hook))));
    let _reset = Reset(previous);
    action()
}

#[cfg(windows)]
fn run_effective_config_create_hook(_path: &Path) {
    #[cfg(any(test, feature = "test-support"))]
    if let Some(hook) = TEST_EFFECTIVE_CONFIG_CREATE_HOOK.with(|slot| slot.borrow_mut().take()) {
        hook(_path);
    }
}

fn run_secure_read_hook(_path: &Path) {
    #[cfg(any(test, feature = "test-support"))]
    if let Some(hook) = TEST_SECURE_READ_HOOK.with(|slot| slot.borrow_mut().take()) {
        hook(_path);
    }
    #[cfg(any(test, feature = "test-support"))]
    let hook = TEST_TARGETED_READ_HOOKS.with(|slot| {
        let mut slot = slot.borrow_mut();
        let (target, before, _) = slot.as_mut()?;
        if target == _path {
            before.take()
        } else {
            None
        }
    });
    #[cfg(any(test, feature = "test-support"))]
    if let Some(hook) = hook {
        hook();
    }
}

fn run_secure_read_after_hook(_path: &Path) {
    #[cfg(any(test, feature = "test-support"))]
    let hook = TEST_TARGETED_READ_HOOKS.with(|slot| {
        let mut slot = slot.borrow_mut();
        let (target, _, after) = slot.as_mut()?;
        if target == _path {
            after.take()
        } else {
            None
        }
    });
    #[cfg(any(test, feature = "test-support"))]
    if let Some(hook) = hook {
        hook();
    }
}

fn run_tree_open_hook(_path: &Path) {
    #[cfg(any(test, feature = "test-support"))]
    if let Some(hook) = TEST_TREE_OPEN_HOOK.with(|slot| slot.borrow_mut().take()) {
        hook(_path);
    }
}

#[cfg(windows)]
fn run_directory_cleanup_open_hook(_path: &Path) {
    #[cfg(any(test, feature = "test-support"))]
    TEST_CLEANUP_OPEN_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().as_mut() {
            hook(_path);
        }
    });
}

pub struct SerializedDumpProjection {
    primary: Vec<u8>,
    local: Option<Vec<u8>>,
    unreadable_guard: Vec<u8>,
}
impl SerializedDumpProjection {
    pub fn from_serialized(
        primary: Vec<u8>,
        local: Option<Vec<u8>>,
        unreadable_guard: Vec<u8>,
    ) -> Self {
        Self {
            primary,
            local,
            unreadable_guard,
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
impl RunnerProjectStore {
    pub fn test_write_config_bytes(&mut self, name: &str, bytes: &[u8]) -> Result<(), String> {
        self.write_config(name, bytes)
    }
    pub fn test_remove_config(&mut self, name: &str) -> Result<(), String> {
        self.remove_config(name)
    }
}

/// The existing host branch classes used by v8 filename and installation policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostPlatform {
    Windows,
    Linux,
    Other,
}

pub fn current_host_platform() -> HostPlatform {
    #[cfg(windows)]
    {
        HostPlatform::Windows
    }
    #[cfg(target_os = "linux")]
    {
        HostPlatform::Linux
    }
    #[cfg(all(not(windows), not(target_os = "linux")))]
    {
        HostPlatform::Other
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn with_runner_projection_hook<T>(
    checkpoint: RunnerProjectionCheckpoint,
    hook: impl FnOnce(&Path) + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<RunnerProjectionHook>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_RUNNER_PROJECTION_HOOK.with(|slot| slot.replace(self.0.take()));
        }
    }
    let old =
        TEST_RUNNER_PROJECTION_HOOK.with(|slot| slot.replace(Some((checkpoint, Box::new(hook)))));
    let _reset = Reset(old);
    action()
}
#[cfg(any(test, feature = "test-support"))]
pub fn with_runner_projection_failpoint<T>(
    checkpoint: RunnerProjectionCheckpoint,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<RunnerProjectionCheckpoint>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_RUNNER_PROJECTION_FAILPOINT.with(|slot| slot.replace(self.0.take()));
        }
    }
    let old = TEST_RUNNER_PROJECTION_FAILPOINT.with(|slot| slot.replace(Some(checkpoint)));
    let _reset = Reset(old);
    action()
}
#[cfg(any(test, feature = "test-support"))]
pub fn with_test_project_lock_contention<T>(
    hook: impl FnOnce() + 'static,
    run: impl FnOnce() -> T,
) -> T {
    struct Restore(Option<Box<dyn FnOnce()>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            TEST_PROJECT_LOCK_CONTENDED.with(|slot| *slot.borrow_mut() = self.0.take());
        }
    }
    let _restore =
        Restore(TEST_PROJECT_LOCK_CONTENDED.with(|slot| slot.replace(Some(Box::new(hook)))));
    run()
}
#[cfg(any(test, feature = "test-support"))]
pub fn fixture_file_identity(file: &File) -> std::io::Result<FileIdentity> {
    file_identity(file)
}
#[cfg(any(test, feature = "test-support"))]
pub fn fixture_verify_private_owner(file: &File) -> std::io::Result<()> {
    verify_private_owner(file)
}
#[cfg(any(test, feature = "test-support"))]
pub fn set_before_fixture_config_rename_hook(hook: impl FnOnce() + 'static) {
    crate::platform::filesystem::set_before_identity_bound_no_replace_rename_hook(hook)
}

#[cfg(any(test, feature = "test-support"))]
pub mod test_support {
    #[cfg(windows)]
    pub use super::with_effective_config_create_hook;
    pub use super::{
        fixture_file_identity, fixture_verify_private_owner, set_before_fixture_config_rename_hook,
        with_runner_projection_failpoint, with_runner_projection_hook,
        with_test_project_lock_contention, RunnerProjectionCheckpoint,
    };
}
