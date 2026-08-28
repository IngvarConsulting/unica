use crate::domain::cancellation::CancellationToken;
#[cfg(test)]
use std::cell::Cell;
use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(25);
const TERMINATION_WAIT_LIMIT: Duration = Duration::from_millis(500);
const READER_WAIT_LIMIT: Duration = Duration::from_millis(500);
pub(crate) const STDOUT_CAPTURE_LIMIT: usize = 1024 * 1024;
pub(crate) const STDERR_CAPTURE_LIMIT: usize = 256 * 1024;

#[derive(Debug)]
pub(crate) enum RuntimeProcessTreeState {
    Running,
    Exited(ExitStatus),
}

/// Retained authority for a runtime job's complete owned process tree.
///
/// Leader exit is remembered but is not terminal until the Unix process group
/// or Windows Job Object proves empty. Callers therefore cannot release a
/// workspace resource merely because `Child::try_wait` reaped the leader.
pub(crate) struct RuntimeProcessTreeHandle {
    tree: ProcessTree,
    leader_exit: Option<ExitStatus>,
    leader_exit_observed: bool,
}

#[cfg_attr(unix, allow(dead_code))]
fn runtime_process_tree_is_terminal(leader_exited: bool, owned_tree_empty: bool) -> bool {
    leader_exited && owned_tree_empty
}

#[cfg(any(test, windows))]
fn windows_job_object_is_empty(active_processes: u32) -> bool {
    active_processes == 0
}

impl RuntimeProcessTreeHandle {
    pub(crate) fn prepare(command: &mut Command) -> io::Result<Self> {
        Ok(Self {
            tree: ProcessTree::prepare_runtime(command)?,
            leader_exit: None,
            leader_exit_observed: false,
        })
    }

    pub(crate) fn attach(&mut self, child: &mut Child) -> io::Result<()> {
        if let Some(status) = self.tree.attach(child)? {
            self.leader_exit = Some(status);
            self.leader_exit_observed = true;
        }
        Ok(())
    }

    pub(crate) fn poll(&mut self, child: &mut Child) -> io::Result<RuntimeProcessTreeState> {
        #[cfg(unix)]
        {
            if let Some(status) = self.leader_exit {
                return Ok(RuntimeProcessTreeState::Exited(status));
            }
            if !self.leader_exit_observed {
                self.leader_exit_observed = self.tree.observe_leader_exit(child)?.is_some();
            }
            if !self.leader_exit_observed {
                return Ok(RuntimeProcessTreeState::Running);
            }
            if !self.tree.is_empty_except_retained_leader(child.id())? {
                return Ok(RuntimeProcessTreeState::Running);
            }
            if self.leader_exit.is_none() {
                self.leader_exit = Some(self.tree.reap_observed_leader(child)?);
            }
            Ok(RuntimeProcessTreeState::Exited(
                self.leader_exit.expect("leader exit was retained above"),
            ))
        }
        #[cfg(not(unix))]
        {
            if self.leader_exit.is_none() {
                self.leader_exit = child.try_wait()?;
            }
            let Some(status) = self.leader_exit else {
                return Ok(RuntimeProcessTreeState::Running);
            };
            if runtime_process_tree_is_terminal(true, self.tree.is_empty()?) {
                Ok(RuntimeProcessTreeState::Exited(status))
            } else {
                Ok(RuntimeProcessTreeState::Running)
            }
        }
    }

    pub(crate) fn terminate(&mut self, child: &mut Child) -> io::Result<()> {
        self.tree.terminate(child)
    }

    /// One bounded cleanup window for every partial startup state. Killing the
    /// leader as a fallback is required when Windows attachment failed before
    /// the child entered the Job Object.
    #[cfg(test)]
    pub(crate) fn terminate_and_reap_bounded(
        &mut self,
        child: &mut Child,
        budget: Duration,
    ) -> io::Result<()> {
        let started = Instant::now();
        let deadline = started.checked_add(budget).unwrap_or(started);
        self.terminate_and_reap_until(child, deadline)
    }

    /// Terminates and proves tree death inside one caller-owned absolute
    /// monotonic window. Reusing the same deadline prevents startup cleanup,
    /// output readers and Drop from each opening a fresh timeout.
    pub(crate) fn terminate_and_reap_until(
        &mut self,
        child: &mut Child,
        deadline: Instant,
    ) -> io::Result<()> {
        #[cfg(test)]
        RUNTIME_TREE_CLEANUP_CALLS.with(|slot| slot.set(slot.get().saturating_add(1)));
        #[cfg(test)]
        if INJECT_RUNTIME_TREE_CLEANUP_TIMEOUT.with(|slot| slot.replace(false)) {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "injected runtime process tree cleanup timeout",
            ));
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "runtime process tree cleanup deadline elapsed",
            ));
        }
        let tree_result = self.tree.terminate(child);
        let _ = child.kill();
        loop {
            match self.poll(child)? {
                RuntimeProcessTreeState::Exited(_) => return tree_result,
                RuntimeProcessTreeState::Running if Instant::now() >= deadline => {
                    return Err(tree_result.err().unwrap_or_else(|| {
                        io::Error::new(
                            io::ErrorKind::TimedOut,
                            "runtime process tree cleanup deadline elapsed",
                        )
                    }));
                }
                RuntimeProcessTreeState::Running => thread::sleep(
                    PROCESS_POLL_INTERVAL.min(deadline.saturating_duration_since(Instant::now())),
                ),
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn leader_exited(&self) -> bool {
        self.leader_exit_observed || self.leader_exit.is_some()
    }
}

#[cfg(unix)]
fn unix_leader_exit_unreaped(process_id: u32) -> io::Result<Option<ExitStatus>> {
    use std::mem::zeroed;
    use std::os::unix::process::ExitStatusExt;

    #[cfg(test)]
    if INJECT_UNIX_WAITID_ERROR.with(|slot| slot.replace(false)) {
        return Err(io::Error::other("injected Unix waitid failure"));
    }

    // WNOWAIT retains the exited leader as the generation authority for its
    // process group. The numeric PGID therefore cannot be recycled while a
    // descendant is still owned or cancellation may still signal the group.
    let mut information: libc::siginfo_t = unsafe { zeroed() };
    let waited = unsafe {
        libc::waitid(
            libc::P_PID,
            process_id as libc::id_t,
            &mut information,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    if waited == -1 {
        return Err(io::Error::last_os_error());
    }
    let observed = unsafe { information.si_pid() };
    if observed == 0 {
        return Ok(None);
    }
    let status = unsafe { information.si_status() };
    let raw_status = match information.si_code {
        libc::CLD_EXITED => status << 8,
        libc::CLD_KILLED => status,
        libc::CLD_DUMPED => status | 0x80,
        code => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unexpected retained leader wait status code {code}"),
            ));
        }
    };
    Ok(Some(ExitStatus::from_raw(raw_status)))
}

#[cfg(test)]
pub(crate) struct RuntimeProcessTreeTestScenario {
    program: PathBuf,
    leader_with_descendant_args: Vec<String>,
    long_lived_args: Vec<String>,
    descendant_pid_path: PathBuf,
}

#[cfg(test)]
impl RuntimeProcessTreeTestScenario {
    pub(crate) fn program(&self) -> PathBuf {
        self.program.clone()
    }

    pub(crate) fn leader_with_descendant_args(&self) -> Vec<String> {
        self.leader_with_descendant_args.clone()
    }

    pub(crate) fn long_lived_args(&self) -> Vec<String> {
        self.long_lived_args.clone()
    }

    pub(crate) fn wait_for_descendant(
        &self,
        timeout: Duration,
    ) -> io::Result<RuntimeProcessTreeTestProbe> {
        let started = Instant::now();
        loop {
            match std::fs::read_to_string(&self.descendant_pid_path) {
                Ok(value) => {
                    if let Ok(process_id) = value.trim().parse::<u32>() {
                        let probe = RuntimeProcessTreeTestProbe { process_id };
                        if probe.is_alive()? {
                            return Ok(probe);
                        }
                    }
                }
                Err(error)
                    if error.kind() == io::ErrorKind::NotFound
                        || (cfg!(windows)
                            && matches!(error.raw_os_error(), Some(32) | Some(33))) => {}
                Err(error) => return Err(error),
            }
            if started.elapsed() >= timeout {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "runtime descendant did not publish a live process id",
                ));
            }
            thread::sleep(PROCESS_POLL_INTERVAL);
        }
    }
}

#[cfg(test)]
pub(crate) struct RuntimeProcessTreeTestProbe {
    process_id: u32,
}

#[cfg(test)]
impl RuntimeProcessTreeTestProbe {
    pub(crate) fn is_alive(&self) -> io::Result<bool> {
        runtime_process_pid_alive_for_test(self.process_id)
    }
}

#[cfg(test)]
pub(crate) fn runtime_process_tree_test_scenario_for_test(
    root: &std::path::Path,
) -> RuntimeProcessTreeTestScenario {
    let descendant_pid_path = root.join("runtime-tree-descendant.pid");
    #[cfg(unix)]
    {
        let escaped_path = descendant_pid_path.to_string_lossy().replace('\'', "'\\''");
        RuntimeProcessTreeTestScenario {
            program: PathBuf::from("/bin/sh"),
            leader_with_descendant_args: vec![
                "-c".to_string(),
                format!(
                    "sleep 30 </dev/null >/dev/null 2>&1 & child=$!; printf '%s' \"$child\" > '{escaped_path}'"
                ),
            ],
            long_lived_args: vec![
                "-c".to_string(),
                format!(
                    "sleep 30 </dev/null >/dev/null 2>&1 & child=$!; printf '%s' \"$child\" > '{escaped_path}'; wait \"$child\""
                ),
            ],
            descendant_pid_path,
        }
    }
    #[cfg(windows)]
    {
        let escaped_path = descendant_pid_path.to_string_lossy().replace('\'', "''");
        let spawn = format!(
            "$p = Start-Process -PassThru -WindowStyle Hidden ping.exe -ArgumentList @('-n','20','127.0.0.1'); Set-Content -NoNewline -LiteralPath '{escaped_path}' -Value $p.Id"
        );
        RuntimeProcessTreeTestScenario {
            program: PathBuf::from("powershell.exe"),
            leader_with_descendant_args: vec![
                "-NoProfile".to_string(),
                "-NonInteractive".to_string(),
                "-Command".to_string(),
                spawn.clone(),
            ],
            long_lived_args: vec![
                "-NoProfile".to_string(),
                "-NonInteractive".to_string(),
                "-Command".to_string(),
                format!("{spawn}; Wait-Process -Id $p.Id"),
            ],
            descendant_pid_path,
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        RuntimeProcessTreeTestScenario {
            program: PathBuf::from("false"),
            leader_with_descendant_args: Vec::new(),
            long_lived_args: Vec::new(),
            descendant_pid_path,
        }
    }
}

#[cfg(all(test, unix))]
fn runtime_process_pid_alive_for_test(process_id: u32) -> io::Result<bool> {
    let process_id = i32::try_from(process_id)
        .map_err(|_| io::Error::other("process id is outside Unix pid range"))?;
    // SAFETY: signal 0 only probes the test-owned process.
    if unsafe { libc::kill(process_id, 0) } == 0 {
        return Ok(true);
    }
    let error = io::Error::last_os_error();
    match error.raw_os_error() {
        Some(libc::ESRCH) => Ok(false),
        Some(libc::EPERM) => Ok(true),
        _ => Err(error),
    }
}

#[cfg(all(test, windows))]
fn runtime_process_pid_alive_for_test(process_id: u32) -> io::Result<bool> {
    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_TIMEOUT};
    use windows_sys::Win32::System::Threading::{
        OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE,
    };

    // SAFETY: the handle is used only for a zero-time test probe and is closed
    // before returning.
    let process = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, process_id) };
    if process.is_null() {
        return Ok(false);
    }
    let alive = unsafe { WaitForSingleObject(process, 0) } == WAIT_TIMEOUT;
    unsafe {
        CloseHandle(process);
    }
    Ok(alive)
}

#[cfg(all(test, not(any(unix, windows))))]
fn runtime_process_pid_alive_for_test(_process_id: u32) -> io::Result<bool> {
    Ok(false)
}

