//! Physical operations for the execution-store prototype.
//!
//! This module deliberately does not use the rollback-oriented retained
//! publishers. Once publication is attempted, an error preserves evidence and
//! never moves an unexpected destination back to the stage name. The owner,
//! not these primitives, supplies serialization and process-lifetime fencing.

use super::{FileIdentity, RetainedDirectoryCapability, RetainedRegularFileCapability};
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{self, Write};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PublicationCertainty {
    NeverPublished,
    MayHavePublished,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum PublishMode {
    CreateNew,
    Replace { expected: FileIdentity },
}

/// An error owns the candidate descriptor but exposes no writable handle or
/// retry capability. Dropping it closes the handle; it never removes a name.
#[derive(Debug)]
pub(crate) struct PublicationFailure {
    certainty: PublicationCertainty,
    error: io::Error,
    _candidate: File,
}

impl PublicationFailure {
    pub(crate) fn certainty(&self) -> PublicationCertainty {
        self.certainty
    }

    pub(crate) fn io_error(&self) -> &io::Error {
        &self.error
    }
}

impl std::fmt::Display for PublicationFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.error.fmt(formatter)
    }
}

impl std::error::Error for PublicationFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

/// No implicit create, fallback, or successful no-op on an unsupported host.
/// Windows directory durability remains an explicit prototype acceptance gap.
pub(crate) fn check_durability_support() -> io::Result<()> {
    if cfg!(unix) {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "execution-store durable directory publication is not established on this host",
        ))
    }
}

pub(crate) fn verify_owner_only_directory(
    directory: &RetainedDirectoryCapability,
) -> io::Result<()> {
    directory.validate_named_identity()?;
    super::verify_owner_only_acl(&directory.retained.directory)
}

/// Creation does not confirm the parent's new entry. The namespace protocol
/// must call `flush_directory(parent)` before advancing its bootstrap phase.
pub(crate) fn create_owner_only_directory(
    parent: &RetainedDirectoryCapability,
    name: &OsStr,
) -> io::Result<RetainedDirectoryCapability> {
    check_durability_support()?;
    parent.validate_named_identity()?;
    let directory = super::create_owner_only_directory_child(&parent.retained.directory, name)?;
    let identity = super::file_identity(&directory)?;
    let child = RetainedDirectoryCapability {
        path: parent.path.join(name),
        retained: std::sync::Arc::new(super::RetainedDirectoryCapabilityInner {
            directory,
            identity,
        }),
        parent: Some(std::sync::Arc::new(super::RetainedDirectoryParent {
            directory: parent.clone(),
            name: name.to_os_string(),
        })),
    };
    child.validate_named_identity()?;
    Ok(child)
}

pub(crate) fn flush_directory(directory: &RetainedDirectoryCapability) -> io::Result<()> {
    check_durability_support()?;
    directory.validate_named_identity()?;
    // The support check excludes the old non-Unix successful no-op.
    super::sync_directory(&directory.retained.directory)?;
    directory.validate_named_identity()
}

/// A unique private file. Only the streaming writer is exposed, never its
/// descriptor; consuming seal closes every path to a late writer.
#[derive(Debug)]
pub(crate) struct Stage {
    parent: RetainedDirectoryCapability,
    name: OsString,
    file: File,
    identity: FileIdentity,
    write_failure: Option<io::Error>,
}

