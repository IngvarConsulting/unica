use crate::infrastructure::platform::filesystem::{
    file_identity, open_any_child_nofollow, open_child_for_secure_tree_use,
    open_directory_child_nofollow, open_regular_child_nofollow, read_directory_names_bounded,
    FileIdentity, OpenedChildKind,
};
use std::collections::HashMap;
use std::ffi::OsString;
use std::fs::File;
use std::io::{self, Read};
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
    StartOpened(PathBuf),
    AfterDirectoryListed(PathBuf),
    BeforeOpenEntry(PathBuf),
    BeforeReadEntry(PathBuf),
    BeforeRebindEntry(PathBuf),
    AfterRebindEntry(PathBuf),
    AfterFinalIdentityProofs,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SecureTreeCaptureLimits {
    pub(crate) maximum_depth: usize,
    pub(crate) maximum_entries: usize,
    pub(crate) maximum_files: usize,
    pub(crate) maximum_bytes: usize,
}

#[derive(Debug)]
pub(crate) struct SecureFileSnapshot {
    pub(crate) files: Vec<SecureFileSnapshotEntry>,
    pub(crate) start_missing: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SecureFileSnapshotEntry {
    pub(crate) logical_path: String,
    pub(crate) bytes: Vec<u8>,
}

#[derive(Debug)]
pub(crate) struct RetainedRootSecureRead {
    root: RetainedDirectoryPath,
    limits: SecureTreeCaptureLimits,
    entry_count: usize,
    file_count: usize,
    total_bytes: usize,
    directories: HashMap<PathBuf, FileIdentity>,
    files: Vec<RetainedRegisteredFile>,
    file_indexes: HashMap<PathBuf, usize>,
    poisoned: bool,
}

#[derive(Debug)]
struct RetainedRegisteredFile {
    logical_path: PathBuf,
    identity: FileIdentity,
    bytes: Vec<u8>,
}

impl RetainedRootSecureRead {
    pub(crate) fn open(
        root: &Path,
        limits: SecureTreeCaptureLimits,
        mut checkpoint: impl FnMut() -> io::Result<()>,
    ) -> io::Result<Self> {
        checkpoint()?;
        let root = imp::open_absolute_directory_path(root)?;
        emit_tree_phase(SecureTreePhase::RootOpened);
        checkpoint()?;
        Ok(Self {
            root,
            limits,
            entry_count: 0,
            file_count: 0,
            total_bytes: 0,
            directories: HashMap::new(),
            files: Vec::new(),
            file_indexes: HashMap::new(),
            poisoned: false,
        })
    }

    pub(crate) fn read_regular_file(
        &mut self,
        logical_path: &Path,
        mut checkpoint: impl FnMut() -> io::Result<()>,
    ) -> io::Result<SecureRead> {
        if self.poisoned {
            return Err(poisoned_secure_read_error());
        }
        let result = self.read_regular_file_unpoisoned(logical_path, &mut checkpoint);
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }

    fn read_regular_file_unpoisoned(
        &mut self,
        logical_path: &Path,
        checkpoint: &mut impl FnMut() -> io::Result<()>,
    ) -> io::Result<SecureRead> {
        checkpoint()?;
        let components = normal_relative_file_components(logical_path)?;
        let logical_path = components.iter().collect::<PathBuf>();
        if let Some(index) = self.file_indexes.get(&logical_path).copied() {
            checkpoint()?;
            return Ok(SecureRead {
                bytes: self.files[index].bytes.clone(),
            });
        }
        let directory_depth = components.len() - 1;
        if directory_depth > self.limits.maximum_depth {
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "retained secure read exceeds the traversal-depth limit",
            ));
        }

        let mut parent = self.root.current().try_clone()?;
        let mut logical_directory = PathBuf::new();
        for name in &components[..directory_depth] {
            checkpoint()?;
            logical_directory.push(name);
            let first_open = !self.directories.contains_key(&logical_directory);
            if first_open {
                self.reserve_entry()?;
            }
            emit_tree_phase(SecureTreePhase::BeforeOpenEntry(logical_directory.clone()));
            let child = open_directory_child_nofollow(&parent, name)?;
            let identity = file_identity(&child)?;
            match self.directories.get(&logical_directory) {
                Some(expected) if *expected != identity => {
                    return Err(io::Error::other(
                        "registered directory identity changed between retained reads",
                    ))
                }
                Some(_) => {}
                None if first_open => {
                    self.directories.insert(logical_directory.clone(), identity);
                }
                None => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "registered directory accounting lost a retained identity",
                    ))
                }
            }
            checkpoint()?;
            emit_tree_phase(SecureTreePhase::BeforeRebindEntry(
                logical_directory.clone(),
            ));
            prove_directory_binding(&parent, name, identity)?;
            emit_tree_phase(SecureTreePhase::AfterRebindEntry(logical_directory.clone()));
            parent = child;
        }

        let name = components
            .last()
            .expect("relative file has a final component");
        self.reserve_entry()?;
        self.reserve_file()?;
        checkpoint()?;
        emit_tree_phase(SecureTreePhase::BeforeOpenEntry(logical_path.clone()));
        let mut file = open_regular_child_nofollow(&parent, name)?;
        let identity = file_identity(&file)?;
        let remaining_bytes = self
            .limits
            .maximum_bytes
            .checked_sub(self.total_bytes)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::FileTooLarge,
                    "retained secure read exceeds the cumulative byte limit",
                )
            })?;
        emit_tree_phase(SecureTreePhase::BeforeReadEntry(logical_path.clone()));
        let bytes = read_open_regular_file(
            &mut file,
            remaining_bytes.min(self.limits.maximum_bytes),
            checkpoint,
        )?;
        checkpoint()?;
        emit_tree_phase(SecureTreePhase::BeforeRebindEntry(logical_path.clone()));
        prove_regular_file_binding(&parent, name, identity)?;
        emit_tree_phase(SecureTreePhase::AfterRebindEntry(logical_path.clone()));

        self.total_bytes = self.total_bytes.checked_add(bytes.len()).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::FileTooLarge,
                "retained secure read byte count overflowed",
            )
        })?;
        let index = self.files.len();
        self.files.push(RetainedRegisteredFile {
            logical_path: logical_path.clone(),
            identity,
            bytes: bytes.clone(),
        });
        self.file_indexes.insert(logical_path, index);
        Ok(SecureRead { bytes })
    }

    pub(crate) fn complete(self, mut checkpoint: impl FnMut() -> io::Result<()>) -> io::Result<()> {
        if self.poisoned {
            return Err(poisoned_secure_read_error());
        }
        checkpoint()?;
        prove_directory_path(&self.root, &mut checkpoint)?;
        for retained in &self.files {
            prove_registered_file(
                self.root.current(),
                &self.directories,
                retained,
                &mut checkpoint,
            )?;
        }
        prove_directory_path(&self.root, &mut checkpoint)?;
        emit_tree_phase(SecureTreePhase::AfterFinalIdentityProofs);
        checkpoint()
    }

    fn reserve_entry(&mut self) -> io::Result<()> {
        if self.entry_count >= self.limits.maximum_entries {
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "retained secure read exceeds the entry-count limit",
            ));
        }
        self.entry_count = self.entry_count.checked_add(1).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::FileTooLarge,
                "retained secure read entry count overflowed",
            )
        })?;
        Ok(())
    }

    fn reserve_file(&mut self) -> io::Result<()> {
        if self.file_count >= self.limits.maximum_files {
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "retained secure read exceeds the file-count limit",
            ));
        }
        self.file_count = self.file_count.checked_add(1).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::FileTooLarge,
                "retained secure read file count overflowed",
            )
        })?;
        Ok(())
    }
}

