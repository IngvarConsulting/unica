use crate::domain::discovery::{ContentHash, PortableRelativePath, PortableRelativePathError};
use std::fmt;
use std::fs::{File, Metadata, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct VerifiedIdentity {
    pub storage: u64,
    pub object: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VerifiedFile {
    pub relative_path: PortableRelativePath,
    pub bytes: Vec<u8>,
    pub raw_sha256: ContentHash,
    pub bytes_read: u64,
    pub identity: VerifiedIdentity,
}

#[derive(Debug)]
pub(crate) enum ContainedFileError {
    RootNotCanonical,
    RootNotDirectory,
    PathOutsideRoot,
    FinalPathOutsideRoot,
    FinalPathMismatch,
    AmbiguousHostPath,
    InvalidRelativePath(PortableRelativePathError),
    SymlinkOrReparsePoint,
    NotRegularFile,
    IdentityChanged,
    SizeLimitExceeded {
        limit: u64,
    },
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

impl fmt::Display for ContainedFileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RootNotCanonical => formatter.write_str("source root is not its canonical path"),
            Self::RootNotDirectory => formatter.write_str("source root is not a directory"),
            Self::PathOutsideRoot => formatter.write_str("file path is outside the source root"),
            Self::FinalPathOutsideRoot => {
                formatter.write_str("opened file resolved outside the source root")
            }
            Self::FinalPathMismatch => {
                formatter.write_str("opened file path differs from the requested contained path")
            }
            Self::AmbiguousHostPath => {
                formatter.write_str("host path does not have a unique portable representation")
            }
            Self::InvalidRelativePath(error) => write!(formatter, "invalid relative path: {error}"),
            Self::SymlinkOrReparsePoint => {
                formatter.write_str("file is a symlink or reparse point")
            }
            Self::NotRegularFile => formatter.write_str("file target is not a regular file"),
            Self::IdentityChanged => {
                formatter.write_str("file identity changed during verified read")
            }
            Self::SizeLimitExceeded { limit } => {
                write!(formatter, "file exceeds the {limit}-byte read limit")
            }
            Self::LengthOverflow => formatter.write_str("file byte count is not representable"),
            Self::UnsupportedHost => {
                formatter.write_str("verified contained reads are unsupported on this host")
            }
            Self::Io { operation, source } => write!(formatter, "{operation}: {source}"),
        }
    }
}

impl std::error::Error for ContainedFileError {
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
            | Self::NotRegularFile
            | Self::IdentityChanged
            | Self::SizeLimitExceeded { .. }
            | Self::LengthOverflow
            | Self::UnsupportedHost => None,
        }
    }
}

pub(crate) fn read_contained_regular_file(
    root: &Path,
    path: &Path,
    max_bytes: u64,
) -> Result<VerifiedFile, ContainedFileError> {
    read_contained_regular_file_observing(root, path, max_bytes, || {}, || {})
}

#[cfg(test)]
fn read_contained_regular_file_with_observer(
    root: &Path,
    path: &Path,
    max_bytes: u64,
    observer: impl FnOnce(),
) -> Result<VerifiedFile, ContainedFileError> {
    read_contained_regular_file_observing(root, path, max_bytes, observer, || {})
}

#[cfg(test)]
fn read_contained_regular_file_with_post_open_observer(
    root: &Path,
    path: &Path,
    max_bytes: u64,
    observer: impl FnOnce(),
) -> Result<VerifiedFile, ContainedFileError> {
    read_contained_regular_file_observing(root, path, max_bytes, || {}, observer)
}