impl Stage {
    pub(crate) fn create(parent: &RetainedDirectoryCapability, name: &OsStr) -> io::Result<Self> {
        check_durability_support()?;
        if !is_stage_name(name) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "execution-store staging name must start with .stage.",
            ));
        }
        parent.validate_named_identity()?;
        let file = super::create_owner_only_file_child(&parent.retained.directory, name)?;
        let identity = super::file_identity(&file)?;
        Ok(Self {
            parent: parent.clone(),
            name: name.to_os_string(),
            file,
            identity,
            write_failure: None,
        })
    }

    pub(crate) fn seal(self) -> io::Result<SealedStage> {
        if let Some(error) = self.write_failure {
            return Err(error);
        }
        self.parent.validate_named_identity()?;
        run_hook(PublicationPoint::BeforeStageSync)?;
        self.file.sync_all()?;
        Ok(SealedStage {
            parent: self.parent,
            name: self.name,
            file: self.file,
            identity: self.identity,
        })
    }

    fn remember_failure(&mut self, error: &io::Error) {
        // Write::write_all retries Interrupted. Latching it would make that
        // standard retry loop return Interrupted forever without another I/O.
        if error.kind() != io::ErrorKind::Interrupted && self.write_failure.is_none() {
            self.write_failure = Some(copy_error(error));
        }
    }
}

impl Write for Stage {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if let Some(error) = &self.write_failure {
            return Err(copy_error(error));
        }
        let result = write_stage(&mut self.file, bytes).and_then(|written| {
            if written == 0 && !bytes.is_empty() {
                Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "stage write made no progress",
                ))
            } else {
                Ok(written)
            }
        });
        if let Err(error) = &result {
            self.remember_failure(error);
        }
        result
    }

    fn flush(&mut self) -> io::Result<()> {
        if let Some(error) = &self.write_failure {
            return Err(copy_error(error));
        }
        let result = self.file.flush();
        if let Err(error) = &result {
            self.remember_failure(error);
        }
        result
    }
}

fn copy_error(error: &io::Error) -> io::Error {
    error.raw_os_error().map_or_else(
        || io::Error::new(error.kind(), error.to_string()),
        io::Error::from_raw_os_error,
    )
}

#[cfg(not(all(test, unix)))]
fn write_stage(file: &mut File, bytes: &[u8]) -> io::Result<usize> {
    file.write(bytes)
}

#[cfg(all(test, unix))]
fn write_stage(file: &mut File, bytes: &[u8]) -> io::Result<usize> {
    TEST_WRITE_LIMIT.with(|slot| {
        let mut slot = slot.borrow_mut();
        if let Some(remaining) = slot.as_mut() {
            if *remaining == 0 && !bytes.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::StorageFull,
                    "injected full stage device",
                ));
            }
            let written = file.write(&bytes[..bytes.len().min(*remaining)])?;
            *remaining -= written;
            Ok(written)
        } else {
            file.write(bytes)
        }
    })
}

fn is_stage_name(name: &OsStr) -> bool {
    name.to_str()
        .is_some_and(|name| name.starts_with(".stage.") && name.len() > ".stage.".len())
}

#[derive(Debug)]
pub(crate) struct SealedStage {
    parent: RetainedDirectoryCapability,
    name: OsString,
    file: File,
    identity: FileIdentity,
}

impl SealedStage {
    pub(crate) fn publish(
        self,
        destination: &OsStr,
        mode: PublishMode,
    ) -> Result<RetainedRegularFileCapability, PublicationFailure> {
        let mut certainty = PublicationCertainty::NeverPublished;
        let outcome = (|| {
            check_durability_support()?;
            if destination == self.name || is_stage_name(destination) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "execution-store destination must not be a private staging name",
                ));
            }
            self.parent.validate_named_identity()?;
            if let PublishMode::Replace { expected } = mode {
                let prior = self.parent.retain_regular_child(destination)?;
                if prior.identity() != expected {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "execution-store destination no longer names the expected prior object",
                    ));
                }
            }
            run_hook(PublicationPoint::BeforeRename)?;
            // Even a syscall error is conservatively unknown once the
            // publication operation has started. Never infer rollback from
            // errno or from a later lookup of the destination.
            certainty = PublicationCertainty::MayHavePublished;
            match mode {
                PublishMode::CreateNew => super::rename_identity_bound_regular_child_no_replace(
                    &self.parent.retained.directory,
                    &self.name,
                    self.identity,
                    &self.file,
                    &self.parent.retained.directory,
                    destination,
                )?,
                PublishMode::Replace { .. } => super::replace_identity_bound_regular_child(
                    &self.parent.retained.directory,
                    &self.name,
                    self.identity,
                    &self.file,
                    destination,
                )?,
            }
            run_hook(PublicationPoint::AfterRename)?;
            let published = self.parent.retain_regular_child(destination)?;
            if published.identity() != self.identity {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "execution-store published identity changed; replacement left untouched",
                ));
            }
            run_hook(PublicationPoint::BeforeFileSync)?;
            self.file.sync_all()?;
            run_hook(PublicationPoint::BeforeDirectorySync)?;
            flush_directory(&self.parent)?;
            published.validate_named_identity()?;
            run_hook(PublicationPoint::AfterDirectorySync)?;
            Ok(published)
        })();
        outcome.map_err(|error| PublicationFailure {
            certainty,
            error,
            _candidate: self.file,
        })
    }
}

