//! Source planning facts and owner-backed in-memory proposal ports.
use crate::events::DomainEvent;
use crate::read::FileIdentity;
use crate::workspace::WorkspaceContext;
use crate::{SourceProfile, SourceSetKind};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyStagingErrorKind {
    Cancelled,
    Deadline,
    ContainmentIdentity,
    AbsentChainOccupied,
    UnsupportedProvider,
    ConcurrentRevision,
    Invariant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyStagingError {
    kind: ApplyStagingErrorKind,
    message: String,
}

impl ApplyStagingError {
    pub fn new(kind: ApplyStagingErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn kind(&self) -> ApplyStagingErrorKind {
        self.kind
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn contains(&self, pattern: &str) -> bool {
        self.message.contains(pattern)
    }
}

impl std::fmt::Display for ApplyStagingError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.message.fmt(formatter)
    }
}

impl std::error::Error for ApplyStagingError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyPlanErrorKind {
    BadValue,
    NotFound,
    ProviderUnavailable,
    InvalidState,
    InvalidSource,
    Staging(ApplyStagingErrorKind),
    Postcondition,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyPlanError {
    kind: ApplyPlanErrorKind,
    path: Option<String>,
    message: String,
    /// The call that continues past this refusal, when the planner knows it.
    next: Option<Box<serde_json::Value>>,
}

impl ApplyPlanError {
    pub fn new(kind: ApplyPlanErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            path: None,
            message: message.into(),
            next: None,
        }
    }

    pub fn with_next(mut self, next: serde_json::Value) -> Self {
        self.next = Some(Box::new(next));
        self
    }

    pub fn next(&self) -> Option<&serde_json::Value> {
        self.next.as_deref()
    }

    pub const fn kind(&self) -> ApplyPlanErrorKind {
        self.kind
    }

    pub fn path(&self) -> Option<&str> {
        self.path.as_deref()
    }

    pub fn at_path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }

    /// Keeps the staging kind for the refusal code and names its cause:
    /// the agent needs to know what to fix, not that "evidence" is missing.
    pub fn staging(error: ApplyStagingError, path: impl Into<String>) -> Self {
        let cause = match error.kind() {
            ApplyStagingErrorKind::Cancelled => "apply planning was cancelled",
            ApplyStagingErrorKind::Deadline => "apply planning deadline elapsed",
            ApplyStagingErrorKind::ContainmentIdentity => {
                "a staged path is a link, a hard-link alias, outside the source root or in the generated subtree"
            }
            ApplyStagingErrorKind::AbsentChainOccupied => {
                "a staged path has another kind of entry in the sources than the plan needs"
            }
            ApplyStagingErrorKind::UnsupportedProvider => {
                "a staged source cannot be read within the apply bounds"
            }
            ApplyStagingErrorKind::ConcurrentRevision => {
                "sources changed while the plan was prepared; prepare a new preview"
            }
            ApplyStagingErrorKind::Invariant => "the staged apply plan is inconsistent",
        };
        Self::new(
            ApplyPlanErrorKind::Staging(error.kind()),
            format!("{cause}: {error}"),
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

pub fn empty_apply_family_batch() -> ApplyPlanError {
    ApplyPlanError::new(
        ApplyPlanErrorKind::BadValue,
        "apply family batch must contain at least one operation",
    )
    .at_path("ops")
}

pub fn hidden_apply_family_unimplemented(op_index: usize) -> ApplyPlanError {
    ApplyPlanError::new(
        ApplyPlanErrorKind::ProviderUnavailable,
        "hidden v0.13 apply family is not implemented",
    )
    .at_path(format!("ops[{op_index}].op"))
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BorrowPlanDetail {
    pub index: usize,
    pub at: String,
    pub from: String,
    pub parent_uuid: String,
    pub created: bool,
    pub changed_properties: Vec<String>,
    pub protected_overrides: Vec<String>,
}

/// Key of one family's section of plan details in the apply answer. A family
/// declares its key as a constant; the product composition registers every
/// key and publishes each non-empty section under it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PlanDetailKey(&'static str);

impl PlanDetailKey {
    pub const fn new(key: &'static str) -> Self {
        Self(key)
    }

    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct PlannedApplyEffects {
    events: Vec<DomainEvent>,
    /// The staged files each event stands for, parallel to `events`. A
    /// planner that knows which file its event describes records it here so
    /// request-level reconciliation can drop exactly the events whose file
    /// was restored; an empty list means "the whole batch".
    paths: Vec<Vec<PathBuf>>,
    /// Typed warnings a planner attaches to a plan that still executes: a
    /// forced removal names the files that keep referring to the object.
    warnings: Vec<serde_json::Value>,
    /// Request operations that attached a warning, by their index in `ops`.
    warned_operations: std::collections::BTreeSet<usize>,
    borrowing: Vec<BorrowPlanDetail>,
    /// Facts a family reports about the final plan, by the family's key. The
    /// contract carries the section without knowing its rows.
    details: std::collections::BTreeMap<PlanDetailKey, Vec<serde_json::Value>>,
}

impl PlannedApplyEffects {
    /// Replaces the family's section with its typed rows; an empty section is
    /// not published. Rows are serialized here and nowhere else.
    pub fn set_details<T: serde::Serialize>(
        &mut self,
        key: PlanDetailKey,
        rows: &[T],
    ) -> Result<(), serde_json::Error> {
        let rows = rows
            .iter()
            .map(serde_json::to_value)
            .collect::<Result<Vec<_>, _>>()?;
        if rows.is_empty() {
            self.details.remove(&key);
        } else {
            self.details.insert(key, rows);
        }
        Ok(())
    }

    /// Non-empty sections in key order.
    pub fn details(&self) -> impl Iterator<Item = (PlanDetailKey, &[serde_json::Value])> {
        self.details
            .iter()
            .map(|(key, rows)| (*key, rows.as_slice()))
    }

    pub fn borrowing(&self) -> &[BorrowPlanDetail] {
        &self.borrowing
    }
    pub fn set_borrowing(&mut self, details: Vec<BorrowPlanDetail>) {
        self.borrowing = details;
    }

    pub fn events(&self) -> &[DomainEvent] {
        &self.events
    }

    pub fn warnings(&self) -> &[serde_json::Value] {
        &self.warnings
    }

    pub fn push_warning(&mut self, warning: serde_json::Value) {
        self.warnings.push(warning);
    }

    /// A warning attached by one request operation, which the answer names.
    pub fn push_operation_warning(&mut self, operation_index: usize, warning: serde_json::Value) {
        self.warned_operations.insert(operation_index);
        self.warnings.push(warning);
    }

    pub fn warned_operations(&self) -> &std::collections::BTreeSet<usize> {
        &self.warned_operations
    }

    pub fn into_events(self) -> Vec<DomainEvent> {
        self.events
    }

    /// Events paired with the files they describe (empty when unknown).
    pub fn into_events_with_paths(self) -> Vec<(DomainEvent, Vec<PathBuf>)> {
        self.events.into_iter().zip(self.paths).collect()
    }

    pub fn append(&mut self, event: DomainEvent) {
        self.append_at(event, Vec::new());
    }

    /// Appends one event that describes the given staged files. A repeated
    /// event (same kind and artifact) merges its files into the first one.
    pub fn append_at(&mut self, event: DomainEvent, paths: Vec<PathBuf>) {
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
pub enum StagedFileState {
    Bytes(Vec<u8>),
    Absent,
}

impl StagedFileState {
    pub fn as_option(&self) -> Option<Vec<u8>> {
        match self {
            Self::Bytes(bytes) => Some(bytes.clone()),
            Self::Absent => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StagedChangeKind {
    Create,
    Replace,
    Remove,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedApplyChange {
    pub relative_path: PathBuf,
    pub kind: StagedChangeKind,
    pub original: StagedFileState,
    pub current: StagedFileState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupportPolicyMode {
    Deny,
    Warn,
    Off,
}

/// Read-only observations used for the early planner scope check. Equality
/// does not create a writer: publication still consumes the issuing owner's
/// original staged state and private writer authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProposalBinding {
    root: FileIdentity,
    writer_scope: u64,
}

impl ProposalBinding {
    pub const fn from_observation(root: FileIdentity, writer_scope: u64) -> Self {
        Self { root, writer_scope }
    }
}

/// The existing captured-input overlay, without finalization or publication.
pub trait SourceProposal {
    fn checkpoint(&self, phase: &str) -> Result<(), ApplyStagingError>;
    fn binding(&self) -> ProposalBinding;
    fn enumerate_tree(&mut self, relative: &Path) -> Result<Vec<PathBuf>, ApplyStagingError>;
    fn enumerate_tree_with_root_absence(
        &mut self,
        relative: &Path,
    ) -> Result<(Vec<PathBuf>, bool), ApplyStagingError>;
    fn read(&mut self, relative: &Path) -> Result<Option<Vec<u8>>, ApplyStagingError>;
    fn retained_relative_path(&mut self, relative: &Path) -> Result<PathBuf, ApplyStagingError>;
    fn require_exact_path_spelling(&mut self, relative: &Path) -> Result<(), ApplyStagingError>;
    fn read_bounded(
        &mut self,
        relative: &Path,
        limit: usize,
    ) -> Result<Option<Vec<u8>>, ApplyStagingError>;
    fn original_image(&self, relative: &Path) -> Option<&[u8]>;
    fn read_guarded_bounded(
        &mut self,
        relative: &Path,
        limit: usize,
    ) -> Result<Option<Vec<u8>>, ApplyStagingError>;
    fn create(&mut self, relative: &Path, bytes: Vec<u8>) -> Result<(), ApplyStagingError>;
    fn replace(
        &mut self,
        relative: &Path,
        expected_current: &[u8],
        bytes: Vec<u8>,
    ) -> Result<(), ApplyStagingError>;
    fn remove(&mut self, relative: &Path, expected_current: &[u8])
        -> Result<(), ApplyStagingError>;
    fn remove_owned_directories(
        &mut self,
        payload: &Path,
        descriptor: &Path,
        collection: &Path,
    ) -> Result<(), ApplyStagingError>;
    fn planned_changes(&self) -> Vec<StagedApplyChange>;
}

/// A borrowed proposal keeps the concrete owner, retained descriptors and
/// writer fields outside a codec's dependency graph.
pub struct ApplyProposal<'a> {
    owner: &'a mut dyn SourceProposal,
}

impl<'a> ApplyProposal<'a> {
    pub fn from_port(owner: &'a mut dyn SourceProposal) -> Self {
        Self { owner }
    }
    pub fn checkpoint(&self, phase: &str) -> Result<(), ApplyStagingError> {
        self.owner.checkpoint(phase)
    }
    pub fn binding(&self) -> ProposalBinding {
        self.owner.binding()
    }
    pub fn enumerate_tree(&mut self, relative: &Path) -> Result<Vec<PathBuf>, ApplyStagingError> {
        self.owner.enumerate_tree(relative)
    }
    pub fn enumerate_tree_with_root_absence(
        &mut self,
        relative: &Path,
    ) -> Result<(Vec<PathBuf>, bool), ApplyStagingError> {
        self.owner.enumerate_tree_with_root_absence(relative)
    }
    pub fn read(&mut self, relative: &Path) -> Result<Option<Vec<u8>>, ApplyStagingError> {
        self.owner.read(relative)
    }
    pub fn retained_relative_path(
        &mut self,
        relative: &Path,
    ) -> Result<PathBuf, ApplyStagingError> {
        self.owner.retained_relative_path(relative)
    }
    pub fn require_exact_path_spelling(
        &mut self,
        relative: &Path,
    ) -> Result<(), ApplyStagingError> {
        self.owner.require_exact_path_spelling(relative)
    }
    pub fn read_bounded(
        &mut self,
        relative: &Path,
        limit: usize,
    ) -> Result<Option<Vec<u8>>, ApplyStagingError> {
        self.owner.read_bounded(relative, limit)
    }
    pub fn original_image(&self, relative: &Path) -> Option<&[u8]> {
        self.owner.original_image(relative)
    }
    pub fn read_guarded_bounded(
        &mut self,
        relative: &Path,
        limit: usize,
    ) -> Result<Option<Vec<u8>>, ApplyStagingError> {
        self.owner.read_guarded_bounded(relative, limit)
    }
    pub fn create(
        &mut self,
        relative: impl AsRef<Path>,
        bytes: Vec<u8>,
    ) -> Result<(), ApplyStagingError> {
        self.owner.create(relative.as_ref(), bytes)
    }
    pub fn replace(
        &mut self,
        relative: impl AsRef<Path>,
        expected_current: impl AsRef<[u8]>,
        bytes: Vec<u8>,
    ) -> Result<(), ApplyStagingError> {
        self.owner
            .replace(relative.as_ref(), expected_current.as_ref(), bytes)
    }
    pub fn remove(
        &mut self,
        relative: impl AsRef<Path>,
        expected_current: impl AsRef<[u8]>,
    ) -> Result<(), ApplyStagingError> {
        self.owner
            .remove(relative.as_ref(), expected_current.as_ref())
    }
    pub fn remove_owned_directories(
        &mut self,
        payload: &Path,
        descriptor: &Path,
        collection: &Path,
    ) -> Result<(), ApplyStagingError> {
        self.owner
            .remove_owned_directories(payload, descriptor, collection)
    }
    pub fn planned_changes(&self) -> Vec<StagedApplyChange> {
        self.owner.planned_changes()
    }
}

/// Admission-issued, read-only planning facts. Concrete family capability
/// conversion is performed by that family from this exact admitted profile.
pub trait ApplySourceAuthority {
    fn source_set_name(&self) -> &str;
    fn source_kind(&self) -> SourceSetKind;
    fn source_profile(&self) -> SourceProfile;
    fn expected_format(&self) -> &str;
    fn support_policy_mode(&self) -> SupportPolicyMode;
    fn workspace_context(&self) -> &WorkspaceContext;
    fn source_root(&self) -> &Path;
    fn source_reader(&self) -> std::sync::Arc<crate::read::SourceDirectory>;
    fn binding(&self) -> ProposalBinding;
    fn owns_staged_state(&self, proposal: &ApplyProposal<'_>) -> bool {
        self.binding() == proposal.binding()
    }
}

mod planning;
pub use planning::{reconcile_effects, IndexedPlanOperation, ProvisionalApplyEffect};

pub mod input;

mod membership;
pub use membership::{
    DirectoryMembershipSelector, DirectoryMembershipSnapshot, DirectoryTopologyEntry,
    DirectoryTopologyEntryKind,
};

/// Existing tracked-input overlay used by legacy source-image planning.
/// The issuer owns both source scopes and all publication; these methods only
/// observe inputs or update the same private in-memory proposal.
pub trait TrackedSourceProposal {
    fn is_planned(&self, path: &Path) -> bool;
    fn exists(&self, path: &Path) -> bool;
    fn read_utf8_sig(&mut self, path: &Path) -> Result<String, String>;
    fn read_dependency_utf8_sig(&mut self, path: &Path) -> Result<String, String>;
    fn write_utf8_bom(&mut self, path: &Path, text: &str) -> Result<(), String>;
}

#[cfg(test)]
mod event_effect_tests {
    use super::*;
    use crate::events::{DomainEvent, DomainEventKind};
    #[test]
    fn planned_effects_deduplicate_by_kind_and_artifact_in_first_occurrence_order() {
        let mut effects = PlannedApplyEffects::default();
        effects.append(DomainEvent::new(
            DomainEventKind::ModuleChanged,
            "main:Module.Form",
        ));
        effects.append(DomainEvent::new(
            DomainEventKind::FormChanged,
            "main:Form.Main",
        ));
        effects.append(DomainEvent::new(
            DomainEventKind::ModuleChanged,
            "main:Module.Form",
        ));
        assert_eq!(effects.events().len(), 2);
        assert_eq!(effects.events()[0].kind, DomainEventKind::ModuleChanged);
        assert_eq!(effects.events()[1].kind, DomainEventKind::FormChanged);
    }

    #[test]
    fn plan_details_keep_key_order_replace_a_section_and_drop_an_empty_one() {
        const LATER: PlanDetailKey = PlanDetailKey::new("zSection");
        const EARLIER: PlanDetailKey = PlanDetailKey::new("aSection");
        let mut effects = PlannedApplyEffects::default();
        effects.set_details(LATER, &[1, 2]).unwrap();
        effects.set_details(EARLIER, &["first"]).unwrap();
        effects.set_details(EARLIER, &["replaced"]).unwrap();
        let sections = effects
            .details()
            .map(|(key, rows)| (key.as_str(), rows.to_vec()))
            .collect::<Vec<_>>();
        assert_eq!(
            sections,
            vec![
                ("aSection", vec![serde_json::json!("replaced")]),
                ("zSection", vec![serde_json::json!(1), serde_json::json!(2)]),
            ]
        );
        effects.set_details::<u8>(LATER, &[]).unwrap();
        assert_eq!(effects.details().count(), 1);
    }
}
