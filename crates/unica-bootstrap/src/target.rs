use std::fmt;

use crate::error::{BootstrapError, Result};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum HostTarget {
    DarwinArm64,
    LinuxX64,
    WinX64,
}

impl HostTarget {
    pub const ALL: [Self; 3] = [Self::DarwinArm64, Self::LinuxX64, Self::WinX64];

    pub fn detect(os: &str, arch: &str) -> Result<Self> {
        let os_lower = os.to_ascii_lowercase();
        let arch_lower = arch.to_ascii_lowercase();
        let target = match (os_lower.as_str(), arch_lower.as_str()) {
            ("darwin" | "macos", "arm64" | "aarch64") => Self::DarwinArm64,
            ("linux", "x86_64" | "amd64") => Self::LinuxX64,
            ("windows", "x86_64" | "amd64") => Self::WinX64,
            (windows, "x86_64" | "amd64")
                if windows.starts_with("mingw")
                    || windows.starts_with("msys")
                    || windows.starts_with("cygwin") =>
            {
                Self::WinX64
            }
            _ => {
                return Err(BootstrapError::new(format!(
                    "unsupported Unica host: {os}-{arch}"
                )))
            }
        };
        Ok(target)
    }

    pub fn current() -> Result<Self> {
        Self::detect(std::env::consts::OS, std::env::consts::ARCH)
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DarwinArm64 => "darwin-arm64",
            Self::LinuxX64 => "linux-x64",
            Self::WinX64 => "win-x64",
        }
    }

    pub const fn target_triple(self) -> &'static str {
        match self {
            Self::DarwinArm64 => "aarch64-apple-darwin",
            Self::LinuxX64 => "x86_64-unknown-linux-gnu",
            Self::WinX64 => "x86_64-pc-windows-msvc",
        }
    }

    pub const fn executable_name(self) -> &'static str {
        match self {
            Self::WinX64 => "unica.exe",
            Self::DarwinArm64 | Self::LinuxX64 => "unica",
        }
    }
}

impl fmt::Display for HostTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}