fn read_contained_regular_file_observing(
    root: &Path,
    path: &Path,
    max_bytes: u64,
    pre_open_observer: impl FnOnce(),
    post_open_observer: impl FnOnce(),
) -> Result<VerifiedFile, ContainedFileError> {
    let canonical_root = std::fs::canonicalize(root)
        .map(|path| {
            crate::infrastructure::platform::filesystem::strip_windows_extended_length_prefix(&path)
        })
        .map_err(|source| ContainedFileError::Io {
            operation: "resolve source root",
            source,
        })?;
    let supplied_root =
        crate::infrastructure::platform::filesystem::strip_windows_extended_length_prefix(root);
    if canonical_root != supplied_root {
        return Err(ContainedFileError::RootNotCanonical);
    }
    let root = canonical_root.as_path();
    let root_metadata =
        std::fs::symlink_metadata(root).map_err(|source| ContainedFileError::Io {
            operation: "inspect source root",
            source,
        })?;
    if crate::infrastructure::platform::filesystem::metadata_is_link_or_reparse_point(
        &root_metadata,
    ) || !root_metadata.file_type().is_dir()
    {
        return Err(ContainedFileError::RootNotDirectory);
    }

    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let relative = candidate
        .strip_prefix(root)
        .map_err(|_| ContainedFileError::PathOutsideRoot)?;
    if !host_relative_path_is_unambiguous(relative) {
        return Err(ContainedFileError::AmbiguousHostPath);
    }
    let relative_path =
        PortableRelativePath::parse(relative).map_err(ContainedFileError::InvalidRelativePath)?;

    let pre_open_identity = identity_at_path(&candidate)?;
    pre_open_observer();
    let file = open_no_follow(&candidate)?;
    let opened_metadata = file.metadata().map_err(|source| ContainedFileError::Io {
        operation: "inspect opened file",
        source,
    })?;
    validate_regular_metadata(&opened_metadata)?;
    let opened_identity = identity_from_open_file(&file, &opened_metadata)?;
    if pre_open_identity != opened_identity {
        return Err(ContainedFileError::IdentityChanged);
    }
    post_open_observer();
    let metadata_exceeds_limit = opened_metadata.len() > max_bytes;
    let mut bytes = Vec::new();
    if !metadata_exceeds_limit {
        (&file)
            .take(max_bytes.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|source| ContainedFileError::Io {
                operation: "read file",
                source,
            })?;
    }
    let bytes_read = u64::try_from(bytes.len()).map_err(|_| ContainedFileError::LengthOverflow)?;
    let read_exceeds_limit = bytes_read > max_bytes;

    if identity_at_path(&candidate)? != opened_identity {
        return Err(ContainedFileError::IdentityChanged);
    }
    let handle_path = final_opened_file_path(&file)?;
    let resolved_path = std::fs::canonicalize(&handle_path)
        .map(|path| {
            crate::infrastructure::platform::filesystem::strip_windows_extended_length_prefix(&path)
        })
        .map_err(|source| ContainedFileError::Io {
            operation: "resolve opened file path",
            source,
        })?;
    if !resolved_path.starts_with(root) {
        return Err(ContainedFileError::FinalPathOutsideRoot);
    }
    let final_relative = resolved_path
        .strip_prefix(root)
        .map_err(|_| ContainedFileError::FinalPathOutsideRoot)?;
    let final_portable = PortableRelativePath::parse(final_relative)
        .map_err(ContainedFileError::InvalidRelativePath)?;
    if final_portable != relative_path {
        return Err(ContainedFileError::FinalPathMismatch);
    }
    let final_metadata =
        std::fs::symlink_metadata(&resolved_path).map_err(|source| ContainedFileError::Io {
            operation: "inspect resolved opened file",
            source,
        })?;
    validate_regular_metadata(&final_metadata)?;
    if identity_at_path(&resolved_path)? != opened_identity
        || identity_at_path(&candidate)? != opened_identity
    {
        return Err(ContainedFileError::IdentityChanged);
    }
    if metadata_exceeds_limit || read_exceeds_limit {
        return Err(ContainedFileError::SizeLimitExceeded { limit: max_bytes });
    }

    let raw_sha256 = ContentHash::sha256(&bytes);
    Ok(VerifiedFile {
        relative_path,
        bytes,
        raw_sha256,
        bytes_read,
        identity: opened_identity,
    })
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

fn validate_regular_metadata(metadata: &Metadata) -> Result<(), ContainedFileError> {
    if crate::infrastructure::platform::filesystem::metadata_is_link_or_reparse_point(metadata) {
        return Err(ContainedFileError::SymlinkOrReparsePoint);
    }
    if !metadata.file_type().is_file() {
        return Err(ContainedFileError::NotRegularFile);
    }
    Ok(())
}

#[cfg(unix)]
fn identity_at_path(path: &Path) -> Result<VerifiedIdentity, ContainedFileError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|source| ContainedFileError::Io {
        operation: "inspect file path identity",
        source,
    })?;
    validate_regular_metadata(&metadata)?;
    identity_from_path_metadata(&metadata)
}

