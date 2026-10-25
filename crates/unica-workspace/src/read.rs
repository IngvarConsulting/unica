//! Read-only access issued by the workspace filesystem owner.
use crate::platform::filesystem;
use std::ffi::{OsStr, OsString};
use std::fs::Metadata;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

#[cfg(any(test, feature = "test-support"))]
pub use crate::platform::secure_read::with_secure_tree_test_hook;
pub use crate::platform::secure_read::{
    capture_root_relative_regular_files, read_root_relative_regular_file,
    read_root_relative_regular_file_checked, visit_root_relative_regular_files,
    RetainedRootSecureRead, SecureRead, SecureReadPhase, SecureTreePhase, SecureTreeVisit,
};
pub use unica_source_contract::read::{
    FileIdentity, FileStamp, SourceDirectory, SourceEntry, SourceFile, SourceNameComparator,
};
use unica_source_contract::read::{
    FileMetadata, FileReadPort, ReadFile as ReadHandle, SourceDirectoryReadPort,
    SourceFileReadPort, SourceNameComparison,
};

impl unica_source_contract::read::RetainedSourceRead for RetainedRootSecureRead {
    fn read_regular_file(
        &mut self,
        relative: &Path,
        checkpoint: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<Vec<u8>> {
        RetainedRootSecureRead::read_regular_file(self, relative, checkpoint)
            .map(|observed| observed.bytes)
    }

    fn complete(self: Box<Self>, checkpoint: &mut dyn FnMut() -> io::Result<()>) -> io::Result<()> {
        RetainedRootSecureRead::complete(*self, checkpoint)
    }
}

/// An opened reader, with neither a writable descriptor nor a raw-handle API.
///
/// ```compile_fail
/// use std::io::Write;
/// fn cannot_publish(mut source: unica_workspace::read::ReadFile) {
///     source.write_all(b"replacement").unwrap();
/// }
/// ```
#[derive(Debug)]
pub struct ReadFile {
    pub(crate) file: std::fs::File,
}

impl ReadFile {
    pub fn metadata(&self) -> io::Result<Metadata> {
        self.file.metadata()
    }

    pub fn identity(&self) -> io::Result<FileIdentity> {
        filesystem::file_identity(&self.file)
    }

    pub fn stamp(&self) -> io::Result<FileStamp> {
        filesystem::file_stamp(&self.file)
    }

    pub fn hard_link_count(&self) -> io::Result<u64> {
        filesystem::hard_link_count(&self.file)
    }

    pub fn verify_owner_only(&self) -> io::Result<()> {
        filesystem::verify_owner_only_acl(&self.file)
    }

    pub fn try_clone(&self) -> io::Result<Self> {
        self.file.try_clone().map(|file| Self { file })
    }
}

impl Read for ReadFile {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.file.read(bytes)
    }
}

impl Read for &ReadFile {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        (&self.file).read(bytes)
    }
}

impl Seek for ReadFile {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.file.seek(position)
    }
}

impl Seek for &ReadFile {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        (&self.file).seek(position)
    }
}

impl FileReadPort for ReadFile {
    fn metadata(&self) -> io::Result<FileMetadata> {
        self.metadata()
            .map(|metadata| FileMetadata::from_observation(metadata.len(), metadata.is_file()))
    }
    fn identity(&self) -> io::Result<FileIdentity> {
        self.identity()
    }
    fn stamp(&self) -> io::Result<FileStamp> {
        self.stamp()
    }
    fn hard_link_count(&self) -> io::Result<u64> {
        self.hard_link_count()
    }
    fn try_clone_reader(&self) -> io::Result<ReadHandle> {
        self.try_clone().map(ReadHandle::from_port)
    }
    fn read_shared(&self, bytes: &mut [u8]) -> io::Result<usize> {
        (&self.file).read(bytes)
    }
    fn seek_shared(&self, position: SeekFrom) -> io::Result<u64> {
        (&self.file).seek(position)
    }
}

/// Trusted composition can issue a reader; neutral format contracts cannot
/// open an ambient path or turn this reader into a state/source writer.
pub fn open_source_directory(path: &Path) -> io::Result<SourceDirectory> {
    filesystem::RetainedDirectoryCapability::open(path).map(retained_source_directory)
}

pub(crate) fn retained_source_directory(
    retained: filesystem::RetainedDirectoryCapability,
) -> SourceDirectory {
    SourceDirectory::from_port(DirectoryRead { retained })
}

