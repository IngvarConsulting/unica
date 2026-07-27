use std::{io, path::Path};

pub(crate) use super::filesystem::{create_dir_symlink_for_test, create_file_symlink_for_test};

pub(crate) fn normalize_path_text_for_test(value: &str) -> String {
    value.replace('\\', "/")
}

pub(crate) fn path_text_for_test(path: &Path) -> String {
    normalize_path_text_for_test(&path.display().to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FileLinkFixtureOutcome {
    Created,
    Unsupported,
    WindowsPrivilegeUnavailable,
}

pub(crate) fn create_file_link_fixture_for_test(
    source: impl AsRef<Path>,
    target: impl AsRef<Path>,
) -> io::Result<FileLinkFixtureOutcome> {
    match create_file_symlink_for_test(source, target) {
        Some(Ok(())) => Ok(FileLinkFixtureOutcome::Created),
        Some(Err(error)) if windows_symlink_privilege_unavailable(&error) => {
            Ok(FileLinkFixtureOutcome::WindowsPrivilegeUnavailable)
        }
        Some(Err(error)) => Err(error),
        None => Ok(FileLinkFixtureOutcome::Unsupported),
    }
}

#[cfg(windows)]
fn windows_symlink_privilege_unavailable(error: &io::Error) -> bool {
    error.raw_os_error() == Some(1314)
}

#[cfg(not(windows))]
fn windows_symlink_privilege_unavailable(_error: &io::Error) -> bool {
    false
}

#[cfg(unix)]
pub(crate) fn set_unix_mode_for_test(path: &Path, mode: u32) -> io::Result<bool> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
    Ok(true)
}

#[cfg(not(unix))]
pub(crate) fn set_unix_mode_for_test(_path: &Path, _mode: u32) -> io::Result<bool> {
    Ok(false)
}

#[cfg(unix)]
pub(crate) fn unix_mode_for_test(path: &Path) -> io::Result<Option<u32>> {
    use std::os::unix::fs::PermissionsExt;
    Ok(Some(std::fs::metadata(path)?.permissions().mode() & 0o7777))
}

#[cfg(not(unix))]
pub(crate) fn unix_mode_for_test(_path: &Path) -> io::Result<Option<u32>> {
    Ok(None)
}
