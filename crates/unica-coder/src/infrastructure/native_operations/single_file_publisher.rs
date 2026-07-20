//! Failure-atomic publication of one exact file payload.
//!
//! Publication locks coordinate Unica processes that use this protocol. They
//! remain held from authoritative inspection through commit and cleanup. This
//! module deliberately does not claim power-loss durability.

use fs2::FileExt;
use sha2::{Digest, Sha256};
#[cfg(test)]
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, ErrorKind, Read, Seek, SeekFrom, Write};
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(test)]
use std::sync::{mpsc::Sender, Barrier};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, TryLockError, Weak};

use crate::infrastructure::platform::filesystem::{
    hard_link_count, install_file_no_clobber, metadata_is_link_or_reparse_point,
    path_lock_identity, portable_permissions, prepare_file_for_removal, replace_file_atomically,
    restrict_stage_to_owner, PortablePermissions,
};

const STAGE_ATTEMPTS: usize = 16;
static STAGE_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static PUBLICATION_PROCESS_LOCKS: OnceLock<Mutex<HashMap<String, Weak<Mutex<()>>>>> =
    OnceLock::new();

#[derive(Debug, Clone, Copy)]
pub(crate) enum PublishMode<'a> {
    CreateOnly,
    ReplaceExisting { expected_preimage: &'a [u8] },
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PublishRequest<'a> {
    pub(crate) target: &'a Path,
    pub(crate) replacement: &'a [u8],
    pub(crate) mode: PublishMode<'a>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PublishEffect {
    Created,
    Replaced,
    Unchanged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PublishReport {
    pub(crate) effect: PublishEffect,
    pub(crate) cleanup_warnings: Vec<CleanupWarning>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CleanupWarning {
    pub(crate) path: PathBuf,
    pub(crate) message: String,
}

impl fmt::Display for CleanupWarning {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.path.display(), self.message)
    }
}

#[derive(Debug)]
pub(crate) struct PublishError {
    kind: PublishErrorKind,
    cleanup_warnings: Vec<CleanupWarning>,
}

impl PublishError {
    fn new(kind: PublishErrorKind) -> Self {
        Self {
            kind,
            cleanup_warnings: Vec::new(),
        }
    }

    fn io(phase: PublishPhase, path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::new(PublishErrorKind::Io {
            phase,
            path: path.into(),
            source,
        })
    }

    pub(crate) fn kind(&self) -> &PublishErrorKind {
        &self.kind
    }

    pub(crate) fn cleanup_warnings(&self) -> &[CleanupWarning] {
        &self.cleanup_warnings
    }

    fn attach_cleanup_warning(&mut self, warning: CleanupWarning) {
        self.cleanup_warnings.push(warning);
    }
}

impl fmt::Display for PublishError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            PublishErrorKind::InvalidTarget { target } => {
                write!(
                    formatter,
                    "invalid publication target: {}",
                    target.display()
                )
            }
            PublishErrorKind::AlreadyExists { target } => write!(
                formatter,
                "create-only publication target already exists: {}",
                target.display()
            ),
            PublishErrorKind::MissingTarget { target } => {
                write!(
                    formatter,
                    "replacement target is missing: {}",
                    target.display()
                )
            }
            PublishErrorKind::LinkOrReparsePoint { target } => write!(
                formatter,
                "publication target is a link or reparse point: {}",
                target.display()
            ),
            PublishErrorKind::NonRegular { target } => write!(
                formatter,
                "publication target is not a regular file: {}",
                target.display()
            ),
            PublishErrorKind::ReadOnly { target } => {
                write!(
                    formatter,
                    "publication target is read-only: {}",
                    target.display()
                )
            }
            PublishErrorKind::MultipleHardLinks { target, count } => write!(
                formatter,
                "publication target has {count} hard links: {}",
                target.display()
            ),
            PublishErrorKind::StalePreimage { target } => write!(
                formatter,
                "publication target differs from the expected preimage: {}",
                target.display()
            ),
            PublishErrorKind::MetadataChanged { target } => write!(
                formatter,
                "publication target metadata changed before commit: {}",
                target.display()
            ),
            PublishErrorKind::StageCollisionsExhausted { target, attempts } => write!(
                formatter,
                "could not reserve a stage for {} after {attempts} attempts",
                target.display()
            ),
            PublishErrorKind::Io {
                phase,
                path,
                source,
            } => write!(
                formatter,
                "publication I/O failed during {phase} for {}: {source}",
                path.display()
            ),
        }?;
        if !self.cleanup_warnings.is_empty() {
            write!(formatter, "; cleanup warnings: ")?;
            for (index, warning) in self.cleanup_warnings.iter().enumerate() {
                if index > 0 {
                    write!(formatter, "; ")?;
                }
                write!(formatter, "{warning}")?;
            }
        }
        Ok(())
    }
}

impl Error for PublishError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match &self.kind {
            PublishErrorKind::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub(crate) enum PublishErrorKind {
    InvalidTarget {
        target: PathBuf,
    },
    AlreadyExists {
        target: PathBuf,
    },
    MissingTarget {
        target: PathBuf,
    },
    LinkOrReparsePoint {
        target: PathBuf,
    },
    NonRegular {
        target: PathBuf,
    },
    ReadOnly {
        target: PathBuf,
    },
    MultipleHardLinks {
        target: PathBuf,
        count: u64,
    },
    StalePreimage {
        target: PathBuf,
    },
    MetadataChanged {
        target: PathBuf,
    },
    StageCollisionsExhausted {
        target: PathBuf,
        attempts: usize,
    },
    Io {
        phase: PublishPhase,
        path: PathBuf,
        source: io::Error,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PublishPhase {
    Inspect,
    Lock,
    Stage,
    Write,
    Flush,
    Sync,
    Permissions,
    Validate,
    Recheck,
    Commit,
    Cleanup,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PublishFailpoint {
    Write,
    Flush,
    Sync,
    Permissions,
    Validate,
    Recheck,
    Commit,
    Cleanup,
}

#[cfg(test)]
impl PublishFailpoint {
    fn phase(self) -> PublishPhase {
        match self {
            Self::Write => PublishPhase::Write,
            Self::Flush => PublishPhase::Flush,
            Self::Sync => PublishPhase::Sync,
            Self::Permissions => PublishPhase::Permissions,
            Self::Validate => PublishPhase::Validate,
            Self::Recheck => PublishPhase::Recheck,
            Self::Commit => PublishPhase::Commit,
            Self::Cleanup => PublishPhase::Cleanup,
        }
    }
}

#[cfg(test)]
#[derive(Clone)]
struct PublicationLockPause {
    acquired: Arc<Barrier>,
    release: Arc<Barrier>,
}

#[cfg(test)]
type BeforeCommitHook = Box<dyn FnOnce(&Path)>;

#[cfg(test)]
thread_local! {
    static TEST_PUBLISH_FAILPOINTS: RefCell<Vec<PublishFailpoint>> = const { RefCell::new(Vec::new()) };
    static TEST_BEFORE_COMMIT_HOOK: RefCell<Option<BeforeCommitHook>> = const { RefCell::new(None) };
    static TEST_PUBLICATION_LOCK_PAUSE: RefCell<Option<PublicationLockPause>> = const { RefCell::new(None) };
    static TEST_PUBLICATION_LOCK_CONTENDED: RefCell<Option<Sender<()>>> = const { RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn with_publish_failpoints<T>(
    failpoints: &[PublishFailpoint],
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Vec<PublishFailpoint>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_PUBLISH_FAILPOINTS.with(|slot| {
                slot.replace(std::mem::take(&mut self.0));
            });
        }
    }

    let previous = TEST_PUBLISH_FAILPOINTS.with(|slot| slot.replace(failpoints.to_vec()));
    let _reset = Reset(previous);
    action()
}

#[cfg(test)]
pub(crate) fn with_before_commit_hook<T>(
    hook: impl FnOnce(&Path) + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<BeforeCommitHook>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_BEFORE_COMMIT_HOOK.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }

    let previous = TEST_BEFORE_COMMIT_HOOK.with(|slot| slot.replace(Some(Box::new(hook))));
    let _reset = Reset(previous);
    action()
}

#[cfg(test)]
pub(crate) fn with_publication_lock_pause<T>(
    acquired: Arc<Barrier>,
    release: Arc<Barrier>,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<PublicationLockPause>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_PUBLICATION_LOCK_PAUSE.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }

    let pause = PublicationLockPause { acquired, release };
    let previous = TEST_PUBLICATION_LOCK_PAUSE.with(|slot| slot.replace(Some(pause)));
    let _reset = Reset(previous);
    action()
}

#[cfg(test)]
pub(crate) fn with_publication_lock_contention_signal<T>(
    sender: Sender<()>,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<Sender<()>>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_PUBLICATION_LOCK_CONTENDED.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }

    let previous = TEST_PUBLICATION_LOCK_CONTENDED.with(|slot| slot.replace(Some(sender)));
    let _reset = Reset(previous);
    action()
}

impl fmt::Display for PublishPhase {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Inspect => "inspect",
            Self::Lock => "lock",
            Self::Stage => "stage",
            Self::Write => "write",
            Self::Flush => "flush",
            Self::Sync => "sync",
            Self::Permissions => "permissions",
            Self::Validate => "validate",
            Self::Recheck => "recheck",
            Self::Commit => "commit",
            Self::Cleanup => "cleanup",
        };
        formatter.write_str(name)
    }
}

