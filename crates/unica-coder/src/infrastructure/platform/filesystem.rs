use std::fs;
#[cfg(unix)]
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[cfg(test)]
thread_local! {
    static TEST_POST_RENAME_SYNC_FAILURE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
pub(crate) fn inject_post_rename_sync_failure_for_test() {
    TEST_POST_RENAME_SYNC_FAILURE.with(|slot| slot.set(true));
}

/// Returns a short, private directory suitable as a child process' Unix runtime
/// directory.  It deliberately lives below `/tmp`: macOS's `TMPDIR` and a
/// caller's `XDG_RUNTIME_DIR` can both be too long once a Unix socket name is
/// appended.
pub(crate) fn short_private_runtime_dir() -> io::Result<Option<PathBuf>> {
    #[cfg(unix)]
    {
        short_private_runtime_dir_unix().map(Some)
    }
    #[cfg(not(unix))]
    {
        Ok(None)
    }
}

#[cfg(unix)]
fn short_private_runtime_dir_unix() -> io::Result<PathBuf> {
    // SAFETY: `geteuid` has no preconditions and only reads the effective UID
    // of this process.
    let uid = unsafe { libc::geteuid() };
    let path = PathBuf::from("/tmp").join(format!("unica-bsl-{uid}"));
    ensure_short_private_runtime_dir_unix(&path, uid)
}

#[cfg(unix)]
fn runtime_directory_permissions_error(path: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!(
            "short runtime directory {} must be owned by the current user and have mode 0700",
            path.display()
        ),
    )
}

#[cfg(unix)]
fn runtime_directory_metadata_is_ready(
    path: &Path,
    metadata: &fs::Metadata,
    uid: libc::uid_t,
) -> io::Result<bool> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("short runtime path {} is not a directory", path.display()),
        ));
    }
    if metadata.uid() != uid {
        return Err(runtime_directory_permissions_error(path));
    }

    Ok(metadata.permissions().mode() & 0o777 == 0o700)
}

#[cfg(unix)]
fn ensure_short_private_runtime_dir_unix(path: &Path, uid: libc::uid_t) -> io::Result<PathBuf> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    use std::os::unix::io::FromRawFd;
    use std::time::Duration;

    const SETUP_ATTEMPTS: usize = 8;
    const RETRY_DELAY: Duration = Duration::from_millis(1);

    for _ in 0..SETUP_ATTEMPTS {
        match fs::symlink_metadata(path) {
            Ok(metadata) => {
                if runtime_directory_metadata_is_ready(path, &metadata, uid)? {
                    return Ok(path.to_path_buf());
                }

                // Another process with the same UID can observe a directory
                // between its creation and the creator's permission
                // normalization. Give that bounded race time to settle.
                std::thread::sleep(RETRY_DELAY);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                match fs::DirBuilder::new().mode(0o700).create(path) {
                    Ok(()) => {
                        // `DirBuilder::mode` is filtered through the caller's
                        // umask. The directory is part of an authentication
                        // boundary for the local socket, so normalize it before
                        // accepting the newly created path.
                        let encoded =
                            CString::new(path.as_os_str().as_bytes()).map_err(|error| {
                                io::Error::new(
                                    io::ErrorKind::InvalidInput,
                                    format!("short runtime path contains an embedded NUL: {error}"),
                                )
                            })?;
                        // SAFETY: `encoded` is NUL-terminated and remains live for
                        // the call. `O_NOFOLLOW` prevents a raced symlink from being
                        // opened before the directory descriptor is owned below.
                        let descriptor = unsafe {
                            libc::open(
                                encoded.as_ptr(),
                                libc::O_RDONLY
                                    | libc::O_DIRECTORY
                                    | libc::O_CLOEXEC
                                    | libc::O_NOFOLLOW,
                            )
                        };
                        if descriptor < 0 {
                            return Err(io::Error::last_os_error());
                        }
                        // SAFETY: `open` returned a new owned descriptor.
                        let directory = unsafe { File::from_raw_fd(descriptor) };
                        directory.set_permissions(fs::Permissions::from_mode(0o700))?;
                    }
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                        std::thread::sleep(RETRY_DELAY);
                    }
                    Err(error) => return Err(error),
                }
            }
            Err(error) => return Err(error),
        }
    }

    // Always validate once after the retry budget. In particular, a successful
    // creation on the last iteration must not be reported as disappeared.
    match fs::symlink_metadata(path) {
        Ok(metadata) if runtime_directory_metadata_is_ready(path, &metadata, uid)? => {
            Ok(path.to_path_buf())
        }
        Ok(_) => Err(runtime_directory_permissions_error(path)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "short runtime directory {} disappeared during setup",
                path.display()
            ),
        )),
        Err(error) => Err(error),
    }
}

#[cfg(all(test, unix))]
pub(crate) fn create_test_directory_link(target: &Path, link: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(all(test, windows))]
pub(crate) fn create_test_directory_link(target: &Path, link: &Path) -> io::Result<()> {
    std::os::windows::fs::symlink_dir(target, link)
}

#[cfg(all(test, windows))]
thread_local! {
    static TEST_CASE_SENSITIVITY_QUERY_ERROR: std::cell::Cell<Option<u32>> = const { std::cell::Cell::new(None) };
}

#[cfg(all(test, windows))]
fn with_case_sensitivity_query_error<T>(error: u32, action: impl FnOnce() -> T) -> T {
    struct Reset(Option<u32>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_CASE_SENSITIVITY_QUERY_ERROR.with(|slot| slot.set(self.0));
        }
    }

    let previous = TEST_CASE_SENSITIVITY_QUERY_ERROR.with(|slot| slot.replace(Some(error)));
    let _reset = Reset(previous);
    action()
}

#[cfg(all(test, not(any(unix, windows))))]
pub(crate) fn create_test_directory_link(_target: &Path, _link: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "test directory links are unavailable on this host",
    ))
}

#[derive(Debug, Clone)]
pub(crate) struct PortablePermissions {
    permissions: fs::Permissions,
    key: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct FileIdentity {
    volume: u64,
    file: u64,
}

impl FileIdentity {
    pub(crate) fn stable_bytes(self) -> [u8; 16] {
        let mut bytes = [0_u8; 16];
        bytes[..8].copy_from_slice(&self.volume.to_le_bytes());
        bytes[8..].copy_from_slice(&self.file.to_le_bytes());
        bytes
    }
}

/// Retained, no-follow capability for one named absolute directory.
///
/// The descriptor keeps the originally admitted directory available for
/// descriptor-relative reads. `validate_named_identity` separately proves
/// that the current namespace entry still names that same physical object;
/// callers must check it both before and after ambient path-based work.
#[derive(Clone)]
pub(crate) struct RetainedDirectoryCapability {
    path: PathBuf,
    retained: Arc<RetainedDirectoryCapabilityInner>,
    parent: Option<Arc<RetainedDirectoryParent>>,
}

/// Retained no-follow authority for one regular child of an admitted directory.
/// The open descriptor is the physical identity; validation proves the current
/// name still resolves to that exact object before a caller publishes or joins.
#[derive(Debug)]
pub(crate) struct RetainedRegularFileCapability {
    parent: RetainedDirectoryCapability,
    name: std::ffi::OsString,
    file: fs::File,
    identity: FileIdentity,
}

struct RetainedDirectoryCapabilityInner {
    directory: fs::File,
    identity: FileIdentity,
}

struct RetainedDirectoryParent {
    directory: RetainedDirectoryCapability,
    name: std::ffi::OsString,
}

impl std::fmt::Debug for RetainedDirectoryCapability {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RetainedDirectoryCapability")
            .field("path", &self.path)
            .field("identity", &self.retained.identity)
            .finish()
    }
}

impl RetainedDirectoryCapability {
    pub(crate) fn open(path: &Path) -> io::Result<Self> {
        let directory = open_absolute_directory_path_nofollow(path)?;
        let identity = file_identity(&directory)?;
        Ok(Self {
            path: path.to_path_buf(),
            retained: Arc::new(RetainedDirectoryCapabilityInner {
                directory,
                identity,
            }),
            parent: None,
        })
    }

    pub(crate) fn open_or_create(path: &Path) -> io::Result<Self> {
        let directory = open_or_create_absolute_directory_path_nofollow(path)?;
        let identity = file_identity(&directory)?;
        Ok(Self {
            path: path.to_path_buf(),
            retained: Arc::new(RetainedDirectoryCapabilityInner {
                directory,
                identity,
            }),
            parent: None,
        })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn identity(&self) -> FileIdentity {
        self.retained.identity
    }

    pub(crate) fn validate_named_identity(&self) -> io::Result<()> {
        let rebound = if let Some(parent) = &self.parent {
            parent.directory.validate_named_identity()?;
            open_directory_child_nofollow(&parent.directory.retained.directory, &parent.name)?
        } else {
            open_absolute_directory_path_nofollow(&self.path)?
        };
        let rebound_identity = file_identity(&rebound)?;
        if rebound_identity != self.retained.identity {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "named directory identity changed after capability admission",
            ));
        }
        Ok(())
    }

    /// Retains one no-follow child directory relative to this exact directory
    /// descriptor. Its later validation also walks through the retained parent
    /// descriptor rather than reopening the child through an ambient path.
    pub(crate) fn retain_directory_child(&self, name: &std::ffi::OsStr) -> io::Result<Self> {
        let directory = open_directory_child_nofollow(&self.retained.directory, name)?;
        let identity = file_identity(&directory)?;
        Ok(Self {
            path: self.path.join(name),
            retained: Arc::new(RetainedDirectoryCapabilityInner {
                directory,
                identity,
            }),
            parent: Some(Arc::new(RetainedDirectoryParent {
                directory: self.clone(),
                name: name.to_os_string(),
            })),
        })
    }

    /// Publishes bytes atomically inside this retained directory. Both the
    /// staging child and rename destination are resolved through the retained
    /// directory handle, so replacing the lexical path cannot redirect bytes.
    pub(crate) fn replace_regular_child_atomically(
        &self,
        stage_name: &std::ffi::OsStr,
        destination_name: &std::ffi::OsStr,
        bytes: &[u8],
    ) -> io::Result<RetainedRegularFileCapability> {
        use std::io::Write;

        let mut stage = create_new_regular_child(&self.retained.directory, stage_name)?;
        let identity = file_identity(&stage)?;
        let publication = (|| {
            stage.write_all(bytes)?;
            stage.sync_data()?;
            replace_identity_bound_regular_child(
                &self.retained.directory,
                stage_name,
                identity,
                &stage,
                destination_name,
            )?;
            sync_renamed_regular_child(&stage)?;
            sync_directory(&self.retained.directory)?;
            let published = RetainedRegularFileCapability {
                parent: self.clone(),
                name: destination_name.to_os_string(),
                file: stage.try_clone()?,
                identity,
            };
            published.validate_named_identity()?;
            Ok(published)
        })();
        if publication.is_err() {
            let _ = remove_identity_bound_regular_child(
                &self.retained.directory,
                stage_name,
                identity,
                &stage,
            );
        }
        publication
    }

    pub(crate) fn read_relative_regular_bounded(
        &self,
        relative: &Path,
        max_bytes: usize,
    ) -> io::Result<Vec<u8>> {
        use std::io::Read;
        use std::path::Component;

        let mut components = relative.components().peekable();
        if components.peek().is_none() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "relative file path must not be empty",
            ));
        }
        let mut directory = self.retained.directory.try_clone()?;
        let mut file = None;
        while let Some(component) = components.next() {
            let Component::Normal(name) = component else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "relative file path contains a non-normal component",
                ));
            };
            if components.peek().is_some() {
                directory = open_directory_child_nofollow(&directory, name)?;
            } else {
                file = Some(open_regular_child_nofollow(&directory, name)?);
            }
        }
        let mut file = file.expect("non-empty relative path has a final component");
        let limit = u64::try_from(max_bytes)
            .unwrap_or(u64::MAX)
            .saturating_add(1);
        let mut bytes = Vec::new();
        file.by_ref().take(limit).read_to_end(&mut bytes)?;
        if bytes.len() > max_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("relative file exceeds the {max_bytes}-byte read limit"),
            ));
        }
        Ok(bytes)
    }

    /// Enumerates the immediate members of this exact retained directory.
    /// Names are bounded and resolved from the retained descriptor, never by
    /// reopening the ambient path.
    pub(crate) fn read_immediate_names_bounded(
        &self,
        maximum_entries: usize,
        checkpoint: impl FnMut() -> io::Result<()>,
    ) -> io::Result<Vec<std::ffi::OsString>> {
        read_directory_names_bounded(&self.retained.directory, maximum_entries, checkpoint)
    }

    pub(crate) fn retain_regular_child(
        &self,
        name: &std::ffi::OsStr,
    ) -> io::Result<RetainedRegularFileCapability> {
        let file = open_regular_child_nofollow(&self.retained.directory, name)?;
        let identity = file_identity(&file)?;
        Ok(RetainedRegularFileCapability {
            parent: self.clone(),
            name: name.to_os_string(),
            file,
            identity,
        })
    }

    pub(crate) fn retain_or_create_regular_child(
        &self,
        name: &std::ffi::OsStr,
    ) -> io::Result<RetainedRegularFileCapability> {
        let file =
            open_or_create_regular_child_read_write_nofollow(&self.retained.directory, name)?;
        let identity = file_identity(&file)?;
        Ok(RetainedRegularFileCapability {
            parent: self.clone(),
            name: name.to_os_string(),
            file,
            identity,
        })
    }
}

fn sync_renamed_regular_child(file: &fs::File) -> io::Result<()> {
    #[cfg(test)]
    if TEST_POST_RENAME_SYNC_FAILURE.with(|slot| slot.replace(false)) {
        return Err(io::Error::other(
            "injected post-rename regular-file sync failure",
        ));
    }
    file.sync_all()
}

impl RetainedRegularFileCapability {
    pub(crate) fn identity(&self) -> FileIdentity {
        self.identity
    }

    pub(crate) fn read_bounded(&self, max_bytes: usize) -> io::Result<Vec<u8>> {
        use std::io::{Read, Seek, SeekFrom};

        let mut file = self.file.try_clone()?;
        file.seek(SeekFrom::Start(0))?;
        let limit = u64::try_from(max_bytes)
            .unwrap_or(u64::MAX)
            .saturating_add(1);
        let mut bytes = Vec::new();
        file.by_ref().take(limit).read_to_end(&mut bytes)?;
        if bytes.len() > max_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("regular file exceeds the {max_bytes}-byte read limit"),
            ));
        }
        Ok(bytes)
    }

    pub(crate) fn try_clone_file(&self) -> io::Result<fs::File> {
        self.file.try_clone()
    }

    pub(crate) fn validate_named_identity(&self) -> io::Result<()> {
        self.parent.validate_named_identity()?;
        let rebound = open_regular_child_nofollow(&self.parent.retained.directory, &self.name)?;
        if file_identity(&rebound)? != self.identity {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "named regular-file identity changed after capability admission",
            ));
        }
        Ok(())
    }

    pub(crate) fn remove_named_identity(&self) -> io::Result<()> {
        self.parent.validate_named_identity()?;
        remove_identity_bound_regular_child(
            &self.parent.retained.directory,
            &self.name,
            self.identity,
            &self.file,
        )
    }
}

impl PortablePermissions {
    pub(crate) fn readonly(&self) -> bool {
        self.permissions.readonly()
    }

    pub(crate) fn matches(&self, metadata: &fs::Metadata) -> bool {
        self.key == portable_permission_key(metadata)
    }

    pub(crate) fn apply_to(&self, file: &fs::File) -> io::Result<()> {
        file.set_permissions(self.permissions.clone())
    }
}

pub(crate) fn portable_permissions(metadata: &fs::Metadata) -> PortablePermissions {
    PortablePermissions {
        permissions: metadata.permissions(),
        key: portable_permission_key(metadata),
    }
}

#[cfg(unix)]
fn portable_permission_key(metadata: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;

    metadata.permissions().mode() & 0o7777
}

#[cfg(not(unix))]
fn portable_permission_key(metadata: &fs::Metadata) -> u32 {
    u32::from(metadata.permissions().readonly())
}

#[cfg(unix)]
pub(crate) fn restrict_stage_to_owner(file: &fs::File) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    file.set_permissions(fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
pub(crate) fn restrict_stage_to_owner(_file: &fs::File) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
pub(crate) fn hard_link_count(file: &fs::File) -> io::Result<u64> {
    use std::os::unix::fs::MetadataExt;

    Ok(file.metadata()?.nlink())
}

#[cfg(unix)]
pub(crate) fn file_identity(file: &fs::File) -> io::Result<FileIdentity> {
    use std::os::unix::fs::MetadataExt;

    let metadata = file.metadata()?;
    Ok(FileIdentity {
        volume: metadata.dev(),
        file: metadata.ino(),
    })
}

#[cfg(windows)]
pub(crate) fn hard_link_count(file: &fs::File) -> io::Result<u64> {
    Ok(u64::from(windows_file_information(file)?.nNumberOfLinks))
}

#[cfg(windows)]
pub(crate) fn file_identity(file: &fs::File) -> io::Result<FileIdentity> {
    let information = windows_file_information(file)?;
    Ok(FileIdentity {
        volume: u64::from(information.dwVolumeSerialNumber),
        file: (u64::from(information.nFileIndexHigh) << 32) | u64::from(information.nFileIndexLow),
    })
}

#[cfg(windows)]
fn windows_file_information(
    file: &fs::File,
) -> io::Result<windows_sys::Win32::Storage::FileSystem::BY_HANDLE_FILE_INFORMATION> {
    use std::mem::MaybeUninit;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };

    let mut information = MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
    // SAFETY: the file owns a valid handle, and the pointer provides writable storage that the
    // API fully initializes before returning a nonzero result.
    let succeeded =
        unsafe { GetFileInformationByHandle(file.as_raw_handle(), information.as_mut_ptr()) };
    if succeeded == 0 {
        Err(io::Error::last_os_error())
    } else {
        // SAFETY: a nonzero result guarantees that GetFileInformationByHandle initialized the
        // entire BY_HANDLE_FILE_INFORMATION value.
        Ok(unsafe { information.assume_init() })
    }
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn hard_link_count(_file: &fs::File) -> io::Result<u64> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "hard-link count is not available on this host",
    ))
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn file_identity(_file: &fs::File) -> io::Result<FileIdentity> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "file identity is not available on this host",
    ))
}

#[cfg(unix)]
fn unix_child_name(name: &std::ffi::OsStr) -> io::Result<std::ffi::CString> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::path::Component;

    let mut components = Path::new(name).components();
    if !matches!(components.next(), Some(Component::Normal(component)) if component == name)
        || components.next().is_some()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "child name must be exactly one normal relative component",
        ));
    }
    CString::new(name.as_bytes()).map_err(Into::into)
}

#[cfg(unix)]
fn open_unix_child(
    parent: &fs::File,
    name: &std::ffi::OsStr,
    flags: libc::c_int,
) -> io::Result<fs::File> {
    use std::os::fd::{AsRawFd, FromRawFd};

    let encoded_name = unix_child_name(name)?;
    // SAFETY: parent owns a live descriptor, name is a NUL-terminated single component, and a
    // successful openat result transfers one newly owned descriptor to this function.
    let descriptor = unsafe { libc::openat(parent.as_raw_fd(), encoded_name.as_ptr(), flags) };
    if descriptor < 0 {
        Err(io::Error::last_os_error())
    } else {
        // SAFETY: descriptor is a newly owned successful openat result.
        Ok(unsafe { fs::File::from_raw_fd(descriptor) })
    }
}

#[cfg(unix)]
pub(crate) fn create_new_regular_child(
    parent: &fs::File,
    name: &std::ffi::OsStr,
) -> io::Result<fs::File> {
    use std::os::fd::{AsRawFd, FromRawFd};

    let encoded_name = unix_child_name(name)?;
    // SAFETY: parent owns a live descriptor, name is one NUL-terminated child component, and a
    // successful openat result transfers one newly owned descriptor to this function. Mode 0666
    // preserves the process-umask default captured by the publication protocol before the file is
    // restricted to owner-only while its bytes are initialized.
    let descriptor = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            encoded_name.as_ptr(),
            libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            0o666,
        )
    };
    if descriptor < 0 {
        Err(io::Error::last_os_error())
    } else {
        // SAFETY: descriptor is a newly owned successful openat result.
        Ok(unsafe { fs::File::from_raw_fd(descriptor) })
    }
}

#[cfg(unix)]
pub(crate) fn create_new_directory_child(
    parent: &fs::File,
    name: &std::ffi::OsStr,
) -> io::Result<fs::File> {
    use std::os::fd::AsRawFd;

    let name_c = unix_child_name(name)?;
    // SAFETY: parent and the validated NUL-terminated child name remain live for the call. Mode
    // 0777 matches std::fs::create_dir before the process umask is applied.
    let status = unsafe { libc::mkdirat(parent.as_raw_fd(), name_c.as_ptr(), 0o777) };
    if status != 0 {
        return Err(io::Error::last_os_error());
    }
    match open_directory_child_nofollow(parent, name) {
        Ok(directory) => Ok(directory),
        Err(primary) => Err(io::Error::new(
            primary.kind(),
            format!(
                "{primary}; newly created directory identity could not be captured and was left untouched"
            ),
        )),
    }
}

#[cfg(unix)]
pub(crate) fn open_directory_nofollow(path: &Path) -> io::Result<fs::File> {
    use std::ffi::CString;
    use std::os::fd::FromRawFd;
    use std::os::unix::ffi::OsStrExt;

    let path = CString::new(path.as_os_str().as_bytes())?;
    // This ambient entry point is used only for a filesystem namespace root. Callers which open
    // an arbitrary absolute path walk its components with open_directory_child_nofollow.
    let descriptor = unsafe {
        libc::open(
            path.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if descriptor < 0 {
        Err(io::Error::last_os_error())
    } else {
        // SAFETY: descriptor is a newly owned successful open result.
        Ok(unsafe { fs::File::from_raw_fd(descriptor) })
    }
}

#[cfg(unix)]
pub(crate) fn open_absolute_directory_path_nofollow(path: &Path) -> io::Result<fs::File> {
    use std::path::Component;

    if !path.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "secure directory path must be absolute",
        ));
    }
    let mut current = open_directory_nofollow(Path::new("/"))?;
    for component in path.components() {
        match component {
            Component::RootDir | Component::CurDir => {}
            Component::Normal(name) => {
                current = open_directory_child_nofollow(&current, name)?;
            }
            Component::ParentDir | Component::Prefix(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "secure directory path contains a non-normal component",
                ));
            }
        }
    }
    Ok(current)
}

/// Opens an absolute directory path component-by-component and creates only absent directory
/// components relative to an already-retained parent. Existing links are never followed and no
/// ambient write occurs before the parent descriptor is verified.
#[cfg(unix)]
pub(crate) fn open_or_create_absolute_directory_path_nofollow(path: &Path) -> io::Result<fs::File> {
    use std::path::Component;

    if !path.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "secure directory path must be absolute",
        ));
    }
    let mut current = open_directory_nofollow(Path::new("/"))?;
    for component in path.components() {
        match component {
            Component::RootDir | Component::CurDir => {}
            Component::Normal(name) => {
                current = match open_directory_child_nofollow(&current, name) {
                    Ok(directory) => directory,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {
                        match create_new_directory_child(&current, name) {
                            Ok(directory) => {
                                sync_directory(&current)?;
                                directory
                            }
                            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                                open_directory_child_nofollow(&current, name)?
                            }
                            Err(error) => return Err(error),
                        }
                    }
                    Err(error) => return Err(error),
                };
            }
            Component::ParentDir | Component::Prefix(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "secure directory path contains a non-normal component",
                ));
            }
        }
    }
    Ok(current)
}

#[cfg(unix)]
pub(crate) fn open_directory_child_nofollow(
    parent: &fs::File,
    name: &std::ffi::OsStr,
) -> io::Result<fs::File> {
    open_unix_child(
        parent,
        name,
        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
    )
}

#[cfg(unix)]
pub(crate) fn open_regular_child_nofollow(
    parent: &fs::File,
    name: &std::ffi::OsStr,
) -> io::Result<fs::File> {
    let file = open_unix_child(
        parent,
        name,
        libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
    )?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "entry is not a regular file",
        ));
    }
    Ok(file)
}

#[cfg(unix)]
fn open_or_create_regular_child_read_write_nofollow(
    parent: &fs::File,
    name: &std::ffi::OsStr,
) -> io::Result<fs::File> {
    use std::os::fd::{AsRawFd, FromRawFd};

    let encoded_name = unix_child_name(name)?;
    // SAFETY: parent is a retained directory descriptor and name is one
    // NUL-terminated component. The returned descriptor is newly owned.
    let descriptor = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            encoded_name.as_ptr(),
            libc::O_RDWR | libc::O_CREAT | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            0o666,
        )
    };
    if descriptor < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: descriptor is the owned successful openat result above.
    let file = unsafe { fs::File::from_raw_fd(descriptor) };
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "descriptor-relative lifecycle lock is not a regular file",
        ));
    }
    Ok(file)
}

#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OpenedChildKind {
    Directory,
    RegularFile,
    #[allow(
        dead_code,
        reason = "Unix rejects links at open; Windows classifies reparse handles"
    )]
    ReparsePoint,
    Unsupported,
}

#[cfg(unix)]
pub(crate) fn opened_child_kind(file: &fs::File) -> io::Result<OpenedChildKind> {
    let file_type = file.metadata()?.file_type();
    if file_type.is_dir() {
        Ok(OpenedChildKind::Directory)
    } else if file_type.is_file() {
        Ok(OpenedChildKind::RegularFile)
    } else {
        Ok(OpenedChildKind::Unsupported)
    }
}

#[cfg(unix)]
pub(crate) fn open_any_child_nofollow(
    parent: &fs::File,
    name: &std::ffi::OsStr,
) -> io::Result<(fs::File, OpenedChildKind)> {
    let file = open_unix_child(
        parent,
        name,
        libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
    )?;
    let kind = opened_child_kind(&file)?;
    Ok((file, kind))
}

