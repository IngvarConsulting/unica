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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EffectiveTokenSource {
    Thread,
    Process,
}

#[cfg(windows)]
fn verify_thread_token_fallback_error(error: u32) -> io::Result<()> {
    use windows_sys::Win32::Foundation::ERROR_NO_TOKEN;

    if error == ERROR_NO_TOKEN {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(error as i32))
    }
}

#[cfg(windows)]
#[allow(
    dead_code,
    reason = "DirectoryAnchor callers are introduced by the following Windows full-dump task"
)]
struct ProcessToken {
    handle: windows_sys::Win32::Foundation::HANDLE,
    user: Vec<u8>,
    source: EffectiveTokenSource,
}

#[cfg(windows)]
impl ProcessToken {
    fn current_user() -> io::Result<Self> {
        use std::ptr;
        use windows_sys::Win32::Foundation::{CloseHandle, ERROR_NO_TOKEN};
        use windows_sys::Win32::Security::{GetTokenInformation, TokenUser, TOKEN_QUERY};
        use windows_sys::Win32::System::Threading::{
            GetCurrentProcess, GetCurrentThread, OpenProcessToken, OpenThreadToken,
        };

        let mut handle = ptr::null_mut();
        // SAFETY: GetCurrentThread returns a valid pseudo-handle; handle is writable storage and
        // OpenAsSelf is true so an impersonating caller can inspect its effective token.
        let source =
            if unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, 1, &mut handle) } != 0 {
                EffectiveTokenSource::Thread
            } else {
                let error = io::Error::last_os_error();
                let error_code = error.raw_os_error().unwrap_or_default() as u32;
                verify_thread_token_fallback_error(error_code)?;
                debug_assert_eq!(error_code, ERROR_NO_TOKEN);
                // SAFETY: GetCurrentProcess returns a valid pseudo-handle; handle is writable storage.
                if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut handle) } == 0 {
                    return Err(io::Error::last_os_error());
                }
                EffectiveTokenSource::Process
            };

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

        Ok(Self {
            handle,
            user,
            source,
        })
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

        // SAFETY: self.handle is an owned token handle returned by OpenThreadToken or
        // OpenProcessToken.
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

    fn security_descriptor(&self) -> windows_sys::Win32::Security::PSECURITY_DESCRIPTOR {
        self.security_descriptor
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WindowsImmutableAclProfile {
    Ancestry,
    Installation,
}

#[cfg(windows)]
struct SelfRelativeSecurityDescriptor {
    storage: Vec<usize>,
    length: usize,
}

#[cfg(windows)]
impl SelfRelativeSecurityDescriptor {
    fn capture(descriptor: windows_sys::Win32::Security::PSECURITY_DESCRIPTOR) -> io::Result<Self> {
        use std::mem::size_of;
        use std::ptr;
        use windows_sys::Win32::Security::{
            GetSecurityDescriptorControl, GetSecurityDescriptorLength, IsValidSecurityDescriptor,
            MakeSelfRelativeSD, SE_SELF_RELATIVE,
        };

        if descriptor.is_null() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "security descriptor is absent",
            ));
        }
        // SAFETY: descriptor is non-null; callers retain the allocation for this call.
        if unsafe { IsValidSecurityDescriptor(descriptor) } == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "security descriptor is malformed",
            ));
        }
        let mut control = 0;
        let mut revision = 0;
        // SAFETY: descriptor was validated and both output pointers are writable.
        if unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) } == 0 {
            return Err(io::Error::last_os_error());
        }

        if control & SE_SELF_RELATIVE != 0 {
            // SAFETY: descriptor is a valid self-relative descriptor.
            let length = unsafe { GetSecurityDescriptorLength(descriptor) } as usize;
            if length == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "security descriptor has zero length",
                ));
            }
            let mut storage = vec![0usize; length.div_ceil(size_of::<usize>())];
            // SAFETY: storage has at least length writable bytes and descriptor has that many
            // readable bytes according to GetSecurityDescriptorLength.
            unsafe {
                ptr::copy_nonoverlapping(
                    descriptor.cast::<u8>(),
                    storage.as_mut_ptr().cast::<u8>(),
                    length,
                )
            };
            return Ok(Self { storage, length });
        }

        let mut length = 0;
        // SAFETY: this size query reads the validated absolute descriptor and writes length.
        unsafe { MakeSelfRelativeSD(descriptor, ptr::null_mut(), &mut length) };
        if length == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut storage = vec![0usize; (length as usize).div_ceil(size_of::<usize>())];
        // SAFETY: storage is aligned and has at least length writable bytes.
        if unsafe { MakeSelfRelativeSD(descriptor, storage.as_mut_ptr().cast(), &mut length) } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            storage,
            length: length as usize,
        })
    }

    fn as_ptr(&self) -> windows_sys::Win32::Security::PSECURITY_DESCRIPTOR {
        self.storage.as_ptr().cast_mut().cast()
    }

    fn as_bytes(&self) -> &[u8] {
        // SAFETY: storage owns at least length initialized bytes copied or written above.
        unsafe { std::slice::from_raw_parts(self.storage.as_ptr().cast::<u8>(), self.length) }
    }
}

#[cfg(windows)]
pub(crate) struct WindowsImmutableEntryEvidence {
    pub(crate) identity: FileIdentity,
    pub(crate) security_descriptor_sha256: [u8; 32],
    descriptor: SelfRelativeSecurityDescriptor,
}

#[cfg(windows)]
impl WindowsImmutableEntryEvidence {
    pub(crate) fn verify(&self, profile: WindowsImmutableAclProfile) -> io::Result<()> {
        verify_windows_immutable_security_descriptor(self.descriptor.as_ptr(), profile)
    }
}

#[cfg(windows)]
pub(crate) fn capture_windows_immutable_entry_evidence(
    file: &fs::File,
) -> io::Result<WindowsImmutableEntryEvidence> {
    use std::os::windows::io::AsRawHandle;
    use std::ptr;
    use windows_sys::Win32::Security::Authorization::{GetSecurityInfo, SE_FILE_OBJECT};
    use windows_sys::Win32::Security::{DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION};

    let mut descriptor = ptr::null_mut();
    // SAFETY: file owns a valid handle and descriptor is writable output storage.
    let status = unsafe {
        GetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            &mut descriptor,
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    let descriptor = LocalSecurityDescriptor(descriptor);
    let descriptor = SelfRelativeSecurityDescriptor::capture(descriptor.0)?;
    use sha2::{Digest, Sha256};
    Ok(WindowsImmutableEntryEvidence {
        identity: file_identity(file)?,
        security_descriptor_sha256: Sha256::digest(descriptor.as_bytes()).into(),
        descriptor,
    })
}

#[cfg(windows)]
fn sid_string(sid: windows_sys::Win32::Security::PSID) -> io::Result<String> {
    use std::ptr;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;

    let mut text = ptr::null_mut();
    // SAFETY: the caller supplies a validated SID and text is writable output storage.
    if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the API returned a live NUL-terminated UTF-16 allocation.
    let length = unsafe {
        let mut length = 0;
        while *text.add(length) != 0 {
            length += 1;
        }
        length
    };
    // SAFETY: length ends before the allocation's terminating NUL.
    let value = String::from_utf16(unsafe { std::slice::from_raw_parts(text, length) })
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error));
    // SAFETY: ConvertSidToStringSidW allocates with LocalAlloc.
    unsafe { LocalFree(text.cast()) };
    value
}