/// Proof that the current callback owns the ordered lock set for these targets.
pub(crate) struct PublicationLockToken<'scope> {
    allowed_identities: HashSet<String>,
    _scope: PhantomData<&'scope ()>,
}

/// Acquire process-local and cross-process advisory locks in stable order.
pub(crate) fn with_publication_locks<T>(
    targets: &[PathBuf],
    action: impl FnOnce(&PublicationLockToken<'_>) -> T,
) -> Result<T, PublishError> {
    let mut identities = targets
        .iter()
        .map(|target| publication_identity(target))
        .collect::<Result<Vec<_>, _>>()?;
    identities.sort();
    identities.dedup();

    let process_locks = publication_process_locks(&identities);
    let process_guards = process_locks
        .iter()
        .map(|lock| lock_publication_process_mutex(lock))
        .collect::<Vec<MutexGuard<'_, ()>>>();
    let file_locks = acquire_publication_file_locks(&identities)?;
    let token = PublicationLockToken {
        allowed_identities: identities.into_iter().collect(),
        _scope: PhantomData,
    };

    pause_after_publication_locks();
    let result = action(&token);

    // Both guard layers intentionally remain alive through the callback. Lock
    // files are persistent because removing one races with existing waiters.
    drop(file_locks);
    drop(process_guards);
    Ok(result)
}

pub(crate) enum PreparedPublication<'request, 'lock, 'scope> {
    Unchanged,
    Create(PreparedCreate<'request, 'lock, 'scope>),
    Replace(PreparedReplace<'request, 'lock, 'scope>),
}

impl PreparedPublication<'_, '_, '_> {
    pub(crate) fn commit(self) -> Result<PublishReport, PublishError> {
        match self {
            Self::Unchanged => Ok(PublishReport {
                effect: PublishEffect::Unchanged,
                cleanup_warnings: Vec::new(),
            }),
            Self::Create(prepared) => prepared.commit(),
            Self::Replace(prepared) => prepared.commit(),
        }
    }

    #[allow(dead_code, reason = "used by the upcoming transaction integration")]
    pub(crate) fn discard(self) -> Vec<CleanupWarning> {
        match self {
            Self::Unchanged => Vec::new(),
            Self::Create(prepared) => prepared.discard(),
            Self::Replace(prepared) => prepared.discard(),
        }
    }
}

pub(crate) struct PreparedCreate<'request, 'lock, 'scope> {
    target: &'request Path,
    _lock: &'lock PublicationLockToken<'scope>,
    stage: StageGuard,
}

impl PreparedCreate<'_, '_, '_> {
    pub(crate) fn commit(mut self) -> Result<PublishReport, PublishError> {
        run_before_commit_hook(self.target);
        let recheck = injected_failure(PublishPhase::Recheck, self.target)
            .map_err(|source| PublishError::io(PublishPhase::Recheck, self.target, source))
            .and_then(|()| inspect_create_target(self.target, PublishPhase::Recheck));
        if let Err(mut error) = recheck {
            attach_stage_cleanup(&mut error, &mut self.stage);
            return Err(error);
        }
        if let Err(source) = injected_failure(PublishPhase::Commit, self.target) {
            let mut error = PublishError::io(PublishPhase::Commit, self.target, source);
            attach_stage_cleanup(&mut error, &mut self.stage);
            return Err(error);
        }
        if let Err(source) = install_file_no_clobber(&self.stage.path, self.target) {
            let mut error = if source.kind() == ErrorKind::AlreadyExists {
                PublishError::new(PublishErrorKind::AlreadyExists {
                    target: self.target.to_path_buf(),
                })
            } else {
                PublishError::io(PublishPhase::Commit, self.target, source)
            };
            attach_stage_cleanup(&mut error, &mut self.stage);
            return Err(error);
        }

        let cleanup_warnings = self.stage.cleanup().err().into_iter().collect();
        Ok(PublishReport {
            effect: PublishEffect::Created,
            cleanup_warnings,
        })
    }

    #[allow(dead_code, reason = "used by the upcoming transaction integration")]
    pub(crate) fn discard(mut self) -> Vec<CleanupWarning> {
        self.stage.cleanup().err().into_iter().collect()
    }
}

pub(crate) struct PreparedReplace<'request, 'lock, 'scope> {
    target: &'request Path,
    snapshot: ReplaceSnapshot,
    _lock: &'lock PublicationLockToken<'scope>,
    stage: StageGuard,
}

impl PreparedReplace<'_, '_, '_> {
    #[allow(dead_code, reason = "used by the upcoming transaction integration")]
    pub(crate) fn portable_permissions(&self) -> &PortablePermissions {
        &self.snapshot.permissions
    }

    pub(crate) fn commit(mut self) -> Result<PublishReport, PublishError> {
        run_before_commit_hook(self.target);
        let recheck = injected_failure(PublishPhase::Recheck, self.target)
            .map_err(|source| PublishError::io(PublishPhase::Recheck, self.target, source))
            .and_then(|()| {
                recheck_replace_target(self.target, &self.snapshot, PublishPhase::Recheck)
            });
        if let Err(mut error) = recheck {
            attach_stage_cleanup(&mut error, &mut self.stage);
            return Err(error);
        }
        if let Err(source) = injected_failure(PublishPhase::Commit, self.target) {
            let mut error = PublishError::io(PublishPhase::Commit, self.target, source);
            attach_stage_cleanup(&mut error, &mut self.stage);
            return Err(error);
        }
        if let Err(source) = replace_file_atomically(&self.stage.path, self.target) {
            let mut error = PublishError::io(PublishPhase::Commit, self.target, source);
            attach_stage_cleanup(&mut error, &mut self.stage);
            return Err(error);
        }
        self.stage.disarm();
        Ok(PublishReport {
            effect: PublishEffect::Replaced,
            cleanup_warnings: Vec::new(),
        })
    }

    #[allow(dead_code, reason = "used by the upcoming transaction integration")]
    pub(crate) fn discard(mut self) -> Vec<CleanupWarning> {
        self.stage.cleanup().err().into_iter().collect()
    }
}