#[cfg(unix)]
pub(crate) fn open_child_for_secure_tree_use(
    _parent: &fs::File,
    _name: &std::ffi::OsStr,
    classification_anchor: fs::File,
    _kind: OpenedChildKind,
) -> io::Result<fs::File> {
    Ok(classification_anchor)
}

#[cfg(unix)]
pub(crate) fn read_directory_names_bounded(
    directory: &fs::File,
    maximum_entries: usize,
    mut checkpoint: impl FnMut() -> io::Result<()>,
) -> io::Result<Vec<std::ffi::OsString>> {
    let mut entries = cap_primitives::fs::read_base_dir(directory)?;
    let mut names = Vec::new();
    loop {
        checkpoint()?;
        let Some(entry) = entries.next() else {
            break;
        };
        let name = entry?.file_name();
        if names.len() >= maximum_entries {
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "directory exceeds the retained enumeration entry limit",
            ));
        }
        names.push(name);
    }
    names.sort();
    Ok(names)
}

#[cfg(any(test, windows))]
fn windows_api_path_from_utf16(mut path: Vec<u16>, absolute: bool) -> Vec<u16> {
    const BACKSLASH: u16 = b'\\' as u16;
    const FORWARD_SLASH: u16 = b'/' as u16;
    const QUESTION_MARK: u16 = b'?' as u16;
    const DOT: u16 = b'.' as u16;

    let has_device_prefix = path.starts_with(&[BACKSLASH, BACKSLASH, QUESTION_MARK, BACKSLASH])
        || path.starts_with(&[BACKSLASH, BACKSLASH, DOT, BACKSLASH]);
    if absolute && !has_device_prefix {
        for unit in &mut path {
            if *unit == FORWARD_SLASH {
                *unit = BACKSLASH;
            }
        }
        let mut extended = if path.starts_with(&[BACKSLASH, BACKSLASH]) {
            r"\\?\UNC\".encode_utf16().collect::<Vec<_>>()
        } else {
            r"\\?\".encode_utf16().collect::<Vec<_>>()
        };
        if path.starts_with(&[BACKSLASH, BACKSLASH]) {
            extended.extend_from_slice(&path[2..]);
        } else {
            extended.extend_from_slice(&path);
        }
        path = extended;
    }
    path.push(0);
    path
}

#[cfg(windows)]
fn windows_api_path(path: &Path) -> io::Result<Vec<u16>> {
    use std::os::windows::ffi::OsStrExt;

    let path = std::path::absolute(path)?;
    let encoded = path.as_os_str().encode_wide().collect::<Vec<_>>();
    if encoded.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Windows path contains NUL",
        ));
    }
    Ok(windows_api_path_from_utf16(encoded, true))
}

#[cfg(unix)]
pub(crate) fn verify_owner_only_acl(file: &fs::File) -> io::Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let metadata = file.metadata()?;
    // SAFETY: geteuid has no preconditions and reads only process credentials.
    let current_uid = unsafe { libc::geteuid() };
    if metadata.uid() != current_uid || metadata.permissions().mode() & 0o077 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "state object must be owned by the current user and owner-only",
        ));
    }
    Ok(())
}

#[cfg(windows)]
#[allow(
    dead_code,
    reason = "DirectoryAnchor callers are introduced by the following Windows full-dump task"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EffectiveTokenSource {
    Thread,
    Process,
}

#[cfg(windows)]
fn verify_thread_token_fallback_error(error: u32) -> io::Result<()> {
    use windows_sys::Win32::Foundation::ERROR_NO_TOKEN;

    if error == ERROR_NO_TOKEN {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(error as i32))
    }
}

#[cfg(windows)]
#[allow(
    dead_code,
    reason = "DirectoryAnchor callers are introduced by the following Windows full-dump task"
)]
struct ProcessToken {
    handle: windows_sys::Win32::Foundation::HANDLE,
    user: Vec<u8>,
    source: EffectiveTokenSource,
}

#[cfg(windows)]
impl ProcessToken {
    fn current_user() -> io::Result<Self> {
        use std::ptr;
        use windows_sys::Win32::Foundation::{CloseHandle, ERROR_NO_TOKEN};
        use windows_sys::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY};
        use windows_sys::Win32::System::Threading::{
            GetCurrentProcess, GetCurrentThread, OpenProcessToken, OpenThreadToken,
        };

        let mut handle = ptr::null_mut();
        // SAFETY: GetCurrentThread returns a valid pseudo-handle; handle is writable storage and
        // OpenAsSelf is true so an impersonating caller can inspect its effective token.
        let source =
            if unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, 1, &mut handle) } != 0 {
                EffectiveTokenSource::Thread
            } else {
                let error = io::Error::last_os_error();
                let error_code = error.raw_os_error().unwrap_or_default() as u32;
                verify_thread_token_fallback_error(error_code)?;
                debug_assert_eq!(error_code, ERROR_NO_TOKEN);
                // SAFETY: GetCurrentProcess returns a valid pseudo-handle; handle is writable storage.
                if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut handle) } == 0 {
                    return Err(io::Error::last_os_error());
                }
                EffectiveTokenSource::Process
            };

        let mut length = 0;
        // SAFETY: the first call requests the buffer size without supplying a buffer.
        unsafe {
            GetTokenInformation(handle, TokenUser, ptr::null_mut(), 0, &mut length);
        }
        if length == 0 {
            let error = io::Error::last_os_error();
            // SAFETY: OpenProcessToken returned this owned handle.
            unsafe { CloseHandle(handle) };
            return Err(error);
        }

        let mut user = vec![0; length as usize];
        // SAFETY: user provides the byte capacity requested by the preceding call.
        if unsafe {
            GetTokenInformation(
                handle,
                TokenUser,
                user.as_mut_ptr().cast(),
                length,
                &mut length,
            )
        } == 0
        {
            let error = io::Error::last_os_error();
            // SAFETY: OpenProcessToken returned this owned handle.
            unsafe { CloseHandle(handle) };
            return Err(error);
        }

        Ok(Self {
            handle,
            user,
            source,
        })
    }

    fn user_sid(&self) -> windows_sys::Win32::Security::PSID {
        use windows_sys::Win32::Security::TOKEN_USER;

        // SAFETY: GetTokenInformation initialized the buffer as TOKEN_USER, including the SID
        // pointer, and the buffer remains owned by self for this borrow.
        unsafe {
            std::ptr::read_unaligned(self.user.as_ptr().cast::<TOKEN_USER>())
                .User
                .Sid
        }
    }
}

#[cfg(windows)]
impl Drop for ProcessToken {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::CloseHandle;

        // SAFETY: self.handle is an owned token handle returned by OpenThreadToken or
        // OpenProcessToken.
        unsafe { CloseHandle(self.handle) };
    }
}

#[cfg(windows)]
#[allow(
    dead_code,
    reason = "DirectoryAnchor callers are introduced by the following Windows full-dump task"
)]
struct OwnerOnlySecurityAttributes {
    token: ProcessToken,
    sid_string: windows_sys::core::PWSTR,
    security_descriptor: windows_sys::Win32::Security::PSECURITY_DESCRIPTOR,
    attributes: windows_sys::Win32::Security::SECURITY_ATTRIBUTES,
}

#[cfg(windows)]
impl OwnerOnlySecurityAttributes {
    fn current_user() -> io::Result<Self> {
        use std::mem::size_of;
        use std::ptr;
        use windows_sys::Win32::Foundation::LocalFree;
        use windows_sys::Win32::Security::Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            SDDL_REVISION_1,
        };
        use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;

        let token = ProcessToken::current_user()?;
        let mut sid_string = ptr::null_mut();
        // SAFETY: token.user_sid() is valid while token is live, and sid_string is writable.
        if unsafe { ConvertSidToStringSidW(token.user_sid(), &mut sid_string) } == 0 {
            return Err(io::Error::last_os_error());
        }

        let mut sddl = "D:P(A;;FA;;;".encode_utf16().collect::<Vec<_>>();
        // SAFETY: ConvertSidToStringSidW returned a NUL-terminated UTF-16 string allocated by
        // LocalAlloc, which remains live until OwnerOnlySecurityAttributes is dropped.
        let sid_length = unsafe {
            let mut length = 0;
            while *sid_string.add(length) != 0 {
                length += 1;
            }
            length
        };
        // SAFETY: sid_length stops at the allocation's terminating NUL.
        sddl.extend_from_slice(unsafe { std::slice::from_raw_parts(sid_string, sid_length) });
        sddl.extend(")".encode_utf16());
        sddl.push(0);

        let mut security_descriptor = ptr::null_mut();
        // SAFETY: sddl is NUL-terminated for the duration of the call and the descriptor output
        // pointer is writable.
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut security_descriptor,
                ptr::null_mut(),
            )
        } == 0
        {
            let error = io::Error::last_os_error();
            // SAFETY: ConvertSidToStringSidW allocated sid_string with LocalAlloc.
            unsafe { LocalFree(sid_string.cast()) };
            return Err(error);
        }

        Ok(Self {
            token,
            sid_string,
            security_descriptor,
            attributes: SECURITY_ATTRIBUTES {
                nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: security_descriptor,
                bInheritHandle: 0,
            },
        })
    }

    fn as_ptr(&self) -> *const windows_sys::Win32::Security::SECURITY_ATTRIBUTES {
        let _ = self.token.handle;
        &self.attributes
    }

    fn security_descriptor(&self) -> windows_sys::Win32::Security::PSECURITY_DESCRIPTOR {
        self.security_descriptor
    }
}

#[cfg(windows)]
impl Drop for OwnerOnlySecurityAttributes {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::LocalFree;

        // SAFETY: both pointers were allocated by documented APIs with LocalAlloc semantics.
        unsafe {
            LocalFree(self.security_descriptor);
            LocalFree(self.sid_string.cast());
        }
    }
}

#[cfg(windows)]
#[allow(
    dead_code,
    reason = "DirectoryAnchor callers are introduced by the following Windows full-dump task"
)]
struct LocalSecurityDescriptor(windows_sys::Win32::Security::PSECURITY_DESCRIPTOR);

#[cfg(windows)]
impl Drop for LocalSecurityDescriptor {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::LocalFree;

        // SAFETY: GetSecurityInfo returns this descriptor through LocalAlloc.
        unsafe { LocalFree(self.0) };
    }
}

#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WindowsImmutableAclProfile {
    Ancestry,
    Installation,
}

#[cfg(windows)]
struct SelfRelativeSecurityDescriptor {
    storage: Vec<usize>,
    length: usize,
}

#[cfg(windows)]
impl SelfRelativeSecurityDescriptor {
    fn capture(descriptor: windows_sys::Win32::Security::PSECURITY_DESCRIPTOR) -> io::Result<Self> {
        use std::mem::size_of;
        use std::ptr;
        use windows_sys::Win32::Security::{
            GetSecurityDescriptorControl, GetSecurityDescriptorLength, IsValidSecurityDescriptor,
            MakeSelfRelativeSD, SE_SELF_RELATIVE,
        };

        if descriptor.is_null() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "security descriptor is absent",
            ));
        }
        // SAFETY: descriptor is non-null; callers retain the allocation for this call.
        if unsafe { IsValidSecurityDescriptor(descriptor) } == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "security descriptor is malformed",
            ));
        }
        let mut control = 0;
        let mut revision = 0;
        // SAFETY: descriptor was validated and both output pointers are writable.
        if unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) } == 0 {
            return Err(io::Error::last_os_error());
        }

        if control & SE_SELF_RELATIVE != 0 {
            // SAFETY: descriptor is a valid self-relative descriptor.
            let length = unsafe { GetSecurityDescriptorLength(descriptor) } as usize;
            if length == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "security descriptor has zero length",
                ));
            }
            let mut storage = vec![0usize; length.div_ceil(size_of::<usize>())];
            // SAFETY: storage has at least length writable bytes and descriptor has that many
            // readable bytes according to GetSecurityDescriptorLength.
            unsafe {
                ptr::copy_nonoverlapping(
                    descriptor.cast::<u8>(),
                    storage.as_mut_ptr().cast::<u8>(),
                    length,
                )
            };
            return Ok(Self { storage, length });
        }

        let mut length = 0;
        // SAFETY: this size query reads the validated absolute descriptor and writes length.
        unsafe { MakeSelfRelativeSD(descriptor, ptr::null_mut(), &mut length) };
        if length == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut storage = vec![0usize; (length as usize).div_ceil(size_of::<usize>())];
        // SAFETY: storage is aligned and has at least length writable bytes.
        if unsafe { MakeSelfRelativeSD(descriptor, storage.as_mut_ptr().cast(), &mut length) } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            storage,
            length: length as usize,
        })
    }

    fn as_ptr(&self) -> windows_sys::Win32::Security::PSECURITY_DESCRIPTOR {
        self.storage.as_ptr().cast_mut().cast()
    }

    fn as_bytes(&self) -> &[u8] {
        // SAFETY: storage owns at least length initialized bytes copied or written above.
        unsafe { std::slice::from_raw_parts(self.storage.as_ptr().cast::<u8>(), self.length) }
    }
}

#[cfg(windows)]
pub(crate) struct WindowsImmutableEntryEvidence {
    pub(crate) identity: FileIdentity,
    pub(crate) security_descriptor_sha256: [u8; 32],
    descriptor: SelfRelativeSecurityDescriptor,
}

#[cfg(windows)]
impl WindowsImmutableEntryEvidence {
    pub(crate) fn verify(&self, profile: WindowsImmutableAclProfile) -> io::Result<()> {
        verify_windows_immutable_security_descriptor(self.descriptor.as_ptr(), profile)
    }
}

#[cfg(windows)]
pub(crate) fn capture_windows_immutable_entry_evidence(
    file: &fs::File,
) -> io::Result<WindowsImmutableEntryEvidence> {
    use std::os::windows::io::AsRawHandle;
    use std::ptr;
    use windows_sys::Win32::Security::Authorization::{GetSecurityInfo, SE_FILE_OBJECT};
    use windows_sys::Win32::Security::{DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION};

    let mut descriptor = ptr::null_mut();
    // SAFETY: file owns a valid handle and descriptor is writable output storage.
    let status = unsafe {
        GetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            &mut descriptor,
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    let descriptor = LocalSecurityDescriptor(descriptor);
    let descriptor = SelfRelativeSecurityDescriptor::capture(descriptor.0)?;
    use sha2::{Digest, Sha256};
    Ok(WindowsImmutableEntryEvidence {
        identity: file_identity(file)?,
        security_descriptor_sha256: Sha256::digest(descriptor.as_bytes()).into(),
        descriptor,
    })
}

#[cfg(windows)]
fn sid_string(sid: windows_sys::Win32::Security::PSID) -> io::Result<String> {
    use std::ptr;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;

    let mut text = ptr::null_mut();
    // SAFETY: the caller supplies a validated SID and text is writable output storage.
    if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the API returned a live NUL-terminated UTF-16 allocation.
    let length = unsafe {
        let mut length = 0;
        while *text.add(length) != 0 {
            length += 1;
        }
        length
    };
    // SAFETY: length ends before the allocation's terminating NUL.
    let value = String::from_utf16(unsafe { std::slice::from_raw_parts(text, length) })
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error));
    // SAFETY: ConvertSidToStringSidW allocates with LocalAlloc.
    unsafe { LocalFree(text.cast()) };
    value
}

#[cfg(windows)]
fn windows_sid_is_trusted(sid: windows_sys::Win32::Security::PSID) -> io::Result<bool> {
    const TRUSTED_INSTALLER: &str =
        "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464";
    Ok(matches!(
        sid_string(sid)?.as_str(),
        TRUSTED_INSTALLER | "S-1-5-18" | "S-1-5-32-544"
    ))
}

#[cfg(windows)]
fn windows_immutable_mutation_mask(profile: WindowsImmutableAclProfile) -> u32 {
    use windows_sys::Win32::Foundation::{GENERIC_ALL, GENERIC_WRITE};
    use windows_sys::Win32::Storage::FileSystem::{
        DELETE, FILE_ADD_FILE, FILE_ADD_SUBDIRECTORY, FILE_DELETE_CHILD, FILE_WRITE_ATTRIBUTES,
        FILE_WRITE_DATA, FILE_WRITE_EA, WRITE_DAC, WRITE_OWNER,
    };

    let substitution =
        DELETE | FILE_DELETE_CHILD | WRITE_DAC | WRITE_OWNER | GENERIC_WRITE | GENERIC_ALL;
    match profile {
        WindowsImmutableAclProfile::Ancestry => substitution,
        WindowsImmutableAclProfile::Installation => {
            substitution
                | FILE_WRITE_DATA
                | FILE_ADD_FILE
                | FILE_ADD_SUBDIRECTORY
                | FILE_WRITE_EA
                | FILE_WRITE_ATTRIBUTES
        }
    }
}

#[cfg(windows)]
pub(crate) fn verify_windows_immutable_security_descriptor(
    descriptor: windows_sys::Win32::Security::PSECURITY_DESCRIPTOR,
    profile: WindowsImmutableAclProfile,
) -> io::Result<()> {
    use std::mem::{offset_of, size_of};
    use std::ptr;
    use windows_sys::Win32::Security::{
        AclSizeInformation, GetAce, GetAclInformation, GetLengthSid, GetSecurityDescriptorDacl,
        GetSecurityDescriptorOwner, IsValidAcl, IsValidSecurityDescriptor, IsValidSid,
        ACCESS_ALLOWED_ACE, ACE_HEADER, ACL, ACL_SIZE_INFORMATION,
    };

    const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;
    const ACCESS_DENIED_ACE_TYPE: u8 = 1;
    const VALID_INHERIT_FLAGS: u8 = 0x1f;
    const INHERIT_ONLY_ACE: u8 = 0x08;

    if descriptor.is_null() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "security descriptor is absent",
        ));
    }
    // SAFETY: descriptor is non-null and remains live for this function.
    if unsafe { IsValidSecurityDescriptor(descriptor) } == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "security descriptor is malformed",
        ));
    }

    let mut owner = ptr::null_mut();
    let mut owner_defaulted = 0;
    // SAFETY: descriptor was validated and output pointers are writable.
    if unsafe { GetSecurityDescriptorOwner(descriptor, &mut owner, &mut owner_defaulted) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if owner.is_null() || unsafe { IsValidSid(owner) } == 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "immutable platform owner SID is absent or malformed",
        ));
    }
    if !windows_sid_is_trusted(owner)? {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "immutable platform owner SID is not trusted",
        ));
    }

    let mut dacl_present = 0;
    let mut dacl: *mut ACL = ptr::null_mut();
    let mut dacl_defaulted = 0;
    // SAFETY: descriptor was validated and output pointers are writable.
    if unsafe {
        GetSecurityDescriptorDacl(
            descriptor,
            &mut dacl_present,
            &mut dacl,
            &mut dacl_defaulted,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    if dacl_present == 0 || dacl.is_null() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "immutable platform security descriptor has an absent or null DACL",
        ));
    }
    // SAFETY: dacl is non-null and points inside the validated descriptor.
    if unsafe { IsValidAcl(dacl) } == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "immutable platform DACL is malformed",
        ));
    }

    let mut information = ACL_SIZE_INFORMATION {
        AceCount: 0,
        AclBytesInUse: 0,
        AclBytesFree: 0,
    };
    // SAFETY: dacl is valid and information is writable.
    if unsafe {
        GetAclInformation(
            dacl,
            (&mut information as *mut ACL_SIZE_INFORMATION).cast(),
            size_of::<ACL_SIZE_INFORMATION>() as u32,
            AclSizeInformation,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }

    let mutation_mask = windows_immutable_mutation_mask(profile);
    for index in 0..information.AceCount {
        let mut ace = ptr::null_mut();
        // SAFETY: index is within the validated ACL's reported ACE count.
        if unsafe { GetAce(dacl, index, &mut ace) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: GetAce returned a pointer to a complete ACE header in the validated ACL.
        let header = unsafe { &*ace.cast::<ACE_HEADER>() };
        if header.AceFlags & !VALID_INHERIT_FLAGS != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "immutable platform DACL contains unsupported ACE flags",
            ));
        }
        if !matches!(
            header.AceType,
            ACCESS_ALLOWED_ACE_TYPE | ACCESS_DENIED_ACE_TYPE
        ) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "immutable platform DACL contains an unsupported ACE type",
            ));
        }

        let sid_offset = offset_of!(ACCESS_ALLOWED_ACE, SidStart);
        let minimum_sid_bytes = 8usize;
        let ace_size = usize::from(header.AceSize);
        if ace_size < sid_offset + minimum_sid_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "immutable platform DACL contains a truncated ACE SID",
            ));
        }
        // SAFETY: the fixed ACE header and minimum SID were bounds-checked against AceSize.
        let allowed = unsafe { &*ace.cast::<ACCESS_ALLOWED_ACE>() };
        let sid: windows_sys::Win32::Security::PSID =
            (&raw const allowed.SidStart).cast_mut().cast();
        // SAFETY: the minimum SID header is contained in this ACE.
        let subauthority_count = unsafe { *sid.cast::<u8>().add(1) } as usize;
        let sid_length = minimum_sid_bytes
            .checked_add(
                subauthority_count
                    .checked_mul(size_of::<u32>())
                    .ok_or_else(|| {
                        io::Error::new(io::ErrorKind::InvalidData, "ACE SID length overflowed")
                    })?,
            )
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "ACE SID length overflowed")
            })?;
        if sid_offset + sid_length > ace_size {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "immutable platform DACL contains an out-of-bounds ACE SID",
            ));
        }
        // SAFETY: the SID is fully contained in the ACE according to its own length field.
        if unsafe { IsValidSid(sid) } == 0 || unsafe { GetLengthSid(sid) } as usize != sid_length {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "immutable platform DACL contains a malformed ACE SID",
            ));
        }

        if header.AceType == ACCESS_DENIED_ACE_TYPE || header.AceFlags & INHERIT_ONLY_ACE != 0 {
            continue;
        }
        if allowed.Mask & mutation_mask != 0 && !windows_sid_is_trusted(sid)? {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "immutable platform DACL grants mutation rights 0x{:08x} to an untrusted SID",
                    allowed.Mask & mutation_mask
                ),
            ));
        }
    }
    Ok(())
}

#[cfg(windows)]
pub(crate) fn verify_windows_elevation_value(is_elevated: u32) -> io::Result<()> {
    if is_elevated == 0 {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "immutable platform execution is refused for an elevated Windows caller",
        ))
    }
}

#[cfg(windows)]
pub(crate) fn verify_unprivileged_windows_platform_caller() -> io::Result<()> {
    use std::mem::size_of;
    use windows_sys::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION};

    let token = ProcessToken::current_user()?;
    let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
    let mut returned = 0;
    // SAFETY: token is a live process-token handle, elevation is writable storage of the exact
    // requested type, and returned is writable.
    if unsafe {
        GetTokenInformation(
            token.handle,
            TokenElevation,
            (&mut elevation as *mut TOKEN_ELEVATION).cast(),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    if returned != size_of::<TOKEN_ELEVATION>() as u32 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Windows token elevation proof returned an unexpected size",
        ));
    }
    verify_windows_elevation_value(elevation.TokenIsElevated)
}

#[cfg(windows)]
#[allow(
    dead_code,
    reason = "DirectoryAnchor callers are introduced by the following Windows full-dump task"
)]
pub(crate) fn open_directory_nofollow(path: &Path) -> io::Result<fs::File> {
    use std::os::windows::io::FromRawHandle;
    use std::ptr;
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_LIST_DIRECTORY,
        FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
        READ_CONTROL,
    };

    let path = windows_api_path(path)?;
    // SAFETY: path is NUL-terminated and all scalar arguments are documented Win32 flags.
    let handle = unsafe {
        CreateFileW(
            path.as_ptr(),
            FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES | READ_CONTROL,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: CreateFileW returned an owned, valid file handle.
    let file = unsafe { fs::File::from_raw_handle(handle) };
    let attributes = windows_file_information(&file)?.dwFileAttributes;
    if attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "directory path resolves to a reparse point",
        ));
    }
    if attributes & FILE_ATTRIBUTE_DIRECTORY == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "directory path does not resolve to a directory",
        ));
    }
    Ok(file)
}

#[cfg(windows)]
pub(crate) fn open_absolute_directory_path_nofollow(path: &Path) -> io::Result<fs::File> {
    use std::path::Component;

    if !path.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "secure directory path must be absolute",
        ));
    }
    let absolute = std::path::absolute(path)?;
    let mut components = absolute.components();
    let Some(Component::Prefix(prefix)) = components.next() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "secure Windows directory path has no prefix",
        ));
    };
    if !matches!(components.next(), Some(Component::RootDir)) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "secure Windows directory path has no root component",
        ));
    }
    let mut anchor = PathBuf::from(prefix.as_os_str());
    anchor.push(r"\");
    let mut current = open_directory_nofollow(&anchor)?;
    for component in components {
        match component {
            Component::Normal(name) => {
                current = open_directory_child_nofollow(&current, name)?;
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "secure directory path contains a non-normal component",
                ));
            }
        }
    }
    Ok(current)
}

#[cfg(windows)]
pub(crate) fn open_or_create_absolute_directory_path_nofollow(path: &Path) -> io::Result<fs::File> {
    use std::path::Component;

    if !path.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "secure directory path must be absolute",
        ));
    }
    let absolute = std::path::absolute(path)?;
    let mut components = absolute.components();
    let Some(Component::Prefix(prefix)) = components.next() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "secure Windows directory path has no prefix",
        ));
    };
    if !matches!(components.next(), Some(Component::RootDir)) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "secure Windows directory path has no root component",
        ));
    }
    let mut anchor = PathBuf::from(prefix.as_os_str());
    anchor.push(r"\");
    let mut current = open_directory_nofollow(&anchor)?;
    for component in components {
        match component {
            Component::Normal(name) => {
                current = match open_directory_child_nofollow(&current, name) {
                    Ok(directory) => directory,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {
                        match create_new_directory_child(&current, name) {
                            Ok(directory) => directory,
                            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                                open_directory_child_nofollow(&current, name)?
                            }
                            Err(error) => return Err(error),
                        }
                    }
                    Err(error) => return Err(error),
                };
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "secure directory path contains a non-normal component",
                ));
            }
        }
    }
    Ok(current)
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn open_absolute_directory_path_nofollow(_path: &Path) -> io::Result<fs::File> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "secure no-follow directory paths are unavailable on this host",
    ))
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn open_or_create_absolute_directory_path_nofollow(
    _path: &Path,
) -> io::Result<fs::File> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "secure no-follow directory creation is unavailable on this host",
    ))
}

