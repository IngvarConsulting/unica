//! Windows creation-time containment for the daemon started by bootstrap.
//! The independent daemon owner holds this Job until private startup is proven.

use super::daemon_gate::{StartupGate, GATE_ENV};
use super::job_object::RuntimeJob;
use std::ffi::{OsStr, OsString};
use std::io;
use std::mem::{size_of, zeroed};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::os::windows::process::ExitStatusExt;
use std::path::Path;
use std::process::ExitStatus;
use std::ptr::{null, null_mut};
use std::time::Instant;
use windows_sys::Win32::Foundation::{
    GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::Globalization::{CompareStringOrdinal, CSTR_GREATER_THAN, CSTR_LESS_THAN};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, GetExitCodeProcess,
    InitializeProcThreadAttributeList, ResumeThread, UpdateProcThreadAttribute,
    WaitForSingleObject, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT,
    EXTENDED_STARTUPINFO_PRESENT, LPPROC_THREAD_ATTRIBUTE_LIST, PROCESS_INFORMATION,
    PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROC_THREAD_ATTRIBUTE_JOB_LIST, STARTF_USESTDHANDLES,
    STARTUPINFOEXW,
};

pub(super) struct PendingDaemonProcess {
    process: OwnedHandle,
    job: Option<RuntimeJob>,
    gate: Option<StartupGate>,
    pid: u32,
    aborted: bool,
}

impl PendingDaemonProcess {
    pub(super) fn spawn(
        program: &Path,
        args: &[OsString],
        cwd: &Path,
        environment: Vec<(OsString, OsString)>,
        deadline: Instant,
    ) -> io::Result<Self> {
        let (child, thread) =
            Self::create_suspended(program, args, cwd, environment, deadline, true)?;
        check_deadline(deadline)?;
        // SAFETY: this is the initial thread handle returned for this suspended process.
        let previous = unsafe { ResumeThread(thread.as_raw_handle() as HANDLE) };
        if previous == 1 {
            return Ok(child);
        }
        let mut child = child;
        let failure = if previous == u32::MAX {
            io::Error::last_os_error()
        } else {
            io::Error::other(format!(
                "unexpected daemon thread suspend count: {previous}"
            ))
        };
        match child.abort(deadline) {
            Ok(()) => Err(failure),
            Err(cleanup) => Err(io::Error::other(format!("{failure}; {cleanup}"))),
        }
    }

