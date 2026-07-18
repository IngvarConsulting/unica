use std::fs::{File, Metadata, OpenOptions};
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

pub(crate) fn canonical_workspace(root: &Path) -> Result<PathBuf, String> {
    let metadata = std::fs::metadata(root)
        .map_err(|error| format!("workspace_root_unavailable: {}: {error}", root.display()))?;
    if !metadata.is_dir() {
        return Err(format!("workspace_root_not_directory: {}", root.display()));
    }
    root.canonicalize()
        .map_err(|error| format!("workspace_root_unavailable: {}: {error}", root.display()))
}

pub(crate) fn validate_configured_relative_path(raw: &str, field: &str) -> Result<(), String> {
    if raw.is_empty() {
        return Err(format!(
            "empty_configured_path: `{field}` must not be empty"
        ));
    }
    if raw.starts_with('/') || looks_like_windows_absolute(raw) {
        return Err(format!("absolute_source_root: `{field}` must be relative"));
    }
    if raw.contains('\\') || raw.chars().any(char::is_control) {
        return Err(format!(
            "invalid_configured_path: `{field}` contains unsafe bytes"
        ));
    }
    let components = raw.split('/').collect::<Vec<_>>();
    if components.iter().any(|component| component.is_empty()) {
        return Err(format!(
            "empty_path_component: `{field}` contains an empty component"
        ));
    }
    if components
        .iter()
        .any(|component| matches!(*component, ".."))
    {
        return Err(format!("path_traversal: `{field}` contains `..`"));
    }
    if raw != "." && components.contains(&".") {
        return Err(format!("embedded_current_dir: `{field}` contains `.`"));
    }
    Ok(())
}

pub(crate) fn normalize_relative(base: &str, path: &str) -> Result<String, String> {
    validate_configured_relative_path(base, "basePath")?;
    validate_configured_relative_path(path, "path")?;
    let mut parts = Vec::new();
    if base != "." {
        parts.extend(base.split('/'));
    }
    if path != "." {
        parts.extend(path.split('/'));
    }
    if parts.is_empty() {
        Ok(".".into())
    } else {
        Ok(parts.join("/"))
    }
}

pub(crate) fn resolve_contained_directory(
    canonical_workspace: &Path,
    relative: &str,
) -> Result<PathBuf, String> {
    validate_configured_relative_path(relative, "source root")?;
    let root = if relative == "." {
        canonical_workspace.to_path_buf()
    } else {
        canonical_workspace.join(relative)
    };
    reject_link_components(canonical_workspace, &root)?;
    let metadata = std::fs::symlink_metadata(&root)
        .map_err(|error| format!("source_root_unavailable: {}: {error}", root.display()))?;
    if metadata_is_link_or_reparse_point(&metadata) {
        return Err(format!("source_root_symlink: {}", root.display()));
    }
    if !metadata.is_dir() {
        return Err(format!("source_root_not_directory: {}", root.display()));
    }
    let canonical = root
        .canonicalize()
        .map_err(|error| format!("source_root_unavailable: {}: {error}", root.display()))?;
    if !canonical.starts_with(canonical_workspace) {
        return Err(format!("source_root_escape: {}", root.display()));
    }
    Ok(canonical)
}

pub(crate) fn reject_link_components(workspace: &Path, target: &Path) -> Result<(), String> {
    let relative = target
        .strip_prefix(workspace)
        .map_err(|_| format!("path_escape: {}", target.display()))?;
    let mut current = workspace.to_path_buf();
    for component in relative.components() {
        if !matches!(component, Component::Normal(_)) {
            return Err(format!("invalid_path_component: {}", target.display()));
        }
        current.push(component.as_os_str());
        let metadata = std::fs::symlink_metadata(&current)
            .map_err(|error| format!("path_unavailable: {}: {error}", current.display()))?;
        if metadata_is_link_or_reparse_point(&metadata) {
            return Err(format!("symlink_or_reparse_escape: {}", current.display()));
        }
    }
    Ok(())
}

pub(crate) struct ContainedOpen {
    file: File,
    #[cfg(windows)]
    parents: Vec<WindowsHandleGuard>,
    #[cfg(windows)]
    leaf_snapshot: WindowsHandleSnapshot,
}