#[cfg(windows)]
#[repr(C)]
struct NtUnicodeString {
    length: u16,
    maximum_length: u16,
    buffer: windows_sys::core::PWSTR,
}

#[cfg(windows)]
#[repr(C)]
struct NtObjectAttributes {
    length: u32,
    root_directory: windows_sys::Win32::Foundation::HANDLE,
    object_name: *mut NtUnicodeString,
    attributes: u32,
    security_descriptor: *mut std::ffi::c_void,
    security_quality_of_service: *mut std::ffi::c_void,
}

#[cfg(windows)]
#[repr(C)]
union NtIoStatusStatus {
    status: i32,
    pointer: *mut std::ffi::c_void,
}

#[cfg(windows)]
#[repr(C)]
struct NtIoStatusBlock {
    status: NtIoStatusStatus,
    information: usize,
}

#[cfg(windows)]
#[repr(C)]
struct NtFileFsDeviceInformation {
    device_type: u32,
    characteristics: u32,
}

#[cfg(windows)]
unsafe extern "system" {
    fn NtCreateFile(
        file_handle: *mut windows_sys::Win32::Foundation::HANDLE,
        desired_access: u32,
        object_attributes: *mut NtObjectAttributes,
        io_status_block: *mut NtIoStatusBlock,
        allocation_size: *mut i64,
        file_attributes: u32,
        share_access: u32,
        create_disposition: u32,
        create_options: u32,
        ea_buffer: *mut std::ffi::c_void,
        ea_length: u32,
    ) -> i32;
    fn NtSetInformationFile(
        file_handle: windows_sys::Win32::Foundation::HANDLE,
        io_status_block: *mut NtIoStatusBlock,
        file_information: *mut std::ffi::c_void,
        length: u32,
        file_information_class: u32,
    ) -> i32;
    fn NtQueryVolumeInformationFile(
        file_handle: windows_sys::Win32::Foundation::HANDLE,
        io_status_block: *mut NtIoStatusBlock,
        fs_information: *mut std::ffi::c_void,
        length: u32,
        fs_information_class: u32,
    ) -> i32;
    fn RtlNtStatusToDosError(status: i32) -> u32;
}

#[cfg(windows)]
fn nt_status_error(status: i32) -> io::Error {
    // SAFETY: RtlNtStatusToDosError accepts all NTSTATUS values and returns a Win32 error code.
    let error = unsafe { RtlNtStatusToDosError(status) };
    io::Error::from_raw_os_error(error as i32)
}

#[cfg(windows)]
fn verify_windows_local_fixed_device_info(
    device_type: u32,
    characteristics: u32,
) -> io::Result<()> {
    const FILE_DEVICE_DISK: u32 = 0x0000_0007;
    const FILE_REMOTE_DEVICE: u32 = 0x0000_0010;
    const FILE_REMOVABLE_MEDIA: u32 = 0x0000_0001;
    const FILE_FLOPPY_DISKETTE: u32 = 0x0000_0004;
    const FILE_WRITE_ONCE_MEDIA: u32 = 0x0000_0008;
    const FILE_VIRTUAL_VOLUME: u32 = 0x0000_0040;
    const FILE_PORTABLE_DEVICE: u32 = 0x0000_4000;
    const FILE_REMOTE_DEVICE_VSMB: u32 = 0x0008_0000;
    const UNTRUSTED_CHARACTERISTICS: u32 = FILE_REMOTE_DEVICE
        | FILE_REMOVABLE_MEDIA
        | FILE_FLOPPY_DISKETTE
        | FILE_WRITE_ONCE_MEDIA
        | FILE_VIRTUAL_VOLUME
        | FILE_PORTABLE_DEVICE
        | FILE_REMOTE_DEVICE_VSMB;

    if device_type != FILE_DEVICE_DISK || characteristics & UNTRUSTED_CHARACTERISTICS != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "immutable platform volume is not a local fixed disk (device type 0x{device_type:08x}, characteristics 0x{characteristics:08x})"
            ),
        ));
    }
    Ok(())
}

#[cfg(windows)]
pub(crate) fn verify_windows_local_fixed_volume(file: &fs::File) -> io::Result<()> {
    use std::mem::size_of;
    use std::os::windows::io::AsRawHandle;

    const FILE_FS_DEVICE_INFORMATION_CLASS: u32 = 4;
    let mut io_status = NtIoStatusBlock {
        status: NtIoStatusStatus { status: 0 },
        information: 0,
    };
    let mut information = NtFileFsDeviceInformation {
        device_type: 0,
        characteristics: 0,
    };
    // SAFETY: file retains a valid handle for the complete call; io_status and information are
    // writable buffers of the exact native layouts for FileFsDeviceInformation.
    let status = unsafe {
        NtQueryVolumeInformationFile(
            file.as_raw_handle(),
            &mut io_status,
            (&mut information as *mut NtFileFsDeviceInformation).cast(),
            size_of::<NtFileFsDeviceInformation>() as u32,
            FILE_FS_DEVICE_INFORMATION_CLASS,
        )
    };
    if status < 0 {
        return Err(nt_status_error(status));
    }
    if io_status.information != size_of::<NtFileFsDeviceInformation>() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Windows volume device proof returned an unexpected size",
        ));
    }
    verify_windows_local_fixed_device_info(information.device_type, information.characteristics)
}

#[cfg(windows)]
fn relative_child_name(name: &std::ffi::OsStr) -> io::Result<Vec<u16>> {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Component;

    let mut components = Path::new(name).components();
    if !matches!(components.next(), Some(Component::Normal(component)) if component == name)
        || components.next().is_some()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "directory child name must be exactly one normal relative component",
        ));
    }
    let wide = name.encode_wide().collect::<Vec<_>>();
    if wide.is_empty()
        || wide.iter().any(|unit| {
            *unit == 0 || *unit == b'\\' as u16 || *unit == b'/' as u16 || *unit == b':' as u16
        })
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "directory child name must not contain separators, a stream name, or NUL",
        ));
    }
    Ok(wide)
}

#[cfg(windows)]
fn validate_directory_handle(file: &fs::File) -> io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
    };

    let attributes = windows_file_information(file)?.dwFileAttributes;
    if attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "directory child resolves to a reparse point",
        ));
    }
    if attributes & FILE_ATTRIBUTE_DIRECTORY == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "directory child does not resolve to a directory",
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn nt_create_options_for_std_file(desired_access: u32, create_options: u32) -> io::Result<u32> {
    use windows_sys::Win32::Foundation::{
        GENERIC_ALL, GENERIC_EXECUTE, GENERIC_READ, GENERIC_WRITE,
    };
    use windows_sys::Win32::Storage::FileSystem::SYNCHRONIZE;

    const FILE_SYNCHRONOUS_IO_NONALERT: u32 = 0x0000_0020;
    let generic_synchronous_access = GENERIC_ALL | GENERIC_EXECUTE | GENERIC_READ | GENERIC_WRITE;
    if desired_access & SYNCHRONIZE == 0 && desired_access & generic_synchronous_access == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "std::fs::File native handles require SYNCHRONIZE-compatible access",
        ));
    }
    Ok(create_options | FILE_SYNCHRONOUS_IO_NONALERT)
}

#[cfg(windows)]
fn query_parent_case_sensitive_flags(parent: &fs::File) -> io::Result<u32> {
    use std::mem::size_of;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        FileCaseSensitiveInfo, GetFileInformationByHandleEx,
    };

    #[repr(C)]
    struct FileCaseSensitiveInformation {
        flags: u32,
    }

    #[cfg(test)]
    if let Some(error) = TEST_CASE_SENSITIVITY_QUERY_ERROR.with(|slot| slot.get()) {
        return Err(io::Error::from_raw_os_error(error as i32));
    }

    let mut information = FileCaseSensitiveInformation { flags: 0 };
    // SAFETY: parent retains a valid directory handle and information is a writable buffer with
    // the exact FileCaseSensitiveInfo layout and size.
    if unsafe {
        GetFileInformationByHandleEx(
            parent.as_raw_handle(),
            FileCaseSensitiveInfo,
            (&mut information as *mut FileCaseSensitiveInformation).cast(),
            size_of::<FileCaseSensitiveInformation>() as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(information.flags)
}

/// Reports whether Windows declined to serve the per-directory case-sensitivity query rather
/// than answering it. Two distinct layers decline the same way: a file system without the
/// feature, and the Win32 entry point on a build that does not carry the information class.
/// Windows Server 2019 (build 17763) is the measured case of the second — `fsutil`, which asks
/// through `NtQueryInformationFile`, answers for the same directory that
/// `GetFileInformationByHandleEx` rejects with `ERROR_INVALID_PARAMETER`.
#[cfg(windows)]
fn case_sensitivity_query_is_unsupported(error: &io::Error) -> bool {
    use windows_sys::Win32::Foundation::{
        ERROR_CALL_NOT_IMPLEMENTED, ERROR_INVALID_FUNCTION, ERROR_INVALID_PARAMETER,
        ERROR_NOT_SUPPORTED,
    };

    matches!(
        error.raw_os_error(),
        Some(code)
            if code == ERROR_INVALID_FUNCTION as i32
                || code == ERROR_INVALID_PARAMETER as i32
                || code == ERROR_NOT_SUPPORTED as i32
                || code == ERROR_CALL_NOT_IMPLEMENTED as i32
    )
}

#[cfg(windows)]
fn relative_child_object_attributes(parent: &fs::File) -> io::Result<u32> {
    const OBJ_CASE_INSENSITIVE: u32 = 0x40;
    const FILE_CS_FLAG_CASE_SENSITIVE_DIR: u32 = 1;

    let flags = match query_parent_case_sensitive_flags(parent) {
        Ok(flags) => flags,
        // A declined query is an answer about the platform, not an unproven parent. Where the
        // file system lacks the feature no directory can be case-sensitive and the insensitive
        // match is exact; where only the Win32 entry point lacks the class, this open matches
        // names the way every ordinary Win32 open on that host already does, including std::fs.
        // Refusing instead would strand the whole surface on a host that is merely older.
        // Every other failure still leaves the parent unproven and fails closed.
        Err(error) if case_sensitivity_query_is_unsupported(&error) => {
            return Ok(OBJ_CASE_INSENSITIVE)
        }
        Err(error) => {
            return Err(io::Error::new(
                error.kind(),
                format!("failed to query parent directory case-sensitive state: {error}"),
            ))
        }
    };
    if flags & !FILE_CS_FLAG_CASE_SENSITIVE_DIR != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("parent directory reported unsupported case-sensitive flags 0x{flags:08x}"),
        ));
    }
    Ok(if flags & FILE_CS_FLAG_CASE_SENSITIVE_DIR != 0 {
        0
    } else {
        OBJ_CASE_INSENSITIVE
    })
}

#[cfg(windows)]
fn open_relative_child(
    parent: &fs::File,
    name: &std::ffi::OsStr,
    desired_access: u32,
    file_attributes: u32,
    create_disposition: u32,
    create_options: u32,
    security_descriptor: Option<windows_sys::Win32::Security::PSECURITY_DESCRIPTOR>,
) -> io::Result<fs::File> {
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };

    open_relative_child_with_share_access(
        parent,
        name,
        desired_access,
        file_attributes,
        create_disposition,
        create_options,
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
        security_descriptor,
    )
}

#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn open_relative_child_with_share_access(
    parent: &fs::File,
    name: &std::ffi::OsStr,
    desired_access: u32,
    file_attributes: u32,
    create_disposition: u32,
    create_options: u32,
    share_access: u32,
    security_descriptor: Option<windows_sys::Win32::Security::PSECURITY_DESCRIPTOR>,
) -> io::Result<fs::File> {
    use std::mem::size_of;
    use std::os::windows::io::{AsRawHandle, FromRawHandle};
    use std::ptr;

    let create_options = nt_create_options_for_std_file(desired_access, create_options)?;
    let object_attributes = relative_child_object_attributes(parent)?;
    let mut name = relative_child_name(name)?;
    let byte_length = name
        .len()
        .checked_mul(size_of::<u16>())
        .and_then(|length| u16::try_from(length).ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "child name is too long"))?;
    let mut object_name = NtUnicodeString {
        length: byte_length,
        maximum_length: byte_length,
        buffer: name.as_mut_ptr(),
    };
    let mut attributes = NtObjectAttributes {
        length: size_of::<NtObjectAttributes>() as u32,
        root_directory: parent.as_raw_handle(),
        object_name: &mut object_name,
        attributes: object_attributes,
        security_descriptor: security_descriptor.unwrap_or(ptr::null_mut()).cast(),
        security_quality_of_service: ptr::null_mut(),
    };
    let mut status = NtIoStatusBlock {
        status: NtIoStatusStatus { status: 0 },
        information: 0,
    };
    let mut handle = ptr::null_mut();
    // SAFETY: the root handle is borrowed from parent, and all pointers refer to valid mutable
    // storage for the duration of NtCreateFile. The child name is validated as one relative
    // component, so RootDirectory binds the operation to the retained parent handle.
    let result = unsafe {
        NtCreateFile(
            &mut handle,
            desired_access,
            &mut attributes,
            &mut status,
            ptr::null_mut(),
            file_attributes,
            share_access,
            create_disposition,
            create_options,
            ptr::null_mut(),
            0,
        )
    };
    if result < 0 {
        return Err(nt_status_error(result));
    }
    // SAFETY: a non-error NtCreateFile result returns an owned file handle.
    Ok(unsafe { fs::File::from_raw_handle(handle) })
}

#[cfg(windows)]
pub(crate) fn open_directory_child_nofollow(
    parent: &fs::File,
    name: &std::ffi::OsStr,
) -> io::Result<fs::File> {
    const FILE_OPEN: u32 = 0x0000_0001;
    const FILE_DIRECTORY_FILE: u32 = 0x0000_0001;
    const FILE_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, READ_CONTROL, SYNCHRONIZE,
    };

    let file = open_relative_child(
        parent,
        name,
        FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES | READ_CONTROL | SYNCHRONIZE,
        0,
        FILE_OPEN,
        FILE_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT,
        None,
    )?;
    validate_directory_handle(&file)?;
    Ok(file)
}

#[cfg(windows)]
pub(crate) fn open_regular_child_nofollow(
    parent: &fs::File,
    name: &std::ffi::OsStr,
) -> io::Result<fs::File> {
    const FILE_OPEN: u32 = 0x0000_0001;
    const FILE_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    use windows_sys::Win32::Foundation::GENERIC_READ;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_READ_ATTRIBUTES, SYNCHRONIZE,
    };

    let file = open_relative_child(
        parent,
        name,
        GENERIC_READ | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
        0,
        FILE_OPEN,
        FILE_OPEN_REPARSE_POINT,
        None,
    )?;
    let attributes = windows_file_information(&file)?.dwFileAttributes;
    if attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "regular child resolves to a reparse point",
        ));
    }
    if attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "entry is not a regular file",
        ));
    }
    Ok(file)
}

#[cfg(windows)]
fn open_or_create_regular_child_read_write_nofollow(
    parent: &fs::File,
    name: &std::ffi::OsStr,
) -> io::Result<fs::File> {
    const FILE_OPEN_IF: u32 = 0x0000_0003;
    const FILE_NON_DIRECTORY_FILE: u32 = 0x0000_0040;
    const FILE_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE};
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_REPARSE_POINT, SYNCHRONIZE,
    };

    let file = open_relative_child(
        parent,
        name,
        GENERIC_READ | GENERIC_WRITE | SYNCHRONIZE,
        FILE_ATTRIBUTE_NORMAL,
        FILE_OPEN_IF,
        FILE_NON_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT,
        None,
    )?;
    let attributes = windows_file_information(&file)?.dwFileAttributes;
    if attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "descriptor-relative lifecycle lock resolves to a reparse point",
        ));
    }
    if attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "descriptor-relative lifecycle lock is not a regular file",
        ));
    }
    Ok(file)
}

#[cfg(windows)]
pub(crate) fn open_any_child_nofollow(
    parent: &fs::File,
    name: &std::ffi::OsStr,
) -> io::Result<(fs::File, OpenedChildKind)> {
    const FILE_OPEN: u32 = 0x0000_0001;
    const FILE_OPEN_FOR_BACKUP_INTENT: u32 = 0x0000_4000;
    const FILE_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    use windows_sys::Win32::Storage::FileSystem::{FILE_READ_ATTRIBUTES, SYNCHRONIZE};

    // FILE_OPEN_FOR_BACKUP_INTENT is required for an untyped handle to a directory entry.
    // Without it, NtCreateFile reports a directory symlink as not found instead of returning
    // the reparse-point handle that opened_child_kind must reject.
    let file = open_relative_child(
        parent,
        name,
        FILE_READ_ATTRIBUTES | SYNCHRONIZE,
        0,
        FILE_OPEN,
        FILE_OPEN_FOR_BACKUP_INTENT | FILE_OPEN_REPARSE_POINT,
        None,
    )?;
    let kind = opened_child_kind(&file)?;
    Ok((file, kind))
}

#[cfg(windows)]
pub(crate) fn open_child_for_secure_tree_use(
    parent: &fs::File,
    name: &std::ffi::OsStr,
    classification_anchor: fs::File,
    kind: OpenedChildKind,
) -> io::Result<fs::File> {
    let anchor_identity = file_identity(&classification_anchor)?;
    let typed = match kind {
        OpenedChildKind::Directory => open_directory_child_nofollow(parent, name)?,
        OpenedChildKind::RegularFile => open_regular_child_nofollow(parent, name)?,
        OpenedChildKind::ReparsePoint | OpenedChildKind::Unsupported => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "secure tree cannot use an untyped child handle",
            ))
        }
    };
    if file_identity(&typed)? != anchor_identity || opened_child_kind(&typed)? != kind {
        return Err(io::Error::other(
            "typed child identity differs from its classification anchor",
        ));
    }
    Ok(typed)
}

#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OpenedChildKind {
    Directory,
    RegularFile,
    ReparsePoint,
    Unsupported,
}

#[cfg(windows)]
pub(crate) fn opened_child_kind(file: &fs::File) -> io::Result<OpenedChildKind> {
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_DEVICE, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
    };

    let attributes = windows_file_information(file)?.dwFileAttributes;
    if attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        Ok(OpenedChildKind::ReparsePoint)
    } else if attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
        Ok(OpenedChildKind::Directory)
    } else if attributes & FILE_ATTRIBUTE_DEVICE != 0 {
        Ok(OpenedChildKind::Unsupported)
    } else {
        Ok(OpenedChildKind::RegularFile)
    }
}

#[cfg(windows)]
pub(crate) fn open_any_child_for_delete(
    parent: &fs::File,
    name: &std::ffi::OsStr,
) -> io::Result<fs::File> {
    const FILE_OPEN: u32 = 0x0000_0001;
    const FILE_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    use windows_sys::Win32::Storage::FileSystem::{DELETE, FILE_READ_ATTRIBUTES, SYNCHRONIZE};

    open_relative_child(
        parent,
        name,
        DELETE | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
        0,
        FILE_OPEN,
        FILE_OPEN_REPARSE_POINT,
        None,
    )
}

#[cfg(windows)]
fn open_child_for_delete_and_attribute_write(
    parent: &fs::File,
    name: &std::ffi::OsStr,
) -> io::Result<fs::File> {
    const FILE_OPEN: u32 = 0x0000_0001;
    const FILE_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    use windows_sys::Win32::Storage::FileSystem::{
        DELETE, FILE_READ_ATTRIBUTES, FILE_WRITE_ATTRIBUTES, SYNCHRONIZE,
    };

    open_relative_child(
        parent,
        name,
        DELETE | FILE_READ_ATTRIBUTES | FILE_WRITE_ATTRIBUTES | SYNCHRONIZE,
        0,
        FILE_OPEN,
        FILE_OPEN_REPARSE_POINT,
        None,
    )
}

#[cfg(windows)]
pub(crate) fn delete_open_child(file: &fs::File) -> io::Result<()> {
    set_delete_disposition(file)
}

#[cfg(windows)]
fn directory_query_is_end(restart: bool, error: &io::Error) -> bool {
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_NO_MORE_FILES};

    matches!(
        error.raw_os_error(),
        Some(code)
            if code == ERROR_NO_MORE_FILES as i32
                || restart && code == ERROR_FILE_NOT_FOUND as i32
    )
}

#[cfg(windows)]
fn parse_directory_information_buffer(
    buffer: &[u8],
    names: &mut Vec<std::ffi::OsString>,
) -> io::Result<()> {
    parse_directory_information_buffer_bounded(buffer, names, usize::MAX, &mut || Ok(()))
}

#[cfg(windows)]
fn parse_directory_information_buffer_bounded(
    buffer: &[u8],
    names: &mut Vec<std::ffi::OsString>,
    maximum_entries: usize,
    checkpoint: &mut impl FnMut() -> io::Result<()>,
) -> io::Result<()> {
    use std::mem::{offset_of, size_of};
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_ID_BOTH_DIR_INFO;

    let file_name_offset = offset_of!(FILE_ID_BOTH_DIR_INFO, FileName);
    let complete_header_size = size_of::<FILE_ID_BOTH_DIR_INFO>();
    let mut offset = 0usize;
    loop {
        let complete_header_end = offset.checked_add(complete_header_size).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "directory entry complete header offset overflowed",
            )
        })?;
        if complete_header_end > buffer.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "directory entry complete header exceeds the enumeration buffer",
            ));
        }
        // SAFETY: the complete Rust structure was bounds-checked above; read_unaligned accepts
        // every record offset supplied by the filesystem.
        let entry = unsafe {
            std::ptr::read_unaligned(buffer.as_ptr().add(offset).cast::<FILE_ID_BOTH_DIR_INFO>())
        };
        let name_bytes = entry.FileNameLength as usize;
        if name_bytes == 0 || !name_bytes.is_multiple_of(size_of::<u16>()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "directory entry has an invalid UTF-16 name length",
            ));
        }
        let name_start = offset.checked_add(file_name_offset).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "directory entry name offset overflowed",
            )
        })?;
        let name_end = name_start.checked_add(name_bytes).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "directory entry name offset overflowed",
            )
        })?;
        if name_end > buffer.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "directory entry name exceeds the enumeration buffer",
            ));
        }
        let mut name = Vec::with_capacity(name_bytes / size_of::<u16>());
        for unit_offset in (name_start..name_end).step_by(size_of::<u16>()) {
            // SAFETY: every two-byte unit lies within the checked name range; read_unaligned
            // accepts the byte buffer's alignment.
            name.push(unsafe {
                std::ptr::read_unaligned(buffer.as_ptr().add(unit_offset).cast::<u16>())
            });
        }
        let name = std::ffi::OsString::from_wide(&name);
        checkpoint()?;
        if name != "." && name != ".." {
            if names.len() >= maximum_entries {
                return Err(io::Error::new(
                    io::ErrorKind::FileTooLarge,
                    "directory exceeds the retained enumeration entry limit",
                ));
            }
            names.push(name);
        }

        let next = entry.NextEntryOffset as usize;
        if next == 0 {
            break;
        }
        let minimum_record_bytes = file_name_offset.checked_add(name_bytes).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "directory entry record length overflowed",
            )
        })?;
        if next < minimum_record_bytes || !next.is_multiple_of(8) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "directory entry next-record offset is not 8-byte-aligned or overlaps the name",
            ));
        }
        offset = offset.checked_add(next).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "directory entry offset overflowed",
            )
        })?;
        if offset >= buffer.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "directory entry offset exceeds the enumeration buffer",
            ));
        }
    }
    Ok(())
}

#[cfg(windows)]
#[allow(
    dead_code,
    reason = "the following Windows full-dump tree-inspection task consumes retained enumeration"
)]
pub(crate) fn read_directory_names(directory: &fs::File) -> io::Result<Vec<std::ffi::OsString>> {
    read_directory_names_bounded(directory, usize::MAX, || Ok(()))
}

#[cfg(windows)]
pub(crate) fn read_directory_names_bounded(
    directory: &fs::File,
    maximum_entries: usize,
    mut checkpoint: impl FnMut() -> io::Result<()>,
) -> io::Result<Vec<std::ffi::OsString>> {
    use std::mem::size_of;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        FileIdBothDirectoryInfo, FileIdBothDirectoryRestartInfo, GetFileInformationByHandleEx,
    };

    const BUFFER_BYTES: usize = 64 * 1024;

    validate_directory_handle(directory)?;
    let word_count = BUFFER_BYTES.div_ceil(size_of::<usize>());
    let mut storage = vec![0usize; word_count];
    let buffer_bytes = storage.len() * size_of::<usize>();
    let mut names = Vec::new();
    let mut restart = true;
    loop {
        storage.fill(0);
        let information_class = if restart {
            FileIdBothDirectoryRestartInfo
        } else {
            FileIdBothDirectoryInfo
        };
        // SAFETY: storage is writable and pointer-aligned, its byte capacity is supplied exactly,
        // and directory remains a live retained handle throughout enumeration.
        let succeeded = unsafe {
            GetFileInformationByHandleEx(
                directory.as_raw_handle(),
                information_class,
                storage.as_mut_ptr().cast(),
                u32::try_from(buffer_bytes).expect("fixed directory buffer fits u32"),
            )
        };
        if succeeded == 0 {
            let error = io::Error::last_os_error();
            if directory_query_is_end(restart, &error) {
                break;
            }
            return Err(error);
        }
        restart = false;

        // SAFETY: storage owns buffer_bytes initialized bytes for this query. The parser performs
        // complete structure, name, and next-record bounds checks before reading each field.
        let buffer =
            unsafe { std::slice::from_raw_parts(storage.as_ptr().cast::<u8>(), buffer_bytes) };
        parse_directory_information_buffer_bounded(
            buffer,
            &mut names,
            maximum_entries,
            &mut checkpoint,
        )?;
    }
    names.sort();
    Ok(names)
}

