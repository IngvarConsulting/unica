#[cfg(any(windows, test))]
mod windows_job_exit;

#[cfg(any(test, feature = "test-support"))]
use std::cell::Cell;
use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, ExitStatus, Stdio};
#[cfg(any(test, feature = "test-support"))]
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
#[cfg(all(any(test, feature = "test-support"), windows))]
use std::sync::{Condvar, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};
use unica_work_control::cancellation::{cancelled_error, CancellationToken};

const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(25);
const TERMINATION_WAIT_LIMIT: Duration = Duration::from_millis(500);
const READER_WAIT_LIMIT: Duration = Duration::from_millis(500);
pub const STREAM_LINE_TOO_LONG_ERROR: &str = "line exceeds configured byte limit";
pub const STREAM_RECORD_UNTERMINATED_ERROR: &str = "output ends inside a record";

#[cfg(all(any(test, feature = "test-support"), windows))]
struct JobAttachGateState {
    spawned: bool,
    released: bool,
}

#[cfg(all(any(test, feature = "test-support"), windows))]
type JobAttachGateSignal = Arc<(Mutex<JobAttachGateState>, Condvar)>;

#[cfg(all(any(test, feature = "test-support"), windows))]
fn job_attach_gates() -> &'static Mutex<std::collections::HashMap<PathBuf, JobAttachGateSignal>> {
    static GATES: OnceLock<Mutex<std::collections::HashMap<PathBuf, JobAttachGateSignal>>> =
        OnceLock::new();
    GATES.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

/// A one-shot test barrier after Windows has spawned the suspended child and
/// before the production Job Object attachment resumes it.
#[cfg(all(any(test, feature = "test-support"), windows))]
pub struct JobAttachGateForTest {
    program: PathBuf,
    signal: JobAttachGateSignal,
}

#[cfg(all(any(test, feature = "test-support"), windows))]
impl JobAttachGateForTest {
    pub const fn supported() -> bool {
        true
    }

    pub fn install(program: PathBuf) -> Self {
        let signal = Arc::new((
            Mutex::new(JobAttachGateState {
                spawned: false,
                released: false,
            }),
            Condvar::new(),
        ));
        let previous = job_attach_gates()
            .lock()
            .expect("job attach gate registry")
            .insert(program.clone(), Arc::clone(&signal));
        assert!(previous.is_none(), "duplicate Job attach test gate");
        Self { program, signal }
    }

    pub fn wait_spawned(&self, timeout: Duration) {
        let (state, ready) = &*self.signal;
        let state = state.lock().expect("job attach gate state");
        let (state, _) = ready
            .wait_timeout_while(state, timeout, |state| !state.spawned)
            .expect("job attach gate wait");
        assert!(state.spawned, "runner did not reach the Job attach gate");
    }

    pub fn release(&self) {
        let (state, ready) = &*self.signal;
        state.lock().expect("job attach gate state").released = true;
        ready.notify_all();
    }
}

#[cfg(all(any(test, feature = "test-support"), not(windows)))]
pub struct JobAttachGateForTest;

#[cfg(all(any(test, feature = "test-support"), not(windows)))]
impl JobAttachGateForTest {
    pub const fn supported() -> bool {
        false
    }

    pub fn install(_program: PathBuf) -> Self {
        unreachable!("Windows Job attachment is unavailable on this host")
    }

    pub fn wait_spawned(&self, _timeout: Duration) {
        unreachable!("Windows Job attachment is unavailable on this host")
    }

    pub fn release(&self) {
        unreachable!("Windows Job attachment is unavailable on this host")
    }
}

#[cfg(all(any(test, feature = "test-support"), windows))]
impl Drop for JobAttachGateForTest {
    fn drop(&mut self) {
        self.release();
        job_attach_gates()
            .lock()
            .expect("job attach gate registry")
            .remove(&self.program);
    }
}

#[cfg(all(any(test, feature = "test-support"), windows))]
fn wait_at_job_attach_gate_for_test(process: &Command) {
    // infobase.create also launches a dry-run preflight on apply. The gate
    // must stop the protected mutation, not that cancellable preflight.
    if process
        .get_args()
        .any(|arg| arg.to_str() == Some("--dry-run"))
    {
        return;
    }
    let program = process.get_program();
    let program = std::fs::canonicalize(program).unwrap_or_else(|_| PathBuf::from(program));
    let gate = job_attach_gates()
        .lock()
        .expect("job attach gate registry")
        .remove(&program);
    if let Some(gate) = gate {
        let (state, ready) = &*gate;
        let mut state = state.lock().expect("job attach gate state");
        state.spawned = true;
        ready.notify_all();
        while !state.released {
            state = ready.wait(state).expect("job attach gate release");
        }
    }
}
pub const STDOUT_CAPTURE_LIMIT: usize = 1024 * 1024;
pub const STDERR_CAPTURE_LIMIT: usize = 256 * 1024;

#[derive(Debug)]
pub enum RuntimeProcessTreeState {
    Running,
    Exited(ExitStatus),
}

/// Retained authority for a runtime job's complete owned process tree.
///
/// Leader exit is remembered but is not terminal until the Unix process group
/// or Windows Job Object proves empty. Callers therefore cannot release a
/// workspace resource merely because `Child::try_wait` reaped the leader.
pub struct RuntimeProcessTreeHandle {
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
    pub fn prepare(command: &mut Command) -> io::Result<Self> {
        Ok(Self {
            tree: ProcessTree::prepare_runtime(command)?,
            leader_exit: None,
            leader_exit_observed: false,
        })
    }

    pub fn attach(&mut self, child: &mut Child) -> io::Result<()> {
        if let Some(status) = self.tree.attach(child)? {
            self.leader_exit = Some(status);
            self.leader_exit_observed = true;
        }
        Ok(())
    }