impl ContainedOpen {
    pub(crate) fn file(&self) -> &File {
        &self.file
    }

    pub(crate) fn file_mut(&mut self) -> &mut File {
        &mut self.file
    }

    pub(crate) fn validate_after_read(&self) -> Result<(), String> {
        #[cfg(windows)]
        {
            for parent in &self.parents {
                validate_windows_handle(&parent.file, &parent.snapshot)?;
            }
            validate_windows_handle(&self.file, &self.leaf_snapshot)?;
        }
        Ok(())
    }
}

#[cfg(windows)]
struct WindowsHandleGuard {
    file: File,
    snapshot: WindowsHandleSnapshot,
}

#[cfg(windows)]
struct WindowsHandleSnapshot {
    expected_final: String,
    identity: FileIdentity,
    directory: bool,
}

pub(crate) fn open_no_follow(workspace: &Path, path: &Path) -> Result<ContainedOpen, String> {
    #[cfg(unix)]
    {
        open_no_follow_unix(workspace, path)
    }
    #[cfg(windows)]
    {
        open_no_follow_windows(workspace, path)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = workspace;
        let _ = path;
        Err("file_identity_unavailable: contained open is unsupported".into())
    }
}

#[cfg(unix)]
fn open_no_follow_unix(workspace: &Path, path: &Path) -> Result<ContainedOpen, String> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::OpenOptionsExt;

    let relative = path
        .strip_prefix(workspace)
        .map_err(|_| format!("path_escape: {}", path.display()))?;
    let components = relative
        .components()
        .map(|component| match component {
            Component::Normal(value) => CString::new(value.as_bytes())
                .map_err(|_| format!("invalid_material_path: {}", path.display())),
            _ => Err(format!("invalid_material_path: {}", path.display())),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let (file_name, parents) = components
        .split_last()
        .ok_or_else(|| "material path must name a file".to_string())?;
    let mut root_options = OpenOptions::new();
    root_options
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC);
    let mut directory = root_options.open(workspace).map_err(|error| {
        format!(
            "workspace_root_unavailable: {}: {error}",
            workspace.display()
        )
    })?;
    for component in parents {
        let descriptor = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                component.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if descriptor < 0 {
            return Err(format!(
                "material_subtree_unreadable: {}: {}",
                path.display(),
                std::io::Error::last_os_error()
            ));
        }
        directory = unsafe { File::from_raw_fd(descriptor) };
    }
    let descriptor = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            file_name.as_ptr(),
            libc::O_RDONLY | libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if descriptor < 0 {
        return Err(format!(
            "material_file_unreadable: {}: {}",
            path.display(),
            std::io::Error::last_os_error()
        ));
    }
    Ok(ContainedOpen {
        file: unsafe { File::from_raw_fd(descriptor) },
    })
}

#[cfg(windows)]
fn open_no_follow_windows(workspace: &Path, path: &Path) -> Result<ContainedOpen, String> {
    use std::os::windows::fs::OpenOptionsExt;

    let relative = path
        .strip_prefix(workspace)
        .map_err(|_| format!("path_escape: {}", path.display()))?;
    let components = relative
        .components()
        .map(|component| match component {
            Component::Normal(value) => Ok(value.to_os_string()),
            _ => Err(format!("invalid_material_path: {}", path.display())),
        })
        .collect::<Result<Vec<_>, _>>()?;
    if components.is_empty() {
        return Err("material path must name a file".into());
    }

    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE,
        FILE_SHARE_READ, FILE_SHARE_WRITE,
    };

    let mut root_options = OpenOptions::new();
    root_options
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS);
    let root = root_options.open(workspace).map_err(|error| {
        format!(
            "workspace_root_unavailable: {}: {error}",
            workspace.display()
        )
    })?;
    let root_snapshot = snapshot_windows_handle(&root, true)?;
    let root_volume = windows_identity_volume(&root_snapshot.identity);
    let mut parents = vec![WindowsHandleGuard {
        file: root,
        snapshot: root_snapshot,
    }];
    let mut expected_final = parents[0].snapshot.expected_final.clone();
    let mut leaf = None;
    for (index, component) in components.iter().enumerate() {
        let is_leaf = index + 1 == components.len();
        let parent = &parents
            .last()
            .ok_or_else(|| "file_identity_unavailable: missing parent handle".to_string())?
            .file;
        let handle = open_windows_component(parent, component, is_leaf)?;
        let component = windows_component_string(component)?;
        expected_final = format!("{}\\{component}", expected_final.trim_end_matches('\\'));
        let mut snapshot = snapshot_windows_handle(&handle, !is_leaf)?;
        if windows_identity_volume(&snapshot.identity) != root_volume
            || !windows_paths_equal(&snapshot.expected_final, &expected_final)?
        {
            return Err(format!("symlink_or_reparse_escape: {expected_final}"));
        }
        snapshot.expected_final = expected_final.clone();
        if is_leaf {
            leaf = Some((handle, snapshot));
        } else {
            parents.push(WindowsHandleGuard {
                file: handle,
                snapshot,
            });
        }
    }
    let (file, leaf_snapshot) = leaf.ok_or_else(|| "material path must name a file".to_string())?;
    Ok(ContainedOpen {
        file,
        parents,
        leaf_snapshot,
    })
}