pub(crate) fn prepare<'request, 'lock, 'scope>(
    lock: &'lock PublicationLockToken<'scope>,
    request: PublishRequest<'request>,
) -> Result<PreparedPublication<'request, 'lock, 'scope>, PublishError> {
    let identity = publication_identity(request.target)?;
    if !lock.allowed_identities.contains(&identity) {
        return Err(PublishError::new(PublishErrorKind::InvalidTarget {
            target: request.target.to_path_buf(),
        }));
    }

    match request.mode {
        PublishMode::CreateOnly => {
            inspect_create_target(request.target, PublishPhase::Inspect)?;
            let stage = create_stage(request.target, request.replacement, None)?;
            Ok(PreparedPublication::Create(PreparedCreate {
                target: request.target,
                _lock: lock,
                stage,
            }))
        }
        PublishMode::ReplaceExisting { expected_preimage } => {
            let snapshot =
                inspect_replace_target(request.target, expected_preimage, PublishPhase::Inspect)?;
            if snapshot.bytes == request.replacement {
                return Ok(PreparedPublication::Unchanged);
            }
            let stage = create_stage(
                request.target,
                request.replacement,
                Some(&snapshot.permissions),
            )?;
            Ok(PreparedPublication::Replace(PreparedReplace {
                target: request.target,
                snapshot,
                _lock: lock,
                stage,
            }))
        }
    }
}

pub(crate) fn publish(request: PublishRequest<'_>) -> Result<PublishReport, PublishError> {
    with_publication_locks(&[request.target.to_path_buf()], |lock| {
        prepare(lock, request)?.commit()
    })?
}

fn publication_identity(target: &Path) -> Result<String, PublishError> {
    let Some(file_name) = target.file_name() else {
        return Err(PublishError::new(PublishErrorKind::InvalidTarget {
            target: target.to_path_buf(),
        }));
    };
    if file_name.is_empty() {
        return Err(PublishError::new(PublishErrorKind::InvalidTarget {
            target: target.to_path_buf(),
        }));
    }

    let parent = target
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let canonical_parent = match fs::canonicalize(parent) {
        Ok(path) => path,
        Err(source)
            if matches!(
                source.kind(),
                ErrorKind::NotFound | ErrorKind::NotADirectory
            ) =>
        {
            return Err(PublishError::new(PublishErrorKind::InvalidTarget {
                target: target.to_path_buf(),
            }));
        }
        Err(source) => return Err(PublishError::io(PublishPhase::Inspect, parent, source)),
    };
    let parent_metadata = fs::metadata(&canonical_parent)
        .map_err(|source| PublishError::io(PublishPhase::Inspect, &canonical_parent, source))?;
    if !parent_metadata.is_dir() {
        return Err(PublishError::new(PublishErrorKind::InvalidTarget {
            target: target.to_path_buf(),
        }));
    }

    Ok(path_lock_identity(&canonical_parent.join(file_name)))
}

fn publication_process_locks(identities: &[String]) -> Vec<Arc<Mutex<()>>> {
    let registry = PUBLICATION_PROCESS_LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut registry = registry
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    registry.retain(|_, lock| lock.strong_count() > 0);
    identities
        .iter()
        .map(|identity| {
            if let Some(lock) = registry.get(identity).and_then(Weak::upgrade) {
                return lock;
            }
            let lock = Arc::new(Mutex::new(()));
            registry.insert(identity.clone(), Arc::downgrade(&lock));
            lock
        })
        .collect()
}

fn lock_publication_process_mutex(lock: &Mutex<()>) -> MutexGuard<'_, ()> {
    match lock.try_lock() {
        Ok(guard) => guard,
        Err(TryLockError::WouldBlock) => {
            signal_publication_lock_contention();
            lock.lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
        }
        Err(TryLockError::Poisoned(error)) => error.into_inner(),
    }
}

fn pause_after_publication_locks() {
    #[cfg(test)]
    TEST_PUBLICATION_LOCK_PAUSE.with(|slot| {
        if let Some(pause) = slot.borrow_mut().take() {
            pause.acquired.wait();
            pause.release.wait();
        }
    });
}

fn signal_publication_lock_contention() {
    #[cfg(test)]
    TEST_PUBLICATION_LOCK_CONTENDED.with(|slot| {
        if let Some(sender) = slot.borrow_mut().take() {
            let _ = sender.send(());
        }
    });
}

fn acquire_publication_file_locks(identities: &[String]) -> Result<Vec<File>, PublishError> {
    identities
        .iter()
        .map(|identity| {
            let path = publication_lock_path(identity);
            let file = open_publication_lock_file(&path)?;
            FileExt::lock_exclusive(&file)
                .map_err(|source| PublishError::io(PublishPhase::Lock, &path, source))?;
            Ok(file)
        })
        .collect()
}

fn publication_lock_path(identity: &str) -> PathBuf {
    let mut hasher = Sha256::new();
    hasher.update(b"unica-single-file-publication-lock-v1\0");
    hasher.update(identity.as_bytes());
    std::env::temp_dir()
        .join("unica-single-file-publication-locks-v1")
        .join(format!("{:x}.lock", hasher.finalize()))
}

fn open_publication_lock_file(path: &Path) -> Result<File, PublishError> {
    let parent = path.parent().ok_or_else(|| {
        PublishError::new(PublishErrorKind::InvalidTarget {
            target: path.to_path_buf(),
        })
    })?;
    fs::create_dir_all(parent)
        .map_err(|source| PublishError::io(PublishPhase::Lock, parent, source))?;
    match OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(file) => Ok(file),
        Err(source) if source.kind() == ErrorKind::AlreadyExists => {
            let metadata = fs::symlink_metadata(path)
                .map_err(|source| PublishError::io(PublishPhase::Lock, path, source))?;
            if metadata_is_link_or_reparse_point(&metadata) || !metadata.is_file() {
                return Err(PublishError::io(
                    PublishPhase::Lock,
                    path,
                    io::Error::new(
                        ErrorKind::InvalidData,
                        "publication lock is not a regular file",
                    ),
                ));
            }
            OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)
                .map_err(|source| PublishError::io(PublishPhase::Lock, path, source))
        }
        Err(source) => Err(PublishError::io(PublishPhase::Lock, path, source)),
    }
}

fn inspect_create_target(target: &Path, phase: PublishPhase) -> Result<(), PublishError> {
    match fs::symlink_metadata(target) {
        Ok(_) => Err(PublishError::new(PublishErrorKind::AlreadyExists {
            target: target.to_path_buf(),
        })),
        Err(source) if source.kind() == ErrorKind::NotFound => Ok(()),
        Err(source) => Err(PublishError::io(phase, target, source)),
    }
}

struct ReplaceSnapshot {
    bytes: Vec<u8>,
    permissions: PortablePermissions,
    hard_link_count: u64,
}

fn inspect_replace_target(
    target: &Path,
    expected_preimage: &[u8],
    phase: PublishPhase,
) -> Result<ReplaceSnapshot, PublishError> {
    inspect_replace_target_against(target, expected_preimage, phase, None)
}

