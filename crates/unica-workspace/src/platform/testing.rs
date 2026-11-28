use std::io;
use std::path::{Path, PathBuf};
#[cfg(all(any(test, feature = "test-support"), unix))]
use std::time::Duration;

#[cfg(windows)]
use super::filesystem::open_directory_nofollow;
#[cfg(any(test, feature = "test-support"))]
pub use super::filesystem::remove_dir_symlink_for_test;
pub use super::filesystem::{create_dir_symlink_for_test, create_file_symlink_for_test};
pub use super::filesystem::{extended_length_spelling_for_test, other_letter_case_for_test};
use super::filesystem::{file_identity, hard_link_count};

pub fn normalize_path_text_for_test(value: &str) -> String {
    value.replace('\\', "/")
}

pub fn path_text_for_test(path: &Path) -> String {
    normalize_path_text_for_test(&path.display().to_string())
}

/// Hold a known regular fixture file open without delete sharing. A retained
/// publisher can still read it, but Windows must reject its rename for recovery.
#[cfg(all(feature = "test-support", windows))]
pub(crate) fn hold_regular_file_without_delete_sharing_for_test(
    path: &Path,
) -> io::Result<Option<std::fs::File>> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };

    std::fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map(Some)
}

#[cfg(all(feature = "test-support", not(windows)))]
pub(crate) fn hold_regular_file_without_delete_sharing_for_test(
    _path: &Path,
) -> io::Result<Option<std::fs::File>> {
    Ok(None)
}

#[cfg(windows)]
pub fn case_distinct_provider_paths_for_test(_parent: &Path) -> Option<(PathBuf, PathBuf)> {
    None
}

#[cfg(not(windows))]
pub fn case_distinct_provider_paths_for_test(parent: &Path) -> Option<(PathBuf, PathBuf)> {
    Some((
        parent.join("CaseSensitiveWorkspace"),
        parent.join("casesensitiveworkspace"),
    ))
}

#[cfg(all(unix, not(target_vendor = "apple")))]
pub fn distinct_non_utf8_provider_paths_for_test(parent: &Path) -> Option<(PathBuf, PathBuf)> {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    Some((
        parent.join(OsString::from_vec(b"provider-\x80".to_vec())),
        parent.join(OsString::from_vec(b"provider-\x81".to_vec())),
    ))
}

#[cfg(not(all(unix, not(target_vendor = "apple"))))]
pub fn distinct_non_utf8_provider_paths_for_test(_parent: &Path) -> Option<(PathBuf, PathBuf)> {
    None
}

#[cfg(unix)]
pub fn non_utf8_relative_path_for_test() -> Option<PathBuf> {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    Some(PathBuf::from(OsString::from_vec(
        b"Module-\xff.bsl".to_vec(),
    )))
}

#[cfg(not(unix))]
pub fn non_utf8_relative_path_for_test() -> Option<PathBuf> {
    None
}

#[cfg(windows)]
pub fn windows_extended_length_path_for_test(path: &Path) -> Option<PathBuf> {
    Some(PathBuf::from(format!(r"\\?\{}", path.display())))
}

#[cfg(not(windows))]
pub fn windows_extended_length_path_for_test(_path: &Path) -> Option<PathBuf> {
    None
}

#[cfg(unix)]
pub fn can_rename_parent_with_retained_cleanup_child_for_test() -> bool {
    true
}

#[cfg(not(unix))]
pub fn can_rename_parent_with_retained_cleanup_child_for_test() -> bool {
    false
}

#[cfg(unix)]
pub fn can_swap_named_child_behind_retained_handle_for_test() -> bool {
    true
}