#[cfg(all(test, windows))]
pub(crate) fn assert_windows_runtime_process_tree_semantics_for_test() -> io::Result<()> {
    let mut command = Command::new("powershell.exe");
    command
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Start-Process -WindowStyle Hidden ping.exe -ArgumentList @('-n','20','127.0.0.1') | Out-Null",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut process_tree = RuntimeProcessTreeHandle::prepare(&mut command)?;
    let mut child = command.spawn()?;
    if let Err(error) = process_tree.attach(&mut child) {
        let _ = process_tree.terminate_and_reap_bounded(&mut child, TERMINATION_WAIT_LIMIT);
        return Err(error);
    }

    let started = Instant::now();
    loop {
        match process_tree.poll(&mut child)? {
            RuntimeProcessTreeState::Running if process_tree.leader_exited() => break,
            RuntimeProcessTreeState::Running if started.elapsed() < Duration::from_secs(5) => {
                thread::sleep(PROCESS_POLL_INTERVAL)
            }
            RuntimeProcessTreeState::Running => {
                let _ = process_tree.terminate_and_reap_bounded(&mut child, TERMINATION_WAIT_LIMIT);
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "Windows runtime leader did not exit while its Job Object descendant lived",
                ));
            }
            RuntimeProcessTreeState::Exited(_) => {
                return Err(io::Error::other(
                    "Windows runtime process tree became terminal with a live descendant",
                ));
            }
        }
    }

    process_tree.terminate_and_reap_bounded(&mut child, TERMINATION_WAIT_LIMIT)
}

#[cfg(test)]
thread_local! {
    static INJECT_WAIT_ERROR: Cell<bool> = const { Cell::new(false) };
    static INJECT_RUNTIME_TREE_CLEANUP_TIMEOUT: Cell<bool> = const { Cell::new(false) };
    static RUNTIME_TREE_CLEANUP_CALLS: Cell<u32> = const { Cell::new(0) };
    #[cfg(unix)]
    static INJECT_UNIX_WAITID_ERROR: Cell<bool> = const { Cell::new(false) };
    #[cfg(unix)]
    static INJECT_UNIX_REAP_ERROR: Cell<bool> = const { Cell::new(false) };
    #[cfg(unix)]
    static UNIX_SIGNAL_COUNT: Cell<u32> = const { Cell::new(0) };
}

#[cfg(all(test, unix))]
pub(crate) fn assert_runtime_ownership_sentinel_for_test() {
    tests::permission_denied_empty_group_policy_requires_exact_platform_evidence();
    tests::runtime_ownership_writer_is_cloexec_in_the_parent();
    tests::runtime_sentinel_preserves_a_meaningful_inherited_fd198();
    tests::runtime_ownership_pipe_accepts_an_exact_quick_terminal_child();
    tests::concurrent_runtime_sentinels_do_not_cross_inherit_or_leak_to_unrelated_exec();
    tests::runner_style_new_process_group_and_closed_stdio_still_hold_sentinel_lifetime();
}

#[cfg(test)]
pub(crate) fn inject_runtime_tree_cleanup_timeout_for_test() {
    INJECT_RUNTIME_TREE_CLEANUP_TIMEOUT.with(|slot| slot.set(true));
}

#[cfg(test)]
pub(crate) fn reset_runtime_tree_cleanup_calls_for_test() {
    RUNTIME_TREE_CLEANUP_CALLS.with(|slot| slot.set(0));
}

#[cfg(test)]
pub(crate) fn runtime_tree_cleanup_calls_for_test() -> u32 {
    RUNTIME_TREE_CLEANUP_CALLS.with(Cell::get)
}

#[cfg(all(test, unix))]
pub(crate) fn inject_unix_waitid_error_for_test() {
    INJECT_UNIX_WAITID_ERROR.with(|slot| slot.set(true));
}

#[cfg(all(test, unix))]
pub(crate) fn inject_unix_reap_error_for_test() {
    INJECT_UNIX_REAP_ERROR.with(|slot| slot.set(true));
}

#[cfg(all(test, unix))]
pub(crate) fn reset_unix_signal_count_for_test() {
    UNIX_SIGNAL_COUNT.with(|slot| slot.set(0));
}

#[cfg(all(test, unix))]
pub(crate) fn unix_signal_count_for_test() -> u32 {
    UNIX_SIGNAL_COUNT.with(Cell::get)
}

#[cfg(all(test, unix))]
pub(crate) fn reap_runtime_authority_test_child(process_id: u32) {
    // SAFETY: this is a test-only cleanup for the exact child just spawned by
    // the caller. The platform facade owns both Unix calls and ignores ESRCH/
    // ECHILD because the regression may already have reaped the child.
    unsafe {
        libc::kill(process_id as i32, libc::SIGKILL);
        let mut status = 0;
        libc::waitpid(process_id as i32, &mut status, 0);
    }
}

#[derive(Debug, Clone)]
pub struct ManagedCommand {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub cwd: PathBuf,
    pub env: Vec<(OsString, OsString)>,
    pub env_remove: Vec<OsString>,
    pub capture_limits: Option<(usize, usize)>,
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
    pub stdout_had_invalid_utf8: bool,
    pub stderr_had_invalid_utf8: bool,
}

#[derive(Debug, Clone)]
pub struct ManagedLineOutput {
    pub status_success: bool,
    pub status: String,
    pub stderr: String,
    pub timed_out: bool,
    pub cancelled: bool,
    pub stopped_by_consumer: bool,
    pub line_error: Option<(usize, String)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamControl {
    Continue,
    Stop,
}

pub struct ManagedChild {
    child: Child,
    process_tree: ProcessTree,
    state: ChildState,
    timeout: Option<Duration>,
    cancellation: CancellationToken,
    capture_limits: Option<(usize, usize)>,
}

/// Owns a freshly spawned long-lived process tree until its readiness handshake
/// succeeds. Dropping this guard terminates the tree; `detach` is the only path
/// that intentionally leaves the process running.
pub(crate) struct ManagedStartupChild {
    child: Option<Child>,
    process_tree: ProcessTree,
    termination_attempted: bool,
    termination_clock: Arc<dyn StartupTerminationClock>,
    #[cfg(test)]
    termination_probe: Option<ManualStartupTerminationProbe>,
}

trait StartupTerminationClock: Send + Sync {
    fn elapsed(&self) -> Duration;
    fn sleep(&self, duration: Duration);
}

struct SystemStartupTerminationClock(Instant);

impl SystemStartupTerminationClock {
    fn new() -> Self {
        Self(Instant::now())
    }
}

impl StartupTerminationClock for SystemStartupTerminationClock {
    fn elapsed(&self) -> Duration {
        self.0.elapsed()
    }

    fn sleep(&self, duration: Duration) {
        thread::sleep(duration);
    }
}

struct StartupTerminationDeadline {
    started: Duration,
    budget: Duration,
    clock: Arc<dyn StartupTerminationClock>,
}

impl StartupTerminationDeadline {
    fn new(budget: Duration, clock: Arc<dyn StartupTerminationClock>) -> Self {
        let started = clock.elapsed();
        Self {
            started,
            budget,
            clock,
        }
    }

    fn system(budget: Duration) -> Self {
        Self::new(budget, Arc::new(SystemStartupTerminationClock::new()))
    }

    fn remaining(&self) -> Duration {
        self.budget
            .saturating_sub(self.clock.elapsed().saturating_sub(self.started))
    }

    fn is_expired(&self) -> bool {
        self.remaining().is_zero()
    }

    fn sleep_poll_interval(&self) {
        let duration = PROCESS_POLL_INTERVAL.min(self.remaining());
        if !duration.is_zero() {
            self.clock.sleep(duration);
        }
    }
}

#[cfg(test)]
#[derive(Debug, Default)]
struct ManualStartupTerminationState {
    elapsed: Duration,
    attempt_count: usize,
    cleanup_remaining_budgets: Vec<Duration>,
}

#[cfg(test)]
#[derive(Clone)]
pub(crate) struct ManualStartupTerminationProbe {
    leader_visible_after: Duration,
    state: Arc<std::sync::Mutex<ManualStartupTerminationState>>,
}

#[cfg(test)]
impl ManualStartupTerminationProbe {
    pub(crate) fn new(leader_visible_after: Duration) -> Self {
        Self {
            leader_visible_after,
            state: Arc::new(std::sync::Mutex::new(
                ManualStartupTerminationState::default(),
            )),
        }
    }

    fn record_attempt(&self) {
        self.state
            .lock()
            .expect("startup termination probe")
            .attempt_count += 1;
    }

    fn record_cleanup_remaining(&self, remaining: Duration) {
        self.state
            .lock()
            .expect("startup termination probe")
            .cleanup_remaining_budgets
            .push(remaining);
    }

    fn leader_is_visible(&self) -> bool {
        self.elapsed() >= self.leader_visible_after
    }

    pub(crate) fn elapsed(&self) -> Duration {
        self.state
            .lock()
            .expect("startup termination probe")
            .elapsed
    }

    pub(crate) fn attempt_count(&self) -> usize {
        self.state
            .lock()
            .expect("startup termination probe")
            .attempt_count
    }

    pub(crate) fn cleanup_remaining_budgets(&self) -> Vec<Duration> {
        self.state
            .lock()
            .expect("startup termination probe")
            .cleanup_remaining_budgets
            .clone()
    }
}

#[cfg(test)]
impl StartupTerminationClock for ManualStartupTerminationProbe {
    fn elapsed(&self) -> Duration {
        self.elapsed()
    }

    fn sleep(&self, duration: Duration) {
        self.state
            .lock()
            .expect("startup termination probe")
            .elapsed += duration;
        thread::yield_now();
    }
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
        process.args(&command.args).current_dir(&command.cwd);
        for name in command.env_remove {
            process.env_remove(name);
        }
        process.envs(command.env);
        Self::spawn_process_with_limits(
            process,
            command.timeout,
            command.cancellation,
            command.capture_limits,
        )
    }

    pub(crate) fn spawn_process(
        process: Command,
        timeout: Option<Duration>,
        cancellation: CancellationToken,
    ) -> Result<Self, String> {
        Self::spawn_process_with_limits(process, timeout, cancellation, None)
    }

