use crate::infrastructure::platform::filesystem::{
    file_identity, open_any_child_nofollow, open_directory_child_nofollow,
    open_regular_child_nofollow, read_directory_names, OpenedChildKind,
};
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub(crate) struct SecureRead {
    pub(crate) bytes: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SecureReadPhase {
    Root,
    Parent,
    BeforeRead,
    BeforeRebind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SecureTreePhase {
    RootOpened,
    BeforeOpenEntry(PathBuf),
    BeforeRebindEntry(PathBuf),
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SecureTreeLimits {
    pub(crate) maximum_depth: usize,
    pub(crate) maximum_entries: usize,
    pub(crate) maximum_files: usize,
}

#[derive(Debug)]
pub(crate) struct SecureFileList {
    pub(crate) files: Vec<SecureFileEntry>,
    pub(crate) start_missing: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SecureFileEntry {
    pub(crate) logical_path: String,
    pub(crate) relative_path: PathBuf,
}

#[cfg(test)]
thread_local! {
    static SECURE_TREE_TEST_HOOK: std::cell::RefCell<Option<Box<dyn FnMut(&SecureTreePhase)>>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn with_secure_tree_test_hook<T>(
    hook: impl FnMut(&SecureTreePhase) + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<Box<dyn FnMut(&SecureTreePhase)>>);
    impl Drop for Reset {
        fn drop(&mut self) {
            SECURE_TREE_TEST_HOOK.with(|slot| *slot.borrow_mut() = self.0.take());
        }
    }

    let previous = SECURE_TREE_TEST_HOOK.with(|slot| slot.replace(Some(Box::new(hook))));
    let _reset = Reset(previous);
    action()
}

fn emit_tree_phase(phase: SecureTreePhase) {
    #[cfg(test)]
    SECURE_TREE_TEST_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().as_mut() {
            hook(&phase);
        }
    });
    #[cfg(not(test))]
    let _ = phase;
}

pub(crate) fn read_root_relative_regular_file(
    root: &Path,
    path: &Path,
    maximum_bytes: usize,
    phase: impl Fn(SecureReadPhase),
) -> io::Result<SecureRead> {
    read_root_relative_regular_file_checked(root, path, maximum_bytes, || Ok(()), phase)
}

pub(crate) fn read_root_relative_regular_file_checked(
    root: &Path,
    path: &Path,
    maximum_bytes: usize,
    mut checkpoint: impl FnMut() -> io::Result<()>,
    mut phase: impl FnMut(SecureReadPhase),
) -> io::Result<SecureRead> {
    let relative = relative_path(root, path)?.to_path_buf();
    imp::read(root, path, maximum_bytes, &mut checkpoint, |read_phase| {
        phase(read_phase);
        match read_phase {
            SecureReadPhase::Root => emit_tree_phase(SecureTreePhase::RootOpened),
            SecureReadPhase::Parent => {
                emit_tree_phase(SecureTreePhase::BeforeOpenEntry(relative.clone()))
            }
            SecureReadPhase::BeforeRebind => {
                emit_tree_phase(SecureTreePhase::BeforeRebindEntry(relative.clone()))
            }
            SecureReadPhase::BeforeRead => {}
        }
    })
}

pub(crate) fn list_root_relative_regular_files(
    root: &Path,
    start: &Path,
    limits: SecureTreeLimits,
    mut descend: impl FnMut(&Path) -> bool,
    mut select: impl FnMut(&Path) -> bool,
    mut checkpoint: impl FnMut() -> io::Result<()>,
) -> io::Result<SecureFileList> {
    checkpoint()?;
    let root_handle = imp::open_absolute_directory(root)?;
    emit_tree_phase(SecureTreePhase::RootOpened);
    let mut start_handle = root_handle.try_clone()?;
    let mut logical_start = PathBuf::new();
    for component in start.components() {
        use std::path::Component;
        let Component::Normal(name) = component else {
            if matches!(component, Component::CurDir) {
                continue;
            }
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "secure tree start contains a non-normal component",
            ));
        };
        logical_start.push(name);
        checkpoint()?;
        emit_tree_phase(SecureTreePhase::BeforeOpenEntry(logical_start.clone()));
        match open_directory_child_nofollow(&start_handle, name) {
            Ok(child) => start_handle = child,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(SecureFileList {
                    files: Vec::new(),
                    start_missing: true,
                })
            }
            Err(error) => return Err(error),
        }
    }

    let mut state = (0usize, Vec::new());
    walk_directory(
        &start_handle,
        &logical_start,
        SecureWalkPosition { depth: 0, limits },
        &mut descend,
        &mut select,
        &mut checkpoint,
        &mut state,
    )?;
    state
        .1
        .sort_by(|left, right| left.logical_path.cmp(&right.logical_path));
    Ok(SecureFileList {
        files: state.1,
        start_missing: false,
    })
}

