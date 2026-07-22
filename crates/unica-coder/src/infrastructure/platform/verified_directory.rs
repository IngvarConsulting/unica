use crate::domain::discovery::{PortableRelativePath, PortableRelativePathError};
use crate::infrastructure::platform::contained_file::VerifiedIdentity;
use std::collections::BTreeMap;
use std::fmt;
use std::fs::{File, Metadata, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VerifiedDirectoryEntryKind {
    Directory,
    RegularFile,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VerifiedDirectoryEntry {
    pub path: PathBuf,
    pub kind: VerifiedDirectoryEntryKind,
    pub identity: VerifiedIdentity,
}

#[derive(Debug)]
pub(crate) enum VerifiedDirectoryError {
    RootNotCanonical,
    RootNotDirectory,
    PathOutsideRoot,
    FinalPathOutsideRoot,
    FinalPathMismatch,
    AmbiguousHostPath,
    InvalidRelativePath(PortableRelativePathError),
    SymlinkOrReparsePoint,
    NotDirectory,
    NonRegularEntry,
    IdentityChanged,
    Cancelled,
    #[cfg_attr(not(windows), allow(dead_code, reason = "used by Windows handle APIs"))]
    LengthOverflow,
    #[cfg_attr(
        any(unix, windows),
        allow(
            dead_code,
            reason = "constructed only by fail-closed unsupported-host cfgs"
        )
    )]
    UnsupportedHost,
    Io {
        operation: &'static str,
        source: io::Error,
    },
}

impl fmt::Display for VerifiedDirectoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RootNotCanonical => formatter.write_str("source root is not its canonical path"),
            Self::RootNotDirectory => formatter.write_str("source root is not a directory"),
            Self::PathOutsideRoot => formatter.write_str("directory path is outside source root"),
            Self::FinalPathOutsideRoot => {
                formatter.write_str("opened directory resolved outside source root")
            }
            Self::FinalPathMismatch => {
                formatter.write_str("opened directory differs from the requested contained path")
            }
            Self::AmbiguousHostPath => {
                formatter.write_str("host path does not have a unique portable representation")
            }
            Self::InvalidRelativePath(error) => write!(formatter, "invalid relative path: {error}"),
            Self::SymlinkOrReparsePoint => {
                formatter.write_str("directory contains a symlink or reparse point")
            }
            Self::NotDirectory => formatter.write_str("opened target is not a directory"),
            Self::NonRegularEntry => {
                formatter.write_str("directory contains a non-regular filesystem entry")
            }
            Self::IdentityChanged => {
                formatter.write_str("directory identity changed during verified enumeration")
            }
            Self::Cancelled => formatter.write_str("verified directory enumeration cancelled"),
            Self::LengthOverflow => {
                formatter.write_str("directory data length is not representable")
            }
            Self::UnsupportedHost => {
                formatter.write_str("verified directory enumeration is unsupported on this host")
            }
            Self::Io { operation, source } => write!(formatter, "{operation}: {source}"),
        }
    }
}

impl std::error::Error for VerifiedDirectoryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidRelativePath(error) => Some(error),
            Self::Io { source, .. } => Some(source),
            Self::RootNotCanonical
            | Self::RootNotDirectory
            | Self::PathOutsideRoot
            | Self::FinalPathOutsideRoot
            | Self::FinalPathMismatch
            | Self::AmbiguousHostPath
            | Self::SymlinkOrReparsePoint
            | Self::NotDirectory
            | Self::NonRegularEntry
            | Self::IdentityChanged
            | Self::Cancelled
            | Self::LengthOverflow
            | Self::UnsupportedHost => None,
        }
    }
}

pub(crate) fn read_verified_contained_directory(
    root: &Path,
    path: &Path,
) -> Result<Vec<VerifiedDirectoryEntry>, VerifiedDirectoryError> {
    read_verified_contained_directory_observing(root, path, None, || {}, || {}, || {}, || false)
}

pub(crate) fn read_verified_contained_directory_cancellable(
    root: &Path,
    path: &Path,
    is_cancelled: impl FnMut() -> bool,
) -> Result<Vec<VerifiedDirectoryEntry>, VerifiedDirectoryError> {
    read_verified_contained_directory_observing(root, path, None, || {}, || {}, || {}, is_cancelled)
}

pub(crate) fn read_verified_contained_directory_with_expected_identity(
    root: &Path,
    path: &Path,
    expected_identity: VerifiedIdentity,
) -> Result<Vec<VerifiedDirectoryEntry>, VerifiedDirectoryError> {
    read_verified_contained_directory_observing(
        root,
        path,
        Some(expected_identity),
        || {},
        || {},
        || {},
        || false,
    )
}

pub(crate) fn read_verified_contained_directory_with_expected_identity_cancellable(
    root: &Path,
    path: &Path,
    expected_identity: VerifiedIdentity,
    is_cancelled: impl FnMut() -> bool,
) -> Result<Vec<VerifiedDirectoryEntry>, VerifiedDirectoryError> {
    read_verified_contained_directory_observing(
        root,
        path,
        Some(expected_identity),
        || {},
        || {},
        || {},
        is_cancelled,
    )
}

#[cfg(test)]
fn read_verified_contained_directory_with_observer(
    root: &Path,
    path: &Path,
    post_open_observer: impl FnOnce(),
    enumeration_observer: impl FnOnce(),
) -> Result<Vec<VerifiedDirectoryEntry>, VerifiedDirectoryError> {
    read_verified_contained_directory_observing(
        root,
        path,
        None,
        post_open_observer,
        enumeration_observer,
        || {},
        || false,
    )
}