    fn create_suspended(
        program: &Path,
        args: &[OsString],
        cwd: &Path,
        mut environment: Vec<(OsString, OsString)>,
        deadline: Instant,
        gated: bool,
    ) -> io::Result<(Self, OwnedHandle)> {
        let job = RuntimeJob::create_pending()?;
        let stdio = open_inheritable_null()?;
        let jobs = [job.raw()];
        let mut handles = vec![stdio.as_raw_handle() as HANDLE];
        let gate = gated.then(StartupGate::new).transpose()?;
        environment.retain(|(key, _)| !key.eq_ignore_ascii_case(GATE_ENV));
        if let Some((_, child)) = gate.as_ref() {
            handles.extend(child.handles());
            environment.push(child.environment(deadline));
        }
        let attributes = StartupAttributes::new(&jobs, &handles)?;
        let program_name = wide_nul(program.as_os_str())?;
        let cwd = wide_nul(cwd.as_os_str())?;
        let mut argv = command_line(program.as_os_str(), args)?;
        let environment = environment_block(environment)?;
        // SAFETY: both are Windows POD types; the required size is supplied below.
        let mut startup: STARTUPINFOEXW = unsafe { zeroed() };
        let mut result: PROCESS_INFORMATION = unsafe { zeroed() };
        startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        startup.StartupInfo.hStdInput = handles[0];
        startup.StartupInfo.hStdOutput = handles[0];
        startup.StartupInfo.hStdError = handles[0];
        startup.lpAttributeList = attributes.raw();
        check_deadline(deadline)?;

        // SAFETY: paths, command line, environment and attribute data outlive the call.
        // Inherit only null stdio and the least-privilege private gate handles.
        // The startup Job owns the child from creation, before its thread can run.
        if unsafe {
            CreateProcessW(
                program_name.as_ptr(),
                argv.as_mut_ptr(),
                null(),
                null(),
                1,
                EXTENDED_STARTUPINFO_PRESENT | CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT,
                environment.as_ptr().cast(),
                cwd.as_ptr(),
                &startup.StartupInfo,
                &mut result,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: CreateProcessW returned ownership of both distinct handles.
        let process = unsafe { OwnedHandle::from_raw_handle(result.hProcess) };
        let thread = unsafe { OwnedHandle::from_raw_handle(result.hThread) };
        Ok((
            Self {
                process,
                job: Some(job),
                gate: gate.map(|(bootstrap, _child)| bootstrap),
                pid: result.dwProcessId,
                aborted: false,
            },
            thread,
        ))
    }

    pub(super) fn pid(&self) -> u32 {
        self.pid
    }

    pub(super) fn try_wait(&self) -> io::Result<Option<ExitStatus>> {
        let process = self.process.as_raw_handle() as HANDLE;
        // SAFETY: process remains owned for both calls.
        match unsafe { WaitForSingleObject(process, 0) } {
            WAIT_TIMEOUT => Ok(None),
            WAIT_OBJECT_0 => {
                let mut code = 0;
                if unsafe { GetExitCodeProcess(process, &mut code) } == 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(Some(ExitStatus::from_raw(code)))
            }
            _ => Err(io::Error::last_os_error()),
        }
    }

    pub(super) fn is_ready(&self) -> io::Result<bool> {
        self.gate.as_ref().map_or(Ok(true), StartupGate::ready)
    }

    pub(super) fn commit(&mut self, deadline: Instant) -> io::Result<()> {
        check_deadline(deadline)?;
        if self.try_wait()?.is_some() {
            return Err(io::Error::other("daemon exited before handoff"));
        }
        if !self.is_ready()? {
            return Err(io::Error::other(
                "daemon has not signalled private readiness",
            ));
        }
        let job = self
            .job
            .as_ref()
            .ok_or_else(|| io::Error::other("daemon startup already finished"))?;
        job.release_pending()?;
        // Only a detached process may publish the public endpoint. If bootstrap
        // dies before signalling, the child's liveness handle wakes its gate.
        if let Some(gate) = self.gate.as_ref() {
            if let Err(error) = gate.permit() {
                let cleanup = self.abort(deadline);
                return match cleanup {
                    Ok(()) => Err(error),
                    Err(cleanup) => Err(io::Error::other(format!("{error}; {cleanup}"))),
                };
            }
        }
        self.job.take();
        Ok(())
    }

    pub(super) fn abort(&mut self, deadline: Instant) -> io::Result<()> {
        if self.aborted {
            return Ok(());
        }
        let job = self
            .job
            .as_ref()
            .ok_or_else(|| io::Error::other("daemon startup is no longer pending"))?;
        job.terminate_until(deadline)?;
        // Job accounting can reach zero before the exact process handle signals.
        // Keep the same cutoff before reporting an acknowledged abort.
        let millis = deadline
            .saturating_duration_since(Instant::now())
            .as_millis()
            .min(u128::from(u32::MAX - 1)) as u32;
        // SAFETY: the process handle is retained for this bounded wait.
        match unsafe { WaitForSingleObject(self.process.as_raw_handle() as HANDLE, millis) } {
            WAIT_OBJECT_0 => {}
            WAIT_TIMEOUT => {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "daemon process has not exited",
                ));
            }
            _ => return Err(io::Error::last_os_error()),
        }
        self.job.take();
        self.aborted = true;
        Ok(())
    }
}

pub(super) fn check_deadline(deadline: Instant) -> io::Result<()> {
    if Instant::now() >= deadline {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "daemon startup deadline expired",
        ));
    }
    Ok(())
}

struct StartupAttributes<'a> {
    storage: Vec<usize>,
    _jobs: &'a [HANDLE],
    _handles: &'a [HANDLE],
}