#[derive(Debug)]
struct DirectoryRead {
    retained: filesystem::RetainedDirectoryCapability,
}

impl SourceDirectoryReadPort for DirectoryRead {
    fn path(&self) -> &Path {
        self.retained.path()
    }
    fn identity(&self) -> FileIdentity {
        self.retained.identity()
    }
    fn named_identity_matches(&self) -> io::Result<bool> {
        self.retained.named_identity_matches()
    }
    fn validate_named_identity(&self) -> io::Result<()> {
        self.retained.validate_named_identity()
    }
    fn child_names_equivalent(&self, left: &OsStr, right: &OsStr) -> io::Result<bool> {
        self.retained.child_names_equivalent(left, right)
    }
    fn child_name_comparator(&self) -> io::Result<SourceNameComparator> {
        self.retained
            .child_name_comparator()
            .map(|retained| SourceNameComparator::from_port(NameComparison { retained }))
    }
    fn retain_directory_child(&self, name: &OsStr) -> io::Result<SourceDirectory> {
        self.retained
            .retain_directory_child(name)
            .map(retained_source_directory)
    }
    fn read_relative_regular_bounded(
        &self,
        relative: &Path,
        maximum_bytes: usize,
    ) -> io::Result<Vec<u8>> {
        self.retained
            .read_relative_regular_bounded(relative, maximum_bytes)
    }
    fn visit_relative_regular_chunks(
        &self,
        relative: &Path,
        checkpoint: &mut dyn FnMut() -> io::Result<()>,
        visit: &mut dyn FnMut(&[u8]) -> io::Result<()>,
    ) -> io::Result<u64> {
        self.retained
            .visit_relative_regular_chunks(relative, checkpoint, visit)
    }
    fn read_relative_regular_prefix(
        &self,
        relative: &Path,
        maximum_bytes: usize,
    ) -> io::Result<Vec<u8>> {
        self.retained
            .read_relative_regular_prefix(relative, maximum_bytes)
    }
    fn relative_file_stamp(&self, relative: &Path) -> io::Result<Option<FileStamp>> {
        self.retained.relative_file_stamp(relative)
    }
    fn open_relative_regular_nofollow(&self, relative: &Path) -> io::Result<ReadHandle> {
        self.retained
            .open_relative_regular_nofollow(relative)
            .map(|file| ReadHandle::from_port(ReadFile { file }))
    }
    fn read_immediate_names_with_limit(
        &self,
        maximum_entries: Option<usize>,
        checkpoint: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<Vec<OsString>> {
        self.retained
            .read_immediate_names_with_limit(maximum_entries, checkpoint)
    }
    fn visit_immediate_names(
        &self,
        visitor: &mut dyn FnMut(OsString) -> io::Result<()>,
    ) -> io::Result<()> {
        self.retained.visit_immediate_names(visitor)
    }
    fn retain_immediate_child_nofollow(&self, name: &OsStr) -> io::Result<SourceEntry> {
        self.retained
            .retain_immediate_child_nofollow(name)
            .map(|entry| match entry {
                filesystem::RetainedChildCapability::Directory(retained) => {
                    SourceEntry::Directory(retained_source_directory(retained))
                }
                filesystem::RetainedChildCapability::RegularFile(retained) => {
                    SourceEntry::RegularFile(SourceFile::from_port(FileRead { retained }))
                }
                filesystem::RetainedChildCapability::ReparsePoint => SourceEntry::ReparsePoint,
                filesystem::RetainedChildCapability::Unsupported => SourceEntry::Unsupported,
            })
    }
    fn retain_regular_child(&self, name: &OsStr) -> io::Result<SourceFile> {
        self.retained
            .retain_regular_child(name)
            .map(|retained| SourceFile::from_port(FileRead { retained }))
    }
}

struct NameComparison {
    retained: filesystem::RetainedChildNameComparator,
}
impl SourceNameComparison for NameComparison {
    fn names_equivalent(&self, left: &OsStr, right: &OsStr) -> io::Result<bool> {
        self.retained.names_equivalent(left, right)
    }
}

#[derive(Debug)]
struct FileRead {
    retained: filesystem::RetainedRegularFileCapability,
}
impl SourceFileReadPort for FileRead {
    fn identity(&self) -> FileIdentity {
        self.retained.identity()
    }
    fn read_bounded(&self, maximum_bytes: usize) -> io::Result<Vec<u8>> {
        self.retained.read_bounded(maximum_bytes)
    }
    fn try_clone_file(&self) -> io::Result<ReadHandle> {
        self.retained
            .try_clone_file()
            .map(|file| ReadHandle::from_port(ReadFile { file }))
    }
    fn hard_link_count(&self) -> io::Result<u64> {
        self.retained.hard_link_count()
    }
    fn validate_named_identity(&self) -> io::Result<()> {
        self.retained.validate_named_identity()
    }
    fn open_named_identity_for_read(&self) -> io::Result<ReadHandle> {
        self.retained
            .open_named_identity_for_read()
            .map(|file| ReadHandle::from_port(ReadFile { file }))
    }
    fn validate_named_identity_relative(&self) -> io::Result<()> {
        self.retained.validate_named_identity_relative()
    }
}

/// A trusted composition probe over the same descriptor-relative reader used by
/// source-map health discovery. No writable descriptor leaves the owner.
#[derive(Debug)]
pub struct DirectoryReader {
    directory: std::fs::File,
}

pub enum DirectoryCandidateError {
    Inspect(io::Error),
    Open(io::Error),
}

pub fn open_directory_reader(path: &Path) -> io::Result<DirectoryReader> {
    filesystem::open_absolute_directory_path_nofollow(path)
        .map(|directory| DirectoryReader { directory })
}

impl DirectoryReader {
    pub fn try_clone(&self) -> io::Result<Self> {
        self.directory
            .try_clone()
            .map(|directory| Self { directory })
    }

