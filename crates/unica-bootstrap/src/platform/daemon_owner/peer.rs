use super::install::FilePin;
use std::ffi::{c_void, OsString};
use std::io;
use std::os::windows::ffi::OsStringExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::PathBuf;
use std::ptr::{null_mut, read_unaligned};
use windows_sys::Win32::Foundation::{
    LocalFree, FILETIME, HANDLE, INVALID_HANDLE_VALUE, WAIT_TIMEOUT,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, TokenIntegrityLevel, TokenUser, PSECURITY_DESCRIPTOR, PSID,
    SECURITY_ATTRIBUTES, TOKEN_MANDATORY_LABEL, TOKEN_QUERY, TOKEN_USER,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetProcessTimes, OpenProcess, OpenProcessToken, QueryFullProcessImageNameW,
    WaitForSingleObject, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
};

#[derive(PartialEq, Eq)]
pub(super) struct Principal {
    user: String,
    integrity: String,
}

impl Principal {
    pub(super) fn current() -> io::Result<Self> {
        // SAFETY: the pseudo-handle is valid for the current process.
        Self::from_process(unsafe { GetCurrentProcess() })
    }

    fn from_process(process: HANDLE) -> io::Result<Self> {
        let mut token = null_mut();
        // SAFETY: the live process is queryable and token is writable.
        if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let token = own(token)?;
        let user = token_bytes(&token, TokenUser)?;
        let integrity = token_bytes(&token, TokenIntegrityLevel)?;
        // SAFETY: these buffers hold the requested Windows structures and their SIDs.
        unsafe {
            Ok(Self {
                user: sid_string(read_unaligned(user.as_ptr().cast::<TOKEN_USER>()).User.Sid)?,
                integrity: sid_string(
                    read_unaligned(integrity.as_ptr().cast::<TOKEN_MANDATORY_LABEL>())
                        .Label
                        .Sid,
                )?,
            })
        }
    }
}

pub(super) fn verified_process(pid: u32, image: &FilePin) -> io::Result<OwnedHandle> {
    // SAFETY: only query and wait rights are requested. The retained handle
    // holds the exact process identity through all subsequent lifetime checks.
    let process = own(unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            0,
            pid,
        )
    })?;
    let raw = process.as_raw_handle();
    let mut path = vec![0u16; 32_768];
    let mut count = path.len() as u32;
    // SAFETY: the output buffer and count remain valid for the call.
    if unsafe { QueryFullProcessImageNameW(raw, 0, path.as_mut_ptr(), &mut count) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let path = PathBuf::from(OsString::from_wide(&path[..count as usize]));
    if !image.matches(&path)?
        || Principal::from_process(raw)? != Principal::current()?
        // SAFETY: the process handle has SYNCHRONIZE access.
        || unsafe { WaitForSingleObject(raw, 0) } != WAIT_TIMEOUT
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "daemon owner peer does not match the installed executable and Windows principal",
        ));
    }
    Ok(process)
}

/// The core is cached independently of plugin installs. Its exact, live
/// launcher must also be the approved bootstrap from this package.
pub(super) fn verified_frontend(
    pid: u32,
    core: &FilePin,
    bootstrap: &FilePin,
) -> io::Result<(OwnedHandle, OwnedHandle)> {
    let frontend = verified_process(pid, core)?;
    let parent = verified_process(parent_pid(pid)?, bootstrap)?;
    if creation_time(&parent)? > creation_time(&frontend)? || !is_alive(&frontend) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "daemon frontend's parent is not its live bootstrap",
        ));
    }
    Ok((frontend, parent))
}

pub(super) fn is_alive(process: &OwnedHandle) -> bool {
    // SAFETY: the retained process is queryable and waitable.
    (unsafe { WaitForSingleObject(process.as_raw_handle(), 0) }) == WAIT_TIMEOUT
}

fn parent_pid(pid: u32) -> io::Result<u32> {
    // SAFETY: the kernel returns a private handle to the process snapshot.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let snapshot = own(snapshot)?;
    // SAFETY: the Win32 input is initialized and carries its struct size.
    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
    let mut found = unsafe { Process32FirstW(snapshot.as_raw_handle(), &mut entry) };
    while found != 0 {
        if entry.th32ProcessID == pid {
            return Ok(entry.th32ParentProcessID);
        }
        // SAFETY: the snapshot and record remain valid.
        found = unsafe { Process32NextW(snapshot.as_raw_handle(), &mut entry) };
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "daemon frontend is absent from the process snapshot",
    ))
}

