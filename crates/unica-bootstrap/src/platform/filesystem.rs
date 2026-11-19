/// Whether a `file://` URI with a host names a native directory: on Windows
/// the host is the server of a UNC share (`\\server\share`).
pub(crate) const fn file_uri_host_names_share() -> bool {
    cfg!(windows)
}

/// A directory link for tests: a junction on Windows, which needs no
/// privilege there, a symbolic link elsewhere.
#[cfg(test)]
pub(crate) fn link_directory_for_test(link: &std::path::Path, target: &std::path::Path) {
    #[cfg(windows)]
    {
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).unwrap();
}
