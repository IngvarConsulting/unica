use crate::domain::cancellation::CancellationToken;
use std::ffi::OsString;
use std::io::{self, Read};
use std::path::PathBuf;
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(25);
const TERMINATION_WAIT_LIMIT: Duration = Duration::from_millis(500);
const READER_WAIT_LIMIT: Duration = Duration::from_millis(500);
pub(crate) const STDOUT_CAPTURE_LIMIT: usize = 1024 * 1024;
pub(crate) const STDERR_CAPTURE_LIMIT: usize = 256 * 1024;

#[derive(Debug, Clone)]
pub struct ManagedCommand {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(OsString, OsString)>,
    pub timeout: Option<Duration>,
    pub cancellation: CancellationToken,
}

#[derive(Debug, Clone)]
pub struct ManagedOutput {
    pub status_success: bool,
    pub status: String,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
    pub cancelled: bool,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
}

pub struct ManagedChild {
    child: Child,
    process_tree: ProcessTree,
    state: ChildState,
    timeout: Option<Duration>,
    cancellation: CancellationToken,
}

/// Owns a freshly spawned long-lived process tree until its readiness handshake
/// succeeds. Dropping this guard terminates the tree; `detach` is the only path
/// that intentionally leaves the process running.
pub(crate) struct ManagedStartupChild {
    child: Option<Child>,
    process_tree: ProcessTree,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChildState {
    Running,
    Terminating,
    Reaped,
}

impl ManagedChild {
    pub fn spawn(command: ManagedCommand) -> Result<Self, String> {
        let mut process = Command::new(&command.program);
        process
            .args(&command.args)
            .current_dir(&command.cwd)
            .envs(command.env);
        Self::spawn_process(process, command.timeout, command.cancellation)
    }

    pub(crate) fn spawn_process(
        mut process: Command,
        timeout: Option<Duration>,
        cancellation: CancellationToken,
    ) -> Result<Self, String> {
        process
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut process_tree = ProcessTree::prepare(&mut process).map_err(process_error)?;
        let mut child = process.spawn().map_err(process_error)?;
        if let Err(error) = process_tree.attach(&child) {
            let _ = process_tree.terminate(&mut child);
            let _ = child.kill();
            let _ = child.try_wait();
            return Err(process_error(error));
        }

        Ok(Self {
            child,
            process_tree,
            state: ChildState::Running,
            timeout,
            cancellation,
        })
    }

    pub fn run(command: ManagedCommand) -> Result<ManagedOutput, String> {
        let mut child = Self::spawn(command)?;
        child.wait_for_output()
    }

    pub fn take_stdin(&mut self) -> Option<ChildStdin> {
        self.child.stdin.take()
    }

    pub fn id(&self) -> u32 {
        self.child.id()
    }

    pub fn is_running(&mut self) -> Result<bool, String> {
        match self.state {
            ChildState::Reaped | ChildState::Terminating => Ok(false),
            ChildState::Running => match self.child.try_wait().map_err(process_error)? {
                Some(_) => {
                    self.process_tree.cleanup_after_leader_exit(&mut self.child);
                    self.state = ChildState::Reaped;
                    Ok(false)
                }
                None => Ok(true),
            },
        }
    }

    pub fn take_stdout(&mut self) -> Option<ChildStdout> {
        self.child.stdout.take()
    }

    pub fn take_stderr(&mut self) -> Option<ChildStderr> {
        self.child.stderr.take()
    }

    pub fn wait_for_output(&mut self) -> Result<ManagedOutput, String> {
        self.wait_for_output_with_poll(PROCESS_POLL_INTERVAL, || {})
    }

    pub fn wait_for_output_with_poll<F>(
        &mut self,
        interval: Duration,
        mut callback: F,
    ) -> Result<ManagedOutput, String>
    where
        F: FnMut(),
    {
        drop(self.take_stdin());
        let stdout = start_reader(self.take_stdout(), STDOUT_CAPTURE_LIMIT);
        let stderr = start_reader(self.take_stderr(), STDERR_CAPTURE_LIMIT);
        let started = Instant::now();
        let mut last_callback = Instant::now();

        loop {
            if self.cancellation.is_cancelled() {
                self.terminate()?;
                return self.finish_after_termination(stdout, stderr, false, true);
            }
            if self.timeout.is_some_and(|limit| started.elapsed() >= limit) {
                self.terminate()?;
                return self.finish_after_termination(stdout, stderr, true, false);
            }
            if let Some(status) = self.child.try_wait().map_err(process_error)? {
                self.process_tree.cleanup_after_leader_exit(&mut self.child);
                self.state = ChildState::Reaped;
                return Ok(finish_output(status, stdout, stderr, false, false));
            }

            thread::sleep(PROCESS_POLL_INTERVAL);
            if last_callback.elapsed() >= interval {
                callback();
                last_callback = Instant::now();
            }
        }
    }

