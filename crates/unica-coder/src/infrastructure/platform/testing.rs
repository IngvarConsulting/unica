use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

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
    classify_file_link_fixture_result(create_file_symlink_for_test(source, target))
}

fn classify_file_link_fixture_result(
    result: Option<io::Result<()>>,
) -> io::Result<FileLinkFixtureOutcome> {
    match result {
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
    const ERROR_PRIVILEGE_NOT_HELD: i32 = 1314;

    error.raw_os_error() == Some(ERROR_PRIVILEGE_NOT_HELD)
}

#[cfg(not(windows))]
fn windows_symlink_privilege_unavailable(_error: &io::Error) -> bool {
    false
}

#[cfg(unix)]
pub(crate) fn set_unix_mode_for_test(path: &Path, mode: u32) -> io::Result<bool> {
    use std::fs::Permissions;
    use std::os::unix::fs::PermissionsExt;

    std::fs::set_permissions(path, Permissions::from_mode(mode))?;
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

pub(crate) struct TestCommand {
    pub(crate) program: PathBuf,
    pub(crate) args: Vec<String>,
}

#[cfg(windows)]
pub(crate) fn command_writing_stdout(text: &str) -> TestCommand {
    TestCommand {
        program: PathBuf::from("powershell"),
        args: vec![
            "-NoProfile".to_string(),
            "-Command".to_string(),
            format!("[Console]::Write('{}')", text.replace('\'', "''")),
        ],
    }
}

#[cfg(not(windows))]
pub(crate) fn command_writing_stdout(text: &str) -> TestCommand {
    TestCommand {
        program: PathBuf::from("sh"),
        args: vec![
            "-c".to_string(),
            format!("printf '%s' '{}'", text.replace('\'', "'\\''")),
        ],
    }
}

#[cfg(windows)]
pub(crate) fn long_running_command() -> TestCommand {
    TestCommand {
        program: PathBuf::from("powershell"),
        args: vec![
            "-NoProfile".to_string(),
            "-Command".to_string(),
            "Start-Sleep -Seconds 10".to_string(),
        ],
    }
}

#[cfg(not(windows))]
pub(crate) fn long_running_command() -> TestCommand {
    TestCommand {
        program: PathBuf::from("sh"),
        args: vec!["-c".to_string(), "sleep 10".to_string()],
    }
}

#[cfg(windows)]
pub(crate) fn line_printing_command(sleep_first: bool, lines: &[String]) -> TestCommand {
    let mut script = String::new();
    if sleep_first {
        script.push_str("Start-Sleep -Milliseconds 20; ");
    }
    for line in lines {
        script.push_str("[Console]::Out.WriteLine('");
        script.push_str(&line.replace('\'', "''"));
        script.push_str("'); ");
    }
    TestCommand {
        program: PathBuf::from("powershell"),
        args: vec!["-NoProfile".to_string(), "-Command".to_string(), script],
    }
}

#[cfg(not(windows))]
pub(crate) fn line_printing_command(sleep_first: bool, lines: &[String]) -> TestCommand {
    let mut script = String::new();
    if sleep_first {
        script.push_str("sleep 0.01; ");
    }
    script.push_str("printf '%s\\n'");
    for line in lines {
        script.push_str(" '");
        script.push_str(&line.replace('\'', "'\\''"));
        script.push('\'');
    }
    TestCommand {
        program: PathBuf::from("/bin/sh"),
        args: vec!["-c".to_string(), script],
    }
}

#[cfg(windows)]
pub(crate) fn fixture_executable_path(directory: &Path, stem: &str) -> PathBuf {
    directory.join(format!("{stem}.exe"))
}

#[cfg(not(windows))]
pub(crate) fn fixture_executable_path(directory: &Path, stem: &str) -> PathBuf {
    directory.join(stem)
}

#[cfg(unix)]
pub(crate) fn wait_for_process_exit(pid: u32, timeout: Duration) -> bool {
    use std::thread;
    use std::time::Instant;

    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        // SAFETY: signal zero only probes whether this PID still exists.
        if unsafe { libc::kill(pid as i32, 0) } == -1 {
            return true;
        }
        thread::sleep(Duration::from_millis(25));
    }
    false
}

#[cfg(windows)]
pub(crate) fn wait_for_process_exit(pid: u32, timeout: Duration) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Threading::{
        OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE,
    };

    // SAFETY: the returned synchronization handle is closed below.
    let process = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    if process.is_null() {
        return true;
    }
    // SAFETY: process is a live synchronization handle.
    let result = unsafe { WaitForSingleObject(process, timeout.as_millis() as u32) };
    // SAFETY: process is owned by this function and closed exactly once.
    unsafe { CloseHandle(process) };
    result == WAIT_OBJECT_0
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn wait_for_process_exit(_pid: u32, _timeout: Duration) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::{
        classify_file_link_fixture_result, create_file_link_fixture_for_test,
        set_unix_mode_for_test, unix_mode_for_test, FileLinkFixtureOutcome,
    };
    use std::fs;
    use std::io;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    #[test]
    fn unix_mode_fixture_is_exact_or_explicitly_unsupported() {
        let root = unique_temp_root("unix-mode");
        let target = root.join("target.bin");
        fs::write(&target, b"target").unwrap();

        let supported = set_unix_mode_for_test(&target, 0o640).unwrap();
        let mode = unix_mode_for_test(&target).unwrap();

        match mode {
            Some(mode) => {
                assert!(supported);
                assert_eq!(mode, 0o640);
            }
            None => assert!(!supported),
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn file_link_fixture_reports_only_explicit_unavailability() {
        let root = unique_temp_root("file-link");
        let source = root.join("source.bin");
        let target = root.join("target.bin");
        fs::write(&source, b"source").unwrap();

        let outcome = create_file_link_fixture_for_test(&source, &target)
            .expect("unexpected file-link creation error must fail the fixture test");

        match outcome {
            FileLinkFixtureOutcome::Created => {
                assert_eq!(fs::read_link(&target).unwrap(), source);
            }
            FileLinkFixtureOutcome::Unsupported => {
                eprintln!("[SKIPPED FIXTURE] file links are unsupported on this host");
            }
            FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => {
                eprintln!("[SKIPPED FIXTURE] Windows file-link privilege is unavailable");
            }
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn generic_permission_denied_file_link_error_is_not_skipped() {
        let error = classify_file_link_fixture_result(Some(Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "synthetic generic permission denial",
        ))))
        .expect_err("generic PermissionDenied must remain a real fixture error");

        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(error.raw_os_error(), None);
    }

    #[cfg(windows)]
    #[test]
    fn windows_error_1314_is_the_only_privilege_skip() {
        let outcome =
            classify_file_link_fixture_result(Some(Err(io::Error::from_raw_os_error(1314))))
                .unwrap();

        assert_eq!(outcome, FileLinkFixtureOutcome::WindowsPrivilegeUnavailable);
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_error_1314_remains_a_real_error() {
        let error =
            classify_file_link_fixture_result(Some(Err(io::Error::from_raw_os_error(1314))))
                .expect_err("raw error 1314 must not be special outside Windows");

        assert_eq!(error.raw_os_error(), Some(1314));
    }

    fn unique_temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "unica-platform-testing-{name}-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }
}