#[cfg(windows)]
fn open_windows_component(
    parent: &File,
    component: &std::ffi::OsStr,
    leaf: bool,
) -> Result<File, String> {
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle};
    use windows_sys::Wdk::Foundation::OBJECT_ATTRIBUTES;
    use windows_sys::Wdk::Storage::FileSystem::{
        NtCreateFile, FILE_DIRECTORY_FILE, FILE_NON_DIRECTORY_FILE, FILE_OPEN,
        FILE_OPEN_REPARSE_POINT, FILE_SYNCHRONOUS_IO_NONALERT,
    };
    use windows_sys::Win32::Foundation::{HANDLE, UNICODE_STRING};
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, FILE_READ_DATA, FILE_SHARE_DELETE,
        FILE_SHARE_READ, FILE_SHARE_WRITE, SYNCHRONIZE,
    };
    use windows_sys::Win32::System::Kernel::OBJ_CASE_INSENSITIVE;
    use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;

    let mut name = component.encode_wide().collect::<Vec<_>>();
    let byte_length = name
        .len()
        .checked_mul(std::mem::size_of::<u16>())
        .and_then(|length| u16::try_from(length).ok())
        .ok_or_else(|| "invalid_material_path: Windows component is too long".to_string())?;
    if name.is_empty() || name.contains(&0) {
        return Err("invalid_material_path: invalid Windows component".into());
    }
    let unicode = UNICODE_STRING {
        Length: byte_length,
        MaximumLength: byte_length,
        Buffer: name.as_mut_ptr(),
    };
    let attributes = OBJECT_ATTRIBUTES {
        Length: std::mem::size_of::<OBJECT_ATTRIBUTES>() as u32,
        RootDirectory: parent.as_raw_handle() as HANDLE,
        ObjectName: &unicode,
        Attributes: OBJ_CASE_INSENSITIVE as u32,
        SecurityDescriptor: std::ptr::null(),
        SecurityQualityOfService: std::ptr::null(),
    };
    let mut raw: HANDLE = std::ptr::null_mut();
    let mut io_status: IO_STATUS_BLOCK = unsafe { std::mem::zeroed() };
    let desired_access = FILE_READ_ATTRIBUTES
        | SYNCHRONIZE
        | if leaf {
            FILE_READ_DATA
        } else {
            FILE_LIST_DIRECTORY
        };
    let create_options = FILE_OPEN_REPARSE_POINT
        | FILE_SYNCHRONOUS_IO_NONALERT
        | if leaf {
            FILE_NON_DIRECTORY_FILE
        } else {
            FILE_DIRECTORY_FILE
        };
    let status = unsafe {
        NtCreateFile(
            &mut raw,
            desired_access,
            &attributes,
            &mut io_status,
            std::ptr::null(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            FILE_OPEN,
            create_options,
            std::ptr::null(),
            0,
        )
    };
    if status < 0 || raw.is_null() {
        return Err(format!(
            "material_file_unreadable: NtCreateFile failed with NTSTATUS 0x{:08x}",
            status as u32
        ));
    }
    Ok(unsafe { File::from_raw_handle(raw as _) })
}