    pub fn terminate(&mut self) -> Result<(), String> {
        if self.state == ChildState::Reaped {
            self.process_tree.cleanup_after_leader_exit(&mut self.child);
            return Ok(());
        }
        if self.state == ChildState::Running {
            if self.child.try_wait().map_err(process_error)?.is_some() {
                self.process_tree.cleanup_after_leader_exit(&mut self.child);
                self.state = ChildState::Reaped;
                return Ok(());
            }
            if let Err(error) = self.process_tree.terminate(&mut self.child) {
                if self.child.try_wait().map_err(process_error)?.is_some() {
                    self.state = ChildState::Reaped;
                    return Ok(());
                }
                return Err(process_error(error));
            }
            self.state = ChildState::Terminating;
        }
        self.reap_bounded()
    }

    fn reap_bounded(&mut self) -> Result<(), String> {
        let started = Instant::now();
        while started.elapsed() < TERMINATION_WAIT_LIMIT {
            if self.child.try_wait().map_err(process_error)?.is_some() {
                self.state = ChildState::Reaped;
                return Ok(());
            }
            thread::sleep(PROCESS_POLL_INTERVAL);
        }
        Ok(())
    }

    fn finish_after_termination(
        &mut self,
        stdout: Option<Receiver<CapturedOutput>>,
        stderr: Option<Receiver<CapturedOutput>>,
        timed_out: bool,
        cancelled: bool,
    ) -> Result<ManagedOutput, String> {
        let started = Instant::now();
        while started.elapsed() < TERMINATION_WAIT_LIMIT {
            if let Some(status) = self.child.try_wait().map_err(process_error)? {
                self.state = ChildState::Reaped;
                return Ok(finish_output(status, stdout, stderr, timed_out, cancelled));
            }
            thread::sleep(PROCESS_POLL_INTERVAL);
        }

        let stdout = receive_output(stdout);
        let stderr = receive_output(stderr);
        let mut output = ManagedOutput {
            status_success: false,
            status: "termination pending".to_string(),
            stdout: String::from_utf8_lossy(&stdout.bytes).into_owned(),
            stderr: String::from_utf8_lossy(&stderr.bytes).into_owned(),
            timed_out,
            cancelled,
            stdout_truncated: stdout.truncated,
            stderr_truncated: stderr.truncated,
        };
        ensure_truncation_diagnostics(&mut output);
        Ok(output)
    }
}

impl Drop for ManagedChild {
    fn drop(&mut self) {
        let _ = self.terminate();
    }
}

impl ManagedStartupChild {
    pub(crate) fn spawn_configured(mut process: Command) -> Result<Self, String> {
        let process_tree = ProcessTree::prepare_detachable(&mut process).map_err(process_error)?;
        let child = process.spawn().map_err(process_error)?;
        let mut managed = Self {
            child: Some(child),
            process_tree,
        };
        if let Err(error) = managed
            .process_tree
            .attach(managed.child.as_ref().expect("startup child exists"))
        {
            let cleanup = managed.terminate_bounded(TERMINATION_WAIT_LIMIT);
            return match cleanup {
                Ok(()) => Err(process_error(error)),
                Err(cleanup_error) => Err(format!("{}; {cleanup_error}", process_error(error))),
            };
        }
        Ok(managed)
    }

    pub(crate) fn id(&self) -> u32 {
        self.child.as_ref().expect("startup child exists").id()
    }

    #[cfg(test)]
    pub(crate) fn is_running(&mut self) -> Result<bool, String> {
        self.child
            .as_mut()
            .expect("startup child exists")
            .try_wait()
            .map(|status| status.is_none())
            .map_err(process_error)
    }

    pub(crate) fn terminate_bounded(&mut self, wait_limit: Duration) -> Result<(), String> {
        let Some(child) = self.child.as_mut() else {
            return Ok(());
        };
        if child.try_wait().map_err(process_error)?.is_some() {
            self.process_tree.cleanup_after_leader_exit(child);
            self.child.take();
            return Ok(());
        }

        let tree_error = self.process_tree.terminate(child).err();
        // Also target the leader directly. This is required when Windows Job Object
        // attachment itself failed, and is harmless after a successful tree kill.
        let child_error = child.kill().err();
        let started = Instant::now();
        while started.elapsed() < wait_limit {
            let child = self.child.as_mut().expect("startup child exists");
            if child.try_wait().map_err(process_error)?.is_some() {
                self.process_tree.cleanup_after_leader_exit(child);
                self.child.take();
                return Ok(());
            }
            thread::sleep(PROCESS_POLL_INTERVAL);
        }

        let pid = self.id();
        let errors = [tree_error, child_error]
            .into_iter()
            .flatten()
            .map(|error| error.to_string())
            .collect::<Vec<_>>();
        if errors.is_empty() {
            Err(format!(
                "process tree rooted at {pid} did not exit within {} ms",
                wait_limit.as_millis()
            ))
        } else {
            Err(format!(
                "failed to terminate process tree rooted at {pid}: {}",
                errors.join("; ")
            ))
        }
    }