fn poisoned_secure_read_error() -> io::Error {
    io::Error::other("retained secure read session is poisoned by a prior failure")
}

fn normal_relative_file_components(path: &Path) -> io::Result<Vec<OsString>> {
    use std::path::Component;

    let mut components = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(name) => components.push(name.to_os_string()),
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "retained secure read path must contain only normal relative components",
                ))
            }
        }
    }
    if components.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "retained secure read path has no file name",
        ));
    }
    Ok(components)
}

fn prove_registered_file(
    root: &File,
    directories: &HashMap<PathBuf, FileIdentity>,
    retained: &RetainedRegisteredFile,
    checkpoint: &mut impl FnMut() -> io::Result<()>,
) -> io::Result<()> {
    let components = normal_relative_file_components(&retained.logical_path)?;
    let directory_depth = components.len() - 1;
    let mut parent = root.try_clone()?;
    let mut logical_directory = PathBuf::new();
    for name in &components[..directory_depth] {
        checkpoint()?;
        logical_directory.push(name);
        let expected = directories.get(&logical_directory).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "registered directory identity is missing from terminal proof",
            )
        })?;
        let child = open_directory_child_nofollow(&parent, name)?;
        if file_identity(&child)? != *expected {
            return Err(io::Error::other(
                "registered directory identity changed before terminal proof",
            ));
        }
        prove_directory_binding(&parent, name, *expected)?;
        parent = child;
    }

    checkpoint()?;
    let name = components
        .last()
        .expect("relative file has a final component");
    let mut file = open_regular_child_nofollow(&parent, name)?;
    if file_identity(&file)? != retained.identity {
        return Err(io::Error::other(
            "registered file identity changed before terminal proof",
        ));
    }
    verify_open_regular_file(&mut file, &retained.bytes, checkpoint)?;
    prove_regular_file_binding(&parent, name, retained.identity)
}

fn prove_directory_binding(
    parent: &File,
    name: &std::ffi::OsStr,
    identity: FileIdentity,
) -> io::Result<()> {
    let rebound = open_directory_child_nofollow(parent, name)?;
    if file_identity(&rebound)? != identity {
        return Err(io::Error::other(
            "registered directory identity changed while reading",
        ));
    }
    Ok(())
}

fn prove_regular_file_binding(
    parent: &File,
    name: &std::ffi::OsStr,
    identity: FileIdentity,
) -> io::Result<()> {
    let rebound = open_regular_child_nofollow(parent, name)?;
    if file_identity(&rebound)? != identity {
        return Err(io::Error::other(
            "registered file identity changed while reading",
        ));
    }
    Ok(())
}

#[derive(Debug)]
struct RetainedDirectoryPath {
    handles: Vec<File>,
    names: Vec<OsString>,
    identities: Vec<FileIdentity>,
}

impl RetainedDirectoryPath {
    fn new(namespace_root: File) -> io::Result<Self> {
        let identity = file_identity(&namespace_root)?;
        Ok(Self {
            handles: vec![namespace_root],
            names: Vec::new(),
            identities: vec![identity],
        })
    }

    fn current(&self) -> &File {
        self.handles
            .last()
            .expect("retained directory path always owns its namespace root")
    }

    fn push(&mut self, name: OsString, directory: File) -> io::Result<()> {
        self.identities.push(file_identity(&directory)?);
        self.names.push(name);
        self.handles.push(directory);
        Ok(())
    }
}

#[derive(Debug)]
struct RetainedTree {
    identity: FileIdentity,
    initial_names: Vec<OsString>,
    children: Vec<RetainedChild>,
}

#[derive(Debug)]
struct RetainedChild {
    name: OsString,
    identity: FileIdentity,
    kind: RetainedChildKind,
}

#[derive(Debug)]
enum RetainedChildKind {
    Directory(Option<Box<RetainedTree>>),
    RegularFile(Option<usize>),
    Unsupported,
}

struct CaptureState {
    entry_count: usize,
    total_bytes: usize,
    files: Vec<SecureFileSnapshotEntry>,
}

struct TraversalDirectory {
    file: File,
}

impl TraversalDirectory {
    fn new(file: File) -> io::Result<Self> {
        record_traversal_directory_open()?;
        Ok(Self { file })
    }

    fn file(&self) -> &File {
        &self.file
    }
}

impl Drop for TraversalDirectory {
    fn drop(&mut self) {
        record_traversal_directory_close();
    }
}

