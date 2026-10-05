use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[cfg(windows)]
use super::filesystem::open_directory_nofollow;
pub(crate) use super::filesystem::{
    create_dir_symlink_for_test, create_file_symlink_for_test, remove_dir_symlink_for_test,
};
use super::filesystem::{file_identity, hard_link_count};

pub(crate) fn normalize_path_text_for_test(value: &str) -> String {
    value.replace('\\', "/")
}

pub(crate) fn path_text_for_test(path: &Path) -> String {
    normalize_path_text_for_test(&path.display().to_string())
}

#[cfg(windows)]
pub(crate) fn case_distinct_provider_paths_for_test(_parent: &Path) -> Option<(PathBuf, PathBuf)> {
    None
}

#[cfg(not(windows))]
pub(crate) fn case_distinct_provider_paths_for_test(parent: &Path) -> Option<(PathBuf, PathBuf)> {
    Some((
        parent.join("CaseSensitiveWorkspace"),
        parent.join("casesensitiveworkspace"),
    ))
}

#[cfg(all(unix, not(target_vendor = "apple")))]
pub(crate) fn distinct_non_utf8_provider_paths_for_test(
    parent: &Path,
) -> Option<(PathBuf, PathBuf)> {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    Some((
        parent.join(OsString::from_vec(b"provider-\x80".to_vec())),
        parent.join(OsString::from_vec(b"provider-\x81".to_vec())),
    ))
}

#[cfg(not(all(unix, not(target_vendor = "apple"))))]
pub(crate) fn distinct_non_utf8_provider_paths_for_test(
    _parent: &Path,
) -> Option<(PathBuf, PathBuf)> {
    None
}

#[cfg(unix)]
pub(crate) fn non_utf8_relative_path_for_test() -> Option<PathBuf> {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    Some(PathBuf::from(OsString::from_vec(
        b"Module-\xff.bsl".to_vec(),
    )))
}

#[cfg(not(unix))]
pub(crate) fn non_utf8_relative_path_for_test() -> Option<PathBuf> {
    None
}

#[cfg(windows)]
pub(crate) fn windows_extended_length_path_for_test(path: &Path) -> Option<PathBuf> {
    Some(PathBuf::from(format!(r"\\?\{}", path.display())))
}

#[cfg(not(windows))]
pub(crate) fn windows_extended_length_path_for_test(_path: &Path) -> Option<PathBuf> {
    None
}

#[cfg(unix)]
pub(crate) fn can_rename_parent_with_retained_cleanup_child_for_test() -> bool {
    true
}

#[cfg(not(unix))]
pub(crate) fn can_rename_parent_with_retained_cleanup_child_for_test() -> bool {
    false
}

#[cfg(unix)]
pub(crate) fn can_swap_named_child_behind_retained_handle_for_test() -> bool {
    true
}

#[cfg(not(unix))]
pub(crate) fn can_swap_named_child_behind_retained_handle_for_test() -> bool {
    false
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RetainedDirectoryReplacementOutcome {
    Replaced,
    PreventedByRetainedHandle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RetainedRegularFileRelocationOutcome {
    Relocated,
    PreventedByRetainedHandle,
}

pub(crate) fn attempt_retained_directory_replacement_for_test(
    named: &Path,
    displaced: &Path,
) -> io::Result<RetainedDirectoryReplacementOutcome> {
    match std::fs::rename(named, displaced) {
        Ok(()) => Ok(RetainedDirectoryReplacementOutcome::Replaced),
        Err(error) if windows_retained_name_relocation_was_prevented(&error) => {
            Ok(RetainedDirectoryReplacementOutcome::PreventedByRetainedHandle)
        }
        Err(error) => Err(error),
    }
}

pub(crate) fn attempt_retained_regular_file_relocation_for_test(
    named: &Path,
    relocated: &Path,
) -> io::Result<RetainedRegularFileRelocationOutcome> {
    match std::fs::rename(named, relocated) {
        Ok(()) => Ok(RetainedRegularFileRelocationOutcome::Relocated),
        Err(error) if windows_retained_name_relocation_was_prevented(&error) => {
            Ok(RetainedRegularFileRelocationOutcome::PreventedByRetainedHandle)
        }
        Err(error) => Err(error),
    }
}

#[cfg(windows)]
fn windows_retained_name_relocation_was_prevented(error: &io::Error) -> bool {
    error.raw_os_error() == Some(windows_sys::Win32::Foundation::ERROR_ACCESS_DENIED as i32)
        || super::filesystem::is_file_sharing_violation(error)
}

#[cfg(not(windows))]
fn windows_retained_name_relocation_was_prevented(_error: &io::Error) -> bool {
    false
}

#[cfg(windows)]
pub(crate) fn sharing_violation_error_cases_for_test() -> [(io::Error, bool); 2] {
    use windows_sys::Win32::Foundation::{ERROR_LOCK_VIOLATION, ERROR_SHARING_VIOLATION};

    [
        (
            io::Error::from_raw_os_error(ERROR_SHARING_VIOLATION as i32),
            true,
        ),
        (
            io::Error::from_raw_os_error(ERROR_LOCK_VIOLATION as i32),
            false,
        ),
    ]
}

#[cfg(not(windows))]
pub(crate) fn sharing_violation_error_cases_for_test() -> [(io::Error, bool); 2] {
    // The same numeric errno values must not acquire Windows semantics here.
    [
        (io::Error::from_raw_os_error(32), false),
        (io::Error::from_raw_os_error(33), false),
    ]
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

pub(crate) fn create_directory_link_fixture_for_test(
    source: impl AsRef<Path>,
    target: impl AsRef<Path>,
) -> io::Result<FileLinkFixtureOutcome> {
    classify_file_link_fixture_result(create_dir_symlink_for_test(source, target))
}

pub(crate) fn file_identity_for_test(path: &Path) -> io::Result<Option<String>> {
    let file = std::fs::File::open(path)?;
    identity_for_open_path_for_test(&file)
}

pub(crate) fn path_identity_for_test(path: &Path) -> io::Result<Option<String>> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path identity fixture does not follow symbolic links",
        ));
    }
    #[cfg(windows)]
    let file = if metadata.is_dir() {
        open_directory_nofollow(path)?
    } else {
        std::fs::File::open(path)?
    };
    #[cfg(not(windows))]
    let file = std::fs::File::open(path)?;
    identity_for_open_path_for_test(&file)
}