fn creation_time(process: &OwnedHandle) -> io::Result<u64> {
    // SAFETY: Windows fills the four distinct FILETIME outputs.
    let [mut created, mut exited, mut kernel, mut user]: [FILETIME; 4] =
        unsafe { std::mem::zeroed() };
    if unsafe {
        GetProcessTimes(
            process.as_raw_handle(),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok((u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))
}

pub(super) struct PipeSecurity(PSECURITY_DESCRIPTOR);

impl PipeSecurity {
    pub(super) fn current_user() -> io::Result<Self> {
        let principal = Principal::current()?;
        let descriptor: Vec<u16> = format!("D:P(A;;GA;;;{})", principal.user)
            .encode_utf16()
            .chain([0])
            .collect();
        let mut converted = null_mut();
        // SAFETY: descriptor is NUL-terminated, output receives owned memory.
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                descriptor.as_ptr(),
                SDDL_REVISION_1,
                &mut converted,
                null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(converted))
    }

    pub(super) fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.0,
            bInheritHandle: 0,
        }
    }
}

impl Drop for PipeSecurity {
    fn drop(&mut self) {
        // SAFETY: the conversion API allocates this pointer with LocalAlloc.
        unsafe { LocalFree(self.0.cast::<c_void>()) };
    }
}

fn own(handle: HANDLE) -> io::Result<OwnedHandle> {
    if handle.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the successful Win32 call transferred ownership.
    Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
}

fn token_bytes(token: &OwnedHandle, kind: i32) -> io::Result<Vec<u8>> {
    let mut count = 0;
    // SAFETY: with no supplied buffer this call returns its required size.
    unsafe { GetTokenInformation(token.as_raw_handle(), kind, null_mut(), 0, &mut count) };
    if count == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut bytes = vec![0; count as usize];
    // SAFETY: the heap buffer remains valid during and after the read.
    if unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            kind,
            bytes.as_mut_ptr().cast(),
            count,
            &mut count,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(bytes)
}

unsafe fn sid_string(sid: PSID) -> io::Result<String> {
    let mut value = null_mut();
    // SAFETY: the caller retains the Windows-owned SID buffer.
    if unsafe { ConvertSidToStringSidW(sid, &mut value) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the conversion returns an allocated NUL-terminated wide string.
    let length = (0..)
        .take_while(|offset| unsafe { *value.add(*offset) } != 0)
        .count();
    let text = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(value, length) });
    // SAFETY: ConvertSidToStringSidW uses LocalAlloc.
    unsafe { LocalFree(value.cast()) };
    Ok(text)
}

#[cfg(test)]
mod medium_tests {
    use super::*;
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::System::Threading::{GetProcessId, CREATE_NO_WINDOW, CREATE_SUSPENDED};

    #[test]
    fn a_shared_core_proves_which_installation_launched_it() {
        let system = PathBuf::from(std::env::var_os("SystemRoot").unwrap());
        let core = system.join("System32").join("cmd.exe");
        let bootstrap = std::env::current_exe().unwrap();
        let temporary = tempfile::tempdir().unwrap();
        let other_installation = temporary.path().join("unica-bootstrap.exe");
        std::fs::copy(&bootstrap, &other_installation).unwrap();

        let mut frontend = std::process::Command::new(&core)
            .creation_flags(CREATE_NO_WINDOW | CREATE_SUSPENDED)
            .spawn()
            .unwrap();
        let core = FilePin::file(&core).unwrap();
        let result = verified_frontend(frontend.id(), &core, &FilePin::file(&bootstrap).unwrap());
        let foreign = verified_frontend(
            frontend.id(),
            &core,
            &FilePin::file(&other_installation).unwrap(),
        );
        let _ = frontend.kill();
        let _ = frontend.wait();
        let (_, approved_launcher) = result.unwrap();
        // SAFETY: the retained queried launcher handle remains open.
        assert_eq!(
            unsafe { GetProcessId(approved_launcher.as_raw_handle()) },
            std::process::id()
        );
        assert!(matches!(foreign, Err(error) if error.kind() == io::ErrorKind::PermissionDenied));
    }
}
