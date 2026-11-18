//! The host branch classes used by v8 filename and installation policy, and
//! the creation of a file that must not be readable more widely than another.

use std::fs::{File, OpenOptions, Permissions};
use std::io;
use std::path::Path;

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

/// Creates `path` anew for writing. On Unix the file is created with the mode
/// bits of `like`, so a staged copy of a private file is never readable more
/// widely, even before its permissions are set; elsewhere the caller sets the
/// permissions after writing.
pub fn create_new_like(path: &Path, like: Option<&Permissions>) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        if let Some(like) = like {
            options.mode(like.mode() & 0o777);
        }
    }
    #[cfg(not(unix))]
    let _ = like;
    options.open(path)
}