#[cfg(windows)]
fn identity_at_path(path: &Path) -> Result<VerifiedIdentity, ContainedFileError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|source| ContainedFileError::Io {
        operation: "inspect file path identity",
        source,
    })?;
    validate_regular_metadata(&metadata)?;
    let file = open_no_follow(path)?;
    identity_from_open_file(&file, &metadata)
}

#[cfg(not(any(unix, windows)))]
fn identity_at_path(_path: &Path) -> Result<VerifiedIdentity, ContainedFileError> {
    Err(ContainedFileError::UnsupportedHost)
}

#[cfg(unix)]
fn identity_from_path_metadata(
    metadata: &Metadata,
) -> Result<VerifiedIdentity, ContainedFileError> {
    use std::os::unix::fs::MetadataExt;

    Ok(VerifiedIdentity {
        storage: metadata.dev(),
        object: metadata.ino(),
    })
}

#[cfg(not(any(unix, windows)))]
fn identity_from_path_metadata(
    _metadata: &Metadata,
) -> Result<VerifiedIdentity, ContainedFileError> {
    Err(ContainedFileError::UnsupportedHost)
}

#[cfg(unix)]
fn identity_from_open_file(
    _file: &File,
    metadata: &Metadata,
) -> Result<VerifiedIdentity, ContainedFileError> {
    identity_from_path_metadata(metadata)
}

#[cfg(windows)]
fn identity_from_open_file(
    file: &File,
    _metadata: &Metadata,
) -> Result<VerifiedIdentity, ContainedFileError> {
    use std::mem::MaybeUninit;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };

    let mut information = MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
    // SAFETY: file owns a valid handle and the pointer targets writable storage initialized by a
    // successful GetFileInformationByHandle call.
    let succeeded =
        unsafe { GetFileInformationByHandle(file.as_raw_handle(), information.as_mut_ptr()) };
    if succeeded == 0 {
        return Err(ContainedFileError::Io {
            operation: "inspect opened Windows file identity",
            source: io::Error::last_os_error(),
        });
    }
    // SAFETY: a nonzero result guarantees complete initialization of the information structure.
    let information = unsafe { information.assume_init() };
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    if information.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(ContainedFileError::SymlinkOrReparsePoint);
    }
    Ok(VerifiedIdentity {
        storage: u64::from(information.dwVolumeSerialNumber),
        object: (u64::from(information.nFileIndexHigh) << 32)
            | u64::from(information.nFileIndexLow),
    })
}

#[cfg(not(any(unix, windows)))]
fn identity_from_open_file(
    _file: &File,
    _metadata: &Metadata,
) -> Result<VerifiedIdentity, ContainedFileError> {
    Err(ContainedFileError::UnsupportedHost)
}

#[cfg(unix)]
fn open_no_follow(path: &Path) -> Result<File, ContainedFileError> {
    use std::os::unix::fs::OpenOptionsExt;

    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(path)
        .map_err(|source| {
            if source.raw_os_error() == Some(libc::ELOOP) {
                ContainedFileError::SymlinkOrReparsePoint
            } else if source.kind() == io::ErrorKind::NotFound {
                ContainedFileError::IdentityChanged
            } else {
                ContainedFileError::Io {
                    operation: "open file without following links",
                    source,
                }
            }
        })
}

#[cfg(windows)]
fn open_no_follow(path: &Path) -> Result<File, ContainedFileError> {
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
                ContainedFileError::IdentityChanged
            } else {
                ContainedFileError::Io {
                    operation: "open Windows file without following reparse points",
                    source,
                }
            }
        })
}

#[cfg(not(any(unix, windows)))]
fn open_no_follow(_path: &Path) -> Result<File, ContainedFileError> {
    Err(ContainedFileError::UnsupportedHost)
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn final_opened_file_path(file: &File) -> Result<PathBuf, ContainedFileError> {
    use std::os::fd::AsRawFd;

    std::fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd())).map_err(|source| {
        ContainedFileError::Io {
            operation: "resolve opened Unix file handle",
            source,
        }
    })
}

#[cfg(target_os = "macos")]
fn final_opened_file_path(file: &File) -> Result<PathBuf, ContainedFileError> {
    use std::ffi::CStr;
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStrExt;

    let mut buffer = vec![0_i8; libc::PATH_MAX as usize];
    // SAFETY: file owns a valid descriptor and buffer is writable for PATH_MAX bytes for the
    // duration of fcntl. F_GETPATH writes a NUL-terminated pathname on success.
    let result = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETPATH, buffer.as_mut_ptr()) };
    if result == -1 {
        return Err(ContainedFileError::Io {
            operation: "resolve opened macOS file handle",
            source: io::Error::last_os_error(),
        });
    }
    // SAFETY: successful F_GETPATH initialized buffer with a NUL-terminated C string.
    let path = unsafe { CStr::from_ptr(buffer.as_ptr()) };
    Ok(PathBuf::from(std::ffi::OsStr::from_bytes(path.to_bytes())))
}

