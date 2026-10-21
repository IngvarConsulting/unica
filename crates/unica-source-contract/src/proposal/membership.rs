//! Observed direct directory membership; capture and validation belong to the owner.
use std::ffi::OsString;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectoryMembershipSelector {
    /// Direct regular files whose extension is exactly lowercase `xml`.
    XmlFiles,
    /// Direct regular files whose extension is ASCII-case-insensitively `cf`.
    CfFilesAsciiCaseInsensitive,
    /// Every direct regular file or directory. This binds recursive scanner
    /// topology one directory at a time without following links.
    AllDirectEntries,
    /// Direct children that are real directories. A container whose *listing*
    /// decided a result — source-set autodetection derives names from the direct
    /// child directories of an extension container — binds exactly that: files,
    /// links and other entry types beside them are not members, so their churn
    /// is not a conflict and their presence does not make the guard unusable.
    DirectChildDirectories,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DirectoryTopologyEntryKind {
    File,
    Directory,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct DirectoryTopologyEntry {
    pub name: OsString,
    pub kind: DirectoryTopologyEntryKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DirectoryMembershipSnapshot {
    /// The path did not exist when the source evidence was captured.
    Absent,
    /// The path was a real directory with this exact filtered direct topology.
    Present(Vec<DirectoryTopologyEntry>),
}