#[cfg(windows)]
fn windows_sid_is_trusted(sid: windows_sys::Win32::Security::PSID) -> io::Result<bool> {
    const TRUSTED_INSTALLER: &str =
        "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464";
    Ok(matches!(
        sid_string(sid)?.as_str(),
        TRUSTED_INSTALLER | "S-1-5-18" | "S-1-5-32-544"
    ))
}

#[cfg(windows)]
fn windows_immutable_mutation_mask(profile: WindowsImmutableAclProfile) -> u32 {
    use windows_sys::Win32::Foundation::{GENERIC_ALL, GENERIC_WRITE};
    use windows_sys::Win32::Storage::FileSystem::{
        DELETE, FILE_ADD_FILE, FILE_ADD_SUBDIRECTORY, FILE_DELETE_CHILD, FILE_WRITE_ATTRIBUTES,
        FILE_WRITE_DATA, FILE_WRITE_EA, WRITE_DAC, WRITE_OWNER,
    };

    let substitution =
        DELETE | FILE_DELETE_CHILD | WRITE_DAC | WRITE_OWNER | GENERIC_WRITE | GENERIC_ALL;
    match profile {
        WindowsImmutableAclProfile::Ancestry => substitution,
        WindowsImmutableAclProfile::Installation => {
            substitution
                | FILE_WRITE_DATA
                | FILE_ADD_FILE
                | FILE_ADD_SUBDIRECTORY
                | FILE_WRITE_EA
                | FILE_WRITE_ATTRIBUTES
        }
    }
}

#[cfg(windows)]
pub(crate) fn verify_windows_immutable_security_descriptor(
    descriptor: windows_sys::Win32::Security::PSECURITY_DESCRIPTOR,
    profile: WindowsImmutableAclProfile,
) -> io::Result<()> {
    use std::mem::{offset_of, size_of};
    use std::ptr;
    use windows_sys::Win32::Security::{
        AclSizeInformation, GetAce, GetAclInformation, GetLengthSid, GetSecurityDescriptorDacl,
        GetSecurityDescriptorOwner, IsValidAcl, IsValidSecurityDescriptor, IsValidSid,
        ACCESS_ALLOWED_ACE, ACE_HEADER, ACL, ACL_SIZE_INFORMATION,
    };

    const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;
    const ACCESS_DENIED_ACE_TYPE: u8 = 1;
    const VALID_INHERIT_FLAGS: u8 = 0x1f;
    const INHERIT_ONLY_ACE: u8 = 0x08;

    if descriptor.is_null() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "security descriptor is absent",
        ));
    }
    // SAFETY: descriptor is non-null and remains live for this function.
    if unsafe { IsValidSecurityDescriptor(descriptor) } == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "security descriptor is malformed",
        ));
    }

    let mut owner = ptr::null_mut();
    let mut owner_defaulted = 0;
    // SAFETY: descriptor was validated and output pointers are writable.
    if unsafe { GetSecurityDescriptorOwner(descriptor, &mut owner, &mut owner_defaulted) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if owner.is_null() || unsafe { IsValidSid(owner) } == 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "immutable platform owner SID is absent or malformed",
        ));
    }
    if !windows_sid_is_trusted(owner)? {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "immutable platform owner SID is not trusted",
        ));
    }

    let mut dacl_present = 0;
    let mut dacl: *mut ACL = ptr::null_mut();
    let mut dacl_defaulted = 0;
    // SAFETY: descriptor was validated and output pointers are writable.
    if unsafe {
        GetSecurityDescriptorDacl(
            descriptor,
            &mut dacl_present,
            &mut dacl,
            &mut dacl_defaulted,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    if dacl_present == 0 || dacl.is_null() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "immutable platform security descriptor has an absent or null DACL",
        ));
    }
    // SAFETY: dacl is non-null and points inside the validated descriptor.
    if unsafe { IsValidAcl(dacl) } == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "immutable platform DACL is malformed",
        ));
    }

    let mut information = ACL_SIZE_INFORMATION {
        AceCount: 0,
        AclBytesInUse: 0,
        AclBytesFree: 0,
    };
    // SAFETY: dacl is valid and information is writable.
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

    let mutation_mask = windows_immutable_mutation_mask(profile);
    for index in 0..information.AceCount {
        let mut ace = ptr::null_mut();
        // SAFETY: index is within the validated ACL's reported ACE count.
        if unsafe { GetAce(dacl, index, &mut ace) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: GetAce returned a pointer to a complete ACE header in the validated ACL.
        let header = unsafe { &*ace.cast::<ACE_HEADER>() };
        if header.AceFlags & !VALID_INHERIT_FLAGS != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "immutable platform DACL contains unsupported ACE flags",
            ));
        }
        if !matches!(
            header.AceType,
            ACCESS_ALLOWED_ACE_TYPE | ACCESS_DENIED_ACE_TYPE
        ) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "immutable platform DACL contains an unsupported ACE type",
            ));
        }

        let sid_offset = offset_of!(ACCESS_ALLOWED_ACE, SidStart);
        let minimum_sid_bytes = 8usize;
        let ace_size = usize::from(header.AceSize);
        if ace_size < sid_offset + minimum_sid_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "immutable platform DACL contains a truncated ACE SID",
            ));
        }
        // SAFETY: the fixed ACE header and minimum SID were bounds-checked against AceSize.
        let allowed = unsafe { &*ace.cast::<ACCESS_ALLOWED_ACE>() };
        let sid: windows_sys::Win32::Security::PSID =
            (&raw const allowed.SidStart).cast_mut().cast();
        // SAFETY: the minimum SID header is contained in this ACE.
        let subauthority_count = unsafe { *sid.cast::<u8>().add(1) } as usize;
        let sid_length = minimum_sid_bytes
            .checked_add(
                subauthority_count
                    .checked_mul(size_of::<u32>())
                    .ok_or_else(|| {
                        io::Error::new(io::ErrorKind::InvalidData, "ACE SID length overflowed")
                    })?,
            )
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "ACE SID length overflowed")
            })?;
        if sid_offset + sid_length > ace_size {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "immutable platform DACL contains an out-of-bounds ACE SID",
            ));
        }
        // SAFETY: the SID is fully contained in the ACE according to its own length field.
        if unsafe { IsValidSid(sid) } == 0 || unsafe { GetLengthSid(sid) } as usize != sid_length {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "immutable platform DACL contains a malformed ACE SID",
            ));
        }

        if header.AceType == ACCESS_DENIED_ACE_TYPE || header.AceFlags & INHERIT_ONLY_ACE != 0 {
            continue;
        }
        if allowed.Mask & mutation_mask != 0 && !windows_sid_is_trusted(sid)? {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "immutable platform DACL grants mutation rights 0x{:08x} to an untrusted SID",
                    allowed.Mask & mutation_mask
                ),
            ));
        }
    }
    Ok(())
}

#[cfg(windows)]
pub(crate) fn verify_windows_elevation_value(is_elevated: u32) -> io::Result<()> {
    if is_elevated == 0 {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "immutable platform execution is refused for an elevated Windows caller",
        ))
    }
}