#[cfg(test)]
fn read_verified_contained_directory_with_final_metadata_observer(
    root: &Path,
    path: &Path,
    final_metadata_observer: impl FnOnce(),
) -> Result<Vec<VerifiedDirectoryEntry>, VerifiedDirectoryError> {
    read_verified_contained_directory_observing(
        root,
        path,
        None,
        || {},
        || {},
        final_metadata_observer,
        || false,
    )
}

fn read_verified_contained_directory_observing(
    root: &Path,
    path: &Path,
    expected_identity: Option<VerifiedIdentity>,
    post_open_observer: impl FnOnce(),
    enumeration_observer: impl FnOnce(),
    final_metadata_observer: impl FnOnce(),
    mut is_cancelled: impl FnMut() -> bool,
) -> Result<Vec<VerifiedDirectoryEntry>, VerifiedDirectoryError> {
    let canonical_root = std::fs::canonicalize(root)
        .map(|path| {
            crate::infrastructure::platform::filesystem::strip_windows_extended_length_prefix(&path)
        })
        .map_err(|source| VerifiedDirectoryError::Io {
            operation: "resolve source root",
            source,
        })?;
    let supplied_root =
        crate::infrastructure::platform::filesystem::strip_windows_extended_length_prefix(root);
    if canonical_root != supplied_root {
        return Err(VerifiedDirectoryError::RootNotCanonical);
    }
    let root_metadata = std::fs::symlink_metadata(&canonical_root).map_err(|source| {
        VerifiedDirectoryError::Io {
            operation: "inspect source root",
            source,
        }
    })?;
    if crate::infrastructure::platform::filesystem::metadata_is_link_or_reparse_point(
        &root_metadata,
    ) || !root_metadata.file_type().is_dir()
    {
        return Err(VerifiedDirectoryError::RootNotDirectory);
    }

    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        canonical_root.join(path)
    };
    let relative = candidate
        .strip_prefix(&canonical_root)
        .map_err(|_| VerifiedDirectoryError::PathOutsideRoot)?;
    let expected_relative = portable_directory_relative(relative)?;

    let pre_open_identity = directory_identity_at_path(&candidate)?;
    if expected_identity.is_some_and(|expected| expected != pre_open_identity) {
        return Err(VerifiedDirectoryError::IdentityChanged);
    }
    let directory = open_directory_no_follow(&candidate)?;
    let opened_metadata = directory
        .metadata()
        .map_err(|source| VerifiedDirectoryError::Io {
            operation: "inspect opened directory",
            source,
        })?;
    validate_directory_metadata(&opened_metadata)?;
    let opened_identity = directory_identity_from_open_file(&directory, &opened_metadata)?;
    if pre_open_identity != opened_identity {
        return Err(VerifiedDirectoryError::IdentityChanged);
    }
    post_open_observer();
    validate_opened_directory_observing(
        &canonical_root,
        &candidate,
        expected_relative.as_ref(),
        &directory,
        opened_identity,
        final_metadata_observer,
    )?;
    enumeration_observer();
    let entries = enumerate_directory_handle(&directory, &candidate, &mut is_cancelled)?;
    let mut ordered_entries = BTreeMap::new();
    for entry in entries {
        if is_cancelled() {
            return Err(VerifiedDirectoryError::Cancelled);
        }
        let relative = entry
            .path
            .strip_prefix(&canonical_root)
            .map_err(|_| VerifiedDirectoryError::PathOutsideRoot)?;
        portable_directory_relative(relative)?.ok_or(
            VerifiedDirectoryError::InvalidRelativePath(PortableRelativePathError::Empty),
        )?;
        ordered_entries.insert(entry.path.clone(), entry);
    }
    validate_opened_directory(
        &canonical_root,
        &candidate,
        expected_relative.as_ref(),
        &directory,
        opened_identity,
    )?;
    let mut entries = Vec::new();
    for entry in ordered_entries.into_values() {
        if is_cancelled() {
            return Err(VerifiedDirectoryError::Cancelled);
        }
        entries.push(entry);
    }
    Ok(entries)
}

fn portable_directory_relative(
    relative: &Path,
) -> Result<Option<PortableRelativePath>, VerifiedDirectoryError> {
    if relative.as_os_str().is_empty() {
        return Ok(None);
    }
    if !host_relative_path_is_unambiguous(relative) {
        return Err(VerifiedDirectoryError::AmbiguousHostPath);
    }
    PortableRelativePath::parse(relative)
        .map(Some)
        .map_err(VerifiedDirectoryError::InvalidRelativePath)
}

#[cfg(unix)]
fn host_relative_path_is_unambiguous(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;

    !path.as_os_str().as_bytes().contains(&b'\\')
}

#[cfg(not(unix))]
fn host_relative_path_is_unambiguous(_path: &Path) -> bool {
    true
}

fn validate_opened_directory(
    root: &Path,
    candidate: &Path,
    expected_relative: Option<&PortableRelativePath>,
    directory: &File,
    opened_identity: VerifiedIdentity,
) -> Result<(), VerifiedDirectoryError> {
    validate_opened_directory_observing(
        root,
        candidate,
        expected_relative,
        directory,
        opened_identity,
        || {},
    )
}

