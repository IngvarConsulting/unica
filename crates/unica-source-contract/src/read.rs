//! Owned source observations and read-only ports. Opening ambient paths and
//! every mutation remain responsibilities of the issuing workspace owner.
use std::ffi::{OsStr, OsString};
use std::fmt::Debug;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FileIdentity {
    volume: u64,
    file: u64,
}

impl FileIdentity {
    pub const fn from_observation(volume: u64, file: u64) -> Self {
        Self { volume, file }
    }

    pub fn stable_bytes(self) -> [u8; 16] {
        let mut bytes = [0_u8; 16];
        bytes[..8].copy_from_slice(&self.volume.to_le_bytes());
        bytes[8..].copy_from_slice(&self.file.to_le_bytes());
        bytes
    }
}

/// The same physical, length and timestamp facts used by retained-input caches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStamp {
    identity: FileIdentity,
    length: u64,
    modified: Option<SystemTime>,
    changed: Option<SystemTime>,
}

impl FileStamp {
    pub const fn from_observation(
        identity: FileIdentity,
        length: u64,
        modified: Option<SystemTime>,
        changed: Option<SystemTime>,
    ) -> Self {
        Self {
            identity,
            length,
            modified,
            changed,
        }
    }

    pub fn settled(&self, now: SystemTime, window: Duration) -> bool {
        [self.modified, self.changed].into_iter().all(|time| {
            time.is_some_and(|time| now.duration_since(time).is_ok_and(|age| age >= window))
        })
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn for_test(
        file: u64,
        length: u64,
        modified: Option<SystemTime>,
        changed: Option<SystemTime>,
    ) -> Self {
        Self::from_observation(
            FileIdentity::from_observation(1, file),
            length,
            modified,
            changed,
        )
    }
}

/// The opened-source checks consume only regular-file kind and exact length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileMetadata {
    length: u64,
    regular_file: bool,
}

impl FileMetadata {
    pub const fn from_observation(length: u64, regular_file: bool) -> Self {
        Self {
            length,
            regular_file,
        }
    }
    pub const fn len(&self) -> u64 {
        self.length
    }
    pub const fn is_empty(&self) -> bool {
        self.length == 0
    }
    pub const fn is_file(&self) -> bool {
        self.regular_file
    }
}

pub trait FileReadPort: Read + Seek + Debug + Send + Sync {
    fn metadata(&self) -> io::Result<FileMetadata>;
    fn identity(&self) -> io::Result<FileIdentity>;
    fn stamp(&self) -> io::Result<FileStamp>;
    fn hard_link_count(&self) -> io::Result<u64>;
    fn try_clone_reader(&self) -> io::Result<ReadFile>;
    fn read_shared(&self, bytes: &mut [u8]) -> io::Result<usize>;
    fn seek_shared(&self, position: SeekFrom) -> io::Result<u64>;
}

#[derive(Debug)]
pub struct ReadFile(Box<dyn FileReadPort>);

impl ReadFile {
    pub fn from_port(port: impl FileReadPort + 'static) -> Self {
        Self(Box::new(port))
    }
    pub fn metadata(&self) -> io::Result<FileMetadata> {
        self.0.metadata()
    }
    pub fn identity(&self) -> io::Result<FileIdentity> {
        self.0.identity()
    }
    pub fn stamp(&self) -> io::Result<FileStamp> {
        self.0.stamp()
    }
    pub fn hard_link_count(&self) -> io::Result<u64> {
        self.0.hard_link_count()
    }
    pub fn try_clone(&self) -> io::Result<Self> {
        self.0.try_clone_reader()
    }
}

impl Read for ReadFile {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.0.read(bytes)
    }
}
impl Seek for ReadFile {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.0.seek(position)
    }
}

impl Read for &ReadFile {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.0.read_shared(bytes)
    }
}

impl Seek for &ReadFile {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.0.seek_shared(position)
    }
}

