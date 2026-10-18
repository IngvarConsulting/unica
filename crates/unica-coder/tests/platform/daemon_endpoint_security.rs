use std::path::{Path, PathBuf};

#[derive(Debug, PartialEq, Eq)]
pub struct FileIdentity(u64, u64);

#[cfg(unix)]
pub fn file_identity(path: &Path) -> FileIdentity {
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::metadata(path).unwrap();
    FileIdentity(metadata.dev(), metadata.ino())
}

#[cfg(windows)]
pub fn file_identity(path: &Path) -> FileIdentity {
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS,
    };
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .unwrap();
    let mut information: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    assert_ne!(
        unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut information) },
        0,
        "{}",
        std::io::Error::last_os_error()
    );
    FileIdentity(
        u64::from(information.dwVolumeSerialNumber),
        (u64::from(information.nFileIndexHigh) << 32) | u64::from(information.nFileIndexLow),
    )
}

#[cfg(unix)]
pub fn assert_owner_only(path: &Path) {
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::metadata(path).unwrap();
    assert_eq!(metadata.uid(), unsafe { libc::geteuid() });
    assert_eq!(
        metadata.mode() & 0o077,
        0,
        "non-owner access to {}",
        path.display()
    );
    let owner_access = if metadata.is_dir() { 0o700 } else { 0o600 };
    assert_eq!(metadata.mode() & owner_access, owner_access);
    #[cfg(target_os = "macos")]
    darwin_acl::Acl::capture(path).assert_only_owner_allows(metadata.uid());
}

#[cfg(windows)]
pub fn assert_owner_only(path: &Path) {
    powershell(
        path,
        r#"
        $acl = Get-Acl -LiteralPath $path
        $sid = [System.Security.Principal.WindowsIdentity]::GetCurrent().User
        if ($acl.GetOwner([System.Security.Principal.SecurityIdentifier]) -ne $sid) { throw 'wrong owner SID' }
        if (-not $acl.AreAccessRulesProtected) { throw 'unprotected DACL' }
        $rules = @($acl.GetAccessRules($true, $true, [System.Security.Principal.SecurityIdentifier]))
        if ($rules.Count -ne 1) { throw 'DACL does not have one owner rule' }
        $rule = $rules[0]
        if ($rule.IdentityReference -ne $sid) { throw 'access granted to another SID' }
        if ($rule.AccessControlType -ne [System.Security.AccessControl.AccessControlType]::Allow) { throw 'owner access is not allowed' }
        if ($rule.IsInherited -or $rule.InheritanceFlags -ne 'None' -or $rule.PropagationFlags -ne 'None') { throw 'inheritable or inherited rule' }
        $full = [System.Security.AccessControl.FileSystemRights]::FullControl
        if (($rule.FileSystemRights -band $full) -ne $full) { throw 'owner lacks full control' }
    "#,
    );
}

#[cfg(unix)]
pub struct SecuritySnapshot {
    permissions: std::fs::Permissions,
    #[cfg(target_os = "macos")]
    acl: darwin_acl::Acl,
}

#[cfg(unix)]
impl SecuritySnapshot {
    pub fn capture(path: &Path) -> Self {
        Self {
            permissions: std::fs::metadata(path).unwrap().permissions(),
            #[cfg(target_os = "macos")]
            acl: darwin_acl::Acl::capture(path),
        }
    }

    pub fn apply_to(&self, path: &Path) {
        std::fs::set_permissions(path, self.permissions.clone()).unwrap();
        #[cfg(target_os = "macos")]
        self.acl.apply_to(path);
    }
}

#[cfg(windows)]
pub struct SecuritySnapshot(String);

#[cfg(windows)]
impl SecuritySnapshot {
    pub fn capture(path: &Path) -> Self {
        Self(powershell(
            path,
            r#"
            $acl = Get-Acl -LiteralPath $path
            $sections = [System.Security.AccessControl.AccessControlSections]::Owner -bor [System.Security.AccessControl.AccessControlSections]::Group -bor [System.Security.AccessControl.AccessControlSections]::Access
            $acl.GetSecurityDescriptorSddlForm($sections)
        "#,
        ))
    }

    pub fn apply_to(&self, path: &Path) {
        let sddl = self.0.trim().replace('\'', "''");
        powershell(
            path,
            &format!(
                r#"
            $acl = Get-Acl -LiteralPath $path
            $acl.SetSecurityDescriptorSddlForm('{sddl}')
            Set-Acl -LiteralPath $path -AclObject $acl
        "#
            ),
        );
    }
}

pub struct InsecureAccess {
    path: PathBuf,
    original: SecuritySnapshot,
}

impl InsecureAccess {
    pub fn install(path: &Path) -> Self {
        let original = SecuritySnapshot::capture(path);
        let guard = Self {
            path: path.to_owned(),
            original,
        };
        expose_to_others(path);
        guard
    }

    #[cfg(target_os = "macos")]
    pub fn install_acl(path: &Path) -> Self {
        use std::os::unix::fs::PermissionsExt;
        let original = SecuritySnapshot::capture(path);
        let mode = std::fs::metadata(path).unwrap().permissions().mode();
        let guard = Self {
            path: path.to_owned(),
            original,
        };
        assert!(std::process::Command::new("chmod")
            .args(["+a", "everyone allow read"])
            .arg(path)
            .status()
            .unwrap()
            .success());
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode(),
            mode,
            "ACL counterexample must retain the private POSIX mode"
        );
        guard
    }
}

impl Drop for InsecureAccess {
    fn drop(&mut self) {
        self.original.apply_to(&self.path);
    }
}