    pub fn open_regular_child(&self, name: &OsStr) -> io::Result<ReadFile> {
        filesystem::open_regular_child_nofollow(&self.directory, name).map(|file| ReadFile { file })
    }

    pub fn open_directory_child(&self, name: &OsStr) -> io::Result<Self> {
        filesystem::open_directory_child_nofollow(&self.directory, name)
            .map(|directory| Self { directory })
    }

    pub fn read_names_with_limit(
        &self,
        maximum_entries: Option<usize>,
        checkpoint: impl FnMut() -> io::Result<()>,
    ) -> io::Result<Vec<OsString>> {
        filesystem::read_directory_names_with_limit(&self.directory, maximum_entries, checkpoint)
    }

    pub fn open_directory_candidate(
        &self,
        name: &OsStr,
    ) -> Result<Option<Self>, DirectoryCandidateError> {
        let (anchor, kind) = filesystem::open_any_child_nofollow(&self.directory, name)
            .map_err(DirectoryCandidateError::Inspect)?;
        if kind != filesystem::OpenedChildKind::Directory {
            return Ok(None);
        }
        filesystem::open_child_for_secure_tree_use(&self.directory, name, anchor, kind)
            .map(|directory| Some(Self { directory }))
            .map_err(DirectoryCandidateError::Open)
    }
}

// Preserve the two-stage Windows lookup: classification keeps its anchor until
// the typed reopen proves the same identity. Neither result is a writer.
pub use crate::platform::filesystem::OpenedChildKind;

pub fn open_read_directory_nofollow(path: &Path) -> io::Result<ReadFile> {
    filesystem::open_directory_nofollow(path).map(|file| ReadFile { file })
}
pub fn open_absolute_read_directory_nofollow(path: &Path) -> io::Result<ReadFile> {
    filesystem::open_absolute_directory_path_nofollow(path).map(|file| ReadFile { file })
}
pub fn open_any_read_child_nofollow(
    parent: &ReadFile,
    name: &OsStr,
) -> io::Result<(ReadFile, OpenedChildKind)> {
    filesystem::open_any_child_nofollow(&parent.file, name)
        .map(|(file, kind)| (ReadFile { file }, kind))
}
pub fn open_read_child_for_secure_tree_use(
    parent: &ReadFile,
    name: &OsStr,
    classification_anchor: ReadFile,
    kind: OpenedChildKind,
) -> io::Result<ReadFile> {
    filesystem::open_child_for_secure_tree_use(&parent.file, name, classification_anchor.file, kind)
        .map(|file| ReadFile { file })
}
impl ReadFile {
    pub fn change_time(&self, metadata: &Metadata) -> io::Result<(i64, i64)> {
        filesystem::file_change_time(&self.file, metadata)
    }
}

/// Read a regular child through the same retained directory observation.
pub fn open_read_regular_child_nofollow(parent: &ReadFile, name: &OsStr) -> io::Result<ReadFile> {
    filesystem::open_regular_child_nofollow(&parent.file, name).map(|file| ReadFile { file })
}