/// Lock authority is a fresh open-file description, not a duplicate of any
/// retained directory or other lock. The process owner decides when it can
/// drop this object; no public clone, handle, or unlock operation exists.
#[derive(Debug)]
pub(crate) struct StableLock {
    parent: RetainedDirectoryCapability,
    name: OsString,
    file: File,
    identity: FileIdentity,
}

impl StableLock {
    pub(crate) fn acquire_existing(
        parent: &RetainedDirectoryCapability,
        name: &OsStr,
    ) -> io::Result<Self> {
        parent.validate_named_identity()?;
        let file = open_existing_lock(&parent.retained.directory, name)?;
        super::verify_owner_only_acl(&file)?;
        let identity = super::file_identity(&file)?;
        fs2::FileExt::try_lock_exclusive(&file)?;
        let held = Self {
            parent: parent.clone(),
            name: name.to_os_string(),
            file,
            identity,
        };
        held.validate()?;
        Ok(held)
    }

    pub(crate) fn identity(&self) -> FileIdentity {
        self.identity
    }

    pub(crate) fn validate(&self) -> io::Result<()> {
        self.parent.validate_named_identity()?;
        let named =
            super::open_regular_child_nofollow(&self.parent.retained.directory, &self.name)?;
        if super::file_identity(&named)? != self.identity
            || super::file_identity(&self.file)? != self.identity
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "execution-store stable lock identity changed",
            ));
        }
        Ok(())
    }
}

#[cfg(unix)]
fn open_existing_lock(parent: &File, name: &OsStr) -> io::Result<File> {
    let file = super::open_unix_child(
        parent,
        name,
        libc::O_RDWR | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
    )?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "lock is not a regular file",
        ));
    }
    Ok(file)
}

#[cfg(windows)]
fn open_existing_lock(parent: &File, name: &OsStr) -> io::Result<File> {
    use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE};
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, READ_CONTROL, SYNCHRONIZE,
    };
    const FILE_OPEN: u32 = 1;
    const FILE_NON_DIRECTORY_FILE: u32 = 0x40;
    const FILE_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    let file = super::open_relative_child_with_share_access(
        parent,
        name,
        GENERIC_READ | GENERIC_WRITE | READ_CONTROL | SYNCHRONIZE,
        FILE_ATTRIBUTE_NORMAL,
        FILE_OPEN,
        FILE_NON_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT,
        FILE_SHARE_READ | FILE_SHARE_WRITE,
        None,
    )?;
    if super::opened_child_kind(&file)? != super::OpenedChildKind::RegularFile {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "lock is not a regular file",
        ));
    }
    Ok(file)
}

#[cfg(not(any(unix, windows)))]
fn open_existing_lock(_parent: &File, _name: &OsStr) -> io::Result<File> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "stable locking is unavailable",
    ))
}