fn validate_opened_directory_observing(
    root: &Path,
    candidate: &Path,
    expected_relative: Option<&PortableRelativePath>,
    directory: &File,
    opened_identity: VerifiedIdentity,
    final_metadata_observer: impl FnOnce(),
) -> Result<(), VerifiedDirectoryError> {
    let metadata = directory
        .metadata()
        .map_err(|source| VerifiedDirectoryError::Io {
            operation: "reinspect opened directory",
            source,
        })?;
    validate_directory_metadata(&metadata)?;
    if directory_identity_from_open_file(directory, &metadata)? != opened_identity {
        return Err(VerifiedDirectoryError::IdentityChanged);
    }
    let handle_path = final_opened_directory_path(directory).map_err(|error| {
        reclassify_directory_final_path_error(candidate, opened_identity, error)
    })?;
    let resolved_path = std::fs::canonicalize(&handle_path)
        .map(|path| {
            crate::infrastructure::platform::filesystem::strip_windows_extended_length_prefix(&path)
        })
        .map_err(|source| {
            reclassify_directory_final_path_error(
                candidate,
                opened_identity,
                VerifiedDirectoryError::Io {
                    operation: "resolve opened directory path",
                    source,
                },
            )
        })?;
    if !resolved_path.starts_with(root) {
        return Err(VerifiedDirectoryError::FinalPathOutsideRoot);
    }
    let final_relative = resolved_path
        .strip_prefix(root)
        .map_err(|_| VerifiedDirectoryError::FinalPathOutsideRoot)?;
    if portable_directory_relative(final_relative)?.as_ref() != expected_relative {
        return Err(VerifiedDirectoryError::FinalPathMismatch);
    }
    final_metadata_observer();
    let final_metadata = std::fs::symlink_metadata(&resolved_path).map_err(|source| {
        reclassify_directory_final_path_error(
            candidate,
            opened_identity,
            VerifiedDirectoryError::Io {
                operation: "inspect resolved opened directory",
                source,
            },
        )
    })?;
    validate_directory_metadata(&final_metadata)?;
    if directory_identity_at_path(&resolved_path)? != opened_identity {
        return Err(VerifiedDirectoryError::IdentityChanged);
    }
    if directory_identity_at_path(candidate)? != opened_identity {
        return Err(VerifiedDirectoryError::IdentityChanged);
    }
    Ok(())
}

fn reclassify_directory_final_path_error(
    candidate: &Path,
    opened_identity: VerifiedIdentity,
    final_path_error: VerifiedDirectoryError,
) -> VerifiedDirectoryError {
    match directory_identity_at_path(candidate) {
        Ok(candidate_identity) => {
            if candidate_identity == opened_identity {
                final_path_error
            } else {
                VerifiedDirectoryError::IdentityChanged
            }
        }
        Err(
            VerifiedDirectoryError::IdentityChanged
            | VerifiedDirectoryError::SymlinkOrReparsePoint
            | VerifiedDirectoryError::NotDirectory
            | VerifiedDirectoryError::NonRegularEntry,
        ) => VerifiedDirectoryError::IdentityChanged,
        Err(
            VerifiedDirectoryError::RootNotCanonical
            | VerifiedDirectoryError::RootNotDirectory
            | VerifiedDirectoryError::PathOutsideRoot
            | VerifiedDirectoryError::FinalPathOutsideRoot
            | VerifiedDirectoryError::FinalPathMismatch
            | VerifiedDirectoryError::AmbiguousHostPath
            | VerifiedDirectoryError::InvalidRelativePath(_)
            | VerifiedDirectoryError::Cancelled
            | VerifiedDirectoryError::LengthOverflow
            | VerifiedDirectoryError::UnsupportedHost
            | VerifiedDirectoryError::Io { .. },
        ) => final_path_error,
    }
}

fn validate_directory_metadata(metadata: &Metadata) -> Result<(), VerifiedDirectoryError> {
    if crate::infrastructure::platform::filesystem::metadata_is_link_or_reparse_point(metadata) {
        return Err(VerifiedDirectoryError::SymlinkOrReparsePoint);
    }
    if !metadata.file_type().is_dir() {
        return Err(VerifiedDirectoryError::NotDirectory);
    }
    Ok(())
}

#[cfg(unix)]
fn directory_identity_at_path(path: &Path) -> Result<VerifiedIdentity, VerifiedDirectoryError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|source| {
        if matches!(source.raw_os_error(), Some(libc::ENOENT | libc::ENOTDIR)) {
            VerifiedDirectoryError::IdentityChanged
        } else {
            VerifiedDirectoryError::Io {
                operation: "inspect directory path identity",
                source,
            }
        }
    })?;
    validate_directory_metadata(&metadata)?;
    directory_identity_from_metadata(&metadata)
}

#[cfg(windows)]
fn directory_identity_at_path(path: &Path) -> Result<VerifiedIdentity, VerifiedDirectoryError> {
    let directory = open_directory_no_follow(path)?;
    let metadata = directory
        .metadata()
        .map_err(|source| VerifiedDirectoryError::Io {
            operation: "inspect directory path identity",
            source,
        })?;
    validate_directory_metadata(&metadata)?;
    directory_identity_from_open_file(&directory, &metadata)
}

#[cfg(not(any(unix, windows)))]
fn directory_identity_at_path(_path: &Path) -> Result<VerifiedIdentity, VerifiedDirectoryError> {
    Err(VerifiedDirectoryError::UnsupportedHost)
}

#[cfg(unix)]
fn directory_identity_from_metadata(
    metadata: &Metadata,
) -> Result<VerifiedIdentity, VerifiedDirectoryError> {
    use std::os::unix::fs::MetadataExt;

    Ok(VerifiedIdentity {
        storage: metadata.dev(),
        object: metadata.ino(),
    })
}