    pub(crate) fn detach(&mut self) -> Result<(), String> {
        let child = self.child.as_mut().expect("startup child exists");
        if child.try_wait().map_err(process_error)?.is_some() {
            self.process_tree.cleanup_after_leader_exit(child);
            return Err("process_failed: startup process exited before detach".to_string());
        }
        self.process_tree.detach().map_err(process_error)?;
        self.child.take();
        Ok(())
    }
}

impl Drop for ManagedStartupChild {
    fn drop(&mut self) {
        let _ = self.terminate_bounded(TERMINATION_WAIT_LIMIT);
    }
}

#[cfg(unix)]
struct ProcessTree {
    process_group: Option<i32>,
    kill_sent: bool,
}

#[cfg(unix)]
impl ProcessTree {
    fn prepare(command: &mut Command) -> io::Result<Self> {
        use std::os::unix::process::CommandExt;

        // SAFETY: `setpgid` is async-signal-safe and the closure performs no allocation.
        unsafe {
            command.pre_exec(|| {
                if libc::setpgid(0, 0) == -1 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        Ok(Self {
            process_group: None,
            kill_sent: false,
        })
    }

    fn prepare_detachable(command: &mut Command) -> io::Result<Self> {
        Self::prepare(command)
    }

    fn attach(&mut self, child: &Child) -> io::Result<()> {
        self.process_group = Some(child.id() as i32);
        self.kill_sent = false;
        Ok(())
    }

    fn terminate(&mut self, _child: &mut Child) -> io::Result<()> {
        let Some(pgid) = self.process_group else {
            return Ok(());
        };
        if self.kill_sent {
            return Ok(());
        }
        let process_group = -pgid;
        // SAFETY: the negative PID targets only the process group created in `prepare`.
        if unsafe { libc::kill(process_group, libc::SIGKILL) } == -1 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ESRCH) {
                self.process_group = None;
                return Ok(());
            }
            return Err(error);
        }
        self.kill_sent = true;
        Ok(())
    }

    fn cleanup_after_leader_exit(&mut self, child: &mut Child) {
        let deadline = Instant::now() + TERMINATION_WAIT_LIMIT;
        let _ = self.terminate(child);
        while let Some(pgid) = self.process_group {
            if Instant::now() >= deadline {
                // SIGKILL was already delivered to the owned group. Forget the numeric PGID
                // rather than risk targeting an unrelated group after identifier reuse.
                self.process_group = None;
                break;
            }
            // SAFETY: signal 0 only probes the group and cannot affect a recycled process.
            if unsafe { libc::kill(-pgid, 0) } == -1
                && io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
            {
                self.process_group = None;
                break;
            }
            thread::sleep(PROCESS_POLL_INTERVAL);
        }
    }

    fn detach(&mut self) -> io::Result<()> {
        self.process_group = None;
        Ok(())
    }
}

#[cfg(windows)]
struct ProcessTree {
    job: windows_sys::Win32::Foundation::HANDLE,
}

// SAFETY: Windows kernel handles may be transferred and used from other threads.
#[cfg(windows)]
unsafe impl Send for ProcessTree {}

// SAFETY: the Job Object APIs used here support concurrent access to the handle.
#[cfg(windows)]
unsafe impl Sync for ProcessTree {}

#[cfg(windows)]
impl ProcessTree {
    fn prepare(command: &mut Command) -> io::Result<Self> {
        Self::prepare_with_policy(command, true)
    }

    fn prepare_detachable(command: &mut Command) -> io::Result<Self> {
        Self::prepare_with_policy(command, false)
    }

    fn prepare_with_policy(command: &mut Command, kill_on_close: bool) -> io::Result<Self> {
        use std::mem::{size_of, zeroed};
        use std::os::windows::process::CommandExt;
        use windows_sys::Win32::System::JobObjects::{
            CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        };
        use windows_sys::Win32::System::Threading::CREATE_SUSPENDED;

        command.creation_flags(CREATE_SUSPENDED);

        // SAFETY: null security attributes and name request an unnamed job with defaults.
        let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if job.is_null() {
            return Err(io::Error::last_os_error());
        }

        if kill_on_close {
            // SAFETY: this Windows POD structure is valid when zero-initialized.
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            // SAFETY: `limits` points to the structure and size required by the information class.
            let configured = unsafe {
                SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    &limits as *const _ as *const _,
                    size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )
            };
            if configured == 0 {
                let error = io::Error::last_os_error();
                // SAFETY: `job` is a live handle created above and is not used after closing.
                unsafe {
                    windows_sys::Win32::Foundation::CloseHandle(job);
                }
                return Err(error);
            }
        }

        Ok(Self { job })
    }

    fn attach(&mut self, child: &Child) -> io::Result<()> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;
        use windows_sys::Win32::System::Threading::ResumeThread;

        // SAFETY: both handles are live for the duration of the call.
        if unsafe { AssignProcessToJobObject(self.job, child.as_raw_handle() as _) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let primary_thread = open_primary_thread(child.id())?;
        // SAFETY: the thread handle was opened with `THREAD_SUSPEND_RESUME` access.
        let previous_suspend_count = unsafe { ResumeThread(primary_thread.0) };
        if previous_suspend_count == u32::MAX {
            return Err(io::Error::last_os_error());
        }
        if previous_suspend_count != 1 {
            return Err(io::Error::other(format!(
                "unexpected primary thread suspend count: {previous_suspend_count}"
            )));
        }
        Ok(())
    }