#[cfg(windows)]
pub(crate) fn verify_unprivileged_windows_platform_caller() -> io::Result<()> {
    use std::mem::size_of;
    use windows_sys::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION};

    let token = ProcessToken::current_user()?;
    let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
    let mut returned = 0;
    // SAFETY: token is a live process-token handle, elevation is writable storage of the exact
    // requested type, and returned is writable.
    if unsafe {
        GetTokenInformation(
            token.handle,
            TokenElevation,
            (&mut elevation as *mut TOKEN_ELEVATION).cast(),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    if returned != size_of::<TOKEN_ELEVATION>() as u32 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Windows token elevation proof returned an unexpected size",
        ));
    }
    verify_windows_elevation_value(elevation.TokenIsElevated)
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
        CreateFileW, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_LIST_DIRECTORY,
        FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
        READ_CONTROL,
    };

    let path = windows_api_path(path)?;
    // SAFETY: path is NUL-terminated and all scalar arguments are documented Win32 flags.
    let handle = unsafe {
        CreateFileW(
            path.as_ptr(),
            FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES | READ_CONTROL,
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
    let attributes = windows_file_information(&file)?.dwFileAttributes;
    if attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "directory path resolves to a reparse point",
        ));
    }
    if attributes & FILE_ATTRIBUTE_DIRECTORY == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "directory path does not resolve to a directory",
        ));
    }
    Ok(file)
}

#[cfg(windows)]
#[repr(C)]
struct NtUnicodeString {
    length: u16,
    maximum_length: u16,
    buffer: windows_sys::core::PWSTR,
}

#[cfg(windows)]
#[repr(C)]
struct NtObjectAttributes {
    length: u32,
    root_directory: windows_sys::Win32::Foundation::HANDLE,
    object_name: *mut NtUnicodeString,
    attributes: u32,
    security_descriptor: *mut std::ffi::c_void,
    security_quality_of_service: *mut std::ffi::c_void,
}

#[cfg(windows)]
#[repr(C)]
union NtIoStatusStatus {
    status: i32,
    pointer: *mut std::ffi::c_void,
}

#[cfg(windows)]
#[repr(C)]
struct NtIoStatusBlock {
    status: NtIoStatusStatus,
    information: usize,
}

#[cfg(windows)]
#[repr(C)]
struct NtFileFsDeviceInformation {
    device_type: u32,
    characteristics: u32,
}

#[cfg(windows)]
unsafe extern "system" {
    fn NtCreateFile(
        file_handle: *mut windows_sys::Win32::Foundation::HANDLE,
        desired_access: u32,
        object_attributes: *mut NtObjectAttributes,
        io_status_block: *mut NtIoStatusBlock,
        allocation_size: *mut i64,
        file_attributes: u32,
        share_access: u32,
        create_disposition: u32,
        create_options: u32,
        ea_buffer: *mut std::ffi::c_void,
        ea_length: u32,
    ) -> i32;
    fn NtSetInformationFile(
        file_handle: windows_sys::Win32::Foundation::HANDLE,
        io_status_block: *mut NtIoStatusBlock,
        file_information: *mut std::ffi::c_void,
        length: u32,
        file_information_class: u32,
    ) -> i32;
    fn NtQueryVolumeInformationFile(
        file_handle: windows_sys::Win32::Foundation::HANDLE,
        io_status_block: *mut NtIoStatusBlock,
        fs_information: *mut std::ffi::c_void,
        length: u32,
        fs_information_class: u32,
    ) -> i32;
    fn RtlNtStatusToDosError(status: i32) -> u32;
}

#[cfg(windows)]
fn nt_status_error(status: i32) -> io::Error {
    // SAFETY: RtlNtStatusToDosError accepts all NTSTATUS values and returns a Win32 error code.
    let error = unsafe { RtlNtStatusToDosError(status) };
    io::Error::from_raw_os_error(error as i32)
}

#[cfg(windows)]
fn verify_windows_local_fixed_device_info(
    device_type: u32,
    characteristics: u32,
) -> io::Result<()> {
    const FILE_DEVICE_DISK: u32 = 0x0000_0007;
    const FILE_REMOTE_DEVICE: u32 = 0x0000_0010;
    const FILE_REMOVABLE_MEDIA: u32 = 0x0000_0001;
    const FILE_FLOPPY_DISKETTE: u32 = 0x0000_0004;
    const FILE_WRITE_ONCE_MEDIA: u32 = 0x0000_0008;
    const FILE_VIRTUAL_VOLUME: u32 = 0x0000_0040;
    const FILE_PORTABLE_DEVICE: u32 = 0x0000_4000;
    const FILE_REMOTE_DEVICE_VSMB: u32 = 0x0008_0000;
    const UNTRUSTED_CHARACTERISTICS: u32 = FILE_REMOTE_DEVICE
        | FILE_REMOVABLE_MEDIA
        | FILE_FLOPPY_DISKETTE
        | FILE_WRITE_ONCE_MEDIA
        | FILE_VIRTUAL_VOLUME
        | FILE_PORTABLE_DEVICE
        | FILE_REMOTE_DEVICE_VSMB;

    if device_type != FILE_DEVICE_DISK || characteristics & UNTRUSTED_CHARACTERISTICS != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "immutable platform volume is not a local fixed disk (device type 0x{device_type:08x}, characteristics 0x{characteristics:08x})"
            ),
        ));
    }
    Ok(())
}

#[cfg(windows)]
pub(crate) fn verify_windows_local_fixed_volume(file: &fs::File) -> io::Result<()> {
    use std::mem::size_of;
    use std::os::windows::io::AsRawHandle;

    const FILE_FS_DEVICE_INFORMATION_CLASS: u32 = 4;
    let mut io_status = NtIoStatusBlock {
        status: NtIoStatusStatus { status: 0 },
        information: 0,
    };
    let mut information = NtFileFsDeviceInformation {
        device_type: 0,
        characteristics: 0,
    };
    // SAFETY: file retains a valid handle for the complete call; io_status and information are
    // writable buffers of the exact native layouts for FileFsDeviceInformation.
    let status = unsafe {
        NtQueryVolumeInformationFile(
            file.as_raw_handle(),
            &mut io_status,
            (&mut information as *mut NtFileFsDeviceInformation).cast(),
            size_of::<NtFileFsDeviceInformation>() as u32,
            FILE_FS_DEVICE_INFORMATION_CLASS,
        )
    };
    if status < 0 {
        return Err(nt_status_error(status));
    }
    if io_status.information != size_of::<NtFileFsDeviceInformation>() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Windows volume device proof returned an unexpected size",
        ));
    }
    verify_windows_local_fixed_device_info(information.device_type, information.characteristics)
}

#[cfg(windows)]
fn relative_child_name(name: &std::ffi::OsStr) -> io::Result<Vec<u16>> {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Component;

    let mut components = Path::new(name).components();
    if !matches!(components.next(), Some(Component::Normal(component)) if component == name)
        || components.next().is_some()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "directory child name must be exactly one normal relative component",
        ));
    }
    let wide = name.encode_wide().collect::<Vec<_>>();
    if wide.is_empty()
        || wide.iter().any(|unit| {
            *unit == 0 || *unit == b'\\' as u16 || *unit == b'/' as u16 || *unit == b':' as u16
        })
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "directory child name must not contain separators, a stream name, or NUL",
        ));
    }
    Ok(wide)
}

#[cfg(windows)]
fn validate_directory_handle(file: &fs::File) -> io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
    };

    let attributes = windows_file_information(&file)?.dwFileAttributes;
    if attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "directory child resolves to a reparse point",
        ));
    }
    if attributes & FILE_ATTRIBUTE_DIRECTORY == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "directory child does not resolve to a directory",
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn nt_create_options_for_std_file(desired_access: u32, create_options: u32) -> io::Result<u32> {
    use windows_sys::Win32::Foundation::{
        GENERIC_ALL, GENERIC_EXECUTE, GENERIC_READ, GENERIC_WRITE,
    };
    use windows_sys::Win32::Storage::FileSystem::SYNCHRONIZE;

    const FILE_SYNCHRONOUS_IO_NONALERT: u32 = 0x0000_0020;
    let generic_synchronous_access = GENERIC_ALL | GENERIC_EXECUTE | GENERIC_READ | GENERIC_WRITE;
    if desired_access & SYNCHRONIZE == 0 && desired_access & generic_synchronous_access == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "std::fs::File native handles require SYNCHRONIZE-compatible access",
        ));
    }
    Ok(create_options | FILE_SYNCHRONOUS_IO_NONALERT)
}