#[cfg(test)]
thread_local! {
    static SECURE_TREE_DIRECTORY_HANDLE_LIMIT: std::cell::Cell<Option<usize>> =
        const { std::cell::Cell::new(None) };
    static SECURE_TREE_DIRECTORY_HANDLE_COUNT: std::cell::Cell<usize> =
        const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn record_traversal_directory_open() -> io::Result<()> {
    SECURE_TREE_DIRECTORY_HANDLE_COUNT.with(|count| {
        let next = count.get().checked_add(1).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::FileTooLarge,
                "secure-tree directory handle count overflowed",
            )
        })?;
        let exceeded = SECURE_TREE_DIRECTORY_HANDLE_LIMIT
            .with(|limit| limit.get().is_some_and(|limit| next > limit));
        if exceeded {
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "secure tree exceeded the traversal directory handle limit",
            ));
        }
        count.set(next);
        Ok(())
    })
}

#[cfg(not(test))]
fn record_traversal_directory_open() -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
fn record_traversal_directory_close() {
    SECURE_TREE_DIRECTORY_HANDLE_COUNT.with(|count| {
        count.set(
            count
                .get()
                .checked_sub(1)
                .expect("tracked secure-tree handle count cannot underflow"),
        );
    });
}

#[cfg(not(test))]
fn record_traversal_directory_close() {}

#[cfg(test)]
fn with_secure_tree_directory_handle_limit<T>(limit: usize, action: impl FnOnce() -> T) -> T {
    struct Reset {
        limit: Option<usize>,
        count: usize,
    }
    impl Drop for Reset {
        fn drop(&mut self) {
            SECURE_TREE_DIRECTORY_HANDLE_LIMIT.with(|limit| limit.set(self.limit));
            SECURE_TREE_DIRECTORY_HANDLE_COUNT.with(|count| count.set(self.count));
        }
    }

    let previous_limit = SECURE_TREE_DIRECTORY_HANDLE_LIMIT.with(|slot| slot.replace(Some(limit)));
    let previous_count = SECURE_TREE_DIRECTORY_HANDLE_COUNT.with(|slot| slot.replace(0));
    let _reset = Reset {
        limit: previous_limit,
        count: previous_count,
    };
    action()
}

#[cfg(test)]
type SecureTreeTestHook = Box<dyn FnMut(&SecureTreePhase)>;

#[cfg(test)]
thread_local! {
    static SECURE_TREE_TEST_HOOK: std::cell::RefCell<Option<SecureTreeTestHook>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn with_secure_tree_test_hook<T>(
    hook: impl FnMut(&SecureTreePhase) + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<SecureTreeTestHook>);
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

pub(crate) fn capture_root_relative_regular_files(
    root: &Path,
    start: &Path,
    limits: SecureTreeCaptureLimits,
    mut descend: impl FnMut(&Path) -> bool,
    mut select: impl FnMut(&Path) -> bool,
    mut checkpoint: impl FnMut() -> io::Result<()>,
) -> io::Result<SecureFileSnapshot> {
    checkpoint()?;
    let mut retained_path = imp::open_absolute_directory_path(root)?;
    emit_tree_phase(SecureTreePhase::RootOpened);
    let mut logical_start = PathBuf::new();
    let entry_count = 0usize;
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
        match open_directory_child_nofollow(retained_path.current(), name) {
            Ok(child) => retained_path.push(name.to_os_string(), child)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return prove_missing_start(
                    retained_path,
                    name,
                    limits.maximum_entries,
                    entry_count,
                    &mut checkpoint,
                )
            }
            Err(error) => return Err(error),
        }
    }
    emit_tree_phase(SecureTreePhase::StartOpened(logical_start.clone()));

    let start_handle = TraversalDirectory::new(retained_path.current().try_clone()?)?;
    let mut state = CaptureState {
        entry_count,
        total_bytes: 0,
        files: Vec::new(),
    };
    let tree = capture_directory(
        start_handle,
        &logical_start,
        0,
        limits,
        &mut descend,
        &mut select,
        &mut checkpoint,
        &mut state,
    )?;
    let proof_root = TraversalDirectory::new(retained_path.current().try_clone()?)?;
    prove_tree(&proof_root, &tree, &state.files, &mut checkpoint)?;
    prove_directory_path(&retained_path, &mut checkpoint)?;
    state
        .files
        .sort_by(|left, right| left.logical_path.cmp(&right.logical_path));
    Ok(SecureFileSnapshot {
        files: state.files,
        start_missing: false,
    })
}

fn prove_missing_start(
    retained_path: RetainedDirectoryPath,
    missing_name: &std::ffi::OsStr,
    maximum_entries: usize,
    entry_count: usize,
    checkpoint: &mut impl FnMut() -> io::Result<()>,
) -> io::Result<SecureFileSnapshot> {
    let remaining = maximum_entries.checked_sub(entry_count).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::FileTooLarge,
            "secure tree exceeds the entry-count limit",
        )
    })?;
    let initial_names =
        read_directory_names_bounded(retained_path.current(), remaining, &mut *checkpoint)?;
    if initial_names.iter().any(|name| name == missing_name) {
        return Err(io::Error::other(
            "secure tree start changed while proving absence",
        ));
    }
    match open_directory_child_nofollow(retained_path.current(), missing_name) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Ok(_) => {
            return Err(io::Error::other(
                "secure tree start appeared while proving absence",
            ))
        }
        Err(error) => return Err(error),
    }
    let final_names = read_directory_names_bounded(
        retained_path.current(),
        initial_names.len(),
        &mut *checkpoint,
    )?;
    if final_names != initial_names {
        return Err(io::Error::other(
            "directory membership changed while proving absent secure-tree start",
        ));
    }
    prove_directory_path(&retained_path, checkpoint)?;
    Ok(SecureFileSnapshot {
        files: Vec::new(),
        start_missing: true,
    })
}

