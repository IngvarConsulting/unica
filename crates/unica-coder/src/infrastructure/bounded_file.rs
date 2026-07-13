use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum BoundedReadError {
    Open,
    Metadata,
    NotRegular,
    TooLarge,
    Read,
    InvalidUtf8,
}

pub(crate) struct BoundedBytes {
    pub(crate) bytes: Vec<u8>,
    pub(crate) bytes_read: u64,
}

pub(crate) fn read_bounded_bytes(
    path: &Path,
    max_bytes: u64,
    containment_root: Option<&Path>,
) -> Result<BoundedBytes, BoundedReadError> {
    let before = fs::symlink_metadata(path).map_err(|_| BoundedReadError::Metadata)?;
    if before.file_type().is_symlink() || !before.file_type().is_file() {
        return Err(BoundedReadError::NotRegular);
    }

    let file = open_bounded_file(path).map_err(|_| BoundedReadError::Open)?;
    let metadata = file.metadata().map_err(|_| BoundedReadError::Metadata)?;
    if !metadata.file_type().is_file() {
        return Err(BoundedReadError::NotRegular);
    }

    let after = fs::symlink_metadata(path).map_err(|_| BoundedReadError::Metadata)?;
    if after.file_type().is_symlink()
        || !after.file_type().is_file()
        || !opened_file_matches_path(path, &file, &before, &metadata, &after)
    {
        return Err(BoundedReadError::NotRegular);
    }

    if containment_root.is_some_and(|root| !opened_file_is_within(&file, root)) {
        return Err(BoundedReadError::NotRegular);
    }
    if metadata.len() > max_bytes {
        return Err(BoundedReadError::TooLarge);
    }

    let capacity = usize::try_from(metadata.len()).map_err(|_| BoundedReadError::TooLarge)?;
    let mut bytes = Vec::with_capacity(capacity);
    file.take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|_| BoundedReadError::Read)?;
    let bytes_read = u64::try_from(bytes.len()).map_err(|_| BoundedReadError::TooLarge)?;
    if bytes_read > max_bytes {
        return Err(BoundedReadError::TooLarge);
    }

    Ok(BoundedBytes { bytes, bytes_read })
}

#[cfg(windows)]
fn open_bounded_file(path: &Path) -> std::io::Result<fs::File> {
    use std::os::windows::fs::OpenOptionsExt;

    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

#[cfg(not(windows))]
fn open_bounded_file(path: &Path) -> std::io::Result<fs::File> {
    fs::File::open(path)
}

#[cfg(unix)]
fn opened_file_matches_path(
    _path: &Path,
    _file: &fs::File,
    before: &fs::Metadata,
    opened: &fs::Metadata,
    after: &fs::Metadata,
) -> bool {
    use std::os::unix::fs::MetadataExt;

    before.dev() == opened.dev()
        && before.ino() == opened.ino()
        && opened.dev() == after.dev()
        && opened.ino() == after.ino()
}

#[cfg(windows)]
fn opened_file_matches_path(
    path: &Path,
    file: &fs::File,
    _before: &fs::Metadata,
    _opened: &fs::Metadata,
    _after: &fs::Metadata,
) -> bool {
    let Ok(current) = open_bounded_file(path) else {
        return false;
    };
    let Ok(metadata) = current.metadata() else {
        return false;
    };
    metadata.file_type().is_file()
        && windows_file_identity(file)
            .zip(windows_file_identity(&current))
            .is_some_and(|(opened, current)| opened == current)
}

#[cfg(windows)]
fn windows_file_identity(file: &fs::File) -> Option<(u32, u64)> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };

    let mut information = unsafe { std::mem::zeroed::<BY_HANDLE_FILE_INFORMATION>() };
    let succeeded =
        unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut information) } != 0;
    succeeded.then(|| {
        let index =
            (u64::from(information.nFileIndexHigh) << 32) | u64::from(information.nFileIndexLow);
        (information.dwVolumeSerialNumber, index)
    })
}

#[cfg(not(any(unix, windows)))]
fn opened_file_matches_path(
    _path: &Path,
    _file: &fs::File,
    _before: &fs::Metadata,
    _opened: &fs::Metadata,
    _after: &fs::Metadata,
) -> bool {
    false
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn opened_file_path(file: &fs::File) -> Option<PathBuf> {
    use std::os::fd::AsRawFd;

    fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd())).ok()
}

#[cfg(target_os = "macos")]
fn opened_file_path(file: &fs::File) -> Option<PathBuf> {
    use std::ffi::{c_char, c_int, CStr, OsStr};
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStrExt;

    const F_GETPATH: c_int = 50;
    const PATH_BUFFER_SIZE: usize = 1024;

    unsafe extern "C" {
        fn fcntl(fd: c_int, command: c_int, ...) -> c_int;
    }

    let mut buffer = [0 as c_char; PATH_BUFFER_SIZE];
    let result = unsafe { fcntl(file.as_raw_fd(), F_GETPATH, buffer.as_mut_ptr()) };
    if result == -1 {
        return None;
    }
    let bytes = unsafe { CStr::from_ptr(buffer.as_ptr()) }.to_bytes();
    Some(PathBuf::from(OsStr::from_bytes(bytes)))
}

#[cfg(windows)]
fn opened_file_path(file: &fs::File) -> Option<PathBuf> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::GetFinalPathNameByHandleW;

    let handle = file.as_raw_handle() as _;
    let required = unsafe { GetFinalPathNameByHandleW(handle, std::ptr::null_mut(), 0, 0) };
    if required == 0 {
        return None;
    }
    let mut buffer = vec![0u16; usize::try_from(required).ok()?];
    let written = unsafe { GetFinalPathNameByHandleW(handle, buffer.as_mut_ptr(), required, 0) };
    if written == 0 || written >= required {
        return None;
    }
    let written = usize::try_from(written).ok()?;
    Some(PathBuf::from(OsString::from_wide(&buffer[..written])))
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "android",
    target_os = "macos",
    windows
)))]
fn opened_file_path(_file: &fs::File) -> Option<PathBuf> {
    None
}

fn opened_file_is_within(file: &fs::File, root: &Path) -> bool {
    let Some(opened_path) = opened_file_path(file) else {
        return false;
    };
    opened_path_has_prefix(&opened_path, root)
}

#[cfg(not(windows))]
fn opened_path_has_prefix(path: &Path, root: &Path) -> bool {
    path.starts_with(root)
}

#[cfg(windows)]
fn opened_path_has_prefix(path: &Path, root: &Path) -> bool {
    use std::os::windows::ffi::OsStrExt;

    fn component_key(component: &std::path::Component<'_>) -> Vec<u16> {
        component
            .as_os_str()
            .encode_wide()
            .map(|unit| match unit {
                0x41..=0x5a => unit + 0x20,
                _ => unit,
            })
            .collect()
    }

    let mut path_components = path.components();
    root.components().all(|root_component| {
        path_components.next().is_some_and(|path_component| {
            component_key(&path_component) == component_key(&root_component)
        })
    })
}
