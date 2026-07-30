use std::fs;
#[cfg(unix)]
use std::fs::File;
use std::io;
use std::path::Path;

#[cfg(all(test, unix))]
pub(crate) fn create_test_directory_link(target: &Path, link: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(all(test, windows))]
pub(crate) fn create_test_directory_link(target: &Path, link: &Path) -> io::Result<()> {
    std::os::windows::fs::symlink_dir(target, link)
}

#[cfg(all(test, not(any(unix, windows))))]
pub(crate) fn create_test_directory_link(_target: &Path, _link: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "test directory links are unavailable on this host",
    ))
}

#[derive(Debug, Clone)]
pub(crate) struct PortablePermissions {
    permissions: fs::Permissions,
    key: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FileIdentity {
    volume: u64,
    file: u64,
}

impl PortablePermissions {
    pub(crate) fn readonly(&self) -> bool {
        self.permissions.readonly()
    }

    pub(crate) fn matches(&self, metadata: &fs::Metadata) -> bool {
        self.key == portable_permission_key(metadata)
    }

    pub(crate) fn apply_to(&self, file: &fs::File) -> io::Result<()> {
        file.set_permissions(self.permissions.clone())
    }
}

pub(crate) fn portable_permissions(metadata: &fs::Metadata) -> PortablePermissions {
    PortablePermissions {
        permissions: metadata.permissions(),
        key: portable_permission_key(metadata),
    }
}

#[cfg(unix)]
fn portable_permission_key(metadata: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;

    metadata.permissions().mode() & 0o7777
}

#[cfg(not(unix))]
fn portable_permission_key(metadata: &fs::Metadata) -> u32 {
    u32::from(metadata.permissions().readonly())
}

#[cfg(unix)]
pub(crate) fn restrict_stage_to_owner(file: &fs::File) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    file.set_permissions(fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
pub(crate) fn restrict_stage_to_owner(_file: &fs::File) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
pub(crate) fn hard_link_count(file: &fs::File) -> io::Result<u64> {
    use std::os::unix::fs::MetadataExt;

    Ok(file.metadata()?.nlink())
}

#[cfg(unix)]
pub(crate) fn file_identity(file: &fs::File) -> io::Result<FileIdentity> {
    use std::os::unix::fs::MetadataExt;

    let metadata = file.metadata()?;
    Ok(FileIdentity {
        volume: metadata.dev(),
        file: metadata.ino(),
    })
}

#[cfg(windows)]
pub(crate) fn hard_link_count(file: &fs::File) -> io::Result<u64> {
    Ok(u64::from(windows_file_information(file)?.nNumberOfLinks))
}

#[cfg(windows)]
pub(crate) fn file_identity(file: &fs::File) -> io::Result<FileIdentity> {
    let information = windows_file_information(file)?;
    Ok(FileIdentity {
        volume: u64::from(information.dwVolumeSerialNumber),
        file: (u64::from(information.nFileIndexHigh) << 32) | u64::from(information.nFileIndexLow),
    })
}

#[cfg(windows)]
fn windows_file_information(
    file: &fs::File,
) -> io::Result<windows_sys::Win32::Storage::FileSystem::BY_HANDLE_FILE_INFORMATION> {
    use std::mem::MaybeUninit;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };

    let mut information = MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
    // SAFETY: the file owns a valid handle, and the pointer provides writable storage that the
    // API fully initializes before returning a nonzero result.
    let succeeded =
        unsafe { GetFileInformationByHandle(file.as_raw_handle(), information.as_mut_ptr()) };
    if succeeded == 0 {
        Err(io::Error::last_os_error())
    } else {
        // SAFETY: a nonzero result guarantees that GetFileInformationByHandle initialized the
        // entire BY_HANDLE_FILE_INFORMATION value.
        Ok(unsafe { information.assume_init() })
    }
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn hard_link_count(_file: &fs::File) -> io::Result<u64> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "hard-link count is not available on this host",
    ))
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn file_identity(_file: &fs::File) -> io::Result<FileIdentity> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "file identity is not available on this host",
    ))
}

#[cfg(any(test, windows))]
fn windows_api_path_from_utf16(mut path: Vec<u16>, absolute: bool) -> Vec<u16> {
    const BACKSLASH: u16 = b'\\' as u16;
    const FORWARD_SLASH: u16 = b'/' as u16;
    const QUESTION_MARK: u16 = b'?' as u16;
    const DOT: u16 = b'.' as u16;

    let has_device_prefix = path.starts_with(&[BACKSLASH, BACKSLASH, QUESTION_MARK, BACKSLASH])
        || path.starts_with(&[BACKSLASH, BACKSLASH, DOT, BACKSLASH]);
    if absolute && !has_device_prefix {
        for unit in &mut path {
            if *unit == FORWARD_SLASH {
                *unit = BACKSLASH;
            }
        }
        let mut extended = if path.starts_with(&[BACKSLASH, BACKSLASH]) {
            r"\\?\UNC\".encode_utf16().collect::<Vec<_>>()
        } else {
            r"\\?\".encode_utf16().collect::<Vec<_>>()
        };
        if path.starts_with(&[BACKSLASH, BACKSLASH]) {
            extended.extend_from_slice(&path[2..]);
        } else {
            extended.extend_from_slice(&path);
        }
        path = extended;
    }
    path.push(0);
    path
}

