//! Passive test evidence from the retained process handle, before cleanup.

use std::process::Child;

#[derive(Clone, Copy, Debug, serde::Serialize)]
pub(crate) struct StartupChildObservation {
    pub(crate) supported: bool,
    pub(crate) signaled: Option<bool>,
    pub(crate) exit_code: Option<u32>,
    pub(crate) exit_filetime_100ns: Option<u64>,
    pub(crate) error: Option<(&'static str, Option<i32>)>,
}

impl StartupChildObservation {
    fn empty(supported: bool) -> Self {
        Self {
            supported,
            signaled: None,
            exit_code: None,
            exit_filetime_100ns: None,
            error: None,
        }
    }
}

#[cfg(not(windows))]
pub(super) fn observe(_child: &Child) -> StartupChildObservation {
    // Do not introduce a second reaper on Unix just for Windows diagnostics.
    StartupChildObservation::empty(false)
}

#[cfg(windows)]
pub(super) fn observe(child: &Child) -> StartupChildObservation {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::{FILETIME, WAIT_OBJECT_0, WAIT_TIMEOUT};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, GetProcessTimes, WaitForSingleObject,
    };

    let mut observation = StartupChildObservation::empty(true);
    let handle = child.as_raw_handle() as _;
    // SAFETY: Child owns this live handle for the whole borrow. Zero timeout
    // observes it without closing, reaping, terminating or reopening by PID.
    match unsafe { WaitForSingleObject(handle, 0) } {
        WAIT_TIMEOUT => observation.signaled = Some(false),
        WAIT_OBJECT_0 => {
            observation.signaled = Some(true);
            let mut code = 0;
            // SAFETY: the borrowed handle and output pointer remain valid.
            if unsafe { GetExitCodeProcess(handle, &mut code) } == 0 {
                observation.error = Some((
                    "GetExitCodeProcess",
                    std::io::Error::last_os_error().raw_os_error(),
                ));
                return observation;
            }
            observation.exit_code = Some(code);
            let zero = FILETIME {
                dwLowDateTime: 0,
                dwHighDateTime: 0,
            };
            let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
            // SAFETY: all four outputs and the borrowed process handle are valid.
            if unsafe { GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) }
                == 0
            {
                observation.error = Some((
                    "GetProcessTimes",
                    std::io::Error::last_os_error().raw_os_error(),
                ));
            } else {
                observation.exit_filetime_100ns = Some(
                    (u64::from(exited.dwHighDateTime) << 32) | u64::from(exited.dwLowDateTime),
                );
            }
        }
        _ => {
            observation.error = Some((
                "WaitForSingleObject",
                std::io::Error::last_os_error().raw_os_error(),
            ))
        }
    }
    observation
}
