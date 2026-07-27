use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    fs::File,
    io::{self, Read},
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex},
};

use sha2::{Digest, Sha256};
use unica_format_core::ports::{OperationalEvidenceRevision, SemanticArtifactId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ArtifactReadLimit {
    Descriptor,
    SupportEvidence,
}

impl ArtifactReadLimit {
    pub(crate) const fn bytes(self) -> u64 {
        match self {
            Self::Descriptor => 8 * 1024 * 1024,
            Self::SupportEvidence => 16 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum DirectoryPageLimit {
    RootDiscovery,
    MetadataRegistry,
}

impl DirectoryPageLimit {
    pub(crate) const fn entries(self) -> usize {
        match self {
            Self::RootDiscovery => 100_000,
            Self::MetadataRegistry => 250_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DirectoryVisit {
    Ignore,
    Selected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SafeRootError {
    Unauthorized,
    Missing,
    LinkOrReparsePoint,
    NotRegular,
    IdentityChanged,
    LimitExceeded,
    Unreadable,
    #[cfg(not(any(unix, windows)))]
    UnsupportedHost,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileIdentity {
    device: u64,
    file: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArtifactEvidence {
    Missing,
    Directory(FileIdentity),
    File {
        identity: FileIdentity,
        digest: Option<[u8; 32]>,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct BoundArtifact {
    relative: PathBuf,
    identity: Option<FileIdentity>,
    directory: bool,
}

impl BoundArtifact {
    pub(crate) fn is_source_root(&self) -> bool {
        self.relative.as_os_str().is_empty()
    }

    pub(crate) fn relative_key(&self) -> Option<String> {
        relative_key(&self.relative)
    }

    pub(crate) const fn is_directory(&self) -> bool {
        self.directory
    }

    pub(crate) const fn is_missing(&self) -> bool {
        self.identity.is_none()
    }

    pub(crate) fn relative(&self) -> &Path {
        &self.relative
    }

    fn rebased(&self, prefix: &Path) -> Result<Self, SafeRootError> {
        let relative = self
            .relative
            .strip_prefix(prefix)
            .map_err(|_| SafeRootError::Unauthorized)?
            .to_path_buf();
        Ok(Self {
            relative,
            identity: self.identity,
            directory: self.directory,
        })
    }
}

#[derive(Debug)]
pub(crate) struct SafeArtifactRead {
    bytes: Vec<u8>,
    id: SemanticArtifactId,
}

impl SafeArtifactRead {
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub(crate) fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    pub(crate) fn id(&self) -> &SemanticArtifactId {
        &self.id
    }
}

#[derive(Debug)]
pub(crate) struct SafeSourceRoot {
    scope_path: PathBuf,
    directory: File,
    identity: FileIdentity,
    baseline_artifact_evidence: Arc<Mutex<BTreeMap<PathBuf, ArtifactEvidence>>>,
    baseline_directory_evidence: Arc<Mutex<BTreeMap<(PathBuf, DirectoryPageLimit), Vec<OsString>>>>,
    artifact_evidence: Mutex<BTreeMap<PathBuf, ArtifactEvidence>>,
    directory_evidence: Mutex<BTreeMap<(PathBuf, DirectoryPageLimit), Vec<OsString>>>,
    sealed: Mutex<bool>,
}

impl SafeSourceRoot {
    pub(crate) fn capture(
        authorized_root: &Path,
        source_root: &Path,
    ) -> Result<Self, SafeRootError> {
        let authorized_path = absolute_lexical_path(authorized_root)?;
        let requested_scope_path = absolute_lexical_path(source_root)?;
        let authorized = open_directory_nofollow(&authorized_path)?;
        let authorized_identity = file_identity(&authorized)?;
        let authorized_rebound = open_directory_nofollow(&authorized_path)?;
        if file_identity(&authorized_rebound)? != authorized_identity {
            return Err(SafeRootError::IdentityChanged);
        }
        let (scope_path, relative, requested_scope_identity): (
            PathBuf,
            PathBuf,
            Option<FileIdentity>,
        ) = match requested_scope_path.strip_prefix(&authorized_path) {
            Ok(relative) => (
                requested_scope_path.clone(),
                validated_relative(relative)?,
                None,
            ),
            Err(_) => {
                #[cfg(unix)]
                {
                    let canonical_authorized = std::fs::canonicalize(&authorized_path)
                        .map_err(|_| SafeRootError::Unauthorized)?;
                    let canonical_scope = std::fs::canonicalize(&requested_scope_path)
                        .map_err(|_| SafeRootError::Unauthorized)?;
                    let canonical_authorized = absolute_lexical_path(&canonical_authorized)?;
                    let canonical_scope = absolute_lexical_path(&canonical_scope)?;
                    let canonical_anchor = open_directory_nofollow(&canonical_authorized)?;
                    if file_identity(&canonical_anchor)? != authorized_identity {
                        return Err(SafeRootError::IdentityChanged);
                    }
                    let relative = validated_relative(
                        canonical_scope
                            .strip_prefix(&canonical_authorized)
                            .map_err(|_| SafeRootError::Unauthorized)?,
                    )?;
                    let requested_scope = open_directory_nofollow(&requested_scope_path)?;
                    (
                        canonical_scope,
                        relative,
                        Some(file_identity(&requested_scope)?),
                    )
                }
                #[cfg(not(unix))]
                {
                    return Err(SafeRootError::Unauthorized);
                }
            }
        };
        let directory = open_directory_relative_nofollow(&authorized, &relative)?;
        let identity = file_identity(&directory)?;
        if requested_scope_identity.is_some_and(|requested| requested != identity) {
            return Err(SafeRootError::IdentityChanged);
        }
        let rebound = open_directory_relative_nofollow(&authorized_rebound, &relative)?;
        if file_identity(&rebound)? != identity {
            return Err(SafeRootError::IdentityChanged);
        }
        Ok(Self {
            scope_path,
            directory,
            identity,
            baseline_artifact_evidence: Arc::new(Mutex::new(BTreeMap::new())),
            baseline_directory_evidence: Arc::new(Mutex::new(BTreeMap::new())),
            artifact_evidence: Mutex::new(BTreeMap::new()),
            directory_evidence: Mutex::new(BTreeMap::new()),
            sealed: Mutex::new(false),
        })
    }

    pub(crate) fn bind_target(
        &self,
        target: &Path,
        allow_missing: bool,
    ) -> Result<BoundArtifact, SafeRootError> {
        let relative = self.relative_leaf(target)?;
        self.bind_relative(&relative, allow_missing)
    }

    pub(crate) fn bind_relative(
        &self,
        relative: &Path,
        allow_missing: bool,
    ) -> Result<BoundArtifact, SafeRootError> {
        self.verify_root()?;
        let relative = validated_relative(relative)?;
        if relative.as_os_str().is_empty() {
            return Ok(BoundArtifact {
                relative,
                identity: Some(self.identity),
                directory: true,
            });
        }
        match self.open_entry(&relative) {
            Ok(OpenedEntry::File(file)) => Ok(BoundArtifact {
                relative,
                identity: Some(file_identity(&file)?),
                directory: false,
            }),
            Ok(OpenedEntry::Directory(directory)) => Ok(BoundArtifact {
                relative,
                identity: Some(file_identity(&directory)?),
                directory: true,
            }),
            Err(SafeRootError::Missing) if allow_missing => Ok(BoundArtifact {
                relative,
                identity: None,
                directory: false,
            }),
            Err(error) => Err(error),
        }
    }

    pub(crate) fn subroot(&self, relative: &Path) -> Result<Self, SafeRootError> {
        self.verify_root()?;
        let relative = validated_relative(relative)?;
        let directory = open_directory_relative_nofollow(&self.directory, &relative)?;
        let identity = file_identity(&directory)?;
        Ok(Self {
            scope_path: self.scope_path.join(relative),
            directory,
            identity,
            baseline_artifact_evidence: Arc::new(Mutex::new(BTreeMap::new())),
            baseline_directory_evidence: Arc::new(Mutex::new(BTreeMap::new())),
            artifact_evidence: Mutex::new(BTreeMap::new()),
            directory_evidence: Mutex::new(BTreeMap::new()),
            sealed: Mutex::new(false),
        })
    }

    pub(crate) fn fork(&self) -> Result<Self, SafeRootError> {
        self.verify_root()?;
        Ok(Self {
            scope_path: self.scope_path.clone(),
            directory: self
                .directory
                .try_clone()
                .map_err(|_| SafeRootError::Unreadable)?,
            identity: self.identity,
            baseline_artifact_evidence: Arc::clone(&self.baseline_artifact_evidence),
            baseline_directory_evidence: Arc::clone(&self.baseline_directory_evidence),
            artifact_evidence: Mutex::new(BTreeMap::new()),
            directory_evidence: Mutex::new(BTreeMap::new()),
            sealed: Mutex::new(false),
        })
    }

    pub(crate) fn rebase_artifact(
        &self,
        artifact: &BoundArtifact,
        prefix: &Path,
    ) -> Result<BoundArtifact, SafeRootError> {
        artifact.rebased(prefix)
    }

    pub(crate) fn read_bound(
        &self,
        artifact: &BoundArtifact,
        limit: ArtifactReadLimit,
    ) -> Result<SafeArtifactRead, SafeRootError> {
        if artifact.directory || artifact.identity.is_none() {
            return Err(SafeRootError::NotRegular);
        }
        let read = self.read_relative_path(&artifact.relative, limit)?;
        if read.1 != artifact.identity.expect("checked above") {
            return Err(SafeRootError::IdentityChanged);
        }
        Ok(read.0)
    }

    pub(crate) fn read_relative(
        &self,
        relative: &str,
        limit: ArtifactReadLimit,
    ) -> Result<SafeArtifactRead, SafeRootError> {
        let relative = validated_relative(Path::new(relative))?;
        self.read_relative_path(&relative, limit)
            .map(|(read, _)| read)
    }

    pub(crate) fn exists_regular(&self, relative: &str) -> Result<bool, SafeRootError> {
        let relative = validated_relative(Path::new(relative))?;
        match self.open_entry(&relative) {
            Ok(OpenedEntry::File(file)) => {
                self.bind_artifact_evidence(
                    &relative,
                    ArtifactEvidence::File {
                        identity: file_identity(&file)?,
                        digest: None,
                    },
                )?;
                Ok(true)
            }
            Ok(OpenedEntry::Directory(directory)) => {
                self.bind_artifact_evidence(
                    &relative,
                    ArtifactEvidence::Directory(file_identity(&directory)?),
                )?;
                Ok(false)
            }
            Err(SafeRootError::Missing) => {
                self.bind_artifact_evidence(&relative, ArtifactEvidence::Missing)?;
                Ok(false)
            }
            Err(error) => Err(error),
        }
    }

    pub(crate) fn is_directory(&self, relative: &Path) -> Result<bool, SafeRootError> {
        let relative = validated_relative(relative)?;
        match self.open_entry(&relative) {
            Ok(OpenedEntry::Directory(directory)) => {
                self.bind_artifact_evidence(
                    &relative,
                    ArtifactEvidence::Directory(file_identity(&directory)?),
                )?;
                Ok(true)
            }
            Ok(OpenedEntry::File(file)) => {
                self.bind_artifact_evidence(
                    &relative,
                    ArtifactEvidence::File {
                        identity: file_identity(&file)?,
                        digest: None,
                    },
                )?;
                Ok(false)
            }
            Err(SafeRootError::NotRegular) => Ok(false),
            Err(error) => Err(error),
        }
    }

    pub(crate) fn visit_directory(
        &self,
        relative: &str,
        limit: DirectoryPageLimit,
        mut visitor: impl FnMut(&OsStr) -> Result<DirectoryVisit, SafeRootError>,
    ) -> Result<(), SafeRootError> {
        self.ensure_unsealed()?;
        self.verify_root()?;
        let relative = validated_relative(Path::new(relative))?;
        let directory = open_directory_relative_nofollow(&self.directory, &relative)?;
        let mut selected = 0usize;
        let mut selected_names = Vec::new();
        visit_directory_names(&directory, &mut |name| {
            if visitor(name)? == DirectoryVisit::Selected {
                selected = selected
                    .checked_add(1)
                    .ok_or(SafeRootError::LimitExceeded)?;
                if selected > limit.entries() {
                    return Err(SafeRootError::LimitExceeded);
                }
                selected_names.push(name.to_os_string());
            }
            Ok(())
        })?;
        selected_names.sort();
        let key = (relative, limit);
        bind_directory_evidence(&self.baseline_directory_evidence, &key, &selected_names)?;
        bind_directory_evidence(&self.directory_evidence, &key, &selected_names)
    }

    fn read_relative_path(
        &self,
        relative: &Path,
        limit: ArtifactReadLimit,
    ) -> Result<(SafeArtifactRead, FileIdentity), SafeRootError> {
        self.verify_root()?;
        let mut file = match self.open_entry(relative) {
            Ok(OpenedEntry::File(file)) => file,
            Ok(OpenedEntry::Directory(directory)) => {
                self.bind_artifact_evidence(
                    relative,
                    ArtifactEvidence::Directory(file_identity(&directory)?),
                )?;
                return Err(SafeRootError::NotRegular);
            }
            Err(SafeRootError::Missing) => {
                self.bind_artifact_evidence(relative, ArtifactEvidence::Missing)?;
                return Err(SafeRootError::Missing);
            }
            Err(error) => return Err(error),
        };
        let identity = file_identity(&file)?;
        let before = file.metadata().map_err(|_| SafeRootError::Unreadable)?;
        run_after_artifact_open(relative);
        let mut bytes = Vec::new();
        file.by_ref()
            .take(limit.bytes() + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| SafeRootError::Unreadable)?;
        if bytes.len() as u64 > limit.bytes() {
            return Err(SafeRootError::LimitExceeded);
        }
        let after = file.metadata().map_err(|_| SafeRootError::Unreadable)?;
        if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
            return Err(SafeRootError::IdentityChanged);
        }
        let rebound = self.open_entry(relative)?;
        let rebound_identity = match rebound {
            OpenedEntry::File(file) => file_identity(&file)?,
            OpenedEntry::Directory(_) => return Err(SafeRootError::IdentityChanged),
        };
        if rebound_identity != identity {
            return Err(SafeRootError::IdentityChanged);
        }
        let digest = Sha256::digest(&bytes);
        let semantic_id = format!("artifact:{digest:x}");
        let digest_bytes: [u8; 32] = digest.into();
        let observed = ArtifactEvidence::File {
            identity,
            digest: Some(digest_bytes),
        };
        self.bind_artifact_evidence(relative, observed)?;
        let id = SemanticArtifactId::new(semantic_id).map_err(|_| SafeRootError::Unreadable)?;
        Ok((SafeArtifactRead { bytes, id }, identity))
    }

    fn relative_leaf(&self, target: &Path) -> Result<PathBuf, SafeRootError> {
        let absolute = absolute_lexical_path(target)?;
        if absolute == self.scope_path {
            return Ok(PathBuf::new());
        }
        let relative = absolute
            .strip_prefix(&self.scope_path)
            .map_err(|_| SafeRootError::Unauthorized)?;
        validated_relative(relative)
    }

    fn open_entry(&self, relative: &Path) -> Result<OpenedEntry, SafeRootError> {
        self.ensure_unsealed()?;
        let (parent, name) = split_relative_leaf(relative)?;
        let parent = open_directory_relative_nofollow(&self.directory, parent)?;
        run_before_artifact_open(relative);
        match open_regular_child_nofollow(&parent, name) {
            Ok(file) => Ok(OpenedEntry::File(file)),
            Err(SafeRootError::NotRegular) => {
                open_directory_child_nofollow(&parent, name).map(OpenedEntry::Directory)
            }
            Err(error) => Err(error),
        }
    }

    fn bind_artifact_evidence(
        &self,
        relative: &Path,
        observed: ArtifactEvidence,
    ) -> Result<(), SafeRootError> {
        bind_artifact_evidence_map(&self.baseline_artifact_evidence, relative, observed)?;
        bind_artifact_evidence_map(&self.artifact_evidence, relative, observed)
    }

    fn verify_root(&self) -> Result<(), SafeRootError> {
        if file_identity(&self.directory)? != self.identity {
            return Err(SafeRootError::IdentityChanged);
        }
        Ok(())
    }

    pub(crate) fn finalize_evidence(
        &self,
        operation: &'static [u8],
    ) -> Result<OperationalEvidenceRevision, SafeRootError> {
        self.verify_root()?;
        let mut sealed = self.sealed.lock().map_err(|_| SafeRootError::Unreadable)?;
        if *sealed {
            return Err(SafeRootError::IdentityChanged);
        }
        let artifacts = self
            .artifact_evidence
            .lock()
            .map_err(|_| SafeRootError::Unreadable)?;
        let directories = self
            .directory_evidence
            .lock()
            .map_err(|_| SafeRootError::Unreadable)?;
        let mut digest = Sha256::new();
        digest.update(b"unica:platform-xml:operational-evidence:v1\0");
        digest.update((operation.len() as u64).to_le_bytes());
        digest.update(operation);
        digest.update(self.identity.device.to_le_bytes());
        digest.update(self.identity.file.to_le_bytes());
        for (relative, evidence) in artifacts.iter() {
            update_relative_path(&mut digest, relative);
            match evidence {
                ArtifactEvidence::Missing => digest.update([0]),
                ArtifactEvidence::Directory(identity) => {
                    digest.update([1]);
                    update_file_identity(&mut digest, *identity);
                }
                ArtifactEvidence::File {
                    identity,
                    digest: bytes_digest,
                } => {
                    digest.update([2]);
                    update_file_identity(&mut digest, *identity);
                    match bytes_digest {
                        Some(bytes_digest) => {
                            digest.update([1]);
                            digest.update(bytes_digest);
                        }
                        None => digest.update([0]),
                    }
                }
            }
        }
        for ((relative, limit), names) in directories.iter() {
            digest.update([3]);
            update_relative_path(&mut digest, relative);
            digest.update(match limit {
                DirectoryPageLimit::RootDiscovery => [0],
                DirectoryPageLimit::MetadataRegistry => [1],
            });
            digest.update((names.len() as u64).to_le_bytes());
            for name in names {
                let name = name.to_string_lossy();
                digest.update((name.len() as u64).to_le_bytes());
                digest.update(name.as_bytes());
            }
        }
        *sealed = true;
        Ok(OperationalEvidenceRevision::from_digest(
            digest.finalize().into(),
        ))
    }

    fn ensure_unsealed(&self) -> Result<(), SafeRootError> {
        if *self.sealed.lock().map_err(|_| SafeRootError::Unreadable)? {
            Err(SafeRootError::IdentityChanged)
        } else {
            Ok(())
        }
    }
}

fn bind_artifact_evidence_map(
    evidence: &Mutex<BTreeMap<PathBuf, ArtifactEvidence>>,
    relative: &Path,
    observed: ArtifactEvidence,
) -> Result<(), SafeRootError> {
    let mut evidence = evidence.lock().map_err(|_| SafeRootError::Unreadable)?;
    match (evidence.get(relative).copied(), observed) {
        (None, observed) => {
            evidence.insert(relative.to_path_buf(), observed);
            Ok(())
        }
        (Some(ArtifactEvidence::Missing), ArtifactEvidence::Missing)
        | (Some(ArtifactEvidence::Directory(_)), ArtifactEvidence::Directory(_))
            if evidence.get(relative).copied() == Some(observed) =>
        {
            Ok(())
        }
        (
            Some(ArtifactEvidence::File {
                identity: expected_identity,
                digest: expected_digest,
            }),
            ArtifactEvidence::File {
                identity: observed_identity,
                digest: observed_digest,
            },
        ) if expected_identity == observed_identity
            && (expected_digest.is_none()
                || observed_digest.is_none()
                || expected_digest == observed_digest) =>
        {
            if expected_digest.is_none() && observed_digest.is_some() {
                evidence.insert(relative.to_path_buf(), observed);
            }
            Ok(())
        }
        _ => Err(SafeRootError::IdentityChanged),
    }
}

fn bind_directory_evidence(
    evidence: &Mutex<BTreeMap<(PathBuf, DirectoryPageLimit), Vec<OsString>>>,
    key: &(PathBuf, DirectoryPageLimit),
    observed: &[OsString],
) -> Result<(), SafeRootError> {
    let mut evidence = evidence.lock().map_err(|_| SafeRootError::Unreadable)?;
    match evidence.get(key) {
        Some(expected) if expected != observed => Err(SafeRootError::IdentityChanged),
        Some(_) => Ok(()),
        None => {
            evidence.insert(key.clone(), observed.to_vec());
            Ok(())
        }
    }
}

fn update_file_identity(digest: &mut Sha256, identity: FileIdentity) {
    digest.update(identity.device.to_le_bytes());
    digest.update(identity.file.to_le_bytes());
}

fn update_relative_path(digest: &mut Sha256, relative: &Path) {
    let relative = relative.to_string_lossy();
    digest.update((relative.len() as u64).to_le_bytes());
    digest.update(relative.as_bytes());
}

enum OpenedEntry {
    File(File),
    Directory(File),
}

fn absolute_lexical_path(path: &Path) -> Result<PathBuf, SafeRootError> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|_| SafeRootError::Unauthorized)?
            .join(path)
    };
    let mut result = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                result.push(component.as_os_str());
            }
            Component::CurDir => {}
            Component::ParentDir => return Err(SafeRootError::Unauthorized),
        }
    }
    if result.is_absolute() {
        Ok(result)
    } else {
        Err(SafeRootError::Unauthorized)
    }
}

fn validated_relative(path: &Path) -> Result<PathBuf, SafeRootError> {
    if path.is_absolute() {
        return Err(SafeRootError::Unauthorized);
    }
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => result.push(value),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(SafeRootError::Unauthorized)
            }
        }
    }
    Ok(result)
}

fn split_relative_leaf(path: &Path) -> Result<(&Path, &OsStr), SafeRootError> {
    let name = path.file_name().ok_or(SafeRootError::NotRegular)?;
    Ok((path.parent().unwrap_or_else(|| Path::new("")), name))
}

fn relative_key(path: &Path) -> Option<String> {
    let mut parts = Vec::new();
    for component in path.components() {
        let Component::Normal(value) = component else {
            return None;
        };
        parts.push(value.to_str()?.to_string());
    }
    Some(parts.join("/"))
}

#[cfg(unix)]
fn open_directory_nofollow(path: &Path) -> Result<File, SafeRootError> {
    use std::{
        ffi::CString,
        os::{unix::ffi::OsStrExt, unix::io::FromRawFd},
    };

    if !path.is_absolute() {
        return Err(SafeRootError::Unauthorized);
    }
    let path =
        CString::new(path.as_os_str().as_bytes()).map_err(|_| SafeRootError::Unauthorized)?;
    // The command-selected root is captured once. All source and artifact
    // traversal after this point is descriptor-relative and no-follow.
    let fd = unsafe {
        libc::open(
            path.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        return Err(map_io(io::Error::last_os_error()));
    }
    // SAFETY: fd was returned as a newly owned descriptor.
    Ok(unsafe { File::from_raw_fd(fd) })
}

#[cfg(unix)]
fn open_directory_relative_nofollow(root: &File, relative: &Path) -> Result<File, SafeRootError> {
    let relative = validated_relative(relative)?;
    let mut current = open_directory_child_nofollow(root, OsStr::new("."))?;
    for component in relative.components() {
        let Component::Normal(name) = component else {
            continue;
        };
        current = open_directory_child_nofollow(&current, name)?;
    }
    Ok(current)
}

#[cfg(unix)]
fn open_directory_child_nofollow(parent: &File, name: &OsStr) -> Result<File, SafeRootError> {
    use std::{
        ffi::CString,
        os::{
            unix::ffi::OsStrExt,
            unix::io::{AsRawFd, FromRawFd},
        },
    };

    let name = CString::new(name.as_bytes()).map_err(|_| SafeRootError::Unauthorized)?;
    // SAFETY: parent and name remain live and ownership of a successful fd is transferred.
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        Err(map_io(io::Error::last_os_error()))
    } else {
        // SAFETY: fd was returned as a newly owned descriptor.
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}

#[cfg(unix)]
fn open_regular_child_nofollow(parent: &File, name: &OsStr) -> Result<File, SafeRootError> {
    use std::{
        ffi::CString,
        os::{
            unix::ffi::OsStrExt,
            unix::io::{AsRawFd, FromRawFd},
        },
    };

    let name = CString::new(name.as_bytes()).map_err(|_| SafeRootError::Unauthorized)?;
    // SAFETY: parent and name remain live and ownership of a successful fd is transferred.
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
        )
    };
    if fd < 0 {
        return Err(map_io(io::Error::last_os_error()));
    }
    // SAFETY: fd was returned as a newly owned descriptor.
    let file = unsafe { File::from_raw_fd(fd) };
    if !file.metadata().map_err(map_io)?.file_type().is_file() {
        return Err(SafeRootError::NotRegular);
    }
    Ok(file)
}

#[cfg(unix)]
fn visit_directory_names(
    directory: &File,
    visitor: &mut dyn FnMut(&OsStr) -> Result<(), SafeRootError>,
) -> Result<(), SafeRootError> {
    use std::{
        ffi::CStr,
        os::{unix::ffi::OsStringExt, unix::io::AsRawFd},
    };

    // SAFETY: fcntl duplicates the live descriptor; fdopendir owns the duplicate on success.
    let duplicate = unsafe { libc::fcntl(directory.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 0) };
    if duplicate < 0 {
        return Err(map_io(io::Error::last_os_error()));
    }
    // SAFETY: duplicate is a valid owned directory descriptor.
    let stream = unsafe { libc::fdopendir(duplicate) };
    if stream.is_null() {
        let error = map_io(io::Error::last_os_error());
        // SAFETY: fdopendir failed and did not consume duplicate.
        unsafe { libc::close(duplicate) };
        return Err(error);
    }
    loop {
        // SAFETY: stream remains live until closed below.
        let entry = unsafe { libc::readdir(stream) };
        if entry.is_null() {
            break;
        }
        // SAFETY: d_name is NUL-terminated and owned by the live dirent.
        let bytes = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
        if bytes != b"." && bytes != b".." {
            let name = OsString::from_vec(bytes.to_vec());
            visitor(&name)?;
        }
    }
    // SAFETY: stream is live and owns duplicate.
    if unsafe { libc::closedir(stream) } != 0 {
        return Err(map_io(io::Error::last_os_error()));
    }
    Ok(())
}

#[cfg(unix)]
fn file_identity(file: &File) -> Result<FileIdentity, SafeRootError> {
    use std::os::unix::fs::MetadataExt;
    let metadata = file.metadata().map_err(map_io)?;
    Ok(FileIdentity {
        device: metadata.dev(),
        file: metadata.ino(),
    })
}

#[cfg(windows)]
fn open_directory_nofollow(path: &Path) -> Result<File, SafeRootError> {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0x0000_0001 | 0x0000_0002 | 0x0000_0004)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(map_io)?;
    reject_windows_reparse(&file)?;
    Ok(file)
}