#[cfg(windows)]
fn windows_api_path(path: &Path) -> io::Result<Vec<u16>> {
    use std::os::windows::ffi::OsStrExt;

    let path = std::path::absolute(path)?;
    let encoded = path.as_os_str().encode_wide().collect::<Vec<_>>();
    if encoded.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Windows path contains NUL",
        ));
    }
    Ok(windows_api_path_from_utf16(encoded, true))
}

#[cfg(windows)]
#[allow(
    dead_code,
    reason = "DirectoryAnchor callers are introduced by the following Windows full-dump task"
)]
struct ProcessToken {
    handle: windows_sys::Win32::Foundation::HANDLE,
    user: Vec<u8>,
}

#[cfg(windows)]
impl ProcessToken {
    fn current_user() -> io::Result<Self> {
        use std::ptr;
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY};
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

        let mut handle = ptr::null_mut();
        // SAFETY: GetCurrentProcess returns a valid pseudo-handle; handle is writable storage.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut handle) } == 0 {
            return Err(io::Error::last_os_error());
        }

        let mut length = 0;
        // SAFETY: the first call requests the buffer size without supplying a buffer.
        unsafe {
            GetTokenInformation(handle, TokenUser, ptr::null_mut(), 0, &mut length);
        }
        if length == 0 {
            let error = io::Error::last_os_error();
            // SAFETY: OpenProcessToken returned this owned handle.
            unsafe { CloseHandle(handle) };
            return Err(error);
        }

        let mut user = vec![0; length as usize];
        // SAFETY: user provides the byte capacity requested by the preceding call.
        if unsafe {
            GetTokenInformation(
                handle,
                TokenUser,
                user.as_mut_ptr().cast(),
                length,
                &mut length,
            )
        } == 0
        {
            let error = io::Error::last_os_error();
            // SAFETY: OpenProcessToken returned this owned handle.
            unsafe { CloseHandle(handle) };
            return Err(error);
        }

        Ok(Self { handle, user })
    }

    fn user_sid(&self) -> windows_sys::Win32::Security::PSID {
        use windows_sys::Win32::Security::TOKEN_USER;

        // SAFETY: GetTokenInformation initialized the buffer as TOKEN_USER, including the SID
        // pointer, and the buffer remains owned by self for this borrow.
        unsafe {
            std::ptr::read_unaligned(self.user.as_ptr().cast::<TOKEN_USER>())
                .User
                .Sid
        }
    }
}

#[cfg(windows)]
impl Drop for ProcessToken {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::CloseHandle;

        // SAFETY: self.handle is an owned token handle returned by OpenProcessToken.
        unsafe { CloseHandle(self.handle) };
    }
}

#[cfg(windows)]
#[allow(
    dead_code,
    reason = "DirectoryAnchor callers are introduced by the following Windows full-dump task"
)]
struct OwnerOnlySecurityAttributes {
    token: ProcessToken,
    sid_string: windows_sys::core::PWSTR,
    security_descriptor: windows_sys::Win32::Security::PSECURITY_DESCRIPTOR,
    attributes: windows_sys::Win32::Security::SECURITY_ATTRIBUTES,
}

#[cfg(windows)]
impl OwnerOnlySecurityAttributes {
    fn current_user() -> io::Result<Self> {
        use std::mem::size_of;
        use std::ptr;
        use windows_sys::Win32::Foundation::LocalFree;
        use windows_sys::Win32::Security::Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            SDDL_REVISION_1,
        };
        use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;

        let token = ProcessToken::current_user()?;
        let mut sid_string = ptr::null_mut();
        // SAFETY: token.user_sid() is valid while token is live, and sid_string is writable.
        if unsafe { ConvertSidToStringSidW(token.user_sid(), &mut sid_string) } == 0 {
            return Err(io::Error::last_os_error());
        }

        let mut sddl = "D:P(A;;FA;;;".encode_utf16().collect::<Vec<_>>();
        // SAFETY: ConvertSidToStringSidW returned a NUL-terminated UTF-16 string allocated by
        // LocalAlloc, which remains live until OwnerOnlySecurityAttributes is dropped.
        let sid_length = unsafe {
            let mut length = 0;
            while *sid_string.add(length) != 0 {
                length += 1;
            }
            length
        };
        // SAFETY: sid_length stops at the allocation's terminating NUL.
        sddl.extend_from_slice(unsafe { std::slice::from_raw_parts(sid_string, sid_length) });
        sddl.extend(")".encode_utf16());
        sddl.push(0);