#[cfg(unix)]
fn expose_to_others(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let metadata = std::fs::metadata(path).unwrap();
    let extra = if metadata.is_dir() { 0o055 } else { 0o044 };
    std::fs::set_permissions(
        path,
        std::fs::Permissions::from_mode(metadata.permissions().mode() | extra),
    )
    .unwrap();
}

#[cfg(windows)]
fn expose_to_others(path: &Path) {
    powershell(
        path,
        r#"
        $acl = Get-Acl -LiteralPath $path
        $everyone = [System.Security.Principal.SecurityIdentifier]::new('S-1-1-0')
        $rule = [System.Security.AccessControl.FileSystemAccessRule]::new($everyone, [System.Security.AccessControl.FileSystemRights]::Read, [System.Security.AccessControl.AccessControlType]::Allow)
        $acl.AddAccessRule($rule)
        Set-Acl -LiteralPath $path -AclObject $acl
    "#,
    );
}

#[cfg(windows)]
fn powershell(path: &Path, body: &str) -> String {
    let escaped = path.to_str().unwrap().replace('\'', "''");
    let script = format!("$ErrorActionPreference='Stop'; $path='{escaped}'; {body}");
    let output = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .expect("inspect or change the isolated fixture's Windows ACL");
    assert!(
        output.status.success(),
        "Windows ACL fixture failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

#[cfg(target_os = "macos")]
mod darwin_acl {
    use std::{
        ffi::{c_char, c_int, c_void, CString},
        os::unix::ffi::OsStrExt,
        path::Path,
    };

    // SDK sys/acl.h and membership.h: extended ACLs use opaque allocations;
    // their entry qualifiers are 16-byte membership UUIDs.
    extern "C" {
        fn acl_get_file(path: *const c_char, kind: c_int) -> *mut c_void;
        fn acl_init(count: c_int) -> *mut c_void;
        fn acl_set_file(path: *const c_char, kind: c_int, acl: *mut c_void) -> c_int;
        fn acl_free(allocation: *mut c_void) -> c_int;
        fn acl_valid(acl: *mut c_void) -> c_int;
        fn acl_get_entry(acl: *mut c_void, entry_id: c_int, entry: *mut *mut c_void) -> c_int;
        fn acl_get_tag_type(entry: *mut c_void, tag: *mut c_int) -> c_int;
        fn acl_get_qualifier(entry: *mut c_void) -> *mut c_void;
        fn mbr_uid_to_uuid(uid: libc::uid_t, uuid: *mut u8) -> c_int;
    }
    const EXTENDED: c_int = 0x100;

    pub struct Acl(*mut c_void);

    impl Acl {
        pub fn capture(path: &Path) -> Self {
            let encoded = CString::new(path.as_os_str().as_bytes()).unwrap();
            let mut acl = unsafe { acl_get_file(encoded.as_ptr(), EXTENDED) };
            if acl.is_null() && std::io::Error::last_os_error().raw_os_error() == Some(libc::ENOENT)
            {
                // Darwin also returns ENOENT for an existing file with no
                // extended ACL. Do not confuse a missing fixture with that case.
                std::fs::metadata(path).expect("the ACL fixture itself must exist");
                acl = unsafe { acl_init(0) };
            }
            assert!(!acl.is_null(), "{}", std::io::Error::last_os_error());
            Self(acl)
        }

        pub fn apply_to(&self, path: &Path) {
            let path = CString::new(path.as_os_str().as_bytes()).unwrap();
            assert_eq!(
                unsafe { acl_set_file(path.as_ptr(), EXTENDED, self.0) },
                0,
                "{}",
                std::io::Error::last_os_error()
            );
        }

        pub fn assert_only_owner_allows(&self, uid: libc::uid_t) {
            assert_eq!(unsafe { acl_valid(self.0) }, 0);
            let mut owner = [0_u8; 16];
            assert_eq!(unsafe { mbr_uid_to_uuid(uid, owner.as_mut_ptr()) }, 0);
            let mut entry = std::ptr::null_mut();
            let mut selector = 0; // ACL_FIRST_ENTRY
            loop {
                let status = unsafe { acl_get_entry(self.0, selector, &mut entry) };
                if status == -1 {
                    assert_eq!(
                        std::io::Error::last_os_error().raw_os_error(),
                        Some(libc::EINVAL),
                        "ACL inspection failed before its documented end"
                    );
                    break;
                }
                assert_eq!(status, 0);
                assert!(!entry.is_null());
                selector = -1; // ACL_NEXT_ENTRY
                let mut tag = 0;
                assert_eq!(unsafe { acl_get_tag_type(entry, &mut tag) }, 0);
                if tag == 1 {
                    // ACL_EXTENDED_ALLOW
                    let qualifier = unsafe { acl_get_qualifier(entry) };
                    assert!(!qualifier.is_null());
                    let uuid =
                        unsafe { std::slice::from_raw_parts(qualifier.cast::<u8>(), 16) }.to_vec();
                    unsafe { acl_free(qualifier) };
                    assert_eq!(uuid, owner, "ACL grants access to a non-owner identity");
                } else {
                    assert_eq!(tag, 2, "unexpected extended ACL tag"); // ACL_EXTENDED_DENY
                }
            }
        }
    }

    impl Drop for Acl {
        fn drop(&mut self) {
            unsafe { acl_free(self.0) };
        }
    }
}

#[cfg(target_os = "macos")]
#[test]
fn frontend_refuses_an_insecure_acl_even_with_private_posix_mode() {
    super::endpoint_ownership::insecure_endpoint_is_refused(false, InsecureAccess::install_acl);
}

#[cfg(target_os = "macos")]
#[test]
fn frontend_refuses_an_insecure_directory_acl_even_with_private_posix_mode() {
    super::endpoint_ownership::insecure_endpoint_is_refused(true, InsecureAccess::install_acl);
}