#[cfg(windows)]
fn open_directory_relative_nofollow(root: &File, relative: &Path) -> Result<File, SafeRootError> {
    let relative = validated_relative(relative)?;
    let mut current = open_directory_child_nofollow(root, OsStr::new("."))?;
    for component in relative.components() {
        let Component::Normal(name) = component else {
            continue;
        };
        current = open_directory_child_nofollow(&current, name)?;
    }
    Ok(current)
}

#[cfg(windows)]
fn open_directory_child_nofollow(parent: &File, name: &OsStr) -> Result<File, SafeRootError> {
    let file = nt_open_relative(parent, name, true)?;
    reject_windows_reparse(&file)?;
    if !file.metadata().map_err(map_io)?.is_dir() {
        return Err(SafeRootError::NotRegular);
    }
    Ok(file)
}

#[cfg(windows)]
fn open_regular_child_nofollow(parent: &File, name: &OsStr) -> Result<File, SafeRootError> {
    let file = nt_open_relative(parent, name, false)?;
    reject_windows_reparse(&file)?;
    if !file.metadata().map_err(map_io)?.is_file() {
        return Err(SafeRootError::NotRegular);
    }
    Ok(file)
}

#[cfg(windows)]
fn nt_open_relative(parent: &File, name: &OsStr, directory: bool) -> Result<File, SafeRootError> {
    use std::{
        ffi::c_void,
        os::windows::{
            ffi::OsStrExt,
            io::{AsRawHandle, FromRawHandle},
        },
        ptr,
    };

    #[repr(C)]
    struct UnicodeString {
        length: u16,
        maximum_length: u16,
        buffer: *mut u16,
    }
    #[repr(C)]
    struct ObjectAttributes {
        length: u32,
        root_directory: *mut c_void,
        object_name: *mut UnicodeString,
        attributes: u32,
        security_descriptor: *mut c_void,
        security_quality_of_service: *mut c_void,
    }
    #[repr(C)]
    struct IoStatusBlock {
        status: isize,
        information: usize,
    }

    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn NtCreateFile(
            file_handle: *mut *mut c_void,
            desired_access: u32,
            object_attributes: *mut ObjectAttributes,
            io_status_block: *mut IoStatusBlock,
            allocation_size: *mut i64,
            file_attributes: u32,
            share_access: u32,
            create_disposition: u32,
            create_options: u32,
            ea_buffer: *mut c_void,
            ea_length: u32,
        ) -> i32;
    }

    const OBJ_CASE_INSENSITIVE: u32 = 0x0000_0040;
    const FILE_LIST_DIRECTORY: u32 = 0x0000_0001;
    const FILE_READ_DATA: u32 = 0x0000_0001;
    const FILE_READ_ATTRIBUTES: u32 = 0x0000_0080;
    const SYNCHRONIZE: u32 = 0x0010_0000;
    const FILE_SHARE_ALL: u32 = 0x0000_0001 | 0x0000_0002 | 0x0000_0004;
    const FILE_OPEN: u32 = 0x0000_0001;
    const FILE_DIRECTORY_FILE: u32 = 0x0000_0001;
    const FILE_SYNCHRONOUS_IO_NONALERT: u32 = 0x0000_0020;
    const FILE_NON_DIRECTORY_FILE: u32 = 0x0000_0040;
    const FILE_OPEN_REPARSE_POINT: u32 = 0x0020_0000;

    let mut encoded = name.encode_wide().collect::<Vec<_>>();
    let byte_len = encoded
        .len()
        .checked_mul(2)
        .and_then(|length| u16::try_from(length).ok())
        .ok_or(SafeRootError::Unauthorized)?;
    let mut object_name = UnicodeString {
        length: byte_len,
        maximum_length: byte_len,
        buffer: encoded.as_mut_ptr(),
    };
    let mut attributes = ObjectAttributes {
        length: std::mem::size_of::<ObjectAttributes>() as u32,
        root_directory: parent.as_raw_handle().cast(),
        object_name: &mut object_name,
        attributes: OBJ_CASE_INSENSITIVE,
        security_descriptor: ptr::null_mut(),
        security_quality_of_service: ptr::null_mut(),
    };
    let mut io_status = IoStatusBlock {
        status: 0,
        information: 0,
    };
    let mut handle = ptr::null_mut();
    let status = unsafe {
        NtCreateFile(
            &mut handle,
            (if directory {
                FILE_LIST_DIRECTORY
            } else {
                FILE_READ_DATA
            }) | FILE_READ_ATTRIBUTES
                | SYNCHRONIZE,
            &mut attributes,
            &mut io_status,
            ptr::null_mut(),
            0,
            FILE_SHARE_ALL,
            FILE_OPEN,
            (if directory {
                FILE_DIRECTORY_FILE
            } else {
                FILE_NON_DIRECTORY_FILE
            }) | FILE_SYNCHRONOUS_IO_NONALERT
                | FILE_OPEN_REPARSE_POINT,
            ptr::null_mut(),
            0,
        )
    };
    if status < 0 || handle.is_null() {
        return Err(map_nt_status(status));
    }
    Ok(unsafe { File::from_raw_handle(handle.cast()) })
}