#[cfg(unix)]
fn directory_identity_from_open_file(
    _directory: &File,
    metadata: &Metadata,
) -> Result<VerifiedIdentity, VerifiedDirectoryError> {
    directory_identity_from_metadata(metadata)
}

#[cfg(windows)]
fn directory_identity_from_open_file(
    directory: &File,
    _metadata: &Metadata,
) -> Result<VerifiedIdentity, VerifiedDirectoryError> {
    use std::mem::MaybeUninit;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };

    let mut information = MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
    // SAFETY: directory owns a valid handle and the pointer targets writable storage initialized
    // completely by a successful GetFileInformationByHandle call.
    let succeeded =
        unsafe { GetFileInformationByHandle(directory.as_raw_handle(), information.as_mut_ptr()) };
    if succeeded == 0 {
        return Err(VerifiedDirectoryError::Io {
            operation: "inspect opened Windows directory identity",
            source: io::Error::last_os_error(),
        });
    }
    // SAFETY: a nonzero result guarantees initialization of the complete information structure.
    let information = unsafe { information.assume_init() };
    const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    if information.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(VerifiedDirectoryError::SymlinkOrReparsePoint);
    }
    if information.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0 {
        return Err(VerifiedDirectoryError::NotDirectory);
    }
    Ok(VerifiedIdentity {
        storage: u64::from(information.dwVolumeSerialNumber),
        object: (u64::from(information.nFileIndexHigh) << 32)
            | u64::from(information.nFileIndexLow),
    })
}

#[cfg(not(any(unix, windows)))]
fn directory_identity_from_open_file(
    _directory: &File,
    _metadata: &Metadata,
) -> Result<VerifiedIdentity, VerifiedDirectoryError> {
    Err(VerifiedDirectoryError::UnsupportedHost)
}

#[cfg(unix)]
fn open_directory_no_follow(path: &Path) -> Result<File, VerifiedDirectoryError> {
    use std::os::unix::fs::OpenOptionsExt;

    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_DIRECTORY)
        .open(path)
        .map_err(|source| {
            if source.raw_os_error() == Some(libc::ELOOP) {
                VerifiedDirectoryError::SymlinkOrReparsePoint
            } else if matches!(source.raw_os_error(), Some(libc::ENOENT | libc::ENOTDIR)) {
                VerifiedDirectoryError::IdentityChanged
            } else {
                VerifiedDirectoryError::Io {
                    operation: "open directory without following links",
                    source,
                }
            }
        })
}

#[cfg(windows)]
fn open_directory_no_follow(path: &Path) -> Result<File, VerifiedDirectoryError> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE,
        FILE_SHARE_READ, FILE_SHARE_WRITE,
    };

    OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .map_err(|source| {
            if source.kind() == io::ErrorKind::NotFound {
                VerifiedDirectoryError::IdentityChanged
            } else {
                VerifiedDirectoryError::Io {
                    operation: "open Windows directory without following reparse points",
                    source,
                }
            }
        })
}

#[cfg(not(any(unix, windows)))]
fn open_directory_no_follow(_path: &Path) -> Result<File, VerifiedDirectoryError> {
    Err(VerifiedDirectoryError::UnsupportedHost)
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn final_opened_directory_path(directory: &File) -> Result<PathBuf, VerifiedDirectoryError> {
    use std::os::fd::AsRawFd;

    std::fs::read_link(format!("/proc/self/fd/{}", directory.as_raw_fd())).map_err(|source| {
        VerifiedDirectoryError::Io {
            operation: "resolve opened Unix directory handle",
            source,
        }
    })
}

#[cfg(target_os = "macos")]
fn final_opened_directory_path(directory: &File) -> Result<PathBuf, VerifiedDirectoryError> {
    use std::ffi::CStr;
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStrExt;

    let mut buffer = vec![0_i8; libc::PATH_MAX as usize];
    // SAFETY: directory owns a valid descriptor and buffer is writable for PATH_MAX bytes.
    let result =
        unsafe { libc::fcntl(directory.as_raw_fd(), libc::F_GETPATH, buffer.as_mut_ptr()) };
    if result == -1 {
        return Err(VerifiedDirectoryError::Io {
            operation: "resolve opened macOS directory handle",
            source: io::Error::last_os_error(),
        });
    }
    // SAFETY: successful F_GETPATH initializes buffer with a NUL-terminated C string.
    let path = unsafe { CStr::from_ptr(buffer.as_ptr()) };
    Ok(PathBuf::from(std::ffi::OsStr::from_bytes(path.to_bytes())))
}

#[cfg(all(
    unix,
    not(any(target_os = "linux", target_os = "android", target_os = "macos"))
))]
fn final_opened_directory_path(_directory: &File) -> Result<PathBuf, VerifiedDirectoryError> {
    Err(VerifiedDirectoryError::UnsupportedHost)
}