    fn terminate(&mut self, _child: &mut Child) -> io::Result<()> {
        use windows_sys::Win32::System::JobObjects::TerminateJobObject;

        // SAFETY: `self.job` remains live until `Drop`.
        if unsafe { TerminateJobObject(self.job, 1) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    fn cleanup_after_leader_exit(&mut self, child: &mut Child) {
        let _ = self.terminate(child);
    }

    fn detach(&mut self) -> io::Result<()> {
        if self.job.is_null() {
            return Ok(());
        }
        // The detachable startup Job Object has no KILL_ON_JOB_CLOSE policy. Closing
        // its last handle releases ownership without terminating the ready service.
        // SAFETY: `self.job` is owned here and nulled only after a successful close.
        if unsafe { windows_sys::Win32::Foundation::CloseHandle(self.job) } == 0 {
            return Err(io::Error::last_os_error());
        }
        self.job = std::ptr::null_mut();
        Ok(())
    }
}

#[cfg(windows)]
struct ScopedWindowsHandle(windows_sys::Win32::Foundation::HANDLE);

// SAFETY: Windows kernel handles may be transferred and used from other threads.
#[cfg(windows)]
unsafe impl Send for ScopedWindowsHandle {}

// SAFETY: this adapter only performs thread-safe Windows handle operations.
#[cfg(windows)]
unsafe impl Sync for ScopedWindowsHandle {}

#[cfg(windows)]
impl Drop for ScopedWindowsHandle {
    fn drop(&mut self) {
        // SAFETY: this wrapper owns a valid handle and closes it exactly once.
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

#[cfg(windows)]
fn open_primary_thread(process_id: u32) -> io::Result<ScopedWindowsHandle> {
    use std::mem::{size_of, zeroed};
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
    };
    use windows_sys::Win32::System::Threading::{OpenThread, THREAD_SUSPEND_RESUME};

    // SAFETY: the flags request a system thread snapshot; the process ID is ignored for it.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let snapshot = ScopedWindowsHandle(snapshot);
    // SAFETY: this Windows POD structure is valid when zero-initialized and sized below.
    let mut entry: THREADENTRY32 = unsafe { zeroed() };
    entry.dwSize = size_of::<THREADENTRY32>() as u32;
    // SAFETY: `snapshot` and `entry` satisfy the ToolHelp API contract.
    if unsafe { Thread32First(snapshot.0, &mut entry) } == 0 {
        return Err(io::Error::last_os_error());
    }

    loop {
        if entry.th32OwnerProcessID == process_id {
            // `CREATE_SUSPENDED` prevents this process from creating any additional threads.
            // SAFETY: the snapshot supplied this live thread ID; inheritance is disabled.
            let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
            if thread.is_null() {
                return Err(io::Error::last_os_error());
            }
            return Ok(ScopedWindowsHandle(thread));
        }
        // SAFETY: `snapshot` and `entry` remain valid across enumeration calls.
        if unsafe { Thread32Next(snapshot.0, &mut entry) } == 0 {
            return Err(io::Error::last_os_error());
        }
    }
}

#[cfg(windows)]
impl Drop for ProcessTree {
    fn drop(&mut self) {
        if self.job.is_null() {
            return;
        }
        // SAFETY: `self.job` is owned by this value and closed exactly once here.
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.job);
        }
    }
}

#[cfg(not(any(unix, windows)))]
struct ProcessTree;

#[cfg(not(any(unix, windows)))]
impl ProcessTree {
    fn prepare(_command: &mut Command) -> io::Result<Self> {
        Ok(Self)
    }

    fn prepare_detachable(command: &mut Command) -> io::Result<Self> {
        Self::prepare(command)
    }

    fn attach(&mut self, _child: &Child) -> io::Result<()> {
        Ok(())
    }

    fn terminate(&mut self, child: &mut Child) -> io::Result<()> {
        child.kill()
    }

    fn cleanup_after_leader_exit(&mut self, child: &mut Child) {
        let _ = self.terminate(child);
    }