impl<'a> StartupAttributes<'a> {
    fn new(jobs: &'a [HANDLE], handles: &'a [HANDLE]) -> io::Result<Self> {
        let mut size = 0;
        // SAFETY: this is the documented size query, for exactly two attributes.
        unsafe { InitializeProcThreadAttributeList(null_mut(), 2, 0, &mut size) };
        if size == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut storage = vec![0_usize; size.div_ceil(size_of::<usize>())];
        // SAFETY: storage is address-aligned, covers size bytes and is not resized.
        if unsafe {
            InitializeProcThreadAttributeList(storage.as_mut_ptr().cast(), 2, 0, &mut size)
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        let mut attributes = Self {
            storage,
            _jobs: jobs,
            _handles: handles,
        };
        attributes.add(PROC_THREAD_ATTRIBUTE_JOB_LIST, jobs)?;
        attributes.add(PROC_THREAD_ATTRIBUTE_HANDLE_LIST, handles)?;
        Ok(attributes)
    }

    fn raw(&self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        self.storage.as_ptr().cast_mut().cast()
    }

    fn add(&mut self, attribute: u32, values: &'a [HANDLE]) -> io::Result<()> {
        // SAFETY: the handle arrays are retained through DeleteProcThreadAttributeList.
        if unsafe {
            UpdateProcThreadAttribute(
                self.raw(),
                0,
                attribute as usize,
                values.as_ptr().cast(),
                std::mem::size_of_val(values),
                null_mut(),
                null(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

impl Drop for StartupAttributes<'_> {
    fn drop(&mut self) {
        // SAFETY: an initialized list is deleted exactly once before its buffer.
        unsafe { DeleteProcThreadAttributeList(self.raw()) };
    }
}

fn open_inheritable_null() -> io::Result<OwnedHandle> {
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: null_mut(),
        bInheritHandle: 1,
    };
    let name = [u16::from(b'N'), u16::from(b'U'), u16::from(b'L'), 0];
    // SAFETY: the Windows null device name is NUL-terminated; the
    // acquisition is limited to this call's stdio and not to arbitrary files.
    let handle = unsafe {
        CreateFileW(
            name.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            &attributes,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: CreateFileW returned an owned handle.
    Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
}

fn wide_nul(value: &OsStr) -> io::Result<Vec<u16>> {
    let mut wide = value.encode_wide().collect::<Vec<_>>();
    if wide.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "embedded Windows NUL",
        ));
    }
    wide.push(0);
    Ok(wide)
}

fn command_line(program: &OsStr, args: &[OsString]) -> io::Result<Vec<u16>> {
    let mut result = Vec::new();
    for value in std::iter::once(program).chain(args.iter().map(OsString::as_os_str)) {
        if !result.is_empty() {
            result.push(u16::from(b' '));
        }
        quote_argument(&mut result, &wide_nul(value)?);
    }
    result.push(0);
    Ok(result)
}

fn quote_argument(result: &mut Vec<u16>, value: &[u16]) {
    let quote = u16::from(b'"');
    let slash = u16::from(b'\\');
    result.push(quote);
    let mut backslashes = 0;
    for &ch in &value[..value.len() - 1] {
        if ch == slash {
            backslashes += 1;
        } else {
            let escaped = if ch == quote {
                backslashes * 2 + 1
            } else {
                backslashes
            };
            result.extend(std::iter::repeat_n(slash, escaped));
            result.push(ch);
            backslashes = 0;
        }
    }
    result.extend(std::iter::repeat_n(slash, backslashes * 2));
    result.push(quote);
}

fn environment_block(values: Vec<(OsString, OsString)>) -> io::Result<Vec<u16>> {
    let mut values = values
        .into_iter()
        .map(|(key, value)| Ok((wide_nul(&key)?, wide_nul(&value)?)))
        .collect::<io::Result<Vec<_>>>()?;
    values.sort_by(|(left, _), (right, _)| {
        // SAFETY: both slices hold valid NUL-terminated UTF-16 storage.
        let order = unsafe { CompareStringOrdinal(left.as_ptr(), -1, right.as_ptr(), -1, 1) };
        match order {
            CSTR_LESS_THAN => std::cmp::Ordering::Less,
            CSTR_GREATER_THAN => std::cmp::Ordering::Greater,
            _ => std::cmp::Ordering::Equal,
        }
    });
    let mut block = Vec::new();
    for (mut key, value) in values {
        *key.last_mut().expect("NUL-terminated key") = u16::from(b'=');
        block.extend(key);
        block.extend(value);
    }
    if block.is_empty() {
        block.push(0);
    }
    block.push(0);
    Ok(block)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use windows_sys::Win32::System::JobObjects::IsProcessInJob;
    use windows_sys::Win32::System::Threading::TerminateProcess;

    fn fixture() -> (PendingDaemonProcess, OwnedHandle) {
        let system_root = std::env::var_os("SystemRoot").expect("SystemRoot");
        let system32 = std::path::PathBuf::from(system_root).join("System32");
        let args = ["-n".into(), "60".into(), "127.0.0.1".into()];
        PendingDaemonProcess::create_suspended(
            &system32.join("ping.exe"),
            &args,
            &system32,
            std::env::vars_os().collect(),
            Instant::now() + Duration::from_secs(10),
            false,
        )
        .expect("launch suspended fixture")
    }

    fn wait_for_exit(process: HANDLE) {
        // SAFETY: the test holds the exact process handle until the wait returns.
        assert_eq!(
            unsafe { WaitForSingleObject(process, 10_000) },
            WAIT_OBJECT_0
        );
    }

    /// rule: INV.PLATFORM.PROCESS-TREE-LIFECYCLE
    #[test]
    fn closing_the_startup_job_terminates_even_a_never_resumed_daemon() {
        let (child, _suspended) = fixture();
        let process = child.process.as_raw_handle() as HANDLE;
        let mut member = 0;
        // SAFETY: the process and Job handles remain valid.
        assert_ne!(
            unsafe { IsProcessInJob(process, child.job.as_ref().unwrap().raw(), &mut member) },
            0
        );
        assert_ne!(member, 0);
        let PendingDaemonProcess {
            process: _process,
            job,
            ..
        } = child;
        drop(job);
        wait_for_exit(process);
    }

    /// rule: INV.PLATFORM.PROCESS-TREE-LIFECYCLE
    #[test]
    fn abort_terminates_a_resumed_daemon_before_returning() {
        let (mut child, thread) = fixture();
        // SAFETY: this is the retained suspended primary thread.
        assert_eq!(unsafe { ResumeThread(thread.as_raw_handle() as HANDLE) }, 1);
        assert!(child.try_wait().unwrap().is_none());
        child
            .abort(Instant::now() + Duration::from_secs(5))
            .unwrap();
        assert!(child.try_wait().unwrap().is_some());
    }

    /// rule: INV.RUNTIME.DAEMON-ACTIVE-OWNERS
    #[test]
    fn after_commit_closing_the_startup_job_keeps_the_daemon_alive() {
        let (mut child, thread) = fixture();
        // SAFETY: this is the retained suspended primary thread.
        assert_eq!(unsafe { ResumeThread(thread.as_raw_handle() as HANDLE) }, 1);
        child
            .commit(Instant::now() + Duration::from_secs(5))
            .unwrap();
        let process = child.process.as_raw_handle() as HANDLE;
        // SAFETY: process is still held; explicitly dispose of the test fixture.
        let state = unsafe { WaitForSingleObject(process, 100) };
        unsafe { TerminateProcess(process, 1) };
        wait_for_exit(process);
        assert_eq!(state, WAIT_TIMEOUT);
    }

    const PUBLICATION_FIXTURE: &str = "UNICA_PENDING_DAEMON_PUBLICATION_FIXTURE";

    #[test]
    #[ignore = "helper process for the inherited private daemon gate"]
    fn publication_gate_child() {
        let marker = std::env::var_os(PUBLICATION_FIXTURE).expect("publication marker");
        crate::await_bootstrap_daemon_publication().expect("owner permitted publication");
        std::fs::write(marker, b"published").expect("record publication");
    }

    /// rule: INV.PLATFORM.PROCESS-TREE-LIFECYCLE
    #[test]
    fn private_readiness_without_commit_cannot_publish_an_endpoint() {
        for commit in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let marker = directory.path().join("publication");
            let environment = vec![(PUBLICATION_FIXTURE.into(), marker.as_os_str().into())];
            let args = [
                "--exact".into(),
                "platform::pending_daemon::tests::publication_gate_child".into(),
                "--ignored".into(),
            ];
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut child = PendingDaemonProcess::spawn(
                &std::env::current_exe().unwrap(),
                &args,
                directory.path(),
                environment,
                deadline,
            )
            .unwrap();
            while !child.is_ready().unwrap() {
                assert!(
                    child.try_wait().unwrap().is_none(),
                    "child exited before private readiness"
                );
                check_deadline(deadline).unwrap();
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(!marker.exists(), "private readiness was exposed early");
            if commit {
                child.commit(deadline).unwrap();
                wait_for_exit(child.process.as_raw_handle());
                assert_eq!(std::fs::read(&marker).unwrap(), b"published");
                assert!(child.try_wait().unwrap().unwrap().success());
            } else {
                child.abort(deadline).unwrap();
                assert!(child.try_wait().unwrap().is_some());
                assert!(!marker.exists());
            }
        }
    }
}