#[cfg(windows)]
fn final_opened_directory_path(directory: &File) -> Result<PathBuf, VerifiedDirectoryError> {
    use std::os::windows::ffi::OsStringExt;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFinalPathNameByHandleW, FILE_NAME_NORMALIZED, VOLUME_NAME_DOS,
    };

    let mut buffer = vec![0_u16; 512];
    loop {
        let buffer_length =
            u32::try_from(buffer.len()).map_err(|_| VerifiedDirectoryError::LengthOverflow)?;
        // SAFETY: directory owns a valid handle and buffer is writable for buffer_length UTF-16
        // code units for the duration of the call.
        let length = unsafe {
            GetFinalPathNameByHandleW(
                directory.as_raw_handle(),
                buffer.as_mut_ptr(),
                buffer_length,
                FILE_NAME_NORMALIZED | VOLUME_NAME_DOS,
            )
        };
        if length == 0 {
            return Err(VerifiedDirectoryError::Io {
                operation: "resolve opened Windows directory handle",
                source: io::Error::last_os_error(),
            });
        }
        let length = usize::try_from(length).map_err(|_| VerifiedDirectoryError::LengthOverflow)?;
        if length < buffer.len() {
            buffer.truncate(length);
            let path = PathBuf::from(std::ffi::OsString::from_wide(&buffer));
            return Ok(
                crate::infrastructure::platform::filesystem::strip_windows_extended_length_prefix(
                    &path,
                ),
            );
        }
        buffer.resize(
            length
                .checked_add(1)
                .ok_or(VerifiedDirectoryError::LengthOverflow)?,
            0,
        );
    }
}

#[cfg(not(any(unix, windows)))]
fn final_opened_directory_path(_directory: &File) -> Result<PathBuf, VerifiedDirectoryError> {
    Err(VerifiedDirectoryError::UnsupportedHost)
}

#[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
fn duplicate_directory_descriptor_cloexec(
    descriptor: std::os::fd::RawFd,
) -> io::Result<std::os::fd::OwnedFd> {
    use std::os::fd::FromRawFd;

    // SAFETY: descriptor is live for this call. F_DUPFD_CLOEXEC creates a distinct descriptor
    // with FD_CLOEXEC set atomically, so no inheritable descriptor is observable between calls.
    let duplicate = unsafe { libc::fcntl(descriptor, libc::F_DUPFD_CLOEXEC, 0) };
    if duplicate == -1 {
        Err(io::Error::last_os_error())
    } else {
        // SAFETY: successful F_DUPFD_CLOEXEC returns a new descriptor owned by this function.
        Ok(unsafe { std::os::fd::OwnedFd::from_raw_fd(duplicate) })
    }
}

#[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
fn enumerate_directory_handle(
    directory: &File,
    path: &Path,
    is_cancelled: &mut dyn FnMut() -> bool,
) -> Result<Vec<VerifiedDirectoryEntry>, VerifiedDirectoryError> {
    use std::ffi::{CStr, CString};
    use std::mem::MaybeUninit;
    use std::os::fd::{AsRawFd, IntoRawFd};
    use std::os::unix::ffi::OsStrExt;

    struct OwnedDirectory(*mut libc::DIR);

    impl Drop for OwnedDirectory {
        fn drop(&mut self) {
            // SAFETY: fdopendir returned this non-null DIR pointer and ownership is unique here.
            unsafe {
                libc::closedir(self.0);
            }
        }
    }

    let descriptor = directory.as_raw_fd();
    let duplicate = duplicate_directory_descriptor_cloexec(descriptor).map_err(|source| {
        VerifiedDirectoryError::Io {
            operation: "duplicate verified directory handle",
            source,
        }
    })?;
    // SAFETY: duplicate is live and remains owned locally until fdopendir succeeds.
    let stream = unsafe { libc::fdopendir(duplicate.as_raw_fd()) };
    if stream.is_null() {
        return Err(VerifiedDirectoryError::Io {
            operation: "create verified directory stream",
            source: io::Error::last_os_error(),
        });
    }
    let _transferred_descriptor = duplicate.into_raw_fd();
    let stream = OwnedDirectory(stream);
    let mut entries = Vec::new();
    loop {
        set_errno(0);
        // SAFETY: stream owns a live DIR pointer; readdir's returned pointer remains valid until
        // the next operation on this stream and is consumed before then.
        let native_entry = unsafe { libc::readdir(stream.0) };
        if native_entry.is_null() {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(0) {
                break;
            }
            return Err(VerifiedDirectoryError::Io {
                operation: "enumerate verified directory handle",
                source: error,
            });
        }
        // SAFETY: native_entry is non-null and d_name is NUL-terminated by readdir.
        let name = unsafe { CStr::from_ptr((*native_entry).d_name.as_ptr()) }.to_bytes();
        if matches!(name, b"." | b"..") {
            continue;
        }
        if is_cancelled() {
            return Err(VerifiedDirectoryError::Cancelled);
        }
        let c_name = CString::new(name).map_err(|source| VerifiedDirectoryError::Io {
            operation: "validate directory entry name",
            source: io::Error::new(io::ErrorKind::InvalidData, source),
        })?;
        let mut status = MaybeUninit::<libc::stat>::uninit();
        // SAFETY: descriptor is the verified directory handle, c_name is NUL-terminated, and
        // status points to writable storage initialized fully by successful fstatat.
        let result = unsafe {
            libc::fstatat(
                descriptor,
                c_name.as_ptr(),
                status.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        if result == -1 {
            let source = io::Error::last_os_error();
            if matches!(source.raw_os_error(), Some(libc::ENOENT | libc::ENOTDIR)) {
                return Err(VerifiedDirectoryError::IdentityChanged);
            }
            return Err(VerifiedDirectoryError::Io {
                operation: "inspect entry from verified directory handle",
                source,
            });
        }
        // SAFETY: successful fstatat initialized the complete stat structure.
        let status = unsafe { status.assume_init() };
        let file_type = status.st_mode & libc::S_IFMT;
        let kind = if file_type == libc::S_IFDIR {
            VerifiedDirectoryEntryKind::Directory
        } else if file_type == libc::S_IFREG {
            VerifiedDirectoryEntryKind::RegularFile
        } else if file_type == libc::S_IFLNK {
            return Err(VerifiedDirectoryError::SymlinkOrReparsePoint);
        } else {
            return Err(VerifiedDirectoryError::NonRegularEntry);
        };
        entries.push(VerifiedDirectoryEntry {
            path: path.join(std::ffi::OsStr::from_bytes(name)),
            kind,
            identity: VerifiedIdentity {
                storage: status.st_dev as u64,
                object: status.st_ino as u64,
            },
        });
    }
    Ok(entries)
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn set_errno(value: i32) {
    // SAFETY: __errno_location returns the calling thread's writable errno slot.
    unsafe {
        *libc::__errno_location() = value;
    }
}

#[cfg(target_os = "macos")]
fn set_errno(value: i32) {
    // SAFETY: __error returns the calling thread's writable errno slot.
    unsafe {
        *libc::__error() = value;
    }
}

#[cfg(all(
    unix,
    not(any(target_os = "linux", target_os = "android", target_os = "macos"))
))]
fn enumerate_directory_handle(
    _directory: &File,
    _path: &Path,
    _is_cancelled: &mut dyn FnMut() -> bool,
) -> Result<Vec<VerifiedDirectoryEntry>, VerifiedDirectoryError> {
    Err(VerifiedDirectoryError::UnsupportedHost)
}

