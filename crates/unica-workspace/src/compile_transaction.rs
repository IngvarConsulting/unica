//! Failure-atomic publication for the metadata compile writers.
//!
//! The compile families build their files in memory, add them to a single
//! [`CompileTransaction`], and publish the transaction once. New files remain
//! create-only; existing regular files require an exact preimage before atomic
//! replacement. Removal trees are streamed into size/SHA-256 snapshots, reject
//! links, and are moved to same-filesystem recovery locations until the whole
//! transaction has passed post-write validation. Canonical XML registrations
//! are still edited textually through the `cf.edit` registrar.
//!
//! "Failure-atomic" here covers reported I/O and validation errors. It does not
//! claim process-crash or power-loss atomicity; those require a persistent journal
//! and directory-entry synchronization that this transaction does not provide.

mod retained_directory_publication;

use roxmltree::Document;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{ErrorKind, Read};
#[cfg(any(test, feature = "test-support"))]
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use unica_source_contract::proposal::{
    DirectoryMembershipSelector, DirectoryMembershipSnapshot, DirectoryTopologyEntry,
    DirectoryTopologyEntryKind,
};

use crate::actor::{
    ApplyPublicationError, ApplyPublicationErrorKind, ApplyWriterAuthority, RetainedApplyFinalGate,
    WorkspaceCacheParticipantAuthority,
};
use crate::paths::normalize_path_identity;
use crate::platform::filesystem::{
    create_new_directory_child, file_identity, hard_link_count, is_file_sharing_violation,
    metadata_is_link_or_reparse_point, open_directory_child_nofollow, open_directory_nofollow,
    open_regular_child_nofollow, prepare_file_for_removal,
    remove_identity_bound_empty_directory_child, remove_identity_bound_regular_child,
    rename_identity_bound_regular_child_no_replace, rename_no_replace,
    retain_regular_child_for_cleanup, FileIdentity, PortablePermissions, RetainedChildCapability,
    RetainedDirectoryCapability,
};

#[cfg(any(test, feature = "test-support"))]
use std::cell::{Cell, RefCell};

#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetainedApplyObservedEvent {
    Source(PathBuf),
    EagerMetadata(PathBuf),
    StateMarker(PathBuf),
    Rollback(PathBuf),
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetainedApplyFailpoint {
    Source(usize),
    EagerMetadata(usize),
    StateMarker,
    AfterAllPostimages,
}

#[cfg(any(test, feature = "test-support"))]
type RetainedApplyAfterReferenceStreamHook = (usize, Box<dyn FnOnce()>);

#[cfg(any(test, feature = "test-support"))]
type RetainedApplyAfterWriteTargetChunkHook = Box<dyn FnOnce()>;

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static RETAINED_APPLY_OBSERVED_EVENTS: RefCell<Vec<RetainedApplyObservedEvent>> = const { RefCell::new(Vec::new()) };
    static RETAINED_APPLY_FAILPOINT: RefCell<Option<RetainedApplyFailpoint>> = const { RefCell::new(None) };
}

#[cfg(any(test, feature = "test-support"))]
pub fn take_retained_apply_observed_events() -> Vec<RetainedApplyObservedEvent> {
    RETAINED_APPLY_OBSERVED_EVENTS.with(|events| std::mem::take(&mut *events.borrow_mut()))
}

#[cfg(any(test, feature = "test-support"))]
pub fn set_retained_apply_failpoint(failpoint: RetainedApplyFailpoint) {
    RETAINED_APPLY_FAILPOINT.with(|slot| *slot.borrow_mut() = Some(failpoint));
}

#[cfg(any(test, feature = "test-support"))]
fn retained_apply_fail_after(event: &RetainedApplyObservedEvent) -> Result<(), String> {
    RETAINED_APPLY_FAILPOINT.with(|slot| {
        let mut pending = slot.borrow_mut();
        let Some(failpoint) = pending.as_mut() else {
            return Ok(());
        };
        let triggered = match (failpoint, event) {
            (RetainedApplyFailpoint::Source(remaining), RetainedApplyObservedEvent::Source(_)) => {
                *remaining = remaining.saturating_sub(1);
                *remaining == 0
            }
            (
                RetainedApplyFailpoint::EagerMetadata(remaining),
                RetainedApplyObservedEvent::EagerMetadata(_),
            ) => {
                *remaining = remaining.saturating_sub(1);
                *remaining == 0
            }
            (RetainedApplyFailpoint::StateMarker, RetainedApplyObservedEvent::StateMarker(_)) => {
                true
            }
            _ => false,
        };
        if triggered {
            pending.take();
            Err("injected retained apply failure after publication".to_string())
        } else {
            Ok(())
        }
    })
}

#[cfg(any(test, feature = "test-support"))]
fn retained_apply_fail_after_all_postimages() -> Result<(), String> {
    RETAINED_APPLY_FAILPOINT.with(|slot| {
        if *slot.borrow() == Some(RetainedApplyFailpoint::AfterAllPostimages) {
            slot.borrow_mut().take();
            Err("injected retained apply failure after all postimages".to_string())
        } else {
            Ok(())
        }
    })
}

#[cfg(feature = "test-support")]
use crate::platform::filesystem::replace_file_atomically;

#[cfg(any(test, feature = "test-support"))]
use std::sync::Barrier;

use super::single_file_publisher::{
    absolute_lexical_path, cleanup_publication_artifact, prepare, verify_publication_artifact,
    with_publication_locks_mode_and_guard_targets, write_exact_new_file_in_directory,
    CleanupArtifact, CleanupWarning, PreparedCreate, PreparedPublication, PreparedReplace,
    PublicationLockToken, PublicationTreeLockMode, PublishError, PublishErrorKind, PublishMode,
    PublishPhase, PublishRequest,
};

#[cfg(feature = "test-support")]
use super::single_file_publisher::with_before_bound_cleanup_hook;
#[cfg(any(test, feature = "test-support"))]
use super::single_file_publisher::{
    with_before_commit_hook, with_publication_lock_contention_signal, with_publication_lock_pause,
};

#[cfg(test)]
use super::single_file_publisher::write_exact_new_file;

const UTF8_BOM: &[u8] = b"\xef\xbb\xbf";
static RECOVERY_SEQUENCE: AtomicU64 = AtomicU64::new(1);

/// Result of asking the canonical registrar to add one child object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistrationStatus {
    Added,
    AlreadyPresent,
    MissingTarget,
}

/// Exact replacement required to turn the original bytes into planned bytes.
///
/// Applying `after` to `byte_range` of the original file reproduces the planned
/// registration file byte-for-byte.  Tests use it to verify the planned
/// registration without reading the target again.
#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistrationDiff {
    pub path: PathBuf,
    pub byte_range: Range<usize>,
    pub before: Vec<u8>,
    pub after: Vec<u8>,
}

