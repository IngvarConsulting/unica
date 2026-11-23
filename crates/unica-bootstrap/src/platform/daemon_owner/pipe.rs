use super::{invalid, peer::PipeSecurity};
use crate::platform::pending_daemon::check_deadline;
use serde::{de::DeserializeOwned, Serialize};
use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::ptr::{null, null_mut};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{
    ERROR_FILE_NOT_FOUND, ERROR_NO_DATA, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED,
    ERROR_PIPE_LISTENING, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0,
    WAIT_TIMEOUT,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, WriteFile, FILE_FLAG_FIRST_PIPE_INSTANCE, OPEN_EXISTING,
    PIPE_ACCESS_DUPLEX, SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT,
};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, GetNamedPipeClientProcessId,
    GetNamedPipeServerProcessId, SetNamedPipeHandleState, PIPE_NOWAIT, PIPE_READMODE_MESSAGE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_MESSAGE,
};
use windows_sys::Win32::System::Threading::WaitForSingleObject;

// resource-limit: bootstrap.daemon-owner-control-frame-bytes
// class: format
// unit: bytes
// scope: frame
// value: 1024
// resource: Размер одного сообщения закрытого управляющего протокола владельца.
// status: unproven
// origin: протокол частного запуска Windows-демона
// problem: Чтение полного управляющего сообщения без неограниченного буфера.
// on-reach: управляющий запрос отклоняется, передача владения не подтверждается.
const FRAME_BYTES: usize = 1024;
// resource-limit-excluded: период опроса канала, отдельный бюджет вызова не задаёт
const CONNECT_POLL: Duration = Duration::from_millis(32);
// resource-limit-excluded: период опроса сообщения, конечный срок приходит от вызова
const MESSAGE_POLL: Duration = Duration::from_millis(4);

/// A local, message-mode control pipe. PIPE_NOWAIT makes each kernel call a
/// bounded poll: no OVERLAPPED pointer or pipe buffer can outlive a cancelled
/// request. Only bootstrap/frontend startup control uses this pipe.
pub(super) struct Pipe(OwnedHandle);

impl Pipe {
    pub(super) fn listen(address: &str) -> io::Result<Self> {
        let name = pipe_name(address);
        let security = PipeSecurity::current_user()?;
        let attributes = security.attributes();
        // SAFETY: the pipe name is local and NUL-terminated; no handle is inherited.
        Self::acquire(unsafe {
            CreateNamedPipeW(
                name.as_ptr(),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_MESSAGE
                    | PIPE_READMODE_MESSAGE
                    | PIPE_NOWAIT
                    | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                FRAME_BYTES as u32,
                FRAME_BYTES as u32,
                0,
                &attributes,
            )
        })
    }

    fn acquire(handle: HANDLE) -> io::Result<Self> {
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: a successful call transfers ownership of this handle.
        Ok(Self(unsafe { OwnedHandle::from_raw_handle(handle) }))
    }

    fn raw(&self) -> HANDLE {
        self.0.as_raw_handle() as HANDLE
    }

