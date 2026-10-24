//! Physical read observations and retained read dependencies for source discovery.
use crate::compile_transaction::CompileTransaction;
use crate::paths::{normalize_lexically, normalize_path_identity};
use crate::platform::filesystem;
use crate::read::{DirectoryReader, ReadFile};
use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    fs, io,
    path::{Path, PathBuf},
};
use unica_source_contract::discovery::*;
use unica_source_contract::proposal::{
    DirectoryMembershipSelector, DirectoryMembershipSnapshot, DirectoryTopologyEntry,
    DirectoryTopologyEntryKind,
};
use unica_source_contract::read::ReadFile as ReadHandle;

/// Issued by trusted composition; no source writer or raw descriptor is exposed.
pub struct ProjectDiscoveryReader {
    workspace_root: PathBuf,
}
impl ProjectDiscoveryReader {
    pub fn new(workspace_root: &Path) -> Self {
        Self {
            workspace_root: workspace_root.to_path_buf(),
        }
    }
}
fn file_type(kind: fs::FileType) -> DiscoveryFileType {
    DiscoveryFileType::from_observation(kind.is_file(), kind.is_dir(), kind.is_symlink())
}
struct Entry(fs::DirEntry);
impl DiscoveryEntryReadPort for Entry {
    fn file_name(&self) -> OsString {
        self.0.file_name()
    }
    fn path(&self) -> PathBuf {
        self.0.path()
    }
    fn file_type(&self) -> io::Result<DiscoveryFileType> {
        self.0.file_type().map(file_type)
    }
}
impl ProjectDiscoveryRead for ProjectDiscoveryReader {
    fn canonicalize(&self, route: &Path) -> io::Result<PathBuf> {
        std::fs::canonicalize(route)
    }

    fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }
    fn host_text_fn(&self) -> fn(String) -> String {
        crate::paths::host_path_text
    }
    fn inspect_input(&self, route: &Path, follow: FollowMode) -> io::Result<DiscoveryMetadata> {
        let metadata = match follow {
            FollowMode::Follow => fs::metadata(route),
            FollowMode::NoFollow => fs::symlink_metadata(route),
        }?;
        Ok(DiscoveryMetadata::from_observation(
            metadata.len(),
            file_type(metadata.file_type()),
            filesystem::metadata_is_link_or_reparse_point(&metadata),
        ))
    }
    fn open_input(&self, route: &Path) -> io::Result<ReadHandle> {
        fs::File::open(route).map(|file| ReadHandle::from_port(ReadFile { file }))
    }
    fn read_entries(&self, route: &Path) -> io::Result<DiscoveryEntries> {
        fs::read_dir(route).map(|entries| {
            Box::new(
                entries.map(|entry| entry.map(|entry| DiscoveryEntry::from_port(Entry(entry)))),
            ) as DiscoveryEntries
        })
    }
    fn open_nofollow_directory(&self, route: &Path) -> io::Result<DiscoveryDirectory> {
        crate::read::open_directory_reader(route).map(DiscoveryDirectory::from_port)
    }
    fn normalize_identity(&self, route: &Path) -> Result<PathBuf, String> {
        normalize_path_identity(route)
    }
    fn normalize_contained_root(&self, configured: &Path) -> Result<PathBuf, String> {
        normalize_contained_source_root(&self.workspace_root, configured)
    }
    fn inspect_declared_route(&self, configured: &str) -> Result<DeclaredSourceRootRoute, String> {
        inspect_declared_source_root_route(&self.workspace_root, configured)
            .map_err(|error| error.to_string())
    }
    fn is_link_loop_error(&self, error: &io::Error) -> bool {
        filesystem::is_link_loop_error(error)
    }
}
impl DiscoveryDirectoryReadPort for DirectoryReader {
    fn try_clone(&self) -> io::Result<DiscoveryDirectory> {
        self.try_clone().map(DiscoveryDirectory::from_port)
    }
    fn open_regular_child(&self, name: &OsStr) -> io::Result<ReadHandle> {
        self.open_regular_child(name).map(ReadHandle::from_port)
    }
    fn open_directory_child(&self, name: &OsStr) -> io::Result<DiscoveryDirectory> {
        self.open_directory_child(name)
            .map(DiscoveryDirectory::from_port)
    }
    fn read_names_with_limit(
        &self,
        limit: Option<usize>,
        checkpoint: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<Vec<OsString>> {
        self.read_names_with_limit(limit, checkpoint)
    }
    fn open_directory_candidate(
        &self,
        name: &OsStr,
    ) -> Result<Option<DiscoveryDirectory>, DirectoryCandidateError> {
        self.open_directory_candidate(name)
            .map(|directory| directory.map(DiscoveryDirectory::from_port))
            .map_err(|error| match error {
                crate::read::DirectoryCandidateError::Inspect(error) => {
                    DirectoryCandidateError::Inspect(error)
                }
                crate::read::DirectoryCandidateError::Open(error) => {
                    DirectoryCandidateError::Open(error)
                }
            })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ProjectSourceMapInput {
    ExactFile(Vec<u8>),
    Absent,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectSourceMapProvenance {
    inputs: BTreeMap<PathBuf, ProjectSourceMapInput>,
    /// Directories whose *listing* discovery consumed. Autodetection derives source-set
    /// names from the filesystem, so which direct children of an extension container are
    /// directories is an input in its own right: a concurrent checkout that adds or
    /// removes `src/cfe/<name>` changes the computed map while every recorded file input
    /// still agrees, and the transaction would otherwise commit against a stale map.
    directories: BTreeMap<PathBuf, DirectoryMembershipSnapshot>,
}

impl ProjectSourceMapProvenance {
    pub fn bind_to(&self, transaction: &mut CompileTransaction) -> Result<(), String> {
        for (path, input) in &self.inputs {
            match input {
                ProjectSourceMapInput::ExactFile(raw) => {
                    if !transaction.protects_path(path)? {
                        transaction.guard_or_verify_exact_preimage(path, raw)?;
                    }
                }
                ProjectSourceMapInput::Absent => {
                    if !transaction.protects_path(path)? {
                        transaction.guard_path_absent(path)?;
                    }
                }
            }
        }
        for (directory, expected) in &self.directories {
            transaction.guard_or_verify_directory_membership(
                directory,
                DirectoryMembershipSelector::DirectChildDirectories,
                expected.clone(),
            )?;
        }
        Ok(())
    }

    fn record_directory_membership(
        &mut self,
        path: PathBuf,
        snapshot: DirectoryMembershipSnapshot,
    ) -> Result<(), String> {
        match self.directories.get(&path) {
            Some(existing) if existing == &snapshot => Ok(()),
            Some(_) => Err(format!(
                "project source-map input changed while resolving: {}",
                path.display()
            )),
            None => {
                self.directories.insert(path, snapshot);
                Ok(())
            }
        }
    }

    fn record_exact_file(&mut self, path: PathBuf, raw: Vec<u8>) -> Result<(), String> {
        match self.inputs.get(&path) {
            Some(ProjectSourceMapInput::ExactFile(existing)) if existing == &raw => Ok(()),
            Some(_) => Err(format!(
                "project source-map input changed while resolving: {}",
                path.display()
            )),
            None => {
                self.inputs
                    .insert(path, ProjectSourceMapInput::ExactFile(raw));
                Ok(())
            }
        }
    }

    fn record_absence(&mut self, path: PathBuf) -> Result<(), String> {
        match self.inputs.get(&path) {
            Some(ProjectSourceMapInput::Absent) => Ok(()),
            Some(_) => Err(format!(
                "project source-map input changed while resolving: {}",
                path.display()
            )),
            None => {
                self.inputs.insert(path, ProjectSourceMapInput::Absent);
                Ok(())
            }
        }
    }
}

impl ProjectDiscoveryProvenance for ProjectSourceMapProvenance {
    fn record_exact_file(&mut self, path: PathBuf, raw: &[u8]) -> Result<(), String> {
        Self::record_exact_file(self, path, raw.to_vec())
    }
    fn record_absence(&mut self, path: PathBuf) -> Result<(), String> {
        Self::record_absence(self, path)
    }
    fn record_directories(
        &mut self,
        path: PathBuf,
        names: Option<&[String]>,
    ) -> Result<(), String> {
        let snapshot = match names {
            None => DirectoryMembershipSnapshot::Absent,
            Some(names) => DirectoryMembershipSnapshot::Present(
                names
                    .iter()
                    .map(|name| DirectoryTopologyEntry {
                        name: name.into(),
                        kind: DirectoryTopologyEntryKind::Directory,
                    })
                    .collect(),
            ),
        };
        self.record_directory_membership(path, snapshot)
    }
}

pub fn inspect_declared_source_root_route(
    workspace_root: &Path,
    configured_path: &str,
) -> Result<DeclaredSourceRootRoute, NamedSourceSetError> {
    let containment =
        |detail: String| NamedSourceSetError::new(NamedSourceSetErrorKind::Containment, detail);
    let lexical_path =
        lexical_contained_source_root(workspace_root, configured_path).map_err(containment)?;
    let identity_path =
        normalize_contained_source_root(workspace_root, configured_path).map_err(containment)?;
    Ok(DeclaredSourceRootRoute {
        lexical_path,
        identity_path,
    })
}
pub fn lexical_contained_source_root(
    workspace_root: &Path,
    configured_path: impl AsRef<Path>,
) -> Result<PathBuf, String> {
    let workspace_root = absolute_lexical(workspace_root)?;
    let configured_path = configured_path.as_ref();
    let candidate = if configured_path.is_absolute() {
        normalize_lexically(configured_path)
    } else {
        normalize_lexically(&workspace_root.join(configured_path))
    };
    // The same host-case and verbatim-prefix policy `WorkspacePathPolicy` uses,
    // so a source root it would accept is not rejected here as uncontained.
    if !crate::paths::path_starts_with_host_root(&candidate, &workspace_root) {
        return Err(format!(
            "configured source root is outside workspace root {}: {}",
            workspace_root.display(),
            candidate.display()
        ));
    }
    Ok(candidate)
}
pub fn reject_linked_source_root_route(
    workspace_root: &Path,
    source_root: &Path,
) -> Result<(), NamedSourceSetError> {
    let containment =
        |detail: String| NamedSourceSetError::new(NamedSourceSetErrorKind::Containment, detail);
    let workspace_root = absolute_lexical(workspace_root).map_err(containment)?;
    let relative = source_root.strip_prefix(&workspace_root).map_err(|_| {
        containment(format!(
            "configured source root is outside workspace root {}",
            workspace_root.display()
        ))
    })?;
    let mut current = workspace_root;
    for component in relative.components() {
        current.push(component.as_os_str());
        let metadata = fs::symlink_metadata(&current).map_err(|error| {
            // A declared source root whose directory does not exist yet is a
            // discovery condition. Reporting it as a containment denial sends
            // the caller looking for a symbolic link that is not there.
            let kind = if error.kind() == std::io::ErrorKind::NotFound {
                NamedSourceSetErrorKind::NotFound
            } else {
                NamedSourceSetErrorKind::Discovery
            };
            NamedSourceSetError::new(
                kind,
                format!(
                    "failed to inspect configured source-root route {}: {error}",
                    current.display()
                ),
            )
        })?;
        if crate::paths::metadata_is_link_or_reparse_point(&metadata) {
            return Err(containment(format!(
                "configured source-root route contains a symbolic link or reparse point: {}",
                current.display()
            )));
        }
    }
    Ok(())
}
fn absolute_lexical(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        Ok(normalize_lexically(path))
    } else {
        std::env::current_dir()
            .map(|cwd| normalize_lexically(&cwd.join(path)))
            .map_err(|error| format!("failed to determine current directory: {error}"))
    }
}
pub fn normalize_contained_source_root(
    workspace_root: &Path,
    configured_path: impl AsRef<Path>,
) -> Result<PathBuf, String> {
    let workspace_root = normalize_path_identity(workspace_root)?;
    let configured_path = configured_path.as_ref();
    let candidate = if configured_path.is_absolute() {
        configured_path.to_path_buf()
    } else {
        workspace_root.join(configured_path)
    };
    let path = normalize_path_identity(&candidate)?;
    ensure_inside_workspace(&path, &workspace_root)?;
    Ok(path)
}
pub fn ensure_inside_workspace(path: &Path, workspace_root: &Path) -> Result<(), String> {
    if path.starts_with(workspace_root) {
        return Ok(());
    }
    Err(format!(
        "sourceDir must be inside workspace root {}: {}",
        workspace_root.display(),
        path.display()
    ))
}

#[cfg(test)]
#[path = "../tests/platform/source_root_contract.rs"]
mod source_root_contract_tests;
