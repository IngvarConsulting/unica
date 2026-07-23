use super::artifacts::ConfigurationIdentity;
use super::instructions::{
    decode_historical_support_conflict_instruction, SupportConflictInstruction,
    SupportCorrectiveInstruction,
};
use super::results::repository::CommitObjectHistoryBinding;
#[cfg(test)]
use super::results::repository::ValidatedCommitObjectAuthority;
use super::scalars::{
    NormalizedUtcInstant, RepositoryIdentityComponent, RepositoryTargetDisplay, RepositoryUsername,
    RepositoryVersion, RequiredNullable,
};
use super::schema::{audit_json_schema, one_of_schema};
use super::status::CurrentDeferredRepositoryAdvanceAuthority;
use super::support::{
    ExternalSupportOwnershipEvidence, SupportHistoryOrderAuthority,
    SupportObservationCorrectiveProjection, SupportPrerequisiteVersionObservation,
    SupportPrerequisiteVersionObservationDigestRecord,
};
use crate::domain::branched_development::canonical_json::{
    canonical_contract_digest, contract_digest_record_sealed, ContractDigestRecord,
};
use crate::domain::branched_development::{
    CapabilityRowId, MetadataObjectId, Sha256Digest, TaskPhase, UnicaId,
};
use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::sync::Arc;

pub(crate) mod lifecycle;
pub(crate) mod update;

#[cfg(test)]
pub(crate) use lifecycle::empty_commit_history_evidence_fixture_test_only;
#[allow(unused_imports)]
pub(crate) use lifecycle::{
    ClassifiedDeferredRepositoryAdvance, CoverageUnknownDeferredRepositoryAdvance,
    DeferredRepositoryAdvance, DeferredRepositoryAdvanceClassification,
    DeferredRepositoryAdvanceClassificationAuthority, DeferredRepositoryAdvanceConsumptionReceipt,
    DeferredRepositoryAdvanceMissingEvidenceAuthority, OriginalCleanRefreshProof,
    OriginalCleanRefreshScanAuthority, PostMergeHistoryGuardAuthority,
    PostMergeHistoryGuardEvidence, RoutineUpdatePhaseAuthority, SupportGateHistoryEvidence,
    SupportGateRelevantBaselineAuthority, UnclassifiedDeferredRepositoryAdvance,
    UnvalidatedDeferredRepositoryAdvance, UnvalidatedOriginalCleanRefreshProof,
};
#[allow(unused_imports)]
pub(crate) use update::{
    RoutineSelectiveRepositoryUpdateCapabilityToken, RoutineSelectiveRepositoryUpdatePlanAuthority,
    SelectiveRepositoryUpdateExecutionAuthority, SelectiveRepositoryUpdatePlan,
    SelectiveRepositoryUpdatePlanAuthority, SelectiveRepositoryUpdateProof,
    SelectiveRepositoryUpdateScope, SupportRecoverySelectiveUpdateEffectObservation,
    SupportRecoverySelectiveUpdateExecutionObservation,
    SupportRecoverySelectiveUpdatePlanObservation,
    SupportRootSelectiveRepositoryUpdateCapabilityToken,
    SupportRootSelectiveRepositoryUpdatePlanAuthority,
};

const CANONICAL_EMPTY_DELTA_DIGEST: &str =
    "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945";
const MAX_METADATA_ITEMS: usize = 100_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RepositoryContractError(&'static str);

impl fmt::Display for RepositoryContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

impl std::error::Error for RepositoryContractError {}

macro_rules! literal_bool {
    ($name:ident, $value:literal) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        struct $name;

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                serializer.serialize_bool($value)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let observed = bool::deserialize(deserializer)?;
                (observed == $value)
                    .then_some(Self)
                    .ok_or_else(|| D::Error::custom(concat!("expected literal ", stringify!($value))))
            }
        }

        impl JsonSchema for $name {
            fn inline_schema() -> bool {
                true
            }

            fn schema_name() -> Cow<'static, str> {
                stringify!($name).into()
            }

            fn json_schema(_: &mut SchemaGenerator) -> Schema {
                json_schema!({ "type": "boolean", "const": $value })
            }
        }
    };
}

literal_bool!(TrueLiteral, true);
literal_bool!(FalseLiteral, false);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) enum RepositoryRelevance {
    Unrelated,
    Relevant,
}

impl RepositoryRelevance {
    fn parse(value: &str) -> Result<Self, RepositoryContractError> {
        match value {
            "unrelated" => Ok(Self::Unrelated),
            "relevant" => Ok(Self::Relevant),
            _ => Err(RepositoryContractError("invalid repository relevance")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct CanonicalEmptyDeltaDigest;

impl CanonicalEmptyDeltaDigest {
    pub(crate) const VALUE: &'static str = CANONICAL_EMPTY_DELTA_DIGEST;

    pub(crate) const fn as_str(&self) -> &'static str {
        Self::VALUE
    }
}

impl Serialize for CanonicalEmptyDeltaDigest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(Self::VALUE)
    }
}

impl<'de> Deserialize<'de> for CanonicalEmptyDeltaDigest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let observed = String::deserialize(deserializer)?;
        (observed == Self::VALUE)
            .then_some(Self)
            .ok_or_else(|| D::Error::custom("expected the canonical empty-delta digest"))
    }
}

impl JsonSchema for CanonicalEmptyDeltaDigest {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        "CanonicalEmptyDeltaDigest".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({ "type": "string", "const": CANONICAL_EMPTY_DELTA_DIGEST })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RepositoryHistoryCursor {
    through_version: RepositoryVersion,
    history_prefix_digest: Sha256Digest,
}

impl RepositoryHistoryCursor {
    pub(crate) fn new(
        through_version: RepositoryVersion,
        history_prefix_digest: Sha256Digest,
    ) -> Self {
        Self {
            through_version,
            history_prefix_digest,
        }
    }

    pub(crate) const fn through_version(&self) -> &RepositoryVersion {
        &self.through_version
    }

    pub(crate) const fn history_prefix_digest(&self) -> &Sha256Digest {
        &self.history_prefix_digest
    }
}

/// One inseparable repository observation projected by the repository adapter.
///
/// The authority is intentionally non-`Clone`, non-`Deserialize`, and has no
/// production raw-field constructor. A repository adapter added with the
/// execution handlers must mint it from one verified observation rather than
/// accepting independently supplied digest fields.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RepositoryAnchorObservationAuthority {
    repository_identity: Sha256Digest,
    history_cursor: RepositoryHistoryCursor,
    configuration_identity: ConfigurationIdentity,
    configuration_fingerprint: Sha256Digest,
}

impl RepositoryAnchorObservationAuthority {
    #[cfg(test)]
    pub(crate) fn test_only(
        repository_identity: Sha256Digest,
        history_cursor: RepositoryHistoryCursor,
        configuration_identity: ConfigurationIdentity,
        configuration_fingerprint: Sha256Digest,
    ) -> Self {
        Self {
            repository_identity,
            history_cursor,
            configuration_identity,
            configuration_fingerprint,
        }
    }

    pub(crate) fn into_anchor(self) -> Result<RepositoryAnchor, RepositoryContractError> {
        RepositoryAnchor::from_observation(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RepositoryAnchorDigestRecord {
    repository_identity: Sha256Digest,
    history_cursor: RepositoryHistoryCursor,
    configuration_identity: ConfigurationIdentity,
    configuration_fingerprint: Sha256Digest,
}

impl RepositoryAnchorDigestRecord {
    fn from_observation(authority: RepositoryAnchorObservationAuthority) -> Self {
        Self {
            repository_identity: authority.repository_identity,
            history_cursor: authority.history_cursor,
            configuration_identity: authority.configuration_identity,
            configuration_fingerprint: authority.configuration_fingerprint,
        }
    }
}

impl contract_digest_record_sealed::Sealed for RepositoryAnchorDigestRecord {}
impl ContractDigestRecord for RepositoryAnchorDigestRecord {}

/// Content-bound repository observation. Deliberately not `Deserialize`;
/// callers cannot inject an `anchorDigest` without recomputation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RepositoryAnchor {
    repository_identity: Sha256Digest,
    history_cursor: RepositoryHistoryCursor,
    configuration_identity: ConfigurationIdentity,
    configuration_fingerprint: Sha256Digest,
    anchor_digest: Sha256Digest,
}

impl RepositoryAnchor {
    fn from_observation(
        authority: RepositoryAnchorObservationAuthority,
    ) -> Result<Self, RepositoryContractError> {
        let record = RepositoryAnchorDigestRecord::from_observation(authority);
        let anchor_digest = canonical_contract_digest(&record, None)
            .map_err(|_| RepositoryContractError("repository anchor digest failed"))?;
        Ok(Self {
            repository_identity: record.repository_identity,
            history_cursor: record.history_cursor,
            configuration_identity: record.configuration_identity,
            configuration_fingerprint: record.configuration_fingerprint,
            anchor_digest,
        })
    }

    /// Internal constructor used only behind request-bound repository adapter
    /// scopes. Adapter-facing code must expose a scoped request method instead
    /// of accepting a freely assembled anchor.
    pub(super) fn from_guarded_observation(
        repository_identity: Sha256Digest,
        history_cursor: RepositoryHistoryCursor,
        configuration_identity: ConfigurationIdentity,
        configuration_fingerprint: Sha256Digest,
    ) -> Result<Self, RepositoryContractError> {
        Self::from_observation(RepositoryAnchorObservationAuthority {
            repository_identity,
            history_cursor,
            configuration_identity,
            configuration_fingerprint,
        })
    }

    pub(crate) const fn repository_identity(&self) -> &Sha256Digest {
        &self.repository_identity
    }

    pub(crate) const fn history_cursor(&self) -> &RepositoryHistoryCursor {
        &self.history_cursor
    }

    pub(crate) const fn configuration_identity(&self) -> &ConfigurationIdentity {
        &self.configuration_identity
    }

    pub(crate) const fn configuration_fingerprint(&self) -> &Sha256Digest {
        &self.configuration_fingerprint
    }

    pub(crate) const fn anchor_digest(&self) -> &Sha256Digest {
        &self.anchor_digest
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RepositoryActorIdentity {
    username: RepositoryUsername,
    #[serde(deserialize_with = "RequiredNullable::deserialize_required")]
    computer: RequiredNullable<RepositoryIdentityComponent>,
    #[serde(deserialize_with = "RequiredNullable::deserialize_required")]
    infobase: RequiredNullable<RepositoryIdentityComponent>,
}

impl RepositoryActorIdentity {
    pub(crate) const fn username(&self) -> &RepositoryUsername {
        &self.username
    }

    pub(crate) const fn computer(&self) -> Option<&RepositoryIdentityComponent> {
        self.computer.as_ref()
    }

    pub(crate) const fn infobase(&self) -> Option<&RepositoryIdentityComponent> {
        self.infobase.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RepositoryOwnerIdentity {
    username: RepositoryUsername,
    #[serde(deserialize_with = "RequiredNullable::deserialize_required")]
    computer: RequiredNullable<RepositoryIdentityComponent>,
    #[serde(deserialize_with = "RequiredNullable::deserialize_required")]
    infobase: RequiredNullable<RepositoryIdentityComponent>,
    #[serde(deserialize_with = "RequiredNullable::deserialize_required")]
    locked_at: RequiredNullable<NormalizedUtcInstant>,
}

impl RepositoryOwnerIdentity {
    pub(crate) const fn username(&self) -> &RepositoryUsername {
        &self.username
    }

    pub(crate) const fn computer(&self) -> Option<&RepositoryIdentityComponent> {
        self.computer.as_ref()
    }

    pub(crate) const fn infobase(&self) -> Option<&RepositoryIdentityComponent> {
        self.infobase.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RoutineRepositoryVersionClassificationEvidenceDigestRecord {
    repository_version: RepositoryVersion,
    relevance: RepositoryRelevance,
    #[serde(deserialize_with = "RequiredNullable::deserialize_required")]
    repository_actor: RequiredNullable<RepositoryActorIdentity>,
    root_delta_digest: Sha256Digest,
    content_delta_digest: Sha256Digest,
    support_transitions_digest: CanonicalEmptyDeltaDigest,
    support_graph_unchanged: TrueLiteral,
}

impl contract_digest_record_sealed::Sealed
    for RoutineRepositoryVersionClassificationEvidenceDigestRecord
{
}
impl ContractDigestRecord for RoutineRepositoryVersionClassificationEvidenceDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RoutineRepositoryVersionClassificationEvidence {
    repository_version: RepositoryVersion,
    relevance: RepositoryRelevance,
    #[serde(deserialize_with = "RequiredNullable::deserialize_required")]
    repository_actor: RequiredNullable<RepositoryActorIdentity>,
    root_delta_digest: Sha256Digest,
    content_delta_digest: Sha256Digest,
    support_transitions_digest: CanonicalEmptyDeltaDigest,
    support_graph_unchanged: TrueLiteral,
    classification_digest: Sha256Digest,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UncheckedRoutineRepositoryVersionClassificationEvidence {
    repository_version: RepositoryVersion,
    relevance: RepositoryRelevance,
    #[serde(deserialize_with = "RequiredNullable::deserialize_required")]
    repository_actor: RequiredNullable<RepositoryActorIdentity>,
    root_delta_digest: Sha256Digest,
    content_delta_digest: Sha256Digest,
    support_transitions_digest: CanonicalEmptyDeltaDigest,
    support_graph_unchanged: TrueLiteral,
    classification_digest: Sha256Digest,
}

impl RoutineRepositoryVersionClassificationEvidence {
    pub(crate) fn new(
        repository_version: &str,
        relevance: &str,
        repository_actor: Option<RepositoryActorIdentity>,
        root_delta_digest: &str,
        content_delta_digest: &str,
    ) -> Result<Self, RepositoryContractError> {
        let record = RoutineRepositoryVersionClassificationEvidenceDigestRecord {
            repository_version: RepositoryVersion::parse(repository_version)
                .map_err(|_| RepositoryContractError("invalid repository version"))?,
            relevance: RepositoryRelevance::parse(relevance)?,
            repository_actor: repository_actor
                .map(RequiredNullable::value)
                .unwrap_or_else(RequiredNullable::null),
            root_delta_digest: Sha256Digest::parse(root_delta_digest)
                .map_err(|_| RepositoryContractError("invalid root delta digest"))?,
            content_delta_digest: Sha256Digest::parse(content_delta_digest)
                .map_err(|_| RepositoryContractError("invalid content delta digest"))?,
            support_transitions_digest: CanonicalEmptyDeltaDigest,
            support_graph_unchanged: TrueLiteral,
        };
        let classification_digest = canonical_contract_digest(&record, None)
            .map_err(|_| RepositoryContractError("routine evidence digest failed"))?;
        Ok(Self::from_record(record, classification_digest))
    }

    fn from_record(
        record: RoutineRepositoryVersionClassificationEvidenceDigestRecord,
        classification_digest: Sha256Digest,
    ) -> Self {
        Self {
            repository_version: record.repository_version,
            relevance: record.relevance,
            repository_actor: record.repository_actor,
            root_delta_digest: record.root_delta_digest,
            content_delta_digest: record.content_delta_digest,
            support_transitions_digest: record.support_transitions_digest,
            support_graph_unchanged: record.support_graph_unchanged,
            classification_digest,
        }
    }

    fn digest_record(&self) -> RoutineRepositoryVersionClassificationEvidenceDigestRecord {
        RoutineRepositoryVersionClassificationEvidenceDigestRecord {
            repository_version: self.repository_version.clone(),
            relevance: self.relevance,
            repository_actor: self.repository_actor.clone(),
            root_delta_digest: self.root_delta_digest.clone(),
            content_delta_digest: self.content_delta_digest.clone(),
            support_transitions_digest: self.support_transitions_digest,
            support_graph_unchanged: self.support_graph_unchanged,
        }
    }

    fn validate_digest(&self) -> Result<(), RepositoryContractError> {
        let expected = canonical_contract_digest(&self.digest_record(), None)
            .map_err(|_| RepositoryContractError("routine evidence digest failed"))?;
        (expected == self.classification_digest)
            .then_some(())
            .ok_or(RepositoryContractError("routine evidence digest mismatch"))
    }
}

impl<'de> Deserialize<'de> for RoutineRepositoryVersionClassificationEvidence {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let unchecked =
            UncheckedRoutineRepositoryVersionClassificationEvidence::deserialize(deserializer)?;
        let value = Self {
            repository_version: unchecked.repository_version,
            relevance: unchecked.relevance,
            repository_actor: unchecked.repository_actor,
            root_delta_digest: unchecked.root_delta_digest,
            content_delta_digest: unchecked.content_delta_digest,
            support_transitions_digest: unchecked.support_transitions_digest,
            support_graph_unchanged: unchecked.support_graph_unchanged,
            classification_digest: unchecked.classification_digest,
        };
        value.validate_digest().map_err(D::Error::custom)?;
        Ok(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
enum NonConflictingConcurrentReason {
    #[serde(rename = "harmlessNonBlockingReferenceExpansion")]
    HarmlessNonBlockingReferenceExpansion,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NonConflictingConcurrentEvidenceDigestRecord {
    repository_version: RepositoryVersion,
    reason: NonConflictingConcurrentReason,
    atomic_commit_safety_capability_id: CapabilityRowId,
    locked_target_set_digest: Sha256Digest,
    changed_object_set_digest: Sha256Digest,
    before_reference_closure_digest: Sha256Digest,
    after_reference_closure_digest: Sha256Digest,
    added_reference_edge_set_digest: Sha256Digest,
    closure_delta_only_adds_non_blocking_references: TrueLiteral,
    disjoint_from_integration_content: TrueLiteral,
    support_graph_unchanged: TrueLiteral,
    validation_inputs_unaffected: TrueLiteral,
    root_unchanged: TrueLiteral,
    locked_targets_unchanged: TrueLiteral,
    blocks_approved_deletion: FalseLiteral,
}

impl contract_digest_record_sealed::Sealed for NonConflictingConcurrentEvidenceDigestRecord {}
impl ContractDigestRecord for NonConflictingConcurrentEvidenceDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct NonConflictingConcurrentEvidence {
    repository_version: RepositoryVersion,
    reason: NonConflictingConcurrentReason,
    atomic_commit_safety_capability_id: CapabilityRowId,
    locked_target_set_digest: Sha256Digest,
    changed_object_set_digest: Sha256Digest,
    before_reference_closure_digest: Sha256Digest,
    after_reference_closure_digest: Sha256Digest,
    added_reference_edge_set_digest: Sha256Digest,
    closure_delta_only_adds_non_blocking_references: TrueLiteral,
    disjoint_from_integration_content: TrueLiteral,
    support_graph_unchanged: TrueLiteral,
    validation_inputs_unaffected: TrueLiteral,
    root_unchanged: TrueLiteral,
    locked_targets_unchanged: TrueLiteral,
    blocks_approved_deletion: FalseLiteral,
    evidence_digest: Sha256Digest,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UnvalidatedNonConflictingConcurrentEvidence {
    repository_version: RepositoryVersion,
    reason: NonConflictingConcurrentReason,
    atomic_commit_safety_capability_id: CapabilityRowId,
    locked_target_set_digest: Sha256Digest,
    changed_object_set_digest: Sha256Digest,
    before_reference_closure_digest: Sha256Digest,
    after_reference_closure_digest: Sha256Digest,
    added_reference_edge_set_digest: Sha256Digest,
    closure_delta_only_adds_non_blocking_references: TrueLiteral,
    disjoint_from_integration_content: TrueLiteral,
    support_graph_unchanged: TrueLiteral,
    validation_inputs_unaffected: TrueLiteral,
    root_unchanged: TrueLiteral,
    locked_targets_unchanged: TrueLiteral,
    blocks_approved_deletion: FalseLiteral,
    evidence_digest: Sha256Digest,
}

impl UnvalidatedNonConflictingConcurrentEvidence {
    fn into_audited_evidence(
        self,
    ) -> Result<NonConflictingConcurrentEvidence, RepositoryContractError> {
        let evidence = NonConflictingConcurrentEvidence {
            repository_version: self.repository_version,
            reason: self.reason,
            atomic_commit_safety_capability_id: self.atomic_commit_safety_capability_id,
            locked_target_set_digest: self.locked_target_set_digest,
            changed_object_set_digest: self.changed_object_set_digest,
            before_reference_closure_digest: self.before_reference_closure_digest,
            after_reference_closure_digest: self.after_reference_closure_digest,
            added_reference_edge_set_digest: self.added_reference_edge_set_digest,
            closure_delta_only_adds_non_blocking_references: self
                .closure_delta_only_adds_non_blocking_references,
            disjoint_from_integration_content: self.disjoint_from_integration_content,
            support_graph_unchanged: self.support_graph_unchanged,
            validation_inputs_unaffected: self.validation_inputs_unaffected,
            root_unchanged: self.root_unchanged,
            locked_targets_unchanged: self.locked_targets_unchanged,
            blocks_approved_deletion: self.blocks_approved_deletion,
            evidence_digest: self.evidence_digest,
        };
        evidence.validate_digest()?;
        Ok(evidence)
    }
}

impl NonConflictingConcurrentEvidence {
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        repository_version: &str,
        atomic_commit_safety_capability_id: &str,
        locked_target_set_digest: &str,
        changed_object_set_digest: &str,
        before_reference_closure_digest: &str,
        after_reference_closure_digest: &str,
        added_reference_edge_set_digest: &str,
    ) -> Result<Self, RepositoryContractError> {
        let record = NonConflictingConcurrentEvidenceDigestRecord {
            repository_version: RepositoryVersion::parse(repository_version)
                .map_err(|_| RepositoryContractError("invalid repository version"))?,
            reason: NonConflictingConcurrentReason::HarmlessNonBlockingReferenceExpansion,
            atomic_commit_safety_capability_id: CapabilityRowId::parse(
                atomic_commit_safety_capability_id,
            )
            .map_err(|_| RepositoryContractError("invalid atomic safety capability"))?,
            locked_target_set_digest: parse_digest(locked_target_set_digest)?,
            changed_object_set_digest: parse_digest(changed_object_set_digest)?,
            before_reference_closure_digest: parse_digest(before_reference_closure_digest)?,
            after_reference_closure_digest: parse_digest(after_reference_closure_digest)?,
            added_reference_edge_set_digest: parse_digest(added_reference_edge_set_digest)?,
            closure_delta_only_adds_non_blocking_references: TrueLiteral,
            disjoint_from_integration_content: TrueLiteral,
            support_graph_unchanged: TrueLiteral,
            validation_inputs_unaffected: TrueLiteral,
            root_unchanged: TrueLiteral,
            locked_targets_unchanged: TrueLiteral,
            blocks_approved_deletion: FalseLiteral,
        };
        let evidence_digest = canonical_contract_digest(&record, None)
            .map_err(|_| RepositoryContractError("concurrent evidence digest failed"))?;
        Ok(Self::from_record(record, evidence_digest))
    }

    #[cfg(test)]
    fn from_record(
        record: NonConflictingConcurrentEvidenceDigestRecord,
        evidence_digest: Sha256Digest,
    ) -> Self {
        Self {
            repository_version: record.repository_version,
            reason: record.reason,
            atomic_commit_safety_capability_id: record.atomic_commit_safety_capability_id,
            locked_target_set_digest: record.locked_target_set_digest,
            changed_object_set_digest: record.changed_object_set_digest,
            before_reference_closure_digest: record.before_reference_closure_digest,
            after_reference_closure_digest: record.after_reference_closure_digest,
            added_reference_edge_set_digest: record.added_reference_edge_set_digest,
            closure_delta_only_adds_non_blocking_references: record
                .closure_delta_only_adds_non_blocking_references,
            disjoint_from_integration_content: record.disjoint_from_integration_content,
            support_graph_unchanged: record.support_graph_unchanged,
            validation_inputs_unaffected: record.validation_inputs_unaffected,
            root_unchanged: record.root_unchanged,
            locked_targets_unchanged: record.locked_targets_unchanged,
            blocks_approved_deletion: record.blocks_approved_deletion,
            evidence_digest,
        }
    }

    pub(crate) const fn repository_version(&self) -> &RepositoryVersion {
        &self.repository_version
    }

    pub(crate) const fn evidence_digest(&self) -> &Sha256Digest {
        &self.evidence_digest
    }

    fn digest_record(&self) -> NonConflictingConcurrentEvidenceDigestRecord {
        NonConflictingConcurrentEvidenceDigestRecord {
            repository_version: self.repository_version.clone(),
            reason: self.reason,
            atomic_commit_safety_capability_id: self.atomic_commit_safety_capability_id.clone(),
            locked_target_set_digest: self.locked_target_set_digest.clone(),
            changed_object_set_digest: self.changed_object_set_digest.clone(),
            before_reference_closure_digest: self.before_reference_closure_digest.clone(),
            after_reference_closure_digest: self.after_reference_closure_digest.clone(),
            added_reference_edge_set_digest: self.added_reference_edge_set_digest.clone(),
            closure_delta_only_adds_non_blocking_references: self
                .closure_delta_only_adds_non_blocking_references,
            disjoint_from_integration_content: self.disjoint_from_integration_content,
            support_graph_unchanged: self.support_graph_unchanged,
            validation_inputs_unaffected: self.validation_inputs_unaffected,
            root_unchanged: self.root_unchanged,
            locked_targets_unchanged: self.locked_targets_unchanged,
            blocks_approved_deletion: self.blocks_approved_deletion,
        }
    }

    fn validate_digest(&self) -> Result<(), RepositoryContractError> {
        let expected = canonical_contract_digest(&self.digest_record(), None)
            .map_err(|_| RepositoryContractError("concurrent evidence digest failed"))?;
        (expected == self.evidence_digest)
            .then_some(())
            .ok_or(RepositoryContractError(
                "concurrent evidence digest mismatch",
            ))
    }
}

fn deserialize_audited_non_conflicting_concurrent_evidence<'de, D>(
    deserializer: D,
) -> Result<NonConflictingConcurrentEvidence, D::Error>
where
    D: Deserializer<'de>,
{
    UnvalidatedNonConflictingConcurrentEvidence::deserialize(deserializer)?
        .into_audited_evidence()
        .map_err(D::Error::custom)
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub(crate) enum EvidenceKind {
    RoutineClassification,
    SupportPrerequisiteObservation,
    NonConflictingConcurrent,
}

impl EvidenceKind {
    const fn declaration_ordinal(self) -> usize {
        match self {
            Self::RoutineClassification => 0,
            Self::SupportPrerequisiteObservation => 1,
            Self::NonConflictingConcurrent => 2,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct EvidenceSourceRegistryEntry {
    evidence_kind: EvidenceKind,
    evidence_schema_digest: Sha256Digest,
    digest_record_schema_digest: Sha256Digest,
    loader_revision_digest: Sha256Digest,
    classification_mapper_revision_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EvidenceSourceRegistryDigestRecord {
    entries: Task8EvidenceSourceRegistryEntries,
}

impl contract_digest_record_sealed::Sealed for EvidenceSourceRegistryDigestRecord {}
impl ContractDigestRecord for EvidenceSourceRegistryDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct Task8EvidenceSourceRegistryEntries([EvidenceSourceRegistryEntry; 3]);

impl JsonSchema for Task8EvidenceSourceRegistryEntries {
    fn schema_name() -> Cow<'static, str> {
        "Task8EvidenceSourceRegistryEntries".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "prefixItems": [
                registry_entry_schema(EvidenceKind::RoutineClassification, generator),
                registry_entry_schema(EvidenceKind::SupportPrerequisiteObservation, generator),
                registry_entry_schema(EvidenceKind::NonConflictingConcurrent, generator),
            ],
            "items": false,
            "minItems": 3,
            "maxItems": 3,
        })
    }
}

fn registry_entry_schema(kind: EvidenceKind, generator: &mut SchemaGenerator) -> Schema {
    let kind = match kind {
        EvidenceKind::RoutineClassification => "routineClassification",
        EvidenceKind::SupportPrerequisiteObservation => "supportPrerequisiteObservation",
        EvidenceKind::NonConflictingConcurrent => "nonConflictingConcurrent",
    };
    let digest = generator.subschema_for::<Sha256Digest>();
    json_schema!({
        "type": "object",
        "properties": {
            "evidenceKind": { "type": "string", "const": kind },
            "evidenceSchemaDigest": digest.clone(),
            "digestRecordSchemaDigest": digest.clone(),
            "loaderRevisionDigest": digest.clone(),
            "classificationMapperRevisionDigest": digest,
        },
        "required": [
            "evidenceKind",
            "evidenceSchemaDigest",
            "digestRecordSchemaDigest",
            "loaderRevisionDigest",
            "classificationMapperRevisionDigest",
        ],
        "additionalProperties": false,
    })
}

#[derive(Serialize)]
#[serde(transparent)]
struct StandaloneSchemaDigestRecord(schemars::Schema);

impl contract_digest_record_sealed::Sealed for StandaloneSchemaDigestRecord {}
impl ContractDigestRecord for StandaloneSchemaDigestRecord {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
enum EvidenceLoaderKind {
    #[serde(rename = "contentAddressedTypedEvidence")]
    ContentAddressedTypedEvidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
enum EvidenceLoaderValidationCheck {
    LookupByEvidenceKindAndDigest,
    RequireSingleRecord,
    StrictTypedDecode,
    RequireCanonicalIJson,
    ProjectNamedDigestRecord,
    RecomputeAndMatchDigest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct EvidenceLoaderValidationChecks([EvidenceLoaderValidationCheck; 6]);

impl EvidenceLoaderValidationChecks {
    const fn canonical() -> Self {
        Self([
            EvidenceLoaderValidationCheck::LookupByEvidenceKindAndDigest,
            EvidenceLoaderValidationCheck::RequireSingleRecord,
            EvidenceLoaderValidationCheck::StrictTypedDecode,
            EvidenceLoaderValidationCheck::RequireCanonicalIJson,
            EvidenceLoaderValidationCheck::ProjectNamedDigestRecord,
            EvidenceLoaderValidationCheck::RecomputeAndMatchDigest,
        ])
    }
}

impl JsonSchema for EvidenceLoaderValidationChecks {
    fn schema_name() -> Cow<'static, str> {
        "EvidenceLoaderValidationChecks".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        let wires = [
            "lookupByEvidenceKindAndDigest",
            "requireSingleRecord",
            "strictTypedDecode",
            "requireCanonicalIJson",
            "projectNamedDigestRecord",
            "recomputeAndMatchDigest",
        ];
        let prefix_items: Vec<_> = wires
            .iter()
            .map(|wire| json_schema!({ "type": "string", "const": wire }))
            .collect();
        json_schema!({
            "type": "array",
            "prefixItems": prefix_items,
            "items": false,
            "minItems": 6,
            "maxItems": 6,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EvidenceLoaderRevisionDigestRecord {
    loader_kind: EvidenceLoaderKind,
    evidence_kind: EvidenceKind,
    validation_checks: EvidenceLoaderValidationChecks,
}

impl contract_digest_record_sealed::Sealed for EvidenceLoaderRevisionDigestRecord {}
impl ContractDigestRecord for EvidenceLoaderRevisionDigestRecord {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
enum EvidenceMapperKind {
    #[serde(rename = "repositoryHistoryPartitionClassification")]
    RepositoryHistoryPartitionClassification,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
enum EvidenceMapperValidationCheck {
    #[serde(rename = "repositoryVersionMatch")]
    RepositoryVersion,
    #[serde(rename = "sourceClassificationMatch")]
    SourceClassification,
    #[serde(rename = "semanticDeltaProjectionMatch")]
    SemanticDeltaProjection,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct EvidenceMapperValidationChecks([EvidenceMapperValidationCheck; 3]);

impl EvidenceMapperValidationChecks {
    const fn canonical() -> Self {
        Self([
            EvidenceMapperValidationCheck::RepositoryVersion,
            EvidenceMapperValidationCheck::SourceClassification,
            EvidenceMapperValidationCheck::SemanticDeltaProjection,
        ])
    }
}

impl JsonSchema for EvidenceMapperValidationChecks {
    fn schema_name() -> Cow<'static, str> {
        "EvidenceMapperValidationChecks".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        let wires = [
            "repositoryVersionMatch",
            "sourceClassificationMatch",
            "semanticDeltaProjectionMatch",
        ];
        let prefix_items: Vec<_> = wires
            .iter()
            .map(|wire| json_schema!({ "type": "string", "const": wire }))
            .collect();
        json_schema!({
            "type": "array",
            "prefixItems": prefix_items,
            "items": false,
            "minItems": 3,
            "maxItems": 3,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
enum SemanticDigestProjection {
    CopyRootDeltaDigest,
    CopyContentDeltaDigest,
    CopyClassificationDigest,
    CopyExternalSupportDisjointnessDigest,
    CopyCorrectiveInstructionDigest,
    CopySupportConflictInstructionDigest,
    ExplicitNull,
    CopyEvidenceDigest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
enum MapperSourceCase {
    Unrelated,
    Relevant,
    RoutineUnrelated,
    RoutineRelevant,
    Authorized,
    ExternalSupport,
    PreArmExternal,
    ActionCorrection,
    ExternalConflictCorrection,
    Invalid,
    HarmlessNonBlockingReferenceExpansion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) enum RepositoryHistoryPartitionClassification {
    UnrelatedRoutine,
    RelevantRoutine,
    AuthorizedSupport,
    ExternalSupport,
    PreArmExternal,
    Invalid,
    Corrective,
    NonConflictingConcurrent,
    TaskCommit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EvidenceClassificationMappingRow {
    source_case: MapperSourceCase,
    partition_classification: RepositoryHistoryPartitionClassification,
    root_delta_digest_projection: SemanticDigestProjection,
    content_delta_digest_projection: SemanticDigestProjection,
    classification_digest_projection: SemanticDigestProjection,
    external_support_disjointness_digest_projection: SemanticDigestProjection,
    corrective_instruction_digest_projection: SemanticDigestProjection,
    non_conflicting_concurrent_evidence_digest_projection: SemanticDigestProjection,
}

impl EvidenceClassificationMappingRow {
    const fn routine(
        source_case: MapperSourceCase,
        partition_classification: RepositoryHistoryPartitionClassification,
    ) -> Self {
        Self {
            source_case,
            partition_classification,
            root_delta_digest_projection: SemanticDigestProjection::CopyRootDeltaDigest,
            content_delta_digest_projection: SemanticDigestProjection::CopyContentDeltaDigest,
            classification_digest_projection: SemanticDigestProjection::CopyClassificationDigest,
            external_support_disjointness_digest_projection: SemanticDigestProjection::ExplicitNull,
            corrective_instruction_digest_projection: SemanticDigestProjection::ExplicitNull,
            non_conflicting_concurrent_evidence_digest_projection:
                SemanticDigestProjection::ExplicitNull,
        }
    }

    const fn non_conflicting() -> Self {
        Self {
            source_case: MapperSourceCase::HarmlessNonBlockingReferenceExpansion,
            partition_classification:
                RepositoryHistoryPartitionClassification::NonConflictingConcurrent,
            root_delta_digest_projection: SemanticDigestProjection::ExplicitNull,
            content_delta_digest_projection: SemanticDigestProjection::ExplicitNull,
            classification_digest_projection: SemanticDigestProjection::ExplicitNull,
            external_support_disjointness_digest_projection: SemanticDigestProjection::ExplicitNull,
            corrective_instruction_digest_projection: SemanticDigestProjection::ExplicitNull,
            non_conflicting_concurrent_evidence_digest_projection:
                SemanticDigestProjection::CopyEvidenceDigest,
        }
    }

    const fn support_observation(
        source_case: MapperSourceCase,
        partition_classification: RepositoryHistoryPartitionClassification,
        external_support_disjointness_digest_projection: SemanticDigestProjection,
    ) -> Self {
        Self {
            source_case,
            partition_classification,
            root_delta_digest_projection: SemanticDigestProjection::CopyRootDeltaDigest,
            content_delta_digest_projection: SemanticDigestProjection::CopyContentDeltaDigest,
            classification_digest_projection: SemanticDigestProjection::CopyClassificationDigest,
            external_support_disjointness_digest_projection,
            corrective_instruction_digest_projection: SemanticDigestProjection::ExplicitNull,
            non_conflicting_concurrent_evidence_digest_projection:
                SemanticDigestProjection::ExplicitNull,
        }
    }

    const fn support_correction(
        source_case: MapperSourceCase,
        corrective_instruction_digest_projection: SemanticDigestProjection,
    ) -> Self {
        Self {
            source_case,
            partition_classification: RepositoryHistoryPartitionClassification::Corrective,
            root_delta_digest_projection: SemanticDigestProjection::CopyRootDeltaDigest,
            content_delta_digest_projection: SemanticDigestProjection::CopyContentDeltaDigest,
            classification_digest_projection: SemanticDigestProjection::CopyClassificationDigest,
            external_support_disjointness_digest_projection: SemanticDigestProjection::ExplicitNull,
            corrective_instruction_digest_projection,
            non_conflicting_concurrent_evidence_digest_projection:
                SemanticDigestProjection::ExplicitNull,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct RoutineEvidenceMappings([EvidenceClassificationMappingRow; 2]);

impl RoutineEvidenceMappings {
    const fn canonical() -> Self {
        Self([
            EvidenceClassificationMappingRow::routine(
                MapperSourceCase::Unrelated,
                RepositoryHistoryPartitionClassification::UnrelatedRoutine,
            ),
            EvidenceClassificationMappingRow::routine(
                MapperSourceCase::Relevant,
                RepositoryHistoryPartitionClassification::RelevantRoutine,
            ),
        ])
    }
}

impl JsonSchema for RoutineEvidenceMappings {
    fn schema_name() -> Cow<'static, str> {
        "RoutineEvidenceMappings".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "prefixItems": [
                mapping_row_schema("unrelated", "unrelatedRoutine", [
                    "copyRootDeltaDigest",
                    "copyContentDeltaDigest",
                    "copyClassificationDigest",
                    "explicitNull",
                    "explicitNull",
                    "explicitNull",
                ]),
                mapping_row_schema("relevant", "relevantRoutine", [
                    "copyRootDeltaDigest",
                    "copyContentDeltaDigest",
                    "copyClassificationDigest",
                    "explicitNull",
                    "explicitNull",
                    "explicitNull",
                ]),
            ],
            "items": false,
            "minItems": 2,
            "maxItems": 2,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct NonConflictingEvidenceMappings([EvidenceClassificationMappingRow; 1]);

impl NonConflictingEvidenceMappings {
    const fn canonical() -> Self {
        Self([EvidenceClassificationMappingRow::non_conflicting()])
    }
}

impl JsonSchema for NonConflictingEvidenceMappings {
    fn schema_name() -> Cow<'static, str> {
        "NonConflictingEvidenceMappings".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "prefixItems": [mapping_row_schema(
                "harmlessNonBlockingReferenceExpansion",
                "nonConflictingConcurrent",
                [
                    "explicitNull",
                    "explicitNull",
                    "explicitNull",
                    "explicitNull",
                    "explicitNull",
                    "copyEvidenceDigest",
                ],
            )],
            "items": false,
            "minItems": 1,
            "maxItems": 1,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct SupportObservationEvidenceMappings([EvidenceClassificationMappingRow; 8]);

impl SupportObservationEvidenceMappings {
    const fn canonical() -> Self {
        Self([
            EvidenceClassificationMappingRow::support_observation(
                MapperSourceCase::RoutineUnrelated,
                RepositoryHistoryPartitionClassification::UnrelatedRoutine,
                SemanticDigestProjection::ExplicitNull,
            ),
            EvidenceClassificationMappingRow::support_observation(
                MapperSourceCase::RoutineRelevant,
                RepositoryHistoryPartitionClassification::RelevantRoutine,
                SemanticDigestProjection::ExplicitNull,
            ),
            EvidenceClassificationMappingRow::support_observation(
                MapperSourceCase::Authorized,
                RepositoryHistoryPartitionClassification::AuthorizedSupport,
                SemanticDigestProjection::ExplicitNull,
            ),
            EvidenceClassificationMappingRow::support_observation(
                MapperSourceCase::ExternalSupport,
                RepositoryHistoryPartitionClassification::ExternalSupport,
                SemanticDigestProjection::CopyExternalSupportDisjointnessDigest,
            ),
            EvidenceClassificationMappingRow::support_observation(
                MapperSourceCase::PreArmExternal,
                RepositoryHistoryPartitionClassification::PreArmExternal,
                SemanticDigestProjection::ExplicitNull,
            ),
            EvidenceClassificationMappingRow::support_correction(
                MapperSourceCase::ActionCorrection,
                SemanticDigestProjection::CopyCorrectiveInstructionDigest,
            ),
            EvidenceClassificationMappingRow::support_correction(
                MapperSourceCase::ExternalConflictCorrection,
                SemanticDigestProjection::CopySupportConflictInstructionDigest,
            ),
            EvidenceClassificationMappingRow::support_observation(
                MapperSourceCase::Invalid,
                RepositoryHistoryPartitionClassification::Invalid,
                SemanticDigestProjection::ExplicitNull,
            ),
        ])
    }
}

impl JsonSchema for SupportObservationEvidenceMappings {
    fn schema_name() -> Cow<'static, str> {
        "SupportObservationEvidenceMappings".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        let ordinary = [
            "copyRootDeltaDigest",
            "copyContentDeltaDigest",
            "copyClassificationDigest",
            "explicitNull",
            "explicitNull",
            "explicitNull",
        ];
        json_schema!({
            "type": "array",
            "prefixItems": [
                mapping_row_schema("routineUnrelated", "unrelatedRoutine", ordinary),
                mapping_row_schema("routineRelevant", "relevantRoutine", ordinary),
                mapping_row_schema("authorized", "authorizedSupport", ordinary),
                mapping_row_schema("externalSupport", "externalSupport", [
                    "copyRootDeltaDigest",
                    "copyContentDeltaDigest",
                    "copyClassificationDigest",
                    "copyExternalSupportDisjointnessDigest",
                    "explicitNull",
                    "explicitNull",
                ]),
                mapping_row_schema("preArmExternal", "preArmExternal", ordinary),
                mapping_row_schema("actionCorrection", "corrective", [
                    "copyRootDeltaDigest",
                    "copyContentDeltaDigest",
                    "copyClassificationDigest",
                    "explicitNull",
                    "copyCorrectiveInstructionDigest",
                    "explicitNull",
                ]),
                mapping_row_schema("externalConflictCorrection", "corrective", [
                    "copyRootDeltaDigest",
                    "copyContentDeltaDigest",
                    "copyClassificationDigest",
                    "explicitNull",
                    "copySupportConflictInstructionDigest",
                    "explicitNull",
                ]),
                mapping_row_schema("invalid", "invalid", ordinary),
            ],
            "items": false,
            "minItems": 8,
            "maxItems": 8,
        })
    }
}

fn mapping_row_schema(
    source_case: &'static str,
    partition_classification: &'static str,
    projections: [&'static str; 6],
) -> Schema {
    json_schema!({
        "type": "object",
        "properties": {
            "sourceCase": { "type": "string", "const": source_case },
            "partitionClassification": {
                "type": "string",
                "const": partition_classification,
            },
            "rootDeltaDigestProjection": { "type": "string", "const": projections[0] },
            "contentDeltaDigestProjection": { "type": "string", "const": projections[1] },
            "classificationDigestProjection": { "type": "string", "const": projections[2] },
            "externalSupportDisjointnessDigestProjection": {
                "type": "string",
                "const": projections[3],
            },
            "correctiveInstructionDigestProjection": {
                "type": "string",
                "const": projections[4],
            },
            "nonConflictingConcurrentEvidenceDigestProjection": {
                "type": "string",
                "const": projections[5],
            },
        },
        "required": [
            "sourceCase",
            "partitionClassification",
            "rootDeltaDigestProjection",
            "contentDeltaDigestProjection",
            "classificationDigestProjection",
            "externalSupportDisjointnessDigestProjection",
            "correctiveInstructionDigestProjection",
            "nonConflictingConcurrentEvidenceDigestProjection",
        ],
        "additionalProperties": false,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RoutineEvidenceClassificationMapperRevisionDigestRecord {
    mapper_kind: EvidenceMapperKind,
    evidence_kind: EvidenceKind,
    validation_checks: EvidenceMapperValidationChecks,
    mappings: RoutineEvidenceMappings,
}

impl contract_digest_record_sealed::Sealed
    for RoutineEvidenceClassificationMapperRevisionDigestRecord
{
}
impl ContractDigestRecord for RoutineEvidenceClassificationMapperRevisionDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NonConflictingEvidenceClassificationMapperRevisionDigestRecord {
    mapper_kind: EvidenceMapperKind,
    evidence_kind: EvidenceKind,
    validation_checks: EvidenceMapperValidationChecks,
    mappings: NonConflictingEvidenceMappings,
}

impl contract_digest_record_sealed::Sealed
    for NonConflictingEvidenceClassificationMapperRevisionDigestRecord
{
}
impl ContractDigestRecord for NonConflictingEvidenceClassificationMapperRevisionDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SupportObservationEvidenceClassificationMapperRevisionDigestRecord {
    mapper_kind: EvidenceMapperKind,
    evidence_kind: EvidenceKind,
    validation_checks: EvidenceMapperValidationChecks,
    mappings: SupportObservationEvidenceMappings,
}

impl contract_digest_record_sealed::Sealed
    for SupportObservationEvidenceClassificationMapperRevisionDigestRecord
{
}
impl ContractDigestRecord for SupportObservationEvidenceClassificationMapperRevisionDigestRecord {}

const TASK7_ROUTINE_EVIDENCE_SCHEMA_DIGEST: &str =
    "f590a096cb42f817f2000d14de97a6dfcb47c6dde569cc916827ed747ded5bc7";
const TASK7_ROUTINE_DIGEST_RECORD_SCHEMA_DIGEST: &str =
    "afbe9c5050546bba77507b90ae35fdb8710a772b6149f6d2012b89eadf584083";
const TASK7_ROUTINE_LOADER_REVISION_DIGEST: &str =
    "26e91a3162b3b03767ea35d2caf3eb826e63e64717ef9f1583585662aa35b90d";
const TASK7_ROUTINE_MAPPER_REVISION_DIGEST: &str =
    "82348174f44e16b35e2b7b002c119108935a4eb4f8ded36b04f31597dccf6617";
const TASK7_NCC_EVIDENCE_SCHEMA_DIGEST: &str =
    "739f8a64f59110610a3325a4593560b3e2617af8701a6975bac9b9891584c1f7";
const TASK7_NCC_DIGEST_RECORD_SCHEMA_DIGEST: &str =
    "cdc45e02e82509a0ab93b9e936d287ee48a1ab389f4bd995c1fbce66b2389ed7";
const TASK7_NCC_LOADER_REVISION_DIGEST: &str =
    "fe36833caada289474fba54b8f5ea13489219557583b7cb51380fe3799613603";
const TASK7_NCC_MAPPER_REVISION_DIGEST: &str =
    "55280fdd689ce29c16185638edc05fafe3a759fadb53e3b0aa0d65a97d150fed";
const TASK7_EVIDENCE_SOURCE_REGISTRY_DIGEST: &str =
    "91c1d9864fe79bc37eded4e1455dead8800e2b85c7bd81888c458accb900fbbc";
const TASK8_SUPPORT_OBSERVATION_EVIDENCE_SCHEMA_DIGEST: &str =
    "7eb7a77b3b57b2cc6236be637ea6333f42bd6e26f5a1713385c6195273366bcd";
const TASK8_SUPPORT_OBSERVATION_DIGEST_RECORD_SCHEMA_DIGEST: &str =
    "499b3e4af9000dd25a8be79188972a27f68d19ba01c1b8b243fafaca3d376126";
const TASK8_SUPPORT_OBSERVATION_LOADER_REVISION_DIGEST: &str =
    "9959dd9df263485c20961a8be2b3c42705f3243e4779f19159e578dd6c2744e8";
const TASK8_SUPPORT_OBSERVATION_MAPPER_REVISION_DIGEST: &str =
    "fd97e2378b0b4125531a6088c99dfbcfe7a9e81ce08e634c1d2dc109225f0a1f";
const TASK8_EVIDENCE_SOURCE_REGISTRY_DIGEST: &str =
    "2cb42be57491f40e046c03e0c92633cd7be5c1853dd01fdb8d21940ad570b4c2";
const TASK9_SUPPORT_OBSERVATION_MAPPER_REVISION_DIGEST: &str =
    "0da7a986536cf8008b5ef53c74f659bec3e79220977915c84d91963c1724f1a6";
const TASK9_EVIDENCE_SOURCE_REGISTRY_DIGEST: &str =
    "c198250d1a992ec1a8b85ae6facddaaa222879de1ded2d721eda1e82432ef6fb";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EvidenceSourceRegistry {
    entries: [EvidenceSourceRegistryEntry; 3],
    registry_digest: Sha256Digest,
}

impl EvidenceSourceRegistry {
    pub(crate) fn task9() -> Result<Self, RepositoryContractError> {
        let entries = [
            Self::entry(EvidenceKind::RoutineClassification)?,
            Self::entry(EvidenceKind::SupportPrerequisiteObservation)?,
            Self::entry(EvidenceKind::NonConflictingConcurrent)?,
        ];
        let registry_digest = canonical_contract_digest(
            &EvidenceSourceRegistryDigestRecord {
                entries: Task8EvidenceSourceRegistryEntries(entries.clone()),
            },
            None,
        )
        .map_err(|_| RepositoryContractError("registry digest failed"))?;
        let registry = Self {
            entries,
            registry_digest,
        };
        registry.verify_committed_artifacts()?;
        Ok(registry)
    }

    #[cfg(test)]
    pub(crate) fn task8() -> Result<Self, RepositoryContractError> {
        Self::task9()
    }

    fn entry(kind: EvidenceKind) -> Result<EvidenceSourceRegistryEntry, RepositoryContractError> {
        let (evidence_schema_digest, digest_record_schema_digest) = match kind {
            EvidenceKind::RoutineClassification => (
                schema_digest::<RoutineRepositoryVersionClassificationEvidence>()?,
                schema_digest::<RoutineRepositoryVersionClassificationEvidenceDigestRecord>()?,
            ),
            EvidenceKind::NonConflictingConcurrent => (
                schema_digest::<NonConflictingConcurrentEvidence>()?,
                schema_digest::<NonConflictingConcurrentEvidenceDigestRecord>()?,
            ),
            EvidenceKind::SupportPrerequisiteObservation => (
                schema_digest::<SupportPrerequisiteVersionObservation>()?,
                schema_digest::<SupportPrerequisiteVersionObservationDigestRecord>()?,
            ),
        };
        let loader_revision_digest = canonical_contract_digest(
            &EvidenceLoaderRevisionDigestRecord {
                loader_kind: EvidenceLoaderKind::ContentAddressedTypedEvidence,
                evidence_kind: kind,
                validation_checks: EvidenceLoaderValidationChecks::canonical(),
            },
            None,
        )
        .map_err(|_| RepositoryContractError("loader revision digest failed"))?;
        let classification_mapper_revision_digest = match kind {
            EvidenceKind::RoutineClassification => canonical_contract_digest(
                &RoutineEvidenceClassificationMapperRevisionDigestRecord {
                    mapper_kind: EvidenceMapperKind::RepositoryHistoryPartitionClassification,
                    evidence_kind: kind,
                    validation_checks: EvidenceMapperValidationChecks::canonical(),
                    mappings: RoutineEvidenceMappings::canonical(),
                },
                None,
            ),
            EvidenceKind::NonConflictingConcurrent => canonical_contract_digest(
                &NonConflictingEvidenceClassificationMapperRevisionDigestRecord {
                    mapper_kind: EvidenceMapperKind::RepositoryHistoryPartitionClassification,
                    evidence_kind: kind,
                    validation_checks: EvidenceMapperValidationChecks::canonical(),
                    mappings: NonConflictingEvidenceMappings::canonical(),
                },
                None,
            ),
            EvidenceKind::SupportPrerequisiteObservation => canonical_contract_digest(
                &SupportObservationEvidenceClassificationMapperRevisionDigestRecord {
                    mapper_kind: EvidenceMapperKind::RepositoryHistoryPartitionClassification,
                    evidence_kind: kind,
                    validation_checks: EvidenceMapperValidationChecks::canonical(),
                    mappings: SupportObservationEvidenceMappings::canonical(),
                },
                None,
            ),
        }
        .map_err(|_| RepositoryContractError("mapper revision digest failed"))?;
        Ok(EvidenceSourceRegistryEntry {
            evidence_kind: kind,
            evidence_schema_digest,
            digest_record_schema_digest,
            loader_revision_digest,
            classification_mapper_revision_digest,
        })
    }

    pub(crate) fn evidence_kinds(&self) -> [EvidenceKind; 3] {
        [
            self.entries[0].evidence_kind,
            self.entries[1].evidence_kind,
            self.entries[2].evidence_kind,
        ]
    }

    pub(crate) fn registry_digest(&self) -> &Sha256Digest {
        &self.registry_digest
    }

    pub(crate) fn verify_committed_artifacts(&self) -> Result<(), RepositoryContractError> {
        let committed = [
            (
                EvidenceKind::RoutineClassification,
                TASK7_ROUTINE_EVIDENCE_SCHEMA_DIGEST,
                TASK7_ROUTINE_DIGEST_RECORD_SCHEMA_DIGEST,
                TASK7_ROUTINE_LOADER_REVISION_DIGEST,
                TASK7_ROUTINE_MAPPER_REVISION_DIGEST,
            ),
            (
                EvidenceKind::SupportPrerequisiteObservation,
                TASK8_SUPPORT_OBSERVATION_EVIDENCE_SCHEMA_DIGEST,
                TASK8_SUPPORT_OBSERVATION_DIGEST_RECORD_SCHEMA_DIGEST,
                TASK8_SUPPORT_OBSERVATION_LOADER_REVISION_DIGEST,
                TASK9_SUPPORT_OBSERVATION_MAPPER_REVISION_DIGEST,
            ),
            (
                EvidenceKind::NonConflictingConcurrent,
                TASK7_NCC_EVIDENCE_SCHEMA_DIGEST,
                TASK7_NCC_DIGEST_RECORD_SCHEMA_DIGEST,
                TASK7_NCC_LOADER_REVISION_DIGEST,
                TASK7_NCC_MAPPER_REVISION_DIGEST,
            ),
        ];
        if self.entries.len() != committed.len() {
            return Err(RepositoryContractError("registry entry count mismatch"));
        }
        for (entry, expected) in self.entries.iter().zip(committed) {
            if entry.evidence_kind != expected.0
                || entry.evidence_schema_digest.as_str() != expected.1
                || entry.digest_record_schema_digest.as_str() != expected.2
                || entry.loader_revision_digest.as_str() != expected.3
                || entry.classification_mapper_revision_digest.as_str() != expected.4
            {
                return Err(RepositoryContractError("registry artifact digest mismatch"));
            }
        }
        if self.registry_digest.as_str() != TASK9_EVIDENCE_SOURCE_REGISTRY_DIGEST {
            return Err(RepositoryContractError("registry digest mismatch"));
        }
        Ok(())
    }
}

fn schema_digest<T: JsonSchema>() -> Result<Sha256Digest, RepositoryContractError> {
    let schema = schemars::schema_for!(T);
    let schema_value = serde_json::to_value(&schema)
        .map_err(|_| RepositoryContractError("standalone schema serialization failed"))?;
    audit_json_schema(&schema_value)
        .map_err(|_| RepositoryContractError("standalone schema audit failed"))?;
    canonical_contract_digest(&StandaloneSchemaDigestRecord(schema), None)
        .map_err(|_| RepositoryContractError("standalone schema digest failed"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
enum ContentAddressedSourceKind {
    #[serde(rename = "contentAddressed")]
    ContentAddressed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RepositoryHistorySourceEvidenceRef {
    source_kind: ContentAddressedSourceKind,
    evidence_kind: EvidenceKind,
    evidence_digest: Sha256Digest,
}

impl RepositoryHistorySourceEvidenceRef {
    pub(crate) fn new(
        evidence_kind: EvidenceKind,
        evidence_digest: &str,
    ) -> Result<Self, RepositoryContractError> {
        Ok(Self {
            source_kind: ContentAddressedSourceKind::ContentAddressed,
            evidence_kind,
            evidence_digest: parse_digest(evidence_digest)?,
        })
    }

    pub(crate) const fn evidence_kind(&self) -> EvidenceKind {
        self.evidence_kind
    }

    pub(crate) const fn evidence_digest(&self) -> &Sha256Digest {
        &self.evidence_digest
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EvidenceSourceIndexCandidateRow {
    Available {
        evidence_kind: EvidenceKind,
        refs: Vec<RepositoryHistorySourceEvidenceRef>,
    },
    Absent {
        evidence_kind: EvidenceKind,
    },
    Unknown {
        evidence_kind: EvidenceKind,
    },
}

impl EvidenceSourceIndexCandidateRow {
    pub(crate) fn available(
        evidence_kind: EvidenceKind,
        refs: Vec<RepositoryHistorySourceEvidenceRef>,
    ) -> Self {
        Self::Available {
            evidence_kind,
            refs,
        }
    }

    pub(crate) const fn absent(evidence_kind: EvidenceKind) -> Self {
        Self::Absent { evidence_kind }
    }

    pub(crate) const fn unknown(evidence_kind: EvidenceKind) -> Self {
        Self::Unknown { evidence_kind }
    }

    const fn evidence_kind(&self) -> EvidenceKind {
        match self {
            Self::Available { evidence_kind, .. }
            | Self::Absent { evidence_kind }
            | Self::Unknown { evidence_kind } => *evidence_kind,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EvidenceSourceIndexCandidate {
    repository_version: RepositoryVersion,
    registry_digest: Sha256Digest,
    source_index_receipt_id: UnicaId,
    availability: Vec<EvidenceSourceIndexCandidateRow>,
}

impl EvidenceSourceIndexCandidate {
    pub(crate) fn from_capability_adapter(
        repository_version: &str,
        registry_digest: &str,
        source_index_receipt_id: &str,
        availability: Vec<EvidenceSourceIndexCandidateRow>,
    ) -> Result<Self, RepositoryContractError> {
        Ok(Self {
            repository_version: RepositoryVersion::parse(repository_version)
                .map_err(|_| RepositoryContractError("invalid source-index version"))?,
            registry_digest: parse_digest(registry_digest)?,
            source_index_receipt_id: UnicaId::parse(source_index_receipt_id)
                .map_err(|_| RepositoryContractError("invalid source-index receipt id"))?,
            availability,
        })
    }
}

pub(crate) trait EvidenceSourceIndex {
    fn candidate_for(
        &self,
        repository_version: &RepositoryVersion,
        registry: &EvidenceSourceRegistry,
    ) -> Result<EvidenceSourceIndexCandidate, RepositoryContractError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
enum AvailableState {
    #[serde(rename = "available")]
    Available,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
enum AbsentStateMarker {
    #[serde(rename = "absent")]
    Absent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct AvailableEvidenceSource {
    evidence_kind: EvidenceKind,
    state: AvailableState,
    source_evidence_ref: RepositoryHistorySourceEvidenceRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct AbsentEvidenceSource {
    evidence_kind: EvidenceKind,
    state: AbsentStateMarker,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub(crate) enum EvidenceSourceAvailability {
    Available(AvailableEvidenceSource),
    Absent(AbsentEvidenceSource),
}

impl JsonSchema for EvidenceSourceAvailability {
    fn schema_name() -> Cow<'static, str> {
        "EvidenceSourceAvailability".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<AvailableEvidenceSource>(),
            generator.subschema_for::<AbsentEvidenceSource>(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct Task8EvidenceSourceAvailability([EvidenceSourceAvailability; 3]);

impl Task8EvidenceSourceAvailability {
    fn row(&self, kind: EvidenceKind) -> Option<&EvidenceSourceAvailability> {
        self.0.iter().find(|row| match row {
            EvidenceSourceAvailability::Available(value) => value.evidence_kind == kind,
            EvidenceSourceAvailability::Absent(value) => value.evidence_kind == kind,
        })
    }
}

impl JsonSchema for Task8EvidenceSourceAvailability {
    fn schema_name() -> Cow<'static, str> {
        "Task8EvidenceSourceAvailability".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "prefixItems": [
                availability_position_schema(EvidenceKind::RoutineClassification, generator),
                availability_position_schema(EvidenceKind::SupportPrerequisiteObservation, generator),
                availability_position_schema(EvidenceKind::NonConflictingConcurrent, generator),
            ],
            "items": false,
            "minItems": 3,
            "maxItems": 3,
        })
    }
}

fn availability_position_schema(kind: EvidenceKind, generator: &mut SchemaGenerator) -> Schema {
    let wire = match kind {
        EvidenceKind::RoutineClassification => "routineClassification",
        EvidenceKind::SupportPrerequisiteObservation => "supportPrerequisiteObservation",
        EvidenceKind::NonConflictingConcurrent => "nonConflictingConcurrent",
    };
    let digest = generator.subschema_for::<Sha256Digest>();
    let source_ref = json_schema!({
        "type": "object",
        "properties": {
            "sourceKind": { "type": "string", "const": "contentAddressed" },
            "evidenceKind": { "type": "string", "const": wire },
            "evidenceDigest": digest,
        },
        "required": ["sourceKind", "evidenceKind", "evidenceDigest"],
        "additionalProperties": false,
    });
    one_of_schema(vec![
        json_schema!({
            "type": "object",
            "properties": {
                "evidenceKind": { "type": "string", "const": wire },
                "state": { "type": "string", "const": "available" },
                "sourceEvidenceRef": source_ref,
            },
            "required": ["evidenceKind", "state", "sourceEvidenceRef"],
            "additionalProperties": false,
        }),
        json_schema!({
            "type": "object",
            "properties": {
                "evidenceKind": { "type": "string", "const": wire },
                "state": { "type": "string", "const": "absent" },
            },
            "required": ["evidenceKind", "state"],
            "additionalProperties": false,
        }),
    ])
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EvidenceSourceIndexProofDigestRecord {
    repository_version: RepositoryVersion,
    registry_digest: Sha256Digest,
    source_index_receipt_id: UnicaId,
    availability: Task8EvidenceSourceAvailability,
}

impl contract_digest_record_sealed::Sealed for EvidenceSourceIndexProofDigestRecord {}
impl ContractDigestRecord for EvidenceSourceIndexProofDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct EvidenceSourceIndexProof {
    repository_version: RepositoryVersion,
    registry_digest: Sha256Digest,
    source_index_receipt_id: UnicaId,
    availability: Task8EvidenceSourceAvailability,
    proof_digest: Sha256Digest,
    #[serde(skip)]
    #[schemars(skip)]
    validated_support_mapping: Option<ValidatedSupportObservationEntryProof>,
}

impl EvidenceSourceIndexProof {
    fn from_candidate(
        candidate: EvidenceSourceIndexCandidate,
        expected_version: &RepositoryVersion,
        registry: &EvidenceSourceRegistry,
    ) -> Result<Self, RepositoryContractError> {
        if &candidate.repository_version != expected_version
            || candidate.registry_digest != *registry.registry_digest()
            || candidate.availability.len() != 3
        {
            return Err(RepositoryContractError("source-index proof scope mismatch"));
        }
        let expected_kinds = registry.evidence_kinds();
        let mut rows = Vec::with_capacity(3);
        for (candidate_row, expected_kind) in candidate.availability.into_iter().zip(expected_kinds)
        {
            if candidate_row.evidence_kind() != expected_kind {
                return Err(RepositoryContractError(
                    "source-index availability order mismatch",
                ));
            }
            let row = match candidate_row {
                EvidenceSourceIndexCandidateRow::Available {
                    evidence_kind,
                    refs,
                } => {
                    if refs.len() != 1 || refs[0].evidence_kind != evidence_kind {
                        return Err(RepositoryContractError(
                            "source-index available row is not a unique matching ref",
                        ));
                    }
                    EvidenceSourceAvailability::Available(AvailableEvidenceSource {
                        evidence_kind,
                        state: AvailableState::Available,
                        source_evidence_ref: refs.into_iter().next().unwrap(),
                    })
                }
                EvidenceSourceIndexCandidateRow::Absent { evidence_kind } => {
                    EvidenceSourceAvailability::Absent(AbsentEvidenceSource {
                        evidence_kind,
                        state: AbsentStateMarker::Absent,
                    })
                }
                EvidenceSourceIndexCandidateRow::Unknown { .. } => {
                    return Err(RepositoryContractError(
                        "source-index absence was not established",
                    ));
                }
            };
            rows.push(row);
        }
        let availability = Task8EvidenceSourceAvailability(
            rows.try_into()
                .map_err(|_| RepositoryContractError("source-index row count mismatch"))?,
        );
        let record = EvidenceSourceIndexProofDigestRecord {
            repository_version: candidate.repository_version,
            registry_digest: candidate.registry_digest,
            source_index_receipt_id: candidate.source_index_receipt_id,
            availability,
        };
        let proof_digest = canonical_contract_digest(&record, None)
            .map_err(|_| RepositoryContractError("source-index proof digest failed"))?;
        Ok(Self {
            repository_version: record.repository_version,
            registry_digest: record.registry_digest,
            source_index_receipt_id: record.source_index_receipt_id,
            availability: record.availability,
            proof_digest,
            validated_support_mapping: None,
        })
    }

    fn row(&self, kind: EvidenceKind) -> Option<&EvidenceSourceAvailability> {
        self.availability.row(kind)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RepositoryHistoryOrderEvidence {
    capability_id: CapabilityRowId,
    from_exclusive: RepositoryHistoryCursor,
    through_inclusive: RepositoryHistoryCursor,
    ordered_versions: Vec<RepositoryVersion>,
    ordered_cursors: Vec<RepositoryHistoryCursor>,
}

impl RepositoryHistoryOrderEvidence {
    pub(crate) fn from_capability_adapter(
        capability_id: &str,
        from_exclusive: RepositoryHistoryCursor,
        through_inclusive: RepositoryHistoryCursor,
        ordered_cursors: Vec<RepositoryHistoryCursor>,
    ) -> Result<Self, RepositoryContractError> {
        if ordered_cursors.is_empty() || ordered_cursors.len() > 1024 {
            return Err(RepositoryContractError(
                "history order evidence must be non-empty and bounded",
            ));
        }
        let ordered_versions = ordered_cursors
            .iter()
            .map(|cursor| cursor.through_version.clone())
            .collect();
        Ok(Self {
            capability_id: CapabilityRowId::parse(capability_id)
                .map_err(|_| RepositoryContractError("invalid history-order capability"))?,
            from_exclusive,
            through_inclusive,
            ordered_versions,
            ordered_cursors,
        })
    }
}

pub(crate) trait RepositoryHistoryOrderResolver {
    fn order_evidence(
        &self,
        from_exclusive: &RepositoryHistoryCursor,
        through_inclusive: &RepositoryHistoryCursor,
    ) -> Result<RepositoryHistoryOrderEvidence, RepositoryContractError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RepositoryHistoryImmediateSuccessorEvidence {
    capability_id: CapabilityRowId,
    from_cursor: RepositoryHistoryCursor,
    first_observed_version: RepositoryVersion,
}

impl RepositoryHistoryImmediateSuccessorEvidence {
    fn from_capability_adapter(
        capability_id: &str,
        from_cursor: RepositoryHistoryCursor,
        first_observed_version: RepositoryVersion,
    ) -> Result<Self, RepositoryContractError> {
        Ok(Self {
            capability_id: CapabilityRowId::parse(capability_id)
                .map_err(|_| RepositoryContractError("invalid successor capability"))?,
            from_cursor,
            first_observed_version,
        })
    }

    pub(crate) const fn anchor_cursor(&self) -> &RepositoryHistoryCursor {
        &self.from_cursor
    }

    pub(crate) const fn first_observed_version(&self) -> &RepositoryVersion {
        &self.first_observed_version
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RepositoryHistoryCoverageGapEvidence {
    capability_id: CapabilityRowId,
    from_cursor: RepositoryHistoryCursor,
}

impl RepositoryHistoryCoverageGapEvidence {
    fn from_capability_adapter(
        capability_id: &str,
        from_cursor: RepositoryHistoryCursor,
    ) -> Result<Self, RepositoryContractError> {
        Ok(Self {
            capability_id: CapabilityRowId::parse(capability_id)
                .map_err(|_| RepositoryContractError("invalid coverage-gap capability"))?,
            from_cursor,
        })
    }

    pub(crate) const fn anchor_cursor(&self) -> &RepositoryHistoryCursor {
        &self.from_cursor
    }
}

pub(crate) trait RepositoryHistoryEvidenceBytesResolver {
    fn load_canonical_evidence_bytes(
        &self,
        reference: &RepositoryHistorySourceEvidenceRef,
    ) -> Result<Vec<u8>, RepositoryContractError>;
}

/// Capability-selected frozen historical source for an action correction.
///
/// This authority is deliberately neither serializable nor deserializable. Its
/// identity fields are typed, while the retained canonical bytes remain input
/// to strict decoding and rehashing by the repository validator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FrozenSupportCorrectiveInstructionSourceAuthority {
    historical_repository_version: RepositoryVersion,
    expected_historical_support_action_id: UnicaId,
    frozen_corrective_instruction_digest: Sha256Digest,
    canonical_instruction_bytes: Vec<u8>,
}

impl FrozenSupportCorrectiveInstructionSourceAuthority {
    pub(crate) fn from_capability_adapter(
        historical_repository_version: RepositoryVersion,
        expected_historical_support_action_id: UnicaId,
        frozen_corrective_instruction_digest: Sha256Digest,
        canonical_instruction_bytes: Vec<u8>,
    ) -> Self {
        Self {
            historical_repository_version,
            expected_historical_support_action_id,
            frozen_corrective_instruction_digest,
            canonical_instruction_bytes,
        }
    }

    pub(crate) const fn historical_repository_version(&self) -> &RepositoryVersion {
        &self.historical_repository_version
    }

    pub(crate) const fn expected_historical_support_action_id(&self) -> &UnicaId {
        &self.expected_historical_support_action_id
    }

    pub(crate) const fn frozen_corrective_instruction_digest(&self) -> &Sha256Digest {
        &self.frozen_corrective_instruction_digest
    }

    fn canonical_instruction_bytes(&self) -> &[u8] {
        &self.canonical_instruction_bytes
    }
}

/// Capability-selected frozen historical source for an external conflict
/// correction. This is a distinct type so the two instruction kinds cannot be
/// interchanged at the authority boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FrozenSupportConflictInstructionSourceAuthority {
    historical_repository_version: RepositoryVersion,
    expected_historical_support_action_id: UnicaId,
    expected_historical_conflict_resolution_id: UnicaId,
    frozen_support_conflict_instruction_digest: Sha256Digest,
    canonical_instruction_bytes: Vec<u8>,
}

impl FrozenSupportConflictInstructionSourceAuthority {
    pub(crate) fn from_capability_adapter(
        historical_repository_version: RepositoryVersion,
        expected_historical_support_action_id: UnicaId,
        expected_historical_conflict_resolution_id: UnicaId,
        frozen_support_conflict_instruction_digest: Sha256Digest,
        canonical_instruction_bytes: Vec<u8>,
    ) -> Self {
        Self {
            historical_repository_version,
            expected_historical_support_action_id,
            expected_historical_conflict_resolution_id,
            frozen_support_conflict_instruction_digest,
            canonical_instruction_bytes,
        }
    }

    pub(crate) const fn historical_repository_version(&self) -> &RepositoryVersion {
        &self.historical_repository_version
    }

    pub(crate) const fn expected_historical_support_action_id(&self) -> &UnicaId {
        &self.expected_historical_support_action_id
    }

    pub(crate) const fn expected_historical_conflict_resolution_id(&self) -> &UnicaId {
        &self.expected_historical_conflict_resolution_id
    }

    pub(crate) const fn frozen_support_conflict_instruction_digest(&self) -> &Sha256Digest {
        &self.frozen_support_conflict_instruction_digest
    }

    fn canonical_instruction_bytes(&self) -> &[u8] {
        &self.canonical_instruction_bytes
    }
}

/// Capability-backed resolver for versioned frozen historical records that
/// alone can make a structurally valid corrective observation authoritative.
///
/// Crucially, neither source method accepts an observation-supplied digest:
/// the capability adapter selects the frozen source independently by the
/// authority-trusted partition entry version.
/// The attribution methods bind facts which are not intrinsic to either
/// instruction record.
pub(crate) trait SupportCorrectiveEvidenceResolver {
    fn historical_frozen_support_corrective_instruction_source(
        &self,
        repository_version: &RepositoryVersion,
    ) -> Result<FrozenSupportCorrectiveInstructionSourceAuthority, RepositoryContractError>;

    fn historical_frozen_support_conflict_instruction_source(
        &self,
        repository_version: &RepositoryVersion,
    ) -> Result<FrozenSupportConflictInstructionSourceAuthority, RepositoryContractError>;

    fn frozen_support_action_id(&self) -> &UnicaId;

    fn support_history_order_authority(&self) -> &dyn SupportHistoryOrderAuthority;

    fn validate_action_correction_attribution(
        &self,
        repository_version: &RepositoryVersion,
        repository_actor: &RepositoryActorIdentity,
        instruction: &SupportCorrectiveInstruction,
    ) -> Result<(), RepositoryContractError>;

    fn validate_external_ownership_attribution(
        &self,
        repository_version: &RepositoryVersion,
        repository_actor: &RepositoryActorIdentity,
        root_delta_digest: &Sha256Digest,
        content_delta_digest: &Sha256Digest,
        evidence: &ExternalSupportOwnershipEvidence,
    ) -> Result<(), RepositoryContractError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
enum EvidenceBackedPartitionClassification {
    UnrelatedRoutine,
    RelevantRoutine,
    AuthorizedSupport,
    ExternalSupport,
    PreArmExternal,
    Invalid,
    Corrective,
}

impl From<EvidenceBackedPartitionClassification> for RepositoryHistoryPartitionClassification {
    fn from(value: EvidenceBackedPartitionClassification) -> Self {
        match value {
            EvidenceBackedPartitionClassification::UnrelatedRoutine => Self::UnrelatedRoutine,
            EvidenceBackedPartitionClassification::RelevantRoutine => Self::RelevantRoutine,
            EvidenceBackedPartitionClassification::AuthorizedSupport => Self::AuthorizedSupport,
            EvidenceBackedPartitionClassification::ExternalSupport => Self::ExternalSupport,
            EvidenceBackedPartitionClassification::PreArmExternal => Self::PreArmExternal,
            EvidenceBackedPartitionClassification::Invalid => Self::Invalid,
            EvidenceBackedPartitionClassification::Corrective => Self::Corrective,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
enum NonConflictingClassification {
    #[serde(rename = "nonConflictingConcurrent")]
    Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
enum TaskCommitClassification {
    #[serde(rename = "taskCommit")]
    Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EvidenceBackedHistoryPartitionEntry {
    repository_version: RepositoryVersion,
    classification: EvidenceBackedPartitionClassification,
    semantic_delta_digest: Sha256Digest,
    source_evidence_ref: RepositoryHistorySourceEvidenceRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NonConflictingHistoryPartitionEntry {
    repository_version: RepositoryVersion,
    classification: NonConflictingClassification,
    semantic_delta_digest: Sha256Digest,
    source_evidence_ref: RepositoryHistorySourceEvidenceRef,
    #[serde(deserialize_with = "deserialize_audited_non_conflicting_concurrent_evidence")]
    non_conflicting_concurrent_evidence: NonConflictingConcurrentEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TaskCommitHistoryPartitionEntry {
    repository_version: RepositoryVersion,
    classification: TaskCommitClassification,
    semantic_delta_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
enum RepositoryHistoryPartitionEntry {
    EvidenceBacked(EvidenceBackedHistoryPartitionEntry),
    NonConflicting(NonConflictingHistoryPartitionEntry),
    TaskCommit(TaskCommitHistoryPartitionEntry),
}

impl RepositoryHistoryPartitionEntry {
    fn repository_version(&self) -> &RepositoryVersion {
        match self {
            Self::EvidenceBacked(value) => &value.repository_version,
            Self::NonConflicting(value) => &value.repository_version,
            Self::TaskCommit(value) => &value.repository_version,
        }
    }

    fn classification(&self) -> RepositoryHistoryPartitionClassification {
        match self {
            Self::EvidenceBacked(value) => value.classification.into(),
            Self::NonConflicting(_) => {
                RepositoryHistoryPartitionClassification::NonConflictingConcurrent
            }
            Self::TaskCommit(_) => RepositoryHistoryPartitionClassification::TaskCommit,
        }
    }

    fn semantic_delta_digest(&self) -> &Sha256Digest {
        match self {
            Self::EvidenceBacked(value) => &value.semantic_delta_digest,
            Self::NonConflicting(value) => &value.semantic_delta_digest,
            Self::TaskCommit(value) => &value.semantic_delta_digest,
        }
    }
}

impl JsonSchema for RepositoryHistoryPartitionEntry {
    fn schema_name() -> Cow<'static, str> {
        "RepositoryHistoryPartitionEntry".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<EvidenceBackedHistoryPartitionEntry>(),
            generator.subschema_for::<NonConflictingHistoryPartitionEntry>(),
            generator.subschema_for::<TaskCommitHistoryPartitionEntry>(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct UnvalidatedRepositoryHistoryEntries(Vec<RepositoryHistoryPartitionEntry>);

impl<'de> Deserialize<'de> for UnvalidatedRepositoryHistoryEntries {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let entries = Vec::<RepositoryHistoryPartitionEntry>::deserialize(deserializer)?;
        if entries.len() > 1024 {
            return Err(D::Error::custom("history partition exceeds 1024 entries"));
        }
        Ok(Self(entries))
    }
}

impl JsonSchema for UnvalidatedRepositoryHistoryEntries {
    fn schema_name() -> Cow<'static, str> {
        "RepositoryHistoryPartitionEntries".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "maxItems": 1024,
            "items": generator.subschema_for::<RepositoryHistoryPartitionEntry>(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "RepositoryHistoryPartition")]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct UnvalidatedRepositoryHistoryPartition {
    from_exclusive: RepositoryHistoryCursor,
    through_inclusive: RepositoryHistoryCursor,
    entries: UnvalidatedRepositoryHistoryEntries,
    partition_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RepositoryHistoryPartitionDigestRecord {
    from_exclusive: RepositoryHistoryCursor,
    through_inclusive: RepositoryHistoryCursor,
    entries: UnvalidatedRepositoryHistoryEntries,
}

impl contract_digest_record_sealed::Sealed for RepositoryHistoryPartitionDigestRecord {}
impl ContractDigestRecord for RepositoryHistoryPartitionDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RepositorySemanticDeltaDigestRecord {
    repository_version: RepositoryVersion,
    partition_classification: RepositoryHistoryPartitionClassification,
    #[serde(deserialize_with = "RequiredNullable::deserialize_required")]
    root_delta_digest: RequiredNullable<Sha256Digest>,
    #[serde(deserialize_with = "RequiredNullable::deserialize_required")]
    content_delta_digest: RequiredNullable<Sha256Digest>,
    #[serde(deserialize_with = "RequiredNullable::deserialize_required")]
    classification_digest: RequiredNullable<Sha256Digest>,
    #[serde(deserialize_with = "RequiredNullable::deserialize_required")]
    external_support_disjointness_digest: RequiredNullable<Sha256Digest>,
    #[serde(deserialize_with = "RequiredNullable::deserialize_required")]
    corrective_instruction_digest: RequiredNullable<Sha256Digest>,
    #[serde(deserialize_with = "RequiredNullable::deserialize_required")]
    non_conflicting_concurrent_evidence_digest: RequiredNullable<Sha256Digest>,
}

impl contract_digest_record_sealed::Sealed for RepositorySemanticDeltaDigestRecord {}
impl ContractDigestRecord for RepositorySemanticDeltaDigestRecord {}

#[derive(Serialize)]
#[serde(transparent)]
struct CanonicalRoutineEvidenceRecord<'a>(&'a RoutineRepositoryVersionClassificationEvidence);

impl contract_digest_record_sealed::Sealed for CanonicalRoutineEvidenceRecord<'_> {}
impl ContractDigestRecord for CanonicalRoutineEvidenceRecord<'_> {}

#[derive(Serialize)]
#[serde(transparent)]
struct CanonicalNonConflictingEvidenceRecord<'a>(&'a NonConflictingConcurrentEvidence);

impl contract_digest_record_sealed::Sealed for CanonicalNonConflictingEvidenceRecord<'_> {}
impl ContractDigestRecord for CanonicalNonConflictingEvidenceRecord<'_> {}

#[derive(Serialize)]
#[serde(transparent)]
struct CanonicalSupportObservationRecord<'a>(&'a SupportPrerequisiteVersionObservation);

impl contract_digest_record_sealed::Sealed for CanonicalSupportObservationRecord<'_> {}
impl ContractDigestRecord for CanonicalSupportObservationRecord<'_> {}

#[derive(Serialize)]
#[serde(transparent)]
struct CanonicalSupportCorrectiveInstructionRecord<'a>(&'a SupportCorrectiveInstruction);

impl contract_digest_record_sealed::Sealed for CanonicalSupportCorrectiveInstructionRecord<'_> {}
impl ContractDigestRecord for CanonicalSupportCorrectiveInstructionRecord<'_> {}

#[derive(Serialize)]
#[serde(transparent)]
struct CanonicalSupportConflictInstructionRecord<'a>(&'a SupportConflictInstruction);

impl contract_digest_record_sealed::Sealed for CanonicalSupportConflictInstructionRecord<'_> {}
impl ContractDigestRecord for CanonicalSupportConflictInstructionRecord<'_> {}

enum ResolvedHistoryEvidence {
    Routine(RoutineRepositoryVersionClassificationEvidence),
    SupportObservation(SupportPrerequisiteVersionObservation),
    NonConflicting(NonConflictingConcurrentEvidence),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ValidatedSupportObservationEntryProof {
    repository_version: RepositoryVersion,
    partition_classification: RepositoryHistoryPartitionClassification,
    semantic_delta_digest: Sha256Digest,
    source_evidence_ref: RepositoryHistorySourceEvidenceRef,
    registry_digest: Sha256Digest,
    source_index_proof_digest: Sha256Digest,
    corrective_source_action_id: Option<UnicaId>,
}

/// Typed, non-wire view of one validated recovery-history entry. Authority
/// code consumes this projection directly and never reconstructs security
/// bindings by traversing serialized JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ValidatedSupportRecoveryHistoryEntryRef<'a> {
    SupportObservation {
        repository_version: &'a RepositoryVersion,
        partition_classification: RepositoryHistoryPartitionClassification,
        semantic_delta_digest: &'a Sha256Digest,
        source_evidence_digest: &'a Sha256Digest,
        corrective_source_action_id: Option<&'a UnicaId>,
    },
    NonConflicting {
        repository_version: &'a RepositoryVersion,
        semantic_delta_digest: &'a Sha256Digest,
        source_evidence_digest: &'a Sha256Digest,
        evidence: &'a NonConflictingConcurrentEvidence,
    },
    Unsupported {
        repository_version: &'a RepositoryVersion,
        partition_classification: RepositoryHistoryPartitionClassification,
        semantic_delta_digest: &'a Sha256Digest,
    },
}

/// Exact immediate-successor support entry proven by the Task 8 resolver.
///
/// The token has no wire constructor. It can be minted only from a validated
/// partition plus independent capability evidence for that partition's first
/// history successor. Downstream control flow must use this token rather than
/// the intrinsic observation projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ValidatedSupportObservationHistoryEntry {
    successor: RepositoryHistoryImmediateSuccessorEvidence,
    repository_version: RepositoryVersion,
    partition_classification: RepositoryHistoryPartitionClassification,
    semantic_delta_digest: Sha256Digest,
    source_evidence_ref: RepositoryHistorySourceEvidenceRef,
    registry_digest: Sha256Digest,
    source_index_proof_digest: Sha256Digest,
}

impl ValidatedSupportObservationHistoryEntry {
    pub(crate) fn from_validated_partition(
        partition: &ValidatedRepositoryHistoryPartition,
        successor: &RepositoryHistoryImmediateSuccessorEvidence,
    ) -> Result<Self, RepositoryContractError> {
        partition.immediate_support_observation_entry(successor)
    }

    pub(crate) const fn successor(&self) -> &RepositoryHistoryImmediateSuccessorEvidence {
        &self.successor
    }

    pub(crate) const fn repository_version(&self) -> &RepositoryVersion {
        &self.repository_version
    }

    pub(crate) const fn partition_classification(
        &self,
    ) -> RepositoryHistoryPartitionClassification {
        self.partition_classification
    }

    pub(crate) const fn semantic_delta_digest(&self) -> &Sha256Digest {
        &self.semantic_delta_digest
    }

    pub(crate) const fn source_evidence_ref(&self) -> &RepositoryHistorySourceEvidenceRef {
        &self.source_evidence_ref
    }

    pub(crate) const fn registry_digest(&self) -> &Sha256Digest {
        &self.registry_digest
    }

    pub(crate) const fn source_index_proof_digest(&self) -> &Sha256Digest {
        &self.source_index_proof_digest
    }
}

/// Capability-backed exact observation projection for a support-prerequisite
/// history range. Every supplied observation must be the content-addressed
/// source that the validated partition resolver bound to that same ordered
/// entry; version-only or lexical reconstruction is impossible.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ValidatedSupportPrerequisiteHistoryProjection {
    partition: ValidatedRepositoryHistoryPartition,
    observations: Vec<SupportPrerequisiteVersionObservation>,
    authorized_index: usize,
}

impl ValidatedSupportPrerequisiteHistoryProjection {
    pub(crate) fn from_validated_partition(
        partition: ValidatedRepositoryHistoryPartition,
        observations: Vec<SupportPrerequisiteVersionObservation>,
    ) -> Result<Self, RepositoryContractError> {
        if observations.len() != partition.entry_count() {
            return Err(RepositoryContractError(
                "support-prerequisite observations do not cover the exact history range",
            ));
        }
        let mut authorized_index = None;
        for (index, (entry, observation)) in partition
            .support_recovery_entries()
            .zip(observations.iter())
            .enumerate()
        {
            let projection =
                observation
                    .task8_mapping_projection()
                    .ok_or(RepositoryContractError(
                        "support-prerequisite history contains a non-Task8 observation",
                    ))?;
            let ValidatedSupportRecoveryHistoryEntryRef::SupportObservation {
                repository_version,
                partition_classification,
                source_evidence_digest,
                ..
            } = entry
            else {
                return Err(RepositoryContractError(
                    "support-prerequisite entry lacks its validated observation source",
                ));
            };
            if repository_version != observation.repository_version()
                || partition_classification != projection.partition_classification()
                || source_evidence_digest != observation.classification_digest()
            {
                return Err(RepositoryContractError(
                    "support-prerequisite observation differs from its exact partition source",
                ));
            }
            if partition_classification
                == RepositoryHistoryPartitionClassification::AuthorizedSupport
                && authorized_index.replace(index).is_some()
            {
                return Err(RepositoryContractError(
                    "support-prerequisite history contains more than one authorized entry",
                ));
            }
            if !matches!(
                partition_classification,
                RepositoryHistoryPartitionClassification::UnrelatedRoutine
                    | RepositoryHistoryPartitionClassification::RelevantRoutine
                    | RepositoryHistoryPartitionClassification::AuthorizedSupport
                    | RepositoryHistoryPartitionClassification::ExternalSupport
            ) {
                return Err(RepositoryContractError(
                    "support-prerequisite history contains an inadmissible classification",
                ));
            }
        }
        Ok(Self {
            partition,
            observations,
            authorized_index: authorized_index.ok_or(RepositoryContractError(
                "support-prerequisite history lacks its authorized entry",
            ))?,
        })
    }

    pub(crate) const fn partition(&self) -> &ValidatedRepositoryHistoryPartition {
        &self.partition
    }

    pub(crate) fn observations(&self) -> &[SupportPrerequisiteVersionObservation] {
        &self.observations
    }

    pub(crate) fn authorized_observation(&self) -> &SupportPrerequisiteVersionObservation {
        &self.observations[self.authorized_index]
    }

    pub(crate) fn final_root_repository_version(&self) -> &RepositoryVersion {
        self.observations
            .iter()
            .rev()
            .find(|observation| {
                observation
                    .task8_mapping_projection()
                    .is_some_and(|projection| {
                        matches!(
                            projection.partition_classification(),
                            RepositoryHistoryPartitionClassification::AuthorizedSupport
                                | RepositoryHistoryPartitionClassification::ExternalSupport
                        )
                    })
            })
            .expect("validated prerequisite history has an authorized root entry")
            .repository_version()
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        ValidatedRepositoryHistoryPartition,
        Vec<SupportPrerequisiteVersionObservation>,
        usize,
    ) {
        (self.partition, self.observations, self.authorized_index)
    }
}

fn load_history_evidence(
    reference: &RepositoryHistorySourceEvidenceRef,
    resolver: &dyn RepositoryHistoryEvidenceBytesResolver,
) -> Result<ResolvedHistoryEvidence, RepositoryContractError> {
    let bytes = resolver.load_canonical_evidence_bytes(reference)?;
    let value = crate::domain::i_json::from_slice(&bytes)
        .map_err(|_| RepositoryContractError("evidence bytes are not strict I-JSON"))?;
    match reference.evidence_kind {
        EvidenceKind::RoutineClassification => {
            let evidence =
                serde_json::from_value::<RoutineRepositoryVersionClassificationEvidence>(value)
                    .map_err(|_| RepositoryContractError("routine evidence typed decode failed"))?;
            if evidence.classification_digest != reference.evidence_digest {
                return Err(RepositoryContractError(
                    "routine evidence ref digest mismatch",
                ));
            }
            canonical_contract_digest(&CanonicalRoutineEvidenceRecord(&evidence), Some(&bytes))
                .map_err(|_| RepositoryContractError("routine evidence is not canonical"))?;
            Ok(ResolvedHistoryEvidence::Routine(evidence))
        }
        EvidenceKind::NonConflictingConcurrent => {
            let evidence =
                serde_json::from_value::<UnvalidatedNonConflictingConcurrentEvidence>(value)
                    .map_err(|_| {
                        RepositoryContractError("concurrent evidence typed decode failed")
                    })?
                    .into_audited_evidence()
                    .map_err(|_| {
                        RepositoryContractError("concurrent evidence typed decode failed")
                    })?;
            if evidence.evidence_digest != reference.evidence_digest {
                return Err(RepositoryContractError(
                    "concurrent evidence ref digest mismatch",
                ));
            }
            canonical_contract_digest(
                &CanonicalNonConflictingEvidenceRecord(&evidence),
                Some(&bytes),
            )
            .map_err(|_| RepositoryContractError("concurrent evidence is not canonical"))?;
            Ok(ResolvedHistoryEvidence::NonConflicting(evidence))
        }
        EvidenceKind::SupportPrerequisiteObservation => {
            let evidence = serde_json::from_value::<SupportPrerequisiteVersionObservation>(value)
                .map_err(|_| {
                RepositoryContractError("support observation typed decode failed")
            })?;
            if evidence.classification_digest() != &reference.evidence_digest {
                return Err(RepositoryContractError(
                    "support observation ref digest mismatch",
                ));
            }
            canonical_contract_digest(&CanonicalSupportObservationRecord(&evidence), Some(&bytes))
                .map_err(|_| RepositoryContractError("support observation is not canonical"))?;
            Ok(ResolvedHistoryEvidence::SupportObservation(evidence))
        }
    }
}

fn load_support_corrective_instruction(
    source: &FrozenSupportCorrectiveInstructionSourceAuthority,
) -> Result<SupportCorrectiveInstruction, RepositoryContractError> {
    let bytes = source.canonical_instruction_bytes();
    let value = crate::domain::i_json::from_slice(bytes).map_err(|_| {
        RepositoryContractError("corrective instruction bytes are not strict I-JSON")
    })?;
    let instruction = serde_json::from_value::<SupportCorrectiveInstruction>(value)
        .map_err(|_| RepositoryContractError("corrective instruction typed decode failed"))?;
    if instruction.corrective_instruction_digest() != source.frozen_corrective_instruction_digest()
        || instruction.support_action_id() != source.expected_historical_support_action_id()
    {
        return Err(RepositoryContractError(
            "corrective instruction frozen source mismatch",
        ));
    }
    canonical_contract_digest(
        &CanonicalSupportCorrectiveInstructionRecord(&instruction),
        Some(bytes),
    )
    .map_err(|_| RepositoryContractError("corrective instruction is not canonical"))?;
    Ok(instruction)
}

fn load_support_conflict_instruction(
    source: &FrozenSupportConflictInstructionSourceAuthority,
    history_order: &dyn SupportHistoryOrderAuthority,
) -> Result<SupportConflictInstruction, RepositoryContractError> {
    let bytes = source.canonical_instruction_bytes();
    let value = crate::domain::i_json::from_slice(bytes).map_err(|_| {
        RepositoryContractError("support-conflict instruction bytes are not strict I-JSON")
    })?;
    let instruction = decode_historical_support_conflict_instruction(value, history_order)
        .map_err(|_| RepositoryContractError("support-conflict instruction typed decode failed"))?;
    if instruction.support_conflict_instruction_digest()
        != source.frozen_support_conflict_instruction_digest()
        || instruction.conflict_resolution_id()
            != source.expected_historical_conflict_resolution_id()
    {
        return Err(RepositoryContractError(
            "support-conflict instruction frozen source mismatch",
        ));
    }
    canonical_contract_digest(
        &CanonicalSupportConflictInstructionRecord(&instruction),
        Some(bytes),
    )
    .map_err(|_| RepositoryContractError("support-conflict instruction is not canonical"))?;
    Ok(instruction)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ValidatedRepositoryHistoryPartition {
    wire: UnvalidatedRepositoryHistoryPartition,
    // The sole taskCommit constructor has no source-index row for its one
    // commit-owned entry. Every other slot remains aligned with an exact
    // authoritative source-index proof.
    source_index_proofs: Vec<Option<EvidenceSourceIndexProof>>,
    order_evidence: Option<RepositoryHistoryOrderEvidence>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ValidatedRepositoryHistoryEntryRef<'a> {
    repository_version: &'a RepositoryVersion,
    classification: RepositoryHistoryPartitionClassification,
    semantic_delta_digest: &'a Sha256Digest,
}

impl<'a> ValidatedRepositoryHistoryEntryRef<'a> {
    pub(crate) const fn repository_version(&self) -> &'a RepositoryVersion {
        self.repository_version
    }

    pub(crate) const fn classification(&self) -> RepositoryHistoryPartitionClassification {
        self.classification
    }

    pub(crate) const fn semantic_delta_digest(&self) -> &'a Sha256Digest {
        self.semantic_delta_digest
    }
}

/// Commit-owned partition plus the exact authority lineage that validated its
/// otherwise source-less taskCommit entry. Deliberately non-`Clone` and
/// non-wire so a generic partition cannot be substituted at completion.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ValidatedTaskCommitHistoryPartition {
    partition: ValidatedRepositoryHistoryPartition,
    object_history_binding_witness: super::results::repository::CommitObjectHistoryBindingWitness,
    repository_version: RepositoryVersion,
    committed_objects_digest: Sha256Digest,
    atomic_commit_safety_capability_id: CapabilityRowId,
}

impl ValidatedTaskCommitHistoryPartition {
    pub(crate) const fn partition(&self) -> &ValidatedRepositoryHistoryPartition {
        &self.partition
    }

    pub(crate) fn binds<Binding>(&self, commit: &Binding) -> bool
    where
        Binding: CommitObjectHistoryBinding + ?Sized,
    {
        self.object_history_binding_witness
            .same_invocation(commit.object_history_binding_witness())
            && &self.repository_version == commit.repository_version()
            && &self.committed_objects_digest == commit.committed_objects_digest()
            && &self.atomic_commit_safety_capability_id
                == commit.atomic_commit_safety_capability_id()
    }

    pub(crate) fn into_partition(self) -> ValidatedRepositoryHistoryPartition {
        self.partition
    }
}

impl Serialize for ValidatedRepositoryHistoryPartition {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.wire.serialize(serializer)
    }
}

impl JsonSchema for ValidatedRepositoryHistoryPartition {
    fn schema_name() -> Cow<'static, str> {
        "RepositoryHistoryPartition".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        <UnvalidatedRepositoryHistoryPartition as JsonSchema>::json_schema(generator)
    }
}

impl ValidatedRepositoryHistoryPartition {
    pub(crate) fn start_cursor(&self) -> &RepositoryHistoryCursor {
        &self.wire.from_exclusive
    }

    pub(crate) fn through_inclusive(&self) -> &RepositoryHistoryCursor {
        &self.wire.through_inclusive
    }

    pub(crate) fn partition_digest(&self) -> &Sha256Digest {
        &self.wire.partition_digest
    }

    pub(crate) fn classifications(
        &self,
    ) -> impl Iterator<Item = RepositoryHistoryPartitionClassification> + '_ {
        self.wire
            .entries
            .0
            .iter()
            .map(|entry| entry.classification())
    }

    pub(crate) fn entry_count(&self) -> usize {
        self.wire.entries.0.len()
    }

    pub(crate) fn entries(
        &self,
    ) -> impl Iterator<Item = ValidatedRepositoryHistoryEntryRef<'_>> + '_ {
        self.wire
            .entries
            .0
            .iter()
            .map(|entry| ValidatedRepositoryHistoryEntryRef {
                repository_version: entry.repository_version(),
                classification: entry.classification(),
                semantic_delta_digest: entry.semantic_delta_digest(),
            })
    }

    pub(crate) fn first_entry(&self) -> Option<ValidatedRepositoryHistoryEntryRef<'_>> {
        self.entries().next()
    }

    pub(crate) fn support_recovery_entries(
        &self,
    ) -> impl Iterator<Item = ValidatedSupportRecoveryHistoryEntryRef<'_>> + '_ {
        self.wire
            .entries
            .0
            .iter()
            .enumerate()
            .map(|(index, entry)| match entry {
                RepositoryHistoryPartitionEntry::EvidenceBacked(value) => self
                    .source_index_proofs
                    .get(index)
                    .and_then(Option::as_ref)
                    .and_then(|proof| proof.validated_support_mapping.as_ref())
                    .map_or_else(
                        || ValidatedSupportRecoveryHistoryEntryRef::Unsupported {
                            repository_version: &value.repository_version,
                            partition_classification: value.classification.into(),
                            semantic_delta_digest: &value.semantic_delta_digest,
                        },
                        |validated| ValidatedSupportRecoveryHistoryEntryRef::SupportObservation {
                            repository_version: &validated.repository_version,
                            partition_classification: validated.partition_classification,
                            semantic_delta_digest: &validated.semantic_delta_digest,
                            source_evidence_digest: validated.source_evidence_ref.evidence_digest(),
                            corrective_source_action_id: validated
                                .corrective_source_action_id
                                .as_ref(),
                        },
                    ),
                RepositoryHistoryPartitionEntry::NonConflicting(value) => {
                    ValidatedSupportRecoveryHistoryEntryRef::NonConflicting {
                        repository_version: &value.repository_version,
                        semantic_delta_digest: &value.semantic_delta_digest,
                        source_evidence_digest: value.source_evidence_ref.evidence_digest(),
                        evidence: &value.non_conflicting_concurrent_evidence,
                    }
                }
                RepositoryHistoryPartitionEntry::TaskCommit(value) => {
                    ValidatedSupportRecoveryHistoryEntryRef::Unsupported {
                        repository_version: &value.repository_version,
                        partition_classification:
                            RepositoryHistoryPartitionClassification::TaskCommit,
                        semantic_delta_digest: &value.semantic_delta_digest,
                    }
                }
            })
    }

    pub(crate) fn has_exact_entry_prefix(&self, prefix: &Self) -> bool {
        self.start_cursor() == prefix.start_cursor()
            && self.contains_cursor(prefix.through_inclusive())
            && self
                .wire
                .entries
                .0
                .starts_with(prefix.wire.entries.0.as_slice())
    }

    /// Compares the validated wire projection while deliberately ignoring the
    /// identity of independently acquired hidden order/source proofs.
    pub(crate) fn is_semantically_exact(&self, other: &Self) -> bool {
        self.wire == other.wire
    }

    /// A strict exact-prefix extension whose appended entries are all routine
    /// and unrelated. No repository-version lexical ordering is inferred here;
    /// ordering comes exclusively from the validated partition evidence.
    pub(crate) fn is_strict_unrelated_extension_of(&self, prefix: &Self) -> bool {
        self.entry_count() > prefix.entry_count()
            && self.has_exact_entry_prefix(prefix)
            && self
                .classifications()
                .skip(prefix.entry_count())
                .all(|classification| {
                    classification == RepositoryHistoryPartitionClassification::UnrelatedRoutine
                })
    }

    pub(crate) fn contains_repository_version(&self, version: &RepositoryVersion) -> bool {
        version == self.start_cursor().through_version()
            || self
                .wire
                .entries
                .0
                .iter()
                .any(|entry| entry.repository_version() == version)
    }

    /// Iterates only repository versions represented by partition entries.
    /// The `fromExclusive` cursor is deliberately excluded: it is an ordering
    /// anchor, not an observed history entry.
    pub(crate) fn entry_versions(&self) -> impl Iterator<Item = &RepositoryVersion> {
        self.wire
            .entries
            .0
            .iter()
            .map(RepositoryHistoryPartitionEntry::repository_version)
    }

    pub(crate) fn all_entries_are_one_of(
        &self,
        allowed: &[RepositoryHistoryPartitionClassification],
    ) -> bool {
        self.classifications()
            .all(|classification| allowed.contains(&classification))
    }

    /// Returns true only when capability-backed order evidence contains this
    /// exact cursor, including its history-prefix digest. The starting cursor
    /// is part of the closed range even for an empty partition.
    pub(crate) fn contains_cursor(&self, cursor: &RepositoryHistoryCursor) -> bool {
        cursor == &self.wire.from_exclusive
            || self.order_evidence.as_ref().is_some_and(|evidence| {
                evidence.ordered_cursors.iter().any(|known| known == cursor)
            })
    }

    pub(crate) fn non_conflicting_entries_bind_atomic_safety_capability(
        &self,
        expected_capability_id: &CapabilityRowId,
    ) -> bool {
        self.wire.entries.0.iter().all(|entry| match entry {
            RepositoryHistoryPartitionEntry::NonConflicting(value) => {
                &value
                    .non_conflicting_concurrent_evidence
                    .atomic_commit_safety_capability_id
                    == expected_capability_id
            }
            RepositoryHistoryPartitionEntry::EvidenceBacked(_)
            | RepositoryHistoryPartitionEntry::TaskCommit(_) => true,
        })
    }

    fn immediate_support_observation_entry(
        &self,
        successor: &RepositoryHistoryImmediateSuccessorEvidence,
    ) -> Result<ValidatedSupportObservationHistoryEntry, RepositoryContractError> {
        let order = self.order_evidence.as_ref().ok_or(RepositoryContractError(
            "support successor requires non-empty history-order evidence",
        ))?;
        let entry = self.wire.entries.0.first().ok_or(RepositoryContractError(
            "support successor requires a first partition entry",
        ))?;
        if successor.anchor_cursor() != &self.wire.from_exclusive
            || &order.from_exclusive != successor.anchor_cursor()
            || order.through_inclusive != self.wire.through_inclusive
            || order.ordered_versions.first() != Some(successor.first_observed_version())
            || entry.repository_version() != successor.first_observed_version()
            || self.source_index_proofs.len() != self.wire.entries.0.len()
        {
            return Err(RepositoryContractError(
                "support successor scope does not match the validated partition",
            ));
        }

        let expected_partition_digest = canonical_contract_digest(
            &RepositoryHistoryPartitionDigestRecord {
                from_exclusive: self.wire.from_exclusive.clone(),
                through_inclusive: self.wire.through_inclusive.clone(),
                entries: self.wire.entries.clone(),
            },
            None,
        )
        .map_err(|_| RepositoryContractError("partition digest failed"))?;
        if expected_partition_digest != self.wire.partition_digest {
            return Err(RepositoryContractError(
                "support successor partition digest mismatch",
            ));
        }

        let entry = match entry {
            RepositoryHistoryPartitionEntry::EvidenceBacked(value) => value,
            RepositoryHistoryPartitionEntry::NonConflicting(_)
            | RepositoryHistoryPartitionEntry::TaskCommit(_) => {
                return Err(RepositoryContractError(
                    "immediate successor is not support-observation backed",
                ));
            }
        };
        let index_proof = self.source_index_proofs[0]
            .as_ref()
            .ok_or(RepositoryContractError(
                "immediate successor lacks its source-index proof",
            ))?;
        let validated =
            index_proof
                .validated_support_mapping
                .as_ref()
                .ok_or(RepositoryContractError(
                    "immediate successor lacks validated support evidence",
                ))?;
        let support_ref = match index_proof
            .row(EvidenceKind::SupportPrerequisiteObservation)
            .ok_or(RepositoryContractError(
                "missing support-observation index row",
            ))? {
            EvidenceSourceAvailability::Available(value) => &value.source_evidence_ref,
            EvidenceSourceAvailability::Absent(_) => {
                return Err(RepositoryContractError(
                    "support-observation source is not available",
                ));
            }
        };
        let expected_index_proof_digest = canonical_contract_digest(
            &EvidenceSourceIndexProofDigestRecord {
                repository_version: index_proof.repository_version.clone(),
                registry_digest: index_proof.registry_digest.clone(),
                source_index_receipt_id: index_proof.source_index_receipt_id.clone(),
                availability: index_proof.availability.clone(),
            },
            None,
        )
        .map_err(|_| RepositoryContractError("source-index proof digest failed"))?;
        let partition_classification =
            RepositoryHistoryPartitionClassification::from(entry.classification);
        if index_proof.repository_version != entry.repository_version
            || index_proof.registry_digest.as_str() != TASK9_EVIDENCE_SOURCE_REGISTRY_DIGEST
            || expected_index_proof_digest != index_proof.proof_digest
            || support_ref.evidence_kind != EvidenceKind::SupportPrerequisiteObservation
            || support_ref != &entry.source_evidence_ref
            || validated.repository_version != entry.repository_version
            || validated.partition_classification != partition_classification
            || validated.semantic_delta_digest != entry.semantic_delta_digest
            || validated.source_evidence_ref != entry.source_evidence_ref
            || validated.registry_digest != index_proof.registry_digest
            || validated.source_index_proof_digest != index_proof.proof_digest
        {
            return Err(RepositoryContractError(
                "support successor disagrees with validated source mapping",
            ));
        }

        Ok(ValidatedSupportObservationHistoryEntry {
            successor: successor.clone(),
            repository_version: validated.repository_version.clone(),
            partition_classification: validated.partition_classification,
            semantic_delta_digest: validated.semantic_delta_digest.clone(),
            source_evidence_ref: validated.source_evidence_ref.clone(),
            registry_digest: validated.registry_digest.clone(),
            source_index_proof_digest: validated.source_index_proof_digest.clone(),
        })
    }
}

/// Canonical repository-target set used by a scoped NCC observation. Empty is
/// valid for sets such as approved deletion targets; call sites that require a
/// non-empty set use `NonEmptyCanonicalRepositoryTargetSet` instead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct CanonicalRepositoryTargetSet(Vec<RepositoryTargetIdentity>);

impl CanonicalRepositoryTargetSet {
    pub(crate) fn new(
        targets: Vec<RepositoryTargetIdentity>,
    ) -> Result<Self, RepositoryContractError> {
        if targets.len() > MAX_METADATA_ITEMS || targets.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(RepositoryContractError(
                "repository target set is not canonical and unique",
            ));
        }
        Ok(Self(targets))
    }

    fn as_slice(&self) -> &[RepositoryTargetIdentity] {
        &self.0
    }

    fn contains(&self, target: &RepositoryTargetIdentity) -> bool {
        self.0.binary_search(target).is_ok()
    }

    fn is_disjoint(&self, other: &Self) -> bool {
        let mut left = self.0.iter().peekable();
        let mut right = other.0.iter().peekable();
        while let (Some(left_target), Some(right_target)) = (left.peek(), right.peek()) {
            match left_target.cmp(right_target) {
                Ordering::Less => {
                    left.next();
                }
                Ordering::Greater => {
                    right.next();
                }
                Ordering::Equal => return false,
            }
        }
        true
    }

    fn digest(&self) -> Result<Sha256Digest, RepositoryContractError> {
        canonical_contract_digest(
            &CanonicalRepositoryTargetSetDigestRecord(self.as_slice()),
            None,
        )
        .map_err(|_| RepositoryContractError("repository target-set digest failed"))
    }
}

#[derive(Serialize)]
#[serde(transparent)]
struct CanonicalRepositoryTargetSetDigestRecord<'a>(&'a [RepositoryTargetIdentity]);

impl contract_digest_record_sealed::Sealed for CanonicalRepositoryTargetSetDigestRecord<'_> {}
impl ContractDigestRecord for CanonicalRepositoryTargetSetDigestRecord<'_> {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct NonEmptyCanonicalRepositoryTargetSet(CanonicalRepositoryTargetSet);

impl NonEmptyCanonicalRepositoryTargetSet {
    pub(crate) fn new(
        targets: Vec<RepositoryTargetIdentity>,
    ) -> Result<Self, RepositoryContractError> {
        let targets = CanonicalRepositoryTargetSet::new(targets)?;
        if targets.as_slice().is_empty() {
            return Err(RepositoryContractError(
                "repository target set must be non-empty",
            ));
        }
        Ok(Self(targets))
    }

    fn as_canonical(&self) -> &CanonicalRepositoryTargetSet {
        &self.0
    }

    fn as_slice(&self) -> &[RepositoryTargetIdentity] {
        self.0.as_slice()
    }

    fn digest(&self) -> Result<Sha256Digest, RepositoryContractError> {
        self.0.digest()
    }
}

/// Directed repository reference from the target containing the reference to
/// the target being referenced. A new edge blocks an approved deletion only
/// when its `referenced_target` is one of the approved deletion targets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RepositoryReferenceEdge {
    referencing_target: RepositoryTargetIdentity,
    referenced_target: RepositoryTargetIdentity,
}

impl RepositoryReferenceEdge {
    pub(crate) const fn new(
        referencing_target: RepositoryTargetIdentity,
        referenced_target: RepositoryTargetIdentity,
    ) -> Self {
        Self {
            referencing_target,
            referenced_target,
        }
    }
}

impl Ord for RepositoryReferenceEdge {
    fn cmp(&self, other: &Self) -> Ordering {
        self.referencing_target
            .cmp(&other.referencing_target)
            .then_with(|| self.referenced_target.cmp(&other.referenced_target))
    }
}

impl PartialOrd for RepositoryReferenceEdge {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct CanonicalRepositoryReferenceEdgeSet(Vec<RepositoryReferenceEdge>);

impl CanonicalRepositoryReferenceEdgeSet {
    pub(crate) fn new(
        edges: Vec<RepositoryReferenceEdge>,
    ) -> Result<Self, RepositoryContractError> {
        if edges.len() > MAX_METADATA_ITEMS || edges.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(RepositoryContractError(
                "repository reference-edge set is not canonical and unique",
            ));
        }
        Ok(Self(edges))
    }

    fn as_slice(&self) -> &[RepositoryReferenceEdge] {
        &self.0
    }

    fn digest(&self) -> Result<Sha256Digest, RepositoryContractError> {
        canonical_contract_digest(
            &CanonicalRepositoryReferenceEdgeSetDigestRecord(self.as_slice()),
            None,
        )
        .map_err(|_| RepositoryContractError("repository reference-edge-set digest failed"))
    }
}

#[derive(Serialize)]
#[serde(transparent)]
struct CanonicalRepositoryReferenceEdgeSetDigestRecord<'a>(&'a [RepositoryReferenceEdge]);

impl contract_digest_record_sealed::Sealed for CanonicalRepositoryReferenceEdgeSetDigestRecord<'_> {}
impl ContractDigestRecord for CanonicalRepositoryReferenceEdgeSetDigestRecord<'_> {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScopedNccObservedTargetSets {
    locked_targets: NonEmptyCanonicalRepositoryTargetSet,
    changed_targets: NonEmptyCanonicalRepositoryTargetSet,
    integration_content_targets: NonEmptyCanonicalRepositoryTargetSet,
    approved_deletion_targets: CanonicalRepositoryTargetSet,
}

impl ScopedNccObservedTargetSets {
    pub(crate) const fn new(
        locked_targets: NonEmptyCanonicalRepositoryTargetSet,
        changed_targets: NonEmptyCanonicalRepositoryTargetSet,
        integration_content_targets: NonEmptyCanonicalRepositoryTargetSet,
        approved_deletion_targets: CanonicalRepositoryTargetSet,
    ) -> Self {
        Self {
            locked_targets,
            changed_targets,
            integration_content_targets,
            approved_deletion_targets,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScopedNccObservedReferenceSets {
    before_reference_closure: CanonicalRepositoryReferenceEdgeSet,
    after_reference_closure: CanonicalRepositoryReferenceEdgeSet,
    added_reference_edges: CanonicalRepositoryReferenceEdgeSet,
    before_support_graph: CanonicalRepositoryReferenceEdgeSet,
    after_support_graph: CanonicalRepositoryReferenceEdgeSet,
}

impl ScopedNccObservedReferenceSets {
    pub(crate) const fn new(
        before_reference_closure: CanonicalRepositoryReferenceEdgeSet,
        after_reference_closure: CanonicalRepositoryReferenceEdgeSet,
        added_reference_edges: CanonicalRepositoryReferenceEdgeSet,
        before_support_graph: CanonicalRepositoryReferenceEdgeSet,
        after_support_graph: CanonicalRepositoryReferenceEdgeSet,
    ) -> Self {
        Self {
            before_reference_closure,
            after_reference_closure,
            added_reference_edges,
            before_support_graph,
            after_support_graph,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScopedNccObservedStateSets {
    before_validation_inputs: RepositoryTargetStates,
    after_validation_inputs: RepositoryTargetStates,
    before_root_states: RepositoryTargetStates,
    after_root_states: RepositoryTargetStates,
    before_locked_target_states: RepositoryTargetStates,
    after_locked_target_states: RepositoryTargetStates,
}

impl ScopedNccObservedStateSets {
    pub(crate) const fn new(
        before_validation_inputs: RepositoryTargetStates,
        after_validation_inputs: RepositoryTargetStates,
        before_root_states: RepositoryTargetStates,
        after_root_states: RepositoryTargetStates,
        before_locked_target_states: RepositoryTargetStates,
        after_locked_target_states: RepositoryTargetStates,
    ) -> Self {
        Self {
            before_validation_inputs,
            after_validation_inputs,
            before_root_states,
            after_root_states,
            before_locked_target_states,
            after_locked_target_states,
        }
    }
}

/// Full adapter observation for one NCC position. It contains no supplied
/// safety booleans or precomputed safety digests; the core derives both from
/// these canonical targets, edges, and complete before/after states.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScopedNccRowFacts {
    atomic_commit_safety_capability_id: CapabilityRowId,
    target_sets: ScopedNccObservedTargetSets,
    reference_sets: ScopedNccObservedReferenceSets,
    states: ScopedNccObservedStateSets,
}

impl ScopedNccRowFacts {
    pub(crate) const fn new(
        atomic_commit_safety_capability_id: CapabilityRowId,
        target_sets: ScopedNccObservedTargetSets,
        reference_sets: ScopedNccObservedReferenceSets,
        states: ScopedNccObservedStateSets,
    ) -> Self {
        Self {
            atomic_commit_safety_capability_id,
            target_sets,
            reference_sets,
            states,
        }
    }
}

#[derive(Debug)]
struct ScopedNccHistoryScanInvocationMarker;

#[derive(Debug)]
struct ScopedNccHistoryScanInvocationCapability(Arc<ScopedNccHistoryScanInvocationMarker>);

#[derive(Debug)]
struct ScopedNccHistoryScanCompletionCapability(Arc<ScopedNccHistoryScanInvocationMarker>);

#[derive(Debug, Clone)]
pub(crate) struct ScopedNccHistoryScanBatchWitness(Arc<ScopedNccHistoryScanInvocationMarker>);

impl ScopedNccHistoryScanInvocationCapability {
    fn mint() -> Self {
        Self(Arc::new(ScopedNccHistoryScanInvocationMarker))
    }

    fn completion(&self) -> ScopedNccHistoryScanCompletionCapability {
        ScopedNccHistoryScanCompletionCapability(Arc::clone(&self.0))
    }

    fn batch_witness(&self) -> ScopedNccHistoryScanBatchWitness {
        ScopedNccHistoryScanBatchWitness(Arc::clone(&self.0))
    }

    fn owns_completion(&self, completion: &ScopedNccHistoryScanCompletionCapability) -> bool {
        Arc::ptr_eq(&self.0, &completion.0)
    }

    fn owns_batch_witness(&self, witness: &ScopedNccHistoryScanBatchWitness) -> bool {
        Arc::ptr_eq(&self.0, &witness.0)
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ScopedNccHistoryScanRowRequestRef<'a> {
    position: usize,
    cursor: &'a RepositoryHistoryCursor,
    repository_version: &'a RepositoryVersion,
    classification: RepositoryHistoryPartitionClassification,
}

impl ScopedNccHistoryScanRowRequestRef<'_> {
    pub(crate) const fn position(&self) -> usize {
        self.position
    }

    pub(crate) const fn cursor(&self) -> &RepositoryHistoryCursor {
        self.cursor
    }

    pub(crate) const fn repository_version(&self) -> &RepositoryVersion {
        self.repository_version
    }

    pub(crate) const fn classification(&self) -> RepositoryHistoryPartitionClassification {
        self.classification
    }
}

/// One invocation-bound full-history scan request. The adapter can observe
/// rows only through this request and cannot mint an accepted row or batch
/// from equal scalar data belonging to another scan invocation.
#[derive(Debug)]
pub(crate) struct ScopedNccHistoryScanRequest<'a> {
    partition: &'a ValidatedRepositoryHistoryPartition,
    expected_capability_id: &'a CapabilityRowId,
    invocation: &'a ScopedNccHistoryScanInvocationCapability,
}

impl<'a> ScopedNccHistoryScanRequest<'a> {
    fn new(
        partition: &'a ValidatedRepositoryHistoryPartition,
        expected_capability_id: &'a CapabilityRowId,
        invocation: &'a ScopedNccHistoryScanInvocationCapability,
    ) -> Self {
        Self {
            partition,
            expected_capability_id,
            invocation,
        }
    }

    pub(crate) fn start_cursor(&self) -> &RepositoryHistoryCursor {
        self.partition.start_cursor()
    }

    pub(crate) fn through_inclusive(&self) -> &RepositoryHistoryCursor {
        self.partition.through_inclusive()
    }

    pub(crate) fn partition_digest(&self) -> &Sha256Digest {
        self.partition.partition_digest()
    }

    pub(crate) const fn expected_capability_id(&self) -> &CapabilityRowId {
        self.expected_capability_id
    }

    pub(crate) fn row_count(&self) -> usize {
        self.partition.entry_count()
    }

    pub(crate) fn row(&self, index: usize) -> Option<ScopedNccHistoryScanRowRequestRef<'_>> {
        let order = self.partition.order_evidence.as_ref()?;
        let cursor = order.ordered_cursors.get(index)?;
        let entry = self.partition.wire.entries.0.get(index)?;
        Some(ScopedNccHistoryScanRowRequestRef {
            position: index + 1,
            cursor,
            repository_version: entry.repository_version(),
            classification: entry.classification(),
        })
    }

    pub(crate) fn batch_witness(&self) -> ScopedNccHistoryScanBatchWitness {
        self.invocation.batch_witness()
    }

    pub(crate) fn complete(
        self,
        lease: Box<dyn ScopedNccHistoryScanLease>,
    ) -> ScopedNccHistoryScanCompletion {
        ScopedNccHistoryScanCompletion {
            completion: self.invocation.completion(),
            lease,
        }
    }
}

#[derive(Debug)]
pub(crate) struct ScopedNccHistoryRowObservationInput {
    position: usize,
    cursor: RepositoryHistoryCursor,
    repository_version: RepositoryVersion,
    facts: Option<ScopedNccRowFacts>,
}

impl ScopedNccHistoryRowObservationInput {
    pub(crate) const fn new(
        position: usize,
        cursor: RepositoryHistoryCursor,
        repository_version: RepositoryVersion,
        facts: Option<ScopedNccRowFacts>,
    ) -> Self {
        Self {
            position,
            cursor,
            repository_version,
            facts,
        }
    }
}

#[derive(Debug)]
pub(crate) struct ScopedNccHistoryRowObservation {
    batch_witness: ScopedNccHistoryScanBatchWitness,
    position: usize,
    cursor: RepositoryHistoryCursor,
    repository_version: RepositoryVersion,
    facts: Option<ScopedNccRowFacts>,
}

impl ScopedNccHistoryRowObservation {
    pub(crate) fn from_capability_adapter(
        request: &ScopedNccHistoryScanRequest<'_>,
        input: ScopedNccHistoryRowObservationInput,
    ) -> Result<Self, RepositoryContractError> {
        if input.position == 0 || input.position > MAX_METADATA_ITEMS {
            return Err(RepositoryContractError(
                "scoped NCC row position is outside the bounded history range",
            ));
        }
        Ok(Self {
            batch_witness: request.batch_witness(),
            position: input.position,
            cursor: input.cursor,
            repository_version: input.repository_version,
            facts: input.facts,
        })
    }
}

#[derive(Debug)]
pub(crate) struct ScopedNccHistoryScanBatchInput {
    from_exclusive: RepositoryHistoryCursor,
    through_inclusive: RepositoryHistoryCursor,
    partition_digest: Sha256Digest,
    rows: Vec<ScopedNccHistoryRowObservation>,
}

impl ScopedNccHistoryScanBatchInput {
    pub(crate) const fn new(
        from_exclusive: RepositoryHistoryCursor,
        through_inclusive: RepositoryHistoryCursor,
        partition_digest: Sha256Digest,
        rows: Vec<ScopedNccHistoryRowObservation>,
    ) -> Self {
        Self {
            from_exclusive,
            through_inclusive,
            partition_digest,
            rows,
        }
    }
}

#[derive(Debug)]
pub(crate) struct ScopedNccHistoryScanObservation {
    batch_witness: ScopedNccHistoryScanBatchWitness,
    from_exclusive: RepositoryHistoryCursor,
    through_inclusive: RepositoryHistoryCursor,
    partition_digest: Sha256Digest,
    rows: Vec<ScopedNccHistoryRowObservation>,
}

impl ScopedNccHistoryScanObservation {
    pub(crate) fn from_capability_adapter(
        request: &ScopedNccHistoryScanRequest<'_>,
        input: ScopedNccHistoryScanBatchInput,
    ) -> Result<Self, RepositoryContractError> {
        if input.rows.len() > MAX_METADATA_ITEMS {
            return Err(RepositoryContractError(
                "scoped NCC scan exceeds the bounded history range",
            ));
        }
        Ok(Self {
            batch_witness: request.batch_witness(),
            from_exclusive: input.from_exclusive,
            through_inclusive: input.through_inclusive,
            partition_digest: input.partition_digest,
            rows: input.rows,
        })
    }
}

pub(crate) trait ScopedNccHistoryScanLease {
    fn batch_witness(&self) -> &ScopedNccHistoryScanBatchWitness;
    fn binds(&self, request: &ScopedNccHistoryScanRequest<'_>) -> bool;
    fn into_observation(self: Box<Self>) -> ScopedNccHistoryScanObservation;
}

pub(crate) struct ScopedNccHistoryScanCompletion {
    completion: ScopedNccHistoryScanCompletionCapability,
    lease: Box<dyn ScopedNccHistoryScanLease>,
}

impl fmt::Debug for ScopedNccHistoryScanCompletion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ScopedNccHistoryScanCompletion")
            .field("completion", &self.completion)
            .field("lease", &"<invocation-bound scoped NCC scan lease>")
            .finish()
    }
}

pub(crate) trait ScopedNccHistoryScanPort {
    fn observe_scoped_ncc_history(
        &mut self,
        request: ScopedNccHistoryScanRequest<'_>,
    ) -> Result<ScopedNccHistoryScanCompletion, RepositoryContractError>;
}

/// The complete input retained whenever a scoped NCC scan cannot establish its
/// neutral authority. It is deliberately owning and non-wire: a later phase
/// may classify the failure, but cannot reconstruct an accepted scan from it.
#[derive(Debug)]
pub(crate) struct ScopedNccHistoryScanFailureSource {
    partition: ValidatedRepositoryHistoryPartition,
    expected_capability_id: CapabilityRowId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScopedNccHistoryScanSourceFailureStage {
    MissingOrderEvidence,
    InvalidCoverage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScopedNccHistoryScanCompletionFailureStage {
    ForeignCompletion,
    ForeignLeaseWitness,
    LeaseBindingMismatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScopedNccHistoryScanObservationFailureStage {
    ForeignBatchWitness,
    FromExclusiveMismatch,
    ThroughInclusiveMismatch,
    PartitionDigestMismatch,
    RowCountMismatch,
    RowWitnessMismatch,
    RowPositionMismatch,
    RowCursorMismatch,
    RowVersionMismatch,
    MissingNccFacts,
    UnexpectedNccFacts,
    NccFactsMismatch,
}

/// Stage-exact owning evidence for a failed scoped NCC scan. Completion owns
/// its lease until the completion checks pass; once the lease is consumed, the
/// observation owns the entire batch and every row that was actually observed.
#[derive(Debug)]
pub(crate) enum ScopedNccHistoryScanFailureEvidence {
    InvalidSource {
        stage: ScopedNccHistoryScanSourceFailureStage,
        error: RepositoryContractError,
    },
    Port {
        error: RepositoryContractError,
    },
    Completion {
        stage: ScopedNccHistoryScanCompletionFailureStage,
        error: RepositoryContractError,
        completion: ScopedNccHistoryScanCompletion,
    },
    Observation {
        stage: ScopedNccHistoryScanObservationFailureStage,
        error: RepositoryContractError,
        observation: ScopedNccHistoryScanObservation,
    },
}

/// A blocked scoped NCC scan retains all source and stage-appropriate evidence.
/// Slice 5b1 intentionally exposes no getters or recovery constructors. A
/// later phase-specific owner may add a consuming path, but cannot reconstruct
/// an accepted scan from this opaque blocked value.
#[derive(Debug)]
pub(crate) struct ScopedNccHistoryScanBlockedAuthority {
    source: ScopedNccHistoryScanFailureSource,
    evidence: ScopedNccHistoryScanFailureEvidence,
}

impl ScopedNccHistoryScanBlockedAuthority {
    fn invalid_source(
        source: ScopedNccHistoryScanFailureSource,
        stage: ScopedNccHistoryScanSourceFailureStage,
        error: RepositoryContractError,
    ) -> Self {
        Self {
            source,
            evidence: ScopedNccHistoryScanFailureEvidence::InvalidSource { stage, error },
        }
    }

    fn port(source: ScopedNccHistoryScanFailureSource, error: RepositoryContractError) -> Self {
        Self {
            source,
            evidence: ScopedNccHistoryScanFailureEvidence::Port { error },
        }
    }

    fn completion(
        source: ScopedNccHistoryScanFailureSource,
        stage: ScopedNccHistoryScanCompletionFailureStage,
        error: RepositoryContractError,
        completion: ScopedNccHistoryScanCompletion,
    ) -> Self {
        Self {
            source,
            evidence: ScopedNccHistoryScanFailureEvidence::Completion {
                stage,
                error,
                completion,
            },
        }
    }

    fn observation(
        source: ScopedNccHistoryScanFailureSource,
        stage: ScopedNccHistoryScanObservationFailureStage,
        error: RepositoryContractError,
        observation: ScopedNccHistoryScanObservation,
    ) -> Self {
        Self {
            source,
            evidence: ScopedNccHistoryScanFailureEvidence::Observation {
                stage,
                error,
                observation,
            },
        }
    }
}

/// Neutral, capability-backed full-history NCC scan set. This authority says
/// only that every NCC row in one exact validated partition was observed under
/// the requested atomic-safety capability; Slice 5b decides whether a later
/// precommit or completion boundary may consume it.
#[derive(Debug)]
pub(crate) struct ScopedNccHistoryScanAuthority {
    partition: ValidatedRepositoryHistoryPartition,
    expected_capability_id: CapabilityRowId,
    rows: Vec<ScopedNccHistoryRowObservation>,
    ncc_entry_count: usize,
}

impl ScopedNccHistoryScanAuthority {
    pub(crate) fn resolve(
        partition: ValidatedRepositoryHistoryPartition,
        expected_capability_id: CapabilityRowId,
        port: &mut dyn ScopedNccHistoryScanPort,
    ) -> Result<Self, Box<ScopedNccHistoryScanBlockedAuthority>> {
        let source = ScopedNccHistoryScanFailureSource {
            partition,
            expected_capability_id,
        };
        let ncc_entry_count = source
            .partition
            .classifications()
            .filter(|classification| {
                *classification
                    == RepositoryHistoryPartitionClassification::NonConflictingConcurrent
            })
            .count();
        let Some(order) = source.partition.order_evidence.as_ref() else {
            return Err(Box::new(
                ScopedNccHistoryScanBlockedAuthority::invalid_source(
                    source,
                    ScopedNccHistoryScanSourceFailureStage::MissingOrderEvidence,
                    RepositoryContractError(
                        "scoped NCC scan requires non-empty validated history order",
                    ),
                ),
            ));
        };
        if ncc_entry_count == 0
            || source.partition.entry_count() != order.ordered_cursors.len()
            || order.from_exclusive != *source.partition.start_cursor()
            || order.through_inclusive != *source.partition.through_inclusive()
        {
            return Err(Box::new(
                ScopedNccHistoryScanBlockedAuthority::invalid_source(
                    source,
                    ScopedNccHistoryScanSourceFailureStage::InvalidCoverage,
                    RepositoryContractError(
                        "scoped NCC scan requires exact non-empty NCC history coverage",
                    ),
                ),
            ));
        }

        let invocation = ScopedNccHistoryScanInvocationCapability::mint();
        let completion = match port.observe_scoped_ncc_history(ScopedNccHistoryScanRequest::new(
            &source.partition,
            &source.expected_capability_id,
            &invocation,
        )) {
            Ok(completion) => completion,
            Err(error) => {
                return Err(Box::new(ScopedNccHistoryScanBlockedAuthority::port(
                    source, error,
                )));
            }
        };
        let completion_failure = {
            let request = ScopedNccHistoryScanRequest::new(
                &source.partition,
                &source.expected_capability_id,
                &invocation,
            );
            if !invocation.owns_completion(&completion.completion) {
                Some((
                    ScopedNccHistoryScanCompletionFailureStage::ForeignCompletion,
                    RepositoryContractError("scoped NCC scan completion is foreign to the request"),
                ))
            } else if !invocation.owns_batch_witness(completion.lease.batch_witness()) {
                Some((
                    ScopedNccHistoryScanCompletionFailureStage::ForeignLeaseWitness,
                    RepositoryContractError("scoped NCC scan lease is foreign to the request"),
                ))
            } else if !completion.lease.binds(&request) {
                Some((
                    ScopedNccHistoryScanCompletionFailureStage::LeaseBindingMismatch,
                    RepositoryContractError("scoped NCC scan lease does not bind the request"),
                ))
            } else {
                None
            }
        };
        if let Some((stage, error)) = completion_failure {
            return Err(Box::new(ScopedNccHistoryScanBlockedAuthority::completion(
                source, stage, error, completion,
            )));
        }
        let observation = completion.lease.into_observation();
        let observation_failure = if !invocation.owns_batch_witness(&observation.batch_witness) {
            Some((
                ScopedNccHistoryScanObservationFailureStage::ForeignBatchWitness,
                RepositoryContractError("scoped NCC scan batch witness is foreign to the request"),
            ))
        } else if observation.from_exclusive != *source.partition.start_cursor() {
            Some((
                ScopedNccHistoryScanObservationFailureStage::FromExclusiveMismatch,
                RepositoryContractError(
                    "scoped NCC scan batch start differs from the validated partition",
                ),
            ))
        } else if observation.through_inclusive != *source.partition.through_inclusive() {
            Some((
                ScopedNccHistoryScanObservationFailureStage::ThroughInclusiveMismatch,
                RepositoryContractError(
                    "scoped NCC scan batch end differs from the validated partition",
                ),
            ))
        } else if observation.partition_digest != *source.partition.partition_digest() {
            Some((
                ScopedNccHistoryScanObservationFailureStage::PartitionDigestMismatch,
                RepositoryContractError(
                    "scoped NCC scan batch digest differs from the validated partition",
                ),
            ))
        } else if observation.rows.len() != source.partition.entry_count() {
            Some((
                ScopedNccHistoryScanObservationFailureStage::RowCountMismatch,
                RepositoryContractError(
                    "scoped NCC scan batch row count differs from the validated partition",
                ),
            ))
        } else {
            None
        };
        if let Some((stage, error)) = observation_failure {
            return Err(Box::new(ScopedNccHistoryScanBlockedAuthority::observation(
                source,
                stage,
                error,
                observation,
            )));
        }

        for (index, ((entry, cursor), observed)) in source
            .partition
            .wire
            .entries
            .0
            .iter()
            .zip(order.ordered_cursors.iter())
            .zip(observation.rows.iter())
            .enumerate()
        {
            let row_failure = if !invocation.owns_batch_witness(&observed.batch_witness) {
                Some((
                    ScopedNccHistoryScanObservationFailureStage::RowWitnessMismatch,
                    RepositoryContractError("scoped NCC row witness is foreign to the request"),
                ))
            } else if observed.position != index + 1 {
                Some((
                    ScopedNccHistoryScanObservationFailureStage::RowPositionMismatch,
                    RepositoryContractError(
                        "scoped NCC row position differs from its exact ordered history position",
                    ),
                ))
            } else if &observed.cursor != cursor
                || observed.cursor.through_version() != entry.repository_version()
            {
                Some((
                    ScopedNccHistoryScanObservationFailureStage::RowCursorMismatch,
                    RepositoryContractError(
                        "scoped NCC row cursor differs from its exact ordered history position",
                    ),
                ))
            } else if &observed.repository_version != entry.repository_version() {
                Some((
                    ScopedNccHistoryScanObservationFailureStage::RowVersionMismatch,
                    RepositoryContractError(
                        "scoped NCC row version differs from its exact ordered history position",
                    ),
                ))
            } else {
                None
            };
            if let Some((stage, error)) = row_failure {
                return Err(Box::new(ScopedNccHistoryScanBlockedAuthority::observation(
                    source,
                    stage,
                    error,
                    observation,
                )));
            }
            match (entry, observed.facts.as_ref()) {
                (RepositoryHistoryPartitionEntry::NonConflicting(entry), Some(facts)) => {
                    if let Err(error) = validate_scoped_ncc_row(
                        entry,
                        facts,
                        &source.expected_capability_id,
                        &observed.repository_version,
                    ) {
                        return Err(Box::new(ScopedNccHistoryScanBlockedAuthority::observation(
                            source,
                            ScopedNccHistoryScanObservationFailureStage::NccFactsMismatch,
                            error,
                            observation,
                        )));
                    }
                }
                (RepositoryHistoryPartitionEntry::NonConflicting(_), None) => {
                    return Err(Box::new(ScopedNccHistoryScanBlockedAuthority::observation(
                        source,
                        ScopedNccHistoryScanObservationFailureStage::MissingNccFacts,
                        RepositoryContractError(
                            "scoped NCC history position lacks its detailed observation",
                        ),
                        observation,
                    )));
                }
                (
                    RepositoryHistoryPartitionEntry::EvidenceBacked(_)
                    | RepositoryHistoryPartitionEntry::TaskCommit(_),
                    Some(_),
                ) => {
                    return Err(Box::new(ScopedNccHistoryScanBlockedAuthority::observation(
                        source,
                        ScopedNccHistoryScanObservationFailureStage::UnexpectedNccFacts,
                        RepositoryContractError("non-NCC history position carries NCC facts"),
                        observation,
                    )));
                }
                (
                    RepositoryHistoryPartitionEntry::EvidenceBacked(_)
                    | RepositoryHistoryPartitionEntry::TaskCommit(_),
                    None,
                ) => {}
            }
        }

        let ScopedNccHistoryScanFailureSource {
            partition,
            expected_capability_id,
        } = source;
        Ok(Self {
            partition,
            expected_capability_id,
            rows: observation.rows,
            ncc_entry_count,
        })
    }

    pub(crate) const fn partition(&self) -> &ValidatedRepositoryHistoryPartition {
        &self.partition
    }

    pub(crate) fn history_entry_count(&self) -> usize {
        self.rows.len()
    }

    pub(crate) const fn ncc_entry_count(&self) -> usize {
        self.ncc_entry_count
    }

    pub(crate) const fn expected_capability_id(&self) -> &CapabilityRowId {
        &self.expected_capability_id
    }
}

fn validate_scoped_ncc_row(
    entry: &NonConflictingHistoryPartitionEntry,
    facts: &ScopedNccRowFacts,
    expected_capability_id: &CapabilityRowId,
    observed_repository_version: &RepositoryVersion,
) -> Result<(), RepositoryContractError> {
    let evidence = &entry.non_conflicting_concurrent_evidence;
    let locked_target_set_digest = facts.target_sets.locked_targets.digest()?;
    let changed_target_set_digest = facts.target_sets.changed_targets.digest()?;
    let before_reference_closure_digest = facts.reference_sets.before_reference_closure.digest()?;
    let after_reference_closure_digest = facts.reference_sets.after_reference_closure.digest()?;
    let added_reference_edge_set_digest = facts.reference_sets.added_reference_edges.digest()?;
    if evidence.repository_version != *observed_repository_version
        || evidence.reason != NonConflictingConcurrentReason::HarmlessNonBlockingReferenceExpansion
        || evidence.atomic_commit_safety_capability_id != *expected_capability_id
        || facts.atomic_commit_safety_capability_id != *expected_capability_id
        || evidence.atomic_commit_safety_capability_id != facts.atomic_commit_safety_capability_id
        || evidence.locked_target_set_digest != locked_target_set_digest
        || evidence.changed_object_set_digest != changed_target_set_digest
        || evidence.before_reference_closure_digest != before_reference_closure_digest
        || evidence.after_reference_closure_digest != after_reference_closure_digest
        || evidence.added_reference_edge_set_digest != added_reference_edge_set_digest
    {
        return Err(RepositoryContractError(
            "scoped NCC facts differ from the audited evidence record",
        ));
    }

    let exact_non_empty_addition = reference_delta_is_exact_non_empty_addition(
        &facts.reference_sets.before_reference_closure,
        &facts.reference_sets.after_reference_closure,
        &facts.reference_sets.added_reference_edges,
    );
    let changed_referrers_are_exact = changed_targets_are_exact_added_edge_referrers(
        &facts.target_sets.changed_targets,
        &facts.reference_sets.added_reference_edges,
    );
    let changed_targets_and_referrers_are_development_objects =
        changed_targets_and_added_referrers_are_development_objects(
            &facts.target_sets.changed_targets,
            &facts.reference_sets.added_reference_edges,
        );
    let disjoint_from_integration = facts
        .target_sets
        .integration_content_targets
        .as_canonical()
        .is_disjoint(facts.target_sets.changed_targets.as_canonical());
    let changed_disjoint_from_locked = facts
        .target_sets
        .changed_targets
        .as_canonical()
        .is_disjoint(facts.target_sets.locked_targets.as_canonical());
    let support_graph_unchanged =
        facts.reference_sets.before_support_graph == facts.reference_sets.after_support_graph;
    let validation_inputs_unaffected =
        facts.states.before_validation_inputs == facts.states.after_validation_inputs;
    let root_unchanged = exact_root_state_set(&facts.states.before_root_states)
        && facts.states.before_root_states == facts.states.after_root_states;
    let locked_states_cover_exact_targets = target_states_cover_exact_targets(
        &facts.states.before_locked_target_states,
        &facts.target_sets.locked_targets,
    ) && target_states_cover_exact_targets(
        &facts.states.after_locked_target_states,
        &facts.target_sets.locked_targets,
    );
    let locked_targets_unchanged = changed_disjoint_from_locked
        && locked_states_cover_exact_targets
        && facts.states.before_locked_target_states == facts.states.after_locked_target_states;
    let blocks_approved_deletion = facts
        .reference_sets
        .added_reference_edges
        .as_slice()
        .iter()
        .any(|edge| {
            facts
                .target_sets
                .approved_deletion_targets
                .contains(&edge.referenced_target)
        });

    if !(exact_non_empty_addition
        && changed_referrers_are_exact
        && changed_targets_and_referrers_are_development_objects
        && evidence.closure_delta_only_adds_non_blocking_references == TrueLiteral
        && disjoint_from_integration
        && evidence.disjoint_from_integration_content == TrueLiteral
        && support_graph_unchanged
        && evidence.support_graph_unchanged == TrueLiteral
        && validation_inputs_unaffected
        && evidence.validation_inputs_unaffected == TrueLiteral
        && root_unchanged
        && evidence.root_unchanged == TrueLiteral
        && locked_targets_unchanged
        && evidence.locked_targets_unchanged == TrueLiteral
        && !blocks_approved_deletion
        && evidence.blocks_approved_deletion == FalseLiteral)
    {
        return Err(RepositoryContractError(
            "scoped NCC observations do not derive every audited safety literal",
        ));
    }
    Ok(())
}

fn reference_delta_is_exact_non_empty_addition(
    before: &CanonicalRepositoryReferenceEdgeSet,
    after: &CanonicalRepositoryReferenceEdgeSet,
    added: &CanonicalRepositoryReferenceEdgeSet,
) -> bool {
    let mut before_index = 0;
    let mut after_index = 0;
    let mut added_index = 0;
    while before_index < before.as_slice().len() && after_index < after.as_slice().len() {
        match before.as_slice()[before_index].cmp(&after.as_slice()[after_index]) {
            Ordering::Equal => {
                before_index += 1;
                after_index += 1;
            }
            Ordering::Less => return false,
            Ordering::Greater => {
                if added.as_slice().get(added_index) != after.as_slice().get(after_index) {
                    return false;
                }
                added_index += 1;
                after_index += 1;
            }
        }
    }
    if before_index != before.as_slice().len() {
        return false;
    }
    while after_index < after.as_slice().len() {
        if added.as_slice().get(added_index) != after.as_slice().get(after_index) {
            return false;
        }
        added_index += 1;
        after_index += 1;
    }
    added_index > 0 && added_index == added.as_slice().len()
}

fn changed_targets_are_exact_added_edge_referrers(
    changed_targets: &NonEmptyCanonicalRepositoryTargetSet,
    added_edges: &CanonicalRepositoryReferenceEdgeSet,
) -> bool {
    let mut changed = changed_targets.as_slice().iter();
    let mut expected = changed.next();
    let mut previous_referrer: Option<&RepositoryTargetIdentity> = None;
    for edge in added_edges.as_slice() {
        if previous_referrer == Some(&edge.referencing_target) {
            continue;
        }
        if expected != Some(&edge.referencing_target) {
            return false;
        }
        previous_referrer = Some(&edge.referencing_target);
        expected = changed.next();
    }
    previous_referrer.is_some() && expected.is_none()
}

fn changed_targets_and_added_referrers_are_development_objects(
    changed_targets: &NonEmptyCanonicalRepositoryTargetSet,
    added_edges: &CanonicalRepositoryReferenceEdgeSet,
) -> bool {
    changed_targets
        .as_slice()
        .iter()
        .all(|target| matches!(target, RepositoryTargetIdentity::DevelopmentObject(_)))
        && added_edges.as_slice().iter().all(|edge| {
            matches!(
                &edge.referencing_target,
                RepositoryTargetIdentity::DevelopmentObject(_)
            )
        })
}

fn locked_repository_target_state_identity(
    state: &RepositoryTargetState,
) -> Option<RepositoryTargetIdentity> {
    match state {
        RepositoryTargetState::RootPresent(_) => {
            Some(RepositoryTargetIdentity::configuration_root())
        }
        RepositoryTargetState::ObjectPresent(value) => Some(
            RepositoryTargetIdentity::development_object(value.object_id.clone()),
        ),
        RepositoryTargetState::ObjectAbsent(_) => None,
    }
}

fn target_states_cover_exact_targets(
    states: &RepositoryTargetStates,
    targets: &NonEmptyCanonicalRepositoryTargetSet,
) -> bool {
    states.as_slice().len() == targets.as_slice().len()
        && states
            .as_slice()
            .iter()
            .zip(targets.as_slice())
            .all(|(state, target)| {
                locked_repository_target_state_identity(state).as_ref() == Some(target)
            })
}

fn exact_root_state_set(states: &RepositoryTargetStates) -> bool {
    matches!(states.as_slice(), [RepositoryTargetState::RootPresent(_)])
}

#[cfg(test)]
pub(crate) fn repository_history_partition_fixture_test_only(
    from_exclusive: RepositoryHistoryCursor,
    entries: Vec<(RepositoryVersion, RepositoryHistoryPartitionClassification)>,
    order_capability_id: &str,
    atomic_commit_safety_capability_id: &str,
) -> Result<ValidatedRepositoryHistoryPartition, RepositoryContractError> {
    let mut ordered_cursors = Vec::with_capacity(entries.len());
    let mut wire_entries = Vec::with_capacity(entries.len());
    for (index, (repository_version, classification)) in entries.into_iter().enumerate() {
        let cursor_character = char::from_digit(((index + 1) % 15 + 1) as u32, 16)
            .expect("fixture cursor digit must be hexadecimal");
        let semantic_character = char::from_digit(((index + 6) % 15 + 1) as u32, 16)
            .expect("fixture semantic digit must be hexadecimal");
        let evidence_character = char::from_digit(((index + 10) % 15 + 1) as u32, 16)
            .expect("fixture evidence digit must be hexadecimal");
        let cursor = RepositoryHistoryCursor::new(
            repository_version.clone(),
            Sha256Digest::parse(&cursor_character.to_string().repeat(64))
                .expect("fixture cursor digest must be valid"),
        );
        let semantic_delta_digest = Sha256Digest::parse(&semantic_character.to_string().repeat(64))
            .expect("fixture semantic digest must be valid");
        let evidence_digest = evidence_character.to_string().repeat(64);
        let entry = match classification {
            RepositoryHistoryPartitionClassification::UnrelatedRoutine
            | RepositoryHistoryPartitionClassification::RelevantRoutine
            | RepositoryHistoryPartitionClassification::AuthorizedSupport
            | RepositoryHistoryPartitionClassification::ExternalSupport
            | RepositoryHistoryPartitionClassification::PreArmExternal
            | RepositoryHistoryPartitionClassification::Invalid
            | RepositoryHistoryPartitionClassification::Corrective => {
                let classification = match classification {
                    RepositoryHistoryPartitionClassification::UnrelatedRoutine => {
                        EvidenceBackedPartitionClassification::UnrelatedRoutine
                    }
                    RepositoryHistoryPartitionClassification::RelevantRoutine => {
                        EvidenceBackedPartitionClassification::RelevantRoutine
                    }
                    RepositoryHistoryPartitionClassification::AuthorizedSupport => {
                        EvidenceBackedPartitionClassification::AuthorizedSupport
                    }
                    RepositoryHistoryPartitionClassification::ExternalSupport => {
                        EvidenceBackedPartitionClassification::ExternalSupport
                    }
                    RepositoryHistoryPartitionClassification::PreArmExternal => {
                        EvidenceBackedPartitionClassification::PreArmExternal
                    }
                    RepositoryHistoryPartitionClassification::Invalid => {
                        EvidenceBackedPartitionClassification::Invalid
                    }
                    RepositoryHistoryPartitionClassification::Corrective => {
                        EvidenceBackedPartitionClassification::Corrective
                    }
                    RepositoryHistoryPartitionClassification::NonConflictingConcurrent
                    | RepositoryHistoryPartitionClassification::TaskCommit => unreachable!(),
                };
                RepositoryHistoryPartitionEntry::EvidenceBacked(
                    EvidenceBackedHistoryPartitionEntry {
                        repository_version,
                        classification,
                        semantic_delta_digest,
                        source_evidence_ref: RepositoryHistorySourceEvidenceRef::new(
                            EvidenceKind::RoutineClassification,
                            &evidence_digest,
                        )?,
                    },
                )
            }
            RepositoryHistoryPartitionClassification::NonConflictingConcurrent => {
                RepositoryHistoryPartitionEntry::NonConflicting(
                    NonConflictingHistoryPartitionEntry {
                        non_conflicting_concurrent_evidence: NonConflictingConcurrentEvidence::new(
                            repository_version.as_str(),
                            atomic_commit_safety_capability_id,
                            &"1".repeat(64),
                            &"2".repeat(64),
                            &"3".repeat(64),
                            &"4".repeat(64),
                            &"5".repeat(64),
                        )?,
                        repository_version,
                        classification: NonConflictingClassification::Value,
                        semantic_delta_digest,
                        source_evidence_ref: RepositoryHistorySourceEvidenceRef::new(
                            EvidenceKind::NonConflictingConcurrent,
                            &evidence_digest,
                        )?,
                    },
                )
            }
            RepositoryHistoryPartitionClassification::TaskCommit => {
                RepositoryHistoryPartitionEntry::TaskCommit(TaskCommitHistoryPartitionEntry {
                    repository_version,
                    classification: TaskCommitClassification::Value,
                    semantic_delta_digest,
                })
            }
        };
        ordered_cursors.push(cursor);
        wire_entries.push(entry);
    }
    let through_inclusive = ordered_cursors
        .last()
        .cloned()
        .unwrap_or_else(|| from_exclusive.clone());
    let entries = UnvalidatedRepositoryHistoryEntries(wire_entries);
    let partition_digest = canonical_contract_digest(
        &RepositoryHistoryPartitionDigestRecord {
            from_exclusive: from_exclusive.clone(),
            through_inclusive: through_inclusive.clone(),
            entries: entries.clone(),
        },
        None,
    )
    .map_err(|_| RepositoryContractError("history fixture partition digest failed"))?;
    let order_evidence = (!ordered_cursors.is_empty())
        .then(|| {
            RepositoryHistoryOrderEvidence::from_capability_adapter(
                order_capability_id,
                from_exclusive.clone(),
                through_inclusive.clone(),
                ordered_cursors,
            )
        })
        .transpose()?;
    let source_index_proofs = (0..entries.0.len()).map(|_| None).collect();
    Ok(ValidatedRepositoryHistoryPartition {
        wire: UnvalidatedRepositoryHistoryPartition {
            from_exclusive,
            through_inclusive,
            entries,
            partition_digest,
        },
        source_index_proofs,
        order_evidence,
    })
}

/// Test-only adversarial fixture for the otherwise validated partition shape.
/// Production constructors never remove history-order evidence once validation
/// has succeeded.
#[cfg(test)]
pub(crate) fn repository_history_partition_without_order_evidence_fixture_test_only(
    mut partition: ValidatedRepositoryHistoryPartition,
) -> ValidatedRepositoryHistoryPartition {
    partition.order_evidence = None;
    partition
}

/// Adversarial result-layer fixture. Production taskCommit authorities are
/// minted only by `RepositoryHistoryPartitionResolver`; this helper can forge
/// malformed shapes so the completion boundary remains defense in depth.
#[cfg(test)]
pub(crate) fn task_commit_history_partition_fixture_test_only(
    mut partition: ValidatedRepositoryHistoryPartition,
    binding: &(impl CommitObjectHistoryBinding + ?Sized),
    task_semantic_delta_digest: Sha256Digest,
) -> Result<ValidatedTaskCommitHistoryPartition, RepositoryContractError> {
    for entry in &mut partition.wire.entries.0 {
        if let RepositoryHistoryPartitionEntry::TaskCommit(task_commit) = entry {
            task_commit.semantic_delta_digest = task_semantic_delta_digest.clone();
        }
    }
    partition.wire.partition_digest = canonical_contract_digest(
        &RepositoryHistoryPartitionDigestRecord {
            from_exclusive: partition.wire.from_exclusive.clone(),
            through_inclusive: partition.wire.through_inclusive.clone(),
            entries: partition.wire.entries.clone(),
        },
        None,
    )
    .map_err(|_| RepositoryContractError("task-commit fixture digest failed"))?;
    Ok(ValidatedTaskCommitHistoryPartition {
        partition,
        object_history_binding_witness: binding.object_history_binding_witness().clone(),
        repository_version: binding.repository_version().clone(),
        committed_objects_digest: binding.committed_objects_digest().clone(),
        atomic_commit_safety_capability_id: binding.atomic_commit_safety_capability_id().clone(),
    })
}

pub(crate) struct RepositoryHistoryPartitionResolver<'a> {
    registry: &'a EvidenceSourceRegistry,
    source_index: &'a dyn EvidenceSourceIndex,
    order_resolver: &'a dyn RepositoryHistoryOrderResolver,
    evidence_resolver: &'a dyn RepositoryHistoryEvidenceBytesResolver,
    corrective_resolver: Option<&'a dyn SupportCorrectiveEvidenceResolver>,
}

impl<'a> RepositoryHistoryPartitionResolver<'a> {
    pub(crate) const fn new(
        registry: &'a EvidenceSourceRegistry,
        source_index: &'a dyn EvidenceSourceIndex,
        order_resolver: &'a dyn RepositoryHistoryOrderResolver,
        evidence_resolver: &'a dyn RepositoryHistoryEvidenceBytesResolver,
    ) -> Self {
        Self {
            registry,
            source_index,
            order_resolver,
            evidence_resolver,
            corrective_resolver: None,
        }
    }

    pub(crate) fn with_corrective_evidence_resolver(
        mut self,
        corrective_resolver: &'a dyn SupportCorrectiveEvidenceResolver,
    ) -> Self {
        self.corrective_resolver = Some(corrective_resolver);
        self
    }

    pub(crate) fn validate(
        &self,
        wire: UnvalidatedRepositoryHistoryPartition,
    ) -> Result<ValidatedRepositoryHistoryPartition, RepositoryContractError> {
        self.registry.verify_committed_artifacts()?;
        let expected_partition_digest = canonical_contract_digest(
            &RepositoryHistoryPartitionDigestRecord {
                from_exclusive: wire.from_exclusive.clone(),
                through_inclusive: wire.through_inclusive.clone(),
                entries: wire.entries.clone(),
            },
            None,
        )
        .map_err(|_| RepositoryContractError("partition digest failed"))?;
        if expected_partition_digest != wire.partition_digest {
            return Err(RepositoryContractError("partition digest mismatch"));
        }

        if wire.entries.0.is_empty() {
            if wire.from_exclusive != wire.through_inclusive {
                return Err(RepositoryContractError(
                    "empty partition requires byte-identical endpoints",
                ));
            }
            return Ok(ValidatedRepositoryHistoryPartition {
                wire,
                source_index_proofs: Vec::new(),
                order_evidence: None,
            });
        }
        if wire.from_exclusive == wire.through_inclusive {
            return Err(RepositoryContractError(
                "equal-endpoint partition must be empty",
            ));
        }
        if wire
            .entries
            .0
            .iter()
            .any(|entry| matches!(entry, RepositoryHistoryPartitionEntry::TaskCommit(_)))
        {
            return Err(RepositoryContractError(
                "generic repository-history validator rejects taskCommit",
            ));
        }

        let order_evidence = self
            .order_resolver
            .order_evidence(&wire.from_exclusive, &wire.through_inclusive)?;
        let entry_versions: Vec<_> = wire
            .entries
            .0
            .iter()
            .map(|entry| entry.repository_version().clone())
            .collect();
        let unique_versions: HashSet<_> = entry_versions.iter().collect();
        if order_evidence.from_exclusive != wire.from_exclusive
            || order_evidence.through_inclusive != wire.through_inclusive
            || order_evidence.ordered_versions != entry_versions
            || order_evidence.ordered_cursors.len() != entry_versions.len()
            || order_evidence
                .ordered_cursors
                .iter()
                .map(|cursor| &cursor.through_version)
                .ne(entry_versions.iter())
            || order_evidence.ordered_cursors.last() != Some(&wire.through_inclusive)
            || unique_versions.len() != entry_versions.len()
            || entry_versions.last() != Some(&wire.through_inclusive.through_version)
        {
            return Err(RepositoryContractError(
                "history-order evidence does not prove exact partition coverage",
            ));
        }

        let mut source_index_proofs = Vec::with_capacity(wire.entries.0.len());
        for entry in &wire.entries.0 {
            let candidate = self
                .source_index
                .candidate_for(entry.repository_version(), self.registry)?;
            let mut proof = EvidenceSourceIndexProof::from_candidate(
                candidate,
                entry.repository_version(),
                self.registry,
            )?;
            proof.validated_support_mapping = self.validate_entry(entry, &proof)?;
            source_index_proofs.push(Some(proof));
        }

        Ok(ValidatedRepositoryHistoryPartition {
            wire,
            source_index_proofs,
            order_evidence: Some(order_evidence),
        })
    }

    /// Validate the one history partition whose taskCommit entry is owned by
    /// the enclosing committed-object authority. No wire-only or generic
    /// path can select this branch.
    pub(crate) fn validate_task_commit_partition(
        &self,
        wire: UnvalidatedRepositoryHistoryPartition,
        commit: &(impl CommitObjectHistoryBinding + ?Sized),
    ) -> Result<ValidatedTaskCommitHistoryPartition, RepositoryContractError> {
        self.registry.verify_committed_artifacts()?;
        let expected_partition_digest = canonical_contract_digest(
            &RepositoryHistoryPartitionDigestRecord {
                from_exclusive: wire.from_exclusive.clone(),
                through_inclusive: wire.through_inclusive.clone(),
                entries: wire.entries.clone(),
            },
            None,
        )
        .map_err(|_| RepositoryContractError("task-commit partition digest failed"))?;
        if expected_partition_digest != wire.partition_digest {
            return Err(RepositoryContractError(
                "task-commit partition digest mismatch",
            ));
        }
        if wire.entries.0.is_empty() || wire.from_exclusive == wire.through_inclusive {
            return Err(RepositoryContractError(
                "task-commit partition requires a non-empty repository range",
            ));
        }

        let mut task_entries = wire.entries.0.iter().filter_map(|entry| match entry {
            RepositoryHistoryPartitionEntry::TaskCommit(value) => Some(value),
            RepositoryHistoryPartitionEntry::EvidenceBacked(_)
            | RepositoryHistoryPartitionEntry::NonConflicting(_) => None,
        });
        let Some(task_entry) = task_entries.next() else {
            return Err(RepositoryContractError(
                "task-commit partition lacks its task version",
            ));
        };
        if task_entries.next().is_some()
            || &task_entry.repository_version != commit.repository_version()
            || &task_entry.semantic_delta_digest != commit.committed_objects_digest()
        {
            return Err(RepositoryContractError(
                "task-commit partition task version or semantic digest mismatch",
            ));
        }
        for entry in &wire.entries.0 {
            match entry {
                RepositoryHistoryPartitionEntry::TaskCommit(_) => {}
                RepositoryHistoryPartitionEntry::EvidenceBacked(value)
                    if value.classification
                        == EvidenceBackedPartitionClassification::UnrelatedRoutine => {}
                RepositoryHistoryPartitionEntry::NonConflicting(_) => {
                    return Err(RepositoryContractError(
                        "Slice 3 task-commit partition rejects concurrent history",
                    ));
                }
                RepositoryHistoryPartitionEntry::EvidenceBacked(_) => {
                    return Err(RepositoryContractError(
                        "task-commit partition contains relevant or unproven concurrent history",
                    ));
                }
            }
        }

        let order_evidence = self
            .order_resolver
            .order_evidence(&wire.from_exclusive, &wire.through_inclusive)?;
        let entry_versions: Vec<_> = wire
            .entries
            .0
            .iter()
            .map(|entry| entry.repository_version().clone())
            .collect();
        let unique_versions: HashSet<_> = entry_versions.iter().collect();
        if order_evidence.from_exclusive != wire.from_exclusive
            || order_evidence.through_inclusive != wire.through_inclusive
            || order_evidence.ordered_versions != entry_versions
            || order_evidence.ordered_cursors.len() != entry_versions.len()
            || order_evidence
                .ordered_cursors
                .iter()
                .map(|cursor| &cursor.through_version)
                .ne(entry_versions.iter())
            || order_evidence.ordered_cursors.last() != Some(&wire.through_inclusive)
            || unique_versions.len() != entry_versions.len()
            || entry_versions.last() != Some(&wire.through_inclusive.through_version)
        {
            return Err(RepositoryContractError(
                "history-order evidence does not prove exact task-commit partition coverage",
            ));
        }

        let mut source_index_proofs = Vec::with_capacity(wire.entries.0.len());
        for entry in &wire.entries.0 {
            if matches!(entry, RepositoryHistoryPartitionEntry::TaskCommit(_)) {
                source_index_proofs.push(None);
                continue;
            }
            let candidate = self
                .source_index
                .candidate_for(entry.repository_version(), self.registry)?;
            let mut proof = EvidenceSourceIndexProof::from_candidate(
                candidate,
                entry.repository_version(),
                self.registry,
            )?;
            proof.validated_support_mapping = self.validate_entry(entry, &proof)?;
            source_index_proofs.push(Some(proof));
        }

        Ok(ValidatedTaskCommitHistoryPartition {
            partition: ValidatedRepositoryHistoryPartition {
                wire,
                source_index_proofs,
                order_evidence: Some(order_evidence),
            },
            object_history_binding_witness: commit.object_history_binding_witness().clone(),
            repository_version: commit.repository_version().clone(),
            committed_objects_digest: commit.committed_objects_digest().clone(),
            atomic_commit_safety_capability_id: commit.atomic_commit_safety_capability_id().clone(),
        })
    }

    fn validate_entry(
        &self,
        entry: &RepositoryHistoryPartitionEntry,
        proof: &EvidenceSourceIndexProof,
    ) -> Result<Option<ValidatedSupportObservationEntryProof>, RepositoryContractError> {
        let support_row = proof
            .row(EvidenceKind::SupportPrerequisiteObservation)
            .ok_or(RepositoryContractError(
                "missing support-observation index row",
            ))?;
        let ncc_row = proof
            .row(EvidenceKind::NonConflictingConcurrent)
            .ok_or(RepositoryContractError("missing concurrent index row"))?;
        match entry {
            RepositoryHistoryPartitionEntry::TaskCommit(_) => Err(RepositoryContractError(
                "generic repository-history validator rejects taskCommit",
            )),
            RepositoryHistoryPartitionEntry::EvidenceBacked(entry) => match support_row {
                EvidenceSourceAvailability::Available(value) => {
                    let selected = &value.source_evidence_ref;
                    if entry.source_evidence_ref != *selected
                        || selected.evidence_kind != EvidenceKind::SupportPrerequisiteObservation
                    {
                        return Err(RepositoryContractError(
                            "support-observation source ref substitution",
                        ));
                    }
                    let observation = match load_history_evidence(selected, self.evidence_resolver)?
                    {
                        ResolvedHistoryEvidence::SupportObservation(value) => value,
                        ResolvedHistoryEvidence::Routine(_)
                        | ResolvedHistoryEvidence::NonConflicting(_) => {
                            return Err(RepositoryContractError(
                                "support-observation source type mismatch",
                            ));
                        }
                    };
                    if observation.repository_version() != &entry.repository_version {
                        return Err(RepositoryContractError(
                            "support-observation version mismatch",
                        ));
                    }
                    let partition_classification =
                        RepositoryHistoryPartitionClassification::from(entry.classification);
                    let (
                        projected_classification,
                        root_delta_digest,
                        content_delta_digest,
                        classification_digest,
                        external_support_disjointness_digest,
                        corrective_instruction_digest,
                        corrective_source_action_id,
                    ) = if let Some(projection) = observation.task8_mapping_projection() {
                        (
                            projection.partition_classification(),
                            projection.root_delta_digest(),
                            projection.content_delta_digest(),
                            projection.classification_digest(),
                            projection.external_support_disjointness_digest(),
                            None,
                            None,
                        )
                    } else {
                        let corrective = observation.task9_corrective_projection().ok_or(
                            RepositoryContractError("unsupported support-observation mapping"),
                        )?;
                        let resolver = self.corrective_resolver.ok_or(RepositoryContractError(
                            "corrective observation lacks historical source authority",
                        ))?;
                        match corrective {
                            SupportObservationCorrectiveProjection::ActionCorrection {
                                repository_actor,
                                manual_target_mode,
                                working_infobase_identity,
                                root_delta_digest,
                                content_delta_digest,
                                corrective_instruction_digest,
                            } => {
                                let source = resolver
                                    .historical_frozen_support_corrective_instruction_source(
                                        &entry.repository_version,
                                    )?;
                                if source.historical_repository_version()
                                    != &entry.repository_version
                                    || source.expected_historical_support_action_id()
                                        != resolver.frozen_support_action_id()
                                {
                                    return Err(RepositoryContractError(
                                        "action-correction historical source authority mismatch",
                                    ));
                                }
                                if &corrective_instruction_digest
                                    != source.frozen_corrective_instruction_digest()
                                {
                                    return Err(RepositoryContractError(
                                        "action-correction observation selected a different historical instruction",
                                    ));
                                }
                                let instruction = load_support_corrective_instruction(&source)?;
                                if instruction.support_action_id()
                                    != resolver.frozen_support_action_id()
                                    || instruction.repository_username()
                                        != repository_actor.username()
                                    || instruction.manual_target_mode() != manual_target_mode
                                    || instruction.working_infobase_identity()
                                        != working_infobase_identity.as_ref()
                                    || instruction.required_root_delta_digest()
                                        != &root_delta_digest
                                    || instruction.required_content_delta_digest()
                                        != &content_delta_digest
                                {
                                    return Err(RepositoryContractError(
                                        "action-correction instruction binding mismatch",
                                    ));
                                }
                                resolver.validate_action_correction_attribution(
                                    &entry.repository_version,
                                    &repository_actor,
                                    &instruction,
                                )?;
                                (
                                    RepositoryHistoryPartitionClassification::Corrective,
                                    Some(root_delta_digest),
                                    Some(content_delta_digest),
                                    observation.classification_digest().clone(),
                                    None,
                                    Some(corrective_instruction_digest),
                                    Some(resolver.frozen_support_action_id().clone()),
                                )
                            }
                            SupportObservationCorrectiveProjection::ExternalConflictCorrection {
                                repository_actor,
                                root_delta_digest,
                                content_delta_digest,
                                conflict_resolution_id,
                                support_conflict_instruction_digest,
                                final_baseline_digest,
                                external_ownership_evidence,
                            } => {
                                let source = resolver
                                    .historical_frozen_support_conflict_instruction_source(
                                        &entry.repository_version,
                                    )?;
                                if source.historical_repository_version()
                                    != &entry.repository_version
                                    || source.expected_historical_support_action_id()
                                        != resolver.frozen_support_action_id()
                                {
                                    return Err(RepositoryContractError(
                                        "external-conflict historical source authority mismatch",
                                    ));
                                }
                                if &support_conflict_instruction_digest
                                    != source.frozen_support_conflict_instruction_digest()
                                    || &conflict_resolution_id
                                        != source.expected_historical_conflict_resolution_id()
                                {
                                    return Err(RepositoryContractError(
                                        "external-conflict observation selected a different historical instruction",
                                    ));
                                }
                                let instruction = load_support_conflict_instruction(
                                    &source,
                                    resolver.support_history_order_authority(),
                                )?;
                                if instruction.conflict_resolution_id()
                                    != &conflict_resolution_id
                                    || instruction.required_final_baseline_digest()
                                        != &final_baseline_digest
                                {
                                    return Err(RepositoryContractError(
                                        "external-conflict instruction binding mismatch",
                                    ));
                                }
                                resolver.validate_external_ownership_attribution(
                                    &entry.repository_version,
                                    &repository_actor,
                                    &root_delta_digest,
                                    &content_delta_digest,
                                    &external_ownership_evidence,
                                )?;
                                (
                                    RepositoryHistoryPartitionClassification::Corrective,
                                    Some(root_delta_digest),
                                    Some(content_delta_digest),
                                    observation.classification_digest().clone(),
                                    None,
                                    Some(support_conflict_instruction_digest),
                                    Some(resolver.frozen_support_action_id().clone()),
                                )
                            }
                        }
                    };
                    if partition_classification != projected_classification {
                        return Err(RepositoryContractError(
                            "support-observation classification mismatch",
                        ));
                    }
                    let semantic = RepositorySemanticDeltaDigestRecord {
                        repository_version: entry.repository_version.clone(),
                        partition_classification: projected_classification,
                        root_delta_digest: root_delta_digest
                            .map(RequiredNullable::value)
                            .unwrap_or_else(RequiredNullable::null),
                        content_delta_digest: content_delta_digest
                            .map(RequiredNullable::value)
                            .unwrap_or_else(RequiredNullable::null),
                        classification_digest: RequiredNullable::value(classification_digest),
                        external_support_disjointness_digest: external_support_disjointness_digest
                            .map(RequiredNullable::value)
                            .unwrap_or_else(RequiredNullable::null),
                        corrective_instruction_digest: corrective_instruction_digest
                            .map(RequiredNullable::value)
                            .unwrap_or_else(RequiredNullable::null),
                        non_conflicting_concurrent_evidence_digest: RequiredNullable::null(),
                    };
                    let expected = canonical_contract_digest(&semantic, None)
                        .map_err(|_| RepositoryContractError("semantic delta digest failed"))?;
                    if expected != entry.semantic_delta_digest {
                        return Err(RepositoryContractError("semantic delta digest mismatch"));
                    }
                    Ok(Some(ValidatedSupportObservationEntryProof {
                        repository_version: entry.repository_version.clone(),
                        partition_classification,
                        semantic_delta_digest: entry.semantic_delta_digest.clone(),
                        source_evidence_ref: entry.source_evidence_ref.clone(),
                        registry_digest: proof.registry_digest.clone(),
                        source_index_proof_digest: proof.proof_digest.clone(),
                        corrective_source_action_id,
                    }))
                }
                EvidenceSourceAvailability::Absent(_) => {
                    if matches!(ncc_row, EvidenceSourceAvailability::Available(_)) {
                        return Err(RepositoryContractError(
                            "available higher-precedence concurrent source cannot fall back",
                        ));
                    }
                    let routine_row = proof
                        .row(EvidenceKind::RoutineClassification)
                        .ok_or(RepositoryContractError("missing routine index row"))?;
                    let selected = match routine_row {
                        EvidenceSourceAvailability::Available(value) => &value.source_evidence_ref,
                        EvidenceSourceAvailability::Absent(_) => {
                            return Err(RepositoryContractError("routine source is absent"));
                        }
                    };
                    if entry.source_evidence_ref != *selected
                        || selected.evidence_kind != EvidenceKind::RoutineClassification
                    {
                        return Err(RepositoryContractError("routine source ref substitution"));
                    }
                    let evidence = match load_history_evidence(selected, self.evidence_resolver)? {
                        ResolvedHistoryEvidence::Routine(value) => value,
                        ResolvedHistoryEvidence::SupportObservation(_)
                        | ResolvedHistoryEvidence::NonConflicting(_) => {
                            return Err(RepositoryContractError("routine source type mismatch"));
                        }
                    };
                    if evidence.repository_version != entry.repository_version {
                        return Err(RepositoryContractError("routine evidence version mismatch"));
                    }
                    let expected_classification = match evidence.relevance {
                        RepositoryRelevance::Unrelated => {
                            EvidenceBackedPartitionClassification::UnrelatedRoutine
                        }
                        RepositoryRelevance::Relevant => {
                            EvidenceBackedPartitionClassification::RelevantRoutine
                        }
                    };
                    if entry.classification != expected_classification {
                        return Err(RepositoryContractError(
                            "routine source classification mismatch",
                        ));
                    }
                    let semantic = RepositorySemanticDeltaDigestRecord {
                        repository_version: entry.repository_version.clone(),
                        partition_classification: entry.classification.into(),
                        root_delta_digest: RequiredNullable::value(evidence.root_delta_digest),
                        content_delta_digest: RequiredNullable::value(
                            evidence.content_delta_digest,
                        ),
                        classification_digest: RequiredNullable::value(
                            evidence.classification_digest,
                        ),
                        external_support_disjointness_digest: RequiredNullable::null(),
                        corrective_instruction_digest: RequiredNullable::null(),
                        non_conflicting_concurrent_evidence_digest: RequiredNullable::null(),
                    };
                    let expected = canonical_contract_digest(&semantic, None)
                        .map_err(|_| RepositoryContractError("semantic delta digest failed"))?;
                    if expected != entry.semantic_delta_digest {
                        return Err(RepositoryContractError("semantic delta digest mismatch"));
                    }
                    Ok(None)
                }
            },
            RepositoryHistoryPartitionEntry::NonConflicting(entry) => {
                if matches!(support_row, EvidenceSourceAvailability::Available(_)) {
                    return Err(RepositoryContractError(
                        "available higher-precedence support source cannot fall back",
                    ));
                }
                let selected = match ncc_row {
                    EvidenceSourceAvailability::Available(value) => &value.source_evidence_ref,
                    EvidenceSourceAvailability::Absent(_) => {
                        return Err(RepositoryContractError("concurrent source is absent"));
                    }
                };
                if entry.source_evidence_ref != *selected
                    || selected.evidence_kind != EvidenceKind::NonConflictingConcurrent
                {
                    return Err(RepositoryContractError(
                        "concurrent source ref substitution",
                    ));
                }
                let evidence = match load_history_evidence(selected, self.evidence_resolver)? {
                    ResolvedHistoryEvidence::NonConflicting(value) => value,
                    ResolvedHistoryEvidence::Routine(_)
                    | ResolvedHistoryEvidence::SupportObservation(_) => {
                        return Err(RepositoryContractError("concurrent source type mismatch"));
                    }
                };
                if evidence.repository_version != entry.repository_version
                    || evidence != entry.non_conflicting_concurrent_evidence
                {
                    return Err(RepositoryContractError(
                        "concurrent inline evidence mismatch",
                    ));
                }
                let semantic = RepositorySemanticDeltaDigestRecord {
                    repository_version: entry.repository_version.clone(),
                    partition_classification:
                        RepositoryHistoryPartitionClassification::NonConflictingConcurrent,
                    root_delta_digest: RequiredNullable::null(),
                    content_delta_digest: RequiredNullable::null(),
                    classification_digest: RequiredNullable::null(),
                    external_support_disjointness_digest: RequiredNullable::null(),
                    corrective_instruction_digest: RequiredNullable::null(),
                    non_conflicting_concurrent_evidence_digest: RequiredNullable::value(
                        evidence.evidence_digest,
                    ),
                };
                let expected = canonical_contract_digest(&semantic, None)
                    .map_err(|_| RepositoryContractError("semantic delta digest failed"))?;
                if expected != entry.semantic_delta_digest {
                    return Err(RepositoryContractError("semantic delta digest mismatch"));
                }
                Ok(None)
            }
        }
    }

    pub(crate) fn late_audit(
        &self,
        validated: &ValidatedRepositoryHistoryPartition,
    ) -> Result<(), RepositoryContractError> {
        let replayed = self.validate(validated.wire.clone())?;
        if &replayed != validated {
            return Err(RepositoryContractError("late history audit mismatch"));
        }
        Ok(())
    }
}

macro_rules! string_literal {
    ($name:ident, $wire:literal) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
        enum $name {
            #[serde(rename = $wire)]
            Value,
        }
    };
}

string_literal!(ConfigurationRootKind, "configurationRoot");
string_literal!(DevelopmentObjectKind, "developmentObject");
string_literal!(PresentState, "present");
string_literal!(AbsentState, "absent");
string_literal!(ModifyAction, "modify");
string_literal!(DeleteAction, "delete");

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub(crate) enum RepositoryTargetKind {
    ConfigurationRoot,
    DevelopmentObject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
enum AddOrModifyAction {
    Add,
    Modify,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RootPresentTargetState {
    target_kind: ConfigurationRootKind,
    state: PresentState,
    repository_version: RepositoryVersion,
    target_fingerprint: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ObjectPresentTargetState {
    target_kind: DevelopmentObjectKind,
    state: PresentState,
    object_id: MetadataObjectId,
    repository_version: RepositoryVersion,
    target_fingerprint: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ObjectAbsentTargetState {
    target_kind: DevelopmentObjectKind,
    state: AbsentState,
    object_id: MetadataObjectId,
    absence_established_at_version: RepositoryVersion,
    expected_absent: TrueLiteral,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum RepositoryTargetState {
    RootPresent(RootPresentTargetState),
    ObjectPresent(ObjectPresentTargetState),
    ObjectAbsent(ObjectAbsentTargetState),
}

/// Borrowed, typed projection used by recovery authority.  Keeping this
/// projection in the repository contract prevents an approval decision from
/// depending on a lossy `serde_json::Value` round trip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RepositoryTargetStateRef<'a> {
    RootPresent {
        repository_version: &'a RepositoryVersion,
        target_fingerprint: &'a Sha256Digest,
    },
    ObjectPresent {
        object_id: &'a MetadataObjectId,
        repository_version: &'a RepositoryVersion,
        target_fingerprint: &'a Sha256Digest,
    },
    ObjectAbsent {
        object_id: &'a MetadataObjectId,
        absence_established_at_version: &'a RepositoryVersion,
    },
}

impl RepositoryTargetState {
    pub(crate) const fn as_ref(&self) -> RepositoryTargetStateRef<'_> {
        match self {
            Self::RootPresent(value) => RepositoryTargetStateRef::RootPresent {
                repository_version: &value.repository_version,
                target_fingerprint: &value.target_fingerprint,
            },
            Self::ObjectPresent(value) => RepositoryTargetStateRef::ObjectPresent {
                object_id: &value.object_id,
                repository_version: &value.repository_version,
                target_fingerprint: &value.target_fingerprint,
            },
            Self::ObjectAbsent(value) => RepositoryTargetStateRef::ObjectAbsent {
                object_id: &value.object_id,
                absence_established_at_version: &value.absence_established_at_version,
            },
        }
    }
}

impl JsonSchema for RepositoryTargetState {
    fn schema_name() -> Cow<'static, str> {
        "RepositoryTargetState".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<RootPresentTargetState>(),
            generator.subschema_for::<ObjectPresentTargetState>(),
            generator.subschema_for::<ObjectAbsentTargetState>(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RootTargetIdentity {
    target_kind: ConfigurationRootKind,
}

impl RootTargetIdentity {
    pub(crate) const fn new() -> Self {
        Self {
            target_kind: ConfigurationRootKind::Value,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ObjectTargetIdentity {
    target_kind: DevelopmentObjectKind,
    object_id: MetadataObjectId,
}

impl ObjectTargetIdentity {
    pub(crate) const fn new(object_id: MetadataObjectId) -> Self {
        Self {
            target_kind: DevelopmentObjectKind::Value,
            object_id,
        }
    }

    pub(crate) const fn object_id(&self) -> &MetadataObjectId {
        &self.object_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum RepositoryTargetIdentity {
    ConfigurationRoot(RootTargetIdentity),
    DevelopmentObject(ObjectTargetIdentity),
}

impl RepositoryTargetIdentity {
    pub(crate) const fn configuration_root() -> Self {
        Self::ConfigurationRoot(RootTargetIdentity::new())
    }

    pub(crate) const fn development_object(object_id: MetadataObjectId) -> Self {
        Self::DevelopmentObject(ObjectTargetIdentity::new(object_id))
    }
}

impl Ord for RepositoryTargetIdentity {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::ConfigurationRoot(_), Self::ConfigurationRoot(_)) => Ordering::Equal,
            (Self::ConfigurationRoot(_), Self::DevelopmentObject(_)) => Ordering::Less,
            (Self::DevelopmentObject(_), Self::ConfigurationRoot(_)) => Ordering::Greater,
            (Self::DevelopmentObject(left), Self::DevelopmentObject(right)) => {
                left.object_id.cmp(&right.object_id)
            }
        }
    }
}

impl PartialOrd for RepositoryTargetIdentity {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl HasTargetKey for RepositoryTargetIdentity {
    fn target_key(&self) -> TargetKey {
        match self {
            Self::ConfigurationRoot(_) => TargetKey::Root,
            Self::DevelopmentObject(value) => {
                TargetKey::Object(value.object_id.as_str().to_owned())
            }
        }
    }
}

impl JsonSchema for RepositoryTargetIdentity {
    fn schema_name() -> Cow<'static, str> {
        "RepositoryTargetIdentity".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<RootTargetIdentity>(),
            generator.subschema_for::<ObjectTargetIdentity>(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RootPlannedChange {
    target_kind: ConfigurationRootKind,
    action: ModifyAction,
    object_display: RepositoryTargetDisplay,
    repository_version: RepositoryVersion,
    target_fingerprint: Sha256Digest,
    relevance: RepositoryRelevance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ObjectPresentPlannedChange {
    target_kind: DevelopmentObjectKind,
    object_id: MetadataObjectId,
    object_display: RepositoryTargetDisplay,
    action: AddOrModifyAction,
    repository_version: RepositoryVersion,
    target_fingerprint: Sha256Digest,
    relevance: RepositoryRelevance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ObjectAbsentPlannedChange {
    target_kind: DevelopmentObjectKind,
    object_id: MetadataObjectId,
    object_display: RepositoryTargetDisplay,
    action: DeleteAction,
    deletion_repository_version: RepositoryVersion,
    expected_absent: TrueLiteral,
    relevance: RepositoryRelevance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum RepositoryPlannedChange {
    RootModify(RootPlannedChange),
    ObjectPresent(ObjectPresentPlannedChange),
    ObjectAbsent(ObjectAbsentPlannedChange),
}

impl RepositoryPlannedChange {
    pub(crate) fn target_identity(&self) -> RepositoryTargetIdentity {
        match self {
            Self::RootModify(_) => RepositoryTargetIdentity::configuration_root(),
            Self::ObjectPresent(value) => {
                RepositoryTargetIdentity::development_object(value.object_id.clone())
            }
            Self::ObjectAbsent(value) => {
                RepositoryTargetIdentity::development_object(value.object_id.clone())
            }
        }
    }

    pub(crate) const fn relevance(&self) -> RepositoryRelevance {
        match self {
            Self::RootModify(value) => value.relevance,
            Self::ObjectPresent(value) => value.relevance,
            Self::ObjectAbsent(value) => value.relevance,
        }
    }

    pub(crate) const fn is_structural(&self) -> bool {
        matches!(
            self,
            Self::ObjectPresent(ObjectPresentPlannedChange {
                action: AddOrModifyAction::Add,
                ..
            }) | Self::ObjectAbsent(_)
        )
    }

    pub(crate) const fn repository_version(&self) -> &RepositoryVersion {
        match self {
            Self::RootModify(value) => &value.repository_version,
            Self::ObjectPresent(value) => &value.repository_version,
            Self::ObjectAbsent(value) => &value.deletion_repository_version,
        }
    }

    pub(crate) fn target_state(&self) -> RepositoryTargetState {
        match self {
            Self::RootModify(value) => RepositoryTargetState::RootPresent(RootPresentTargetState {
                target_kind: ConfigurationRootKind::Value,
                state: PresentState::Value,
                repository_version: value.repository_version.clone(),
                target_fingerprint: value.target_fingerprint.clone(),
            }),
            Self::ObjectPresent(value) => {
                RepositoryTargetState::ObjectPresent(ObjectPresentTargetState {
                    target_kind: DevelopmentObjectKind::Value,
                    state: PresentState::Value,
                    object_id: value.object_id.clone(),
                    repository_version: value.repository_version.clone(),
                    target_fingerprint: value.target_fingerprint.clone(),
                })
            }
            Self::ObjectAbsent(value) => {
                RepositoryTargetState::ObjectAbsent(ObjectAbsentTargetState {
                    target_kind: DevelopmentObjectKind::Value,
                    state: AbsentState::Value,
                    object_id: value.object_id.clone(),
                    absence_established_at_version: value.deletion_repository_version.clone(),
                    expected_absent: TrueLiteral,
                })
            }
        }
    }
}

impl JsonSchema for RepositoryPlannedChange {
    fn schema_name() -> Cow<'static, str> {
        "RepositoryPlannedChange".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<RootPlannedChange>(),
            generator.subschema_for::<ObjectPresentPlannedChange>(),
            generator.subschema_for::<ObjectAbsentPlannedChange>(),
        ])
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub(crate) enum RepositoryUpdateLockReason {
    SupportGraphGuard,
    UpdateTarget,
    ParentClosure,
    ReferenceClosure,
    StructuralClosure,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(crate) struct RepositoryUpdateLockReasons(Vec<RepositoryUpdateLockReason>);

impl<'de> Deserialize<'de> for RepositoryUpdateLockReasons {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let reasons = Vec::<RepositoryUpdateLockReason>::deserialize(deserializer)?;
        if reasons.is_empty() || reasons.len() > 5 {
            return Err(D::Error::custom(
                "lock reasons must contain one through five items",
            ));
        }
        if reasons.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(D::Error::custom(
                "lock reasons must be unique and follow declaration order",
            ));
        }
        Ok(Self(reasons))
    }
}

impl JsonSchema for RepositoryUpdateLockReasons {
    fn schema_name() -> Cow<'static, str> {
        "RepositoryUpdateLockReasons".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "minItems": 1,
            "maxItems": 5,
            "items": generator.subschema_for::<RepositoryUpdateLockReason>(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RootUpdateLockTarget {
    target_kind: ConfigurationRootKind,
    object_display: RepositoryTargetDisplay,
    reasons: RepositoryUpdateLockReasons,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ObjectUpdateLockTarget {
    target_kind: DevelopmentObjectKind,
    object_id: MetadataObjectId,
    object_display: RepositoryTargetDisplay,
    reasons: RepositoryUpdateLockReasons,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum RepositoryUpdateLockTarget {
    ConfigurationRoot(RootUpdateLockTarget),
    DevelopmentObject(ObjectUpdateLockTarget),
}

/// Borrowed lock projection for exact cross-contract binding without JSON
/// inspection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RepositoryUpdateLockTargetRef<'a> {
    ConfigurationRoot {
        object_display: &'a RepositoryTargetDisplay,
        reasons: &'a [RepositoryUpdateLockReason],
    },
    DevelopmentObject {
        object_id: &'a MetadataObjectId,
        object_display: &'a RepositoryTargetDisplay,
        reasons: &'a [RepositoryUpdateLockReason],
    },
}

impl RepositoryUpdateLockTarget {
    pub(crate) fn as_ref(&self) -> RepositoryUpdateLockTargetRef<'_> {
        match self {
            Self::ConfigurationRoot(value) => RepositoryUpdateLockTargetRef::ConfigurationRoot {
                object_display: &value.object_display,
                reasons: &value.reasons.0,
            },
            Self::DevelopmentObject(value) => RepositoryUpdateLockTargetRef::DevelopmentObject {
                object_id: &value.object_id,
                object_display: &value.object_display,
                reasons: &value.reasons.0,
            },
        }
    }
}

impl JsonSchema for RepositoryUpdateLockTarget {
    fn schema_name() -> Cow<'static, str> {
        "RepositoryUpdateLockTarget".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<RootUpdateLockTarget>(),
            generator.subschema_for::<ObjectUpdateLockTarget>(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum TargetKey {
    Root,
    Object(String),
}

trait HasTargetKey {
    fn target_key(&self) -> TargetKey;
}

impl HasTargetKey for RepositoryTargetState {
    fn target_key(&self) -> TargetKey {
        match self {
            Self::RootPresent(_) => TargetKey::Root,
            Self::ObjectPresent(value) => TargetKey::Object(value.object_id.as_str().to_owned()),
            Self::ObjectAbsent(value) => TargetKey::Object(value.object_id.as_str().to_owned()),
        }
    }
}

impl HasTargetKey for RepositoryPlannedChange {
    fn target_key(&self) -> TargetKey {
        match self {
            Self::RootModify(_) => TargetKey::Root,
            Self::ObjectPresent(value) => TargetKey::Object(value.object_id.as_str().to_owned()),
            Self::ObjectAbsent(value) => TargetKey::Object(value.object_id.as_str().to_owned()),
        }
    }
}

impl HasTargetKey for RepositoryUpdateLockTarget {
    fn target_key(&self) -> TargetKey {
        match self {
            Self::ConfigurationRoot(_) => TargetKey::Root,
            Self::DevelopmentObject(value) => {
                TargetKey::Object(value.object_id.as_str().to_owned())
            }
        }
    }
}

fn validate_forward<T: HasTargetKey>(values: &[T], require_root: bool) -> bool {
    if values.len() > MAX_METADATA_ITEMS || require_root && values.is_empty() {
        return false;
    }
    let keys: Vec<_> = values.iter().map(HasTargetKey::target_key).collect();
    if require_root && keys.first() != Some(&TargetKey::Root) {
        return false;
    }
    keys.windows(2).all(|pair| pair[0] < pair[1])
}

fn validate_reverse<T: HasTargetKey>(values: &[T]) -> bool {
    if values.is_empty() || values.len() > MAX_METADATA_ITEMS {
        return false;
    }
    let keys: Vec<_> = values.iter().map(HasTargetKey::target_key).collect();
    if keys.last() != Some(&TargetKey::Root) {
        return false;
    }
    keys.windows(2).all(|pair| pair[0] > pair[1])
}

macro_rules! validated_target_collection {
    ($name:ident, $item:ty, $validate:expr, $min:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize)]
        #[serde(transparent)]
        pub(crate) struct $name(Vec<$item>);

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let values = Vec::<$item>::deserialize(deserializer)?;
                if !($validate)(&values) {
                    return Err(D::Error::custom(concat!(
                        stringify!($name),
                        " violates canonical identity order"
                    )));
                }
                Ok(Self(values))
            }
        }

        impl JsonSchema for $name {
            fn schema_name() -> Cow<'static, str> {
                stringify!($name).into()
            }

            fn json_schema(generator: &mut SchemaGenerator) -> Schema {
                json_schema!({
                    "type": "array",
                    "minItems": $min,
                    "maxItems": MAX_METADATA_ITEMS,
                    "items": generator.subschema_for::<$item>(),
                })
            }
        }

        impl $name {
            pub(crate) fn as_slice(&self) -> &[$item] {
                &self.0
            }
        }
    };
}

validated_target_collection!(
    RepositoryTargetStates,
    RepositoryTargetState,
    |values: &[RepositoryTargetState]| validate_forward(values, false),
    0
);
validated_target_collection!(
    RepositoryPlannedChanges,
    RepositoryPlannedChange,
    |values: &[RepositoryPlannedChange]| validate_forward(values, false),
    0
);
validated_target_collection!(
    RepositoryUpdateLockTargets,
    RepositoryUpdateLockTarget,
    |values: &[RepositoryUpdateLockTarget]| validate_forward(values, true),
    1
);
validated_target_collection!(
    AcquiredRepositoryUpdateLockTargets,
    RepositoryUpdateLockTarget,
    |values: &[RepositoryUpdateLockTarget]| validate_forward(values, true),
    1
);
validated_target_collection!(
    ReleasedRepositoryUpdateLockTargets,
    RepositoryUpdateLockTarget,
    |values: &[RepositoryUpdateLockTarget]| validate_reverse(values),
    1
);

/// Adapter-observed target changes for one exact validated history entry.
/// The enclosing capability token binds the version/digest tuple to the
/// complete partition before the core independently folds these changes.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RoutineRepositoryVersionChangeObservation {
    repository_version: RepositoryVersion,
    semantic_delta_digest: Sha256Digest,
    changes: RepositoryPlannedChanges,
}

impl RoutineRepositoryVersionChangeObservation {
    pub(crate) const fn from_capability_adapter(
        repository_version: RepositoryVersion,
        semantic_delta_digest: Sha256Digest,
        changes: RepositoryPlannedChanges,
    ) -> Self {
        Self {
            repository_version,
            semantic_delta_digest,
            changes,
        }
    }
}

/// Complete capability-backed history-to-target observation for routine
/// update. Deliberately non-`Clone` and non-wire.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RoutineUpdateHistoryFoldCapabilityToken {
    before_anchor_digest: Sha256Digest,
    partition_digest: Sha256Digest,
    before_target_states: RepositoryTargetStates,
    entry_changes: Vec<RoutineRepositoryVersionChangeObservation>,
    target_projection_capability_id: CapabilityRowId,
}

impl RoutineUpdateHistoryFoldCapabilityToken {
    pub(crate) fn from_capability_adapter(
        before_anchor: &RepositoryAnchor,
        history_partition: &ValidatedRepositoryHistoryPartition,
        before_target_states: RepositoryTargetStates,
        entry_changes: Vec<RoutineRepositoryVersionChangeObservation>,
        target_projection_capability_id: CapabilityRowId,
    ) -> Result<Self, RepositoryContractError> {
        if before_anchor.history_cursor() != history_partition.start_cursor()
            || entry_changes.len() != history_partition.entry_count()
            || !history_partition
                .entries()
                .zip(entry_changes.iter())
                .all(|(entry, observation)| {
                    entry.repository_version() == &observation.repository_version
                        && entry.semantic_delta_digest() == &observation.semantic_delta_digest
                })
        {
            return Err(RepositoryContractError(
                "routine target projection is not bound to the complete history partition",
            ));
        }
        Ok(Self {
            before_anchor_digest: before_anchor.anchor_digest().clone(),
            partition_digest: history_partition.partition_digest().clone(),
            before_target_states,
            entry_changes,
            target_projection_capability_id,
        })
    }
}

pub(crate) trait RoutineUpdateProjectionResolver {
    fn resolve_history_fold(
        &mut self,
        before_anchor: &RepositoryAnchor,
        history_partition: &ValidatedRepositoryHistoryPartition,
    ) -> Result<RoutineUpdateHistoryFoldCapabilityToken, RepositoryContractError>;

    fn resolve_selective_plan(
        &mut self,
        planned_targets: &RepositoryTargetStates,
        structural_targets: &[RepositoryTargetIdentity],
    ) -> Result<RoutineSelectiveRepositoryUpdatePlanAuthority, RepositoryContractError>;

    fn resolve_phase(
        &mut self,
        before_anchor: &RepositoryAnchor,
        history_partition: &ValidatedRepositoryHistoryPartition,
    ) -> Result<RoutineUpdatePhaseAuthority, RepositoryContractError>;
}

struct FoldedRoutineUpdate {
    planned_changes: RepositoryPlannedChanges,
    planned_targets: RepositoryTargetStates,
    planned_relevant_objects: Vec<RepositoryTargetIdentity>,
    planned_unrelated_objects: Vec<RepositoryTargetIdentity>,
    structural_changes: RepositoryPlannedChanges,
    structural_targets: Vec<RepositoryTargetIdentity>,
    contains_relevant_advance: bool,
}

fn classification_relevance(
    classification: RepositoryHistoryPartitionClassification,
) -> Option<RepositoryRelevance> {
    match classification {
        RepositoryHistoryPartitionClassification::UnrelatedRoutine => {
            Some(RepositoryRelevance::Unrelated)
        }
        RepositoryHistoryPartitionClassification::RelevantRoutine
        | RepositoryHistoryPartitionClassification::AuthorizedSupport
        | RepositoryHistoryPartitionClassification::Invalid
        | RepositoryHistoryPartitionClassification::Corrective => {
            Some(RepositoryRelevance::Relevant)
        }
        RepositoryHistoryPartitionClassification::ExternalSupport
        | RepositoryHistoryPartitionClassification::PreArmExternal
        | RepositoryHistoryPartitionClassification::NonConflictingConcurrent
        | RepositoryHistoryPartitionClassification::TaskCommit => None,
    }
}

fn validate_routine_partition_grammar(
    partition: &ValidatedRepositoryHistoryPartition,
    deferred: Option<&DeferredRepositoryAdvance>,
) -> Result<bool, RepositoryContractError> {
    let classifications: Vec<_> = partition.classifications().collect();
    if deferred.is_some() {
        let Some((first, rest)) = classifications.split_first() else {
            return Err(RepositoryContractError(
                "current deferred advance requires its immediate successor",
            ));
        };
        if !matches!(
            first,
            RepositoryHistoryPartitionClassification::AuthorizedSupport
                | RepositoryHistoryPartitionClassification::Invalid
                | RepositoryHistoryPartitionClassification::Corrective
        ) || rest.iter().any(|classification| {
            !matches!(
                classification,
                RepositoryHistoryPartitionClassification::UnrelatedRoutine
                    | RepositoryHistoryPartitionClassification::RelevantRoutine
            )
        }) {
            return Err(RepositoryContractError(
                "deferred routine partition has an invalid first or tail classification",
            ));
        }
    } else if classifications.iter().any(|classification| {
        !matches!(
            classification,
            RepositoryHistoryPartitionClassification::UnrelatedRoutine
                | RepositoryHistoryPartitionClassification::RelevantRoutine
        )
    }) {
        return Err(RepositoryContractError(
            "ordinary routine partition contains non-routine history",
        ));
    }
    Ok(classifications.into_iter().any(|classification| {
        classification_relevance(classification) == Some(RepositoryRelevance::Relevant)
    }))
}

fn target_state_semantically_equal(
    left: &RepositoryTargetState,
    right: &RepositoryTargetState,
) -> bool {
    match (left, right) {
        (RepositoryTargetState::RootPresent(left), RepositoryTargetState::RootPresent(right)) => {
            left.target_fingerprint == right.target_fingerprint
        }
        (
            RepositoryTargetState::ObjectPresent(left),
            RepositoryTargetState::ObjectPresent(right),
        ) => {
            left.object_id == right.object_id && left.target_fingerprint == right.target_fingerprint
        }
        (RepositoryTargetState::ObjectAbsent(left), RepositoryTargetState::ObjectAbsent(right)) => {
            left.object_id == right.object_id
        }
        _ => false,
    }
}

fn change_applies_to_state(
    current: &RepositoryTargetState,
    change: &RepositoryPlannedChange,
) -> bool {
    match (current, change) {
        (RepositoryTargetState::RootPresent(_), RepositoryPlannedChange::RootModify(_)) => true,
        (
            RepositoryTargetState::ObjectAbsent(current),
            RepositoryPlannedChange::ObjectPresent(change),
        ) => current.object_id == change.object_id && change.action == AddOrModifyAction::Add,
        (
            RepositoryTargetState::ObjectPresent(current),
            RepositoryPlannedChange::ObjectPresent(change),
        ) => current.object_id == change.object_id && change.action == AddOrModifyAction::Modify,
        (
            RepositoryTargetState::ObjectPresent(current),
            RepositoryPlannedChange::ObjectAbsent(change),
        ) => current.object_id == change.object_id,
        _ => false,
    }
}

fn finalize_folded_change(
    mut change: RepositoryPlannedChange,
    before: &RepositoryTargetState,
    relevance: RepositoryRelevance,
) -> Result<RepositoryPlannedChange, RepositoryContractError> {
    match (&mut change, before) {
        (RepositoryPlannedChange::RootModify(value), RepositoryTargetState::RootPresent(_)) => {
            value.relevance = relevance;
        }
        (
            RepositoryPlannedChange::ObjectPresent(value),
            RepositoryTargetState::ObjectPresent(_),
        ) => {
            value.action = AddOrModifyAction::Modify;
            value.relevance = relevance;
        }
        (RepositoryPlannedChange::ObjectPresent(value), RepositoryTargetState::ObjectAbsent(_)) => {
            value.action = AddOrModifyAction::Add;
            value.relevance = relevance;
        }
        (RepositoryPlannedChange::ObjectAbsent(value), RepositoryTargetState::ObjectPresent(_)) => {
            value.relevance = relevance;
        }
        _ => {
            return Err(RepositoryContractError(
                "routine history final state is incompatible with its before state",
            ));
        }
    }
    Ok(change)
}

fn fold_routine_history(
    before_anchor: &RepositoryAnchor,
    partition: &ValidatedRepositoryHistoryPartition,
    token: RoutineUpdateHistoryFoldCapabilityToken,
    contains_relevant_advance: bool,
) -> Result<FoldedRoutineUpdate, RepositoryContractError> {
    if token.before_anchor_digest != *before_anchor.anchor_digest()
        || token.partition_digest != *partition.partition_digest()
        || token.entry_changes.len() != partition.entry_count()
    {
        return Err(RepositoryContractError(
            "routine fold capability belongs to another anchor or partition",
        ));
    }
    let _target_projection_capability_id = token.target_projection_capability_id;
    let mut states: BTreeMap<TargetKey, (RepositoryTargetState, RepositoryTargetState)> = token
        .before_target_states
        .as_slice()
        .iter()
        .cloned()
        .map(|state| (state.target_key(), (state.clone(), state)))
        .collect();
    let mut last_changes = BTreeMap::<TargetKey, (RepositoryPlannedChange, bool)>::new();

    for (entry, observation) in partition.entries().zip(token.entry_changes) {
        if entry.repository_version() != &observation.repository_version
            || entry.semantic_delta_digest() != &observation.semantic_delta_digest
        {
            return Err(RepositoryContractError(
                "routine fold entry observation was substituted",
            ));
        }
        let expected_relevance = classification_relevance(entry.classification()).ok_or(
            RepositoryContractError("routine fold received an unsupported classification"),
        )?;
        for change in observation.changes.0 {
            if change.repository_version() != entry.repository_version()
                || change.relevance() != expected_relevance
            {
                return Err(RepositoryContractError(
                    "routine target change disagrees with its history entry",
                ));
            }
            let key = change.target_key();
            let Some((_, current)) = states.get_mut(&key) else {
                return Err(RepositoryContractError(
                    "routine target projection omitted a touched before state",
                ));
            };
            if !change_applies_to_state(current, &change) {
                return Err(RepositoryContractError(
                    "routine target change is not a valid ordered state transition",
                ));
            }
            *current = change.target_state();
            last_changes
                .entry(key)
                .and_modify(|(last, relevant)| {
                    *last = change.clone();
                    *relevant |= expected_relevance == RepositoryRelevance::Relevant;
                })
                .or_insert((change, expected_relevance == RepositoryRelevance::Relevant));
        }
    }
    if states.len() != last_changes.len()
        || states.keys().any(|key| !last_changes.contains_key(key))
    {
        return Err(RepositoryContractError(
            "routine target projection before-state set is not exact",
        ));
    }

    let mut planned_changes = Vec::new();
    let mut planned_targets = Vec::new();
    let mut relevant_targets = Vec::new();
    let mut unrelated_targets = Vec::new();
    let mut structural_changes = Vec::new();
    let mut structural_targets = Vec::new();
    for (key, (last, relevant)) in last_changes {
        let (before, current) = states.get(&key).expect("exact state key was validated");
        if target_state_semantically_equal(before, current) {
            continue;
        }
        let relevance = if relevant {
            RepositoryRelevance::Relevant
        } else {
            RepositoryRelevance::Unrelated
        };
        let change = finalize_folded_change(last, before, relevance)?;
        let target = change.target_identity();
        if relevance == RepositoryRelevance::Relevant {
            relevant_targets.push(target.clone());
        } else {
            unrelated_targets.push(target.clone());
        }
        if change.is_structural() {
            structural_targets.push(target);
            structural_changes.push(change.clone());
        }
        planned_targets.push(change.target_state());
        planned_changes.push(change);
    }
    Ok(FoldedRoutineUpdate {
        planned_changes: RepositoryPlannedChanges(planned_changes),
        planned_targets: RepositoryTargetStates(planned_targets),
        planned_relevant_objects: relevant_targets,
        planned_unrelated_objects: unrelated_targets,
        structural_changes: RepositoryPlannedChanges(structural_changes),
        structural_targets,
        contains_relevant_advance,
    })
}

/// Capability-complete, non-wire routine-update projection consumed by result
/// construction. It has no raw DTO constructor and is deliberately non-Clone.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ValidatedRoutineUpdateProjection {
    before_anchor: RepositoryAnchor,
    expected_history_cursor: RepositoryHistoryCursor,
    observed_history_cursor: RepositoryHistoryCursor,
    deferred_repository_advance: Option<DeferredRepositoryAdvance>,
    deferred_terminal_receipt_id: Option<UnicaId>,
    deferred_advance_resolution_digest: Option<Sha256Digest>,
    planned_changes: RepositoryPlannedChanges,
    planned_relevant_objects: Vec<RepositoryTargetIdentity>,
    planned_unrelated_objects: Vec<RepositoryTargetIdentity>,
    structural_changes: RepositoryPlannedChanges,
    history_partition: ValidatedRepositoryHistoryPartition,
    selective_update_plan: SelectiveRepositoryUpdatePlan,
    resulting_phase: TaskPhase,
}

impl ValidatedRoutineUpdateProjection {
    pub(crate) fn resolve<R: RoutineUpdateProjectionResolver>(
        before_anchor: RepositoryAnchor,
        history_partition: ValidatedRepositoryHistoryPartition,
        current_deferred: Option<CurrentDeferredRepositoryAdvanceAuthority>,
        mut resolver: R,
    ) -> Result<Self, RepositoryContractError> {
        let (deferred_terminal_receipt_id, deferred_repository_advance) = match current_deferred {
            Some(authority) => {
                let (terminal_receipt_id, advance) = authority.into_parts();
                (Some(terminal_receipt_id), Some(advance))
            }
            None => (None, None),
        };
        if before_anchor.history_cursor() != history_partition.start_cursor() {
            return Err(RepositoryContractError(
                "routine projection anchor does not start its history partition",
            ));
        }
        let contains_relevant_advance = validate_routine_partition_grammar(
            &history_partition,
            deferred_repository_advance.as_ref(),
        )?;
        let fold_token = resolver.resolve_history_fold(&before_anchor, &history_partition)?;
        let folded = fold_routine_history(
            &before_anchor,
            &history_partition,
            fold_token,
            contains_relevant_advance,
        )?;
        let plan_authority =
            resolver.resolve_selective_plan(&folded.planned_targets, &folded.structural_targets)?;
        let selective_update_plan = SelectiveRepositoryUpdatePlan::routine_from_authority(
            plan_authority,
            &folded.planned_targets,
            &folded.structural_targets,
        )?;
        let resulting_phase = resolver
            .resolve_phase(&before_anchor, &history_partition)?
            .into_resulting_phase(folded.contains_relevant_advance)?;
        let deferred_advance_resolution_digest = deferred_repository_advance
            .as_ref()
            .map(|deferred| deferred.routine_resolution_digest(&history_partition, resulting_phase))
            .transpose()?;
        let expected_history_cursor = before_anchor.history_cursor().clone();
        let observed_history_cursor = history_partition.through_inclusive().clone();
        Ok(Self {
            before_anchor,
            expected_history_cursor,
            observed_history_cursor,
            deferred_repository_advance,
            deferred_terminal_receipt_id,
            deferred_advance_resolution_digest,
            planned_changes: folded.planned_changes,
            planned_relevant_objects: folded.planned_relevant_objects,
            planned_unrelated_objects: folded.planned_unrelated_objects,
            structural_changes: folded.structural_changes,
            history_partition,
            selective_update_plan,
            resulting_phase,
        })
    }

    pub(crate) const fn before_anchor(&self) -> &RepositoryAnchor {
        &self.before_anchor
    }

    pub(crate) const fn expected_history_cursor(&self) -> &RepositoryHistoryCursor {
        &self.expected_history_cursor
    }

    pub(crate) const fn observed_history_cursor(&self) -> &RepositoryHistoryCursor {
        &self.observed_history_cursor
    }

    pub(crate) const fn deferred_repository_advance(&self) -> Option<&DeferredRepositoryAdvance> {
        self.deferred_repository_advance.as_ref()
    }

    pub(crate) const fn deferred_terminal_receipt_id(&self) -> Option<&UnicaId> {
        self.deferred_terminal_receipt_id.as_ref()
    }

    pub(crate) const fn deferred_advance_resolution_digest(&self) -> Option<&Sha256Digest> {
        self.deferred_advance_resolution_digest.as_ref()
    }

    pub(crate) const fn planned_changes(&self) -> &RepositoryPlannedChanges {
        &self.planned_changes
    }

    pub(crate) fn planned_relevant_objects(&self) -> &[RepositoryTargetIdentity] {
        &self.planned_relevant_objects
    }

    pub(crate) fn planned_unrelated_objects(&self) -> &[RepositoryTargetIdentity] {
        &self.planned_unrelated_objects
    }

    pub(crate) const fn structural_changes(&self) -> &RepositoryPlannedChanges {
        &self.structural_changes
    }

    pub(crate) fn structural_confirmation_required(&self) -> bool {
        !self.structural_changes.as_slice().is_empty()
    }

    pub(crate) const fn history_partition(&self) -> &ValidatedRepositoryHistoryPartition {
        &self.history_partition
    }

    pub(crate) const fn selective_update_plan(&self) -> &SelectiveRepositoryUpdatePlan {
        &self.selective_update_plan
    }

    pub(crate) const fn resulting_phase(&self) -> TaskPhase {
        self.resulting_phase
    }
}

fn parse_digest(value: &str) -> Result<Sha256Digest, RepositoryContractError> {
    Sha256Digest::parse(value).map_err(|_| RepositoryContractError("invalid SHA-256 digest"))
}

#[cfg(test)]
pub(crate) fn validated_task_commit_partition_fixture_test_only(
    commit: &ValidatedCommitObjectAuthority,
) -> ValidatedTaskCommitHistoryPartition {
    tests::validated_task_commit_partition_for_results(commit)
}

#[cfg(test)]
mod tests {
    use super::{
        AcquiredRepositoryUpdateLockTargets, CanonicalEmptyDeltaDigest, EvidenceKind,
        EvidenceSourceIndex, EvidenceSourceIndexCandidate, EvidenceSourceIndexCandidateRow,
        EvidenceSourceRegistry, FrozenSupportConflictInstructionSourceAuthority,
        FrozenSupportCorrectiveInstructionSourceAuthority, NonConflictingConcurrentEvidence,
        ReleasedRepositoryUpdateLockTargets, RepositoryActorIdentity, RepositoryAnchor,
        RepositoryAnchorDigestRecord, RepositoryAnchorObservationAuthority,
        RepositoryHistoryCursor, RepositoryHistoryEvidenceBytesResolver,
        RepositoryHistoryOrderEvidence, RepositoryHistoryOrderResolver,
        RepositoryHistoryPartitionResolver, RepositoryHistorySourceEvidenceRef,
        RepositoryOwnerIdentity, RepositoryPlannedChanges, RepositoryTargetIdentity,
        RepositoryTargetKind, RepositoryTargetStates, RepositoryUpdateLockReason,
        RepositoryUpdateLockReasons, RepositoryUpdateLockTargets,
        RoutineRepositoryVersionChangeObservation, RoutineRepositoryVersionClassificationEvidence,
        RoutineUpdateHistoryFoldCapabilityToken, RoutineUpdateProjectionResolver,
        SupportCorrectiveEvidenceResolver, UnvalidatedRepositoryHistoryPartition,
        ValidatedRoutineUpdateProjection, ValidatedSupportRecoveryHistoryEntryRef,
    };
    use crate::domain::branched_development::contracts::artifacts::ConfigurationIdentity;
    use crate::domain::branched_development::contracts::instructions::{
        SupportConflictInstruction, SupportCorrectiveInstruction,
        SupportCorrectiveInstructionAuthority, SupportRecoveryTransition,
    };
    use crate::domain::branched_development::contracts::results::repository::validated_commit_object_authority_fixture_test_only;
    use crate::domain::branched_development::contracts::scalars::{
        Diagnostic, EmptyOrName, Name, RepositoryTargetDisplay, RepositoryUsername,
        RepositoryVersion, RequiredNullable,
    };
    use crate::domain::branched_development::contracts::schema::audit_json_schema;
    use crate::domain::branched_development::contracts::support::{
        ExternalSupportOwnershipEvidence, ManualSupportTargetMode, SupportActionPurpose,
        SupportContractError, SupportHistoryOrderAuthority, SupportObservationCorrectiveProjection,
        SupportObservationTask8Projection, SupportTransition, SupportTransitionConflict,
        SupportTransitionConflicts, SupportTransitionOverlapKind,
    };
    use crate::domain::branched_development::contracts::support_terminalization::{
        SupportRecoveryLockTarget, SupportRecoveryLockTargets,
    };
    use crate::domain::branched_development::{
        CapabilityRowId, MetadataObjectId, Sha256Digest, SupportLayerId, TaskPhase, UnicaId,
    };
    use schemars::schema_for;
    use serde_json::{json, Value};
    use sha2::{Digest, Sha256};
    use std::cmp::Ordering;
    use std::collections::BTreeMap;
    use std::sync::{
        atomic::{AtomicUsize, Ordering as AtomicOrdering},
        Arc,
    };

    const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const SHA_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const UUID_A: &str = "123e4567-e89b-12d3-a456-426614174000";
    const UUID_B: &str = "223e4567-e89b-42d3-a456-426614174000";
    const OBJECT_A: &str = "00000000-0000-0000-0000-000000000001";
    const OBJECT_B: &str = "00000000-0000-0000-0000-000000000002";

    fn assert_closed<T: schemars::JsonSchema>() {
        let schema = serde_json::to_value(schema_for!(T)).unwrap();
        audit_json_schema(&schema).expect("repository schema must be recursively closed");
    }

    fn test_digest(value: &Value) -> String {
        format!(
            "{:x}",
            Sha256::digest(serde_json_canonicalizer::to_vec(value).unwrap())
        )
    }

    trait AmbiguousIfDeserializeOwned<Marker> {
        fn marker() {}
    }

    impl<T: ?Sized> AmbiguousIfDeserializeOwned<()> for T {}
    impl<T: serde::de::DeserializeOwned> AmbiguousIfDeserializeOwned<u8> for T {}

    trait AmbiguousIfClone<Marker> {
        fn marker() {}
    }

    impl<T: ?Sized> AmbiguousIfClone<()> for T {}
    impl<T: Clone> AmbiguousIfClone<u8> for T {}

    trait AmbiguousIfSerialize<Marker> {
        fn marker() {}
    }

    impl<T: ?Sized> AmbiguousIfSerialize<()> for T {}
    impl<T: serde::Serialize> AmbiguousIfSerialize<u8> for T {}

    #[test]
    fn capability_derived_authority_types_have_no_deserialize_backdoor() {
        let _ = <NonConflictingConcurrentEvidence as AmbiguousIfDeserializeOwned<_>>::marker;
        let _ =
            <super::ValidatedRepositoryHistoryPartition as AmbiguousIfDeserializeOwned<_>>::marker;
        let _ = <super::EvidenceSourceIndexProof as AmbiguousIfDeserializeOwned<_>>::marker;
        let _ = <SupportObservationTask8Projection as AmbiguousIfDeserializeOwned<_>>::marker;
        let _ = <SupportObservationCorrectiveProjection as AmbiguousIfDeserializeOwned<_>>::marker;
        let _ = <FrozenSupportCorrectiveInstructionSourceAuthority as AmbiguousIfDeserializeOwned<
            _,
        >>::marker;
        let _ = <FrozenSupportConflictInstructionSourceAuthority as AmbiguousIfDeserializeOwned<
            _,
        >>::marker;
        let _ = <super::ValidatedSupportObservationHistoryEntry as AmbiguousIfDeserializeOwned<
            _,
        >>::marker;
        let _ = <RepositoryAnchor as AmbiguousIfDeserializeOwned<_>>::marker;
        let _ = <RepositoryAnchorDigestRecord as AmbiguousIfDeserializeOwned<_>>::marker;
        let _ = <RepositoryAnchorObservationAuthority as AmbiguousIfDeserializeOwned<_>>::marker;
        let _ = <RepositoryAnchorObservationAuthority as AmbiguousIfClone<_>>::marker;
        let _ = <super::RoutineUpdateHistoryFoldCapabilityToken as AmbiguousIfDeserializeOwned<
            _,
        >>::marker;
        let _ = <super::ValidatedRoutineUpdateProjection as AmbiguousIfDeserializeOwned<_>>::marker;
        let _ = <super::ValidatedRoutineUpdateProjection as AmbiguousIfClone<_>>::marker;
    }

    fn configuration_identity(version: &str) -> ConfigurationIdentity {
        ConfigurationIdentity::new(
            MetadataObjectId::parse("00000000-0000-0000-0000-000000000001").unwrap(),
            Name::parse("Demo configuration").unwrap(),
            EmptyOrName::parse("Demo vendor").unwrap(),
            EmptyOrName::parse(version).unwrap(),
        )
    }

    #[test]
    fn task12_repository_anchor_is_produced_from_one_consumed_observation_authority() {
        let repository_identity = Sha256Digest::parse(SHA_A).unwrap();
        let history_cursor = RepositoryHistoryCursor::new(
            RepositoryVersion::parse("opaque-v17").unwrap(),
            Sha256Digest::parse(SHA_B).unwrap(),
        );
        let configuration_identity = configuration_identity("8.3.27");
        let configuration_fingerprint =
            Sha256Digest::parse("cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc")
                .unwrap();
        let expected_digest = Sha256Digest::parse(&test_digest(&json!({
            "repositoryIdentity": SHA_A,
            "historyCursor": {
                "throughVersion": "opaque-v17",
                "historyPrefixDigest": SHA_B,
            },
            "configurationIdentity": {
                "metadataUuid": "00000000-0000-0000-0000-000000000001",
                "name": "Demo configuration",
                "vendor": "Demo vendor",
                "version": "8.3.27",
            },
            "configurationFingerprint": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        })))
        .unwrap();

        let authority = RepositoryAnchorObservationAuthority::test_only(
            repository_identity.clone(),
            history_cursor.clone(),
            configuration_identity.clone(),
            configuration_fingerprint.clone(),
        );
        let anchor = authority.into_anchor().unwrap();

        assert_eq!(history_cursor.through_version().as_str(), "opaque-v17");
        assert_eq!(history_cursor.history_prefix_digest().as_str(), SHA_B);
        assert_eq!(anchor.repository_identity(), &repository_identity);
        assert_eq!(anchor.history_cursor(), &history_cursor);
        assert_eq!(anchor.configuration_identity(), &configuration_identity);
        assert_eq!(
            anchor.configuration_fingerprint(),
            &configuration_fingerprint
        );
        assert_eq!(anchor.anchor_digest(), &expected_digest);
        assert_eq!(
            serde_json::to_value(&anchor).unwrap(),
            json!({
                "repositoryIdentity": SHA_A,
                "historyCursor": {
                    "throughVersion": "opaque-v17",
                    "historyPrefixDigest": SHA_B,
                },
                "configurationIdentity": {
                    "metadataUuid": "00000000-0000-0000-0000-000000000001",
                    "name": "Demo configuration",
                    "vendor": "Demo vendor",
                    "version": "8.3.27",
                },
                "configurationFingerprint": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
                "anchorDigest": expected_digest,
            })
        );
        assert_closed::<RepositoryAnchorDigestRecord>();
        assert_closed::<RepositoryAnchor>();
    }

    #[test]
    fn task12_repository_anchor_digest_binds_every_authoritative_preimage_field() {
        let cursor_a = RepositoryHistoryCursor::new(
            RepositoryVersion::parse("opaque-v17").unwrap(),
            Sha256Digest::parse(SHA_A).unwrap(),
        );
        let cursor_b = RepositoryHistoryCursor::new(
            RepositoryVersion::parse("opaque-v18").unwrap(),
            Sha256Digest::parse(SHA_A).unwrap(),
        );
        let base = RepositoryAnchorObservationAuthority::test_only(
            Sha256Digest::parse(SHA_A).unwrap(),
            cursor_a.clone(),
            configuration_identity("8.3.27"),
            Sha256Digest::parse(SHA_B).unwrap(),
        )
        .into_anchor()
        .unwrap();
        let variations = [
            RepositoryAnchorObservationAuthority::test_only(
                Sha256Digest::parse(SHA_B).unwrap(),
                cursor_a.clone(),
                configuration_identity("8.3.27"),
                Sha256Digest::parse(SHA_B).unwrap(),
            )
            .into_anchor()
            .unwrap(),
            RepositoryAnchorObservationAuthority::test_only(
                Sha256Digest::parse(SHA_A).unwrap(),
                cursor_b,
                configuration_identity("8.3.27"),
                Sha256Digest::parse(SHA_B).unwrap(),
            )
            .into_anchor()
            .unwrap(),
            RepositoryAnchorObservationAuthority::test_only(
                Sha256Digest::parse(SHA_A).unwrap(),
                cursor_a.clone(),
                configuration_identity("8.3.28"),
                Sha256Digest::parse(SHA_B).unwrap(),
            )
            .into_anchor()
            .unwrap(),
            RepositoryAnchorObservationAuthority::test_only(
                Sha256Digest::parse(SHA_A).unwrap(),
                cursor_a,
                configuration_identity("8.3.27"),
                Sha256Digest::parse(
                    "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
                )
                .unwrap(),
            )
            .into_anchor()
            .unwrap(),
        ];

        for variation in variations {
            assert_ne!(variation.anchor_digest(), base.anchor_digest());
        }
    }

    #[test]
    fn canonical_empty_delta_digest_accepts_only_the_normative_literal() {
        const EMPTY: &str = "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945";
        let digest: CanonicalEmptyDeltaDigest = serde_json::from_value(json!(EMPTY)).unwrap();
        assert_eq!(digest.as_str(), EMPTY);
        assert_eq!(CanonicalEmptyDeltaDigest::VALUE, EMPTY);
        for invalid in [
            SHA_A,
            "4F53CDA18C2BAA0C0354BB5F9A3ECBE5ED12AB4D8E11BA873C2F11161202B945",
        ] {
            assert!(serde_json::from_value::<CanonicalEmptyDeltaDigest>(json!(invalid)).is_err());
        }
        let schema = serde_json::to_value(schema_for!(CanonicalEmptyDeltaDigest)).unwrap();
        assert_eq!(schema["const"], json!(EMPTY));
        assert_closed::<CanonicalEmptyDeltaDigest>();
    }

    #[test]
    fn repository_actor_and_owner_require_explicit_nullable_members() {
        let actor = json!({"username":"repo-user","computer":null,"infobase":null});
        serde_json::from_value::<RepositoryActorIdentity>(actor.clone()).unwrap();
        let actor_schema = serde_json::to_value(schema_for!(RepositoryActorIdentity)).unwrap();
        let actor_validator = jsonschema::validator_for(&actor_schema).unwrap();
        assert!(actor_validator.is_valid(&actor));
        assert_eq!(
            actor_schema["required"],
            json!(["username", "computer", "infobase"])
        );
        for omitted in ["computer", "infobase"] {
            let mut invalid = actor.as_object().unwrap().clone();
            invalid.remove(omitted);
            let invalid = Value::Object(invalid);
            assert!(serde_json::from_value::<RepositoryActorIdentity>(invalid.clone()).is_err());
            assert!(!actor_validator.is_valid(&invalid));
        }

        let owner = json!({
            "username":"repo-user",
            "computer":null,
            "infobase":null,
            "lockedAt":null
        });
        serde_json::from_value::<RepositoryOwnerIdentity>(owner.clone()).unwrap();
        let owner_schema = serde_json::to_value(schema_for!(RepositoryOwnerIdentity)).unwrap();
        let owner_validator = jsonschema::validator_for(&owner_schema).unwrap();
        assert!(owner_validator.is_valid(&owner));
        assert_eq!(
            owner_schema["required"],
            json!(["username", "computer", "infobase", "lockedAt"])
        );
        for omitted in ["computer", "infobase", "lockedAt"] {
            let mut invalid = owner.as_object().unwrap().clone();
            invalid.remove(omitted);
            let invalid = Value::Object(invalid);
            assert!(serde_json::from_value::<RepositoryOwnerIdentity>(invalid.clone()).is_err());
            assert!(!owner_validator.is_valid(&invalid));
        }
        assert_closed::<RepositoryActorIdentity>();
        assert_closed::<RepositoryOwnerIdentity>();
    }

    #[test]
    fn history_cursor_is_closed_and_repository_version_remains_opaque() {
        let cursor = json!({"throughVersion":"version-10","historyPrefixDigest":SHA_A});
        serde_json::from_value::<RepositoryHistoryCursor>(cursor.clone()).unwrap();
        let mut with_path = cursor.as_object().unwrap().clone();
        with_path.insert("path".into(), json!("/forbidden"));
        assert!(
            serde_json::from_value::<RepositoryHistoryCursor>(Value::Object(with_path)).is_err()
        );
        assert_closed::<RepositoryHistoryCursor>();
    }

    #[test]
    fn routine_evidence_recomputes_its_named_digest_and_requires_actor_null_key() {
        let evidence = RoutineRepositoryVersionClassificationEvidence::new(
            "opaque-v10",
            "unrelated",
            None,
            SHA_A,
            SHA_B,
        )
        .unwrap();
        let value = serde_json::to_value(&evidence).unwrap();
        assert_eq!(value["repositoryActor"], Value::Null);
        assert_eq!(
            value["supportTransitionsDigest"],
            json!(CanonicalEmptyDeltaDigest::VALUE)
        );
        assert_eq!(value["supportGraphUnchanged"], json!(true));
        serde_json::from_value::<RoutineRepositoryVersionClassificationEvidence>(value.clone())
            .unwrap();
        let schema =
            serde_json::to_value(schema_for!(RoutineRepositoryVersionClassificationEvidence))
                .unwrap();
        let validator = jsonschema::validator_for(&schema).unwrap();
        assert!(schema["required"]
            .as_array()
            .unwrap()
            .contains(&json!("repositoryActor")));
        assert!(validator.is_valid(&value));

        let mut omitted = value.as_object().unwrap().clone();
        omitted.remove("repositoryActor");
        let omitted = Value::Object(omitted);
        assert!(
            serde_json::from_value::<RoutineRepositoryVersionClassificationEvidence>(
                omitted.clone()
            )
            .is_err()
        );
        assert!(!validator.is_valid(&omitted));
        let mut wrong_digest = value.as_object().unwrap().clone();
        wrong_digest.insert("classificationDigest".into(), json!(SHA_A));
        assert!(
            serde_json::from_value::<RoutineRepositoryVersionClassificationEvidence>(
                Value::Object(wrong_digest)
            )
            .is_err()
        );
        assert_closed::<RoutineRepositoryVersionClassificationEvidence>();
    }

    #[test]
    fn non_conflicting_evidence_requires_every_safety_literal_and_exact_digest() {
        let evidence = NonConflictingConcurrentEvidence::new(
            "opaque-v11",
            UUID_A,
            SHA_A,
            SHA_B,
            SHA_A,
            SHA_B,
            SHA_A,
        )
        .unwrap();
        let value = serde_json::to_value(&evidence).unwrap();
        for field in [
            "closureDeltaOnlyAddsNonBlockingReferences",
            "disjointFromIntegrationContent",
            "supportGraphUnchanged",
            "validationInputsUnaffected",
            "rootUnchanged",
            "lockedTargetsUnchanged",
        ] {
            assert_eq!(value[field], json!(true));
        }
        assert_eq!(value["blocksApprovedDeletion"], json!(false));
        serde_json::from_value::<super::UnvalidatedNonConflictingConcurrentEvidence>(value.clone())
            .unwrap()
            .into_audited_evidence()
            .unwrap();

        let mut wrong_literal = value.as_object().unwrap().clone();
        wrong_literal.insert("rootUnchanged".into(), json!(false));
        assert!(
            serde_json::from_value::<super::UnvalidatedNonConflictingConcurrentEvidence>(
                Value::Object(wrong_literal)
            )
            .is_err()
        );
        let mut wrong_digest = value.as_object().unwrap().clone();
        wrong_digest.insert("evidenceDigest".into(), json!(SHA_A));
        assert!(
            serde_json::from_value::<super::UnvalidatedNonConflictingConcurrentEvidence>(
                Value::Object(wrong_digest)
            )
            .unwrap()
            .into_audited_evidence()
            .is_err()
        );
        assert_closed::<NonConflictingConcurrentEvidence>();
    }

    fn root_state() -> Value {
        json!({
            "targetKind":"configurationRoot",
            "state":"present",
            "repositoryVersion":"root-v1",
            "targetFingerprint":SHA_A
        })
    }

    fn object_state(object_id: &str) -> Value {
        json!({
            "targetKind":"developmentObject",
            "state":"present",
            "objectId":object_id,
            "repositoryVersion":"object-v1",
            "targetFingerprint":SHA_B
        })
    }

    fn root_lock() -> Value {
        json!({
            "targetKind":"configurationRoot",
            "objectDisplay":"Configuration",
            "reasons":["supportGraphGuard"]
        })
    }

    fn object_lock(object_id: &str) -> Value {
        json!({
            "targetKind":"developmentObject",
            "objectId":object_id,
            "objectDisplay":"Catalog",
            "reasons":["updateTarget","referenceClosure"]
        })
    }

    #[test]
    fn target_state_and_planned_change_collections_enforce_identity_order() {
        serde_json::from_value::<RepositoryTargetStates>(json!([])).unwrap();
        serde_json::from_value::<RepositoryTargetStates>(json!([
            root_state(),
            object_state(OBJECT_A),
            object_state(OBJECT_B)
        ]))
        .unwrap();
        for invalid in [
            json!([object_state(OBJECT_B), object_state(OBJECT_A)]),
            json!([object_state(OBJECT_A), object_state(OBJECT_A)]),
            json!([object_state(OBJECT_A), root_state()]),
        ] {
            assert!(serde_json::from_value::<RepositoryTargetStates>(invalid).is_err());
        }

        let root_change = json!({
            "targetKind":"configurationRoot",
            "action":"modify",
            "objectDisplay":"Configuration",
            "repositoryVersion":"root-v1",
            "targetFingerprint":SHA_A,
            "relevance":"relevant"
        });
        let object_change = |object_id: &str| {
            json!({
                "targetKind":"developmentObject",
                "objectId":object_id,
                "objectDisplay":"Catalog",
                "action":"modify",
                "repositoryVersion":"object-v1",
                "targetFingerprint":SHA_B,
                "relevance":"unrelated"
            })
        };
        serde_json::from_value::<RepositoryPlannedChanges>(json!([])).unwrap();
        serde_json::from_value::<RepositoryPlannedChanges>(json!([
            root_change,
            object_change(OBJECT_A),
            object_change(OBJECT_B)
        ]))
        .unwrap();
        assert!(serde_json::from_value::<RepositoryPlannedChanges>(json!([
            object_change(OBJECT_B),
            object_change(OBJECT_A)
        ]))
        .is_err());
        assert_closed::<RepositoryTargetStates>();
        assert_closed::<RepositoryPlannedChanges>();
    }

    struct RoutineProjectionResolverFixture {
        fold: Option<RoutineUpdateHistoryFoldCapabilityToken>,
        plan: Option<super::update::RoutineSelectiveRepositoryUpdatePlanAuthority>,
        phase: Option<super::lifecycle::RoutineUpdatePhaseAuthority>,
    }

    impl RoutineUpdateProjectionResolver for RoutineProjectionResolverFixture {
        fn resolve_history_fold(
            &mut self,
            _before_anchor: &RepositoryAnchor,
            _history_partition: &super::ValidatedRepositoryHistoryPartition,
        ) -> Result<RoutineUpdateHistoryFoldCapabilityToken, super::RepositoryContractError>
        {
            self.fold.take().ok_or(super::RepositoryContractError(
                "routine fold fixture already consumed",
            ))
        }

        fn resolve_selective_plan(
            &mut self,
            _planned_targets: &RepositoryTargetStates,
            _structural_targets: &[RepositoryTargetIdentity],
        ) -> Result<
            super::update::RoutineSelectiveRepositoryUpdatePlanAuthority,
            super::RepositoryContractError,
        > {
            self.plan.take().ok_or(super::RepositoryContractError(
                "routine plan fixture already consumed",
            ))
        }

        fn resolve_phase(
            &mut self,
            _before_anchor: &RepositoryAnchor,
            _history_partition: &super::ValidatedRepositoryHistoryPartition,
        ) -> Result<super::lifecycle::RoutineUpdatePhaseAuthority, super::RepositoryContractError>
        {
            self.phase.take().ok_or(super::RepositoryContractError(
                "routine phase fixture already consumed",
            ))
        }
    }

    fn routine_projection_fixture(
        plan_targets: RepositoryTargetStates,
        phase: super::lifecycle::RoutineUpdatePhaseAuthority,
    ) -> (
        RepositoryAnchor,
        super::ValidatedRepositoryHistoryPartition,
        RoutineProjectionResolverFixture,
    ) {
        let registry = EvidenceSourceRegistry::task8().unwrap();
        let (partition_json, source_ref, evidence) =
            routine_partition_fixture_for("relevant", "relevantRoutine");
        let partition = validate_routine_fixture(
            partition_json,
            routine_candidate(&registry, source_ref.clone()),
            routine_order(),
            serde_json_canonicalizer::to_vec(&evidence).unwrap(),
            &source_ref,
        )
        .unwrap();
        let anchor = RepositoryAnchorObservationAuthority::test_only(
            Sha256Digest::parse(SHA_A).unwrap(),
            partition.start_cursor().clone(),
            configuration_identity("8.3.27"),
            Sha256Digest::parse(SHA_A).unwrap(),
        )
        .into_anchor()
        .unwrap();
        let before_states: RepositoryTargetStates = serde_json::from_value(json!([{
            "targetKind": "configurationRoot",
            "state": "present",
            "repositoryVersion": "opaque-v0",
            "targetFingerprint": SHA_A,
        }]))
        .unwrap();
        let changes: RepositoryPlannedChanges = serde_json::from_value(json!([{
            "targetKind": "configurationRoot",
            "action": "modify",
            "objectDisplay": "Configuration",
            "repositoryVersion": "opaque-v1",
            "targetFingerprint": SHA_B,
            "relevance": "relevant",
        }]))
        .unwrap();
        let first = partition
            .first_entry()
            .expect("one-entry routine partition has a first entry");
        let fold = RoutineUpdateHistoryFoldCapabilityToken::from_capability_adapter(
            &anchor,
            &partition,
            before_states,
            vec![
                RoutineRepositoryVersionChangeObservation::from_capability_adapter(
                    first.repository_version().clone(),
                    first.semantic_delta_digest().clone(),
                    changes,
                ),
            ],
            CapabilityRowId::parse("routine.history-target-projection.v1").unwrap(),
        )
        .unwrap();
        let lock_targets: RepositoryUpdateLockTargets = serde_json::from_value(json!([{
            "targetKind": "configurationRoot",
            "objectDisplay": "Configuration",
            "reasons": ["updateTarget"],
        }]))
        .unwrap();
        let plan_token =
            super::update::RoutineSelectiveRepositoryUpdateCapabilityToken::from_capability_adapter(
                plan_targets,
                Vec::new(),
                lock_targets,
                CapabilityRowId::parse("selective.objects.v1").unwrap(),
                None,
                Vec::new(),
            )
            .unwrap();
        let plan =
            super::update::RoutineSelectiveRepositoryUpdatePlanAuthority::from_capability_token(
                plan_token,
            )
            .unwrap();
        (
            anchor,
            partition,
            RoutineProjectionResolverFixture {
                fold: Some(fold),
                plan: Some(plan),
                phase: Some(phase),
            },
        )
    }

    #[test]
    fn routine_projection_folds_history_and_rejects_foreign_phase_or_plan() {
        let expected_targets: RepositoryTargetStates = serde_json::from_value(json!([{
            "targetKind": "configurationRoot",
            "state": "present",
            "repositoryVersion": "opaque-v1",
            "targetFingerprint": SHA_B,
        }]))
        .unwrap();
        let (anchor, partition, resolver) = routine_projection_fixture(
            expected_targets.clone(),
            super::lifecycle::RoutineUpdatePhaseAuthority::routine_test_only(
                TaskPhase::Synchronized,
            )
            .unwrap(),
        );
        let projection =
            ValidatedRoutineUpdateProjection::resolve(anchor, partition, None, resolver).unwrap();
        assert_eq!(projection.planned_changes().as_slice().len(), 1);
        assert_eq!(projection.planned_relevant_objects().len(), 1);
        assert!(projection.planned_unrelated_objects().is_empty());
        assert!(projection.structural_changes().as_slice().is_empty());
        assert!(!projection.structural_confirmation_required());
        assert_eq!(
            projection.selective_update_plan().planned_targets(),
            &expected_targets
        );
        assert_eq!(projection.resulting_phase(), TaskPhase::LocalVerified);
        assert!(projection.deferred_repository_advance().is_none());
        assert!(projection.deferred_advance_resolution_digest().is_none());

        let wrong_targets: RepositoryTargetStates = serde_json::from_value(json!([{
            "targetKind": "configurationRoot",
            "state": "present",
            "repositoryVersion": "opaque-v1",
            "targetFingerprint": SHA_A,
        }]))
        .unwrap();
        let (anchor, partition, wrong_plan) = routine_projection_fixture(
            wrong_targets,
            super::lifecycle::RoutineUpdatePhaseAuthority::routine_test_only(
                TaskPhase::Synchronized,
            )
            .unwrap(),
        );
        assert!(
            ValidatedRoutineUpdateProjection::resolve(anchor, partition, None, wrong_plan,)
                .is_err()
        );

        let (anchor, partition, foreign_phase) = routine_projection_fixture(
            expected_targets,
            super::lifecycle::RoutineUpdatePhaseAuthority::foreign_test_only(
                TaskPhase::RecoveryRequired,
            ),
        );
        assert!(
            ValidatedRoutineUpdateProjection::resolve(anchor, partition, None, foreign_phase,)
                .is_err()
        );
    }

    #[test]
    fn repository_target_kind_and_display_have_their_own_exact_contracts() {
        for wire in ["configurationRoot", "developmentObject"] {
            serde_json::from_value::<RepositoryTargetKind>(json!(wire)).unwrap();
        }
        for foreign in ["task", "supportLayer", "root", "object"] {
            assert!(serde_json::from_value::<RepositoryTargetKind>(json!(foreign)).is_err());
        }
        let kind_schema = serde_json::to_value(schema_for!(RepositoryTargetKind)).unwrap();
        assert_eq!(
            kind_schema["enum"],
            json!(["configurationRoot", "developmentObject"])
        );

        let root = serde_json::from_value::<RepositoryTargetIdentity>(
            json!({ "targetKind": "configurationRoot" }),
        )
        .unwrap();
        let object_a = serde_json::from_value::<RepositoryTargetIdentity>(json!({
            "targetKind": "developmentObject",
            "objectId": OBJECT_A,
        }))
        .unwrap();
        let object_b = serde_json::from_value::<RepositoryTargetIdentity>(json!({
            "targetKind": "developmentObject",
            "objectId": OBJECT_B,
        }))
        .unwrap();
        assert!(root < object_a && object_a < object_b);

        for display in ["x".to_owned(), "界".repeat(512)] {
            let mut target = object_lock(OBJECT_A);
            target["objectDisplay"] = json!(display);
            serde_json::from_value::<RepositoryUpdateLockTargets>(json!([root_lock(), target]))
                .unwrap();
        }
        for display in [String::new(), "界".repeat(513), "invalid\tname".to_owned()] {
            let mut target = object_lock(OBJECT_A);
            target["objectDisplay"] = json!(display);
            assert!(
                serde_json::from_value::<RepositoryUpdateLockTargets>(json!([root_lock(), target]))
                    .is_err()
            );
        }
        assert_closed::<RepositoryTargetKind>();
    }

    #[test]
    fn lock_reasons_and_sequences_enforce_exact_forward_and_reverse_order() {
        serde_json::from_value::<RepositoryUpdateLockReasons>(json!([
            "supportGraphGuard",
            "updateTarget",
            "referenceClosure"
        ]))
        .unwrap();
        for invalid in [
            json!([]),
            json!(["updateTarget", "supportGraphGuard"]),
            json!(["updateTarget", "updateTarget"]),
        ] {
            assert!(serde_json::from_value::<RepositoryUpdateLockReasons>(invalid).is_err());
        }

        let forward = json!([root_lock(), object_lock(OBJECT_A), object_lock(OBJECT_B)]);
        serde_json::from_value::<RepositoryUpdateLockTargets>(forward.clone()).unwrap();
        serde_json::from_value::<AcquiredRepositoryUpdateLockTargets>(forward).unwrap();
        serde_json::from_value::<ReleasedRepositoryUpdateLockTargets>(json!([
            object_lock(OBJECT_B),
            object_lock(OBJECT_A),
            root_lock()
        ]))
        .unwrap();

        for invalid in [
            json!([]),
            json!([object_lock(OBJECT_A), root_lock()]),
            json!([root_lock(), object_lock(OBJECT_B), object_lock(OBJECT_A)]),
        ] {
            assert!(
                serde_json::from_value::<RepositoryUpdateLockTargets>(invalid.clone()).is_err()
            );
            assert!(
                serde_json::from_value::<AcquiredRepositoryUpdateLockTargets>(invalid).is_err()
            );
        }
        assert!(
            serde_json::from_value::<ReleasedRepositoryUpdateLockTargets>(json!([
                root_lock(),
                object_lock(OBJECT_A)
            ]))
            .is_err()
        );
        assert_closed::<RepositoryUpdateLockReasons>();
        assert_closed::<RepositoryUpdateLockTargets>();
        assert_closed::<AcquiredRepositoryUpdateLockTargets>();
        assert_closed::<ReleasedRepositoryUpdateLockTargets>();
    }

    #[test]
    fn task9_registry_artifact_digest_constants_match_generated_preimages() {
        let entries = [
            super::EvidenceSourceRegistry::entry(EvidenceKind::RoutineClassification).unwrap(),
            super::EvidenceSourceRegistry::entry(EvidenceKind::SupportPrerequisiteObservation)
                .unwrap(),
            super::EvidenceSourceRegistry::entry(EvidenceKind::NonConflictingConcurrent).unwrap(),
        ];
        let support = &entries[1];
        let registry_digest = super::canonical_contract_digest(
            &super::EvidenceSourceRegistryDigestRecord {
                entries: super::Task8EvidenceSourceRegistryEntries(entries.clone()),
            },
            None,
        )
        .unwrap();
        let actual = [
            support.evidence_schema_digest.as_str(),
            support.digest_record_schema_digest.as_str(),
            support.loader_revision_digest.as_str(),
            support.classification_mapper_revision_digest.as_str(),
            registry_digest.as_str(),
        ];
        let committed = [
            super::TASK8_SUPPORT_OBSERVATION_EVIDENCE_SCHEMA_DIGEST,
            super::TASK8_SUPPORT_OBSERVATION_DIGEST_RECORD_SCHEMA_DIGEST,
            super::TASK8_SUPPORT_OBSERVATION_LOADER_REVISION_DIGEST,
            super::TASK9_SUPPORT_OBSERVATION_MAPPER_REVISION_DIGEST,
            super::TASK9_EVIDENCE_SOURCE_REGISTRY_DIGEST,
        ];
        assert_eq!(actual, committed);
    }

    #[test]
    fn task9_evidence_registry_binds_exact_ordered_schema_loader_and_mapper_digests() {
        let registry = EvidenceSourceRegistry::task9().unwrap();
        assert_eq!(
            registry.evidence_kinds(),
            [
                EvidenceKind::RoutineClassification,
                EvidenceKind::SupportPrerequisiteObservation,
                EvidenceKind::NonConflictingConcurrent,
            ]
        );
        registry.verify_committed_artifacts().unwrap();
        assert_eq!(registry.registry_digest().as_str().len(), 64);
        assert_ne!(
            registry.registry_digest().as_str(),
            super::TASK8_EVIDENCE_SOURCE_REGISTRY_DIGEST
        );

        for entry_index in 0..3 {
            for mutate in 0..4 {
                let mut substituted = registry.clone();
                let entry = &mut substituted.entries[entry_index];
                match mutate {
                    0 => entry.evidence_schema_digest = Sha256Digest::parse(SHA_A).unwrap(),
                    1 => entry.digest_record_schema_digest = Sha256Digest::parse(SHA_A).unwrap(),
                    2 => entry.loader_revision_digest = Sha256Digest::parse(SHA_A).unwrap(),
                    3 => {
                        entry.classification_mapper_revision_digest =
                            Sha256Digest::parse(SHA_A).unwrap()
                    }
                    _ => unreachable!(),
                }
                assert!(substituted.verify_committed_artifacts().is_err());
            }
        }

        let mut reordered = registry.clone();
        reordered.entries.reverse();
        assert!(reordered.verify_committed_artifacts().is_err());

        let mut duplicate = registry.clone();
        duplicate.entries[1] = duplicate.entries[0].clone();
        assert!(duplicate.verify_committed_artifacts().is_err());

        let mut stale_task8_mapper = registry;
        stale_task8_mapper.entries[1].classification_mapper_revision_digest =
            Sha256Digest::parse(super::TASK8_SUPPORT_OBSERVATION_MAPPER_REVISION_DIGEST).unwrap();
        assert!(stale_task8_mapper.verify_committed_artifacts().is_err());
    }

    #[test]
    fn task8_registry_digest_record_schema_is_an_exact_three_position_tuple() {
        let registry = EvidenceSourceRegistry::task8().unwrap();
        let record = super::EvidenceSourceRegistryDigestRecord {
            entries: super::Task8EvidenceSourceRegistryEntries(registry.entries.clone()),
        };
        let valid = serde_json::to_value(record).unwrap();
        let schema =
            serde_json::to_value(schema_for!(super::EvidenceSourceRegistryDigestRecord)).unwrap();
        audit_json_schema(&schema).unwrap();
        let validator = jsonschema::validator_for(&schema).unwrap();
        assert!(validator.is_valid(&valid));

        let tuple_schema = &schema["$defs"]["Task8EvidenceSourceRegistryEntries"];
        assert_eq!(tuple_schema["minItems"], json!(3));
        assert_eq!(tuple_schema["maxItems"], json!(3));
        assert_eq!(tuple_schema["items"], json!(false));
        assert_eq!(
            tuple_schema["prefixItems"][0]["properties"]["evidenceKind"]["const"],
            json!("routineClassification")
        );
        assert_eq!(
            tuple_schema["prefixItems"][1]["properties"]["evidenceKind"]["const"],
            json!("supportPrerequisiteObservation")
        );
        assert_eq!(
            tuple_schema["prefixItems"][2]["properties"]["evidenceKind"]["const"],
            json!("nonConflictingConcurrent")
        );

        let entries = valid["entries"].as_array().unwrap();
        for length in [0, 1, 2, 4] {
            let mut invalid = valid.clone();
            invalid["entries"] = Value::Array(
                (0..length)
                    .map(|index| entries[index % entries.len()].clone())
                    .collect(),
            );
            assert!(
                !validator.is_valid(&invalid),
                "accepted tuple length {length}"
            );
        }

        let mut duplicate = valid.clone();
        duplicate["entries"] = json!([entries[0], entries[0], entries[2]]);
        assert!(!validator.is_valid(&duplicate));

        let mut reversed = valid.clone();
        reversed["entries"] = json!([entries[2], entries[1], entries[0]]);
        assert!(!validator.is_valid(&reversed));

        let mut missing_field = valid.clone();
        missing_field["entries"][0]
            .as_object_mut()
            .unwrap()
            .remove("loaderRevisionDigest");
        assert!(!validator.is_valid(&missing_field));

        let mut extra_field = valid;
        extra_field["entries"][1]["unexpected"] = json!(true);
        assert!(!validator.is_valid(&extra_field));
    }

    #[test]
    fn support_observation_mapper_schema_is_the_exact_eight_row_task9_table() {
        let schema = serde_json::to_value(schema_for!(
            super::SupportObservationEvidenceClassificationMapperRevisionDigestRecord
        ))
        .unwrap();
        audit_json_schema(&schema).unwrap();
        let rows = schema["$defs"]["SupportObservationEvidenceMappings"]["prefixItems"]
            .as_array()
            .unwrap();
        assert_eq!(rows.len(), 8);
        let expected = [
            (
                "routineUnrelated",
                "unrelatedRoutine",
                "explicitNull",
                "explicitNull",
            ),
            (
                "routineRelevant",
                "relevantRoutine",
                "explicitNull",
                "explicitNull",
            ),
            (
                "authorized",
                "authorizedSupport",
                "explicitNull",
                "explicitNull",
            ),
            (
                "externalSupport",
                "externalSupport",
                "copyExternalSupportDisjointnessDigest",
                "explicitNull",
            ),
            (
                "preArmExternal",
                "preArmExternal",
                "explicitNull",
                "explicitNull",
            ),
            (
                "actionCorrection",
                "corrective",
                "explicitNull",
                "copyCorrectiveInstructionDigest",
            ),
            (
                "externalConflictCorrection",
                "corrective",
                "explicitNull",
                "copySupportConflictInstructionDigest",
            ),
            ("invalid", "invalid", "explicitNull", "explicitNull"),
        ];
        for (row, (source_case, classification, external_projection, corrective_projection)) in
            rows.iter().zip(expected)
        {
            assert_eq!(row["properties"]["sourceCase"]["const"], json!(source_case));
            assert_eq!(
                row["properties"]["partitionClassification"]["const"],
                json!(classification)
            );
            for (field, projection) in [
                ("rootDeltaDigestProjection", "copyRootDeltaDigest"),
                ("contentDeltaDigestProjection", "copyContentDeltaDigest"),
                ("classificationDigestProjection", "copyClassificationDigest"),
                (
                    "externalSupportDisjointnessDigestProjection",
                    external_projection,
                ),
                (
                    "correctiveInstructionDigestProjection",
                    corrective_projection,
                ),
                (
                    "nonConflictingConcurrentEvidenceDigestProjection",
                    "explicitNull",
                ),
            ] {
                assert_eq!(row["properties"][field]["const"], json!(projection));
            }
            assert_eq!(row["additionalProperties"], json!(false));
        }
        let mapping_tuple = &schema["$defs"]["SupportObservationEvidenceMappings"];
        assert_eq!(mapping_tuple["minItems"], json!(8));
        assert_eq!(mapping_tuple["maxItems"], json!(8));
        assert_eq!(mapping_tuple["items"], json!(false));
    }

    #[test]
    fn task8_registry_schema_preimages_are_closed_and_recompute_to_committed_digests() {
        let registry = EvidenceSourceRegistry::task8().unwrap();
        assert_closed::<RoutineRepositoryVersionClassificationEvidence>();
        assert_closed::<super::RoutineRepositoryVersionClassificationEvidenceDigestRecord>();
        assert_closed::<NonConflictingConcurrentEvidence>();
        assert_closed::<super::NonConflictingConcurrentEvidenceDigestRecord>();
        assert_closed::<super::EvidenceLoaderRevisionDigestRecord>();
        assert_closed::<super::RoutineEvidenceClassificationMapperRevisionDigestRecord>();
        assert_closed::<super::NonConflictingEvidenceClassificationMapperRevisionDigestRecord>();
        assert_closed::<super::SupportPrerequisiteVersionObservation>();
        assert_closed::<super::SupportPrerequisiteVersionObservationDigestRecord>();
        assert_closed::<super::SupportObservationEvidenceClassificationMapperRevisionDigestRecord>(
        );
        assert_closed::<super::EvidenceSourceIndexProofDigestRecord>();
        assert_closed::<super::EvidenceSourceIndexProof>();
        assert_closed::<super::RepositoryHistoryPartitionDigestRecord>();
        assert_closed::<UnvalidatedRepositoryHistoryPartition>();
        assert_closed::<super::RepositorySemanticDeltaDigestRecord>();

        let expected = [
            (
                super::schema_digest::<RoutineRepositoryVersionClassificationEvidence>().unwrap(),
                super::schema_digest::<
                    super::RoutineRepositoryVersionClassificationEvidenceDigestRecord,
                >()
                .unwrap(),
            ),
            (
                super::schema_digest::<super::SupportPrerequisiteVersionObservation>().unwrap(),
                super::schema_digest::<super::SupportPrerequisiteVersionObservationDigestRecord>()
                    .unwrap(),
            ),
            (
                super::schema_digest::<NonConflictingConcurrentEvidence>().unwrap(),
                super::schema_digest::<super::NonConflictingConcurrentEvidenceDigestRecord>()
                    .unwrap(),
            ),
        ];
        for (entry, (evidence_schema_digest, digest_record_schema_digest)) in
            registry.entries.iter().zip(expected)
        {
            assert_eq!(entry.evidence_schema_digest, evidence_schema_digest);
            assert_eq!(
                entry.digest_record_schema_digest,
                digest_record_schema_digest
            );
        }
    }

    #[derive(Clone)]
    struct FakeIndex {
        candidates: BTreeMap<String, EvidenceSourceIndexCandidate>,
    }

    impl EvidenceSourceIndex for FakeIndex {
        fn candidate_for(
            &self,
            repository_version: &super::RepositoryVersion,
            _registry: &EvidenceSourceRegistry,
        ) -> Result<EvidenceSourceIndexCandidate, super::RepositoryContractError> {
            self.candidates
                .get(repository_version.as_str())
                .cloned()
                .ok_or(super::RepositoryContractError(
                    "missing fake index candidate",
                ))
        }
    }

    struct UnexpectedIndex;

    impl EvidenceSourceIndex for UnexpectedIndex {
        fn candidate_for(
            &self,
            _repository_version: &super::RepositoryVersion,
            _registry: &EvidenceSourceRegistry,
        ) -> Result<EvidenceSourceIndexCandidate, super::RepositoryContractError> {
            panic!("source index must not be consulted before structural rejection")
        }
    }

    #[derive(Clone)]
    struct FakeOrder {
        evidence: RepositoryHistoryOrderEvidence,
    }

    impl RepositoryHistoryOrderResolver for FakeOrder {
        fn order_evidence(
            &self,
            _from_exclusive: &RepositoryHistoryCursor,
            _through_inclusive: &RepositoryHistoryCursor,
        ) -> Result<RepositoryHistoryOrderEvidence, super::RepositoryContractError> {
            Ok(self.evidence.clone())
        }
    }

    struct UnexpectedOrder;

    impl RepositoryHistoryOrderResolver for UnexpectedOrder {
        fn order_evidence(
            &self,
            _from_exclusive: &RepositoryHistoryCursor,
            _through_inclusive: &RepositoryHistoryCursor,
        ) -> Result<RepositoryHistoryOrderEvidence, super::RepositoryContractError> {
            panic!("history order must not be consulted before structural rejection")
        }
    }

    #[derive(Default)]
    struct FakeEvidenceBytes {
        bytes: BTreeMap<(EvidenceKind, String), Vec<u8>>,
    }

    impl RepositoryHistoryEvidenceBytesResolver for FakeEvidenceBytes {
        fn load_canonical_evidence_bytes(
            &self,
            reference: &RepositoryHistorySourceEvidenceRef,
        ) -> Result<Vec<u8>, super::RepositoryContractError> {
            self.bytes
                .get(&(
                    reference.evidence_kind(),
                    reference.evidence_digest().as_str().to_owned(),
                ))
                .cloned()
                .ok_or(super::RepositoryContractError(
                    "missing fake evidence bytes",
                ))
        }
    }

    #[derive(Clone)]
    struct FakeCorrectiveEvidence {
        corrective_sources: BTreeMap<String, FrozenSupportCorrectiveInstructionSourceAuthority>,
        conflict_sources: BTreeMap<String, FrozenSupportConflictInstructionSourceAuthority>,
        expected_actor: RepositoryActorIdentity,
        expected_external_ownership: ExternalSupportOwnershipEvidence,
        action_attribution_valid: bool,
        external_attribution_valid: bool,
        frozen_support_action_id: UnicaId,
        support_history_order: FakeSupportHistoryOrder,
    }

    impl SupportCorrectiveEvidenceResolver for FakeCorrectiveEvidence {
        fn historical_frozen_support_corrective_instruction_source(
            &self,
            repository_version: &RepositoryVersion,
        ) -> Result<FrozenSupportCorrectiveInstructionSourceAuthority, super::RepositoryContractError>
        {
            self.corrective_sources
                .get(repository_version.as_str())
                .cloned()
                .ok_or(super::RepositoryContractError(
                    "missing fake corrective instruction",
                ))
        }

        fn historical_frozen_support_conflict_instruction_source(
            &self,
            repository_version: &RepositoryVersion,
        ) -> Result<FrozenSupportConflictInstructionSourceAuthority, super::RepositoryContractError>
        {
            self.conflict_sources
                .get(repository_version.as_str())
                .cloned()
                .ok_or(super::RepositoryContractError(
                    "missing fake conflict instruction",
                ))
        }

        fn support_history_order_authority(&self) -> &dyn SupportHistoryOrderAuthority {
            &self.support_history_order
        }

        fn frozen_support_action_id(&self) -> &UnicaId {
            &self.frozen_support_action_id
        }

        fn validate_action_correction_attribution(
            &self,
            repository_version: &RepositoryVersion,
            repository_actor: &RepositoryActorIdentity,
            instruction: &SupportCorrectiveInstruction,
        ) -> Result<(), super::RepositoryContractError> {
            if self.action_attribution_valid
                && self
                    .corrective_sources
                    .contains_key(repository_version.as_str())
                && repository_actor == &self.expected_actor
                && instruction.support_action_id() == &self.frozen_support_action_id
            {
                Ok(())
            } else {
                Err(super::RepositoryContractError(
                    "fake action attribution rejected",
                ))
            }
        }

        fn validate_external_ownership_attribution(
            &self,
            repository_version: &RepositoryVersion,
            repository_actor: &RepositoryActorIdentity,
            _root_delta_digest: &Sha256Digest,
            _content_delta_digest: &Sha256Digest,
            evidence: &ExternalSupportOwnershipEvidence,
        ) -> Result<(), super::RepositoryContractError> {
            if self.external_attribution_valid
                && self
                    .conflict_sources
                    .contains_key(repository_version.as_str())
                && repository_actor == &self.expected_actor
                && evidence == &self.expected_external_ownership
            {
                Ok(())
            } else {
                Err(super::RepositoryContractError(
                    "fake external attribution rejected",
                ))
            }
        }
    }

    #[derive(Debug, Clone, Copy)]
    struct FakeSupportHistoryOrder;

    impl SupportHistoryOrderAuthority for FakeSupportHistoryOrder {
        fn compare_versions(
            &self,
            left: &RepositoryVersion,
            right: &RepositoryVersion,
        ) -> Result<Ordering, SupportContractError> {
            Ok(left.as_str().cmp(right.as_str()))
        }

        fn compare_cursors(
            &self,
            left: &RepositoryHistoryCursor,
            right: &RepositoryHistoryCursor,
        ) -> Result<Ordering, SupportContractError> {
            Ok(left
                .through_version
                .as_str()
                .cmp(right.through_version.as_str())
                .then_with(|| {
                    left.history_prefix_digest
                        .as_str()
                        .cmp(right.history_prefix_digest.as_str())
                }))
        }
    }

    fn test_id(value: &str) -> UnicaId {
        UnicaId::parse(value).unwrap()
    }

    fn test_sha(value: &str) -> Sha256Digest {
        Sha256Digest::parse(value).unwrap()
    }

    fn corrective_instruction_for(support_action_id: &str) -> SupportCorrectiveInstruction {
        corrective_instruction_for_deltas(support_action_id, SHA_A, SHA_B)
    }

    fn corrective_instruction_for_deltas(
        support_action_id: &str,
        root_delta_digest: &str,
        content_delta_digest: &str,
    ) -> SupportCorrectiveInstruction {
        let root_lock = SupportRecoveryLockTarget::configuration_root(
            RepositoryTargetDisplay::parse("Configuration").unwrap(),
            vec![RepositoryUpdateLockReason::SupportGraphGuard],
        )
        .unwrap();
        let locks = SupportRecoveryLockTargets::new(vec![root_lock]).unwrap();
        let transition =
            SupportRecoveryTransition::ordinary(SupportTransition::enable_configuration_changes(
                RepositoryTargetDisplay::parse("Configuration").unwrap(),
                SupportLayerId::parse("layer-a").unwrap(),
            ));
        SupportCorrectiveInstruction::new(
            SupportCorrectiveInstructionAuthority::test_only(
                test_id(support_action_id),
                SupportActionPurpose::AbandonmentCleanup,
                ManualSupportTargetMode::ReservedOriginal,
                RepositoryUsername::parse("repository-user").unwrap(),
                None,
                serde_json::from_value(cursor("opaque-v0", SHA_A)).unwrap(),
                locks.clone(),
                locks,
                vec![transition],
                Vec::new(),
                Vec::new(),
                Vec::new(),
                test_sha(root_delta_digest),
                test_sha(content_delta_digest),
            )
            .unwrap(),
        )
        .unwrap()
    }

    fn corrective_instruction() -> SupportCorrectiveInstruction {
        corrective_instruction_for(UUID_A)
    }

    fn conflict_instruction_for(conflict_resolution_id: &str) -> SupportConflictInstruction {
        let display = RepositoryTargetDisplay::parse("Configuration").unwrap();
        let layer = SupportLayerId::parse("layer-a").unwrap();
        let transition =
            SupportTransition::enable_configuration_changes(display.clone(), layer.clone());
        let conflict = SupportTransitionConflict::from_capability_adapter(
            RepositoryVersion::parse("opaque-v1").unwrap(),
            RequiredNullable::value(
                serde_json::from_value::<RepositoryActorIdentity>(support_actor()).unwrap(),
            ),
            None,
            display,
            layer,
            transition,
            test_sha(SHA_A),
            SupportTransitionOverlapKind::SameTarget,
            Diagnostic::parse("external support overlap").unwrap(),
        )
        .unwrap();
        let conflicts =
            SupportTransitionConflicts::new(vec![conflict], &FakeSupportHistoryOrder).unwrap();
        SupportConflictInstruction::new(test_id(conflict_resolution_id), conflicts, test_sha(SHA_B))
            .unwrap()
    }

    fn conflict_instruction() -> SupportConflictInstruction {
        conflict_instruction_for(UUID_A)
    }

    fn fake_corrective_resolver(
        corrective_bytes: Option<Vec<u8>>,
        conflict_bytes: Option<Vec<u8>>,
    ) -> FakeCorrectiveEvidence {
        let historical_corrective_instruction = corrective_instruction();
        let historical_conflict_instruction = conflict_instruction();
        FakeCorrectiveEvidence {
            corrective_sources: corrective_bytes
                .map(|bytes| {
                    let version = RepositoryVersion::parse("opaque-v1").unwrap();
                    BTreeMap::from([(
                        version.as_str().to_owned(),
                        FrozenSupportCorrectiveInstructionSourceAuthority::from_capability_adapter(
                            version,
                            historical_corrective_instruction
                                .support_action_id()
                                .clone(),
                            historical_corrective_instruction
                                .corrective_instruction_digest()
                                .clone(),
                            bytes,
                        ),
                    )])
                })
                .unwrap_or_default(),
            conflict_sources: conflict_bytes
                .map(|bytes| {
                    let version = RepositoryVersion::parse("opaque-v1").unwrap();
                    BTreeMap::from([(
                        version.as_str().to_owned(),
                        FrozenSupportConflictInstructionSourceAuthority::from_capability_adapter(
                            version,
                            historical_corrective_instruction
                                .support_action_id()
                                .clone(),
                            historical_conflict_instruction
                                .conflict_resolution_id()
                                .clone(),
                            historical_conflict_instruction
                                .support_conflict_instruction_digest()
                                .clone(),
                            bytes,
                        ),
                    )])
                })
                .unwrap_or_default(),
            expected_actor: serde_json::from_value(support_actor()).unwrap(),
            expected_external_ownership:
                ExternalSupportOwnershipEvidence::support_prerequisite_receipt(
                    test_id(UUID_A),
                    test_sha(SHA_A),
                ),
            action_attribution_valid: true,
            external_attribution_valid: true,
            frozen_support_action_id: test_id(UUID_A),
            support_history_order: FakeSupportHistoryOrder,
        }
    }

    fn cursor(version: &str, digest: &str) -> Value {
        json!({"throughVersion":version,"historyPrefixDigest":digest})
    }

    fn routine_partition_fixture() -> (
        Value,
        RepositoryHistorySourceEvidenceRef,
        RoutineRepositoryVersionClassificationEvidence,
    ) {
        routine_partition_fixture_for("unrelated", "unrelatedRoutine")
    }

    fn routine_partition_fixture_for(
        relevance: &str,
        partition_classification: &str,
    ) -> (
        Value,
        RepositoryHistorySourceEvidenceRef,
        RoutineRepositoryVersionClassificationEvidence,
    ) {
        let evidence = RoutineRepositoryVersionClassificationEvidence::new(
            "opaque-v1",
            relevance,
            None,
            SHA_A,
            SHA_B,
        )
        .unwrap();
        let evidence_value = serde_json::to_value(&evidence).unwrap();
        let evidence_digest = evidence_value["classificationDigest"].as_str().unwrap();
        let source_ref = RepositoryHistorySourceEvidenceRef::new(
            EvidenceKind::RoutineClassification,
            evidence_digest,
        )
        .unwrap();
        let semantic = json!({
            "repositoryVersion":"opaque-v1",
            "partitionClassification":partition_classification,
            "rootDeltaDigest":SHA_A,
            "contentDeltaDigest":SHA_B,
            "classificationDigest":evidence_digest,
            "externalSupportDisjointnessDigest":null,
            "correctiveInstructionDigest":null,
            "nonConflictingConcurrentEvidenceDigest":null
        });
        let entry = json!({
            "repositoryVersion":"opaque-v1",
            "classification":partition_classification,
            "semanticDeltaDigest":test_digest(&semantic),
            "sourceEvidenceRef":serde_json::to_value(&source_ref).unwrap()
        });
        let from = cursor("opaque-v0", SHA_A);
        let through = cursor("opaque-v1", SHA_B);
        let digest_record = json!({
            "fromExclusive":from,
            "throughInclusive":through,
            "entries":[entry]
        });
        let mut partition = digest_record.as_object().unwrap().clone();
        partition.insert("partitionDigest".into(), json!(test_digest(&digest_record)));
        (Value::Object(partition), source_ref, evidence)
    }

    fn recalculate_partition_digest(partition: &mut Value) {
        let object = partition.as_object_mut().unwrap();
        object.remove("partitionDigest");
        let digest = test_digest(&Value::Object(object.clone()));
        object.insert("partitionDigest".into(), json!(digest));
    }

    fn routine_candidate(
        registry: &EvidenceSourceRegistry,
        source_ref: RepositoryHistorySourceEvidenceRef,
    ) -> EvidenceSourceIndexCandidate {
        EvidenceSourceIndexCandidate::from_capability_adapter(
            "opaque-v1",
            registry.registry_digest().as_str(),
            UUID_A,
            vec![
                EvidenceSourceIndexCandidateRow::available(
                    EvidenceKind::RoutineClassification,
                    vec![source_ref],
                ),
                EvidenceSourceIndexCandidateRow::absent(
                    EvidenceKind::SupportPrerequisiteObservation,
                ),
                EvidenceSourceIndexCandidateRow::absent(EvidenceKind::NonConflictingConcurrent),
            ],
        )
        .unwrap()
    }

    fn routine_order() -> RepositoryHistoryOrderEvidence {
        RepositoryHistoryOrderEvidence::from_capability_adapter(
            "history-order-v1",
            serde_json::from_value(cursor("opaque-v0", SHA_A)).unwrap(),
            serde_json::from_value(cursor("opaque-v1", SHA_B)).unwrap(),
            vec![serde_json::from_value(cursor("opaque-v1", SHA_B)).unwrap()],
        )
        .unwrap()
    }

    pub(super) fn validated_task_commit_partition_for_results(
        commit: &crate::domain::branched_development::contracts::results::repository::ValidatedCommitObjectAuthority,
    ) -> super::ValidatedTaskCommitHistoryPartition {
        let registry = EvidenceSourceRegistry::task8().unwrap();
        let from: RepositoryHistoryCursor =
            serde_json::from_value(cursor("opaque-v0", SHA_A)).unwrap();
        let through = RepositoryHistoryCursor::new(
            commit.repository_version().clone(),
            Sha256Digest::parse(SHA_B).unwrap(),
        );
        let order = FakeOrder {
            evidence: RepositoryHistoryOrderEvidence::from_capability_adapter(
                "history-order-v1",
                from.clone(),
                through.clone(),
                vec![through.clone()],
            )
            .unwrap(),
        };
        let mut partition = json!({
            "fromExclusive": from,
            "throughInclusive": through,
            "entries": [{
                "repositoryVersion": commit.repository_version(),
                "classification": "taskCommit",
                "semanticDeltaDigest": commit.committed_objects_digest(),
            }],
        });
        recalculate_partition_digest(&mut partition);
        let bytes = FakeEvidenceBytes::default();
        RepositoryHistoryPartitionResolver::new(&registry, &UnexpectedIndex, &order, &bytes)
            .validate_task_commit_partition(serde_json::from_value(partition).unwrap(), commit)
            .unwrap()
    }

    fn support_actor() -> Value {
        json!({
            "username":"repository-user",
            "computer":null,
            "infobase":null
        })
    }

    fn finalize_support_observation(mut value: Value) -> Value {
        let mut digest_record = value.clone();
        digest_record
            .as_object_mut()
            .unwrap()
            .remove("classificationDigest");
        value["classificationDigest"] = json!(test_digest(&digest_record));
        value
    }

    fn support_observation_fixture(
        source_case: &str,
    ) -> (Value, RepositoryHistorySourceEvidenceRef, Value) {
        let (observation, partition_classification, root_delta, content_delta, external_digest) =
            match source_case {
                "routineUnrelated" | "routineRelevant" => {
                    let relevance = if source_case == "routineUnrelated" {
                        "unrelated"
                    } else {
                        "relevant"
                    };
                    let partition_classification = if source_case == "routineUnrelated" {
                        "unrelatedRoutine"
                    } else {
                        "relevantRoutine"
                    };
                    (
                        finalize_support_observation(json!({
                            "repositoryVersion":"opaque-v1",
                            "classification":"routine",
                            "classificationDigest":SHA_A,
                            "mismatchKinds":[],
                            "repositoryActor":support_actor(),
                            "relevance":relevance,
                            "rootDeltaDigest":SHA_A,
                            "contentDeltaDigest":SHA_B,
                            "supportTransitionsDigest":CanonicalEmptyDeltaDigest::VALUE,
                            "supportGraphUnchanged":true
                        })),
                        partition_classification,
                        Some(SHA_A),
                        Some(SHA_B),
                        None,
                    )
                }
                "authorized" => (
                    finalize_support_observation(json!({
                        "repositoryVersion":"opaque-v1",
                        "classification":"authorized",
                        "classificationDigest":SHA_A,
                        "mismatchKinds":[],
                        "repositoryActor":support_actor(),
                        "supportActionId":UUID_A,
                        "supportActionDigest":SHA_A,
                        "armingReceiptId":UUID_A,
                        "armingReceiptDigest":SHA_A,
                        "firstRootSupportAfterArming":true,
                        "actionAttributionEvidenceDigest":SHA_A,
                        "authorizedTransitionsDigest":SHA_B,
                        "manualTargetMode":"reservedOriginal",
                        "rootDeltaDigest":SHA_A,
                        "contentDeltaDigest":CanonicalEmptyDeltaDigest::VALUE,
                        "observedSupportTransitionsDigest":SHA_B,
                        "rootDeltaContainsOnlyAuthorizedSupportTransitions":true
                    })),
                    "authorizedSupport",
                    Some(SHA_A),
                    Some(CanonicalEmptyDeltaDigest::VALUE),
                    None,
                ),
                "externalSupport" => (
                    finalize_support_observation(json!({
                        "repositoryVersion":"opaque-v1",
                        "classification":"externalSupport",
                        "classificationDigest":SHA_A,
                        "mismatchKinds":[],
                        "repositoryActor":support_actor(),
                        "rootDeltaDigest":SHA_A,
                        "contentDeltaDigest":CanonicalEmptyDeltaDigest::VALUE,
                        "provenNotThisAction":true,
                        "overlapWithAuthorizedTransitions":false,
                        "supportOnlyDelta":true,
                        "externalSupportDisjointnessDigest":SHA_B,
                        "externalOwnershipEvidence":{
                            "kind":"supportPrerequisiteReceipt",
                            "receiptId":UUID_A,
                            "receiptDigest":SHA_A
                        }
                    })),
                    "externalSupport",
                    Some(SHA_A),
                    Some(CanonicalEmptyDeltaDigest::VALUE),
                    Some(SHA_B),
                ),
                "preArmExternal" => (
                    finalize_support_observation(json!({
                        "repositoryVersion":"opaque-v1",
                        "classification":"preArmExternal",
                        "classificationDigest":SHA_A,
                        "mismatchKinds":["armingOrderViolated"],
                        "pendingSupportActionId":UUID_A,
                        "pendingSupportActionDigest":SHA_A,
                        "authorizationState":"awaitingArm",
                        "armingReceiptAbsent":true,
                        "repositoryActor":support_actor(),
                        "rootDeltaDigest":SHA_A,
                        "contentDeltaDigest":SHA_B,
                        "supportTransitionsDigest":SHA_A,
                        "preserveAsExternalBaseline":true
                    })),
                    "preArmExternal",
                    Some(SHA_A),
                    Some(SHA_B),
                    None,
                ),
                "invalid" => (
                    finalize_support_observation(json!({
                        "repositoryVersion":"opaque-v1",
                        "classification":"invalid",
                        "classificationDigest":SHA_A,
                        "mismatchKinds":["versionUnattributed"],
                        "provenance":"unattributed",
                        "repositoryActor":null,
                        "rootDeltaDigest":null,
                        "contentDeltaDigest":null,
                        "missingEvidenceKinds":["repositoryActorUnavailable"]
                    })),
                    "invalid",
                    None,
                    None,
                    None,
                ),
                "corrective" => (
                    finalize_support_observation(json!({
                        "repositoryVersion":"opaque-v1",
                        "classification":"corrective",
                        "classificationDigest":SHA_A,
                        "mismatchKinds":[],
                        "correctionKind":"actionCorrection",
                        "repositoryActor":support_actor(),
                        "manualTargetMode":"reservedOriginal",
                        "rootDeltaDigest":SHA_A,
                        "contentDeltaDigest":SHA_B,
                        "correctiveInstructionDigest":SHA_A
                    })),
                    "corrective",
                    Some(SHA_A),
                    Some(SHA_B),
                    None,
                ),
                _ => panic!("unknown support source case {source_case}"),
            };

        serde_json::from_value::<super::SupportPrerequisiteVersionObservation>(observation.clone())
            .unwrap();
        let classification_digest = observation["classificationDigest"].as_str().unwrap();
        let source_ref = RepositoryHistorySourceEvidenceRef::new(
            EvidenceKind::SupportPrerequisiteObservation,
            classification_digest,
        )
        .unwrap();
        let semantic = json!({
            "repositoryVersion":"opaque-v1",
            "partitionClassification":partition_classification,
            "rootDeltaDigest":root_delta,
            "contentDeltaDigest":content_delta,
            "classificationDigest":classification_digest,
            "externalSupportDisjointnessDigest":external_digest,
            "correctiveInstructionDigest":if source_case == "corrective" { Some(SHA_A) } else { None },
            "nonConflictingConcurrentEvidenceDigest":null
        });
        let entry = json!({
            "repositoryVersion":"opaque-v1",
            "classification":partition_classification,
            "semanticDeltaDigest":test_digest(&semantic),
            "sourceEvidenceRef":serde_json::to_value(&source_ref).unwrap()
        });
        let mut partition = json!({
            "fromExclusive":cursor("opaque-v0", SHA_A),
            "throughInclusive":cursor("opaque-v1", SHA_B),
            "entries":[entry]
        });
        recalculate_partition_digest(&mut partition);
        (partition, source_ref, observation)
    }

    fn corrective_partition_from_observation(
        observation: Value,
        instruction_digest: &str,
    ) -> (Value, RepositoryHistorySourceEvidenceRef, Value) {
        let observation = finalize_support_observation(observation);
        serde_json::from_value::<super::SupportPrerequisiteVersionObservation>(observation.clone())
            .unwrap();
        let classification_digest = observation["classificationDigest"].as_str().unwrap();
        let source_ref = RepositoryHistorySourceEvidenceRef::new(
            EvidenceKind::SupportPrerequisiteObservation,
            classification_digest,
        )
        .unwrap();
        let repository_version = observation["repositoryVersion"].as_str().unwrap();
        let semantic = json!({
            "repositoryVersion":repository_version,
            "partitionClassification":"corrective",
            "rootDeltaDigest":observation["rootDeltaDigest"],
            "contentDeltaDigest":observation["contentDeltaDigest"],
            "classificationDigest":classification_digest,
            "externalSupportDisjointnessDigest":null,
            "correctiveInstructionDigest":instruction_digest,
            "nonConflictingConcurrentEvidenceDigest":null
        });
        let entry = json!({
            "repositoryVersion":repository_version,
            "classification":"corrective",
            "semanticDeltaDigest":test_digest(&semantic),
            "sourceEvidenceRef":serde_json::to_value(&source_ref).unwrap()
        });
        let mut partition = json!({
            "fromExclusive":cursor("opaque-v0", SHA_A),
            "throughInclusive":cursor(repository_version, SHA_B),
            "entries":[entry]
        });
        recalculate_partition_digest(&mut partition);
        (partition, source_ref, observation)
    }

    fn action_corrective_partition_fixture(
        instruction: &SupportCorrectiveInstruction,
    ) -> (Value, RepositoryHistorySourceEvidenceRef, Value) {
        action_corrective_partition_fixture_for("opaque-v1", instruction)
    }

    fn action_corrective_partition_fixture_for(
        repository_version: &str,
        instruction: &SupportCorrectiveInstruction,
    ) -> (Value, RepositoryHistorySourceEvidenceRef, Value) {
        corrective_partition_from_observation(
            json!({
                "repositoryVersion":repository_version,
                "classification":"corrective",
                "classificationDigest":SHA_A,
                "mismatchKinds":[],
                "correctionKind":"actionCorrection",
                "repositoryActor":support_actor(),
                "manualTargetMode":"reservedOriginal",
                "rootDeltaDigest":instruction.required_root_delta_digest().as_str(),
                "contentDeltaDigest":instruction.required_content_delta_digest().as_str(),
                "correctiveInstructionDigest":instruction.corrective_instruction_digest().as_str()
            }),
            instruction.corrective_instruction_digest().as_str(),
        )
    }

    fn external_corrective_partition_fixture(
        instruction: &SupportConflictInstruction,
    ) -> (Value, RepositoryHistorySourceEvidenceRef, Value) {
        external_corrective_partition_fixture_for("opaque-v1", instruction)
    }

    fn external_corrective_partition_fixture_for(
        repository_version: &str,
        instruction: &SupportConflictInstruction,
    ) -> (Value, RepositoryHistorySourceEvidenceRef, Value) {
        corrective_partition_from_observation(
            json!({
                "repositoryVersion":repository_version,
                "classification":"corrective",
                "classificationDigest":SHA_A,
                "mismatchKinds":[],
                "correctionKind":"externalConflictCorrection",
                "repositoryActor":support_actor(),
                "rootDeltaDigest":SHA_A,
                "contentDeltaDigest":SHA_B,
                "conflictResolutionId":instruction.conflict_resolution_id().as_str(),
                "supportConflictInstructionDigest":instruction
                    .support_conflict_instruction_digest()
                    .as_str(),
                "finalBaselineDigest":instruction.required_final_baseline_digest().as_str(),
                "externalOwnershipEvidence":{
                    "kind":"supportPrerequisiteReceipt",
                    "receiptId":UUID_A,
                    "receiptDigest":SHA_A
                }
            }),
            instruction.support_conflict_instruction_digest().as_str(),
        )
    }

    fn support_candidate(
        registry: &EvidenceSourceRegistry,
        source_ref: RepositoryHistorySourceEvidenceRef,
        ncc_available: bool,
    ) -> EvidenceSourceIndexCandidate {
        support_candidate_for("opaque-v1", registry, source_ref, ncc_available)
    }

    fn support_candidate_for(
        repository_version: &str,
        registry: &EvidenceSourceRegistry,
        source_ref: RepositoryHistorySourceEvidenceRef,
        ncc_available: bool,
    ) -> EvidenceSourceIndexCandidate {
        let ncc_row = if ncc_available {
            EvidenceSourceIndexCandidateRow::available(
                EvidenceKind::NonConflictingConcurrent,
                vec![RepositoryHistorySourceEvidenceRef::new(
                    EvidenceKind::NonConflictingConcurrent,
                    SHA_B,
                )
                .unwrap()],
            )
        } else {
            EvidenceSourceIndexCandidateRow::absent(EvidenceKind::NonConflictingConcurrent)
        };
        EvidenceSourceIndexCandidate::from_capability_adapter(
            repository_version,
            registry.registry_digest().as_str(),
            UUID_A,
            vec![
                EvidenceSourceIndexCandidateRow::available(
                    EvidenceKind::RoutineClassification,
                    vec![RepositoryHistorySourceEvidenceRef::new(
                        EvidenceKind::RoutineClassification,
                        SHA_A,
                    )
                    .unwrap()],
                ),
                EvidenceSourceIndexCandidateRow::available(
                    EvidenceKind::SupportPrerequisiteObservation,
                    vec![source_ref],
                ),
                ncc_row,
            ],
        )
        .unwrap()
    }

    fn validate_support_fixture(
        partition_json: Value,
        candidate: EvidenceSourceIndexCandidate,
        bytes: Vec<u8>,
        source_ref: &RepositoryHistorySourceEvidenceRef,
    ) -> Result<super::ValidatedRepositoryHistoryPartition, super::RepositoryContractError> {
        let registry = EvidenceSourceRegistry::task8().unwrap();
        let index = FakeIndex {
            candidates: BTreeMap::from([("opaque-v1".into(), candidate)]),
        };
        let order = FakeOrder {
            evidence: routine_order(),
        };
        let evidence_bytes = FakeEvidenceBytes {
            bytes: BTreeMap::from([(
                (
                    EvidenceKind::SupportPrerequisiteObservation,
                    source_ref.evidence_digest().as_str().to_owned(),
                ),
                bytes,
            )]),
        };
        RepositoryHistoryPartitionResolver::new(&registry, &index, &order, &evidence_bytes)
            .validate(serde_json::from_value(partition_json).unwrap())
    }

    fn validate_corrective_fixture(
        partition_json: Value,
        source_ref: &RepositoryHistorySourceEvidenceRef,
        observation_bytes: Vec<u8>,
        corrective_resolver: Option<&dyn SupportCorrectiveEvidenceResolver>,
    ) -> Result<super::ValidatedRepositoryHistoryPartition, super::RepositoryContractError> {
        let registry = EvidenceSourceRegistry::task9().unwrap();
        let index = FakeIndex {
            candidates: BTreeMap::from([(
                "opaque-v1".into(),
                support_candidate(&registry, source_ref.clone(), false),
            )]),
        };
        let order = FakeOrder {
            evidence: routine_order(),
        };
        let evidence_bytes = FakeEvidenceBytes {
            bytes: BTreeMap::from([(
                (
                    EvidenceKind::SupportPrerequisiteObservation,
                    source_ref.evidence_digest().as_str().to_owned(),
                ),
                observation_bytes,
            )]),
        };
        let resolver =
            RepositoryHistoryPartitionResolver::new(&registry, &index, &order, &evidence_bytes);
        match corrective_resolver {
            Some(corrective_resolver) => resolver
                .with_corrective_evidence_resolver(corrective_resolver)
                .validate(serde_json::from_value(partition_json).unwrap()),
            None => resolver.validate(serde_json::from_value(partition_json).unwrap()),
        }
    }

    fn combine_corrective_partitions(partitions: &[Value]) -> Value {
        assert!(!partitions.is_empty());
        let entries = partitions
            .iter()
            .map(|partition| partition["entries"][0].clone())
            .collect::<Vec<_>>();
        let mut combined = json!({
            "fromExclusive":partitions.first().unwrap()["fromExclusive"].clone(),
            "throughInclusive":partitions.last().unwrap()["throughInclusive"].clone(),
            "entries":entries
        });
        recalculate_partition_digest(&mut combined);
        combined
    }

    fn validate_versioned_corrective_fixture(
        partition_json: Value,
        evidence: Vec<(String, RepositoryHistorySourceEvidenceRef, Vec<u8>)>,
        corrective_resolver: &dyn SupportCorrectiveEvidenceResolver,
    ) -> Result<super::ValidatedRepositoryHistoryPartition, super::RepositoryContractError> {
        let registry = EvidenceSourceRegistry::task9().unwrap();
        let candidates = evidence
            .iter()
            .map(|(version, source_ref, _)| {
                (
                    version.clone(),
                    support_candidate_for(version, &registry, source_ref.clone(), false),
                )
            })
            .collect();
        let evidence_bytes = FakeEvidenceBytes {
            bytes: evidence
                .into_iter()
                .map(|(_, source_ref, bytes)| {
                    (
                        (
                            EvidenceKind::SupportPrerequisiteObservation,
                            source_ref.evidence_digest().as_str().to_owned(),
                        ),
                        bytes,
                    )
                })
                .collect(),
        };
        let from_exclusive =
            serde_json::from_value(partition_json["fromExclusive"].clone()).unwrap();
        let through_inclusive: RepositoryHistoryCursor =
            serde_json::from_value(partition_json["throughInclusive"].clone()).unwrap();
        let entries = partition_json["entries"].as_array().unwrap();
        let ordered_cursors = entries
            .iter()
            .enumerate()
            .map(|(index, entry)| {
                if index + 1 == entries.len() {
                    through_inclusive.clone()
                } else {
                    serde_json::from_value(cursor(
                        entry["repositoryVersion"].as_str().unwrap(),
                        SHA_A,
                    ))
                    .unwrap()
                }
            })
            .collect();
        let order = FakeOrder {
            evidence: RepositoryHistoryOrderEvidence::from_capability_adapter(
                "versioned-corrective-order-v1",
                from_exclusive,
                through_inclusive,
                ordered_cursors,
            )
            .unwrap(),
        };
        RepositoryHistoryPartitionResolver::new(
            &registry,
            &FakeIndex { candidates },
            &order,
            &evidence_bytes,
        )
        .with_corrective_evidence_resolver(corrective_resolver)
        .validate(serde_json::from_value(partition_json).unwrap())
    }

    fn validate_routine_fixture(
        partition_json: Value,
        candidate: EvidenceSourceIndexCandidate,
        order: RepositoryHistoryOrderEvidence,
        bytes: Vec<u8>,
        source_ref: &RepositoryHistorySourceEvidenceRef,
    ) -> Result<super::ValidatedRepositoryHistoryPartition, super::RepositoryContractError> {
        let registry = EvidenceSourceRegistry::task8().unwrap();
        let index = FakeIndex {
            candidates: BTreeMap::from([("opaque-v1".into(), candidate)]),
        };
        let order = FakeOrder { evidence: order };
        let evidence_bytes = FakeEvidenceBytes {
            bytes: BTreeMap::from([(
                (
                    source_ref.evidence_kind(),
                    source_ref.evidence_digest().as_str().to_owned(),
                ),
                bytes,
            )]),
        };
        let resolver =
            RepositoryHistoryPartitionResolver::new(&registry, &index, &order, &evidence_bytes);
        resolver.validate(
            serde_json::from_value::<UnvalidatedRepositoryHistoryPartition>(partition_json)
                .unwrap(),
        )
    }

    #[test]
    fn capability_resolver_constructs_only_a_fully_validated_routine_partition() {
        let registry = EvidenceSourceRegistry::task8().unwrap();
        let (partition_json, source_ref, evidence) = routine_partition_fixture();
        let unvalidated =
            serde_json::from_value::<UnvalidatedRepositoryHistoryPartition>(partition_json.clone())
                .unwrap();

        let candidate = EvidenceSourceIndexCandidate::from_capability_adapter(
            "opaque-v1",
            registry.registry_digest().as_str(),
            UUID_A,
            vec![
                EvidenceSourceIndexCandidateRow::available(
                    EvidenceKind::RoutineClassification,
                    vec![source_ref.clone()],
                ),
                EvidenceSourceIndexCandidateRow::absent(
                    EvidenceKind::SupportPrerequisiteObservation,
                ),
                EvidenceSourceIndexCandidateRow::absent(EvidenceKind::NonConflictingConcurrent),
            ],
        )
        .unwrap();
        let index = FakeIndex {
            candidates: BTreeMap::from([("opaque-v1".into(), candidate)]),
        };
        let order = FakeOrder {
            evidence: RepositoryHistoryOrderEvidence::from_capability_adapter(
                "history-order-v1",
                serde_json::from_value(cursor("opaque-v0", SHA_A)).unwrap(),
                serde_json::from_value(cursor("opaque-v1", SHA_B)).unwrap(),
                vec![serde_json::from_value(cursor("opaque-v1", SHA_B)).unwrap()],
            )
            .unwrap(),
        };
        let mut evidence_bytes = FakeEvidenceBytes::default();
        evidence_bytes.bytes.insert(
            (
                EvidenceKind::RoutineClassification,
                source_ref.evidence_digest().as_str().to_owned(),
            ),
            serde_json_canonicalizer::to_vec(&evidence).unwrap(),
        );

        let resolver =
            RepositoryHistoryPartitionResolver::new(&registry, &index, &order, &evidence_bytes);
        let validated = resolver.validate(unvalidated).unwrap();
        assert_eq!(serde_json::to_value(&validated).unwrap(), partition_json);
        resolver.late_audit(&validated).unwrap();
        assert_closed::<UnvalidatedRepositoryHistoryPartition>();
    }

    #[test]
    fn relevant_routine_source_maps_to_the_relevant_partition_branch() {
        let registry = EvidenceSourceRegistry::task8().unwrap();
        let (partition, source_ref, evidence) =
            routine_partition_fixture_for("relevant", "relevantRoutine");
        let validated = validate_routine_fixture(
            partition.clone(),
            routine_candidate(&registry, source_ref.clone()),
            routine_order(),
            serde_json_canonicalizer::to_vec(&evidence).unwrap(),
            &source_ref,
        )
        .unwrap();
        assert_eq!(serde_json::to_value(&validated).unwrap(), partition);
        assert_eq!(
            validated.classifications().collect::<Vec<_>>(),
            vec![super::RepositoryHistoryPartitionClassification::RelevantRoutine]
        );
    }

    #[test]
    fn support_observation_maps_all_six_task8_cases_with_highest_precedence() {
        let registry = EvidenceSourceRegistry::task8().unwrap();
        let cases = [
            (
                "routineUnrelated",
                super::RepositoryHistoryPartitionClassification::UnrelatedRoutine,
            ),
            (
                "routineRelevant",
                super::RepositoryHistoryPartitionClassification::RelevantRoutine,
            ),
            (
                "authorized",
                super::RepositoryHistoryPartitionClassification::AuthorizedSupport,
            ),
            (
                "externalSupport",
                super::RepositoryHistoryPartitionClassification::ExternalSupport,
            ),
            (
                "preArmExternal",
                super::RepositoryHistoryPartitionClassification::PreArmExternal,
            ),
            (
                "invalid",
                super::RepositoryHistoryPartitionClassification::Invalid,
            ),
        ];

        for (source_case, expected_classification) in cases {
            let (partition, source_ref, observation) = support_observation_fixture(source_case);
            let validated = validate_support_fixture(
                partition.clone(),
                support_candidate(&registry, source_ref.clone(), true),
                serde_json_canonicalizer::to_vec(&observation).unwrap(),
                &source_ref,
            )
            .unwrap_or_else(|error| panic!("{source_case} rejected: {error}"));
            assert_eq!(serde_json::to_value(&validated).unwrap(), partition);
            assert_eq!(
                validated.classifications().collect::<Vec<_>>(),
                vec![expected_classification]
            );
        }
    }

    #[test]
    fn prerequisite_history_projection_rejects_same_version_foreign_observation() {
        let registry = EvidenceSourceRegistry::task8().unwrap();
        let (partition, source_ref, observation_json) = support_observation_fixture("authorized");
        let validated = validate_support_fixture(
            partition,
            support_candidate(&registry, source_ref.clone(), false),
            serde_json_canonicalizer::to_vec(&observation_json).unwrap(),
            &source_ref,
        )
        .unwrap();
        let exact: super::SupportPrerequisiteVersionObservation =
            serde_json::from_value(observation_json.clone()).unwrap();
        super::ValidatedSupportPrerequisiteHistoryProjection::from_validated_partition(
            validated.clone(),
            vec![exact],
        )
        .unwrap();

        let mut foreign = observation_json;
        foreign["repositoryActor"]["username"] = json!("foreign-user");
        foreign = finalize_support_observation(foreign);
        let foreign: super::SupportPrerequisiteVersionObservation =
            serde_json::from_value(foreign).unwrap();
        assert!(
            super::ValidatedSupportPrerequisiteHistoryProjection::from_validated_partition(
                validated,
                vec![foreign],
            )
            .is_err()
        );
    }

    #[test]
    fn immediate_support_entry_token_requires_the_exact_validated_mapping_and_successor() {
        let registry = EvidenceSourceRegistry::task8().unwrap();
        let (partition, source_ref, observation) = support_observation_fixture("authorized");
        let validated = validate_support_fixture(
            partition,
            support_candidate(&registry, source_ref.clone(), false),
            serde_json_canonicalizer::to_vec(&observation).unwrap(),
            &source_ref,
        )
        .unwrap();
        let successor =
            super::RepositoryHistoryImmediateSuccessorEvidence::from_capability_adapter(
                "history-order-v1",
                serde_json::from_value(cursor("opaque-v0", SHA_A)).unwrap(),
                super::RepositoryVersion::parse("opaque-v1").unwrap(),
            )
            .unwrap();
        let token = super::ValidatedSupportObservationHistoryEntry::from_validated_partition(
            &validated, &successor,
        )
        .unwrap();
        assert_eq!(token.successor(), &successor);
        assert_eq!(token.repository_version().as_str(), "opaque-v1");
        assert_eq!(
            token.partition_classification(),
            super::RepositoryHistoryPartitionClassification::AuthorizedSupport
        );
        assert_eq!(
            token.semantic_delta_digest().as_str(),
            serde_json::to_value(&validated).unwrap()["entries"][0]["semanticDeltaDigest"]
                .as_str()
                .unwrap()
        );
        assert_eq!(
            token.source_evidence_ref().evidence_kind(),
            EvidenceKind::SupportPrerequisiteObservation
        );
        assert_eq!(token.registry_digest(), registry.registry_digest());
        assert_eq!(token.source_index_proof_digest().as_str().len(), 64);
        let index_proof_json =
            serde_json::to_value(validated.source_index_proofs[0].as_ref().unwrap()).unwrap();
        assert!(index_proof_json.get("validatedSupportMapping").is_none());
        let index_proof_schema =
            serde_json::to_value(schema_for!(super::EvidenceSourceIndexProof)).unwrap();
        assert!(index_proof_schema["properties"]
            .get("validatedSupportMapping")
            .is_none());

        for wrong_successor in [
            super::RepositoryHistoryImmediateSuccessorEvidence::from_capability_adapter(
                "history-order-v1",
                serde_json::from_value(cursor("other-v0", SHA_A)).unwrap(),
                super::RepositoryVersion::parse("opaque-v1").unwrap(),
            )
            .unwrap(),
            super::RepositoryHistoryImmediateSuccessorEvidence::from_capability_adapter(
                "history-order-v1",
                serde_json::from_value(cursor("opaque-v0", SHA_A)).unwrap(),
                super::RepositoryVersion::parse("opaque-v2").unwrap(),
            )
            .unwrap(),
        ] {
            assert!(
                super::ValidatedSupportObservationHistoryEntry::from_validated_partition(
                    &validated,
                    &wrong_successor,
                )
                .is_err()
            );
        }

        let mut wrong_registry = validated.clone();
        wrong_registry.source_index_proofs[0]
            .as_mut()
            .unwrap()
            .registry_digest = Sha256Digest::parse(SHA_A).unwrap();
        assert!(
            super::ValidatedSupportObservationHistoryEntry::from_validated_partition(
                &wrong_registry,
                &successor,
            )
            .is_err()
        );

        let mut wrong_selected_ref = validated.clone();
        match &mut wrong_selected_ref.source_index_proofs[0]
            .as_mut()
            .unwrap()
            .availability
            .0[1]
        {
            super::EvidenceSourceAvailability::Available(row) => {
                row.source_evidence_ref = RepositoryHistorySourceEvidenceRef::new(
                    EvidenceKind::SupportPrerequisiteObservation,
                    SHA_A,
                )
                .unwrap();
            }
            super::EvidenceSourceAvailability::Absent(_) => unreachable!(),
        }
        assert!(
            super::ValidatedSupportObservationHistoryEntry::from_validated_partition(
                &wrong_selected_ref,
                &successor,
            )
            .is_err()
        );

        let mut wrong_semantic = validated.clone();
        wrong_semantic.source_index_proofs[0]
            .as_mut()
            .unwrap()
            .validated_support_mapping
            .as_mut()
            .unwrap()
            .semantic_delta_digest = Sha256Digest::parse(SHA_A).unwrap();
        assert!(
            super::ValidatedSupportObservationHistoryEntry::from_validated_partition(
                &wrong_semantic,
                &successor,
            )
            .is_err()
        );

        let (routine_partition, routine_ref, routine_evidence) = routine_partition_fixture();
        let routine_validated = validate_routine_fixture(
            routine_partition,
            routine_candidate(&registry, routine_ref.clone()),
            routine_order(),
            serde_json_canonicalizer::to_vec(&routine_evidence).unwrap(),
            &routine_ref,
        )
        .unwrap();
        assert!(
            super::ValidatedSupportObservationHistoryEntry::from_validated_partition(
                &routine_validated,
                &successor,
            )
            .is_err()
        );
    }

    #[test]
    fn support_observation_mapping_and_loader_substitutions_fail_closed() {
        let registry = EvidenceSourceRegistry::task8().unwrap();
        let (partition, source_ref, observation) = support_observation_fixture("externalSupport");
        let candidate = support_candidate(&registry, source_ref.clone(), true);
        let canonical_bytes = serde_json_canonicalizer::to_vec(&observation).unwrap();

        let mut wrong_classification = partition.clone();
        wrong_classification["entries"][0]["classification"] = json!("relevantRoutine");
        recalculate_partition_digest(&mut wrong_classification);
        assert!(validate_support_fixture(
            wrong_classification,
            candidate.clone(),
            canonical_bytes.clone(),
            &source_ref,
        )
        .is_err());

        let mut wrong_external_projection = partition.clone();
        let wrong_semantic = json!({
            "repositoryVersion":"opaque-v1",
            "partitionClassification":"externalSupport",
            "rootDeltaDigest":SHA_A,
            "contentDeltaDigest":CanonicalEmptyDeltaDigest::VALUE,
            "classificationDigest":observation["classificationDigest"],
            "externalSupportDisjointnessDigest":null,
            "correctiveInstructionDigest":null,
            "nonConflictingConcurrentEvidenceDigest":null
        });
        wrong_external_projection["entries"][0]["semanticDeltaDigest"] =
            json!(test_digest(&wrong_semantic));
        recalculate_partition_digest(&mut wrong_external_projection);
        assert!(validate_support_fixture(
            wrong_external_projection,
            candidate.clone(),
            canonical_bytes.clone(),
            &source_ref,
        )
        .is_err());

        let mut wrong_ref = partition.clone();
        wrong_ref["entries"][0]["sourceEvidenceRef"]["evidenceDigest"] = json!(SHA_A);
        recalculate_partition_digest(&mut wrong_ref);
        assert!(validate_support_fixture(
            wrong_ref,
            candidate.clone(),
            canonical_bytes.clone(),
            &source_ref,
        )
        .is_err());

        assert!(validate_support_fixture(
            partition.clone(),
            candidate.clone(),
            serde_json::to_vec_pretty(&observation).unwrap(),
            &source_ref,
        )
        .is_err());

        let mut wrong_kind_candidate = candidate.clone();
        wrong_kind_candidate.availability[1] = EvidenceSourceIndexCandidateRow::available(
            EvidenceKind::SupportPrerequisiteObservation,
            vec![RepositoryHistorySourceEvidenceRef::new(
                EvidenceKind::RoutineClassification,
                SHA_A,
            )
            .unwrap()],
        );
        assert!(validate_support_fixture(
            partition.clone(),
            wrong_kind_candidate,
            canonical_bytes.clone(),
            &source_ref,
        )
        .is_err());

        let wrong_type = RoutineRepositoryVersionClassificationEvidence::new(
            "opaque-v1",
            "unrelated",
            None,
            SHA_A,
            SHA_B,
        )
        .unwrap();
        assert!(validate_support_fixture(
            partition.clone(),
            candidate,
            serde_json_canonicalizer::to_vec(&wrong_type).unwrap(),
            &source_ref,
        )
        .is_err());

        let mut wrong_version_observation = observation.clone();
        wrong_version_observation["repositoryVersion"] = json!("opaque-v2");
        let wrong_version_observation = finalize_support_observation(wrong_version_observation);
        let wrong_version_ref = RepositoryHistorySourceEvidenceRef::new(
            EvidenceKind::SupportPrerequisiteObservation,
            wrong_version_observation["classificationDigest"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
        let mut wrong_version_partition = partition.clone();
        wrong_version_partition["entries"][0]["sourceEvidenceRef"] =
            serde_json::to_value(&wrong_version_ref).unwrap();
        recalculate_partition_digest(&mut wrong_version_partition);
        assert!(validate_support_fixture(
            wrong_version_partition,
            support_candidate(&registry, wrong_version_ref.clone(), false),
            serde_json_canonicalizer::to_vec(&wrong_version_observation).unwrap(),
            &wrong_version_ref,
        )
        .is_err());

        let substituted_ref = RepositoryHistorySourceEvidenceRef::new(
            EvidenceKind::SupportPrerequisiteObservation,
            SHA_A,
        )
        .unwrap();
        let mut substituted_partition = partition;
        substituted_partition["entries"][0]["sourceEvidenceRef"] =
            serde_json::to_value(&substituted_ref).unwrap();
        recalculate_partition_digest(&mut substituted_partition);
        assert!(validate_support_fixture(
            substituted_partition,
            support_candidate(&registry, substituted_ref.clone(), false),
            canonical_bytes,
            &substituted_ref,
        )
        .is_err());
    }

    #[test]
    fn routine_projection_resolves_current_deferred_against_the_same_partition() {
        let registry = EvidenceSourceRegistry::task8().unwrap();
        let (partition_json, source_ref, observation) = support_observation_fixture("authorized");
        let partition = validate_support_fixture(
            partition_json,
            support_candidate(&registry, source_ref.clone(), false),
            serde_json_canonicalizer::to_vec(&observation).unwrap(),
            &source_ref,
        )
        .unwrap();
        let anchor = RepositoryAnchorObservationAuthority::test_only(
            Sha256Digest::parse(SHA_A).unwrap(),
            partition.start_cursor().clone(),
            configuration_identity("8.3.27"),
            Sha256Digest::parse(SHA_A).unwrap(),
        )
        .into_anchor()
        .unwrap();
        let before_states: RepositoryTargetStates = serde_json::from_value(json!([{
            "targetKind": "configurationRoot",
            "state": "present",
            "repositoryVersion": "opaque-v0",
            "targetFingerprint": SHA_A,
        }]))
        .unwrap();
        let changes: RepositoryPlannedChanges = serde_json::from_value(json!([{
            "targetKind": "configurationRoot",
            "action": "modify",
            "objectDisplay": "Configuration",
            "repositoryVersion": "opaque-v1",
            "targetFingerprint": SHA_B,
            "relevance": "relevant",
        }]))
        .unwrap();
        let first = partition.first_entry().unwrap();
        let fold = RoutineUpdateHistoryFoldCapabilityToken::from_capability_adapter(
            &anchor,
            &partition,
            before_states,
            vec![
                RoutineRepositoryVersionChangeObservation::from_capability_adapter(
                    first.repository_version().clone(),
                    first.semantic_delta_digest().clone(),
                    changes,
                ),
            ],
            CapabilityRowId::parse("routine.history-target-projection.v1").unwrap(),
        )
        .unwrap();
        let planned_targets: RepositoryTargetStates = serde_json::from_value(json!([{
            "targetKind": "configurationRoot",
            "state": "present",
            "repositoryVersion": "opaque-v1",
            "targetFingerprint": SHA_B,
        }]))
        .unwrap();
        let locks: RepositoryUpdateLockTargets = serde_json::from_value(json!([{
            "targetKind": "configurationRoot",
            "objectDisplay": "Configuration",
            "reasons": ["updateTarget"],
        }]))
        .unwrap();
        let plan = super::update::RoutineSelectiveRepositoryUpdatePlanAuthority::from_capability_token(
            super::update::RoutineSelectiveRepositoryUpdateCapabilityToken::from_capability_adapter(
                planned_targets,
                Vec::new(),
                locks,
                CapabilityRowId::parse("selective.objects.v1").unwrap(),
                None,
                Vec::new(),
            )
            .unwrap(),
        )
        .unwrap();
        let resolver = RoutineProjectionResolverFixture {
            fold: Some(fold),
            plan: Some(plan),
            phase: Some(
                super::lifecycle::RoutineUpdatePhaseAuthority::routine_test_only(
                    TaskPhase::Synchronized,
                )
                .unwrap(),
            ),
        };
        let deferred = super::DeferredRepositoryAdvance::coverage_unknown_test_only(
            partition.start_cursor().clone(),
        )
        .unwrap();
        let projection = ValidatedRoutineUpdateProjection::resolve(
            anchor,
            partition,
            Some(super::CurrentDeferredRepositoryAdvanceAuthority::test_only(
                UnicaId::parse("123e4567-e89b-12d3-a456-426614174000").unwrap(),
                deferred.clone(),
            )),
            resolver,
        )
        .unwrap();
        assert_eq!(
            projection
                .deferred_repository_advance()
                .unwrap()
                .observation_digest(),
            deferred.observation_digest()
        );
        assert!(projection.deferred_advance_resolution_digest().is_some());
    }

    #[test]
    fn action_correction_requires_exact_historical_instruction_and_attribution_authority() {
        let instruction = corrective_instruction();
        let instruction_bytes = serde_json_canonicalizer::to_vec(&instruction).unwrap();
        let conflict_bytes = serde_json_canonicalizer::to_vec(&conflict_instruction()).unwrap();
        let (partition, source_ref, observation) =
            action_corrective_partition_fixture(&instruction);
        let observation_bytes = serde_json_canonicalizer::to_vec(&observation).unwrap();
        let resolver = fake_corrective_resolver(
            Some(instruction_bytes.clone()),
            Some(conflict_bytes.clone()),
        );

        assert!(validate_corrective_fixture(
            partition.clone(),
            &source_ref,
            observation_bytes.clone(),
            Some(&resolver),
        )
        .is_ok());

        // The observation must not be able to select another internally
        // consistent historical instruction merely by supplying its digest.
        let foreign_instruction = corrective_instruction_for(UUID_B);
        let (foreign_partition, foreign_ref, foreign_observation) =
            action_corrective_partition_fixture(&foreign_instruction);
        let foreign_resolver = fake_corrective_resolver(
            Some(serde_json_canonicalizer::to_vec(&foreign_instruction).unwrap()),
            None,
        );
        assert!(validate_corrective_fixture(
            foreign_partition,
            &foreign_ref,
            serde_json_canonicalizer::to_vec(&foreign_observation).unwrap(),
            Some(&foreign_resolver),
        )
        .is_err());

        assert!(validate_corrective_fixture(
            partition.clone(),
            &source_ref,
            observation_bytes.clone(),
            None,
        )
        .is_err());

        let missing = fake_corrective_resolver(None, Some(conflict_bytes.clone()));
        assert!(validate_corrective_fixture(
            partition.clone(),
            &source_ref,
            observation_bytes.clone(),
            Some(&missing),
        )
        .is_err());

        let cross_kind = fake_corrective_resolver(Some(conflict_bytes), None);
        assert!(validate_corrective_fixture(
            partition.clone(),
            &source_ref,
            observation_bytes.clone(),
            Some(&cross_kind),
        )
        .is_err());

        let mut noncanonical = serde_json::to_vec_pretty(&instruction).unwrap();
        noncanonical.push(b'\n');
        let noncanonical = fake_corrective_resolver(Some(noncanonical), None);
        assert!(validate_corrective_fixture(
            partition.clone(),
            &source_ref,
            observation_bytes.clone(),
            Some(&noncanonical),
        )
        .is_err());

        let mut rejected_actor = resolver.clone();
        rejected_actor.action_attribution_valid = false;
        assert!(validate_corrective_fixture(
            partition.clone(),
            &source_ref,
            observation_bytes.clone(),
            Some(&rejected_actor),
        )
        .is_err());

        let mut wrong_historical_action = resolver.clone();
        wrong_historical_action
            .corrective_sources
            .get_mut("opaque-v1")
            .unwrap()
            .expected_historical_support_action_id = test_id(UUID_B);
        assert!(validate_corrective_fixture(
            partition.clone(),
            &source_ref,
            observation_bytes.clone(),
            Some(&wrong_historical_action),
        )
        .is_err());

        for (field, replacement, semantic_instruction_digest) in [
            (
                "rootDeltaDigest",
                json!(SHA_A),
                instruction.corrective_instruction_digest().as_str(),
            ),
            (
                "contentDeltaDigest",
                json!(SHA_B),
                instruction.corrective_instruction_digest().as_str(),
            ),
            ("correctiveInstructionDigest", json!(SHA_A), SHA_A),
            (
                "repositoryVersion",
                json!("opaque-v2"),
                instruction.corrective_instruction_digest().as_str(),
            ),
        ] {
            let mut substituted = observation.clone();
            substituted[field] = replacement;
            let (substituted_partition, substituted_ref, substituted_observation) =
                corrective_partition_from_observation(substituted, semantic_instruction_digest);
            assert!(validate_corrective_fixture(
                substituted_partition,
                &substituted_ref,
                serde_json_canonicalizer::to_vec(&substituted_observation).unwrap(),
                Some(&resolver),
            )
            .is_err());
        }

        let mut wrong_actor = observation.clone();
        wrong_actor["repositoryActor"]["username"] = json!("another-user");
        let (wrong_actor_partition, wrong_actor_ref, wrong_actor_observation) =
            corrective_partition_from_observation(
                wrong_actor,
                instruction.corrective_instruction_digest().as_str(),
            );
        assert!(validate_corrective_fixture(
            wrong_actor_partition,
            &wrong_actor_ref,
            serde_json_canonicalizer::to_vec(&wrong_actor_observation).unwrap(),
            Some(&resolver),
        )
        .is_err());

        let mut wrong_semantic = partition;
        wrong_semantic["entries"][0]["semanticDeltaDigest"] = json!(SHA_A);
        recalculate_partition_digest(&mut wrong_semantic);
        assert!(validate_corrective_fixture(
            wrong_semantic,
            &source_ref,
            observation_bytes,
            Some(&resolver),
        )
        .is_err());
    }

    #[test]
    fn action_correction_resolves_each_historical_version_to_its_exact_frozen_instruction() {
        let first_instruction = corrective_instruction_for(UUID_A);
        let second_instruction = corrective_instruction_for_deltas(UUID_A, SHA_B, SHA_A);
        assert_eq!(
            first_instruction.support_action_id(),
            second_instruction.support_action_id()
        );
        assert_ne!(
            first_instruction.corrective_instruction_digest(),
            second_instruction.corrective_instruction_digest()
        );
        let (first_partition, first_ref, first_observation) =
            action_corrective_partition_fixture_for("opaque-v1", &first_instruction);
        let (second_partition, second_ref, second_observation) =
            action_corrective_partition_fixture_for("opaque-v2", &second_instruction);
        let partition = combine_corrective_partitions(&[first_partition, second_partition]);
        let evidence = vec![
            (
                "opaque-v1".to_owned(),
                first_ref,
                serde_json_canonicalizer::to_vec(&first_observation).unwrap(),
            ),
            (
                "opaque-v2".to_owned(),
                second_ref,
                serde_json_canonicalizer::to_vec(&second_observation).unwrap(),
            ),
        ];
        let mut exact = fake_corrective_resolver(None, None);
        for (version, instruction) in [
            ("opaque-v1", &first_instruction),
            ("opaque-v2", &second_instruction),
        ] {
            let repository_version = RepositoryVersion::parse(version).unwrap();
            exact.corrective_sources.insert(
                version.to_owned(),
                FrozenSupportCorrectiveInstructionSourceAuthority::from_capability_adapter(
                    repository_version,
                    instruction.support_action_id().clone(),
                    instruction.corrective_instruction_digest().clone(),
                    serde_json_canonicalizer::to_vec(instruction).unwrap(),
                ),
            );
        }

        assert!(
            validate_versioned_corrective_fixture(partition.clone(), evidence.clone(), &exact,)
                .is_ok()
        );

        let mut swapped = exact;
        let first_source = swapped.corrective_sources.remove("opaque-v1").unwrap();
        let second_source = swapped.corrective_sources.remove("opaque-v2").unwrap();
        swapped
            .corrective_sources
            .insert("opaque-v1".to_owned(), second_source);
        swapped
            .corrective_sources
            .insert("opaque-v2".to_owned(), first_source);
        assert!(validate_versioned_corrective_fixture(partition, evidence, &swapped).is_err());

        // Even a fully self-consistent per-version source/observation pair
        // cannot splice another frozen action into this action-bound history.
        let foreign_instruction = corrective_instruction_for_deltas(UUID_B, SHA_B, SHA_A);
        let (first_partition, first_ref, first_observation) =
            action_corrective_partition_fixture_for("opaque-v1", &first_instruction);
        let (foreign_partition, foreign_ref, foreign_observation) =
            action_corrective_partition_fixture_for("opaque-v2", &foreign_instruction);
        let mixed_partition = combine_corrective_partitions(&[first_partition, foreign_partition]);
        let mixed_evidence = vec![
            (
                "opaque-v1".to_owned(),
                first_ref,
                serde_json_canonicalizer::to_vec(&first_observation).unwrap(),
            ),
            (
                "opaque-v2".to_owned(),
                foreign_ref,
                serde_json_canonicalizer::to_vec(&foreign_observation).unwrap(),
            ),
        ];
        let mut mixed = fake_corrective_resolver(None, None);
        for (version, instruction) in [
            ("opaque-v1", &first_instruction),
            ("opaque-v2", &foreign_instruction),
        ] {
            let repository_version = RepositoryVersion::parse(version).unwrap();
            mixed.corrective_sources.insert(
                version.to_owned(),
                FrozenSupportCorrectiveInstructionSourceAuthority::from_capability_adapter(
                    repository_version,
                    instruction.support_action_id().clone(),
                    instruction.corrective_instruction_digest().clone(),
                    serde_json_canonicalizer::to_vec(instruction).unwrap(),
                ),
            );
        }
        assert!(
            validate_versioned_corrective_fixture(mixed_partition, mixed_evidence, &mixed,)
                .is_err()
        );
    }

    #[test]
    fn external_conflict_correction_binds_conflict_baseline_and_immutable_ownership() {
        let instruction = conflict_instruction();
        let instruction_bytes = serde_json_canonicalizer::to_vec(&instruction).unwrap();
        let corrective_bytes = serde_json_canonicalizer::to_vec(&corrective_instruction()).unwrap();
        let (partition, source_ref, observation) =
            external_corrective_partition_fixture(&instruction);
        let observation_bytes = serde_json_canonicalizer::to_vec(&observation).unwrap();
        let resolver = fake_corrective_resolver(
            Some(corrective_bytes.clone()),
            Some(instruction_bytes.clone()),
        );

        assert!(validate_corrective_fixture(
            partition.clone(),
            &source_ref,
            observation_bytes.clone(),
            Some(&resolver),
        )
        .is_ok());

        // A fully self-consistent foreign conflict pair is still not the
        // capability-selected frozen historical conflict instruction.
        let foreign_instruction = conflict_instruction_for(UUID_B);
        let (foreign_partition, foreign_ref, foreign_observation) =
            external_corrective_partition_fixture(&foreign_instruction);
        let foreign_resolver = fake_corrective_resolver(
            None,
            Some(serde_json_canonicalizer::to_vec(&foreign_instruction).unwrap()),
        );
        assert!(validate_corrective_fixture(
            foreign_partition,
            &foreign_ref,
            serde_json_canonicalizer::to_vec(&foreign_observation).unwrap(),
            Some(&foreign_resolver),
        )
        .is_err());

        let mut rejected_ownership = resolver.clone();
        rejected_ownership.external_attribution_valid = false;
        assert!(validate_corrective_fixture(
            partition.clone(),
            &source_ref,
            observation_bytes.clone(),
            Some(&rejected_ownership),
        )
        .is_err());

        let mut wrong_historical_conflict = resolver.clone();
        wrong_historical_conflict
            .conflict_sources
            .get_mut("opaque-v1")
            .unwrap()
            .expected_historical_conflict_resolution_id = test_id(UUID_B);
        assert!(validate_corrective_fixture(
            partition.clone(),
            &source_ref,
            observation_bytes.clone(),
            Some(&wrong_historical_conflict),
        )
        .is_err());

        let mut wrong_action_binding = resolver.clone();
        wrong_action_binding
            .conflict_sources
            .get_mut("opaque-v1")
            .unwrap()
            .expected_historical_support_action_id = test_id(UUID_B);
        assert!(validate_corrective_fixture(
            partition.clone(),
            &source_ref,
            observation_bytes.clone(),
            Some(&wrong_action_binding),
        )
        .is_err());

        let missing = fake_corrective_resolver(Some(corrective_bytes.clone()), None);
        assert!(validate_corrective_fixture(
            partition.clone(),
            &source_ref,
            observation_bytes.clone(),
            Some(&missing),
        )
        .is_err());

        let cross_kind = fake_corrective_resolver(None, Some(corrective_bytes.clone()));
        assert!(validate_corrective_fixture(
            partition.clone(),
            &source_ref,
            observation_bytes.clone(),
            Some(&cross_kind),
        )
        .is_err());

        let mut noncanonical = serde_json::to_vec_pretty(&instruction).unwrap();
        noncanonical.push(b'\n');
        let noncanonical = fake_corrective_resolver(Some(corrective_bytes), Some(noncanonical));
        assert!(validate_corrective_fixture(
            partition.clone(),
            &source_ref,
            observation_bytes.clone(),
            Some(&noncanonical),
        )
        .is_err());

        for (field, replacement) in [
            (
                "conflictResolutionId",
                json!("22222222-2222-4222-8222-222222222222"),
            ),
            ("finalBaselineDigest", json!(SHA_A)),
        ] {
            let mut substituted = observation.clone();
            substituted[field] = replacement;
            let (substituted_partition, substituted_ref, substituted_observation) =
                corrective_partition_from_observation(
                    substituted,
                    instruction.support_conflict_instruction_digest().as_str(),
                );
            assert!(validate_corrective_fixture(
                substituted_partition,
                &substituted_ref,
                serde_json_canonicalizer::to_vec(&substituted_observation).unwrap(),
                Some(&resolver),
            )
            .is_err());
        }

        let mut wrong_instruction_digest = observation.clone();
        wrong_instruction_digest["supportConflictInstructionDigest"] = json!(SHA_A);
        let (wrong_partition, wrong_ref, wrong_observation) =
            corrective_partition_from_observation(wrong_instruction_digest, SHA_A);
        assert!(validate_corrective_fixture(
            wrong_partition,
            &wrong_ref,
            serde_json_canonicalizer::to_vec(&wrong_observation).unwrap(),
            Some(&resolver),
        )
        .is_err());

        let mut wrong_ownership = observation;
        wrong_ownership["externalOwnershipEvidence"]["receiptDigest"] = json!(SHA_B);
        let (wrong_partition, wrong_ref, wrong_observation) = corrective_partition_from_observation(
            wrong_ownership,
            instruction.support_conflict_instruction_digest().as_str(),
        );
        assert!(validate_corrective_fixture(
            wrong_partition,
            &wrong_ref,
            serde_json_canonicalizer::to_vec(&wrong_observation).unwrap(),
            Some(&resolver),
        )
        .is_err());
    }

    #[test]
    fn external_conflict_correction_resolves_each_historical_version_to_its_exact_frozen_instruction(
    ) {
        let first_instruction = conflict_instruction_for(UUID_A);
        let second_instruction = conflict_instruction_for(UUID_B);
        assert_ne!(
            first_instruction.support_conflict_instruction_digest(),
            second_instruction.support_conflict_instruction_digest()
        );
        let (first_partition, first_ref, first_observation) =
            external_corrective_partition_fixture_for("opaque-v1", &first_instruction);
        let (second_partition, second_ref, second_observation) =
            external_corrective_partition_fixture_for("opaque-v2", &second_instruction);
        let partition = combine_corrective_partitions(&[first_partition, second_partition]);
        let evidence = vec![
            (
                "opaque-v1".to_owned(),
                first_ref,
                serde_json_canonicalizer::to_vec(&first_observation).unwrap(),
            ),
            (
                "opaque-v2".to_owned(),
                second_ref,
                serde_json_canonicalizer::to_vec(&second_observation).unwrap(),
            ),
        ];
        let mut exact = fake_corrective_resolver(None, None);
        for (version, instruction) in [
            ("opaque-v1", &first_instruction),
            ("opaque-v2", &second_instruction),
        ] {
            let repository_version = RepositoryVersion::parse(version).unwrap();
            exact.conflict_sources.insert(
                version.to_owned(),
                FrozenSupportConflictInstructionSourceAuthority::from_capability_adapter(
                    repository_version,
                    test_id(UUID_A),
                    instruction.conflict_resolution_id().clone(),
                    instruction.support_conflict_instruction_digest().clone(),
                    serde_json_canonicalizer::to_vec(instruction).unwrap(),
                ),
            );
        }

        assert!(
            validate_versioned_corrective_fixture(partition.clone(), evidence.clone(), &exact,)
                .is_ok()
        );

        let mut swapped = exact;
        let first_source = swapped.conflict_sources.remove("opaque-v1").unwrap();
        let second_source = swapped.conflict_sources.remove("opaque-v2").unwrap();
        swapped
            .conflict_sources
            .insert("opaque-v1".to_owned(), second_source);
        swapped
            .conflict_sources
            .insert("opaque-v2".to_owned(), first_source);
        assert!(validate_versioned_corrective_fixture(partition, evidence, &swapped).is_err());
    }

    #[test]
    fn available_support_source_never_falls_back_to_lower_precedence_rows() {
        let registry = EvidenceSourceRegistry::task8().unwrap();
        let (routine_partition, routine_ref, _routine_evidence) = routine_partition_fixture();
        let (_support_partition, support_ref, support_observation) =
            support_observation_fixture("externalSupport");
        assert!(validate_support_fixture(
            routine_partition,
            support_candidate(&registry, support_ref.clone(), true),
            serde_json_canonicalizer::to_vec(&support_observation).unwrap(),
            &support_ref,
        )
        .is_err());

        let (concurrent_partition, concurrent_ref, _) = non_conflicting_partition_fixture();
        let concurrent_candidate = EvidenceSourceIndexCandidate::from_capability_adapter(
            "opaque-v1",
            registry.registry_digest().as_str(),
            UUID_A,
            vec![
                EvidenceSourceIndexCandidateRow::absent(EvidenceKind::RoutineClassification),
                EvidenceSourceIndexCandidateRow::available(
                    EvidenceKind::SupportPrerequisiteObservation,
                    vec![support_ref],
                ),
                EvidenceSourceIndexCandidateRow::available(
                    EvidenceKind::NonConflictingConcurrent,
                    vec![concurrent_ref],
                ),
            ],
        )
        .unwrap();
        let index = FakeIndex {
            candidates: BTreeMap::from([("opaque-v1".into(), concurrent_candidate)]),
        };
        let order = FakeOrder {
            evidence: routine_order(),
        };
        assert!(RepositoryHistoryPartitionResolver::new(
            &registry,
            &index,
            &order,
            &FakeEvidenceBytes::default(),
        )
        .validate(serde_json::from_value(concurrent_partition).unwrap())
        .is_err());

        let mut stale_candidate = routine_candidate(&registry, routine_ref.clone());
        stale_candidate.registry_digest =
            Sha256Digest::parse(super::TASK8_EVIDENCE_SOURCE_REGISTRY_DIGEST).unwrap();
        let (_, _, routine_evidence) = routine_partition_fixture();
        assert!(validate_routine_fixture(
            routine_partition_fixture().0,
            stale_candidate,
            routine_order(),
            serde_json_canonicalizer::to_vec(&routine_evidence).unwrap(),
            &routine_ref,
        )
        .is_err());
    }

    #[test]
    fn endpoint_emptiness_and_order_evidence_are_fail_closed() {
        let registry = EvidenceSourceRegistry::task8().unwrap();
        let dummy_index = FakeIndex {
            candidates: BTreeMap::new(),
        };
        let dummy_order = FakeOrder {
            evidence: routine_order(),
        };
        let dummy_bytes = FakeEvidenceBytes::default();
        let resolver = RepositoryHistoryPartitionResolver::new(
            &registry,
            &dummy_index,
            &dummy_order,
            &dummy_bytes,
        );

        let endpoint = cursor("opaque-v0", SHA_A);
        let mut equal_empty = json!({
            "fromExclusive":endpoint,
            "throughInclusive":endpoint,
            "entries":[]
        });
        recalculate_partition_digest(&mut equal_empty);
        let validated = resolver
            .validate(serde_json::from_value(equal_empty.clone()).unwrap())
            .unwrap();
        assert_eq!(serde_json::to_value(validated).unwrap(), equal_empty);

        let mut differing_empty = json!({
            "fromExclusive":cursor("opaque-v0", SHA_A),
            "throughInclusive":cursor("opaque-v1", SHA_B),
            "entries":[]
        });
        recalculate_partition_digest(&mut differing_empty);
        assert!(resolver
            .validate(serde_json::from_value(differing_empty).unwrap())
            .is_err());

        let (mut equal_nonempty, source_ref, evidence) = routine_partition_fixture();
        equal_nonempty["throughInclusive"] = equal_nonempty["fromExclusive"].clone();
        recalculate_partition_digest(&mut equal_nonempty);
        let candidate = routine_candidate(&registry, source_ref.clone());
        assert!(validate_routine_fixture(
            equal_nonempty,
            candidate,
            routine_order(),
            serde_json_canonicalizer::to_vec(&evidence).unwrap(),
            &source_ref,
        )
        .is_err());

        let (partition, source_ref, evidence) = routine_partition_fixture();
        let candidate = routine_candidate(&registry, source_ref.clone());
        let mut wrong_order = routine_order();
        wrong_order.ordered_versions = vec![super::RepositoryVersion::parse("opaque-v2").unwrap()];
        assert!(validate_routine_fixture(
            partition,
            candidate,
            wrong_order,
            serde_json_canonicalizer::to_vec(&evidence).unwrap(),
            &source_ref,
        )
        .is_err());
    }

    #[test]
    fn history_partition_schema_is_a_structural_superset_of_runtime_relations() {
        let schema =
            serde_json::to_value(schema_for!(UnvalidatedRepositoryHistoryPartition)).unwrap();
        let validator = jsonschema::validator_for(&schema).unwrap();
        let registry = EvidenceSourceRegistry::task8().unwrap();
        let dummy_index = FakeIndex {
            candidates: BTreeMap::new(),
        };
        let dummy_order = FakeOrder {
            evidence: routine_order(),
        };
        let dummy_bytes = FakeEvidenceBytes::default();
        let resolver = RepositoryHistoryPartitionResolver::new(
            &registry,
            &dummy_index,
            &dummy_order,
            &dummy_bytes,
        );

        let mut differing_empty = json!({
            "fromExclusive":cursor("opaque-v0", SHA_A),
            "throughInclusive":cursor("opaque-v1", SHA_B),
            "entries":[]
        });
        recalculate_partition_digest(&mut differing_empty);
        assert!(
            validator.is_valid(&differing_empty),
            "schema intentionally cannot compare endpoint equality with entry emptiness"
        );
        assert!(resolver
            .validate(serde_json::from_value(differing_empty.clone()).unwrap())
            .is_err());

        differing_empty["partitionDigest"] = json!(SHA_A);
        assert!(
            validator.is_valid(&differing_empty),
            "schema intentionally cannot recompute the partition digest"
        );
        assert!(resolver
            .validate(serde_json::from_value(differing_empty).unwrap())
            .is_err());
    }

    #[test]
    fn structural_superset_matrix_defers_cross_item_and_capability_relations_to_runtime() {
        let registry = EvidenceSourceRegistry::task8().unwrap();
        let (partition, source_ref, evidence) = routine_partition_fixture();
        let partition_validator = jsonschema::validator_for(
            &serde_json::to_value(schema_for!(UnvalidatedRepositoryHistoryPartition)).unwrap(),
        )
        .unwrap();
        let bytes = serde_json_canonicalizer::to_vec(&evidence).unwrap();
        assert!(partition_validator.is_valid(&partition));

        // Adapter order is opaque RepositoryVersion evidence, not a JSON relation.
        let candidate = routine_candidate(&registry, source_ref.clone());
        let mut wrong_order = routine_order();
        wrong_order.ordered_versions = vec![super::RepositoryVersion::parse("opaque-v2").unwrap()];
        assert!(validate_routine_fixture(
            partition.clone(),
            candidate,
            wrong_order,
            bytes.clone(),
            &source_ref,
        )
        .is_err());

        // Registry/index alignment and source precedence are capability-backed.
        let mut wrong_registry = routine_candidate(&registry, source_ref.clone());
        wrong_registry.registry_digest = Sha256Digest::parse(SHA_A).unwrap();
        assert!(validate_routine_fixture(
            partition.clone(),
            wrong_registry,
            routine_order(),
            bytes.clone(),
            &source_ref,
        )
        .is_err());
        let higher_ref =
            RepositoryHistorySourceEvidenceRef::new(EvidenceKind::NonConflictingConcurrent, SHA_A)
                .unwrap();
        let higher_available = EvidenceSourceIndexCandidate::from_capability_adapter(
            "opaque-v1",
            registry.registry_digest().as_str(),
            UUID_A,
            vec![
                EvidenceSourceIndexCandidateRow::available(
                    EvidenceKind::RoutineClassification,
                    vec![source_ref.clone()],
                ),
                EvidenceSourceIndexCandidateRow::absent(
                    EvidenceKind::SupportPrerequisiteObservation,
                ),
                EvidenceSourceIndexCandidateRow::available(
                    EvidenceKind::NonConflictingConcurrent,
                    vec![higher_ref],
                ),
            ],
        )
        .unwrap();
        assert!(validate_routine_fixture(
            partition.clone(),
            higher_available,
            routine_order(),
            bytes.clone(),
            &source_ref,
        )
        .is_err());

        // Nested ref/classification agreement and semantic digest projections
        // are valid physical JSON but are recomputed by the typed resolver.
        let mut wrong_nested_ref = partition.clone();
        wrong_nested_ref["entries"][0]["sourceEvidenceRef"]["evidenceKind"] =
            json!("nonConflictingConcurrent");
        recalculate_partition_digest(&mut wrong_nested_ref);
        assert!(partition_validator.is_valid(&wrong_nested_ref));
        assert!(validate_routine_fixture(
            wrong_nested_ref,
            routine_candidate(&registry, source_ref.clone()),
            routine_order(),
            bytes.clone(),
            &source_ref,
        )
        .is_err());

        let mut wrong_semantic_projection = partition;
        wrong_semantic_projection["entries"][0]["semanticDeltaDigest"] = json!(SHA_A);
        recalculate_partition_digest(&mut wrong_semantic_projection);
        assert!(partition_validator.is_valid(&wrong_semantic_projection));
        assert!(validate_routine_fixture(
            wrong_semantic_projection,
            routine_candidate(&registry, source_ref.clone()),
            routine_order(),
            bytes,
            &source_ref,
        )
        .is_err());

        // Array item schemas cannot express canonical identity/reason order or
        // uniqueness; the validated collection deserializers do.
        let target_validator = jsonschema::validator_for(
            &serde_json::to_value(schema_for!(RepositoryTargetStates)).unwrap(),
        )
        .unwrap();
        for invalid in [
            json!([object_state(OBJECT_B), object_state(OBJECT_A)]),
            json!([object_state(OBJECT_A), object_state(OBJECT_A)]),
        ] {
            assert!(target_validator.is_valid(&invalid));
            assert!(serde_json::from_value::<RepositoryTargetStates>(invalid).is_err());
        }

        let object_change = |object_id: &str| {
            json!({
                "targetKind":"developmentObject",
                "objectId":object_id,
                "objectDisplay":"Catalog",
                "action":"modify",
                "repositoryVersion":"object-v1",
                "targetFingerprint":SHA_B,
                "relevance":"unrelated"
            })
        };
        let planned_validator = jsonschema::validator_for(
            &serde_json::to_value(schema_for!(RepositoryPlannedChanges)).unwrap(),
        )
        .unwrap();
        for invalid in [
            json!([object_change(OBJECT_B), object_change(OBJECT_A)]),
            json!([object_change(OBJECT_A), object_change(OBJECT_A)]),
        ] {
            assert!(planned_validator.is_valid(&invalid));
            assert!(serde_json::from_value::<RepositoryPlannedChanges>(invalid).is_err());
        }

        let reason_validator = jsonschema::validator_for(
            &serde_json::to_value(schema_for!(RepositoryUpdateLockReasons)).unwrap(),
        )
        .unwrap();
        for invalid in [
            json!(["updateTarget", "supportGraphGuard"]),
            json!(["updateTarget", "updateTarget"]),
        ] {
            assert!(reason_validator.is_valid(&invalid));
            assert!(serde_json::from_value::<RepositoryUpdateLockReasons>(invalid).is_err());
        }

        let lock_validator = jsonschema::validator_for(
            &serde_json::to_value(schema_for!(RepositoryUpdateLockTargets)).unwrap(),
        )
        .unwrap();
        let wrong_lock_order = json!([object_lock(OBJECT_A), root_lock()]);
        assert!(lock_validator.is_valid(&wrong_lock_order));
        assert!(serde_json::from_value::<RepositoryUpdateLockTargets>(wrong_lock_order).is_err());
        let release_validator = jsonschema::validator_for(
            &serde_json::to_value(schema_for!(ReleasedRepositoryUpdateLockTargets)).unwrap(),
        )
        .unwrap();
        let wrong_release_order = json!([root_lock(), object_lock(OBJECT_A)]);
        assert!(release_validator.is_valid(&wrong_release_order));
        assert!(
            serde_json::from_value::<ReleasedRepositoryUpdateLockTargets>(wrong_release_order)
                .is_err()
        );
    }

    #[test]
    fn history_order_rejects_non_adjacent_duplicates_and_wrong_terminal_version_before_index() {
        let registry = EvidenceSourceRegistry::task8().unwrap();
        let (baseline, _, _) = routine_partition_fixture();
        let entry = baseline["entries"][0].clone();
        let dummy_bytes = FakeEvidenceBytes::default();

        let mut non_adjacent_duplicate = baseline.clone();
        let mut middle = entry.clone();
        middle["repositoryVersion"] = json!("opaque-v2");
        non_adjacent_duplicate["entries"] = json!([entry, middle, entry]);
        non_adjacent_duplicate["throughInclusive"] = cursor("opaque-v1", SHA_B);
        recalculate_partition_digest(&mut non_adjacent_duplicate);
        let duplicate_order = FakeOrder {
            evidence: RepositoryHistoryOrderEvidence::from_capability_adapter(
                "history-order-v1",
                serde_json::from_value(cursor("opaque-v0", SHA_A)).unwrap(),
                serde_json::from_value(cursor("opaque-v1", SHA_B)).unwrap(),
                ["opaque-v1", "opaque-v2", "opaque-v1"]
                    .into_iter()
                    .map(|version| serde_json::from_value(cursor(version, SHA_B)).unwrap())
                    .collect(),
            )
            .unwrap(),
        };
        let resolver = RepositoryHistoryPartitionResolver::new(
            &registry,
            &UnexpectedIndex,
            &duplicate_order,
            &dummy_bytes,
        );
        assert!(resolver
            .validate(serde_json::from_value(non_adjacent_duplicate).unwrap())
            .is_err());

        let mut wrong_terminal = baseline;
        let mut second = wrong_terminal["entries"][0].clone();
        second["repositoryVersion"] = json!("opaque-v2");
        wrong_terminal["entries"] = json!([wrong_terminal["entries"][0], second]);
        wrong_terminal["throughInclusive"] = cursor("opaque-v3", SHA_B);
        recalculate_partition_digest(&mut wrong_terminal);
        let terminal_order = FakeOrder {
            evidence: RepositoryHistoryOrderEvidence::from_capability_adapter(
                "history-order-v1",
                serde_json::from_value(cursor("opaque-v0", SHA_A)).unwrap(),
                serde_json::from_value(cursor("opaque-v3", SHA_B)).unwrap(),
                ["opaque-v1", "opaque-v2"]
                    .into_iter()
                    .map(|version| serde_json::from_value(cursor(version, SHA_B)).unwrap())
                    .collect(),
            )
            .unwrap(),
        };
        let resolver = RepositoryHistoryPartitionResolver::new(
            &registry,
            &UnexpectedIndex,
            &terminal_order,
            &dummy_bytes,
        );
        assert!(resolver
            .validate(serde_json::from_value(wrong_terminal).unwrap())
            .is_err());

        let (wrong_cursor_digest, _, _) = routine_partition_fixture();
        let cursor_digest_order = FakeOrder {
            evidence: RepositoryHistoryOrderEvidence::from_capability_adapter(
                "history-order-v1",
                serde_json::from_value(cursor("opaque-v0", SHA_A)).unwrap(),
                serde_json::from_value(cursor("opaque-v1", SHA_B)).unwrap(),
                vec![serde_json::from_value(cursor("opaque-v1", SHA_A)).unwrap()],
            )
            .unwrap(),
        };
        let resolver = RepositoryHistoryPartitionResolver::new(
            &registry,
            &UnexpectedIndex,
            &cursor_digest_order,
            &dummy_bytes,
        );
        assert!(resolver
            .validate(serde_json::from_value(wrong_cursor_digest).unwrap())
            .is_err());
    }

    #[test]
    fn index_proof_rejects_missing_duplicate_reordered_unknown_and_substituted_rows() {
        let registry = EvidenceSourceRegistry::task8().unwrap();
        let (partition, source_ref, evidence) = routine_partition_fixture();
        let bytes = serde_json_canonicalizer::to_vec(&evidence).unwrap();
        let baseline = routine_candidate(&registry, source_ref.clone());

        let mut candidates = Vec::new();
        let mut missing = baseline.clone();
        missing.availability.pop();
        candidates.push(missing);
        let mut duplicate = baseline.clone();
        duplicate.availability[1] = duplicate.availability[0].clone();
        candidates.push(duplicate);
        let mut reordered = baseline.clone();
        reordered.availability.reverse();
        candidates.push(reordered);
        let mut unknown = baseline.clone();
        unknown.availability[1] =
            EvidenceSourceIndexCandidateRow::unknown(EvidenceKind::SupportPrerequisiteObservation);
        candidates.push(unknown);
        let mut wrong_registry = baseline.clone();
        wrong_registry.registry_digest = Sha256Digest::parse(SHA_A).unwrap();
        candidates.push(wrong_registry);
        let mut wrong_version = baseline.clone();
        wrong_version.repository_version = super::RepositoryVersion::parse("opaque-v2").unwrap();
        candidates.push(wrong_version);
        let mut multiple = baseline.clone();
        multiple.availability[0] = EvidenceSourceIndexCandidateRow::available(
            EvidenceKind::RoutineClassification,
            vec![source_ref.clone(), source_ref.clone()],
        );
        candidates.push(multiple);
        let mut multiple_support = baseline.clone();
        let support_ref = RepositoryHistorySourceEvidenceRef::new(
            EvidenceKind::SupportPrerequisiteObservation,
            SHA_A,
        )
        .unwrap();
        multiple_support.availability[1] = EvidenceSourceIndexCandidateRow::available(
            EvidenceKind::SupportPrerequisiteObservation,
            vec![support_ref.clone(), support_ref],
        );
        candidates.push(multiple_support);

        for candidate in candidates {
            assert!(validate_routine_fixture(
                partition.clone(),
                candidate,
                routine_order(),
                bytes.clone(),
                &source_ref,
            )
            .is_err());
        }
    }

    #[test]
    fn loader_rejects_missing_wrong_type_noncanonical_and_digest_substitution() {
        let registry = EvidenceSourceRegistry::task8().unwrap();
        let (partition, source_ref, evidence) = routine_partition_fixture();
        let candidate = routine_candidate(&registry, source_ref.clone());

        for invalid_bytes in [
            b"{}".to_vec(),
            serde_json::to_vec_pretty(&evidence).unwrap(),
            b"{\"repositoryVersion\":\"opaque-v1\"}".to_vec(),
        ] {
            assert!(validate_routine_fixture(
                partition.clone(),
                candidate.clone(),
                routine_order(),
                invalid_bytes,
                &source_ref,
            )
            .is_err());
        }

        let mut semantic_substitution = partition.clone();
        semantic_substitution["entries"][0]["semanticDeltaDigest"] = json!(SHA_A);
        recalculate_partition_digest(&mut semantic_substitution);
        assert!(validate_routine_fixture(
            semantic_substitution,
            candidate.clone(),
            routine_order(),
            serde_json_canonicalizer::to_vec(&evidence).unwrap(),
            &source_ref,
        )
        .is_err());

        let mut partition_substitution = partition;
        partition_substitution["partitionDigest"] = json!(SHA_A);
        assert!(validate_routine_fixture(
            partition_substitution,
            candidate,
            routine_order(),
            serde_json_canonicalizer::to_vec(&evidence).unwrap(),
            &source_ref,
        )
        .is_err());
    }

    #[test]
    fn wire_task_commit_branch_exists_but_task8_generic_validation_rejects_it() {
        let registry = EvidenceSourceRegistry::task8().unwrap();
        let (mut support_partition, source_ref, evidence) = routine_partition_fixture();
        support_partition["entries"][0]["classification"] = json!("authorizedSupport");
        support_partition["entries"][0]["sourceEvidenceRef"]["evidenceKind"] =
            json!("supportPrerequisiteObservation");
        recalculate_partition_digest(&mut support_partition);
        assert!(
            serde_json::from_value::<UnvalidatedRepositoryHistoryPartition>(
                support_partition.clone()
            )
            .is_ok()
        );
        assert!(validate_routine_fixture(
            support_partition,
            routine_candidate(&registry, source_ref.clone()),
            routine_order(),
            serde_json_canonicalizer::to_vec(&evidence).unwrap(),
            &source_ref,
        )
        .is_err());

        let mut task_commit = json!({
            "fromExclusive":cursor("opaque-v0", SHA_A),
            "throughInclusive":cursor("opaque-v1", SHA_B),
            "entries":[{
                "repositoryVersion":"opaque-v1",
                "classification":"taskCommit",
                "semanticDeltaDigest":SHA_A
            }]
        });
        recalculate_partition_digest(&mut task_commit);
        assert!(
            serde_json::from_value::<UnvalidatedRepositoryHistoryPartition>(task_commit.clone())
                .is_ok()
        );
        assert!(validate_routine_fixture(
            task_commit,
            routine_candidate(&registry, source_ref.clone()),
            routine_order(),
            serde_json_canonicalizer::to_vec(&evidence).unwrap(),
            &source_ref,
        )
        .is_err());
    }

    #[test]
    fn task_commit_is_rejected_before_any_capability_or_source_lookup() {
        let registry = EvidenceSourceRegistry::task8().unwrap();
        let mut task_commit = json!({
            "fromExclusive":cursor("opaque-v0", SHA_A),
            "throughInclusive":cursor("opaque-v1", SHA_B),
            "entries":[{
                "repositoryVersion":"opaque-v1",
                "classification":"taskCommit",
                "semanticDeltaDigest":SHA_A
            }]
        });
        recalculate_partition_digest(&mut task_commit);
        let dummy_bytes = FakeEvidenceBytes::default();
        let resolver = RepositoryHistoryPartitionResolver::new(
            &registry,
            &UnexpectedIndex,
            &UnexpectedOrder,
            &dummy_bytes,
        );
        assert!(resolver
            .validate(serde_json::from_value(task_commit).unwrap())
            .is_err());
    }

    #[test]
    fn commit_owned_partition_accepts_only_the_exact_singleton_task_version_and_digest() {
        let registry = EvidenceSourceRegistry::task8().unwrap();
        let capability = CapabilityRowId::parse("repository.atomic-commit-safety.v1").unwrap();
        let authority = validated_commit_object_authority_fixture_test_only(
            RepositoryVersion::parse("opaque-v1").unwrap(),
            capability,
        );
        let mut task_commit = json!({
            "fromExclusive":cursor("opaque-v0", SHA_A),
            "throughInclusive":cursor("opaque-v1", SHA_B),
            "entries":[{
                "repositoryVersion":"opaque-v1",
                "classification":"taskCommit",
                "semanticDeltaDigest":authority.committed_objects_digest()
            }]
        });
        recalculate_partition_digest(&mut task_commit);
        let evidence_bytes = FakeEvidenceBytes::default();
        let order = FakeOrder {
            evidence: routine_order(),
        };
        let resolver = RepositoryHistoryPartitionResolver::new(
            &registry,
            &UnexpectedIndex,
            &order,
            &evidence_bytes,
        );

        let validated = resolver
            .validate_task_commit_partition(
                serde_json::from_value(task_commit.clone()).unwrap(),
                &authority,
            )
            .unwrap();
        assert!(validated.binds(&authority));
        let foreign_authority = validated_commit_object_authority_fixture_test_only(
            RepositoryVersion::parse("opaque-v1").unwrap(),
            CapabilityRowId::parse("repository.other-atomic-safety.v1").unwrap(),
        );
        assert!(!validated.binds(&foreign_authority));

        task_commit["entries"][0]["semanticDeltaDigest"] = json!(SHA_A);
        recalculate_partition_digest(&mut task_commit);
        assert!(resolver
            .validate_task_commit_partition(
                serde_json::from_value(task_commit).unwrap(),
                &authority,
            )
            .is_err());
    }

    #[test]
    fn commit_owned_partition_resolves_every_non_task_entry_through_the_source_index() {
        let registry = EvidenceSourceRegistry::task8().unwrap();
        let (routine, source_ref, evidence) = routine_partition_fixture();
        let authority = validated_commit_object_authority_fixture_test_only(
            RepositoryVersion::parse("opaque-v2").unwrap(),
            CapabilityRowId::parse("repository.atomic-commit-safety.v1").unwrap(),
        );
        let mut partition = json!({
            "fromExclusive":cursor("opaque-v0", SHA_A),
            "throughInclusive":cursor("opaque-v2", SHA_A),
            "entries":[
                routine["entries"][0].clone(),
                {
                    "repositoryVersion":"opaque-v2",
                    "classification":"taskCommit",
                    "semanticDeltaDigest":authority.committed_objects_digest()
                }
            ]
        });
        recalculate_partition_digest(&mut partition);
        let order = FakeOrder {
            evidence: RepositoryHistoryOrderEvidence::from_capability_adapter(
                "history-order-v1",
                serde_json::from_value(cursor("opaque-v0", SHA_A)).unwrap(),
                serde_json::from_value(cursor("opaque-v2", SHA_A)).unwrap(),
                vec![
                    serde_json::from_value(cursor("opaque-v1", SHA_B)).unwrap(),
                    serde_json::from_value(cursor("opaque-v2", SHA_A)).unwrap(),
                ],
            )
            .unwrap(),
        };
        let index = FakeIndex {
            candidates: BTreeMap::from([(
                "opaque-v1".into(),
                routine_candidate(&registry, source_ref.clone()),
            )]),
        };
        let evidence_bytes = FakeEvidenceBytes {
            bytes: BTreeMap::from([(
                (
                    EvidenceKind::RoutineClassification,
                    source_ref.evidence_digest().as_str().to_owned(),
                ),
                serde_json_canonicalizer::to_vec(&evidence).unwrap(),
            )]),
        };
        let resolver =
            RepositoryHistoryPartitionResolver::new(&registry, &index, &order, &evidence_bytes);

        assert!(resolver
            .validate_task_commit_partition(
                serde_json::from_value(partition.clone()).unwrap(),
                &authority,
            )
            .is_ok());

        let missing_index = FakeIndex {
            candidates: BTreeMap::new(),
        };
        assert!(RepositoryHistoryPartitionResolver::new(
            &registry,
            &missing_index,
            &order,
            &evidence_bytes,
        )
        .validate_task_commit_partition(serde_json::from_value(partition).unwrap(), &authority)
        .is_err());
    }

    fn non_conflicting_partition_fixture() -> (
        Value,
        RepositoryHistorySourceEvidenceRef,
        NonConflictingConcurrentEvidence,
    ) {
        let evidence = NonConflictingConcurrentEvidence::new(
            "opaque-v1",
            UUID_A,
            SHA_A,
            SHA_B,
            SHA_A,
            SHA_B,
            SHA_A,
        )
        .unwrap();
        let evidence_value = serde_json::to_value(&evidence).unwrap();
        let evidence_digest = evidence_value["evidenceDigest"].as_str().unwrap();
        let source_ref = RepositoryHistorySourceEvidenceRef::new(
            EvidenceKind::NonConflictingConcurrent,
            evidence_digest,
        )
        .unwrap();
        let semantic = json!({
            "repositoryVersion":"opaque-v1",
            "partitionClassification":"nonConflictingConcurrent",
            "rootDeltaDigest":null,
            "contentDeltaDigest":null,
            "classificationDigest":null,
            "externalSupportDisjointnessDigest":null,
            "correctiveInstructionDigest":null,
            "nonConflictingConcurrentEvidenceDigest":evidence_digest
        });
        let entry = json!({
            "repositoryVersion":"opaque-v1",
            "classification":"nonConflictingConcurrent",
            "semanticDeltaDigest":test_digest(&semantic),
            "sourceEvidenceRef":serde_json::to_value(&source_ref).unwrap(),
            "nonConflictingConcurrentEvidence":evidence_value
        });
        let mut partition = json!({
            "fromExclusive":cursor("opaque-v0", SHA_A),
            "throughInclusive":cursor("opaque-v1", SHA_B),
            "entries":[entry]
        });
        recalculate_partition_digest(&mut partition);
        (partition, source_ref, evidence)
    }

    fn validate_non_conflicting_fixture(
        partition_json: Value,
        source_ref: &RepositoryHistorySourceEvidenceRef,
        evidence: &NonConflictingConcurrentEvidence,
    ) -> Result<super::ValidatedRepositoryHistoryPartition, super::RepositoryContractError> {
        let registry = EvidenceSourceRegistry::task8().unwrap();
        let candidate = EvidenceSourceIndexCandidate::from_capability_adapter(
            "opaque-v1",
            registry.registry_digest().as_str(),
            UUID_A,
            vec![
                EvidenceSourceIndexCandidateRow::available(
                    EvidenceKind::RoutineClassification,
                    vec![RepositoryHistorySourceEvidenceRef::new(
                        EvidenceKind::RoutineClassification,
                        SHA_A,
                    )
                    .unwrap()],
                ),
                EvidenceSourceIndexCandidateRow::absent(
                    EvidenceKind::SupportPrerequisiteObservation,
                ),
                EvidenceSourceIndexCandidateRow::available(
                    EvidenceKind::NonConflictingConcurrent,
                    vec![source_ref.clone()],
                ),
            ],
        )
        .unwrap();
        let index = FakeIndex {
            candidates: BTreeMap::from([("opaque-v1".into(), candidate)]),
        };
        let order = FakeOrder {
            evidence: routine_order(),
        };
        let evidence_bytes = FakeEvidenceBytes {
            bytes: BTreeMap::from([(
                (
                    EvidenceKind::NonConflictingConcurrent,
                    source_ref.evidence_digest().as_str().to_owned(),
                ),
                serde_json_canonicalizer::to_vec(evidence).unwrap(),
            )]),
        };
        RepositoryHistoryPartitionResolver::new(&registry, &index, &order, &evidence_bytes)
            .validate(serde_json::from_value(partition_json).unwrap())
    }

    #[test]
    fn concurrent_source_has_precedence_and_requires_exact_inline_evidence() {
        let registry = EvidenceSourceRegistry::task8().unwrap();
        let (partition, source_ref, evidence) = non_conflicting_partition_fixture();
        let routine_ref =
            RepositoryHistorySourceEvidenceRef::new(EvidenceKind::RoutineClassification, SHA_A)
                .unwrap();
        let candidate = EvidenceSourceIndexCandidate::from_capability_adapter(
            "opaque-v1",
            registry.registry_digest().as_str(),
            UUID_A,
            vec![
                EvidenceSourceIndexCandidateRow::available(
                    EvidenceKind::RoutineClassification,
                    vec![routine_ref],
                ),
                EvidenceSourceIndexCandidateRow::absent(
                    EvidenceKind::SupportPrerequisiteObservation,
                ),
                EvidenceSourceIndexCandidateRow::available(
                    EvidenceKind::NonConflictingConcurrent,
                    vec![source_ref.clone()],
                ),
            ],
        )
        .unwrap();
        let index = FakeIndex {
            candidates: BTreeMap::from([("opaque-v1".into(), candidate.clone())]),
        };
        let order = FakeOrder {
            evidence: routine_order(),
        };
        let evidence_bytes = FakeEvidenceBytes {
            bytes: BTreeMap::from([(
                (
                    EvidenceKind::NonConflictingConcurrent,
                    source_ref.evidence_digest().as_str().to_owned(),
                ),
                serde_json_canonicalizer::to_vec(&evidence).unwrap(),
            )]),
        };
        let resolver =
            RepositoryHistoryPartitionResolver::new(&registry, &index, &order, &evidence_bytes);
        let validated = resolver
            .validate(serde_json::from_value(partition.clone()).unwrap())
            .unwrap();
        assert_eq!(serde_json::to_value(validated).unwrap(), partition);

        let validated = resolver
            .validate(serde_json::from_value(partition.clone()).unwrap())
            .unwrap();
        let entries = validated.support_recovery_entries().collect::<Vec<_>>();
        assert_eq!(entries.len(), 1);
        let ValidatedSupportRecoveryHistoryEntryRef::NonConflicting {
            repository_version,
            semantic_delta_digest,
            source_evidence_digest,
            evidence: projected_evidence,
        } = entries[0]
        else {
            panic!("validated NCC entry must retain its typed evidence")
        };
        assert_eq!(repository_version.as_str(), "opaque-v1");
        assert_eq!(
            semantic_delta_digest.as_str(),
            partition["entries"][0]["semanticDeltaDigest"]
        );
        assert_eq!(source_evidence_digest, evidence.evidence_digest());
        assert_eq!(projected_evidence, &evidence);
        assert!(validated.has_exact_entry_prefix(&validated));

        let replacement = NonConflictingConcurrentEvidence::new(
            "opaque-v1",
            UUID_A,
            SHA_B,
            SHA_B,
            SHA_A,
            SHA_B,
            SHA_A,
        )
        .unwrap();
        let mut inline_substitution = partition.clone();
        inline_substitution["entries"][0]["nonConflictingConcurrentEvidence"] =
            serde_json::to_value(replacement).unwrap();
        recalculate_partition_digest(&mut inline_substitution);
        assert!(resolver
            .validate(serde_json::from_value(inline_substitution).unwrap())
            .is_err());

        let (routine_partition, routine_ref, routine_evidence) = routine_partition_fixture();
        let higher_available = EvidenceSourceIndexCandidate::from_capability_adapter(
            "opaque-v1",
            registry.registry_digest().as_str(),
            UUID_A,
            vec![
                EvidenceSourceIndexCandidateRow::available(
                    EvidenceKind::RoutineClassification,
                    vec![routine_ref.clone()],
                ),
                EvidenceSourceIndexCandidateRow::absent(
                    EvidenceKind::SupportPrerequisiteObservation,
                ),
                EvidenceSourceIndexCandidateRow::available(
                    EvidenceKind::NonConflictingConcurrent,
                    vec![source_ref],
                ),
            ],
        )
        .unwrap();
        assert!(validate_routine_fixture(
            routine_partition,
            higher_available,
            routine_order(),
            serde_json_canonicalizer::to_vec(&routine_evidence).unwrap(),
            &routine_ref,
        )
        .is_err());

        let mut ref_substitution = partition;
        ref_substitution["entries"][0]["sourceEvidenceRef"]["evidenceDigest"] = json!(SHA_A);
        recalculate_partition_digest(&mut ref_substitution);
        assert!(resolver
            .validate(serde_json::from_value(ref_substitution).unwrap())
            .is_err());
    }

    #[test]
    fn gate_b3_scoped_ncc_canonical_sets_reject_duplicate_reversed_and_empty_inputs() {
        let root = RepositoryTargetIdentity::configuration_root();
        let object_a = scoped_ncc_target(OBJECT_A);
        let object_b = scoped_ncc_target(OBJECT_B);

        assert!(
            super::CanonicalRepositoryTargetSet::new(vec![root.clone(), root.clone()]).is_err()
        );
        assert!(
            super::CanonicalRepositoryTargetSet::new(vec![object_a.clone(), root.clone()]).is_err()
        );
        assert!(super::NonEmptyCanonicalRepositoryTargetSet::new(Vec::new()).is_err());

        let edge_a = super::RepositoryReferenceEdge::new(root.clone(), object_a);
        let edge_b = super::RepositoryReferenceEdge::new(root, object_b);
        assert!(super::CanonicalRepositoryReferenceEdgeSet::new(vec![
            edge_a.clone(),
            edge_a.clone(),
        ])
        .is_err());
        assert!(super::CanonicalRepositoryReferenceEdgeSet::new(vec![edge_b, edge_a]).is_err());
    }

    #[derive(Clone, Copy)]
    enum ScopedNccScanMutation {
        None,
        PortError,
        MissingNccFacts,
        MissingRow,
        ExtraRow,
        ReorderedRows,
        DuplicatePosition,
        FactsAtRoutinePosition,
        ForeignCursor,
        ForeignVersion,
        ForeignPartition,
        ForeignStartEndpoint,
        ForeignEndpoint,
        ForeignCapability,
        ForeignCompletionCapability,
        ForeignLeaseWitness,
        LeaseBindsFalse,
        ForeignBatchWitness,
        CrossScanRowSplice,
        RemovedReferenceEdge,
        NoGenuineExpansion,
        AddedEdgeSetMismatch,
        IntegrationOverlap,
        ChangedLockedOverlap,
        ChangedReferrerMismatch,
        RootChangedReferrer,
        SupportGraphChanged,
        ValidationInputsChanged,
        RootStateChanged,
        LockedStateChanged,
        AbsentLockedState,
        BlocksApprovedDeletion,
    }

    #[derive(Clone, Copy)]
    enum ScopedNccEvidenceMutation {
        None,
        RepositoryVersion,
        Capability,
        LockedTargetDigest,
        ChangedTargetDigest,
        BeforeClosureDigest,
        AfterClosureDigest,
        AddedEdgeDigest,
    }

    fn scoped_ncc_target(object_id: &str) -> RepositoryTargetIdentity {
        RepositoryTargetIdentity::development_object(MetadataObjectId::parse(object_id).unwrap())
    }

    fn scoped_ncc_states(value: Value) -> RepositoryTargetStates {
        serde_json::from_value(value).unwrap()
    }

    fn scoped_ncc_safe_facts() -> super::ScopedNccRowFacts {
        let root = RepositoryTargetIdentity::configuration_root();
        let locked = scoped_ncc_target(OBJECT_A);
        let changed = scoped_ncc_target(OBJECT_B);
        let before_edge = super::RepositoryReferenceEdge::new(root.clone(), locked.clone());
        let added_edge = super::RepositoryReferenceEdge::new(changed.clone(), locked.clone());
        let before_reference_closure =
            super::CanonicalRepositoryReferenceEdgeSet::new(vec![before_edge.clone()]).unwrap();
        let added_reference_edges =
            super::CanonicalRepositoryReferenceEdgeSet::new(vec![added_edge]).unwrap();
        let after_reference_closure = super::CanonicalRepositoryReferenceEdgeSet::new(vec![
            before_edge,
            super::RepositoryReferenceEdge::new(changed.clone(), locked.clone()),
        ])
        .unwrap();
        let empty_edges = super::CanonicalRepositoryReferenceEdgeSet::new(Vec::new()).unwrap();
        let target_sets = super::ScopedNccObservedTargetSets::new(
            super::NonEmptyCanonicalRepositoryTargetSet::new(vec![locked]).unwrap(),
            super::NonEmptyCanonicalRepositoryTargetSet::new(vec![changed]).unwrap(),
            super::NonEmptyCanonicalRepositoryTargetSet::new(vec![root]).unwrap(),
            super::CanonicalRepositoryTargetSet::new(Vec::new()).unwrap(),
        );
        let reference_sets = super::ScopedNccObservedReferenceSets::new(
            before_reference_closure,
            after_reference_closure,
            added_reference_edges,
            empty_edges.clone(),
            empty_edges,
        );
        let root_states = scoped_ncc_states(json!([{
            "targetKind":"configurationRoot",
            "state":"present",
            "repositoryVersion":"root-v1",
            "targetFingerprint":SHA_A
        }]));
        let locked_states = scoped_ncc_states(json!([{
            "targetKind":"developmentObject",
            "state":"present",
            "objectId":OBJECT_A,
            "repositoryVersion":"locked-v1",
            "targetFingerprint":SHA_A
        }]));
        let validation_inputs = scoped_ncc_states(json!([
            {
                "targetKind":"configurationRoot",
                "state":"present",
                "repositoryVersion":"root-v1",
                "targetFingerprint":SHA_A
            },
            {
                "targetKind":"developmentObject",
                "state":"present",
                "objectId":OBJECT_A,
                "repositoryVersion":"locked-v1",
                "targetFingerprint":SHA_A
            }
        ]));
        let states = super::ScopedNccObservedStateSets::new(
            validation_inputs.clone(),
            validation_inputs,
            root_states.clone(),
            root_states,
            locked_states.clone(),
            locked_states,
        );
        super::ScopedNccRowFacts::new(
            CapabilityRowId::parse(UUID_A).unwrap(),
            target_sets,
            reference_sets,
            states,
        )
    }

    fn scoped_ncc_evidence(
        facts: &super::ScopedNccRowFacts,
        mutation: ScopedNccEvidenceMutation,
    ) -> NonConflictingConcurrentEvidence {
        let repository_version = match mutation {
            ScopedNccEvidenceMutation::RepositoryVersion => "opaque-v-foreign",
            _ => "opaque-v2",
        };
        let capability = match mutation {
            ScopedNccEvidenceMutation::Capability => UUID_B,
            _ => UUID_A,
        };
        let locked = facts.target_sets.locked_targets.digest().unwrap();
        let changed = facts.target_sets.changed_targets.digest().unwrap();
        let before = facts
            .reference_sets
            .before_reference_closure
            .digest()
            .unwrap();
        let after = facts
            .reference_sets
            .after_reference_closure
            .digest()
            .unwrap();
        let added = facts.reference_sets.added_reference_edges.digest().unwrap();
        NonConflictingConcurrentEvidence::new(
            repository_version,
            capability,
            if matches!(mutation, ScopedNccEvidenceMutation::LockedTargetDigest) {
                SHA_B
            } else {
                locked.as_str()
            },
            if matches!(mutation, ScopedNccEvidenceMutation::ChangedTargetDigest) {
                SHA_A
            } else {
                changed.as_str()
            },
            if matches!(mutation, ScopedNccEvidenceMutation::BeforeClosureDigest) {
                SHA_B
            } else {
                before.as_str()
            },
            if matches!(mutation, ScopedNccEvidenceMutation::AfterClosureDigest) {
                SHA_A
            } else {
                after.as_str()
            },
            if matches!(mutation, ScopedNccEvidenceMutation::AddedEdgeDigest) {
                SHA_B
            } else {
                added.as_str()
            },
        )
        .unwrap()
    }

    fn scoped_ncc_partition(
        facts: &super::ScopedNccRowFacts,
        mutation: ScopedNccEvidenceMutation,
    ) -> super::ValidatedRepositoryHistoryPartition {
        let from = RepositoryHistoryCursor::new(
            RepositoryVersion::parse("opaque-v0").unwrap(),
            Sha256Digest::parse(SHA_A).unwrap(),
        );
        let mut partition = super::repository_history_partition_fixture_test_only(
            from,
            vec![
                (
                    RepositoryVersion::parse("opaque-v1").unwrap(),
                    super::RepositoryHistoryPartitionClassification::UnrelatedRoutine,
                ),
                (
                    RepositoryVersion::parse("opaque-v2").unwrap(),
                    super::RepositoryHistoryPartitionClassification::NonConflictingConcurrent,
                ),
            ],
            "history-order-v1",
            UUID_A,
        )
        .unwrap();
        let evidence = scoped_ncc_evidence(facts, mutation);
        let entry = match &mut partition.wire.entries.0[1] {
            super::RepositoryHistoryPartitionEntry::NonConflicting(entry) => entry,
            _ => panic!("fixture row two must be NCC"),
        };
        entry.repository_version = RepositoryVersion::parse("opaque-v2").unwrap();
        entry.source_evidence_ref = RepositoryHistorySourceEvidenceRef::new(
            EvidenceKind::NonConflictingConcurrent,
            evidence.evidence_digest().as_str(),
        )
        .unwrap();
        entry.non_conflicting_concurrent_evidence = evidence;
        entry.semantic_delta_digest = super::canonical_contract_digest(
            &super::RepositorySemanticDeltaDigestRecord {
                repository_version: entry.repository_version.clone(),
                partition_classification:
                    super::RepositoryHistoryPartitionClassification::NonConflictingConcurrent,
                root_delta_digest: RequiredNullable::null(),
                content_delta_digest: RequiredNullable::null(),
                classification_digest: RequiredNullable::null(),
                external_support_disjointness_digest: RequiredNullable::null(),
                corrective_instruction_digest: RequiredNullable::null(),
                non_conflicting_concurrent_evidence_digest: RequiredNullable::value(
                    entry
                        .non_conflicting_concurrent_evidence
                        .evidence_digest()
                        .clone(),
                ),
            },
            None,
        )
        .unwrap();
        partition.wire.partition_digest = super::canonical_contract_digest(
            &super::RepositoryHistoryPartitionDigestRecord {
                from_exclusive: partition.wire.from_exclusive.clone(),
                through_inclusive: partition.wire.through_inclusive.clone(),
                entries: partition.wire.entries.clone(),
            },
            None,
        )
        .unwrap();
        partition
    }

    fn apply_scoped_ncc_safety_mutation(
        facts: &mut super::ScopedNccRowFacts,
        mutation: ScopedNccScanMutation,
    ) {
        match mutation {
            ScopedNccScanMutation::RemovedReferenceEdge => {
                facts.reference_sets.after_reference_closure =
                    super::CanonicalRepositoryReferenceEdgeSet::new(Vec::new()).unwrap();
            }
            ScopedNccScanMutation::NoGenuineExpansion => {
                facts.reference_sets.after_reference_closure =
                    facts.reference_sets.before_reference_closure.clone();
                facts.reference_sets.added_reference_edges =
                    super::CanonicalRepositoryReferenceEdgeSet::new(Vec::new()).unwrap();
            }
            ScopedNccScanMutation::AddedEdgeSetMismatch => {
                facts.reference_sets.added_reference_edges =
                    super::CanonicalRepositoryReferenceEdgeSet::new(Vec::new()).unwrap();
            }
            ScopedNccScanMutation::IntegrationOverlap => {
                facts.target_sets.integration_content_targets =
                    facts.target_sets.changed_targets.clone();
            }
            ScopedNccScanMutation::ChangedLockedOverlap => {
                facts.target_sets.changed_targets = facts.target_sets.locked_targets.clone();
                let root = RepositoryTargetIdentity::configuration_root();
                let locked = scoped_ncc_target(OBJECT_A);
                let referenced = scoped_ncc_target(OBJECT_B);
                facts.reference_sets.added_reference_edges =
                    super::CanonicalRepositoryReferenceEdgeSet::new(vec![
                        super::RepositoryReferenceEdge::new(locked.clone(), referenced.clone()),
                    ])
                    .unwrap();
                facts.reference_sets.after_reference_closure =
                    super::CanonicalRepositoryReferenceEdgeSet::new(vec![
                        super::RepositoryReferenceEdge::new(root, locked.clone()),
                        super::RepositoryReferenceEdge::new(locked, referenced),
                    ])
                    .unwrap();
            }
            ScopedNccScanMutation::ChangedReferrerMismatch => {
                let root = RepositoryTargetIdentity::configuration_root();
                let locked = scoped_ncc_target(OBJECT_A);
                let changed = scoped_ncc_target(OBJECT_B);
                facts.reference_sets.added_reference_edges =
                    super::CanonicalRepositoryReferenceEdgeSet::new(vec![
                        super::RepositoryReferenceEdge::new(root.clone(), changed),
                    ])
                    .unwrap();
                facts.reference_sets.after_reference_closure =
                    super::CanonicalRepositoryReferenceEdgeSet::new(vec![
                        super::RepositoryReferenceEdge::new(root.clone(), locked),
                        super::RepositoryReferenceEdge::new(root, scoped_ncc_target(OBJECT_B)),
                    ])
                    .unwrap();
            }
            ScopedNccScanMutation::RootChangedReferrer => {
                let root = RepositoryTargetIdentity::configuration_root();
                let locked = scoped_ncc_target(OBJECT_A);
                let referenced = scoped_ncc_target(OBJECT_B);
                facts.target_sets.changed_targets =
                    super::NonEmptyCanonicalRepositoryTargetSet::new(vec![root.clone()]).unwrap();
                facts.target_sets.integration_content_targets =
                    super::NonEmptyCanonicalRepositoryTargetSet::new(vec![referenced.clone()])
                        .unwrap();
                facts.reference_sets.added_reference_edges =
                    super::CanonicalRepositoryReferenceEdgeSet::new(vec![
                        super::RepositoryReferenceEdge::new(root.clone(), referenced.clone()),
                    ])
                    .unwrap();
                facts.reference_sets.after_reference_closure =
                    super::CanonicalRepositoryReferenceEdgeSet::new(vec![
                        super::RepositoryReferenceEdge::new(root.clone(), locked),
                        super::RepositoryReferenceEdge::new(root, referenced),
                    ])
                    .unwrap();
            }
            ScopedNccScanMutation::SupportGraphChanged => {
                facts.reference_sets.after_support_graph =
                    facts.reference_sets.added_reference_edges.clone();
            }
            ScopedNccScanMutation::ValidationInputsChanged => {
                facts.states.after_validation_inputs = scoped_ncc_states(json!([]));
            }
            ScopedNccScanMutation::RootStateChanged => {
                facts.states.after_root_states = scoped_ncc_states(json!([{
                    "targetKind":"configurationRoot",
                    "state":"present",
                    "repositoryVersion":"root-v2",
                    "targetFingerprint":SHA_B
                }]));
            }
            ScopedNccScanMutation::LockedStateChanged => {
                facts.states.after_locked_target_states = scoped_ncc_states(json!([{
                    "targetKind":"developmentObject",
                    "state":"present",
                    "objectId":OBJECT_A,
                    "repositoryVersion":"locked-v2",
                    "targetFingerprint":SHA_B
                }]));
            }
            ScopedNccScanMutation::AbsentLockedState => {
                let absent = scoped_ncc_states(json!([{
                    "targetKind":"developmentObject",
                    "state":"absent",
                    "objectId":OBJECT_A,
                    "absenceEstablishedAtVersion":"locked-v1",
                    "expectedAbsent":true
                }]));
                facts.states.before_locked_target_states = absent.clone();
                facts.states.after_locked_target_states = absent;
            }
            ScopedNccScanMutation::BlocksApprovedDeletion => {
                facts.target_sets.approved_deletion_targets =
                    super::CanonicalRepositoryTargetSet::new(vec![scoped_ncc_target(OBJECT_A)])
                        .unwrap();
            }
            _ => {}
        }
    }

    struct FixtureScopedNccLease {
        witness: super::ScopedNccHistoryScanBatchWitness,
        observation: super::ScopedNccHistoryScanObservation,
        binds: bool,
        observation_consumptions: Option<Arc<AtomicUsize>>,
    }

    impl super::ScopedNccHistoryScanLease for FixtureScopedNccLease {
        fn batch_witness(&self) -> &super::ScopedNccHistoryScanBatchWitness {
            &self.witness
        }

        fn binds(&self, _request: &super::ScopedNccHistoryScanRequest<'_>) -> bool {
            self.binds
        }

        fn into_observation(self: Box<Self>) -> super::ScopedNccHistoryScanObservation {
            if let Some(observation_consumptions) = &self.observation_consumptions {
                observation_consumptions.fetch_add(1, AtomicOrdering::SeqCst);
            }
            self.observation
        }
    }

    #[derive(Default)]
    struct FixtureScopedNccCompletionCapture {
        completion_marker: Option<Arc<super::ScopedNccHistoryScanInvocationMarker>>,
        lease_witness_marker: Option<Arc<super::ScopedNccHistoryScanInvocationMarker>>,
        observation_consumptions: Arc<AtomicUsize>,
    }

    struct FixtureScopedNccPort {
        facts: super::ScopedNccRowFacts,
        mutation: ScopedNccScanMutation,
        calls: usize,
        capture: Option<FixtureScopedNccCompletionCapture>,
    }

    impl super::ScopedNccHistoryScanPort for FixtureScopedNccPort {
        fn observe_scoped_ncc_history(
            &mut self,
            request: super::ScopedNccHistoryScanRequest<'_>,
        ) -> Result<super::ScopedNccHistoryScanCompletion, super::RepositoryContractError> {
            self.calls += 1;
            if matches!(self.mutation, ScopedNccScanMutation::PortError) {
                return Err(super::RepositoryContractError(
                    "scoped NCC fixture port error",
                ));
            }
            let mut rows = Vec::with_capacity(request.row_count());
            for index in 0..request.row_count() {
                let row = request.row(index).unwrap();
                let mut facts = (row.classification()
                    == super::RepositoryHistoryPartitionClassification::NonConflictingConcurrent)
                    .then(|| self.facts.clone());
                if index == 0
                    && matches!(self.mutation, ScopedNccScanMutation::FactsAtRoutinePosition)
                {
                    facts = Some(self.facts.clone());
                }
                if let Some(facts) = facts.as_mut() {
                    if matches!(self.mutation, ScopedNccScanMutation::ForeignCapability) {
                        facts.atomic_commit_safety_capability_id =
                            CapabilityRowId::parse(UUID_B).unwrap();
                    }
                }
                rows.push(
                    super::ScopedNccHistoryRowObservation::from_capability_adapter(
                        &request,
                        super::ScopedNccHistoryRowObservationInput::new(
                            row.position(),
                            row.cursor().clone(),
                            row.repository_version().clone(),
                            facts,
                        ),
                    )?,
                );
            }

            match self.mutation {
                ScopedNccScanMutation::MissingRow => {
                    rows.pop();
                }
                ScopedNccScanMutation::MissingNccFacts => rows[1].facts = None,
                ScopedNccScanMutation::ExtraRow => {
                    let row = request.row(request.row_count() - 1).unwrap();
                    rows.push(
                        super::ScopedNccHistoryRowObservation::from_capability_adapter(
                            &request,
                            super::ScopedNccHistoryRowObservationInput::new(
                                request.row_count() + 1,
                                row.cursor().clone(),
                                row.repository_version().clone(),
                                None,
                            ),
                        )?,
                    );
                }
                ScopedNccScanMutation::ReorderedRows => rows.swap(0, 1),
                ScopedNccScanMutation::DuplicatePosition => rows[1].position = rows[0].position,
                ScopedNccScanMutation::ForeignCursor => {
                    rows[1].cursor = serde_json::from_value(cursor("opaque-v2", SHA_A)).unwrap();
                }
                ScopedNccScanMutation::ForeignVersion => {
                    rows[1].repository_version =
                        RepositoryVersion::parse("opaque-v-foreign").unwrap();
                }
                ScopedNccScanMutation::CrossScanRowSplice => {
                    let foreign_invocation =
                        super::ScopedNccHistoryScanInvocationCapability::mint();
                    let foreign_request = super::ScopedNccHistoryScanRequest::new(
                        request.partition,
                        request.expected_capability_id,
                        &foreign_invocation,
                    );
                    let row = foreign_request.row(1).unwrap();
                    rows[1] = super::ScopedNccHistoryRowObservation::from_capability_adapter(
                        &foreign_request,
                        super::ScopedNccHistoryRowObservationInput::new(
                            row.position(),
                            row.cursor().clone(),
                            row.repository_version().clone(),
                            Some(self.facts.clone()),
                        ),
                    )?;
                }
                _ => {}
            }

            let witness = request.batch_witness();
            let mut batch = super::ScopedNccHistoryScanBatchInput::new(
                request.start_cursor().clone(),
                request.through_inclusive().clone(),
                request.partition_digest().clone(),
                rows,
            );
            if matches!(self.mutation, ScopedNccScanMutation::ForeignPartition) {
                batch.partition_digest = Sha256Digest::parse(SHA_A).unwrap();
            }
            if matches!(self.mutation, ScopedNccScanMutation::ForeignStartEndpoint) {
                batch.from_exclusive =
                    serde_json::from_value(cursor("opaque-v-foreign", SHA_A)).unwrap();
            }
            if matches!(self.mutation, ScopedNccScanMutation::ForeignEndpoint) {
                batch.through_inclusive =
                    serde_json::from_value(cursor("opaque-v-foreign", SHA_A)).unwrap();
            }
            let observation =
                super::ScopedNccHistoryScanObservation::from_capability_adapter(&request, batch)?;
            let foreign_invocation = super::ScopedNccHistoryScanInvocationCapability::mint();
            let lease_witness =
                if matches!(self.mutation, ScopedNccScanMutation::ForeignLeaseWitness) {
                    foreign_invocation.batch_witness()
                } else {
                    witness
                };
            let mut observation = observation;
            if matches!(self.mutation, ScopedNccScanMutation::ForeignBatchWitness) {
                observation.batch_witness = foreign_invocation.batch_witness();
            }
            let mut completion = request.complete(Box::new(FixtureScopedNccLease {
                witness: lease_witness,
                observation,
                binds: !matches!(self.mutation, ScopedNccScanMutation::LeaseBindsFalse),
                observation_consumptions: self
                    .capture
                    .as_ref()
                    .map(|capture| Arc::clone(&capture.observation_consumptions)),
            }));
            if matches!(
                self.mutation,
                ScopedNccScanMutation::ForeignCompletionCapability
            ) {
                completion.completion = foreign_invocation.completion();
            }
            if let Some(capture) = &mut self.capture {
                capture.completion_marker = Some(Arc::clone(&completion.completion.0));
                capture.lease_witness_marker =
                    Some(Arc::clone(&completion.lease.batch_witness().0));
            }
            Ok(completion)
        }
    }

    fn run_scoped_ncc_scan(
        scan_mutation: ScopedNccScanMutation,
        evidence_mutation: ScopedNccEvidenceMutation,
    ) -> Result<
        super::ScopedNccHistoryScanAuthority,
        Box<super::ScopedNccHistoryScanBlockedAuthority>,
    > {
        let mut facts = scoped_ncc_safe_facts();
        apply_scoped_ncc_safety_mutation(&mut facts, scan_mutation);
        let partition = scoped_ncc_partition(&facts, evidence_mutation);
        let mut port = FixtureScopedNccPort {
            facts,
            mutation: scan_mutation,
            calls: 0,
            capture: None,
        };
        super::ScopedNccHistoryScanAuthority::resolve(
            partition,
            CapabilityRowId::parse(UUID_A).unwrap(),
            &mut port,
        )
    }

    fn scoped_ncc_blocked(
        scan_mutation: ScopedNccScanMutation,
        evidence_mutation: ScopedNccEvidenceMutation,
    ) -> super::ScopedNccHistoryScanBlockedAuthority {
        *run_scoped_ncc_scan(scan_mutation, evidence_mutation)
            .expect_err("fixture mutation must be retained as a blocked scoped NCC scan")
    }

    fn scoped_ncc_blocked_with_completion_capture(
        scan_mutation: ScopedNccScanMutation,
    ) -> (
        super::ScopedNccHistoryScanBlockedAuthority,
        FixtureScopedNccCompletionCapture,
    ) {
        let facts = scoped_ncc_safe_facts();
        let partition = scoped_ncc_partition(&facts, ScopedNccEvidenceMutation::None);
        let mut port = FixtureScopedNccPort {
            facts,
            mutation: scan_mutation,
            calls: 0,
            capture: Some(FixtureScopedNccCompletionCapture::default()),
        };
        let blocked = super::ScopedNccHistoryScanAuthority::resolve(
            partition,
            CapabilityRowId::parse(UUID_A).unwrap(),
            &mut port,
        )
        .expect_err("completion fixture mutation must retain the owning completion");
        (
            *blocked,
            port.capture
                .take()
                .expect("fixture must capture the completion and lease payload"),
        )
    }

    fn assert_scoped_ncc_retained_source(
        source: &super::ScopedNccHistoryScanFailureSource,
        expected_entries: usize,
        expected_order_evidence: bool,
    ) {
        assert_eq!(source.partition.entry_count(), expected_entries);
        assert_eq!(source.expected_capability_id.as_str(), UUID_A);
        assert_eq!(
            source.partition.order_evidence.is_some(),
            expected_order_evidence
        );
    }

    #[test]
    fn gate_b3_scoped_ncc_blocked_failure_types_are_non_clone_and_non_wire() {
        let _ = <super::ScopedNccHistoryScanBlockedAuthority as AmbiguousIfClone<_>>::marker;
        let _ = <super::ScopedNccHistoryScanFailureSource as AmbiguousIfClone<_>>::marker;
        let _ = <super::ScopedNccHistoryScanFailureEvidence as AmbiguousIfClone<_>>::marker;
        let _ = <super::ScopedNccHistoryScanCompletion as AmbiguousIfClone<_>>::marker;
        let _ = <super::ScopedNccHistoryScanObservation as AmbiguousIfClone<_>>::marker;
        let _ =
            <super::ScopedNccHistoryScanBlockedAuthority as AmbiguousIfDeserializeOwned<_>>::marker;
        let _ =
            <super::ScopedNccHistoryScanFailureSource as AmbiguousIfDeserializeOwned<_>>::marker;
        let _ =
            <super::ScopedNccHistoryScanFailureEvidence as AmbiguousIfDeserializeOwned<_>>::marker;
        let _ = <super::ScopedNccHistoryScanCompletion as AmbiguousIfDeserializeOwned<_>>::marker;
        let _ = <super::ScopedNccHistoryScanObservation as AmbiguousIfDeserializeOwned<_>>::marker;
        let _ = <super::ScopedNccHistoryScanBlockedAuthority as AmbiguousIfSerialize<_>>::marker;
        let _ = <super::ScopedNccHistoryScanFailureSource as AmbiguousIfSerialize<_>>::marker;
        let _ = <super::ScopedNccHistoryScanFailureEvidence as AmbiguousIfSerialize<_>>::marker;
        let _ = <super::ScopedNccHistoryScanCompletion as AmbiguousIfSerialize<_>>::marker;
        let _ = <super::ScopedNccHistoryScanObservation as AmbiguousIfSerialize<_>>::marker;
    }

    #[test]
    fn gate_b3_scoped_ncc_source_failures_retain_exact_stage_and_never_call_the_port() {
        let facts = scoped_ncc_safe_facts();
        let partition =
            super::repository_history_partition_without_order_evidence_fixture_test_only(
                scoped_ncc_partition(&facts, ScopedNccEvidenceMutation::None),
            );
        let expected_digest = partition.partition_digest().clone();
        let mut port = FixtureScopedNccPort {
            facts: facts.clone(),
            mutation: ScopedNccScanMutation::None,
            calls: 0,
            capture: None,
        };
        let blocked = super::ScopedNccHistoryScanAuthority::resolve(
            partition,
            CapabilityRowId::parse(UUID_A).unwrap(),
            &mut port,
        )
        .expect_err("missing order evidence must retain an owning blocked result");
        assert_eq!(port.calls, 0);
        assert_scoped_ncc_retained_source(&blocked.source, 2, false);
        assert_eq!(
            blocked.source.partition.partition_digest(),
            &expected_digest
        );
        match blocked.evidence {
            super::ScopedNccHistoryScanFailureEvidence::InvalidSource { stage, error } => {
                assert_eq!(
                    stage,
                    super::ScopedNccHistoryScanSourceFailureStage::MissingOrderEvidence
                );
                assert_eq!(
                    error.0,
                    "scoped NCC scan requires non-empty validated history order"
                );
            }
            evidence => panic!("expected invalid-source evidence, got {evidence:?}"),
        }

        let from = RepositoryHistoryCursor::new(
            RepositoryVersion::parse("opaque-v0").unwrap(),
            Sha256Digest::parse(SHA_A).unwrap(),
        );
        let partition = super::repository_history_partition_fixture_test_only(
            from,
            vec![(
                RepositoryVersion::parse("opaque-v1").unwrap(),
                super::RepositoryHistoryPartitionClassification::UnrelatedRoutine,
            )],
            "history-order-v1",
            UUID_A,
        )
        .unwrap();
        let mut port = FixtureScopedNccPort {
            facts: scoped_ncc_safe_facts(),
            mutation: ScopedNccScanMutation::None,
            calls: 0,
            capture: None,
        };
        let blocked = super::ScopedNccHistoryScanAuthority::resolve(
            partition,
            CapabilityRowId::parse(UUID_A).unwrap(),
            &mut port,
        )
        .expect_err("partition without an NCC row must be an owning blocked result");
        assert_eq!(port.calls, 0);
        assert_scoped_ncc_retained_source(&blocked.source, 1, true);
        match blocked.evidence {
            super::ScopedNccHistoryScanFailureEvidence::InvalidSource { stage, error } => {
                assert_eq!(
                    stage,
                    super::ScopedNccHistoryScanSourceFailureStage::InvalidCoverage
                );
                assert_eq!(
                    error.0,
                    "scoped NCC scan requires exact non-empty NCC history coverage"
                );
            }
            evidence => panic!("expected invalid-source evidence, got {evidence:?}"),
        }
    }

    #[test]
    fn gate_b3_scoped_ncc_port_error_retains_source_and_error() {
        let blocked = scoped_ncc_blocked(
            ScopedNccScanMutation::PortError,
            ScopedNccEvidenceMutation::None,
        );
        assert_scoped_ncc_retained_source(&blocked.source, 2, true);
        match blocked.evidence {
            super::ScopedNccHistoryScanFailureEvidence::Port { error } => {
                assert_eq!(error.0, "scoped NCC fixture port error");
            }
            evidence => panic!("expected port evidence, got {evidence:?}"),
        }
    }

    #[test]
    fn gate_b3_scoped_ncc_completion_failures_retain_exact_stage_and_complete_payload() {
        for (mutation, expected_stage, expected_error) in [
            (
                ScopedNccScanMutation::ForeignCompletionCapability,
                super::ScopedNccHistoryScanCompletionFailureStage::ForeignCompletion,
                "scoped NCC scan completion is foreign to the request",
            ),
            (
                ScopedNccScanMutation::ForeignLeaseWitness,
                super::ScopedNccHistoryScanCompletionFailureStage::ForeignLeaseWitness,
                "scoped NCC scan lease is foreign to the request",
            ),
            (
                ScopedNccScanMutation::LeaseBindsFalse,
                super::ScopedNccHistoryScanCompletionFailureStage::LeaseBindingMismatch,
                "scoped NCC scan lease does not bind the request",
            ),
        ] {
            let (blocked, capture) = scoped_ncc_blocked_with_completion_capture(mutation);
            assert_scoped_ncc_retained_source(&blocked.source, 2, true);
            let expected_start = blocked.source.partition.start_cursor().clone();
            let expected_end = blocked.source.partition.through_inclusive().clone();
            let expected_digest = blocked.source.partition.partition_digest().clone();
            let expected_facts = scoped_ncc_safe_facts();
            match blocked.evidence {
                super::ScopedNccHistoryScanFailureEvidence::Completion {
                    stage,
                    error,
                    completion,
                } => {
                    assert_eq!(stage, expected_stage);
                    assert_eq!(error.0, expected_error);
                    assert!(Arc::ptr_eq(
                        &completion.completion.0,
                        capture
                            .completion_marker
                            .as_ref()
                            .expect("captured completion marker"),
                    ));
                    assert!(Arc::ptr_eq(
                        &completion.lease.batch_witness().0,
                        capture
                            .lease_witness_marker
                            .as_ref()
                            .expect("captured lease witness marker"),
                    ));
                    assert_eq!(
                        capture
                            .observation_consumptions
                            .load(AtomicOrdering::SeqCst),
                        0,
                        "pre-consumption failure must retain, not consume, the lease"
                    );
                    let observation = completion.lease.into_observation();
                    assert_eq!(observation.from_exclusive, expected_start);
                    assert_eq!(observation.through_inclusive, expected_end);
                    assert_eq!(observation.partition_digest, expected_digest);
                    assert_eq!(observation.rows.len(), 2);
                    assert!(observation.rows[0].facts.is_none());
                    assert_eq!(observation.rows[1].facts.as_ref(), Some(&expected_facts));
                    assert_eq!(
                        capture
                            .observation_consumptions
                            .load(AtomicOrdering::SeqCst),
                        1,
                        "the retained lease must expose its original observation exactly once"
                    );
                }
                evidence => panic!("expected completion evidence, got {evidence:?}"),
            }
        }
    }

    #[test]
    fn gate_b3_scoped_ncc_observation_failures_retain_exact_stage_and_full_observation() {
        for (scan_mutation, evidence_mutation, expected_stage, expected_error) in [
            (
                ScopedNccScanMutation::ForeignBatchWitness,
                ScopedNccEvidenceMutation::None,
                super::ScopedNccHistoryScanObservationFailureStage::ForeignBatchWitness,
                "scoped NCC scan batch witness is foreign to the request",
            ),
            (
                ScopedNccScanMutation::ForeignStartEndpoint,
                ScopedNccEvidenceMutation::None,
                super::ScopedNccHistoryScanObservationFailureStage::FromExclusiveMismatch,
                "scoped NCC scan batch start differs from the validated partition",
            ),
            (
                ScopedNccScanMutation::ForeignEndpoint,
                ScopedNccEvidenceMutation::None,
                super::ScopedNccHistoryScanObservationFailureStage::ThroughInclusiveMismatch,
                "scoped NCC scan batch end differs from the validated partition",
            ),
            (
                ScopedNccScanMutation::ForeignPartition,
                ScopedNccEvidenceMutation::None,
                super::ScopedNccHistoryScanObservationFailureStage::PartitionDigestMismatch,
                "scoped NCC scan batch digest differs from the validated partition",
            ),
            (
                ScopedNccScanMutation::MissingRow,
                ScopedNccEvidenceMutation::None,
                super::ScopedNccHistoryScanObservationFailureStage::RowCountMismatch,
                "scoped NCC scan batch row count differs from the validated partition",
            ),
            (
                ScopedNccScanMutation::ExtraRow,
                ScopedNccEvidenceMutation::None,
                super::ScopedNccHistoryScanObservationFailureStage::RowCountMismatch,
                "scoped NCC scan batch row count differs from the validated partition",
            ),
            (
                ScopedNccScanMutation::CrossScanRowSplice,
                ScopedNccEvidenceMutation::None,
                super::ScopedNccHistoryScanObservationFailureStage::RowWitnessMismatch,
                "scoped NCC row witness is foreign to the request",
            ),
            (
                ScopedNccScanMutation::ReorderedRows,
                ScopedNccEvidenceMutation::None,
                super::ScopedNccHistoryScanObservationFailureStage::RowPositionMismatch,
                "scoped NCC row position differs from its exact ordered history position",
            ),
            (
                ScopedNccScanMutation::DuplicatePosition,
                ScopedNccEvidenceMutation::None,
                super::ScopedNccHistoryScanObservationFailureStage::RowPositionMismatch,
                "scoped NCC row position differs from its exact ordered history position",
            ),
            (
                ScopedNccScanMutation::ForeignCursor,
                ScopedNccEvidenceMutation::None,
                super::ScopedNccHistoryScanObservationFailureStage::RowCursorMismatch,
                "scoped NCC row cursor differs from its exact ordered history position",
            ),
            (
                ScopedNccScanMutation::ForeignVersion,
                ScopedNccEvidenceMutation::None,
                super::ScopedNccHistoryScanObservationFailureStage::RowVersionMismatch,
                "scoped NCC row version differs from its exact ordered history position",
            ),
            (
                ScopedNccScanMutation::MissingNccFacts,
                ScopedNccEvidenceMutation::None,
                super::ScopedNccHistoryScanObservationFailureStage::MissingNccFacts,
                "scoped NCC history position lacks its detailed observation",
            ),
            (
                ScopedNccScanMutation::FactsAtRoutinePosition,
                ScopedNccEvidenceMutation::None,
                super::ScopedNccHistoryScanObservationFailureStage::UnexpectedNccFacts,
                "non-NCC history position carries NCC facts",
            ),
            (
                ScopedNccScanMutation::ForeignCapability,
                ScopedNccEvidenceMutation::None,
                super::ScopedNccHistoryScanObservationFailureStage::NccFactsMismatch,
                "scoped NCC facts differ from the audited evidence record",
            ),
        ] {
            let blocked = scoped_ncc_blocked(scan_mutation, evidence_mutation);
            let super::ScopedNccHistoryScanBlockedAuthority { source, evidence } = blocked;
            assert_scoped_ncc_retained_source(&source, 2, true);
            let super::ScopedNccHistoryScanFailureEvidence::Observation {
                stage,
                error,
                observation,
            } = evidence
            else {
                panic!("expected observation evidence");
            };
            assert_eq!(stage, expected_stage);
            assert_eq!(error.0, expected_error);

            let expected_start = source.partition.start_cursor().clone();
            let expected_end = source.partition.through_inclusive().clone();
            let expected_digest = source.partition.partition_digest().clone();
            assert_eq!(
                observation.rows.len(),
                if matches!(scan_mutation, ScopedNccScanMutation::MissingRow) {
                    1
                } else if matches!(scan_mutation, ScopedNccScanMutation::ExtraRow) {
                    3
                } else {
                    2
                }
            );
            if !matches!(scan_mutation, ScopedNccScanMutation::ForeignStartEndpoint) {
                assert_eq!(observation.from_exclusive, expected_start);
            }
            if !matches!(scan_mutation, ScopedNccScanMutation::ForeignEndpoint) {
                assert_eq!(observation.through_inclusive, expected_end);
            }
            if !matches!(scan_mutation, ScopedNccScanMutation::ForeignPartition) {
                assert_eq!(observation.partition_digest, expected_digest);
            }
            match scan_mutation {
                ScopedNccScanMutation::ForeignStartEndpoint => assert_eq!(
                    observation.from_exclusive,
                    serde_json::from_value(cursor("opaque-v-foreign", SHA_A)).unwrap()
                ),
                ScopedNccScanMutation::ForeignEndpoint => assert_eq!(
                    observation.through_inclusive,
                    serde_json::from_value(cursor("opaque-v-foreign", SHA_A)).unwrap()
                ),
                ScopedNccScanMutation::ForeignPartition => assert_eq!(
                    observation.partition_digest,
                    Sha256Digest::parse(SHA_A).unwrap()
                ),
                ScopedNccScanMutation::ReorderedRows => assert_eq!(
                    observation
                        .rows
                        .iter()
                        .map(|row| row.position)
                        .collect::<Vec<_>>(),
                    vec![2, 1]
                ),
                ScopedNccScanMutation::DuplicatePosition => assert_eq!(
                    observation
                        .rows
                        .iter()
                        .map(|row| row.position)
                        .collect::<Vec<_>>(),
                    vec![1, 1]
                ),
                ScopedNccScanMutation::ForeignCursor => assert_eq!(
                    observation.rows[1].cursor,
                    serde_json::from_value(cursor("opaque-v2", SHA_A)).unwrap()
                ),
                ScopedNccScanMutation::ForeignVersion => assert_eq!(
                    observation.rows[1].repository_version.as_str(),
                    "opaque-v-foreign"
                ),
                ScopedNccScanMutation::MissingNccFacts => {
                    assert!(observation.rows[1].facts.is_none())
                }
                ScopedNccScanMutation::FactsAtRoutinePosition => {
                    assert!(observation.rows[0].facts.is_some())
                }
                ScopedNccScanMutation::ForeignCapability => assert_eq!(
                    observation.rows[1]
                        .facts
                        .as_ref()
                        .expect("mutated NCC facts must be retained")
                        .atomic_commit_safety_capability_id
                        .as_str(),
                    UUID_B
                ),
                _ => {}
            }
        }
    }

    #[test]
    fn gate_b3_scoped_ncc_safety_and_evidence_mutations_retain_exact_ncc_fact_stage() {
        for (scan_mutation, evidence_mutation, expected_error) in [
            (
                ScopedNccScanMutation::RemovedReferenceEdge,
                ScopedNccEvidenceMutation::None,
                "scoped NCC observations do not derive every audited safety literal",
            ),
            (
                ScopedNccScanMutation::NoGenuineExpansion,
                ScopedNccEvidenceMutation::None,
                "scoped NCC observations do not derive every audited safety literal",
            ),
            (
                ScopedNccScanMutation::AddedEdgeSetMismatch,
                ScopedNccEvidenceMutation::None,
                "scoped NCC observations do not derive every audited safety literal",
            ),
            (
                ScopedNccScanMutation::IntegrationOverlap,
                ScopedNccEvidenceMutation::None,
                "scoped NCC observations do not derive every audited safety literal",
            ),
            (
                ScopedNccScanMutation::ChangedLockedOverlap,
                ScopedNccEvidenceMutation::None,
                "scoped NCC observations do not derive every audited safety literal",
            ),
            (
                ScopedNccScanMutation::ChangedReferrerMismatch,
                ScopedNccEvidenceMutation::None,
                "scoped NCC observations do not derive every audited safety literal",
            ),
            (
                ScopedNccScanMutation::RootChangedReferrer,
                ScopedNccEvidenceMutation::None,
                "scoped NCC observations do not derive every audited safety literal",
            ),
            (
                ScopedNccScanMutation::SupportGraphChanged,
                ScopedNccEvidenceMutation::None,
                "scoped NCC observations do not derive every audited safety literal",
            ),
            (
                ScopedNccScanMutation::ValidationInputsChanged,
                ScopedNccEvidenceMutation::None,
                "scoped NCC observations do not derive every audited safety literal",
            ),
            (
                ScopedNccScanMutation::RootStateChanged,
                ScopedNccEvidenceMutation::None,
                "scoped NCC observations do not derive every audited safety literal",
            ),
            (
                ScopedNccScanMutation::LockedStateChanged,
                ScopedNccEvidenceMutation::None,
                "scoped NCC observations do not derive every audited safety literal",
            ),
            (
                ScopedNccScanMutation::AbsentLockedState,
                ScopedNccEvidenceMutation::None,
                "scoped NCC observations do not derive every audited safety literal",
            ),
            (
                ScopedNccScanMutation::BlocksApprovedDeletion,
                ScopedNccEvidenceMutation::None,
                "scoped NCC observations do not derive every audited safety literal",
            ),
            (
                ScopedNccScanMutation::None,
                ScopedNccEvidenceMutation::RepositoryVersion,
                "scoped NCC facts differ from the audited evidence record",
            ),
            (
                ScopedNccScanMutation::None,
                ScopedNccEvidenceMutation::Capability,
                "scoped NCC facts differ from the audited evidence record",
            ),
            (
                ScopedNccScanMutation::None,
                ScopedNccEvidenceMutation::LockedTargetDigest,
                "scoped NCC facts differ from the audited evidence record",
            ),
            (
                ScopedNccScanMutation::None,
                ScopedNccEvidenceMutation::ChangedTargetDigest,
                "scoped NCC facts differ from the audited evidence record",
            ),
            (
                ScopedNccScanMutation::None,
                ScopedNccEvidenceMutation::BeforeClosureDigest,
                "scoped NCC facts differ from the audited evidence record",
            ),
            (
                ScopedNccScanMutation::None,
                ScopedNccEvidenceMutation::AfterClosureDigest,
                "scoped NCC facts differ from the audited evidence record",
            ),
            (
                ScopedNccScanMutation::None,
                ScopedNccEvidenceMutation::AddedEdgeDigest,
                "scoped NCC facts differ from the audited evidence record",
            ),
        ] {
            let blocked = scoped_ncc_blocked(scan_mutation, evidence_mutation);
            let super::ScopedNccHistoryScanBlockedAuthority { source, evidence } = blocked;
            assert_scoped_ncc_retained_source(&source, 2, true);
            match evidence {
                super::ScopedNccHistoryScanFailureEvidence::Observation {
                    stage,
                    error,
                    observation,
                } => {
                    assert_eq!(
                        stage,
                        super::ScopedNccHistoryScanObservationFailureStage::NccFactsMismatch
                    );
                    assert_eq!(error.0, expected_error);
                    assert_eq!(observation.rows.len(), source.partition.entry_count());
                    assert_eq!(observation.from_exclusive, *source.partition.start_cursor());
                    assert_eq!(
                        observation.through_inclusive,
                        *source.partition.through_inclusive()
                    );
                    assert_eq!(
                        observation.partition_digest,
                        *source.partition.partition_digest()
                    );
                    let mut expected_facts = scoped_ncc_safe_facts();
                    apply_scoped_ncc_safety_mutation(&mut expected_facts, scan_mutation);
                    assert_eq!(
                        observation.rows[1].facts.as_ref(),
                        Some(&expected_facts),
                        "the blocked observation must retain the exact NCC facts that failed validation"
                    );
                }
                evidence => panic!("expected NCC-fact observation evidence, got {evidence:?}"),
            }
        }
    }

    #[test]
    fn gate_b3_scoped_ncc_authority_is_non_clone_and_non_wire() {
        let _ = <super::ScopedNccHistoryScanAuthority as AmbiguousIfClone<_>>::marker;
        let _ = <super::ScopedNccHistoryScanAuthority as AmbiguousIfDeserializeOwned<_>>::marker;
        let _ = <super::ScopedNccHistoryScanAuthority as AmbiguousIfSerialize<_>>::marker;
    }

    #[test]
    fn gate_b3_scoped_ncc_happy_scan_covers_every_ordered_history_position() {
        let authority =
            run_scoped_ncc_scan(ScopedNccScanMutation::None, ScopedNccEvidenceMutation::None)
                .unwrap();
        assert_eq!(authority.history_entry_count(), 2);
        assert_eq!(authority.ncc_entry_count(), 1);
        assert_eq!(authority.partition().entry_count(), 2);
        assert_eq!(authority.expected_capability_id().as_str(), UUID_A);
        assert_eq!(
            authority.partition().partition_digest().as_str(),
            scoped_ncc_partition(&scoped_ncc_safe_facts(), ScopedNccEvidenceMutation::None)
                .partition_digest()
                .as_str()
        );
    }

    #[test]
    fn gate_b3_scoped_ncc_rejects_inexact_rows_and_foreign_scope() {
        for mutation in [
            ScopedNccScanMutation::MissingRow,
            ScopedNccScanMutation::ExtraRow,
            ScopedNccScanMutation::ReorderedRows,
            ScopedNccScanMutation::DuplicatePosition,
            ScopedNccScanMutation::FactsAtRoutinePosition,
            ScopedNccScanMutation::ForeignCursor,
            ScopedNccScanMutation::ForeignVersion,
            ScopedNccScanMutation::ForeignPartition,
            ScopedNccScanMutation::ForeignStartEndpoint,
            ScopedNccScanMutation::ForeignEndpoint,
            ScopedNccScanMutation::ForeignCapability,
        ] {
            assert!(run_scoped_ncc_scan(mutation, ScopedNccEvidenceMutation::None).is_err());
        }
    }

    #[test]
    fn gate_b3_scoped_ncc_rejects_equal_scalar_cross_scan_row_splice() {
        assert!(run_scoped_ncc_scan(
            ScopedNccScanMutation::CrossScanRowSplice,
            ScopedNccEvidenceMutation::None,
        )
        .is_err());
    }

    #[test]
    fn gate_b3_scoped_ncc_pointer_rejects_foreign_completion_capability() {
        assert!(run_scoped_ncc_scan(
            ScopedNccScanMutation::ForeignCompletionCapability,
            ScopedNccEvidenceMutation::None,
        )
        .is_err());
    }

    #[test]
    fn gate_b3_scoped_ncc_pointer_rejects_foreign_lease_witness() {
        assert!(run_scoped_ncc_scan(
            ScopedNccScanMutation::ForeignLeaseWitness,
            ScopedNccEvidenceMutation::None,
        )
        .is_err());
    }

    #[test]
    fn gate_b3_scoped_ncc_pointer_rejects_lease_binding_failure() {
        assert!(run_scoped_ncc_scan(
            ScopedNccScanMutation::LeaseBindsFalse,
            ScopedNccEvidenceMutation::None,
        )
        .is_err());
    }

    #[test]
    fn gate_b3_scoped_ncc_pointer_rejects_foreign_batch_witness() {
        assert!(run_scoped_ncc_scan(
            ScopedNccScanMutation::ForeignBatchWitness,
            ScopedNccEvidenceMutation::None,
        )
        .is_err());
    }

    #[test]
    fn gate_b3_scoped_ncc_derives_every_safety_literal_from_full_observations() {
        for mutation in [
            ScopedNccScanMutation::RemovedReferenceEdge,
            ScopedNccScanMutation::NoGenuineExpansion,
            ScopedNccScanMutation::AddedEdgeSetMismatch,
            ScopedNccScanMutation::IntegrationOverlap,
            ScopedNccScanMutation::SupportGraphChanged,
            ScopedNccScanMutation::ValidationInputsChanged,
            ScopedNccScanMutation::RootStateChanged,
            ScopedNccScanMutation::LockedStateChanged,
            ScopedNccScanMutation::AbsentLockedState,
            ScopedNccScanMutation::BlocksApprovedDeletion,
        ] {
            assert!(run_scoped_ncc_scan(mutation, ScopedNccEvidenceMutation::None).is_err());
        }
    }

    #[test]
    fn gate_b3_scoped_ncc_changed_and_locked_overlap_is_rejected_independently() {
        assert!(run_scoped_ncc_scan(
            ScopedNccScanMutation::ChangedLockedOverlap,
            ScopedNccEvidenceMutation::None,
        )
        .is_err());
    }

    #[test]
    fn gate_b3_scoped_ncc_changed_set_is_exact_added_edge_referrer_set() {
        assert!(run_scoped_ncc_scan(
            ScopedNccScanMutation::ChangedReferrerMismatch,
            ScopedNccEvidenceMutation::None,
        )
        .is_err());
    }

    #[test]
    fn gate_b3_scoped_ncc_rejects_configuration_root_as_changed_referrer() {
        assert!(run_scoped_ncc_scan(
            ScopedNccScanMutation::RootChangedReferrer,
            ScopedNccEvidenceMutation::None,
        )
        .is_err());
    }

    #[test]
    fn gate_b3_scoped_ncc_compares_every_derived_field_with_audited_evidence() {
        for mutation in [
            ScopedNccEvidenceMutation::RepositoryVersion,
            ScopedNccEvidenceMutation::Capability,
            ScopedNccEvidenceMutation::LockedTargetDigest,
            ScopedNccEvidenceMutation::ChangedTargetDigest,
            ScopedNccEvidenceMutation::BeforeClosureDigest,
            ScopedNccEvidenceMutation::AfterClosureDigest,
            ScopedNccEvidenceMutation::AddedEdgeDigest,
        ] {
            assert!(run_scoped_ncc_scan(ScopedNccScanMutation::None, mutation).is_err());
        }
    }

    #[test]
    fn gate_b3_scoped_ncc_set_requires_at_least_one_ncc_position() {
        let from = RepositoryHistoryCursor::new(
            RepositoryVersion::parse("opaque-v0").unwrap(),
            Sha256Digest::parse(SHA_A).unwrap(),
        );
        let partition = super::repository_history_partition_fixture_test_only(
            from,
            vec![(
                RepositoryVersion::parse("opaque-v1").unwrap(),
                super::RepositoryHistoryPartitionClassification::UnrelatedRoutine,
            )],
            "history-order-v1",
            UUID_A,
        )
        .unwrap();
        let mut port = FixtureScopedNccPort {
            facts: scoped_ncc_safe_facts(),
            mutation: ScopedNccScanMutation::None,
            calls: 0,
            capture: None,
        };
        assert!(super::ScopedNccHistoryScanAuthority::resolve(
            partition,
            CapabilityRowId::parse(UUID_A).unwrap(),
            &mut port,
        )
        .is_err());
        assert_eq!(port.calls, 0);
    }

    #[test]
    fn semantic_delta_projection_matrix_binds_all_six_required_nullable_slots() {
        let slots = [
            "rootDeltaDigest",
            "contentDeltaDigest",
            "classificationDigest",
            "externalSupportDisjointnessDigest",
            "correctiveInstructionDigest",
            "nonConflictingConcurrentEvidenceDigest",
        ];
        let registry = EvidenceSourceRegistry::task8().unwrap();

        let (routine_partition, routine_ref, routine_evidence) = routine_partition_fixture();
        let routine_record = super::RepositorySemanticDeltaDigestRecord {
            repository_version: super::RepositoryVersion::parse("opaque-v1").unwrap(),
            partition_classification:
                super::RepositoryHistoryPartitionClassification::UnrelatedRoutine,
            root_delta_digest: super::RequiredNullable::value(Sha256Digest::parse(SHA_A).unwrap()),
            content_delta_digest: super::RequiredNullable::value(
                Sha256Digest::parse(SHA_B).unwrap(),
            ),
            classification_digest: super::RequiredNullable::value(
                routine_evidence.classification_digest.clone(),
            ),
            external_support_disjointness_digest: super::RequiredNullable::null(),
            corrective_instruction_digest: super::RequiredNullable::null(),
            non_conflicting_concurrent_evidence_digest: super::RequiredNullable::null(),
        };
        let routine_record_json = serde_json::to_value(&routine_record).unwrap();
        assert_eq!(
            routine_partition["entries"][0]["semanticDeltaDigest"],
            serde_json::to_value(super::canonical_contract_digest(&routine_record, None).unwrap())
                .unwrap()
        );
        for slot in slots {
            let mut omitted = routine_record_json.clone();
            omitted.as_object_mut().unwrap().remove(slot);
            assert!(
                serde_json::from_value::<super::RepositorySemanticDeltaDigestRecord>(omitted)
                    .is_err()
            );

            let mut substituted = routine_record_json.clone();
            substituted[slot] = if substituted[slot].is_null() {
                json!(SHA_A)
            } else {
                Value::Null
            };
            let substituted_record =
                serde_json::from_value::<super::RepositorySemanticDeltaDigestRecord>(substituted)
                    .unwrap();
            let substituted_digest =
                super::canonical_contract_digest(&substituted_record, None).unwrap();
            let mut partition = routine_partition.clone();
            partition["entries"][0]["semanticDeltaDigest"] =
                serde_json::to_value(substituted_digest).unwrap();
            recalculate_partition_digest(&mut partition);
            assert!(validate_routine_fixture(
                partition,
                routine_candidate(&registry, routine_ref.clone()),
                routine_order(),
                serde_json_canonicalizer::to_vec(&routine_evidence).unwrap(),
                &routine_ref,
            )
            .is_err());
        }

        let (concurrent_partition, concurrent_ref, concurrent_evidence) =
            non_conflicting_partition_fixture();
        let concurrent_record = super::RepositorySemanticDeltaDigestRecord {
            repository_version: super::RepositoryVersion::parse("opaque-v1").unwrap(),
            partition_classification:
                super::RepositoryHistoryPartitionClassification::NonConflictingConcurrent,
            root_delta_digest: super::RequiredNullable::null(),
            content_delta_digest: super::RequiredNullable::null(),
            classification_digest: super::RequiredNullable::null(),
            external_support_disjointness_digest: super::RequiredNullable::null(),
            corrective_instruction_digest: super::RequiredNullable::null(),
            non_conflicting_concurrent_evidence_digest: super::RequiredNullable::value(
                concurrent_evidence.evidence_digest.clone(),
            ),
        };
        let concurrent_record_json = serde_json::to_value(&concurrent_record).unwrap();
        assert_eq!(
            concurrent_partition["entries"][0]["semanticDeltaDigest"],
            serde_json::to_value(
                super::canonical_contract_digest(&concurrent_record, None).unwrap()
            )
            .unwrap()
        );
        for slot in slots {
            let mut omitted = concurrent_record_json.clone();
            omitted.as_object_mut().unwrap().remove(slot);
            assert!(
                serde_json::from_value::<super::RepositorySemanticDeltaDigestRecord>(omitted)
                    .is_err()
            );

            let mut substituted = concurrent_record_json.clone();
            substituted[slot] = if substituted[slot].is_null() {
                json!(SHA_A)
            } else {
                Value::Null
            };
            let substituted_record =
                serde_json::from_value::<super::RepositorySemanticDeltaDigestRecord>(substituted)
                    .unwrap();
            let substituted_digest =
                super::canonical_contract_digest(&substituted_record, None).unwrap();
            let mut partition = concurrent_partition.clone();
            partition["entries"][0]["semanticDeltaDigest"] =
                serde_json::to_value(substituted_digest).unwrap();
            recalculate_partition_digest(&mut partition);
            assert!(validate_non_conflicting_fixture(
                partition,
                &concurrent_ref,
                &concurrent_evidence,
            )
            .is_err());
        }
    }
}
