use std::path::{Path, PathBuf};
use std::time::Duration;

pub(crate) use super::filesystem::{create_dir_symlink_for_test, create_file_symlink_for_test};

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
