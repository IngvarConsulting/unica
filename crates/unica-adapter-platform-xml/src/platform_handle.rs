use std::{fs::File, io};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OpenedFileIdentity {
    pub(crate) volume: u64,
    pub(crate) file: u128,
    pub(crate) links: u64,
}

#[cfg_attr(not(any(test, windows)), allow(dead_code))]
pub(crate) fn from_windows_file_id_parts(
    volume_serial_number: Option<u64>,
    file_id: Option<[u8; 16]>,
    number_of_links: Option<u64>,
) -> io::Result<OpenedFileIdentity> {
    let (Some(volume), Some(file_id), Some(links)) =
        (volume_serial_number, file_id, number_of_links)
    else {
        return Err(identity_unavailable());
    };
    if volume == 0 || file_id == [0; 16] || links == 0 {
        return Err(identity_unavailable());
    }
    Ok(OpenedFileIdentity {
        volume,
        file: u128::from_le_bytes(file_id),
        links,
    })
}

fn identity_unavailable() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "opened Windows handle has no complete stable identity",
    )
}

#[cfg(windows)]
pub(crate) fn query(file: &File) -> io::Result<OpenedFileIdentity> {
    use std::{mem::MaybeUninit, os::windows::io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::{
        FileIdInfo, FileStandardInfo, GetFileInformationByHandleEx, FILE_ID_INFO,
        FILE_STANDARD_INFO,
    };

    let handle = file.as_raw_handle().cast();
    let mut identity = MaybeUninit::<FILE_ID_INFO>::uninit();
    let identity_size = u32::try_from(std::mem::size_of::<FILE_ID_INFO>())
        .expect("FILE_ID_INFO size fits the Windows API width");
    let identity_success = unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileIdInfo,
            identity.as_mut_ptr().cast(),
            identity_size,
        )
    };
    if identity_success == 0 {
        return preserve_query_failure(Err(io::Error::last_os_error()));
    }
    let identity = unsafe { identity.assume_init() };

    let mut standard = MaybeUninit::<FILE_STANDARD_INFO>::uninit();
    let standard_size = u32::try_from(std::mem::size_of::<FILE_STANDARD_INFO>())
        .expect("FILE_STANDARD_INFO size fits the Windows API width");
    let standard_success = unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileStandardInfo,
            standard.as_mut_ptr().cast(),
            standard_size,
        )
    };
    if standard_success == 0 {
        return preserve_query_failure(Err(io::Error::last_os_error()));
    }
    let standard = unsafe { standard.assume_init() };

    preserve_query_failure(from_windows_file_id_parts(
        Some(identity.VolumeSerialNumber),
        Some(identity.FileId.Identifier),
        Some(u64::from(standard.NumberOfLinks)),
    ))
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
    fn file_id_info_identity_preserves_all_128_file_id_bits() {
        let file_id = [
            0x10, 0x32, 0x54, 0x76, 0x98, 0xba, 0xdc, 0xfe, 0xef, 0xcd, 0xab, 0x89, 0x67, 0x45,
            0x23, 0x01,
        ];
        let identity =
            from_windows_file_id_parts(Some(0xfedc_ba98_7654_3210), Some(file_id), Some(7))
                .unwrap();

        assert_eq!(identity.volume, 0xfedc_ba98_7654_3210);
        assert_eq!(identity.file, u128::from_le_bytes(file_id));
        assert_eq!(identity.links, 7);
    }

    #[test]
    fn missing_file_id_info_fields_are_unavailable_not_comparable() {
        let file_id = [1; 16];
        for result in [
            from_windows_file_id_parts(None, Some(file_id), Some(1)),
            from_windows_file_id_parts(Some(1), None, Some(1)),
            from_windows_file_id_parts(Some(1), Some(file_id), None),
        ] {
            assert_eq!(result.unwrap_err().kind(), io::ErrorKind::Unsupported);
        }
    }

    #[test]
    fn zero_identity_fields_are_unavailable_not_comparable() {
        for result in [
            from_windows_file_id_parts(Some(0), Some([1; 16]), Some(1)),
            from_windows_file_id_parts(Some(1), Some([0; 16]), Some(1)),
            from_windows_file_id_parts(Some(1), Some([1; 16]), Some(0)),
        ] {
            assert_eq!(result.unwrap_err().kind(), io::ErrorKind::Unsupported);
        }
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
