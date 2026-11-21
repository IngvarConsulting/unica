//! Reads the child resource observation of a metadata object makes. `check`
//! serves them from project discovery, `view` from the source root it holds.
use std::ffi::OsString;
use std::io;
use std::path::Path;

/// What one entry of a listed directory is, without following links.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypedChildEntryKind {
    File,
    Directory,
    Link,
    Other,
}

/// Whole files, existence and the immediate entries of a directory.
pub trait TypedChildResourceRead {
    fn read_file(&self, path: &Path) -> io::Result<Vec<u8>>;
    fn exists(&self, path: &Path) -> bool;
    fn entries(&self, path: &Path) -> io::Result<Vec<(OsString, TypedChildEntryKind)>>;
}