#[cfg(windows)]
pub(crate) fn open_directory_child_for_rename(
    parent: &fs::File,
    name: &std::ffi::OsStr,
) -> io::Result<fs::File> {
    const FILE_OPEN: u32 = 0x0000_0001;
    const FILE_DIRECTORY_FILE: u32 = 0x0000_0001;
    const FILE_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    use windows_sys::Win32::Storage::FileSystem::{
        DELETE, FILE_READ_ATTRIBUTES, READ_CONTROL, SYNCHRONIZE,
    };

    let file = open_relative_child(
        parent,
        name,
        DELETE | FILE_READ_ATTRIBUTES | READ_CONTROL | SYNCHRONIZE,
        0,
        FILE_OPEN,
        FILE_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT,
        None,
    )?;
    validate_directory_handle(&file)?;
    Ok(file)
}

#[cfg(windows)]
pub(crate) fn rename_directory_handle_child_no_replace(
    source: &fs::File,
    destination_parent: &fs::File,
    destination_name: &std::ffi::OsStr,
) -> io::Result<()> {
    rename_open_child_no_replace(source, destination_parent, destination_name)
}

#[cfg(windows)]
fn rename_open_child_no_replace(
    source: &fs::File,
    destination_parent: &fs::File,
    destination_name: &std::ffi::OsStr,
) -> io::Result<()> {
    rename_open_child(source, destination_parent, destination_name, false)
}

#[cfg(windows)]
fn rename_open_child(
    source: &fs::File,
    destination_parent: &fs::File,
    destination_name: &std::ffi::OsStr,
    replace_if_exists: bool,
) -> io::Result<()> {
    use std::mem::{offset_of, size_of};
    use std::os::windows::io::AsRawHandle;
    use std::ptr;

    #[repr(C)]
    union RenameFlags {
        replace_if_exists: u8,
        flags: u32,
    }

    #[repr(C)]
    struct RenameInformation {
        anonymous: RenameFlags,
        root_directory: windows_sys::Win32::Foundation::HANDLE,
        file_name_length: u32,
        file_name: [u16; 1],
    }

    const FILE_RENAME_INFORMATION_CLASS: u32 = 10;

    let name = relative_child_name(destination_name)?;
    let name_bytes = name
        .len()
        .checked_mul(size_of::<u16>())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "child name is too long"))?;
    let information_length = offset_of!(RenameInformation, file_name)
        .checked_add(name_bytes)
        .and_then(|length| u32::try_from(length).ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "child name is too long"))?;
    let word_count = (information_length as usize).div_ceil(size_of::<usize>());
    let mut storage = vec![0usize; word_count];
    let information = storage.as_mut_ptr().cast::<RenameInformation>();
    // SAFETY: storage is pointer-aligned and large enough for the fixed header plus the complete
    // UTF-16 name. All fields are initialized before NtSetInformationFile reads the buffer.
    unsafe {
        ptr::addr_of_mut!((*information).anonymous).write(RenameFlags {
            replace_if_exists: u8::from(replace_if_exists),
        });
        ptr::addr_of_mut!((*information).root_directory).write(destination_parent.as_raw_handle());
        ptr::addr_of_mut!((*information).file_name_length).write(name_bytes as u32);
        ptr::copy_nonoverlapping(
            name.as_ptr(),
            ptr::addr_of_mut!((*information).file_name).cast::<u16>(),
            name.len(),
        );
    }
    let mut status = NtIoStatusBlock {
        status: NtIoStatusStatus { status: 0 },
        information: 0,
    };
    // SAFETY: source is an owned child handle opened with DELETE access, destination_parent
    // remains live, and the initialized buffer describes one relative destination name.
    let result = unsafe {
        NtSetInformationFile(
            source.as_raw_handle(),
            &mut status,
            information.cast(),
            information_length,
            FILE_RENAME_INFORMATION_CLASS,
        )
    };
    if result < 0 {
        Err(nt_status_error(result))
    } else {
        Ok(())
    }
}

#[cfg(unix)]
pub(crate) fn create_owner_only_directory_child(
    parent: &fs::File,
    name: &std::ffi::OsStr,
) -> io::Result<fs::File> {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::PermissionsExt;

    let encoded_name = unix_child_name(name)?;
    // SAFETY: the retained parent descriptor and validated child name remain live. 0700 is
    // owner-only even before the process umask is applied, so there is no permissive creation
    // window before the retained handle is normalized below.
    let status = unsafe { libc::mkdirat(parent.as_raw_fd(), encoded_name.as_ptr(), 0o700) };
    if status != 0 {
        return Err(io::Error::last_os_error());
    }
    let directory = open_directory_child_nofollow(parent, name)?;
    directory.set_permissions(fs::Permissions::from_mode(0o700))?;
    verify_owner_only_acl(&directory)?;
    Ok(directory)
}

#[cfg(unix)]
pub(crate) fn create_owner_only_file_child(
    parent: &fs::File,
    name: &std::ffi::OsStr,
) -> io::Result<fs::File> {
    use std::os::fd::{AsRawFd, FromRawFd};

    let name = unix_child_name(name)?;
    // SAFETY: parent and name remain live, and a successful result transfers one newly owned
    // descriptor. Mode 0600 is private before umask filtering and O_NOFOLLOW rejects links.
    let descriptor = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            0o600,
        )
    };
    if descriptor < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: descriptor is a newly owned successful openat result.
    let file = unsafe { fs::File::from_raw_fd(descriptor) };
    restrict_stage_to_owner(&file)?;
    verify_owner_only_acl(&file)?;
    Ok(file)
}

#[cfg(windows)]
pub(crate) fn create_owner_only_directory_child(
    parent: &fs::File,
    name: &std::ffi::OsStr,
) -> io::Result<fs::File> {
    const FILE_CREATE: u32 = 0x0000_0002;
    const FILE_DIRECTORY_FILE: u32 = 0x0000_0001;
    const FILE_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    use windows_sys::Win32::Storage::FileSystem::{
        DELETE, FILE_ATTRIBUTE_DIRECTORY, FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, READ_CONTROL,
        SYNCHRONIZE,
    };

    let security = OwnerOnlySecurityAttributes::current_user()?;
    let created = open_relative_child(
        parent,
        name,
        DELETE | FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES | READ_CONTROL | SYNCHRONIZE,
        FILE_ATTRIBUTE_DIRECTORY,
        FILE_CREATE,
        FILE_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT,
        Some(security.security_descriptor()),
    )?;
    if let Err(error) = validate_directory_handle(&created) {
        return match discard_created_child(&created) {
            Ok(()) => Err(error),
            Err(cleanup) => Err(io::Error::new(
                error.kind(),
                format!("{error}; failed to remove invalid created directory: {cleanup}"),
            )),
        };
    }
    let expected_identity = match file_identity(&created) {
        Ok(identity) => identity,
        Err(error) => {
            return match discard_created_child(&created) {
                Ok(()) => Err(error),
                Err(cleanup) => Err(io::Error::new(
                    error.kind(),
                    format!("{error}; failed to remove unverified created directory: {cleanup}"),
                )),
            };
        }
    };
    let reopened = match open_directory_child_nofollow(parent, name) {
        Ok(reopened) => reopened,
        Err(error) => {
            return match discard_created_child(&created) {
                Ok(()) => Err(error),
                Err(cleanup) => Err(io::Error::new(
                    error.kind(),
                    format!("{error}; failed to remove unreopenable created directory: {cleanup}"),
                )),
            };
        }
    };
    let actual_identity = match file_identity(&reopened) {
        Ok(identity) => identity,
        Err(error) => {
            return match discard_created_child(&created) {
                Ok(()) => Err(error),
                Err(cleanup) => Err(io::Error::new(
                    error.kind(),
                    format!("{error}; failed to remove unverified created directory: {cleanup}"),
                )),
            };
        }
    };
    if actual_identity != expected_identity {
        let error = io::Error::new(
            io::ErrorKind::InvalidData,
            "created directory identity changed before least-privilege reopen",
        );
        return match discard_created_child(&created) {
            Ok(()) => Err(error),
            Err(cleanup) => Err(io::Error::new(
                error.kind(),
                format!("{error}; failed to remove replaced created directory: {cleanup}"),
            )),
        };
    }
    drop(created);
    Ok(reopened)
}

#[cfg(windows)]
pub(crate) fn create_owner_only_file_child(
    parent: &fs::File,
    name: &std::ffi::OsStr,
) -> io::Result<fs::File> {
    const FILE_CREATE: u32 = 0x0000_0002;
    const FILE_NON_DIRECTORY_FILE: u32 = 0x0000_0040;
    const FILE_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE};
    use windows_sys::Win32::Storage::FileSystem::{DELETE, FILE_ATTRIBUTE_NORMAL, SYNCHRONIZE};

    let security = OwnerOnlySecurityAttributes::current_user()?;
    open_relative_child(
        parent,
        name,
        GENERIC_READ | GENERIC_WRITE | DELETE | SYNCHRONIZE,
        FILE_ATTRIBUTE_NORMAL,
        FILE_CREATE,
        FILE_NON_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT,
        Some(security.security_descriptor()),
    )
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn create_owner_only_directory_child(
    _parent: &fs::File,
    _name: &std::ffi::OsStr,
) -> io::Result<fs::File> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "owner-only directory creation is unavailable on this host",
    ))
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn create_owner_only_file_child(
    _parent: &fs::File,
    _name: &std::ffi::OsStr,
) -> io::Result<fs::File> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "owner-only file creation is unavailable on this host",
    ))
}

#[cfg(windows)]
pub(crate) fn create_new_regular_child(
    parent: &fs::File,
    name: &std::ffi::OsStr,
) -> io::Result<fs::File> {
    const FILE_CREATE: u32 = 0x0000_0002;
    const FILE_NON_DIRECTORY_FILE: u32 = 0x0000_0040;
    const FILE_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE};
    use windows_sys::Win32::Storage::FileSystem::{DELETE, FILE_ATTRIBUTE_NORMAL, SYNCHRONIZE};

    open_relative_child(
        parent,
        name,
        GENERIC_READ | GENERIC_WRITE | DELETE | SYNCHRONIZE,
        FILE_ATTRIBUTE_NORMAL,
        FILE_CREATE,
        FILE_NON_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT,
        None,
    )
}

/// Returns the stable object on which a store can hold its lifetime ownership
/// lock without rediscovering ownership through a replaceable name.
///
/// Unix locks the retained physical directory object itself. Windows uses a
/// descriptor-relative child opened without delete sharing, so its directory
/// entry cannot be renamed, unlinked, or replaced while the handle is live.
#[cfg(unix)]
pub(crate) fn open_directory_ownership_lock(
    directory: &fs::File,
    _lock_name: &std::ffi::OsStr,
) -> io::Result<fs::File> {
    directory.try_clone()
}

#[cfg(windows)]
pub(crate) fn open_directory_ownership_lock(
    directory: &fs::File,
    lock_name: &std::ffi::OsStr,
) -> io::Result<fs::File> {
    const FILE_OPEN_IF: u32 = 0x0000_0003;
    const FILE_NON_DIRECTORY_FILE: u32 = 0x0000_0040;
    const FILE_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE};
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_REPARSE_POINT,
        FILE_SHARE_READ, FILE_SHARE_WRITE, SYNCHRONIZE,
    };

    let security = OwnerOnlySecurityAttributes::current_user()?;
    let file = open_relative_child_with_share_access(
        directory,
        lock_name,
        GENERIC_READ | GENERIC_WRITE | SYNCHRONIZE,
        FILE_ATTRIBUTE_NORMAL,
        FILE_OPEN_IF,
        FILE_NON_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT,
        FILE_SHARE_READ | FILE_SHARE_WRITE,
        Some(security.security_descriptor()),
    )?;
    let attributes = windows_file_information(&file)?.dwFileAttributes;
    if attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "directory ownership lock resolves to a reparse point",
        ));
    }
    if attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "directory ownership lock is not a regular file",
        ));
    }
    Ok(file)
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn open_directory_ownership_lock(
    _directory: &fs::File,
    _lock_name: &std::ffi::OsStr,
) -> io::Result<fs::File> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "stable directory ownership locking is unavailable on this host",
    ))
}

#[cfg(windows)]
pub(crate) fn create_new_directory_child(
    parent: &fs::File,
    name: &std::ffi::OsStr,
) -> io::Result<fs::File> {
    const FILE_CREATE: u32 = 0x0000_0002;
    const FILE_DIRECTORY_FILE: u32 = 0x0000_0001;
    const FILE_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    use windows_sys::Win32::Storage::FileSystem::{
        DELETE, FILE_ATTRIBUTE_DIRECTORY, FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, READ_CONTROL,
        SYNCHRONIZE,
    };

    let created = open_relative_child(
        parent,
        name,
        DELETE | FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES | READ_CONTROL | SYNCHRONIZE,
        FILE_ATTRIBUTE_DIRECTORY,
        FILE_CREATE,
        FILE_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT,
        None,
    )?;
    if let Err(primary) = validate_directory_handle(&created) {
        return match discard_created_child(&created) {
            Ok(()) => Err(primary),
            Err(cleanup) => Err(io::Error::new(
                primary.kind(),
                format!(
                    "{primary}; failed to remove invalid directory created through the retained parent: {cleanup}"
                ),
            )),
        };
    }
    // A directory handle with listing or deletion access blocks the internal
    // cross-directory target open performed by FILE_RENAME_INFORMATION on
    // Windows. Keep the creation handle until the identity-safe reopen has
    // succeeded, then retain only the least-privilege destination handle.
    let expected_identity = match file_identity(&created) {
        Ok(identity) => identity,
        Err(primary) => {
            return match discard_created_child(&created) {
                Ok(()) => Err(primary),
                Err(cleanup) => Err(io::Error::new(
                    primary.kind(),
                    format!(
                        "{primary}; failed to remove unverified directory created through the retained parent: {cleanup}"
                    ),
                )),
            };
        }
    };
    let reopened = match open_directory_child_for_rename_destination(parent, name) {
        Ok(reopened) => reopened,
        Err(primary) => {
            return match discard_created_child(&created) {
                Ok(()) => Err(primary),
                Err(cleanup) => Err(io::Error::new(
                    primary.kind(),
                    format!(
                        "{primary}; failed to remove unreopenable directory created through the retained parent: {cleanup}"
                    ),
                )),
            };
        }
    };
    let actual_identity = match file_identity(&reopened) {
        Ok(identity) => identity,
        Err(primary) => {
            return match discard_created_child(&created) {
                Ok(()) => Err(primary),
                Err(cleanup) => Err(io::Error::new(
                    primary.kind(),
                    format!(
                        "{primary}; failed to remove unverified reopened directory created through the retained parent: {cleanup}"
                    ),
                )),
            };
        }
    };
    if actual_identity != expected_identity {
        let primary = io::Error::new(
            io::ErrorKind::InvalidData,
            "created directory identity changed before rename-compatible reopen",
        );
        return match discard_created_child(&created) {
            Ok(()) => Err(primary),
            Err(cleanup) => Err(io::Error::new(
                primary.kind(),
                format!("{primary}; failed to remove replaced created directory: {cleanup}"),
            )),
        };
    }
    drop(created);
    Ok(reopened)
}

#[cfg(windows)]
fn open_directory_child_for_rename_destination(
    parent: &fs::File,
    name: &std::ffi::OsStr,
) -> io::Result<fs::File> {
    const FILE_OPEN: u32 = 0x0000_0001;
    const FILE_DIRECTORY_FILE: u32 = 0x0000_0001;
    const FILE_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_READ_ATTRIBUTES, FILE_TRAVERSE, SYNCHRONIZE,
    };

    let directory = open_relative_child(
        parent,
        name,
        FILE_TRAVERSE | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
        0,
        FILE_OPEN,
        FILE_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT,
        None,
    )?;
    validate_directory_handle(&directory)?;
    Ok(directory)
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn create_new_regular_child(
    _parent: &fs::File,
    _name: &std::ffi::OsStr,
) -> io::Result<fs::File> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "descriptor-relative regular-file creation is unavailable on this host",
    ))
}

#[cfg(not(any(unix, windows)))]
fn open_or_create_regular_child_read_write_nofollow(
    _parent: &fs::File,
    _name: &std::ffi::OsStr,
) -> io::Result<fs::File> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "descriptor-relative lifecycle locking is unavailable on this host",
    ))
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn create_new_directory_child(
    _parent: &fs::File,
    _name: &std::ffi::OsStr,
) -> io::Result<fs::File> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "descriptor-relative directory creation is unavailable on this host",
    ))
}

#[cfg(windows)]
pub(crate) fn discard_created_child(file: &fs::File) -> io::Result<()> {
    set_delete_disposition(file)
}

#[cfg(windows)]
fn set_delete_disposition(file: &fs::File) -> io::Result<()> {
    use std::os::windows::io::AsRawHandle;

    const FILE_DISPOSITION_INFORMATION: u32 = 13;
    let mut delete_file = 1u8;
    let mut status = NtIoStatusBlock {
        status: NtIoStatusStatus { status: 0 },
        information: 0,
    };
    // SAFETY: file is an owned handle opened with DELETE access by the child-creation helpers,
    // and delete_file/status remain writable for the duration of the native call. Once marked,
    // the only subsequent operation on the handle is Drop/close.
    let result = unsafe {
        NtSetInformationFile(
            file.as_raw_handle(),
            &mut status,
            (&mut delete_file as *mut u8).cast(),
            std::mem::size_of_val(&delete_file) as u32,
            FILE_DISPOSITION_INFORMATION,
        )
    };
    if result < 0 {
        Err(nt_status_error(result))
    } else {
        Ok(())
    }
}

#[cfg(windows)]
#[allow(
    dead_code,
    reason = "DirectoryAnchor callers are introduced by the following Windows full-dump task"
)]
pub(crate) fn create_owner_only_directory(path: &Path) -> io::Result<fs::File> {
    use windows_sys::Win32::Storage::FileSystem::CreateDirectoryW;

    let api_path = windows_api_path(path)?;
    let security = OwnerOnlySecurityAttributes::current_user()?;
    // SAFETY: path is NUL-terminated and security owns the descriptor for this call.
    if unsafe { CreateDirectoryW(api_path.as_ptr(), security.as_ptr()) } == 0 {
        return Err(io::Error::last_os_error());
    }
    open_directory_nofollow(path)
}

#[cfg(windows)]
#[allow(
    dead_code,
    reason = "DirectoryAnchor callers are introduced by the following Windows full-dump task"
)]
pub(crate) fn create_owner_only_file(path: &Path) -> io::Result<fs::File> {
    use std::os::windows::io::FromRawHandle;
    use std::ptr;
    use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, CREATE_NEW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_DELETE,
    };

    let path = windows_api_path(path)?;
    let security = OwnerOnlySecurityAttributes::current_user()?;
    // SAFETY: path is NUL-terminated and security owns the descriptor for this call.
    let handle = unsafe {
        CreateFileW(
            path.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_DELETE,
            security.as_ptr(),
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        Err(io::Error::last_os_error())
    } else {
        // SAFETY: CreateFileW returned an owned, valid file handle.
        Ok(unsafe { fs::File::from_raw_handle(handle) })
    }
}

#[cfg(windows)]
#[allow(
    dead_code,
    reason = "DirectoryAnchor callers are introduced by the following Windows full-dump task"
)]
pub(crate) fn verify_owner_only_acl(file: &fs::File) -> io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use std::ptr;
    use windows_sys::Win32::Security::Authorization::{GetSecurityInfo, SE_FILE_OBJECT};
    use windows_sys::Win32::Security::DACL_SECURITY_INFORMATION;

    let mut dacl = ptr::null_mut();
    let mut descriptor = ptr::null_mut();
    // SAFETY: file owns a valid handle and all requested output pointers are writable.
    let status = unsafe {
        GetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            &mut dacl,
            ptr::null_mut(),
            &mut descriptor,
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    let descriptor = LocalSecurityDescriptor(descriptor);
    verify_owner_only_security_descriptor(descriptor.0)
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn verify_owner_only_acl(_file: &fs::File) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "owner-only ACL verification is unavailable on this host",
    ))
}

#[cfg(windows)]
fn verify_owner_only_security_descriptor(
    descriptor: windows_sys::Win32::Security::PSECURITY_DESCRIPTOR,
) -> io::Result<()> {
    use std::mem::size_of;
    use std::ptr;
    use windows_sys::Win32::Security::{
        AclSizeInformation, EqualSid, GetAce, GetAclInformation, GetSecurityDescriptorControl,
        GetSecurityDescriptorDacl, ACCESS_ALLOWED_ACE, ACE_HEADER, ACL_SIZE_INFORMATION,
        SE_DACL_PROTECTED,
    };
    use windows_sys::Win32::Storage::FileSystem::FILE_ALL_ACCESS;

    const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;

    if descriptor.is_null() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "owner-only object has no security descriptor",
        ));
    }
    let token = ProcessToken::current_user()?;
    let mut dacl_present = 0;
    let mut dacl = ptr::null_mut();
    let mut dacl_defaulted = 0;
    // SAFETY: descriptor is non-null and all requested output pointers are writable.
    if unsafe {
        GetSecurityDescriptorDacl(
            descriptor,
            &mut dacl_present,
            &mut dacl,
            &mut dacl_defaulted,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    if dacl_present == 0 || dacl.is_null() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "owner-only object has no DACL",
        ));
    }

    let mut control = 0;
    let mut revision = 0;
    // SAFETY: descriptor is valid for the duration of this call.
    if unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if control & SE_DACL_PROTECTED == 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "owner-only object DACL is not protected",
        ));
    }

    let mut information = ACL_SIZE_INFORMATION {
        AceCount: 0,
        AclBytesInUse: 0,
        AclBytesFree: 0,
    };
    // SAFETY: dacl is valid for the descriptor lifetime and information is writable storage.
    if unsafe {
        GetAclInformation(
            dacl,
            (&mut information as *mut ACL_SIZE_INFORMATION).cast(),
            size_of::<ACL_SIZE_INFORMATION>() as u32,
            AclSizeInformation,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    if information.AceCount != 1 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "owner-only object DACL does not contain exactly one ACE",
        ));
    }

    let mut ace = ptr::null_mut();
    // SAFETY: dacl is valid and ACE index zero is within the exactly-one ACE DACL.
    if unsafe { GetAce(dacl, 0, &mut ace) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: GetAce returned a pointer to a valid ACE in dacl.
    let header = unsafe { &*ace.cast::<ACE_HEADER>() };
    if header.AceType != ACCESS_ALLOWED_ACE_TYPE {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "owner-only object DACL has an unexpected ACE",
        ));
    }
    if header.AceFlags != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "owner-only object DACL has unexpected ACE flags",
        ));
    }
    // SAFETY: the ACE type is ACCESS_ALLOWED_ACE_TYPE, so its payload has this layout.
    let allowed = unsafe { &*ace.cast::<ACCESS_ALLOWED_ACE>() };
    if allowed.Mask != FILE_ALL_ACCESS {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "owner-only object DACL does not grant full access",
        ));
    }
    let ace_sid: windows_sys::Win32::Security::PSID =
        (&raw const allowed.SidStart).cast_mut().cast();
    // SAFETY: both SIDs are valid: one belongs to the ACL and the other to the current token.
    if unsafe { EqualSid(ace_sid, token.user_sid()) } == 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "owner-only object DACL grants access to a different SID",
        ));
    }
    Ok(())
}

pub(crate) fn install_file_no_clobber(source: &Path, target: &Path) -> io::Result<()> {
    fs::hard_link(source, target)
}

#[cfg(any(target_os = "linux", target_os = "android"))]
pub(crate) fn rename_no_replace(source: &Path, target: &Path) -> io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let source = CString::new(source.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "source path contains NUL"))?;
    let target = CString::new(target.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "target path contains NUL"))?;
    #[cfg(target_os = "linux")]
    let no_replace_flag = libc::RENAME_NOREPLACE;
    #[cfg(target_os = "android")]
    let no_replace_flag = libc::RENAME_NOREPLACE as libc::c_uint;
    // SAFETY: both C strings are NUL-terminated and remain live for the syscall. The raw syscall
    // avoids a glibc-only symbol so this remains available on Linux musl; RENAME_NOREPLACE asks
    // the kernel to fail atomically when the destination already exists.
    let moved = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            target.as_ptr(),
            no_replace_flag,
        )
    };
    if moved == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(target_vendor = "apple")]
pub(crate) fn rename_no_replace(source: &Path, target: &Path) -> io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let source = CString::new(source.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "source path contains NUL"))?;
    let target = CString::new(target.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "target path contains NUL"))?;
    // SAFETY: both C strings are NUL-terminated and remain live for the call. RENAME_EXCL makes
    // destination existence an atomic failure instead of replacing it.
    let moved = unsafe { libc::renamex_np(source.as_ptr(), target.as_ptr(), libc::RENAME_EXCL) };
    if moved == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(windows)]
pub(crate) fn rename_no_replace(source: &Path, target: &Path) -> io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH};

    let source = windows_api_path(source)?;
    let target = windows_api_path(target)?;
    // SAFETY: both pointers reference NUL-terminated UTF-16 buffers for the call duration.
    // Omitting MOVEFILE_REPLACE_EXISTING makes destination existence an atomic failure.
    let moved = unsafe { MoveFileExW(source.as_ptr(), target.as_ptr(), MOVEFILE_WRITE_THROUGH) };
    if moved == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "android",
    target_vendor = "apple",
    windows
)))]
pub(crate) fn rename_no_replace(_source: &Path, _target: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "atomic no-replace rename is not available on this host",
    ))
}