pub trait SourceFileReadPort: Debug + Send + Sync {
    fn identity(&self) -> FileIdentity;
    fn read_bounded(&self, maximum_bytes: usize) -> io::Result<Vec<u8>>;
    fn try_clone_file(&self) -> io::Result<ReadFile>;
    fn hard_link_count(&self) -> io::Result<u64>;
    fn validate_named_identity(&self) -> io::Result<()>;
    fn open_named_identity_for_read(&self) -> io::Result<ReadFile>;
    fn validate_named_identity_relative(&self) -> io::Result<()>;
}

#[derive(Debug, Clone)]
pub struct SourceFile(Arc<dyn SourceFileReadPort>);

impl SourceFile {
    pub fn from_port(port: impl SourceFileReadPort + 'static) -> Self {
        Self(Arc::new(port))
    }
    pub fn identity(&self) -> FileIdentity {
        self.0.identity()
    }
    pub fn read_bounded(&self, maximum_bytes: usize) -> io::Result<Vec<u8>> {
        self.0.read_bounded(maximum_bytes)
    }
    pub fn try_clone_file(&self) -> io::Result<ReadFile> {
        self.0.try_clone_file()
    }
    pub fn hard_link_count(&self) -> io::Result<u64> {
        self.0.hard_link_count()
    }
    pub fn validate_named_identity(&self) -> io::Result<()> {
        self.0.validate_named_identity()
    }
    pub fn open_named_identity_for_read(&self) -> io::Result<ReadFile> {
        self.0.open_named_identity_for_read()
    }
    pub fn validate_named_identity_relative(&self) -> io::Result<()> {
        self.0.validate_named_identity_relative()
    }
}

pub trait SourceNameComparison: Send + Sync {
    fn names_equivalent(&self, left: &OsStr, right: &OsStr) -> io::Result<bool>;
}

pub struct SourceNameComparator(Box<dyn SourceNameComparison>);

impl SourceNameComparator {
    pub fn from_port(port: impl SourceNameComparison + 'static) -> Self {
        Self(Box::new(port))
    }
    pub fn names_equivalent(&self, left: &OsStr, right: &OsStr) -> io::Result<bool> {
        self.0.names_equivalent(left, right)
    }
}

#[derive(Debug)]
pub enum SourceEntry {
    Directory(SourceDirectory),
    RegularFile(SourceFile),
    ReparsePoint,
    Unsupported,
}