        let mut security_descriptor = ptr::null_mut();
        // SAFETY: sddl is NUL-terminated for the duration of the call and the descriptor output
        // pointer is writable.
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut security_descriptor,
                ptr::null_mut(),
            )
        } == 0
        {
            let error = io::Error::last_os_error();
            // SAFETY: ConvertSidToStringSidW allocated sid_string with LocalAlloc.
            unsafe { LocalFree(sid_string.cast()) };
            return Err(error);
        }

        Ok(Self {
            token,
            sid_string,
            security_descriptor,
            attributes: SECURITY_ATTRIBUTES {
                nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: security_descriptor,
                bInheritHandle: 0,
            },
        })
    }

    fn as_ptr(&self) -> *const windows_sys::Win32::Security::SECURITY_ATTRIBUTES {
        let _ = self.token.handle;
        &self.attributes
    }
}

#[cfg(windows)]
impl Drop for OwnerOnlySecurityAttributes {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::LocalFree;

        // SAFETY: both pointers were allocated by documented APIs with LocalAlloc semantics.
        unsafe {
            LocalFree(self.security_descriptor);
            LocalFree(self.sid_string.cast());
        }
    }
}

#[cfg(windows)]
#[allow(
    dead_code,
    reason = "DirectoryAnchor callers are introduced by the following Windows full-dump task"
)]
struct LocalSecurityDescriptor(windows_sys::Win32::Security::PSECURITY_DESCRIPTOR);

#[cfg(windows)]
impl Drop for LocalSecurityDescriptor {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::LocalFree;

        // SAFETY: GetSecurityInfo returns this descriptor through LocalAlloc.
        unsafe { LocalFree(self.0) };
    }
}

#[cfg(windows)]
#[allow(
    dead_code,
    reason = "DirectoryAnchor callers are introduced by the following Windows full-dump task"
)]
pub(crate) fn open_directory_nofollow(path: &Path) -> io::Result<fs::File> {
    use std::os::windows::io::FromRawHandle;
    use std::ptr;
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, OPEN_EXISTING, READ_CONTROL,
    };

    let path = windows_api_path(path)?;
    // SAFETY: path is NUL-terminated and all scalar arguments are documented Win32 flags.
    let handle = unsafe {
        CreateFileW(
            path.as_ptr(),
            FILE_READ_ATTRIBUTES | READ_CONTROL,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: CreateFileW returned an owned, valid file handle.
    let file = unsafe { fs::File::from_raw_handle(handle) };
    if windows_file_information(&file)?.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "directory path resolves to a reparse point",
        ));
    }
    Ok(file)
}

#[cfg(windows)]
#[allow(
    dead_code,
    reason = "DirectoryAnchor callers are introduced by the following Windows full-dump task"
)]
pub(crate) fn create_owner_only_directory(path: &Path) -> io::Result<fs::File> {
    use windows_sys::Win32::Storage::FileSystem::CreateDirectoryW;

    let api_path = windows_api_path(path)?;
    let security = OwnerOnlySecurityAttributes::current_user()?;
    // SAFETY: path is NUL-terminated and security owns the descriptor for this call.
    if unsafe { CreateDirectoryW(api_path.as_ptr(), security.as_ptr()) } == 0 {
        return Err(io::Error::last_os_error());
    }
    open_directory_nofollow(path)
}

#[cfg(windows)]
#[allow(
    dead_code,
    reason = "DirectoryAnchor callers are introduced by the following Windows full-dump task"
)]
pub(crate) fn create_owner_only_file(path: &Path) -> io::Result<fs::File> {
    use std::os::windows::io::FromRawHandle;
    use std::ptr;
    use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, CREATE_NEW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_DELETE,
    };

    let path = windows_api_path(path)?;
    let security = OwnerOnlySecurityAttributes::current_user()?;
    // SAFETY: path is NUL-terminated and security owns the descriptor for this call.
    let handle = unsafe {
        CreateFileW(
            path.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_DELETE,
            security.as_ptr(),
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        Err(io::Error::last_os_error())
    } else {
        // SAFETY: CreateFileW returned an owned, valid file handle.
        Ok(unsafe { fs::File::from_raw_handle(handle) })
    }
}

