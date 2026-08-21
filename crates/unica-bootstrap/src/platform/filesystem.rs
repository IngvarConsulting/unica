use std::fs;
use std::path::Path;

#[cfg(unix)]
use crate::error::BootstrapError;
use crate::error::Result;

#[cfg(unix)]
pub(crate) fn set_executable(path: &Path, executable: bool) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mode = if executable { 0o755 } else { 0o644 };
    fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    let actual = fs::metadata(path)?.permissions().mode() & 0o111 != 0;
    if actual != executable {
        return Err(BootstrapError::new(format!(
            "runtime file executable mode was not applied: {}",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(not(unix))]
pub(crate) fn set_executable(_path: &Path, _executable: bool) -> Result<()> {
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::set_executable;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn executable_mode_application_is_verified_inside_the_platform_facade() {
        let path = std::env::temp_dir().join(format!(
            "unica-bootstrap-mode-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::write(&path, b"runtime").unwrap();

        set_executable(&path, true).unwrap();
        assert_ne!(fs::metadata(&path).unwrap().permissions().mode() & 0o111, 0);
        set_executable(&path, false).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o111, 0);

        fs::remove_file(path).unwrap();
    }
}
