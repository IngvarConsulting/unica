//! Extended ACLs can grant access independently of a private POSIX mode.

use std::ffi::{c_int, c_void};
use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;
use std::ptr::NonNull;

// ABI constants and signatures from the macOS SDK's sys/acl.h and membership.h.
const ACL_TYPE_EXTENDED: c_int = 0x100;
const ACL_FIRST_ENTRY: c_int = 0;
const ACL_NEXT_ENTRY: c_int = -1;
const ACL_EXTENDED_ALLOW: c_int = 1;
const ACL_EXTENDED_DENY: c_int = 2;

extern "C" {
    fn acl_get_fd_np(fd: c_int, kind: c_int) -> *mut c_void;
    fn acl_valid(acl: *mut c_void) -> c_int;
    fn acl_get_entry(acl: *mut c_void, selector: c_int, entry: *mut *mut c_void) -> c_int;
    fn acl_get_tag_type(entry: *mut c_void, tag: *mut c_int) -> c_int;
    fn acl_get_qualifier(entry: *mut c_void) -> *mut c_void;
    fn acl_free(allocation: *mut c_void) -> c_int;
    fn mbr_uid_to_uuid(uid: libc::uid_t, uuid: *mut u8) -> c_int;
}

struct Allocation(NonNull<c_void>);

impl Allocation {
    fn as_ptr(&self) -> *mut c_void {
        self.0.as_ptr()
    }
}

impl Drop for Allocation {
    fn drop(&mut self) {
        // SAFETY: ACLs and qualifiers are independent allocations owned here.
        unsafe { acl_free(self.as_ptr()) };
    }
}

pub(super) fn verify_owner_only_acl(file: &File, owner_uid: libc::uid_t) -> io::Result<()> {
    // SAFETY: file retains the exact open object; the result is an independent
    // ACL copy, not a path lookup or a view that changes during iteration.
    let raw = unsafe { acl_get_fd_np(file.as_raw_fd(), ACL_TYPE_EXTENDED) };
    let Some(raw) = NonNull::new(raw) else {
        let error = io::Error::last_os_error();
        // Darwin reports ENOENT for no extended ACL on an existing open file.
        // The caller already verified ownership/mode using this retained fd.
        return if error.raw_os_error() == Some(libc::ENOENT) {
            Ok(())
        } else {
            Err(error)
        };
    };
    let acl = Allocation(raw);
    if unsafe { acl_valid(acl.as_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }

    let mut selector = ACL_FIRST_ENTRY;
    let mut owner_uuid = None;
    loop {
        let mut entry = std::ptr::null_mut();
        let found = unsafe { acl_get_entry(acl.as_ptr(), selector, &mut entry) };
        if found == -1 {
            let error = io::Error::last_os_error();
            // On Darwin, unlike POSIX's iterator convention, 0 means entry;
            // -1/EINVAL ends this validated, immutable copy with valid selectors.
            return if error.raw_os_error() == Some(libc::EINVAL) {
                Ok(())
            } else {
                Err(error)
            };
        }
        if found != 0 || entry.is_null() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid macOS ACL entry",
            ));
        }
        selector = ACL_NEXT_ENTRY;
        let mut tag = 0;
        if unsafe { acl_get_tag_type(entry, &mut tag) } != 0 {
            return Err(io::Error::last_os_error());
        }
        match tag {
            ACL_EXTENDED_DENY => continue,
            ACL_EXTENDED_ALLOW => {}
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unknown macOS ACL entry tag",
                ))
            }
        }
        let owner = match owner_uuid {
            Some(owner) => owner,
            None => {
                let mut owner = [0_u8; 16];
                let status = unsafe { mbr_uid_to_uuid(owner_uid, owner.as_mut_ptr()) };
                if status != 0 {
                    // membership.h returns the error number, without setting errno.
                    return Err(io::Error::from_raw_os_error(status));
                }
                owner_uuid = Some(owner);
                owner
            }
        };
        let qualifier = NonNull::new(unsafe { acl_get_qualifier(entry) })
            .map(Allocation)
            .ok_or_else(io::Error::last_os_error)?;
        // ACL_EXTENDED_ALLOW qualifiers are membership UUIDs, exactly 16 bytes.
        let allowed = unsafe { std::slice::from_raw_parts(qualifier.as_ptr().cast::<u8>(), 16) };
        if allowed != owner {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "macOS ACL grants access to a non-owner identity",
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{ffi::OsStr, fs, os::unix::fs::PermissionsExt, process::Command};

    #[test]
    fn inherited_foreign_allow_refuses_private_children_before_returning_a_writable_file() {
        let root = tempfile::tempdir().unwrap();
        let parent = fs::File::open(root.path()).unwrap();
        assert!(Command::new("chmod")
            .args(["+a", "everyone allow read,file_inherit,directory_inherit"])
            .arg(root.path())
            .status()
            .unwrap()
            .success());

        let result = super::super::create_owner_only_file_child(&parent, OsStr::new("staging"));
        assert!(
            result.is_err(),
            "inherited access must refuse before returning the token writer"
        );
        assert_eq!(fs::metadata(root.path().join("staging")).unwrap().len(), 0);
        assert_eq!(
            fs::metadata(root.path().join("staging"))
                .unwrap()
                .permissions()
                .mode()
                & 0o077,
            0
        );

        let result =
            super::super::create_owner_only_directory_child(&parent, OsStr::new("private"));
        assert!(
            result.is_err(),
            "private directory accepted inherited foreign access"
        );
        assert_eq!(
            fs::metadata(root.path().join("private"))
                .unwrap()
                .permissions()
                .mode()
                & 0o077,
            0
        );
    }

    #[test]
    fn private_acl_accepts_owner_allow_and_non_owner_deny() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("private");
        fs::write(&path, b"private fixture").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let file = fs::File::open(&path).unwrap();
        let username = Command::new("id").arg("-un").output().unwrap();
        assert!(username.status.success());
        let username = String::from_utf8(username.stdout).unwrap();
        let allow = format!("user:{} allow read", username.trim());
        for entry in [&allow, "everyone deny write"] {
            assert!(Command::new("chmod")
                .args(["+a", entry])
                .arg(&path)
                .status()
                .unwrap()
                .success());
            super::super::verify_owner_only_acl(&file)
                .expect("owner allow and deny entries remain private");
        }
    }
}
