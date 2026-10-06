use std::path::Path;

use crate::error::Result;

/// Whether a `file://` URI with a host names a native directory: on Windows
/// the host is the server of a UNC share (`\\server\share`).
pub(crate) const fn file_uri_host_names_share() -> bool {
    cfg!(windows)
}

#[cfg(unix)]
pub(crate) fn set_executable(path: &Path, executable: bool) -> Result<()> {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    let mode = if executable { 0o755 } else { 0o644 };
    fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    Ok(())
}

#[cfg(not(unix))]
pub(crate) fn set_executable(_path: &Path, _executable: bool) -> Result<()> {
    Ok(())
}