#[cfg(not(unix))]
pub fn can_swap_named_child_behind_retained_handle_for_test() -> bool {
    false
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetainedDirectoryReplacementOutcome {
    Replaced,
    PreventedByRetainedHandle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetainedRegularFileRelocationOutcome {
    Relocated,
    PreventedByRetainedHandle,
}

pub fn attempt_retained_directory_replacement_for_test(
    named: &Path,
    displaced: &Path,
) -> io::Result<RetainedDirectoryReplacementOutcome> {
    match std::fs::rename(named, displaced) {
        Ok(()) => Ok(RetainedDirectoryReplacementOutcome::Replaced),
        Err(error) if windows_retained_name_relocation_was_prevented(&error) => {
            Ok(RetainedDirectoryReplacementOutcome::PreventedByRetainedHandle)
        }
        Err(error) => Err(error),
    }
}

pub fn attempt_retained_regular_file_relocation_for_test(
    named: &Path,
    relocated: &Path,
) -> io::Result<RetainedRegularFileRelocationOutcome> {
    match std::fs::rename(named, relocated) {
        Ok(()) => Ok(RetainedRegularFileRelocationOutcome::Relocated),
        Err(error) if windows_retained_name_relocation_was_prevented(&error) => {
            Ok(RetainedRegularFileRelocationOutcome::PreventedByRetainedHandle)
        }
        Err(error) => Err(error),
    }
}

#[cfg(windows)]
fn windows_retained_name_relocation_was_prevented(error: &io::Error) -> bool {
    error.raw_os_error() == Some(windows_sys::Win32::Foundation::ERROR_ACCESS_DENIED as i32)
        || super::filesystem::is_file_sharing_violation(error)
}

#[cfg(not(windows))]
fn windows_retained_name_relocation_was_prevented(_error: &io::Error) -> bool {
    false
}

#[cfg(windows)]
pub fn sharing_violation_error_cases_for_test() -> [(io::Error, bool); 2] {
    use windows_sys::Win32::Foundation::{ERROR_LOCK_VIOLATION, ERROR_SHARING_VIOLATION};

    [
        (
            io::Error::from_raw_os_error(ERROR_SHARING_VIOLATION as i32),
            true,
        ),
        (
            io::Error::from_raw_os_error(ERROR_LOCK_VIOLATION as i32),
            false,
        ),
    ]
}

#[cfg(not(windows))]
pub fn sharing_violation_error_cases_for_test() -> [(io::Error, bool); 2] {
    // The same numeric errno values must not acquire Windows semantics here.
    [
        (io::Error::from_raw_os_error(32), false),
        (io::Error::from_raw_os_error(33), false),
    ]
}

/// Соединение каталогов — вторая разновидность точки повторного разбора, и
/// единственная, которую Windows создаёт без привилегии ссылок. Так проверка,
/// чей предмет — точка разбора, а не ссылка, исполняется и там, где
/// привилегии нет.
///
/// Успех определяется состоянием файловой системы, а не кодом возврата:
/// `mklink /J` с отсутствующей целью отдаёт ноль и оставляет висячее
/// соединение — атрибут точки стоит, а цели нет. Код возврата и вывод идут
/// только уликой в сообщении об ошибке; вывод берётся с потерями и не
/// разбирается, потому что приходит в кодовой странице консоли.
///
/// Буфер точки через `FSCTL_SET_REPARSE_POINT` здесь не собирается: сборка
/// в `unsafe` рискует тонкой ошибкой, из-за которой тест замигает вместо
/// того, чтобы упасть, — а это ровно то, что такие проверки и лечат.
#[cfg(all(any(test, feature = "test-support"), windows))]
pub fn create_directory_junction_for_test(target: &Path, link: &Path) -> io::Result<()> {
    use std::os::windows::fs::MetadataExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

    let done = std::process::Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .output()?;
    let evidence = || {
        format!(
            "mklink /J {} {} exited with {:?}: {}{}",
            link.display(),
            target.display(),
            done.status.code(),
            String::from_utf8_lossy(&done.stdout).trim(),
            String::from_utf8_lossy(&done.stderr).trim(),
        )
    };
    let metadata = std::fs::symlink_metadata(link).map_err(|error| {
        io::Error::other(format!("{}: no junction at the link; {error}", evidence()))
    })?;
    if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0 {
        return Err(io::Error::other(format!(
            "{}: the link carries no reparse point",
            evidence()
        )));
    }
    let (resolved, wanted) = (std::fs::canonicalize(link), std::fs::canonicalize(target));
    match (resolved, wanted) {
        (Ok(resolved), Ok(wanted)) if resolved == wanted => Ok(()),
        (resolved, wanted) => Err(io::Error::other(format!(
            "{}: the junction does not lead to the target; link resolves to {resolved:?}, \
             target resolves to {wanted:?}",
            evidence()
        ))),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileLinkFixtureOutcome {
    Created,
    Unsupported,
    WindowsPrivilegeUnavailable,
}

pub fn create_file_link_fixture_for_test(
    source: impl AsRef<Path>,
    target: impl AsRef<Path>,
) -> io::Result<FileLinkFixtureOutcome> {
    classify_file_link_fixture_result(create_file_symlink_for_test(source, target))
}

pub fn create_directory_link_fixture_for_test(
    source: impl AsRef<Path>,
    target: impl AsRef<Path>,
) -> io::Result<FileLinkFixtureOutcome> {
    classify_file_link_fixture_result(create_dir_symlink_for_test(source, target))
}

/// Creates a directory redirection fixture without requiring Windows symlink privileges.
pub fn create_directory_link_or_junction_for_test(target: &Path, link: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        let target: PathBuf = target.components().collect();
        let link: PathBuf = link.components().collect();
        create_directory_junction_for_test(&target, &link)
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (target, link);
        Err(io::ErrorKind::Unsupported.into())
    }
}

pub fn file_identity_for_test(path: &Path) -> io::Result<Option<String>> {
    let file = std::fs::File::open(path)?;
    identity_for_open_path_for_test(&file)
}

pub fn path_identity_for_test(path: &Path) -> io::Result<Option<String>> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path identity fixture does not follow symbolic links",
        ));
    }
    #[cfg(windows)]
    let file = if metadata.is_dir() {
        open_directory_nofollow(path)?
    } else {
        std::fs::File::open(path)?
    };
    #[cfg(not(windows))]
    let file = std::fs::File::open(path)?;
    identity_for_open_path_for_test(&file)
}