#[cfg(windows)]
#[allow(
    dead_code,
    reason = "DirectoryAnchor callers are introduced by the following Windows full-dump task"
)]
pub(crate) fn verify_owner_only_acl(file: &fs::File) -> io::Result<()> {
    use std::mem::size_of;
    use std::os::windows::io::AsRawHandle;
    use std::ptr;
    use windows_sys::Win32::Security::Authorization::{GetSecurityInfo, SE_FILE_OBJECT};
    use windows_sys::Win32::Security::{
        EqualSid, GetAce, GetAclInformation, GetSecurityDescriptorControl, ACCESS_ALLOWED_ACE,
        ACL_SIZE_INFORMATION, AclSizeInformation, ACE_HEADER, DACL_SECURITY_INFORMATION,
        INHERITED_ACE, SE_DACL_PROTECTED,
    };

    const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;

    let token = ProcessToken::current_user()?;
    let mut dacl = ptr::null_mut();
    let mut descriptor = ptr::null_mut();
    // SAFETY: file owns a valid handle and all requested output pointers are writable.
    let status = unsafe {
        GetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            &mut dacl,
            ptr::null_mut(),
            &mut descriptor,
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    let descriptor = LocalSecurityDescriptor(descriptor);
    if dacl.is_null() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "owner-only object has no DACL",
        ));
    }

    let mut control = 0;
    let mut revision = 0;
    // SAFETY: descriptor is valid until its RAII wrapper drops.
    if unsafe { GetSecurityDescriptorControl(descriptor.0, &mut control, &mut revision) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if control & SE_DACL_PROTECTED == 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "owner-only object DACL is not protected",
        ));
    }

    let mut information = ACL_SIZE_INFORMATION {
        AceCount: 0,
        AclBytesInUse: 0,
        AclBytesFree: 0,
    };
    // SAFETY: dacl is valid for the descriptor lifetime and information is writable storage.
    if unsafe {
        GetAclInformation(
            dacl,
            (&mut information as *mut ACL_SIZE_INFORMATION).cast(),
            size_of::<ACL_SIZE_INFORMATION>() as u32,
            AclSizeInformation,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    if information.AceCount != 1 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "owner-only object DACL does not contain exactly one ACE",
        ));
    }

    let mut ace = ptr::null_mut();
    // SAFETY: dacl is valid and ACE index zero is within the exactly-one ACE DACL.
    if unsafe { GetAce(dacl, 0, &mut ace) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: GetAce returned a pointer to a valid ACE in dacl.
    let header = unsafe { &*ace.cast::<ACE_HEADER>() };
    if header.AceType != ACCESS_ALLOWED_ACE_TYPE
        || u32::from(header.AceFlags) & INHERITED_ACE != 0
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "owner-only object DACL has an unexpected ACE",
        ));
    }
    // SAFETY: the ACE type is ACCESS_ALLOWED_ACE_TYPE, so its payload has this layout.
    let allowed = unsafe { &*ace.cast::<ACCESS_ALLOWED_ACE>() };
    let ace_sid: windows_sys::Win32::Security::PSID =
        (&raw const allowed.SidStart).cast_mut().cast();
    // SAFETY: both SIDs are valid: one belongs to the ACL and the other to the current token.
    if unsafe { EqualSid(ace_sid, token.user_sid()) } == 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "owner-only object DACL grants access to a different SID",
        ));
    }
    Ok(())
}

pub(crate) fn install_file_no_clobber(source: &Path, target: &Path) -> io::Result<()> {
    fs::hard_link(source, target)
}

#[cfg(any(target_os = "linux", target_os = "android"))]
pub(crate) fn rename_no_replace(source: &Path, target: &Path) -> io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let source = CString::new(source.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "source path contains NUL"))?;
    let target = CString::new(target.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "target path contains NUL"))?;
    #[cfg(target_os = "linux")]
    let no_replace_flag = libc::RENAME_NOREPLACE;
    #[cfg(target_os = "android")]
    let no_replace_flag = libc::RENAME_NOREPLACE as libc::c_uint;
    // SAFETY: both C strings are NUL-terminated and remain live for the syscall. The raw syscall
    // avoids a glibc-only symbol so this remains available on Linux musl; RENAME_NOREPLACE asks
    // the kernel to fail atomically when the destination already exists.
    let moved = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            target.as_ptr(),
            no_replace_flag,
        )
    };
    if moved == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(target_vendor = "apple")]
pub(crate) fn rename_no_replace(source: &Path, target: &Path) -> io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let source = CString::new(source.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "source path contains NUL"))?;
    let target = CString::new(target.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "target path contains NUL"))?;
    // SAFETY: both C strings are NUL-terminated and remain live for the call. RENAME_EXCL makes
    // destination existence an atomic failure instead of replacing it.
    let moved = unsafe { libc::renamex_np(source.as_ptr(), target.as_ptr(), libc::RENAME_EXCL) };
    if moved == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(windows)]
