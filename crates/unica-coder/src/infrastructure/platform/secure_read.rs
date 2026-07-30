use std::io;
use std::path::Path;

#[derive(Debug)]
pub(crate) struct SecureRead {
    pub(crate) bytes: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SecureReadPhase {
    Root,
    Parent,
    BeforeRead,
}

pub(crate) fn read_root_relative_regular_file(
    root: &Path,
    path: &Path,
    maximum_bytes: usize,
    phase: impl Fn(SecureReadPhase),
) -> io::Result<SecureRead> {
    imp::read(root, path, maximum_bytes, phase)
}

fn relative_path<'a>(root: &Path, path: &'a Path) -> io::Result<&'a Path> {
    path.strip_prefix(root).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "resource path is outside its source root",
        )
    })
}

#[cfg(unix)]
mod imp {
    use super::{relative_path, SecureRead, SecureReadPhase};
    use crate::infrastructure::platform::filesystem::file_identity;
    use std::ffi::{CString, OsStr};
    use std::fs::File;
    use std::io::{self, Read};
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::path::{Component, Path};

    pub(super) fn read(
        root: &Path,
        path: &Path,
        maximum_bytes: usize,
        phase: impl Fn(SecureReadPhase),
    ) -> io::Result<SecureRead> {
        let relative = relative_path(root, path)?;
        let name = relative.file_name().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "resource path has no file name",
            )
        })?;
        let parent_path = relative.parent().unwrap_or_else(|| Path::new(""));
        let root = open_absolute_directory(root)?;
        phase(SecureReadPhase::Root);
        let parent = open_relative_directory(&root, parent_path)?;
        phase(SecureReadPhase::Parent);
        let mut file = open_regular_child(&parent, name)?;
        let identity = file_identity(&file)?;
        let opened = file.metadata()?;
        phase(SecureReadPhase::BeforeRead);
        if usize::try_from(opened.len()).map_or(true, |length| length > maximum_bytes) {
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "resource exceeds the snapshot byte limit",
            ));
        }
        let mut bytes = Vec::with_capacity(opened.len() as usize);
        file.by_ref()
            .take(maximum_bytes.saturating_add(1) as u64)
            .read_to_end(&mut bytes)?;
        if bytes.len() > maximum_bytes {
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "resource exceeds the snapshot byte limit",
            ));
        }
        let after = file.metadata()?;
        if opened.len() != after.len() || opened.modified().ok() != after.modified().ok() {
            return Err(io::Error::other("resource changed while reading"));
        }
        let rebound = open_regular_child(&parent, name)?;
        if file_identity(&rebound)? != identity {
            return Err(io::Error::other("resource identity changed while reading"));
        }
        Ok(SecureRead { bytes })
    }

    fn open_absolute_directory(path: &Path) -> io::Result<File> {
        if !path.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "secure root must be absolute",
            ));
        }
        let root = CString::new("/")?;
        let descriptor = unsafe {
            libc::open(
                root.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        if descriptor < 0 {
            return Err(io::Error::last_os_error());
        }
        let mut current = unsafe { File::from_raw_fd(descriptor) };
        for component in path.components() {
            match component {
                Component::RootDir | Component::CurDir => {}
                Component::Normal(name) => current = open_directory_child(&current, name)?,
                Component::ParentDir | Component::Prefix(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "secure root contains a non-normal component",
                    ))
                }
            }
        }
        Ok(current)
    }

    fn open_relative_directory(root: &File, path: &Path) -> io::Result<File> {
        let mut current = root.try_clone()?;
        for component in path.components() {
            match component {
                Component::CurDir => {}
                Component::Normal(name) => current = open_directory_child(&current, name)?,
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "resource path contains a non-normal component",
                    ))
                }
            }
        }
        Ok(current)
    }

    fn open_directory_child(parent: &File, name: &OsStr) -> io::Result<File> {
        open_child(
            parent,
            name,
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    }

    fn open_regular_child(parent: &File, name: &OsStr) -> io::Result<File> {
        let file = open_child(
            parent,
            name,
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
        )?;
        if !file.metadata()?.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "resource is not a regular file",
            ));
        }
        Ok(file)
    }

    fn open_child(parent: &File, name: &OsStr, flags: libc::c_int) -> io::Result<File> {
        let name = CString::new(name.as_bytes())?;
        let descriptor = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
        if descriptor < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(unsafe { File::from_raw_fd(descriptor) })
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::symlink;
    use uuid::Uuid;

    struct Fixture {
        root: std::path::PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("unica-secure-read-{}", Uuid::new_v4()));
            fs::create_dir_all(root.join("parent")).unwrap();
            fs::write(root.join("parent/resource.xml"), b"trusted").unwrap();
            Self { root }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn rejects_final_component_swap_after_parent_handle_is_open() {
        let fixture = Fixture::new();
        let path = fixture.root.join("parent/resource.xml");
        let outside = fixture.root.join("outside.xml");
        fs::write(&outside, b"outside").unwrap();
        let result = read_root_relative_regular_file(&fixture.root, &path, 1024, |phase| {
            if phase == SecureReadPhase::Parent {
                fs::rename(&path, fixture.root.join("parent/original.xml")).unwrap();
                symlink(&outside, &path).unwrap();
            }
        });
        assert!(result.is_err());
    }

    #[test]
    fn rejects_parent_component_swap_after_root_handle_is_open() {
        let fixture = Fixture::new();
        let path = fixture.root.join("parent/resource.xml");
        let outside = fixture.root.join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("resource.xml"), b"outside").unwrap();
        let result = read_root_relative_regular_file(&fixture.root, &path, 1024, |phase| {
            if phase == SecureReadPhase::Root {
                fs::rename(
                    fixture.root.join("parent"),
                    fixture.root.join("original-parent"),
                )
                .unwrap();
                symlink(&outside, fixture.root.join("parent")).unwrap();
            }
        });
        assert!(result.is_err());
    }

    #[test]
    fn rejects_file_growth_during_bounded_read() {
        let fixture = Fixture::new();
        let path = fixture.root.join("parent/resource.xml");
        let result = read_root_relative_regular_file(&fixture.root, &path, 1024, |phase| {
            if phase == SecureReadPhase::BeforeRead {
                use std::io::Write;
                let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
                file.write_all(b"-changed").unwrap();
            }
        });
        assert!(result.is_err());
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;
    use std::fs;
    use std::os::windows::fs::{symlink_dir, symlink_file};
    use uuid::Uuid;

    struct Fixture {
        root: std::path::PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("unica-secure-read-win-{}", Uuid::new_v4()));
            fs::create_dir_all(root.join("parent")).unwrap();
            fs::write(root.join("parent/resource.xml"), b"trusted").unwrap();
            Self { root }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn reads_normal_root_relative_file() {
        let fixture = Fixture::new();
        let read = read_root_relative_regular_file(
            &fixture.root,
            &fixture.root.join("parent/resource.xml"),
            1024,
            |_| {},
        )
        .unwrap();
        assert_eq!(read.bytes, b"trusted");
    }

    #[test]
    fn rejects_final_file_reparse_point() {
        let fixture = Fixture::new();
        let path = fixture.root.join("parent/resource.xml");
        let outside = fixture.root.join("outside.xml");
        fs::write(&outside, b"outside").unwrap();
        fs::remove_file(&path).unwrap();
        symlink_file(&outside, &path).unwrap();
        assert!(read_root_relative_regular_file(&fixture.root, &path, 1024, |_| {}).is_err());
    }

    #[test]
    fn rejects_parent_directory_reparse_point() {
        let fixture = Fixture::new();
        let path = fixture.root.join("parent/resource.xml");
        let outside = fixture.root.join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("resource.xml"), b"outside").unwrap();
        fs::remove_dir_all(fixture.root.join("parent")).unwrap();
        symlink_dir(&outside, fixture.root.join("parent")).unwrap();
        assert!(read_root_relative_regular_file(&fixture.root, &path, 1024, |_| {}).is_err());
    }

    #[test]
    fn rejects_reparse_source_root() {
        let fixture = Fixture::new();
        let alias = fixture.root.with_extension("alias");
        symlink_dir(&fixture.root, &alias).unwrap();
        let result = read_root_relative_regular_file(
            &alias,
            &alias.join("parent/resource.xml"),
            1024,
            |_| {},
        );
        let _ = fs::remove_dir(&alias);
        assert!(result.is_err());
    }

    #[test]
    fn rejects_file_above_hard_byte_cap() {
        let fixture = Fixture::new();
        let path = fixture.root.join("parent/resource.xml");
        assert!(read_root_relative_regular_file(&fixture.root, &path, 6, |_| {}).is_err());
    }

    #[test]
    fn swap_attempt_cannot_change_identity_bound_bytes() {
        let fixture = Fixture::new();
        let path = fixture.root.join("parent/resource.xml");
        let replacement = fixture.root.join("replacement.xml");
        let original = fixture.root.join("original.xml");
        fs::write(&replacement, b"outside").unwrap();
        let read = read_root_relative_regular_file(&fixture.root, &path, 1024, |phase| {
            if phase == SecureReadPhase::BeforeRead && fs::rename(&path, &original).is_ok() {
                fs::rename(&replacement, &path).unwrap();
            }
        });
        assert!(read.is_err() || read.unwrap().bytes == b"trusted");
    }
}