fn identity_for_open_path_for_test(file: &std::fs::File) -> io::Result<Option<String>> {
    let identity = match file_identity(file) {
        Ok(identity) => identity,
        Err(error) if error.kind() == io::ErrorKind::Unsupported => return Ok(None),
        Err(error) => return Err(error),
    };
    let links = match hard_link_count(file) {
        Ok(links) => links,
        Err(error) if error.kind() == io::ErrorKind::Unsupported => return Ok(None),
        Err(error) => return Err(error),
    };
    Ok(Some(format!("{identity:?}; links={links}")))
}

fn classify_file_link_fixture_result(
    result: Option<io::Result<()>>,
) -> io::Result<FileLinkFixtureOutcome> {
    match result {
        Some(Ok(())) => Ok(FileLinkFixtureOutcome::Created),
        Some(Err(error)) if windows_symlink_privilege_unavailable(&error) => {
            Ok(FileLinkFixtureOutcome::WindowsPrivilegeUnavailable)
        }
        Some(Err(error)) => Err(error),
        None => Ok(FileLinkFixtureOutcome::Unsupported),
    }
}

#[cfg(windows)]
fn windows_symlink_privilege_unavailable(error: &io::Error) -> bool {
    const ERROR_PRIVILEGE_NOT_HELD: i32 = 1314;

    error.raw_os_error() == Some(ERROR_PRIVILEGE_NOT_HELD)
}

#[cfg(not(windows))]
fn windows_symlink_privilege_unavailable(_error: &io::Error) -> bool {
    false
}

#[cfg(unix)]
pub fn set_unix_mode_for_test(path: &Path, mode: u32) -> io::Result<bool> {
    use std::fs::Permissions;
    use std::os::unix::fs::PermissionsExt;

    std::fs::set_permissions(path, Permissions::from_mode(mode))?;
    Ok(true)
}

#[cfg(not(unix))]
pub fn set_unix_mode_for_test(_path: &Path, _mode: u32) -> io::Result<bool> {
    Ok(false)
}

#[cfg(unix)]
pub fn unix_mode_for_test(path: &Path) -> io::Result<Option<u32>> {
    use std::os::unix::fs::PermissionsExt;

    Ok(Some(std::fs::metadata(path)?.permissions().mode() & 0o7777))
}

#[cfg(not(unix))]
pub fn unix_mode_for_test(_path: &Path) -> io::Result<Option<u32>> {
    Ok(None)
}