    pub(super) fn connect(address: &str, deadline: Instant) -> io::Result<Self> {
        let name = pipe_name(address);
        let pipe = loop {
            check_deadline(deadline)?;
            // SAFETY: the local pipe address is validated and NUL-terminated.
            let acquired = Self::acquire(unsafe {
                CreateFileW(
                    name.as_ptr(),
                    GENERIC_READ | GENERIC_WRITE,
                    0,
                    null(),
                    OPEN_EXISTING,
                    SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
                    null_mut(),
                )
            });
            match acquired {
                Ok(pipe) => break pipe,
                Err(error) if has_code(&error, ERROR_PIPE_BUSY) => {
                    wait(Some(deadline), None, MESSAGE_POLL)?;
                }
                Err(error) if has_code(&error, ERROR_FILE_NOT_FOUND) => {
                    return Err(io::Error::new(
                        io::ErrorKind::NotFound,
                        "run the approved Unica daemon owner for this installation and state",
                    ));
                }
                Err(error) => return Err(error),
            }
        };
        if unsafe {
            SetNamedPipeHandleState(
                pipe.raw(),
                &(PIPE_READMODE_MESSAGE | PIPE_NOWAIT),
                null(),
                null(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(pipe)
    }

    pub(super) fn server_pid(&self) -> io::Result<u32> {
        let mut pid = 0;
        // SAFETY: the pipe is live and PID storage is writable.
        if unsafe { GetNamedPipeServerProcessId(self.raw(), &mut pid) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(pid)
    }

    pub(super) fn client_pid(&self) -> io::Result<u32> {
        let mut pid = 0;
        // SAFETY: the connected handle and PID storage outlive the call.
        if unsafe { GetNamedPipeClientProcessId(self.raw(), &mut pid) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(pid)
    }

    pub(super) fn accept(&self, stop: Option<&OwnedHandle>) -> io::Result<()> {
        loop {
            check(None, stop)?;
            // SAFETY: this retained listener uses nonblocking synchronous mode.
            if unsafe { ConnectNamedPipe(self.raw(), null_mut()) } == 0 {
                let error = io::Error::last_os_error();
                if has_code(&error, ERROR_PIPE_CONNECTED) {
                    return Ok(());
                }
                if has_code(&error, ERROR_NO_DATA) {
                    // A client connected and closed before we saw it.
                    self.disconnect();
                } else if !has_code(&error, ERROR_PIPE_LISTENING) {
                    return Err(error);
                }
            } else if self.client_pid().is_ok() {
                return Ok(());
            }
            wait(None, stop, CONNECT_POLL)?;
        }
    }

    pub(super) fn disconnect(&self) {
        // SAFETY: synchronous nonblocking polling keeps no outstanding operation.
        unsafe { DisconnectNamedPipe(self.raw()) };
    }

    pub(super) fn read<T: DeserializeOwned>(
        &self,
        deadline: Option<Instant>,
        caller: Option<&OwnedHandle>,
    ) -> io::Result<T> {
        let mut bytes = [0u8; FRAME_BYTES];
        loop {
            check(deadline, caller)?;
            let mut count = 0;
            // SAFETY: buffer and count are valid for this synchronous call.
            if unsafe {
                ReadFile(
                    self.raw(),
                    bytes.as_mut_ptr(),
                    FRAME_BYTES as u32,
                    &mut count,
                    null_mut(),
                )
            } != 0
            {
                return serde_json::from_slice(&bytes[..count as usize]).map_err(io::Error::other);
            }
            let error = io::Error::last_os_error();
            if !has_code(&error, ERROR_NO_DATA) {
                return Err(error);
            }
            wait(deadline, caller, MESSAGE_POLL)?;
        }
    }

    pub(super) fn write(
        &self,
        value: &impl Serialize,
        deadline: Option<Instant>,
        caller: Option<&OwnedHandle>,
    ) -> io::Result<()> {
        let bytes = serde_json::to_vec(value).map_err(io::Error::other)?;
        if bytes.len() > FRAME_BYTES {
            return Err(invalid("oversized daemon control frame"));
        }
        loop {
            check(deadline, caller)?;
            let mut count = 0;
            // SAFETY: buffer and count are valid for this synchronous call.
            if unsafe {
                WriteFile(
                    self.raw(),
                    bytes.as_ptr(),
                    bytes.len() as u32,
                    &mut count,
                    null_mut(),
                )
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
            if count as usize == bytes.len() {
                return Ok(());
            }
            if count != 0 {
                return Err(invalid("partial daemon control message"));
            }
            // A nonblocking message pipe writes nothing when its buffer is full.
            wait(deadline, caller, MESSAGE_POLL)?;
        }
    }
}

fn has_code(error: &io::Error, code: u32) -> bool {
    error.raw_os_error() == Some(code as i32)
}

fn check(deadline: Option<Instant>, caller: Option<&OwnedHandle>) -> io::Result<()> {
    if let Some(deadline) = deadline {
        check_deadline(deadline)?;
    }
    if let Some(caller) = caller {
        check_frontend(caller, 0)?;
    }
    Ok(())
}

fn wait(
    deadline: Option<Instant>,
    caller: Option<&OwnedHandle>,
    interval: Duration,
) -> io::Result<()> {
    check(deadline, caller)?;
    let interval = deadline.map_or(interval, |at| {
        interval.min(at.saturating_duration_since(Instant::now()))
    });
    if let Some(caller) = caller {
        check_frontend(caller, interval.as_millis() as u32)?;
    } else {
        std::thread::sleep(interval);
    }
    Ok(())
}

fn check_frontend(frontend: &OwnedHandle, millis: u32) -> io::Result<()> {
    // SAFETY: the exact frontend process handle is retained for this wait.
    match unsafe { WaitForSingleObject(frontend.as_raw_handle() as HANDLE, millis) } {
        WAIT_TIMEOUT => Ok(()),
        WAIT_OBJECT_0 => Err(io::Error::new(io::ErrorKind::BrokenPipe, "frontend exited")),
        _ => Err(io::Error::last_os_error()),
    }
}

fn pipe_name(address: &str) -> Vec<u16> {
    format!(r"\\.\pipe\unica-daemon-owner-v1-{address}")
        .encode_utf16()
        .chain([0])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::System::Threading::{CreateEventW, SetEvent};

    fn event() -> OwnedHandle {
        // SAFETY: fresh non-inherited test-only event, owned until the test returns.
        let raw = unsafe { CreateEventW(null(), 1, 0, null()) };
        assert!(!raw.is_null());
        unsafe { OwnedHandle::from_raw_handle(raw) }
    }

    /// rule: INV.PLATFORM.PROCESS-TREE-LIFECYCLE
    #[test]
    fn pending_connect_read_and_write_stop_with_frontend() {
        let address = format!("pipe-test-{}", uuid::Uuid::new_v4());
        let server = Pipe::listen(&address).unwrap();
        let caller = event();
        // Exercise the kernel's pending listen state first.
        let connected = unsafe { ConnectNamedPipe(server.raw(), null_mut()) };
        assert_eq!(connected, 0);
        assert!(has_code(&io::Error::last_os_error(), ERROR_PIPE_LISTENING));
        let deadline = Instant::now() + Duration::from_secs(5);
        let client = Pipe::connect(&address, deadline).unwrap();
        server.accept(Some(&caller)).unwrap();

        let short_deadline = Instant::now() + Duration::from_millis(30);
        assert_eq!(
            server
                .read::<bool>(Some(short_deadline), Some(&caller))
                .unwrap_err()
                .kind(),
            io::ErrorKind::TimedOut,
        );

        // Fill the receiver's buffer without ever asking the asynchronous
        // scheduler to race the writer or leaving a queued write request.
        let bytes = [0u8; FRAME_BYTES];
        let mut blocked = false;
        for _ in 0..32 {
            let mut count = 0;
            // SAFETY: this is a synchronous nonblocking test write.
            let accepted = unsafe {
                WriteFile(
                    server.raw(),
                    bytes.as_ptr(),
                    FRAME_BYTES as u32,
                    &mut count,
                    null_mut(),
                )
            };
            assert_ne!(accepted, 0);
            if count == 0 {
                blocked = true;
                break;
            }
            assert_eq!(count as usize, FRAME_BYTES);
        }
        assert!(blocked);
        let short_deadline = Instant::now() + Duration::from_millis(30);
        let payload = "x".repeat(FRAME_BYTES - 2);
        assert_eq!(
            server
                .write(&payload, Some(short_deadline), Some(&caller))
                .unwrap_err()
                .kind(),
            io::ErrorKind::TimedOut,
        );
        // The same wait handle is used by production for frontend death.
        assert_ne!(unsafe { SetEvent(caller.as_raw_handle() as HANDLE) }, 0);
        assert_eq!(
            server
                .read::<bool>(Some(deadline), Some(&caller))
                .unwrap_err()
                .kind(),
            io::ErrorKind::BrokenPipe,
        );
        assert_eq!(
            server
                .write(&payload, Some(deadline), Some(&caller))
                .unwrap_err()
                .kind(),
            io::ErrorKind::BrokenPipe,
        );
        drop(client);
        server.disconnect();
        assert_eq!(
            server.accept(Some(&caller)).unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
    }
}