fn identity_for_open_path_for_test(file: &std::fs::File) -> io::Result<Option<String>> {
    let identity = match file_identity(file) {
        Ok(identity) => identity,
        Err(error) if error.kind() == io::ErrorKind::Unsupported => return Ok(None),
        Err(error) => return Err(error),
    };
    let links = match hard_link_count(file) {
        Ok(links) => links,
        Err(error) if error.kind() == io::ErrorKind::Unsupported => return Ok(None),
        Err(error) => return Err(error),
    };
    Ok(Some(format!("{identity:?}; links={links}")))
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
pub(crate) fn run_exact_test_in_owned_process_for_test(
    test_name: &str,
    marker: &str,
    extra_environment: &[(&str, &std::ffi::OsStr)],
) -> io::Result<std::process::ExitStatus> {
    use std::process::{Child, Command};
    use std::time::Instant;

    struct OwnedTestChild {
        child: Child,
        reaped: bool,
    }
    impl Drop for OwnedTestChild {
        fn drop(&mut self) {
            if !self.reaped {
                let _ = self.child.kill();
                let _ = self.child.wait();
            }
        }
    }

    let mut command = Command::new(std::env::current_exe()?);
    command.args(["--exact", test_name, "--nocapture"]);
    command.env(marker, "1");
    for &(name, value) in extra_environment {
        command.env(name, value);
    }
    let child = command.spawn()?;
    let mut child = OwnedTestChild {
        child,
        reaped: false,
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.child.try_wait()? {
            child.reaped = true;
            return Ok(status);
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "isolated test process did not exit before its deadline",
            ));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(all(test, unix))]
pub(crate) struct ForkedInheritedDescriptorForTest {
    pid: libc::pid_t,
    release_writer: Option<std::os::fd::OwnedFd>,
}

#[cfg(all(test, unix))]
impl ForkedInheritedDescriptorForTest {
    pub(crate) fn start() -> io::Result<Self> {
        use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
        use std::time::Instant;

        fn pipe() -> io::Result<(OwnedFd, OwnedFd)> {
            let mut descriptors = [-1; 2];
            // SAFETY: `pipe` initializes both descriptors on success; each is
            // immediately owned by an `OwnedFd` in the parent.
            if unsafe { libc::pipe(descriptors.as_mut_ptr()) } == -1 {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: successful `pipe` returned distinct owned descriptors.
            Ok(unsafe {
                (
                    OwnedFd::from_raw_fd(descriptors[0]),
                    OwnedFd::from_raw_fd(descriptors[1]),
                )
            })
        }

        let (ready_reader, ready_writer) = pipe()?;
        let (release_reader, release_writer) = pipe()?;
        // SAFETY: the child executes only async-signal-safe syscalls and then
        // `_exit`; it never touches Rust state after forking the test harness.
        let pid = unsafe { libc::fork() };
        if pid == -1 {
            return Err(io::Error::last_os_error());
        }
        if pid == 0 {
            // SAFETY: each syscall below is async-signal-safe. The parent
            // owns and reaps this child, including if readiness fails.
            unsafe {
                libc::close(ready_reader.as_raw_fd());
                libc::close(release_writer.as_raw_fd());
                let ready = [1_u8];
                if libc::write(ready_writer.as_raw_fd(), ready.as_ptr().cast(), 1) != 1 {
                    libc::_exit(101);
                }
                let mut release = [0_u8];
                let received =
                    libc::read(release_reader.as_raw_fd(), release.as_mut_ptr().cast(), 1);
                libc::_exit(if received == 0 { 0 } else { 102 });
            }
        }

        drop(ready_writer);
        drop(release_reader);
        let child = Self {
            pid,
            release_writer: Some(release_writer),
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "forked lock holder did not report readiness",
                ));
            }
            let mut poll = libc::pollfd {
                fd: ready_reader.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            let timeout = i32::try_from(remaining.as_millis()).unwrap_or(i32::MAX);
            // SAFETY: `poll` receives a valid pointer to one initialized item.
            let polled = unsafe { libc::poll(&mut poll, 1, timeout) };
            if polled == -1 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(error);
            }
            if polled == 0 {
                continue;
            }
            let mut ready = [0_u8];
            // SAFETY: `poll` reported readiness on this owned descriptor.
            let count =
                unsafe { libc::read(ready_reader.as_raw_fd(), ready.as_mut_ptr().cast(), 1) };
            if count == 1 && ready[0] == 1 {
                return Ok(child);
            }
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "forked lock holder did not send its ready byte",
            ));
        }
    }

    pub(crate) fn release_and_wait(mut self) -> io::Result<()> {
        use std::time::Instant;

        // Closing the sole writer releases the child by EOF. A byte write
        // would risk SIGPIPE if it exited between READY and this call.
        self.release_writer.take();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let mut status = 0;
            // SAFETY: this fixture is the only owner that waits for this pid.
            let observed = unsafe { libc::waitpid(self.pid, &mut status, libc::WNOHANG) };
            if observed == self.pid {
                self.pid = 0;
                if libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0 {
                    return Ok(());
                }
                return Err(io::Error::other("forked lock holder exited unsuccessfully"));
            }
            if observed == -1 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                if error.raw_os_error() == Some(libc::ECHILD) {
                    self.pid = 0;
                }
                return Err(error);
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "forked lock holder did not exit after release",
                ));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