#[cfg(all(
    unix,
    not(any(target_os = "linux", target_os = "android", target_os = "macos"))
))]
fn final_opened_file_path(_file: &File) -> Result<PathBuf, ContainedFileError> {
    Err(ContainedFileError::UnsupportedHost)
}

#[cfg(windows)]
fn final_opened_file_path(file: &File) -> Result<PathBuf, ContainedFileError> {
    use std::os::windows::ffi::OsStringExt;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFinalPathNameByHandleW, FILE_NAME_NORMALIZED, VOLUME_NAME_DOS,
    };

    let mut buffer = vec![0_u16; 512];
    loop {
        let buffer_length =
            u32::try_from(buffer.len()).map_err(|_| ContainedFileError::LengthOverflow)?;
        // SAFETY: file owns a valid handle and buffer provides writable UTF-16 storage for the
        // declared length for the duration of this call.
        let length = unsafe {
            GetFinalPathNameByHandleW(
                file.as_raw_handle(),
                buffer.as_mut_ptr(),
                buffer_length,
                FILE_NAME_NORMALIZED | VOLUME_NAME_DOS,
            )
        };
        if length == 0 {
            return Err(ContainedFileError::Io {
                operation: "resolve opened Windows file handle",
                source: io::Error::last_os_error(),
            });
        }
        let length = usize::try_from(length).map_err(|_| ContainedFileError::LengthOverflow)?;
        if length < buffer.len() {
            buffer.truncate(length);
            let path = PathBuf::from(std::ffi::OsString::from_wide(&buffer));
            return Ok(
                crate::infrastructure::platform::filesystem::strip_windows_extended_length_prefix(
                    &path,
                ),
            );
        }
        let required = length
            .checked_add(1)
            .ok_or(ContainedFileError::LengthOverflow)?;
        buffer.resize(required, 0);
    }
}

