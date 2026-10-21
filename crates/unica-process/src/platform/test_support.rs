//! Fixtures and fault injection for integration tests of process consumers.

#[cfg(unix)]
pub use super::process::assert_released_unix_group_never_signals_reused_identity_for_test;
#[cfg(windows)]
pub use super::process::assert_windows_runtime_process_tree_semantics_for_test;
pub use super::process::{
    inject_runtime_tree_cleanup_timeout_for_test, reset_runtime_tree_cleanup_calls_for_test,
    runtime_process_tree_test_scenario_for_test, runtime_tree_cleanup_calls_for_test,
    JobAttachGateForTest, RuntimeProcessTreeTestProbe, RuntimeProcessTreeTestScenario,
};

pub use super::host_capability::{
    skip_without_host_capability, HostCapability, HOST_CAPABILITY_SKIPS_ENV,
};

/// Availability of the existing Unix process-authority fixture.
pub fn unix_runtime_authority_tests_supported() -> bool {
    cfg!(unix)
}

// Unix process-authority faults. Callers skip these scenarios through
// `unix_runtime_authority_tests_supported`; other hosts keep them inert.
pub fn inject_unix_waitid_error_for_test() {
    #[cfg(unix)]
    super::process::inject_unix_waitid_error_for_test();
}

pub fn inject_unix_reap_error_for_test() {
    #[cfg(unix)]
    super::process::inject_unix_reap_error_for_test();
}

pub fn reset_unix_signal_count_for_test() {
    #[cfg(unix)]
    super::process::reset_unix_signal_count_for_test();
}

pub fn unix_signal_count_for_test() -> u32 {
    #[cfg(unix)]
    {
        super::process::unix_signal_count_for_test()
    }
    #[cfg(not(unix))]
    0
}

pub fn reap_runtime_authority_test_child(process_id: u32) {
    #[cfg(unix)]
    super::process::reap_runtime_authority_test_child(process_id);
    #[cfg(not(unix))]
    let _ = process_id;
}

pub fn run_exact_test_in_owned_process_for_test(
    test_name: &str,
    marker: &str,
    extra_environment: &[(&str, &std::ffi::OsStr)],
) -> std::io::Result<std::process::ExitStatus> {
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
    let deadline = Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if let Some(status) = child.child.try_wait()? {
            child.reaped = true;
            return Ok(status);
        }
        if Instant::now() >= deadline {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "isolated test process did not exit before its deadline",
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

use std::path::{Path, PathBuf};
use std::time::Duration;

pub struct TestCommand {
    pub program: PathBuf,
    pub args: Vec<String>,
}

#[cfg(windows)]
pub fn command_writing_stdout(text: &str) -> TestCommand {
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
pub fn command_writing_stdout(text: &str) -> TestCommand {
    TestCommand {
        program: PathBuf::from("sh"),
        args: vec![
            "-c".to_string(),
            format!("printf '%s' '{}'", text.replace('\'', "'\\''")),
        ],
    }
}

#[cfg(windows)]
pub fn long_running_command() -> TestCommand {
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
pub fn long_running_command() -> TestCommand {
    TestCommand {
        program: PathBuf::from("sh"),
        args: vec!["-c".to_string(), "sleep 10".to_string()],
    }
}

#[cfg(windows)]
pub fn line_printing_command(sleep_first: bool, lines: &[String]) -> TestCommand {
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
pub fn line_printing_command(sleep_first: bool, lines: &[String]) -> TestCommand {
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
pub fn fixture_executable_path(directory: &Path, stem: &str) -> PathBuf {
    directory.join(format!("{stem}.exe"))
}

#[cfg(not(windows))]
pub fn fixture_executable_path(directory: &Path, stem: &str) -> PathBuf {
    directory.join(stem)
}

#[cfg(unix)]
pub fn wait_for_process_exit(pid: u32, timeout: Duration) -> bool {
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
pub fn wait_for_process_exit(pid: u32, timeout: Duration) -> bool {
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
pub fn wait_for_process_exit(_pid: u32, _timeout: Duration) -> bool {
    false
}