/// Count descriptors (Unix) or handles (Windows) this process holds on `root`
/// and the files and directories under it. A test that owns `root` counts
/// what its own work keeps open there; whatever the test harness and
/// concurrent tests hold elsewhere is not in the count.
///
/// The count asks the operating system rather than counting the opens of the
/// filesystem facade, and a simpler counter would stop checking the subject:
/// - the bound (`INV.APP.BOUNDED-OPEN-DESCRIPTORS`) is on descriptors the
///   operating system keeps open, not on the opens the product makes;
/// - opens do not all pass the facade: portable `std::fs` opens files outside
///   `platform/`, and `cap_primitives` opens the directories of a route
///   inside it, out of sight of product code;
/// - counting retention through a wrapper on `Drop` of every held `File` or
///   `Dir` would change product types for a test and still not be "closed in
///   the operating system".
pub fn open_handles_under_for_test(root: &Path) -> io::Result<usize> {
    let root = std::fs::canonicalize(root)?;
    Ok(open_handle_paths_for_test()?
        .iter()
        .filter(|path| is_at_or_under(path, &root))
        .count())
}

#[cfg(windows)]
fn is_at_or_under(path: &Path, root: &Path) -> bool {
    // Windows paths compare without regard to letter case.
    let path = path.to_string_lossy().to_lowercase();
    let root = root.to_string_lossy().to_lowercase();
    let root = root.trim_end_matches('\\');
    path == root || path.starts_with(&format!("{root}\\"))
}

#[cfg(not(windows))]
fn is_at_or_under(path: &Path, root: &Path) -> bool {
    path.starts_with(root)
}

/// The paths of the descriptors this process holds now; a descriptor that is
/// not a file or directory is left out.
#[cfg(target_os = "linux")]
fn open_handle_paths_for_test() -> io::Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for entry in std::fs::read_dir("/proc/self/fd")? {
        // A descriptor closed while the directory is read has no link left.
        if let Ok(target) = std::fs::read_link(entry?.path()) {
            paths.push(target);
        }
    }
    Ok(paths)
}

#[cfg(target_os = "macos")]
fn open_handle_paths_for_test() -> io::Result<Vec<PathBuf>> {
    use std::os::unix::ffi::OsStrExt;

    let mut paths = Vec::new();
    for entry in std::fs::read_dir("/dev/fd")? {
        let Some(fd) = entry?
            .file_name()
            .to_str()
            .and_then(|name| name.parse().ok())
        else {
            continue;
        };
        let mut buffer = [0_u8; libc::PATH_MAX as usize];
        // SAFETY: F_GETPATH writes at most PATH_MAX bytes into `buffer`; a
        // descriptor closed meanwhile only makes the call fail.
        if unsafe { libc::fcntl(fd, libc::F_GETPATH, buffer.as_mut_ptr()) } == -1 {
            continue;
        }
        let length = buffer
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(buffer.len());
        paths.push(PathBuf::from(std::ffi::OsStr::from_bytes(
            &buffer[..length],
        )));
    }
    Ok(paths)
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
fn open_handle_paths_for_test() -> io::Result<Vec<PathBuf>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "descriptor paths are read on Linux and macOS",
    ))
}