#[cfg(windows)]
fn visit_directory_names(
    directory: &File,
    visitor: &mut dyn FnMut(&OsStr) -> Result<(), SafeRootError>,
) -> Result<(), SafeRootError> {
    use std::{
        ffi::c_void,
        os::windows::{ffi::OsStringExt, io::AsRawHandle},
        ptr,
    };

    #[repr(C)]
    struct IoStatusBlock {
        status: isize,
        information: usize,
    }
    #[repr(C)]
    struct FileNamesInformation {
        next_entry_offset: u32,
        file_index: u32,
        file_name_length: u32,
        file_name: [u16; 1],
    }

    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn NtQueryDirectoryFile(
            file_handle: *mut c_void,
            event: *mut c_void,
            apc_routine: *mut c_void,
            apc_context: *mut c_void,
            io_status_block: *mut IoStatusBlock,
            file_information: *mut c_void,
            length: u32,
            file_information_class: u32,
            return_single_entry: u8,
            file_name: *mut c_void,
            restart_scan: u8,
        ) -> i32;
    }

    const FILE_NAMES_INFORMATION_CLASS: u32 = 12;
    const STATUS_NO_MORE_FILES: i32 = 0x8000_0006u32 as i32;
    let mut buffer = vec![0u8; 64 * 1024];
    let mut restart = 1u8;
    loop {
        let mut io_status = IoStatusBlock {
            status: 0,
            information: 0,
        };
        let status = unsafe {
            NtQueryDirectoryFile(
                directory.as_raw_handle().cast(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                &mut io_status,
                buffer.as_mut_ptr().cast(),
                buffer.len() as u32,
                FILE_NAMES_INFORMATION_CLASS,
                0,
                ptr::null_mut(),
                restart,
            )
        };
        restart = 0;
        if status == STATUS_NO_MORE_FILES {
            return Ok(());
        }
        if status < 0 {
            return Err(map_nt_status(status));
        }
        let mut offset = 0usize;
        while offset < io_status.information {
            let entry = unsafe { &*(buffer.as_ptr().add(offset).cast::<FileNamesInformation>()) };
            let length = entry.file_name_length as usize / 2;
            let name = unsafe { std::slice::from_raw_parts(entry.file_name.as_ptr(), length) };
            let name = OsString::from_wide(name);
            if name != "." && name != ".." {
                visitor(&name)?;
            }
            if entry.next_entry_offset == 0 {
                break;
            }
            offset = offset
                .checked_add(entry.next_entry_offset as usize)
                .ok_or(SafeRootError::Unreadable)?;
        }
    }
}

