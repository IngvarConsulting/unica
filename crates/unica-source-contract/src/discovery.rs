//! Read observations consumed by the existing project source-map grammar.
use crate::read::ReadFile;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy)]
pub enum FollowMode {
    Follow,
    NoFollow,
}

#[derive(Clone, Copy)]
pub struct DiscoveryFileType {
    file: bool,
    directory: bool,
    symlink: bool,
}
impl DiscoveryFileType {
    pub fn from_observation(file: bool, directory: bool, symlink: bool) -> Self {
        Self {
            file,
            directory,
            symlink,
        }
    }
    pub fn is_file(self) -> bool {
        self.file
    }
    pub fn is_dir(self) -> bool {
        self.directory
    }
    pub fn is_symlink(self) -> bool {
        self.symlink
    }
}
#[derive(Clone, Copy)]
pub struct DiscoveryMetadata {
    length: u64,
    kind: DiscoveryFileType,
    link_or_reparse: bool,
}
impl DiscoveryMetadata {
    pub fn from_observation(length: u64, kind: DiscoveryFileType, link_or_reparse: bool) -> Self {
        Self {
            length,
            kind,
            link_or_reparse,
        }
    }
    pub fn len(self) -> u64 {
        self.length
    }
    pub fn is_empty(self) -> bool {
        self.length == 0
    }
    pub fn is_file(self) -> bool {
        self.kind.is_file()
    }
    pub fn is_dir(self) -> bool {
        self.kind.is_dir()
    }
    pub fn file_type(self) -> DiscoveryFileType {
        self.kind
    }
    pub fn is_link_or_reparse(self) -> bool {
        self.link_or_reparse
    }
}

/// `file_type` remains a separate, delayed observation of the actual entry.
pub trait DiscoveryEntryReadPort {
    fn file_name(&self) -> OsString;
    fn path(&self) -> PathBuf;
    fn file_type(&self) -> io::Result<DiscoveryFileType>;
}
pub struct DiscoveryEntry(Box<dyn DiscoveryEntryReadPort>);
impl DiscoveryEntry {
    pub fn from_port(port: impl DiscoveryEntryReadPort + 'static) -> Self {
        Self(Box::new(port))
    }
    pub fn file_name(&self) -> OsString {
        self.0.file_name()
    }
    pub fn path(&self) -> PathBuf {
        self.0.path()
    }
    pub fn file_type(&self) -> io::Result<DiscoveryFileType> {
        self.0.file_type()
    }
}
pub type DiscoveryEntries = Box<dyn Iterator<Item = io::Result<DiscoveryEntry>>>;

pub enum DirectoryCandidateError {
    Inspect(io::Error),
    Open(io::Error),
}
pub trait DiscoveryDirectoryReadPort {
    fn try_clone(&self) -> io::Result<DiscoveryDirectory>;
    fn open_regular_child(&self, name: &OsStr) -> io::Result<ReadFile>;
    fn open_directory_child(&self, name: &OsStr) -> io::Result<DiscoveryDirectory>;
    fn read_names_with_limit(
        &self,
        limit: Option<usize>,
        checkpoint: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<Vec<OsString>>;
    fn open_directory_candidate(
        &self,
        name: &OsStr,
    ) -> Result<Option<DiscoveryDirectory>, DirectoryCandidateError>;
}
pub struct DiscoveryDirectory(Box<dyn DiscoveryDirectoryReadPort>);
impl DiscoveryDirectory {
    pub fn from_port(port: impl DiscoveryDirectoryReadPort + 'static) -> Self {
        Self(Box::new(port))
    }
    pub fn try_clone(&self) -> io::Result<Self> {
        self.0.try_clone()
    }
    pub fn open_regular_child(&self, name: &OsStr) -> io::Result<ReadFile> {
        self.0.open_regular_child(name)
    }
    pub fn open_directory_child(&self, name: &OsStr) -> io::Result<Self> {
        self.0.open_directory_child(name)
    }
    pub fn read_names_with_limit(
        &self,
        limit: Option<usize>,
        mut checkpoint: impl FnMut() -> io::Result<()>,
    ) -> io::Result<Vec<OsString>> {
        self.0.read_names_with_limit(limit, &mut checkpoint)
    }
    pub fn open_directory_candidate(
        &self,
        name: &OsStr,
    ) -> Result<Option<Self>, DirectoryCandidateError> {
        self.0.open_directory_candidate(name)
    }
}
#[derive(Debug, Clone)]
pub struct DeclaredSourceRootRoute {
    pub lexical_path: PathBuf,
    pub identity_path: PathBuf,
}

/// A trusted composition issues this session for one existing discovery call.
/// The contract has no ambient reader constructor, raw handle or write method.
pub trait ProjectDiscoveryRead {
    fn workspace_root(&self) -> &Path;
    fn host_text_fn(&self) -> fn(String) -> String;
    fn inspect_input(&self, route: &Path, follow: FollowMode) -> io::Result<DiscoveryMetadata>;
    fn open_input(&self, route: &Path) -> io::Result<ReadFile>;
    fn read_entries(&self, route: &Path) -> io::Result<DiscoveryEntries>;
    fn open_nofollow_directory(&self, route: &Path) -> io::Result<DiscoveryDirectory>;
    fn normalize_identity(&self, route: &Path) -> Result<PathBuf, String>;
    fn canonicalize(&self, route: &Path) -> io::Result<PathBuf>;
    fn normalize_contained_root(&self, configured: &Path) -> Result<PathBuf, String>;
    fn inspect_declared_route(&self, configured: &str) -> Result<DeclaredSourceRootRoute, String>;
    fn is_link_loop_error(&self, error: &io::Error) -> bool;
    fn path_is_file(&self, route: &Path) -> bool {
        self.inspect_input(route, FollowMode::Follow)
            .map(|metadata| metadata.is_file())
            .unwrap_or(false)
    }
}

/// Records read dependencies only. It cannot issue admission or publish.
pub trait ProjectDiscoveryProvenance {
    fn record_exact_file(&mut self, path: PathBuf, raw: &[u8]) -> Result<(), String>;
    fn record_absence(&mut self, path: PathBuf) -> Result<(), String>;
    fn record_directories(&mut self, path: PathBuf, names: Option<&[String]>)
        -> Result<(), String>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NamedSourceSetErrorKind {
    NotFound,
    Ambiguous,
    Containment,
    Discovery,
}

#[derive(Debug)]
pub struct NamedSourceSetError {
    pub kind: NamedSourceSetErrorKind,
    detail: String,
}

impl NamedSourceSetError {
    pub fn new(kind: NamedSourceSetErrorKind, detail: impl Into<String>) -> Self {
        Self {
            kind,
            detail: detail.into(),
        }
    }
}

impl fmt::Display for NamedSourceSetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.detail)
    }
}

impl std::error::Error for NamedSourceSetError {}