#[derive(Clone, Copy)]
struct SecureWalkPosition {
    depth: usize,
    limits: SecureTreeLimits,
}

fn walk_directory(
    directory: &File,
    logical_directory: &Path,
    position: SecureWalkPosition,
    descend: &mut impl FnMut(&Path) -> bool,
    select: &mut impl FnMut(&Path) -> bool,
    checkpoint: &mut impl FnMut() -> io::Result<()>,
    state: &mut impl SecureWalkState,
) -> io::Result<()> {
    checkpoint()?;
    let initial_names = read_directory_names(directory)?;
    state.add_entries(initial_names.len(), position.limits.maximum_entries)?;
    for name in &initial_names {
        checkpoint()?;
        let logical_path = logical_directory.join(name);
        emit_tree_phase(SecureTreePhase::BeforeOpenEntry(logical_path.clone()));
        let (child, kind) = open_any_child_nofollow(directory, name)?;
        let identity = file_identity(&child)?;
        match kind {
            OpenedChildKind::Directory => {
                let should_descend = descend(&logical_path);
                if should_descend && position.depth >= position.limits.maximum_depth {
                    return Err(io::Error::new(
                        io::ErrorKind::FileTooLarge,
                        "secure tree exceeds the traversal-depth limit",
                    ));
                }
                if should_descend {
                    walk_directory(
                        &child,
                        &logical_path,
                        SecureWalkPosition {
                            depth: position.depth + 1,
                            ..position
                        },
                        descend,
                        select,
                        checkpoint,
                        state,
                    )?;
                }
                checkpoint()?;
                emit_tree_phase(SecureTreePhase::BeforeRebindEntry(logical_path.clone()));
                let rebound = open_directory_child_nofollow(directory, name)?;
                if file_identity(&rebound)? != identity {
                    return Err(io::Error::other(
                        "directory identity changed while enumerating secure tree",
                    ));
                }
            }
            OpenedChildKind::RegularFile => {
                if select(&logical_path) {
                    state.add_file(logical_path.clone(), position.limits.maximum_files)?;
                }
                checkpoint()?;
                emit_tree_phase(SecureTreePhase::BeforeRebindEntry(logical_path.clone()));
                let rebound = open_regular_child_nofollow(directory, name)?;
                if file_identity(&rebound)? != identity {
                    return Err(io::Error::other(
                        "file identity changed while enumerating secure tree",
                    ));
                }
            }
            OpenedChildKind::ReparsePoint => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "secure tree contains a link or reparse point",
                ))
            }
            OpenedChildKind::Unsupported => {
                if select(&logical_path) {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "selected secure-tree entry is not a regular file",
                    ));
                }
            }
        }
    }
    checkpoint()?;
    if read_directory_names(directory)? != initial_names {
        return Err(io::Error::other(
            "directory membership changed while enumerating secure tree",
        ));
    }
    Ok(())
}

trait SecureWalkState {
    fn add_entries(&mut self, count: usize, maximum: usize) -> io::Result<()>;
    fn add_file(&mut self, path: PathBuf, maximum: usize) -> io::Result<()>;
}