#[cfg(windows)]
fn file_identity(file: &File) -> Result<FileIdentity, SafeRootError> {
    let identity = crate::platform_handle::query(file).map_err(map_io)?;
    Ok(FileIdentity {
        device: identity.volume,
        file: identity.file,
    })
}

#[cfg(windows)]
fn reject_windows_reparse(file: &File) -> Result<(), SafeRootError> {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    if file.metadata().map_err(map_io)?.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(SafeRootError::LinkOrReparsePoint);
    }
    Ok(())
}

#[cfg(windows)]
fn map_nt_status(status: i32) -> SafeRootError {
    const STATUS_OBJECT_NAME_NOT_FOUND: i32 = 0xC000_0034u32 as i32;
    const STATUS_OBJECT_PATH_NOT_FOUND: i32 = 0xC000_003Au32 as i32;
    const STATUS_REPARSE_POINT_ENCOUNTERED: i32 = 0xC000_050Bu32 as i32;
    match status {
        STATUS_OBJECT_NAME_NOT_FOUND | STATUS_OBJECT_PATH_NOT_FOUND => SafeRootError::Missing,
        STATUS_REPARSE_POINT_ENCOUNTERED => SafeRootError::LinkOrReparsePoint,
        _ => SafeRootError::Unreadable,
    }
}