/// Enumeration owns one resumable cursor. Memory is bounded by one native
/// directory batch, not the total number of entries. No sorting is promised.
#[derive(Debug)]
pub(crate) struct DirectoryCursor {
    directory: RetainedDirectoryCapability,
    inner: CursorInner,
    finished: bool,
    failure: Option<io::Error>,
}

impl DirectoryCursor {
    pub(crate) fn open(directory: &RetainedDirectoryCapability) -> io::Result<Self> {
        directory.validate_named_identity()?;
        let inner = CursorInner::open(directory)?;
        directory.validate_named_identity()?;
        Ok(Self {
            directory: directory.clone(),
            inner,
            finished: false,
            failure: None,
        })
    }

    pub(crate) fn next_name(&mut self) -> io::Result<Option<OsString>> {
        if let Some(error) = &self.failure {
            return Err(copy_error(error));
        }
        if self.finished {
            return Ok(None);
        }
        let result = (|| {
            self.directory.validate_named_identity()?;
            let next = self.inner.next_name()?;
            self.directory.validate_named_identity()?;
            Ok(next)
        })();
        match &result {
            Ok(None) => self.finished = true,
            Err(error) => self.failure = Some(copy_error(error)),
            Ok(Some(_)) => {}
        }
        result
    }
}

#[cfg(unix)]
#[derive(Debug)]
struct CursorInner(cap_primitives::fs::ReadDir);

#[cfg(unix)]
impl CursorInner {
    fn open(directory: &RetainedDirectoryCapability) -> io::Result<Self> {
        cap_primitives::fs::read_base_dir(&directory.retained.directory).map(Self)
    }

    fn next_name(&mut self) -> io::Result<Option<OsString>> {
        self.0
            .next()
            .map(|entry| entry.map(|entry| entry.file_name()))
            .transpose()
    }
}

#[cfg(windows)]
#[derive(Debug)]
struct CursorInner {
    directory: File,
    storage: Vec<usize>,
    pending: std::collections::VecDeque<OsString>,
    restart: bool,
    ended: bool,
}

#[cfg(windows)]
impl CursorInner {
    fn open(directory: &RetainedDirectoryCapability) -> io::Result<Self> {
        // cap-primitives 4.0.2 uses an ambient path for Windows read_base_dir.
        // Reopen this child relative to its retained parent instead, with an
        // independent native enumeration position. Store records are children.
        let parent = directory.parent.as_ref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::Unsupported,
                "execution-store cursor requires a retained parent on Windows",
            )
        })?;
        let reopened = super::open_directory_child_nofollow(
            &parent.directory.retained.directory,
            &parent.name,
        )?;
        if super::file_identity(&reopened)? != directory.identity() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "enumeration directory changed",
            ));
        }
        Ok(Self {
            directory: reopened,
            storage: vec![0; (64 * 1024_usize).div_ceil(std::mem::size_of::<usize>())],
            pending: std::collections::VecDeque::new(),
            restart: true,
            ended: false,
        })
    }

    fn next_name(&mut self) -> io::Result<Option<OsString>> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            FileIdBothDirectoryInfo, FileIdBothDirectoryRestartInfo, GetFileInformationByHandleEx,
        };
        loop {
            if let Some(name) = self.pending.pop_front() {
                return Ok(Some(name));
            }
            if self.ended {
                return Ok(None);
            }
            self.storage.fill(0);
            let bytes = self.storage.len() * std::mem::size_of::<usize>();
            let class = if self.restart {
                FileIdBothDirectoryRestartInfo
            } else {
                FileIdBothDirectoryInfo
            };
            // SAFETY: the exclusive enumeration handle is live; storage is
            // aligned, initialized, and has exactly the supplied byte length.
            let status = unsafe {
                GetFileInformationByHandleEx(
                    self.directory.as_raw_handle(),
                    class,
                    self.storage.as_mut_ptr().cast(),
                    u32::try_from(bytes).expect("fixed 64 KiB buffer"),
                )
            };
            if status == 0 {
                let error = io::Error::last_os_error();
                if super::directory_query_is_end(self.restart, &error) {
                    self.ended = true;
                    return Ok(None);
                }
                return Err(error);
            }
            self.restart = false;
            // SAFETY: the same initialized backing storage remains borrowed
            // until the existing checked native-buffer parser finishes.
            let buffer =
                unsafe { std::slice::from_raw_parts(self.storage.as_ptr().cast::<u8>(), bytes) };
            super::visit_directory_information_buffer(buffer, |name| {
                self.pending.push_back(name);
                Ok::<_, io::Error>(())
            })?;
        }
    }
}