    fn detach(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn process_error(error: io::Error) -> String {
    format!("process_failed: {error}")
}

#[derive(Default)]
struct CapturedOutput {
    bytes: Vec<u8>,
    truncated: bool,
}

fn start_reader<R>(pipe: Option<R>, limit: usize) -> Option<Receiver<CapturedOutput>>
where
    R: Read + Send + 'static,
{
    pipe.map(|mut pipe| {
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let mut captured = CapturedOutput::default();
            let mut chunk = [0_u8; 8192];
            loop {
                match pipe.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(count) => retain_tail(&mut captured, &chunk[..count], limit),
                }
            }
            let _ = sender.send(captured);
        });
        receiver
    })
}

fn finish_output(
    status: ExitStatus,
    stdout: Option<Receiver<CapturedOutput>>,
    stderr: Option<Receiver<CapturedOutput>>,
    timed_out: bool,
    cancelled: bool,
) -> ManagedOutput {
    let stdout = receive_output(stdout);
    let stderr = receive_output(stderr);
    let mut output = ManagedOutput {
        status_success: status.success() && !stdout.truncated && !cancelled && !timed_out,
        status: status.to_string(),
        stdout: String::from_utf8_lossy(&stdout.bytes).into_owned(),
        stderr: String::from_utf8_lossy(&stderr.bytes).into_owned(),
        timed_out,
        cancelled,
        stdout_truncated: stdout.truncated,
        stderr_truncated: stderr.truncated,
    };
    ensure_truncation_diagnostics(&mut output);
    output
}

pub(crate) fn ensure_truncation_diagnostics(output: &mut ManagedOutput) {
    let mut captured = CapturedOutput {
        bytes: output.stderr.as_bytes().to_vec(),
        truncated: output.stderr_truncated,
    };
    if output.stdout_truncated && !output.stderr.contains("stdout capture truncated") {
        retain_tail(
            &mut captured,
            b"\n[unica: stdout capture truncated; result is not parseable]\n",
            STDERR_CAPTURE_LIMIT,
        );
    }
    if output.stderr_truncated && !output.stderr.contains("earlier stderr diagnostics omitted") {
        retain_tail(
            &mut captured,
            b"\n[unica: stderr capture truncated; earlier stderr diagnostics omitted]\n",
            STDERR_CAPTURE_LIMIT,
        );
    }
    output.stderr = String::from_utf8_lossy(&captured.bytes).into_owned();
    output.stderr_truncated = captured.truncated;
}

fn receive_output(receiver: Option<Receiver<CapturedOutput>>) -> CapturedOutput {
    receiver
        .and_then(|receiver| receiver.recv_timeout(READER_WAIT_LIMIT).ok())
        .unwrap_or_default()
}

fn retain_tail(captured: &mut CapturedOutput, chunk: &[u8], limit: usize) {
    if chunk.len() >= limit {
        captured.bytes.clear();
        captured
            .bytes
            .extend_from_slice(&chunk[chunk.len() - limit..]);
        captured.truncated = true;
        return;
    }
    let overflow = captured
        .bytes
        .len()
        .saturating_add(chunk.len())
        .saturating_sub(limit);
    if overflow > 0 {
        captured.bytes.drain(..overflow);
        captured.truncated = true;
    }
    captured.bytes.extend_from_slice(chunk);
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use super::ProcessTree;
    use super::{ChildState, ManagedChild, ManagedCommand, ManagedOutput, ManagedStartupChild};
    use crate::domain::cancellation::CancellationToken;
    use std::ffi::OsString;
    use std::io::Read;
    use std::path::{Path, PathBuf};
    #[cfg(windows)]
    use std::process::Child;
    use std::process::{Command, Stdio};
    use std::thread;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    const HELPER_ENV: &str = "UNICA_MANAGED_CHILD_HELPER";
    const HELPER_PID_FILE_ENV: &str = "UNICA_MANAGED_CHILD_PID_FILE";

    #[test]
    #[allow(clippy::zombie_processes)] // Fixture intentionally exits while its descendant remains alive.
    fn managed_child_test_helper() {
        let Ok(mode) = std::env::var(HELPER_ENV) else {
            return;
        };

        match mode.as_str() {
            "success" => {
                print!("managed stdout");
                eprint!("managed stderr");
            }
            "read_stdin" => {
                let mut input = String::new();
                std::io::stdin().read_to_string(&mut input).unwrap();
                print!("stdin closed");
            }
            "sleep" => thread::sleep(Duration::from_secs(10)),
            "process_tree_immediate_parent" => {
                let pid_file = std::env::var_os(HELPER_PID_FILE_ENV).unwrap();
                let mut child = Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "infrastructure::managed_child::tests::managed_child_test_helper",
                        "--nocapture",
                    ])
                    .env(HELPER_ENV, "process_tree_child")
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .unwrap();
                std::fs::write(
                    pid_file,
                    format!("{}\n{}\n", std::process::id(), child.id()),
                )
                .unwrap();
                child.wait().unwrap();
            }
            "process_tree_child" => thread::sleep(Duration::from_secs(10)),
            "process_tree_detached_leader" => {
                let pid_file = std::env::var_os(HELPER_PID_FILE_ENV).unwrap();
                let child = Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "infrastructure::managed_child::tests::managed_child_test_helper",
                        "--nocapture",
                    ])
                    .env(HELPER_ENV, "process_tree_child")
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .unwrap();
                std::fs::write(
                    pid_file,
                    format!("{}\n{}\n", std::process::id(), child.id()),
                )
                .unwrap();
            }
            "noisy" => {
                let chunk = vec![b'o'; 64 * 1024];
                for _ in 0..40 {
                    std::io::Write::write_all(&mut std::io::stdout(), &chunk).unwrap();
                }
                let err = vec![b'e'; 64 * 1024];
                for _ in 0..12 {
                    std::io::Write::write_all(&mut std::io::stderr(), &err).unwrap();
                }
            }
            "write_marker" => {
                let marker = std::env::var_os(HELPER_PID_FILE_ENV).unwrap();
                std::fs::write(marker, b"started").unwrap();
            }
            other => panic!("unknown managed child helper mode: {other}"),
        }
    }

    #[cfg(windows)]
    mod process_test_support {
        use windows_sys::Win32::Foundation::{CloseHandle, WAIT_TIMEOUT};
        use windows_sys::Win32::System::Threading::{
            OpenProcess, TerminateProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE,
            PROCESS_TERMINATE,
        };

        pub fn is_alive(pid: u32) -> bool {
            unsafe {
                let process = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
                if process.is_null() {
                    return false;
                }
                let alive = WaitForSingleObject(process, 0) == WAIT_TIMEOUT;
                CloseHandle(process);
                alive
            }
        }

        pub fn terminate(pid: u32) {
            unsafe {
                let process = OpenProcess(PROCESS_TERMINATE, 0, pid);
                if !process.is_null() {
                    TerminateProcess(process, 1);
                    CloseHandle(process);
                }
            }
        }
    }

    #[cfg(unix)]
    mod process_test_support {
        pub fn is_alive(pid: u32) -> bool {
            unsafe { libc::kill(pid as i32, 0) == 0 }
        }

        pub fn terminate(pid: u32) {
            unsafe {
                libc::kill(pid as i32, libc::SIGKILL);
            }
        }
    }

    struct ProcessCleanupGuard(Vec<u32>);

    impl ProcessCleanupGuard {
        fn disarm(&mut self) {
            self.0.clear();
        }
    }

    impl Drop for ProcessCleanupGuard {
        fn drop(&mut self) {
            for &pid in &self.0 {
                process_test_support::terminate(pid);
            }
        }
    }

    struct FileCleanupGuard(PathBuf);

    impl Drop for FileCleanupGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[cfg(windows)]
    struct ChildCleanupGuard(Option<Child>);

    #[cfg(windows)]
    impl ChildCleanupGuard {
        fn child(&self) -> &Child {
            self.0.as_ref().unwrap()
        }

        fn wait(mut self) {
            self.0.as_mut().unwrap().wait().unwrap();
            self.0 = None;
        }
    }

    #[cfg(windows)]
    impl Drop for ChildCleanupGuard {
        fn drop(&mut self) {
            if let Some(child) = &mut self.0 {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }

    struct ManagedChildCleanupGuard {
        managed: Option<ManagedChild>,
        cancellation: CancellationToken,
    }

    impl ManagedChildCleanupGuard {
        fn new(managed: ManagedChild, cancellation: CancellationToken) -> Self {
            Self {
                managed: Some(managed),
                cancellation,
            }
        }

        fn managed_mut(&mut self) -> &mut ManagedChild {
            self.managed.as_mut().unwrap()
        }

        fn disarm(&mut self) {
            self.managed = None;
        }
    }

    impl Drop for ManagedChildCleanupGuard {
        fn drop(&mut self) {
            if let Some(managed) = &mut self.managed {
                self.cancellation.cancel();
                let _ = managed.wait_for_output();
            }
        }
    }

    fn read_helper_pids(path: &Path, timeout: Duration) -> Vec<u32> {
        let started = Instant::now();
        while started.elapsed() < timeout {
            if let Ok(contents) = std::fs::read_to_string(path) {
                let pids = contents
                    .lines()
                    .filter_map(|line| line.parse().ok())
                    .collect::<Vec<_>>();
                if pids.len() == 2 {
                    return pids;
                }
            }
            thread::sleep(Duration::from_millis(25));
        }
        panic!("helper did not record both process IDs within {timeout:?}");
    }

    fn wait_until_dead(pid: u32, timeout: Duration) -> bool {
        let started = Instant::now();
        while started.elapsed() < timeout {
            if !process_test_support::is_alive(pid) {
                return true;
            }
            thread::sleep(Duration::from_millis(25));
        }
        !process_test_support::is_alive(pid)
    }

    fn run_helper(
        mode: &str,
        timeout: Duration,
        cancellation: CancellationToken,
    ) -> Result<ManagedOutput, String> {
        ManagedChild::run(ManagedCommand {
            program: std::env::current_exe().map_err(|error| error.to_string())?,
            args: vec![
                "--exact".to_string(),
                "infrastructure::managed_child::tests::managed_child_test_helper".to_string(),
                "--nocapture".to_string(),
            ],
            cwd: std::env::current_dir().map_err(|error| error.to_string())?,
            env: vec![(OsString::from(HELPER_ENV), OsString::from(mode))],
            timeout: Some(timeout),
            cancellation,
        })
    }

    #[test]
    fn managed_child_collects_stdout_and_stderr_on_success() {
        let output =
            run_helper("success", Duration::from_secs(2), CancellationToken::new()).unwrap();

        assert!(output.status_success, "status was {}", output.status);
        assert!(
            output.stdout.contains("managed stdout"),
            "{}",
            output.stdout
        );
        assert!(
            output.stderr.contains("managed stderr"),
            "{}",
            output.stderr
        );
        assert!(!output.timed_out);
        assert!(!output.cancelled);
    }

    #[test]
    fn managed_child_spawn_failure_uses_stable_process_failed_prefix() {
        let error = ManagedChild::spawn(ManagedCommand {
            program: std::env::temp_dir().join("unica-managed-child-missing-executable"),
            args: Vec::new(),
            cwd: std::env::current_dir().unwrap(),
            env: Vec::new(),
            timeout: None,
            cancellation: CancellationToken::new(),
        })
        .err()
        .expect("missing executable must fail to spawn");

        assert!(error.starts_with("process_failed:"), "{error}");
    }

    #[test]
    fn managed_child_timeout_returns_within_a_bounded_interval() {
        let started = Instant::now();
        let output = run_helper(
            "sleep",
            Duration::from_millis(100),
            CancellationToken::new(),
        )
        .unwrap();

        assert!(output.timed_out);
        assert!(!output.cancelled);
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn managed_child_closes_unused_stdin_before_waiting() {
        let output = run_helper(
            "read_stdin",
            Duration::from_millis(300),
            CancellationToken::new(),
        )
        .unwrap();

        assert!(!output.timed_out);
        assert!(output.stdout.contains("stdin closed"), "{}", output.stdout);
    }

    #[test]
    fn managed_child_cancellation_returns_within_a_bounded_interval() {
        let cancellation = CancellationToken::new();
        let cancellation_for_thread = cancellation.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(100));
            cancellation_for_thread.cancel();
        });
        let started = Instant::now();
        let output = run_helper("sleep", Duration::from_secs(10), cancellation).unwrap();

        assert!(output.cancelled);
        assert!(!output.timed_out);
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn cancellation_wins_over_already_successful_exit() {
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let output = run_helper("success", Duration::from_secs(2), cancellation).unwrap();
        assert!(output.cancelled);
        assert!(!output.status_success);
    }

    #[test]
    fn managed_child_bounds_noisy_output_without_deadlock() {
        let output = run_helper("noisy", Duration::from_secs(5), CancellationToken::new()).unwrap();
        assert!(output.stdout_truncated);
        assert!(output.stderr_truncated);
        assert!(output.stdout.len() <= super::STDOUT_CAPTURE_LIMIT);
        assert!(output.stderr.len() <= super::STDERR_CAPTURE_LIMIT);
        assert!(
            !output.status_success,
            "partial stdout must not be treated as parseable success"
        );
        assert!(
            output.stderr.contains("stdout capture truncated"),
            "{}",
            output.stderr
        );
        assert!(
            output.stderr.contains("earlier stderr diagnostics omitted"),
            "{}",
            output.stderr
        );
    }

    #[cfg(unix)]
    #[test]
    fn reaped_leader_does_not_release_living_process_group_descendant() {
        let pid_file = std::env::temp_dir().join(format!(
            "unica-managed-child-detached-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _file = FileCleanupGuard(pid_file.clone());
        let mut managed = ManagedChild::spawn(ManagedCommand {
            program: std::env::current_exe().unwrap(),
            args: vec![
                "--exact".into(),
                "infrastructure::managed_child::tests::managed_child_test_helper".into(),
                "--nocapture".into(),
            ],
            cwd: std::env::current_dir().unwrap(),
            env: vec![
                (
                    OsString::from(HELPER_ENV),
                    OsString::from("process_tree_detached_leader"),
                ),
                (
                    OsString::from(HELPER_PID_FILE_ENV),
                    pid_file.clone().into_os_string(),
                ),
            ],
            timeout: None,
            cancellation: CancellationToken::new(),
        })
        .unwrap();
        let pids = read_helper_pids(&pid_file, Duration::from_secs(2));
        let mut cleanup = ProcessCleanupGuard(pids.clone());
        let descendant = pids[1];
        let started = Instant::now();
        while managed.is_running().unwrap() && started.elapsed() < Duration::from_secs(2) {}
        drop(managed);
        assert!(wait_until_dead(descendant, Duration::from_secs(2)));
        cleanup.disarm();
    }

    #[test]
    fn managed_child_termination_is_idempotent_and_reaps_direct_child() {
        let mut managed = ManagedChild::spawn(ManagedCommand {
            program: std::env::current_exe().unwrap(),
            args: vec![
                "--exact".to_string(),
                "infrastructure::managed_child::tests::managed_child_test_helper".to_string(),
                "--nocapture".to_string(),
            ],
            cwd: std::env::current_dir().unwrap(),
            env: vec![(OsString::from(HELPER_ENV), OsString::from("sleep"))],
            timeout: None,
            cancellation: CancellationToken::new(),
        })
        .unwrap();

        managed.terminate().unwrap();
        assert_eq!(managed.state, ChildState::Reaped);
        let second = Instant::now();
        managed.terminate().unwrap();
        assert!(second.elapsed() < Duration::from_millis(100));
        assert_eq!(managed.state, ChildState::Reaped);
    }

    #[test]
    fn managed_child_reaps_already_exited_process_without_tree_kill() {
        let mut managed = ManagedChild::spawn(ManagedCommand {
            program: std::env::current_exe().unwrap(),
            args: vec![
                "--exact".to_string(),
                "infrastructure::managed_child::tests::managed_child_test_helper".to_string(),
                "--nocapture".to_string(),
            ],
            cwd: std::env::current_dir().unwrap(),
            env: vec![(OsString::from(HELPER_ENV), OsString::from("success"))],
            timeout: None,
            cancellation: CancellationToken::new(),
        })
        .unwrap();
        thread::sleep(Duration::from_millis(100));

        managed.terminate().unwrap();
        assert_eq!(managed.state, ChildState::Reaped);
    }

    #[test]
    fn managed_child_drop_terminates_and_reaps_running_process() {
        let managed = ManagedChild::spawn(ManagedCommand {
            program: std::env::current_exe().unwrap(),
            args: vec![
                "--exact".to_string(),
                "infrastructure::managed_child::tests::managed_child_test_helper".to_string(),
                "--nocapture".to_string(),
            ],
            cwd: std::env::current_dir().unwrap(),
            env: vec![(OsString::from(HELPER_ENV), OsString::from("sleep"))],
            timeout: None,
            cancellation: CancellationToken::new(),
        })
        .unwrap();
        let pid = managed.id();
        drop(managed);
        assert!(wait_until_dead(pid, Duration::from_secs(2)));
    }

    #[test]
    fn managed_child_kills_descendants() {
        let pid_file = std::env::temp_dir().join(format!(
            "unica-managed-child-pids-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _pid_file_cleanup = FileCleanupGuard(pid_file.clone());
        let cancellation = CancellationToken::new();
        let managed = ManagedChild::spawn(ManagedCommand {
            program: std::env::current_exe().unwrap(),
            args: vec![
                "--exact".to_string(),
                "infrastructure::managed_child::tests::managed_child_test_helper".to_string(),
                "--nocapture".to_string(),
            ],
            cwd: std::env::current_dir().unwrap(),
            env: vec![
                (
                    OsString::from(HELPER_ENV),
                    OsString::from("process_tree_immediate_parent"),
                ),
                (
                    OsString::from(HELPER_PID_FILE_ENV),
                    pid_file.clone().into_os_string(),
                ),
            ],
            timeout: Some(Duration::from_secs(10)),
            cancellation: cancellation.clone(),
        })
        .unwrap();
        let mut managed_cleanup = ManagedChildCleanupGuard::new(managed, cancellation.clone());
        let pids = read_helper_pids(&pid_file, Duration::from_secs(2));
        let mut cleanup = ProcessCleanupGuard(pids.clone());
        let parent_pid = pids[0];
        let child_pid = pids[1];

        cancellation.cancel();
        let output = managed_cleanup.managed_mut().wait_for_output().unwrap();
        managed_cleanup.disarm();

        assert!(output.cancelled);
        assert!(wait_until_dead(parent_pid, Duration::from_secs(2)));
        assert!(wait_until_dead(child_pid, Duration::from_secs(2)));
        cleanup.disarm();
    }

    #[test]
    fn startup_child_cleanup_kills_descendants() {
        let pid_file = std::env::temp_dir().join(format!(
            "unica-startup-child-pids-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _pid_file_cleanup = FileCleanupGuard(pid_file.clone());
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "infrastructure::managed_child::tests::managed_child_test_helper",
                "--nocapture",
            ])
            .env(HELPER_ENV, "process_tree_immediate_parent")
            .env(HELPER_PID_FILE_ENV, &pid_file)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut startup = ManagedStartupChild::spawn_configured(command).unwrap();
        let pids = read_helper_pids(&pid_file, Duration::from_secs(2));
        let mut cleanup = ProcessCleanupGuard(pids.clone());

        startup.terminate_bounded(Duration::from_secs(2)).unwrap();

        assert!(wait_until_dead(pids[0], Duration::from_secs(2)));
        assert!(wait_until_dead(pids[1], Duration::from_secs(2)));
        cleanup.disarm();
    }

    #[test]
    fn startup_child_detach_leaves_process_running() {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "infrastructure::managed_child::tests::managed_child_test_helper",
                "--nocapture",
            ])
            .env(HELPER_ENV, "sleep")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut startup = ManagedStartupChild::spawn_configured(command).unwrap();
        let pid = startup.id();
        let _cleanup = ProcessCleanupGuard(vec![pid]);

        startup.detach().unwrap();

        thread::sleep(Duration::from_millis(75));
        assert!(process_test_support::is_alive(pid));
    }

    #[test]
    fn managed_child_preserves_thread_safe_auto_traits() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<ManagedChild>();
    }

    #[cfg(windows)]
    #[test]
    fn process_tree_keeps_child_suspended_until_attach() {
        let marker = std::env::temp_dir().join(format!(
            "unica-managed-child-marker-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _marker_cleanup = FileCleanupGuard(marker.clone());
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "infrastructure::managed_child::tests::managed_child_test_helper",
                "--nocapture",
            ])
            .env(HELPER_ENV, "write_marker")
            .env(HELPER_PID_FILE_ENV, &marker)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut process_tree = ProcessTree::prepare(&mut command).unwrap();
        let child = ChildCleanupGuard(Some(command.spawn().unwrap()));

        thread::sleep(Duration::from_millis(500));
        assert!(!marker.exists(), "child ran before process-tree attachment");

        process_tree.attach(child.child()).unwrap();
        let started = Instant::now();
        while !marker.exists() && started.elapsed() < Duration::from_secs(2) {
            thread::sleep(Duration::from_millis(25));
        }
        assert!(marker.exists(), "child did not resume after attachment");
        child.wait();
    }
}
