#[cfg(test)]
use super::repository::{
    original_merge_lock_projection_fixture_test_only,
    original_merge_production_lock_projection_fixture_test_only,
};
use super::repository::{
    JournaledRepositoryLock, LockPlanData, RepositoryIntegrationEntries, RepositoryRelevantAnchors,
    ValidatedOriginalMergeLockProjection,
};
use crate::domain::branched_development::canonical_json::{
    canonical_contract_digest, contract_digest_record_sealed, ContractDigestRecord,
};
use crate::domain::branched_development::contracts::artifacts::SafeResultCount;
use crate::domain::branched_development::contracts::change_receipts::MetadataPropertyAffectedTarget;
use crate::domain::branched_development::contracts::repository::{
    RepositoryAnchor, RepositoryHistoryCursor, RepositoryTargetIdentity,
    RepositoryUpdateLockTargets, SupportGateHistoryEvidence,
};
#[cfg(test)]
use crate::domain::branched_development::contracts::scalars::NormalizedUtcInstant;
use crate::domain::branched_development::contracts::scalars::{
    Name, PropertyPath, RepositoryTargetDisplay,
};
use crate::domain::branched_development::contracts::schema::one_of_schema;
use crate::domain::branched_development::contracts::status::{
    ResolutionChangeReceiptResumeHandle, SelectableResolutionChangeReceiptAuthority,
};
#[cfg(test)]
use crate::domain::branched_development::contracts::support::ready_preflight_authority_fixture_test_only;
use crate::domain::branched_development::contracts::support::{
    ReadySupportPreflightAuthority, SupportPreflightData,
};
use crate::domain::branched_development::{
    CapabilityRowId, MetadataObjectId, Sha256Digest, UnicaId,
};
use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
use serde::Serialize;
use std::borrow::Cow;
use std::collections::BTreeSet;
use std::fmt;
use std::marker::PhantomData;
use std::sync::Arc;

const MAX_RESULT_ITEMS: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MergeResultContractError(&'static str);

impl fmt::Display for MergeResultContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

impl std::error::Error for MergeResultContractError {}

fn merge_digest<T: ContractDigestRecord>(
    record: &T,
    message: &'static str,
) -> Result<Sha256Digest, MergeResultContractError> {
    canonical_contract_digest(record, None).map_err(|_| MergeResultContractError(message))
}

macro_rules! wire_literal {
    ($name:ident, $wire:literal) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
        enum $name {
            #[serde(rename = $wire)]
            Value,
        }
    };
}

wire_literal!(SupportedUpdateMode, "supportedUpdate");
wire_literal!(ResolvedReplayMode, "resolvedReplay");
wire_literal!(MainIntegrationMode, "mainIntegration");
wire_literal!(TaskTarget, "task");
wire_literal!(OriginalTarget, "original");
wire_literal!(LocalCheckpointScope, "localCheckpoint");
wire_literal!(SynchronizedTaskScope, "synchronizedTask");
wire_literal!(MainSandboxScope, "mainSandbox");
wire_literal!(MainIntegrationScope, "mainIntegration");
wire_literal!(ValidOutcome, "valid");
wire_literal!(InvalidOutcome, "invalid");
wire_literal!(EquivalentOutcome, "equivalent");
wire_literal!(AdaptedOutcome, "adapted");
wire_literal!(UnexpectedOutcome, "unexpected");
wire_literal!(UndecidedStateKind, "undecided");
wire_literal!(CurrentStateKind, "current");
wire_literal!(ReplacementPendingStateKind, "replacementPending");

/// A count literal used by physical no-conflict result leaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ZeroCount;

impl Serialize for ZeroCount {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_u64(0)
    }
}

impl JsonSchema for ZeroCount {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        "ZeroCount".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({ "type": "integer", "const": 0 })
    }
}

/// A positive I-JSON-safe count used by physical conflict-bearing leaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct PositiveCount(SafeResultCount);

impl PositiveCount {
    fn new(value: SafeResultCount) -> Result<Self, MergeResultContractError> {
        (value.get() > 0)
            .then_some(Self(value))
            .ok_or(MergeResultContractError("conflict count must be positive"))
    }

    const fn get(self) -> u64 {
        self.0.get()
    }
}

impl JsonSchema for PositiveCount {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        "PositiveCount".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "integer",
            "minimum": 1,
            "maximum": 9_007_199_254_740_991_u64,
        })
    }
}

// -------------------------------------------------------------------------
// Comparison

/// The active specification deliberately does not publish a closed vocabulary
/// for platform-specific unsupported change kinds. Values are therefore typed
/// as bounded names, while the collection itself remains canonical and closed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct UnsupportedChangeKinds(Vec<Name>);

impl UnsupportedChangeKinds {
    fn new(values: Vec<Name>) -> Result<Self, MergeResultContractError> {
        if values.len() > MAX_RESULT_ITEMS
            || values
                .windows(2)
                .any(|pair| pair[0].as_str().as_bytes() >= pair[1].as_str().as_bytes())
        {
            return Err(MergeResultContractError(
                "unsupported kinds must be bounded and strictly ordered by UTF-8 bytes",
            ));
        }
        Ok(Self(values))
    }
}

impl JsonSchema for UnsupportedChangeKinds {
    fn schema_name() -> Cow<'static, str> {
        "UnsupportedChangeKinds".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "minItems": 0,
            "maxItems": MAX_RESULT_ITEMS,
            "uniqueItems": true,
            "items": generator.subschema_for::<Name>(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ComparisonScopeKind {
    ProjectDelta,
    MainIntegration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ComparisonAnchorKind {
    OriginalCurrent,
    Repository,
    TaskCurrent,
    TaskVendor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ComparisonArtifactRole {
    BaselineDistribution,
    RefreshDistribution,
    OrdinaryResult,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ComparisonOperandIdentity {
    Anchor(ComparisonAnchorKind),
    Artifact {
        artifact_id: UnicaId,
        role: ComparisonArtifactRole,
    },
}

/// One exact comparison operand after workspace/delivery verification. Artifact
/// identity and its verified role travel with the content anchor and cannot be
/// reconstructed from a digest alone.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ComparisonOperandAuthority {
    identity: ComparisonOperandIdentity,
    anchor: Sha256Digest,
}

impl ComparisonOperandAuthority {
    const fn anchor_from_workspace_adapter(
        kind: ComparisonAnchorKind,
        anchor: Sha256Digest,
    ) -> Self {
        Self {
            identity: ComparisonOperandIdentity::Anchor(kind),
            anchor,
        }
    }

    pub(crate) const fn original_current_from_workspace_adapter(anchor: Sha256Digest) -> Self {
        Self::anchor_from_workspace_adapter(ComparisonAnchorKind::OriginalCurrent, anchor)
    }

    pub(crate) const fn repository_from_workspace_adapter(anchor: Sha256Digest) -> Self {
        Self::anchor_from_workspace_adapter(ComparisonAnchorKind::Repository, anchor)
    }

    pub(crate) const fn task_current_from_workspace_adapter(anchor: Sha256Digest) -> Self {
        Self::anchor_from_workspace_adapter(ComparisonAnchorKind::TaskCurrent, anchor)
    }

    pub(crate) const fn task_vendor_from_workspace_adapter(anchor: Sha256Digest) -> Self {
        Self::anchor_from_workspace_adapter(ComparisonAnchorKind::TaskVendor, anchor)
    }

    const fn artifact_from_delivery_adapter(
        artifact_id: UnicaId,
        role: ComparisonArtifactRole,
        anchor: Sha256Digest,
    ) -> Self {
        Self {
            identity: ComparisonOperandIdentity::Artifact { artifact_id, role },
            anchor,
        }
    }

    pub(crate) const fn baseline_distribution_from_delivery_adapter(
        artifact_id: UnicaId,
        anchor: Sha256Digest,
    ) -> Self {
        Self::artifact_from_delivery_adapter(
            artifact_id,
            ComparisonArtifactRole::BaselineDistribution,
            anchor,
        )
    }

    pub(crate) const fn refresh_distribution_from_delivery_adapter(
        artifact_id: UnicaId,
        anchor: Sha256Digest,
    ) -> Self {
        Self::artifact_from_delivery_adapter(
            artifact_id,
            ComparisonArtifactRole::RefreshDistribution,
            anchor,
        )
    }

    pub(crate) const fn ordinary_result_from_delivery_adapter(
        artifact_id: UnicaId,
        anchor: Sha256Digest,
    ) -> Self {
        Self::artifact_from_delivery_adapter(
            artifact_id,
            ComparisonArtifactRole::OrdinaryResult,
            anchor,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ComparisonSelectionAuthority {
    scope: ComparisonScopeKind,
    left_identity: ComparisonOperandIdentity,
    left_anchor: Sha256Digest,
    right_identity: ComparisonOperandIdentity,
    right_anchor: Sha256Digest,
}

impl ComparisonSelectionAuthority {
    fn from_operands(
        scope: ComparisonScopeKind,
        left: ComparisonOperandAuthority,
        right: ComparisonOperandAuthority,
    ) -> Result<Self, MergeResultContractError> {
        let role_is_allowed = |identity: &ComparisonOperandIdentity| match identity {
            ComparisonOperandIdentity::Anchor(_) => true,
            ComparisonOperandIdentity::Artifact { role, .. } => matches!(
                (scope, role),
                (
                    ComparisonScopeKind::ProjectDelta,
                    ComparisonArtifactRole::BaselineDistribution
                        | ComparisonArtifactRole::RefreshDistribution
                ) | (
                    ComparisonScopeKind::MainIntegration,
                    ComparisonArtifactRole::OrdinaryResult
                )
            ),
        };
        if !role_is_allowed(&left.identity) || !role_is_allowed(&right.identity) {
            return Err(MergeResultContractError(
                "comparison artifact role is not allowed in the selected scope",
            ));
        }
        Ok(Self {
            scope,
            left_identity: left.identity,
            left_anchor: left.anchor,
            right_identity: right.identity,
            right_anchor: right.anchor,
        })
    }

    const fn capability_scope(&self) -> ComparisonCapabilityScope<'_> {
        ComparisonCapabilityScope { selection: self }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ProjectDeltaComparisonSelectionAuthority(ComparisonSelectionAuthority);

impl ProjectDeltaComparisonSelectionAuthority {
    pub(crate) fn from_operands(
        left: ComparisonOperandAuthority,
        right: ComparisonOperandAuthority,
    ) -> Result<Self, MergeResultContractError> {
        ComparisonSelectionAuthority::from_operands(ComparisonScopeKind::ProjectDelta, left, right)
            .map(Self)
    }

    const fn capability_scope(&self) -> ComparisonCapabilityScope<'_> {
        self.0.capability_scope()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct MainIntegrationComparisonSelectionAuthority(ComparisonSelectionAuthority);

impl MainIntegrationComparisonSelectionAuthority {
    pub(crate) fn from_operands(
        left: ComparisonOperandAuthority,
        right: ComparisonOperandAuthority,
    ) -> Result<Self, MergeResultContractError> {
        ComparisonSelectionAuthority::from_operands(
            ComparisonScopeKind::MainIntegration,
            left,
            right,
        )
        .map(Self)
    }

    const fn capability_scope(&self) -> ComparisonCapabilityScope<'_> {
        self.0.capability_scope()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ComparisonData {
    comparison_id: UnicaId,
    left_anchor: Sha256Digest,
    right_anchor: Sha256Digest,
    platform_report_id: UnicaId,
    canonical_manifest_id: UnicaId,
    delta_digest: Sha256Digest,
    change_count: SafeResultCount,
    unsupported_kinds: UnsupportedChangeKinds,
}

impl ComparisonData {
    pub(crate) const fn comparison_id(&self) -> &UnicaId {
        &self.comparison_id
    }

    pub(crate) const fn delta_digest(&self) -> &Sha256Digest {
        &self.delta_digest
    }

    fn from_capability_resolver(
        selection: &ComparisonSelectionAuthority,
        resolver: &mut dyn PlatformComparisonCapabilityResolver,
    ) -> Result<Self, MergeResultContractError> {
        let scope = selection.capability_scope();
        let observation = resolver.compare(scope)?;
        if observation.selection != *selection {
            return Err(MergeResultContractError(
                "comparison capability observation disagrees with the selected operands",
            ));
        }
        let change_count = SafeResultCount::new(observation.classified_changes.len() as u64)
            .map_err(|_| MergeResultContractError("comparison change count is not I-JSON-safe"))?;
        Ok(Self {
            comparison_id: observation.comparison_id,
            left_anchor: observation.selection.left_anchor,
            right_anchor: observation.selection.right_anchor,
            platform_report_id: observation.platform_report_id,
            canonical_manifest_id: observation.canonical_manifest_id,
            delta_digest: observation.delta_digest,
            change_count,
            unsupported_kinds: UnsupportedChangeKinds::new(observation.unsupported_kinds)?,
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ProjectDeltaComparisonAuthority {
    data: ComparisonData,
}

impl ProjectDeltaComparisonAuthority {
    pub(crate) fn from_capability_resolver(
        selection: &ProjectDeltaComparisonSelectionAuthority,
        resolver: &mut dyn PlatformComparisonCapabilityResolver,
    ) -> Result<Self, MergeResultContractError> {
        ComparisonData::from_capability_resolver(&selection.0, resolver).map(|data| Self { data })
    }

    pub(crate) const fn data(&self) -> &ComparisonData {
        &self.data
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct MainIntegrationComparisonAuthority {
    data: ComparisonData,
}

impl MainIntegrationComparisonAuthority {
    pub(crate) fn from_capability_resolver(
        selection: &MainIntegrationComparisonSelectionAuthority,
        resolver: &mut dyn PlatformComparisonCapabilityResolver,
    ) -> Result<Self, MergeResultContractError> {
        ComparisonData::from_capability_resolver(&selection.0, resolver).map(|data| Self { data })
    }

    pub(crate) const fn data(&self) -> &ComparisonData {
        &self.data
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ComparisonCapabilityScope<'a> {
    selection: &'a ComparisonSelectionAuthority,
}

impl ComparisonCapabilityScope<'_> {
    pub(crate) const fn is_project_delta(&self) -> bool {
        matches!(self.selection.scope, ComparisonScopeKind::ProjectDelta)
    }

    pub(crate) const fn left_anchor(&self) -> &Sha256Digest {
        &self.selection.left_anchor
    }

    pub(crate) const fn right_anchor(&self) -> &Sha256Digest {
        &self.selection.right_anchor
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ClassifiedComparisonChangeAuthority {
    change_id: UnicaId,
}

/// The classifier publishes its ordered change set atomically. There is no
/// caller-supplied ordinal that can be rewritten after classification.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ClassifiedComparisonChangeBatchAuthority {
    changes: Vec<ClassifiedComparisonChangeAuthority>,
}

impl ClassifiedComparisonChangeBatchAuthority {
    pub(crate) fn from_classifier_adapter(
        change_ids: Vec<UnicaId>,
    ) -> Result<Self, MergeResultContractError> {
        if change_ids.len() > MAX_RESULT_ITEMS {
            return Err(MergeResultContractError(
                "comparison classifier batch exceeds the result bound",
            ));
        }
        let mut seen = BTreeSet::new();
        if change_ids
            .iter()
            .any(|change_id| !seen.insert(change_id.as_str()))
        {
            return Err(MergeResultContractError(
                "comparison classifier batch contains duplicate changes",
            ));
        }
        Ok(Self {
            changes: change_ids
                .into_iter()
                .map(|change_id| ClassifiedComparisonChangeAuthority { change_id })
                .collect(),
        })
    }

    const fn len(&self) -> usize {
        self.changes.len()
    }
}

/// One atomic platform comparison result. `changeCount` is intentionally not
/// accepted: the result producer derives it from the ordered classifier rows.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PlatformComparisonCapabilitySnapshot {
    selection: ComparisonSelectionAuthority,
    comparison_id: UnicaId,
    platform_report_id: UnicaId,
    canonical_manifest_id: UnicaId,
    delta_digest: Sha256Digest,
    classified_changes: ClassifiedComparisonChangeBatchAuthority,
    unsupported_kinds: Vec<Name>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PlatformComparisonCapabilitySnapshotInput {
    comparison_id: UnicaId,
    platform_report_id: UnicaId,
    canonical_manifest_id: UnicaId,
    delta_digest: Sha256Digest,
    classified_changes: ClassifiedComparisonChangeBatchAuthority,
    unsupported_kinds: Vec<Name>,
}

impl PlatformComparisonCapabilitySnapshotInput {
    pub(crate) const fn from_comparison_adapter(
        comparison_id: UnicaId,
        platform_report_id: UnicaId,
        canonical_manifest_id: UnicaId,
        delta_digest: Sha256Digest,
        classified_changes: ClassifiedComparisonChangeBatchAuthority,
        unsupported_kinds: Vec<Name>,
    ) -> Self {
        Self {
            comparison_id,
            platform_report_id,
            canonical_manifest_id,
            delta_digest,
            classified_changes,
            unsupported_kinds,
        }
    }
}

impl PlatformComparisonCapabilitySnapshot {
    pub(crate) fn from_comparison_adapter(
        scope: ComparisonCapabilityScope<'_>,
        input: PlatformComparisonCapabilitySnapshotInput,
    ) -> Self {
        Self {
            selection: scope.selection.clone(),
            comparison_id: input.comparison_id,
            platform_report_id: input.platform_report_id,
            canonical_manifest_id: input.canonical_manifest_id,
            delta_digest: input.delta_digest,
            classified_changes: input.classified_changes,
            unsupported_kinds: input.unsupported_kinds,
        }
    }
}

/// Capability boundary for one contained platform comparison.
pub(crate) trait PlatformComparisonCapabilityResolver {
    fn compare(
        &mut self,
        scope: ComparisonCapabilityScope<'_>,
    ) -> Result<PlatformComparisonCapabilitySnapshot, MergeResultContractError>;
}

// -------------------------------------------------------------------------
// Conflict inventory and canonical evolving state

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) enum MergeConflictKind {
    TwiceChanged,
    DeleteModify,
    AddAddNameCollision,
    UuidMismatch,
    UnresolvedReference,
    SupportRuleBlocked,
    MergeSettingsRejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) enum MergeResolution {
    TakeOurs,
    TakeTheirs,
    Combine,
    Manual,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct AllowedMergeResolutions(Vec<MergeResolution>);

impl AllowedMergeResolutions {
    fn new(values: Vec<MergeResolution>) -> Result<Self, MergeResultContractError> {
        if values.is_empty() || values.len() > 4 || values.windows(2).any(|pair| pair[0] >= pair[1])
        {
            return Err(MergeResultContractError(
                "allowed resolutions must be a non-empty declaration-ordered unique subset",
            ));
        }
        Ok(Self(values))
    }

    fn contains(&self, resolution: MergeResolution) -> bool {
        self.0.contains(&resolution)
    }
}

impl JsonSchema for AllowedMergeResolutions {
    fn schema_name() -> Cow<'static, str> {
        "AllowedMergeResolutions".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "minItems": 1,
            "maxItems": 4,
            "uniqueItems": true,
            "items": generator.subschema_for::<MergeResolution>(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct UndecidedConflictState {
    state_kind: UndecidedStateKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CurrentConflictState {
    state_kind: CurrentStateKind,
    decision_id: UnicaId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ReplacementPendingConflictState {
    state_kind: ReplacementPendingStateKind,
    decision_id: UnicaId,
    caused_by_change_receipt_id: UnicaId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub(crate) enum ConflictDecisionState {
    Undecided(UndecidedConflictState),
    Current(CurrentConflictState),
    ReplacementPending(ReplacementPendingConflictState),
}

impl ConflictDecisionState {
    #[cfg(test)]
    fn undecided_test_only() -> Self {
        Self::Undecided(UndecidedConflictState {
            state_kind: UndecidedStateKind::Value,
        })
    }

    #[cfg(test)]
    fn current_test_only(decision_id: UnicaId) -> Self {
        Self::Current(CurrentConflictState {
            state_kind: CurrentStateKind::Value,
            decision_id,
        })
    }

    #[cfg(test)]
    fn replacement_pending_test_only(
        decision_id: UnicaId,
        caused_by_change_receipt_id: UnicaId,
    ) -> Self {
        Self::ReplacementPending(ReplacementPendingConflictState {
            state_kind: ReplacementPendingStateKind::Value,
            decision_id,
            caused_by_change_receipt_id,
        })
    }

    fn predecessor_id(&self) -> Option<&UnicaId> {
        match self {
            Self::Undecided(_) => None,
            Self::Current(value) => Some(&value.decision_id),
            Self::ReplacementPending(value) => Some(&value.decision_id),
        }
    }
}

impl JsonSchema for ConflictDecisionState {
    fn schema_name() -> Cow<'static, str> {
        "ConflictDecisionState".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<UndecidedConflictState>(),
            generator.subschema_for::<CurrentConflictState>(),
            generator.subschema_for::<ReplacementPendingConflictState>(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct MergeConflict {
    conflict_id: UnicaId,
    object_id: MetadataObjectId,
    object_display: RepositoryTargetDisplay,
    property_path: PropertyPath,
    kind: MergeConflictKind,
    base_sha256: Sha256Digest,
    ours_sha256: Sha256Digest,
    theirs_sha256: Sha256Digest,
    allowed_resolutions: AllowedMergeResolutions,
    decision_state: ConflictDecisionState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct MergeConflictIdentityInput {
    conflict_id: UnicaId,
    object_id: MetadataObjectId,
    object_display: RepositoryTargetDisplay,
    property_path: PropertyPath,
    kind: MergeConflictKind,
}

impl MergeConflictIdentityInput {
    pub(crate) const fn from_classifier_adapter(
        conflict_id: UnicaId,
        object_id: MetadataObjectId,
        object_display: RepositoryTargetDisplay,
        property_path: PropertyPath,
        kind: MergeConflictKind,
    ) -> Self {
        Self {
            conflict_id,
            object_id,
            object_display,
            property_path,
            kind,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct MergeConflictContentInput {
    base_sha256: Sha256Digest,
    ours_sha256: Sha256Digest,
    theirs_sha256: Sha256Digest,
    allowed_resolutions: Vec<MergeResolution>,
}

impl MergeConflictContentInput {
    pub(crate) const fn from_classifier_adapter(
        base_sha256: Sha256Digest,
        ours_sha256: Sha256Digest,
        theirs_sha256: Sha256Digest,
        allowed_resolutions: Vec<MergeResolution>,
    ) -> Self {
        Self {
            base_sha256,
            ours_sha256,
            theirs_sha256,
            allowed_resolutions,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ClassifiedMergeConflictInput {
    identity: MergeConflictIdentityInput,
    content: MergeConflictContentInput,
}

impl ClassifiedMergeConflictInput {
    pub(crate) const fn from_classifier_adapter(
        identity: MergeConflictIdentityInput,
        content: MergeConflictContentInput,
    ) -> Self {
        Self { identity, content }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MergeConflictClassifierOrderDigestRecord {
    conflicts: Vec<ClassifiedMergeConflictInput>,
}

impl contract_digest_record_sealed::Sealed for MergeConflictClassifierOrderDigestRecord {}
impl ContractDigestRecord for MergeConflictClassifierOrderDigestRecord {}

/// Opaque order commitment returned by the classifier together with its batch.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct MergeConflictClassifierOrderAuthority {
    digest: Sha256Digest,
}

impl MergeConflictClassifierOrderAuthority {
    pub(crate) fn from_classifier_adapter(
        conflicts: &[ClassifiedMergeConflictInput],
    ) -> Result<Self, MergeResultContractError> {
        Ok(Self {
            digest: merge_digest(
                &MergeConflictClassifierOrderDigestRecord {
                    conflicts: conflicts.to_vec(),
                },
                "classifier order digest failed",
            )?,
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
struct ClassifiedMergeConflictAuthority {
    conflict_id: UnicaId,
    object_id: MetadataObjectId,
    object_display: RepositoryTargetDisplay,
    property_path: PropertyPath,
    kind: MergeConflictKind,
    base_sha256: Sha256Digest,
    ours_sha256: Sha256Digest,
    theirs_sha256: Sha256Digest,
    allowed_resolutions: AllowedMergeResolutions,
}

impl ClassifiedMergeConflictAuthority {
    fn from_input(value: ClassifiedMergeConflictInput) -> Result<Self, MergeResultContractError> {
        Ok(Self {
            conflict_id: value.identity.conflict_id,
            object_id: value.identity.object_id,
            object_display: value.identity.object_display,
            property_path: value.identity.property_path,
            kind: value.identity.kind,
            base_sha256: value.content.base_sha256,
            ours_sha256: value.content.ours_sha256,
            theirs_sha256: value.content.theirs_sha256,
            allowed_resolutions: AllowedMergeResolutions::new(value.content.allowed_resolutions)?,
        })
    }

    fn materialize(self) -> MergeConflict {
        MergeConflict {
            conflict_id: self.conflict_id,
            object_id: self.object_id,
            object_display: self.object_display,
            property_path: self.property_path,
            kind: self.kind,
            base_sha256: self.base_sha256,
            ours_sha256: self.ours_sha256,
            theirs_sha256: self.theirs_sha256,
            allowed_resolutions: self.allowed_resolutions,
            decision_state: ConflictDecisionState::Undecided(UndecidedConflictState {
                state_kind: UndecidedStateKind::Value,
            }),
        }
    }
}

/// A sealed classifier snapshot: callers can move it, but cannot extract rows,
/// renumber them, or reorder them before session materialization.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ClassifiedMergeConflictBatchAuthority {
    conflicts: Vec<ClassifiedMergeConflictAuthority>,
}

impl ClassifiedMergeConflictBatchAuthority {
    pub(crate) fn from_classifier_snapshot_adapter(
        order: MergeConflictClassifierOrderAuthority,
        conflicts: Vec<ClassifiedMergeConflictInput>,
    ) -> Result<Self, MergeResultContractError> {
        let observed_digest = merge_digest(
            &MergeConflictClassifierOrderDigestRecord {
                conflicts: conflicts.clone(),
            },
            "classifier order digest failed",
        )?;
        if order.digest != observed_digest {
            return Err(MergeResultContractError(
                "merge conflicts were reordered after classification",
            ));
        }
        if conflicts.is_empty() || conflicts.len() > MAX_RESULT_ITEMS {
            return Err(MergeResultContractError(
                "classifier conflict batch must be non-empty and bounded",
            ));
        }
        Ok(Self {
            conflicts: conflicts
                .into_iter()
                .map(ClassifiedMergeConflictAuthority::from_input)
                .collect::<Result<Vec<_>, _>>()?,
        })
    }

    fn into_materialized(self) -> Vec<MergeConflict> {
        self.conflicts
            .into_iter()
            .map(ClassifiedMergeConflictAuthority::materialize)
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct CanonicalMergeConflicts(Vec<MergeConflict>);

impl CanonicalMergeConflicts {
    fn new(
        values: Vec<MergeConflict>,
        allow_empty: bool,
    ) -> Result<Self, MergeResultContractError> {
        if (!allow_empty && values.is_empty()) || values.len() > MAX_RESULT_ITEMS {
            return Err(MergeResultContractError(
                "conflicts must be non-empty when required and bounded",
            ));
        }
        // The specification intentionally delegates canonical conflict order
        // to the platform classifier; UUID lexical order is not that authority.
        // The enclosing base-session digest binds the exact classifier order.
        let mut seen = BTreeSet::new();
        if values
            .iter()
            .any(|conflict| !seen.insert(conflict.conflict_id.as_str()))
        {
            return Err(MergeResultContractError(
                "conflicts must be unique in classifier order",
            ));
        }
        Ok(Self(values))
    }

    fn as_slice(&self) -> &[MergeConflict] {
        &self.0
    }
}

impl JsonSchema for CanonicalMergeConflicts {
    fn schema_name() -> Cow<'static, str> {
        "CanonicalMergeConflicts".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "minItems": 1,
            "maxItems": MAX_RESULT_ITEMS,
            "uniqueItems": true,
            "items": generator.subschema_for::<MergeConflict>(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ConflictImmutableDigestRecord {
    conflict_id: UnicaId,
    object_id: MetadataObjectId,
    object_display: RepositoryTargetDisplay,
    property_path: PropertyPath,
    kind: MergeConflictKind,
    base_sha256: Sha256Digest,
    ours_sha256: Sha256Digest,
    theirs_sha256: Sha256Digest,
    allowed_resolutions: AllowedMergeResolutions,
}

impl From<&MergeConflict> for ConflictImmutableDigestRecord {
    fn from(value: &MergeConflict) -> Self {
        Self {
            conflict_id: value.conflict_id.clone(),
            object_id: value.object_id.clone(),
            object_display: value.object_display.clone(),
            property_path: value.property_path.clone(),
            kind: value.kind,
            base_sha256: value.base_sha256.clone(),
            ours_sha256: value.ours_sha256.clone(),
            theirs_sha256: value.theirs_sha256.clone(),
            allowed_resolutions: value.allowed_resolutions.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ConflictDecisionStateDigestRecord {
    conflict_id: UnicaId,
    decision_state: ConflictDecisionState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DecisionSetDigestRecord {
    conflicts: Vec<ConflictDecisionStateDigestRecord>,
}

impl contract_digest_record_sealed::Sealed for DecisionSetDigestRecord {}
impl ContractDigestRecord for DecisionSetDigestRecord {}

fn decision_set_digest(
    conflicts: &[MergeConflict],
) -> Result<Sha256Digest, MergeResultContractError> {
    merge_digest(
        &DecisionSetDigestRecord {
            conflicts: conflicts
                .iter()
                .map(|conflict| ConflictDecisionStateDigestRecord {
                    conflict_id: conflict.conflict_id.clone(),
                    decision_state: conflict.decision_state.clone(),
                })
                .collect(),
        },
        "decision-set digest failed",
    )
}

// -------------------------------------------------------------------------
// Session preparation

/// Named closed semantic inputs frozen by a preparation session. Every field
/// is copied from its typed producer, never supplied as a generic hash slot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ImmutableMergeInputHashes {
    checkpoint_verification_digest: Sha256Digest,
    comparison_delta_digest: Sha256Digest,
    source_artifact_sha256: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SupportedUpdateResolvedSessionData {
    session_id: UnicaId,
    mode: SupportedUpdateMode,
    checkpoint_id: UnicaId,
    incoming_distribution_id: UnicaId,
    immutable_input_hashes: ImmutableMergeInputHashes,
    anchor_digest: Sha256Digest,
    settings_digest: Sha256Digest,
    comparison_id: UnicaId,
    result_digest: Sha256Digest,
    conflict_count: ZeroCount,
    base_session_digest: Sha256Digest,
    decision_set_digest: Sha256Digest,
    resolved_session_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SupportedUpdateConflictedSessionData {
    session_id: UnicaId,
    mode: SupportedUpdateMode,
    checkpoint_id: UnicaId,
    incoming_distribution_id: UnicaId,
    immutable_input_hashes: ImmutableMergeInputHashes,
    anchor_digest: Sha256Digest,
    settings_digest: Sha256Digest,
    comparison_id: UnicaId,
    conflict_count: PositiveCount,
    merge_resolution_workspace_id: UnicaId,
    base_session_digest: Sha256Digest,
    decision_set_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SupportedUpdateConflictedWithoutWorkspaceSessionData {
    session_id: UnicaId,
    mode: SupportedUpdateMode,
    checkpoint_id: UnicaId,
    incoming_distribution_id: UnicaId,
    immutable_input_hashes: ImmutableMergeInputHashes,
    anchor_digest: Sha256Digest,
    settings_digest: Sha256Digest,
    comparison_id: UnicaId,
    conflict_count: PositiveCount,
    base_session_digest: Sha256Digest,
    decision_set_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ResolvedReplayResolvedSessionData {
    session_id: UnicaId,
    mode: ResolvedReplayMode,
    checkpoint_id: UnicaId,
    incoming_distribution_id: UnicaId,
    immutable_input_hashes: ImmutableMergeInputHashes,
    anchor_digest: Sha256Digest,
    settings_digest: Sha256Digest,
    comparison_id: UnicaId,
    result_digest: Sha256Digest,
    conflict_count: ZeroCount,
    base_session_digest: Sha256Digest,
    decision_set_digest: Sha256Digest,
    resolved_session_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ResolvedReplayConflictedSessionData {
    session_id: UnicaId,
    mode: ResolvedReplayMode,
    checkpoint_id: UnicaId,
    incoming_distribution_id: UnicaId,
    immutable_input_hashes: ImmutableMergeInputHashes,
    anchor_digest: Sha256Digest,
    settings_digest: Sha256Digest,
    comparison_id: UnicaId,
    conflict_count: PositiveCount,
    merge_resolution_workspace_id: UnicaId,
    base_session_digest: Sha256Digest,
    decision_set_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ResolvedReplayConflictedWithoutWorkspaceSessionData {
    session_id: UnicaId,
    mode: ResolvedReplayMode,
    checkpoint_id: UnicaId,
    incoming_distribution_id: UnicaId,
    immutable_input_hashes: ImmutableMergeInputHashes,
    anchor_digest: Sha256Digest,
    settings_digest: Sha256Digest,
    comparison_id: UnicaId,
    conflict_count: PositiveCount,
    base_session_digest: Sha256Digest,
    decision_set_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct MainIntegrationSessionData {
    session_id: UnicaId,
    mode: MainIntegrationMode,
    checkpoint_id: UnicaId,
    immutable_input_hashes: ImmutableMergeInputHashes,
    anchor_digest: Sha256Digest,
    settings_digest: Sha256Digest,
    ordinary_result_artifact_id: UnicaId,
    comparison_id: UnicaId,
    result_digest: Sha256Digest,
    conflict_count: ZeroCount,
    base_session_digest: Sha256Digest,
    decision_set_digest: Sha256Digest,
    resolved_session_digest: Sha256Digest,
    support_gate_id: UnicaId,
    support_gate_digest: Sha256Digest,
    support_gate_history_evidence_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub(crate) enum MergeSessionData {
    SupportedUpdateResolved(SupportedUpdateResolvedSessionData),
    SupportedUpdateConflicted(SupportedUpdateConflictedSessionData),
    SupportedUpdateConflictedWithoutWorkspace(SupportedUpdateConflictedWithoutWorkspaceSessionData),
    ResolvedReplayResolved(ResolvedReplayResolvedSessionData),
    ResolvedReplayConflicted(ResolvedReplayConflictedSessionData),
    ResolvedReplayConflictedWithoutWorkspace(ResolvedReplayConflictedWithoutWorkspaceSessionData),
    MainIntegration(MainIntegrationSessionData),
}

impl JsonSchema for MergeSessionData {
    fn schema_name() -> Cow<'static, str> {
        "MergeSessionData".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<SupportedUpdateResolvedSessionData>(),
            generator.subschema_for::<SupportedUpdateConflictedSessionData>(),
            generator.subschema_for::<SupportedUpdateConflictedWithoutWorkspaceSessionData>(),
            generator.subschema_for::<ResolvedReplayResolvedSessionData>(),
            generator.subschema_for::<ResolvedReplayConflictedSessionData>(),
            generator.subschema_for::<ResolvedReplayConflictedWithoutWorkspaceSessionData>(),
            generator.subschema_for::<MainIntegrationSessionData>(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct MergeSessionBaseDigestRecord {
    session_id: UnicaId,
    mode: MergeSessionModeDigest,
    checkpoint_id: UnicaId,
    #[serde(skip_serializing_if = "Option::is_none")]
    incoming_distribution_id: Option<UnicaId>,
    immutable_input_hashes: ImmutableMergeInputHashes,
    anchor_digest: Sha256Digest,
    settings_digest: Sha256Digest,
    #[serde(skip_serializing_if = "Option::is_none")]
    ordinary_result_artifact_id: Option<UnicaId>,
    comparison_id: UnicaId,
    #[serde(skip_serializing_if = "Option::is_none")]
    result_digest: Option<Sha256Digest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    merge_resolution_workspace_id: Option<UnicaId>,
    conflicts: Vec<ConflictImmutableDigestRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    support_gate_id: Option<UnicaId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    support_gate_digest: Option<Sha256Digest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    support_gate_history_evidence_digest: Option<Sha256Digest>,
}

impl contract_digest_record_sealed::Sealed for MergeSessionBaseDigestRecord {}
impl ContractDigestRecord for MergeSessionBaseDigestRecord {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
enum MergeSessionModeDigest {
    SupportedUpdate,
    ResolvedReplay,
    MainIntegration,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ResolvedSessionDigestRecord {
    base_session_digest: Sha256Digest,
    decision_set_digest: Sha256Digest,
    result_digest: Sha256Digest,
    applied_decision_ids: Vec<UnicaId>,
}

impl contract_digest_record_sealed::Sealed for ResolvedSessionDigestRecord {}
impl ContractDigestRecord for ResolvedSessionDigestRecord {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VerifiedMergeArtifactKind {
    RefreshDistribution,
    OrdinaryResult,
}

/// Delivery-owned source projection used by merge preparation. The distinct
/// constructors prevent a refresh distribution and an ordinary result from
/// being relabelled after delivery validation.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct VerifiedMergeArtifactAuthority {
    artifact_id: UnicaId,
    sha256: Sha256Digest,
    kind: VerifiedMergeArtifactKind,
}

impl VerifiedMergeArtifactAuthority {
    pub(crate) const fn refresh_distribution_from_delivery_adapter(
        artifact_id: UnicaId,
        sha256: Sha256Digest,
    ) -> Self {
        Self {
            artifact_id,
            sha256,
            kind: VerifiedMergeArtifactKind::RefreshDistribution,
        }
    }

    pub(crate) const fn ordinary_result_from_delivery_adapter(
        artifact_id: UnicaId,
        sha256: Sha256Digest,
    ) -> Self {
        Self {
            artifact_id,
            sha256,
            kind: VerifiedMergeArtifactKind::OrdinaryResult,
        }
    }
}

/// Exact checkpoint/distribution/comparison lineage selected before one
/// supported-update sandbox run.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct SupportedUpdatePreparationAuthority {
    checkpoint_id: UnicaId,
    checkpoint_verification_digest: Sha256Digest,
    incoming_distribution_id: UnicaId,
    source_artifact_sha256: Sha256Digest,
    comparison_id: UnicaId,
    comparison_delta_digest: Sha256Digest,
}

impl SupportedUpdatePreparationAuthority {
    pub(crate) fn from_authorities(
        checkpoint: &ValidatedLocalCheckpointVerificationAuthority,
        refresh: VerifiedMergeArtifactAuthority,
        comparison: &ProjectDeltaComparisonAuthority,
    ) -> Result<Self, MergeResultContractError> {
        if refresh.kind != VerifiedMergeArtifactKind::RefreshDistribution {
            return Err(MergeResultContractError(
                "supported update requires a verified refresh distribution",
            ));
        }
        let comparison = comparison.data();
        Ok(Self {
            checkpoint_id: checkpoint.checkpoint_id().clone(),
            checkpoint_verification_digest: checkpoint.verification_digest().clone(),
            incoming_distribution_id: refresh.artifact_id,
            source_artifact_sha256: refresh.sha256,
            comparison_id: comparison.comparison_id.clone(),
            comparison_delta_digest: comparison.delta_digest.clone(),
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
enum SupportedUpdateSessionOutcomeAuthority {
    Resolved {
        result_digest: Sha256Digest,
    },
    Conflicted {
        merge_resolution_workspace_id: Option<UnicaId>,
        conflicts: ClassifiedMergeConflictBatchAuthority,
    },
}

/// One atomic supported-update sandbox observation, already bound to the
/// selected preparation inputs. Counts and session digests are absent and are
/// derived only after lineage and classifier-order validation.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct SupportedUpdateSessionObservationAuthority {
    checkpoint_id: UnicaId,
    checkpoint_verification_digest: Sha256Digest,
    incoming_distribution_id: UnicaId,
    source_artifact_sha256: Sha256Digest,
    comparison_id: UnicaId,
    comparison_delta_digest: Sha256Digest,
    session_id: UnicaId,
    anchor_digest: Sha256Digest,
    settings_digest: Sha256Digest,
    outcome: SupportedUpdateSessionOutcomeAuthority,
}

impl SupportedUpdateSessionObservationAuthority {
    pub(crate) fn resolved_from_sandbox_adapter(
        inputs: &SupportedUpdatePreparationAuthority,
        session_id: UnicaId,
        anchor_digest: Sha256Digest,
        settings_digest: Sha256Digest,
        result_digest: Sha256Digest,
    ) -> Self {
        Self::from_inputs(
            inputs,
            session_id,
            anchor_digest,
            settings_digest,
            SupportedUpdateSessionOutcomeAuthority::Resolved { result_digest },
        )
    }

    pub(crate) fn conflicted_from_sandbox_adapter(
        inputs: &SupportedUpdatePreparationAuthority,
        session_id: UnicaId,
        anchor_digest: Sha256Digest,
        settings_digest: Sha256Digest,
        merge_resolution_workspace_id: Option<UnicaId>,
        conflicts: ClassifiedMergeConflictBatchAuthority,
    ) -> Self {
        Self::from_inputs(
            inputs,
            session_id,
            anchor_digest,
            settings_digest,
            SupportedUpdateSessionOutcomeAuthority::Conflicted {
                merge_resolution_workspace_id,
                conflicts,
            },
        )
    }

    fn from_inputs(
        inputs: &SupportedUpdatePreparationAuthority,
        session_id: UnicaId,
        anchor_digest: Sha256Digest,
        settings_digest: Sha256Digest,
        outcome: SupportedUpdateSessionOutcomeAuthority,
    ) -> Self {
        Self {
            checkpoint_id: inputs.checkpoint_id.clone(),
            checkpoint_verification_digest: inputs.checkpoint_verification_digest.clone(),
            incoming_distribution_id: inputs.incoming_distribution_id.clone(),
            source_artifact_sha256: inputs.source_artifact_sha256.clone(),
            comparison_id: inputs.comparison_id.clone(),
            comparison_delta_digest: inputs.comparison_delta_digest.clone(),
            session_id,
            anchor_digest,
            settings_digest,
            outcome,
        }
    }

    fn matches(&self, inputs: &SupportedUpdatePreparationAuthority) -> bool {
        self.checkpoint_id == inputs.checkpoint_id
            && self.checkpoint_verification_digest == inputs.checkpoint_verification_digest
            && self.incoming_distribution_id == inputs.incoming_distribution_id
            && self.source_artifact_sha256 == inputs.source_artifact_sha256
            && self.comparison_id == inputs.comparison_id
            && self.comparison_delta_digest == inputs.comparison_delta_digest
    }
}

/// Exact successful synchronized checkpoint plus ready-preflight lineage used
/// to publish a main-integration session.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct MainIntegrationPreparationInputAuthority {
    checkpoint_id: UnicaId,
    checkpoint_verification_digest: Sha256Digest,
    ordinary_result_artifact_id: UnicaId,
    source_artifact_sha256: Sha256Digest,
    comparison_id: UnicaId,
    comparison_delta_digest: Sha256Digest,
    preflight: ReadySupportPreflightAuthority,
}

impl MainIntegrationPreparationInputAuthority {
    pub(crate) fn from_authorities(
        checkpoint: &ValidatedSynchronizedCheckpointVerificationAuthority,
        ordinary_result: VerifiedMergeArtifactAuthority,
        comparison: &MainIntegrationComparisonAuthority,
        preflight: ReadySupportPreflightAuthority,
    ) -> Result<Self, MergeResultContractError> {
        let comparison = comparison.data();
        if ordinary_result.kind != VerifiedMergeArtifactKind::OrdinaryResult
            || preflight.ordinary_result_artifact_id() != &ordinary_result.artifact_id
            || preflight.comparison_id() != comparison.comparison_id()
        {
            return Err(MergeResultContractError(
                "main-integration inputs disagree with the ready preflight",
            ));
        }
        Ok(Self {
            checkpoint_id: checkpoint.checkpoint_id().clone(),
            checkpoint_verification_digest: checkpoint.verification_digest().clone(),
            ordinary_result_artifact_id: ordinary_result.artifact_id,
            source_artifact_sha256: ordinary_result.sha256,
            comparison_id: comparison.comparison_id.clone(),
            comparison_delta_digest: comparison.delta_digest.clone(),
            preflight,
        })
    }
}

/// One bound no-conflict main-sandbox observation. Result/settings/support
/// values are not accepted because they come from the consumed ready gate.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct MainIntegrationSessionObservationAuthority {
    checkpoint_id: UnicaId,
    checkpoint_verification_digest: Sha256Digest,
    ordinary_result_artifact_id: UnicaId,
    source_artifact_sha256: Sha256Digest,
    comparison_id: UnicaId,
    comparison_delta_digest: Sha256Digest,
    support_gate_id: UnicaId,
    support_gate_digest: Sha256Digest,
    support_gate_history_evidence_digest: Sha256Digest,
    session_id: UnicaId,
    anchor_digest: Sha256Digest,
}

impl MainIntegrationSessionObservationAuthority {
    pub(crate) fn from_sandbox_adapter(
        inputs: &MainIntegrationPreparationInputAuthority,
        session_id: UnicaId,
        anchor_digest: Sha256Digest,
    ) -> Self {
        Self {
            checkpoint_id: inputs.checkpoint_id.clone(),
            checkpoint_verification_digest: inputs.checkpoint_verification_digest.clone(),
            ordinary_result_artifact_id: inputs.ordinary_result_artifact_id.clone(),
            source_artifact_sha256: inputs.source_artifact_sha256.clone(),
            comparison_id: inputs.comparison_id.clone(),
            comparison_delta_digest: inputs.comparison_delta_digest.clone(),
            support_gate_id: inputs.preflight.support_gate_id().clone(),
            support_gate_digest: inputs.preflight.support_gate_digest().clone(),
            support_gate_history_evidence_digest: inputs
                .preflight
                .history_evidence_digest()
                .clone(),
            session_id,
            anchor_digest,
        }
    }

    fn matches(&self, inputs: &MainIntegrationPreparationInputAuthority) -> bool {
        self.checkpoint_id == inputs.checkpoint_id
            && self.checkpoint_verification_digest == inputs.checkpoint_verification_digest
            && self.ordinary_result_artifact_id == inputs.ordinary_result_artifact_id
            && self.source_artifact_sha256 == inputs.source_artifact_sha256
            && self.comparison_id == inputs.comparison_id
            && self.comparison_delta_digest == inputs.comparison_delta_digest
            && self.support_gate_id == *inputs.preflight.support_gate_id()
            && self.support_gate_digest == *inputs.preflight.support_gate_digest()
            && self.support_gate_history_evidence_digest
                == *inputs.preflight.history_evidence_digest()
    }
}

impl MergeSessionData {
    pub(crate) fn supported_update_from_authorities(
        inputs: &SupportedUpdatePreparationAuthority,
        observation: SupportedUpdateSessionObservationAuthority,
    ) -> Result<Self, MergeResultContractError> {
        if !observation.matches(inputs) {
            return Err(MergeResultContractError(
                "supported-update observation belongs to different preparation inputs",
            ));
        }
        let immutable_input_hashes = ImmutableMergeInputHashes {
            checkpoint_verification_digest: inputs.checkpoint_verification_digest.clone(),
            comparison_delta_digest: inputs.comparison_delta_digest.clone(),
            source_artifact_sha256: inputs.source_artifact_sha256.clone(),
        };
        match observation.outcome {
            SupportedUpdateSessionOutcomeAuthority::Resolved { result_digest } => {
                let base_record = MergeSessionBaseDigestRecord {
                    session_id: observation.session_id,
                    mode: MergeSessionModeDigest::SupportedUpdate,
                    checkpoint_id: inputs.checkpoint_id.clone(),
                    incoming_distribution_id: Some(inputs.incoming_distribution_id.clone()),
                    immutable_input_hashes,
                    anchor_digest: observation.anchor_digest,
                    settings_digest: observation.settings_digest,
                    ordinary_result_artifact_id: None,
                    comparison_id: inputs.comparison_id.clone(),
                    result_digest: Some(result_digest.clone()),
                    merge_resolution_workspace_id: None,
                    conflicts: vec![],
                    support_gate_id: None,
                    support_gate_digest: None,
                    support_gate_history_evidence_digest: None,
                };
                let base_session_digest = merge_digest(&base_record, "base-session digest failed")?;
                let decision_set_digest = decision_set_digest(&[])?;
                let resolved_session_digest = merge_digest(
                    &ResolvedSessionDigestRecord {
                        base_session_digest: base_session_digest.clone(),
                        decision_set_digest: decision_set_digest.clone(),
                        result_digest: result_digest.clone(),
                        applied_decision_ids: vec![],
                    },
                    "resolved-session digest failed",
                )?;
                Ok(Self::SupportedUpdateResolved(
                    SupportedUpdateResolvedSessionData {
                        session_id: base_record.session_id,
                        mode: SupportedUpdateMode::Value,
                        checkpoint_id: base_record.checkpoint_id,
                        incoming_distribution_id: base_record
                            .incoming_distribution_id
                            .expect("set above"),
                        immutable_input_hashes: base_record.immutable_input_hashes,
                        anchor_digest: base_record.anchor_digest,
                        settings_digest: base_record.settings_digest,
                        comparison_id: base_record.comparison_id,
                        result_digest,
                        conflict_count: ZeroCount,
                        base_session_digest,
                        decision_set_digest,
                        resolved_session_digest,
                    },
                ))
            }
            SupportedUpdateSessionOutcomeAuthority::Conflicted {
                merge_resolution_workspace_id,
                conflicts,
            } => {
                let conflicts = conflicts.into_materialized();
                let conflicts = CanonicalMergeConflicts::new(conflicts, false)?;
                let workspace_required = conflicts.as_slice().iter().any(|conflict| {
                    conflict
                        .allowed_resolutions
                        .contains(MergeResolution::Combine)
                        || conflict
                            .allowed_resolutions
                            .contains(MergeResolution::Manual)
                });
                if workspace_required != merge_resolution_workspace_id.is_some() {
                    return Err(MergeResultContractError(
                        "classifier resolution capabilities disagree with workspace presence",
                    ));
                }
                let immutable_conflicts = conflicts
                    .as_slice()
                    .iter()
                    .map(ConflictImmutableDigestRecord::from)
                    .collect();
                let base_record = MergeSessionBaseDigestRecord {
                    session_id: observation.session_id,
                    mode: MergeSessionModeDigest::SupportedUpdate,
                    checkpoint_id: inputs.checkpoint_id.clone(),
                    incoming_distribution_id: Some(inputs.incoming_distribution_id.clone()),
                    immutable_input_hashes,
                    anchor_digest: observation.anchor_digest,
                    settings_digest: observation.settings_digest,
                    ordinary_result_artifact_id: None,
                    comparison_id: inputs.comparison_id.clone(),
                    result_digest: None,
                    merge_resolution_workspace_id: merge_resolution_workspace_id.clone(),
                    conflicts: immutable_conflicts,
                    support_gate_id: None,
                    support_gate_digest: None,
                    support_gate_history_evidence_digest: None,
                };
                let base_session_digest = merge_digest(&base_record, "base-session digest failed")?;
                let decision_set_digest = decision_set_digest(conflicts.as_slice())?;
                let conflict_count = PositiveCount::new(
                    SafeResultCount::new(conflicts.as_slice().len() as u64).map_err(|_| {
                        MergeResultContractError("conflict count is not I-JSON-safe")
                    })?,
                )?;
                Ok(match merge_resolution_workspace_id {
                    Some(merge_resolution_workspace_id) => {
                        Self::SupportedUpdateConflicted(SupportedUpdateConflictedSessionData {
                            session_id: base_record.session_id,
                            mode: SupportedUpdateMode::Value,
                            checkpoint_id: base_record.checkpoint_id,
                            incoming_distribution_id: base_record
                                .incoming_distribution_id
                                .expect("set above"),
                            immutable_input_hashes: base_record.immutable_input_hashes,
                            anchor_digest: base_record.anchor_digest,
                            settings_digest: base_record.settings_digest,
                            comparison_id: base_record.comparison_id,
                            conflict_count,
                            merge_resolution_workspace_id,
                            base_session_digest,
                            decision_set_digest,
                        })
                    }
                    None => Self::SupportedUpdateConflictedWithoutWorkspace(
                        SupportedUpdateConflictedWithoutWorkspaceSessionData {
                            session_id: base_record.session_id,
                            mode: SupportedUpdateMode::Value,
                            checkpoint_id: base_record.checkpoint_id,
                            incoming_distribution_id: base_record
                                .incoming_distribution_id
                                .expect("set above"),
                            immutable_input_hashes: base_record.immutable_input_hashes,
                            anchor_digest: base_record.anchor_digest,
                            settings_digest: base_record.settings_digest,
                            comparison_id: base_record.comparison_id,
                            conflict_count,
                            base_session_digest,
                            decision_set_digest,
                        },
                    ),
                })
            }
        }
    }

    pub(crate) fn main_integration_from_authorities(
        inputs: MainIntegrationPreparationInputAuthority,
        observation: MainIntegrationSessionObservationAuthority,
    ) -> Result<ValidatedMainIntegrationPreparationAuthority, MergeResultContractError> {
        if !observation.matches(&inputs) {
            return Err(MergeResultContractError(
                "main-integration observation belongs to different preparation inputs",
            ));
        }
        let result_digest = inputs.preflight.sandbox_result_digest().clone();
        let settings_digest = inputs.preflight.settings_digest().clone();
        let base_record = MergeSessionBaseDigestRecord {
            session_id: observation.session_id,
            mode: MergeSessionModeDigest::MainIntegration,
            checkpoint_id: inputs.checkpoint_id,
            incoming_distribution_id: None,
            immutable_input_hashes: ImmutableMergeInputHashes {
                checkpoint_verification_digest: inputs.checkpoint_verification_digest,
                comparison_delta_digest: inputs.comparison_delta_digest,
                source_artifact_sha256: inputs.source_artifact_sha256,
            },
            anchor_digest: observation.anchor_digest,
            settings_digest,
            ordinary_result_artifact_id: Some(inputs.ordinary_result_artifact_id),
            comparison_id: inputs.comparison_id,
            result_digest: Some(result_digest.clone()),
            merge_resolution_workspace_id: None,
            conflicts: vec![],
            support_gate_id: Some(inputs.preflight.support_gate_id().clone()),
            support_gate_digest: Some(inputs.preflight.support_gate_digest().clone()),
            support_gate_history_evidence_digest: Some(
                inputs.preflight.history_evidence_digest().clone(),
            ),
        };
        let base_session_digest = merge_digest(&base_record, "base-session digest failed")?;
        let decision_set_digest = decision_set_digest(&[])?;
        let resolved_session_digest = merge_digest(
            &ResolvedSessionDigestRecord {
                base_session_digest: base_session_digest.clone(),
                decision_set_digest: decision_set_digest.clone(),
                result_digest,
                applied_decision_ids: vec![],
            },
            "resolved-session digest failed",
        )?;
        let session = MainIntegrationSessionData {
            session_id: base_record.session_id,
            mode: MainIntegrationMode::Value,
            checkpoint_id: base_record.checkpoint_id,
            immutable_input_hashes: base_record.immutable_input_hashes,
            anchor_digest: base_record.anchor_digest,
            settings_digest: base_record.settings_digest,
            ordinary_result_artifact_id: base_record
                .ordinary_result_artifact_id
                .expect("set above"),
            comparison_id: base_record.comparison_id,
            result_digest: base_record.result_digest.expect("set above"),
            conflict_count: ZeroCount,
            base_session_digest,
            decision_set_digest,
            resolved_session_digest,
            support_gate_id: base_record.support_gate_id.expect("set above"),
            support_gate_digest: base_record.support_gate_digest.expect("set above"),
            support_gate_history_evidence_digest: base_record
                .support_gate_history_evidence_digest
                .expect("set above"),
        };
        ValidatedMainIntegrationPreparationAuthority::new(inputs.preflight, session)
    }

    fn mode(&self) -> MergeSessionModeDigest {
        match self {
            Self::SupportedUpdateResolved(_)
            | Self::SupportedUpdateConflicted(_)
            | Self::SupportedUpdateConflictedWithoutWorkspace(_) => {
                MergeSessionModeDigest::SupportedUpdate
            }
            Self::ResolvedReplayResolved(_)
            | Self::ResolvedReplayConflicted(_)
            | Self::ResolvedReplayConflictedWithoutWorkspace(_) => {
                MergeSessionModeDigest::ResolvedReplay
            }
            Self::MainIntegration(_) => MergeSessionModeDigest::MainIntegration,
        }
    }

    fn session_id(&self) -> &UnicaId {
        match self {
            Self::SupportedUpdateResolved(value) => &value.session_id,
            Self::SupportedUpdateConflicted(value) => &value.session_id,
            Self::SupportedUpdateConflictedWithoutWorkspace(value) => &value.session_id,
            Self::ResolvedReplayResolved(value) => &value.session_id,
            Self::ResolvedReplayConflicted(value) => &value.session_id,
            Self::ResolvedReplayConflictedWithoutWorkspace(value) => &value.session_id,
            Self::MainIntegration(value) => &value.session_id,
        }
    }

    fn base_session_digest(&self) -> &Sha256Digest {
        match self {
            Self::SupportedUpdateResolved(value) => &value.base_session_digest,
            Self::SupportedUpdateConflicted(value) => &value.base_session_digest,
            Self::SupportedUpdateConflictedWithoutWorkspace(value) => &value.base_session_digest,
            Self::ResolvedReplayResolved(value) => &value.base_session_digest,
            Self::ResolvedReplayConflicted(value) => &value.base_session_digest,
            Self::ResolvedReplayConflictedWithoutWorkspace(value) => &value.base_session_digest,
            Self::MainIntegration(value) => &value.base_session_digest,
        }
    }

    fn decision_set_digest(&self) -> &Sha256Digest {
        match self {
            Self::SupportedUpdateResolved(value) => &value.decision_set_digest,
            Self::SupportedUpdateConflicted(value) => &value.decision_set_digest,
            Self::SupportedUpdateConflictedWithoutWorkspace(value) => &value.decision_set_digest,
            Self::ResolvedReplayResolved(value) => &value.decision_set_digest,
            Self::ResolvedReplayConflicted(value) => &value.decision_set_digest,
            Self::ResolvedReplayConflictedWithoutWorkspace(value) => &value.decision_set_digest,
            Self::MainIntegration(value) => &value.decision_set_digest,
        }
    }

    fn resolved_parts(&self) -> Option<(&Sha256Digest, &Sha256Digest)> {
        match self {
            Self::SupportedUpdateResolved(value) => {
                Some((&value.result_digest, &value.resolved_session_digest))
            }
            Self::ResolvedReplayResolved(value) => {
                Some((&value.result_digest, &value.resolved_session_digest))
            }
            Self::MainIntegration(value) => {
                Some((&value.result_digest, &value.resolved_session_digest))
            }
            Self::SupportedUpdateConflicted(_)
            | Self::SupportedUpdateConflictedWithoutWorkspace(_)
            | Self::ResolvedReplayConflicted(_)
            | Self::ResolvedReplayConflictedWithoutWorkspace(_) => None,
        }
    }

    fn resolved_session_digest(&self) -> Option<&Sha256Digest> {
        self.resolved_parts().map(|(_, digest)| digest)
    }

    fn conflicted_parts(&self) -> Option<(u64, Option<&UnicaId>)> {
        match self {
            Self::SupportedUpdateConflicted(value) => Some((
                value.conflict_count.get(),
                Some(&value.merge_resolution_workspace_id),
            )),
            Self::SupportedUpdateConflictedWithoutWorkspace(value) => {
                Some((value.conflict_count.get(), None))
            }
            Self::ResolvedReplayConflicted(value) => Some((
                value.conflict_count.get(),
                Some(&value.merge_resolution_workspace_id),
            )),
            Self::ResolvedReplayConflictedWithoutWorkspace(value) => {
                Some((value.conflict_count.get(), None))
            }
            Self::SupportedUpdateResolved(_)
            | Self::ResolvedReplayResolved(_)
            | Self::MainIntegration(_) => None,
        }
    }

    fn base_record(&self, conflicts: &[MergeConflict]) -> MergeSessionBaseDigestRecord {
        let immutable_conflicts = conflicts
            .iter()
            .map(ConflictImmutableDigestRecord::from)
            .collect();
        match self {
            Self::SupportedUpdateResolved(value) => MergeSessionBaseDigestRecord {
                session_id: value.session_id.clone(),
                mode: MergeSessionModeDigest::SupportedUpdate,
                checkpoint_id: value.checkpoint_id.clone(),
                incoming_distribution_id: Some(value.incoming_distribution_id.clone()),
                immutable_input_hashes: value.immutable_input_hashes.clone(),
                anchor_digest: value.anchor_digest.clone(),
                settings_digest: value.settings_digest.clone(),
                ordinary_result_artifact_id: None,
                comparison_id: value.comparison_id.clone(),
                result_digest: Some(value.result_digest.clone()),
                merge_resolution_workspace_id: None,
                conflicts: immutable_conflicts,
                support_gate_id: None,
                support_gate_digest: None,
                support_gate_history_evidence_digest: None,
            },
            Self::SupportedUpdateConflicted(value) => MergeSessionBaseDigestRecord {
                session_id: value.session_id.clone(),
                mode: MergeSessionModeDigest::SupportedUpdate,
                checkpoint_id: value.checkpoint_id.clone(),
                incoming_distribution_id: Some(value.incoming_distribution_id.clone()),
                immutable_input_hashes: value.immutable_input_hashes.clone(),
                anchor_digest: value.anchor_digest.clone(),
                settings_digest: value.settings_digest.clone(),
                ordinary_result_artifact_id: None,
                comparison_id: value.comparison_id.clone(),
                result_digest: None,
                merge_resolution_workspace_id: Some(value.merge_resolution_workspace_id.clone()),
                conflicts: immutable_conflicts,
                support_gate_id: None,
                support_gate_digest: None,
                support_gate_history_evidence_digest: None,
            },
            Self::SupportedUpdateConflictedWithoutWorkspace(value) => {
                MergeSessionBaseDigestRecord {
                    session_id: value.session_id.clone(),
                    mode: MergeSessionModeDigest::SupportedUpdate,
                    checkpoint_id: value.checkpoint_id.clone(),
                    incoming_distribution_id: Some(value.incoming_distribution_id.clone()),
                    immutable_input_hashes: value.immutable_input_hashes.clone(),
                    anchor_digest: value.anchor_digest.clone(),
                    settings_digest: value.settings_digest.clone(),
                    ordinary_result_artifact_id: None,
                    comparison_id: value.comparison_id.clone(),
                    result_digest: None,
                    merge_resolution_workspace_id: None,
                    conflicts: immutable_conflicts,
                    support_gate_id: None,
                    support_gate_digest: None,
                    support_gate_history_evidence_digest: None,
                }
            }
            Self::ResolvedReplayResolved(value) => MergeSessionBaseDigestRecord {
                session_id: value.session_id.clone(),
                mode: MergeSessionModeDigest::ResolvedReplay,
                checkpoint_id: value.checkpoint_id.clone(),
                incoming_distribution_id: Some(value.incoming_distribution_id.clone()),
                immutable_input_hashes: value.immutable_input_hashes.clone(),
                anchor_digest: value.anchor_digest.clone(),
                settings_digest: value.settings_digest.clone(),
                ordinary_result_artifact_id: None,
                comparison_id: value.comparison_id.clone(),
                result_digest: Some(value.result_digest.clone()),
                merge_resolution_workspace_id: None,
                conflicts: immutable_conflicts,
                support_gate_id: None,
                support_gate_digest: None,
                support_gate_history_evidence_digest: None,
            },
            Self::ResolvedReplayConflicted(value) => MergeSessionBaseDigestRecord {
                session_id: value.session_id.clone(),
                mode: MergeSessionModeDigest::ResolvedReplay,
                checkpoint_id: value.checkpoint_id.clone(),
                incoming_distribution_id: Some(value.incoming_distribution_id.clone()),
                immutable_input_hashes: value.immutable_input_hashes.clone(),
                anchor_digest: value.anchor_digest.clone(),
                settings_digest: value.settings_digest.clone(),
                ordinary_result_artifact_id: None,
                comparison_id: value.comparison_id.clone(),
                result_digest: None,
                merge_resolution_workspace_id: Some(value.merge_resolution_workspace_id.clone()),
                conflicts: immutable_conflicts,
                support_gate_id: None,
                support_gate_digest: None,
                support_gate_history_evidence_digest: None,
            },
            Self::ResolvedReplayConflictedWithoutWorkspace(value) => MergeSessionBaseDigestRecord {
                session_id: value.session_id.clone(),
                mode: MergeSessionModeDigest::ResolvedReplay,
                checkpoint_id: value.checkpoint_id.clone(),
                incoming_distribution_id: Some(value.incoming_distribution_id.clone()),
                immutable_input_hashes: value.immutable_input_hashes.clone(),
                anchor_digest: value.anchor_digest.clone(),
                settings_digest: value.settings_digest.clone(),
                ordinary_result_artifact_id: None,
                comparison_id: value.comparison_id.clone(),
                result_digest: None,
                merge_resolution_workspace_id: None,
                conflicts: immutable_conflicts,
                support_gate_id: None,
                support_gate_digest: None,
                support_gate_history_evidence_digest: None,
            },
            Self::MainIntegration(value) => MergeSessionBaseDigestRecord {
                session_id: value.session_id.clone(),
                mode: MergeSessionModeDigest::MainIntegration,
                checkpoint_id: value.checkpoint_id.clone(),
                incoming_distribution_id: None,
                immutable_input_hashes: value.immutable_input_hashes.clone(),
                anchor_digest: value.anchor_digest.clone(),
                settings_digest: value.settings_digest.clone(),
                ordinary_result_artifact_id: Some(value.ordinary_result_artifact_id.clone()),
                comparison_id: value.comparison_id.clone(),
                result_digest: Some(value.result_digest.clone()),
                merge_resolution_workspace_id: None,
                conflicts: immutable_conflicts,
                support_gate_id: Some(value.support_gate_id.clone()),
                support_gate_digest: Some(value.support_gate_digest.clone()),
                support_gate_history_evidence_digest: Some(
                    value.support_gate_history_evidence_digest.clone(),
                ),
            },
        }
    }

    fn validate_conflicts(
        &self,
        conflicts: &[MergeConflict],
    ) -> Result<(), MergeResultContractError> {
        let expected_count = self
            .conflicted_parts()
            .map_or(0, |(count, _)| count as usize);
        let workspace_present = self
            .conflicted_parts()
            .and_then(|(_, workspace_id)| workspace_id)
            .is_some();
        let workspace_required = conflicts.iter().any(|conflict| {
            conflict
                .allowed_resolutions
                .contains(MergeResolution::Combine)
                || conflict
                    .allowed_resolutions
                    .contains(MergeResolution::Manual)
        });
        if conflicts.len() != expected_count
            || workspace_present != workspace_required
            || merge_digest(&self.base_record(conflicts), "base-session digest failed")?
                != *self.base_session_digest()
            || decision_set_digest(conflicts)? != *self.decision_set_digest()
        {
            return Err(MergeResultContractError(
                "session conflicts disagree with the immutable or evolving session lineage",
            ));
        }
        Ok(())
    }

    fn validates_resolved_projection(
        &self,
        applied_decision_ids: &[UnicaId],
    ) -> Result<bool, MergeResultContractError> {
        let Some((result_digest, resolved_session_digest)) = self.resolved_parts() else {
            return Ok(false);
        };
        let record = ResolvedSessionDigestRecord {
            base_session_digest: self.base_session_digest().clone(),
            decision_set_digest: self.decision_set_digest().clone(),
            result_digest: result_digest.clone(),
            applied_decision_ids: applied_decision_ids.to_vec(),
        };
        Ok(merge_digest(&record, "resolved-session digest failed")? == *resolved_session_digest)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct ReadySupportPreflightData(SupportPreflightData);

impl ReadySupportPreflightData {
    fn from_authority(authority: ReadySupportPreflightAuthority) -> Self {
        Self(authority.into_data())
    }

    fn history_evidence(&self) -> &SupportGateHistoryEvidence {
        self.0.history_evidence()
    }
}

impl JsonSchema for ReadySupportPreflightData {
    fn schema_name() -> Cow<'static, str> {
        "ReadySupportPreflightData".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let schema = SupportPreflightData::json_schema(generator).to_value();
        let ready = schema
            .get("oneOf")
            .and_then(serde_json::Value::as_array)
            .and_then(|branches| branches.first())
            .and_then(serde_json::Value::as_object)
            .expect("SupportPreflightData keeps ready as its first closed branch")
            .clone();
        Schema::from(ready)
    }
}

/// Consuming proof that the wire preparation is a projection of one exact
/// action-free support preflight and its matching main-integration session.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ValidatedMainIntegrationPreparationAuthority {
    preflight: ReadySupportPreflightData,
    session: MainIntegrationSessionData,
}

impl ValidatedMainIntegrationPreparationAuthority {
    pub(crate) fn new(
        preflight: ReadySupportPreflightAuthority,
        session: MainIntegrationSessionData,
    ) -> Result<Self, MergeResultContractError> {
        let session_projection = MergeSessionData::MainIntegration(session.clone());
        session_projection.validate_conflicts(&[])?;
        if !session_projection.validates_resolved_projection(&[])? {
            return Err(MergeResultContractError(
                "main-integration session has an invalid resolved projection",
            ));
        }
        if preflight.support_gate_id() != &session.support_gate_id
            || preflight.support_gate_digest() != &session.support_gate_digest
            || preflight.history_evidence_digest() != &session.support_gate_history_evidence_digest
            || preflight.ordinary_result_artifact_id() != &session.ordinary_result_artifact_id
            || preflight.comparison_id() != &session.comparison_id
            || preflight.settings_digest() != &session.settings_digest
            || preflight.sandbox_result_digest() != &session.result_digest
        {
            return Err(MergeResultContractError(
                "ready preflight and main-integration session lineage disagree",
            ));
        }
        Ok(Self {
            preflight: ReadySupportPreflightData::from_authority(preflight),
            session,
        })
    }

    pub(crate) fn data(&self) -> MainIntegrationPreparationData {
        MainIntegrationPreparationData {
            preflight: self.preflight.clone(),
            session: self.session.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct MainIntegrationPreparationData {
    preflight: ReadySupportPreflightData,
    session: MainIntegrationSessionData,
}

impl MainIntegrationPreparationData {
    pub(crate) fn from_authority(authority: ValidatedMainIntegrationPreparationAuthority) -> Self {
        Self {
            preflight: authority.preflight,
            session: authority.session,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ConflictListWithWorkspaceData {
    session_id: UnicaId,
    base_session_digest: Sha256Digest,
    decision_set_digest: Sha256Digest,
    merge_resolution_workspace_id: UnicaId,
    conflicts: CanonicalMergeConflicts,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ConflictListWithoutWorkspaceData {
    session_id: UnicaId,
    base_session_digest: Sha256Digest,
    decision_set_digest: Sha256Digest,
    conflicts: CanonicalMergeConflicts,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub(crate) enum ConflictListData {
    WithWorkspace(ConflictListWithWorkspaceData),
    WithoutWorkspace(ConflictListWithoutWorkspaceData),
}

impl JsonSchema for ConflictListData {
    fn schema_name() -> Cow<'static, str> {
        "ConflictListData".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<ConflictListWithWorkspaceData>(),
            generator.subschema_for::<ConflictListWithoutWorkspaceData>(),
        ])
    }
}

impl ConflictListData {
    pub(crate) fn from_current_session(
        session: &MergeSessionData,
        conflicts: Vec<MergeConflict>,
    ) -> Result<Self, MergeResultContractError> {
        let conflicts = CanonicalMergeConflicts::new(conflicts, false)?;
        session.validate_conflicts(conflicts.as_slice())?;
        let (_, workspace_id) = session.conflicted_parts().ok_or(MergeResultContractError(
            "a conflict list requires a conflicted session",
        ))?;
        let needs_workspace = conflicts.as_slice().iter().any(|conflict| {
            conflict
                .allowed_resolutions
                .contains(MergeResolution::Combine)
                || conflict
                    .allowed_resolutions
                    .contains(MergeResolution::Manual)
        });
        if needs_workspace != workspace_id.is_some() {
            return Err(MergeResultContractError(
                "resolution-workspace presence disagrees with persisted allowed resolutions",
            ));
        }
        Ok(match workspace_id {
            Some(workspace_id) => Self::WithWorkspace(ConflictListWithWorkspaceData {
                session_id: session.session_id().clone(),
                base_session_digest: session.base_session_digest().clone(),
                decision_set_digest: session.decision_set_digest().clone(),
                merge_resolution_workspace_id: workspace_id.clone(),
                conflicts,
            }),
            None => Self::WithoutWorkspace(ConflictListWithoutWorkspaceData {
                session_id: session.session_id().clone(),
                base_session_digest: session.base_session_digest().clone(),
                decision_set_digest: session.decision_set_digest().clone(),
                conflicts,
            }),
        })
    }
}

// -------------------------------------------------------------------------
// Resolution results

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TakeOursDecisionDigestRecord {
    decision_id: UnicaId,
    session_id: UnicaId,
    base_session_digest: Sha256Digest,
    conflict_id: UnicaId,
    resolution: TakeOursResolution,
    rationale_digest: Sha256Digest,
    #[serde(skip_serializing_if = "Option::is_none")]
    replaces_decision_id: Option<UnicaId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TakeTheirsDecisionDigestRecord {
    decision_id: UnicaId,
    session_id: UnicaId,
    base_session_digest: Sha256Digest,
    conflict_id: UnicaId,
    resolution: TakeTheirsResolution,
    rationale_digest: Sha256Digest,
    #[serde(skip_serializing_if = "Option::is_none")]
    replaces_decision_id: Option<UnicaId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CombineDecisionDigestRecord {
    decision_id: UnicaId,
    session_id: UnicaId,
    base_session_digest: Sha256Digest,
    conflict_id: UnicaId,
    resolution: CombineResolution,
    rationale_digest: Sha256Digest,
    change_receipt_digest: Sha256Digest,
    #[serde(skip_serializing_if = "Option::is_none")]
    replaces_decision_id: Option<UnicaId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ManualDecisionDigestRecord {
    decision_id: UnicaId,
    session_id: UnicaId,
    base_session_digest: Sha256Digest,
    conflict_id: UnicaId,
    resolution: ManualResolution,
    rationale_digest: Sha256Digest,
    change_receipt_digest: Sha256Digest,
    #[serde(skip_serializing_if = "Option::is_none")]
    replaces_decision_id: Option<UnicaId>,
}

macro_rules! seal_digest_record {
    ($($name:ty),+ $(,)?) => {
        $(
            impl contract_digest_record_sealed::Sealed for $name {}
            impl ContractDigestRecord for $name {}
        )+
    };
}

seal_digest_record!(
    TakeOursDecisionDigestRecord,
    TakeTheirsDecisionDigestRecord,
    CombineDecisionDigestRecord,
    ManualDecisionDigestRecord,
);

wire_literal!(TakeOursResolution, "takeOurs");
wire_literal!(TakeTheirsResolution, "takeTheirs");
wire_literal!(CombineResolution, "combine");
wire_literal!(ManualResolution, "manual");

macro_rules! decision_data_leaf {
    ($name:ident, $resolution:ty $(, $field:ident : $field_type:ty )* $(,)?) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        pub(crate) struct $name {
            decision_id: UnicaId,
            session_id: UnicaId,
            base_session_digest: Sha256Digest,
            conflict_id: UnicaId,
            resolution: $resolution,
            rationale_digest: Sha256Digest,
            $($field: $field_type,)*
            #[serde(skip_serializing_if = "Option::is_none")]
            replaces_decision_id: Option<UnicaId>,
            decision_digest: Sha256Digest,
            revised_decision_set_digest: Sha256Digest,
        }
    };
}

decision_data_leaf!(TakeOursDecisionData, TakeOursResolution);
decision_data_leaf!(TakeTheirsDecisionData, TakeTheirsResolution);
decision_data_leaf!(
    CombineDecisionData,
    CombineResolution,
    change_receipt_digest: Sha256Digest,
);
decision_data_leaf!(
    ManualDecisionData,
    ManualResolution,
    change_receipt_digest: Sha256Digest,
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub(crate) enum ConflictDecisionData {
    TakeOurs(TakeOursDecisionData),
    TakeTheirs(TakeTheirsDecisionData),
    Combine(CombineDecisionData),
    Manual(ManualDecisionData),
}

impl JsonSchema for ConflictDecisionData {
    fn schema_name() -> Cow<'static, str> {
        "ConflictDecisionData".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<TakeOursDecisionData>(),
            generator.subschema_for::<TakeTheirsDecisionData>(),
            generator.subschema_for::<CombineDecisionData>(),
            generator.subschema_for::<ManualDecisionData>(),
        ])
    }
}

impl ConflictDecisionData {
    fn decision_id(&self) -> &UnicaId {
        match self {
            Self::TakeOurs(value) => &value.decision_id,
            Self::TakeTheirs(value) => &value.decision_id,
            Self::Combine(value) => &value.decision_id,
            Self::Manual(value) => &value.decision_id,
        }
    }

    fn session_id(&self) -> &UnicaId {
        match self {
            Self::TakeOurs(value) => &value.session_id,
            Self::TakeTheirs(value) => &value.session_id,
            Self::Combine(value) => &value.session_id,
            Self::Manual(value) => &value.session_id,
        }
    }

    fn base_session_digest(&self) -> &Sha256Digest {
        match self {
            Self::TakeOurs(value) => &value.base_session_digest,
            Self::TakeTheirs(value) => &value.base_session_digest,
            Self::Combine(value) => &value.base_session_digest,
            Self::Manual(value) => &value.base_session_digest,
        }
    }

    fn conflict_id(&self) -> &UnicaId {
        match self {
            Self::TakeOurs(value) => &value.conflict_id,
            Self::TakeTheirs(value) => &value.conflict_id,
            Self::Combine(value) => &value.conflict_id,
            Self::Manual(value) => &value.conflict_id,
        }
    }

    fn validates_decision_digest(&self) -> Result<bool, MergeResultContractError> {
        match self {
            Self::TakeOurs(value) => merge_digest(
                &TakeOursDecisionDigestRecord {
                    decision_id: value.decision_id.clone(),
                    session_id: value.session_id.clone(),
                    base_session_digest: value.base_session_digest.clone(),
                    conflict_id: value.conflict_id.clone(),
                    resolution: value.resolution,
                    rationale_digest: value.rationale_digest.clone(),
                    replaces_decision_id: value.replaces_decision_id.clone(),
                },
                "merge-decision digest failed",
            )
            .map(|digest| digest == value.decision_digest),
            Self::TakeTheirs(value) => merge_digest(
                &TakeTheirsDecisionDigestRecord {
                    decision_id: value.decision_id.clone(),
                    session_id: value.session_id.clone(),
                    base_session_digest: value.base_session_digest.clone(),
                    conflict_id: value.conflict_id.clone(),
                    resolution: value.resolution,
                    rationale_digest: value.rationale_digest.clone(),
                    replaces_decision_id: value.replaces_decision_id.clone(),
                },
                "merge-decision digest failed",
            )
            .map(|digest| digest == value.decision_digest),
            Self::Combine(value) => merge_digest(
                &CombineDecisionDigestRecord {
                    decision_id: value.decision_id.clone(),
                    session_id: value.session_id.clone(),
                    base_session_digest: value.base_session_digest.clone(),
                    conflict_id: value.conflict_id.clone(),
                    resolution: value.resolution,
                    rationale_digest: value.rationale_digest.clone(),
                    change_receipt_digest: value.change_receipt_digest.clone(),
                    replaces_decision_id: value.replaces_decision_id.clone(),
                },
                "merge-decision digest failed",
            )
            .map(|digest| digest == value.decision_digest),
            Self::Manual(value) => merge_digest(
                &ManualDecisionDigestRecord {
                    decision_id: value.decision_id.clone(),
                    session_id: value.session_id.clone(),
                    base_session_digest: value.base_session_digest.clone(),
                    conflict_id: value.conflict_id.clone(),
                    resolution: value.resolution,
                    rationale_digest: value.rationale_digest.clone(),
                    change_receipt_digest: value.change_receipt_digest.clone(),
                    replaces_decision_id: value.replaces_decision_id.clone(),
                },
                "merge-decision digest failed",
            )
            .map(|digest| digest == value.decision_digest),
        }
    }

    fn from_conflict_parts(
        session: &MergeSessionData,
        conflicts: &[MergeConflict],
        decision_id: UnicaId,
        conflict_id: &UnicaId,
        resolution: MergeResolution,
        rationale_digest: Sha256Digest,
        change_receipt_digest: Option<Sha256Digest>,
    ) -> Result<Self, MergeResultContractError> {
        session.validate_conflicts(conflicts)?;
        let position = conflicts
            .iter()
            .position(|conflict| &conflict.conflict_id == conflict_id)
            .ok_or(MergeResultContractError(
                "decision conflict is absent from the session",
            ))?;
        let conflict = &conflicts[position];
        if !conflict.allowed_resolutions.contains(resolution) {
            return Err(MergeResultContractError(
                "decision resolution is not allowed for this conflict",
            ));
        }
        let receipt_required = matches!(
            resolution,
            MergeResolution::Combine | MergeResolution::Manual
        );
        if receipt_required != change_receipt_digest.is_some() {
            return Err(MergeResultContractError(
                "change-receipt digest presence disagrees with the resolution",
            ));
        }
        let predecessor = conflict.decision_state.predecessor_id().cloned();
        let mut revised = conflicts.to_vec();
        revised[position].decision_state = ConflictDecisionState::Current(CurrentConflictState {
            state_kind: CurrentStateKind::Value,
            decision_id: decision_id.clone(),
        });
        let revised_decision_set_digest = decision_set_digest(&revised)?;
        let common = (
            decision_id,
            session.session_id().clone(),
            session.base_session_digest().clone(),
            conflict_id.clone(),
            rationale_digest,
            predecessor,
        );
        Ok(match resolution {
            MergeResolution::TakeOurs => {
                let record = TakeOursDecisionDigestRecord {
                    decision_id: common.0,
                    session_id: common.1,
                    base_session_digest: common.2,
                    conflict_id: common.3,
                    resolution: TakeOursResolution::Value,
                    rationale_digest: common.4,
                    replaces_decision_id: common.5,
                };
                let decision_digest = merge_digest(&record, "merge-decision digest failed")?;
                Self::TakeOurs(TakeOursDecisionData {
                    decision_id: record.decision_id,
                    session_id: record.session_id,
                    base_session_digest: record.base_session_digest,
                    conflict_id: record.conflict_id,
                    resolution: record.resolution,
                    rationale_digest: record.rationale_digest,
                    replaces_decision_id: record.replaces_decision_id,
                    decision_digest,
                    revised_decision_set_digest,
                })
            }
            MergeResolution::TakeTheirs => {
                let record = TakeTheirsDecisionDigestRecord {
                    decision_id: common.0,
                    session_id: common.1,
                    base_session_digest: common.2,
                    conflict_id: common.3,
                    resolution: TakeTheirsResolution::Value,
                    rationale_digest: common.4,
                    replaces_decision_id: common.5,
                };
                let decision_digest = merge_digest(&record, "merge-decision digest failed")?;
                Self::TakeTheirs(TakeTheirsDecisionData {
                    decision_id: record.decision_id,
                    session_id: record.session_id,
                    base_session_digest: record.base_session_digest,
                    conflict_id: record.conflict_id,
                    resolution: record.resolution,
                    rationale_digest: record.rationale_digest,
                    replaces_decision_id: record.replaces_decision_id,
                    decision_digest,
                    revised_decision_set_digest,
                })
            }
            MergeResolution::Combine => {
                let record = CombineDecisionDigestRecord {
                    decision_id: common.0,
                    session_id: common.1,
                    base_session_digest: common.2,
                    conflict_id: common.3,
                    resolution: CombineResolution::Value,
                    rationale_digest: common.4,
                    change_receipt_digest: change_receipt_digest.expect("presence checked"),
                    replaces_decision_id: common.5,
                };
                let decision_digest = merge_digest(&record, "merge-decision digest failed")?;
                Self::Combine(CombineDecisionData {
                    decision_id: record.decision_id,
                    session_id: record.session_id,
                    base_session_digest: record.base_session_digest,
                    conflict_id: record.conflict_id,
                    resolution: record.resolution,
                    rationale_digest: record.rationale_digest,
                    change_receipt_digest: record.change_receipt_digest,
                    replaces_decision_id: record.replaces_decision_id,
                    decision_digest,
                    revised_decision_set_digest,
                })
            }
            MergeResolution::Manual => {
                let record = ManualDecisionDigestRecord {
                    decision_id: common.0,
                    session_id: common.1,
                    base_session_digest: common.2,
                    conflict_id: common.3,
                    resolution: ManualResolution::Value,
                    rationale_digest: common.4,
                    change_receipt_digest: change_receipt_digest.expect("presence checked"),
                    replaces_decision_id: common.5,
                };
                let decision_digest = merge_digest(&record, "merge-decision digest failed")?;
                Self::Manual(ManualDecisionData {
                    decision_id: record.decision_id,
                    session_id: record.session_id,
                    base_session_digest: record.base_session_digest,
                    conflict_id: record.conflict_id,
                    resolution: record.resolution,
                    rationale_digest: record.rationale_digest,
                    change_receipt_digest: record.change_receipt_digest,
                    replaces_decision_id: record.replaces_decision_id,
                    decision_digest,
                    revised_decision_set_digest,
                })
            }
        })
    }

    #[cfg(test)]
    fn from_conflict_test_only(
        session: &MergeSessionData,
        conflicts: &[MergeConflict],
        decision_id: UnicaId,
        conflict_id: &UnicaId,
        resolution: MergeResolution,
        rationale_digest: Sha256Digest,
        change_receipt_digest: Option<Sha256Digest>,
    ) -> Result<Self, MergeResultContractError> {
        Self::from_conflict_parts(
            session,
            conflicts,
            decision_id,
            conflict_id,
            resolution,
            rationale_digest,
            change_receipt_digest,
        )
    }
}

/// Digest-bound current-head map projected in the platform classifier's exact
/// conflict order. The caller supplies immutable decision records, never a
/// free-standing ID list; undecided/replacement-pending states and historical
/// revisions cannot be promoted to this authority.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CurrentConflictDecisionHeadsAuthority {
    session_id: UnicaId,
    base_session_digest: Sha256Digest,
    decision_set_digest: Sha256Digest,
    applied_decision_ids: AppliedDecisionIds,
}

impl CurrentConflictDecisionHeadsAuthority {
    pub(crate) fn new(
        session: &MergeSessionData,
        conflicts: &[MergeConflict],
        decisions: Vec<ConflictDecisionData>,
    ) -> Result<Self, MergeResultContractError> {
        session.validate_conflicts(conflicts)?;
        if conflicts.is_empty() || decisions.len() != conflicts.len() {
            return Err(MergeResultContractError(
                "resolved replay requires one current decision for every conflict",
            ));
        }
        let mut applied = Vec::with_capacity(conflicts.len());
        for (conflict, decision) in conflicts.iter().zip(decisions.iter()) {
            let ConflictDecisionState::Current(current) = &conflict.decision_state else {
                return Err(MergeResultContractError(
                    "resolved replay rejects undecided or replacement-pending conflicts",
                ));
            };
            if &current.decision_id != decision.decision_id()
                || decision.session_id() != session.session_id()
                || decision.base_session_digest() != session.base_session_digest()
                || decision.conflict_id() != &conflict.conflict_id
                || !decision.validates_decision_digest()?
            {
                return Err(MergeResultContractError(
                    "resolved replay decision is reordered, historical, or from another session",
                ));
            }
            applied.push(decision.decision_id().clone());
        }
        Ok(Self {
            session_id: session.session_id().clone(),
            base_session_digest: session.base_session_digest().clone(),
            decision_set_digest: session.decision_set_digest().clone(),
            applied_decision_ids: AppliedDecisionIds::new(applied)?,
        })
    }

    pub(crate) fn applied_decision_ids(&self) -> &[UnicaId] {
        &self.applied_decision_ids.0
    }
}

/// The only applied-decision projection accepted by a later task apply. It is
/// minted together with the resolved-replay session whose digest covers the
/// exact current-head IDs.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ResolvedApplyDecisionProjectionAuthority {
    session_id: UnicaId,
    resolved_session_digest: Sha256Digest,
    applied_decision_ids: AppliedDecisionIds,
}

impl ResolvedApplyDecisionProjectionAuthority {
    pub(crate) fn applied_decision_ids(&self) -> &[UnicaId] {
        &self.applied_decision_ids.0
    }

    pub(crate) fn for_no_conflict_session(
        session: &MergeSessionData,
    ) -> Result<Self, MergeResultContractError> {
        if !matches!(
            session,
            MergeSessionData::SupportedUpdateResolved(_) | MergeSessionData::MainIntegration(_)
        ) || !session.validates_resolved_projection(&[])?
        {
            return Err(MergeResultContractError(
                "empty apply projection requires an exact non-replay zero-conflict session",
            ));
        }
        let (_, resolved_session_digest) = session.resolved_parts().ok_or(
            MergeResultContractError("resolved apply projection requires a resolved session"),
        )?;
        Ok(Self {
            session_id: session.session_id().clone(),
            resolved_session_digest: resolved_session_digest.clone(),
            applied_decision_ids: AppliedDecisionIds::new(vec![])?,
        })
    }
}

/// Consuming projection of the exact resolved main-integration session that
/// repository planning may bind. Repository code cannot mint a plan from raw
/// caller-supplied session/gate/result digests.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ValidatedRepositoryPlanSessionProjection {
    merge_session_id: UnicaId,
    resolved_session_digest: Sha256Digest,
    support_gate_id: UnicaId,
    support_gate_digest: Sha256Digest,
    support_gate_history_evidence: SupportGateHistoryEvidence,
    settings_digest: Sha256Digest,
    comparison_id: UnicaId,
    ordinary_result_artifact_id: UnicaId,
    result_digest: Sha256Digest,
    applied_decision_ids: AppliedDecisionIds,
}

impl ValidatedRepositoryPlanSessionProjection {
    pub(crate) fn from_main_preparation(
        preparation: ValidatedMainIntegrationPreparationAuthority,
        decision_projection: ResolvedApplyDecisionProjectionAuthority,
    ) -> Result<Self, MergeResultContractError> {
        let main = &preparation.session;
        let session = MergeSessionData::MainIntegration(main.clone());
        if decision_projection.session_id != main.session_id
            || decision_projection.resolved_session_digest != main.resolved_session_digest
            || !session
                .validates_resolved_projection(&decision_projection.applied_decision_ids.0)?
        {
            return Err(MergeResultContractError(
                "repository planning session projection disagrees with resolved decision lineage",
            ));
        }
        Ok(Self {
            merge_session_id: main.session_id.clone(),
            resolved_session_digest: main.resolved_session_digest.clone(),
            support_gate_id: main.support_gate_id.clone(),
            support_gate_digest: main.support_gate_digest.clone(),
            support_gate_history_evidence: preparation.preflight.history_evidence().clone(),
            settings_digest: main.settings_digest.clone(),
            comparison_id: main.comparison_id.clone(),
            ordinary_result_artifact_id: main.ordinary_result_artifact_id.clone(),
            result_digest: main.result_digest.clone(),
            applied_decision_ids: decision_projection.applied_decision_ids,
        })
    }

    pub(crate) fn merge_session_id(&self) -> &UnicaId {
        &self.merge_session_id
    }

    pub(crate) fn resolved_session_digest(&self) -> &Sha256Digest {
        &self.resolved_session_digest
    }

    pub(crate) fn support_gate_id(&self) -> &UnicaId {
        &self.support_gate_id
    }

    pub(crate) fn support_gate_digest(&self) -> &Sha256Digest {
        &self.support_gate_digest
    }

    pub(crate) fn support_gate_history_evidence_digest(&self) -> &Sha256Digest {
        self.support_gate_history_evidence.evidence_digest()
    }

    pub(crate) fn support_gate_history_evidence(&self) -> &SupportGateHistoryEvidence {
        &self.support_gate_history_evidence
    }

    pub(crate) fn settings_digest(&self) -> &Sha256Digest {
        &self.settings_digest
    }

    pub(crate) fn comparison_id(&self) -> &UnicaId {
        &self.comparison_id
    }

    pub(crate) fn ordinary_result_artifact_id(&self) -> &UnicaId {
        &self.ordinary_result_artifact_id
    }

    pub(crate) fn result_digest(&self) -> &Sha256Digest {
        &self.result_digest
    }

    pub(crate) fn applied_decision_ids(&self) -> &[UnicaId] {
        &self.applied_decision_ids.0
    }
}

/// Atomic resolved-replay result plus the sealed current-head projection that
/// must be carried into apply. All immutable source inputs are copied from the
/// prior conflicted session; callers choose only the new ID and observed
/// replay result digest.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ResolvedReplayPreparationAuthority {
    session: ResolvedReplayResolvedSessionData,
    projection: ResolvedApplyDecisionProjectionAuthority,
}

impl ResolvedReplayPreparationAuthority {
    pub(crate) fn new(
        prior_session: &MergeSessionData,
        heads: CurrentConflictDecisionHeadsAuthority,
        replay_session_id: UnicaId,
        result_digest: Sha256Digest,
    ) -> Result<Self, MergeResultContractError> {
        if heads.session_id != *prior_session.session_id()
            || heads.base_session_digest != *prior_session.base_session_digest()
            || heads.decision_set_digest != *prior_session.decision_set_digest()
        {
            return Err(MergeResultContractError(
                "resolved-replay heads do not belong to the prior current session",
            ));
        }
        let (
            checkpoint_id,
            incoming_distribution_id,
            immutable_input_hashes,
            anchor_digest,
            settings_digest,
            comparison_id,
        ) = match prior_session {
            MergeSessionData::SupportedUpdateConflicted(value) => (
                &value.checkpoint_id,
                &value.incoming_distribution_id,
                &value.immutable_input_hashes,
                &value.anchor_digest,
                &value.settings_digest,
                &value.comparison_id,
            ),
            MergeSessionData::SupportedUpdateConflictedWithoutWorkspace(value) => (
                &value.checkpoint_id,
                &value.incoming_distribution_id,
                &value.immutable_input_hashes,
                &value.anchor_digest,
                &value.settings_digest,
                &value.comparison_id,
            ),
            MergeSessionData::ResolvedReplayConflicted(value) => (
                &value.checkpoint_id,
                &value.incoming_distribution_id,
                &value.immutable_input_hashes,
                &value.anchor_digest,
                &value.settings_digest,
                &value.comparison_id,
            ),
            MergeSessionData::ResolvedReplayConflictedWithoutWorkspace(value) => (
                &value.checkpoint_id,
                &value.incoming_distribution_id,
                &value.immutable_input_hashes,
                &value.anchor_digest,
                &value.settings_digest,
                &value.comparison_id,
            ),
            MergeSessionData::SupportedUpdateResolved(_)
            | MergeSessionData::ResolvedReplayResolved(_)
            | MergeSessionData::MainIntegration(_) => {
                return Err(MergeResultContractError(
                    "resolved replay requires a prior conflicted supported session",
                ));
            }
        };
        let base_record = MergeSessionBaseDigestRecord {
            session_id: replay_session_id,
            mode: MergeSessionModeDigest::ResolvedReplay,
            checkpoint_id: checkpoint_id.clone(),
            incoming_distribution_id: Some(incoming_distribution_id.clone()),
            immutable_input_hashes: immutable_input_hashes.clone(),
            anchor_digest: anchor_digest.clone(),
            settings_digest: settings_digest.clone(),
            ordinary_result_artifact_id: None,
            comparison_id: comparison_id.clone(),
            result_digest: Some(result_digest),
            merge_resolution_workspace_id: None,
            conflicts: vec![],
            support_gate_id: None,
            support_gate_digest: None,
            support_gate_history_evidence_digest: None,
        };
        let base_session_digest =
            merge_digest(&base_record, "resolved-replay base-session digest failed")?;
        let decision_set_digest = decision_set_digest(&[])?;
        let resolved_session_digest = merge_digest(
            &ResolvedSessionDigestRecord {
                base_session_digest: base_session_digest.clone(),
                decision_set_digest: decision_set_digest.clone(),
                result_digest: base_record.result_digest.clone().expect("set above"),
                applied_decision_ids: heads.applied_decision_ids.0.clone(),
            },
            "resolved-replay resolved-session digest failed",
        )?;
        let session = ResolvedReplayResolvedSessionData {
            session_id: base_record.session_id.clone(),
            mode: ResolvedReplayMode::Value,
            checkpoint_id: base_record.checkpoint_id,
            incoming_distribution_id: base_record.incoming_distribution_id.expect("set above"),
            immutable_input_hashes: base_record.immutable_input_hashes,
            anchor_digest: base_record.anchor_digest,
            settings_digest: base_record.settings_digest,
            comparison_id: base_record.comparison_id,
            result_digest: base_record.result_digest.expect("set above"),
            conflict_count: ZeroCount,
            base_session_digest,
            decision_set_digest,
            resolved_session_digest: resolved_session_digest.clone(),
        };
        let projection = ResolvedApplyDecisionProjectionAuthority {
            session_id: session.session_id.clone(),
            resolved_session_digest,
            applied_decision_ids: heads.applied_decision_ids,
        };
        Ok(Self {
            session,
            projection,
        })
    }

    pub(crate) fn into_parts(self) -> (MergeSessionData, ResolvedApplyDecisionProjectionAuthority) {
        (
            MergeSessionData::ResolvedReplayResolved(self.session),
            self.projection,
        )
    }
}

/// One atomic journal projection for a receipt-bound conflict decision. The
/// decision and consumed receipt handle cannot be produced independently.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ConflictDecisionCommitAuthority {
    decision: ConflictDecisionData,
    consumed_receipt_handle: ResolutionChangeReceiptResumeHandle,
}

/// Linear commit for take-ours/take-theirs. Its distinct type makes it
/// impossible to pretend that a combine/manual change receipt was consumed.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ReceiptFreeConflictDecisionCommitAuthority {
    decision: ConflictDecisionData,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ChangedConflictDecisionCommitInput {
    decision_id: UnicaId,
    conflict_id: UnicaId,
    resolution: MergeResolution,
    rationale_digest: Sha256Digest,
    requested_target: MetadataPropertyAffectedTarget,
    expected_result_sha256: Sha256Digest,
}

impl ChangedConflictDecisionCommitInput {
    pub(crate) const fn new(
        decision_id: UnicaId,
        conflict_id: UnicaId,
        resolution: MergeResolution,
        rationale_digest: Sha256Digest,
        requested_target: MetadataPropertyAffectedTarget,
        expected_result_sha256: Sha256Digest,
    ) -> Self {
        Self {
            decision_id,
            conflict_id,
            resolution,
            rationale_digest,
            requested_target,
            expected_result_sha256,
        }
    }
}

impl ReceiptFreeConflictDecisionCommitAuthority {
    pub(crate) fn into_data(self) -> ConflictDecisionData {
        self.decision
    }
}

impl ConflictDecisionCommitAuthority {
    pub(crate) fn without_changed_receipt(
        session: &MergeSessionData,
        conflicts: &[MergeConflict],
        decision_id: UnicaId,
        conflict_id: &UnicaId,
        resolution: MergeResolution,
        rationale_digest: Sha256Digest,
    ) -> Result<ReceiptFreeConflictDecisionCommitAuthority, MergeResultContractError> {
        if !matches!(
            resolution,
            MergeResolution::TakeOurs | MergeResolution::TakeTheirs
        ) {
            return Err(MergeResultContractError(
                "receipt-free commit is only valid for takeOurs or takeTheirs",
            ));
        }
        Ok(ReceiptFreeConflictDecisionCommitAuthority {
            decision: ConflictDecisionData::from_conflict_parts(
                session,
                conflicts,
                decision_id,
                conflict_id,
                resolution,
                rationale_digest,
                None,
            )?,
        })
    }

    pub(crate) fn with_changed_receipt(
        session: &MergeSessionData,
        conflicts: &[MergeConflict],
        input: ChangedConflictDecisionCommitInput,
        receipt: SelectableResolutionChangeReceiptAuthority,
    ) -> Result<Self, MergeResultContractError> {
        if !matches!(
            input.resolution,
            MergeResolution::Combine | MergeResolution::Manual
        ) {
            return Err(MergeResultContractError(
                "a changed receipt is only valid for combine or manual resolution",
            ));
        }
        session.validate_conflicts(conflicts)?;
        let conflict = conflicts
            .iter()
            .find(|value| value.conflict_id == input.conflict_id)
            .ok_or(MergeResultContractError(
                "decision conflict is absent from the session",
            ))?;
        let persisted_target = MetadataPropertyAffectedTarget::new(
            conflict.object_id.clone(),
            conflict.property_path.clone(),
        );
        let workspace_generation = session
            .conflicted_parts()
            .and_then(|(_, workspace)| workspace)
            .ok_or(MergeResultContractError(
                "receipt-bound resolution requires a live resolution workspace",
            ))?;
        if receipt.session_id() != session.session_id()
            || receipt.base_session_digest() != session.base_session_digest()
            || receipt.workspace_generation_id() != workspace_generation
            || receipt.affected_target() != &persisted_target
            || receipt.affected_target() != &input.requested_target
            || receipt.after_sha256() != &input.expected_result_sha256
        {
            return Err(MergeResultContractError(
                "selected changed receipt disagrees with session, generation, target, or result hash",
            ));
        }
        let change_receipt_digest = receipt.change_receipt_digest().clone();
        let decision = ConflictDecisionData::from_conflict_parts(
            session,
            conflicts,
            input.decision_id,
            &input.conflict_id,
            input.resolution,
            input.rationale_digest,
            Some(change_receipt_digest),
        )?;
        let consumed_receipt_handle = receipt
            .into_consumed_handle()
            .map_err(|_| MergeResultContractError("consumed receipt projection failed"))?;
        Ok(Self {
            decision,
            consumed_receipt_handle,
        })
    }

    pub(crate) fn into_parts(self) -> (ConflictDecisionData, ResolutionChangeReceiptResumeHandle) {
        (self.decision, self.consumed_receipt_handle)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct AdaptedDeltaDecisionDigestRecord {
    decision_id: UnicaId,
    verification_id: UnicaId,
    canonical_delta_digest: Sha256Digest,
    difference_digest: Sha256Digest,
    rationale_digest: Sha256Digest,
}

impl contract_digest_record_sealed::Sealed for AdaptedDeltaDecisionDigestRecord {}
impl ContractDigestRecord for AdaptedDeltaDecisionDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct AdaptedDeltaDecisionData {
    decision_id: UnicaId,
    verification_id: UnicaId,
    canonical_delta_digest: Sha256Digest,
    difference_digest: Sha256Digest,
    rationale_digest: Sha256Digest,
    adaptation_decision_digest: Sha256Digest,
}

impl AdaptedDeltaDecisionData {
    fn validates_digest(&self) -> Result<bool, MergeResultContractError> {
        merge_digest(
            &AdaptedDeltaDecisionDigestRecord {
                decision_id: self.decision_id.clone(),
                verification_id: self.verification_id.clone(),
                canonical_delta_digest: self.canonical_delta_digest.clone(),
                difference_digest: self.difference_digest.clone(),
                rationale_digest: self.rationale_digest.clone(),
            },
            "adapted-delta decision digest failed",
        )
        .map(|digest| digest == self.adaptation_decision_digest)
    }

    #[cfg(test)]
    fn new_test_only(
        decision_id: UnicaId,
        verification_id: UnicaId,
        canonical_delta_digest: Sha256Digest,
        difference_digest: Sha256Digest,
        rationale_digest: Sha256Digest,
    ) -> Result<Self, MergeResultContractError> {
        let record = AdaptedDeltaDecisionDigestRecord {
            decision_id,
            verification_id,
            canonical_delta_digest,
            difference_digest,
            rationale_digest,
        };
        let adaptation_decision_digest =
            merge_digest(&record, "adapted-delta decision digest failed")?;
        Ok(Self {
            decision_id: record.decision_id,
            verification_id: record.verification_id,
            canonical_delta_digest: record.canonical_delta_digest,
            difference_digest: record.difference_digest,
            rationale_digest: record.rationale_digest,
            adaptation_decision_digest,
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct SynchronizedUnexpectedVerificationObservationAuthority {
    session_id: UnicaId,
    resolved_session_digest: Sha256Digest,
    verification_id: UnicaId,
    canonical_delta_digest: Sha256Digest,
    difference_manifest_id: UnicaId,
    difference_digest: Sha256Digest,
    validation_receipt_ids: ValidationReceiptIds,
    support_audit_digest: Sha256Digest,
    selected_object_fingerprints: SelectedObjectFingerprints,
}

impl SynchronizedUnexpectedVerificationObservationAuthority {
    pub(crate) fn from_verifier_adapter(
        session: &MergeSessionData,
        input: ResolvedTaskVerificationObservationInputAuthority,
        difference_manifest_id: UnicaId,
        difference_digest: Sha256Digest,
    ) -> Result<Self, MergeResultContractError> {
        if !matches!(
            session,
            MergeSessionData::SupportedUpdateResolved(_)
                | MergeSessionData::ResolvedReplayResolved(_)
        ) {
            return Err(MergeResultContractError(
                "unexpected verification requires a resolved task session",
            ));
        }
        let common = input.into_common_resolved_task(session)?;
        Ok(Self {
            session_id: session.session_id().clone(),
            resolved_session_digest: session
                .resolved_session_digest()
                .expect("resolved branch checked above")
                .clone(),
            verification_id: common.verification_id,
            canonical_delta_digest: common.canonical_delta_digest,
            difference_manifest_id,
            difference_digest,
            validation_receipt_ids: common.validation_receipt_ids,
            support_audit_digest: common.support_audit_digest,
            selected_object_fingerprints: common.selected_object_fingerprints,
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CurrentUnexpectedVerificationAuthority {
    verification: SynchronizedTaskUnexpectedVerificationData,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct UnexpectedVerificationCommitAuthority {
    data: MergeVerificationData,
    current: CurrentUnexpectedVerificationAuthority,
}

impl UnexpectedVerificationCommitAuthority {
    pub(crate) fn into_parts(
        self,
    ) -> (
        MergeVerificationData,
        CurrentUnexpectedVerificationAuthority,
    ) {
        (self.data, self.current)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CurrentAdaptationDecisionAuthority {
    unexpected_verification: SynchronizedTaskUnexpectedVerificationData,
    adaptation_decision: AdaptedDeltaDecisionData,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct AdaptedDeltaDecisionCommitAuthority {
    data: AdaptedDeltaDecisionData,
    current: CurrentAdaptationDecisionAuthority,
}

impl AdaptedDeltaDecisionCommitAuthority {
    pub(crate) fn from_current_unexpected(
        unexpected: CurrentUnexpectedVerificationAuthority,
        decision_id: UnicaId,
        rationale_digest: Sha256Digest,
    ) -> Result<Self, MergeResultContractError> {
        let unexpected = unexpected.verification;
        let record = AdaptedDeltaDecisionDigestRecord {
            decision_id: decision_id.clone(),
            verification_id: unexpected.verification_id.clone(),
            canonical_delta_digest: unexpected.canonical_delta_digest.clone(),
            difference_digest: unexpected.difference_digest.clone(),
            rationale_digest,
        };
        let adaptation_decision_digest =
            merge_digest(&record, "adapted-delta decision digest failed")?;
        let data = AdaptedDeltaDecisionData {
            decision_id: record.decision_id.clone(),
            verification_id: record.verification_id.clone(),
            canonical_delta_digest: record.canonical_delta_digest.clone(),
            difference_digest: record.difference_digest.clone(),
            rationale_digest: record.rationale_digest,
            adaptation_decision_digest: adaptation_decision_digest.clone(),
        };
        Ok(Self {
            data: data.clone(),
            current: CurrentAdaptationDecisionAuthority {
                unexpected_verification: unexpected,
                adaptation_decision: data,
            },
        })
    }

    pub(crate) fn into_parts(
        self,
    ) -> (AdaptedDeltaDecisionData, CurrentAdaptationDecisionAuthority) {
        (self.data, self.current)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct AdaptedVerificationObservationAuthority {
    session_id: UnicaId,
    resolved_session_digest: Sha256Digest,
    adaptation_decision_id: UnicaId,
    adaptation_decision_digest: Sha256Digest,
    verification_id: UnicaId,
    checkpoint_id: UnicaId,
    canonical_delta_digest: Sha256Digest,
    difference_manifest_id: UnicaId,
    difference_digest: Sha256Digest,
    validation_receipt_ids: ValidationReceiptIds,
    support_audit_digest: Sha256Digest,
    selected_object_fingerprints: SelectedObjectFingerprints,
}

impl AdaptedVerificationObservationAuthority {
    pub(crate) fn from_verifier_adapter(
        session: &MergeSessionData,
        current: &CurrentAdaptationDecisionAuthority,
        input: ResolvedTaskVerificationObservationInputAuthority,
        checkpoint_id: UnicaId,
    ) -> Result<Self, MergeResultContractError> {
        if !matches!(
            session,
            MergeSessionData::SupportedUpdateResolved(_)
                | MergeSessionData::ResolvedReplayResolved(_)
        ) || current.unexpected_verification.session_id != *session.session_id()
            || !current.adaptation_decision.validates_digest()?
        {
            return Err(MergeResultContractError(
                "adapted observation requires the current decision for this resolved session",
            ));
        }
        let common = input.into_common_resolved_task(session)?;
        if common.canonical_delta_digest != current.unexpected_verification.canonical_delta_digest {
            return Err(MergeResultContractError(
                "adapted observation disagrees with the current canonical delta",
            ));
        }
        Ok(Self {
            session_id: session.session_id().clone(),
            resolved_session_digest: session
                .resolved_session_digest()
                .expect("resolved branch checked above")
                .clone(),
            adaptation_decision_id: current.adaptation_decision.decision_id.clone(),
            adaptation_decision_digest: current
                .adaptation_decision
                .adaptation_decision_digest
                .clone(),
            verification_id: common.verification_id,
            checkpoint_id,
            canonical_delta_digest: current
                .unexpected_verification
                .canonical_delta_digest
                .clone(),
            difference_manifest_id: current
                .unexpected_verification
                .difference_manifest_id
                .clone(),
            difference_digest: current.unexpected_verification.difference_digest.clone(),
            validation_receipt_ids: common.validation_receipt_ids,
            support_audit_digest: common.support_audit_digest,
            selected_object_fingerprints: common.selected_object_fingerprints,
        })
    }
}

// -------------------------------------------------------------------------
// Apply results

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct AppliedDecisionIds(Vec<UnicaId>);

impl AppliedDecisionIds {
    fn new(values: Vec<UnicaId>) -> Result<Self, MergeResultContractError> {
        if values.len() > MAX_RESULT_ITEMS {
            return Err(MergeResultContractError(
                "applied decision list is oversized",
            ));
        }
        let mut seen = BTreeSet::new();
        if values.iter().any(|value| !seen.insert(value.as_str())) {
            return Err(MergeResultContractError(
                "applied decisions must be unique in canonical conflict order",
            ));
        }
        Ok(Self(values))
    }
}

impl JsonSchema for AppliedDecisionIds {
    fn schema_name() -> Cow<'static, str> {
        "AppliedDecisionIds".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "minItems": 0,
            "maxItems": MAX_RESULT_ITEMS,
            "uniqueItems": true,
            "items": generator.subschema_for::<UnicaId>(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct TaskMergeApplyData {
    merge_receipt_id: UnicaId,
    session_id: UnicaId,
    resolved_session_digest: Sha256Digest,
    target: TaskTarget,
    before_anchor: Sha256Digest,
    after_anchor: Sha256Digest,
    result_fingerprint: Sha256Digest,
    support_audit_digest: Sha256Digest,
    applied_decision_ids: AppliedDecisionIds,
    source_publication_id: UnicaId,
    source_fingerprint: Sha256Digest,
    task_infobase_fingerprint: Sha256Digest,
}

/// Adapter observation proving that the staged XML publication and task
/// infobase have one canonical fingerprint. The result fingerprint is derived
/// from this proof rather than accepted independently.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct TaskSourcePublicationAuthority {
    source_publication_id: UnicaId,
    canonical_fingerprint: Sha256Digest,
}

impl TaskSourcePublicationAuthority {
    pub(crate) fn from_publisher_adapter(
        source_publication_id: UnicaId,
        source_fingerprint: Sha256Digest,
        task_infobase_fingerprint: Sha256Digest,
    ) -> Result<Self, MergeResultContractError> {
        if source_fingerprint != task_infobase_fingerprint {
            return Err(MergeResultContractError(
                "task source publication and task infobase fingerprints differ",
            ));
        }
        Ok(Self {
            source_publication_id,
            canonical_fingerprint: source_fingerprint,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct OriginalMergeApplyData {
    merge_receipt_id: UnicaId,
    session_id: UnicaId,
    resolved_session_digest: Sha256Digest,
    target: OriginalTarget,
    before_anchor: Sha256Digest,
    after_anchor: Sha256Digest,
    result_fingerprint: Sha256Digest,
    repository_history_cursor: RepositoryHistoryCursor,
    support_audit_digest: Sha256Digest,
    applied_decision_ids: AppliedDecisionIds,
    rollback_checkpoint_id: UnicaId,
    integration_set_digest: Sha256Digest,
    lock_set_digest: Sha256Digest,
    support_gate_digest: Sha256Digest,
    support_gate_history_evidence_digest: Sha256Digest,
}

fn original_merge_root_before_anchor(
    projection: &ValidatedOriginalMergeLockProjection,
) -> Option<&RepositoryAnchor> {
    projection
        .plan()
        .relevant_anchors()
        .as_slice()
        .iter()
        .find(|value| value.target() == &RepositoryTargetIdentity::configuration_root())
        .map(|value| value.anchor())
}

fn original_merge_checkpoint_source_is_complete(
    projection: &ValidatedOriginalMergeLockProjection,
) -> bool {
    projection
        .current_gate_authority()
        .zip(projection.root_lock_receipt())
        .zip(projection.journaled_lock_receipts())
        .zip(projection.root_reread_capability_id())
        .zip(original_merge_root_before_anchor(projection))
        .is_some_and(|((((current_gate, root), receipts), _), root_anchor)| {
            !receipts.is_empty()
                && receipts.len() == projection.plan().lock_entries().as_slice().len()
                && receipts.first() == Some(root)
                && current_gate.support_gate_id() == projection.support_gate_id()
                && current_gate.support_gate_digest() == projection.support_gate_digest()
                && current_gate.history_evidence() == projection.support_gate_history_evidence()
                && root_anchor.history_cursor()
                    == current_gate.history_evidence().classified_through_cursor()
                && root_anchor.configuration_fingerprint() == current_gate.original_fingerprint()
        })
}

#[derive(Debug)]
struct OriginalMergeRollbackCheckpointInvocationMarker;

#[derive(Debug)]
struct OriginalMergeRollbackCheckpointInvocationCapability(
    Arc<OriginalMergeRollbackCheckpointInvocationMarker>,
);

#[derive(Debug)]
struct OriginalMergeRollbackCheckpointCompletionCapability(
    Arc<OriginalMergeRollbackCheckpointInvocationMarker>,
);

impl OriginalMergeRollbackCheckpointInvocationCapability {
    fn mint() -> Self {
        Self(Arc::new(OriginalMergeRollbackCheckpointInvocationMarker))
    }

    fn completion(&self) -> OriginalMergeRollbackCheckpointCompletionCapability {
        OriginalMergeRollbackCheckpointCompletionCapability(Arc::clone(&self.0))
    }

    fn owns_completion(
        &self,
        completion: &OriginalMergeRollbackCheckpointCompletionCapability,
    ) -> bool {
        Arc::ptr_eq(&self.0, &completion.0)
    }
}

/// Exact source presented to the checkpoint adapter. The request borrows one
/// non-Clone B1 projection; no parallel session, gate, or lock IDs are
/// accepted from the caller or adapter.
#[derive(Debug)]
pub(crate) struct OriginalMergeRollbackCheckpointRequest<'a> {
    source: &'a ValidatedOriginalMergeLockProjection,
    invocation: &'a OriginalMergeRollbackCheckpointInvocationCapability,
}

impl OriginalMergeRollbackCheckpointRequest<'_> {
    pub(crate) fn merge_session_id(&self) -> &UnicaId {
        self.source.merge_session_id()
    }

    pub(crate) fn resolved_session_digest(&self) -> &Sha256Digest {
        self.source.resolved_session_digest()
    }

    pub(crate) fn plan_id(&self) -> &UnicaId {
        self.source.plan_id()
    }

    pub(crate) fn plan_digest(&self) -> &Sha256Digest {
        self.source.plan_digest()
    }

    pub(crate) fn lock_set_id(&self) -> &UnicaId {
        self.source.lock_set_id()
    }

    pub(crate) fn lock_set_digest(&self) -> &Sha256Digest {
        self.source.lock_set_digest()
    }

    pub(crate) fn support_gate_id(&self) -> &UnicaId {
        self.current_gate().support_gate_id()
    }

    pub(crate) fn support_gate_digest(&self) -> &Sha256Digest {
        self.current_gate().support_gate_digest()
    }

    pub(crate) fn support_gate_history_evidence(&self) -> &SupportGateHistoryEvidence {
        self.current_gate().history_evidence()
    }

    pub(crate) fn current_state_revision(&self) -> &Sha256Digest {
        self.current_gate().current_state_revision()
    }

    pub(crate) fn original_fingerprint(&self) -> &Sha256Digest {
        self.current_gate().original_fingerprint()
    }

    pub(crate) fn root_before_anchor(&self) -> &RepositoryAnchor {
        original_merge_root_before_anchor(self.source)
            .expect("checkpoint source validation requires the root before-anchor")
    }

    pub(crate) fn root_reread_capability_id(&self) -> &CapabilityRowId {
        self.source
            .root_reread_capability_id()
            .expect("checkpoint source validation requires the root reread capability")
    }

    fn current_gate(
        &self,
    ) -> &crate::domain::branched_development::contracts::support::CurrentReadySupportGateAuthority
    {
        self.source
            .current_gate_authority()
            .expect("checkpoint source validation requires a production current gate")
    }

    pub(crate) fn complete(
        self,
        checkpoint_id: UnicaId,
        checkpoint_fingerprint: Sha256Digest,
        root_before_anchor: RepositoryAnchor,
        observed_current_state_revision: Sha256Digest,
        checkpoint_capability_id: CapabilityRowId,
    ) -> OriginalMergeRollbackCheckpointCompletion {
        OriginalMergeRollbackCheckpointCompletion {
            completion: self.invocation.completion(),
            checkpoint_id,
            checkpoint_fingerprint,
            root_before_anchor,
            observed_current_state_revision,
            checkpoint_capability_id,
        }
    }
}

#[derive(Debug)]
pub(crate) struct OriginalMergeRollbackCheckpointCompletion {
    completion: OriginalMergeRollbackCheckpointCompletionCapability,
    checkpoint_id: UnicaId,
    checkpoint_fingerprint: Sha256Digest,
    root_before_anchor: RepositoryAnchor,
    observed_current_state_revision: Sha256Digest,
    checkpoint_capability_id: CapabilityRowId,
}

pub(crate) trait OriginalMergeRollbackCheckpointPort {
    fn create_and_verify_original_rollback_checkpoint(
        &mut self,
        request: OriginalMergeRollbackCheckpointRequest<'_>,
    ) -> Result<OriginalMergeRollbackCheckpointCompletion, MergeResultContractError>;
}

#[derive(Debug)]
pub(crate) enum OriginalMergeRollbackCheckpointFailureEvidence {
    SourceLineageMismatch,
    PortError(MergeResultContractError),
    CompletionAttemptMismatch(OriginalMergeRollbackCheckpointCompletion),
    ObservationMismatch(OriginalMergeRollbackCheckpointCompletion),
}

/// Recovery-only checkpoint failure. It always owns the exact B1 source which
/// the adapter was asked to checkpoint.
#[derive(Debug)]
pub(crate) struct OriginalMergeRollbackCheckpointBlockedAuthority {
    source: ValidatedOriginalMergeLockProjection,
    failure: OriginalMergeRollbackCheckpointFailureEvidence,
}

impl OriginalMergeRollbackCheckpointBlockedAuthority {
    fn new(
        source: ValidatedOriginalMergeLockProjection,
        failure: OriginalMergeRollbackCheckpointFailureEvidence,
    ) -> Box<Self> {
        Box::new(Self { source, failure })
    }

    pub(crate) fn failure(&self) -> &OriginalMergeRollbackCheckpointFailureEvidence {
        &self.failure
    }

    pub(crate) fn into_recovery_parts(
        self: Box<Self>,
    ) -> (
        ValidatedOriginalMergeLockProjection,
        OriginalMergeRollbackCheckpointFailureEvidence,
    ) {
        let Self { source, failure } = *self;
        (source, failure)
    }
}

/// Capability-proven rollback source created and verified for one exact B1
/// projection before authoritative mutation. It owns that projection, so a
/// checkpoint can never be paired later with a field-equal or newer source.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct VerifiedOriginalRollbackCheckpointAuthority {
    source: ValidatedOriginalMergeLockProjection,
    checkpoint_id: UnicaId,
    checkpoint_fingerprint: Sha256Digest,
    root_before_anchor: RepositoryAnchor,
    observed_current_state_revision: Sha256Digest,
    checkpoint_capability_id: CapabilityRowId,
}

impl VerifiedOriginalRollbackCheckpointAuthority {
    pub(crate) fn create(
        source: ValidatedOriginalMergeLockProjection,
        port: &mut dyn OriginalMergeRollbackCheckpointPort,
    ) -> Result<Self, Box<OriginalMergeRollbackCheckpointBlockedAuthority>> {
        if !original_merge_checkpoint_source_is_complete(&source) {
            return Err(OriginalMergeRollbackCheckpointBlockedAuthority::new(
                source,
                OriginalMergeRollbackCheckpointFailureEvidence::SourceLineageMismatch,
            ));
        }
        let invocation = OriginalMergeRollbackCheckpointInvocationCapability::mint();
        let request = OriginalMergeRollbackCheckpointRequest {
            source: &source,
            invocation: &invocation,
        };
        let completion = match port.create_and_verify_original_rollback_checkpoint(request) {
            Ok(completion) => completion,
            Err(error) => {
                return Err(OriginalMergeRollbackCheckpointBlockedAuthority::new(
                    source,
                    OriginalMergeRollbackCheckpointFailureEvidence::PortError(error),
                ));
            }
        };
        if !invocation.owns_completion(&completion.completion) {
            return Err(OriginalMergeRollbackCheckpointBlockedAuthority::new(
                source,
                OriginalMergeRollbackCheckpointFailureEvidence::CompletionAttemptMismatch(
                    completion,
                ),
            ));
        }
        let request = OriginalMergeRollbackCheckpointRequest {
            source: &source,
            invocation: &invocation,
        };
        if completion.checkpoint_fingerprint != *request.original_fingerprint()
            || completion.root_before_anchor != *request.root_before_anchor()
            || completion.observed_current_state_revision != *request.current_state_revision()
        {
            return Err(OriginalMergeRollbackCheckpointBlockedAuthority::new(
                source,
                OriginalMergeRollbackCheckpointFailureEvidence::ObservationMismatch(completion),
            ));
        }
        let OriginalMergeRollbackCheckpointCompletion {
            completion: _,
            checkpoint_id,
            checkpoint_fingerprint,
            root_before_anchor,
            observed_current_state_revision,
            checkpoint_capability_id,
        } = completion;
        Ok(Self {
            source,
            checkpoint_id,
            checkpoint_fingerprint,
            root_before_anchor,
            observed_current_state_revision,
            checkpoint_capability_id,
        })
    }

    fn lock_projection(&self) -> &ValidatedOriginalMergeLockProjection {
        &self.source
    }

    pub(crate) fn checkpoint_id(&self) -> &UnicaId {
        &self.checkpoint_id
    }
}

/// Exact read-only context presented to the repository adapter immediately
/// before the original merge intent. Every value is borrowed from the owned
/// B1 projection, resolved session, decisions, or rollback checkpoint; callers
/// cannot supply a parallel scalar context.
#[derive(Debug)]
pub(crate) struct OriginalMergePreIntentRequest<'a> {
    authority: &'a OriginalMergePreIntentAttemptAuthority,
    invocation: &'a OriginalMergePreIntentInvocationCapability,
}

#[derive(Debug)]
struct OriginalMergePreIntentInvocationMarker;

#[derive(Debug)]
struct OriginalMergePreIntentInvocationCapability(Arc<OriginalMergePreIntentInvocationMarker>);

#[derive(Debug)]
struct OriginalMergePreIntentCompletionCapability(Arc<OriginalMergePreIntentInvocationMarker>);

impl OriginalMergePreIntentInvocationCapability {
    fn mint() -> Self {
        Self(Arc::new(OriginalMergePreIntentInvocationMarker))
    }

    fn completion(&self) -> OriginalMergePreIntentCompletionCapability {
        OriginalMergePreIntentCompletionCapability(Arc::clone(&self.0))
    }

    fn owns_completion(&self, completion: &OriginalMergePreIntentCompletionCapability) -> bool {
        Arc::ptr_eq(&self.0, &completion.0)
    }
}

impl OriginalMergePreIntentRequest<'_> {
    pub(crate) fn plan(&self) -> &LockPlanData {
        self.authority.rollback.lock_projection().plan()
    }

    pub(crate) fn plan_id(&self) -> &UnicaId {
        self.authority.rollback.lock_projection().plan_id()
    }

    pub(crate) fn plan_digest(&self) -> &Sha256Digest {
        self.authority.rollback.lock_projection().plan_digest()
    }

    pub(crate) fn plan_support_gate_id(&self) -> &UnicaId {
        self.authority.rollback.lock_projection().support_gate_id()
    }

    pub(crate) fn support_gate_id(&self) -> &UnicaId {
        self.current_gate().support_gate_id()
    }

    pub(crate) fn support_gate_digest(&self) -> &Sha256Digest {
        self.current_gate().support_gate_digest()
    }

    pub(crate) fn support_gate_history_evidence(&self) -> &SupportGateHistoryEvidence {
        self.current_gate().history_evidence()
    }

    pub(crate) fn current_state_revision(&self) -> &Sha256Digest {
        self.current_gate().current_state_revision()
    }

    pub(crate) fn lock_projection_current_state_revision(&self) -> &Sha256Digest {
        self.current_state_revision()
    }

    pub(crate) fn session_id(&self) -> &UnicaId {
        self.authority.rollback.lock_projection().merge_session_id()
    }

    pub(crate) fn resolved_session_digest(&self) -> &Sha256Digest {
        self.authority
            .rollback
            .lock_projection()
            .resolved_session_digest()
    }

    pub(crate) fn integration_set_id(&self) -> &UnicaId {
        self.authority
            .rollback
            .lock_projection()
            .integration_set_id()
    }

    pub(crate) fn integration_set_digest(&self) -> &Sha256Digest {
        self.authority
            .rollback
            .lock_projection()
            .integration_set_digest()
    }

    pub(crate) fn integration_entries(&self) -> &RepositoryIntegrationEntries {
        self.plan().integration_entries()
    }

    pub(crate) fn lock_set_id(&self) -> &UnicaId {
        self.authority.rollback.lock_projection().lock_set_id()
    }

    pub(crate) fn lock_set_digest(&self) -> &Sha256Digest {
        self.authority.rollback.lock_projection().lock_set_digest()
    }

    pub(crate) fn planned_locks(&self) -> &RepositoryUpdateLockTargets {
        self.plan().lock_entries()
    }

    pub(crate) fn planned_lock_count(&self) -> usize {
        self.planned_locks().as_slice().len()
    }

    pub(crate) fn root_lock_receipt(&self) -> &JournaledRepositoryLock {
        self.authority
            .rollback
            .lock_projection()
            .root_lock_receipt()
            .expect("pre-intent rejects non-production B1 projections")
    }

    pub(crate) fn journaled_lock_receipts(&self) -> &[JournaledRepositoryLock] {
        self.authority
            .rollback
            .lock_projection()
            .journaled_lock_receipts()
            .expect("pre-intent rejects non-production B1 projections")
    }

    pub(crate) fn root_reread_capability_id(&self) -> &CapabilityRowId {
        self.authority
            .rollback
            .lock_projection()
            .root_reread_capability_id()
            .expect("pre-intent rejects non-production B1 projections")
    }

    pub(crate) fn relevant_anchors(&self) -> &RepositoryRelevantAnchors {
        self.plan().relevant_anchors()
    }

    pub(crate) fn support_graph_digest(&self) -> &Sha256Digest {
        self.current_gate().support_graph_digest()
    }

    pub(crate) fn original_fingerprint(&self) -> &Sha256Digest {
        self.current_gate().original_fingerprint()
    }

    pub(crate) fn reference_closure_digest(&self) -> &Sha256Digest {
        self.plan().reference_closure_digest()
    }

    pub(crate) fn settings_digest(&self) -> &Sha256Digest {
        self.plan().settings_digest()
    }

    pub(crate) fn verification_id(&self) -> &UnicaId {
        self.plan().verification_id()
    }

    pub(crate) fn verification_digest(&self) -> &Sha256Digest {
        self.plan().verification_digest()
    }

    pub(crate) fn prevalidation_diagnostics_digest(&self) -> &Sha256Digest {
        self.plan().prevalidation_diagnostics_digest()
    }

    pub(crate) fn applied_decision_ids(&self) -> &[UnicaId] {
        &self.authority.decision_projection.applied_decision_ids.0
    }

    pub(crate) fn rollback_checkpoint_id(&self) -> &UnicaId {
        self.authority.rollback.checkpoint_id()
    }

    pub(crate) fn rollback_checkpoint_capability_id(&self) -> &CapabilityRowId {
        &self.authority.rollback.checkpoint_capability_id
    }

    fn current_gate(
        &self,
    ) -> &crate::domain::branched_development::contracts::support::CurrentReadySupportGateAuthority
    {
        self.authority
            .rollback
            .lock_projection()
            .current_gate_authority()
            .expect("pre-intent rejects non-production B1 projections")
    }

    pub(crate) fn complete(
        self,
        lease: Box<dyn OriginalMergePreIntentLease>,
    ) -> OriginalMergePreIntentCompletion {
        OriginalMergePreIntentCompletion {
            completion: self.invocation.completion(),
            lease,
        }
    }
}

/// Trusted temporal lease returned by the repository adapter after it rereads
/// the exact request immediately before intent.
pub(crate) trait OriginalMergePreIntentLease {
    fn binds(&self, request: &OriginalMergePreIntentRequest<'_>) -> bool;
    fn preintent_capability_id(&self) -> &CapabilityRowId;
}

pub(crate) trait OriginalMergePreIntentPort {
    fn reread_immediately_before_original_merge(
        &mut self,
        request: OriginalMergePreIntentRequest<'_>,
    ) -> Result<OriginalMergePreIntentCompletion, MergeResultContractError>;
}

pub(crate) struct OriginalMergePreIntentCompletion {
    completion: OriginalMergePreIntentCompletionCapability,
    lease: Box<dyn OriginalMergePreIntentLease>,
}

#[derive(Debug, PartialEq, Eq)]
struct OriginalMergePreIntentAttemptAuthority {
    session: MergeSessionData,
    decision_projection: ResolvedApplyDecisionProjectionAuthority,
    rollback: VerifiedOriginalRollbackCheckpointAuthority,
}

/// Stop/recovery evidence which deliberately retains the complete locked gate
/// attempt when the immediate reread fails or does not bind.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct OriginalMergePreIntentBlockedAuthority {
    attempt: OriginalMergePreIntentAttemptAuthority,
}

impl OriginalMergePreIntentBlockedAuthority {
    pub(crate) fn plan_id(&self) -> &UnicaId {
        self.attempt.rollback.lock_projection().plan_id()
    }

    pub(crate) fn lock_set_id(&self) -> &UnicaId {
        self.attempt.rollback.lock_projection().lock_set_id()
    }
}

/// Linear, non-wire proof of the final current-gate reread. It owns the full
/// B1 lock/gate proof, resolved decisions, and rollback checkpoint until the
/// typed effect executor consumes it.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct OriginalMergePreIntentAuthority {
    attempt: OriginalMergePreIntentAttemptAuthority,
    preintent_capability_id: CapabilityRowId,
}

impl OriginalMergePreIntentAuthority {
    pub(crate) fn recheck(
        session: &MergeSessionData,
        decision_projection: ResolvedApplyDecisionProjectionAuthority,
        rollback: VerifiedOriginalRollbackCheckpointAuthority,
        port: &mut dyn OriginalMergePreIntentPort,
    ) -> Result<Self, Box<OriginalMergePreIntentBlockedAuthority>> {
        let attempt = OriginalMergePreIntentAttemptAuthority {
            session: session.clone(),
            decision_projection,
            rollback,
        };
        let MergeSessionData::MainIntegration(main) = &attempt.session else {
            return Err(Box::new(OriginalMergePreIntentBlockedAuthority { attempt }));
        };
        let projection = attempt.rollback.lock_projection();
        let has_complete_production_lock_lineage =
            original_merge_checkpoint_source_is_complete(projection);
        let rollback_still_binds_source = projection
            .current_gate_authority()
            .zip(original_merge_root_before_anchor(projection))
            .is_some_and(|(current_gate, root_anchor)| {
                attempt.rollback.checkpoint_fingerprint == *current_gate.original_fingerprint()
                    && attempt.rollback.root_before_anchor == *root_anchor
                    && attempt.rollback.observed_current_state_revision
                        == *current_gate.current_state_revision()
            });
        let lineage_matches = attempt.decision_projection.session_id == main.session_id
            && attempt.decision_projection.resolved_session_digest == main.resolved_session_digest
            && attempt
                .session
                .validates_resolved_projection(&attempt.decision_projection.applied_decision_ids.0)
                .unwrap_or(false)
            && projection.merge_session_id() == &main.session_id
            && projection.resolved_session_digest() == &main.resolved_session_digest
            && projection.support_gate_id() == &main.support_gate_id
            && projection.support_gate_digest() == &main.support_gate_digest
            && projection.support_gate_history_evidence().evidence_digest()
                == &main.support_gate_history_evidence_digest;
        if !has_complete_production_lock_lineage || !rollback_still_binds_source || !lineage_matches
        {
            return Err(Box::new(OriginalMergePreIntentBlockedAuthority { attempt }));
        }
        let invocation = OriginalMergePreIntentInvocationCapability::mint();
        let request = OriginalMergePreIntentRequest {
            authority: &attempt,
            invocation: &invocation,
        };
        let completed = match port.reread_immediately_before_original_merge(request) {
            Ok(completed) => completed,
            Err(_) => {
                return Err(Box::new(OriginalMergePreIntentBlockedAuthority { attempt }));
            }
        };
        if !invocation.owns_completion(&completed.completion) {
            return Err(Box::new(OriginalMergePreIntentBlockedAuthority { attempt }));
        }
        let request = OriginalMergePreIntentRequest {
            authority: &attempt,
            invocation: &invocation,
        };
        let lease = completed.lease;
        if !lease.binds(&request) {
            return Err(Box::new(OriginalMergePreIntentBlockedAuthority { attempt }));
        }
        let preintent_capability_id = lease.preintent_capability_id().clone();
        Ok(Self {
            attempt,
            preintent_capability_id,
        })
    }

    fn lock_projection(&self) -> &ValidatedOriginalMergeLockProjection {
        self.attempt.rollback.lock_projection()
    }
}

/// Typed execution request. There is no production constructor accepting a
/// raw effect observation; the only live route is through this port.
#[derive(Debug)]
pub(crate) struct OriginalMergeExecutionRequest<'a> {
    preintent: &'a OriginalMergePreIntentAuthority,
    merge_receipt_id: &'a UnicaId,
    attempt: &'a OriginalMergeExecutionAttemptCapability,
}

#[derive(Debug)]
struct OriginalMergeExecutionAttemptMarker;

#[derive(Debug)]
struct OriginalMergeExecutionAttemptCapability(Arc<OriginalMergeExecutionAttemptMarker>);

#[derive(Debug)]
struct OriginalMergeExecutionCompletionCapability(Arc<OriginalMergeExecutionAttemptMarker>);

impl OriginalMergeExecutionAttemptCapability {
    fn mint() -> Self {
        Self(Arc::new(OriginalMergeExecutionAttemptMarker))
    }

    fn completion(&self) -> OriginalMergeExecutionCompletionCapability {
        OriginalMergeExecutionCompletionCapability(Arc::clone(&self.0))
    }

    fn owns_completion(&self, completion: &OriginalMergeExecutionCompletionCapability) -> bool {
        Arc::ptr_eq(&self.0, &completion.0)
    }
}

impl OriginalMergeExecutionRequest<'_> {
    pub(crate) fn merge_receipt_id(&self) -> &UnicaId {
        self.merge_receipt_id
    }

    pub(crate) fn plan_id(&self) -> &UnicaId {
        self.preintent.lock_projection().plan_id()
    }

    pub(crate) fn preintent_plan_id(&self) -> &UnicaId {
        self.plan_id()
    }

    pub(crate) fn session_id(&self) -> &UnicaId {
        self.preintent.lock_projection().merge_session_id()
    }

    pub(crate) fn resolved_session_digest(&self) -> &Sha256Digest {
        self.preintent.lock_projection().resolved_session_digest()
    }

    pub(crate) fn rollback_checkpoint_id(&self) -> &UnicaId {
        self.preintent.attempt.rollback.checkpoint_id()
    }

    /// Attests raw adapter output to this exact invocation. A previously
    /// completed request cannot mint an observation for a later invocation.
    pub(crate) fn complete(
        self,
        before_anchor: Sha256Digest,
        after_anchor: Sha256Digest,
        result_fingerprint: Sha256Digest,
        support_audit_digest: Sha256Digest,
    ) -> OriginalMergeEffectObservationAuthority {
        OriginalMergeEffectObservationAuthority {
            completion: self.attempt.completion(),
            before_anchor,
            after_anchor,
            result_fingerprint,
            support_audit_digest,
        }
    }
}

pub(crate) trait OriginalMergeExecutionPort {
    fn execute_original_merge(
        &mut self,
        request: OriginalMergeExecutionRequest<'_>,
    ) -> Result<OriginalMergeExecutionPortOutcome, MergeResultContractError>;
}

/// Post-effect observation from the original-merge executor. It deliberately
/// does not carry gate/plan/lock lineage; those values come only from the
/// consumed validated lock projection.
#[derive(Debug)]
pub(crate) struct OriginalMergeEffectObservationAuthority {
    completion: OriginalMergeExecutionCompletionCapability,
    before_anchor: Sha256Digest,
    after_anchor: Sha256Digest,
    result_fingerprint: Sha256Digest,
    support_audit_digest: Sha256Digest,
}

impl OriginalMergeEffectObservationAuthority {
    #[cfg(test)]
    fn fixture_test_only(
        before_anchor: Sha256Digest,
        after_anchor: Sha256Digest,
        result_fingerprint: Sha256Digest,
        support_audit_digest: Sha256Digest,
    ) -> Self {
        let attempt = OriginalMergeExecutionAttemptCapability::mint();
        Self {
            completion: attempt.completion(),
            before_anchor,
            after_anchor,
            result_fingerprint,
            support_audit_digest,
        }
    }
}

/// The executor reached the repository adapter, but the adapter cannot prove
/// whether the original-configuration effect happened. This is deliberately
/// distinct from a successful effect observation.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct OriginalMergeExecutionUnknownObservationAuthority;

impl OriginalMergeExecutionUnknownObservationAuthority {
    pub(crate) fn from_repository_adapter() -> Self {
        Self
    }
}

/// Typed boundary result. Transport/adapter errors remain the outer `Err`;
/// an explicitly unknown repository outcome is represented in the successful
/// transport response and still cannot become a pending receipt.
#[derive(Debug)]
pub(crate) enum OriginalMergeExecutionPortOutcome {
    Observed(OriginalMergeEffectObservationAuthority),
    Unknown(OriginalMergeExecutionUnknownObservationAuthority),
}

#[derive(Debug)]
enum OriginalMergeExecutionBlockedReason {
    PortError(MergeResultContractError),
    OutcomeUnknown(OriginalMergeExecutionUnknownObservationAuthority),
    CompletionAttemptMismatch(OriginalMergeEffectObservationAuthority),
}

/// Stop/recovery authority for a failed or unknown effect-port call. It owns
/// the exact receipt candidate and the complete pre-intent attempt, including
/// the B1 receipts, current gate, decisions, and rollback checkpoint.
#[derive(Debug)]
pub(crate) struct OriginalMergeExecutionBlockedAuthority {
    preintent: OriginalMergePreIntentAuthority,
    merge_receipt_id: UnicaId,
    reason: OriginalMergeExecutionBlockedReason,
}

impl OriginalMergeExecutionBlockedAuthority {
    pub(crate) fn is_port_error(&self) -> bool {
        matches!(
            self.reason,
            OriginalMergeExecutionBlockedReason::PortError(_)
        )
    }

    pub(crate) fn is_outcome_unknown(&self) -> bool {
        matches!(
            self.reason,
            OriginalMergeExecutionBlockedReason::OutcomeUnknown(_)
        )
    }

    pub(crate) fn is_completion_attempt_mismatch(&self) -> bool {
        matches!(
            self.reason,
            OriginalMergeExecutionBlockedReason::CompletionAttemptMismatch(_)
        )
    }

    pub(crate) fn merge_receipt_id(&self) -> &UnicaId {
        &self.merge_receipt_id
    }

    pub(crate) fn plan_id(&self) -> &UnicaId {
        self.preintent.lock_projection().plan_id()
    }

    pub(crate) fn lock_set_id(&self) -> &UnicaId {
        self.preintent.lock_projection().lock_set_id()
    }

    pub(crate) fn rollback_checkpoint_id(&self) -> &UnicaId {
        self.preintent.attempt.rollback.checkpoint_id()
    }

    pub(crate) fn root_lock_receipt(&self) -> &JournaledRepositoryLock {
        self.preintent
            .lock_projection()
            .root_lock_receipt()
            .expect("execution-blocked authority retains production B1 lineage")
    }

    pub(crate) fn journaled_lock_receipts(&self) -> &[JournaledRepositoryLock] {
        self.preintent
            .lock_projection()
            .journaled_lock_receipts()
            .expect("execution-blocked authority retains production B1 lineage")
    }
}

/// Effect evidence which has not yet been atomically journaled together with
/// the support-gate transition. It is recovery evidence, not a successful
/// original-merge receipt.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PendingOriginalMergeReceiptAuthority {
    data: OriginalMergeApplyData,
    preintent: OriginalMergePreIntentAuthority,
}

impl PendingOriginalMergeReceiptAuthority {
    pub(crate) fn execute(
        preintent: OriginalMergePreIntentAuthority,
        merge_receipt_id: UnicaId,
        port: &mut dyn OriginalMergeExecutionPort,
    ) -> Result<Self, Box<OriginalMergeExecutionBlockedAuthority>> {
        let attempt = OriginalMergeExecutionAttemptCapability::mint();
        let request = OriginalMergeExecutionRequest {
            preintent: &preintent,
            merge_receipt_id: &merge_receipt_id,
            attempt: &attempt,
        };
        let observation = match port.execute_original_merge(request) {
            Ok(OriginalMergeExecutionPortOutcome::Observed(observation)) => observation,
            Ok(OriginalMergeExecutionPortOutcome::Unknown(unknown)) => {
                return Err(Box::new(OriginalMergeExecutionBlockedAuthority {
                    preintent,
                    merge_receipt_id,
                    reason: OriginalMergeExecutionBlockedReason::OutcomeUnknown(unknown),
                }));
            }
            Err(error) => {
                return Err(Box::new(OriginalMergeExecutionBlockedAuthority {
                    preintent,
                    merge_receipt_id,
                    reason: OriginalMergeExecutionBlockedReason::PortError(error),
                }));
            }
        };
        if !attempt.owns_completion(&observation.completion) {
            return Err(Box::new(OriginalMergeExecutionBlockedAuthority {
                preintent,
                merge_receipt_id,
                reason: OriginalMergeExecutionBlockedReason::CompletionAttemptMismatch(observation),
            }));
        }
        let OriginalMergeEffectObservationAuthority {
            completion: _,
            before_anchor,
            after_anchor,
            result_fingerprint,
            support_audit_digest,
        } = observation;
        let projection = preintent.lock_projection();
        let history = projection.support_gate_history_evidence();
        let data = OriginalMergeApplyData {
            merge_receipt_id,
            session_id: preintent.attempt.decision_projection.session_id.clone(),
            resolved_session_digest: preintent
                .attempt
                .decision_projection
                .resolved_session_digest
                .clone(),
            target: OriginalTarget::Value,
            before_anchor,
            after_anchor,
            result_fingerprint,
            repository_history_cursor: history.classified_through_cursor().clone(),
            support_audit_digest,
            applied_decision_ids: preintent
                .attempt
                .decision_projection
                .applied_decision_ids
                .clone(),
            rollback_checkpoint_id: preintent.attempt.rollback.checkpoint_id().clone(),
            integration_set_digest: projection.integration_set_digest().clone(),
            lock_set_digest: projection.lock_set_digest().clone(),
            support_gate_digest: projection.support_gate_digest().clone(),
            support_gate_history_evidence_digest: history.evidence_digest().clone(),
        };
        Ok(Self { data, preintent })
    }

    pub(crate) fn merge_receipt_id(&self) -> &UnicaId {
        &self.data.merge_receipt_id
    }

    pub(crate) fn result_fingerprint(&self) -> &Sha256Digest {
        &self.data.result_fingerprint
    }

    pub(crate) fn repository_history_cursor(&self) -> &RepositoryHistoryCursor {
        &self.data.repository_history_cursor
    }

    pub(crate) fn plan_id(&self) -> &UnicaId {
        self.preintent.lock_projection().plan_id()
    }

    pub(crate) fn plan_digest(&self) -> &Sha256Digest {
        self.preintent.lock_projection().plan_digest()
    }
}

/// Exact CAS preimage derived from the pending effect and owned temporal
/// lineage. It is deliberately non-wire and has no caller-facing constructor.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct SupportGateOriginalMergeCasBinding {
    current_state_revision: Sha256Digest,
    support_gate_id: UnicaId,
    support_gate_digest: Sha256Digest,
    support_gate_history_evidence_digest: Sha256Digest,
    session_id: UnicaId,
    resolved_session_digest: Sha256Digest,
    plan_id: UnicaId,
    plan_digest: Sha256Digest,
    integration_set_id: UnicaId,
    integration_set_digest: Sha256Digest,
    lock_set_id: UnicaId,
    lock_set_digest: Sha256Digest,
    root_lock_receipt: JournaledRepositoryLock,
    ordered_retained_lock_receipts: Vec<JournaledRepositoryLock>,
    missing_lock_receipt_count: usize,
    merge_receipt_id: UnicaId,
    merge_receipt_cursor: RepositoryHistoryCursor,
    result_fingerprint: Sha256Digest,
    rollback_checkpoint_id: UnicaId,
    before_anchor: Sha256Digest,
    after_anchor: Sha256Digest,
    support_audit_digest: Sha256Digest,
    preintent_capability_id: CapabilityRowId,
    root_reread_capability_id: CapabilityRowId,
    support_graph_digest: Sha256Digest,
    original_fingerprint: Sha256Digest,
    reference_closure_digest: Sha256Digest,
    applied_decision_ids: Vec<UnicaId>,
}

impl SupportGateOriginalMergeCasBinding {
    fn from_pending(pending: &PendingOriginalMergeReceiptAuthority) -> Self {
        let projection = pending.preintent.lock_projection();
        let current_gate = projection
            .current_gate_authority()
            .expect("pending receipt can only come from a production pre-intent");
        let ordered_retained_lock_receipts = projection
            .journaled_lock_receipts()
            .expect("pending receipt preserves the complete production B1 journal")
            .to_vec();
        Self {
            current_state_revision: current_gate.current_state_revision().clone(),
            support_gate_id: projection.support_gate_id().clone(),
            support_gate_digest: projection.support_gate_digest().clone(),
            support_gate_history_evidence_digest: projection
                .support_gate_history_evidence()
                .evidence_digest()
                .clone(),
            session_id: pending.data.session_id.clone(),
            resolved_session_digest: pending.data.resolved_session_digest.clone(),
            plan_id: projection.plan_id().clone(),
            plan_digest: projection.plan_digest().clone(),
            integration_set_id: projection.integration_set_id().clone(),
            integration_set_digest: projection.integration_set_digest().clone(),
            lock_set_id: projection.lock_set_id().clone(),
            lock_set_digest: projection.lock_set_digest().clone(),
            root_lock_receipt: projection
                .root_lock_receipt()
                .expect("pending receipt preserves the production B1 root receipt")
                .clone(),
            missing_lock_receipt_count: projection
                .plan()
                .lock_entries()
                .as_slice()
                .len()
                .saturating_sub(ordered_retained_lock_receipts.len()),
            ordered_retained_lock_receipts,
            merge_receipt_id: pending.data.merge_receipt_id.clone(),
            merge_receipt_cursor: pending.data.repository_history_cursor.clone(),
            result_fingerprint: pending.data.result_fingerprint.clone(),
            rollback_checkpoint_id: pending.data.rollback_checkpoint_id.clone(),
            before_anchor: pending.data.before_anchor.clone(),
            after_anchor: pending.data.after_anchor.clone(),
            support_audit_digest: pending.data.support_audit_digest.clone(),
            preintent_capability_id: pending.preintent.preintent_capability_id.clone(),
            root_reread_capability_id: projection
                .root_reread_capability_id()
                .expect("pending receipt preserves the production root reread")
                .clone(),
            support_graph_digest: current_gate.support_graph_digest().clone(),
            original_fingerprint: current_gate.original_fingerprint().clone(),
            reference_closure_digest: projection.plan().reference_closure_digest().clone(),
            applied_decision_ids: pending.data.applied_decision_ids.0.clone(),
        }
    }

    pub(crate) fn current_state_revision(&self) -> &Sha256Digest {
        &self.current_state_revision
    }

    pub(crate) fn support_gate_id(&self) -> &UnicaId {
        &self.support_gate_id
    }

    pub(crate) fn support_gate_digest(&self) -> &Sha256Digest {
        &self.support_gate_digest
    }

    pub(crate) fn support_gate_history_evidence_digest(&self) -> &Sha256Digest {
        &self.support_gate_history_evidence_digest
    }

    pub(crate) fn session_id(&self) -> &UnicaId {
        &self.session_id
    }

    pub(crate) fn resolved_session_digest(&self) -> &Sha256Digest {
        &self.resolved_session_digest
    }

    pub(crate) fn plan_id(&self) -> &UnicaId {
        &self.plan_id
    }

    pub(crate) fn plan_digest(&self) -> &Sha256Digest {
        &self.plan_digest
    }

    pub(crate) fn integration_set_id(&self) -> &UnicaId {
        &self.integration_set_id
    }

    pub(crate) fn integration_set_digest(&self) -> &Sha256Digest {
        &self.integration_set_digest
    }

    pub(crate) fn lock_set_id(&self) -> &UnicaId {
        &self.lock_set_id
    }

    pub(crate) fn lock_set_digest(&self) -> &Sha256Digest {
        &self.lock_set_digest
    }

    pub(crate) fn root_lock_receipt(&self) -> &JournaledRepositoryLock {
        &self.root_lock_receipt
    }

    pub(crate) fn journaled_lock_receipts(&self) -> &[JournaledRepositoryLock] {
        &self.ordered_retained_lock_receipts
    }

    pub(crate) fn missing_lock_receipt_count(&self) -> usize {
        self.missing_lock_receipt_count
    }

    pub(crate) fn merge_receipt_id(&self) -> &UnicaId {
        &self.merge_receipt_id
    }

    pub(crate) fn merge_receipt_cursor(&self) -> &RepositoryHistoryCursor {
        &self.merge_receipt_cursor
    }

    pub(crate) fn result_fingerprint(&self) -> &Sha256Digest {
        &self.result_fingerprint
    }

    pub(crate) fn rollback_checkpoint_id(&self) -> &UnicaId {
        &self.rollback_checkpoint_id
    }

    pub(crate) fn before_anchor(&self) -> &Sha256Digest {
        &self.before_anchor
    }

    pub(crate) fn after_anchor(&self) -> &Sha256Digest {
        &self.after_anchor
    }

    pub(crate) fn support_audit_digest(&self) -> &Sha256Digest {
        &self.support_audit_digest
    }

    pub(crate) fn preintent_capability_id(&self) -> &CapabilityRowId {
        &self.preintent_capability_id
    }

    pub(crate) fn root_reread_capability_id(&self) -> &CapabilityRowId {
        &self.root_reread_capability_id
    }

    pub(crate) fn support_graph_digest(&self) -> &Sha256Digest {
        &self.support_graph_digest
    }

    pub(crate) fn original_fingerprint(&self) -> &Sha256Digest {
        &self.original_fingerprint
    }

    pub(crate) fn reference_closure_digest(&self) -> &Sha256Digest {
        &self.reference_closure_digest
    }

    pub(crate) fn applied_decision_ids(&self) -> &[UnicaId] {
        &self.applied_decision_ids
    }
}

/// One atomic storage operation. Implementations persist the pending receipt
/// and `current -> consumedByOriginalMerge` transition together or return no
/// success. The boxed lease and `self` receiver make commit one-shot.
pub(crate) trait SupportGateOriginalMergeCasLease {
    fn binds(&self, binding: &SupportGateOriginalMergeCasBinding) -> bool;

    fn commit_receipt_and_consume_gate(
        self: Box<Self>,
        pending: &PendingOriginalMergeReceiptAuthority,
    ) -> Result<(), MergeResultContractError>;
}

#[derive(Debug)]
struct SupportGateOriginalMergeCasInvocationMarker;

#[derive(Debug)]
struct SupportGateOriginalMergeCasInvocationCapability(
    Arc<SupportGateOriginalMergeCasInvocationMarker>,
);

#[derive(Debug)]
struct SupportGateOriginalMergeCasCompletionCapability(
    Arc<SupportGateOriginalMergeCasInvocationMarker>,
);

impl SupportGateOriginalMergeCasInvocationCapability {
    fn mint() -> Self {
        Self(Arc::new(SupportGateOriginalMergeCasInvocationMarker))
    }

    fn completion(&self) -> SupportGateOriginalMergeCasCompletionCapability {
        SupportGateOriginalMergeCasCompletionCapability(Arc::clone(&self.0))
    }

    fn owns_completion(
        &self,
        completion: &SupportGateOriginalMergeCasCompletionCapability,
    ) -> bool {
        Arc::ptr_eq(&self.0, &completion.0)
    }
}

#[derive(Debug)]
pub(crate) struct SupportGateOriginalMergeCasRequest<'a> {
    binding: &'a SupportGateOriginalMergeCasBinding,
    invocation: &'a SupportGateOriginalMergeCasInvocationCapability,
}

impl SupportGateOriginalMergeCasRequest<'_> {
    pub(crate) fn binding(&self) -> &SupportGateOriginalMergeCasBinding {
        self.binding
    }

    pub(crate) fn complete(
        self,
        lease: Box<dyn SupportGateOriginalMergeCasLease>,
    ) -> SupportGateOriginalMergeCasResolution {
        SupportGateOriginalMergeCasResolution {
            completion: self.invocation.completion(),
            lease,
        }
    }
}

pub(crate) struct SupportGateOriginalMergeCasResolution {
    completion: SupportGateOriginalMergeCasCompletionCapability,
    lease: Box<dyn SupportGateOriginalMergeCasLease>,
}

pub(crate) trait SupportGateOriginalMergeCasResolver {
    fn resolve_original_merge_cas(
        &mut self,
        request: SupportGateOriginalMergeCasRequest<'_>,
    ) -> Result<SupportGateOriginalMergeCasResolution, MergeResultContractError>;
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ConsumedSupportGateAuthority {
    binding: SupportGateOriginalMergeCasBinding,
    preintent: OriginalMergePreIntentAuthority,
}

impl ConsumedSupportGateAuthority {
    pub(crate) fn authorized_result_fingerprint(&self) -> &Sha256Digest {
        &self.binding.result_fingerprint
    }

    pub(crate) fn support_gate_id(&self) -> &UnicaId {
        &self.binding.support_gate_id
    }

    pub(crate) fn support_gate_digest(&self) -> &Sha256Digest {
        &self.binding.support_gate_digest
    }

    pub(crate) fn current_state_revision(&self) -> &Sha256Digest {
        &self.binding.current_state_revision
    }

    fn lock_projection(&self) -> &ValidatedOriginalMergeLockProjection {
        self.preintent.lock_projection()
    }
}

pub(crate) struct ValidatedSupportGateOriginalMergeCasAuthority {
    binding: SupportGateOriginalMergeCasBinding,
    pending: PendingOriginalMergeReceiptAuthority,
    lease: Box<dyn SupportGateOriginalMergeCasLease>,
}

impl ValidatedSupportGateOriginalMergeCasAuthority {
    pub(crate) fn resolve(
        pending: PendingOriginalMergeReceiptAuthority,
        resolver: &mut dyn SupportGateOriginalMergeCasResolver,
    ) -> Result<Self, Box<OriginalMergeCasBlockedAuthority>> {
        let binding = SupportGateOriginalMergeCasBinding::from_pending(&pending);
        let invocation = SupportGateOriginalMergeCasInvocationCapability::mint();
        let resolution =
            match resolver.resolve_original_merge_cas(SupportGateOriginalMergeCasRequest {
                binding: &binding,
                invocation: &invocation,
            }) {
                Ok(resolution) => resolution,
                Err(_) => return Err(Box::new(OriginalMergeCasBlockedAuthority { pending })),
            };
        if !invocation.owns_completion(&resolution.completion) {
            return Err(Box::new(OriginalMergeCasBlockedAuthority { pending }));
        }
        let lease = resolution.lease;
        if !lease.binds(&binding) {
            return Err(Box::new(OriginalMergeCasBlockedAuthority { pending }));
        }
        Ok(Self {
            binding,
            pending,
            lease,
        })
    }

    pub(crate) fn commit(
        self,
    ) -> Result<ValidatedOriginalMergeReceiptAuthority, Box<OriginalMergeCasBlockedAuthority>> {
        let Self {
            binding,
            pending,
            lease,
        } = self;
        if lease.commit_receipt_and_consume_gate(&pending).is_err() {
            return Err(Box::new(OriginalMergeCasBlockedAuthority { pending }));
        }
        let PendingOriginalMergeReceiptAuthority { data, preintent } = pending;
        Ok(ValidatedOriginalMergeReceiptAuthority {
            data,
            lineage: OriginalMergeReceiptLineage::Production(Box::new(
                ConsumedSupportGateAuthority { binding, preintent },
            )),
        })
    }
}

/// Failed/unknown atomic completion retains the effect and all locks/gate
/// evidence for response-loss resolution or recovery. It cannot project a
/// successful receipt.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct OriginalMergeCasBlockedAuthority {
    pending: PendingOriginalMergeReceiptAuthority,
}

impl OriginalMergeCasBlockedAuthority {
    pub(crate) fn into_pending(self) -> PendingOriginalMergeReceiptAuthority {
        self.pending
    }
}

#[derive(Debug, PartialEq, Eq)]
enum OriginalMergeReceiptLineage {
    Production(Box<ConsumedSupportGateAuthority>),
    #[cfg(test)]
    Fixture(Box<ValidatedOriginalMergeLockProjection>),
}

/// Linear post-merge receipt. The cloneable wire data is only a projection;
/// plan/lock/gate ownership remains in this non-clone authority for subsequent
/// main-integration verification and commit preview.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ValidatedOriginalMergeReceiptAuthority {
    data: OriginalMergeApplyData,
    lineage: OriginalMergeReceiptLineage,
}

impl ValidatedOriginalMergeReceiptAuthority {
    pub(crate) fn data(&self) -> MergeApplyData {
        MergeApplyData::Original(self.data.clone())
    }

    pub(crate) fn merge_receipt_id(&self) -> &UnicaId {
        &self.data.merge_receipt_id
    }

    pub(crate) fn session_id(&self) -> &UnicaId {
        &self.data.session_id
    }

    pub(crate) fn resolved_session_digest(&self) -> &Sha256Digest {
        &self.data.resolved_session_digest
    }

    pub(crate) fn result_fingerprint(&self) -> &Sha256Digest {
        &self.data.result_fingerprint
    }

    pub(crate) fn repository_history_cursor(&self) -> &RepositoryHistoryCursor {
        &self.data.repository_history_cursor
    }

    pub(crate) fn rollback_checkpoint_id(&self) -> &UnicaId {
        &self.data.rollback_checkpoint_id
    }

    pub(crate) fn plan_id(&self) -> &UnicaId {
        self.lock_projection().plan_id()
    }

    pub(crate) fn plan_digest(&self) -> &Sha256Digest {
        self.lock_projection().plan_digest()
    }

    pub(crate) fn support_gate_id(&self) -> &UnicaId {
        self.lock_projection().support_gate_id()
    }

    pub(crate) fn support_gate_digest(&self) -> &Sha256Digest {
        &self.data.support_gate_digest
    }

    pub(crate) fn support_gate_history_evidence(&self) -> &SupportGateHistoryEvidence {
        self.lock_projection().support_gate_history_evidence()
    }

    pub(crate) fn integration_set_digest(&self) -> &Sha256Digest {
        &self.data.integration_set_digest
    }

    pub(crate) fn integration_set_id(&self) -> &UnicaId {
        self.lock_projection().integration_set_id()
    }

    pub(crate) fn lock_set_id(&self) -> &UnicaId {
        self.lock_projection().lock_set_id()
    }

    pub(crate) fn lock_set_digest(&self) -> &Sha256Digest {
        &self.data.lock_set_digest
    }

    pub(crate) fn applied_decision_ids(&self) -> &[UnicaId] {
        &self.data.applied_decision_ids.0
    }

    pub(crate) fn lock_projection(&self) -> &ValidatedOriginalMergeLockProjection {
        match &self.lineage {
            OriginalMergeReceiptLineage::Production(consumed) => consumed.lock_projection(),
            #[cfg(test)]
            OriginalMergeReceiptLineage::Fixture(projection) => projection,
        }
    }

    pub(crate) fn consumed_gate(&self) -> &ConsumedSupportGateAuthority {
        match &self.lineage {
            OriginalMergeReceiptLineage::Production(consumed) => consumed,
            #[cfg(test)]
            OriginalMergeReceiptLineage::Fixture(_) => {
                panic!("fixture receipt has no persisted consumed support gate")
            }
        }
    }

    fn production_consumed_gate(&self) -> Option<&ConsumedSupportGateAuthority> {
        match &self.lineage {
            OriginalMergeReceiptLineage::Production(consumed) => Some(consumed),
            #[cfg(test)]
            OriginalMergeReceiptLineage::Fixture(_) => None,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct ConsumedSupportGateObservationBinding {
    current_state_revision: Sha256Digest,
    support_gate_id: UnicaId,
    support_gate_digest: Sha256Digest,
    support_gate_history_evidence_digest: Sha256Digest,
    session_id: UnicaId,
    resolved_session_digest: Sha256Digest,
    plan_id: UnicaId,
    plan_digest: Sha256Digest,
    integration_set_id: UnicaId,
    integration_set_digest: Sha256Digest,
    lock_set_id: UnicaId,
    lock_set_digest: Sha256Digest,
    root_lock_receipt: JournaledRepositoryLock,
    ordered_retained_lock_receipts: Vec<JournaledRepositoryLock>,
    missing_lock_receipt_count: usize,
    merge_receipt_id: UnicaId,
    merge_receipt_cursor: RepositoryHistoryCursor,
    result_fingerprint: Sha256Digest,
    rollback_checkpoint_id: UnicaId,
    before_anchor: Sha256Digest,
    after_anchor: Sha256Digest,
    support_audit_digest: Sha256Digest,
    preintent_capability_id: CapabilityRowId,
    root_reread_capability_id: CapabilityRowId,
    support_graph_digest: Sha256Digest,
    original_fingerprint: Sha256Digest,
    reference_closure_digest: Sha256Digest,
    applied_decision_ids: Vec<UnicaId>,
}

impl ConsumedSupportGateObservationBinding {
    fn from_cas(binding: &SupportGateOriginalMergeCasBinding) -> Self {
        Self {
            current_state_revision: binding.current_state_revision.clone(),
            support_gate_id: binding.support_gate_id.clone(),
            support_gate_digest: binding.support_gate_digest.clone(),
            support_gate_history_evidence_digest: binding
                .support_gate_history_evidence_digest
                .clone(),
            session_id: binding.session_id.clone(),
            resolved_session_digest: binding.resolved_session_digest.clone(),
            plan_id: binding.plan_id.clone(),
            plan_digest: binding.plan_digest.clone(),
            integration_set_id: binding.integration_set_id.clone(),
            integration_set_digest: binding.integration_set_digest.clone(),
            lock_set_id: binding.lock_set_id.clone(),
            lock_set_digest: binding.lock_set_digest.clone(),
            root_lock_receipt: binding.root_lock_receipt.clone(),
            ordered_retained_lock_receipts: binding.ordered_retained_lock_receipts.clone(),
            missing_lock_receipt_count: binding.missing_lock_receipt_count,
            merge_receipt_id: binding.merge_receipt_id.clone(),
            merge_receipt_cursor: binding.merge_receipt_cursor.clone(),
            result_fingerprint: binding.result_fingerprint.clone(),
            rollback_checkpoint_id: binding.rollback_checkpoint_id.clone(),
            before_anchor: binding.before_anchor.clone(),
            after_anchor: binding.after_anchor.clone(),
            support_audit_digest: binding.support_audit_digest.clone(),
            preintent_capability_id: binding.preintent_capability_id.clone(),
            root_reread_capability_id: binding.root_reread_capability_id.clone(),
            support_graph_digest: binding.support_graph_digest.clone(),
            original_fingerprint: binding.original_fingerprint.clone(),
            reference_closure_digest: binding.reference_closure_digest.clone(),
            applied_decision_ids: binding.applied_decision_ids.clone(),
        }
    }

    fn from_pending(pending: &PendingOriginalMergeReceiptAuthority) -> Self {
        Self::from_cas(&SupportGateOriginalMergeCasBinding::from_pending(pending))
    }

    fn from_consumed(consumed: &ConsumedSupportGateAuthority) -> Self {
        Self::from_cas(&consumed.binding)
    }
}

/// Exact persisted-consumed query. It is derived from a pending response-loss
/// lineage or an already validated receipt and never from a resume/status DTO.
#[derive(Debug)]
pub(crate) struct ConsumedSupportGateObservationRequest<'a> {
    binding: &'a ConsumedSupportGateObservationBinding,
    invocation: &'a ConsumedSupportGateObservationInvocationCapability,
}

#[derive(Debug)]
struct ConsumedSupportGateObservationInvocationMarker;

#[derive(Debug)]
struct ConsumedSupportGateObservationInvocationCapability(
    Arc<ConsumedSupportGateObservationInvocationMarker>,
);

#[derive(Debug)]
struct ConsumedSupportGateObservationCompletionCapability(
    Arc<ConsumedSupportGateObservationInvocationMarker>,
);

impl ConsumedSupportGateObservationInvocationCapability {
    fn mint() -> Self {
        Self(Arc::new(ConsumedSupportGateObservationInvocationMarker))
    }

    fn completion(&self) -> ConsumedSupportGateObservationCompletionCapability {
        ConsumedSupportGateObservationCompletionCapability(Arc::clone(&self.0))
    }

    fn owns_completion(
        &self,
        completion: &ConsumedSupportGateObservationCompletionCapability,
    ) -> bool {
        Arc::ptr_eq(&self.0, &completion.0)
    }
}

impl ConsumedSupportGateObservationRequest<'_> {
    pub(crate) fn current_state_revision(&self) -> &Sha256Digest {
        &self.binding.current_state_revision
    }

    pub(crate) fn support_gate_id(&self) -> &UnicaId {
        &self.binding.support_gate_id
    }

    pub(crate) fn support_gate_digest(&self) -> &Sha256Digest {
        &self.binding.support_gate_digest
    }

    pub(crate) fn support_gate_history_evidence_digest(&self) -> &Sha256Digest {
        &self.binding.support_gate_history_evidence_digest
    }

    pub(crate) fn session_id(&self) -> &UnicaId {
        &self.binding.session_id
    }

    pub(crate) fn resolved_session_digest(&self) -> &Sha256Digest {
        &self.binding.resolved_session_digest
    }

    pub(crate) fn plan_id(&self) -> &UnicaId {
        &self.binding.plan_id
    }

    pub(crate) fn plan_digest(&self) -> &Sha256Digest {
        &self.binding.plan_digest
    }

    pub(crate) fn integration_set_id(&self) -> &UnicaId {
        &self.binding.integration_set_id
    }

    pub(crate) fn integration_set_digest(&self) -> &Sha256Digest {
        &self.binding.integration_set_digest
    }

    pub(crate) fn lock_set_id(&self) -> &UnicaId {
        &self.binding.lock_set_id
    }

    pub(crate) fn lock_set_digest(&self) -> &Sha256Digest {
        &self.binding.lock_set_digest
    }

    pub(crate) fn root_lock_receipt(&self) -> &JournaledRepositoryLock {
        &self.binding.root_lock_receipt
    }

    pub(crate) fn journaled_lock_receipts(&self) -> &[JournaledRepositoryLock] {
        &self.binding.ordered_retained_lock_receipts
    }

    pub(crate) fn missing_lock_receipt_count(&self) -> usize {
        self.binding.missing_lock_receipt_count
    }

    pub(crate) fn merge_receipt_id(&self) -> &UnicaId {
        &self.binding.merge_receipt_id
    }

    pub(crate) fn merge_receipt_cursor(&self) -> &RepositoryHistoryCursor {
        &self.binding.merge_receipt_cursor
    }

    pub(crate) fn result_fingerprint(&self) -> &Sha256Digest {
        &self.binding.result_fingerprint
    }

    pub(crate) fn rollback_checkpoint_id(&self) -> &UnicaId {
        &self.binding.rollback_checkpoint_id
    }

    pub(crate) fn before_anchor(&self) -> &Sha256Digest {
        &self.binding.before_anchor
    }

    pub(crate) fn after_anchor(&self) -> &Sha256Digest {
        &self.binding.after_anchor
    }

    pub(crate) fn support_audit_digest(&self) -> &Sha256Digest {
        &self.binding.support_audit_digest
    }

    pub(crate) fn preintent_capability_id(&self) -> &CapabilityRowId {
        &self.binding.preintent_capability_id
    }

    pub(crate) fn root_reread_capability_id(&self) -> &CapabilityRowId {
        &self.binding.root_reread_capability_id
    }

    pub(crate) fn support_graph_digest(&self) -> &Sha256Digest {
        &self.binding.support_graph_digest
    }

    pub(crate) fn original_fingerprint(&self) -> &Sha256Digest {
        &self.binding.original_fingerprint
    }

    pub(crate) fn reference_closure_digest(&self) -> &Sha256Digest {
        &self.binding.reference_closure_digest
    }

    pub(crate) fn applied_decision_ids(&self) -> &[UnicaId] {
        &self.binding.applied_decision_ids
    }

    pub(crate) fn complete(
        self,
        lease: Box<dyn ConsumedSupportGateStateLease>,
    ) -> ConsumedSupportGateStateResolution {
        ConsumedSupportGateStateResolution {
            completion: self.invocation.completion(),
            lease,
        }
    }
}

pub(crate) trait ConsumedSupportGateStateLease {
    fn binds(&self, request: &ConsumedSupportGateObservationRequest<'_>) -> bool;
    fn consumed_state_revision(&self) -> &Sha256Digest;
    fn observation_capability_id(&self) -> &CapabilityRowId;
}

pub(crate) trait ConsumedSupportGateStateResolver {
    fn resolve_consumed_by_original_merge(
        &mut self,
        request: ConsumedSupportGateObservationRequest<'_>,
    ) -> Result<ConsumedSupportGateStateResolution, MergeResultContractError>;
}

pub(crate) struct ConsumedSupportGateStateResolution {
    completion: ConsumedSupportGateObservationCompletionCapability,
    lease: Box<dyn ConsumedSupportGateStateLease>,
}

/// Sealed trusted observation of the authoritative persisted consumed state.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ObservedConsumedSupportGateAuthority {
    binding: ConsumedSupportGateObservationBinding,
    consumed_state_revision: Sha256Digest,
    observation_capability_id: CapabilityRowId,
}

impl ObservedConsumedSupportGateAuthority {
    fn resolve_binding<Source>(
        source: Source,
        binding: ConsumedSupportGateObservationBinding,
        resolver: &mut dyn ConsumedSupportGateStateResolver,
    ) -> Result<(Source, Self), Box<ConsumedSupportGateResolutionBlockedAuthority<Source>>> {
        let invocation = ConsumedSupportGateObservationInvocationCapability::mint();
        let request = ConsumedSupportGateObservationRequest {
            binding: &binding,
            invocation: &invocation,
        };
        let resolution = match resolver.resolve_consumed_by_original_merge(request) {
            Ok(resolution) => resolution,
            Err(error) => {
                return Err(ConsumedSupportGateResolutionBlockedAuthority::new(
                    source, error,
                ));
            }
        };
        if !invocation.owns_completion(&resolution.completion) {
            return Err(ConsumedSupportGateResolutionBlockedAuthority::new(
                source,
                MergeResultContractError(
                    "consumed-state completion belongs to another resolution attempt",
                ),
            ));
        }
        let request = ConsumedSupportGateObservationRequest {
            binding: &binding,
            invocation: &invocation,
        };
        let lease = resolution.lease;
        if !lease.binds(&request) {
            return Err(ConsumedSupportGateResolutionBlockedAuthority::new(
                source,
                MergeResultContractError(
                    "authoritative state did not resolve the exact consumed support gate and receipt",
                ),
            ));
        }
        Ok((
            source,
            Self {
                binding,
                consumed_state_revision: lease.consumed_state_revision().clone(),
                observation_capability_id: lease.observation_capability_id().clone(),
            },
        ))
    }

    pub(crate) fn consumed_state_revision(&self) -> &Sha256Digest {
        &self.consumed_state_revision
    }

    pub(crate) fn observation_capability_id(&self) -> &CapabilityRowId {
        &self.observation_capability_id
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ConsumedSupportGateResolutionBlockedAuthority<Source> {
    source: Source,
    failure: MergeResultContractError,
}

impl<Source> ConsumedSupportGateResolutionBlockedAuthority<Source> {
    fn new(source: Source, failure: MergeResultContractError) -> Box<Self> {
        Box::new(Self { source, failure })
    }

    pub(crate) fn into_recovery_parts(self) -> (Source, MergeResultContractError) {
        let Self { source, failure } = self;
        (source, failure)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ResolvedPendingConsumedSupportGateAuthority {
    pending: PendingOriginalMergeReceiptAuthority,
    observed: ObservedConsumedSupportGateAuthority,
}

impl ResolvedPendingConsumedSupportGateAuthority {
    pub(crate) fn resolve(
        pending: PendingOriginalMergeReceiptAuthority,
        resolver: &mut dyn ConsumedSupportGateStateResolver,
    ) -> Result<
        Self,
        Box<ConsumedSupportGateResolutionBlockedAuthority<PendingOriginalMergeReceiptAuthority>>,
    > {
        let binding = ConsumedSupportGateObservationBinding::from_pending(&pending);
        let (pending, observed) =
            ObservedConsumedSupportGateAuthority::resolve_binding(pending, binding, resolver)?;
        Ok(Self { pending, observed })
    }

    pub(crate) fn finalize(self) -> ValidatedOriginalMergeReceiptAuthority {
        let Self { pending, .. } = self;
        let binding = SupportGateOriginalMergeCasBinding::from_pending(&pending);
        let PendingOriginalMergeReceiptAuthority { data, preintent } = pending;
        ValidatedOriginalMergeReceiptAuthority {
            data,
            lineage: OriginalMergeReceiptLineage::Production(Box::new(
                ConsumedSupportGateAuthority { binding, preintent },
            )),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ResolvedReceiptConsumedSupportGateAuthority {
    receipt: ValidatedOriginalMergeReceiptAuthority,
    observed: ObservedConsumedSupportGateAuthority,
}

impl ResolvedReceiptConsumedSupportGateAuthority {
    pub(crate) fn resolve(
        receipt: ValidatedOriginalMergeReceiptAuthority,
        resolver: &mut dyn ConsumedSupportGateStateResolver,
    ) -> Result<
        Self,
        Box<ConsumedSupportGateResolutionBlockedAuthority<ValidatedOriginalMergeReceiptAuthority>>,
    > {
        let Some(consumed) = receipt.production_consumed_gate() else {
            return Err(ConsumedSupportGateResolutionBlockedAuthority::new(
                receipt,
                MergeResultContractError(
                    "fixture receipt cannot resolve authoritative consumed state",
                ),
            ));
        };
        let binding = ConsumedSupportGateObservationBinding::from_consumed(consumed);
        let (receipt, observed) =
            ObservedConsumedSupportGateAuthority::resolve_binding(receipt, binding, resolver)?;
        Ok(Self { receipt, observed })
    }

    pub(crate) fn rebind(self) -> ValidatedConsumedOriginalMergeLineageAuthority {
        let Self { receipt, observed } = self;
        ValidatedConsumedOriginalMergeLineageAuthority { receipt, observed }
    }
}

/// Receipt plus the independently re-resolved persisted consumed state. This
/// is the only production input accepted by main-integration verification.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ValidatedConsumedOriginalMergeLineageAuthority {
    receipt: ValidatedOriginalMergeReceiptAuthority,
    observed: ObservedConsumedSupportGateAuthority,
}

impl ValidatedConsumedOriginalMergeLineageAuthority {
    fn receipt(&self) -> &ValidatedOriginalMergeReceiptAuthority {
        &self.receipt
    }

    pub(crate) fn merge_receipt_id(&self) -> &UnicaId {
        self.receipt.merge_receipt_id()
    }

    pub(crate) fn support_gate_id(&self) -> &UnicaId {
        self.receipt.support_gate_id()
    }

    pub(crate) fn lock_set_id(&self) -> &UnicaId {
        self.receipt.lock_set_id()
    }

    pub(crate) fn consumed_gate(&self) -> &ConsumedSupportGateAuthority {
        self.receipt.consumed_gate()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub(crate) enum MergeApplyData {
    Task(TaskMergeApplyData),
    Original(OriginalMergeApplyData),
}

#[cfg(test)]
struct TaskMergeApplyFixtureInput {
    merge_receipt_id: UnicaId,
    before_anchor: Sha256Digest,
    after_anchor: Sha256Digest,
    result_fingerprint: Sha256Digest,
    support_audit_digest: Sha256Digest,
    applied_decision_ids: Vec<UnicaId>,
    source_publication_id: UnicaId,
    source_fingerprint: Sha256Digest,
    task_infobase_fingerprint: Sha256Digest,
}

#[cfg(test)]
struct OriginalMergeApplyFixtureInput {
    merge_receipt_id: UnicaId,
    before_anchor: Sha256Digest,
    after_anchor: Sha256Digest,
    result_fingerprint: Sha256Digest,
    repository_history_cursor: RepositoryHistoryCursor,
    support_audit_digest: Sha256Digest,
    applied_decision_ids: Vec<UnicaId>,
    rollback_checkpoint_id: UnicaId,
    integration_set_digest: Sha256Digest,
    lock_set_digest: Sha256Digest,
}

impl JsonSchema for MergeApplyData {
    fn schema_name() -> Cow<'static, str> {
        "MergeApplyData".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<TaskMergeApplyData>(),
            generator.subschema_for::<OriginalMergeApplyData>(),
        ])
    }
}

impl MergeApplyData {
    pub(crate) fn task_from_authorities(
        session: &MergeSessionData,
        decision_projection: ResolvedApplyDecisionProjectionAuthority,
        publication: TaskSourcePublicationAuthority,
        merge_receipt_id: UnicaId,
        before_anchor: Sha256Digest,
        after_anchor: Sha256Digest,
        support_audit_digest: Sha256Digest,
    ) -> Result<Self, MergeResultContractError> {
        if session.mode() == MergeSessionModeDigest::MainIntegration
            || decision_projection.session_id != *session.session_id()
            || session.resolved_parts().map(|(_, digest)| digest)
                != Some(&decision_projection.resolved_session_digest)
            || !session
                .validates_resolved_projection(&decision_projection.applied_decision_ids.0)?
        {
            return Err(MergeResultContractError(
                "task apply requires the exact resolved task-session decision projection",
            ));
        }
        let fingerprint = publication.canonical_fingerprint;
        Ok(Self::Task(TaskMergeApplyData {
            merge_receipt_id,
            session_id: decision_projection.session_id,
            resolved_session_digest: decision_projection.resolved_session_digest,
            target: TaskTarget::Value,
            before_anchor,
            after_anchor,
            result_fingerprint: fingerprint.clone(),
            support_audit_digest,
            applied_decision_ids: decision_projection.applied_decision_ids,
            source_publication_id: publication.source_publication_id,
            source_fingerprint: fingerprint.clone(),
            task_infobase_fingerprint: fingerprint,
        }))
    }

    #[cfg(test)]
    pub(crate) fn original_from_authorities(
        session: &MergeSessionData,
        decision_projection: ResolvedApplyDecisionProjectionAuthority,
        rollback: VerifiedOriginalRollbackCheckpointAuthority,
        observation: OriginalMergeEffectObservationAuthority,
        merge_receipt_id: UnicaId,
    ) -> Result<ValidatedOriginalMergeReceiptAuthority, MergeResultContractError> {
        let MergeSessionData::MainIntegration(main) = session else {
            return Err(MergeResultContractError(
                "original apply requires a main-integration session",
            ));
        };
        let lock_projection = rollback.lock_projection();
        let history = lock_projection.support_gate_history_evidence().clone();
        if decision_projection.session_id != main.session_id
            || decision_projection.resolved_session_digest != main.resolved_session_digest
            || !session
                .validates_resolved_projection(&decision_projection.applied_decision_ids.0)?
            || lock_projection.merge_session_id() != &main.session_id
            || lock_projection.resolved_session_digest() != &main.resolved_session_digest
            || lock_projection.support_gate_id() != &main.support_gate_id
            || lock_projection.support_gate_digest() != &main.support_gate_digest
            || history.evidence_digest() != &main.support_gate_history_evidence_digest
            || !original_merge_checkpoint_source_is_complete(lock_projection)
        {
            return Err(MergeResultContractError(
                "original apply plan, locks, gate history, rollback, and session lineage disagree",
            ));
        }
        let VerifiedOriginalRollbackCheckpointAuthority {
            source: lock_projection,
            checkpoint_id,
            checkpoint_fingerprint: _,
            root_before_anchor: _,
            observed_current_state_revision: _,
            checkpoint_capability_id: _,
        } = rollback;
        let data = OriginalMergeApplyData {
            merge_receipt_id,
            session_id: decision_projection.session_id,
            resolved_session_digest: decision_projection.resolved_session_digest,
            target: OriginalTarget::Value,
            before_anchor: observation.before_anchor,
            after_anchor: observation.after_anchor,
            result_fingerprint: observation.result_fingerprint,
            repository_history_cursor: history.classified_through_cursor().clone(),
            support_audit_digest: observation.support_audit_digest,
            applied_decision_ids: decision_projection.applied_decision_ids,
            rollback_checkpoint_id: checkpoint_id,
            integration_set_digest: lock_projection.integration_set_digest().clone(),
            lock_set_digest: lock_projection.lock_set_digest().clone(),
            support_gate_digest: lock_projection.support_gate_digest().clone(),
            support_gate_history_evidence_digest: history.evidence_digest().clone(),
        };
        Ok(ValidatedOriginalMergeReceiptAuthority {
            data,
            lineage: OriginalMergeReceiptLineage::Fixture(Box::new(lock_projection)),
        })
    }

    #[cfg(test)]
    fn task_test_only(
        session: &MergeSessionData,
        input: TaskMergeApplyFixtureInput,
    ) -> Result<Self, MergeResultContractError> {
        if session.mode() == MergeSessionModeDigest::MainIntegration {
            return Err(MergeResultContractError(
                "task apply requires a supported-update or resolved-replay session",
            ));
        }
        let applied_decision_ids = AppliedDecisionIds::new(input.applied_decision_ids)?;
        if !session.validates_resolved_projection(&applied_decision_ids.0)? {
            return Err(MergeResultContractError(
                "task apply does not reproduce the resolved-session decision projection",
            ));
        }
        let (_, resolved_session_digest) = session.resolved_parts().expect("validated above");
        Ok(Self::Task(TaskMergeApplyData {
            merge_receipt_id: input.merge_receipt_id,
            session_id: session.session_id().clone(),
            resolved_session_digest: resolved_session_digest.clone(),
            target: TaskTarget::Value,
            before_anchor: input.before_anchor,
            after_anchor: input.after_anchor,
            result_fingerprint: input.result_fingerprint,
            support_audit_digest: input.support_audit_digest,
            applied_decision_ids,
            source_publication_id: input.source_publication_id,
            source_fingerprint: input.source_fingerprint,
            task_infobase_fingerprint: input.task_infobase_fingerprint,
        }))
    }

    #[cfg(test)]
    fn original_test_only(
        session: &MergeSessionData,
        input: OriginalMergeApplyFixtureInput,
    ) -> Result<Self, MergeResultContractError> {
        let MergeSessionData::MainIntegration(main) = session else {
            return Err(MergeResultContractError(
                "original apply requires a main-integration session",
            ));
        };
        let applied_decision_ids = AppliedDecisionIds::new(input.applied_decision_ids)?;
        if !session.validates_resolved_projection(&applied_decision_ids.0)? {
            return Err(MergeResultContractError(
                "original apply does not reproduce the resolved-session decision projection",
            ));
        }
        Ok(Self::Original(OriginalMergeApplyData {
            merge_receipt_id: input.merge_receipt_id,
            session_id: main.session_id.clone(),
            resolved_session_digest: main.resolved_session_digest.clone(),
            target: OriginalTarget::Value,
            before_anchor: input.before_anchor,
            after_anchor: input.after_anchor,
            result_fingerprint: input.result_fingerprint,
            repository_history_cursor: input.repository_history_cursor,
            support_audit_digest: input.support_audit_digest,
            applied_decision_ids,
            rollback_checkpoint_id: input.rollback_checkpoint_id,
            integration_set_digest: input.integration_set_digest,
            lock_set_digest: input.lock_set_digest,
            support_gate_digest: main.support_gate_digest.clone(),
            support_gate_history_evidence_digest: main.support_gate_history_evidence_digest.clone(),
        }))
    }
}

// -------------------------------------------------------------------------
// Verification results

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct ValidationReceiptIds(Vec<UnicaId>);

/// Configuration-owned exact validation-check sequence. Order is deliberately
/// not canonicalized: it is the configured execution order.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ConfiguredValidationCheckPlanAuthority {
    check_ids: Vec<Name>,
}

impl ConfiguredValidationCheckPlanAuthority {
    pub(crate) fn from_configuration_adapter(
        check_ids: Vec<Name>,
    ) -> Result<Self, MergeResultContractError> {
        if check_ids.len() > MAX_RESULT_ITEMS {
            return Err(MergeResultContractError(
                "configured validation check list is oversized",
            ));
        }
        let mut seen = BTreeSet::new();
        if check_ids
            .iter()
            .any(|check_id| !seen.insert(check_id.as_str()))
        {
            return Err(MergeResultContractError(
                "configured validation checks must be unique in execution order",
            ));
        }
        Ok(Self { check_ids })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum LocalCheckpointValidationExecutionScope {}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ResolvedTaskValidationExecutionScope {}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum MainSandboxValidationExecutionScope {}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum MainIntegrationValidationExecutionScope {}

#[derive(Debug, PartialEq, Eq)]
enum ConfiguredValidationExecutionSubjectAuthority {
    LocalCheckpoint {
        verification_id: UnicaId,
    },
    ResolvedTask {
        verification_id: UnicaId,
        session_id: UnicaId,
        resolved_session_digest: Sha256Digest,
    },
    MainSandbox {
        verification_id: UnicaId,
        session_id: UnicaId,
        resolved_session_digest: Sha256Digest,
        support_gate_id: UnicaId,
        support_gate_digest: Sha256Digest,
        support_gate_history_evidence_digest: Sha256Digest,
        settings_digest: Sha256Digest,
        comparison_id: UnicaId,
        ordinary_result_artifact_id: UnicaId,
        result_digest: Sha256Digest,
        applied_decision_ids: Vec<UnicaId>,
    },
    MainIntegration {
        verification_id: UnicaId,
        lineage: MainIntegrationVerificationLineage,
    },
}

impl ConfiguredValidationExecutionSubjectAuthority {
    const fn verification_id(&self) -> &UnicaId {
        match self {
            Self::LocalCheckpoint { verification_id }
            | Self::ResolvedTask {
                verification_id, ..
            }
            | Self::MainSandbox {
                verification_id, ..
            }
            | Self::MainIntegration {
                verification_id, ..
            } => verification_id,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ConfiguredValidationExecutionSelectionAuthority<Scope> {
    configured_check_ids: Vec<Name>,
    subject: ConfiguredValidationExecutionSubjectAuthority,
    scope: PhantomData<fn() -> Scope>,
}

pub(crate) type LocalCheckpointConfiguredValidationExecutionSelectionAuthority =
    ConfiguredValidationExecutionSelectionAuthority<LocalCheckpointValidationExecutionScope>;
pub(crate) type ResolvedTaskConfiguredValidationExecutionSelectionAuthority =
    ConfiguredValidationExecutionSelectionAuthority<ResolvedTaskValidationExecutionScope>;
pub(crate) type MainSandboxConfiguredValidationExecutionSelectionAuthority =
    ConfiguredValidationExecutionSelectionAuthority<MainSandboxValidationExecutionScope>;
pub(crate) type MainIntegrationConfiguredValidationExecutionSelectionAuthority =
    ConfiguredValidationExecutionSelectionAuthority<MainIntegrationValidationExecutionScope>;

impl ConfiguredValidationExecutionSelectionAuthority<LocalCheckpointValidationExecutionScope> {
    pub(crate) fn local_checkpoint(
        plan: &ConfiguredValidationCheckPlanAuthority,
        verification_id: UnicaId,
    ) -> Self {
        Self {
            configured_check_ids: plan.check_ids.clone(),
            subject: ConfiguredValidationExecutionSubjectAuthority::LocalCheckpoint {
                verification_id,
            },
            scope: PhantomData,
        }
    }

    fn matches_local_checkpoint(&self, verification_id: &UnicaId) -> bool {
        matches!(
            &self.subject,
            ConfiguredValidationExecutionSubjectAuthority::LocalCheckpoint {
                verification_id: selected,
            } if selected == verification_id
        )
    }
}

impl ConfiguredValidationExecutionSelectionAuthority<ResolvedTaskValidationExecutionScope> {
    pub(crate) fn resolved_task(
        plan: &ConfiguredValidationCheckPlanAuthority,
        session: &MergeSessionData,
        verification_id: UnicaId,
    ) -> Result<Self, MergeResultContractError> {
        if !matches!(
            session,
            MergeSessionData::SupportedUpdateResolved(_)
                | MergeSessionData::ResolvedReplayResolved(_)
        ) {
            return Err(MergeResultContractError(
                "configured validation task execution requires a resolved task session",
            ));
        }
        Ok(Self {
            configured_check_ids: plan.check_ids.clone(),
            subject: ConfiguredValidationExecutionSubjectAuthority::ResolvedTask {
                verification_id,
                session_id: session.session_id().clone(),
                resolved_session_digest: session
                    .resolved_session_digest()
                    .expect("resolved branch checked above")
                    .clone(),
            },
            scope: PhantomData,
        })
    }

    fn matches_resolved_task(&self, session: &MergeSessionData, verification_id: &UnicaId) -> bool {
        matches!(
            &self.subject,
            ConfiguredValidationExecutionSubjectAuthority::ResolvedTask {
                verification_id: selected,
                session_id,
                resolved_session_digest,
            } if selected == verification_id
                && session_id == session.session_id()
                && session.resolved_session_digest() == Some(resolved_session_digest)
        )
    }
}

impl ConfiguredValidationExecutionSelectionAuthority<MainSandboxValidationExecutionScope> {
    pub(crate) fn main_sandbox(
        plan: &ConfiguredValidationCheckPlanAuthority,
        planning: &ValidatedRepositoryPlanSessionProjection,
        verification_id: UnicaId,
    ) -> Self {
        Self {
            configured_check_ids: plan.check_ids.clone(),
            subject: ConfiguredValidationExecutionSubjectAuthority::MainSandbox {
                verification_id,
                session_id: planning.merge_session_id().clone(),
                resolved_session_digest: planning.resolved_session_digest().clone(),
                support_gate_id: planning.support_gate_id().clone(),
                support_gate_digest: planning.support_gate_digest().clone(),
                support_gate_history_evidence_digest: planning
                    .support_gate_history_evidence()
                    .evidence_digest()
                    .clone(),
                settings_digest: planning.settings_digest().clone(),
                comparison_id: planning.comparison_id().clone(),
                ordinary_result_artifact_id: planning.ordinary_result_artifact_id().clone(),
                result_digest: planning.result_digest().clone(),
                applied_decision_ids: planning.applied_decision_ids().to_vec(),
            },
            scope: PhantomData,
        }
    }

    fn matches_main_sandbox(
        &self,
        planning: &ValidatedRepositoryPlanSessionProjection,
        verification_id: &UnicaId,
    ) -> bool {
        matches!(
            &self.subject,
            ConfiguredValidationExecutionSubjectAuthority::MainSandbox {
                verification_id: selected,
                session_id,
                resolved_session_digest,
                support_gate_id,
                support_gate_digest,
                support_gate_history_evidence_digest,
                settings_digest,
                comparison_id,
                ordinary_result_artifact_id,
                result_digest,
                applied_decision_ids,
            } if selected == verification_id
                && session_id == planning.merge_session_id()
                && resolved_session_digest == planning.resolved_session_digest()
                && support_gate_id == planning.support_gate_id()
                && support_gate_digest == planning.support_gate_digest()
                && support_gate_history_evidence_digest
                    == planning.support_gate_history_evidence().evidence_digest()
                && settings_digest == planning.settings_digest()
                && comparison_id == planning.comparison_id()
                && ordinary_result_artifact_id == planning.ordinary_result_artifact_id()
                && result_digest == planning.result_digest()
                && applied_decision_ids == planning.applied_decision_ids()
        )
    }
}

impl ConfiguredValidationExecutionSelectionAuthority<MainIntegrationValidationExecutionScope> {
    pub(crate) fn main_integration(
        plan: &ConfiguredValidationCheckPlanAuthority,
        lineage: ValidatedConsumedOriginalMergeLineageAuthority,
        verification_id: UnicaId,
    ) -> Self {
        let subject = ConfiguredValidationExecutionSubjectAuthority::MainIntegration {
            verification_id,
            lineage: MainIntegrationVerificationLineage::from_consumed(lineage),
        };
        Self {
            configured_check_ids: plan.check_ids.clone(),
            subject,
            scope: PhantomData,
        }
    }

    fn main_integration_lineage(&self) -> &MainIntegrationVerificationLineage {
        let ConfiguredValidationExecutionSubjectAuthority::MainIntegration { lineage, .. } =
            &self.subject
        else {
            unreachable!("main-integration scope marker is minted only with its lineage")
        };
        lineage
    }

    fn into_main_integration_lineage(self) -> MainIntegrationVerificationLineage {
        let ConfiguredValidationExecutionSubjectAuthority::MainIntegration { lineage, .. } =
            self.subject
        else {
            unreachable!("main-integration scope marker is minted only with its lineage")
        };
        lineage
    }
}

/// Read-only exact validation target exposed to the configured-check adapter.
/// Every value is derived from the selected authority; the adapter cannot
/// provide or replace any context scalar.
#[derive(Debug, Clone, Copy)]
pub(crate) enum ConfiguredValidationCheckExecutionContext<'a> {
    LocalCheckpoint {
        verification_id: &'a UnicaId,
    },
    ResolvedTask {
        verification_id: &'a UnicaId,
        session_id: &'a UnicaId,
        resolved_session_digest: &'a Sha256Digest,
    },
    MainSandbox {
        verification_id: &'a UnicaId,
        session_id: &'a UnicaId,
        resolved_session_digest: &'a Sha256Digest,
        support_gate_id: &'a UnicaId,
        support_gate_digest: &'a Sha256Digest,
        support_gate_history_evidence_digest: &'a Sha256Digest,
        settings_digest: &'a Sha256Digest,
        comparison_id: &'a UnicaId,
        ordinary_result_artifact_id: &'a UnicaId,
        result_digest: &'a Sha256Digest,
        applied_decision_ids: &'a [UnicaId],
    },
    MainIntegration {
        verification_id: &'a UnicaId,
        session_id: &'a UnicaId,
        resolved_session_digest: &'a Sha256Digest,
        merge_receipt_id: &'a UnicaId,
        before_anchor: &'a Sha256Digest,
        after_anchor: &'a Sha256Digest,
        result_fingerprint: &'a Sha256Digest,
        repository_history_cursor: &'a RepositoryHistoryCursor,
        support_audit_digest: &'a Sha256Digest,
        applied_decision_ids: &'a [UnicaId],
        rollback_checkpoint_id: &'a UnicaId,
        integration_set_id: &'a UnicaId,
        integration_set_digest: &'a Sha256Digest,
        lock_set_id: &'a UnicaId,
        lock_set_digest: &'a Sha256Digest,
        plan_id: &'a UnicaId,
        plan_digest: &'a Sha256Digest,
        support_gate_id: &'a UnicaId,
        support_gate_digest: &'a Sha256Digest,
        support_gate_history_evidence_digest: &'a Sha256Digest,
    },
}

/// One-shot request created only by `from_execution_port`. The adapter can
/// inspect its read-only target and can complete only this exact request.
#[derive(Debug)]
pub(crate) struct ConfiguredValidationCheckExecutionRequest<'a> {
    configured_check_ids: &'a [Name],
    subject: &'a ConfiguredValidationExecutionSubjectAuthority,
    attempt: &'a ConfiguredValidationExecutionAttemptCapability,
}

#[derive(Debug)]
struct ConfiguredValidationExecutionAttemptMarker;

#[derive(Debug)]
struct ConfiguredValidationExecutionAttemptCapability(
    Arc<ConfiguredValidationExecutionAttemptMarker>,
);

#[derive(Debug)]
struct ConfiguredValidationExecutionCompletionCapability(
    Arc<ConfiguredValidationExecutionAttemptMarker>,
);

impl ConfiguredValidationExecutionAttemptCapability {
    fn mint() -> Self {
        Self(Arc::new(ConfiguredValidationExecutionAttemptMarker))
    }

    fn completion(&self) -> ConfiguredValidationExecutionCompletionCapability {
        ConfiguredValidationExecutionCompletionCapability(Arc::clone(&self.0))
    }

    fn owns_completion(
        &self,
        completion: &ConfiguredValidationExecutionCompletionCapability,
    ) -> bool {
        Arc::ptr_eq(&self.0, &completion.0)
    }
}

impl ConfiguredValidationCheckExecutionRequest<'_> {
    pub(crate) fn configured_check_ids(&self) -> &[Name] {
        self.configured_check_ids
    }

    pub(crate) fn context(&self) -> ConfiguredValidationCheckExecutionContext<'_> {
        match self.subject {
            ConfiguredValidationExecutionSubjectAuthority::LocalCheckpoint { verification_id } => {
                ConfiguredValidationCheckExecutionContext::LocalCheckpoint { verification_id }
            }
            ConfiguredValidationExecutionSubjectAuthority::ResolvedTask {
                verification_id,
                session_id,
                resolved_session_digest,
            } => ConfiguredValidationCheckExecutionContext::ResolvedTask {
                verification_id,
                session_id,
                resolved_session_digest,
            },
            ConfiguredValidationExecutionSubjectAuthority::MainSandbox {
                verification_id,
                session_id,
                resolved_session_digest,
                support_gate_id,
                support_gate_digest,
                support_gate_history_evidence_digest,
                settings_digest,
                comparison_id,
                ordinary_result_artifact_id,
                result_digest,
                applied_decision_ids,
            } => ConfiguredValidationCheckExecutionContext::MainSandbox {
                verification_id,
                session_id,
                resolved_session_digest,
                support_gate_id,
                support_gate_digest,
                support_gate_history_evidence_digest,
                settings_digest,
                comparison_id,
                ordinary_result_artifact_id,
                result_digest,
                applied_decision_ids,
            },
            ConfiguredValidationExecutionSubjectAuthority::MainIntegration {
                verification_id,
                lineage,
            } => {
                let receipt = lineage.receipt();
                ConfiguredValidationCheckExecutionContext::MainIntegration {
                    verification_id,
                    session_id: receipt.session_id(),
                    resolved_session_digest: receipt.resolved_session_digest(),
                    merge_receipt_id: receipt.merge_receipt_id(),
                    before_anchor: &receipt.data.before_anchor,
                    after_anchor: &receipt.data.after_anchor,
                    result_fingerprint: receipt.result_fingerprint(),
                    repository_history_cursor: receipt.repository_history_cursor(),
                    support_audit_digest: &receipt.data.support_audit_digest,
                    applied_decision_ids: receipt.applied_decision_ids(),
                    rollback_checkpoint_id: receipt.rollback_checkpoint_id(),
                    integration_set_id: receipt.integration_set_id(),
                    integration_set_digest: receipt.integration_set_digest(),
                    lock_set_id: receipt.lock_set_id(),
                    lock_set_digest: receipt.lock_set_digest(),
                    plan_id: receipt.plan_id(),
                    plan_digest: receipt.plan_digest(),
                    support_gate_id: receipt.support_gate_id(),
                    support_gate_digest: receipt.support_gate_digest(),
                    support_gate_history_evidence_digest: receipt
                        .support_gate_history_evidence()
                        .evidence_digest(),
                }
            }
        }
    }

    pub(crate) fn complete(
        self,
        input: ConfiguredValidationReceiptBatchSnapshotInput,
    ) -> ConfiguredValidationReceiptBatchSnapshot {
        ConfiguredValidationReceiptBatchSnapshot {
            completion: self.attempt.completion(),
            observed_check_ids: input.observed_check_ids,
            receipt_ids: input.receipt_ids,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ConfiguredValidationReceiptBatchSnapshotInput {
    observed_check_ids: Vec<Name>,
    receipt_ids: Vec<UnicaId>,
}

impl ConfiguredValidationReceiptBatchSnapshotInput {
    pub(crate) const fn from_execution_adapter(
        observed_check_ids: Vec<Name>,
        receipt_ids: Vec<UnicaId>,
    ) -> Self {
        Self {
            observed_check_ids,
            receipt_ids,
        }
    }
}

#[derive(Debug)]
pub(crate) struct ConfiguredValidationReceiptBatchSnapshot {
    completion: ConfiguredValidationExecutionCompletionCapability,
    observed_check_ids: Vec<Name>,
    receipt_ids: Vec<UnicaId>,
}

pub(crate) trait ConfiguredValidationCheckExecutionPort {
    fn execute(
        &mut self,
        request: ConfiguredValidationCheckExecutionRequest<'_>,
    ) -> Result<ConfiguredValidationReceiptBatchSnapshot, MergeResultContractError>;
}

/// The only receipt authority accepted by verification observations. It owns
/// the complete configured execution result and has no row-level mint or
/// caller-supplied ordinal.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ConfiguredValidationReceiptBatchAuthority<Scope> {
    selection: ConfiguredValidationExecutionSelectionAuthority<Scope>,
    receipt_ids: ValidationReceiptIds,
}

pub(crate) type MainIntegrationConfiguredValidationReceiptBatchAuthority =
    ConfiguredValidationReceiptBatchAuthority<MainIntegrationValidationExecutionScope>;

/// A failed configured-check execution keeps the derived non-Clone selection
/// available for retry/recovery. The main-integration specialization owns its
/// exact concrete lineage.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ConfiguredValidationExecutionBlockedAuthority<Scope> {
    selection: ConfiguredValidationExecutionSelectionAuthority<Scope>,
    failure: MergeResultContractError,
}

pub(crate) type MainIntegrationConfiguredValidationExecutionBlockedAuthority =
    ConfiguredValidationExecutionBlockedAuthority<MainIntegrationValidationExecutionScope>;

impl<Scope> ConfiguredValidationExecutionBlockedAuthority<Scope> {
    fn new(
        selection: ConfiguredValidationExecutionSelectionAuthority<Scope>,
        failure: MergeResultContractError,
    ) -> Box<Self> {
        Box::new(Self { selection, failure })
    }

    pub(crate) fn into_recovery_parts(
        self: Box<Self>,
    ) -> (
        ConfiguredValidationExecutionSelectionAuthority<Scope>,
        MergeResultContractError,
    ) {
        let Self { selection, failure } = *self;
        (selection, failure)
    }
}

impl<Scope> ConfiguredValidationReceiptBatchAuthority<Scope> {
    pub(crate) fn from_execution_port(
        selection: ConfiguredValidationExecutionSelectionAuthority<Scope>,
        port: &mut dyn ConfiguredValidationCheckExecutionPort,
    ) -> Result<Self, Box<ConfiguredValidationExecutionBlockedAuthority<Scope>>> {
        let attempt = ConfiguredValidationExecutionAttemptCapability::mint();
        let snapshot = match port.execute(ConfiguredValidationCheckExecutionRequest {
            configured_check_ids: &selection.configured_check_ids,
            subject: &selection.subject,
            attempt: &attempt,
        }) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                return Err(ConfiguredValidationExecutionBlockedAuthority::new(
                    selection, error,
                ));
            }
        };
        if !attempt.owns_completion(&snapshot.completion) {
            return Err(ConfiguredValidationExecutionBlockedAuthority::new(
                selection,
                MergeResultContractError(
                    "configured validation completion belongs to another execution attempt",
                ),
            ));
        }
        let ConfiguredValidationReceiptBatchSnapshot {
            completion: _,
            observed_check_ids,
            receipt_ids,
        } = snapshot;
        if observed_check_ids != selection.configured_check_ids
            || receipt_ids.len() != selection.configured_check_ids.len()
        {
            return Err(ConfiguredValidationExecutionBlockedAuthority::new(
                selection,
                MergeResultContractError(
                    "configured validation execution disagrees with its selected check order",
                ),
            ));
        }
        let receipt_ids = match ValidationReceiptIds::new(receipt_ids) {
            Ok(receipt_ids) => receipt_ids,
            Err(error) => {
                return Err(ConfiguredValidationExecutionBlockedAuthority::new(
                    selection, error,
                ));
            }
        };
        Ok(Self {
            selection,
            receipt_ids,
        })
    }
}

impl ValidationReceiptIds {
    fn new(values: Vec<UnicaId>) -> Result<Self, MergeResultContractError> {
        if values.len() > MAX_RESULT_ITEMS {
            return Err(MergeResultContractError(
                "validation receipt list is oversized",
            ));
        }
        let mut seen = BTreeSet::new();
        if values.iter().any(|value| !seen.insert(value.as_str())) {
            return Err(MergeResultContractError(
                "validation receipts must be unique in configured check order",
            ));
        }
        Ok(Self(values))
    }
}

impl JsonSchema for ValidationReceiptIds {
    fn schema_name() -> Cow<'static, str> {
        "ValidationReceiptIds".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "minItems": 0,
            "maxItems": MAX_RESULT_ITEMS,
            "uniqueItems": true,
            "items": generator.subschema_for::<UnicaId>(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SelectedObjectFingerprint {
    target: RepositoryTargetIdentity,
    fingerprint: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct SelectedObjectFingerprints(Vec<SelectedObjectFingerprint>);

impl SelectedObjectFingerprints {
    fn new(values: Vec<SelectedObjectFingerprint>) -> Result<Self, MergeResultContractError> {
        if values.len() > MAX_RESULT_ITEMS
            || values
                .windows(2)
                .any(|pair| pair[0].target >= pair[1].target)
        {
            return Err(MergeResultContractError(
                "selected object fingerprints must be canonical and unique by target",
            ));
        }
        Ok(Self(values))
    }
}

impl JsonSchema for SelectedObjectFingerprints {
    fn schema_name() -> Cow<'static, str> {
        "SelectedObjectFingerprints".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "minItems": 0,
            "maxItems": MAX_RESULT_ITEMS,
            "uniqueItems": true,
            "items": generator.subschema_for::<SelectedObjectFingerprint>(),
        })
    }
}

macro_rules! verification_leaf {
    ($name:ident, $scope:ty, $outcome:ty $(, $field:ident : $field_type:ty )* $(,)?) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        pub(crate) struct $name {
            verification_id: UnicaId,
            scope: $scope,
            outcome: $outcome,
            canonical_delta_digest: Sha256Digest,
            $($field: $field_type,)*
            validation_receipt_ids: ValidationReceiptIds,
            support_audit_digest: Sha256Digest,
            selected_object_fingerprints: SelectedObjectFingerprints,
            verification_digest: Sha256Digest,
        }
    };
}

verification_leaf!(
    LocalCheckpointValidVerificationData,
    LocalCheckpointScope,
    ValidOutcome,
    checkpoint_id: UnicaId,
);
verification_leaf!(
    LocalCheckpointInvalidVerificationData,
    LocalCheckpointScope,
    InvalidOutcome,
);
verification_leaf!(
    SynchronizedTaskEquivalentVerificationData,
    SynchronizedTaskScope,
    EquivalentOutcome,
    session_id: UnicaId,
    checkpoint_id: UnicaId,
);
verification_leaf!(
    SynchronizedTaskAdaptedVerificationData,
    SynchronizedTaskScope,
    AdaptedOutcome,
    session_id: UnicaId,
    checkpoint_id: UnicaId,
    difference_manifest_id: UnicaId,
    difference_digest: Sha256Digest,
    adaptation_decision_id: UnicaId,
);
verification_leaf!(
    SynchronizedTaskUnexpectedVerificationData,
    SynchronizedTaskScope,
    UnexpectedOutcome,
    session_id: UnicaId,
    difference_manifest_id: UnicaId,
    difference_digest: Sha256Digest,
);
verification_leaf!(
    SynchronizedTaskInvalidVerificationData,
    SynchronizedTaskScope,
    InvalidOutcome,
    session_id: UnicaId,
);
verification_leaf!(
    MainSandboxValidVerificationData,
    MainSandboxScope,
    ValidOutcome,
    session_id: UnicaId,
    support_gate_digest: Sha256Digest,
    support_gate_history_evidence: SupportGateHistoryEvidence,
);
verification_leaf!(
    MainSandboxInvalidVerificationData,
    MainSandboxScope,
    InvalidOutcome,
    session_id: UnicaId,
    support_gate_digest: Sha256Digest,
    support_gate_history_evidence: SupportGateHistoryEvidence,
);
verification_leaf!(
    MainIntegrationValidVerificationData,
    MainIntegrationScope,
    ValidOutcome,
    session_id: UnicaId,
    merge_receipt_id: UnicaId,
    integration_set_digest: Sha256Digest,
    support_gate_digest: Sha256Digest,
    support_gate_history_evidence: SupportGateHistoryEvidence,
);
verification_leaf!(
    MainIntegrationInvalidVerificationData,
    MainIntegrationScope,
    InvalidOutcome,
    session_id: UnicaId,
    merge_receipt_id: UnicaId,
    integration_set_digest: Sha256Digest,
    support_gate_digest: Sha256Digest,
    support_gate_history_evidence: SupportGateHistoryEvidence,
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub(crate) enum MergeVerificationData {
    LocalCheckpointValid(LocalCheckpointValidVerificationData),
    LocalCheckpointInvalid(LocalCheckpointInvalidVerificationData),
    SynchronizedTaskEquivalent(SynchronizedTaskEquivalentVerificationData),
    SynchronizedTaskAdapted(SynchronizedTaskAdaptedVerificationData),
    SynchronizedTaskUnexpected(SynchronizedTaskUnexpectedVerificationData),
    SynchronizedTaskInvalid(SynchronizedTaskInvalidVerificationData),
    MainSandboxValid(MainSandboxValidVerificationData),
    MainSandboxInvalid(MainSandboxInvalidVerificationData),
    MainIntegrationValid(MainIntegrationValidVerificationData),
    MainIntegrationInvalid(MainIntegrationInvalidVerificationData),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum VerificationObservationInputFailureEvidence {
    VerificationIdMismatch,
    InvalidSelectedObjectFingerprints(MergeResultContractError),
}

/// Failed verifier-input construction retains the exact configured batch and
/// every candidate value. In the main-integration branch that batch owns the
/// concrete non-Clone consumed lineage.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct VerificationObservationInputBlockedAuthority<Scope> {
    verification_id: UnicaId,
    canonical_delta_digest: Sha256Digest,
    validation_receipts: ConfiguredValidationReceiptBatchAuthority<Scope>,
    support_audit_digest: Sha256Digest,
    selected_object_fingerprints: Vec<SelectedObjectFingerprint>,
    failure: VerificationObservationInputFailureEvidence,
}

pub(crate) type MainIntegrationVerificationObservationInputBlockedAuthority =
    VerificationObservationInputBlockedAuthority<MainIntegrationValidationExecutionScope>;

impl<Scope> VerificationObservationInputBlockedAuthority<Scope> {
    fn new(
        verification_id: UnicaId,
        canonical_delta_digest: Sha256Digest,
        validation_receipts: ConfiguredValidationReceiptBatchAuthority<Scope>,
        support_audit_digest: Sha256Digest,
        selected_object_fingerprints: Vec<SelectedObjectFingerprint>,
        failure: VerificationObservationInputFailureEvidence,
    ) -> Box<Self> {
        Box::new(Self {
            verification_id,
            canonical_delta_digest,
            validation_receipts,
            support_audit_digest,
            selected_object_fingerprints,
            failure,
        })
    }

    pub(crate) fn into_recovery_parts(
        self: Box<Self>,
    ) -> (
        UnicaId,
        Sha256Digest,
        ConfiguredValidationReceiptBatchAuthority<Scope>,
        Sha256Digest,
        Vec<SelectedObjectFingerprint>,
        VerificationObservationInputFailureEvidence,
    ) {
        let Self {
            verification_id,
            canonical_delta_digest,
            validation_receipts,
            support_audit_digest,
            selected_object_fingerprints,
            failure,
        } = *self;
        (
            verification_id,
            canonical_delta_digest,
            validation_receipts,
            support_audit_digest,
            selected_object_fingerprints,
            failure,
        )
    }
}

/// Scope-typed verifier result plus its one-shot configured-check batch.
/// The scope marker is minted by the corresponding selection constructor and
/// survives execution, so observation APIs cannot accept another scope.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct VerificationObservationInputAuthority<Scope> {
    verification_id: UnicaId,
    canonical_delta_digest: Sha256Digest,
    validation_receipts: ConfiguredValidationReceiptBatchAuthority<Scope>,
    support_audit_digest: Sha256Digest,
    selected_object_fingerprints: SelectedObjectFingerprints,
}

pub(crate) type LocalCheckpointVerificationObservationInputAuthority =
    VerificationObservationInputAuthority<LocalCheckpointValidationExecutionScope>;
pub(crate) type ResolvedTaskVerificationObservationInputAuthority =
    VerificationObservationInputAuthority<ResolvedTaskValidationExecutionScope>;
pub(crate) type MainSandboxVerificationObservationInputAuthority =
    VerificationObservationInputAuthority<MainSandboxValidationExecutionScope>;
pub(crate) type MainIntegrationVerificationObservationInputAuthority =
    VerificationObservationInputAuthority<MainIntegrationValidationExecutionScope>;

impl<Scope> VerificationObservationInputAuthority<Scope> {
    pub(crate) fn from_verifier_adapter(
        verification_id: UnicaId,
        canonical_delta_digest: Sha256Digest,
        validation_receipts: ConfiguredValidationReceiptBatchAuthority<Scope>,
        support_audit_digest: Sha256Digest,
        selected_object_fingerprints: Vec<SelectedObjectFingerprint>,
    ) -> Result<Self, Box<VerificationObservationInputBlockedAuthority<Scope>>> {
        if validation_receipts.selection.subject.verification_id() != &verification_id {
            return Err(VerificationObservationInputBlockedAuthority::new(
                verification_id,
                canonical_delta_digest,
                validation_receipts,
                support_audit_digest,
                selected_object_fingerprints,
                VerificationObservationInputFailureEvidence::VerificationIdMismatch,
            ));
        }
        if selected_object_fingerprints.len() > MAX_RESULT_ITEMS
            || selected_object_fingerprints
                .windows(2)
                .any(|pair| pair[0].target >= pair[1].target)
        {
            return Err(VerificationObservationInputBlockedAuthority::new(
                verification_id,
                canonical_delta_digest,
                validation_receipts,
                support_audit_digest,
                selected_object_fingerprints,
                VerificationObservationInputFailureEvidence::InvalidSelectedObjectFingerprints(
                    MergeResultContractError(
                        "selected object fingerprints must be canonical and unique by target",
                    ),
                ),
            ));
        }
        Ok(Self {
            verification_id,
            canonical_delta_digest,
            validation_receipts,
            support_audit_digest,
            selected_object_fingerprints: SelectedObjectFingerprints(selected_object_fingerprints),
        })
    }

    fn into_common(
        self,
        context_matches: impl FnOnce(
            &ConfiguredValidationExecutionSelectionAuthority<Scope>,
            &UnicaId,
        ) -> bool,
    ) -> Result<CommonVerificationObservationAuthority, MergeResultContractError> {
        if !context_matches(&self.validation_receipts.selection, &self.verification_id) {
            return Err(MergeResultContractError(
                "configured validation batch belongs to another verification context",
            ));
        }
        Ok(CommonVerificationObservationAuthority {
            verification_id: self.verification_id,
            canonical_delta_digest: self.canonical_delta_digest,
            validation_receipt_ids: self.validation_receipts.receipt_ids,
            support_audit_digest: self.support_audit_digest,
            selected_object_fingerprints: self.selected_object_fingerprints,
        })
    }
}

impl VerificationObservationInputAuthority<LocalCheckpointValidationExecutionScope> {
    fn into_common_local(
        self,
    ) -> Result<CommonVerificationObservationAuthority, MergeResultContractError> {
        self.into_common(|selection, verification_id| {
            selection.matches_local_checkpoint(verification_id)
        })
    }
}

impl VerificationObservationInputAuthority<ResolvedTaskValidationExecutionScope> {
    fn into_common_resolved_task(
        self,
        session: &MergeSessionData,
    ) -> Result<CommonVerificationObservationAuthority, MergeResultContractError> {
        self.into_common(|selection, verification_id| {
            selection.matches_resolved_task(session, verification_id)
        })
    }
}

impl VerificationObservationInputAuthority<MainSandboxValidationExecutionScope> {
    fn into_common_main_sandbox(
        self,
        planning: &ValidatedRepositoryPlanSessionProjection,
    ) -> Result<CommonVerificationObservationAuthority, MergeResultContractError> {
        self.into_common(|selection, verification_id| {
            selection.matches_main_sandbox(planning, verification_id)
        })
    }
}

impl VerificationObservationInputAuthority<MainIntegrationValidationExecutionScope> {
    fn into_common_main_integration(
        self,
    ) -> (
        CommonVerificationObservationAuthority,
        MainIntegrationVerificationLineage,
    ) {
        let Self {
            verification_id,
            canonical_delta_digest,
            validation_receipts,
            support_audit_digest,
            selected_object_fingerprints,
        } = self;
        let ConfiguredValidationReceiptBatchAuthority {
            selection,
            receipt_ids,
        } = validation_receipts;
        let lineage = selection.into_main_integration_lineage();
        (
            CommonVerificationObservationAuthority {
                verification_id,
                canonical_delta_digest,
                validation_receipt_ids: receipt_ids,
                support_audit_digest,
                selected_object_fingerprints,
            },
            lineage,
        )
    }
}

#[derive(Debug, PartialEq, Eq)]
struct CommonVerificationObservationAuthority {
    verification_id: UnicaId,
    canonical_delta_digest: Sha256Digest,
    validation_receipt_ids: ValidationReceiptIds,
    support_audit_digest: Sha256Digest,
    selected_object_fingerprints: SelectedObjectFingerprints,
}

#[derive(Debug, PartialEq, Eq)]
enum LocalCheckpointVerificationOutcomeAuthority {
    Valid { checkpoint_id: UnicaId },
    Invalid,
}

/// Atomic local verifier observation. The caller cannot provide a count or
/// verification digest; a successful checkpoint ID is physically absent from
/// the invalid branch.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct LocalCheckpointVerificationObservationAuthority {
    common: CommonVerificationObservationAuthority,
    outcome: LocalCheckpointVerificationOutcomeAuthority,
}

impl LocalCheckpointVerificationObservationAuthority {
    pub(crate) fn valid_from_verifier_adapter(
        input: LocalCheckpointVerificationObservationInputAuthority,
        checkpoint_id: UnicaId,
    ) -> Result<Self, MergeResultContractError> {
        Ok(Self {
            common: input.into_common_local()?,
            outcome: LocalCheckpointVerificationOutcomeAuthority::Valid { checkpoint_id },
        })
    }

    pub(crate) fn invalid_from_verifier_adapter(
        input: LocalCheckpointVerificationObservationInputAuthority,
    ) -> Result<Self, MergeResultContractError> {
        Ok(Self {
            common: input.into_common_local()?,
            outcome: LocalCheckpointVerificationOutcomeAuthority::Invalid,
        })
    }
}

/// Successful immutable checkpoint proof. Both supported update and main
/// integration copy checkpoint identity/digest only from this authority.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ValidatedLocalCheckpointVerificationAuthority {
    data: MergeVerificationData,
    checkpoint_id: UnicaId,
    verification_digest: Sha256Digest,
}

impl ValidatedLocalCheckpointVerificationAuthority {
    pub(crate) fn data(&self) -> MergeVerificationData {
        self.data.clone()
    }

    pub(crate) const fn checkpoint_id(&self) -> &UnicaId {
        &self.checkpoint_id
    }

    pub(crate) const fn verification_digest(&self) -> &Sha256Digest {
        &self.verification_digest
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ValidatedSynchronizedCheckpointVerificationAuthority {
    data: MergeVerificationData,
    checkpoint_id: UnicaId,
    verification_digest: Sha256Digest,
}

impl ValidatedSynchronizedCheckpointVerificationAuthority {
    pub(crate) fn data(&self) -> MergeVerificationData {
        self.data.clone()
    }

    pub(crate) const fn checkpoint_id(&self) -> &UnicaId {
        &self.checkpoint_id
    }

    pub(crate) const fn verification_digest(&self) -> &Sha256Digest {
        &self.verification_digest
    }
}

#[derive(Debug, PartialEq, Eq)]
enum SynchronizedVerificationOutcomeAuthority {
    Equivalent { checkpoint_id: UnicaId },
    Invalid,
}

/// Atomic synchronized verifier observation bound to one resolved task
/// session. Reusing it with another session is rejected by the producer.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct SynchronizedVerificationObservationAuthority {
    session_id: UnicaId,
    resolved_session_digest: Sha256Digest,
    common: CommonVerificationObservationAuthority,
    outcome: SynchronizedVerificationOutcomeAuthority,
}

impl SynchronizedVerificationObservationAuthority {
    pub(crate) fn equivalent_from_verifier_adapter(
        session: &MergeSessionData,
        input: ResolvedTaskVerificationObservationInputAuthority,
        checkpoint_id: UnicaId,
    ) -> Result<Self, MergeResultContractError> {
        Self::from_verifier_adapter(
            session,
            input,
            SynchronizedVerificationOutcomeAuthority::Equivalent { checkpoint_id },
        )
    }

    pub(crate) fn invalid_from_verifier_adapter(
        session: &MergeSessionData,
        input: ResolvedTaskVerificationObservationInputAuthority,
    ) -> Result<Self, MergeResultContractError> {
        Self::from_verifier_adapter(
            session,
            input,
            SynchronizedVerificationOutcomeAuthority::Invalid,
        )
    }

    fn from_verifier_adapter(
        session: &MergeSessionData,
        input: ResolvedTaskVerificationObservationInputAuthority,
        outcome: SynchronizedVerificationOutcomeAuthority,
    ) -> Result<Self, MergeResultContractError> {
        if !matches!(
            session,
            MergeSessionData::SupportedUpdateResolved(_)
                | MergeSessionData::ResolvedReplayResolved(_)
        ) {
            return Err(MergeResultContractError(
                "synchronized verification requires a resolved task session",
            ));
        }
        Ok(Self {
            session_id: session.session_id().clone(),
            resolved_session_digest: session
                .resolved_session_digest()
                .expect("resolved branch checked above")
                .clone(),
            common: input.into_common_resolved_task(session)?,
            outcome,
        })
    }

    fn matches(&self, session: &MergeSessionData) -> bool {
        self.session_id == *session.session_id()
            && session.resolved_session_digest() == Some(&self.resolved_session_digest)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MainVerificationOutcomeAuthority {
    Valid,
    Invalid,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct MainSandboxVerificationObservationAuthority {
    session_id: UnicaId,
    resolved_session_digest: Sha256Digest,
    support_gate_id: UnicaId,
    support_gate_digest: Sha256Digest,
    support_gate_history_evidence_digest: Sha256Digest,
    common: CommonVerificationObservationAuthority,
    outcome: MainVerificationOutcomeAuthority,
}

impl MainSandboxVerificationObservationAuthority {
    pub(crate) fn valid_from_verifier_adapter(
        planning: &ValidatedRepositoryPlanSessionProjection,
        input: MainSandboxVerificationObservationInputAuthority,
    ) -> Result<Self, MergeResultContractError> {
        Self::from_verifier_adapter(planning, input, MainVerificationOutcomeAuthority::Valid)
    }

    pub(crate) fn invalid_from_verifier_adapter(
        planning: &ValidatedRepositoryPlanSessionProjection,
        input: MainSandboxVerificationObservationInputAuthority,
    ) -> Result<Self, MergeResultContractError> {
        Self::from_verifier_adapter(planning, input, MainVerificationOutcomeAuthority::Invalid)
    }

    fn from_verifier_adapter(
        planning: &ValidatedRepositoryPlanSessionProjection,
        input: MainSandboxVerificationObservationInputAuthority,
        outcome: MainVerificationOutcomeAuthority,
    ) -> Result<Self, MergeResultContractError> {
        Ok(Self {
            session_id: planning.merge_session_id().clone(),
            resolved_session_digest: planning.resolved_session_digest().clone(),
            support_gate_id: planning.support_gate_id().clone(),
            support_gate_digest: planning.support_gate_digest().clone(),
            support_gate_history_evidence_digest: planning
                .support_gate_history_evidence()
                .evidence_digest()
                .clone(),
            common: input.into_common_main_sandbox(planning)?,
            outcome,
        })
    }

    fn matches(&self, planning: &ValidatedRepositoryPlanSessionProjection) -> bool {
        self.session_id == *planning.merge_session_id()
            && self.resolved_session_digest == *planning.resolved_session_digest()
            && self.support_gate_id == *planning.support_gate_id()
            && self.support_gate_digest == *planning.support_gate_digest()
            && self.support_gate_history_evidence_digest
                == *planning.support_gate_history_evidence().evidence_digest()
    }
}

/// Successful sandbox verification plus the exact main-session/preflight
/// lineage that repository lock planning must consume.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ValidatedMainSandboxVerificationAuthority {
    planning: ValidatedRepositoryPlanSessionProjection,
    verification: MainSandboxValidVerificationData,
}

impl ValidatedMainSandboxVerificationAuthority {
    pub(crate) fn data(&self) -> MergeVerificationData {
        MergeVerificationData::MainSandboxValid(self.verification.clone())
    }

    pub(crate) fn verification_id(&self) -> &UnicaId {
        &self.verification.verification_id
    }

    pub(crate) fn verification_digest(&self) -> &Sha256Digest {
        &self.verification.verification_digest
    }

    pub(crate) fn merge_session_id(&self) -> &UnicaId {
        self.planning.merge_session_id()
    }

    pub(crate) fn resolved_session_digest(&self) -> &Sha256Digest {
        self.planning.resolved_session_digest()
    }

    pub(crate) fn comparison_id(&self) -> &UnicaId {
        self.planning.comparison_id()
    }

    pub(crate) fn support_gate_id(&self) -> &UnicaId {
        self.planning.support_gate_id()
    }

    pub(crate) fn support_gate_digest(&self) -> &Sha256Digest {
        self.planning.support_gate_digest()
    }

    pub(crate) fn support_gate_history_evidence(&self) -> &SupportGateHistoryEvidence {
        self.planning.support_gate_history_evidence()
    }

    pub(crate) fn settings_digest(&self) -> &Sha256Digest {
        self.planning.settings_digest()
    }

    pub(crate) fn ordinary_result_artifact_id(&self) -> &UnicaId {
        self.planning.ordinary_result_artifact_id()
    }

    pub(crate) fn result_digest(&self) -> &Sha256Digest {
        self.planning.result_digest()
    }

    pub(crate) fn applied_decision_ids(&self) -> &[UnicaId] {
        self.planning.applied_decision_ids()
    }

    pub(crate) fn planning(&self) -> &ValidatedRepositoryPlanSessionProjection {
        &self.planning
    }

    pub(crate) fn into_planning(self) -> ValidatedRepositoryPlanSessionProjection {
        self.planning
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct MainIntegrationVerifierObservationEvidenceAuthority {
    session_id: UnicaId,
    resolved_session_digest: Sha256Digest,
    merge_receipt_id: UnicaId,
    integration_set_digest: Sha256Digest,
    support_gate_id: UnicaId,
    support_gate_digest: Sha256Digest,
    support_gate_history_evidence_digest: Sha256Digest,
    result_fingerprint: Sha256Digest,
    common: CommonVerificationObservationAuthority,
    outcome: MainVerificationOutcomeAuthority,
}

/// The verifier observation owns the concrete consumed lineage selected for
/// execution. No second lineage can be injected at final result production.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct MainIntegrationVerificationObservationAuthority {
    lineage: MainIntegrationVerificationLineage,
    evidence: MainIntegrationVerifierObservationEvidenceAuthority,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MainIntegrationVerificationFailureEvidence {
    ValidConstructorReceivedInvalidOutcome,
    InvalidConstructorReceivedValidOutcome,
    DigestError(MergeResultContractError),
}

/// A failed main-integration transition retains the complete owned stage for
/// retry/recovery instead of returning an authority-erasing scalar error.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct MainIntegrationVerificationBlockedAuthority {
    lineage: MainIntegrationVerificationLineage,
    observation: MainIntegrationVerifierObservationEvidenceAuthority,
    failure: MainIntegrationVerificationFailureEvidence,
}

impl MainIntegrationVerificationBlockedAuthority {
    fn new(
        lineage: MainIntegrationVerificationLineage,
        observation: MainIntegrationVerifierObservationEvidenceAuthority,
        failure: MainIntegrationVerificationFailureEvidence,
    ) -> Box<Self> {
        Box::new(Self {
            lineage,
            observation,
            failure,
        })
    }

    pub(crate) fn failure(&self) -> &MainIntegrationVerificationFailureEvidence {
        &self.failure
    }

    pub(crate) fn into_recovery_parts(
        self: Box<Self>,
    ) -> (
        ValidatedConsumedOriginalMergeLineageAuthority,
        MainIntegrationVerifierObservationEvidenceAuthority,
        MainIntegrationVerificationFailureEvidence,
    ) {
        let Self {
            lineage,
            observation,
            failure,
        } = *self;
        (*lineage.0, observation, failure)
    }
}

impl MainIntegrationVerificationObservationAuthority {
    pub(crate) fn valid_from_verifier_adapter(
        input: MainIntegrationVerificationObservationInputAuthority,
    ) -> Self {
        Self::from_verifier_adapter(input, MainVerificationOutcomeAuthority::Valid)
    }

    pub(crate) fn invalid_from_verifier_adapter(
        input: MainIntegrationVerificationObservationInputAuthority,
    ) -> Self {
        Self::from_verifier_adapter(input, MainVerificationOutcomeAuthority::Invalid)
    }

    fn from_verifier_adapter(
        input: MainIntegrationVerificationObservationInputAuthority,
        outcome: MainVerificationOutcomeAuthority,
    ) -> Self {
        let (common, lineage) = input.into_common_main_integration();
        let receipt = lineage.receipt();
        let evidence = MainIntegrationVerifierObservationEvidenceAuthority {
            session_id: receipt.session_id().clone(),
            resolved_session_digest: receipt.resolved_session_digest().clone(),
            merge_receipt_id: receipt.merge_receipt_id().clone(),
            integration_set_digest: receipt.integration_set_digest().clone(),
            support_gate_id: receipt.support_gate_id().clone(),
            support_gate_digest: receipt.support_gate_digest().clone(),
            support_gate_history_evidence_digest: receipt
                .support_gate_history_evidence()
                .evidence_digest()
                .clone(),
            result_fingerprint: receipt.result_fingerprint().clone(),
            common,
            outcome,
        };
        Self { lineage, evidence }
    }
}

/// Linear carrier for the exact owned-lock original merge lineage from
/// validation selection through observation and finalization. It proves
/// ownership continuity, not a successful outcome; only the validated success
/// authority can convert it into commit lineage.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct MainIntegrationVerificationLineage(
    Box<ValidatedConsumedOriginalMergeLineageAuthority>,
);

impl MainIntegrationVerificationLineage {
    fn from_consumed(lineage: ValidatedConsumedOriginalMergeLineageAuthority) -> Self {
        Self(Box::new(lineage))
    }

    fn receipt(&self) -> &ValidatedOriginalMergeReceiptAuthority {
        self.0.receipt()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ValidatedMainIntegrationVerificationAuthority {
    lineage: MainIntegrationVerificationLineage,
    verification: MainIntegrationValidVerificationData,
}

impl ValidatedMainIntegrationVerificationAuthority {
    pub(crate) fn data(&self) -> MergeVerificationData {
        MergeVerificationData::MainIntegrationValid(self.verification.clone())
    }

    pub(crate) fn verification_id(&self) -> &UnicaId {
        &self.verification.verification_id
    }

    pub(crate) fn verification_digest(&self) -> &Sha256Digest {
        &self.verification.verification_digest
    }

    pub(crate) fn merge_receipt_id(&self) -> &UnicaId {
        self.lineage.receipt().merge_receipt_id()
    }

    pub(crate) fn session_id(&self) -> &UnicaId {
        self.lineage.receipt().session_id()
    }

    pub(crate) fn resolved_session_digest(&self) -> &Sha256Digest {
        self.lineage.receipt().resolved_session_digest()
    }

    pub(crate) fn result_fingerprint(&self) -> &Sha256Digest {
        self.lineage.receipt().result_fingerprint()
    }

    pub(crate) fn repository_history_cursor(&self) -> &RepositoryHistoryCursor {
        self.lineage.receipt().repository_history_cursor()
    }

    pub(crate) fn rollback_checkpoint_id(&self) -> &UnicaId {
        self.lineage.receipt().rollback_checkpoint_id()
    }

    pub(crate) fn plan_id(&self) -> &UnicaId {
        self.lineage.receipt().plan_id()
    }

    pub(crate) fn plan_digest(&self) -> &Sha256Digest {
        self.lineage.receipt().plan_digest()
    }

    pub(crate) fn support_gate_id(&self) -> &UnicaId {
        self.lineage.receipt().support_gate_id()
    }

    pub(crate) fn support_gate_digest(&self) -> &Sha256Digest {
        self.lineage.receipt().support_gate_digest()
    }

    pub(crate) fn support_gate_history_evidence(&self) -> &SupportGateHistoryEvidence {
        self.lineage.receipt().support_gate_history_evidence()
    }

    pub(crate) fn integration_set_digest(&self) -> &Sha256Digest {
        self.lineage.receipt().integration_set_digest()
    }

    pub(crate) fn integration_set_id(&self) -> &UnicaId {
        self.lineage.receipt().integration_set_id()
    }

    pub(crate) fn lock_set_id(&self) -> &UnicaId {
        self.lineage.receipt().lock_set_id()
    }

    pub(crate) fn lock_set_digest(&self) -> &Sha256Digest {
        self.lineage.receipt().lock_set_digest()
    }

    pub(crate) fn applied_decision_ids(&self) -> &[UnicaId] {
        self.lineage.receipt().applied_decision_ids()
    }

    pub(crate) fn lock_projection(&self) -> &ValidatedOriginalMergeLockProjection {
        self.lineage.receipt().lock_projection()
    }

    #[cfg(test)]
    pub(crate) fn into_lock_projection(self) -> ValidatedOriginalMergeLockProjection {
        let receipt = self.lineage.0.receipt;
        match receipt.lineage {
            OriginalMergeReceiptLineage::Production(consumed) => {
                let consumed = *consumed;
                consumed.preintent.attempt.rollback.source
            }
            OriginalMergeReceiptLineage::Fixture(projection) => *projection,
        }
    }

    pub(crate) fn into_commit_lineage(self) -> ValidatedMainIntegrationCommitLineageAuthority {
        let lineage = *self.lineage.0;
        ValidatedMainIntegrationCommitLineageAuthority {
            lineage,
            verification: self.verification,
        }
    }
}

/// An invalid main-integration result is a recovery authority, not a terminal
/// wire value. It keeps the exact consumed receipt/B1/gate lineage and the
/// verifier observation while allowing a read-only wire projection.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ValidatedMainIntegrationInvalidAuthority {
    lineage: MainIntegrationVerificationLineage,
    observation: MainIntegrationVerifierObservationEvidenceAuthority,
    verification: MainIntegrationInvalidVerificationData,
}

impl ValidatedMainIntegrationInvalidAuthority {
    pub(crate) fn data(&self) -> MergeVerificationData {
        MergeVerificationData::MainIntegrationInvalid(self.verification.clone())
    }

    pub(crate) fn into_recovery_parts(
        self,
    ) -> (
        ValidatedConsumedOriginalMergeLineageAuthority,
        MainIntegrationVerifierObservationEvidenceAuthority,
        MainIntegrationInvalidVerificationData,
    ) {
        let Self {
            lineage,
            observation,
            verification,
        } = self;
        (*lineage.0, observation, verification)
    }
}

/// Non-erasing commit input: verified data remains owned together with the
/// consumed gate, original receipt, B1 locks, plan, and authoritative consumed
/// observation.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ValidatedMainIntegrationCommitLineageAuthority {
    lineage: ValidatedConsumedOriginalMergeLineageAuthority,
    verification: MainIntegrationValidVerificationData,
}

impl ValidatedMainIntegrationCommitLineageAuthority {
    fn receipt(&self) -> &ValidatedOriginalMergeReceiptAuthority {
        self.lineage.receipt()
    }

    pub(crate) fn verification_id(&self) -> &UnicaId {
        &self.verification.verification_id
    }

    pub(crate) fn verification_digest(&self) -> &Sha256Digest {
        &self.verification.verification_digest
    }

    pub(crate) fn merge_receipt_id(&self) -> &UnicaId {
        self.receipt().merge_receipt_id()
    }

    pub(crate) fn session_id(&self) -> &UnicaId {
        self.receipt().session_id()
    }

    pub(crate) fn resolved_session_digest(&self) -> &Sha256Digest {
        self.receipt().resolved_session_digest()
    }

    pub(crate) fn merge_receipt_cursor(&self) -> &RepositoryHistoryCursor {
        self.receipt().repository_history_cursor()
    }

    pub(crate) fn result_fingerprint(&self) -> &Sha256Digest {
        self.receipt().result_fingerprint()
    }

    pub(crate) fn support_gate_id(&self) -> &UnicaId {
        self.receipt().support_gate_id()
    }

    pub(crate) fn support_gate_digest(&self) -> &Sha256Digest {
        self.receipt().support_gate_digest()
    }

    pub(crate) fn support_gate_history_evidence(&self) -> &SupportGateHistoryEvidence {
        self.receipt().support_gate_history_evidence()
    }

    pub(crate) fn plan_id(&self) -> &UnicaId {
        self.receipt().plan_id()
    }

    pub(crate) fn plan_digest(&self) -> &Sha256Digest {
        self.receipt().plan_digest()
    }

    pub(crate) fn integration_set_id(&self) -> &UnicaId {
        self.receipt().integration_set_id()
    }

    pub(crate) fn integration_set_digest(&self) -> &Sha256Digest {
        self.receipt().integration_set_digest()
    }

    pub(crate) fn lock_set_id(&self) -> &UnicaId {
        self.receipt().lock_set_id()
    }

    pub(crate) fn lock_set_digest(&self) -> &Sha256Digest {
        self.receipt().lock_set_digest()
    }

    pub(crate) fn rollback_checkpoint_id(&self) -> &UnicaId {
        self.receipt().rollback_checkpoint_id()
    }

    pub(crate) fn journaled_lock_receipts(&self) -> &[JournaledRepositoryLock] {
        self.receipt()
            .lock_projection()
            .journaled_lock_receipts()
            .expect("commit lineage always retains production B1 receipts")
    }

    pub(crate) fn lock_plan(&self) -> &LockPlanData {
        self.receipt().lock_projection().plan()
    }

    pub(crate) fn root_reread_capability_id(&self) -> &CapabilityRowId {
        self.receipt()
            .lock_projection()
            .root_reread_capability_id()
            .expect("commit lineage always retains the production B1 root reread")
    }

    pub(crate) fn consumed_gate(&self) -> &ConsumedSupportGateAuthority {
        self.lineage.receipt.consumed_gate()
    }

    pub(crate) fn consumed_observation(&self) -> &ObservedConsumedSupportGateAuthority {
        &self.lineage.observed
    }
}

/// Fresh authoritative consumed-state observation which owns the exact
/// verified commit lineage it can guard. The lineage cannot be replaced by a
/// field-equal value between resolution and the post-merge guard.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ResolvedCommitLineageConsumedSupportGateAuthority {
    lineage: ValidatedMainIntegrationCommitLineageAuthority,
    observed: ObservedConsumedSupportGateAuthority,
}

impl ResolvedCommitLineageConsumedSupportGateAuthority {
    pub(crate) fn resolve(
        lineage: ValidatedMainIntegrationCommitLineageAuthority,
        resolver: &mut dyn ConsumedSupportGateStateResolver,
    ) -> Result<
        Self,
        Box<
            ConsumedSupportGateResolutionBlockedAuthority<
                ValidatedMainIntegrationCommitLineageAuthority,
            >,
        >,
    > {
        let binding = ConsumedSupportGateObservationBinding::from_consumed(lineage.consumed_gate());
        let (lineage, observed) =
            ObservedConsumedSupportGateAuthority::resolve_binding(lineage, binding, resolver)?;
        Ok(Self { lineage, observed })
    }

    pub(crate) fn lineage(&self) -> &ValidatedMainIntegrationCommitLineageAuthority {
        &self.lineage
    }

    pub(crate) fn consumed_gate_observation(&self) -> &ObservedConsumedSupportGateAuthority {
        &self.observed
    }
}

impl JsonSchema for MergeVerificationData {
    fn schema_name() -> Cow<'static, str> {
        "MergeVerificationData".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<LocalCheckpointValidVerificationData>(),
            generator.subschema_for::<LocalCheckpointInvalidVerificationData>(),
            generator.subschema_for::<SynchronizedTaskEquivalentVerificationData>(),
            generator.subschema_for::<SynchronizedTaskAdaptedVerificationData>(),
            generator.subschema_for::<SynchronizedTaskUnexpectedVerificationData>(),
            generator.subschema_for::<SynchronizedTaskInvalidVerificationData>(),
            generator.subschema_for::<MainSandboxValidVerificationData>(),
            generator.subschema_for::<MainSandboxInvalidVerificationData>(),
            generator.subschema_for::<MainIntegrationValidVerificationData>(),
            generator.subschema_for::<MainIntegrationInvalidVerificationData>(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct MergeVerificationDigestRecord {
    verification_id: UnicaId,
    scope: VerificationScopeDigest,
    outcome: VerificationOutcomeDigest,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<UnicaId>,
    canonical_delta_digest: Sha256Digest,
    #[serde(skip_serializing_if = "Option::is_none")]
    checkpoint_id: Option<UnicaId>,
    validation_receipt_ids: ValidationReceiptIds,
    support_audit_digest: Sha256Digest,
    selected_object_fingerprints: SelectedObjectFingerprints,
    #[serde(skip_serializing_if = "Option::is_none")]
    difference_manifest_id: Option<UnicaId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    difference_digest: Option<Sha256Digest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    adaptation_decision_id: Option<UnicaId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    merge_receipt_id: Option<UnicaId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    integration_set_digest: Option<Sha256Digest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    support_gate_digest: Option<Sha256Digest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    support_gate_history_evidence: Option<SupportGateHistoryEvidence>,
}

impl contract_digest_record_sealed::Sealed for MergeVerificationDigestRecord {}
impl ContractDigestRecord for MergeVerificationDigestRecord {}

#[derive(Debug)]
struct DigestedMainIntegrationVerificationAuthority {
    lineage: MainIntegrationVerificationLineage,
    observation: MainIntegrationVerifierObservationEvidenceAuthority,
    record: MergeVerificationDigestRecord,
    verification_digest: Sha256Digest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
enum VerificationScopeDigest {
    LocalCheckpoint,
    SynchronizedTask,
    MainSandbox,
    MainIntegration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
enum VerificationOutcomeDigest {
    Valid,
    Invalid,
    Equivalent,
    Adapted,
    Unexpected,
}

#[cfg(test)]
struct SynchronizedAdaptedVerificationFixtureInput {
    verification_id: UnicaId,
    session_id: UnicaId,
    canonical_delta_digest: Sha256Digest,
    checkpoint_id: UnicaId,
    difference_manifest_id: UnicaId,
    difference_digest: Sha256Digest,
    validation_receipt_ids: Vec<UnicaId>,
    support_audit_digest: Sha256Digest,
    selected_object_fingerprints: Vec<SelectedObjectFingerprint>,
}

impl MergeVerificationData {
    pub(crate) fn local_checkpoint_valid_from_authority(
        observation: LocalCheckpointVerificationObservationAuthority,
    ) -> Result<ValidatedLocalCheckpointVerificationAuthority, MergeResultContractError> {
        let LocalCheckpointVerificationObservationAuthority {
            common,
            outcome: LocalCheckpointVerificationOutcomeAuthority::Valid { checkpoint_id },
        } = observation
        else {
            return Err(MergeResultContractError(
                "local valid producer requires a valid checkpoint observation",
            ));
        };
        let record = MergeVerificationDigestRecord {
            verification_id: common.verification_id,
            scope: VerificationScopeDigest::LocalCheckpoint,
            outcome: VerificationOutcomeDigest::Valid,
            session_id: None,
            canonical_delta_digest: common.canonical_delta_digest,
            checkpoint_id: Some(checkpoint_id.clone()),
            validation_receipt_ids: common.validation_receipt_ids,
            support_audit_digest: common.support_audit_digest,
            selected_object_fingerprints: common.selected_object_fingerprints,
            difference_manifest_id: None,
            difference_digest: None,
            adaptation_decision_id: None,
            merge_receipt_id: None,
            integration_set_digest: None,
            support_gate_digest: None,
            support_gate_history_evidence: None,
        };
        let verification_digest = merge_digest(&record, "merge-verification digest failed")?;
        let data = Self::LocalCheckpointValid(LocalCheckpointValidVerificationData {
            verification_id: record.verification_id,
            scope: LocalCheckpointScope::Value,
            outcome: ValidOutcome::Value,
            canonical_delta_digest: record.canonical_delta_digest,
            checkpoint_id: record.checkpoint_id.expect("set above"),
            validation_receipt_ids: record.validation_receipt_ids,
            support_audit_digest: record.support_audit_digest,
            selected_object_fingerprints: record.selected_object_fingerprints,
            verification_digest: verification_digest.clone(),
        });
        Ok(ValidatedLocalCheckpointVerificationAuthority {
            data,
            checkpoint_id,
            verification_digest,
        })
    }

    pub(crate) fn local_checkpoint_invalid_from_authority(
        observation: LocalCheckpointVerificationObservationAuthority,
    ) -> Result<Self, MergeResultContractError> {
        let LocalCheckpointVerificationObservationAuthority {
            common,
            outcome: LocalCheckpointVerificationOutcomeAuthority::Invalid,
        } = observation
        else {
            return Err(MergeResultContractError(
                "local invalid producer requires an invalid observation",
            ));
        };
        let record = MergeVerificationDigestRecord {
            verification_id: common.verification_id,
            scope: VerificationScopeDigest::LocalCheckpoint,
            outcome: VerificationOutcomeDigest::Invalid,
            session_id: None,
            canonical_delta_digest: common.canonical_delta_digest,
            checkpoint_id: None,
            validation_receipt_ids: common.validation_receipt_ids,
            support_audit_digest: common.support_audit_digest,
            selected_object_fingerprints: common.selected_object_fingerprints,
            difference_manifest_id: None,
            difference_digest: None,
            adaptation_decision_id: None,
            merge_receipt_id: None,
            integration_set_digest: None,
            support_gate_digest: None,
            support_gate_history_evidence: None,
        };
        let verification_digest = merge_digest(&record, "merge-verification digest failed")?;
        Ok(Self::LocalCheckpointInvalid(
            LocalCheckpointInvalidVerificationData {
                verification_id: record.verification_id,
                scope: LocalCheckpointScope::Value,
                outcome: InvalidOutcome::Value,
                canonical_delta_digest: record.canonical_delta_digest,
                validation_receipt_ids: record.validation_receipt_ids,
                support_audit_digest: record.support_audit_digest,
                selected_object_fingerprints: record.selected_object_fingerprints,
                verification_digest,
            },
        ))
    }

    pub(crate) fn synchronized_equivalent_from_authorities(
        session: &MergeSessionData,
        observation: SynchronizedVerificationObservationAuthority,
    ) -> Result<ValidatedSynchronizedCheckpointVerificationAuthority, MergeResultContractError>
    {
        if !observation.matches(session) {
            return Err(MergeResultContractError(
                "synchronized observation belongs to another resolved session",
            ));
        }
        let SynchronizedVerificationObservationAuthority {
            common,
            outcome: SynchronizedVerificationOutcomeAuthority::Equivalent { checkpoint_id },
            ..
        } = observation
        else {
            return Err(MergeResultContractError(
                "synchronized equivalent producer requires an equivalent observation",
            ));
        };
        let record = MergeVerificationDigestRecord {
            verification_id: common.verification_id,
            scope: VerificationScopeDigest::SynchronizedTask,
            outcome: VerificationOutcomeDigest::Equivalent,
            session_id: Some(session.session_id().clone()),
            canonical_delta_digest: common.canonical_delta_digest,
            checkpoint_id: Some(checkpoint_id.clone()),
            validation_receipt_ids: common.validation_receipt_ids,
            support_audit_digest: common.support_audit_digest,
            selected_object_fingerprints: common.selected_object_fingerprints,
            difference_manifest_id: None,
            difference_digest: None,
            adaptation_decision_id: None,
            merge_receipt_id: None,
            integration_set_digest: None,
            support_gate_digest: None,
            support_gate_history_evidence: None,
        };
        let verification_digest = merge_digest(&record, "merge-verification digest failed")?;
        let data = Self::SynchronizedTaskEquivalent(SynchronizedTaskEquivalentVerificationData {
            verification_id: record.verification_id,
            scope: SynchronizedTaskScope::Value,
            outcome: EquivalentOutcome::Value,
            canonical_delta_digest: record.canonical_delta_digest,
            session_id: record.session_id.expect("set above"),
            checkpoint_id: record.checkpoint_id.expect("set above"),
            validation_receipt_ids: record.validation_receipt_ids,
            support_audit_digest: record.support_audit_digest,
            selected_object_fingerprints: record.selected_object_fingerprints,
            verification_digest: verification_digest.clone(),
        });
        Ok(ValidatedSynchronizedCheckpointVerificationAuthority {
            data,
            checkpoint_id,
            verification_digest,
        })
    }

    pub(crate) fn synchronized_invalid_from_authorities(
        session: &MergeSessionData,
        observation: SynchronizedVerificationObservationAuthority,
    ) -> Result<Self, MergeResultContractError> {
        if !observation.matches(session) {
            return Err(MergeResultContractError(
                "synchronized observation belongs to another resolved session",
            ));
        }
        let SynchronizedVerificationObservationAuthority {
            common,
            outcome: SynchronizedVerificationOutcomeAuthority::Invalid,
            ..
        } = observation
        else {
            return Err(MergeResultContractError(
                "synchronized invalid producer requires an invalid observation",
            ));
        };
        let record = MergeVerificationDigestRecord {
            verification_id: common.verification_id,
            scope: VerificationScopeDigest::SynchronizedTask,
            outcome: VerificationOutcomeDigest::Invalid,
            session_id: Some(session.session_id().clone()),
            canonical_delta_digest: common.canonical_delta_digest,
            checkpoint_id: None,
            validation_receipt_ids: common.validation_receipt_ids,
            support_audit_digest: common.support_audit_digest,
            selected_object_fingerprints: common.selected_object_fingerprints,
            difference_manifest_id: None,
            difference_digest: None,
            adaptation_decision_id: None,
            merge_receipt_id: None,
            integration_set_digest: None,
            support_gate_digest: None,
            support_gate_history_evidence: None,
        };
        let verification_digest = merge_digest(&record, "merge-verification digest failed")?;
        Ok(Self::SynchronizedTaskInvalid(
            SynchronizedTaskInvalidVerificationData {
                verification_id: record.verification_id,
                scope: SynchronizedTaskScope::Value,
                outcome: InvalidOutcome::Value,
                canonical_delta_digest: record.canonical_delta_digest,
                session_id: record.session_id.expect("set above"),
                validation_receipt_ids: record.validation_receipt_ids,
                support_audit_digest: record.support_audit_digest,
                selected_object_fingerprints: record.selected_object_fingerprints,
                verification_digest,
            },
        ))
    }

    pub(crate) fn main_sandbox_valid_from_authorities(
        planning: ValidatedRepositoryPlanSessionProjection,
        observation: MainSandboxVerificationObservationAuthority,
    ) -> Result<ValidatedMainSandboxVerificationAuthority, MergeResultContractError> {
        if !observation.matches(&planning)
            || observation.outcome != MainVerificationOutcomeAuthority::Valid
        {
            return Err(MergeResultContractError(
                "main-sandbox valid observation belongs to another planning scope or outcome",
            ));
        }
        let common = observation.common;
        let record = MergeVerificationDigestRecord {
            verification_id: common.verification_id,
            scope: VerificationScopeDigest::MainSandbox,
            outcome: VerificationOutcomeDigest::Valid,
            session_id: Some(planning.merge_session_id().clone()),
            canonical_delta_digest: common.canonical_delta_digest,
            checkpoint_id: None,
            validation_receipt_ids: common.validation_receipt_ids,
            support_audit_digest: common.support_audit_digest,
            selected_object_fingerprints: common.selected_object_fingerprints,
            difference_manifest_id: None,
            difference_digest: None,
            adaptation_decision_id: None,
            merge_receipt_id: None,
            integration_set_digest: None,
            support_gate_digest: Some(planning.support_gate_digest().clone()),
            support_gate_history_evidence: Some(planning.support_gate_history_evidence().clone()),
        };
        let verification_digest = merge_digest(&record, "merge-verification digest failed")?;
        let verification = MainSandboxValidVerificationData {
            verification_id: record.verification_id,
            scope: MainSandboxScope::Value,
            outcome: ValidOutcome::Value,
            session_id: record.session_id.expect("set above"),
            canonical_delta_digest: record.canonical_delta_digest,
            support_gate_digest: record.support_gate_digest.expect("set above"),
            support_gate_history_evidence: record.support_gate_history_evidence.expect("set above"),
            validation_receipt_ids: record.validation_receipt_ids,
            support_audit_digest: record.support_audit_digest,
            selected_object_fingerprints: record.selected_object_fingerprints,
            verification_digest,
        };
        Ok(ValidatedMainSandboxVerificationAuthority {
            planning,
            verification,
        })
    }

    pub(crate) fn main_sandbox_invalid_from_authorities(
        planning: ValidatedRepositoryPlanSessionProjection,
        observation: MainSandboxVerificationObservationAuthority,
    ) -> Result<Self, MergeResultContractError> {
        if !observation.matches(&planning)
            || observation.outcome != MainVerificationOutcomeAuthority::Invalid
        {
            return Err(MergeResultContractError(
                "main-sandbox invalid observation belongs to another planning scope or outcome",
            ));
        }
        let common = observation.common;
        let record = MergeVerificationDigestRecord {
            verification_id: common.verification_id,
            scope: VerificationScopeDigest::MainSandbox,
            outcome: VerificationOutcomeDigest::Invalid,
            session_id: Some(planning.merge_session_id().clone()),
            canonical_delta_digest: common.canonical_delta_digest,
            checkpoint_id: None,
            validation_receipt_ids: common.validation_receipt_ids,
            support_audit_digest: common.support_audit_digest,
            selected_object_fingerprints: common.selected_object_fingerprints,
            difference_manifest_id: None,
            difference_digest: None,
            adaptation_decision_id: None,
            merge_receipt_id: None,
            integration_set_digest: None,
            support_gate_digest: Some(planning.support_gate_digest().clone()),
            support_gate_history_evidence: Some(planning.support_gate_history_evidence().clone()),
        };
        let verification_digest = merge_digest(&record, "merge-verification digest failed")?;
        Ok(Self::MainSandboxInvalid(
            MainSandboxInvalidVerificationData {
                verification_id: record.verification_id,
                scope: MainSandboxScope::Value,
                outcome: InvalidOutcome::Value,
                session_id: record.session_id.expect("set above"),
                canonical_delta_digest: record.canonical_delta_digest,
                support_gate_digest: record.support_gate_digest.expect("set above"),
                support_gate_history_evidence: record
                    .support_gate_history_evidence
                    .expect("set above"),
                validation_receipt_ids: record.validation_receipt_ids,
                support_audit_digest: record.support_audit_digest,
                selected_object_fingerprints: record.selected_object_fingerprints,
                verification_digest,
            },
        ))
    }

    pub(crate) fn main_integration_valid_from_authorities(
        observation: MainIntegrationVerificationObservationAuthority,
    ) -> Result<
        ValidatedMainIntegrationVerificationAuthority,
        Box<MainIntegrationVerificationBlockedAuthority>,
    > {
        Self::main_integration_valid_from_authorities_using_digest(observation, |record| {
            merge_digest(record, "merge-verification digest failed")
        })
    }

    fn main_integration_valid_from_authorities_using_digest<F>(
        observation: MainIntegrationVerificationObservationAuthority,
        digest: F,
    ) -> Result<
        ValidatedMainIntegrationVerificationAuthority,
        Box<MainIntegrationVerificationBlockedAuthority>,
    >
    where
        F: FnOnce(&MergeVerificationDigestRecord) -> Result<Sha256Digest, MergeResultContractError>,
    {
        let DigestedMainIntegrationVerificationAuthority {
            lineage,
            observation: _,
            record,
            verification_digest,
        } = Self::main_integration_from_authorities_using_digest(
            observation,
            MainVerificationOutcomeAuthority::Valid,
            VerificationOutcomeDigest::Valid,
            digest,
        )?;
        let verification = MainIntegrationValidVerificationData {
            verification_id: record.verification_id,
            scope: MainIntegrationScope::Value,
            outcome: ValidOutcome::Value,
            session_id: record.session_id.expect("set above"),
            canonical_delta_digest: record.canonical_delta_digest,
            merge_receipt_id: record.merge_receipt_id.expect("set above"),
            integration_set_digest: record.integration_set_digest.expect("set above"),
            support_gate_digest: record.support_gate_digest.expect("set above"),
            support_gate_history_evidence: record.support_gate_history_evidence.expect("set above"),
            validation_receipt_ids: record.validation_receipt_ids,
            support_audit_digest: record.support_audit_digest,
            selected_object_fingerprints: record.selected_object_fingerprints,
            verification_digest,
        };
        Ok(ValidatedMainIntegrationVerificationAuthority {
            lineage,
            verification,
        })
    }

    pub(crate) fn main_integration_invalid_from_authorities(
        observation: MainIntegrationVerificationObservationAuthority,
    ) -> Result<
        ValidatedMainIntegrationInvalidAuthority,
        Box<MainIntegrationVerificationBlockedAuthority>,
    > {
        Self::main_integration_invalid_from_authorities_using_digest(observation, |record| {
            merge_digest(record, "merge-verification digest failed")
        })
    }

    fn main_integration_invalid_from_authorities_using_digest<F>(
        observation: MainIntegrationVerificationObservationAuthority,
        digest: F,
    ) -> Result<
        ValidatedMainIntegrationInvalidAuthority,
        Box<MainIntegrationVerificationBlockedAuthority>,
    >
    where
        F: FnOnce(&MergeVerificationDigestRecord) -> Result<Sha256Digest, MergeResultContractError>,
    {
        let DigestedMainIntegrationVerificationAuthority {
            lineage,
            observation,
            record,
            verification_digest,
        } = Self::main_integration_from_authorities_using_digest(
            observation,
            MainVerificationOutcomeAuthority::Invalid,
            VerificationOutcomeDigest::Invalid,
            digest,
        )?;
        Ok(ValidatedMainIntegrationInvalidAuthority {
            lineage,
            observation,
            verification: MainIntegrationInvalidVerificationData {
                verification_id: record.verification_id,
                scope: MainIntegrationScope::Value,
                outcome: InvalidOutcome::Value,
                session_id: record.session_id.expect("set above"),
                canonical_delta_digest: record.canonical_delta_digest,
                merge_receipt_id: record.merge_receipt_id.expect("set above"),
                integration_set_digest: record.integration_set_digest.expect("set above"),
                support_gate_digest: record.support_gate_digest.expect("set above"),
                support_gate_history_evidence: record
                    .support_gate_history_evidence
                    .expect("set above"),
                validation_receipt_ids: record.validation_receipt_ids,
                support_audit_digest: record.support_audit_digest,
                selected_object_fingerprints: record.selected_object_fingerprints,
                verification_digest,
            },
        })
    }

    fn main_integration_from_authorities_using_digest<F>(
        observation: MainIntegrationVerificationObservationAuthority,
        expected_outcome: MainVerificationOutcomeAuthority,
        digest_outcome: VerificationOutcomeDigest,
        digest: F,
    ) -> Result<
        DigestedMainIntegrationVerificationAuthority,
        Box<MainIntegrationVerificationBlockedAuthority>,
    >
    where
        F: FnOnce(&MergeVerificationDigestRecord) -> Result<Sha256Digest, MergeResultContractError>,
    {
        let MainIntegrationVerificationObservationAuthority {
            lineage,
            evidence: observation,
        } = observation;
        if observation.outcome != expected_outcome {
            let failure = match expected_outcome {
                MainVerificationOutcomeAuthority::Valid => {
                    MainIntegrationVerificationFailureEvidence::ValidConstructorReceivedInvalidOutcome
                }
                MainVerificationOutcomeAuthority::Invalid => {
                    MainIntegrationVerificationFailureEvidence::InvalidConstructorReceivedValidOutcome
                }
            };
            return Err(MainIntegrationVerificationBlockedAuthority::new(
                lineage,
                observation,
                failure,
            ));
        }

        let receipt = lineage.receipt();
        let common = &observation.common;
        let record = MergeVerificationDigestRecord {
            verification_id: common.verification_id.clone(),
            scope: VerificationScopeDigest::MainIntegration,
            outcome: digest_outcome,
            session_id: Some(receipt.session_id().clone()),
            canonical_delta_digest: common.canonical_delta_digest.clone(),
            checkpoint_id: None,
            validation_receipt_ids: common.validation_receipt_ids.clone(),
            support_audit_digest: common.support_audit_digest.clone(),
            selected_object_fingerprints: common.selected_object_fingerprints.clone(),
            difference_manifest_id: None,
            difference_digest: None,
            adaptation_decision_id: None,
            merge_receipt_id: Some(receipt.merge_receipt_id().clone()),
            integration_set_digest: Some(receipt.integration_set_digest().clone()),
            support_gate_digest: Some(receipt.support_gate_digest().clone()),
            support_gate_history_evidence: Some(receipt.support_gate_history_evidence().clone()),
        };
        let verification_digest = match digest(&record) {
            Ok(value) => value,
            Err(error) => {
                return Err(MainIntegrationVerificationBlockedAuthority::new(
                    lineage,
                    observation,
                    MainIntegrationVerificationFailureEvidence::DigestError(error),
                ));
            }
        };
        Ok(DigestedMainIntegrationVerificationAuthority {
            lineage,
            observation,
            record,
            verification_digest,
        })
    }

    pub(crate) fn synchronized_unexpected_from_authorities(
        session: &MergeSessionData,
        observation: SynchronizedUnexpectedVerificationObservationAuthority,
    ) -> Result<UnexpectedVerificationCommitAuthority, MergeResultContractError> {
        if !matches!(
            session,
            MergeSessionData::SupportedUpdateResolved(_)
                | MergeSessionData::ResolvedReplayResolved(_)
        ) {
            return Err(MergeResultContractError(
                "synchronized unexpected verification requires a resolved task session",
            ));
        }
        if observation.session_id != *session.session_id()
            || session.resolved_session_digest() != Some(&observation.resolved_session_digest)
        {
            return Err(MergeResultContractError(
                "unexpected observation belongs to another resolved session",
            ));
        }
        let record = MergeVerificationDigestRecord {
            verification_id: observation.verification_id,
            scope: VerificationScopeDigest::SynchronizedTask,
            outcome: VerificationOutcomeDigest::Unexpected,
            session_id: Some(session.session_id().clone()),
            canonical_delta_digest: observation.canonical_delta_digest,
            checkpoint_id: None,
            validation_receipt_ids: observation.validation_receipt_ids,
            support_audit_digest: observation.support_audit_digest,
            selected_object_fingerprints: observation.selected_object_fingerprints,
            difference_manifest_id: Some(observation.difference_manifest_id),
            difference_digest: Some(observation.difference_digest),
            adaptation_decision_id: None,
            merge_receipt_id: None,
            integration_set_digest: None,
            support_gate_digest: None,
            support_gate_history_evidence: None,
        };
        let verification_digest = merge_digest(&record, "merge-verification digest failed")?;
        let verification = SynchronizedTaskUnexpectedVerificationData {
            verification_id: record.verification_id,
            scope: SynchronizedTaskScope::Value,
            outcome: UnexpectedOutcome::Value,
            session_id: record.session_id.expect("set above"),
            canonical_delta_digest: record.canonical_delta_digest,
            difference_manifest_id: record.difference_manifest_id.expect("set above"),
            difference_digest: record.difference_digest.expect("set above"),
            validation_receipt_ids: record.validation_receipt_ids,
            support_audit_digest: record.support_audit_digest,
            selected_object_fingerprints: record.selected_object_fingerprints,
            verification_digest,
        };
        Ok(UnexpectedVerificationCommitAuthority {
            data: Self::SynchronizedTaskUnexpected(verification.clone()),
            current: CurrentUnexpectedVerificationAuthority { verification },
        })
    }

    pub(crate) fn synchronized_adapted_from_authorities(
        session: &MergeSessionData,
        current: CurrentAdaptationDecisionAuthority,
        observation: AdaptedVerificationObservationAuthority,
    ) -> Result<ValidatedSynchronizedCheckpointVerificationAuthority, MergeResultContractError>
    {
        if !matches!(
            session,
            MergeSessionData::SupportedUpdateResolved(_)
                | MergeSessionData::ResolvedReplayResolved(_)
        ) {
            return Err(MergeResultContractError(
                "synchronized adapted verification requires a resolved task session",
            ));
        }
        let unexpected = current.unexpected_verification;
        let adaptation = current.adaptation_decision;
        if unexpected.session_id != *session.session_id()
            || observation.session_id != *session.session_id()
            || session.resolved_session_digest() != Some(&observation.resolved_session_digest)
            || observation.verification_id == unexpected.verification_id
            || adaptation.verification_id != unexpected.verification_id
            || observation.adaptation_decision_id != adaptation.decision_id
            || observation.adaptation_decision_digest != adaptation.adaptation_decision_digest
            || observation.canonical_delta_digest != unexpected.canonical_delta_digest
            || observation.canonical_delta_digest != adaptation.canonical_delta_digest
            || observation.difference_manifest_id != unexpected.difference_manifest_id
            || observation.difference_digest != unexpected.difference_digest
            || observation.difference_digest != adaptation.difference_digest
            || !Self::SynchronizedTaskUnexpected(unexpected.clone()).validates_digest()?
            || !adaptation.validates_digest()?
        {
            return Err(MergeResultContractError(
                "adapted verification disagrees with its current unexpected-session decision lineage",
            ));
        }
        let record = MergeVerificationDigestRecord {
            verification_id: observation.verification_id,
            scope: VerificationScopeDigest::SynchronizedTask,
            outcome: VerificationOutcomeDigest::Adapted,
            session_id: Some(unexpected.session_id),
            canonical_delta_digest: observation.canonical_delta_digest,
            checkpoint_id: Some(observation.checkpoint_id),
            validation_receipt_ids: observation.validation_receipt_ids,
            support_audit_digest: observation.support_audit_digest,
            selected_object_fingerprints: observation.selected_object_fingerprints,
            difference_manifest_id: Some(observation.difference_manifest_id),
            difference_digest: Some(observation.difference_digest),
            adaptation_decision_id: Some(adaptation.decision_id),
            merge_receipt_id: None,
            integration_set_digest: None,
            support_gate_digest: None,
            support_gate_history_evidence: None,
        };
        let verification_digest = merge_digest(&record, "merge-verification digest failed")?;
        let checkpoint_id = record.checkpoint_id.clone().expect("set above");
        let data = Self::SynchronizedTaskAdapted(SynchronizedTaskAdaptedVerificationData {
            verification_id: record.verification_id,
            scope: SynchronizedTaskScope::Value,
            outcome: AdaptedOutcome::Value,
            session_id: record.session_id.expect("set above"),
            canonical_delta_digest: record.canonical_delta_digest,
            checkpoint_id: record.checkpoint_id.expect("set above"),
            difference_manifest_id: record.difference_manifest_id.expect("set above"),
            difference_digest: record.difference_digest.expect("set above"),
            adaptation_decision_id: record.adaptation_decision_id.expect("set above"),
            validation_receipt_ids: record.validation_receipt_ids,
            support_audit_digest: record.support_audit_digest,
            selected_object_fingerprints: record.selected_object_fingerprints,
            verification_digest: verification_digest.clone(),
        });
        Ok(ValidatedSynchronizedCheckpointVerificationAuthority {
            data,
            checkpoint_id,
            verification_digest,
        })
    }

    fn validates_digest(&self) -> Result<bool, MergeResultContractError> {
        let expected = merge_digest(&self.digest_record(), "merge-verification digest failed")?;
        let observed = match self {
            Self::LocalCheckpointValid(value) => &value.verification_digest,
            Self::LocalCheckpointInvalid(value) => &value.verification_digest,
            Self::SynchronizedTaskEquivalent(value) => &value.verification_digest,
            Self::SynchronizedTaskAdapted(value) => &value.verification_digest,
            Self::SynchronizedTaskUnexpected(value) => &value.verification_digest,
            Self::SynchronizedTaskInvalid(value) => &value.verification_digest,
            Self::MainSandboxValid(value) => &value.verification_digest,
            Self::MainSandboxInvalid(value) => &value.verification_digest,
            Self::MainIntegrationValid(value) => &value.verification_digest,
            Self::MainIntegrationInvalid(value) => &value.verification_digest,
        };
        Ok(&expected == observed)
    }

    fn digest_record(&self) -> MergeVerificationDigestRecord {
        macro_rules! record {
            (
                $value:expr, $scope:expr, $outcome:expr,
                session = $session:expr,
                checkpoint = $checkpoint:expr,
                difference = ($difference_manifest:expr, $difference_digest:expr),
                adaptation = $adaptation:expr,
                merge = ($merge_receipt:expr, $integration_set:expr),
                support = ($support_gate:expr, $history:expr)
            ) => {
                MergeVerificationDigestRecord {
                    verification_id: $value.verification_id.clone(),
                    scope: $scope,
                    outcome: $outcome,
                    session_id: $session,
                    canonical_delta_digest: $value.canonical_delta_digest.clone(),
                    checkpoint_id: $checkpoint,
                    validation_receipt_ids: $value.validation_receipt_ids.clone(),
                    support_audit_digest: $value.support_audit_digest.clone(),
                    selected_object_fingerprints: $value.selected_object_fingerprints.clone(),
                    difference_manifest_id: $difference_manifest,
                    difference_digest: $difference_digest,
                    adaptation_decision_id: $adaptation,
                    merge_receipt_id: $merge_receipt,
                    integration_set_digest: $integration_set,
                    support_gate_digest: $support_gate,
                    support_gate_history_evidence: $history,
                }
            };
        }

        match self {
            Self::LocalCheckpointValid(value) => record!(
                value,
                VerificationScopeDigest::LocalCheckpoint,
                VerificationOutcomeDigest::Valid,
                session = None,
                checkpoint = Some(value.checkpoint_id.clone()),
                difference = (None, None),
                adaptation = None,
                merge = (None, None),
                support = (None, None)
            ),
            Self::LocalCheckpointInvalid(value) => record!(
                value,
                VerificationScopeDigest::LocalCheckpoint,
                VerificationOutcomeDigest::Invalid,
                session = None,
                checkpoint = None,
                difference = (None, None),
                adaptation = None,
                merge = (None, None),
                support = (None, None)
            ),
            Self::SynchronizedTaskEquivalent(value) => record!(
                value,
                VerificationScopeDigest::SynchronizedTask,
                VerificationOutcomeDigest::Equivalent,
                session = Some(value.session_id.clone()),
                checkpoint = Some(value.checkpoint_id.clone()),
                difference = (None, None),
                adaptation = None,
                merge = (None, None),
                support = (None, None)
            ),
            Self::SynchronizedTaskAdapted(value) => record!(
                value,
                VerificationScopeDigest::SynchronizedTask,
                VerificationOutcomeDigest::Adapted,
                session = Some(value.session_id.clone()),
                checkpoint = Some(value.checkpoint_id.clone()),
                difference = (
                    Some(value.difference_manifest_id.clone()),
                    Some(value.difference_digest.clone())
                ),
                adaptation = Some(value.adaptation_decision_id.clone()),
                merge = (None, None),
                support = (None, None)
            ),
            Self::SynchronizedTaskUnexpected(value) => record!(
                value,
                VerificationScopeDigest::SynchronizedTask,
                VerificationOutcomeDigest::Unexpected,
                session = Some(value.session_id.clone()),
                checkpoint = None,
                difference = (
                    Some(value.difference_manifest_id.clone()),
                    Some(value.difference_digest.clone())
                ),
                adaptation = None,
                merge = (None, None),
                support = (None, None)
            ),
            Self::SynchronizedTaskInvalid(value) => record!(
                value,
                VerificationScopeDigest::SynchronizedTask,
                VerificationOutcomeDigest::Invalid,
                session = Some(value.session_id.clone()),
                checkpoint = None,
                difference = (None, None),
                adaptation = None,
                merge = (None, None),
                support = (None, None)
            ),
            Self::MainSandboxValid(value) => record!(
                value,
                VerificationScopeDigest::MainSandbox,
                VerificationOutcomeDigest::Valid,
                session = Some(value.session_id.clone()),
                checkpoint = None,
                difference = (None, None),
                adaptation = None,
                merge = (None, None),
                support = (
                    Some(value.support_gate_digest.clone()),
                    Some(value.support_gate_history_evidence.clone())
                )
            ),
            Self::MainSandboxInvalid(value) => record!(
                value,
                VerificationScopeDigest::MainSandbox,
                VerificationOutcomeDigest::Invalid,
                session = Some(value.session_id.clone()),
                checkpoint = None,
                difference = (None, None),
                adaptation = None,
                merge = (None, None),
                support = (
                    Some(value.support_gate_digest.clone()),
                    Some(value.support_gate_history_evidence.clone())
                )
            ),
            Self::MainIntegrationValid(value) => record!(
                value,
                VerificationScopeDigest::MainIntegration,
                VerificationOutcomeDigest::Valid,
                session = Some(value.session_id.clone()),
                checkpoint = None,
                difference = (None, None),
                adaptation = None,
                merge = (
                    Some(value.merge_receipt_id.clone()),
                    Some(value.integration_set_digest.clone())
                ),
                support = (
                    Some(value.support_gate_digest.clone()),
                    Some(value.support_gate_history_evidence.clone())
                )
            ),
            Self::MainIntegrationInvalid(value) => record!(
                value,
                VerificationScopeDigest::MainIntegration,
                VerificationOutcomeDigest::Invalid,
                session = Some(value.session_id.clone()),
                checkpoint = None,
                difference = (None, None),
                adaptation = None,
                merge = (
                    Some(value.merge_receipt_id.clone()),
                    Some(value.integration_set_digest.clone())
                ),
                support = (
                    Some(value.support_gate_digest.clone()),
                    Some(value.support_gate_history_evidence.clone())
                )
            ),
        }
    }

    #[cfg(test)]
    fn synchronized_adapted_test_only(
        input: SynchronizedAdaptedVerificationFixtureInput,
        adaptation: &AdaptedDeltaDecisionData,
    ) -> Result<Self, MergeResultContractError> {
        if adaptation.canonical_delta_digest != input.canonical_delta_digest
            || adaptation.difference_digest != input.difference_digest
        {
            return Err(MergeResultContractError(
                "adapted verification disagrees with its adaptation decision",
            ));
        }
        let validation_receipt_ids = ValidationReceiptIds::new(input.validation_receipt_ids)?;
        let selected_object_fingerprints =
            SelectedObjectFingerprints::new(input.selected_object_fingerprints)?;
        let record = MergeVerificationDigestRecord {
            verification_id: input.verification_id,
            scope: VerificationScopeDigest::SynchronizedTask,
            outcome: VerificationOutcomeDigest::Adapted,
            session_id: Some(input.session_id),
            canonical_delta_digest: input.canonical_delta_digest,
            checkpoint_id: Some(input.checkpoint_id),
            validation_receipt_ids,
            support_audit_digest: input.support_audit_digest,
            selected_object_fingerprints,
            difference_manifest_id: Some(input.difference_manifest_id),
            difference_digest: Some(input.difference_digest),
            adaptation_decision_id: Some(adaptation.decision_id.clone()),
            merge_receipt_id: None,
            integration_set_digest: None,
            support_gate_digest: None,
            support_gate_history_evidence: None,
        };
        let verification_digest = merge_digest(&record, "merge-verification digest failed")?;
        Ok(Self::SynchronizedTaskAdapted(
            SynchronizedTaskAdaptedVerificationData {
                verification_id: record.verification_id,
                scope: SynchronizedTaskScope::Value,
                outcome: AdaptedOutcome::Value,
                session_id: record.session_id.expect("set above"),
                canonical_delta_digest: record.canonical_delta_digest,
                checkpoint_id: record.checkpoint_id.expect("set above"),
                difference_manifest_id: record.difference_manifest_id.expect("set above"),
                difference_digest: record.difference_digest.expect("set above"),
                adaptation_decision_id: record.adaptation_decision_id.expect("set above"),
                validation_receipt_ids: record.validation_receipt_ids,
                support_audit_digest: record.support_audit_digest,
                selected_object_fingerprints: record.selected_object_fingerprints,
                verification_digest,
            },
        ))
    }
}

#[cfg(test)]
fn consumer_fixture_digest(character: char) -> Sha256Digest {
    Sha256Digest::parse(&character.to_string().repeat(64)).unwrap()
}

#[cfg(test)]
fn consumer_fixture_id(value: &str) -> UnicaId {
    UnicaId::parse(value).unwrap()
}

#[cfg(test)]
fn verified_original_rollback_checkpoint_fixture_test_only(
    source: ValidatedOriginalMergeLockProjection,
    checkpoint_id: UnicaId,
) -> VerifiedOriginalRollbackCheckpointAuthority {
    struct FixtureCheckpointPort {
        checkpoint_id: Option<UnicaId>,
    }

    impl OriginalMergeRollbackCheckpointPort for FixtureCheckpointPort {
        fn create_and_verify_original_rollback_checkpoint(
            &mut self,
            request: OriginalMergeRollbackCheckpointRequest<'_>,
        ) -> Result<OriginalMergeRollbackCheckpointCompletion, MergeResultContractError> {
            let checkpoint_id = self.checkpoint_id.take().ok_or(MergeResultContractError(
                "rollback checkpoint fixture completion replayed",
            ))?;
            let checkpoint_fingerprint = request.original_fingerprint().clone();
            let root_before_anchor = request.root_before_anchor().clone();
            let observed_current_state_revision = request.current_state_revision().clone();
            Ok(request.complete(
                checkpoint_id,
                checkpoint_fingerprint,
                root_before_anchor,
                observed_current_state_revision,
                CapabilityRowId::parse("repository.rollback-checkpoint.fixture").unwrap(),
            ))
        }
    }

    VerifiedOriginalRollbackCheckpointAuthority::create(
        source,
        &mut FixtureCheckpointPort {
            checkpoint_id: Some(checkpoint_id),
        },
    )
    .unwrap()
}

#[cfg(test)]
struct ConsumerFixtureConfiguredValidationPort {
    snapshot_input: Option<ConfiguredValidationReceiptBatchSnapshotInput>,
}

#[cfg(test)]
impl ConfiguredValidationCheckExecutionPort for ConsumerFixtureConfiguredValidationPort {
    fn execute(
        &mut self,
        request: ConfiguredValidationCheckExecutionRequest<'_>,
    ) -> Result<ConfiguredValidationReceiptBatchSnapshot, MergeResultContractError> {
        let input = self.snapshot_input.take().ok_or(MergeResultContractError(
            "consumer fixture validation snapshot replayed",
        ))?;
        Ok(request.complete(input))
    }
}

#[cfg(test)]
fn consumer_fixture_verification_input<Scope: fmt::Debug>(
    selection: ConfiguredValidationExecutionSelectionAuthority<Scope>,
    canonical_delta_digest: Sha256Digest,
    support_audit_digest: Sha256Digest,
) -> VerificationObservationInputAuthority<Scope> {
    let verification_id = selection.subject.verification_id().clone();
    let mut port = ConsumerFixtureConfiguredValidationPort {
        snapshot_input: Some(
            ConfiguredValidationReceiptBatchSnapshotInput::from_execution_adapter(vec![], vec![]),
        ),
    };
    let batch =
        ConfiguredValidationReceiptBatchAuthority::from_execution_port(selection, &mut port)
            .unwrap();
    VerificationObservationInputAuthority::from_verifier_adapter(
        verification_id,
        canonical_delta_digest,
        batch,
        support_audit_digest,
        vec![],
    )
    .unwrap()
}

#[cfg(test)]
fn consumer_fixture_main_session(
    preflight: &ReadySupportPreflightAuthority,
) -> MainIntegrationSessionData {
    let result_digest = preflight.sandbox_result_digest().clone();
    let base_record = MergeSessionBaseDigestRecord {
        session_id: consumer_fixture_id("8a000000-0000-4000-8000-000000000001"),
        mode: MergeSessionModeDigest::MainIntegration,
        checkpoint_id: consumer_fixture_id("8a000000-0000-4000-8000-000000000002"),
        incoming_distribution_id: None,
        immutable_input_hashes: ImmutableMergeInputHashes {
            checkpoint_verification_digest: consumer_fixture_digest('1'),
            comparison_delta_digest: consumer_fixture_digest('2'),
            source_artifact_sha256: consumer_fixture_digest('3'),
        },
        anchor_digest: consumer_fixture_digest('4'),
        settings_digest: preflight.settings_digest().clone(),
        ordinary_result_artifact_id: Some(preflight.ordinary_result_artifact_id().clone()),
        comparison_id: preflight.comparison_id().clone(),
        result_digest: Some(result_digest.clone()),
        merge_resolution_workspace_id: None,
        conflicts: vec![],
        support_gate_id: Some(preflight.support_gate_id().clone()),
        support_gate_digest: Some(preflight.support_gate_digest().clone()),
        support_gate_history_evidence_digest: Some(preflight.history_evidence_digest().clone()),
    };
    let base_session_digest =
        merge_digest(&base_record, "fixture main base digest failed").unwrap();
    let decision_set_digest = decision_set_digest(&[]).unwrap();
    let resolved_session_digest = merge_digest(
        &ResolvedSessionDigestRecord {
            base_session_digest: base_session_digest.clone(),
            decision_set_digest: decision_set_digest.clone(),
            result_digest,
            applied_decision_ids: vec![],
        },
        "fixture main resolved digest failed",
    )
    .unwrap();
    MainIntegrationSessionData {
        session_id: base_record.session_id,
        mode: MainIntegrationMode::Value,
        checkpoint_id: base_record.checkpoint_id,
        immutable_input_hashes: base_record.immutable_input_hashes,
        anchor_digest: base_record.anchor_digest,
        settings_digest: base_record.settings_digest,
        ordinary_result_artifact_id: base_record.ordinary_result_artifact_id.unwrap(),
        comparison_id: base_record.comparison_id,
        result_digest: base_record.result_digest.unwrap(),
        conflict_count: ZeroCount,
        base_session_digest,
        decision_set_digest,
        resolved_session_digest,
        support_gate_id: base_record.support_gate_id.unwrap(),
        support_gate_digest: base_record.support_gate_digest.unwrap(),
        support_gate_history_evidence_digest: base_record
            .support_gate_history_evidence_digest
            .unwrap(),
    }
}

#[cfg(test)]
pub(crate) fn validated_main_sandbox_verification_fixture_test_only(
) -> ValidatedMainSandboxVerificationAuthority {
    let preflight = ready_preflight_authority_fixture_test_only();
    let main = consumer_fixture_main_session(&preflight);
    let session = MergeSessionData::MainIntegration(main.clone());
    let decision_projection =
        ResolvedApplyDecisionProjectionAuthority::for_no_conflict_session(&session).unwrap();
    let preparation = ValidatedMainIntegrationPreparationAuthority::new(preflight, main).unwrap();
    let planning = ValidatedRepositoryPlanSessionProjection::from_main_preparation(
        preparation,
        decision_projection,
    )
    .unwrap();
    let plan = ConfiguredValidationCheckPlanAuthority::from_configuration_adapter(vec![]).unwrap();
    let selection = ConfiguredValidationExecutionSelectionAuthority::main_sandbox(
        &plan,
        &planning,
        consumer_fixture_id("8a000000-0000-4000-8000-000000000003"),
    );
    let observation = MainSandboxVerificationObservationAuthority::valid_from_verifier_adapter(
        &planning,
        consumer_fixture_verification_input(
            selection,
            consumer_fixture_digest('5'),
            consumer_fixture_digest('6'),
        ),
    )
    .unwrap();
    MergeVerificationData::main_sandbox_valid_from_authorities(planning, observation).unwrap()
}

#[cfg(test)]
pub(crate) fn validated_main_integration_verification_fixture_test_only(
) -> ValidatedMainIntegrationVerificationAuthority {
    let lineage = validated_consumed_original_merge_context_fixture_test_only(
        consumer_fixture_id("8a000000-0000-4000-8000-000000000005"),
        consumer_fixture_digest('9'),
    );
    let plan = ConfiguredValidationCheckPlanAuthority::from_configuration_adapter(vec![]).unwrap();
    let selection = ConfiguredValidationExecutionSelectionAuthority::main_integration(
        &plan,
        lineage,
        consumer_fixture_id("8a000000-0000-4000-8000-000000000006"),
    );
    let observation = MainIntegrationVerificationObservationAuthority::valid_from_verifier_adapter(
        consumer_fixture_verification_input(
            selection,
            consumer_fixture_digest('b'),
            consumer_fixture_digest('c'),
        ),
    );
    MergeVerificationData::main_integration_valid_from_authorities(observation).unwrap()
}

#[cfg(test)]
pub(crate) fn validated_consumed_original_merge_context_fixture_test_only(
    merge_receipt_id: UnicaId,
    result_fingerprint: Sha256Digest,
) -> ValidatedConsumedOriginalMergeLineageAuthority {
    validated_consumed_original_merge_context_with_lock_identity_fixture_internal(
        merge_receipt_id,
        result_fingerprint,
        None,
    )
}

#[cfg(test)]
fn validated_consumed_original_merge_context_with_lock_identity_fixture_internal(
    merge_receipt_id: UnicaId,
    result_fingerprint: Sha256Digest,
    lock_identity: Option<(UnicaId, NormalizedUtcInstant)>,
) -> ValidatedConsumedOriginalMergeLineageAuthority {
    use super::repository::{
        original_merge_production_lock_projection_fixture_test_only,
        original_merge_production_lock_projection_with_identity_fixture_test_only,
    };

    struct PreIntentLease;
    impl OriginalMergePreIntentLease for PreIntentLease {
        fn binds(&self, _request: &OriginalMergePreIntentRequest<'_>) -> bool {
            true
        }

        fn preintent_capability_id(&self) -> &CapabilityRowId {
            static ID: std::sync::OnceLock<CapabilityRowId> = std::sync::OnceLock::new();
            ID.get_or_init(|| CapabilityRowId::parse("repository.preintent.b2-fixture").unwrap())
        }
    }

    struct PreIntentPort;
    impl OriginalMergePreIntentPort for PreIntentPort {
        fn reread_immediately_before_original_merge(
            &mut self,
            request: OriginalMergePreIntentRequest<'_>,
        ) -> Result<OriginalMergePreIntentCompletion, MergeResultContractError> {
            Ok(request.complete(Box::new(PreIntentLease)))
        }
    }

    struct EffectPort(Sha256Digest);
    impl OriginalMergeExecutionPort for EffectPort {
        fn execute_original_merge(
            &mut self,
            request: OriginalMergeExecutionRequest<'_>,
        ) -> Result<OriginalMergeExecutionPortOutcome, MergeResultContractError> {
            Ok(OriginalMergeExecutionPortOutcome::Observed(
                request.complete(
                    consumer_fixture_digest('7'),
                    consumer_fixture_digest('8'),
                    self.0.clone(),
                    consumer_fixture_digest('a'),
                ),
            ))
        }
    }

    struct CasLease;
    impl SupportGateOriginalMergeCasLease for CasLease {
        fn binds(&self, _binding: &SupportGateOriginalMergeCasBinding) -> bool {
            true
        }

        fn commit_receipt_and_consume_gate(
            self: Box<Self>,
            _pending: &PendingOriginalMergeReceiptAuthority,
        ) -> Result<(), MergeResultContractError> {
            Ok(())
        }
    }

    struct CasResolver;
    impl SupportGateOriginalMergeCasResolver for CasResolver {
        fn resolve_original_merge_cas(
            &mut self,
            request: SupportGateOriginalMergeCasRequest<'_>,
        ) -> Result<SupportGateOriginalMergeCasResolution, MergeResultContractError> {
            Ok(request.complete(Box::new(CasLease)))
        }
    }

    struct ConsumedLease;
    impl ConsumedSupportGateStateLease for ConsumedLease {
        fn binds(&self, _request: &ConsumedSupportGateObservationRequest<'_>) -> bool {
            true
        }

        fn consumed_state_revision(&self) -> &Sha256Digest {
            static REVISION: std::sync::OnceLock<Sha256Digest> = std::sync::OnceLock::new();
            REVISION.get_or_init(|| consumer_fixture_digest('c'))
        }

        fn observation_capability_id(&self) -> &CapabilityRowId {
            static ID: std::sync::OnceLock<CapabilityRowId> = std::sync::OnceLock::new();
            ID.get_or_init(|| CapabilityRowId::parse("repository.consumed.b2-fixture").unwrap())
        }
    }

    struct ConsumedResolver;
    impl ConsumedSupportGateStateResolver for ConsumedResolver {
        fn resolve_consumed_by_original_merge(
            &mut self,
            request: ConsumedSupportGateObservationRequest<'_>,
        ) -> Result<ConsumedSupportGateStateResolution, MergeResultContractError> {
            Ok(request.complete(Box::new(ConsumedLease)))
        }
    }

    let preflight = ready_preflight_authority_fixture_test_only();
    let main = consumer_fixture_main_session(&preflight);
    let session = MergeSessionData::MainIntegration(main.clone());
    let decisions =
        ResolvedApplyDecisionProjectionAuthority::for_no_conflict_session(&session).unwrap();
    let projection = match lock_identity {
        Some((lock_set_id, observed_at)) => {
            original_merge_production_lock_projection_with_identity_fixture_test_only(
                main.session_id.clone(),
                main.resolved_session_digest.clone(),
                preflight,
                main.settings_digest.clone(),
                lock_set_id,
                observed_at,
            )
        }
        None => original_merge_production_lock_projection_fixture_test_only(
            main.session_id.clone(),
            main.resolved_session_digest.clone(),
            preflight,
            main.settings_digest.clone(),
        ),
    };
    let rollback = verified_original_rollback_checkpoint_fixture_test_only(
        projection,
        consumer_fixture_id("8a000000-0000-4000-8000-000000000020"),
    );
    let preintent =
        OriginalMergePreIntentAuthority::recheck(&session, decisions, rollback, &mut PreIntentPort)
            .unwrap();
    let pending = PendingOriginalMergeReceiptAuthority::execute(
        preintent,
        merge_receipt_id,
        &mut EffectPort(result_fingerprint),
    )
    .unwrap();
    let receipt = ValidatedSupportGateOriginalMergeCasAuthority::resolve(pending, &mut CasResolver)
        .unwrap()
        .commit()
        .unwrap();
    ResolvedReceiptConsumedSupportGateAuthority::resolve(receipt, &mut ConsumedResolver)
        .unwrap()
        .rebind()
}

#[cfg(test)]
pub(crate) fn validated_main_integration_commit_context_fixture_test_only(
    merge_receipt_id: UnicaId,
    result_fingerprint: Sha256Digest,
) -> ResolvedCommitLineageConsumedSupportGateAuthority {
    validated_main_integration_commit_context_with_lock_identity_fixture_internal(
        merge_receipt_id,
        result_fingerprint,
        None,
    )
}

#[cfg(test)]
pub(crate) fn validated_main_integration_commit_context_with_lock_identity_fixture_test_only(
    merge_receipt_id: UnicaId,
    result_fingerprint: Sha256Digest,
    lock_set_id: UnicaId,
    observed_at: NormalizedUtcInstant,
) -> ResolvedCommitLineageConsumedSupportGateAuthority {
    validated_main_integration_commit_context_with_lock_identity_fixture_internal(
        merge_receipt_id,
        result_fingerprint,
        Some((lock_set_id, observed_at)),
    )
}

#[cfg(test)]
fn validated_main_integration_commit_context_with_lock_identity_fixture_internal(
    merge_receipt_id: UnicaId,
    result_fingerprint: Sha256Digest,
    lock_identity: Option<(UnicaId, NormalizedUtcInstant)>,
) -> ResolvedCommitLineageConsumedSupportGateAuthority {
    struct FreshConsumedLease;
    impl ConsumedSupportGateStateLease for FreshConsumedLease {
        fn binds(&self, _request: &ConsumedSupportGateObservationRequest<'_>) -> bool {
            true
        }

        fn consumed_state_revision(&self) -> &Sha256Digest {
            static REVISION: std::sync::OnceLock<Sha256Digest> = std::sync::OnceLock::new();
            REVISION.get_or_init(|| consumer_fixture_digest('b'))
        }

        fn observation_capability_id(&self) -> &CapabilityRowId {
            static ID: std::sync::OnceLock<CapabilityRowId> = std::sync::OnceLock::new();
            ID.get_or_init(|| {
                CapabilityRowId::parse("repository.consumed-gate.commit-fixture").unwrap()
            })
        }
    }

    struct FreshConsumedResolver;
    impl ConsumedSupportGateStateResolver for FreshConsumedResolver {
        fn resolve_consumed_by_original_merge(
            &mut self,
            request: ConsumedSupportGateObservationRequest<'_>,
        ) -> Result<ConsumedSupportGateStateResolution, MergeResultContractError> {
            Ok(request.complete(Box::new(FreshConsumedLease)))
        }
    }

    let lineage = validated_consumed_original_merge_context_with_lock_identity_fixture_internal(
        merge_receipt_id,
        result_fingerprint,
        lock_identity,
    );
    let check_plan =
        ConfiguredValidationCheckPlanAuthority::from_configuration_adapter(vec![]).unwrap();
    let selection = ConfiguredValidationExecutionSelectionAuthority::main_integration(
        &check_plan,
        lineage,
        consumer_fixture_id("8a000000-0000-4000-8000-000000000021"),
    );
    let observation = MainIntegrationVerificationObservationAuthority::valid_from_verifier_adapter(
        consumer_fixture_verification_input(
            selection,
            consumer_fixture_digest('b'),
            consumer_fixture_digest('c'),
        ),
    );
    let verified =
        MergeVerificationData::main_integration_valid_from_authorities(observation).unwrap();
    let commit = verified.into_commit_lineage();
    ResolvedCommitLineageConsumedSupportGateAuthority::resolve(commit, &mut FreshConsumedResolver)
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::branched_development::contracts::change_receipts::{
        BranchedChangeReceipt, BranchedChangeReceiptAuthority, ChangeReceiptSequence,
        MergeResolutionDecisionLineageAuthority, MergeResolutionSelectableReceiptAuthority,
    };
    use crate::domain::branched_development::contracts::scalars::RepositoryVersion;
    use crate::domain::branched_development::contracts::schema::audit_json_schema;
    use crate::domain::branched_development::contracts::status::MergeResolutionWorkspaceResumeHandle;
    use crate::domain::branched_development::contracts::support::ready_preflight_authority_fixture_test_only;
    use schemars::schema_for;
    use serde::de::DeserializeOwned;
    use serde_json::{json, Value};

    const ID_1: &str = "123e4567-e89b-12d3-a456-426614174001";
    const ID_2: &str = "123e4567-e89b-12d3-a456-426614174002";
    const ID_3: &str = "123e4567-e89b-12d3-a456-426614174003";
    const ID_4: &str = "123e4567-e89b-12d3-a456-426614174004";
    const ID_5: &str = "123e4567-e89b-12d3-a456-426614174005";
    const ID_6: &str = "123e4567-e89b-12d3-a456-426614174006";
    const ID_7: &str = "123e4567-e89b-12d3-a456-426614174007";
    const OBJECT_1: &str = "223e4567-e89b-12d3-a456-426614174001";
    const OBJECT_2: &str = "223e4567-e89b-12d3-a456-426614174002";

    fn id(value: &str) -> UnicaId {
        UnicaId::parse(value).unwrap()
    }

    fn digest(character: char) -> Sha256Digest {
        Sha256Digest::parse(&character.to_string().repeat(64)).unwrap()
    }

    struct StaticConfiguredValidationPort {
        snapshot_input: Option<ConfiguredValidationReceiptBatchSnapshotInput>,
    }

    impl ConfiguredValidationCheckExecutionPort for StaticConfiguredValidationPort {
        fn execute(
            &mut self,
            request: ConfiguredValidationCheckExecutionRequest<'_>,
        ) -> Result<ConfiguredValidationReceiptBatchSnapshot, MergeResultContractError> {
            let input = self.snapshot_input.take().ok_or(MergeResultContractError(
                "configured validation snapshot replayed",
            ))?;
            Ok(request.complete(input))
        }
    }

    fn configured_validation_plan(receipt_count: usize) -> ConfiguredValidationCheckPlanAuthority {
        ConfiguredValidationCheckPlanAuthority::from_configuration_adapter(
            (0..receipt_count)
                .map(|index| Name::parse(&format!("check{index}")).unwrap())
                .collect(),
        )
        .unwrap()
    }

    fn verification_input_for_selection<Scope: fmt::Debug>(
        selection: ConfiguredValidationExecutionSelectionAuthority<Scope>,
        canonical_delta_digest: Sha256Digest,
        receipt_ids: Vec<UnicaId>,
        support_audit_digest: Sha256Digest,
        selected_object_fingerprints: Vec<SelectedObjectFingerprint>,
    ) -> VerificationObservationInputAuthority<Scope> {
        let verification_id = selection.subject.verification_id().clone();
        let configured_check_ids = selection.configured_check_ids.clone();
        let mut port = StaticConfiguredValidationPort {
            snapshot_input: Some(
                ConfiguredValidationReceiptBatchSnapshotInput::from_execution_adapter(
                    configured_check_ids,
                    receipt_ids,
                ),
            ),
        };
        let batch =
            ConfiguredValidationReceiptBatchAuthority::from_execution_port(selection, &mut port)
                .unwrap();
        VerificationObservationInputAuthority::from_verifier_adapter(
            verification_id,
            canonical_delta_digest,
            batch,
            support_audit_digest,
            selected_object_fingerprints,
        )
        .unwrap()
    }

    fn local_verification_input(
        verification_id: UnicaId,
        canonical_delta_digest: Sha256Digest,
        receipt_ids: Vec<UnicaId>,
        support_audit_digest: Sha256Digest,
        selected_object_fingerprints: Vec<SelectedObjectFingerprint>,
    ) -> LocalCheckpointVerificationObservationInputAuthority {
        let plan = configured_validation_plan(receipt_ids.len());
        let selection = ConfiguredValidationExecutionSelectionAuthority::local_checkpoint(
            &plan,
            verification_id,
        );
        verification_input_for_selection(
            selection,
            canonical_delta_digest,
            receipt_ids,
            support_audit_digest,
            selected_object_fingerprints,
        )
    }

    fn resolved_task_verification_input(
        session: &MergeSessionData,
        verification_id: UnicaId,
        canonical_delta_digest: Sha256Digest,
        receipt_ids: Vec<UnicaId>,
        support_audit_digest: Sha256Digest,
        selected_object_fingerprints: Vec<SelectedObjectFingerprint>,
    ) -> ResolvedTaskVerificationObservationInputAuthority {
        let plan = configured_validation_plan(receipt_ids.len());
        let selection = ConfiguredValidationExecutionSelectionAuthority::resolved_task(
            &plan,
            session,
            verification_id,
        )
        .unwrap();
        verification_input_for_selection(
            selection,
            canonical_delta_digest,
            receipt_ids,
            support_audit_digest,
            selected_object_fingerprints,
        )
    }

    fn main_sandbox_verification_input(
        planning: &ValidatedRepositoryPlanSessionProjection,
        verification_id: UnicaId,
        canonical_delta_digest: Sha256Digest,
        receipt_ids: Vec<UnicaId>,
        support_audit_digest: Sha256Digest,
        selected_object_fingerprints: Vec<SelectedObjectFingerprint>,
    ) -> MainSandboxVerificationObservationInputAuthority {
        let plan = configured_validation_plan(receipt_ids.len());
        let selection = ConfiguredValidationExecutionSelectionAuthority::main_sandbox(
            &plan,
            planning,
            verification_id,
        );
        verification_input_for_selection(
            selection,
            canonical_delta_digest,
            receipt_ids,
            support_audit_digest,
            selected_object_fingerprints,
        )
    }

    fn main_integration_verification_input(
        lineage: ValidatedConsumedOriginalMergeLineageAuthority,
        verification_id: UnicaId,
        canonical_delta_digest: Sha256Digest,
        receipt_ids: Vec<UnicaId>,
        support_audit_digest: Sha256Digest,
        selected_object_fingerprints: Vec<SelectedObjectFingerprint>,
    ) -> MainIntegrationVerificationObservationInputAuthority {
        let plan = configured_validation_plan(receipt_ids.len());
        let selection = ConfiguredValidationExecutionSelectionAuthority::main_integration(
            &plan,
            lineage,
            verification_id,
        );
        verification_input_for_selection(
            selection,
            canonical_delta_digest,
            receipt_ids,
            support_audit_digest,
            selected_object_fingerprints,
        )
    }

    struct StaticComparisonResolver {
        observation: Option<PlatformComparisonCapabilitySnapshot>,
    }

    impl PlatformComparisonCapabilityResolver for StaticComparisonResolver {
        fn compare(
            &mut self,
            _scope: ComparisonCapabilityScope<'_>,
        ) -> Result<PlatformComparisonCapabilitySnapshot, MergeResultContractError> {
            self.observation
                .take()
                .ok_or(MergeResultContractError("comparison observation replayed"))
        }
    }

    fn comparison_snapshot_input(
        comparison_id: UnicaId,
        delta_digest: Sha256Digest,
        classified_change_ids: Vec<UnicaId>,
    ) -> PlatformComparisonCapabilitySnapshotInput {
        PlatformComparisonCapabilitySnapshotInput::from_comparison_adapter(
            comparison_id,
            id(ID_5),
            id(ID_6),
            delta_digest,
            ClassifiedComparisonChangeBatchAuthority::from_classifier_adapter(
                classified_change_ids,
            )
            .unwrap(),
            vec![],
        )
    }

    fn classifier_row_input(
        conflict_id: &str,
        object_id: &str,
        object_display: &str,
        allowed_resolutions: Vec<MergeResolution>,
    ) -> ClassifiedMergeConflictInput {
        ClassifiedMergeConflictInput::from_classifier_adapter(
            MergeConflictIdentityInput::from_classifier_adapter(
                id(conflict_id),
                MetadataObjectId::parse(object_id).unwrap(),
                RepositoryTargetDisplay::parse(object_display).unwrap(),
                PropertyPath::parse("Module.Text").unwrap(),
                MergeConflictKind::TwiceChanged,
            ),
            MergeConflictContentInput::from_classifier_adapter(
                digest('b'),
                digest('c'),
                digest('d'),
                allowed_resolutions,
            ),
        )
    }

    fn classifier_batch(
        conflicts: Vec<ClassifiedMergeConflictInput>,
    ) -> ClassifiedMergeConflictBatchAuthority {
        let order =
            MergeConflictClassifierOrderAuthority::from_classifier_adapter(&conflicts).unwrap();
        ClassifiedMergeConflictBatchAuthority::from_classifier_snapshot_adapter(order, conflicts)
            .unwrap()
    }

    fn input_hashes() -> ImmutableMergeInputHashes {
        ImmutableMergeInputHashes {
            checkpoint_verification_digest: digest('1'),
            comparison_delta_digest: digest('2'),
            source_artifact_sha256: digest('3'),
        }
    }

    fn conflict(conflict_id: &str, object_id: &str) -> MergeConflict {
        MergeConflict {
            conflict_id: id(conflict_id),
            object_id: MetadataObjectId::parse(object_id).unwrap(),
            object_display: RepositoryTargetDisplay::parse("Catalog.Product").unwrap(),
            property_path: PropertyPath::parse("Module.Text").unwrap(),
            kind: MergeConflictKind::TwiceChanged,
            base_sha256: digest('4'),
            ours_sha256: digest('5'),
            theirs_sha256: digest('6'),
            allowed_resolutions: AllowedMergeResolutions::new(vec![
                MergeResolution::TakeOurs,
                MergeResolution::TakeTheirs,
                MergeResolution::Combine,
                MergeResolution::Manual,
            ])
            .unwrap(),
            decision_state: ConflictDecisionState::undecided_test_only(),
        }
    }

    fn conflict_without_manual_workspace(conflict_id: &str, object_id: &str) -> MergeConflict {
        let mut value = conflict(conflict_id, object_id);
        value.allowed_resolutions = AllowedMergeResolutions::new(vec![
            MergeResolution::TakeOurs,
            MergeResolution::TakeTheirs,
        ])
        .unwrap();
        value
    }

    fn conflicted_session(conflicts: &[MergeConflict]) -> MergeSessionData {
        let base_record = MergeSessionBaseDigestRecord {
            session_id: id(ID_1),
            mode: MergeSessionModeDigest::SupportedUpdate,
            checkpoint_id: id(ID_2),
            incoming_distribution_id: Some(id(ID_3)),
            immutable_input_hashes: input_hashes(),
            anchor_digest: digest('7'),
            settings_digest: digest('8'),
            ordinary_result_artifact_id: None,
            comparison_id: id(ID_4),
            result_digest: None,
            merge_resolution_workspace_id: Some(id(ID_5)),
            conflicts: conflicts
                .iter()
                .map(ConflictImmutableDigestRecord::from)
                .collect(),
            support_gate_id: None,
            support_gate_digest: None,
            support_gate_history_evidence_digest: None,
        };
        MergeSessionData::SupportedUpdateConflicted(SupportedUpdateConflictedSessionData {
            session_id: base_record.session_id.clone(),
            mode: SupportedUpdateMode::Value,
            checkpoint_id: base_record.checkpoint_id.clone(),
            incoming_distribution_id: base_record
                .incoming_distribution_id
                .clone()
                .expect("set above"),
            immutable_input_hashes: base_record.immutable_input_hashes.clone(),
            anchor_digest: base_record.anchor_digest.clone(),
            settings_digest: base_record.settings_digest.clone(),
            comparison_id: base_record.comparison_id.clone(),
            conflict_count: PositiveCount::new(
                SafeResultCount::new(conflicts.len() as u64).unwrap(),
            )
            .unwrap(),
            merge_resolution_workspace_id: base_record
                .merge_resolution_workspace_id
                .clone()
                .expect("set above"),
            base_session_digest: merge_digest(&base_record, "test base digest failed").unwrap(),
            decision_set_digest: decision_set_digest(conflicts).unwrap(),
        })
    }

    fn changed_receipt_for_conflict(
        session: &MergeSessionData,
        conflict: &MergeConflict,
        after_sha256: Sha256Digest,
    ) -> BranchedChangeReceipt {
        let authority = BranchedChangeReceiptAuthority::merge_resolution_changed_test_only(
            id(ID_4),
            MetadataPropertyAffectedTarget::new(
                conflict.object_id.clone(),
                conflict.property_path.clone(),
            ),
            conflict.ours_sha256.clone(),
            after_sha256,
            vec![id(ID_3)],
            vec![],
            MergeResolutionDecisionLineageAuthority::undecided_test_only(
                session.decision_set_digest().clone(),
            ),
            session.base_session_digest().clone(),
            id(ID_5),
            ChangeReceiptSequence::new(1).unwrap(),
        )
        .unwrap();
        BranchedChangeReceipt::new(&authority).unwrap()
    }

    fn current_resolution_workspace(
        session: &MergeSessionData,
    ) -> MergeResolutionWorkspaceResumeHandle {
        MergeResolutionWorkspaceResumeHandle::new(
            session.session_id().clone(),
            id(ID_5),
            session.base_session_digest().clone(),
        )
    }

    fn selectable_changed_receipt(
        session: &MergeSessionData,
        conflict: &MergeConflict,
        after_sha256: Sha256Digest,
    ) -> SelectableResolutionChangeReceiptAuthority {
        let receipt = changed_receipt_for_conflict(session, conflict, after_sha256);
        let workspace = current_resolution_workspace(session);
        let handle = ResolutionChangeReceiptResumeHandle::selectable_from_changed_receipt(
            &receipt, &workspace,
        )
        .unwrap();
        SelectableResolutionChangeReceiptAuthority::try_from_current(receipt, handle, &workspace)
            .unwrap()
    }

    fn no_change_receipt_for_conflict(
        session: &MergeSessionData,
        conflict: &MergeConflict,
    ) -> BranchedChangeReceipt {
        let authority = BranchedChangeReceiptAuthority::merge_resolution_no_change_test_only(
            id(ID_4),
            MetadataPropertyAffectedTarget::new(
                conflict.object_id.clone(),
                conflict.property_path.clone(),
            ),
            conflict.ours_sha256.clone(),
            session.decision_set_digest().clone(),
            session.base_session_digest().clone(),
            id(ID_5),
            ChangeReceiptSequence::new(1).unwrap(),
        )
        .unwrap();
        BranchedChangeReceipt::new(&authority).unwrap()
    }

    fn conflicted_session_without_workspace(conflicts: &[MergeConflict]) -> MergeSessionData {
        let base_record = MergeSessionBaseDigestRecord {
            session_id: id(ID_1),
            mode: MergeSessionModeDigest::SupportedUpdate,
            checkpoint_id: id(ID_2),
            incoming_distribution_id: Some(id(ID_3)),
            immutable_input_hashes: input_hashes(),
            anchor_digest: digest('7'),
            settings_digest: digest('8'),
            ordinary_result_artifact_id: None,
            comparison_id: id(ID_4),
            result_digest: None,
            merge_resolution_workspace_id: None,
            conflicts: conflicts
                .iter()
                .map(ConflictImmutableDigestRecord::from)
                .collect(),
            support_gate_id: None,
            support_gate_digest: None,
            support_gate_history_evidence_digest: None,
        };
        MergeSessionData::SupportedUpdateConflictedWithoutWorkspace(
            SupportedUpdateConflictedWithoutWorkspaceSessionData {
                session_id: base_record.session_id.clone(),
                mode: SupportedUpdateMode::Value,
                checkpoint_id: base_record.checkpoint_id.clone(),
                incoming_distribution_id: base_record
                    .incoming_distribution_id
                    .clone()
                    .expect("set above"),
                immutable_input_hashes: base_record.immutable_input_hashes.clone(),
                anchor_digest: base_record.anchor_digest.clone(),
                settings_digest: base_record.settings_digest.clone(),
                comparison_id: base_record.comparison_id.clone(),
                conflict_count: PositiveCount::new(
                    SafeResultCount::new(conflicts.len() as u64).unwrap(),
                )
                .unwrap(),
                base_session_digest: merge_digest(&base_record, "test base digest failed").unwrap(),
                decision_set_digest: decision_set_digest(conflicts).unwrap(),
            },
        )
    }

    fn resolved_session() -> MergeSessionData {
        let base_record = MergeSessionBaseDigestRecord {
            session_id: id(ID_1),
            mode: MergeSessionModeDigest::SupportedUpdate,
            checkpoint_id: id(ID_2),
            incoming_distribution_id: Some(id(ID_3)),
            immutable_input_hashes: input_hashes(),
            anchor_digest: digest('7'),
            settings_digest: digest('8'),
            ordinary_result_artifact_id: None,
            comparison_id: id(ID_4),
            result_digest: Some(digest('9')),
            merge_resolution_workspace_id: None,
            conflicts: vec![],
            support_gate_id: None,
            support_gate_digest: None,
            support_gate_history_evidence_digest: None,
        };
        let base_session_digest = merge_digest(&base_record, "test base digest failed").unwrap();
        let decision_set_digest = decision_set_digest(&[]).unwrap();
        let resolved_session_digest = merge_digest(
            &ResolvedSessionDigestRecord {
                base_session_digest: base_session_digest.clone(),
                decision_set_digest: decision_set_digest.clone(),
                result_digest: base_record.result_digest.clone().expect("set above"),
                applied_decision_ids: vec![],
            },
            "test resolved digest failed",
        )
        .unwrap();
        MergeSessionData::SupportedUpdateResolved(SupportedUpdateResolvedSessionData {
            session_id: base_record.session_id,
            mode: SupportedUpdateMode::Value,
            checkpoint_id: base_record.checkpoint_id,
            incoming_distribution_id: base_record.incoming_distribution_id.expect("set above"),
            immutable_input_hashes: base_record.immutable_input_hashes,
            anchor_digest: base_record.anchor_digest,
            settings_digest: base_record.settings_digest,
            comparison_id: base_record.comparison_id,
            result_digest: base_record.result_digest.expect("set above"),
            conflict_count: ZeroCount,
            base_session_digest,
            decision_set_digest,
            resolved_session_digest,
        })
    }

    fn main_integration_session() -> MergeSessionData {
        let base_record = MergeSessionBaseDigestRecord {
            session_id: id(ID_1),
            mode: MergeSessionModeDigest::MainIntegration,
            checkpoint_id: id(ID_2),
            incoming_distribution_id: None,
            immutable_input_hashes: input_hashes(),
            anchor_digest: digest('7'),
            settings_digest: digest('8'),
            ordinary_result_artifact_id: Some(id(ID_3)),
            comparison_id: id(ID_4),
            result_digest: Some(digest('9')),
            merge_resolution_workspace_id: None,
            conflicts: vec![],
            support_gate_id: Some(id(ID_5)),
            support_gate_digest: Some(digest('a')),
            support_gate_history_evidence_digest: Some(digest('b')),
        };
        let base_session_digest =
            merge_digest(&base_record, "test main base digest failed").unwrap();
        let decision_set_digest = decision_set_digest(&[]).unwrap();
        let resolved_session_digest = merge_digest(
            &ResolvedSessionDigestRecord {
                base_session_digest: base_session_digest.clone(),
                decision_set_digest: decision_set_digest.clone(),
                result_digest: base_record.result_digest.clone().expect("set above"),
                applied_decision_ids: vec![],
            },
            "test main resolved digest failed",
        )
        .unwrap();
        MergeSessionData::MainIntegration(MainIntegrationSessionData {
            session_id: base_record.session_id,
            mode: MainIntegrationMode::Value,
            checkpoint_id: base_record.checkpoint_id,
            immutable_input_hashes: base_record.immutable_input_hashes,
            anchor_digest: base_record.anchor_digest,
            settings_digest: base_record.settings_digest,
            ordinary_result_artifact_id: base_record
                .ordinary_result_artifact_id
                .expect("set above"),
            comparison_id: base_record.comparison_id,
            result_digest: base_record.result_digest.expect("set above"),
            conflict_count: ZeroCount,
            base_session_digest,
            decision_set_digest,
            resolved_session_digest,
            support_gate_id: base_record.support_gate_id.expect("set above"),
            support_gate_digest: base_record.support_gate_digest.expect("set above"),
            support_gate_history_evidence_digest: base_record
                .support_gate_history_evidence_digest
                .expect("set above"),
        })
    }

    fn main_integration_session_for_preflight(
        preflight: &ReadySupportPreflightAuthority,
    ) -> MainIntegrationSessionData {
        let base_record = MergeSessionBaseDigestRecord {
            session_id: id(ID_1),
            mode: MergeSessionModeDigest::MainIntegration,
            checkpoint_id: id(ID_2),
            incoming_distribution_id: None,
            immutable_input_hashes: input_hashes(),
            anchor_digest: digest('7'),
            settings_digest: preflight.settings_digest().clone(),
            ordinary_result_artifact_id: Some(preflight.ordinary_result_artifact_id().clone()),
            comparison_id: preflight.comparison_id().clone(),
            result_digest: Some(preflight.sandbox_result_digest().clone()),
            merge_resolution_workspace_id: None,
            conflicts: vec![],
            support_gate_id: Some(preflight.support_gate_id().clone()),
            support_gate_digest: Some(preflight.support_gate_digest().clone()),
            support_gate_history_evidence_digest: Some(preflight.history_evidence_digest().clone()),
        };
        let base_session_digest =
            merge_digest(&base_record, "test bound main base digest failed").unwrap();
        let decision_set_digest = decision_set_digest(&[]).unwrap();
        let resolved_session_digest = merge_digest(
            &ResolvedSessionDigestRecord {
                base_session_digest: base_session_digest.clone(),
                decision_set_digest: decision_set_digest.clone(),
                result_digest: base_record.result_digest.clone().expect("set above"),
                applied_decision_ids: vec![],
            },
            "test bound main resolved digest failed",
        )
        .unwrap();
        MainIntegrationSessionData {
            session_id: base_record.session_id,
            mode: MainIntegrationMode::Value,
            checkpoint_id: base_record.checkpoint_id,
            immutable_input_hashes: base_record.immutable_input_hashes,
            anchor_digest: base_record.anchor_digest,
            settings_digest: base_record.settings_digest,
            ordinary_result_artifact_id: base_record
                .ordinary_result_artifact_id
                .expect("set above"),
            comparison_id: base_record.comparison_id,
            result_digest: base_record.result_digest.expect("set above"),
            conflict_count: ZeroCount,
            base_session_digest,
            decision_set_digest,
            resolved_session_digest,
            support_gate_id: base_record.support_gate_id.expect("set above"),
            support_gate_digest: base_record.support_gate_digest.expect("set above"),
            support_gate_history_evidence_digest: base_record
                .support_gate_history_evidence_digest
                .expect("set above"),
        }
    }

    fn schema<T: JsonSchema>() -> Value {
        serde_json::to_value(schema_for!(T)).unwrap()
    }

    fn schema_accepts<T: JsonSchema>(value: &Value) -> bool {
        jsonschema::options()
            .with_draft(jsonschema::Draft::Draft202012)
            .build(&schema::<T>())
            .unwrap()
            .is_valid(value)
    }

    fn assert_closed<T: JsonSchema>() {
        audit_json_schema(&schema::<T>()).unwrap();
    }

    fn assert_forbidden_fields_rejected_recursively<T: JsonSchema>(valid: Value) {
        fn object_pointers(value: &Value, pointer: String, output: &mut Vec<String>) {
            match value {
                Value::Object(object) => {
                    output.push(pointer.clone());
                    for (key, nested) in object {
                        let key = key.replace('~', "~0").replace('/', "~1");
                        object_pointers(nested, format!("{pointer}/{key}"), output);
                    }
                }
                Value::Array(values) => {
                    for (index, nested) in values.iter().enumerate() {
                        object_pointers(nested, format!("{pointer}/{index}"), output);
                    }
                }
                Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
            }
        }

        assert!(schema_accepts::<T>(&valid));
        assert_closed::<T>();
        let mut pointers = Vec::new();
        object_pointers(&valid, String::new(), &mut pointers);
        for pointer in pointers {
            for forbidden in [
                "cwd",
                "localPath",
                "stateRoot",
                "workRoot",
                "pid",
                "processHandle",
                "password",
                "token",
                "secret",
                "credentialRef",
                "rawConnectionString",
                "serviceEndpoint",
            ] {
                let mut invalid = valid.clone();
                invalid
                    .pointer_mut(&pointer)
                    .unwrap()
                    .as_object_mut()
                    .unwrap()
                    .insert(forbidden.to_owned(), json!("forbidden"));
                assert!(
                    !schema_accepts::<T>(&invalid),
                    "{} accepted {forbidden} at {pointer}",
                    T::schema_name(),
                );
            }
        }
    }

    macro_rules! assert_not_deserialize_owned {
        ($type:ty) => {
            const _: fn() = || {
                trait AmbiguousIfDeserialize<Marker> {
                    fn assert_not_deserialize() {}
                }
                struct ImplementsDeserialize;
                impl<T: ?Sized> AmbiguousIfDeserialize<()> for T {}
                impl<T: DeserializeOwned> AmbiguousIfDeserialize<ImplementsDeserialize> for T {}
                let _ = <$type as AmbiguousIfDeserialize<_>>::assert_not_deserialize;
            };
        };
    }

    assert_not_deserialize_owned!(ComparisonData);
    assert_not_deserialize_owned!(MergeSessionData);
    assert_not_deserialize_owned!(MainIntegrationPreparationData);
    assert_not_deserialize_owned!(ConflictListData);
    assert_not_deserialize_owned!(ConflictDecisionData);
    assert_not_deserialize_owned!(AdaptedDeltaDecisionData);
    assert_not_deserialize_owned!(MergeApplyData);
    assert_not_deserialize_owned!(MergeVerificationData);
    assert_not_deserialize_owned!(MergeSessionBaseDigestRecord);
    assert_not_deserialize_owned!(DecisionSetDigestRecord);
    assert_not_deserialize_owned!(ResolvedSessionDigestRecord);
    assert_not_deserialize_owned!(AdaptedDeltaDecisionDigestRecord);
    assert_not_deserialize_owned!(MergeVerificationDigestRecord);

    #[test]
    fn merge_result_comparison_is_closed_and_uses_canonical_unsupported_kinds() {
        let comparison = ComparisonData {
            comparison_id: id(ID_1),
            left_anchor: digest('a'),
            right_anchor: digest('b'),
            platform_report_id: id(ID_2),
            canonical_manifest_id: id(ID_3),
            delta_digest: digest('c'),
            change_count: SafeResultCount::new(2).unwrap(),
            unsupported_kinds: UnsupportedChangeKinds::new(vec![
                Name::parse("addAdd").unwrap(),
                Name::parse("uuidMismatch").unwrap(),
            ])
            .unwrap(),
        };
        assert_forbidden_fields_rejected_recursively::<ComparisonData>(
            serde_json::to_value(comparison).unwrap(),
        );
        assert!(UnsupportedChangeKinds::new(vec![
            Name::parse("z").unwrap(),
            Name::parse("a").unwrap(),
        ])
        .is_err());
        assert!(UnsupportedChangeKinds::new(vec![
            Name::parse("same").unwrap(),
            Name::parse("same").unwrap(),
        ])
        .is_err());
    }

    #[test]
    fn merge_result_session_schema_rejects_mode_and_presence_splices() {
        let resolved = serde_json::to_value(resolved_session()).unwrap();
        assert!(schema_accepts::<MergeSessionData>(&resolved));

        let mut workspace_in_resolved = resolved.clone();
        workspace_in_resolved["mergeResolutionWorkspaceId"] = json!(ID_5);
        assert!(!schema_accepts::<MergeSessionData>(&workspace_in_resolved));

        let conflicts = vec![conflict(ID_6, OBJECT_1)];
        let conflicted = serde_json::to_value(conflicted_session(&conflicts)).unwrap();
        assert!(schema_accepts::<MergeSessionData>(&conflicted));
        let mut gate_in_supported_update = conflicted.clone();
        gate_in_supported_update["supportGateId"] = json!(ID_7);
        assert!(!schema_accepts::<MergeSessionData>(
            &gate_in_supported_update
        ));

        let mut replay_resolved = serde_json::to_value(resolved_session()).unwrap();
        replay_resolved["mode"] = json!("resolvedReplay");
        assert!(schema_accepts::<MergeSessionData>(&replay_resolved));
        let mut replay_conflicted = conflicted.clone();
        replay_conflicted["mode"] = json!("resolvedReplay");
        assert!(schema_accepts::<MergeSessionData>(&replay_conflicted));

        let main = serde_json::to_value(main_integration_session()).unwrap();
        assert!(schema_accepts::<MergeSessionData>(&main));
        let mut incoming_in_main = main.clone();
        incoming_in_main["incomingDistributionId"] = json!(ID_3);
        assert!(!schema_accepts::<MergeSessionData>(&incoming_in_main));
        let mut missing_gate = main;
        missing_gate
            .as_object_mut()
            .unwrap()
            .remove("supportGateHistoryEvidenceDigest");
        assert!(!schema_accepts::<MergeSessionData>(&missing_gate));

        assert_forbidden_fields_rejected_recursively::<MergeSessionData>(resolved);
        assert_closed::<MergeSessionBaseDigestRecord>();
        assert_closed::<DecisionSetDigestRecord>();
        assert_closed::<ResolvedSessionDigestRecord>();
    }

    #[test]
    fn merge_result_conflicted_session_allows_absent_manual_workspace() {
        let conflicts = vec![conflict_without_manual_workspace(ID_6, OBJECT_1)];
        let conflicted =
            serde_json::to_value(conflicted_session_without_workspace(&conflicts)).unwrap();

        assert!(schema_accepts::<MergeSessionData>(&conflicted));
        let list = ConflictListData::from_current_session(
            &conflicted_session_without_workspace(&conflicts),
            conflicts,
        )
        .unwrap();
        assert!(serde_json::to_value(list)
            .unwrap()
            .get("mergeResolutionWorkspaceId")
            .is_none());
    }

    #[test]
    fn merge_result_main_preparation_consumes_one_ready_lineage_authority() {
        let preflight = ready_preflight_authority_fixture_test_only();
        let session = main_integration_session_for_preflight(&preflight);
        let authority =
            ValidatedMainIntegrationPreparationAuthority::new(preflight, session).unwrap();
        let value = serde_json::to_value(MainIntegrationPreparationData::from_authority(authority))
            .unwrap();
        assert!(schema_accepts::<MainIntegrationPreparationData>(&value));

        let preflight = ready_preflight_authority_fixture_test_only();
        let mut mismatched = main_integration_session_for_preflight(&preflight);
        mismatched.support_gate_history_evidence_digest = digest('f');
        assert!(ValidatedMainIntegrationPreparationAuthority::new(preflight, mismatched).is_err());
    }

    #[test]
    fn merge_result_conflict_inventory_binds_base_state_and_workspace() {
        let conflicts = vec![conflict(ID_6, OBJECT_1)];
        let session = conflicted_session(&conflicts);
        let list = ConflictListData::from_current_session(&session, conflicts.clone()).unwrap();
        assert_forbidden_fields_rejected_recursively::<ConflictListData>(
            serde_json::to_value(list).unwrap(),
        );

        let mut substituted = conflicts.clone();
        substituted[0].ours_sha256 = digest('f');
        assert!(ConflictListData::from_current_session(&session, substituted).is_err());

        let ordered = vec![conflict(ID_6, OBJECT_1), conflict(ID_7, OBJECT_2)];
        let ordered_session = conflicted_session(&ordered);
        let reversed = vec![conflict(ID_7, OBJECT_2), conflict(ID_6, OBJECT_1)];
        assert!(CanonicalMergeConflicts::new(reversed.clone(), false).is_ok());
        assert!(ConflictListData::from_current_session(&ordered_session, reversed).is_err());
        let duplicate = vec![conflict(ID_6, OBJECT_1), conflict(ID_6, OBJECT_2)];
        assert!(CanonicalMergeConflicts::new(duplicate, false).is_err());
        assert!(AllowedMergeResolutions::new(vec![
            MergeResolution::Manual,
            MergeResolution::TakeOurs,
        ])
        .is_err());
    }

    #[test]
    fn merge_result_conflict_decision_binds_resolution_predecessor_and_digest() {
        let conflicts = vec![conflict(ID_6, OBJECT_1)];
        let session = conflicted_session(&conflicts);
        let decision = ConflictDecisionCommitAuthority::without_changed_receipt(
            &session,
            &conflicts,
            id(ID_7),
            &id(ID_6),
            MergeResolution::TakeOurs,
            digest('a'),
        )
        .unwrap()
        .into_data();
        assert!(decision.validates_decision_digest().unwrap());
        let mut substituted_digest = decision.clone();
        let ConflictDecisionData::TakeOurs(value) = &mut substituted_digest else {
            unreachable!()
        };
        value.decision_digest = digest('f');
        assert!(!substituted_digest.validates_decision_digest().unwrap());
        let value = serde_json::to_value(&decision).unwrap();
        assert_forbidden_fields_rejected_recursively::<ConflictDecisionData>(value.clone());

        let mut cross_leaf = value;
        cross_leaf["changeReceiptDigest"] = json!(digest('b').as_str());
        assert!(!schema_accepts::<ConflictDecisionData>(&cross_leaf));

        for (resolution, receipt) in [
            (MergeResolution::TakeTheirs, None),
            (MergeResolution::Combine, Some(digest('b'))),
            (MergeResolution::Manual, Some(digest('b'))),
        ] {
            let leaf = ConflictDecisionData::from_conflict_test_only(
                &session,
                &conflicts,
                id(ID_7),
                &id(ID_6),
                resolution,
                digest('a'),
                receipt,
            )
            .unwrap();
            assert!(leaf.validates_decision_digest().unwrap());
            assert!(schema_accepts::<ConflictDecisionData>(
                &serde_json::to_value(leaf).unwrap()
            ));
        }

        assert!(ConflictDecisionData::from_conflict_test_only(
            &session,
            &conflicts,
            id(ID_7),
            &id(ID_6),
            MergeResolution::Combine,
            digest('a'),
            None,
        )
        .is_err());

        let mut with_predecessor = conflicts.clone();
        with_predecessor[0].decision_state = ConflictDecisionState::current_test_only(id(ID_5));
        let predecessor_session = conflicted_session(&with_predecessor);
        let replacement = ConflictDecisionData::from_conflict_test_only(
            &predecessor_session,
            &with_predecessor,
            id(ID_7),
            &id(ID_6),
            MergeResolution::Manual,
            digest('a'),
            Some(digest('b')),
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(replacement).unwrap()["replacesDecisionId"],
            json!(ID_5),
        );
    }

    #[test]
    fn merge_result_receipt_decision_consumes_exact_current_changed_handle_atomically() {
        let conflicts = vec![conflict(ID_6, OBJECT_1)];
        let session = conflicted_session(&conflicts);
        let receipt = changed_receipt_for_conflict(&session, &conflicts[0], digest('c'));
        let workspace = current_resolution_workspace(&session);
        let current_handle = ResolutionChangeReceiptResumeHandle::selectable_from_changed_receipt(
            &receipt, &workspace,
        )
        .unwrap();
        let selectable = SelectableResolutionChangeReceiptAuthority::try_from_current(
            receipt,
            current_handle,
            &workspace,
        )
        .unwrap();
        let transition = ConflictDecisionCommitAuthority::with_changed_receipt(
            &session,
            &conflicts,
            ChangedConflictDecisionCommitInput::new(
                id(ID_7),
                id(ID_6),
                MergeResolution::Combine,
                digest('a'),
                MetadataPropertyAffectedTarget::new(
                    MetadataObjectId::parse(OBJECT_1).unwrap(),
                    PropertyPath::parse("Module.Text").unwrap(),
                ),
                digest('c'),
            ),
            selectable,
        )
        .unwrap();
        let (decision, consumed_handle) = transition.into_parts();
        assert!(decision.validates_decision_digest().unwrap());
        let consumed = serde_json::to_value(consumed_handle).unwrap();
        assert_eq!(consumed["consumed"], json!(true));
        assert_eq!(consumed["selectable"], json!(false));

        let workspace = current_resolution_workspace(&session);
        assert!(
            ResolutionChangeReceiptResumeHandle::selectable_from_changed_receipt(
                &no_change_receipt_for_conflict(&session, &conflicts[0]),
                &workspace,
            )
            .is_err()
        );

        let wrong_target = MetadataPropertyAffectedTarget::new(
            MetadataObjectId::parse(OBJECT_2).unwrap(),
            PropertyPath::parse("Module.Text").unwrap(),
        );
        assert!(ConflictDecisionCommitAuthority::with_changed_receipt(
            &session,
            &conflicts,
            ChangedConflictDecisionCommitInput::new(
                id(ID_7),
                id(ID_6),
                MergeResolution::Manual,
                digest('a'),
                wrong_target,
                digest('c'),
            ),
            selectable_changed_receipt(&session, &conflicts[0], digest('c')),
        )
        .is_err());
        assert!(ConflictDecisionCommitAuthority::with_changed_receipt(
            &session,
            &conflicts,
            ChangedConflictDecisionCommitInput::new(
                id(ID_7),
                id(ID_6),
                MergeResolution::Manual,
                digest('a'),
                MetadataPropertyAffectedTarget::new(
                    MetadataObjectId::parse(OBJECT_1).unwrap(),
                    PropertyPath::parse("Module.Text").unwrap(),
                ),
                digest('d'),
            ),
            selectable_changed_receipt(&session, &conflicts[0], digest('c')),
        )
        .is_err());

        let receipt = changed_receipt_for_conflict(&session, &conflicts[0], digest('c'));
        let fresh = changed_receipt_for_conflict(&session, &conflicts[0], digest('c'));
        let workspace = current_resolution_workspace(&session);
        let current = ResolutionChangeReceiptResumeHandle::selectable_from_changed_receipt(
            &receipt, &workspace,
        )
        .unwrap();
        let selected = SelectableResolutionChangeReceiptAuthority::try_from_current(
            receipt, current, &workspace,
        )
        .unwrap();
        let consumed = selected.into_consumed_handle().unwrap();
        assert!(
            SelectableResolutionChangeReceiptAuthority::try_from_current(
                fresh, consumed, &workspace,
            )
            .is_err()
        );

        let prior = changed_receipt_for_conflict(&session, &conflicts[0], digest('c'));
        let prior_handle = ResolutionChangeReceiptResumeHandle::selectable_from_changed_receipt(
            &prior, &workspace,
        )
        .unwrap();
        let target = MetadataPropertyAffectedTarget::new(
            conflicts[0].object_id.clone(),
            conflicts[0].property_path.clone(),
        );
        let later_authority = BranchedChangeReceiptAuthority::merge_resolution_changed_test_only(
            id(ID_2),
            target.clone(),
            digest('c'),
            digest('d'),
            vec![id(ID_3)],
            vec![MergeResolutionSelectableReceiptAuthority::test_only(
                id(ID_4),
                ChangeReceiptSequence::new(1).unwrap(),
                id(ID_5),
                target,
            )],
            MergeResolutionDecisionLineageAuthority::undecided_test_only(
                session.decision_set_digest().clone(),
            ),
            session.base_session_digest().clone(),
            id(ID_5),
            ChangeReceiptSequence::new(2).unwrap(),
        )
        .unwrap();
        let later = BranchedChangeReceipt::new(&later_authority).unwrap();
        let superseded = prior_handle.superseded_by_changed_receipt(&later).unwrap();
        assert!(
            SelectableResolutionChangeReceiptAuthority::try_from_current(
                prior, superseded, &workspace,
            )
            .is_err()
        );

        let wrong_session_workspace = MergeResolutionWorkspaceResumeHandle::new(
            id(ID_2),
            id(ID_5),
            session.base_session_digest().clone(),
        );
        let receipt = changed_receipt_for_conflict(&session, &conflicts[0], digest('c'));
        let handle = ResolutionChangeReceiptResumeHandle::selectable_from_changed_receipt(
            &receipt,
            &wrong_session_workspace,
        )
        .unwrap();
        let selected = SelectableResolutionChangeReceiptAuthority::try_from_current(
            receipt,
            handle,
            &wrong_session_workspace,
        )
        .unwrap();
        assert!(ConflictDecisionCommitAuthority::with_changed_receipt(
            &session,
            &conflicts,
            ChangedConflictDecisionCommitInput::new(
                id(ID_7),
                id(ID_6),
                MergeResolution::Manual,
                digest('a'),
                MetadataPropertyAffectedTarget::new(
                    MetadataObjectId::parse(OBJECT_1).unwrap(),
                    PropertyPath::parse("Module.Text").unwrap(),
                ),
                digest('c'),
            ),
            selected,
        )
        .is_err());
    }

    #[test]
    fn merge_result_current_head_authority_derives_canonical_replay_ids() {
        let mut states = vec![conflict(ID_6, OBJECT_1), conflict(ID_7, OBJECT_2)];
        let initial_session = conflicted_session(&states);
        let first = ConflictDecisionData::from_conflict_test_only(
            &initial_session,
            &states,
            id(ID_3),
            &id(ID_6),
            MergeResolution::TakeOurs,
            digest('a'),
            None,
        )
        .unwrap();
        states[0].decision_state = ConflictDecisionState::current_test_only(id(ID_3));
        let mid_session = conflicted_session(&states);
        let second = ConflictDecisionData::from_conflict_test_only(
            &mid_session,
            &states,
            id(ID_4),
            &id(ID_7),
            MergeResolution::TakeTheirs,
            digest('b'),
            None,
        )
        .unwrap();
        states[1].decision_state = ConflictDecisionState::current_test_only(id(ID_4));
        let final_session = conflicted_session(&states);

        let heads = CurrentConflictDecisionHeadsAuthority::new(
            &final_session,
            &states,
            vec![first.clone(), second.clone()],
        )
        .unwrap();
        assert_eq!(heads.applied_decision_ids(), &[id(ID_3), id(ID_4)]);
        assert!(CurrentConflictDecisionHeadsAuthority::new(
            &final_session,
            &states,
            vec![second.clone(), first.clone()],
        )
        .is_err());
        assert!(
            CurrentConflictDecisionHeadsAuthority::new(&final_session, &states, vec![]).is_err()
        );

        let mut pending = states.clone();
        pending[1].decision_state =
            ConflictDecisionState::replacement_pending_test_only(id(ID_4), id(ID_2));
        let pending_session = conflicted_session(&pending);
        assert!(CurrentConflictDecisionHeadsAuthority::new(
            &pending_session,
            &pending,
            vec![ConflictDecisionData::from_conflict_test_only(
                &initial_session,
                &[conflict(ID_6, OBJECT_1), conflict(ID_7, OBJECT_2)],
                id(ID_3),
                &id(ID_6),
                MergeResolution::TakeOurs,
                digest('a'),
                None,
            )
            .unwrap(),],
        )
        .is_err());

        let replay_heads = CurrentConflictDecisionHeadsAuthority::new(
            &final_session,
            &states,
            vec![first, second],
        )
        .unwrap();
        let replay = ResolvedReplayPreparationAuthority::new(
            &final_session,
            replay_heads,
            id(ID_2),
            digest('9'),
        )
        .unwrap();
        let (replay_session, replay_projection) = replay.into_parts();
        assert_eq!(
            replay_session.mode(),
            MergeSessionModeDigest::ResolvedReplay
        );
        assert!(replay_session
            .validates_resolved_projection(replay_projection.applied_decision_ids())
            .unwrap());
    }

    #[test]
    fn merge_result_adaptation_and_verification_bind_exact_difference_lineage() {
        let adaptation = AdaptedDeltaDecisionData::new_test_only(
            id(ID_1),
            id(ID_2),
            digest('a'),
            digest('b'),
            digest('c'),
        )
        .unwrap();
        assert!(adaptation.validates_digest().unwrap());
        assert_closed::<AdaptedDeltaDecisionDigestRecord>();

        let verification = MergeVerificationData::synchronized_adapted_test_only(
            SynchronizedAdaptedVerificationFixtureInput {
                verification_id: id(ID_3),
                session_id: id(ID_7),
                canonical_delta_digest: digest('a'),
                checkpoint_id: id(ID_4),
                difference_manifest_id: id(ID_5),
                difference_digest: digest('b'),
                validation_receipt_ids: vec![id(ID_6), id(ID_7)],
                support_audit_digest: digest('d'),
                selected_object_fingerprints: vec![],
            },
            &adaptation,
        )
        .unwrap();
        assert!(verification.validates_digest().unwrap());
        let value = serde_json::to_value(&verification).unwrap();
        assert_forbidden_fields_rejected_recursively::<MergeVerificationData>(value.clone());
        let mut gate_splice = value.clone();
        gate_splice["supportGateDigest"] = json!(digest('e').as_str());
        assert!(!schema_accepts::<MergeVerificationData>(&gate_splice));
        let mut missing_checkpoint = value;
        missing_checkpoint
            .as_object_mut()
            .unwrap()
            .remove("checkpointId");
        assert!(!schema_accepts::<MergeVerificationData>(
            &missing_checkpoint
        ));

        assert!(MergeVerificationData::synchronized_adapted_test_only(
            SynchronizedAdaptedVerificationFixtureInput {
                verification_id: id(ID_3),
                session_id: id(ID_7),
                canonical_delta_digest: digest('f'),
                checkpoint_id: id(ID_4),
                difference_manifest_id: id(ID_5),
                difference_digest: digest('b'),
                validation_receipt_ids: vec![],
                support_audit_digest: digest('d'),
                selected_object_fingerprints: vec![],
            },
            &adaptation,
        )
        .is_err());
        assert!(ValidationReceiptIds::new(vec![id(ID_6), id(ID_6)]).is_err());
        let root: RepositoryTargetIdentity =
            serde_json::from_value(json!({ "targetKind": "configurationRoot" })).unwrap();
        let object: RepositoryTargetIdentity = serde_json::from_value(json!({
            "targetKind": "developmentObject",
            "objectId": OBJECT_1,
        }))
        .unwrap();
        assert!(SelectedObjectFingerprints::new(vec![
            SelectedObjectFingerprint {
                target: object.clone(),
                fingerprint: digest('1'),
            },
            SelectedObjectFingerprint {
                target: root.clone(),
                fingerprint: digest('2'),
            },
        ])
        .is_err());
        assert!(SelectedObjectFingerprints::new(vec![
            SelectedObjectFingerprint {
                target: root.clone(),
                fingerprint: digest('1'),
            },
            SelectedObjectFingerprint {
                target: root,
                fingerprint: digest('2'),
            },
        ])
        .is_err());
        let mut substituted_verification = verification.clone();
        let MergeVerificationData::SynchronizedTaskAdapted(value) = &mut substituted_verification
        else {
            unreachable!()
        };
        value.verification_digest = digest('f');
        assert!(!substituted_verification.validates_digest().unwrap());

        let mut substituted_adaptation = adaptation.clone();
        substituted_adaptation.adaptation_decision_digest = digest('f');
        assert!(!substituted_adaptation.validates_digest().unwrap());
        assert_closed::<MergeVerificationDigestRecord>();
    }

    #[test]
    fn merge_result_adapted_flow_consumes_current_unexpected_session_lineage() {
        let session = resolved_session();
        let unexpected_observation =
            SynchronizedUnexpectedVerificationObservationAuthority::from_verifier_adapter(
                &session,
                resolved_task_verification_input(
                    &session,
                    id(ID_1),
                    digest('a'),
                    vec![id(ID_6)],
                    digest('d'),
                    vec![],
                ),
                id(ID_5),
                digest('b'),
            )
            .unwrap();
        let unexpected = MergeVerificationData::synchronized_unexpected_from_authorities(
            &session,
            unexpected_observation,
        )
        .unwrap();
        let (unexpected_data, current_unexpected) = unexpected.into_parts();
        assert!(unexpected_data.validates_digest().unwrap());

        let decision = AdaptedDeltaDecisionCommitAuthority::from_current_unexpected(
            current_unexpected,
            id(ID_2),
            digest('c'),
        )
        .unwrap();
        let (adaptation_data, current_adaptation) = decision.into_parts();
        assert!(adaptation_data.validates_digest().unwrap());
        let adapted_observation = AdaptedVerificationObservationAuthority::from_verifier_adapter(
            &session,
            &current_adaptation,
            resolved_task_verification_input(
                &session,
                id(ID_3),
                digest('a'),
                vec![id(ID_6)],
                digest('d'),
                vec![],
            ),
            id(ID_4),
        )
        .unwrap();
        let adapted = MergeVerificationData::synchronized_adapted_from_authorities(
            &session,
            current_adaptation,
            adapted_observation,
        )
        .unwrap();
        assert!(adapted.data().validates_digest().unwrap());
        let encoded = serde_json::to_value(adapted.data()).unwrap();
        assert_eq!(encoded["sessionId"], json!(ID_1));
        assert_eq!(encoded["adaptationDecisionId"], json!(ID_2));
    }

    #[test]
    fn merge_result_production_compare_and_session_chain_rejects_splices_and_classifier_reorder() {
        let project_selection = ProjectDeltaComparisonSelectionAuthority::from_operands(
            ComparisonOperandAuthority::task_current_from_workspace_adapter(digest('1')),
            ComparisonOperandAuthority::task_vendor_from_workspace_adapter(digest('2')),
        )
        .unwrap();
        let comparison_id = id(ID_4);
        let mut comparison_resolver = StaticComparisonResolver {
            observation: Some(
                PlatformComparisonCapabilitySnapshot::from_comparison_adapter(
                    project_selection.capability_scope(),
                    PlatformComparisonCapabilitySnapshotInput::from_comparison_adapter(
                        comparison_id.clone(),
                        id(ID_5),
                        id(ID_6),
                        digest('3'),
                        ClassifiedComparisonChangeBatchAuthority::from_classifier_adapter(vec![
                            id(ID_1),
                            id(ID_2),
                        ])
                        .unwrap(),
                        vec![Name::parse("platformSpecific").unwrap()],
                    ),
                ),
            ),
        };
        let comparison = ProjectDeltaComparisonAuthority::from_capability_resolver(
            &project_selection,
            &mut comparison_resolver,
        )
        .unwrap();
        assert_eq!(comparison.data().comparison_id(), &comparison_id);
        assert_eq!(comparison.data().change_count.get(), 2);

        let substituted_selection = ProjectDeltaComparisonSelectionAuthority::from_operands(
            ComparisonOperandAuthority::task_current_from_workspace_adapter(digest('1')),
            ComparisonOperandAuthority::baseline_distribution_from_delivery_adapter(
                id(ID_7),
                digest('2'),
            ),
        )
        .unwrap();
        let mut substituted_anchor_resolver = StaticComparisonResolver {
            observation: Some(
                PlatformComparisonCapabilitySnapshot::from_comparison_adapter(
                    substituted_selection.capability_scope(),
                    comparison_snapshot_input(id(ID_3), digest('4'), vec![]),
                ),
            ),
        };
        assert!(ProjectDeltaComparisonAuthority::from_capability_resolver(
            &project_selection,
            &mut substituted_anchor_resolver,
        )
        .is_err());

        let cross_scope_selection = MainIntegrationComparisonSelectionAuthority::from_operands(
            ComparisonOperandAuthority::task_current_from_workspace_adapter(digest('1')),
            ComparisonOperandAuthority::task_vendor_from_workspace_adapter(digest('2')),
        )
        .unwrap();
        let mut cross_scope_resolver = StaticComparisonResolver {
            observation: Some(
                PlatformComparisonCapabilitySnapshot::from_comparison_adapter(
                    cross_scope_selection.capability_scope(),
                    comparison_snapshot_input(id(ID_3), digest('4'), vec![]),
                ),
            ),
        };
        assert!(ProjectDeltaComparisonAuthority::from_capability_resolver(
            &project_selection,
            &mut cross_scope_resolver,
        )
        .is_err());

        let local_checkpoint = MergeVerificationData::local_checkpoint_valid_from_authority(
            LocalCheckpointVerificationObservationAuthority::valid_from_verifier_adapter(
                local_verification_input(id(ID_1), digest('5'), vec![], digest('6'), vec![]),
                id(ID_2),
            )
            .unwrap(),
        )
        .unwrap();
        let refresh = VerifiedMergeArtifactAuthority::refresh_distribution_from_delivery_adapter(
            id(ID_3),
            digest('7'),
        );
        let supported_inputs = SupportedUpdatePreparationAuthority::from_authorities(
            &local_checkpoint,
            refresh,
            &comparison,
        )
        .unwrap();
        let resolved_observation =
            SupportedUpdateSessionObservationAuthority::resolved_from_sandbox_adapter(
                &supported_inputs,
                id(ID_5),
                digest('8'),
                digest('9'),
                digest('a'),
            );
        let resolved = MergeSessionData::supported_update_from_authorities(
            &supported_inputs,
            resolved_observation,
        )
        .unwrap();
        assert!(matches!(
            resolved,
            MergeSessionData::SupportedUpdateResolved(_)
        ));

        let conflicting_inputs = SupportedUpdatePreparationAuthority::from_authorities(
            &local_checkpoint,
            VerifiedMergeArtifactAuthority::refresh_distribution_from_delivery_adapter(
                id(ID_3),
                digest('7'),
            ),
            &comparison,
        )
        .unwrap();
        let conflict_batch = classifier_batch(vec![classifier_row_input(
            ID_6,
            OBJECT_1,
            "Catalog.Product",
            vec![MergeResolution::TakeOurs, MergeResolution::Manual],
        )]);
        let conflicted_observation =
            SupportedUpdateSessionObservationAuthority::conflicted_from_sandbox_adapter(
                &conflicting_inputs,
                id(ID_1),
                digest('8'),
                digest('9'),
                Some(id(ID_7)),
                conflict_batch,
            );
        let conflicted = MergeSessionData::supported_update_from_authorities(
            &conflicting_inputs,
            conflicted_observation,
        )
        .unwrap();
        assert!(matches!(
            conflicted,
            MergeSessionData::SupportedUpdateConflicted(_)
        ));

        let other_checkpoint = MergeVerificationData::local_checkpoint_valid_from_authority(
            LocalCheckpointVerificationObservationAuthority::valid_from_verifier_adapter(
                local_verification_input(id(ID_7), digest('5'), vec![], digest('6'), vec![]),
                id(ID_6),
            )
            .unwrap(),
        )
        .unwrap();
        let spliced_inputs = SupportedUpdatePreparationAuthority::from_authorities(
            &other_checkpoint,
            VerifiedMergeArtifactAuthority::refresh_distribution_from_delivery_adapter(
                id(ID_3),
                digest('7'),
            ),
            &comparison,
        )
        .unwrap();
        let observation_for_other_inputs =
            SupportedUpdateSessionObservationAuthority::resolved_from_sandbox_adapter(
                &supported_inputs,
                id(ID_5),
                digest('8'),
                digest('9'),
                digest('a'),
            );
        assert!(MergeSessionData::supported_update_from_authorities(
            &spliced_inputs,
            observation_for_other_inputs,
        )
        .is_err());

        let synchronized_checkpoint =
            MergeVerificationData::synchronized_equivalent_from_authorities(
                &resolved,
                SynchronizedVerificationObservationAuthority::equivalent_from_verifier_adapter(
                    &resolved,
                    resolved_task_verification_input(
                        &resolved,
                        id(ID_6),
                        digest('3'),
                        vec![],
                        digest('4'),
                        vec![],
                    ),
                    id(ID_7),
                )
                .unwrap(),
            )
            .unwrap();
        let preflight = ready_preflight_authority_fixture_test_only();
        let main_comparison_id = preflight.comparison_id().clone();
        let main_selection = MainIntegrationComparisonSelectionAuthority::from_operands(
            ComparisonOperandAuthority::repository_from_workspace_adapter(digest('1')),
            ComparisonOperandAuthority::task_current_from_workspace_adapter(digest('2')),
        )
        .unwrap();
        let mut main_comparison_resolver = StaticComparisonResolver {
            observation: Some(
                PlatformComparisonCapabilitySnapshot::from_comparison_adapter(
                    main_selection.capability_scope(),
                    comparison_snapshot_input(main_comparison_id, digest('5'), vec![]),
                ),
            ),
        };
        let main_comparison = MainIntegrationComparisonAuthority::from_capability_resolver(
            &main_selection,
            &mut main_comparison_resolver,
        )
        .unwrap();
        let ordinary = VerifiedMergeArtifactAuthority::ordinary_result_from_delivery_adapter(
            preflight.ordinary_result_artifact_id().clone(),
            digest('6'),
        );
        let main_inputs = MainIntegrationPreparationInputAuthority::from_authorities(
            &synchronized_checkpoint,
            ordinary,
            &main_comparison,
            preflight,
        )
        .unwrap();
        let main_observation = MainIntegrationSessionObservationAuthority::from_sandbox_adapter(
            &main_inputs,
            id(ID_1),
            digest('7'),
        );
        let main =
            MergeSessionData::main_integration_from_authorities(main_inputs, main_observation)
                .unwrap();
        let encoded =
            serde_json::to_value(MainIntegrationPreparationData::from_authority(main)).unwrap();
        assert_eq!(encoded["session"]["mode"], json!("mainIntegration"));
    }

    #[test]
    fn merge_result_checkpoint_typestate_excludes_both_cross_scope_preparation_splices() {
        fn supported_signature(
            _producer: fn(
                &ValidatedLocalCheckpointVerificationAuthority,
                VerifiedMergeArtifactAuthority,
                &ProjectDeltaComparisonAuthority,
            )
                -> Result<SupportedUpdatePreparationAuthority, MergeResultContractError>,
        ) {
        }
        fn main_signature(
            _producer: fn(
                &ValidatedSynchronizedCheckpointVerificationAuthority,
                VerifiedMergeArtifactAuthority,
                &MainIntegrationComparisonAuthority,
                ReadySupportPreflightAuthority,
            ) -> Result<
                MainIntegrationPreparationInputAuthority,
                MergeResultContractError,
            >,
        ) {
        }

        supported_signature(SupportedUpdatePreparationAuthority::from_authorities);
        main_signature(MainIntegrationPreparationInputAuthority::from_authorities);
        assert_ne!(
            std::any::TypeId::of::<ValidatedLocalCheckpointVerificationAuthority>(),
            std::any::TypeId::of::<ValidatedSynchronizedCheckpointVerificationAuthority>(),
        );
    }

    #[test]
    fn merge_result_supported_update_rejects_ordinary_result_artifact_in_release() {
        trait RejectionProbe {
            fn rejected(self) -> bool;
        }
        impl RejectionProbe for SupportedUpdatePreparationAuthority {
            fn rejected(self) -> bool {
                false
            }
        }
        impl RejectionProbe for Result<SupportedUpdatePreparationAuthority, MergeResultContractError> {
            fn rejected(self) -> bool {
                self.is_err()
            }
        }

        let local = MergeVerificationData::local_checkpoint_valid_from_authority(
            LocalCheckpointVerificationObservationAuthority::valid_from_verifier_adapter(
                local_verification_input(id(ID_1), digest('1'), vec![], digest('2'), vec![]),
                id(ID_2),
            )
            .unwrap(),
        )
        .unwrap();
        let selection = ProjectDeltaComparisonSelectionAuthority::from_operands(
            ComparisonOperandAuthority::task_current_from_workspace_adapter(digest('3')),
            ComparisonOperandAuthority::task_vendor_from_workspace_adapter(digest('4')),
        )
        .unwrap();
        let mut resolver = StaticComparisonResolver {
            observation: Some(
                PlatformComparisonCapabilitySnapshot::from_comparison_adapter(
                    selection.capability_scope(),
                    comparison_snapshot_input(id(ID_3), digest('5'), vec![]),
                ),
            ),
        };
        let comparison =
            ProjectDeltaComparisonAuthority::from_capability_resolver(&selection, &mut resolver)
                .unwrap();
        let ordinary = VerifiedMergeArtifactAuthority::ordinary_result_from_delivery_adapter(
            id(ID_6),
            digest('6'),
        );

        assert!(SupportedUpdatePreparationAuthority::from_authorities(
            &local,
            ordinary,
            &comparison,
        )
        .rejected());
    }

    #[test]
    fn merge_result_comparison_authority_binds_scope_operand_role_and_artifact_identity() {
        let baseline = ComparisonOperandAuthority::baseline_distribution_from_delivery_adapter(
            id(ID_1),
            digest('1'),
        );
        let refresh = ComparisonOperandAuthority::refresh_distribution_from_delivery_adapter(
            id(ID_2),
            digest('2'),
        );
        let ordinary = ComparisonOperandAuthority::ordinary_result_from_delivery_adapter(
            id(ID_3),
            digest('3'),
        );

        assert!(
            ProjectDeltaComparisonSelectionAuthority::from_operands(baseline, refresh,).is_ok()
        );
        assert!(ProjectDeltaComparisonSelectionAuthority::from_operands(
            ComparisonOperandAuthority::task_current_from_workspace_adapter(digest('4')),
            ordinary,
        )
        .is_err());
        assert!(MainIntegrationComparisonSelectionAuthority::from_operands(
            ComparisonOperandAuthority::repository_from_workspace_adapter(digest('5')),
            ComparisonOperandAuthority::refresh_distribution_from_delivery_adapter(
                id(ID_4),
                digest('6'),
            ),
        )
        .is_err());

        fn supported_signature(
            _producer: fn(
                &ValidatedLocalCheckpointVerificationAuthority,
                VerifiedMergeArtifactAuthority,
                &ProjectDeltaComparisonAuthority,
            )
                -> Result<SupportedUpdatePreparationAuthority, MergeResultContractError>,
        ) {
        }
        fn main_signature(
            _producer: fn(
                &ValidatedSynchronizedCheckpointVerificationAuthority,
                VerifiedMergeArtifactAuthority,
                &MainIntegrationComparisonAuthority,
                ReadySupportPreflightAuthority,
            ) -> Result<
                MainIntegrationPreparationInputAuthority,
                MergeResultContractError,
            >,
        ) {
        }
        supported_signature(SupportedUpdatePreparationAuthority::from_authorities);
        main_signature(MainIntegrationPreparationInputAuthority::from_authorities);
    }

    #[test]
    fn merge_result_classifier_batch_rejects_reorder_without_caller_ordinals() {
        fn classifier_row(conflict_id: &str, object_id: &str) -> ClassifiedMergeConflictInput {
            ClassifiedMergeConflictInput::from_classifier_adapter(
                MergeConflictIdentityInput::from_classifier_adapter(
                    id(conflict_id),
                    MetadataObjectId::parse(object_id).unwrap(),
                    RepositoryTargetDisplay::parse("Catalog.Product").unwrap(),
                    PropertyPath::parse("Module.Text").unwrap(),
                    MergeConflictKind::TwiceChanged,
                ),
                MergeConflictContentInput::from_classifier_adapter(
                    digest('a'),
                    digest('b'),
                    digest('c'),
                    vec![MergeResolution::TakeOurs],
                ),
            )
        }

        let original = vec![
            classifier_row(ID_6, OBJECT_1),
            classifier_row(ID_7, OBJECT_2),
        ];
        let order =
            MergeConflictClassifierOrderAuthority::from_classifier_adapter(&original).unwrap();
        let reordered = vec![original[1].clone(), original[0].clone()];

        assert!(
            ClassifiedMergeConflictBatchAuthority::from_classifier_snapshot_adapter(
                order, reordered,
            )
            .is_err()
        );
    }

    #[test]
    fn merge_result_production_verifier_reaches_all_ten_leaves_and_rejects_lineage_splices() {
        let local_valid = MergeVerificationData::local_checkpoint_valid_from_authority(
            LocalCheckpointVerificationObservationAuthority::valid_from_verifier_adapter(
                local_verification_input(
                    id(ID_1),
                    digest('1'),
                    vec![id(ID_3)],
                    digest('2'),
                    vec![],
                ),
                id(ID_2),
            )
            .unwrap(),
        )
        .unwrap();
        let local_invalid = MergeVerificationData::local_checkpoint_invalid_from_authority(
            LocalCheckpointVerificationObservationAuthority::invalid_from_verifier_adapter(
                local_verification_input(
                    id(ID_2),
                    digest('1'),
                    vec![id(ID_3)],
                    digest('2'),
                    vec![],
                ),
            )
            .unwrap(),
        )
        .unwrap();

        let session = resolved_session();
        let synchronized_equivalent =
            MergeVerificationData::synchronized_equivalent_from_authorities(
                &session,
                SynchronizedVerificationObservationAuthority::equivalent_from_verifier_adapter(
                    &session,
                    resolved_task_verification_input(
                        &session,
                        id(ID_3),
                        digest('3'),
                        vec![id(ID_5)],
                        digest('4'),
                        vec![],
                    ),
                    id(ID_4),
                )
                .unwrap(),
            )
            .unwrap();
        let synchronized_invalid = MergeVerificationData::synchronized_invalid_from_authorities(
            &session,
            SynchronizedVerificationObservationAuthority::invalid_from_verifier_adapter(
                &session,
                resolved_task_verification_input(
                    &session,
                    id(ID_4),
                    digest('3'),
                    vec![id(ID_5)],
                    digest('4'),
                    vec![],
                ),
            )
            .unwrap(),
        )
        .unwrap();
        let unexpected = MergeVerificationData::synchronized_unexpected_from_authorities(
            &session,
            SynchronizedUnexpectedVerificationObservationAuthority::from_verifier_adapter(
                &session,
                resolved_task_verification_input(
                    &session,
                    id(ID_5),
                    digest('5'),
                    vec![],
                    digest('7'),
                    vec![],
                ),
                id(ID_6),
                digest('6'),
            )
            .unwrap(),
        )
        .unwrap();
        let (unexpected_data, current_unexpected) = unexpected.into_parts();
        let decision = AdaptedDeltaDecisionCommitAuthority::from_current_unexpected(
            current_unexpected,
            id(ID_7),
            digest('8'),
        )
        .unwrap();
        let (_, current_adaptation) = decision.into_parts();
        let adapted_observation = AdaptedVerificationObservationAuthority::from_verifier_adapter(
            &session,
            &current_adaptation,
            resolved_task_verification_input(
                &session,
                id(ID_6),
                digest('5'),
                vec![],
                digest('9'),
                vec![],
            ),
            id(ID_7),
        )
        .unwrap();
        let synchronized_adapted = MergeVerificationData::synchronized_adapted_from_authorities(
            &session,
            current_adaptation,
            adapted_observation,
        )
        .unwrap();

        let make_planning =
            |main: MainIntegrationSessionData, preflight: ReadySupportPreflightAuthority| {
                let session = MergeSessionData::MainIntegration(main.clone());
                let preparation =
                    ValidatedMainIntegrationPreparationAuthority::new(preflight, main).unwrap();
                ValidatedRepositoryPlanSessionProjection::from_main_preparation(
                    preparation,
                    ResolvedApplyDecisionProjectionAuthority::for_no_conflict_session(&session)
                        .unwrap(),
                )
                .unwrap()
            };
        let preflight = ready_preflight_authority_fixture_test_only();
        let main = consumer_fixture_main_session(&preflight);
        let planning = make_planning(main.clone(), preflight);
        let sandbox_valid_observation =
            MainSandboxVerificationObservationAuthority::valid_from_verifier_adapter(
                &planning,
                main_sandbox_verification_input(
                    &planning,
                    id(ID_1),
                    digest('a'),
                    vec![],
                    digest('b'),
                    vec![],
                ),
            )
            .unwrap();
        let main_sandbox_valid = MergeVerificationData::main_sandbox_valid_from_authorities(
            planning,
            sandbox_valid_observation,
        )
        .unwrap();

        let preflight = ready_preflight_authority_fixture_test_only();
        let invalid_main = main_integration_session_for_preflight(&preflight);
        let invalid_planning = make_planning(invalid_main, preflight);
        let sandbox_invalid_observation =
            MainSandboxVerificationObservationAuthority::invalid_from_verifier_adapter(
                &invalid_planning,
                main_sandbox_verification_input(
                    &invalid_planning,
                    id(ID_2),
                    digest('a'),
                    vec![],
                    digest('b'),
                    vec![],
                ),
            )
            .unwrap();
        let main_sandbox_invalid = MergeVerificationData::main_sandbox_invalid_from_authorities(
            invalid_planning,
            sandbox_invalid_observation,
        )
        .unwrap();

        let make_lineage = |merge_receipt_id: UnicaId| {
            validated_consumed_original_merge_context_fixture_test_only(
                merge_receipt_id,
                digest('e'),
            )
        };
        let lineage = make_lineage(id(ID_5));
        let integration_valid_observation =
            MainIntegrationVerificationObservationAuthority::valid_from_verifier_adapter(
                main_integration_verification_input(
                    lineage,
                    id(ID_3),
                    digest('a'),
                    vec![],
                    digest('b'),
                    vec![],
                ),
            );
        let main_integration_valid =
            MergeVerificationData::main_integration_valid_from_authorities(
                integration_valid_observation,
            )
            .unwrap();
        let invalid_lineage = make_lineage(id(ID_6));
        let integration_invalid_observation =
            MainIntegrationVerificationObservationAuthority::invalid_from_verifier_adapter(
                main_integration_verification_input(
                    invalid_lineage,
                    id(ID_4),
                    digest('a'),
                    vec![],
                    digest('b'),
                    vec![],
                ),
            );
        let main_integration_invalid =
            MergeVerificationData::main_integration_invalid_from_authorities(
                integration_invalid_observation,
            )
            .unwrap();

        let all_leaves = vec![
            local_valid.data(),
            local_invalid,
            synchronized_equivalent.data(),
            synchronized_adapted.data(),
            unexpected_data,
            synchronized_invalid,
            main_sandbox_valid.data(),
            main_sandbox_invalid,
            main_integration_valid.data(),
            main_integration_invalid.data(),
        ];
        assert_eq!(all_leaves.len(), 10);
        assert!(all_leaves
            .iter()
            .all(|verification| verification.validates_digest().unwrap()));

        let session_for_observation = resolved_session();
        let cross_session_observation =
            SynchronizedVerificationObservationAuthority::invalid_from_verifier_adapter(
                &session_for_observation,
                resolved_task_verification_input(
                    &session_for_observation,
                    id(ID_1),
                    digest('1'),
                    vec![],
                    digest('2'),
                    vec![],
                ),
            )
            .unwrap();
        let mut other_session = resolved_session();
        let MergeSessionData::SupportedUpdateResolved(other) = &mut other_session else {
            unreachable!()
        };
        other.session_id = id(ID_7);
        assert!(
            MergeVerificationData::synchronized_invalid_from_authorities(
                &other_session,
                cross_session_observation,
            )
            .is_err()
        );

        let lineage_for_observation = make_lineage(id(ID_5));
        let cross_receipt_observation =
            MainIntegrationVerificationObservationAuthority::invalid_from_verifier_adapter(
                main_integration_verification_input(
                    lineage_for_observation,
                    id(ID_1),
                    digest('1'),
                    vec![],
                    digest('2'),
                    vec![],
                ),
            );
        let retained_invalid = MergeVerificationData::main_integration_invalid_from_authorities(
            cross_receipt_observation,
        )
        .unwrap();
        let MergeVerificationData::MainIntegrationInvalid(retained_invalid_data) =
            retained_invalid.data()
        else {
            unreachable!()
        };
        assert_eq!(retained_invalid_data.merge_receipt_id, id(ID_5));

        let unexpected_a = MergeVerificationData::synchronized_unexpected_from_authorities(
            &session,
            SynchronizedUnexpectedVerificationObservationAuthority::from_verifier_adapter(
                &session,
                resolved_task_verification_input(
                    &session,
                    id(ID_1),
                    digest('1'),
                    vec![],
                    digest('3'),
                    vec![],
                ),
                id(ID_2),
                digest('2'),
            )
            .unwrap(),
        )
        .unwrap();
        let (_, current_a) = unexpected_a.into_parts();
        let (_, decision_a) = AdaptedDeltaDecisionCommitAuthority::from_current_unexpected(
            current_a,
            id(ID_3),
            digest('4'),
        )
        .unwrap()
        .into_parts();
        let difference_a_observation =
            AdaptedVerificationObservationAuthority::from_verifier_adapter(
                &session,
                &decision_a,
                resolved_task_verification_input(
                    &session,
                    id(ID_4),
                    digest('1'),
                    vec![],
                    digest('5'),
                    vec![],
                ),
                id(ID_5),
            )
            .unwrap();
        let unexpected_b = MergeVerificationData::synchronized_unexpected_from_authorities(
            &session,
            SynchronizedUnexpectedVerificationObservationAuthority::from_verifier_adapter(
                &session,
                resolved_task_verification_input(
                    &session,
                    id(ID_2),
                    digest('1'),
                    vec![],
                    digest('3'),
                    vec![],
                ),
                id(ID_6),
                digest('6'),
            )
            .unwrap(),
        )
        .unwrap();
        let (_, current_b) = unexpected_b.into_parts();
        let (_, decision_b) = AdaptedDeltaDecisionCommitAuthority::from_current_unexpected(
            current_b,
            id(ID_7),
            digest('4'),
        )
        .unwrap()
        .into_parts();
        assert!(
            MergeVerificationData::synchronized_adapted_from_authorities(
                &session,
                decision_b,
                difference_a_observation,
            )
            .is_err()
        );

        let gate_preflight_a = ready_preflight_authority_fixture_test_only();
        let gate_main_a = consumer_fixture_main_session(&gate_preflight_a);
        let gate_planning_a = make_planning(gate_main_a, gate_preflight_a);
        let cross_gate_observation =
            MainSandboxVerificationObservationAuthority::invalid_from_verifier_adapter(
                &gate_planning_a,
                main_sandbox_verification_input(
                    &gate_planning_a,
                    id(ID_1),
                    digest('1'),
                    vec![],
                    digest('2'),
                    vec![],
                ),
            )
            .unwrap();
        let gate_preflight_b = ready_preflight_authority_fixture_test_only();
        let gate_main_b = main_integration_session_for_preflight(&gate_preflight_b);
        let gate_planning_b = make_planning(gate_main_b, gate_preflight_b);
        assert!(
            MergeVerificationData::main_sandbox_invalid_from_authorities(
                gate_planning_b,
                cross_gate_observation,
            )
            .is_err()
        );
    }

    #[test]
    fn merge_result_configured_check_batch_rejects_freshly_renumbered_reorder_and_replay() {
        struct StaticConfiguredCheckPort {
            snapshot_input: Option<ConfiguredValidationReceiptBatchSnapshotInput>,
        }

        impl ConfiguredValidationCheckExecutionPort for StaticConfiguredCheckPort {
            fn execute(
                &mut self,
                request: ConfiguredValidationCheckExecutionRequest<'_>,
            ) -> Result<ConfiguredValidationReceiptBatchSnapshot, MergeResultContractError>
            {
                let input = self.snapshot_input.take().ok_or(MergeResultContractError(
                    "configured-check snapshot replayed",
                ))?;
                Ok(request.complete(input))
            }
        }

        struct LegacyRenumberedRow {
            fresh_ordinal: usize,
            check_id: Name,
            receipt_id: UnicaId,
        }

        let syntax = Name::parse("syntax").unwrap();
        let support = Name::parse("support").unwrap();
        let plan = ConfiguredValidationCheckPlanAuthority::from_configuration_adapter(vec![
            syntax.clone(),
            support.clone(),
        ])
        .unwrap();
        let local_selection =
            ConfiguredValidationExecutionSelectionAuthority::local_checkpoint(&plan, id(ID_1));
        let freshly_renumbered = [
            LegacyRenumberedRow {
                fresh_ordinal: 0,
                check_id: support,
                receipt_id: id(ID_3),
            },
            LegacyRenumberedRow {
                fresh_ordinal: 1,
                check_id: syntax,
                receipt_id: id(ID_2),
            },
        ];
        assert!(freshly_renumbered
            .iter()
            .enumerate()
            .all(|(index, row)| row.fresh_ordinal == index));
        let mut reordered_port = StaticConfiguredCheckPort {
            snapshot_input: Some(
                ConfiguredValidationReceiptBatchSnapshotInput::from_execution_adapter(
                    freshly_renumbered
                        .iter()
                        .map(|row| row.check_id.clone())
                        .collect(),
                    freshly_renumbered
                        .into_iter()
                        .map(|row| row.receipt_id)
                        .collect(),
                ),
            ),
        };
        let blocked = ConfiguredValidationReceiptBatchAuthority::from_execution_port(
            local_selection,
            &mut reordered_port,
        )
        .unwrap_err();
        let (local_selection, _) = blocked.into_recovery_parts();

        let mut valid_port = StaticConfiguredCheckPort {
            snapshot_input: Some(
                ConfiguredValidationReceiptBatchSnapshotInput::from_execution_adapter(
                    vec![
                        Name::parse("syntax").unwrap(),
                        Name::parse("support").unwrap(),
                    ],
                    vec![id(ID_2), id(ID_3)],
                ),
            ),
        };
        let batch = ConfiguredValidationReceiptBatchAuthority::from_execution_port(
            local_selection,
            &mut valid_port,
        )
        .unwrap();
        let replay_selection =
            ConfiguredValidationExecutionSelectionAuthority::local_checkpoint(&plan, id(ID_1));
        assert!(
            ConfiguredValidationReceiptBatchAuthority::from_execution_port(
                replay_selection,
                &mut valid_port,
            )
            .is_err()
        );
        let observation =
            LocalCheckpointVerificationObservationAuthority::valid_from_verifier_adapter(
                VerificationObservationInputAuthority::from_verifier_adapter(
                    id(ID_1),
                    digest('1'),
                    batch,
                    digest('2'),
                    vec![],
                )
                .unwrap(),
                id(ID_4),
            )
            .unwrap();
        assert!(MergeVerificationData::local_checkpoint_valid_from_authority(observation).is_ok());

        let session_a = resolved_session();
        let task_selection = ConfiguredValidationExecutionSelectionAuthority::resolved_task(
            &plan,
            &session_a,
            id(ID_5),
        )
        .unwrap();
        let mut task_port = StaticConfiguredCheckPort {
            snapshot_input: Some(
                ConfiguredValidationReceiptBatchSnapshotInput::from_execution_adapter(
                    vec![
                        Name::parse("syntax").unwrap(),
                        Name::parse("support").unwrap(),
                    ],
                    vec![id(ID_6), id(ID_7)],
                ),
            ),
        };
        let task_batch = ConfiguredValidationReceiptBatchAuthority::from_execution_port(
            task_selection,
            &mut task_port,
        )
        .unwrap();
        let mut session_b = resolved_session();
        if let MergeSessionData::SupportedUpdateResolved(data) = &mut session_b {
            data.session_id = id(ID_7);
        }
        assert!(
            SynchronizedVerificationObservationAuthority::invalid_from_verifier_adapter(
                &session_b,
                VerificationObservationInputAuthority::from_verifier_adapter(
                    id(ID_5),
                    digest('1'),
                    task_batch,
                    digest('2'),
                    vec![],
                )
                .unwrap(),
            )
            .is_err()
        );
    }

    #[test]
    fn merge_result_main_validation_batch_binds_exact_receipt_fingerprint_and_port_context() {
        struct FingerprintObservingPort {
            expected_verification_id: UnicaId,
            expected_session_id: UnicaId,
            expected_resolved_session_digest: Sha256Digest,
            expected_merge_receipt_id: UnicaId,
            expected_integration_set_digest: Sha256Digest,
            expected_result_fingerprint: Sha256Digest,
            observed_exact_context: bool,
            snapshot_input: Option<ConfiguredValidationReceiptBatchSnapshotInput>,
        }

        impl ConfiguredValidationCheckExecutionPort for FingerprintObservingPort {
            fn execute(
                &mut self,
                request: ConfiguredValidationCheckExecutionRequest<'_>,
            ) -> Result<ConfiguredValidationReceiptBatchSnapshot, MergeResultContractError>
            {
                let ConfiguredValidationCheckExecutionContext::MainIntegration {
                    verification_id,
                    session_id,
                    resolved_session_digest,
                    merge_receipt_id,
                    integration_set_digest,
                    result_fingerprint,
                    ..
                } = request.context()
                else {
                    return Err(MergeResultContractError(
                        "expected main-integration validation context",
                    ));
                };
                self.observed_exact_context = verification_id == &self.expected_verification_id
                    && session_id == &self.expected_session_id
                    && resolved_session_digest == &self.expected_resolved_session_digest
                    && merge_receipt_id == &self.expected_merge_receipt_id
                    && integration_set_digest == &self.expected_integration_set_digest
                    && result_fingerprint == &self.expected_result_fingerprint;
                let input = self.snapshot_input.take().ok_or(MergeResultContractError(
                    "main validation execution replayed",
                ))?;
                Ok(request.complete(input))
            }
        }

        let make_lineage = |result_fingerprint: Sha256Digest| {
            validated_consumed_original_merge_context_fixture_test_only(
                id(ID_5),
                result_fingerprint,
            )
        };
        let lineage_a = make_lineage(digest('6'));
        let lineage_b = make_lineage(digest('8'));
        let receipt_a = lineage_a.receipt();
        let receipt_b = lineage_b.receipt();
        assert_eq!(receipt_a.session_id(), receipt_b.session_id());
        assert_eq!(
            receipt_a.resolved_session_digest(),
            receipt_b.resolved_session_digest()
        );
        assert_eq!(receipt_a.merge_receipt_id(), receipt_b.merge_receipt_id());
        assert_eq!(
            receipt_a.integration_set_digest(),
            receipt_b.integration_set_digest()
        );
        assert_ne!(
            receipt_a.result_fingerprint(),
            receipt_b.result_fingerprint()
        );
        let expected_session_id = receipt_a.session_id().clone();
        let expected_resolved_session_digest = receipt_a.resolved_session_digest().clone();
        let expected_merge_receipt_id = receipt_a.merge_receipt_id().clone();
        let expected_integration_set_digest = receipt_a.integration_set_digest().clone();
        let expected_result_fingerprint = receipt_a.result_fingerprint().clone();

        let plan =
            ConfiguredValidationCheckPlanAuthority::from_configuration_adapter(vec![]).unwrap();
        let verification_id = id(ID_3);
        let selection = ConfiguredValidationExecutionSelectionAuthority::main_integration(
            &plan,
            lineage_a,
            verification_id.clone(),
        );
        let mut port = FingerprintObservingPort {
            expected_verification_id: verification_id.clone(),
            expected_session_id,
            expected_resolved_session_digest,
            expected_merge_receipt_id,
            expected_integration_set_digest,
            expected_result_fingerprint: expected_result_fingerprint.clone(),
            observed_exact_context: false,
            snapshot_input: Some(
                ConfiguredValidationReceiptBatchSnapshotInput::from_execution_adapter(
                    vec![],
                    vec![],
                ),
            ),
        };
        let batch =
            ConfiguredValidationReceiptBatchAuthority::from_execution_port(selection, &mut port)
                .unwrap();
        assert!(port.observed_exact_context);

        let observation =
            MainIntegrationVerificationObservationAuthority::valid_from_verifier_adapter(
                VerificationObservationInputAuthority::from_verifier_adapter(
                    verification_id,
                    digest('9'),
                    batch,
                    digest('a'),
                    vec![],
                )
                .unwrap(),
            );
        let verified =
            MergeVerificationData::main_integration_valid_from_authorities(observation).unwrap();
        assert_eq!(verified.result_fingerprint(), &expected_result_fingerprint);
    }

    #[test]
    fn merge_result_verification_schema_covers_all_ten_physical_leaves() {
        let common = json!({
            "verificationId": ID_1,
            "canonicalDeltaDigest": digest('a').as_str(),
            "validationReceiptIds": [],
            "supportAuditDigest": digest('b').as_str(),
            "selectedObjectFingerprints": [],
            "verificationDigest": digest('c').as_str(),
        });
        let history_evidence = json!({
            "gateObservedCursor": {
                "throughVersion": "10",
                "historyPrefixDigest": digest('d').as_str(),
            },
            "classifiedThroughCursor": {
                "throughVersion": "10",
                "historyPrefixDigest": digest('d').as_str(),
            },
            "partition": {
                "fromExclusive": {
                    "throughVersion": "10",
                    "historyPrefixDigest": digest('d').as_str(),
                },
                "throughInclusive": {
                    "throughVersion": "10",
                    "historyPrefixDigest": digest('d').as_str(),
                },
                "entries": [],
                "partitionDigest": digest('e').as_str(),
            },
            "relevantBaselineDigest": digest('f').as_str(),
            "evidenceDigest": digest('0').as_str(),
        });
        let with = |fields: &[(&str, Value)]| {
            let mut value = common.clone();
            let object = value.as_object_mut().unwrap();
            for (name, field) in fields {
                object.insert((*name).to_owned(), field.clone());
            }
            value
        };
        let fixtures = [
            with(&[
                ("scope", json!("localCheckpoint")),
                ("outcome", json!("valid")),
                ("checkpointId", json!(ID_2)),
            ]),
            with(&[
                ("scope", json!("localCheckpoint")),
                ("outcome", json!("invalid")),
            ]),
            with(&[
                ("scope", json!("synchronizedTask")),
                ("outcome", json!("equivalent")),
                ("sessionId", json!(ID_7)),
                ("checkpointId", json!(ID_2)),
            ]),
            with(&[
                ("scope", json!("synchronizedTask")),
                ("outcome", json!("adapted")),
                ("sessionId", json!(ID_7)),
                ("checkpointId", json!(ID_2)),
                ("differenceManifestId", json!(ID_3)),
                ("differenceDigest", json!(digest('1').as_str())),
                ("adaptationDecisionId", json!(ID_4)),
            ]),
            with(&[
                ("scope", json!("synchronizedTask")),
                ("outcome", json!("unexpected")),
                ("sessionId", json!(ID_7)),
                ("differenceManifestId", json!(ID_3)),
                ("differenceDigest", json!(digest('1').as_str())),
            ]),
            with(&[
                ("scope", json!("synchronizedTask")),
                ("outcome", json!("invalid")),
                ("sessionId", json!(ID_7)),
            ]),
            with(&[
                ("scope", json!("mainSandbox")),
                ("outcome", json!("valid")),
                ("sessionId", json!(ID_7)),
                ("supportGateDigest", json!(digest('2').as_str())),
                ("supportGateHistoryEvidence", history_evidence.clone()),
            ]),
            with(&[
                ("scope", json!("mainSandbox")),
                ("outcome", json!("invalid")),
                ("sessionId", json!(ID_7)),
                ("supportGateDigest", json!(digest('2').as_str())),
                ("supportGateHistoryEvidence", history_evidence.clone()),
            ]),
            with(&[
                ("scope", json!("mainIntegration")),
                ("outcome", json!("valid")),
                ("sessionId", json!(ID_7)),
                ("mergeReceiptId", json!(ID_5)),
                ("integrationSetDigest", json!(digest('3').as_str())),
                ("supportGateDigest", json!(digest('2').as_str())),
                ("supportGateHistoryEvidence", history_evidence.clone()),
            ]),
            with(&[
                ("scope", json!("mainIntegration")),
                ("outcome", json!("invalid")),
                ("sessionId", json!(ID_7)),
                ("mergeReceiptId", json!(ID_5)),
                ("integrationSetDigest", json!(digest('3').as_str())),
                ("supportGateDigest", json!(digest('2').as_str())),
                ("supportGateHistoryEvidence", history_evidence),
            ]),
        ];

        for (index, fixture) in fixtures.iter().enumerate() {
            assert!(
                schema_accepts::<MergeVerificationData>(fixture),
                "verification leaf {index} was rejected: {fixture}",
            );
        }

        let mut invalid = fixtures[8].clone();
        invalid["checkpointId"] = json!(ID_6);
        assert!(!schema_accepts::<MergeVerificationData>(&invalid));
        let mut invalid = fixtures[6].clone();
        invalid["mergeReceiptId"] = json!(ID_5);
        assert!(!schema_accepts::<MergeVerificationData>(&invalid));
        let mut local_with_session = fixtures[0].clone();
        local_with_session["sessionId"] = json!(ID_7);
        assert!(!schema_accepts::<MergeVerificationData>(
            &local_with_session
        ));
    }

    #[test]
    fn merge_result_apply_is_physical_and_binds_resolved_projection() {
        let session = resolved_session();
        assert!(TaskSourcePublicationAuthority::from_publisher_adapter(
            id(ID_3),
            digest('e'),
            digest('f'),
        )
        .is_err());
        let publication = TaskSourcePublicationAuthority::from_publisher_adapter(
            id(ID_3),
            digest('e'),
            digest('e'),
        )
        .unwrap();
        let decision_projection =
            ResolvedApplyDecisionProjectionAuthority::for_no_conflict_session(&session).unwrap();
        let apply = MergeApplyData::task_from_authorities(
            &session,
            decision_projection,
            publication,
            id(ID_2),
            digest('a'),
            digest('b'),
            digest('d'),
        )
        .unwrap();
        let value = serde_json::to_value(&apply).unwrap();
        assert_forbidden_fields_rejected_recursively::<MergeApplyData>(value.clone());

        for (field, injected) in [
            (
                "repositoryHistoryCursor",
                json!({
                    "repositoryVersion": "10",
                    "historyPrefixDigest": digest('1').as_str(),
                }),
            ),
            ("rollbackCheckpointId", json!(ID_4)),
            ("integrationSetDigest", json!(digest('2').as_str())),
            ("lockSetDigest", json!(digest('3').as_str())),
            ("supportGateDigest", json!(digest('4').as_str())),
        ] {
            let mut invalid = value.clone();
            invalid[field] = injected;
            assert!(
                !schema_accepts::<MergeApplyData>(&invalid),
                "accepted {field}"
            );
        }

        assert!(MergeApplyData::task_test_only(
            &session,
            TaskMergeApplyFixtureInput {
                merge_receipt_id: id(ID_2),
                before_anchor: digest('a'),
                after_anchor: digest('b'),
                result_fingerprint: digest('c'),
                support_audit_digest: digest('d'),
                applied_decision_ids: vec![id(ID_7)],
                source_publication_id: id(ID_3),
                source_fingerprint: digest('e'),
                task_infobase_fingerprint: digest('f'),
            },
        )
        .is_err());
        assert!(AppliedDecisionIds::new(vec![id(ID_7), id(ID_7)]).is_err());

        let main = main_integration_session();
        assert!(MergeApplyData::task_test_only(
            &main,
            TaskMergeApplyFixtureInput {
                merge_receipt_id: id(ID_2),
                before_anchor: digest('a'),
                after_anchor: digest('b'),
                result_fingerprint: digest('c'),
                support_audit_digest: digest('d'),
                applied_decision_ids: vec![],
                source_publication_id: id(ID_3),
                source_fingerprint: digest('e'),
                task_infobase_fingerprint: digest('f'),
            },
        )
        .is_err());

        let original = MergeApplyData::original_test_only(
            &main,
            OriginalMergeApplyFixtureInput {
                merge_receipt_id: id(ID_2),
                before_anchor: digest('a'),
                after_anchor: digest('b'),
                result_fingerprint: digest('c'),
                repository_history_cursor: RepositoryHistoryCursor::new(
                    RepositoryVersion::parse("10").unwrap(),
                    digest('1'),
                ),
                support_audit_digest: digest('d'),
                applied_decision_ids: vec![],
                rollback_checkpoint_id: id(ID_3),
                integration_set_digest: digest('e'),
                lock_set_digest: digest('f'),
            },
        )
        .unwrap();
        let original_value = serde_json::to_value(original).unwrap();
        assert!(schema_accepts::<MergeApplyData>(&original_value));
        for field in [
            "sourcePublicationId",
            "sourceFingerprint",
            "taskInfobaseFingerprint",
        ] {
            let mut invalid = original_value.clone();
            invalid[field] = json!(ID_7);
            assert!(
                !schema_accepts::<MergeApplyData>(&invalid),
                "accepted {field}"
            );
        }
        assert!(MergeApplyData::original_test_only(
            &session,
            OriginalMergeApplyFixtureInput {
                merge_receipt_id: id(ID_2),
                before_anchor: digest('a'),
                after_anchor: digest('b'),
                result_fingerprint: digest('c'),
                repository_history_cursor: RepositoryHistoryCursor::new(
                    RepositoryVersion::parse("10").unwrap(),
                    digest('1'),
                ),
                support_audit_digest: digest('d'),
                applied_decision_ids: vec![],
                rollback_checkpoint_id: id(ID_3),
                integration_set_digest: digest('e'),
                lock_set_digest: digest('f'),
            },
        )
        .is_err());
    }

    #[test]
    fn merge_result_repository_plan_projection_consumes_exact_main_session() {
        let preflight = ready_preflight_authority_fixture_test_only();
        let main = consumer_fixture_main_session(&preflight);
        let session = MergeSessionData::MainIntegration(main.clone());
        let decision_projection =
            ResolvedApplyDecisionProjectionAuthority::for_no_conflict_session(&session).unwrap();
        let preparation =
            ValidatedMainIntegrationPreparationAuthority::new(preflight, main.clone()).unwrap();
        let projection = ValidatedRepositoryPlanSessionProjection::from_main_preparation(
            preparation,
            decision_projection,
        )
        .unwrap();
        assert_eq!(projection.merge_session_id(), &main.session_id);
        assert_eq!(
            projection.support_gate_history_evidence_digest(),
            &main.support_gate_history_evidence_digest
        );
        assert_eq!(projection.comparison_id(), &main.comparison_id);
        assert_eq!(projection.applied_decision_ids(), &[]);

        let task_session = resolved_session();
        let task_projection =
            ResolvedApplyDecisionProjectionAuthority::for_no_conflict_session(&task_session)
                .unwrap();
        let preflight = ready_preflight_authority_fixture_test_only();
        let main = main_integration_session_for_preflight(&preflight);
        let preparation =
            ValidatedMainIntegrationPreparationAuthority::new(preflight, main).unwrap();
        assert!(
            ValidatedRepositoryPlanSessionProjection::from_main_preparation(
                preparation,
                task_projection,
            )
            .is_err()
        );
    }

    #[test]
    fn merge_result_main_sandbox_verification_preserves_lock_planning_lineage() {
        let preflight = ready_preflight_authority_fixture_test_only();
        let main = main_integration_session_for_preflight(&preflight);
        let session = MergeSessionData::MainIntegration(main.clone());
        let preparation =
            ValidatedMainIntegrationPreparationAuthority::new(preflight, main.clone()).unwrap();
        let planning = ValidatedRepositoryPlanSessionProjection::from_main_preparation(
            preparation,
            ResolvedApplyDecisionProjectionAuthority::for_no_conflict_session(&session).unwrap(),
        )
        .unwrap();
        let observation = MainSandboxVerificationObservationAuthority::valid_from_verifier_adapter(
            &planning,
            main_sandbox_verification_input(
                &planning,
                id(ID_6),
                digest('1'),
                vec![id(ID_7)],
                digest('2'),
                vec![],
            ),
        )
        .unwrap();
        let verified =
            MergeVerificationData::main_sandbox_valid_from_authorities(planning, observation)
                .unwrap();
        assert_eq!(verified.comparison_id(), &main.comparison_id);
        let MergeVerificationData::MainSandboxValid(data) = verified.data() else {
            unreachable!()
        };
        assert_eq!(data.session_id, main.session_id);
        assert_eq!(data.support_gate_digest, main.support_gate_digest);
        assert_eq!(
            data.support_gate_history_evidence.evidence_digest(),
            &main.support_gate_history_evidence_digest
        );
        assert!(MergeVerificationData::MainSandboxValid(data)
            .validates_digest()
            .unwrap());
    }

    #[test]
    fn merge_result_original_apply_consumes_exact_lock_and_rollback_lineage() {
        let preflight = ready_preflight_authority_fixture_test_only();
        let session =
            MergeSessionData::MainIntegration(main_integration_session_for_preflight(&preflight));
        let MergeSessionData::MainIntegration(main) = &session else {
            unreachable!()
        };
        let lock_projection = |merge_session_id: UnicaId| {
            original_merge_production_lock_projection_fixture_test_only(
                merge_session_id,
                main.resolved_session_digest.clone(),
                ready_preflight_authority_fixture_test_only(),
                main.settings_digest.clone(),
            )
        };

        let wrong_rollback = verified_original_rollback_checkpoint_fixture_test_only(
            lock_projection(id(ID_2)),
            id(ID_4),
        );
        assert!(MergeApplyData::original_from_authorities(
            &session,
            ResolvedApplyDecisionProjectionAuthority::for_no_conflict_session(&session).unwrap(),
            wrong_rollback,
            OriginalMergeEffectObservationAuthority::fixture_test_only(
                digest('4'),
                digest('5'),
                digest('6'),
                digest('7'),
            ),
            id(ID_5),
        )
        .is_err());

        let lock_projection = lock_projection(main.session_id.clone());
        let expected_integration_set_digest = lock_projection.integration_set_digest().clone();
        let expected_lock_set_digest = lock_projection.lock_set_digest().clone();
        let rollback =
            verified_original_rollback_checkpoint_fixture_test_only(lock_projection, id(ID_4));
        let receipt = MergeApplyData::original_from_authorities(
            &session,
            ResolvedApplyDecisionProjectionAuthority::for_no_conflict_session(&session).unwrap(),
            rollback,
            OriginalMergeEffectObservationAuthority::fixture_test_only(
                digest('4'),
                digest('5'),
                digest('6'),
                digest('7'),
            ),
            id(ID_5),
        )
        .unwrap();
        let MergeApplyData::Original(apply) = receipt.data() else {
            unreachable!()
        };
        assert_eq!(
            apply.repository_history_cursor,
            *preflight.history_evidence().classified_through_cursor()
        );
        assert_eq!(
            apply.integration_set_digest,
            expected_integration_set_digest
        );
        assert_eq!(apply.lock_set_digest, expected_lock_set_digest);
        assert_eq!(
            apply.support_gate_history_evidence_digest,
            *preflight.history_evidence_digest()
        );
    }

    #[test]
    fn merge_result_main_verification_consumes_original_receipt_lineage() {
        let lineage =
            validated_consumed_original_merge_context_fixture_test_only(id(ID_5), digest('6'));
        let receipt = lineage.receipt();
        let expected_integration_set_id = receipt.integration_set_id().clone();
        let expected_integration_set_digest = receipt.integration_set_digest().clone();
        let expected_session_id = receipt.session_id().clone();
        let expected_history_evidence_digest = receipt
            .support_gate_history_evidence()
            .evidence_digest()
            .clone();
        let observation =
            MainIntegrationVerificationObservationAuthority::valid_from_verifier_adapter(
                main_integration_verification_input(
                    lineage,
                    id(ID_3),
                    digest('8'),
                    vec![id(ID_2)],
                    digest('9'),
                    vec![],
                ),
            );
        let verified =
            MergeVerificationData::main_integration_valid_from_authorities(observation).unwrap();
        assert_eq!(verified.integration_set_id(), &expected_integration_set_id);
        let MergeVerificationData::MainIntegrationValid(data) = verified.data() else {
            unreachable!()
        };
        assert_eq!(data.session_id, expected_session_id);
        assert_eq!(data.merge_receipt_id, id(ID_5));
        assert_eq!(data.integration_set_digest, expected_integration_set_digest);
        assert_eq!(
            data.support_gate_history_evidence.evidence_digest(),
            &expected_history_evidence_digest
        );
        assert!(MergeVerificationData::MainIntegrationValid(data)
            .validates_digest()
            .unwrap());
    }

    #[test]
    fn merge_result_all_physical_leaf_schemas_are_closed() {
        assert_closed::<ComparisonData>();
        assert_closed::<MergeSessionData>();
        assert_closed::<MainIntegrationPreparationData>();
        assert_closed::<ConflictListData>();
        assert_closed::<ConflictDecisionData>();
        assert_closed::<AdaptedDeltaDecisionData>();
        assert_closed::<MergeApplyData>();
        assert_closed::<MergeVerificationData>();

        let preparation_schema = schema::<MainIntegrationPreparationData>();
        assert_eq!(
            preparation_schema["$defs"]["ReadySupportPreflightData"]["properties"]["outcome"]
                ["const"],
            json!("ready"),
        );

        // Exercise the scalar schema used by original-apply history cursors so
        // a future field-type regression cannot silently weaken it to a string.
        let _: RepositoryVersion = RepositoryVersion::parse("10").unwrap();
    }
}

#[cfg(test)]
mod gate_b2_atomic_tests {
    use super::*;
    use crate::domain::branched_development::contracts::repository::RepositoryAnchorObservationAuthority;
    use crate::domain::branched_development::contracts::results::repository::original_merge_production_lock_projection_with_revision_fixture_test_only;
    use crate::domain::branched_development::contracts::scalars::NormalizedUtcInstant;
    use crate::domain::branched_development::CapabilityRowId;
    use std::cell::RefCell;
    use std::rc::Rc;

    const RECEIPT: &str = "b2000000-0000-4000-8000-000000000001";

    fn digest(character: char) -> Sha256Digest {
        Sha256Digest::parse(&character.to_string().repeat(64)).unwrap()
    }

    fn id(value: &str) -> UnicaId {
        UnicaId::parse(value).unwrap()
    }

    struct CompletionThenReplayConfiguredValidationPort {
        snapshot_input: Option<ConfiguredValidationReceiptBatchSnapshotInput>,
        stored_snapshot: Option<ConfiguredValidationReceiptBatchSnapshot>,
    }

    impl ConfiguredValidationCheckExecutionPort for CompletionThenReplayConfiguredValidationPort {
        fn execute(
            &mut self,
            request: ConfiguredValidationCheckExecutionRequest<'_>,
        ) -> Result<ConfiguredValidationReceiptBatchSnapshot, MergeResultContractError> {
            if let Some(input) = self.snapshot_input.take() {
                self.stored_snapshot = Some(request.complete(input));
                return Err(MergeResultContractError(
                    "configured validation response was lost after completion",
                ));
            }
            self.stored_snapshot.take().ok_or(MergeResultContractError(
                "configured validation stored snapshot replayed",
            ))
        }
    }

    fn original_context() -> (
        MergeSessionData,
        ResolvedApplyDecisionProjectionAuthority,
        ValidatedOriginalMergeLockProjection,
    ) {
        original_context_with_revision(digest('b'))
    }

    fn original_context_with_revision(
        current_state_revision: Sha256Digest,
    ) -> (
        MergeSessionData,
        ResolvedApplyDecisionProjectionAuthority,
        ValidatedOriginalMergeLockProjection,
    ) {
        let preflight = ready_preflight_authority_fixture_test_only();
        let main = consumer_fixture_main_session(&preflight);
        let session = MergeSessionData::MainIntegration(main.clone());
        let decisions =
            ResolvedApplyDecisionProjectionAuthority::for_no_conflict_session(&session).unwrap();
        let locks = original_merge_production_lock_projection_with_revision_fixture_test_only(
            main.session_id,
            main.resolved_session_digest,
            preflight,
            main.settings_digest,
            current_state_revision,
        );
        (session, decisions, locks)
    }

    enum TestRollbackCheckpointObservation {
        Exact,
        Fingerprint(Sha256Digest),
        RootBeforeAnchor(RepositoryAnchor),
        CurrentStateRevision(Sha256Digest),
        PortError,
    }

    struct TestRollbackCheckpointPort {
        called: Rc<RefCell<usize>>,
        checkpoint_id: UnicaId,
        observation: TestRollbackCheckpointObservation,
    }

    impl OriginalMergeRollbackCheckpointPort for TestRollbackCheckpointPort {
        fn create_and_verify_original_rollback_checkpoint(
            &mut self,
            request: OriginalMergeRollbackCheckpointRequest<'_>,
        ) -> Result<OriginalMergeRollbackCheckpointCompletion, MergeResultContractError> {
            *self.called.borrow_mut() += 1;
            let _ = (
                request.merge_session_id(),
                request.resolved_session_digest(),
                request.plan_id(),
                request.plan_digest(),
                request.lock_set_id(),
                request.lock_set_digest(),
                request.support_gate_id(),
                request.support_gate_digest(),
                request.support_gate_history_evidence(),
                request.root_reread_capability_id(),
            );
            if matches!(
                &self.observation,
                TestRollbackCheckpointObservation::PortError
            ) {
                return Err(MergeResultContractError(
                    "rollback checkpoint adapter failed",
                ));
            }
            let checkpoint_fingerprint = match &self.observation {
                TestRollbackCheckpointObservation::Fingerprint(value) => value.clone(),
                _ => request.original_fingerprint().clone(),
            };
            let root_before_anchor = match &self.observation {
                TestRollbackCheckpointObservation::RootBeforeAnchor(value) => value.clone(),
                _ => request.root_before_anchor().clone(),
            };
            let observed_current_state_revision = match &self.observation {
                TestRollbackCheckpointObservation::CurrentStateRevision(value) => value.clone(),
                _ => request.current_state_revision().clone(),
            };
            Ok(request.complete(
                self.checkpoint_id.clone(),
                checkpoint_fingerprint,
                root_before_anchor,
                observed_current_state_revision,
                CapabilityRowId::parse("repository.rollback-checkpoint.test").unwrap(),
            ))
        }
    }

    struct CompletionThenReplayRollbackCheckpointPort {
        checkpoint_id: UnicaId,
        stored_completion: Option<OriginalMergeRollbackCheckpointCompletion>,
    }

    impl OriginalMergeRollbackCheckpointPort for CompletionThenReplayRollbackCheckpointPort {
        fn create_and_verify_original_rollback_checkpoint(
            &mut self,
            request: OriginalMergeRollbackCheckpointRequest<'_>,
        ) -> Result<OriginalMergeRollbackCheckpointCompletion, MergeResultContractError> {
            if self.stored_completion.is_none() {
                let fingerprint = request.original_fingerprint().clone();
                let root = request.root_before_anchor().clone();
                let revision = request.current_state_revision().clone();
                self.stored_completion = Some(request.complete(
                    self.checkpoint_id.clone(),
                    fingerprint,
                    root,
                    revision,
                    CapabilityRowId::parse("repository.rollback-checkpoint.replayed").unwrap(),
                ));
                return Err(MergeResultContractError(
                    "rollback checkpoint response was lost after completion",
                ));
            }
            self.stored_completion
                .take()
                .ok_or(MergeResultContractError(
                    "stored rollback checkpoint completion already replayed",
                ))
        }
    }

    struct TestPreIntentLease {
        binds: bool,
        capability_id: CapabilityRowId,
    }

    impl OriginalMergePreIntentLease for TestPreIntentLease {
        fn binds(&self, _request: &OriginalMergePreIntentRequest<'_>) -> bool {
            self.binds
        }

        fn preintent_capability_id(&self) -> &CapabilityRowId {
            &self.capability_id
        }
    }

    struct TestPreIntentPort {
        called: Rc<RefCell<usize>>,
        binds: bool,
        capability_id: &'static str,
    }

    impl OriginalMergePreIntentPort for TestPreIntentPort {
        fn reread_immediately_before_original_merge(
            &mut self,
            request: OriginalMergePreIntentRequest<'_>,
        ) -> Result<OriginalMergePreIntentCompletion, MergeResultContractError> {
            *self.called.borrow_mut() += 1;
            assert_eq!(request.support_gate_id(), request.plan_support_gate_id());
            assert_eq!(
                request.current_state_revision(),
                request.lock_projection_current_state_revision()
            );
            assert_eq!(
                request.journaled_lock_receipts().len(),
                request.planned_lock_count()
            );
            let _ = request.rollback_checkpoint_capability_id();
            Ok(request.complete(Box::new(TestPreIntentLease {
                binds: self.binds,
                capability_id: CapabilityRowId::parse(self.capability_id).unwrap(),
            })))
        }
    }

    struct RereadThenReplayPreIntentPort {
        stored_completion: Option<OriginalMergePreIntentCompletion>,
    }

    impl OriginalMergePreIntentPort for RereadThenReplayPreIntentPort {
        fn reread_immediately_before_original_merge(
            &mut self,
            request: OriginalMergePreIntentRequest<'_>,
        ) -> Result<OriginalMergePreIntentCompletion, MergeResultContractError> {
            if self.stored_completion.is_none() {
                self.stored_completion = Some(
                    request.complete(Box::new(TestPreIntentLease {
                        binds: true,
                        capability_id: CapabilityRowId::parse(
                            "repository.original.preintent.replayed-lease",
                        )
                        .unwrap(),
                    })),
                );
                return Err(MergeResultContractError(
                    "pre-intent reread response was lost after lease creation",
                ));
            }
            self.stored_completion
                .take()
                .ok_or(MergeResultContractError(
                    "stored pre-intent lease already replayed",
                ))
        }
    }

    struct TestOriginalMergePort {
        called: Rc<RefCell<usize>>,
        before_anchor: Sha256Digest,
        after_anchor: Sha256Digest,
        result_fingerprint: Sha256Digest,
        support_audit_digest: Sha256Digest,
    }

    impl OriginalMergeExecutionPort for TestOriginalMergePort {
        fn execute_original_merge(
            &mut self,
            request: OriginalMergeExecutionRequest<'_>,
        ) -> Result<OriginalMergeExecutionPortOutcome, MergeResultContractError> {
            *self.called.borrow_mut() += 1;
            let _receipt_id = request.merge_receipt_id();
            assert_eq!(request.plan_id(), request.preintent_plan_id());
            Ok(OriginalMergeExecutionPortOutcome::Observed(
                request.complete(
                    self.before_anchor.clone(),
                    self.after_anchor.clone(),
                    self.result_fingerprint.clone(),
                    self.support_audit_digest.clone(),
                ),
            ))
        }
    }

    struct CompletionThenReplayOriginalMergePort {
        raw_observation: Option<(Sha256Digest, Sha256Digest, Sha256Digest, Sha256Digest)>,
        stored_observation: Option<OriginalMergeEffectObservationAuthority>,
    }

    impl OriginalMergeExecutionPort for CompletionThenReplayOriginalMergePort {
        fn execute_original_merge(
            &mut self,
            request: OriginalMergeExecutionRequest<'_>,
        ) -> Result<OriginalMergeExecutionPortOutcome, MergeResultContractError> {
            if let Some((before_anchor, after_anchor, result_fingerprint, support_audit_digest)) =
                self.raw_observation.take()
            {
                self.stored_observation = Some(request.complete(
                    before_anchor,
                    after_anchor,
                    result_fingerprint,
                    support_audit_digest,
                ));
                return Err(MergeResultContractError(
                    "original merge response was lost after effect completion",
                ));
            }
            Ok(OriginalMergeExecutionPortOutcome::Observed(
                self.stored_observation
                    .take()
                    .ok_or(MergeResultContractError(
                        "stored original merge observation already replayed",
                    ))?,
            ))
        }
    }

    struct FailingOriginalMergePort;

    impl OriginalMergeExecutionPort for FailingOriginalMergePort {
        fn execute_original_merge(
            &mut self,
            _request: OriginalMergeExecutionRequest<'_>,
        ) -> Result<OriginalMergeExecutionPortOutcome, MergeResultContractError> {
            Err(MergeResultContractError("original merge adapter failed"))
        }
    }

    struct UnknownOriginalMergePort;

    impl OriginalMergeExecutionPort for UnknownOriginalMergePort {
        fn execute_original_merge(
            &mut self,
            _request: OriginalMergeExecutionRequest<'_>,
        ) -> Result<OriginalMergeExecutionPortOutcome, MergeResultContractError> {
            Ok(OriginalMergeExecutionPortOutcome::Unknown(
                OriginalMergeExecutionUnknownObservationAuthority::from_repository_adapter(),
            ))
        }
    }

    fn original_preintent(rollback_checkpoint_id: UnicaId) -> OriginalMergePreIntentAuthority {
        original_preintent_with_capability(rollback_checkpoint_id, "repository.original.preintent")
    }

    fn original_preintent_with_capability(
        rollback_checkpoint_id: UnicaId,
        capability_id: &'static str,
    ) -> OriginalMergePreIntentAuthority {
        let (session, decisions, locks) = original_context();
        let rollback =
            verified_original_rollback_checkpoint_fixture_test_only(locks, rollback_checkpoint_id);
        OriginalMergePreIntentAuthority::recheck(
            &session,
            decisions,
            rollback,
            &mut TestPreIntentPort {
                called: Rc::new(RefCell::new(0)),
                binds: true,
                capability_id,
            },
        )
        .unwrap()
    }

    fn pending_original_merge(
        receipt_id: UnicaId,
        result_fingerprint: Sha256Digest,
    ) -> PendingOriginalMergeReceiptAuthority {
        pending_original_merge_with_lineage(
            receipt_id,
            result_fingerprint,
            id("b2000000-0000-4000-8000-000000000002"),
            "repository.original.preintent",
            digest('1'),
            digest('2'),
            digest('4'),
        )
    }

    fn pending_original_merge_with_lineage(
        receipt_id: UnicaId,
        result_fingerprint: Sha256Digest,
        rollback_checkpoint_id: UnicaId,
        preintent_capability_id: &'static str,
        before_anchor: Sha256Digest,
        after_anchor: Sha256Digest,
        support_audit_digest: Sha256Digest,
    ) -> PendingOriginalMergeReceiptAuthority {
        let preintent =
            original_preintent_with_capability(rollback_checkpoint_id, preintent_capability_id);
        PendingOriginalMergeReceiptAuthority::execute(
            preintent,
            receipt_id,
            &mut TestOriginalMergePort {
                called: Rc::new(RefCell::new(0)),
                before_anchor,
                after_anchor,
                result_fingerprint,
                support_audit_digest,
            },
        )
        .unwrap()
    }

    #[test]
    fn original_merge_execution_error_retains_full_preintent_authority() {
        let rollback_id = id("b2000000-0000-4000-8000-000000000002");
        let preintent = original_preintent(rollback_id.clone());
        let expected_plan_id = preintent.lock_projection().plan_id().clone();
        let expected_lock_set_id = preintent.lock_projection().lock_set_id().clone();
        let expected_root_receipt = preintent
            .lock_projection()
            .root_lock_receipt()
            .unwrap()
            .clone();

        let blocked = PendingOriginalMergeReceiptAuthority::execute(
            preintent,
            id(RECEIPT),
            &mut FailingOriginalMergePort,
        )
        .unwrap_err();

        assert!(blocked.is_port_error());
        assert_eq!(blocked.merge_receipt_id(), &id(RECEIPT));
        assert_eq!(blocked.plan_id(), &expected_plan_id);
        assert_eq!(blocked.lock_set_id(), &expected_lock_set_id);
        assert_eq!(blocked.rollback_checkpoint_id(), &rollback_id);
        assert_eq!(blocked.root_lock_receipt(), &expected_root_receipt);
    }

    #[test]
    fn original_merge_unknown_outcome_retains_full_preintent_authority() {
        let rollback_id = id("b2000000-0000-4000-8000-000000000002");
        let preintent = original_preintent(rollback_id.clone());
        let expected_receipts = preintent
            .lock_projection()
            .journaled_lock_receipts()
            .unwrap()
            .to_vec();

        let blocked = PendingOriginalMergeReceiptAuthority::execute(
            preintent,
            id(RECEIPT),
            &mut UnknownOriginalMergePort,
        )
        .unwrap_err();

        assert!(blocked.is_outcome_unknown());
        assert_eq!(blocked.rollback_checkpoint_id(), &rollback_id);
        assert_eq!(blocked.journaled_lock_receipts(), expected_receipts);
    }

    #[test]
    fn original_merge_effect_completion_replay_rejects_cross_receipt_and_retains_current_preintent()
    {
        let preintent_a = original_preintent_with_capability(
            id("b2000000-0000-4000-8000-000000000047"),
            "repository.original.preintent.effect-cross-a",
        );
        let preintent_b = original_preintent_with_capability(
            id("b2000000-0000-4000-8000-000000000048"),
            "repository.original.preintent.effect-cross-b",
        );
        let expected_b_plan = preintent_b.lock_projection().plan_id().clone();
        let expected_b_locks = preintent_b
            .lock_projection()
            .journaled_lock_receipts()
            .unwrap()
            .to_vec();
        let receipt_a = id("b2000000-0000-4000-8000-000000000049");
        let receipt_b = id("b2000000-0000-4000-8000-000000000050");
        let mut port = CompletionThenReplayOriginalMergePort {
            raw_observation: Some((digest('1'), digest('2'), digest('3'), digest('4'))),
            stored_observation: None,
        };

        let first_blocked =
            PendingOriginalMergeReceiptAuthority::execute(preintent_a, receipt_a, &mut port)
                .unwrap_err();
        assert!(first_blocked.is_port_error());

        let blocked = PendingOriginalMergeReceiptAuthority::execute(
            preintent_b,
            receipt_b.clone(),
            &mut port,
        )
        .unwrap_err();
        assert!(blocked.is_completion_attempt_mismatch());
        assert_eq!(blocked.merge_receipt_id(), &receipt_b);
        assert_eq!(blocked.plan_id(), &expected_b_plan);
        assert_eq!(blocked.journaled_lock_receipts(), expected_b_locks);
        assert_eq!(
            blocked.preintent.preintent_capability_id,
            CapabilityRowId::parse("repository.original.preintent.effect-cross-b").unwrap()
        );
    }

    #[test]
    fn original_merge_effect_completion_replay_rejects_hidden_lineage_and_retains_current_preintent(
    ) {
        let rollback = id("b2000000-0000-4000-8000-000000000051");
        let preintent_a = original_preintent_with_capability(
            rollback.clone(),
            "repository.original.preintent.effect-hidden-a",
        );
        let preintent_b = original_preintent_with_capability(
            rollback.clone(),
            "repository.original.preintent.effect-hidden-b",
        );
        let expected_b_root = preintent_b
            .lock_projection()
            .root_lock_receipt()
            .unwrap()
            .clone();
        let receipt = id("b2000000-0000-4000-8000-000000000052");
        let mut port = CompletionThenReplayOriginalMergePort {
            raw_observation: Some((digest('1'), digest('2'), digest('3'), digest('4'))),
            stored_observation: None,
        };

        PendingOriginalMergeReceiptAuthority::execute(preintent_a, receipt.clone(), &mut port)
            .unwrap_err();
        let blocked =
            PendingOriginalMergeReceiptAuthority::execute(preintent_b, receipt.clone(), &mut port)
                .unwrap_err();
        assert!(blocked.is_completion_attempt_mismatch());
        assert_eq!(blocked.merge_receipt_id(), &receipt);
        assert_eq!(blocked.rollback_checkpoint_id(), &rollback);
        assert_eq!(blocked.root_lock_receipt(), &expected_b_root);
        assert_eq!(
            blocked.preintent.preintent_capability_id,
            CapabilityRowId::parse("repository.original.preintent.effect-hidden-b").unwrap()
        );
    }

    #[test]
    fn original_merge_effect_completion_replay_rejects_exact_preintent_retry_under_new_invocation()
    {
        let rollback = id("b2000000-0000-4000-8000-000000000053");
        let preintent = original_preintent_with_capability(
            rollback.clone(),
            "repository.original.preintent.effect-retry",
        );
        let receipt = id("b2000000-0000-4000-8000-000000000054");
        let mut port = CompletionThenReplayOriginalMergePort {
            raw_observation: Some((digest('1'), digest('2'), digest('3'), digest('4'))),
            stored_observation: None,
        };

        let first_blocked =
            PendingOriginalMergeReceiptAuthority::execute(preintent, receipt.clone(), &mut port)
                .unwrap_err();
        let OriginalMergeExecutionBlockedAuthority {
            preintent,
            merge_receipt_id,
            ..
        } = *first_blocked;
        assert_eq!(merge_receipt_id, receipt);

        let blocked = PendingOriginalMergeReceiptAuthority::execute(
            preintent,
            merge_receipt_id.clone(),
            &mut port,
        )
        .unwrap_err();
        assert!(blocked.is_completion_attempt_mismatch());
        assert_eq!(blocked.merge_receipt_id(), &merge_receipt_id);
        assert_eq!(blocked.rollback_checkpoint_id(), &rollback);
        assert_eq!(
            blocked.preintent.preintent_capability_id,
            CapabilityRowId::parse("repository.original.preintent.effect-retry").unwrap()
        );
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct PersistedAtomicState {
        merge_receipt_id: UnicaId,
        result_fingerprint: Sha256Digest,
        plan_id: UnicaId,
        receipt_persisted: bool,
        gate_consumed: bool,
    }

    struct TestCasLease {
        state: Rc<RefCell<Option<PersistedAtomicState>>>,
        expected_receipt_id: UnicaId,
        expected_fingerprint: Sha256Digest,
        expected_plan_id: UnicaId,
    }

    impl SupportGateOriginalMergeCasLease for TestCasLease {
        fn binds(&self, binding: &SupportGateOriginalMergeCasBinding) -> bool {
            binding.merge_receipt_id() == &self.expected_receipt_id
                && binding.result_fingerprint() == &self.expected_fingerprint
                && binding.plan_id() == &self.expected_plan_id
        }

        fn commit_receipt_and_consume_gate(
            self: Box<Self>,
            pending: &PendingOriginalMergeReceiptAuthority,
        ) -> Result<(), MergeResultContractError> {
            if self.state.borrow().is_some() {
                return Err(MergeResultContractError(
                    "support gate was already consumed",
                ));
            }
            *self.state.borrow_mut() = Some(PersistedAtomicState {
                merge_receipt_id: pending.merge_receipt_id().clone(),
                result_fingerprint: pending.result_fingerprint().clone(),
                plan_id: pending.plan_id().clone(),
                receipt_persisted: true,
                gate_consumed: true,
            });
            Ok(())
        }
    }

    struct TestCasResolver {
        state: Rc<RefCell<Option<PersistedAtomicState>>>,
        expected_receipt_id: UnicaId,
        expected_fingerprint: Sha256Digest,
        expected_plan_id: UnicaId,
    }

    impl SupportGateOriginalMergeCasResolver for TestCasResolver {
        fn resolve_original_merge_cas(
            &mut self,
            request: SupportGateOriginalMergeCasRequest<'_>,
        ) -> Result<SupportGateOriginalMergeCasResolution, MergeResultContractError> {
            if self.state.borrow().is_some() {
                return Err(MergeResultContractError("support gate is not current"));
            }
            Ok(request.complete(Box::new(TestCasLease {
                state: Rc::clone(&self.state),
                expected_receipt_id: self.expected_receipt_id.clone(),
                expected_fingerprint: self.expected_fingerprint.clone(),
                expected_plan_id: self.expected_plan_id.clone(),
            })))
        }
    }

    struct ReceiptBoundCasLease {
        expected_receipt_id: UnicaId,
        expected_fingerprint: Sha256Digest,
        expected_plan_id: UnicaId,
        expected_root_receipt: JournaledRepositoryLock,
        expected_receipts: Vec<JournaledRepositoryLock>,
    }

    impl SupportGateOriginalMergeCasLease for ReceiptBoundCasLease {
        fn binds(&self, binding: &SupportGateOriginalMergeCasBinding) -> bool {
            binding.merge_receipt_id() == &self.expected_receipt_id
                && binding.result_fingerprint() == &self.expected_fingerprint
                && binding.plan_id() == &self.expected_plan_id
                && binding.root_lock_receipt() == &self.expected_root_receipt
                && binding.journaled_lock_receipts() == self.expected_receipts
        }

        fn commit_receipt_and_consume_gate(
            self: Box<Self>,
            _pending: &PendingOriginalMergeReceiptAuthority,
        ) -> Result<(), MergeResultContractError> {
            Ok(())
        }
    }

    struct ReceiptBoundCasResolver {
        lease: Option<ReceiptBoundCasLease>,
    }

    impl SupportGateOriginalMergeCasResolver for ReceiptBoundCasResolver {
        fn resolve_original_merge_cas(
            &mut self,
            request: SupportGateOriginalMergeCasRequest<'_>,
        ) -> Result<SupportGateOriginalMergeCasResolution, MergeResultContractError> {
            Ok(request.complete(Box::new(self.lease.take().unwrap())))
        }
    }

    struct ResolveThenReplayCasResolver {
        stored_resolution: Option<SupportGateOriginalMergeCasResolution>,
    }

    impl SupportGateOriginalMergeCasResolver for ResolveThenReplayCasResolver {
        fn resolve_original_merge_cas(
            &mut self,
            request: SupportGateOriginalMergeCasRequest<'_>,
        ) -> Result<SupportGateOriginalMergeCasResolution, MergeResultContractError> {
            if self.stored_resolution.is_none() {
                let binding = request.binding();
                let lease = ReceiptBoundCasLease {
                    expected_receipt_id: binding.merge_receipt_id().clone(),
                    expected_fingerprint: binding.result_fingerprint().clone(),
                    expected_plan_id: binding.plan_id().clone(),
                    expected_root_receipt: binding.root_lock_receipt().clone(),
                    expected_receipts: binding.journaled_lock_receipts().to_vec(),
                };
                self.stored_resolution = Some(request.complete(Box::new(lease)));
                return Err(MergeResultContractError(
                    "CAS resolver response was lost after lease creation",
                ));
            }
            self.stored_resolution
                .take()
                .ok_or(MergeResultContractError(
                    "stored CAS lease already replayed",
                ))
        }
    }

    fn mismatched_root_before_anchor(
        source: &ValidatedOriginalMergeLockProjection,
    ) -> RepositoryAnchor {
        let root = original_merge_root_before_anchor(source).unwrap();
        RepositoryAnchorObservationAuthority::test_only(
            digest('9'),
            root.history_cursor().clone(),
            root.configuration_identity().clone(),
            root.configuration_fingerprint().clone(),
        )
        .into_anchor()
        .unwrap()
    }

    fn assert_checkpoint_observation_mismatch(observation: TestRollbackCheckpointObservation) {
        let (_, _, source) = original_context();
        let expected_plan_id = source.plan_id().clone();
        let expected_lock_set_id = source.lock_set_id().clone();
        let calls = Rc::new(RefCell::new(0));
        let blocked = VerifiedOriginalRollbackCheckpointAuthority::create(
            source,
            &mut TestRollbackCheckpointPort {
                called: Rc::clone(&calls),
                checkpoint_id: id("b2000000-0000-4000-8000-000000000063"),
                observation,
            },
        )
        .unwrap_err();
        assert_eq!(*calls.borrow(), 1);
        assert!(matches!(
            blocked.failure(),
            OriginalMergeRollbackCheckpointFailureEvidence::ObservationMismatch(_)
        ));
        let (retained, failure) = blocked.into_recovery_parts();
        assert!(matches!(
            failure,
            OriginalMergeRollbackCheckpointFailureEvidence::ObservationMismatch(_)
        ));
        assert_eq!(retained.plan_id(), &expected_plan_id);
        assert_eq!(retained.lock_set_id(), &expected_lock_set_id);
    }

    #[test]
    fn original_rollback_checkpoint_exact_completion_owns_source_and_propagates_lineage() {
        let (session, decisions, source) = original_context();
        let expected_plan_id = source.plan_id().clone();
        let checkpoint_id = id("b2000000-0000-4000-8000-000000000062");
        let checkpoint_calls = Rc::new(RefCell::new(0));
        let rollback = VerifiedOriginalRollbackCheckpointAuthority::create(
            source,
            &mut TestRollbackCheckpointPort {
                called: Rc::clone(&checkpoint_calls),
                checkpoint_id: checkpoint_id.clone(),
                observation: TestRollbackCheckpointObservation::Exact,
            },
        )
        .unwrap();
        assert_eq!(*checkpoint_calls.borrow(), 1);
        assert_eq!(rollback.checkpoint_id(), &checkpoint_id);
        assert_eq!(rollback.lock_projection().plan_id(), &expected_plan_id);
        assert_eq!(
            rollback.checkpoint_capability_id,
            CapabilityRowId::parse("repository.rollback-checkpoint.test").unwrap()
        );

        let preintent_calls = Rc::new(RefCell::new(0));
        let preintent = OriginalMergePreIntentAuthority::recheck(
            &session,
            decisions,
            rollback,
            &mut TestPreIntentPort {
                called: Rc::clone(&preintent_calls),
                binds: true,
                capability_id: "repository.original.preintent",
            },
        )
        .unwrap();
        assert_eq!(*preintent_calls.borrow(), 1);
        assert_eq!(preintent.attempt.rollback.checkpoint_id(), &checkpoint_id);
        assert_eq!(preintent.lock_projection().plan_id(), &expected_plan_id);
    }

    #[test]
    fn original_merge_checkpoint_and_preintent_apis_require_the_combined_owning_authority() {
        type OriginalMergePreIntentRecheckFn = fn(
            &MergeSessionData,
            ResolvedApplyDecisionProjectionAuthority,
            VerifiedOriginalRollbackCheckpointAuthority,
            &mut dyn OriginalMergePreIntentPort,
        ) -> Result<
            OriginalMergePreIntentAuthority,
            Box<OriginalMergePreIntentBlockedAuthority>,
        >;

        let _checkpoint_create: fn(
            ValidatedOriginalMergeLockProjection,
            &mut dyn OriginalMergeRollbackCheckpointPort,
        ) -> Result<
            VerifiedOriginalRollbackCheckpointAuthority,
            Box<OriginalMergeRollbackCheckpointBlockedAuthority>,
        > = VerifiedOriginalRollbackCheckpointAuthority::create;
        let _preintent_recheck: OriginalMergePreIntentRecheckFn =
            OriginalMergePreIntentAuthority::recheck;
    }

    #[test]
    fn original_rollback_checkpoint_rejects_fingerprint_mismatch_and_retains_source() {
        assert_checkpoint_observation_mismatch(TestRollbackCheckpointObservation::Fingerprint(
            digest('9'),
        ));
    }

    #[test]
    fn original_rollback_checkpoint_rejects_root_anchor_mismatch_and_retains_source() {
        let (_, _, source) = original_context();
        let mismatched_root = mismatched_root_before_anchor(&source);
        let expected_plan_id = source.plan_id().clone();
        let calls = Rc::new(RefCell::new(0));
        let blocked = VerifiedOriginalRollbackCheckpointAuthority::create(
            source,
            &mut TestRollbackCheckpointPort {
                called: Rc::clone(&calls),
                checkpoint_id: id("b2000000-0000-4000-8000-000000000063"),
                observation: TestRollbackCheckpointObservation::RootBeforeAnchor(mismatched_root),
            },
        )
        .unwrap_err();
        assert_eq!(*calls.borrow(), 1);
        assert!(matches!(
            blocked.failure(),
            OriginalMergeRollbackCheckpointFailureEvidence::ObservationMismatch(_)
        ));
        let (retained, _) = blocked.into_recovery_parts();
        assert_eq!(retained.plan_id(), &expected_plan_id);
    }

    #[test]
    fn original_rollback_checkpoint_rejects_revision_mismatch_and_retains_source() {
        assert_checkpoint_observation_mismatch(
            TestRollbackCheckpointObservation::CurrentStateRevision(digest('9')),
        );
    }

    #[test]
    fn original_rollback_checkpoint_port_error_retains_exact_source_for_recovery() {
        let (_, _, source) = original_context();
        let expected_plan_id = source.plan_id().clone();
        let expected_lock_set_id = source.lock_set_id().clone();
        let calls = Rc::new(RefCell::new(0));
        let blocked = VerifiedOriginalRollbackCheckpointAuthority::create(
            source,
            &mut TestRollbackCheckpointPort {
                called: Rc::clone(&calls),
                checkpoint_id: id("b2000000-0000-4000-8000-000000000063"),
                observation: TestRollbackCheckpointObservation::PortError,
            },
        )
        .unwrap_err();
        assert_eq!(*calls.borrow(), 1);
        assert!(matches!(
            blocked.failure(),
            OriginalMergeRollbackCheckpointFailureEvidence::PortError(_)
        ));
        let (retained, failure) = blocked.into_recovery_parts();
        assert!(matches!(
            failure,
            OriginalMergeRollbackCheckpointFailureEvidence::PortError(_)
        ));
        assert_eq!(retained.plan_id(), &expected_plan_id);
        assert_eq!(retained.lock_set_id(), &expected_lock_set_id);
    }

    #[test]
    fn original_merge_requires_preintent_current_gate_authority() {
        let preflight = ready_preflight_authority_fixture_test_only();
        let main = consumer_fixture_main_session(&preflight);
        let fixture = original_merge_lock_projection_fixture_test_only(
            main.session_id.clone(),
            main.resolved_session_digest.clone(),
            main.support_gate_id.clone(),
            main.support_gate_digest.clone(),
            preflight.history_evidence().clone(),
            main.settings_digest.clone(),
        );
        let preintent_calls = Rc::new(RefCell::new(0));
        let checkpoint_calls = Rc::new(RefCell::new(0));
        let blocked = VerifiedOriginalRollbackCheckpointAuthority::create(
            fixture,
            &mut TestRollbackCheckpointPort {
                called: Rc::clone(&checkpoint_calls),
                checkpoint_id: id("b2000000-0000-4000-8000-000000000002"),
                observation: TestRollbackCheckpointObservation::Exact,
            },
        )
        .unwrap_err();
        assert!(matches!(
            blocked.failure(),
            OriginalMergeRollbackCheckpointFailureEvidence::SourceLineageMismatch
        ));
        assert_eq!(*checkpoint_calls.borrow(), 0);
        assert_eq!(*preintent_calls.borrow(), 0);

        let effect_calls = Rc::new(RefCell::new(0));
        let (session, decisions, production) = original_context();
        let rollback = verified_original_rollback_checkpoint_fixture_test_only(
            production,
            id("b2000000-0000-4000-8000-000000000002"),
        );
        let blocked = OriginalMergePreIntentAuthority::recheck(
            &session,
            decisions,
            rollback,
            &mut TestPreIntentPort {
                called: Rc::clone(&preintent_calls),
                binds: false,
                capability_id: "repository.original.preintent",
            },
        );
        match blocked {
            Ok(preintent) => {
                let _ = PendingOriginalMergeReceiptAuthority::execute(
                    preintent,
                    id(RECEIPT),
                    &mut TestOriginalMergePort {
                        called: Rc::clone(&effect_calls),
                        before_anchor: digest('1'),
                        after_anchor: digest('2'),
                        result_fingerprint: digest('3'),
                        support_audit_digest: digest('4'),
                    },
                );
                panic!("mismatched pre-intent unexpectedly authorized the effect port");
            }
            Err(_blocked) => {}
        }
        assert_eq!(*preintent_calls.borrow(), 1);
        assert_eq!(
            *effect_calls.borrow(),
            0,
            "effect port was called after failed preintent"
        );
    }

    #[test]
    fn original_merge_preintent_rejects_same_session_checkpoint_from_earlier_state() {
        let (session_a, _, locks_a) = original_context_with_revision(digest('a'));
        let earlier_checkpoint_id = id("b2000000-0000-4000-8000-000000000054");
        let mut port = CompletionThenReplayRollbackCheckpointPort {
            checkpoint_id: earlier_checkpoint_id,
            stored_completion: None,
        };
        let first =
            VerifiedOriginalRollbackCheckpointAuthority::create(locks_a, &mut port).unwrap_err();
        let (retained_a, failure) = first.into_recovery_parts();
        assert!(matches!(
            failure,
            OriginalMergeRollbackCheckpointFailureEvidence::PortError(_)
        ));
        assert_eq!(
            retained_a
                .current_gate_authority()
                .unwrap()
                .current_state_revision(),
            &digest('a')
        );

        let (session_b, _, locks_b) = original_context_with_revision(digest('b'));
        assert_eq!(session_a.session_id(), session_b.session_id());
        let blocked =
            VerifiedOriginalRollbackCheckpointAuthority::create(locks_b, &mut port).unwrap_err();
        assert!(matches!(
            blocked.failure(),
            OriginalMergeRollbackCheckpointFailureEvidence::CompletionAttemptMismatch(_)
        ));
        let (retained_b, _) = blocked.into_recovery_parts();
        assert_eq!(
            retained_b
                .current_gate_authority()
                .unwrap()
                .current_state_revision(),
            &digest('b')
        );
    }

    #[test]
    fn original_rollback_checkpoint_rejects_completion_replay_for_field_equal_projection() {
        let (_, _, source_a) = original_context();
        let (_, _, source_b) = original_context();
        assert_eq!(&source_a, &source_b);
        let expected_plan = source_b.plan_id().clone();
        let mut port = CompletionThenReplayRollbackCheckpointPort {
            checkpoint_id: id("b2000000-0000-4000-8000-000000000064"),
            stored_completion: None,
        };

        assert!(VerifiedOriginalRollbackCheckpointAuthority::create(source_a, &mut port).is_err());
        let blocked =
            VerifiedOriginalRollbackCheckpointAuthority::create(source_b, &mut port).unwrap_err();
        assert!(matches!(
            blocked.failure(),
            OriginalMergeRollbackCheckpointFailureEvidence::CompletionAttemptMismatch(_)
        ));
        let (retained_b, _) = blocked.into_recovery_parts();
        assert_eq!(retained_b.plan_id(), &expected_plan);
    }

    #[test]
    fn original_rollback_checkpoint_rejects_old_completion_on_exact_source_retry() {
        let (_, _, source) = original_context();
        let expected_lock_set = source.lock_set_id().clone();
        let mut port = CompletionThenReplayRollbackCheckpointPort {
            checkpoint_id: id("b2000000-0000-4000-8000-000000000065"),
            stored_completion: None,
        };

        let first =
            VerifiedOriginalRollbackCheckpointAuthority::create(source, &mut port).unwrap_err();
        let (source, _) = first.into_recovery_parts();
        let blocked =
            VerifiedOriginalRollbackCheckpointAuthority::create(source, &mut port).unwrap_err();
        assert!(matches!(
            blocked.failure(),
            OriginalMergeRollbackCheckpointFailureEvidence::CompletionAttemptMismatch(_)
        ));
        let (retained, _) = blocked.into_recovery_parts();
        assert_eq!(retained.lock_set_id(), &expected_lock_set);
    }

    #[test]
    fn original_merge_preintent_rejects_completed_lease_replay_for_equivalent_new_attempt() {
        let rollback_id = id("b2000000-0000-4000-8000-000000000055");
        let mut port = RereadThenReplayPreIntentPort {
            stored_completion: None,
        };
        let (session_a, decisions_a, locks_a) = original_context();
        let rollback_a =
            verified_original_rollback_checkpoint_fixture_test_only(locks_a, rollback_id.clone());
        assert!(OriginalMergePreIntentAuthority::recheck(
            &session_a,
            decisions_a,
            rollback_a,
            &mut port,
        )
        .is_err());

        let (session_b, decisions_b, locks_b) = original_context();
        let rollback_b =
            verified_original_rollback_checkpoint_fixture_test_only(locks_b, rollback_id.clone());
        let blocked = OriginalMergePreIntentAuthority::recheck(
            &session_b,
            decisions_b,
            rollback_b,
            &mut port,
        )
        .unwrap_err();
        assert_eq!(blocked.attempt.rollback.checkpoint_id(), &rollback_id);
        assert_eq!(
            blocked.plan_id(),
            blocked.attempt.rollback.lock_projection().plan_id()
        );
    }

    #[test]
    fn original_merge_atomically_persists_receipt_and_consumes_exact_gate() {
        let pending = pending_original_merge(id(RECEIPT), digest('3'));
        let state = Rc::new(RefCell::new(None));
        let mut resolver = TestCasResolver {
            state: Rc::clone(&state),
            expected_receipt_id: pending.merge_receipt_id().clone(),
            expected_fingerprint: pending.result_fingerprint().clone(),
            expected_plan_id: pending.plan_id().clone(),
        };
        let cas =
            ValidatedSupportGateOriginalMergeCasAuthority::resolve(pending, &mut resolver).unwrap();
        let receipt = cas.commit().unwrap();
        let persisted = state.borrow().clone().unwrap();
        assert!(persisted.receipt_persisted && persisted.gate_consumed);
        assert_eq!(receipt.merge_receipt_id(), &persisted.merge_receipt_id);
        assert_eq!(receipt.result_fingerprint(), &persisted.result_fingerprint);
        assert_eq!(receipt.plan_id(), &persisted.plan_id);
        assert_eq!(
            receipt.consumed_gate().authorized_result_fingerprint(),
            receipt.result_fingerprint()
        );
    }

    #[test]
    fn original_merge_cas_rejects_completed_lease_replay_for_equivalent_new_pending() {
        let pending_a = pending_original_merge_with_lineage(
            id("b2000000-0000-4000-8000-000000000056"),
            digest('3'),
            id("b2000000-0000-4000-8000-000000000057"),
            "repository.original.preintent.cas-replay",
            digest('1'),
            digest('2'),
            digest('4'),
        );
        let pending_b = pending_original_merge_with_lineage(
            id("b2000000-0000-4000-8000-000000000056"),
            digest('3'),
            id("b2000000-0000-4000-8000-000000000057"),
            "repository.original.preintent.cas-replay",
            digest('1'),
            digest('2'),
            digest('4'),
        );
        let mut resolver = ResolveThenReplayCasResolver {
            stored_resolution: None,
        };

        assert!(
            ValidatedSupportGateOriginalMergeCasAuthority::resolve(pending_a, &mut resolver,)
                .is_err()
        );
        assert!(
            ValidatedSupportGateOriginalMergeCasAuthority::resolve(pending_b, &mut resolver,)
                .is_err()
        );
    }

    #[test]
    fn original_merge_cas_authority_owns_exact_pending_and_failed_commit_retains_it() {
        let pending = pending_original_merge_with_lineage(
            id("b2000000-0000-4000-8000-000000000058"),
            digest('3'),
            id("b2000000-0000-4000-8000-000000000059"),
            "repository.original.preintent.cas-authority-replay",
            digest('1'),
            digest('2'),
            digest('4'),
        );
        let expected_receipt = pending.merge_receipt_id().clone();
        let expected_rollback = pending.data.rollback_checkpoint_id.clone();
        let state = Rc::new(RefCell::new(None));
        let mut resolver = ResponseLossCasResolver {
            state: Rc::clone(&state),
        };
        let cas =
            ValidatedSupportGateOriginalMergeCasAuthority::resolve(pending, &mut resolver).unwrap();
        let _commit_signature: fn(
            ValidatedSupportGateOriginalMergeCasAuthority,
        ) -> Result<
            ValidatedOriginalMergeReceiptAuthority,
            Box<OriginalMergeCasBlockedAuthority>,
        > = ValidatedSupportGateOriginalMergeCasAuthority::commit;

        let retained = cas.commit().unwrap_err().into_pending();
        assert_eq!(retained.merge_receipt_id(), &expected_receipt);
        assert_eq!(retained.data.rollback_checkpoint_id, expected_rollback);
        assert_eq!(
            retained.preintent.preintent_capability_id,
            CapabilityRowId::parse("repository.original.preintent.cas-authority-replay").unwrap()
        );
    }

    #[test]
    fn original_merge_cas_rejects_same_scalar_lineage_with_different_lock_receipts() {
        let pending = pending_original_merge(id(RECEIPT), digest('3'));
        let projection = pending.preintent.lock_projection();
        let actual_root = projection.root_lock_receipt().unwrap();
        let mut different_receipts = projection.journaled_lock_receipts().unwrap().to_vec();
        different_receipts[0] = JournaledRepositoryLock::new(
            actual_root.target().clone(),
            actual_root.target_display().clone(),
            actual_root.lock_set_id().clone(),
            NormalizedUtcInstant::parse("2026-07-23T01:00:01Z").unwrap(),
        );
        let different_root = different_receipts[0].clone();
        let mut resolver = ReceiptBoundCasResolver {
            lease: Some(ReceiptBoundCasLease {
                expected_receipt_id: pending.merge_receipt_id().clone(),
                expected_fingerprint: pending.result_fingerprint().clone(),
                expected_plan_id: pending.plan_id().clone(),
                expected_root_receipt: different_root,
                expected_receipts: different_receipts,
            }),
        };

        assert!(
            ValidatedSupportGateOriginalMergeCasAuthority::resolve(pending, &mut resolver,)
                .is_err()
        );
    }

    #[test]
    fn original_merge_rejects_cross_receipt_cross_fingerprint_and_cross_plan_cas_binding() {
        for mismatch in ["receipt", "fingerprint", "plan"] {
            let pending = pending_original_merge(id(RECEIPT), digest('3'));
            let state = Rc::new(RefCell::new(None));
            let mut resolver = TestCasResolver {
                state,
                expected_receipt_id: if mismatch == "receipt" {
                    id("b2000000-0000-4000-8000-000000000099")
                } else {
                    pending.merge_receipt_id().clone()
                },
                expected_fingerprint: if mismatch == "fingerprint" {
                    digest('9')
                } else {
                    pending.result_fingerprint().clone()
                },
                expected_plan_id: if mismatch == "plan" {
                    id("b2000000-0000-4000-8000-000000000098")
                } else {
                    pending.plan_id().clone()
                },
            };
            assert!(
                ValidatedSupportGateOriginalMergeCasAuthority::resolve(pending, &mut resolver,)
                    .is_err()
            );
        }
    }

    #[test]
    fn original_merge_consumed_gate_cas_is_one_shot() {
        let state = Rc::new(RefCell::new(None));
        let first = pending_original_merge(id(RECEIPT), digest('3'));
        let mut resolver = TestCasResolver {
            state: Rc::clone(&state),
            expected_receipt_id: first.merge_receipt_id().clone(),
            expected_fingerprint: first.result_fingerprint().clone(),
            expected_plan_id: first.plan_id().clone(),
        };
        ValidatedSupportGateOriginalMergeCasAuthority::resolve(first, &mut resolver)
            .unwrap()
            .commit()
            .unwrap();

        let second =
            pending_original_merge(id("b2000000-0000-4000-8000-000000000003"), digest('3'));
        assert!(
            ValidatedSupportGateOriginalMergeCasAuthority::resolve(second, &mut resolver,).is_err()
        );
    }

    macro_rules! assert_not_clone {
        ($type:ty) => {
            const _: fn() = || {
                trait AmbiguousIfClone<Marker> {
                    fn assert_not_clone() {}
                }
                struct ImplementsClone;
                impl<T: ?Sized> AmbiguousIfClone<()> for T {}
                impl<T: Clone> AmbiguousIfClone<ImplementsClone> for T {}
                let _ = <$type as AmbiguousIfClone<_>>::assert_not_clone;
            };
        };
    }

    macro_rules! assert_not_serialize {
        ($type:ty) => {
            const _: fn() = || {
                trait AmbiguousIfSerialize<Marker> {
                    fn assert_not_serialize() {}
                }
                struct ImplementsSerialize;
                impl<T: ?Sized> AmbiguousIfSerialize<()> for T {}
                impl<T: ?Sized + serde::Serialize> AmbiguousIfSerialize<ImplementsSerialize> for T {}
                let _ = <$type as AmbiguousIfSerialize<_>>::assert_not_serialize;
            };
        };
    }

    macro_rules! assert_not_deserialize_owned {
        ($type:ty) => {
            const _: fn() = || {
                trait AmbiguousIfDeserialize<Marker> {
                    fn assert_not_deserialize() {}
                }
                struct ImplementsDeserialize;
                impl<T: ?Sized> AmbiguousIfDeserialize<()> for T {}
                impl<T: serde::de::DeserializeOwned> AmbiguousIfDeserialize<ImplementsDeserialize>
                    for T
                {
                }
                let _ = <$type as AmbiguousIfDeserialize<_>>::assert_not_deserialize;
            };
        };
    }

    assert_not_clone!(OriginalMergeRollbackCheckpointRequest<'static>);
    assert_not_clone!(OriginalMergeRollbackCheckpointInvocationCapability);
    assert_not_clone!(OriginalMergeRollbackCheckpointCompletionCapability);
    assert_not_clone!(OriginalMergeRollbackCheckpointCompletion);
    assert_not_clone!(OriginalMergeRollbackCheckpointFailureEvidence);
    assert_not_clone!(OriginalMergeRollbackCheckpointBlockedAuthority);
    assert_not_clone!(VerifiedOriginalRollbackCheckpointAuthority);
    assert_not_clone!(OriginalMergePreIntentAuthority);
    assert_not_clone!(OriginalMergePreIntentRequest<'static>);
    assert_not_clone!(OriginalMergePreIntentInvocationCapability);
    assert_not_clone!(OriginalMergePreIntentCompletionCapability);
    assert_not_clone!(OriginalMergePreIntentCompletion);
    assert_not_clone!(OriginalMergeExecutionRequest<'static>);
    assert_not_clone!(OriginalMergeExecutionAttemptCapability);
    assert_not_clone!(OriginalMergeExecutionCompletionCapability);
    assert_not_clone!(OriginalMergeEffectObservationAuthority);
    assert_not_clone!(OriginalMergeExecutionPortOutcome);
    assert_not_clone!(OriginalMergeExecutionBlockedAuthority);
    assert_not_clone!(PendingOriginalMergeReceiptAuthority);
    assert_not_clone!(ValidatedSupportGateOriginalMergeCasAuthority);
    assert_not_clone!(SupportGateOriginalMergeCasRequest<'static>);
    assert_not_clone!(SupportGateOriginalMergeCasInvocationCapability);
    assert_not_clone!(SupportGateOriginalMergeCasCompletionCapability);
    assert_not_clone!(SupportGateOriginalMergeCasResolution);
    assert_not_clone!(ConsumedSupportGateAuthority);
    assert_not_serialize!(OriginalMergeRollbackCheckpointRequest<'static>);
    assert_not_serialize!(OriginalMergeRollbackCheckpointInvocationCapability);
    assert_not_serialize!(OriginalMergeRollbackCheckpointCompletionCapability);
    assert_not_serialize!(OriginalMergeRollbackCheckpointCompletion);
    assert_not_serialize!(OriginalMergeRollbackCheckpointFailureEvidence);
    assert_not_serialize!(OriginalMergeRollbackCheckpointBlockedAuthority);
    assert_not_serialize!(VerifiedOriginalRollbackCheckpointAuthority);
    assert_not_serialize!(OriginalMergePreIntentAuthority);
    assert_not_serialize!(OriginalMergePreIntentRequest<'static>);
    assert_not_serialize!(OriginalMergePreIntentInvocationCapability);
    assert_not_serialize!(OriginalMergePreIntentCompletionCapability);
    assert_not_serialize!(OriginalMergePreIntentCompletion);
    assert_not_serialize!(OriginalMergeExecutionRequest<'static>);
    assert_not_serialize!(OriginalMergeExecutionAttemptCapability);
    assert_not_serialize!(OriginalMergeExecutionCompletionCapability);
    assert_not_serialize!(OriginalMergeEffectObservationAuthority);
    assert_not_serialize!(OriginalMergeExecutionPortOutcome);
    assert_not_serialize!(OriginalMergeExecutionBlockedAuthority);
    assert_not_serialize!(PendingOriginalMergeReceiptAuthority);
    assert_not_serialize!(ValidatedSupportGateOriginalMergeCasAuthority);
    assert_not_serialize!(SupportGateOriginalMergeCasRequest<'static>);
    assert_not_serialize!(SupportGateOriginalMergeCasInvocationCapability);
    assert_not_serialize!(SupportGateOriginalMergeCasCompletionCapability);
    assert_not_serialize!(SupportGateOriginalMergeCasResolution);
    assert_not_serialize!(ConsumedSupportGateAuthority);
    assert_not_deserialize_owned!(OriginalMergeRollbackCheckpointRequest<'static>);
    assert_not_deserialize_owned!(OriginalMergeRollbackCheckpointInvocationCapability);
    assert_not_deserialize_owned!(OriginalMergeRollbackCheckpointCompletionCapability);
    assert_not_deserialize_owned!(OriginalMergeRollbackCheckpointCompletion);
    assert_not_deserialize_owned!(OriginalMergeRollbackCheckpointFailureEvidence);
    assert_not_deserialize_owned!(OriginalMergeRollbackCheckpointBlockedAuthority);
    assert_not_deserialize_owned!(VerifiedOriginalRollbackCheckpointAuthority);
    assert_not_deserialize_owned!(OriginalMergePreIntentAuthority);
    assert_not_deserialize_owned!(OriginalMergePreIntentRequest<'static>);
    assert_not_deserialize_owned!(OriginalMergePreIntentInvocationCapability);
    assert_not_deserialize_owned!(OriginalMergePreIntentCompletionCapability);
    assert_not_deserialize_owned!(OriginalMergePreIntentCompletion);
    assert_not_deserialize_owned!(OriginalMergeExecutionRequest<'static>);
    assert_not_deserialize_owned!(OriginalMergeExecutionAttemptCapability);
    assert_not_deserialize_owned!(OriginalMergeExecutionCompletionCapability);
    assert_not_deserialize_owned!(OriginalMergeEffectObservationAuthority);
    assert_not_deserialize_owned!(OriginalMergeExecutionPortOutcome);
    assert_not_deserialize_owned!(OriginalMergeExecutionBlockedAuthority);
    assert_not_deserialize_owned!(PendingOriginalMergeReceiptAuthority);
    assert_not_deserialize_owned!(ValidatedSupportGateOriginalMergeCasAuthority);
    assert_not_deserialize_owned!(SupportGateOriginalMergeCasRequest<'static>);
    assert_not_deserialize_owned!(SupportGateOriginalMergeCasInvocationCapability);
    assert_not_deserialize_owned!(SupportGateOriginalMergeCasCompletionCapability);
    assert_not_deserialize_owned!(SupportGateOriginalMergeCasResolution);
    assert_not_deserialize_owned!(ConsumedSupportGateAuthority);

    struct ResponseLossCasLease {
        state: Rc<RefCell<Option<PersistedAtomicState>>>,
    }

    impl SupportGateOriginalMergeCasLease for ResponseLossCasLease {
        fn binds(&self, _binding: &SupportGateOriginalMergeCasBinding) -> bool {
            true
        }

        fn commit_receipt_and_consume_gate(
            self: Box<Self>,
            pending: &PendingOriginalMergeReceiptAuthority,
        ) -> Result<(), MergeResultContractError> {
            *self.state.borrow_mut() = Some(PersistedAtomicState {
                merge_receipt_id: pending.merge_receipt_id().clone(),
                result_fingerprint: pending.result_fingerprint().clone(),
                plan_id: pending.plan_id().clone(),
                receipt_persisted: true,
                gate_consumed: true,
            });
            Err(MergeResultContractError("atomic response was lost"))
        }
    }

    struct ResponseLossCasResolver {
        state: Rc<RefCell<Option<PersistedAtomicState>>>,
    }

    impl SupportGateOriginalMergeCasResolver for ResponseLossCasResolver {
        fn resolve_original_merge_cas(
            &mut self,
            request: SupportGateOriginalMergeCasRequest<'_>,
        ) -> Result<SupportGateOriginalMergeCasResolution, MergeResultContractError> {
            Ok(request.complete(Box::new(ResponseLossCasLease {
                state: Rc::clone(&self.state),
            })))
        }
    }

    struct TestConsumedGateLease {
        state: PersistedAtomicState,
        consumed_state_revision: Sha256Digest,
        capability_id: CapabilityRowId,
    }

    impl ConsumedSupportGateStateLease for TestConsumedGateLease {
        fn binds(&self, request: &ConsumedSupportGateObservationRequest<'_>) -> bool {
            request.merge_receipt_id() == &self.state.merge_receipt_id
                && request.result_fingerprint() == &self.state.result_fingerprint
                && request.plan_id() == &self.state.plan_id
                && self.state.receipt_persisted
                && self.state.gate_consumed
        }

        fn consumed_state_revision(&self) -> &Sha256Digest {
            &self.consumed_state_revision
        }

        fn observation_capability_id(&self) -> &CapabilityRowId {
            &self.capability_id
        }
    }

    struct TestConsumedGateResolver {
        state: PersistedAtomicState,
    }

    impl ConsumedSupportGateStateResolver for TestConsumedGateResolver {
        fn resolve_consumed_by_original_merge(
            &mut self,
            request: ConsumedSupportGateObservationRequest<'_>,
        ) -> Result<ConsumedSupportGateStateResolution, MergeResultContractError> {
            Ok(request.complete(Box::new(TestConsumedGateLease {
                state: self.state.clone(),
                consumed_state_revision: digest('c'),
                capability_id: CapabilityRowId::parse("repository.consumed-gate.observe").unwrap(),
            })))
        }
    }

    struct ResolveThenReplayConsumedGateResolver {
        stored_resolution: Option<ConsumedSupportGateStateResolution>,
    }

    impl ConsumedSupportGateStateResolver for ResolveThenReplayConsumedGateResolver {
        fn resolve_consumed_by_original_merge(
            &mut self,
            request: ConsumedSupportGateObservationRequest<'_>,
        ) -> Result<ConsumedSupportGateStateResolution, MergeResultContractError> {
            if self.stored_resolution.is_none() {
                let lease = TestConsumedGateLease {
                    state: PersistedAtomicState {
                        merge_receipt_id: request.merge_receipt_id().clone(),
                        result_fingerprint: request.result_fingerprint().clone(),
                        plan_id: request.plan_id().clone(),
                        receipt_persisted: true,
                        gate_consumed: true,
                    },
                    consumed_state_revision: digest('c'),
                    capability_id: CapabilityRowId::parse(
                        "repository.consumed-gate.replayed-lease",
                    )
                    .unwrap(),
                };
                self.stored_resolution = Some(request.complete(Box::new(lease)));
                return Err(MergeResultContractError(
                    "consumed-state response was lost after lease creation",
                ));
            }
            self.stored_resolution
                .take()
                .ok_or(MergeResultContractError(
                    "stored consumed-state lease already replayed",
                ))
        }
    }

    fn successful_receipt_and_state(
        receipt_id: UnicaId,
        fingerprint: Sha256Digest,
    ) -> (ValidatedOriginalMergeReceiptAuthority, PersistedAtomicState) {
        successful_receipt_and_state_with_hidden_lineage(
            receipt_id,
            fingerprint,
            id("b2000000-0000-4000-8000-000000000002"),
            "repository.original.preintent",
        )
    }

    fn successful_receipt_and_state_with_hidden_lineage(
        receipt_id: UnicaId,
        fingerprint: Sha256Digest,
        rollback_checkpoint_id: UnicaId,
        preintent_capability_id: &'static str,
    ) -> (ValidatedOriginalMergeReceiptAuthority, PersistedAtomicState) {
        let pending = pending_original_merge_with_lineage(
            receipt_id,
            fingerprint,
            rollback_checkpoint_id,
            preintent_capability_id,
            digest('1'),
            digest('2'),
            digest('4'),
        );
        let state = Rc::new(RefCell::new(None));
        let mut resolver = TestCasResolver {
            state: Rc::clone(&state),
            expected_receipt_id: pending.merge_receipt_id().clone(),
            expected_fingerprint: pending.result_fingerprint().clone(),
            expected_plan_id: pending.plan_id().clone(),
        };
        let receipt =
            ValidatedSupportGateOriginalMergeCasAuthority::resolve(pending, &mut resolver)
                .unwrap()
                .commit()
                .unwrap();
        let persisted = state.borrow().clone().unwrap();
        (receipt, persisted)
    }

    fn consumed_lineage(
        receipt_id: UnicaId,
        fingerprint: Sha256Digest,
    ) -> ValidatedConsumedOriginalMergeLineageAuthority {
        let (receipt, state) = successful_receipt_and_state(receipt_id, fingerprint);
        ResolvedReceiptConsumedSupportGateAuthority::resolve(
            receipt,
            &mut TestConsumedGateResolver { state },
        )
        .unwrap()
        .rebind()
    }

    fn consumed_lineage_with_hidden_lineage(
        receipt_id: UnicaId,
        fingerprint: Sha256Digest,
        rollback_checkpoint_id: UnicaId,
        preintent_capability_id: &'static str,
    ) -> ValidatedConsumedOriginalMergeLineageAuthority {
        let (receipt, state) = successful_receipt_and_state_with_hidden_lineage(
            receipt_id,
            fingerprint,
            rollback_checkpoint_id,
            preintent_capability_id,
        );
        ResolvedReceiptConsumedSupportGateAuthority::resolve(
            receipt,
            &mut TestConsumedGateResolver { state },
        )
        .unwrap()
        .rebind()
    }

    struct MainIntegrationLineageExpectation {
        receipt_id: UnicaId,
        lock_set_id: UnicaId,
        rollback_checkpoint_id: UnicaId,
        current_state_revision: Sha256Digest,
        preintent_capability_id: CapabilityRowId,
        root_lock_receipt: JournaledRepositoryLock,
        journaled_lock_receipts: Vec<JournaledRepositoryLock>,
        consumed_state_revision: Sha256Digest,
        consumed_observation_capability_id: CapabilityRowId,
    }

    struct MainIntegrationFailureExpectation {
        lineage: MainIntegrationLineageExpectation,
        verifier_observation_receipt_id: UnicaId,
        verifier_observation_id: UnicaId,
    }

    fn main_integration_lineage_expectation(
        lineage: &ValidatedConsumedOriginalMergeLineageAuthority,
    ) -> MainIntegrationLineageExpectation {
        MainIntegrationLineageExpectation {
            receipt_id: lineage.merge_receipt_id().clone(),
            lock_set_id: lineage.lock_set_id().clone(),
            rollback_checkpoint_id: lineage.receipt().rollback_checkpoint_id().clone(),
            current_state_revision: lineage
                .receipt()
                .consumed_gate()
                .current_state_revision()
                .clone(),
            preintent_capability_id: lineage
                .receipt()
                .consumed_gate()
                .binding
                .preintent_capability_id
                .clone(),
            root_lock_receipt: lineage
                .receipt()
                .consumed_gate()
                .binding
                .root_lock_receipt
                .clone(),
            journaled_lock_receipts: lineage
                .receipt()
                .lock_projection()
                .journaled_lock_receipts()
                .unwrap()
                .to_vec(),
            consumed_state_revision: lineage.observed.consumed_state_revision().clone(),
            consumed_observation_capability_id: lineage
                .observed
                .observation_capability_id()
                .clone(),
        }
    }

    fn assert_main_integration_lineage(
        lineage: &ValidatedConsumedOriginalMergeLineageAuthority,
        expected: &MainIntegrationLineageExpectation,
    ) {
        assert_eq!(lineage.merge_receipt_id(), &expected.receipt_id);
        assert_eq!(lineage.lock_set_id(), &expected.lock_set_id);
        assert_eq!(
            lineage.receipt().rollback_checkpoint_id(),
            &expected.rollback_checkpoint_id
        );
        assert_eq!(
            lineage.receipt().consumed_gate().current_state_revision(),
            &expected.current_state_revision
        );
        assert_eq!(
            &lineage
                .receipt()
                .consumed_gate()
                .binding
                .preintent_capability_id,
            &expected.preintent_capability_id
        );
        assert_eq!(
            &lineage.receipt().consumed_gate().binding.root_lock_receipt,
            &expected.root_lock_receipt
        );
        assert_eq!(
            lineage
                .receipt()
                .lock_projection()
                .journaled_lock_receipts()
                .unwrap(),
            expected.journaled_lock_receipts
        );
        assert_eq!(
            lineage.observed.consumed_state_revision(),
            &expected.consumed_state_revision
        );
        assert_eq!(
            lineage.observed.observation_capability_id(),
            &expected.consumed_observation_capability_id
        );
    }

    fn main_integration_failure_expectation(
        observation: &MainIntegrationVerificationObservationAuthority,
    ) -> MainIntegrationFailureExpectation {
        MainIntegrationFailureExpectation {
            lineage: main_integration_lineage_expectation(&observation.lineage.0),
            verifier_observation_receipt_id: observation.evidence.merge_receipt_id.clone(),
            verifier_observation_id: observation.evidence.common.verification_id.clone(),
        }
    }

    fn assert_main_integration_failure_retains_owned_stage(
        blocked: Box<MainIntegrationVerificationBlockedAuthority>,
        expected: MainIntegrationFailureExpectation,
        expected_failure: MainIntegrationVerificationFailureEvidence,
    ) {
        let (lineage, observation, failure) = blocked.into_recovery_parts();
        assert_eq!(failure, expected_failure);
        assert_main_integration_lineage(&lineage, &expected.lineage);
        assert_eq!(
            observation.merge_receipt_id,
            expected.verifier_observation_receipt_id
        );
        assert_eq!(
            observation.common.verification_id,
            expected.verifier_observation_id
        );
    }

    #[test]
    fn original_merge_response_loss_resolves_the_same_consumed_gate_and_receipt() {
        let pending = pending_original_merge(id(RECEIPT), digest('3'));
        let state = Rc::new(RefCell::new(None));
        let cas = ValidatedSupportGateOriginalMergeCasAuthority::resolve(
            pending,
            &mut ResponseLossCasResolver {
                state: Rc::clone(&state),
            },
        )
        .unwrap();
        let blocked = cas.commit().unwrap_err();
        let pending = blocked.into_pending();
        let persisted = state.borrow().clone().unwrap();
        let resolved = ResolvedPendingConsumedSupportGateAuthority::resolve(
            pending,
            &mut TestConsumedGateResolver {
                state: persisted.clone(),
            },
        )
        .unwrap();
        let receipt = resolved.finalize();
        assert_eq!(receipt.merge_receipt_id(), &persisted.merge_receipt_id);
        assert_eq!(receipt.result_fingerprint(), &persisted.result_fingerprint);
        assert_eq!(receipt.plan_id(), &persisted.plan_id);
    }

    #[test]
    fn consumed_state_resolver_rejects_completed_lease_replay_for_equivalent_pending() {
        let make_pending = || {
            pending_original_merge_with_lineage(
                id("b2000000-0000-4000-8000-000000000060"),
                digest('3'),
                id("b2000000-0000-4000-8000-000000000061"),
                "repository.original.preintent.consumed-resolver-replay",
                digest('1'),
                digest('2'),
                digest('4'),
            )
        };
        let pending_a = make_pending();
        let pending_b = make_pending();
        let mut resolver = ResolveThenReplayConsumedGateResolver {
            stored_resolution: None,
        };

        assert!(
            ResolvedPendingConsumedSupportGateAuthority::resolve(pending_a, &mut resolver,)
                .is_err()
        );
        assert!(
            ResolvedPendingConsumedSupportGateAuthority::resolve(pending_b, &mut resolver,)
                .is_err()
        );
    }

    #[test]
    fn response_loss_resolution_owns_exact_pending_through_infallible_finalization() {
        let pending = pending_original_merge_with_lineage(
            id("b2000000-0000-4000-8000-000000000062"),
            digest('3'),
            id("b2000000-0000-4000-8000-000000000063"),
            "repository.original.preintent.response-loss-replay",
            digest('1'),
            digest('2'),
            digest('4'),
        );
        let expected_receipt = pending.merge_receipt_id().clone();
        let state = PersistedAtomicState {
            merge_receipt_id: pending.merge_receipt_id().clone(),
            result_fingerprint: pending.result_fingerprint().clone(),
            plan_id: pending.plan_id().clone(),
            receipt_persisted: true,
            gate_consumed: true,
        };
        let resolved = ResolvedPendingConsumedSupportGateAuthority::resolve(
            pending,
            &mut TestConsumedGateResolver { state },
        )
        .unwrap();
        let _finalize_signature: fn(
            ResolvedPendingConsumedSupportGateAuthority,
        ) -> ValidatedOriginalMergeReceiptAuthority =
            ResolvedPendingConsumedSupportGateAuthority::finalize;

        let receipt = resolved.finalize();
        assert_eq!(receipt.merge_receipt_id(), &expected_receipt);
        assert_eq!(
            receipt.consumed_gate().binding.preintent_capability_id,
            CapabilityRowId::parse("repository.original.preintent.response-loss-replay").unwrap()
        );
    }

    #[test]
    fn consumed_rebind_resolution_owns_exact_receipt_without_injection_point() {
        let (receipt, state) = successful_receipt_and_state_with_hidden_lineage(
            id("b2000000-0000-4000-8000-000000000064"),
            digest('3'),
            id("b2000000-0000-4000-8000-000000000065"),
            "repository.original.preintent.consumed-rebind-replay",
        );
        let expected_receipt = receipt.merge_receipt_id().clone();
        let resolved = ResolvedReceiptConsumedSupportGateAuthority::resolve(
            receipt,
            &mut TestConsumedGateResolver { state },
        )
        .unwrap();
        let _rebind_signature: fn(
            ResolvedReceiptConsumedSupportGateAuthority,
        ) -> ValidatedConsumedOriginalMergeLineageAuthority =
            ResolvedReceiptConsumedSupportGateAuthority::rebind;

        let lineage = resolved.rebind();
        assert_eq!(lineage.merge_receipt_id(), &expected_receipt);
        assert_eq!(
            lineage.consumed_gate().binding.preintent_capability_id,
            CapabilityRowId::parse("repository.original.preintent.consumed-rebind-replay").unwrap()
        );
        assert_eq!(
            lineage.observed.observation_capability_id(),
            &CapabilityRowId::parse("repository.consumed-gate.observe").unwrap()
        );
    }

    #[test]
    fn consumed_pending_resolution_binding_failure_retains_exact_pending() {
        let pending = pending_original_merge_with_lineage(
            id(RECEIPT),
            digest('3'),
            id("b2000000-0000-4000-8000-000000000002"),
            "repository.original.preintent",
            digest('1'),
            digest('2'),
            digest('4'),
        );
        let expected_receipt = pending.merge_receipt_id().clone();
        let expected_rollback = pending.data.rollback_checkpoint_id.clone();
        let state = PersistedAtomicState {
            merge_receipt_id: id("b2000000-0000-4000-8000-000000000099"),
            result_fingerprint: pending.result_fingerprint().clone(),
            plan_id: pending.plan_id().clone(),
            receipt_persisted: true,
            gate_consumed: true,
        };
        let blocked = ResolvedPendingConsumedSupportGateAuthority::resolve(
            pending,
            &mut TestConsumedGateResolver { state },
        )
        .unwrap_err();
        let (retained_pending, failure) = blocked.into_recovery_parts();
        assert_eq!(retained_pending.merge_receipt_id(), &expected_receipt);
        assert_eq!(
            retained_pending.data.rollback_checkpoint_id,
            expected_rollback
        );
        assert_eq!(
            failure,
            MergeResultContractError(
                "authoritative state did not resolve the exact consumed support gate and receipt"
            )
        );
    }

    #[test]
    fn consumed_receipt_resolution_binding_failure_retains_exact_receipt() {
        let (receipt, _state) = successful_receipt_and_state(id(RECEIPT), digest('3'));
        let expected_receipt = receipt.merge_receipt_id().clone();
        let expected_rollback = receipt.rollback_checkpoint_id().clone();
        let state = PersistedAtomicState {
            merge_receipt_id: id("b2000000-0000-4000-8000-000000000098"),
            result_fingerprint: receipt.result_fingerprint().clone(),
            plan_id: receipt.plan_id().clone(),
            receipt_persisted: true,
            gate_consumed: true,
        };
        let blocked = ResolvedReceiptConsumedSupportGateAuthority::resolve(
            receipt,
            &mut TestConsumedGateResolver { state },
        )
        .unwrap_err();
        let (retained_receipt, failure) = blocked.into_recovery_parts();
        assert_eq!(retained_receipt.merge_receipt_id(), &expected_receipt);
        assert_eq!(
            retained_receipt.rollback_checkpoint_id(),
            &expected_rollback
        );
        assert_eq!(
            failure,
            MergeResultContractError(
                "authoritative state did not resolve the exact consumed support gate and receipt"
            )
        );
    }

    #[test]
    fn main_integration_execution_rejects_replayed_duplicate_selection_and_retains_second_lineage()
    {
        let lineage_a = consumed_lineage_with_hidden_lineage(
            id(RECEIPT),
            digest('3'),
            id("b2000000-0000-4000-8000-000000000010"),
            "repository.original.preintent.replay",
        );
        let lineage_b = consumed_lineage_with_hidden_lineage(
            id(RECEIPT),
            digest('3'),
            id("b2000000-0000-4000-8000-000000000010"),
            "repository.original.preintent.replay",
        );
        assert_eq!(lineage_a, lineage_b);
        let expected_b = main_integration_lineage_expectation(&lineage_b);
        let plan =
            ConfiguredValidationCheckPlanAuthority::from_configuration_adapter(vec![]).unwrap();
        let verification_id = id("b2000000-0000-4000-8000-000000000011");
        let selection_a = ConfiguredValidationExecutionSelectionAuthority::main_integration(
            &plan,
            lineage_a,
            verification_id.clone(),
        );
        let selection_b = ConfiguredValidationExecutionSelectionAuthority::main_integration(
            &plan,
            lineage_b,
            verification_id,
        );
        let mut port = ConsumerFixtureConfiguredValidationPort {
            snapshot_input: Some(
                ConfiguredValidationReceiptBatchSnapshotInput::from_execution_adapter(
                    vec![],
                    vec![],
                ),
            ),
        };
        let first =
            ConfiguredValidationReceiptBatchAuthority::from_execution_port(selection_a, &mut port)
                .unwrap();
        assert_eq!(
            first
                .selection
                .main_integration_lineage()
                .receipt()
                .merge_receipt_id(),
            &id(RECEIPT)
        );

        let blocked =
            ConfiguredValidationReceiptBatchAuthority::from_execution_port(selection_b, &mut port)
                .unwrap_err();
        let (selection, failure) = blocked.into_recovery_parts();
        assert_eq!(
            failure,
            MergeResultContractError("consumer fixture validation snapshot replayed")
        );
        assert_main_integration_lineage(&selection.main_integration_lineage().0, &expected_b);
    }

    #[test]
    fn configured_validation_snapshot_replay_rejects_cross_scope_attempt_and_retains_current_selection(
    ) {
        let plan =
            ConfiguredValidationCheckPlanAuthority::from_configuration_adapter(vec![]).unwrap();
        let local_verification_id = id("b2000000-0000-4000-8000-000000000040");
        let local_selection = ConfiguredValidationExecutionSelectionAuthority::local_checkpoint(
            &plan,
            local_verification_id.clone(),
        );
        let current_lineage = consumed_lineage_with_hidden_lineage(
            id(RECEIPT),
            digest('3'),
            id("b2000000-0000-4000-8000-000000000041"),
            "repository.original.preintent.snapshot-cross-scope",
        );
        let expected_current = main_integration_lineage_expectation(&current_lineage);
        let current_selection = ConfiguredValidationExecutionSelectionAuthority::main_integration(
            &plan,
            current_lineage,
            id("b2000000-0000-4000-8000-000000000046"),
        );
        let mut port = CompletionThenReplayConfiguredValidationPort {
            snapshot_input: Some(
                ConfiguredValidationReceiptBatchSnapshotInput::from_execution_adapter(
                    vec![],
                    vec![],
                ),
            ),
            stored_snapshot: None,
        };

        let first_blocked = ConfiguredValidationReceiptBatchAuthority::from_execution_port(
            local_selection,
            &mut port,
        )
        .unwrap_err();
        let (local_selection, failure) = first_blocked.into_recovery_parts();
        assert_eq!(
            failure,
            MergeResultContractError("configured validation response was lost after completion")
        );
        assert!(local_selection.matches_local_checkpoint(&local_verification_id));

        let blocked = ConfiguredValidationReceiptBatchAuthority::from_execution_port(
            current_selection,
            &mut port,
        )
        .unwrap_err();
        let (current_selection, failure) = blocked.into_recovery_parts();
        assert_eq!(
            failure,
            MergeResultContractError(
                "configured validation completion belongs to another execution attempt"
            )
        );
        assert_main_integration_lineage(
            &current_selection.main_integration_lineage().0,
            &expected_current,
        );
    }

    #[test]
    fn configured_validation_snapshot_replay_rejects_same_scope_hidden_lineage_and_retains_current_selection(
    ) {
        let lineage_a = consumed_lineage_with_hidden_lineage(
            id(RECEIPT),
            digest('3'),
            id("b2000000-0000-4000-8000-000000000042"),
            "repository.original.preintent.snapshot-a",
        );
        let lineage_b = consumed_lineage_with_hidden_lineage(
            id(RECEIPT),
            digest('3'),
            id("b2000000-0000-4000-8000-000000000042"),
            "repository.original.preintent.snapshot-b",
        );
        let expected_a = main_integration_lineage_expectation(&lineage_a);
        let expected_b = main_integration_lineage_expectation(&lineage_b);
        let plan =
            ConfiguredValidationCheckPlanAuthority::from_configuration_adapter(vec![]).unwrap();
        let verification_id = id("b2000000-0000-4000-8000-000000000043");
        let selection_a = ConfiguredValidationExecutionSelectionAuthority::main_integration(
            &plan,
            lineage_a,
            verification_id.clone(),
        );
        let selection_b = ConfiguredValidationExecutionSelectionAuthority::main_integration(
            &plan,
            lineage_b,
            verification_id,
        );
        let mut port = CompletionThenReplayConfiguredValidationPort {
            snapshot_input: Some(
                ConfiguredValidationReceiptBatchSnapshotInput::from_execution_adapter(
                    vec![],
                    vec![],
                ),
            ),
            stored_snapshot: None,
        };

        let first_blocked =
            ConfiguredValidationReceiptBatchAuthority::from_execution_port(selection_a, &mut port)
                .unwrap_err();
        let (selection_a, _) = first_blocked.into_recovery_parts();
        assert_main_integration_lineage(&selection_a.main_integration_lineage().0, &expected_a);

        let blocked =
            ConfiguredValidationReceiptBatchAuthority::from_execution_port(selection_b, &mut port)
                .unwrap_err();
        let (selection_b, failure) = blocked.into_recovery_parts();
        assert_eq!(
            failure,
            MergeResultContractError(
                "configured validation completion belongs to another execution attempt"
            )
        );
        assert_main_integration_lineage(&selection_b.main_integration_lineage().0, &expected_b);
    }

    #[test]
    fn configured_validation_snapshot_replay_rejects_same_selection_retry_under_new_attempt() {
        let lineage = consumed_lineage_with_hidden_lineage(
            id(RECEIPT),
            digest('3'),
            id("b2000000-0000-4000-8000-000000000044"),
            "repository.original.preintent.snapshot-retry",
        );
        let expected = main_integration_lineage_expectation(&lineage);
        let plan =
            ConfiguredValidationCheckPlanAuthority::from_configuration_adapter(vec![]).unwrap();
        let selection = ConfiguredValidationExecutionSelectionAuthority::main_integration(
            &plan,
            lineage,
            id("b2000000-0000-4000-8000-000000000045"),
        );
        let mut port = CompletionThenReplayConfiguredValidationPort {
            snapshot_input: Some(
                ConfiguredValidationReceiptBatchSnapshotInput::from_execution_adapter(
                    vec![],
                    vec![],
                ),
            ),
            stored_snapshot: None,
        };

        let first_blocked =
            ConfiguredValidationReceiptBatchAuthority::from_execution_port(selection, &mut port)
                .unwrap_err();
        let (selection, _) = first_blocked.into_recovery_parts();

        let blocked =
            ConfiguredValidationReceiptBatchAuthority::from_execution_port(selection, &mut port)
                .unwrap_err();
        let (selection, failure) = blocked.into_recovery_parts();
        assert_eq!(
            failure,
            MergeResultContractError(
                "configured validation completion belongs to another execution attempt"
            )
        );
        assert_main_integration_lineage(&selection.main_integration_lineage().0, &expected);
    }

    #[test]
    fn main_integration_verifier_input_failure_retains_exact_selection() {
        let lineage = consumed_lineage_with_hidden_lineage(
            id(RECEIPT),
            digest('3'),
            id("b2000000-0000-4000-8000-000000000028"),
            "repository.original.preintent.input-retained",
        );
        let verification_id = id("b2000000-0000-4000-8000-000000000029");
        let expected = main_integration_lineage_expectation(&lineage);
        let plan =
            ConfiguredValidationCheckPlanAuthority::from_configuration_adapter(vec![]).unwrap();
        let selection = ConfiguredValidationExecutionSelectionAuthority::main_integration(
            &plan,
            lineage,
            verification_id.clone(),
        );
        let mut port = ConsumerFixtureConfiguredValidationPort {
            snapshot_input: Some(
                ConfiguredValidationReceiptBatchSnapshotInput::from_execution_adapter(
                    vec![],
                    vec![],
                ),
            ),
        };
        let batch =
            ConfiguredValidationReceiptBatchAuthority::from_execution_port(selection, &mut port)
                .unwrap();

        let blocked = VerificationObservationInputAuthority::from_verifier_adapter(
            id("b2000000-0000-4000-8000-000000000030"),
            digest('5'),
            batch,
            digest('6'),
            vec![],
        )
        .unwrap_err();
        let (_, _, batch, _, _, failure) = blocked.into_recovery_parts();
        assert_eq!(
            failure,
            VerificationObservationInputFailureEvidence::VerificationIdMismatch
        );
        assert_eq!(batch.selection.subject.verification_id(), &verification_id);
        assert_main_integration_lineage(&batch.selection.main_integration_lineage().0, &expected);
    }

    #[test]
    fn main_integration_verifier_fingerprint_failure_retains_exact_selection() {
        let lineage = consumed_lineage_with_hidden_lineage(
            id(RECEIPT),
            digest('3'),
            id("b2000000-0000-4000-8000-000000000034"),
            "repository.original.preintent.fingerprint-retained",
        );
        let expected = main_integration_lineage_expectation(&lineage);
        let verification_id = id("b2000000-0000-4000-8000-000000000035");
        let plan =
            ConfiguredValidationCheckPlanAuthority::from_configuration_adapter(vec![]).unwrap();
        let selection = ConfiguredValidationExecutionSelectionAuthority::main_integration(
            &plan,
            lineage,
            verification_id.clone(),
        );
        let mut port = ConsumerFixtureConfiguredValidationPort {
            snapshot_input: Some(
                ConfiguredValidationReceiptBatchSnapshotInput::from_execution_adapter(
                    vec![],
                    vec![],
                ),
            ),
        };
        let batch =
            ConfiguredValidationReceiptBatchAuthority::from_execution_port(selection, &mut port)
                .unwrap();
        let root: RepositoryTargetIdentity =
            serde_json::from_value(serde_json::json!({ "targetKind": "configurationRoot" }))
                .unwrap();
        let candidates = vec![
            SelectedObjectFingerprint {
                target: root.clone(),
                fingerprint: digest('5'),
            },
            SelectedObjectFingerprint {
                target: root,
                fingerprint: digest('6'),
            },
        ];
        let expected_candidates = candidates.clone();

        let blocked = VerificationObservationInputAuthority::from_verifier_adapter(
            verification_id,
            digest('7'),
            batch,
            digest('8'),
            candidates,
        )
        .unwrap_err();
        let (_, _, batch, _, retained_candidates, failure) = blocked.into_recovery_parts();
        assert_eq!(retained_candidates, expected_candidates);
        assert!(matches!(
            failure,
            VerificationObservationInputFailureEvidence::InvalidSelectedObjectFingerprints(_)
        ));
        assert_main_integration_lineage(&batch.selection.main_integration_lineage().0, &expected);
    }

    #[test]
    fn main_integration_verifier_inputs_are_scope_typed_at_all_observation_boundaries() {
        let _local_valid: fn(
            LocalCheckpointVerificationObservationInputAuthority,
            UnicaId,
        ) -> Result<
            LocalCheckpointVerificationObservationAuthority,
            MergeResultContractError,
        > = LocalCheckpointVerificationObservationAuthority::valid_from_verifier_adapter;
        let _local_invalid: fn(
            LocalCheckpointVerificationObservationInputAuthority,
        ) -> Result<
            LocalCheckpointVerificationObservationAuthority,
            MergeResultContractError,
        > = LocalCheckpointVerificationObservationAuthority::invalid_from_verifier_adapter;
        let _synchronized_equivalent: fn(
            &MergeSessionData,
            ResolvedTaskVerificationObservationInputAuthority,
            UnicaId,
        ) -> Result<
            SynchronizedVerificationObservationAuthority,
            MergeResultContractError,
        > = SynchronizedVerificationObservationAuthority::equivalent_from_verifier_adapter;
        let _synchronized_invalid: fn(
            &MergeSessionData,
            ResolvedTaskVerificationObservationInputAuthority,
        ) -> Result<
            SynchronizedVerificationObservationAuthority,
            MergeResultContractError,
        > = SynchronizedVerificationObservationAuthority::invalid_from_verifier_adapter;
        let _unexpected: fn(
            &MergeSessionData,
            ResolvedTaskVerificationObservationInputAuthority,
            UnicaId,
            Sha256Digest,
        ) -> Result<
            SynchronizedUnexpectedVerificationObservationAuthority,
            MergeResultContractError,
        > = SynchronizedUnexpectedVerificationObservationAuthority::from_verifier_adapter;
        let _adapted: fn(
            &MergeSessionData,
            &CurrentAdaptationDecisionAuthority,
            ResolvedTaskVerificationObservationInputAuthority,
            UnicaId,
        ) -> Result<
            AdaptedVerificationObservationAuthority,
            MergeResultContractError,
        > = AdaptedVerificationObservationAuthority::from_verifier_adapter;
        let _sandbox_valid: fn(
            &ValidatedRepositoryPlanSessionProjection,
            MainSandboxVerificationObservationInputAuthority,
        ) -> Result<
            MainSandboxVerificationObservationAuthority,
            MergeResultContractError,
        > = MainSandboxVerificationObservationAuthority::valid_from_verifier_adapter;
        let _sandbox_invalid: fn(
            &ValidatedRepositoryPlanSessionProjection,
            MainSandboxVerificationObservationInputAuthority,
        ) -> Result<
            MainSandboxVerificationObservationAuthority,
            MergeResultContractError,
        > = MainSandboxVerificationObservationAuthority::invalid_from_verifier_adapter;
        let _main_valid: fn(
            MainIntegrationVerificationObservationInputAuthority,
        ) -> MainIntegrationVerificationObservationAuthority =
            MainIntegrationVerificationObservationAuthority::valid_from_verifier_adapter;
        let _main_invalid: fn(
            MainIntegrationVerificationObservationInputAuthority,
        ) -> MainIntegrationVerificationObservationAuthority =
            MainIntegrationVerificationObservationAuthority::invalid_from_verifier_adapter;
    }

    #[test]
    fn main_integration_valid_moves_hidden_lineage_without_rebinding() {
        let lineage = consumed_lineage_with_hidden_lineage(
            id(RECEIPT),
            digest('3'),
            id("b2000000-0000-4000-8000-000000000022"),
            "repository.original.preintent.hidden-a",
        );
        let expected = main_integration_lineage_expectation(&lineage);

        let plan =
            ConfiguredValidationCheckPlanAuthority::from_configuration_adapter(vec![]).unwrap();
        let selection = ConfiguredValidationExecutionSelectionAuthority::main_integration(
            &plan,
            lineage,
            id("b2000000-0000-4000-8000-000000000024"),
        );
        let observation =
            MainIntegrationVerificationObservationAuthority::valid_from_verifier_adapter(
                consumer_fixture_verification_input(selection, digest('5'), digest('6')),
            );
        let verified =
            MergeVerificationData::main_integration_valid_from_authorities(observation).unwrap();
        let commit = verified.into_commit_lineage();
        assert_main_integration_lineage(&commit.lineage, &expected);
    }

    #[test]
    fn main_integration_failure_wrong_outcome_retains_owned_stage() {
        let invalid_lineage = consumed_lineage(id(RECEIPT), digest('3'));
        let plan =
            ConfiguredValidationCheckPlanAuthority::from_configuration_adapter(vec![]).unwrap();
        let invalid_selection = ConfiguredValidationExecutionSelectionAuthority::main_integration(
            &plan,
            invalid_lineage,
            id("b2000000-0000-4000-8000-000000000016"),
        );
        let invalid_observation =
            MainIntegrationVerificationObservationAuthority::invalid_from_verifier_adapter(
                consumer_fixture_verification_input(invalid_selection, digest('5'), digest('6')),
            );
        let invalid_expected = main_integration_failure_expectation(&invalid_observation);
        let invalid_blocked =
            MergeVerificationData::main_integration_valid_from_authorities(invalid_observation)
                .unwrap_err();
        assert_main_integration_failure_retains_owned_stage(
            invalid_blocked,
            invalid_expected,
            MainIntegrationVerificationFailureEvidence::ValidConstructorReceivedInvalidOutcome,
        );

        let valid_lineage =
            consumed_lineage(id("b2000000-0000-4000-8000-000000000017"), digest('3'));
        let valid_selection = ConfiguredValidationExecutionSelectionAuthority::main_integration(
            &plan,
            valid_lineage,
            id("b2000000-0000-4000-8000-000000000018"),
        );
        let valid_observation =
            MainIntegrationVerificationObservationAuthority::valid_from_verifier_adapter(
                consumer_fixture_verification_input(valid_selection, digest('7'), digest('8')),
            );
        let valid_expected = main_integration_failure_expectation(&valid_observation);
        let valid_blocked =
            MergeVerificationData::main_integration_invalid_from_authorities(valid_observation)
                .unwrap_err();
        assert_main_integration_failure_retains_owned_stage(
            valid_blocked,
            valid_expected,
            MainIntegrationVerificationFailureEvidence::InvalidConstructorReceivedValidOutcome,
        );
    }

    #[test]
    fn main_integration_failure_digest_error_retains_owned_stage() {
        let valid_lineage = consumed_lineage(id(RECEIPT), digest('3'));
        let plan =
            ConfiguredValidationCheckPlanAuthority::from_configuration_adapter(vec![]).unwrap();
        let valid_selection = ConfiguredValidationExecutionSelectionAuthority::main_integration(
            &plan,
            valid_lineage,
            id("b2000000-0000-4000-8000-000000000019"),
        );
        let valid_observation =
            MainIntegrationVerificationObservationAuthority::valid_from_verifier_adapter(
                consumer_fixture_verification_input(valid_selection, digest('9'), digest('a')),
            );
        let valid_expected = main_integration_failure_expectation(&valid_observation);
        let valid_blocked =
            MergeVerificationData::main_integration_valid_from_authorities_using_digest(
                valid_observation,
                |_| {
                    Err(MergeResultContractError(
                        "forced verification digest failure",
                    ))
                },
            )
            .unwrap_err();
        assert_main_integration_failure_retains_owned_stage(
            valid_blocked,
            valid_expected,
            MainIntegrationVerificationFailureEvidence::DigestError(MergeResultContractError(
                "forced verification digest failure",
            )),
        );

        let invalid_lineage =
            consumed_lineage(id("b2000000-0000-4000-8000-000000000020"), digest('3'));
        let invalid_selection = ConfiguredValidationExecutionSelectionAuthority::main_integration(
            &plan,
            invalid_lineage,
            id("b2000000-0000-4000-8000-000000000021"),
        );
        let invalid_observation =
            MainIntegrationVerificationObservationAuthority::invalid_from_verifier_adapter(
                consumer_fixture_verification_input(invalid_selection, digest('b'), digest('c')),
            );
        let invalid_expected = main_integration_failure_expectation(&invalid_observation);
        let invalid_blocked =
            MergeVerificationData::main_integration_invalid_from_authorities_using_digest(
                invalid_observation,
                |_| {
                    Err(MergeResultContractError(
                        "forced verification digest failure",
                    ))
                },
            )
            .unwrap_err();
        assert_main_integration_failure_retains_owned_stage(
            invalid_blocked,
            invalid_expected,
            MainIntegrationVerificationFailureEvidence::DigestError(MergeResultContractError(
                "forced verification digest failure",
            )),
        );
    }

    #[test]
    fn main_integration_verification_preserves_consumed_gate_into_commit_lineage() {
        let lineage = consumed_lineage(id(RECEIPT), digest('3'));
        let expected_receipt = lineage.merge_receipt_id().clone();
        let expected_gate = lineage.support_gate_id().clone();
        let expected_lock = lineage.lock_set_id().clone();
        let plan =
            ConfiguredValidationCheckPlanAuthority::from_configuration_adapter(vec![]).unwrap();
        let selection = ConfiguredValidationExecutionSelectionAuthority::main_integration(
            &plan,
            lineage,
            id("b2000000-0000-4000-8000-000000000013"),
        );
        let observation =
            MainIntegrationVerificationObservationAuthority::valid_from_verifier_adapter(
                consumer_fixture_verification_input(selection, digest('7'), digest('8')),
            );
        let verified =
            MergeVerificationData::main_integration_valid_from_authorities(observation).unwrap();
        let commit = verified.into_commit_lineage();
        assert_eq!(commit.merge_receipt_id(), &expected_receipt);
        assert_eq!(commit.support_gate_id(), &expected_gate);
        assert_eq!(commit.lock_set_id(), &expected_lock);
        assert!(
            commit.consumed_gate().authorized_result_fingerprint() == commit.result_fingerprint()
        );
    }

    #[test]
    fn main_integration_invalid_consumes_concrete_consumed_lineage() {
        let lineage = consumed_lineage_with_hidden_lineage(
            id(RECEIPT),
            digest('3'),
            id("b2000000-0000-4000-8000-000000000025"),
            "repository.original.preintent.hidden-invalid",
        );
        let expected_receipt = lineage.merge_receipt_id().clone();
        let expected_rollback = lineage.receipt().rollback_checkpoint_id().clone();
        let expected_preintent_capability = lineage
            .consumed_gate()
            .binding
            .preintent_capability_id()
            .clone();
        let plan =
            ConfiguredValidationCheckPlanAuthority::from_configuration_adapter(vec![]).unwrap();
        let selection = ConfiguredValidationExecutionSelectionAuthority::main_integration(
            &plan,
            lineage,
            id("b2000000-0000-4000-8000-000000000014"),
        );
        let observation =
            MainIntegrationVerificationObservationAuthority::invalid_from_verifier_adapter(
                consumer_fixture_verification_input(selection, digest('9'), digest('a')),
            );

        let invalid =
            MergeVerificationData::main_integration_invalid_from_authorities(observation).unwrap();
        let MergeVerificationData::MainIntegrationInvalid(data) = invalid.data() else {
            unreachable!()
        };
        assert_eq!(data.merge_receipt_id, expected_receipt);
        let (lineage, observation, retained_data) = invalid.into_recovery_parts();
        assert_eq!(lineage.merge_receipt_id(), &expected_receipt);
        assert_eq!(
            lineage.receipt().rollback_checkpoint_id(),
            &expected_rollback
        );
        assert_eq!(
            lineage.consumed_gate().binding.preintent_capability_id(),
            &expected_preintent_capability
        );
        assert_eq!(observation.merge_receipt_id, expected_receipt);
        assert_eq!(retained_data.merge_receipt_id, expected_receipt);
    }

    #[test]
    fn main_integration_api_has_concrete_consumed_lineage_signatures() {
        type MainIntegrationInputConstructor = fn(
            UnicaId,
            Sha256Digest,
            MainIntegrationConfiguredValidationReceiptBatchAuthority,
            Sha256Digest,
            Vec<SelectedObjectFingerprint>,
        ) -> Result<
            MainIntegrationVerificationObservationInputAuthority,
            Box<MainIntegrationVerificationObservationInputBlockedAuthority>,
        >;

        let _selection_constructor: fn(
            &ConfiguredValidationCheckPlanAuthority,
            ValidatedConsumedOriginalMergeLineageAuthority,
            UnicaId,
        ) -> MainIntegrationConfiguredValidationExecutionSelectionAuthority =
            ConfiguredValidationExecutionSelectionAuthority::main_integration;
        let _execution_constructor: fn(
            MainIntegrationConfiguredValidationExecutionSelectionAuthority,
            &mut dyn ConfiguredValidationCheckExecutionPort,
        ) -> Result<
            MainIntegrationConfiguredValidationReceiptBatchAuthority,
            Box<MainIntegrationConfiguredValidationExecutionBlockedAuthority>,
        > = ConfiguredValidationReceiptBatchAuthority::<
            MainIntegrationValidationExecutionScope,
        >::from_execution_port;
        let _input_constructor: MainIntegrationInputConstructor =
            VerificationObservationInputAuthority::<
            MainIntegrationValidationExecutionScope,
        >::from_verifier_adapter;
        let _valid_observation_constructor: fn(
            MainIntegrationVerificationObservationInputAuthority,
        )
            -> MainIntegrationVerificationObservationAuthority =
            MainIntegrationVerificationObservationAuthority::valid_from_verifier_adapter;
        let _invalid_observation_constructor: fn(
            MainIntegrationVerificationObservationInputAuthority,
        ) -> MainIntegrationVerificationObservationAuthority =
            MainIntegrationVerificationObservationAuthority::invalid_from_verifier_adapter;
        let _valid_result_constructor: fn(
            MainIntegrationVerificationObservationAuthority,
        ) -> Result<
            ValidatedMainIntegrationVerificationAuthority,
            Box<MainIntegrationVerificationBlockedAuthority>,
        > = MergeVerificationData::main_integration_valid_from_authorities;
        let _invalid_result_constructor: fn(
            MainIntegrationVerificationObservationAuthority,
        ) -> Result<
            ValidatedMainIntegrationInvalidAuthority,
            Box<MainIntegrationVerificationBlockedAuthority>,
        > = MergeVerificationData::main_integration_invalid_from_authorities;
    }

    assert_not_clone!(ObservedConsumedSupportGateAuthority);
    assert_not_clone!(ConsumedSupportGateObservationRequest<'static>);
    assert_not_clone!(ConsumedSupportGateObservationInvocationCapability);
    assert_not_clone!(ConsumedSupportGateObservationCompletionCapability);
    assert_not_clone!(ConsumedSupportGateStateResolution);
    assert_not_clone!(ResolvedPendingConsumedSupportGateAuthority);
    assert_not_clone!(ResolvedReceiptConsumedSupportGateAuthority);
    assert_not_clone!(ResolvedCommitLineageConsumedSupportGateAuthority);
    assert_not_clone!(ConfiguredValidationExecutionAttemptCapability);
    assert_not_clone!(ConfiguredValidationExecutionCompletionCapability);
    assert_not_clone!(ConfiguredValidationReceiptBatchSnapshot);
    assert_not_clone!(MainIntegrationConfiguredValidationExecutionSelectionAuthority);
    assert_not_clone!(MainIntegrationConfiguredValidationExecutionBlockedAuthority);
    assert_not_clone!(MainIntegrationVerificationObservationInputBlockedAuthority);
    assert_not_clone!(MainIntegrationConfiguredValidationReceiptBatchAuthority);
    assert_not_clone!(MainIntegrationVerificationObservationInputAuthority);
    assert_not_clone!(MainIntegrationVerificationLineage);
    assert_not_clone!(MainIntegrationVerificationObservationAuthority);
    assert_not_clone!(MainIntegrationVerifierObservationEvidenceAuthority);
    assert_not_clone!(MainIntegrationVerificationBlockedAuthority);
    assert_not_clone!(ValidatedMainIntegrationVerificationAuthority);
    assert_not_clone!(ValidatedMainIntegrationInvalidAuthority);
    assert_not_clone!(ValidatedConsumedOriginalMergeLineageAuthority);
    assert_not_clone!(ValidatedMainIntegrationCommitLineageAuthority);
    assert_not_serialize!(ObservedConsumedSupportGateAuthority);
    assert_not_serialize!(ConsumedSupportGateObservationRequest<'static>);
    assert_not_serialize!(ConsumedSupportGateObservationInvocationCapability);
    assert_not_serialize!(ConsumedSupportGateObservationCompletionCapability);
    assert_not_serialize!(ConsumedSupportGateStateResolution);
    assert_not_serialize!(ResolvedPendingConsumedSupportGateAuthority);
    assert_not_serialize!(ResolvedReceiptConsumedSupportGateAuthority);
    assert_not_serialize!(ResolvedCommitLineageConsumedSupportGateAuthority);
    assert_not_serialize!(ConfiguredValidationExecutionAttemptCapability);
    assert_not_serialize!(ConfiguredValidationExecutionCompletionCapability);
    assert_not_serialize!(ConfiguredValidationReceiptBatchSnapshot);
    assert_not_serialize!(MainIntegrationConfiguredValidationExecutionSelectionAuthority);
    assert_not_serialize!(MainIntegrationConfiguredValidationExecutionBlockedAuthority);
    assert_not_serialize!(MainIntegrationVerificationObservationInputBlockedAuthority);
    assert_not_serialize!(MainIntegrationConfiguredValidationReceiptBatchAuthority);
    assert_not_serialize!(MainIntegrationVerificationObservationInputAuthority);
    assert_not_serialize!(MainIntegrationVerificationLineage);
    assert_not_serialize!(MainIntegrationVerificationObservationAuthority);
    assert_not_serialize!(MainIntegrationVerifierObservationEvidenceAuthority);
    assert_not_serialize!(MainIntegrationVerificationBlockedAuthority);
    assert_not_serialize!(ValidatedMainIntegrationVerificationAuthority);
    assert_not_serialize!(ValidatedMainIntegrationInvalidAuthority);
    assert_not_serialize!(ValidatedConsumedOriginalMergeLineageAuthority);
    assert_not_serialize!(ValidatedMainIntegrationCommitLineageAuthority);
    assert_not_deserialize_owned!(ObservedConsumedSupportGateAuthority);
    assert_not_deserialize_owned!(ConsumedSupportGateObservationRequest<'static>);
    assert_not_deserialize_owned!(ConsumedSupportGateObservationInvocationCapability);
    assert_not_deserialize_owned!(ConsumedSupportGateObservationCompletionCapability);
    assert_not_deserialize_owned!(ConsumedSupportGateStateResolution);
    assert_not_deserialize_owned!(ResolvedPendingConsumedSupportGateAuthority);
    assert_not_deserialize_owned!(ResolvedReceiptConsumedSupportGateAuthority);
    assert_not_deserialize_owned!(ResolvedCommitLineageConsumedSupportGateAuthority);
    assert_not_deserialize_owned!(ConfiguredValidationExecutionAttemptCapability);
    assert_not_deserialize_owned!(ConfiguredValidationExecutionCompletionCapability);
    assert_not_deserialize_owned!(ConfiguredValidationReceiptBatchSnapshot);
    assert_not_deserialize_owned!(MainIntegrationConfiguredValidationExecutionSelectionAuthority);
    assert_not_deserialize_owned!(MainIntegrationConfiguredValidationExecutionBlockedAuthority);
    assert_not_deserialize_owned!(MainIntegrationVerificationObservationInputBlockedAuthority);
    assert_not_deserialize_owned!(MainIntegrationConfiguredValidationReceiptBatchAuthority);
    assert_not_deserialize_owned!(MainIntegrationVerificationObservationInputAuthority);
    assert_not_deserialize_owned!(MainIntegrationVerificationLineage);
    assert_not_deserialize_owned!(MainIntegrationVerificationObservationAuthority);
    assert_not_deserialize_owned!(MainIntegrationVerifierObservationEvidenceAuthority);
    assert_not_deserialize_owned!(MainIntegrationVerificationBlockedAuthority);
    assert_not_deserialize_owned!(ValidatedMainIntegrationVerificationAuthority);
    assert_not_deserialize_owned!(ValidatedMainIntegrationInvalidAuthority);
    assert_not_deserialize_owned!(ValidatedConsumedOriginalMergeLineageAuthority);
    assert_not_deserialize_owned!(ValidatedMainIntegrationCommitLineageAuthority);
}