#[cfg(windows)]
fn open_handle_paths_for_test() -> io::Result<Vec<PathBuf>> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Wdk::System::SystemInformation::NtQuerySystemInformation;
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileType, GetFinalPathNameByHandleW, FILE_NAME_NORMALIZED, FILE_TYPE_DISK,
        VOLUME_NAME_DOS,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;

    // SystemExtendedHandleInformation: every handle of every process, with
    // its owner and value; the layout is the documented
    // SYSTEM_HANDLE_TABLE_ENTRY_INFO_EX.
    const SYSTEM_EXTENDED_HANDLE_INFORMATION: i32 = 64;
    const STATUS_INFO_LENGTH_MISMATCH: i32 = 0xC000_0004_u32 as i32;
    #[repr(C)]
    struct Entry {
        object: *mut core::ffi::c_void,
        process_id: usize,
        handle_value: usize,
        granted_access: u32,
        creator_back_trace_index: u16,
        object_type_index: u16,
        handle_attributes: u32,
        reserved: u32,
    }

    let mut buffer = vec![0_usize; 1 << 16];
    let count = loop {
        let length = (buffer.len() * std::mem::size_of::<usize>()) as u32;
        let mut needed = 0_u32;
        // SAFETY: `buffer` is writable for `length` bytes and aligned for the
        // pointer-sized fields of the returned table.
        let status = unsafe {
            NtQuerySystemInformation(
                SYSTEM_EXTENDED_HANDLE_INFORMATION,
                buffer.as_mut_ptr().cast(),
                length,
                &mut needed,
            )
        };
        if status == STATUS_INFO_LENGTH_MISMATCH {
            // Handles are opened while the table is read: grow with room.
            let wanted = (needed as usize).max(length as usize) * 2;
            buffer = vec![0; wanted / std::mem::size_of::<usize>() + 1];
            continue;
        }
        if status < 0 {
            return Err(io::Error::other(format!(
                "NtQuerySystemInformation failed with status {status:#x}"
            )));
        }
        break buffer[0];
    };
    // SAFETY: on success the table is a count and a reserved word followed by
    // `count` entries, all inside `buffer`.
    let entries =
        unsafe { std::slice::from_raw_parts(buffer.as_ptr().add(2).cast::<Entry>(), count) };
    let process = unsafe { GetCurrentProcessId() } as usize;
    let mut paths = Vec::new();
    for entry in entries.iter().filter(|entry| entry.process_id == process) {
        let handle = entry.handle_value as HANDLE;
        // Only a disk file or directory has a path; asking a pipe or a
        // console for one can block. A handle closed and reused meanwhile is
        // classified by what it is now.
        // SAFETY: the value names a handle of this process or none; the call
        // only reads its type.
        if unsafe { GetFileType(handle) } != FILE_TYPE_DISK {
            continue;
        }
        let mut name = vec![0_u16; 1024];
        // SAFETY: `name` is writable for its length in UTF-16 units.
        let length = unsafe {
            GetFinalPathNameByHandleW(
                handle,
                name.as_mut_ptr(),
                name.len() as u32,
                FILE_NAME_NORMALIZED | VOLUME_NAME_DOS,
            )
        } as usize;
        if length == 0 || length >= name.len() {
            continue;
        }
        paths.push(PathBuf::from(std::ffi::OsString::from_wide(
            &name[..length],
        )));
    }
    Ok(paths)
}

#[cfg(all(any(test, feature = "test-support"), unix))]
pub struct ForkedInheritedDescriptorForTest {
    pid: libc::pid_t,
    release_writer: Option<std::os::fd::OwnedFd>,
}

