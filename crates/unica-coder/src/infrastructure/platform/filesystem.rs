use std::fs;
#[cfg(unix)]
use std::fs::File;
use std::io;
use std::path::Path;

#[derive(Debug, Clone)]
pub(crate) struct PortablePermissions {
    permissions: fs::Permissions,
    key: u32,
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

#[cfg(windows)]
pub(crate) fn hard_link_count(file: &fs::File) -> io::Result<u64> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };

    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: the file owns a valid handle and `information` is writable for this call.
    let succeeded = unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut information) };
    if succeeded == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(u64::from(information.nNumberOfLinks))
    }
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn hard_link_count(_file: &fs::File) -> io::Result<u64> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "hard-link count is not available on this host",
    ))
}

pub(crate) fn install_file_no_clobber(source: &Path, target: &Path) -> io::Result<()> {
    fs::hard_link(source, target)
}

#[cfg(windows)]
pub(crate) fn host_path_text(path: String) -> String {
    path.replace('\\', "/")
}

#[cfg(not(windows))]
pub(crate) fn host_path_text(path: String) -> String {
    path
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
    use std::os::windows::ffi::OsStrExt;

    const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(existing: *const u16, replacement: *const u16, flags: u32) -> i32;
    }

    let source = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let target = target
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
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

pub(crate) fn path_lock_identity(path: &Path) -> String {
    path_lock_identity_text(&path.to_string_lossy())
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
    use super::path_lock_identity_text;
    use std::fs;
    use std::io;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[cfg(windows)]
    use super::strip_windows_extended_length_prefix;

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

    #[test]
    fn lock_identity_follows_host_case_policy() {
        let identity = path_lock_identity_text("/Workspace/Configuration.xml");
        if cfg!(any(windows, target_os = "macos")) {
            assert_eq!(identity, "/workspace/configuration.xml");
        } else {
            assert_eq!(identity, "/Workspace/Configuration.xml");
        }
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