#[cfg(windows)]
fn windows_component_string(component: &std::ffi::OsStr) -> Result<String, String> {
    use std::os::windows::ffi::OsStrExt;
    String::from_utf16(&component.encode_wide().collect::<Vec<_>>())
        .map_err(|_| "invalid_material_path: Windows component is not valid UTF-16".into())
}

#[cfg(windows)]
fn windows_final_path(file: &File) -> Result<String, String> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::GetFinalPathNameByHandleW;

    let required =
        unsafe { GetFinalPathNameByHandleW(file.as_raw_handle() as _, std::ptr::null_mut(), 0, 0) };
    if required == 0 {
        return Err("file_identity_unavailable: final handle path length".into());
    }
    let mut buffer = vec![0u16; required as usize + 1];
    let length = unsafe {
        GetFinalPathNameByHandleW(
            file.as_raw_handle() as _,
            buffer.as_mut_ptr(),
            u32::try_from(buffer.len())
                .map_err(|_| "file_identity_unavailable: final handle path too long")?,
            0,
        )
    };
    if length == 0 || length as usize >= buffer.len() {
        return Err("file_identity_unavailable: final handle path".into());
    }
    String::from_utf16(&buffer[..length as usize])
        .map_err(|_| "file_identity_unavailable: final handle path is not UTF-16".into())
}

#[cfg(windows)]
fn windows_paths_equal(left: &str, right: &str) -> Result<bool, String> {
    use windows_sys::Win32::Globalization::{CompareStringOrdinal, CSTR_EQUAL};

    let left = left.encode_utf16().collect::<Vec<_>>();
    let right = right.encode_utf16().collect::<Vec<_>>();
    let result = unsafe {
        CompareStringOrdinal(
            left.as_ptr(),
            i32::try_from(left.len())
                .map_err(|_| "file_identity_unavailable: final path too long")?,
            right.as_ptr(),
            i32::try_from(right.len())
                .map_err(|_| "file_identity_unavailable: final path too long")?,
            1,
        )
    };
    if result == 0 {
        return Err("file_identity_unavailable: final path comparison failed".into());
    }
    Ok(result == CSTR_EQUAL)
}

#[cfg(windows)]
fn snapshot_windows_handle(file: &File, directory: bool) -> Result<WindowsHandleSnapshot, String> {
    let metadata = file
        .metadata()
        .map_err(|error| format!("file_identity_unavailable: handle metadata: {error}"))?;
    if metadata_is_link_or_reparse_point(&metadata)
        || (directory && !metadata.is_dir())
        || (!directory && !metadata.is_file())
    {
        return Err("symlink_or_reparse_escape: opened handle type".into());
    }
    Ok(WindowsHandleSnapshot {
        expected_final: windows_final_path(file)?,
        identity: windows_file_identity(file)?,
        directory,
    })
}

#[cfg(windows)]
fn validate_windows_handle(file: &File, expected: &WindowsHandleSnapshot) -> Result<(), String> {
    let actual = snapshot_windows_handle(file, expected.directory)?;
    if actual.identity != expected.identity
        || !windows_paths_equal(&actual.expected_final, &expected.expected_final)?
    {
        return Err("source_snapshot_unavailable: contained handle changed after open".into());
    }
    Ok(())
}

#[cfg(windows)]
fn windows_file_identity(file: &File) -> Result<FileIdentity, String> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        FileIdInfo, GetFileInformationByHandleEx, FILE_ID_INFO,
    };

    let mut information: FILE_ID_INFO = unsafe { std::mem::zeroed() };
    let success = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle() as _,
            FileIdInfo,
            (&mut information as *mut FILE_ID_INFO).cast(),
            std::mem::size_of::<FILE_ID_INFO>() as u32,
        )
    };
    if success == 0 {
        return Err("file_identity_unavailable: GetFileInformationByHandleEx failed".into());
    }
    Ok(FileIdentity::Windows {
        volume: information.VolumeSerialNumber,
        id: information.FileId.Identifier,
    })
}

