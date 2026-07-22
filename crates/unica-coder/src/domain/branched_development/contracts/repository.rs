use super::instructions::{
    decode_historical_support_conflict_instruction, SupportConflictInstruction,
    SupportCorrectiveInstruction,
};
use super::scalars::{
    NormalizedUtcInstant, RepositoryIdentityComponent, RepositoryTargetDisplay, RepositoryUsername,
    RepositoryVersion, RequiredNullable,
};
use super::schema::{audit_json_schema, one_of_schema};
use super::support::{
    ExternalSupportOwnershipEvidence, SupportHistoryOrderAuthority,
    SupportObservationCorrectiveProjection, SupportPrerequisiteVersionObservation,
    SupportPrerequisiteVersionObservationDigestRecord,
};
use crate::domain::branched_development::canonical_json::{
    canonical_contract_digest, contract_digest_record_sealed, ContractDigestRecord,
};
use crate::domain::branched_development::{
    CapabilityRowId, MetadataObjectId, Sha256Digest, UnicaId,
};
use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::HashSet;
use std::fmt;

pub(crate) mod lifecycle;
pub(crate) mod update;

#[allow(unused_imports)]
pub(crate) use lifecycle::{
    ClassifiedDeferredRepositoryAdvance, CoverageUnknownDeferredRepositoryAdvance,
    DeferredRepositoryAdvance, DeferredRepositoryAdvanceClassification,
    DeferredRepositoryAdvanceClassificationAuthority, DeferredRepositoryAdvanceConsumptionReceipt,
    DeferredRepositoryAdvanceMissingEvidenceAuthority, OriginalCleanRefreshProof,
    OriginalCleanRefreshScanAuthority, PostMergeHistoryGuardAuthority,
    PostMergeHistoryGuardEvidence, SupportGateHistoryEvidence,
    SupportGateRelevantBaselineAuthority, UnclassifiedDeferredRepositoryAdvance,
    UnvalidatedDeferredRepositoryAdvance, UnvalidatedOriginalCleanRefreshProof,
};
#[allow(unused_imports)]
pub(crate) use update::{
    SelectiveRepositoryUpdateExecutionAuthority, SelectiveRepositoryUpdatePlan,
    SelectiveRepositoryUpdatePlanAuthority, SelectiveRepositoryUpdateProof,
    SelectiveRepositoryUpdateScope,
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
struct UncheckedNonConflictingConcurrentEvidence {
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

impl NonConflictingConcurrentEvidence {
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

impl<'de> Deserialize<'de> for NonConflictingConcurrentEvidence {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let unchecked = UncheckedNonConflictingConcurrentEvidence::deserialize(deserializer)?;
        let value = Self {
            repository_version: unchecked.repository_version,
            reason: unchecked.reason,
            atomic_commit_safety_capability_id: unchecked.atomic_commit_safety_capability_id,
            locked_target_set_digest: unchecked.locked_target_set_digest,
            changed_object_set_digest: unchecked.changed_object_set_digest,
            before_reference_closure_digest: unchecked.before_reference_closure_digest,
            after_reference_closure_digest: unchecked.after_reference_closure_digest,
            added_reference_edge_set_digest: unchecked.added_reference_edge_set_digest,
            closure_delta_only_adds_non_blocking_references: unchecked
                .closure_delta_only_adds_non_blocking_references,
            disjoint_from_integration_content: unchecked.disjoint_from_integration_content,
            support_graph_unchanged: unchecked.support_graph_unchanged,
            validation_inputs_unaffected: unchecked.validation_inputs_unaffected,
            root_unchanged: unchecked.root_unchanged,
            locked_targets_unchanged: unchecked.locked_targets_unchanged,
            blocks_approved_deletion: unchecked.blocks_approved_deletion,
            evidence_digest: unchecked.evidence_digest,
        };
        value.validate_digest().map_err(D::Error::custom)?;
        Ok(value)
    }
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
            let evidence = serde_json::from_value::<NonConflictingConcurrentEvidence>(value)
                .map_err(|_| RepositoryContractError("concurrent evidence typed decode failed"))?;
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
    source_index_proofs: Vec<EvidenceSourceIndexProof>,
    order_evidence: Option<RepositoryHistoryOrderEvidence>,
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
        let index_proof = &self.source_index_proofs[0];
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
            source_index_proofs.push(proof);
        }

        Ok(ValidatedRepositoryHistoryPartition {
            wire,
            source_index_proofs,
            order_evidence: Some(order_evidence),
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
                    ) = if let Some(projection) = observation.task8_mapping_projection() {
                        (
                            projection.partition_classification(),
                            projection.root_delta_digest(),
                            projection.content_delta_digest(),
                            projection.classification_digest(),
                            projection.external_support_disjointness_digest(),
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ObjectTargetIdentity {
    target_kind: DevelopmentObjectKind,
    object_id: MetadataObjectId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum RepositoryTargetIdentity {
    ConfigurationRoot(RootTargetIdentity),
    DevelopmentObject(ObjectTargetIdentity),
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

fn parse_digest(value: &str) -> Result<Sha256Digest, RepositoryContractError> {
    Sha256Digest::parse(value).map_err(|_| RepositoryContractError("invalid SHA-256 digest"))
}

#[cfg(test)]
mod tests {
    use super::{
        AcquiredRepositoryUpdateLockTargets, CanonicalEmptyDeltaDigest, EvidenceKind,
        EvidenceSourceIndex, EvidenceSourceIndexCandidate, EvidenceSourceIndexCandidateRow,
        EvidenceSourceRegistry, FrozenSupportConflictInstructionSourceAuthority,
        FrozenSupportCorrectiveInstructionSourceAuthority, NonConflictingConcurrentEvidence,
        ReleasedRepositoryUpdateLockTargets, RepositoryActorIdentity, RepositoryHistoryCursor,
        RepositoryHistoryEvidenceBytesResolver, RepositoryHistoryOrderEvidence,
        RepositoryHistoryOrderResolver, RepositoryHistoryPartitionResolver,
        RepositoryHistorySourceEvidenceRef, RepositoryOwnerIdentity, RepositoryPlannedChanges,
        RepositoryTargetIdentity, RepositoryTargetKind, RepositoryTargetStates,
        RepositoryUpdateLockReason, RepositoryUpdateLockReasons, RepositoryUpdateLockTargets,
        RoutineRepositoryVersionClassificationEvidence, SupportCorrectiveEvidenceResolver,
        UnvalidatedRepositoryHistoryPartition,
    };
    use crate::domain::branched_development::contracts::instructions::{
        SupportConflictInstruction, SupportCorrectiveInstruction,
        SupportCorrectiveInstructionAuthority, SupportRecoveryTransition,
    };
    use crate::domain::branched_development::contracts::scalars::{
        Diagnostic, RepositoryTargetDisplay, RepositoryUsername, RepositoryVersion,
        RequiredNullable,
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
    use crate::domain::branched_development::{Sha256Digest, SupportLayerId, UnicaId};
    use schemars::schema_for;
    use serde_json::{json, Value};
    use sha2::{Digest, Sha256};
    use std::cmp::Ordering;
    use std::collections::BTreeMap;

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

    #[test]
    fn capability_derived_authority_types_have_no_deserialize_backdoor() {
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
        serde_json::from_value::<NonConflictingConcurrentEvidence>(value.clone()).unwrap();

        let mut wrong_literal = value.as_object().unwrap().clone();
        wrong_literal.insert("rootUnchanged".into(), json!(false));
        assert!(
            serde_json::from_value::<NonConflictingConcurrentEvidence>(Value::Object(
                wrong_literal
            ))
            .is_err()
        );
        let mut wrong_digest = value.as_object().unwrap().clone();
        wrong_digest.insert("evidenceDigest".into(), json!(SHA_A));
        assert!(
            serde_json::from_value::<NonConflictingConcurrentEvidence>(Value::Object(wrong_digest))
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
        let index_proof_json = serde_json::to_value(&validated.source_index_proofs[0]).unwrap();
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
        wrong_registry.source_index_proofs[0].registry_digest = Sha256Digest::parse(SHA_A).unwrap();
        assert!(
            super::ValidatedSupportObservationHistoryEntry::from_validated_partition(
                &wrong_registry,
                &successor,
            )
            .is_err()
        );

        let mut wrong_selected_ref = validated.clone();
        match &mut wrong_selected_ref.source_index_proofs[0].availability.0[1] {
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
