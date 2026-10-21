use crate::compile_transaction::{
    CompileTransaction, RetainedApplyChangeBinding, RetainedApplyReadGuardBinding,
    RetainedApplyValidationError, RetainedApplyValidationErrorKind,
};
use crate::paths::GENERATED_DIR_NAME;
use crate::platform::filesystem::{
    FileIdentity, RetainedChildCapability, RetainedChildNameComparator, RetainedDirectoryCapability,
};
use sha2::Digest;
use std::ffi::{OsStr, OsString};
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use unica_work_control::cancellation::CancellationToken;
use unica_work_control::provider_deadline::ProviderDeadline;

const MAX_APPLY_FILE_BYTES: usize = 32 * 1024 * 1024;
pub(super) const MAX_APPLY_NAMESPACE_ENTRIES: usize = 1_000_000;
pub(super) const MAX_APPLY_NAMESPACE_DEPTH: usize = 256;

/// A filesystem call on the planned route failed: access denied, I/O, or
/// a retained name that no longer resolves. The source cannot be read, which
/// is not a containment violation (links and aliases are rejected by kind).
fn source_route_io_error(error: std::io::Error) -> ApplyStagingError {
    ApplyStagingError::new(
        ApplyStagingErrorKind::UnsupportedProvider,
        format!("source route cannot be read: {error}"),
    )
}

/// A planned path whose identity bytes cannot be derived for the plan hash.
fn path_identity_error(error: impl std::fmt::Display) -> ApplyStagingError {
    ApplyStagingError::new(
        ApplyStagingErrorKind::UnsupportedProvider,
        format!("source path identity cannot be derived: {error}"),
    )
}

fn reject_generated_component(
    comparator: Option<&RetainedChildNameComparator>,
    component: &OsStr,
) -> Result<(), ApplyStagingError> {
    let Some(comparator) = comparator else {
        return Ok(());
    };
    if comparator
        .names_equivalent(component, OsStr::new(GENERATED_DIR_NAME))
        .map_err(source_route_io_error)?
    {
        return Err(ApplyStagingError::new(
            ApplyStagingErrorKind::ContainmentIdentity,
            "source participant cannot address the generated subtree",
        ));
    }
    Ok(())
}

pub(crate) use unica_source_contract::proposal::{
    ApplyStagingError, ApplyStagingErrorKind, StagedApplyChange, StagedChangeKind, StagedFileState,
};

impl From<RetainedApplyValidationError> for ApplyStagingError {
    fn from(error: RetainedApplyValidationError) -> Self {
        let kind = match error.kind() {
            RetainedApplyValidationErrorKind::Cancelled => ApplyStagingErrorKind::Cancelled,
            RetainedApplyValidationErrorKind::Deadline => ApplyStagingErrorKind::Deadline,
            RetainedApplyValidationErrorKind::ConcurrentRevision => {
                ApplyStagingErrorKind::ConcurrentRevision
            }
            RetainedApplyValidationErrorKind::ContainmentIdentity => {
                ApplyStagingErrorKind::ContainmentIdentity
            }
            RetainedApplyValidationErrorKind::AbsentChainOccupied => {
                ApplyStagingErrorKind::AbsentChainOccupied
            }
            RetainedApplyValidationErrorKind::UnsupportedProvider => {
                ApplyStagingErrorKind::UnsupportedProvider
            }
            RetainedApplyValidationErrorKind::Invariant => ApplyStagingErrorKind::Invariant,
        };
        Self::new(kind, error.to_string())
    }
}

#[derive(Debug)]
struct StagedEntry {
    relative_path: PathBuf,
    route_identities: Vec<FileIdentity>,
    missing_parent_chain: Vec<OsString>,
    name: OsString,
    target_identity: StagedTargetIdentity,
    original: StagedFileState,
    current: StagedFileState,
    original_file_identity: Option<FileIdentity>,
    original_len: Option<u64>,
    original_sha256: Option<[u8; 32]>,
}

#[derive(Debug)]
enum StagedTargetIdentity {
    Existing(FileIdentity),
    Absent {
        ancestor: FileIdentity,
        suffix: Vec<OsString>,
    },
}

/// A directory that a planner actually enumerated. Child kinds, including
/// absence, are input evidence; only this operation's publications may alter it.
#[derive(Debug)]
pub(super) struct RetainedNamespaceGuard {
    root: Arc<RetainedDirectoryCapability>,
    relative: PathBuf,
    identity: Option<FileIdentity>,
    children: Option<std::collections::BTreeMap<OsString, u8>>,
    entry_limit: usize,
    deadline: ProviderDeadline,
    cancellation: CancellationToken,
}

type NamespaceObservation = (
    Option<FileIdentity>,
    Option<std::collections::BTreeMap<OsString, u8>>,
);

impl RetainedNamespaceGuard {
    pub(super) fn relative_path(&self) -> &Path {
        &self.relative
    }

    pub(super) fn rebind_execution_context(
        &mut self,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) {
        self.deadline = deadline;
        self.cancellation = cancellation.clone();
    }

    fn capture(
        root: Arc<RetainedDirectoryCapability>,
        relative: PathBuf,
        entry_limit: usize,
        deadline: ProviderDeadline,
        cancellation: CancellationToken,
    ) -> Result<Self, ApplyStagingError> {
        let mut guard = Self {
            root,
            relative,
            identity: None,
            children: None,
            entry_limit,
            deadline,
            cancellation,
        };
        let (identity, children) = guard.observe(0)?;
        guard.identity = identity;
        guard.children = children;
        Ok(guard)
    }

    fn observe(
        &self,
        own_entry_allowance: usize,
    ) -> Result<NamespaceObservation, ApplyStagingError> {
        let mut directory = self.root.as_ref().clone();
        directory
            .validate_named_identity()
            .map_err(source_route_io_error)?;
        for component in self.relative.components() {
            self.checkpoint()?;
            let Component::Normal(name) = component else {
                return Err(ApplyStagingError::new(
                    ApplyStagingErrorKind::Invariant,
                    "namespace path is not relative",
                ));
            };
            match directory.retain_immediate_child_nofollow(name) {
                Ok(RetainedChildCapability::Directory(child)) => directory = child,
                Err(error) if error.kind() == ErrorKind::NotFound => return Ok((None, None)),
                Ok(RetainedChildCapability::ReparsePoint) => {
                    return Err(ApplyStagingError::new(
                        ApplyStagingErrorKind::ContainmentIdentity,
                        "namespace input is a link/reparse point",
                    ))
                }
                Ok(_) => {
                    return Err(ApplyStagingError::new(
                        ApplyStagingErrorKind::AbsentChainOccupied,
                        "namespace input is not a directory",
                    ))
                }
                Err(error) => return Err(source_route_io_error(error)),
            }
        }
        let mut stopped = None;
        let names = directory
            .read_immediate_names_bounded(
                self.entry_limit.saturating_add(own_entry_allowance),
                || {
                    self.checkpoint().map_err(|error| {
                        stopped = Some(error);
                        std::io::Error::other("namespace input checkpoint stopped enumeration")
                    })
                },
            )
            .map_err(|error| stopped.unwrap_or_else(|| source_route_io_error(error)))?;
        let comparator = directory
            .child_name_comparator()
            .map_err(source_route_io_error)?;
        let mut children = std::collections::BTreeMap::new();
        for name in names {
            self.checkpoint()?;
            if comparator
                .names_equivalent(&name, OsStr::new(GENERATED_DIR_NAME))
                .map_err(source_route_io_error)?
            {
                continue;
            }
            let kind = match directory
                .retain_immediate_child_nofollow(&name)
                .map_err(source_route_io_error)?
            {
                RetainedChildCapability::Directory(_) => 1,
                RetainedChildCapability::RegularFile(_) => 2,
                RetainedChildCapability::ReparsePoint => {
                    return Err(ApplyStagingError::new(
                        ApplyStagingErrorKind::ContainmentIdentity,
                        "enumerated input contains a link/reparse point",
                    ))
                }
                RetainedChildCapability::Unsupported => {
                    return Err(ApplyStagingError::new(
                        ApplyStagingErrorKind::AbsentChainOccupied,
                        "enumerated input contains an unsupported entry",
                    ))
                }
            };
            children.insert(name, kind);
        }
        directory
            .validate_named_identity()
            .map_err(source_route_io_error)?;
        self.checkpoint()?;
        Ok((Some(directory.identity()), Some(children)))
    }

    fn checkpoint(&self) -> Result<(), ApplyStagingError> {
        if self.cancellation.is_cancelled() {
            return Err(ApplyStagingError::new(
                ApplyStagingErrorKind::Cancelled,
                "namespace input check cancelled",
            ));
        }
        if self.deadline.is_elapsed() {
            return Err(ApplyStagingError::new(
                ApplyStagingErrorKind::Deadline,
                "namespace input check deadline elapsed",
            ));
        }
        Ok(())
    }

    pub(super) fn validate(
        &self,
        deltas: &[(PathBuf, Option<u8>)],
    ) -> Result<(), ApplyStagingError> {
        let (identity, actual) = self.observe(deltas.len())?;
        if self.identity.is_some() && self.identity != identity {
            return Err(ApplyStagingError::new(
                ApplyStagingErrorKind::ConcurrentRevision,
                "enumerated directory identity changed",
            ));
        }
        let mut expected = self.children.clone();
        for (path, kind) in deltas {
            if path
                .components()
                .any(|part| part.as_os_str() == OsStr::new(GENERATED_DIR_NAME))
            {
                continue;
            }
            if path == &self.relative && *kind == Some(1) {
                expected.get_or_insert_with(Default::default);
            }
            if path.parent() == Some(self.relative.as_path()) {
                if let Some(name) = path.file_name() {
                    if let Some(kind) = kind {
                        expected
                            .get_or_insert_with(Default::default)
                            .insert(name.to_os_string(), *kind);
                    } else if let Some(entries) = expected.as_mut() {
                        entries.remove(name);
                    }
                }
            }
        }
        if actual != expected {
            return Err(ApplyStagingError::new(
                ApplyStagingErrorKind::ConcurrentRevision,
                format!("enumerated namespace changed: {}", self.relative.display()),
            ));
        }
        self.checkpoint()
    }

    pub(super) fn validate_relocated(
        &self,
        root: Arc<RetainedDirectoryCapability>,
        relative: PathBuf,
    ) -> Result<(), ApplyStagingError> {
        Self {
            root,
            relative,
            identity: self.identity,
            children: self.children.clone(),
            entry_limit: self.entry_limit,
            deadline: self.deadline,
            cancellation: self.cancellation.clone(),
        }
        .validate(&[])
    }

    fn fingerprint(&self, hash: &mut sha2::Sha256) -> Result<(), ApplyStagingError> {
        use sha2::Digest;
        self.checkpoint()?;
        let path = crate::platform::filesystem::stable_path_identity_bytes(&self.relative)
            .map_err(path_identity_error)?;
        hash.update((path.len() as u64).to_be_bytes());
        hash.update(path);
        match &self.children {
            None => hash.update([0]),
            Some(children) => {
                hash.update([1]);
                hash.update((children.len() as u64).to_be_bytes());
                for (name, kind) in children {
                    self.checkpoint()?;
                    let name =
                        crate::platform::filesystem::stable_path_identity_bytes(Path::new(name))
                            .map_err(path_identity_error)?;
                    hash.update((name.len() as u64).to_be_bytes());
                    hash.update(name);
                    hash.update([*kind]);
                }
            }
        }
        Ok(())
    }
}

/// The spelling a surviving file creation needs in a case-sensitive checkout.
/// This is independent of the overlay entry's first, possibly aliased path.
#[derive(Debug)]
pub(super) struct RetainedPathSpellingGuard {
    root: Arc<RetainedDirectoryCapability>,
    relative: PathBuf,
    entry_limit: usize,
    deadline: ProviderDeadline,
    cancellation: CancellationToken,
}

impl RetainedPathSpellingGuard {
    pub(super) fn relative_path(&self) -> &Path {
        &self.relative
    }

    pub(super) fn rebind_execution_context(
        &mut self,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) {
        self.deadline = deadline;
        self.cancellation = cancellation.clone();
    }

    fn checkpoint(&self) -> Result<(), ApplyStagingError> {
        if self.cancellation.is_cancelled() {
            return Err(ApplyStagingError::new(
                ApplyStagingErrorKind::Cancelled,
                "destination spelling check cancelled",
            ));
        }
        if self.deadline.is_elapsed() {
            return Err(ApplyStagingError::new(
                ApplyStagingErrorKind::Deadline,
                "destination spelling check deadline elapsed",
            ));
        }
        Ok(())
    }

    fn validate_initial(&self) -> Result<(), ApplyStagingError> {
        self.validate_with_mismatch_kind(ApplyStagingErrorKind::UnsupportedProvider)
    }

    pub(super) fn validate(&self) -> Result<(), ApplyStagingError> {
        self.validate_with_mismatch_kind(ApplyStagingErrorKind::ConcurrentRevision)
    }

    fn validate_with_mismatch_kind(
        &self,
        mismatch_kind: ApplyStagingErrorKind,
    ) -> Result<(), ApplyStagingError> {
        self.checkpoint()?;
        let mut parent = self.root.as_ref().clone();
        parent
            .validate_named_identity()
            .map_err(source_route_io_error)?;
        let mut entries = 0usize;
        let mut components = self.relative.components().peekable();
        while let Some(component) = components.next() {
            self.checkpoint()?;
            let Component::Normal(name) = component else {
                return Err(ApplyStagingError::new(
                    ApplyStagingErrorKind::Invariant,
                    "destination spelling guard path is not relative",
                ));
            };
            let child = match parent.retain_immediate_child_nofollow(name) {
                Ok(child) => child,
                Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
                Err(error) => return Err(source_route_io_error(error)),
            };
            let mut exact_name_found = false;
            let mut stopped = None;
            parent
                .visit_immediate_names::<std::io::Error>(|candidate| {
                    self.checkpoint().map_err(|error| {
                        stopped = Some(error);
                        std::io::Error::other("destination spelling enumeration stopped")
                    })?;
                    entries = entries
                        .checked_add(1)
                        .filter(|count| *count <= self.entry_limit)
                        .ok_or_else(|| {
                            stopped = Some(ApplyStagingError::new(
                                ApplyStagingErrorKind::UnsupportedProvider,
                                "destination spelling check exceeds namespace entry bound",
                            ));
                            std::io::Error::other("destination spelling entry bound exceeded")
                        })?;
                    exact_name_found |= candidate == name;
                    Ok(())
                })
                .map_err(|error| stopped.unwrap_or_else(|| source_route_io_error(error)))?;
            if !exact_name_found {
                return Err(ApplyStagingError::new(
                    mismatch_kind,
                    format!(
                        "destination route has no exact spelling for `{}`: {}",
                        name.to_string_lossy(),
                        self.relative.to_string_lossy().replace('\\', "/")
                    ),
                ));
            }
            match child {
                RetainedChildCapability::Directory(directory) if components.peek().is_some() => {
                    parent = directory;
                }
                RetainedChildCapability::RegularFile(_) if components.peek().is_none() => {
                    self.checkpoint()?;
                    return Ok(());
                }
                RetainedChildCapability::ReparsePoint => {
                    return Err(ApplyStagingError::new(
                        ApplyStagingErrorKind::ContainmentIdentity,
                        "destination spelling route contains a link/reparse point",
                    ));
                }
                _ => {
                    return Err(ApplyStagingError::new(
                        ApplyStagingErrorKind::AbsentChainOccupied,
                        "destination spelling route has an incompatible entry kind",
                    ));
                }
            }
        }
        Ok(())
    }

    fn fingerprint(&self, hash: &mut sha2::Sha256) -> Result<(), ApplyStagingError> {
        self.checkpoint()?;
        let path = crate::platform::filesystem::stable_path_identity_bytes(&self.relative)
            .map_err(path_identity_error)?;
        hash.update((path.len() as u64).to_be_bytes());
        hash.update(path);
        Ok(())
    }
}

#[derive(Debug)]
pub struct ApplyStagedState {
    root: Arc<RetainedDirectoryCapability>,
    entries: Vec<StagedEntry>,
    read_guards: std::collections::BTreeMap<FileIdentity, RetainedApplyReadGuardBinding>,
    namespaces: std::collections::BTreeMap<PathBuf, RetainedNamespaceGuard>,
    path_spellings: std::collections::BTreeMap<usize, RetainedPathSpellingGuard>,
    directory_removals: Vec<super::retained_directory::DirectoryRemoval>,
    deadline: ProviderDeadline,
    cancellation: CancellationToken,
    writer_authority: crate::actor::ApplyWriterAuthority,
    generated_subtree_forbidden: bool,
    #[cfg(test)]
    absent_name_identity_for_test: Option<fn(&std::ffi::OsStr, &std::ffi::OsStr) -> bool>,
}