pub trait SourceDirectoryReadPort: Debug + Send + Sync {
    fn path(&self) -> &Path;
    fn identity(&self) -> FileIdentity;
    fn named_identity_matches(&self) -> io::Result<bool>;
    fn validate_named_identity(&self) -> io::Result<()>;
    fn child_names_equivalent(&self, left: &OsStr, right: &OsStr) -> io::Result<bool>;
    fn child_name_comparator(&self) -> io::Result<SourceNameComparator>;
    fn retain_directory_child(&self, name: &OsStr) -> io::Result<SourceDirectory>;
    fn read_relative_regular_bounded(
        &self,
        relative: &Path,
        maximum_bytes: usize,
    ) -> io::Result<Vec<u8>>;
    fn visit_relative_regular_chunks(
        &self,
        relative: &Path,
        checkpoint: &mut dyn FnMut() -> io::Result<()>,
        visit: &mut dyn FnMut(&[u8]) -> io::Result<()>,
    ) -> io::Result<u64>;
    fn read_relative_regular_prefix(
        &self,
        relative: &Path,
        maximum_bytes: usize,
    ) -> io::Result<Vec<u8>>;
    fn relative_file_stamp(&self, relative: &Path) -> io::Result<Option<FileStamp>>;
    fn open_relative_regular_nofollow(&self, relative: &Path) -> io::Result<ReadFile>;
    fn read_immediate_names_with_limit(
        &self,
        maximum_entries: Option<usize>,
        checkpoint: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<Vec<OsString>>;
    fn visit_immediate_names(
        &self,
        visitor: &mut dyn FnMut(OsString) -> io::Result<()>,
    ) -> io::Result<()>;
    fn retain_immediate_child_nofollow(&self, name: &OsStr) -> io::Result<SourceEntry>;
    fn retain_regular_child(&self, name: &OsStr) -> io::Result<SourceFile>;
}

/// No ambient path constructor, mutation, raw handle, or Deref is available.
#[derive(Debug, Clone)]
pub struct SourceDirectory(Arc<dyn SourceDirectoryReadPort>);

impl SourceDirectory {
    pub fn from_port(port: impl SourceDirectoryReadPort + 'static) -> Self {
        Self(Arc::new(port))
    }
    pub fn path(&self) -> &Path {
        self.0.path()
    }
    pub fn identity(&self) -> FileIdentity {
        self.0.identity()
    }
    pub fn named_identity_matches(&self) -> io::Result<bool> {
        self.0.named_identity_matches()
    }
    pub fn validate_named_identity(&self) -> io::Result<()> {
        self.0.validate_named_identity()
    }
    pub fn child_names_equivalent(&self, left: &OsStr, right: &OsStr) -> io::Result<bool> {
        self.0.child_names_equivalent(left, right)
    }
    pub fn child_name_comparator(&self) -> io::Result<SourceNameComparator> {
        self.0.child_name_comparator()
    }
    pub fn retain_directory_child(&self, name: &OsStr) -> io::Result<Self> {
        self.0.retain_directory_child(name)
    }
    pub fn read_relative_regular_bounded(
        &self,
        relative: &Path,
        maximum_bytes: usize,
    ) -> io::Result<Vec<u8>> {
        self.0
            .read_relative_regular_bounded(relative, maximum_bytes)
    }
    pub fn visit_relative_regular_chunks(
        &self,
        relative: &Path,
        mut checkpoint: impl FnMut() -> io::Result<()>,
        mut visit: impl FnMut(&[u8]) -> io::Result<()>,
    ) -> io::Result<u64> {
        self.0
            .visit_relative_regular_chunks(relative, &mut checkpoint, &mut visit)
    }
    pub fn read_relative_regular_prefix(
        &self,
        relative: &Path,
        maximum_bytes: usize,
    ) -> io::Result<Vec<u8>> {
        self.0.read_relative_regular_prefix(relative, maximum_bytes)
    }
    pub fn relative_file_stamp(&self, relative: &Path) -> io::Result<Option<FileStamp>> {
        self.0.relative_file_stamp(relative)
    }
    pub fn open_relative_regular_nofollow(&self, relative: &Path) -> io::Result<ReadFile> {
        self.0.open_relative_regular_nofollow(relative)
    }
    pub fn read_immediate_names_bounded(
        &self,
        maximum_entries: usize,
        checkpoint: impl FnMut() -> io::Result<()>,
    ) -> io::Result<Vec<OsString>> {
        self.read_immediate_names_with_limit(Some(maximum_entries), checkpoint)
    }
    pub fn read_immediate_names_with_limit(
        &self,
        maximum_entries: Option<usize>,
        mut checkpoint: impl FnMut() -> io::Result<()>,
    ) -> io::Result<Vec<OsString>> {
        self.0
            .read_immediate_names_with_limit(maximum_entries, &mut checkpoint)
    }
    pub fn visit_immediate_names<E: From<io::Error>>(
        &self,
        mut visitor: impl FnMut(OsString) -> Result<(), E>,
    ) -> Result<(), E> {
        let mut stopped = None;
        let result = self
            .0
            .visit_immediate_names(&mut |name| match visitor(name) {
                Ok(()) => Ok(()),
                Err(error) => {
                    stopped = Some(error);
                    Err(io::Error::other("source directory visitor stopped"))
                }
            });
        if let Some(error) = stopped {
            return Err(error);
        }
        result.map_err(E::from)
    }
    pub fn retain_immediate_child_nofollow(&self, name: &OsStr) -> io::Result<SourceEntry> {
        self.0.retain_immediate_child_nofollow(name)
    }
    pub fn retain_regular_child(&self, name: &OsStr) -> io::Result<SourceFile> {
        self.0.retain_regular_child(name)
    }
}

/// Exact observations made by the retained source-selection pass. The pass
/// keeps physical evidence; the discovery backend receives only these reads.
pub enum SelectionRegularObservation {
    Exact(Arc<[u8]>),
    Present,
    Oversized,
    Absent,
    WrongKind,
}

pub enum SelectionDirectoryObservation {
    Present(SourceDirectory),
    AbsentOrWrongKind,
}

pub struct SelectionMembershipEntry {
    pub name: OsString,
    pub directory: Option<SourceDirectory>,
}

pub trait SourceSelectionReadPass {
    fn workspace_path(&self) -> &Path;
    fn remaining_member_budget(&self) -> Option<usize>;
    fn observe_regular(
        &mut self,
        relative: &Path,
        maximum_bytes: usize,
        checkpoint: &mut dyn FnMut() -> Result<(), String>,
    ) -> Result<SelectionRegularObservation, String>;
    fn observe_regular_unbounded(
        &mut self,
        relative: &Path,
        checkpoint: &mut dyn FnMut() -> Result<(), String>,
    ) -> Result<SelectionRegularObservation, String>;
    fn observe_regular_presence(
        &mut self,
        relative: &Path,
        checkpoint: &mut dyn FnMut() -> Result<(), String>,
    ) -> Result<SelectionRegularObservation, String>;
    fn observe_directory(
        &mut self,
        relative: &Path,
        checkpoint: &mut dyn FnMut() -> Result<(), String>,
    ) -> Result<SelectionDirectoryObservation, String>;
    fn observe_membership_with_limit(
        &mut self,
        relative: &Path,
        directory: &SourceDirectory,
        maximum_entries: Option<usize>,
        checkpoint: &mut dyn FnMut() -> Result<(), String>,
    ) -> Result<Vec<SelectionMembershipEntry>, String>;
}

/// The same backend is reinvoked over fresh owner-captured evidence at both
/// admission passes and the final post-publication semantic recheck.
pub trait SourceMapDiscovery: Send + Sync {
    fn discover(
        &self,
        pass: &mut dyn SourceSelectionReadPass,
        checkpoint: &mut dyn FnMut() -> Result<(), String>,
    ) -> Result<crate::ProjectSourceMap, String>;
}

/// Reads registered files through one retained owner-issued session and then
/// rechecks that session. The consumer cannot open a root or publish bytes.
pub trait RetainedSourceRead {
    fn read_regular_file(
        &mut self,
        relative: &Path,
        checkpoint: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<Vec<u8>>;

    fn complete(self: Box<Self>, checkpoint: &mut dyn FnMut() -> io::Result<()>) -> io::Result<()>;
}

/// Immutable images produced by an owner-guarded prepared source plan.
pub struct SourceImage<'a> {
    pub path_encoding: &'static str,
    pub path: String,
    pub before: Option<&'a [u8]>,
    pub after: Option<&'a [u8]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevisionArtifactDisposition {
    Content,
    Presence,
    Ignored,
}

pub trait BoundSourceArtifactPolicy: Send + Sync {
    fn classify(&self, relative: &Path) -> RevisionArtifactDisposition;
}

pub trait SourceArtifactClassifier: Send + Sync {
    fn bind(
        &self,
        source_kind: crate::SourceSetKind,
        source_format: crate::SourceFormat,
        source_profile: crate::SourceProfile,
    ) -> Result<Arc<dyn BoundSourceArtifactPolicy>, String>;
}

pub mod full_dump;

#[derive(Debug, Clone, Copy)]
pub struct SecureTreeCaptureLimits {
    pub maximum_depth: usize,
    pub maximum_entries: usize,
    pub maximum_files: usize,
    pub maximum_bytes: usize,
}

#[derive(Debug)]
pub struct SecureFileSnapshot {
    pub files: Vec<SecureFileSnapshotEntry>,
    pub start_missing: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecureFileSnapshotEntry {
    pub logical_path: String,
    pub bytes: Vec<u8>,
}