pub(crate) fn rename_no_replace(source: &Path, target: &Path) -> io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH};

    let source = windows_api_path(source)?;
    let target = windows_api_path(target)?;
    // SAFETY: both pointers reference NUL-terminated UTF-16 buffers for the call duration.
    // Omitting MOVEFILE_REPLACE_EXISTING makes destination existence an atomic failure.
    let moved = unsafe { MoveFileExW(source.as_ptr(), target.as_ptr(), MOVEFILE_WRITE_THROUGH) };
    if moved == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "android",
    target_vendor = "apple",
    windows
)))]
pub(crate) fn rename_no_replace(_source: &Path, _target: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "atomic no-replace rename is not available on this host",
    ))
}

#[cfg(windows)]
pub(crate) fn host_path_text(path: String) -> String {
    path.replace('\\', "/")
}

#[cfg(not(windows))]
pub(crate) fn host_path_text(path: String) -> String {
    path
}

#[cfg(windows)]
pub(crate) fn strip_windows_extended_length_prefix(path: &Path) -> std::path::PathBuf {
    use std::path::PathBuf;

    let path = path.as_os_str().to_string_lossy();
    if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{unc}"));
    }
    if let Some(regular) = path.strip_prefix(r"\\?\") {
        let bytes = regular.as_bytes();
        if bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && matches!(bytes[2], b'\\' | b'/')
        {
            return PathBuf::from(regular);
        }
    }
    PathBuf::from(path.as_ref())
}

#[cfg(not(windows))]
pub(crate) fn strip_windows_extended_length_prefix(path: &Path) -> std::path::PathBuf {
    path.to_path_buf()
}

#[cfg(windows)]
pub(crate) fn path_starts_with_host_root(path: &Path, root: &Path) -> bool {
    let path = strip_windows_extended_length_prefix(path);
    let root = strip_windows_extended_length_prefix(root);
    let path_components = path.components().collect::<Vec<_>>();
    let root_components = root.components().collect::<Vec<_>>();
    path_components.len() >= root_components.len()
        && path_components
            .iter()
            .zip(root_components.iter())
            .all(|(left, right)| {
                left.as_os_str().to_string_lossy().to_lowercase()
                    == right.as_os_str().to_string_lossy().to_lowercase()
            })
}

#[cfg(not(windows))]
pub(crate) fn path_starts_with_host_root(path: &Path, root: &Path) -> bool {
    path.starts_with(root)
}

#[cfg(all(test, unix))]
pub(crate) fn create_file_symlink_for_test(
    source: impl AsRef<Path>,
    target: impl AsRef<Path>,
) -> Option<io::Result<()>> {
    use std::os::unix::fs::symlink;

    Some(symlink(source, target))
}

#[cfg(all(test, unix))]
pub(crate) fn create_dir_symlink_for_test(
    source: impl AsRef<Path>,
    target: impl AsRef<Path>,
) -> Option<io::Result<()>> {
    use std::os::unix::fs::symlink;

    Some(symlink(source, target))
}

#[cfg(all(test, windows))]
pub(crate) fn create_file_symlink_for_test(
    source: impl AsRef<Path>,
    target: impl AsRef<Path>,
) -> Option<io::Result<()>> {
    use std::os::windows::fs::symlink_file;

    Some(symlink_file(source, target))
}

#[cfg(all(test, windows))]
pub(crate) fn create_dir_symlink_for_test(
    source: impl AsRef<Path>,
    target: impl AsRef<Path>,
) -> Option<io::Result<()>> {
    use std::os::windows::fs::symlink_dir;

    Some(symlink_dir(source, target))
}

#[cfg(all(test, not(any(unix, windows))))]
pub(crate) fn create_file_symlink_for_test(
    _source: impl AsRef<Path>,
    _target: impl AsRef<Path>,
) -> Option<io::Result<()>> {
    None
}

#[cfg(all(test, not(any(unix, windows))))]
pub(crate) fn create_dir_symlink_for_test(
    _source: impl AsRef<Path>,
    _target: impl AsRef<Path>,
) -> Option<io::Result<()>> {
    None
}

pub(crate) fn metadata_is_link_or_reparse_point(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    metadata_is_reparse_point(metadata)
}

#[cfg(windows)]
fn metadata_is_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn metadata_is_reparse_point(_metadata: &fs::Metadata) -> bool {
    false
}

#[cfg(not(windows))]
pub(crate) fn replace_file_atomically(source: &Path, target: &Path) -> io::Result<()> {
    fs::rename(source, target)
}

#[cfg(windows)]
pub(crate) fn replace_file_atomically(source: &Path, target: &Path) -> io::Result<()> {
    const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(existing: *const u16, replacement: *const u16, flags: u32) -> i32;
    }

    let source = windows_api_path(source)?;
    let target = windows_api_path(target)?;
    // SAFETY: both pointers reference NUL-terminated UTF-16 buffers for the call duration.
    let moved = unsafe {
        MoveFileExW(
            source.as_ptr(),
            target.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(unix)]
pub(crate) fn sync_parent_directory(parent: &Path) -> io::Result<()> {
    File::open(parent).and_then(|directory| directory.sync_all())
}

#[cfg(not(unix))]
pub(crate) fn sync_parent_directory(_parent: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(not(windows))]
pub(crate) fn prepare_file_for_removal(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(windows)]
#[allow(
    clippy::permissions_set_readonly_false,
    reason = "on Windows this only clears the FILE_ATTRIBUTE_READONLY flag"
)]
pub(crate) fn prepare_file_for_removal(path: &Path) -> io::Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let mut permissions = metadata.permissions();
    if permissions.readonly() {
        permissions.set_readonly(false);
        fs::set_permissions(path, permissions)?;
    }
    Ok(())
}

