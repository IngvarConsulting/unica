use std::{fs::File, io};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OpenedFileIdentity {
    pub(crate) volume: u64,
    pub(crate) file: u64,
    pub(crate) links: u64,
}

#[cfg_attr(not(any(test, windows)), allow(dead_code))]
pub(crate) const fn from_windows_parts(
    volume_serial_number: u32,
    file_index_high: u32,
    file_index_low: u32,
    number_of_links: u32,
) -> OpenedFileIdentity {
    OpenedFileIdentity {
        volume: volume_serial_number as u64,
        file: ((file_index_high as u64) << 32) | file_index_low as u64,
        links: number_of_links as u64,
    }
}

#[cfg(windows)]
pub(crate) fn query(file: &File) -> io::Result<OpenedFileIdentity> {
    use std::{mem::MaybeUninit, os::windows::io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };

    let mut information = MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
    let success = unsafe {
        GetFileInformationByHandle(file.as_raw_handle().cast(), information.as_mut_ptr())
    };
    if success == 0 {
        return preserve_query_failure(Err(io::Error::last_os_error()));
    }
    let information = unsafe { information.assume_init() };
    preserve_query_failure(Ok(from_windows_parts(
        information.dwVolumeSerialNumber,
        information.nFileIndexHigh,
        information.nFileIndexLow,
        information.nNumberOfLinks,
    )))
}

#[cfg(not(windows))]
#[allow(dead_code)]
pub(crate) fn query(_file: &File) -> io::Result<OpenedFileIdentity> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Windows handle identity is unavailable on this host",
    ))
}

#[cfg(any(test, windows))]
fn preserve_query_failure(
    result: io::Result<OpenedFileIdentity>,
) -> io::Result<OpenedFileIdentity> {
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_identity_parts_are_widened_without_truncation() {
        let identity = from_windows_parts(0xfedc_ba98, 0x7654_3210, 0x89ab_cdef, 7);

        assert_eq!(identity.volume, 0x0000_0000_fedc_ba98);
        assert_eq!(identity.file, 0x7654_3210_89ab_cdef);
        assert_eq!(identity.links, 7);
    }

    #[test]
    fn zero_parts_remain_data_instead_of_a_query_failure_sentinel() {
        assert_eq!(
            from_windows_parts(0, 0, 0, 0),
            OpenedFileIdentity {
                volume: 0,
                file: 0,
                links: 0,
            }
        );
    }

    #[test]
    fn identity_query_failure_is_preserved_without_a_default_identity() {
        let error = preserve_query_failure(Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "injected identity query failure",
        )))
        .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    }
}