#[cfg(windows)]
fn enumerate_directory_handle(
    directory: &File,
    path: &Path,
    is_cancelled: &mut dyn FnMut() -> bool,
) -> Result<Vec<VerifiedDirectoryEntry>, VerifiedDirectoryError> {
    use std::mem::{offset_of, size_of};
    use std::os::windows::ffi::OsStringExt;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::ERROR_NO_MORE_FILES;
    use windows_sys::Win32::Storage::FileSystem::{
        FileIdBothDirectoryInfo, FileIdBothDirectoryRestartInfo, GetFileInformationByHandleEx,
        FILE_ID_BOTH_DIR_INFO,
    };

    const BUFFER_BYTES: usize = 64 * 1_024;
    const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
    const FILE_ATTRIBUTE_DEVICE: u32 = 0x40;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

    let mut storage = vec![0_u64; BUFFER_BYTES / size_of::<u64>()];
    let buffer_size =
        u32::try_from(BUFFER_BYTES).map_err(|_| VerifiedDirectoryError::LengthOverflow)?;
    let mut restart = true;
    let mut entries = Vec::new();
    let directory_metadata = directory
        .metadata()
        .map_err(|source| VerifiedDirectoryError::Io {
            operation: "inspect opened Windows directory during enumeration",
            source,
        })?;
    let directory_identity = directory_identity_from_open_file(directory, &directory_metadata)?;
    loop {
        let information_class = if restart {
            FileIdBothDirectoryRestartInfo
        } else {
            FileIdBothDirectoryInfo
        };
        restart = false;
        // SAFETY: directory owns a valid handle and storage is aligned, writable, and at least
        // buffer_size bytes for the duration of this synchronous call.
        let succeeded = unsafe {
            GetFileInformationByHandleEx(
                directory.as_raw_handle(),
                information_class,
                storage.as_mut_ptr().cast(),
                buffer_size,
            )
        };
        if succeeded == 0 {
            let source = io::Error::last_os_error();
            if source.raw_os_error() == i32::try_from(ERROR_NO_MORE_FILES).ok() {
                break;
            }
            return Err(VerifiedDirectoryError::Io {
                operation: "enumerate verified Windows directory handle",
                source,
            });
        }

        let buffer = storage.as_ptr().cast::<u8>();
        let mut offset = 0_usize;
        loop {
            let header_end = offset
                .checked_add(offset_of!(FILE_ID_BOTH_DIR_INFO, FileName))
                .ok_or(VerifiedDirectoryError::LengthOverflow)?;
            if header_end > BUFFER_BYTES {
                return Err(VerifiedDirectoryError::Io {
                    operation: "validate Windows directory entry buffer",
                    source: io::Error::new(io::ErrorKind::InvalidData, "truncated entry header"),
                });
            }
            // SAFETY: offset is kept within the aligned API buffer and header bounds were checked.
            let information = unsafe { &*buffer.add(offset).cast::<FILE_ID_BOTH_DIR_INFO>() };
            let name_bytes = usize::try_from(information.FileNameLength)
                .map_err(|_| VerifiedDirectoryError::LengthOverflow)?;
            if name_bytes % size_of::<u16>() != 0 {
                return Err(VerifiedDirectoryError::Io {
                    operation: "validate Windows directory entry buffer",
                    source: io::Error::new(io::ErrorKind::InvalidData, "odd UTF-16 byte length"),
                });
            }
            let name_end = header_end
                .checked_add(name_bytes)
                .ok_or(VerifiedDirectoryError::LengthOverflow)?;
            if name_end > BUFFER_BYTES {
                return Err(VerifiedDirectoryError::Io {
                    operation: "validate Windows directory entry buffer",
                    source: io::Error::new(io::ErrorKind::InvalidData, "truncated entry name"),
                });
            }
            // SAFETY: FileName begins at header_end and name_end was checked within the live
            // buffer; Windows supplies an aligned UTF-16 sequence of FileNameLength bytes.
            let name = unsafe {
                std::slice::from_raw_parts(
                    buffer.add(header_end).cast::<u16>(),
                    name_bytes / size_of::<u16>(),
                )
            };
            if name != [b'.' as u16] && name != [b'.' as u16, b'.' as u16] {
                if is_cancelled() {
                    return Err(VerifiedDirectoryError::Cancelled);
                }
                if information.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                    return Err(VerifiedDirectoryError::SymlinkOrReparsePoint);
                }
                let kind = if information.FileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
                    VerifiedDirectoryEntryKind::Directory
                } else if information.FileAttributes & FILE_ATTRIBUTE_DEVICE == 0 {
                    VerifiedDirectoryEntryKind::RegularFile
                } else {
                    return Err(VerifiedDirectoryError::NonRegularEntry);
                };
                entries.push(VerifiedDirectoryEntry {
                    path: path.join(std::ffi::OsString::from_wide(name)),
                    kind,
                    identity: VerifiedIdentity {
                        storage: directory_identity.storage,
                        object: information.FileId as u64,
                    },
                });
            }

            if information.NextEntryOffset == 0 {
                break;
            }
            let next = usize::try_from(information.NextEntryOffset)
                .map_err(|_| VerifiedDirectoryError::LengthOverflow)?;
            if next < offset_of!(FILE_ID_BOTH_DIR_INFO, FileName) || next % size_of::<u64>() != 0 {
                return Err(VerifiedDirectoryError::Io {
                    operation: "validate Windows directory entry buffer",
                    source: io::Error::new(io::ErrorKind::InvalidData, "invalid next-entry offset"),
                });
            }
            offset = offset
                .checked_add(next)
                .ok_or(VerifiedDirectoryError::LengthOverflow)?;
            if offset >= BUFFER_BYTES {
                return Err(VerifiedDirectoryError::Io {
                    operation: "validate Windows directory entry buffer",
                    source: io::Error::new(
                        io::ErrorKind::InvalidData,
                        "next entry is out of bounds",
                    ),
                });
            }
        }
    }
    Ok(entries)
}