#[cfg(not(any(unix, windows)))]
#[derive(Debug)]
struct CursorInner;

#[cfg(not(any(unix, windows)))]
impl CursorInner {
    fn open(_directory: &RetainedDirectoryCapability) -> io::Result<Self> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "retained cursor is unavailable",
        ))
    }
    fn next_name(&mut self) -> io::Result<Option<OsString>> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "retained cursor is unavailable",
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PublicationPoint {
    BeforeStageSync,
    BeforeRename,
    AfterRename,
    BeforeFileSync,
    BeforeDirectorySync,
    AfterDirectorySync,
}

#[cfg(not(test))]
fn run_hook(_point: PublicationPoint) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
fn run_hook(point: PublicationPoint) -> io::Result<()> {
    TEST_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().as_mut() {
            hook(point)
        } else {
            Ok(())
        }
    })
}

#[cfg(test)]
type TestHook = Box<dyn FnMut(PublicationPoint) -> io::Result<()>>;

#[cfg(test)]
thread_local! {
    static TEST_HOOK: std::cell::RefCell<Option<TestHook>> = const { std::cell::RefCell::new(None) };
}

#[cfg(all(test, unix))]
thread_local! {
    static TEST_WRITE_LIMIT: std::cell::RefCell<Option<usize>> = const { std::cell::RefCell::new(None) };
}

#[cfg(all(test, unix))]
pub(crate) struct StageWriteLimitGuard(std::marker::PhantomData<std::rc::Rc<()>>);

#[cfg(all(test, unix))]
impl Drop for StageWriteLimitGuard {
    fn drop(&mut self) {
        TEST_WRITE_LIMIT.with(|slot| *slot.borrow_mut() = None);
    }
}

/// Real writes consume this byte budget before the next write reports a full
/// device. This is explicit fault injection, not an actual filesystem quota.
#[cfg(all(test, unix))]
pub(crate) fn install_stage_write_limit(bytes: usize) -> StageWriteLimitGuard {
    TEST_WRITE_LIMIT.with(|slot| {
        assert!(
            slot.borrow().is_none(),
            "stage write limit already installed"
        );
        *slot.borrow_mut() = Some(bytes);
    });
    StageWriteLimitGuard(std::marker::PhantomData)
}

#[cfg(test)]
pub(crate) struct PublicationHookGuard(std::marker::PhantomData<std::rc::Rc<()>>);

#[cfg(test)]
impl Drop for PublicationHookGuard {
    fn drop(&mut self) {
        TEST_HOOK.with(|slot| *slot.borrow_mut() = None);
    }
}

#[cfg(test)]
pub(crate) fn install_publication_hook(
    hook: impl FnMut(PublicationPoint) -> io::Result<()> + 'static,
) -> PublicationHookGuard {
    TEST_HOOK.with(|slot| {
        assert!(
            slot.borrow().is_none(),
            "publication hook already installed on this thread"
        );
        *slot.borrow_mut() = Some(Box::new(hook));
    });
    PublicationHookGuard(std::marker::PhantomData)
}

#[cfg(test)]
mod tests;

#[cfg(all(test, unix))]
mod process_tests;

#[cfg(all(test, unix))]
mod bootstrap_tests;

#[cfg(all(test, unix))]
mod file_tests;

#[cfg(all(test, unix))]
mod owner_tests;