#[cfg(not(any(unix, windows)))]
fn open_directory_nofollow(_path: &Path) -> Result<File, SafeRootError> {
    Err(SafeRootError::UnsupportedHost)
}

#[cfg(not(any(unix, windows)))]
fn open_directory_relative_nofollow(_root: &File, _relative: &Path) -> Result<File, SafeRootError> {
    Err(SafeRootError::UnsupportedHost)
}

#[cfg(not(any(unix, windows)))]
fn open_directory_child_nofollow(_parent: &File, _name: &OsStr) -> Result<File, SafeRootError> {
    Err(SafeRootError::UnsupportedHost)
}

#[cfg(not(any(unix, windows)))]
fn open_regular_child_nofollow(_parent: &File, _name: &OsStr) -> Result<File, SafeRootError> {
    Err(SafeRootError::UnsupportedHost)
}

#[cfg(not(any(unix, windows)))]
fn visit_directory_names(
    _directory: &File,
    _visitor: &mut dyn FnMut(&OsStr) -> Result<(), SafeRootError>,
) -> Result<(), SafeRootError> {
    Err(SafeRootError::UnsupportedHost)
}

#[cfg(not(any(unix, windows)))]
fn file_identity(_file: &File) -> Result<FileIdentity, SafeRootError> {
    Err(SafeRootError::UnsupportedHost)
}