#[cfg(windows)]
mod imp {
    use super::{relative_path, SecureRead, SecureReadPhase};
    use crate::infrastructure::platform::filesystem::file_identity;
    use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt, OpenOptionsMaybeDirExt};
    use cap_primitives::ambient_authority;
    use cap_primitives::fs::{open, open_ambient, open_dir_nofollow, OpenOptions};
    use std::fs::File;
    use std::io::{self, Read};
    use std::os::windows::fs::MetadataExt;
    use std::path::{Component, Path};
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

    pub(super) fn read(
        root: &Path,
        path: &Path,
        maximum_bytes: usize,
        phase: impl Fn(SecureReadPhase),
    ) -> io::Result<SecureRead> {
        let relative = relative_path(root, path)?;
        let name = relative.file_name().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "resource path has no file name",
            )
        })?;
        let parent_path = relative.parent().unwrap_or_else(|| Path::new(""));
        let root = open_root_nofollow(root)?;
        reject_reparse(&root)?;
        phase(SecureReadPhase::Root);
        let mut parent = root;
        for component in parent_path.components() {
            match component {
                Component::CurDir => {}
                Component::Normal(name) => {
                    parent = open_dir_nofollow(&parent, Path::new(name))?;
                    reject_reparse(&parent)?;
                }
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "resource path contains a non-normal component",
                    ))
                }
            }
        }
        phase(SecureReadPhase::Parent);
        let mut file = open_regular_child(&parent, name)?;
        let identity = file_identity(&file)?;
        let opened = file.metadata()?;
        phase(SecureReadPhase::BeforeRead);
        if usize::try_from(opened.len()).map_or(true, |length| length > maximum_bytes) {
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "resource exceeds the snapshot byte limit",
            ));
        }
        let mut bytes = Vec::with_capacity(opened.len() as usize);
        file.by_ref()
            .take(maximum_bytes.saturating_add(1) as u64)
            .read_to_end(&mut bytes)?;
        if bytes.len() > maximum_bytes {
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "resource exceeds the snapshot byte limit",
            ));
        }
        let after = file.metadata()?;
        if opened.len() != after.len() || opened.modified().ok() != after.modified().ok() {
            return Err(io::Error::other("resource changed while reading"));
        }
        let rebound = open_regular_child(&parent, name)?;
        if file_identity(&rebound)? != identity {
            return Err(io::Error::other("resource identity changed while reading"));
        }
        Ok(SecureRead { bytes })
    }

    fn open_regular_child(parent: &File, name: &std::ffi::OsStr) -> io::Result<File> {
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        let file = open(parent, Path::new(name), &options)?;
        let metadata = file.metadata()?;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 || !metadata.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "resource is not a non-reparse regular file",
            ));
        }
        Ok(file)
    }

    fn open_root_nofollow(path: &Path) -> io::Result<File> {
        let mut options = OpenOptions::new();
        options
            .read(true)
            .follow(FollowSymlinks::No)
            .maybe_dir(true);
        let root = open_ambient(path, &options, ambient_authority())?;
        if !root.metadata()?.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "secure root is not a directory",
            ));
        }
        Ok(root)
    }

    fn reject_reparse(file: &File) -> io::Result<()> {
        if file.metadata()?.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "resource path traverses a reparse point",
            ));
        }
        Ok(())
    }
}

#[cfg(not(any(unix, windows)))]
mod imp {
    use super::{SecureRead, SecureReadPhase};
    use std::io;
    use std::path::Path;

    pub(super) fn read(
        _root: &Path,
        _path: &Path,
        _maximum_bytes: usize,
        _phase: impl Fn(SecureReadPhase),
    ) -> io::Result<SecureRead> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "secure no-follow reads are unavailable on this platform",
        ))
    }
}