#[cfg(windows)]
fn open_relative_child(
    parent: &fs::File,
    name: &std::ffi::OsStr,
    desired_access: u32,
    file_attributes: u32,
    create_disposition: u32,
    create_options: u32,
    security_descriptor: Option<windows_sys::Win32::Security::PSECURITY_DESCRIPTOR>,
) -> io::Result<fs::File> {
    use std::mem::size_of;
    use std::os::windows::io::{AsRawHandle, FromRawHandle};
    use std::ptr;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };

    const OBJ_CASE_INSENSITIVE: u32 = 0x40;
    let create_options = nt_create_options_for_std_file(desired_access, create_options)?;
    let mut name = relative_child_name(name)?;
    let byte_length = name
        .len()
        .checked_mul(size_of::<u16>())
        .and_then(|length| u16::try_from(length).ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "child name is too long"))?;
    let mut object_name = NtUnicodeString {
        length: byte_length,
        maximum_length: byte_length,
        buffer: name.as_mut_ptr(),
    };
    let mut attributes = NtObjectAttributes {
        length: size_of::<NtObjectAttributes>() as u32,
        root_directory: parent.as_raw_handle(),
        object_name: &mut object_name,
        attributes: OBJ_CASE_INSENSITIVE,
        security_descriptor: security_descriptor.unwrap_or(ptr::null_mut()).cast(),
        security_quality_of_service: ptr::null_mut(),
    };
    let mut status = NtIoStatusBlock {
        status: NtIoStatusStatus { status: 0 },
        information: 0,
    };
    let mut handle = ptr::null_mut();
    // SAFETY: the root handle is borrowed from parent, and all pointers refer to valid mutable
    // storage for the duration of NtCreateFile. The child name is validated as one relative
    // component, so RootDirectory binds the operation to the retained parent handle.
    let result = unsafe {
        NtCreateFile(
            &mut handle,
            desired_access,
            &mut attributes,
            &mut status,
            ptr::null_mut(),
            file_attributes,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            create_disposition,
            create_options,
            ptr::null_mut(),
            0,
        )
    };
    if result < 0 {
        return Err(nt_status_error(result));
    }
    // SAFETY: a non-error NtCreateFile result returns an owned file handle.
    Ok(unsafe { fs::File::from_raw_handle(handle) })
}

#[cfg(windows)]
pub(crate) fn open_directory_child_nofollow(
    parent: &fs::File,
    name: &std::ffi::OsStr,
) -> io::Result<fs::File> {
    const FILE_OPEN: u32 = 0x0000_0001;
    const FILE_DIRECTORY_FILE: u32 = 0x0000_0001;
    const FILE_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, READ_CONTROL, SYNCHRONIZE,
    };

    let file = open_relative_child(
        parent,
        name,
        FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES | READ_CONTROL | SYNCHRONIZE,
        0,
        FILE_OPEN,
        FILE_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT,
        None,
    )?;
    validate_directory_handle(&file)?;
    Ok(file)
}

#[cfg(windows)]
pub(crate) fn open_regular_child_nofollow(
    parent: &fs::File,
    name: &std::ffi::OsStr,
) -> io::Result<fs::File> {
    const FILE_OPEN: u32 = 0x0000_0001;
    const FILE_NON_DIRECTORY_FILE: u32 = 0x0000_0040;
    const FILE_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    use windows_sys::Win32::Foundation::GENERIC_READ;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_READ_ATTRIBUTES, SYNCHRONIZE,
    };

    let file = open_relative_child(
        parent,
        name,
        GENERIC_READ | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
        0,
        FILE_OPEN,
        FILE_NON_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT,
        None,
    )?;
    let attributes = windows_file_information(&file)?.dwFileAttributes;
    if attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "regular child resolves to a reparse point",
        ));
    }
    if attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "entry is not a regular file",
        ));
    }
    Ok(file)
}

#[cfg(windows)]
pub(crate) fn open_any_child_nofollow(
    parent: &fs::File,
    name: &std::ffi::OsStr,
) -> io::Result<fs::File> {
    const FILE_OPEN: u32 = 0x0000_0001;
    const FILE_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_READ_ATTRIBUTES, SYNCHRONIZE,
    };

    let file = open_relative_child(
        parent,
        name,
        FILE_READ_ATTRIBUTES | SYNCHRONIZE,
        0,
        FILE_OPEN,
        FILE_OPEN_REPARSE_POINT,
        None,
    )?;
    if windows_file_information(&file)?.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "child resolves to a reparse point",
        ));
    }
    Ok(file)
}

#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OpenedChildKind {
    Directory,
    RegularFile,
    ReparsePoint,
    Unsupported,
}

#[cfg(windows)]
pub(crate) fn opened_child_kind(file: &fs::File) -> io::Result<OpenedChildKind> {
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_DEVICE, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
    };

    let attributes = windows_file_information(file)?.dwFileAttributes;
    if attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        Ok(OpenedChildKind::ReparsePoint)
    } else if attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
        Ok(OpenedChildKind::Directory)
    } else if attributes & FILE_ATTRIBUTE_DEVICE != 0 {
        Ok(OpenedChildKind::Unsupported)
    } else {
        Ok(OpenedChildKind::RegularFile)
    }
}

#[cfg(windows)]
pub(crate) fn open_any_child_for_delete(
    parent: &fs::File,
    name: &std::ffi::OsStr,
) -> io::Result<fs::File> {
    const FILE_OPEN: u32 = 0x0000_0001;
    const FILE_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    use windows_sys::Win32::Storage::FileSystem::{DELETE, FILE_READ_ATTRIBUTES, SYNCHRONIZE};

    open_relative_child(
        parent,
        name,
        DELETE | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
        0,
        FILE_OPEN,
        FILE_OPEN_REPARSE_POINT,
        None,
    )
}

#[cfg(windows)]
pub(crate) fn delete_open_child(file: &fs::File) -> io::Result<()> {
    set_delete_disposition(file)
}

#[cfg(windows)]
fn directory_query_is_end(restart: bool, error: &io::Error) -> bool {
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_NO_MORE_FILES};

    matches!(
        error.raw_os_error(),
        Some(code)
            if code == ERROR_NO_MORE_FILES as i32
                || restart && code == ERROR_FILE_NOT_FOUND as i32
    )
}

