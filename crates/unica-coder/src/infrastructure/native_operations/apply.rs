use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::ProviderDeadline;
use crate::domain::events::DomainEvent;
use crate::infrastructure::native_operations::compile_transaction::{
    CompileTransaction, RetainedApplyChangeBinding, RetainedApplyReadGuardBinding,
    RetainedApplyValidationError, RetainedApplyValidationErrorKind,
};
use crate::infrastructure::platform::filesystem::{
    FileIdentity, RetainedChildCapability, RetainedChildNameComparator, RetainedDirectoryCapability,
};
use crate::infrastructure::source_roots::GENERATED_DIR_NAME;
use sha2::Digest;
use std::ffi::{OsStr, OsString};
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

const MAX_APPLY_FILE_BYTES: usize = 32 * 1024 * 1024;

fn generated_component_identity_error(error: std::io::Error) -> ApplyStagingError {
    ApplyStagingError::new(
        ApplyStagingErrorKind::ContainmentIdentity,
        format!("generated source component identity cannot be proven: {error}"),
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
        .map_err(generated_component_identity_error)?
    {
        return Err(ApplyStagingError::new(
            ApplyStagingErrorKind::ContainmentIdentity,
            "source participant cannot address the generated subtree",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ApplyStagingErrorKind {
    Cancelled,
    Deadline,
    ContainmentIdentity,
    MissingParent,
    AbsentChainOccupied,
    UnsupportedProvider,
    ConcurrentRevision,
    Invariant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ApplyStagingError {
    kind: ApplyStagingErrorKind,
    message: String,
}

impl ApplyStagingError {
    pub(in crate::infrastructure) fn new(
        kind: ApplyStagingErrorKind,
        message: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub(crate) fn kind(&self) -> ApplyStagingErrorKind {
        self.kind
    }

    #[cfg(test)]
    fn contains(&self, pattern: &str) -> bool {
        self.message.contains(pattern)
    }
}

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

impl std::fmt::Display for ApplyStagingError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.message.fmt(formatter)
    }
}

impl std::error::Error for ApplyStagingError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ApplyPlanErrorKind {
    BadValue,
    NotFound,
    ProviderUnavailable,
    InvalidState,
    InvalidSource,
    Staging(ApplyStagingErrorKind),
    Postcondition,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ApplyPlanError {
    kind: ApplyPlanErrorKind,
    path: Option<String>,
    message: String,
}

impl ApplyPlanError {
    pub(crate) fn new(kind: ApplyPlanErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            path: None,
            message: message.into(),
        }
    }

    pub(crate) const fn kind(&self) -> ApplyPlanErrorKind {
        self.kind
    }

    pub(crate) fn path(&self) -> Option<&str> {
        self.path.as_deref()
    }

    pub(crate) fn at_path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }

    pub(crate) fn staging(error: ApplyStagingError, path: impl Into<String>) -> Self {
        Self::new(
            ApplyPlanErrorKind::Staging(error.kind()),
            "staged source evidence is unavailable",
        )
        .at_path(path)
    }
}

impl std::fmt::Display for ApplyPlanError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.message.fmt(formatter)
    }
}

impl std::error::Error for ApplyPlanError {}

pub(super) fn empty_apply_family_batch() -> ApplyPlanError {
    ApplyPlanError::new(
        ApplyPlanErrorKind::BadValue,
        "apply family batch must contain at least one operation",
    )
    .at_path("ops")
}

pub(super) fn hidden_apply_family_unimplemented(op_index: usize) -> ApplyPlanError {
    ApplyPlanError::new(
        ApplyPlanErrorKind::ProviderUnavailable,
        "hidden v0.13 apply family is not implemented",
    )
    .at_path(format!("ops[{op_index}].op"))
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BorrowPlanDetail {
    pub(crate) index: usize,
    pub(crate) at: String,
    pub(crate) from: String,
    pub(crate) parent_uuid: String,
    pub(crate) created: bool,
    pub(crate) changed_properties: Vec<String>,
    pub(crate) protected_overrides: Vec<String>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct PlannedApplyEffects {
    events: Vec<DomainEvent>,
    /// The staged files each event stands for, parallel to `events`. A
    /// planner that knows which file its event describes records it here so
    /// request-level reconciliation can drop exactly the events whose file
    /// was restored; an empty list means "the whole batch".
    paths: Vec<Vec<PathBuf>>,
    /// Typed warnings a planner attaches to a plan that still executes: a
    /// forced removal names the files that keep referring to the object.
    warnings: Vec<serde_json::Value>,
    borrowing: Vec<BorrowPlanDetail>,
}

impl PlannedApplyEffects {
    pub(crate) fn borrowing(&self) -> &[BorrowPlanDetail] {
        &self.borrowing
    }
    pub(crate) fn set_borrowing(&mut self, details: Vec<BorrowPlanDetail>) {
        self.borrowing = details;
    }

    pub(crate) fn events(&self) -> &[DomainEvent] {
        &self.events
    }

    pub(crate) fn warnings(&self) -> &[serde_json::Value] {
        &self.warnings
    }

    pub(crate) fn push_warning(&mut self, warning: serde_json::Value) {
        self.warnings.push(warning);
    }

    pub(crate) fn into_events(self) -> Vec<DomainEvent> {
        self.events
    }

    /// Events paired with the files they describe (empty when unknown).
    pub(crate) fn into_events_with_paths(self) -> Vec<(DomainEvent, Vec<PathBuf>)> {
        self.events.into_iter().zip(self.paths).collect()
    }

    pub(crate) fn append(&mut self, event: DomainEvent) {
        self.append_at(event, Vec::new());
    }

    /// Appends one event that describes the given staged files. A repeated
    /// event (same kind and artifact) merges its files into the first one.
    pub(crate) fn append_at(&mut self, event: DomainEvent, paths: Vec<PathBuf>) {
        if let Some(index) = self
            .events
            .iter()
            .position(|current| current.kind == event.kind && current.artifact == event.artifact)
        {
            for path in paths {
                if !self.paths[index].contains(&path) {
                    self.paths[index].push(path);
                }
            }
            return;
        }
        self.events.push(event);
        self.paths.push(paths);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StagedFileState {
    Bytes(Vec<u8>),
    Absent,
}

impl StagedFileState {
    fn as_option(&self) -> Option<Vec<u8>> {
        match self {
            Self::Bytes(bytes) => Some(bytes.clone()),
            Self::Absent => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StagedChangeKind {
    Create,
    Replace,
    Remove,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StagedApplyChange {
    pub(crate) relative_path: PathBuf,
    pub(crate) kind: StagedChangeKind,
    pub(crate) original: StagedFileState,
    pub(crate) current: StagedFileState,
}

#[derive(Debug)]
struct StagedEntry {
    relative_path: PathBuf,
    ancestor: RetainedDirectoryCapability,
    missing_parent_chain: Vec<OsString>,
    name: OsString,
    target_identity: StagedTargetIdentity,
    original: StagedFileState,
    current: StagedFileState,
    original_file:
        Option<crate::infrastructure::platform::filesystem::RetainedRegularFileCapability>,
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
            .map_err(generated_component_identity_error)?;
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
                Ok(_) => {
                    return Err(ApplyStagingError::new(
                        ApplyStagingErrorKind::ContainmentIdentity,
                        "namespace input is not a directory",
                    ))
                }
                Err(error) => return Err(generated_component_identity_error(error)),
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
            .map_err(|error| {
                stopped.unwrap_or_else(|| generated_component_identity_error(error))
            })?;
        let comparator = directory
            .child_name_comparator()
            .map_err(generated_component_identity_error)?;
        let mut children = std::collections::BTreeMap::new();
        for name in names {
            self.checkpoint()?;
            if comparator
                .names_equivalent(&name, OsStr::new(GENERATED_DIR_NAME))
                .map_err(generated_component_identity_error)?
            {
                continue;
            }
            let kind = match directory
                .retain_immediate_child_nofollow(&name)
                .map_err(generated_component_identity_error)?
            {
                RetainedChildCapability::Directory(_) => 1,
                RetainedChildCapability::RegularFile(_) => 2,
                _ => {
                    return Err(ApplyStagingError::new(
                        ApplyStagingErrorKind::ContainmentIdentity,
                        "enumerated input contains a link or unsupported entry",
                    ))
                }
            };
            children.insert(name, kind);
        }
        directory
            .validate_named_identity()
            .map_err(generated_component_identity_error)?;
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
        if self.deadline.remaining().is_zero() {
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
                ApplyStagingErrorKind::ContainmentIdentity,
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

    fn fingerprint(&self, hash: &mut sha2::Sha256) -> Result<(), ApplyStagingError> {
        use sha2::Digest;
        self.checkpoint()?;
        let path =
            crate::infrastructure::platform::filesystem::stable_path_identity_bytes(&self.relative)
                .map_err(|error| {
                    ApplyStagingError::new(ApplyStagingErrorKind::ContainmentIdentity, error)
                })?;
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
                        crate::infrastructure::platform::filesystem::stable_path_identity_bytes(
                            Path::new(name),
                        )
                        .map_err(|error| {
                            ApplyStagingError::new(
                                ApplyStagingErrorKind::ContainmentIdentity,
                                error,
                            )
                        })?;
                    hash.update((name.len() as u64).to_be_bytes());
                    hash.update(name);
                    hash.update([*kind]);
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
pub(crate) struct ApplyStagedState {
    root: Arc<RetainedDirectoryCapability>,
    entries: Vec<StagedEntry>,
    read_guards: std::collections::BTreeMap<FileIdentity, RetainedApplyReadGuardBinding>,
    namespaces: std::collections::BTreeMap<PathBuf, RetainedNamespaceGuard>,
    deadline: ProviderDeadline,
    cancellation: CancellationToken,
    writer_authority: crate::infrastructure::workspace_actor::ApplyWriterAuthority,
    generated_subtree_forbidden: bool,
    #[cfg(test)]
    absent_name_identity_for_test: Option<fn(&std::ffi::OsStr, &std::ffi::OsStr) -> bool>,
}

impl ApplyStagedState {
    pub(in crate::infrastructure) fn from_retained_root(
        root: Arc<RetainedDirectoryCapability>,
        deadline: ProviderDeadline,
        cancellation: CancellationToken,
        writer_authority: crate::infrastructure::workspace_actor::ApplyWriterAuthority,
    ) -> Self {
        Self {
            root,
            entries: Vec::new(),
            read_guards: Default::default(),
            namespaces: Default::default(),
            deadline,
            cancellation,
            writer_authority,
            generated_subtree_forbidden: false,
            #[cfg(test)]
            absent_name_identity_for_test: None,
        }
    }

    pub(in crate::infrastructure) fn forbid_generated_subtree(mut self) -> Self {
        self.generated_subtree_forbidden = true;
        self
    }

    /// Enumerate only a subtree needed by this planner and retain its membership.
    pub(crate) fn enumerate_tree(
        &mut self,
        relative: &Path,
    ) -> Result<Vec<PathBuf>, ApplyStagingError> {
        self.enumerate_tree_with_limits(relative, 256, 1_000_000)
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

    pub(crate) fn read(&mut self, relative: &Path) -> Result<Option<Vec<u8>>, ApplyStagingError> {
        self.read_bounded(relative, MAX_APPLY_FILE_BYTES)
    }

    pub(crate) fn read_bounded(
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

    /// Read one reference input without retaining its complete body in the
    /// saved plan. The caller owns the temporary bytes only for this scan.
    pub(crate) fn read_guarded_bounded(
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
        for component in &components[..components.len() - 1] {
            self.checkpoint("apply reference input route")?;
            if self.generated_subtree_forbidden {
                let policy = parent
                    .child_name_comparator()
                    .map_err(generated_component_identity_error)?;
                reject_generated_component(Some(&policy), component)?;
            }
            parent = match parent.retain_immediate_child_nofollow(component) {
                Ok(RetainedChildCapability::Directory(child)) => child,
                Err(error) if error.kind() == ErrorKind::NotFound => {
                    return self.read_bounded(&relative, limit);
                }
                _ => {
                    return Err(ApplyStagingError::new(
                        ApplyStagingErrorKind::ContainmentIdentity,
                        format!("reference input parent changed: {}", relative.display()),
                    ))
                }
            };
        }
        let name = components.last().expect("strict path has a name").clone();
        if self.generated_subtree_forbidden {
            let policy = parent
                .child_name_comparator()
                .map_err(generated_component_identity_error)?;
            reject_generated_component(Some(&policy), &name)?;
        }
        let file = match parent.retain_immediate_child_nofollow(&name) {
            Ok(RetainedChildCapability::RegularFile(file)) => file,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return self.read_bounded(&relative, limit);
            }
            _ => {
                return Err(ApplyStagingError::new(
                    ApplyStagingErrorKind::ContainmentIdentity,
                    format!(
                        "reference input is not a regular file: {}",
                        relative.display()
                    ),
                ))
            }
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
                    ancestor: parent,
                    name,
                    original_file: file,
                    expected_len: bytes.len() as u64,
                    expected_sha256: digest,
                    deadline: self.deadline,
                    cancellation: self.cancellation.clone(),
                },
            );
        }
        Ok(Some(bytes))
    }

    pub(crate) fn create(
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
        Ok(())
    }

    /// Stages one absent terminal only when its immediate parent was retained
    /// as an existing directory. Family planners that do not own topology
    /// creation use this instead of the generic multi-component create path.
    pub(crate) fn create_leaf_below_retained_parent(
        &mut self,
        relative: impl AsRef<Path>,
        bytes: Vec<u8>,
    ) -> Result<(), ApplyStagingError> {
        let relative = strict_relative(relative.as_ref())?;
        let index = self.ensure_loaded(&relative)?;
        let entry = &mut self.entries[index];
        if !entry.missing_parent_chain.is_empty() {
            return Err(ApplyStagingError::new(
                ApplyStagingErrorKind::MissingParent,
                "staged leaf requires an already retained immediate parent",
            ));
        }
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
        Ok(())
    }

    pub(crate) fn replace(
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

    pub(crate) fn remove(
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

    /// All observed inputs, including read-only dependencies and absent targets,
    /// and their planned postimages bind the preview to its exact write intent.
    pub(crate) fn fingerprint_inputs(
        &self,
        hash: &mut sha2::Sha256,
    ) -> Result<(), ApplyStagingError> {
        use sha2::Digest;
        hash.update(self.root.identity().stable_bytes());
        hash.update((self.entries.len() as u64).to_be_bytes());
        let mut entries = self.entries.iter().collect::<Vec<_>>();
        entries.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
        for entry in entries {
            self.checkpoint("apply plan inputs")?;
            let path = crate::infrastructure::platform::filesystem::stable_path_identity_bytes(
                &entry.relative_path,
            )
            .map_err(|error| {
                ApplyStagingError::new(ApplyStagingErrorKind::ContainmentIdentity, error)
            })?;
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
            let path = crate::infrastructure::platform::filesystem::stable_path_identity_bytes(
                &guard.relative_path,
            )
            .map_err(|error| {
                ApplyStagingError::new(ApplyStagingErrorKind::ContainmentIdentity, error)
            })?;
            hash.update((path.len() as u64).to_be_bytes());
            hash.update(path);
            hash.update(guard.expected_len.to_be_bytes());
            hash.update(guard.expected_sha256);
        }
        hash.update((self.namespaces.len() as u64).to_be_bytes());
        for guard in self.namespaces.values() {
            guard.fingerprint(hash)?;
        }
        Ok(())
    }

    pub(crate) fn planned_changes(&self) -> Vec<StagedApplyChange> {
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

    pub(crate) fn finalize(self) -> Result<CompileTransaction, ApplyStagingError> {
        self.checkpoint("apply finalization")?;
        let mut transaction = CompileTransaction::new();
        transaction
            .bind_retained_apply_root(Arc::clone(&self.root), &self.writer_authority)
            .map_err(|error| ApplyStagingError::new(ApplyStagingErrorKind::Invariant, error))?;
        transaction.bind_retained_namespaces(self.namespaces.into_values().collect());
        transaction.bind_retained_apply_read_guards(self.read_guards.into_values().collect());
        let mut entries = self.entries;
        entries.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
        for entry in entries {
            transaction
                .bind_retained_apply_change(
                    RetainedApplyChangeBinding {
                        root: Arc::clone(&self.root),
                        relative_path: entry.relative_path,
                        ancestor: entry.ancestor,
                        missing_parent_chain: entry.missing_parent_chain,
                        name: entry.name,
                        original: entry.original.as_option(),
                        current: entry.current.as_option(),
                        original_file: entry.original_file,
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

    pub(in crate::infrastructure) fn retained_root_identity(
        &self,
    ) -> crate::infrastructure::platform::filesystem::FileIdentity {
        self.root.identity()
    }

    pub(in crate::infrastructure) fn has_writer_authority(
        &self,
        authority: &crate::infrastructure::workspace_actor::ApplyWriterAuthority,
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
        let mut missing_parent_chain = Vec::new();
        let mut generated_name_comparator = self
            .generated_subtree_forbidden
            .then(|| ancestor.child_name_comparator())
            .transpose()
            .map_err(generated_component_identity_error)?;
        for (index, component) in components[..components.len() - 1].iter().enumerate() {
            reject_generated_component(generated_name_comparator.as_ref(), component)?;
            match ancestor.retain_immediate_child_nofollow(component) {
                Ok(RetainedChildCapability::Directory(directory)) => {
                    ancestor = directory;
                    generated_name_comparator = self
                        .generated_subtree_forbidden
                        .then(|| ancestor.child_name_comparator())
                        .transpose()
                        .map_err(generated_component_identity_error)?;
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
        let (original, original_file) = match retained_child {
            None => (StagedFileState::Absent, None),
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
                (StagedFileState::Bytes(bytes), Some(file))
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
                    ApplyStagingErrorKind::ContainmentIdentity,
                    format!(
                        "staged target is not a regular file: {}",
                        relative.display()
                    ),
                ))
            }
            Some(Err(error)) if error.kind() == ErrorKind::NotFound => {
                (StagedFileState::Absent, None)
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
                guard
                    .original_file
                    .validate_named_identity()
                    .map_err(|error| {
                        ApplyStagingError::new(
                            ApplyStagingErrorKind::ConcurrentRevision,
                            format!("reference input changed before write staging: {error}"),
                        )
                    })?;
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
            ancestor,
            missing_parent_chain,
            name,
            target_identity: target_identity.expect("regular or absent target has an identity"),
            current: original.clone(),
            original,
            original_file,
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
        } else if self.deadline.remaining().is_zero() {
            Err(ApplyStagingError::new(
                ApplyStagingErrorKind::Deadline,
                format!("{phase} deadline exceeded"),
            ))
        } else {
            Ok(())
        }
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

#[cfg(test)]
pub(crate) mod tests {
    use super::{
        ApplyStagedState, ApplyStagingErrorKind, StagedChangeKind, StagedFileState,
        MAX_APPLY_FILE_BYTES,
    };
    use crate::domain::cancellation::CancellationToken;
    use crate::domain::code_intelligence::ProviderDeadline;
    use crate::infrastructure::platform::filesystem::{
        file_identity, inject_post_rename_sync_failure_for_test,
        set_before_identity_bound_directory_cleanup_mutation_hook,
        set_before_identity_bound_no_replace_rename_hook, RetainedChildCapability,
        RetainedDirectoryCapability,
    };
    use crate::infrastructure::platform::testing::{
        attempt_retained_directory_replacement_for_test, create_directory_link_fixture_for_test,
        path_identity_for_test, FileLinkFixtureOutcome, RetainedDirectoryReplacementOutcome,
    };
    use std::cell::Cell;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    fn staged(root: &Path) -> ApplyStagedState {
        staged_with_authority(
            root,
            crate::infrastructure::workspace_actor::apply_writer_authority_for_test(),
        )
    }

    fn staged_with_authority(
        root: &Path,
        authority: crate::infrastructure::workspace_actor::ApplyWriterAuthority,
    ) -> ApplyStagedState {
        let canonical = std::fs::canonicalize(root).unwrap();
        ApplyStagedState::from_retained_root(
            Arc::new(RetainedDirectoryCapability::open(&canonical).unwrap()),
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            CancellationToken::new(),
            authority,
        )
    }

    fn cache_participant_authority(
        root: &Path,
        authority: crate::infrastructure::workspace_actor::ApplyWriterAuthority,
    ) -> crate::infrastructure::workspace_actor::WorkspaceCacheParticipantAuthority {
        let canonical = std::fs::canonicalize(root).unwrap();
        let retained = RetainedDirectoryCapability::open(&canonical).unwrap();
        crate::infrastructure::workspace_actor::workspace_cache_participant_authority_for_test(
            authority, &retained,
        )
    }

    #[test]
    pub(crate) fn retained_transaction_roles_require_explicit_roots_and_cache_authority() {
        let root = temp_root("closed-participant-roots");
        let source = root.join("source");
        let cache = root.join("cache");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&cache).unwrap();
        let authority = crate::infrastructure::workspace_actor::apply_writer_authority_for_test();
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
        let authority = crate::infrastructure::workspace_actor::apply_writer_authority_for_test();
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

        let foreign_authority =
            crate::infrastructure::workspace_actor::apply_writer_authority_for_test();
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
        let authority = crate::infrastructure::workspace_actor::apply_writer_authority_for_test();
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
            let authority =
                crate::infrastructure::workspace_actor::apply_writer_authority_for_test();
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
        for late in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path();
            std::fs::create_dir(root.join("Payload")).unwrap();
            let original = root.join("Payload/Module.bsl");
            let foreign = root.join("Payload/Added.bsl");
            std::fs::write(&original, b"original").unwrap();
            let authority =
                crate::infrastructure::workspace_actor::apply_writer_authority_for_test();
            let mut state = staged_with_authority(root, authority.clone());
            assert_eq!(
                state.enumerate_tree(Path::new("Payload")).unwrap(),
                vec![PathBuf::from("Payload/Module.bsl")]
            );
            state.remove("Payload/Module.bsl", b"original").unwrap();
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
            assert!(result.is_err(), "namespace race returned success");
            assert_eq!(std::fs::read(&original).unwrap(), b"original");
            assert_eq!(std::fs::read(&foreign).unwrap(), b"foreign");
            assert_eq!(injected, late);
        }
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
        crate::infrastructure::platform::filesystem::create_test_directory_link(
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
    fn reference_scan_refuses_a_file_larger_than_the_per_file_budget() {
        use crate::infrastructure::native_operations::meta::remove::META_REMOVE_REFERENCE_FILE_MAX_BYTES;
        let root = temp_root("reference-file-budget");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::File::create(root.join("Huge.xml"))
            .unwrap()
            .set_len(META_REMOVE_REFERENCE_FILE_MAX_BYTES as u64 + 1)
            .unwrap();
        let mut state = staged(&root);
        let error = state
            .read_guarded_bounded(Path::new("Huge.xml"), META_REMOVE_REFERENCE_FILE_MAX_BYTES)
            .unwrap_err();
        assert!(error.to_string().contains("bound"), "{error}");
        assert!(
            state.entries.is_empty(),
            "oversized input must not enter the staged byte set"
        );
        std::fs::write(root.join("Small.xml"), "\u{feff}<Root/>".as_bytes()).unwrap();
        assert_eq!(
            state
                .read_guarded_bounded(Path::new("Small.xml"), META_REMOVE_REFERENCE_FILE_MAX_BYTES)
                .unwrap(),
            Some("\u{feff}<Root/>".as_bytes().to_vec())
        );
        assert!(state.entries.is_empty());
        assert_eq!(state.read_guards.len(), 1);
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
        let authority = crate::infrastructure::workspace_actor::apply_writer_authority_for_test();
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

    #[test]
    fn reference_guard_rejects_late_change_and_rolls_back_own_write() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        std::fs::write(root.join("Reference.bsl"), b"abcdef").unwrap();
        std::fs::write(root.join("Owner.bsl"), b"before").unwrap();
        let authority = crate::infrastructure::workspace_actor::apply_writer_authority_for_test();
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
        crate::infrastructure::native_operations::compile_transaction::
            set_retained_apply_before_post_validation_hook(move || {
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
        let authority = crate::infrastructure::workspace_actor::apply_writer_authority_for_test();
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
        let authority = crate::infrastructure::workspace_actor::apply_writer_authority_for_test();
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
        let authority = crate::infrastructure::workspace_actor::apply_writer_authority_for_test();
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
        crate::infrastructure::native_operations::compile_transaction::
            set_retained_apply_after_reference_stream_hook(1, move || {
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
        let authority = crate::infrastructure::workspace_actor::apply_writer_authority_for_test();
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
        let authority = crate::infrastructure::workspace_actor::apply_writer_authority_for_test();
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
                    assert!(raced.finalize().unwrap_err().contains("link/reparse"));
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
        if !crate::infrastructure::platform::testing::can_swap_named_child_behind_retained_handle_for_test() {
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
        crate::infrastructure::platform::filesystem::set_before_identity_bound_no_replace_rename_hook(
            move || {
                let stage = apply_artifact(&hook_parent);
                std::fs::rename(&stage, &hook_owned).unwrap();
                std::fs::write(&stage, b"concurrent-stage").unwrap();
            },
        );

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
        if !crate::infrastructure::platform::testing::can_swap_named_child_behind_retained_handle_for_test() {
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
        crate::infrastructure::native_operations::compile_transaction::set_retained_apply_before_post_validation_hook(
            move || {
                crate::infrastructure::platform::filesystem::set_before_identity_bound_no_replace_rename_hook(
                    move || {
                        let recovery = apply_artifact(&hook_parent);
                        std::fs::rename(&recovery, &hook_owned).unwrap();
                        std::fs::write(&recovery, b"concurrent-recovery").unwrap();
                    },
                );
            },
        );

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
        if !crate::infrastructure::platform::testing::can_swap_named_child_behind_retained_handle_for_test() {
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
        crate::infrastructure::platform::filesystem::set_before_identity_bound_cleanup_mutation_hook(
            move || {
                std::fs::rename(&hook_target, &hook_owned).unwrap();
                std::fs::write(&hook_target, b"concurrent-target").unwrap();
            },
        );

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
        if !crate::infrastructure::platform::testing::can_swap_named_child_behind_retained_handle_for_test() {
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
        crate::infrastructure::platform::filesystem::set_before_identity_bound_cleanup_mutation_hook(
            move || {
                let recovery = apply_artifact(&hook_parent);
                std::fs::rename(&recovery, &hook_owned).unwrap();
                std::fs::write(&recovery, b"concurrent-recovery").unwrap();
            },
        );

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
        let _policy = crate::infrastructure::platform::filesystem::set_retained_directory_case_policy_for_test(
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
            crate::infrastructure::workspace_actor::apply_writer_authority_for_test(),
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

    #[test]
    fn retained_parent_leaf_create_is_narrower_than_generic_topology_staging() {
        let root = temp_root("retained-parent-leaf-create");
        std::fs::create_dir_all(root.join("Existing")).unwrap();
        let mut state = staged(&root);

        state
            .create_leaf_below_retained_parent("Existing/Module.bsl", b"leaf".to_vec())
            .unwrap();
        let error = state
            .create_leaf_below_retained_parent("Missing/Module.bsl", b"leaf".to_vec())
            .unwrap_err();

        assert_eq!(error.kind(), ApplyStagingErrorKind::MissingParent);
        assert!(!root.join("Existing/Module.bsl").exists());
        assert!(!root.join("Missing").exists());
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
        let authority = crate::infrastructure::workspace_actor::apply_writer_authority_for_test();
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
        if !crate::infrastructure::platform::testing::can_swap_named_child_behind_retained_handle_for_test() {
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
        if !crate::infrastructure::platform::testing::can_swap_named_child_behind_retained_handle_for_test() {
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
        crate::infrastructure::native_operations::compile_transaction::set_retained_apply_before_post_validation_hook(
            move || {
                std::fs::write(raced_root.join("Ext/Form/foreign.txt"), b"foreign").unwrap();
            },
        );

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

    fn temp_root(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "unica-apply-stage-{name}-{}-{nonce}",
            std::process::id()
        ))
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