/// Files actually published by a successful transaction.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommitReport {
    pub created: Vec<PathBuf>,
    pub updated: Vec<PathBuf>,
    /// Cleanup failures do not invalidate already-validated published bytes.
    /// They are surfaced so a caller can report an orphaned recovery copy explicitly.
    pub cleanup_warnings: Vec<String>,
    /// Retained-apply cleanup has a closed diagnostic shape so the workspace
    /// actor can surface actionable logical context without exposing a root.
    pub retained_apply_cleanup_diagnostics: Vec<RetainedApplyCleanupDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetainedApplyCleanupDiagnostic {
    logical_target: PathBuf,
    artifact_name: OsString,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetainedApplyValidationErrorKind {
    Cancelled,
    Deadline,
    ConcurrentRevision,
    ContainmentIdentity,
    AbsentChainOccupied,
    UnsupportedProvider,
    Invariant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApplyInputRole {
    SourceWrite,
    ReadOnly,
    SharedCache,
    DirectoryMembership,
    PathSpelling,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ApplyInputContext {
    relative_path: PathBuf,
    role: ApplyInputRole,
}

impl ApplyInputContext {
    fn for_entry(entry: &PlannedRetainedApplyChange, source_participant: bool) -> Self {
        Self {
            relative_path: entry.relative_path.clone(),
            role: if !source_participant {
                ApplyInputRole::SharedCache
            } else if entry.original == entry.current {
                ApplyInputRole::ReadOnly
            } else {
                ApplyInputRole::SourceWrite
            },
        }
    }

    fn explain(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let reason = match self.role {
            ApplyInputRole::SourceWrite => "source write target; the plan retains its original state to prevent overwriting changes",
            ApplyInputRole::ReadOnly => "read-only input; the plan was prepared using this input",
            ApplyInputRole::SharedCache => "shared cache input; the plan retains its original state for atomic publication of source and cache changes",
            ApplyInputRole::DirectoryMembership => "directory membership input; the plan enumerated this directory to retain its identity and child names",
            ApplyInputRole::PathSpelling => "path spelling input; the plan requires the exact names of this path and its ancestors",
        };
        let path = if self.relative_path.as_os_str().is_empty() {
            Path::new(".")
        } else {
            &self.relative_path
        };
        write!(formatter, "; input `{}`: {reason}", path.display())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetainedApplyValidationError {
    kind: RetainedApplyValidationErrorKind,
    message: String,
    input: Option<ApplyInputContext>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetainedApplyPublishErrorKind {
    Cancelled,
    Deadline,
    ConcurrentRevision,
    ContainmentIdentity,
    Provider,
    Invariant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetainedApplyPublishError {
    kind: RetainedApplyPublishErrorKind,
    message: String,
    input: Option<ApplyInputContext>,
}

impl RetainedApplyPublishError {
    fn new(kind: RetainedApplyPublishErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            input: None,
        }
    }

    fn with_input(mut self, input: ApplyInputContext) -> Self {
        self.input = Some(input);
        self
    }

    pub fn kind(&self) -> RetainedApplyPublishErrorKind {
        self.kind
    }

    fn provider(message: impl Into<String>) -> Self {
        Self::new(RetainedApplyPublishErrorKind::Provider, message)
    }

    fn containment(message: impl Into<String>) -> Self {
        Self::new(RetainedApplyPublishErrorKind::ContainmentIdentity, message)
    }
}

impl From<RetainedApplyValidationError> for RetainedApplyPublishError {
    fn from(error: RetainedApplyValidationError) -> Self {
        let kind = match error.kind() {
            RetainedApplyValidationErrorKind::Cancelled => RetainedApplyPublishErrorKind::Cancelled,
            RetainedApplyValidationErrorKind::Deadline => RetainedApplyPublishErrorKind::Deadline,
            RetainedApplyValidationErrorKind::ConcurrentRevision => {
                RetainedApplyPublishErrorKind::ConcurrentRevision
            }
            RetainedApplyValidationErrorKind::ContainmentIdentity
            | RetainedApplyValidationErrorKind::AbsentChainOccupied => {
                RetainedApplyPublishErrorKind::ContainmentIdentity
            }
            RetainedApplyValidationErrorKind::UnsupportedProvider => {
                RetainedApplyPublishErrorKind::Provider
            }
            RetainedApplyValidationErrorKind::Invariant => RetainedApplyPublishErrorKind::Invariant,
        };
        Self {
            kind,
            message: error.message,
            input: error.input,
        }
    }
}

impl std::fmt::Display for RetainedApplyPublishError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.message.fmt(formatter)?;
        if let Some(input) = &self.input {
            input.explain(formatter)?;
        }
        Ok(())
    }
}

impl std::error::Error for RetainedApplyPublishError {}

fn retained_apply_publish_publication_error(
    error: RetainedApplyPublishError,
) -> ApplyPublicationError {
    let kind = match error.kind() {
        RetainedApplyPublishErrorKind::Cancelled => ApplyPublicationErrorKind::Cancelled,
        RetainedApplyPublishErrorKind::Deadline => ApplyPublicationErrorKind::Deadline,
        RetainedApplyPublishErrorKind::ConcurrentRevision => {
            ApplyPublicationErrorKind::ConcurrentRevision
        }
        RetainedApplyPublishErrorKind::ContainmentIdentity => {
            ApplyPublicationErrorKind::ContainmentIdentity
        }
        RetainedApplyPublishErrorKind::Provider => {
            ApplyPublicationErrorKind::ProviderPostvalidation
        }
        RetainedApplyPublishErrorKind::Invariant => ApplyPublicationErrorKind::Invariant,
    };
    ApplyPublicationError::new(kind, error.to_string())
}

impl RetainedApplyValidationError {
    fn new(kind: RetainedApplyValidationErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            input: None,
        }
    }

    fn with_input(mut self, input: ApplyInputContext) -> Self {
        self.input = Some(input);
        self
    }

    pub fn kind(&self) -> RetainedApplyValidationErrorKind {
        self.kind
    }
}

impl std::fmt::Display for RetainedApplyValidationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.message.fmt(formatter)?;
        if let Some(input) = &self.input {
            input.explain(formatter)?;
        }
        Ok(())
    }
}

impl std::error::Error for RetainedApplyValidationError {}

fn retained_apply_validation_publication_error(
    error: RetainedApplyValidationError,
) -> ApplyPublicationError {
    let kind = match error.kind() {
        RetainedApplyValidationErrorKind::Cancelled => ApplyPublicationErrorKind::Cancelled,
        RetainedApplyValidationErrorKind::Deadline => ApplyPublicationErrorKind::Deadline,
        RetainedApplyValidationErrorKind::ConcurrentRevision => {
            ApplyPublicationErrorKind::ConcurrentRevision
        }
        RetainedApplyValidationErrorKind::ContainmentIdentity
        | RetainedApplyValidationErrorKind::AbsentChainOccupied => {
            ApplyPublicationErrorKind::ContainmentIdentity
        }
        RetainedApplyValidationErrorKind::UnsupportedProvider => {
            ApplyPublicationErrorKind::ProviderPostvalidation
        }
        RetainedApplyValidationErrorKind::Invariant => ApplyPublicationErrorKind::Invariant,
    };
    ApplyPublicationError::new(kind, error.to_string())
}

impl RetainedApplyCleanupDiagnostic {
    pub fn into_parts(self) -> (PathBuf, OsString) {
        (self.logical_target, self.artifact_name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitFailureKind {
    ConcurrentModification,
    ProviderUnavailable,
    RollbackFailed,
}

#[derive(Debug)]
pub struct CommitFailure {
    kind: CommitFailureKind,
    message: String,
}

impl CommitFailure {
    pub fn concurrent(message: impl Into<String>) -> Self {
        Self {
            kind: CommitFailureKind::ConcurrentModification,
            message: message.into(),
        }
    }

    pub fn provider(message: impl Into<String>) -> Self {
        Self {
            kind: CommitFailureKind::ProviderUnavailable,
            message: message.into(),
        }
    }

    fn rollback(message: impl Into<String>) -> Self {
        Self {
            kind: CommitFailureKind::RollbackFailed,
            message: message.into(),
        }
    }

    pub fn kind(&self) -> CommitFailureKind {
        self.kind
    }

    fn into_message(self) -> String {
        self.message
    }
}

impl From<String> for CommitFailure {
    fn from(message: String) -> Self {
        Self::provider(message)
    }
}

#[derive(Debug)]
struct PlannedCreate {
    path: PathBuf,
    identity: PathBuf,
    bytes: Vec<u8>,
}

#[derive(Debug)]
struct PlannedRegistration {
    path: PathBuf,
    original: Vec<u8>,
    updated: Vec<u8>,
}

#[derive(Debug)]
struct PlannedReadGuard {
    path: PathBuf,
    expected_preimage: Vec<u8>,
}

#[derive(Debug)]
struct PlannedRemoval {
    path: PathBuf,
    identity: PathBuf,
    snapshot: RemovalSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RemovalSnapshot {
    entries: Vec<RemovalEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RemovalEntry {
    relative_path: PathBuf,
    kind: RemovalEntryKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RemovalEntryKind {
    File { size: u64, sha256: [u8; 32] },
    Directory,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DirectoryMembershipGuard {
    directory: PathBuf,
    selector: DirectoryMembershipSelector,
    expected: DirectoryMembershipSnapshot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlannedPathKind {
    Create,
    Registration,
    ReadGuard,
    AbsenceGuard,
    Removal,
}

impl PlannedRegistration {
    fn changed(&self) -> bool {
        self.original != self.updated
    }

    #[cfg(any(test, feature = "test-support"))]
    fn diff(&self) -> Option<RegistrationDiff> {
        self.changed()
            .then(|| byte_diff(&self.path, &self.original, &self.updated))
    }
}

/// In-memory plan for one compile invocation, including a `meta.compile` batch.
#[derive(Debug, Default)]
pub struct CompileTransaction {
    creates: Vec<PlannedCreate>,
    registrations: BTreeMap<PathBuf, PlannedRegistration>,
    read_guards: BTreeMap<PathBuf, PlannedReadGuard>,
    absence_guards: BTreeSet<PathBuf>,
    directory_membership_guards: BTreeMap<PathBuf, DirectoryMembershipGuard>,
    removals: Vec<PlannedRemoval>,
    planned_path_identities: BTreeMap<PathBuf, PlannedPathKind>,
    retained_apply: Vec<PlannedRetainedApplyChange>,
    retained_apply_read_guards: Vec<RetainedApplyReadGuardBinding>,
    retained_namespaces: Vec<super::apply::RetainedNamespaceGuard>,
    retained_directory_removals: Vec<super::retained_directory::DirectoryRemoval>,
    retained_path_spellings: Vec<super::apply::RetainedPathSpellingGuard>,
    retained_apply_authority: Option<ApplyWriterAuthority>,
    retained_apply_root: Option<Arc<RetainedDirectoryCapability>>,
    retained_apply_cache_root: Option<Arc<RetainedDirectoryCapability>>,
    retained_apply_cache_start: Option<usize>,
}

#[derive(Debug)]
struct PlannedRetainedApplyChange {
    root: Arc<RetainedDirectoryCapability>,
    relative_path: PathBuf,
    route_identities: Vec<FileIdentity>,
    missing_parent_chain: Vec<OsString>,
    name: OsString,
    original: Option<Vec<u8>>,
    current: Option<Vec<u8>>,
    original_file_identity: Option<FileIdentity>,
    original_len: Option<u64>,
    original_sha256: Option<[u8; 32]>,
    deadline: unica_work_control::provider_deadline::ProviderDeadline,
    cancellation: unica_work_control::cancellation::CancellationToken,
}

pub(super) struct RetainedApplyChangeBinding {
    pub(super) root: Arc<RetainedDirectoryCapability>,
    pub(super) relative_path: PathBuf,
    pub(super) route_identities: Vec<FileIdentity>,
    pub(super) missing_parent_chain: Vec<OsString>,
    pub(super) name: OsString,
    pub(super) original: Option<Vec<u8>>,
    pub(super) current: Option<Vec<u8>>,
    pub(super) original_file_identity: Option<FileIdentity>,
    pub(super) original_len: Option<u64>,
    pub(super) original_sha256: Option<[u8; 32]>,
    pub(super) deadline: unica_work_control::provider_deadline::ProviderDeadline,
    pub(super) cancellation: unica_work_control::cancellation::CancellationToken,
}

#[derive(Debug)]
struct RetainedApplyExecutionBinding {
    ancestor: RetainedDirectoryCapability,
    original_file: Option<crate::platform::filesystem::RetainedRegularFileCapability>,
}

/// An unchanged input inspected by a retained apply planner. It keeps the
/// physical identities of its route and a content fingerprint, but neither the
/// file body nor an open descriptor: a reference scan over a whole
/// configuration must not hold one descriptor per inspected file. The check
/// walks the route from the retained root again and proves the same physical
/// objects and bytes. Without a held descriptor a filesystem may reuse an
/// identity for a recreated object; the bytes are still rehashed, so only a
/// recreation with identical content passes, which changes no planned input.
#[derive(Debug)]
pub(super) struct RetainedApplyReadGuardBinding {
    pub(super) root: Arc<RetainedDirectoryCapability>,
    pub(super) relative_path: PathBuf,
    /// Physical identity of each directory between the root and the file.
    pub(super) route_identities: Vec<crate::platform::filesystem::FileIdentity>,
    pub(super) file_identity: crate::platform::filesystem::FileIdentity,
    pub(super) expected_len: u64,
    pub(super) expected_sha256: [u8; 32],
    pub(super) deadline: unica_work_control::provider_deadline::ProviderDeadline,
    pub(super) cancellation: unica_work_control::cancellation::CancellationToken,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlannedChangeKind {
    Create,
    Update,
    Remove,
}

impl CompileTransaction {
    pub fn saved_source_images(&self) -> Vec<unica_source_contract::read::SourceImage<'_>> {
        let end = self
            .retained_apply_cache_start
            .unwrap_or(self.retained_apply.len());
        self.retained_apply[..end]
            .iter()
            .map(|change| {
                let (path_encoding, path) =
                    crate::platform::filesystem::encode_relative_source_path(&change.relative_path)
                        .into_parts();
                unica_source_contract::read::SourceImage {
                    path_encoding,
                    path,
                    before: change.original.as_deref(),
                    after: change.current.as_deref(),
                }
            })
            .collect()
    }

    pub fn new() -> Self {
        Self::default()
    }

    pub(super) fn bind_retained_apply_authority(
        &mut self,
        authority: &ApplyWriterAuthority,
    ) -> Result<(), String> {
        match &self.retained_apply_authority {
            Some(bound) if bound != authority => {
                Err("retained apply plans cannot mix actor authorities".to_string())
            }
            Some(_) => Ok(()),
            None => {
                self.retained_apply_authority = Some(authority.clone());
                Ok(())
            }
        }
    }

    pub(super) fn bind_retained_apply_root(
        &mut self,
        root: Arc<RetainedDirectoryCapability>,
        authority: &ApplyWriterAuthority,
    ) -> Result<(), String> {
        self.bind_retained_apply_authority(authority)?;
        match &self.retained_apply_root {
            Some(bound) if bound.identity() != root.identity() => {
                Err("one retained apply transaction cannot mix source roots".to_string())
            }
            Some(_) => Ok(()),
            None => {
                self.retained_apply_root = Some(root);
                Ok(())
            }
        }
    }

    pub(super) fn bind_retained_namespaces(
        &mut self,
        guards: Vec<super::apply::RetainedNamespaceGuard>,
    ) {
        self.retained_namespaces = guards;
    }

    pub(super) fn bind_retained_apply_read_guards(
        &mut self,
        guards: Vec<RetainedApplyReadGuardBinding>,
    ) {
        self.retained_apply_read_guards = guards;
    }

    pub(super) fn bind_retained_path_spellings(
        &mut self,
        guards: Vec<super::apply::RetainedPathSpellingGuard>,
    ) {
        self.retained_path_spellings = guards;
    }

    fn validate_retained_path_spellings(&self) -> Result<(), RetainedApplyValidationError> {
        for guard in &self.retained_path_spellings {
            guard.validate().map_err(|error| {
                retained_guard_validation_error(
                    error,
                    guard.relative_path(),
                    ApplyInputRole::PathSpelling,
                )
            })?;
        }
        Ok(())
    }

    fn validate_retained_namespaces(
        &self,
        deltas: &[(PathBuf, Option<u8>)],
    ) -> Result<(), RetainedApplyValidationError> {
        for guard in &self.retained_namespaces {
            guard.validate(deltas).map_err(|error| {
                retained_guard_validation_error(
                    error,
                    guard.relative_path(),
                    ApplyInputRole::DirectoryMembership,
                )
            })?;
        }
        Ok(())
    }

    fn validate_published_namespaces(
        &self,
        published: &[PublishedRetainedApplyChange],
        created: &[CreatedRetainedApplyDirectory],
        removed: &[super::retained_directory::PublishedDirectoryRemoval],
    ) -> Result<(), RetainedApplyValidationError> {
        let Some(root) = &self.retained_apply_root else {
            return Ok(());
        };
        let mut deltas = Vec::new();
        for entry in &self.retained_apply[..self
            .retained_apply_cache_start
            .unwrap_or(self.retained_apply.len())]
        {
            if entry.original != entry.current {
                deltas.push((
                    entry.relative_path.clone(),
                    entry.current.as_ref().map(|_| 2),
                ));
            }
        }
        for directory in created {
            if directory.source_participant {
                deltas.push((directory.logical_path.clone(), Some(1)));
            }
        }
        for change in published {
            if let Some(name) = &change.recovery_name {
                if let Ok(parent) = change.parent.path().strip_prefix(root.path()) {
                    deltas.push((parent.join(name), Some(2)));
                }
            }
        }
        for change in removed {
            deltas.push((change.plan.relative.clone(), None));
            deltas.push((
                change
                    .plan
                    .relative
                    .parent()
                    .expect("directory parent")
                    .join(&change.recovery_name),
                Some(1),
            ));
        }
        for guard in &self.retained_namespaces {
            let relocated = removed
                .iter()
                .find(|change| change.plan.contains(guard.relative_path()));
            let result = match relocated {
                Some(change) => guard.validate_relocated(
                    Arc::new(change.recovery.clone()),
                    guard
                        .relative_path()
                        .strip_prefix(&change.plan.relative)
                        .expect("contained namespace")
                        .to_path_buf(),
                ),
                None => guard.validate(&deltas),
            };
            result.map_err(|error| {
                retained_guard_validation_error(
                    error,
                    guard.relative_path(),
                    ApplyInputRole::DirectoryMembership,
                )
            })?;
        }
        Ok(())
    }

    pub(super) fn bind_retained_apply_change(
        &mut self,
        binding: RetainedApplyChangeBinding,
        authority: &ApplyWriterAuthority,
    ) -> Result<(), String> {
        let RetainedApplyChangeBinding {
            root,
            relative_path,
            route_identities,
            missing_parent_chain,
            name,
            original,
            current,
            original_file_identity,
            original_len,
            original_sha256,
            deadline,
            cancellation,
        } = binding;
        validate_strict_relative_file_path(&relative_path)?;
        let fingerprint_is_complete =
            original_file_identity.is_some() && original_len.is_some() && original_sha256.is_some();
        if original.is_some() != fingerprint_is_complete {
            return Err("retained apply preimage bytes and fingerprint disagree".to_string());
        }
        self.bind_retained_apply_root(Arc::clone(&root), authority)?;
        if self.retained_apply.iter().any(|entry| {
            entry.root.identity() == root.identity() && entry.relative_path == relative_path
        }) {
            return Err(format!(
                "duplicate retained apply target: {}",
                relative_path.display()
            ));
        }
        if self
            .retained_apply
            .iter()
            .any(|entry| entry.root.identity() != root.identity())
        {
            return Err("one retained apply transaction cannot mix source roots".to_string());
        }
        self.retained_apply.push(PlannedRetainedApplyChange {
            root,
            relative_path,
            route_identities,
            missing_parent_chain,
            name,
            original,
            current,
            original_file_identity,
            original_len,
            original_sha256,
            deadline,
            cancellation,
        });
        self.retained_apply
            .sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
        Ok(())
    }

    pub(crate) fn close_with_workspace_cache_participant(
        mut self,
        cache: CompileTransaction,
        participant: &WorkspaceCacheParticipantAuthority,
    ) -> Result<Self, String> {
        let authority = participant.writer_authority();
        if self.retained_apply_cache_start.is_some() || self.retained_apply_cache_root.is_some() {
            return Err(
                "retained apply transaction already has a workspace-cache participant".to_string(),
            );
        }
        if cache.retained_apply_cache_start.is_some() || cache.retained_apply_cache_root.is_some() {
            return Err(
                "workspace-cache participant must be an unclosed retained transaction".to_string(),
            );
        }
        if self.retained_apply_authority.as_ref() != Some(authority)
            || cache.retained_apply_authority.as_ref() != Some(authority)
        {
            return Err(
                "source and workspace-cache participants require one actor authority".to_string(),
            );
        }
        let source_root = self
            .retained_apply_root
            .as_ref()
            .ok_or_else(|| "source participant requires one explicit retained root".to_string())?;
        let cache_root = cache.retained_apply_root.as_ref().ok_or_else(|| {
            "workspace-cache participant requires one explicit retained root".to_string()
        })?;
        if cache_root.identity() != participant.anchor_identity() {
            return Err(
                "workspace-cache participant root does not match actor-captured authority"
                    .to_string(),
            );
        }
        if source_root.identity() == cache_root.identity() {
            let relative_cache = participant
                .logical_root()
                .strip_prefix(source_root.path())
                .map_err(|_| {
                    "source and workspace-cache participants physically alias".to_string()
                })?;
            if relative_cache.as_os_str().is_empty()
                || self.retained_apply.iter().any(|entry| {
                    entry.relative_path.starts_with(relative_cache)
                        || relative_cache.starts_with(&entry.relative_path)
                })
                || cache
                    .retained_apply
                    .iter()
                    .any(|entry| !entry.relative_path.starts_with(relative_cache))
            {
                return Err(
                    "source and workspace-cache participant targets physically overlap".to_string(),
                );
            }
        }
        self.validate_directory_cache_boundary(&cache, participant)?;
        let cache_start = self.retained_apply.len();
        self.retained_apply.extend(cache.retained_apply);
        self.retained_apply_read_guards
            .extend(cache.retained_apply_read_guards);
        self.retained_path_spellings
            .extend(cache.retained_path_spellings);
        self.retained_apply_cache_root = cache.retained_apply_root;
        self.retained_apply_cache_start = Some(cache_start);
        Ok(self)
    }

    pub(crate) fn validate_retained_for_apply_typed(
        &self,
    ) -> Result<(), RetainedApplyValidationError> {
        self.validate_retained_for_apply_and_bind().map(drop)
    }

    fn validate_retained_for_apply_and_bind(
        &self,
    ) -> Result<Vec<RetainedApplyExecutionBinding>, RetainedApplyValidationError> {
        #[cfg(any(test, feature = "test-support"))]
        if TEST_RETAINED_APPLY_VALIDATION_PROVIDER_FAILURE.with(|slot| slot.replace(false)) {
            return Err(RetainedApplyValidationError::new(
                RetainedApplyValidationErrorKind::UnsupportedProvider,
                "injected retained apply provider validation failure",
            ));
        }
        self.validate_retained_path_spellings()?;
        self.validate_retained_namespaces(&[])?;
        self.validate_retained_apply_read_guards()?;
        self.validate_directory_plans()?;
        if self.retained_apply.is_empty() {
            return Ok(Vec::new());
        }
        if !self.creates.is_empty()
            || !self.registrations.is_empty()
            || !self.read_guards.is_empty()
            || !self.absence_guards.is_empty()
            || !self.directory_membership_guards.is_empty()
            || !self.removals.is_empty()
        {
            return Err(RetainedApplyValidationError::new(
                RetainedApplyValidationErrorKind::Invariant,
                "retained apply entries cannot mix with ambient transaction entries",
            ));
        }
        let mut bindings = Vec::with_capacity(self.retained_apply.len());
        for (index, entry) in self.retained_apply.iter().enumerate() {
            let binding = retain_retained_apply_execution_binding(entry).map_err(|error| {
                error.with_input(ApplyInputContext::for_entry(
                    entry,
                    self.retained_apply_cache_start
                        .is_none_or(|start| index < start),
                ))
            })?;
            if let Some(bytes) = &entry.current {
                validate_xml_when_applicable(&entry.root.path().join(&entry.relative_path), bytes)
                    .map_err(|error| {
                        RetainedApplyValidationError::new(
                            RetainedApplyValidationErrorKind::Invariant,
                            error,
                        )
                    })?;
            }
            // A directory publication carries these originals as one retained
            // tree. Do not keep descendant handles open across its rename.
            bindings.push(if self.directory_contains(entry) {
                RetainedApplyExecutionBinding {
                    ancestor: entry.root.as_ref().clone(),
                    original_file: None,
                }
            } else {
                binding
            });
        }
        Ok(bindings)
    }

    fn validate_retained_apply_read_guards(&self) -> Result<(), RetainedApplyValidationError> {
        for guard in &self.retained_apply_read_guards {
            validate_retained_apply_read_guard(guard).map_err(|error| {
                error.with_input(ApplyInputContext {
                    relative_path: guard.relative_path.clone(),
                    role: ApplyInputRole::ReadOnly,
                })
            })?;
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn commit_retained_apply_with<T>(
        self,
        authority: ApplyWriterAuthority,
        mut checkpoint: impl FnMut() -> Result<(), String>,
        post_validation: impl FnOnce() -> Result<T, String>,
    ) -> Result<(CommitReport, T), String> {
        if self.retained_apply_authority.as_ref() != Some(&authority) {
            return Err("retained apply commit requires its actor-issued authority".to_string());
        }
        let execution_bindings = self
            .validate_retained_for_apply_and_bind()
            .map_err(|error| error.to_string())?;
        checkpoint()?;
        let mut published = Vec::with_capacity(self.retained_apply.len());
        let mut created_directories = Vec::new();
        let mut removed_directories = Vec::new();
        let operation = (|| {
            self.publish_directories(&mut removed_directories)
                .map_err(|error| error.to_string())?;
            for (_index, (entry, binding)) in self
                .retained_apply
                .iter()
                .zip(&execution_bindings)
                .enumerate()
            {
                checkpoint()?;
                if self.directory_contains(entry) {
                    continue;
                }
                publish_retained_apply_change(
                    entry,
                    binding,
                    &mut published,
                    &mut created_directories,
                    self.retained_apply_cache_start
                        .is_none_or(|start| _index < start),
                )
                .map_err(|error| error.to_string())?;
                #[cfg(any(test, feature = "test-support"))]
                RETAINED_APPLY_OBSERVED_EVENTS.with(|events| {
                    let event = if self
                        .retained_apply_cache_start
                        .is_some_and(|cache_start| _index >= cache_start)
                    {
                        if entry.relative_path.ends_with("state.json") {
                            RetainedApplyObservedEvent::StateMarker(entry.relative_path.clone())
                        } else {
                            RetainedApplyObservedEvent::EagerMetadata(entry.relative_path.clone())
                        }
                    } else {
                        RetainedApplyObservedEvent::Source(entry.relative_path.clone())
                    };
                    events.borrow_mut().push(event);
                });
            }
            for (entry, binding) in self.retained_apply.iter().zip(&execution_bindings) {
                checkpoint()?;
                if self.directory_contains(entry) {
                    continue;
                }
                validate_retained_apply_postimage(
                    entry,
                    binding,
                    &entry.current,
                    &created_directories,
                )
                .map_err(|error| error.to_string())?;
            }
            #[cfg(any(test, feature = "test-support"))]
            run_retained_apply_before_post_validation_hook();
            self.validate_retained_path_spellings()
                .map_err(|error| error.to_string())?;
            self.validate_published_namespaces(
                &published,
                &created_directories,
                &removed_directories,
            )
            .map_err(|error| error.to_string())?;
            self.validate_retained_apply_read_guards()
                .map_err(|error| error.to_string())?;
            Self::validate_removed_directories(&removed_directories)
                .map_err(|error| error.to_string())?;
            let validated = post_validation()?;
            let mut report = CommitReport::default();
            finalize_retained_apply_recovery(&mut published, &mut report);
            Self::finalize_removed_directories(&removed_directories, &mut report);
            for entry in &self.retained_apply {
                let path = entry.root.path().join(&entry.relative_path);
                match (&entry.original, &entry.current) {
                    (None, Some(_)) => report.created.push(path),
                    (Some(before), Some(after)) if before != after => report.updated.push(path),
                    (Some(_), Some(_)) => {}
                    (Some(_), None) | (None, None) => {}
                }
            }
            Ok((report, validated))
        })();
        match operation {
            Ok(result) => Ok(result),
            Err(primary) => {
                #[cfg(any(test, feature = "test-support"))]
                run_retained_apply_before_rollback_hook();
                let mut rollback_errors =
                    rollback_retained_apply(&mut published, &mut created_directories);
                Self::rollback_removed_directories(&removed_directories, &mut rollback_errors);
                if rollback_errors.is_empty() {
                    Err(primary)
                } else {
                    Err(format!(
                        "{primary}; retained apply rollback encountered: {}",
                        rollback_errors.join("; ")
                    ))
                }
            }
        }
    }

    pub(crate) fn commit_retained_apply<R, C>(
        self,
        authority: ApplyWriterAuthority,
        final_gate: RetainedApplyFinalGate<'_, R, C>,
    ) -> Result<CommitReport, ApplyPublicationError> {
        if self.retained_apply_authority.as_ref() != Some(&authority) {
            return Err(ApplyPublicationError::new(
                ApplyPublicationErrorKind::Invariant,
                "retained apply commit requires its actor-issued authority",
            ));
        }
        let cache_start = self.retained_apply_cache_start.ok_or_else(|| {
            ApplyPublicationError::new(
                ApplyPublicationErrorKind::Invariant,
                "retained apply commit requires a closed workspace-cache participant",
            )
        })?;
        let execution_bindings = self
            .validate_retained_for_apply_and_bind()
            .map_err(retained_apply_validation_publication_error)?;
        final_gate.checkpoint("prepared apply commit")?;
        let mut published = Vec::with_capacity(self.retained_apply.len());
        let mut created_directories = Vec::new();
        let mut removed_directories = Vec::new();
        let operation = (|| {
            self.publish_directories(&mut removed_directories)?;
            for (entry, binding) in self.retained_apply[..cache_start]
                .iter()
                .zip(&execution_bindings[..cache_start])
            {
                final_gate.checkpoint("prepared apply source publication")?;
                if self.directory_contains(entry) {
                    continue;
                }
                publish_retained_apply_change(
                    entry,
                    binding,
                    &mut published,
                    &mut created_directories,
                    true,
                )
                .map_err(retained_apply_publish_publication_error)?;
                #[cfg(any(test, feature = "test-support"))]
                {
                    let event = RetainedApplyObservedEvent::Source(entry.relative_path.clone());
                    RETAINED_APPLY_OBSERVED_EVENTS
                        .with(|events| events.borrow_mut().push(event.clone()));
                    retained_apply_fail_after(&event).map_err(|error| {
                        ApplyPublicationError::new(
                            ApplyPublicationErrorKind::ProviderPostvalidation,
                            error,
                        )
                    })?;
                }
            }
            #[cfg(any(test, feature = "test-support"))]
            run_retained_apply_before_revision_validation_hook();
            let root = self.retained_apply_root.as_deref().ok_or_else(|| {
                ApplyPublicationError::new(
                    ApplyPublicationErrorKind::Invariant,
                    "retained apply source root is unavailable",
                )
            })?;
            validate_journal_recoveries(root, &published)?;
            Self::validate_removed_directories(&removed_directories)?;
            for (entry, binding) in self.retained_apply[cache_start..]
                .iter()
                .zip(&execution_bindings[cache_start..])
            {
                final_gate.checkpoint("prepared apply cache publication")?;
                publish_retained_apply_change(
                    entry,
                    binding,
                    &mut published,
                    &mut created_directories,
                    false,
                )
                .map_err(retained_apply_publish_publication_error)?;
                #[cfg(any(test, feature = "test-support"))]
                {
                    let event = if entry.relative_path.ends_with("state.json") {
                        RetainedApplyObservedEvent::StateMarker(entry.relative_path.clone())
                    } else {
                        RetainedApplyObservedEvent::EagerMetadata(entry.relative_path.clone())
                    };
                    RETAINED_APPLY_OBSERVED_EVENTS
                        .with(|events| events.borrow_mut().push(event.clone()));
                    retained_apply_fail_after(&event).map_err(|error| {
                        ApplyPublicationError::new(
                            ApplyPublicationErrorKind::ProviderPostvalidation,
                            error,
                        )
                    })?;
                }
            }
            #[cfg(any(test, feature = "test-support"))]
            run_retained_apply_before_postimages_hook();
            for (index, (entry, binding)) in self
                .retained_apply
                .iter()
                .zip(&execution_bindings)
                .enumerate()
            {
                final_gate.checkpoint("prepared apply postimage validation")?;
                if self.directory_contains(entry) {
                    continue;
                }
                validate_retained_apply_postimage(
                    entry,
                    binding,
                    &entry.current,
                    &created_directories,
                )
                .map_err(|error| {
                    error.with_input(ApplyInputContext::for_entry(entry, index < cache_start))
                })
                .map_err(retained_apply_publish_publication_error)?;
            }
            #[cfg(any(test, feature = "test-support"))]
            retained_apply_fail_after_all_postimages().map_err(|error| {
                ApplyPublicationError::new(ApplyPublicationErrorKind::ProviderPostvalidation, error)
            })?;
            #[cfg(any(test, feature = "test-support"))]
            run_retained_apply_before_post_validation_hook();
            let published_replacements = self.retained_apply[..cache_start]
                .iter()
                .filter_map(|entry| match (&entry.original, &entry.current) {
                    (Some(original), Some(current)) if original != current => Some((
                        entry.root.path().join(&entry.relative_path),
                        current.clone(),
                    )),
                    _ => None,
                })
                .collect::<Vec<_>>();
            self.validate_retained_path_spellings()
                .map_err(retained_apply_validation_publication_error)?;
            self.validate_published_namespaces(
                &published,
                &created_directories,
                &removed_directories,
            )
            .map_err(retained_apply_validation_publication_error)?;
            self.validate_retained_apply_read_guards()
                .map_err(retained_apply_validation_publication_error)?;
            Self::validate_removed_directories(&removed_directories)?;
            final_gate.validate_after_publication(&published_replacements)?;
            let mut report = CommitReport::default();
            finalize_retained_apply_recovery(&mut published, &mut report);
            Self::finalize_removed_directories(&removed_directories, &mut report);
            for entry in &self.retained_apply {
                let path = entry.root.path().join(&entry.relative_path);
                match (&entry.original, &entry.current) {
                    (None, Some(_)) => report.created.push(path),
                    (Some(before), Some(after)) if before != after => report.updated.push(path),
                    (Some(_), Some(_)) => {}
                    (Some(_), None) | (None, None) => {}
                }
            }
            Ok(report)
        })();
        match operation {
            Ok(result) => Ok(result),
            Err(primary) => {
                #[cfg(any(test, feature = "test-support"))]
                run_retained_apply_before_rollback_hook();
                let mut rollback_errors =
                    rollback_retained_apply(&mut published, &mut created_directories);
                Self::rollback_removed_directories(&removed_directories, &mut rollback_errors);
                if rollback_errors.is_empty() {
                    Err(primary)
                } else {
                    Err(ApplyPublicationError::new(
                        ApplyPublicationErrorKind::RollbackIncomplete,
                        format!(
                            "{primary}; retained apply rollback encountered: {}",
                            rollback_errors.join("; ")
                        ),
                    ))
                }
            }
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn retained_planned_change_count_for_test(&self) -> usize {
        self.retained_apply.len()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn retained_role_root_counts_for_test(&self) -> (usize, usize) {
        (
            usize::from(self.retained_apply_root.is_some()),
            usize::from(self.retained_apply_cache_root.is_some()),
        )
    }

    /// Whether an existing exact plan already protects this path with the
    /// transaction lock/preimage machinery. Callers use this to avoid adding
    /// a read guard for an owner that the same transaction replaces/removes.
    pub fn protects_path(&self, path: &Path) -> Result<bool, String> {
        let identity = normalize_transaction_path_identity(path)?;
        Ok(find_overlapping_planned_identity(&self.planned_path_identities, &identity).is_some())
    }

    /// Plan a create-only file with exact bytes.
    pub fn create_bytes(
        &mut self,
        path: impl Into<PathBuf>,
        bytes: impl Into<Vec<u8>>,
    ) -> Result<(), String> {
        let path = path.into();
        let identity = self.reject_duplicate_plan_path(&path)?;
        match fs::symlink_metadata(&path) {
            Ok(metadata) => {
                let kind = if metadata.file_type().is_symlink() {
                    "symbolic link"
                } else if metadata.is_dir() {
                    "directory"
                } else {
                    "existing file"
                };
                return Err(format!(
                    "create-only compile target is already a {kind}: {}",
                    path.display()
                ));
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "failed to inspect create-only target {}: {error}",
                    path.display()
                ));
            }
        }
        self.record_planned_path(identity.clone(), PlannedPathKind::Create);
        self.creates.push(PlannedCreate {
            path,
            identity,
            bytes: bytes.into(),
        });
        Ok(())
    }

    /// Plan a create-only UTF-8 file without adding a BOM.
    #[allow(dead_code)]
    pub fn create_text(
        &mut self,
        path: impl Into<PathBuf>,
        text: impl AsRef<str>,
    ) -> Result<(), String> {
        self.create_bytes(path, text.as_ref().as_bytes().to_vec())
    }

    /// Plan a create-only UTF-8 file with exactly one leading BOM.
    pub fn create_utf8_bom_text(
        &mut self,
        path: impl Into<PathBuf>,
        text: impl AsRef<str>,
    ) -> Result<(), String> {
        let text = text.as_ref().trim_start_matches('\u{feff}');
        let mut bytes = Vec::with_capacity(UTF8_BOM.len() + text.len());
        bytes.extend_from_slice(UTF8_BOM);
        bytes.extend_from_slice(text.as_bytes());
        self.create_bytes(path, bytes)
    }

    /// Require an existing owner to retain one exact byte preimage while this
    /// transaction publishes other paths. The owner joins the transaction's
    /// publication lock set but is never staged, replaced, or rolled back.
    pub fn guard_exact_preimage(
        &mut self,
        path: impl Into<PathBuf>,
        expected_preimage: impl AsRef<[u8]>,
    ) -> Result<(), String> {
        let path = path.into();
        let identity = self.reject_duplicate_plan_path(&path)?;
        let expected_preimage = expected_preimage.as_ref().to_vec();
        validate_exact_read_guard(&path, &expected_preimage, "while planning")?;
        self.read_guards.insert(
            identity.clone(),
            PlannedReadGuard {
                path,
                expected_preimage,
            },
        );
        self.record_planned_path(identity, PlannedPathKind::ReadGuard);
        Ok(())
    }

    /// Require one exact normalized path to remain absent for the complete
    /// transaction. This binds plans that intentionally tolerate a missing
    /// semantic owner so a concurrently appearing owner cannot be left
    /// unregistered.
    pub fn guard_path_absent(&mut self, path: impl Into<PathBuf>) -> Result<(), String> {
        let requested_path = path.into();
        let identity = normalize_transaction_path_identity(&requested_path)?;
        if self.absence_guards.contains(&identity) {
            return validate_absence_guard(&identity, "while planning");
        }
        self.reject_duplicate_normalized_plan_path(&identity, &requested_path)?;
        validate_absence_guard(&identity, "while planning")?;
        self.absence_guards.insert(identity.clone());
        self.record_planned_path(identity, PlannedPathKind::AbsenceGuard);
        Ok(())
    }

    /// Bind the exact existence state and relevant direct-child listing of a
    /// directory to this transaction. Membership guards conservatively take
    /// the global publication-tree lock exclusively at commit, so another
    /// cooperating Unica writer cannot create/delete the directory or add or
    /// remove a matching entry between checks.
    pub fn guard_or_verify_directory_membership(
        &mut self,
        directory: impl Into<PathBuf>,
        selector: DirectoryMembershipSelector,
        expected: DirectoryMembershipSnapshot,
    ) -> Result<(), String> {
        self.guard_or_verify_directory_entries(directory.into(), selector, expected)
    }

    /// Bind the exact directory-existence state plus direct-child names and
    /// filesystem kinds observed by a recursive scanner. File-to-directory
    /// replacement must be detected even when the entry name itself is
    /// unchanged, because the replacement can introduce an unscanned subtree.
    pub fn guard_or_verify_directory_topology(
        &mut self,
        directory: impl Into<PathBuf>,
        expected: DirectoryMembershipSnapshot,
    ) -> Result<(), String> {
        self.guard_or_verify_directory_entries(
            directory.into(),
            DirectoryMembershipSelector::AllDirectEntries,
            expected,
        )
    }

    fn guard_or_verify_directory_entries(
        &mut self,
        requested_directory: PathBuf,
        selector: DirectoryMembershipSelector,
        expected: DirectoryMembershipSnapshot,
    ) -> Result<(), String> {
        let expected = normalize_directory_membership_snapshot(selector, expected)?;
        validate_directory_membership_guard(
            &requested_directory,
            selector,
            &expected,
            "while planning",
        )?;
        let directory = normalize_transaction_path_identity(&requested_directory)?;
        if let Some(existing) = self.directory_membership_guards.get(&directory) {
            if existing.selector == selector && existing.expected == expected {
                return Ok(());
            }
            return Err(format!(
                "directory membership changed while planning: {}",
                requested_directory.display()
            ));
        }
        for (removal, kind) in &self.planned_path_identities {
            if *kind == PlannedPathKind::Removal && directory.starts_with(removal) {
                return Err(format!(
                    "compile transaction cannot guard membership of a directory scheduled for removal: {}",
                    requested_directory.display()
                ));
            }
        }
        self.directory_membership_guards.insert(
            directory.clone(),
            DirectoryMembershipGuard {
                directory,
                selector,
                expected,
            },
        );
        Ok(())
    }

    /// Bind bytes that were read before another plan entry for the same file
    /// was created. A registration already carries an exact preimage, so it
    /// satisfies this guard only when it was derived from those same bytes.
    pub fn guard_or_verify_exact_preimage(
        &mut self,
        path: impl Into<PathBuf>,
        expected_preimage: impl AsRef<[u8]>,
    ) -> Result<(), String> {
        let path = path.into();
        let expected_preimage = expected_preimage.as_ref();
        let identity = normalize_transaction_path_identity(&path)?;
        if self.planned_path_identities.get(&identity) == Some(&PlannedPathKind::Registration) {
            let registration = self
                .registrations
                .get(&identity)
                .expect("registered path identity must retain its registration");
            if registration.original == expected_preimage {
                return Ok(());
            }
            return Err(format!(
                "protected path changed while planning: {}",
                path.display()
            ));
        }
        if self.planned_path_identities.get(&identity) == Some(&PlannedPathKind::ReadGuard) {
            let guarded = self
                .read_guards
                .get(&identity)
                .expect("guarded path identity must retain its read guard");
            if guarded.expected_preimage == expected_preimage {
                return Ok(());
            }
            return Err(format!(
                "protected path changed while planning: {}",
                path.display()
            ));
        }
        if matches!(
            self.planned_path_identities.get(&identity),
            Some(PlannedPathKind::Create | PlannedPathKind::AbsenceGuard)
        ) {
            return validate_absence_guard(&identity, "while planning");
        }
        // An overlap is reported in both directions, and only one of them can
        // answer this guard: a removal that *contains* the path snapshotted it
        // and carries its exact bytes. A removal planned below the path is an
        // overlapping plan instead, and falls through to be rejected as one.
        if let Some((removal_identity, PlannedPathKind::Removal)) =
            find_overlapping_planned_identity(&self.planned_path_identities, &identity)
        {
            if let Ok(relative_path) = identity.strip_prefix(removal_identity) {
                let removal = self
                    .removals
                    .iter()
                    .find(|removal| removal.identity == *removal_identity)
                    .expect("planned removal identity must retain its snapshot");
                let expected_size = u64::try_from(expected_preimage.len())
                    .map_err(|_| format!("protected path is too large: {}", path.display()))?;
                let expected_sha256: [u8; 32] = Sha256::digest(expected_preimage).into();
                let matches_snapshot = removal.snapshot.entries.iter().any(|entry| {
                    entry.relative_path == relative_path
                        && matches!(
                            entry.kind,
                            RemovalEntryKind::File { size, sha256 }
                                if size == expected_size && sha256 == expected_sha256
                        )
                });
                if matches_snapshot {
                    return Ok(());
                }
                return Err(format!(
                    "protected path changed while planning: {}",
                    path.display()
                ));
            }
        }
        self.reject_duplicate_normalized_plan_path(&identity, &path)?;
        validate_exact_read_guard(&path, expected_preimage, "while planning")?;
        self.read_guards.insert(
            identity.clone(),
            PlannedReadGuard {
                path,
                expected_preimage: expected_preimage.to_vec(),
            },
        );
        self.record_planned_path(identity, PlannedPathKind::ReadGuard);
        Ok(())
    }

    /// Plan creation or replacement according to the target state observed
    /// now. Existing bytes become the exact preimage required at commit.
    pub fn create_or_replace_bytes(
        &mut self,
        path: impl Into<PathBuf>,
        replacement: impl Into<Vec<u8>>,
    ) -> Result<(), String> {
        let path = path.into();
        let replacement = replacement.into();
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == ErrorKind::NotFound => {
                self.create_bytes(path, replacement)
            }
            Err(error) => Err(format!(
                "failed to inspect publication target {}: {error}",
                path.display()
            )),
            Ok(metadata) => {
                if metadata_is_link_or_reparse_point(&metadata) {
                    return Err(format!(
                        "publication target must not be a symbolic link or reparse point: {}",
                        path.display()
                    ));
                }
                if !metadata.is_file() {
                    return Err(format!(
                        "publication target is not a regular file: {}",
                        path.display()
                    ));
                }
                let original = fs::read(&path).map_err(|error| {
                    format!(
                        "failed to read publication target {}: {error}",
                        path.display()
                    )
                })?;
                self.replace_bytes(path, &original, replacement)
            }
        }
    }

    /// Plan an exact replacement of an existing regular file. The caller's
    /// preimage is rechecked both while planning and immediately before the
    /// atomic publication, so a concurrent edit is never overwritten.
    pub fn replace_bytes(
        &mut self,
        path: impl Into<PathBuf>,
        expected_preimage: impl AsRef<[u8]>,
        replacement: impl Into<Vec<u8>>,
    ) -> Result<(), String> {
        self.replace_bytes_classified(path, expected_preimage, replacement)
            .map_err(CommitFailure::into_message)
    }

    /// Plan an exact replacement while retaining the typed reason a target
    /// cannot be bound. Logical mutation APIs use this form to distinguish a
    /// stale/missing/link-swapped preimage from an unavailable provider.
    pub fn replace_bytes_classified(
        &mut self,
        path: impl Into<PathBuf>,
        expected_preimage: impl AsRef<[u8]>,
        replacement: impl Into<Vec<u8>>,
    ) -> Result<(), CommitFailure> {
        let path = path.into();
        let identity = self
            .reject_duplicate_plan_path(&path)
            .map_err(CommitFailure::provider)?;
        let original = expected_preimage.as_ref().to_vec();
        super::single_file_publisher::validate_replace_target(
            &path,
            &original,
            PublishPhase::Inspect,
        )
        .map_err(|error| adapt_publish_error(&error, PublicationRole::Registration))?;
        let updated = replacement.into();
        validate_xml_when_applicable(&path, &updated).map_err(CommitFailure::provider)?;
        self.registrations.insert(
            identity.clone(),
            PlannedRegistration {
                path,
                original,
                updated,
            },
        );
        self.record_planned_path(identity, PlannedPathKind::Registration);
        Ok(())
    }

    /// Plan removal of one regular file or directory tree. The complete tree
    /// is snapshotted before commit and moved to a same-filesystem recovery
    /// location during publication, allowing byte-exact rollback.
    pub fn remove_path(&mut self, path: impl Into<PathBuf>) -> Result<(), String> {
        let path = path.into();
        let removal_identity = self.reject_duplicate_plan_path(&path)?;
        if self
            .directory_membership_guards
            .keys()
            .any(|directory| directory.starts_with(&removal_identity))
        {
            return Err(format!(
                "compile transaction cannot remove a directory whose membership is guarded: {}",
                path.display()
            ));
        }
        let snapshot = snapshot_removal_path(&path)?;
        self.record_planned_path(removal_identity.clone(), PlannedPathKind::Removal);
        self.removals.push(PlannedRemoval {
            path,
            identity: removal_identity,
            snapshot,
        });
        Ok(())
    }

    /// Plan removal of a collection directory only when its exact direct-child
    /// inventory is the caller's target set. The decision is bound to the same
    /// full removal snapshot that commit rechecks, so a concurrent new sibling
    /// cannot be captured and deleted accidentally.
    pub fn remove_directory_if_only_direct_entries(
        &mut self,
        path: impl Into<PathBuf>,
        expected_names: Vec<OsString>,
    ) -> Result<bool, String> {
        let path = path.into();
        let expected_names = normalize_direct_entry_names(expected_names)?;
        if snapshot_direct_entry_names(&path)? != expected_names {
            return Ok(false);
        }

        let removal_identity = self.reject_duplicate_plan_path(&path)?;
        if self
            .directory_membership_guards
            .keys()
            .any(|directory| directory.starts_with(&removal_identity))
        {
            return Err(format!(
                "compile transaction cannot remove a directory whose membership is guarded: {}",
                path.display()
            ));
        }
        let snapshot = snapshot_removal_path(&path)?;
        if removal_snapshot_direct_entry_names(&snapshot)? != expected_names {
            return Ok(false);
        }
        self.record_planned_path(removal_identity.clone(), PlannedPathKind::Removal);
        self.removals.push(PlannedRemoval {
            path,
            identity: removal_identity,
            snapshot,
        });
        Ok(true)
    }

    /// Add one child object to an existing XML target using the canonical
    /// registrar. Multiple calls for the same target accumulate in memory and
    /// result in one replacement during commit.
    pub fn register_canonical_child(
        &mut self,
        target: impl Into<PathBuf>,
        type_name: &str,
        object_name: &str,
        registrar: fn(&mut String, &str, &str) -> Result<bool, String>,
    ) -> Result<RegistrationStatus, String> {
        let target = target.into();
        let normalized_target = normalize_transaction_path_identity(&target)?;
        if self.absence_guards.contains(&normalized_target) {
            validate_absence_guard(&normalized_target, "while planning")?;
            return Ok(RegistrationStatus::MissingTarget);
        }
        let existing_registration = self.planned_path_identities.get(&normalized_target)
            == Some(&PlannedPathKind::Registration);
        if !existing_registration {
            if let Some((_, kind)) =
                find_overlapping_planned_identity(&self.planned_path_identities, &normalized_target)
            {
                let conflict = match kind {
                    PlannedPathKind::Create => "create-only",
                    PlannedPathKind::Registration => "another registration",
                    PlannedPathKind::ReadGuard => "an exact read guard",
                    PlannedPathKind::AbsenceGuard => "an absence guard",
                    PlannedPathKind::Removal => "a removal",
                };
                return Err(format!(
                    "compile transaction path is both {conflict} and a registration target: {}",
                    target.display()
                ));
            }
        }

        if !existing_registration {
            let metadata = match fs::symlink_metadata(&target) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == ErrorKind::NotFound => {
                    validate_absence_guard(&normalized_target, "while planning")?;
                    self.absence_guards.insert(normalized_target.clone());
                    self.record_planned_path(normalized_target, PlannedPathKind::AbsenceGuard);
                    return Ok(RegistrationStatus::MissingTarget);
                }
                Err(error) => {
                    return Err(format!(
                        "failed to inspect registration target {}: {error}",
                        target.display()
                    ));
                }
            };
            if metadata.file_type().is_symlink() {
                return Err(format!(
                    "registration target must not be a symbolic link: {}",
                    target.display()
                ));
            }
            if !metadata.is_file() {
                return Err(format!(
                    "registration target is not a regular file: {}",
                    target.display()
                ));
            }
            let original = fs::read(&target).map_err(|error| {
                format!(
                    "failed to read registration target {}: {error}",
                    target.display()
                )
            })?;
            validate_xml_bytes(&target, &original)?;
            self.registrations.insert(
                normalized_target.clone(),
                PlannedRegistration {
                    path: target.clone(),
                    updated: original.clone(),
                    original,
                },
            );
            self.record_planned_path(normalized_target.clone(), PlannedPathKind::Registration);
        }

        let registration = self
            .registrations
            .get_mut(&normalized_target)
            .ok_or_else(|| format!("registration target was not cached: {}", target.display()))?;
        let (bom, payload) = unica_xml_format::text::split_utf8_bom_prefix(&registration.updated);
        let source = std::str::from_utf8(payload).map_err(|error| {
            format!(
                "registration target is not valid UTF-8 {}: {error}",
                target.display()
            )
        })?;
        let source = source.to_string();
        let mut updated = source.clone();
        let changed = registrar(&mut updated, type_name, object_name).map_err(|error| {
            format!(
                "failed to plan registration in {}: {error}",
                target.display()
            )
        })?;
        if !changed {
            return Ok(RegistrationStatus::AlreadyPresent);
        }

        updated = unica_xml_format::text::preserve_inserted_line_endings(&source, &updated);
        let mut updated_bytes = Vec::with_capacity(bom.len() + updated.len());
        updated_bytes.extend_from_slice(bom);
        updated_bytes.extend_from_slice(updated.as_bytes());
        validate_xml_bytes(&target, &updated_bytes)?;
        registration.updated = updated_bytes;
        Ok(RegistrationStatus::Added)
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.retained_apply.is_empty()
            && self.retained_apply_read_guards.is_empty()
            && self.retained_path_spellings.is_empty()
            && self.creates.is_empty()
            && self.read_guards.is_empty()
            && self.absence_guards.is_empty()
            && self.directory_membership_guards.is_empty()
            && self.removals.is_empty()
            && self
                .registrations
                .values()
                .all(|registration| !registration.changed())
    }

    /// Saved evidence belongs to the plan; its stop budget belongs to execution.
    pub(crate) fn rebind_retained_apply_execution_context(
        &mut self,
        deadline: unica_work_control::provider_deadline::ProviderDeadline,
        cancellation: &unica_work_control::cancellation::CancellationToken,
    ) {
        for plan in &mut self.retained_directory_removals {
            plan.rebind(deadline, cancellation.clone());
        }
        for guard in &mut self.retained_namespaces {
            guard.rebind_execution_context(deadline, cancellation);
        }
        for guard in &mut self.retained_path_spellings {
            guard.rebind_execution_context(deadline, cancellation);
        }
        for guard in &mut self.retained_apply_read_guards {
            guard.deadline = deadline;
            guard.cancellation = cancellation.clone();
        }
        for entry in &mut self.retained_apply {
            entry.deadline = deadline;
            entry.cancellation = cancellation.clone();
        }
    }

    /// Полный план изменений транзакции: созданные, обновлённые и удаляемые
    /// пути — единственный источник для структурной квитанции мутации.
    /// Порядок детерминирован: create, update, remove; внутри —
    /// по пути.
    pub fn planned_changes(&self) -> Vec<(PlannedChangeKind, PathBuf)> {
        let mut changes = Vec::new();
        let mut creates = self.planned_created_paths();
        creates.sort();
        changes.extend(
            creates
                .into_iter()
                .map(|path| (PlannedChangeKind::Create, path)),
        );
        let mut updates = self.planned_updated_paths();
        updates.sort();
        changes.extend(
            updates
                .into_iter()
                .map(|path| (PlannedChangeKind::Update, path)),
        );
        let mut removals = self
            .removals
            .iter()
            .map(|removal| removal.path.clone())
            .collect::<Vec<_>>();
        removals.sort();
        changes.extend(
            removals
                .into_iter()
                .map(|path| (PlannedChangeKind::Remove, path)),
        );
        changes
    }

    pub fn planned_created_paths(&self) -> Vec<PathBuf> {
        self.creates.iter().map(|file| file.path.clone()).collect()
    }

    pub fn planned_updated_paths(&self) -> Vec<PathBuf> {
        self.registrations
            .values()
            .filter(|registration| registration.changed())
            .map(|registration| registration.path.clone())
            .collect()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn registration_diffs(&self) -> Vec<RegistrationDiff> {
        self.registrations
            .values()
            .filter_map(PlannedRegistration::diff)
            .collect()
    }

    /// Returns the exact in-memory post-image for one registration target.
    /// Typed metadata mutations use it to validate the same bytes that the
    /// transaction will publish, without reopening the physical owner.
    pub fn planned_registration_image(&self, path: &Path) -> Option<Vec<u8>> {
        let identity = normalize_transaction_path_identity(path).ok()?;
        self.registrations
            .get(&identity)
            .map(|registration| registration.updated.clone())
    }

    /// Stage, publish, validate, and finalize every planned change as one
    /// failure-atomic transaction for reported errors. This is not a
    /// process-crash or power-loss atomicity guarantee.
    pub fn commit(self) -> Result<CommitReport, String> {
        self.commit_with_post_validation(|| Ok(()))
    }

    /// Commit the transaction and run one caller-provided validation while
    /// every publication lock and rollback recovery is still held. A
    /// validation error rolls the complete transaction back before success is
    /// finalized.
    pub fn commit_with_post_validation<F>(self, post_validation: F) -> Result<CommitReport, String>
    where
        F: FnOnce() -> Result<(), String>,
    {
        self.commit_with_classified_post_validation(|| {
            post_validation().map_err(CommitFailure::provider)
        })
        .map_err(CommitFailure::into_message)
    }

    pub fn commit_with_classified_post_validation<F>(
        self,
        post_validation: F,
    ) -> Result<CommitReport, CommitFailure>
    where
        F: FnOnce() -> Result<(), CommitFailure>,
    {
        self.commit_with_classified_pre_and_post_validation(|| Ok(()), post_validation)
    }

    /// Recheck a caller's admission under publication locks, after staging and
    /// recovery preparation, immediately before each source mutation. A refusal
    /// discards prepared files and rolls back any earlier writes in this transaction.
    pub fn commit_with_classified_pre_and_post_validation<P, F>(
        self,
        mut pre_publication: P,
        post_validation: F,
    ) -> Result<CommitReport, CommitFailure>
    where
        P: FnMut() -> Result<(), CommitFailure>,
        F: FnOnce() -> Result<(), CommitFailure>,
    {
        if self.retained_apply_authority.is_some() || !self.retained_apply.is_empty() {
            return Err(CommitFailure::provider(
                "retained apply plans require the actor-owned commit path",
            ));
        }
        let mut state = PublishState::default();
        self.recheck_planned_path_identities("before parent preparation")
            .map_err(CommitFailure::concurrent)?;
        self.semantic_preflight().map_err(CommitFailure::provider)?;

        for create in &self.creates {
            if let Err(error) = ensure_parent_directories(&create.path, &mut state.created_dirs) {
                let cleanup_errors = cleanup_created_directories(&mut state.created_dirs);
                return Err(with_cleanup_diagnostics(
                    CommitFailure::provider(error),
                    cleanup_errors,
                ));
            }
        }
        for registration in self.registrations.values().filter(|item| item.changed()) {
            if let Err(error) =
                ensure_parent_directories(&registration.path, &mut state.created_dirs)
            {
                let cleanup_errors = cleanup_created_directories(&mut state.created_dirs);
                return Err(with_cleanup_diagnostics(
                    CommitFailure::provider(error),
                    cleanup_errors,
                ));
            }
        }

        let mut targets = self
            .creates
            .iter()
            .map(|create| create.path.clone())
            .collect::<Vec<_>>();
        targets.extend(
            self.registrations
                .values()
                .map(|registration| registration.path.clone()),
        );
        targets.extend(self.read_guards.values().map(|guard| guard.path.clone()));
        targets.extend(self.removals.iter().map(|removal| removal.path.clone()));
        let mut guard_targets = self.absence_guards.iter().cloned().collect::<Vec<_>>();
        guard_targets.extend(self.directory_membership_guards.keys().cloned());

        let mut post_validation = Some(post_validation);
        let tree_lock_mode =
            if self.removals.is_empty() && self.directory_membership_guards.is_empty() {
                PublicationTreeLockMode::Shared
            } else {
                PublicationTreeLockMode::Exclusive
            };
        match with_publication_locks_mode_and_guard_targets(
            &targets,
            &guard_targets,
            tree_lock_mode,
            |lock| self.commit_locked(lock, &mut state, &mut pre_publication, &mut post_validation),
        ) {
            Ok(result) => result,
            Err(error) => {
                let primary = adapt_publish_error(&error, PublicationRole::Transaction);
                record_publish_error_cleanup(&mut state, &error);
                let mut cleanup_errors = retry_and_finish_recovery_cleanups(&mut state);
                cleanup_errors.extend(cleanup_created_directories(&mut state.created_dirs));
                cleanup_errors.extend(std::mem::take(&mut state.cleanup_warnings));
                Err(with_cleanup_diagnostics(primary, cleanup_errors))
            }
        }
    }

    fn commit_locked<'request, 'lock, 'scope, P, F>(
        &'request self,
        lock: &'lock PublicationLockToken<'scope>,
        state: &mut PublishState,
        pre_publication: &mut P,
        post_validation: &mut Option<F>,
    ) -> Result<CommitReport, CommitFailure>
    where
        P: FnMut() -> Result<(), CommitFailure>,
        F: FnOnce() -> Result<(), CommitFailure>,
    {
        let mut prepared_creates: VecDeque<(
            &'request PlannedCreate,
            PreparedCreate<'request, 'lock, 'scope>,
        )> = VecDeque::new();
        let mut prepared_registrations: VecDeque<(
            &'request PlannedRegistration,
            PreparedReplace<'request, 'lock, 'scope>,
        )> = VecDeque::new();
        let mut prepared_removals: VecDeque<(&'request PlannedRemoval, PendingRemovalRecovery)> =
            VecDeque::new();

        let operation = (|| -> Result<CommitReport, CommitFailure> {
            self.recheck_planned_path_identities("under publication lock")
                .map_err(CommitFailure::concurrent)?;
            self.recheck_exact_read_guards("before publication")
                .map_err(CommitFailure::concurrent)?;
            self.recheck_absence_guards("before publication")
                .map_err(CommitFailure::concurrent)?;
            self.recheck_directory_membership_guards(
                false,
                &state.created_dirs,
                "before publication",
            )
            .map_err(CommitFailure::concurrent)?;

            for removal in &self.removals {
                recheck_removal(removal).map_err(CommitFailure::concurrent)?;
            }

            for create in &self.creates {
                let publication = prepare(
                    lock,
                    PublishRequest {
                        target: &create.path,
                        replacement: &create.bytes,
                        mode: PublishMode::CreateOnly,
                    },
                )
                .map_err(|error| {
                    let message = adapt_publish_error(&error, PublicationRole::Create);
                    record_publish_error_cleanup(state, &error);
                    message
                })?;
                match publication {
                    PreparedPublication::Create(prepared) => {
                        prepared_creates.push_back((create, prepared));
                    }
                    PreparedPublication::Replace(prepared) => {
                        record_cleanup_warnings(state, prepared.discard());
                        return Err(format!(
                            "create-only publication prepared an invalid state for {}",
                            create.path.display()
                        )
                        .into());
                    }
                    PreparedPublication::Unchanged => {
                        return Err(format!(
                            "create-only publication prepared an invalid state for {}",
                            create.path.display()
                        )
                        .into());
                    }
                }
            }

            for registration in self.registrations.values() {
                let publication = prepare(
                    lock,
                    PublishRequest {
                        target: &registration.path,
                        replacement: &registration.updated,
                        mode: PublishMode::ReplaceExisting {
                            expected_preimage: &registration.original,
                        },
                    },
                )
                .map_err(|error| {
                    let message = adapt_publish_error(&error, PublicationRole::Registration);
                    record_publish_error_cleanup(state, &error);
                    message
                })?;
                match publication {
                    PreparedPublication::Replace(prepared) if registration.changed() => {
                        prepared_registrations.push_back((registration, prepared));
                    }
                    PreparedPublication::Replace(prepared) => {
                        record_cleanup_warnings(state, prepared.discard());
                        return Err(format!(
                            "unchanged registration prepared a replacement for {}",
                            registration.path.display()
                        )
                        .into());
                    }
                    PreparedPublication::Create(prepared) => {
                        record_cleanup_warnings(state, prepared.discard());
                        return Err(format!(
                            "changed registration prepared an invalid state for {}",
                            registration.path.display()
                        )
                        .into());
                    }
                    PreparedPublication::Unchanged if !registration.changed() => {}
                    PreparedPublication::Unchanged => {
                        return Err(format!(
                            "changed registration prepared an invalid state for {}",
                            registration.path.display()
                        )
                        .into());
                    }
                }
            }

            for removal in &self.removals {
                prepared_removals.push_back((removal, reserve_removal_recovery(&removal.path)?));
            }

            while let Some((create, prepared)) = prepared_creates.pop_front() {
                let published_identity = match prepared.staged_file_identity() {
                    Ok(identity) => identity,
                    Err(error) => {
                        let message = adapt_publish_error(&error, PublicationRole::Create);
                        record_publish_error_cleanup(state, &error);
                        record_cleanup_warnings(state, prepared.discard());
                        return Err(message);
                    }
                };
                if let Err(error) = pre_publication() {
                    record_cleanup_warnings(state, prepared.discard());
                    return Err(error);
                }
                let report = prepared.commit().map_err(|error| {
                    let message = adapt_publish_error(&error, PublicationRole::Create);
                    record_publish_error_cleanup(state, &error);
                    message
                })?;
                record_cleanup_warnings(state, report.cleanup_warnings);
                state.published_creates.push(PublishedCreate {
                    target: create.path.clone(),
                    published: PublishedFileExpectation::new(
                        published_identity,
                        create.bytes.clone(),
                    ),
                });
            }

            failpoint_after_object_files()?;

            while let Some((registration, prepared)) = prepared_registrations.pop_front() {
                let published_identity = match prepared.staged_file_identity() {
                    Ok(identity) => identity,
                    Err(error) => {
                        let message = adapt_publish_error(&error, PublicationRole::Registration);
                        record_publish_error_cleanup(state, &error);
                        record_cleanup_warnings(state, prepared.discard());
                        return Err(message);
                    }
                };
                let permissions = prepared.portable_permissions().clone();
                let mut recovery = match reserve_recovery(&registration.path) {
                    Ok(recovery) => recovery,
                    Err(error) => {
                        record_cleanup_warnings(state, prepared.discard());
                        return Err(error.into());
                    }
                };
                let recovery_path = recovery.path();
                if let Err(error) = recovery.cleanup.directory.verify() {
                    record_cleanup_warnings(state, prepared.discard());
                    record_recovery_cleanup(state, &mut recovery);
                    return Err(error.into());
                }
                let recovery_directory = match recovery.cleanup.directory.try_clone_directory() {
                    Ok(directory) => directory,
                    Err(error) => {
                        record_cleanup_warnings(state, prepared.discard());
                        record_recovery_cleanup(state, &mut recovery);
                        return Err(error.into());
                    }
                };
                let recovery_artifact = match write_exact_new_file_in_directory(
                    &recovery_path,
                    recovery_directory,
                    recovery.cleanup.directory.identity,
                    &registration.original,
                    &permissions,
                ) {
                    Ok(artifact) => artifact,
                    Err(error) => {
                        let message = adapt_publish_error(&error, PublicationRole::Recovery);
                        record_publish_error_cleanup(state, &error);
                        record_cleanup_warnings(state, prepared.discard());
                        record_recovery_cleanup(state, &mut recovery);
                        return Err(message);
                    }
                };
                if let Err(error) = recovery.bind_artifact(recovery_artifact) {
                    record_cleanup_warnings(state, prepared.discard());
                    record_recovery_cleanup(state, &mut recovery);
                    return Err(error.into());
                }

                pause_after_registration_recovery();
                if let Err(error) = failpoint_after_registration_backup() {
                    record_cleanup_warnings(state, prepared.discard());
                    record_recovery_cleanup(state, &mut recovery);
                    return Err(error.into());
                }

                if let Err(error) = pre_publication() {
                    record_cleanup_warnings(state, prepared.discard());
                    record_recovery_cleanup(state, &mut recovery);
                    return Err(error);
                }
                let report = match prepared.commit() {
                    Ok(report) => report,
                    Err(error) => {
                        let message = adapt_publish_error(&error, PublicationRole::Registration);
                        record_publish_error_cleanup(state, &error);
                        record_recovery_cleanup(state, &mut recovery);
                        return Err(message);
                    }
                };
                record_cleanup_warnings(state, report.cleanup_warnings);
                state.published_registrations.push(recovery.into_published(
                    registration.path.clone(),
                    registration.original.clone(),
                    PublishedFileExpectation::new(published_identity, registration.updated.clone()),
                    permissions,
                ));
            }

            while let Some((removal, _)) = prepared_removals.front() {
                recheck_removal(removal).map_err(CommitFailure::concurrent)?;
                pre_publication()?;
                let (removal, recovery) = prepared_removals.pop_front().expect("nonempty removals");
                rename_no_replace(&removal.path, &recovery.path).map_err(|error| {
                    // `AlreadyExists` here means another writer took the
                    // recovery name between the preflight and this rename, and
                    // `NotFound` that the removal target itself disappeared.
                    // Both are races, and collapsing them into a provider
                    // failure would tell the caller to retry the wrong thing.
                    let message = format!(
                        "failed to move removal target {} to no-clobber recovery {}: {error}",
                        removal.path.display(),
                        recovery.path.display()
                    );
                    match error.kind() {
                        ErrorKind::AlreadyExists | ErrorKind::NotFound => {
                            CommitFailure::concurrent(message)
                        }
                        _ => CommitFailure::provider(message),
                    }
                })?;
                state
                    .published_removals
                    .push(recovery.into_published(removal.path.clone(), removal.snapshot.clone()));
                let published = state
                    .published_removals
                    .last()
                    .expect("published removal was just recorded");
                let moved_snapshot = snapshot_removal_path(&published.recovery)?;
                if moved_snapshot != removal.snapshot {
                    return Err(CommitFailure::concurrent(format!(
                        "removal target changed while moving to recovery: {}",
                        removal.path.display()
                    )));
                }
            }

            self.post_validate()?;
            let validate = post_validation.take().ok_or_else(|| {
                "compile transaction post-validation was already consumed".to_string()
            })?;
            validate()?;
            failpoint_post_write_validation()?;
            self.recheck_exact_read_guards("before successful completion")
                .map_err(CommitFailure::concurrent)?;
            self.recheck_absence_guards("before successful completion")
                .map_err(CommitFailure::concurrent)?;
            let mut transient_directories = state.created_dirs.clone();
            transient_directories.extend(
                state
                    .published_registrations
                    .iter()
                    .map(|published| published.recovery.directory_path().to_path_buf())
                    .chain(
                        state
                            .published_removals
                            .iter()
                            .map(|published| published.recovery_directory.clone()),
                    ),
            );
            self.recheck_directory_membership_guards(
                true,
                &transient_directories,
                "before successful completion",
            )
            .map_err(CommitFailure::concurrent)?;

            Ok(CommitReport {
                created: self.planned_created_paths(),
                updated: self.planned_updated_paths(),
                cleanup_warnings: Vec::new(),
                retained_apply_cleanup_diagnostics: Vec::new(),
            })
        })();

        match operation {
            Ok(mut report) => {
                debug_assert!(prepared_creates.is_empty());
                debug_assert!(prepared_registrations.is_empty());
                debug_assert!(prepared_removals.is_empty());
                finalize_success(state);
                report.cleanup_warnings = std::mem::take(&mut state.cleanup_warnings);
                Ok(report)
            }
            Err(primary) => {
                discard_prepared(state, &mut prepared_creates, &mut prepared_registrations);
                discard_prepared_removals(state, &mut prepared_removals);
                let mut rollback_errors = rollback(state);
                rollback_errors.extend(std::mem::take(&mut state.cleanup_warnings));
                Err(with_rollback_diagnostics(primary, rollback_errors))
            }
        }
    }

    fn reject_duplicate_plan_path(&self, path: &Path) -> Result<PathBuf, String> {
        let identity = normalize_transaction_path_identity(path)?;
        self.reject_duplicate_normalized_plan_path(&identity, path)?;
        Ok(identity)
    }

    fn reject_duplicate_normalized_plan_path(
        &self,
        identity: &Path,
        requested_path: &Path,
    ) -> Result<(), String> {
        if find_overlapping_planned_identity(&self.planned_path_identities, identity).is_some() {
            Err(format!(
                "compile transaction contains duplicate or overlapping path: {}",
                requested_path.display()
            ))
        } else {
            Ok(())
        }
    }

    fn record_planned_path(&mut self, identity: PathBuf, kind: PlannedPathKind) {
        let previous = self.planned_path_identities.insert(identity, kind);
        debug_assert!(
            previous.is_none(),
            "validated planned path identity must be unique"
        );
    }

    /// Предпросмотр вызывает ту же семантическую проверку плановых
    /// байтов, что применение выполняет перед публикацией.
    pub fn semantic_preflight(&self) -> Result<(), String> {
        for create in &self.creates {
            validate_xml_when_applicable(&create.path, &create.bytes)?;
        }
        for registration in self.registrations.values().filter(|item| item.changed()) {
            validate_xml_when_applicable(&registration.path, &registration.updated)?;
        }
        Ok(())
    }

    fn recheck_planned_path_identities(&self, phase: &str) -> Result<(), String> {
        for create in &self.creates {
            recheck_planned_path_identity(&create.path, &create.identity, phase)?;
        }
        for (identity, registration) in &self.registrations {
            recheck_planned_path_identity(&registration.path, identity, phase)?;
        }
        for (identity, guard) in &self.read_guards {
            recheck_planned_path_identity(&guard.path, identity, phase)?;
        }
        for removal in &self.removals {
            recheck_planned_path_identity(&removal.path, &removal.identity, phase)?;
        }
        for identity in &self.absence_guards {
            recheck_planned_path_identity(identity, identity, phase)?;
        }
        for (identity, guard) in &self.directory_membership_guards {
            recheck_planned_path_identity(&guard.directory, identity, phase)?;
        }
        Ok(())
    }

    fn recheck_exact_read_guards(&self, phase: &str) -> Result<(), String> {
        for guard in self.read_guards.values() {
            validate_exact_read_guard(&guard.path, &guard.expected_preimage, phase)?;
        }
        Ok(())
    }

    fn recheck_absence_guards(&self, phase: &str) -> Result<(), String> {
        for path in &self.absence_guards {
            validate_absence_guard(path, phase)?;
        }
        Ok(())
    }

    fn recheck_directory_membership_guards(
        &self,
        include_planned_deltas: bool,
        transient_additions: &[PathBuf],
        phase: &str,
    ) -> Result<(), String> {
        for guard in self.directory_membership_guards.values() {
            let expected = if include_planned_deltas || !transient_additions.is_empty() {
                self.directory_membership_after_planned_deltas(
                    guard,
                    include_planned_deltas,
                    transient_additions,
                )?
            } else {
                guard.expected.clone()
            };
            validate_directory_membership_guard(
                &guard.directory,
                guard.selector,
                &expected,
                phase,
            )?;
        }
        Ok(())
    }

    fn directory_membership_after_planned_deltas(
        &self,
        guard: &DirectoryMembershipGuard,
        include_planned_deltas: bool,
        transient_additions: &[PathBuf],
    ) -> Result<DirectoryMembershipSnapshot, String> {
        let (mut exists, mut expected) = match &guard.expected {
            DirectoryMembershipSnapshot::Absent => (false, BTreeMap::new()),
            DirectoryMembershipSnapshot::Present(entries) => (
                true,
                entries
                    .iter()
                    .map(|entry| (entry.name.clone(), entry.kind))
                    .collect::<BTreeMap<_, _>>(),
            ),
        };
        if include_planned_deltas {
            for create in &self.creates {
                apply_direct_membership_delta(
                    &guard.directory,
                    guard.selector,
                    &create.path,
                    true,
                    Some(DirectoryTopologyEntryKind::File),
                    &mut expected,
                )?;
            }
            for removal in &self.removals {
                apply_direct_membership_delta(
                    &guard.directory,
                    guard.selector,
                    &removal.path,
                    false,
                    None,
                    &mut expected,
                )?;
            }
        }
        for path in transient_additions {
            if normalize_transaction_path_identity(path)? == guard.directory {
                exists = true;
                continue;
            }
            apply_direct_membership_delta(
                &guard.directory,
                guard.selector,
                path,
                true,
                Some(DirectoryTopologyEntryKind::Directory),
                &mut expected,
            )?;
        }
        let entries = expected
            .into_iter()
            .map(|(name, kind)| DirectoryTopologyEntry { name, kind })
            .collect::<Vec<_>>();
        if exists {
            Ok(DirectoryMembershipSnapshot::Present(entries))
        } else if entries.is_empty() {
            Ok(DirectoryMembershipSnapshot::Absent)
        } else {
            Err(format!(
                "directory membership guard plan adds entries below an absent directory: {}",
                guard.directory.display()
            ))
        }
    }

    fn post_validate(&self) -> Result<(), String> {
        for create in &self.creates {
            validate_published_file(&create.path, &create.bytes)?;
        }
        for registration in self.registrations.values() {
            validate_published_file(&registration.path, &registration.updated)?;
            validate_xml_when_applicable(&registration.path, &registration.updated)?;
        }
        for removal in &self.removals {
            match fs::symlink_metadata(&removal.path) {
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Ok(_) => {
                    return Err(format!(
                        "post-write removal validation failed for {}",
                        removal.path.display()
                    ));
                }
                Err(error) => {
                    return Err(format!(
                        "failed to validate removed {}: {error}",
                        removal.path.display()
                    ));
                }
            }
        }
        Ok(())
    }
}

fn validate_strict_relative_file_path(relative: &Path) -> Result<(), String> {
    use std::path::Component;

    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!(
            "retained apply target must contain only normal relative components: {}",
            relative.display()
        ));
    }
    Ok(())
}

fn retained_guard_validation_error(
    error: super::apply::ApplyStagingError,
    relative_path: &Path,
    role: ApplyInputRole,
) -> RetainedApplyValidationError {
    use super::apply::ApplyStagingErrorKind as Kind;
    let kind = match error.kind() {
        Kind::Cancelled => RetainedApplyValidationErrorKind::Cancelled,
        Kind::Deadline => RetainedApplyValidationErrorKind::Deadline,
        Kind::ConcurrentRevision => RetainedApplyValidationErrorKind::ConcurrentRevision,
        Kind::ContainmentIdentity | Kind::AbsentChainOccupied => {
            RetainedApplyValidationErrorKind::ContainmentIdentity
        }
        Kind::UnsupportedProvider => RetainedApplyValidationErrorKind::UnsupportedProvider,
        Kind::Invariant => RetainedApplyValidationErrorKind::Invariant,
    };
    let error = RetainedApplyValidationError::new(kind, error.to_string());
    if matches!(
        kind,
        RetainedApplyValidationErrorKind::ConcurrentRevision
            | RetainedApplyValidationErrorKind::ContainmentIdentity
    ) {
        error.with_input(ApplyInputContext {
            relative_path: relative_path.to_owned(),
            role,
        })
    } else {
        error
    }
}

fn retained_apply_entry_checkpoint(
    entry: &PlannedRetainedApplyChange,
) -> Result<(), RetainedApplyValidationError> {
    if entry.cancellation.is_cancelled() {
        return Err(RetainedApplyValidationError::new(
            RetainedApplyValidationErrorKind::Cancelled,
            "retained apply write-target check cancelled",
        ));
    }
    if entry.deadline.is_elapsed() {
        return Err(RetainedApplyValidationError::new(
            RetainedApplyValidationErrorKind::Deadline,
            "retained apply write-target check deadline elapsed",
        ));
    }
    Ok(())
}

fn retain_retained_apply_execution_binding(
    entry: &PlannedRetainedApplyChange,
) -> Result<RetainedApplyExecutionBinding, RetainedApplyValidationError> {
    retained_apply_entry_checkpoint(entry)?;
    entry.root.validate_named_identity().map_err(|error| {
        RetainedApplyValidationError::new(
            RetainedApplyValidationErrorKind::ContainmentIdentity,
            format!("retained apply source-root physical identity changed: {error}"),
        )
    })?;
    let parent_components = entry
        .relative_path
        .parent()
        .expect("retained file path has a parent")
        .components()
        .map(|component| match component {
            std::path::Component::Normal(name) => Ok(name),
            _ => Err(RetainedApplyValidationError::new(
                RetainedApplyValidationErrorKind::Invariant,
                "retained apply parent path is not relative",
            )),
        })
        .collect::<Result<Vec<_>, _>>()?;
    if parent_components.len() != entry.route_identities.len() + entry.missing_parent_chain.len() {
        return Err(RetainedApplyValidationError::new(
            RetainedApplyValidationErrorKind::Invariant,
            "retained apply parent route does not match its evidence",
        ));
    }
    let mut ancestor = entry.root.as_ref().clone();
    for (component, expected) in parent_components
        .iter()
        .take(entry.route_identities.len())
        .zip(&entry.route_identities)
    {
        retained_apply_entry_checkpoint(entry)?;
        ancestor = match ancestor.retain_immediate_child_nofollow(component) {
            Ok(RetainedChildCapability::Directory(directory))
                if directory.identity() == *expected =>
            {
                directory
            }
            Ok(RetainedChildCapability::ReparsePoint) => {
                return Err(RetainedApplyValidationError::new(
                    RetainedApplyValidationErrorKind::ContainmentIdentity,
                    format!(
                        "retained apply target parent became a link/reparse point: {}",
                        entry.relative_path.display()
                    ),
                ))
            }
            _ => {
                return Err(RetainedApplyValidationError::new(
                    RetainedApplyValidationErrorKind::ContainmentIdentity,
                    format!(
                        "retained apply target parent link/reparse route or physical identity changed: {}",
                        entry.relative_path.display()
                    ),
                ))
            }
        };
    }
    retained_apply_entry_checkpoint(entry)?;
    if let Some(first_missing) = entry.missing_parent_chain.first() {
        if entry.original.is_some() {
            return Err(RetainedApplyValidationError::new(
                RetainedApplyValidationErrorKind::Invariant,
                "retained apply preimage exists below an absent parent",
            ));
        }
        return match ancestor.retain_immediate_child_nofollow(first_missing) {
            Err(error) if error.kind() == ErrorKind::NotFound => {
                Ok(RetainedApplyExecutionBinding {
                    ancestor,
                    original_file: None,
                })
            }
            Ok(RetainedChildCapability::ReparsePoint) => Err(RetainedApplyValidationError::new(
                RetainedApplyValidationErrorKind::ContainmentIdentity,
                format!(
                    "retained apply absent parent chain became a link/reparse point: {}",
                    entry.relative_path.display()
                ),
            )),
            Ok(_) => Err(RetainedApplyValidationError::new(
                RetainedApplyValidationErrorKind::AbsentChainOccupied,
                format!(
                    "retained apply absent parent chain is occupied: {}",
                    entry.relative_path.display()
                ),
            )),
            Err(error) => Err(RetainedApplyValidationError::new(
                RetainedApplyValidationErrorKind::UnsupportedProvider,
                format!(
                    "retained apply absent parent chain cannot be inspected {}: {error}",
                    entry.relative_path.display()
                ),
            )),
        };
    }

    let observed = match ancestor.retain_immediate_child_nofollow(&entry.name) {
        Ok(RetainedChildCapability::RegularFile(file)) => Some(file),
        Err(error) if error.kind() == ErrorKind::NotFound => None,
        Ok(RetainedChildCapability::ReparsePoint) => {
            return Err(RetainedApplyValidationError::new(
                RetainedApplyValidationErrorKind::ContainmentIdentity,
                format!(
                    "retained apply target became a link/reparse point: {}",
                    entry.relative_path.display()
                ),
            ))
        }
        Ok(RetainedChildCapability::Directory(_) | RetainedChildCapability::Unsupported) => {
            return Err(RetainedApplyValidationError::new(
                if entry.original.is_none() {
                    RetainedApplyValidationErrorKind::AbsentChainOccupied
                } else {
                    RetainedApplyValidationErrorKind::ContainmentIdentity
                },
                format!(
                    "retained apply preimage changed: {}",
                    entry.relative_path.display()
                ),
            ))
        }
        Err(error) => {
            return Err(RetainedApplyValidationError::new(
                RetainedApplyValidationErrorKind::UnsupportedProvider,
                format!(
                    "retained apply target cannot be inspected {}: {error}",
                    entry.relative_path.display()
                ),
            ))
        }
    };
    let Some(observed) = observed else {
        if entry.original.is_none() {
            return Ok(RetainedApplyExecutionBinding {
                ancestor,
                original_file: None,
            });
        }
        return Err(RetainedApplyValidationError::new(
            RetainedApplyValidationErrorKind::ContainmentIdentity,
            format!(
                "retained apply preimage changed: {}",
                entry.relative_path.display()
            ),
        ));
    };
    let (Some(expected_identity), Some(expected_len), Some(expected_sha256)) = (
        entry.original_file_identity,
        entry.original_len,
        entry.original_sha256,
    ) else {
        return Err(RetainedApplyValidationError::new(
            RetainedApplyValidationErrorKind::AbsentChainOccupied,
            format!(
                "retained apply preimage changed: {}",
                entry.relative_path.display()
            ),
        ));
    };
    if observed.hard_link_count().map_err(|error| {
        RetainedApplyValidationError::new(
            RetainedApplyValidationErrorKind::UnsupportedProvider,
            format!("retained apply hard-link count cannot be read: {error}"),
        )
    })? != 1
    {
        return Err(RetainedApplyValidationError::new(
            RetainedApplyValidationErrorKind::ContainmentIdentity,
            format!(
                "retained apply target has a hard-link alias: {}",
                entry.relative_path.display()
            ),
        ));
    }
    if observed.identity() != expected_identity {
        return Err(RetainedApplyValidationError::new(
            RetainedApplyValidationErrorKind::ConcurrentRevision,
            format!(
                "retained apply preimage changed: {}",
                entry.relative_path.display()
            ),
        ));
    }
    let mut file = observed.open_named_identity_for_read().map_err(|error| {
        RetainedApplyValidationError::new(
            RetainedApplyValidationErrorKind::ConcurrentRevision,
            format!("retained apply target cannot be reopened: {error}"),
        )
    })?;
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut length = 0_u64;
    loop {
        retained_apply_entry_checkpoint(entry)?;
        let count = match file.read(&mut buffer) {
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            result => result.map_err(|error| {
                RetainedApplyValidationError::new(
                    RetainedApplyValidationErrorKind::UnsupportedProvider,
                    format!("retained apply target cannot be rechecked: {error}"),
                )
            })?,
        };
        if count == 0 {
            break;
        }
        length = length.checked_add(count as u64).ok_or_else(|| {
            RetainedApplyValidationError::new(
                RetainedApplyValidationErrorKind::UnsupportedProvider,
                "retained apply target length overflow",
            )
        })?;
        if length > expected_len {
            return Err(RetainedApplyValidationError::new(
                RetainedApplyValidationErrorKind::ConcurrentRevision,
                format!(
                    "retained apply preimage changed: {}",
                    entry.relative_path.display()
                ),
            ));
        }
        hash.update(&buffer[..count]);
        #[cfg(any(test, feature = "test-support"))]
        run_retained_apply_after_write_target_chunk_hook();
    }
    retained_apply_entry_checkpoint(entry)?;
    if length != expected_len || <[u8; 32]>::from(hash.finalize()) != expected_sha256 {
        return Err(RetainedApplyValidationError::new(
            RetainedApplyValidationErrorKind::ConcurrentRevision,
            format!(
                "retained apply preimage changed: {}",
                entry.relative_path.display()
            ),
        ));
    }
    drop(file);
    observed.validate_named_identity().map_err(|_error| {
        RetainedApplyValidationError::new(
            RetainedApplyValidationErrorKind::ConcurrentRevision,
            format!(
                "retained apply preimage changed: {}",
                entry.relative_path.display()
            ),
        )
    })?;
    Ok(RetainedApplyExecutionBinding {
        ancestor,
        original_file: Some(observed),
    })
}

impl RetainedApplyReadGuardBinding {
    /// Walk the guarded route from the retained root again and retain the
    /// file it names now. Every directory and the file must be the physical
    /// objects observed while planning; the descriptors of this walk are
    /// released when the returned capability is dropped.
    pub(super) fn retain_named_route(
        &self,
    ) -> Result<
        crate::platform::filesystem::RetainedRegularFileCapability,
        RetainedApplyValidationError,
    > {
        let components = self
            .relative_path
            .components()
            .map(|component| match component {
                std::path::Component::Normal(name) => Ok(name),
                _ => Err(RetainedApplyValidationError::new(
                    RetainedApplyValidationErrorKind::Invariant,
                    "reference input path is not relative",
                )),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let Some((name, parents)) = components.split_last() else {
            return Err(RetainedApplyValidationError::new(
                RetainedApplyValidationErrorKind::Invariant,
                "reference input path is empty",
            ));
        };
        if parents.len() != self.route_identities.len() {
            return Err(RetainedApplyValidationError::new(
                RetainedApplyValidationErrorKind::Invariant,
                "reference input route does not match its identities",
            ));
        }
        let mut parent = self.root.as_ref().clone();
        for (component, expected) in parents.iter().zip(&self.route_identities) {
            if self.cancellation.is_cancelled() {
                return Err(RetainedApplyValidationError::new(
                    RetainedApplyValidationErrorKind::Cancelled,
                    "retained apply reference check cancelled",
                ));
            }
            if self.deadline.is_elapsed() {
                return Err(RetainedApplyValidationError::new(
                    RetainedApplyValidationErrorKind::Deadline,
                    "retained apply reference check deadline elapsed",
                ));
            }
            parent = match parent.retain_immediate_child_nofollow(component) {
                Ok(RetainedChildCapability::Directory(child)) if child.identity() == *expected => {
                    child
                }
                _ => {
                    return Err(RetainedApplyValidationError::new(
                        RetainedApplyValidationErrorKind::ContainmentIdentity,
                        format!(
                            "reference input parent changed: {}",
                            self.relative_path.display()
                        ),
                    ));
                }
            };
        }
        match parent.retain_immediate_child_nofollow(name) {
            Ok(RetainedChildCapability::RegularFile(file)) => Ok(file),
            _ => Err(RetainedApplyValidationError::new(
                RetainedApplyValidationErrorKind::ContainmentIdentity,
                format!(
                    "reference input is no longer a regular file: {}",
                    self.relative_path.display()
                ),
            )),
        }
    }
}

fn validate_retained_apply_read_guard(
    guard: &RetainedApplyReadGuardBinding,
) -> Result<(), RetainedApplyValidationError> {
    let checkpoint = || {
        if guard.cancellation.is_cancelled() {
            return Err(RetainedApplyValidationError::new(
                RetainedApplyValidationErrorKind::Cancelled,
                "retained apply reference check cancelled",
            ));
        }
        if guard.deadline.is_elapsed() {
            return Err(RetainedApplyValidationError::new(
                RetainedApplyValidationErrorKind::Deadline,
                "retained apply reference check deadline elapsed",
            ));
        }
        Ok(())
    };
    checkpoint()?;
    guard.root.validate_named_identity().map_err(|error| {
        RetainedApplyValidationError::new(
            RetainedApplyValidationErrorKind::ContainmentIdentity,
            format!("reference input root changed: {error}"),
        )
    })?;
    let observed = guard.retain_named_route()?;
    if observed.hard_link_count().map_err(|error| {
        RetainedApplyValidationError::new(
            RetainedApplyValidationErrorKind::UnsupportedProvider,
            format!("reference input hard-link count failed: {error}"),
        )
    })? != 1
    {
        return Err(RetainedApplyValidationError::new(
            RetainedApplyValidationErrorKind::ContainmentIdentity,
            format!(
                "reference input has a hard-link alias: {}",
                guard.relative_path.display()
            ),
        ));
    }
    if observed.identity() != guard.file_identity {
        return Err(RetainedApplyValidationError::new(
            RetainedApplyValidationErrorKind::ConcurrentRevision,
            format!(
                "reference input identity changed: {}",
                guard.relative_path.display()
            ),
        ));
    }
    let mut file = observed.open_named_identity_for_read().map_err(|error| {
        RetainedApplyValidationError::new(
            RetainedApplyValidationErrorKind::ConcurrentRevision,
            format!("reference input cannot be reopened: {error}"),
        )
    })?;
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut length = 0_u64;
    loop {
        checkpoint()?;
        let count = match file.read(&mut buffer) {
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            result => result.map_err(|error| {
                RetainedApplyValidationError::new(
                    RetainedApplyValidationErrorKind::UnsupportedProvider,
                    format!("reference input cannot be rechecked: {error}"),
                )
            })?,
        };
        if count == 0 {
            break;
        }
        length = length.checked_add(count as u64).ok_or_else(|| {
            RetainedApplyValidationError::new(
                RetainedApplyValidationErrorKind::UnsupportedProvider,
                "reference input length overflow",
            )
        })?;
        if length > guard.expected_len {
            return Err(RetainedApplyValidationError::new(
                RetainedApplyValidationErrorKind::ConcurrentRevision,
                format!("reference input grew: {}", guard.relative_path.display()),
            ));
        }
        hash.update(&buffer[..count]);
    }
    checkpoint()?;
    if length != guard.expected_len || <[u8; 32]>::from(hash.finalize()) != guard.expected_sha256 {
        return Err(RetainedApplyValidationError::new(
            RetainedApplyValidationErrorKind::ConcurrentRevision,
            format!(
                "reference input bytes changed: {}",
                guard.relative_path.display()
            ),
        ));
    }
    #[cfg(any(test, feature = "test-support"))]
    run_retained_apply_after_reference_stream_hook();
    drop(file);
    observed.validate_named_identity().map_err(|error| {
        RetainedApplyValidationError::new(
            RetainedApplyValidationErrorKind::ConcurrentRevision,
            format!("reference input identity changed during content check: {error}"),
        )
    })?;
    Ok(())
}

fn validate_retained_apply_state_typed(
    entry: &PlannedRetainedApplyChange,
    binding: &RetainedApplyExecutionBinding,
    expected: &Option<Vec<u8>>,
    validate_root_name: bool,
) -> Result<(), RetainedApplyValidationError> {
    if validate_root_name {
        entry.root.validate_named_identity().map_err(|error| {
            RetainedApplyValidationError::new(
                RetainedApplyValidationErrorKind::ContainmentIdentity,
                format!("retained apply source-root physical identity changed: {error}"),
            )
        })?;
    }
    if !entry.missing_parent_chain.is_empty() {
        binding.ancestor.validate_named_identity().map_err(|error| {
            RetainedApplyValidationError::new(
                RetainedApplyValidationErrorKind::ContainmentIdentity,
                format!(
                    "retained apply nearest ancestor link/reparse route or physical identity changed {}: {error}",
                    entry.relative_path.display()
                ),
            )
        })?;
        if expected.is_some() {
            return Err(RetainedApplyValidationError::new(
                RetainedApplyValidationErrorKind::Invariant,
                format!(
                    "retained apply staging invariant requires an absent file below an absent parent: {}",
                    entry.relative_path.display()
                ),
            ));
        }
        return match binding
            .ancestor
            .retain_immediate_child_nofollow(&entry.missing_parent_chain[0])
        {
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Ok(RetainedChildCapability::ReparsePoint) => Err(RetainedApplyValidationError::new(
                RetainedApplyValidationErrorKind::ContainmentIdentity,
                format!(
                    "retained apply absent parent chain became a link/reparse point: {}",
                    entry.relative_path.display()
                ),
            )),
            Ok(_) => Err(RetainedApplyValidationError::new(
                RetainedApplyValidationErrorKind::AbsentChainOccupied,
                format!(
                    "retained apply absent parent chain is occupied: {}",
                    entry.relative_path.display()
                ),
            )),
            Err(error) => Err(RetainedApplyValidationError::new(
                RetainedApplyValidationErrorKind::UnsupportedProvider,
                format!(
                    "retained apply absent parent chain cannot be inspected {}: {error}",
                    entry.relative_path.display()
                ),
            )),
        };
    }
    validate_retained_apply_state_at_parent_typed(entry, &binding.ancestor, expected)
}

fn validate_retained_apply_state_at_parent_typed(
    entry: &PlannedRetainedApplyChange,
    parent: &RetainedDirectoryCapability,
    expected: &Option<Vec<u8>>,
) -> Result<(), RetainedApplyValidationError> {
    parent.validate_named_identity().map_err(|error| {
        RetainedApplyValidationError::new(
            RetainedApplyValidationErrorKind::ContainmentIdentity,
            format!(
                "retained apply target parent link/reparse route or physical identity changed {}: {error}",
                entry.relative_path.display()
            ),
        )
    })?;
    let observed = match parent.retain_immediate_child_nofollow(&entry.name) {
        Ok(RetainedChildCapability::RegularFile(file)) => {
            if file.hard_link_count().map_err(|error| {
                RetainedApplyValidationError::new(
                    RetainedApplyValidationErrorKind::UnsupportedProvider,
                    format!("retained apply hard-link count cannot be read: {error}"),
                )
            })? != 1
            {
                return Err(RetainedApplyValidationError::new(
                    RetainedApplyValidationErrorKind::ContainmentIdentity,
                    format!(
                        "retained apply target has a hard-link alias: {}",
                        entry.relative_path.display()
                    ),
                ));
            }
            Some(file.read_bounded(32 * 1024 * 1024).map_err(|error| {
                RetainedApplyValidationError::new(
                    RetainedApplyValidationErrorKind::UnsupportedProvider,
                    format!("retained apply preimage cannot be read: {error}"),
                )
            })?)
        }
        Ok(RetainedChildCapability::ReparsePoint) => {
            return Err(RetainedApplyValidationError::new(
                RetainedApplyValidationErrorKind::ContainmentIdentity,
                format!(
                    "retained apply target is a link/reparse point: {}",
                    entry.relative_path.display()
                ),
            ))
        }
        Ok(RetainedChildCapability::Directory(_) | RetainedChildCapability::Unsupported) => {
            let kind = if expected.is_none() {
                RetainedApplyValidationErrorKind::AbsentChainOccupied
            } else {
                RetainedApplyValidationErrorKind::ContainmentIdentity
            };
            return Err(RetainedApplyValidationError::new(
                kind,
                format!(
                    "retained apply target is not a regular file: {}",
                    entry.relative_path.display()
                ),
            ));
        }
        Err(error) if error.kind() == ErrorKind::NotFound => None,
        Err(error) => {
            return Err(RetainedApplyValidationError::new(
                RetainedApplyValidationErrorKind::UnsupportedProvider,
                format!(
                    "retained apply target cannot be inspected {}: {error}",
                    entry.relative_path.display()
                ),
            ))
        }
    };
    if observed != *expected {
        let kind = if expected.is_none() {
            RetainedApplyValidationErrorKind::AbsentChainOccupied
        } else if observed.is_none() {
            RetainedApplyValidationErrorKind::ContainmentIdentity
        } else {
            RetainedApplyValidationErrorKind::ConcurrentRevision
        };
        return Err(RetainedApplyValidationError::new(
            kind,
            format!(
                "retained apply preimage changed: {}",
                entry.relative_path.display()
            ),
        ));
    }
    Ok(())
}

#[derive(Debug)]
struct PublishedRetainedApplyChange {
    parent: RetainedDirectoryCapability,
    relative_path: PathBuf,
    name: OsString,
    recovery_name: Option<OsString>,
    kind: PlannedChangeKind,
    published: Option<crate::platform::filesystem::RetainedRegularFileCapability>,
    displaced: Option<crate::platform::filesystem::RetainedRegularFileCapability>,
}

// Recovery files remain rollback authority until publication is accepted. Check
// only journal-owned files: unrelated source entries do not belong to this plan.
fn validate_journal_recoveries(
    root: &RetainedDirectoryCapability,
    published: &[PublishedRetainedApplyChange],
) -> Result<(), ApplyPublicationError> {
    root.validate_named_identity().map_err(|error| {
        ApplyPublicationError::new(
            ApplyPublicationErrorKind::ContainmentIdentity,
            format!("retained apply recovery root identity changed: {error}"),
        )
    })?;
    let mut names = BTreeSet::new();
    for change in published {
        let (name, recovery) = match (&change.recovery_name, &change.displaced) {
            (Some(name), Some(recovery)) => (name, recovery),
            (None, None) => continue,
            _ => {
                return Err(ApplyPublicationError::new(
                    ApplyPublicationErrorKind::Invariant,
                    "retained apply journal recovery name/capability pair is incomplete",
                ));
            }
        };
        change.parent.validate_named_identity().map_err(|error| {
            ApplyPublicationError::new(
                ApplyPublicationErrorKind::ContainmentIdentity,
                format!("retained apply recovery parent identity changed: {error}"),
            )
        })?;
        recovery.validate_named_identity().map_err(|error| {
            ApplyPublicationError::new(
                ApplyPublicationErrorKind::ContainmentIdentity,
                format!("retained apply recovery identity changed: {error}"),
            )
        })?;
        if recovery.hard_link_count().map_err(|error| {
            ApplyPublicationError::new(
                ApplyPublicationErrorKind::ProviderPostvalidation,
                format!("retained apply recovery hard-link count failed: {error}"),
            )
        })? != 1
        {
            return Err(ApplyPublicationError::new(
                ApplyPublicationErrorKind::ContainmentIdentity,
                "retained apply recovery gained a hard-link alias",
            ));
        }
        if !names.insert((change.parent.identity(), name)) {
            return Err(ApplyPublicationError::new(
                ApplyPublicationErrorKind::Invariant,
                "retained apply journal contains a duplicate recovery name",
            ));
        }
    }
    Ok(())
}

#[derive(Debug)]
struct CreatedRetainedApplyDirectory {
    source_participant: bool,
    parent: RetainedDirectoryCapability,
    name: OsString,
    directory: RetainedDirectoryCapability,
    logical_path: PathBuf,
    published: bool,
}

/// Every staged copy, preimage and private directory a publication leaves in
/// the source tree starts with this prefix; it is never a source name.
pub const PUBLICATION_ARTIFACT_PREFIX: &str = ".unica-apply-";

fn retained_apply_stage_name() -> OsString {
    OsString::from(format!(
        "{PUBLICATION_ARTIFACT_PREFIX}{}",
        uuid::Uuid::new_v4()
    ))
}

fn retained_apply_directory_stage_name() -> OsString {
    OsString::from(format!(
        "{PUBLICATION_ARTIFACT_PREFIX}dir-{}",
        uuid::Uuid::new_v4()
    ))
}

fn retained_apply_directory_cleanup_name() -> OsString {
    OsString::from(format!(
        "{PUBLICATION_ARTIFACT_PREFIX}dir-cleanup-{}",
        uuid::Uuid::new_v4()
    ))
}

fn create_retained_apply_parent_chain(
    entry: &PlannedRetainedApplyChange,
    binding: &RetainedApplyExecutionBinding,
    created: &mut Vec<CreatedRetainedApplyDirectory>,
    source_participant: bool,
) -> Result<RetainedDirectoryCapability, RetainedApplyPublishError> {
    entry.root.validate_named_identity().map_err(|error| {
        RetainedApplyPublishError::containment(format!(
            "retained apply source-root physical identity changed: {error}"
        ))
    })?;
    binding
        .ancestor
        .validate_named_identity()
        .map_err(|error| {
            RetainedApplyPublishError::containment(format!(
                "retained apply nearest ancestor physical identity changed {}: {error}",
                entry.relative_path.display()
            ))
        })?;
    let parent_path = entry
        .relative_path
        .parent()
        .expect("retained file path has a parent");
    let parent_components = parent_path
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(name) => Some(name.to_os_string()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let retained_prefix_len = parent_components
        .len()
        .checked_sub(entry.missing_parent_chain.len())
        .ok_or_else(|| {
            RetainedApplyPublishError::new(
                RetainedApplyPublishErrorKind::Invariant,
                "retained apply missing-parent chain exceeds its logical path",
            )
        })?;
    let mut logical_path = parent_components[..retained_prefix_len]
        .iter()
        .collect::<PathBuf>();
    let mut parent = binding.ancestor.clone();
    for name in &entry.missing_parent_chain {
        logical_path.push(name);
        if let Some(directory) = owned_retained_apply_directory_child(&parent, name, created)? {
            parent = directory;
            continue;
        }
        match parent.retain_immediate_child_nofollow(name) {
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Ok(_) => {
                return Err(RetainedApplyPublishError::containment(format!(
                    "retained apply absent parent chain is occupied: {}",
                    logical_path.display()
                )))
            }
            Err(error) => {
                return Err(RetainedApplyPublishError::provider(format!(
                    "retained apply absent parent chain cannot be inspected {}: {error}",
                    logical_path.display()
                )))
            }
        }
        let private_name = retained_apply_directory_stage_name();
        let directory = match parent.create_directory_child_atomically(&private_name, name) {
            Ok(directory) => directory,
            Err(error) => {
                let (error, artifact) = error.into_parts();
                if let Some((directory, artifact_name, published)) = artifact {
                    created.push(CreatedRetainedApplyDirectory {
                        source_participant,
                        parent: parent.clone(),
                        name: artifact_name,
                        directory,
                        logical_path: logical_path.clone(),
                        published,
                    });
                }
                let kind = if error.kind() == ErrorKind::AlreadyExists
                    || error.kind() == ErrorKind::NotFound
                {
                    RetainedApplyPublishErrorKind::ContainmentIdentity
                } else {
                    RetainedApplyPublishErrorKind::Provider
                };
                return Err(RetainedApplyPublishError::new(
                    kind,
                    format!(
                        "retained apply absent parent publication failed {}: {error}",
                        logical_path.display()
                    ),
                ));
            }
        };
        created.push(CreatedRetainedApplyDirectory {
            source_participant,
            parent: parent.clone(),
            name: name.clone(),
            directory: directory.clone(),
            logical_path: logical_path.clone(),
            published: true,
        });
        parent = directory;
    }
    Ok(parent)
}

fn owned_retained_apply_directory_child(
    parent: &RetainedDirectoryCapability,
    name: &OsStr,
    created: &[CreatedRetainedApplyDirectory],
) -> Result<Option<RetainedDirectoryCapability>, RetainedApplyPublishError> {
    for owned in created {
        if !owned.published || owned.parent.identity() != parent.identity() {
            continue;
        }
        if !parent
            .child_names_equivalent(&owned.name, name)
            .map_err(|error| {
                RetainedApplyPublishError::provider(format!(
                    "retained apply child identity cannot be proven: {error}"
                ))
            })?
        {
            continue;
        }
        owned.directory.validate_named_identity().map_err(|error| {
            RetainedApplyPublishError::containment(format!(
                "batch-created retained directory identity changed {}: {error}",
                owned.logical_path.display()
            ))
        })?;
        return Ok(Some(owned.directory.clone()));
    }
    Ok(None)
}

fn validate_retained_apply_postimage(
    entry: &PlannedRetainedApplyChange,
    binding: &RetainedApplyExecutionBinding,
    expected: &Option<Vec<u8>>,
    created: &[CreatedRetainedApplyDirectory],
) -> Result<(), RetainedApplyPublishError> {
    if entry.missing_parent_chain.is_empty() {
        return validate_retained_apply_state_typed(entry, binding, expected, true)
            .map_err(RetainedApplyPublishError::from);
    }
    if entry.original.is_none() && entry.current.is_none() {
        // Resolve only the path already observed absent. A sibling write may
        // have created its parents, but must not create this observed file.
        let mut parent = binding.ancestor.clone();
        for name in &entry.missing_parent_chain {
            match parent.retain_immediate_child_nofollow(name) {
                Ok(RetainedChildCapability::Directory(directory)) => parent = directory,
                Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
                _ => {
                    return Err(RetainedApplyPublishError::containment(
                        "read-only absent parent changed its kind",
                    ))
                }
            }
        }
        return validate_retained_apply_state_at_parent_typed(entry, &parent, expected)
            .map_err(RetainedApplyPublishError::from);
    }
    entry.root.validate_named_identity().map_err(|error| {
        RetainedApplyPublishError::containment(format!(
            "retained apply source-root physical identity changed: {error}"
        ))
    })?;
    let mut parent = binding.ancestor.clone();
    for name in &entry.missing_parent_chain {
        parent =
            owned_retained_apply_directory_child(&parent, name, created)?.ok_or_else(|| {
                RetainedApplyPublishError::new(
                    RetainedApplyPublishErrorKind::Invariant,
                    format!(
                        "retained apply postimage parent is not transaction-owned: {}",
                        entry.relative_path.display()
                    ),
                )
            })?;
    }
    validate_retained_apply_state_at_parent_typed(entry, &parent, expected)
        .map_err(RetainedApplyPublishError::from)
}

fn retained_apply_displacement_error_kind(error: &std::io::Error) -> RetainedApplyPublishErrorKind {
    if matches!(
        error.kind(),
        ErrorKind::PermissionDenied
            | ErrorKind::ReadOnlyFilesystem
            | ErrorKind::StorageFull
            | ErrorKind::QuotaExceeded
    ) || is_file_sharing_violation(error)
    {
        RetainedApplyPublishErrorKind::Provider
    } else {
        RetainedApplyPublishErrorKind::ContainmentIdentity
    }
}

fn publish_retained_apply_change(
    entry: &PlannedRetainedApplyChange,
    binding: &RetainedApplyExecutionBinding,
    journal: &mut Vec<PublishedRetainedApplyChange>,
    created_directories: &mut Vec<CreatedRetainedApplyDirectory>,
    source_participant: bool,
) -> Result<(), RetainedApplyPublishError> {
    publish_retained_apply_change_inner(
        entry,
        binding,
        journal,
        created_directories,
        source_participant,
    )
    .map_err(|error| error.with_input(ApplyInputContext::for_entry(entry, source_participant)))
}

fn publish_retained_apply_change_inner(
    entry: &PlannedRetainedApplyChange,
    binding: &RetainedApplyExecutionBinding,
    journal: &mut Vec<PublishedRetainedApplyChange>,
    created_directories: &mut Vec<CreatedRetainedApplyDirectory>,
    source_participant: bool,
) -> Result<(), RetainedApplyPublishError> {
    if entry.original == entry.current {
        return Ok(());
    }
    let parent = if entry.missing_parent_chain.is_empty() {
        binding.ancestor.clone()
    } else {
        let parent = create_retained_apply_parent_chain(
            entry,
            binding,
            created_directories,
            source_participant,
        )?;
        validate_retained_apply_state_at_parent_typed(entry, &parent, &entry.original)
            .map_err(RetainedApplyPublishError::from)?;
        parent
    };
    parent.validate_named_identity().map_err(|error| {
        RetainedApplyPublishError::containment(format!(
            "retained apply parent identity cannot be validated: {error}"
        ))
    })?;
    #[cfg(any(test, feature = "test-support"))]
    run_retained_apply_before_provider_io_hook();
    let name = entry.name.clone();
    match (&entry.original, &entry.current) {
        (None, Some(bytes)) => {
            let result =
                parent.create_regular_child_atomically(&retained_apply_stage_name(), &name, bytes);
            match result {
                Ok(published) => journal.push(PublishedRetainedApplyChange {
                    parent,
                    relative_path: entry.relative_path.clone(),
                    name,
                    recovery_name: None,
                    kind: PlannedChangeKind::Create,
                    published: Some(published),
                    displaced: None,
                }),
                Err(error) => {
                    let (error, published) = error.into_parts();
                    if let Some(published) = published {
                        journal.push(PublishedRetainedApplyChange {
                            parent,
                            relative_path: entry.relative_path.clone(),
                            name,
                            recovery_name: None,
                            kind: PlannedChangeKind::Create,
                            published: Some(published),
                            displaced: None,
                        });
                    }
                    let kind = if error.kind() == ErrorKind::AlreadyExists
                        || error.kind() == ErrorKind::NotFound
                    {
                        RetainedApplyPublishErrorKind::ContainmentIdentity
                    } else {
                        RetainedApplyPublishErrorKind::Provider
                    };
                    return Err(RetainedApplyPublishError::new(
                        kind,
                        format!("retained apply create failed: {error}"),
                    ));
                }
            }
        }
        (Some(original), current) => {
            let expected = binding.original_file.as_ref().ok_or_else(|| {
                RetainedApplyPublishError::new(
                    RetainedApplyPublishErrorKind::Invariant,
                    "retained apply original file authority is missing",
                )
            })?;
            let recovery_name = retained_apply_stage_name();
            let displaced = parent
                .displace_regular_child_no_replace(&name, expected, &recovery_name)
                .map_err(|error| {
                    RetainedApplyPublishError::new(
                        retained_apply_displacement_error_kind(&error),
                        format!("retained apply destination identity/preimage changed: {error}"),
                    )
                })?;
            let kind = if current.is_some() {
                PlannedChangeKind::Update
            } else {
                PlannedChangeKind::Remove
            };
            journal.push(PublishedRetainedApplyChange {
                parent,
                relative_path: entry.relative_path.clone(),
                name,
                recovery_name: Some(recovery_name),
                kind,
                published: None,
                displaced: Some(displaced),
            });
            validate_displaced_apply_preimage_typed(
                journal
                    .last()
                    .and_then(|change| change.displaced.as_ref())
                    .expect("displacement journal owns the original file"),
                original,
            )?;
            if let Some(bytes) = current {
                let change = journal.last_mut().expect("displacement journal exists");
                match change.parent.create_regular_child_atomically(
                    &retained_apply_stage_name(),
                    &change.name,
                    bytes,
                ) {
                    Ok(published) => change.published = Some(published),
                    Err(error) => {
                        let (error, published) = error.into_parts();
                        change.published = published;
                        let kind = if error.kind() == ErrorKind::AlreadyExists
                            || error.kind() == ErrorKind::NotFound
                        {
                            RetainedApplyPublishErrorKind::ContainmentIdentity
                        } else {
                            RetainedApplyPublishErrorKind::Provider
                        };
                        return Err(RetainedApplyPublishError::new(
                            kind,
                            format!("retained apply replace failed: {error}"),
                        ));
                    }
                }
            }
        }
        (None, None) => {
            return Err(RetainedApplyPublishError::new(
                RetainedApplyPublishErrorKind::Invariant,
                "retained apply no-op reached publication",
            ))
        }
    }
    Ok(())
}

fn validate_displaced_apply_preimage_typed(
    displaced: &crate::platform::filesystem::RetainedRegularFileCapability,
    expected: &[u8],
) -> Result<(), RetainedApplyPublishError> {
    if displaced.hard_link_count().map_err(|error| {
        RetainedApplyPublishError::provider(format!(
            "retained apply displaced hard-link count failed: {error}"
        ))
    })? != 1
    {
        return Err(RetainedApplyPublishError::containment(
            "retained apply destination gained a hard-link alias",
        ));
    }
    let observed = displaced.read_bounded(32 * 1024 * 1024).map_err(|error| {
        RetainedApplyPublishError::provider(format!(
            "retained apply displaced preimage read failed: {error}"
        ))
    })?;
    if observed != expected {
        return Err(RetainedApplyPublishError::new(
            RetainedApplyPublishErrorKind::ConcurrentRevision,
            "retained apply destination preimage changed at mutation boundary",
        ));
    }
    Ok(())
}

fn rollback_retained_apply(
    published: &mut Vec<PublishedRetainedApplyChange>,
    created_directories: &mut Vec<CreatedRetainedApplyDirectory>,
) -> Vec<String> {
    let mut errors = Vec::new();
    while let Some(mut change) = published.pop() {
        #[cfg(any(test, feature = "test-support"))]
        RETAINED_APPLY_OBSERVED_EVENTS.with(|events| {
            events
                .borrow_mut()
                .push(RetainedApplyObservedEvent::Rollback(
                    change.relative_path.clone(),
                ));
        });
        if let Some(file) = change.published.take() {
            let removal = file
                .remove_named_identity_relative_for_retained_apply(&retained_apply_stage_name());
            // Retained-apply cleanup unlinks the name at once on Windows (POSIX
            // delete semantics); the classic fallback keeps a delete-pending name
            // until every handle closes. Release the journal's retained handle
            // before restoring a displaced preimage at the same name.
            drop(file);
            if let Err(error) = removal {
                errors.push(format!("published apply child was preserved: {error}"));
            }
        } else if change.kind == PlannedChangeKind::Create {
            errors.push("published create capability is missing".to_string());
        }
        if let Some(displaced) = change.displaced.as_ref() {
            match displaced.hard_link_count() {
                Ok(1) => {}
                Ok(_) => {
                    errors.push(
                        "displaced apply preimage gained a hard-link alias; recovery was preserved"
                            .to_string(),
                    );
                    continue;
                }
                Err(error) => {
                    errors.push(format!(
                        "displaced apply preimage hard-link count failed; recovery was preserved: {error}"
                    ));
                    continue;
                }
            }
            if let Err(error) = change
                .parent
                .restore_displaced_regular_child_no_replace(displaced, &change.name)
            {
                errors.push(format!("displaced apply preimage was preserved: {error}"));
            }
        }
    }
    while let Some(created) = created_directories.pop() {
        if let Err(error) = created
            .directory
            .remove_empty_named_identity_relative_for_retained_apply(
                &retained_apply_directory_cleanup_name(),
            )
        {
            errors.push(format!(
                "batch-created directory was preserved {}: {error}",
                created.logical_path.display()
            ));
        }
    }
    errors
}

fn finalize_retained_apply_recovery(
    published: &mut [PublishedRetainedApplyChange],
    report: &mut CommitReport,
) {
    for change in published {
        if let Some(displaced) = change.displaced.take() {
            if let Err(error) = displaced
                .remove_named_identity_relative_for_retained_apply(&retained_apply_stage_name())
            {
                report.cleanup_warnings.push(format!(
                    "retained apply recovery child was preserved after success: {error}"
                ));
                report
                    .retained_apply_cleanup_diagnostics
                    .push(RetainedApplyCleanupDiagnostic {
                        logical_target: change.relative_path.clone(),
                        artifact_name: change
                            .recovery_name
                            .clone()
                            .expect("displaced retained apply change owns a recovery name"),
                    });
            }
        }
    }
}

pub fn snapshot_directory_membership(
    directory: &Path,
    selector: DirectoryMembershipSelector,
) -> Result<DirectoryMembershipSnapshot, String> {
    snapshot_directory_membership_entries(directory, selector)
}

fn snapshot_directory_membership_entries(
    directory: &Path,
    selector: DirectoryMembershipSelector,
) -> Result<DirectoryMembershipSnapshot, String> {
    let metadata = match fs::symlink_metadata(directory) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Ok(DirectoryMembershipSnapshot::Absent)
        }
        Err(error) => {
            return Err(format!(
                "failed to inspect directory membership guard {}: {error}",
                directory.display()
            ));
        }
    };
    // A guard over the child directories of a container states which of them exist.
    // A path that is not a usable directory holds none, which is the same answer as
    // an absent path — and the only one that keeps the guard takeable. Erroring
    // instead would make every compile transaction in a workspace whose container
    // path is a stray file or a link fail at planning.
    let holds_no_child_directories =
        selector == DirectoryMembershipSelector::DirectChildDirectories;
    if metadata_is_link_or_reparse_point(&metadata) {
        if holds_no_child_directories {
            return Ok(DirectoryMembershipSnapshot::Absent);
        }
        return Err(format!(
            "directory membership guard must not be a symbolic link or reparse point: {}",
            directory.display()
        ));
    }
    if !metadata.is_dir() {
        if holds_no_child_directories {
            return Ok(DirectoryMembershipSnapshot::Absent);
        }
        return Err(format!(
            "directory membership guard target is not a directory: {}",
            directory.display()
        ));
    }

    let mut entries = fs::read_dir(directory)
        .map_err(|error| {
            format!(
                "failed to read directory membership guard {}: {error}",
                directory.display()
            )
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            format!(
                "failed to read directory membership guard entry in {}: {error}",
                directory.display()
            )
        })?;
    entries.sort_by_key(|entry| entry.file_name());
    let mut names = Vec::new();
    for entry in entries {
        let name = entry.file_name();
        if !directory_membership_name_matches(selector, &name) {
            continue;
        }
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|error| {
            format!(
                "failed to inspect directory membership entry {}: {error}",
                path.display()
            )
        })?;
        // A selector that binds only the child directories of a container is used
        // where the container legitimately holds other things too. Those are not
        // members: skipping them is what the guard means, while failing would make
        // it unusable on any real container.
        let skips_non_members = selector == DirectoryMembershipSelector::DirectChildDirectories;
        if metadata_is_link_or_reparse_point(&metadata) {
            if skips_non_members {
                continue;
            }
            return Err(format!(
                "directory membership entry must not be a symbolic link or reparse point: {}",
                path.display()
            ));
        }
        let entry_kind = if metadata.is_dir() {
            Some(DirectoryTopologyEntryKind::Directory)
        } else if metadata.is_file() {
            Some(DirectoryTopologyEntryKind::File)
        } else {
            None
        };
        let Some(kind) =
            entry_kind.filter(|kind| directory_membership_accepts_kind(selector, *kind))
        else {
            if skips_non_members {
                continue;
            }
            return Err(format!(
                "directory membership entry has an unsupported filesystem type: {}",
                path.display()
            ));
        };
        names.push(DirectoryTopologyEntry { name, kind });
    }
    Ok(DirectoryMembershipSnapshot::Present(names))
}

fn snapshot_direct_entry_names(directory: &Path) -> Result<Vec<OsString>, String> {
    let metadata = fs::symlink_metadata(directory).map_err(|error| {
        format!(
            "failed to inspect conditional removal directory {}: {error}",
            directory.display()
        )
    })?;
    if metadata_is_link_or_reparse_point(&metadata) {
        return Err(format!(
            "conditional removal directory must not be a symbolic link or reparse point: {}",
            directory.display()
        ));
    }
    if !metadata.is_dir() {
        return Err(format!(
            "conditional removal target is not a directory: {}",
            directory.display()
        ));
    }
    let mut names = fs::read_dir(directory)
        .map_err(|error| {
            format!(
                "failed to enumerate conditional removal directory {}: {error}",
                directory.display()
            )
        })?
        .map(|entry| {
            entry.map(|entry| entry.file_name()).map_err(|error| {
                format!(
                    "failed to enumerate conditional removal directory {}: {error}",
                    directory.display()
                )
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    names.sort();
    Ok(names)
}

fn normalize_direct_entry_names(expected_names: Vec<OsString>) -> Result<Vec<OsString>, String> {
    let mut normalized = BTreeSet::new();
    for name in expected_names {
        let path = Path::new(&name);
        let mut components = path.components();
        if !matches!(components.next(), Some(std::path::Component::Normal(_)))
            || components.next().is_some()
        {
            return Err(format!(
                "invalid conditional removal entry name: {}",
                path.display()
            ));
        }
        if !normalized.insert(name.clone()) {
            return Err(format!(
                "duplicate conditional removal entry name: {}",
                path.display()
            ));
        }
    }
    Ok(normalized.into_iter().collect())
}

fn normalize_expected_directory_entries(
    selector: DirectoryMembershipSelector,
    expected_entries: Vec<DirectoryTopologyEntry>,
) -> Result<Vec<DirectoryTopologyEntry>, String> {
    let mut normalized = BTreeMap::new();
    for entry in expected_entries {
        let path = Path::new(&entry.name);
        let mut components = path.components();
        if !matches!(components.next(), Some(std::path::Component::Normal(_)))
            || components.next().is_some()
            || !directory_membership_name_matches(selector, &entry.name)
            || !directory_membership_accepts_kind(selector, entry.kind)
        {
            return Err(format!(
                "invalid directory membership entry name: {}",
                path.display()
            ));
        }
        if normalized.insert(entry.name.clone(), entry.kind).is_some() {
            return Err(format!(
                "duplicate directory membership entry name: {}",
                path.display()
            ));
        }
    }
    Ok(normalized
        .into_iter()
        .map(|(name, kind)| DirectoryTopologyEntry { name, kind })
        .collect())
}

fn normalize_directory_membership_snapshot(
    selector: DirectoryMembershipSelector,
    snapshot: DirectoryMembershipSnapshot,
) -> Result<DirectoryMembershipSnapshot, String> {
    match snapshot {
        DirectoryMembershipSnapshot::Absent => Ok(DirectoryMembershipSnapshot::Absent),
        DirectoryMembershipSnapshot::Present(entries) => Ok(DirectoryMembershipSnapshot::Present(
            normalize_expected_directory_entries(selector, entries)?,
        )),
    }
}

fn directory_membership_name_matches(selector: DirectoryMembershipSelector, name: &OsStr) -> bool {
    match selector {
        DirectoryMembershipSelector::XmlFiles => {
            Path::new(name).extension() == Some(OsStr::new("xml"))
        }
        DirectoryMembershipSelector::CfFilesAsciiCaseInsensitive => Path::new(name)
            .extension()
            .and_then(OsStr::to_str)
            .is_some_and(|extension| extension.eq_ignore_ascii_case("cf")),
        DirectoryMembershipSelector::AllDirectEntries
        | DirectoryMembershipSelector::DirectChildDirectories => true,
    }
}

/// Whether an entry of this filesystem kind is a member under this selector.
fn directory_membership_accepts_kind(
    selector: DirectoryMembershipSelector,
    kind: DirectoryTopologyEntryKind,
) -> bool {
    match selector {
        DirectoryMembershipSelector::AllDirectEntries => true,
        DirectoryMembershipSelector::DirectChildDirectories => {
            kind == DirectoryTopologyEntryKind::Directory
        }
        DirectoryMembershipSelector::XmlFiles
        | DirectoryMembershipSelector::CfFilesAsciiCaseInsensitive => {
            kind == DirectoryTopologyEntryKind::File
        }
    }
}

fn validate_directory_membership_guard(
    directory: &Path,
    selector: DirectoryMembershipSelector,
    expected: &DirectoryMembershipSnapshot,
    phase: &str,
) -> Result<(), String> {
    let actual = snapshot_directory_membership_entries(directory, selector)?;
    if &actual != expected {
        let render = |snapshot: &DirectoryMembershipSnapshot| match snapshot {
            DirectoryMembershipSnapshot::Absent => "absent".to_string(),
            DirectoryMembershipSnapshot::Present(entries) => {
                let entries = entries
                    .iter()
                    .map(|entry| {
                        let suffix = match entry.kind {
                            DirectoryTopologyEntryKind::File => "",
                            DirectoryTopologyEntryKind::Directory => "/",
                        };
                        format!("{}{suffix}", entry.name.to_string_lossy())
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("present [{entries}]")
            }
        };
        return Err(format!(
            "compile transaction directory membership guard changed after planning {phase}: {} (expected {}, actual {})",
            directory.display(),
            render(expected),
            render(&actual)
        ));
    }
    Ok(())
}

fn apply_direct_membership_delta(
    directory: &Path,
    selector: DirectoryMembershipSelector,
    path: &Path,
    add: bool,
    added_kind: Option<DirectoryTopologyEntryKind>,
    expected: &mut BTreeMap<OsString, DirectoryTopologyEntryKind>,
) -> Result<(), String> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    if normalize_transaction_path_identity(parent)?
        != normalize_transaction_path_identity(directory)?
    {
        return Ok(());
    }
    let Some(name) = path.file_name() else {
        return Ok(());
    };
    if !directory_membership_name_matches(selector, name) {
        return Ok(());
    }
    if add {
        let kind = added_kind.ok_or_else(|| {
            format!(
                "missing directory membership kind for planned addition: {}",
                path.display()
            )
        })?;
        if !directory_membership_accepts_kind(selector, kind) {
            // A planned entry the selector does not admit is simply not a member of
            // this guard; only the file-shaped selectors treat it as a planning
            // error, because for them a non-file under a matching name is a defect.
            if selector == DirectoryMembershipSelector::DirectChildDirectories {
                return Ok(());
            }
            return Err(format!(
                "planned directory membership entry is not a regular file: {}",
                path.display()
            ));
        }
        expected.insert(name.to_os_string(), kind);
    } else {
        expected.remove(name);
    }
    Ok(())
}

fn validate_exact_read_guard(
    path: &Path,
    expected_preimage: &[u8],
    phase: &str,
) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        format!(
            "failed to inspect compile transaction read guard {} {phase}: {error}",
            path.display()
        )
    })?;
    if metadata_is_link_or_reparse_point(&metadata) {
        return Err(format!(
            "compile transaction read guard must not be a symbolic link or reparse point: {}",
            path.display()
        ));
    }
    if !metadata.is_file() {
        return Err(format!(
            "compile transaction read guard is not a regular file: {}",
            path.display()
        ));
    }
    let actual = fs::read(path).map_err(|error| {
        format!(
            "failed to read compile transaction read guard {} {phase}: {error}",
            path.display()
        )
    })?;
    if actual != expected_preimage {
        return Err(format!(
            "compile transaction read guard changed after planning {phase}: {}",
            path.display()
        ));
    }
    Ok(())
}

fn validate_absence_guard(path: &Path, phase: &str) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Ok(_) => Err(format!(
            "compile transaction absence guard was violated {phase}: {}",
            path.display()
        )),
        Err(error) => Err(format!(
            "failed to inspect compile transaction absence guard {} {phase}: {error}",
            path.display()
        )),
    }
}

#[derive(Debug)]
struct PublishedCreate {
    target: PathBuf,
    published: PublishedFileExpectation,
}

#[derive(Debug)]
struct PublishedRegistration {
    target: PathBuf,
    recovery: RecoveryCleanup,
    original: Vec<u8>,
    published: PublishedFileExpectation,
    original_permissions: PortablePermissions,
}

#[derive(Debug)]
struct PublishedFileExpectation {
    identity: FileIdentity,
    bytes: Vec<u8>,
    sha256: [u8; 32],
}

impl PublishedFileExpectation {
    fn new(identity: FileIdentity, bytes: Vec<u8>) -> Self {
        let sha256 = Sha256::digest(&bytes).into();
        Self {
            identity,
            bytes,
            sha256,
        }
    }
}

#[derive(Debug, Clone)]
struct RecoveryDirectory {
    path: PathBuf,
    identity: FileIdentity,
    parent_identity: FileIdentity,
    parent: Arc<fs::File>,
    directory: Arc<Mutex<Option<fs::File>>>,
}

impl RecoveryDirectory {
    fn reserve(path: &Path) -> Result<Self, (ErrorKind, String)> {
        let route = absolute_lexical_path(path)
            .map_err(|error| (ErrorKind::InvalidInput, error.to_string()))?;
        let name = route
            .file_name()
            .filter(|name| !name.is_empty())
            .ok_or_else(|| {
                (
                    ErrorKind::InvalidInput,
                    format!(
                        "registration recovery directory has no child name: {}",
                        route.display()
                    ),
                )
            })?;
        let lexical_parent = route.parent().ok_or_else(|| {
            (
                ErrorKind::InvalidInput,
                format!(
                    "registration recovery directory has no parent: {}",
                    route.display()
                ),
            )
        })?;
        let canonical_parent = fs::canonicalize(lexical_parent).map_err(|error| {
            (
                error.kind(),
                format!(
                    "failed to resolve registration recovery parent identity {}: {error}",
                    lexical_parent.display()
                ),
            )
        })?;
        let parent = open_directory_nofollow(&canonical_parent).map_err(|error| {
            (
                error.kind(),
                format!(
                    "failed to bind registration recovery parent identity {}: {error}",
                    canonical_parent.display()
                ),
            )
        })?;
        let parent_identity = file_identity(&parent).map_err(|error| {
            (
                error.kind(),
                format!(
                    "failed to inspect registration recovery parent identity {}: {error}",
                    canonical_parent.display()
                ),
            )
        })?;
        let directory = create_new_directory_child(&parent, name).map_err(|error| {
            (
                error.kind(),
                format!(
                    "failed to reserve registration recovery directory {}: {error}",
                    route.display()
                ),
            )
        })?;
        let identity = file_identity(&directory).map_err(|error| {
            (
                error.kind(),
                format!(
                    "failed to inspect registration recovery directory identity {}; the unverified directory was left untouched: {error}",
                    route.display()
                ),
            )
        })?;
        Ok(Self {
            path: route,
            identity,
            parent_identity,
            parent: Arc::new(parent),
            directory: Arc::new(Mutex::new(Some(directory))),
        })
    }

    fn try_clone_directory(&self) -> Result<Arc<fs::File>, String> {
        let retained = self.directory.lock().map_err(|_| {
            format!(
                "registration recovery directory-handle state is poisoned: {}",
                self.path.display()
            )
        })?;
        let directory = retained.as_ref().ok_or_else(|| {
            format!(
                "retained registration recovery directory handle is missing: {}",
                self.path.display()
            )
        })?;
        directory.try_clone().map(Arc::new).map_err(|error| {
            format!(
                "failed to duplicate registration recovery directory handle {}: {error}",
                self.path.display()
            )
        })
    }

    fn verify(&self) -> Result<(), String> {
        let name = self.path.file_name().ok_or_else(|| {
            format!(
                "registration recovery directory has no child name: {}",
                self.path.display()
            )
        })?;
        let lexical_parent = self.path.parent().ok_or_else(|| {
            format!(
                "registration recovery directory has no parent: {}",
                self.path.display()
            )
        })?;
        let canonical_parent = fs::canonicalize(lexical_parent).map_err(|error| {
            format!(
                "registration recovery parent route could not be resolved; replacement left untouched at {}: {error}",
                lexical_parent.display()
            )
        })?;
        let parent = open_directory_nofollow(&canonical_parent).map_err(|error| {
            format!(
                "registration recovery parent identity could not be proven; replacement left untouched at {}: {error}",
                canonical_parent.display()
            )
        })?;
        let parent_identity = file_identity(&parent).map_err(|error| {
            format!(
                "failed to recheck registration recovery parent identity {}; replacement left untouched: {error}",
                canonical_parent.display()
            )
        })?;
        if parent_identity != self.parent_identity {
            return Err(format!(
                "registration recovery parent identity changed; replacement left untouched at {}",
                lexical_parent.display()
            ));
        }
        let retained_parent_identity = file_identity(&self.parent).map_err(|error| {
            format!(
                "failed to recheck retained registration recovery parent identity {}; replacement left untouched: {error}",
                self.path.display()
            )
        })?;
        if retained_parent_identity != self.parent_identity {
            return Err(format!(
                "retained registration recovery parent identity changed; replacement left untouched at {}",
                self.path.display()
            ));
        }
        let directory = open_directory_child_nofollow(&parent, name).map_err(|error| {
            format!(
                "registration recovery directory identity could not be proven; replacement left untouched at {}: {error}",
                self.path.display()
            )
        })?;
        let identity = file_identity(&directory).map_err(|error| {
            format!(
                "failed to recheck registration recovery directory identity {}; replacement left untouched: {error}",
                self.path.display()
            )
        })?;
        if identity != self.identity {
            return Err(format!(
                "registration recovery directory identity changed; replacement left untouched at {}",
                self.path.display()
            ));
        }
        let retained = self.directory.lock().map_err(|_| {
            format!(
                "registration recovery directory-handle state is poisoned; replacement left untouched at {}",
                self.path.display()
            )
        })?;
        let retained = retained.as_ref().ok_or_else(|| {
            format!(
                "retained registration recovery directory handle is missing at {}",
                self.path.display()
            )
        })?;
        let retained_identity = file_identity(retained).map_err(|error| {
            format!(
                "failed to recheck retained registration recovery directory identity {}; replacement left untouched: {error}",
                self.path.display()
            )
        })?;
        if retained_identity != self.identity {
            return Err(format!(
                "retained registration recovery directory identity changed; replacement left untouched at {}",
                self.path.display()
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
struct RecoveryCleanup {
    directory: RecoveryDirectory,
    artifact: Option<CleanupArtifact>,
    directory_removed: bool,
}

impl RecoveryCleanup {
    fn reserve(directory: &Path) -> Result<Self, (ErrorKind, String)> {
        Ok(Self {
            directory: RecoveryDirectory::reserve(directory)?,
            artifact: None,
            directory_removed: false,
        })
    }

    fn path(&self) -> PathBuf {
        self.directory.path.join("original")
    }

    fn directory_path(&self) -> &Path {
        &self.directory.path
    }

    fn bind_artifact(&mut self, artifact: CleanupArtifact) -> Result<(), String> {
        let expected_path = self.path();
        let binding_changed = artifact.path() != expected_path
            || artifact.directory_identity() != self.directory.identity;
        self.artifact = Some(artifact);
        if binding_changed {
            return Err(format!(
                "registration recovery artifact identity is not bound to its reserved directory: {}",
                expected_path.display()
            ));
        }
        self.directory.verify()?;
        Ok(())
    }

    fn artifact(&self) -> Option<&CleanupArtifact> {
        self.artifact.as_ref()
    }

    fn verify_artifact(&self) -> Result<(), String> {
        self.directory.verify()?;
        let artifact = self.artifact.as_ref().ok_or_else(|| {
            format!(
                "registration recovery artifact is no longer retained at {}",
                self.path().display()
            )
        })?;
        match verify_publication_artifact(artifact) {
            Ok(true) => Ok(()),
            Ok(false) => Err(format!(
                "registration recovery artifact identity is missing at {}",
                artifact.path().display()
            )),
            Err(warning) => Err(warning.to_string()),
        }
    }

    fn cleanup_file(&mut self) -> Result<(), CleanupWarning> {
        let Some(artifact) = self.artifact.as_ref() else {
            return Ok(());
        };
        cleanup_publication_artifact(artifact)?;
        self.artifact = None;
        Ok(())
    }

    fn advance_completed_file_cleanup(&mut self) -> Result<bool, String> {
        let Some(artifact) = self.artifact.as_ref() else {
            return Ok(false);
        };
        if !artifact.cleanup_completed().map_err(|error| {
            format!(
                "failed to inspect registration recovery cleanup state {}: {error}",
                artifact.path().display()
            )
        })? {
            return Ok(false);
        }
        self.artifact = None;
        Ok(true)
    }

    fn cleanup_directory(&mut self) -> Result<(), String> {
        if self.directory_removed {
            return Ok(());
        }
        if self.artifact.is_some() {
            return Err(format!(
                "registration recovery file is still present in {}",
                self.directory.path.display()
            ));
        }
        self.directory.verify()?;
        let name = self.directory.path.file_name().ok_or_else(|| {
            format!(
                "registration recovery directory has no child name: {}",
                self.directory.path.display()
            )
        })?;
        let mut retained = self.directory.directory.lock().map_err(|_| {
            format!(
                "registration recovery directory-handle state is poisoned: {}",
                self.directory.path.display()
            )
        })?;
        let retained_directory = retained.as_ref().ok_or_else(|| {
            format!(
                "retained registration recovery directory handle is missing: {}",
                self.directory.path.display()
            )
        })?;
        remove_identity_bound_empty_directory_child(
            &self.directory.parent,
            name,
            self.directory.identity,
            retained_directory,
        )
        .map_err(|error| {
            format!(
                "failed to remove identity-bound registration recovery directory {}: {error}",
                self.directory.path.display()
            )
        })?;
        *retained = None;
        self.directory_removed = true;
        Ok(())
    }
}

#[derive(Debug)]
struct PendingRecovery {
    cleanup: RecoveryCleanup,
    armed: bool,
}

#[derive(Debug, Default)]
struct RecoveryCleanupReport {
    diagnostics: Vec<String>,
    artifact_warnings: Vec<CleanupWarning>,
}

impl RecoveryCleanupReport {
    #[cfg(test)]
    fn is_empty(&self) -> bool {
        self.diagnostics.is_empty() && self.artifact_warnings.is_empty()
    }
}

impl PendingRecovery {
    fn cleanup(&mut self) -> RecoveryCleanupReport {
        if !self.armed {
            return RecoveryCleanupReport::default();
        }

        if let Err(warning) = self.cleanup.cleanup_file() {
            return RecoveryCleanupReport {
                diagnostics: vec![format!(
                    "failed to remove pending registration recovery {warning}"
                )],
                artifact_warnings: vec![warning],
            };
        }
        match self.cleanup.cleanup_directory() {
            Ok(()) => {
                self.armed = false;
                RecoveryCleanupReport::default()
            }
            Err(error) => RecoveryCleanupReport {
                diagnostics: vec![format!(
                    "failed to remove pending registration recovery directory: {error}"
                )],
                artifact_warnings: Vec::new(),
            },
        }
    }

    fn path(&self) -> PathBuf {
        self.cleanup.path()
    }

    fn bind_artifact(&mut self, artifact: CleanupArtifact) -> Result<(), String> {
        self.cleanup.bind_artifact(artifact)
    }

    fn defer_cleanup(&mut self) -> Option<RecoveryCleanup> {
        if !self.armed {
            return None;
        }
        self.armed = false;
        Some(self.cleanup.clone())
    }

    fn into_published(
        mut self,
        target: PathBuf,
        original: Vec<u8>,
        published: PublishedFileExpectation,
        original_permissions: PortablePermissions,
    ) -> PublishedRegistration {
        self.armed = false;
        PublishedRegistration {
            target,
            recovery: self.cleanup.clone(),
            original,
            published,
            original_permissions,
        }
    }
}

impl Drop for PendingRecovery {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.cleanup();
        }
    }
}

#[derive(Debug)]
struct PublishedRemoval {
    target: PathBuf,
    recovery: PathBuf,
    recovery_directory: PathBuf,
    snapshot: RemovalSnapshot,
}

#[derive(Debug)]
struct PendingRemovalRecovery {
    directory: PathBuf,
    path: PathBuf,
    armed: bool,
}

impl PendingRemovalRecovery {
    fn cleanup(&mut self) -> Vec<String> {
        if !self.armed {
            return Vec::new();
        }
        match fs::symlink_metadata(&self.path) {
            Ok(_) => {
                return vec![format!(
                    "pending removal recovery is preserved at {}",
                    self.path.display()
                )];
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => {
                return vec![format!(
                    "failed to inspect pending removal recovery {}: {error}",
                    self.path.display()
                )];
            }
        }
        match fs::remove_dir(&self.directory) {
            Ok(()) => {
                self.armed = false;
                Vec::new()
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {
                self.armed = false;
                Vec::new()
            }
            Err(error) => vec![format!(
                "failed to remove pending removal recovery directory {}: {error}",
                self.directory.display()
            )],
        }
    }

    fn into_published(mut self, target: PathBuf, snapshot: RemovalSnapshot) -> PublishedRemoval {
        self.armed = false;
        PublishedRemoval {
            target,
            recovery: self.path.clone(),
            recovery_directory: self.directory.clone(),
            snapshot,
        }
    }
}

impl Drop for PendingRemovalRecovery {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.cleanup();
        }
    }
}

#[derive(Debug, Default)]
struct PublishState {
    published_creates: Vec<PublishedCreate>,
    published_registrations: Vec<PublishedRegistration>,
    published_removals: Vec<PublishedRemoval>,
    created_dirs: Vec<PathBuf>,
    pending_artifact_cleanups: Vec<CleanupArtifact>,
    pending_recovery_cleanups: Vec<RecoveryCleanup>,
    cleanup_warnings: Vec<String>,
}

fn validate_published_file(path: &Path, expected: &[u8]) -> Result<(), String> {
    let actual = fs::read(path)
        .map_err(|error| format!("failed to validate published {}: {error}", path.display()))?;
    if actual != expected {
        return Err(format!(
            "post-write byte validation failed for {}",
            path.display()
        ));
    }
    validate_xml_when_applicable(path, &actual)
}

fn validate_xml_when_applicable(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let is_xml = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("xml"));
    if is_xml {
        validate_xml_bytes(path, bytes)
    } else {
        Ok(())
    }
}

fn validate_xml_bytes(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let (_, payload) = unica_xml_format::text::split_utf8_bom_prefix(bytes);
    let text = std::str::from_utf8(payload)
        .map_err(|error| format!("{} is not valid UTF-8: {error}", path.display()))?;
    Document::parse(text.trim_start_matches('\u{feff}'))
        .map(|_| ())
        .map_err(|error| format!("XML parse error in {}: {error}", path.display()))
}

fn snapshot_removal_path(path: &Path) -> Result<RemovalSnapshot, String> {
    fn visit(
        current: &Path,
        relative_path: PathBuf,
        entries: &mut Vec<RemovalEntry>,
    ) -> Result<(), String> {
        let metadata = fs::symlink_metadata(current).map_err(|error| {
            format!(
                "failed to inspect removal target {}: {error}",
                current.display()
            )
        })?;
        if metadata_is_link_or_reparse_point(&metadata) {
            return Err(format!(
                "removal target tree must not contain a symbolic link or reparse point: {}",
                current.display()
            ));
        }
        if metadata.is_file() {
            let mut file = fs::File::open(current).map_err(|error| {
                format!(
                    "failed to open removal target {}: {error}",
                    current.display()
                )
            })?;
            let link_count = hard_link_count(&file).map_err(|error| {
                format!(
                    "failed to inspect hard links for removal target {}: {error}",
                    current.display()
                )
            })?;
            if link_count != 1 {
                return Err(format!(
                    "removal target has multiple hard links ({link_count}): {}",
                    current.display()
                ));
            }
            let mut hasher = Sha256::new();
            let mut buffer = vec![0u8; 64 * 1024];
            let mut size = 0u64;
            loop {
                let count = file.read(&mut buffer).map_err(|error| {
                    format!(
                        "failed to read removal target {}: {error}",
                        current.display()
                    )
                })?;
                if count == 0 {
                    break;
                }
                size = size
                    .checked_add(count as u64)
                    .ok_or_else(|| format!("removal target is too large: {}", current.display()))?;
                hasher.update(&buffer[..count]);
            }
            let final_link_count = hard_link_count(&file).map_err(|error| {
                format!(
                    "failed to recheck hard links for removal target {}: {error}",
                    current.display()
                )
            })?;
            if final_link_count != 1 {
                return Err(format!(
                    "removal target has multiple hard links ({final_link_count}): {}",
                    current.display()
                ));
            }
            let final_path_metadata = fs::symlink_metadata(current).map_err(|error| {
                format!(
                    "failed to recheck removal target {}: {error}",
                    current.display()
                )
            })?;
            if metadata_is_link_or_reparse_point(&final_path_metadata)
                || !final_path_metadata.is_file()
            {
                return Err(format!(
                    "removal target type changed while snapshotting: {}",
                    current.display()
                ));
            }
            if final_path_metadata.len() != size {
                return Err(format!(
                    "removal target size changed while snapshotting: {}",
                    current.display()
                ));
            }
            let sha256 = hasher.finalize().into();
            entries.push(RemovalEntry {
                relative_path,
                kind: RemovalEntryKind::File { size, sha256 },
            });
            return Ok(());
        }
        if !metadata.is_dir() {
            return Err(format!(
                "removal target is neither a regular file nor a directory: {}",
                current.display()
            ));
        }

        entries.push(RemovalEntry {
            relative_path: relative_path.clone(),
            kind: RemovalEntryKind::Directory,
        });
        let mut children = fs::read_dir(current)
            .map_err(|error| {
                format!(
                    "failed to read removal directory {}: {error}",
                    current.display()
                )
            })?
            .map(|entry| {
                entry
                    .map(|entry| (entry.file_name(), entry.path()))
                    .map_err(|error| {
                        format!(
                            "failed to read removal directory entry in {}: {error}",
                            current.display()
                        )
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        children.sort_by(|left, right| left.0.cmp(&right.0));
        for (name, child) in children {
            visit(&child, relative_path.join(name), entries)?;
        }
        Ok(())
    }

    let mut entries = Vec::new();
    visit(path, PathBuf::new(), &mut entries)?;
    Ok(RemovalSnapshot { entries })
}

fn removal_snapshot_direct_entry_names(
    snapshot: &RemovalSnapshot,
) -> Result<Vec<OsString>, String> {
    let root_is_directory = snapshot.entries.iter().any(|entry| {
        entry.relative_path.as_os_str().is_empty()
            && matches!(entry.kind, RemovalEntryKind::Directory)
    });
    if !root_is_directory {
        return Err("conditional removal target snapshot is not a directory".to_string());
    }

    let mut names = snapshot
        .entries
        .iter()
        .filter_map(|entry| {
            let mut components = entry.relative_path.components();
            let first = components.next()?;
            if components.next().is_some() {
                return None;
            }
            match first {
                std::path::Component::Normal(name) => Some(name.to_os_string()),
                _ => None,
            }
        })
        .collect::<Vec<_>>();
    names.sort();
    Ok(names)
}

fn recheck_removal(removal: &PlannedRemoval) -> Result<(), String> {
    let current = snapshot_removal_path(&removal.path)?;
    if current == removal.snapshot {
        Ok(())
    } else {
        Err(format!(
            "removal target changed after planning: {}",
            removal.path.display()
        ))
    }
}

fn reserve_removal_recovery(target: &Path) -> Result<PendingRemovalRecovery, String> {
    for attempt in 1..=16 {
        let directory = unique_recovery_directory(target);
        match fs::create_dir(&directory) {
            Ok(()) => {
                return Ok(PendingRemovalRecovery {
                    path: directory.join("original"),
                    directory,
                    armed: true,
                });
            }
            Err(error) if error.kind() == ErrorKind::AlreadyExists && attempt < 16 => continue,
            Err(error) => {
                return Err(format!(
                    "failed to reserve removal recovery for {} at {}: {error}",
                    target.display(),
                    directory.display()
                ));
            }
        }
    }
    Err(format!(
        "failed to reserve removal recovery for {}",
        target.display()
    ))
}

fn reserve_recovery(target: &Path) -> Result<PendingRecovery, String> {
    reserve_recovery_with(target, || unique_recovery_directory(target))
}

fn reserve_recovery_with(
    target: &Path,
    mut next_directory: impl FnMut() -> PathBuf,
) -> Result<PendingRecovery, String> {
    for attempt in 1..=16 {
        let directory = next_directory();
        match RecoveryCleanup::reserve(&directory) {
            Ok(cleanup) => {
                return Ok(PendingRecovery {
                    cleanup,
                    armed: true,
                });
            }
            Err((ErrorKind::AlreadyExists, _)) if attempt < 16 => continue,
            Err((_kind, error)) => {
                return Err(format!(
                    "failed to reserve no-clobber recovery for {} at {}: {error}",
                    target.display(),
                    directory.display()
                ));
            }
        }
    }
    Err(format!(
        "failed to reserve no-clobber recovery for {}",
        target.display()
    ))
}

fn unique_recovery_directory(target: &Path) -> PathBuf {
    let parent = usable_parent(target);
    let mut name = OsString::from(".");
    name.push(
        target
            .file_name()
            .unwrap_or_else(|| std::ffi::OsStr::new("compile")),
    );
    name.push(format!(
        ".unica-recovery-{}-{}",
        std::process::id(),
        RECOVERY_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    parent.join(name)
}

fn usable_parent(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn normalize_transaction_path_identity(path: &Path) -> Result<PathBuf, String> {
    #[cfg(any(test, feature = "test-support"))]
    TEST_PATH_IDENTITY_NORMALIZATIONS.with(|count| count.set(count.get() + 1));
    normalize_path_identity(path).map_err(|error| {
        format!(
            "failed to normalize compile transaction path identity {}: {error}",
            path.display()
        )
    })
}

fn recheck_planned_path_identity(
    path: &Path,
    planned_identity: &Path,
    phase: &str,
) -> Result<(), String> {
    let current_identity = normalize_transaction_path_identity(path)?;
    if current_identity == planned_identity {
        return Ok(());
    }
    Err(format!(
        "publication target identity changed after planning ({phase}): {}",
        path.display()
    ))
}

fn find_overlapping_planned_identity<'a>(
    identities: &'a BTreeMap<PathBuf, PlannedPathKind>,
    target: &Path,
) -> Option<(&'a PathBuf, &'a PlannedPathKind)> {
    for ancestor in target.ancestors() {
        if let Some(existing) = identities.get_key_value(ancestor) {
            return Some(existing);
        }
    }
    identities
        .range(target.to_path_buf()..)
        .next()
        .filter(|(identity, _)| identity.starts_with(target))
}

fn ensure_parent_directories(path: &Path, created_dirs: &mut Vec<PathBuf>) -> Result<(), String> {
    let mut current = usable_parent(path).to_path_buf();
    let mut missing = Vec::new();
    loop {
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if !metadata.is_dir() {
                    return Err(format!(
                        "compile target parent is not a directory: {}",
                        current.display()
                    ));
                }
                break;
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {
                missing.push(current.clone());
                let Some(parent) = current.parent() else {
                    return Err(format!(
                        "cannot find an existing ancestor for {}",
                        path.display()
                    ));
                };
                current = if parent.as_os_str().is_empty() {
                    PathBuf::from(".")
                } else {
                    parent.to_path_buf()
                };
            }
            Err(error) => {
                return Err(format!(
                    "failed to inspect parent {}: {error}",
                    current.display()
                ));
            }
        }
    }

    for directory in missing.into_iter().rev() {
        match fs::create_dir(&directory) {
            Ok(()) => created_dirs.push(directory),
            Err(error) if error.kind() == ErrorKind::AlreadyExists && directory.is_dir() => {}
            Err(error) => {
                return Err(format!(
                    "failed to create directory {}: {error}",
                    directory.display()
                ));
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
enum PublicationRole {
    Create,
    Registration,
    Recovery,
    Transaction,
}

fn adapt_publish_error(error: &PublishError, role: PublicationRole) -> CommitFailure {
    let message = match error.kind() {
        PublishErrorKind::StalePreimage { target } => match role {
            PublicationRole::Registration => format!(
                "registration target changed after planning: {}",
                target.display()
            ),
            PublicationRole::Create | PublicationRole::Recovery | PublicationRole::Transaction => {
                error.to_string()
            }
        },
        PublishErrorKind::MetadataChanged { target } => match role {
            PublicationRole::Registration => format!(
                "registration target metadata changed after planning: {}",
                target.display()
            ),
            PublicationRole::Create | PublicationRole::Recovery | PublicationRole::Transaction => {
                error.to_string()
            }
        },
        PublishErrorKind::MissingTarget { target } => match role {
            PublicationRole::Registration => format!(
                "registration target disappeared before commit: {}",
                target.display()
            ),
            PublicationRole::Create | PublicationRole::Recovery | PublicationRole::Transaction => {
                error.to_string()
            }
        },
        PublishErrorKind::InvalidTarget { .. }
        | PublishErrorKind::AlreadyExists { .. }
        | PublishErrorKind::LinkOrReparsePoint { .. }
        | PublishErrorKind::NonRegular { .. }
        | PublishErrorKind::ReadOnly { .. }
        | PublishErrorKind::MultipleHardLinks { .. }
        | PublishErrorKind::StageCollisionsExhausted { .. }
        | PublishErrorKind::Io { .. } => error.to_string(),
    };
    let kind = match error.kind() {
        PublishErrorKind::AlreadyExists { .. }
        | PublishErrorKind::MissingTarget { .. }
        | PublishErrorKind::LinkOrReparsePoint { .. }
        | PublishErrorKind::NonRegular { .. }
        | PublishErrorKind::MultipleHardLinks { .. }
        | PublishErrorKind::StalePreimage { .. }
        | PublishErrorKind::MetadataChanged { .. } => CommitFailureKind::ConcurrentModification,
        PublishErrorKind::InvalidTarget { .. }
        | PublishErrorKind::ReadOnly { .. }
        | PublishErrorKind::StageCollisionsExhausted { .. }
        | PublishErrorKind::Io { .. } => CommitFailureKind::ProviderUnavailable,
    };
    CommitFailure { kind, message }
}

fn record_publish_error_cleanup(state: &mut PublishState, error: &PublishError) {
    record_cleanup_warnings(state, error.cleanup_warnings().iter().cloned());
}

fn record_cleanup_warnings(
    state: &mut PublishState,
    warnings: impl IntoIterator<Item = CleanupWarning>,
) {
    for warning in warnings {
        if !state.pending_artifact_cleanups.contains(&warning.artifact) {
            state
                .pending_artifact_cleanups
                .push(warning.artifact.clone());
        }
        state.cleanup_warnings.push(warning.to_string());
    }
}

fn record_cleanup_strings(state: &mut PublishState, warnings: impl IntoIterator<Item = String>) {
    state.cleanup_warnings.extend(warnings);
}

fn record_recovery_cleanup(state: &mut PublishState, recovery: &mut PendingRecovery) {
    let report = recovery.cleanup();
    record_cleanup_warnings(state, report.artifact_warnings);
    record_cleanup_strings(state, report.diagnostics);
    if let Some(cleanup) = recovery.defer_cleanup() {
        state.pending_recovery_cleanups.push(cleanup);
    }
}

fn discard_prepared<'request, 'lock, 'scope>(
    state: &mut PublishState,
    creates: &mut VecDeque<(
        &'request PlannedCreate,
        PreparedCreate<'request, 'lock, 'scope>,
    )>,
    registrations: &mut VecDeque<(
        &'request PlannedRegistration,
        PreparedReplace<'request, 'lock, 'scope>,
    )>,
) {
    while let Some((_create_plan, prepared)) = creates.pop_front() {
        record_cleanup_warnings(state, prepared.discard());
    }
    while let Some((_registration_plan, prepared)) = registrations.pop_front() {
        record_cleanup_warnings(state, prepared.discard());
    }
}

fn discard_prepared_removals(
    state: &mut PublishState,
    removals: &mut VecDeque<(&PlannedRemoval, PendingRemovalRecovery)>,
) {
    while let Some((_removal, mut recovery)) = removals.pop_front() {
        record_cleanup_strings(state, recovery.cleanup());
    }
}

fn retry_warned_artifacts(state: &mut PublishState) -> Vec<String> {
    #[cfg(any(test, feature = "test-support"))]
    run_before_cleanup_retry();
    let mut errors = Vec::new();
    for artifact in std::mem::take(&mut state.pending_artifact_cleanups) {
        if let Err(warning) = cleanup_publication_artifact(&artifact) {
            errors.push(format!("failed to retry publication cleanup {warning}"));
            state.pending_artifact_cleanups.push(artifact);
        }
    }
    errors
}

fn finish_pending_recovery_cleanups(state: &mut PublishState) -> Vec<String> {
    let mut errors = Vec::new();
    let mut pending = Vec::new();
    for mut cleanup in std::mem::take(&mut state.pending_recovery_cleanups) {
        if let Err(error) = cleanup.advance_completed_file_cleanup() {
            errors.push(error);
            pending.push(cleanup);
            continue;
        }
        if cleanup.artifact().is_some() {
            pending.push(cleanup);
            continue;
        }
        if let Err(error) = cleanup.cleanup_directory() {
            errors.push(format!(
                "failed to finish pending registration recovery cleanup {}: {error}",
                cleanup.directory_path().display()
            ));
            pending.push(cleanup);
        }
    }
    state.pending_recovery_cleanups = pending;
    errors
}

fn finish_retried_registration_recoveries(
    registrations: &mut [PublishedRegistration],
) -> Vec<String> {
    let mut errors = Vec::new();
    for published in registrations {
        match published.recovery.advance_completed_file_cleanup() {
            Ok(_) => {}
            Err(error) => {
                errors.push(error);
                continue;
            }
        }
        if published.recovery.artifact().is_none() {
            if let Err(error) = published.recovery.cleanup_directory() {
                errors.push(format!(
                    "failed to finish registration recovery cleanup {}: {error}",
                    published.recovery.directory_path().display()
                ));
            }
        }
    }
    errors
}

/// Retries identity-bound files before advancing their recovery owners to
/// directory cleanup. The order is load-bearing because both owner queues
/// observe completion through the shared file-handle slot.
fn retry_and_finish_recovery_cleanups(state: &mut PublishState) -> Vec<String> {
    let mut errors = retry_warned_artifacts(state);
    errors.extend(finish_pending_recovery_cleanups(state));
    errors.extend(finish_retried_registration_recoveries(
        &mut state.published_registrations,
    ));
    errors
}

fn cleanup_created_directories(created_dirs: &mut Vec<PathBuf>) -> Vec<String> {
    let mut errors = Vec::new();
    for directory in created_dirs.iter().rev() {
        match fs::remove_dir(directory) {
            Ok(()) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    ErrorKind::NotFound | ErrorKind::DirectoryNotEmpty
                ) => {}
            Err(error) => errors.push(format!(
                "failed to remove transaction-created directory {}: {error}",
                directory.display()
            )),
        }
    }
    created_dirs.clear();
    errors
}

fn with_cleanup_diagnostics(mut primary: CommitFailure, diagnostics: Vec<String>) -> CommitFailure {
    if diagnostics.is_empty() {
        primary
    } else {
        primary.message = format!(
            "{}; cleanup encountered: {}",
            primary.message,
            diagnostics.join("; ")
        );
        primary
    }
}

fn with_rollback_diagnostics(primary: CommitFailure, diagnostics: Vec<String>) -> CommitFailure {
    if diagnostics.is_empty() {
        primary
    } else {
        CommitFailure::rollback(format!(
            "{}; rollback encountered: {}",
            primary.message,
            diagnostics.join("; ")
        ))
    }
}

fn remove_if_exists(path: &Path) -> std::io::Result<()> {
    prepare_file_for_removal(path)?;
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn remove_recovery_path(path: &Path) -> std::io::Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if metadata.is_dir() {
        fs::remove_dir_all(path)
    } else {
        remove_if_exists(path)
    }
}

enum PublishedTargetState {
    Matches,
    Missing,
    Conflict(String),
}

fn inspect_published_target(
    path: &Path,
    expected: &PublishedFileExpectation,
) -> PublishedTargetState {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return PublishedTargetState::Missing;
        }
        Err(error) => {
            return PublishedTargetState::Conflict(format!(
                "failed to inspect the published path: {error}"
            ));
        }
    };
    if metadata_is_link_or_reparse_point(&metadata) {
        return PublishedTargetState::Conflict(
            "the published path became a symbolic link or reparse point".to_string(),
        );
    }
    if !metadata.is_file() {
        return PublishedTargetState::Conflict(
            "the published path is no longer a regular file".to_string(),
        );
    }

    let mut file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) => {
            return PublishedTargetState::Conflict(format!(
                "failed to open the published path: {error}"
            ));
        }
    };
    let identity = match file_identity(&file) {
        Ok(identity) => identity,
        Err(error) => {
            return PublishedTargetState::Conflict(format!(
                "failed to identify the published file: {error}"
            ));
        }
    };
    if identity != expected.identity {
        return PublishedTargetState::Conflict(
            "the path now names a different file identity".to_string(),
        );
    }
    let links = match hard_link_count(&file) {
        Ok(links) => links,
        Err(error) => {
            return PublishedTargetState::Conflict(format!(
                "failed to inspect published hard links: {error}"
            ));
        }
    };
    if links != 1 {
        return PublishedTargetState::Conflict(format!(
            "the published file now has {links} hard links"
        ));
    }

    let mut bytes = Vec::new();
    if let Err(error) = file.read_to_end(&mut bytes) {
        return PublishedTargetState::Conflict(format!(
            "failed to read the published file: {error}"
        ));
    }
    let sha256: [u8; 32] = Sha256::digest(&bytes).into();
    if sha256 != expected.sha256 || bytes != expected.bytes {
        return PublishedTargetState::Conflict(
            "the published file bytes changed after publication".to_string(),
        );
    }

    let final_metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) => {
            return PublishedTargetState::Conflict(format!(
                "failed to recheck the published path: {error}"
            ));
        }
    };
    if metadata_is_link_or_reparse_point(&final_metadata) || !final_metadata.is_file() {
        return PublishedTargetState::Conflict(
            "the published path type changed during rollback inspection".to_string(),
        );
    }
    let final_file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) => {
            return PublishedTargetState::Conflict(format!(
                "failed to reopen the published path: {error}"
            ));
        }
    };
    match file_identity(&final_file) {
        Ok(final_identity) if final_identity == expected.identity => PublishedTargetState::Matches,
        Ok(_) => PublishedTargetState::Conflict(
            "the published path identity changed during rollback inspection".to_string(),
        ),
        Err(error) => PublishedTargetState::Conflict(format!(
            "failed to re-identify the published path: {error}"
        )),
    }
}

fn reserve_rollback_quarantine(target: &Path) -> Result<(PathBuf, PathBuf), String> {
    for attempt in 1..=16 {
        let directory = unique_recovery_directory(target);
        match fs::create_dir(&directory) {
            Ok(()) => {
                let path = directory.join("published");
                return Ok((directory, path));
            }
            Err(error) if error.kind() == ErrorKind::AlreadyExists && attempt < 16 => continue,
            Err(error) => {
                return Err(format!(
                    "failed to reserve rollback quarantine for {} at {}: {error}",
                    target.display(),
                    directory.display()
                ));
            }
        }
    }
    Err(format!(
        "failed to reserve rollback quarantine for {}",
        target.display()
    ))
}

fn restore_quarantined_file_no_clobber(
    quarantine_parent: &fs::File,
    quarantine: &Path,
    target_parent: &fs::File,
    target: &Path,
) -> Result<(), String> {
    let quarantine_parent_path = quarantine.parent().ok_or_else(|| {
        format!(
            "rollback quarantine has no parent directory: {}",
            quarantine.display()
        )
    })?;
    let quarantine_name = quarantine.file_name().ok_or_else(|| {
        format!(
            "rollback quarantine has no child name: {}",
            quarantine.display()
        )
    })?;
    let target_parent_path = target.parent().ok_or_else(|| {
        format!(
            "restored rollback target has no parent directory: {}",
            target.display()
        )
    })?;
    let target_name = target.file_name().ok_or_else(|| {
        format!(
            "restored rollback target has no child name: {}",
            target.display()
        )
    })?;
    let retained_quarantine = open_regular_child_nofollow(quarantine_parent, quarantine_name)
        .map_err(|error| {
            format!(
                "failed to retain quarantined concurrent file {} before restoration: {error}",
                quarantine.display()
            )
        })?;
    let quarantine_identity = file_identity(&retained_quarantine).map_err(|error| {
        format!(
            "failed to capture quarantined concurrent file identity {}: {error}",
            quarantine.display()
        )
    })?;

    run_before_rollback_quarantine_cleanup(quarantine);
    let current_quarantine_parent =
        open_directory_nofollow(quarantine_parent_path).map_err(|error| {
            format!(
                "failed to verify rollback quarantine route {}: {error}",
                quarantine_parent_path.display()
            )
        })?;
    if file_identity(&current_quarantine_parent).map_err(|error| {
        format!(
            "failed to capture rollback quarantine route identity {}: {error}",
            quarantine_parent_path.display()
        )
    })? != file_identity(quarantine_parent).map_err(|error| {
        format!(
            "failed to capture retained rollback quarantine directory identity {}: {error}",
            quarantine_parent_path.display()
        )
    })? {
        return Err(format!(
            "rollback quarantine directory identity changed at {}; replacement left untouched",
            quarantine_parent_path.display()
        ));
    }
    let current_quarantine =
        open_regular_child_nofollow(&current_quarantine_parent, quarantine_name).map_err(
            |error| {
                format!(
                    "failed to verify quarantined concurrent file route {}: {error}",
                    quarantine.display()
                )
            },
        )?;
    if file_identity(&current_quarantine).map_err(|error| {
        format!(
            "failed to capture current quarantined file identity {}: {error}",
            quarantine.display()
        )
    })? != quarantine_identity
    {
        return Err(format!(
            "quarantined concurrent file identity changed at {}; replacement left untouched",
            quarantine.display()
        ));
    }
    let current_target_parent = open_directory_nofollow(target_parent_path).map_err(|error| {
        format!(
            "failed to verify rollback target parent route {}: {error}",
            target_parent_path.display()
        )
    })?;
    if file_identity(&current_target_parent).map_err(|error| {
        format!(
            "failed to capture rollback target parent route identity {}: {error}",
            target_parent_path.display()
        )
    })? != file_identity(target_parent).map_err(|error| {
        format!(
            "failed to capture retained rollback target parent identity {}: {error}",
            target_parent_path.display()
        )
    })? {
        return Err(format!(
            "rollback target parent identity changed at {}; replacement left untouched",
            target_parent_path.display()
        ));
    }
    drop(current_quarantine);
    drop(current_quarantine_parent);
    drop(current_target_parent);

    run_before_rollback_quarantine_restore_rename(quarantine, target);
    rename_identity_bound_regular_child_no_replace(
        quarantine_parent,
        quarantine_name,
        quarantine_identity,
        &retained_quarantine,
        target_parent,
        target_name,
    )
    .map_err(|error| {
        format!(
            "failed to restore quarantined concurrent file {} to {} without clobbering: {error}",
            quarantine.display(),
            target.display()
        )
    })?;
    let restored_target =
        open_regular_child_nofollow(target_parent, target_name).map_err(|error| {
            format!(
                "failed to verify restored rollback target {}: {error}",
                target.display()
            )
        })?;
    let restored_identity = file_identity(&restored_target).map_err(|error| {
        format!(
            "failed to capture restored rollback target identity {}: {error}",
            target.display()
        )
    })?;
    if restored_identity != quarantine_identity {
        return Err(format!(
            "restored rollback target identity differs from retained quarantine {}; unexpected target is left untouched at {}",
            quarantine.display(),
            target.display()
        ));
    }
    Ok(())
}

fn rollback_registration(
    published: &mut PublishedRegistration,
    errors: &mut Vec<String>,
    cleanup_warnings: &mut Vec<CleanupWarning>,
) {
    let recovery_path = published.recovery.path();
    let recovery_directory = published.recovery.directory_path().to_path_buf();
    if let Err(error) = published.recovery.verify_artifact() {
        errors.push(format!(
            "rollback conflict for registration {}: {error}; recovery route is unverified at {}",
            published.target.display(),
            recovery_path.display()
        ));
        return;
    }
    match inspect_published_target(&published.target, &published.published) {
        PublishedTargetState::Matches => {}
        PublishedTargetState::Missing => {
            errors.push(format!(
                "rollback conflict for registration {}: the published target was removed concurrently; recovery is preserved at {}",
                published.target.display(),
                recovery_path.display()
            ));
            return;
        }
        PublishedTargetState::Conflict(reason) => {
            errors.push(format!(
                "rollback conflict for registration {}: {reason}; concurrent target is preserved; recovery is preserved at {}",
                published.target.display(),
                recovery_path.display()
            ));
            return;
        }
    }
    run_before_rollback_mutation(&published.target);
    if let Err(error) = published.recovery.verify_artifact() {
        errors.push(format!(
            "rollback conflict for registration {}: {error}; published target and recovery are left untouched",
            published.target.display()
        ));
        return;
    }

    let quarantined = recovery_directory.join("published");
    if let Err(error) = rename_no_replace(&published.target, &quarantined) {
        let message = if fs::symlink_metadata(&quarantined).is_ok() {
            format!(
                "rollback conflict for registration {}: rollback quarantine appeared concurrently at {}; published target and recovery are preserved at {}: {error}",
                published.target.display(),
                quarantined.display(),
                recovery_path.display()
            )
        } else {
            format!(
                "failed to quarantine published registration {} without clobbering before rollback; recovery is preserved at {}: {error}",
                published.target.display(),
                recovery_path.display()
            )
        };
        errors.push(message);
        return;
    }
    let quarantine_name = match quarantined.file_name() {
        Some(name) => name.to_os_string(),
        None => {
            errors.push(format!(
                "rollback quarantine has no child name after moving registration {}; recovery and quarantined publication are preserved at {}",
                published.target.display(),
                recovery_path.display()
            ));
            return;
        }
    };
    let quarantine_directory_handle = match published.recovery.directory.try_clone_directory() {
        Ok(directory) => directory,
        Err(error) => {
            errors.push(format!(
                "failed to retain rollback quarantine directory after moving registration {}; recovery and quarantined publication are preserved: {error}",
                published.target.display()
            ));
            return;
        }
    };
    let target_parent_handle = Arc::clone(&published.recovery.directory.parent);
    match inspect_published_target(&quarantined, &published.published) {
        PublishedTargetState::Matches => {}
        PublishedTargetState::Missing => {
            errors.push(format!(
                "rollback conflict for registration {}: quarantined publication disappeared; original recovery is preserved at {}",
                published.target.display(),
                recovery_path.display()
            ));
            return;
        }
        PublishedTargetState::Conflict(reason) => {
            let restore = restore_quarantined_file_no_clobber(
                &quarantine_directory_handle,
                &quarantined,
                &target_parent_handle,
                &published.target,
            );
            let preservation = match restore {
                Ok(()) => "concurrent target was restored without clobbering".to_string(),
                Err(error) => format!(
                    "{error}; concurrent file is preserved at {}",
                    quarantined.display()
                ),
            };
            errors.push(format!(
                "rollback conflict for registration {}: {reason}; {preservation}; original recovery is preserved at {}",
                published.target.display(),
                recovery_path.display()
            ));
            return;
        }
    }
    let quarantined_file = match retain_regular_child_for_cleanup(
        &quarantine_directory_handle,
        &quarantine_name,
        published.published.identity,
    ) {
        Ok(file) => file,
        Err(error) => {
            errors.push(format!(
                "failed to bind quarantined registration {} to its published identity; recovery and quarantined publication are preserved at {}: {error}",
                published.target.display(),
                quarantined.display()
            ));
            return;
        }
    };

    if let Err(error) = published.recovery.verify_artifact() {
        let restore = restore_quarantined_file_no_clobber(
            &quarantine_directory_handle,
            &quarantined,
            &target_parent_handle,
            &published.target,
        );
        errors.push(format!(
            "rollback conflict for registration {}: recovery identity changed before restoration: {error}; published-byte restoration: {}",
            published.target.display(),
            restore
                .map(|()| "restored".to_string())
                .unwrap_or_else(|restore_error| format!("failed: {restore_error}"))
        ));
        return;
    }
    if let Err(error) = fs::hard_link(&recovery_path, &published.target) {
        errors.push(format!(
            "failed to restore registration {} without clobbering; original recovery is preserved at {} and published bytes at {}: {error}",
            published.target.display(),
            recovery_path.display(),
            quarantined.display()
        ));
        return;
    }
    run_after_registration_rollback_restore(&published.target, &quarantined);

    let bytes_restored = match fs::read(&published.target) {
        Ok(bytes) if bytes == published.original => true,
        Ok(_) => {
            errors.push(format!(
                "restored registration bytes differ from original: {}",
                published.target.display()
            ));
            false
        }
        Err(error) => {
            errors.push(format!(
                "failed to verify restored registration {}: {error}",
                published.target.display()
            ));
            false
        }
    };
    let permissions_restored = match fs::metadata(&published.target) {
        Ok(metadata) if published.original_permissions.matches(&metadata) => true,
        Ok(_) => {
            errors.push(format!(
                "restored registration permissions differ from original: {}",
                published.target.display()
            ));
            false
        }
        Err(error) => {
            errors.push(format!(
                "failed to verify restored registration permissions {}: {error}",
                published.target.display()
            ));
            false
        }
    };
    if !bytes_restored || !permissions_restored {
        errors.push(format!(
            "quarantined published bytes are preserved at {}",
            quarantined.display()
        ));
        preserve_rollback_recovery_copy(published, errors, cleanup_warnings);
        return;
    }
    if let Err(warning) = published.recovery.cleanup_file() {
        errors.push(format!(
            "restored registration {}, but failed to remove identity-bound original recovery {warning}",
            published.target.display()
        ));
        cleanup_warnings.push(warning);
    }
    run_before_rollback_quarantine_cleanup(&quarantined);
    let quarantine_cleanup = remove_identity_bound_regular_child(
        &quarantine_directory_handle,
        &quarantine_name,
        published.published.identity,
        &quarantined_file,
    );
    drop(quarantined_file);
    if let Err(error) = quarantine_cleanup {
        errors.push(format!(
            "restored registration {}, but failed to remove identity-bound quarantined published bytes {}; quarantine is preserved: {error}",
            published.target.display(),
            quarantined.display()
        ));
    }
    if published.recovery.artifact().is_none() {
        if let Err(error) = published.recovery.cleanup_directory() {
            errors.push(format!(
                "failed to remove restored registration recovery directory {}: {error}",
                recovery_directory.display()
            ));
        }
    }
}

fn rollback_create(published: &PublishedCreate, errors: &mut Vec<String>) {
    match inspect_published_target(&published.target, &published.published) {
        PublishedTargetState::Matches => {}
        PublishedTargetState::Missing => return,
        PublishedTargetState::Conflict(reason) => {
            errors.push(format!(
                "rollback conflict for create-only target {}: {reason}; concurrent target is preserved",
                published.target.display()
            ));
            return;
        }
    }
    run_before_rollback_mutation(&published.target);

    let (quarantine_directory, quarantined) = match reserve_rollback_quarantine(&published.target) {
        Ok(quarantine) => quarantine,
        Err(error) => {
            errors.push(error);
            return;
        }
    };
    if let Err(error) = rename_no_replace(&published.target, &quarantined) {
        let message = if fs::symlink_metadata(&quarantined).is_ok() {
            format!(
                "rollback conflict for create-only target {}: rollback quarantine appeared concurrently at {}; published target is preserved: {error}",
                published.target.display(),
                quarantined.display()
            )
        } else {
            format!(
                "failed to quarantine create-only target {} without clobbering before rollback: {error}",
                published.target.display()
            )
        };
        errors.push(message);
        let _ = fs::remove_dir(&quarantine_directory);
        return;
    }

    match inspect_published_target(&quarantined, &published.published) {
        PublishedTargetState::Matches => {
            let concurrent_target_exists = match fs::symlink_metadata(&published.target) {
                Err(error) if error.kind() == ErrorKind::NotFound => false,
                Ok(_) => true,
                Err(error) => {
                    errors.push(format!(
                        "failed to inspect create-only target {} after quarantine: {error}",
                        published.target.display()
                    ));
                    true
                }
            };
            if let Err(error) = remove_if_exists(&quarantined) {
                errors.push(format!(
                    "failed to remove quarantined create-only publication {}; recovery is preserved: {error}",
                    quarantined.display()
                ));
                return;
            }
            if concurrent_target_exists {
                errors.push(format!(
                    "rollback conflict for create-only target {}: a concurrent target appeared during rollback and is preserved",
                    published.target.display()
                ));
            }
        }
        PublishedTargetState::Missing => {
            errors.push(format!(
                "rollback conflict for create-only target {}: quarantined publication disappeared",
                published.target.display()
            ));
        }
        PublishedTargetState::Conflict(reason) => {
            let restore = open_directory_nofollow(&quarantine_directory)
                .map_err(|error| {
                    format!(
                        "failed to retain create-only rollback quarantine directory {}: {error}",
                        quarantine_directory.display()
                    )
                })
                .and_then(|quarantine_directory_handle| {
                    open_directory_nofollow(usable_parent(&published.target))
                        .map_err(|error| {
                            format!(
                                "failed to retain create-only rollback target parent {}: {error}",
                                usable_parent(&published.target).display()
                            )
                        })
                        .and_then(|target_parent_handle| {
                            restore_quarantined_file_no_clobber(
                                &quarantine_directory_handle,
                                &quarantined,
                                &target_parent_handle,
                                &published.target,
                            )
                        })
                });
            let preservation = match restore {
                Ok(()) => "concurrent target was restored without clobbering".to_string(),
                Err(error) => format!(
                    "{error}; concurrent file is preserved at {}",
                    quarantined.display()
                ),
            };
            errors.push(format!(
                "rollback conflict for create-only target {}: {reason}; {preservation}",
                published.target.display()
            ));
        }
    }
    if let Err(error) = fs::remove_dir(&quarantine_directory) {
        if error.kind() != ErrorKind::DirectoryNotEmpty {
            errors.push(format!(
                "failed to remove create-only rollback quarantine directory {}: {error}",
                quarantine_directory.display()
            ));
        }
    }
}

fn rollback(state: &mut PublishState) -> Vec<String> {
    let mut errors = Vec::new();
    let mut cleanup_warnings = Vec::new();

    for published in state.published_removals.iter().rev() {
        match fs::symlink_metadata(&published.target) {
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Ok(_) => {
                errors.push(format!(
                    "rollback conflict for removed target {}: a concurrent target already exists and is preserved; recovery is preserved at {}",
                    published.target.display(),
                    published.recovery.display()
                ));
                continue;
            }
            Err(error) => {
                errors.push(format!(
                    "failed to inspect removed target before rollback {}: {error}; recovery is preserved at {}",
                    published.target.display(),
                    published.recovery.display()
                ));
                continue;
            }
        }
        run_before_removal_rollback_restore(&published.recovery, &published.target);
        if let Err(error) = rename_no_replace(&published.recovery, &published.target) {
            let message = if fs::symlink_metadata(&published.target).is_ok() {
                format!(
                    "rollback conflict for removed target {}: a concurrent target appeared before atomic restoration and is preserved; recovery is preserved at {}: {error}",
                    published.target.display(),
                    published.recovery.display()
                )
            } else {
                format!(
                    "failed to restore removed target {} from {} without clobbering; recovery is preserved: {error}",
                    published.target.display(),
                    published.recovery.display()
                )
            };
            errors.push(message);
            continue;
        }
        match snapshot_removal_path(&published.target) {
            Ok(snapshot) if snapshot == published.snapshot => {}
            Ok(_) => errors.push(format!(
                "restored removal target differs from its planned snapshot: {}",
                published.target.display()
            )),
            Err(error) => errors.push(format!(
                "failed to validate restored removal target {}: {error}",
                published.target.display()
            )),
        }
        if let Err(error) = fs::remove_dir(&published.recovery_directory) {
            errors.push(format!(
                "failed to remove restored removal recovery directory {}: {error}",
                published.recovery_directory.display()
            ));
        }
    }

    for published in state.published_registrations.iter_mut().rev() {
        rollback_registration(published, &mut errors, &mut cleanup_warnings);
    }

    record_cleanup_warnings(state, cleanup_warnings);

    for published in state.published_creates.iter().rev() {
        rollback_create(published, &mut errors);
    }
    errors.extend(retry_and_finish_recovery_cleanups(state));
    errors.extend(cleanup_created_directories(&mut state.created_dirs));
    errors
}

fn preserve_rollback_recovery_copy(
    published: &mut PublishedRegistration,
    diagnostics: &mut Vec<String>,
    cleanup_warnings: &mut Vec<CleanupWarning>,
) {
    let recovery_path = published.recovery.path();
    if let Some(artifact) = published.recovery.artifact() {
        match verify_publication_artifact(artifact) {
            Ok(true) => diagnostics.push(format!(
                "registration recovery is preserved at {}",
                recovery_path.display()
            )),
            Ok(false) => diagnostics.push(format!(
                "failed to preserve recovery copy {}: its proven file identity is missing",
                recovery_path.display()
            )),
            Err(warning) => diagnostics.push(format!(
                "failed to preserve recovery copy {}: {warning}",
                recovery_path.display()
            )),
        }
        return;
    }
    if let Err(error) = published.recovery.directory.verify() {
        diagnostics.push(format!(
            "failed to preserve recovery copy {} in an unverified directory: {error}",
            recovery_path.display()
        ));
        return;
    }
    let recovery_directory = match published.recovery.directory.try_clone_directory() {
        Ok(directory) => directory,
        Err(error) => {
            diagnostics.push(format!(
                "failed to preserve recovery copy {}: {error}",
                recovery_path.display()
            ));
            return;
        }
    };
    match write_exact_new_file_in_directory(
        &recovery_path,
        recovery_directory,
        published.recovery.directory.identity,
        &published.original,
        &published.original_permissions,
    ) {
        Ok(artifact) => match published.recovery.bind_artifact(artifact) {
            Ok(()) => diagnostics.push(format!(
                "registration recovery is preserved at {}",
                recovery_path.display()
            )),
            Err(error) => diagnostics.push(format!(
                "failed to bind preserved recovery copy {} after rollback cleanup failure: {error}",
                recovery_path.display()
            )),
        },
        Err(error) => {
            cleanup_warnings.extend(error.cleanup_warnings().iter().cloned());
            diagnostics.push(format!(
                "failed to preserve recovery copy {} after rollback cleanup failure: {error}",
                recovery_path.display()
            ));
        }
    }
}

fn finalize_success(state: &mut PublishState) {
    let mut cleanup_warnings = Vec::new();
    for published in &state.published_removals {
        if let Err(error) = remove_recovery_path(&published.recovery) {
            state.cleanup_warnings.push(format!(
                "failed to remove recovery for deleted target {}; recovery is preserved at {}: {error}",
                published.target.display(),
                published.recovery.display()
            ));
            continue;
        }
        if let Err(error) = fs::remove_dir(&published.recovery_directory) {
            state.cleanup_warnings.push(format!(
                "failed to remove deleted-target recovery directory {}: {error}",
                published.recovery_directory.display()
            ));
        }
    }
    for published in &mut state.published_registrations {
        let recovery_path = published.recovery.path();
        let recovery_directory = published.recovery.directory_path().to_path_buf();
        if let Err(mut warning) = published.recovery.cleanup_file() {
            warning.message = format!(
                "failed to remove registration recovery; recovery is preserved at {}: {}",
                recovery_path.display(),
                warning.message
            );
            cleanup_warnings.push(warning);
            continue;
        }
        if let Err(error) = published.recovery.cleanup_directory() {
            state.cleanup_warnings.push(format!(
                "failed to remove registration recovery directory {}: {error}",
                recovery_directory.display()
            ));
        }
    }
    record_cleanup_warnings(state, cleanup_warnings);
    let recovery_warnings = retry_and_finish_recovery_cleanups(state);
    state.cleanup_warnings.extend(recovery_warnings);
}

#[cfg(any(test, feature = "test-support"))]
fn byte_diff(path: &Path, before: &[u8], after: &[u8]) -> RegistrationDiff {
    let prefix = unica_xml_format::text::common_prefix_len(before, after);
    let suffix = unica_xml_format::text::common_suffix_len(&before[prefix..], &after[prefix..]);
    let before_end = before.len() - suffix;
    let after_end = after.len() - suffix;
    RegistrationDiff {
        path: path.to_path_buf(),
        byte_range: prefix..before_end,
        before: before[prefix..before_end].to_vec(),
        after: after[prefix..after_end].to_vec(),
    }
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitFailpoint {
    AfterObjectFiles,
    AfterRegistrationBackup,
    PostWriteValidation,
}

#[cfg(any(test, feature = "test-support"))]
struct RegistrationRecoveryPause {
    ready: std::sync::mpsc::Sender<()>,
    release: std::sync::mpsc::Receiver<()>,
}

#[cfg(any(test, feature = "test-support"))]
type BeforeRollbackMutationHook = Box<dyn FnOnce(&Path)>;

#[cfg(any(test, feature = "test-support"))]
type BeforeRemovalRollbackRestoreHook = Box<dyn FnOnce(&Path, &Path)>;

#[cfg(any(test, feature = "test-support"))]
type BeforeCleanupRetryHook = Box<dyn FnOnce()>;

#[cfg(any(test, feature = "test-support"))]
type BeforeRollbackQuarantineCleanupHook = Box<dyn FnOnce(&Path)>;

#[cfg(any(test, feature = "test-support"))]
type BeforeRollbackQuarantineRestoreRenameHook = Box<dyn FnOnce(&Path, &Path)>;

#[cfg(any(test, feature = "test-support"))]
type AfterRegistrationRollbackRestoreHook = Box<dyn FnOnce(&Path, &Path)>;

#[cfg(any(test, feature = "test-support"))]
type RetainedApplyBeforePostValidationHook = Box<dyn FnOnce()>;

#[cfg(any(test, feature = "test-support"))]
type RetainedApplyBeforePostimagesHook = Box<dyn FnOnce()>;

#[cfg(any(test, feature = "test-support"))]
type RetainedApplyBeforeRollbackHook = Box<dyn FnOnce()>;

#[cfg(any(test, feature = "test-support"))]
type RetainedApplyBeforeProviderIoHook = Box<dyn FnOnce()>;

#[cfg(any(test, feature = "test-support"))]
type RetainedApplyBeforeRevisionValidationHook = Box<dyn FnOnce()>;

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static TEST_FAILPOINT: Cell<Option<CommitFailpoint>> = const { Cell::new(None) };
    static TEST_REGISTRATION_RECOVERY_PAUSE: RefCell<Option<RegistrationRecoveryPause>> = const { RefCell::new(None) };
    static TEST_PATH_IDENTITY_NORMALIZATIONS: Cell<usize> = const { Cell::new(0) };
    static TEST_BEFORE_ROLLBACK_MUTATION_HOOK: RefCell<Option<BeforeRollbackMutationHook>> = const { RefCell::new(None) };
    static TEST_BEFORE_REMOVAL_ROLLBACK_RESTORE_HOOK: RefCell<Option<BeforeRemovalRollbackRestoreHook>> = const { RefCell::new(None) };
    static TEST_BEFORE_CLEANUP_RETRY_HOOK: RefCell<Option<BeforeCleanupRetryHook>> = const { RefCell::new(None) };
    static TEST_BEFORE_ROLLBACK_QUARANTINE_CLEANUP_HOOK: RefCell<Option<BeforeRollbackQuarantineCleanupHook>> = const { RefCell::new(None) };
    static TEST_BEFORE_ROLLBACK_QUARANTINE_RESTORE_RENAME_HOOK: RefCell<Option<BeforeRollbackQuarantineRestoreRenameHook>> = const { RefCell::new(None) };
    static TEST_AFTER_REGISTRATION_ROLLBACK_RESTORE_HOOK: RefCell<Option<AfterRegistrationRollbackRestoreHook>> = const { RefCell::new(None) };
    static TEST_RETAINED_APPLY_BEFORE_POST_VALIDATION_HOOK: RefCell<Option<RetainedApplyBeforePostValidationHook>> = const { RefCell::new(None) };
    static TEST_RETAINED_APPLY_BEFORE_POSTIMAGES_HOOK: RefCell<Option<RetainedApplyBeforePostimagesHook>> = const { RefCell::new(None) };
    static TEST_RETAINED_APPLY_BEFORE_ROLLBACK_HOOK: RefCell<Option<RetainedApplyBeforeRollbackHook>> = const { RefCell::new(None) };
    static TEST_RETAINED_APPLY_BEFORE_PROVIDER_IO_HOOK: RefCell<Option<RetainedApplyBeforeProviderIoHook>> = const { RefCell::new(None) };
    static TEST_RETAINED_APPLY_BEFORE_REVISION_VALIDATION_HOOK: RefCell<Option<RetainedApplyBeforeRevisionValidationHook>> = const { RefCell::new(None) };
    static TEST_RETAINED_APPLY_AFTER_REFERENCE_STREAM_HOOK: RefCell<Option<RetainedApplyAfterReferenceStreamHook>> = const { RefCell::new(None) };
    static TEST_RETAINED_APPLY_AFTER_WRITE_TARGET_CHUNK_HOOK: RefCell<Option<RetainedApplyAfterWriteTargetChunkHook>> = const { RefCell::new(None) };
    static TEST_RETAINED_APPLY_VALIDATION_PROVIDER_FAILURE: Cell<bool> = const { Cell::new(false) };
}

#[cfg(any(test, feature = "test-support"))]
pub fn inject_retained_apply_validation_provider_failure_for_test() {
    TEST_RETAINED_APPLY_VALIDATION_PROVIDER_FAILURE.with(|slot| slot.set(true));
}

#[cfg(any(test, feature = "test-support"))]
pub fn set_retained_apply_before_post_validation_hook(hook: impl FnOnce() + 'static) {
    TEST_RETAINED_APPLY_BEFORE_POST_VALIDATION_HOOK.with(|slot| {
        slot.replace(Some(Box::new(hook)));
    });
}

#[cfg(any(test, feature = "test-support"))]
pub fn set_retained_apply_after_reference_stream_hook(
    skip_checks: usize,
    hook: impl FnOnce() + 'static,
) {
    TEST_RETAINED_APPLY_AFTER_REFERENCE_STREAM_HOOK.with(|slot| {
        slot.replace(Some((skip_checks, Box::new(hook))));
    });
}

#[cfg(any(test, feature = "test-support"))]
pub fn set_retained_apply_after_write_target_chunk_hook(hook: impl FnOnce() + 'static) {
    TEST_RETAINED_APPLY_AFTER_WRITE_TARGET_CHUNK_HOOK.with(|slot| {
        slot.replace(Some(Box::new(hook)));
    });
}

#[cfg(any(test, feature = "test-support"))]
fn run_retained_apply_after_write_target_chunk_hook() {
    if let Some(hook) =
        TEST_RETAINED_APPLY_AFTER_WRITE_TARGET_CHUNK_HOOK.with(|slot| slot.borrow_mut().take())
    {
        hook();
    }
}

#[cfg(any(test, feature = "test-support"))]
fn run_retained_apply_after_reference_stream_hook() {
    let hook = TEST_RETAINED_APPLY_AFTER_REFERENCE_STREAM_HOOK.with(|slot| {
        let mut pending = slot.borrow_mut();
        let (remaining, _) = pending.as_mut()?;
        if *remaining > 0 {
            *remaining -= 1;
            return None;
        }
        pending.take().map(|(_, hook)| hook)
    });
    if let Some(hook) = hook {
        hook();
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn set_retained_apply_before_postimages_hook(hook: impl FnOnce() + 'static) {
    TEST_RETAINED_APPLY_BEFORE_POSTIMAGES_HOOK.with(|slot| {
        slot.replace(Some(Box::new(hook)));
    });
}

#[cfg(any(test, feature = "test-support"))]
pub fn set_retained_apply_before_rollback_hook(hook: impl FnOnce() + 'static) {
    TEST_RETAINED_APPLY_BEFORE_ROLLBACK_HOOK.with(|slot| {
        slot.replace(Some(Box::new(hook)));
    });
}

#[cfg(any(test, feature = "test-support"))]
pub fn set_retained_apply_before_provider_io_hook(hook: impl FnOnce() + 'static) {
    TEST_RETAINED_APPLY_BEFORE_PROVIDER_IO_HOOK.with(|slot| {
        slot.replace(Some(Box::new(hook)));
    });
}

#[cfg(any(test, feature = "test-support"))]
pub fn set_retained_apply_before_revision_validation_hook(hook: impl FnOnce() + 'static) {
    TEST_RETAINED_APPLY_BEFORE_REVISION_VALIDATION_HOOK.with(|slot| {
        slot.replace(Some(Box::new(hook)));
    });
}

#[cfg(any(test, feature = "test-support"))]
fn run_retained_apply_before_provider_io_hook() {
    if let Some(hook) =
        TEST_RETAINED_APPLY_BEFORE_PROVIDER_IO_HOOK.with(|slot| slot.borrow_mut().take())
    {
        hook();
    }
}

#[cfg(any(test, feature = "test-support"))]
fn run_retained_apply_before_revision_validation_hook() {
    if let Some(hook) =
        TEST_RETAINED_APPLY_BEFORE_REVISION_VALIDATION_HOOK.with(|slot| slot.borrow_mut().take())
    {
        hook();
    }
}

#[cfg(any(test, feature = "test-support"))]
fn run_retained_apply_before_postimages_hook() {
    if let Some(hook) =
        TEST_RETAINED_APPLY_BEFORE_POSTIMAGES_HOOK.with(|slot| slot.borrow_mut().take())
    {
        hook();
    }
}

#[cfg(any(test, feature = "test-support"))]
fn run_retained_apply_before_rollback_hook() {
    if let Some(hook) =
        TEST_RETAINED_APPLY_BEFORE_ROLLBACK_HOOK.with(|slot| slot.borrow_mut().take())
    {
        hook();
    }
}

#[cfg(any(test, feature = "test-support"))]
fn run_retained_apply_before_post_validation_hook() {
    if let Some(hook) =
        TEST_RETAINED_APPLY_BEFORE_POST_VALIDATION_HOOK.with(|slot| slot.borrow_mut().take())
    {
        hook();
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn with_commit_failpoint<T>(failpoint: CommitFailpoint, action: impl FnOnce() -> T) -> T {
    struct Reset(Option<CommitFailpoint>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_FAILPOINT.with(|slot| slot.set(self.0));
        }
    }

    let previous = TEST_FAILPOINT.with(|slot| slot.replace(Some(failpoint)));
    let _reset = Reset(previous);
    action()
}

#[cfg(feature = "test-support")]
fn with_registration_recovery_pause<T>(
    ready: std::sync::mpsc::Sender<()>,
    release: std::sync::mpsc::Receiver<()>,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<RegistrationRecoveryPause>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_REGISTRATION_RECOVERY_PAUSE.with(|slot| slot.replace(self.0.take()));
        }
    }

    let pause = RegistrationRecoveryPause { ready, release };
    let previous = TEST_REGISTRATION_RECOVERY_PAUSE.with(|slot| slot.replace(Some(pause)));
    let _reset = Reset(previous);
    action()
}

#[cfg(any(test, feature = "test-support"))]
pub fn with_before_rollback_mutation_hook<T>(
    hook: impl FnOnce(&Path) + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<BeforeRollbackMutationHook>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_BEFORE_ROLLBACK_MUTATION_HOOK.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }

    let previous =
        TEST_BEFORE_ROLLBACK_MUTATION_HOOK.with(|slot| slot.replace(Some(Box::new(hook))));
    let _reset = Reset(previous);
    action()
}

#[cfg(test)]
fn with_before_removal_rollback_restore_hook<T>(
    hook: impl FnOnce(&Path, &Path) + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<BeforeRemovalRollbackRestoreHook>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_BEFORE_REMOVAL_ROLLBACK_RESTORE_HOOK.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }

    let previous =
        TEST_BEFORE_REMOVAL_ROLLBACK_RESTORE_HOOK.with(|slot| slot.replace(Some(Box::new(hook))));
    let _reset = Reset(previous);
    action()
}

#[cfg(feature = "test-support")]
fn with_before_cleanup_retry_hook<T>(
    hook: impl FnOnce() + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<BeforeCleanupRetryHook>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_BEFORE_CLEANUP_RETRY_HOOK.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }

    let previous = TEST_BEFORE_CLEANUP_RETRY_HOOK.with(|slot| slot.replace(Some(Box::new(hook))));
    let _reset = Reset(previous);
    action()
}

#[cfg(feature = "test-support")]
fn with_before_rollback_quarantine_cleanup_hook<T>(
    hook: impl FnOnce(&Path) + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<BeforeRollbackQuarantineCleanupHook>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_BEFORE_ROLLBACK_QUARANTINE_CLEANUP_HOOK.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }

    let previous = TEST_BEFORE_ROLLBACK_QUARANTINE_CLEANUP_HOOK
        .with(|slot| slot.replace(Some(Box::new(hook))));
    let _reset = Reset(previous);
    action()
}

#[cfg(feature = "test-support")]
fn with_before_rollback_quarantine_restore_rename_hook<T>(
    hook: impl FnOnce(&Path, &Path) + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<BeforeRollbackQuarantineRestoreRenameHook>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_BEFORE_ROLLBACK_QUARANTINE_RESTORE_RENAME_HOOK.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }

    let previous = TEST_BEFORE_ROLLBACK_QUARANTINE_RESTORE_RENAME_HOOK
        .with(|slot| slot.replace(Some(Box::new(hook))));
    let _reset = Reset(previous);
    action()
}

#[cfg(feature = "test-support")]
fn with_after_registration_rollback_restore_hook<T>(
    hook: impl FnOnce(&Path, &Path) + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<AfterRegistrationRollbackRestoreHook>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_AFTER_REGISTRATION_ROLLBACK_RESTORE_HOOK.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }

    let previous = TEST_AFTER_REGISTRATION_ROLLBACK_RESTORE_HOOK
        .with(|slot| slot.replace(Some(Box::new(hook))));
    let _reset = Reset(previous);
    action()
}

#[cfg(any(test, feature = "test-support"))]
fn run_before_cleanup_retry() {
    if let Some(hook) = TEST_BEFORE_CLEANUP_RETRY_HOOK.with(|slot| slot.borrow_mut().take()) {
        hook();
    }
}

fn run_before_rollback_quarantine_cleanup(_path: &Path) {
    #[cfg(any(test, feature = "test-support"))]
    if let Some(hook) =
        TEST_BEFORE_ROLLBACK_QUARANTINE_CLEANUP_HOOK.with(|slot| slot.borrow_mut().take())
    {
        hook(_path);
    }
}

fn run_before_rollback_quarantine_restore_rename(_quarantine: &Path, _target: &Path) {
    #[cfg(any(test, feature = "test-support"))]
    if let Some(hook) =
        TEST_BEFORE_ROLLBACK_QUARANTINE_RESTORE_RENAME_HOOK.with(|slot| slot.borrow_mut().take())
    {
        hook(_quarantine, _target);
    }
}

fn run_after_registration_rollback_restore(_target: &Path, _quarantine: &Path) {
    #[cfg(any(test, feature = "test-support"))]
    if let Some(hook) =
        TEST_AFTER_REGISTRATION_ROLLBACK_RESTORE_HOOK.with(|slot| slot.borrow_mut().take())
    {
        hook(_target, _quarantine);
    }
}

fn run_before_rollback_mutation(_path: &Path) {
    #[cfg(any(test, feature = "test-support"))]
    if let Some(hook) = TEST_BEFORE_ROLLBACK_MUTATION_HOOK.with(|slot| slot.borrow_mut().take()) {
        hook(_path);
    }
}

fn run_before_removal_rollback_restore(_recovery: &Path, _target: &Path) {
    #[cfg(any(test, feature = "test-support"))]
    if let Some(hook) =
        TEST_BEFORE_REMOVAL_ROLLBACK_RESTORE_HOOK.with(|slot| slot.borrow_mut().take())
    {
        hook(_recovery, _target);
    }
}

fn pause_after_registration_recovery() {
    #[cfg(any(test, feature = "test-support"))]
    let pause = TEST_REGISTRATION_RECOVERY_PAUSE.with(|slot| slot.borrow_mut().take());
    #[cfg(any(test, feature = "test-support"))]
    if let Some(pause) = pause {
        pause
            .ready
            .send(())
            .expect("registration recovery observer must remain available");
        pause
            .release
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("registration recovery observer must release the bounded pause");
    }
}

fn failpoint_after_object_files() -> Result<(), String> {
    #[cfg(any(test, feature = "test-support"))]
    if TEST_FAILPOINT.with(|slot| slot.get()) == Some(CommitFailpoint::AfterObjectFiles) {
        return Err("injected compile transaction failure after object files".to_string());
    }
    Ok(())
}

fn failpoint_after_registration_backup() -> Result<(), String> {
    #[cfg(any(test, feature = "test-support"))]
    if TEST_FAILPOINT.with(|slot| slot.get()) == Some(CommitFailpoint::AfterRegistrationBackup) {
        return Err("injected compile transaction failure after registration backup".to_string());
    }
    Ok(())
}

fn failpoint_post_write_validation() -> Result<(), String> {
    #[cfg(any(test, feature = "test-support"))]
    if TEST_FAILPOINT.with(|slot| slot.get()) == Some(CommitFailpoint::PostWriteValidation) {
        return Err("injected compile transaction post-write validation failure".to_string());
    }
    Ok(())
}

#[cfg(feature = "test-support")]
#[path = "compile_transaction_contract_tests.rs"]
pub mod contract_tests;
#[cfg(any(test, feature = "test-support"))]
#[path = "../tests/support/compile_transaction.rs"]
mod test_support;
#[cfg(test)]
#[path = "compile_transaction_local_tests.rs"]
pub(crate) mod tests;