pub(crate) fn path_lock_identity(path: &Path) -> String {
    path_lock_identity_text(&path.to_string_lossy())
}

#[cfg(any(windows, target_os = "macos"))]
fn path_lock_identity_text(path: &str) -> String {
    path.to_lowercase()
}

#[cfg(not(any(windows, target_os = "macos")))]
fn path_lock_identity_text(path: &str) -> String {
    path.to_string()
}

#[cfg(test)]
mod tests {
    use super::{path_lock_identity_text, path_starts_with_host_root, windows_api_path_from_utf16};
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[cfg(windows)]
    use super::strip_windows_extended_length_prefix;

    #[cfg(windows)]
    mod windows {
        use super::{fs, io, unique_temp_root};
        use crate::infrastructure::platform::filesystem::{
            create_owner_only_directory, create_owner_only_file, file_identity,
            open_directory_nofollow, verify_owner_only_acl,
        };

        #[test]
        fn owner_only_directory_is_opened_without_following_reparse_points() {
            let root = unique_temp_root("owner-only-directory");
            fs::create_dir_all(&root).unwrap();
            let private = root.join("private");

            let handle = create_owner_only_directory(&private).unwrap();

            verify_owner_only_acl(&handle).unwrap();
            assert_eq!(
                file_identity(&handle).unwrap(),
                file_identity(&open_directory_nofollow(&private).unwrap()).unwrap()
            );
            fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn directory_open_rejects_a_reparse_point() {
            let root = unique_temp_root("directory-reparse");
            let real = root.join("real");
            let link = root.join("link");
            fs::create_dir_all(&real).unwrap();
            std::os::windows::fs::symlink_dir(&real, &link).unwrap();

            let error = open_directory_nofollow(&link).unwrap_err();

            assert!(matches!(
                error.kind(),
                io::ErrorKind::InvalidInput | io::ErrorKind::PermissionDenied
            ));
            fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn owner_only_file_has_the_final_acl_at_creation() {
            let root = unique_temp_root("owner-only-file");
            fs::create_dir_all(&root).unwrap();
            let private = root.join("effective.yaml");

            let handle = create_owner_only_file(&private).unwrap();

            verify_owner_only_acl(&handle).unwrap();
            fs::remove_dir_all(root).unwrap();
        }
    }

    fn unique_temp_root(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "unica-filesystem-{name}-{}-{nanos}",
            std::process::id()
        ))
    }

    fn windows_api_path_text(path: &str, absolute: bool) -> String {
        let mut encoded = windows_api_path_from_utf16(path.encode_utf16().collect(), absolute);
        assert_eq!(encoded.pop(), Some(0));
        String::from_utf16(&encoded).unwrap()
    }

    #[test]
    fn windows_api_paths_use_extended_prefixes_without_lossy_text_conversion() {
        assert_eq!(
            windows_api_path_text(r"C:/deep/source.xml", true),
            r"\\?\C:\deep\source.xml"
        );
        assert_eq!(
            windows_api_path_text(r"\\server\share/deep/source.xml", true),
            r"\\?\UNC\server\share\deep\source.xml"
        );
        assert_eq!(
            windows_api_path_text(r"\\?\C:\deep\source.xml", true),
            r"\\?\C:\deep\source.xml"
        );
        assert_eq!(
            windows_api_path_text(r"relative/source.xml", false),
            r"relative/source.xml"
        );
    }

