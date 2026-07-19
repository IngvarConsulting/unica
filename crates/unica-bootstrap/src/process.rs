use std::path::Path;
use std::process::Command;

use crate::error::{BootstrapError, Result};

#[cfg(unix)]
pub fn launch_runtime(entrypoint: &Path, args: &[String]) -> Result<i32> {
    use std::os::unix::process::CommandExt;

    let error = Command::new(entrypoint).args(args).exec();
    Err(BootstrapError::new(format!(
        "failed to exec Unica runtime {}: {error}",
        entrypoint.display()
    )))
}

#[cfg(windows)]
pub fn launch_runtime(entrypoint: &Path, args: &[String]) -> Result<i32> {
    use std::mem::size_of;
    use std::os::windows::io::AsRawHandle;
    use std::ptr;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    let mut child = Command::new(entrypoint)
        .args(args)
        .spawn()
        .map_err(|error| {
            BootstrapError::new(format!(
                "failed to start Unica runtime {}: {error}",
                entrypoint.display()
            ))
        })?;

    let job = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
    if job.is_null() {
        let _ = child.kill();
        let _ = child.wait();
        return Err(BootstrapError::new(
            "failed to create Windows Job Object for Unica runtime",
        ));
    }
    let mut information: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    information.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    let configured = unsafe {
        SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &information as *const _ as *const _,
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    };
    let assigned = unsafe { AssignProcessToJobObject(job, child.as_raw_handle() as HANDLE) };
    if configured == 0 || assigned == 0 {
        unsafe { CloseHandle(job) };
        let _ = child.kill();
        let _ = child.wait();
        return Err(BootstrapError::new(
            "failed to supervise Unica runtime with a Windows Job Object",
        ));
    }

    let status = child
        .wait()
        .map_err(|error| BootstrapError::new(format!("failed to wait for Unica runtime: {error}")));
    unsafe { CloseHandle(job) };
    let status = status?;
    Ok(status.code().unwrap_or(1))
}

#[cfg(not(any(unix, windows)))]
pub fn launch_runtime(entrypoint: &Path, _args: &[String]) -> Result<i32> {
    Err(BootstrapError::new(format!(
        "runtime launch is unsupported on this platform: {}",
        entrypoint.display()
    )))
}
