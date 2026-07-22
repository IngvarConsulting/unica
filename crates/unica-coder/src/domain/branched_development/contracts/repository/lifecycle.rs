use super::super::scalars::RepositoryVersion;
use super::super::schema::one_of_schema;
use super::super::status::ExistingTaskStatusData;
use super::super::SupportMissingEvidenceKind;
use super::{
    FalseLiteral, RepositoryContractError, RepositoryHistoryCoverageGapEvidence,
    RepositoryHistoryCursor, RepositoryHistoryImmediateSuccessorEvidence,
    RepositoryHistoryPartitionClassification, TrueLiteral, UnvalidatedRepositoryHistoryPartition,
    ValidatedRepositoryHistoryPartition,
};
#[cfg(test)]
use super::{RepositoryHistoryPartitionDigestRecord, UnvalidatedRepositoryHistoryEntries};
use crate::domain::branched_development::canonical_json::{
    canonical_contract_digest, contract_digest_record_sealed, ContractDigestRecord,
};
use crate::domain::branched_development::{CapabilityRowId, Sha256Digest, TaskPhase, UnicaId};
use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use std::borrow::Cow;

const MAX_SUPPORT_MISSING_EVIDENCE_KINDS: usize = SupportMissingEvidenceKind::ALL.len();

macro_rules! wire_literal {
    ($name:ident, $wire:literal) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
        enum $name {
            #[serde(rename = $wire)]
            Value,
        }
    };
}

wire_literal!(ClassifiedState, "classified");
wire_literal!(UnclassifiedState, "unclassified");
wire_literal!(CoverageUnknownState, "coverageUnknown");
wire_literal!(RoutineUpdateMode, "routine");

/// Non-wire proof of the live task phase used by one routine update.
///
/// Production construction accepts only the validated current status leaf;
/// callers cannot assert either the origin or result phase independently.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RoutineUpdatePhaseAuthority {
    origin_phase: TaskPhase,
}

impl RoutineUpdatePhaseAuthority {
    pub(crate) fn from_live_status(
        status: &ExistingTaskStatusData,
    ) -> Result<Self, RepositoryContractError> {
        Self::from_phase(status.phase())
    }

    fn from_phase(origin_phase: TaskPhase) -> Result<Self, RepositoryContractError> {
        if !matches!(
            origin_phase,
            TaskPhase::BaselineReady
                | TaskPhase::Developing
                | TaskPhase::LocalVerified
                | TaskPhase::SynchronizationPrepared
                | TaskPhase::SynchronizationConflicts
                | TaskPhase::Synchronized
                | TaskPhase::IntegrationPlanned
                | TaskPhase::BlockedByForeignLock
                | TaskPhase::StaleRelevantBaseline
                | TaskPhase::LockPlanExpansionRequired
                | TaskPhase::StaleSupportPreflight
                | TaskPhase::UnexpectedDelta
                | TaskPhase::ValidationFailed
                | TaskPhase::AbandonmentReady
        ) {
            return Err(RepositoryContractError(
                "live task phase does not allow a routine repository update",
            ));
        }
        Ok(Self { origin_phase })
    }

    #[cfg(test)]
    pub(crate) fn routine_test_only(
        origin_phase: TaskPhase,
    ) -> Result<Self, RepositoryContractError> {
        Self::from_phase(origin_phase)
    }

    /// Adversarial cross-contract fixture. Production has no equivalent raw
    /// phase mint; the projection must still reject this forged authority.
    #[cfg(test)]
    pub(crate) const fn foreign_test_only(origin_phase: TaskPhase) -> Self {
        Self { origin_phase }
    }

    pub(crate) fn into_resulting_phase(
        self,
        contains_relevant_advance: bool,
    ) -> Result<TaskPhase, RepositoryContractError> {
        Self::from_phase(self.origin_phase)?;
        let resulting_phase = if self.origin_phase == TaskPhase::AbandonmentReady {
            TaskPhase::AbandonmentReady
        } else if contains_relevant_advance {
            TaskPhase::LocalVerified
        } else {
            self.origin_phase
        };
        Ok(resulting_phase)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) enum DeferredRepositoryAdvanceClassification {
    AuthorizedSupport,
    Invalid,
    Corrective,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct DeferredMissingEvidenceKinds(Vec<SupportMissingEvidenceKind>);

impl DeferredMissingEvidenceKinds {
    fn new(values: Vec<SupportMissingEvidenceKind>) -> Result<Self, RepositoryContractError> {
        if values.is_empty() || values.len() > MAX_SUPPORT_MISSING_EVIDENCE_KINDS {
            return Err(RepositoryContractError(
                "deferred missing-evidence kinds must be non-empty and within the shared vocabulary bound",
            ));
        }
        if values.contains(&SupportMissingEvidenceKind::RepositoryHistoryCoverageIncomplete) {
            return Err(RepositoryContractError(
                "known-successor evidence gaps cannot claim unknown history coverage",
            ));
        }
        if values.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(RepositoryContractError(
                "deferred missing-evidence kinds must be unique and canonical",
            ));
        }
        Ok(Self(values))
    }

    pub(crate) fn as_slice(&self) -> &[SupportMissingEvidenceKind] {
        &self.0
    }
}

impl<'de> Deserialize<'de> for DeferredMissingEvidenceKinds {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(Vec::<SupportMissingEvidenceKind>::deserialize(
            deserializer,
        )?)
        .map_err(D::Error::custom)
    }
}