#[allow(clippy::too_many_arguments)]
fn capture_directory(
    directory: TraversalDirectory,
    logical_directory: &Path,
    depth: usize,
    limits: SecureTreeCaptureLimits,
    descend: &mut impl FnMut(&Path) -> bool,
    select: &mut impl FnMut(&Path) -> bool,
    checkpoint: &mut impl FnMut() -> io::Result<()>,
    state: &mut CaptureState,
) -> io::Result<RetainedTree> {
    checkpoint()?;
    let directory_identity = file_identity(directory.file())?;
    let remaining_entries = limits
        .maximum_entries
        .checked_sub(state.entry_count)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::FileTooLarge,
                "secure tree exceeds the entry-count limit",
            )
        })?;
    let initial_names =
        read_directory_names_bounded(directory.file(), remaining_entries, &mut *checkpoint)?;
    state.entry_count = state
        .entry_count
        .checked_add(initial_names.len())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::FileTooLarge,
                "secure-tree entry count overflowed",
            )
        })?;
    emit_tree_phase(SecureTreePhase::AfterDirectoryListed(
        logical_directory.to_path_buf(),
    ));
    let mut children = Vec::with_capacity(initial_names.len());
    for name in &initial_names {
        checkpoint()?;
        let logical_path = logical_directory.join(name);
        emit_tree_phase(SecureTreePhase::BeforeOpenEntry(logical_path.clone()));
        let (classification_anchor, kind) = open_any_child_nofollow(directory.file(), name)?;
        let identity = file_identity(&classification_anchor)?;
        let retained_kind = match kind {
            OpenedChildKind::Directory => {
                let should_descend = descend(&logical_path);
                if should_descend && depth >= limits.maximum_depth {
                    return Err(io::Error::new(
                        io::ErrorKind::FileTooLarge,
                        "secure tree exceeds the traversal-depth limit",
                    ));
                }
                let subtree = if should_descend {
                    let typed = open_child_for_secure_tree_use(
                        directory.file(),
                        name,
                        classification_anchor,
                        kind,
                    )?;
                    Some(Box::new(capture_directory(
                        TraversalDirectory::new(typed)?,
                        &logical_path,
                        depth + 1,
                        limits,
                        descend,
                        select,
                        checkpoint,
                        state,
                    )?))
                } else {
                    drop(classification_anchor);
                    None
                };
                RetainedChildKind::Directory(subtree)
            }
            OpenedChildKind::RegularFile => {
                let captured_index = if select(&logical_path) {
                    if state.files.len() >= limits.maximum_files {
                        return Err(io::Error::new(
                            io::ErrorKind::FileTooLarge,
                            "secure tree exceeds the selected-file limit",
                        ));
                    }
                    let remaining_bytes = limits
                        .maximum_bytes
                        .checked_sub(state.total_bytes)
                        .ok_or_else(|| {
                            io::Error::new(
                                io::ErrorKind::FileTooLarge,
                                "secure tree exceeds the captured-byte limit",
                            )
                        })?;
                    emit_tree_phase(SecureTreePhase::BeforeReadEntry(logical_path.clone()));
                    let mut typed = open_child_for_secure_tree_use(
                        directory.file(),
                        name,
                        classification_anchor,
                        kind,
                    )?;
                    let bytes = read_open_regular_file(&mut typed, remaining_bytes, checkpoint)?;
                    state.total_bytes =
                        state.total_bytes.checked_add(bytes.len()).ok_or_else(|| {
                            io::Error::new(
                                io::ErrorKind::FileTooLarge,
                                "secure-tree captured byte count overflowed",
                            )
                        })?;
                    let logical_name = logical_utf8_path(&logical_path)?;
                    let index = state.files.len();
                    state.files.push(SecureFileSnapshotEntry {
                        logical_path: logical_name,
                        bytes,
                    });
                    Some(index)
                } else {
                    drop(classification_anchor);
                    None
                };
                RetainedChildKind::RegularFile(captured_index)
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
                drop(classification_anchor);
                RetainedChildKind::Unsupported
            }
        };
        checkpoint()?;
        emit_tree_phase(SecureTreePhase::BeforeRebindEntry(logical_path.clone()));
        prove_child_binding(directory.file(), name, identity, kind)?;
        emit_tree_phase(SecureTreePhase::AfterRebindEntry(logical_path.clone()));
        children.push(RetainedChild {
            name: name.clone(),
            identity,
            kind: retained_kind,
        });
    }
    Ok(RetainedTree {
        identity: directory_identity,
        initial_names,
        children,
    })
}

fn prove_tree(
    directory: &TraversalDirectory,
    tree: &RetainedTree,
    captured: &[SecureFileSnapshotEntry],
    checkpoint: &mut impl FnMut() -> io::Result<()>,
) -> io::Result<()> {
    checkpoint()?;
    if file_identity(directory.file())? != tree.identity {
        return Err(io::Error::other(
            "directory identity changed before secure-tree final proof",
        ));
    }
    let final_names =
        read_directory_names_bounded(directory.file(), tree.initial_names.len(), &mut *checkpoint)?;
    if final_names != tree.initial_names {
        return Err(io::Error::other(
            "directory membership changed while capturing secure tree",
        ));
    }
    for child in &tree.children {
        checkpoint()?;
        let (classification_anchor, kind) = open_any_child_nofollow(directory.file(), &child.name)?;
        if file_identity(&classification_anchor)? != child.identity
            || !retained_kind_matches(&child.kind, kind)
        {
            return Err(io::Error::other(
                "entry identity changed before secure-tree final proof",
            ));
        }
        match &child.kind {
            RetainedChildKind::Directory(Some(subtree)) => {
                let typed = open_child_for_secure_tree_use(
                    directory.file(),
                    &child.name,
                    classification_anchor,
                    kind,
                )?;
                let typed = TraversalDirectory::new(typed)?;
                prove_tree(&typed, subtree, captured, checkpoint)?;
                prove_child_binding(
                    directory.file(),
                    &child.name,
                    child.identity,
                    OpenedChildKind::Directory,
                )?;
            }
            RetainedChildKind::RegularFile(Some(index)) => {
                let expected = captured.get(*index).ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        "captured file proof index is invalid",
                    )
                })?;
                let mut typed = open_child_for_secure_tree_use(
                    directory.file(),
                    &child.name,
                    classification_anchor,
                    kind,
                )?;
                verify_open_regular_file(&mut typed, &expected.bytes, checkpoint)?;
                prove_child_binding(
                    directory.file(),
                    &child.name,
                    child.identity,
                    OpenedChildKind::RegularFile,
                )?;
            }
            RetainedChildKind::Directory(None)
            | RetainedChildKind::RegularFile(None)
            | RetainedChildKind::Unsupported => drop(classification_anchor),
        }
    }
    Ok(())
}

