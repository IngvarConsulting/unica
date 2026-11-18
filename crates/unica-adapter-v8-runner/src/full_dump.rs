//! Platform installation roots and the pinned full-dump flag.

use std::path::PathBuf;
use unica_workspace::host_platform::{current_host_platform, HostPlatform};

/// The one enumeration of platform installation roots in the project — reused
/// by `documentation_registry`'s installation resolver in
/// `infrastructure::application_ports` so a second such list never appears.
pub fn default_platform_roots() -> Vec<PathBuf> {
    match current_host_platform() {
        HostPlatform::Windows => vec![
            PathBuf::from(r"C:\Program Files\1cv8"),
            PathBuf::from(r"C:\Program Files (x86)\1cv8"),
        ],
        HostPlatform::Linux => vec![
            PathBuf::from("/opt/1cv8/x86_64"),
            PathBuf::from("/opt/1cv8/i386"),
            PathBuf::from("/usr/local/1cv8"),
        ],
        HostPlatform::Other => vec![PathBuf::from("/opt/1cv8")],
    }
}

/// The v8-runner flag that makes `dump` (`pull`) a full dump replacing the
/// source tree; the pinned runner refuses the former `--mode full`.
pub const FULL_DUMP_FLAG: &str = "--force";