#[cfg(not(any(unix, windows)))]
fn enumerate_directory_handle(
    _directory: &File,
    _path: &Path,
    _is_cancelled: &mut dyn FnMut() -> bool,
) -> Result<Vec<VerifiedDirectoryEntry>, VerifiedDirectoryError> {
    Err(VerifiedDirectoryError::UnsupportedHost)
}

#[cfg(test)]
mod tests {
    use super::{
        read_verified_contained_directory, read_verified_contained_directory_cancellable,
        read_verified_contained_directory_with_expected_identity,
        read_verified_contained_directory_with_final_metadata_observer,
        read_verified_contained_directory_with_observer, VerifiedDirectoryEntryKind,
        VerifiedDirectoryError,
    };
    use crate::infrastructure::platform::testing::create_dir_symlink_for_test;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    #[test]
    fn enumerates_from_a_verified_directory_handle_in_name_order() {
        let root = fixture_root("entries");
        fs::create_dir(root.join("a")).expect("directory fixture");
        fs::write(root.join("z.xml"), b"z").expect("file fixture");

        let entries = read_verified_contained_directory(&root, &root).expect("verified directory");

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].path, root.join("a"));
        assert_eq!(entries[0].kind, VerifiedDirectoryEntryKind::Directory);
        assert_eq!(entries[1].path, root.join("z.xml"));
        assert_eq!(entries[1].kind, VerifiedDirectoryEntryKind::RegularFile);
        cleanup(&root);
    }

    #[test]
    fn cancellation_is_observed_inside_a_flat_directory_enumeration() {
        let root = fixture_root("cancel-flat-enumeration");
        fs::write(root.join("a.xml"), b"a").expect("first file fixture");
        fs::write(root.join("b.xml"), b"b").expect("second file fixture");
        fs::write(root.join("c.xml"), b"c").expect("third file fixture");
        let mut entries_observed = 0_u8;

        let error = read_verified_contained_directory_cancellable(&root, &root, || {
            entries_observed += 1;
            entries_observed == 2
        })
        .expect_err("enumeration must stop at the cancellation boundary");

        assert!(matches!(error, VerifiedDirectoryError::Cancelled));
        assert_eq!(entries_observed, 2);
        cleanup(&root);
    }

    #[test]
    fn rejects_a_link_entry_from_the_directory_handle() {
        let root = fixture_root("link-entry");
        let outside = fixture_root("link-entry-outside");
        fs::write(outside.join("Document.xml"), b"outside").expect("outside fixture");
        let link = root.join("linked");
        let Some(link_result) = create_dir_symlink_for_test(&outside, &link) else {
            cleanup(&root);
            cleanup(&outside);
            return;
        };
        if let Err(error) = link_result {
            if error.raw_os_error() == Some(1_314) {
                cleanup(&root);
                cleanup(&outside);
                return;
            }
            panic!("directory-link fixture failed: {error}");
        }

        let error = read_verified_contained_directory(&root, &root)
            .expect_err("directory link entry must fail closed");

        assert!(matches!(
            error,
            VerifiedDirectoryError::SymlinkOrReparsePoint
        ));
        cleanup(&root);
        cleanup(&outside);
    }

    #[test]
    fn rejects_a_child_directory_replaced_after_parent_handle_enumeration() {
        let root = fixture_root("child-replacement-after-enumeration");
        let directory = root.join("Objects");
        let displaced = root.join("displaced");
        fs::create_dir(&directory).expect("directory fixture");
        let entry = read_verified_contained_directory(&root, &root)
            .expect("parent enumeration")
            .into_iter()
            .find(|entry| entry.path == directory)
            .expect("enumerated child directory");
        fs::rename(&directory, &displaced).expect("displace enumerated directory");
        fs::create_dir(&directory).expect("replacement directory");

        let error = read_verified_contained_directory_with_expected_identity(
            &root,
            &directory,
            entry.identity,
        )
        .expect_err("replacement after enumeration must fail closed");

        assert!(matches!(error, VerifiedDirectoryError::IdentityChanged));
        cleanup(&root);
    }

    #[test]
    fn disappearance_during_final_directory_metadata_validation_is_an_identity_change() {
        let root = fixture_root("disappear-during-final-directory-metadata");
        let directory = root.join("Objects");
        fs::create_dir(&directory).expect("directory fixture");

        let error = read_verified_contained_directory_with_final_metadata_observer(
            &root,
            &directory,
            || fs::remove_dir(&directory).expect("remove opened directory during validation"),
        )
        .expect_err("disappearance during final metadata validation must fail closed");

        assert!(matches!(error, VerifiedDirectoryError::IdentityChanged));
        cleanup(&root);
    }

    #[test]
    fn unchanged_directory_candidate_preserves_a_final_handle_facility_failure() {
        let root = fixture_root("unchanged-directory-final-handle-failure");
        let directory = root.join("Objects");
        fs::create_dir(&directory).expect("directory fixture");
        let identity = super::directory_identity_at_path(&directory).expect("fixture identity");
        let facility_failure = VerifiedDirectoryError::Io {
            operation: "resolve opened Unix directory handle",
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "procfd unavailable"),
        };

        let error =
            super::reclassify_directory_final_path_error(&directory, identity, facility_failure);

        assert!(matches!(
            error,
            VerifiedDirectoryError::Io {
                operation: "resolve opened Unix directory handle",
                ..
            }
        ));
        cleanup(&root);
    }

    #[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
    #[test]
    fn duplicated_directory_descriptors_are_close_on_exec() {
        use std::os::fd::AsRawFd;

        let root = fixture_root("directory-descriptor-cloexec");
        let directory = fs::File::open(&root).expect("open directory fixture");

        let duplicate = super::duplicate_directory_descriptor_cloexec(directory.as_raw_fd())
            .expect("duplicate directory descriptor");
        // SAFETY: duplicate owns a live descriptor for the duration of this query.
        let flags = unsafe { libc::fcntl(duplicate.as_raw_fd(), libc::F_GETFD) };

        assert_ne!(flags, -1);
        assert_ne!(flags & libc::FD_CLOEXEC, 0);
        cleanup(&root);
    }

    #[cfg(unix)]
    #[test]
    fn directory_replacement_is_rejected_before_entries_are_published() {
        use std::os::unix::fs::symlink;

        let root = fixture_root("directory-replacement");
        let outside = fixture_root("directory-replacement-outside");
        let directory = root.join("Objects");
        let escaped = outside.join("escaped-original");
        fs::create_dir(&directory).expect("directory fixture");
        fs::write(directory.join("secret.xml"), b"secret").expect("contained fixture");
        fs::write(outside.join("replacement.xml"), b"replacement").expect("outside fixture");
        let enumeration_observed = AtomicBool::new(false);

        let error = read_verified_contained_directory_with_observer(
            &root,
            &directory,
            || {
                fs::rename(&directory, &escaped).expect("move opened directory outside root");
                symlink(&outside, &directory).expect("install escaping directory link");
            },
            || enumeration_observed.store(true, Ordering::SeqCst),
        )
        .expect_err("directory replacement must fail closed");

        assert!(matches!(
            error,
            VerifiedDirectoryError::FinalPathOutsideRoot
                | VerifiedDirectoryError::FinalPathMismatch
                | VerifiedDirectoryError::IdentityChanged
        ));
        assert!(!enumeration_observed.load(Ordering::SeqCst));
        cleanup(&root);
        cleanup(&outside);
    }

    #[cfg(unix)]
    #[test]
    fn directory_replacement_during_enumeration_invalidates_all_entries() {
        use std::os::unix::fs::symlink;

        let root = fixture_root("directory-replacement-during-enumeration");
        let outside = fixture_root("directory-replacement-during-enumeration-outside");
        let directory = root.join("Objects");
        let escaped = outside.join("escaped-original");
        fs::create_dir(&directory).expect("directory fixture");
        fs::write(directory.join("secret.xml"), b"secret").expect("contained fixture");
        fs::write(outside.join("replacement.xml"), b"replacement").expect("outside fixture");

        let error = read_verified_contained_directory_with_observer(
            &root,
            &directory,
            || {},
            || {
                fs::rename(&directory, &escaped).expect("move enumerated directory outside root");
                symlink(&outside, &directory).expect("install escaping directory link");
            },
        )
        .expect_err("replacement during enumeration must invalidate every entry");

        assert!(matches!(
            error,
            VerifiedDirectoryError::FinalPathOutsideRoot
                | VerifiedDirectoryError::FinalPathMismatch
                | VerifiedDirectoryError::IdentityChanged
        ));
        cleanup(&root);
        cleanup(&outside);
    }

    fn fixture_root(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "unica-verified-directory-{label}-{}-{nanos}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).expect("fixture root");
        fs::canonicalize(root).expect("canonical fixture root")
    }

    fn cleanup(root: &Path) {
        fs::remove_dir_all(root).expect("fixture cleanup");
    }
}