fn prove_directory_path(
    path: &RetainedDirectoryPath,
    checkpoint: &mut impl FnMut() -> io::Result<()>,
) -> io::Result<()> {
    for (handle, identity) in path.handles.iter().zip(&path.identities) {
        checkpoint()?;
        if file_identity(handle)? != *identity {
            return Err(io::Error::other(
                "retained absolute directory identity changed",
            ));
        }
    }
    for (index, name) in path.names.iter().enumerate() {
        checkpoint()?;
        let rebound = open_directory_child_nofollow(&path.handles[index], name)?;
        if file_identity(&rebound)? != path.identities[index + 1] {
            return Err(io::Error::other(
                "absolute root or secure-tree start identity changed",
            ));
        }
    }
    Ok(())
}

fn read_open_regular_file(
    file: &mut File,
    maximum_bytes: usize,
    checkpoint: &mut impl FnMut() -> io::Result<()>,
) -> io::Result<Vec<u8>> {
    let opened = file.metadata()?;
    if !opened.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "secure-tree entry is not a regular file",
        ));
    }
    if usize::try_from(opened.len()).map_or(true, |length| length > maximum_bytes) {
        return Err(io::Error::new(
            io::ErrorKind::FileTooLarge,
            "secure tree exceeds the captured-byte limit",
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
                "secure tree exceeds the captured-byte limit",
            ));
        }
    }
    let after = file.metadata()?;
    if opened.len() != after.len() || opened.modified().ok() != after.modified().ok() {
        return Err(io::Error::other(
            "secure-tree file changed while capturing bytes",
        ));
    }
    Ok(bytes)
}

fn verify_open_regular_file(
    file: &mut File,
    expected: &[u8],
    checkpoint: &mut impl FnMut() -> io::Result<()>,
) -> io::Result<()> {
    let opened = file.metadata()?;
    if !opened.is_file() || opened.len() != expected.len() as u64 {
        return Err(io::Error::other(
            "secure-tree file bytes changed before final proof",
        ));
    }
    let mut offset = 0usize;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        checkpoint()?;
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        let end = offset.checked_add(read).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "secure-tree byte offset overflowed",
            )
        })?;
        if expected.get(offset..end) != Some(&buffer[..read]) {
            return Err(io::Error::other(
                "secure-tree file bytes changed before final proof",
            ));
        }
        offset = end;
    }
    let after = file.metadata()?;
    if offset != expected.len()
        || opened.len() != after.len()
        || opened.modified().ok() != after.modified().ok()
    {
        return Err(io::Error::other(
            "secure-tree file changed during final proof",
        ));
    }
    Ok(())
}

fn prove_child_binding(
    directory: &File,
    name: &std::ffi::OsStr,
    identity: FileIdentity,
    kind: OpenedChildKind,
) -> io::Result<()> {
    let (rebound, rebound_kind) = open_any_child_nofollow(directory, name)?;
    if file_identity(&rebound)? != identity || rebound_kind != kind {
        return Err(io::Error::other(
            "entry identity changed while capturing secure tree",
        ));
    }
    Ok(())
}

fn retained_kind_matches(retained: &RetainedChildKind, actual: OpenedChildKind) -> bool {
    matches!(
        (retained, actual),
        (RetainedChildKind::Directory(_), OpenedChildKind::Directory)
            | (
                RetainedChildKind::RegularFile(_),
                OpenedChildKind::RegularFile
            )
            | (RetainedChildKind::Unsupported, OpenedChildKind::Unsupported)
    )
}