impl SecureWalkState for (usize, Vec<SecureFileEntry>) {
    fn add_entries(&mut self, count: usize, maximum: usize) -> io::Result<()> {
        self.0 = self.0.checked_add(count).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::FileTooLarge,
                "secure-tree entry count overflowed",
            )
        })?;
        if self.0 > maximum {
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "secure tree exceeds the entry-count limit",
            ));
        }
        Ok(())
    }

    fn add_file(&mut self, path: PathBuf, maximum: usize) -> io::Result<()> {
        if self.1.len() >= maximum {
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "secure tree exceeds the selected-file limit",
            ));
        }
        let logical_path = path
            .components()
            .map(|component| {
                component.as_os_str().to_str().ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        "secure-tree logical path is not UTF-8",
                    )
                })
            })
            .collect::<io::Result<Vec<_>>>()?
            .join("/");
        self.1.push(SecureFileEntry {
            logical_path,
            relative_path: path,
        });
        Ok(())
    }
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
    use crate::infrastructure::platform::filesystem::{
        file_identity, open_directory_child_nofollow, open_directory_nofollow,
        open_regular_child_nofollow,
    };
    use std::fs::File;
    use std::io::{self, Read};
    use std::path::{Component, Path};

    pub(super) fn read(
        root: &Path,
        path: &Path,
        maximum_bytes: usize,
        checkpoint: &mut impl FnMut() -> io::Result<()>,
        mut phase: impl FnMut(SecureReadPhase),
    ) -> io::Result<SecureRead> {
        checkpoint()?;
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
        let mut file = open_regular_child_nofollow(&parent, name)?;
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
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            checkpoint()?;
            let read = file.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            bytes.extend_from_slice(&buffer[..read]);
            if bytes.len() > maximum_bytes {
                return Err(io::Error::new(
                    io::ErrorKind::FileTooLarge,
                    "resource exceeds the snapshot byte limit",
                ));
            }
        }
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
        checkpoint()?;
        phase(SecureReadPhase::BeforeRebind);
        let rebound = open_regular_child_nofollow(&parent, name)?;
        if file_identity(&rebound)? != identity {
            return Err(io::Error::other("resource identity changed while reading"));
        }
        Ok(SecureRead { bytes })
    }

    pub(super) fn open_absolute_directory(path: &Path) -> io::Result<File> {
        if !path.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "secure root must be absolute",
            ));
        }
        let mut names = Vec::new();
        for component in path.components() {
            match component {
                Component::RootDir | Component::CurDir => {}
                Component::Normal(name) => names.push(name.to_os_string()),
                Component::ParentDir => {
                    names.pop().ok_or_else(|| {
                        io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "secure root escapes the filesystem root",
                        )
                    })?;
                }
                Component::Prefix(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "secure Unix root contains a prefix component",
                    ))
                }
            }
        }
        let mut current = open_directory_nofollow(Path::new("/"))?;
        for name in names {
            current = open_directory_child_nofollow(&current, &name)?;
        }
        Ok(current)
    }

    fn open_relative_directory(root: &File, path: &Path) -> io::Result<File> {
        let mut current = root.try_clone()?;
        for component in path.components() {
            match component {
                Component::CurDir => {}
                Component::Normal(name) => current = open_directory_child_nofollow(&current, name)?,
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
        /// The root is canonicalized because the walk opens every component
        /// with `O_NOFOLLOW`: on macOS `std::env::temp_dir()` sits under the
        /// `/var` symlink, so an uncanonicalized root fails on its first
        /// component and every negative assertion below would pass without
        /// ever reaching the guard it names.
        fn new() -> Self {
            let root = fs::canonicalize(std::env::temp_dir())
                .unwrap()
                .join(format!("unica-secure-read-{}", Uuid::new_v4()));
            fs::create_dir_all(root.join("parent")).unwrap();
            fs::write(root.join("parent/resource.xml"), b"trusted").unwrap();
            let fixture = Self { root };
            fixture.assert_undisturbed_read_succeeds();
            fixture
        }

        /// Proves the fixture itself is readable, so a later `is_err()` can
        /// only come from the disturbance the test performs.
        fn assert_undisturbed_read_succeeds(&self) {
            let read = read_root_relative_regular_file(
                &self.root,
                &self.root.join("parent/resource.xml"),
                1024,
                |_| {},
            )
            .expect("undisturbed fixture read must succeed");
            assert_eq!(read.bytes, b"trusted");
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn reads_normal_root_relative_file() {
        Fixture::new().assert_undisturbed_read_succeeds();
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
        assert!(
            result.is_err(),
            "a swapped final component must never be read"
        );
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
        assert!(
            result.is_err(),
            "a swapped parent component must never be read"
        );
    }

    #[test]
    fn rejects_file_growth_during_bounded_read() {
        let fixture = Fixture::new();
        let path = fixture.root.join("parent/resource.xml");
        let error = read_root_relative_regular_file(&fixture.root, &path, 1024, |phase| {
            if phase == SecureReadPhase::BeforeRead {
                use std::io::Write;
                let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
                file.write_all(b"-changed").unwrap();
            }
        })
        .expect_err("a file that grows mid-read must not be returned");
        assert!(
            error.to_string().contains("changed while reading"),
            "{error}"
        );
    }

    #[test]
    fn retained_tree_rejects_directory_to_symlink_swap_before_child_open() {
        let fixture = Fixture::new();
        let nested = fixture.root.join("nested");
        let displaced = fixture.root.join("nested-original");
        let outside = fixture.root.join("outside-tree");
        fs::create_dir(&nested).unwrap();
        fs::write(nested.join("trusted.xml"), b"trusted").unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("outside.xml"), b"outside").unwrap();
        let nested_for_hook = nested.clone();
        let result = with_secure_tree_test_hook(
            move |phase| {
                if phase == &SecureTreePhase::BeforeOpenEntry(PathBuf::from("nested")) {
                    fs::rename(&nested_for_hook, &displaced).unwrap();
                    symlink(&outside, &nested_for_hook).unwrap();
                }
            },
            || {
                list_root_relative_regular_files(
                    &fixture.root,
                    Path::new(""),
                    SecureTreeLimits {
                        maximum_depth: 8,
                        maximum_entries: 100,
                        maximum_files: 100,
                    },
                    |_| true,
                    |path| path.extension().and_then(|value| value.to_str()) == Some("xml"),
                    || Ok(()),
                )
            },
        );
        assert!(
            result.is_err(),
            "a raced directory symlink must fail closed"
        );
    }

    #[test]
    fn retained_tree_rejects_file_to_symlink_swap_before_child_open() {
        let fixture = Fixture::new();
        let candidate = fixture.root.join("candidate.xml");
        let displaced = fixture.root.join("candidate-original.xml");
        let outside = fixture.root.join("outside.xml");
        fs::write(&candidate, b"trusted").unwrap();
        fs::write(&outside, b"outside").unwrap();
        let candidate_for_hook = candidate.clone();
        let result = with_secure_tree_test_hook(
            move |phase| {
                if phase == &SecureTreePhase::BeforeOpenEntry(PathBuf::from("candidate.xml")) {
                    fs::rename(&candidate_for_hook, &displaced).unwrap();
                    symlink(&outside, &candidate_for_hook).unwrap();
                }
            },
            || {
                list_root_relative_regular_files(
                    &fixture.root,
                    Path::new(""),
                    SecureTreeLimits {
                        maximum_depth: 8,
                        maximum_entries: 100,
                        maximum_files: 100,
                    },
                    |_| true,
                    |path| path.extension().and_then(|value| value.to_str()) == Some("xml"),
                    || Ok(()),
                )
            },
        );
        assert!(result.is_err(), "a raced file symlink must fail closed");
    }

    #[test]
    fn retained_tree_rejects_same_name_identity_replacement() {
        let fixture = Fixture::new();
        let candidate = fixture.root.join("candidate.xml");
        let displaced = fixture.root.join("candidate-original.xml");
        let replacement = fixture.root.join("replacement.xml");
        fs::write(&candidate, b"trusted").unwrap();
        fs::write(&replacement, b"replacement").unwrap();
        let candidate_for_hook = candidate.clone();
        let result = with_secure_tree_test_hook(
            move |phase| {
                if phase == &SecureTreePhase::BeforeRebindEntry(PathBuf::from("candidate.xml")) {
                    fs::rename(&candidate_for_hook, &displaced).unwrap();
                    fs::rename(&replacement, &candidate_for_hook).unwrap();
                }
            },
            || {
                list_root_relative_regular_files(
                    &fixture.root,
                    Path::new(""),
                    SecureTreeLimits {
                        maximum_depth: 8,
                        maximum_entries: 100,
                        maximum_files: 100,
                    },
                    |_| true,
                    |path| path.extension().and_then(|value| value.to_str()) == Some("xml"),
                    || Ok(()),
                )
            },
        );
        let error = result.expect_err("same-name identity replacement must fail closed");
        assert!(error.to_string().contains("identity changed"), "{error}");
    }

    #[test]
    fn retained_tree_is_sorted_bounded_and_checkpointed() {
        let fixture = Fixture::new();
        fs::write(fixture.root.join("z.xml"), b"z").unwrap();
        fs::write(fixture.root.join("a.xml"), b"a").unwrap();
        let listed = list_root_relative_regular_files(
            &fixture.root,
            Path::new(""),
            SecureTreeLimits {
                maximum_depth: 8,
                maximum_entries: 100,
                maximum_files: 100,
            },
            |_| true,
            |path| path.extension().and_then(|value| value.to_str()) == Some("xml"),
            || Ok(()),
        )
        .unwrap();
        assert_eq!(
            listed
                .files
                .iter()
                .map(|entry| entry.logical_path.as_str())
                .collect::<Vec<_>>(),
            vec!["a.xml", "parent/resource.xml", "z.xml"]
        );
        let cap = list_root_relative_regular_files(
            &fixture.root,
            Path::new(""),
            SecureTreeLimits {
                maximum_depth: 8,
                maximum_entries: 2,
                maximum_files: 100,
            },
            |_| true,
            |_| true,
            || Ok(()),
        );
        assert_eq!(cap.unwrap_err().kind(), io::ErrorKind::FileTooLarge);
        let depth_cap = list_root_relative_regular_files(
            &fixture.root,
            Path::new(""),
            SecureTreeLimits {
                maximum_depth: 0,
                maximum_entries: 100,
                maximum_files: 100,
            },
            |_| true,
            |_| true,
            || Ok(()),
        );
        assert_eq!(depth_cap.unwrap_err().kind(), io::ErrorKind::FileTooLarge);
        let cancelled = list_root_relative_regular_files(
            &fixture.root,
            Path::new(""),
            SecureTreeLimits {
                maximum_depth: 8,
                maximum_entries: 100,
                maximum_files: 100,
            },
            |_| true,
            |_| true,
            || Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled")),
        );
        assert_eq!(cancelled.unwrap_err().kind(), io::ErrorKind::Interrupted);
    }

    #[test]
    fn retained_tree_rejects_a_selected_special_file() {
        use std::os::unix::ffi::OsStrExt;

        let fixture = Fixture::new();
        let fifo = fixture.root.join("candidate.xml");
        let fifo_name = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
        // SAFETY: fifo_name is a live NUL-terminated pathname and mode is a valid permission mask.
        assert_eq!(unsafe { libc::mkfifo(fifo_name.as_ptr(), 0o600) }, 0);

        let result = list_root_relative_regular_files(
            &fixture.root,
            Path::new(""),
            SecureTreeLimits {
                maximum_depth: 8,
                maximum_entries: 100,
                maximum_files: 100,
            },
            |_| true,
            |path| path.extension().and_then(|value| value.to_str()) == Some("xml"),
            || Ok(()),
        );

        assert!(result.is_err(), "a selected FIFO must fail closed");
    }

    #[test]
    fn retained_tree_rejects_a_static_symlinked_start_directory() {
        let fixture = Fixture::new();
        let outside = fixture.root.join("outside-start");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("outside.xml"), b"outside").unwrap();
        symlink(&outside, fixture.root.join("Documents")).unwrap();

        let result = list_root_relative_regular_files(
            &fixture.root,
            Path::new("Documents"),
            SecureTreeLimits {
                maximum_depth: 0,
                maximum_entries: 100,
                maximum_files: 100,
            },
            |_| false,
            |_| true,
            || Ok(()),
        );

        assert!(
            result.is_err(),
            "a static symlinked scan root must not look missing or be traversed"
        );
    }

    #[test]
    fn checked_file_read_honors_cancellation_checkpoint() {
        let fixture = Fixture::new();
        let result = read_root_relative_regular_file_checked(
            &fixture.root,
            &fixture.root.join("parent/resource.xml"),
            1024,
            || Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled")),
            |_| {},
        );

        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::Interrupted);
    }

    #[test]
    fn windows_identity_rebound_uses_safe_capability_metadata() {
        let source = include_str!("secure_read.rs");
        let windows = source
            .split_once("#[cfg(windows)]\nmod imp")
            .unwrap()
            .1
            .split_once("#[cfg(not(any(unix, windows)))]")
            .unwrap()
            .0;
        assert!(!windows.contains("filesystem::file_identity"));
        assert!(!windows.contains("unsafe"));
        assert!(windows.contains("cap_primitives::fs::Metadata::from_file"));
        assert!(windows.contains("CapabilityMetadataExt::dev"));
        assert!(windows.contains("CapabilityMetadataExt::ino"));
        assert!(windows.contains("volume_serial_number"));
        assert!(windows.contains("file_index"));
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
    use crate::infrastructure::platform::filesystem::{
        open_directory_child_nofollow, open_directory_nofollow, open_regular_child_nofollow,
    };
    use cap_fs_ext::MetadataExt as CapabilityMetadataExt;
    use std::fs::File;
    use std::io::{self, Read};
    use std::path::{Component, Path};

    pub(super) fn read(
        root: &Path,
        path: &Path,
        maximum_bytes: usize,
        checkpoint: &mut impl FnMut() -> io::Result<()>,
        mut phase: impl FnMut(SecureReadPhase),
    ) -> io::Result<SecureRead> {
        checkpoint()?;
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
        let mut parent = root;
        for component in parent_path.components() {
            match component {
                Component::CurDir => {}
                Component::Normal(name) => {
                    parent = open_directory_child_nofollow(&parent, name)?;
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
        let mut file = open_regular_child_nofollow(&parent, name)?;
        let identity = windows_file_identity(&file)?;
        let opened = file.metadata()?;
        phase(SecureReadPhase::BeforeRead);
        if usize::try_from(opened.len()).map_or(true, |length| length > maximum_bytes) {
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "resource exceeds the snapshot byte limit",
            ));
        }
        let mut bytes = Vec::with_capacity(opened.len() as usize);
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            checkpoint()?;
            let read = file.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            bytes.extend_from_slice(&buffer[..read]);
            if bytes.len() > maximum_bytes {
                return Err(io::Error::new(
                    io::ErrorKind::FileTooLarge,
                    "resource exceeds the snapshot byte limit",
                ));
            }
        }
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
        checkpoint()?;
        phase(SecureReadPhase::BeforeRebind);
        let rebound = open_regular_child_nofollow(&parent, name)?;
        if windows_file_identity(&rebound)? != identity {
            return Err(io::Error::other("resource identity changed while reading"));
        }
        Ok(SecureRead { bytes })
    }

    pub(super) fn open_absolute_directory(path: &Path) -> io::Result<File> {
        use std::path::{Component, PathBuf};

        if !path.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "secure root must be absolute",
            ));
        }
        let mut components = path.components();
        let Some(Component::Prefix(prefix)) = components.next() else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "secure Windows root has no volume prefix",
            ));
        };
        let mut namespace_root = PathBuf::from(prefix.as_os_str());
        namespace_root.push(Path::new(r"\"));
        let mut names = Vec::new();
        for component in components {
            match component {
                Component::RootDir | Component::CurDir => {}
                Component::Normal(name) => names.push(name.to_os_string()),
                Component::ParentDir => {
                    names.pop().ok_or_else(|| {
                        io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "secure root escapes the volume root",
                        )
                    })?;
                }
                Component::Prefix(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "secure root contains a second volume prefix",
                    ))
                }
            }
        }
        let mut current = open_directory_nofollow(&namespace_root)?;
        for name in names {
            current = open_directory_child_nofollow(&current, &name)?;
        }
        Ok(current)
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct WindowsFileIdentity {
        volume_serial_number: u64,
        file_index: u64,
    }

    fn windows_file_identity(file: &File) -> io::Result<WindowsFileIdentity> {
        let metadata = cap_primitives::fs::Metadata::from_file(file)?;
        Ok(WindowsFileIdentity {
            volume_serial_number: CapabilityMetadataExt::dev(&metadata),
            file_index: CapabilityMetadataExt::ino(&metadata),
        })
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
        _checkpoint: &mut impl FnMut() -> io::Result<()>,
        _phase: impl FnMut(SecureReadPhase),
    ) -> io::Result<SecureRead> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "secure no-follow reads are unavailable on this platform",
        ))
    }

    pub(super) fn open_absolute_directory(_path: &Path) -> io::Result<std::fs::File> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "secure no-follow directory traversal is unavailable on this platform",
        ))
    }
}
