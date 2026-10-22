//! Read-only full-dump observations. A publisher binds every entry to its own
//! captured identity, length and digest; validators receive no writer or path opener.
use crate::{SourceFormat, SourceSetKind};
use std::path::Path;

/// Format-owned observations; arbitrary parser and filesystem text stays private.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DumpValidationReason {
    UnsupportedRoot,
    WrongVersion,
    InvalidXml,
    InvalidOwner,
    MissingOwner,
    EmptyDump,
    UnreadableSource,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(tag = "kind", content = "literal", rename_all = "camelCase")]
pub enum DumpVersionRequirement {
    Exact(&'static str),
    Absent,
}

#[derive(Debug)]
pub struct DumpValidationFailure {
    pub reason: DumpValidationReason,
    pub path: Option<std::path::PathBuf>,
    pub root: Option<(String, String)>,
    pub version_requirement: Option<DumpVersionRequirement>,
    detail: Box<str>,
}
impl DumpValidationFailure {
    pub fn new(reason: DumpValidationReason, path: Option<&Path>, detail: String) -> Self {
        Self {
            reason,
            path: path.map(Path::to_path_buf),
            root: None,
            version_requirement: None,
            detail: detail.into_boxed_str(),
        }
    }
    pub fn with_version(mut self, requirement: DumpVersionRequirement) -> Self {
        self.version_requirement = Some(requirement);
        self
    }
    pub fn with_root(mut self, namespace: &str, local_name: &str) -> Self {
        self.root = Some((namespace.to_owned(), local_name.to_owned()));
        self
    }
}
impl std::fmt::Display for DumpValidationFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.detail)
    }
}
impl From<String> for DumpValidationFailure {
    fn from(detail: String) -> Self {
        Self::new(DumpValidationReason::UnreadableSource, None, detail)
    }
}

pub trait DumpTreeReadPort {
    fn display_root(&self) -> &Path;
    fn is_directory(&self) -> bool;
    fn entry_count(&self) -> usize;
    fn entry_path(&self, index: usize) -> Option<&Path>;
    fn entry_is_file(&self, index: usize) -> bool;
    fn read_entry_exact(&self, index: usize, role: &str) -> Result<Vec<u8>, String>;
}

#[derive(Clone, Copy)]
pub struct DumpTreeObservation<'a> {
    reader: &'a dyn DumpTreeReadPort,
}
impl<'a> DumpTreeObservation<'a> {
    pub fn from_read_port(reader: &'a dyn DumpTreeReadPort) -> Self {
        Self { reader }
    }
    pub fn display_root(&self) -> &Path {
        self.reader.display_root()
    }
    pub fn is_directory(&self) -> bool {
        self.reader.is_directory()
    }
    pub fn entries(&self) -> impl Iterator<Item = ObservedDumpEntry<'a>> {
        let reader = self.reader;
        (0..reader.entry_count()).filter_map(move |index| {
            reader.entry_path(index).map(|relative| ObservedDumpEntry {
                reader,
                index,
                relative,
            })
        })
    }
}

pub struct ObservedDumpEntry<'a> {
    reader: &'a dyn DumpTreeReadPort,
    index: usize,
    relative: &'a Path,
}
impl ObservedDumpEntry<'_> {
    pub fn relative_path(&self) -> &Path {
        self.relative
    }
    pub fn display_path(&self) -> std::path::PathBuf {
        self.reader.display_root().join(self.relative)
    }
    pub fn is_file(&self) -> bool {
        self.reader.entry_is_file(self.index)
    }
    pub fn read_exact(&self, role: &str) -> Result<Vec<u8>, String> {
        self.reader.read_entry_exact(self.index, role)
    }
}

/// A semantic verdict is not a publication or process-completion permit.
pub trait DumpSemanticValidator: Send + Sync {
    fn validate_stage(
        &self,
        tree: &DumpTreeObservation<'_>,
        kind: SourceSetKind,
    ) -> Result<(), DumpValidationFailure>;
    fn validate_existing_target(
        &self,
        tree: &DumpTreeObservation<'_>,
        kind: SourceSetKind,
    ) -> Result<(), String>;
}

pub type DumpInventoryClassifier = fn(SourceSetKind, &[&Path]) -> SourceFormat;