fn logical_utf8_path(path: &Path) -> io::Result<String> {
    path.components()
        .map(|component| {
            component.as_os_str().to_str().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "secure-tree logical path is not UTF-8",
                )
            })
        })
        .collect::<io::Result<Vec<_>>>()
        .map(|components| components.join("/"))
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
    use super::{relative_path, RetainedDirectoryPath, SecureRead, SecureReadPhase};
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
        open_absolute_directory_path(path)?.current().try_clone()
    }

    pub(super) fn open_absolute_directory_path(path: &Path) -> io::Result<RetainedDirectoryPath> {
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
        let namespace_root = open_directory_nofollow(Path::new("/"))?;
        let mut retained = RetainedDirectoryPath::new(namespace_root)?;
        for name in names {
            let child = open_directory_child_nofollow(retained.current(), &name)?;
            retained.push(name, child)?;
        }
        Ok(retained)
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
                capture_root_relative_regular_files(
                    &fixture.root,
                    Path::new(""),
                    capture_limits(),
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
                capture_root_relative_regular_files(
                    &fixture.root,
                    Path::new(""),
                    capture_limits(),
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
                capture_root_relative_regular_files(
                    &fixture.root,
                    Path::new(""),
                    capture_limits(),
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
        let listed = capture_root_relative_regular_files(
            &fixture.root,
            Path::new(""),
            capture_limits(),
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
        let mut entry_cap = capture_limits();
        entry_cap.maximum_entries = 2;
        let cap = capture_root_relative_regular_files(
            &fixture.root,
            Path::new(""),
            entry_cap,
            |_| true,
            |_| true,
            || Ok(()),
        );
        assert_eq!(cap.unwrap_err().kind(), io::ErrorKind::FileTooLarge);
        let mut depth_limits = capture_limits();
        depth_limits.maximum_depth = 0;
        let depth_cap = capture_root_relative_regular_files(
            &fixture.root,
            Path::new(""),
            depth_limits,
            |_| true,
            |_| true,
            || Ok(()),
        );
        assert_eq!(depth_cap.unwrap_err().kind(), io::ErrorKind::FileTooLarge);
        let cancelled = capture_root_relative_regular_files(
            &fixture.root,
            Path::new(""),
            capture_limits(),
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

        let result = capture_root_relative_regular_files(
            &fixture.root,
            Path::new(""),
            capture_limits(),
            |_| true,
            |path| path.extension().and_then(|value| value.to_str()) == Some("xml"),
            || Ok(()),
        );

        assert_eq!(
            result.expect_err("a selected FIFO must fail closed").kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn retained_tree_rejects_a_static_symlinked_start_directory() {
        let fixture = Fixture::new();
        let outside = fixture.root.join("outside-start");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("outside.xml"), b"outside").unwrap();
        symlink(&outside, fixture.root.join("Documents")).unwrap();

        let mut limits = capture_limits();
        limits.maximum_depth = 0;
        let result = capture_root_relative_regular_files(
            &fixture.root,
            Path::new("Documents"),
            limits,
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

    fn capture_limits() -> SecureTreeCaptureLimits {
        SecureTreeCaptureLimits {
            maximum_depth: 8,
            maximum_entries: 100,
            maximum_files: 100,
            maximum_bytes: 1024,
        }
    }

    #[test]
    fn captured_tree_returns_exact_bytes_only_after_a_stable_complete_proof() {
        let fixture = Fixture::new();
        fs::write(fixture.root.join("a.xml"), b"alpha").unwrap();
        fs::write(fixture.root.join("z.xml"), b"zeta").unwrap();

        let captured = capture_root_relative_regular_files(
            &fixture.root,
            Path::new(""),
            capture_limits(),
            |_| true,
            |path| path.extension().and_then(|value| value.to_str()) == Some("xml"),
            || Ok(()),
        )
        .unwrap();

        assert!(!captured.start_missing);
        assert_eq!(
            captured
                .files
                .iter()
                .map(|file| (file.logical_path.as_str(), file.bytes.as_slice()))
                .collect::<Vec<_>>(),
            vec![
                ("a.xml", b"alpha".as_slice()),
                ("parent/resource.xml", b"trusted".as_slice()),
                ("z.xml", b"zeta".as_slice()),
            ]
        );
    }

    #[test]
    fn captured_tree_rejects_a_new_file_created_after_listing_before_bytes() {
        let fixture = Fixture::new();
        fs::write(fixture.root.join("candidate.xml"), b"trusted-candidate").unwrap();
        let inserted = fixture.root.join("inserted.xml");
        let result = with_secure_tree_test_hook(
            move |phase| {
                if phase == &SecureTreePhase::AfterDirectoryListed(PathBuf::new()) {
                    fs::write(&inserted, b"must-not-be-published").unwrap();
                }
            },
            || {
                capture_root_relative_regular_files(
                    &fixture.root,
                    Path::new(""),
                    capture_limits(),
                    |_| true,
                    |path| path.extension().and_then(|value| value.to_str()) == Some("xml"),
                    || Ok(()),
                )
            },
        );

        assert!(result.is_err(), "a post-listing create must fail closed");
    }

    #[test]
    fn captured_tree_rejects_absolute_root_same_name_replacement() {
        let fixture = Fixture::new();
        let root = fixture.root.clone();
        let displaced = root.with_extension("original-root");
        let root_for_hook = root.clone();
        let displaced_for_hook = displaced.clone();
        let result = with_secure_tree_test_hook(
            move |phase| {
                if phase == &SecureTreePhase::RootOpened {
                    fs::rename(&root_for_hook, &displaced_for_hook).unwrap();
                    fs::create_dir(&root_for_hook).unwrap();
                    fs::write(root_for_hook.join("outside.xml"), b"outside-root").unwrap();
                }
            },
            || {
                capture_root_relative_regular_files(
                    &root,
                    Path::new(""),
                    capture_limits(),
                    |_| true,
                    |_| true,
                    || Ok(()),
                )
            },
        );
        fs::remove_dir_all(&root).unwrap();
        fs::rename(&displaced, &root).unwrap();

        assert!(
            result.is_err(),
            "same-name root replacement must fail closed"
        );
    }

    #[test]
    fn captured_tree_rejects_start_directory_same_name_replacement() {
        let fixture = Fixture::new();
        let start = fixture.root.join("Documents");
        let displaced = fixture.root.join("Documents-original");
        fs::create_dir(&start).unwrap();
        fs::write(start.join("trusted.xml"), b"trusted-document").unwrap();
        let start_for_hook = start.clone();
        let displaced_for_hook = displaced.clone();
        let result = with_secure_tree_test_hook(
            move |phase| {
                if phase == &SecureTreePhase::StartOpened(PathBuf::from("Documents")) {
                    fs::rename(&start_for_hook, &displaced_for_hook).unwrap();
                    fs::create_dir(&start_for_hook).unwrap();
                    fs::write(start_for_hook.join("outside.xml"), b"outside-start").unwrap();
                }
            },
            || {
                capture_root_relative_regular_files(
                    &fixture.root,
                    Path::new("Documents"),
                    capture_limits(),
                    |_| false,
                    |_| true,
                    || Ok(()),
                )
            },
        );
        fs::remove_dir_all(&start).unwrap();
        fs::rename(&displaced, &start).unwrap();

        assert!(
            result.is_err(),
            "same-name start replacement must fail closed"
        );
    }

    #[test]
    fn captured_tree_final_proof_rejects_early_child_replacement_after_immediate_rebind() {
        let fixture = Fixture::new();
        let early = fixture.root.join("a-early.xml");
        let displaced = fixture.root.join("a-early-original.xml");
        let replacement = fixture.root.join("replacement.bin");
        fs::write(&early, b"trusted-early").unwrap();
        fs::write(fixture.root.join("z-late.xml"), b"trusted-late").unwrap();
        fs::write(&replacement, b"outside-early").unwrap();
        let early_for_hook = early.clone();
        let result = with_secure_tree_test_hook(
            move |phase| {
                if phase == &SecureTreePhase::AfterRebindEntry(PathBuf::from("a-early.xml")) {
                    fs::rename(&early_for_hook, &displaced).unwrap();
                    fs::rename(&replacement, &early_for_hook).unwrap();
                }
            },
            || {
                capture_root_relative_regular_files(
                    &fixture.root,
                    Path::new(""),
                    capture_limits(),
                    |_| true,
                    |path| path.extension().and_then(|value| value.to_str()) == Some("xml"),
                    || Ok(()),
                )
            },
        );

        assert!(
            result.is_err(),
            "an early same-name swap must fail final proof"
        );
    }

    #[test]
    fn captured_tree_final_proof_rejects_same_identity_content_mutation() {
        let fixture = Fixture::new();
        let candidate = fixture.root.join("a-early.xml");
        fs::write(&candidate, b"trusted-early").unwrap();
        fs::write(fixture.root.join("z-late.xml"), b"trusted-late").unwrap();
        let candidate_for_hook = candidate.clone();
        let result = with_secure_tree_test_hook(
            move |phase| {
                if phase == &SecureTreePhase::AfterRebindEntry(PathBuf::from("a-early.xml")) {
                    fs::write(&candidate_for_hook, b"mutated-early").unwrap();
                }
            },
            || {
                capture_root_relative_regular_files(
                    &fixture.root,
                    Path::new(""),
                    capture_limits(),
                    |_| true,
                    |path| path.extension().and_then(|value| value.to_str()) == Some("xml"),
                    || Ok(()),
                )
            },
        );

        assert!(
            result.is_err(),
            "same-identity byte mutation must fail final proof"
        );
    }

    #[test]
    fn retained_registration_session_revalidates_file_identity_at_terminal_proof() {
        let fixture = Fixture::new();
        let path = fixture.root.join("parent/resource.xml");
        let displaced = fixture.root.join("parent/original.xml");
        let replacement = fixture.root.join("replacement.xml");
        fs::write(&replacement, b"replacement").unwrap();
        let mut session =
            RetainedRootSecureRead::open(&fixture.root, capture_limits(), || Ok(())).unwrap();
        assert_eq!(
            session
                .read_regular_file(Path::new("parent/resource.xml"), || Ok(()))
                .unwrap()
                .bytes,
            b"trusted"
        );
        fs::rename(&path, &displaced).unwrap();
        fs::rename(&replacement, &path).unwrap();

        let error = session
            .complete(|| Ok(()))
            .expect_err("a same-name registered file replacement must fail terminal proof");

        assert!(error.to_string().contains("identity"), "{error}");
    }

    #[test]
    fn retained_registration_session_revalidates_directory_identity_at_terminal_proof() {
        let fixture = Fixture::new();
        let parent = fixture.root.join("parent");
        let displaced = fixture.root.join("parent-original");
        let mut session =
            RetainedRootSecureRead::open(&fixture.root, capture_limits(), || Ok(())).unwrap();
        session
            .read_regular_file(Path::new("parent/resource.xml"), || Ok(()))
            .unwrap();
        fs::rename(&parent, &displaced).unwrap();
        fs::create_dir(&parent).unwrap();
        fs::write(parent.join("resource.xml"), b"trusted").unwrap();

        let error = session
            .complete(|| Ok(()))
            .expect_err("a same-name registered directory replacement must fail terminal proof");

        assert!(error.to_string().contains("identity"), "{error}");
    }

    #[test]
    fn retained_registration_session_spends_budgets_only_on_requested_files() {
        let fixture = Fixture::new();
        fs::write(fixture.root.join("unrelated.xml"), vec![b'x'; 2_048]).unwrap();
        symlink(
            fixture.root.join("unrelated.xml"),
            fixture.root.join("unrelated-link.xml"),
        )
        .unwrap();
        let limits = SecureTreeCaptureLimits {
            maximum_depth: 1,
            maximum_entries: 2,
            maximum_files: 1,
            maximum_bytes: b"trusted".len(),
        };
        let mut session = RetainedRootSecureRead::open(&fixture.root, limits, || Ok(())).unwrap();

        let read = session
            .read_regular_file(Path::new("parent/resource.xml"), || Ok(()))
            .expect("unrequested files and links must not spend registration budgets");

        assert_eq!(read.bytes, b"trusted");
        session.complete(|| Ok(())).unwrap();
    }

    #[test]
    fn retained_registration_session_counts_each_new_directory_and_file_path() {
        let fixture = Fixture::new();
        let limits = SecureTreeCaptureLimits {
            maximum_depth: 1,
            maximum_entries: 1,
            maximum_files: 1,
            maximum_bytes: b"trusted".len(),
        };
        let mut session = RetainedRootSecureRead::open(&fixture.root, limits, || Ok(())).unwrap();

        let error = session
            .read_regular_file(Path::new("parent/resource.xml"), || Ok(()))
            .expect_err("the parent directory and file must spend two entry slots");

        assert_eq!(error.kind(), io::ErrorKind::FileTooLarge);
        assert!(error.to_string().contains("entry-count"), "{error}");
    }

    #[test]
    fn retained_registration_session_counts_new_files_cumulatively_under_a_retained_directory() {
        let fixture = Fixture::new();
        fs::write(fixture.root.join("parent/second.xml"), b"second").unwrap();
        let limits = SecureTreeCaptureLimits {
            maximum_depth: 1,
            maximum_entries: 2,
            maximum_files: 2,
            maximum_bytes: b"trusted".len() + b"second".len(),
        };
        let mut session = RetainedRootSecureRead::open(&fixture.root, limits, || Ok(())).unwrap();
        session
            .read_regular_file(Path::new("parent/resource.xml"), || Ok(()))
            .unwrap();

        let error = session
            .read_regular_file(Path::new("parent/second.xml"), || Ok(()))
            .expect_err("a second file must spend a third cumulative entry slot");

        assert_eq!(error.kind(), io::ErrorKind::FileTooLarge);
        assert!(error.to_string().contains("entry-count"), "{error}");
    }

    #[test]
    fn retained_registration_session_is_poisoned_after_a_failed_read() {
        let fixture = Fixture::new();
        fs::write(fixture.root.join("small.xml"), b"x").unwrap();
        let limits = SecureTreeCaptureLimits {
            maximum_depth: 1,
            maximum_entries: 4,
            maximum_files: 2,
            maximum_bytes: b"trusted".len() - 1,
        };
        let mut session = RetainedRootSecureRead::open(&fixture.root, limits, || Ok(())).unwrap();
        let first = session
            .read_regular_file(Path::new("parent/resource.xml"), || Ok(()))
            .expect_err("the first registered file exceeds the byte budget");
        assert_eq!(first.kind(), io::ErrorKind::FileTooLarge);

        let second = session
            .read_regular_file(Path::new("small.xml"), || Ok(()))
            .expect_err("a failed partial read must make the capability unusable");
        assert!(second.to_string().contains("poisoned"), "{second}");

        let complete = session
            .complete(|| Ok(()))
            .expect_err("a poisoned capability cannot publish a proof");
        assert!(complete.to_string().contains("poisoned"), "{complete}");
    }

    #[test]
    fn captured_tree_reads_from_open_file_identity_and_rejects_per_file_swap() {
        let fixture = Fixture::new();
        let candidate = fixture.root.join("candidate.xml");
        let displaced = fixture.root.join("candidate-original.xml");
        let replacement = fixture.root.join("replacement.bin");
        fs::write(&candidate, b"trusted-candidate").unwrap();
        fs::write(&replacement, b"outside-candidate").unwrap();
        let candidate_for_hook = candidate.clone();
        let result = with_secure_tree_test_hook(
            move |phase| {
                if phase == &SecureTreePhase::BeforeReadEntry(PathBuf::from("candidate.xml")) {
                    fs::rename(&candidate_for_hook, &displaced).unwrap();
                    fs::rename(&replacement, &candidate_for_hook).unwrap();
                }
            },
            || {
                capture_root_relative_regular_files(
                    &fixture.root,
                    Path::new(""),
                    capture_limits(),
                    |_| true,
                    |path| path.extension().and_then(|value| value.to_str()) == Some("xml"),
                    || Ok(()),
                )
            },
        );

        assert!(result.is_err(), "a per-file identity swap must fail closed");
    }

    #[test]
    fn captured_tree_enforces_total_bytes_and_cancels_during_entry_enumeration() {
        let fixture = Fixture::new();
        for index in 0..32 {
            fs::write(fixture.root.join(format!("{index:02}.xml")), b"payload").unwrap();
        }
        let mut tiny = capture_limits();
        tiny.maximum_bytes = 8;
        let oversized = capture_root_relative_regular_files(
            &fixture.root,
            Path::new(""),
            tiny,
            |_| true,
            |_| true,
            || Ok(()),
        );
        assert_eq!(oversized.unwrap_err().kind(), io::ErrorKind::FileTooLarge);

        let checkpoints = std::cell::Cell::new(0usize);
        let cancelled = capture_root_relative_regular_files(
            &fixture.root,
            Path::new(""),
            capture_limits(),
            |_| true,
            |_| true,
            || {
                let next = checkpoints.get() + 1;
                checkpoints.set(next);
                if next == 6 {
                    Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"))
                } else {
                    Ok(())
                }
            },
        );
        assert_eq!(cancelled.unwrap_err().kind(), io::ErrorKind::Interrupted);
        assert_eq!(checkpoints.get(), 6);
    }

    #[test]
    fn captured_wide_tree_uses_only_depth_bounded_directory_handles() {
        let fixture = Fixture::new();
        for index in 0..1_024 {
            fs::create_dir(fixture.root.join(format!("wide-{index:04}"))).unwrap();
        }
        let limits = SecureTreeCaptureLimits {
            maximum_depth: 1,
            maximum_entries: 2_048,
            maximum_files: 0,
            maximum_bytes: 0,
        };

        let captured = with_secure_tree_directory_handle_limit(8, || {
            capture_root_relative_regular_files(
                &fixture.root,
                Path::new(""),
                limits,
                |_| true,
                |_| false,
                || Ok(()),
            )
        })
        .expect("wide sibling trees must not retain one handle per directory");

        assert!(captured.files.is_empty());
        assert!(!captured.start_missing);
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

    #[test]
    fn windows_secure_tree_reopens_typed_handles_before_directory_or_file_use() {
        let secure_read = include_str!("secure_read.rs");
        let filesystem = include_str!("filesystem.rs");

        assert!(secure_read.contains("open_child_for_secure_tree_use"));
        assert!(filesystem.contains("typed child identity differs from its classification anchor"));
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
    fn captured_tree_enumerates_and_reads_through_typed_windows_handles() {
        let fixture = Fixture::new();
        let captured = capture_root_relative_regular_files(
            &fixture.root,
            Path::new(""),
            SecureTreeCaptureLimits {
                maximum_depth: 4,
                maximum_entries: 16,
                maximum_files: 4,
                maximum_bytes: 1_024,
            },
            |_| true,
            |path| path.extension().and_then(|value| value.to_str()) == Some("xml"),
            || Ok(()),
        )
        .unwrap();

        assert_eq!(captured.files.len(), 1);
        assert_eq!(captured.files[0].logical_path, "parent/resource.xml");
        assert_eq!(captured.files[0].bytes, b"trusted");
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
    use super::{relative_path, RetainedDirectoryPath, SecureRead, SecureReadPhase};
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
        open_absolute_directory_path(path)?.current().try_clone()
    }

    pub(super) fn open_absolute_directory_path(path: &Path) -> io::Result<RetainedDirectoryPath> {
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
        let namespace_root = open_directory_nofollow(&namespace_root)?;
        let mut retained = RetainedDirectoryPath::new(namespace_root)?;
        for name in names {
            let child = open_directory_child_nofollow(retained.current(), &name)?;
            retained.push(name, child)?;
        }
        Ok(retained)
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
    use super::{RetainedDirectoryPath, SecureRead, SecureReadPhase};
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

    pub(super) fn open_absolute_directory_path(_path: &Path) -> io::Result<RetainedDirectoryPath> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "secure no-follow directory traversal is unavailable on this platform",
        ))
    }
}