#[cfg(all(any(test, feature = "test-support"), unix))]
impl ForkedInheritedDescriptorForTest {
    pub fn start() -> io::Result<Self> {
        use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
        use std::time::Instant;

        fn pipe() -> io::Result<(OwnedFd, OwnedFd)> {
            let mut descriptors = [-1; 2];
            // SAFETY: `pipe` initializes both descriptors on success; each is
            // immediately owned by an `OwnedFd` in the parent.
            if unsafe { libc::pipe(descriptors.as_mut_ptr()) } == -1 {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: successful `pipe` returned distinct owned descriptors.
            Ok(unsafe {
                (
                    OwnedFd::from_raw_fd(descriptors[0]),
                    OwnedFd::from_raw_fd(descriptors[1]),
                )
            })
        }

        let (ready_reader, ready_writer) = pipe()?;
        let (release_reader, release_writer) = pipe()?;
        // SAFETY: the child executes only async-signal-safe syscalls and then
        // `_exit`; it never touches Rust state after forking the test harness.
        let pid = unsafe { libc::fork() };
        if pid == -1 {
            return Err(io::Error::last_os_error());
        }
        if pid == 0 {
            // SAFETY: each syscall below is async-signal-safe. The parent
            // owns and reaps this child, including if readiness fails.
            unsafe {
                libc::close(ready_reader.as_raw_fd());
                libc::close(release_writer.as_raw_fd());
                let ready = [1_u8];
                if libc::write(ready_writer.as_raw_fd(), ready.as_ptr().cast(), 1) != 1 {
                    libc::_exit(101);
                }
                let mut release = [0_u8];
                let received =
                    libc::read(release_reader.as_raw_fd(), release.as_mut_ptr().cast(), 1);
                libc::_exit(if received == 0 { 0 } else { 102 });
            }
        }

        drop(ready_writer);
        drop(release_reader);
        let child = Self {
            pid,
            release_writer: Some(release_writer),
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "forked lock holder did not report readiness",
                ));
            }
            let mut poll = libc::pollfd {
                fd: ready_reader.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            let timeout = i32::try_from(remaining.as_millis()).unwrap_or(i32::MAX);
            // SAFETY: `poll` receives a valid pointer to one initialized item.
            let polled = unsafe { libc::poll(&mut poll, 1, timeout) };
            if polled == -1 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(error);
            }
            if polled == 0 {
                continue;
            }
            let mut ready = [0_u8];
            // SAFETY: `poll` reported readiness on this owned descriptor.
            let count =
                unsafe { libc::read(ready_reader.as_raw_fd(), ready.as_mut_ptr().cast(), 1) };
            if count == 1 && ready[0] == 1 {
                return Ok(child);
            }
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "forked lock holder did not send its ready byte",
            ));
        }
    }

    pub fn release_and_wait(mut self) -> io::Result<()> {
        use std::time::Instant;

        // Closing the sole writer releases the child by EOF. A byte write
        // would risk SIGPIPE if it exited between READY and this call.
        self.release_writer.take();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let mut status = 0;
            // SAFETY: this fixture is the only owner that waits for this pid.
            let observed = unsafe { libc::waitpid(self.pid, &mut status, libc::WNOHANG) };
            if observed == self.pid {
                self.pid = 0;
                if libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0 {
                    return Ok(());
                }
                return Err(io::Error::other("forked lock holder exited unsuccessfully"));
            }
            if observed == -1 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                if error.raw_os_error() == Some(libc::ECHILD) {
                    self.pid = 0;
                }
                return Err(error);
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "forked lock holder did not exit after release",
                ));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

#[cfg(all(any(test, feature = "test-support"), unix))]
impl Drop for ForkedInheritedDescriptorForTest {
    fn drop(&mut self) {
        self.release_writer.take();
        if self.pid > 0 {
            // SAFETY: this fixture still owns an unreaped child pid. SIGKILL
            // breaks a stuck pipe read; waitpid then prevents a zombie.
            unsafe {
                libc::kill(self.pid, libc::SIGKILL);
                let mut status = 0;
                while libc::waitpid(self.pid, &mut status, 0) == -1
                    && io::Error::last_os_error().kind() == io::ErrorKind::Interrupted
                {
                }
            }
        }
    }
}

/// Clear the execute bits of a fixture file so the OS refuses to start it.
/// Windows decides executability by the file contents, not by a mode bit.
#[cfg(unix)]
pub fn clear_execute_permission_for_test(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644))
}