#[cfg(windows)]
pub(crate) fn host_path_text(path: String) -> String {
    path.replace('\\', "/")
}

#[cfg(not(windows))]
pub(crate) fn host_path_text(path: String) -> String {
    path
}

/// Returns a version-stable byte representation for persistent path identity.
/// Valid Unicode paths retain their UTF-8 contract on every supported host;
/// non-Unicode paths use an explicit, platform-tagged native encoding instead
/// of `OsStr::as_encoded_bytes`, whose representation may change between Rust
/// versions.
pub(crate) fn stable_path_identity_bytes(path: &Path) -> Result<Vec<u8>, String> {
    if let Some(path) = path.to_str() {
        return Ok(path.as_bytes().to_vec());
    }
    stable_non_utf8_path_identity_bytes(path)
}

#[cfg(unix)]
fn stable_non_utf8_path_identity_bytes(path: &Path) -> Result<Vec<u8>, String> {
    use std::os::unix::ffi::OsStrExt;

    let mut encoded = b"\xffunica-path-unix-v1\0".to_vec();
    encoded.extend_from_slice(path.as_os_str().as_bytes());
    Ok(encoded)
}

#[cfg(windows)]
fn stable_non_utf8_path_identity_bytes(path: &Path) -> Result<Vec<u8>, String> {
    use std::os::windows::ffi::OsStrExt;

    let mut encoded = b"\xffunica-path-utf16le-v1\0".to_vec();
    for unit in path.as_os_str().encode_wide() {
        encoded.extend_from_slice(&unit.to_le_bytes());
    }
    Ok(encoded)
}

#[cfg(not(any(unix, windows)))]
fn stable_non_utf8_path_identity_bytes(path: &Path) -> Result<Vec<u8>, String> {
    Err(format!(
        "stable non-Unicode path identity is unsupported on this platform: {}",
        path.display()
    ))
}

#[cfg(all(test, unix))]
pub(crate) fn distinct_non_unicode_paths_for_test() -> Option<(PathBuf, PathBuf)> {
    use std::os::unix::ffi::OsStringExt;

    Some((
        PathBuf::from(std::ffi::OsString::from_vec(
            b"/workspace/source-\x80".to_vec(),
        )),
        PathBuf::from(std::ffi::OsString::from_vec(
            b"/workspace/source-\x81".to_vec(),
        )),
    ))
}

#[cfg(all(test, windows))]
pub(crate) fn distinct_non_unicode_paths_for_test() -> Option<(PathBuf, PathBuf)> {
    use std::os::windows::ffi::OsStringExt;

    let prefix = "/workspace/source-".encode_utf16().collect::<Vec<_>>();
    let mut first = prefix.clone();
    first.push(0xd800);
    let mut second = prefix;
    second.push(0xd801);
    Some((
        PathBuf::from(std::ffi::OsString::from_wide(&first)),
        PathBuf::from(std::ffi::OsString::from_wide(&second)),
    ))
}

#[cfg(all(test, not(any(unix, windows))))]
pub(crate) fn distinct_non_unicode_paths_for_test() -> Option<(PathBuf, PathBuf)> {
    None
}

#[cfg(windows)]
pub(crate) fn host_filesystem_case_sensitive(path: &Path) -> io::Result<bool> {
    open_absolute_directory_path_nofollow(path)
        .and_then(|directory| relative_child_object_attributes(&directory))
        .map(|attributes| attributes == 0)
}

#[cfg(target_vendor = "apple")]
pub(crate) fn host_filesystem_case_sensitive(path: &Path) -> io::Result<bool> {
    use std::os::fd::AsRawFd;

    let directory = open_absolute_directory_path_nofollow(path)?;
    // SAFETY: directory owns a valid descriptor and fpathconf only queries it.
    let result = unsafe { libc::fpathconf(directory.as_raw_fd(), libc::_PC_CASE_SENSITIVE) };
    if result == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(result != 0)
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn host_filesystem_case_sensitive(path: &Path) -> io::Result<bool> {
    use std::mem::MaybeUninit;
    use std::os::fd::AsRawFd;

    let directory = open_absolute_directory_path_nofollow(path)?;
    let mut filesystem = MaybeUninit::<libc::statfs>::uninit();
    // SAFETY: directory owns a valid descriptor and fstatfs initializes the
    // pointed structure on success.
    if unsafe { libc::fstatfs(directory.as_raw_fd(), filesystem.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: fstatfs succeeded above.
    let filesystem_type = unsafe { filesystem.assume_init() }.f_type as u32 as u64;
    let directory_flags = if matches!(
        filesystem_type,
        LINUX_EXT4_SUPER_MAGIC | LINUX_F2FS_SUPER_MAGIC
    ) {
        let mut flags: libc::c_long = 0;
        // SAFETY: the descriptor is an open directory and flags points to
        // writable storage of the type required by FS_IOC_GETFLAGS.
        if unsafe { libc::ioctl(directory.as_raw_fd(), libc::FS_IOC_GETFLAGS, &mut flags) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Some(flags as u32)
    } else {
        None
    };
    linux_filesystem_case_sensitive_from_metadata(filesystem_type, directory_flags)
}

#[cfg(any(target_os = "linux", test))]
const LINUX_EXT4_SUPER_MAGIC: u64 = 0x0000_ef53;
#[cfg(any(target_os = "linux", test))]
const LINUX_F2FS_SUPER_MAGIC: u64 = 0xf2f5_2010;
#[cfg(any(target_os = "linux", test))]
const LINUX_BTRFS_SUPER_MAGIC: u64 = 0x9123_683e;
#[cfg(any(target_os = "linux", test))]
const LINUX_TMPFS_MAGIC: u64 = 0x0102_1994;
#[cfg(any(target_os = "linux", test))]
const LINUX_RAMFS_MAGIC: u64 = 0x8584_58f6;
#[cfg(any(target_os = "linux", test))]
const LINUX_FS_CASEFOLD_FL: u32 = 0x4000_0000;

#[cfg(any(target_os = "linux", test))]
fn linux_filesystem_case_sensitive_from_metadata(
    filesystem_type: u64,
    directory_flags: Option<u32>,
) -> io::Result<bool> {
    match filesystem_type {
        LINUX_EXT4_SUPER_MAGIC | LINUX_F2FS_SUPER_MAGIC => {
            let flags = directory_flags.ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::Unsupported,
                    "Linux filesystem did not expose directory casefold flags",
                )
            })?;
            if flags & LINUX_FS_CASEFOLD_FL != 0 {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "kernel Unicode casefold semantics cannot be proven by a userspace key",
                ));
            }
            Ok(true)
        }
        LINUX_BTRFS_SUPER_MAGIC | LINUX_TMPFS_MAGIC | LINUX_RAMFS_MAGIC => Ok(true),
        _ => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!(
                "case-sensitivity cannot be proven for Linux filesystem type {filesystem_type:#x}"
            ),
        )),
    }
}

#[cfg(all(not(windows), not(target_vendor = "apple"), not(target_os = "linux")))]
pub(crate) fn host_filesystem_case_sensitive(_path: &Path) -> io::Result<bool> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "filesystem case-sensitivity cannot be proven on this host",
    ))
}

#[cfg(windows)]
pub(crate) fn host_path_components_equal(
    left: &std::ffi::OsStr,
    right: &std::ffi::OsStr,
    case_sensitive: bool,
) -> io::Result<bool> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Globalization::{CompareStringOrdinal, CSTR_EQUAL};

    if case_sensitive {
        return Ok(left == right);
    }
    let left = left.encode_wide().collect::<Vec<_>>();
    let right = right.encode_wide().collect::<Vec<_>>();
    let left_len = i32::try_from(left.len()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "left path component is too long",
        )
    })?;
    let right_len = i32::try_from(right.len()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "right path component is too long",
        )
    })?;
    // SAFETY: both vectors remain live for the call and their explicit lengths
    // bound every read. CompareStringOrdinal is the identity relation used by
    // case-insensitive Windows path lookup; a userspace Unicode transform is
    // not equivalent because it may expand one component into several chars.
    let comparison =
        unsafe { CompareStringOrdinal(left.as_ptr(), left_len, right.as_ptr(), right_len, 1) };
    if comparison == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(comparison == CSTR_EQUAL)
    }
}

pub(crate) fn host_directory_child_names_equal(
    parent_path: &Path,
    left: &std::ffi::OsStr,
    right: &std::ffi::OsStr,
    case_sensitive: bool,
) -> io::Result<bool> {
    if left == right {
        return Ok(true);
    }
    if left.as_encoded_bytes().is_ascii() && right.as_encoded_bytes().is_ascii() {
        return if case_sensitive {
            Ok(false)
        } else {
            Ok(left
                .as_encoded_bytes()
                .eq_ignore_ascii_case(right.as_encoded_bytes()))
        };
    }
    if case_sensitive && !cfg!(target_vendor = "apple") {
        return Ok(false);
    }
    let parent = match open_absolute_directory_path_nofollow(parent_path) {
        Ok(parent) => parent,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            #[cfg(target_vendor = "apple")]
            {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "Apple path identity needs an existing parent directory",
                ));
            }
            #[cfg(not(target_vendor = "apple"))]
            {
                return host_path_components_equal(left, right, case_sensitive);
            }
        }
        Err(error) => return Err(error),
    };
    let left_child = open_directory_child_nofollow(&parent, left);
    let right_child = open_directory_child_nofollow(&parent, right);
    match (left_child, right_child) {
        (Ok(left_child), Ok(right_child)) => {
            Ok(file_identity(&left_child)? == file_identity(&right_child)?)
        }
        (Err(left_error), Err(right_error))
            if left_error.kind() == io::ErrorKind::NotFound
                && right_error.kind() == io::ErrorKind::NotFound =>
        {
            #[cfg(target_vendor = "apple")]
            {
                Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "Apple path identity needs an existing child directory",
                ))
            }
            #[cfg(not(target_vendor = "apple"))]
            {
                host_path_components_equal(left, right, case_sensitive)
            }
        }
        (Err(error), _) | (_, Err(error)) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        (Err(error), _) | (_, Err(error)) => Err(error),
    }
}

pub(crate) fn host_directory_child_identity(
    parent_path: &Path,
    child: &std::ffi::OsStr,
) -> io::Result<Option<FileIdentity>> {
    let parent = match open_absolute_directory_path_nofollow(parent_path) {
        Ok(parent) => parent,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    match open_directory_child_nofollow(&parent, child) {
        Ok(child) => file_identity(&child).map(Some),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
            ) =>
        {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

#[cfg(not(windows))]
pub(crate) fn host_path_components_equal(
    left: &std::ffi::OsStr,
    right: &std::ffi::OsStr,
    case_sensitive: bool,
) -> io::Result<bool> {
    #[cfg(target_vendor = "apple")]
    let _ = case_sensitive;
    if left == right {
        return Ok(true);
    }
    #[cfg(target_vendor = "apple")]
    return Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Apple path identity needs an existing directory object",
    ));
    #[cfg(not(target_vendor = "apple"))]
    {
        if case_sensitive {
            return Ok(false);
        }
        let left = left.to_str().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "left path component is not valid UTF-8",
            )
        })?;
        let right = right.to_str().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "right path component is not valid UTF-8",
            )
        })?;
        // Linux casefold directories never reach this branch: their kernel Unicode
        // table is rejected as unprovable by host_filesystem_case_sensitive. This
        // simple, non-expanding fold models only injected case-insensitive policy
        // in tests without conflating `ß` and `ss`.
        let simple_upper = |text: &str| {
            text.chars()
                .flat_map(|character| {
                    let uppercase = character.to_uppercase().collect::<Vec<_>>();
                    if uppercase.len() == 1 {
                        uppercase
                    } else {
                        vec![character]
                    }
                })
                .collect::<String>()
        };
        Ok(simple_upper(&left) == simple_upper(&right))
    }
}

#[cfg(unix)]
pub(crate) fn is_link_loop_error(error: &io::Error) -> bool {
    error.raw_os_error() == Some(libc::ELOOP)
}

#[cfg(not(unix))]
pub(crate) fn is_link_loop_error(_error: &io::Error) -> bool {
    false
}

#[cfg(windows)]
pub(crate) fn strip_windows_extended_length_prefix(path: &Path) -> std::path::PathBuf {
    use std::path::PathBuf;

    let path = path.as_os_str().to_string_lossy();
    if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{unc}"));
    }
    if let Some(regular) = path.strip_prefix(r"\\?\") {
        let bytes = regular.as_bytes();
        if bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && matches!(bytes[2], b'\\' | b'/')
        {
            return PathBuf::from(regular);
        }
    }
    PathBuf::from(path.as_ref())
}

#[cfg(not(windows))]
pub(crate) fn strip_windows_extended_length_prefix(path: &Path) -> std::path::PathBuf {
    path.to_path_buf()
}

#[cfg(windows)]
pub(crate) fn is_foreign_absolute_path(_path: &str) -> bool {
    false
}

#[cfg(not(windows))]
pub(crate) fn is_foreign_absolute_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    (bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'/')
        || path.starts_with("//")
}

#[cfg(windows)]
pub(crate) fn path_starts_with_host_root(path: &Path, root: &Path) -> bool {
    let path = strip_windows_extended_length_prefix(path);
    let root = strip_windows_extended_length_prefix(root);
    let path_components = path.components().collect::<Vec<_>>();
    let root_components = root.components().collect::<Vec<_>>();
    path_components.len() >= root_components.len()
        && path_components
            .iter()
            .zip(root_components.iter())
            .all(|(left, right)| {
                left.as_os_str().to_string_lossy().to_lowercase()
                    == right.as_os_str().to_string_lossy().to_lowercase()
            })
}

#[cfg(not(windows))]
pub(crate) fn path_starts_with_host_root(path: &Path, root: &Path) -> bool {
    path.starts_with(root)
}

#[cfg(all(test, unix))]
pub(crate) fn create_file_symlink_for_test(
    source: impl AsRef<Path>,
    target: impl AsRef<Path>,
) -> Option<io::Result<()>> {
    use std::os::unix::fs::symlink;

    Some(symlink(source, target))
}

#[cfg(all(test, unix))]
pub(crate) fn create_dir_symlink_for_test(
    source: impl AsRef<Path>,
    target: impl AsRef<Path>,
) -> Option<io::Result<()>> {
    use std::os::unix::fs::symlink;

    Some(symlink(source, target))
}

#[cfg(all(test, unix))]
pub(crate) fn remove_dir_symlink_for_test(path: impl AsRef<Path>) -> io::Result<()> {
    fs::remove_file(path)
}

#[cfg(all(test, windows))]
pub(crate) fn create_file_symlink_for_test(
    source: impl AsRef<Path>,
    target: impl AsRef<Path>,
) -> Option<io::Result<()>> {
    use std::os::windows::fs::symlink_file;

    Some(symlink_file(source, target))
}

#[cfg(all(test, windows))]
pub(crate) fn create_dir_symlink_for_test(
    source: impl AsRef<Path>,
    target: impl AsRef<Path>,
) -> Option<io::Result<()>> {
    use std::os::windows::fs::symlink_dir;

    Some(symlink_dir(source, target))
}

#[cfg(all(test, windows))]
pub(crate) fn remove_dir_symlink_for_test(path: impl AsRef<Path>) -> io::Result<()> {
    fs::remove_dir(path)
}

#[cfg(all(test, not(any(unix, windows))))]
pub(crate) fn create_file_symlink_for_test(
    _source: impl AsRef<Path>,
    _target: impl AsRef<Path>,
) -> Option<io::Result<()>> {
    None
}

#[cfg(all(test, not(any(unix, windows))))]
pub(crate) fn create_dir_symlink_for_test(
    _source: impl AsRef<Path>,
    _target: impl AsRef<Path>,
) -> Option<io::Result<()>> {
    None
}

#[cfg(all(test, not(any(unix, windows))))]
pub(crate) fn remove_dir_symlink_for_test(_path: impl AsRef<Path>) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "test directory links are unavailable on this host",
    ))
}

pub(crate) fn metadata_is_link_or_reparse_point(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    metadata_is_reparse_point(metadata)
}

#[cfg(windows)]
fn metadata_is_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn metadata_is_reparse_point(_metadata: &fs::Metadata) -> bool {
    false
}

#[cfg(not(windows))]
pub(crate) fn replace_file_atomically(source: &Path, target: &Path) -> io::Result<()> {
    fs::rename(source, target)
}

#[cfg(windows)]
pub(crate) fn replace_file_atomically(source: &Path, target: &Path) -> io::Result<()> {
    const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(existing: *const u16, replacement: *const u16, flags: u32) -> i32;
    }

    let source = windows_api_path(source)?;
    let target = windows_api_path(target)?;
    // SAFETY: both pointers reference NUL-terminated UTF-16 buffers for the call duration.
    let moved = unsafe {
        MoveFileExW(
            source.as_ptr(),
            target.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(unix)]
pub(crate) fn sync_parent_directory(parent: &Path) -> io::Result<()> {
    File::open(parent).and_then(|directory| directory.sync_all())
}

#[cfg(not(unix))]
pub(crate) fn sync_parent_directory(_parent: &Path) -> io::Result<()> {
    Ok(())
}

/// Flushes mutations made through a retained directory handle where the host
/// supports durable directory synchronization.
#[cfg(unix)]
pub(crate) fn sync_directory(directory: &fs::File) -> io::Result<()> {
    directory.sync_all()
}

#[cfg(not(unix))]
pub(crate) fn sync_directory(_directory: &fs::File) -> io::Result<()> {
    Ok(())
}

#[cfg(not(windows))]
pub(crate) fn prepare_file_for_removal(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(windows)]
#[allow(
    clippy::permissions_set_readonly_false,
    reason = "on Windows this only clears the FILE_ATTRIBUTE_READONLY flag"
)]
pub(crate) fn prepare_file_for_removal(path: &Path) -> io::Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let mut permissions = metadata.permissions();
    if permissions.readonly() {
        permissions.set_readonly(false);
        fs::set_permissions(path, permissions)?;
    }
    Ok(())
}

/// Best-effort cleanup for a just-created regular child whose stable identity
/// could not be captured. The caller still retains both creation anchors.
#[cfg(unix)]
pub(crate) fn discard_created_regular_child(
    _parent: &fs::File,
    _name: &std::ffi::OsStr,
    _file: &fs::File,
) -> io::Result<()> {
    Err(io::Error::other(
        "created regular-file identity is unavailable; artifact left untouched",
    ))
}

#[cfg(any(unix, windows))]
pub(crate) fn retain_regular_child_for_cleanup(
    parent: &fs::File,
    name: &std::ffi::OsStr,
    expected_identity: FileIdentity,
) -> io::Result<fs::File> {
    let (retained, kind) = open_any_child_nofollow(parent, name)?;
    if kind != OpenedChildKind::RegularFile || file_identity(&retained)? != expected_identity {
        return Err(io::Error::other(
            "cleanup retention target identity changed; replacement left untouched",
        ));
    }
    Ok(retained)
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn retain_regular_child_for_cleanup(
    _parent: &fs::File,
    _name: &std::ffi::OsStr,
    _expected_identity: FileIdentity,
) -> io::Result<fs::File> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "identity-only cleanup retention is unavailable on this host",
    ))
}

/// Moves an identity-bound regular child between retained parent directories
/// without replacing an existing destination.
///
/// The child name is rechecked immediately before the descriptor-relative
/// rename used under the publication lock, so replacing either lexical parent
/// route cannot redirect the move. Windows additionally moves through the
/// verified child handle.
#[cfg(unix)]
pub(crate) fn rename_identity_bound_regular_child_no_replace(
    source_parent: &fs::File,
    source_name: &std::ffi::OsStr,
    expected_identity: FileIdentity,
    retained: &fs::File,
    destination_parent: &fs::File,
    destination_name: &std::ffi::OsStr,
) -> io::Result<()> {
    if file_identity(retained)? != expected_identity {
        return Err(io::Error::other(
            "retained regular child identity changed; replacement left untouched",
        ));
    }
    let named = open_regular_child_nofollow(source_parent, source_name)?;
    if file_identity(&named)? != expected_identity {
        return Err(io::Error::other(
            "regular child identity changed; replacement left untouched",
        ));
    }
    rename_regular_child_at_no_replace(
        source_parent,
        source_name,
        destination_parent,
        destination_name,
    )
}

/// Atomically replaces a child in one retained directory with an
/// identity-bound staged regular file from that same directory.
///
/// Both source lookup and destination resolution stay relative to the retained
/// parent handle, so replacing the lexical route to the directory cannot
/// redirect publication.
#[cfg(unix)]
pub(crate) fn replace_identity_bound_regular_child(
    parent: &fs::File,
    source_name: &std::ffi::OsStr,
    expected_identity: FileIdentity,
    retained: &fs::File,
    destination_name: &std::ffi::OsStr,
) -> io::Result<()> {
    use std::os::fd::AsRawFd;

    if file_identity(retained)? != expected_identity {
        return Err(io::Error::other(
            "retained regular child identity changed; replacement left untouched",
        ));
    }
    let named = open_regular_child_nofollow(parent, source_name)?;
    if file_identity(&named)? != expected_identity {
        return Err(io::Error::other(
            "regular child identity changed; replacement left untouched",
        ));
    }
    let source_name = unix_child_name(source_name)?;
    let destination_name = unix_child_name(destination_name)?;
    // SAFETY: the retained directory descriptor and both validated,
    // NUL-terminated child names remain live for the atomic same-directory call.
    let status = unsafe {
        libc::renameat(
            parent.as_raw_fd(),
            source_name.as_ptr(),
            parent.as_raw_fd(),
            destination_name.as_ptr(),
        )
    };
    if status == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(windows)]
pub(crate) fn replace_identity_bound_regular_child(
    parent: &fs::File,
    source_name: &std::ffi::OsStr,
    expected_identity: FileIdentity,
    retained: &fs::File,
    destination_name: &std::ffi::OsStr,
) -> io::Result<()> {
    let named = open_any_child_for_delete(parent, source_name)?;
    if opened_child_kind(&named)? != OpenedChildKind::RegularFile
        || file_identity(&named)? != expected_identity
    {
        return Err(io::Error::other(
            "regular child identity changed; replacement left untouched",
        ));
    }
    if opened_child_kind(retained)? != OpenedChildKind::RegularFile
        || file_identity(retained)? != expected_identity
    {
        return Err(io::Error::other(
            "retained regular child identity changed; replacement left untouched",
        ));
    }
    rename_open_child(&named, parent, destination_name, true)
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn replace_identity_bound_regular_child(
    _parent: &fs::File,
    _source_name: &std::ffi::OsStr,
    _expected_identity: FileIdentity,
    _retained: &fs::File,
    _destination_name: &std::ffi::OsStr,
) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "identity-bound regular-file replacement is unavailable on this host",
    ))
}

#[cfg(windows)]
pub(crate) fn rename_identity_bound_regular_child_no_replace(
    source_parent: &fs::File,
    source_name: &std::ffi::OsStr,
    expected_identity: FileIdentity,
    retained: &fs::File,
    destination_parent: &fs::File,
    destination_name: &std::ffi::OsStr,
) -> io::Result<()> {
    let named = open_any_child_for_delete(source_parent, source_name)?;
    if opened_child_kind(&named)? != OpenedChildKind::RegularFile
        || file_identity(&named)? != expected_identity
    {
        return Err(io::Error::other(
            "regular child identity changed; replacement left untouched",
        ));
    }
    if opened_child_kind(retained)? != OpenedChildKind::RegularFile
        || file_identity(retained)? != expected_identity
    {
        return Err(io::Error::other(
            "retained regular child identity changed; replacement left untouched",
        ));
    }
    rename_open_child_no_replace(&named, destination_parent, destination_name)
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn rename_identity_bound_regular_child_no_replace(
    _source_parent: &fs::File,
    _source_name: &std::ffi::OsStr,
    _expected_identity: FileIdentity,
    _retained: &fs::File,
    _destination_parent: &fs::File,
    _destination_name: &std::ffi::OsStr,
) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "identity-bound regular-file rename is unavailable on this host",
    ))
}

#[cfg(unix)]
fn rename_regular_child_at_no_replace(
    source_parent: &fs::File,
    source_name: &std::ffi::OsStr,
    destination_parent: &fs::File,
    destination_name: &std::ffi::OsStr,
) -> io::Result<()> {
    use std::os::fd::AsRawFd;

    let source_name = unix_child_name(source_name)?;
    let destination_name = unix_child_name(destination_name)?;
    #[cfg(target_os = "linux")]
    let no_replace_flag = libc::RENAME_NOREPLACE;
    #[cfg(target_os = "android")]
    let no_replace_flag = libc::RENAME_NOREPLACE as libc::c_uint;
    #[cfg(any(target_os = "linux", target_os = "android"))]
    // SAFETY: both retained directory descriptors and both NUL-terminated child names remain
    // live for the syscall. RENAME_NOREPLACE atomically protects destination absence.
    let status = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            source_parent.as_raw_fd(),
            source_name.as_ptr(),
            destination_parent.as_raw_fd(),
            destination_name.as_ptr(),
            no_replace_flag,
        )
    };
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    // SAFETY: both retained directory descriptors and both NUL-terminated child names remain
    // live for the syscall. RENAME_EXCL atomically protects destination absence.
    let status = unsafe {
        libc::renameatx_np(
            source_parent.as_raw_fd(),
            source_name.as_ptr(),
            destination_parent.as_raw_fd(),
            destination_name.as_ptr(),
            libc::RENAME_EXCL,
        )
    };
    #[cfg(not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios"
    )))]
    let status = {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "descriptor-relative no-replace rename is unavailable on this host",
        ));
    };
    if status == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(windows)]
pub(crate) fn discard_created_regular_child(
    _parent: &fs::File,
    _name: &std::ffi::OsStr,
    file: &fs::File,
) -> io::Result<()> {
    discard_created_child(file)
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn discard_created_regular_child(
    _parent: &fs::File,
    _name: &std::ffi::OsStr,
    _file: &fs::File,
) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "created regular-file cleanup is unavailable on this host",
    ))
}