#[cfg(windows)]
fn parse_directory_information_buffer(
    buffer: &[u8],
    names: &mut Vec<std::ffi::OsString>,
) -> io::Result<()> {
    use std::mem::{offset_of, size_of};
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_ID_BOTH_DIR_INFO;

    let file_name_offset = offset_of!(FILE_ID_BOTH_DIR_INFO, FileName);
    let complete_header_size = size_of::<FILE_ID_BOTH_DIR_INFO>();
    let mut offset = 0usize;
    loop {
        let complete_header_end = offset.checked_add(complete_header_size).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "directory entry complete header offset overflowed",
            )
        })?;
        if complete_header_end > buffer.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "directory entry complete header exceeds the enumeration buffer",
            ));
        }
        // SAFETY: the complete Rust structure was bounds-checked above; read_unaligned accepts
        // every record offset supplied by the filesystem.
        let entry = unsafe {
            std::ptr::read_unaligned(buffer.as_ptr().add(offset).cast::<FILE_ID_BOTH_DIR_INFO>())
        };
        let name_bytes = entry.FileNameLength as usize;
        if name_bytes == 0 || name_bytes % size_of::<u16>() != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "directory entry has an invalid UTF-16 name length",
            ));
        }
        let name_start = offset.checked_add(file_name_offset).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "directory entry name offset overflowed",
            )
        })?;
        let name_end = name_start.checked_add(name_bytes).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "directory entry name offset overflowed",
            )
        })?;
        if name_end > buffer.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "directory entry name exceeds the enumeration buffer",
            ));
        }
        let mut name = Vec::with_capacity(name_bytes / size_of::<u16>());
        for unit_offset in (name_start..name_end).step_by(size_of::<u16>()) {
            // SAFETY: every two-byte unit lies within the checked name range; read_unaligned
            // accepts the byte buffer's alignment.
            name.push(unsafe {
                std::ptr::read_unaligned(buffer.as_ptr().add(unit_offset).cast::<u16>())
            });
        }
        let name = std::ffi::OsString::from_wide(&name);
        if name != "." && name != ".." {
            names.push(name);
        }

        let next = entry.NextEntryOffset as usize;
        if next == 0 {
            break;
        }
        let minimum_record_bytes = file_name_offset.checked_add(name_bytes).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "directory entry record length overflowed",
            )
        })?;
        if next < minimum_record_bytes || next % 8 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "directory entry next-record offset is not 8-byte-aligned or overlaps the name",
            ));
        }
        offset = offset.checked_add(next).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "directory entry offset overflowed",
            )
        })?;
        if offset >= buffer.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "directory entry offset exceeds the enumeration buffer",
            ));
        }
    }
    Ok(())
}

#[cfg(windows)]
#[allow(
    dead_code,
    reason = "the following Windows full-dump tree-inspection task consumes retained enumeration"
)]
pub(crate) fn read_directory_names(directory: &fs::File) -> io::Result<Vec<std::ffi::OsString>> {
    use std::mem::size_of;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        FileIdBothDirectoryInfo, FileIdBothDirectoryRestartInfo, GetFileInformationByHandleEx,
    };

    const BUFFER_BYTES: usize = 64 * 1024;

    validate_directory_handle(directory)?;
    let word_count = BUFFER_BYTES.div_ceil(size_of::<usize>());
    let mut storage = vec![0usize; word_count];
    let buffer_bytes = storage.len() * size_of::<usize>();
    let mut names = Vec::new();
    let mut restart = true;
    loop {
        storage.fill(0);
        let information_class = if restart {
            FileIdBothDirectoryRestartInfo
        } else {
            FileIdBothDirectoryInfo
        };
        // SAFETY: storage is writable and pointer-aligned, its byte capacity is supplied exactly,
        // and directory remains a live retained handle throughout enumeration.
        let succeeded = unsafe {
            GetFileInformationByHandleEx(
                directory.as_raw_handle(),
                information_class,
                storage.as_mut_ptr().cast(),
                u32::try_from(buffer_bytes).expect("fixed directory buffer fits u32"),
            )
        };
        if succeeded == 0 {
            let error = io::Error::last_os_error();
            if directory_query_is_end(restart, &error) {
                break;
            }
            return Err(error);
        }
        restart = false;

        // SAFETY: storage owns buffer_bytes initialized bytes for this query. The parser performs
        // complete structure, name, and next-record bounds checks before reading each field.
        let buffer =
            unsafe { std::slice::from_raw_parts(storage.as_ptr().cast::<u8>(), buffer_bytes) };
        parse_directory_information_buffer(buffer, &mut names)?;
    }
    names.sort();
    Ok(names)
}

#[cfg(windows)]
pub(crate) fn open_directory_child_for_rename(
    parent: &fs::File,
    name: &std::ffi::OsStr,
) -> io::Result<fs::File> {
    const FILE_OPEN: u32 = 0x0000_0001;
    const FILE_DIRECTORY_FILE: u32 = 0x0000_0001;
    const FILE_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    use windows_sys::Win32::Storage::FileSystem::{
        DELETE, FILE_READ_ATTRIBUTES, READ_CONTROL, SYNCHRONIZE,
    };

    let file = open_relative_child(
        parent,
        name,
        DELETE | FILE_READ_ATTRIBUTES | READ_CONTROL | SYNCHRONIZE,
        0,
        FILE_OPEN,
        FILE_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT,
        None,
    )?;
    validate_directory_handle(&file)?;
    Ok(file)
}

#[cfg(windows)]
pub(crate) fn rename_directory_handle_child_no_replace(
    source: &fs::File,
    destination_parent: &fs::File,
    destination_name: &std::ffi::OsStr,
) -> io::Result<()> {
    use std::mem::{offset_of, size_of};
    use std::os::windows::io::AsRawHandle;
    use std::ptr;

    #[repr(C)]
    union RenameFlags {
        replace_if_exists: u8,
        flags: u32,
    }

    #[repr(C)]
    struct RenameInformation {
        anonymous: RenameFlags,
        root_directory: windows_sys::Win32::Foundation::HANDLE,
        file_name_length: u32,
        file_name: [u16; 1],
    }

    const FILE_RENAME_INFORMATION_CLASS: u32 = 10;

    let name = relative_child_name(destination_name)?;
    let name_bytes = name
        .len()
        .checked_mul(size_of::<u16>())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "child name is too long"))?;
    let information_length = offset_of!(RenameInformation, file_name)
        .checked_add(name_bytes)
        .and_then(|length| u32::try_from(length).ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "child name is too long"))?;
    let word_count = (information_length as usize).div_ceil(size_of::<usize>());
    let mut storage = vec![0usize; word_count];
    let information = storage.as_mut_ptr().cast::<RenameInformation>();
    // SAFETY: storage is pointer-aligned and large enough for the fixed header plus the complete
    // UTF-16 name. All fields are initialized before NtSetInformationFile reads the buffer.
    unsafe {
        ptr::addr_of_mut!((*information).anonymous).write(RenameFlags {
            replace_if_exists: 0,
        });
        ptr::addr_of_mut!((*information).root_directory).write(destination_parent.as_raw_handle());
        ptr::addr_of_mut!((*information).file_name_length).write(name_bytes as u32);
        ptr::copy_nonoverlapping(
            name.as_ptr(),
            ptr::addr_of_mut!((*information).file_name).cast::<u16>(),
            name.len(),
        );
    }
    let mut status = NtIoStatusBlock {
        status: NtIoStatusStatus { status: 0 },
        information: 0,
    };
    // SAFETY: source is an owned directory handle opened with DELETE access, destination_parent
    // remains live, and the initialized buffer describes one relative destination name.
    let result = unsafe {
        NtSetInformationFile(
            source.as_raw_handle(),
            &mut status,
            information.cast(),
            information_length,
            FILE_RENAME_INFORMATION_CLASS,
        )
    };
    if result < 0 {
        Err(nt_status_error(result))
    } else {
        Ok(())
    }
}

