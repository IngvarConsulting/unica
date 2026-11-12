//! The host branch classes used by v8 filename and installation policy.

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