/// Removes the regular child only while its retained parent handle and file
/// identity still name the object captured by the caller.
///
/// The parent handle is the mutation anchor, so replacing the lexical parent
/// route cannot redirect cleanup into another directory. The child name is
/// rechecked immediately before the descriptor-relative unlink used under the
/// publication lock; Windows additionally deletes through the verified child
/// handle.
#[cfg(unix)]
pub(crate) fn remove_identity_bound_regular_child(
    parent: &fs::File,
    name: &std::ffi::OsStr,
    expected_identity: FileIdentity,
    retained: &fs::File,
) -> io::Result<()> {
    if file_identity(retained)? != expected_identity {
        return Err(io::Error::other(
            "retained regular child identity changed; replacement left untouched",
        ));
    }
    let child = open_regular_child_nofollow(parent, name)?;
    if file_identity(&child)? != expected_identity {
        return Err(io::Error::other(
            "regular child identity changed; replacement left untouched",
        ));
    }
    unlink_child_at(parent, name, 0)
}

#[cfg(windows)]
#[allow(
    clippy::permissions_set_readonly_false,
    reason = "the verified child handle changes only FILE_ATTRIBUTE_READONLY before deletion"
)]
pub(crate) fn remove_identity_bound_regular_child(
    parent: &fs::File,
    name: &std::ffi::OsStr,
    expected_identity: FileIdentity,
    retained: &fs::File,
) -> io::Result<()> {
    let named = open_child_for_delete_and_attribute_write(parent, name)?;
    if opened_child_kind(&named)? != OpenedChildKind::RegularFile
        || file_identity(&named)? != expected_identity
    {
        return Err(io::Error::other(
            "regular child identity changed; replacement left untouched",
        ));
    }
    if opened_child_kind(retained)? != OpenedChildKind::RegularFile {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "identity-bound cleanup target is not a regular file",
        ));
    }
    if file_identity(retained)? != expected_identity {
        return Err(io::Error::other(
            "regular child identity changed; replacement left untouched",
        ));
    }
    let mut permissions = named.metadata()?.permissions();
    if permissions.readonly() {
        permissions.set_readonly(false);
        named.set_permissions(permissions)?;
    }
    delete_open_child(&named)
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn remove_identity_bound_regular_child(
    _parent: &fs::File,
    _name: &std::ffi::OsStr,
    _expected_identity: FileIdentity,
    _retained: &fs::File,
) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "identity-bound regular-file cleanup is unavailable on this host",
    ))
}

/// Removes an empty directory child through its retained parent handle after
/// proving the exact directory identity.
#[cfg(unix)]
pub(crate) fn remove_identity_bound_empty_directory_child(
    parent: &fs::File,
    name: &std::ffi::OsStr,
    expected_identity: FileIdentity,
    retained: &fs::File,
) -> io::Result<()> {
    if file_identity(retained)? != expected_identity {
        return Err(io::Error::other(
            "retained directory child identity changed; replacement left untouched",
        ));
    }
    let child = open_directory_child_nofollow(parent, name)?;
    if file_identity(&child)? != expected_identity {
        return Err(io::Error::other(
            "directory child identity changed; replacement left untouched",
        ));
    }
    unlink_child_at(parent, name, libc::AT_REMOVEDIR)
}

#[cfg(windows)]
pub(crate) fn remove_identity_bound_empty_directory_child(
    parent: &fs::File,
    name: &std::ffi::OsStr,
    expected_identity: FileIdentity,
    retained: &fs::File,
) -> io::Result<()> {
    let named = open_any_child_for_delete(parent, name)?;
    if opened_child_kind(&named)? != OpenedChildKind::Directory
        || file_identity(&named)? != expected_identity
    {
        return Err(io::Error::other(
            "directory child identity changed; replacement left untouched",
        ));
    }
    if opened_child_kind(retained)? != OpenedChildKind::Directory {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "identity-bound cleanup target is not a directory",
        ));
    }
    if file_identity(retained)? != expected_identity {
        return Err(io::Error::other(
            "directory child identity changed; replacement left untouched",
        ));
    }
    delete_open_child(&named)
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn remove_identity_bound_empty_directory_child(
    _parent: &fs::File,
    _name: &std::ffi::OsStr,
    _expected_identity: FileIdentity,
    _retained: &fs::File,
) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "identity-bound directory cleanup is unavailable on this host",
    ))
}

#[cfg(unix)]
fn unlink_child_at(
    parent: &fs::File,
    name: &std::ffi::OsStr,
    flags: libc::c_int,
) -> io::Result<()> {
    use std::os::fd::AsRawFd;

    let name = unix_child_name(name)?;
    // SAFETY: parent remains open and name is a live NUL-terminated string.
    let status = unsafe { libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), flags) };
    if status == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

pub(crate) fn path_lock_identity(path: &Path) -> String {
    path_lock_identity_text(&path.to_string_lossy())
}

#[cfg(windows)]
pub(crate) fn provider_state_path_identity(path: &Path) -> Vec<u8> {
    path_lock_identity(path).into_bytes()
}