#[cfg(not(any(unix, windows)))]
fn final_opened_file_path(_file: &File) -> Result<PathBuf, ContainedFileError> {
    Err(ContainedFileError::UnsupportedHost)
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NonRegularFixtureOutcome {
    Created,
    Unsupported,
}

#[cfg(all(test, unix))]
pub(crate) fn create_non_regular_fixture_for_test(
    path: &Path,
) -> io::Result<NonRegularFixtureOutcome> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let path = CString::new(path.as_os_str().as_bytes())
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    // SAFETY: path is a live NUL-terminated pathname and the supplied mode is valid.
    let result = unsafe { libc::mkfifo(path.as_ptr(), 0o600) };
    if result == 0 {
        Ok(NonRegularFixtureOutcome::Created)
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(all(test, not(unix)))]
pub(crate) fn create_non_regular_fixture_for_test(
    _path: &Path,
) -> io::Result<NonRegularFixtureOutcome> {
    Ok(NonRegularFixtureOutcome::Unsupported)
}

#[cfg(test)]
mod tests {
    use super::{
        read_contained_regular_file, read_contained_regular_file_with_observer,
        read_contained_regular_file_with_post_open_observer, ContainedFileError,
    };
    use crate::domain::discovery::{ContentHash, PortableRelativePath};
    use crate::infrastructure::platform::testing::{
        create_dir_symlink_for_test, create_file_link_fixture_for_test, FileLinkFixtureOutcome,
    };
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    #[test]
    fn reads_exact_raw_bytes_from_a_contained_regular_file() {
        let root = fixture_root("regular");
        let path = root.join("Objects/Document.xml");
        fs::create_dir_all(path.parent().expect("fixture file parent")).expect("fixture parent");
        fs::write(&path, b"<Document/>\r\n").expect("fixture file");

        let verified = read_contained_regular_file(&root, &path, 1_024).expect("verified read");

        assert_eq!(
            verified.relative_path,
            PortableRelativePath::parse_str("Objects/Document.xml").expect("portable path")
        );
        assert_eq!(verified.bytes, b"<Document/>\r\n");
        assert_eq!(verified.bytes_read, 13);
        assert_eq!(verified.raw_sha256, ContentHash::sha256(b"<Document/>\r\n"));
        cleanup(&root);
    }

    #[test]
    fn raw_hash_and_byte_count_preserve_bom_and_line_endings() {
        let bom_lf = read_fixture("bom-lf", &[0xef, 0xbb, 0xbf, b'a', b'\n']);
        let plain_crlf = read_fixture("plain-crlf", b"a\r\n");

        assert_ne!(bom_lf.raw_sha256, plain_crlf.raw_sha256);
        assert_eq!(bom_lf.bytes_read, 5);
        assert_eq!(bom_lf.bytes, &[0xef, 0xbb, 0xbf, b'a', b'\n']);
        assert_eq!(plain_crlf.bytes, b"a\r\n");
    }

    #[test]
    fn rejects_an_absolute_path_outside_the_canonical_root() {
        let root = fixture_root("outside-root");
        let outside = fixture_root("outside-file");
        let path = outside.join("Document.xml");
        fs::write(&path, b"outside").expect("outside file");

        let error = read_contained_regular_file(&root, &path, 1_024)
            .expect_err("outside file must not be read");

        assert!(matches!(error, ContainedFileError::PathOutsideRoot));
        cleanup(&root);
        cleanup(&outside);
    }

    #[test]
    fn exposes_a_stable_neutral_identity_for_the_opened_file() {
        let root = fixture_root("stable-identity");
        let path = root.join("Document.xml");
        fs::write(&path, b"identity").expect("fixture file");

        let first = read_contained_regular_file(&root, &path, 1_024).expect("first read");
        let second = read_contained_regular_file(&root, &path, 1_024).expect("second read");

        assert_eq!(first.identity, second.identity);
        assert_ne!(first.identity.object, 0);
        cleanup(&root);
    }

    #[test]
    fn rejects_a_symlink_or_reparse_point_file() {
        let root = fixture_root("file-link");
        let target = root.join("target.xml");
        let link = root.join("link.xml");
        fs::write(&target, b"target").expect("link target");
        let outcome = create_file_link_fixture_for_test(&target, &link).expect("link fixture");
        if outcome != FileLinkFixtureOutcome::Created {
            cleanup(&root);
            return;
        }

        let error =
            read_contained_regular_file(&root, &link, 1_024).expect_err("link must not be read");

        assert!(matches!(error, ContainedFileError::SymlinkOrReparsePoint));
        cleanup(&root);
    }

    #[test]
    fn rejects_a_file_reached_through_an_escaping_directory_link() {
        let root = fixture_root("directory-link");
        let outside = fixture_root("directory-link-outside");
        let outside_file = outside.join("Document.xml");
        fs::write(&outside_file, b"outside").expect("outside file");
        let linked_directory = root.join("linked");
        let Some(link_result) = create_dir_symlink_for_test(&outside, &linked_directory) else {
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

        let error =
            read_contained_regular_file(&root, &linked_directory.join("Document.xml"), 1_024)
                .expect_err("escaping directory link must not be read");

        assert!(matches!(error, ContainedFileError::FinalPathOutsideRoot));
        cleanup(&root);
        cleanup(&outside);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_a_fifo_without_opening_it() {
        let root = fixture_root("fifo");
        let fifo = root.join("stream.xml");
        assert_eq!(
            super::create_non_regular_fixture_for_test(&fifo).expect("FIFO fixture"),
            super::NonRegularFixtureOutcome::Created
        );

        let error = read_contained_regular_file(&root, &fifo, 1_024)
            .expect_err("FIFO must be rejected before open");

        assert!(matches!(error, ContainedFileError::NotRegularFile));
        cleanup(&root);
    }

    #[test]
    fn rejects_a_regular_file_that_exceeds_the_per_read_bound() {
        let root = fixture_root("size-bound");
        let path = root.join("Module.bsl");
        fs::write(&path, b"12345").expect("fixture file");

        let error = read_contained_regular_file(&root, &path, 4)
            .expect_err("oversized file must be bounded");

        assert!(matches!(
            error,
            ContainedFileError::SizeLimitExceeded { limit: 4 }
        ));
        cleanup(&root);
    }

    #[test]
    fn rejects_a_source_root_that_is_not_canonical() {
        let container = fixture_root("noncanonical-root");
        let actual = container.join("actual");
        let alias = container.join("alias");
        fs::create_dir(&actual).expect("actual root");
        let Some(link_result) = create_dir_symlink_for_test(&actual, &alias) else {
            cleanup(&container);
            return;
        };
        if let Err(error) = link_result {
            if error.raw_os_error() == Some(1_314) {
                cleanup(&container);
                return;
            }
            panic!("root-link fixture failed: {error}");
        }
        let path = alias.join("Document.xml");
        fs::write(&path, b"document").expect("fixture file");

        let error = read_contained_regular_file(&alias, &path, 1_024)
            .expect_err("noncanonical root must fail closed");

        assert!(matches!(error, ContainedFileError::RootNotCanonical));
        cleanup(&container);
    }

    #[test]
    fn rejects_path_replacement_between_precheck_and_open() {
        let root = fixture_root("path-swap");
        let path = root.join("Document.xml");
        let displaced = root.join("displaced.xml");
        fs::write(&path, b"first").expect("fixture file");

        let error = read_contained_regular_file_with_observer(&root, &path, 1_024, || {
            fs::rename(&path, &displaced).expect("displace original");
            fs::write(&path, b"replacement").expect("replacement file");
        })
        .expect_err("path replacement must fail closed");

        assert!(matches!(error, ContainedFileError::IdentityChanged));
        cleanup(&root);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_link_replacement_between_precheck_and_open_as_security_violation() {
        use std::os::unix::fs::symlink;

        let root = fixture_root("link-swap");
        let path = root.join("Document.xml");
        let target = root.join("target.xml");
        fs::write(&path, b"first").expect("fixture file");
        fs::write(&target, b"target").expect("link target");

        let error = read_contained_regular_file_with_observer(&root, &path, 1_024, || {
            fs::remove_file(&path).expect("remove original");
            symlink(&target, &path).expect("replacement link");
        })
        .expect_err("link replacement must fail closed");

        assert!(matches!(error, ContainedFileError::SymlinkOrReparsePoint));
        cleanup(&root);
    }

    #[test]
    fn rejects_path_replacement_after_open_and_before_postcheck() {
        let root = fixture_root("post-open-swap");
        let path = root.join("Document.xml");
        let displaced = root.join("displaced.xml");
        fs::write(&path, b"first").expect("fixture file");

        let error =
            read_contained_regular_file_with_post_open_observer(&root, &path, 1_024, || {
                fs::rename(&path, &displaced).expect("displace opened file");
                fs::write(&path, b"replacement").expect("replacement file");
            })
            .expect_err("post-open path replacement must fail closed");

        assert!(matches!(error, ContainedFileError::IdentityChanged));
        cleanup(&root);
    }

    #[test]
    fn identity_violation_takes_precedence_over_a_size_bound() {
        let root = fixture_root("swap-over-size-bound");
        let path = root.join("Document.xml");
        let displaced = root.join("displaced.xml");
        fs::write(&path, b"oversized").expect("fixture file");

        let error = read_contained_regular_file_with_post_open_observer(&root, &path, 1, || {
            fs::rename(&path, &displaced).expect("displace opened file");
            fs::write(&path, b"replacement").expect("replacement file");
        })
        .expect_err("identity violation must outrank the size bound");

        assert!(matches!(error, ContainedFileError::IdentityChanged));
        cleanup(&root);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_a_unix_filename_that_would_alias_a_portable_separator() {
        let root = fixture_root("backslash-alias");
        let path = root.join("a\\b.xml");
        fs::write(&path, b"ambiguous").expect("fixture file");

        let error = read_contained_regular_file(&root, &path, 1_024)
            .expect_err("host path must map uniquely to its portable identity");

        assert!(matches!(error, ContainedFileError::AmbiguousHostPath));
        cleanup(&root);
    }

    fn fixture_root(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "unica-contained-file-{label}-{}-{nanos}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).expect("fixture root");
        fs::canonicalize(root).expect("canonical fixture root")
    }

    fn cleanup(root: &Path) {
        fs::remove_dir_all(root).expect("fixture cleanup");
    }

    fn read_fixture(label: &str, bytes: &[u8]) -> super::VerifiedFile {
        let root = fixture_root(label);
        let path = root.join("fixture.bsl");
        fs::write(&path, bytes).expect("fixture file");
        let verified = read_contained_regular_file(&root, &path, 1_024).expect("verified fixture");
        cleanup(&root);
        verified
    }
}