#[cfg(windows)]
pub(crate) fn create_owner_only_directory_child(
    parent: &fs::File,
    name: &std::ffi::OsStr,
) -> io::Result<fs::File> {
    const FILE_CREATE: u32 = 0x0000_0002;
    const FILE_DIRECTORY_FILE: u32 = 0x0000_0001;
    const FILE_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    use windows_sys::Win32::Storage::FileSystem::{
        DELETE, FILE_ATTRIBUTE_DIRECTORY, FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, READ_CONTROL,
        SYNCHRONIZE,
    };

    let security = OwnerOnlySecurityAttributes::current_user()?;
    let file = open_relative_child(
        parent,
        name,
        DELETE | FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES | READ_CONTROL | SYNCHRONIZE,
        FILE_ATTRIBUTE_DIRECTORY,
        FILE_CREATE,
        FILE_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT,
        Some(security.security_descriptor()),
    )?;
    if let Err(error) = validate_directory_handle(&file) {
        return match discard_created_child(&file) {
            Ok(()) => Err(error),
            Err(cleanup) => Err(io::Error::new(
                error.kind(),
                format!("{error}; failed to remove invalid created directory: {cleanup}"),
            )),
        };
    }
    Ok(file)
}

#[cfg(windows)]
pub(crate) fn create_owner_only_file_child(
    parent: &fs::File,
    name: &std::ffi::OsStr,
) -> io::Result<fs::File> {
    const FILE_CREATE: u32 = 0x0000_0002;
    const FILE_NON_DIRECTORY_FILE: u32 = 0x0000_0040;
    const FILE_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE};
    use windows_sys::Win32::Storage::FileSystem::{DELETE, FILE_ATTRIBUTE_NORMAL};

    let security = OwnerOnlySecurityAttributes::current_user()?;
    open_relative_child(
        parent,
        name,
        GENERIC_READ | GENERIC_WRITE | DELETE,
        FILE_ATTRIBUTE_NORMAL,
        FILE_CREATE,
        FILE_NON_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT,
        Some(security.security_descriptor()),
    )
}

#[cfg(windows)]
pub(crate) fn discard_created_child(file: &fs::File) -> io::Result<()> {
    set_delete_disposition(file)
}