fn map_io(error: io::Error) -> SafeRootError {
    if error.kind() == io::ErrorKind::NotFound {
        SafeRootError::Missing
    } else if error.raw_os_error() == Some(libc_eloop()) {
        SafeRootError::LinkOrReparsePoint
    } else {
        SafeRootError::Unreadable
    }
}

#[cfg(unix)]
const fn libc_eloop() -> i32 {
    libc::ELOOP
}

#[cfg(not(unix))]
const fn libc_eloop() -> i32 {
    -1
}

#[cfg(test)]
thread_local! {
    static BEFORE_ARTIFACT_OPEN: std::cell::RefCell<Option<Box<dyn FnOnce(&Path)>>> =
        std::cell::RefCell::new(None);
    static AFTER_ARTIFACT_OPEN: std::cell::RefCell<Option<Box<dyn FnOnce(&Path)>>> =
        std::cell::RefCell::new(None);
    static ARTIFACT_OPEN_LOG: std::cell::RefCell<Option<Vec<PathBuf>>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn run_before_artifact_open(relative: &Path) {
    ARTIFACT_OPEN_LOG.with(|slot| {
        if let Some(log) = slot.borrow_mut().as_mut() {
            log.push(relative.to_path_buf());
        }
    });
    BEFORE_ARTIFACT_OPEN.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook(relative);
        }
    });
}