impl ApplyStagedState {
    pub fn proposal(&mut self) -> unica_source_contract::proposal::ApplyProposal<'_> {
        unica_source_contract::proposal::ApplyProposal::from_port(self)
    }

    pub(crate) fn from_retained_root(
        root: Arc<RetainedDirectoryCapability>,
        deadline: ProviderDeadline,
        cancellation: CancellationToken,
        writer_authority: crate::actor::ApplyWriterAuthority,
    ) -> Self {
        Self {
            root,
            entries: Vec::new(),
            read_guards: Default::default(),
            namespaces: Default::default(),
            path_spellings: Default::default(),
            directory_removals: Vec::new(),
            deadline,
            cancellation,
            writer_authority,
            generated_subtree_forbidden: false,
            #[cfg(test)]
            absent_name_identity_for_test: None,
        }
    }

    pub(crate) fn forbid_generated_subtree(mut self) -> Self {
        self.generated_subtree_forbidden = true;
        self
    }

    /// Enumerate only a subtree needed by this planner and retain its membership.
    pub fn enumerate_tree(&mut self, relative: &Path) -> Result<Vec<PathBuf>, ApplyStagingError> {
        self.enumerate_tree_with_limits(
            relative,
            MAX_APPLY_NAMESPACE_DEPTH,
            MAX_APPLY_NAMESPACE_ENTRIES,
        )
    }

    /// Retain the enumerated membership and distinguish a missing root from
    /// an existing empty directory. Both observations remain apply preconditions.
    pub fn enumerate_tree_with_root_absence(
        &mut self,
        relative: &Path,
    ) -> Result<(Vec<PathBuf>, bool), ApplyStagingError> {
        let files = self.enumerate_tree(relative)?;
        let guard = self.namespaces.get(relative).ok_or_else(|| {
            ApplyStagingError::new(
                ApplyStagingErrorKind::Invariant,
                "enumerated namespace proof is missing",
            )
        })?;
        Ok((files, guard.children.is_none()))
    }

    fn enumerate_tree_with_limits(
        &mut self,
        relative: &Path,
        max_depth: usize,
        max_entries: usize,
    ) -> Result<Vec<PathBuf>, ApplyStagingError> {
        let initial_depth = relative.components().count();
        let mut pending = vec![relative.to_path_buf()];
        let mut files = Vec::new();
        let mut count = 0usize;
        while let Some(path) = pending.pop() {
            if path.components().count() - initial_depth > max_depth {
                return Err(ApplyStagingError::new(
                    ApplyStagingErrorKind::UnsupportedProvider,
                    "namespace input exceeds maximum depth",
                ));
            }
            let guard = RetainedNamespaceGuard::capture(
                Arc::clone(&self.root),
                path.clone(),
                max_entries.saturating_sub(count),
                self.deadline,
                self.cancellation.clone(),
            )?;
            if let Some(children) = &guard.children {
                count = count.checked_add(children.len()).ok_or_else(|| {
                    ApplyStagingError::new(
                        ApplyStagingErrorKind::UnsupportedProvider,
                        "namespace entry count overflow",
                    )
                })?;
                if count > max_entries {
                    return Err(ApplyStagingError::new(
                        ApplyStagingErrorKind::UnsupportedProvider,
                        "namespace input exceeds entry bound",
                    ));
                }
                for (name, kind) in children {
                    if *kind == 1 {
                        pending.push(path.join(name));
                    } else {
                        files.push(path.join(name));
                    }
                }
            }
            if let Some(previous) = self.namespaces.get(&path) {
                previous.validate(&[])?;
            } else {
                self.namespaces.insert(path, guard);
            }
        }
        files.sort();
        Ok(files)
    }

    pub fn read(&mut self, relative: &Path) -> Result<Option<Vec<u8>>, ApplyStagingError> {
        self.read_bounded(relative, MAX_APPLY_FILE_BYTES)
    }

    /// The path that identifies this retained overlay entry in final changes.
    /// Filesystem aliases keep the first retained spelling, so events must use
    /// it when the caller reached that same entry through another spelling.
    pub fn retained_relative_path(
        &mut self,
        relative: &Path,
    ) -> Result<PathBuf, ApplyStagingError> {
        let relative = strict_relative(relative)?;
        let index = self.ensure_loaded(&relative)?;
        Ok(self.entries[index].relative_path.clone())
    }

    /// Register the destination intended by this creation. Later creations of
    /// the same retained entry replace it; a final removal makes it inactive.
    /// Validation waits for the final batch, including byte-neutral aliases.
    pub fn require_exact_path_spelling(
        &mut self,
        relative: &Path,
    ) -> Result<(), ApplyStagingError> {
        let relative = strict_relative(relative)?;
        let index = self.ensure_loaded(&relative)?;
        self.path_spellings.insert(
            index,
            RetainedPathSpellingGuard {
                root: Arc::clone(&self.root),
                relative,
                entry_limit: MAX_APPLY_NAMESPACE_ENTRIES,
                deadline: self.deadline,
                cancellation: self.cancellation.clone(),
            },
        );
        Ok(())
    }

    pub fn read_bounded(
        &mut self,
        relative: &Path,
        limit: usize,
    ) -> Result<Option<Vec<u8>>, ApplyStagingError> {
        let relative = strict_relative(relative)?;
        let index = self.ensure_loaded_bounded(&relative, limit.min(MAX_APPLY_FILE_BYTES))?;
        if matches!(&self.entries[index].current, StagedFileState::Bytes(bytes) if bytes.len() > limit)
        {
            return Err(ApplyStagingError::new(
                ApplyStagingErrorKind::UnsupportedProvider,
                format!("staged input exceeds read bound: {}", relative.display()),
            ));
        }
        Ok(self.entries[index].current.as_option())
    }

    /// Borrow the already retained first image of this one target; no new read
    /// or copy is made. A local editor can restore a reverted XML subtree
    /// without overwriting other properties changed by an intervening family.
    pub fn original_image(&self, relative: &Path) -> Option<&[u8]> {
        self.entries
            .iter()
            .find(|entry| entry.relative_path == relative)
            .and_then(|entry| match &entry.original {
                StagedFileState::Bytes(bytes) => Some(bytes.as_slice()),
                StagedFileState::Absent => None,
            })
    }

    /// Read one reference input without retaining its complete body in the
    /// saved plan. The caller owns the temporary bytes only for this scan.
    pub fn read_guarded_bounded(
        &mut self,
        relative: &Path,
        limit: usize,
    ) -> Result<Option<Vec<u8>>, ApplyStagingError> {
        let relative = strict_relative(relative)?;
        self.checkpoint("apply reference input")?;
        let components = relative
            .components()
            .map(|part| match part {
                Component::Normal(name) => Ok(name.to_os_string()),
                _ => Err(ApplyStagingError::new(
                    ApplyStagingErrorKind::ContainmentIdentity,
                    "reference input path is not relative",
                )),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut parent = self.root.as_ref().clone();
        let mut route_identities = Vec::with_capacity(components.len() - 1);
        for component in &components[..components.len() - 1] {
            self.checkpoint("apply reference input route")?;
            if self.generated_subtree_forbidden {
                let policy = parent
                    .child_name_comparator()
                    .map_err(source_route_io_error)?;
                reject_generated_component(Some(&policy), component)?;
            }
            parent = match parent.retain_immediate_child_nofollow(component) {
                Ok(RetainedChildCapability::Directory(child)) => {
                    route_identities.push(child.identity());
                    child
                }
                Err(error) if error.kind() == ErrorKind::NotFound => {
                    return self.read_bounded(&relative, limit);
                }
                Ok(RetainedChildCapability::ReparsePoint) => {
                    return Err(ApplyStagingError::new(
                        ApplyStagingErrorKind::ContainmentIdentity,
                        format!(
                            "reference input parent is a link/reparse point: {}",
                            relative.display()
                        ),
                    ))
                }
                Ok(_) => {
                    return Err(ApplyStagingError::new(
                        ApplyStagingErrorKind::AbsentChainOccupied,
                        format!(
                            "reference input parent is not a directory: {}",
                            relative.display()
                        ),
                    ))
                }
                Err(error) => return Err(source_route_io_error(error)),
            };
        }
        let name = components.last().expect("strict path has a name").clone();
        if self.generated_subtree_forbidden {
            let policy = parent
                .child_name_comparator()
                .map_err(source_route_io_error)?;
            reject_generated_component(Some(&policy), &name)?;
        }
        let file = match parent.retain_immediate_child_nofollow(&name) {
            Ok(RetainedChildCapability::RegularFile(file)) => file,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return self.read_bounded(&relative, limit);
            }
            Ok(RetainedChildCapability::ReparsePoint) => {
                return Err(ApplyStagingError::new(
                    ApplyStagingErrorKind::ContainmentIdentity,
                    format!(
                        "reference input is a link/reparse point: {}",
                        relative.display()
                    ),
                ))
            }
            Ok(_) => {
                return Err(ApplyStagingError::new(
                    ApplyStagingErrorKind::AbsentChainOccupied,
                    format!(
                        "reference input is not a regular file: {}",
                        relative.display()
                    ),
                ))
            }
            Err(error) => return Err(source_route_io_error(error)),
        };
        if file.hard_link_count().map_err(|error| {
            ApplyStagingError::new(
                ApplyStagingErrorKind::UnsupportedProvider,
                format!("reference input hard-link count failed: {error}"),
            )
        })? != 1
        {
            return Err(ApplyStagingError::new(
                ApplyStagingErrorKind::ContainmentIdentity,
                format!(
                    "reference input has a hard-link alias: {}",
                    relative.display()
                ),
            ));
        }
        let id = file.identity();
        if let Some(entry) = self.entries.iter().find(|entry| {
            matches!(&entry.target_identity, StagedTargetIdentity::Existing(existing) if *existing == id)
        }) {
            let current = entry.current.as_option();
            if current.as_ref().is_some_and(|bytes| bytes.len() > limit) {
                return Err(ApplyStagingError::new(
                    ApplyStagingErrorKind::UnsupportedProvider,
                    format!("staged input exceeds read bound: {}", relative.display()),
                ));
            }
            return Ok(current);
        }
        let bytes = file
            .read_bounded(limit.min(MAX_APPLY_FILE_BYTES))
            .map_err(|error| {
                ApplyStagingError::new(
                    ApplyStagingErrorKind::UnsupportedProvider,
                    format!("reference input fixed read bound failed: {error}"),
                )
            })?;
        self.checkpoint("apply reference input result")?;
        let digest: [u8; 32] = sha2::Sha256::digest(&bytes).into();
        if let Some(guard) = self.read_guards.get(&id) {
            if guard.expected_len != bytes.len() as u64 || guard.expected_sha256 != digest {
                return Err(ApplyStagingError::new(
                    ApplyStagingErrorKind::ConcurrentRevision,
                    format!(
                        "reference input changed while planning: {}",
                        relative.display()
                    ),
                ));
            }
        } else {
            self.read_guards.insert(
                id,
                RetainedApplyReadGuardBinding {
                    root: Arc::clone(&self.root),
                    relative_path: relative,
                    route_identities,
                    file_identity: id,
                    expected_len: bytes.len() as u64,
                    expected_sha256: digest,
                    deadline: self.deadline,
                    cancellation: self.cancellation.clone(),
                },
            );
        }
        Ok(Some(bytes))
    }

    pub fn create(
        &mut self,
        relative: impl AsRef<Path>,
        bytes: Vec<u8>,
    ) -> Result<(), ApplyStagingError> {
        let relative = strict_relative(relative.as_ref())?;
        let index = self.ensure_loaded(&relative)?;
        let entry = &mut self.entries[index];
        if entry.current != StagedFileState::Absent {
            return Err(ApplyStagingError::new(
                ApplyStagingErrorKind::Invariant,
                format!(
                    "staged create target already exists: {}",
                    relative.display()
                ),
            ));
        }
        entry.current = StagedFileState::Bytes(bytes);
        if let Some(guard) = self.path_spellings.get_mut(&index) {
            guard.relative = relative;
        }
        Ok(())
    }

    pub fn replace(
        &mut self,
        relative: impl AsRef<Path>,
        expected_current: impl AsRef<[u8]>,
        bytes: Vec<u8>,
    ) -> Result<(), ApplyStagingError> {
        let relative = strict_relative(relative.as_ref())?;
        let index = self.ensure_loaded(&relative)?;
        let entry = &mut self.entries[index];
        if entry.current != StagedFileState::Bytes(expected_current.as_ref().to_vec()) {
            return Err(ApplyStagingError::new(
                ApplyStagingErrorKind::Invariant,
                format!("staged replace preimage changed: {}", relative.display()),
            ));
        }
        entry.current = StagedFileState::Bytes(bytes);
        Ok(())
    }

    pub fn remove(
        &mut self,
        relative: impl AsRef<Path>,
        expected_current: impl AsRef<[u8]>,
    ) -> Result<(), ApplyStagingError> {
        let relative = strict_relative(relative.as_ref())?;
        let index = self.ensure_loaded(&relative)?;
        let entry = &mut self.entries[index];
        if entry.current != StagedFileState::Bytes(expected_current.as_ref().to_vec()) {
            return Err(ApplyStagingError::new(
                ApplyStagingErrorKind::Invariant,
                format!("staged remove preimage changed: {}", relative.display()),
            ));
        }
        entry.current = StagedFileState::Absent;
        Ok(())
    }

    /// Authority covers this selected payload and its two collection entries,
    /// never unrelated siblings or the generated subtree. Retained manifests
    /// include empty directories, which file images cannot represent.
    pub fn remove_owned_directories(
        &mut self,
        payload: &Path,
        descriptor: &Path,
        collection: &Path,
    ) -> Result<(), ApplyStagingError> {
        use super::retained_directory::{DirectoryRemoval, RemovedFileImage};
        if !self.generated_subtree_forbidden {
            return Err(ApplyStagingError::new(
                ApplyStagingErrorKind::Invariant,
                "directory removal requires the source participant",
            ));
        }
        for plan in &self.directory_removals {
            plan.validate_source()?;
        }
        let files = self
            .entries
            .iter()
            .filter_map(|entry| match (&entry.original, &entry.current) {
                (StagedFileState::Bytes(bytes), StagedFileState::Absent) => Some((
                    entry
                        .original_file_identity
                        .expect("retained original identity"),
                    RemovedFileImage {
                        len: bytes.len(),
                        sha256: sha2::Sha256::digest(bytes).into(),
                    },
                )),
                _ => None,
            })
            .collect();
        let plans = DirectoryRemoval::capture_owned(
            Arc::clone(&self.root),
            payload,
            descriptor,
            collection,
            &files,
            self.deadline,
            self.cancellation.clone(),
        )?;
        self.directory_removals.extend(plans);
        self.directory_removals
            .sort_by(|left, right| left.relative.cmp(&right.relative));
        Ok(())
    }

    fn active_directory_removals(&self) -> Vec<super::retained_directory::DirectoryRemoval> {
        let surviving_paths = self
            .entries
            .iter()
            .filter(|entry| matches!(entry.current, StagedFileState::Bytes(_)))
            .map(|entry| entry.relative_path.clone())
            .collect::<Vec<_>>();
        self.directory_removals
            .iter()
            .filter(|plan| {
                self.entries.iter().any(|entry| {
                    entry.relative_path == plan.owner_descriptor
                        && matches!(entry.current, StagedFileState::Absent)
                })
            })
            .flat_map(|plan| plan.partition(&surviving_paths))
            .collect()
    }

    /// All observed inputs, including read-only dependencies and absent targets,
    /// and their planned postimages bind the preview to its exact write intent.
    pub fn fingerprint_inputs(&self, hash: &mut sha2::Sha256) -> Result<(), ApplyStagingError> {
        use sha2::Digest;
        hash.update(self.root.identity().stable_bytes());
        hash.update((self.entries.len() as u64).to_be_bytes());
        let mut entries = self.entries.iter().collect::<Vec<_>>();
        entries.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
        for entry in entries {
            self.checkpoint("apply plan inputs")?;
            let path =
                crate::platform::filesystem::stable_path_identity_bytes(&entry.relative_path)
                    .map_err(path_identity_error)?;
            hash.update((path.len() as u64).to_be_bytes());
            hash.update(path);
            for state in [&entry.original, &entry.current] {
                match state {
                    StagedFileState::Absent => hash.update([0]),
                    StagedFileState::Bytes(bytes) => {
                        hash.update([1]);
                        hash.update((bytes.len() as u64).to_be_bytes());
                        for chunk in bytes.chunks(64 * 1024) {
                            self.checkpoint("apply plan input content")?;
                            hash.update(chunk);
                        }
                    }
                }
            }
        }
        hash.update((self.read_guards.len() as u64).to_be_bytes());
        let mut guards = self.read_guards.values().collect::<Vec<_>>();
        guards.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
        for guard in guards {
            self.checkpoint("apply plan reference guard")?;
            let path =
                crate::platform::filesystem::stable_path_identity_bytes(&guard.relative_path)
                    .map_err(path_identity_error)?;
            hash.update((path.len() as u64).to_be_bytes());
            hash.update(path);
            hash.update(guard.expected_len.to_be_bytes());
            hash.update(guard.expected_sha256);
        }
        let directories = self.active_directory_removals();
        hash.update((directories.len() as u64).to_be_bytes());
        for plan in &directories {
            plan.fingerprint(hash)?;
        }
        hash.update((self.namespaces.len() as u64).to_be_bytes());
        for guard in self.namespaces.values() {
            guard.fingerprint(hash)?;
        }
        let mut path_spellings = self
            .path_spellings
            .iter()
            .filter(|(index, _)| matches!(self.entries[**index].current, StagedFileState::Bytes(_)))
            .map(|(_, guard)| guard)
            .collect::<Vec<_>>();
        path_spellings.sort_by(|left, right| left.relative.cmp(&right.relative));
        hash.update((path_spellings.len() as u64).to_be_bytes());
        for guard in path_spellings {
            guard.fingerprint(hash)?;
        }
        Ok(())
    }

    pub fn planned_changes(&self) -> Vec<StagedApplyChange> {
        let mut changes = self
            .entries
            .iter()
            .filter(|entry| entry.original != entry.current)
            .map(|entry| StagedApplyChange {
                relative_path: entry.relative_path.clone(),
                kind: match (&entry.original, &entry.current) {
                    (StagedFileState::Absent, StagedFileState::Bytes(_)) => {
                        StagedChangeKind::Create
                    }
                    (StagedFileState::Bytes(_), StagedFileState::Bytes(_)) => {
                        StagedChangeKind::Replace
                    }
                    (StagedFileState::Bytes(_), StagedFileState::Absent) => {
                        StagedChangeKind::Remove
                    }
                    (StagedFileState::Absent, StagedFileState::Absent) => unreachable!(),
                },
                original: entry.original.clone(),
                current: entry.current.clone(),
            })
            .collect::<Vec<_>>();
        changes.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
        changes
    }

    pub fn finalize(self) -> Result<CompileTransaction, ApplyStagingError> {
        self.checkpoint("apply finalization")?;
        let directories = self.active_directory_removals();
        let mut transaction = CompileTransaction::new();
        transaction
            .bind_retained_apply_root(Arc::clone(&self.root), &self.writer_authority)
            .map_err(|error| ApplyStagingError::new(ApplyStagingErrorKind::Invariant, error))?;
        transaction.bind_retained_directory_removals(directories);
        transaction.bind_retained_namespaces(self.namespaces.into_values().collect());
        transaction.bind_retained_apply_read_guards(self.read_guards.into_values().collect());
        let path_spellings = self
            .path_spellings
            .into_iter()
            .filter(|(index, _)| matches!(self.entries[*index].current, StagedFileState::Bytes(_)))
            .map(|(_, guard)| guard)
            .collect::<Vec<_>>();
        for guard in &path_spellings {
            guard.validate_initial()?;
        }
        transaction.bind_retained_path_spellings(path_spellings);
        let mut entries = self.entries;
        entries.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
        for entry in entries {
            transaction
                .bind_retained_apply_change(
                    RetainedApplyChangeBinding {
                        root: Arc::clone(&self.root),
                        relative_path: entry.relative_path,
                        route_identities: entry.route_identities,
                        missing_parent_chain: entry.missing_parent_chain,
                        name: entry.name,
                        original: entry.original.as_option(),
                        current: entry.current.as_option(),
                        original_file_identity: entry.original_file_identity,
                        original_len: entry.original_len,
                        original_sha256: entry.original_sha256,
                        deadline: self.deadline,
                        cancellation: self.cancellation.clone(),
                    },
                    &self.writer_authority,
                )
                .map_err(|error| ApplyStagingError::new(ApplyStagingErrorKind::Invariant, error))?;
        }
        transaction
            .validate_retained_for_apply_typed()
            .map_err(ApplyStagingError::from)?;
        Ok(transaction)
    }

    pub(crate) fn retained_root_identity(&self) -> crate::platform::filesystem::FileIdentity {
        self.root.identity()
    }

    pub(crate) fn has_writer_authority(
        &self,
        authority: &crate::actor::ApplyWriterAuthority,
    ) -> bool {
        &self.writer_authority == authority
    }

    #[cfg(test)]
    fn set_absent_name_identity_for_test(
        &mut self,
        comparator: fn(&std::ffi::OsStr, &std::ffi::OsStr) -> bool,
    ) {
        self.absent_name_identity_for_test = Some(comparator);
    }

    fn ensure_loaded(&mut self, relative: &Path) -> Result<usize, ApplyStagingError> {
        self.ensure_loaded_bounded(relative, MAX_APPLY_FILE_BYTES)
    }

    fn ensure_loaded_bounded(
        &mut self,
        relative: &Path,
        limit: usize,
    ) -> Result<usize, ApplyStagingError> {
        self.checkpoint("apply staged read")?;
        if let Some(index) = self
            .entries
            .iter()
            .position(|entry| entry.relative_path == relative)
        {
            return Ok(index);
        }
        let components = relative
            .components()
            .map(|component| match component {
                Component::Normal(name) => Ok(name.to_os_string()),
                _ => Err(ApplyStagingError::new(
                    ApplyStagingErrorKind::ContainmentIdentity,
                    format!(
                        "staged target must contain only normal relative components: {}",
                        relative.display()
                    ),
                )),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let name = components
            .last()
            .expect("strict non-empty relative path has a terminal name")
            .clone();
        let mut ancestor = self.root.as_ref().clone();
        let mut route_identities = Vec::new();
        let mut missing_parent_chain = Vec::new();
        let mut generated_name_comparator = self
            .generated_subtree_forbidden
            .then(|| ancestor.child_name_comparator())
            .transpose()
            .map_err(source_route_io_error)?;
        for (index, component) in components[..components.len() - 1].iter().enumerate() {
            reject_generated_component(generated_name_comparator.as_ref(), component)?;
            match ancestor.retain_immediate_child_nofollow(component) {
                Ok(RetainedChildCapability::Directory(directory)) => {
                    ancestor = directory;
                    route_identities.push(ancestor.identity());
                    generated_name_comparator = self
                        .generated_subtree_forbidden
                        .then(|| ancestor.child_name_comparator())
                        .transpose()
                        .map_err(source_route_io_error)?;
                }
                Err(error) if error.kind() == ErrorKind::NotFound => {
                    missing_parent_chain
                        .extend(components[index..components.len() - 1].iter().cloned());
                    for suffix in &components[index + 1..] {
                        reject_generated_component(generated_name_comparator.as_ref(), suffix)?;
                    }
                    break;
                }
                Ok(RetainedChildCapability::ReparsePoint) => {
                    return Err(ApplyStagingError::new(
                        ApplyStagingErrorKind::ContainmentIdentity,
                        format!(
                            "staged target parent is a link/reparse point: {}",
                            relative.display()
                        ),
                    ))
                }
                Ok(
                    RetainedChildCapability::RegularFile(_) | RetainedChildCapability::Unsupported,
                ) => {
                    return Err(ApplyStagingError::new(
                        ApplyStagingErrorKind::AbsentChainOccupied,
                        format!(
                            "staged target parent is not a directory: {}",
                            relative.display()
                        ),
                    ))
                }
                Err(error) => {
                    return Err(ApplyStagingError::new(
                        ApplyStagingErrorKind::UnsupportedProvider,
                        format!("staged target parent rejected link/reparse traversal: {error}"),
                    ))
                }
            }
        }
        if missing_parent_chain.is_empty() {
            reject_generated_component(generated_name_comparator.as_ref(), &name)?;
        }
        let retained_child = if missing_parent_chain.is_empty() {
            Some(ancestor.retain_immediate_child_nofollow(&name))
        } else {
            None
        };
        if let Some(Ok(RetainedChildCapability::RegularFile(file))) = retained_child.as_ref() {
            if file.hard_link_count().map_err(|error| {
                ApplyStagingError::new(
                    ApplyStagingErrorKind::UnsupportedProvider,
                    format!("hard-link count failed: {error}"),
                )
            })? != 1
            {
                return Err(ApplyStagingError::new(
                    ApplyStagingErrorKind::ContainmentIdentity,
                    format!(
                        "staged target has a hard-link alias: {}",
                        relative.display()
                    ),
                ));
            }
        }
        let target_identity = match retained_child.as_ref() {
            None => {
                let mut suffix = missing_parent_chain.clone();
                suffix.push(name.clone());
                Some(StagedTargetIdentity::Absent {
                    ancestor: ancestor.identity(),
                    suffix,
                })
            }
            Some(child) => match child {
                Ok(RetainedChildCapability::RegularFile(file)) => {
                    Some(StagedTargetIdentity::Existing(file.identity()))
                }
                Err(error) if error.kind() == ErrorKind::NotFound => {
                    Some(StagedTargetIdentity::Absent {
                        ancestor: ancestor.identity(),
                        suffix: vec![name.clone()],
                    })
                }
                _ => None,
            },
        };
        if let Some(target_identity) = target_identity.as_ref() {
            for (index, entry) in self.entries.iter().enumerate() {
                if self.same_target(&entry.target_identity, target_identity, &ancestor)? {
                    return Ok(index);
                }
            }
        }
        let (original, original_file_identity, original_len, original_sha256) = match retained_child
        {
            None => (StagedFileState::Absent, None, None, None),
            Some(Ok(RetainedChildCapability::RegularFile(file))) => {
                if file.hard_link_count().map_err(|error| {
                    ApplyStagingError::new(
                        ApplyStagingErrorKind::UnsupportedProvider,
                        format!("hard-link count failed: {error}"),
                    )
                })? != 1
                {
                    return Err(ApplyStagingError::new(
                        ApplyStagingErrorKind::ContainmentIdentity,
                        format!(
                            "staged target has a hard-link alias: {}",
                            relative.display()
                        ),
                    ));
                }
                let bytes = file.read_bounded(limit).map_err(|error| {
                    ApplyStagingError::new(
                        ApplyStagingErrorKind::UnsupportedProvider,
                        format!("staged target fixed read bound failed: {error}"),
                    )
                })?;
                let identity = file.identity();
                let len = bytes.len() as u64;
                let sha256 = sha2::Sha256::digest(&bytes).into();
                (
                    StagedFileState::Bytes(bytes),
                    Some(identity),
                    Some(len),
                    Some(sha256),
                )
            }
            Some(Ok(RetainedChildCapability::ReparsePoint)) => {
                return Err(ApplyStagingError::new(
                    ApplyStagingErrorKind::ContainmentIdentity,
                    format!(
                        "staged target is a link/reparse point: {}",
                        relative.display()
                    ),
                ))
            }
            Some(Ok(
                RetainedChildCapability::Directory(_) | RetainedChildCapability::Unsupported,
            )) => {
                return Err(ApplyStagingError::new(
                    ApplyStagingErrorKind::AbsentChainOccupied,
                    format!(
                        "staged target is not a regular file: {}",
                        relative.display()
                    ),
                ))
            }
            Some(Err(error)) if error.kind() == ErrorKind::NotFound => {
                (StagedFileState::Absent, None, None, None)
            }
            Some(Err(error)) => {
                return Err(ApplyStagingError::new(
                    ApplyStagingErrorKind::UnsupportedProvider,
                    format!("staged target inspection failed: {error}"),
                ))
            }
        };
        if let StagedTargetIdentity::Existing(identity) = target_identity
            .as_ref()
            .expect("regular or absent target has an identity")
        {
            if let Some(guard) = self.read_guards.get(identity) {
                let observed = guard.retain_named_route().map_err(|error| {
                    // A stop request keeps its own kind; any route change stales the plan.
                    let kind = match error.kind() {
                        RetainedApplyValidationErrorKind::Cancelled => {
                            ApplyStagingErrorKind::Cancelled
                        }
                        RetainedApplyValidationErrorKind::Deadline => {
                            ApplyStagingErrorKind::Deadline
                        }
                        _ => ApplyStagingErrorKind::ConcurrentRevision,
                    };
                    ApplyStagingError::new(
                        kind,
                        format!("reference input changed before write staging: {error}"),
                    )
                })?;
                if observed.identity() != guard.file_identity {
                    return Err(ApplyStagingError::new(
                        ApplyStagingErrorKind::ConcurrentRevision,
                        format!(
                            "reference input changed before write staging: {}",
                            relative.display()
                        ),
                    ));
                }
                drop(observed);
                let StagedFileState::Bytes(bytes) = &original else {
                    unreachable!("guarded target was a regular file")
                };
                let digest: [u8; 32] = sha2::Sha256::digest(bytes).into();
                if guard.expected_len != bytes.len() as u64 || guard.expected_sha256 != digest {
                    return Err(ApplyStagingError::new(
                        ApplyStagingErrorKind::ConcurrentRevision,
                        format!(
                            "reference input changed before write staging: {}",
                            relative.display()
                        ),
                    ));
                }
                self.read_guards.remove(identity);
            }
        }
        self.entries.push(StagedEntry {
            relative_path: relative.to_path_buf(),
            route_identities,
            missing_parent_chain,
            name,
            target_identity: target_identity.expect("regular or absent target has an identity"),
            current: original.clone(),
            original,
            original_file_identity,
            original_len,
            original_sha256,
        });
        Ok(self.entries.len() - 1)
    }

    fn same_target(
        &self,
        left: &StagedTargetIdentity,
        right: &StagedTargetIdentity,
        right_parent: &RetainedDirectoryCapability,
    ) -> Result<bool, ApplyStagingError> {
        match (left, right) {
            (StagedTargetIdentity::Existing(left), StagedTargetIdentity::Existing(right)) => {
                Ok(left == right)
            }
            (
                StagedTargetIdentity::Absent {
                    ancestor: left_ancestor,
                    suffix: left_suffix,
                },
                StagedTargetIdentity::Absent {
                    ancestor: right_ancestor,
                    suffix: right_suffix,
                },
            ) if left_ancestor == right_ancestor && left_suffix.len() == right_suffix.len() => {
                for (left_name, right_name) in left_suffix.iter().zip(right_suffix) {
                    #[cfg(test)]
                    if let Some(comparator) = self.absent_name_identity_for_test {
                        if !comparator(left_name, right_name) {
                            return Ok(false);
                        }
                        continue;
                    }
                    if !right_parent
                        .child_names_equivalent(left_name, right_name)
                        .map_err(|error| {
                            ApplyStagingError::new(
                                ApplyStagingErrorKind::UnsupportedProvider,
                                format!("staged child-name identity cannot be proven: {error}"),
                            )
                        })?
                    {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    pub(super) fn checkpoint(&self, phase: &str) -> Result<(), ApplyStagingError> {
        if self.cancellation.is_cancelled() {
            Err(ApplyStagingError::new(
                ApplyStagingErrorKind::Cancelled,
                format!("{phase} cancelled"),
            ))
        } else if self.deadline.is_elapsed() {
            Err(ApplyStagingError::new(
                ApplyStagingErrorKind::Deadline,
                format!("{phase} deadline exceeded"),
            ))
        } else {
            Ok(())
        }
    }
}

impl unica_source_contract::proposal::SourceProposal for ApplyStagedState {
    fn checkpoint(&self, phase: &str) -> Result<(), ApplyStagingError> {
        self.checkpoint(phase)
    }
    fn binding(&self) -> unica_source_contract::proposal::ProposalBinding {
        unica_source_contract::proposal::ProposalBinding::from_observation(
            self.root.identity(),
            self.writer_authority.scope_identity(),
        )
    }
    fn enumerate_tree(&mut self, relative: &Path) -> Result<Vec<PathBuf>, ApplyStagingError> {
        self.enumerate_tree(relative)
    }
    fn enumerate_tree_with_root_absence(
        &mut self,
        relative: &Path,
    ) -> Result<(Vec<PathBuf>, bool), ApplyStagingError> {
        self.enumerate_tree_with_root_absence(relative)
    }
    fn read(&mut self, relative: &Path) -> Result<Option<Vec<u8>>, ApplyStagingError> {
        self.read(relative)
    }
    fn retained_relative_path(&mut self, relative: &Path) -> Result<PathBuf, ApplyStagingError> {
        self.retained_relative_path(relative)
    }
    fn require_exact_path_spelling(&mut self, relative: &Path) -> Result<(), ApplyStagingError> {
        self.require_exact_path_spelling(relative)
    }
    fn read_bounded(
        &mut self,
        relative: &Path,
        limit: usize,
    ) -> Result<Option<Vec<u8>>, ApplyStagingError> {
        self.read_bounded(relative, limit)
    }
    fn original_image(&self, relative: &Path) -> Option<&[u8]> {
        self.original_image(relative)
    }
    fn read_guarded_bounded(
        &mut self,
        relative: &Path,
        limit: usize,
    ) -> Result<Option<Vec<u8>>, ApplyStagingError> {
        self.read_guarded_bounded(relative, limit)
    }
    fn create(&mut self, relative: &Path, bytes: Vec<u8>) -> Result<(), ApplyStagingError> {
        self.create(relative, bytes)
    }
    fn replace(
        &mut self,
        relative: &Path,
        expected_current: &[u8],
        bytes: Vec<u8>,
    ) -> Result<(), ApplyStagingError> {
        self.replace(relative, expected_current, bytes)
    }
    fn remove(
        &mut self,
        relative: &Path,
        expected_current: &[u8],
    ) -> Result<(), ApplyStagingError> {
        self.remove(relative, expected_current)
    }
    fn remove_owned_directories(
        &mut self,
        payload: &Path,
        descriptor: &Path,
        collection: &Path,
    ) -> Result<(), ApplyStagingError> {
        self.remove_owned_directories(payload, descriptor, collection)
    }
    fn planned_changes(&self) -> Vec<StagedApplyChange> {
        self.planned_changes()
    }
}

fn strict_relative(relative: &Path) -> Result<PathBuf, ApplyStagingError> {
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(ApplyStagingError::new(
            ApplyStagingErrorKind::ContainmentIdentity,
            format!(
                "staged target must contain only normal relative components: {}",
                relative.display()
            ),
        ));
    }
    Ok(relative.to_path_buf())
}

#[cfg(feature = "test-support")]
#[path = "apply_budget_contract_tests.rs"]
pub mod budget_contract_tests;
#[cfg(any(test, feature = "test-support"))]
#[path = "../tests/support/apply.rs"]
mod test_support;

#[cfg(test)]
pub(crate) mod tests {
    use super::test_support::*;
    use super::{
        ApplyStagedState, ApplyStagingErrorKind, StagedChangeKind, StagedFileState,
        MAX_APPLY_FILE_BYTES,
    };
    use crate::platform::filesystem::{
        file_identity, inject_post_rename_sync_failure_for_test,
        set_before_identity_bound_directory_cleanup_mutation_hook,
        set_before_identity_bound_no_replace_rename_hook, RetainedChildCapability,
        RetainedDirectoryCapability,
    };
    use crate::platform::testing::{
        attempt_retained_directory_replacement_for_test, create_directory_link_fixture_for_test,
        path_identity_for_test, FileLinkFixtureOutcome, RetainedDirectoryReplacementOutcome,
    };
    use std::cell::Cell;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};
    use unica_work_control::cancellation::CancellationToken;
    use unica_work_control::provider_deadline::ProviderDeadline;

    thread_local! {
        static REFERENCE_SCAN_TEST_NOW: Cell<Instant> = Cell::new(Instant::now());
    }

    fn reference_scan_test_now() -> Instant {
        REFERENCE_SCAN_TEST_NOW.get()
    }

    fn cache_participant_authority(
        root: &Path,
        authority: crate::actor::ApplyWriterAuthority,
    ) -> crate::actor::WorkspaceCacheParticipantAuthority {
        let canonical = std::fs::canonicalize(root).unwrap();
        let retained = RetainedDirectoryCapability::open(&canonical).unwrap();
        crate::actor::workspace_cache_participant_authority_for_test(authority, &retained)
    }

    #[test]
    pub(crate) fn retained_transaction_roles_require_explicit_roots_and_cache_authority() {
        let root = temp_root("closed-participant-roots");
        let source = root.join("source");
        let cache = root.join("cache");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&cache).unwrap();
        let authority = crate::actor::apply_writer_authority_for_test();
        let cache_participant = cache_participant_authority(&cache, authority.clone());

        let closed = staged_with_authority(&source, authority.clone())
            .finalize()
            .unwrap()
            .close_with_workspace_cache_participant(
                staged_with_authority(&cache, authority.clone())
                    .finalize()
                    .unwrap(),
                &cache_participant,
            )
            .unwrap();

        assert_eq!(closed.retained_role_root_counts_for_test(), (1, 1));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    pub(crate) fn arbitrary_second_transaction_cannot_masquerade_as_actor_cache_authority() {
        let root = temp_root("closed-participant-authority");
        let source = root.join("source");
        let foreign = root.join("foreign");
        let cache = root.join("cache");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&foreign).unwrap();
        std::fs::create_dir_all(&cache).unwrap();
        let authority = crate::actor::apply_writer_authority_for_test();
        let source = staged_with_authority(&source, authority.clone())
            .finalize()
            .unwrap();
        let foreign = staged_with_authority(&foreign, authority.clone())
            .finalize()
            .unwrap();
        let cache_participant = cache_participant_authority(&cache, authority.clone());

        let error = source
            .close_with_workspace_cache_participant(foreign, &cache_participant)
            .unwrap_err();
        assert!(
            !error.contains(&cache.display().to_string()),
            "cache authority diagnostic exposed its absolute root: {error}"
        );

        let foreign_authority = crate::actor::apply_writer_authority_for_test();
        for (source_authority, cache_authority) in [
            (foreign_authority.clone(), authority.clone()),
            (authority.clone(), foreign_authority),
        ] {
            let source = staged_with_authority(&root.join("source"), source_authority)
                .finalize()
                .unwrap();
            let cache = staged_with_authority(&cache, cache_authority)
                .finalize()
                .unwrap();
            let error = source
                .close_with_workspace_cache_participant(cache, &cache_participant)
                .unwrap_err();
            assert!(error.contains("one actor authority"), "{error}");
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    pub(crate) fn closed_transaction_rejects_physical_alias_and_second_cache_participant() {
        let root = temp_root("closed-participant-cardinality");
        let source = root.join("source");
        let cache = root.join("cache");
        let other = root.join("other-cache");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        let authority = crate::actor::apply_writer_authority_for_test();
        let aliased_participant = cache_participant_authority(&source, authority.clone());

        let aliased_source = staged_with_authority(&source, authority.clone())
            .finalize()
            .unwrap();
        let aliased_cache = staged_with_authority(&source, authority.clone())
            .finalize()
            .unwrap();
        assert!(aliased_source
            .close_with_workspace_cache_participant(aliased_cache, &aliased_participant)
            .is_err());

        let cache_participant = cache_participant_authority(&cache, authority.clone());
        let closed = staged_with_authority(&source, authority.clone())
            .finalize()
            .unwrap()
            .close_with_workspace_cache_participant(
                staged_with_authority(&cache, authority.clone())
                    .finalize()
                    .unwrap(),
                &cache_participant,
            )
            .unwrap();
        let other_participant = cache_participant_authority(&other, authority.clone());
        assert!(closed
            .close_with_workspace_cache_participant(
                staged_with_authority(&other, authority.clone())
                    .finalize()
                    .unwrap(),
                &other_participant,
            )
            .is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn read_only_absence_below_missing_parent_is_guarded_without_creating_it() {
        for occupied in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path();
            std::fs::write(root.join("Module.bsl"), b"before").unwrap();
            let authority = crate::actor::apply_writer_authority_for_test();
            let mut state = staged_with_authority(root, authority.clone());
            assert!(state
                .read(Path::new("Ext/ParentConfigurations.bin"))
                .unwrap()
                .is_none());
            state
                .replace("Module.bsl", b"before", b"after".to_vec())
                .unwrap();
            let transaction = state.finalize().unwrap();
            if occupied {
                std::fs::create_dir(root.join("Ext")).unwrap();
                std::fs::write(root.join("Ext/ParentConfigurations.bin"), b"locked").unwrap();
            }
            let result = transaction.commit_retained_apply_with(authority, || Ok(()), || Ok(()));
            if occupied {
                assert!(result.is_err());
                assert_eq!(std::fs::read(root.join("Module.bsl")).unwrap(), b"before");
            } else {
                result.unwrap();
                assert!(!root.join("Ext").exists());
                assert_eq!(std::fs::read(root.join("Module.bsl")).unwrap(), b"after");
            }
        }
    }

    #[test]
    fn enumerated_payload_changes_refuse_before_write_and_roll_back_late_writes() {
        for relative in ["", "Payload"] {
            for late in [false, true] {
                let temp = tempfile::tempdir().unwrap();
                let root = temp.path();
                let directory = root.join(relative);
                std::fs::create_dir_all(&directory).unwrap();
                let module = Path::new(relative).join("Module.bsl");
                let original = root.join(&module);
                let foreign = directory.join("Added.bsl");
                std::fs::write(&original, b"original").unwrap();
                let authority = crate::actor::apply_writer_authority_for_test();
                let mut state = staged_with_authority(root, authority.clone());
                assert_eq!(
                    state.enumerate_tree(Path::new(relative)).unwrap(),
                    vec![module.clone()]
                );
                state.remove(&module, b"original").unwrap();
                let transaction = state.finalize().unwrap();
                if !late {
                    std::fs::write(&foreign, b"foreign").unwrap();
                }
                let mut injected = false;
                let result = transaction.commit_retained_apply_with(
                    authority,
                    || {
                        if late && !original.exists() && !injected {
                            std::fs::write(&foreign, b"foreign").unwrap();
                            injected = true;
                        }
                        Ok(())
                    },
                    || Ok(()),
                );
                let error = result.expect_err("namespace race returned success");
                let input = if relative.is_empty() { "." } else { relative };
                assert!(error.contains(&format!("input `{input}`")), "{error}");
                assert!(error.contains("directory membership input"), "{error}");
                assert!(
                    error.contains("the plan enumerated this directory"),
                    "{error}"
                );
                assert!(!error.contains(&root.display().to_string()), "{error}");
                assert_eq!(std::fs::read(&original).unwrap(), b"original");
                assert_eq!(std::fs::read(&foreign).unwrap(), b"foreign");
                assert_eq!(injected, late);
            }
        }
    }

    #[test]
    fn enumerated_directory_identity_conflict_names_its_dependency_before_writing() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        std::fs::create_dir(root.join("Payload")).unwrap();
        std::fs::write(root.join("Payload/Input.xml"), b"observed").unwrap();
        std::fs::write(root.join("Module.bsl"), b"before").unwrap();
        let mut state = staged(root);
        state.enumerate_tree(Path::new("Payload")).unwrap();
        state
            .replace("Module.bsl", b"before", b"after".to_vec())
            .unwrap();
        let transaction = state.finalize().unwrap();
        std::fs::rename(root.join("Payload"), root.join("RetainedPayload")).unwrap();
        std::fs::create_dir(root.join("Payload")).unwrap();
        std::fs::write(root.join("Payload/Input.xml"), b"foreign").unwrap();
        let error = transaction.validate_retained_for_apply_typed().unwrap_err();
        assert_eq!(
            error.kind(),
            super::super::compile_transaction::RetainedApplyValidationErrorKind::ConcurrentRevision
        );
        let message = error.to_string();
        assert!(message.contains("input `Payload`"), "{message}");
        assert!(message.contains("directory membership input"), "{message}");
        assert!(!message.contains(&root.display().to_string()), "{message}");
        assert_eq!(std::fs::read(root.join("Module.bsl")).unwrap(), b"before");
        assert_eq!(
            std::fs::read(root.join("Payload/Input.xml")).unwrap(),
            b"foreign"
        );
        assert_eq!(
            std::fs::read(root.join("RetainedPayload/Input.xml")).unwrap(),
            b"observed"
        );
    }

    #[test]
    fn reference_scan_entry_budget_stops_incrementally_at_a_test_limit() {
        let root = temp_root("namespace-entry-budget");
        std::fs::create_dir_all(&root).unwrap();
        for name in ["A.xml", "B.xml", "C.xml"] {
            std::fs::write(root.join(name), b"<Root/>").unwrap();
        }
        let mut state = staged(&root);
        let error = state
            .enumerate_tree_with_limits(Path::new(""), 4, 1)
            .unwrap_err();
        assert!(error.to_string().contains("entry limit"), "{error}");
        assert!(
            state.namespaces.is_empty(),
            "over-budget enumeration must not admit the directory"
        );
        assert!(
            state.entries.is_empty(),
            "no file bytes may be read past the enumeration budget"
        );
    }

    #[test]
    fn reference_scan_depth_budget_stops_before_recursive_descent() {
        let root = temp_root("namespace-depth-budget");
        std::fs::create_dir_all(root.join("Level1/Level2")).unwrap();
        std::fs::write(root.join("Level1/Level2/deep.xml"), b"<Root/>").unwrap();
        let mut state = staged(&root);
        let error = state
            .enumerate_tree_with_limits(Path::new(""), 1, 8)
            .unwrap_err();
        assert!(error.to_string().contains("maximum depth"), "{error}");
        assert!(!state.namespaces.contains_key(Path::new("Level1/Level2")));
        assert!(state.entries.is_empty());
    }

    #[test]
    fn reference_scan_rejects_a_direct_symlink_before_any_recursive_descent() {
        let root = temp_root("namespace-symlink-before-recursion");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir(root.join("A-directory")).unwrap();
        crate::platform::filesystem::create_test_directory_link(
            &root.join("external"),
            &root.join("Z-symlink-directory"),
        )
        .unwrap();
        let error = staged(&root)
            .enumerate_tree_with_limits(Path::new(""), 0, 8)
            .unwrap_err();
        assert!(error.to_string().contains("link"), "{error}");
        assert!(!error.to_string().contains("maximum depth"), "{error}");
    }

    #[test]
    fn reference_scan_keeps_content_guard_without_retaining_every_body() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        for name in ["A.xml", "B.xml", "C.bsl"] {
            std::fs::write(root.join(name), b"<Reference/>\n").unwrap();
        }
        let mut state = staged(root);
        for name in ["A.xml", "B.xml", "C.bsl"] {
            assert_eq!(
                state.read_guarded_bounded(Path::new(name), 1024).unwrap(),
                Some(b"<Reference/>\n".to_vec())
            );
        }
        assert!(
            state.entries.is_empty(),
            "read-only bodies must not be retained"
        );
        assert_eq!(state.read_guards.len(), 3);

        // A later edit of an already scanned file takes ownership of its
        // preimage. Its old read guard must not reject the plan's own write.
        state
            .replace("B.xml", b"<Reference/>\n", b"<Changed/>\n".to_vec())
            .unwrap();
        assert_eq!(state.entries.len(), 1);
        assert_eq!(state.read_guards.len(), 2);
    }

    #[test]
    fn guarded_read_keeps_absence_and_guard_only_plan_closed_to_generic_commit() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        std::fs::write(root.join("Reference.bsl"), b"before").unwrap();
        let mut state = staged(root);
        assert_eq!(
            state
                .read_guarded_bounded(Path::new("Missing/Absent.xml"), 16)
                .unwrap(),
            None
        );
        assert_eq!(
            state.entries.len(),
            1,
            "an absent input still needs its read guard"
        );
        let mut guarded = staged(root);
        assert_eq!(
            guarded
                .read_guarded_bounded(Path::new("Reference.bsl"), 16)
                .unwrap(),
            Some(b"before".to_vec())
        );
        assert!(guarded.entries.is_empty());
        let transaction = guarded.finalize().unwrap();
        assert!(!transaction.is_empty());
        assert!(
            transaction.commit().is_err(),
            "retained plan cannot use generic commit"
        );
    }

    #[test]
    fn reference_guard_rejects_same_inode_same_size_mutation_without_writing() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        std::fs::write(root.join("Reference.bsl"), b"abcdef").unwrap();
        std::fs::write(root.join("Owner.bsl"), b"before").unwrap();
        let authority = crate::actor::apply_writer_authority_for_test();
        let mut state = staged_with_authority(root, authority.clone());
        assert_eq!(
            state
                .read_guarded_bounded(Path::new("Reference.bsl"), 16)
                .unwrap(),
            Some(b"abcdef".to_vec())
        );
        state
            .replace("Owner.bsl", b"before", b"after!".to_vec())
            .unwrap();
        let transaction = state.finalize().unwrap();
        std::fs::write(root.join("Reference.bsl"), b"abcdeg").unwrap();
        let result = transaction.commit_retained_apply_with(authority, || Ok(()), || Ok(()));
        assert!(
            result.is_err(),
            "same-size content edit must stale the plan"
        );
        assert_eq!(std::fs::read(root.join("Owner.bsl")).unwrap(), b"before");
        assert_eq!(
            std::fs::read(root.join("Reference.bsl")).unwrap(),
            b"abcdeg"
        );
    }

    /// A reference scan over a whole configuration (issue #1466) must not
    /// keep a descriptor per inspected file or directory: the saved guard is
    /// an identity and a fingerprint, re-proved by walking the route again.
    /// This resource invariant has no operation cutoff; the finite-deadline
    /// refusal is checked separately below.
    #[test]
    fn reference_scan_descriptor_count_does_not_grow_with_inputs() {
        use crate::platform::testing::open_handle_count_for_test;
        const DIRECTORIES: usize = 32;
        const FILES_PER_DIRECTORY: usize = 16;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let mut inputs = Vec::new();
        for directory in 0..DIRECTORIES {
            let parent = Path::new("Catalogs")
                .join(format!("Item{directory}"))
                .join("Ext");
            std::fs::create_dir_all(root.join(&parent)).unwrap();
            for file in 0..FILES_PER_DIRECTORY {
                let relative = parent.join(format!("Module{file}.bsl"));
                std::fs::write(root.join(&relative), b"Procedure A()\nEndProcedure\n").unwrap();
                inputs.push(relative);
            }
        }
        std::fs::write(root.join("Owner.bsl"), b"before").unwrap();
        let authority = crate::actor::apply_writer_authority_for_test();
        let mut state = staged_with_authority_and_deadline(
            root,
            authority.clone(),
            ProviderDeadline::no_deadline(),
        );
        let before = open_handle_count_for_test().unwrap();
        for relative in &inputs {
            assert!(state
                .read_guarded_bounded(relative, 1024)
                .unwrap()
                .is_some());
        }
        assert_eq!(state.read_guards.len(), inputs.len());
        let after_scan = open_handle_count_for_test().unwrap();
        state
            .replace("Owner.bsl", b"before", b"after!".to_vec())
            .unwrap();
        let transaction = state.finalize().unwrap();
        let after_finalize = open_handle_count_for_test().unwrap();
        // The old guard kept the file and every directory of its route open:
        // more than three descriptors per input here. A fixed allowance
        // absorbs descriptors of concurrently running tests.
        let allowance = 32;
        assert!(
            after_scan.saturating_sub(before) < allowance,
            "scan of {} inputs kept {} descriptors open",
            inputs.len(),
            after_scan.saturating_sub(before)
        );
        assert!(
            after_finalize.saturating_sub(before) < allowance,
            "saved plan of {} inputs keeps {} descriptors open",
            inputs.len(),
            after_finalize.saturating_sub(before)
        );
        transaction
            .commit_retained_apply_with(authority, || Ok(()), || Ok(()))
            .unwrap();
        assert_eq!(std::fs::read(root.join("Owner.bsl")).unwrap(), b"after!");
    }

    #[test]
    fn reference_guard_preserves_finite_deadline_after_capture_without_writing() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        std::fs::write(root.join("Reference.bsl"), b"abcdef").unwrap();
        std::fs::write(root.join("Owner.bsl"), b"before").unwrap();
        let authority = crate::actor::apply_writer_authority_for_test();
        let started = Instant::now();
        REFERENCE_SCAN_TEST_NOW.set(started);
        let mut state = staged_with_authority_and_deadline(
            root,
            authority.clone(),
            ProviderDeadline::with_clock(started + Duration::from_secs(5), reference_scan_test_now),
        );
        assert_eq!(
            state
                .read_guarded_bounded(Path::new("Reference.bsl"), 16)
                .unwrap(),
            Some(b"abcdef".to_vec())
        );
        state
            .replace("Owner.bsl", b"before", b"after!".to_vec())
            .unwrap();
        let transaction = state.finalize().unwrap();
        REFERENCE_SCAN_TEST_NOW.set(started + Duration::from_secs(6));
        assert_eq!(
            transaction
                .validate_retained_for_apply_typed()
                .unwrap_err()
                .kind(),
            super::RetainedApplyValidationErrorKind::Deadline
        );
        assert!(transaction
            .commit_retained_apply_with(authority, || Ok(()), || Ok(()))
            .is_err());
        assert_eq!(std::fs::read(root.join("Owner.bsl")).unwrap(), b"before");
        assert_eq!(
            std::fs::read(root.join("Reference.bsl")).unwrap(),
            b"abcdef"
        );
    }

    /// Without a held descriptor the saved plan still refuses a nested input
    /// that changed in place, was replaced by another file or lost its parent
    /// directory to another directory with the same bytes.
    #[test]
    fn reference_guard_without_descriptors_rejects_nested_input_substitution() {
        enum Substitution {
            InPlaceEdit,
            FileReplacedByRename,
            ParentDirectoryReplaced,
        }
        for substitution in [
            Substitution::InPlaceEdit,
            Substitution::FileReplacedByRename,
            Substitution::ParentDirectoryReplaced,
        ] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path();
            std::fs::create_dir_all(root.join("Catalogs/Item/Ext")).unwrap();
            std::fs::write(root.join("Catalogs/Item/Ext/Module.bsl"), b"abcdef").unwrap();
            std::fs::write(root.join("Owner.bsl"), b"before").unwrap();
            let authority = crate::actor::apply_writer_authority_for_test();
            let mut state = staged_with_authority(root, authority.clone());
            assert_eq!(
                state
                    .read_guarded_bounded(Path::new("Catalogs/Item/Ext/Module.bsl"), 16)
                    .unwrap(),
                Some(b"abcdef".to_vec())
            );
            state
                .replace("Owner.bsl", b"before", b"after!".to_vec())
                .unwrap();
            let transaction = state.finalize().unwrap();
            let expected = match substitution {
                Substitution::InPlaceEdit => {
                    std::fs::write(root.join("Catalogs/Item/Ext/Module.bsl"), b"abcdeg").unwrap();
                    b"abcdeg".as_slice()
                }
                Substitution::FileReplacedByRename => {
                    std::fs::write(root.join("Catalogs/Item/Ext/Next.bsl"), b"abcdef").unwrap();
                    std::fs::rename(
                        root.join("Catalogs/Item/Ext/Next.bsl"),
                        root.join("Catalogs/Item/Ext/Module.bsl"),
                    )
                    .unwrap();
                    b"abcdef".as_slice()
                }
                Substitution::ParentDirectoryReplaced => {
                    std::fs::rename(root.join("Catalogs/Item"), root.join("Catalogs/Moved"))
                        .unwrap();
                    std::fs::create_dir_all(root.join("Catalogs/Item/Ext")).unwrap();
                    std::fs::write(root.join("Catalogs/Item/Ext/Module.bsl"), b"abcdef").unwrap();
                    b"abcdef".as_slice()
                }
            };
            let error = transaction
                .commit_retained_apply_with(authority, || Ok(()), || Ok(()))
                .expect_err("substituted reference input must stale the plan");
            assert!(
                error.contains("reference input"),
                "the refusal names the substituted reference input: {error}"
            );
            assert_eq!(std::fs::read(root.join("Owner.bsl")).unwrap(), b"before");
            assert_eq!(
                std::fs::read(root.join("Catalogs/Item/Ext/Module.bsl")).unwrap(),
                expected
            );
        }
    }

    #[test]
    fn reference_guard_rejects_late_change_and_rolls_back_own_write() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        std::fs::write(root.join("Reference.bsl"), b"abcdef").unwrap();
        std::fs::write(root.join("Owner.bsl"), b"before").unwrap();
        let authority = crate::actor::apply_writer_authority_for_test();
        let mut state = staged_with_authority(root, authority.clone());
        state
            .read_guarded_bounded(Path::new("Reference.bsl"), 16)
            .unwrap();
        state
            .replace("Owner.bsl", b"before", b"after!".to_vec())
            .unwrap();
        let transaction = state.finalize().unwrap();
        let reference = root.join("Reference.bsl");
        let owner = root.join("Owner.bsl");
        crate::compile_transaction::set_retained_apply_before_post_validation_hook(move || {
            assert_eq!(std::fs::read(&owner).unwrap(), b"after!");
            std::fs::write(&reference, b"abcdeg").unwrap();
        });
        let result = transaction.commit_retained_apply_with(authority, || Ok(()), || Ok(()));
        assert!(result.is_err(), "late reference edit must fail publication");
        assert_eq!(std::fs::read(root.join("Owner.bsl")).unwrap(), b"before");
        assert_eq!(
            std::fs::read(root.join("Reference.bsl")).unwrap(),
            b"abcdeg"
        );
    }

    #[test]
    fn scanned_file_promoted_to_writer_accepts_its_own_postimage() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        std::fs::write(root.join("Reference.bsl"), b"before").unwrap();
        let authority = crate::actor::apply_writer_authority_for_test();
        let mut state = staged_with_authority(root, authority.clone());
        state
            .read_guarded_bounded(Path::new("Reference.bsl"), 16)
            .unwrap();
        state
            .replace("Reference.bsl", b"before", b"after!".to_vec())
            .unwrap();
        assert!(state.read_guards.is_empty());
        let transaction = state.finalize().unwrap();
        transaction
            .commit_retained_apply_with(authority, || Ok(()), || Ok(()))
            .unwrap();
        assert_eq!(
            std::fs::read(root.join("Reference.bsl")).unwrap(),
            b"after!"
        );
    }

    #[test]
    fn reference_guard_rejects_same_bytes_at_replaced_file_identity() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        std::fs::write(root.join("Reference.bsl"), b"abcdef").unwrap();
        std::fs::write(root.join("Owner.bsl"), b"before").unwrap();
        let authority = crate::actor::apply_writer_authority_for_test();
        let mut state = staged_with_authority(root, authority.clone());
        state
            .read_guarded_bounded(Path::new("Reference.bsl"), 16)
            .unwrap();
        state
            .replace("Owner.bsl", b"before", b"after!".to_vec())
            .unwrap();
        let transaction = state.finalize().unwrap();
        std::fs::rename(root.join("Reference.bsl"), root.join("Displaced.bsl")).unwrap();
        std::fs::write(root.join("Reference.bsl"), b"abcdef").unwrap();
        assert!(transaction
            .commit_retained_apply_with(authority, || Ok(()), || Ok(()))
            .is_err());
        assert_eq!(std::fs::read(root.join("Owner.bsl")).unwrap(), b"before");
    }

    #[test]
    fn reference_guard_rechecks_identity_after_stream_and_rolls_back() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        std::fs::write(root.join("Reference.bsl"), b"abcdef").unwrap();
        std::fs::write(root.join("Owner.bsl"), b"before").unwrap();
        let authority = crate::actor::apply_writer_authority_for_test();
        let mut state = staged_with_authority(root, authority.clone());
        state
            .read_guarded_bounded(Path::new("Reference.bsl"), 16)
            .unwrap();
        state
            .replace("Owner.bsl", b"before", b"after!".to_vec())
            .unwrap();
        let transaction = state.finalize().unwrap();
        let reference = root.join("Reference.bsl");
        let displaced = root.join("Displaced.bsl");
        let owner = root.join("Owner.bsl");
        crate::compile_transaction::set_retained_apply_after_reference_stream_hook(1, move || {
            assert_eq!(std::fs::read(&owner).unwrap(), b"after!");
            std::fs::rename(&reference, &displaced).unwrap();
            std::fs::write(&reference, b"abcdef").unwrap();
        });
        let result = transaction.commit_retained_apply_with(authority, || Ok(()), || Ok(()));
        assert!(
            result.is_err(),
            "the old file descriptor must not certify a replaced name"
        );
        assert_eq!(std::fs::read(root.join("Owner.bsl")).unwrap(), b"before");
        assert_eq!(
            std::fs::read(root.join("Reference.bsl")).unwrap(),
            b"abcdef"
        );
    }

    #[test]
    fn reference_guard_uses_execution_cancellation_context() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        std::fs::write(root.join("Reference.bsl"), b"abcdef").unwrap();
        std::fs::write(root.join("Owner.bsl"), b"before").unwrap();
        let authority = crate::actor::apply_writer_authority_for_test();
        let old_cancellation = CancellationToken::new();
        let canonical = std::fs::canonicalize(root).unwrap();
        let retained = Arc::new(RetainedDirectoryCapability::open(&canonical).unwrap());
        let mut state = ApplyStagedState::from_retained_root(
            retained,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            old_cancellation.clone(),
            authority.clone(),
        );
        state
            .read_guarded_bounded(Path::new("Reference.bsl"), 16)
            .unwrap();
        state
            .replace("Owner.bsl", b"before", b"after!".to_vec())
            .unwrap();
        let mut transaction = state.finalize().unwrap();
        old_cancellation.cancel();
        assert_eq!(
            transaction
                .validate_retained_for_apply_typed()
                .unwrap_err()
                .kind(),
            super::RetainedApplyValidationErrorKind::Cancelled
        );
        let execution_cancellation = CancellationToken::new();
        transaction.rebind_retained_apply_execution_context(
            ProviderDeadline::from_budget(Duration::ZERO),
            &execution_cancellation,
        );
        assert_eq!(
            transaction
                .validate_retained_for_apply_typed()
                .unwrap_err()
                .kind(),
            super::RetainedApplyValidationErrorKind::Deadline
        );
        transaction.rebind_retained_apply_execution_context(
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &execution_cancellation,
        );
        transaction
            .commit_retained_apply_with(authority, || Ok(()), || Ok(()))
            .unwrap();
        assert_eq!(std::fs::read(root.join("Owner.bsl")).unwrap(), b"after!");
    }

    #[test]
    fn write_target_recheck_uses_execution_cancellation_context() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let original = vec![b'x'; 128 * 1024];
        std::fs::write(root.join("Owner.bsl"), &original).unwrap();
        let authority = crate::actor::apply_writer_authority_for_test();
        let old_cancellation = CancellationToken::new();
        let canonical = std::fs::canonicalize(root).unwrap();
        let retained = Arc::new(RetainedDirectoryCapability::open(&canonical).unwrap());
        let mut state = ApplyStagedState::from_retained_root(
            retained,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            old_cancellation.clone(),
            authority.clone(),
        );
        state
            .replace("Owner.bsl", &original, b"after".to_vec())
            .unwrap();
        let mut transaction = state.finalize().unwrap();
        old_cancellation.cancel();
        assert_eq!(
            transaction
                .validate_retained_for_apply_typed()
                .unwrap_err()
                .kind(),
            super::RetainedApplyValidationErrorKind::Cancelled
        );
        let execution_cancellation = CancellationToken::new();
        transaction.rebind_retained_apply_execution_context(
            ProviderDeadline::from_budget(Duration::ZERO),
            &execution_cancellation,
        );
        assert_eq!(
            transaction
                .validate_retained_for_apply_typed()
                .unwrap_err()
                .kind(),
            super::RetainedApplyValidationErrorKind::Deadline
        );
        transaction.rebind_retained_apply_execution_context(
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &execution_cancellation,
        );
        transaction
            .commit_retained_apply_with(authority, || Ok(()), || Ok(()))
            .unwrap();
        assert_eq!(std::fs::read(root.join("Owner.bsl")).unwrap(), b"after");
    }

    #[test]
    fn write_target_recheck_stops_between_hash_blocks_before_any_write() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let original = vec![b'x'; 128 * 1024];
        std::fs::write(root.join("Owner.bsl"), &original).unwrap();
        let authority = crate::actor::apply_writer_authority_for_test();
        let cancellation = CancellationToken::new();
        let mut state = ApplyStagedState::from_retained_root(
            Arc::new(
                RetainedDirectoryCapability::open(&std::fs::canonicalize(root).unwrap()).unwrap(),
            ),
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            cancellation.clone(),
            authority,
        );
        state
            .replace("Owner.bsl", &original, b"after".to_vec())
            .unwrap();
        let transaction = state.finalize().unwrap();
        let stop = cancellation.clone();
        crate::compile_transaction::set_retained_apply_after_write_target_chunk_hook(move || {
            stop.cancel()
        });

        assert_eq!(
            transaction
                .validate_retained_for_apply_typed()
                .unwrap_err()
                .kind(),
            super::RetainedApplyValidationErrorKind::Cancelled
        );
        assert_eq!(std::fs::read(root.join("Owner.bsl")).unwrap(), original);
    }

    #[test]
    fn reference_scan_refuses_a_path_outside_the_source_root() {
        let root = temp_root("reference-root-bound");
        std::fs::create_dir_all(&root).unwrap();
        let outside = temp_root("reference-outside").join("outside.xml");
        std::fs::create_dir_all(outside.parent().unwrap()).unwrap();
        std::fs::write(&outside, b"<Secret/>").unwrap();
        let mut state = staged(&root);
        assert!(state.read(&outside).is_err());
        assert!(state.read(Path::new("../outside.xml")).is_err());
        assert!(state.entries.is_empty());
        assert_eq!(std::fs::read(outside).unwrap(), b"<Secret/>");
    }

    #[test]
    fn namespace_inputs_bind_the_plan_and_accept_only_owned_removals() {
        use sha2::Digest;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        std::fs::create_dir(root.join("Payload")).unwrap();
        std::fs::write(root.join("Payload/Module.bsl"), b"original").unwrap();
        let authority = crate::actor::apply_writer_authority_for_test();
        let mut state = staged_with_authority(root, authority.clone());
        state.enumerate_tree(Path::new("Payload")).unwrap();
        state.remove("Payload/Module.bsl", b"original").unwrap();
        let mut first = sha2::Sha256::new();
        state.fingerprint_inputs(&mut first).unwrap();
        std::fs::create_dir(root.join("Payload/NewEmptyDirectory")).unwrap();
        let mut changed = staged_with_authority(root, authority.clone());
        changed.enumerate_tree(Path::new("Payload")).unwrap();
        changed.remove("Payload/Module.bsl", b"original").unwrap();
        let mut second = sha2::Sha256::new();
        changed.fingerprint_inputs(&mut second).unwrap();
        assert_ne!(first.finalize(), second.finalize());
        changed
            .finalize()
            .unwrap()
            .commit_retained_apply_with(authority, || Ok(()), || Ok(()))
            .unwrap();
        assert!(!root.join("Payload/Module.bsl").exists());
        assert!(root.join("Payload/NewEmptyDirectory").is_dir());
    }

    #[test]
    fn apply_staged_state_composes_ordered_same_file_postimages() {
        let root = temp_root("composition");
        std::fs::create_dir_all(root.join("Ext")).unwrap();
        std::fs::write(root.join("Ext/existing.txt"), b"original").unwrap();
        let mut state = staged(&root);

        assert_eq!(state.read(Path::new("Ext/new.txt")).unwrap(), None);
        state.create("Ext/new.txt", b"created".to_vec()).unwrap();
        assert_eq!(
            state.read(Path::new("Ext/new.txt")).unwrap(),
            Some(b"created".to_vec())
        );
        state
            .replace("Ext/new.txt", b"created", b"created-then-replaced".to_vec())
            .unwrap();
        assert_eq!(
            state.read(Path::new("Ext/new.txt")).unwrap(),
            Some(b"created-then-replaced".to_vec())
        );

        state
            .replace("Ext/existing.txt", b"original", b"replacement-1".to_vec())
            .unwrap();
        state
            .replace(
                "Ext/existing.txt",
                b"replacement-1",
                b"replacement-2".to_vec(),
            )
            .unwrap();
        assert_eq!(
            state.read(Path::new("Ext/existing.txt")).unwrap(),
            Some(b"replacement-2".to_vec())
        );

        let changes = state.planned_changes();
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0].relative_path, PathBuf::from("Ext/existing.txt"));
        assert_eq!(changes[0].kind, StagedChangeKind::Replace);
        assert_eq!(
            changes[0].original,
            StagedFileState::Bytes(b"original".to_vec())
        );
        assert_eq!(
            changes[0].current,
            StagedFileState::Bytes(b"replacement-2".to_vec())
        );
        assert_eq!(changes[1].relative_path, PathBuf::from("Ext/new.txt"));
        assert_eq!(changes[1].kind, StagedChangeKind::Create);

        let transaction = state.finalize().unwrap();
        assert_eq!(transaction.retained_planned_change_count_for_test(), 2);
        assert_eq!(
            std::fs::read(root.join("Ext/existing.txt")).unwrap(),
            b"original"
        );
        assert!(!root.join("Ext/new.txt").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn apply_staged_state_collapses_create_remove_and_replace_remove_to_final_state() {
        let root = temp_root("removals");
        std::fs::create_dir_all(root.join("Ext")).unwrap();
        std::fs::write(root.join("Ext/existing.txt"), b"original").unwrap();
        let mut state = staged(&root);

        state
            .create("Ext/transient.txt", b"temporary".to_vec())
            .unwrap();
        state.remove("Ext/transient.txt", b"temporary").unwrap();
        state
            .replace("Ext/existing.txt", b"original", b"replacement".to_vec())
            .unwrap();
        state.remove("Ext/existing.txt", b"replacement").unwrap();

        let changes = state.planned_changes();
        assert_eq!(
            changes.len(),
            1,
            "create->remove must collapse to no physical change"
        );
        assert_eq!(changes[0].kind, StagedChangeKind::Remove);
        assert_eq!(
            changes[0].original,
            StagedFileState::Bytes(b"original".to_vec())
        );
        assert_eq!(changes[0].current, StagedFileState::Absent);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn apply_staged_state_allows_remove_then_create_from_the_staged_postimage() {
        let root = temp_root("remove-create");
        std::fs::create_dir_all(root.join("Ext")).unwrap();
        std::fs::write(root.join("Ext/existing.txt"), b"original").unwrap();
        let mut state = staged(&root);

        state.remove("Ext/existing.txt", b"original").unwrap();
        state
            .create("Ext/existing.txt", b"recreated".to_vec())
            .unwrap();

        let changes = state.planned_changes();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].kind, StagedChangeKind::Replace);
        assert_eq!(
            changes[0].original,
            StagedFileState::Bytes(b"original".to_vec())
        );
        assert_eq!(
            changes[0].current,
            StagedFileState::Bytes(b"recreated".to_vec())
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn apply_staged_state_rejects_duplicate_create_escape_links_and_hard_link_aliases() {
        let root = temp_root("invalid-targets");
        std::fs::create_dir_all(root.join("Ext/real")).unwrap();
        std::fs::write(root.join("Ext/real/a.txt"), b"same inode").unwrap();
        let mut state = staged(&root);
        state.create("Ext/new.txt", b"one".to_vec()).unwrap();
        assert!(state
            .create("Ext/new.txt", b"two".to_vec())
            .unwrap_err()
            .contains("create"));
        assert!(state
            .read(Path::new("../escape.txt"))
            .unwrap_err()
            .contains("normal"));

        let link_outcome =
            create_directory_link_fixture_for_test(root.join("Ext/real"), root.join("Ext/link"))
                .unwrap();
        if link_outcome == FileLinkFixtureOutcome::Created {
            assert!(state
                .read(Path::new("Ext/link/a.txt"))
                .unwrap_err()
                .contains("link"));
            std::fs::create_dir_all(root.join("Race")).unwrap();
            std::fs::write(root.join("Race/Module.bsl"), b"original race bytes").unwrap();
            let external = temp_root("nested-link-external");
            std::fs::create_dir_all(&external).unwrap();
            std::fs::write(external.join("Module.bsl"), b"external decoy").unwrap();
            let mut raced = staged(&root);
            assert_eq!(
                raced.read(Path::new("Race/Module.bsl")).unwrap(),
                Some(b"original race bytes".to_vec())
            );
            let displaced = root.join("Race-displaced");
            let race_root = root.join("Race");
            let retained_identity = path_identity_for_test(&race_root)
                .unwrap()
                .expect("race root identity must be available on supported CI platforms");
            let replacement =
                attempt_retained_directory_replacement_for_test(&race_root, &displaced).unwrap();
            match replacement {
                RetainedDirectoryReplacementOutcome::Replaced => {
                    assert_eq!(
                        create_directory_link_fixture_for_test(&external, &race_root).unwrap(),
                        FileLinkFixtureOutcome::Created
                    );
                    raced
                        .replace(
                            "Race/Module.bsl",
                            b"original race bytes",
                            b"must not redirect".to_vec(),
                        )
                        .unwrap();
                    let error = raced.finalize().unwrap_err();
                    assert!(error.contains("link/reparse"), "{error:?}");
                    assert_eq!(
                        std::fs::read(displaced.join("Module.bsl")).unwrap(),
                        b"original race bytes"
                    );
                }
                RetainedDirectoryReplacementOutcome::PreventedByRetainedHandle => {
                    assert_eq!(
                        path_identity_for_test(&race_root).unwrap().as_deref(),
                        Some(retained_identity.as_str())
                    );
                    assert!(!displaced.exists());
                    raced
                        .replace(
                            "Race/Module.bsl",
                            b"original race bytes",
                            b"must not redirect".to_vec(),
                        )
                        .unwrap();
                    let changes = raced.planned_changes();
                    assert_eq!(changes.len(), 1);
                    assert_eq!(changes[0].relative_path, PathBuf::from("Race/Module.bsl"));
                    assert_eq!(changes[0].kind, StagedChangeKind::Replace);
                    assert_eq!(
                        changes[0].original,
                        StagedFileState::Bytes(b"original race bytes".to_vec())
                    );
                    assert_eq!(
                        changes[0].current,
                        StagedFileState::Bytes(b"must not redirect".to_vec())
                    );
                    raced.finalize().unwrap();
                    assert_eq!(
                        std::fs::read(race_root.join("Module.bsl")).unwrap(),
                        b"original race bytes"
                    );
                }
            }
            assert_eq!(
                std::fs::read(external.join("Module.bsl")).unwrap(),
                b"external decoy"
            );
            std::fs::remove_dir_all(external).unwrap();
        }
        std::fs::hard_link(root.join("Ext/real/a.txt"), root.join("Ext/real/b.txt")).unwrap();
        assert!(state
            .read(Path::new("Ext/real/a.txt"))
            .unwrap_err()
            .contains("hard-link"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn retained_finalize_validation_failure_changes_nothing() {
        let root = temp_root("precommit-validation");
        std::fs::create_dir_all(root.join("Ext")).unwrap();
        let target = root.join("Ext/Form.xml");
        std::fs::write(&target, b"<Form/>").unwrap();
        let mut state = staged(&root);
        state
            .replace("Ext/Form.xml", b"<Form/>", b"<broken".to_vec())
            .unwrap();
        assert!(state.finalize().unwrap_err().contains("XML"));
        assert_eq!(std::fs::read(&target).unwrap(), b"<Form/>");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn retained_apply_plan_is_not_empty_and_generic_commit_refuses_actor_bypass() {
        let root = temp_root("actor-bypass");
        std::fs::create_dir_all(root.join("Ext")).unwrap();
        std::fs::write(root.join("Ext/Module.bsl"), b"original").unwrap();
        let mut state = staged(&root);
        state
            .replace("Ext/Module.bsl", b"original", b"bypass".to_vec())
            .unwrap();
        let transaction = state.finalize().unwrap();

        assert!(
            !transaction.is_empty(),
            "retained apply entries were ignored"
        );
        let error = transaction.commit().unwrap_err();
        assert!(error.contains("actor"), "{error}");
        assert_eq!(
            std::fs::read(root.join("Ext/Module.bsl")).unwrap(),
            b"original"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn retained_create_stage_source_swap_leaves_no_destination_mutation_and_loses_neither_file() {
        if !crate::platform::testing::can_swap_named_child_behind_retained_handle_for_test() {
            return;
        }
        let root = temp_root("stage-source-swap");
        let parent = root.join("Ext");
        std::fs::create_dir_all(&parent).unwrap();
        let target = parent.join("new.txt");
        let owned_stage = parent.join("owned-stage.txt");
        let mut state = staged(&root);
        state
            .create("Ext/new.txt", b"apply-stage".to_vec())
            .unwrap();
        let authority = state.writer_authority.clone();
        let transaction = state.finalize().unwrap();
        let hook_parent = parent.clone();
        let hook_owned = owned_stage.clone();
        crate::platform::filesystem::set_before_identity_bound_no_replace_rename_hook(move || {
            let stage = apply_artifact(&hook_parent);
            std::fs::rename(&stage, &hook_owned).unwrap();
            std::fs::write(&stage, b"concurrent-stage").unwrap();
        });

        let error = transaction
            .commit_retained_apply_with(authority, || Ok(()), || Ok(()))
            .unwrap_err();

        assert!(
            error.contains("identity") || error.contains("source"),
            "{error}"
        );
        assert!(
            !target.exists(),
            "reported failure left a destination mutation"
        );
        assert_eq!(std::fs::read(&owned_stage).unwrap(), b"apply-stage");
        assert!(apply_artifact_contents(&parent)
            .iter()
            .any(|bytes| bytes == b"concurrent-stage"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn retained_rollback_restore_recovery_swap_never_reports_a_different_inode_restored() {
        if !crate::platform::testing::can_swap_named_child_behind_retained_handle_for_test() {
            return;
        }
        let root = temp_root("restore-recovery-swap");
        let parent = root.join("Ext");
        std::fs::create_dir_all(&parent).unwrap();
        let target = parent.join("Module.bsl");
        let owned_recovery = parent.join("owned-recovery.bsl");
        std::fs::write(&target, b"original").unwrap();
        let mut state = staged(&root);
        state
            .replace("Ext/Module.bsl", b"original", b"apply-bytes".to_vec())
            .unwrap();
        let authority = state.writer_authority.clone();
        let transaction = state.finalize().unwrap();
        let hook_parent = parent.clone();
        let hook_owned = owned_recovery.clone();
        crate::compile_transaction::set_retained_apply_before_post_validation_hook(move || {
            crate::platform::filesystem::set_before_identity_bound_no_replace_rename_hook(
                move || {
                    let recovery = apply_artifact(&hook_parent);
                    std::fs::rename(&recovery, &hook_owned).unwrap();
                    std::fs::write(&recovery, b"concurrent-recovery").unwrap();
                },
            );
        });

        let error = transaction
            .commit_retained_apply_with(
                authority,
                || Ok(()),
                || Err::<(), _>("post validation failure".to_string()),
            )
            .unwrap_err();

        assert!(error.contains("rollback"), "{error}");
        assert!(!target.exists(), "a different recovery inode was restored");
        assert_eq!(std::fs::read(&owned_recovery).unwrap(), b"original");
        assert!(apply_artifact_contents(&parent)
            .iter()
            .any(|bytes| bytes == b"concurrent-recovery"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn retained_rollback_published_cleanup_never_unlinks_a_concurrent_child() {
        if !crate::platform::testing::can_swap_named_child_behind_retained_handle_for_test() {
            return;
        }
        let root = temp_root("published-cleanup-swap");
        let parent = root.join("Ext");
        std::fs::create_dir_all(&parent).unwrap();
        let target = parent.join("new.txt");
        let owned_published = parent.join("owned-published.txt");
        let mut state = staged(&root);
        state
            .create("Ext/new.txt", b"apply-bytes".to_vec())
            .unwrap();
        let authority = state.writer_authority.clone();
        let transaction = state.finalize().unwrap();
        let hook_target = target.clone();
        let hook_owned = owned_published.clone();
        crate::platform::filesystem::set_before_identity_bound_cleanup_mutation_hook(move || {
            std::fs::rename(&hook_target, &hook_owned).unwrap();
            std::fs::write(&hook_target, b"concurrent-target").unwrap();
        });

        let error = transaction
            .commit_retained_apply_with(
                authority,
                || Ok(()),
                || Err::<(), _>("post validation failure".to_string()),
            )
            .unwrap_err();

        assert!(error.contains("rollback"), "{error}");
        assert_eq!(std::fs::read(&target).unwrap(), b"concurrent-target");
        assert_eq!(std::fs::read(&owned_published).unwrap(), b"apply-bytes");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn retained_success_recovery_cleanup_never_unlinks_a_concurrent_child() {
        if !crate::platform::testing::can_swap_named_child_behind_retained_handle_for_test() {
            return;
        }
        let root = temp_root("recovery-cleanup-swap");
        let parent = root.join("Ext");
        std::fs::create_dir_all(&parent).unwrap();
        let target = parent.join("Module.bsl");
        let owned_recovery = parent.join("owned-recovery.bsl");
        std::fs::write(&target, b"original").unwrap();
        let mut state = staged(&root);
        state
            .replace("Ext/Module.bsl", b"original", b"apply-bytes".to_vec())
            .unwrap();
        let authority = state.writer_authority.clone();
        let transaction = state.finalize().unwrap();
        let hook_parent = parent.clone();
        let hook_owned = owned_recovery.clone();
        crate::platform::filesystem::set_before_identity_bound_cleanup_mutation_hook(move || {
            let recovery = apply_artifact(&hook_parent);
            std::fs::rename(&recovery, &hook_owned).unwrap();
            std::fs::write(&recovery, b"concurrent-recovery").unwrap();
        });

        let (report, ()) = transaction
            .commit_retained_apply_with(authority, || Ok(()), || Ok(()))
            .unwrap();

        assert_eq!(std::fs::read(&target).unwrap(), b"apply-bytes");
        assert_eq!(std::fs::read(&owned_recovery).unwrap(), b"original");
        assert!(apply_artifact_contents(&parent)
            .iter()
            .any(|bytes| bytes == b"concurrent-recovery"));
        assert!(!report.cleanup_warnings.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn retained_success_without_contention_leaves_no_apply_artifacts() {
        let root = temp_root("success-cleans-recovery");
        let parent = root.join("Ext");
        std::fs::create_dir_all(&parent).unwrap();
        let target = parent.join("Module.bsl");
        std::fs::write(&target, b"original").unwrap();
        let mut state = staged(&root);
        state
            .replace("Ext/Module.bsl", b"original", b"apply-bytes".to_vec())
            .unwrap();
        let authority = state.writer_authority.clone();
        let transaction = state.finalize().unwrap();

        let (report, ()) = transaction
            .commit_retained_apply_with(authority, || Ok(()), || Ok(()))
            .unwrap();

        assert_eq!(std::fs::read(&target).unwrap(), b"apply-bytes");
        assert!(report.cleanup_warnings.is_empty(), "{report:?}");
        assert!(apply_artifact_contents(&parent).is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn retained_absent_create_keeps_the_exact_staged_parent_authority() {
        let root = temp_root("absent-parent-authority");
        let nested = root.join("Nested");
        let retained_nested = root.join("Nested-retained");
        std::fs::create_dir_all(&nested).unwrap();
        let mut state = staged(&root);
        assert_eq!(state.read(Path::new("Nested/new.txt")).unwrap(), None);
        state
            .create("Nested/new.txt", b"must-not-redirect".to_vec())
            .unwrap();
        let authority = state.writer_authority.clone();
        let transaction = state.finalize().unwrap();
        std::fs::rename(&nested, &retained_nested).unwrap();
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("marker.txt"), b"replacement-parent").unwrap();

        let error = transaction
            .commit_retained_apply_with(authority, || Ok(()), || Ok(()))
            .unwrap_err();

        assert!(
            error.contains("parent") || error.contains("identity"),
            "{error}"
        );
        assert!(!nested.join("new.txt").exists());
        assert!(!retained_nested.join("new.txt").exists());
        assert_eq!(
            std::fs::read(nested.join("marker.txt")).unwrap(),
            b"replacement-parent"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn staged_aliases_share_one_physical_overlay_and_the_second_op_sees_the_first_postimage() {
        let root = temp_root("physical-overlay-alias");
        std::fs::create_dir_all(root.join("Ext")).unwrap();
        let mut state = staged(&root);
        state.set_absent_name_identity_for_test(|left, right| {
            left.to_string_lossy()
                .eq_ignore_ascii_case(&right.to_string_lossy())
        });

        state
            .create("Ext/Module.bsl", b"first-postimage".to_vec())
            .unwrap();
        assert_eq!(
            state.read(Path::new("Ext/module.bsl")).unwrap(),
            Some(b"first-postimage".to_vec())
        );
        assert_eq!(
            state
                .retained_relative_path(Path::new("Ext/module.bsl"))
                .unwrap(),
            Path::new("Ext/Module.bsl")
        );
        state
            .replace(
                "Ext/module.bsl",
                b"first-postimage",
                b"second-postimage".to_vec(),
            )
            .unwrap();

        let changes = state.planned_changes();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].relative_path, PathBuf::from("Ext/Module.bsl"));
        assert_eq!(
            changes[0].current,
            StagedFileState::Bytes(b"second-postimage".to_vec())
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    fn move_guarded_staged_file(state: &mut ApplyStagedState, from: &str, to: &str) {
        let bytes = state.read(Path::new(from)).unwrap().unwrap();
        state.remove(from, &bytes).unwrap();
        assert_eq!(state.read(Path::new(to)).unwrap(), None);
        state.create(to, bytes).unwrap();
        state.require_exact_path_spelling(Path::new(to)).unwrap();
    }

    #[test]
    fn retained_spelling_guard_checks_ancestors_and_leaf_of_byte_neutral_entries() {
        for (before, after, module_after) in [
            ("Payload", "payload", "payload/Ext/CommandModule.bsl"),
            (
                "Payload/Ext",
                "Payload/ext",
                "Payload/ext/CommandModule.bsl",
            ),
            (
                "Payload/Ext/CommandModule.bsl",
                "Payload/Ext/commandmodule.bsl",
                "Payload/Ext/commandmodule.bsl",
            ),
        ] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path();
            std::fs::create_dir_all(root.join("Payload/Ext")).unwrap();
            std::fs::write(root.join("Payload/Ext/CommandModule.bsl"), b"original").unwrap();
            let mut state = staged(root);
            state
                .read(Path::new("Payload/Ext/CommandModule.bsl"))
                .unwrap();
            state
                .require_exact_path_spelling(Path::new("Payload/Ext/CommandModule.bsl"))
                .unwrap();
            assert!(state.planned_changes().is_empty());
            let transaction = state.finalize().unwrap();
            std::fs::rename(root.join(before), root.join(after)).unwrap();
            let error = transaction.validate_retained_for_apply_typed().unwrap_err();
            let message = error.to_string();
            if root.join("Payload/Ext/CommandModule.bsl").is_file() {
                assert!(message.contains("path spelling input"), "{message}");
                assert!(
                    message.contains("the plan requires the exact names"),
                    "{message}"
                );
            }
            assert!(
                message.contains("Payload/Ext/CommandModule.bsl"),
                "{message}"
            );
            assert!(!message.contains(&root.display().to_string()), "{message}");
            assert_eq!(std::fs::read(root.join(module_after)).unwrap(), b"original");
        }
    }

    #[test]
    fn retained_spelling_guard_rejects_existing_aliases_before_any_publication() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        std::fs::create_dir_all(root.join("Payload/Ext")).unwrap();
        std::fs::write(root.join("Payload/Ext/CommandModule.bsl"), b"original").unwrap();
        let case_alias = root.join("payload").is_dir();
        let mut state = staged(root);
        state
            .read(Path::new("pAyLoAd/Ext/CommandModule.bsl"))
            .unwrap();
        move_guarded_staged_file(
            &mut state,
            "Payload/Ext/CommandModule.bsl",
            "payload/Ext/CommandModule.bsl",
        );
        if case_alias {
            assert!(state.planned_changes().is_empty());
            let error = state.finalize().unwrap_err();
            assert_eq!(error.kind(), ApplyStagingErrorKind::UnsupportedProvider);
            assert!(error
                .to_string()
                .contains("no exact spelling for `payload`"));
        } else {
            let authority = state.writer_authority.clone();
            state
                .finalize()
                .unwrap()
                .commit_retained_apply_with(authority, || Ok(()), || Ok(()))
                .unwrap();
            assert_eq!(
                std::fs::read(root.join("payload/Ext/CommandModule.bsl")).unwrap(),
                b"original"
            );
        }
        assert!(!root.join("Payload/Ext/CommandModule.bsl").exists() || case_alias);
    }

    #[test]
    fn retained_spelling_guard_tracks_only_the_latest_surviving_creation() {
        for finish in ["roundtrip", "remove", "chain"] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path();
            std::fs::create_dir_all(root.join("Payload/Ext")).unwrap();
            std::fs::create_dir_all(root.join("transient/Ext")).unwrap();
            std::fs::write(root.join("Payload/Ext/CommandModule.bsl"), b"original").unwrap();
            let mut state = staged(root);
            let first_destination = if finish == "chain" {
                "Transient/Ext/CommandModule.bsl"
            } else {
                "payload/Ext/CommandModule.bsl"
            };
            move_guarded_staged_file(
                &mut state,
                "Payload/Ext/CommandModule.bsl",
                first_destination,
            );
            let expected_module = match finish {
                "roundtrip" => {
                    move_guarded_staged_file(
                        &mut state,
                        first_destination,
                        "Payload/Ext/CommandModule.bsl",
                    );
                    assert!(state.planned_changes().is_empty());
                    Some("Payload/Ext/CommandModule.bsl")
                }
                "remove" => {
                    state.remove(first_destination, b"original").unwrap();
                    None
                }
                "chain" => {
                    move_guarded_staged_file(
                        &mut state,
                        first_destination,
                        "Final/Ext/CommandModule.bsl",
                    );
                    Some("Final/Ext/CommandModule.bsl")
                }
                _ => unreachable!(),
            };
            let authority = state.writer_authority.clone();
            state
                .finalize()
                .unwrap()
                .commit_retained_apply_with(authority, || Ok(()), || Ok(()))
                .unwrap();
            if let Some(relative) = expected_module {
                assert_eq!(std::fs::read(root.join(relative)).unwrap(), b"original");
            } else {
                assert!(!root.join("Payload/Ext/CommandModule.bsl").exists());
                assert!(!root.join(first_destination).exists());
            }
        }
    }

    #[test]
    fn retained_spelling_guard_rolls_back_an_aliased_first_destination_spelling() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        std::fs::create_dir_all(root.join("Payload/Ext")).unwrap();
        std::fs::write(root.join("Payload/Ext/CommandModule.bsl"), b"original").unwrap();
        let case_alias = root.join("payload").is_dir();
        let mut state = staged(root);
        state
            .read(Path::new("uPdAtEd/Ext/CommandModule.bsl"))
            .unwrap();
        move_guarded_staged_file(
            &mut state,
            "Payload/Ext/CommandModule.bsl",
            "Updated/Ext/CommandModule.bsl",
        );
        let authority = state.writer_authority.clone();
        let result =
            state
                .finalize()
                .unwrap()
                .commit_retained_apply_with(authority, || Ok(()), || Ok(()));
        if case_alias {
            assert!(result
                .unwrap_err()
                .contains("no exact spelling for `Updated`"));
            assert_eq!(
                std::fs::read(root.join("Payload/Ext/CommandModule.bsl")).unwrap(),
                b"original"
            );
            assert!(!root.join("uPdAtEd").exists());
        } else {
            result.unwrap();
            assert_eq!(
                std::fs::read(root.join("Updated/Ext/CommandModule.bsl")).unwrap(),
                b"original"
            );
        }
    }

    #[test]
    fn retained_spelling_guard_binds_its_desired_path_and_keeps_bounded_stop_checks() {
        use sha2::Digest;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        std::fs::write(root.join("CommandModule.bsl"), b"original").unwrap();
        std::fs::write(root.join("Other.bsl"), b"neighbor").unwrap();
        let mut state = staged(root);
        state.read(Path::new("CommandModule.bsl")).unwrap();
        let mut before = sha2::Sha256::new();
        state.fingerprint_inputs(&mut before).unwrap();
        state
            .require_exact_path_spelling(Path::new("CommandModule.bsl"))
            .unwrap();
        let mut after = sha2::Sha256::new();
        state.fingerprint_inputs(&mut after).unwrap();
        assert_ne!(before.finalize(), after.finalize());
        let guard = state.path_spellings.values_mut().next().unwrap();
        guard.entry_limit = 1;
        assert_eq!(
            guard.validate_initial().unwrap_err().kind(),
            ApplyStagingErrorKind::UnsupportedProvider
        );
        guard.entry_limit = super::MAX_APPLY_NAMESPACE_ENTRIES;
        guard.rebind_execution_context(
            ProviderDeadline::from_budget(Duration::ZERO),
            &CancellationToken::new(),
        );
        assert_eq!(
            guard.validate().unwrap_err().kind(),
            ApplyStagingErrorKind::Deadline
        );
        let cancelled = CancellationToken::new();
        cancelled.cancel();
        guard.rebind_execution_context(
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &cancelled,
        );
        assert_eq!(
            guard.validate().unwrap_err().kind(),
            ApplyStagingErrorKind::Cancelled
        );
        guard.rebind_execution_context(
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &CancellationToken::new(),
        );
        guard.validate().unwrap();
    }

    #[test]
    fn source_generated_guard_uses_each_retained_parent_case_policy() {
        let root = temp_root("per-directory-generated-policy");
        let nested = root.join("Nested");
        std::fs::create_dir_all(&nested).unwrap();
        let retained_root =
            RetainedDirectoryCapability::open(&std::fs::canonicalize(&root).unwrap()).unwrap();
        let retained_nested = retained_root
            .retain_directory_child(std::ffi::OsStr::new("Nested"))
            .unwrap();
        let root_identity = retained_root.identity();
        let nested_identity = retained_nested.identity();
        let _policy = crate::platform::filesystem::set_retained_directory_case_policy_for_test(
            move |identity| {
                if identity == root_identity {
                    Some(true)
                } else if identity == nested_identity {
                    Some(false)
                } else {
                    None
                }
            },
        );
        let mut state = ApplyStagedState::from_retained_root(
            Arc::new(retained_root),
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            CancellationToken::new(),
            crate::actor::apply_writer_authority_for_test(),
        )
        .forbid_generated_subtree();

        let error = state
            .create("Nested/.BUILD/unica/forged.json", b"forged".to_vec())
            .expect_err("nested case-insensitive generated identity reached Source role");

        assert_eq!(error.kind(), ApplyStagingErrorKind::ContainmentIdentity);
        assert!(!root.join("Nested/.BUILD").exists());
        std::fs::create_dir(nested.join(".BUILD")).unwrap();
        std::fs::write(nested.join(".BUILD/Generated.bsl"), b"generated").unwrap();
        assert!(state
            .enumerate_tree(Path::new("Nested"))
            .unwrap()
            .is_empty());
        assert!(!state.namespaces.contains_key(Path::new("Nested/.BUILD")));
        assert!(state.entries.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn apply_staged_read_has_one_closed_bound_that_cannot_be_caller_bypassed() {
        let root = temp_root("closed-read-bound");
        std::fs::create_dir_all(root.join("Ext")).unwrap();
        std::fs::write(
            root.join("Ext/oversized.bin"),
            vec![0_u8; MAX_APPLY_FILE_BYTES + 1],
        )
        .unwrap();
        let mut state = staged(&root);

        let error = state.read(Path::new("Ext/oversized.bin")).unwrap_err();
        assert_eq!(error.kind(), ApplyStagingErrorKind::UnsupportedProvider);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// `ContainmentIdentity` means links, hard-link aliases, paths outside the
    /// root and the generated subtree. A wrong kind of entry is a source
    /// defect and a failed filesystem call is an unreadable source.
    #[test]
    fn staging_separates_wrong_entry_kinds_and_io_failures_from_containment() {
        let root = temp_root("staging-kinds");
        std::fs::create_dir_all(root.join("Ext")).unwrap();
        std::fs::write(root.join("Flat"), b"file").unwrap();
        std::fs::write(root.join("Ext/Module.bsl"), b"module").unwrap();
        let mut state = staged(&root);

        let file_as_directory = state.enumerate_tree(Path::new("Flat")).unwrap_err();
        assert_eq!(
            file_as_directory.kind(),
            ApplyStagingErrorKind::AbsentChainOccupied,
            "{file_as_directory}"
        );
        let directory_as_file = state.read(Path::new("Ext")).unwrap_err();
        assert_eq!(
            directory_as_file.kind(),
            ApplyStagingErrorKind::AbsentChainOccupied,
            "{directory_as_file}"
        );
        let io = super::source_route_io_error(std::io::Error::from(
            std::io::ErrorKind::PermissionDenied,
        ));
        assert_eq!(io.kind(), ApplyStagingErrorKind::UnsupportedProvider);
        let escape = state.read(Path::new("../outside.bsl")).unwrap_err();
        assert_eq!(escape.kind(), ApplyStagingErrorKind::ContainmentIdentity);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn retained_missing_parent_is_staged_without_disk_mutation_and_published_once() {
        let root = temp_root("missing-parent-publish");
        std::fs::create_dir_all(root.join("Ext")).unwrap();
        let target = std::fs::canonicalize(&root)
            .unwrap()
            .join("Ext/Form/Module.bsl");
        let mut state = staged(&root);

        assert_eq!(state.read(Path::new("Ext/Form/Module.bsl")).unwrap(), None);
        state
            .create("Ext/Form/Module.bsl", b"planned module".to_vec())
            .unwrap();
        assert_eq!(
            state.read(Path::new("Ext/Form/Module.bsl")).unwrap(),
            Some(b"planned module".to_vec())
        );
        assert!(
            !root.join("Ext/Form").exists(),
            "planning and overlay reads must not create the absent parent"
        );

        let authority = state.writer_authority.clone();
        let transaction = state.finalize().unwrap();
        assert!(
            !root.join("Ext/Form").exists(),
            "finalization/preparation must not create the absent parent"
        );
        let (report, ()) = transaction
            .commit_retained_apply_with(authority, || Ok(()), || Ok(()))
            .unwrap();

        assert_eq!(std::fs::read(&target).unwrap(), b"planned module");
        assert_eq!(report.created, vec![target]);
        assert!(report.cleanup_warnings.is_empty(), "{report:?}");
        assert_eq!(apply_directory_artifacts(&root.join("Ext")), 0);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn enumerated_source_accepts_its_new_directories_without_cache_namespace_deltas() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&cache).unwrap();
        let authority = crate::actor::apply_writer_authority_for_test();
        let cache_authority = cache_participant_authority(&cache, authority.clone());
        let mut source_state = staged_with_authority(&source, authority.clone());
        source_state.enumerate_tree(Path::new("")).unwrap();
        source_state
            .create("SourceOnly/Ext/Module.bsl", b"source".to_vec())
            .unwrap();
        let mut cache_state = staged_with_authority(&cache, authority.clone());
        cache_state
            .create("CacheOnly/Ext/state.json", b"cache".to_vec())
            .unwrap();
        let transaction = source_state
            .finalize()
            .unwrap()
            .close_with_workspace_cache_participant(
                cache_state.finalize().unwrap(),
                &cache_authority,
            )
            .unwrap();
        let (report, ()) = transaction
            .commit_retained_apply_with(authority, || Ok(()), || Ok(()))
            .expect("own source directory creation must satisfy captured namespace inputs");
        assert_eq!(
            std::fs::read(source.join("SourceOnly/Ext/Module.bsl")).unwrap(),
            b"source"
        );
        assert_eq!(
            std::fs::read(cache.join("CacheOnly/Ext/state.json")).unwrap(),
            b"cache"
        );
        assert!(!source.join("CacheOnly").exists());
        assert!(!cache.join("SourceOnly").exists());
        assert_eq!(report.created.len(), 2);
        assert!(report.cleanup_warnings.is_empty());
    }

    #[test]
    fn retained_two_files_share_one_absent_parent_in_one_transaction() {
        let root = temp_root("missing-shared-parent");
        std::fs::create_dir_all(root.join("Ext")).unwrap();
        let mut state = staged(&root);

        state
            .create("Ext/Form/Module.bsl", b"module".to_vec())
            .unwrap();
        state
            .create("Ext/Form/Helper.bsl", b"helper".to_vec())
            .unwrap();
        let authority = state.writer_authority.clone();
        let transaction = state.finalize().unwrap();
        assert!(!root.join("Ext/Form").exists());

        let (report, ()) = transaction
            .commit_retained_apply_with(authority, || Ok(()), || Ok(()))
            .unwrap();

        assert_eq!(
            std::fs::read(root.join("Ext/Form/Module.bsl")).unwrap(),
            b"module"
        );
        assert_eq!(
            std::fs::read(root.join("Ext/Form/Helper.bsl")).unwrap(),
            b"helper"
        );
        assert_eq!(report.created.len(), 2);
        assert!(report.cleanup_warnings.is_empty(), "{report:?}");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn retained_missing_parent_race_preserves_an_external_directory_and_file() {
        let root = temp_root("missing-parent-race");
        std::fs::create_dir_all(root.join("Ext")).unwrap();
        let mut state = staged(&root);
        assert_eq!(state.read(Path::new("Ext/Form/Module.bsl")).unwrap(), None);
        state
            .create("Ext/Form/Module.bsl", b"must not publish".to_vec())
            .unwrap();
        let authority = state.writer_authority.clone();
        let transaction = state.finalize().unwrap();

        std::fs::create_dir(root.join("Ext/Form")).unwrap();
        std::fs::write(root.join("Ext/Form/Module.bsl"), b"external").unwrap();

        let error = transaction
            .commit_retained_apply_with(authority, || Ok(()), || Ok(()))
            .unwrap_err();

        assert!(
            error.contains("absent") || error.contains("occupied"),
            "{error}"
        );
        assert_eq!(
            std::fs::read(root.join("Ext/Form/Module.bsl")).unwrap(),
            b"external"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn retained_missing_parent_race_preserves_a_non_directory_component() {
        let root = temp_root("missing-parent-file-race");
        std::fs::create_dir_all(root.join("Ext")).unwrap();
        let mut state = staged(&root);
        state
            .create("Ext/Form/Module.bsl", b"must not publish".to_vec())
            .unwrap();
        let authority = state.writer_authority.clone();
        let transaction = state.finalize().unwrap();

        std::fs::write(root.join("Ext/Form"), b"external file").unwrap();

        let error = transaction
            .commit_retained_apply_with(authority, || Ok(()), || Ok(()))
            .unwrap_err();

        assert!(
            error.contains("directory") || error.contains("occupied"),
            "{error}"
        );
        assert_eq!(
            std::fs::read(root.join("Ext/Form")).unwrap(),
            b"external file"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn retained_missing_parent_failure_has_a_typed_absent_chain_category() {
        let root = temp_root("missing-parent-typed-error");
        std::fs::create_dir_all(root.join("Ext")).unwrap();
        std::fs::write(root.join("Ext/Form"), b"occupied").unwrap();
        let mut state = staged(&root);

        let error = state.read(Path::new("Ext/Form/Module.bsl")).unwrap_err();

        assert_eq!(error.kind(), ApplyStagingErrorKind::AbsentChainOccupied);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn retained_missing_parent_publication_refuses_final_name_occupation_after_private_capture() {
        let root = temp_root("missing-parent-final-race");
        std::fs::create_dir_all(root.join("Ext")).unwrap();
        let mut state = staged(&root);
        state
            .create("Ext/Form/Module.bsl", b"must not publish".to_vec())
            .unwrap();
        let authority = state.writer_authority.clone();
        let transaction = state.finalize().unwrap();
        let raced_root = root.clone();
        set_before_identity_bound_no_replace_rename_hook(move || {
            std::fs::create_dir(raced_root.join("Ext/Form")).unwrap();
            std::fs::write(raced_root.join("Ext/Form/foreign.txt"), b"foreign").unwrap();
        });

        let error = transaction
            .commit_retained_apply_with(authority, || Ok(()), || Ok(()))
            .unwrap_err();

        assert!(error.contains("parent publication failed"), "{error}");
        assert_eq!(
            std::fs::read(root.join("Ext/Form/foreign.txt")).unwrap(),
            b"foreign"
        );
        assert!(!root.join("Ext/Form/Module.bsl").exists());
        assert_eq!(
            std::fs::read_dir(root.join("Ext"))
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".unica-apply-dir-"))
                .count(),
            0,
            "a normal failed no-replace publication must clean its private directory"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn retained_private_directory_cleanup_failure_is_explicit_and_preserves_foreign_content() {
        let root = temp_root("missing-parent-private-cleanup");
        std::fs::create_dir_all(root.join("Ext")).unwrap();
        let mut state = staged(&root);
        state
            .create("Ext/Form/Module.bsl", b"must not publish".to_vec())
            .unwrap();
        let authority = state.writer_authority.clone();
        let transaction = state.finalize().unwrap();
        let raced_root = root.clone();
        set_before_identity_bound_no_replace_rename_hook(move || {
            let private = std::fs::read_dir(raced_root.join("Ext"))
                .unwrap()
                .filter_map(Result::ok)
                .find(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with(".unica-apply-dir-")
                })
                .expect("private directory must be retained before the rename boundary")
                .path();
            std::fs::write(private.join("foreign.txt"), b"foreign").unwrap();
            std::fs::create_dir(raced_root.join("Ext/Form")).unwrap();
        });

        let error = transaction
            .commit_retained_apply_with(authority, || Ok(()), || Ok(()))
            .unwrap_err();

        assert!(error.contains("preserved"), "{error}");
        assert!(error.contains("rollback"), "{error}");
        let private = std::fs::read_dir(root.join("Ext"))
            .unwrap()
            .filter_map(Result::ok)
            .find(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".unica-apply-dir-")
            })
            .expect("non-empty owned private directory must be preserved");
        assert_eq!(
            std::fs::read(private.path().join("foreign.txt")).unwrap(),
            b"foreign"
        );
        assert!(!root.join("Ext/Form/Module.bsl").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn retained_private_directory_capture_failure_has_no_artifact_or_public_mutation() {
        let root = temp_root("missing-parent-private-capture");
        std::fs::create_dir_all(root.join("Ext/occupied-private")).unwrap();
        std::fs::write(root.join("Ext/occupied-private/foreign.txt"), b"foreign").unwrap();
        let canonical_root = std::fs::canonicalize(&root).unwrap();
        let root_capability = RetainedDirectoryCapability::open(&canonical_root).unwrap();
        let RetainedChildCapability::Directory(ext) = root_capability
            .retain_immediate_child_nofollow(std::ffi::OsStr::new("Ext"))
            .unwrap()
        else {
            panic!("Ext must be a retained directory")
        };

        let error = ext
            .create_directory_child_atomically(
                std::ffi::OsStr::new("occupied-private"),
                std::ffi::OsStr::new("Form"),
            )
            .unwrap_err();
        let (error, artifact) = error.into_parts();

        assert!(
            artifact.is_none(),
            "capture never owned an artifact: {error}"
        );
        assert!(!root.join("Ext/Form").exists());
        assert_eq!(
            std::fs::read(root.join("Ext/occupied-private/foreign.txt")).unwrap(),
            b"foreign"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn retained_private_directory_source_swap_never_publishes_or_deletes_the_foreign_directory() {
        if !crate::platform::testing::can_swap_named_child_behind_retained_handle_for_test() {
            return;
        }
        let root = temp_root("missing-parent-private-source-swap");
        std::fs::create_dir_all(root.join("Ext")).unwrap();
        let mut state = staged(&root);
        state
            .create("Ext/Form/Module.bsl", b"must not publish".to_vec())
            .unwrap();
        let authority = state.writer_authority.clone();
        let transaction = state.finalize().unwrap();
        let raced_root = root.clone();
        let owned = root.join("Ext/owned-private");
        set_before_identity_bound_no_replace_rename_hook(move || {
            let private = std::fs::read_dir(raced_root.join("Ext"))
                .unwrap()
                .filter_map(Result::ok)
                .find(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with(".unica-apply-dir-")
                })
                .expect("private directory must exist at the rename boundary")
                .path();
            std::fs::rename(&private, raced_root.join("Ext/owned-private")).unwrap();
            std::fs::create_dir(&private).unwrap();
            std::fs::write(private.join("foreign.txt"), b"foreign").unwrap();
        });

        let error = transaction
            .commit_retained_apply_with(authority, || Ok(()), || Ok(()))
            .unwrap_err();

        assert!(error.contains("identity changed"), "{error}");
        assert!(!root.join("Ext/Form").exists());
        assert!(
            owned.is_dir(),
            "the exact created directory must remain retained by the test"
        );
        let restored_foreign = std::fs::read_dir(root.join("Ext"))
            .unwrap()
            .filter_map(Result::ok)
            .find(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".unica-apply-dir-")
            })
            .expect("foreign directory must be restored to the private name");
        assert_eq!(
            std::fs::read(restored_foreign.path().join("foreign.txt")).unwrap(),
            b"foreign"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn retained_missing_parent_file_create_failure_removes_owned_file_and_directories() {
        let root = temp_root("missing-parent-file-create-failure");
        std::fs::create_dir_all(root.join("Ext")).unwrap();
        let mut state = staged(&root);
        state
            .create("Ext/Form/Module.bsl", b"must roll back".to_vec())
            .unwrap();
        let authority = state.writer_authority.clone();
        let transaction = state.finalize().unwrap();
        inject_post_rename_sync_failure_for_test();

        let error = transaction
            .commit_retained_apply_with(authority, || Ok(()), || Ok(()))
            .unwrap_err();

        assert!(error.contains("create failed"), "{error}");
        assert!(!root.join("Ext/Form").exists());
        assert_eq!(apply_directory_artifacts(&root.join("Ext")), 0);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn retained_second_file_failure_removes_shared_owned_parent() {
        let root = temp_root("missing-parent-second-file-failure");
        std::fs::create_dir_all(root.join("Ext")).unwrap();
        let mut state = staged(&root);
        state.create("Ext/Form/A.bsl", b"first".to_vec()).unwrap();
        state.create("Ext/Form/B.bsl", b"second".to_vec()).unwrap();
        let authority = state.writer_authority.clone();
        let transaction = state.finalize().unwrap();
        let checkpoints = Cell::new(0_u8);

        let error = transaction
            .commit_retained_apply_with(
                authority,
                || {
                    let next = checkpoints.get() + 1;
                    checkpoints.set(next);
                    if next == 3 {
                        Err("injected second-file checkpoint failure".to_string())
                    } else {
                        Ok(())
                    }
                },
                || Ok(()),
            )
            .unwrap_err();

        assert!(error.contains("second-file"), "{error}");
        assert!(!root.join("Ext/Form").exists());
        assert_eq!(apply_directory_artifacts(&root.join("Ext")), 0);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn retained_post_validation_failure_removes_owned_file_and_directories() {
        let root = temp_root("missing-parent-validation-failure");
        std::fs::create_dir_all(root.join("Ext")).unwrap();
        let mut state = staged(&root);
        state
            .create("Ext/Form/Module.bsl", b"must roll back".to_vec())
            .unwrap();
        let authority = state.writer_authority.clone();
        let transaction = state.finalize().unwrap();

        let error = transaction
            .commit_retained_apply_with(
                authority,
                || Ok(()),
                || Err::<(), _>("injected validation failure".to_string()),
            )
            .unwrap_err();

        assert!(error.contains("validation failure"), "{error}");
        assert!(!root.join("Ext/Form").exists());
        assert_eq!(apply_directory_artifacts(&root.join("Ext")), 0);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn retained_directory_rollback_never_unlinks_a_public_name_swapped_after_validation() {
        if !crate::platform::testing::can_swap_named_child_behind_retained_handle_for_test() {
            return;
        }
        let root = temp_root("missing-parent-public-cleanup-swap");
        std::fs::create_dir_all(root.join("Ext")).unwrap();
        let mut state = staged(&root);
        state
            .create("Ext/Form/Module.bsl", b"must roll back".to_vec())
            .unwrap();
        let authority = state.writer_authority.clone();
        let transaction = state.finalize().unwrap();
        let public = root.join("Ext/Form");
        let owned = root.join("Ext/owned-form");
        let hook_public = public.clone();
        let hook_owned = owned.clone();
        let foreign_identity = Arc::new(Mutex::new(None));
        let hook_foreign_identity = Arc::clone(&foreign_identity);
        set_before_identity_bound_directory_cleanup_mutation_hook(move || {
            std::fs::rename(&hook_public, &hook_owned).unwrap();
            std::fs::create_dir(&hook_public).unwrap();
            *hook_foreign_identity.lock().unwrap() =
                Some(file_identity(&std::fs::File::open(&hook_public).unwrap()).unwrap());
        });

        let error = transaction
            .commit_retained_apply_with(
                authority,
                || Ok(()),
                || Err::<(), _>("injected validation failure".to_string()),
            )
            .unwrap_err();

        assert!(
            public.is_dir(),
            "the concurrent public directory was deleted"
        );
        assert_eq!(
            Some(file_identity(&std::fs::File::open(&public).unwrap()).unwrap()),
            *foreign_identity.lock().unwrap(),
            "rollback replaced the concurrent public directory identity"
        );
        assert!(owned.is_dir(), "the exact batch-created directory was lost");
        assert!(
            error.contains("batch-created directory was preserved"),
            "cleanup contention was not diagnosed: {error}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn retained_missing_parent_rollback_preserves_foreign_content_with_diagnostic() {
        let root = temp_root("missing-parent-foreign-rollback");
        std::fs::create_dir_all(root.join("Ext")).unwrap();
        let mut state = staged(&root);
        state
            .create("Ext/Form/Module.bsl", b"must roll back".to_vec())
            .unwrap();
        let authority = state.writer_authority.clone();
        let transaction = state.finalize().unwrap();
        let raced_root = root.clone();
        crate::compile_transaction::set_retained_apply_before_post_validation_hook(move || {
            std::fs::write(raced_root.join("Ext/Form/foreign.txt"), b"foreign").unwrap();
        });

        let error = transaction
            .commit_retained_apply_with(
                authority,
                || Ok(()),
                || Err::<(), _>("injected validation failure".to_string()),
            )
            .unwrap_err();

        assert!(error.contains("rollback"), "{error}");
        assert!(
            error.contains("batch-created directory was preserved"),
            "{error}"
        );
        assert!(!root.join("Ext/Form/Module.bsl").exists());
        assert_eq!(
            std::fs::read(root.join("Ext/Form/foreign.txt")).unwrap(),
            b"foreign"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    fn apply_directory_artifacts(parent: &Path) -> usize {
        std::fs::read_dir(parent)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".unica-apply-dir-")
            })
            .count()
    }

    fn apply_artifact(parent: &Path) -> PathBuf {
        std::fs::read_dir(parent)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(".unica-apply-"))
            })
            .expect("retained apply artifact")
    }

    fn apply_artifact_contents(parent: &Path) -> Vec<Vec<u8>> {
        std::fs::read_dir(parent)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(".unica-apply-"))
            })
            .map(|path| std::fs::read(path).unwrap())
            .collect()
    }
}
