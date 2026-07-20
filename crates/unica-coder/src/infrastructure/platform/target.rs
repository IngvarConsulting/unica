pub(crate) fn current_target_id() -> Result<&'static str, String> {
    target_id(std::env::consts::OS, std::env::consts::ARCH)
}

fn target_id(os: &str, arch: &str) -> Result<&'static str, String> {
    match (os, arch) {
        ("macos", "aarch64") => Ok("darwin-arm64"),
        ("linux", "x86_64") => Ok("linux-x64"),
        ("windows", "x86_64") => Ok("win-x64"),
        (os, arch) => Err(format!("Unica does not ship binaries for {os}-{arch}.")),
    }
}

#[cfg(test)]
mod tests {
    use super::target_id;

    #[test]
    fn maps_supported_package_targets() {
        assert_eq!(target_id("macos", "aarch64").unwrap(), "darwin-arm64");
        assert_eq!(target_id("linux", "x86_64").unwrap(), "linux-x64");
        assert_eq!(target_id("windows", "x86_64").unwrap(), "win-x64");
    }

    #[test]
    fn preserves_unsupported_host_error() {
        assert_eq!(
            target_id("linux", "aarch64").unwrap_err(),
            "Unica does not ship binaries for linux-aarch64."
        );
    }
}