    pub fn poll(&mut self, child: &mut Child) -> io::Result<RuntimeProcessTreeState> {
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
            // Collect while the leader is still alive too: short-lived
            // descendants can disappear before its final exit is observed.
            #[cfg(windows)]
            self.tree.observe_runtime_members()?;
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

    pub fn terminate(&mut self, child: &mut Child) -> io::Result<()> {
        self.tree.terminate(child)
    }

    /// One bounded cleanup window for every partial startup state. Killing the
    /// leader as a fallback is required when Windows attachment failed before
    /// the child entered the Job Object.
    #[cfg(any(test, feature = "test-support"))]
    pub fn terminate_and_reap_bounded(
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
    pub fn terminate_and_reap_until(
        &mut self,
        child: &mut Child,
        deadline: Instant,
    ) -> io::Result<()> {
        #[cfg(any(test, feature = "test-support"))]
        RUNTIME_TREE_CLEANUP_CALLS.with(|slot| slot.set(slot.get().saturating_add(1)));
        #[cfg(any(test, feature = "test-support"))]
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

    #[cfg(any(test, feature = "test-support"))]
    pub fn leader_exited(&self) -> bool {
        self.leader_exit_observed || self.leader_exit.is_some()
    }
}

#[cfg(unix)]
fn unix_leader_exit_unreaped(process_id: u32) -> io::Result<Option<ExitStatus>> {
    use std::mem::zeroed;
    use std::os::unix::process::ExitStatusExt;

    #[cfg(any(test, feature = "test-support"))]
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

#[cfg(any(test, feature = "test-support"))]
pub struct RuntimeProcessTreeTestScenario {
    program: PathBuf,
    leader_with_descendant_args: Vec<String>,
    long_lived_args: Vec<String>,
    descendant_pid_path: PathBuf,
    #[cfg(target_os = "macos")]
    _descendant_lifetime: std::fs::File,
}

#[cfg(any(test, feature = "test-support"))]
impl RuntimeProcessTreeTestScenario {
    pub fn program(&self) -> PathBuf {
        self.program.clone()
    }

    pub fn leader_with_descendant_args(&self) -> Vec<String> {
        self.leader_with_descendant_args.clone()
    }

    pub fn long_lived_args(&self) -> Vec<String> {
        self.long_lived_args.clone()
    }

    pub fn wait_for_descendant(
        &self,
        timeout: Duration,
    ) -> io::Result<RuntimeProcessTreeTestProbe> {
        let started = Instant::now();
        loop {
            match std::fs::read_to_string(&self.descendant_pid_path) {
                Ok(value) => {
                    if let Ok(process_id) = value.trim().parse::<u32>() {
                        let probe = RuntimeProcessTreeTestProbe::capture(process_id)?;
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

#[cfg(any(test, feature = "test-support"))]
pub struct RuntimeProcessTreeTestProbe {
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    process_id: u32,
    #[cfg(target_os = "linux")]
    process: std::fs::File,
    #[cfg(target_os = "macos")]
    process: MacosRuntimeProcessTestProbe,
    #[cfg(windows)]
    process: ScopedWindowsHandle,
}

#[cfg(any(test, feature = "test-support"))]
impl RuntimeProcessTreeTestProbe {
    fn capture(process_id: u32) -> io::Result<Self> {
        #[cfg(target_os = "macos")]
        {
            Ok(Self {
                process: MacosRuntimeProcessTestProbe::capture(process_id)?,
            })
        }
        #[cfg(target_os = "linux")]
        {
            use std::os::fd::FromRawFd;

            let pid = libc::pid_t::try_from(process_id)
                .map_err(|_| io::Error::other("process id is outside Linux pid range"))?;
            // The fixture keeps its descendant alive until cancellation. The
            // retained pidfd then observes that exact process even after reap.
            let descriptor = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0_u32) };
            if descriptor == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(Self {
                process: unsafe { std::fs::File::from_raw_fd(descriptor as i32) },
            })
        }
        #[cfg(windows)]
        {
            use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE};

            // SAFETY: the live fixture publishes its PID before cancellation.
            // A non-inheritable handle pins the observed process across later probes.
            let process = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, process_id) };
            if process.is_null() {
                return Err(io::Error::last_os_error());
            }
            Ok(Self {
                process: ScopedWindowsHandle(process),
            })
        }
        #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
        {
            Ok(Self { process_id })
        }
    }

    pub fn is_alive(&self) -> io::Result<bool> {
        #[cfg(target_os = "macos")]
        {
            self.process.is_alive()
        }
        #[cfg(target_os = "linux")]
        {
            runtime_process_pidfd_alive_for_test(&self.process)
        }
        #[cfg(windows)]
        {
            runtime_process_handle_alive_for_test(&self.process)
        }
        #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
        {
            runtime_process_pid_alive_for_test(self.process_id)
        }
    }
}

#[cfg(all(any(test, feature = "test-support"), target_os = "macos"))]
struct MacosRuntimeProcessTestProbe {
    queue: std::fs::File,
    process_id: u32,
    exited: Cell<bool>,
}

#[cfg(all(any(test, feature = "test-support"), target_os = "macos"))]
impl MacosRuntimeProcessTestProbe {
    // The caller must hold the observed process alive through registration.
    // The runtime scenario retains its FIFO writer; direct controls retain stdin.
    fn capture(process_id: u32) -> io::Result<Self> {
        use std::os::fd::{AsRawFd, FromRawFd};

        if process_id == 0 || process_id > i32::MAX as u32 {
            return Err(io::Error::other("invalid macOS process id"));
        }
        let descriptor = unsafe { libc::kqueue() };
        if descriptor == -1 {
            return Err(io::Error::last_os_error());
        }
        // Own the queue immediately, including every registration failure path.
        let queue = unsafe { std::fs::File::from_raw_fd(descriptor) };
        if unsafe { libc::fcntl(queue.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } == -1 {
            return Err(io::Error::last_os_error());
        }
        let registration = libc::kevent {
            ident: process_id as libc::uintptr_t,
            filter: libc::EVFILT_PROC,
            flags: libc::EV_ADD | libc::EV_ENABLE | libc::EV_RECEIPT,
            fflags: libc::NOTE_EXIT,
            data: 0,
            udata: std::ptr::null_mut(),
        };
        let mut receipt: libc::kevent = unsafe { std::mem::zeroed() };
        let timeout = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        let count = unsafe {
            libc::kevent(
                queue.as_raw_fd(),
                &registration,
                1,
                &mut receipt,
                1,
                &timeout,
            )
        };
        if count == -1 {
            return Err(io::Error::last_os_error());
        }
        if count != 1
            || receipt.ident != registration.ident
            || receipt.filter != libc::EVFILT_PROC
            || receipt.flags & libc::EV_ERROR == 0
        {
            return Err(io::Error::other(
                "unexpected macOS process registration receipt",
            ));
        }
        if receipt.data != 0 {
            let errno = i32::try_from(receipt.data)
                .map_err(|_| io::Error::other("invalid macOS process registration error"))?;
            return Err(io::Error::from_raw_os_error(errno));
        }
        // EV_RECEIPT acknowledges registration; it is not an exit observation.
        Ok(Self {
            queue,
            process_id,
            exited: Cell::new(false),
        })
    }

    fn is_alive(&self) -> io::Result<bool> {
        use std::os::fd::AsRawFd;

        if self.exited.get() {
            return Ok(false);
        }
        let mut event: libc::kevent = unsafe { std::mem::zeroed() };
        let timeout = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        let count = unsafe {
            libc::kevent(
                self.queue.as_raw_fd(),
                std::ptr::null(),
                0,
                &mut event,
                1,
                &timeout,
            )
        };
        if count == -1 {
            return Err(io::Error::last_os_error());
        }
        if count == 0 {
            return Ok(true);
        }
        if count != 1 {
            return Err(io::Error::other("unexpected macOS process event count"));
        }
        validate_macos_runtime_exit_event(self.process_id, &event)?;
        // NOTE_EXIT may be consumed only once. Never consult the numeric PID
        // again after observing this retained registration's terminal event.
        self.exited.set(true);
        Ok(false)
    }
}

#[cfg(all(any(test, feature = "test-support"), target_os = "macos"))]
fn validate_macos_runtime_exit_event(process_id: u32, event: &libc::kevent) -> io::Result<()> {
    if event.ident != process_id as libc::uintptr_t
        || event.filter != libc::EVFILT_PROC
        || event.flags & libc::EV_ERROR != 0
        || event.fflags != libc::NOTE_EXIT
    {
        return Err(io::Error::other("unexpected macOS process exit event"));
    }
    Ok(())
}

#[cfg(any(test, feature = "test-support"))]
pub fn runtime_process_tree_test_scenario_for_test(
    root: &std::path::Path,
) -> RuntimeProcessTreeTestScenario {
    let descendant_pid_path = root.join("runtime-tree-descendant.pid");
    #[cfg(unix)]
    {
        let escaped_path = descendant_pid_path.to_string_lossy().replace('\'', "'\\''");
        #[cfg(target_os = "macos")]
        let (descendant_command, descendant_lifetime) = {
            use std::os::unix::ffi::OsStrExt;
            use std::os::unix::fs::OpenOptionsExt;

            let lifetime_path = root.join("runtime-tree-descendant.lifetime");
            let native_path = std::ffi::CString::new(lifetime_path.as_os_str().as_bytes()).unwrap();
            // mkfifo refuses an existing path; this belongs to the fresh cache.
            assert_eq!(
                unsafe { libc::mkfifo(native_path.as_ptr(), 0o600) },
                0,
                "create owned descendant lifetime FIFO: {}",
                io::Error::last_os_error()
            );
            let reader = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
                .open(&lifetime_path)
                .expect("open initial owned FIFO reader");
            let writer = std::fs::OpenOptions::new()
                .write(true)
                .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
                .open(&lifetime_path)
                .expect("retain owned descendant lifetime writer");
            drop(reader);
            let escaped_lifetime = lifetime_path.to_string_lossy().replace('\'', "'\\''");
            (
                format!("(IFS= read -r held < '{escaped_lifetime}')"),
                writer,
            )
        };
        // Linux's GNU sleep does not expire while the fixture is paused before
        // capturing its pidfd. Cancellation remains the sole lifetime owner.
        #[cfg(not(target_os = "macos"))]
        let descendant_command = if cfg!(target_os = "linux") {
            "sleep infinity"
        } else {
            "sleep 30"
        };
        RuntimeProcessTreeTestScenario {
            program: PathBuf::from("/bin/sh"),
            leader_with_descendant_args: vec![
                "-c".to_string(),
                format!(
                    "{descendant_command} </dev/null >/dev/null 2>&1 & child=$!; printf '%s' \"$child\" > '{escaped_path}'"
                ),
            ],
            long_lived_args: vec![
                "-c".to_string(),
                format!(
                    "{descendant_command} </dev/null >/dev/null 2>&1 & child=$!; printf '%s' \"$child\" > '{escaped_path}'; wait \"$child\""
                ),
            ],
            descendant_pid_path,
            #[cfg(target_os = "macos")]
            _descendant_lifetime: descendant_lifetime,
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

#[cfg(all(any(test, feature = "test-support"), target_os = "linux"))]
fn runtime_process_pidfd_alive_for_test(process: &std::fs::File) -> io::Result<bool> {
    use std::os::fd::AsRawFd;

    let mut descriptor = libc::pollfd {
        fd: process.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    // Readiness means terminated, including an unreaped zombie; it does not
    // depend on whether the old PID exists or has since been reused.
    let ready = unsafe { libc::poll(&mut descriptor, 1, 0) };
    if ready == -1 {
        return Err(io::Error::last_os_error());
    }
    if descriptor.revents & (libc::POLLNVAL | libc::POLLERR) != 0 {
        return Err(io::Error::other(format!(
            "runtime process pidfd poll failed: events={:#x}",
            descriptor.revents
        )));
    }
    if ready == 0 {
        return Ok(true);
    }
    if descriptor.revents & (libc::POLLIN | libc::POLLHUP) != 0 {
        return Ok(false);
    }
    Err(io::Error::other(format!(
        "unexpected runtime process pidfd events: {:#x}",
        descriptor.revents
    )))
}

#[cfg(all(
    any(test, feature = "test-support"),
    unix,
    not(any(target_os = "linux", target_os = "macos"))
))]
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

#[cfg(all(any(test, feature = "test-support"), windows))]
fn runtime_process_handle_alive_for_test(process: &ScopedWindowsHandle) -> io::Result<bool> {
    use windows_sys::Win32::Foundation::{WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT};
    use windows_sys::Win32::System::Threading::WaitForSingleObject;

    // SAFETY: the probe retains the exact handle captured before cancellation.
    match unsafe { WaitForSingleObject(process.0, 0) } {
        WAIT_TIMEOUT => Ok(true),
        WAIT_OBJECT_0 => Ok(false),
        WAIT_FAILED => Err(io::Error::last_os_error()),
        state => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unexpected runtime process wait state: {state}"),
        )),
    }
}

#[cfg(all(any(test, feature = "test-support"), not(any(unix, windows))))]
fn runtime_process_pid_alive_for_test(_process_id: u32) -> io::Result<bool> {
    Ok(false)
}

/// How long the probe lets the PowerShell leader start, spawn its descendant
/// and exit. The semantics under test do not depend on this number: the
/// descendant is a Job Object member that outlives the leader, and the tree
/// is torn down explicitly at the end. Five seconds was a cold-start budget
/// for `powershell.exe`, and the nightly `large` tier on `windows-latest`
/// overran it in two of six nights (runs 34317173405 and 34676550320) while
/// this probe ran nested inside `daemon_exact_long_work_ownership_contract`
/// next to the full suite (#750). The descendant lives long enough for the
/// leader budget to expire with it still alive, so a slow leader is told apart
/// from a tree that went terminal.
#[cfg(all(any(test, feature = "test-support"), windows))]
const WINDOWS_PROBE_LEADER_EXIT_BUDGET: Duration = Duration::from_secs(30);
#[cfg(all(any(test, feature = "test-support"), windows))]
const WINDOWS_PROBE_DESCENDANT_PINGS: &str = "90";

#[cfg(all(any(test, feature = "test-support"), windows))]
pub fn assert_windows_runtime_process_tree_semantics_for_test() -> io::Result<()> {
    let mut command = Command::new("powershell.exe");
    let start_descendant = format!(
        "Start-Process -WindowStyle Hidden ping.exe -ArgumentList @('-n','{}','127.0.0.1') | Out-Null",
        WINDOWS_PROBE_DESCENDANT_PINGS
    );
    command
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            start_descendant.as_str(),
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
            RuntimeProcessTreeState::Running
                if started.elapsed() < WINDOWS_PROBE_LEADER_EXIT_BUDGET =>
            {
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

#[cfg(any(test, feature = "test-support"))]
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

#[cfg(any(test, feature = "test-support"))]
pub fn inject_runtime_tree_cleanup_timeout_for_test() {
    INJECT_RUNTIME_TREE_CLEANUP_TIMEOUT.with(|slot| slot.set(true));
}

#[cfg(any(test, feature = "test-support"))]
pub fn reset_runtime_tree_cleanup_calls_for_test() {
    RUNTIME_TREE_CLEANUP_CALLS.with(|slot| slot.set(0));
}

#[cfg(any(test, feature = "test-support"))]
pub fn runtime_tree_cleanup_calls_for_test() -> u32 {
    RUNTIME_TREE_CLEANUP_CALLS.with(Cell::get)
}

#[cfg(all(any(test, feature = "test-support"), unix))]
pub fn inject_unix_waitid_error_for_test() {
    INJECT_UNIX_WAITID_ERROR.with(|slot| slot.set(true));
}

#[cfg(all(any(test, feature = "test-support"), unix))]
pub fn inject_unix_reap_error_for_test() {
    INJECT_UNIX_REAP_ERROR.with(|slot| slot.set(true));
}

#[cfg(all(any(test, feature = "test-support"), unix))]
pub fn reset_unix_signal_count_for_test() {
    UNIX_SIGNAL_COUNT.with(|slot| slot.set(0));
}

#[cfg(all(any(test, feature = "test-support"), unix))]
pub fn unix_signal_count_for_test() -> u32 {
    UNIX_SIGNAL_COUNT.with(Cell::get)
}

#[cfg(all(any(test, feature = "test-support"), unix))]
pub fn reap_runtime_authority_test_child(process_id: u32) {
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

/// A response to one live stdout line. Replies contain exactly one nonempty,
/// LF-terminated UTF-8 line, including the LF, and at most 4096 bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessLineReply {
    Continue,
    Reply(Vec<u8>),
    /// The caller has already committed its external effect under its
    /// cancellation gate. Stop observing cancellation before writing this
    /// reply and finish within this budget. The command's explicit timeout
    /// still applies; further protected replies cannot extend this budget.
    ReplyAndProtect {
        reply: Vec<u8>,
        finish_timeout: Duration,
    },
    Stop,
}

const STREAM_REPLY_LIMIT: usize = 4096;

/// Only the pipe writer runs off-thread. ManagedChild remains the sole owner
/// of process polling, cancellation, deadlines and process-tree cleanup.
struct LineReplyWriter {
    requests: mpsc::SyncSender<Vec<u8>>,
    results: Receiver<Result<(), String>>,
    writing: bool,
}

impl LineReplyWriter {
    fn start(mut stdin: ChildStdin) -> Self {
        let (requests, pending) = mpsc::sync_channel::<Vec<u8>>(1);
        let (finished, results) = mpsc::sync_channel(1);
        thread::spawn(move || {
            while let Ok(bytes) = pending.recv() {
                let result = stdin.write_all(&bytes).map_err(|error| {
                    format!("process_failed: process stdin reply write failed: {error}")
                });
                let failed = result.is_err();
                if finished.send(result).is_err() || failed {
                    break;
                }
            }
        });
        Self {
            requests,
            results,
            writing: false,
        }
    }

    fn reply(&mut self, bytes: Vec<u8>) -> Result<(), String> {
        if bytes.len() < 2
            || bytes.len() > STREAM_REPLY_LIMIT
            || bytes.last() != Some(&b'\n')
            || bytes[..bytes.len() - 1]
                .iter()
                .any(|byte| matches!(*byte, b'\n' | b'\r'))
            || std::str::from_utf8(&bytes).is_err()
        {
            return Err("process_failed: process stdin reply must be one LF-terminated UTF-8 line of at most 4096 bytes".into());
        }
        if self.writing {
            return Err("process_failed: process stdin reply is still pending".into());
        }
        self.requests
            .try_send(bytes)
            .map_err(|_| "process_failed: process stdin reply writer is unavailable".to_string())?;
        self.writing = true;
        Ok(())
    }

    fn poll(&mut self) -> Result<(), String> {
        if !self.writing {
            return Ok(());
        }
        match self.results.try_recv() {
            Ok(result) => {
                self.writing = false;
                result
            }
            Err(TryRecvError::Empty) => Ok(()),
            Err(TryRecvError::Disconnected) => {
                Err("process_failed: process stdin reply writer disconnected".into())
            }
        }
    }
}

enum LineStreamInput {
    Closed,
    Replies(LineReplyWriter),
}

#[derive(Clone, Copy)]
enum LineStreamKind {
    Observe,
    Controlled,
}

impl LineStreamInput {
    fn is_controlled(&self) -> bool {
        matches!(self, Self::Replies(_))
    }

    fn is_writing(&self) -> bool {
        match self {
            Self::Closed => false,
            Self::Replies(writer) => writer.writing,
        }
    }

    fn kind(&self) -> LineStreamKind {
        match self {
            Self::Closed => LineStreamKind::Observe,
            Self::Replies(_) => LineStreamKind::Controlled,
        }
    }

    fn poll(&mut self) -> Result<(), String> {
        match self {
            Self::Closed => Ok(()),
            Self::Replies(writer) => writer.poll(),
        }
    }

    fn reply(&mut self, bytes: Vec<u8>) -> Result<(), String> {
        match self {
            Self::Closed => Err("process_failed: process stdin is closed".into()),
            Self::Replies(writer) => writer.reply(bytes),
        }
    }
}

pub struct ManagedChild {
    child: Child,
    process_tree: ProcessTree,
    state: ChildState,
    timeout: Option<Duration>,
    cancellation: CancellationToken,
    capture_limits: Option<(usize, usize)>,
}

/// Where a run that hands its process tree over failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingRunFailure {
    /// The launcher did not start; the start failure says where.
    Spawn(ManagedSpawnFailure),
    /// The launcher started and its run failed.
    Run(String),
}

/// Owns the runner's process tree after its leader exited, while the caller
/// validates the complete captured reply. An unverified reply leaves this guard
/// armed: dropping it terminates the still-owned descendants.
pub struct PendingProcessHandoff {
    child: Option<ManagedChild>,
    released: bool,
    #[cfg(any(test, feature = "test-support"))]
    probe: Option<(Arc<AtomicUsize>, Arc<AtomicUsize>)>,
}

impl PendingProcessHandoff {
    /// Release the tree only after the caller verified the launch receipt. A
    /// failed release keeps ownership so Drop can still attempt cleanup.
    pub fn release(&mut self) -> Result<(), String> {
        if self.released {
            return Err("process_failed: launch process handoff was already released".into());
        }
        if let Some(child) = self.child.as_mut() {
            child
                .process_tree
                .release_handoff(&mut child.child)
                .map_err(process_error)?;
            child.state = ChildState::Reaped;
        }
        self.released = true;
        #[cfg(any(test, feature = "test-support"))]
        if let Some((released, _)) = &self.probe {
            released.fetch_add(1, Ordering::Relaxed);
        }
        self.child.take();
        Ok(())
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn for_test(released: Arc<AtomicUsize>, discarded: Arc<AtomicUsize>) -> Self {
        Self {
            child: None,
            released: false,
            probe: Some((released, discarded)),
        }
    }
}

impl Drop for PendingProcessHandoff {
    fn drop(&mut self) {
        #[cfg(any(test, feature = "test-support"))]
        if !self.released {
            if let Some((_, discarded)) = &self.probe {
                discarded.fetch_add(1, Ordering::Relaxed);
            }
        }
        // The armed ManagedChild is dropped after this method and performs the
        // existing bounded process-tree cleanup, including an exited leader.
    }
}

/// Owns a freshly spawned long-lived process tree until its readiness handshake
/// succeeds. Dropping this guard terminates the tree; `detach` is the only path
/// that intentionally leaves the process running.
pub struct ManagedStartupChild {
    child: Option<Child>,
    process_tree: ProcessTree,
    termination_attempted: bool,
    termination_clock: Arc<dyn ProcessWaitClock>,
    #[cfg(any(test, feature = "test-support"))]
    termination_probe: Option<ManualStartupTerminationProbe>,
}

trait ProcessWaitClock: Send + Sync {
    fn elapsed(&self) -> Duration;
    fn sleep(&self, duration: Duration);
}

struct SystemProcessWaitClock(Instant);

impl SystemProcessWaitClock {
    fn new() -> Self {
        Self(Instant::now())
    }
}

impl ProcessWaitClock for SystemProcessWaitClock {
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
    clock: Arc<dyn ProcessWaitClock>,
}

impl StartupTerminationDeadline {
    fn new(budget: Duration, clock: Arc<dyn ProcessWaitClock>) -> Self {
        let started = clock.elapsed();
        Self {
            started,
            budget,
            clock,
        }
    }

    fn system(budget: Duration) -> Self {
        Self::new(budget, Arc::new(SystemProcessWaitClock::new()))
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

#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Default)]
struct ManualStartupTerminationState {
    elapsed: Duration,
    attempt_count: usize,
    cleanup_remaining_budgets: Vec<Duration>,
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Clone)]
pub struct ManualStartupTerminationProbe {
    leader_visible_after: Duration,
    state: Arc<std::sync::Mutex<ManualStartupTerminationState>>,
}

#[cfg(any(test, feature = "test-support"))]
impl ManualStartupTerminationProbe {
    pub fn new(leader_visible_after: Duration) -> Self {
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

    pub fn elapsed(&self) -> Duration {
        self.state
            .lock()
            .expect("startup termination probe")
            .elapsed
    }

    pub fn attempt_count(&self) -> usize {
        self.state
            .lock()
            .expect("startup termination probe")
            .attempt_count
    }

    pub fn cleanup_remaining_budgets(&self) -> Vec<Duration> {
        self.state
            .lock()
            .expect("startup termination probe")
            .cleanup_remaining_budgets
            .clone()
    }
}

#[cfg(any(test, feature = "test-support"))]
impl ProcessWaitClock for ManualStartupTerminationProbe {
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

/// Where starting a managed process failed.
///
/// Only [`ManagedSpawnFailure::Launch`] means the operating system did not
/// create the process; a caller may call that an unstartable program. A
/// failure to prepare the process tree or the working directory never tried
/// the program, and a failure to attach an existing process is not a failed
/// launch either.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManagedSpawnFailure {
    /// The process tree or the working directory was not ready, or the call
    /// was cancelled before launch; the program was not tried.
    Prepare(String),
    /// The operating system did not create the process.
    Launch(String),
    /// The process existed but could not be placed under Unica's control and
    /// was terminated.
    Attach(String),
}

impl ManagedSpawnFailure {
    pub fn into_message(self) -> String {
        match self {
            Self::Prepare(message) | Self::Launch(message) | Self::Attach(message) => message,
        }
    }
}

impl ManagedChild {
    pub fn spawn(command: ManagedCommand) -> Result<Self, String> {
        Self::spawn_staged(command).map_err(ManagedSpawnFailure::into_message)
    }

    /// [`ManagedChild::spawn`] that names where the start failed.
    pub fn spawn_staged(command: ManagedCommand) -> Result<Self, ManagedSpawnFailure> {
        let mut process = Command::new(&command.program);
        process.args(&command.args).current_dir(&command.cwd);
        for name in command.env_remove {
            process.env_remove(name);
        }
        process.envs(command.env);
        Self::spawn_process_staged(
            process,
            command.timeout,
            command.cancellation,
            command.capture_limits,
        )
    }

    pub fn spawn_process(
        process: Command,
        timeout: Option<Duration>,
        cancellation: CancellationToken,
    ) -> Result<Self, String> {
        Self::spawn_process_with_limits(process, timeout, cancellation, None)
    }

    fn spawn_process_with_limits(
        process: Command,
        timeout: Option<Duration>,
        cancellation: CancellationToken,
        capture_limits: Option<(usize, usize)>,
    ) -> Result<Self, String> {
        Self::spawn_process_staged(process, timeout, cancellation, capture_limits)
            .map_err(ManagedSpawnFailure::into_message)
    }

    fn spawn_process_staged(
        mut process: Command,
        timeout: Option<Duration>,
        cancellation: CancellationToken,
        capture_limits: Option<(usize, usize)>,
    ) -> Result<Self, ManagedSpawnFailure> {
        // A missing working directory fails the OS launch with the same error
        // as a missing program; name it before the program is tried.
        if let Some(cwd) = process.get_current_dir() {
            if !cwd.is_dir() {
                return Err(ManagedSpawnFailure::Prepare(
                    "process_failed: working directory is not available".to_string(),
                ));
            }
        }
        process
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut process_tree = ProcessTree::prepare(&mut process)
            .map_err(|error| ManagedSpawnFailure::Prepare(process_error(error)))?;
        // Cancellation and OS launch (including Windows Job attachment) are
        // serialized. A failed launch never claims the protected phase.
        let stage = std::cell::Cell::new(None);
        let gated = cancellation.spawn_with_gate(|| {
            let mut child = process.spawn().map_err(|error| {
                stage.set(Some(false));
                process_error(error)
            })?;
            #[cfg(all(any(test, feature = "test-support"), windows))]
            wait_at_job_attach_gate_for_test(&process);
            if let Err(error) = process_tree.attach(&mut child) {
                let _ = process_tree.terminate(&mut child);
                let _ = child.kill();
                let _ = child.try_wait();
                stage.set(Some(true));
                return Err(process_error(error));
            }
            Ok(child)
        });
        let (child, cancellation) = gated.map_err(|message| match stage.get() {
            Some(false) => ManagedSpawnFailure::Launch(message),
            Some(true) => ManagedSpawnFailure::Attach(message),
            None => ManagedSpawnFailure::Prepare(message),
        })?;

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

    #[cfg(any(test, feature = "test-support"))]
    pub fn run_pending_handoff(
        command: ManagedCommand,
    ) -> Result<(ManagedOutput, PendingProcessHandoff), String> {
        Self::run_pending_handoff_staged(command).map_err(|failure| match failure {
            PendingRunFailure::Spawn(failure) => failure.into_message(),
            PendingRunFailure::Run(message) => message,
        })
    }

    /// Run a launcher while retaining its exact process-tree authority until
    /// the upper layer validates the complete response. This path never uses
    /// the generic leader reap, which terminates surviving descendants.
    ///
    /// The failure keeps whether the launcher was ever started.
    pub fn run_pending_handoff_staged(
        command: ManagedCommand,
    ) -> Result<(ManagedOutput, PendingProcessHandoff), PendingRunFailure> {
        let mut child = Self::spawn_staged(command).map_err(PendingRunFailure::Spawn)?;
        let output = child
            .wait_for_pending_handoff_output()
            .map_err(PendingRunFailure::Run)?;
        Ok((
            output,
            PendingProcessHandoff {
                child: Some(child),
                released: false,
                #[cfg(any(test, feature = "test-support"))]
                probe: None,
            },
        ))
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

    /// Confirms that the owned leader was reaped, including after a pending
    /// termination. This does not prove completion of a whole runtime tree.
    pub fn poll_owned_leader_exit(&mut self) -> Result<bool, String> {
        if self.state == ChildState::Reaped {
            return Ok(true);
        }
        if self.try_wait_owned_leader()?.is_some() {
            self.process_tree.cleanup_after_leader_exit(&mut self.child);
            self.state = ChildState::Reaped;
            return Ok(true);
        }
        Ok(false)
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
        #[cfg(any(test, feature = "test-support"))]
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

    fn wait_for_pending_handoff_output(&mut self) -> Result<ManagedOutput, String> {
        drop(self.take_stdin());
        let (stdout_limit, stderr_limit) = self
            .capture_limits
            .unwrap_or((STDOUT_CAPTURE_LIMIT, STDERR_CAPTURE_LIMIT));
        let stdout = start_reader(self.take_stdout(), stdout_limit)
            .ok_or("process_failed: launch runner stdout pipe is unavailable")?;
        let stderr = start_reader(self.take_stderr(), stderr_limit)
            .ok_or("process_failed: launch runner stderr pipe is unavailable")?;
        let started = Instant::now();
        let mut leader_status = None;
        let mut captured_stdout = None;
        let mut captured_stderr = None;

        loop {
            if self.cancellation.is_cancelled() {
                self.terminate_gracefully()?;
                return Err(cancelled_error(
                    "launch runner was cancelled before handoff",
                ));
            }
            if self.timeout.is_some_and(|limit| started.elapsed() >= limit) {
                self.terminate()?;
                return Err("process_failed: launch runner timed out before handoff".into());
            }
            if leader_status.is_none() {
                leader_status = self
                    .process_tree
                    .observe_handoff_leader(&mut self.child)
                    .map_err(process_error)?;
            }
            if captured_stdout.is_none() {
                captured_stdout = receive_complete_capture(&stdout)?;
            }
            if captured_stderr.is_none() {
                captured_stderr = receive_complete_capture(&stderr)?;
            }
            if let Some(status) = leader_status {
                if captured_stdout.is_some() && captured_stderr.is_some() {
                    // Both streams reached real EOF while the exited leader still
                    // pins this process-group generation (or the Windows Job is held).
                    return Ok(finish_captured_output(
                        status,
                        captured_stdout.take().expect("checked stdout EOF"),
                        captured_stderr.take().expect("checked stderr EOF"),
                        false,
                        false,
                    ));
                }
            }
            thread::sleep(PROCESS_POLL_INTERVAL);
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
        self.wait_for_line_output_inner(
            StreamFraming::Lines {
                max_bytes: max_line_bytes,
            },
            &mut |number, bytes| match on_line(number, bytes) {
                StreamControl::Continue => ProcessLineReply::Continue,
                StreamControl::Stop => ProcessLineReply::Stop,
            },
            LineStreamInput::Closed,
        )
    }

    pub fn wait_for_controlled_line_output(
        &mut self,
        max_line_bytes: usize,
        on_line: &mut dyn FnMut(usize, &[u8]) -> ProcessLineReply,
    ) -> Result<ManagedLineOutput, String> {
        let stdin = self
            .take_stdin()
            .ok_or("process_failed: process stdin is unavailable")?;
        self.wait_for_line_output_inner(
            StreamFraming::Lines {
                max_bytes: max_line_bytes,
            },
            on_line,
            LineStreamInput::Replies(LineReplyWriter::start(stdin)),
        )
    }

    /// Stream stdout as `delimiter`-terminated records of any length while
    /// `input` is written to stdin, so neither side waits for the other.
    /// Nothing of stdout is retained beyond the record being delivered; an
    /// unterminated tail is reported in `line_error`.
    pub fn wait_for_record_output(
        &mut self,
        input: Vec<u8>,
        delimiter: u8,
        on_record: &mut dyn FnMut(&[u8]) -> StreamControl,
    ) -> Result<ManagedLineOutput, String> {
        self.wait_for_stream_output(input, StreamFraming::Records { delimiter }, on_record)
    }

    /// [`ManagedChild::wait_for_record_output`] for raw bytes: stdout is
    /// handed over in the pieces it is read in.
    pub fn wait_for_byte_output(
        &mut self,
        input: Vec<u8>,
        on_chunk: &mut dyn FnMut(&[u8]) -> StreamControl,
    ) -> Result<ManagedLineOutput, String> {
        self.wait_for_stream_output(input, StreamFraming::Chunks, on_chunk)
    }

    fn wait_for_stream_output(
        &mut self,
        input: Vec<u8>,
        framing: StreamFraming,
        on_record: &mut dyn FnMut(&[u8]) -> StreamControl,
    ) -> Result<ManagedLineOutput, String> {
        let mut stdin = self
            .take_stdin()
            .ok_or_else(|| "process_failed: process stdin is unavailable".to_string())?;
        let writer = thread::spawn(move || stdin.write_all(&input));
        let output = self.wait_for_line_output_inner(
            framing,
            &mut |_, bytes| match on_record(bytes) {
                StreamControl::Continue => ProcessLineReply::Continue,
                StreamControl::Stop => ProcessLineReply::Stop,
            },
            LineStreamInput::Closed,
        );
        let output = match output {
            Ok(output) => output,
            Err(error) => {
                // As in `run_with_input`: a failed wait no longer proves that
                // every descendant released stdin, so the writer is detached.
                self.terminate_after_wait_error();
                drop(writer);
                return Err(error);
            }
        };
        let write_result = writer
            .join()
            .map_err(|_| "process_failed: process stdin writer panicked".to_string())?;
        if !(output.timed_out || output.cancelled || output.stopped_by_consumer) {
            write_result.map_err(process_error)?;
        }
        Ok(output)
    }

    fn wait_for_line_output_inner(
        &mut self,
        framing: StreamFraming,
        on_line: &mut dyn FnMut(usize, &[u8]) -> ProcessLineReply,
        input: LineStreamInput,
    ) -> Result<ManagedLineOutput, String> {
        self.wait_for_line_output_inner_with_clock(
            framing,
            on_line,
            input,
            &SystemProcessWaitClock::new(),
        )
    }

    fn wait_for_line_output_inner_with_clock(
        &mut self,
        framing: StreamFraming,
        on_line: &mut dyn FnMut(usize, &[u8]) -> ProcessLineReply,
        mut input: LineStreamInput,
        clock: &dyn ProcessWaitClock,
    ) -> Result<ManagedLineOutput, String> {
        let stdout = self.take_stdout();
        let stderr = self.take_stderr();
        if input.is_controlled() && (stdout.is_none() || stderr.is_none()) {
            self.terminate_after_wait_error();
            return Err(
                "process_failed: controlled output requires stdout and stderr pipes".into(),
            );
        }
        let stdout = start_line_reader(stdout, framing);
        let stderr = start_reader(stderr, STDERR_CAPTURE_LIMIT);
        let started = clock.elapsed();
        let mut first_line_error = None;
        let mut leader_status = None;
        let mut stdout_eof = false;
        let mut protected_finish: Option<(Duration, Duration)> = None;

        loop {
            let reply_write_error = input.poll().err();
            let deadline_expired = self
                .timeout
                .is_some_and(|limit| clock.elapsed().saturating_sub(started) >= limit)
                || protected_finish
                    .is_some_and(|(anchor, limit)| clock.elapsed().saturating_sub(anchor) >= limit);
            let skip_drain = stdout_eof
                || input.is_writing()
                || reply_write_error.is_some()
                || (input.is_controlled() && deadline_expired);
            // An observer drain already waited up to one poll interval.
            let drain_waited = !skip_drain && !input.is_controlled();
            let drained = if skip_drain {
                LineDrain::Pending
            } else {
                drain_replied_line_messages(
                    &stdout,
                    on_line,
                    &mut first_line_error,
                    false,
                    input.kind(),
                )
            };
            let reply = match drained {
                LineDrain::Pending => ProcessLineReply::Continue,
                LineDrain::End => {
                    stdout_eof = true;
                    ProcessLineReply::Continue
                }
                LineDrain::Disconnected => {
                    self.terminate_after_wait_error();
                    return Err("process_failed: stdout reader disconnected before EOF".into());
                }
                LineDrain::Reply(reply) => reply,
            };
            // This response reports an effect that already happened in the
            // caller. A concurrent cancellation between that commit and this
            // match must not prevent its ACK or replace its factual outcome.
            let reply = match reply {
                ProcessLineReply::ReplyAndProtect {
                    reply,
                    finish_timeout,
                } => {
                    self.cancellation = CancellationToken::new();
                    match protected_finish.as_mut() {
                        Some((anchor, budget)) => {
                            *budget = (*budget).min(
                                clock
                                    .elapsed()
                                    .saturating_sub(*anchor)
                                    .saturating_add(finish_timeout),
                            );
                        }
                        None => protected_finish = Some((clock.elapsed(), finish_timeout)),
                    }
                    ProcessLineReply::Reply(reply)
                }
                reply => reply,
            };
            if reply == ProcessLineReply::Stop {
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
                        receive_output(stderr),
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
                    leader_status,
                    receive_output(stderr),
                    false,
                    false,
                    leader_status.is_none_or(|status: ExitStatus| status.success()),
                    first_line_error,
                ));
            }
            if let ProcessLineReply::Reply(bytes) = reply {
                if !self.cancellation.is_cancelled() {
                    if let Err(error) = input.reply(bytes) {
                        self.terminate_after_wait_error();
                        return Err(error);
                    }
                }
            }
            if self.cancellation.is_cancelled() {
                self.terminate_gracefully()?;
                drain_finishing_line_messages(
                    &stdout,
                    on_line,
                    &mut first_line_error,
                    input.kind(),
                );
                return Ok(finish_line_output(
                    None,
                    receive_output(stderr),
                    false,
                    true,
                    false,
                    first_line_error,
                ));
            }
            if self
                .timeout
                .is_some_and(|limit| clock.elapsed().saturating_sub(started) >= limit)
                || protected_finish
                    .is_some_and(|(anchor, limit)| clock.elapsed().saturating_sub(anchor) >= limit)
            {
                self.terminate()?;
                drain_finishing_line_messages(
                    &stdout,
                    on_line,
                    &mut first_line_error,
                    input.kind(),
                );
                return Ok(finish_line_output(
                    None,
                    receive_output(stderr),
                    true,
                    false,
                    false,
                    first_line_error,
                ));
            }
            if let Some(error) = reply_write_error {
                self.terminate_after_wait_error();
                return Err(error);
            }
            if let Some(status) = leader_status {
                if !input.is_writing() && stdout_eof {
                    let captured_stderr = if input.is_controlled() {
                        let receiver = stderr.as_ref().expect("controlled stderr was checked");
                        match receive_complete_capture(receiver) {
                            Ok(Some(captured)) => captured,
                            Ok(None) => {
                                clock.sleep(PROCESS_POLL_INTERVAL);
                                continue;
                            }
                            Err(error) => {
                                self.terminate_after_wait_error();
                                return Err(error);
                            }
                        }
                    } else {
                        receive_output(stderr)
                    };
                    return Ok(finish_line_output(
                        Some(status),
                        captured_stderr,
                        false,
                        false,
                        false,
                        first_line_error,
                    ));
                }
            }
            if leader_status.is_none() {
                if let Some(status) = self.try_wait_owned_leader()? {
                    self.process_tree.cleanup_after_leader_exit(&mut self.child);
                    self.state = ChildState::Reaped;
                    if !input.is_controlled() {
                        return Ok(finish_completed_line_output(
                            status,
                            &stdout,
                            stderr,
                            &mut |number, bytes| match on_line(number, bytes) {
                                ProcessLineReply::Continue => StreamControl::Continue,
                                ProcessLineReply::Reply(_)
                                | ProcessLineReply::ReplyAndProtect { .. }
                                | ProcessLineReply::Stop => StreamControl::Stop,
                            },
                            &mut first_line_error,
                        ));
                    }
                    leader_status = Some(status);
                    continue;
                }
            }
            if !drain_waited {
                clock.sleep(PROCESS_POLL_INTERVAL);
            }
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

/// Снять флаг наследования со стандартных дескрипторов этого процесса.
///
/// Хост создаёт stdio для Unica наследуемыми — иначе он не смог бы передать
/// их при запуске. На Windows `CreateProcessW` копирует ребёнку каждый
/// наследуемый дескриптор родителя, а не только те три, что назначены ему
/// как stdio. Так отсоединённый демон со `Stdio::null()` уносил с собой
/// копии труб MCP-хоста и держал их до собственного выхода: после выхода
/// сервера хост не видел EOF на stdout ещё четверть часа простоя демона.
///
/// `Stdio::inherit()` детей от этого не страдает: std дублирует дескриптор
/// для ребёнка заново и уже с наследованием. На Unix дескрипторы и так
/// закрываются при exec, поэтому там это пустая операция.
#[cfg(windows)]
pub fn detach_std_handles_from_inheritance() {
    use windows_sys::Win32::Foundation::{
        SetHandleInformation, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::System::Console::{
        GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
    };

    for id in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        // SAFETY: `GetStdHandle` takes a constant and returns a handle the
        // process already owns, null, or `INVALID_HANDLE_VALUE`.
        let handle = unsafe { GetStdHandle(id) };
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            continue;
        }
        // SAFETY: the handle stays live for the whole process; clearing the
        // inherit flag changes no I/O. A failure keeps the old behaviour and
        // is not worth refusing to start for.
        unsafe {
            SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0);
        }
    }
}

#[cfg(not(windows))]
pub fn detach_std_handles_from_inheritance() {}

impl ManagedStartupChild {
    pub fn spawn_configured(process: Command) -> Result<Self, String> {
        Self::spawn_configured_before(process, Instant::now() + TERMINATION_WAIT_LIMIT)
    }

    pub fn spawn_configured_before(
        mut process: Command,
        deadline: Instant,
    ) -> Result<Self, String> {
        if Instant::now() >= deadline {
            return Err("startup deadline expired before process creation".to_string());
        }
        let process_tree = ProcessTree::prepare_detachable(&mut process).map_err(process_error)?;
        let child = process.spawn().map_err(process_error)?;
        let mut managed = Self {
            child: Some(child),
            process_tree,
            termination_attempted: false,
            termination_clock: Arc::new(SystemProcessWaitClock::new()),
            #[cfg(any(test, feature = "test-support"))]
            termination_probe: None,
        };
        if let Err(error) = managed
            .process_tree
            .attach(managed.child.as_mut().expect("startup child exists"))
        {
            let cleanup = managed.terminate_bounded(
                deadline
                    .saturating_duration_since(Instant::now())
                    .min(TERMINATION_WAIT_LIMIT),
            );
            return match cleanup {
                Ok(()) => Err(process_error(error)),
                Err(cleanup_error) => Err(format!("{}; {cleanup_error}", process_error(error))),
            };
        }
        Ok(managed)
    }

    pub fn id(&self) -> u32 {
        self.child.as_ref().expect("startup child exists").id()
    }

    pub fn try_wait_status(&mut self) -> Result<Option<ExitStatus>, String> {
        self.process_tree
            .try_reap_leader(self.child.as_mut().expect("startup child exists"))
            .map_err(process_error)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn is_running(&mut self) -> Result<bool, String> {
        self.try_wait_status().map(|status| status.is_none())
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn install_termination_probe_for_test(&mut self, probe: &ManualStartupTerminationProbe) {
        self.termination_clock = Arc::new(probe.clone());
        self.termination_probe = Some(probe.clone());
    }

    pub fn terminate_bounded(&mut self, wait_limit: Duration) -> Result<(), String> {
        self.termination_attempted = true;
        #[cfg(any(test, feature = "test-support"))]
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
        #[cfg(any(test, feature = "test-support"))]
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
        #[cfg(any(test, feature = "test-support"))]
        if let Some(probe) = &self.termination_probe {
            probe.record_cleanup_remaining(deadline.remaining());
        }
        self.process_tree.cleanup_after_leader_exit_until(
            self.child.as_mut().expect("startup child exists"),
            deadline,
        );
    }

    pub fn detach(&mut self) -> Result<(), String> {
        self.detach_before(Instant::now() + TERMINATION_WAIT_LIMIT)
    }

    pub fn detach_before(&mut self, deadline: Instant) -> Result<(), String> {
        if Instant::now() >= deadline {
            return Err("startup deadline expired before detach".to_string());
        }
        let child = self.child.as_mut().expect("startup child exists");
        if self
            .process_tree
            .try_reap_leader(child)
            .map_err(process_error)?
            .is_some()
        {
            let cleanup_deadline = StartupTerminationDeadline::system(
                deadline.saturating_duration_since(Instant::now()),
            );
            self.process_tree
                .cleanup_after_leader_exit_until(child, &cleanup_deadline);
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

#[cfg(unix)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum ReapFailurePolicy {
    LoseAuthority,
    RetainVerifiedHandoff,
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

    #[cfg(any(test, feature = "test-support"))]
    fn released_for_test(pgid: i32) -> Self {
        Self {
            pgid,
            leader_pid: pgid as u32,
            generation: UnixGroupGeneration::Released,
        }
    }

    fn signal(&self, signal: i32) -> io::Result<()> {
        self.signal_with(signal, |group, signal| {
            #[cfg(any(test, feature = "test-support"))]
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

    #[cfg(any(test, feature = "test-support"))]
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

#[cfg(all(any(test, feature = "test-support"), unix))]
pub fn assert_released_unix_group_never_signals_reused_identity_for_test() {
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

    fn observe_handoff_leader(&mut self, child: &mut Child) -> io::Result<Option<ExitStatus>> {
        self.observe_leader_exit(child)
    }

    fn release_handoff(&mut self, child: &mut Child) -> io::Result<()> {
        if !self.retains_exact_exited_leader(child.id()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "launch handoff no longer owns the exited runner leader",
            ));
        }
        // Reap without signalling the group: validation has transferred the
        // surviving client to the caller, and the zombie pins its PGID until now.
        self.reap_observed_leader_with_policy(child, ReapFailurePolicy::RetainVerifiedHandoff)
            .map(|_| ())
    }

    fn reap_observed_leader(&mut self, child: &mut Child) -> io::Result<ExitStatus> {
        self.reap_observed_leader_with_policy(child, ReapFailurePolicy::LoseAuthority)
    }

    fn reap_observed_leader_with_policy(
        &mut self,
        child: &mut Child,
        failure_policy: ReapFailurePolicy,
    ) -> io::Result<ExitStatus> {
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
        #[cfg(any(test, feature = "test-support"))]
        if INJECT_UNIX_REAP_ERROR.with(|slot| slot.replace(false)) {
            return self.reap_failure(
                child,
                observed_status,
                io::Error::other("injected Unix reap failure"),
                failure_policy,
            );
        }
        let status = match child.try_wait() {
            Ok(Some(status)) => status,
            Err(error)
                if error.raw_os_error() == Some(libc::ECHILD)
                    && failure_policy == ReapFailurePolicy::LoseAuthority =>
            {
                observed_status
            }
            Ok(None) => {
                return self.reap_failure(
                    child,
                    observed_status,
                    io::Error::other("retained managed-process leader could not be reaped"),
                    failure_policy,
                );
            }
            Err(error) => {
                if error.raw_os_error() == Some(libc::ECHILD) {
                    // The handoff must not release a group after another wait
                    // owner has reaped its leader. That numeric PGID can be reused.
                    self.lose_authority();
                    return Err(error);
                }
                return self.reap_failure(child, observed_status, error, failure_policy);
            }
        };
        if let Some(mut authority) = self.process_group.take() {
            authority.release();
        }
        self.ownership_pipe = None;
        self.leader = UnixLeaderOwnership::Reaped(status);
        Ok(status)
    }

    fn reap_failure<T>(
        &mut self,
        child: &Child,
        observed_status: ExitStatus,
        error: io::Error,
        failure_policy: ReapFailurePolicy,
    ) -> io::Result<T> {
        let still_owned = failure_policy == ReapFailurePolicy::RetainVerifiedHandoff
            && self.retains_exact_exited_leader(child.id())
            && matches!(
                unix_leader_exit_unreaped(child.id()),
                Ok(Some(status)) if status == observed_status
            );
        if !still_owned {
            self.lose_authority();
        }
        Err(error)
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
    exit_census: Option<windows_job_exit::Census>,
}

// SAFETY: Windows kernel handles may be transferred and used from other threads.
#[cfg(windows)]
unsafe impl Send for ProcessTree {}

// SAFETY: Job Object APIs support concurrent access. The Census receiver and
// its mutable coverage are private and accessed only through exclusive &mut self.
#[cfg(windows)]
unsafe impl Sync for ProcessTree {}

#[cfg(windows)]
impl ProcessTree {
    fn prepare(command: &mut Command) -> io::Result<Self> {
        Self::prepare_with_policy(command, true)
    }

    fn prepare_runtime(command: &mut Command) -> io::Result<Self> {
        let mut tree = Self::prepare(command)?;
        tree.exit_census = Some(windows_job_exit::Census::associate(tree.job)?);
        Ok(tree)
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

        Ok(Self {
            job,
            exit_census: None,
        })
    }

    fn attach(&mut self, child: &mut Child) -> io::Result<Option<ExitStatus>> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;
        use windows_sys::Win32::System::Threading::ResumeThread;

        // SAFETY: both handles are live for the duration of the call.
        if unsafe { AssignProcessToJobObject(self.job, child.as_raw_handle() as _) } == 0 {
            return Err(io::Error::last_os_error());
        }
        if let Some(census) = &mut self.exit_census {
            census.retain_assigned_root(child.id(), child.as_raw_handle() as _)?;
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

    fn observe_handoff_leader(&mut self, child: &mut Child) -> io::Result<Option<ExitStatus>> {
        child.try_wait()
    }

    fn release_handoff(&mut self, _child: &mut Child) -> io::Result<()> {
        use std::mem::{size_of, zeroed};
        use windows_sys::Win32::System::JobObjects::{
            JobObjectExtendedLimitInformation, QueryInformationJobObject, SetInformationJobObject,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        };

        // Keep crash cleanup armed on the same owned Job until the caller has
        // verified the receipt. Preserve all other limits when releasing it.
        // SAFETY: this Windows POD structure is valid when zero-initialized.
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
        // SAFETY: the Job is still owned and the class, pointer and size agree.
        if unsafe {
            QueryInformationJobObject(
                self.job,
                JobObjectExtendedLimitInformation,
                &mut limits as *mut _ as *mut _,
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        if limits.BasicLimitInformation.LimitFlags & JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE == 0 {
            return Err(io::Error::other(
                "pending launch Job has no crash cleanup policy",
            ));
        }
        limits.BasicLimitInformation.LimitFlags &= !JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: this updates the live owned Job with its queried limits.
        // An error retains the handle and armed guard for explicit cleanup.
        if unsafe {
            SetInformationJobObject(
                self.job,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const _,
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        self.detach()
    }

    fn observe_runtime_members(&mut self) -> io::Result<()> {
        if let Some(census) = &mut self.exit_census {
            census.observe(self.job)?;
        }
        Ok(())
    }

    fn is_empty(&mut self) -> io::Result<bool> {
        if let Some(census) = &mut self.exit_census {
            return census.ready(self.job);
        }
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
        // The startup Job was created without KILL_ON_JOB_CLOSE; a verified
        // client handoff clears it above. Closing releases either ready process.
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

    fn observe_handoff_leader(&mut self, child: &mut Child) -> io::Result<Option<ExitStatus>> {
        child.try_wait()
    }

    fn release_handoff(&mut self, _child: &mut Child) -> io::Result<()> {
        self.detach()
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
    complete: bool,
}

enum LineMessage {
    End,
    Line(usize, Vec<u8>),
    TooLong(usize),
    Unterminated(usize),
    ReadError(usize, String),
    /// The records completed by one read, handed over together: one
    /// handover per record would cost a thread switch each.
    Records(Vec<(usize, Vec<u8>)>),
}

enum LineDrain {
    Pending,
    End,
    Disconnected,
    Reply(ProcessLineReply),
}

/// How a stdout stream is cut into the units its consumer receives.
#[derive(Debug, Clone, Copy)]
enum StreamFraming {
    /// LF-terminated lines of at most `max_bytes`; an unterminated tail is
    /// still a line.
    Lines { max_bytes: usize },
    /// `delimiter`-terminated records of any length; an unterminated tail is
    /// an error, never a record.
    Records { delimiter: u8 },
    /// The bytes exactly as read, in pieces of any size.
    Chunks,
}

/// One read of a byte stream: a step of the stream, not a bound of its size.
const BYTE_STREAM_CHUNK_BYTES: usize = 64 * 1024;

fn start_chunk_reader<R>(pipe: Option<R>) -> Option<Receiver<LineMessage>>
where
    R: Read + Send + 'static,
{
    pipe.map(|mut pipe| {
        let (sender, receiver) = mpsc::sync_channel(1);
        thread::spawn(move || {
            let mut chunk = vec![0_u8; BYTE_STREAM_CHUNK_BYTES];
            let mut number = 1_usize;
            loop {
                match pipe.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(count) => {
                        let message = LineMessage::Line(number, chunk[..count].to_vec());
                        if sender.send(message).is_err() {
                            return;
                        }
                        number = number.saturating_add(1);
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                    Err(error) => {
                        let _ = sender.send(LineMessage::ReadError(number, error.to_string()));
                        return;
                    }
                }
            }
            let _ = sender.send(LineMessage::End);
        });
        receiver
    })
}

fn start_line_reader<R>(pipe: Option<R>, framing: StreamFraming) -> Option<Receiver<LineMessage>>
where
    R: Read + Send + 'static,
{
    let (delimiter, max_bytes) = match framing {
        StreamFraming::Lines { max_bytes } => (b'\n', Some(max_bytes)),
        StreamFraming::Records { delimiter } => (delimiter, None),
        StreamFraming::Chunks => return start_chunk_reader(pipe),
    };
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
                let mut rest = &chunk[..count];
                let mut records = Vec::new();
                while !rest.is_empty() {
                    let end = rest.iter().position(|byte| *byte == delimiter);
                    let part = &rest[..end.unwrap_or(rest.len())];
                    if !too_long {
                        if max_bytes.is_some_and(|max| line.len() + part.len() > max) {
                            line.clear();
                            too_long = true;
                        } else {
                            line.extend_from_slice(part);
                        }
                    }
                    let Some(end) = end else { break };
                    rest = &rest[end + 1..];
                    let message = if too_long {
                        LineMessage::TooLong(line_number)
                    } else if matches!(framing, StreamFraming::Records { .. }) {
                        records.push((line_number, std::mem::take(&mut line)));
                        line_number = line_number.saturating_add(1);
                        continue;
                    } else {
                        LineMessage::Line(line_number, std::mem::take(&mut line))
                    };
                    if sender.send(message).is_err() {
                        return;
                    }
                    line.clear();
                    too_long = false;
                    line_number = line_number.saturating_add(1);
                }
                if !records.is_empty() && sender.send(LineMessage::Records(records)).is_err() {
                    return;
                }
            }
            if too_long {
                let _ = sender.send(LineMessage::TooLong(line_number));
            } else if !line.is_empty() {
                let message = match framing {
                    StreamFraming::Lines { .. } => LineMessage::Line(line_number, line),
                    StreamFraming::Records { .. } | StreamFraming::Chunks => {
                        LineMessage::Unterminated(line_number)
                    }
                };
                let _ = sender.send(message);
            }
            let _ = sender.send(LineMessage::End);
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
    match drain_replied_line_messages(
        receiver,
        &mut |number, bytes| match on_line(number, bytes) {
            StreamControl::Continue => ProcessLineReply::Continue,
            StreamControl::Stop => ProcessLineReply::Stop,
        },
        first_error,
        wait_for_end,
        LineStreamKind::Observe,
    ) {
        LineDrain::Pending
        | LineDrain::End
        | LineDrain::Disconnected
        | LineDrain::Reply(ProcessLineReply::Continue) => StreamControl::Continue,
        LineDrain::Reply(ProcessLineReply::Reply(_))
        | LineDrain::Reply(ProcessLineReply::ReplyAndProtect { .. })
        | LineDrain::Reply(ProcessLineReply::Stop) => StreamControl::Stop,
    }
}

fn drain_finishing_line_messages(
    receiver: &Option<Receiver<LineMessage>>,
    on_line: &mut dyn FnMut(usize, &[u8]) -> ProcessLineReply,
    first_error: &mut Option<(usize, String)>,
    kind: LineStreamKind,
) {
    // Legacy observers still receive the final drain after cancellation or
    // timeout. A controlled callback could publish a new effect, so never
    // invoke it after the owned process has already been stopped.
    let _ = drain_replied_line_messages(
        receiver,
        &mut |number, bytes| match kind {
            LineStreamKind::Observe => on_line(number, bytes),
            LineStreamKind::Controlled => ProcessLineReply::Continue,
        },
        first_error,
        true,
        LineStreamKind::Observe,
    );
}

fn drain_replied_line_messages(
    receiver: &Option<Receiver<LineMessage>>,
    on_line: &mut dyn FnMut(usize, &[u8]) -> ProcessLineReply,
    first_error: &mut Option<(usize, String)>,
    wait_for_end: bool,
    kind: LineStreamKind,
) -> LineDrain {
    let Some(receiver) = receiver else {
        return match kind {
            LineStreamKind::Observe => LineDrain::End,
            LineStreamKind::Controlled => LineDrain::Disconnected,
        };
    };
    // An observer waits for the next line within one poll interval instead
    // of returning at the first empty channel: the reader hands over one line
    // at a time, so polling would cap a long stream at one line per interval.
    let observe_slice_end = Instant::now() + PROCESS_POLL_INTERVAL;
    loop {
        let wait = if wait_for_end {
            Some(READER_WAIT_LIMIT)
        } else if matches!(kind, LineStreamKind::Observe) {
            Some(observe_slice_end.saturating_duration_since(Instant::now()))
        } else {
            None
        };
        let message = if let Some(wait) = wait {
            match receiver.recv_timeout(wait) {
                Ok(message) => message,
                Err(mpsc::RecvTimeoutError::Timeout) => return LineDrain::Pending,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return match kind {
                        LineStreamKind::Observe => LineDrain::End,
                        LineStreamKind::Controlled => LineDrain::Disconnected,
                    };
                }
            }
        } else {
            match receiver.try_recv() {
                Ok(message) => message,
                Err(TryRecvError::Empty) => return LineDrain::Pending,
                Err(TryRecvError::Disconnected) => {
                    return match kind {
                        LineStreamKind::Observe => LineDrain::End,
                        LineStreamKind::Controlled => LineDrain::Disconnected,
                    };
                }
            }
        };
        match message {
            LineMessage::End => return LineDrain::End,
            LineMessage::Line(number, bytes) => {
                let reply = on_line(number, &bytes);
                if reply != ProcessLineReply::Continue {
                    return LineDrain::Reply(reply);
                }
            }
            LineMessage::Records(records) => {
                for (number, bytes) in records {
                    let reply = on_line(number, &bytes);
                    if reply != ProcessLineReply::Continue {
                        return LineDrain::Reply(reply);
                    }
                }
            }
            LineMessage::TooLong(number) => {
                first_error.get_or_insert_with(|| (number, STREAM_LINE_TOO_LONG_ERROR.to_string()));
            }
            LineMessage::Unterminated(number) => {
                first_error
                    .get_or_insert_with(|| (number, STREAM_RECORD_UNTERMINATED_ERROR.to_string()));
            }
            LineMessage::ReadError(number, error) => {
                first_error.get_or_insert_with(|| {
                    (number, format!("failed to read process stdout: {error}"))
                });
            }
        }
        if matches!(kind, LineStreamKind::Controlled) && first_error.is_some() {
            return LineDrain::Reply(ProcessLineReply::Stop);
        }
        // Controlled callbacks yield after every line, including the queued
        // tail after leader exit, so each line remains inside its deadline.
        if matches!(kind, LineStreamKind::Controlled) {
            return LineDrain::Reply(ProcessLineReply::Continue);
        }
    }
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
                    Ok(0) => {
                        captured.complete = true;
                        break;
                    }
                    Err(_) => break,
                    Ok(count) => retain_tail(&mut captured, &chunk[..count], limit),
                }
            }
            let _ = sender.send(captured);
        });
        receiver
    })
}

fn receive_complete_capture(
    receiver: &Receiver<CapturedOutput>,
) -> Result<Option<CapturedOutput>, String> {
    match receiver.try_recv() {
        Ok(captured) if captured.complete => Ok(Some(captured)),
        Ok(_) => Err("process_failed: launch runner output ended without EOF".into()),
        Err(TryRecvError::Empty) => Ok(None),
        Err(TryRecvError::Disconnected) => {
            Err("process_failed: launch runner output reader disconnected".into())
        }
    }
}

fn finish_output(
    status: ExitStatus,
    stdout: Option<Receiver<CapturedOutput>>,
    stderr: Option<Receiver<CapturedOutput>>,
    timed_out: bool,
    cancelled: bool,
) -> ManagedOutput {
    finish_captured_output(
        status,
        receive_output(stdout),
        receive_output(stderr),
        timed_out,
        cancelled,
    )
}

fn finish_captured_output(
    status: ExitStatus,
    stdout: CapturedOutput,
    mut stderr: CapturedOutput,
    timed_out: bool,
    cancelled: bool,
) -> ManagedOutput {
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

fn finish_completed_line_output<F>(
    status: ExitStatus,
    stdout: &Option<Receiver<LineMessage>>,
    stderr: Option<Receiver<CapturedOutput>>,
    on_line: &mut F,
    first_line_error: &mut Option<(usize, String)>,
) -> ManagedLineOutput
where
    F: FnMut(usize, &[u8]) -> StreamControl,
{
    let stopped_by_consumer = drain_line_messages(stdout, on_line, first_line_error, true)
        == StreamControl::Stop
        && status.success();
    finish_line_output(
        Some(status),
        receive_output(stderr),
        false,
        false,
        stopped_by_consumer,
        first_line_error.take(),
    )
}

fn finish_line_output(
    status: Option<ExitStatus>,
    stderr: CapturedOutput,
    timed_out: bool,
    cancelled: bool,
    stopped_by_consumer: bool,
    line_error: Option<(usize, String)>,
) -> ManagedLineOutput {
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

pub fn ensure_truncation_diagnostics(output: &mut ManagedOutput) {
    let mut captured = CapturedOutput {
        bytes: output.stderr.as_bytes().to_vec(),
        truncated: output.stderr_truncated,
        complete: true,
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
        ManagedStartupChild, ManualStartupTerminationProbe, ProcessLineReply, StreamControl,
    };
    use std::ffi::OsString;
    use std::io::{BufRead, Read, Write};
    use std::path::{Path, PathBuf};
    use std::process::Child;
    use std::process::{Command, Stdio};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
    use unica_work_control::cancellation::CancellationToken;

    const HELPER_ENV: &str = "UNICA_MANAGED_CHILD_HELPER";
    const HELPER_PID_FILE_ENV: &str = "UNICA_MANAGED_CHILD_PID_FILE";
    const HELPER_STARTED_ENV: &str = "UNICA_MANAGED_CHILD_STARTED";

    /// Runtime proof that the helper body ran. Launched without `--ignored`,
    /// libtest answers `running 0 tests` and exits 0, so a parent that only
    /// terminates, observes or counts output lines would pass without its
    /// helper. The helper writes this marker before its mode starts.
    struct HelperStart {
        _directory: tempfile::TempDir,
        marker: PathBuf,
    }

    impl HelperStart {
        fn new() -> Self {
            let directory = tempfile::tempdir().unwrap();
            let marker = directory.path().join("started");
            Self {
                _directory: directory,
                marker,
            }
        }

        fn env(&self) -> (OsString, OsString) {
            (
                OsString::from(HELPER_STARTED_ENV),
                self.marker.clone().into_os_string(),
            )
        }

        fn assert_started(&self) {
            wait_for_helper_start(&self.marker);
        }
    }

    fn wait_for_helper_start(marker: &Path) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !marker.exists() {
            assert!(
                Instant::now() < deadline,
                "managed_child_test_helper did not start; was it launched with --ignored?"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }

    /// Marker of the `process_tree_child` grandchild. The intermediate helper
    /// waits for it before publishing the grandchild PID, so a descendant
    /// that never ran cannot pass as one that was killed.
    fn grandchild_started_marker(pid_file: &std::ffi::OsStr) -> PathBuf {
        PathBuf::from(format!("{}.started", pid_file.to_string_lossy()))
    }
    const CONTROLLED_STDERR_SOCKET_ENV: &str = "UNICA_CONTROLLED_STDERR_SOCKET";
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

    fn helper_expect_line_reply(expected: &[u8]) {
        let mut reply = Vec::new();
        std::io::stdin()
            .lock()
            .read_until(b'\n', &mut reply)
            .unwrap();
        assert_eq!(reply, expected);
    }

    fn helper_frame(bytes: &[u8]) {
        let mut stdout = std::io::stdout().lock();
        stdout.write_all(bytes).unwrap();
        stdout.write_all(b"\n").unwrap();
        stdout.flush().unwrap();
    }

    fn helper_close_stdin() {
        #[cfg(unix)]
        // SAFETY: this fixture closes only its own inherited stdin descriptor.
        assert_eq!(unsafe { libc::close(libc::STDIN_FILENO) }, 0);
        #[cfg(windows)]
        {
            use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
            use windows_sys::Win32::System::Console::{GetStdHandle, STD_INPUT_HANDLE};
            // SAFETY: this fixture closes only its own inherited stdin handle.
            let stdin = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
            assert!(!stdin.is_null() && stdin != INVALID_HANDLE_VALUE);
            assert_ne!(unsafe { CloseHandle(stdin) }, 0);
        }
    }

    /// Ревью #549: незапуском программы называется только отказ ОС создать
    /// процесс. Отсутствующий рабочий каталог программу не пробовал, хотя ОС
    /// ответила бы на него той же ошибкой, что на отсутствующий файл.
    #[test]
    fn only_a_refused_launch_is_a_launch_failure() {
        let root = tempfile::tempdir().unwrap();
        let command = |program: PathBuf, cwd: PathBuf| ManagedCommand {
            program,
            args: Vec::new(),
            cwd,
            env: Vec::new(),
            env_remove: Vec::new(),
            capture_limits: None,
            timeout: None,
            cancellation: CancellationToken::new(),
        };
        let missing_program = ManagedChild::spawn_staged(command(
            root.path().join("no-such-program"),
            root.path().to_path_buf(),
        ));
        assert!(
            matches!(missing_program, Err(super::ManagedSpawnFailure::Launch(_))),
            "{:?}",
            missing_program.err()
        );
        let missing_cwd = ManagedChild::spawn_staged(command(
            std::env::current_exe().unwrap(),
            root.path().join("no-such-directory"),
        ));
        assert!(
            matches!(missing_cwd, Err(super::ManagedSpawnFailure::Prepare(_))),
            "{:?}",
            missing_cwd.err()
        );
    }

    #[test]
    #[ignore = "helper process: spawned by the managed-child tests with HELPER_ENV; not a standalone test"]
    #[allow(clippy::zombie_processes)] // Fixture intentionally exits while its descendant remains alive.
    fn managed_child_test_helper() {
        let mode =
            std::env::var(HELPER_ENV).expect("helper process: the parent test sets HELPER_ENV");
        if let Some(marker) = std::env::var_os(HELPER_STARTED_ENV) {
            std::fs::write(marker, &mode).expect("record helper start");
        }

        match mode.as_str() {
            "success" => {
                print!("managed stdout");
                eprint!("managed stderr");
            }
            #[cfg(windows)]
            "windows_finished_child_parent" => {
                let mut child = Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "platform::process::tests::managed_child_test_helper",
                        "--ignored",
                        "--nocapture",
                    ])
                    .env(HELPER_ENV, "success")
                    .env_remove(HELPER_STARTED_ENV)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .unwrap();
                std::fs::write(
                    std::env::var_os(HELPER_PID_FILE_ENV).unwrap(),
                    child.id().to_string(),
                )
                .unwrap();
                let status = child.wait().unwrap();
                // Close the last fixture-owned descendant handle before the
                // root can signal. The outer observer retains only the root.
                drop(child);
                assert!(status.success(), "finished-child helper failed: {status}");
            }
            "read_stdin" => {
                let mut input = String::new();
                std::io::stdin().read_to_string(&mut input).unwrap();
                print!("stdin closed");
            }
            "echo_stdin" => {
                // The harness has already printed its banner; exiting here
                // keeps its trailer out of the echoed stream.
                let mut stdout = std::io::stdout().lock();
                std::io::copy(&mut std::io::stdin(), &mut stdout).unwrap();
                stdout.flush().unwrap();
                std::process::exit(0);
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
            "controlled_two_replies" => {
                helper_frame(br#"{"event":"prepare"}"#);
                helper_expect_line_reply(b"{\"decision\":\"stage\"}\n");
                helper_frame(br#"{"event":"ready"}"#);
                helper_expect_line_reply(b"{\"decision\":\"commit\"}\n");
                helper_frame(br#"{"event":"terminal"}"#);
                std::process::exit(0);
            }
            "controlled_empty_eof" => std::process::exit(0),
            "controlled_invalid_frame" => {
                helper_frame(b"{\"event\":\"bad\"}\xff");
                thread::sleep(Duration::from_secs(10));
            }
            "controlled_oversized_frame" => {
                helper_frame(&vec![b'x'; 8193]);
                thread::sleep(Duration::from_secs(10));
            }
            "controlled_closed_stdin" => {
                helper_close_stdin();
                helper_frame(br#"{"event":"ready"}"#);
                thread::sleep(Duration::from_secs(10));
            }
            "controlled_wait_after_reply" => {
                helper_frame(br#"{"event":"ready"}"#);
                helper_expect_line_reply(b"{\"decision\":\"commit\"}\n");
                helper_frame(br#"{"event":"waiting"}"#);
                thread::sleep(Duration::from_secs(10));
            }
            "controlled_tail_after_reply" => {
                helper_frame(br#"{"event":"ready"}"#);
                helper_expect_line_reply(b"{\"decision\":\"commit\"}\n");
                let release = PathBuf::from(std::env::var_os(HELPER_PID_FILE_ENV).unwrap());
                let watchdog = Instant::now() + Duration::from_secs(5);
                while !release.exists() {
                    assert!(
                        Instant::now() < watchdog,
                        "tail fixture exit was not released"
                    );
                    thread::yield_now();
                }
                // The complete tail fits into every supported host's pipe,
                // so the owner can observe real exit before draining it.
                for _ in 0..16 {
                    helper_frame(br#"{"event":"tail"}"#);
                }
                std::process::exit(0);
            }
            "controlled_stderr_writer" | "controlled_stderr_leader" => {
                let address = std::env::var(CONTROLLED_STDERR_SOCKET_ENV).unwrap();
                let mut control = std::net::TcpStream::connect(address).unwrap();
                control
                    .set_read_timeout(Some(Duration::from_secs(10)))
                    .unwrap();
                if mode == "controlled_stderr_writer" {
                    std::io::stderr().write_all(b"stderr-before\n").unwrap();
                    std::io::stderr().flush().unwrap();
                    control.write_all(b"w").unwrap();
                    control.read_exact(&mut [0_u8]).unwrap();
                    std::io::stderr().write_all(b"stderr-after\n").unwrap();
                    std::io::stderr().flush().unwrap();
                } else {
                    control.write_all(b"l").unwrap();
                    helper_frame(br#"{"event":"terminal"}"#);
                    control.read_exact(&mut [0_u8]).unwrap();
                }
                std::process::exit(0);
            }
            "controlled_max_reply" => {
                helper_frame(br#"{"event":"ready"}"#);
                let mut reply = Vec::new();
                std::io::stdin()
                    .lock()
                    .read_until(b'\n', &mut reply)
                    .unwrap();
                assert_eq!(reply.len(), 4096);
                assert!(reply.starts_with(b"{\"padding\":\""));
                assert!(reply.ends_with(b"\"}\n"));
                helper_frame(br#"{"event":"terminal"}"#);
                std::process::exit(0);
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
            "short_sleep" => thread::sleep(Duration::from_millis(800)),
            "stream_forever" => loop {
                println!("streamed line");
                std::io::Write::flush(&mut std::io::stdout()).unwrap();
                thread::sleep(Duration::from_millis(5));
            },
            "stream_then_exit" => {
                for _ in 0..202 {
                    println!("x");
                }
            }
            "stream_then_fail" => {
                for _ in 0..202 {
                    println!("x");
                }
                eprintln!("expected child failure");
                std::process::exit(128);
            }
            "process_tree_immediate_parent" => {
                let pid_file = std::env::var_os(HELPER_PID_FILE_ENV).unwrap();
                let grandchild_marker = grandchild_started_marker(&pid_file);
                let mut child = Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "platform::process::tests::managed_child_test_helper",
                        "--ignored",
                        "--nocapture",
                    ])
                    .env(HELPER_ENV, "process_tree_child")
                    .env(HELPER_STARTED_ENV, &grandchild_marker)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .unwrap();
                wait_for_helper_start(&grandchild_marker);
                std::fs::remove_file(&grandchild_marker).unwrap();
                std::fs::write(
                    pid_file,
                    format!("{}\n{}\n", std::process::id(), child.id()),
                )
                .unwrap();
                child.wait().unwrap();
            }
            "inherited_pipe_parent" => {
                let pid_file = std::env::var_os(HELPER_PID_FILE_ENV).unwrap();
                let grandchild_marker = grandchild_started_marker(&pid_file);
                let child = Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "platform::process::tests::managed_child_test_helper",
                        "--ignored",
                        "--nocapture",
                    ])
                    .env(HELPER_ENV, "process_tree_child")
                    .env(HELPER_STARTED_ENV, &grandchild_marker)
                    .spawn()
                    .unwrap();
                print!("inherited-pipe-before-timeout");
                std::io::Write::flush(&mut std::io::stdout()).unwrap();
                wait_for_helper_start(&grandchild_marker);
                std::fs::remove_file(&grandchild_marker).unwrap();
                std::fs::write(
                    pid_file,
                    format!("{}\n{}\n", std::process::id(), child.id()),
                )
                .unwrap();
                thread::sleep(Duration::from_secs(10));
            }
            "process_tree_child" => thread::sleep(Duration::from_secs(10)),
            #[cfg(windows)]
            "windows_parked_handoff_client" => loop {
                thread::park();
            },
            #[cfg(windows)]
            "windows_exit_leaving_parked_client" => {
                super::detach_std_handles_from_inheritance();
                let pid_file = PathBuf::from(std::env::var_os(HELPER_PID_FILE_ENV).unwrap());
                let mut client = ChildCleanupGuard(Some(
                    Command::new(std::env::current_exe().unwrap())
                        .args([
                            "--exact",
                            "platform::process::tests::managed_child_test_helper",
                            "--ignored",
                            "--nocapture",
                        ])
                        .env(HELPER_ENV, "windows_parked_handoff_client")
                        .env_remove(HELPER_STARTED_ENV)
                        .stdin(Stdio::null())
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .spawn()
                        .unwrap(),
                ));
                publish_handoff_probe_file(&pid_file, &client.child_mut().id().to_string());
                // The owner above this launcher retains the Job authority.
                drop(client.0.take());
                print!("grandchild-detached");
                std::io::stdout().flush().unwrap();
            }
            #[cfg(windows)]
            "windows_pending_handoff_owner" | "windows_released_handoff_owner" => {
                let pid_file = PathBuf::from(std::env::var_os(HELPER_PID_FILE_ENV).unwrap());
                let (output, mut pending) =
                    ManagedChild::run_pending_handoff(pending_handoff_command(
                        "windows_exit_leaving_parked_client",
                        &pid_file,
                        CancellationToken::new(),
                    ))
                    .unwrap();
                assert!(output.status_success, "{output:?}");
                assert!(output.stdout.contains("grandchild-detached"));
                if mode == "windows_released_handoff_owner" {
                    pending.release().unwrap();
                }
                publish_handoff_probe_file(&pid_file.with_extension("ready"), "ready");
                // The outer test terminates this process without running Drop.
                // EOF on an abandoned test instead lets the armed guard clean up.
                let _ = std::io::stdin().read_exact(&mut [0_u8]);
                drop(pending);
            }
            "exit_leaving_detached_grandchild" => {
                super::detach_std_handles_from_inheritance();
                let pid_file = std::env::var_os(HELPER_PID_FILE_ENV).unwrap();
                let grandchild = Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "platform::process::tests::managed_child_test_helper",
                        "--ignored",
                        "--nocapture",
                    ])
                    .env(HELPER_ENV, "process_tree_child")
                    .env_remove(HELPER_STARTED_ENV)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .unwrap();
                std::fs::write(pid_file, format!("{}\n", grandchild.id())).unwrap();
                print!("grandchild-detached");
                std::io::Write::flush(&mut std::io::stdout()).unwrap();
            }
            "exit_leaving_inherited_pipe_grandchild" => {
                let pid_file = std::env::var_os(HELPER_PID_FILE_ENV).unwrap();
                let grandchild = Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "platform::process::tests::managed_child_test_helper",
                        "--ignored",
                        "--nocapture",
                    ])
                    .env(HELPER_ENV, "process_tree_child")
                    .env_remove(HELPER_STARTED_ENV)
                    .spawn()
                    .unwrap();
                print!("{{\"ok\":true,\"pid\":{}}}", grandchild.id());
                std::io::Write::flush(&mut std::io::stdout()).unwrap();
                std::fs::write(
                    pid_file,
                    format!("{}\n{}\n", std::process::id(), grandchild.id()),
                )
                .unwrap();
            }
            #[cfg(unix)]
            "graceful_runner_with_external_process_group" => {
                use std::os::unix::process::CommandExt;

                install_helper_sigterm_handler();
                let pid_file = std::env::var_os(HELPER_PID_FILE_ENV).unwrap();
                let grandchild_marker = grandchild_started_marker(&pid_file);
                let mut command = Command::new(std::env::current_exe().unwrap());
                command
                    .args([
                        "--exact",
                        "platform::process::tests::managed_child_test_helper",
                        "--ignored",
                        "--nocapture",
                    ])
                    .env(HELPER_ENV, "process_tree_child")
                    .env(HELPER_STARTED_ENV, &grandchild_marker)
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
                wait_for_helper_start(&grandchild_marker);
                std::fs::remove_file(&grandchild_marker).unwrap();
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
                let grandchild_marker = grandchild_started_marker(&pid_file);
                let child = Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "platform::process::tests::managed_child_test_helper",
                        "--ignored",
                        "--nocapture",
                    ])
                    .env(HELPER_ENV, "process_tree_child")
                    .env(HELPER_STARTED_ENV, &grandchild_marker)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .unwrap();
                wait_for_helper_start(&grandchild_marker);
                std::fs::remove_file(&grandchild_marker).unwrap();
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
        use windows_sys::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0, WAIT_TIMEOUT};
        use windows_sys::Win32::System::Threading::{
            OpenProcess, TerminateProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE,
            PROCESS_TERMINATE,
        };

        pub(super) struct RetainedProcess(super::super::ScopedWindowsHandle);

        impl RetainedProcess {
            pub(super) fn open(pid: u32) -> Self {
                // SAFETY: the live fixture publishes this PID before its owner can exit;
                // the resulting non-inheritable handle pins the exact process object.
                let handle =
                    unsafe { OpenProcess(PROCESS_SYNCHRONIZE | PROCESS_TERMINATE, 0, pid) };
                assert!(
                    !handle.is_null(),
                    "open fixture process: {}",
                    std::io::Error::last_os_error()
                );
                Self(super::super::ScopedWindowsHandle(handle))
            }

            pub(super) fn exited_within(&self, milliseconds: u32) -> bool {
                // SAFETY: the retained handle has SYNCHRONIZE access and remains live.
                let result = unsafe { WaitForSingleObject(self.0 .0, milliseconds) };
                match result {
                    WAIT_OBJECT_0 => true,
                    WAIT_TIMEOUT => false,
                    _ => panic!(
                        "wait for fixture process: {}",
                        std::io::Error::last_os_error()
                    ),
                }
            }

            pub(super) fn terminate_and_wait(&self) {
                // SAFETY: this handle retains PROCESS_TERMINATE access to our fixture.
                assert_ne!(unsafe { TerminateProcess(self.0 .0, 1) }, 0);
                assert!(self.exited_within(5_000), "fixture cleanup did not finish");
            }
        }

        impl Drop for RetainedProcess {
            fn drop(&mut self) {
                // SAFETY: cleanup uses only this retained process handle, never a
                // freshly reopened PID. Termination and waiting are bounded on failure.
                unsafe {
                    if WaitForSingleObject(self.0 .0, 0) == WAIT_TIMEOUT {
                        TerminateProcess(self.0 .0, 1);
                        WaitForSingleObject(self.0 .0, 5_000);
                    }
                }
            }
        }

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

    struct ChildCleanupGuard(Option<Child>);

    impl ChildCleanupGuard {
        fn child_mut(&mut self) -> &mut Child {
            self.0.as_mut().unwrap()
        }

        fn wait(mut self) {
            self.0.as_mut().unwrap().wait().unwrap();
            self.0 = None;
        }
    }

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

    fn read_helper_pid(path: &Path, timeout: Duration) -> u32 {
        let started = Instant::now();
        while started.elapsed() < timeout {
            if let Ok(contents) = std::fs::read_to_string(path) {
                if let Ok(pid) = contents.trim().parse() {
                    return pid;
                }
            }
            thread::sleep(Duration::from_millis(25));
        }
        panic!("helper did not record its descendant process ID within {timeout:?}");
    }

    fn pending_handoff_command(
        mode: &str,
        pid_file: &Path,
        cancellation: CancellationToken,
    ) -> ManagedCommand {
        ManagedCommand {
            program: std::env::current_exe().unwrap(),
            args: vec![
                "--exact".into(),
                "platform::process::tests::managed_child_test_helper".into(),
                "--ignored".into(),
                "--nocapture".into(),
            ],
            cwd: std::env::current_dir().unwrap(),
            env: vec![
                (OsString::from(HELPER_ENV), OsString::from(mode)),
                (
                    OsString::from(HELPER_PID_FILE_ENV),
                    pid_file.as_os_str().to_os_string(),
                ),
            ],
            env_remove: Vec::new(),
            capture_limits: Some((64 * 1024, 64 * 1024)),
            timeout: None,
            cancellation,
        }
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
                "platform::process::tests::managed_child_test_helper".into(),
                "--ignored".into(),
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
                    "platform::process::tests::managed_child_test_helper".into(),
                    "--ignored".into(),
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

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_runtime_descendant_probe_distinguishes_exited_unreaped_process() {
        let start = HelperStart::new();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "platform::process::tests::managed_child_test_helper",
                "--ignored",
                "--nocapture",
            ])
            .env(HELPER_ENV, "echo_stdin_len")
            .env(HELPER_STARTED_ENV, &start.marker)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // The existing helper cannot finish before we close its stdin. Retain
        // the Child until the exact exited process has been observed and reaped.
        let mut child = ChildCleanupGuard(Some(command.spawn().unwrap()));
        let pid = child.child_mut().id();
        let probe = super::RuntimeProcessTreeTestProbe::capture(pid)
            .expect("capture the live fixture process before releasing stdin");
        assert!(probe.is_alive().expect("observe the held live process"));
        start.assert_started();

        drop(child.child_mut().stdin.take());
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut information: libc::siginfo_t = unsafe { std::mem::zeroed() };
        loop {
            // WNOWAIT proves exit without releasing this child's PID identity.
            assert_eq!(
                unsafe {
                    libc::waitid(
                        libc::P_PID,
                        pid as libc::id_t,
                        &mut information,
                        libc::WEXITED | libc::WNOWAIT | libc::WNOHANG,
                    )
                },
                0,
                "the fixture must retain its exact child"
            );
            if unsafe { information.si_pid() } == pid as libc::pid_t {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "controlled fixture did not exit after stdin EOF"
            );
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(information.si_code, libc::CLD_EXITED);
        assert_eq!(unsafe { information.si_status() }, 0);
        let exited_reported_alive = probe
            .is_alive()
            .expect("observe the exact exited child before reap");
        assert_eq!(
            probe.is_alive().expect("repeat the terminal observation"),
            exited_reported_alive,
            "consuming NOTE_EXIT must not make the process live again"
        );
        let numeric_pid_exists = unsafe { libc::kill(pid as libc::pid_t, 0) } == 0;
        assert!(child
            .child_mut()
            .try_wait()
            .expect("reap the child whose exit was proven")
            .expect("WNOWAIT already proved exit")
            .success());
        child.wait();
        assert!(!probe
            .is_alive()
            .expect("observe the same process after reap"));
        assert!(!probe
            .is_alive()
            .expect("retain terminal state after the event and process are gone"));
        eprintln!(
            "[MACOS-PROBE] live=true pid={pid} waitid=CLD_EXITED status=0 \
             unreaped_alive={exited_reported_alive} numeric_pid_exists={numeric_pid_exists} \
             reaped=true after_reap_alive=false"
        );
        assert!(
            !exited_reported_alive,
            "an exited but unreaped macOS process is not a live process"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_runtime_descendant_probe_refuses_unknown_observations() {
        use std::os::fd::AsRawFd;

        assert!(super::RuntimeProcessTreeTestProbe::capture(0).is_err());
        assert!(super::RuntimeProcessTreeTestProbe::capture(u32::MAX).is_err());
        let pid = std::process::id();
        let probe = super::MacosRuntimeProcessTestProbe::capture(pid).unwrap();
        // EV_RECEIPT/data=0 acknowledges a live registration, not an exit.
        assert!(probe.is_alive().unwrap());
        let queue_flags = unsafe { libc::fcntl(probe.queue.as_raw_fd(), libc::F_GETFD) };
        assert_ne!(queue_flags, -1);
        assert_ne!(queue_flags & libc::FD_CLOEXEC, 0);
        let bad_queue = super::MacosRuntimeProcessTestProbe {
            queue: std::fs::File::open("/dev/null").unwrap(),
            process_id: pid,
            exited: std::cell::Cell::new(false),
        };
        assert!(bad_queue.is_alive().is_err());
        assert!(!bad_queue.exited.get(), "a syscall error cannot prove exit");

        let terminal = libc::kevent {
            ident: pid as libc::uintptr_t,
            filter: libc::EVFILT_PROC,
            flags: libc::EV_EOF | libc::EV_ONESHOT,
            fflags: libc::NOTE_EXIT,
            data: 0,
            udata: std::ptr::null_mut(),
        };
        super::validate_macos_runtime_exit_event(pid, &terminal).unwrap();
        for wrong in [
            libc::kevent {
                ident: terminal.ident + 1,
                ..terminal
            },
            libc::kevent {
                filter: libc::EVFILT_READ,
                ..terminal
            },
            libc::kevent {
                flags: libc::EV_ERROR,
                ..terminal
            },
            libc::kevent {
                fflags: 0,
                ..terminal
            },
            libc::kevent {
                fflags: libc::NOTE_EXIT | libc::NOTE_FORK,
                ..terminal
            },
        ] {
            assert!(super::validate_macos_runtime_exit_event(pid, &wrong).is_err());
        }
        assert!(
            probe.is_alive().unwrap(),
            "invalid events did not terminate this process"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_runtime_fixture_keeps_private_writer_and_refuses_path_reuse() {
        use std::os::fd::AsRawFd;
        use std::os::unix::fs::FileTypeExt;

        let root = tempfile::tempdir().unwrap();
        let scenario = super::runtime_process_tree_test_scenario_for_test(root.path());
        let fifo = root.path().join("runtime-tree-descendant.lifetime");
        assert!(std::fs::symlink_metadata(&fifo)
            .unwrap()
            .file_type()
            .is_fifo());
        let writer_flags =
            unsafe { libc::fcntl(scenario._descendant_lifetime.as_raw_fd(), libc::F_GETFD) };
        assert_ne!(writer_flags, -1);
        assert_ne!(
            writer_flags & libc::FD_CLOEXEC,
            0,
            "the parent writer must not escape into the runtime process"
        );
        drop(scenario);

        let occupied = tempfile::tempdir().unwrap();
        let path = occupied.path().join("runtime-tree-descendant.lifetime");
        std::fs::write(&path, b"unrelated existing bytes").unwrap();
        assert!(std::panic::catch_unwind(|| {
            super::runtime_process_tree_test_scenario_for_test(occupied.path());
        })
        .is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"unrelated existing bytes");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_runtime_descendant_probe_refuses_failed_pidfd_capture() {
        assert!(super::RuntimeProcessTreeTestProbe::capture(0).is_err());
        assert!(super::RuntimeProcessTreeTestProbe::capture(u32::MAX).is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_runtime_descendant_probe_does_not_report_poll_errors_as_death() {
        use std::os::unix::fs::OpenOptionsExt;

        // An owned O_PATH descriptor is valid but cannot be polled. This
        // exercises POLLNVAL without closing or manufacturing a descriptor.
        let process = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_PATH | libc::O_CLOEXEC)
            .open("/proc/self")
            .unwrap();
        let probe = super::RuntimeProcessTreeTestProbe { process };
        assert!(probe.is_alive().is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_runtime_descendant_probe_distinguishes_zombie_from_live_process() {
        const HARNESS_ENV: &str = "UNICA_TEST_RUNTIME_ZOMBIE_PROBE_HARNESS";
        if std::env::var_os(HARNESS_ENV).is_none() {
            let status = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "platform::process::tests::linux_runtime_descendant_probe_distinguishes_zombie_from_live_process",
                    "--nocapture",
                ])
                .env(HARNESS_ENV, "1")
                .status()
                .expect("run the isolated Linux subreaper");
            assert!(status.success(), "isolated Linux process probe failed");
            return;
        }

        // Adoption is process-wide, so this test runs alone in another process.
        // Only the runtime reaps its shell leader; we reap the adopted sleep by
        // its exact PID after observing it with WNOWAIT.
        assert_eq!(unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1) }, 0);
        struct Fixture {
            child: std::process::Child,
            tree: super::RuntimeProcessTreeHandle,
            descendant: Option<libc::pid_t>,
        }
        impl Drop for Fixture {
            fn drop(&mut self) {
                let _ = self
                    .tree
                    .terminate_and_reap_bounded(&mut self.child, Duration::from_secs(2));
                if let Some(pid) = self.descendant.take() {
                    // The owned group was killed above. Never signal a numeric
                    // PID here, nor reap any child other than this descendant.
                    let deadline = Instant::now() + Duration::from_secs(2);
                    while unsafe { libc::waitpid(pid, std::ptr::null_mut(), libc::WNOHANG) } == 0
                        && Instant::now() < deadline
                    {
                        thread::sleep(Duration::from_millis(1));
                    }
                }
            }
        }
        let root = tempfile::tempdir().expect("create runtime fixture root");
        let scenario = super::runtime_process_tree_test_scenario_for_test(root.path());
        let mut command = Command::new(scenario.program());
        command
            .args(scenario.long_lived_args())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut tree = super::RuntimeProcessTreeHandle::prepare(&mut command).unwrap();
        let mut child = command.spawn().expect("spawn the runtime-owned tree");
        tree.attach(&mut child)
            .expect("attach the runtime-owned tree");
        let mut fixture = Fixture {
            child,
            tree,
            descendant: None,
        };
        let probe = scenario
            .wait_for_descendant(Duration::from_secs(5))
            .expect("capture the live runtime descendant");
        let pid: libc::pid_t = std::fs::read_to_string(&scenario.descendant_pid_path)
            .unwrap()
            .parse()
            .unwrap();
        fixture.descendant = Some(pid);
        assert!(probe.is_alive().expect("observe the live descendant"));

        fixture.tree.terminate(&mut fixture.child).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while matches!(
            fixture.tree.poll(&mut fixture.child).unwrap(),
            super::RuntimeProcessTreeState::Running
        ) {
            assert!(
                Instant::now() < deadline,
                "runtime did not prove tree death"
            );
            thread::sleep(Duration::from_millis(1));
        }
        let mut information: libc::siginfo_t = unsafe { std::mem::zeroed() };
        loop {
            assert_eq!(
                unsafe {
                    libc::waitid(
                        libc::P_PID,
                        pid as libc::id_t,
                        &mut information,
                        libc::WEXITED | libc::WNOWAIT | libc::WNOHANG,
                    )
                },
                0,
                "the subreaper must own the exited descendant"
            );
            if unsafe { information.si_pid() } == pid {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "descendant did not become waitable"
            );
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(unsafe { information.si_pid() }, pid);
        assert_eq!(information.si_code, libc::CLD_KILLED);
        assert_eq!(unsafe { libc::kill(pid, 0) }, 0, "zombie PID still exists");
        let zombie_reported_alive = probe.is_alive().expect("observe the retained zombie");
        assert_eq!(unsafe { libc::waitpid(pid, std::ptr::null_mut(), 0) }, pid);
        fixture.descendant = None;
        assert!(!probe
            .is_alive()
            .expect("observe the same process after reap"));
        assert!(
            !zombie_reported_alive,
            "a completed runtime descendant is a zombie, not a live process: alive={zombie_reported_alive}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_runtime_descendant_probe_retains_the_observed_process_after_child_drop() {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "platform::process::tests::managed_child_test_helper",
                "--ignored",
                "--nocapture",
            ])
            .env(HELPER_ENV, "echo_stdin_len")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut child = ChildCleanupGuard(Some(command.spawn().unwrap()));
        let completion = process_test_support::RetainedProcess::open(child.child_mut().id());
        let probe = super::RuntimeProcessTreeTestProbe::capture(child.child_mut().id())
            .expect("capture the live fixture process");
        assert!(probe.is_alive().expect("probe before stdin EOF"));

        drop(child.child_mut().stdin.take());
        assert!(
            completion.exited_within(5_000),
            "fixture did not finish after stdin EOF"
        );
        assert!(child
            .child_mut()
            .try_wait()
            .expect("reap the signaled fixture child")
            .expect("the fixture completion handle was signaled")
            .success());
        child.wait();
        drop(completion);

        assert!(!probe
            .is_alive()
            .expect("probe after Child and its handle were dropped"));
        assert!(!probe
            .is_alive()
            .expect("repeat the same retained process probe"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_runtime_descendant_probe_refuses_failed_handle_capture() {
        assert!(super::RuntimeProcessTreeTestProbe::capture(0).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn windows_runtime_descendant_probe_does_not_report_wait_errors_as_death() {
        use windows_sys::Win32::System::Threading::{
            OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };

        // SAFETY: a real non-inheritable query-only handle belongs to this test.
        // It deliberately lacks SYNCHRONIZE; it is neither invalid nor closed.
        let process =
            unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, std::process::id()) };
        assert!(!process.is_null(), "{}", std::io::Error::last_os_error());
        let probe = super::RuntimeProcessTreeTestProbe {
            process: super::ScopedWindowsHandle(process),
        };
        assert!(
            probe.is_alive().is_err(),
            "a wait failure proves no process death"
        );
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
    #[ignore = "helper process: spawned by runtime_sentinel_preserves_a_meaningful_inherited_fd198 with RUNTIME_SENTINEL_FD198_MARKER_ENV; not a standalone test"]
    #[allow(clippy::zombie_processes)]
    // The parent test owns and reaps the detached descendant after observing
    // sentinel-only lifetime; waiting here would erase the tested boundary.
    fn runtime_preexisting_fd198_helper() {
        use std::os::fd::{FromRawFd, IntoRawFd};
        use std::os::unix::fs::MetadataExt;
        use std::os::unix::process::CommandExt;

        let marker = std::env::var_os(RUNTIME_SENTINEL_FD198_MARKER_ENV)
            .expect("helper process: the fd198 harness sets the marker");
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
                    "platform::process::tests::runtime_sentinel_preserves_a_meaningful_inherited_fd198",
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
                "platform::process::tests::runtime_preexisting_fd198_helper",
                "--ignored",
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
    #[ignore = "helper process: spawned by the runtime sentinel tests with RUNTIME_SENTINEL_HELPER_ENV; not a standalone test"]
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
                        "platform::process::tests::runtime_runner_style_sentinel_helper",
                        "--ignored",
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
            other => panic!("helper process: unexpected sentinel mode {other:?}"),
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
                "platform::process::tests::runtime_runner_style_sentinel_helper",
                "--ignored",
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
    fn windows_runtime_tree_proves_a_child_that_finished_before_first_poll() {
        use std::os::windows::io::AsRawHandle;

        let pid_file = std::env::temp_dir().join(format!(
            "unica-runtime-finished-child-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _file_cleanup = FileCleanupGuard(pid_file.clone());
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "platform::process::tests::managed_child_test_helper",
                "--ignored",
                "--nocapture",
            ])
            .env(HELPER_ENV, "windows_finished_child_parent")
            .env(HELPER_PID_FILE_ENV, &pid_file)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let runtime = super::RuntimeProcessTreeHandle::prepare(&mut command).unwrap();
        let mut fixture = WindowsRuntimeTreeFixture {
            runtime,
            child: Some(command.spawn().unwrap()),
            descendant: None,
        };
        let child = fixture.child.as_mut().unwrap();
        fixture.runtime.attach(child).unwrap();
        assert!(
            windows_fixture_process_signaled_within(child.as_raw_handle() as _, 10_000).unwrap()
        );
        // No runtime poll has run before the parent reaped and closed its own
        // child's handle. Lifetime coverage must recover this ordinary case.
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let super::RuntimeProcessTreeState::Exited(status) =
                fixture.runtime.poll(child).unwrap()
            {
                assert!(status.success());
                break;
            }
            assert!(
                Instant::now() < deadline,
                "finished child left an unprovable coverage gap"
            );
            thread::yield_now();
        }
        assert!(
            std::fs::read_to_string(pid_file)
                .unwrap()
                .parse::<u32>()
                .unwrap()
                > 0
        );
        fixture
            .cleanup()
            .expect("release the completed runtime fixture");
    }

    #[cfg(windows)]
    #[test]
    fn windows_runtime_terminal_waits_for_the_descendant_process_signal() {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::JobObjects::IsProcessInJob;
        use windows_sys::Win32::System::Threading::{
            OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
        };

        for _ in 0..16 {
            let temp = tempfile::tempdir().unwrap();
            let pid_file = temp.path().join("descendant.pid");
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "platform::process::tests::managed_child_test_helper",
                    "--ignored",
                    "--nocapture",
                ])
                .env(HELPER_ENV, "exit_leaving_detached_grandchild")
                .env(HELPER_PID_FILE_ENV, &pid_file)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            let runtime = super::RuntimeProcessTreeHandle::prepare(&mut command).unwrap();
            let mut fixture = WindowsRuntimeTreeFixture {
                runtime,
                child: Some(command.spawn().unwrap()),
                descendant: None,
            };
            let child = fixture.child.as_mut().unwrap();
            fixture.runtime.attach(child).unwrap();
            assert!(
                windows_fixture_process_signaled_within(child.as_raw_handle() as _, 5000).unwrap()
            );
            let pid = std::fs::read_to_string(pid_file)
                .unwrap()
                .trim()
                .parse::<u32>()
                .unwrap();
            // SAFETY: this read-only capture is attested against our exact Job
            // before it is retained. Process control stays on the private Job.
            let process = unsafe {
                OpenProcess(
                    PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                    0,
                    pid,
                )
            };
            assert!(
                !process.is_null(),
                "capture live descendant: {}",
                std::io::Error::last_os_error()
            );
            fixture.descendant = Some(super::ScopedWindowsHandle(process));
            let mut in_job = 0;
            // SAFETY: both owned handles remain live for this membership query.
            assert_ne!(
                unsafe { IsProcessInJob(process, fixture.runtime.tree.job, &mut in_job) },
                0
            );
            assert_ne!(in_job, 0, "fixture descendant is outside its private Job");
            assert!(!windows_fixture_process_signaled_within(process, 0).unwrap());
            assert!(matches!(
                fixture.runtime.poll(child).unwrap(),
                super::RuntimeProcessTreeState::Running
            ));
            assert!(
                fixture.runtime.leader_exited(),
                "root exit must be known before termination"
            );

            fixture.runtime.terminate(child).unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                if let super::RuntimeProcessTreeState::Exited(_) =
                    fixture.runtime.poll(child).unwrap()
                {
                    assert!(
                        windows_fixture_process_signaled_within(process, 0).unwrap(),
                        "runtime terminal preceded descendant process signal"
                    );
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "terminated runtime tree stayed unproven"
                );
                thread::yield_now();
            }
            fixture
                .cleanup()
                .expect("release runtime regression fixture");
        }
    }

    #[cfg(windows)]
    fn windows_fixture_process_signaled_within(
        process: windows_sys::Win32::Foundation::HANDLE,
        millis: u32,
    ) -> std::io::Result<bool> {
        use windows_sys::Win32::Foundation::{WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT};
        use windows_sys::Win32::System::Threading::WaitForSingleObject;
        // SAFETY: callers retain the exact original Child handle across probes.
        match unsafe { WaitForSingleObject(process, millis) } {
            WAIT_OBJECT_0 => Ok(true),
            WAIT_TIMEOUT => Ok(false),
            WAIT_FAILED => Err(std::io::Error::last_os_error()),
            state => Err(std::io::Error::other(format!(
                "unexpected process wait state {state}"
            ))),
        }
    }

    #[cfg(windows)]
    struct WindowsRuntimeTreeFixture {
        runtime: super::RuntimeProcessTreeHandle,
        child: Option<Child>,
        descendant: Option<super::ScopedWindowsHandle>,
    }

    #[cfg(windows)]
    impl WindowsRuntimeTreeFixture {
        fn cleanup(&mut self) -> std::io::Result<()> {
            let Some(mut child) = self.child.take() else {
                return Ok(());
            };
            let deadline = Instant::now() + Duration::from_secs(5);
            let result = self.runtime.terminate_and_reap_until(&mut child, deadline);
            if let Some(descendant) = &self.descendant {
                // Failure cleanup also waits on the independent fixture witness;
                // it cannot mask the assertion made before cleanup above.
                while !windows_fixture_process_signaled_within(descendant.0, 0)? {
                    if Instant::now() >= deadline {
                        return Err(result.err().unwrap_or_else(|| {
                            std::io::Error::other(
                                "Windows fixture descendant cleanup exceeded its deadline",
                            )
                        }));
                    }
                    thread::yield_now();
                }
            }
            result
        }
    }

    #[cfg(windows)]
    impl Drop for WindowsRuntimeTreeFixture {
        fn drop(&mut self) {
            if let Err(error) = self.cleanup() {
                eprintln!("Windows runtime fixture cleanup failed: {error}");
            }
        }
    }

    #[cfg(windows)]
    #[test]
    fn runtime_process_tree_handle_waits_for_windows_job_object_descendants() {
        assert_windows_runtime_process_tree_semantics_for_test()
            .expect("retain and terminate Windows runtime Job Object");
    }

    #[test]
    fn detached_null_stdio_grandchild_never_holds_the_parent_stdout_pipe() {
        // Регрессия дыма упакованного MCP на Windows: отсоединённый демон со
        // `Stdio::null()` уносил копии труб хоста, и хост не видел EOF на
        // stdout после выхода сервера, пока демон не выйдет сам.
        let pid_file = std::env::temp_dir().join(format!(
            "unica-detached-grandchild-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _pid_file_cleanup = FileCleanupGuard(pid_file.clone());
        let mut grandchild_cleanup = ProcessCleanupGuard(Vec::new());
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "platform::process::tests::managed_child_test_helper",
                "--ignored",
                "--nocapture",
            ])
            .env(HELPER_ENV, "exit_leaving_detached_grandchild")
            .env(HELPER_PID_FILE_ENV, &pid_file)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut stdout = child.stdout.take().unwrap();
        let started = Instant::now();
        let mut output = String::new();
        stdout.read_to_string(&mut output).unwrap();
        let eof_after = started.elapsed();
        let status = child.wait().unwrap();
        if let Some(pid) = std::fs::read_to_string(&pid_file)
            .ok()
            .and_then(|text| text.trim().parse::<u32>().ok())
        {
            grandchild_cleanup.0.push(pid);
        }

        assert!(status.success(), "{status}");
        assert!(output.contains("grandchild-detached"), "{output}");
        assert!(
            eof_after < Duration::from_secs(5),
            "stdout EOF waited for the detached grandchild: {eof_after:?}"
        );
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
                "platform::process::tests::managed_child_test_helper".into(),
                "--ignored".into(),
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

    fn record_echo_helper() -> ManagedChild {
        ManagedChild::spawn(ManagedCommand {
            program: std::env::current_exe().unwrap(),
            args: vec![
                "--exact".into(),
                "platform::process::tests::managed_child_test_helper".into(),
                "--ignored".into(),
                "--nocapture".into(),
            ],
            cwd: std::env::current_dir().unwrap(),
            env: vec![(OsString::from(HELPER_ENV), OsString::from("echo_stdin"))],
            env_remove: Vec::new(),
            capture_limits: None,
            timeout: Some(Duration::from_secs(30)),
            cancellation: CancellationToken::new(),
        })
        .unwrap()
    }

    #[test]
    fn record_stream_delivers_every_record_past_the_capture_limit_while_writing_stdin() {
        // The echo starts answering before stdin ends. Both sides exceed a
        // pipe buffer and the whole output exceeds the default capture, so a
        // capture or a sequential write would truncate or deadlock.
        let long = vec![b'x'; 2 * super::STDOUT_CAPTURE_LIMIT];
        // The leading NUL ends the harness banner as its own record.
        let mut input = vec![0];
        input.extend_from_slice(&long);
        input.push(0);
        let short_records = 100_000_usize;
        for number in 0..short_records {
            input.extend_from_slice(format!("record-{number}").as_bytes());
            input.push(0);
        }
        assert!(input.len() > 3 * super::STDOUT_CAPTURE_LIMIT);
        let mut records = Vec::new();
        let output = record_echo_helper()
            .wait_for_record_output(input, 0, &mut |record| {
                records.push(record.to_vec());
                StreamControl::Continue
            })
            .unwrap();

        assert!(output.status_success, "{output:?}");
        assert_eq!(output.line_error, None);
        assert_eq!(records.len(), short_records + 2);
        assert_eq!(records[1], long);
        assert_eq!(records[2], b"record-0");
        assert_eq!(
            records.last().unwrap(),
            format!("record-{}", short_records - 1).as_bytes()
        );
    }

    #[test]
    fn byte_stream_delivers_the_exact_bytes_past_the_capture_limit() {
        let mut input = vec![0];
        for number in 0..(3 * super::STDOUT_CAPTURE_LIMIT) {
            input.push((number % 251) as u8);
        }
        let mut received = Vec::new();
        let output = record_echo_helper()
            .wait_for_byte_output(input.clone(), &mut |chunk| {
                received.extend_from_slice(chunk);
                StreamControl::Continue
            })
            .unwrap();

        assert!(output.status_success, "{output:?}");
        assert_eq!(output.line_error, None);
        // The harness banner ends at the first NUL of the echoed input.
        let start = received.len() - input.len();
        assert_eq!(received[start..], input[..]);
    }

    #[test]
    fn record_stream_reports_an_unterminated_tail_instead_of_a_record() {
        let mut records = Vec::new();
        let output = record_echo_helper()
            .wait_for_record_output(b"\0first\0second\0tail".to_vec(), 0, &mut |record| {
                records.push(record.to_vec());
                StreamControl::Continue
            })
            .unwrap();

        assert_eq!(records[1..], [b"first".to_vec(), b"second".to_vec()]);
        assert_eq!(
            output.line_error,
            Some((4, super::STREAM_RECORD_UNTERMINATED_ERROR.to_string()))
        );
    }

    #[test]
    fn record_stream_consumer_can_stop_before_the_input_is_consumed() {
        let mut input = Vec::new();
        for _ in 0..200_000 {
            input.extend_from_slice(b"record\0");
        }
        let mut seen = 0;
        let output = record_echo_helper()
            .wait_for_record_output(input, 0, &mut |_| {
                seen += 1;
                if seen == 3 {
                    StreamControl::Stop
                } else {
                    StreamControl::Continue
                }
            })
            .unwrap();

        assert_eq!(seen, 3);
        assert!(output.stopped_by_consumer, "{output:?}");
        assert!(!output.status_success);
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
    fn managed_child_owned_leader_exit_does_not_accept_pending_termination() {
        let directory = tempfile::tempdir().unwrap();
        let cancellation = CancellationToken::new();
        let child = ManagedChild::spawn(pending_handoff_command(
            "sleep",
            &directory.path().join("unused-pid"),
            cancellation.clone(),
        ))
        .unwrap();
        let mut cleanup = ManagedChildCleanupGuard::new(child, cancellation);
        let child = cleanup.managed_mut();
        // Reproduce a termination that has not yet earned an exit observation.
        child.state = ChildState::Terminating;
        assert!(!child.is_running().unwrap());
        assert!(!child.poll_owned_leader_exit().unwrap());
        child.state = ChildState::Running;
        child.terminate().unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !child.poll_owned_leader_exit().unwrap() {
            assert!(Instant::now() < deadline, "the owned leader did not exit");
            thread::yield_now();
        }
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
                "platform::process::tests::managed_child_test_helper".into(),
                "--ignored".into(),
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
        let start = HelperStart::new();
        let mut managed = ManagedChild::spawn(ManagedCommand {
            program: std::env::current_exe().unwrap(),
            args: vec![
                "--exact".into(),
                "platform::process::tests::managed_child_test_helper".into(),
                "--ignored".into(),
                "--nocapture".into(),
            ],
            cwd: std::env::current_dir().unwrap(),
            env: vec![
                (OsString::from(HELPER_ENV), OsString::from("sleep")),
                start.env(),
            ],
            env_remove: Vec::new(),
            capture_limits: None,
            timeout: None,
            cancellation: CancellationToken::new(),
        })
        .unwrap();
        start.assert_started();

        managed.terminate().unwrap();
        assert_eq!(managed.state, ChildState::Reaped);
        let second = Instant::now();
        managed.terminate().unwrap();
        assert!(second.elapsed() < Duration::from_millis(100));
        assert_eq!(managed.state, ChildState::Reaped);
    }

    #[test]
    fn protected_process_finishes_after_cancellation_on_every_host() {
        let start = HelperStart::new();
        let cancellation = CancellationToken::new();
        let signal = cancellation.clone();
        let canceller = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            while !signal.protected_process_started() {
                assert!(Instant::now() < deadline, "protected process did not start");
                thread::sleep(Duration::from_millis(1));
            }
            signal.cancel();
        });
        let output = ManagedChild::run(ManagedCommand {
            program: std::env::current_exe().unwrap(),
            args: vec![
                "--exact".into(),
                "platform::process::tests::managed_child_test_helper".into(),
                "--ignored".into(),
                "--nocapture".into(),
            ],
            cwd: std::env::current_dir().unwrap(),
            env: vec![
                (OsString::from(HELPER_ENV), OsString::from("short_sleep")),
                start.env(),
            ],
            env_remove: Vec::new(),
            capture_limits: None,
            timeout: None,
            cancellation: cancellation.protect_process_on_spawn(),
        })
        .unwrap();
        canceller.join().unwrap();
        start.assert_started();
        assert!(cancellation.protected_process_started());
        assert!(output.status_success, "{output:?}");
        assert!(!output.cancelled);
    }

    #[test]
    fn line_consumer_can_stop_and_reap_the_process_before_timeout() {
        let start = HelperStart::new();
        let mut managed = ManagedChild::spawn(ManagedCommand {
            program: std::env::current_exe().unwrap(),
            args: vec![
                "--exact".into(),
                "platform::process::tests::managed_child_test_helper".into(),
                "--ignored".into(),
                "--nocapture".into(),
            ],
            cwd: std::env::current_dir().unwrap(),
            env: vec![
                (OsString::from(HELPER_ENV), OsString::from("stream_forever")),
                start.env(),
            ],
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

        start.assert_started();
        assert_eq!(lines, 3);
        assert!(output.stopped_by_consumer);
        assert!(!output.timed_out);
        assert!(!output.cancelled);
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(managed.state, ChildState::Reaped);
    }

    fn controlled_helper(mode: &str, cancellation: CancellationToken) -> ManagedChild {
        ManagedChild::spawn(ManagedCommand {
            program: std::env::current_exe().unwrap(),
            args: vec![
                "--exact".into(),
                "platform::process::tests::managed_child_test_helper".into(),
                "--ignored".into(),
                "--nocapture".into(),
            ],
            cwd: std::env::current_dir().unwrap(),
            env: vec![(OsString::from(HELPER_ENV), OsString::from(mode))],
            env_remove: Vec::new(),
            capture_limits: None,
            timeout: Some(Duration::from_secs(5)),
            cancellation,
        })
        .unwrap()
    }

    struct ControlledStderrFixture {
        writer: Option<ChildCleanupGuard>,
        writer_control: std::net::TcpStream,
        leader_control: std::net::TcpStream,
        #[cfg(unix)]
        leader_id: u32,
        #[cfg(windows)]
        leader: super::RuntimeProcessTreeTestProbe,
    }

    impl ControlledStderrFixture {
        fn accept(listener: &std::net::TcpListener, expected: u8) -> std::net::TcpStream {
            listener.set_nonblocking(true).unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                match listener.accept() {
                    Ok((mut control, _)) => {
                        control.set_nonblocking(false).unwrap();
                        control
                            .set_read_timeout(Some(Duration::from_secs(5)))
                            .unwrap();
                        control
                            .set_write_timeout(Some(Duration::from_secs(5)))
                            .unwrap();
                        let mut ready = [0_u8];
                        control.read_exact(&mut ready).unwrap();
                        assert_eq!(ready, [expected]);
                        return control;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "stream fixture did not connect");
                        thread::yield_now();
                    }
                    Err(error) => panic!("accept stream fixture: {error}"),
                }
            }
        }

        fn spawn() -> (ManagedChild, Self) {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap().to_string();
            let mut writer = ChildCleanupGuard(Some(
                Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "platform::process::tests::managed_child_test_helper",
                        "--ignored",
                        "--nocapture",
                    ])
                    .env(HELPER_ENV, "controlled_stderr_writer")
                    .env(CONTROLLED_STDERR_SOCKET_ENV, &address)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::piped())
                    .spawn()
                    .unwrap(),
            ));
            let writer_control = Self::accept(&listener, b'w');
            let mut managed = ManagedChild::spawn(ManagedCommand {
                program: std::env::current_exe().unwrap(),
                args: vec![
                    "--exact".into(),
                    "platform::process::tests::managed_child_test_helper".into(),
                    "--ignored".into(),
                    "--nocapture".into(),
                ],
                cwd: std::env::current_dir().unwrap(),
                env: vec![
                    (HELPER_ENV.into(), "controlled_stderr_leader".into()),
                    (CONTROLLED_STDERR_SOCKET_ENV.into(), address.into()),
                ],
                env_remove: Vec::new(),
                capture_limits: None,
                timeout: Some(Duration::from_secs(2)),
                cancellation: CancellationToken::new(),
            })
            .unwrap();
            let leader_control = Self::accept(&listener, b'l');
            // Only the test connects this actual pipe to the controlled owner.
            // Its writer remains our direct Child, independently reapable even
            // after the managed leader has exited. This tests stream completion,
            // without asserting a process-tree completion guarantee.
            managed.child.stderr = writer.child_mut().stderr.take();
            let fixture = Self {
                writer: Some(writer),
                writer_control,
                leader_control,
                #[cfg(unix)]
                leader_id: managed.id(),
                #[cfg(windows)]
                leader: super::RuntimeProcessTreeTestProbe::capture(managed.id()).unwrap(),
            };
            (managed, fixture)
        }

        fn release_leader_and_observe_exit(&mut self) {
            self.leader_control.write_all(b"x").unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                #[cfg(unix)]
                let exited = super::unix_leader_exit_unreaped(self.leader_id)
                    .expect("observe our unreaped leader")
                    .is_some();
                #[cfg(windows)]
                let exited = !self.leader.is_alive().expect("observe retained leader");
                if exited {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "controlled fixture leader did not exit"
                );
                thread::yield_now();
            }
        }

        fn release_stderr_and_reap(&mut self) {
            self.writer_control.write_all(b"x").unwrap();
            let mut writer = self.writer.take().unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                if let Some(status) = writer.child_mut().try_wait().unwrap() {
                    writer.wait();
                    assert!(status.success(), "stderr fixture failed");
                    return;
                }
                assert!(Instant::now() < deadline, "stderr fixture did not exit");
                thread::yield_now();
            }
        }
    }

    #[test]
    fn controlled_stream_held_stderr_keeps_timeout_after_leader_exit() {
        let (mut managed, mut fixture) = ControlledStderrFixture::spawn();
        fixture.release_leader_and_observe_exit();
        let mut terminals = 0;
        let result = managed.wait_for_controlled_line_output(1024, &mut |_, bytes| {
            if bytes == br#"{"event":"terminal"}"# {
                terminals += 1;
            }
            ProcessLineReply::Continue
        });
        let writer_was_still_alive = fixture
            .writer
            .as_mut()
            .unwrap()
            .child_mut()
            .try_wait()
            .unwrap()
            .is_none();
        fixture.release_stderr_and_reap();
        let output = result.unwrap();
        assert!(
            writer_was_still_alive,
            "stderr must remain held until test release"
        );
        assert_eq!(terminals, 1);
        assert!(
            output.timed_out,
            "controlled success escaped an unfinished stderr capture"
        );
        assert!(!output.status_success && !output.cancelled);
        assert_eq!(managed.state, ChildState::Reaped);
    }

    #[test]
    fn controlled_stream_late_stderr_eof_preserves_tail_after_leader_exit() {
        let (mut managed, mut fixture) = ControlledStderrFixture::spawn();
        let mut terminals = 0;
        let output = managed
            .wait_for_controlled_line_output(1024, &mut |_, bytes| {
                if bytes == br#"{"event":"terminal"}"# {
                    terminals += 1;
                    fixture.release_leader_and_observe_exit();
                    fixture.release_stderr_and_reap();
                }
                ProcessLineReply::Continue
            })
            .unwrap();
        assert_eq!(terminals, 1);
        assert!(output.status_success, "{output:?}");
        assert!(!output.timed_out && !output.cancelled && output.line_error.is_none());
        assert_eq!(output.stderr, "stderr-before\nstderr-after\n");
        assert_eq!(managed.state, ChildState::Reaped);
    }

    enum ControlledReaderFault {
        Error,
        Panic,
    }

    struct ControlledFaultingRead {
        prefix: std::io::Cursor<&'static [u8]>,
        fault: ControlledReaderFault,
        finished: std::sync::mpsc::Sender<()>,
    }

    impl Read for ControlledFaultingRead {
        fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
            if self.prefix.position() < self.prefix.get_ref().len() as u64 {
                return self.prefix.read(bytes);
            }
            match self.fault {
                ControlledReaderFault::Error => Err(std::io::Error::other("fixture read fault")),
                ControlledReaderFault::Panic => panic!("fixture reader panic"),
            }
        }
    }

    impl Drop for ControlledFaultingRead {
        fn drop(&mut self) {
            let _ = self.finished.send(());
        }
    }

    fn controlled_faulting_read(
        fault: ControlledReaderFault,
    ) -> (ControlledFaultingRead, std::sync::mpsc::Receiver<()>) {
        let (finished, receiver) = std::sync::mpsc::channel();
        (
            ControlledFaultingRead {
                prefix: std::io::Cursor::new(b"frame\n"),
                fault,
                finished,
            },
            receiver,
        )
    }

    #[test]
    fn controlled_stdout_read_error_stays_an_error_after_disconnect() {
        let (reader, finished) = controlled_faulting_read(ControlledReaderFault::Error);
        let receiver = super::start_line_reader(
            Some(reader),
            super::StreamFraming::Lines { max_bytes: 1024 },
        );
        let mut error = None;
        let mut lines = 0;
        let mut on_line = |_, bytes: &[u8]| {
            assert_eq!(bytes, b"frame");
            lines += 1;
            ProcessLineReply::Continue
        };
        assert!(matches!(
            super::drain_replied_line_messages(
                &receiver,
                &mut on_line,
                &mut error,
                true,
                super::LineStreamKind::Controlled,
            ),
            super::LineDrain::Reply(ProcessLineReply::Continue)
        ));
        finished.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(
            super::drain_replied_line_messages(
                &receiver,
                &mut on_line,
                &mut error,
                true,
                super::LineStreamKind::Controlled,
            ),
            super::LineDrain::Reply(ProcessLineReply::Stop)
        ));
        assert!(error
            .as_ref()
            .is_some_and(|(_, text)| text.contains("fixture read fault")));
        assert!(
            !matches!(
                super::drain_replied_line_messages(
                    &receiver,
                    &mut on_line,
                    &mut error,
                    true,
                    super::LineStreamKind::Controlled,
                ),
                super::LineDrain::End
                    | super::LineDrain::Pending
                    | super::LineDrain::Reply(ProcessLineReply::Continue)
            ),
            "disconnect after a read error is not clean EOF"
        );
        assert_eq!(lines, 1);
    }

    #[test]
    fn controlled_stdout_reader_panic_is_not_eof() {
        let (reader, finished) = controlled_faulting_read(ControlledReaderFault::Panic);
        let receiver = super::start_line_reader(
            Some(reader),
            super::StreamFraming::Lines { max_bytes: 1024 },
        );
        let mut error = None;
        let mut lines = 0;
        let mut on_line = |_, bytes: &[u8]| {
            assert_eq!(bytes, b"frame");
            lines += 1;
            ProcessLineReply::Continue
        };
        assert!(matches!(
            super::drain_replied_line_messages(
                &receiver,
                &mut on_line,
                &mut error,
                true,
                super::LineStreamKind::Controlled,
            ),
            super::LineDrain::Reply(ProcessLineReply::Continue)
        ));
        finished.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(
            !matches!(
                super::drain_replied_line_messages(
                    &receiver,
                    &mut on_line,
                    &mut error,
                    true,
                    super::LineStreamKind::Controlled,
                ),
                super::LineDrain::End
                    | super::LineDrain::Pending
                    | super::LineDrain::Reply(ProcessLineReply::Continue)
            ),
            "a panicked reader did not observe EOF"
        );
        assert_eq!(lines, 1);
    }

    #[test]
    fn controlled_stderr_reader_faults_do_not_satisfy_eof() {
        for fault in [ControlledReaderFault::Error, ControlledReaderFault::Panic] {
            let (reader, finished) = controlled_faulting_read(fault);
            let receiver = super::start_reader(Some(reader), 1024).unwrap();
            finished.recv_timeout(Duration::from_secs(5)).unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                match super::receive_complete_capture(&receiver) {
                    Err(_) => break,
                    Ok(None) => {
                        assert!(Instant::now() < deadline, "reader did not disconnect");
                        thread::yield_now();
                    }
                    Ok(Some(_)) => panic!("reader fault was accepted as stderr EOF"),
                }
            }
        }
    }

    #[test]
    fn controlled_stream_exchanges_two_replies_over_real_pipes() {
        let mut managed = controlled_helper("controlled_two_replies", CancellationToken::new());
        let mut frames = Vec::new();
        let output = managed
            .wait_for_controlled_line_output(1024, &mut |number, bytes| {
                if !bytes.starts_with(b"{\"event\":") {
                    return ProcessLineReply::Continue;
                }
                frames.push((number, bytes.to_vec()));
                match bytes {
                    br#"{"event":"prepare"}"# => {
                        ProcessLineReply::Reply(b"{\"decision\":\"stage\"}\n".to_vec())
                    }
                    br#"{"event":"ready"}"# => {
                        ProcessLineReply::Reply(b"{\"decision\":\"commit\"}\n".to_vec())
                    }
                    _ => ProcessLineReply::Continue,
                }
            })
            .unwrap();
        assert!(output.status_success, "{output:?}");
        assert_eq!(frames.len(), 3, "{frames:?}");
        assert_eq!(frames[2].1, br#"{"event":"terminal"}"#);
        assert!(frames.windows(2).all(|pair| pair[0].0 < pair[1].0));
        assert_eq!(managed.state, ChildState::Reaped);
    }

    #[test]
    fn controlled_stream_cancel_before_publication_stops_without_a_reply() {
        let cancellation = CancellationToken::new();
        let mut managed = controlled_helper("controlled_two_replies", cancellation.clone());
        let mut prepare_seen = false;
        let output = managed
            .wait_for_controlled_line_output(1024, &mut |_, bytes| {
                if bytes == br#"{"event":"prepare"}"# {
                    prepare_seen = true;
                    cancellation.cancel();
                    return ProcessLineReply::Reply(b"{\"decision\":\"stage\"}\n".to_vec());
                }
                ProcessLineReply::Continue
            })
            .unwrap();
        assert!(prepare_seen);
        assert!(output.cancelled, "{output:?}");
        assert!(!output.status_success);
        assert!(!cancellation.protected_process_started());
        assert_eq!(managed.state, ChildState::Reaped);
    }

    #[test]
    fn controlled_stream_committed_reply_survives_cancel_before_its_ack() {
        let cancellation = CancellationToken::new();
        let mut managed = controlled_helper("controlled_two_replies", cancellation.clone());
        let mut published = false;
        let mut terminal_seen = false;
        let output = managed
            .wait_for_controlled_line_output(1024, &mut |_, bytes| match bytes {
                br#"{"event":"prepare"}"# => {
                    ProcessLineReply::Reply(b"{\"decision\":\"stage\"}\n".to_vec())
                }
                br#"{"event":"ready"}"# => {
                    cancellation
                        .handoff_with_gate(|| {
                            published = true;
                            Ok(())
                        })
                        .unwrap();
                    // Deterministically land cancellation after the effect and
                    // before ManagedChild can dispatch the commit ACK.
                    cancellation.cancel();
                    ProcessLineReply::ReplyAndProtect {
                        reply: b"{\"decision\":\"commit\"}\n".to_vec(),
                        finish_timeout: Duration::from_secs(2),
                    }
                }
                br#"{"event":"terminal"}"# => {
                    terminal_seen = true;
                    ProcessLineReply::Continue
                }
                _ => ProcessLineReply::Continue,
            })
            .unwrap();
        assert!(published && terminal_seen);
        assert!(cancellation.is_cancelled());
        assert!(cancellation.protected_process_started());
        assert!(output.status_success, "{output:?}");
        assert!(!output.cancelled);
        assert_eq!(managed.state, ChildState::Reaped);
    }

    #[test]
    fn controlled_stream_eof_invalid_and_oversized_frames_release_the_process() {
        let mut empty = controlled_helper("controlled_empty_eof", CancellationToken::new());
        let mut frames = 0;
        let output = empty
            .wait_for_controlled_line_output(1024, &mut |_, bytes| {
                if bytes.starts_with(b"{\"event\":") {
                    frames += 1;
                }
                ProcessLineReply::Continue
            })
            .unwrap();
        assert!(output.status_success, "{output:?}");
        assert_eq!(frames, 0);
        assert_eq!(empty.state, ChildState::Reaped);

        let mut invalid = controlled_helper("controlled_invalid_frame", CancellationToken::new());
        let output = invalid
            .wait_for_controlled_line_output(1024, &mut |_, bytes| {
                if std::str::from_utf8(bytes).is_err() {
                    ProcessLineReply::Stop
                } else {
                    ProcessLineReply::Continue
                }
            })
            .unwrap();
        assert!(output.stopped_by_consumer, "{output:?}");
        assert!(!output.cancelled && !output.timed_out);
        assert_eq!(invalid.state, ChildState::Reaped);

        let mut oversized =
            controlled_helper("controlled_oversized_frame", CancellationToken::new());
        let mut oversized_delivered = false;
        let output = oversized
            .wait_for_controlled_line_output(1024, &mut |_, bytes| {
                oversized_delivered |= bytes.len() > 1024;
                ProcessLineReply::Continue
            })
            .unwrap();
        assert!(!oversized_delivered);
        assert!(output.stopped_by_consumer, "{output:?}");
        assert!(output
            .line_error
            .as_ref()
            .is_some_and(|(_, message)| { message == super::STREAM_LINE_TOO_LONG_ERROR }));
        assert!(!output.timed_out);
        assert_eq!(oversized.state, ChildState::Reaped);
    }

    #[test]
    fn controlled_stream_ack_write_failure_reaps_without_erasing_publication() {
        let cancellation = CancellationToken::new();
        let mut managed = controlled_helper("controlled_closed_stdin", cancellation.clone());
        let mut published = false;
        let error = managed
            .wait_for_controlled_line_output(1024, &mut |_, bytes| {
                if bytes != br#"{"event":"ready"}"# {
                    return ProcessLineReply::Continue;
                }
                cancellation
                    .handoff_with_gate(|| {
                        published = true;
                        Ok(())
                    })
                    .unwrap();
                cancellation.cancel();
                ProcessLineReply::ReplyAndProtect {
                    reply: b"{\"decision\":\"commit\"}\n".to_vec(),
                    finish_timeout: Duration::from_secs(2),
                }
            })
            .expect_err("closed stdin must fail its ACK write");
        assert!(error.contains("stdin reply write failed"), "{error}");
        assert!(published);
        assert!(cancellation.protected_process_started());
        assert_eq!(managed.state, ChildState::Reaped);
    }

    #[test]
    fn controlled_stream_reply_limit_accepts_one_line_and_reaps_invalid_replies() {
        let mut maximum = controlled_helper("controlled_max_reply", CancellationToken::new());
        let mut terminal_seen = false;
        let output = maximum
            .wait_for_controlled_line_output(1024, &mut |_, bytes| match bytes {
                br#"{"event":"ready"}"# => ProcessLineReply::Reply(
                    format!("{{\"padding\":\"{}\"}}\n", "x".repeat(4081)).into_bytes(),
                ),
                br#"{"event":"terminal"}"# => {
                    terminal_seen = true;
                    ProcessLineReply::Continue
                }
                _ => ProcessLineReply::Continue,
            })
            .unwrap();
        assert!(terminal_seen && output.status_success, "{output:?}");

        for reply in [
            b"{}".to_vec(),
            b"{}\n{}\n".to_vec(),
            b"\xff\n".to_vec(),
            format!("{{\"padding\":\"{}\"}}\n", "x".repeat(4082)).into_bytes(),
        ] {
            let mut invalid =
                controlled_helper("controlled_wait_after_reply", CancellationToken::new());
            let mut reply = Some(reply);
            let error = invalid
                .wait_for_controlled_line_output(1024, &mut |_, bytes| {
                    if bytes == br#"{"event":"ready"}"# {
                        ProcessLineReply::Reply(reply.take().unwrap())
                    } else {
                        ProcessLineReply::Continue
                    }
                })
                .expect_err("invalid reply must stop before its write");
            assert!(error.contains("one LF-terminated UTF-8 line"), "{error}");
            assert_eq!(invalid.state, ChildState::Reaped);
        }
    }

    #[test]
    fn controlled_stream_protection_bounds_finish_and_keeps_consumer_stop() {
        for (overall_timeout, finish_timeout, stop_after_ack) in [
            (None, Duration::from_millis(250), false),
            (Some(Duration::from_secs(2)), Duration::from_secs(30), false),
            (None, Duration::from_secs(2), true),
        ] {
            let cancellation = CancellationToken::new();
            let mut managed =
                controlled_helper("controlled_wait_after_reply", cancellation.clone());
            managed.timeout = overall_timeout;
            let mut ack_seen = false;
            let output = managed
                .wait_for_controlled_line_output(1024, &mut |_, bytes| match bytes {
                    br#"{"event":"ready"}"# => {
                        cancellation.handoff_with_gate(|| Ok(())).unwrap();
                        cancellation.cancel();
                        ProcessLineReply::ReplyAndProtect {
                            reply: b"{\"decision\":\"commit\"}\n".to_vec(),
                            finish_timeout,
                        }
                    }
                    br#"{"event":"waiting"}"# => {
                        ack_seen = true;
                        if stop_after_ack {
                            ProcessLineReply::Stop
                        } else {
                            ProcessLineReply::Continue
                        }
                    }
                    _ => ProcessLineReply::Continue,
                })
                .unwrap();
            assert!(ack_seen, "{output:?}");
            assert!(!output.cancelled, "{output:?}");
            assert_eq!(output.stopped_by_consumer, stop_after_ack, "{output:?}");
            assert_eq!(output.timed_out, !stop_after_ack, "{output:?}");
            assert_eq!(managed.state, ChildState::Reaped);
        }
    }

    #[test]
    fn controlled_stream_finish_deadline_covers_the_tail_after_real_leader_exit() {
        struct TailClock {
            now: std::sync::Mutex<Duration>,
            release_on_sleep: AtomicBool,
            exit_observed: AtomicBool,
            release: PathBuf,
            #[cfg(unix)]
            process_id: u32,
            #[cfg(windows)]
            process: super::RuntimeProcessTreeTestProbe,
        }

        impl super::ProcessWaitClock for TailClock {
            fn elapsed(&self) -> Duration {
                *self.now.lock().unwrap()
            }

            fn sleep(&self, duration: Duration) {
                if self.release_on_sleep.swap(false, Ordering::SeqCst) {
                    std::fs::write(&self.release, b"release owned fixture exit").unwrap();
                    let watchdog = Instant::now() + Duration::from_secs(5);
                    loop {
                        #[cfg(unix)]
                        let exited = super::unix_leader_exit_unreaped(self.process_id)
                            .expect("observe owned fixture exit without reaping")
                            .is_some();
                        #[cfg(windows)]
                        let exited = !self
                            .process
                            .is_alive()
                            .expect("observe retained fixture handle");
                        if exited {
                            break;
                        }
                        assert!(Instant::now() < watchdog, "tail fixture did not exit");
                        thread::yield_now();
                    }
                    self.exit_observed.store(true, Ordering::SeqCst);
                }
                thread::sleep(duration);
            }
        }

        let root = tempfile::tempdir().unwrap();
        let release = root.path().join("release-tail-exit");
        let cancellation = CancellationToken::new();
        let mut managed = ManagedChild::spawn(pending_handoff_command(
            "controlled_tail_after_reply",
            &release,
            cancellation.clone(),
        ))
        .unwrap();
        let clock = TailClock {
            now: std::sync::Mutex::new(Duration::ZERO),
            release_on_sleep: AtomicBool::new(false),
            exit_observed: AtomicBool::new(false),
            release,
            #[cfg(unix)]
            process_id: managed.id(),
            #[cfg(windows)]
            process: super::RuntimeProcessTreeTestProbe::capture(managed.id()).unwrap(),
        };
        let input = super::LineStreamInput::Replies(super::LineReplyWriter::start(
            managed.take_stdin().unwrap(),
        ));
        let mut tails = 0;
        let output = managed
            .wait_for_line_output_inner_with_clock(
                super::StreamFraming::Lines { max_bytes: 1024 },
                &mut |_, bytes| match bytes {
                    br#"{"event":"ready"}"# => {
                        cancellation.handoff_with_gate(|| Ok(())).unwrap();
                        clock.release_on_sleep.store(true, Ordering::SeqCst);
                        ProcessLineReply::ReplyAndProtect {
                            reply: b"{\"decision\":\"commit\"}\n".to_vec(),
                            finish_timeout: Duration::from_secs(1),
                        }
                    }
                    br#"{"event":"tail"}"# => {
                        assert!(clock.exit_observed.load(Ordering::SeqCst));
                        tails += 1;
                        if tails == 2 {
                            // The first callback is followed by the owner's
                            // leader reap. This second callback expires the
                            // protected budget inside its queued final tail.
                            *clock.now.lock().unwrap() = Duration::from_secs(2);
                        }
                        ProcessLineReply::Continue
                    }
                    _ => ProcessLineReply::Continue,
                },
                input,
                &clock,
            )
            .unwrap();
        assert_eq!(tails, 2, "callbacks after the expired finish window ran");
        assert!(output.timed_out, "{output:?}");
        assert!(!output.cancelled && !output.status_success);
        assert_eq!(managed.state, ChildState::Reaped);
        assert!(cancellation.protected_process_started());
    }

    #[test]
    fn completed_line_drain_keeps_stop_only_for_successful_children() {
        for (mode, expected_stop) in [("stream_then_exit", true), ("stream_then_fail", false)] {
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "platform::process::tests::managed_child_test_helper",
                    "--ignored",
                    "--nocapture",
                ])
                .env(HELPER_ENV, mode)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            let stdout = super::start_line_reader(
                child.stdout.take(),
                super::StreamFraming::Lines { max_bytes: 1024 },
            );
            let stderr = super::start_reader(child.stderr.take(), super::STDERR_CAPTURE_LIMIT);
            let status = child.wait().expect("child finished before output drain");
            let mut first_error = None;
            let mut lines = 0;
            let output = super::finish_completed_line_output(
                status,
                &stdout,
                stderr,
                &mut |_, _| {
                    lines += 1;
                    if lines == 200 {
                        StreamControl::Stop
                    } else {
                        StreamControl::Continue
                    }
                },
                &mut first_error,
            );
            assert_eq!(lines, 200, "{mode}");
            assert_eq!(
                output.stopped_by_consumer, expected_stop,
                "{mode}: {output:?}"
            );
            assert!(!output.status_success);
            if !expected_stop {
                assert!(output.stderr.contains("expected child failure"));
            }
        }
    }

    #[test]
    fn managed_child_reaps_already_exited_process_without_tree_kill() {
        let start = HelperStart::new();
        let mut managed = ManagedChild::spawn(ManagedCommand {
            program: std::env::current_exe().unwrap(),
            args: vec![
                "--exact".into(),
                "platform::process::tests::managed_child_test_helper".into(),
                "--ignored".into(),
                "--nocapture".into(),
            ],
            cwd: std::env::current_dir().unwrap(),
            env: vec![
                (OsString::from(HELPER_ENV), OsString::from("success")),
                start.env(),
            ],
            env_remove: Vec::new(),
            capture_limits: None,
            timeout: None,
            cancellation: CancellationToken::new(),
        })
        .unwrap();
        thread::sleep(Duration::from_millis(100));
        start.assert_started();

        managed.terminate().unwrap();
        assert_eq!(managed.state, ChildState::Reaped);
    }

    #[test]
    fn managed_child_drop_terminates_and_reaps_running_process() {
        let start = HelperStart::new();
        let managed = ManagedChild::spawn(ManagedCommand {
            program: std::env::current_exe().unwrap(),
            args: vec![
                "--exact".into(),
                "platform::process::tests::managed_child_test_helper".into(),
                "--ignored".into(),
                "--nocapture".into(),
            ],
            cwd: std::env::current_dir().unwrap(),
            env: vec![
                (OsString::from(HELPER_ENV), OsString::from("sleep")),
                start.env(),
            ],
            env_remove: Vec::new(),
            capture_limits: None,
            timeout: None,
            cancellation: CancellationToken::new(),
        })
        .unwrap();
        start.assert_started();
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
                "platform::process::tests::managed_child_test_helper".into(),
                "--ignored".into(),
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

    #[test]
    fn pending_launch_release_preserves_detached_descendant() {
        let directory = tempfile::tempdir().unwrap();
        let pid_file = directory.path().join("descendant.pid");
        #[cfg(unix)]
        super::reset_unix_signal_count_for_test();
        let (output, mut pending) = ManagedChild::run_pending_handoff(pending_handoff_command(
            "exit_leaving_detached_grandchild",
            &pid_file,
            CancellationToken::new(),
        ))
        .unwrap();
        let descendant = read_helper_pid(&pid_file, Duration::from_secs(2));
        let _cleanup = ProcessCleanupGuard(vec![descendant]);
        assert!(output.status_success, "{output:?}");
        assert!(output.stdout.contains("grandchild-detached"));
        assert!(process_test_support::is_alive(descendant));
        pending.release().unwrap();
        drop(pending);
        assert!(
            process_test_support::is_alive(descendant),
            "verified release must not terminate the detached client"
        );
        #[cfg(unix)]
        assert_eq!(super::unix_signal_count_for_test(), 0);
    }

    #[test]
    fn pending_launch_rejection_drops_owned_descendant() {
        let directory = tempfile::tempdir().unwrap();
        let pid_file = directory.path().join("descendant.pid");
        let (output, pending) = ManagedChild::run_pending_handoff(pending_handoff_command(
            "exit_leaving_detached_grandchild",
            &pid_file,
            CancellationToken::new(),
        ))
        .unwrap();
        let descendant = read_helper_pid(&pid_file, Duration::from_secs(2));
        let mut cleanup = ProcessCleanupGuard(vec![descendant]);
        assert!(output.status_success, "{output:?}");
        drop(pending);
        assert!(wait_until_dead(descendant, Duration::from_secs(2)));
        cleanup.disarm();
    }

    #[cfg(windows)]
    fn publish_handoff_probe_file(path: &Path, contents: &str) {
        let pending = path.with_extension("publishing");
        let mut file = std::fs::File::create(&pending).unwrap();
        file.write_all(contents.as_bytes()).unwrap();
        file.sync_all().unwrap();
        drop(file);
        std::fs::rename(pending, path).unwrap();
    }

    #[cfg(windows)]
    fn assert_pending_handoff_owner_crash(released: bool) {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
        use windows_sys::Win32::System::Threading::WaitForSingleObject;

        let directory = tempfile::tempdir().unwrap();
        let pid_file = directory.path().join("client.pid");
        let ready_file = pid_file.with_extension("ready");
        // A plain child is essential: an outer managed Job would hide whether
        // the pending handoff's own Job kills the client when its owner crashes.
        let mut owner = ChildCleanupGuard(Some(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "platform::process::tests::managed_child_test_helper",
                    "--ignored",
                    "--nocapture",
                ])
                .env(
                    HELPER_ENV,
                    if released {
                        "windows_released_handoff_owner"
                    } else {
                        "windows_pending_handoff_owner"
                    },
                )
                .env(HELPER_PID_FILE_ENV, &pid_file)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap(),
        ));
        let pid = read_helper_pid(&pid_file, Duration::from_secs(10));
        let client = process_test_support::RetainedProcess::open(pid);
        let deadline = Instant::now() + Duration::from_secs(10);
        while !ready_file.exists() {
            assert!(
                owner.child_mut().try_wait().unwrap().is_none(),
                "owner exited before its barrier"
            );
            assert!(
                Instant::now() < deadline,
                "owner did not publish its handoff barrier"
            );
            thread::yield_now();
        }
        assert_eq!(std::fs::read_to_string(ready_file).unwrap(), "ready");
        assert!(
            !client.exited_within(0),
            "client must be alive before owner termination"
        );
        owner.child_mut().kill().unwrap();
        // SAFETY: Child owns a live process handle with synchronization access.
        assert_eq!(
            unsafe { WaitForSingleObject(owner.child_mut().as_raw_handle() as _, 5_000) },
            WAIT_OBJECT_0,
            "terminated owner did not exit within the cleanup bound"
        );
        owner.wait();
        assert_eq!(
            client.exited_within(2_000),
            !released,
            "client lifetime must follow whether the owner released its pending handoff"
        );
        if released {
            client.terminate_and_wait();
        }
    }

    #[cfg(windows)]
    #[test]
    fn pending_launch_owner_crash_kills_unreleased_client() {
        assert_pending_handoff_owner_crash(false);
    }

    #[cfg(windows)]
    #[test]
    fn released_launch_survives_owner_crash() {
        assert_pending_handoff_owner_crash(true);
    }

    #[cfg(unix)]
    #[test]
    fn failed_pending_release_keeps_retained_leader_authority_for_drop_cleanup() {
        let directory = tempfile::tempdir().unwrap();
        let pid_file = directory.path().join("descendant.pid");
        let (output, mut pending) = ManagedChild::run_pending_handoff(pending_handoff_command(
            "exit_leaving_detached_grandchild",
            &pid_file,
            CancellationToken::new(),
        ))
        .unwrap();
        let descendant = read_helper_pid(&pid_file, Duration::from_secs(2));
        let mut cleanup = ProcessCleanupGuard(vec![descendant]);
        assert!(output.status_success, "{output:?}");
        assert!(process_test_support::is_alive(descendant));

        super::inject_unix_reap_error_for_test();
        let error = pending
            .release()
            .expect_err("injected reap must not release");
        assert!(error.contains("injected Unix reap failure"), "{error}");
        assert!(process_test_support::is_alive(descendant));
        drop(pending);
        assert!(
            wait_until_dead(descendant, Duration::from_secs(2)),
            "a failed release must still clean its retained process group"
        );
        cleanup.disarm();
    }

    #[cfg(unix)]
    #[test]
    fn pending_release_does_not_signal_after_external_leader_reap() {
        let directory = tempfile::tempdir().unwrap();
        let pid_file = directory.path().join("descendant.pid");
        let (_, mut pending) = ManagedChild::run_pending_handoff(pending_handoff_command(
            "exit_leaving_detached_grandchild",
            &pid_file,
            CancellationToken::new(),
        ))
        .unwrap();
        let descendant = read_helper_pid(&pid_file, Duration::from_secs(2));
        let _cleanup = ProcessCleanupGuard(vec![descendant]);
        let leader = pending.child.as_ref().expect("real handoff").child.id();
        let mut status = 0;
        // Model an unrelated process-wide reaper consuming the runner's zombie
        // after our WNOWAIT observation but before release.
        assert_eq!(
            unsafe { libc::waitpid(leader as i32, &mut status, 0) },
            leader as i32
        );
        super::reset_unix_signal_count_for_test();
        assert!(pending.release().is_err());
        drop(pending);
        assert_eq!(
            super::unix_signal_count_for_test(),
            0,
            "lost leader authority must never signal a numeric PGID"
        );
        assert!(
            process_test_support::is_alive(descendant),
            "the unrelated reaper already removed our cleanup authority"
        );
    }

    #[test]
    fn pending_launch_cancel_during_startup_cleans_owned_descendant() {
        let directory = tempfile::tempdir().unwrap();
        let pid_file = directory.path().join("pids");
        let cancellation = CancellationToken::new();
        let command = pending_handoff_command(
            "process_tree_immediate_parent",
            &pid_file,
            cancellation.clone(),
        );
        let launched = thread::spawn(move || ManagedChild::run_pending_handoff(command));
        let pids = read_helper_pids(&pid_file, Duration::from_secs(2));
        let mut cleanup = ProcessCleanupGuard(pids.clone());
        cancellation.cancel();
        let error = match launched.join().unwrap() {
            Ok(_) => panic!("cancelled startup unexpectedly returned a handoff"),
            Err(error) => error,
        };
        assert!(error.starts_with(unica_work_control::cancellation::CANCELLED_PREFIX));
        assert!(wait_until_dead(pids[1], Duration::from_secs(2)));
        cleanup.disarm();
    }

    #[cfg(unix)]
    #[test]
    fn pending_launch_complete_json_without_eof_remains_cancelable() {
        let directory = tempfile::tempdir().unwrap();
        let pid_file = directory.path().join("pids");
        let cancellation = CancellationToken::new();
        let command = pending_handoff_command(
            "exit_leaving_inherited_pipe_grandchild",
            &pid_file,
            cancellation.clone(),
        );
        let launched = thread::spawn(move || ManagedChild::run_pending_handoff(command));
        let pids = read_helper_pids(&pid_file, Duration::from_secs(2));
        let mut cleanup = ProcessCleanupGuard(pids.clone());
        let deadline = Instant::now() + Duration::from_secs(2);
        while super::unix_leader_exit_unreaped(pids[0]).unwrap().is_none() {
            assert!(Instant::now() < deadline, "runner leader did not exit");
            thread::yield_now();
        }
        assert!(
            !launched.is_finished(),
            "a complete JSON prefix cannot release the group before actual EOF"
        );
        cancellation.cancel();
        let error = match launched.join().unwrap() {
            Ok(_) => panic!("inherited pipe unexpectedly produced a handoff"),
            Err(error) => error,
        };
        assert!(error.starts_with(unica_work_control::cancellation::CANCELLED_PREFIX));
        assert!(wait_until_dead(pids[1], Duration::from_secs(2)));
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
                "platform::process::tests::managed_child_test_helper".into(),
                "--ignored".into(),
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
                "platform::process::tests::managed_child_test_helper".into(),
                "--ignored".into(),
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
    fn expired_startup_deadline_refuses_before_spawning() {
        let command = Command::new("unica-nonexistent-startup-fixture");
        let result = ManagedStartupChild::spawn_configured_before(command, Instant::now());
        assert!(
            matches!(result, Err(message) if message == "startup deadline expired before process creation")
        );
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
                "platform::process::tests::managed_child_test_helper",
                "--ignored",
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
        let start = HelperStart::new();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "platform::process::tests::managed_child_test_helper",
                "--ignored",
                "--nocapture",
            ])
            .env(HELPER_ENV, "success")
            .env(HELPER_STARTED_ENV, &start.marker)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut startup = ManagedStartupChild::spawn_configured(command).unwrap();
        start.assert_started();
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
        let start = HelperStart::new();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "platform::process::tests::managed_child_test_helper",
                "--ignored",
                "--nocapture",
            ])
            .env(HELPER_ENV, "sleep")
            .env(HELPER_STARTED_ENV, &start.marker)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut startup = ManagedStartupChild::spawn_configured(command).unwrap();
        let pid = startup.id();
        let _cleanup = ProcessCleanupGuard(vec![pid]);
        start.assert_started();
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
        let start = HelperStart::new();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "platform::process::tests::managed_child_test_helper",
                "--ignored",
                "--nocapture",
            ])
            .env(HELPER_ENV, "sleep")
            .env(HELPER_STARTED_ENV, &start.marker)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut startup = ManagedStartupChild::spawn_configured(command).unwrap();
        start.assert_started();
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
        #[cfg(windows)]
        process_tree_keeps_child_suspended_until_attach();
    }

    #[test]
    fn startup_child_detach_leaves_process_running() {
        let start = HelperStart::new();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "platform::process::tests::managed_child_test_helper",
                "--ignored",
                "--nocapture",
            ])
            .env(HELPER_ENV, "sleep")
            .env(HELPER_STARTED_ENV, &start.marker)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut startup = ManagedStartupChild::spawn_configured(command).unwrap();
        let pid = startup.id();
        let _cleanup = ProcessCleanupGuard(vec![pid]);
        start.assert_started();

        startup.detach().unwrap();

        thread::sleep(Duration::from_millis(75));
        assert!(process_test_support::is_alive(pid));
    }

    #[test]
    fn startup_child_exposes_exit_status_without_detaching() {
        let start = HelperStart::new();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "platform::process::tests::managed_child_test_helper",
                "--ignored",
                "--nocapture",
            ])
            .env(HELPER_ENV, "success")
            .env(HELPER_STARTED_ENV, &start.marker)
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
        start.assert_started();
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
                "platform::process::tests::managed_child_test_helper",
                "--ignored",
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

    #[cfg(windows)]
    #[test]
    fn slow_job_attach_delays_cancel_without_starting_the_mutation_early() {
        use std::sync::mpsc;

        let marker = std::env::temp_dir().join(format!(
            "unica-slow-attach-marker-{}-{}",
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
                "platform::process::tests::managed_child_test_helper",
                "--ignored",
                "--nocapture",
            ])
            .env(HELPER_ENV, "write_marker")
            .env(HELPER_PID_FILE_ENV, &marker)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut tree = ProcessTree::prepare(&mut command).unwrap();
        let cancellation = CancellationToken::new();
        let protected = cancellation.protect_process_on_spawn();
        let (spawned_tx, spawned_rx) = mpsc::channel();
        let (attach_tx, attach_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            let (child, detached_cancellation) = protected
                .spawn_with_gate(|| {
                    let mut child = ChildCleanupGuard(Some(command.spawn().unwrap()));
                    spawned_tx.send(()).unwrap();
                    attach_rx.recv_timeout(Duration::from_secs(10)).unwrap();
                    tree.attach(child.child_mut()).unwrap();
                    Ok(child)
                })
                .unwrap();
            assert!(!detached_cancellation.is_cancelled());
            child.wait();
        });
        spawned_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(
            !marker.exists(),
            "suspended child ran before Job attachment"
        );

        let (started_tx, started_rx) = mpsc::channel();
        let (cancelled_tx, cancelled_rx) = mpsc::channel();
        let canceller = thread::spawn(move || {
            let began = Instant::now();
            started_tx.send(()).unwrap();
            cancellation.cancel();
            cancelled_tx.send(began.elapsed()).unwrap();
            cancellation
        });
        started_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(
            cancelled_rx
                .recv_timeout(Duration::from_millis(250))
                .is_err(),
            "cancel returned while Job attachment was still in flight"
        );
        assert!(!marker.exists(), "mutation began before Job attachment");
        attach_tx.send(()).unwrap();
        let elapsed = cancelled_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(
            elapsed >= Duration::from_millis(250),
            "cancel waited only {elapsed:?}"
        );
        let cancellation = canceller.join().unwrap();
        worker.join().unwrap();
        assert!(cancellation.protected_process_started());
        assert!(
            marker.exists(),
            "protected child did not finish after late cancel"
        );
    }
}