#[cfg(windows)]
fn windows_identity_volume(identity: &FileIdentity) -> u64 {
    let FileIdentity::Windows { volume, .. } = identity;
    *volume
}

pub(crate) fn metadata_is_link_or_reparse_point(metadata: &Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileObservation {
    pub(crate) identity: FileIdentity,
    pub(crate) length: u64,
    pub(crate) modified: Option<SystemTime>,
    pub(crate) platform_metadata: u128,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FileIdentity {
    #[cfg(unix)]
    Unix { device: u64, inode: u64 },
    #[cfg(windows)]
    Windows { volume: u64, id: [u8; 16] },
    #[cfg(not(any(unix, windows)))]
    Unsupported,
}

pub(crate) fn observe_regular_file(
    metadata: &Metadata,
    path: &Path,
) -> Result<FileObservation, String> {
    if metadata_is_link_or_reparse_point(metadata) || !metadata.is_file() {
        return Err(format!("material_file_not_regular: {}", path.display()));
    }
    #[cfg(unix)]
    let (identity, platform_metadata) = {
        use std::os::unix::fs::MetadataExt;
        (
            FileIdentity::Unix {
                device: metadata.dev(),
                inode: metadata.ino(),
            },
            ((metadata.mode() as u128) << 96)
                | ((metadata.ctime() as u64 as u128) << 32)
                | metadata.ctime_nsec() as u64 as u128,
        )
    };
    #[cfg(windows)]
    let (identity, platform_metadata) = {
        return Err(format!(
            "file_identity_unavailable: {}: path metadata has no stable file identity",
            path.display()
        ));
        #[allow(unreachable_code)]
        (
            FileIdentity::Windows {
                volume: 0,
                id: [0; 16],
            },
            0,
        )
    };
    #[cfg(not(any(unix, windows)))]
    let (identity, platform_metadata) = {
        return Err(format!("file_identity_unavailable: {}", path.display()));
        #[allow(unreachable_code)]
        (FileIdentity::Unsupported, 0)
    };
    Ok(FileObservation {
        identity,
        length: metadata.len(),
        modified: metadata.modified().ok(),
        platform_metadata,
    })
}

pub(crate) fn observe_open_file(file: &File, path: &Path) -> Result<FileObservation, String> {
    let metadata = file
        .metadata()
        .map_err(|error| format!("material_file_unreadable: {}: {error}", path.display()))?;
    if metadata_is_link_or_reparse_point(&metadata) || !metadata.is_file() {
        return Err(format!("material_file_not_regular: {}", path.display()));
    }
    #[cfg(unix)]
    {
        observe_regular_file(&metadata, path)
    }
    #[cfg(windows)]
    {
        Ok(FileObservation {
            identity: windows_file_identity(file)?,
            length: metadata.len(),
            modified: metadata.modified().ok(),
            platform_metadata: 0,
        })
    }
    #[cfg(not(any(unix, windows)))]
    {
        Err(format!("file_identity_unavailable: {}", path.display()))
    }
}

pub(crate) fn slash_relative(workspace: &Path, path: &Path) -> Result<String, String> {
    let relative = path
        .strip_prefix(workspace)
        .map_err(|_| format!("path_escape: {}", path.display()))?;
    let mut parts = Vec::new();
    for component in relative.components() {
        let Component::Normal(value) = component else {
            return Err(format!("invalid_path_component: {}", path.display()));
        };
        let value = value
            .to_str()
            .ok_or_else(|| format!("non_utf8_material_path: {}", path.display()))?;
        if value.is_empty() || matches!(value, "." | "..") || value.chars().any(char::is_control) {
            return Err(format!("invalid_material_path: {}", path.display()));
        }
        parts.push(value);
    }
    if parts.is_empty() {
        return Err("material path must name a file".into());
    }
    Ok(parts.join("/"))
}

fn looks_like_windows_absolute(raw: &str) -> bool {
    let bytes = raw.as_bytes();
    (bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':')
        || raw.starts_with("//")
        || raw.starts_with("\\\\")
}
