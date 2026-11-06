// Linux observes opens of the actual private inode, including direct std::fs
// calls and ignored errors. Provider reads use its already-open descriptor;
// fixture reads and snapshots stay outside this window. Metadata probes do not
// open the file. Any event, including watch loss/overflow, fails closed.
#[cfg(target_os = "linux")]
pub(crate) struct PrivateIndexReadBoundary(std::fs::File);
#[cfg(target_os = "linux")]
impl PrivateIndexReadBoundary {
    pub(crate) fn new(path: &std::path::Path) -> Self {
        use std::os::{fd::FromRawFd, unix::ffi::OsStrExt};
        let path = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        // SAFETY: inotify_init1 returns a fresh owned descriptor or -1.
        let fd = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
        assert!(fd >= 0, "{}", std::io::Error::last_os_error());
        // SAFETY: fd is newly owned and is transferred exactly once.
        let file = unsafe { std::fs::File::from_raw_fd(fd) };
        // SAFETY: path is NUL-terminated and remains live through the call.
        let watch = unsafe { libc::inotify_add_watch(fd, path.as_ptr(), libc::IN_OPEN) };
        assert!(watch >= 0, "{}", std::io::Error::last_os_error());
        Self(file)
    }
    pub(crate) fn assert_no_consumer_open(mut self) {
        use std::io::Read;
        let mut events = [0_u8; 4096];
        match self.0.read(&mut events) {
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            other => panic!("consumer opened private index or read boundary was lost: {other:?}"),
        }
    }
}

#[cfg(not(target_os = "linux"))]
pub(crate) struct PrivateIndexReadBoundary;
#[cfg(not(target_os = "linux"))]
impl PrivateIndexReadBoundary {
    pub(crate) fn new(_: &std::path::Path) -> Self {
        Self
    }
    pub(crate) fn assert_no_consumer_open(self) {}
}