    fn spawn_process_with_limits(
        mut process: Command,
        timeout: Option<Duration>,
        cancellation: CancellationToken,
        capture_limits: Option<(usize, usize)>,
    ) -> Result<Self, String> {
        process
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut process_tree = ProcessTree::prepare(&mut process).map_err(process_error)?;
        let mut child = process.spawn().map_err(process_error)?;
        if let Err(error) = process_tree.attach(&mut child) {
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
            capture_limits,
        })
    }

    pub fn run(command: ManagedCommand) -> Result<ManagedOutput, String> {
        let mut child = Self::spawn(command)?;
        child.wait_for_output()
    }

    pub fn run_with_input(
        command: ManagedCommand,
        input: Vec<u8>,
    ) -> Result<ManagedOutput, String> {
        let mut child = Self::spawn(command)?;
        let mut stdin = child
            .take_stdin()
            .ok_or_else(|| "process_failed: process stdin is unavailable".to_string())?;
        let writer = thread::spawn(move || stdin.write_all(&input));
        let output = child.wait_for_output();
        if output.is_err() {
            child.terminate_after_wait_error();
            // A failed wait no longer proves that every descendant released the
            // inherited stdin pipe. Joining the producer here would make this
            // error path unbounded, so preserve the wait error after best-effort
            // process-tree cleanup and detach the writer handle.
            drop(writer);
            return output;
        }
        let write_result = writer
            .join()
            .map_err(|_| "process_failed: process stdin writer panicked".to_string())?;

        match output {
            Ok(output) if output.timed_out || output.cancelled => Ok(output),
            Ok(output) => {
                write_result.map_err(process_error)?;
                Ok(output)
            }
            Err(error) => Err(error),
        }
    }

    pub fn take_stdin(&mut self) -> Option<ChildStdin> {
        self.child.stdin.take()
    }

    pub fn id(&self) -> u32 {
        self.child.id()
    }

    fn try_wait_owned_leader(&mut self) -> Result<Option<ExitStatus>, String> {
        self.process_tree
            .try_reap_leader(&mut self.child)
            .map_err(process_error)
    }

    pub fn is_running(&mut self) -> Result<bool, String> {
        match self.state {
            ChildState::Reaped | ChildState::Terminating => Ok(false),
            ChildState::Running => match self.try_wait_owned_leader()? {
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
        let (stdout_limit, stderr_limit) = self
            .capture_limits
            .unwrap_or((STDOUT_CAPTURE_LIMIT, STDERR_CAPTURE_LIMIT));
        let stdout = start_reader(self.take_stdout(), stdout_limit);
        let stderr = start_reader(self.take_stderr(), stderr_limit);
        #[cfg(test)]
        if INJECT_WAIT_ERROR.with(|slot| slot.replace(false)) {
            return Err("process_failed: injected process wait failure".to_string());
        }
        let started = Instant::now();
        let mut last_callback = Instant::now();

        loop {
            if self.cancellation.is_cancelled() {
                self.terminate_gracefully()?;
                return self.finish_after_termination(stdout, stderr, false, true);
            }
            if self.timeout.is_some_and(|limit| started.elapsed() >= limit) {
                self.terminate()?;
                return self.finish_after_termination(stdout, stderr, true, false);
            }
            if let Some(status) = self.try_wait_owned_leader()? {
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

    pub fn wait_for_line_output<F>(
        &mut self,
        max_line_bytes: usize,
        mut on_line: F,
    ) -> Result<ManagedLineOutput, String>
    where
        F: FnMut(usize, &[u8]) -> StreamControl,
    {
        drop(self.take_stdin());
        let stdout = start_line_reader(self.take_stdout(), max_line_bytes);
        let stderr = start_reader(self.take_stderr(), STDERR_CAPTURE_LIMIT);
        let started = Instant::now();
        let mut first_line_error = None;

        loop {
            if drain_line_messages(&stdout, &mut on_line, &mut first_line_error, false)
                == StreamControl::Stop
            {
                if self.cancellation.is_cancelled() {
                    self.terminate_gracefully()?;
                    drain_line_messages(
                        &stdout,
                        &mut |_, _| StreamControl::Continue,
                        &mut first_line_error,
                        true,
                    );
                    return Ok(finish_line_output(
                        None,
                        stderr,
                        false,
                        true,
                        false,
                        first_line_error,
                    ));
                }
                self.terminate()?;
                drain_line_messages(
                    &stdout,
                    &mut |_, _| StreamControl::Continue,
                    &mut first_line_error,
                    true,
                );
                return Ok(finish_line_output(
                    None,
                    stderr,
                    false,
                    false,
                    true,
                    first_line_error,
                ));
            }
            if self.cancellation.is_cancelled() {
                self.terminate_gracefully()?;
                drain_line_messages(&stdout, &mut on_line, &mut first_line_error, true);
                return Ok(finish_line_output(
                    None,
                    stderr,
                    false,
                    true,
                    false,
                    first_line_error,
                ));
            }
            if self.timeout.is_some_and(|limit| started.elapsed() >= limit) {
                self.terminate()?;
                drain_line_messages(&stdout, &mut on_line, &mut first_line_error, true);
                return Ok(finish_line_output(
                    None,
                    stderr,
                    true,
                    false,
                    false,
                    first_line_error,
                ));
            }
            if let Some(status) = self.try_wait_owned_leader()? {
                self.process_tree.cleanup_after_leader_exit(&mut self.child);
                self.state = ChildState::Reaped;
                drain_line_messages(&stdout, &mut on_line, &mut first_line_error, true);
                return Ok(finish_line_output(
                    Some(status),
                    stderr,
                    false,
                    false,
                    false,
                    first_line_error,
                ));
            }
            thread::sleep(PROCESS_POLL_INTERVAL);
        }
    }

    pub fn terminate(&mut self) -> Result<(), String> {
        if self.state == ChildState::Reaped {
            self.process_tree.cleanup_after_leader_exit(&mut self.child);
            return Ok(());
        }
        if self.state == ChildState::Running {
            if self.try_wait_owned_leader()?.is_some() {
                self.process_tree.cleanup_after_leader_exit(&mut self.child);
                self.state = ChildState::Reaped;
                return Ok(());
            }
            if let Err(error) = self.process_tree.terminate(&mut self.child) {
                if self.try_wait_owned_leader()?.is_some() {
                    self.state = ChildState::Reaped;
                    return Ok(());
                }
                return Err(process_error(error));
            }
            self.state = ChildState::Terminating;
        }
        self.reap_bounded()
    }

    fn terminate_after_wait_error(&mut self) {
        if self.state == ChildState::Reaped {
            return;
        }
        let _ = self.process_tree.terminate(&mut self.child);
        let _ = self.child.kill();
        self.state = ChildState::Terminating;
        let _ = self.reap_bounded();
    }

    fn terminate_gracefully(&mut self) -> Result<(), String> {
        if self.state == ChildState::Reaped {
            self.process_tree.cleanup_after_leader_exit(&mut self.child);
            return Ok(());
        }
        if self.state == ChildState::Running {
            if self.try_wait_owned_leader()?.is_some() {
                self.process_tree.cleanup_after_leader_exit(&mut self.child);
                self.state = ChildState::Reaped;
                return Ok(());
            }
            if let Err(error) = self
                .process_tree
                .request_graceful_termination(&mut self.child)
            {
                if self.try_wait_owned_leader()?.is_some() {
                    self.process_tree.cleanup_after_leader_exit(&mut self.child);
                    self.state = ChildState::Reaped;
                    return Ok(());
                }
                return Err(process_error(error));
            }
            self.state = ChildState::Terminating;
            self.reap_bounded()?;
        }
        if self.state == ChildState::Reaped {
            self.process_tree.cleanup_after_leader_exit(&mut self.child);
            return Ok(());
        }

        if let Err(error) = self.process_tree.terminate(&mut self.child) {
            if self.try_wait_owned_leader()?.is_some() {
                self.process_tree.cleanup_after_leader_exit(&mut self.child);
                self.state = ChildState::Reaped;
                return Ok(());
            }
            return Err(process_error(error));
        }
        self.reap_bounded()?;
        if self.state == ChildState::Reaped {
            self.process_tree.cleanup_after_leader_exit(&mut self.child);
        }
        Ok(())
    }

    fn reap_bounded(&mut self) -> Result<(), String> {
        let started = Instant::now();
        while started.elapsed() < TERMINATION_WAIT_LIMIT {
            if self.try_wait_owned_leader()?.is_some() {
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
            if let Some(status) = self.try_wait_owned_leader()? {
                self.state = ChildState::Reaped;
                return Ok(finish_output(status, stdout, stderr, timed_out, cancelled));
            }
            thread::sleep(PROCESS_POLL_INTERVAL);
        }

        let stdout = receive_output(stdout);
        let stderr = receive_output(stderr);
        let (stdout_text, stdout_had_invalid_utf8) = decode_captured_text(&stdout.bytes);
        let (stderr_text, stderr_had_invalid_utf8) = decode_captured_text(&stderr.bytes);
        let mut output = ManagedOutput {
            status_success: false,
            status: "termination pending".to_string(),
            stdout: stdout_text,
            stderr: stderr_text,
            timed_out,
            cancelled,
            stdout_truncated: stdout.truncated,
            stderr_truncated: stderr.truncated,
            stdout_had_invalid_utf8,
            stderr_had_invalid_utf8,
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
            termination_attempted: false,
            termination_clock: Arc::new(SystemStartupTerminationClock::new()),
            #[cfg(test)]
            termination_probe: None,
        };
        if let Err(error) = managed
            .process_tree
            .attach(managed.child.as_mut().expect("startup child exists"))
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

    pub(crate) fn try_wait_status(&mut self) -> Result<Option<ExitStatus>, String> {
        self.process_tree
            .try_reap_leader(self.child.as_mut().expect("startup child exists"))
            .map_err(process_error)
    }

    #[cfg(test)]
    pub(crate) fn is_running(&mut self) -> Result<bool, String> {
        self.try_wait_status().map(|status| status.is_none())
    }

    #[cfg(test)]
    pub(crate) fn install_termination_probe_for_test(
        &mut self,
        probe: &ManualStartupTerminationProbe,
    ) {
        self.termination_clock = Arc::new(probe.clone());
        self.termination_probe = Some(probe.clone());
    }

    pub(crate) fn terminate_bounded(&mut self, wait_limit: Duration) -> Result<(), String> {
        self.termination_attempted = true;
        #[cfg(test)]
        if let Some(probe) = &self.termination_probe {
            probe.record_attempt();
        }
        let deadline =
            StartupTerminationDeadline::new(wait_limit, Arc::clone(&self.termination_clock));
        if self.child.is_none() {
            return Ok(());
        }
        if self.try_wait_during_termination()?.is_some() {
            self.cleanup_after_startup_leader_exit(&deadline);
            self.child.take();
            return Ok(());
        }

        let (tree_error, child_error) = self.signal_termination_best_effort();
        while !deadline.is_expired() {
            if self.try_wait_during_termination()?.is_some() {
                self.cleanup_after_startup_leader_exit(&deadline);
                self.child.take();
                return Ok(());
            }
            deadline.sleep_poll_interval();
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

    fn try_wait_during_termination(&mut self) -> Result<Option<ExitStatus>, String> {
        #[cfg(test)]
        if let Some(probe) = &self.termination_probe {
            if !probe.leader_is_visible() {
                return Ok(None);
            }
            return self
                .child
                .as_mut()
                .expect("startup child exists")
                .wait()
                .map(Some)
                .map_err(process_error);
        }
        self.process_tree
            .try_reap_leader(self.child.as_mut().expect("startup child exists"))
            .map_err(process_error)
    }

    fn signal_termination_best_effort(&mut self) -> (Option<io::Error>, Option<io::Error>) {
        let Some(child) = self.child.as_mut() else {
            return (None, None);
        };
        let tree_error = self.process_tree.terminate(child).err();
        // Also target the leader directly. This is required when Windows Job Object
        // attachment itself failed, and is harmless after a successful tree kill.
        let child_error = child.kill().err();
        (tree_error, child_error)
    }

    fn cleanup_after_startup_leader_exit(&mut self, deadline: &StartupTerminationDeadline) {
        #[cfg(test)]
        if let Some(probe) = &self.termination_probe {
            probe.record_cleanup_remaining(deadline.remaining());
        }
        self.process_tree.cleanup_after_leader_exit_until(
            self.child.as_mut().expect("startup child exists"),
            deadline,
        );
    }

    pub(crate) fn detach(&mut self) -> Result<(), String> {
        let child = self.child.as_mut().expect("startup child exists");
        if self
            .process_tree
            .try_reap_leader(child)
            .map_err(process_error)?
            .is_some()
        {
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
        if self.termination_attempted {
            // An explicit bounded attempt already owned its one deadline. Preserve best-effort
            // signalling for an early wait error, but never start a second Drop wait window.
            let _ = self.signal_termination_best_effort();
        } else {
            let _ = self.terminate_bounded(TERMINATION_WAIT_LIMIT);
        }
    }
}

#[cfg(unix)]
struct ProcessTree {
    process_group: Option<UnixProcessGroupAuthority>,
    ownership_pipe: Option<UnixOwnershipPipe>,
    leader: UnixLeaderOwnership,
    kill_sent: bool,
}

#[cfg(unix)]
#[derive(Clone, Copy, Debug)]
enum UnixLeaderOwnership {
    Running,
    ExitedUnreaped(ExitStatus),
    Reaped(ExitStatus),
    AuthorityLost,
}

/// Darwin's EPERM-on-zombie rule is evidence only for the generic managed
/// child. Runtime jobs install a stronger descendant-lifetime sentinel and
/// therefore require its EOF. `exact_retained_leader` prevents either branch
/// from authorizing a released/recycled numeric group.
#[cfg(any(unix, test))]
fn permission_denied_proves_empty_group(
    host_is_darwin: bool,
    exact_retained_leader: bool,
    runtime_sentinel_empty: Option<bool>,
) -> bool {
    exact_retained_leader
        && match runtime_sentinel_empty {
            Some(empty) => empty,
            None => host_is_darwin,
        }
}

#[cfg(unix)]
struct UnixOwnershipPipe {
    reader: std::fs::File,
    parent_writer: Option<std::fs::File>,
}

#[cfg(all(unix, target_vendor = "apple"))]
fn atomic_cloexec_ownership_pair() -> io::Result<[i32; 2]> {
    use std::os::unix::ffi::OsStrExt;

    let mut template = std::env::temp_dir().as_os_str().as_bytes().to_vec();
    template.extend_from_slice(b"/unica-runtime-sentinel.XXXXXX\0");
    let directory_path = unsafe { libc::mkdtemp(template.as_mut_ptr().cast()) };
    if directory_path.is_null() {
        return Err(io::Error::last_os_error());
    }
    let directory = unsafe {
        libc::open(
            directory_path,
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if directory == -1 {
        let error = io::Error::last_os_error();
        unsafe { libc::rmdir(directory_path) };
        return Err(error);
    }
    let name = b"ownership\0";
    if unsafe { libc::mkfifoat(directory, name.as_ptr().cast(), 0o600) } == -1 {
        let error = io::Error::last_os_error();
        unsafe {
            libc::close(directory);
            libc::rmdir(directory_path);
        }
        return Err(error);
    }
    let reader = unsafe {
        libc::openat(
            directory,
            name.as_ptr().cast(),
            libc::O_RDONLY | libc::O_NONBLOCK | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    let writer = if reader == -1 {
        -1
    } else {
        unsafe {
            libc::openat(
                directory,
                name.as_ptr().cast(),
                libc::O_WRONLY | libc::O_NONBLOCK | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        }
    };
    let open_error = (reader == -1 || writer == -1).then(io::Error::last_os_error);
    unsafe {
        libc::unlinkat(directory, name.as_ptr().cast(), 0);
        libc::close(directory);
        libc::rmdir(directory_path);
    }
    if let Some(error) = open_error {
        if reader != -1 {
            unsafe { libc::close(reader) };
        }
        return Err(error);
    }
    Ok([reader, writer])
}

#[cfg(all(unix, not(target_vendor = "apple")))]
fn atomic_cloexec_ownership_pair() -> io::Result<[i32; 2]> {
    let mut descriptors = [-1_i32; 2];
    if unsafe { libc::pipe2(descriptors.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) } == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(descriptors)
    }
}

#[cfg(unix)]
impl UnixOwnershipPipe {
    fn install(command: &mut Command) -> io::Result<Self> {
        use std::os::fd::{AsRawFd, FromRawFd};
        use std::os::unix::process::CommandExt;

        let descriptors = atomic_cloexec_ownership_pair()?;
        let reader = unsafe { std::fs::File::from_raw_fd(descriptors[0]) };
        let writer = unsafe { std::fs::File::from_raw_fd(descriptors[1]) };
        let writer_fd = writer.as_raw_fd();
        // SAFETY: fcntl is async-signal-safe. The parent keeps this exact
        // descriptor CLOEXEC at all times; only this child clears the flag on
        // the descriptor it actually inherited. No fixed target is overwritten.
        unsafe {
            command.pre_exec(move || {
                let flags = libc::fcntl(writer_fd, libc::F_GETFD);
                if flags == -1
                    || libc::fcntl(writer_fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) == -1
                {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        command.env("UNICA_RUNTIME_TREE_OWNERSHIP_FD", writer_fd.to_string());
        Ok(Self {
            reader,
            parent_writer: Some(writer),
        })
    }

    fn close_parent_writer_and_is_empty(&mut self) -> io::Result<bool> {
        self.parent_writer.take();
        self.is_empty()
    }

    fn is_empty(&self) -> io::Result<bool> {
        use std::os::fd::AsRawFd;

        let mut byte = [0_u8; 1];
        let read = unsafe { libc::read(self.reader.as_raw_fd(), byte.as_mut_ptr().cast(), 1) };
        if read == 0 {
            return Ok(true);
        }
        if read == -1 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::WouldBlock {
                return Ok(false);
            }
            return Err(error);
        }
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "runtime ownership capability carried unexpected bytes",
        ))
    }
}

#[cfg(unix)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UnixGroupGeneration {
    RetainedLeader,
    Released,
}

#[cfg(unix)]
struct UnixProcessGroupAuthority {
    pgid: i32,
    leader_pid: u32,
    generation: UnixGroupGeneration,
}

#[cfg(unix)]
impl UnixProcessGroupAuthority {
    fn retained(leader_pid: u32) -> Self {
        Self {
            pgid: leader_pid as i32,
            leader_pid,
            generation: UnixGroupGeneration::RetainedLeader,
        }
    }

    #[cfg(test)]
    fn released_for_test(pgid: i32) -> Self {
        Self {
            pgid,
            leader_pid: pgid as u32,
            generation: UnixGroupGeneration::Released,
        }
    }

    fn signal(&self, signal: i32) -> io::Result<()> {
        self.signal_with(signal, |group, signal| {
            #[cfg(test)]
            UNIX_SIGNAL_COUNT.with(|slot| slot.set(slot.get().saturating_add(1)));
            if unsafe { libc::kill(group, signal) } == -1 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        })
    }

    fn signal_with(
        &self,
        signal: i32,
        send: impl FnOnce(i32, i32) -> io::Result<()>,
    ) -> io::Result<()> {
        if self.generation != UnixGroupGeneration::RetainedLeader {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "Unix process-group generation authority was released",
            ));
        }
        send(-self.pgid, signal)
    }

    #[cfg(test)]
    fn signal_with_for_test(
        &self,
        signal: i32,
        send: impl FnOnce(i32, i32) -> io::Result<()>,
    ) -> io::Result<()> {
        self.signal_with(signal, send)
    }

    fn release(&mut self) {
        self.generation = UnixGroupGeneration::Released;
    }
}

#[cfg(all(test, unix))]
pub(crate) fn assert_released_unix_group_never_signals_reused_identity_for_test() {
    let signals = std::cell::Cell::new(0_u32);
    let authority = UnixProcessGroupAuthority::released_for_test(41_337);
    let error = authority
        .signal_with_for_test(libc::SIGKILL, |_group, _signal| {
            signals.set(signals.get().saturating_add(1));
            Ok(())
        })
        .expect_err("a released generation must fail closed");
    assert_eq!(signals.get(), 0, "foreign reused group was signalled");
    assert_eq!(error.kind(), io::ErrorKind::NotFound);
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
            ownership_pipe: None,
            leader: UnixLeaderOwnership::Running,
            kill_sent: false,
        })
    }

    fn prepare_runtime(command: &mut Command) -> io::Result<Self> {
        let mut tree = Self::prepare(command)?;
        tree.ownership_pipe = Some(UnixOwnershipPipe::install(command)?);
        Ok(tree)
    }

    fn prepare_detachable(command: &mut Command) -> io::Result<Self> {
        Self::prepare(command)
    }

    fn attach(&mut self, child: &mut Child) -> io::Result<Option<ExitStatus>> {
        self.process_group = Some(UnixProcessGroupAuthority::retained(child.id()));
        self.leader = UnixLeaderOwnership::Running;
        self.kill_sent = false;
        if let Some(ownership_pipe) = self.ownership_pipe.as_mut() {
            if ownership_pipe.close_parent_writer_and_is_empty()? {
                if self.observe_leader_exit(child)?.is_some() {
                    return self.reap_observed_leader(child).map(Some);
                }
                self.lose_authority();
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "runtime child did not retain its ownership capability",
                ));
            }
        }
        Ok(None)
    }

    fn lose_authority(&mut self) {
        if let Some(mut authority) = self.process_group.take() {
            authority.release();
        }
        self.ownership_pipe = None;
        self.kill_sent = true;
        self.leader = UnixLeaderOwnership::AuthorityLost;
    }

    fn signalable_authority(&self) -> io::Result<Option<&UnixProcessGroupAuthority>> {
        match self.leader {
            UnixLeaderOwnership::Running | UnixLeaderOwnership::ExitedUnreaped(_) => {
                self.process_group.as_ref().map(Some).ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::NotFound,
                        "Unix process-group authority is absent",
                    )
                })
            }
            UnixLeaderOwnership::Reaped(_) => Ok(None),
            UnixLeaderOwnership::AuthorityLost => Err(io::Error::new(
                io::ErrorKind::NotFound,
                "Unix process-group generation authority was lost",
            )),
        }
    }

    fn terminate(&mut self, _child: &mut Child) -> io::Result<()> {
        let Some(authority) = self.signalable_authority()? else {
            return Ok(());
        };
        if self.kill_sent {
            return Ok(());
        }
        if let Err(error) = authority.signal(libc::SIGKILL) {
            if error.raw_os_error() == Some(libc::ESRCH) {
                return Ok(());
            }
            return Err(error);
        }
        self.kill_sent = true;
        Ok(())
    }

    fn request_graceful_termination(&mut self, _child: &mut Child) -> io::Result<()> {
        let Some(authority) = self.signalable_authority()? else {
            return Ok(());
        };
        if self.kill_sent {
            return Ok(());
        }
        // Give v8-runner a chance to observe SIGTERM and clean up the separately
        // grouped 1C client before the bounded SIGKILL fallback.
        if let Err(error) = authority.signal(libc::SIGTERM) {
            if error.raw_os_error() == Some(libc::ESRCH) {
                return Ok(());
            }
            return Err(error);
        }
        Ok(())
    }

    fn try_reap_leader(&mut self, child: &mut Child) -> io::Result<Option<ExitStatus>> {
        if self.observe_leader_exit(child)?.is_none() {
            return Ok(None);
        }

        // The zombie leader still pins this exact process-group generation.
        // Deliver the one terminal group signal before reaping it; afterwards
        // the numeric PGID is discarded and can never be probed or signalled.
        if let Err(error) = self.terminate(child) {
            // macOS reports EPERM when the retained zombie leader is the only
            // remaining group member: there is no signalable process left.
            // Descendants owned by this process remain signalable and take the
            // successful path (covered by the real detached-descendant test).
            let exact_retained_leader = self.retains_exact_exited_leader(child.id());
            let runtime_sentinel_empty = self
                .ownership_pipe
                .as_ref()
                .map(|pipe| pipe.is_empty().unwrap_or(false));
            // Darwin reports EPERM for a group containing only its retained
            // zombie leader. That host-specific rule is sufficient for the
            // generic ManagedChild path, which has no descendant-lifetime
            // sentinel. Runtime jobs are stronger: if they installed a
            // sentinel, EOF is additionally required before EPERM can mean
            // empty. Other hosts never turn EPERM into success here.
            let exact_empty_group = error.kind() == io::ErrorKind::PermissionDenied
                && permission_denied_proves_empty_group(
                    cfg!(target_vendor = "apple"),
                    exact_retained_leader,
                    runtime_sentinel_empty,
                );
            if !exact_empty_group {
                return Err(error);
            }
        }
        self.reap_observed_leader(child).map(Some)
    }

    fn retains_exact_exited_leader(&self, leader_pid: u32) -> bool {
        matches!(self.leader, UnixLeaderOwnership::ExitedUnreaped(_))
            && self.process_group.as_ref().is_some_and(|authority| {
                authority.leader_pid == leader_pid
                    && authority.generation == UnixGroupGeneration::RetainedLeader
            })
    }

    fn observe_leader_exit(&mut self, child: &Child) -> io::Result<Option<ExitStatus>> {
        match self.leader {
            UnixLeaderOwnership::Running => match unix_leader_exit_unreaped(child.id()) {
                Ok(Some(status)) => {
                    self.leader = UnixLeaderOwnership::ExitedUnreaped(status);
                    Ok(Some(status))
                }
                Ok(None) => Ok(None),
                Err(error) => {
                    self.lose_authority();
                    Err(error)
                }
            },
            UnixLeaderOwnership::ExitedUnreaped(status) | UnixLeaderOwnership::Reaped(status) => {
                Ok(Some(status))
            }
            UnixLeaderOwnership::AuthorityLost => Err(io::Error::new(
                io::ErrorKind::NotFound,
                "Unix leader generation authority was lost",
            )),
        }
    }

    fn reap_observed_leader(&mut self, child: &mut Child) -> io::Result<ExitStatus> {
        let observed_status = match self.leader {
            UnixLeaderOwnership::ExitedUnreaped(status) => status,
            UnixLeaderOwnership::Reaped(status) => return Ok(status),
            UnixLeaderOwnership::Running => {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "Unix leader has not exited",
                ));
            }
            UnixLeaderOwnership::AuthorityLost => {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "Unix leader generation authority was lost",
                ));
            }
        };
        #[cfg(test)]
        if INJECT_UNIX_REAP_ERROR.with(|slot| slot.replace(false)) {
            self.lose_authority();
            return Err(io::Error::other("injected Unix reap failure"));
        }
        let status = match child.try_wait() {
            Ok(Some(status)) => status,
            Err(error) if error.raw_os_error() == Some(libc::ECHILD) => observed_status,
            Ok(None) => {
                self.lose_authority();
                return Err(io::Error::other(
                    "retained managed-process leader could not be reaped",
                ));
            }
            Err(error) => {
                self.lose_authority();
                return Err(error);
            }
        };
        if let Some(mut authority) = self.process_group.take() {
            authority.release();
        }
        self.ownership_pipe = None;
        self.leader = UnixLeaderOwnership::Reaped(status);
        Ok(status)
    }

    fn is_empty_except_retained_leader(&mut self, leader_pid: u32) -> io::Result<bool> {
        let authority = self.process_group.as_ref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "runtime process-group authority is absent",
            )
        })?;
        if authority.leader_pid != leader_pid {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "runtime process-group leader identity changed",
            ));
        }
        let ownership_pipe = self.ownership_pipe.as_ref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::Unsupported,
                "runtime child lacks the bundled-runner ownership capability",
            )
        })?;
        ownership_pipe.is_empty()
    }

    fn cleanup_after_leader_exit(&mut self, child: &mut Child) {
        let deadline = StartupTerminationDeadline::system(TERMINATION_WAIT_LIMIT);
        self.cleanup_after_leader_exit_until(child, &deadline);
    }

    fn cleanup_after_leader_exit_until(
        &mut self,
        _child: &mut Child,
        _deadline: &StartupTerminationDeadline,
    ) {
        // Generic managed-child paths have already reaped the leader before
        // entering this hook. Numeric group identity is no longer authority at
        // that point, so fail closed instead of signalling a potentially reused
        // group. Runtime jobs use `RuntimeProcessTreeHandle`, which retains the
        // leader with WNOWAIT until all descendants are accounted for.
        if !matches!(self.leader, UnixLeaderOwnership::Reaped(_)) {
            self.lose_authority();
        } else {
            if let Some(mut authority) = self.process_group.take() {
                authority.release();
            }
            self.ownership_pipe = None;
        }
    }

    fn detach(&mut self) -> io::Result<()> {
        self.lose_authority();
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

    fn prepare_runtime(command: &mut Command) -> io::Result<Self> {
        Self::prepare(command)
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

    fn attach(&mut self, child: &mut Child) -> io::Result<Option<ExitStatus>> {
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
        Ok(None)
    }

    fn terminate(&mut self, _child: &mut Child) -> io::Result<()> {
        use windows_sys::Win32::System::JobObjects::TerminateJobObject;

        // SAFETY: `self.job` remains live until `Drop`.
        if unsafe { TerminateJobObject(self.job, 1) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    fn request_graceful_termination(&mut self, child: &mut Child) -> io::Result<()> {
        self.terminate(child)
    }

    fn try_reap_leader(&mut self, child: &mut Child) -> io::Result<Option<ExitStatus>> {
        child.try_wait()
    }

    fn is_empty(&mut self) -> io::Result<bool> {
        use std::mem::{size_of, zeroed};
        use windows_sys::Win32::System::JobObjects::{
            JobObjectBasicAccountingInformation, QueryInformationJobObject,
            JOBOBJECT_BASIC_ACCOUNTING_INFORMATION,
        };

        // SAFETY: this POD is valid when zeroed and the live Job Object handle
        // and exact structure size are supplied to the query.
        let mut information: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { zeroed() };
        let queried = unsafe {
            QueryInformationJobObject(
                self.job,
                JobObjectBasicAccountingInformation,
                &mut information as *mut _ as *mut _,
                size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                std::ptr::null_mut(),
            )
        };
        if queried == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(windows_job_object_is_empty(information.ActiveProcesses))
    }

    fn cleanup_after_leader_exit(&mut self, child: &mut Child) {
        let deadline = StartupTerminationDeadline::system(TERMINATION_WAIT_LIMIT);
        self.cleanup_after_leader_exit_until(child, &deadline);
    }

    fn cleanup_after_leader_exit_until(
        &mut self,
        child: &mut Child,
        _deadline: &StartupTerminationDeadline,
    ) {
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

    fn prepare_runtime(command: &mut Command) -> io::Result<Self> {
        Self::prepare(command)
    }

    fn prepare_detachable(command: &mut Command) -> io::Result<Self> {
        Self::prepare(command)
    }

    fn attach(&mut self, _child: &mut Child) -> io::Result<Option<ExitStatus>> {
        Ok(None)
    }

    fn terminate(&mut self, child: &mut Child) -> io::Result<()> {
        child.kill()
    }

    fn request_graceful_termination(&mut self, child: &mut Child) -> io::Result<()> {
        self.terminate(child)
    }

    fn try_reap_leader(&mut self, child: &mut Child) -> io::Result<Option<ExitStatus>> {
        child.try_wait()
    }

    fn is_empty(&mut self) -> io::Result<bool> {
        Ok(true)
    }

    fn cleanup_after_leader_exit(&mut self, child: &mut Child) {
        let deadline = StartupTerminationDeadline::system(TERMINATION_WAIT_LIMIT);
        self.cleanup_after_leader_exit_until(child, &deadline);
    }

    fn cleanup_after_leader_exit_until(
        &mut self,
        child: &mut Child,
        _deadline: &StartupTerminationDeadline,
    ) {
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

enum LineMessage {
    Line(usize, Vec<u8>),
    TooLong(usize),
    ReadError(usize, String),
}

fn start_line_reader<R>(pipe: Option<R>, max_line_bytes: usize) -> Option<Receiver<LineMessage>>
where
    R: Read + Send + 'static,
{
    pipe.map(|mut pipe| {
        let (sender, receiver) = mpsc::sync_channel(1);
        thread::spawn(move || {
            let mut chunk = [0_u8; 8192];
            let mut line = Vec::new();
            let mut line_number = 1usize;
            let mut too_long = false;
            loop {
                let count = match pipe.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(count) => count,
                    Err(error) => {
                        let _ = sender.send(LineMessage::ReadError(line_number, error.to_string()));
                        return;
                    }
                };
                for byte in &chunk[..count] {
                    if *byte == b'\n' {
                        let message = if too_long {
                            LineMessage::TooLong(line_number)
                        } else {
                            LineMessage::Line(line_number, std::mem::take(&mut line))
                        };
                        if sender.send(message).is_err() {
                            return;
                        }
                        line.clear();
                        too_long = false;
                        line_number = line_number.saturating_add(1);
                    } else if !too_long {
                        if line.len() == max_line_bytes {
                            line.clear();
                            too_long = true;
                        } else {
                            line.push(*byte);
                        }
                    }
                }
            }
            if too_long {
                let _ = sender.send(LineMessage::TooLong(line_number));
            } else if !line.is_empty() {
                let _ = sender.send(LineMessage::Line(line_number, line));
            }
        });
        receiver
    })
}

fn drain_line_messages<F>(
    receiver: &Option<Receiver<LineMessage>>,
    on_line: &mut F,
    first_error: &mut Option<(usize, String)>,
    wait_for_end: bool,
) -> StreamControl
where
    F: FnMut(usize, &[u8]) -> StreamControl,
{
    let Some(receiver) = receiver else {
        return StreamControl::Continue;
    };
    loop {
        let message = if wait_for_end {
            receiver.recv_timeout(READER_WAIT_LIMIT).ok()
        } else {
            receiver.try_recv().ok()
        };
        let Some(message) = message else {
            break;
        };
        match message {
            LineMessage::Line(number, bytes) => {
                if on_line(number, &bytes) == StreamControl::Stop {
                    return StreamControl::Stop;
                }
            }
            LineMessage::TooLong(number) => {
                first_error.get_or_insert_with(|| {
                    (number, "line exceeds configured byte limit".to_string())
                });
            }
            LineMessage::ReadError(number, error) => {
                first_error.get_or_insert_with(|| {
                    (number, format!("failed to read process stdout: {error}"))
                });
            }
        }
    }
    StreamControl::Continue
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
    let mut stderr = receive_output(stderr);
    if stderr.truncated {
        retain_tail(
            &mut stderr,
            b"\n[unica: stderr capture truncated; earlier stderr diagnostics omitted]\n",
            STDERR_CAPTURE_LIMIT,
        );
    }
    let (stdout_text, stdout_had_invalid_utf8) = decode_captured_text(&stdout.bytes);
    let (stderr_text, stderr_had_invalid_utf8) = decode_captured_text(&stderr.bytes);
    let mut output = ManagedOutput {
        status_success: status.success() && !stdout.truncated && !cancelled && !timed_out,
        status: status.to_string(),
        stdout: stdout_text,
        stderr: stderr_text,
        timed_out,
        cancelled,
        stdout_truncated: stdout.truncated,
        stderr_truncated: stderr.truncated,
        stdout_had_invalid_utf8,
        stderr_had_invalid_utf8,
    };
    ensure_truncation_diagnostics(&mut output);
    output
}

fn decode_captured_text(bytes: &[u8]) -> (String, bool) {
    match String::from_utf8(bytes.to_vec()) {
        Ok(text) => (text, false),
        Err(error) => (String::from_utf8_lossy(error.as_bytes()).into_owned(), true),
    }
}

fn finish_line_output(
    status: Option<ExitStatus>,
    stderr: Option<Receiver<CapturedOutput>>,
    timed_out: bool,
    cancelled: bool,
    stopped_by_consumer: bool,
    line_error: Option<(usize, String)>,
) -> ManagedLineOutput {
    let stderr = receive_output(stderr);
    ManagedLineOutput {
        status_success: status.is_some_and(|status| status.success())
            && !timed_out
            && !cancelled
            && !stopped_by_consumer,
        status: status.map_or_else(
            || {
                if cancelled {
                    "cancelled".to_string()
                } else if timed_out {
                    "timeout".to_string()
                } else if stopped_by_consumer {
                    "stopped by consumer".to_string()
                } else {
                    "termination pending".to_string()
                }
            },
            |status| status.to_string(),
        ),
        stderr: String::from_utf8_lossy(&stderr.bytes).into_owned(),
        timed_out,
        cancelled,
        stopped_by_consumer,
        line_error,
    }
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
    use super::{assert_windows_runtime_process_tree_semantics_for_test, ProcessTree};
    use super::{
        permission_denied_proves_empty_group, runtime_process_tree_is_terminal,
        windows_job_object_is_empty, ChildState, ManagedChild, ManagedCommand, ManagedOutput,
        ManagedStartupChild, ManualStartupTerminationProbe, StreamControl,
    };
    use crate::domain::cancellation::CancellationToken;
    use std::ffi::OsString;
    use std::io::{Read, Write};
    use std::path::{Path, PathBuf};
    #[cfg(windows)]
    use std::process::Child;
    use std::process::{Command, Stdio};
    #[cfg(unix)]
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    const HELPER_ENV: &str = "UNICA_MANAGED_CHILD_HELPER";
    const HELPER_PID_FILE_ENV: &str = "UNICA_MANAGED_CHILD_PID_FILE";
    #[cfg(unix)]
    const RUNTIME_SENTINEL_HELPER_ENV: &str = "UNICA_RUNTIME_SENTINEL_HELPER";
    #[cfg(unix)]
    const RUNTIME_SENTINEL_PID_FILE_ENV: &str = "UNICA_RUNTIME_SENTINEL_PID_FILE";
    #[cfg(unix)]
    const RUNTIME_SENTINEL_FD198_MARKER_ENV: &str = "UNICA_RUNTIME_SENTINEL_FD198_MARKER";
    #[cfg(unix)]
    const RUNTIME_SENTINEL_FD198_HARNESS_ENV: &str = "UNICA_RUNTIME_SENTINEL_FD198_HARNESS";

    fn with_wait_error<T>(action: impl FnOnce() -> T) -> T {
        struct Reset(bool);

        impl Drop for Reset {
            fn drop(&mut self) {
                super::INJECT_WAIT_ERROR.with(|slot| slot.set(self.0));
            }
        }

        let previous = super::INJECT_WAIT_ERROR.with(|slot| slot.replace(true));
        let _reset = Reset(previous);
        action()
    }

    #[cfg(unix)]
    static HELPER_SIGTERM_RECEIVED: AtomicBool = AtomicBool::new(false);

    #[cfg(unix)]
    extern "C" fn record_helper_sigterm(_signal: libc::c_int) {
        HELPER_SIGTERM_RECEIVED.store(true, Ordering::SeqCst);
    }

    #[cfg(unix)]
    fn install_helper_sigterm_handler() {
        let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
        action.sa_sigaction = record_helper_sigterm as *const () as usize;
        unsafe {
            libc::sigemptyset(&mut action.sa_mask);
        }
        let result = unsafe { libc::sigaction(libc::SIGTERM, &action, std::ptr::null_mut()) };
        assert_eq!(
            result,
            0,
            "install SIGTERM handler: {}",
            std::io::Error::last_os_error()
        );
    }

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
            "echo_stdin_len" => {
                let mut input = Vec::new();
                std::io::stdin().read_to_end(&mut input).unwrap();
                print!(
                    "bytes={} nul={}",
                    input.len(),
                    input.iter().filter(|byte| **byte == 0).count()
                );
            }
            "write_invalid_utf8" => {
                std::io::stdout().write_all(b"ok\xffend").unwrap();
                std::io::stderr().write_all(b"err\xffend").unwrap();
            }
            "write_literal_replacement" => print!("ok\u{fffd}end"),
            "print_removed_env" => print!(
                "{}",
                std::env::var("PATH").unwrap_or_else(|_| "missing".into())
            ),
            "sleep" => thread::sleep(Duration::from_secs(10)),
            "stream_forever" => loop {
                println!("streamed line");
                std::io::Write::flush(&mut std::io::stdout()).unwrap();
                thread::sleep(Duration::from_millis(5));
            },
            "process_tree_immediate_parent" => {
                let pid_file = std::env::var_os(HELPER_PID_FILE_ENV).unwrap();
                let mut child = Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "infrastructure::platform::process::tests::managed_child_test_helper",
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
            "inherited_pipe_parent" => {
                let pid_file = std::env::var_os(HELPER_PID_FILE_ENV).unwrap();
                let child = Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "infrastructure::platform::process::tests::managed_child_test_helper",
                        "--nocapture",
                    ])
                    .env(HELPER_ENV, "process_tree_child")
                    .spawn()
                    .unwrap();
                print!("inherited-pipe-before-timeout");
                std::io::Write::flush(&mut std::io::stdout()).unwrap();
                std::fs::write(
                    pid_file,
                    format!("{}\n{}\n", std::process::id(), child.id()),
                )
                .unwrap();
                thread::sleep(Duration::from_secs(10));
            }
            "process_tree_child" => thread::sleep(Duration::from_secs(10)),
            #[cfg(unix)]
            "graceful_runner_with_external_process_group" => {
                use std::os::unix::process::CommandExt;

                install_helper_sigterm_handler();
                let pid_file = std::env::var_os(HELPER_PID_FILE_ENV).unwrap();
                let mut command = Command::new(std::env::current_exe().unwrap());
                command
                    .args([
                        "--exact",
                        "infrastructure::platform::process::tests::managed_child_test_helper",
                        "--nocapture",
                    ])
                    .env(HELPER_ENV, "process_tree_child")
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null());
                unsafe {
                    command.pre_exec(|| {
                        if libc::setpgid(0, 0) == -1 {
                            return Err(std::io::Error::last_os_error());
                        }
                        Ok(())
                    });
                }
                let mut external_child = command.spawn().unwrap();
                std::fs::write(
                    pid_file,
                    format!("{}\n{}\n", std::process::id(), external_child.id()),
                )
                .unwrap();

                loop {
                    if HELPER_SIGTERM_RECEIVED.load(Ordering::SeqCst) {
                        unsafe {
                            libc::kill(-(external_child.id() as i32), libc::SIGKILL);
                        }
                        external_child.wait().unwrap();
                        break;
                    }
                    if external_child.try_wait().unwrap().is_some() {
                        break;
                    }
                    thread::sleep(Duration::from_millis(10));
                }
            }
            "process_tree_detached_leader" => {
                let pid_file = std::env::var_os(HELPER_PID_FILE_ENV).unwrap();
                let child = Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "infrastructure::platform::process::tests::managed_child_test_helper",
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
        fn child_mut(&mut self) -> &mut Child {
            self.0.as_mut().unwrap()
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
                "--exact".into(),
                "infrastructure::platform::process::tests::managed_child_test_helper".into(),
                "--nocapture".into(),
            ],
            cwd: std::env::current_dir().map_err(|error| error.to_string())?,
            env: vec![(OsString::from(HELPER_ENV), OsString::from(mode))],
            env_remove: Vec::new(),
            capture_limits: None,
            timeout: Some(timeout),
            cancellation,
        })
    }

    fn run_helper_with_input(
        mode: &str,
        input: Vec<u8>,
        timeout: Duration,
        cancellation: CancellationToken,
    ) -> Result<ManagedOutput, String> {
        ManagedChild::run_with_input(
            ManagedCommand {
                program: std::env::current_exe().map_err(|error| error.to_string())?,
                args: vec![
                    "--exact".into(),
                    "infrastructure::platform::process::tests::managed_child_test_helper".into(),
                    "--nocapture".into(),
                ],
                cwd: std::env::current_dir().map_err(|error| error.to_string())?,
                env: vec![(OsString::from(HELPER_ENV), OsString::from(mode))],
                env_remove: Vec::new(),
                capture_limits: None,
                timeout: Some(timeout),
                cancellation,
            },
            input,
        )
    }

    #[test]
    fn managed_child_writes_binary_stdin_and_closes_it() {
        let input = b"src/.build/probe\0src/ConfigDumpInfo.xml\0".to_vec();
        let expected_bytes = input.len();

        let output = run_helper_with_input(
            "echo_stdin_len",
            input,
            Duration::from_secs(2),
            CancellationToken::new(),
        )
        .unwrap();

        assert!(output.status_success, "{}", output.status);
        assert!(
            output.stdout.contains(&format!("bytes={expected_bytes}")),
            "{}",
            output.stdout
        );
        assert!(output.stdout.contains("nul=2"), "{}", output.stdout);
    }

    #[test]
    fn runtime_tree_policy_keeps_windows_job_or_unix_group_running_after_leader_exit() {
        assert!(!runtime_process_tree_is_terminal(false, false));
        assert!(!runtime_process_tree_is_terminal(false, true));
        assert!(!runtime_process_tree_is_terminal(true, false));
        assert!(runtime_process_tree_is_terminal(true, true));
    }

    #[test]
    fn runtime_tree_windows_job_accounting_retains_descendant_after_leader_exit() {
        assert!(!windows_job_object_is_empty(2));
        assert!(!windows_job_object_is_empty(1));
        assert!(windows_job_object_is_empty(0));
        assert!(!runtime_process_tree_is_terminal(
            true,
            windows_job_object_is_empty(1)
        ));
        assert!(runtime_process_tree_is_terminal(
            true,
            windows_job_object_is_empty(0)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn released_unix_group_generation_never_signals_a_reused_numeric_pgid() {
        super::assert_released_unix_group_never_signals_reused_identity_for_test();
    }

    #[cfg(unix)]
    #[test]
    fn authority_lost_process_tree_never_signals_a_reused_numeric_group() {
        let mut command = Command::new("/bin/sh");
        command.arg("-c").arg("exit 0");
        let mut tree = super::ProcessTree::prepare(&mut command).unwrap();
        let mut child = command.spawn().unwrap();
        tree.attach(&mut child).unwrap();
        child.wait().unwrap();
        tree.observe_leader_exit(&child)
            .expect_err("externally reaped leader loses generation authority");
        let signals = std::cell::Cell::new(0_u32);
        if let Some(authority) = tree.process_group.as_ref() {
            let _ = authority.signal_with_for_test(libc::SIGKILL, |_group, _signal| {
                signals.set(signals.get().saturating_add(1));
                Ok(())
            });
        }
        assert_eq!(
            signals.get(),
            0,
            "AuthorityLost retained a signalable numeric process group"
        );
    }

    #[test]
    pub(super) fn permission_denied_empty_group_policy_requires_exact_platform_evidence() {
        assert!(permission_denied_proves_empty_group(true, true, None));
        assert!(!permission_denied_proves_empty_group(
            true,
            true,
            Some(false)
        ));
        assert!(permission_denied_proves_empty_group(true, true, Some(true)));
        assert!(!permission_denied_proves_empty_group(false, true, None));
        assert!(!permission_denied_proves_empty_group(true, false, None));
        assert!(!permission_denied_proves_empty_group(
            true,
            false,
            Some(true)
        ));
    }

    #[cfg(unix)]
    #[test]
    pub(super) fn runtime_ownership_writer_is_cloexec_in_the_parent() {
        use std::os::fd::AsRawFd;

        let mut command = Command::new("/usr/bin/true");
        let pipe = super::UnixOwnershipPipe::install(&mut command).unwrap();
        let writer = pipe
            .parent_writer
            .as_ref()
            .expect("parent retains writer until spawn");
        let flags = unsafe { libc::fcntl(writer.as_raw_fd(), libc::F_GETFD) };
        assert_ne!(flags, -1);
        assert_ne!(
            flags & libc::FD_CLOEXEC,
            0,
            "ownership writer is globally inheritable by unrelated concurrent spawns"
        );
    }

    #[cfg(unix)]
    #[test]
    #[allow(clippy::zombie_processes)]
    // The parent test owns and reaps the detached descendant after observing
    // sentinel-only lifetime; waiting here would erase the tested boundary.
    fn runtime_preexisting_fd198_helper() {
        use std::os::fd::{FromRawFd, IntoRawFd};
        use std::os::unix::fs::MetadataExt;
        use std::os::unix::process::CommandExt;

        let Some(marker) = std::env::var_os(RUNTIME_SENTINEL_FD198_MARKER_ENV) else {
            return;
        };
        let mut inherited = unsafe { std::fs::File::from_raw_fd(198) };
        let metadata = inherited.metadata().unwrap();
        let mut bytes = Vec::new();
        inherited.read_to_end(&mut bytes).unwrap();
        let _ = inherited.into_raw_fd();
        let mut descendant = Command::new("/bin/sleep");
        descendant
            .arg("30")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        unsafe {
            descendant.pre_exec(|| {
                if libc::setpgid(0, 0) == -1 {
                    Err(std::io::Error::last_os_error())
                } else {
                    Ok(())
                }
            });
        }
        let descendant = descendant.spawn().unwrap();
        std::fs::write(
            marker,
            format!(
                "{}:{}:{}\n{}",
                metadata.dev(),
                metadata.ino(),
                String::from_utf8(bytes).unwrap(),
                descendant.id()
            ),
        )
        .unwrap();
    }

    #[cfg(unix)]
    #[test]
    pub(super) fn runtime_sentinel_preserves_a_meaningful_inherited_fd198() {
        use std::os::fd::FromRawFd;
        use std::os::unix::fs::MetadataExt;

        if std::env::var_os(RUNTIME_SENTINEL_FD198_HARNESS_ENV).is_none() {
            let status = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "infrastructure::platform::process::tests::runtime_sentinel_preserves_a_meaningful_inherited_fd198",
                    "--nocapture",
                ])
                .env(RUNTIME_SENTINEL_FD198_HARNESS_ENV, "1")
                .status()
                .unwrap();
            assert!(status.success(), "isolated inherited-fd harness failed");
            return;
        }

        struct RestoreFd198 {
            saved: Option<i32>,
            flags: Option<i32>,
        }
        impl Drop for RestoreFd198 {
            fn drop(&mut self) {
                match self.saved {
                    Some(saved) => unsafe {
                        libc::dup2(saved, 198);
                        if let Some(flags) = self.flags {
                            libc::fcntl(198, libc::F_SETFD, flags);
                        }
                        libc::close(saved);
                    },
                    None => unsafe {
                        libc::close(198);
                    },
                };
            }
        }

        let original_flags = unsafe { libc::fcntl(198, libc::F_GETFD) };
        let saved = unsafe { libc::fcntl(198, libc::F_DUPFD_CLOEXEC, 256) };
        let _restore = RestoreFd198 {
            saved: (saved >= 0).then_some(saved),
            flags: (original_flags >= 0).then_some(original_flags),
        };
        let descriptors =
            super::atomic_cloexec_ownership_pair().expect("create inherited fd198 fixture");
        assert_ne!(unsafe { libc::dup2(descriptors[0], 198) }, -1);
        unsafe { libc::close(descriptors[0]) };
        let mut payload = unsafe { std::fs::File::from_raw_fd(descriptors[1]) };
        let nonce = format!("fd198-authority-{}", std::process::id());
        payload.write_all(nonce.as_bytes()).unwrap();
        drop(payload);
        let metadata = unsafe { std::fs::File::from_raw_fd(libc::dup(198)) }
            .metadata()
            .unwrap();
        let expected = format!("{}:{}:{}", metadata.dev(), metadata.ino(), nonce);
        let marker = std::env::temp_dir().join(format!(
            "unica-runtime-fd198-{}-{:?}",
            std::process::id(),
            thread::current().id()
        ));
        let _ = std::fs::remove_file(&marker);
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "infrastructure::platform::process::tests::runtime_preexisting_fd198_helper",
                "--nocapture",
            ])
            .env(RUNTIME_SENTINEL_FD198_MARKER_ENV, &marker)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut handle = super::RuntimeProcessTreeHandle::prepare(&mut command).unwrap();
        let sentinel_fd = command
            .get_envs()
            .find_map(|(name, value)| {
                (name == "UNICA_RUNTIME_TREE_OWNERSHIP_FD")
                    .then(|| value.unwrap().to_string_lossy().parse::<i32>().unwrap())
            })
            .unwrap();
        assert_ne!(
            sentinel_fd, 198,
            "sentinel selected an occupied authority fd"
        );
        let mut child = command.spawn().unwrap();
        handle.attach(&mut child).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        let descendant = loop {
            if let Ok(actual) = std::fs::read_to_string(&marker) {
                if !actual.is_empty() {
                    let (actual, descendant) = actual
                        .split_once('\n')
                        .expect("helper publishes authority proof and descendant pid");
                    assert_eq!(
                        actual, expected,
                        "child inherited a different fd198 identity/data"
                    );
                    break descendant.parse::<u32>().unwrap();
                }
            }
            assert!(
                Instant::now() < deadline,
                "child never proved inherited fd198"
            );
            assert!(matches!(
                handle.poll(&mut child).unwrap(),
                super::RuntimeProcessTreeState::Running
            ));
            thread::yield_now();
        };
        loop {
            match handle.poll(&mut child).unwrap() {
                super::RuntimeProcessTreeState::Running if handle.leader_exited() => break,
                super::RuntimeProcessTreeState::Running if Instant::now() < deadline => {
                    thread::yield_now();
                }
                other => panic!("dynamic sentinel did not retain its exact lifetime: {other:?}"),
            }
        }
        unsafe { libc::kill(descendant as i32, libc::SIGKILL) };
        loop {
            match handle.poll(&mut child).unwrap() {
                super::RuntimeProcessTreeState::Exited(_) => break,
                super::RuntimeProcessTreeState::Running if Instant::now() < deadline => {
                    thread::yield_now();
                }
                other => panic!("dynamic sentinel did not close after descendant death: {other:?}"),
            }
        }
        let _ = std::fs::remove_file(marker);
        assert_ne!(unsafe { libc::fcntl(198, libc::F_GETFD) }, -1);
        assert_eq!(
            unsafe { libc::fcntl(198, libc::F_GETFD) } & libc::FD_CLOEXEC,
            0
        );
        assert_ne!(unsafe { libc::fcntl(198, libc::F_GETFD) }, -1);
    }

    #[cfg(unix)]
    #[test]
    pub(super) fn runtime_ownership_pipe_accepts_an_exact_quick_terminal_child() {
        let mut command = Command::new("/usr/bin/true");
        let mut handle = super::RuntimeProcessTreeHandle::prepare(&mut command).unwrap();
        let mut child = command.spawn().unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while super::unix_leader_exit_unreaped(child.id())
            .unwrap()
            .is_none()
        {
            assert!(Instant::now() < deadline, "quick child did not exit");
            thread::yield_now();
        }

        handle
            .attach(&mut child)
            .expect("EOF plus an exact retained quick-terminal leader is valid");
        assert!(matches!(
            handle.poll(&mut child).unwrap(),
            super::RuntimeProcessTreeState::Exited(_)
        ));
    }

    #[cfg(unix)]
    #[test]
    pub(super) fn concurrent_runtime_sentinels_do_not_cross_inherit_or_leak_to_unrelated_exec() {
        use std::os::fd::AsRawFd;

        let mut command_a = Command::new("/bin/sh");
        command_a.arg("-c").arg("exec sleep 30");
        let mut handle_a = super::RuntimeProcessTreeHandle::prepare(&mut command_a).unwrap();
        let writer_a = handle_a
            .tree
            .ownership_pipe
            .as_ref()
            .and_then(|pipe| pipe.parent_writer.as_ref())
            .unwrap()
            .as_raw_fd();

        let mut command_b = Command::new("/bin/sh");
        command_b.arg("-c").arg("exec sleep 30");
        let mut handle_b = super::RuntimeProcessTreeHandle::prepare(&mut command_b).unwrap();
        let writer_b = handle_b
            .tree
            .ownership_pipe
            .as_ref()
            .and_then(|pipe| pipe.parent_writer.as_ref())
            .unwrap()
            .as_raw_fd();

        let unrelated = Command::new("/bin/sh")
            .arg("-c")
            .arg(format!(
                "test ! -e /dev/fd/{writer_a} && test ! -e /dev/fd/{writer_b}"
            ))
            .status()
            .unwrap();
        assert!(
            unrelated.success(),
            "unrelated exec inherited a sentinel endpoint"
        );

        let mut child_a = command_a.spawn().unwrap();
        let mut child_b = command_b.spawn().unwrap();
        handle_a.attach(&mut child_a).unwrap();
        handle_b.attach(&mut child_b).unwrap();
        // B was spawned after A while B's parent endpoint was live.  The old
        // globally-inheritable implementation let A retain B's writer, so B
        // could never observe EOF while A stayed alive.  Terminate B first to
        // exercise that exact A -> B inheritance direction.
        handle_b.terminate(&mut child_b).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match handle_b.poll(&mut child_b).unwrap() {
                super::RuntimeProcessTreeState::Exited(_) => break,
                super::RuntimeProcessTreeState::Running if Instant::now() < deadline => {
                    thread::yield_now();
                }
                _ => panic!("runtime B did not reach its independent EOF"),
            }
        }
        assert!(matches!(
            handle_a.poll(&mut child_a).unwrap(),
            super::RuntimeProcessTreeState::Running
        ));
        handle_a
            .terminate_and_reap_bounded(&mut child_a, Duration::from_secs(2))
            .unwrap();
    }

    #[cfg(unix)]
    #[test]
    #[allow(clippy::zombie_processes)]
    // This helper intentionally exits before its detached descendant: the
    // parent test owns the descendant PID, terminates it, and observes sentinel
    // EOF. Waiting here would erase the runner-style lifetime being tested.
    pub(super) fn runtime_runner_style_sentinel_helper() {
        use std::os::unix::process::CommandExt;

        match std::env::var(RUNTIME_SENTINEL_HELPER_ENV).as_deref() {
            Ok("runner") => {
                let executable = std::env::current_exe().unwrap();
                let mut descendant = Command::new(executable);
                descendant
                    .args([
                        "--exact",
                        "infrastructure::platform::process::tests::runtime_runner_style_sentinel_helper",
                        "--nocapture",
                    ])
                    .env(RUNTIME_SENTINEL_HELPER_ENV, "descendant")
                    .env(
                        RUNTIME_SENTINEL_PID_FILE_ENV,
                        std::env::var(RUNTIME_SENTINEL_PID_FILE_ENV).unwrap(),
                    )
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null());
                unsafe {
                    descendant.pre_exec(|| {
                        if libc::setpgid(0, 0) == -1 {
                            Err(std::io::Error::last_os_error())
                        } else {
                            Ok(())
                        }
                    });
                }
                descendant.spawn().unwrap();
            }
            Ok("descendant") => {
                std::fs::write(
                    std::env::var(RUNTIME_SENTINEL_PID_FILE_ENV).unwrap(),
                    std::process::id().to_string(),
                )
                .unwrap();
                thread::sleep(Duration::from_secs(30));
            }
            _ => {}
        }
    }

    #[cfg(unix)]
    #[test]
    pub(super) fn runner_style_new_process_group_and_closed_stdio_still_hold_sentinel_lifetime() {
        let pid_file = std::env::temp_dir().join(format!(
            "unica-runtime-sentinel-{}-{:?}.pid",
            std::process::id(),
            thread::current().id()
        ));
        let _ = std::fs::remove_file(&pid_file);
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "infrastructure::platform::process::tests::runtime_runner_style_sentinel_helper",
                "--nocapture",
            ])
            .env(RUNTIME_SENTINEL_HELPER_ENV, "runner")
            .env(RUNTIME_SENTINEL_PID_FILE_ENV, &pid_file)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut handle = super::RuntimeProcessTreeHandle::prepare(&mut command).unwrap();
        let mut leader = command.spawn().unwrap();
        handle.attach(&mut leader).unwrap();

        let deadline = Instant::now() + Duration::from_secs(5);
        let descendant = loop {
            if let Ok(pid) = std::fs::read_to_string(&pid_file) {
                if let Ok(pid) = pid.parse::<u32>() {
                    break pid;
                }
            }
            assert!(Instant::now() < deadline, "runner descendant did not start");
            thread::sleep(Duration::from_millis(10));
        };
        loop {
            match handle.poll(&mut leader).unwrap() {
                super::RuntimeProcessTreeState::Running if handle.leader_exited() => break,
                super::RuntimeProcessTreeState::Running if Instant::now() < deadline => {
                    thread::yield_now();
                }
                other => panic!("runner sentinel did not retain lifetime: {other:?}"),
            }
        }

        // The runner exited, its child changed process group and both stdio
        // streams were closed. Only the inherited sentinel keeps this Running.
        unsafe { libc::kill(descendant as i32, libc::SIGKILL) };
        loop {
            match handle.poll(&mut leader).unwrap() {
                super::RuntimeProcessTreeState::Exited(_) => break,
                super::RuntimeProcessTreeState::Running if Instant::now() < deadline => {
                    thread::yield_now();
                }
                other => panic!("sentinel did not close after descendant death: {other:?}"),
            }
        }
        let _ = std::fs::remove_file(pid_file);
    }

    #[cfg(windows)]
    #[test]
    fn runtime_process_tree_handle_waits_for_windows_job_object_descendants() {
        assert_windows_runtime_process_tree_semantics_for_test()
            .expect("retain and terminate Windows runtime Job Object");
    }

    #[test]
    fn managed_child_input_wait_error_terminates_before_joining_writer() {
        let started = Instant::now();
        let error = with_wait_error(|| {
            run_helper_with_input(
                "sleep",
                vec![b'x'; 8 * 1024 * 1024],
                Duration::from_secs(20),
                CancellationToken::new(),
            )
        })
        .expect_err("injected wait failure must remain an error");

        assert!(error.contains("injected process wait failure"), "{error}");
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "stdin writer was joined before terminating the child: {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn managed_child_removes_selected_inherited_environment() {
        let output = ManagedChild::run(ManagedCommand {
            program: std::env::current_exe().unwrap(),
            args: vec![
                "--exact".into(),
                "infrastructure::platform::process::tests::managed_child_test_helper".into(),
                "--nocapture".into(),
            ],
            cwd: std::env::current_dir().unwrap(),
            env: vec![(
                OsString::from(HELPER_ENV),
                OsString::from("print_removed_env"),
            )],
            env_remove: vec![OsString::from("PATH")],
            capture_limits: None,
            timeout: Some(Duration::from_secs(5)),
            cancellation: CancellationToken::new(),
        })
        .unwrap();

        assert!(output.status_success, "{output:?}");
        assert!(output.stdout.contains("missing"), "{}", output.stdout);
    }

    #[test]
    fn managed_child_drains_output_while_writing_more_than_a_pipe_buffer() {
        let input = vec![b'x'; 2 * 1024 * 1024];
        let expected_bytes = input.len();

        let output = run_helper_with_input(
            "echo_stdin_len",
            input,
            Duration::from_secs(5),
            CancellationToken::new(),
        )
        .unwrap();

        assert!(
            output.status_success,
            "{}: {}",
            output.status, output.stderr
        );
        assert!(
            output.stdout.contains(&format!("bytes={expected_bytes}")),
            "{}",
            output.stdout
        );
    }

    #[test]
    fn managed_child_input_writer_stops_after_timeout() {
        let started = Instant::now();
        let output = run_helper_with_input(
            "sleep",
            vec![b'x'; 2 * 1024 * 1024],
            Duration::from_millis(50),
            CancellationToken::new(),
        )
        .unwrap();

        assert!(output.timed_out);
        assert!(!output.cancelled);
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn managed_child_input_writer_stops_after_cancellation() {
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let started = Instant::now();
        let output = run_helper_with_input(
            "sleep",
            vec![b'x'; 2 * 1024 * 1024],
            Duration::from_secs(5),
            cancellation,
        )
        .unwrap();

        assert!(output.cancelled);
        assert!(!output.timed_out);
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn managed_child_reports_early_stdin_close_as_bounded_process_failure() {
        let started = Instant::now();
        let error = run_helper_with_input(
            "success",
            vec![b'x'; 2 * 1024 * 1024],
            Duration::from_secs(2),
            CancellationToken::new(),
        )
        .expect_err("early stdin close must not be reported as a successful write");

        assert!(error.starts_with("process_failed:"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn managed_child_reports_invalid_utf8_without_misclassifying_literal_replacement() {
        let invalid = run_helper(
            "write_invalid_utf8",
            Duration::from_secs(2),
            CancellationToken::new(),
        )
        .unwrap();
        let literal = run_helper(
            "write_literal_replacement",
            Duration::from_secs(2),
            CancellationToken::new(),
        )
        .unwrap();

        assert!(invalid.stdout_had_invalid_utf8);
        assert!(invalid.stderr_had_invalid_utf8);
        assert!(invalid.stdout.contains('\u{fffd}'));
        assert!(!literal.stdout_had_invalid_utf8);
        assert!(!literal.stderr_had_invalid_utf8);
        assert!(literal.stdout.contains('\u{fffd}'));
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
            env_remove: Vec::new(),
            capture_limits: None,
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
    fn system_runtime_job_cancellation_reaps_the_owned_process_tree() {
        crate::infrastructure::runtime_jobs::assert_system_cancellation_reaps_process_tree();
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
                "infrastructure::platform::process::tests::managed_child_test_helper".into(),
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
            env_remove: Vec::new(),
            capture_limits: None,
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
                "--exact".into(),
                "infrastructure::platform::process::tests::managed_child_test_helper".into(),
                "--nocapture".into(),
            ],
            cwd: std::env::current_dir().unwrap(),
            env: vec![(OsString::from(HELPER_ENV), OsString::from("sleep"))],
            env_remove: Vec::new(),
            capture_limits: None,
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
    fn line_consumer_can_stop_and_reap_the_process_before_timeout() {
        let mut managed = ManagedChild::spawn(ManagedCommand {
            program: std::env::current_exe().unwrap(),
            args: vec![
                "--exact".into(),
                "infrastructure::platform::process::tests::managed_child_test_helper".into(),
                "--nocapture".into(),
            ],
            cwd: std::env::current_dir().unwrap(),
            env: vec![(OsString::from(HELPER_ENV), OsString::from("stream_forever"))],
            env_remove: Vec::new(),
            capture_limits: None,
            timeout: Some(Duration::from_secs(5)),
            cancellation: CancellationToken::new(),
        })
        .unwrap();
        let mut lines = 0;
        let started = Instant::now();

        let output = managed
            .wait_for_line_output(1024, |_, _| {
                lines += 1;
                if lines == 3 {
                    StreamControl::Stop
                } else {
                    StreamControl::Continue
                }
            })
            .unwrap();

        assert_eq!(lines, 3);
        assert!(output.stopped_by_consumer);
        assert!(!output.timed_out);
        assert!(!output.cancelled);
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(managed.state, ChildState::Reaped);
    }

    #[test]
    fn managed_child_reaps_already_exited_process_without_tree_kill() {
        let mut managed = ManagedChild::spawn(ManagedCommand {
            program: std::env::current_exe().unwrap(),
            args: vec![
                "--exact".into(),
                "infrastructure::platform::process::tests::managed_child_test_helper".into(),
                "--nocapture".into(),
            ],
            cwd: std::env::current_dir().unwrap(),
            env: vec![(OsString::from(HELPER_ENV), OsString::from("success"))],
            env_remove: Vec::new(),
            capture_limits: None,
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
                "--exact".into(),
                "infrastructure::platform::process::tests::managed_child_test_helper".into(),
                "--nocapture".into(),
            ],
            cwd: std::env::current_dir().unwrap(),
            env: vec![(OsString::from(HELPER_ENV), OsString::from("sleep"))],
            env_remove: Vec::new(),
            capture_limits: None,
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
                "--exact".into(),
                "infrastructure::platform::process::tests::managed_child_test_helper".into(),
                "--nocapture".into(),
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
            env_remove: Vec::new(),
            capture_limits: None,
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

    #[cfg(unix)]
    #[test]
    fn cancellation_lets_runner_cleanup_its_external_process_group() {
        let pid_file = std::env::temp_dir().join(format!(
            "unica-graceful-cancellation-pids-{}-{}",
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
                "--exact".into(),
                "infrastructure::platform::process::tests::managed_child_test_helper".into(),
                "--nocapture".into(),
            ],
            cwd: std::env::current_dir().unwrap(),
            env: vec![
                (
                    OsString::from(HELPER_ENV),
                    OsString::from("graceful_runner_with_external_process_group"),
                ),
                (
                    OsString::from(HELPER_PID_FILE_ENV),
                    pid_file.clone().into_os_string(),
                ),
            ],
            env_remove: Vec::new(),
            capture_limits: None,
            timeout: Some(Duration::from_secs(10)),
            cancellation: cancellation.clone(),
        })
        .unwrap();
        let mut managed_cleanup = ManagedChildCleanupGuard::new(managed, cancellation.clone());
        let pids = read_helper_pids(&pid_file, Duration::from_secs(2));
        let mut cleanup = ProcessCleanupGuard(pids.clone());

        cancellation.cancel();
        let output = managed_cleanup.managed_mut().wait_for_output().unwrap();
        managed_cleanup.disarm();

        assert!(output.cancelled);
        assert!(wait_until_dead(pids[0], Duration::from_secs(2)));
        assert!(
            wait_until_dead(pids[1], Duration::from_secs(2)),
            "runner did not get a chance to clean up its external process group"
        );
        cleanup.disarm();
    }

    #[test]
    fn inherited_pipe_descendant_does_not_block_timeout_collection() {
        let pid_file = std::env::temp_dir().join(format!(
            "unica-inherited-pipe-pids-{}-{}",
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
                "--exact".into(),
                "infrastructure::platform::process::tests::managed_child_test_helper".into(),
                "--nocapture".into(),
            ],
            cwd: std::env::current_dir().unwrap(),
            env: vec![
                (
                    OsString::from(HELPER_ENV),
                    OsString::from("inherited_pipe_parent"),
                ),
                (
                    OsString::from(HELPER_PID_FILE_ENV),
                    pid_file.clone().into_os_string(),
                ),
            ],
            env_remove: Vec::new(),
            capture_limits: None,
            timeout: Some(Duration::from_millis(200)),
            cancellation: cancellation.clone(),
        })
        .unwrap();
        let mut managed_cleanup = ManagedChildCleanupGuard::new(managed, cancellation);
        let pids = read_helper_pids(&pid_file, Duration::from_secs(2));
        let mut cleanup = ProcessCleanupGuard(pids.clone());
        let started = Instant::now();
        let output = managed_cleanup.managed_mut().wait_for_output().unwrap();
        managed_cleanup.disarm();

        assert!(output.timed_out);
        assert!(
            output.stdout.contains("inherited-pipe-before-timeout"),
            "{}",
            output.stdout
        );
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(wait_until_dead(pids[0], Duration::from_secs(2)));
        assert!(wait_until_dead(pids[1], Duration::from_secs(2)));
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
                "infrastructure::platform::process::tests::managed_child_test_helper",
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
    fn startup_child_tree_cleanup_uses_only_the_remaining_absolute_budget() {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "infrastructure::platform::process::tests::managed_child_test_helper",
                "--nocapture",
            ])
            .env(HELPER_ENV, "success")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut startup = ManagedStartupChild::spawn_configured(command).unwrap();
        let probe = ManualStartupTerminationProbe::new(Duration::from_millis(450));
        startup.install_termination_probe_for_test(&probe);

        startup
            .terminate_bounded(Duration::from_millis(500))
            .unwrap();

        assert_eq!(probe.attempt_count(), 1);
        assert_eq!(probe.elapsed(), Duration::from_millis(450));
        assert_eq!(
            probe.cleanup_remaining_budgets(),
            vec![Duration::from_millis(50)]
        );
    }

    #[test]
    fn startup_child_explicit_timeout_is_not_retried_by_drop() {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "infrastructure::platform::process::tests::managed_child_test_helper",
                "--nocapture",
            ])
            .env(HELPER_ENV, "sleep")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut startup = ManagedStartupChild::spawn_configured(command).unwrap();
        let pid = startup.id();
        let _cleanup = ProcessCleanupGuard(vec![pid]);
        let probe = ManualStartupTerminationProbe::new(Duration::from_secs(1));
        startup.install_termination_probe_for_test(&probe);

        let error = startup
            .terminate_bounded(Duration::from_millis(100))
            .unwrap_err();
        assert!(error.contains("did not exit within 100 ms"), "{error}");
        assert_eq!(probe.elapsed(), Duration::from_millis(100));
        drop(startup);

        assert_eq!(probe.attempt_count(), 1);
        assert_eq!(probe.elapsed(), Duration::from_millis(100));
    }

    #[test]
    fn startup_child_drop_without_explicit_attempt_keeps_bounded_cleanup() {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "infrastructure::platform::process::tests::managed_child_test_helper",
                "--nocapture",
            ])
            .env(HELPER_ENV, "sleep")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut startup = ManagedStartupChild::spawn_configured(command).unwrap();
        let probe = ManualStartupTerminationProbe::new(Duration::from_millis(25));
        startup.install_termination_probe_for_test(&probe);

        drop(startup);

        assert_eq!(probe.attempt_count(), 1);
        assert_eq!(probe.elapsed(), Duration::from_millis(25));
        assert_eq!(
            probe.cleanup_remaining_budgets(),
            vec![Duration::from_millis(475)]
        );
    }

    #[test]
    fn managed_process_tree_lifecycle_is_bounded() {
        managed_child_timeout_returns_within_a_bounded_interval();
        managed_child_cancellation_returns_within_a_bounded_interval();
        managed_child_drop_terminates_and_reaps_running_process();
        managed_child_kills_descendants();
        startup_child_cleanup_kills_descendants();
        system_runtime_job_cancellation_reaps_the_owned_process_tree();
        #[cfg(windows)]
        process_tree_keeps_child_suspended_until_attach();
    }

    #[test]
    fn startup_child_detach_leaves_process_running() {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "infrastructure::platform::process::tests::managed_child_test_helper",
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
    fn startup_child_exposes_exit_status_without_detaching() {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "infrastructure::platform::process::tests::managed_child_test_helper",
                "--nocapture",
            ])
            .env(HELPER_ENV, "success")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut child = ManagedStartupChild::spawn_configured(command).unwrap();

        let deadline = Instant::now() + Duration::from_secs(2);
        let status = loop {
            if let Some(status) = child.try_wait_status().unwrap() {
                break status;
            }
            assert!(Instant::now() < deadline, "startup child did not exit");
            thread::yield_now();
        };

        assert!(status.success());
        child.terminate_bounded(Duration::from_secs(1)).unwrap();
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
                "infrastructure::platform::process::tests::managed_child_test_helper",
                "--nocapture",
            ])
            .env(HELPER_ENV, "write_marker")
            .env(HELPER_PID_FILE_ENV, &marker)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut process_tree = ProcessTree::prepare(&mut command).unwrap();
        let mut child = ChildCleanupGuard(Some(command.spawn().unwrap()));

        thread::sleep(Duration::from_millis(500));
        assert!(!marker.exists(), "child ran before process-tree attachment");

        process_tree.attach(child.child_mut()).unwrap();
        let started = Instant::now();
        while !marker.exists() && started.elapsed() < Duration::from_secs(2) {
            thread::sleep(Duration::from_millis(25));
        }
        assert!(marker.exists(), "child did not resume after attachment");
        child.wait();
    }
}