impl JsonSchema for DeferredMissingEvidenceKinds {
    fn schema_name() -> Cow<'static, str> {
        "DeferredRepositoryAdvanceMissingEvidenceKinds".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        let allowed_wires: Vec<_> = SupportMissingEvidenceKind::ALL
            .iter()
            .copied()
            .filter(|kind| *kind != SupportMissingEvidenceKind::RepositoryHistoryCoverageIncomplete)
            .map(|kind| kind.as_str())
            .collect();
        json_schema!({
            "type": "array",
            "minItems": 1,
            "maxItems": MAX_SUPPORT_MISSING_EVIDENCE_KINDS,
            "uniqueItems": true,
            "items": {
                "type": "string",
                "enum": allowed_wires,
            },
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct CoverageUnknownMissingEvidenceKinds([SupportMissingEvidenceKind; 1]);

impl CoverageUnknownMissingEvidenceKinds {
    const fn canonical() -> Self {
        Self([SupportMissingEvidenceKind::RepositoryHistoryCoverageIncomplete])
    }
}

impl<'de> Deserialize<'de> for CoverageUnknownMissingEvidenceKinds {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let values = <[SupportMissingEvidenceKind; 1]>::deserialize(deserializer)?;
        (values[0] == SupportMissingEvidenceKind::RepositoryHistoryCoverageIncomplete)
            .then_some(Self(values))
            .ok_or_else(|| {
                D::Error::custom(
                    "coverage-unknown evidence kinds must be the exact coverage-incomplete tuple",
                )
            })
    }
}

impl JsonSchema for CoverageUnknownMissingEvidenceKinds {
    fn schema_name() -> Cow<'static, str> {
        "CoverageUnknownMissingEvidenceKinds".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        let coverage_incomplete =
            SupportMissingEvidenceKind::RepositoryHistoryCoverageIncomplete.as_str();
        json_schema!({
            "type": "array",
            "prefixItems": [{
                "type": "string",
                "const": coverage_incomplete,
            }],
            "items": false,
            "minItems": 1,
            "maxItems": 1,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ClassifiedDeferredRepositoryAdvanceObservationDigestRecord {
    state: ClassifiedState,
    from_cursor: RepositoryHistoryCursor,
    first_observed_version: RepositoryVersion,
    classification: DeferredRepositoryAdvanceClassification,
    semantic_delta_digest: Sha256Digest,
    required_next_mode: RoutineUpdateMode,
}

impl contract_digest_record_sealed::Sealed
    for ClassifiedDeferredRepositoryAdvanceObservationDigestRecord
{
}
impl ContractDigestRecord for ClassifiedDeferredRepositoryAdvanceObservationDigestRecord {}

/// Capability-backed semantic classification for the immediate successor.
///
/// Task 7 deliberately exposes no raw production constructor. Task 8/9 must add
/// exact typed resolver factories here once their source evidence mappings exist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeferredRepositoryAdvanceClassificationAuthority {
    successor: RepositoryHistoryImmediateSuccessorEvidence,
    classification: DeferredRepositoryAdvanceClassification,
    semantic_delta_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ClassifiedDeferredRepositoryAdvance {
    state: ClassifiedState,
    from_cursor: RepositoryHistoryCursor,
    first_observed_version: RepositoryVersion,
    classification: DeferredRepositoryAdvanceClassification,
    semantic_delta_digest: Sha256Digest,
    required_next_mode: RoutineUpdateMode,
    observation_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct UnvalidatedClassifiedDeferredRepositoryAdvance {
    state: ClassifiedState,
    from_cursor: RepositoryHistoryCursor,
    first_observed_version: RepositoryVersion,
    classification: DeferredRepositoryAdvanceClassification,
    semantic_delta_digest: Sha256Digest,
    required_next_mode: RoutineUpdateMode,
    observation_digest: Sha256Digest,
}

impl ClassifiedDeferredRepositoryAdvance {
    pub(crate) fn new(
        authority: &DeferredRepositoryAdvanceClassificationAuthority,
    ) -> Result<Self, RepositoryContractError> {
        let record = ClassifiedDeferredRepositoryAdvanceObservationDigestRecord {
            state: ClassifiedState::Value,
            from_cursor: authority.successor.anchor_cursor().clone(),
            first_observed_version: authority.successor.first_observed_version().clone(),
            classification: authority.classification,
            semantic_delta_digest: authority.semantic_delta_digest.clone(),
            required_next_mode: RoutineUpdateMode::Value,
        };
        let observation_digest = contract_digest(
            &record,
            "classified deferred-advance observation digest failed",
        )?;
        Ok(Self::from_record(record, observation_digest))
    }

    pub(crate) fn from_wire(
        wire: UnvalidatedClassifiedDeferredRepositoryAdvance,
        authority: &DeferredRepositoryAdvanceClassificationAuthority,
    ) -> Result<Self, RepositoryContractError> {
        if &wire.from_cursor != authority.successor.anchor_cursor()
            || &wire.first_observed_version != authority.successor.first_observed_version()
            || wire.classification != authority.classification
            || wire.semantic_delta_digest != authority.semantic_delta_digest
        {
            return Err(RepositoryContractError(
                "classified deferred advance disagrees with its semantic authority",
            ));
        }
        let value = Self {
            state: wire.state,
            from_cursor: wire.from_cursor,
            first_observed_version: wire.first_observed_version,
            classification: wire.classification,
            semantic_delta_digest: wire.semantic_delta_digest,
            required_next_mode: wire.required_next_mode,
            observation_digest: wire.observation_digest,
        };
        value.validate_digest()?;
        Ok(value)
    }

    fn from_record(
        record: ClassifiedDeferredRepositoryAdvanceObservationDigestRecord,
        observation_digest: Sha256Digest,
    ) -> Self {
        Self {
            state: record.state,
            from_cursor: record.from_cursor,
            first_observed_version: record.first_observed_version,
            classification: record.classification,
            semantic_delta_digest: record.semantic_delta_digest,
            required_next_mode: record.required_next_mode,
            observation_digest,
        }
    }

    fn digest_record(&self) -> ClassifiedDeferredRepositoryAdvanceObservationDigestRecord {
        ClassifiedDeferredRepositoryAdvanceObservationDigestRecord {
            state: self.state,
            from_cursor: self.from_cursor.clone(),
            first_observed_version: self.first_observed_version.clone(),
            classification: self.classification,
            semantic_delta_digest: self.semantic_delta_digest.clone(),
            required_next_mode: self.required_next_mode,
        }
    }

    fn validate_digest(&self) -> Result<(), RepositoryContractError> {
        validate_digest(
            &self.digest_record(),
            &self.observation_digest,
            "classified deferred-advance observation digest failed",
            "classified deferred-advance observation digest mismatch",
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UnclassifiedDeferredRepositoryAdvanceObservationDigestRecord {
    state: UnclassifiedState,
    from_cursor: RepositoryHistoryCursor,
    first_observed_version: RepositoryVersion,
    missing_evidence_kinds: DeferredMissingEvidenceKinds,
    required_next_mode: RoutineUpdateMode,
}

impl contract_digest_record_sealed::Sealed
    for UnclassifiedDeferredRepositoryAdvanceObservationDigestRecord
{
}
impl ContractDigestRecord for UnclassifiedDeferredRepositoryAdvanceObservationDigestRecord {}

/// Capability-backed missing-evidence result for the immediate successor.
///
/// Task 7 deliberately exposes no raw production constructor. Task 8/9 must add
/// exact typed resolver factories here once their evidence-gap producers exist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeferredRepositoryAdvanceMissingEvidenceAuthority {
    successor: RepositoryHistoryImmediateSuccessorEvidence,
    missing_evidence_kinds: DeferredMissingEvidenceKinds,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct UnclassifiedDeferredRepositoryAdvance {
    state: UnclassifiedState,
    from_cursor: RepositoryHistoryCursor,
    first_observed_version: RepositoryVersion,
    missing_evidence_kinds: DeferredMissingEvidenceKinds,
    required_next_mode: RoutineUpdateMode,
    observation_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct UnvalidatedUnclassifiedDeferredRepositoryAdvance {
    state: UnclassifiedState,
    from_cursor: RepositoryHistoryCursor,
    first_observed_version: RepositoryVersion,
    missing_evidence_kinds: DeferredMissingEvidenceKinds,
    required_next_mode: RoutineUpdateMode,
    observation_digest: Sha256Digest,
}

impl UnclassifiedDeferredRepositoryAdvance {
    pub(crate) fn new(
        authority: &DeferredRepositoryAdvanceMissingEvidenceAuthority,
    ) -> Result<Self, RepositoryContractError> {
        let record = UnclassifiedDeferredRepositoryAdvanceObservationDigestRecord {
            state: UnclassifiedState::Value,
            from_cursor: authority.successor.anchor_cursor().clone(),
            first_observed_version: authority.successor.first_observed_version().clone(),
            missing_evidence_kinds: authority.missing_evidence_kinds.clone(),
            required_next_mode: RoutineUpdateMode::Value,
        };
        let observation_digest = contract_digest(
            &record,
            "unclassified deferred-advance observation digest failed",
        )?;
        Ok(Self::from_record(record, observation_digest))
    }

    pub(crate) fn from_wire(
        wire: UnvalidatedUnclassifiedDeferredRepositoryAdvance,
        authority: &DeferredRepositoryAdvanceMissingEvidenceAuthority,
    ) -> Result<Self, RepositoryContractError> {
        if &wire.from_cursor != authority.successor.anchor_cursor()
            || &wire.first_observed_version != authority.successor.first_observed_version()
            || wire.missing_evidence_kinds != authority.missing_evidence_kinds
        {
            return Err(RepositoryContractError(
                "unclassified deferred advance disagrees with its missing-evidence authority",
            ));
        }
        let value = Self {
            state: wire.state,
            from_cursor: wire.from_cursor,
            first_observed_version: wire.first_observed_version,
            missing_evidence_kinds: wire.missing_evidence_kinds,
            required_next_mode: wire.required_next_mode,
            observation_digest: wire.observation_digest,
        };
        value.validate_digest()?;
        Ok(value)
    }

    fn from_record(
        record: UnclassifiedDeferredRepositoryAdvanceObservationDigestRecord,
        observation_digest: Sha256Digest,
    ) -> Self {
        Self {
            state: record.state,
            from_cursor: record.from_cursor,
            first_observed_version: record.first_observed_version,
            missing_evidence_kinds: record.missing_evidence_kinds,
            required_next_mode: record.required_next_mode,
            observation_digest,
        }
    }

    fn digest_record(&self) -> UnclassifiedDeferredRepositoryAdvanceObservationDigestRecord {
        UnclassifiedDeferredRepositoryAdvanceObservationDigestRecord {
            state: self.state,
            from_cursor: self.from_cursor.clone(),
            first_observed_version: self.first_observed_version.clone(),
            missing_evidence_kinds: self.missing_evidence_kinds.clone(),
            required_next_mode: self.required_next_mode,
        }
    }

    fn validate_digest(&self) -> Result<(), RepositoryContractError> {
        validate_digest(
            &self.digest_record(),
            &self.observation_digest,
            "unclassified deferred-advance observation digest failed",
            "unclassified deferred-advance observation digest mismatch",
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CoverageUnknownDeferredRepositoryAdvanceObservationDigestRecord {
    state: CoverageUnknownState,
    from_cursor: RepositoryHistoryCursor,
    missing_evidence_kinds: CoverageUnknownMissingEvidenceKinds,
    required_next_mode: RoutineUpdateMode,
}

impl contract_digest_record_sealed::Sealed
    for CoverageUnknownDeferredRepositoryAdvanceObservationDigestRecord
{
}
impl ContractDigestRecord for CoverageUnknownDeferredRepositoryAdvanceObservationDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CoverageUnknownDeferredRepositoryAdvance {
    state: CoverageUnknownState,
    from_cursor: RepositoryHistoryCursor,
    missing_evidence_kinds: CoverageUnknownMissingEvidenceKinds,
    required_next_mode: RoutineUpdateMode,
    observation_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct UnvalidatedCoverageUnknownDeferredRepositoryAdvance {
    state: CoverageUnknownState,
    from_cursor: RepositoryHistoryCursor,
    missing_evidence_kinds: CoverageUnknownMissingEvidenceKinds,
    required_next_mode: RoutineUpdateMode,
    observation_digest: Sha256Digest,
}

impl CoverageUnknownDeferredRepositoryAdvance {
    pub(crate) fn new(
        coverage_gap: &RepositoryHistoryCoverageGapEvidence,
    ) -> Result<Self, RepositoryContractError> {
        let record = CoverageUnknownDeferredRepositoryAdvanceObservationDigestRecord {
            state: CoverageUnknownState::Value,
            from_cursor: coverage_gap.anchor_cursor().clone(),
            missing_evidence_kinds: CoverageUnknownMissingEvidenceKinds::canonical(),
            required_next_mode: RoutineUpdateMode::Value,
        };
        let observation_digest = contract_digest(
            &record,
            "coverage-unknown deferred-advance observation digest failed",
        )?;
        Ok(Self::from_record(record, observation_digest))
    }

    pub(crate) fn from_wire(
        wire: UnvalidatedCoverageUnknownDeferredRepositoryAdvance,
        coverage_gap: &RepositoryHistoryCoverageGapEvidence,
    ) -> Result<Self, RepositoryContractError> {
        if &wire.from_cursor != coverage_gap.anchor_cursor() {
            return Err(RepositoryContractError(
                "coverage-unknown deferred advance disagrees with coverage-gap evidence",
            ));
        }
        let value = Self {
            state: wire.state,
            from_cursor: wire.from_cursor,
            missing_evidence_kinds: wire.missing_evidence_kinds,
            required_next_mode: wire.required_next_mode,
            observation_digest: wire.observation_digest,
        };
        value.validate_digest()?;
        Ok(value)
    }

    fn from_record(
        record: CoverageUnknownDeferredRepositoryAdvanceObservationDigestRecord,
        observation_digest: Sha256Digest,
    ) -> Self {
        Self {
            state: record.state,
            from_cursor: record.from_cursor,
            missing_evidence_kinds: record.missing_evidence_kinds,
            required_next_mode: record.required_next_mode,
            observation_digest,
        }
    }

    fn digest_record(&self) -> CoverageUnknownDeferredRepositoryAdvanceObservationDigestRecord {
        CoverageUnknownDeferredRepositoryAdvanceObservationDigestRecord {
            state: self.state,
            from_cursor: self.from_cursor.clone(),
            missing_evidence_kinds: self.missing_evidence_kinds.clone(),
            required_next_mode: self.required_next_mode,
        }
    }

    fn validate_digest(&self) -> Result<(), RepositoryContractError> {
        validate_digest(
            &self.digest_record(),
            &self.observation_digest,
            "coverage-unknown deferred-advance observation digest failed",
            "coverage-unknown deferred-advance observation digest mismatch",
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub(crate) enum DeferredRepositoryAdvance {
    Classified(ClassifiedDeferredRepositoryAdvance),
    Unclassified(UnclassifiedDeferredRepositoryAdvance),
    CoverageUnknown(CoverageUnknownDeferredRepositoryAdvance),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum UnvalidatedDeferredRepositoryAdvance {
    Classified(UnvalidatedClassifiedDeferredRepositoryAdvance),
    Unclassified(UnvalidatedUnclassifiedDeferredRepositoryAdvance),
    CoverageUnknown(UnvalidatedCoverageUnknownDeferredRepositoryAdvance),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DeferredRepositoryAdvanceResolutionDigestRecord {
    advance_observation_digest: Sha256Digest,
    resolved_history_partition_digest: Sha256Digest,
    first_resolved_version: RepositoryVersion,
    first_resolved_classification: RepositoryHistoryPartitionClassification,
    first_resolved_semantic_delta_digest: Sha256Digest,
    resulting_phase: TaskPhase,
}

impl contract_digest_record_sealed::Sealed for DeferredRepositoryAdvanceResolutionDigestRecord {}
impl ContractDigestRecord for DeferredRepositoryAdvanceResolutionDigestRecord {}

impl JsonSchema for UnvalidatedDeferredRepositoryAdvance {
    fn schema_name() -> Cow<'static, str> {
        "DeferredRepositoryAdvance".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<UnvalidatedClassifiedDeferredRepositoryAdvance>(),
            generator.subschema_for::<UnvalidatedUnclassifiedDeferredRepositoryAdvance>(),
            generator.subschema_for::<UnvalidatedCoverageUnknownDeferredRepositoryAdvance>(),
        ])
    }
}

impl DeferredRepositoryAdvance {
    pub(crate) fn anchor_cursor(&self) -> &RepositoryHistoryCursor {
        match self {
            Self::Classified(value) => &value.from_cursor,
            Self::Unclassified(value) => &value.from_cursor,
            Self::CoverageUnknown(value) => &value.from_cursor,
        }
    }

    pub(crate) fn observation_digest(&self) -> &Sha256Digest {
        match self {
            Self::Classified(value) => &value.observation_digest,
            Self::Unclassified(value) => &value.observation_digest,
            Self::CoverageUnknown(value) => &value.observation_digest,
        }
    }

    /// Known immediate successor retained outside the completed terminal
    /// partition. Coverage-unknown handles intentionally have no invented
    /// version.
    pub(crate) fn first_observed_version(&self) -> Option<&RepositoryVersion> {
        match self {
            Self::Classified(value) => Some(&value.first_observed_version),
            Self::Unclassified(value) => Some(&value.first_observed_version),
            Self::CoverageUnknown(_) => None,
        }
    }

    pub(crate) fn routine_resolution_digest(
        &self,
        partition: &ValidatedRepositoryHistoryPartition,
        resulting_phase: TaskPhase,
    ) -> Result<Sha256Digest, RepositoryContractError> {
        if self.anchor_cursor() != partition.start_cursor() {
            return Err(RepositoryContractError(
                "deferred advance does not bind the routine partition start",
            ));
        }
        let first = partition.first_entry().ok_or(RepositoryContractError(
            "deferred advance resolution requires its immediate successor",
        ))?;
        let expected_classification = match self {
            Self::Classified(value) => {
                if &value.first_observed_version != first.repository_version()
                    || &value.semantic_delta_digest != first.semantic_delta_digest()
                {
                    return Err(RepositoryContractError(
                        "classified deferred successor differs from routine history",
                    ));
                }
                Some(match value.classification {
                    DeferredRepositoryAdvanceClassification::AuthorizedSupport => {
                        RepositoryHistoryPartitionClassification::AuthorizedSupport
                    }
                    DeferredRepositoryAdvanceClassification::Invalid => {
                        RepositoryHistoryPartitionClassification::Invalid
                    }
                    DeferredRepositoryAdvanceClassification::Corrective => {
                        RepositoryHistoryPartitionClassification::Corrective
                    }
                })
            }
            Self::Unclassified(value) => {
                if &value.first_observed_version != first.repository_version() {
                    return Err(RepositoryContractError(
                        "unclassified deferred successor version was substituted",
                    ));
                }
                None
            }
            Self::CoverageUnknown(_) => None,
        };
        if expected_classification.is_some_and(|expected| expected != first.classification())
            || !matches!(
                first.classification(),
                RepositoryHistoryPartitionClassification::AuthorizedSupport
                    | RepositoryHistoryPartitionClassification::Invalid
                    | RepositoryHistoryPartitionClassification::Corrective
            )
        {
            return Err(RepositoryContractError(
                "deferred successor classification is not an exact routine resolution",
            ));
        }
        contract_digest(
            &DeferredRepositoryAdvanceResolutionDigestRecord {
                advance_observation_digest: self.observation_digest().clone(),
                resolved_history_partition_digest: partition.partition_digest().clone(),
                first_resolved_version: first.repository_version().clone(),
                first_resolved_classification: first.classification(),
                first_resolved_semantic_delta_digest: first.semantic_delta_digest().clone(),
                resulting_phase,
            },
            "deferred repository advance resolution digest failed",
        )
    }

    #[cfg(test)]
    pub(crate) fn coverage_unknown_test_only(
        from_cursor: RepositoryHistoryCursor,
    ) -> Result<Self, RepositoryContractError> {
        let gap = RepositoryHistoryCoverageGapEvidence::from_capability_adapter(
            "repository.history.coverage-gap.v1",
            from_cursor,
        )?;
        Ok(Self::CoverageUnknown(
            CoverageUnknownDeferredRepositoryAdvance::new(&gap)?,
        ))
    }
}

impl JsonSchema for DeferredRepositoryAdvance {
    fn schema_name() -> Cow<'static, str> {
        "DeferredRepositoryAdvance".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        one_of_schema(vec![
            generator.subschema_for::<ClassifiedDeferredRepositoryAdvance>(),
            generator.subschema_for::<UnclassifiedDeferredRepositoryAdvance>(),
            generator.subschema_for::<CoverageUnknownDeferredRepositoryAdvance>(),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DeferredRepositoryAdvanceConsumptionReceiptDigestRecord {
    consumption_receipt_id: UnicaId,
    terminal_receipt_id: UnicaId,
    advance_observation_digest: Sha256Digest,
    routine_update_receipt_id: UnicaId,
    resolved_history_partition_digest: Sha256Digest,
    resulting_phase: TaskPhase,
}

impl contract_digest_record_sealed::Sealed
    for DeferredRepositoryAdvanceConsumptionReceiptDigestRecord
{
}
impl ContractDigestRecord for DeferredRepositoryAdvanceConsumptionReceiptDigestRecord {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DeferredRepositoryAdvanceConsumptionReceipt {
    consumption_receipt_id: UnicaId,
    terminal_receipt_id: UnicaId,
    advance_observation_digest: Sha256Digest,
    routine_update_receipt_id: UnicaId,
    resolved_history_partition_digest: Sha256Digest,
    resulting_phase: TaskPhase,
    receipt_digest: Sha256Digest,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UncheckedDeferredRepositoryAdvanceConsumptionReceipt {
    consumption_receipt_id: UnicaId,
    terminal_receipt_id: UnicaId,
    advance_observation_digest: Sha256Digest,
    routine_update_receipt_id: UnicaId,
    resolved_history_partition_digest: Sha256Digest,
    resulting_phase: TaskPhase,
    receipt_digest: Sha256Digest,
}

impl DeferredRepositoryAdvanceConsumptionReceipt {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        consumption_receipt_id: UnicaId,
        terminal_receipt_id: UnicaId,
        advance_observation_digest: Sha256Digest,
        routine_update_receipt_id: UnicaId,
        resolved_history_partition_digest: Sha256Digest,
        resulting_phase: TaskPhase,
    ) -> Result<Self, RepositoryContractError> {
        let record = DeferredRepositoryAdvanceConsumptionReceiptDigestRecord {
            consumption_receipt_id,
            terminal_receipt_id,
            advance_observation_digest,
            routine_update_receipt_id,
            resolved_history_partition_digest,
            resulting_phase,
        };
        let receipt_digest = contract_digest(
            &record,
            "deferred-advance consumption receipt digest failed",
        )?;
        Ok(Self::from_record(record, receipt_digest))
    }

    fn from_record(
        record: DeferredRepositoryAdvanceConsumptionReceiptDigestRecord,
        receipt_digest: Sha256Digest,
    ) -> Self {
        Self {
            consumption_receipt_id: record.consumption_receipt_id,
            terminal_receipt_id: record.terminal_receipt_id,
            advance_observation_digest: record.advance_observation_digest,
            routine_update_receipt_id: record.routine_update_receipt_id,
            resolved_history_partition_digest: record.resolved_history_partition_digest,
            resulting_phase: record.resulting_phase,
            receipt_digest,
        }
    }

    fn digest_record(&self) -> DeferredRepositoryAdvanceConsumptionReceiptDigestRecord {
        DeferredRepositoryAdvanceConsumptionReceiptDigestRecord {
            consumption_receipt_id: self.consumption_receipt_id.clone(),
            terminal_receipt_id: self.terminal_receipt_id.clone(),
            advance_observation_digest: self.advance_observation_digest.clone(),
            routine_update_receipt_id: self.routine_update_receipt_id.clone(),
            resolved_history_partition_digest: self.resolved_history_partition_digest.clone(),
            resulting_phase: self.resulting_phase,
        }
    }

    fn validate_digest(&self) -> Result<(), RepositoryContractError> {
        validate_digest(
            &self.digest_record(),
            &self.receipt_digest,
            "deferred-advance consumption receipt digest failed",
            "deferred-advance consumption receipt digest mismatch",
        )
    }

    pub(crate) const fn consumption_receipt_id(&self) -> &UnicaId {
        &self.consumption_receipt_id
    }

    pub(crate) const fn terminal_receipt_id(&self) -> &UnicaId {
        &self.terminal_receipt_id
    }

    pub(crate) const fn advance_observation_digest(&self) -> &Sha256Digest {
        &self.advance_observation_digest
    }

    pub(crate) const fn routine_update_receipt_id(&self) -> &UnicaId {
        &self.routine_update_receipt_id
    }

    pub(crate) const fn resolved_history_partition_digest(&self) -> &Sha256Digest {
        &self.resolved_history_partition_digest
    }

    pub(crate) fn receipt_digest(&self) -> &Sha256Digest {
        &self.receipt_digest
    }

    pub(crate) const fn resulting_phase(&self) -> TaskPhase {
        self.resulting_phase
    }
}

impl<'de> Deserialize<'de> for DeferredRepositoryAdvanceConsumptionReceipt {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let unchecked =
            UncheckedDeferredRepositoryAdvanceConsumptionReceipt::deserialize(deserializer)?;
        let value = Self {
            consumption_receipt_id: unchecked.consumption_receipt_id,
            terminal_receipt_id: unchecked.terminal_receipt_id,
            advance_observation_digest: unchecked.advance_observation_digest,
            routine_update_receipt_id: unchecked.routine_update_receipt_id,
            resolved_history_partition_digest: unchecked.resolved_history_partition_digest,
            resulting_phase: unchecked.resulting_phase,
            receipt_digest: unchecked.receipt_digest,
        };
        value.validate_digest().map_err(D::Error::custom)?;
        Ok(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SupportGateHistoryEvidenceDigestRecord {
    gate_observed_cursor: RepositoryHistoryCursor,
    classified_through_cursor: RepositoryHistoryCursor,
    partition_digest: Sha256Digest,
    relevant_baseline_digest: Sha256Digest,
}

impl contract_digest_record_sealed::Sealed for SupportGateHistoryEvidenceDigestRecord {}
impl ContractDigestRecord for SupportGateHistoryEvidenceDigestRecord {}

/// Capability-backed comparison of the current gate baseline with the baseline
/// recomputed by folding the exact validated history partition.
///
/// Task 7 deliberately exposes no raw production constructor. Task 8 must add
/// the exact typed baseline resolver factory here once that producer exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SupportGateRelevantBaselineAuthority {
    gate_observed_cursor: RepositoryHistoryCursor,
    classified_through_cursor: RepositoryHistoryCursor,
    partition_digest: Sha256Digest,
    current_gate_relevant_baseline_digest: Sha256Digest,
    recomputed_relevant_baseline_digest: Sha256Digest,
}

/// Capability boundary that folds the exact validated history partition into
/// the gate's relevant-baseline projection. Implementations are platform
/// adapters; callers cannot supply the claimed recomputed digest directly.
pub(crate) trait SupportGateRelevantBaselineResolver {
    fn recompute_relevant_baseline_digest(
        &self,
        partition: &ValidatedRepositoryHistoryPartition,
        current_gate_relevant_baseline_digest: &Sha256Digest,
    ) -> Result<Sha256Digest, RepositoryContractError>;
}

impl SupportGateRelevantBaselineAuthority {
    pub(crate) fn resolve(
        partition: &ValidatedRepositoryHistoryPartition,
        current_gate_relevant_baseline_digest: Sha256Digest,
        resolver: &dyn SupportGateRelevantBaselineResolver,
    ) -> Result<Self, RepositoryContractError> {
        let recomputed_relevant_baseline_digest = resolver.recompute_relevant_baseline_digest(
            partition,
            &current_gate_relevant_baseline_digest,
        )?;
        if current_gate_relevant_baseline_digest != recomputed_relevant_baseline_digest {
            return Err(RepositoryContractError(
                "support-gate current and recomputed relevant baselines disagree",
            ));
        }
        Ok(Self {
            gate_observed_cursor: partition.start_cursor().clone(),
            classified_through_cursor: partition.through_inclusive().clone(),
            partition_digest: partition.partition_digest().clone(),
            current_gate_relevant_baseline_digest,
            recomputed_relevant_baseline_digest,
        })
    }
}

/// Endpoint-bound, validated all-routine history evidence for reuse of a support gate.
///
/// Deliberately not `Deserialize`: the nested partition can only come from the
/// capability-backed repository-history order resolver.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SupportGateHistoryEvidence {
    gate_observed_cursor: RepositoryHistoryCursor,
    classified_through_cursor: RepositoryHistoryCursor,
    partition: ValidatedRepositoryHistoryPartition,
    relevant_baseline_digest: Sha256Digest,
    evidence_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "SupportGateHistoryEvidence")]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct UnvalidatedSupportGateHistoryEvidence {
    gate_observed_cursor: RepositoryHistoryCursor,
    classified_through_cursor: RepositoryHistoryCursor,
    partition: UnvalidatedRepositoryHistoryPartition,
    relevant_baseline_digest: Sha256Digest,
    evidence_digest: Sha256Digest,
}

impl SupportGateHistoryEvidence {
    pub(crate) fn new(
        partition: ValidatedRepositoryHistoryPartition,
        authority: &SupportGateRelevantBaselineAuthority,
    ) -> Result<Self, RepositoryContractError> {
        if partition.start_cursor() != &authority.gate_observed_cursor
            || partition.through_inclusive() != &authority.classified_through_cursor
            || partition.partition_digest() != &authority.partition_digest
        {
            return Err(RepositoryContractError(
                "support-gate baseline authority scope disagrees with its partition",
            ));
        }
        if authority.current_gate_relevant_baseline_digest
            != authority.recomputed_relevant_baseline_digest
        {
            return Err(RepositoryContractError(
                "support-gate current and recomputed relevant baselines disagree",
            ));
        }
        if !partition.classifications().all(|classification| {
            classification == RepositoryHistoryPartitionClassification::UnrelatedRoutine
        }) {
            return Err(RepositoryContractError(
                "support-gate history evidence must contain only unrelated routine entries",
            ));
        }
        let record = SupportGateHistoryEvidenceDigestRecord {
            gate_observed_cursor: authority.gate_observed_cursor.clone(),
            classified_through_cursor: authority.classified_through_cursor.clone(),
            partition_digest: partition.partition_digest().clone(),
            relevant_baseline_digest: authority.current_gate_relevant_baseline_digest.clone(),
        };
        let evidence_digest =
            contract_digest(&record, "support-gate history evidence digest failed")?;
        Ok(Self {
            gate_observed_cursor: authority.gate_observed_cursor.clone(),
            classified_through_cursor: authority.classified_through_cursor.clone(),
            partition,
            relevant_baseline_digest: authority.current_gate_relevant_baseline_digest.clone(),
            evidence_digest,
        })
    }

    pub(crate) fn from_wire(
        wire: UnvalidatedSupportGateHistoryEvidence,
        partition: ValidatedRepositoryHistoryPartition,
        authority: &SupportGateRelevantBaselineAuthority,
    ) -> Result<Self, RepositoryContractError> {
        if wire.partition != partition.wire {
            return Err(RepositoryContractError(
                "support-gate history evidence partition was not the resolved wire partition",
            ));
        }
        let value = Self::new(partition, authority)?;
        if wire.gate_observed_cursor != value.gate_observed_cursor
            || wire.classified_through_cursor != value.classified_through_cursor
            || wire.relevant_baseline_digest != value.relevant_baseline_digest
            || wire.evidence_digest != value.evidence_digest
        {
            return Err(RepositoryContractError(
                "support-gate history evidence disagrees with its validated authority",
            ));
        }
        Ok(value)
    }

    pub(crate) fn evidence_digest(&self) -> &Sha256Digest {
        &self.evidence_digest
    }

    pub(crate) fn gate_observed_cursor(&self) -> &RepositoryHistoryCursor {
        &self.gate_observed_cursor
    }

    pub(crate) fn classified_through_cursor(&self) -> &RepositoryHistoryCursor {
        &self.classified_through_cursor
    }

    pub(crate) fn relevant_baseline_digest(&self) -> &Sha256Digest {
        &self.relevant_baseline_digest
    }

    /// Publication evidence is anchored at one cursor and therefore has no
    /// intervening history entries. Later gate reuse may replace it only with
    /// a validated all-unrelated prefix.
    pub(crate) fn partition_is_empty(&self) -> bool {
        self.partition.classifications().next().is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PostMergeHistoryGuardEvidenceDigestRecord {
    merge_receipt_cursor: RepositoryHistoryCursor,
    classified_through_cursor: RepositoryHistoryCursor,
    partition_digest: Sha256Digest,
    recomputed_reference_closure_digest: Sha256Digest,
    relevant_tail_absent: TrueLiteral,
    atomic_commit_safety_capability_id: CapabilityRowId,
}

impl contract_digest_record_sealed::Sealed for PostMergeHistoryGuardEvidenceDigestRecord {}
impl ContractDigestRecord for PostMergeHistoryGuardEvidenceDigestRecord {}

/// Capability-backed post-merge closure observation for one exact partition.
///
/// Production construction derives its endpoints and partition digest from a
/// validated partition; the adapter may provide only the receipt start,
/// recomputed closure and capability identity observed atomically.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PostMergeHistoryGuardAuthority {
    merge_receipt_cursor: RepositoryHistoryCursor,
    classified_through_cursor: RepositoryHistoryCursor,
    partition_digest: Sha256Digest,
    recomputed_reference_closure_digest: Sha256Digest,
    atomic_commit_safety_capability_id: CapabilityRowId,
}

impl PostMergeHistoryGuardAuthority {
    /// Seals one atomic repository-adapter observation to the exact validated
    /// history partition that starts at the merge receipt cursor.
    pub(crate) fn from_capability_adapter(
        partition: &ValidatedRepositoryHistoryPartition,
        merge_receipt_cursor: RepositoryHistoryCursor,
        recomputed_reference_closure_digest: Sha256Digest,
        atomic_commit_safety_capability_id: CapabilityRowId,
    ) -> Result<Self, RepositoryContractError> {
        if partition.start_cursor() != &merge_receipt_cursor {
            return Err(RepositoryContractError(
                "post-merge capability observation starts at another merge receipt cursor",
            ));
        }
        Ok(Self {
            merge_receipt_cursor,
            classified_through_cursor: partition.through_inclusive().clone(),
            partition_digest: partition.partition_digest().clone(),
            recomputed_reference_closure_digest,
            atomic_commit_safety_capability_id,
        })
    }
}

/// Endpoint-bound post-merge guard backed by a validated history partition.
///
/// Deliberately not `Deserialize`: callers cannot promote an unvalidated wire
/// partition into commit-control evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PostMergeHistoryGuardEvidence {
    merge_receipt_cursor: RepositoryHistoryCursor,
    classified_through_cursor: RepositoryHistoryCursor,
    partition: ValidatedRepositoryHistoryPartition,
    recomputed_reference_closure_digest: Sha256Digest,
    relevant_tail_absent: TrueLiteral,
    atomic_commit_safety_capability_id: CapabilityRowId,
    evidence_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "PostMergeHistoryGuardEvidence")]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct UnvalidatedPostMergeHistoryGuardEvidence {
    merge_receipt_cursor: RepositoryHistoryCursor,
    classified_through_cursor: RepositoryHistoryCursor,
    partition: UnvalidatedRepositoryHistoryPartition,
    recomputed_reference_closure_digest: Sha256Digest,
    relevant_tail_absent: TrueLiteral,
    atomic_commit_safety_capability_id: CapabilityRowId,
    evidence_digest: Sha256Digest,
}

impl PostMergeHistoryGuardEvidence {
    pub(crate) fn new(
        partition: ValidatedRepositoryHistoryPartition,
        authority: &PostMergeHistoryGuardAuthority,
    ) -> Result<Self, RepositoryContractError> {
        if partition.start_cursor() != &authority.merge_receipt_cursor
            || partition.through_inclusive() != &authority.classified_through_cursor
            || partition.partition_digest() != &authority.partition_digest
        {
            return Err(RepositoryContractError(
                "post-merge history guard authority scope disagrees with its partition",
            ));
        }
        if !partition.classifications().all(|classification| {
            matches!(
                classification,
                RepositoryHistoryPartitionClassification::UnrelatedRoutine
                    | RepositoryHistoryPartitionClassification::NonConflictingConcurrent
            )
        }) {
            return Err(RepositoryContractError(
                "post-merge history guard contains a relevant or unsafe repository entry",
            ));
        }
        if !partition.non_conflicting_entries_bind_atomic_safety_capability(
            &authority.atomic_commit_safety_capability_id,
        ) {
            return Err(RepositoryContractError(
                "post-merge history guard concurrent evidence uses another atomic-safety capability",
            ));
        }
        let record = PostMergeHistoryGuardEvidenceDigestRecord {
            merge_receipt_cursor: authority.merge_receipt_cursor.clone(),
            classified_through_cursor: authority.classified_through_cursor.clone(),
            partition_digest: partition.partition_digest().clone(),
            recomputed_reference_closure_digest: authority
                .recomputed_reference_closure_digest
                .clone(),
            relevant_tail_absent: TrueLiteral,
            atomic_commit_safety_capability_id: authority
                .atomic_commit_safety_capability_id
                .clone(),
        };
        let evidence_digest =
            contract_digest(&record, "post-merge history guard evidence digest failed")?;
        Ok(Self {
            merge_receipt_cursor: authority.merge_receipt_cursor.clone(),
            classified_through_cursor: authority.classified_through_cursor.clone(),
            partition,
            recomputed_reference_closure_digest: authority
                .recomputed_reference_closure_digest
                .clone(),
            relevant_tail_absent: TrueLiteral,
            atomic_commit_safety_capability_id: authority
                .atomic_commit_safety_capability_id
                .clone(),
            evidence_digest,
        })
    }

    pub(crate) fn from_wire(
        wire: UnvalidatedPostMergeHistoryGuardEvidence,
        partition: ValidatedRepositoryHistoryPartition,
        authority: &PostMergeHistoryGuardAuthority,
    ) -> Result<Self, RepositoryContractError> {
        if wire.partition != partition.wire {
            return Err(RepositoryContractError(
                "post-merge history guard partition was not the resolved wire partition",
            ));
        }
        let value = Self::new(partition, authority)?;
        if wire.merge_receipt_cursor != value.merge_receipt_cursor
            || wire.classified_through_cursor != value.classified_through_cursor
            || wire.recomputed_reference_closure_digest != value.recomputed_reference_closure_digest
            || wire.relevant_tail_absent != value.relevant_tail_absent
            || wire.atomic_commit_safety_capability_id != value.atomic_commit_safety_capability_id
            || wire.evidence_digest != value.evidence_digest
        {
            return Err(RepositoryContractError(
                "post-merge history guard disagrees with its validated authority",
            ));
        }
        Ok(value)
    }

    pub(crate) fn evidence_digest(&self) -> &Sha256Digest {
        &self.evidence_digest
    }

    pub(crate) const fn merge_receipt_cursor(&self) -> &RepositoryHistoryCursor {
        &self.merge_receipt_cursor
    }

    pub(crate) const fn classified_through_cursor(&self) -> &RepositoryHistoryCursor {
        &self.classified_through_cursor
    }

    pub(crate) const fn partition(&self) -> &ValidatedRepositoryHistoryPartition {
        &self.partition
    }

    pub(crate) const fn atomic_commit_safety_capability_id(&self) -> &CapabilityRowId {
        &self.atomic_commit_safety_capability_id
    }

    pub(crate) const fn recomputed_reference_closure_digest(&self) -> &Sha256Digest {
        &self.recomputed_reference_closure_digest
    }
}

/// Test fixture that still traverses both validated evidence constructors.
/// It is intentionally cfg-only: production must obtain both authorities from
/// their capability adapters rather than supplying closure/baseline digests.
#[cfg(test)]
pub(crate) fn empty_commit_history_evidence_fixture_test_only(
    cursor: RepositoryHistoryCursor,
    relevant_baseline_digest: Sha256Digest,
    recomputed_reference_closure_digest: Sha256Digest,
    atomic_commit_safety_capability_id: CapabilityRowId,
) -> Result<(SupportGateHistoryEvidence, PostMergeHistoryGuardEvidence), RepositoryContractError> {
    let entries = UnvalidatedRepositoryHistoryEntries(Vec::new());
    let partition_digest = canonical_contract_digest(
        &RepositoryHistoryPartitionDigestRecord {
            from_exclusive: cursor.clone(),
            through_inclusive: cursor.clone(),
            entries: entries.clone(),
        },
        None,
    )
    .map_err(|_| RepositoryContractError("empty fixture partition digest failed"))?;
    let partition = ValidatedRepositoryHistoryPartition {
        wire: UnvalidatedRepositoryHistoryPartition {
            from_exclusive: cursor.clone(),
            through_inclusive: cursor,
            entries,
            partition_digest,
        },
        source_index_proofs: Vec::new(),
        order_evidence: None,
    };
    let baseline_authority = SupportGateRelevantBaselineAuthority {
        gate_observed_cursor: partition.start_cursor().clone(),
        classified_through_cursor: partition.through_inclusive().clone(),
        partition_digest: partition.partition_digest().clone(),
        current_gate_relevant_baseline_digest: relevant_baseline_digest.clone(),
        recomputed_relevant_baseline_digest: relevant_baseline_digest,
    };
    let guard_authority = PostMergeHistoryGuardAuthority {
        merge_receipt_cursor: partition.start_cursor().clone(),
        classified_through_cursor: partition.through_inclusive().clone(),
        partition_digest: partition.partition_digest().clone(),
        recomputed_reference_closure_digest,
        atomic_commit_safety_capability_id,
    };
    Ok((
        SupportGateHistoryEvidence::new(partition.clone(), &baseline_authority)?,
        PostMergeHistoryGuardEvidence::new(partition, &guard_authority)?,
    ))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OriginalCleanRefreshProofDigestRecord {
    expected_original_fingerprint: Sha256Digest,
    observed_original_fingerprint: Sha256Digest,
    observed_history_cursor: RepositoryHistoryCursor,
    repository_clean_at_observed_cursor: TrueLiteral,
    task_merge_started: FalseLiteral,
    capability_row_id: CapabilityRowId,
}

impl contract_digest_record_sealed::Sealed for OriginalCleanRefreshProofDigestRecord {}
impl ContractDigestRecord for OriginalCleanRefreshProofDigestRecord {}

/// Capability-backed clean-scan result for the original configuration.
///
/// Task 7 deliberately exposes no raw production constructor. The exact scan
/// adapter must add its typed factory here before production can mint this token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OriginalCleanRefreshScanAuthority {
    expected_original_fingerprint: Sha256Digest,
    observed_original_fingerprint: Sha256Digest,
    observed_history_cursor: RepositoryHistoryCursor,
    repository_clean_at_observed_cursor: TrueLiteral,
    task_merge_started: FalseLiteral,
    capability_row_id: CapabilityRowId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct OriginalCleanRefreshProof {
    expected_original_fingerprint: Sha256Digest,
    observed_original_fingerprint: Sha256Digest,
    observed_history_cursor: RepositoryHistoryCursor,
    repository_clean_at_observed_cursor: TrueLiteral,
    task_merge_started: FalseLiteral,
    capability_row_id: CapabilityRowId,
    proof_digest: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "OriginalCleanRefreshProof")]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct UnvalidatedOriginalCleanRefreshProof {
    expected_original_fingerprint: Sha256Digest,
    observed_original_fingerprint: Sha256Digest,
    observed_history_cursor: RepositoryHistoryCursor,
    repository_clean_at_observed_cursor: TrueLiteral,
    task_merge_started: FalseLiteral,
    capability_row_id: CapabilityRowId,
    proof_digest: Sha256Digest,
}

impl OriginalCleanRefreshProof {
    pub(crate) fn new(
        authority: &OriginalCleanRefreshScanAuthority,
    ) -> Result<Self, RepositoryContractError> {
        let record = OriginalCleanRefreshProofDigestRecord {
            expected_original_fingerprint: authority.expected_original_fingerprint.clone(),
            observed_original_fingerprint: authority.observed_original_fingerprint.clone(),
            observed_history_cursor: authority.observed_history_cursor.clone(),
            repository_clean_at_observed_cursor: authority.repository_clean_at_observed_cursor,
            task_merge_started: authority.task_merge_started,
            capability_row_id: authority.capability_row_id.clone(),
        };
        let proof_digest = contract_digest(&record, "original-clean refresh proof digest failed")?;
        Ok(Self::from_record(record, proof_digest))
    }

    fn from_record(
        record: OriginalCleanRefreshProofDigestRecord,
        proof_digest: Sha256Digest,
    ) -> Self {
        Self {
            expected_original_fingerprint: record.expected_original_fingerprint,
            observed_original_fingerprint: record.observed_original_fingerprint,
            observed_history_cursor: record.observed_history_cursor,
            repository_clean_at_observed_cursor: record.repository_clean_at_observed_cursor,
            task_merge_started: record.task_merge_started,
            capability_row_id: record.capability_row_id,
            proof_digest,
        }
    }

    pub(crate) fn proof_digest(&self) -> &Sha256Digest {
        &self.proof_digest
    }

    pub(crate) fn from_wire(
        wire: UnvalidatedOriginalCleanRefreshProof,
        authority: &OriginalCleanRefreshScanAuthority,
    ) -> Result<Self, RepositoryContractError> {
        let value = Self::new(authority)?;
        if wire.expected_original_fingerprint != value.expected_original_fingerprint
            || wire.observed_original_fingerprint != value.observed_original_fingerprint
            || wire.observed_history_cursor != value.observed_history_cursor
            || wire.repository_clean_at_observed_cursor != value.repository_clean_at_observed_cursor
            || wire.task_merge_started != value.task_merge_started
            || wire.capability_row_id != value.capability_row_id
            || wire.proof_digest != value.proof_digest
        {
            return Err(RepositoryContractError(
                "original-clean refresh proof disagrees with its clean-scan authority",
            ));
        }
        Ok(value)
    }
}

fn contract_digest<T: ContractDigestRecord>(
    record: &T,
    failure: &'static str,
) -> Result<Sha256Digest, RepositoryContractError> {
    canonical_contract_digest(record, None).map_err(|_| RepositoryContractError(failure))
}

fn validate_digest<T: ContractDigestRecord>(
    record: &T,
    observed: &Sha256Digest,
    computation_failure: &'static str,
    mismatch: &'static str,
) -> Result<(), RepositoryContractError> {
    let expected = contract_digest(record, computation_failure)?;
    (&expected == observed)
        .then_some(())
        .ok_or(RepositoryContractError(mismatch))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::branched_development::contracts::schema::audit_json_schema;
    use schemars::schema_for;
    use serde::de::DeserializeOwned;
    use serde_json::{json, Value};

    // Stable negative-impl assertion: if the type ever gains DeserializeOwned,
    // the inferred marker below becomes ambiguous and compilation fails.
    macro_rules! assert_not_deserialize_owned {
        ($type:ty) => {
            const _: fn() = || {
                trait AmbiguousIfDeserialize<Marker> {
                    fn assert_not_deserialize() {}
                }
                struct ImplementsDeserialize;
                impl<T: ?Sized> AmbiguousIfDeserialize<()> for T {}
                impl<T: ?Sized + DeserializeOwned>
                    AmbiguousIfDeserialize<ImplementsDeserialize> for T
                {
                }
                let _ = <$type as AmbiguousIfDeserialize<_>>::assert_not_deserialize;
            };
        };
    }

    macro_rules! assert_not_clone {
        ($type:ty) => {
            const _: fn() = || {
                trait AmbiguousIfClone<Marker> {
                    fn assert_not_clone() {}
                }
                struct ImplementsClone;
                impl<T: ?Sized> AmbiguousIfClone<()> for T {}
                impl<T: ?Sized + Clone> AmbiguousIfClone<ImplementsClone> for T {}
                let _ = <$type as AmbiguousIfClone<_>>::assert_not_clone;
            };
        };
    }

    assert_not_deserialize_owned!(ClassifiedDeferredRepositoryAdvance);
    assert_not_deserialize_owned!(DeferredRepositoryAdvanceClassificationAuthority);
    assert_not_deserialize_owned!(UnclassifiedDeferredRepositoryAdvance);
    assert_not_deserialize_owned!(DeferredRepositoryAdvanceMissingEvidenceAuthority);
    assert_not_deserialize_owned!(CoverageUnknownDeferredRepositoryAdvance);
    assert_not_deserialize_owned!(DeferredRepositoryAdvance);
    assert_not_deserialize_owned!(SupportGateHistoryEvidence);
    assert_not_deserialize_owned!(SupportGateRelevantBaselineAuthority);
    assert_not_deserialize_owned!(PostMergeHistoryGuardEvidence);
    assert_not_deserialize_owned!(PostMergeHistoryGuardAuthority);
    assert_not_deserialize_owned!(OriginalCleanRefreshProof);
    assert_not_deserialize_owned!(OriginalCleanRefreshScanAuthority);
    assert_not_deserialize_owned!(RoutineUpdatePhaseAuthority);
    assert_not_clone!(RoutineUpdatePhaseAuthority);

    #[test]
    fn routine_phase_authority_derives_only_closed_live_status_transitions() {
        assert_eq!(
            RoutineUpdatePhaseAuthority::routine_test_only(TaskPhase::Synchronized)
                .unwrap()
                .into_resulting_phase(false),
            Ok(TaskPhase::Synchronized)
        );
        assert_eq!(
            RoutineUpdatePhaseAuthority::routine_test_only(TaskPhase::Synchronized)
                .unwrap()
                .into_resulting_phase(true),
            Ok(TaskPhase::LocalVerified)
        );
        assert_eq!(
            RoutineUpdatePhaseAuthority::routine_test_only(TaskPhase::AbandonmentReady)
                .unwrap()
                .into_resulting_phase(true),
            Ok(TaskPhase::AbandonmentReady)
        );
        assert!(
            RoutineUpdatePhaseAuthority::routine_test_only(TaskPhase::RecoveryRequired).is_err()
        );
        assert!(
            RoutineUpdatePhaseAuthority::routine_test_only(TaskPhase::ArchivedSuccess).is_err()
        );
    }

    const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const ID_1: &str = "11111111-1111-4111-8111-111111111111";
    const ID_2: &str = "22222222-2222-4222-8222-222222222222";
    const ID_3: &str = "33333333-3333-4333-8333-333333333333";

    fn cursor() -> RepositoryHistoryCursor {
        cursor_at("v1", A)
    }

    fn cursor_at(version: &str, digest: &str) -> RepositoryHistoryCursor {
        serde_json::from_value(json!({
            "throughVersion": version,
            "historyPrefixDigest": digest,
        }))
        .unwrap()
    }

    fn successor() -> RepositoryHistoryImmediateSuccessorEvidence {
        RepositoryHistoryImmediateSuccessorEvidence::from_capability_adapter(
            "repository.history.immediate-successor.v1",
            cursor(),
            RepositoryVersion::parse("v2").unwrap(),
        )
        .unwrap()
    }

    fn classified_authority(
        classification: DeferredRepositoryAdvanceClassification,
        semantic_delta_digest: &str,
    ) -> DeferredRepositoryAdvanceClassificationAuthority {
        DeferredRepositoryAdvanceClassificationAuthority {
            successor: successor(),
            classification,
            semantic_delta_digest: Sha256Digest::parse(semantic_delta_digest).unwrap(),
        }
    }

    fn missing_evidence_authority(
        missing_evidence_kinds: Vec<SupportMissingEvidenceKind>,
    ) -> DeferredRepositoryAdvanceMissingEvidenceAuthority {
        DeferredRepositoryAdvanceMissingEvidenceAuthority {
            successor: successor(),
            missing_evidence_kinds: DeferredMissingEvidenceKinds::new(missing_evidence_kinds)
                .unwrap(),
        }
    }

    fn coverage_gap() -> RepositoryHistoryCoverageGapEvidence {
        RepositoryHistoryCoverageGapEvidence::from_capability_adapter(
            "repository.history.coverage-gap.v1",
            cursor(),
        )
        .unwrap()
    }

    fn empty_partition() -> ValidatedRepositoryHistoryPartition {
        let from_exclusive = cursor();
        let through_inclusive = from_exclusive.clone();
        let entries = super::super::UnvalidatedRepositoryHistoryEntries(Vec::new());
        let partition_digest = canonical_contract_digest(
            &super::super::RepositoryHistoryPartitionDigestRecord {
                from_exclusive: from_exclusive.clone(),
                through_inclusive: through_inclusive.clone(),
                entries: entries.clone(),
            },
            None,
        )
        .unwrap();
        ValidatedRepositoryHistoryPartition {
            wire: super::super::UnvalidatedRepositoryHistoryPartition {
                from_exclusive,
                through_inclusive,
                entries,
                partition_digest,
            },
            source_index_proofs: Vec::new(),
            order_evidence: None,
        }
    }

    fn deferred_resolution_partition(
        classification: &str,
        semantic_delta_digest: &str,
    ) -> ValidatedRepositoryHistoryPartition {
        let from_exclusive = cursor();
        let through_inclusive = cursor_at("v2", B);
        let entries =
            serde_json::from_value::<super::super::UnvalidatedRepositoryHistoryEntries>(json!([{
                "repositoryVersion": "v2",
                "classification": classification,
                "semanticDeltaDigest": semantic_delta_digest,
                "sourceEvidenceRef": {
                    "sourceKind": "contentAddressed",
                    "evidenceKind": "supportPrerequisiteObservation",
                    "evidenceDigest": A,
                },
            }]))
            .unwrap();
        let partition_digest = canonical_contract_digest(
            &super::super::RepositoryHistoryPartitionDigestRecord {
                from_exclusive: from_exclusive.clone(),
                through_inclusive: through_inclusive.clone(),
                entries: entries.clone(),
            },
            None,
        )
        .unwrap();
        ValidatedRepositoryHistoryPartition {
            wire: super::super::UnvalidatedRepositoryHistoryPartition {
                from_exclusive,
                through_inclusive,
                entries,
                partition_digest,
            },
            source_index_proofs: vec![None],
            order_evidence: None,
        }
    }

    #[test]
    fn deferred_routine_resolution_binds_exact_first_entry_and_result_phase() {
        let classified = DeferredRepositoryAdvance::Classified(
            ClassifiedDeferredRepositoryAdvance::new(&classified_authority(
                DeferredRepositoryAdvanceClassification::AuthorizedSupport,
                A,
            ))
            .unwrap(),
        );
        let partition = deferred_resolution_partition("authorizedSupport", A);
        let digest = classified
            .routine_resolution_digest(&partition, TaskPhase::LocalVerified)
            .unwrap();
        assert_ne!(&digest, classified.observation_digest());
        assert!(classified
            .routine_resolution_digest(
                &deferred_resolution_partition("invalid", A),
                TaskPhase::LocalVerified,
            )
            .is_err());
        assert!(classified
            .routine_resolution_digest(
                &deferred_resolution_partition("authorizedSupport", B),
                TaskPhase::LocalVerified,
            )
            .is_err());

        let unclassified = DeferredRepositoryAdvance::Unclassified(
            UnclassifiedDeferredRepositoryAdvance::new(&missing_evidence_authority(vec![
                SupportMissingEvidenceKind::RepositoryActorUnavailable,
            ]))
            .unwrap(),
        );
        assert!(unclassified
            .routine_resolution_digest(&partition, TaskPhase::LocalVerified)
            .is_ok());
        let coverage_unknown = DeferredRepositoryAdvance::CoverageUnknown(
            CoverageUnknownDeferredRepositoryAdvance::new(&coverage_gap()).unwrap(),
        );
        assert!(coverage_unknown
            .routine_resolution_digest(&partition, TaskPhase::LocalVerified)
            .is_ok());
        assert!(coverage_unknown
            .routine_resolution_digest(&empty_partition(), TaskPhase::LocalVerified)
            .is_err());
    }

    fn gate_baseline_authority(
        partition: &ValidatedRepositoryHistoryPartition,
        current_baseline_digest: &str,
        recomputed_baseline_digest: &str,
    ) -> SupportGateRelevantBaselineAuthority {
        SupportGateRelevantBaselineAuthority {
            gate_observed_cursor: partition.start_cursor().clone(),
            classified_through_cursor: partition.through_inclusive().clone(),
            partition_digest: partition.partition_digest().clone(),
            current_gate_relevant_baseline_digest: Sha256Digest::parse(current_baseline_digest)
                .unwrap(),
            recomputed_relevant_baseline_digest: Sha256Digest::parse(recomputed_baseline_digest)
                .unwrap(),
        }
    }

    fn post_merge_guard_authority(
        partition: &ValidatedRepositoryHistoryPartition,
        recomputed_reference_closure_digest: &str,
        atomic_commit_safety_capability_id: &str,
    ) -> PostMergeHistoryGuardAuthority {
        PostMergeHistoryGuardAuthority {
            merge_receipt_cursor: partition.start_cursor().clone(),
            classified_through_cursor: partition.through_inclusive().clone(),
            partition_digest: partition.partition_digest().clone(),
            recomputed_reference_closure_digest: Sha256Digest::parse(
                recomputed_reference_closure_digest,
            )
            .unwrap(),
            atomic_commit_safety_capability_id: CapabilityRowId::parse(
                atomic_commit_safety_capability_id,
            )
            .unwrap(),
        }
    }

    fn original_clean_scan_authority() -> OriginalCleanRefreshScanAuthority {
        OriginalCleanRefreshScanAuthority {
            expected_original_fingerprint: Sha256Digest::parse(A).unwrap(),
            observed_original_fingerprint: Sha256Digest::parse(B).unwrap(),
            observed_history_cursor: cursor(),
            repository_clean_at_observed_cursor: TrueLiteral,
            task_merge_started: FalseLiteral,
            capability_row_id: CapabilityRowId::parse("repository.original-clean-refresh.v1")
                .unwrap(),
        }
    }

    fn non_conflicting_partition(capability_id: &str) -> ValidatedRepositoryHistoryPartition {
        let evidence =
            super::super::NonConflictingConcurrentEvidence::new("v2", capability_id, A, B, A, B, A)
                .unwrap();
        let evidence_json = serde_json::to_value(&evidence).unwrap();
        let evidence_digest = evidence_json["evidenceDigest"].clone();
        let wire = serde_json::from_value::<UnvalidatedRepositoryHistoryPartition>(json!({
            "fromExclusive": serde_json::to_value(cursor()).unwrap(),
            "throughInclusive": serde_json::to_value(cursor_at("v2", B)).unwrap(),
            "entries": [{
                "repositoryVersion": "v2",
                "classification": "nonConflictingConcurrent",
                "semanticDeltaDigest": A,
                "sourceEvidenceRef": {
                    "sourceKind": "contentAddressed",
                    "evidenceKind": "nonConflictingConcurrent",
                    "evidenceDigest": evidence_digest,
                },
                "nonConflictingConcurrentEvidence": evidence_json,
            }],
            "partitionDigest": A,
        }))
        .unwrap();
        ValidatedRepositoryHistoryPartition {
            wire,
            source_index_proofs: Vec::new(),
            order_evidence: None,
        }
    }

    fn assert_schema_closed<T: JsonSchema>() {
        let schema = serde_json::to_value(schema_for!(T)).unwrap();
        audit_json_schema(&schema).unwrap();
    }

    fn assert_deserialize_owned<T: DeserializeOwned>() {}

    fn assert_digest_and_shape<T>(value: &T, digest_field: &str)
    where
        T: Serialize + DeserializeOwned + JsonSchema,
    {
        let encoded = serde_json::to_value(value).unwrap();
        serde_json::from_value::<T>(encoded.clone()).unwrap();

        let mut substituted = encoded.clone();
        substituted[digest_field] = json!(B);
        assert!(serde_json::from_value::<T>(substituted).is_err());

        let mut unknown = encoded.clone();
        unknown["unexpected"] = json!(true);
        assert!(serde_json::from_value::<T>(unknown).is_err());

        let mut omitted = encoded.clone();
        omitted.as_object_mut().unwrap().remove(digest_field);
        assert!(serde_json::from_value::<T>(omitted).is_err());

        let mut explicit_null = encoded;
        explicit_null[digest_field] = Value::Null;
        assert!(serde_json::from_value::<T>(explicit_null).is_err());
        assert_schema_closed::<T>();
    }

    #[test]
    fn coverage_unknown_uses_the_exact_tuple_and_hashes_every_member() {
        let authority = coverage_gap();
        let value = CoverageUnknownDeferredRepositoryAdvance::new(&authority).unwrap();
        let encoded = serde_json::to_value(&value).unwrap();
        let coverage_incomplete =
            SupportMissingEvidenceKind::RepositoryHistoryCoverageIncomplete.as_str();
        let wire = serde_json::from_value::<UnvalidatedCoverageUnknownDeferredRepositoryAdvance>(
            encoded.clone(),
        )
        .unwrap();
        assert!(CoverageUnknownDeferredRepositoryAdvance::from_wire(wire, &authority).is_ok());
        let mut substituted = encoded.clone();
        substituted["observationDigest"] = json!(B);
        let wire = serde_json::from_value::<UnvalidatedCoverageUnknownDeferredRepositoryAdvance>(
            substituted,
        )
        .unwrap();
        assert!(CoverageUnknownDeferredRepositoryAdvance::from_wire(wire, &authority).is_err());
        let mut unknown = encoded.clone();
        unknown["unexpected"] = json!(true);
        assert!(
            serde_json::from_value::<UnvalidatedCoverageUnknownDeferredRepositoryAdvance>(unknown)
                .is_err()
        );
        assert_eq!(
            encoded["missingEvidenceKinds"],
            json!([coverage_incomplete])
        );
        for invalid in [
            json!([]),
            json!(["rootDeltaUnavailable"]),
            json!([coverage_incomplete, coverage_incomplete]),
        ] {
            let mut candidate = encoded.clone();
            candidate["missingEvidenceKinds"] = invalid;
            assert!(
                serde_json::from_value::<UnvalidatedCoverageUnknownDeferredRepositoryAdvance>(
                    candidate
                )
                .is_err()
            );
        }
        assert_schema_closed::<CoverageUnknownDeferredRepositoryAdvance>();
        assert_schema_closed::<UnvalidatedCoverageUnknownDeferredRepositoryAdvance>();
    }

    #[test]
    fn classified_and_unclassified_advances_copy_capability_proven_successor() {
        let classified_authority = classified_authority(
            DeferredRepositoryAdvanceClassification::AuthorizedSupport,
            B,
        );
        let classified = ClassifiedDeferredRepositoryAdvance::new(&classified_authority).unwrap();
        let encoded = serde_json::to_value(&classified).unwrap();
        assert_eq!(
            encoded["fromCursor"],
            serde_json::to_value(cursor()).unwrap()
        );
        assert_eq!(encoded["firstObservedVersion"], json!("v2"));
        assert_eq!(encoded["classification"], json!("authorizedSupport"));
        assert_eq!(encoded["requiredNextMode"], json!("routine"));
        let wire = serde_json::from_value::<UnvalidatedClassifiedDeferredRepositoryAdvance>(
            encoded.clone(),
        )
        .unwrap();
        assert!(
            ClassifiedDeferredRepositoryAdvance::from_wire(wire, &classified_authority).is_ok()
        );
        let mut substituted = encoded;
        substituted["observationDigest"] = json!(A);
        let wire =
            serde_json::from_value::<UnvalidatedClassifiedDeferredRepositoryAdvance>(substituted)
                .unwrap();
        assert!(
            ClassifiedDeferredRepositoryAdvance::from_wire(wire, &classified_authority).is_err()
        );

        let missing_authority = missing_evidence_authority(vec![
            SupportMissingEvidenceKind::CandidateClassificationUnavailable,
            SupportMissingEvidenceKind::RootDeltaUnavailable,
        ]);
        let unclassified = UnclassifiedDeferredRepositoryAdvance::new(&missing_authority).unwrap();
        let encoded = serde_json::to_value(&unclassified).unwrap();
        assert_eq!(encoded["firstObservedVersion"], json!("v2"));
        assert_eq!(
            encoded["missingEvidenceKinds"],
            json!(["candidateClassificationUnavailable", "rootDeltaUnavailable"])
        );
        let wire = serde_json::from_value::<UnvalidatedUnclassifiedDeferredRepositoryAdvance>(
            encoded.clone(),
        )
        .unwrap();
        assert!(UnclassifiedDeferredRepositoryAdvance::from_wire(wire, &missing_authority).is_ok());
        let mut substituted = encoded.clone();
        substituted["observationDigest"] = json!(A);
        let wire =
            serde_json::from_value::<UnvalidatedUnclassifiedDeferredRepositoryAdvance>(substituted)
                .unwrap();
        assert!(
            UnclassifiedDeferredRepositoryAdvance::from_wire(wire, &missing_authority).is_err()
        );
        let mut unknown = encoded;
        unknown["unexpected"] = json!(true);
        assert!(
            serde_json::from_value::<UnvalidatedUnclassifiedDeferredRepositoryAdvance>(unknown)
                .is_err()
        );
        assert_schema_closed::<UnvalidatedDeferredRepositoryAdvance>();
    }

    #[test]
    fn classified_promotion_rejects_rehashed_semantic_authority_substitution() {
        let successor = successor();
        let authority = DeferredRepositoryAdvanceClassificationAuthority {
            successor: successor.clone(),
            classification: DeferredRepositoryAdvanceClassification::AuthorizedSupport,
            semantic_delta_digest: Sha256Digest::parse(A).unwrap(),
        };
        let classified = ClassifiedDeferredRepositoryAdvance::new(&authority).unwrap();
        let mut substituted = serde_json::to_value(classified).unwrap();
        substituted["classification"] = json!("invalid");
        substituted["semanticDeltaDigest"] = json!(B);
        substituted["observationDigest"] = serde_json::to_value(
            contract_digest(
                &ClassifiedDeferredRepositoryAdvanceObservationDigestRecord {
                    state: ClassifiedState::Value,
                    from_cursor: successor.anchor_cursor().clone(),
                    first_observed_version: successor.first_observed_version().clone(),
                    classification: DeferredRepositoryAdvanceClassification::Invalid,
                    semantic_delta_digest: Sha256Digest::parse(B).unwrap(),
                    required_next_mode: RoutineUpdateMode::Value,
                },
                "test classified digest failed",
            )
            .unwrap(),
        )
        .unwrap();

        let schema =
            serde_json::to_value(schema_for!(UnvalidatedClassifiedDeferredRepositoryAdvance))
                .unwrap();
        assert!(jsonschema::validator_for(&schema)
            .unwrap()
            .is_valid(&substituted));
        let wire = serde_json::from_value(substituted).unwrap();
        assert!(ClassifiedDeferredRepositoryAdvance::from_wire(wire, &authority).is_err());
    }

    #[test]
    fn unclassified_promotion_rejects_rehashed_missing_evidence_substitution() {
        let successor = successor();
        let authority = DeferredRepositoryAdvanceMissingEvidenceAuthority {
            successor: successor.clone(),
            missing_evidence_kinds: DeferredMissingEvidenceKinds::new(vec![
                SupportMissingEvidenceKind::CandidateClassificationUnavailable,
                SupportMissingEvidenceKind::RootDeltaUnavailable,
            ])
            .unwrap(),
        };
        let unclassified = UnclassifiedDeferredRepositoryAdvance::new(&authority).unwrap();
        let mut substituted = serde_json::to_value(unclassified).unwrap();
        let substituted_kinds = DeferredMissingEvidenceKinds::new(vec![
            SupportMissingEvidenceKind::RootDeltaUnavailable,
        ])
        .unwrap();
        substituted["missingEvidenceKinds"] = serde_json::to_value(&substituted_kinds).unwrap();
        substituted["observationDigest"] = serde_json::to_value(
            contract_digest(
                &UnclassifiedDeferredRepositoryAdvanceObservationDigestRecord {
                    state: UnclassifiedState::Value,
                    from_cursor: successor.anchor_cursor().clone(),
                    first_observed_version: successor.first_observed_version().clone(),
                    missing_evidence_kinds: substituted_kinds,
                    required_next_mode: RoutineUpdateMode::Value,
                },
                "test unclassified digest failed",
            )
            .unwrap(),
        )
        .unwrap();

        let schema = serde_json::to_value(schema_for!(
            UnvalidatedUnclassifiedDeferredRepositoryAdvance
        ))
        .unwrap();
        assert!(jsonschema::validator_for(&schema)
            .unwrap()
            .is_valid(&substituted));
        let wire = serde_json::from_value(substituted).unwrap();
        assert!(UnclassifiedDeferredRepositoryAdvance::from_wire(wire, &authority).is_err());
    }

    #[test]
    fn unclassified_gap_collection_is_nonempty_canonical_bounded_and_excludes_coverage() {
        for invalid in [
            vec![],
            vec![SupportMissingEvidenceKind::RepositoryHistoryCoverageIncomplete],
            vec![
                SupportMissingEvidenceKind::RootDeltaUnavailable,
                SupportMissingEvidenceKind::CandidateClassificationUnavailable,
            ],
            vec![
                SupportMissingEvidenceKind::RootDeltaUnavailable,
                SupportMissingEvidenceKind::RootDeltaUnavailable,
            ],
        ] {
            assert!(DeferredMissingEvidenceKinds::new(invalid).is_err());
        }
        assert!(DeferredMissingEvidenceKinds::new(
            (0..=MAX_SUPPORT_MISSING_EVIDENCE_KINDS)
                .map(|_| SupportMissingEvidenceKind::RootDeltaUnavailable)
                .collect(),
        )
        .is_err());
        assert!(DeferredMissingEvidenceKinds::new(vec![
            SupportMissingEvidenceKind::CandidateClassificationUnavailable,
            SupportMissingEvidenceKind::RootDeltaUnavailable,
        ])
        .is_ok());
        assert_schema_closed::<DeferredRepositoryAdvance>();
    }

    #[test]
    fn consumption_receipt_hashes_closed_task_phase_and_rejects_substitution() {
        let value = DeferredRepositoryAdvanceConsumptionReceipt::new(
            UnicaId::parse(ID_1).unwrap(),
            UnicaId::parse(ID_2).unwrap(),
            Sha256Digest::parse(A).unwrap(),
            UnicaId::parse(ID_3).unwrap(),
            Sha256Digest::parse(B).unwrap(),
            TaskPhase::Synchronized,
        )
        .unwrap();
        assert_eq!(value.consumption_receipt_id().as_str(), ID_1);
        assert_eq!(value.terminal_receipt_id().as_str(), ID_2);
        assert_eq!(value.advance_observation_digest().as_str(), A);
        assert_eq!(value.resulting_phase(), TaskPhase::Synchronized);
        assert_digest_and_shape(&value, "receiptDigest");

        let mut substituted = serde_json::to_value(&value).unwrap();
        substituted["resultingPhase"] = json!("developing");
        assert!(
            serde_json::from_value::<DeferredRepositoryAdvanceConsumptionReceipt>(substituted)
                .is_err()
        );
    }

    #[test]
    fn original_clean_refresh_proof_retains_both_literal_booleans_and_exact_digest() {
        let authority = original_clean_scan_authority();
        let value = OriginalCleanRefreshProof::new(&authority).unwrap();
        let encoded = serde_json::to_value(&value).unwrap();
        let wire = serde_json::from_value::<UnvalidatedOriginalCleanRefreshProof>(encoded.clone())
            .unwrap();
        assert!(OriginalCleanRefreshProof::from_wire(wire, &authority).is_ok());
        let mut substituted_digest = encoded.clone();
        substituted_digest["proofDigest"] = json!(A);
        let wire = serde_json::from_value(substituted_digest).unwrap();
        assert!(OriginalCleanRefreshProof::from_wire(wire, &authority).is_err());
        assert_eq!(encoded["repositoryCleanAtObservedCursor"], json!(true));
        assert_eq!(encoded["taskMergeStarted"], json!(false));
        for (field, wrong) in [
            ("repositoryCleanAtObservedCursor", json!(false)),
            ("taskMergeStarted", json!(true)),
        ] {
            let mut candidate = encoded.clone();
            candidate[field] = wrong;
            assert!(
                serde_json::from_value::<UnvalidatedOriginalCleanRefreshProof>(candidate).is_err()
            );
        }
        assert_schema_closed::<OriginalCleanRefreshProof>();
        assert_schema_closed::<UnvalidatedOriginalCleanRefreshProof>();
    }

    #[test]
    fn original_clean_refresh_rejects_a_rehashed_unproven_clean_scan() {
        let authority = original_clean_scan_authority();
        let proof = OriginalCleanRefreshProof::new(&authority).unwrap();
        let mut substituted = serde_json::to_value(proof).unwrap();
        let substituted_expected = Sha256Digest::parse(B).unwrap();
        substituted["expectedOriginalFingerprint"] =
            serde_json::to_value(&substituted_expected).unwrap();
        substituted["proofDigest"] = serde_json::to_value(
            contract_digest(
                &OriginalCleanRefreshProofDigestRecord {
                    expected_original_fingerprint: substituted_expected,
                    observed_original_fingerprint: Sha256Digest::parse(B).unwrap(),
                    observed_history_cursor: cursor(),
                    repository_clean_at_observed_cursor: TrueLiteral,
                    task_merge_started: FalseLiteral,
                    capability_row_id: CapabilityRowId::parse(
                        "repository.original-clean-refresh.v1",
                    )
                    .unwrap(),
                },
                "test original-clean refresh proof digest failed",
            )
            .unwrap(),
        )
        .unwrap();

        let schema =
            serde_json::to_value(schema_for!(UnvalidatedOriginalCleanRefreshProof)).unwrap();
        assert!(jsonschema::validator_for(&schema)
            .unwrap()
            .is_valid(&substituted));
        let wire = serde_json::from_value(substituted).unwrap();
        assert!(OriginalCleanRefreshProof::from_wire(wire, &authority).is_err());
    }

    #[test]
    fn gate_and_post_merge_evidence_bind_validated_partition_endpoints_and_digest() {
        let partition = empty_partition();
        let baseline_authority = gate_baseline_authority(&partition, A, A);
        let gate = SupportGateHistoryEvidence::new(partition.clone(), &baseline_authority).unwrap();
        let gate_json = serde_json::to_value(&gate).unwrap();
        assert_eq!(
            gate_json["partition"]["partitionDigest"],
            serde_json::to_value(partition.partition_digest()).unwrap()
        );
        assert_ne!(gate_json["evidenceDigest"], Value::Null);
        let gate_wire =
            serde_json::from_value::<UnvalidatedSupportGateHistoryEvidence>(gate_json.clone())
                .unwrap();
        assert!(SupportGateHistoryEvidence::from_wire(
            gate_wire,
            partition.clone(),
            &baseline_authority,
        )
        .is_ok());
        let mut substituted_gate = gate_json.clone();
        substituted_gate["evidenceDigest"] = json!(B);
        let gate_wire =
            serde_json::from_value::<UnvalidatedSupportGateHistoryEvidence>(substituted_gate)
                .unwrap();
        assert!(SupportGateHistoryEvidence::from_wire(
            gate_wire,
            partition.clone(),
            &baseline_authority,
        )
        .is_err());

        let gate_schema = serde_json::to_value(schema_for!(SupportGateHistoryEvidence)).unwrap();
        let gate_validator = jsonschema::options()
            .with_draft(jsonschema::Draft::Draft202012)
            .build(&gate_schema)
            .unwrap();
        assert!(gate_validator.is_valid(&gate_json));
        let mut nested_unknown = gate_json.clone();
        nested_unknown["partition"]["unexpected"] = json!(true);
        assert!(!gate_validator.is_valid(&nested_unknown));
        assert!(
            serde_json::from_value::<UnvalidatedSupportGateHistoryEvidence>(nested_unknown)
                .is_err()
        );

        let guard_authority =
            post_merge_guard_authority(&partition, B, "repository.atomic-commit-safety.v1");
        let guard =
            PostMergeHistoryGuardEvidence::new(partition.clone(), &guard_authority).unwrap();
        let guard_json = serde_json::to_value(&guard).unwrap();
        assert_eq!(guard_json["relevantTailAbsent"], json!(true));
        let guard_wire =
            serde_json::from_value::<UnvalidatedPostMergeHistoryGuardEvidence>(guard_json.clone())
                .unwrap();
        assert!(PostMergeHistoryGuardEvidence::from_wire(
            guard_wire,
            partition.clone(),
            &guard_authority,
        )
        .is_ok());
        let mut substituted_guard = guard_json.clone();
        substituted_guard["evidenceDigest"] = json!(A);
        let guard_wire =
            serde_json::from_value::<UnvalidatedPostMergeHistoryGuardEvidence>(substituted_guard)
                .unwrap();
        assert!(
            PostMergeHistoryGuardEvidence::from_wire(guard_wire, partition, &guard_authority,)
                .is_err()
        );

        let guard_schema =
            serde_json::to_value(schema_for!(PostMergeHistoryGuardEvidence)).unwrap();
        let guard_validator = jsonschema::options()
            .with_draft(jsonschema::Draft::Draft202012)
            .build(&guard_schema)
            .unwrap();
        assert!(guard_validator.is_valid(&guard_json));
        let mut nested_unknown = guard_json;
        nested_unknown["partition"]["unexpected"] = json!(true);
        assert!(!guard_validator.is_valid(&nested_unknown));
        assert!(
            serde_json::from_value::<UnvalidatedPostMergeHistoryGuardEvidence>(nested_unknown)
                .is_err()
        );

        let partition = empty_partition();
        let mut mismatched_authority = gate_baseline_authority(&partition, A, A);
        mismatched_authority.classified_through_cursor = cursor_at("v2", B);
        assert!(SupportGateHistoryEvidence::new(partition, &mismatched_authority).is_err());
    }

    #[test]
    fn post_merge_guard_binds_every_concurrent_entry_to_its_atomic_safety_capability() {
        let capability = CapabilityRowId::parse("repository.atomic-commit-safety.v1").unwrap();
        let substituted_capability =
            CapabilityRowId::parse("repository.atomic-commit-safety.v2").unwrap();
        let partition = non_conflicting_partition(capability.as_str());
        let through = cursor_at("v2", B);
        let closure_digest = Sha256Digest::parse(B).unwrap();
        let authority = post_merge_guard_authority(&partition, B, capability.as_str());
        let guard = PostMergeHistoryGuardEvidence::new(partition.clone(), &authority).unwrap();

        let mut substituted = serde_json::to_value(guard).unwrap();
        substituted["atomicCommitSafetyCapabilityId"] =
            serde_json::to_value(&substituted_capability).unwrap();
        substituted["evidenceDigest"] = serde_json::to_value(
            contract_digest(
                &PostMergeHistoryGuardEvidenceDigestRecord {
                    merge_receipt_cursor: cursor(),
                    classified_through_cursor: through,
                    partition_digest: partition.partition_digest().clone(),
                    recomputed_reference_closure_digest: closure_digest,
                    relevant_tail_absent: TrueLiteral,
                    atomic_commit_safety_capability_id: substituted_capability,
                },
                "test guard digest failed",
            )
            .unwrap(),
        )
        .unwrap();

        let schema =
            serde_json::to_value(schema_for!(UnvalidatedPostMergeHistoryGuardEvidence)).unwrap();
        let validator = jsonschema::validator_for(&schema).unwrap();
        assert!(
            validator.is_valid(&substituted),
            "schema is intentionally a structural superset and cannot compare nested capability IDs"
        );
        let wire = serde_json::from_value(substituted).unwrap();
        assert!(PostMergeHistoryGuardEvidence::from_wire(wire, partition, &authority).is_err());
    }

    #[test]
    fn post_merge_guard_exposes_only_its_validated_commit_lineage() {
        let partition = empty_partition();
        let authority = PostMergeHistoryGuardAuthority::from_capability_adapter(
            &partition,
            partition.start_cursor().clone(),
            Sha256Digest::parse(B).unwrap(),
            CapabilityRowId::parse("repository.atomic-commit-safety.v1").unwrap(),
        )
        .unwrap();
        let guard = PostMergeHistoryGuardEvidence::new(partition.clone(), &authority).unwrap();

        assert_eq!(guard.merge_receipt_cursor(), partition.start_cursor());
        assert_eq!(
            guard.classified_through_cursor(),
            partition.through_inclusive()
        );
        assert_eq!(
            guard.atomic_commit_safety_capability_id(),
            &CapabilityRowId::parse("repository.atomic-commit-safety.v1").unwrap()
        );
        assert_eq!(
            guard.recomputed_reference_closure_digest(),
            &Sha256Digest::parse(B).unwrap()
        );
        assert!(PostMergeHistoryGuardAuthority::from_capability_adapter(
            &partition,
            cursor_at("v9", A),
            Sha256Digest::parse(B).unwrap(),
            CapabilityRowId::parse("repository.atomic-commit-safety.v1").unwrap(),
        )
        .is_err());
    }

    #[test]
    fn post_merge_guard_rejects_rehashed_unproven_closure_and_capability_substitution() {
        let partition = empty_partition();
        let authority =
            post_merge_guard_authority(&partition, A, "repository.atomic-commit-safety.v1");
        let guard = PostMergeHistoryGuardEvidence::new(partition.clone(), &authority).unwrap();
        let mut substituted = serde_json::to_value(guard).unwrap();
        let substituted_capability =
            CapabilityRowId::parse("repository.atomic-commit-safety.v2").unwrap();
        substituted["recomputedReferenceClosureDigest"] = json!(B);
        substituted["atomicCommitSafetyCapabilityId"] =
            serde_json::to_value(&substituted_capability).unwrap();
        substituted["evidenceDigest"] = serde_json::to_value(
            contract_digest(
                &PostMergeHistoryGuardEvidenceDigestRecord {
                    merge_receipt_cursor: cursor(),
                    classified_through_cursor: cursor(),
                    partition_digest: partition.partition_digest().clone(),
                    recomputed_reference_closure_digest: Sha256Digest::parse(B).unwrap(),
                    relevant_tail_absent: TrueLiteral,
                    atomic_commit_safety_capability_id: substituted_capability,
                },
                "test post-merge guard digest failed",
            )
            .unwrap(),
        )
        .unwrap();

        let schema =
            serde_json::to_value(schema_for!(UnvalidatedPostMergeHistoryGuardEvidence)).unwrap();
        assert!(jsonschema::validator_for(&schema)
            .unwrap()
            .is_valid(&substituted));
        let wire = serde_json::from_value(substituted).unwrap();
        assert!(PostMergeHistoryGuardEvidence::from_wire(wire, partition, &authority).is_err());
    }

    #[test]
    fn gate_schema_is_a_structural_superset_of_promotion_relations() {
        let partition = empty_partition();
        let baseline_digest = Sha256Digest::parse(A).unwrap();
        let authority = gate_baseline_authority(&partition, A, A);
        let gate = SupportGateHistoryEvidence::new(partition.clone(), &authority).unwrap();
        let schema =
            serde_json::to_value(schema_for!(UnvalidatedSupportGateHistoryEvidence)).unwrap();
        let validator = jsonschema::validator_for(&schema).unwrap();

        let mismatched_cursor = cursor_at("v2", B);
        let mut endpoint_substitution = serde_json::to_value(&gate).unwrap();
        endpoint_substitution["gateObservedCursor"] =
            serde_json::to_value(&mismatched_cursor).unwrap();
        endpoint_substitution["evidenceDigest"] = serde_json::to_value(
            contract_digest(
                &SupportGateHistoryEvidenceDigestRecord {
                    gate_observed_cursor: mismatched_cursor,
                    classified_through_cursor: cursor(),
                    partition_digest: partition.partition_digest().clone(),
                    relevant_baseline_digest: baseline_digest,
                },
                "test gate digest failed",
            )
            .unwrap(),
        )
        .unwrap();
        assert!(
            validator.is_valid(&endpoint_substitution),
            "schema intentionally cannot compare gate endpoints with nested partition endpoints"
        );
        let wire = serde_json::from_value(endpoint_substitution).unwrap();
        assert!(
            SupportGateHistoryEvidence::from_wire(wire, partition.clone(), &authority).is_err()
        );

        let mut digest_substitution = serde_json::to_value(gate).unwrap();
        digest_substitution["evidenceDigest"] = json!(B);
        assert!(
            validator.is_valid(&digest_substitution),
            "schema intentionally cannot recompute the evidence digest"
        );
        let wire = serde_json::from_value(digest_substitution).unwrap();
        assert!(SupportGateHistoryEvidence::from_wire(wire, partition, &authority).is_err());
    }

    #[test]
    fn gate_promotion_rejects_a_rehashed_unproven_baseline_substitution() {
        let partition = empty_partition();
        let authority = gate_baseline_authority(&partition, A, A);
        let gate = SupportGateHistoryEvidence::new(partition.clone(), &authority).unwrap();
        let mut substituted = serde_json::to_value(gate).unwrap();
        let substituted_baseline = Sha256Digest::parse(B).unwrap();
        substituted["relevantBaselineDigest"] =
            serde_json::to_value(&substituted_baseline).unwrap();
        substituted["evidenceDigest"] = serde_json::to_value(
            contract_digest(
                &SupportGateHistoryEvidenceDigestRecord {
                    gate_observed_cursor: cursor(),
                    classified_through_cursor: cursor(),
                    partition_digest: partition.partition_digest().clone(),
                    relevant_baseline_digest: substituted_baseline,
                },
                "test gate evidence digest failed",
            )
            .unwrap(),
        )
        .unwrap();

        let schema =
            serde_json::to_value(schema_for!(UnvalidatedSupportGateHistoryEvidence)).unwrap();
        assert!(jsonschema::validator_for(&schema)
            .unwrap()
            .is_valid(&substituted));
        let wire = serde_json::from_value(substituted).unwrap();
        assert!(SupportGateHistoryEvidence::from_wire(wire, partition, &authority).is_err());

        let mismatched_authority = gate_baseline_authority(&empty_partition(), A, B);
        assert!(SupportGateHistoryEvidence::new(empty_partition(), &mismatched_authority).is_err());
    }

    #[test]
    fn all_lifecycle_digest_records_have_closed_recursive_schemas() {
        assert_schema_closed::<ClassifiedDeferredRepositoryAdvanceObservationDigestRecord>();
        assert_schema_closed::<UnclassifiedDeferredRepositoryAdvanceObservationDigestRecord>();
        assert_schema_closed::<CoverageUnknownDeferredRepositoryAdvanceObservationDigestRecord>();
        assert_schema_closed::<DeferredRepositoryAdvanceConsumptionReceiptDigestRecord>();
        assert_schema_closed::<SupportGateHistoryEvidenceDigestRecord>();
        assert_schema_closed::<SupportGateHistoryEvidence>();
        assert_schema_closed::<UnvalidatedSupportGateHistoryEvidence>();
        assert_schema_closed::<PostMergeHistoryGuardEvidenceDigestRecord>();
        assert_schema_closed::<PostMergeHistoryGuardEvidence>();
        assert_schema_closed::<UnvalidatedPostMergeHistoryGuardEvidence>();
        assert_schema_closed::<OriginalCleanRefreshProofDigestRecord>();
        assert_schema_closed::<OriginalCleanRefreshProof>();
        assert_schema_closed::<UnvalidatedOriginalCleanRefreshProof>();
    }

    #[test]
    fn only_closed_unvalidated_authority_claim_dtos_are_deserializable() {
        assert_deserialize_owned::<UnvalidatedClassifiedDeferredRepositoryAdvance>();
        assert_deserialize_owned::<UnvalidatedUnclassifiedDeferredRepositoryAdvance>();
        assert_deserialize_owned::<UnvalidatedCoverageUnknownDeferredRepositoryAdvance>();
        assert_deserialize_owned::<UnvalidatedDeferredRepositoryAdvance>();
        assert_deserialize_owned::<UnvalidatedSupportGateHistoryEvidence>();
        assert_deserialize_owned::<UnvalidatedPostMergeHistoryGuardEvidence>();
        assert_deserialize_owned::<UnvalidatedOriginalCleanRefreshProof>();
    }
}
