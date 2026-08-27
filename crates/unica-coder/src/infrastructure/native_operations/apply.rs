use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::ProviderDeadline;
use crate::domain::events::DomainEvent;
use crate::infrastructure::native_operations::compile_transaction::{
    CompileTransaction, RetainedApplyChangeBinding, RetainedApplyValidationError,
    RetainedApplyValidationErrorKind,
};
use crate::infrastructure::platform::filesystem::{
    FileIdentity, RetainedChildCapability, RetainedChildNameComparator, RetainedDirectoryCapability,
};
use crate::infrastructure::source_roots::GENERATED_DIR_NAME;
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

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct PlannedApplyEffects {
    events: Vec<DomainEvent>,
}

impl PlannedApplyEffects {
    pub(crate) fn events(&self) -> &[DomainEvent] {
        &self.events
    }

    pub(crate) fn into_events(self) -> Vec<DomainEvent> {
        self.events
    }

    pub(crate) fn append(&mut self, event: DomainEvent) {
        if !self
            .events
            .iter()
            .any(|current| current.kind == event.kind && current.artifact == event.artifact)
        {
            self.events.push(event);
        }
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

#[derive(Debug)]
pub(crate) struct ApplyStagedState {
    root: Arc<RetainedDirectoryCapability>,
    entries: Vec<StagedEntry>,
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

    pub(crate) fn read(&mut self, relative: &Path) -> Result<Option<Vec<u8>>, ApplyStagingError> {
        let relative = strict_relative(relative)?;
        let index = self.ensure_loaded(&relative)?;
        Ok(self.entries[index].current.as_option())
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
                let bytes = file.read_bounded(MAX_APPLY_FILE_BYTES).map_err(|error| {
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

    fn checkpoint(&self, phase: &str) -> Result<(), ApplyStagingError> {
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
        create_directory_link_fixture_for_test, FileLinkFixtureOutcome,
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
            std::fs::rename(root.join("Race"), &displaced).unwrap();
            assert_eq!(
                create_directory_link_fixture_for_test(&external, root.join("Race")).unwrap(),
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