fn recheck_replace_target(
    target: &Path,
    initial: &ReplaceSnapshot,
    phase: PublishPhase,
) -> Result<(), PublishError> {
    inspect_replace_target_against(target, &initial.bytes, phase, Some(initial)).map(|_| ())
}

fn inspect_replace_target_against(
    target: &Path,
    expected_preimage: &[u8],
    phase: PublishPhase,
    initial: Option<&ReplaceSnapshot>,
) -> Result<ReplaceSnapshot, PublishError> {
    let metadata = match fs::symlink_metadata(target) {
        Ok(metadata) => metadata,
        Err(source) if source.kind() == ErrorKind::NotFound => {
            return Err(PublishError::new(PublishErrorKind::MissingTarget {
                target: target.to_path_buf(),
            }));
        }
        Err(source) => return Err(PublishError::io(phase, target, source)),
    };
    if metadata_is_link_or_reparse_point(&metadata) {
        return Err(PublishError::new(PublishErrorKind::LinkOrReparsePoint {
            target: target.to_path_buf(),
        }));
    }
    if !metadata.is_file() {
        return Err(PublishError::new(PublishErrorKind::NonRegular {
            target: target.to_path_buf(),
        }));
    }

    let mut file = File::open(target).map_err(|source| PublishError::io(phase, target, source))?;
    let opened_metadata = file
        .metadata()
        .map_err(|source| PublishError::io(phase, target, source))?;
    let permissions = portable_permissions(&opened_metadata);
    if permissions.readonly() {
        return Err(PublishError::new(PublishErrorKind::ReadOnly {
            target: target.to_path_buf(),
        }));
    }
    let current_hard_link_count =
        hard_link_count(&file).map_err(|source| PublishError::io(phase, target, source))?;
    if current_hard_link_count != 1 {
        return Err(PublishError::new(PublishErrorKind::MultipleHardLinks {
            target: target.to_path_buf(),
            count: current_hard_link_count,
        }));
    }

    let mut current = Vec::new();
    file.read_to_end(&mut current)
        .map_err(|source| PublishError::io(phase, target, source))?;
    if current != expected_preimage {
        return Err(PublishError::new(PublishErrorKind::StalePreimage {
            target: target.to_path_buf(),
        }));
    }
    if initial.is_some_and(|snapshot| {
        snapshot.hard_link_count != current_hard_link_count
            || !snapshot.permissions.matches(&opened_metadata)
    }) {
        return Err(PublishError::new(PublishErrorKind::MetadataChanged {
            target: target.to_path_buf(),
        }));
    }

    Ok(ReplaceSnapshot {
        bytes: current,
        permissions,
        hard_link_count: current_hard_link_count,
    })
}

fn create_stage(
    target: &Path,
    replacement: &[u8],
    final_permissions: Option<&PortablePermissions>,
) -> Result<StageGuard, PublishError> {
    create_stage_with_candidates(target, replacement, final_permissions, || {
        next_stage_path(target)
    })
}

fn create_stage_with_candidates(
    target: &Path,
    replacement: &[u8],
    final_permissions: Option<&PortablePermissions>,
    mut next_candidate: impl FnMut() -> PathBuf,
) -> Result<StageGuard, PublishError> {
    for attempt in 1..=STAGE_ATTEMPTS {
        let path = next_candidate();
        let open = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path);
        let mut file = match open {
            Ok(file) => file,
            Err(source)
                if source.kind() == ErrorKind::AlreadyExists && attempt < STAGE_ATTEMPTS =>
            {
                continue;
            }
            Err(source) if source.kind() == ErrorKind::AlreadyExists => {
                return Err(PublishError::new(
                    PublishErrorKind::StageCollisionsExhausted {
                        target: target.to_path_buf(),
                        attempts: STAGE_ATTEMPTS,
                    },
                ));
            }
            Err(source) => return Err(PublishError::io(PublishPhase::Stage, &path, source)),
        };
        let mut stage = StageGuard::new(path.clone());
        let result = initialize_stage(&mut file, &path, replacement, final_permissions);
        drop(file);
        match result {
            Ok(()) => return Ok(stage),
            Err(mut error) => {
                attach_stage_cleanup(&mut error, &mut stage);
                return Err(error);
            }
        }
    }
    Err(PublishError::new(
        PublishErrorKind::StageCollisionsExhausted {
            target: target.to_path_buf(),
            attempts: STAGE_ATTEMPTS,
        },
    ))
}

fn initialize_stage(
    file: &mut File,
    path: &Path,
    replacement: &[u8],
    final_permissions: Option<&PortablePermissions>,
) -> Result<(), PublishError> {
    injected_failure(PublishPhase::Permissions, path)
        .map_err(|source| PublishError::io(PublishPhase::Permissions, path, source))?;
    let process_default_permissions = file
        .metadata()
        .map(|metadata| portable_permissions(&metadata))
        .map_err(|source| PublishError::io(PublishPhase::Permissions, path, source))?;
    restrict_stage_to_owner(file)
        .map_err(|source| PublishError::io(PublishPhase::Permissions, path, source))?;
    injected_failure(PublishPhase::Write, path)
        .map_err(|source| PublishError::io(PublishPhase::Write, path, source))?;
    file.write_all(replacement)
        .map_err(|source| PublishError::io(PublishPhase::Write, path, source))?;
    injected_failure(PublishPhase::Flush, path)
        .map_err(|source| PublishError::io(PublishPhase::Flush, path, source))?;
    file.flush()
        .map_err(|source| PublishError::io(PublishPhase::Flush, path, source))?;
    injected_failure(PublishPhase::Sync, path)
        .map_err(|source| PublishError::io(PublishPhase::Sync, path, source))?;
    file.sync_all()
        .map_err(|source| PublishError::io(PublishPhase::Sync, path, source))?;
    injected_failure(PublishPhase::Permissions, path)
        .map_err(|source| PublishError::io(PublishPhase::Permissions, path, source))?;
    final_permissions
        .unwrap_or(&process_default_permissions)
        .apply_to(file)
        .map_err(|source| PublishError::io(PublishPhase::Permissions, path, source))?;
    injected_failure(PublishPhase::Sync, path)
        .map_err(|source| PublishError::io(PublishPhase::Sync, path, source))?;
    file.sync_all()
        .map_err(|source| PublishError::io(PublishPhase::Sync, path, source))?;
    injected_failure(PublishPhase::Validate, path)
        .map_err(|source| PublishError::io(PublishPhase::Validate, path, source))?;
    file.seek(SeekFrom::Start(0))
        .map_err(|source| PublishError::io(PublishPhase::Validate, path, source))?;
    let mut actual = Vec::with_capacity(replacement.len());
    file.read_to_end(&mut actual)
        .map_err(|source| PublishError::io(PublishPhase::Validate, path, source))?;
    if actual != replacement {
        return Err(PublishError::io(
            PublishPhase::Validate,
            path,
            io::Error::new(
                ErrorKind::InvalidData,
                "staged bytes differ from replacement",
            ),
        ));
    }
    Ok(())
}