#[cfg(test)]
pub(crate) fn with_artifact_open_log<T>(action: impl FnOnce() -> T) -> (T, Vec<PathBuf>) {
    ARTIFACT_OPEN_LOG.with(|slot| {
        assert!(slot.borrow().is_none());
        *slot.borrow_mut() = Some(Vec::new());
    });
    let result = action();
    let log = ARTIFACT_OPEN_LOG.with(|slot| slot.borrow_mut().take().unwrap_or_default());
    (result, log)
}

#[cfg(not(test))]
fn run_before_artifact_open(_relative: &Path) {}

#[cfg(test)]
fn run_after_artifact_open(relative: &Path) {
    AFTER_ARTIFACT_OPEN.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook(relative);
        }
    });
}

#[cfg(not(test))]
fn run_after_artifact_open(_relative: &Path) {}

#[cfg(test)]
pub(crate) fn with_before_artifact_open<T>(
    hook: impl FnOnce(&Path) + 'static,
    action: impl FnOnce() -> T,
) -> T {
    BEFORE_ARTIFACT_OPEN.with(|slot| {
        assert!(slot.borrow().is_none());
        *slot.borrow_mut() = Some(Box::new(hook));
    });
    let result = action();
    BEFORE_ARTIFACT_OPEN.with(|slot| {
        slot.borrow_mut().take();
    });
    result
}