#[cfg(windows)]
fn set_delete_disposition(file: &fs::File) -> io::Result<()> {
    use std::os::windows::io::AsRawHandle;

    const FILE_DISPOSITION_INFORMATION: u32 = 13;
    let mut delete_file = 1u8;
    let mut status = NtIoStatusBlock {
        status: NtIoStatusStatus { status: 0 },
        information: 0,
    };
    // SAFETY: file is an owned handle opened with DELETE access by the child-creation helpers,
    // and delete_file/status remain writable for the duration of the native call. Once marked,
    // the only subsequent operation on the handle is Drop/close.
    let result = unsafe {
        NtSetInformationFile(
            file.as_raw_handle(),
            &mut status,
            (&mut delete_file as *mut u8).cast(),
            std::mem::size_of_val(&delete_file) as u32,
            FILE_DISPOSITION_INFORMATION,
        )
    };
    if result < 0 {
        Err(nt_status_error(result))
    } else {
        Ok(())
    }
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
        AclSizeInformation, EqualSid, GetAce, GetAclInformation, GetSecurityDescriptorControl,
        ACCESS_ALLOWED_ACE, ACE_HEADER, ACL_SIZE_INFORMATION, DACL_SECURITY_INFORMATION,
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
    if header.AceType != ACCESS_ALLOWED_ACE_TYPE || u32::from(header.AceFlags) & INHERITED_ACE != 0
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
            capture_windows_immutable_entry_evidence, create_owner_only_directory,
            create_owner_only_directory_child, create_owner_only_file, directory_query_is_end,
            file_identity, nt_create_options_for_std_file, open_directory_child_nofollow,
            open_directory_nofollow, parse_directory_information_buffer, read_directory_names,
            verify_owner_only_acl, verify_thread_token_fallback_error,
            verify_windows_elevation_value, verify_windows_immutable_security_descriptor,
            verify_windows_local_fixed_device_info, verify_windows_local_fixed_volume,
            EffectiveTokenSource, ProcessToken, WindowsImmutableAclProfile,
        };
        use std::ffi::OsString;
        use std::mem::{offset_of, size_of};
        use std::ptr;
        use windows_sys::Win32::Foundation::{LocalFree, GENERIC_ALL, GENERIC_WRITE};
        use windows_sys::Win32::Foundation::{
            ERROR_ACCESS_DENIED, ERROR_CANT_OPEN_ANONYMOUS, ERROR_FILE_NOT_FOUND,
            ERROR_NO_MORE_FILES, ERROR_NO_TOKEN,
        };
        use windows_sys::Win32::Security::Authorization::{
            ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
        };
        use windows_sys::Win32::Security::PSECURITY_DESCRIPTOR;
        use windows_sys::Win32::Security::{ImpersonateSelf, RevertToSelf, SecurityImpersonation};
        use windows_sys::Win32::Storage::FileSystem::{
            DELETE, FILE_ADD_FILE, FILE_ADD_SUBDIRECTORY, FILE_DELETE_CHILD, FILE_ID_BOTH_DIR_INFO,
            FILE_WRITE_ATTRIBUTES, FILE_WRITE_DATA, FILE_WRITE_EA, WRITE_DAC, WRITE_OWNER,
        };

        struct TestSecurityDescriptor(PSECURITY_DESCRIPTOR);

        impl Drop for TestSecurityDescriptor {
            fn drop(&mut self) {
                // SAFETY: the SDDL conversion API allocated this descriptor with LocalAlloc.
                unsafe { LocalFree(self.0) };
            }
        }

        fn descriptor_from_sddl(sddl: &str) -> TestSecurityDescriptor {
            let mut wide = sddl.encode_utf16().collect::<Vec<_>>();
            wide.push(0);
            let mut descriptor = ptr::null_mut();
            // SAFETY: wide is NUL-terminated and descriptor is writable output storage.
            let converted = unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    wide.as_ptr(),
                    SDDL_REVISION_1,
                    &mut descriptor,
                    ptr::null_mut(),
                )
            };
            assert_ne!(converted, 0, "{}", io::Error::last_os_error());
            TestSecurityDescriptor(descriptor)
        }

        fn descriptor_with_untrusted_mask(
            owner: &str,
            mask: u32,
            flags: &str,
        ) -> TestSecurityDescriptor {
            descriptor_from_sddl(&format!(
                "O:{owner}D:(A;;FA;;;SY)(A;{flags};0x{mask:08x};;;BU)"
            ))
        }

        #[test]
        fn windows_immutable_platform_accepts_trusted_owners_and_read_execute_users() {
            const TRUSTED_INSTALLER: &str =
                "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464";
            for owner in [TRUSTED_INSTALLER, "SY", "BA"] {
                let descriptor = descriptor_from_sddl(&format!(
                    "O:{owner}D:(A;;FA;;;SY)(A;;FA;;;BA)(A;;0x001200a9;;;BU)"
                ));

                verify_windows_immutable_security_descriptor(
                    descriptor.0,
                    WindowsImmutableAclProfile::Installation,
                )
                .unwrap();
            }
        }

        #[test]
        fn windows_immutable_platform_rejects_an_untrusted_owner() {
            let descriptor = descriptor_from_sddl("O:BUD:(A;;FA;;;SY)(A;;FRFX;;;BU)");

            let error = verify_windows_immutable_security_descriptor(
                descriptor.0,
                WindowsImmutableAclProfile::Installation,
            )
            .unwrap_err();

            assert!(error.to_string().contains("owner"), "{error}");
        }

        #[test]
        fn windows_immutable_platform_ancestry_allows_sibling_creation_only() {
            let descriptor =
                descriptor_with_untrusted_mask("SY", FILE_ADD_FILE | FILE_ADD_SUBDIRECTORY, "");

            verify_windows_immutable_security_descriptor(
                descriptor.0,
                WindowsImmutableAclProfile::Ancestry,
            )
            .unwrap();
        }

        #[test]
        fn windows_immutable_platform_ancestry_rejects_untrusted_substitution_rights() {
            for mask in [
                DELETE,
                FILE_DELETE_CHILD,
                WRITE_DAC,
                WRITE_OWNER,
                GENERIC_WRITE,
                GENERIC_ALL,
            ] {
                let descriptor = descriptor_with_untrusted_mask("SY", mask, "");

                let error = verify_windows_immutable_security_descriptor(
                    descriptor.0,
                    WindowsImmutableAclProfile::Ancestry,
                )
                .unwrap_err();

                assert!(
                    error.to_string().contains("mutation"),
                    "mask 0x{mask:08x}: {error}"
                );
            }
        }

        #[test]
        fn windows_immutable_platform_inventory_rejects_every_untrusted_mutation_right() {
            for mask in [
                FILE_WRITE_DATA,
                FILE_ADD_FILE,
                FILE_ADD_SUBDIRECTORY,
                FILE_WRITE_EA,
                FILE_WRITE_ATTRIBUTES,
                FILE_DELETE_CHILD,
                DELETE,
                WRITE_DAC,
                WRITE_OWNER,
                GENERIC_WRITE,
                GENERIC_ALL,
            ] {
                let descriptor = descriptor_with_untrusted_mask("BA", mask, "");

                let error = verify_windows_immutable_security_descriptor(
                    descriptor.0,
                    WindowsImmutableAclProfile::Installation,
                )
                .unwrap_err();

                assert!(
                    error.to_string().contains("mutation"),
                    "mask 0x{mask:08x}: {error}"
                );
            }
        }

        #[test]
        fn windows_immutable_platform_ignores_inherit_only_capabilities_on_current_entry() {
            let descriptor = descriptor_with_untrusted_mask("SY", GENERIC_ALL, "OICIIO");

            verify_windows_immutable_security_descriptor(
                descriptor.0,
                WindowsImmutableAclProfile::Installation,
            )
            .unwrap();
        }

        #[test]
        fn windows_immutable_platform_rejects_missing_null_and_unsupported_dacls() {
            let absent = descriptor_from_sddl("O:SY");
            let null = descriptor_from_sddl("O:SYD:NO_ACCESS_CONTROL");
            let object =
                descriptor_from_sddl("O:SYD:(OA;;FA;00112233-4455-6677-8899-aabbccddeeff;;BU)");
            let mut malformed = [0usize; 4];

            for descriptor in [
                absent.0,
                null.0,
                object.0,
                malformed.as_mut_ptr().cast(),
                ptr::null_mut(),
            ] {
                assert!(verify_windows_immutable_security_descriptor(
                    descriptor,
                    WindowsImmutableAclProfile::Installation,
                )
                .is_err());
            }
        }

        #[test]
        fn windows_immutable_platform_rejects_an_elevated_caller() {
            let error = verify_windows_elevation_value(1).unwrap_err();

            assert!(error.to_string().contains("elevated"), "{error}");
            verify_windows_elevation_value(0).unwrap();
        }

        #[test]
        fn windows_immutable_platform_effective_token_falls_back_only_when_thread_has_no_token() {
            verify_thread_token_fallback_error(ERROR_NO_TOKEN).unwrap();

            for expected in [ERROR_ACCESS_DENIED, ERROR_CANT_OPEN_ANONYMOUS] {
                let error = verify_thread_token_fallback_error(expected).unwrap_err();
                assert_eq!(error.raw_os_error(), Some(expected as i32));
            }
        }

        #[test]
        fn windows_immutable_platform_effective_token_uses_process_token_without_impersonation() {
            let token = ProcessToken::current_user().unwrap();

            assert_eq!(token.source, EffectiveTokenSource::Process);
        }

        #[test]
        fn windows_immutable_platform_effective_token_selects_an_impersonation_token() {
            struct RevertGuard;

            impl Drop for RevertGuard {
                fn drop(&mut self) {
                    // SAFETY: this guard is created only after ImpersonateSelf succeeds.
                    assert_ne!(unsafe { RevertToSelf() }, 0);
                }
            }

            // SAFETY: SecurityImpersonation is a documented impersonation level.
            assert_ne!(
                unsafe { ImpersonateSelf(SecurityImpersonation) },
                0,
                "{}",
                io::Error::last_os_error()
            );
            let _guard = RevertGuard;

            let token = ProcessToken::current_user().unwrap();

            assert_eq!(token.source, EffectiveTokenSource::Thread);
        }

        #[test]
        fn windows_immutable_platform_rejects_nonlocal_or_nonfixed_device_information() {
            const FILE_DEVICE_CD_ROM: u32 = 0x0000_0002;
            const FILE_DEVICE_DISK: u32 = 0x0000_0007;
            const FILE_DEVICE_NETWORK_FILE_SYSTEM: u32 = 0x0000_0014;
            const FILE_DEVICE_VIRTUAL_DISK: u32 = 0x0000_0024;
            const FILE_PORTABLE_DEVICE: u32 = 0x0000_4000;
            const FILE_REMOTE_DEVICE: u32 = 0x0000_0010;
            const FILE_REMOTE_DEVICE_VSMB: u32 = 0x0008_0000;
            const FILE_REMOVABLE_MEDIA: u32 = 0x0000_0001;
            const FILE_DEVICE_IS_MOUNTED: u32 = 0x0000_0020;

            verify_windows_local_fixed_device_info(FILE_DEVICE_DISK, FILE_DEVICE_IS_MOUNTED)
                .unwrap();
            for (device_type, characteristics) in [
                (FILE_DEVICE_DISK, FILE_REMOTE_DEVICE),
                (FILE_DEVICE_DISK, FILE_REMOTE_DEVICE_VSMB),
                (FILE_DEVICE_DISK, FILE_PORTABLE_DEVICE),
                (FILE_DEVICE_DISK, FILE_REMOVABLE_MEDIA),
                (FILE_DEVICE_NETWORK_FILE_SYSTEM, FILE_DEVICE_IS_MOUNTED),
                (FILE_DEVICE_CD_ROM, FILE_DEVICE_IS_MOUNTED),
                (FILE_DEVICE_VIRTUAL_DISK, FILE_DEVICE_IS_MOUNTED),
                (0xffff_ffff, 0),
            ] {
                assert!(
                    verify_windows_local_fixed_device_info(device_type, characteristics).is_err(),
                    "device type 0x{device_type:08x}, characteristics 0x{characteristics:08x}"
                );
            }
        }

        #[test]
        fn windows_immutable_platform_accepts_a_real_local_fixed_volume_handle() {
            let root = unique_temp_root("immutable-local-volume");
            fs::create_dir_all(&root).unwrap();
            let handle = open_directory_nofollow(&root).unwrap();

            verify_windows_local_fixed_volume(&handle).unwrap();

            drop(handle);
            fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn windows_immutable_platform_attestation_handle_evidence_stays_bound_to_the_open_object() {
            let root = unique_temp_root("immutable-handle-evidence");
            fs::create_dir_all(&root).unwrap();
            let original = root.join("entry");
            let displaced = root.join("displaced");
            let original_handle = create_owner_only_file(&original).unwrap();
            let expected_identity = file_identity(&original_handle).unwrap();
            fs::rename(&original, &displaced).unwrap();
            fs::write(&original, b"decoy").unwrap();
            let decoy_handle = fs::File::open(&original).unwrap();

            let retained = capture_windows_immutable_entry_evidence(&original_handle).unwrap();
            let decoy = capture_windows_immutable_entry_evidence(&decoy_handle).unwrap();

            assert_eq!(retained.identity, expected_identity);
            assert_ne!(retained.identity, decoy.identity);
            assert_ne!(
                retained.security_descriptor_sha256,
                decoy.security_descriptor_sha256
            );
            drop(decoy_handle);
            drop(original_handle);
            fs::remove_dir_all(root).unwrap();
        }

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
        fn directory_open_rejects_an_ordinary_file() {
            let root = unique_temp_root("directory-file");
            fs::create_dir_all(&root).unwrap();
            let file = root.join("not-a-directory");
            fs::write(&file, b"not a directory").unwrap();

            let error = open_directory_nofollow(&file).unwrap_err();

            assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
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

        #[test]
        fn windows_handle_enumeration_uses_the_retained_directory_after_path_replacement() {
            let root = unique_temp_root("retained-enumeration");
            let original = root.join("directory");
            let displaced = root.join("directory-displaced");
            fs::create_dir_all(&original).unwrap();
            fs::write(original.join("alpha.txt"), b"alpha").unwrap();
            fs::write(original.join("zeta.txt"), b"zeta").unwrap();
            let directory = open_directory_nofollow(&original).unwrap();
            fs::rename(&original, &displaced).unwrap();
            fs::create_dir(&original).unwrap();
            fs::write(original.join("decoy.txt"), b"decoy").unwrap();

            let names = read_directory_names(&directory).unwrap();

            assert_eq!(
                names,
                vec![OsString::from("alpha.txt"), OsString::from("zeta.txt")]
            );
            drop(directory);
            fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn windows_handle_enumeration_accepts_a_handle_relative_child() {
            let root = unique_temp_root("child-enumeration");
            let child_path = root.join("child");
            fs::create_dir_all(&child_path).unwrap();
            fs::write(child_path.join("entry.txt"), b"entry").unwrap();
            let parent = open_directory_nofollow(&root).unwrap();
            let child =
                open_directory_child_nofollow(&parent, std::ffi::OsStr::new("child")).unwrap();

            let names = read_directory_names(&child).unwrap();

            assert_eq!(names, vec![OsString::from("entry.txt")]);
            drop(child);
            drop(parent);
            fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn windows_handle_enumeration_accepts_a_new_private_child() {
            let root = unique_temp_root("private-child-enumeration");
            fs::create_dir_all(&root).unwrap();
            let parent = open_directory_nofollow(&root).unwrap();
            let child =
                create_owner_only_directory_child(&parent, std::ffi::OsStr::new("child")).unwrap();
            fs::write(root.join("child").join("entry.txt"), b"entry").unwrap();

            let names = read_directory_names(&child).unwrap();

            assert_eq!(names, vec![OsString::from("entry.txt")]);
            drop(child);
            drop(parent);
            fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn windows_nt_create_options_for_std_file_are_synchronous() {
            const FILE_DIRECTORY_FILE: u32 = 0x0000_0001;
            const FILE_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
            const FILE_SYNCHRONOUS_IO_ALERT: u32 = 0x0000_0010;
            const FILE_SYNCHRONOUS_IO_NONALERT: u32 = 0x0000_0020;
            use windows_sys::Win32::Storage::FileSystem::SYNCHRONIZE;

            let options = nt_create_options_for_std_file(
                SYNCHRONIZE,
                FILE_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT,
            )
            .unwrap();

            assert_ne!(options & FILE_SYNCHRONOUS_IO_NONALERT, 0);
            assert_eq!(options & FILE_SYNCHRONOUS_IO_ALERT, 0);
            assert_ne!(options & FILE_DIRECTORY_FILE, 0);
            assert_ne!(options & FILE_OPEN_REPARSE_POINT, 0);
        }

        #[test]
        fn windows_nt_create_options_reject_asynchronous_std_file_access() {
            let error = nt_create_options_for_std_file(0, 0).unwrap_err();

            assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
            assert!(error.to_string().contains("SYNCHRONIZE"), "{error}");
        }

        #[test]
        fn windows_directory_enumeration_status_classifier_distinguishes_first_query_eos() {
            let no_more = io::Error::from_raw_os_error(ERROR_NO_MORE_FILES as i32);
            let no_match = io::Error::from_raw_os_error(ERROR_FILE_NOT_FOUND as i32);
            let denied = io::Error::from_raw_os_error(ERROR_ACCESS_DENIED as i32);

            assert!(directory_query_is_end(true, &no_more));
            assert!(directory_query_is_end(false, &no_more));
            assert!(directory_query_is_end(true, &no_match));
            assert!(!directory_query_is_end(false, &no_match));
            assert!(!directory_query_is_end(true, &denied));
        }

        #[test]
        fn windows_directory_enumeration_accepts_an_empty_retained_directory() {
            let root = unique_temp_root("empty-enumeration");
            fs::create_dir_all(&root).unwrap();
            let directory = open_directory_nofollow(&root).unwrap();

            assert!(read_directory_names(&directory).unwrap().is_empty());

            drop(directory);
            fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn windows_directory_enumeration_reads_multiple_native_buffers() {
            let root = unique_temp_root("multi-buffer-enumeration");
            fs::create_dir_all(&root).unwrap();
            let mut expected = Vec::new();
            for index in 0..900 {
                let name = format!("{index:04}-{}.xml", "x".repeat(80));
                fs::write(root.join(&name), b"x").unwrap();
                expected.push(OsString::from(name));
            }
            let directory = open_directory_nofollow(&root).unwrap();

            let names = read_directory_names(&directory).unwrap();

            assert_eq!(names, expected);
            drop(directory);
            fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn windows_directory_parser_rejects_an_unaligned_next_record() {
            let mut buffer = vec![0u8; 512];
            let next = 108u32;
            let name_length = 2u32;
            // SAFETY: the test buffer is large enough for each field and unaligned writes accept
            // Vec<u8>'s alignment.
            unsafe {
                std::ptr::write_unaligned(
                    buffer
                        .as_mut_ptr()
                        .add(offset_of!(FILE_ID_BOTH_DIR_INFO, NextEntryOffset))
                        .cast::<u32>(),
                    next,
                );
                std::ptr::write_unaligned(
                    buffer
                        .as_mut_ptr()
                        .add(offset_of!(FILE_ID_BOTH_DIR_INFO, FileNameLength))
                        .cast::<u32>(),
                    name_length,
                );
                std::ptr::write_unaligned(
                    buffer
                        .as_mut_ptr()
                        .add(offset_of!(FILE_ID_BOTH_DIR_INFO, FileName))
                        .cast::<u16>(),
                    b'a' as u16,
                );
            }
            let mut names = Vec::new();

            let error = parse_directory_information_buffer(&buffer, &mut names).unwrap_err();

            assert!(error.to_string().contains("8-byte-aligned"), "{error}");
        }

        #[test]
        fn windows_directory_parser_rejects_a_truncated_rust_header() {
            let buffer = vec![0u8; size_of::<FILE_ID_BOTH_DIR_INFO>() - 1];
            let mut names = Vec::new();

            let error = parse_directory_information_buffer(&buffer, &mut names).unwrap_err();

            assert!(error.to_string().contains("complete header"), "{error}");
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