fn next_stage_path(target: &Path) -> PathBuf {
    let parent = target
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut name = OsString::from(".");
    name.push(
        target
            .file_name()
            .unwrap_or_else(|| std::ffi::OsStr::new("publication")),
    );
    name.push(format!(
        ".unica-stage-{}-{}",
        std::process::id(),
        STAGE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    parent.join(name)
}

fn injected_failure(_phase: PublishPhase, _path: &Path) -> io::Result<()> {
    #[cfg(test)]
    {
        let should_fail = TEST_PUBLISH_FAILPOINTS.with(|slot| {
            slot.borrow()
                .iter()
                .any(|failpoint| failpoint.phase() == _phase)
        });
        if should_fail {
            return Err(io::Error::other(format!(
                "injected publication {_phase} failure"
            )));
        }
    }
    Ok(())
}

fn run_before_commit_hook(_target: &Path) {
    #[cfg(test)]
    if let Some(hook) = TEST_BEFORE_COMMIT_HOOK.with(|slot| slot.borrow_mut().take()) {
        hook(_target);
    }
}

fn attach_stage_cleanup(error: &mut PublishError, stage: &mut StageGuard) {
    if let Err(warning) = stage.cleanup() {
        error.attach_cleanup_warning(warning);
    }
}

#[derive(Debug)]
struct StageGuard {
    path: PathBuf,
    armed: bool,
}

impl StageGuard {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    fn cleanup(&mut self) -> Result<(), CleanupWarning> {
        if !self.armed {
            return Ok(());
        }
        match remove_stage(&self.path) {
            Ok(()) => {
                self.armed = false;
                Ok(())
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {
                self.armed = false;
                Ok(())
            }
            Err(error) => Err(CleanupWarning {
                path: self.path.clone(),
                message: error.to_string(),
            }),
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for StageGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = remove_stage(&self.path);
        }
    }
}

fn remove_stage(path: &Path) -> io::Result<()> {
    match prepare_file_for_removal(path) {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    }
    injected_failure(PublishPhase::Cleanup, path)?;
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        create_stage_with_candidates, publish, remove_stage, with_before_commit_hook,
        with_publication_lock_contention_signal, with_publication_lock_pause,
        with_publication_locks, with_publish_failpoints, PublishEffect, PublishErrorKind,
        PublishFailpoint, PublishMode, PublishPhase, PublishRequest,
    };
    use crate::infrastructure::platform::filesystem::{
        hard_link_count, metadata_is_link_or_reparse_point, portable_permissions,
        prepare_file_for_removal,
    };
    use crate::infrastructure::platform::testing::{
        create_file_link_fixture_for_test, set_unix_mode_for_test, unix_mode_for_test,
        FileLinkFixtureOutcome,
    };
    use std::fs;
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{mpsc, Arc, Barrier};
    use std::thread;
    use std::time::Duration;

    static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    #[test]
    fn create_only_publishes_exact_bytes_and_returns_created() {
        let root = unique_temp_root("create-only");
        let target = root.join("created.bin");
        let replacement = b"\0exact\r\nbytes\xff";

        let report = publish(PublishRequest {
            target: &target,
            replacement,
            mode: PublishMode::CreateOnly,
        })
        .expect("create-only publication must succeed");

        assert_eq!(report.effect, PublishEffect::Created);
        assert!(report.cleanup_warnings.is_empty());
        assert_eq!(fs::read(&target).unwrap(), replacement);
        assert!(publication_debris(&root).is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn replace_existing_publishes_exact_bytes_and_returns_replaced() {
        let root = unique_temp_root("replace-existing");
        let target = root.join("existing.bin");
        let original = b"original\r\nbytes";
        let replacement = b"replacement\0bytes\xff";
        fs::write(&target, original).unwrap();

        let report = publish(PublishRequest {
            target: &target,
            replacement,
            mode: PublishMode::ReplaceExisting {
                expected_preimage: original,
            },
        })
        .expect("replacement publication must succeed");

        assert_eq!(report.effect, PublishEffect::Replaced);
        assert!(report.cleanup_warnings.is_empty());
        assert_eq!(fs::read(&target).unwrap(), replacement);
        assert!(publication_debris(&root).is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn identical_replacement_returns_unchanged_without_staging() {
        let root = unique_temp_root("unchanged");
        let target = root.join("existing.bin");
        let original = b"already exact";
        fs::write(&target, original).unwrap();

        let report = publish(PublishRequest {
            target: &target,
            replacement: original,
            mode: PublishMode::ReplaceExisting {
                expected_preimage: original,
            },
        })
        .expect("identical replacement must be a successful no-op");

        assert_eq!(report.effect, PublishEffect::Unchanged);
        assert!(report.cleanup_warnings.is_empty());
        assert_eq!(fs::read(&target).unwrap(), original);
        assert!(publication_debris(&root).is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn stale_preimage_is_rejected_before_staging() {
        let root = unique_temp_root("stale-preimage");
        let target = root.join("existing.bin");
        let current = b"concurrent bytes";
        fs::write(&target, current).unwrap();

        let error = publish(PublishRequest {
            target: &target,
            replacement: b"replacement",
            mode: PublishMode::ReplaceExisting {
                expected_preimage: b"stale bytes",
            },
        })
        .expect_err("stale preimage must be rejected");

        assert!(matches!(
            error.kind(),
            PublishErrorKind::StalePreimage { .. }
        ));
        assert!(error.cleanup_warnings().is_empty());
        assert_eq!(fs::read(&target).unwrap(), current);
        assert!(publication_debris(&root).is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn target_changed_after_staging_is_rejected() {
        let root = unique_temp_root("target-changed-after-staging");
        let target = root.join("existing.bin");
        let original = b"original";
        let concurrent = b"concurrent replacement";
        fs::write(&target, original).unwrap();
        let original_permissions = portable_permissions(&fs::metadata(&target).unwrap());
        let hook_target = target.clone();

        let error = with_before_commit_hook(
            move |hook_path| {
                assert_eq!(hook_path, hook_target);
                fs::write(hook_path, concurrent).unwrap();
            },
            || {
                publish(PublishRequest {
                    target: &target,
                    replacement: b"our replacement",
                    mode: PublishMode::ReplaceExisting {
                        expected_preimage: original,
                    },
                })
            },
        )
        .expect_err("a target changed after staging must be rejected");

        assert!(matches!(
            error.kind(),
            PublishErrorKind::StalePreimage { .. }
        ));
        assert!(error.cleanup_warnings().is_empty());
        assert_eq!(fs::read(&target).unwrap(), concurrent);
        assert!(original_permissions.matches(&fs::metadata(&target).unwrap()));
        assert!(publication_debris(&root).is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn stage_collisions_retry_without_clobbering() {
        let root = unique_temp_root("stage-collision-retry");
        let target = root.join("target.bin");
        let collider = root.join(".target.bin.unica-stage-collider");
        let available = root.join(".target.bin.unica-stage-available");
        let collider_bytes = b"do not clobber";
        fs::write(&collider, collider_bytes).unwrap();
        let collider_permissions = portable_permissions(&fs::metadata(&collider).unwrap());
        let permissions_probe = root.join("permissions-probe.bin");
        fs::write(&permissions_probe, b"probe").unwrap();
        let expected_stage_permissions =
            portable_permissions(&fs::metadata(&permissions_probe).unwrap());
        fs::remove_file(&permissions_probe).unwrap();
        let mut candidates = [collider.clone(), available.clone()].into_iter();

        let mut stage = create_stage_with_candidates(&target, b"replacement", None, || {
            candidates.next().expect("two candidates must be enough")
        })
        .expect("the second unused stage candidate must succeed");

        assert_eq!(stage.path, available);
        assert_eq!(fs::read(&stage.path).unwrap(), b"replacement");
        assert!(expected_stage_permissions.matches(&fs::metadata(&stage.path).unwrap()));
        assert_eq!(fs::read(&collider).unwrap(), collider_bytes);
        assert!(collider_permissions.matches(&fs::metadata(&collider).unwrap()));
        stage.cleanup().expect("stage cleanup must succeed");
        fs::remove_file(&collider).unwrap();
        assert!(publication_debris(&root).is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn stage_collision_exhaustion_is_typed() {
        let root = unique_temp_root("stage-collision-exhaustion");
        let target = root.join("target.bin");
        let colliders = (0..16)
            .map(|index| {
                let path = root.join(format!(".target.bin.unica-stage-collider-{index}"));
                let bytes = format!("collider {index}").into_bytes();
                fs::write(&path, &bytes).unwrap();
                let permissions = portable_permissions(&fs::metadata(&path).unwrap());
                (path, bytes, permissions)
            })
            .collect::<Vec<_>>();
        let mut attempts = 0;

        let error = create_stage_with_candidates(&target, b"replacement", None, || {
            attempts += 1;
            colliders
                .get(attempts - 1)
                .unwrap_or_else(|| panic!("stage creation requested a seventeenth candidate"))
                .0
                .clone()
        })
        .expect_err("sixteen colliding candidates must exhaust stage creation");

        assert_eq!(attempts, 16);
        assert!(matches!(
            error.kind(),
            PublishErrorKind::StageCollisionsExhausted { attempts: 16, .. }
        ));
        assert!(error.cleanup_warnings().is_empty());
        for (collider, bytes, permissions) in &colliders {
            assert_eq!(fs::read(collider).unwrap(), *bytes);
            assert!(permissions.matches(&fs::metadata(collider).unwrap()));
            fs::remove_file(collider).unwrap();
        }
        assert!(publication_debris(&root).is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn precommit_failpoints_preserve_target_and_remove_stage() {
        let cases = [
            (PublishFailpoint::Write, PublishPhase::Write),
            (PublishFailpoint::Flush, PublishPhase::Flush),
            (PublishFailpoint::Sync, PublishPhase::Sync),
            (PublishFailpoint::Permissions, PublishPhase::Permissions),
            (PublishFailpoint::Validate, PublishPhase::Validate),
            (PublishFailpoint::Recheck, PublishPhase::Recheck),
            (PublishFailpoint::Commit, PublishPhase::Commit),
        ];

        for (failpoint, expected_phase) in cases {
            let root = unique_temp_root(&format!("failpoint-{failpoint:?}"));
            let target = root.join("existing.bin");
            let original = b"original";
            fs::write(&target, original).unwrap();
            let original_permissions = portable_permissions(&fs::metadata(&target).unwrap());

            let error = with_publish_failpoints(&[failpoint], || {
                publish(PublishRequest {
                    target: &target,
                    replacement: b"replacement",
                    mode: PublishMode::ReplaceExisting {
                        expected_preimage: original,
                    },
                })
            })
            .expect_err("an injected pre-commit failure must abort publication");

            assert!(
                matches!(
                    error.kind(),
                    PublishErrorKind::Io { phase, .. } if *phase == expected_phase
                ),
                "unexpected typed error for {failpoint:?}: {error:?}"
            );
            assert!(error.cleanup_warnings().is_empty());
            assert_eq!(fs::read(&target).unwrap(), original);
            assert!(original_permissions.matches(&fs::metadata(&target).unwrap()));
            assert!(publication_debris(&root).is_empty());
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn cleanup_failure_is_attached_to_primary_error() {
        let root = unique_temp_root("primary-and-cleanup-failure");
        let target = root.join("existing.bin");
        let original = b"original";
        fs::write(&target, original).unwrap();
        let original_permissions = portable_permissions(&fs::metadata(&target).unwrap());

        let error = with_publish_failpoints(
            &[PublishFailpoint::Write, PublishFailpoint::Cleanup],
            || {
                publish(PublishRequest {
                    target: &target,
                    replacement: b"replacement",
                    mode: PublishMode::ReplaceExisting {
                        expected_preimage: original,
                    },
                })
            },
        )
        .expect_err("the primary write failure must remain the returned error");

        assert!(matches!(
            error.kind(),
            PublishErrorKind::Io {
                phase: PublishPhase::Write,
                ..
            }
        ));
        assert_eq!(error.cleanup_warnings().len(), 1);
        assert!(error.cleanup_warnings()[0]
            .message
            .contains("injected publication cleanup failure"));
        assert_eq!(fs::read(&target).unwrap(), original);
        assert!(original_permissions.matches(&fs::metadata(&target).unwrap()));
        let debris = publication_debris(&root);
        assert_eq!(debris, [error.cleanup_warnings()[0].path.clone()]);
        assert_eq!(fs::read(&debris[0]).unwrap(), b"");
        remove_stage(&debris[0]).expect("manual cleanup after failpoint scope must succeed");
        assert!(publication_debris(&root).is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn committed_create_with_cleanup_failure_returns_warning() {
        let root = unique_temp_root("committed-create-cleanup-warning");
        let target = root.join("created.bin");
        let permissions_probe = root.join("permissions-probe.bin");
        fs::write(&permissions_probe, b"probe").unwrap();
        let expected_permissions = portable_permissions(&fs::metadata(&permissions_probe).unwrap());
        fs::remove_file(&permissions_probe).unwrap();

        let report = with_publish_failpoints(&[PublishFailpoint::Cleanup], || {
            publish(PublishRequest {
                target: &target,
                replacement: b"committed bytes",
                mode: PublishMode::CreateOnly,
            })
        })
        .expect("cleanup after a committed create must be a warning, not an error");

        assert_eq!(report.effect, PublishEffect::Created);
        assert_eq!(report.cleanup_warnings.len(), 1);
        assert!(report.cleanup_warnings[0]
            .message
            .contains("injected publication cleanup failure"));
        assert_eq!(fs::read(&target).unwrap(), b"committed bytes");
        assert!(expected_permissions.matches(&fs::metadata(&target).unwrap()));
        let debris = publication_debris(&root);
        assert_eq!(debris, [report.cleanup_warnings[0].path.clone()]);
        assert_eq!(fs::read(&debris[0]).unwrap(), b"committed bytes");
        assert!(expected_permissions.matches(&fs::metadata(&debris[0]).unwrap()));
        assert_eq!(
            hard_link_count(&fs::File::open(&target).unwrap()).unwrap(),
            2
        );
        remove_stage(&debris[0]).expect("manual cleanup after failpoint scope must succeed");
        assert_eq!(
            hard_link_count(&fs::File::open(&target).unwrap()).unwrap(),
            1
        );
        assert!(publication_debris(&root).is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn panic_in_before_commit_hook_still_removes_stage() {
        let root = unique_temp_root("panic-before-commit");
        let target = root.join("existing.bin");
        let original = b"original";
        fs::write(&target, original).unwrap();
        let original_permissions = portable_permissions(&fs::metadata(&target).unwrap());

        let unwind = catch_unwind(AssertUnwindSafe(|| {
            with_before_commit_hook(
                |_| panic!("injected hook panic"),
                || {
                    let _ = publish(PublishRequest {
                        target: &target,
                        replacement: b"replacement",
                        mode: PublishMode::ReplaceExisting {
                            expected_preimage: original,
                        },
                    });
                },
            );
        }));

        assert!(unwind.is_err());
        assert_eq!(fs::read(&target).unwrap(), original);
        assert!(original_permissions.matches(&fs::metadata(&target).unwrap()));
        assert!(publication_debris(&root).is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failpoint_scope_is_restored_after_unwind() {
        let root = unique_temp_root("failpoint-scope-unwind");
        let target = root.join("existing.bin");
        let original = b"original";
        fs::write(&target, original).unwrap();

        let unwind = catch_unwind(AssertUnwindSafe(|| {
            with_publish_failpoints(&[PublishFailpoint::Write], || {
                panic!("panic inside failpoint scope");
            });
        }));
        assert!(unwind.is_err());

        let report = publish(PublishRequest {
            target: &target,
            replacement: b"replacement",
            mode: PublishMode::ReplaceExisting {
                expected_preimage: original,
            },
        })
        .expect("failpoint scope must be reset while unwinding");

        assert_eq!(report.effect, PublishEffect::Replaced);
        assert!(report.cleanup_warnings.is_empty());
        assert_eq!(fs::read(&target).unwrap(), b"replacement");
        assert!(publication_debris(&root).is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn create_only_detects_target_created_before_commit() {
        let root = unique_temp_root("create-race");
        let target = root.join("created-by-other.bin");
        let hook_target = target.clone();
        let concurrent = b"other writer won";
        let permissions_probe = root.join("permissions-probe.bin");
        fs::write(&permissions_probe, b"probe").unwrap();
        let concurrent_permissions =
            portable_permissions(&fs::metadata(&permissions_probe).unwrap());
        fs::remove_file(&permissions_probe).unwrap();

        let error = with_before_commit_hook(
            move |hook_path| {
                assert_eq!(hook_path, hook_target);
                fs::write(hook_path, concurrent).unwrap();
            },
            || {
                publish(PublishRequest {
                    target: &target,
                    replacement: b"our bytes",
                    mode: PublishMode::CreateOnly,
                })
            },
        )
        .expect_err("create-only must reject a target created before commit");

        assert!(matches!(
            error.kind(),
            PublishErrorKind::AlreadyExists { .. }
        ));
        assert!(error.cleanup_warnings().is_empty());
        assert_eq!(fs::read(&target).unwrap(), concurrent);
        assert!(concurrent_permissions.matches(&fs::metadata(&target).unwrap()));
        assert!(publication_debris(&root).is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn publication_lock_pause_and_contention_signal_are_scoped() {
        let root = unique_temp_root("publication-lock-contention");
        let target = root.join("target.bin");
        let acquired = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let acquired_by_a = acquired.clone();
        let release_a = release.clone();
        let target_a = target.clone();
        let thread_a = thread::spawn(move || {
            with_publication_lock_pause(acquired_by_a, release_a, || {
                with_publication_locks(&[target_a], |_| ())
            })
        });
        acquired.wait();

        let (contended_sender, contended_receiver) = mpsc::channel();
        let target_b = target.clone();
        let thread_b = thread::spawn(move || {
            with_publication_lock_contention_signal(contended_sender, || {
                with_publication_locks(&[target_b], |_| ())
            })
        });
        let contention = contended_receiver.recv_timeout(Duration::from_secs(2));
        release.wait();

        thread_a
            .join()
            .expect("first lock thread must not panic")
            .expect("first lock acquisition must succeed");
        thread_b
            .join()
            .expect("second lock thread must not panic")
            .expect("second lock acquisition must succeed");
        contention.expect("the second thread must signal in-process lock contention");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn replace_preserves_unix_mode_0600() {
        let root = unique_temp_root("preserve-mode-0600");
        let target = root.join("existing.bin");
        let original = b"original";
        let replacement = b"replacement";
        fs::write(&target, original).unwrap();
        let unix_mode_supported = set_unix_mode_for_test(&target, 0o600).unwrap();
        if unix_mode_supported {
            assert_eq!(unix_mode_for_test(&target).unwrap(), Some(0o600));
        }
        let original_permissions = portable_permissions(&fs::metadata(&target).unwrap());

        let report = publish(PublishRequest {
            target: &target,
            replacement,
            mode: PublishMode::ReplaceExisting {
                expected_preimage: original,
            },
        })
        .expect("replacement must preserve the target permissions");

        assert_eq!(report.effect, PublishEffect::Replaced);
        assert_eq!(fs::read(&target).unwrap(), replacement);
        assert!(original_permissions.matches(&fs::metadata(&target).unwrap()));
        if unix_mode_supported {
            assert_eq!(unix_mode_for_test(&target).unwrap(), Some(0o600));
        }
        assert!(publication_debris(&root).is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn read_only_target_is_rejected_unchanged() {
        let root = unique_temp_root("read-only");
        let target = root.join("existing.bin");
        let original = b"original";
        fs::write(&target, original).unwrap();
        let unix_mode_supported = set_unix_mode_for_test(&target, 0o400).unwrap();
        if unix_mode_supported {
            assert_eq!(unix_mode_for_test(&target).unwrap(), Some(0o400));
        } else {
            let mut permissions = fs::metadata(&target).unwrap().permissions();
            permissions.set_readonly(true);
            fs::set_permissions(&target, permissions).unwrap();
        }
        let original_permissions = portable_permissions(&fs::metadata(&target).unwrap());

        let error = publish(PublishRequest {
            target: &target,
            replacement: original,
            mode: PublishMode::ReplaceExisting {
                expected_preimage: original,
            },
        })
        .expect_err("read-only replacement target must be rejected");

        assert!(matches!(error.kind(), PublishErrorKind::ReadOnly { .. }));
        assert!(error.cleanup_warnings().is_empty());
        assert_eq!(fs::read(&target).unwrap(), original);
        assert!(original_permissions.matches(&fs::metadata(&target).unwrap()));
        if unix_mode_supported {
            assert_eq!(unix_mode_for_test(&target).unwrap(), Some(0o400));
        }
        assert!(publication_debris(&root).is_empty());
        prepare_file_for_removal(&target).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn link_or_reparse_target_is_rejected_without_touching_referent() {
        let root = unique_temp_root("link-or-reparse");
        let referent = root.join("referent.bin");
        let target = root.join("target.bin");
        let original = b"referent bytes";
        fs::write(&referent, original).unwrap();
        let referent_permissions = portable_permissions(&fs::metadata(&referent).unwrap());
        if !create_file_link_fixture(&referent, &target) {
            fs::remove_dir_all(root).unwrap();
            return;
        }
        let original_link = fs::read_link(&target).unwrap();
        let target_permissions = portable_permissions(&fs::symlink_metadata(&target).unwrap());

        let error = publish(PublishRequest {
            target: &target,
            replacement: b"replacement",
            mode: PublishMode::ReplaceExisting {
                expected_preimage: original,
            },
        })
        .expect_err("link or reparse-point replacement target must be rejected");

        assert!(matches!(
            error.kind(),
            PublishErrorKind::LinkOrReparsePoint { .. }
        ));
        assert!(metadata_is_link_or_reparse_point(
            &fs::symlink_metadata(&target).unwrap()
        ));
        assert_eq!(fs::read_link(&target).unwrap(), original_link);
        assert_eq!(fs::read(&target).unwrap(), original);
        assert_eq!(fs::read(&referent).unwrap(), original);
        assert!(target_permissions.matches(&fs::symlink_metadata(&target).unwrap()));
        assert!(referent_permissions.matches(&fs::metadata(&referent).unwrap()));
        assert!(publication_debris(&root).is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn permission_change_before_commit_is_rejected() {
        let root = unique_temp_root("metadata-changed");
        let target = root.join("existing.bin");
        let original = b"original";
        fs::write(&target, original).unwrap();
        if !set_unix_mode_for_test(&target, 0o600).unwrap() {
            eprintln!(
                "[SKIPPED FIXTURE] this host cannot represent a writable 0600-to-0640 permission change"
            );
            fs::remove_dir_all(root).unwrap();
            return;
        }
        assert_eq!(unix_mode_for_test(&target).unwrap(), Some(0o600));

        let hook_target = target.clone();
        let result = with_before_commit_hook(
            move |hook_path| {
                assert_eq!(hook_path, hook_target);
                assert!(
                    set_unix_mode_for_test(hook_path, 0o640)
                        .expect("Unix mode fixture must remain supported after staging"),
                    "Unix mode fixture must remain supported after staging"
                );
            },
            || {
                publish(PublishRequest {
                    target: &target,
                    replacement: b"replacement",
                    mode: PublishMode::ReplaceExisting {
                        expected_preimage: original,
                    },
                })
            },
        );
        let changed_permissions = portable_permissions(&fs::metadata(&target).unwrap());

        let error = result.expect_err("changed writable permissions must reject the commit");

        assert!(matches!(
            error.kind(),
            PublishErrorKind::MetadataChanged { .. }
        ));
        assert_eq!(fs::read(&target).unwrap(), original);
        assert!(changed_permissions.matches(&fs::metadata(&target).unwrap()));
        assert_eq!(unix_mode_for_test(&target).unwrap(), Some(0o640));
        assert!(publication_debris(&root).is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn non_regular_target_is_rejected() {
        let root = unique_temp_root("non-regular");
        let target = root.join("target-directory");
        let marker = target.join("marker.bin");
        fs::create_dir(&target).unwrap();
        fs::write(&marker, b"marker").unwrap();
        let target_permissions = portable_permissions(&fs::metadata(&target).unwrap());

        let error = publish(PublishRequest {
            target: &target,
            replacement: b"replacement",
            mode: PublishMode::ReplaceExisting {
                expected_preimage: b"unused",
            },
        })
        .expect_err("non-regular replacement target must be rejected");

        assert!(matches!(error.kind(), PublishErrorKind::NonRegular { .. }));
        assert!(target.is_dir());
        assert_eq!(fs::read(&marker).unwrap(), b"marker");
        assert!(target_permissions.matches(&fs::metadata(&target).unwrap()));
        assert!(publication_debris(&root).is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn multiple_hard_links_are_rejected() {
        let root = unique_temp_root("multiple-hard-links");
        let target = root.join("target.bin");
        let alias = root.join("alias.bin");
        let original = b"shared bytes";
        fs::write(&target, original).unwrap();
        fs::hard_link(&target, &alias).unwrap();
        let target_permissions = portable_permissions(&fs::metadata(&target).unwrap());
        let alias_permissions = portable_permissions(&fs::metadata(&alias).unwrap());

        let error = publish(PublishRequest {
            target: &target,
            replacement: b"replacement",
            mode: PublishMode::ReplaceExisting {
                expected_preimage: original,
            },
        })
        .expect_err("multiply-linked replacement target must be rejected");

        assert!(matches!(
            error.kind(),
            PublishErrorKind::MultipleHardLinks { count: 2, .. }
        ));
        assert_eq!(fs::read(&target).unwrap(), original);
        assert_eq!(fs::read(&alias).unwrap(), original);
        assert!(target_permissions.matches(&fs::metadata(&target).unwrap()));
        assert!(alias_permissions.matches(&fs::metadata(&alias).unwrap()));
        assert!(publication_debris(&root).is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn create_only_rejects_every_existing_target_kind() {
        let root = unique_temp_root("create-only-existing-kinds");

        let regular = root.join("regular.bin");
        fs::write(&regular, b"regular").unwrap();
        let regular_permissions = portable_permissions(&fs::metadata(&regular).unwrap());
        assert_create_only_rejected(&regular);
        assert_eq!(fs::read(&regular).unwrap(), b"regular");
        assert!(regular_permissions.matches(&fs::metadata(&regular).unwrap()));

        let directory = root.join("directory");
        let marker = directory.join("marker.bin");
        fs::create_dir(&directory).unwrap();
        fs::write(&marker, b"marker").unwrap();
        let directory_permissions = portable_permissions(&fs::metadata(&directory).unwrap());
        assert_create_only_rejected(&directory);
        assert_eq!(fs::read(&marker).unwrap(), b"marker");
        assert!(directory_permissions.matches(&fs::metadata(&directory).unwrap()));

        let hard_link = root.join("hard-link.bin");
        let hard_link_alias = root.join("hard-link-alias.bin");
        fs::write(&hard_link, b"hard-linked").unwrap();
        fs::hard_link(&hard_link, &hard_link_alias).unwrap();
        let hard_link_permissions = portable_permissions(&fs::metadata(&hard_link).unwrap());
        assert_create_only_rejected(&hard_link);
        assert_eq!(fs::read(&hard_link).unwrap(), b"hard-linked");
        assert_eq!(fs::read(&hard_link_alias).unwrap(), b"hard-linked");
        assert!(hard_link_permissions.matches(&fs::metadata(&hard_link).unwrap()));
        assert!(hard_link_permissions.matches(&fs::metadata(&hard_link_alias).unwrap()));

        let referent = root.join("referent.bin");
        let link = root.join("link.bin");
        fs::write(&referent, b"referent").unwrap();
        let referent_permissions = portable_permissions(&fs::metadata(&referent).unwrap());
        if create_file_link_fixture(&referent, &link) {
            let original_link = fs::read_link(&link).unwrap();
            let link_permissions = portable_permissions(&fs::symlink_metadata(&link).unwrap());
            assert_create_only_rejected(&link);
            assert!(metadata_is_link_or_reparse_point(
                &fs::symlink_metadata(&link).unwrap()
            ));
            assert_eq!(fs::read_link(&link).unwrap(), original_link);
            assert_eq!(fs::read(&link).unwrap(), b"referent");
            assert_eq!(fs::read(&referent).unwrap(), b"referent");
            assert!(link_permissions.matches(&fs::symlink_metadata(&link).unwrap()));
            assert!(referent_permissions.matches(&fs::metadata(&referent).unwrap()));
        }

        assert!(publication_debris(&root).is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    fn assert_create_only_rejected(target: &Path) {
        let error = publish(PublishRequest {
            target,
            replacement: b"replacement",
            mode: PublishMode::CreateOnly,
        })
        .expect_err("every existing target kind must fail create-only publication");

        assert!(matches!(
            error.kind(),
            PublishErrorKind::AlreadyExists { .. }
        ));
        assert!(error.cleanup_warnings().is_empty());
    }

    fn create_file_link_fixture(referent: &Path, link: &Path) -> bool {
        match create_file_link_fixture_for_test(referent, link)
            .expect("unexpected file-link fixture error must fail the test")
        {
            FileLinkFixtureOutcome::Created => true,
            FileLinkFixtureOutcome::Unsupported => {
                eprintln!(
                    "[SKIPPED FIXTURE] this host does not expose a file-link test implementation"
                );
                false
            }
            FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => {
                eprintln!(
                    "[SKIPPED FIXTURE] Windows privilege required to create a file link is unavailable"
                );
                false
            }
        }
    }

    fn unique_temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "unica-single-file-publisher-{name}-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn publication_debris(root: &Path) -> Vec<PathBuf> {
        fs::read_dir(root)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .is_some_and(|name| name.to_string_lossy().contains(".unica-stage-"))
            })
            .collect()
    }
}
