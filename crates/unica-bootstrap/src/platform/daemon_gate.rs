use std::ffi::OsString;
use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::ptr::null;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{
    DuplicateHandle, GetHandleInformation, SetHandleInformation, HANDLE, HANDLE_FLAG_INHERIT,
    WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::System::SystemInformation::GetTickCount64;
use windows_sys::Win32::System::Threading::{
    CreateEventW, GetCurrentProcess, OpenProcess, SetEvent, WaitForMultipleObjects,
    WaitForSingleObject, EVENT_MODIFY_STATE, PROCESS_SYNCHRONIZE,
};

pub(super) const GATE_ENV: &str = "UNICA_BOOTSTRAP_DAEMON_GATE";

/// The startup daemon receives only a ready sender, a publication receiver
/// and a wait-only handle to this bootstrap. None of these objects are named.
pub(super) struct StartupGate {
    ready: OwnedHandle,
    publish: OwnedHandle,
}

pub(super) struct ChildGate {
    ready: OwnedHandle,
    publish: OwnedHandle,
    bootstrap: OwnedHandle,
}

impl StartupGate {
    pub(super) fn new() -> io::Result<(Self, ChildGate)> {
        let ready = event()?;
        let publish = event()?;
        let child = ChildGate {
            ready: inheritable(&ready, EVENT_MODIFY_STATE)?,
            publish: inheritable(&publish, PROCESS_SYNCHRONIZE)?,
            // SAFETY: opens a real, wait-only handle to the live bootstrap.
            bootstrap: acquire(unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 1, std::process::id()) })?,
        };
        Ok((Self { ready, publish }, child))
    }

    pub(super) fn ready(&self) -> io::Result<bool> {
        // SAFETY: this is a retained event with SYNCHRONIZE access.
        match unsafe { WaitForSingleObject(raw(&self.ready), 0) } {
            WAIT_OBJECT_0 => Ok(true),
            WAIT_TIMEOUT => Ok(false),
            _ => Err(io::Error::last_os_error()),
        }
    }

    pub(super) fn permit(&self) -> io::Result<()> {
        set_event(&self.publish)
    }
}

impl ChildGate {
    pub(super) fn handles(&self) -> [HANDLE; 3] {
        [raw(&self.ready), raw(&self.publish), raw(&self.bootstrap)]
    }

    pub(super) fn environment(&self, deadline: Instant) -> (OsString, OsString) {
        let [ready, publish, bootstrap] = self.handles().map(|handle| handle as usize);
        let cutoff = cutoff_at(deadline);
        (
            GATE_ENV.into(),
            format!("{ready}:{publish}:{bootstrap}:{cutoff}").into(),
        )
    }
}

/// Called only after the exact daemon's private listener and runtime are ready.
#[doc(hidden)]
pub fn await_bootstrap_daemon_publication() -> io::Result<()> {
    let encoded = match std::env::var(GATE_ENV) {
        Ok(encoded) => encoded,
        Err(std::env::VarError::NotPresent) => return Ok(()),
        Err(_) => return Err(invalid_gate()),
    };
    let mut words = encoded.split(':');
    let mut next = || words.next().ok_or_else(invalid_gate);
    let ready: usize = next()?.parse().map_err(|_| invalid_gate())?;
    let publish: usize = next()?.parse().map_err(|_| invalid_gate())?;
    let bootstrap: usize = next()?.parse().map_err(|_| invalid_gate())?;
    let cutoff: u64 = next()?.parse().map_err(|_| invalid_gate())?;
    let deadline = deadline_at(cutoff)?;
    if words.next().is_some()
        || [ready, publish, bootstrap].contains(&0)
        || ready == publish
        || ready == bootstrap
        || publish == bootstrap
    {
        return Err(invalid_gate());
    }
    // Check and clear the inherit bit before adopting each inherited handle.
    // This prevents leaking gate rights to any daemon descendants.
    let child = ChildGate {
        ready: from_inherited(ready as HANDLE)?,
        publish: from_inherited(publish as HANDLE)?,
        bootstrap: from_inherited(bootstrap as HANDLE)?,
    };
    child.await_permit(deadline)
}

impl ChildGate {
    fn await_permit(&self, deadline: Instant) -> io::Result<()> {
        super::pending_daemon::check_deadline(deadline)?;
        set_event(&self.ready)?;
        // Publication wins if the permit arrived before bootstrap exited.
        let handles = [raw(&self.publish), raw(&self.bootstrap)];
        let millis = deadline
            .saturating_duration_since(Instant::now())
            .as_millis();
        let millis = millis.min(u128::from(u32::MAX - 1)) as u32;
        // SAFETY: both wait-only descriptors remain in this scope.
        match unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, millis) } {
            WAIT_OBJECT_0 => super::pending_daemon::check_deadline(deadline),
            WAIT_TIMEOUT => Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "daemon publication was not permitted",
            )),
            exited if exited == WAIT_OBJECT_0 + 1 => Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "bootstrap exited before daemon publication",
            )),
            _ => Err(io::Error::last_os_error()),
        }
    }
}

pub(super) fn cutoff_at(deadline: Instant) -> u64 {
    // SAFETY: the Windows tick clock is shared and has no preconditions.
    let now = unsafe { GetTickCount64() };
    let remaining = deadline.saturating_duration_since(Instant::now());
    now.saturating_add(u64::try_from(remaining.as_millis()).unwrap_or(u64::MAX))
}

pub(super) fn deadline_at(cutoff: u64) -> io::Result<Instant> {
    // SAFETY: the Windows tick clock is shared and has no preconditions.
    let now = unsafe { GetTickCount64() };
    Instant::now()
        .checked_add(Duration::from_millis(cutoff.saturating_sub(now)))
        .ok_or_else(invalid_gate)
}

fn event() -> io::Result<OwnedHandle> {
    // SAFETY: creates a private, non-inheritable manual-reset event.
    acquire(unsafe { CreateEventW(null(), 1, 0, null()) })
}

fn inheritable(handle: &OwnedHandle, access: u32) -> io::Result<OwnedHandle> {
    let mut child = std::ptr::null_mut();
    // SAFETY: the original handle outlives this call and the new one is returned here.
    let duplicated = unsafe {
        let process = GetCurrentProcess();
        DuplicateHandle(process, raw(handle), process, &mut child, access, 1, 0)
    };
    if duplicated == 0 {
        return Err(io::Error::last_os_error());
    }
    acquire(child)
}

fn raw(handle: &OwnedHandle) -> HANDLE {
    handle.as_raw_handle() as HANDLE
}

fn acquire(handle: HANDLE) -> io::Result<OwnedHandle> {
    if handle.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: a successful Windows call handed over a new owned handle.
    Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
}

fn from_inherited(handle: HANDLE) -> io::Result<OwnedHandle> {
    let mut flags = 0;
    // SAFETY: Windows validates the supplied handle and stores its flags here.
    if unsafe { GetHandleInformation(handle, &mut flags) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if flags & HANDLE_FLAG_INHERIT == 0 {
        return Err(invalid_gate());
    }
    // SAFETY: the validated inherited handle remains live in this daemon.
    if unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    acquire(handle)
}

fn set_event(handle: &OwnedHandle) -> io::Result<()> {
    // SAFETY: the event is retained and has EVENT_MODIFY_STATE.
    if unsafe { SetEvent(raw(handle)) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn invalid_gate() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "invalid bootstrap daemon publication gate",
    )
}