#[cfg(not(unix))]
pub fn clear_execute_permission_for_test(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod medium_tests {
    use super::{
        classify_file_link_fixture_result, create_file_link_fixture_for_test,
        file_identity_for_test, path_identity_for_test, set_unix_mode_for_test, unix_mode_for_test,
        FileLinkFixtureOutcome,
    };
    use std::fs;
    use std::io;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    /// `mklink /J` отдаёт ноль и при отсутствующей цели, оставляя висячее
    /// соединение: атрибут точки стоит, цели нет. Помощник решает по
    /// состоянию файловой системы, поэтому такой вызов успехом не считает —
    /// иначе подготовка проверки формально прошла бы, а проверялось бы не то.
    #[cfg(windows)]
    #[test]
    fn a_junction_to_a_missing_target_is_not_a_success() {
        let root = unique_temp_root("junction-dangling");
        let link = root.join("link");
        let absent = root.join("absent");

        let error = super::create_directory_junction_for_test(&absent, &link)
            .expect_err("a junction to a missing target must not pass as created");

        let message = error.to_string();
        assert!(message.contains("does not lead to the target"), "{message}");
        assert!(message.contains("mklink /J"), "{message}");
        // Висячее соединение осталось на диске: помощник судит о состоянии,
        // а не убирает за `mklink`.
        assert!(fs::symlink_metadata(&link).is_ok(), "{message}");
        fs::remove_dir_all(&root).unwrap();
    }

    /// Соединение к существующему каталогу — успех, и его видно по состоянию:
    /// точка стоит и ведёт к цели.
    #[cfg(windows)]
    #[test]
    fn a_junction_to_a_directory_is_created_without_the_symlink_privilege() {
        let root = unique_temp_root("junction-created");
        let target = root.join("target");
        let link = root.join("link");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("inside.txt"), b"inside").unwrap();

        super::create_directory_junction_for_test(&target, &link).unwrap();

        assert_eq!(fs::read(link.join("inside.txt")).unwrap(), b"inside");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn unix_mode_fixture_is_exact_or_explicitly_unsupported() {
        let root = unique_temp_root("unix-mode");
        let target = root.join("target.bin");
        fs::write(&target, b"target").unwrap();

        let supported = set_unix_mode_for_test(&target, 0o640).unwrap();
        let mode = unix_mode_for_test(&target).unwrap();

        match mode {
            Some(mode) => {
                assert!(supported);
                assert_eq!(mode, 0o640);
            }
            None => assert!(!supported),
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn file_link_fixture_reports_only_explicit_unavailability() {
        let root = unique_temp_root("file-link");
        let source = root.join("source.bin");
        let target = root.join("target.bin");
        fs::write(&source, b"source").unwrap();

        let outcome = create_file_link_fixture_for_test(&source, &target)
            .expect("unexpected file-link creation error must fail the fixture test");

        match outcome {
            FileLinkFixtureOutcome::Created => {
                assert_eq!(fs::read_link(&target).unwrap(), source);
            }
            FileLinkFixtureOutcome::Unsupported => {
                fs::remove_dir_all(&root).unwrap();
                unica_process::test_support::skip_without_host_capability(
                    unica_process::test_support::HostCapability::Symlink,
                    "file links are unsupported on this host",
                );
                return;
            }
            FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => {
                fs::remove_dir_all(&root).unwrap();
                unica_process::test_support::skip_without_host_capability(
                    unica_process::test_support::HostCapability::Symlink,
                    "Windows file-link privilege is unavailable",
                );
                return;
            }
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn file_identity_fixture_observes_hard_link_aliases_when_supported() {
        let root = unique_temp_root("file-identity");
        let source = root.join("source.bin");
        let alias = root.join("alias.bin");
        fs::write(&source, b"source").unwrap();
        fs::hard_link(&source, &alias).unwrap();

        let source_identity = file_identity_for_test(&source)
            .unwrap()
            .expect("supported platform must expose file identity");
        let alias_identity = file_identity_for_test(&alias)
            .unwrap()
            .expect("supported platform must expose file identity");

        assert_eq!(source_identity, alias_identity);
        assert!(source_identity.contains("links=2"), "{source_identity}");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn path_identity_fixture_reads_directory_identity() {
        let root = unique_temp_root("path-identity-directory");

        let first = path_identity_for_test(&root).unwrap();
        let second = path_identity_for_test(&root).unwrap();

        assert_eq!(first, second);
        assert!(first.is_some(), "directory identity must be available");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn generic_permission_denied_file_link_error_is_not_skipped() {
        let error = classify_file_link_fixture_result(Some(Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "synthetic generic permission denial",
        ))))
        .expect_err("generic PermissionDenied must remain a real fixture error");

        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(error.raw_os_error(), None);
    }

    #[cfg(windows)]
    #[test]
    fn windows_error_1314_is_the_only_privilege_skip() {
        let outcome =
            classify_file_link_fixture_result(Some(Err(io::Error::from_raw_os_error(1314))))
                .unwrap();

        assert_eq!(outcome, FileLinkFixtureOutcome::WindowsPrivilegeUnavailable);
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_error_1314_remains_a_real_error() {
        let error =
            classify_file_link_fixture_result(Some(Err(io::Error::from_raw_os_error(1314))))
                .expect_err("raw error 1314 must not be special outside Windows");

        assert_eq!(error.raw_os_error(), Some(1314));
    }

    fn unique_temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "unica-platform-testing-{name}-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }
}