#[cfg(all(test, unix))]
impl Drop for ForkedInheritedDescriptorForTest {
    fn drop(&mut self) {
        self.release_writer.take();
        if self.pid > 0 {
            // SAFETY: this fixture still owns an unreaped child pid. SIGKILL
            // breaks a stuck pipe read; waitpid then prevents a zombie.
            unsafe {
                libc::kill(self.pid, libc::SIGKILL);
                let mut status = 0;
                while libc::waitpid(self.pid, &mut status, 0) == -1
                    && io::Error::last_os_error().kind() == io::ErrorKind::Interrupted
                {
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        classify_file_link_fixture_result, create_file_link_fixture_for_test,
        file_identity_for_test, path_identity_for_test, set_unix_mode_for_test, unix_mode_for_test,
        FileLinkFixtureOutcome,
    };
    use std::fs;
    use std::io;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[cfg(unix)]
    #[test]
    fn forked_child_retains_receipt_lock_after_store_drop_until_child_exit() {
        use crate::application::receipt_ledger::ReceiptLedgerError;
        use crate::infrastructure::receipt_ledger::ReceiptLedgerStore;

        const HELPER_MARKER: &str = "UNICA_RECEIPT_LEDGER_FORKED_LOCK_PROBE_HELPER";
        if std::env::var_os(HELPER_MARKER).is_none() {
            let status = crate::infrastructure::platform::testing::run_exact_test_in_owned_process_for_test(
                "infrastructure::platform::testing::tests::forked_child_retains_receipt_lock_after_store_drop_until_child_exit",
                HELPER_MARKER,
                &[],
            )
            .expect("run isolated fork/lock proof process");
            assert!(status.success(), "isolated fork/lock proof failed");
            return;
        }

        let root = tempfile::tempdir().expect("temporary root");
        let receipts = fs::canonicalize(root.path())
            .expect("physical temporary root")
            .join("receipts");
        let store = ReceiptLedgerStore::open(&receipts).expect("open receipt ledger");
        let child =
            crate::infrastructure::platform::testing::ForkedInheritedDescriptorForTest::start()
                .expect("fork a child retaining the receipt lock description");

        drop(store);
        assert_eq!(
            ReceiptLedgerStore::open(&receipts).err(),
            Some(ReceiptLedgerError::AlreadyOwned),
            "dropping only the parent owner must not release the inherited lock"
        );

        child
            .release_and_wait()
            .expect("reap the inherited lock holder");
        let reopened = ReceiptLedgerStore::open(&receipts)
            .expect("the lock becomes available after the child really exits");
        assert_eq!(reopened.generation().expect("recovered generation"), 0);
    }

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
    fn file_identity_fixture_observes_hard_link_aliases_when_supported() {
        let root = unique_temp_root("file-identity");
        let source = root.join("source.bin");
        let alias = root.join("alias.bin");
        fs::write(&source, b"source").unwrap();
        fs::hard_link(&source, &alias).unwrap();

        let source_identity = file_identity_for_test(&source).unwrap();
        let alias_identity = file_identity_for_test(&alias).unwrap();

        assert_eq!(source_identity, alias_identity);
        if let Some(identity) = source_identity {
            assert!(identity.contains("links=2"), "{identity}");
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn path_identity_fixture_reads_directory_identity() {
        let root = unique_temp_root("path-identity-directory");

        let first = path_identity_for_test(&root).unwrap();
        let second = path_identity_for_test(&root).unwrap();

        assert_eq!(first, second);
        assert!(first.is_some(), "directory identity must be available");
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