#[cfg(unix)]
pub(crate) fn provider_state_path_identity(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;

    path.as_os_str().as_bytes().to_vec()
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn provider_state_path_identity(path: &Path) -> Vec<u8> {
    path.to_string_lossy().into_owned().into_bytes()
}

#[cfg(any(windows, target_os = "macos"))]
fn path_lock_identity_text(path: &str) -> String {
    path.to_lowercase()
}

#[cfg(not(any(windows, target_os = "macos")))]
fn path_lock_identity_text(path: &str) -> String {
    path.to_string()
}

#[cfg(test)]
mod tests {
    use super::{
        path_lock_identity_text, path_starts_with_host_root, provider_state_path_identity,
        stable_path_identity_bytes, windows_api_path_from_utf16,
    };
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[cfg(windows)]
    use super::strip_windows_extended_length_prefix;

    #[cfg(any(unix, windows))]
    #[test]
    fn secure_absolute_directory_creation_builds_missing_components_from_anchors() {
        use super::{
            file_identity, open_directory_nofollow, open_or_create_absolute_directory_path_nofollow,
        };

        let root = unique_temp_root("secure-open-or-create");
        fs::create_dir_all(&root).unwrap();
        let physical_root = fs::canonicalize(&root).unwrap();
        let requested = physical_root.join("provider").join("state");

        let opened = open_or_create_absolute_directory_path_nofollow(&requested).unwrap();

        assert_eq!(
            file_identity(&opened).unwrap(),
            file_identity(&open_directory_nofollow(&requested).unwrap()).unwrap()
        );
        drop(opened);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn secure_absolute_directory_creation_rejects_link_ancestor_without_writing_through_it() {
        use super::{create_test_directory_link, open_or_create_absolute_directory_path_nofollow};

        #[cfg(windows)]
        const ERROR_PRIVILEGE_NOT_HELD: i32 = 1314;
        let root = unique_temp_root("secure-open-or-create-link");
        fs::create_dir_all(&root).unwrap();
        let physical_root = fs::canonicalize(&root).unwrap();
        let redirected = physical_root.join("redirected");
        fs::create_dir(&redirected).unwrap();
        let link = physical_root.join("link");
        if let Err(error) = create_test_directory_link(&redirected, &link) {
            #[cfg(windows)]
            if error.raw_os_error() == Some(ERROR_PRIVILEGE_NOT_HELD) {
                fs::remove_dir_all(root).unwrap();
                return;
            }
            panic!("failed to create directory link fixture: {error}");
        }

        assert!(open_or_create_absolute_directory_path_nofollow(&link.join("state")).is_err());
        assert!(!redirected.join("state").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    mod windows {
        use super::{fs, io, unique_temp_root};
        use crate::infrastructure::platform::filesystem::{
            capture_windows_immutable_entry_evidence, create_new_directory_child,
            create_owner_only_directory, create_owner_only_directory_child, create_owner_only_file,
            create_owner_only_file_child, delete_open_child, directory_query_is_end, file_identity,
            nt_create_options_for_std_file, open_any_child_for_delete, open_any_child_nofollow,
            open_directory_child_for_rename, open_directory_child_nofollow,
            open_directory_nofollow, open_regular_child_nofollow, opened_child_kind,
            parse_directory_information_buffer, read_directory_names,
            rename_directory_handle_child_no_replace,
            rename_identity_bound_regular_child_no_replace, verify_owner_only_acl,
            verify_owner_only_security_descriptor, verify_thread_token_fallback_error,
            verify_windows_elevation_value, verify_windows_immutable_security_descriptor,
            verify_windows_local_fixed_device_info, verify_windows_local_fixed_volume,
            with_case_sensitivity_query_error, EffectiveTokenSource, OpenedChildKind,
            OwnerOnlySecurityAttributes, ProcessToken, RetainedDirectoryCapability,
            WindowsImmutableAclProfile,
        };
        use std::ffi::OsString;
        use std::mem::{offset_of, size_of};
        use std::ptr;
        use windows_sys::Win32::Foundation::{LocalFree, GENERIC_ALL, GENERIC_WRITE};
        use windows_sys::Win32::Foundation::{
            ERROR_ACCESS_DENIED, ERROR_CALL_NOT_IMPLEMENTED, ERROR_CANT_OPEN_ANONYMOUS,
            ERROR_FILE_NOT_FOUND, ERROR_INVALID_FUNCTION, ERROR_INVALID_PARAMETER,
            ERROR_NOT_SUPPORTED, ERROR_NO_MORE_FILES, ERROR_NO_TOKEN,
        };
        use windows_sys::Win32::Security::Authorization::{
            ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
        };
        use windows_sys::Win32::Security::PSECURITY_DESCRIPTOR;
        use windows_sys::Win32::Security::{ImpersonateSelf, RevertToSelf, SecurityImpersonation};
        use windows_sys::Win32::Storage::FileSystem::{
            DELETE, FILE_ADD_FILE, FILE_ADD_SUBDIRECTORY, FILE_DELETE_CHILD, FILE_ID_BOTH_DIR_INFO,
            FILE_WRITE_ATTRIBUTES, FILE_WRITE_DATA, FILE_WRITE_EA, WRITE_DAC, WRITE_OWNER,
        };

        struct TestSecurityDescriptor(PSECURITY_DESCRIPTOR);

        impl Drop for TestSecurityDescriptor {
            fn drop(&mut self) {
                // SAFETY: the SDDL conversion API allocated this descriptor with LocalAlloc.
                unsafe { LocalFree(self.0) };
            }
        }

        fn descriptor_from_sddl(sddl: &str) -> TestSecurityDescriptor {
            let mut wide = sddl.encode_utf16().collect::<Vec<_>>();
            wide.push(0);
            let mut descriptor = ptr::null_mut();
            // SAFETY: wide is NUL-terminated and descriptor is writable output storage.
            let converted = unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    wide.as_ptr(),
                    SDDL_REVISION_1,
                    &mut descriptor,
                    ptr::null_mut(),
                )
            };
            assert_ne!(converted, 0, "{}", io::Error::last_os_error());
            TestSecurityDescriptor(descriptor)
        }

        fn descriptor_with_untrusted_mask(
            owner: &str,
            mask: u32,
            flags: &str,
        ) -> TestSecurityDescriptor {
            descriptor_from_sddl(&format!(
                "O:{owner}D:(A;;FA;;;SY)(A;{flags};0x{mask:08x};;;BU)"
            ))
        }

        #[test]
        fn windows_immutable_platform_accepts_trusted_owners_and_read_execute_users() {
            const TRUSTED_INSTALLER: &str =
                "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464";
            for owner in [TRUSTED_INSTALLER, "SY", "BA"] {
                let descriptor = descriptor_from_sddl(&format!(
                    "O:{owner}D:(A;;FA;;;SY)(A;;FA;;;BA)(A;;0x001200a9;;;BU)"
                ));

                verify_windows_immutable_security_descriptor(
                    descriptor.0,
                    WindowsImmutableAclProfile::Installation,
                )
                .unwrap();
            }
        }

        #[test]
        fn windows_immutable_platform_rejects_an_untrusted_owner() {
            let descriptor = descriptor_from_sddl("O:BUD:(A;;FA;;;SY)(A;;FRFX;;;BU)");

            let error = verify_windows_immutable_security_descriptor(
                descriptor.0,
                WindowsImmutableAclProfile::Installation,
            )
            .unwrap_err();

            assert!(error.to_string().contains("owner"), "{error}");
        }

        #[test]
        fn windows_immutable_platform_ancestry_allows_sibling_creation_only() {
            let descriptor =
                descriptor_with_untrusted_mask("SY", FILE_ADD_FILE | FILE_ADD_SUBDIRECTORY, "");

            verify_windows_immutable_security_descriptor(
                descriptor.0,
                WindowsImmutableAclProfile::Ancestry,
            )
            .unwrap();
        }

        #[test]
        fn windows_immutable_platform_ancestry_rejects_untrusted_substitution_rights() {
            for mask in [
                DELETE,
                FILE_DELETE_CHILD,
                WRITE_DAC,
                WRITE_OWNER,
                GENERIC_WRITE,
                GENERIC_ALL,
            ] {
                let descriptor = descriptor_with_untrusted_mask("SY", mask, "");

                let error = verify_windows_immutable_security_descriptor(
                    descriptor.0,
                    WindowsImmutableAclProfile::Ancestry,
                )
                .unwrap_err();

                assert!(
                    error.to_string().contains("mutation"),
                    "mask 0x{mask:08x}: {error}"
                );
            }
        }

        #[test]
        fn windows_immutable_platform_inventory_rejects_every_untrusted_mutation_right() {
            for mask in [
                FILE_WRITE_DATA,
                FILE_ADD_FILE,
                FILE_ADD_SUBDIRECTORY,
                FILE_WRITE_EA,
                FILE_WRITE_ATTRIBUTES,
                FILE_DELETE_CHILD,
                DELETE,
                WRITE_DAC,
                WRITE_OWNER,
                GENERIC_WRITE,
                GENERIC_ALL,
            ] {
                let descriptor = descriptor_with_untrusted_mask("BA", mask, "");

                let error = verify_windows_immutable_security_descriptor(
                    descriptor.0,
                    WindowsImmutableAclProfile::Installation,
                )
                .unwrap_err();

                assert!(
                    error.to_string().contains("mutation"),
                    "mask 0x{mask:08x}: {error}"
                );
            }
        }

        #[test]
        fn windows_immutable_platform_ignores_inherit_only_capabilities_on_current_entry() {
            let descriptor = descriptor_with_untrusted_mask("SY", GENERIC_ALL, "OICIIO");

            verify_windows_immutable_security_descriptor(
                descriptor.0,
                WindowsImmutableAclProfile::Installation,
            )
            .unwrap();
        }

        #[test]
        fn windows_immutable_platform_rejects_missing_null_and_unsupported_dacls() {
            let absent = descriptor_from_sddl("O:SY");
            let null = descriptor_from_sddl("O:SYD:NO_ACCESS_CONTROL");
            let object =
                descriptor_from_sddl("O:SYD:(OA;;FA;00112233-4455-6677-8899-aabbccddeeff;;BU)");
            let mut malformed = [0usize; 4];

            for descriptor in [
                absent.0,
                null.0,
                object.0,
                malformed.as_mut_ptr().cast(),
                ptr::null_mut(),
            ] {
                assert!(verify_windows_immutable_security_descriptor(
                    descriptor,
                    WindowsImmutableAclProfile::Installation,
                )
                .is_err());
            }
        }

        #[test]
        fn windows_immutable_platform_rejects_an_elevated_caller() {
            let error = verify_windows_elevation_value(1).unwrap_err();

            assert!(error.to_string().contains("elevated"), "{error}");
            verify_windows_elevation_value(0).unwrap();
        }

        #[test]
        fn windows_immutable_platform_effective_token_falls_back_only_when_thread_has_no_token() {
            verify_thread_token_fallback_error(ERROR_NO_TOKEN).unwrap();

            for expected in [ERROR_ACCESS_DENIED, ERROR_CANT_OPEN_ANONYMOUS] {
                let error = verify_thread_token_fallback_error(expected).unwrap_err();
                assert_eq!(error.raw_os_error(), Some(expected as i32));
            }
        }

        #[test]
        fn windows_immutable_platform_effective_token_uses_process_token_without_impersonation() {
            let token = ProcessToken::current_user().unwrap();

            assert_eq!(token.source, EffectiveTokenSource::Process);
        }

        #[test]
        fn windows_immutable_platform_effective_token_selects_an_impersonation_token() {
            struct RevertGuard;

            impl Drop for RevertGuard {
                fn drop(&mut self) {
                    // SAFETY: this guard is created only after ImpersonateSelf succeeds.
                    assert_ne!(unsafe { RevertToSelf() }, 0);
                }
            }

            // SAFETY: SecurityImpersonation is a documented impersonation level.
            assert_ne!(
                unsafe { ImpersonateSelf(SecurityImpersonation) },
                0,
                "{}",
                io::Error::last_os_error()
            );
            let _guard = RevertGuard;

            let token = ProcessToken::current_user().unwrap();

            assert_eq!(token.source, EffectiveTokenSource::Thread);
        }

        #[test]
        fn windows_immutable_platform_rejects_nonlocal_or_nonfixed_device_information() {
            const FILE_DEVICE_CD_ROM: u32 = 0x0000_0002;
            const FILE_DEVICE_DISK: u32 = 0x0000_0007;
            const FILE_DEVICE_NETWORK_FILE_SYSTEM: u32 = 0x0000_0014;
            const FILE_DEVICE_VIRTUAL_DISK: u32 = 0x0000_0024;
            const FILE_PORTABLE_DEVICE: u32 = 0x0000_4000;
            const FILE_REMOTE_DEVICE: u32 = 0x0000_0010;
            const FILE_REMOTE_DEVICE_VSMB: u32 = 0x0008_0000;
            const FILE_REMOVABLE_MEDIA: u32 = 0x0000_0001;
            const FILE_DEVICE_IS_MOUNTED: u32 = 0x0000_0020;

            verify_windows_local_fixed_device_info(FILE_DEVICE_DISK, FILE_DEVICE_IS_MOUNTED)
                .unwrap();
            for (device_type, characteristics) in [
                (FILE_DEVICE_DISK, FILE_REMOTE_DEVICE),
                (FILE_DEVICE_DISK, FILE_REMOTE_DEVICE_VSMB),
                (FILE_DEVICE_DISK, FILE_PORTABLE_DEVICE),
                (FILE_DEVICE_DISK, FILE_REMOVABLE_MEDIA),
                (FILE_DEVICE_NETWORK_FILE_SYSTEM, FILE_DEVICE_IS_MOUNTED),
                (FILE_DEVICE_CD_ROM, FILE_DEVICE_IS_MOUNTED),
                (FILE_DEVICE_VIRTUAL_DISK, FILE_DEVICE_IS_MOUNTED),
                (0xffff_ffff, 0),
            ] {
                assert!(
                    verify_windows_local_fixed_device_info(device_type, characteristics).is_err(),
                    "device type 0x{device_type:08x}, characteristics 0x{characteristics:08x}"
                );
            }
        }

        #[test]
        fn windows_immutable_platform_accepts_a_real_local_fixed_volume_handle() {
            let root = unique_temp_root("immutable-local-volume");
            fs::create_dir_all(&root).unwrap();
            let handle = open_directory_nofollow(&root).unwrap();

            verify_windows_local_fixed_volume(&handle).unwrap();

            drop(handle);
            fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn windows_immutable_platform_attestation_handle_evidence_stays_bound_to_the_open_object() {
            let root = unique_temp_root("immutable-handle-evidence");
            fs::create_dir_all(&root).unwrap();
            let original = root.join("entry");
            let displaced = root.join("displaced");
            let original_handle = create_owner_only_file(&original).unwrap();
            let expected_identity = file_identity(&original_handle).unwrap();
            fs::rename(&original, &displaced).unwrap();
            fs::write(&original, b"decoy").unwrap();
            let decoy_handle = fs::File::open(&original).unwrap();

            let retained = capture_windows_immutable_entry_evidence(&original_handle).unwrap();
            let decoy = capture_windows_immutable_entry_evidence(&decoy_handle).unwrap();

            assert_eq!(retained.identity, expected_identity);
            assert_ne!(retained.identity, decoy.identity);
            assert_ne!(
                retained.security_descriptor_sha256,
                decoy.security_descriptor_sha256
            );
            drop(decoy_handle);
            drop(original_handle);
            fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn owner_only_directory_is_opened_without_following_reparse_points() {
            let root = unique_temp_root("owner-only-directory");
            fs::create_dir_all(&root).unwrap();
            let private = root.join("private");

            let handle = create_owner_only_directory(&private).unwrap();

            verify_owner_only_acl(&handle).unwrap();
            assert_eq!(
                file_identity(&handle).unwrap(),
                file_identity(&open_directory_nofollow(&private).unwrap()).unwrap()
            );
            fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn directory_open_rejects_a_reparse_point() {
            let root = unique_temp_root("directory-reparse");
            let real = root.join("real");
            let link = root.join("link");
            fs::create_dir_all(&real).unwrap();
            std::os::windows::fs::symlink_dir(&real, &link).unwrap();

            let error = open_directory_nofollow(&link).unwrap_err();

            assert!(matches!(
                error.kind(),
                io::ErrorKind::InvalidInput | io::ErrorKind::PermissionDenied
            ));
            fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn directory_open_rejects_an_ordinary_file() {
            let root = unique_temp_root("directory-file");
            fs::create_dir_all(&root).unwrap();
            let file = root.join("not-a-directory");
            fs::write(&file, b"not a directory").unwrap();

            let error = open_directory_nofollow(&file).unwrap_err();

            assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
            fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn owner_only_file_has_the_final_acl_at_creation() {
            let root = unique_temp_root("owner-only-file");
            fs::create_dir_all(&root).unwrap();
            let private = root.join("effective.yaml");

            let handle = create_owner_only_file(&private).unwrap();

            verify_owner_only_acl(&handle).unwrap();
            fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn owner_only_acl_verifier_rejects_a_reduced_access_mask() {
            use windows_sys::Win32::Security::{
                GetAce, GetSecurityDescriptorDacl, ACCESS_ALLOWED_ACE,
            };

            let security = OwnerOnlySecurityAttributes::current_user().unwrap();
            let mut dacl_present = 0;
            let mut dacl = ptr::null_mut();
            let mut dacl_defaulted = 0;
            // SAFETY: the security owner retains a valid descriptor and all output pointers are
            // writable for the duration of the call.
            assert_ne!(
                unsafe {
                    GetSecurityDescriptorDacl(
                        security.security_descriptor(),
                        &mut dacl_present,
                        &mut dacl,
                        &mut dacl_defaulted,
                    )
                },
                0
            );
            assert_ne!(dacl_present, 0);
            let mut ace = ptr::null_mut();
            // SAFETY: the owner-only descriptor contains exactly one access-allowed ACE.
            assert_ne!(unsafe { GetAce(dacl, 0, &mut ace) }, 0);
            // SAFETY: the SDDL fixture creates an ACCESS_ALLOWED_ACE at index zero.
            unsafe { (*ace.cast::<ACCESS_ALLOWED_ACE>()).Mask = 0 };

            let error =
                verify_owner_only_security_descriptor(security.security_descriptor()).unwrap_err();

            assert!(error.to_string().contains("full access"), "{error}");
        }

        #[test]
        fn owner_only_acl_verifier_rejects_an_unexpected_ace_flag() {
            use windows_sys::Win32::Security::{
                GetAce, GetSecurityDescriptorDacl, ACE_HEADER, INHERIT_ONLY_ACE,
            };

            let security = OwnerOnlySecurityAttributes::current_user().unwrap();
            let mut dacl_present = 0;
            let mut dacl = ptr::null_mut();
            let mut dacl_defaulted = 0;
            // SAFETY: the security owner retains a valid descriptor and all output pointers are
            // writable for the duration of the call.
            assert_ne!(
                unsafe {
                    GetSecurityDescriptorDacl(
                        security.security_descriptor(),
                        &mut dacl_present,
                        &mut dacl,
                        &mut dacl_defaulted,
                    )
                },
                0
            );
            assert_ne!(dacl_present, 0);
            let mut ace = ptr::null_mut();
            // SAFETY: the owner-only descriptor contains exactly one ACE.
            assert_ne!(unsafe { GetAce(dacl, 0, &mut ace) }, 0);
            // SAFETY: GetAce returned a valid ACE header inside the live descriptor.
            unsafe { (*ace.cast::<ACE_HEADER>()).AceFlags = INHERIT_ONLY_ACE as u8 };

            let error =
                verify_owner_only_security_descriptor(security.security_descriptor()).unwrap_err();

            assert!(error.to_string().contains("flags"), "{error}");
        }

        #[test]
        fn owner_only_file_child_is_created_through_its_retained_parent() {
            use std::io::Write;

            let root = unique_temp_root("owner-only-file-child");
            fs::create_dir_all(&root).unwrap();
            let parent = open_directory_nofollow(&root).unwrap();

            let mut handle =
                create_owner_only_file_child(&parent, std::ffi::OsStr::new("effective.yaml"))
                    .unwrap();

            handle.write_all(b"private").unwrap();
            verify_owner_only_acl(&handle).unwrap();
            drop(handle);
            drop(parent);
            assert_eq!(fs::read(root.join("effective.yaml")).unwrap(), b"private");
            fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn windows_relative_open_fails_closed_when_parent_case_sensitivity_is_ambiguous() {
            let root = unique_temp_root("ambiguous-parent-case-sensitivity");
            fs::create_dir_all(&root).unwrap();
            fs::write(root.join("entry.txt"), b"entry").unwrap();
            let parent = open_directory_nofollow(&root).unwrap();

            let error = with_case_sensitivity_query_error(ERROR_ACCESS_DENIED, || {
                open_regular_child_nofollow(&parent, std::ffi::OsStr::new("entry.txt"))
            })
            .expect_err("an ambiguous parent case-sensitivity query must fail closed");

            assert!(error.to_string().contains("case-sensitive"), "{error}");
            drop(parent);
            fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn windows_relative_open_accepts_a_declined_case_sensitivity_query() {
            use std::io::Read;

            for reported in [
                ERROR_INVALID_FUNCTION,
                ERROR_INVALID_PARAMETER,
                ERROR_NOT_SUPPORTED,
                ERROR_CALL_NOT_IMPLEMENTED,
            ] {
                let root = unique_temp_root("unsupported-parent-case-sensitivity");
                fs::create_dir_all(&root).unwrap();
                fs::write(root.join("entry.txt"), b"entry").unwrap();
                let parent = open_directory_nofollow(&root).unwrap();

                let mut child = with_case_sensitivity_query_error(reported, || {
                    open_regular_child_nofollow(&parent, std::ffi::OsStr::new("entry.txt"))
                })
                .unwrap_or_else(|error| {
                    panic!("a declined case-sensitivity query must still open the child, but Windows error {reported} was reported as {error}")
                });

                let mut contents = Vec::new();
                child.read_to_end(&mut contents).unwrap();
                assert_eq!(contents, b"entry");
                drop(child);
                drop(parent);
                fs::remove_dir_all(root).unwrap();
            }
        }

        #[test]
        fn windows_regular_child_open_reports_directory_as_wrong_kind() {
            let root = unique_temp_root("regular-child-directory");
            fs::create_dir_all(root.join("Archive.xml")).unwrap();
            let parent = open_directory_nofollow(&root).unwrap();

            let error = open_regular_child_nofollow(&parent, std::ffi::OsStr::new("Archive.xml"))
                .expect_err("a directory must not be returned as a regular child");

            assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{error}");
            drop(parent);
            fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn owner_only_directory_child_can_be_reopened_for_delete() {
            let root = unique_temp_root("owner-only-directory-delete-reopen");
            fs::create_dir_all(&root).unwrap();
            let root_handle = open_directory_nofollow(&root).unwrap();
            let parent =
                create_owner_only_directory_child(&root_handle, std::ffi::OsStr::new("parent"))
                    .unwrap();
            let created =
                create_owner_only_directory_child(&parent, std::ffi::OsStr::new("private"))
                    .unwrap();
            let retained_file =
                create_owner_only_file_child(&created, std::ffi::OsStr::new("retained.txt"))
                    .unwrap();
            let expected_identity = file_identity(&created).unwrap();

            let reopened =
                open_any_child_for_delete(&parent, std::ffi::OsStr::new("private")).unwrap();

            assert_eq!(
                opened_child_kind(&reopened).unwrap(),
                OpenedChildKind::Directory
            );
            assert_eq!(file_identity(&reopened).unwrap(), expected_identity);
            drop(reopened);
            drop(retained_file);
            drop(created);
            drop(parent);
            drop(root_handle);
            fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn regular_child_moves_into_live_created_directory_handle() {
            let root = unique_temp_root("regular-child-live-destination");
            fs::create_dir_all(&root).unwrap();
            let root_handle = open_directory_nofollow(&root).unwrap();
            let source_name = std::ffi::OsStr::new("published.bin");
            let destination_name = std::ffi::OsStr::new("quarantined.bin");
            fs::write(root.join(source_name), b"published bytes").unwrap();
            let retained_source = open_regular_child_nofollow(&root_handle, source_name).unwrap();
            let source_identity = file_identity(&retained_source).unwrap();
            let recovery =
                create_new_directory_child(&root_handle, std::ffi::OsStr::new("recovery")).unwrap();

            rename_identity_bound_regular_child_no_replace(
                &root_handle,
                source_name,
                source_identity,
                &retained_source,
                &recovery,
                destination_name,
            )
            .expect("a retained recovery directory must accept an identity-bound child move");

            assert!(!root.join(source_name).exists());
            assert_eq!(
                fs::read(root.join("recovery").join(destination_name)).unwrap(),
                b"published bytes"
            );
            drop(retained_source);
            drop(recovery);
            drop(root_handle);
            fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn identity_bound_regular_child_atomically_replaces_in_retained_parent() {
            use crate::infrastructure::platform::filesystem::{
                create_new_regular_child, replace_identity_bound_regular_child,
            };
            use std::io::Write;

            let root = unique_temp_root("identity-bound-file-replace");
            fs::create_dir_all(&root).unwrap();
            let parent = open_directory_nofollow(&root).unwrap();
            let stage_name = std::ffi::OsStr::new("stage.bin");
            let target_name = std::ffi::OsStr::new("record.bin");
            fs::write(root.join(target_name), b"old").unwrap();
            let mut stage = create_new_regular_child(&parent, stage_name).unwrap();
            stage.write_all(b"new").unwrap();
            stage.sync_all().unwrap();
            let identity = file_identity(&stage).unwrap();

            replace_identity_bound_regular_child(
                &parent,
                stage_name,
                identity,
                &stage,
                target_name,
            )
            .unwrap();

            assert_eq!(fs::read(root.join(target_name)).unwrap(), b"new");
            drop(stage);
            drop(parent);
            fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn windows_retained_atomic_replace_returns_the_named_destination_capability() {
            let root = unique_temp_root("retained-atomic-replace-destination");
            fs::create_dir_all(&root).unwrap();
            fs::write(root.join("record.json"), b"old").unwrap();
            let directory = RetainedDirectoryCapability::open(&root).unwrap();

            let published = directory
                .replace_regular_child_atomically(
                    std::ffi::OsStr::new("stage.tmp"),
                    std::ffi::OsStr::new("record.json"),
                    b"new",
                )
                .expect("native Windows replacement must flush and retain its renamed handle");

            published.validate_named_identity().unwrap();
            assert_eq!(published.read_bounded(16).unwrap(), b"new");
            drop(published);
            drop(directory);
            fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn ownership_lock_child_cannot_be_replaced_or_deleted_while_locked() {
            use crate::infrastructure::platform::filesystem::{
                open_directory_ownership_lock, replace_file_atomically,
            };
            use fs2::FileExt;

            let root = unique_temp_root("directory-ownership-lock-sharing");
            fs::create_dir_all(&root).unwrap();
            let parent = open_directory_nofollow(&root).unwrap();
            let lock_name = std::ffi::OsStr::new(".owner.lock");
            let owner = open_directory_ownership_lock(&parent, lock_name).unwrap();
            owner.try_lock_exclusive().unwrap();

            assert!(fs::rename(root.join(lock_name), root.join("displaced.lock")).is_err());
            assert!(fs::remove_file(root.join(lock_name)).is_err());
            let replacement = root.join("replacement.lock");
            fs::write(&replacement, b"replacement").unwrap();
            assert!(replace_file_atomically(&replacement, &root.join(lock_name)).is_err());
            let contender = open_directory_ownership_lock(&parent, lock_name).unwrap();
            assert!(contender.try_lock_exclusive().is_err());

            drop(contender);
            drop(owner);
            drop(parent);
            fs::remove_file(replacement).unwrap();
            fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn generic_delete_child_open_does_not_require_attribute_write_access() {
            use std::os::windows::ffi::OsStrExt;
            use windows_sys::Win32::Security::{
                GetAce, GetSecurityDescriptorDacl, SetFileSecurityW, ACCESS_ALLOWED_ACE,
                DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
            };
            use windows_sys::Win32::Storage::FileSystem::{FILE_READ_ATTRIBUTES, SYNCHRONIZE};

            let root = unique_temp_root("delete-child-without-write-attributes");
            fs::create_dir_all(&root).unwrap();
            let child_path = root.join("cleanup.bin");
            fs::write(&child_path, b"cleanup").unwrap();
            let security = OwnerOnlySecurityAttributes::current_user().unwrap();
            let mut dacl_present = 0;
            let mut dacl = ptr::null_mut();
            let mut dacl_defaulted = 0;
            // SAFETY: the security owner retains a valid descriptor and all output pointers are
            // writable for the duration of the call.
            assert_ne!(
                unsafe {
                    GetSecurityDescriptorDacl(
                        security.security_descriptor(),
                        &mut dacl_present,
                        &mut dacl,
                        &mut dacl_defaulted,
                    )
                },
                0
            );
            assert_ne!(dacl_present, 0);
            let mut ace = ptr::null_mut();
            // SAFETY: the owner-only descriptor contains exactly one access-allowed ACE.
            assert_ne!(unsafe { GetAce(dacl, 0, &mut ace) }, 0);
            // SAFETY: the owner-only descriptor creates an ACCESS_ALLOWED_ACE at index zero.
            unsafe {
                (*ace.cast::<ACCESS_ALLOWED_ACE>()).Mask =
                    DELETE | FILE_READ_ATTRIBUTES | SYNCHRONIZE | WRITE_DAC;
            }
            let mut wide_path = child_path.as_os_str().encode_wide().collect::<Vec<_>>();
            wide_path.push(0);
            // SAFETY: the path is NUL-terminated and the security descriptor stays live through
            // the call. The reduced DACL deliberately grants deletion but not attribute writes.
            assert_ne!(
                unsafe {
                    SetFileSecurityW(
                        wide_path.as_ptr(),
                        DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                        security.security_descriptor(),
                    )
                },
                0,
                "{}",
                io::Error::last_os_error()
            );
            let parent = open_directory_nofollow(&root).unwrap();

            let child = open_any_child_for_delete(&parent, std::ffi::OsStr::new("cleanup.bin"))
                .expect("generic delete-only callers must not require FILE_WRITE_ATTRIBUTES");
            delete_open_child(&child).unwrap();

            drop(child);
            drop(parent);
            assert!(!child_path.exists());
            fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn windows_open_any_child_nofollow_classifies_a_reparse_point() {
            let root = unique_temp_root("open-any-reparse");
            fs::create_dir_all(&root).unwrap();
            let target = root.join("target.txt");
            fs::write(&target, b"target").unwrap();
            std::os::windows::fs::symlink_file(&target, root.join("link.txt")).unwrap();
            let parent = open_directory_nofollow(&root).unwrap();

            let (opened, kind) = open_any_child_nofollow(&parent, std::ffi::OsStr::new("link.txt"))
                .expect("a no-follow open must return the reparse point itself");

            assert_eq!(kind, OpenedChildKind::ReparsePoint);
            assert_eq!(opened_child_kind(&opened).unwrap(), kind);
            drop(opened);
            drop(parent);
            fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn windows_open_any_child_nofollow_classifies_a_directory_reparse_point() {
            const ERROR_PRIVILEGE_NOT_HELD: i32 = 1314;

            let root = unique_temp_root("open-any-directory-reparse");
            fs::create_dir_all(&root).unwrap();
            let target = root.join("target");
            fs::create_dir(&target).unwrap();
            let link = root.join("link");
            if let Err(error) = std::os::windows::fs::symlink_dir(&target, &link) {
                if error.raw_os_error() == Some(ERROR_PRIVILEGE_NOT_HELD) {
                    fs::remove_dir_all(root).unwrap();
                    return;
                }
                panic!("failed to create directory symlink: {error}");
            }
            let parent = open_directory_nofollow(&root).unwrap();

            let (opened, kind) = open_any_child_nofollow(&parent, std::ffi::OsStr::new("link"))
                .expect("a no-follow open must return the directory reparse point itself");

            assert_eq!(kind, OpenedChildKind::ReparsePoint);
            assert_eq!(opened_child_kind(&opened).unwrap(), kind);
            drop(opened);
            drop(parent);
            fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn populated_child_moves_between_live_owner_only_parent_handles() {
            use std::io::Write;

            let root = unique_temp_root("owner-only-directory-rename");
            fs::create_dir_all(&root).unwrap();
            let root_handle = open_directory_nofollow(&root).unwrap();
            let source_parent = create_owner_only_directory_child(
                &root_handle,
                std::ffi::OsStr::new("source-parent"),
            )
            .unwrap();
            let destination_parent = create_owner_only_directory_child(
                &root_handle,
                std::ffi::OsStr::new("destination-parent"),
            )
            .unwrap();
            let mut effective =
                create_owner_only_file_child(&source_parent, std::ffi::OsStr::new("config.yaml"))
                    .unwrap();
            effective.write_all(b"private").unwrap();
            let source =
                create_owner_only_directory_child(&source_parent, std::ffi::OsStr::new("payload"))
                    .unwrap();
            let mut payload =
                create_owner_only_file_child(&source, std::ffi::OsStr::new("new.txt")).unwrap();
            payload.write_all(b"new").unwrap();
            drop(payload);
            drop(source);
            let source_for_rename =
                open_directory_child_for_rename(&source_parent, std::ffi::OsStr::new("payload"))
                    .unwrap();

            rename_directory_handle_child_no_replace(
                &source_for_rename,
                &destination_parent,
                std::ffi::OsStr::new("payload"),
            )
            .unwrap();
            let effective_for_delete =
                open_any_child_for_delete(&source_parent, std::ffi::OsStr::new("config.yaml"))
                    .unwrap();
            delete_open_child(&effective_for_delete).unwrap();
            drop(effective_for_delete);
            drop(effective);
            let source_parent_for_delete =
                open_any_child_for_delete(&root_handle, std::ffi::OsStr::new("source-parent"))
                    .unwrap();

            assert!(!root.join("source-parent").join("payload").exists());
            assert_eq!(
                fs::read(
                    root.join("destination-parent")
                        .join("payload")
                        .join("new.txt")
                )
                .unwrap(),
                b"new"
            );
            drop(source_parent_for_delete);
            drop(source_for_rename);
            drop(destination_parent);
            drop(source_parent);
            drop(root_handle);
            fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn windows_handle_enumeration_uses_the_retained_directory_after_path_replacement() {
            let root = unique_temp_root("retained-enumeration");
            let original = root.join("directory");
            let displaced = root.join("directory-displaced");
            fs::create_dir_all(&original).unwrap();
            fs::write(original.join("alpha.txt"), b"alpha").unwrap();
            fs::write(original.join("zeta.txt"), b"zeta").unwrap();
            let directory = open_directory_nofollow(&original).unwrap();
            fs::rename(&original, &displaced).unwrap();
            fs::create_dir(&original).unwrap();
            fs::write(original.join("decoy.txt"), b"decoy").unwrap();

            let names = read_directory_names(&directory).unwrap();

            assert_eq!(
                names,
                vec![OsString::from("alpha.txt"), OsString::from("zeta.txt")]
            );
            drop(directory);
            fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn windows_handle_enumeration_accepts_a_handle_relative_child() {
            let root = unique_temp_root("child-enumeration");
            let child_path = root.join("child");
            fs::create_dir_all(&child_path).unwrap();
            fs::write(child_path.join("entry.txt"), b"entry").unwrap();
            let parent = open_directory_nofollow(&root).unwrap();
            let child =
                open_directory_child_nofollow(&parent, std::ffi::OsStr::new("child")).unwrap();

            let names = read_directory_names(&child).unwrap();

            assert_eq!(names, vec![OsString::from("entry.txt")]);
            drop(child);
            drop(parent);
            fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn windows_handle_enumeration_accepts_a_new_private_child() {
            let root = unique_temp_root("private-child-enumeration");
            fs::create_dir_all(&root).unwrap();
            let parent = open_directory_nofollow(&root).unwrap();
            let child =
                create_owner_only_directory_child(&parent, std::ffi::OsStr::new("child")).unwrap();
            fs::write(root.join("child").join("entry.txt"), b"entry").unwrap();

            let names = read_directory_names(&child).unwrap();

            assert_eq!(names, vec![OsString::from("entry.txt")]);
            drop(child);
            drop(parent);
            fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn windows_nt_create_options_for_std_file_are_synchronous() {
            const FILE_DIRECTORY_FILE: u32 = 0x0000_0001;
            const FILE_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
            const FILE_SYNCHRONOUS_IO_ALERT: u32 = 0x0000_0010;
            const FILE_SYNCHRONOUS_IO_NONALERT: u32 = 0x0000_0020;
            use windows_sys::Win32::Storage::FileSystem::SYNCHRONIZE;

            let options = nt_create_options_for_std_file(
                SYNCHRONIZE,
                FILE_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT,
            )
            .unwrap();

            assert_ne!(options & FILE_SYNCHRONOUS_IO_NONALERT, 0);
            assert_eq!(options & FILE_SYNCHRONOUS_IO_ALERT, 0);
            assert_ne!(options & FILE_DIRECTORY_FILE, 0);
            assert_ne!(options & FILE_OPEN_REPARSE_POINT, 0);
        }

        #[test]
        fn windows_nt_create_options_reject_asynchronous_std_file_access() {
            let error = nt_create_options_for_std_file(0, 0).unwrap_err();

            assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
            assert!(error.to_string().contains("SYNCHRONIZE"), "{error}");
        }

        #[test]
        fn windows_directory_enumeration_status_classifier_distinguishes_first_query_eos() {
            let no_more = io::Error::from_raw_os_error(ERROR_NO_MORE_FILES as i32);
            let no_match = io::Error::from_raw_os_error(ERROR_FILE_NOT_FOUND as i32);
            let denied = io::Error::from_raw_os_error(ERROR_ACCESS_DENIED as i32);

            assert!(directory_query_is_end(true, &no_more));
            assert!(directory_query_is_end(false, &no_more));
            assert!(directory_query_is_end(true, &no_match));
            assert!(!directory_query_is_end(false, &no_match));
            assert!(!directory_query_is_end(true, &denied));
        }

        #[test]
        fn windows_directory_enumeration_accepts_an_empty_retained_directory() {
            let root = unique_temp_root("empty-enumeration");
            fs::create_dir_all(&root).unwrap();
            let directory = open_directory_nofollow(&root).unwrap();

            assert!(read_directory_names(&directory).unwrap().is_empty());

            drop(directory);
            fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn windows_directory_enumeration_reads_multiple_native_buffers() {
            let root = unique_temp_root("multi-buffer-enumeration");
            fs::create_dir_all(&root).unwrap();
            let mut expected = Vec::new();
            for index in 0..900 {
                let name = format!("{index:04}-{}.xml", "x".repeat(80));
                fs::write(root.join(&name), b"x").unwrap();
                expected.push(OsString::from(name));
            }
            let directory = open_directory_nofollow(&root).unwrap();

            let names = read_directory_names(&directory).unwrap();

            assert_eq!(names, expected);
            drop(directory);
            fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn windows_directory_parser_rejects_an_unaligned_next_record() {
            let mut buffer = vec![0u8; 512];
            let name_length = 2u32;
            let minimum_record_bytes =
                offset_of!(FILE_ID_BOTH_DIR_INFO, FileName) + name_length as usize;
            let next = (((minimum_record_bytes + 7) & !7) + 4) as u32;
            assert!(next as usize >= minimum_record_bytes);
            assert_eq!(next % 8, 4);
            // SAFETY: the test buffer is large enough for each field and unaligned writes accept
            // Vec<u8>'s alignment.
            unsafe {
                std::ptr::write_unaligned(
                    buffer
                        .as_mut_ptr()
                        .add(offset_of!(FILE_ID_BOTH_DIR_INFO, NextEntryOffset))
                        .cast::<u32>(),
                    next,
                );
                std::ptr::write_unaligned(
                    buffer
                        .as_mut_ptr()
                        .add(offset_of!(FILE_ID_BOTH_DIR_INFO, FileNameLength))
                        .cast::<u32>(),
                    name_length,
                );
                std::ptr::write_unaligned(
                    buffer
                        .as_mut_ptr()
                        .add(offset_of!(FILE_ID_BOTH_DIR_INFO, FileName))
                        .cast::<u16>(),
                    b'a' as u16,
                );
            }
            let mut names = Vec::new();

            let error = parse_directory_information_buffer(&buffer, &mut names).unwrap_err();

            assert!(error.to_string().contains("8-byte-aligned"), "{error}");
        }

        #[test]
        fn windows_directory_parser_rejects_a_truncated_rust_header() {
            let buffer = vec![0u8; size_of::<FILE_ID_BOTH_DIR_INFO>() - 1];
            let mut names = Vec::new();

            let error = parse_directory_information_buffer(&buffer, &mut names).unwrap_err();

            assert!(error.to_string().contains("complete header"), "{error}");
        }
    }

    fn unique_temp_root(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "unica-filesystem-{name}-{}-{nanos}",
            std::process::id()
        ))
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn identity_bound_regular_child_cleanup_preserves_same_name_replacement() {
        use crate::infrastructure::platform::filesystem::{
            create_new_regular_child, file_identity, open_directory_nofollow,
            remove_identity_bound_regular_child,
        };
        use std::ffi::OsStr;
        use std::io::Write;

        let root = unique_temp_root("identity-bound-file-cleanup");
        fs::create_dir_all(&root).unwrap();
        let parent = open_directory_nofollow(&root).unwrap();
        let name = OsStr::new("stage.bin");
        let route = root.join(name);
        let displaced = root.join("displaced.bin");
        let mut retained = create_new_regular_child(&parent, name).unwrap();
        retained.write_all(b"owned").unwrap();
        let expected = file_identity(&retained).unwrap();
        fs::rename(&route, &displaced).unwrap();
        fs::write(&route, b"decoy").unwrap();

        let error =
            remove_identity_bound_regular_child(&parent, name, expected, &retained).unwrap_err();

        assert!(error.to_string().contains("identity changed"), "{error}");
        assert_eq!(fs::read(&route).unwrap(), b"decoy");
        assert_eq!(fs::read(&displaced).unwrap(), b"owned");

        let removable_name = OsStr::new("removable.bin");
        let removable = root.join(removable_name);
        let mut removable_retained = create_new_regular_child(&parent, removable_name).unwrap();
        removable_retained.write_all(b"remove me").unwrap();
        let removable_identity = file_identity(&removable_retained).unwrap();
        remove_identity_bound_regular_child(
            &parent,
            removable_name,
            removable_identity,
            &removable_retained,
        )
        .unwrap();
        drop(removable_retained);
        assert!(!removable.exists());

        drop(retained);
        drop(parent);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn identity_bound_replace_stays_with_retained_parent_after_symlink_swap() {
        use crate::infrastructure::platform::filesystem::{
            create_new_regular_child, file_identity, open_directory_nofollow,
            replace_identity_bound_regular_child,
        };
        use std::ffi::OsStr;
        use std::io::Write;
        use std::os::unix::fs::symlink;

        let parent_path = unique_temp_root("identity-bound-replace-parent-swap");
        let root_path = parent_path.join("root");
        let retained_path = parent_path.join("retained");
        let attacker_path = parent_path.join("attacker");
        fs::create_dir_all(&root_path).unwrap();
        fs::create_dir(&attacker_path).unwrap();
        fs::write(root_path.join("record.json"), b"old").unwrap();
        fs::write(attacker_path.join("record.json"), b"attacker").unwrap();
        let root = open_directory_nofollow(&root_path).unwrap();
        let stage_name = OsStr::new("stage.tmp");
        let mut stage = create_new_regular_child(&root, stage_name).unwrap();
        stage.write_all(b"new").unwrap();
        stage.sync_all().unwrap();
        let identity = file_identity(&stage).unwrap();
        fs::rename(&root_path, &retained_path).unwrap();
        symlink(&attacker_path, &root_path).unwrap();

        replace_identity_bound_regular_child(
            &root,
            stage_name,
            identity,
            &stage,
            OsStr::new("record.json"),
        )
        .unwrap();

        assert_eq!(fs::read(retained_path.join("record.json")).unwrap(), b"new");
        assert_eq!(
            fs::read(attacker_path.join("record.json")).unwrap(),
            b"attacker"
        );
        drop(stage);
        drop(root);
        fs::remove_file(&root_path).unwrap();
        fs::remove_dir_all(parent_path).unwrap();
    }

    #[test]
    fn retained_atomic_replace_flushes_renamed_handle_before_directory_sync() {
        let source = include_str!("filesystem.rs");
        let start = source
            .find("pub(crate) fn replace_regular_child_atomically(")
            .expect("retained atomic publisher exists");
        let end = source[start..]
            .find("pub(crate) fn read_relative_regular_bounded(")
            .map(|offset| start + offset)
            .expect("publisher has a bounded source slice");
        let publisher = &source[start..end];
        let rename = publisher
            .find("replace_identity_bound_regular_child(")
            .expect("publisher retains descriptor-relative rename");
        let renamed_flush = publisher
            .find("sync_renamed_regular_child(")
            .expect("publisher flushes the renamed writable handle");
        let directory_sync = publisher
            .find("sync_directory(")
            .expect("publisher retains supported directory durability");

        assert!(
            rename < renamed_flush && renamed_flush < directory_sync,
            "rename must precede descriptor flush, which must precede directory sync"
        );

        let windows_start = source
            .find("#[cfg(windows)]\npub(crate) fn replace_identity_bound_regular_child(")
            .expect("Windows descriptor-relative replacement exists");
        let windows_end = source[windows_start..]
            .find("#[cfg(not(any(unix, windows)))]")
            .map(|offset| windows_start + offset)
            .expect("Windows replacement has a bounded source slice");
        let windows_replace = &source[windows_start..windows_end];
        assert!(windows_replace.contains("rename_open_child("));
        assert!(!windows_replace.contains("MoveFileExW"));
    }

    #[cfg(windows)]
    #[test]
    fn identity_only_cleanup_handle_allows_a_restrictive_hardlink_reader() {
        use crate::infrastructure::platform::filesystem::{
            create_new_regular_child, file_identity, open_directory_nofollow,
            retain_regular_child_for_cleanup,
        };
        use std::ffi::OsStr;
        use std::io::{Read, Write};
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ;

        let root = unique_temp_root("identity-only-cleanup-handle");
        fs::create_dir_all(&root).unwrap();
        let parent = open_directory_nofollow(&root).unwrap();
        let stage_name = OsStr::new("stage.bin");
        let stage = root.join(stage_name);
        let target = root.join("target.bin");
        let mut creation = create_new_regular_child(&parent, stage_name).unwrap();
        creation.write_all(b"published bytes").unwrap();
        let identity = file_identity(&creation).unwrap();
        let retained = retain_regular_child_for_cleanup(&parent, stage_name, identity).unwrap();
        drop(creation);
        fs::hard_link(&stage, &target).unwrap();

        let mut reader = fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(&target)
            .expect("identity-only cleanup handle must not require write/delete sharing");
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"published bytes");

        drop(reader);
        drop(retained);
        drop(parent);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn retained_cleanup_child_blocks_windows_parent_route_displacement() {
        use crate::infrastructure::platform::filesystem::{
            create_new_regular_child, file_identity, open_directory_nofollow,
            retain_regular_child_for_cleanup,
        };
        use std::ffi::OsStr;
        use std::io::Write;

        let root = unique_temp_root("retained-cleanup-parent-route");
        let active = root.join("active");
        let displaced = root.join("displaced");
        fs::create_dir_all(&active).unwrap();
        let parent = open_directory_nofollow(&active).unwrap();
        let stage_name = OsStr::new("stage.bin");
        let stage = active.join(stage_name);
        let mut creation = create_new_regular_child(&parent, stage_name).unwrap();
        creation.write_all(b"owned stage").unwrap();
        let identity = file_identity(&creation).unwrap();
        let retained = retain_regular_child_for_cleanup(&parent, stage_name, identity).unwrap();
        drop(creation);

        let error = fs::rename(&active, &displaced)
            .expect_err("Windows must not reroute a parent with a retained cleanup child");

        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied, "{error}");
        assert_eq!(fs::read(&stage).unwrap(), b"owned stage");
        assert!(!displaced.exists());

        drop(retained);
        drop(parent);
        fs::rename(&active, &displaced)
            .expect("the route must become movable after cleanup anchors are released");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn retained_parent_creation_cannot_be_redirected_to_a_replacement_route() {
        use crate::infrastructure::platform::filesystem::{
            create_new_directory_child, create_new_regular_child, open_directory_nofollow,
        };
        use std::ffi::OsStr;
        use std::io::Write;

        let root = unique_temp_root("retained-parent-create");
        let active = root.join("active");
        let displaced = root.join("displaced");
        fs::create_dir_all(&active).unwrap();
        let parent = open_directory_nofollow(&active).unwrap();
        fs::rename(&active, &displaced).unwrap();
        fs::create_dir(&active).unwrap();
        fs::write(active.join("stage.bin"), b"same-name file decoy").unwrap();
        fs::create_dir(active.join("recovery")).unwrap();

        let mut created_file = create_new_regular_child(&parent, OsStr::new("stage.bin")).unwrap();
        created_file.write_all(b"owned stage").unwrap();
        let created_directory =
            create_new_directory_child(&parent, OsStr::new("recovery")).unwrap();

        assert_eq!(
            fs::read(active.join("stage.bin")).unwrap(),
            b"same-name file decoy"
        );
        assert_eq!(
            fs::read(displaced.join("stage.bin")).unwrap(),
            b"owned stage"
        );
        assert!(active.join("recovery").is_dir());
        assert!(displaced.join("recovery").is_dir());

        drop(created_directory);
        drop(created_file);
        drop(parent);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn identity_bound_empty_directory_cleanup_preserves_same_name_replacement() {
        use crate::infrastructure::platform::filesystem::{
            create_new_directory_child, file_identity, open_directory_nofollow,
            remove_identity_bound_empty_directory_child,
        };
        use std::ffi::OsStr;

        let root = unique_temp_root("identity-bound-directory-cleanup");
        fs::create_dir_all(&root).unwrap();
        let parent = open_directory_nofollow(&root).unwrap();
        let name = OsStr::new("recovery");
        let route = root.join(name);
        let displaced = root.join("displaced-recovery");
        let retained = create_new_directory_child(&parent, name).unwrap();
        let expected = file_identity(&retained).unwrap();
        fs::rename(&route, &displaced).unwrap();
        fs::create_dir(&route).unwrap();

        let error = remove_identity_bound_empty_directory_child(&parent, name, expected, &retained)
            .unwrap_err();

        assert!(error.to_string().contains("identity changed"), "{error}");
        assert!(route.is_dir());
        assert!(displaced.is_dir());

        let removable_name = OsStr::new("removable-recovery");
        let removable = root.join(removable_name);
        let removable_retained = create_new_directory_child(&parent, removable_name).unwrap();
        let removable_identity = file_identity(&removable_retained).unwrap();
        remove_identity_bound_empty_directory_child(
            &parent,
            removable_name,
            removable_identity,
            &removable_retained,
        )
        .unwrap();
        drop(removable_retained);
        assert!(!removable.exists());

        drop(retained);
        drop(parent);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    mod unix_runtime_directory {
        use super::{fs, unique_temp_root};
        use crate::infrastructure::platform::filesystem::ensure_short_private_runtime_dir_unix;
        use std::io;
        use std::os::unix::fs::{symlink, MetadataExt, PermissionsExt};

        fn fixture(name: &str) -> (std::path::PathBuf, std::path::PathBuf) {
            let root = unique_temp_root(name);
            fs::create_dir_all(&root).unwrap();
            let runtime = root.join("runtime");
            (root, runtime)
        }

        fn effective_uid() -> libc::uid_t {
            // SAFETY: `geteuid` has no preconditions and only reads the
            // effective UID of this process.
            unsafe { libc::geteuid() }
        }

        #[test]
        fn creates_owner_only_runtime_directory() {
            let (root, runtime) = fixture("short-runtime-create");

            let actual = ensure_short_private_runtime_dir_unix(&runtime, effective_uid()).unwrap();
            let metadata = fs::symlink_metadata(&runtime).unwrap();

            assert_eq!(actual, runtime);
            assert!(metadata.is_dir());
            assert_eq!(metadata.permissions().mode() & 0o777, 0o700);
            fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn rejects_symlink_at_runtime_path() {
            let (root, runtime) = fixture("short-runtime-symlink");
            let target = root.join("target");
            fs::create_dir(&target).unwrap();
            symlink(&target, &runtime).unwrap();

            let error =
                ensure_short_private_runtime_dir_unix(&runtime, effective_uid()).unwrap_err();

            assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
            fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn rejects_regular_file_at_runtime_path() {
            let (root, runtime) = fixture("short-runtime-file");
            fs::write(&runtime, b"not a directory").unwrap();

            let error =
                ensure_short_private_runtime_dir_unix(&runtime, effective_uid()).unwrap_err();

            assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
            fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn rejects_runtime_directory_owned_by_another_uid() {
            let (root, runtime) = fixture("short-runtime-owner");
            fs::create_dir(&runtime).unwrap();
            fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();
            let actual_uid = fs::symlink_metadata(&runtime).unwrap().uid();

            let error = ensure_short_private_runtime_dir_unix(&runtime, actual_uid.wrapping_add(1))
                .unwrap_err();

            assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
            fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn rejects_runtime_directory_with_non_private_mode() {
            let (root, runtime) = fixture("short-runtime-mode");
            fs::create_dir(&runtime).unwrap();
            fs::set_permissions(&runtime, fs::Permissions::from_mode(0o755)).unwrap();

            let error =
                ensure_short_private_runtime_dir_unix(&runtime, effective_uid()).unwrap_err();

            assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
            fs::remove_dir_all(root).unwrap();
        }
    }

    fn windows_api_path_text(path: &str, absolute: bool) -> String {
        let mut encoded = windows_api_path_from_utf16(path.encode_utf16().collect(), absolute);
        assert_eq!(encoded.pop(), Some(0));
        String::from_utf16(&encoded).unwrap()
    }

    #[test]
    fn windows_api_paths_use_extended_prefixes_without_lossy_text_conversion() {
        assert_eq!(
            windows_api_path_text(r"C:/deep/source.xml", true),
            r"\\?\C:\deep\source.xml"
        );
        assert_eq!(
            windows_api_path_text(r"\\server\share/deep/source.xml", true),
            r"\\?\UNC\server\share\deep\source.xml"
        );
        assert_eq!(
            windows_api_path_text(r"\\?\C:\deep\source.xml", true),
            r"\\?\C:\deep\source.xml"
        );
        assert_eq!(
            windows_api_path_text(r"relative/source.xml", false),
            r"relative/source.xml"
        );
    }

    #[test]
    fn no_clobber_install_never_replaces_an_existing_target() {
        use super::install_file_no_clobber;

        let root = unique_temp_root("no-clobber-install");
        fs::create_dir_all(&root).unwrap();
        let staged = root.join("staged");
        let target = root.join("target");
        fs::write(&staged, b"replacement").unwrap();
        fs::write(&target, b"original").unwrap();

        let error = install_file_no_clobber(&staged, &target).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&staged).unwrap(), b"replacement");
        assert_eq!(fs::read(&target).unwrap(), b"original");

        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn hard_link_count_observes_a_second_name() {
        use super::hard_link_count;

        let root = unique_temp_root("hard-link-count");
        fs::create_dir_all(&root).unwrap();
        let target = root.join("target");
        let alias = root.join("alias");
        fs::write(&target, b"content").unwrap();
        fs::hard_link(&target, &alias).unwrap();

        let target_file = fs::File::open(&target).unwrap();

        assert_eq!(hard_link_count(&target_file).unwrap(), 2);

        drop(target_file);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn portable_permissions_round_trip_mode_0600() {
        use super::{portable_permissions, restrict_stage_to_owner};
        use std::os::unix::fs::PermissionsExt;

        let root = unique_temp_root("portable-permissions");
        fs::create_dir_all(&root).unwrap();
        let source = root.join("source");
        let staged = root.join("staged");
        fs::write(&source, b"source").unwrap();
        fs::set_permissions(&source, fs::Permissions::from_mode(0o600)).unwrap();
        let expected = portable_permissions(&fs::metadata(&source).unwrap());
        let staged_file = fs::File::create(&staged).unwrap();

        assert!(!expected.readonly());
        restrict_stage_to_owner(&staged_file).unwrap();
        expected.apply_to(&staged_file).unwrap();
        let staged_metadata = staged_file.metadata().unwrap();

        assert!(expected.matches(&staged_metadata));
        assert_eq!(staged_metadata.permissions().mode() & 0o7777, 0o600);

        drop(staged_file);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn retained_directory_enumeration_applies_cap_and_checkpoint_incrementally() {
        use super::{open_directory_nofollow, read_directory_names_bounded};

        let root = unique_temp_root("bounded-retained-enumeration");
        fs::create_dir_all(&root).unwrap();
        for index in 0..64 {
            fs::write(root.join(format!("{index:02}.xml")), b"x").unwrap();
        }
        let directory = open_directory_nofollow(&root).unwrap();

        let capped = read_directory_names_bounded(&directory, 3, || Ok(()));
        assert_eq!(capped.unwrap_err().kind(), io::ErrorKind::FileTooLarge);

        let checkpoints = std::cell::Cell::new(0usize);
        let cancelled = read_directory_names_bounded(&directory, 64, || {
            let next = checkpoints.get() + 1;
            checkpoints.set(next);
            if next == 5 {
                Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"))
            } else {
                Ok(())
            }
        });
        assert_eq!(cancelled.unwrap_err().kind(), io::ErrorKind::Interrupted);
        assert_eq!(checkpoints.get(), 5);

        drop(directory);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn secure_tree_typed_reopen_provides_directory_listing_and_file_bytes() {
        use super::{
            open_any_child_nofollow, open_child_for_secure_tree_use, open_directory_nofollow,
            read_directory_names_bounded, OpenedChildKind,
        };
        use std::io::Read;

        let root = unique_temp_root("secure-tree-typed-reopen");
        fs::create_dir_all(root.join("nested")).unwrap();
        fs::write(root.join("nested/inside.xml"), b"inside").unwrap();
        fs::write(root.join("payload.xml"), b"payload").unwrap();
        let directory = open_directory_nofollow(&root).unwrap();

        let (nested_anchor, nested_kind) =
            open_any_child_nofollow(&directory, std::ffi::OsStr::new("nested")).unwrap();
        assert_eq!(nested_kind, OpenedChildKind::Directory);
        let nested = open_child_for_secure_tree_use(
            &directory,
            std::ffi::OsStr::new("nested"),
            nested_anchor,
            nested_kind,
        )
        .unwrap();
        assert_eq!(
            read_directory_names_bounded(&nested, 2, || Ok(())).unwrap(),
            [std::ffi::OsString::from("inside.xml")]
        );

        let (file_anchor, file_kind) =
            open_any_child_nofollow(&directory, std::ffi::OsStr::new("payload.xml")).unwrap();
        assert_eq!(file_kind, OpenedChildKind::RegularFile);
        let mut file = open_child_for_secure_tree_use(
            &directory,
            std::ffi::OsStr::new("payload.xml"),
            file_anchor,
            file_kind,
        )
        .unwrap();
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"payload");

        drop(file);
        drop(nested);
        drop(directory);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn portable_permissions_restore_mode_after_stage_restriction() {
        use super::{portable_permissions, restrict_stage_to_owner};
        use std::os::unix::fs::PermissionsExt;

        let root = unique_temp_root("portable-permissions-restore");
        fs::create_dir_all(&root).unwrap();
        let source = root.join("source");
        let staged = root.join("staged");
        fs::write(&source, b"source").unwrap();
        fs::set_permissions(&source, fs::Permissions::from_mode(0o640)).unwrap();
        let expected = portable_permissions(&fs::metadata(&source).unwrap());
        let staged_file = fs::File::create(&staged).unwrap();

        restrict_stage_to_owner(&staged_file).unwrap();
        assert_eq!(
            staged_file.metadata().unwrap().permissions().mode() & 0o7777,
            0o600
        );

        expected.apply_to(&staged_file).unwrap();
        let staged_metadata = staged_file.metadata().unwrap();

        assert!(expected.matches(&staged_metadata));
        assert_eq!(staged_metadata.permissions().mode() & 0o7777, 0o640);

        drop(staged_file);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn lock_identity_follows_host_case_policy() {
        let identity = path_lock_identity_text("/Workspace/Configuration.xml");
        if cfg!(any(windows, target_os = "macos")) {
            assert_eq!(identity, "/workspace/configuration.xml");
        } else {
            assert_eq!(identity, "/Workspace/Configuration.xml");
        }
    }

    #[test]
    fn provider_state_identity_preserves_case_except_on_windows() {
        let identity = provider_state_path_identity(Path::new("/Workspace/Configuration.xml"));
        if cfg!(windows) {
            assert_eq!(identity, b"/workspace/configuration.xml");
        } else {
            assert_eq!(identity, b"/Workspace/Configuration.xml");
        }
    }

    #[test]
    fn containment_prefix_follows_host_case_policy() {
        let matches = path_starts_with_host_root(
            Path::new("/WORKSPACE/src/Module.bsl"),
            Path::new("/workspace"),
        );
        if cfg!(windows) {
            assert!(matches);
        } else {
            assert!(!matches);
        }
    }

    #[cfg(unix)]
    #[test]
    fn workspace_policy_rejects_lexically_external_symlink_into_workspace() {
        use crate::domain::workspace::WorkspaceContext;
        use crate::infrastructure::path_policy::WorkspacePathPolicy;
        use std::os::unix::fs::symlink;

        let temp = unique_temp_root("path-policy-inbound-link");
        let workspace = temp.join("workspace");
        let outside = temp.join("outside");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(workspace.join("Configuration.xml"), "<MetaDataObject/>").unwrap();
        symlink(&workspace, outside.join("workspace-alias")).unwrap();
        let context = WorkspaceContext {
            cwd: workspace.clone(),
            workspace_root: workspace.clone(),
            cache_root: workspace.join(".build").join("unica"),
            workspace_epoch: 1,
        };
        let policy = WorkspacePathPolicy::new(&context);

        let error = policy
            .resolve_write(outside.join("workspace-alias/Configuration.xml"))
            .unwrap_err();

        assert!(error.contains("outside workspace root"));
        fs::remove_dir_all(temp).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn workspace_policy_rejects_lexically_internal_symlink_outside_workspace() {
        use crate::domain::workspace::WorkspaceContext;
        use crate::infrastructure::path_policy::WorkspacePathPolicy;
        use std::os::unix::fs::symlink;

        let temp = unique_temp_root("path-policy-outbound-link");
        let workspace = temp.join("workspace");
        let outside = temp.join("outside");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("Configuration.xml"), "<MetaDataObject/>").unwrap();
        symlink(&outside, workspace.join("outside-alias")).unwrap();
        let context = WorkspaceContext {
            cwd: workspace.clone(),
            workspace_root: workspace.clone(),
            cache_root: workspace.join(".build").join("unica"),
            workspace_epoch: 1,
        };
        let policy = WorkspacePathPolicy::new(&context);

        let error = policy
            .resolve_write(workspace.join("outside-alias/Configuration.xml"))
            .unwrap_err();

        assert!(error.contains("outside workspace root"));
        fs::remove_dir_all(temp).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn workspace_policy_accepts_normalized_child_of_verbatim_workspace_root() {
        use crate::domain::workspace::WorkspaceContext;
        use crate::infrastructure::path_policy::WorkspacePathPolicy;
        use crate::infrastructure::source_roots::normalize_path_identity;

        let regular_root = unique_temp_root("path-policy-verbatim");
        let child = regular_root.join("src/CommonModules/Example.xml");
        fs::create_dir_all(child.parent().unwrap()).unwrap();
        fs::write(&child, "<MetaDataObject/>").unwrap();
        let verbatim_root = PathBuf::from(format!(r"\\?\{}", regular_root.display()));
        let context = WorkspaceContext {
            cwd: verbatim_root.clone(),
            workspace_root: verbatim_root.clone(),
            cache_root: verbatim_root.join(".build").join("unica"),
            workspace_epoch: 1,
        };
        let policy = WorkspacePathPolicy::new(&context);
        let normalized_child = normalize_path_identity(&child).unwrap();

        assert_eq!(
            policy.resolve_write(normalized_child.clone()).unwrap(),
            normalized_child
        );

        fs::remove_dir_all(regular_root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn containment_prefix_follows_windows_case_policy() {
        assert!(path_starts_with_host_root(
            Path::new(r"C:\WORKSPACE\src\Module.bsl"),
            Path::new(r"c:\workspace")
        ));
    }

    #[cfg(windows)]
    #[test]
    fn secure_windows_directory_open_rejects_relative_path() {
        let error = super::open_absolute_directory_path_nofollow(Path::new("relative"))
            .expect_err("relative path must not be converted through the process cwd");

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains("must be absolute"));
    }

    #[cfg(windows)]
    #[test]
    fn extended_length_unc_prefix_is_stripped_without_filesystem_access() {
        use std::path::PathBuf;

        let extended = PathBuf::from(r"\\?\UNC\server\share\source");

        assert_eq!(
            PathBuf::from(r"\\server\share\source"),
            strip_windows_extended_length_prefix(&extended)
        );
    }

    #[cfg(windows)]
    #[test]
    fn raw_windows_move_primitives_support_extended_length_paths() {
        use super::{rename_no_replace, replace_file_atomically};

        let base = unique_temp_root("long-moves");
        let mut root = base.clone();
        while root.display().to_string().len() < 270 {
            root.push("long-path-segment");
        }
        fs::create_dir_all(&root).unwrap();

        let replacement_stage = root.join("replacement-stage");
        let replacement_target = root.join("replacement-target");
        fs::write(&replacement_stage, b"replacement").unwrap();
        fs::write(&replacement_target, b"original").unwrap();
        replace_file_atomically(&replacement_stage, &replacement_target).unwrap();
        assert_eq!(fs::read(&replacement_target).unwrap(), b"replacement");
        assert!(!replacement_stage.exists());

        let move_source = root.join("move-source");
        let move_target = root.join("move-target");
        fs::write(&move_source, b"moved").unwrap();
        rename_no_replace(&move_source, &move_target).unwrap();
        assert_eq!(fs::read(&move_target).unwrap(), b"moved");
        assert!(!move_source.exists());

        fs::remove_dir_all(base).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn extended_length_and_regular_paths_have_same_identity() {
        use crate::infrastructure::source_roots::normalize_path_identity;
        use std::path::PathBuf;
        use std::time::{SystemTime, UNIX_EPOCH};

        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "unica-path-identity-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let regular = normalize_path_identity(&root).unwrap();
        let extended = PathBuf::from(format!(r"\\?\{}", root.display()));

        assert_eq!(regular, normalize_path_identity(&extended).unwrap());

        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(target_vendor = "apple")]
    #[test]
    fn filesystem_case_policy_does_not_follow_a_linked_directory() {
        use std::os::unix::fs::symlink;

        let root = unique_temp_root("case-policy-linked-directory");
        let target = root.join("target");
        let linked = root.join("linked");
        fs::create_dir_all(&target).unwrap();
        symlink(&target, &linked).unwrap();

        let result = super::host_filesystem_case_sensitive(&linked);

        assert!(
            result.is_err(),
            "linked directory must fail closed: {result:?}"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(target_vendor = "apple")]
    #[test]
    fn apple_caseless_child_identity_uses_the_filesystem_object() {
        let root = unique_temp_root("apple-caseless-child-identity");
        fs::create_dir_all(&root).unwrap();
        let root_identity = fs::canonicalize(&root).unwrap();
        if super::host_filesystem_case_sensitive(&root_identity).unwrap() {
            let _ = fs::remove_dir_all(root);
            return;
        }
        fs::create_dir(root_identity.join("ß")).unwrap();

        assert!(super::host_directory_child_names_equal(
            &root_identity,
            std::ffi::OsStr::new("ß"),
            std::ffi::OsStr::new("ẞ"),
            false,
        )
        .unwrap());

        let _ = fs::remove_dir_all(root);
    }

    #[cfg(target_vendor = "apple")]
    #[test]
    fn apple_missing_unicode_alias_is_not_approximated_in_userspace() {
        let error = super::host_path_components_equal(
            std::ffi::OsStr::new("é"),
            std::ffi::OsStr::new("e\u{301}"),
            true,
        )
        .expect_err("a missing Apple path component has no proven host identity");

        assert_eq!(error.kind(), io::ErrorKind::Unsupported);
    }

    #[test]
    fn linux_filesystem_case_policy_is_mount_aware_and_fails_closed() {
        const EXT4_SUPER_MAGIC: u64 = 0x0000_ef53;
        const XFS_SUPER_MAGIC: u64 = 0x5846_5342;
        const OVERLAYFS_SUPER_MAGIC: u64 = 0x794c_7630;
        const CIFS_SUPER_MAGIC: u64 = 0xff53_4d42;
        const FS_CASEFOLD_FL: u32 = 0x4000_0000;

        assert!(
            super::linux_filesystem_case_sensitive_from_metadata(EXT4_SUPER_MAGIC, Some(0),)
                .unwrap()
        );
        let casefold_error = super::linux_filesystem_case_sensitive_from_metadata(
            EXT4_SUPER_MAGIC,
            Some(FS_CASEFOLD_FL),
        )
        .expect_err("kernel Unicode casefold semantics must not be approximated");
        assert_eq!(casefold_error.kind(), io::ErrorKind::Unsupported);
        for filesystem_type in [
            XFS_SUPER_MAGIC,
            OVERLAYFS_SUPER_MAGIC,
            CIFS_SUPER_MAGIC,
            0x1234_5678,
        ] {
            let error = super::linux_filesystem_case_sensitive_from_metadata(filesystem_type, None)
                .expect_err("unproven Linux filesystem semantics must fail closed");
            assert_eq!(error.kind(), io::ErrorKind::Unsupported);
        }
    }

    #[cfg(all(not(target_vendor = "apple"), not(windows)))]
    #[test]
    fn injected_case_insensitive_component_identity_collapses_final_sigma() {
        assert!(super::host_path_components_equal(
            std::ffi::OsStr::new("σ"),
            std::ffi::OsStr::new("ς"),
            false,
        )
        .unwrap());
        assert!(!super::host_path_components_equal(
            std::ffi::OsStr::new("ß"),
            std::ffi::OsStr::new("ss"),
            false,
        )
        .unwrap());
    }

    #[cfg(windows)]
    #[test]
    fn windows_component_identity_uses_compare_string_ordinal() {
        assert!(!super::host_path_components_equal(
            std::ffi::OsStr::new("σ"),
            std::ffi::OsStr::new("ς"),
            false,
        )
        .unwrap());
        assert!(super::host_path_components_equal(
            std::ffi::OsStr::new("a"),
            std::ffi::OsStr::new("A"),
            false,
        )
        .unwrap());
    }

    #[cfg(windows)]
    #[test]
    fn preserves_non_drive_verbatim_path_namespaces() {
        use crate::infrastructure::source_roots::normalize_path_identity;
        use std::path::PathBuf;

        let verbatim = PathBuf::from(r"\\?\Volume{01234567-89ab-cdef-0123-456789abcdef}\source");

        assert_eq!(verbatim, normalize_path_identity(&verbatim).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn stable_path_identity_bytes_are_explicit_for_non_utf8_unix_paths() {
        use std::os::unix::ffi::OsStringExt;

        assert_eq!(
            stable_path_identity_bytes(Path::new("/workspace")).unwrap(),
            b"/workspace"
        );
        let path = PathBuf::from(std::ffi::OsString::from_vec(
            b"/workspace/source-\x80".to_vec(),
        ));
        let mut expected = b"\xffunica-path-unix-v1\0".to_vec();
        expected.extend_from_slice(b"/workspace/source-\x80");

        assert_eq!(stable_path_identity_bytes(&path).unwrap(), expected);
    }

    #[cfg(unix)]
    #[test]
    fn source_root_policy_rejects_parent_traversal_after_directory_symlink() {
        use crate::infrastructure::source_roots::normalize_contained_source_root;
        use std::os::unix::fs::symlink;
        use std::time::{SystemTime, UNIX_EPOCH};

        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let workspace = std::env::temp_dir().join(format!(
            "unica-source-roots-parent-workspace-{}-{nanos}",
            std::process::id()
        ));
        let outside = std::env::temp_dir().join(format!(
            "unica-source-roots-parent-outside-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        symlink(&outside, workspace.join("external")).unwrap();

        let error =
            normalize_contained_source_root(&workspace, workspace.join("external/../escaped-new"))
                .unwrap_err();

        assert!(error.contains("sourceDir must be inside workspace root"));
        let _ = std::fs::remove_dir_all(workspace);
        let _ = std::fs::remove_dir_all(outside);
    }
}