#[cfg(all(test, unix))]
pub(crate) fn with_artifact_open_hooks<T>(
    before: impl FnOnce(&Path) + 'static,
    after: impl FnOnce(&Path) + 'static,
    action: impl FnOnce() -> T,
) -> T {
    BEFORE_ARTIFACT_OPEN.with(|slot| {
        assert!(slot.borrow().is_none());
        *slot.borrow_mut() = Some(Box::new(before));
    });
    AFTER_ARTIFACT_OPEN.with(|slot| {
        assert!(slot.borrow().is_none());
        *slot.borrow_mut() = Some(Box::new(after));
    });
    let result = action();
    BEFORE_ARTIFACT_OPEN.with(|slot| {
        slot.borrow_mut().take();
    });
    AFTER_ARTIFACT_OPEN.with(|slot| {
        slot.borrow_mut().take();
    });
    result
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn authorized_root_alias_and_canonical_source_bind_to_the_same_opened_capability() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let parent = std::env::temp_dir().join(format!(
            "unica-safe-root-alias-{}-{nonce}",
            std::process::id()
        ));
        let real = parent.join("real");
        let workspace = real.join("workspace");
        let source = workspace.join("source");
        std::fs::create_dir_all(&source).unwrap();
        symlink(&real, parent.join("alias")).unwrap();

        let authorized_alias = parent.join("alias/workspace");
        let canonical_source = std::fs::canonicalize(&source).unwrap();
        let root = SafeSourceRoot::capture(&authorized_alias, &canonical_source).unwrap();

        assert_eq!(root.identity, file_identity(&root.directory).unwrap());
        std::fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn swap_open_swap_back_never_returns_replacement_bytes() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let parent = std::env::temp_dir().join(format!(
            "unica-safe-root-swap-{}-{nonce}",
            std::process::id()
        ));
        let root_path = parent.join("root");
        let outside_path = parent.join("outside");
        std::fs::create_dir_all(&root_path).unwrap();
        std::fs::write(root_path.join("owner.xml"), b"authorized").unwrap();
        std::fs::write(&outside_path, b"outside").unwrap();
        let root = SafeSourceRoot::capture(&root_path, &root_path).unwrap();

        let before_root = root_path.clone();
        let before_outside = outside_path.clone();
        let after_root = root_path.clone();
        let after_outside = outside_path.clone();
        let result = with_artifact_open_hooks(
            move |_| {
                std::fs::rename(
                    before_root.join("owner.xml"),
                    before_root.join("owner.saved"),
                )
                .unwrap();
                std::fs::rename(&before_outside, before_root.join("owner.xml")).unwrap();
            },
            move |_| {
                std::fs::rename(after_root.join("owner.xml"), &after_outside).unwrap();
                std::fs::rename(after_root.join("owner.saved"), after_root.join("owner.xml"))
                    .unwrap();
            },
            || root.read_relative("owner.xml", ArtifactReadLimit::Descriptor),
        );

        assert!(matches!(result, Err(SafeRootError::IdentityChanged)));
        assert_eq!(
            std::fs::read(root_path.join("owner.xml")).unwrap(),
            b"authorized"
        );
        std::fs::remove_dir_all(parent).unwrap();
    }
}