    #[test]
    fn no_clobber_install_never_replaces_an_existing_target() {
        use super::install_file_no_clobber;

        let root = unique_temp_root("no-clobber-install");
        fs::create_dir_all(&root).unwrap();
        let staged = root.join("staged");
        let target = root.join("target");
        fs::write(&staged, b"replacement").unwrap();
        fs::write(&target, b"original").unwrap();

        let error = install_file_no_clobber(&staged, &target).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&staged).unwrap(), b"replacement");
        assert_eq!(fs::read(&target).unwrap(), b"original");

        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn hard_link_count_observes_a_second_name() {
        use super::hard_link_count;

        let root = unique_temp_root("hard-link-count");
        fs::create_dir_all(&root).unwrap();
        let target = root.join("target");
        let alias = root.join("alias");
        fs::write(&target, b"content").unwrap();
        fs::hard_link(&target, &alias).unwrap();

        let target_file = fs::File::open(&target).unwrap();

        assert_eq!(hard_link_count(&target_file).unwrap(), 2);

        drop(target_file);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn portable_permissions_round_trip_mode_0600() {
        use super::{portable_permissions, restrict_stage_to_owner};
        use std::os::unix::fs::PermissionsExt;

        let root = unique_temp_root("portable-permissions");
        fs::create_dir_all(&root).unwrap();
        let source = root.join("source");
        let staged = root.join("staged");
        fs::write(&source, b"source").unwrap();
        fs::set_permissions(&source, fs::Permissions::from_mode(0o600)).unwrap();
        let expected = portable_permissions(&fs::metadata(&source).unwrap());
        let staged_file = fs::File::create(&staged).unwrap();

        assert!(!expected.readonly());
        restrict_stage_to_owner(&staged_file).unwrap();
        expected.apply_to(&staged_file).unwrap();
        let staged_metadata = staged_file.metadata().unwrap();

        assert!(expected.matches(&staged_metadata));
        assert_eq!(staged_metadata.permissions().mode() & 0o7777, 0o600);

        drop(staged_file);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn portable_permissions_restore_mode_after_stage_restriction() {
        use super::{portable_permissions, restrict_stage_to_owner};
        use std::os::unix::fs::PermissionsExt;

        let root = unique_temp_root("portable-permissions-restore");
        fs::create_dir_all(&root).unwrap();
        let source = root.join("source");
        let staged = root.join("staged");
        fs::write(&source, b"source").unwrap();
        fs::set_permissions(&source, fs::Permissions::from_mode(0o640)).unwrap();
        let expected = portable_permissions(&fs::metadata(&source).unwrap());
        let staged_file = fs::File::create(&staged).unwrap();

        restrict_stage_to_owner(&staged_file).unwrap();
        assert_eq!(
            staged_file.metadata().unwrap().permissions().mode() & 0o7777,
            0o600
        );

        expected.apply_to(&staged_file).unwrap();
        let staged_metadata = staged_file.metadata().unwrap();

        assert!(expected.matches(&staged_metadata));
        assert_eq!(staged_metadata.permissions().mode() & 0o7777, 0o640);

        drop(staged_file);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn lock_identity_follows_host_case_policy() {
        let identity = path_lock_identity_text("/Workspace/Configuration.xml");
        if cfg!(any(windows, target_os = "macos")) {
            assert_eq!(identity, "/workspace/configuration.xml");
        } else {
            assert_eq!(identity, "/Workspace/Configuration.xml");
        }
    }

    #[test]
    fn containment_prefix_follows_host_case_policy() {
        let matches = path_starts_with_host_root(
            Path::new("/WORKSPACE/src/Module.bsl"),
            Path::new("/workspace"),
        );
        if cfg!(windows) {
            assert!(matches);
        } else {
            assert!(!matches);
        }
    }

    #[cfg(unix)]
    #[test]
    fn workspace_policy_rejects_lexically_external_symlink_into_workspace() {
        use crate::domain::workspace::WorkspaceContext;
        use crate::infrastructure::path_policy::WorkspacePathPolicy;
        use std::os::unix::fs::symlink;

        let temp = unique_temp_root("path-policy-inbound-link");
        let workspace = temp.join("workspace");
        let outside = temp.join("outside");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(workspace.join("Configuration.xml"), "<MetaDataObject/>").unwrap();
        symlink(&workspace, outside.join("workspace-alias")).unwrap();
        let context = WorkspaceContext {
            cwd: workspace.clone(),
            workspace_root: workspace.clone(),
            cache_root: workspace.join(".build").join("unica"),
            workspace_epoch: 1,
        };
        let policy = WorkspacePathPolicy::new(&context);

        let error = policy
            .resolve_write(outside.join("workspace-alias/Configuration.xml"))
            .unwrap_err();

        assert!(error.contains("outside workspace root"));
        fs::remove_dir_all(temp).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn workspace_policy_rejects_lexically_internal_symlink_outside_workspace() {
        use crate::domain::workspace::WorkspaceContext;
        use crate::infrastructure::path_policy::WorkspacePathPolicy;
        use std::os::unix::fs::symlink;

        let temp = unique_temp_root("path-policy-outbound-link");
        let workspace = temp.join("workspace");
        let outside = temp.join("outside");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("Configuration.xml"), "<MetaDataObject/>").unwrap();
        symlink(&outside, workspace.join("outside-alias")).unwrap();
        let context = WorkspaceContext {
            cwd: workspace.clone(),
            workspace_root: workspace.clone(),
            cache_root: workspace.join(".build").join("unica"),
            workspace_epoch: 1,
        };
        let policy = WorkspacePathPolicy::new(&context);

        let error = policy
            .resolve_write(workspace.join("outside-alias/Configuration.xml"))
            .unwrap_err();

        assert!(error.contains("outside workspace root"));
        fs::remove_dir_all(temp).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn workspace_policy_accepts_normalized_child_of_verbatim_workspace_root() {
        use crate::domain::workspace::WorkspaceContext;
        use crate::infrastructure::path_policy::WorkspacePathPolicy;
        use crate::infrastructure::source_roots::normalize_path_identity;

        let regular_root = unique_temp_root("path-policy-verbatim");
        let child = regular_root.join("src/CommonModules/Example.xml");
        fs::create_dir_all(child.parent().unwrap()).unwrap();
        fs::write(&child, "<MetaDataObject/>").unwrap();
        let verbatim_root = PathBuf::from(format!(r"\\?\{}", regular_root.display()));
        let context = WorkspaceContext {
            cwd: verbatim_root.clone(),
            workspace_root: verbatim_root.clone(),
            cache_root: verbatim_root.join(".build").join("unica"),
            workspace_epoch: 1,
        };
        let policy = WorkspacePathPolicy::new(&context);
        let normalized_child = normalize_path_identity(&child).unwrap();

        assert_eq!(
            policy.resolve_write(normalized_child.clone()).unwrap(),
            normalized_child
        );

        fs::remove_dir_all(regular_root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn containment_prefix_follows_windows_case_policy() {
        assert!(path_starts_with_host_root(
            Path::new(r"C:\WORKSPACE\src\Module.bsl"),
            Path::new(r"c:\workspace")
        ));
    }

    #[cfg(windows)]
    #[test]
    fn extended_length_unc_prefix_is_stripped_without_filesystem_access() {
        use std::path::PathBuf;

        let extended = PathBuf::from(r"\\?\UNC\server\share\source");

        assert_eq!(
            PathBuf::from(r"\\server\share\source"),
            strip_windows_extended_length_prefix(&extended)
        );
    }

    #[cfg(windows)]
    #[test]
    fn raw_windows_move_primitives_support_extended_length_paths() {
        use super::{rename_no_replace, replace_file_atomically};

        let base = unique_temp_root("long-moves");
        let mut root = base.clone();
        while root.display().to_string().len() < 270 {
            root.push("long-path-segment");
        }
        fs::create_dir_all(&root).unwrap();

        let replacement_stage = root.join("replacement-stage");
        let replacement_target = root.join("replacement-target");
        fs::write(&replacement_stage, b"replacement").unwrap();
        fs::write(&replacement_target, b"original").unwrap();
        replace_file_atomically(&replacement_stage, &replacement_target).unwrap();
        assert_eq!(fs::read(&replacement_target).unwrap(), b"replacement");
        assert!(!replacement_stage.exists());

        let move_source = root.join("move-source");
        let move_target = root.join("move-target");
        fs::write(&move_source, b"moved").unwrap();
        rename_no_replace(&move_source, &move_target).unwrap();
        assert_eq!(fs::read(&move_target).unwrap(), b"moved");
        assert!(!move_source.exists());

        fs::remove_dir_all(base).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn extended_length_and_regular_paths_have_same_identity() {
        use crate::infrastructure::source_roots::normalize_path_identity;
        use std::path::PathBuf;
        use std::time::{SystemTime, UNIX_EPOCH};

        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "unica-path-identity-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let regular = normalize_path_identity(&root).unwrap();
        let extended = PathBuf::from(format!(r"\\?\{}", root.display()));

        assert_eq!(regular, normalize_path_identity(&extended).unwrap());

        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(windows)]
    #[test]
    fn preserves_non_drive_verbatim_path_namespaces() {
        use crate::infrastructure::source_roots::normalize_path_identity;
        use std::path::PathBuf;

        let verbatim = PathBuf::from(r"\\?\Volume{01234567-89ab-cdef-0123-456789abcdef}\source");

        assert_eq!(verbatim, normalize_path_identity(&verbatim).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn source_root_policy_rejects_parent_traversal_after_directory_symlink() {
        use crate::infrastructure::source_roots::normalize_contained_source_root;
        use std::os::unix::fs::symlink;
        use std::time::{SystemTime, UNIX_EPOCH};

        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let workspace = std::env::temp_dir().join(format!(
            "unica-source-roots-parent-workspace-{}-{nanos}",
            std::process::id()
        ));
        let outside = std::env::temp_dir().join(format!(
            "unica-source-roots-parent-outside-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        symlink(&outside, workspace.join("external")).unwrap();

        let error =
            normalize_contained_source_root(&workspace, workspace.join("external/../escaped-new"))
                .unwrap_err();

        assert!(error.contains("sourceDir must be inside workspace root"));
        let _ = std::fs::remove_dir_all(workspace);
        let _ = std::fs::remove_dir_all(outside);
    }
}
